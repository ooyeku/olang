//! Hot-function promotion to the bytecode tier.
//!
//! The interpreter consults this on every user function call. Once a function
//! has been called `threshold` times, the tier tries to compile it; if that
//! succeeds, subsequent calls execute as bytecode instead of walking the AST.
//!
//! The design rule is that promotion may never change what a program does.
//! Everything the bytecode tier cannot handle is *rejected at compile time*
//! and permanently falls back to the interpreter:
//!
//! - bodies referencing anything but their own parameters and locals (the
//!   VM has no environment, so captured/global variables are unresolvable)
//! - unsupported expressions (match, lambdas, pipelines, for loops, …)
//! - calls to anything but the function itself or a VM-implemented builtin
//! - arguments or results that don't round-trip through the OVM value model
//!
//! `tests/bytecode_differential_test.rs` is the safety net: it asserts the VM
//! and interpreter agree on every program the VM accepts.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use crate::ast::{Function, FunctionDecl, Value};
use crate::ovm::bytecode::{BytecodeError, BytecodeVm};
use crate::ovm::{FunctionId, OvmValue};

/// Default number of calls before a function is considered hot.
pub const DEFAULT_PROMOTION_THRESHOLD: u32 = 50;

/// Outcome of asking the tier to run a call.
pub enum TierOutcome {
    /// The call ran on the bytecode VM.
    Ran(Result<Value, String>),
    /// Not eligible (or not hot yet) — the caller should interpret it.
    Fallback,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TierStats {
    pub promoted: u32,
    pub rejected: u32,
    pub bytecode_calls: u64,
    pub instructions_executed: u64,
}

pub struct BytecodeTier {
    vm: BytecodeVm,
    threshold: u32,
    call_counts: HashMap<String, u32>,
    /// Name -> (compiled id, the exact body it was compiled from). The body is
    /// part of the key on purpose: trait dispatch resolves a bare method name
    /// (`desc`) to *different* function bodies depending on the receiver's
    /// type — a trait default for one type, an `impl` override for another.
    /// The name alone cannot tell those apart, so `try_call` guards a cache
    /// hit on body identity and treats a mismatch as a polymorphic name.
    compiled: HashMap<String, (FunctionId, Arc<crate::ast::Expr>)>,
    /// Names that failed compilation — never retried
    rejected: HashSet<String>,
    /// User functions the interpreter has declared, so a promoted function
    /// calling a helper can have that helper compiled too
    known_functions: HashMap<String, Function>,
    /// Names bound to more than one distinct function body — e.g. the same
    /// function name defined in two different modules. The tier resolves
    /// callees by name, which cannot tell such functions apart, so an
    /// ambiguous name is never tiered: the interpreter runs it and resolves it
    /// through each function's own closure. Correctness over speed.
    ambiguous: HashSet<String>,
    stats: TierStats,
    /// Emit a line when a function is promoted (for --ovm-stats / debugging)
    verbose: bool,
    /// Pointer-identity cache for argument conversion. Arc-backed lists are
    /// immutable, so a list already converted (and validated representable)
    /// converts for free on every later call — a hot function taking a large
    /// list no longer pays a deep Value->OvmValue walk per call. Entries are
    /// keyed by allocation address and validated with a Weak upgrade, so a
    /// freed-and-reused address can never produce a stale hit.
    arg_cache: HashMap<usize, (Weak<[Value]>, OvmValue)>,
}

impl BytecodeTier {
    /// The promotion threshold this tier was built with — used to give
    /// worker-thread interpreters a fresh tier with the same policy.
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    pub fn new(threshold: u32) -> Self {
        Self {
            vm: BytecodeVm::new(),
            threshold,
            call_counts: HashMap::new(),
            compiled: HashMap::new(),
            rejected: HashSet::new(),
            known_functions: HashMap::new(),
            ambiguous: HashSet::new(),
            arg_cache: HashMap::new(),
            stats: TierStats::default(),
            verbose: false,
        }
    }

    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    pub fn stats(&self) -> TierStats {
        // The instruction counter lives on the VM, not in the tier's own
        // tally, so it is read through at reporting time.
        TierStats {
            instructions_executed: self.vm.instructions_executed(),
            ..self.stats
        }
    }

    /// Record a struct declaration so literals of the type can validate at
    /// compile time. A redeclaration with a different field set invalidates
    /// every compiled function (their baked validation may be stale); they
    /// recompile on next call, and literals of the changed type refuse.
    pub fn note_struct(
        &mut self,
        name: String,
        fields: Vec<String>,
        field_checks: HashMap<String, crate::ast::FieldTypeCheck>,
    ) {
        if self.vm.note_struct(name, fields, field_checks) {
            self.compiled.clear();
        }
    }

    /// Record trait dispatch facts as declarations evaluate. Any change
    /// to the dispatch landscape invalidates compiled functions — a
    /// CallMethod site may hold a resolution that no longer matches the
    /// interpreter's.
    pub fn note_trait_impl(&mut self, type_name: String, method: String, func: Function) {
        if self.vm.note_trait_impl(type_name, method, func) {
            self.compiled.clear();
        }
    }

    pub fn note_trait_default(&mut self, trait_name: String, method: String, func: Function) {
        if self.vm.note_trait_default(trait_name, method, func) {
            self.compiled.clear();
        }
    }

    pub fn note_type_trait(&mut self, type_name: String, trait_name: String) {
        if self.vm.note_type_trait(type_name, trait_name) {
            self.compiled.clear();
        }
    }

    /// Record a declared unit enum variant. A NEW name invalidates every
    /// compiled function: a bare-identifier pattern of this name compiled
    /// as a binding is now an equality match.
    pub fn note_unit_variant(&mut self, name: String) {
        if self.vm.note_unit_variant(name) {
            self.compiled.clear();
        }
    }

    /// Record a user function declaration so calls to it can be compiled.
    pub fn note_function(&mut self, name: String, func: Function) {
        // Already known to be ambiguous: a second module's same-named function
        // stays off the tier for the rest of the run.
        if self.ambiguous.contains(&name) {
            return;
        }

        if let Some(existing) = self.known_functions.get(&name) {
            // The same declaration re-noted (e.g. a module re-closing its
            // exports over the full scope) keeps the same body — nothing to do.
            if Arc::ptr_eq(&existing.body, &func.body) {
                return;
            }
            // A different function now shares this name. Calls can no longer be
            // resolved by name: drop every trace and mark the name ambiguous so
            // it is interpreted from here on. Clearing `compiled` wholesale is
            // deliberate — a previously compiled caller may have resolved the
            // old id, and must recompile without it.
            self.ambiguous.insert(name.clone());
            self.known_functions.remove(&name);
            self.call_counts.remove(&name);
            self.rejected.remove(&name);
            self.vm.unregister_function(&name);
            self.compiled.clear();
            return;
        }

        // A user definition shadows any builtin of the same name
        self.vm.shadow_builtin(&name);
        self.vm.note_function_value(name.clone(), func.clone());
        self.known_functions.insert(name, func);
    }

    /// Try to execute `func(args)` on the bytecode VM.
    pub fn try_call(&mut self, func: &Function, args: &[Value]) -> TierOutcome {
        // Borrowed, not cloned: `func` is the caller's, independent of
        // `self`, so the lookups below need no owned copy. This used to
        // allocate a String on every call of every named function purely to
        // probe three maps with it.
        let name: &str = match &func.name {
            Some(name) => name,
            // Anonymous lambdas have no stable identity to profile
            None => return TierOutcome::Fallback,
        };

        if self.rejected.contains(name) {
            return TierOutcome::Fallback;
        }

        // A name shared by two distinct functions can't be tiered soundly.
        if self.ambiguous.contains(name) {
            return TierOutcome::Fallback;
        }

        // A cache hit is only valid for the *same* body. Trait methods share a
        // bare name across distinct bodies (a trait default and a type's
        // override), and the interpreter hands us whichever body its
        // receiver-type dispatch resolved. Replaying the first-seen body for a
        // second type would compute a wrong result, so a body mismatch makes
        // the name ambiguous: the interpreter owns it from here on, resolving
        // by receiver type. (The end-to-end guard also covers the case a trait
        // default reaches the tier through a channel `note_function` never
        // sees.)
        let cached = self
            .compiled
            .get(name)
            .map(|(id, body)| (*id, Arc::ptr_eq(body, &func.body)));
        let func_id = match cached {
            Some((id, true)) => id,
            Some((_, false)) => {
                self.mark_ambiguous(name);
                return TierOutcome::Fallback;
            }
            None => {
                let count = self.call_counts.entry(name.to_string()).or_insert(0);
                *count += 1;
                if *count < self.threshold {
                    return TierOutcome::Fallback;
                }
                match self.compile(name, func) {
                    Some(id) => id,
                    None => return TierOutcome::Fallback,
                }
            }
        };

        // Arguments must round-trip through the OVM value model
        let mut ovm_args = Vec::with_capacity(args.len());
        for arg in args {
            match self.convert_arg(arg) {
                Some(v) => ovm_args.push(v),
                None => return TierOutcome::Fallback,
            }
        }

        self.stats.bytecode_calls += 1;
        match self.vm.execute(func_id, &ovm_args) {
            Ok(value) => match value.to_ast() {
                Ok(ast) => TierOutcome::Ran(Ok(ast)),
                // A result we can't convert would be observable as a wrong
                // value; refuse rather than return something else.
                Err(_) => {
                    self.reject(name);
                    TierOutcome::Fallback
                }
            },
            Err(e) => {
                // RuntimeError's Display already says "Runtime error: {msg}",
                // and the consumer wraps this string in an
                // InterpreterError::RuntimeError that prepends the same words
                // — which doubled the prefix relative to the interpreter's
                // own errors. Hand over the bare message instead.
                let message = match e {
                    crate::ovm::bytecode::BytecodeError::RuntimeError(msg) => msg,
                    other => other.to_string(),
                };
                // Some builtin errors arrive already carrying the
                // interpreter's own "Runtime error: " prefix (they wrap an
                // InterpreterError's Display). The consumer re-wraps this
                // string in an InterpreterError::RuntimeError that prepends
                // the same words, so a redundant leading copy would double
                // the prefix relative to the interpreter. Strip one so the
                // result reads with a single prefix either way.
                let message = message
                    .strip_prefix("Runtime error: ")
                    .map(str::to_string)
                    .unwrap_or(message);
                TierOutcome::Ran(Err(message))
            }
        }
    }

    /// Compile `name`, pulling in any user functions it calls.
    ///
    /// The compiler reports an unresolved callee rather than failing outright,
    /// so this compiles the callee and retries. Ids are registered with the VM
    /// *before* compilation, which is what lets mutually recursive functions
    /// resolve each other.
    fn compile(&mut self, name: &str, func: &Function) -> Option<FunctionId> {
        // Bound the retry loop: each iteration resolves one callee, so this is
        // only reached by a pathological dependency graph.
        const MAX_RESOLUTION_STEPS: usize = 64;

        let func_id = self.register(name, func)?;

        for _ in 0..MAX_RESOLUTION_STEPS {
            let decl = match Self::declaration(name, func) {
                Some(decl) => decl,
                None => {
                    self.reject(name);
                    return None;
                }
            };

            match self
                .vm
                .compile_function_with_closure(func_id, &decl, func.closure.clone())
            {
                Ok(()) => {
                    self.compiled
                        .insert(name.to_string(), (func_id, func.body.clone()));
                    self.stats.promoted += 1;
                    if self.verbose {
                        eprintln!("[ovm] promoted '{}' to the bytecode tier", name);
                    }
                    return Some(func_id);
                }
                Err(BytecodeError::UnresolvedCallee(callee)) => {
                    // Resolve the dependency, then retry this function
                    if !self.compile_dependency(&callee) {
                        if self.verbose {
                            eprintln!(
                                "[ovm] '{}' stays interpreted: cannot compile callee '{}'",
                                name, callee
                            );
                        }
                        self.reject(name);
                        return None;
                    }
                }
                Err(e) => {
                    if self.verbose {
                        eprintln!("[ovm] '{}' stays interpreted: {}", name, e);
                    }
                    self.reject(name);
                    return None;
                }
            }
        }

        if self.verbose {
            eprintln!(
                "[ovm] '{}' stays interpreted: dependency chain too deep",
                name
            );
        }
        self.reject(name);
        None
    }

    /// Compile a callee so the caller can resolve it. Returns whether the
    /// callee is now available to the VM.
    fn compile_dependency(&mut self, name: &str) -> bool {
        if self.compiled.contains_key(name) {
            return true;
        }
        if self.rejected.contains(name) || self.ambiguous.contains(name) {
            // An ambiguous callee can't be resolved by name; the caller that
            // needs it therefore can't be tiered either.
            return false;
        }

        let func = match self.known_functions.get(name) {
            Some(func) => func.clone(),
            // Not a user function we know about (a builtin the VM lacks, or a
            // value that isn't a plain function)
            None => return false,
        };

        self.compile(name, &func).is_some()
    }

    /// Assign a function id and make the name resolvable before compiling, so
    /// self- and mutual recursion can refer to it.
    fn register(&mut self, name: &str, func: &Function) -> Option<FunctionId> {
        // Default parameter values are evaluated by the interpreter
        if func.parameters.iter().any(|p| p.default_value.is_some()) {
            self.reject(name);
            return None;
        }

        let func_id = FunctionId::new();
        self.vm.register_function(name.to_string(), func_id);
        Some(func_id)
    }

    /// Build the declaration the VM compiles.
    ///
    /// Note: `func.closure` is NOT an eligibility signal — every user function
    /// captures the whole prelude, so it is never empty. The real gate is the
    /// compiler, which rejects a body referencing anything but its own
    /// parameters and locals.
    fn declaration(name: &str, func: &Function) -> Option<FunctionDecl> {
        Some(FunctionDecl {
            name_span: None,
            name: name.to_string(),
            type_params: Vec::new(),
            type_param_bounds: Vec::new(),
            parameters: func.parameters.clone(),
            return_type: None,
            body: (*func.body).clone(),
        })
    }

    fn reject(&mut self, name: &str) {
        // Withdraw the pre-compilation registration so nothing else resolves a
        // call against a name that has no bytecode
        self.vm.unregister_function(name);
        self.rejected.insert(name.to_string());
        self.compiled.remove(name);
        self.stats.rejected += 1;
    }

    /// A bare name has resolved to more than one distinct function body —
    /// two `impl` overrides of the same trait method, or a trait default and
    /// an override. The tier resolves and caches callees by name and cannot
    /// tell such bodies apart, so the name can never be tiered soundly: the
    /// interpreter runs it from here on, dispatching by receiver type. Mirrors
    /// the module-collision cleanup in `note_function` (including the wholesale
    /// `compiled.clear()`, since a previously compiled caller may have baked
    /// the now-withdrawn id).
    fn mark_ambiguous(&mut self, name: &str) {
        self.ambiguous.insert(name.to_string());
        self.known_functions.remove(name);
        self.call_counts.remove(name);
        self.rejected.remove(name);
        self.vm.unregister_function(name);
        self.compiled.clear();
    }

    /// Values the OVM model round-trips losslessly.
    ///
    /// Defers to the VM's definition rather than keeping a second copy: an
    /// earlier duplicate omitted Ok/Err, so every call passing a Result fell
    /// back even though Results round-trip fine.
    fn is_representable(value: &Value) -> bool {
        BytecodeVm::round_trips(value)
    }

    /// Convert one argument for the VM, or None if it isn't representable.
    /// Arc-backed lists hit the pointer-identity cache: the same (immutable)
    /// list converts once, not once per call. Correctness of a hit is
    /// guaranteed by the Weak upgrade — if the original allocation died, the
    /// upgrade fails and the entry is replaced; if it is alive, the address
    /// identifies exactly that list.
    fn convert_arg(&mut self, arg: &Value) -> Option<OvmValue> {
        if let Value::List(items) = arg {
            let key = Arc::as_ptr(items) as *const u8 as usize;
            if let Some((weak, cached)) = self.arg_cache.get(&key)
                && let Some(live) = weak.upgrade()
                && Arc::ptr_eq(&live, items)
            {
                return Some(cached.clone());
            }
            if !Self::is_representable(arg) {
                return None;
            }
            let converted = OvmValue::from_ast(arg.clone());
            // Bound the cache; wholesale clear is fine — entries repopulate
            // on the next call and hits dominate in steady state.
            if self.arg_cache.len() >= 512 {
                self.arg_cache.clear();
            }
            self.arg_cache
                .insert(key, (Arc::downgrade(items), converted.clone()));
            return Some(converted);
        }
        if !Self::is_representable(arg) {
            return None;
        }
        Some(OvmValue::from_ast(arg.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOp, Expr, Parameter};
    use std::sync::Arc;

    fn param(name: &str) -> Parameter {
        Parameter {
            name: name.to_string(),
            type_annotation: None,
            default_value: None,
        }
    }

    /// fn double(x) = x * 2
    fn double_fn() -> Function {
        Function {
            name: Some("double".to_string()),
            parameters: vec![param("x")],
            body: Arc::new(Expr::BinaryOp {
                left: Box::new(Expr::Identifier("x".to_string())),
                op: BinaryOp::Multiply,
                right: Box::new(Expr::Integer(2)),
            }),
            closure: Arc::new(im::HashMap::new()),
            param_bounds: Vec::new(),
        }
    }

    #[test]
    fn promotes_only_after_threshold() {
        let mut tier = BytecodeTier::new(3);
        let func = double_fn();

        for _ in 0..2 {
            assert!(matches!(
                tier.try_call(&func, &[Value::Integer(5)]),
                TierOutcome::Fallback
            ));
        }
        assert_eq!(tier.stats().promoted, 0);

        match tier.try_call(&func, &[Value::Integer(5)]) {
            TierOutcome::Ran(Ok(Value::Integer(10))) => {}
            other => panic!(
                "expected bytecode result 10, got {:?}",
                matches!(other, TierOutcome::Fallback)
            ),
        }
        assert_eq!(tier.stats().promoted, 1);
    }

    /// fn triple(x) = x * 3 — a distinct body under a name we can reuse.
    fn triple_fn() -> Function {
        Function {
            body: Arc::new(Expr::BinaryOp {
                left: Box::new(Expr::Identifier("x".to_string())),
                op: BinaryOp::Multiply,
                right: Box::new(Expr::Integer(3)),
            }),
            ..double_fn()
        }
    }

    #[test]
    fn same_name_distinct_bodies_are_never_tiered() {
        // Two modules declaring a function of the same name with different
        // bodies must not share a compilation — the tier resolves by name and
        // cannot tell them apart, so both fall back to the interpreter.
        let mut tier = BytecodeTier::new(1);
        let double = double_fn();
        let triple = triple_fn();

        tier.note_function("double".to_string(), double.clone());
        tier.note_function("double".to_string(), triple.clone());

        assert!(matches!(
            tier.try_call(&double, &[Value::Integer(5)]),
            TierOutcome::Fallback
        ));
        assert!(matches!(
            tier.try_call(&triple, &[Value::Integer(5)]),
            TierOutcome::Fallback
        ));
        assert_eq!(tier.stats().promoted, 0);
    }

    #[test]
    fn re_noting_the_same_body_still_tiers() {
        // Re-noting the exact same function (e.g. a module re-closing its
        // exports) shares the body Arc and must not be mistaken for a clash.
        let mut tier = BytecodeTier::new(1);
        let func = double_fn();
        tier.note_function("double".to_string(), func.clone());
        tier.note_function("double".to_string(), func.clone());

        match tier.try_call(&func, &[Value::Integer(5)]) {
            TierOutcome::Ran(Ok(Value::Integer(10))) => {}
            other => panic!(
                "same-body re-note must still tier; fell back: {}",
                matches!(other, TierOutcome::Fallback)
            ),
        }
    }

    #[test]
    fn prelude_capture_does_not_block_promotion() {
        // Every user function captures the prelude; that must not stop a
        // self-contained function from being promoted.
        let mut tier = BytecodeTier::new(1);
        let mut closure = im::HashMap::new();
        closure.insert("println".to_string(), Value::Unit);
        let func = Function {
            closure: Arc::new(closure),
            ..double_fn()
        };

        match tier.try_call(&func, &[Value::Integer(5)]) {
            TierOutcome::Ran(Ok(Value::Integer(10))) => {}
            _ => panic!("self-contained function should be promoted"),
        }
    }

    #[test]
    fn body_referencing_a_capture_compiles_it_as_a_constant() {
        // A free identifier resolving in the function's own closure bakes as
        // a constant — the closure is exactly the environment the interpreter
        // would install, and closures are snapshots. (This used to assert
        // rejection, back when any free identifier refused the function.)
        let mut tier = BytecodeTier::new(1);
        let mut closure = im::HashMap::new();
        closure.insert("captured".to_string(), Value::Integer(1));
        let func = Function {
            name: Some("uses_capture".to_string()),
            parameters: vec![param("x")],
            body: Arc::new(Expr::BinaryOp {
                left: Box::new(Expr::Identifier("x".to_string())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Identifier("captured".to_string())),
            }),
            closure: Arc::new(closure),
            param_bounds: Vec::new(),
        };

        match tier.try_call(&func, &[Value::Integer(5)]) {
            TierOutcome::Ran(Ok(Value::Integer(6))) => {}
            TierOutcome::Ran(other) => panic!("expected Ok(6), got {:?}", other),
            TierOutcome::Fallback => panic!("expected the capture to compile, got Fallback"),
        }
        assert_eq!(tier.stats().rejected, 0);
        assert_eq!(tier.stats().promoted, 1);
    }

    #[test]
    fn anonymous_functions_are_skipped() {
        let mut tier = BytecodeTier::new(1);
        let func = Function {
            name: None,
            ..double_fn()
        };
        assert!(matches!(
            tier.try_call(&func, &[Value::Integer(5)]),
            TierOutcome::Fallback
        ));
    }

    #[test]
    fn rejection_is_permanent() {
        let mut tier = BytecodeTier::new(1);
        // A body referencing an unknown global can't compile
        let func = Function {
            name: Some("bad".to_string()),
            parameters: vec![param("x")],
            body: Arc::new(Expr::Identifier("nonexistent_global".to_string())),
            closure: Arc::new(im::HashMap::new()),
            param_bounds: Vec::new(),
        };

        for _ in 0..5 {
            assert!(matches!(
                tier.try_call(&func, &[Value::Integer(1)]),
                TierOutcome::Fallback
            ));
        }
        // Compiled once, rejected once — never retried
        assert_eq!(tier.stats().rejected, 1);
    }

    #[test]
    fn non_representable_arguments_fall_back() {
        let mut tier = BytecodeTier::new(1);
        let func = double_fn();
        // Maps round-trip now; a TypeInfo value never converts, so a map
        // holding one is the durable non-representable specimen
        let mut bad = std::collections::HashMap::new();
        bad.insert(
            "t".to_string(),
            Value::TypeInfo {
                name: "X".to_string(),
                definition: crate::ast::TypeDefinition::Struct { fields: Vec::new() },
            },
        );
        let map_arg = Value::Map(std::sync::Arc::new(bad));
        assert!(matches!(
            tier.try_call(&func, &[map_arg]),
            TierOutcome::Fallback
        ));
    }

    #[test]
    fn function_arguments_cross_the_boundary_and_error_like_the_interpreter() {
        // Function values round-trip now (AstFunction wraps them verbatim),
        // so double(<function>) runs on the VM — and multiplying a function
        // by 2 is a type error there exactly as it is interpreted.
        let mut tier = BytecodeTier::new(1);
        let func = double_fn();
        match tier.try_call(&func, &[Value::Function(double_fn())]) {
            TierOutcome::Ran(Err(_)) => {}
            TierOutcome::Ran(Ok(v)) => panic!("expected a type error, got {:?}", v),
            TierOutcome::Fallback => panic!("function arguments should cross the boundary"),
        }
    }

    #[test]
    fn runtime_errors_propagate() {
        let mut tier = BytecodeTier::new(1);
        let func = Function {
            name: Some("div".to_string()),
            parameters: vec![param("a"), param("b")],
            body: Arc::new(Expr::BinaryOp {
                left: Box::new(Expr::Identifier("a".to_string())),
                op: BinaryOp::Divide,
                right: Box::new(Expr::Identifier("b".to_string())),
            }),
            closure: Arc::new(im::HashMap::new()),
            param_bounds: Vec::new(),
        };

        match tier.try_call(&func, &[Value::Integer(1), Value::Integer(0)]) {
            TierOutcome::Ran(Err(_)) => {}
            _ => panic!("division by zero should surface as an error"),
        }
    }
}

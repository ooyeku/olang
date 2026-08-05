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
}

pub struct BytecodeTier {
    vm: BytecodeVm,
    threshold: u32,
    call_counts: HashMap<String, u32>,
    compiled: HashMap<String, FunctionId>,
    /// Names that failed compilation — never retried
    rejected: HashSet<String>,
    /// User functions the interpreter has declared, so a promoted function
    /// calling a helper can have that helper compiled too
    known_functions: HashMap<String, Function>,
    stats: TierStats,
    /// Emit a line when a function is promoted (for --ovm-stats / debugging)
    verbose: bool,
}

impl BytecodeTier {
    pub fn new(threshold: u32) -> Self {
        Self {
            vm: BytecodeVm::new(),
            threshold,
            call_counts: HashMap::new(),
            compiled: HashMap::new(),
            rejected: HashSet::new(),
            known_functions: HashMap::new(),
            stats: TierStats::default(),
            verbose: false,
        }
    }

    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    pub fn stats(&self) -> TierStats {
        self.stats
    }

    /// Record a user function declaration so calls to it can be compiled.
    pub fn note_function(&mut self, name: String, func: Function) {
        // A redefinition invalidates anything compiled against the old body
        if self.compiled.remove(&name).is_some() || self.rejected.remove(&name) {
            // Compiled code may have inlined nothing, but callers resolved the
            // old id; drop everything so the next call recompiles cleanly.
            self.compiled.clear();
            self.rejected.clear();
        }
        // A user definition shadows any builtin of the same name
        self.vm.shadow_builtin(&name);
        self.known_functions.insert(name, func);
    }

    /// Try to execute `func(args)` on the bytecode VM.
    pub fn try_call(&mut self, func: &Function, args: &[Value]) -> TierOutcome {
        let name = match &func.name {
            Some(name) => name.clone(),
            // Anonymous lambdas have no stable identity to profile
            None => return TierOutcome::Fallback,
        };

        if self.rejected.contains(&name) {
            return TierOutcome::Fallback;
        }

        let func_id = match self.compiled.get(&name) {
            Some(id) => *id,
            None => {
                let count = self.call_counts.entry(name.clone()).or_insert(0);
                *count += 1;
                if *count < self.threshold {
                    return TierOutcome::Fallback;
                }
                match self.compile(&name, func) {
                    Some(id) => id,
                    None => return TierOutcome::Fallback,
                }
            }
        };

        // Arguments must round-trip through the OVM value model
        let mut ovm_args = Vec::with_capacity(args.len());
        for arg in args {
            if !Self::is_representable(arg) {
                return TierOutcome::Fallback;
            }
            ovm_args.push(OvmValue::from_ast(arg.clone()));
        }

        self.stats.bytecode_calls += 1;
        match self.vm.execute(func_id, &ovm_args) {
            Ok(value) => match value.to_ast() {
                Ok(ast) => TierOutcome::Ran(Ok(ast)),
                // A result we can't convert would be observable as a wrong
                // value; refuse rather than return something else.
                Err(_) => {
                    self.reject(&name);
                    TierOutcome::Fallback
                }
            },
            Err(e) => TierOutcome::Ran(Err(e.to_string())),
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
                    self.compiled.insert(name.to_string(), func_id);
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
        if self.rejected.contains(name) {
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
            name: name.to_string(),
            type_params: Vec::new(),
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

    /// Values the OVM model round-trips losslessly.
    ///
    /// Defers to the VM's definition rather than keeping a second copy: an
    /// earlier duplicate omitted Ok/Err, so every call passing a Result fell
    /// back even though Results round-trip fine.
    fn is_representable(value: &Value) -> bool {
        BytecodeVm::round_trips(value)
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
    fn body_referencing_a_capture_is_rejected() {
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
        };

        assert!(matches!(
            tier.try_call(&func, &[Value::Integer(5)]),
            TierOutcome::Fallback
        ));
        assert_eq!(tier.stats().rejected, 1);
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
        // Passing a function value must not go to the VM
        assert!(matches!(
            tier.try_call(&func, &[Value::Function(double_fn())]),
            TierOutcome::Fallback
        ));
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
        };

        match tier.try_call(&func, &[Value::Integer(1), Value::Integer(0)]) {
            TierOutcome::Ran(Err(_)) => {}
            _ => panic!("division by zero should surface as an error"),
        }
    }
}

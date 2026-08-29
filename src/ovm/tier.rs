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

/// The allocation a cached conversion belongs to, held weakly.
///
/// A cache keyed on an address alone is unsound: the allocation can die
/// and a new one land at the same address. Holding a `Weak` to the exact
/// original and re-checking pointer identity on every hit makes a stale
/// entry impossible — a dead allocation fails to upgrade, and a live one
/// at a reused address fails `ptr_eq`.
///
/// One variant per Arc-backed compound `Value`, because the weak type
/// differs per payload. A value whose contents could change behind the
/// pointer must never be listed here; every variant below is immutable.
enum CachedOwner {
    Tuple(Weak<Vec<Value>>),
    Map(Weak<std::collections::HashMap<String, Value>>),
    Struct(Weak<std::collections::HashMap<String, Value>>),
}

impl CachedOwner {
    /// The cache key and the weak handle, for a value worth caching.
    /// `None` for scalars, where conversion is already trivial and an
    /// entry would cost more than it saves.
    fn of(value: &Value) -> Option<(usize, CachedOwner)> {
        match value {
            // Lists are deliberately absent: they convert in O(1) (the
            // AstList wrap) and they are the one value the in-place
            // fusions mutate under a stable address — uncacheable by
            // this scheme.
            Value::Tuple(items) => Some((
                Arc::as_ptr(items) as *const u8 as usize,
                CachedOwner::Tuple(Arc::downgrade(items)),
            )),
            Value::Map(fields) => Some((
                Arc::as_ptr(fields) as *const u8 as usize,
                CachedOwner::Map(Arc::downgrade(fields)),
            )),
            Value::Struct { fields, .. } => Some((
                Arc::as_ptr(fields) as *const u8 as usize,
                CachedOwner::Struct(Arc::downgrade(fields)),
            )),
            _ => None,
        }
    }

    /// Is this cache entry still about exactly this value? Variant *and*
    /// allocation must match: two different `Value`s can share an address
    /// only if one is dead, and a struct must not answer for a map that
    /// happens to sit where it used to.
    fn still_is(&self, value: &Value) -> bool {
        match (self, value) {
            (CachedOwner::Tuple(w), Value::Tuple(items)) => {
                w.upgrade().is_some_and(|live| Arc::ptr_eq(&live, items))
            }
            (CachedOwner::Map(w), Value::Map(fields))
            | (CachedOwner::Struct(w), Value::Struct { fields, .. }) => {
                w.upgrade().is_some_and(|live| Arc::ptr_eq(&live, fields))
            }
            _ => false,
        }
    }
}

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
    /// Calls that ran as native (JIT) code. Zero when nothing qualified —
    /// including trivial constructors, which decline the call boundary on
    /// purpose (a native body that only allocates cannot pay for the
    /// marshalling around it).
    pub jit_native_calls: u64,
}

pub struct BytecodeTier {
    vm: BytecodeVm,
    threshold: u32,
    /// Warm-start hints from a previous run of this source: function
    /// name → the scalar kind names its specialization served (empty =
    /// compile eagerly, specialize on first call as usual). Kept as
    /// strings so the tier builds without the native JIT; parsed at
    /// apply time. Applied once per name, when the declaration is noted.
    warm_hints: HashMap<String, Vec<String>>,
    /// The trace of the most recent Ran(Err(..)) outcome: the innermost
    /// located statement's span, the VM call frames (innermost-first),
    /// and a parameter-check frame that survived without a span. The
    /// interpreter fetches it right after mapping the error message.
    last_error_trace: (Option<(u32, u32)>, Vec<String>, Option<String>),
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
    /// Why each rejected name stays interpreted — the raw compiler
    /// message, kept so `:ovm` can turn "0 rejected, 0 promoted" into
    /// something a developer can act on.
    rejected_reasons: HashMap<String, String>,
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
    /// Pointer-identity cache for argument conversion. Every Arc-backed
    /// compound value is immutable, so one already converted (and validated
    /// representable) converts for free on every later call — a hot
    /// function taking a large value no longer pays a deep Value->OvmValue
    /// walk per call. Entries are keyed by allocation address and validated
    /// with a Weak upgrade, so a freed-and-reused address can never produce
    /// a stale hit.
    ///
    /// This covered lists only, which made a struct argument 20x more
    /// expensive than the same data in a list: passing a struct holding
    /// 5,000 elements to a hot function cost 84ms against a list's 1ms,
    /// entirely in re-converting the same unchanged value 2,000 times.
    /// Every variant with a stable allocation now shares the cache.
    arg_cache: HashMap<usize, (CachedOwner, OvmValue)>,
}

impl BytecodeTier {
    /// The promotion threshold this tier was built with — used to give
    /// worker-thread interpreters a fresh tier with the same policy.
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    pub fn new(threshold: u32) -> Self {
        Self {
            last_error_trace: (None, Vec::new(), None),
            vm: BytecodeVm::new(),
            threshold,
            warm_hints: HashMap::new(),
            call_counts: HashMap::new(),
            compiled: HashMap::new(),
            rejected: HashSet::new(),
            rejected_reasons: HashMap::new(),
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

    /// Install the run's capability table. Passed straight down to the
    /// VM, which seeds it into the bridge interpreter that dispatches
    /// builtins, so a promoted function is gated exactly as the
    /// interpreted one was.
    pub fn set_max_call_depth(&mut self, depth: u32) {
        self.vm.set_max_call_depth(depth);
    }

    pub fn set_capabilities(&mut self, caps: Option<Arc<crate::caps::CapTable>>) {
        self.vm.set_capabilities(caps);
    }

    /// Share the `--trace-caps` set, so effects a promoted function
    /// performs land in the same profile as interpreted ones.
    pub fn set_caps_trace(
        &mut self,
        trace: Arc<std::sync::Mutex<std::collections::BTreeSet<crate::caps::CapUse>>>,
    ) {
        self.vm.set_caps_trace(trace);
    }

    /// Every rejected name with the reason it stays interpreted, sorted.
    pub fn rejections(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = self
            .rejected
            .iter()
            .map(|n| {
                let why = self
                    .rejected_reasons
                    .get(n)
                    .cloned()
                    .unwrap_or_else(|| "reason not recorded".to_string());
                (n.clone(), why)
            })
            .collect();
        rows.sort();
        rows
    }

    /// Names compiled to bytecode this session, sorted.
    pub fn compiled_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.compiled.keys().cloned().collect();
        names.sort();
        names
    }

    /// Names the tier can never dispatch because two distinct bodies share
    /// them (trait default + override); the interpreter resolves these by
    /// receiver type — correct, and worth *seeing*.
    pub fn ambiguous_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.ambiguous.iter().cloned().collect();
        names.sort();
        names
    }

    /// Functions counting toward the promotion threshold but not yet
    /// compiled or rejected: hot-in-waiting, with their call counts.
    pub fn pending_counts(&self) -> Vec<(String, u32)> {
        let mut rows: Vec<(String, u32)> = self
            .call_counts
            .iter()
            .filter(|(n, _)| !self.compiled.contains_key(*n) && !self.rejected.contains(*n))
            .map(|(n, c)| (n.clone(), *c))
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        rows
    }

    /// How many warm-start hints this tier was seeded with.
    pub fn warm_hint_count(&self) -> usize {
        self.warm_hints.len()
    }

    /// The VM's own execution counters (instructions, calls, cache
    /// traffic, compile/execute time, OSR loop entries).
    pub fn vm_statistics(&self) -> &crate::ovm::bytecode::VmStatistics {
        self.vm.statistics()
    }

    pub fn stats(&self) -> TierStats {
        // The instruction counter lives on the VM, not in the tier's own
        // tally, so it is read through at reporting time.
        TierStats {
            instructions_executed: self.vm.instructions_executed(),
            jit_native_calls: self.vm.jit_native_calls(),
            // "Promoted" = named functions compiled to the tier, whichever
            // channel compiled them — direct promotion or a lambda's
            // dependency resolution.
            promoted: self.stats.promoted + self.vm.hof_promotions(),
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

    /// Mirror a declared enum's type name so the tier recognizes it in an
    /// annotation (and reports a name that is *not* declared as unknown).
    pub fn note_enum_type(&mut self, name: String) {
        self.vm.note_enum_type(name);
    }

    /// Record a user function declaration so calls to it can be compiled.
    /// Install a previous run's warm profile. Only entries whose native
    /// call count proved the work pays are kept, and kinds survive only
    /// when every one is a process-independent scalar.
    pub fn set_warm_profile(&mut self, profile: &crate::ovm::warm::WarmProfile) {
        for f in &profile.functions {
            if f.native_calls < crate::ovm::warm::WARM_MIN_CALLS {
                continue;
            }
            let all_scalar = !f.kinds.is_empty()
                && f.kinds
                    .iter()
                    .all(|k| matches!(k.as_str(), "Int" | "Float" | "Bool"));
            self.warm_hints.insert(
                f.name.clone(),
                if all_scalar {
                    f.kinds.clone()
                } else {
                    Vec::new()
                },
            );
        }
    }

    /// What this run learned, for the next one: every name-compiled
    /// function that served native calls, with its specialization kinds.
    pub fn collect_warm_profile(&self) -> crate::ovm::warm::WarmProfile {
        #[cfg(not(feature = "native"))]
        {
            crate::ovm::warm::WarmProfile::default()
        }
        #[cfg(feature = "native")]
        {
            use crate::ovm::jit::Kind;
            let mut functions = Vec::new();
            for (name, (func_id, _)) in &self.compiled {
                let Some((kinds, native_calls)) = self.vm.jit_warm_view(*func_id) else {
                    continue;
                };
                if native_calls == 0 {
                    continue;
                }
                let kind_names: Vec<String> = kinds
                    .iter()
                    .map(|k| match k {
                        Kind::Int => "Int".to_string(),
                        Kind::Float => "Float".to_string(),
                        Kind::Bool => "Bool".to_string(),
                        other => format!("{:?}", other),
                    })
                    .collect();
                functions.push(crate::ovm::warm::WarmFn {
                    name: name.clone(),
                    kinds: kind_names,
                    native_calls,
                });
            }
            crate::ovm::warm::WarmProfile { functions }
        }
    }

    /// The per-function tier report behind `OLANG_TIER_STATS=1`: every
    /// name the VM can call, with the native calls its JIT entry served
    /// and its specialization kinds. Unlike the warm profile this is
    /// unfiltered — a compiled function with zero native calls is the
    /// interesting row, because it names exactly what the JIT declined.
    /// Sorted by name so runs of the same program diff cleanly.
    pub fn tier_report(&self) -> Vec<(String, u64, Vec<String>)> {
        #[cfg(not(feature = "native"))]
        {
            Vec::new()
        }
        #[cfg(feature = "native")]
        {
            use crate::ovm::jit::Kind;
            let mut rows: Vec<(String, u64, Vec<String>)> = self
                .vm
                .registered_functions()
                .map(|(name, func_id)| {
                    let (kinds, native_calls) = self.vm.jit_warm_view(*func_id).unwrap_or_default();
                    let kind_names = kinds
                        .iter()
                        .map(|k| match k {
                            Kind::Int => "Int".to_string(),
                            Kind::Float => "Float".to_string(),
                            Kind::Bool => "Bool".to_string(),
                            other => format!("{:?}", other),
                        })
                        .collect();
                    (name.clone(), native_calls, kind_names)
                })
                .collect();
            rows.sort();
            rows
        }
    }

    pub fn note_function(&mut self, name: String, func: Function) {
        // Already known to be ambiguous: a second module's same-named function
        // stays off the tier for the rest of the run.
        if self.ambiguous.contains(&name) {
            return;
        }

        if let Some(existing) = self.known_functions.get(&name) {
            // The same declaration re-noted — which is exactly what a
            // module does when it re-closes its exports over the full
            // module scope. The body is unchanged but the CLOSURE may
            // now carry siblings declared later in the file, so refresh
            // the recorded value: the bridge and the compiler's escaped
            // forms resolve bare names through it, and a stale
            // declaration-time closure loses `expr`-style mutual
            // references ("Undefined variable" from a path that worked
            // interpreted).
            if Arc::ptr_eq(&existing.body, &func.body) {
                if !Arc::ptr_eq(&existing.closure, &func.closure) {
                    self.vm.note_function_value(name.clone(), func.clone());
                    self.known_functions.insert(name, func);
                }
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
            self.rejected_reasons.remove(&name);
            self.vm.unregister_function(&name);
            self.compiled.clear();
            return;
        }

        // A user definition shadows any builtin of the same name
        self.vm.shadow_builtin(&name);
        self.vm.note_function_value(name.clone(), func.clone());

        // Warm start: a previous run of this exact source proved this
        // function hot, so do at declaration time what that run did at
        // first call — compile it, and when the recorded kinds are
        // scalars, specialize the native code too. A hint that no
        // longer compiles costs one refused attempt, nothing more.
        if let Some(kind_names) = self.warm_hints.remove(&name)
            && !self.compiled.contains_key(&name)
            && let Some(func_id) = self.compile(&name, &func)
            && !kind_names.is_empty()
        {
            #[cfg(feature = "native")]
            {
                let kinds: Vec<crate::ovm::jit::Kind> = kind_names
                    .iter()
                    .filter_map(|k| match k.as_str() {
                        "Int" => Some(crate::ovm::jit::Kind::Int),
                        "Float" => Some(crate::ovm::jit::Kind::Float),
                        "Bool" => Some(crate::ovm::jit::Kind::Bool),
                        _ => None,
                    })
                    .collect();
                if kinds.len() == kind_names.len() {
                    self.vm.warm_specialize(func_id, &kinds);
                }
            }
            #[cfg(not(feature = "native"))]
            let _ = func_id;
        }

        self.known_functions.insert(name, func);
    }

    /// The trace of the most recent Ran(Err(..)): span + frames
    /// (innermost-first), reset on take.
    pub fn take_error_trace(&mut self) -> (Option<(u32, u32)>, Vec<String>, Option<String>) {
        std::mem::take(&mut self.last_error_trace)
    }

    /// Try to execute `func(args)` on the bytecode VM. `depth_base` is
    /// the interpreter's current call depth, seeded into the VM so both
    /// tiers spend from the one shared budget (see `set_depth_base`).
    pub fn try_call_at_depth(
        &mut self,
        func: &Function,
        args: &mut [Value],
        depth_base: u32,
    ) -> TierOutcome {
        // The caller passes its depth *including* the frame it counted
        // for this very call — the frame the VM is about to push and
        // count again. Seed one less, or the boundary frame is charged
        // twice and a recursion exactly at the cap dies one frame early.
        self.vm.set_depth_base(depth_base.saturating_sub(1));
        self.try_call(func, args)
    }

    /// Modules pinned to the tree-walking interpreter. Collections lived
    /// here until the T2 argument-conversion lane made the VM the faster
    /// home (2–7× across the bundled benchmarks); the list survives as an
    /// escape hatch — OLANG_COLLECTIONS_ON_INTERP=1 restores the old
    /// placement for A/B measurement.
    const INTERPRETER_RESIDENT: &'static [&'static str] = &[
        "__embedded__/heap",
        "__embedded__/deque",
        "__embedded__/bitset",
        "__embedded__/dsu",
        "__embedded__/table",
        "__embedded__/alg",
    ];

    /// Try to execute `func(args)` on the bytecode VM.
    pub fn try_call(&mut self, func: &Function, args: &mut [Value]) -> TierOutcome {
        if std::env::var_os("OLANG_COLLECTIONS_ON_INTERP").is_some()
            && let Some(def_file) = func.def_file.as_deref()
            && Self::INTERPRETER_RESIDENT.contains(&def_file)
        {
            return TierOutcome::Fallback;
        }
        // Borrowed, not cloned: `func` is the caller's, independent of
        // `self`, so the lookups below need no owned copy. This used to
        // allocate a String on every call of every named function purely to
        // probe three maps with it.
        let name: &str = match &func.name {
            Some(name) => name,
            // An anonymous lambda has no name to profile by — but it has
            // identity: its body and closure Arcs, exactly what the HOF
            // cache keys on. Compile and run by identity, the same route
            // ambiguous names take. This is what lets a lambda kernel
            // (`map((x) => ...)`, a par_map worker's function) run on the
            // compiled tiers instead of tree-walking every element; a
            // body the tier cannot take caches its refusal, so the cost
            // of an uncompilable lambda is one attempt, ever.
            None => {
                let arity = func.parameters.len();
                return match self.vm.hof_function_id(func, arity) {
                    Some(func_id) => self.run_on_vm(func_id, args, None),
                    None => TierOutcome::Fallback,
                };
            }
        };

        if self.rejected.contains(name) {
            return TierOutcome::Fallback;
        }

        // A name shared by two distinct functions can't be dispatched *by
        // name*, but the caller handed us the specific body — so compile and
        // run it by identity (its body pointer), bypassing the name and its
        // ambiguity entirely. This is what lets a user function whose name
        // collides with an embedded package's private helper (`viz`'s `col`,
        // `opt`, …) still promote instead of dropping the whole program to
        // the interpreter. `hof_function_id` compiles under the function's
        // own closure and caches per body, and returns None for bodies the
        // tier can't take (arity/defaults/bounds) — an honest fallback.
        if self.ambiguous.contains(name) {
            let arity = func.parameters.len();
            return match self.vm.hof_function_id(func, arity) {
                Some(func_id) => self.run_on_vm(func_id, args, None),
                None => TierOutcome::Fallback,
            };
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

        self.run_on_vm(func_id, args, Some(name))
    }

    /// Run a whole `map`/`filter` on the VM: the list converts once, the
    /// native loop runs (kernel compiled by identity, JIT included), and
    /// the result converts once. Returns None when the VM declines —
    /// the caller's per-element path is the unchanged fallback. This is
    /// what keeps a top-level `xs |> map((n) => ...)` off the
    /// per-element boundary: the interpreter used to cross into the
    /// tier once per item, and the crossing cost more than the kernel.
    pub fn try_hof(
        &mut self,
        name: &str,
        kernel: &crate::ast::Function,
        items: &[Value],
        init: Option<&Value>,
    ) -> Option<Result<Value, String>> {
        let mut ovm_items = Vec::with_capacity(items.len());
        for v in items {
            ovm_items.push(crate::ovm::value::OvmValue::from_ast(v.clone()));
        }
        let mut args = vec![crate::ovm::value::OvmValue::new_list(ovm_items)];
        if let Some(v) = init {
            args.push(crate::ovm::value::OvmValue::from_ast(v.clone()));
        }
        args.push(crate::ovm::value::OvmValue::from_ast(Value::Function(
            kernel.clone(),
        )));
        let out = self.vm.native_hof(name, &args);
        if std::env::var_os("OLANG_DEBUG_HOF").is_some() {
            eprintln!(
                "[hof] tier {} n={} -> {}",
                name,
                items.len(),
                match &out {
                    None => "declined",
                    Some(Ok(_)) => "ran",
                    Some(Err(_)) => "error",
                }
            );
        }
        let out = out?;
        self.stats.bytecode_calls += 1;
        Some(match out {
            Ok(value) => match value.to_ast() {
                Ok(ast) => Ok(ast),
                Err(_) => return None,
            },
            Err(e) => Err(format!("{}", e)),
        })
    }

    /// Convert the arguments, run the compiled body on the VM, and convert
    /// the result back — the shared tail of every tiered call, whether the
    /// callee was resolved by name or (for an ambiguous name) by identity.
    /// `reject_name` is `Some` only for the name-dispatched path: a result
    /// the VM value model can't represent poisons that name so it isn't
    /// retried. The identity path passes `None` — there is no name to poison.
    fn run_on_vm(
        &mut self,
        func_id: FunctionId,
        args: &mut [Value],
        reject_name: Option<&str>,
    ) -> TierOutcome {
        // Arguments must round-trip through the OVM value model. Lists
        // move: the slot is taken (left Unit) so the wrapped Arc crosses
        // solely owned and the VM's in-place writes stay in place — the
        // boundary twin of the interpreter's by-move argument binding.
        // On a conversion refusal the taken lists are restored (an O(1)
        // unwrap each), so the interpreter's fallback sees its arguments
        // intact.
        let mut ovm_args = Vec::with_capacity(args.len());
        for i in 0..args.len() {
            match self.convert_arg(&mut args[i]) {
                Some(v) => ovm_args.push(v),
                None => {
                    for (slot, converted) in args.iter_mut().zip(ovm_args.iter()) {
                        if matches!(slot, Value::Unit)
                            && let Ok(back) = converted.to_ast()
                            && !matches!(back, Value::Unit)
                        {
                            *slot = back;
                        }
                    }
                    return TierOutcome::Fallback;
                }
            }
        }

        self.stats.bytecode_calls += 1;
        self.vm.clear_error_trace();
        match self.vm.execute_taking(func_id, &mut ovm_args) {
            Ok(value) => match value.to_ast() {
                Ok(ast) => TierOutcome::Ran(Ok(ast)),
                // A result we can't convert would be observable as a wrong
                // value; refuse rather than return something else.
                Err(_) => {
                    if let Some(name) = reject_name {
                        self.reject(
                            name,
                            "returned a value the tier boundary cannot convert back",
                        );
                    }
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
                self.last_error_trace = self.vm.take_error_trace();
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
                    self.reject(
                        name,
                        "its declaration shape cannot be rebuilt for the compiler",
                    );
                    return None;
                }
            };

            match self.vm.compile_function_with_closure(
                func_id,
                &decl,
                func.closure.clone(),
                func.param_checks.clone().into(),
                func.return_check.clone(),
                func.def_file.as_deref().map(Arc::from),
            ) {
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
                        self.reject(name, format!("calls '{}', which cannot compile", callee));
                        return None;
                    }
                }
                Err(e) => {
                    if self.verbose {
                        eprintln!("[ovm] '{}' stays interpreted: {}", name, e);
                    }
                    let reason = e.to_string();
                    self.reject(name, reason);
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
        self.reject(name, "its dependency chain is too deep to resolve");
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
            self.reject(
                name,
                "has default parameter values (defaults are evaluated by the interpreter)",
            );
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

    fn reject(&mut self, name: &str, reason: impl Into<String>) {
        // Withdraw the pre-compilation registration so nothing else resolves a
        // call against a name that has no bytecode
        self.vm.unregister_function(name);
        self.rejected.insert(name.to_string());
        self.rejected_reasons
            .insert(name.to_string(), reason.into());
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
        self.rejected_reasons.remove(name);
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
    /// Arc-backed compound values hit the pointer-identity cache: the same
    /// (immutable) value converts once, not once per call. Correctness of a
    /// hit is guaranteed by the Weak upgrade — if the original allocation
    /// died, the upgrade fails and the entry is replaced; if it is alive,
    /// the address identifies exactly that value.
    fn convert_arg(&mut self, arg: &mut Value) -> Option<OvmValue> {
        // Lists convert in O(1) — small ones copy a handful of elements,
        // large ones wrap as AstList sharing the interpreter's Arc — and
        // they convert *by move*: the caller's slot is taken, so nothing
        // on the interpreter side pins the Arc and the VM's sole-owner
        // writes actually run in place. They also skip both the
        // representability scan (the wrap is total; elements convert as
        // they are touched) and the identity cache — skipping the cache
        // is a soundness fix, since the in-place fusions mutate a list
        // under an unchanged address, exactly what an address-keyed
        // cache of conversions cannot detect.
        if matches!(arg, Value::List(_)) {
            let owned = std::mem::replace(arg, Value::Unit);
            if std::env::var_os("OLANG_DEBUG_ASTLIST").is_some()
                && let Value::List(items) = &owned
                && items.len() > 64
            {
                eprintln!(
                    "[astlist] boundary rc={} len={}",
                    std::sync::Arc::strong_count(items),
                    items.len()
                );
            }
            return Some(OvmValue::from_ast(owned));
        }
        if let Some((key, owner)) = CachedOwner::of(arg) {
            if let Some((cached_owner, cached)) = self.arg_cache.get(&key)
                && cached_owner.still_is(arg)
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
            self.arg_cache.insert(key, (owner, converted.clone()));
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
    use super::CachedOwner;

    /// The cache is keyed on an address, which is only sound because a
    /// `Weak` proves the entry still describes *that* allocation. These
    /// pin the two ways it could go wrong.
    #[test]
    fn a_cached_owner_recognises_its_own_value() {
        // Tuples stand in for the cached compound values; lists are
        // deliberately uncacheable (they convert in O(1) and the
        // in-place fusions mutate them under a stable address).
        let tuple = Value::Tuple(Arc::new(vec![Value::Integer(1)]));
        let (_, owner) = CachedOwner::of(&tuple).expect("tuples are cached");
        assert!(owner.still_is(&tuple));

        let other = Value::Tuple(Arc::new(vec![Value::Integer(1)]));
        assert!(
            !owner.still_is(&other),
            "an equal-but-distinct allocation must not hit"
        );

        let list = Value::List(Arc::new(vec![Value::Integer(1)]));
        assert!(
            CachedOwner::of(&list).is_none(),
            "lists must never enter the conversion cache"
        );
    }

    #[test]
    fn a_cached_owner_rejects_a_different_variant() {
        // A struct and a map share a payload type, so without the variant
        // check a struct entry could answer for a map at a reused address.
        let mut fields = std::collections::HashMap::new();
        fields.insert("a".to_string(), Value::Integer(1));
        let fields = Arc::new(fields);
        let as_struct = Value::Struct {
            type_name: "T".to_string(),
            fields: fields.clone(),
        };
        let as_map = Value::Map(fields);
        let (skey, owner) = CachedOwner::of(&as_struct).expect("structs are cached");
        let (mkey, _) = CachedOwner::of(&as_map).expect("maps are cached");
        assert_eq!(skey, mkey, "same allocation, so the addresses match");
        assert!(owner.still_is(&as_struct));
        assert!(
            !owner.still_is(&as_map),
            "the variant must be part of the identity"
        );
    }

    #[test]
    fn a_cached_owner_goes_stale_when_its_value_dies() {
        let owner = {
            let tuple = Value::Tuple(Arc::new(vec![Value::Integer(1)]));
            CachedOwner::of(&tuple).expect("cached").1
        };
        let fresh = Value::Tuple(Arc::new(vec![Value::Integer(1)]));
        assert!(
            !owner.still_is(&fresh),
            "a dead allocation must never produce a hit"
        );
    }

    /// Structs must cross the tier boundary as cheaply as lists. They did
    /// not: the conversion cache covered lists only, so a struct argument
    /// re-converted its whole payload on every call — 84ms against a
    /// list's 1ms for the same 5,000 elements. The bound is deliberately
    /// loose (a timing test that is tight is a flaky test); the regression
    /// it guards was 80x, not 8x.
    #[test]
    fn a_struct_argument_is_not_dramatically_worse_than_a_list() {
        use crate::Interpreter;
        use crate::Parser;

        let source = r#"
type Box = struct { payload: [Int] }
let mut big = []
for i in 0..2000 { big = big + [i] }
let boxed = Box { payload: big }
let listed = [big]
fn touch_s(b) = len(b.payload)
fn touch_l(l) = len(l[0])
let t0 = time.monotonic_ms()
let mut a = 0
for i in 0..500 { a = a + touch_s(boxed) }
let t1 = time.monotonic_ms()
let mut b = 0
for i in 0..500 { b = b + touch_l(listed) }
let t2 = time.monotonic_ms()
[t1 - t0, t2 - t1]
"#;
        let program = Parser::new().parse(source).expect("parses");
        let mut interpreter = Interpreter::new();
        interpreter.enable_bytecode_tier(2, false);
        let out = interpreter.eval_program(program).expect("runs");
        let (struct_ms, list_ms) = match &out {
            Value::List(v) => match (&v[0], &v[1]) {
                (Value::Integer(s), Value::Integer(l)) => (*s, *l),
                _ => panic!("expected two timings, got {out:?}"),
            },
            _ => panic!("expected a list, got {out:?}"),
        };
        assert!(
            struct_ms <= list_ms.max(4) * 8,
            "a struct argument cost {struct_ms}ms against a list's {list_ms}ms — \
             the tier conversion cache has stopped covering structs"
        );
    }

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
            param_checks: Vec::new(),
            return_check: None,
            def_file: None,
        }
    }

    #[test]
    fn promotes_only_after_threshold() {
        let mut tier = BytecodeTier::new(3);
        let func = double_fn();

        for _ in 0..2 {
            assert!(matches!(
                tier.try_call(&func, &mut [Value::Integer(5)]),
                TierOutcome::Fallback
            ));
        }
        assert_eq!(tier.stats().promoted, 0);

        match tier.try_call(&func, &mut [Value::Integer(5)]) {
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
    fn same_name_distinct_bodies_tier_by_identity() {
        // Two functions declaring the same name with different bodies (a user
        // function shadowing an embedded package's private helper) can't be
        // dispatched *by name* — that stays ambiguous. But the caller hands
        // `try_call` the specific body, so each is compiled and run by
        // identity and returns its OWN correct result, instead of both being
        // dropped to the interpreter (viz finding #3).
        let mut tier = BytecodeTier::new(1);
        let double = double_fn();
        let triple = triple_fn();

        tier.note_function("double".to_string(), double.clone());
        tier.note_function("double".to_string(), triple.clone());

        // double(5) = 10, triple(5) = 15 — the right body each time.
        assert!(matches!(
            tier.try_call(&double, &mut [Value::Integer(5)]),
            TierOutcome::Ran(Ok(Value::Integer(10)))
        ));
        assert!(matches!(
            tier.try_call(&triple, &mut [Value::Integer(5)]),
            TierOutcome::Ran(Ok(Value::Integer(15)))
        ));
    }

    #[test]
    fn re_noting_the_same_body_still_tiers() {
        // Re-noting the exact same function (e.g. a module re-closing its
        // exports) shares the body Arc and must not be mistaken for a clash.
        let mut tier = BytecodeTier::new(1);
        let func = double_fn();
        tier.note_function("double".to_string(), func.clone());
        tier.note_function("double".to_string(), func.clone());

        match tier.try_call(&func, &mut [Value::Integer(5)]) {
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

        match tier.try_call(&func, &mut [Value::Integer(5)]) {
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
            param_checks: Vec::new(),
            return_check: None,
            def_file: None,
        };

        match tier.try_call(&func, &mut [Value::Integer(5)]) {
            TierOutcome::Ran(Ok(Value::Integer(6))) => {}
            TierOutcome::Ran(other) => panic!("expected Ok(6), got {:?}", other),
            TierOutcome::Fallback => panic!("expected the capture to compile, got Fallback"),
        }
        assert_eq!(tier.stats().rejected, 0);
        assert_eq!(tier.stats().promoted, 1);
    }

    #[test]
    fn anonymous_functions_promote_by_identity() {
        // A lambda has no name to profile, but its body/closure Arcs are
        // identity enough: it compiles through the HOF cache and RUNS on
        // the tier. This used to assert the opposite — anonymous kernels
        // fell back to the tree walk, which is why `map((x) => ...)` and
        // every par_map lambda ran an order of magnitude slower than the
        // same function with a name.
        let mut tier = BytecodeTier::new(1);
        let func = Function {
            name: None,
            ..double_fn()
        };
        match tier.try_call(&func, &mut [Value::Integer(5)]) {
            TierOutcome::Ran(Ok(v)) => assert_eq!(v, Value::Integer(10)),
            TierOutcome::Ran(Err(e)) => panic!("lambda errored on the tier: {e}"),
            TierOutcome::Fallback => panic!("lambda must run on the tier, not fall back"),
        }
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
            param_checks: Vec::new(),
            return_check: None,
            def_file: None,
        };

        for _ in 0..5 {
            assert!(matches!(
                tier.try_call(&func, &mut [Value::Integer(1)]),
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
            tier.try_call(&func, &mut [map_arg]),
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
        match tier.try_call(&func, &mut [Value::Function(double_fn())]) {
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
            param_checks: Vec::new(),
            return_check: None,
            def_file: None,
        };

        match tier.try_call(&func, &mut [Value::Integer(1), Value::Integer(0)]) {
            TierOutcome::Ran(Err(_)) => {}
            _ => panic!("division by zero should surface as an error"),
        }
    }
}

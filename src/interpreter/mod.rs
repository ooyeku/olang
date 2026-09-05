use crate::ast::{
    Argument, BinaryOp, BuiltinFunction, EnumVariantData, Expr, Function, FunctionDecl, LetDecl,
    MatchArm, Parameter, Program, ShareDecl, Statement, TestDecl, TypeAnnotation, UseDecl, Value,
};
use crate::builtin::BuiltinFunctions;
use crate::ovm::gc::SafepointManager;
use im::HashMap as ImHashMap;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

mod errors;
pub(crate) mod loop_promo;
mod modules;
mod ops;

/// The recursion limit that turns runaway recursion into a clean
/// "maximum call depth exceeded" error. It is a *logical* cap, not a
/// physical one: each user-function call grows the Rust stack in
/// segments when headroom runs low (see `with_stack_headroom`), so any
/// depth under the cap is physically reachable — which is also what
/// keeps the tiers honest, since the VM's heap-allocated frames would
/// otherwise sail past a depth where interpreter frames physically
/// died. `--max-depth` overrides it per run.
///
/// The playground (wasm) build uses a far lower value: wasm's call
/// stack is much smaller than a native thread's and cannot grow, and a
/// deep interpreter recursion overflows it *physically* — a hard trap
/// that poisons the whole wasm instance so every later program also
/// traps. A low limit makes the depth guard win the race, so the
/// playground reports the error and stays alive.
#[cfg(feature = "native")]
pub const DEFAULT_MAX_CALL_DEPTH: usize = 100_000;
#[cfg(not(feature = "native"))]
pub const DEFAULT_MAX_CALL_DEPTH: usize = 400;

/// Grow the stack before it runs out, then run `f`. Below 1MB of
/// headroom a fresh 32MB segment is allocated and execution continues
/// there — the segmented-stack discipline (rustc's own) that makes deep
/// recursion a memory question instead of a crash. Costs one stack-
/// pointer read per call when headroom is fine.
#[cfg(feature = "native")]
#[inline]
pub(crate) fn with_stack_headroom<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(1024 * 1024, 32 * 1024 * 1024, f)
}

#[cfg(not(feature = "native"))]
#[inline]
pub(crate) fn with_stack_headroom<R>(f: impl FnOnce() -> R) -> R {
    f()
}

/// The message for a missing module member, with the nearest real
/// member suggested when one is close. Shared wording with the VM's
/// twin so the tiers report identically.
pub(crate) fn module_member_miss<'a>(
    field: &str,
    members: impl Iterator<Item = &'a str>,
) -> String {
    let max_distance = (field.chars().count() / 3).clamp(1, 3);
    let nearest = members
        .filter(|m| *m != field)
        .map(|m| {
            (
                crate::interpreter::errors::IntuitiveErrorFormatter::levenshtein_distance(field, m),
                m,
            )
        })
        .filter(|(d, _)| *d <= max_distance)
        .min();
    match nearest {
        Some((_, m)) => format!(
            "Function '{}' not found in module — did you mean '{}'?",
            field, m
        ),
        None => format!("Function '{}' not found in module", field),
    }
}

/// The result of walking a function body's tail positions: an ordinary
/// value, or a self-call whose evaluated arguments the trampoline in
/// `call_user_function_inner` rebinds instead of pushing a frame.
enum TailFlow {
    Value(Value),
    SelfCall(Vec<Value>),
}
mod patterns;
pub use errors::{InterpreterError, IntuitiveErrorFormatter};

mod environment;
pub use environment::{Environment, ModuleDebugConfig};

/// Olang interpreter with optional type checking
pub struct Interpreter {
    environment: Environment,
    /// Shared, not owned: every builtin call hands the registry to
    /// `BuiltinFunctions::call` alongside `&mut self`, and cloning a
    /// registry of a few hundred entries per call was a real cost.
    builtin_functions: Arc<BuiltinFunctions>,
    safepoint_manager: Arc<SafepointManager>,
    pub module_debug_config: ModuleDebugConfig,

    // Enhanced module system
    module_cache: HashMap<String, ModuleCacheEntry>,
    /// Dotted import name → the file it resolved to, first resolution
    /// wins. The cache itself is keyed by file; this is the name view
    /// the REPL's `:help` needs (`geometry.point` → its index.ol).
    module_name_index: HashMap<String, std::path::PathBuf>,
    dependency_tracker: ModuleDependencyTracker,
    current_module_path: Option<String>, // For tracking current module during loading
    module_loading_stack: Vec<String>, // Feature 7: Track modules currently being loaded for circular detection

    // Feature 8: Smart caching system
    smart_cache_config: SmartCacheConfig,
    cache_statistics: CacheStatistics,

    // Feature 9: Intuitive error messages
    error_formatter: IntuitiveErrorFormatter,

    // MEMORY PROTECTION: Prevent exponential memory growth
    call_depth: usize,
    max_call_depth: usize,

    /// The evaluation budget of a sandboxed run (`meta.eval` with
    /// options): steps left — a step is a loop iteration or a call —
    /// and a wall-clock deadline. Both None for an ordinary run, which
    /// costs one branch per step.
    step_budget: Option<u64>,
    deadline: Option<(crate::clock::Instant, std::time::Duration)>,

    /// Stack of source positions of the statements currently being
    /// evaluated (innermost last). Maintained by eval_statement's
    /// Located arm; used to attribute runtime errors to source lines.
    stmt_span_stack: Vec<(u32, u32)>,
    /// Function names currently on the interpreter call stack
    /// (outermost first), for error reports.
    call_stack_names: Vec<String>,
    /// Manifest dependencies that failed to resolve at startup, by name,
    /// with the reason — so `use web` says why, and a module that never
    /// imports the missing package still runs.
    missing_dependencies: HashMap<String, String>,
    /// A one-line hint computed at error-raise time (e.g. did-you-mean
    /// candidates for an undefined name), folded into the captured
    /// ErrorLocation for top-level reporters.
    pending_error_hint: Option<String>,
    /// The location captured when the current error first surfaced.
    /// Taken (and cleared) by the top-level reporter; cleared whenever
    /// an error is consumed (caught) so a later error can't inherit a
    /// stale position.
    pending_error_location: Option<crate::ast::ErrorLocation>,
    /// The program's entry file as `set_current_file` recorded it —
    /// what `error_file` compares against to decide whether a location
    /// needs a file name at all.
    entry_file: Option<String>,
    /// The call stack as it stood in the deepest frame when an error
    /// began propagating — captured before unwinding pops the frames,
    /// so the located-statement report can name the whole chain (the
    /// VM's error trace keeps its frames; the interpreter must too, or
    /// the two tiers render different stacks for the same error).
    pending_error_frames: Option<Vec<String>>,

    /// Top-level bindings this interpreter has seen, with their
    /// mutability — the seed for validating the next program. A script is
    /// one program, but a REPL session is many, and `let mut n = 0` on one
    /// line must still authorize `n = 1` on the next.
    scope_bindings: crate::scoping::Predefined,

    /// Optional bytecode tier: hot functions are compiled and executed on the
    /// OVM instead of walking the AST. Disabled unless explicitly enabled.
    /// Boxed deliberately: the tier owns the whole VM (compiler, caches,
    /// execution state — 1.4 KB), and `call_function` moves it out of the
    /// interpreter and back on every call so the tier can borrow the
    /// interpreter for builtins. Inline, that was ~2.8 KB of memcpy per
    /// interpreted call; behind a box it is two pointer moves.
    bytecode_tier: Option<Box<crate::ovm::tier::BytecodeTier>>,

    /// Trait method implementations, keyed by (type name, method name) ->
    /// the concrete function. Populated by `impl` blocks; consulted when a
    /// `value.method(args)` call's field isn't a struct field.
    trait_impls: HashMap<(String, String), Function>,
    /// Default method bodies from `trait` declarations, keyed by
    /// (trait name, method name). Used when an impl doesn't override them.
    trait_defaults: HashMap<(String, String), Function>,
    /// Which traits each type implements, so a type can reach its trait's
    /// default methods: type name -> set of trait names.
    type_traits: HashMap<String, Vec<String>>,

    /// Names of declared unit enum variants (e.g. `North`, `AtEnd`). A bare
    /// identifier pattern is a variant equality test only when its name is one
    /// of these — otherwise it is a fresh binding. Without this, a binding
    /// sub-pattern like `b` in `Concat(a, b)` would be misread as a variant
    /// test whenever some `b` already in scope happened to hold a unit variant.
    unit_variant_names: HashSet<String>,

    /// Declared enum *type* names (not variants). A field/parameter/return
    /// annotation naming a struct or enum reduces to `FieldTypeCheck::Named`,
    /// which is enforced by comparing runtime type names — so an *undeclared*
    /// name matches nothing and produced a misleading "expects X, got Y".
    /// Struct names live in `struct_defs`; this holds the enum names, and the
    /// two together let the enforcer tell an unknown type from a real
    /// mismatch and say so.
    enum_type_names: HashSet<String>,

    /// Test-runner mode (`olang test`): when on, `test` blocks record their
    /// outcome here and execution continues past failures instead of
    /// aborting. When off (normal runs), a failing test block is an error —
    /// the long-standing inline behavior.
    test_mode: bool,
    test_results: Vec<TestOutcome>,

    /// Line-coverage recording (`olang test --coverage`). When `Some`,
    /// every executed located statement records its line under the file
    /// that owns the code (the enclosing function's `def_file`, or the
    /// current module at top level). `coverage_file_stack` tracks that
    /// owning file across calls: `call_function` pushes the callee's
    /// `def_file` and pops it on return, so a test in one file that calls
    /// into another attributes each line to the file it actually lives in.
    coverage: Option<HashMap<String, std::collections::BTreeSet<u32>>>,
    coverage_file_stack: Vec<Option<String>>,

    /// Meta mode: this interpreter is running `meta fn` bodies at macro
    /// expansion time (src/expand.rs). The effectful and nondeterministic
    /// modules refuse, so expansion is a pure function of the source.
    meta_mode: bool,

    /// Capability enforcement (`[capabilities]` in olang.toml, embedded
    /// bundle manifests, or `--deny`). None = everything allowed, and the
    /// gate costs a single branch. When Some, `coverage_file_stack` is
    /// kept live so gated builtin calls attribute to the package whose
    /// code made them.
    caps: Option<std::sync::Arc<crate::caps::CapTable>>,

    /// `--trace-caps`: the set of capabilities this run has exercised so
    /// far. When Some, every gated builtin call records its demand at the
    /// dispatch choke point (independent of whether a manifest is loaded),
    /// so the profiler can report a least-privilege grant at end of run.
    ///
    /// Shared behind an Arc because the bytecode tier's bridge
    /// interpreter is a *different* `Interpreter` that dispatches the same
    /// builtins. It has to write into this set, not a copy of it, or a
    /// promoted function's effects would vanish from the profile.
    caps_trace:
        Option<std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<crate::caps::CapUse>>>>,

    /// The Open Timeline (`--record` / `olang replay`). When present,
    /// every nondeterministic builtin call is logged (record) or served
    /// from the log (replay). Like capabilities, it runs on the
    /// interpreter tier so the one dispatch choke point sees every call.
    timeline: Option<crate::timeline::Timeline>,
    /// Canonicalized-path memo for capability attribution (def_file
    /// strings -> real paths), so the gate never repeats a syscall.
    caps_path_cache: HashMap<String, std::path::PathBuf>,
    /// One-shot pre-grant from the bytecode compiler (see
    /// `set_cap_pregranted`); consumed by the next `capability_denial`.
    cap_pregranted: bool,
    /// A previous run's warm profile, held until a tier exists to take it.
    warm_profile: Option<crate::ovm::warm::WarmProfile>,

    /// Declared struct types: name -> field names. Construction of a
    /// declared struct validates its field set; an undeclared struct-literal
    /// name is an error.
    struct_defs: HashMap<String, Vec<String>>,

    /// Declared struct field types, for the fields whose annotation the
    /// runtime can enforce: struct name -> (field name -> checkable type).
    /// Construction rejects a field value whose runtime type does not match
    /// its declared annotation. Fields with an unenforceable annotation
    /// (generic parameter, list, map, function, union, ...) are absent and
    /// stay unchecked.
    struct_field_checks: HashMap<String, HashMap<String, crate::ast::FieldTypeCheck>>,

    /// Package dependency map: dependency name -> the directory whose `.ol`
    /// files it exposes. A `use foo.bar` whose first segment is a dependency
    /// name resolves inside that directory rather than relative to the
    /// current file. Populated by the package manager before execution.
    dependency_map: HashMap<String, std::path::PathBuf>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

/// One argument position at a call boundary: provided by the caller,
/// or to be filled from the parameter's default. Defaults are
/// evaluated by `fill_default_arguments` — in the callee's scope,
/// never the caller's.
pub enum ArgSlot {
    Given(Value),
    FromDefault,
}

impl Interpreter {
    pub fn new() -> Self {
        // Feature 8: Initialize smart caching
        let smart_cache_config = SmartCacheConfig::default();

        let mut interpreter = Self {
            environment: Environment::new(),
            builtin_functions: Arc::new(BuiltinFunctions::new()),
            safepoint_manager: Arc::new(SafepointManager::new()),
            module_debug_config: ModuleDebugConfig::default(),

            // Enhanced module system
            module_cache: HashMap::new(),
            module_name_index: HashMap::new(),
            dependency_tracker: ModuleDependencyTracker::new(),
            current_module_path: None, // For tracking current module during loading
            module_loading_stack: Vec::new(), // Feature 7: Track modules currently being loaded for circular detection

            // Feature 8: Smart caching system
            smart_cache_config,
            cache_statistics: CacheStatistics::default(),
            error_formatter: IntuitiveErrorFormatter::default(),

            // MEMORY PROTECTION: Initialize recursion depth tracking
            call_depth: 0,
            max_call_depth: DEFAULT_MAX_CALL_DEPTH,
            step_budget: None,
            deadline: None,
            stmt_span_stack: Vec::new(),
            call_stack_names: Vec::new(),
            missing_dependencies: HashMap::new(),
            pending_error_location: None,
            entry_file: None,
            pending_error_frames: None,
            pending_error_hint: None,
            scope_bindings: crate::scoping::Predefined::new(),
            bytecode_tier: None,
            trait_impls: HashMap::new(),
            trait_defaults: HashMap::new(),
            type_traits: HashMap::new(),
            unit_variant_names: HashSet::new(),
            enum_type_names: HashSet::new(),
            test_mode: false,
            test_results: Vec::new(),
            coverage: None,
            coverage_file_stack: Vec::new(),
            caps: None,
            meta_mode: false,
            caps_trace: None,
            caps_path_cache: HashMap::new(),
            cap_pregranted: false,
            warm_profile: None,
            timeline: None,
            struct_defs: HashMap::new(),
            struct_field_checks: HashMap::new(),
            dependency_map: HashMap::new(),
        };

        // Register built-in functions
        interpreter.register_builtins();
        interpreter
    }

    /// Set the current file path for module resolution context
    /// This allows relative module imports to be resolved correctly
    /// when running a file from a different directory
    pub fn set_current_file(&mut self, file_path: &std::path::Path) {
        // Store the file path as the current module context
        if let Some(path_str) = file_path.to_str() {
            self.current_module_path = Some(path_str.to_string());
            self.entry_file = Some(path_str.to_string());
            if let Some(tier) = self.bytecode_tier.as_mut() {
                tier.set_entry_file(self.entry_file.clone());
            }

            // Calculate content hash if the file exists (to prevent cache invalidation)
            let content_hash = if file_path.exists() {
                self.calculate_file_hash(file_path).unwrap_or_default()
            } else {
                String::new()
            };

            // Also cache a dummy entry so discover_module_same_directory can find the directory
            let cache_entry = ModuleCacheEntry {
                module: Value::Unit,
                file_path: Some(file_path.to_path_buf()),
                last_modified: Some(crate::clock::system_now()),
                dependencies: vec![],
                content_hash,
                compilation_time: std::time::Duration::default(),
                access_count: 0,
                last_accessed: crate::clock::system_now(),
                cache_generation: 0,
                memory_size: 0,
            };
            self.module_cache.insert(path_str.to_string(), cache_entry);
        }
    }

    /// Clear the current file context
    pub fn clear_current_file(&mut self) {
        self.current_module_path = None;
    }

    /// Register built-in functions in the environment
    fn register_builtins(&mut self) {
        for (name, func) in self.builtin_functions.get_functions() {
            self.environment
                .define(name.clone(), Value::Builtin(func.clone()));
        }

        // Register stdlib modules
        let stdlib = crate::stdlib::get_stdlib();
        for (module_name, module_value) in stdlib {
            self.environment.define(module_name, module_value);
        }
    }

    /// Evaluate a program.
    ///
    /// Scoping and mutability are validated first, over the whole program:
    /// a `let`-less first assignment or a write to a binding that is not
    /// `mut` is refused before any statement runs, so a program either
    /// obeys the rules everywhere or does nothing at all. The rules are
    /// lexical, so this check is the same on every execution tier.
    pub fn eval_program(&mut self, program: Program) -> Result<Value, InterpreterError> {
        let mut known = std::mem::take(&mut self.scope_bindings);
        let scope_errors = crate::scoping::validate_program(&program, &mut known);
        self.scope_bindings = known;
        if let Some(first) = scope_errors.first() {
            self.pending_error_location = Some(crate::ast::ErrorLocation {
                line: first.line,
                column: first.column,
                file: self.error_file(),
                call_stack: Vec::new(),
                hint: None,
            });
            return Err(InterpreterError::RuntimeError {
                message: first.message.clone(),
            });
        }

        let mut last_value = Value::Unit;
        for statement in &program.statements {
            // A fresh top-level statement gets a clean slate: any location
            // captured for an earlier (recovered) failure must not be
            // inherited by a later error.
            self.pending_error_location = None;
            self.pending_error_frames = None;
            self.pending_error_hint = None;
            last_value = self.eval_statement(statement)?;
        }
        Ok(last_value)
    }

    /// "an Int", "a String" — the article English wants for a type name.
    fn with_article(ty: &str) -> String {
        let vowel = matches!(
            ty.as_bytes().first(),
            Some(b'A' | b'E' | b'I' | b'O' | b'U')
        );
        format!("{} {}", if vowel { "an" } else { "a" }, ty)
    }

    /// A module path for display: relative to the entry file's directory
    /// when it lies under it, the path as stored otherwise.
    fn short_file(entry: Option<&str>, file: &str) -> String {
        if let Some(entry) = entry
            && let Some(dir_end) = entry.rfind('/')
            && let Some(rest) = file.strip_prefix(&entry[..=dir_end])
        {
            return rest.to_string();
        }
        file.to_string()
    }

    /// When a call failed because the callee is not a function, put the
    /// caller's NAME in the message: "'x' is an Int, not a function".
    fn name_uncallable(e: InterpreterError, name: Option<&str>) -> InterpreterError {
        match (&e, name) {
            (InterpreterError::TypeError { message }, Some(n))
                if message.starts_with("cannot call a") =>
            {
                let ty = message
                    .trim_start_matches("cannot call an ")
                    .trim_start_matches("cannot call a ")
                    .split(':')
                    .next()
                    .unwrap_or("value")
                    .to_string();
                let hint = match ty.as_str() {
                    "Map" | "List" => format!(" To index it, write {}[...] instead", n),
                    _ => format!(
                        " If a function named '{}' exists elsewhere, this local \
                         binding shadows it",
                        n
                    ),
                };
                InterpreterError::TypeError {
                    message: format!(
                        "'{}' is {}, not a function — it cannot be called.{}",
                        n,
                        Self::with_article(&ty),
                        hint
                    ),
                }
            }
            _ => e,
        }
    }

    /// Loop/return/`?` signals travel as Err but aren't errors — they must
    /// never capture an error location (their consumption is normal flow).
    /// The tier hands back a bare message string; map it onto the
    /// interpreter's own error variants so the Display a user sees is
    /// identical no matter which tier executed the function.
    fn map_tier_error_message(message: String) -> InterpreterError {
        if message == "Pattern match failed" {
            InterpreterError::PatternMatchFailed
        } else if let Some(rest) = message.strip_prefix("Type error: ") {
            // A non-function called inside a compiled frame: the VM does
            // not know the identifier, and the interpreted caller above
            // must not claim the error as its own callee's — that named a
            // function frames away from the parameter that shadowed one.
            if rest.starts_with("cannot call a") {
                return InterpreterError::TypeError {
                    message: format!(
                        "inside a compiled function, {} — a parameter or local named like \
the function it shadows is the usual cause; `olang check` names the parameter",
                        rest
                    ),
                };
            }
            InterpreterError::TypeError {
                message: rest.to_string(),
            }
        } else if let Some(name) = message.strip_prefix("Undefined variable: ") {
            InterpreterError::UndefinedVariable {
                name: name.to_string(),
            }
        } else if let Some(rest) = message.strip_prefix("Arity mismatch: expected ") {
            // "N, got M" — both ends are ours, parse back into the
            // structured variant so Display is bare (no "Runtime error:"
            // prefix), exactly like the interpreter's own arity errors.
            let mut parts = rest.splitn(2, ", got ");
            match (
                parts.next().and_then(|s| s.parse().ok()),
                parts.next().and_then(|s| s.parse().ok()),
            ) {
                (Some(expected), Some(got)) => InterpreterError::ArityMismatch { expected, got },
                _ => InterpreterError::RuntimeError { message },
            }
        } else {
            InterpreterError::runtime(message)
        }
    }

    /// Compose the interpreter's live stack with a tier trace's frames
    /// (innermost-first). The tier notes every VM frame including the
    /// boundary callee, which the interpreter's stack already ends
    /// with — drop that duplicate when the two meet.
    fn splice_tier_stack(&self, frames: Vec<String>) -> Vec<String> {
        let mut call_stack = self.call_stack_names.clone();
        let mut outermost_first = frames.into_iter().rev().peekable();
        if let (Some(last), Some(first)) = (call_stack.last(), outermost_first.peek())
            && last == first
        {
            outermost_first.next();
        }
        call_stack.extend(outermost_first);
        call_stack
    }

    fn is_control_signal(e: &InterpreterError) -> bool {
        matches!(
            e,
            InterpreterError::BreakSignal(_)
                | InterpreterError::ContinueSignal
                | InterpreterError::ReturnSignal(_)
                | InterpreterError::ErrPropagation(_)
        )
    }

    /// Does this statement introduce a name into the environment? A block
    /// only needs its own frame when something inside it binds; type,
    /// trait, and impl declarations register globally rather than in a
    /// scope, so they do not count.
    fn statement_binds(statement: &Statement) -> bool {
        match statement {
            Statement::Located { stmt, .. } => Self::statement_binds(stmt),
            Statement::LetDecl(_)
            | Statement::FunctionDecl(_)
            | Statement::UseDecl(_)
            | Statement::ShareDecl(_) => true,
            _ => false,
        }
    }

    /// Rank every visible binding by edit distance to `name` and offer
    /// the closest few — the REPL has had this for typos; file mode now
    /// gets it too. Distance is capped relative to the name's length so
    /// short names don't suggest everything.
    /// Bind a name at the REPL's global scope — the REPL uses this for
    /// `it`, the last printed result, and the VM's bridge uses it to
    /// seed the program's function landscape.
    pub fn define_global(&mut self, name: &str, value: Value) {
        self.environment.define(name.to_string(), value);
    }

    /// The bytecode tier, when one is enabled — the VM's bridge seeds
    /// its registry through this.
    /// Whether this interpreter carries a bytecode tier (diagnostics).
    pub fn bytecode_tier_present(&self) -> bool {
        self.bytecode_tier.is_some()
    }

    /// Run a whole `map`/`filter` over `items` on the compiled tier, or
    /// None when the tier declines and the caller's per-element loop
    /// should run instead. Kernels with defaults or bounds stay on the
    /// interpreter, where those features live.
    pub fn tier_hof(
        &mut self,
        name: &str,
        function: &Value,
        items: &[Value],
    ) -> Option<Result<Value, InterpreterError>> {
        self.tier_hof_with(name, function, items, None)
    }

    /// The fold shape: same door, with the initial accumulator.
    pub fn tier_hof_with(
        &mut self,
        name: &str,
        function: &Value,
        items: &[Value],
        init: Option<&Value>,
    ) -> Option<Result<Value, InterpreterError>> {
        let Value::Function(f) = function else {
            return None;
        };
        let arity = if init.is_some() { 2 } else { 1 };
        if f.parameters.len() != arity
            || f.parameters.iter().any(|p| p.default_value.is_some())
            || !f.param_bounds.is_empty()
        {
            return None;
        }
        let globals = self.global_bindings();
        let mut tier = self.bytecode_tier.take()?;
        tier.set_host_globals(globals);
        let out = tier.try_hof(name, f, items, init);
        self.bytecode_tier = Some(tier);
        match out? {
            Ok(v) => Some(Ok(v)),
            Err(message) => {
                let err = Self::map_tier_error_message(message);
                // Splice the kernel's error trace exactly as the tiered
                // call path does — without this, an error inside a hot
                // `map`/`filter` kernel reported the caller's top-level
                // line instead of the failing statement.
                if self.pending_error_location.is_none() && !Self::is_control_signal(&err) {
                    let (span, frames, _leak) = self
                        .bytecode_tier
                        .as_mut()
                        .map(|t| t.take_error_trace())
                        .unwrap_or_default();
                    if let Some((line, column)) = span {
                        let trace_file = self
                            .bytecode_tier
                            .as_mut()
                            .and_then(|t| t.take_error_trace_file())
                            .filter(|f| {
                                !f.starts_with("__")
                                    && self.entry_file.as_deref() != Some(f.as_str())
                            });
                        self.pending_error_location = Some(crate::ast::ErrorLocation {
                            line,
                            column,
                            file: trace_file.or_else(|| self.error_file()),
                            call_stack: self.splice_tier_stack(frames),
                            hint: self.pending_error_hint.take(),
                        });
                    } else if !frames.is_empty() && self.pending_error_frames.is_none() {
                        self.pending_error_frames = Some(self.splice_tier_stack(frames));
                    }
                }
                Some(Err(err))
            }
        }
    }

    pub fn bytecode_tier_mut(&mut self) -> Option<&mut crate::ovm::tier::BytecodeTier> {
        self.bytecode_tier.as_deref_mut()
    }

    fn did_you_mean(&self, name: &str) -> Option<String> {
        let max_distance = (name.chars().count() / 3).clamp(1, 3);
        let mut candidates: Vec<(usize, String)> = self
            .environment
            .get_all_variables()
            .keys()
            .filter(|k| k.as_str() != name)
            .map(|k| {
                (
                    crate::interpreter::errors::IntuitiveErrorFormatter::levenshtein_distance(
                        name, k,
                    ),
                    k.clone(),
                )
            })
            .filter(|(d, _)| *d <= max_distance)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        candidates.sort();
        let names: Vec<String> = candidates.into_iter().take(3).map(|(_, n)| n).collect();
        Some(format!("did you mean: {}?", names.join(", ")))
    }

    /// Take (and clear) the location captured for the error currently
    /// propagating, if any. Top-level reporters call this after a failed
    /// run to render "where" alongside the error's own "what".
    /// The file a raised error belongs to: the module whose statements are
    /// executing, or None when that is the entry program itself. A module
    /// loaded at import runs with `current_module_path` set to its own
    /// file, so an error inside it names that file rather than the `use`
    /// line that triggered the load.
    fn error_file(&self) -> Option<String> {
        let current = self.current_module_path.as_deref()?;
        if current.starts_with("__") {
            return None;
        }
        match self.entry_file.as_deref() {
            Some(entry) if entry == current => None,
            None => None,
            _ => Some(current.to_string()),
        }
    }

    pub fn take_error_location(&mut self) -> Option<crate::ast::ErrorLocation> {
        self.pending_error_frames = None;
        self.pending_error_location.take()
    }

    pub fn eval_statement(&mut self, statement: &Statement) -> Result<Value, InterpreterError> {
        // Safepoint poll for GC coordination
        self.safepoint_poll()?;

        match statement {
            // Safety net: macros are expanded away in Parser::parse. A meta
            // construct reaching evaluation means the program bypassed
            // expansion (built by hand, or a bug) — refuse loudly rather
            // than half-running it.
            Statement::MetaFnDecl { .. } => Err(InterpreterError::RuntimeError {
                message: "a `meta fn` reached the interpreter without expansion — \
                          parse the program with Parser::parse (which expands macros), \
                          not a hand-built AST"
                    .to_string(),
            }),
            Statement::DecoratedDecl { .. } => Err(InterpreterError::RuntimeError {
                message: "a decorated declaration reached the interpreter without \
                          expansion — parse the program with Parser::parse"
                    .to_string(),
            }),
            Statement::Located { line, column, stmt } => {
                // Position discipline: push this statement's span for the
                // duration of its evaluation (nested blocks push deeper
                // spans; calls into interpreted functions do too). When an
                // error first surfaces, the innermost span on the stack is
                // where it happened — capture it once, together with the
                // call stack, for the top-level reporter.
                self.stmt_span_stack.push((*line, *column));
                // Coverage: tally this line under the file that owns the code
                // — the innermost call frame's def_file, or the current
                // module at top level. Cheap `is_some` guard when off.
                if self.coverage.is_some() {
                    let file = self
                        .coverage_file_stack
                        .last()
                        .and_then(|o| o.clone())
                        .or_else(|| self.current_module_path.clone());
                    if let (Some(cov), Some(f)) = (self.coverage.as_mut(), file) {
                        cov.entry(f).or_default().insert(*line);
                    }
                }
                let result = self.eval_statement(stmt);
                if let Err(e) = &result
                    && self.pending_error_location.is_none()
                    && !Self::is_control_signal(e)
                {
                    self.pending_error_location = Some(crate::ast::ErrorLocation {
                        line: *line,
                        column: *column,
                        file: self.error_file(),
                        call_stack: self
                            .pending_error_frames
                            .take()
                            .unwrap_or_else(|| self.call_stack_names.clone()),
                        hint: self.pending_error_hint.take(),
                    });
                }
                self.stmt_span_stack.pop();
                result
            }
            Statement::Expression(expr) => self.eval_expr(expr),
            Statement::LetDecl(let_decl) => self.eval_let_decl(let_decl),
            Statement::FunctionDecl(func_decl) => self.eval_function_decl(func_decl.clone()),
            Statement::TypeDecl(type_decl) => self.eval_type_decl(type_decl.clone()),
            Statement::ErrorTypeDecl(error_type_decl) => {
                self.eval_error_type_decl(error_type_decl.clone())
            }
            Statement::ShareDecl(share_decl) => self.eval_share_decl(share_decl.clone()),
            Statement::UseDecl(use_decl) => self.eval_use_decl(use_decl.clone()),
            Statement::TestDecl(test_decl) => self.eval_test_decl(test_decl.clone()),
            Statement::TraitDecl(trait_decl) => self.eval_trait_decl(trait_decl.clone()),
            Statement::ImplDecl(impl_decl) => self.eval_impl_decl(impl_decl.clone()),
        }
    }

    /// Record a trait's default method bodies. The trait itself introduces no
    /// runtime binding; it is a contract that `impl` blocks fulfil.
    fn eval_trait_decl(
        &mut self,
        trait_decl: crate::ast::TraitDecl,
    ) -> Result<Value, InterpreterError> {
        let closure = self.environment.flat_snapshot();
        for method in &trait_decl.methods {
            if let Some(body) = &method.default_body {
                let function = Function {
                    name: Some(method.name.clone()),
                    param_checks: crate::ast::param_checks_of(&method.parameters, &[]),
                    return_check: None,
                    parameters: method.parameters.clone(),
                    body: Arc::new(body.clone()),
                    closure: Arc::new(closure.clone()),
                    param_bounds: Vec::new(),
                    def_file: self.current_module_path.clone(),
                };
                if let Some(tier) = self.bytecode_tier.as_mut() {
                    tier.note_trait_default(
                        trait_decl.name.clone(),
                        method.name.clone(),
                        function.clone(),
                    );
                }
                self.trait_defaults
                    .insert((trait_decl.name.clone(), method.name.clone()), function);
            }
        }
        Ok(Value::Unit)
    }

    /// Register the methods of an `impl Trait for Type` block for runtime
    /// dispatch, and note that Type implements Trait.
    fn eval_impl_decl(
        &mut self,
        impl_decl: crate::ast::ImplDecl,
    ) -> Result<Value, InterpreterError> {
        let closure = self.environment.flat_snapshot();
        for method in &impl_decl.methods {
            let function = Function {
                name: Some(method.name.clone()),
                param_checks: crate::ast::param_checks_of(&method.parameters, &method.type_params),
                return_check: crate::ast::return_check_of(
                    method.return_type.as_ref(),
                    &method.type_params,
                ),
                parameters: method.parameters.clone(),
                body: Arc::new(method.body.clone()),
                closure: Arc::new(closure.clone()),
                param_bounds: Vec::new(),
                def_file: self.current_module_path.clone(),
            };
            // Tell the tier this method body exists under its bare name. A
            // method dispatched by receiver type (`s.area()`) reaches the tier
            // keyed only by "area"; when two impls define `area`, the tier's
            // ambiguity guard sees two distinct bodies and keeps the name on
            // the interpreter — so dispatch stays correct. A single-impl
            // method is unambiguous and still promotes. (Before struct
            // arguments became tier-representable, these never compiled, which
            // hid the need to note them.)
            if let Some(tier) = self.bytecode_tier.as_mut() {
                tier.note_function(method.name.clone(), function.clone());
                tier.note_trait_impl(
                    impl_decl.type_name.clone(),
                    method.name.clone(),
                    function.clone(),
                );
            }
            self.trait_impls
                .insert((impl_decl.type_name.clone(), method.name.clone()), function);
        }
        if let Some(tier) = self.bytecode_tier.as_mut() {
            tier.note_type_trait(impl_decl.type_name.clone(), impl_decl.trait_name.clone());
        }
        let traits = self
            .type_traits
            .entry(impl_decl.type_name.clone())
            .or_default();
        if !traits.contains(&impl_decl.trait_name) {
            traits.push(impl_decl.trait_name.clone());
        }
        Ok(Value::Unit)
    }

    /// Map trait bounds on type parameters to the positions of parameters
    /// annotated with those type variables. `<T: Show>(x: T)` yields
    /// `[(0, ["Show"])]`. A parameter must be annotated with the bare type
    /// variable (`x: T`) for the bound to attach.
    fn resolve_param_bounds(
        type_param_bounds: &[(String, Vec<String>)],
        parameters: &[Parameter],
    ) -> Vec<(usize, Vec<String>)> {
        if type_param_bounds.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (index, param) in parameters.iter().enumerate() {
            let annotated = match &param.type_annotation {
                Some(TypeAnnotation::TypeVariable(name)) => Some(name),
                Some(TypeAnnotation::Custom(name)) => Some(name),
                _ => None,
            };
            if let Some(type_var) = annotated {
                for (bound_var, traits) in type_param_bounds {
                    if bound_var == type_var {
                        out.push((index, traits.clone()));
                    }
                }
            }
        }
        out
    }

    /// Public: whether a value's runtime type implements a trait. Backs the
    /// `implements(value, "Trait")` builtin.
    pub fn value_implements(&self, value: &Value, trait_name: &str) -> bool {
        self.type_implements(&value.type_name(), trait_name)
    }

    /// Whether a runtime type name satisfies a trait — either via an explicit
    /// `impl Trait for Type`, or because the type is the trait's own name
    /// (allowing a bound to name a concrete type too).
    fn type_implements(&self, type_name: &str, trait_name: &str) -> bool {
        if type_name == trait_name {
            return true;
        }
        self.type_traits
            .get(type_name)
            .is_some_and(|traits| traits.iter().any(|t| t == trait_name))
    }

    /// Check every trait bound of a function against the supplied arguments.
    /// Returns a clear error naming the argument, its type, and the unmet
    /// trait when a bound is violated.
    fn check_param_bounds(
        &self,
        func: &Function,
        arguments: &[Value],
    ) -> Result<(), InterpreterError> {
        for (index, traits) in &func.param_bounds {
            if let Some(arg) = arguments.get(*index) {
                let type_name = arg.type_name();
                for trait_name in traits {
                    if !self.type_implements(&type_name, trait_name) {
                        let fn_name = func.name.as_deref().unwrap_or("<lambda>");
                        return Err(InterpreterError::TypeError {
                            message: format!(
                                "{}: argument {} of type {} does not implement trait {}",
                                fn_name,
                                index + 1,
                                type_name,
                                trait_name
                            ),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Enforce declared parameter types at the call boundary. Annotations
    /// are promises: an unannotated (or generic) parameter has no check and
    /// stays fully dynamic; an annotated one rejects a mismatched argument
    /// here, with the same message on every tier.
    /// The value view `FieldTypeCheck::check_value` wants: the outer type
    /// name, which Result side is present (and its payload's name), and a
    /// callable's (required, total) parameter counts where the value
    /// exposes them.
    #[allow(clippy::type_complexity)]
    fn value_view(v: &Value) -> (String, Option<(bool, String)>, Option<(usize, usize)>) {
        let payload = match v {
            Value::Ok(p) => Some((true, p.type_name())),
            Value::Err(p) => Some((false, p.type_name())),
            _ => None,
        };
        let arity = match v {
            Value::Function(f) => Some((
                f.parameters
                    .iter()
                    .filter(|p| p.default_value.is_none())
                    .count(),
                f.parameters.len(),
            )),
            // Builtins don't expose arity; the base check still applies.
            _ => None,
        };
        (v.type_name(), payload, arity)
    }

    /// The scalar leg of the view, for literal-type checks.
    fn scalar_view(v: &Value) -> Option<crate::ast::ScalarView<'_>> {
        match v {
            Value::Integer(i) => Some(crate::ast::ScalarView::Int(*i)),
            Value::String(s) => Some(crate::ast::ScalarView::Str(s.as_ref())),
            Value::Boolean(b) => Some(crate::ast::ScalarView::Bool(*b)),
            _ => None,
        }
    }

    fn check_param_types(
        &self,
        func: &Function,
        arguments: &[Value],
    ) -> Result<(), InterpreterError> {
        for (index, check) in func.param_checks.iter().enumerate() {
            if let (Some(check), Some(arg)) = (check, arguments.get(index)) {
                let (actual, payload, fn_arity) = Self::value_view(arg);
                if let Some((expected, got)) = check.check_value(
                    &actual,
                    payload.as_ref().map(|(o, p)| (*o, p.as_str())),
                    fn_arity,
                    Self::scalar_view(arg),
                ) {
                    let fn_name = func.name.as_deref().unwrap_or("<lambda>");
                    let param = func
                        .parameters
                        .get(index)
                        .map(|p| p.name.as_str())
                        .unwrap_or("?");
                    return Err(InterpreterError::TypeError {
                        message: self.annotation_error(
                            &format!("parameter '{}' of {}", param, fn_name),
                            check,
                            &expected,
                            &got,
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Enforce a declared return type on the value a call produced.
    fn check_return_type(&self, func: &Function, value: &Value) -> Result<(), InterpreterError> {
        if let Some(check) = &func.return_check {
            let (actual, payload, fn_arity) = Self::value_view(value);
            if let Some((expected, got)) = check.check_value(
                &actual,
                payload.as_ref().map(|(o, p)| (*o, p.as_str())),
                fn_arity,
                Self::scalar_view(value),
            ) {
                let fn_name = func.name.as_deref().unwrap_or("<lambda>");
                return Err(InterpreterError::TypeError {
                    message: self.annotation_error(
                        &format!("return value of {}", fn_name),
                        check,
                        &expected,
                        &got,
                    ),
                });
            }
        }
        Ok(())
    }

    /// Resolve a method for `type_name`: a concrete impl first, then any
    /// default from a trait that type implements.
    fn lookup_method(&self, type_name: &str, method: &str) -> Option<Function> {
        if let Some(f) = self
            .trait_impls
            .get(&(type_name.to_string(), method.to_string()))
        {
            return Some(f.clone());
        }
        if let Some(traits) = self.type_traits.get(type_name) {
            for trait_name in traits {
                if let Some(f) = self
                    .trait_defaults
                    .get(&(trait_name.clone(), method.to_string()))
                {
                    return Some(f.clone());
                }
            }
        }
        None
    }

    fn eval_error_type_decl(
        &mut self,
        error_type_decl: crate::ast::ErrorTypeDecl,
    ) -> Result<Value, InterpreterError> {
        // An error declaration behaves like an enum: each bare variant binds
        // as a unit value, each payload variant as a constructor taking the
        // payload fields positionally. The resulting values are ordinary
        // enum values, so `Err(NotFound)` and `match r { Err(Invalid(m)) =>
        // ... }` use the existing Result and pattern machinery.
        for variant in &error_type_decl.variants {
            let value = if variant.fields.is_empty() {
                self.unit_variant_names.insert(variant.name.clone());
                if let Some(tier) = self.bytecode_tier.as_mut() {
                    tier.note_unit_variant(variant.name.clone());
                }
                Value::Enum {
                    type_name: error_type_decl.name.clone(),
                    variant_name: variant.name.clone(),
                    variant_data: EnumVariantData::Unit,
                }
            } else {
                Value::EnumConstructor {
                    type_name: error_type_decl.name.clone(),
                    variant_name: variant.name.clone(),
                    arity: variant.fields.len(),
                }
            };
            self.environment.define(variant.name.clone(), value);
        }
        Ok(Value::Unit)
    }

    fn eval_let_decl(&mut self, let_decl: &LetDecl) -> Result<Value, InterpreterError> {
        let value = if let Some(expr) = &let_decl.value {
            self.eval_expr(expr)?
        } else {
            Value::Unit
        };

        // Enforce a declared binding type: `let x: Int = ...` promises the
        // bound value is an Int. Unannotated (or generic/unreducible)
        // bindings stay fully dynamic.
        if let Some(check) = let_decl
            .type_annotation
            .as_ref()
            .and_then(|ann| crate::ast::FieldTypeCheck::from_annotation(ann, &[]))
        {
            let (actual, payload, fn_arity) = Self::value_view(&value);
            if let Some((expected, got)) = check.check_value(
                &actual,
                payload.as_ref().map(|(o, p)| (*o, p.as_str())),
                fn_arity,
                Self::scalar_view(&value),
            ) {
                let binding = match &let_decl.pattern {
                    crate::ast::Pattern::Identifier(name) => name.as_str(),
                    _ => "value",
                };
                return Err(InterpreterError::TypeError {
                    message: self.annotation_error(
                        &format!("let binding '{}'", binding),
                        &check,
                        &expected,
                        &got,
                    ),
                });
            }
        }

        // Use pattern matching to bind variables from the pattern
        let mut bindings = HashMap::new();
        if !self.pattern_matches_bind(&let_decl.pattern, &value, &mut bindings)? {
            return Err(InterpreterError::PatternMatchFailed);
        }

        // Bind all variables from the pattern
        for (var_name, var_value) in bindings {
            self.environment.define(var_name, var_value);
        }

        Ok(value)
    }

    fn eval_function_decl(&mut self, func_decl: FunctionDecl) -> Result<Value, InterpreterError> {
        // Convert ImHashMap to regular HashMap for closure storage
        // O(1): the environment's flat map is persistent, adopt it directly
        let closure = self.environment.flat_snapshot();

        // Resolve identifiers to frame slots once, at declaration — the
        // call path then indexes instead of probing names (with per-use
        // verification and name fallback, so this can never change results)
        let param_names: Vec<String> = func_decl
            .parameters
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let resolved_body = crate::resolve::Resolver::resolve_function_body(
            &func_decl.body,
            Some(&func_decl.name),
            &param_names,
        );

        // Resolve trait bounds to parameter positions: a parameter annotated
        // with a bounded type variable (`x: T` where `T: Show`) records
        // (index, required traits), checked against the argument at call time.
        let param_bounds =
            Self::resolve_param_bounds(&func_decl.type_param_bounds, &func_decl.parameters);

        let param_checks =
            crate::ast::param_checks_of(&func_decl.parameters, &func_decl.type_params);
        let return_check =
            crate::ast::return_check_of(func_decl.return_type.as_ref(), &func_decl.type_params);
        let function = Function {
            name: Some(func_decl.name.clone()),
            parameters: func_decl.parameters,
            body: Arc::new(resolved_body),
            closure: Arc::new(closure),
            param_bounds,
            param_checks,
            return_check,
            def_file: self.current_module_path.clone(),
        };

        // Let the bytecode tier know this function exists, so a promoted
        // function that calls it can have it compiled too
        if let Some(tier) = self.bytecode_tier.as_mut() {
            tier.note_function(func_decl.name.clone(), function.clone());
        }

        let function_value = Value::Function(function);

        // Define the function in the current environment so it can be called recursively
        self.environment
            .define(func_decl.name, function_value.clone());

        Ok(function_value)
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, InterpreterError> {
        match expr {
            Expr::MacroCall { name, .. } => Err(InterpreterError::RuntimeError {
                message: format!(
                    "macro '@{}' reached the interpreter without expansion — parse the \
                     program with Parser::parse (which expands macros)",
                    name
                ),
            }),
            Expr::Integer(n) => Ok(Value::Integer(*n)),
            Expr::Float(x) => Ok(Value::Float(*x)),
            Expr::String(s) => Ok(Value::String(s.clone())),
            Expr::Boolean(b) => Ok(Value::Boolean(*b)),
            Expr::List(items_rc) => {
                let mut values = Vec::with_capacity(items_rc.len()); // Pre-allocate
                for item in items_rc.iter() {
                    values.push(self.eval_expr(item)?);
                }
                Ok(Value::List(std::sync::Arc::from(values)))
            }
            Expr::Tuple(items_rc) => {
                // The empty tuple is Unit — `()` in source reaches here.
                if items_rc.is_empty() {
                    return Ok(Value::Unit);
                }
                let mut values = Vec::new();
                for item in items_rc.iter() {
                    values.push(self.eval_expr(item)?);
                }
                Ok(Value::Tuple(std::sync::Arc::new(values)))
            }
            Expr::Identifier(name) => match self.environment.get(name) {
                Some(v) => Ok(v),
                None => {
                    // The bundled `collections` module is automatically
                    // available: the unbound name loads it here, on first
                    // touch — `collections.heap.push(...)` works in a
                    // bare script with no `use`, and a program that never
                    // reaches for it never pays for it. A user binding of
                    // the same name wins by construction: a bound name
                    // never misses. Loading rides the normal `use`
                    // machinery (and its per-process parsed-AST cache),
                    // so `use collections` remains equivalent and legal.
                    if crate::stdlib::embedded::is_auto(name) {
                        self.eval_use_decl(crate::ast::UseDecl {
                            path: vec![name.clone()],
                            items: Vec::new(),
                        })?;
                        if let Some(v) = self.environment.get(name) {
                            return Ok(v);
                        }
                    }
                    self.pending_error_hint = self.did_you_mean(name);
                    Err(InterpreterError::UndefinedVariable { name: name.clone() })
                }
            },
            Expr::LocalRef { name, depth, slot } => {
                match self.environment.get_slot(name, *depth, *slot) {
                    Some(v) => Ok(v),
                    None => {
                        self.pending_error_hint = self.did_you_mean(name);
                        Err(InterpreterError::UndefinedVariable { name: name.clone() })
                    }
                }
            }
            Expr::LocalAssign {
                name,
                depth,
                slot,
                value,
            } => {
                if let Some(result) = self.try_fused_list_write(name, value) {
                    return result;
                }
                if let Some(result) = self.try_fused_move_call(name, value) {
                    return result;
                }
                let val = self.eval_expr(value)?;
                match self.environment.set_slot(name, *depth, *slot, val.clone()) {
                    Ok(()) => Ok(val),
                    Err(_) => {
                        // Same behavior as unresolved assignment to a new name
                        self.environment.define(name.clone(), val.clone());
                        Ok(val)
                    }
                }
            }
            Expr::Call { callee, arguments } => {
                // Method-call dispatch: `receiver.method(args)` where `method`
                // is not a struct field resolves to a trait implementation for
                // the receiver's runtime type, with the receiver passed as
                // `self`. Struct fields take precedence, preserving field
                // access that happens to hold a callable.
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    let receiver = self.eval_expr(object)?;
                    let is_struct_field = matches!(
                        &receiver,
                        Value::Struct { fields, .. } if fields.contains_key(field)
                    );
                    if !is_struct_field
                        && let Some(method) = self.lookup_method(&receiver.type_name(), field)
                    {
                        let mut arg_values = vec![receiver];
                        for arg in arguments {
                            let expr = match arg {
                                Argument::Positional(e) => e,
                                Argument::Named { value, .. } => value,
                            };
                            arg_values.push(self.eval_expr(expr)?);
                        }
                        return self.call_function(Value::Function(method), arg_values);
                    }
                }

                let callee_value = self.eval_expr(callee)?;

                // The name being called, for the "is an Int, not a
                // function" message — resolving the argument slots is
                // where a non-function callee is first refused, so it is
                // named there too.
                let callee_name = match callee.as_ref() {
                    Expr::Identifier(n) => Some(n.clone()),
                    Expr::LocalRef { name, .. } => Some(name.clone()),
                    _ => None,
                };
                let arg_slots = self
                    .resolve_argument_slots(&callee_value, arguments)
                    .map_err(|e| Self::name_uncallable(e, callee_name.as_deref()))?;
                self.call_function_slots(callee_value, arg_slots)
                    .map_err(|e| Self::name_uncallable(e, callee_name.as_deref()))
            }
            Expr::Lambda {
                parameters, body, ..
            } => {
                // Capture all accessible variables from the environment chain
                let closure = self.collect_all_accessible_variables();
                let param_names: Vec<String> = parameters.iter().map(|p| p.name.clone()).collect();
                let resolved_body =
                    crate::resolve::Resolver::resolve_function_body(body, None, &param_names);
                Ok(Value::Function(Function {
                    name: None,
                    param_checks: crate::ast::param_checks_of(parameters, &[]),
                    return_check: None,
                    parameters: parameters.clone(),
                    body: Arc::new(resolved_body),
                    closure: Arc::new(closure),
                    param_bounds: Vec::new(),
                    def_file: self.current_module_path.clone(),
                }))
            }
            Expr::Pipeline { left, right } => {
                let left_value = self.eval_expr(left)?;
                match right.as_ref() {
                    Expr::Call { callee, arguments } => {
                        // Enhanced named argument resolution for pipelines
                        let callee_value = self.eval_expr(callee)?;

                        // The piped value fills the first parameter, so resolve
                        // the explicit arguments against the remaining ones —
                        // otherwise `5 |> add(3)` errors on "missing" param.
                        let resolve_target = match &callee_value {
                            Value::Function(func) if !func.parameters.is_empty() => {
                                let mut shifted = func.clone();
                                shifted.parameters.remove(0);
                                Value::Function(shifted)
                            }
                            other => other.clone(),
                        };
                        let additional_slots =
                            self.resolve_argument_slots(&resolve_target, arguments)?;

                        // Prepend the piped value as the first argument
                        let mut final_slots = vec![ArgSlot::Given(left_value)];
                        final_slots.extend(additional_slots);

                        self.call_function_slots(callee_value, final_slots)
                    }
                    Expr::Identifier(name) => {
                        let function_value = self.environment.get(name).ok_or_else(|| {
                            InterpreterError::UndefinedVariable { name: name.clone() }
                        })?;
                        self.call_function(function_value, vec![left_value])
                    }
                    // Any other expression is evaluated to a callable and
                    // applied to the piped value: `x |> ((a) => a + 1)`,
                    // `x |> handlers[i]`, `x |> pick(mode)`. The bytecode
                    // tier has always desugared these to a call; the
                    // interpreter used to refuse them, so the same program
                    // ran in the browser and failed natively.
                    other => {
                        let function_value = self.eval_expr(other)?;
                        self.call_function(function_value, vec![left_value])
                    }
                }
            }
            Expr::Match { value, arms } => {
                let value = self.eval_expr(value)?;
                self.eval_match(value, arms)
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = self.eval_expr(condition)?;
                let condition_bool = self.to_boolean(&condition)?;

                if condition_bool {
                    self.eval_expr(then_branch)
                } else if let Some(else_expr) = else_branch {
                    self.eval_expr(else_expr)
                } else {
                    Ok(Value::Unit)
                }
            }
            Expr::Block(statements) => {
                // A block scopes its bindings: a `let` inside one is gone at
                // the closing brace, and may shadow an outer binding while it
                // lasts. Blocks that declare nothing — the overwhelming
                // majority, including most loop bodies — skip the frame
                // entirely, so scoping costs nothing where nothing is bound.
                // Intermediate statements drop their results before the
                // next one runs: a retained result is a second Arc on
                // whatever the statement produced, and one extra reference
                // is exactly what defeats the sole-owner fusions
                // (`xs = xs + [..]`, `xs = col.set(xs, i, v)`) on the very
                // next line. Only the final statement's value is the
                // block's value.
                if statements.iter().any(Self::statement_binds) {
                    self.environment = Environment::with_parent(self.environment.clone());
                    let mut result = Ok(Value::Unit);
                    if let Some((last, init)) = statements.split_last() {
                        for statement in init {
                            if let Err(e) = self.eval_statement(statement) {
                                result = Err(e);
                                break;
                            }
                        }
                        if result.is_ok() {
                            result = self.eval_statement(last);
                        }
                    }
                    // Pop on every path: a `break`, `return`, or `?` leaves
                    // through Err and must not strand the frame.
                    if let Some(parent) = self.environment.parent.take() {
                        self.environment =
                            Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                    }
                    return result;
                }
                let Some((last, init)) = statements.split_last() else {
                    return Ok(Value::Unit);
                };
                for statement in init {
                    self.eval_statement(statement)?;
                }
                self.eval_statement(last)
            }
            Expr::BinaryOp { left, op, right } => {
                // The logical operators short-circuit: the right operand is
                // evaluated only when the left doesn't already settle the
                // result. `false && x` is false and `true || x` is true
                // without touching `x` — so guards like
                // `i < len && ok(at(i))` are safe. The non-short-circuiting
                // path still runs through eval_binary_op for its type checks.
                match op {
                    BinaryOp::And => {
                        let left = self.eval_expr(left)?;
                        if matches!(left, Value::Boolean(false)) {
                            return Ok(Value::Boolean(false));
                        }
                        let right = self.eval_expr(right)?;
                        self.eval_binary_op(left, op.clone(), right)
                    }
                    BinaryOp::Or => {
                        let left = self.eval_expr(left)?;
                        if matches!(left, Value::Boolean(true)) {
                            return Ok(Value::Boolean(true));
                        }
                        let right = self.eval_expr(right)?;
                        self.eval_binary_op(left, op.clone(), right)
                    }
                    _ => {
                        let left = self.eval_expr(left)?;
                        let right = self.eval_expr(right)?;
                        self.eval_binary_op(left, op.clone(), right)
                    }
                }
            }
            Expr::UnaryOp { op, operand } => {
                let operand = self.eval_expr(operand)?;
                self.eval_unary_op(op.clone(), operand)
            }
            Expr::Range {
                start,
                end,
                inclusive,
            } => {
                let start_val = self.eval_expr(start)?;
                let end_val = self.eval_expr(end)?;
                self.eval_range(start_val, end_val, *inclusive)
            }
            Expr::StructLiteral(struct_literal) => self.eval_struct_literal(struct_literal),
            Expr::AnonymousObject { fields } => self.eval_anonymous_object(fields),
            Expr::MapLiteral { entries } => self.eval_map_literal(entries),
            Expr::FieldAccess { object, field } => self.eval_field_access(object, field),
            Expr::ResultOk(expr) => {
                let value = self.eval_expr(expr)?;
                Ok(Value::Ok(Box::new(value)))
            }
            Expr::ResultErr(expr) => {
                let value = self.eval_expr(expr)?;
                Ok(Value::Err(Box::new(value)))
            }
            Expr::Try(expr) => {
                let value = self.eval_expr(expr)?;
                match value {
                    Value::Ok(inner) => Ok(*inner),
                    // Early-return: unwind to the enclosing function call,
                    // which returns this Err to its caller.
                    err @ Value::Err(_) => Err(InterpreterError::ErrPropagation(err)),
                    _ => Err(InterpreterError::TypeError {
                        message: "Try operator can only be used on Result values".to_string(),
                    }),
                }
            }
            Expr::ForLoop {
                variable,
                iterable,
                body,
            } => self.eval_for_loop(variable, iterable, body),
            Expr::ParForLoop {
                variable,
                iterable,
                body,
            } => self.eval_par_for_loop(variable, iterable, body),
            Expr::WhileLoop { condition, body } => self.eval_while_loop(condition, body),
            Expr::Loop { body } => self.eval_loop(body),
            Expr::Break(value) => {
                let v = match value {
                    Some(e) => self.eval_expr(e)?,
                    None => Value::Unit,
                };
                Err(InterpreterError::BreakSignal(v))
            }
            Expr::Continue => Err(InterpreterError::ContinueSignal),
            Expr::Return(value) => {
                let v = match value {
                    Some(e) => self.eval_expr(e)?,
                    None => Value::Unit,
                };
                Err(InterpreterError::ReturnSignal(v))
            }
            Expr::Assignment { target, value } => {
                if let Some(result) = self.try_fused_list_write(target, value) {
                    return result;
                }
                if let Some(result) = self.try_fused_move_call(target, value) {
                    return result;
                }
                // Fuse `xs = xs + [..]` into an in-place extend when xs holds
                // a sole-owned list — the interpreter half of finding #1,
                // turning O(n²) accumulation into O(n). Narrowed to a list
                // *literal* rhs so numeric and string accumulation keep their
                // existing path untouched. Guarded exactly like the bytecode
                // tier: fusion evaluates rhs before touching xs, so rhs must
                // provably assign nothing (else the read order would change),
                // and `try_extend_list`'s `Arc::get_mut` guard copies instead
                // of mutating whenever the list is aliased — so a snapshot, a
                // nested list, or a captured closure is never disturbed.
                if let Expr::BinaryOp {
                    left,
                    op: crate::ast::BinaryOp::Add,
                    right,
                } = value.as_ref()
                    && matches!(right.as_ref(), Expr::List(_))
                    && matches!(left.as_ref(), Expr::Identifier(n) if n == target)
                    && crate::ovm::bytecode::BytecodeCompiler::assignment_free(right)
                {
                    let rhs = self.eval_expr(right)?;
                    if let Value::List(items) = &rhs
                        && self.environment.try_extend_list(target, items)
                    {
                        return self.environment.get(target).ok_or_else(|| {
                            InterpreterError::UndefinedVariable {
                                name: target.clone(),
                            }
                        });
                    }
                    // Aliased, or xs is not a list: finish as an ordinary
                    // `xs + rhs`. rhs is assignment-free, so reading xs now
                    // (after rhs) gives the same value the normal left-first
                    // order would have.
                    let current = self.environment.get(target).ok_or_else(|| {
                        InterpreterError::UndefinedVariable {
                            name: target.clone(),
                        }
                    })?;
                    let val = self.eval_binary_op(current, crate::ast::BinaryOp::Add, rhs)?;
                    self.environment.set(target, val.clone()).or_else(|_| {
                        self.environment.define(target.clone(), val.clone());
                        Ok(())
                    })?;
                    return Ok(val);
                }
                let val = self.eval_expr(value)?;
                self.environment.set(target, val.clone()).or_else(|_| {
                    // If variable not defined, define it
                    self.environment.define(target.clone(), val.clone());
                    Ok(())
                })?;
                Ok(val)
            }
            Expr::RawString(s) => Ok(Value::String(std::sync::Arc::new(s.as_str().to_string()))),
            Expr::TemplateString { parts } => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        crate::ast::TemplatePart::Literal(s) => result.push_str(s),
                        crate::ast::TemplatePart::Interpolation(expr) => {
                            let val = self.eval_expr(expr)?;
                            // For template interpolation, we want raw values without quotes
                            match val {
                                Value::String(s) => result.push_str(&s),
                                Value::Integer(n) => result.push_str(&n.to_string()),
                                Value::Float(x) => result.push_str(&crate::ast::format_float(x)),
                                Value::Boolean(b) => result.push_str(&b.to_string()),
                                other => result.push_str(&format!("{}", other)),
                            }
                        }
                    }
                }
                Ok(Value::String(result.into()))
            }
            Expr::BitwiseOp { left, op, right } => {
                let left_val = self.eval_expr(left)?;
                let right_val = self.eval_expr(right)?;

                match (left_val, right_val) {
                    (Value::Integer(l), Value::Integer(r)) => {
                        let shift_amount = |r: i64| {
                            u32::try_from(r).ok().filter(|s| *s < 64).ok_or_else(|| {
                                InterpreterError::RuntimeError {
                                    message: format!(
                                        "Shift amount {} out of range (must be 0..64)",
                                        r
                                    ),
                                }
                            })
                        };
                        let result = match op {
                            crate::ast::BitwiseOp::And => l & r,
                            crate::ast::BitwiseOp::Or => l | r,
                            crate::ast::BitwiseOp::Xor => l ^ r,
                            crate::ast::BitwiseOp::Shl => l.wrapping_shl(shift_amount(r)?),
                            crate::ast::BitwiseOp::Shr => l.wrapping_shr(shift_amount(r)?),
                        };
                        Ok(Value::Integer(result))
                    }
                    _ => Err(InterpreterError::TypeError {
                        message: "integer operands required for bitwise operation".to_string(),
                    }),
                }
            }
            Expr::Spread(expr) => {
                // For now, just evaluate the inner expression
                // Spread semantics would be handled at the call site
                self.eval_expr(expr)
            }
            Expr::Rest(expr) => {
                // For now, just evaluate the inner expression
                // Rest semantics would be handled in pattern matching
                self.eval_expr(expr)
            }
            Expr::Index { object, index } => {
                let object_value = self.eval_expr(object)?;
                let index_value = self.eval_expr(index)?;

                match (object_value, index_value) {
                    (Value::List(list), Value::Integer(idx)) => {
                        let index = if idx < 0 {
                            // Negative indexing from end
                            (list.len() as i64 + idx) as usize
                        } else {
                            idx as usize
                        };

                        if index < list.len() {
                            Ok(list[index].clone())
                        } else {
                            Err(InterpreterError::RuntimeError {
                                message: format!(
                                    "Index {} out of bounds for list of length {}",
                                    idx,
                                    list.len()
                                ),
                            })
                        }
                    }
                    (Value::Tuple(tuple), Value::Integer(idx)) => {
                        let index = if idx < 0 {
                            // Negative indexing from end
                            (tuple.len() as i64 + idx) as usize
                        } else {
                            idx as usize
                        };

                        if index < tuple.len() {
                            Ok(tuple[index].clone())
                        } else {
                            Err(InterpreterError::RuntimeError {
                                message: format!(
                                    "Index {} out of bounds for tuple of length {}",
                                    idx,
                                    tuple.len()
                                ),
                            })
                        }
                    }
                    (Value::String(string), Value::Integer(idx)) => {
                        let string_len = string.chars().count();
                        let index = if idx < 0 {
                            // Negative indexing from end
                            if (-idx) as usize > string_len {
                                return Err(InterpreterError::RuntimeError {
                                    message: format!(
                                        "Index {} out of bounds for string of length {}",
                                        idx, string_len
                                    ),
                                });
                            }
                            string_len - ((-idx) as usize)
                        } else {
                            idx as usize
                        };

                        if let Some(ch) = string.chars().nth(index) {
                            // More efficient: create single-char string directly
                            Ok(Value::String(ch.to_string().into()))
                        } else {
                            Err(InterpreterError::RuntimeError {
                                message: format!(
                                    "Index {} out of bounds for string of length {}",
                                    idx, string_len
                                ),
                            })
                        }
                    }
                    // A native value gets first refusal on its own
                    // subscript, before either generic error: a Frame
                    // indexed by a name it does not have can say which
                    // names it does have.
                    (Value::Native(handle), key) => match handle.0.index(&key) {
                        Some(Ok(value)) => Ok(value),
                        Some(Err(message)) => Err(InterpreterError::RuntimeError { message }),
                        None => Err(InterpreterError::TypeError {
                            message: format!("A {} cannot be indexed", handle.0.type_name()),
                        }),
                    },
                    // A map (or struct/object) is read by key, not position:
                    // `[]` never applies, so say so rather than complaining
                    // about the key's type.
                    (Value::Map(_), _) => Err(InterpreterError::TypeError {
                        message: "a Map is not indexed with `[]`; read a key with \
                                  `map_get(m, key)`"
                            .to_string(),
                    }),
                    (Value::Struct { .. }, _) => Err(InterpreterError::TypeError {
                        message: "a struct or object is read by field (`value.name`) or with \
                                  `map_get(value, name)`, not with `[]`"
                            .to_string(),
                    }),
                    (_, Value::Integer(_)) => Err(InterpreterError::TypeError {
                        message: "Only lists, tuples, and strings can be indexed".to_string(),
                    }),
                    (_, _) => Err(InterpreterError::TypeError {
                        message: "Index must be an integer".to_string(),
                    }),
                }
            }
            Expr::Spawn(expression) => {
                if self.meta_mode {
                    return Err(InterpreterError::RuntimeError {
                        message: "spawn is not available at expansion time: a meta fn is a \
                                  pure function of its arguments (docs/macros.md)"
                            .to_string(),
                    });
                }
                // Real background execution: the expression evaluates on its
                // own OS thread against a thread-safe clone of this
                // interpreter — the same worker pattern http.serve uses. The
                // clone snapshots current bindings, so `spawn` captures by
                // value, exactly like closures do. `task.join` joins the
                // thread (memoized, so a cloned handle can be joined more
                // than once).
                let mut worker = self.thread_safe_clone();
                let expr = expression.as_ref().clone();
                let task_id = spawn_registry::next_id();
                let handle = std::thread::Builder::new()
                    .name(format!("olang-spawn-{}", task_id))
                    .stack_size(64 * 1024 * 1024)
                    .spawn(move || {
                        // In the stall detector's census for its whole
                        // life: this thread runs olang code.
                        let _live = crate::stdlib::chan::live_guard();
                        let outcome = worker.eval_expr(&expr).map_err(|e| e.to_string());
                        // A task that ends by raising says so, once, on
                        // stderr — as an uncaught error in main would. The
                        // Err still reaches `task.join`; what this adds is
                        // the line for the task nobody joins (a channel
                        // service whose death otherwise leaves every
                        // `chan.recv` on it waiting forever, silently).
                        if let Err(e) = &outcome {
                            eprintln!("task olang-spawn-{} failed: {}", task_id, e);
                        }
                        outcome
                    })
                    .map_err(|e| InterpreterError::RuntimeError {
                        message: format!("spawn: could not start thread: {}", e),
                    })?;
                spawn_registry::register(task_id, handle);
                Ok(crate::stdlib::task::handle(task_id))
            }

            // Test assertions
            Expr::AssertEq {
                actual,
                expected,
                message,
            } => {
                let actual_val = self.eval_expr(actual)?;
                let expected_val = self.eval_expr(expected)?;
                if actual_val != expected_val {
                    let msg = message.clone().unwrap_or_else(|| {
                        format!("Assertion failed: {:?} != {:?}", actual_val, expected_val)
                    });
                    return Err(InterpreterError::RuntimeError { message: msg });
                }
                Ok(Value::Unit)
            }
            Expr::AssertNe {
                actual,
                expected,
                message,
            } => {
                let actual_val = self.eval_expr(actual)?;
                let expected_val = self.eval_expr(expected)?;
                if actual_val == expected_val {
                    let msg = message.clone().unwrap_or_else(|| {
                        format!("Assertion failed: {:?} == {:?}", actual_val, expected_val)
                    });
                    return Err(InterpreterError::RuntimeError { message: msg });
                }
                Ok(Value::Unit)
            }
            Expr::Assert { condition, message } => {
                let condition_val = self.eval_expr(condition)?;
                match condition_val {
                    Value::Boolean(true) => Ok(Value::Unit),
                    Value::Boolean(false) => {
                        let msg = message
                            .clone()
                            .unwrap_or_else(|| "Assertion failed: condition is false".to_string());
                        Err(InterpreterError::RuntimeError { message: msg })
                    }
                    _ => {
                        let msg = message.clone().unwrap_or_else(|| {
                            format!(
                                "Assertion failed: condition is not boolean: {:?}",
                                condition_val
                            )
                        });
                        Err(InterpreterError::RuntimeError { message: msg })
                    }
                }
            }
            Expr::AssertTrue {
                expression,
                message,
            } => {
                let val = self.eval_expr(expression)?;
                match val {
                    Value::Boolean(true) => Ok(Value::Unit),
                    _ => {
                        let msg = message.clone().unwrap_or_else(|| {
                            format!("Assertion failed: expected true, got {:?}", val)
                        });
                        Err(InterpreterError::RuntimeError { message: msg })
                    }
                }
            }
            Expr::AssertFalse {
                expression,
                message,
            } => {
                let val = self.eval_expr(expression)?;
                match val {
                    Value::Boolean(false) => Ok(Value::Unit),
                    _ => {
                        let msg = message.clone().unwrap_or_else(|| {
                            format!("Assertion failed: expected false, got {:?}", val)
                        });
                        Err(InterpreterError::RuntimeError { message: msg })
                    }
                }
            }
        }
    }

    /// Enable promotion of hot functions to the OVM bytecode tier.
    ///
    /// Promotion never changes program behavior: anything the tier can't
    /// compile (closures, unsupported expressions, unresolved callees) stays
    /// interpreted. See `crate::ovm::tier` and the differential test suite.
    /// Set the logical call-depth cap (`--max-depth`). Applied to the
    /// bytecode tier too, present or future, so both tiers raise the
    /// same "Maximum call depth" error at the same depth.
    /// Bound a run: `max_steps` loop iterations and calls (deterministic),
    /// and/or a wall-clock `timeout`. Exceeding either raises a runtime
    /// error naming the bound — `meta.eval`'s sandbox for user-authored
    /// source, where a `while true {}` must cost one failure, not a thread.
    pub fn set_eval_budget(
        &mut self,
        max_steps: Option<u64>,
        timeout: Option<std::time::Duration>,
    ) {
        self.step_budget = max_steps;
        self.deadline = timeout.map(|t| (crate::clock::Instant::now(), t));
    }

    /// Is this run bounded? A bounded run stays on the interpreter —
    /// promotion would move iterations and calls where they are not
    /// charged.
    #[inline]
    fn budgeted(&self) -> bool {
        self.step_budget.is_some() || self.deadline.is_some()
    }

    /// Charge one step against the evaluation budget, if one is set.
    #[inline]
    fn spend_step(&mut self) -> Result<(), InterpreterError> {
        if let Some(left) = self.step_budget.as_mut() {
            if *left == 0 {
                return Err(InterpreterError::RuntimeError {
                    message: "budget exceeded: the evaluation used up its max_steps".to_string(),
                });
            }
            *left -= 1;
        }
        if let Some((started, limit)) = &self.deadline
            && started.elapsed() >= *limit
        {
            return Err(InterpreterError::RuntimeError {
                message: format!(
                    "budget exceeded: the evaluation ran past its timeout_ms ({} ms)",
                    limit.as_millis()
                ),
            });
        }
        Ok(())
    }

    pub fn set_max_call_depth(&mut self, depth: usize) {
        self.max_call_depth = depth.max(1);
        if let Some(tier) = self.bytecode_tier.as_mut() {
            tier.set_max_call_depth(self.max_call_depth as u32);
        }
    }

    pub fn enable_bytecode_tier(&mut self, threshold: u32, verbose: bool) {
        let mut tier = crate::ovm::tier::BytecodeTier::new(threshold).with_verbose(verbose);
        // Order-independent: capabilities may be installed before or after
        // the tier is turned on, and the tier must enforce either way.
        tier.set_capabilities(self.caps.clone());
        tier.set_entry_file(self.entry_file.clone());
        // The tiers share one logical depth cap; a tier created after
        // --max-depth was applied must inherit it.
        tier.set_max_call_depth(self.max_call_depth as u32);
        if let Some(trace) = self.caps_trace.clone() {
            tier.set_caps_trace(trace);
        }
        self.bytecode_tier = Some(Box::new(tier));
        if let Some(profile) = self.warm_profile.clone()
            && let Some(t) = self.bytecode_tier.as_mut()
        {
            t.set_warm_profile(&profile);
        }
    }

    /// Install a previous run's warm profile (see `ovm::warm`); applied
    /// to the tier now if it exists, or when one is enabled.
    pub fn set_warm_profile(&mut self, profile: crate::ovm::warm::WarmProfile) {
        if let Some(t) = self.bytecode_tier.as_mut() {
            t.set_warm_profile(&profile);
        }
        self.warm_profile = Some(profile);
    }

    /// What this run's tier learned, for the next run of this source.
    pub fn collect_warm_profile(&self) -> crate::ovm::warm::WarmProfile {
        self.bytecode_tier
            .as_ref()
            .map(|t| t.collect_warm_profile())
            .unwrap_or_default()
    }

    pub fn bytecode_tier_stats(&self) -> Option<crate::ovm::tier::TierStats> {
        self.bytecode_tier.as_ref().map(|t| t.stats())
    }

    /// The tier itself, read-only. `:ovm` renders its full insight
    /// surface: rejections with reasons, per-function tiers, VM counters.
    pub fn bytecode_tier_ref(&self) -> Option<&crate::ovm::tier::BytecodeTier> {
        self.bytecode_tier.as_deref()
    }

    /// The modules this run has loaded, as (registered name, file)
    /// pairs — the REPL's `:help` maps a module-qualified query
    /// (`geometry.point`) to the file behind the name, which a bare
    /// file stem cannot do (a package's entry file is `index.ol`).
    pub fn loaded_modules(&self) -> Vec<(String, std::path::PathBuf)> {
        let mut out: Vec<(String, std::path::PathBuf)> = self
            .module_name_index
            .iter()
            .filter(|(_, p)| !p.to_string_lossy().starts_with("__"))
            .map(|(k, p)| (k.clone(), p.clone()))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Files backing the modules this run has loaded — the REPL's
    /// `:help` scans them for `///` doc comments, so a user's own
    /// documented functions are as reachable as the builtins.
    pub fn loaded_module_files(&self) -> Vec<std::path::PathBuf> {
        let mut out: Vec<std::path::PathBuf> = self
            .module_cache
            .values()
            .filter_map(|e| e.file_path.clone())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The per-function tier report (`OLANG_TIER_STATS=1`): every
    /// VM-callable name with its native call count and kinds.
    pub fn tier_report(&self) -> Vec<(String, u64, Vec<String>)> {
        self.bytecode_tier
            .as_ref()
            .map(|t| t.tier_report())
            .unwrap_or_default()
    }

    /// Seed the declaration-level state a *bridge* interpreter needs to run
    /// user code faithfully. The bytecode VM bridges builtins it cannot run
    /// natively (e.g. `fold`) back to a throwaway interpreter; when such a
    /// builtin invokes a user lambda that dispatches a trait method
    /// (`x.tag()`) or constructs a declared struct, that interpreter must
    /// carry the same trait/struct/variant tables the program declared, or the
    /// call diverges from the tree-walk (a spurious "Field not found"). The VM
    /// mirrors these facts as declarations evaluate and installs them here.
    // One setter per parallel declaration registry; bundling them into a
    // struct would only move the argument list, not shorten it.
    #[allow(clippy::too_many_arguments)]
    pub fn seed_bridge_state(
        &mut self,
        trait_impls: HashMap<(String, String), Function>,
        trait_defaults: HashMap<(String, String), Function>,
        type_traits: HashMap<String, Vec<String>>,
        struct_defs: HashMap<String, Vec<String>>,
        struct_field_checks: HashMap<String, HashMap<String, crate::ast::FieldTypeCheck>>,
        unit_variant_names: HashSet<String>,
        enum_type_names: HashSet<String>,
    ) {
        self.trait_impls = trait_impls;
        self.trait_defaults = trait_defaults;
        self.type_traits = type_traits;
        self.struct_defs = struct_defs;
        self.struct_field_checks = struct_field_checks;
        self.unit_variant_names = unit_variant_names;
        self.enum_type_names = enum_type_names;
        // The bridge's own tier compiles against the same declaration
        // landscape: without this forwarding, a bridged fold whose
        // lambda dispatches a trait method promoted into a VM that had
        // never heard of the trait.
        if let Some(tier) = self.bytecode_tier.as_mut() {
            for (name, fields) in self.struct_defs.clone() {
                let checks = self
                    .struct_field_checks
                    .get(&name)
                    .cloned()
                    .unwrap_or_default();
                tier.note_struct(name, fields, checks);
            }
            for ((type_name, method), func) in self.trait_impls.clone() {
                tier.note_trait_impl(type_name, method, func);
            }
            for ((trait_name, method), func) in self.trait_defaults.clone() {
                tier.note_trait_default(trait_name, method, func);
            }
            for (type_name, traits) in self.type_traits.clone() {
                for trait_name in traits {
                    tier.note_type_trait(type_name.clone(), trait_name);
                }
            }
            for name in self.unit_variant_names.clone() {
                tier.note_unit_variant(name);
            }
            for name in self.enum_type_names.clone() {
                tier.note_enum_type(name);
            }
        }
    }

    /// Is `name` a declared type — a struct or an enum? Primitive checks
    /// never reach here (they are their own `FieldTypeCheck` variants), so a
    /// `Named` check that is neither a known struct nor a known enum names a
    /// type that does not exist, and the enforcer says so.
    fn is_declared_type(&self, name: &str) -> bool {
        self.struct_defs.contains_key(name) || self.enum_type_names.contains(name)
    }

    /// The error text for a failed annotation check at `site`. When the
    /// declared type is a named type that was never declared, the value is
    /// not the problem — the annotation is — so it says "unknown type"
    /// rather than "expects X, got Y", the way construction already reports
    /// an undeclared struct name.
    fn annotation_error(
        &self,
        site: &str,
        check: &crate::ast::FieldTypeCheck,
        expected: &str,
        got: &str,
    ) -> String {
        match check.named_type() {
            Some(name) if !self.is_declared_type(name) => format!(
                "{site} names unknown type '{name}' — declare it with \
                 `type {name} = struct {{ ... }}` (or `enum`), or annotate with a known type"
            ),
            _ => format!("{site} expects {expected}, got {got}"),
        }
    }

    /// Give a bridge interpreter the capability context of the compiled
    /// function that is calling through it: the run's grant table, the
    /// shared `--trace-caps` set, and the file whose code is making the
    /// call. The gate in builtin dispatch reads exactly these, so a
    /// promoted function is judged by the same rule and the same package
    /// attribution as the interpreted one it replaced.
    pub fn seed_bridge_caps(
        &mut self,
        caps: Option<std::sync::Arc<crate::caps::CapTable>>,
        trace: Option<
            std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<crate::caps::CapUse>>>,
        >,
        attributed_to: Option<String>,
    ) {
        self.caps = caps;
        self.caps_trace = trace;
        // One frame, replaced per dispatch: the bridge runs one builtin at
        // a time and `current_caps` reads only the top of this stack.
        self.coverage_file_stack.clear();
        self.coverage_file_stack.push(attributed_to);
        // The bridge's own tier enforces the same grant: seeding the
        // fields alone would leave a tier enabled earlier holding the
        // table from a previous dispatch.
        if let Some(tier) = self.bytecode_tier.as_mut() {
            tier.set_capabilities(self.caps.clone());
            if let Some(trace) = self.caps_trace.clone() {
                tier.set_caps_trace(trace);
            }
        }
    }

    /// Seed the interpreter's frame counter with the frames already live
    /// in the caller — the bridge-interpreter side of the shared
    /// call-depth budget (the mirror of the VM's `set_depth_base`), so a
    /// recursion that crosses the tier boundary in either direction
    /// errors at exactly the depth a single-tier run would.
    pub fn set_call_depth_base(&mut self, depth: usize) {
        self.call_depth = depth;
    }

    /// Call a user function by reference — the hot path `map`,
    /// `filter`, the parallel workers, and every higher-order builtin
    /// go through. `call_function` used to be the only entry, and it
    /// takes the callee by VALUE: every per-element call cloned the
    /// whole `Function` — its parameter vector, its name, its check
    /// Fuse `x = col.set(x, i, v)` / `x = col.swap(x, i, j)` into an
    /// in-place write when `x` holds the only reference to its list —
    /// the indexed twin of the `xs = xs + [..]` extend fusion, and the
    /// primitive that makes the olang-source collections O(1) per
    /// write. Returns `None` when the shape doesn't match (the caller
    /// evaluates normally), `Some(result)` when it does — where an
    /// aliased or non-list binding has already been finished on the
    /// ordinary copy path, so the answer is identical either way.
    ///
    /// The guards, in order: the callee must be the *builtin* `col.set`
    /// or `col.swap` (resolved through the environment, so a shadowed
    /// `col` is somebody else's and declines); all three arguments
    /// positional, the first naming the assignment target itself; the
    /// index/value arguments assignment-free, because fusion evaluates
    /// them before touching `x` and an argument that wrote `x` would
    /// observe the wrong order.
    fn try_fused_list_write(
        &mut self,
        target: &str,
        value: &Expr,
    ) -> Option<Result<Value, InterpreterError>> {
        let Expr::Call { callee, arguments } = value else {
            return None;
        };
        let Expr::FieldAccess { object, field } = callee.as_ref() else {
            return None;
        };
        if !matches!(object.as_ref(), Expr::Identifier(m) if m == "col")
            || !matches!(field.as_str(), "set" | "swap")
            || arguments.len() != 3
        {
            return None;
        }
        let mut exprs = Vec::with_capacity(3);
        for argument in arguments {
            match argument {
                crate::ast::Argument::Positional(e) => exprs.push(e),
                crate::ast::Argument::Named { .. } => return None,
            }
        }
        let names_target = matches!(exprs[0], Expr::Identifier(n) if n == target)
            || matches!(exprs[0], Expr::LocalRef { name, .. } if name == target);
        if !names_target
            || !crate::ovm::bytecode::BytecodeCompiler::assignment_free(exprs[1])
            || !crate::ovm::bytecode::BytecodeCompiler::assignment_free(exprs[2])
        {
            return None;
        }
        // `col` must still be the builtin module, and the field its
        // builtin — a user's own `col` binding takes the normal path.
        let is_builtin = matches!(
            self.environment.get("col"),
            Some(Value::Struct { type_name, fields })
                if type_name == "Module"
                    && matches!(
                        fields.get(field.as_str()),
                        Some(Value::Builtin(b)) if b.name == format!("col.{}", field)
                    )
        );
        if !is_builtin {
            return None;
        }

        let swap = field == "swap";
        let a1 = match self.eval_expr(exprs[1]) {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };
        let a2 = match self.eval_expr(exprs[2]) {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };
        // Int indexes only on the fused path; anything else falls back
        // to the builtin, whose type errors are the single source of
        // truth.
        let in_place: Option<Result<(), String>> = match (&a1, &a2, swap) {
            (Value::Integer(i), _, false) => {
                let i = *i;
                let v = a2.clone();
                self.environment.try_list_update(target, |items| {
                    let at = crate::stdlib::collections::resolve_index("set", i, items.len())?;
                    items[at] = v;
                    Ok(())
                })
            }
            (Value::Integer(i), Value::Integer(j), true) => {
                let (i, j) = (*i, *j);
                self.environment.try_list_update(target, |items| {
                    let a = crate::stdlib::collections::resolve_index("swap", i, items.len())?;
                    let b = crate::stdlib::collections::resolve_index("swap", j, items.len())?;
                    items.swap(a, b);
                    Ok(())
                })
            }
            _ => None,
        };
        match in_place {
            Some(Ok(())) => Some(self.environment.get(target).ok_or_else(|| {
                InterpreterError::UndefinedVariable {
                    name: target.to_string(),
                }
            })),
            Some(Err(message)) => Some(Err(InterpreterError::RuntimeError { message })),
            // Aliased or not a list: the ordinary builtin call, with the
            // already-evaluated arguments (assignment-free, so reading
            // the target now matches the normal left-to-right order).
            None => {
                let current = match self.environment.get(target) {
                    Some(v) => v,
                    None => {
                        return Some(Err(InterpreterError::UndefinedVariable {
                            name: target.to_string(),
                        }));
                    }
                };
                let result = crate::stdlib::collections::call_collections_function(
                    field,
                    vec![current, a1, a2],
                    self,
                );
                match result {
                    Ok(val) => {
                        if let Err(e) = self.environment.set(target, val.clone()).or_else(|_| {
                            self.environment.define(target.to_string(), val.clone());
                            Ok::<(), InterpreterError>(())
                        }) {
                            return Some(Err(e));
                        }
                        Some(Ok(val))
                    }
                    Err(e) => Some(Err(e)),
                }
            }
        }
    }

    /// Fuse `x = f(x, ...)` — a call to a user function that rebinds
    /// its own first argument — into a *move*: `x` is taken out of its
    /// slot (leaving Unit) rather than copied, so the callee receives
    /// the value solely owned and its own writes stay in place. This is
    /// the calling convention the olang-source collections are written
    /// against (`h = heap.push(h, p, v)`), and what makes a
    /// handle-based API O(1) per operation instead of O(n) at every
    /// call boundary — for any olang library, not only the bundled
    /// ones. The value that arrives is
    /// identical; only reference counts differ, which is unobservable
    /// except as speed. Remaining arguments must be assignment-free
    /// (they evaluate before the take; one that wrote `x` would observe
    /// the wrong order), and a shared scope declines the take and calls
    /// with a copy, exactly as before.
    fn try_fused_move_call(
        &mut self,
        target: &str,
        value: &Expr,
    ) -> Option<Result<Value, InterpreterError>> {
        let Expr::Call { callee, arguments } = value else {
            return None;
        };
        // The callee must be a *pure lookup* — a bare name or a
        // `module.field` on a bare name — because deciding to move
        // means evaluating it before the arguments, and an effectful
        // callee expression would run out of order.
        fn is_pure_lookup(e: &Expr) -> bool {
            match e {
                Expr::Identifier(_) | Expr::LocalRef { .. } => true,
                // A field chain over a bare name — `collections.heap.push`
                // — is still lookups all the way down.
                Expr::FieldAccess { object, .. } => is_pure_lookup(object),
                _ => false,
            }
        }
        let callee_is_lookup = is_pure_lookup(callee.as_ref());
        if !callee_is_lookup || arguments.is_empty() {
            return None;
        }
        let mut exprs = Vec::with_capacity(arguments.len());
        for argument in arguments {
            match argument {
                crate::ast::Argument::Positional(e) => exprs.push(e),
                crate::ast::Argument::Named { .. } => return None,
            }
        }
        let names_target = matches!(exprs[0], Expr::Identifier(n) if n == target)
            || matches!(exprs[0], Expr::LocalRef { name, .. } if name == target);
        if !names_target {
            return None;
        }
        if !exprs[1..]
            .iter()
            .all(|e| crate::ovm::bytecode::BytecodeCompiler::assignment_free(e))
        {
            return None;
        }
        // Resolve the callee — a pure lookup (auto-loading the module on
        // first touch, like any other reference to it). Only a Function
        // from the module value takes the move path; anything else (a
        // shadowed name, a builtin) declines.
        let callee_value = match self.eval_expr(callee) {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };
        // User functions take the move path; among builtins, exactly
        // map_set does too — its receiver is Arc-backed and the builtin
        // inserts in place when the moved argument makes it sole-owner
        // (the fix for the quadratic hash-count loop the wordfreq
        // benchmark exposed). Other builtins keep the plain path.
        let is_movable_builtin = matches!(&callee_value, Value::Builtin(b) if b.name == "map_set");
        if !matches!(callee_value, Value::Function(_)) && !is_movable_builtin {
            return None;
        }
        let mut args = Vec::with_capacity(exprs.len());
        // Arguments after the first evaluate before the take, in their
        // ordinary order.
        let mut rest = Vec::with_capacity(exprs.len() - 1);
        for e in &exprs[1..] {
            match self.eval_expr(e) {
                Ok(v) => rest.push(v),
                Err(err) => return Some(Err(err)),
            }
        }
        let moved = match self.environment.take_for_move(target) {
            Some(v) => {
                if std::env::var_os("OLANG_DEBUG_ASTLIST").is_some()
                    && let Value::List(items) = &v
                    && items.len() > 64
                {
                    eprintln!(
                        "[astlist] taken rc={} len={}",
                        std::sync::Arc::strong_count(items),
                        items.len()
                    );
                }
                v
            }
            None => match self.environment.get(target) {
                Some(v) => v,
                None => {
                    return Some(Err(InterpreterError::UndefinedVariable {
                        name: target.to_string(),
                    }));
                }
            },
        };
        args.push(moved);
        args.extend(rest);
        let result = self.call_function(callee_value, args);
        match result {
            Ok(val) => {
                if let Err(e) = self.environment.set(target, val.clone()).or_else(|_| {
                    self.environment.define(target.to_string(), val.clone());
                    Ok::<(), InterpreterError>(())
                }) {
                    return Some(Err(e));
                }
                Some(Ok(val))
            }
            Err(e) => Some(Err(e)),
        }
    }

    /// The checks every call crosses: arity against required and total
    /// parameters, trait bounds, and declared parameter types. One
    /// function because an *elided* tail frame must re-run exactly what
    /// a real frame would have (see the trampoline below) — a drifted
    /// copy would make deep recursion the one place annotations stop
    /// being promises.
    fn check_call_boundary(
        &mut self,
        func: &Function,
        arguments: &[Value],
    ) -> Result<(), InterpreterError> {
        let required_params = func
            .parameters
            .iter()
            .filter(|p| p.default_value.is_none())
            .count();
        if arguments.len() < required_params {
            return Err(InterpreterError::ArityMismatch {
                expected: required_params,
                got: arguments.len(),
            });
        }
        if arguments.len() > func.parameters.len() {
            return Err(InterpreterError::ArityMismatch {
                expected: func.parameters.len(),
                got: arguments.len(),
            });
        }
        // Trait bounds fail at the boundary with a clear message, not
        // deep inside the body; declared parameter types run before the
        // tier so every execution path sees the same boundary.
        if !func.param_bounds.is_empty() {
            self.check_param_bounds(func, arguments)?;
        }
        if !func.param_checks.is_empty() {
            self.check_param_types(func, arguments)?;
        }
        Ok(())
    }

    /// tables — heap allocations per element, and allocator contention
    /// once a dozen cores did it at once. Borrowing removes the clone;
    /// nothing here needed ownership.
    /// Produce the full positional argument vector for `func`,
    /// evaluating defaults for the missing positions. A default
    /// evaluates as if it were the first statement of the body: in a
    /// fresh callee environment carrying the function's closure and
    /// its own name, with every earlier parameter already bound — so
    /// `fn f(x, y = x * 2)` sees the parameter `x`, and a caller-side
    /// variable that happens to share a default's name is invisible.
    /// Left to right, once per call.
    fn fill_default_arguments(
        &mut self,
        func: &Function,
        slots: Vec<ArgSlot>,
    ) -> Result<Vec<Value>, InterpreterError> {
        let mut env = Environment::with_parent(self.environment.clone());
        if !func.closure.is_empty() {
            env.variables = func.closure.clone();
        }
        if let Some(name) = &func.name {
            env.define_local(name.clone(), Value::Function(func.clone()));
        }
        let saved = std::mem::replace(&mut self.environment, env);
        let mut slots = slots.into_iter();
        let mut filled = Vec::with_capacity(func.parameters.len());
        let mut fill_error = None;
        for param in func.parameters.iter() {
            let value = match slots.next() {
                Some(ArgSlot::Given(v)) => Ok(v),
                Some(ArgSlot::FromDefault) | None => match &param.default_value {
                    Some(default_expr) => self.eval_expr(default_expr),
                    None => Err(InterpreterError::RuntimeError {
                        message: format!("Missing argument for parameter {}", param.name),
                    }),
                },
            };
            match value {
                Ok(v) => {
                    self.environment.define_local(param.name.clone(), v.clone());
                    filled.push(v);
                }
                Err(e) => {
                    fill_error = Some(e);
                    break;
                }
            }
        }
        self.environment = saved;
        match fill_error {
            Some(e) => Err(e),
            None => Ok(filled),
        }
    }

    /// `call_user_function` for a slot vector from named-argument
    /// resolution (holes fill from defaults in the callee's scope).
    pub fn call_user_function_slots(
        &mut self,
        func: &Function,
        slots: Vec<ArgSlot>,
    ) -> Result<Value, InterpreterError> {
        let needs_fill = slots.len() < func.parameters.len()
            || slots.iter().any(|s| matches!(s, ArgSlot::FromDefault));
        let arguments = if needs_fill {
            self.fill_default_arguments(func, slots)?
        } else {
            slots
                .into_iter()
                .map(|s| match s {
                    ArgSlot::Given(v) => v,
                    ArgSlot::FromDefault => unreachable!("needs_fill checked"),
                })
                .collect()
        };
        self.call_user_function(func, arguments)
    }

    pub fn call_user_function(
        &mut self,
        func: &Function,
        mut arguments: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        // A short positional call fills its trailing parameters from
        // their defaults here, before the depth/tier machinery, so the
        // promoted path receives the same full vector the interpreter
        // binds. Only when defaults exist: a short call to a function
        // without them keeps its ArityMismatch from the boundary check.
        if arguments.len() < func.parameters.len()
            && func.parameters.iter().any(|p| p.default_value.is_some())
        {
            let slots = arguments.into_iter().map(ArgSlot::Given).collect();
            arguments = self.fill_default_arguments(func, slots)?;
        }
        if self.call_depth >= self.max_call_depth {
            return Err(InterpreterError::RuntimeError {
                message: format!(
                    "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                    self.max_call_depth
                ),
            });
        }
        // Profiling shadow frame (`olang profile`). Pushed here, at the
        // one entry every user call passes through, and marked
        // Interpreter: if the call is promoted, the tier pushes its own
        // frame underneath and *that* becomes the sampled leaf, so the
        // tier column reports where the work actually ran.
        let profiled = crate::profile::push(
            func.name.as_deref().unwrap_or("<lambda>"),
            crate::profile::Tier::Interpreter,
        );
        // Every frame under the cap must be physically reachable, or the
        // cap is a lie the stack tells first.
        let result = with_stack_headroom(|| self.call_user_function_inner(func, arguments));
        if profiled {
            crate::profile::pop();
        }
        result
    }

    fn call_user_function_inner(
        &mut self,
        func: &Function,
        mut arguments: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        {
            self.spend_step()?;
            // Increment call depth for user functions
            self.call_depth += 1;
            // A frame from another file names it — "open_db (lib/sql.ol)"
            // — so an error inside a dependency is placed by the call
            // stack even when its own location is not known.
            let frame = func.name.clone().unwrap_or_else(|| "<lambda>".to_string());
            let frame = match &func.def_file {
                Some(file)
                    if !file.starts_with("__")
                        && self.entry_file.as_deref() != Some(file.as_str()) =>
                {
                    format!(
                        "{} ({})",
                        frame,
                        Self::short_file(self.entry_file.as_deref(), file)
                    )
                }
                _ => frame,
            };
            self.call_stack_names.push(frame);

            self.check_call_boundary(func, &arguments)?;

            // Hot-function promotion: run on the bytecode tier when the
            // function is eligible, otherwise fall through to the AST walk
            if self.bytecode_tier.is_some()
                && arguments.len() == func.parameters.len()
                && !self.budgeted()
            {
                let globals = self.global_bindings();
                let mut tier = self.bytecode_tier.take();
                // The interpreter's frames (this one included) count
                // against the same budget the VM spends from.
                let depth = self.call_depth as u32;
                let outcome = tier
                    .as_mut()
                    .map(|t| {
                        t.set_host_globals(globals);
                        t.try_call_at_depth(func, &mut arguments, depth)
                    })
                    .unwrap_or(crate::ovm::tier::TierOutcome::Fallback);
                self.bytecode_tier = tier;

                if let crate::ovm::tier::TierOutcome::Ran(result) = outcome {
                    self.call_depth -= 1;
                    self.call_stack_names.pop();
                    return match result {
                        Ok(v) => Ok(v),
                        Err(message) => {
                            let err = Self::map_tier_error_message(message);
                            // The VM tracked where the error happened
                            // (innermost located statement) and which of
                            // its frames were live there — splice them
                            // onto the interpreter's own live stack so
                            // the report is identical to a pure
                            // interpreter run.
                            if self.pending_error_location.is_none()
                                && !Self::is_control_signal(&err)
                            {
                                let (span, frames, leak) = self
                                    .bytecode_tier
                                    .as_mut()
                                    .map(|t| t.take_error_trace())
                                    .unwrap_or_default();
                                if let Some((line, column)) = span {
                                    let trace_file = self
                                        .bytecode_tier
                                        .as_mut()
                                        .and_then(|t| t.take_error_trace_file())
                                        .filter(|f| {
                                            !f.starts_with("__")
                                                && self.entry_file.as_deref() != Some(f.as_str())
                                        });
                                    self.pending_error_location = Some(crate::ast::ErrorLocation {
                                        line,
                                        column,
                                        file: trace_file.or_else(|| self.error_file()),
                                        call_stack: self.splice_tier_stack(frames),
                                        hint: self.pending_error_hint.take(),
                                    });
                                } else if !frames.is_empty() && self.pending_error_frames.is_none()
                                {
                                    // No located statement inside the VM,
                                    // but the frames are real: keep them
                                    // for the located capture upstream.
                                    self.pending_error_frames =
                                        Some(self.splice_tier_stack(frames));
                                } else if let Some(leaked) = leak {
                                    // No located statement inside the VM:
                                    // the capture happens at an enclosing
                                    // interpreter statement — with the
                                    // parameter-check frame still alive,
                                    // exactly like the interpreter's own
                                    // (never-popped) frame.
                                    self.call_stack_names.push(leaked);
                                }
                            }
                            Err(err)
                        }
                    };
                }
            }

            // Coverage: while this body runs, lines belong to the file
            // the function was defined in, not the caller's file.
            // Capability enforcement rides the same stack — a gated
            // builtin call is attributed to the file (and so the
            // package) of the function that made it.
            let track_coverage = self.coverage.is_some() || self.caps.is_some();
            if track_coverage {
                self.coverage_file_stack.push(func.def_file.clone());
            }

            // The tail-call trampoline: a self-call in tail position
            // hands its evaluated arguments back here instead of pushing
            // a frame, so tail recursion runs in one frame at O(1) stack
            // *and* O(1) logical depth — the recursion-depth cap measures
            // frames that are genuinely live. Each elided frame re-runs
            // the same boundary checks a real frame would.
            let mut tail_frames_elided = false;
            let result = loop {
                // Create new environment with current environment as parent
                let mut new_env = Environment::with_parent(self.environment.clone());

                // Adopt the closure as the environment's flat map in O(1) —
                // the persistent map is shared, not copied. This was a loop
                // defining every closure entry (the whole prelude, ~200
                // entries) on every single call.
                if !func.closure.is_empty() {
                    new_env.variables = func.closure.clone();
                }

                // If this is a named function, add it to its own scope for recursion
                if let Some(name) = &func.name {
                    new_env.define_local(name.clone(), Value::Function(func.clone()));
                }

                // Parameters are defined over the shared map; copy-on-write
                // clones only the touched structure. Arguments bind by
                // *move* — the vector's slot is left with Unit — so a
                // value passed here is not also pinned alive by the
                // argument vector for the whole call. That pin was a
                // hidden second reference on every argument, and one
                // extra reference is exactly what turns the callee's
                // sole-owner writes (`h = col.set(h, ...)`) into a full
                // copy per operation.
                let mut bind_error = None;
                for (i, param) in func.parameters.iter().enumerate() {
                    // Defaults were filled at the call boundary (in the
                    // callee's scope, by fill_default_arguments); a short
                    // vector reaching the bind loop is a missing argument.
                    let value = if i < arguments.len() {
                        std::mem::replace(&mut arguments[i], Value::Unit)
                    } else {
                        bind_error = Some(InterpreterError::RuntimeError {
                            message: format!("Missing argument for parameter {}", param.name),
                        });
                        break;
                    };

                    new_env.define_local(param.name.clone(), value);
                }
                if let Some(e) = bind_error {
                    break Err(e);
                }

                let saved = std::mem::replace(&mut self.environment, new_env);
                let flow = self.eval_tail_expr(&func.body, func);
                self.environment = saved;

                match flow {
                    Ok(TailFlow::Value(v)) => break Ok(v),
                    Ok(TailFlow::SelfCall(args)) => {
                        if let Err(e) = self.check_call_boundary(func, &args) {
                            break Err(e);
                        }
                        // A trampolined self-call is a call: it spends a
                        // step like the frames it elides would have.
                        if let Err(e) = self.spend_step() {
                            break Err(e);
                        }
                        arguments = args;
                        // The trace must say frames are missing here, or a
                        // one-frame stack under a deep recursion reads as
                        // a lie.
                        if !tail_frames_elided {
                            tail_frames_elided = true;
                            if let Some(top) = self.call_stack_names.last_mut() {
                                top.push_str(" (tail calls elided)");
                            }
                        }
                    }
                    // `?` hit an Err inside this body: the function returns
                    // that Err to its caller — the early-return semantics.
                    Err(InterpreterError::ErrPropagation(err)) => break Ok(err),
                    // `return v` inside this body: the function's value is v.
                    Err(InterpreterError::ReturnSignal(v)) => break Ok(v),
                    Err(e) => break Err(e),
                }
            };

            if track_coverage {
                self.coverage_file_stack.pop();
            }

            // Enforce the declared return type on whatever the body
            // produced (explicit return or final expression alike).
            let result = match result {
                Ok(v) => {
                    self.check_return_type(func, &v)?;
                    Ok(v)
                }
                other => other,
            };

            // An error leaving this frame: remember the stack as it
            // stands, deepest frame included, before the pop below
            // erases it. The located-statement capture upstream uses
            // this instead of its own (already-unwound) view.
            if let Err(e) = &result
                && !Self::is_control_signal(e)
                && self.pending_error_location.is_none()
                && self.pending_error_frames.is_none()
            {
                self.pending_error_frames = Some(self.call_stack_names.clone());
            }

            // Decrement call depth when function completes
            self.call_depth -= 1;
            self.call_stack_names.pop();

            result
        }
    }

    pub fn call_function(
        &mut self,
        callee: Value,
        arguments: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        // MEMORY PROTECTION: Check recursion depth to prevent exponential memory growth
        if self.call_depth >= self.max_call_depth {
            return Err(InterpreterError::RuntimeError {
                message: format!(
                    "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                    self.max_call_depth
                ),
            });
        }

        match callee {
            Value::Function(func) => self.call_user_function(&func, arguments),
            Value::Builtin(builtin) => {
                let name = builtin.name.clone();
                let builtin_functions = self.builtin_functions.clone();
                BuiltinFunctions::call(&builtin_functions, &name, arguments, self)
            }
            // Applying a tuple-variant constructor builds the enum value
            Value::EnumConstructor {
                type_name,
                variant_name,
                arity,
            } => {
                // call_depth is only incremented in the Function arm, so
                // there is nothing to unwind here
                if arguments.len() != arity {
                    return Err(InterpreterError::ArityMismatch {
                        expected: arity,
                        got: arguments.len(),
                    });
                }
                Ok(Value::Enum {
                    type_name,
                    variant_name,
                    variant_data: crate::ast::EnumVariantData::Tuple(arguments),
                })
            }
            // A module that exports `new` is callable, and calling it *is*
            // calling `new`: `cell(0)` is `cell.new(0)`. Constructing one
            // kind of value is the module's whole purpose in that case, so
            // the shorter spelling costs no clarity — and the long form
            // stays available and means exactly the same thing.
            Value::Struct {
                ref type_name,
                ref fields,
            } if type_name == "Module" => match fields.get("new") {
                Some(constructor) => {
                    let constructor = constructor.clone();
                    self.call_function(constructor, arguments)
                }
                None => Err(InterpreterError::TypeError {
                    message: "Cannot call a module that has no `new`".to_string(),
                }),
            },
            other => Err(InterpreterError::TypeError {
                message: format!(
                    "cannot call {}: it is a value, not a function",
                    Self::with_article(&other.type_name())
                ),
            }),
        }
    }

    /// The program's top-level bindings — the root of the scope chain —
    /// as the persistent map they live in. Cheap to clone (an Arc), and
    /// its pointer moves exactly when a top-level binding does.
    fn global_bindings(&self) -> Arc<ImHashMap<String, Value>> {
        let mut env = &self.environment;
        while let Some(parent) = env.parent.as_deref() {
            env = parent;
        }
        env.variables.clone()
    }

    /// Call a global builtin by name — how a module mirrors a global
    /// (`col.take` forwarding to `take`) without re-implementing it.
    pub fn call_global_builtin(
        &mut self,
        name: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        let builtin_functions = self.builtin_functions.clone();
        BuiltinFunctions::call(&builtin_functions, name, arguments, self)
    }

    /// Feature 9: Get the intuitive error formatter
    pub fn get_error_formatter(&self) -> &IntuitiveErrorFormatter {
        &self.error_formatter
    }

    /// Feature 9: Format an interpreter error with enhanced context and suggestions
    pub fn format_error(&self, error: &InterpreterError) -> String {
        self.error_formatter.format_error(error)
    }

    /// Create a thread-safe clone for parallel operations
    pub fn thread_safe_clone(&self) -> Self {
        Self {
            stmt_span_stack: Vec::new(),
            call_stack_names: Vec::new(),
            missing_dependencies: HashMap::new(),
            pending_error_location: None,
            entry_file: None,
            pending_error_frames: None,
            pending_error_hint: None,
            scope_bindings: self.scope_bindings.clone(),
            environment: self.environment.clone(),
            builtin_functions: self.builtin_functions.clone(),
            safepoint_manager: self.safepoint_manager.clone(),
            module_debug_config: self.module_debug_config.clone(),

            // Enhanced module system
            module_cache: self.module_cache.clone(),
            module_name_index: self.module_name_index.clone(),
            dependency_tracker: self.dependency_tracker.clone(),
            current_module_path: self.current_module_path.clone(), // For tracking current module during loading
            module_loading_stack: Vec::new(), // Feature 7: Each thread gets its own loading stack

            // Feature 8: Smart caching system
            smart_cache_config: self.smart_cache_config.clone(),
            cache_statistics: self.cache_statistics.clone(),

            // Feature 9: Intuitive error messages
            error_formatter: self.error_formatter.clone(),

            // MEMORY PROTECTION: Initialize fresh recursion tracking for each thread
            call_depth: 0,
            max_call_depth: self.max_call_depth,
            step_budget: self.step_budget,
            deadline: self.deadline,

            // Each thread profiles independently; the VM is not shared, so
            // the clone gets a fresh, quiet tier with the same promotion
            // policy, and the declaration knowledge the parent accumulated
            // (functions, struct shapes, traits, variants) is replayed into
            // it — the worker never re-evaluates the declarations, so
            // without the replay nothing could promote. (This used to be
            // None — worker threads ran the pure tree-walker and silently
            // lost the tier's speed.)
            bytecode_tier: self.bytecode_tier.as_ref().map(|t| {
                let mut tier = crate::ovm::tier::BytecodeTier::new(t.threshold());
                for (name, fields) in &self.struct_defs {
                    tier.note_struct(
                        name.clone(),
                        fields.clone(),
                        self.struct_field_checks
                            .get(name)
                            .cloned()
                            .unwrap_or_default(),
                    );
                }
                for name in &self.unit_variant_names {
                    tier.note_unit_variant(name.clone());
                }
                for ((type_name, method), func) in &self.trait_impls {
                    tier.note_function(method.clone(), func.clone());
                    tier.note_trait_impl(type_name.clone(), method.clone(), func.clone());
                }
                for ((trait_name, method), func) in &self.trait_defaults {
                    tier.note_trait_default(trait_name.clone(), method.clone(), func.clone());
                }
                for (type_name, traits) in &self.type_traits {
                    for trait_name in traits {
                        tier.note_type_trait(type_name.clone(), trait_name.clone());
                    }
                }
                for (name, value) in self.environment.get_all_variables() {
                    if let Value::Function(func) = value {
                        tier.note_function(name, func);
                    }
                }
                // The gate rides the tier's bridge interpreter, so a fresh
                // worker tier without it enforces nothing — a promoted
                // function on a worker thread would reach `fs`/`net`/`db`
                // unrestricted even under an attenuating manifest. Seed it
                // here so a spawned or par-mapped dependency is judged by
                // the same grant it would be on the main thread. Same for
                // the `--trace-caps` set (a shareable Arc): a worker's
                // effects must reach the profile, or `--trace-caps --write`
                // authors a manifest that omits them and then denies them.
                tier.set_capabilities(self.caps.clone());
                if let Some(trace) = self.caps_trace.clone() {
                    tier.set_caps_trace(trace);
                }
                Box::new(tier)
            }),
            trait_impls: self.trait_impls.clone(),
            trait_defaults: self.trait_defaults.clone(),
            type_traits: self.type_traits.clone(),
            unit_variant_names: self.unit_variant_names.clone(),
            enum_type_names: self.enum_type_names.clone(),
            test_mode: false,
            test_results: Vec::new(),
            // Coverage is single-threaded: worker clones don't record.
            coverage: None,
            coverage_file_stack: Vec::new(),
            // Capabilities follow the code onto every thread — the gate
            // (above, seeded into the worker tier) and the grant table both.
            caps: self.caps.clone(),
            meta_mode: self.meta_mode,
            // The `--trace-caps` set is shared, not per-thread: a capability
            // a worker exercises is one the *program* exercised, and manifest
            // authoring (`--trace-caps --write`) must see it. The Arc<Mutex>
            // makes the shared write safe; coverage and the timeline stay
            // single-threaded because they are per-line and per-run, not
            // per-program.
            caps_trace: self.caps_trace.clone(),
            caps_path_cache: HashMap::new(),
            cap_pregranted: false,
            warm_profile: None,
            // The timeline does not span worker threads (v1 records a
            // single thread of effects); workers run live. That silently
            // breaks the "clean replay is proof" property, so crossing a
            // thread boundary under an attached timeline warns — loudly,
            // once — instead of letting a non-reproducing trace look clean.
            timeline: {
                if self.timeline.is_some() {
                    static TIMELINE_THREAD_WARNING: std::sync::Once = std::sync::Once::new();
                    TIMELINE_THREAD_WARNING.call_once(|| {
                        eprintln!(
                            "warning: this run is being recorded or replayed, but it started a \
                             task or worker thread. The timeline covers the main thread only: \
                             effects on other threads run live, are not captured, and will not \
                             replay (docs/tooling.md)."
                        );
                    });
                }
                None
            },
            struct_defs: self.struct_defs.clone(),
            struct_field_checks: self.struct_field_checks.clone(),
            dependency_map: self.dependency_map.clone(),
        }
    }

    /// Call function in a thread-safe manner (immutable)
    pub fn call_function_safe(
        &self,
        function: Value,
        args: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        // Create a local copy of interpreter state for this thread
        let mut local_interpreter = self.thread_safe_clone();
        local_interpreter.call_function(function, args)
    }

    /// Optimized function call that reuses function references where possible
    /// This reduces cloning for repeated calls with the same function
    pub fn call_function_optimized(
        &mut self,
        function: &Value,
        args: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        // Clone only when necessary
        match function {
            Value::Function(func) => self.call_user_function(func, args),
            Value::Builtin(builtin) => {
                // For builtin functions, we can optimize
                let name = builtin.name.clone();
                let builtin_functions = self.builtin_functions.clone();
                BuiltinFunctions::call(&builtin_functions, &name, args, self)
            }
            other => Err(InterpreterError::TypeError {
                message: format!(
                    "cannot call {}: it is a value, not a function",
                    Self::with_article(&other.type_name())
                ),
            }),
        }
    }

    /// MEMORY OPTIMIZED: Evaluate expression with scoped variables instead of environment replacement
    /// This completely avoids expensive environment moving operations
    /// Evaluate `expr` — the body of `me` — descending only through
    /// *tail positions*: both `if` branches, match arms, a block's final
    /// expression, and the expression of a `return`. A call to `me`
    /// itself found there evaluates its arguments and hands them back as
    /// `TailFlow::SelfCall` instead of recursing; everything else
    /// evaluates exactly as `eval_expr` would (non-tail subexpressions
    /// *are* evaluated by `eval_expr`). Self is decided by identity —
    /// the callee's body and closure Arcs are `me`'s own — so a
    /// same-named shadow or a sibling closure over the same body is a
    /// normal call, and the probe (an identifier lookup) is pure, so
    /// deciding costs no double evaluation.
    fn eval_tail_expr(&mut self, expr: &Expr, me: &Function) -> Result<TailFlow, InterpreterError> {
        match expr {
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = self.eval_expr(condition)?;
                let condition_bool = self.to_boolean(&condition)?;
                if condition_bool {
                    self.eval_tail_expr(then_branch, me)
                } else if let Some(else_expr) = else_branch {
                    self.eval_tail_expr(else_expr, me)
                } else {
                    Ok(TailFlow::Value(Value::Unit))
                }
            }
            Expr::Block(statements) => {
                let Some((last, init)) = statements.split_last() else {
                    return Ok(TailFlow::Value(Value::Unit));
                };
                // Mirror eval_expr's Block scoping exactly: a frame only
                // when something binds, popped on every path.
                let scoped = statements.iter().any(Self::statement_binds);
                if scoped {
                    self.environment = Environment::with_parent(self.environment.clone());
                }
                let mut result = Ok(TailFlow::Value(Value::Unit));
                for statement in init {
                    if let Err(e) = self.eval_statement(statement) {
                        result = Err(e);
                        break;
                    }
                }
                if result.is_ok() {
                    result = self.eval_tail_statement(last, me);
                }
                if scoped && let Some(parent) = self.environment.parent.take() {
                    self.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                }
                result
            }
            Expr::Match { value, arms } => {
                let value = self.eval_expr(value)?;
                self.eval_tail_match(value, arms, me)
            }
            Expr::Return(value) => match value {
                Some(e) => self.eval_tail_expr(e, me),
                None => Ok(TailFlow::Value(Value::Unit)),
            },
            Expr::Call { callee, arguments }
                if matches!(callee.as_ref(), Expr::Identifier(_) | Expr::LocalRef { .. }) =>
            {
                // The name being called, for the "is an Int, not a
                // function" message: without it a shadowed parameter
                // (`fn row(s, span) = span(s)`) was blamed on the
                // enclosing call, frames away from the parameter.
                let callee_name = match callee.as_ref() {
                    Expr::Identifier(n) | Expr::LocalRef { name: n, .. } => Some(n.clone()),
                    _ => None,
                };
                let callee_value = self.eval_expr(callee)?;
                if let Value::Function(target) = &callee_value
                    && Arc::ptr_eq(&target.body, &me.body)
                    && Arc::ptr_eq(&target.closure, &me.closure)
                {
                    let slots = self
                        .resolve_argument_slots(&callee_value, arguments)
                        .map_err(|e| Self::name_uncallable(e, callee_name.as_deref()))?;
                    // Only a fully applied self-call elides; a call with
                    // default holes takes the ordinary path, which fills
                    // them in the callee's scope before binding.
                    if slots.len() == me.parameters.len()
                        && slots.iter().all(|s| matches!(s, ArgSlot::Given(_)))
                    {
                        let args = slots
                            .into_iter()
                            .map(|s| match s {
                                ArgSlot::Given(v) => v,
                                ArgSlot::FromDefault => unreachable!("checked all Given"),
                            })
                            .collect();
                        return Ok(TailFlow::SelfCall(args));
                    }
                    return self
                        .call_function_slots(callee_value, slots)
                        .map(TailFlow::Value)
                        .map_err(|e| Self::name_uncallable(e, callee_name.as_deref()));
                }
                // Not a self-call: complete it here — same order as the
                // normal path (callee, then arguments, then call).
                let arg_slots = self
                    .resolve_argument_slots(&callee_value, arguments)
                    .map_err(|e| Self::name_uncallable(e, callee_name.as_deref()))?;
                self.call_function_slots(callee_value, arg_slots)
                    .map(TailFlow::Value)
                    .map_err(|e| Self::name_uncallable(e, callee_name.as_deref()))
            }
            _ => self.eval_expr(expr).map(TailFlow::Value),
        }
    }

    /// The final statement of a tail-position block. A `Located`
    /// wrapper keeps its full duties — span for error positions, the
    /// coverage tally, and the error-location capture (with the live
    /// call stack, which is where the "tail calls elided" note is
    /// visible) — exactly as `eval_statement` gives it; only a bare
    /// expression underneath is walked for tail calls.
    fn eval_tail_statement(
        &mut self,
        statement: &Statement,
        me: &Function,
    ) -> Result<TailFlow, InterpreterError> {
        match statement {
            Statement::Located { line, column, stmt } => {
                self.stmt_span_stack.push((*line, *column));
                if self.coverage.is_some() {
                    let file = self
                        .coverage_file_stack
                        .last()
                        .and_then(|o| o.clone())
                        .or_else(|| self.current_module_path.clone());
                    if let (Some(cov), Some(f)) = (self.coverage.as_mut(), file) {
                        cov.entry(f).or_default().insert(*line);
                    }
                }
                let result = self.eval_tail_statement(stmt, me);
                if let Err(e) = &result
                    && self.pending_error_location.is_none()
                    && !Self::is_control_signal(e)
                {
                    self.pending_error_location = Some(crate::ast::ErrorLocation {
                        line: *line,
                        column: *column,
                        file: self.error_file(),
                        call_stack: self
                            .pending_error_frames
                            .take()
                            .unwrap_or_else(|| self.call_stack_names.clone()),
                        hint: self.pending_error_hint.take(),
                    });
                }
                self.stmt_span_stack.pop();
                result
            }
            Statement::Expression(e) => self.eval_tail_expr(e, me),
            other => self.eval_statement(other).map(TailFlow::Value),
        }
    }

    /// `eval_match`'s twin for tail positions: identical arm selection,
    /// guard, and binding-scope handling, with the matched arm's
    /// expression walked for tail calls instead of plainly evaluated.
    fn eval_tail_match(
        &mut self,
        value: Value,
        arms: &[MatchArm],
        me: &Function,
    ) -> Result<TailFlow, InterpreterError> {
        for arm in arms {
            let mut bindings = HashMap::new();
            if self.pattern_matches_bind(&arm.pattern, &value, &mut bindings)? {
                let guard_passed = if let Some(guard_expr) = &arm.guard {
                    let parent = self.environment.clone();
                    self.environment = Environment::with_parent(parent);
                    for (k, v) in &bindings {
                        self.environment.define(k.clone(), v.clone());
                    }
                    let guard_result = self.eval_expr(guard_expr);
                    if let Some(parent) = self.environment.parent.take() {
                        self.environment =
                            Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                    }
                    self.to_boolean(&guard_result?)?
                } else {
                    true
                };

                if guard_passed {
                    let parent = self.environment.clone();
                    self.environment = Environment::with_parent(parent);
                    for (k, v) in bindings {
                        self.environment.define(k, v);
                    }
                    let result = self.eval_tail_expr(&arm.expression, me);
                    if let Some(parent) = self.environment.parent.take() {
                        self.environment =
                            Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                    }
                    return result;
                }
            }
        }
        Err(InterpreterError::PatternMatchFailed)
    }

    fn eval_match(&mut self, value: Value, arms: &[MatchArm]) -> Result<Value, InterpreterError> {
        for arm in arms {
            let mut bindings = HashMap::new();
            if self.pattern_matches_bind(&arm.pattern, &value, &mut bindings)? {
                // Pattern matched, now check guard clause if present
                let guard_passed = if let Some(guard_expr) = &arm.guard {
                    // Create scope with pattern bindings for guard evaluation
                    let parent = self.environment.clone();
                    self.environment = Environment::with_parent(parent);
                    for (k, v) in &bindings {
                        self.environment.define(k.clone(), v.clone());
                    }

                    let guard_result = self.eval_expr(guard_expr);

                    // Restore parent environment
                    if let Some(parent) = self.environment.parent.take() {
                        self.environment =
                            Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                    }

                    // Surface guard errors instead of silently treating them
                    // as "no match" (which hid typos like undefined variables)
                    self.to_boolean(&guard_result?)?
                } else {
                    true // No guard clause, pattern match is sufficient
                };

                if guard_passed {
                    // Execute the match arm expression with pattern bindings
                    let parent = self.environment.clone();
                    self.environment = Environment::with_parent(parent);
                    for (k, v) in bindings {
                        self.environment.define(k, v);
                    }
                    let result = self.eval_expr(&arm.expression);
                    if let Some(parent) = self.environment.parent.take() {
                        self.environment =
                            Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                    }
                    return result;
                }
                // Pattern matched but guard failed, continue to next arm
            }
        }
        Err(InterpreterError::PatternMatchFailed)
    }

    fn eval_type_decl(
        &mut self,
        type_decl: crate::ast::TypeDecl,
    ) -> Result<Value, InterpreterError> {
        use crate::ast::{EnumVariantData, TypeDefinition};

        // Enum declarations bind each variant into scope so it can be
        // constructed. Unit variants become `Enum` values directly; tuple
        // variants become constructor callables (`Circle(radius)`).
        //
        // Type parameters (`enum Option<T>`) are erased at runtime — the
        // language is dynamically typed, so a generic variant constructs for
        // any argument type. The static side is the type checker's concern.
        if let TypeDefinition::Enum { variants } = &type_decl.definition {
            // Remember the enum's type name so an annotation naming it is
            // recognized as a real type (and a name that is *not* declared
            // is reported as unknown rather than as a value mismatch). The
            // tier keeps its own mirror, for its own enforcement sites.
            self.enum_type_names.insert(type_decl.name.clone());
            if let Some(tier) = self.bytecode_tier.as_mut() {
                tier.note_enum_type(type_decl.name.clone());
            }
            for variant in variants {
                let value = match &variant.data {
                    None => {
                        // Remember unit-variant names so a pattern can tell a
                        // variant test from a fresh binding by name.
                        self.unit_variant_names.insert(variant.name.clone());
                        if let Some(tier) = self.bytecode_tier.as_mut() {
                            tier.note_unit_variant(variant.name.clone());
                        }
                        Value::Enum {
                            type_name: type_decl.name.clone(),
                            variant_name: variant.name.clone(),
                            variant_data: EnumVariantData::Unit,
                        }
                    }
                    Some(fields) => Value::EnumConstructor {
                        type_name: type_decl.name.clone(),
                        variant_name: variant.name.clone(),
                        arity: fields.len(),
                    },
                };
                self.environment.define(variant.name.clone(), value);
            }
        }

        // Struct declarations register their shape (the field-name set) and
        // the field types the runtime can enforce. Construction validates the
        // field set and rejects a field value whose runtime type does not
        // match its declared annotation.
        if let TypeDefinition::Struct { fields } = &type_decl.definition {
            let field_names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
            let field_checks: HashMap<String, crate::ast::FieldTypeCheck> = fields
                .iter()
                .filter_map(|f| {
                    crate::ast::FieldTypeCheck::from_annotation(
                        &f.field_type,
                        &type_decl.type_params,
                    )
                    .map(|check| (f.name.clone(), check))
                })
                .collect();
            // The bytecode tier validates literals against the same shape and
            // enforces the same field types.
            if let Some(tier) = self.bytecode_tier.as_mut() {
                tier.note_struct(
                    type_decl.name.clone(),
                    field_names.clone(),
                    field_checks.clone(),
                );
            }
            self.struct_defs.insert(type_decl.name.clone(), field_names);
            self.struct_field_checks
                .insert(type_decl.name.clone(), field_checks);
        }

        Ok(Value::Unit)
    }

    fn eval_struct_literal(
        &mut self,
        struct_literal: &crate::ast::StructLiteral,
    ) -> Result<Value, InterpreterError> {
        // A struct literal names a declared type, and construction validates
        // the field-name set against the declaration — missing or surprise
        // fields are errors, and an undeclared name is an error (use an
        // anonymous `{ ... }` object for free-form records). Field VALUES are
        // also checked against their declared annotations where the runtime
        // can: a value whose runtime type does not match a field's declared
        // Int/Float/Bool/String or struct/enum type is a type error. Fields
        // with an unenforceable annotation (a generic parameter, a list, a
        // function, ...) stay dynamic.
        let declared = match self.struct_defs.get(&struct_literal.type_name) {
            Some(fields) => fields.clone(),
            None => {
                return Err(InterpreterError::TypeError {
                    message: format!(
                        "unknown struct type '{}' — declare it with `type {} = struct {{ ... }}`, \
                         or use an anonymous object `{{ ... }}` for a free-form record",
                        struct_literal.type_name, struct_literal.type_name
                    ),
                });
            }
        };

        let given: Vec<&String> = struct_literal.fields.iter().map(|f| &f.name).collect();
        for required in &declared {
            if !given.contains(&required) {
                return Err(InterpreterError::TypeError {
                    message: format!(
                        "struct '{}' is missing field '{}' (declared fields: {})",
                        struct_literal.type_name,
                        required,
                        declared.join(", ")
                    ),
                });
            }
        }
        for name in &given {
            if !declared.contains(name) {
                return Err(InterpreterError::TypeError {
                    message: format!(
                        "struct '{}' has no field '{}' (declared fields: {})",
                        struct_literal.type_name,
                        name,
                        declared.join(", ")
                    ),
                });
            }
        }

        let mut fields = std::collections::HashMap::new();
        for field_value in &struct_literal.fields {
            let value = self.eval_expr(&field_value.value)?;
            // Enforce the declared field type where the runtime can check it.
            // A field with an unenforceable annotation (generic parameter,
            // list, function, ...) has no entry and stays dynamic.
            if let Some(check) = self
                .struct_field_checks
                .get(&struct_literal.type_name)
                .and_then(|c| c.get(&field_value.name))
            {
                let (actual, payload, fn_arity) = Self::value_view(&value);
                if let Some((expected, got)) = check.check_value(
                    &actual,
                    payload.as_ref().map(|(o, p)| (*o, p.as_str())),
                    fn_arity,
                    Self::scalar_view(&value),
                ) {
                    return Err(InterpreterError::TypeError {
                        message: self.annotation_error(
                            &format!(
                                "field '{}' of {}",
                                field_value.name, struct_literal.type_name
                            ),
                            check,
                            &expected,
                            &got,
                        ),
                    });
                }
            }
            fields.insert(field_value.name.clone(), value);
        }

        Ok(Value::Struct {
            type_name: struct_literal.type_name.clone(),
            fields: std::sync::Arc::new(fields),
        })
    }

    fn eval_anonymous_object(
        &mut self,
        field_values: &[crate::ast::FieldValue],
    ) -> Result<Value, InterpreterError> {
        let mut fields = std::collections::HashMap::new();

        for field_value in field_values {
            let value = self.eval_expr(&field_value.value)?;
            fields.insert(field_value.name.clone(), value);
        }

        // Use a generic type name for anonymous objects
        Ok(Value::Struct {
            type_name: "Object".to_string(),
            fields: std::sync::Arc::new(fields),
        })
    }

    fn eval_map_literal(
        &mut self,
        entries: &[crate::ast::MapEntry],
    ) -> Result<Value, InterpreterError> {
        let mut map = std::collections::HashMap::new();

        for entry in entries {
            let key = self.eval_expr(&entry.key)?;
            let value = self.eval_expr(&entry.value)?;

            // Convert key to string (maps in Olang use string keys)
            let key_str = match key {
                Value::String(s) => s.as_ref().clone(),
                Value::Integer(i) => i.to_string(),
                Value::Float(f) => crate::ast::format_float(f),
                Value::Boolean(b) => b.to_string(),
                _ => {
                    return Err(InterpreterError::TypeError {
                        message: "Map keys must be strings, integers, floats, or booleans"
                            .to_string(),
                    });
                }
            };

            map.insert(key_str, value);
        }

        Ok(Value::Map(std::sync::Arc::new(map)))
    }

    /// The message for accessing a name that a struct does not have as a
    /// field. If the name is a *method* — one some trait declares (seen as a
    /// default body, or as an `impl` on another type) — then the fix is an
    /// `impl`, not a field, so it says so instead of reporting a missing
    /// field. `value.method()` reaches this path when method resolution
    /// found no `impl`, so this is exactly where "no field 'a'" misled.
    fn no_field_or_method(&self, type_name: &str, field: &str) -> String {
        let declaring_trait = self
            .trait_defaults
            .keys()
            .find(|(_, method)| method == field)
            .map(|(t, _)| t.as_str());
        let impld_elsewhere = self.trait_impls.keys().any(|(_, method)| method == field);
        match (declaring_trait, impld_elsewhere) {
            (Some(t), _) => format!(
                "no method '{field}' for {type_name}: the trait {t} declares it, but there is \
                 no `impl {t} for {type_name}`"
            ),
            (None, true) => format!(
                "no method '{field}' for {type_name}: it is a trait method implemented for other \
                 types but not this one — add an `impl ... for {type_name}`"
            ),
            (None, false) => format!("{type_name} has no field or method '{field}'"),
        }
    }

    fn eval_field_access(
        &mut self,
        object: &crate::ast::Expr,
        field: &str,
    ) -> Result<Value, InterpreterError> {
        let object_value = self.eval_expr(object)?;

        match object_value {
            Value::Struct { fields, type_name } => {
                if type_name == "Module" {
                    // Handle module function access (e.g., fs.read_file).
                    // A miss names the nearest member — `heap` for
                    // `headp` — because at a module boundary the mistake
                    // is almost always a spelling, and the module knows
                    // its own names.
                    fields
                        .get(field)
                        .cloned()
                        .ok_or_else(|| InterpreterError::TypeError {
                            message: module_member_miss(field, fields.keys().map(|k| k.as_str())),
                        })
                } else {
                    // Handle regular struct field access
                    fields
                        .get(field)
                        .cloned()
                        .ok_or_else(|| InterpreterError::TypeError {
                            message: self.no_field_or_method(&type_name, field),
                        })
                }
            }
            _ => Err(InterpreterError::TypeError {
                message: format!("Cannot access field '{}' on non-struct value", field),
            }),
        }
    }

    /// Get a reference to the current environment for REPL inspection
    pub fn get_environment(&self) -> &Environment {
        &self.environment
    }

    /// Get all user-defined variables (excluding built-ins)
    /// If `name` is bound to a module (native stdlib, embedded, or a package),
    /// return its member function names, sorted. Used by `:help <module>` so
    /// imported modules are discoverable. Returns None for non-module bindings.
    pub fn module_members(&self, name: &str) -> Option<Vec<String>> {
        match self.environment.get(name)? {
            Value::Struct { type_name, fields } if type_name == "Module" => {
                let mut names: Vec<String> = fields.keys().cloned().collect();
                names.sort();
                Some(names)
            }
            _ => None,
        }
    }

    pub fn get_user_variables(&self) -> HashMap<String, &Value> {
        let mut user_vars = HashMap::new();
        let builtin_names: std::collections::HashSet<String> = self
            .builtin_functions
            .get_functions()
            .keys()
            .cloned()
            .collect();

        // Dereference Arc to iterate over ImHashMap
        for (name, value) in self.environment.variables.iter() {
            if !builtin_names.contains(name) {
                user_vars.insert(name.clone(), value);
            }
        }
        user_vars
    }

    /// Get all built-in functions
    pub fn get_builtin_functions(&self) -> &HashMap<String, BuiltinFunction> {
        self.builtin_functions.get_functions()
    }

    /// Clear user-defined variables (keep built-ins and stdlib modules)
    pub fn clear_user_environment(&mut self) {
        let builtin_names: std::collections::HashSet<String> = self
            .builtin_functions
            .get_functions()
            .keys()
            .cloned()
            .collect();

        // Identify stdlib modules before the retain operation
        let stdlib_names: std::collections::HashSet<String> = self
            .environment
            .variables
            .iter()
            .filter_map(|(name, value)| {
                if Self::is_stdlib_module_static(name, value) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();

        // Use Arc::make_mut for copy-on-write mutation of the ImHashMap
        let vars = Arc::make_mut(&mut self.environment.variables);
        vars.retain(|name, _| {
            // Keep builtin functions
            builtin_names.contains(name) ||
            // Keep stdlib modules
            stdlib_names.contains(name)
        });
    }

    /// Check if a variable is a module binding worth keeping across a
    /// `:run` cleanup. Any Module-typed value qualifies: the old
    /// hardcoded name list silently dropped `str` and `time` (among
    /// others), so the script after the first one found its ambient
    /// modules wiped while the load tracking still called them loaded —
    /// "Undefined variable: str" on the second `:run`. Modules are
    /// namespaces of functions, not state; retaining them all is the
    /// same persistence `use` already has at the prompt.
    fn is_stdlib_module_static(_name: &str, value: &Value) -> bool {
        matches!(value, Value::Struct { type_name, .. } if type_name == "Module")
    }

    /// Define a variable in the current environment (for REPL use)
    pub fn define_variable(&mut self, name: String, value: Value) {
        self.environment.define(name, value);
    }

    /// Perform safepoint poll for GC coordination
    /// This should be called periodically during evaluation
    pub fn safepoint_poll(&self) -> Result<(), InterpreterError> {
        self.safepoint_manager
            .safepoint_poll()
            .map_err(|e| InterpreterError::RuntimeError {
                message: format!("Safepoint coordination failed: {}", e),
            })
    }

    /// Register this thread with the safepoint manager
    pub fn register_thread(&self) {
        self.safepoint_manager.register_thread();
    }

    /// Unregister this thread from the safepoint manager
    pub fn unregister_thread(&self) {
        self.safepoint_manager.unregister_thread();
    }

    /// Get the safepoint manager for external coordination
    pub fn get_safepoint_manager(&self) -> Arc<SafepointManager> {
        Arc::clone(&self.safepoint_manager)
    }

    /// Collect all accessible variables from the current environment and its parent chain
    fn collect_all_accessible_variables(&self) -> ImHashMap<String, Value> {
        // Union the chain innermost-first: im's union prefers entries from
        // self on collision, so inner scopes shadow outer ones. Structural
        // sharing makes this near-O(1) for the common shallow chains,
        // versus copying every entry of every scope.
        // NOTE: im::HashMap::union is unusable here — its collision bias
        // depends on which map is LARGER (it swaps sides internally as a
        // size optimization), so "inner scope wins" silently became
        // "bigger scope wins" and a captured variable could resolve to an
        // ancestor frame's stale value. Insert explicitly instead: existing
        // entries always win, so inner scopes shadow outer ones.
        let mut all_variables = self.environment.flat_snapshot();
        let mut current_env = &self.environment;
        while let Some(parent) = current_env.parent.as_ref() {
            // Within a scope, call-frame locals shadow its flat map
            for (name, value) in parent.locals.iter().rev() {
                if !all_variables.contains_key(name) {
                    all_variables.insert(name.clone(), value.clone());
                }
            }
            for (name, value) in parent.variables.iter() {
                if !all_variables.contains_key(name) {
                    all_variables.insert(name.clone(), value.clone());
                }
            }
            current_env = parent;
        }

        all_variables
    }

    fn eval_for_loop(
        &mut self,
        variable: &str,
        iterable: &Expr,
        body: &Expr,
    ) -> Result<Value, InterpreterError> {
        let iterable_value = self.eval_expr(iterable)?;

        match iterable_value {
            Value::List(items) => {
                let parent_env = std::mem::take(&mut self.environment);
                self.environment.parent = Some(Arc::new(parent_env));
                self.environment.is_frame = true;

                let result = self.run_loop_body(body, items.iter().cloned(), Some(variable));

                // Restore parent environment (also on error, so a failing body
                // doesn't leak the loop scope into subsequent statements)
                if let Some(parent) = self.environment.parent.take() {
                    self.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                }

                result
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                let parent_env = std::mem::take(&mut self.environment);
                self.environment.parent = Some(Arc::new(parent_env));
                self.environment.is_frame = true;

                let result = self.run_range_loop(variable, start, end, inclusive, body);

                // Restore parent environment
                if let Some(parent) = self.environment.parent.take() {
                    self.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                }

                result
            }
            // A tuple iterates over its elements. It is a fixed-shape
            // group rather than a collection, so this is mostly for
            // `for (k, v) in entries(m)`-shaped code and for symmetry —
            // refusing it was a surprise with no reason behind it.
            Value::Tuple(items) => {
                let parent_env = std::mem::take(&mut self.environment);
                self.environment.parent = Some(Arc::new(parent_env));
                self.environment.is_frame = true;

                let values: Vec<Value> = items.as_ref().clone();
                let result = self.run_loop_body(body, values.into_iter(), Some(variable));

                if let Some(parent) = self.environment.parent.take() {
                    self.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                }

                result
            }
            // Strings iterate by character, each a 1-character string —
            // consistent with 'a' literals being strings.
            Value::String(s) => {
                let parent_env = std::mem::take(&mut self.environment);
                self.environment.parent = Some(Arc::new(parent_env));
                self.environment.is_frame = true;

                let chars: Vec<Value> = s
                    .chars()
                    .map(|c| Value::String(Arc::new(c.to_string())))
                    .collect();
                let result = self.run_loop_body(body, chars.into_iter(), Some(variable));

                if let Some(parent) = self.environment.parent.take() {
                    self.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                }

                result
            }
            _ => {
                // Never dump the value's Debug representation — it leaks the
                // internal `Map({"a": Integer(1)})` shape. Name the type, and
                // point a map at the pairs form, which is the usual intent.
                let hint = match &iterable_value {
                    Value::Map(_) | Value::Struct { .. } => {
                        " — iterate its pairs with `for (k, v) in entries(m)`"
                    }
                    _ => "",
                };
                Err(InterpreterError::TypeError {
                    message: format!(
                        "cannot iterate over a {}{}",
                        iterable_value.type_name(),
                        hint
                    ),
                })
            }
        }
    }

    /// How many interpreted iterations a top-level loop runs before the
    /// remainder is offered to the tier. High enough that short loops
    /// never pay a compile, low enough that a million-iteration loop
    /// spends its life natively.
    const LOOP_PROMOTE_AFTER: i64 = 512;

    /// A `for` over a range, with hot-loop promotion: after
    /// LOOP_PROMOTE_AFTER interpreted iterations, the remaining range is
    /// synthesized into a continuation function and run on the tier in
    /// one boundary crossing (see interpreter/loop_promo.rs). Fallback
    /// at any point — analysis refusal, unconverted value, compile
    /// rejection — resumes right here as if nothing happened.
    fn run_range_loop(
        &mut self,
        variable: &str,
        start: i64,
        end: i64,
        inclusive: bool,
        body: &Expr,
    ) -> Result<Value, InterpreterError> {
        let mut last_value = Value::Unit;
        let mut i = start;
        let mut iters: i64 = 0;
        let mut try_promotion = self.bytecode_tier.is_some() && !self.budgeted();
        loop {
            let done = if inclusive { i > end } else { i >= end };
            if done {
                break;
            }
            if iters == Self::LOOP_PROMOTE_AFTER && try_promotion {
                try_promotion = false;
                if let Some(v) =
                    self.promote_loop_remainder(Some((variable, i, end, inclusive)), None, body)?
                {
                    return Ok(v);
                }
            }
            self.safepoint_poll()?;
            self.environment
                .define(variable.to_string(), Value::Integer(i));
            match self.eval_expr(body) {
                Ok(_) => {}
                Err(InterpreterError::BreakSignal(v)) => {
                    last_value = v;
                    break;
                }
                Err(InterpreterError::ContinueSignal) => {}
                Err(e) => return Err(e),
            }
            // Terminate before incrementing: `i + 1` at `end == i64::MAX`
            // would overflow (the guarantee the old RangeIter carried).
            if i == end {
                break;
            }
            i += 1;
            iters += 1;
        }
        Ok(last_value)
    }

    /// Synthesize and run the remainder of a hot loop on the tier.
    /// `range` carries `for`-loop state (variable, next value, end,
    /// inclusive); `while_parts` carries a `while` loop's condition.
    /// `Ok(Some(v))` means the loop ran to its end natively and `v` is
    /// its value (live-outs already written back); `Ok(None)` means the
    /// tier declined before running anything — keep interpreting.
    fn promote_loop_remainder(
        &mut self,
        range: Option<(&str, i64, i64, bool)>,
        while_condition: Option<&Expr>,
        body: &Expr,
    ) -> Result<Option<Value>, InterpreterError> {
        if self.bytecode_tier.is_none() {
            return Ok(None);
        }
        let loop_var = range.map(|(v, _, _, _)| v);
        let Some(facts) = loop_promo::analyze(body, loop_var) else {
            return Ok(None);
        };
        // For a while loop the condition's free names are live too.
        let facts = match while_condition {
            None => facts,
            Some(cond) => {
                let Some(cond_facts) = loop_promo::analyze(cond, loop_var) else {
                    return Ok(None);
                };
                let mut reads = facts.reads;
                for r in cond_facts.reads {
                    if !reads.contains(&r) {
                        reads.push(r);
                    }
                }
                if !cond_facts.writes.is_empty() {
                    // A condition that assigns is exotic; keep it
                    // interpreted rather than reason about it.
                    return Ok(None);
                }
                loop_promo::BodyFacts {
                    reads,
                    writes: facts.writes,
                }
            }
        };

        // Live-ins: free reads that are VARIABLES here and now. Free
        // names holding functions are left free — the tier resolves
        // known functions by name, which is what lets it inline them;
        // a name it cannot resolve fails the compile and we fall back.
        let mut live_names: Vec<String> = Vec::new();
        let mut live_values: Vec<Value> = Vec::new();
        for name in &facts.reads {
            match self.environment.get(name) {
                Some(Value::Function(_)) | Some(Value::Builtin(_)) | None => {}
                Some(v) => {
                    live_names.push(name.clone());
                    live_values.push(v);
                }
            }
        }
        // Every written name must be a live-in variable we can hand
        // back; a write target that didn't resolve stays interpreted.
        for w in &facts.writes {
            if !live_names.contains(w) {
                return Ok(None);
            }
        }

        let func = match (range, while_condition) {
            (Some((variable, _, _, inclusive)), None) => loop_promo::synthesize_range_continuation(
                variable,
                inclusive,
                body,
                &live_names,
                &facts.writes,
                self.current_module_path.clone(),
            ),
            (None, Some(cond)) => loop_promo::synthesize_while_continuation(
                cond,
                body,
                &live_names,
                &facts.writes,
                self.current_module_path.clone(),
            ),
            _ => return Ok(None),
        };

        let mut args: Vec<Value> = Vec::with_capacity(2 + live_values.len());
        if let Some((_, next, end, _)) = range {
            args.push(Value::Integer(next));
            args.push(Value::Integer(end));
        }
        args.extend(live_values);

        let globals = self.global_bindings();
        let mut tier = self.bytecode_tier.take();
        let outcome = tier
            .as_mut()
            .map(|t| {
                t.set_host_globals(globals);
                t.try_call_function_value(&func, &mut args)
            })
            .unwrap_or(crate::ovm::tier::TierOutcome::Fallback);
        self.bytecode_tier = tier;

        match outcome {
            crate::ovm::tier::TierOutcome::Ran(Ok(result)) => {
                let (loop_value, outs) = if facts.writes.is_empty() {
                    (result, Vec::new())
                } else {
                    match result {
                        Value::Tuple(items) => {
                            let mut items = items.as_ref().clone();
                            let rest = items.split_off(1);
                            (items.pop().unwrap_or(Value::Unit), rest)
                        }
                        // The synthesized shape IS a tuple when writes
                        // exist; anything else means the tier diverged —
                        // refuse the result rather than corrupt state.
                        _ => return Ok(None),
                    }
                };
                for (name, value) in facts.writes.iter().zip(outs) {
                    self.environment.set(name, value)?;
                }
                Ok(Some(loop_value))
            }
            crate::ovm::tier::TierOutcome::Ran(Err(message)) => {
                let err = Self::map_tier_error_message(message);
                // Splice the VM's error location onto the interpreter's
                // stack exactly as the ordinary tiered-call path does —
                // minus the synthetic "<hot loop>" frame, which no
                // interpreter-only run would show.
                if self.pending_error_location.is_none() && !Self::is_control_signal(&err) {
                    let (span, frames, _leak) = self
                        .bytecode_tier
                        .as_mut()
                        .map(|t| t.take_error_trace())
                        .unwrap_or_default();
                    let frames: Vec<String> =
                        frames.into_iter().filter(|f| f != "<hot loop>").collect();
                    if let Some((line, column)) = span {
                        let trace_file = self
                            .bytecode_tier
                            .as_mut()
                            .and_then(|t| t.take_error_trace_file())
                            .filter(|f| {
                                !f.starts_with("__")
                                    && self.entry_file.as_deref() != Some(f.as_str())
                            });
                        self.pending_error_location = Some(crate::ast::ErrorLocation {
                            line,
                            column,
                            file: trace_file.or_else(|| self.error_file()),
                            call_stack: self.splice_tier_stack(frames),
                            hint: self.pending_error_hint.take(),
                        });
                    } else if !frames.is_empty() && self.pending_error_frames.is_none() {
                        self.pending_error_frames = Some(self.splice_tier_stack(frames));
                    }
                }
                Err(err)
            }
            crate::ovm::tier::TierOutcome::Fallback => Ok(None),
        }
    }

    /// Run a loop body over an iterator of items, honoring break/continue.
    /// `par for`: fan the iterations across worker threads. Spawn-style
    /// snapshot semantics — each worker runs against a clone of the
    /// interpreter, so mutations to enclosing state are not visible to
    /// the caller; effects (println, fs) are real. Implicit barrier at
    /// the end; the loop evaluates to Unit. If several iterations fail,
    /// the error reported is the one the sequential loop would have hit
    /// first. `break` and `return` cannot cross the parallel boundary
    /// and are errors; `continue` works within an iteration.
    fn eval_par_for_loop(
        &mut self,
        variable: &str,
        iterable: &Expr,
        body: &Expr,
    ) -> Result<Value, InterpreterError> {
        // Same gate as `spawn` and `par_map`: worker interleaving is
        // scheduling nondeterminism, which expansion must not observe.
        // (This arm was missing while the gate was a name denylist —
        // `par for` is a syntax form, not a builtin name.)
        if self.meta_mode {
            return Err(InterpreterError::RuntimeError {
                message: "par for is not available at expansion time: a meta fn is a \
                          pure function of its arguments (docs/macros.md)"
                    .to_string(),
            });
        }
        let iterable_value = self.eval_expr(iterable)?;
        let items: Vec<Value> = match iterable_value {
            Value::List(items) => items.to_vec(),
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                let stop = if inclusive {
                    end.saturating_add(1)
                } else {
                    end
                };
                (start..stop).map(Value::Integer).collect()
            }
            Value::String(s) => s
                .chars()
                .map(|c| Value::String(Arc::new(c.to_string())))
                .collect(),
            other => {
                return Err(InterpreterError::TypeError {
                    message: format!(
                        "par for: cannot iterate over {} (lists, ranges, and strings)",
                        other.type_name()
                    ),
                });
            }
        };
        if items.is_empty() {
            return Ok(Value::Unit);
        }

        let run_one = |worker: &mut Interpreter, item: Value| -> Result<(), InterpreterError> {
            let parent_env = std::mem::take(&mut worker.environment);
            worker.environment.parent = Some(Arc::new(parent_env));
            worker.environment.is_frame = true;
            worker.environment.define(variable.to_string(), item);
            let result = match worker.eval_expr(body) {
                Ok(_) | Err(InterpreterError::ContinueSignal) => Ok(()),
                Err(InterpreterError::BreakSignal(_)) => Err(InterpreterError::RuntimeError {
                    message: "break cannot cross a par for boundary".to_string(),
                }),
                Err(InterpreterError::ReturnSignal(_)) => Err(InterpreterError::RuntimeError {
                    message: "return cannot cross a par for boundary".to_string(),
                }),
                Err(e) => Err(e),
            };
            if let Some(parent) = worker.environment.parent.take() {
                worker.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
            }
            result
        };

        #[cfg(feature = "native")]
        {
            let workers = crate::parallel::get_config()
                .max_threads
                .clamp(1, items.len());
            if workers > 1 {
                let chunk_size = items.len().div_ceil(workers);
                let joined: Vec<Result<(), (usize, InterpreterError)>> =
                    std::thread::scope(|scope| {
                        let handles: Vec<_> = items
                            .chunks(chunk_size)
                            .enumerate()
                            .map(|(chunk_idx, chunk)| {
                                let mut worker = self.thread_safe_clone();
                                scope.spawn(move || {
                                    let _live = crate::stdlib::chan::live_guard();
                                    for (i, item) in chunk.iter().enumerate() {
                                        run_one(&mut worker, item.clone())
                                            .map_err(|e| (chunk_idx * chunk_size + i, e))?;
                                    }
                                    Ok(())
                                })
                            })
                            .collect();
                        handles
                            .into_iter()
                            .map(|h| {
                                h.join().unwrap_or_else(|_| {
                                    Err((
                                        usize::MAX,
                                        InterpreterError::RuntimeError {
                                            message: "par for worker thread panicked".to_string(),
                                        },
                                    ))
                                })
                            })
                            .collect()
                    });
                let mut first_err: Option<(usize, InterpreterError)> = None;
                for r in joined {
                    if let Err((idx, e)) = r
                        && first_err.as_ref().map(|(fi, _)| idx < *fi).unwrap_or(true)
                    {
                        first_err = Some((idx, e));
                    }
                }
                if let Some((_, e)) = first_err {
                    return Err(e);
                }
                return Ok(Value::Unit);
            }
        }

        // Single worker (or the playground): sequential, same semantics.
        for item in items {
            run_one(self, item)?;
        }
        Ok(Value::Unit)
    }

    fn run_loop_body(
        &mut self,
        body: &Expr,
        items: impl Iterator<Item = Value>,
        variable: Option<&str>,
    ) -> Result<Value, InterpreterError> {
        // Loops evaluate to Unit unless `break value` exits them — matching
        // the bytecode tier ("For loops evaluate to Unit"). Discarding each
        // body value also matters for performance: retaining it aliased the
        // body's result across iterations, which defeated the sole-owner
        // append fusion whenever `xs = xs + [..]` was the body's last
        // statement (the accumulation loop, exactly).
        let mut last_value = Value::Unit;
        for item in items {
            // Safepoint poll for GC coordination during iteration
            self.safepoint_poll()?;
            self.spend_step()?;

            if let Some(name) = variable {
                self.environment.define(name.to_string(), item);
            }
            match self.eval_expr(body) {
                Ok(_) => {}
                Err(InterpreterError::BreakSignal(v)) => {
                    last_value = v;
                    break;
                }
                Err(InterpreterError::ContinueSignal) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(last_value)
    }

    fn eval_while_loop(
        &mut self,
        condition: &Expr,
        body: &Expr,
    ) -> Result<Value, InterpreterError> {
        // Unit unless `break value` — see run_loop_body for why body values
        // are discarded rather than retained.
        let mut last_value = Value::Unit;
        let mut iters: i64 = 0;
        let mut try_promotion = self.bytecode_tier.is_some() && !self.budgeted();

        loop {
            // Safepoint poll for GC coordination at start of each iteration
            self.safepoint_poll()?;
            self.spend_step()?;

            if iters == Self::LOOP_PROMOTE_AFTER && try_promotion {
                try_promotion = false;
                if let Some(v) = self.promote_loop_remainder(None, Some(condition), body)? {
                    return Ok(v);
                }
            }
            iters += 1;

            let condition_value = self.eval_expr(condition)?;
            let condition_bool = self.to_boolean(&condition_value)?;

            if !condition_bool {
                break;
            }

            match self.eval_expr(body) {
                Ok(_) => {}
                Err(InterpreterError::BreakSignal(v)) => {
                    last_value = v;
                    break;
                }
                Err(InterpreterError::ContinueSignal) => continue,
                Err(e) => return Err(e),
            }
        }

        Ok(last_value)
    }

    fn eval_loop(&mut self, body: &Expr) -> Result<Value, InterpreterError> {
        // Unit unless `break value` — see run_loop_body for why body values
        // are discarded rather than retained.
        loop {
            // Safepoint poll for GC coordination at start of each iteration
            self.safepoint_poll()?;
            self.spend_step()?;

            match self.eval_expr(body) {
                Ok(_) => {}
                Err(InterpreterError::BreakSignal(v)) => return Ok(v),
                Err(InterpreterError::ContinueSignal) => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Resolve arguments (both positional and named) for function calls.
    /// Produces one slot per parameter position: `Given` for provided
    /// values (named arguments moved to their positions), `FromDefault`
    /// for holes a parameter default will fill — evaluation of those
    /// defaults happens later, in the callee's scope, never here in the
    /// caller's.
    fn resolve_argument_slots(
        &mut self,
        callee: &Value,
        arguments: &[Argument],
    ) -> Result<Vec<ArgSlot>, InterpreterError> {
        // Get function parameter information if available
        let parameters = match callee {
            Value::Function(func) => Some(&func.parameters),
            _ => None, // For builtin functions and other callables, use positional-only
        };

        let mut resolved_args: Vec<ArgSlot> = Vec::new();
        // Vec keeps source order — a HashMap would append named args to
        // builtins in nondeterministic order
        let mut named_args: Vec<(String, Value)> = Vec::new();
        let mut positional_count = 0;

        // First pass: collect positional and named arguments
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    if !named_args.is_empty() {
                        return Err(InterpreterError::RuntimeError {
                            message: "Positional arguments cannot come after named arguments"
                                .to_string(),
                        });
                    }
                    resolved_args.push(ArgSlot::Given(self.eval_expr(expr)?));
                    positional_count += 1;
                }
                Argument::Named { name, value } => {
                    let evaluated_value = self.eval_expr(value)?;
                    if named_args.iter().any(|(n, _)| n == name) {
                        return Err(InterpreterError::RuntimeError {
                            message: format!("Duplicate named argument: {}", name),
                        });
                    }
                    named_args.push((name.clone(), evaluated_value));
                }
            }
        }

        // Second pass: resolve named arguments to correct positions (if we have parameter info)
        if let Some(params) = parameters {
            // Check for conflicts between positional and named arguments
            for (arg_name, _) in &named_args {
                if let Some(param_index) = params.iter().position(|p| &p.name == arg_name)
                    && param_index < positional_count
                {
                    return Err(InterpreterError::RuntimeError {
                        message: format!(
                            "Argument '{}' specified both positionally and by name",
                            arg_name
                        ),
                    });
                }
            }

            // Extend resolved_args to cover all parameters, filling with named args or defaults
            while resolved_args.len() < params.len() {
                let param_index = resolved_args.len();
                let param = &params[param_index];

                if let Some(pos) = named_args.iter().position(|(n, _)| n == &param.name) {
                    // Use named argument value
                    resolved_args.push(ArgSlot::Given(named_args.remove(pos).1));
                } else if param.default_value.is_some() {
                    // A hole for the default — evaluated at the call
                    // boundary in the callee's scope, not here.
                    resolved_args.push(ArgSlot::FromDefault);
                } else {
                    // Missing required argument
                    return Err(InterpreterError::RuntimeError {
                        message: format!("Missing required argument: {}", param.name),
                    });
                }
            }

            // Check for unrecognized named arguments
            if !named_args.is_empty() {
                let unrecognized: Vec<String> = named_args.iter().map(|(n, _)| n.clone()).collect();
                return Err(InterpreterError::RuntimeError {
                    message: format!(
                        "Unrecognized named argument(s): {}",
                        unrecognized.join(", ")
                    ),
                });
            }
        } else {
            // For builtin functions, just append named arguments as positional
            for (_, value) in named_args {
                resolved_args.push(ArgSlot::Given(value));
            }
        }

        Ok(resolved_args)
    }

    /// Dispatch a slot vector: user functions fill their default holes
    /// in their own scope; every other callee takes plain values (its
    /// slots are always `Given` — `FromDefault` is only produced when
    /// the callee is a `Function` with parameter information).
    fn call_function_slots(
        &mut self,
        callee: Value,
        slots: Vec<ArgSlot>,
    ) -> Result<Value, InterpreterError> {
        if let Value::Function(func) = &callee {
            let func = func.clone();
            return self.call_user_function_slots(&func, slots);
        }
        let args = slots
            .into_iter()
            .map(|s| match s {
                ArgSlot::Given(v) => v,
                ArgSlot::FromDefault => Value::Unit,
            })
            .collect();
        self.call_function(callee, args)
    }

    fn eval_share_decl(&mut self, share: ShareDecl) -> Result<Value, InterpreterError> {
        match share {
            ShareDecl::Function(func) => self.eval_function_decl(func),
            ShareDecl::Let(letd) => self.eval_let_decl(&letd),
            ShareDecl::Type(typed) => self.eval_type_decl(typed),
            ShareDecl::Use(use_decl) => self.eval_transitive_share(use_decl),
            // Traits and impls register in global registries, not the
            // environment, so `share` is cosmetic — declaring them already
            // makes them available to any module that loads this one.
            ShareDecl::Trait(trait_decl) => self.eval_trait_decl(trait_decl),
            ShareDecl::Impl(impl_decl) => self.eval_impl_decl(impl_decl),
        }
    }

    /// Handle transitive sharing: share use module { items }
    fn eval_transitive_share(&mut self, use_decl: UseDecl) -> Result<Value, InterpreterError> {
        // Load the module and import the specified items
        let module_path = use_decl.path.join(".");
        let module = self.load_module_from_file(&module_path)?;

        // Import items into current environment (makes them available locally)
        self.bind_module_imports(&module, &Some(use_decl.items.clone()))?;

        // Note: The re-sharing is handled in load_module_from_file when processing ShareDecl::Use
        // This just ensures the items are available in the current module's environment

        Ok(Value::Unit)
    }

    fn eval_test_decl(&mut self, test_decl: TestDecl) -> Result<Value, InterpreterError> {
        if !self.test_mode {
            // Test blocks are inert outside `olang test`: a normal run —
            // and, critically, a `use` of a module, whose top-level
            // statements execute at load — must not run assertions,
            // print, or pay for them. (They used to execute inline;
            // importing any tested library ran its whole suite.)
            return Ok(Value::Unit);
        }

        // Runner behavior (`olang test`): record the outcome and keep going,
        // so one failing block doesn't hide the others. A block fails on a
        // raised error OR on any testing.assert_* that returned Err inside
        // it — asserts tally rather than raise, and a runner that only
        // watched for raises reported ✓ over failing assertions.
        let (_, failed_before) = crate::stdlib::testing::tally_snapshot();
        // A test block is its own scope, as a function body is: a `let`
        // inside it ends at the closing brace instead of replacing a
        // module-level name for every later block in the file (a test's
        // `let fs = …` once shadowed the `fs` module 400 lines down).
        self.environment = Environment::with_parent(self.environment.clone());
        let mut error = None;
        for statement in &test_decl.body {
            if let Err(e) = self.eval_statement(statement) {
                error = Some(e.to_string());
                break;
            }
        }
        if let Some(parent) = self.environment.parent.take() {
            self.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
        }
        if error.is_none() {
            let (_, failed_after) = crate::stdlib::testing::tally_snapshot();
            let failed = failed_after - failed_before;
            if failed > 0 {
                error = Some(format!(
                    "{} assertion{} failed",
                    failed,
                    if failed == 1 { "" } else { "s" }
                ));
            }
        }
        self.test_results.push(TestOutcome {
            name: test_decl.name,
            error,
        });
        Ok(Value::Unit)
    }

    /// Turn on test-runner mode: `test` blocks record outcomes (readable via
    /// `take_test_results`) and execution continues past failing blocks.
    /// Install a capability table. Every subsequent gated builtin call
    /// (fs/http/db/proc and the environment surface of os) is checked
    /// against the grant of the package whose code makes the call.
    /// Attach a timeline (record or replay). Like capabilities, this
    /// forces the interpreter tier: the timeline must observe every
    /// nondeterministic builtin, and the bytecode tier bridges some through
    /// a throwaway interpreter that carries no timeline.
    pub fn set_timeline(&mut self, timeline: crate::timeline::Timeline) {
        self.timeline = Some(timeline);
        self.bytecode_tier = None;
    }

    /// Called from builtin dispatch for every nondeterministic op.
    /// - Replay: returns Some(Ok(recorded value)) — the real call is
    ///   skipped entirely — or Some(Err(divergence message)).
    /// - Record: returns None (the caller performs the real call and then
    ///   calls `timeline_record`).
    /// - No timeline, or a deterministic op: returns None.
    pub fn timeline_replay(&mut self, op: &str, args: &[Value]) -> Option<Result<Value, String>> {
        if !crate::timeline::Timeline::is_recorded(op) {
            return None;
        }
        // Fingerprint only when a recorded op is actually being replayed.
        let fp = crate::timeline::Timeline::fingerprint(args);
        let timeline = self.timeline.as_mut()?;
        if timeline.mode() != crate::timeline::Mode::Replay {
            return None;
        }
        Some(
            timeline
                .replay_next(op, &fp)
                .map_err(|d| format!("timeline: {}", d)),
        )
    }

    /// Record the result of a nondeterministic op after it ran (record
    /// mode only; a no-op otherwise).
    pub fn timeline_record(&mut self, op: &str, args_fp: &str, result: &Value) {
        if let Some(timeline) = self.timeline.as_mut()
            && timeline.mode() == crate::timeline::Mode::Record
            && crate::timeline::Timeline::is_recorded(op)
        {
            timeline.record_result(op, args_fp, result);
        }
    }

    /// Whether a timeline is attached in record mode.
    pub fn timeline_recording(&self) -> bool {
        self.timeline
            .as_ref()
            .map(|t| t.mode() == crate::timeline::Mode::Record)
            .unwrap_or(false)
    }

    /// Take the timeline back out (to write the trace at end of run).
    pub fn take_timeline(&mut self) -> Option<crate::timeline::Timeline> {
        self.timeline.take()
    }

    /// Enter meta mode (macro expansion). See `expansion_denial`.
    pub fn set_meta_mode(&mut self, on: bool) {
        self.meta_mode = on;
    }

    /// Is this interpreter running meta fn bodies (macro expansion)?
    /// Auto-parallel bulk operations check this: expansion must never
    /// spawn threads, even for a provably pure kernel.
    pub fn in_meta_mode(&self) -> bool {
        self.meta_mode
    }

    /// `meta.eval` outside macro expansion: evaluate source in a child
    /// interpreter that inherits this run's authority — the capability
    /// table (evaluated code is judged by the same grants, so eval is
    /// not an escalation), the `--trace-caps` set, the dependency map,
    /// and the current file for module resolution. Effects are allowed;
    /// the pure sandbox applies only at expansion time, where it is
    /// what makes expansion deterministic. Parse and runtime failures
    /// come back as `Err(message)` values, matching the sandboxed form.
    ///
    /// A run being recorded or replayed does not extend into the child:
    /// the timeline covers the main program's own dispatch only.
    ///
    /// A budget (`max_steps`, `timeout`) bounds the child: it then runs
    /// interpreted, where every loop iteration and call is charged, so a
    /// runaway rule costs one `Err("budget exceeded ...")` rather than a
    /// thread.
    pub fn eval_source_at_runtime(
        &self,
        source: &str,
        budget: (Option<u64>, Option<std::time::Duration>),
    ) -> Value {
        let err = |m: String| Value::Err(Box::new(Value::String(std::sync::Arc::new(m))));
        let program = match crate::parser::Parser::new().parse(source) {
            Ok(p) => p,
            Err(e) => return err(format!("{}", e)),
        };
        let mut child = Interpreter::new();
        let (max_steps, timeout) = budget;
        let budgeted = max_steps.is_some() || timeout.is_some();
        // The child runs with the caller's execution model: without
        // this, evaluated source tree-walks everything — the tier and
        // the JIT exist only where a tier was enabled. A budgeted child
        // stays on the interpreter, the tier that meters steps.
        if let Some(tier) = self.bytecode_tier.as_ref()
            && !budgeted
        {
            child.enable_bytecode_tier(tier.threshold(), false);
        }
        child.set_eval_budget(max_steps, timeout);
        child.caps = self.caps.clone();
        child.caps_trace = self.caps_trace.clone();
        child.dependency_map = self.dependency_map.clone();
        // Re-seed through the setter, not the field: module discovery
        // reads the path back out of the module cache, which the setter
        // populates.
        if let Some(path) = self.current_module_path.as_ref() {
            child.set_current_file(std::path::Path::new(path));
        }
        match child.eval_program(program) {
            Ok(v) => Value::Ok(Box::new(v)),
            Err(e) => err(format!("{}", e)),
        }
    }

    /// Is line coverage being collected? Auto-parallel checks this too:
    /// worker clones do not record coverage, so fanning out would
    /// silently drop the kernel's lines from the report.
    pub fn coverage_active(&self) -> bool {
        self.coverage.is_some()
    }

    /// The purity gate for macro expansion: in meta mode, the modules
    /// that reach the outside world or the clock refuse. Checked at the
    /// same dispatch chokepoint as the capability gate, so nothing
    /// bridges around it. None = allowed; Some(message) = refused.
    pub fn expansion_denial(&self, full_name: &str) -> Option<String> {
        if !self.meta_mode {
            return None;
        }
        // The classification lives in `crate::effects`, shared with the
        // capability gate and record/replay. Anything gated, recorded, or
        // thread-shaped is refused here — which is what closed the drift
        // holes (`dates.now`, `crypto.random_*`, `ods` file I/O) that a
        // locally-curated denylist had accumulated.
        if crate::effects::expansion_blocked(full_name) {
            return Some(format!(
                "{} is not available at expansion time: a meta fn is a pure \
                 function of its arguments, so macro expansion is deterministic \
                 and reproducible (docs/macros.md)",
                full_name
            ));
        }
        None
    }

    /// Call a function bound in the global environment by name — the
    /// expander's entry for invoking a meta fn.
    pub fn call_named_function(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        let function =
            self.environment
                .get(name)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: format!(
                        "no meta fn named '{}' — declare it with `meta fn {}(...) = ...` \
                     in this file, above its first use",
                        name, name
                    ),
                })?;
        self.call_function_optimized(&function, args)
    }

    pub fn set_capabilities(&mut self, table: crate::caps::CapTable) {
        self.caps = Some(std::sync::Arc::new(table));
        // Both tiers enforce, through the same gate. `BuiltinFunctions::
        // call_internal` is the one choke point every builtin passes, on
        // either tier — what used to be missing was not the check but the
        // *context*: the tier reaches builtins through a bridge
        // interpreter that carried no capability table and no idea which
        // function was executing, so the gate saw an unrestricted run.
        // The tier now carries the table and the executing function's
        // defining file, and seeds both into the bridge before dispatch.
        if let Some(tier) = self.bytecode_tier.as_mut() {
            tier.set_capabilities(self.caps.clone());
        }
    }

    /// Turn on `--trace-caps` profiling. Runs on the interpreter tier so
    /// every effect passes the dispatch choke point (a promoted function
    /// bridges some builtins past it), exactly like the gate and coverage.
    pub fn enable_caps_trace(&mut self) {
        let trace = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
        self.caps_trace = Some(trace.clone());
        if let Some(tier) = self.bytecode_tier.as_mut() {
            tier.set_caps_trace(trace);
        }
    }

    /// Record that `full_name` exercised a capability, if profiling is on.
    /// Called from builtin dispatch alongside the gate; a no-op (one branch)
    /// when `--trace-caps` is off.
    pub fn record_caps_use(&mut self, full_name: &str) {
        if let Some(trace) = self.caps_trace.as_ref()
            && let Some(u) = crate::caps::required(full_name)
        {
            trace.lock().unwrap().insert(u);
        }
    }

    /// Take the exercised-capability set back out (to print the profile at
    /// end of run).
    pub fn take_caps_trace(&mut self) -> Option<std::collections::BTreeSet<crate::caps::CapUse>> {
        let trace = self.caps_trace.take()?;
        let set = trace.lock().unwrap().clone();
        Some(set)
    }

    /// The capability set governing the currently-executing code, and the
    /// attenuated dependency (if any) that code belongs to. None when no
    /// capabilities are active (the gate then costs a single branch).
    /// Attribution is by the executing function's `def_file`, compared as a
    /// canonicalized (memoized) real path.
    fn current_caps(&mut self) -> Option<(crate::caps::Caps, Option<String>)> {
        let table = self.caps.clone()?;
        let def_file = self
            .coverage_file_stack
            .last()
            .and_then(|f| f.as_deref())
            .or(self.current_module_path.as_deref())
            .map(|s| s.to_string());
        let canon = def_file.as_ref().map(|f| {
            self.caps_path_cache
                .entry(f.clone())
                .or_insert_with(|| {
                    std::fs::canonicalize(f).unwrap_or_else(|_| std::path::PathBuf::from(f))
                })
                .clone()
        });
        let (caps, package) = table.caps_for(canon.as_deref());
        Some((*caps, package.map(|s| s.to_string())))
    }

    /// The grant governing the currently-executing code, for the `caps`
    /// module to report. With no manifest loaded a program is
    /// unrestricted, so the default grant (everything) is the honest
    /// answer — `caps.allowed("fs")` is true precisely when an `fs` call
    /// would be permitted.
    pub fn effective_caps(&mut self) -> crate::caps::Caps {
        self.current_caps()
            .map(|(caps, _)| caps)
            .unwrap_or_default()
    }

    /// The capability gate, called from builtin dispatch. None = allowed.
    /// Some(message) = denied, with the message naming the capability,
    /// the call, and the package whose grant refused it.
    /// Arm the compile-time capability pre-grant for the next builtin
    /// dispatch: the bytecode compiler proved the static manifest grants
    /// that call, so `capability_denial` skips its table walk once.
    /// One-shot by design — consumed by the very next check, which is
    /// the first thing `call_internal` does.
    pub fn set_cap_pregranted(&mut self) {
        self.cap_pregranted = true;
    }

    pub fn capability_denial(&mut self, full_name: &str) -> Option<String> {
        if self.cap_pregranted {
            self.cap_pregranted = false;
            return None;
        }
        let (caps, package) = self.current_caps()?;
        let denied = crate::caps::check(&caps, full_name)?;
        Some(match package {
            Some(pkg) => format!(
                "capability '{}' denied: {} requires it, and dependency '{}' is granted {} (olang.toml [capabilities.dependencies.{}])",
                denied,
                full_name,
                pkg,
                caps.summary(),
                pkg
            ),
            None => format!(
                "capability '{}' denied: {} requires it, and this program is granted {} ([capabilities] manifest or --deny)",
                denied,
                full_name,
                caps.summary()
            ),
        })
    }

    /// The filesystem sub-gate: some effectful builtins take a path argument
    /// and touch the filesystem beyond their own module capability
    /// (`db.open` on a file, a `db` query running `ATTACH`). This confines
    /// that effect under `fs`, so `db` is not a latent filesystem
    /// capability. Called from builtin dispatch alongside `capability_denial`;
    /// a no-op (one branch) when no capabilities are active.
    pub fn implied_fs_denial(&mut self, full_name: &str, args: &[Value]) -> Option<String> {
        self.caps.as_ref()?;
        let relevant = match full_name {
            "db.open" => args.first(),
            "db.execute" | "db.query" | "db.query_one" => args.get(1),
            _ => return None,
        };
        let path = match relevant {
            Some(Value::String(s)) => Some(s.as_str().to_string()),
            _ => None,
        };
        let needed = crate::caps::implied_fs(full_name, path.as_deref())?;
        let (caps, package) = self.current_caps()?;
        if caps.fs.allows(needed) {
            return None;
        }
        Some(match package {
            Some(pkg) => format!(
                "capability 'fs' denied: {} reaches the filesystem (needs fs={}), and dependency '{}' is granted {} (olang.toml [capabilities.dependencies.{}])",
                full_name,
                needed.word(),
                pkg,
                caps.summary(),
                pkg
            ),
            None => format!(
                "capability 'fs' denied: {} reaches the filesystem (needs fs={}), and this program is granted {} ([capabilities] manifest or --deny)",
                full_name,
                needed.word(),
                caps.summary()
            ),
        })
    }

    /// The application's filesystem grant (or `Full` when no capabilities are
    /// active). Used where an effect happens outside the dispatch gate — the
    /// `http.serve` response builder reading a `body_file`.
    pub fn effective_fs(&self) -> crate::caps::FsCap {
        self.caps
            .as_ref()
            .map(|t| t.app.fs)
            .unwrap_or(crate::caps::FsCap::Full)
    }

    pub fn enable_test_mode(&mut self) {
        self.test_mode = true;
    }

    /// The file whose code is running, for artifacts that live beside it
    /// (`testing.snapshot`): the current module when one is loading, the
    /// entry file otherwise.
    pub fn current_file_for_snapshots(&self) -> Option<std::path::PathBuf> {
        self.current_module_path
            .as_deref()
            .filter(|p| !p.starts_with("__"))
            .or(self.entry_file.as_deref())
            .map(std::path::PathBuf::from)
    }

    /// The outcomes of every `test` block run so far, clearing the record.
    pub fn take_test_results(&mut self) -> Vec<TestOutcome> {
        std::mem::take(&mut self.test_results)
    }

    /// Turn on line-coverage recording. Every executed located statement is
    /// tallied under the file that owns the code. Enable this *instead of*
    /// the bytecode tier: coverage is instrumented on the AST walk, so a
    /// promoted function would run past the hook unrecorded.
    pub fn enable_coverage(&mut self) {
        self.coverage = Some(HashMap::new());
    }

    /// The recorded coverage so far — file path -> the set of line numbers
    /// executed in it — clearing the record.
    pub fn take_coverage(&mut self) -> Option<HashMap<String, std::collections::BTreeSet<u32>>> {
        self.coverage.take()
    }
}

/// The result of one `test "name" { ... }` block under `olang test`.
#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub name: String,
    /// `None` when the block passed; the failure message otherwise.
    pub error: Option<String>,
}

pub(crate) mod spawn_registry;

mod module_cache;
pub use module_cache::{
    CacheCleanupStats, CacheStatistics, ModuleCacheEntry, ModuleDependencyTracker, SmartCacheConfig,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_cache_creation_and_retrieval() {
        let mut interpreter = Interpreter::new();

        // Test initial state
        assert_eq!(interpreter.module_cache.len(), 0);
        assert!(!interpreter.is_module_cached("test_module"));

        // Cache a module
        let test_module = Value::Struct {
            type_name: "Module".to_string(),
            fields: std::sync::Arc::new(HashMap::new()),
        };

        interpreter
            .cache_module(
                "test_module".to_string(),
                test_module.clone(),
                None,
                Vec::new(),
            )
            .unwrap();

        // Verify module is cached
        assert!(interpreter.is_module_cached("test_module"));
        let cached = interpreter
            .get_cached_module("test_module")
            .unwrap()
            .unwrap();
        assert_eq!(cached.dependencies.len(), 0);

        // Test cache statistics
        let (cached_modules, _, _) = interpreter.get_module_cache_stats();
        assert_eq!(cached_modules, 1);
    }

    #[test]
    fn test_module_dependency_tracking() {
        let mut tracker = ModuleDependencyTracker::new();

        // Add some dependencies: A -> B, A -> C, B -> D
        tracker.add_dependency("A".to_string(), "B".to_string());
        tracker.add_dependency("A".to_string(), "C".to_string());
        tracker.add_dependency("B".to_string(), "D".to_string());

        // Test dependency queries
        assert_eq!(tracker.dependencies.get("A").unwrap().len(), 2);
        assert!(
            tracker
                .dependencies
                .get("A")
                .unwrap()
                .contains(&"B".to_string())
        );
        assert!(
            tracker
                .dependencies
                .get("A")
                .unwrap()
                .contains(&"C".to_string())
        );

        // Test dependent queries
        assert_eq!(tracker.dependents.get("B").unwrap().len(), 1);
        assert!(
            tracker
                .dependents
                .get("B")
                .unwrap()
                .contains(&"A".to_string())
        );

        // Test circular dependency detection
        assert!(!tracker.check_circular_dependency("A", "D")); // A -> B -> D (no cycle)
        assert!(tracker.check_circular_dependency("D", "A")); // D -> A would create cycle
    }

    // Removed test_circular_dependency_prevention as it used the old import system

    #[test]
    fn test_module_cache_invalidation() {
        let mut interpreter = Interpreter::new();

        // Cache some modules with dependencies
        let module_a = Value::Struct {
            type_name: "Module".to_string(),
            fields: std::sync::Arc::new(HashMap::new()),
        };
        let module_b = Value::Struct {
            type_name: "Module".to_string(),
            fields: std::sync::Arc::new(HashMap::new()),
        };

        interpreter
            .cache_module("module_a".to_string(), module_a, None, Vec::new())
            .unwrap();
        interpreter
            .cache_module("module_b".to_string(), module_b, None, Vec::new())
            .unwrap();

        // Set up dependency: B depends on A
        interpreter
            .dependency_tracker
            .add_dependency("module_b".to_string(), "module_a".to_string());

        // Verify both modules are cached
        assert!(interpreter.is_module_cached("module_a"));
        assert!(interpreter.is_module_cached("module_b"));

        // Invalidate module A
        interpreter.invalidate_module("module_a");

        // Verify both A and its dependent B are invalidated
        assert!(!interpreter.is_module_cached("module_a"));
        assert!(!interpreter.is_module_cached("module_b"));
    }

    #[test]
    fn test_dependency_chain_analysis() {
        let mut interpreter = Interpreter::new();

        // Create dependency chain: A -> B -> C -> D
        interpreter
            .dependency_tracker
            .add_dependency("A".to_string(), "B".to_string());
        interpreter
            .dependency_tracker
            .add_dependency("B".to_string(), "C".to_string());
        interpreter
            .dependency_tracker
            .add_dependency("C".to_string(), "D".to_string());

        let chain = interpreter.get_dependency_chain("A");

        // Should include all dependencies in the chain
        assert!(chain.contains(&"B".to_string()));
        assert!(chain.contains(&"C".to_string()));
        assert!(chain.contains(&"D".to_string()));

        // Test dependency graph export
        let graph = interpreter.export_dependency_graph();
        assert!(graph.contains_key("A"));
        assert!(graph.contains_key("B"));
        assert!(graph.contains_key("C"));
    }

    #[test]
    fn test_module_cache_clearing() {
        let mut interpreter = Interpreter::new();

        // Cache some modules
        let test_module = Value::Struct {
            type_name: "Module".to_string(),
            fields: std::sync::Arc::new(HashMap::new()),
        };

        interpreter
            .cache_module("module1".to_string(), test_module.clone(), None, Vec::new())
            .unwrap();
        interpreter
            .cache_module("module2".to_string(), test_module, None, Vec::new())
            .unwrap();
        interpreter
            .dependency_tracker
            .add_dependency("module1".to_string(), "module2".to_string());

        // Verify modules are cached and dependencies exist
        assert!(interpreter.is_module_cached("module1"));
        assert!(interpreter.is_module_cached("module2"));
        assert!(!interpreter.dependency_tracker.dependencies.is_empty());

        // Clear cache
        interpreter.clear_module_cache();

        // Verify everything is cleared
        assert!(!interpreter.is_module_cached("module1"));
        assert!(!interpreter.is_module_cached("module2"));
        assert!(interpreter.dependency_tracker.dependencies.is_empty());
        assert!(interpreter.dependency_tracker.dependents.is_empty());
    }

    #[test]
    fn test_would_create_circular_dependency() {
        let mut interpreter = Interpreter::new();

        // Set up a simple dependency chain: A -> B -> C
        interpreter
            .dependency_tracker
            .add_dependency("A".to_string(), "B".to_string());
        interpreter
            .dependency_tracker
            .add_dependency("B".to_string(), "C".to_string());

        // Test various potential circular dependencies
        assert!(!interpreter.would_create_circular_dependency("A", "D")); // No cycle
        assert!(!interpreter.would_create_circular_dependency("D", "E")); // No cycle
        assert!(interpreter.would_create_circular_dependency("C", "A")); // Would create cycle
        assert!(interpreter.would_create_circular_dependency("C", "B")); // Would create cycle
        assert!(interpreter.would_create_circular_dependency("B", "A")); // Would create cycle
    }
}

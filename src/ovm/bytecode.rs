//! Register-based bytecode VM for intermediate-tier execution between interpreter and JIT

// The lint wants `ok_or` where the error payload is Copy, but BytecodeError
// has drop glue (String variants), and eagerly constructing an error on every
// successful register/constant access measured at ~15% of VM samples in a
// profile. Lazy construction is deliberate on the dispatch hot path.
#![allow(clippy::unnecessary_lazy_evaluations)]

use crate::ast::{BinaryOp, Expr, FunctionDecl, UnaryOp, Value};
use crate::builtin::BuiltinFunctions;
use crate::ovm::{FunctionId, OvmValue};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// The most native frames one JIT call chain may push before it deopts
/// back to bytecode. The logical call-depth cap is 100k, but jitted
/// recursion lives on the real Rust stack where headroom below a grown
/// segment's red zone is only guaranteed to ~1MB — this is the depth
/// that provably fits it, and exactly the ceiling the JIT lived under
/// when the cap itself was 1000. JIT groups are pure, so exhaustion
/// abandons the native run and re-executes on bytecode: deeper pure
/// recursion is slower, never wrong.
#[cfg(feature = "native")]
const JIT_NATIVE_DEPTH_BUDGET: u32 = 1000;

/// Back-edges a single frame spends on the VM before its loop is
/// offered to on-stack replacement. High enough that short loops never
/// pay the synthesis + compile cost; against a loop hot enough to
/// matter (hundreds of thousands of iterations), the ~8k VM iterations
/// spent warming up are noise.
#[cfg(feature = "native")]
const OSR_BACK_EDGE_THRESHOLD: u64 = 8192;

/// Register-based bytecode virtual machine
pub struct BytecodeVm {
    // Bytecode compiler
    compiler: BytecodeCompiler,

    // Compiled bytecode cache
    // Arc-wrapped so a call clones a pointer, not the instruction vector
    bytecode_cache: Arc<RwLock<HashMap<FunctionId, Arc<CompiledBytecode>>>>,
    /// Lock-free mirror of the cache for the call path, indexed directly by
    /// FunctionId, BOUNDED to `HOT_CAP` slots. Ids come from a global
    /// monotonic counter, so a program that compiles functions in many
    /// short-lived VMs (a `spawn`ed worker pool per tick) would otherwise
    /// size this Vec to the global high-water — a worker minting id 96_000
    /// allocating 96_000 slots for its two functions (the per-tick soak
    /// leak). Above the cap, calls fall through to `bytecode_cache`; the
    /// mirror still covers every function a normal program actually runs
    /// hot (its ids are assigned early, well under the cap). Sound
    /// because ids come from a global monotonic counter and are never reused:
    /// an entry, once cached, can never refer to different bytecode.
    bytecode_hot: Vec<Option<Arc<CompiledBytecode>>>,

    // Runtime execution state
    execution_state: ExecutionState,

    // Performance statistics
    stats: VmStatistics,

    // Function registry for dynamic calls
    function_registry: HashMap<String, FunctionId>,

    // Builtin function registry
    /// Builtins the VM will execute, by name. Kept to a curated set: each
    /// takes only value arguments and returns a value that round-trips
    /// losslessly through the OVM value model.
    builtin_names: std::collections::HashSet<String>,
    /// The interpreter's builtin implementations, used directly rather than
    /// reimplemented — reimplementation would drift from the semantics the
    /// differential tests hold the VM to.
    builtins: BuiltinFunctions,
    /// Scratch interpreter that builtin calls run against. Boxed to break the
    /// Interpreter -> BytecodeTier -> BytecodeVm -> Interpreter type cycle,
    /// and created on first use since most functions call no builtins.
    builtin_interpreter: Option<Box<crate::interpreter::Interpreter>>,

    /// The run's capability table, and the `--trace-caps` set, both shared
    /// with the owning interpreter. The bridge interpreter below is seeded
    /// from these before every builtin dispatch: it is a separate
    /// `Interpreter`, so without this it would present an unrestricted run
    /// to the gate.
    caps: Option<Arc<crate::caps::CapTable>>,
    caps_trace: Option<Arc<std::sync::Mutex<std::collections::BTreeSet<crate::caps::CapUse>>>>,
    /// Count of named functions compiled via the hof dependency channel.
    hof_promotions: u32,
    /// Defining file of each compiled function currently executing,
    /// innermost last — the tier's mirror of the interpreter's
    /// `coverage_file_stack`. This is what attributes a gated builtin to
    /// the package whose code called it.
    caps_file_stack: Vec<Option<Arc<str>>>,
    /// Current nesting depth of execute(); bounds Rust stack growth from
    /// recursive CallNamed so runaway recursion errors instead of aborting
    call_depth: u32,
    max_call_depth: u32,
    /// The baseline JIT: native code for pure-integer hot functions,
    /// entered from execute()/execute_from_regs when the entry guard
    /// (all-Integer arguments) holds. See src/ovm/jit.rs.
    #[cfg(feature = "native")]
    jit: crate::ovm::jit::JitCache,
    /// On-stack replacement table: per original function, the compiled
    /// hot-loop region (None = analyzed and refused, never retried).
    /// Filled lazily, the first time a frame's back-edge counter crosses
    /// OSR_BACK_EDGE_THRESHOLD.
    #[cfg(feature = "native")]
    osr_regions: HashMap<FunctionId, Option<std::sync::Arc<crate::ovm::osr::OsrRegion>>>,
    /// Trace of the error currently unwinding: the innermost located
    /// statement's span, and function names innermost-first. Frames
    /// deeper than the first span-owning frame are dropped — exactly the
    /// frames the interpreter has already popped when it captures at its
    /// innermost Located statement.
    error_trace_span: Option<(u32, u32)>,
    error_trace_frames: Vec<String>,
    /// A parameter-check failure keeps exactly one extra frame alive in
    /// the interpreter (the frame is pushed before the check and the
    /// early error return skips the pop; each enclosing frame's own pop
    /// then consumes the leaked slot, so the surviving extra frame
    /// shifts outward until a span captures). Mirror that observable.
    error_trace_leak: Option<String>,
    /// Spare execution frames, pooled so register/local vectors keep their
    /// allocated capacity across calls
    /// Pooled argument buffers for Call* instructions, so marshaling a call's
    /// arguments does not malloc on every call.
    arg_pool: Vec<Vec<OvmValue>>,
    /// Function *values* compiled on demand for the native higher-order
    /// path, keyed by *body and captured-environment* allocation identity
    /// and validated by Weak upgrade (a freed-and-reused address yields a
    /// dead Weak, never a stale hit). `None` records a failed compile so
    /// it is not retried.
    ///
    /// The environment must be part of the key. `compile_function_with_
    /// closure` bakes the captures into the compiled body, so two closures
    /// from one factory — same lambda body, different captures — are
    /// different compiled functions. Keying on the body alone made the
    /// second closure run the first one's captures:
    ///
    /// ```text
    /// fn apply(f, x) = f(x)
    /// fn adder(k) = (n) => n + k
    /// apply(adder(1), 0)      // 1
    /// apply(adder(100), 0)    // 1   — was 100 on the interpreter
    /// ```
    #[allow(clippy::type_complexity)]
    hof_cache: HashMap<
        (usize, usize),
        (
            std::sync::Weak<Expr>,
            std::sync::Weak<im::HashMap<String, crate::ast::Value>>,
            Option<FunctionId>,
        ),
    >,
    /// Declared struct shapes (type name -> field names), mirrored from the
    /// interpreter's registry so struct literals validate at compile time
    /// with exactly the interpreter's rules.
    struct_defs: HashMap<String, Vec<String>>,
    /// Declared struct field types (type name -> field name -> checkable
    /// type), mirrored from the interpreter. MakeStruct enforces these at
    /// run time so a field value whose runtime type does not match its
    /// declared annotation is the same error the interpreter raises.
    struct_field_checks: HashMap<String, HashMap<String, crate::ast::FieldTypeCheck>>,
    /// Declared enum *type* names, mirrored from the interpreter alongside
    /// struct_defs, so the tier's annotation enforcement can tell an unknown
    /// type from a real mismatch and report it the same way — an undeclared
    /// name in an annotation matches nothing, and blaming the value misleads.
    enum_type_names: std::collections::HashSet<String>,
    /// User function VALUES by name, mirrored from the tier's
    /// declarations. Used when a lambda's free variable is a registered
    /// function: the compiled body calls it through the registry, but the
    /// lambda's AST form must still CARRY the function in its closure so
    /// it behaves identically when it escapes to the interpreter (bridged
    /// builtins, returned values).
    known_function_values: HashMap<String, crate::ast::Function>,
    /// Bare names bound to more than one distinct function body over the
    /// run — a nested `fn insert` in two modules, say. The bridge seeds
    /// its environment only with unambiguous names; a name here would
    /// bind one module's function (with one call's captures) where
    /// another module's was meant.
    ambiguous_function_names: std::collections::HashSet<String>,
    /// Trait dispatch registries, mirrored from the interpreter as
    /// declarations evaluate: (type name, method) → impl method,
    /// (trait name, method) → default method, and type → traits it
    /// implements. Lookup order matches `Interpreter::lookup_method`
    /// exactly: a direct impl wins, then the type's traits are scanned in
    /// registration order for a default.
    trait_impls: HashMap<(String, String), crate::ast::Function>,
    trait_defaults: HashMap<(String, String), crate::ast::Function>,
    type_traits: HashMap<String, Vec<String>>,
    /// Declared unit enum variant names: a bare identifier pattern with
    /// one of these names is an equality match, not a binding — the same
    /// rule the interpreter applies.
    unit_variant_names: std::collections::HashSet<String>,
    /// Types redeclared with a *different* field set. Compile-time
    /// validation would go stale for them, so their literals are never
    /// compiled again — the interpreter (whose registry is live) stays the
    /// authority.
    poisoned_structs: std::collections::HashSet<String>,
}

/// Bytecode compiler that transforms AST to bytecode
pub struct BytecodeCompiler {
    /// The run's capability table, when one is installed — the source of
    /// compile-time verdicts. Kept in sync by `set_capabilities`.
    static_caps: Option<Arc<crate::caps::CapTable>>,
    /// Canonicalized provenance of the function being compiled; grants
    /// attenuate by dependency directory, so the verdict needs it.
    current_def_file: Option<std::path::PathBuf>,
    /// Checks for the function currently being compiled, handed in by the
    /// tier (precomputed at declaration; generic params already erased).
    pub pending_param_checks: std::sync::Arc<[Option<crate::ast::FieldTypeCheck>]>,
    pub pending_return_check: Option<crate::ast::FieldTypeCheck>,
    /// Defining file of the function currently being compiled, handed in
    /// the same way and baked into the emitted `CompiledBytecode` so the
    /// capability gate can attribute its effects.
    pub pending_def_file: Option<Arc<str>>,
    // Register allocator
    register_allocator: RegisterAllocator,

    // Instruction emitter
    emitter: InstructionEmitter,

    // Optimization passes
    optimizer: BytecodeOptimizer,

    // Local variable tracking
    /// Variable name -> the register that holds it (a register window:
    /// parameters occupy registers 0..n, so reading a variable is free
    /// rather than a LoadLocal that clones out of a separate array)
    local_variables: HashMap<String, Register>,
    /// Builtin names the VM implements (for compile-time callee validation)
    builtin_names: std::collections::HashSet<String>,
    /// Enclosing loops, innermost last: (continue target, break target)
    loop_targets: Vec<(Label, Label)>,
    /// Enclosing Located statements, innermost last, for span markers.
    span_stack: Vec<(u32, u32)>,
    /// The enclosing function's declaration-time closure. Lambdas whose free
    /// variables all resolve here can carry it verbatim, which is exactly the
    /// snapshot the interpreter layers over the call-site chain.
    enclosing_closure: std::sync::Arc<im::HashMap<String, Value>>,
    /// Every name the enclosing function ever binds or assigns (params, lets,
    /// loop variables, match bindings, assignment targets). A lambda free
    /// variable in this set is a capture of runtime state, not of the
    /// closure, and must be rejected.
    enclosing_bound_names: std::collections::HashSet<String>,

    // Label tracking for control flow
    _label_counter: u32,

    // Function registry for calls
    function_registry: HashMap<String, FunctionId>,

    /// Declared struct shapes for compile-time literal validation
    /// (mirrored from the interpreter; poisoned types are absent).
    struct_defs: HashMap<String, Vec<String>>,

    /// Declared struct field types for run-time MakeStruct enforcement
    /// (mirrored from the interpreter).
    struct_field_checks: HashMap<String, HashMap<String, crate::ast::FieldTypeCheck>>,

    /// Declared unit enum variant names (see BytecodeVm::unit_variant_names).
    unit_variant_names: std::collections::HashSet<String>,

    /// User function values for lambda-closure attachment (see
    /// BytecodeVm::known_function_values).
    known_function_values: HashMap<String, crate::ast::Function>,

    /// Set while compiling a named nested fn's pending body: (name, id,
    /// real parameter count, capture count). A self-call must append the
    /// body's own capture parameters — registers real..real+captures — or
    /// recursion arrives without its captures (binary_search's nested `go`
    /// recursed with 2 args into a 4-parameter body before this).
    self_call: Option<(String, FunctionId, usize, usize)>,

    /// Lambdas with runtime captures met during this compile: each is the
    /// lambda body as a standalone declaration whose trailing parameters
    /// are the captured names, compiled by the VM after the enclosing
    /// function (the compiler's per-function state can't nest). If any of
    /// them fails, the whole enclosing compilation fails — a MakeClosure
    /// must never reference an id with no bytecode behind it.
    pending_lambdas: Vec<PendingLambda>,
}

/// A deferred lambda compile: its id, the body as a standalone declaration
/// (captures as trailing parameters), the declaration-time closure to
/// compile it against, and — for a named nested fn — its own name, bound
/// to its own id during compilation so self-recursion resolves.
type PendingLambda = (
    FunctionId,
    FunctionDecl,
    std::sync::Arc<im::HashMap<String, Value>>,
    Option<(String, usize)>,
);

/// Bytecode optimization engine
#[allow(dead_code)]
/// Placeholder for future optimization passes.
///
/// The previous pipeline (dead-code elimination, register renaming, peephole
/// rewrites, control-flow "optimization") was deleted rather than fixed: it
/// removed live control flow and stores, renamed registers for only a subset
/// of opcodes, and treated label IDs as instruction addresses. Passes may
/// return once they can be validated against the differential test suite.
pub struct BytecodeOptimizer {}

/// Compiled bytecode representation
#[derive(Debug, Clone)]
pub struct CompiledBytecode {
    pub function_id: FunctionId,
    pub instructions: Vec<Instruction>,
    pub register_count: u32,
    pub local_count: u32,
    /// Declared parameter count, enforced at call time
    pub param_count: usize,
    /// The file this function was declared in, for capability
    /// attribution. `None` for synthesized functions and for programs
    /// with no file (the REPL), which attribute to the program itself.
    pub def_file: Option<Arc<str>>,
    /// Parameter names in declaration order — only for arity-error
    /// messages, which must match the interpreter's word-for-word.
    pub param_names: std::sync::Arc<[String]>,
    /// Runtime type checks for annotated parameters (position-aligned;
    /// all-None when the function is fully dynamic). Precomputed at
    /// declaration — generic parameters are already erased to None, so
    /// the VM never re-derives them from annotations.
    pub param_checks: std::sync::Arc<[Option<crate::ast::FieldTypeCheck>]>,
    /// Runtime check for the declared return type.
    pub return_check: Option<crate::ast::FieldTypeCheck>,
    pub constants: Vec<OvmValue>,
    /// Statement-granularity source spans: (first instruction index,
    /// line, column), in emission order. On a runtime error the VM maps
    /// the failing pc to the last marker at or before it — the same
    /// "innermost located statement" the interpreter reports.
    pub span_table: Vec<(u32, u32, u32)>,
    pub debug_info: BytecodeDebugInfo,
    pub optimization_level: u8,
    pub entry_point: usize,
}

/// One piece of a compiled template string.
#[derive(Debug, Clone, PartialEq)]
pub enum TplPart {
    Literal(String),
    Reg(Register),
}

/// A GetField site's one-entry inline cache, packing (shape id << 32 |
/// field index) into one atomic word so concurrent VMs sharing the
/// bytecode can never see a torn pair. Zero means cold (shape ids start
/// at 1). Cloned instructions start cold; caches never affect equality.
#[derive(Debug, Default)]
pub struct FieldCache(std::sync::atomic::AtomicU64);

impl FieldCache {
    #[inline]
    pub fn load(&self) -> (u32, u32) {
        let packed = self.0.load(std::sync::atomic::Ordering::Relaxed);
        ((packed >> 32) as u32, packed as u32)
    }
    #[inline]
    pub fn store(&self, shape_id: u32, index: u32) {
        self.0.store(
            ((shape_id as u64) << 32) | index as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl Clone for FieldCache {
    fn clone(&self) -> Self {
        FieldCache::default()
    }
}

impl PartialEq for FieldCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Bytecode instruction set - Enhanced with more operations
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    // Load/Store operations
    LoadConst {
        dst: Register,
        const_idx: u32,
    },
    LoadLocal {
        dst: Register,
        local_idx: u32,
    },
    StoreLocal {
        src: Register,
        local_idx: u32,
    },
    Move {
        dst: Register,
        src: Register,
    },

    // Arithmetic operations
    Add {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    /// The fused accumulate pattern `x = x + rhs`. Semantically exactly
    /// Add{dst: target, lhs: target} followed by nothing (the result
    /// lands in place) — but when the target holds the only reference
    /// to its string, the executor appends in place, making string
    /// building O(n) instead of O(n^2). Strings are immutable values
    /// with content equality, so identity is unobservable.
    /// `x = col.set(x, i, v)`, fused (Campaign 6): write one element of
    /// the list in `target`'s register, in place when the register holds
    /// the only reference — the indexed twin of AddAssign's extend
    /// fusion, and the write primitive of the olang-source collections.
    ListSetAssign {
        target: Register,
        index: Register,
        value: Register,
    },
    /// `x = col.swap(x, i, j)`, fused: exchange two elements in place
    /// under the same sole-owner discipline.
    ListSwapAssign {
        target: Register,
        i: Register,
        j: Register,
    },
    AddAssign {
        target: Register,
        rhs: Register,
    },
    Sub {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Mul {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Div {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Mod {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Neg {
        dst: Register,
        src: Register,
    },

    // Comparison operations
    Eq {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Ne {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Lt {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Le {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Gt {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Ge {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },

    // Logical operations
    And {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Or {
        dst: Register,
        lhs: Register,
        rhs: Register,
    },
    Not {
        dst: Register,
        src: Register,
    },

    // Control flow
    Jump {
        target: Label,
    },
    JumpIfTrue {
        condition: Register,
        target: Label,
    },
    JumpIfFalse {
        condition: Register,
        target: Label,
    },

    // Function operations
    Call {
        dst: Register,
        function: Register,
        args: Vec<Register>,
        arg_count: u32,
    },
    /// Build a struct (or anonymous object): the shape was interned at
    /// compile time and `field_regs` is already in the shape's field
    /// order, so execution just moves values into place. The literal was
    /// validated against the declared field set with the interpreter's
    /// exact rules.
    MakeStruct {
        dst: Register,
        shape: Arc<crate::ovm::value::StructShape>,
        field_regs: Vec<Register>,
        /// Declared field types in the shape's field order, parallel to
        /// `field_regs`. `Some` fields are checked against the value's
        /// runtime type at construction; `None` fields (anonymous objects,
        /// or fields with an unenforceable annotation) stay dynamic.
        field_types: Arc<[Option<crate::ast::FieldTypeCheck>]>,
    },
    /// Build a map from (key, value) register pairs, coercing keys with
    /// the interpreter's exact rule: String raw, Int/Float/Bool via
    /// to_string, anything else the interpreter's type error.
    MakeMap {
        dst: Register,
        entries: Vec<(Register, Register)>,
    },
    /// Build a template string: literal chunks verbatim, interpolated
    /// registers stringified with the interpreter's exact rules (String
    /// raw, Int/Float/Bool via to_string, everything else through the AST
    /// value's Display).
    MakeTemplate {
        dst: Register,
        parts: Vec<TplPart>,
    },
    /// A method call `receiver.m(args)` on a receiver held in a register.
    /// Mirrors the interpreter's dispatch exactly: a struct FIELD named
    /// `m` takes precedence (called without self); otherwise the method
    /// resolves through the trait registries on the receiver's runtime
    /// type and is called with the receiver prepended as self; otherwise
    /// the field-access error. Only compiled when the receiver expression
    /// is a plain local, so the interpreter's re-evaluation quirk on the
    /// field path is unobservable.
    CallMethod {
        dst: Register,
        object: Register,
        method: String,
        args: Vec<Register>,
    },
    /// Call whatever function value the callee register holds — a
    /// parameter, a local, the result of another call. Compiled function
    /// values run in the VM; anything else takes the interpreter, which
    /// stays the authority for arity errors, defaults, and non-callables.
    CallValue {
        dst: Register,
        callee: Register,
        args: Vec<Register>,
    },
    /// A binary operation whose right operand is a compile-time numeric
    /// literal, carried in the instruction — no LoadConst dispatch, no
    /// constant register. Semantics are identical to the two-instruction
    /// form: binary_fast with execute_binary_op as the fallback, so
    /// overflow, division by zero, and type errors match exactly.
    BinImm {
        op: BinaryOp,
        dst: Register,
        lhs: Register,
        imm: OvmValue,
        /// True when the compiler flipped `imm <op> x` into this form:
        /// the immediate was the SOURCE-LEFT operand. Execution semantics
        /// already account for the flip via `op`; this flag only lets the
        /// type-error path report operands in source order, so the message
        /// matches the interpreter word-for-word.
        swapped: bool,
    },
    /// Build a tuple-variant enum value from argument registers (the only
    /// runtime construction form: unit variants are constants, and struct
    /// variants have no construction syntax).
    MakeEnum {
        dst: Register,
        type_name: String,
        variant_name: String,
        args: Vec<Register>,
    },
    /// The enum-variant pattern test, mirroring the interpreter's decision
    /// tree exactly: an Enum matches when the variant name matches and the
    /// payload count equals `pattern_count` (Unit counts as zero); a plain
    /// Tuple of matching length also matches (legacy behavior, variant
    /// name ignored); anything else does not match.
    PatternTestEnum {
        dst: Register,
        value: Register,
        variant_name: String,
        pattern_count: usize,
    },
    /// Extract payload element `index` after PatternTestEnum has passed:
    /// tuple payloads by position, struct-variant payloads by position in
    /// field-name order (the interpreter sorts by name for positional
    /// matching), legacy tuples by position.
    ExtractEnumPayload {
        dst: Register,
        value: Register,
        index: usize,
    },
    /// True when the value is a struct carrying the named field. Never
    /// errors: a non-struct value is simply `false`, as in the
    /// interpreter's pattern matcher.
    PatternTestStructField {
        dst: Register,
        value: Register,
        field_name: String,
    },
    /// Build a runtime closure: clone the template ClosureObject stored at
    /// `template_const` and fill its `captured` values from the given
    /// registers, read at this instant — the interpreter's capture-by-value
    /// moment. The template's func_id points at the lambda body compiled
    /// with the captures as hidden trailing parameters.
    MakeClosure {
        dst: Register,
        template_const: u32,
        captures: Vec<Register>,
    },
    /// Call a user function resolved to its id at COMPILE time - no name
    /// hash on the call path. Emitted whenever the compiler sees the callee
    /// in its registry (always true for user functions, which pre-register
    /// before compilation, including mutual recursion).
    CallFn {
        dst: Register,
        func_id: FunctionId,
        args: Vec<Register>,
        /// Bitmask (by argument position) of argument registers the call
        /// consumes by *move* rather than clone — set by the optimizer's
        /// liveness pass for registers dead after the call, so a handle
        /// passed into a callee is not also pinned in the caller's frame
        /// for the callee's whole run (Campaign 7, T2). Bit i covers
        /// args[i]; only single-occurrence registers are ever marked.
        arg_moves: u64,
    },
    CallBuiltin {
        dst: Register,
        builtin_id: u32,
        args: Vec<Register>,
    },
    CallNamed {
        dst: Register,
        function_name: String,
        args: Vec<Register>,
        /// Compile-time capability verdict (Campaign: caps-specialized
        /// compilation): true when the static manifest fully grants this
        /// call for the enclosing function's provenance, so the runtime
        /// gate's per-call table walk is skipped. Denied and unknown
        /// verdicts stay false — the runtime gate is the authority for
        /// them, message and all.
        pregranted: bool,
    },
    Return {
        value: Option<Register>,
    },
    /// Move a value between registers, leaving Unit behind (Campaign 7,
    /// T2): the compiled form of passing a handle *by move* into a call
    /// that rebinds it — `t = grow_if_crowded(t)` — so the caller's
    /// register does not pin the Arc for the callee's whole run. The
    /// source register is rewritten by the assignment when the call
    /// returns; an error aborts the program before anything can read
    /// the Unit.
    TakeMove {
        dst: Register,
        src: Register,
    },
    /// A self-call in tail position (Campaign 5, R4b): rebind the
    /// parameter registers and jump to the entry point instead of
    /// pushing a frame. Rewritten from `CallFn` by the compiler's
    /// tail-call pass when the call's result flows untouched to a
    /// `Return`, so tail recursion runs at O(1) stack and O(1) logical
    /// depth — identically to the interpreter's trampoline.
    TailCallSelf {
        args: Vec<Register>,
    },

    // Collection operations
    MakeList {
        dst: Register,
        elements: Vec<Register>,
    },
    /// Iteration count for a `for` loop source (a list or a range).
    /// Ranges are not materialized — this is the count the interpreter's
    /// loop would produce.
    IterLen {
        dst: Register,
        src: Register,
    },
    /// The `idx`-th element of a `for` loop source (a list or a range).
    IterGet {
        dst: Register,
        src: Register,
        idx: Register,
    },
    /// Total equality used by pattern tests: operands of different types
    /// compare unequal rather than raising a type error, because a pattern
    /// that doesn't apply must simply not match.
    PatternEq {
        dst: Register,
        value: Register,
        other: Register,
    },
    /// Total range test for range patterns: a non-integer scrutinee compares
    /// false rather than raising a type error.
    PatternInRange {
        dst: Register,
        value: Register,
        lo: i64,
        hi: i64,
        inclusive: bool,
    },
    /// Whether a value is an `Ok` (or `Err` when `want_ok` is false).
    PatternTestResult {
        dst: Register,
        value: Register,
        want_ok: bool,
    },
    /// The payload of an `Ok`/`Err`. Guarded by PatternTestResult, so a
    /// mismatch here means miscompiled bytecode rather than a failed match.
    ExtractResult {
        dst: Register,
        value: Register,
        want_ok: bool,
    },
    /// Whether a value is a list of the required length (at least `min_len`
    /// when `exact` is false, for patterns with a rest binding).
    PatternTestList {
        dst: Register,
        value: Register,
        min_len: usize,
        exact: bool,
    },
    /// Whether a value is a tuple of exactly `len` elements.
    PatternTestTuple {
        dst: Register,
        value: Register,
        len: usize,
    },
    /// The `index`-th element of a list or tuple, for destructuring.
    ExtractElement {
        dst: Register,
        value: Register,
        index: usize,
    },
    /// The elements of a list from `from` onward, for `...rest` bindings.
    ExtractRest {
        dst: Register,
        value: Register,
        from: usize,
    },
    /// Construct an `Ok(value)` (or `Err(value)` when `ok` is false).
    MakeResult {
        dst: Register,
        value: Register,
        ok: bool,
    },
    /// No match arm applied to the scrutinee.
    MatchFail,

    // Range operations
    MakeRange {
        dst: Register,
        start: Register,
        end: Register,
        inclusive: bool,
    },

    // Tuple operations
    MakeTuple {
        dst: Register,
        elements: Vec<Register>,
    },

    /// Field access by name: `dst = object.<constants[name_const]>`. Reads a
    /// struct/object/module field, matching the interpreter.
    GetField {
        dst: Register,
        object: Register,
        name_const: u32,
        /// One-entry inline cache: shape id → field index. Interior-mutable
        /// because bytecode is shared (Arc) across calls and threads.
        cache: FieldCache,
    },
    /// Subscript: `dst = object[index]`. Lists, tuples, and strings with an
    /// integer index (negative counts from the end), matching the interpreter.
    IndexGet {
        dst: Register,
        object: Register,
        index: Register,
    },

    // Debug operations
    Nop,
}

/// Register identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Register(pub u32);

/// Label identifier for jumps
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Label(pub u32);

/// Debug information for bytecode
#[derive(Debug, Clone, Default)]
pub struct BytecodeDebugInfo {
    pub instruction_to_source: HashMap<usize, SourceLocation>,
    pub register_names: HashMap<Register, String>,
    pub function_name: Option<String>,
    pub local_variables: HashMap<u32, String>,
    pub line_table: Vec<(usize, u32)>, // (instruction_index, line_number)
}

/// Source location information
#[derive(Debug, Clone)]
pub struct SourceLocation {
    pub line: u32,
    pub column: u32,
    pub file: Option<String>,
}

/// VM execution state
#[derive(Debug)]
pub struct ExecutionState {
    /// One contiguous slab holding every live frame's registers. A call is
    /// a window `[base, top)` into it: entering a function bumps the window
    /// past the caller's, returning restores two integers. The slab only
    /// grows — stale values above the logical top are reset (drop-skipping
    /// immediates) when the next call claims them, which is the same
    /// recycle-in-place the old per-call frame pool did, minus the pool,
    /// the ExecutionState swap, and the per-call Vec bookkeeping.
    stack: Vec<OvmValue>,
    /// Start of the current frame's register window.
    base: usize,
    /// End of the current frame's register window (== base + register_count).
    top: usize,
}

/// VM performance statistics
#[derive(Debug, Default)]
pub struct VmStatistics {
    pub instructions_executed: u64,
    pub function_calls: u64,
    pub bytecode_cache_hits: u64,
    pub bytecode_cache_misses: u64,
    pub compilation_time: std::time::Duration,
    pub execution_time: std::time::Duration,
    pub optimization_time: std::time::Duration,
    pub memory_allocations: u64,
    pub gc_triggers: u64,
    /// Loops entered natively mid-frame (Campaign 7, T3).
    pub osr_entries: u64,
}

/// Register allocator for bytecode generation
pub struct RegisterAllocator {
    next_register: u32,
    free_registers: Vec<Register>,
    max_registers: u32,
}

/// Instruction emitter
pub struct InstructionEmitter {
    instructions: Vec<Instruction>,
    span_table: Vec<(u32, u32, u32)>,
    /// Label id -> instruction offset where the label was placed
    label_positions: HashMap<u32, usize>,
    next_label_id: u32,
    constants: Vec<OvmValue>,
    constant_map: HashMap<String, u32>, // For deduplication
    current_line: u32,
    debug_info: BytecodeDebugInfo,
}

/// VM errors
#[derive(Debug, Error)]
pub enum BytecodeError {
    #[error("Compilation failed: {0}")]
    CompilationFailed(String),

    /// A call to a function the VM doesn't know yet. Reported separately from
    /// CompilationFailed so a caller can compile the callee and retry rather
    /// than giving up on the whole function.
    #[error("Unresolved callee: {0}")]
    UnresolvedCallee(String),

    #[error("Runtime error: {0}")]
    RuntimeError(String),

    #[error("Invalid register: {0:?}")]
    InvalidRegister(Register),

    #[error("Invalid instruction at PC {pc}: {instruction:?}")]
    InvalidInstruction { pc: usize, instruction: Instruction },

    #[error("Stack overflow")]
    StackOverflow,

    #[error("Stack underflow")]
    StackUnderflow,

    #[error("Function not found: {0:?}")]
    FunctionNotFound(FunctionId),

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Invalid constant index: {0}")]
    InvalidConstantIndex(u32),

    #[error("Invalid local index: {0}")]
    InvalidLocalIndex(u32),

    #[error("Division by zero")]
    DivisionByZero,

    /// Kept distinct from DivisionByZero because the interpreter's `%`
    /// error says "Modulo by zero" — the tier must say the same words.
    #[error("Modulo by zero")]
    ModuloByZero,

    #[error("Index out of bounds: {index} for length {length}")]
    IndexOutOfBounds { index: i64, length: usize },

    #[error("Unresolved label: {0:?}")]
    UnresolvedLabel(Label),

    #[error("Exception thrown: {0}")]
    ExceptionThrown(String),

    #[error("Named function not found: {0}")]
    NamedFunctionNotFound(String),
}

/// VM exception types
#[derive(Debug, Clone)]
pub enum VmException {
    RuntimeError(String),
    TypeError(String),
    StackOverflow,
    DivisionByZero,
    IndexOutOfBounds,
    NullPointerException,
    InvalidOperation(String),
}

// Default implementations

impl BytecodeVm {
    pub fn new() -> Self {
        // Builtins whose calls the compiler will emit rather than refuse.
        //
        // The original rule was "value arguments and losslessly
        // representable results", which excluded the higher-order builtins
        // (a function argument could not reach the VM) and the map builders
        // (a Map did not survive the round trip). Both exclusions have since
        // been lifted — non-capturing lambdas compile to function values, and
        // maps round-trip — so `map`, `filter`, `reduce`, `fold`, and the map
        // builtins are all in the list below, each with a note at its group.
        // `group_by` is the one still held out, because it returns a Map keyed
        // by arbitrary values.
        let builtin_names: std::collections::HashSet<String> = [
            // conversion and inspection
            "to_string",
            "to_int",
            "to_float",
            "typeof",
            "len",
            // list access and construction
            "head",
            "tail",
            "cons",
            "concat",
            "reverse",
            "sort",
            "take",
            "skip",
            "flatten",
            "zip",
            "enumerate",
            "chunk",
            "range",
            // aggregation
            "sum",
            "min",
            "max",
            "average",
            "contains",
            // strings
            "split",
            "join",
            "starts_with",
            "ends_with",
            // results
            "is_ok",
            "is_err",
            "unwrap",
            "unwrap_or",
            // numeric
            "clamp",
            // pure `math` module functions — scalar in, scalar out, all
            // round-trippable. Reached via `math.sqrt(x)` (a module call),
            // which the compiler recognizes as a builtin below. This is what
            // lets a field-access-heavy numeric kernel (an N-body force loop,
            // say) actually promote instead of falling back on the first sqrt.
            "math.abs",
            "math.sqrt",
            "math.cbrt",
            "math.floor",
            "math.ceil",
            "math.round",
            "math.trunc",
            "math.sign",
            "math.fract",
            "math.pow",
            "math.exp",
            "math.exp2",
            "math.ln",
            "math.log",
            "math.log2",
            "math.log10",
            "math.sin",
            "math.cos",
            "math.tan",
            "math.asin",
            "math.acos",
            "math.atan",
            "math.atan2",
            "math.sinh",
            "math.cosh",
            "math.tanh",
            "math.degrees",
            "math.radians",
            "math.gcd",
            "math.lcm",
            "math.factorial",
            "math.min",
            "math.max",
            // pure `str` module functions — strings and lists of strings in
            // and out (parse_int/parse_float return Results), all
            // round-trippable, reached as `str.length(s)` module calls like
            // math.*. These are what string-heavy code (parsers, regex
            // engines, template renderers) lives on.
            "str.to_upper",
            "str.to_lower",
            "str.trim",
            "str.trim_start",
            "str.trim_end",
            "str.reverse",
            "str.chars",
            "str.lines",
            "str.words",
            "str.capitalize",
            "str.is_empty",
            "str.length",
            "str.parse_int",
            "str.parse_float",
            "str.contains",
            "str.starts_with",
            "str.ends_with",
            "str.split",
            "str.index_of",
            "str.last_index_of",
            "str.repeat",
            "str.count",
            "str.char_at",
            "str.replace",
            "str.replace_first",
            "str.substring",
            "str.pad_start",
            "str.pad_end",
            "str.join",
            "str.fmt",
            // map builtins — maps round-trip losslessly now, so builders
            // and readers both bridge safely
            "entries",
            // Campaign 6 mutation primitives: named here so the compiler
            // recognizes the real `col.set`/`col.swap` behind the
            // assignment fusion (and compiles plain calls to CallNamed).
            "col.set",
            "col.swap",
            "col.filled",
            "map_get",
            "map_set",
            "map_remove",
            "map_keys",
            "map_values",
            "map_len",
            "map_merge",
            "map_clear",
            "map_has_key",
            "group_by",
            // stringification (same bridge as to_string)
            "show",
            // output
            "print",
            "println",
            // higher-order: reachable now that non-capturing lambdas compile
            // to function values. group_by is still excluded because it
            // returns a Map, which does not survive the round trip.
            "map",
            "filter",
            "reduce",
            "fold",
            "find",
            "map_filtered",
            "result_map",
            "result_map_err",
            "unwrap_or_else",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        Self {
            compiler: BytecodeCompiler::new(),
            bytecode_cache: Arc::new(RwLock::new(HashMap::new())),
            bytecode_hot: Vec::new(),
            execution_state: ExecutionState::new(),
            stats: VmStatistics::default(),
            function_registry: HashMap::new(),
            builtin_names,
            builtins: BuiltinFunctions::new(),
            builtin_interpreter: None,
            caps: None,
            caps_trace: None,
            hof_promotions: 0,
            caps_file_stack: Vec::new(),
            call_depth: 0,
            arg_pool: Vec::new(),
            hof_cache: HashMap::new(),
            struct_defs: HashMap::new(),
            enum_type_names: std::collections::HashSet::new(),
            struct_field_checks: HashMap::new(),
            unit_variant_names: std::collections::HashSet::new(),
            known_function_values: HashMap::new(),
            ambiguous_function_names: std::collections::HashSet::new(),
            trait_impls: HashMap::new(),
            trait_defaults: HashMap::new(),
            type_traits: HashMap::new(),
            poisoned_structs: std::collections::HashSet::new(),
            // Must match the interpreter's own limit: a program that recurses
            // 900 deep has to behave the same whether or not it was promoted
            max_call_depth: crate::interpreter::DEFAULT_MAX_CALL_DEPTH as u32,
            #[cfg(feature = "native")]
            jit: crate::ovm::jit::JitCache::new(),
            #[cfg(feature = "native")]
            osr_regions: HashMap::new(),
            error_trace_span: None,
            error_trace_frames: Vec::new(),
            error_trace_leak: None,
        }
    }

    /// Register a function for dynamic calls
    /// Record a struct declaration's field-name set. A redeclaration with
    /// a different set poisons the type: baked compile-time validation
    /// cannot follow a live registry, so literals of that type refuse from
    /// then on. Returns true when the shape landscape changed in a way that
    /// invalidates previously compiled functions.
    pub fn note_struct(
        &mut self,
        name: String,
        fields: Vec<String>,
        field_checks: HashMap<String, crate::ast::FieldTypeCheck>,
    ) -> bool {
        self.builtin_interpreter = None;
        if self.poisoned_structs.contains(&name) {
            return false;
        }
        match self.struct_defs.get(&name) {
            Some(existing) if *existing == fields => false,
            Some(_) => {
                self.struct_defs.remove(&name);
                self.struct_field_checks.remove(&name);
                self.poisoned_structs.insert(name);
                true
            }
            None => {
                self.struct_field_checks.insert(name.clone(), field_checks);
                self.struct_defs.insert(name, fields);
                false
            }
        }
    }

    /// Record a user function's VALUE for lambda-closure attachment.
    /// A name rebound to a different body is marked ambiguous — the
    /// bridge must not resolve it bare.
    pub fn note_function_value(&mut self, name: String, func: crate::ast::Function) {
        if let Some(existing) = self.known_function_values.get(&name)
            && !std::sync::Arc::ptr_eq(&existing.body, &func.body)
        {
            self.ambiguous_function_names.insert(name.clone());
        }
        self.known_function_values.insert(name, func);
    }

    /// Record an `impl Trait for Type` method. Returns true when the
    /// dispatch landscape changed (new method, or an existing key rebound
    /// to a different body) — compiled functions may hold stale
    /// resolutions and must recompile.
    pub fn note_trait_impl(
        &mut self,
        type_name: String,
        method: String,
        func: crate::ast::Function,
    ) -> bool {
        // The bridge interpreter snapshots these tables at creation; a change
        // invalidates that snapshot.
        self.builtin_interpreter = None;
        match self.trait_impls.insert((type_name, method), func.clone()) {
            Some(old) => !std::sync::Arc::ptr_eq(&old.body, &func.body),
            None => true,
        }
    }

    /// Record a trait's default method. Same change-tracking as
    /// note_trait_impl.
    pub fn note_trait_default(
        &mut self,
        trait_name: String,
        method: String,
        func: crate::ast::Function,
    ) -> bool {
        self.builtin_interpreter = None;
        match self
            .trait_defaults
            .insert((trait_name, method), func.clone())
        {
            Some(old) => !std::sync::Arc::ptr_eq(&old.body, &func.body),
            None => true,
        }
    }

    /// Record that a type implements a trait (registration order matters:
    /// default lookup scans it in order, as the interpreter does).
    pub fn note_type_trait(&mut self, type_name: String, trait_name: String) -> bool {
        self.builtin_interpreter = None;
        let traits = self.type_traits.entry(type_name).or_default();
        if traits.contains(&trait_name) {
            false
        } else {
            traits.push(trait_name);
            true
        }
    }

    /// `Interpreter::lookup_method`, mirrored: direct impl first, then the
    /// type's traits in registration order for a default.
    fn lookup_method(&self, type_name: &str, method: &str) -> Option<&crate::ast::Function> {
        if let Some(f) = self
            .trait_impls
            .get(&(type_name.to_string(), method.to_string()))
        {
            return Some(f);
        }
        if let Some(traits) = self.type_traits.get(type_name) {
            for trait_name in traits {
                if let Some(f) = self
                    .trait_defaults
                    .get(&(trait_name.clone(), method.to_string()))
                {
                    return Some(f);
                }
            }
        }
        None
    }

    /// `Value::type_name`, mirrored for the VM value model — dispatch keys
    /// on the receiver's runtime type name, so the two must agree exactly.
    fn ovm_type_name(value: &OvmValue) -> &str {
        use crate::ovm::value::ValueData;
        match &value.data {
            ValueData::Integer(_) => "Int",
            ValueData::Float(_) => "Float",
            ValueData::String(_) => "String",
            ValueData::Boolean(_) => "Bool",
            ValueData::List(_) | ValueData::AstList(_) => "List",
            ValueData::Tuple(_) => "Tuple",
            ValueData::Function(_) | ValueData::AstFunction(_) | ValueData::Closure(_) => {
                "Function"
            }
            ValueData::Builtin(_) => "Builtin",
            ValueData::Struct(s) => s.type_name(),
            ValueData::Range(_) => "Range",
            ValueData::Result(_) => "Result",
            ValueData::Unit => "Unit",
            ValueData::Enum(e) => &e.type_name,
            ValueData::Map(_) => "Map",
            // Never constructed by compiled code; a failed lookup falls to
            // the field-access error, which is what the interpreter's
            // generic path produces too.
            _ => "<internal>",
        }
    }

    /// Record a declared unit enum variant name. Returns true when the
    /// name is new — previously compiled functions may have compiled a
    /// bare-identifier pattern of this name as a binding, which is now an
    /// equality match, so they must recompile.
    pub fn note_unit_variant(&mut self, name: String) -> bool {
        self.builtin_interpreter = None;
        self.unit_variant_names.insert(name)
    }

    /// Mirror a declared enum's type name, so `is_declared_type` recognizes
    /// it and an annotation naming it is a real type, not an unknown one.
    pub fn note_enum_type(&mut self, name: String) -> bool {
        self.enum_type_names.insert(name)
    }

    /// A struct or enum the program declared. Shared by the four annotation
    /// enforcement sites to distinguish an unknown type from a mismatch.
    fn is_declared_type(&self, name: &str) -> bool {
        self.struct_defs.contains_key(name) || self.enum_type_names.contains(name)
    }

    /// Error text for a failed annotation check, identical to the
    /// interpreter's `annotation_error` so the two tiers never disagree.
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

    pub fn register_function(&mut self, name: String, func_id: FunctionId) {
        #[cfg(feature = "native")]
        self.jit.note_shadow(&name);
        self.function_registry.insert(name, func_id);
    }

    /// Note that `name` is a user-defined function, so it shadows any builtin
    /// of the same name — matching the interpreter, where an environment
    /// lookup finds the user's definition first.
    pub fn shadow_builtin(&mut self, name: &str) {
        self.builtin_names.remove(name);
        #[cfg(feature = "native")]
        self.jit.note_shadow(name);
    }

    /// Withdraw a registration.
    ///
    /// Names are registered before compilation so recursive calls resolve; if
    /// compilation then fails the name must be withdrawn, or a later function
    /// will compile a call against an id that has no bytecode and fail at
    /// runtime with FunctionNotFound.
    pub fn unregister_function(&mut self, name: &str) {
        self.function_registry.remove(name);
    }

    /// Check if function has compiled bytecode
    pub fn has_bytecode(&self, func_id: FunctionId) -> bool {
        match self.bytecode_cache.read() {
            Ok(cache) => cache.contains_key(&func_id),
            _ => false,
        }
    }

    /// Compile function to bytecode with no enclosing closure. Lambdas in
    /// the body can then only reference their own parameters; the tier passes
    /// the real closure via compile_function_with_closure.
    pub fn compile_function(
        &mut self,
        func_id: FunctionId,
        func: &FunctionDecl,
    ) -> Result<(), BytecodeError> {
        let checks = crate::ast::param_checks_of(&func.parameters, &func.type_params);
        let ret = crate::ast::return_check_of(func.return_type.as_ref(), &func.type_params);
        self.compile_function_with_closure(
            func_id,
            func,
            std::sync::Arc::new(im::HashMap::new()),
            checks.into(),
            ret,
            None,
        )
    }

    /// Compile function to bytecode, with the function's declaration-time
    /// closure available for lambda eligibility and attachment.
    /// Install the run's capability table. The VM keeps it so the bridge
    /// interpreter — a separate `Interpreter` that dispatches builtins the
    /// VM cannot run natively — is seeded with it before every call, and
    /// so presents the same grant the interpreter tier would.
    /// Set the logical call-depth cap — kept identical to the
    /// interpreter's so the tiers raise the same error at the same
    /// depth.
    pub fn set_max_call_depth(&mut self, depth: u32) {
        self.max_call_depth = depth;
    }

    /// Seed the VM's frame counter with the interpreter frames already
    /// on the stack. The cap is one logical call depth shared by both
    /// tiers; without the seed, a recursion that promotes mid-descent
    /// would get a fresh budget on top of the frames it already spent —
    /// and a program near the cap would overflow on one tier and
    /// succeed on the other. Symmetric increments mean the counter is
    /// back at the seed when the entry call returns.
    pub fn set_depth_base(&mut self, base: u32) {
        self.call_depth = base;
    }

    /// Every name the VM can call by id — both compilation channels:
    /// tier promotion and a lambda's dependency resolution. The tier's
    /// own `compiled` map sees only the first, so the tier report reads
    /// this registry instead.
    pub(crate) fn registered_functions(&self) -> impl Iterator<Item = (&String, &FunctionId)> {
        self.function_registry.iter()
    }

    /// Warm start: the JIT view the tier persists (see ovm::warm).
    #[cfg(feature = "native")]
    pub(crate) fn jit_warm_view(
        &self,
        func_id: FunctionId,
    ) -> Option<(Vec<crate::ovm::jit::Kind>, u64)> {
        self.jit.warm_view(func_id)
    }

    #[cfg(not(feature = "native"))]
    pub(crate) fn jit_warm_view(&self, _func_id: FunctionId) -> Option<(Vec<()>, u64)> {
        None
    }

    /// Warm start: specialize a just-compiled function on the kinds a
    /// previous run recorded, ahead of its first call.
    #[cfg(feature = "native")]
    pub(crate) fn warm_specialize(&mut self, func_id: FunctionId, kinds: &[crate::ovm::jit::Kind]) {
        let Some(bytecode) = self.get_bytecode(func_id).ok() else {
            return;
        };
        let hot = &self.bytecode_hot;
        let cache = &self.bytecode_cache;
        let lookup = |id: FunctionId| -> Option<Arc<CompiledBytecode>> {
            hot.get(id.index())
                .and_then(|s| s.clone())
                .or_else(|| cache.read().ok().and_then(|c| c.get(&id).cloned()))
        };
        self.jit.warm_specialize(func_id, &bytecode, kinds, &lookup);
    }

    #[cfg(not(feature = "native"))]
    pub(crate) fn warm_specialize(&mut self, _func_id: FunctionId, _kinds: &[()]) {}

    pub fn set_capabilities(&mut self, caps: Option<Arc<crate::caps::CapTable>>) {
        self.caps = caps.clone();
        // The compiler folds capability queries and pre-grants gated
        // calls against this same table, so it must see every change.
        self.compiler.static_caps = caps;
        // The bridge caches its state; drop it so the next dispatch
        // rebuilds one that carries the table.
        self.builtin_interpreter = None;
    }

    /// Share the `--trace-caps` set, so a promoted function's effects land
    /// in the same profile as an interpreted one's.
    pub fn set_caps_trace(
        &mut self,
        trace: Arc<std::sync::Mutex<std::collections::BTreeSet<crate::caps::CapUse>>>,
    ) {
        self.caps_trace = Some(trace);
        self.builtin_interpreter = None;
    }

    pub fn compile_function_with_closure(
        &mut self,
        func_id: FunctionId,
        func: &FunctionDecl,
        closure: std::sync::Arc<im::HashMap<String, Value>>,
        param_checks: std::sync::Arc<[Option<crate::ast::FieldTypeCheck>]>,
        return_check: Option<crate::ast::FieldTypeCheck>,
        def_file: Option<Arc<str>>,
    ) -> Result<(), BytecodeError> {
        let start_time = crate::clock::Instant::now();

        // Set up registries so the compiler can validate callees
        self.compiler.function_registry = self.function_registry.clone();
        self.compiler.builtin_names = self.builtin_names.clone();
        self.compiler.struct_defs = self.struct_defs.clone();
        self.compiler.struct_field_checks = self.struct_field_checks.clone();
        self.compiler.pending_param_checks = param_checks;
        self.compiler.pending_return_check = return_check;
        self.compiler.pending_def_file = def_file;
        self.compiler.known_function_values = self.known_function_values.clone();
        self.compiler.unit_variant_names = self.unit_variant_names.clone();
        self.compiler.enclosing_closure = closure;

        self.compiler.pending_lambdas.clear();
        let bytecode = self.compiler.compile_function(func_id, func)?;

        // Compile the runtime-capture lambdas this function created, each
        // as a standalone function with the captures as trailing
        // parameters. Compiling one can queue more (nested lambdas); a
        // failure fails the whole compilation, because a MakeClosure must
        // never point at an id with no bytecode behind it.
        let mut rounds = 0;
        while !self.compiler.pending_lambdas.is_empty() {
            rounds += 1;
            if rounds > 32 {
                return Err(BytecodeError::CompilationFailed(
                    "lambda nesting too deep in the bytecode tier".to_string(),
                ));
            }
            let pending = std::mem::take(&mut self.compiler.pending_lambdas);
            for (lambda_id, decl, lambda_closure, self_binding) in pending {
                self.compiler.enclosing_closure = lambda_closure;
                // Each lambda enforces ITS OWN annotations, not the
                // enclosing function's. Without this reset the enclosing
                // param_checks were stamped onto the lambda and applied
                // positionally to [own params..., captures...] — a capture
                // landing on an annotated slot failed with the wrong type
                // ("parameter 'x' of <lambda> expects Int, got String").
                // Capture parameters get None checks (they are environment,
                // not caller arguments).
                self.compiler.pending_param_checks =
                    crate::ast::param_checks_of(&decl.parameters, &decl.type_params).into();
                self.compiler.pending_return_check =
                    crate::ast::return_check_of(decl.return_type.as_ref(), &decl.type_params);
                // A named nested fn binds its own name through the
                // self_call channel, which appends the body's own capture
                // parameters to recursive calls.
                self.compiler.self_call = self_binding.as_ref().map(|(name, captures)| {
                    (
                        name.clone(),
                        lambda_id,
                        decl.parameters.len() - captures,
                        *captures,
                    )
                });
                let compiled = self.compiler.compile_function(lambda_id, &decl);
                self.compiler.self_call = None;
                let lambda_bytecode = Arc::new(compiled?);
                let idx = lambda_id.index();
                self.mirror_hot(idx, &lambda_bytecode);
                #[cfg(feature = "native")]
                self.jit.try_compile(lambda_id, &lambda_bytecode);
                if let Ok(mut cache) = self.bytecode_cache.write() {
                    cache.insert(lambda_id, lambda_bytecode);
                }
            }
        }

        let bytecode = Arc::new(bytecode);
        let idx = func_id.index();
        self.mirror_hot(idx, &bytecode);
        #[cfg(feature = "native")]
        self.jit.try_compile(func_id, &bytecode);
        if let Ok(mut cache) = self.bytecode_cache.write() {
            cache.insert(func_id, bytecode);
        }

        self.stats.compilation_time += start_time.elapsed();
        Ok(())
    }

    /// The bytecode-hot mirror is capped: only ids below `HOT_CAP` are
    /// mirrored in the Vec, so a short-lived VM (a spawned worker) never
    /// sizes it to the global FunctionId high-water. Above the cap, the
    /// shared `bytecode_cache` HashMap serves the function — correct, just
    /// one hash per call for the rare high-id function.
    fn mirror_hot(&mut self, idx: usize, bytecode: &Arc<CompiledBytecode>) {
        const HOT_CAP: usize = 8192;
        if idx >= HOT_CAP {
            return;
        }
        if self.bytecode_hot.len() <= idx {
            self.bytecode_hot.resize(idx + 1, None);
        }
        self.bytecode_hot[idx] = Some(bytecode.clone());
    }

    /// Execute function with bytecode
    /// Fetch a function's compiled bytecode: the lock-free hot mirror first,
    /// the shared cache on miss (populating the mirror). Factored out so a
    /// hot loop that calls the same function per element (`map`/`filter`)
    /// can fetch the `Arc` once instead of on every call.
    fn get_bytecode(
        &mut self,
        func_id: FunctionId,
    ) -> Result<Arc<CompiledBytecode>, BytecodeError> {
        let idx = func_id.index();
        match self.bytecode_hot.get(idx).and_then(|slot| slot.as_ref()) {
            Some(b) => Ok(b.clone()),
            None => {
                let fetched = self
                    .bytecode_cache
                    .read()
                    .ok()
                    .and_then(|cache| cache.get(&func_id).cloned())
                    .ok_or(BytecodeError::FunctionNotFound(func_id))?;
                self.mirror_hot(idx, &fetched);
                Ok(fetched)
            }
        }
    }

    /// Run an already-fetched, already-validated function body — the tail of
    /// `execute` with the per-call preamble (bytecode fetch, arity check,
    /// parameter-type checks, JIT dispatch) hoisted to the caller. A hot
    /// loop that has verified those invariants once (they do not vary
    /// element to element) calls this per element instead of `execute`,
    /// paying only the frame push/pop and the body. The result is identical
    /// to `execute`; only the redundant preamble is skipped.
    fn execute_prepared(
        &mut self,
        bytecode: &Arc<CompiledBytecode>,
        args: &[OvmValue],
    ) -> Result<OvmValue, BytecodeError> {
        if self.call_depth >= self.max_call_depth {
            return Err(BytecodeError::RuntimeError(format!(
                "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                self.max_call_depth
            )));
        }
        self.call_depth += 1;
        let saved = self
            .execution_state
            .push_frame(bytecode.register_count as usize, args);
        self.stats.bytecode_cache_hits += 1;
        self.stats.function_calls += 1;
        self.push_caps_frame(bytecode);
        let result = self.execute_bytecode(bytecode);
        self.pop_caps_frame();
        self.execution_state.pop_frame(saved);
        self.call_depth -= 1;
        result
    }

    pub fn execute(
        &mut self,
        func_id: FunctionId,
        args: &[OvmValue],
    ) -> Result<OvmValue, BytecodeError> {
        // Fetch bytecode: index the lock-free mirror first, shared cache on miss
        let bytecode = self.get_bytecode(func_id)?;

        if args.len() != bytecode.param_count {
            // Word-for-word the interpreter's messages: a missing argument
            // names the first absent parameter; a surplus reports arity.
            return Err(BytecodeError::RuntimeError(
                if args.len() < bytecode.param_count {
                    format!(
                        "Missing required argument: {}",
                        bytecode.param_names[args.len()]
                    )
                } else {
                    format!(
                        "Arity mismatch: expected {}, got {}",
                        bytecode.param_count,
                        args.len()
                    )
                },
            ));
        }

        // Enforce declared parameter types — the same boundary, same
        // message, as the interpreter (annotations are promises on every
        // tier). All-None check lists skip in O(params).
        if !bytecode.param_checks.is_empty() {
            for (i, check) in bytecode.param_checks.iter().enumerate() {
                if let (Some(check), Some(arg)) = (check, args.get(i)) {
                    let (actual, payload, fn_arity, scalar) = ovm_value_view(arg);
                    if let Some((expected, got)) =
                        check.check_value(actual, payload, fn_arity, scalar)
                    {
                        let fn_name = bytecode
                            .debug_info
                            .function_name
                            .as_deref()
                            .unwrap_or("<fn>");
                        // The interpreter pushes the callee before this
                        // check and its early error return skips the pop —
                        // the frame stays visible. Mirror the leak.
                        self.error_trace_leak = bytecode.debug_info.function_name.clone();
                        return Err(BytecodeError::TypeError(self.annotation_error(
                            &format!("parameter '{}' of {}", bytecode.param_names[i], fn_name),
                            check,
                            &expected,
                            &got,
                        )));
                    }
                }
            }
        }

        // The cap gates the JIT attempt too: a base-case frame exactly at
        // the limit would otherwise run natively to completion without
        // ever meeting the depth check below, letting recursion finish
        // one frame past the cap on the native path alone.
        if self.call_depth >= self.max_call_depth {
            return Err(BytecodeError::RuntimeError(format!(
                "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                self.max_call_depth
            )));
        }
        // Native tier: if a JIT body exists (or can be specialized on this
        // call's argument kinds), run it. None means it declined (or
        // deopted) — the bytecode path below is the unchanged fallback.
        #[cfg(feature = "native")]
        if self.jit.has(func_id) {
            // The JIT spends this as frames *beyond* its entry frame, so
            // the entry itself is charged here — without the +1 a
            // recursion one past the cap completes instead of erroring.
            let remaining = self
                .max_call_depth
                .saturating_sub(self.call_depth + 1)
                .min(JIT_NATIVE_DEPTH_BUDGET);
            // Disjoint field borrows: the JIT plans call graphs through
            // this lookup while it holds &mut self.jit.
            let hot = &self.bytecode_hot;
            let cache = &self.bytecode_cache;
            let lookup = |id: FunctionId| -> Option<Arc<CompiledBytecode>> {
                hot.get(id.index())
                    .and_then(|s| s.clone())
                    .or_else(|| cache.read().ok().and_then(|c| c.get(&id).cloned()))
            };
            let profiled = crate::profile::push(
                bytecode
                    .debug_info
                    .function_name
                    .as_deref()
                    .unwrap_or("<anonymous>"),
                crate::profile::Tier::Native,
            );
            let native = self
                .jit
                .try_call(func_id, &bytecode, args, remaining, &lookup);
            if profiled {
                crate::profile::pop();
            }
            if let Some(result) = native {
                return Ok(result);
            }
        }

        if self.call_depth >= self.max_call_depth {
            return Err(BytecodeError::RuntimeError(format!(
                "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                self.max_call_depth
            )));
        }
        self.call_depth += 1;

        // The frame is a window on the shared register slab: entering bumps
        // past the caller's window, returning restores two integers.
        let saved = self
            .execution_state
            .push_frame(bytecode.register_count as usize, args);

        self.stats.bytecode_cache_hits += 1;
        self.stats.function_calls += 1;

        self.push_caps_frame(&bytecode);
        // Profiling shadow frame (`olang profile`): off, this is one
        // relaxed load and a predicted-false branch.
        let profiled = crate::profile::push(
            bytecode
                .debug_info
                .function_name
                .as_deref()
                .unwrap_or("<anonymous>"),
            crate::profile::Tier::Vm,
        );
        let result = self.execute_bytecode(&bytecode);
        if profiled {
            crate::profile::pop();
        }
        self.pop_caps_frame();

        // Restore the caller's window on both success and error paths
        self.execution_state.pop_frame(saved);
        self.call_depth -= 1;

        result
    }

    /// execute(), but the argument buffer is consumed: each value moves
    /// into the callee's frame, leaving Unit behind. This is the tier
    /// boundary's entry — a by-move list argument must arrive in its
    /// register *solely owned*, and the copy `execute` takes from its
    /// borrowed slice would pin the Arc for the whole call, turning
    /// every in-place write into a whole-list copy. The prologue
    /// (arity, annotations, the JIT attempt) reads by reference and is
    /// shared with `execute` verbatim.
    pub fn execute_taking(
        &mut self,
        func_id: FunctionId,
        args: &mut [OvmValue],
    ) -> Result<OvmValue, BytecodeError> {
        let bytecode = match self.execute_prologue(func_id, args)? {
            Ok(b) => b,
            Err(jit_result) => return Ok(jit_result),
        };
        if self.call_depth >= self.max_call_depth {
            return Err(BytecodeError::RuntimeError(format!(
                "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                self.max_call_depth
            )));
        }
        self.call_depth += 1;
        let saved = self
            .execution_state
            .push_frame_taking(bytecode.register_count as usize, args);
        if std::env::var_os("OLANG_DEBUG_ASTLIST").is_some() {
            for r in 0..bytecode.param_count {
                if let Ok(v) = self.execution_state.register_ref(Register(r as u32))
                    && let crate::ovm::value::ValueData::AstList(items) = &v.data
                {
                    eprintln!(
                        "[astlist] frame-entry reg{} rc={} len={}",
                        r,
                        std::sync::Arc::strong_count(items),
                        items.len()
                    );
                }
            }
        }
        self.stats.bytecode_cache_hits += 1;
        self.stats.function_calls += 1;
        self.push_caps_frame(&bytecode);
        // Profiling shadow frame (`olang profile`): off, this is one
        // relaxed load and a predicted-false branch.
        let profiled = crate::profile::push(
            bytecode
                .debug_info
                .function_name
                .as_deref()
                .unwrap_or("<anonymous>"),
            crate::profile::Tier::Vm,
        );
        let result = self.execute_bytecode(&bytecode);
        if profiled {
            crate::profile::pop();
        }
        self.pop_caps_frame();
        self.execution_state.pop_frame(saved);
        self.call_depth -= 1;
        result
    }

    /// The shared front half of `execute`/`execute_taking`: bytecode
    /// fetch, arity and annotation checks, and the JIT attempt — all by
    /// reference. `Err(value)` in the inner Result is a JIT result.
    #[allow(clippy::type_complexity)]
    fn execute_prologue(
        &mut self,
        func_id: FunctionId,
        args: &[OvmValue],
    ) -> Result<Result<Arc<CompiledBytecode>, OvmValue>, BytecodeError> {
        let bytecode = self.get_bytecode(func_id)?;
        if args.len() != bytecode.param_count {
            return Err(BytecodeError::RuntimeError(
                if args.len() < bytecode.param_count {
                    format!(
                        "Missing required argument: {}",
                        bytecode.param_names[args.len()]
                    )
                } else {
                    format!(
                        "Arity mismatch: expected {}, got {}",
                        bytecode.param_count,
                        args.len()
                    )
                },
            ));
        }
        if !bytecode.param_checks.is_empty() {
            for (i, check) in bytecode.param_checks.iter().enumerate() {
                if let (Some(check), Some(arg)) = (check, args.get(i)) {
                    let (actual, payload, fn_arity, scalar) = ovm_value_view(arg);
                    if let Some((expected, got)) =
                        check.check_value(actual, payload, fn_arity, scalar)
                    {
                        let fn_name = bytecode
                            .debug_info
                            .function_name
                            .as_deref()
                            .unwrap_or("<fn>");
                        self.error_trace_leak = bytecode.debug_info.function_name.clone();
                        return Err(BytecodeError::TypeError(self.annotation_error(
                            &format!("parameter '{}' of {}", bytecode.param_names[i], fn_name),
                            check,
                            &expected,
                            &got,
                        )));
                    }
                }
            }
        }
        // The cap gates the JIT attempt too: a base-case frame exactly at
        // the limit would otherwise run natively to completion without
        // ever meeting the depth check below, letting recursion finish
        // one frame past the cap on the native path alone.
        if self.call_depth >= self.max_call_depth {
            return Err(BytecodeError::RuntimeError(format!(
                "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                self.max_call_depth
            )));
        }
        #[cfg(feature = "native")]
        if self.jit.has(func_id) {
            // The JIT spends this as frames *beyond* its entry frame, so
            // the entry itself is charged here — without the +1 a
            // recursion one past the cap completes instead of erroring.
            let remaining = self
                .max_call_depth
                .saturating_sub(self.call_depth + 1)
                .min(JIT_NATIVE_DEPTH_BUDGET);
            let hot = &self.bytecode_hot;
            let cache = &self.bytecode_cache;
            let lookup = |id: FunctionId| -> Option<Arc<CompiledBytecode>> {
                hot.get(id.index())
                    .and_then(|s| s.clone())
                    .or_else(|| cache.read().ok().and_then(|c| c.get(&id).cloned()))
            };
            let profiled = crate::profile::push(
                bytecode
                    .debug_info
                    .function_name
                    .as_deref()
                    .unwrap_or("<anonymous>"),
                crate::profile::Tier::Native,
            );
            let native = self
                .jit
                .try_call(func_id, &bytecode, args, remaining, &lookup);
            if profiled {
                crate::profile::pop();
            }
            if let Some(result) = native {
                return Ok(Err(result));
            }
        }
        Ok(Ok(bytecode))
    }

    /// execute(), but the arguments come straight from the caller's
    /// registers into the callee's window — the CallFn/CallValue hot path,
    /// with no argument buffer in between.
    /// On-stack replacement (Campaign 7, T3): the current frame's hot
    /// loop, entered natively mid-frame. Called from the dispatch loop
    /// when a back-edge to `head` crosses the threshold. On success the
    /// loop has run to completion natively — the surviving registers are
    /// written back and the returned pc (the loop's exit) is where the
    /// VM resumes. On any failure the VM simply resumes at the head with
    /// its registers untouched: every instruction a region may contain
    /// is pure with respect to caller-visible state, so the failed
    /// attempt never happened, and a real runtime error re-raises on the
    /// VM with proper spans.
    #[cfg(feature = "native")]
    fn try_osr(&mut self, bytecode: &CompiledBytecode, head: usize) -> Option<usize> {
        use crate::ovm::jit::Kind as JitKind;
        if std::env::var_os("OLANG_OSR_OFF").is_some() {
            return None;
        }
        let fid = bytecode.function_id;
        if !self.osr_regions.contains_key(&fid) {
            let region = crate::ovm::osr::synthesize(bytecode, head).map(std::sync::Arc::new);
            if let Some(r) = &region {
                self.jit.try_compile(r.region_id, &r.synth);
            }
            self.osr_regions.insert(fid, region);
        }
        let region = self.osr_regions.get(&fid)?.clone()?;
        let osr_debug = std::env::var_os("OLANG_OSR_DEBUG").is_some();
        if region.head != head || region.live_in.len() > 16 {
            if osr_debug {
                eprintln!(
                    "[osr] fn#{}: hot head {} is not the region head {}",
                    fid.index(),
                    head,
                    region.head
                );
            }
            return None;
        }

        // Marshal the live-in registers exactly the way the call-boundary
        // raw entry marshals arguments (that block is the reference; the
        // kinds it cannot carry refuse here the same way).
        let mut bits = [0i64; 16];
        let mut kinds = [JitKind::Int; 16];
        let mut any_ref = false;
        // T2's by-move representation: a large list arrives as an AstList
        // handle, which native list ops don't read. Materialize the
        // OvmValue form once — the loop is about to iterate it natively
        // thousands of times — and keep the conversion alive alongside
        // the other retained list arguments for the duration of the call.
        let mut converted_lists: Vec<std::sync::Arc<Vec<crate::ovm::value::OvmValue>>> = Vec::new();
        for (i, reg) in region.live_in.iter().enumerate() {
            match self.execution_state.register_ref(*reg).map(|v| &v.data) {
                Ok(crate::ovm::value::ValueData::Integer(v)) => {
                    bits[i] = *v;
                    kinds[i] = JitKind::Int;
                }
                Ok(crate::ovm::value::ValueData::Boolean(b)) => {
                    bits[i] = *b as i64;
                    kinds[i] = JitKind::Bool;
                }
                Ok(crate::ovm::value::ValueData::AstList(items)) => {
                    let conv: std::sync::Arc<Vec<crate::ovm::value::OvmValue>> =
                        std::sync::Arc::new(
                            items
                                .iter()
                                .map(|v| crate::ovm::value::OvmValue::from_ast(v.clone()))
                                .collect(),
                        );
                    match crate::ovm::jit::classify_list(&conv) {
                        Some(k) => {
                            bits[i] = std::sync::Arc::as_ptr(&conv) as i64;
                            kinds[i] = k;
                            converted_lists.push(conv);
                            any_ref = true;
                        }
                        None => {
                            if osr_debug {
                                eprintln!(
                                    "[osr] fn#{}: live-in r{} is a list the JIT cannot classify",
                                    fid.index(),
                                    reg.0
                                );
                            }
                            return None;
                        }
                    }
                }
                Ok(crate::ovm::value::ValueData::Float(f)) => {
                    bits[i] = f.to_bits() as i64;
                    kinds[i] = JitKind::Float;
                }
                Ok(crate::ovm::value::ValueData::Struct(obj)) => {
                    bits[i] = std::sync::Arc::as_ptr(obj) as i64;
                    kinds[i] = JitKind::Struct(obj.shape.id);
                    any_ref = true;
                }
                Ok(crate::ovm::value::ValueData::String(st)) => {
                    bits[i] = std::sync::Arc::as_ptr(st) as i64;
                    kinds[i] = JitKind::Str;
                    any_ref = true;
                }
                Ok(crate::ovm::value::ValueData::List(items)) => {
                    let k = crate::ovm::jit::classify_list(items)?;
                    bits[i] = std::sync::Arc::as_ptr(items) as i64;
                    kinds[i] = k;
                    any_ref = true;
                }
                Ok(crate::ovm::value::ValueData::Result(r)) => {
                    let k = crate::ovm::jit::classify_result(r)?;
                    bits[i] = std::sync::Arc::as_ptr(r) as i64;
                    kinds[i] = k;
                    any_ref = true;
                }
                Ok(crate::ovm::value::ValueData::Map(m)) => {
                    let k = crate::ovm::jit::classify_map(m)?;
                    bits[i] = std::sync::Arc::as_ptr(m) as i64;
                    kinds[i] = k;
                    any_ref = true;
                }
                other => {
                    if osr_debug {
                        eprintln!(
                            "[osr] fn#{}: live-in r{} holds {}; cannot marshal",
                            fid.index(),
                            reg.0,
                            match other {
                                Ok(d) => format!("{:?}", std::mem::discriminant(d)),
                                Err(_) => "an invalid register".to_string(),
                            }
                        );
                    }
                    return None;
                }
            }
        }

        // This frame is already counted in call_depth; native code
        // continues it rather than entering a new one, so no extra entry
        // charge here — the budget covers the region's own callees.
        let remaining = self
            .max_call_depth
            .saturating_sub(self.call_depth)
            .min(JIT_NATIVE_DEPTH_BUDGET);
        let mut shapes = std::collections::HashMap::new();
        if self.jit.is_pending(region.region_id) {
            for reg in &region.live_in {
                let Ok(v) = self.execution_state.register_ref(*reg) else {
                    continue;
                };
                crate::ovm::jit::note_shapes(v, &mut shapes);
            }
        }
        let hot = &self.bytecode_hot;
        let cache = &self.bytecode_cache;
        let lookup = |id: FunctionId| -> Option<Arc<CompiledBytecode>> {
            hot.get(id.index())
                .and_then(|s| s.clone())
                .or_else(|| cache.read().ok().and_then(|c| c.get(&id).cloned()))
        };
        let mut struct_args: Vec<std::sync::Arc<crate::ovm::value::StructObject>> = Vec::new();
        let mut str_args: Vec<std::sync::Arc<String>> = Vec::new();
        let mut result_args: Vec<std::sync::Arc<crate::ovm::value::ResultObject>> = Vec::new();
        let mut list_args: Vec<std::sync::Arc<Vec<crate::ovm::value::OvmValue>>> = Vec::new();
        let mut map_args: Vec<
            std::sync::Arc<std::collections::HashMap<String, crate::ovm::value::OvmValue>>,
        > = Vec::new();
        list_args.extend(converted_lists.iter().cloned());
        if any_ref {
            for reg in &region.live_in {
                if let Ok(v) = self.execution_state.register_ref(*reg) {
                    if let crate::ovm::value::ValueData::Struct(obj) = &v.data {
                        struct_args.push(obj.clone());
                    }
                    if let crate::ovm::value::ValueData::String(st) = &v.data {
                        str_args.push(st.clone());
                    }
                    if let crate::ovm::value::ValueData::Result(r) = &v.data {
                        result_args.push(r.clone());
                    }
                    if let crate::ovm::value::ValueData::List(l) = &v.data {
                        list_args.push(l.clone());
                    }
                    if let crate::ovm::value::ValueData::Map(m) = &v.data {
                        map_args.push(m.clone());
                    }
                }
            }
        }
        let k = region.live_in.len();
        let result = self.jit.try_call_raw_with_shapes(
            region.region_id,
            &region.synth,
            &bits[..k],
            &kinds[..k],
            remaining,
            &lookup,
            &shapes,
            &struct_args,
            &str_args,
            &result_args,
            &list_args,
            &map_args,
        );
        let Some(result) = result else {
            // Refused, deopted, or errored: the loop stays on the VM for
            // good (a real error is about to re-raise there anyway).
            if osr_debug {
                eprintln!(
                    "[osr] fn#{} region fn#{} declined at entry; loop stays on the VM",
                    fid.index(),
                    region.region_id.index()
                );
            }
            self.osr_regions.insert(fid, None);
            return None;
        };
        // A tuple of live-outs must unmarshal faithfully — heap kinds in
        // tuple slots come back as reinterpreted numbers, so such a
        // region's (pure, unobserved) result is discarded once and the
        // loop stays on the VM.
        if region.live_out.len() > 1 && !self.jit.ready_tuple_ret_scalar(region.region_id) {
            if osr_debug {
                eprintln!(
                    "[osr] fn#{} region fn#{} returns heap kinds in a tuple; discarded",
                    fid.index(),
                    region.region_id.index()
                );
            }
            self.osr_regions.insert(fid, None);
            return None;
        }
        if region.live_out.len() == 1 {
            self.execution_state
                .set_register(region.live_out[0], result)
                .ok()?;
        } else {
            let crate::ovm::value::ValueData::Tuple(elems) = &result.data else {
                self.osr_regions.insert(fid, None);
                return None;
            };
            if elems.len() != region.live_out.len() {
                self.osr_regions.insert(fid, None);
                return None;
            }
            for (reg, v) in region.live_out.iter().zip(elems.iter()) {
                self.execution_state.set_register(*reg, v.clone()).ok()?;
            }
        }
        self.stats.osr_entries += 1;
        if osr_debug {
            eprintln!(
                "[osr] fn#{} loop entered natively; resuming at pc {}",
                fid.index(),
                region.exit_pc
            );
        }
        Some(region.exit_pc)
    }

    fn execute_from_regs(
        &mut self,
        func_id: FunctionId,
        arg_regs: &[Register],
        arg_moves: u64,
    ) -> Result<OvmValue, BytecodeError> {
        let idx = func_id.index();
        let bytecode = match self.bytecode_hot.get(idx).and_then(|slot| slot.as_ref()) {
            Some(b) => b.clone(),
            None => {
                let fetched = self
                    .bytecode_cache
                    .read()
                    .ok()
                    .and_then(|cache| cache.get(&func_id).cloned())
                    .ok_or_else(|| BytecodeError::FunctionNotFound(func_id))?;
                self.mirror_hot(idx, &fetched);
                fetched
            }
        };

        if arg_regs.len() != bytecode.param_count {
            // Word-for-word the interpreter's messages: a missing argument
            // names the first absent parameter; a surplus reports arity.
            return Err(BytecodeError::RuntimeError(
                if arg_regs.len() < bytecode.param_count {
                    format!(
                        "Missing required argument: {}",
                        bytecode.param_names[arg_regs.len()]
                    )
                } else {
                    format!(
                        "Arity mismatch: expected {}, got {}",
                        bytecode.param_count,
                        arg_regs.len()
                    )
                },
            ));
        }

        // Enforce declared parameter types — the same boundary, same
        // message, as the interpreter (annotations are promises on every
        // tier). All-None check lists skip in O(params).
        if !bytecode.param_checks.is_empty() {
            for (i, check) in bytecode.param_checks.iter().enumerate() {
                if let (Some(check), Some(arg)) = (check, arg_regs.get(i)) {
                    let value = self.execution_state.register_ref(*arg)?;
                    let (actual, payload, fn_arity, scalar) = ovm_value_view(value);
                    if let Some((expected, got)) =
                        check.check_value(actual, payload, fn_arity, scalar)
                    {
                        let fn_name = bytecode
                            .debug_info
                            .function_name
                            .as_deref()
                            .unwrap_or("<fn>");
                        // The interpreter pushes the callee before this
                        // check and its early error return skips the pop —
                        // the frame stays visible. Mirror the leak.
                        self.error_trace_leak = bytecode.debug_info.function_name.clone();
                        return Err(BytecodeError::TypeError(self.annotation_error(
                            &format!("parameter '{}' of {}", bytecode.param_names[i], fn_name),
                            check,
                            &expected,
                            &got,
                        )));
                    }
                }
            }
        }

        // Native tier: extract raw bits and kinds straight from the
        // caller's registers (only after the cheap has() check) and run
        // the JIT body. None → the unchanged bytecode path below.
        #[cfg(feature = "native")]
        if self.jit.has(func_id) && arg_regs.len() <= 16 {
            // Same cap gate as execute/execute_prologue: never attempt
            // native for a frame the depth check would refuse.
            if self.call_depth >= self.max_call_depth {
                return Err(BytecodeError::RuntimeError(format!(
                    "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                    self.max_call_depth
                )));
            }
            use crate::ovm::jit::Kind as JitKind;
            let mut bits = [0i64; 16];
            let mut kinds = [JitKind::Int; 16];
            let mut extractable = true;
            let mut any_ref = false;
            for (i, reg) in arg_regs.iter().enumerate() {
                match self.execution_state.register_ref(*reg).map(|v| &v.data) {
                    Ok(crate::ovm::value::ValueData::Integer(v)) => {
                        bits[i] = *v;
                        kinds[i] = JitKind::Int;
                    }
                    Ok(crate::ovm::value::ValueData::Float(f)) => {
                        bits[i] = f.to_bits() as i64;
                        kinds[i] = JitKind::Float;
                    }
                    Ok(crate::ovm::value::ValueData::Struct(obj)) => {
                        bits[i] = std::sync::Arc::as_ptr(obj) as i64;
                        kinds[i] = JitKind::Struct(obj.shape.id);
                        any_ref = true;
                    }
                    Ok(crate::ovm::value::ValueData::String(s)) => {
                        bits[i] = std::sync::Arc::as_ptr(s) as i64;
                        kinds[i] = JitKind::Str;
                        any_ref = true;
                    }
                    Ok(crate::ovm::value::ValueData::List(items)) => {
                        match crate::ovm::jit::classify_list(items) {
                            Some(k) => {
                                bits[i] = std::sync::Arc::as_ptr(items) as i64;
                                kinds[i] = k;
                                any_ref = true;
                            }
                            None => {
                                extractable = false;
                                break;
                            }
                        }
                    }
                    Ok(crate::ovm::value::ValueData::Result(r)) => {
                        match crate::ovm::jit::classify_result(r) {
                            Some(k) => {
                                bits[i] = std::sync::Arc::as_ptr(r) as i64;
                                kinds[i] = k;
                                any_ref = true;
                            }
                            None => {
                                extractable = false;
                                break;
                            }
                        }
                    }
                    Ok(crate::ovm::value::ValueData::Map(m)) => {
                        match crate::ovm::jit::classify_map(m) {
                            Some(k) => {
                                bits[i] = std::sync::Arc::as_ptr(m) as i64;
                                kinds[i] = k;
                                any_ref = true;
                            }
                            None => {
                                extractable = false;
                                break;
                            }
                        }
                    }
                    _ => {
                        extractable = false;
                        break;
                    }
                }
            }
            if extractable {
                // Entry frame charged here too — see the note above.
                let remaining = self
                    .max_call_depth
                    .saturating_sub(self.call_depth + 1)
                    .min(JIT_NATIVE_DEPTH_BUDGET);
                // Shape specs are only needed to specialize (first call).
                let mut shapes = std::collections::HashMap::new();
                if self.jit.is_pending(func_id) {
                    for reg in arg_regs {
                        let Ok(v) = self.execution_state.register_ref(*reg) else {
                            continue;
                        };
                        crate::ovm::jit::note_shapes(v, &mut shapes);
                    }
                }
                let hot = &self.bytecode_hot;
                let cache = &self.bytecode_cache;
                let lookup = |id: FunctionId| -> Option<Arc<CompiledBytecode>> {
                    hot.get(id.index())
                        .and_then(|s| s.clone())
                        .or_else(|| cache.read().ok().and_then(|c| c.get(&id).cloned()))
                };
                let mut struct_args: Vec<std::sync::Arc<crate::ovm::value::StructObject>> =
                    Vec::new();
                let mut str_args: Vec<std::sync::Arc<String>> = Vec::new();
                let mut result_args: Vec<std::sync::Arc<crate::ovm::value::ResultObject>> =
                    Vec::new();
                let mut list_args: Vec<std::sync::Arc<Vec<crate::ovm::value::OvmValue>>> =
                    Vec::new();
                let mut map_args: Vec<
                    std::sync::Arc<std::collections::HashMap<String, crate::ovm::value::OvmValue>>,
                > = Vec::new();
                // The per-family sweep only matters when a reference-kind
                // argument exists; all-scalar calls (the common boundary)
                // skip it whole.
                if any_ref {
                    for reg in arg_regs {
                        if let Ok(v) = self.execution_state.register_ref(*reg) {
                            if let crate::ovm::value::ValueData::Struct(obj) = &v.data {
                                struct_args.push(obj.clone());
                            }
                            if let crate::ovm::value::ValueData::String(s) = &v.data {
                                str_args.push(s.clone());
                            }
                            if let crate::ovm::value::ValueData::Result(r) = &v.data {
                                result_args.push(r.clone());
                            }
                            if let crate::ovm::value::ValueData::List(l) = &v.data {
                                list_args.push(l.clone());
                            }
                            if let crate::ovm::value::ValueData::Map(m) = &v.data {
                                map_args.push(m.clone());
                            }
                        }
                    }
                }
                let profiled = crate::profile::push(
                    bytecode
                        .debug_info
                        .function_name
                        .as_deref()
                        .unwrap_or("<anonymous>"),
                    crate::profile::Tier::Native,
                );
                let native = self.jit.try_call_raw_with_shapes(
                    func_id,
                    &bytecode,
                    &bits[..arg_regs.len()],
                    &kinds[..arg_regs.len()],
                    remaining,
                    &lookup,
                    &shapes,
                    &struct_args,
                    &str_args,
                    &result_args,
                    &list_args,
                    &map_args,
                );
                if profiled {
                    crate::profile::pop();
                }
                if let Some(result) = native {
                    return Ok(result);
                }
            }
        }

        if self.call_depth >= self.max_call_depth {
            return Err(BytecodeError::RuntimeError(format!(
                "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                self.max_call_depth
            )));
        }
        self.call_depth += 1;

        let saved = match self.execution_state.push_frame_from_regs(
            bytecode.register_count as usize,
            arg_regs,
            arg_moves,
        ) {
            Ok(saved) => saved,
            Err(e) => {
                self.call_depth -= 1;
                return Err(e);
            }
        };

        self.stats.bytecode_cache_hits += 1;
        self.stats.function_calls += 1;

        self.push_caps_frame(&bytecode);
        // Profiling shadow frame (`olang profile`): off, this is one
        // relaxed load and a predicted-false branch.
        let profiled = crate::profile::push(
            bytecode
                .debug_info
                .function_name
                .as_deref()
                .unwrap_or("<anonymous>"),
            crate::profile::Tier::Vm,
        );
        let result = self.execute_bytecode(&bytecode);
        if profiled {
            crate::profile::pop();
        }
        self.pop_caps_frame();

        self.execution_state.pop_frame(saved);
        self.call_depth -= 1;

        result
    }

    /// Bytecode instructions retired so far. The VM has always counted
    /// these; surfacing them makes the tier's actual workload visible
    /// (`--ovm-stats`), which is what a per-instruction cost is measured
    /// against.
    pub fn instructions_executed(&self) -> u64 {
        self.stats.instructions_executed
    }

    /// Calls that ran as native (JIT) code rather than bytecode. The
    /// counter makes the boundary policy observable: trivial constructors
    /// decline the boundary (see jit::boundary_unprofitable) and must
    /// leave this at zero when only they are called.
    pub fn jit_native_calls(&self) -> u64 {
        #[cfg(feature = "native")]
        {
            self.jit.native_calls
        }
        #[cfg(not(feature = "native"))]
        {
            0
        }
    }

    /// Execute bytecode instructions - Complete implementation
    fn execute_bytecode(&mut self, bytecode: &CompiledBytecode) -> Result<OvmValue, BytecodeError> {
        // Deep VM recursion rides the Rust stack one frame per user call;
        // the same segmented growth the interpreter uses keeps every
        // depth under the (shared) logical cap physically reachable.
        crate::interpreter::with_stack_headroom(|| self.execute_bytecode_inner(bytecode))
    }

    fn execute_bytecode_inner(
        &mut self,
        bytecode: &CompiledBytecode,
    ) -> Result<OvmValue, BytecodeError> {
        // Count instructions in a local and flush once: a stats-field write in
        // the dispatch loop costs a memory op per instruction executed.
        let mut executed: u64 = 0;
        let mut err_pc: usize = bytecode.entry_point;
        let result = self.dispatch_loop(bytecode, &mut executed, &mut err_pc);
        self.stats.instructions_executed += executed;
        if result.is_err() {
            self.note_error_frame(bytecode, err_pc);
        }
        result
    }

    /// One unwinding frame's contribution to the error trace. The first
    /// frame that owns a span for its failing pc sets the location and
    /// starts the visible stack; frames deeper than it stay invisible
    /// (the interpreter's model: those frames were already popped when
    /// the innermost Located statement captured).
    fn note_error_frame(&mut self, bytecode: &CompiledBytecode, err_pc: usize) {
        if self.error_trace_span.is_none() {
            let pc = err_pc as u32;
            match bytecode
                .span_table
                .iter()
                .rev()
                .find(|&&(start, _, _)| start <= pc)
            {
                Some(&(_, line, column)) => {
                    self.error_trace_span = Some((line, column));
                    if let Some(leaked) = self.error_trace_leak.take() {
                        self.error_trace_frames.push(leaked);
                    }
                }
                None => {
                    // No span here: this frame is invisible (the
                    // interpreter has already popped it) — but its pop
                    // consumes any leaked slot, so the leak becomes this
                    // frame instead.
                    if self.error_trace_leak.is_some() {
                        self.error_trace_leak = bytecode.debug_info.function_name.clone();
                    }
                    return;
                }
            }
        }
        if let Some(name) = &bytecode.debug_info.function_name {
            self.error_trace_frames.push(name.clone());
        }
    }

    /// Hand the finished trace to the tier boundary (resetting it). The
    /// frames come back innermost-first.
    pub fn take_error_trace(&mut self) -> (Option<(u32, u32)>, Vec<String>, Option<String>) {
        (
            self.error_trace_span.take(),
            std::mem::take(&mut self.error_trace_frames),
            self.error_trace_leak.take(),
        )
    }

    /// Reset any stale trace before a fresh top-level run (the tier may
    /// have consumed an error internally, e.g. an unresolved-callee
    /// compile-and-retry).
    pub fn clear_error_trace(&mut self) {
        self.error_trace_span = None;
        self.error_trace_frames.clear();
        self.error_trace_leak = None;
    }

    fn dispatch_loop(
        &mut self,
        bytecode: &CompiledBytecode,
        executed: &mut u64,
        err_pc: &mut usize,
    ) -> Result<OvmValue, BytecodeError> {
        let mut pc = bytecode.entry_point;
        // Backward jumps taken by THIS frame; at OSR_BACK_EDGE_THRESHOLD
        // the loop being spun is offered to on-stack replacement, once.
        #[cfg(feature = "native")]
        let mut back_edges: u64 = 0;

        while pc < bytecode.instructions.len() {
            let instruction = &bytecode.instructions[pc];
            *executed += 1;
            *err_pc = pc;

            match instruction {
                Instruction::LoadConst { dst, const_idx } => {
                    let value = bytecode
                        .constants
                        .get(*const_idx as usize)
                        .ok_or_else(|| BytecodeError::InvalidConstantIndex(*const_idx))?;
                    self.execution_state.set_register(*dst, value.clone())?;
                }

                // Locals live in registers; the compiler has not emitted
                // these since the register-window design. Reaching one means
                // hand-written or stale bytecode.
                Instruction::LoadLocal { .. } | Instruction::StoreLocal { .. } => {
                    return Err(BytecodeError::RuntimeError(
                        "LoadLocal/StoreLocal are not emitted by the compiler".to_string(),
                    ));
                }

                Instruction::TakeMove { dst, src } => {
                    let v = self.execution_state.take_register(*src)?;
                    self.execution_state.set_register(*dst, v)?;
                }
                Instruction::Move { dst, src } => {
                    let value = self.execution_state.get_register(*src)?;
                    self.execution_state.set_register(*dst, value)?;
                }

                // Arithmetic operations
                Instruction::Add { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::Add) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::Add)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::ListSetAssign {
                    target,
                    index,
                    value,
                } => {
                    use crate::ovm::value::ValueData;
                    let idx_val = self.execution_state.get_register(*index)?;
                    let idx = match &idx_val.data {
                        ValueData::Integer(i) => *i,
                        _ => {
                            return Err(BytecodeError::TypeError(format!(
                                "col.set: argument 2 must be an Int, got {}",
                                idx_val.type_name()
                            )));
                        }
                    };
                    let v = self.execution_state.get_register(*value)?;
                    // Move the list out of its register so a sole owner is
                    // recognizable; aliased lists copy, exactly like the
                    // interpreter's fusion and AddAssign's extend.
                    let target_val = self.execution_state.take_register(*target)?;
                    // A wrapped interpreter list writes in place too:
                    // one element converts, the arc stays shared with
                    // the interpreter side — the boundary-free write the
                    // collections' handles ride on.
                    if let ValueData::AstList(mut arc) = target_val.data {
                        if std::env::var_os("OLANG_DEBUG_ASTLIST").is_some()
                            && std::sync::Arc::strong_count(&arc) > 1
                        {
                            let mut holders = Vec::new();
                            for r in 0..64u32 {
                                if let Ok(v) = self.execution_state.register_ref(Register(r))
                                    && let ValueData::AstList(other) = &v.data
                                    && std::sync::Arc::ptr_eq(other, &arc)
                                {
                                    holders.push(r);
                                }
                            }
                            eprintln!(
                                "[astlist] set copy in {:?}: rc={} len={} frame_holders={:?} stack_holders={:?} base={}",
                                bytecode.debug_info.function_name.as_deref().unwrap_or("?"),
                                std::sync::Arc::strong_count(&arc),
                                arc.len(),
                                holders,
                                self.execution_state.debug_slots_holding(&arc),
                                self.execution_state.debug_base()
                            );
                        }
                        let at = crate::stdlib::collections::resolve_index("set", idx, arc.len())
                            .map_err(BytecodeError::RuntimeError)?;
                        let ast_v = v
                            .to_ast()
                            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
                        match std::sync::Arc::get_mut(&mut arc) {
                            Some(items) => items[at] = ast_v,
                            None => {
                                let mut items = (*arc).clone();
                                items[at] = ast_v;
                                arc = std::sync::Arc::new(items);
                            }
                        }
                        self.execution_state.set_register(
                            *target,
                            OvmValue {
                                data: ValueData::AstList(arc),
                            },
                        )?;
                        pc += 1;
                        continue;
                    }
                    if !matches!(target_val.data, ValueData::List(_)) {
                        let msg = format!(
                            "col.set: argument 1 must be a list, got {}",
                            target_val.type_name()
                        );
                        self.execution_state.set_register(*target, target_val)?;
                        return Err(BytecodeError::TypeError(msg));
                    }
                    let ValueData::List(mut arc) = target_val.data else {
                        unreachable!("matched above");
                    };
                    let at = crate::stdlib::collections::resolve_index("set", idx, arc.len())
                        .map_err(BytecodeError::RuntimeError)?;
                    match std::sync::Arc::get_mut(&mut arc) {
                        Some(items) => items[at] = v,
                        None => {
                            let mut items = (*arc).clone();
                            items[at] = v;
                            arc = std::sync::Arc::new(items);
                        }
                    }
                    self.execution_state.set_register(
                        *target,
                        OvmValue {
                            data: ValueData::List(arc),
                        },
                    )?;
                }
                Instruction::ListSwapAssign { target, i, j } => {
                    use crate::ovm::value::ValueData;
                    let read_idx = |vm: &Self, r: Register| -> Result<i64, BytecodeError> {
                        let v = vm.execution_state.get_register(r)?;
                        match &v.data {
                            ValueData::Integer(n) => Ok(*n),
                            _ => Err(BytecodeError::TypeError(format!(
                                "col.swap: index must be an Int, got {}",
                                v.type_name()
                            ))),
                        }
                    };
                    let ia = read_idx(self, *i)?;
                    let ib = read_idx(self, *j)?;
                    let target_val = self.execution_state.take_register(*target)?;
                    if let ValueData::AstList(mut arc) = target_val.data {
                        if std::env::var_os("OLANG_DEBUG_ASTLIST").is_some()
                            && std::sync::Arc::strong_count(&arc) > 1
                        {
                            let holders = self.execution_state.debug_slots_holding(&arc);
                            eprintln!(
                                "[astlist] swap rc={} len={} slab_holders={:?} base={}",
                                std::sync::Arc::strong_count(&arc),
                                arc.len(),
                                holders,
                                self.execution_state.debug_base()
                            );
                        }
                        let a = crate::stdlib::collections::resolve_index("swap", ia, arc.len())
                            .map_err(BytecodeError::RuntimeError)?;
                        let b = crate::stdlib::collections::resolve_index("swap", ib, arc.len())
                            .map_err(BytecodeError::RuntimeError)?;
                        match std::sync::Arc::get_mut(&mut arc) {
                            Some(items) => items.swap(a, b),
                            None => {
                                let mut items = (*arc).clone();
                                items.swap(a, b);
                                arc = std::sync::Arc::new(items);
                            }
                        }
                        self.execution_state.set_register(
                            *target,
                            OvmValue {
                                data: ValueData::AstList(arc),
                            },
                        )?;
                        pc += 1;
                        continue;
                    }
                    if !matches!(target_val.data, ValueData::List(_)) {
                        let msg = format!(
                            "col.swap: argument 1 must be a list, got {}",
                            target_val.type_name()
                        );
                        self.execution_state.set_register(*target, target_val)?;
                        return Err(BytecodeError::TypeError(msg));
                    }
                    let ValueData::List(mut arc) = target_val.data else {
                        unreachable!("matched above");
                    };
                    let a = crate::stdlib::collections::resolve_index("swap", ia, arc.len())
                        .map_err(BytecodeError::RuntimeError)?;
                    let b = crate::stdlib::collections::resolve_index("swap", ib, arc.len())
                        .map_err(BytecodeError::RuntimeError)?;
                    match std::sync::Arc::get_mut(&mut arc) {
                        Some(items) => items.swap(a, b),
                        None => {
                            let mut items = (*arc).clone();
                            items.swap(a, b);
                            arc = std::sync::Arc::new(items);
                        }
                    }
                    self.execution_state.set_register(
                        *target,
                        OvmValue {
                            data: ValueData::List(arc),
                        },
                    )?;
                }
                Instruction::AddAssign { target, rhs } => {
                    use crate::ovm::value::ValueData;
                    let rhs_val = self.execution_state.get_register(*rhs)?;
                    let target_val = self.execution_state.take_register(*target)?;
                    if let (ValueData::String(_), ValueData::String(b)) =
                        (&target_val.data, &rhs_val.data)
                    {
                        let ValueData::String(mut arc) = target_val.data else {
                            unreachable!("matched above");
                        };
                        match std::sync::Arc::get_mut(&mut arc) {
                            // Sole owner: append in place — O(1) amortized.
                            Some(s) => s.push_str(b),
                            // Aliased somewhere (another register, a constant,
                            // a value already sent elsewhere): copy, exactly
                            // like Add would.
                            None => {
                                let mut s = (*arc).clone();
                                s.push_str(b);
                                arc = std::sync::Arc::new(s);
                            }
                        }
                        self.execution_state.set_register(
                            *target,
                            OvmValue {
                                data: ValueData::String(arc),
                            },
                        )?;
                    } else if let (ValueData::AstList(_), ValueData::List(b)) =
                        (&target_val.data, &rhs_val.data)
                    {
                        // Extend the wrapped interpreter list in place:
                        // each appended element converts once.
                        let b = b.clone();
                        let ValueData::AstList(mut arc) = target_val.data else {
                            unreachable!("matched above");
                        };
                        let mut appended = Vec::with_capacity(b.len());
                        for v in b.iter() {
                            appended
                                .push(v.to_ast().map_err(|e| {
                                    BytecodeError::RuntimeError(format!("{:?}", e))
                                })?);
                        }
                        match std::sync::Arc::get_mut(&mut arc) {
                            Some(items) => items.extend(appended),
                            None => {
                                if std::env::var_os("OLANG_DEBUG_ASTLIST").is_some() {
                                    eprintln!(
                                        "[astlist] extend copy rc={} len={}",
                                        std::sync::Arc::strong_count(&arc),
                                        arc.len()
                                    );
                                }
                                let mut items = (*arc).clone();
                                items.extend(appended);
                                arc = std::sync::Arc::new(items);
                            }
                        }
                        self.execution_state.set_register(
                            *target,
                            OvmValue {
                                data: ValueData::AstList(arc),
                            },
                        )?;
                    } else if let (ValueData::List(_), ValueData::List(b)) =
                        (&target_val.data, &rhs_val.data)
                    {
                        // The same accumulate fusion for lists: `xs = xs + [v]`
                        // in a loop is O(n²) as a copy per iteration; when `xs`
                        // holds the only reference, extend in place for O(1)
                        // amortized. The aliasing guard is identical to the
                        // string case — `Arc::get_mut` returns None the moment
                        // another register, a constant, or a value sent
                        // elsewhere shares the Vec, and we copy exactly like
                        // Add would, so aliased accumulators stay correct.
                        let ValueData::List(mut arc) = target_val.data else {
                            unreachable!("matched above");
                        };
                        match std::sync::Arc::get_mut(&mut arc) {
                            Some(v) => v.extend(b.iter().cloned()),
                            None => {
                                let mut v = (*arc).clone();
                                v.extend(b.iter().cloned());
                                arc = std::sync::Arc::new(v);
                            }
                        }
                        self.execution_state.set_register(
                            *target,
                            OvmValue {
                                data: ValueData::List(arc),
                            },
                        )?;
                    } else {
                        let result = match Self::binary_fast(&target_val, &rhs_val, BinaryOp::Add) {
                            Some(v) => v,
                            None => self.execute_binary_op(&target_val, &rhs_val, BinaryOp::Add)?,
                        };
                        self.execution_state.set_register(*target, result)?;
                    }
                }

                Instruction::Sub { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::Subtract) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::Subtract)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Mul { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::Multiply) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::Multiply)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Div { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::Divide) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::Divide)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Mod { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::Modulo) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::Modulo)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Neg { dst, src } => {
                    let result = self.execute_unary_op(
                        self.execution_state.register_ref(*src)?,
                        UnaryOp::Negate,
                    )?;

                    self.execution_state.set_register(*dst, result)?;
                }

                // Comparison operations
                Instruction::Eq { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::Equal) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::Equal)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Ne { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::NotEqual) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::NotEqual)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Lt { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::LessThan) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::LessThan)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Le { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::LessThanEqual) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::LessThanEqual)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Gt { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::GreaterThan) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::GreaterThan)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Ge { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = match Self::binary_fast(left, right, BinaryOp::GreaterThanEqual) {
                        Some(v) => v,
                        None => self.execute_binary_op(left, right, BinaryOp::GreaterThanEqual)?,
                    };

                    self.execution_state.set_register(*dst, result)?;
                }

                // Logical operations
                Instruction::And { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_logical_and(left, right)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Or { dst, lhs, rhs } => {
                    let (left, right) = self.execution_state.register_pair(*lhs, *rhs)?;

                    let result = self.execute_logical_or(left, right)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::Not { dst, src } => {
                    let result = self
                        .execute_unary_op(self.execution_state.register_ref(*src)?, UnaryOp::Not)?;

                    self.execution_state.set_register(*dst, result)?;
                }

                // Control flow
                Instruction::Jump { target } => {
                    let t = target.0 as usize;
                    #[cfg(feature = "native")]
                    if t <= pc {
                        back_edges += 1;
                        if back_edges == OSR_BACK_EDGE_THRESHOLD
                            && let Some(resume) = self.try_osr(bytecode, t)
                        {
                            pc = resume;
                            continue;
                        }
                    }
                    pc = t;
                    continue;
                }

                Instruction::JumpIfTrue { condition, target } => {
                    if self.is_truthy(self.execution_state.register_ref(*condition)?) {
                        let t = target.0 as usize;
                        #[cfg(feature = "native")]
                        if t <= pc {
                            back_edges += 1;
                            if back_edges == OSR_BACK_EDGE_THRESHOLD
                                && let Some(resume) = self.try_osr(bytecode, t)
                            {
                                pc = resume;
                                continue;
                            }
                        }
                        pc = t;
                        continue;
                    }
                }

                Instruction::JumpIfFalse { condition, target } => {
                    if !self.is_truthy(self.execution_state.register_ref(*condition)?) {
                        let t = target.0 as usize;
                        #[cfg(feature = "native")]
                        if t <= pc {
                            back_edges += 1;
                            if back_edges == OSR_BACK_EDGE_THRESHOLD
                                && let Some(resume) = self.try_osr(bytecode, t)
                            {
                                pc = resume;
                                continue;
                            }
                        }
                        pc = t;
                        continue;
                    }
                }

                Instruction::TailCallSelf { args } => {
                    // Collect first: an argument register may be the very
                    // parameter register it is about to rebind.
                    let mut values = Vec::with_capacity(args.len());
                    for reg in args {
                        values.push(self.execution_state.get_register(*reg)?);
                    }
                    // The same boundary a real call would cross — the
                    // interpreter's trampoline re-checks its elided
                    // frames too, and annotations must not stop being
                    // promises at exactly the depth they matter most.
                    if !bytecode.param_checks.is_empty() {
                        for (i, check) in bytecode.param_checks.iter().enumerate() {
                            if let (Some(check), Some(arg)) = (check, values.get(i)) {
                                let (actual, payload, fn_arity, scalar) = ovm_value_view(arg);
                                if let Some((expected, got)) =
                                    check.check_value(actual, payload, fn_arity, scalar)
                                {
                                    let fn_name = bytecode
                                        .debug_info
                                        .function_name
                                        .as_deref()
                                        .unwrap_or("<fn>");
                                    return Err(BytecodeError::TypeError(self.annotation_error(
                                        &format!(
                                            "parameter '{}' of {}",
                                            bytecode.param_names[i], fn_name
                                        ),
                                        check,
                                        &expected,
                                        &got,
                                    )));
                                }
                            }
                        }
                    }
                    let arg_count = values.len();
                    for (i, value) in values.into_iter().enumerate() {
                        self.execution_state
                            .set_register(Register(i as u32), value)?;
                    }
                    // Registers past the parameters reset to Unit, exactly
                    // as push_frame leaves them for a real call.
                    for r in arg_count..bytecode.register_count as usize {
                        self.execution_state
                            .set_register(Register(r as u32), OvmValue::new_unit())?;
                    }
                    pc = bytecode.entry_point;
                    continue;
                }
                Instruction::Return { value } => {
                    let result = if let Some(reg) = value {
                        self.execution_state.get_register(*reg)?
                    } else {
                        OvmValue::new_unit()
                    };
                    // Enforce the declared return type, same message as the
                    // interpreter's boundary.
                    if let Some(check) = &bytecode.return_check {
                        let (actual, payload, fn_arity, scalar) = ovm_value_view(&result);
                        if let Some((expected, got)) =
                            check.check_value(actual, payload, fn_arity, scalar)
                        {
                            let fn_name = bytecode
                                .debug_info
                                .function_name
                                .as_deref()
                                .unwrap_or("<fn>");
                            return Err(BytecodeError::TypeError(self.annotation_error(
                                &format!("return value of {}", fn_name),
                                check,
                                &expected,
                                &got,
                            )));
                        }
                    }
                    return Ok(result);
                }

                // Function operations. The compiler only emits CallNamed;
                // a dynamic Call reaching the VM means a compilation bug, and
                // the old placeholder silently returned Unit for it.
                Instruction::Call { .. } => {
                    return Err(BytecodeError::RuntimeError(
                        "Dynamic function calls are not supported in the bytecode tier".to_string(),
                    ));
                }

                // Float-math fast path: id resolved at compile time, args read
                // as f64 with no AST conversion. Non-numeric arguments fall
                // back to the interpreter path so errors stay identical.
                Instruction::CallBuiltin {
                    dst,
                    builtin_id,
                    args,
                } => {
                    let id = *builtin_id as usize;
                    let (name, arity) = *Self::FLOAT_MATH.get(id).ok_or_else(|| {
                        BytecodeError::RuntimeError(format!(
                            "unknown builtin id {} in CallBuiltin",
                            builtin_id
                        ))
                    })?;
                    let mut nums = [0.0f64; 2];
                    let mut all_numeric = args.len() == arity && arity <= 2;
                    if all_numeric {
                        for (i, arg_reg) in args.iter().enumerate() {
                            match &self.execution_state.register_ref(*arg_reg)?.data {
                                crate::ovm::value::ValueData::Integer(n) => nums[i] = *n as f64,
                                crate::ovm::value::ValueData::Float(f) => nums[i] = *f,
                                _ => {
                                    all_numeric = false;
                                    break;
                                }
                            }
                        }
                    }
                    // The eval_float_math shortcut computes raw results and so
                    // cannot raise the domain errors the interpreter's math
                    // module does (sqrt of a negative, ln of a non-positive,
                    // asin/acos out of range). When an argument is out of
                    // domain, fall through to execute_builtin_call — the exact
                    // code the interpreter runs — so the error is byte-identical
                    // across tiers rather than a silent NaN.
                    let result = if all_numeric && !Self::float_math_out_of_domain(id, nums[0]) {
                        OvmValue::new_float(Self::eval_float_math(id, nums[0], nums[1]))
                    } else {
                        let mut arg_values = Vec::with_capacity(args.len());
                        for arg_reg in args {
                            arg_values.push(self.execution_state.get_register(*arg_reg)?);
                        }
                        self.execute_builtin_call(name, &arg_values, false)?
                    };
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::MakeStruct {
                    dst,
                    shape,
                    field_regs,
                    field_types,
                } => {
                    let mut values = Vec::with_capacity(field_regs.len());
                    for reg in field_regs {
                        values.push(self.execution_state.get_register(*reg)?);
                    }
                    // Enforce declared field types where the runtime can check
                    // them — the same rule and the same message the
                    // interpreter raises. `field_types` is in shape order, so
                    // it aligns with `values` and `shape.field_names`.
                    for (i, check) in field_types.iter().enumerate() {
                        if let Some(check) = check {
                            let (actual, payload, fn_arity, scalar) = ovm_value_view(&values[i]);
                            if let Some((expected, got)) =
                                check.check_value(actual, payload, fn_arity, scalar)
                            {
                                return Err(BytecodeError::TypeError(self.annotation_error(
                                    &format!(
                                        "field '{}' of {}",
                                        shape.field_names[i], shape.type_name
                                    ),
                                    check,
                                    &expected,
                                    &got,
                                )));
                            }
                        }
                    }
                    let obj = crate::ovm::value::StructObject {
                        shape: shape.clone(),
                        values,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::new_struct(Arc::new(obj)))?;
                }

                Instruction::MakeMap { dst, entries } => {
                    use crate::ovm::value::ValueData;
                    let mut map = std::collections::HashMap::with_capacity(entries.len());
                    for (key_reg, value_reg) in entries {
                        let key = match &self.execution_state.register_ref(*key_reg)?.data {
                            ValueData::String(s) => s.as_ref().clone(),
                            ValueData::Integer(i) => i.to_string(),
                            ValueData::Float(f) => crate::ast::format_float(*f),
                            ValueData::Boolean(b) => b.to_string(),
                            _ => {
                                return Err(BytecodeError::TypeError(
                                    "Map keys must be strings, integers, floats, or booleans"
                                        .to_string(),
                                ));
                            }
                        };
                        let value = self.execution_state.get_register(*value_reg)?;
                        map.insert(key, value);
                    }
                    self.execution_state
                        .set_register(*dst, OvmValue::new_map(Arc::new(map)))?;
                }

                Instruction::MakeTemplate { dst, parts } => {
                    use crate::ovm::value::ValueData;
                    let mut out = String::new();
                    for part in parts {
                        match part {
                            TplPart::Literal(text) => out.push_str(text),
                            TplPart::Reg(reg) => {
                                let value = self.execution_state.register_ref(*reg)?;
                                match &value.data {
                                    ValueData::String(s) => out.push_str(s),
                                    ValueData::Integer(n) => out.push_str(&n.to_string()),
                                    ValueData::Float(f) => {
                                        out.push_str(&crate::ast::format_float(*f))
                                    }
                                    ValueData::Boolean(b) => out.push_str(&b.to_string()),
                                    _ => {
                                        // The interpreter formats everything
                                        // else through Value's Display; go
                                        // through the same impl.
                                        let ast = value.to_ast().map_err(|e| {
                                            BytecodeError::RuntimeError(format!("{:?}", e))
                                        })?;
                                        out.push_str(&format!("{}", ast));
                                    }
                                }
                            }
                        }
                    }
                    self.execution_state
                        .set_register(*dst, OvmValue::new_string(out))?;
                }

                Instruction::CallMethod {
                    dst,
                    object,
                    method,
                    args,
                } => {
                    let receiver = self.execution_state.get_register(*object)?;
                    // Struct fields take precedence over methods, exactly as
                    // interpreted: field access that happens to hold a
                    // callable stays field access.
                    let field_callee = match &receiver.data {
                        crate::ovm::value::ValueData::Struct(st) => st.field(method).cloned(),
                        _ => None,
                    };
                    let result = if let Some(callee) = field_callee {
                        let mut arg_values = self.arg_pool.pop().unwrap_or_default();
                        arg_values.reserve(args.len());
                        for arg_reg in args {
                            arg_values.push(self.execution_state.get_register(*arg_reg)?);
                        }
                        let r = self.call_function_value(&callee, &arg_values);
                        arg_values.clear();
                        if self.arg_pool.len() < 64 {
                            self.arg_pool.push(arg_values);
                        }
                        r?
                    } else if let Some(m) =
                        self.lookup_method(Self::ovm_type_name(&receiver), method)
                    {
                        let m = OvmValue::new_ast_function(m.clone());
                        let mut arg_values = self.arg_pool.pop().unwrap_or_default();
                        arg_values.reserve(args.len() + 1);
                        arg_values.push(receiver);
                        for arg_reg in args {
                            arg_values.push(self.execution_state.get_register(*arg_reg)?);
                        }
                        let r = self.call_function_value(&m, &arg_values);
                        arg_values.clear();
                        if self.arg_pool.len() < 64 {
                            self.arg_pool.push(arg_values);
                        }
                        r?
                    } else {
                        // No field, no method: the interpreter's generic path
                        // evaluates the field access, which raises the
                        // missing-field / non-struct error.
                        return Err(match self.execute_get_field(&receiver, method) {
                            Err(e) => e,
                            Ok(_) => BytecodeError::RuntimeError(
                                "method dispatch reached an impossible state".to_string(),
                            ),
                        });
                    };
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::CallValue { dst, callee, args } => {
                    let mut arg_values = self.arg_pool.pop().unwrap_or_default();
                    arg_values.reserve(args.len());
                    for arg_reg in args {
                        arg_values.push(self.execution_state.get_register(*arg_reg)?);
                    }
                    let callee_value = self.execution_state.get_register(*callee)?;
                    let result = self.call_function_value(&callee_value, &arg_values);
                    arg_values.clear();
                    if self.arg_pool.len() < 64 {
                        self.arg_pool.push(arg_values);
                    }
                    self.execution_state.set_register(*dst, result?)?;
                }

                Instruction::BinImm {
                    op,
                    dst,
                    lhs,
                    imm,
                    swapped,
                } => {
                    let left = self.execution_state.register_ref(*lhs)?;
                    let result = match Self::binary_fast(left, imm, op.clone()) {
                        Some(v) => v,
                        None if *swapped => {
                            // The immediate was the source-left operand; run
                            // it in source order (un-flipping the op) so a
                            // type error reports operands as written.
                            self.execute_binary_op(imm, left, Self::flip_op(op))?
                        }
                        None => self.execute_binary_op(left, imm, op.clone())?,
                    };
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::MakeEnum {
                    dst,
                    type_name,
                    variant_name,
                    args,
                } => {
                    let mut values = Vec::with_capacity(args.len());
                    for reg in args {
                        values.push(self.execution_state.get_register(*reg)?);
                    }
                    let obj = crate::ovm::value::EnumObject {
                        type_name: type_name.clone(),
                        variant_name: variant_name.clone(),
                        data: crate::ovm::value::EnumData::Tuple(values),
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::new_enum(Arc::new(obj)))?;
                }

                Instruction::PatternTestEnum {
                    dst,
                    value,
                    variant_name,
                    pattern_count,
                } => {
                    use crate::ovm::value::{EnumData, ValueData};
                    let matches = match &self.execution_state.register_ref(*value)?.data {
                        ValueData::Enum(e) => {
                            e.variant_name == *variant_name
                                && match &e.data {
                                    EnumData::Unit => *pattern_count == 0,
                                    EnumData::Tuple(vs) => vs.len() == *pattern_count,
                                    EnumData::Struct(fs) => fs.len() == *pattern_count,
                                }
                        }
                        // Legacy: an enum pattern matches a plain tuple of the
                        // same length, variant name ignored (interpreter parity)
                        ValueData::Tuple(vs) => vs.len() == *pattern_count,
                        _ => false,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::new_boolean(matches))?;
                }

                Instruction::ExtractEnumPayload { dst, value, index } => {
                    use crate::ovm::value::{EnumData, ValueData};
                    let extracted = match &self.execution_state.register_ref(*value)?.data {
                        ValueData::Enum(e) => match &e.data {
                            EnumData::Tuple(vs) => vs.get(*index).cloned(),
                            EnumData::Struct(fs) => {
                                let mut ordered: Vec<&(String, OvmValue)> = fs.iter().collect();
                                ordered.sort_by(|a, b| a.0.cmp(&b.0));
                                ordered.get(*index).map(|(_, v)| v.clone())
                            }
                            EnumData::Unit => None,
                        },
                        ValueData::Tuple(vs) => vs.get(*index).cloned(),
                        _ => None,
                    };
                    let extracted = extracted.ok_or_else(|| {
                        BytecodeError::RuntimeError(
                            "enum payload extraction after a passing test cannot miss".to_string(),
                        )
                    })?;
                    self.execution_state.set_register(*dst, extracted)?;
                }

                Instruction::PatternTestStructField {
                    dst,
                    value,
                    field_name,
                } => {
                    use crate::ovm::value::ValueData;
                    let has = match &self.execution_state.register_ref(*value)?.data {
                        ValueData::Struct(s) => s.shape.field_index(field_name).is_some(),
                        _ => false,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::new_boolean(has))?;
                }

                Instruction::MakeClosure {
                    dst,
                    template_const,
                    captures,
                } => {
                    let template = match bytecode
                        .constants
                        .get(*template_const as usize)
                        .map(|c| &c.data)
                    {
                        Some(crate::ovm::value::ValueData::Closure(t)) => t.clone(),
                        _ => {
                            return Err(BytecodeError::RuntimeError(
                                "MakeClosure: template constant is not a closure".to_string(),
                            ));
                        }
                    };
                    let mut captured = Vec::with_capacity(captures.len());
                    for reg in captures {
                        captured.push(self.execution_state.get_register(*reg)?);
                    }
                    let closure = crate::ovm::value::ClosureObject {
                        template: template.template.clone(),
                        capture_names: template.capture_names.clone(),
                        captured,
                        func_id: template.func_id,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::new_closure(Arc::new(closure)))?;
                }

                Instruction::CallFn {
                    dst,
                    func_id,
                    args,
                    arg_moves,
                } => {
                    let result = self.execute_from_regs(*func_id, args, *arg_moves)?;
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::CallNamed {
                    dst,
                    function_name,
                    args,
                    pregranted,
                } => {
                    // Pool the argument buffer: a hot loop calling a builtin
                    // per element (`map(recs, r => map_get(r, k))`) would
                    // otherwise heap-allocate a fresh Vec on every call.
                    let mut arg_values = self.arg_pool.pop().unwrap_or_default();
                    arg_values.clear();
                    arg_values.reserve(args.len());
                    for arg_reg in args {
                        arg_values.push(self.execution_state.get_register(*arg_reg)?);
                    }

                    // Check if it's a builtin function first
                    // User functions first: a user definition shadows a
                    // builtin of the same name, as it does in the interpreter
                    let outcome = if let Some(&func_id) = self.function_registry.get(function_name)
                    {
                        self.execute(func_id, &arg_values)
                    } else if self.builtin_names.contains(function_name)
                        || function_name.contains('.')
                    {
                        // Module-prefixed names ("db.execute") were validated
                        // at compile time against the module's own field set;
                        // the bridge dispatches them by prefix exactly as the
                        // interpreter does.
                        self.execute_builtin_call(function_name, &arg_values, *pregranted)
                    } else {
                        Err(BytecodeError::NamedFunctionNotFound(function_name.clone()))
                    };
                    arg_values.clear();
                    if self.arg_pool.len() < 64 {
                        self.arg_pool.push(arg_values);
                    }
                    self.execution_state.set_register(*dst, outcome?)?;
                }

                // Collection operations
                Instruction::MakeList { dst, elements } => {
                    let mut list_values = Vec::with_capacity(elements.len());
                    for elem_reg in elements {
                        list_values.push(self.execution_state.get_register(*elem_reg)?);
                    }
                    self.execution_state
                        .set_register(*dst, OvmValue::new_list(list_values))?;
                }

                Instruction::MakeRange {
                    dst,
                    start,
                    end,
                    inclusive,
                } => {
                    let start_val = self.execution_state.get_register(*start)?;
                    let end_val = self.execution_state.get_register(*end)?;

                    // Extract integer values for the range
                    let start_int = match &start_val.data {
                        crate::ovm::value::ValueData::Integer(i) => *i,
                        _ => {
                            return Err(BytecodeError::RuntimeError(
                                "Range start must be integer".to_string(),
                            ));
                        }
                    };

                    let end_int = match &end_val.data {
                        crate::ovm::value::ValueData::Integer(i) => *i,
                        _ => {
                            return Err(BytecodeError::RuntimeError(
                                "Range end must be integer".to_string(),
                            ));
                        }
                    };

                    let range_value = Value::Range {
                        start: start_int,
                        end: end_int,
                        inclusive: *inclusive,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::from_ast(range_value))?;
                }

                Instruction::IterLen { dst, src } => {
                    let source = self.execution_state.register_ref(*src)?;
                    let len = Self::iter_len(source)?;
                    self.execution_state
                        .set_register(*dst, OvmValue::new_integer(len))?;
                }

                Instruction::IterGet { dst, src, idx } => {
                    let index = match &self.execution_state.register_ref(*idx)?.data {
                        crate::ovm::value::ValueData::Integer(i) => *i,
                        _ => {
                            return Err(BytecodeError::TypeError(
                                "Iteration index must be an integer".to_string(),
                            ));
                        }
                    };
                    let source = self.execution_state.register_ref(*src)?;
                    let value = Self::iter_get(source, index)?;
                    self.execution_state.set_register(*dst, value)?;
                }

                Instruction::PatternEq { dst, value, other } => {
                    let (a, b) = self.execution_state.register_pair(*value, *other)?;
                    let matches = Self::pattern_eq(a, b);
                    self.execution_state
                        .set_register(*dst, OvmValue::new_boolean(matches))?;
                }

                Instruction::PatternInRange {
                    dst,
                    value,
                    lo,
                    hi,
                    inclusive,
                } => {
                    let matches = match &self.execution_state.register_ref(*value)?.data {
                        crate::ovm::value::ValueData::Integer(n) => {
                            *n >= *lo && if *inclusive { *n <= *hi } else { *n < *hi }
                        }
                        // A range pattern only matches integers; anything else
                        // simply doesn't match
                        _ => false,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::new_boolean(matches))?;
                }

                Instruction::PatternTestResult {
                    dst,
                    value,
                    want_ok,
                } => {
                    use crate::ovm::value::ValueData;
                    let matches = match &self.execution_state.register_ref(*value)?.data {
                        ValueData::Result(r) => {
                            if *want_ok {
                                r.ok.is_some()
                            } else {
                                r.err.is_some()
                            }
                        }
                        _ => false,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::new_boolean(matches))?;
                }

                Instruction::ExtractResult {
                    dst,
                    value,
                    want_ok,
                } => {
                    use crate::ovm::value::ValueData;
                    let inner = match &self.execution_state.register_ref(*value)?.data {
                        ValueData::Result(r) => {
                            let side = if *want_ok { &r.ok } else { &r.err };
                            side.as_ref().map(|v| v.clone_simple())
                        }
                        _ => None,
                    };
                    match inner {
                        Some(v) => self.execution_state.set_register(*dst, v)?,
                        None => {
                            return Err(BytecodeError::RuntimeError(
                                "Result payload extraction on a non-matching value".to_string(),
                            ));
                        }
                    }
                }

                Instruction::PatternTestList {
                    dst,
                    value,
                    min_len,
                    exact,
                } => {
                    use crate::ovm::value::ValueData;
                    let matches = match &self.execution_state.register_ref(*value)?.data {
                        ValueData::AstList(items) => {
                            if *exact {
                                items.len() == *min_len
                            } else {
                                items.len() >= *min_len
                            }
                        }
                        ValueData::List(items) => {
                            if *exact {
                                items.len() == *min_len
                            } else {
                                items.len() >= *min_len
                            }
                        }
                        _ => false,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::new_boolean(matches))?;
                }

                Instruction::PatternTestTuple { dst, value, len } => {
                    use crate::ovm::value::ValueData;
                    let matches = match &self.execution_state.register_ref(*value)?.data {
                        ValueData::Tuple(items) => items.len() == *len,
                        _ => false,
                    };
                    self.execution_state
                        .set_register(*dst, OvmValue::new_boolean(matches))?;
                }

                Instruction::ExtractElement { dst, value, index } => {
                    use crate::ovm::value::ValueData;
                    let element = match &self.execution_state.register_ref(*value)?.data {
                        ValueData::List(items) | ValueData::Tuple(items) => {
                            items.get(*index).cloned()
                        }
                        ValueData::AstList(items) => {
                            items.get(*index).map(|v| OvmValue::from_ast(v.clone()))
                        }
                        _ => None,
                    };
                    match element {
                        Some(v) => self.execution_state.set_register(*dst, v)?,
                        None => {
                            return Err(BytecodeError::RuntimeError(
                                "Destructuring element out of bounds".to_string(),
                            ));
                        }
                    }
                }

                Instruction::ExtractRest { dst, value, from } => {
                    use crate::ovm::value::ValueData;
                    let rest = match &self.execution_state.register_ref(*value)?.data {
                        ValueData::List(items) => {
                            Some(items.iter().skip(*from).cloned().collect::<Vec<_>>())
                        }
                        ValueData::AstList(items) => Some(
                            items
                                .iter()
                                .skip(*from)
                                .map(|v| OvmValue::from_ast(v.clone()))
                                .collect::<Vec<_>>(),
                        ),
                        _ => None,
                    };
                    match rest {
                        Some(values) => self
                            .execution_state
                            .set_register(*dst, OvmValue::new_list(values))?,
                        None => {
                            return Err(BytecodeError::RuntimeError(
                                "Rest binding on a non-list value".to_string(),
                            ));
                        }
                    }
                }

                Instruction::MakeResult { dst, value, ok } => {
                    let inner = self.execution_state.get_register(*value)?;
                    let result = OvmValue::new_result(inner, *ok);
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::MatchFail => {
                    return Err(BytecodeError::RuntimeError(
                        "Pattern match failed".to_string(),
                    ));
                }

                // Tuple operations
                Instruction::MakeTuple { dst, elements } => {
                    let mut tuple_values = Vec::with_capacity(elements.len());
                    for elem_reg in elements {
                        tuple_values.push(self.execution_state.get_register(*elem_reg)?);
                    }
                    self.execution_state
                        .set_register(*dst, OvmValue::new_tuple(tuple_values))?;
                }

                // Debug operations
                Instruction::Nop => {
                    // No operation
                }

                Instruction::GetField {
                    dst,
                    object,
                    name_const,
                    cache,
                } => {
                    // Inline-cache fast path: same shape as last time means
                    // the field index is already known — an integer compare
                    // and an array read, no hashing. Misses take the cold
                    // outlined path, which refills the cache.
                    let object_value = self.execution_state.register_ref(*object)?;
                    let hit = match &object_value.data {
                        crate::ovm::value::ValueData::Struct(st) => {
                            let (cached_shape, cached_idx) = cache.load();
                            if cached_shape == st.shape.id {
                                st.values.get(cached_idx as usize).cloned()
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    let result = match hit {
                        Some(value) => value,
                        None => self.get_field_slow(bytecode, *name_const, object_value, cache)?,
                    };
                    self.execution_state.set_register(*dst, result)?;
                }

                Instruction::IndexGet { dst, object, index } => {
                    let (object_value, index_value) =
                        self.execution_state.register_pair(*object, *index)?;
                    let result = Self::execute_index_get(object_value, index_value)?;
                    self.execution_state.set_register(*dst, result)?;
                }
            }

            pc += 1;
        }

        // If we reach here without a return, return unit
        Ok(OvmValue::from_ast(Value::Unit))
    }

    /// Execute binary operation
    /// Numeric fast path for the dispatch handlers. Called with a constant
    /// `op` from each instruction arm, so inlining + constant propagation
    /// collapses the op match away and each arithmetic instruction compiles
    /// to a type check and the operation itself. Returns None (falling back
    /// to execute_binary_op, which owns the error messages) for anything
    /// but an in-range same-type numeric case: mixed Int/Float operands,
    /// integer overflow, division by zero, non-numeric types.
    #[inline(always)]
    fn binary_fast(left: &OvmValue, right: &OvmValue, op: BinaryOp) -> Option<OvmValue> {
        use crate::ovm::value::ValueData;
        Some(match (&left.data, &right.data) {
            (ValueData::Integer(a), ValueData::Integer(b)) => match op {
                BinaryOp::Add => OvmValue::new_integer(a.checked_add(*b)?),
                BinaryOp::Subtract => OvmValue::new_integer(a.checked_sub(*b)?),
                BinaryOp::Multiply => OvmValue::new_integer(a.checked_mul(*b)?),
                BinaryOp::Equal => OvmValue::new_boolean(a == b),
                BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
                BinaryOp::LessThan => OvmValue::new_boolean(a < b),
                BinaryOp::LessThanEqual => OvmValue::new_boolean(a <= b),
                BinaryOp::GreaterThan => OvmValue::new_boolean(a > b),
                BinaryOp::GreaterThanEqual => OvmValue::new_boolean(a >= b),
                _ => return None,
            },
            (ValueData::Float(a), ValueData::Float(b)) => match op {
                BinaryOp::Add => OvmValue::new_float(a + b),
                BinaryOp::Subtract => OvmValue::new_float(a - b),
                BinaryOp::Multiply => OvmValue::new_float(a * b),
                BinaryOp::Divide if *b != 0.0 => OvmValue::new_float(a / b),
                BinaryOp::Modulo if *b != 0.0 => OvmValue::new_float(a % b),
                BinaryOp::Equal => OvmValue::new_boolean(a == b),
                BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
                BinaryOp::LessThan => OvmValue::new_boolean(a < b),
                BinaryOp::LessThanEqual => OvmValue::new_boolean(a <= b),
                BinaryOp::GreaterThan => OvmValue::new_boolean(a > b),
                BinaryOp::GreaterThanEqual => OvmValue::new_boolean(a >= b),
                _ => return None,
            },
            _ => return None,
        })
    }

    /// Invert a comparison's direction (its own inverse); commutative ops
    /// map to themselves. Only ever called for ops the compiler's
    /// immediate-flip emits, all of which are in this set.
    fn flip_op(op: &BinaryOp) -> BinaryOp {
        match op {
            BinaryOp::LessThan => BinaryOp::GreaterThan,
            BinaryOp::LessThanEqual => BinaryOp::GreaterThanEqual,
            BinaryOp::GreaterThan => BinaryOp::LessThan,
            BinaryOp::GreaterThanEqual => BinaryOp::LessThanEqual,
            other => other.clone(),
        }
    }

    fn execute_binary_op(
        &self,
        left: &OvmValue,
        right: &OvmValue,
        op: BinaryOp,
    ) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;

        // Native extension values: offer the op to the owning OVM module —
        // the same hook the interpreter's eval_binary_op calls, on the same
        // Arc-shared value, so the tiers cannot diverge. to_ast on a Native
        // operand is an Arc clone, O(1).
        if matches!(left.data, ValueData::Native(_)) || matches!(right.data, ValueData::Native(_)) {
            let l = left
                .to_ast()
                .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
            let r = right
                .to_ast()
                .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
            if let Some(result) = crate::native::binary_op_hook(&op, &l, &r) {
                let value = result.map_err(BytecodeError::RuntimeError)?;
                return Ok(OvmValue::from_ast(value));
            }
        }

        // `x == ()` / `x != ()` is the presence test and is total (0.68),
        // mirroring the interpreter's rule in eval_binary_op: Unit is the
        // absence value, so equality against it answers for every value.
        // Ordering/arithmetic against Unit still falls through to the
        // type-error arms below, identically to the interpreter.
        if matches!(left.data, ValueData::Unit) || matches!(right.data, ValueData::Unit) {
            let both_unit =
                matches!(left.data, ValueData::Unit) && matches!(right.data, ValueData::Unit);
            match op {
                BinaryOp::Equal => return Ok(OvmValue::new_boolean(both_unit)),
                BinaryOp::NotEqual => return Ok(OvmValue::new_boolean(!both_unit)),
                _ => {}
            }
        }

        // Operate directly on ValueData — converting operands through the AST
        // representation on every instruction dominated the dispatch loop.
        let result = match (&left.data, &right.data) {
            (ValueData::Integer(a), ValueData::Integer(b)) => match op {
                BinaryOp::Add => OvmValue::new_integer(a.checked_add(*b).ok_or_else(|| {
                    BytecodeError::RuntimeError("Integer overflow in addition (bigint.of gives arbitrary precision)".to_string())
                })?),
                BinaryOp::Subtract => {
                    OvmValue::new_integer(a.checked_sub(*b).ok_or_else(|| {
                        BytecodeError::RuntimeError("Integer overflow in subtraction (bigint.of gives arbitrary precision)".to_string())
                    })?)
                }
                BinaryOp::Multiply => {
                    OvmValue::new_integer(a.checked_mul(*b).ok_or_else(|| {
                        BytecodeError::RuntimeError(
                            "Integer overflow in multiplication (bigint.of gives arbitrary precision)".to_string(),
                        )
                    })?)
                }
                BinaryOp::Divide => {
                    if *b == 0 {
                        return Err(BytecodeError::DivisionByZero);
                    }
                    // checked_div also rejects i64::MIN / -1, which overflows
                    OvmValue::new_integer(a.checked_div(*b).ok_or_else(|| {
                        BytecodeError::RuntimeError("Integer overflow in division (bigint.of gives arbitrary precision)".to_string())
                    })?)
                }
                BinaryOp::Modulo => {
                    if *b == 0 {
                        return Err(BytecodeError::ModuloByZero);
                    }
                    OvmValue::new_integer(a.checked_rem(*b).ok_or_else(|| {
                        BytecodeError::RuntimeError("Integer overflow in modulo (bigint.of gives arbitrary precision)".to_string())
                    })?)
                }
                BinaryOp::Equal => OvmValue::new_boolean(a == b),
                BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
                BinaryOp::LessThan => OvmValue::new_boolean(a < b),
                BinaryOp::LessThanEqual => OvmValue::new_boolean(a <= b),
                BinaryOp::GreaterThan => OvmValue::new_boolean(a > b),
                BinaryOp::GreaterThanEqual => OvmValue::new_boolean(a >= b),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Invalid binary operation: cannot apply '{}' to {} and {}",
                        op.symbol(),
                        left.type_name(),
                        right.type_name()
                    )));
                }
            },
            (ValueData::Float(a), ValueData::Float(b)) => self
                .execute_float_binary_op(*a, *b, op.clone())
                .map_err(|e| Self::name_operand_types(e, op, left, right))?,
            (ValueData::Integer(a), ValueData::Float(b)) => self
                .execute_float_binary_op(*a as f64, *b, op.clone())
                .map_err(|e| Self::name_operand_types(e, op, left, right))?,
            (ValueData::Float(a), ValueData::Integer(b)) => self
                .execute_float_binary_op(*a, *b as f64, op.clone())
                .map_err(|e| Self::name_operand_types(e, op, left, right))?,
            (ValueData::String(a), ValueData::String(b)) => match op {
                BinaryOp::Add => OvmValue::new_string(format!("{}{}", a, b)),
                BinaryOp::Equal => OvmValue::new_boolean(a == b),
                BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
                BinaryOp::LessThan => OvmValue::new_boolean(a < b),
                BinaryOp::LessThanEqual => OvmValue::new_boolean(a <= b),
                BinaryOp::GreaterThan => OvmValue::new_boolean(a > b),
                BinaryOp::GreaterThanEqual => OvmValue::new_boolean(a >= b),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Invalid binary operation: cannot apply '{}' to {} and {}",
                        op.symbol(),
                        left.type_name(),
                        right.type_name()
                    )));
                }
            },
            (ValueData::Boolean(a), ValueData::Boolean(b)) => match op {
                BinaryOp::Equal => OvmValue::new_boolean(a == b),
                BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
                BinaryOp::And => OvmValue::new_boolean(*a && *b),
                BinaryOp::Or => OvmValue::new_boolean(*a || *b),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Invalid binary operation: cannot apply '{}' to {} and {}",
                        op.symbol(),
                        left.type_name(),
                        right.type_name()
                    )));
                }
            },
            // Mixing a number and a string under `+` is a type error, not a
            // silent stringify (Python-3 style), mirroring the interpreter.
            // Concatenation stays string+string; convert with to_string(...).
            (ValueData::String(_), ValueData::Integer(_)) if matches!(op, BinaryOp::Add) => {
                return Err(BytecodeError::TypeError(
                    "cannot add String and Int; use to_string(...) to convert".to_string(),
                ));
            }
            (ValueData::Integer(_), ValueData::String(_)) if matches!(op, BinaryOp::Add) => {
                return Err(BytecodeError::TypeError(
                    "cannot add Int and String; use to_string(...) to convert".to_string(),
                ));
            }
            (ValueData::String(_), ValueData::Float(_)) if matches!(op, BinaryOp::Add) => {
                return Err(BytecodeError::TypeError(
                    "cannot add String and Float; use to_string(...) to convert".to_string(),
                ));
            }
            (ValueData::Float(_), ValueData::String(_)) if matches!(op, BinaryOp::Add) => {
                return Err(BytecodeError::TypeError(
                    "cannot add Float and String; use to_string(...) to convert".to_string(),
                ));
            }
            (ValueData::Enum(_), ValueData::Enum(_))
            | (ValueData::Struct(_), ValueData::Struct(_))
            | (ValueData::Map(_), ValueData::Map(_)) => match op {
                BinaryOp::Equal => OvmValue::new_boolean(Self::pattern_eq(left, right)),
                BinaryOp::NotEqual => OvmValue::new_boolean(!Self::pattern_eq(left, right)),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Invalid binary operation: cannot apply '{}' to {} and {}",
                        op.symbol(),
                        left.type_name(),
                        right.type_name()
                    )));
                }
            },
            (ValueData::List(a), ValueData::List(b)) => match op {
                BinaryOp::Add => {
                    let mut items = Vec::with_capacity(a.len() + b.len());
                    items.extend(a.iter().cloned());
                    items.extend(b.iter().cloned());
                    OvmValue::new_list(items)
                }
                // Deep structural equality, mirroring the interpreter's
                // List/List arms (Value's derived PartialEq).
                BinaryOp::Equal => OvmValue::new_boolean(Self::pattern_eq(left, right)),
                BinaryOp::NotEqual => OvmValue::new_boolean(!Self::pattern_eq(left, right)),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Invalid binary operation: cannot apply '{}' to {} and {}",
                        op.symbol(),
                        left.type_name(),
                        right.type_name()
                    )));
                }
            },
            // A wrapped list on either side: concatenation and equality
            // are O(n) operations, so a per-element view costs nothing
            // extra. Concatenation of two wrapped lists stays wrapped —
            // Value elements clone by Arc bump — and a mixed pair
            // converts the smaller representation into the other.
            (ValueData::AstList(a), ValueData::AstList(b)) => match op {
                BinaryOp::Add => {
                    let mut items = Vec::with_capacity(a.len() + b.len());
                    items.extend(a.iter().cloned());
                    items.extend(b.iter().cloned());
                    OvmValue {
                        data: ValueData::AstList(std::sync::Arc::new(items)),
                    }
                }
                BinaryOp::Equal => OvmValue::new_boolean(Self::pattern_eq(left, right)),
                BinaryOp::NotEqual => OvmValue::new_boolean(!Self::pattern_eq(left, right)),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Invalid binary operation: cannot apply '{}' to {} and {}",
                        op.symbol(),
                        left.type_name(),
                        right.type_name()
                    )));
                }
            },
            (ValueData::AstList(_), ValueData::List(_))
            | (ValueData::List(_), ValueData::AstList(_)) => match op {
                BinaryOp::Add => {
                    let to_items = |v: &OvmValue| -> Result<Vec<crate::ast::Value>, BytecodeError> {
                        match &v.data {
                            ValueData::AstList(items) => Ok((**items).clone()),
                            ValueData::List(items) => items
                                .iter()
                                .map(|x| {
                                    x.to_ast().map_err(|e| {
                                        BytecodeError::RuntimeError(format!("{:?}", e))
                                    })
                                })
                                .collect(),
                            _ => unreachable!("matched above"),
                        }
                    };
                    let mut items = to_items(left)?;
                    items.extend(to_items(right)?);
                    OvmValue {
                        data: ValueData::AstList(std::sync::Arc::new(items)),
                    }
                }
                BinaryOp::Equal => OvmValue::new_boolean(Self::pattern_eq(left, right)),
                BinaryOp::NotEqual => OvmValue::new_boolean(!Self::pattern_eq(left, right)),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Invalid binary operation: cannot apply '{}' to {} and {}",
                        op.symbol(),
                        left.type_name(),
                        right.type_name()
                    )));
                }
            },
            (ValueData::Tuple(_), ValueData::Tuple(_)) => match op {
                BinaryOp::Equal => OvmValue::new_boolean(Self::pattern_eq(left, right)),
                BinaryOp::NotEqual => OvmValue::new_boolean(!Self::pattern_eq(left, right)),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Invalid binary operation: cannot apply '{}' to {} and {}",
                        op.symbol(),
                        left.type_name(),
                        right.type_name()
                    )));
                }
            },
            (ValueData::Unit, ValueData::Unit) => match op {
                BinaryOp::Equal => OvmValue::new_boolean(true),
                BinaryOp::NotEqual => OvmValue::new_boolean(false),
                _ => {
                    return Err(BytecodeError::TypeError(format!(
                        "Invalid binary operation: cannot apply '{}' to {} and {}",
                        op.symbol(),
                        left.type_name(),
                        right.type_name()
                    )));
                }
            },
            _ => {
                return Err(BytecodeError::TypeError(format!(
                    "Invalid binary operation: cannot apply '{}' to {} and {}",
                    op.symbol(),
                    left.type_name(),
                    right.type_name()
                )));
            }
        };

        Ok(result)
    }

    /// Rewrite the float helper's typeless unsupported-op error into the
    /// interpreter's uniform message, naming the real operand types (so a
    /// mixed Int/Float pair reports "Int and Float", not "Float and Float").
    fn name_operand_types(
        e: BytecodeError,
        op: BinaryOp,
        left: &OvmValue,
        right: &OvmValue,
    ) -> BytecodeError {
        match e {
            BytecodeError::TypeError(msg) if msg.starts_with("Unsupported operation") => {
                BytecodeError::TypeError(format!(
                    "Invalid binary operation: cannot apply '{}' to {} and {}",
                    op.symbol(),
                    left.type_name(),
                    right.type_name()
                ))
            }
            other => other,
        }
    }

    /// Float arithmetic shared by the Float/Float and mixed Int/Float paths,
    /// mirroring the interpreter's coercion semantics
    fn execute_float_binary_op(
        &self,
        a: f64,
        b: f64,
        op: BinaryOp,
    ) -> Result<OvmValue, BytecodeError> {
        Ok(match op {
            BinaryOp::Add => OvmValue::new_float(a + b),
            BinaryOp::Subtract => OvmValue::new_float(a - b),
            BinaryOp::Multiply => OvmValue::new_float(a * b),
            BinaryOp::Divide => {
                if b == 0.0 {
                    return Err(BytecodeError::DivisionByZero);
                }
                OvmValue::new_float(a / b)
            }
            BinaryOp::Modulo => {
                if b == 0.0 {
                    return Err(BytecodeError::ModuloByZero);
                }
                OvmValue::new_float(a % b)
            }
            BinaryOp::Equal => OvmValue::new_boolean(a == b),
            BinaryOp::NotEqual => OvmValue::new_boolean(a != b),
            BinaryOp::LessThan => OvmValue::new_boolean(a < b),
            BinaryOp::LessThanEqual => OvmValue::new_boolean(a <= b),
            BinaryOp::GreaterThan => OvmValue::new_boolean(a > b),
            BinaryOp::GreaterThanEqual => OvmValue::new_boolean(a >= b),
            _ => {
                return Err(BytecodeError::TypeError(format!(
                    "Unsupported operation: {:?}",
                    op
                )));
            }
        })
    }

    /// Execute unary operation
    fn execute_unary_op(&self, value: &OvmValue, op: UnaryOp) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;

        match (&value.data, &op) {
            (ValueData::Integer(a), UnaryOp::Negate) => {
                Ok(OvmValue::new_integer(a.checked_neg().ok_or_else(|| {
                    BytecodeError::RuntimeError(
                        "Integer overflow in negation (bigint.of gives arbitrary precision)"
                            .to_string(),
                    )
                })?))
            }
            (ValueData::Float(a), UnaryOp::Negate) => Ok(OvmValue::new_float(-a)),
            (ValueData::Boolean(a), UnaryOp::Not) => Ok(OvmValue::new_boolean(!a)),
            (_, op) => Err(BytecodeError::TypeError(format!(
                "Invalid unary operation: cannot apply '{}' to {}",
                op.symbol(),
                value.type_name()
            ))),
        }
    }

    /// Execute logical AND operation
    fn execute_logical_and(
        &self,
        left: &OvmValue,
        right: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        // Strict Boolean semantics, matching the interpreter: non-boolean
        // operands are a type error, never coerced by truthiness. (Short-
        // circuiting happens in the compiled jump sequence, not here.)
        use crate::ovm::value::ValueData;
        match (&left.data, &right.data) {
            (ValueData::Boolean(a), ValueData::Boolean(b)) => Ok(OvmValue::new_boolean(*a && *b)),
            _ => Err(BytecodeError::TypeError(format!(
                "Invalid binary operation: cannot apply '&&' to {} and {}",
                left.type_name(),
                right.type_name()
            ))),
        }
    }

    /// Execute logical OR operation
    fn execute_logical_or(
        &self,
        left: &OvmValue,
        right: &OvmValue,
    ) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;
        match (&left.data, &right.data) {
            (ValueData::Boolean(a), ValueData::Boolean(b)) => Ok(OvmValue::new_boolean(*a || *b)),
            _ => Err(BytecodeError::TypeError(format!(
                "Invalid binary operation: cannot apply '||' to {} and {}",
                left.type_name(),
                right.type_name()
            ))),
        }
    }

    /// Execute function call
    /// Execute builtin function call
    /// Execute a builtin by delegating to the interpreter's implementation.
    ///
    /// Reimplementing builtins in the VM would be a second source of truth
    /// that could drift from the interpreter; delegating makes them identical
    /// by construction. The cost is a value round trip per call, which is
    /// dominated by the builtin's own work.
    /// Resolve a function *value* (a lambda constant or a function passed
    /// by value) to compiled bytecode, compiling it on first sight. The
    /// value's own attached closure is the compilation environment — the
    /// interpreter installs exactly that closure when calling it, so baked
    /// closure constants resolve identically. Returns None (caller bridges
    /// to the interpreter) for arity mismatch, default parameters, trait
    /// bounds, or an uncompilable body; failures are cached per body so
    /// they are not retried.
    pub(crate) fn hof_function_id(
        &mut self,
        func: &crate::ast::Function,
        arity: usize,
    ) -> Option<FunctionId> {
        if func.parameters.len() != arity
            || func.parameters.iter().any(|p| p.default_value.is_some())
            || !func.param_bounds.is_empty()
        {
            return None;
        }

        // Both halves of the identity: the body decides what the code is,
        // the captured environment decides what it closes over, and the
        // compiled artifact depends on both.
        let key = (
            Arc::as_ptr(&func.body) as usize,
            Arc::as_ptr(&func.closure) as usize,
        );
        if let Some((body_weak, env_weak, cached)) = self.hof_cache.get(&key)
            && let Some(live_body) = body_weak.upgrade()
            && Arc::ptr_eq(&live_body, &func.body)
            && let Some(live_env) = env_weak.upgrade()
            && Arc::ptr_eq(&live_env, &func.closure)
        {
            return *cached;
        }

        let decl = FunctionDecl {
            name_span: None,
            name: func
                .name
                .clone()
                .unwrap_or_else(|| "<function value>".to_string()),
            type_params: Vec::new(),
            type_param_bounds: Vec::new(),
            parameters: func.parameters.clone(),
            return_type: None,
            body: (*func.body).clone(),
        };
        let func_id = FunctionId::new();
        // The named tier path resolves unresolved callees (compile the
        // dependency, retry the caller) — the lambda path gets the same
        // loop here. This is what lets `map(xs, (n) => helper(n))`
        // compile when `helper` is a recursive top-level function the
        // registry has not seen yet: the callee compiles by name into
        // the registry, and the retry's registry lookup emits a direct
        // CallFn.
        let mut result = None;
        for _ in 0..8 {
            match self.compile_function_with_closure(
                func_id,
                &decl,
                func.closure.clone(),
                func.param_checks.clone().into(),
                func.return_check.clone(),
                func.def_file.as_deref().map(Arc::from),
            ) {
                Ok(()) => {
                    result = Some(func_id);
                    break;
                }
                Err(BytecodeError::UnresolvedCallee(callee)) => {
                    if !self.compile_hof_dependency(&callee, 0) {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if self.hof_cache.len() >= 512 {
            self.hof_cache.clear();
        }
        self.hof_cache.insert(
            key,
            (
                Arc::downgrade(&func.body),
                Arc::downgrade(&func.closure),
                result,
            ),
        );
        result
    }

    /// Call a function *value*: the fast paths run compiled bytecode (a
    /// plain function compiles on demand through the hof cache; a runtime
    /// closure already carries its func_id, captures appended after the
    /// arguments). Everything else — builtins, constructors, functions the
    /// compiler declines, non-callables — converts to AST and runs through
    /// the bridge interpreter, whose call_function owns the exact semantics
    /// of arity errors, default parameters, trait bounds, and the
    /// "Cannot call non-function value" error.
    /// Ensure the bridge interpreter exists and carries the program's
    /// declaration-level state. Builtins the VM cannot run natively (`fold`,
    /// and any function value declined by `call_function_value`) bridge to
    /// this interpreter; when they invoke a user lambda that dispatches a
    /// trait method or builds a declared struct, it must see the same
    /// trait/struct/variant tables the VM has accumulated — otherwise a
    /// promoted function's fold-lambda calling `x.tag()` raises a spurious
    /// "Field not found" that the tree-walk never would. Recreated (not
    /// mutated in place) whenever those tables change, which is why the
    /// `note_*` methods drop it.
    /// Enter a compiled function for capability purposes: remember whose
    /// file it came from. Mirrors the interpreter pushing `def_file` onto
    /// `coverage_file_stack` in `call_function`, and is what lets a
    /// promoted function in a dependency be gated by *that dependency's*
    /// grant rather than the program's.
    ///
    /// Costs a push and a pop per call when a manifest or a trace is
    /// active, and a single branch when neither is — an unrestricted run
    /// pays nothing.
    #[inline]
    fn push_caps_frame(&mut self, bytecode: &CompiledBytecode) {
        if self.caps.is_some() || self.caps_trace.is_some() {
            self.caps_file_stack.push(bytecode.def_file.clone());
        }
    }

    #[inline]
    fn pop_caps_frame(&mut self) {
        if self.caps.is_some() || self.caps_trace.is_some() {
            self.caps_file_stack.pop();
        }
    }

    fn ensure_bridge_interpreter(&mut self) {
        if self.builtin_interpreter.is_none() {
            let mut interp = Box::new(crate::interpreter::Interpreter::new());
            // Tier first: seed_bridge_state forwards the declaration
            // tables into an existing tier, so order matters here.
            if std::env::var_os("OLANG_BRIDGE_TIER_OFF").is_none() {
                interp.enable_bytecode_tier(1, false);
            }
            // The bridge must see the same function landscape the VM
            // does: a module's mutually recursive functions resolve
            // through the registry (a def-time closure cannot hold a
            // sibling defined later), and without this seeding the
            // bridge's tier compiled such a body against a world where
            // the sibling did not exist — and its tree-walk fallback
            // missed it too.
            // Only *unambiguous* names seed the environment: two
            // modules' private `insert`s must each resolve to their own,
            // which bare-name seeding cannot promise. The tier registry
            // gets every name — it carries its own ambiguity guard.
            for (name, func) in self.known_function_values.clone() {
                if !self.ambiguous_function_names.contains(&name) {
                    interp.define_global(&name, crate::ast::Value::Function(func.clone()));
                }
                if let Some(tier) = interp.bytecode_tier_mut() {
                    tier.note_function(name, func);
                }
            }
            interp.seed_bridge_state(
                self.trait_impls.clone(),
                self.trait_defaults.clone(),
                self.type_traits.clone(),
                self.struct_defs.clone(),
                self.struct_field_checks.clone(),
                self.unit_variant_names.clone(),
                // The bridge dispatches builtins; it never constructs a user
                // struct or checks a user function's parameter annotations,
                // so it never reaches the annotation enforcer and needs no
                // enum-type registry of its own.
                std::collections::HashSet::new(),
            );
            // The bridge is a full interpreter, and a full interpreter
            // has a compiled tier (enabled above, before the state
            // seeding): a function value the VM declines used to strand
            // everything it called on a tree-walk — the
            // callback-in-a-harness trap, measured at three orders of
            // magnitude. Its capabilities are (re)seeded per dispatch by
            // seed_bridge_caps, which forwards them into this tier.
            self.builtin_interpreter = Some(interp);
        }
    }

    fn call_function_value(
        &mut self,
        callee: &OvmValue,
        args: &[OvmValue],
    ) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;
        match &callee.data {
            ValueData::AstFunction(f) => {
                let f = f.clone();
                if let Some(func_id) = self.hof_function_id(&f, args.len()) {
                    return self.execute(func_id, args);
                }
            }
            ValueData::Closure(c) if c.template.parameters.len() == args.len() => {
                let c = c.clone();
                let mut full_args = Vec::with_capacity(args.len() + c.captured.len());
                full_args.extend(args.iter().cloned());
                full_args.extend(c.captured.iter().cloned());
                return self.execute(c.func_id, &full_args);
            }
            _ => {}
        }

        // Interpreter fallback: exact semantics for everything declined
        let callee_ast = callee
            .to_ast()
            .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?;
        let mut ast_args = Vec::with_capacity(args.len());
        for arg in args {
            ast_args.push(
                arg.to_ast()
                    .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?,
            );
        }
        self.ensure_bridge_interpreter();
        // The callee runs a whole user-function body in the bridge: it
        // must carry the run's capability grant (this path previously
        // seeded nothing — a declined function value ran ungated) and
        // the live call depth, so the shared budget holds across the
        // boundary.
        let caps = self.caps.clone();
        let trace = self.caps_trace.clone();
        let attributed_to = self
            .caps_file_stack
            .last()
            .cloned()
            .flatten()
            .map(|f| f.to_string());
        let depth = self.call_depth as usize;
        let interpreter = self.builtin_interpreter.as_mut().expect("just ensured");
        interpreter.seed_bridge_caps(caps, trace, attributed_to);
        interpreter.set_call_depth_base(depth);
        let result = interpreter
            .call_function(callee_ast, ast_args)
            .map_err(|e| BytecodeError::RuntimeError(e.to_string()))?;
        if !Self::round_trips(&result) {
            return Err(BytecodeError::RuntimeError(
                "function value returned a value the bytecode tier cannot represent".to_string(),
            ));
        }
        Ok(OvmValue::from_ast(result))
    }

    /// map_get with the interpreter's exact semantics and check order:
    /// receiver must be a map or struct-like first, then the key coerces
    /// (String raw, Int/Float/Bool via to_string); a missing key is Unit.
    fn native_map_get(
        receiver: &OvmValue,
        key: &OvmValue,
        who: &str,
    ) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;
        enum Recv<'a> {
            Map(&'a std::collections::HashMap<String, OvmValue>),
            Struct(&'a crate::ovm::value::StructObject),
        }
        let recv = match &receiver.data {
            ValueData::Map(m) => Recv::Map(m),
            ValueData::Struct(st) => Recv::Struct(st),
            _ => {
                return Err(BytecodeError::TypeError(format!(
                    "{}: first argument must be a map or object",
                    who
                )));
            }
        };
        let key_string;
        let key_str: &str = match &key.data {
            ValueData::String(st) => st,
            ValueData::Integer(i) => {
                key_string = i.to_string();
                &key_string
            }
            ValueData::Float(f) => {
                key_string = f.to_string();
                &key_string
            }
            ValueData::Boolean(b) => {
                key_string = b.to_string();
                &key_string
            }
            _ => {
                return Err(BytecodeError::TypeError(format!(
                    "{}: key must be string, integer, float, or boolean",
                    who
                )));
            }
        };
        Ok(match recv {
            Recv::Map(m) => m.get(key_str).cloned().unwrap_or_else(OvmValue::new_unit),
            Recv::Struct(st) => st
                .field(key_str)
                .cloned()
                .unwrap_or_else(OvmValue::new_unit),
        })
    }

    fn native_map_has_key(receiver: &OvmValue, key: &OvmValue) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;
        let contains: Box<dyn Fn(&str) -> bool> = match &receiver.data {
            ValueData::Map(m) => Box::new(move |k| m.contains_key(k)),
            ValueData::Struct(st) => Box::new(move |k| st.shape.field_index(k).is_some()),
            _ => {
                return Err(BytecodeError::TypeError(
                    "map_has_key: first argument must be a map or object".to_string(),
                ));
            }
        };
        let key_string;
        let key_str: &str = match &key.data {
            ValueData::String(st) => st,
            ValueData::Integer(i) => {
                key_string = i.to_string();
                &key_string
            }
            ValueData::Float(f) => {
                key_string = f.to_string();
                &key_string
            }
            ValueData::Boolean(b) => {
                key_string = b.to_string();
                &key_string
            }
            _ => {
                return Err(BytecodeError::TypeError(
                    "map_has_key: key must be string, integer, float, or boolean".to_string(),
                ));
            }
        };
        Ok(OvmValue::new_boolean(contains(key_str)))
    }

    /// Native execution for the higher-order builtins when the collection
    /// is a list and the function argument compiles: the loop runs inside
    /// the VM, one `execute()` per element, no AST conversion anywhere.
    /// Returns None to route the call through the interpreter bridge,
    /// which remains the semantic authority for everything declined here.
    /// Never falls back mid-loop: once the loop starts, an element error is
    /// the call's error, exactly as the interpreter propagates it.
    /// Named functions compiled through the hof dependency channel — the
    /// tier folds these into its `promoted` stat, since "compiled to the
    /// tier by name" is what that number has always meant, whichever
    /// channel did the compiling.
    pub(crate) fn hof_promotions(&self) -> u32 {
        self.hof_promotions
    }

    /// Compile a named callee a lambda body needs, from the VM's own
    /// noted function values — the hof-path twin of the tier's
    /// `compile_dependency`. Registered before compiling so self- and
    /// mutual recursion resolve; a failure withdraws the registration
    /// so nothing calls a name with no bytecode.
    fn compile_hof_dependency(&mut self, name: &str, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        if self.compiler.function_registry.contains_key(name) {
            return true;
        }
        if self.ambiguous_function_names.contains(name) {
            return false;
        }
        let Some(func) = self.known_function_values.get(name).cloned() else {
            return false;
        };
        if func.parameters.iter().any(|p| p.default_value.is_some())
            || !func.param_bounds.is_empty()
        {
            return false;
        }
        let dep_id = FunctionId::new();
        self.register_function(name.to_string(), dep_id);
        let decl = FunctionDecl {
            name_span: None,
            name: name.to_string(),
            type_params: Vec::new(),
            type_param_bounds: Vec::new(),
            parameters: func.parameters.clone(),
            return_type: None,
            body: (*func.body).clone(),
        };
        for _ in 0..8 {
            match self.compile_function_with_closure(
                dep_id,
                &decl,
                func.closure.clone(),
                func.param_checks.clone().into(),
                func.return_check.clone(),
                func.def_file.as_deref().map(Arc::from),
            ) {
                Ok(()) => {
                    self.hof_promotions += 1;
                    return true;
                }
                Err(BytecodeError::UnresolvedCallee(inner)) => {
                    if !self.compile_hof_dependency(&inner, depth + 1) {
                        self.unregister_function(name);
                        return false;
                    }
                }
                Err(_) => {
                    self.unregister_function(name);
                    return false;
                }
            }
        }
        self.unregister_function(name);
        false
    }

    /// The tier-facing door to the native higher-order loops: the
    /// interpreter's builtin `map`/`filter` convert their list once,
    /// call this, and convert the result once — where the per-element
    /// route paid a full interpreter→tier boundary per item.
    pub(crate) fn native_hof(
        &mut self,
        name: &str,
        args: &[OvmValue],
    ) -> Option<Result<OvmValue, BytecodeError>> {
        self.try_native_higher_order(name, args)
    }

    fn try_native_higher_order(
        &mut self,
        name: &str,
        args: &[OvmValue],
    ) -> Option<Result<OvmValue, BytecodeError>> {
        use crate::ovm::value::ValueData;
        // A wrapped interpreter list routes to the bridge — where it
        // arrives O(1) and the interpreter (with its own tier, its
        // fusions, and auto-parallel machinery) is the semantic
        // authority — except for the O(1) probes below, which answer
        // through the wrapper without conversion.
        if args.iter().any(|a| matches!(a.data, ValueData::AstList(_)))
            && !matches!(name, "len" | "head")
        {
            return None;
        }
        match name {
            "map" | "filter" if args.len() == 2 => {
                let items = match &args[0].data {
                    ValueData::List(items) => items.clone(),
                    _ => return None,
                };
                // Plain function values compile on demand and are called
                // with just the element; runtime closures carry a func_id
                // already compiled with their captures as trailing
                // parameters, appended after the element on every call.
                let (func_id, captures) = match &args[1].data {
                    ValueData::AstFunction(f) => {
                        let f = f.clone();
                        (self.hof_function_id(&f, 1)?, Vec::new())
                    }
                    ValueData::Closure(c) if c.template.parameters.len() == 1 => {
                        (c.func_id, c.captured.clone())
                    }
                    _ => return None,
                };
                let is_map = name == "map";

                // Fused fast path: the callee, its arity, its (absent)
                // parameter checks, and — on native — its JIT status do not
                // change across elements, so hoist all of that out of the
                // loop. Fetch the bytecode `Arc` once and call the body per
                // element with the per-call preamble already discharged. On
                // native we defer to the plain `execute` path whenever the
                // JIT owns this function (compiled or pending), so native code
                // still runs and this never bypasses it; the fused loop only
                // takes over when the body is pure bytecode — every call under
                // wasm, and the common builtin-calling lambdas natively.
                let fused_ok = {
                    if let Ok(bytecode) = self.get_bytecode(func_id) {
                        let jit_owned = {
                            #[cfg(feature = "native")]
                            {
                                self.jit.has(func_id) || self.jit.is_pending(func_id)
                            }
                            #[cfg(not(feature = "native"))]
                            {
                                false
                            }
                        };
                        if !jit_owned
                            && bytecode.param_checks.is_empty()
                            && bytecode.param_count == 1 + captures.len()
                        {
                            Some(bytecode)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(bytecode) = fused_ok {
                    // `call_args` is [element, cap0, cap1, ...]; captures are
                    // constant, so build them once and only overwrite slot 0.
                    let mut call_args = Vec::with_capacity(1 + captures.len());
                    call_args.push(OvmValue::new_unit());
                    call_args.extend(captures.iter().cloned());
                    let mut out = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        call_args[0] = item.clone();
                        let result = match self.execute_prepared(&bytecode, &call_args) {
                            Ok(r) => r,
                            Err(e) => return Some(Err(e)),
                        };
                        if is_map {
                            out.push(result);
                        } else if matches!(result.data, ValueData::Boolean(true)) {
                            out.push(item.clone());
                        }
                    }
                    return Some(Ok(OvmValue::new_list(out)));
                }

                // Fallback: the plain per-element `execute` (JIT-aware, or a
                // lambda with parameter checks / an arity we let `execute`
                // diagnose). Identical results; only the preamble differs.
                let mut call_args = Vec::with_capacity(1 + captures.len());
                let mut run = || -> Result<OvmValue, BytecodeError> {
                    let mut out = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        call_args.clear();
                        call_args.push(item.clone());
                        call_args.extend(captures.iter().cloned());
                        let result = self.execute(func_id, &call_args)?;
                        if is_map {
                            out.push(result);
                        } else if matches!(result.data, ValueData::Boolean(true)) {
                            // The interpreter keeps an element only when the
                            // predicate is exactly Boolean(true); any other
                            // result silently drops it.
                            out.push(item.clone());
                        }
                    }
                    Ok(OvmValue::new_list(out))
                };
                Some(run())
            }
            "fold" | "reduce" if args.len() == 3 => {
                let items = match &args[0].data {
                    ValueData::List(items) => items.clone(),
                    _ => return None,
                };
                let (func_id, captures) = match &args[2].data {
                    ValueData::AstFunction(f) => {
                        let f = f.clone();
                        (self.hof_function_id(&f, 2)?, Vec::new())
                    }
                    ValueData::Closure(c) if c.template.parameters.len() == 2 => {
                        (c.func_id, c.captured.clone())
                    }
                    _ => return None,
                };
                let mut acc = args[1].clone();
                // Same two-lane structure as map: a fused loop when the
                // body is pure bytecode with no checks, the JIT-aware
                // `execute` otherwise. The accumulator threads through
                // slot 0; the element takes slot 1.
                let fused_ok = {
                    if let Ok(bytecode) = self.get_bytecode(func_id) {
                        let jit_owned = {
                            #[cfg(feature = "native")]
                            {
                                self.jit.has(func_id) || self.jit.is_pending(func_id)
                            }
                            #[cfg(not(feature = "native"))]
                            {
                                false
                            }
                        };
                        if !jit_owned
                            && bytecode.param_checks.is_empty()
                            && bytecode.param_count == 2 + captures.len()
                        {
                            Some(bytecode)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(bytecode) = fused_ok {
                    let mut call_args = Vec::with_capacity(2 + captures.len());
                    call_args.push(OvmValue::new_unit());
                    call_args.push(OvmValue::new_unit());
                    call_args.extend(captures.iter().cloned());
                    for item in items.iter() {
                        call_args[0] = acc;
                        call_args[1] = item.clone();
                        acc = match self.execute_prepared(&bytecode, &call_args) {
                            Ok(r) => r,
                            Err(e) => return Some(Err(e)),
                        };
                    }
                    return Some(Ok(acc));
                }
                let mut call_args = Vec::with_capacity(2 + captures.len());
                let mut run = || -> Result<OvmValue, BytecodeError> {
                    for item in items.iter() {
                        call_args.clear();
                        call_args.push(acc.clone());
                        call_args.push(item.clone());
                        call_args.extend(captures.iter().cloned());
                        acc = self.execute(func_id, &call_args)?;
                    }
                    Ok(acc.clone())
                };
                Some(run())
            }
            // ── native collection builtins ─────────────────────────────
            // These operate directly on the VM value model with no boundary
            // conversion — the bridged versions converted whole collections
            // to AST per call, which turned environment-threading programs
            // (the minilisp example) from a 57x workload into a 1.35x one.
            // Every arm mirrors the interpreter's checks in the same order
            // with the same messages; anything not matched falls to the
            // bridge, which stays the authority.
            "len" if args.len() == 1 => Some(match &args[0].data {
                ValueData::List(items) => Ok(OvmValue::new_integer(items.len() as i64)),
                ValueData::AstList(items) => Ok(OvmValue::new_integer(items.len() as i64)),
                ValueData::Tuple(items) => Ok(OvmValue::new_integer(items.len() as i64)),
                ValueData::String(st) => Ok(OvmValue::new_integer(st.chars().count() as i64)),
                // Mirrors the interpreter: a native that declares a length
                // (Bytes) answers; one that doesn't keeps the same error.
                ValueData::Native(h) if h.0.length().is_some() => {
                    Ok(OvmValue::new_integer(h.0.length().unwrap_or(0) as i64))
                }
                _ => Err(BytecodeError::TypeError(
                    "len: argument must be a list, string, or tuple".to_string(),
                )),
            }),
            "head" if args.len() == 1 => Some(match &args[0].data {
                ValueData::List(items) => match items.first() {
                    Some(v) => Ok(v.clone()),
                    None => Err(BytecodeError::RuntimeError(
                        "head: cannot get head of empty list".to_string(),
                    )),
                },
                ValueData::AstList(items) => match items.first() {
                    Some(v) => Ok(OvmValue::from_ast(v.clone())),
                    None => Err(BytecodeError::RuntimeError(
                        "head: cannot get head of empty list".to_string(),
                    )),
                },
                _ => Err(BytecodeError::TypeError(
                    "head: argument must be a list".to_string(),
                )),
            }),
            "tail" if args.len() == 1 => Some(match &args[0].data {
                ValueData::List(items) => {
                    if items.is_empty() {
                        Err(BytecodeError::RuntimeError(
                            "tail: cannot get tail of empty list".to_string(),
                        ))
                    } else {
                        Ok(OvmValue::new_list(items[1..].to_vec()))
                    }
                }
                _ => Err(BytecodeError::TypeError(
                    "tail: argument must be a list".to_string(),
                )),
            }),
            "cons" if args.len() == 2 => Some(match &args[1].data {
                ValueData::List(items) => {
                    let mut out = Vec::with_capacity(items.len() + 1);
                    out.push(args[0].clone());
                    out.extend(items.iter().cloned());
                    Ok(OvmValue::new_list(out))
                }
                _ => Err(BytecodeError::TypeError(
                    "cons: second argument must be a list".to_string(),
                )),
            }),
            "concat" if args.len() == 2 => Some(match (&args[0].data, &args[1].data) {
                (ValueData::List(a), ValueData::List(b)) => {
                    let mut out = Vec::with_capacity(a.len() + b.len());
                    out.extend(a.iter().cloned());
                    out.extend(b.iter().cloned());
                    Ok(OvmValue::new_list(out))
                }
                _ => Err(BytecodeError::TypeError(
                    "concat: arguments must be lists".to_string(),
                )),
            }),
            "skip" if args.len() == 2 => Some((|| {
                let n = match &args[1].data {
                    ValueData::Integer(i) if *i >= 0 => *i as usize,
                    _ => {
                        return Err(BytecodeError::TypeError(
                            "skip: second argument must be a non-negative integer".to_string(),
                        ));
                    }
                };
                match &args[0].data {
                    ValueData::List(items) => {
                        Ok(OvmValue::new_list(items.iter().skip(n).cloned().collect()))
                    }
                    _ => Err(BytecodeError::TypeError(
                        "skip: argument must be a list".to_string(),
                    )),
                }
            })()),
            "map_get" if args.len() == 2 => {
                Some(Self::native_map_get(&args[0], &args[1], "map_get"))
            }
            // Presence, not value: a key explicitly holding Unit still
            // exists, so this cannot ride on map_get's Unit-for-missing.
            "map_has_key" if args.len() == 2 => Some(Self::native_map_has_key(&args[0], &args[1])),
            "map_set" if args.len() == 3 => Some((|| {
                // Key coercion first, then the receiver — the interpreter's
                // check order.
                let key = match &args[1].data {
                    ValueData::String(st) => st.as_ref().clone(),
                    ValueData::Integer(i) => i.to_string(),
                    ValueData::Float(f) => crate::ast::format_float(*f),
                    ValueData::Boolean(b) => b.to_string(),
                    _ => {
                        return Err(BytecodeError::TypeError(
                            "map_set: key must be string, integer, float, or boolean".to_string(),
                        ));
                    }
                };
                match &args[0].data {
                    ValueData::Map(m) => {
                        let mut new_map = (**m).clone();
                        new_map.insert(key, args[2].clone());
                        Ok(OvmValue::new_map(Arc::new(new_map)))
                    }
                    ValueData::Struct(st) => {
                        let mut pairs: Vec<(String, OvmValue)> = st
                            .iter()
                            .filter(|(name, _)| **name != key)
                            .map(|(name, v)| (name.clone(), v.clone()))
                            .collect();
                        pairs.push((key, args[2].clone()));
                        Ok(OvmValue::new_struct(Arc::new(
                            crate::ovm::value::StructObject::from_pairs(st.type_name(), pairs),
                        )))
                    }
                    _ => Err(BytecodeError::TypeError(
                        "map_set: first argument must be a map or object".to_string(),
                    )),
                }
            })()),
            "entries" if args.len() == 1 => Some((|| {
                let mut pairs: Vec<(String, OvmValue)> = match &args[0].data {
                    ValueData::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                    ValueData::Struct(st) => {
                        st.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                    }
                    _ => {
                        return Err(BytecodeError::TypeError(
                            "entries: argument must be a map or object".to_string(),
                        ));
                    }
                };
                // The interpreter sorts keys, so entries is deterministic
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(OvmValue::new_list(
                    pairs
                        .into_iter()
                        .map(|(k, v)| OvmValue::new_tuple(vec![OvmValue::new_string(k), v]))
                        .collect(),
                ))
            })()),

            // Mirrors the interpreter's *sequential* sum exactly; lists past
            // the parallel threshold bridge out so the parallel behavior
            // (including its different overflow and float-order profile)
            // stays the interpreter's.
            "sum" if args.len() == 1 => {
                let items = match &args[0].data {
                    ValueData::List(items) => items.clone(),
                    _ => return None,
                };
                if crate::parallel::should_parallelize(items.len()) {
                    return None;
                }
                let mut int_acc: i64 = 0;
                let mut float_acc: f64 = 0.0;
                let mut is_float = false;
                for item in items.iter() {
                    match &item.data {
                        ValueData::Integer(n) => {
                            if is_float {
                                float_acc += *n as f64;
                            } else {
                                match int_acc.checked_add(*n) {
                                    Some(v) => int_acc = v,
                                    None => {
                                        return Some(Err(BytecodeError::RuntimeError(
                                            "sum: integer overflow".to_string(),
                                        )));
                                    }
                                }
                            }
                        }
                        ValueData::Float(f) => {
                            if !is_float {
                                float_acc = int_acc as f64;
                                is_float = true;
                            }
                            float_acc += *f;
                        }
                        _ => {
                            return Some(Err(BytecodeError::TypeError(
                                "sum: list must contain only numbers".to_string(),
                            )));
                        }
                    }
                }
                Some(Ok(if is_float {
                    OvmValue::new_float(float_acc)
                } else {
                    OvmValue::new_integer(int_acc)
                }))
            }
            _ => None,
        }
    }

    /// The float-math fast path: `math` builtins that accept numbers and
    /// always return Float in the interpreter (see stdlib/math.rs). A
    /// CallBuiltin's builtin_id indexes this table; entries record arity.
    /// Excluded on purpose: abs/min/max (integer-preserving), log (optional
    /// base), and the integer functions (factorial, gcd, lcm, ...).
    pub(crate) const FLOAT_MATH: &'static [(&'static str, usize)] = &[
        ("math.sqrt", 1),
        ("math.cbrt", 1),
        ("math.floor", 1),
        ("math.ceil", 1),
        ("math.round", 1),
        ("math.trunc", 1),
        ("math.fract", 1),
        ("math.sin", 1),
        ("math.cos", 1),
        ("math.tan", 1),
        ("math.asin", 1),
        ("math.acos", 1),
        ("math.atan", 1),
        ("math.sinh", 1),
        ("math.cosh", 1),
        ("math.tanh", 1),
        ("math.exp", 1),
        ("math.exp2", 1),
        ("math.ln", 1),
        ("math.log2", 1),
        ("math.log10", 1),
        ("math.to_degrees", 1),
        ("math.to_radians", 1),
        ("math.pow", 2),
        ("math.atan2", 2),
    ];

    fn float_math_id(name: &str, arity: usize) -> Option<u32> {
        Self::FLOAT_MATH
            .iter()
            .position(|(n, a)| *n == name && *a == arity)
            .map(|i| i as u32)
    }

    /// Mirrors the interpreter implementations exactly: every entry is a
    /// pure f64 operation from std. atan2 is y.atan2(x) with y = args[0].
    /// Whether a float-math builtin's argument falls outside the domain the
    /// interpreter's math module rejects. The ids match the FLOAT_MATH table.
    /// Domain-free functions (sin, exp, floor, pow, …) always return false.
    pub(crate) fn float_math_out_of_domain(id: usize, a: f64) -> bool {
        match id {
            0 => a < 0.0,                          // sqrt
            10 | 11 => !(-1.0..=1.0).contains(&a), // asin, acos
            18..=20 => a <= 0.0,                   // ln, log2, log10
            _ => false,
        }
    }

    pub(crate) fn eval_float_math(id: usize, a: f64, b: f64) -> f64 {
        match id {
            0 => a.sqrt(),
            1 => a.cbrt(),
            2 => a.floor(),
            3 => a.ceil(),
            4 => a.round(),
            5 => a.trunc(),
            6 => a.fract(),
            7 => a.sin(),
            8 => a.cos(),
            9 => a.tan(),
            10 => a.asin(),
            11 => a.acos(),
            12 => a.atan(),
            13 => a.sinh(),
            14 => a.cosh(),
            15 => a.tanh(),
            16 => a.exp(),
            17 => a.exp2(),
            18 => a.ln(),
            19 => a.log2(),
            20 => a.log10(),
            21 => a.to_degrees(),
            22 => a.to_radians(),
            23 => a.powf(b),
            24 => a.atan2(b),
            _ => f64::NAN,
        }
    }

    fn execute_builtin_call(
        &mut self,
        name: &str,
        args: &[OvmValue],
        pregranted: bool,
    ) -> Result<OvmValue, BytecodeError> {
        // Higher-order builtins loop natively when the function argument
        // compiles — otherwise everything below bridges to the interpreter.
        if let Some(result) = self.try_native_higher_order(name, args) {
            return result;
        }

        let mut ast_args = Vec::with_capacity(args.len());
        for arg in args {
            ast_args.push(
                arg.to_ast()
                    .map_err(|e| BytecodeError::RuntimeError(format!("{:?}", e)))?,
            );
        }

        self.ensure_bridge_interpreter();
        // Hand the bridge the enforcement context before it dispatches.
        // The gate lives in `BuiltinFunctions::call_internal` and reads it
        // off the interpreter it is given; the bridge is a *different*
        // interpreter from the one that owns the run, so without this it
        // would present an unrestricted, unattributed call and the tier
        // would be a hole in the manifest.
        let caps = self.caps.clone();
        let trace = self.caps_trace.clone();
        let attributed_to = self
            .caps_file_stack
            .last()
            .cloned()
            .flatten()
            .map(|f| f.to_string());
        let depth = self.call_depth as usize;
        let interpreter = self.builtin_interpreter.as_mut().expect("just ensured");
        interpreter.seed_bridge_caps(caps, trace, attributed_to);
        interpreter.set_call_depth_base(depth);
        if pregranted {
            // The compiler proved the static manifest grants this call
            // for this provenance; the gate's per-call table walk is
            // skipped. --trace-caps recording and the argument-dependent
            // fs sub-gate still run — only the yes/no lookup is elided.
            interpreter.set_cap_pregranted();
        }

        let result = BuiltinFunctions::call(&self.builtins, name, ast_args, interpreter)
            .map_err(|e| BytecodeError::RuntimeError(e.to_string()))?;

        // Defence in depth: the curated builtin set should only ever produce
        // representable values, but returning something lossy would silently
        // become Unit rather than failing, so check before converting.
        if !Self::round_trips(&result) {
            return Err(BytecodeError::RuntimeError(format!(
                "builtin '{}' returned a value the bytecode tier cannot represent",
                name
            )));
        }

        Ok(OvmValue::from_ast(result))
    }

    /// Iteration count for a `for` loop source, matching the interpreter:
    /// lists iterate by element, ranges by value without being materialized.
    fn iter_len(source: &OvmValue) -> Result<i64, BytecodeError> {
        use crate::ovm::value::ValueData;
        match &source.data {
            ValueData::List(items) => Ok(items.len() as i64),
            ValueData::AstList(items) => Ok(items.len() as i64),
            ValueData::Range(range) => {
                let span = if range.inclusive {
                    (range.end as i128) - (range.start as i128) + 1
                } else {
                    (range.end as i128) - (range.start as i128)
                };
                Ok(span.max(0).min(i64::MAX as i128) as i64)
            }
            // Strings iterate by character, matching the interpreter.
            ValueData::String(s) => Ok(s.chars().count() as i64),
            _ => {
                let hint = match &source.data {
                    ValueData::Map(_) | ValueData::Struct(_) => {
                        " — iterate its pairs with `for (k, v) in entries(m)`"
                    }
                    _ => "",
                };
                Err(BytecodeError::TypeError(format!(
                    "cannot iterate over a {}{}",
                    source.type_name(),
                    hint
                )))
            }
        }
    }

    /// The `idx`-th element of a `for` loop source.
    fn iter_get(source: &OvmValue, idx: i64) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;
        match &source.data {
            ValueData::AstList(items) => items
                .get(idx as usize)
                .map(|v| OvmValue::from_ast(v.clone()))
                .ok_or_else(|| {
                    BytecodeError::RuntimeError(format!("Iteration index {} out of bounds", idx))
                }),
            ValueData::List(items) => {
                items
                    .get(idx as usize)
                    .cloned()
                    .ok_or_else(|| BytecodeError::IndexOutOfBounds {
                        index: idx,
                        length: items.len(),
                    })
            }
            ValueData::Range(range) => range
                .start
                .checked_add(idx)
                .map(OvmValue::new_integer)
                .ok_or_else(|| {
                    BytecodeError::RuntimeError("Integer overflow iterating range".to_string())
                }),
            // Strings iterate by character, matching the interpreter.
            ValueData::String(s) => s
                .chars()
                .nth(idx as usize)
                .map(|c| OvmValue::new_string(c.to_string()))
                .ok_or_else(|| BytecodeError::IndexOutOfBounds {
                    index: idx,
                    length: s.chars().count(),
                }),
            _ => {
                let hint = match &source.data {
                    ValueData::Map(_) | ValueData::Struct(_) => {
                        " — iterate its pairs with `for (k, v) in entries(m)`"
                    }
                    _ => "",
                };
                Err(BytecodeError::TypeError(format!(
                    "cannot iterate over a {}{}",
                    source.type_name(),
                    hint
                )))
            }
        }
    }

    /// Total equality for pattern tests. Unlike the `Eq` instruction, operands
    /// of different types compare unequal instead of raising a type error — a
    /// literal pattern that doesn't apply must simply not match.
    fn pattern_eq(a: &OvmValue, b: &OvmValue) -> bool {
        use crate::ovm::value::ValueData;
        match (&a.data, &b.data) {
            (ValueData::Integer(x), ValueData::Integer(y)) => x == y,
            (ValueData::Float(x), ValueData::Float(y)) => x == y,
            (ValueData::Integer(x), ValueData::Float(y)) => (*x as f64) == *y,
            (ValueData::Float(x), ValueData::Integer(y)) => *x == (*y as f64),
            (ValueData::Boolean(x), ValueData::Boolean(y)) => x == y,
            (ValueData::String(x), ValueData::String(y)) => x == y,
            (ValueData::Unit, ValueData::Unit) => true,
            // Enums, structs, and collections compare structurally, as the
            // interpreter's Value equality does (its arms use Value's derived
            // PartialEq); conversion is exact so comparing the AST forms is
            // the same relation. Note this keeps the interpreter's asymmetry:
            // 1 == 1.0 is true (scalar arms above), but [1] == [1.0] is false
            // (Value's structural equality distinguishes element kinds).
            (ValueData::Enum(_), ValueData::Enum(_))
            | (ValueData::Struct(_), ValueData::Struct(_))
            | (ValueData::Map(_), ValueData::Map(_))
            | (ValueData::List(_), ValueData::List(_))
            | (ValueData::AstList(_), ValueData::AstList(_))
            | (ValueData::AstList(_), ValueData::List(_))
            | (ValueData::List(_), ValueData::AstList(_))
            | (ValueData::Tuple(_), ValueData::Tuple(_)) => match (a.to_ast(), b.to_ast()) {
                (Ok(x), Ok(y)) => x == y,
                _ => false,
            },
            _ => false,
        }
    }

    /// Whether a value survives conversion to the OVM model and back.
    ///
    /// Maps, structs, enums, and functions do not: they either collapse to a
    /// different type or to Unit. This is the single definition — the tier
    /// uses it too, rather than keeping a second copy that can drift.
    pub fn round_trips(value: &Value) -> bool {
        match value {
            Value::Integer(_)
            | Value::Float(_)
            | Value::Boolean(_)
            | Value::String(_)
            | Value::Unit
            | Value::Range { .. } => true,
            Value::List(items) => items.iter().all(Self::round_trips),
            Value::Tuple(items) => items.iter().all(Self::round_trips),
            Value::Ok(inner) | Value::Err(inner) => Self::round_trips(inner),
            // Structs, anonymous objects, and parsed JSON objects convert
            // symmetrically (type_name + fields), so they round-trip *when
            // every field does*. This excludes modules (their fields are
            // builtins) and any struct holding a function/map — those keep
            // the function on the interpreter. `from_ast` maps enums to a
            // struct shape lossily, so enums are deliberately not included.
            Value::Struct { fields, .. } => fields.values().all(Self::round_trips),
            // Function values wrap verbatim (AstFunction), so they always
            // round-trip — which is what lets user functions be passed as
            // arguments into promoted functions.
            Value::Function(_) => true,
            // Maps convert losslessly now that the OVM has a first-class
            // map value; they round-trip when every entry does.
            Value::Map(map) => map.values().all(Self::round_trips),
            // Enums convert losslessly (type, variant, payload) since the
            // OVM grew a first-class enum value; they round-trip when the
            // payload does.
            Value::Enum { variant_data, .. } => match variant_data {
                crate::ast::EnumVariantData::Unit => true,
                crate::ast::EnumVariantData::Tuple(values) => values.iter().all(Self::round_trips),
                crate::ast::EnumVariantData::Struct(fields) => {
                    fields.values().all(Self::round_trips)
                }
            },
            // Native values cross as the same Arc in both directions —
            // lossless by construction (pinned by tests/ods_module_test.rs).
            Value::Native(_) => true,
            _ => false,
        }
    }

    /// Check if value is truthy
    /// Truthiness for `if`/`while` conditions. Must match the interpreter's
    /// `to_boolean` (src/interpreter/ops.rs) exactly, or a promoted function
    /// disagrees with the tree-walker: an *empty* string, list, tuple, or
    /// range is falsy (like `0`/`false`/Unit), not just Unit. The old
    /// catch-all made empty collections truthy, so `if xs => …` on a
    /// promoted function took the wrong branch for an empty `xs`.
    fn is_truthy(&self, value: &OvmValue) -> bool {
        use crate::ovm::value::ValueData;

        match &value.data {
            ValueData::Boolean(b) => *b,
            ValueData::Integer(i) => *i != 0,
            ValueData::Float(f) => *f != 0.0,
            ValueData::String(s) => !s.is_empty(),
            ValueData::List(items) => !items.is_empty(),
            ValueData::AstList(items) => !items.is_empty(),
            ValueData::Tuple(items) => !items.is_empty(),
            ValueData::Range(r) => {
                if r.inclusive {
                    r.start <= r.end
                } else {
                    r.start < r.end
                }
            }
            ValueData::Unit => false,
            _ => true,
        }
    }

    /// Get VM statistics
    pub fn get_stats(&self) -> &VmStatistics {
        &self.stats
    }

    /// GetField's miss path: resolve by name, refill the inline cache when
    /// the object is a struct, and produce the interpreter-exact errors.
    /// Outlined so the dispatch arm stays small enough not to perturb the
    /// loop's code layout.
    #[cold]
    #[inline(never)]
    fn get_field_slow(
        &self,
        bytecode: &CompiledBytecode,
        name_const: u32,
        object: &OvmValue,
        cache: &FieldCache,
    ) -> Result<OvmValue, BytecodeError> {
        let name_value = bytecode
            .constants
            .get(name_const as usize)
            .ok_or(BytecodeError::InvalidConstantIndex(name_const))?;
        let name: &str = match &name_value.data {
            crate::ovm::value::ValueData::String(s) => s,
            _ => {
                return Err(BytecodeError::RuntimeError(
                    "GetField: field name constant is not a string".to_string(),
                ));
            }
        };
        if let crate::ovm::value::ValueData::Struct(st) = &object.data
            && let Some(idx) = st.shape.field_index(name)
        {
            cache.store(st.shape.id, idx);
        }
        self.execute_get_field(object, name)
    }

    /// Field access by name, matching `Interpreter::eval_field_access`
    /// exactly: struct/object/module fields look up by name; a module reports
    /// a "Function not found" message, a struct a "Field not found" one; a
    /// non-struct is a type error.
    /// Identical to `Interpreter::no_field_or_method`, so the two engines
    /// word a missing field/method the same way (tier agreement).
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

    fn execute_get_field(&self, object: &OvmValue, field: &str) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;
        match &object.data {
            ValueData::Struct(s) => s.field(field).cloned().ok_or_else(|| {
                if s.type_name() == "Module" {
                    // Same wording (and nearest-member suggestion) as the
                    // interpreter — the tiers must report identically.
                    BytecodeError::TypeError(crate::interpreter::module_member_miss(
                        field,
                        s.iter().map(|(name, _)| name.as_str()),
                    ))
                } else {
                    BytecodeError::TypeError(self.no_field_or_method(s.type_name(), field))
                }
            }),
            _ => Err(BytecodeError::TypeError(format!(
                "Cannot access field '{}' on non-struct value",
                field
            ))),
        }
    }

    /// Subscript, matching `Interpreter`'s `Expr::Index`: lists, tuples, and
    /// strings with an integer index; negatives count from the end; out of
    /// bounds is a runtime error with the same shape of message.
    fn execute_index_get(object: &OvmValue, index: &OvmValue) -> Result<OvmValue, BytecodeError> {
        use crate::ovm::value::ValueData;

        // A native value gets first refusal on its own subscript, and it
        // is asked before the integer-index requirement below — a Frame
        // is indexed by column name, which is the whole point.
        if let ValueData::Native(handle) = &object.data {
            let key = index
                .to_ast()
                .map_err(|err| BytecodeError::TypeError(err.to_string()))?;
            return match handle.0.index(&key) {
                Some(Ok(value)) => Ok(OvmValue::from_ast(value)),
                Some(Err(message)) => Err(BytecodeError::RuntimeError(message)),
                None => Err(BytecodeError::TypeError(format!(
                    "A {} cannot be indexed",
                    handle.0.type_name()
                ))),
            };
        }

        match &object.data {
            ValueData::Map(_) => {
                return Err(BytecodeError::TypeError(
                    "a Map is not indexed with `[]`; read a key with `map_get(m, key)`".to_string(),
                ));
            }
            ValueData::Struct(_) => {
                return Err(BytecodeError::TypeError(
                    "a struct or object is read by field (`value.name`) or with \
                     `map_get(value, name)`, not with `[]`"
                        .to_string(),
                ));
            }
            _ => {}
        }
        let idx = match &index.data {
            ValueData::Integer(i) => *i,
            _ => {
                return Err(BytecodeError::TypeError(
                    "Index must be an integer".to_string(),
                ));
            }
        };
        // Resolve a possibly-negative index against a length; None if OOB.
        let resolve = |len: usize| -> Option<usize> {
            let pos = if idx < 0 { len as i64 + idx } else { idx };
            if pos >= 0 && (pos as usize) < len {
                Some(pos as usize)
            } else {
                None
            }
        };

        match &object.data {
            ValueData::List(list) => {
                resolve(list.len()).map(|i| list[i].clone()).ok_or_else(|| {
                    BytecodeError::RuntimeError(format!(
                        "Index {} out of bounds for list of length {}",
                        idx,
                        list.len()
                    ))
                })
            }
            // A wrapped interpreter list reads element-wise: one
            // conversion per access, never a whole-list one.
            ValueData::AstList(items) => resolve(items.len())
                .map(|i| OvmValue::from_ast(items[i].clone()))
                .ok_or_else(|| {
                    BytecodeError::RuntimeError(format!(
                        "Index {} out of bounds for list of length {}",
                        idx,
                        items.len()
                    ))
                }),
            ValueData::Tuple(tuple) => {
                resolve(tuple.len())
                    .map(|i| tuple[i].clone())
                    .ok_or_else(|| {
                        BytecodeError::RuntimeError(format!(
                            "Index {} out of bounds for tuple of length {}",
                            idx,
                            tuple.len()
                        ))
                    })
            }
            ValueData::String(s) => {
                let chars: Vec<char> = s.chars().collect();
                resolve(chars.len())
                    .map(|i| OvmValue::new_string(chars[i].to_string()))
                    .ok_or_else(|| {
                        BytecodeError::RuntimeError(format!(
                            "Index {} out of bounds for string of length {}",
                            idx,
                            chars.len()
                        ))
                    })
            }
            _ => Err(BytecodeError::TypeError(
                "Only lists, tuples, and strings can be indexed".to_string(),
            )),
        }
    }
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionState {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            base: 0,
            top: 0,
        }
    }

    /// Claim the next window on the slab for a new frame: arguments land in
    /// the first registers (the compiler assigns parameters 0..n), the rest
    /// reset to Unit with the drop-skip for immediates. Returns the caller's
    /// (base, top) for pop_frame.
    #[inline]
    /// push_frame, consuming the argument buffer: each value moves into
    /// its register (the buffer slot is left Unit), so an Arc-backed
    /// argument arrives solely owned and in-place writes stay in place.
    pub fn push_frame_taking(
        &mut self,
        register_count: usize,
        args: &mut [OvmValue],
    ) -> (usize, usize) {
        let saved = (self.base, self.top);
        let new_base = self.top;
        let new_top = new_base + register_count;
        if self.stack.len() < new_top {
            self.stack.resize(new_top, OvmValue::new_unit());
        }
        let window = &mut self.stack[new_base..new_top];
        for (i, slot) in window.iter_mut().enumerate() {
            match args.get_mut(i) {
                Some(arg) => Self::write_slot(slot, std::mem::replace(arg, OvmValue::new_unit())),
                None => Self::reset_slot(slot),
            }
        }
        self.base = new_base;
        self.top = new_top;
        saved
    }

    pub fn push_frame(&mut self, register_count: usize, args: &[OvmValue]) -> (usize, usize) {
        let saved = (self.base, self.top);
        let new_base = self.top;
        let new_top = new_base + register_count;
        if self.stack.len() < new_top {
            self.stack.resize(new_top, OvmValue::new_unit());
        }
        let window = &mut self.stack[new_base..new_top];
        for (i, slot) in window.iter_mut().enumerate() {
            match args.get(i) {
                Some(arg) => Self::write_slot(slot, arg.clone()),
                None => Self::reset_slot(slot),
            }
        }
        self.base = new_base;
        self.top = new_top;
        saved
    }

    /// push_frame, but the arguments come straight from the CALLER's
    /// registers — one clone from the caller's window into the callee's,
    /// no intermediate buffer.
    #[inline]
    pub fn push_frame_from_regs(
        &mut self,
        register_count: usize,
        arg_regs: &[Register],
        arg_moves: u64,
    ) -> Result<(usize, usize), BytecodeError> {
        let saved = (self.base, self.top);
        let new_base = self.top;
        let new_top = new_base + register_count;
        if self.stack.len() < new_top {
            self.stack.resize(new_top, OvmValue::new_unit());
        }
        for (i, reg) in arg_regs.iter().enumerate() {
            let src = self.base + reg.0 as usize;
            if src >= self.top {
                return Err(BytecodeError::InvalidRegister(*reg));
            }
            // A masked argument is dead in the caller after this call:
            // take it instead of cloning, releasing the caller's hold on
            // whatever the register carried (the T2 handle-pin fix).
            let value = if i < 64 && arg_moves & (1 << i) != 0 {
                std::mem::replace(&mut self.stack[src], OvmValue::new_unit())
            } else {
                self.stack[src].clone()
            };
            if new_base + i < new_top {
                Self::write_slot(&mut self.stack[new_base + i], value);
            }
        }
        for slot in &mut self.stack[new_base + arg_regs.len().min(register_count)..new_top] {
            Self::reset_slot(slot);
        }
        self.base = new_base;
        self.top = new_top;
        Ok(saved)
    }

    /// Return to the caller's window. The callee's values stay on the slab
    /// above the logical top and are recycled by the next push_frame.
    #[inline]
    /// Debug: absolute slab indices whose slot holds this AstList arc.
    pub fn debug_slots_holding(&self, arc: &Arc<Vec<crate::ast::Value>>) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, slot) in self.stack.iter().enumerate() {
            if let crate::ovm::value::ValueData::AstList(other) = &slot.data
                && Arc::ptr_eq(other, arc)
            {
                out.push(i);
            }
        }
        out
    }

    /// Debug: the current frame base.
    pub fn debug_base(&self) -> usize {
        self.base
    }

    pub fn pop_frame(&mut self, saved: (usize, usize)) {
        // Release the popped window's values, not just the window: a
        // stale slot would pin every Arc it held until some future frame
        // happened to overwrite it — and because the in-place fusions
        // mutate a list under the SAME Arc across calls, one stale slot
        // pinned a collections handle forever, turning every subsequent
        // sole-owner write into a whole-list copy. Slot reuse keeps the
        // recycle-in-place cheapness (reset_slot leaves the allocation).
        for slot in &mut self.stack[self.base..self.top] {
            Self::reset_slot(slot);
        }
        self.base = saved.0;
        self.top = saved.1;
    }

    #[inline]
    pub fn get_register(&self, reg: Register) -> Result<OvmValue, BytecodeError> {
        let idx = self.base + reg.0 as usize;
        if idx < self.top {
            Ok(self.stack[idx].clone())
        } else {
            Err(BytecodeError::InvalidRegister(reg))
        }
    }

    /// Borrow a register without cloning. Cloning an OvmValue rebuilds it
    /// (and bumps an Arc for heap payloads), which dominated the dispatch
    /// loop when every operand read went through get_register.
    #[inline]
    pub fn register_ref(&self, reg: Register) -> Result<&OvmValue, BytecodeError> {
        let idx = self.base + reg.0 as usize;
        if idx < self.top {
            Ok(&self.stack[idx])
        } else {
            Err(BytecodeError::InvalidRegister(reg))
        }
    }

    /// Borrow two registers at once (operands of a binary instruction).
    #[inline]
    pub fn register_pair(
        &self,
        lhs: Register,
        rhs: Register,
    ) -> Result<(&OvmValue, &OvmValue), BytecodeError> {
        Ok((self.register_ref(lhs)?, self.register_ref(rhs)?))
    }

    /// True when a value owns no heap payload, read off the data itself
    /// rather than the header tag so it cannot disagree with reality.
    #[inline]
    fn owns_nothing(value: &OvmValue) -> bool {
        use crate::ovm::value::ValueData;
        matches!(
            value.data,
            ValueData::Integer(_) | ValueData::Float(_) | ValueData::Boolean(_) | ValueData::Unit
        )
    }

    /// Overwrite a slot with Unit, skipping drop glue when the old value
    /// owned nothing. `forget` on a value that owns nothing leaks nothing.
    #[inline]
    fn reset_slot(slot: &mut OvmValue) {
        if Self::owns_nothing(slot) {
            std::mem::forget(std::mem::replace(slot, OvmValue::new_unit()));
        } else {
            *slot = OvmValue::new_unit();
        }
    }

    #[inline]
    /// Move a value out of a register, leaving Unit. Used by AddAssign
    /// so a uniquely-held string can be appended in place; the register
    /// is rewritten before the instruction completes.
    pub fn take_register(&mut self, reg: Register) -> Result<OvmValue, BytecodeError> {
        let idx = self.base + reg.0 as usize;
        if idx < self.top {
            Ok(std::mem::replace(
                &mut self.stack[idx],
                OvmValue::new_unit(),
            ))
        } else {
            Err(BytecodeError::InvalidRegister(reg))
        }
    }

    pub fn set_register(&mut self, reg: Register, value: OvmValue) -> Result<(), BytecodeError> {
        let idx = self.base + reg.0 as usize;
        if idx < self.top {
            Self::write_slot(&mut self.stack[idx], value);
            Ok(())
        } else {
            Err(BytecodeError::InvalidRegister(reg))
        }
    }

    /// Overwrite a slot, skipping drop glue when the old value owned
    /// nothing — the common case in a numeric kernel, where the glue was
    /// ~25% of VM samples in a profile.
    #[inline]
    fn write_slot(slot: &mut OvmValue, value: OvmValue) {
        if Self::owns_nothing(slot) {
            std::mem::forget(std::mem::replace(slot, value));
        } else {
            *slot = value;
        }
    }
}

// Compiler and optimization implementations
impl Default for BytecodeCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl BytecodeCompiler {
    pub fn new() -> Self {
        Self {
            static_caps: None,
            current_def_file: None,
            pending_param_checks: std::sync::Arc::from(Vec::new()),
            pending_return_check: None,
            pending_def_file: None,
            register_allocator: RegisterAllocator::new(),
            emitter: InstructionEmitter::new(),
            optimizer: BytecodeOptimizer::new(),
            local_variables: HashMap::new(),
            builtin_names: std::collections::HashSet::new(),
            loop_targets: Vec::new(),
            span_stack: Vec::new(),
            enclosing_closure: std::sync::Arc::new(im::HashMap::new()),
            enclosing_bound_names: std::collections::HashSet::new(),
            _label_counter: 0,
            function_registry: HashMap::new(),
            struct_defs: HashMap::new(),
            struct_field_checks: HashMap::new(),
            unit_variant_names: std::collections::HashSet::new(),
            known_function_values: HashMap::new(),
            self_call: None,
            pending_lambdas: Vec::new(),
        }
    }

    /// The grant governing the function being compiled, when a static
    /// table is installed. None = no manifest, no folding.
    fn static_grant(&self) -> Option<crate::caps::Caps> {
        let table = self.static_caps.as_ref()?;
        let (caps, _) = table.caps_for(self.current_def_file.as_deref());
        Some(*caps)
    }

    /// Compile-time answer to `caps.allowed(name)`: Some(verdict) when a
    /// static manifest is installed and the name is a known capability.
    /// The table is fixed for the run and grants attenuate by
    /// provenance, both known here — so the runtime answer cannot
    /// differ.
    fn fold_cap_allowed(&self, cap: &str) -> Option<bool> {
        let caps = self.static_grant()?;
        crate::stdlib::caps_mod::holds(&caps, cap)
    }

    /// True when the static manifest fully grants `builtin` for this
    /// function's provenance — the pre-grant that lets the runtime skip
    /// its per-call gate walk. False for denied (the runtime gate owns
    /// the error), for ungated builtins (nothing to skip), and when no
    /// manifest is installed (the gate is already one branch).
    fn pregrant(&self, builtin: &str) -> bool {
        if crate::caps::required(builtin).is_none() {
            return false;
        }
        match self.static_grant() {
            Some(caps) => crate::caps::check(&caps, builtin).is_none(),
            None => false,
        }
    }

    pub fn compile_function(
        &mut self,
        func_id: FunctionId,
        func: &FunctionDecl,
    ) -> Result<CompiledBytecode, BytecodeError> {
        // The provenance the capability table attenuates by; resolved
        // once per compile so per-callsite verdicts are map lookups.
        self.current_def_file = self
            .pending_def_file
            .as_deref()
            .map(|f| std::fs::canonicalize(f).unwrap_or_else(|_| std::path::PathBuf::from(f)));
        // Reset state
        self.register_allocator.reset();
        self.emitter.reset();
        self.local_variables.clear();
        self.loop_targets.clear();

        // Names this function ever binds or assigns, for lambda eligibility
        self.enclosing_bound_names.clear();
        for param in &func.parameters {
            self.enclosing_bound_names.insert(param.name.clone());
        }
        Self::collect_bound_names(&func.body, &mut self.enclosing_bound_names);

        // Parameters occupy the first registers, in declaration order
        for param in &func.parameters {
            let reg = self.register_allocator.allocate_register();
            self.local_variables.insert(param.name.clone(), reg);
        }

        // Compile function body
        let result_reg = self.compile_expression(&func.body)?;

        // Ensure function returns
        if !self.emitter.has_return() {
            self.emitter.emit_return(Some(result_reg));
        }

        // Patch jump targets from label ids to instruction offsets
        self.emitter.resolve_labels()?;

        let mut instructions = self.emitter.take_instructions();
        let constants = self.emitter.take_constants();

        instructions = self
            .optimizer
            .optimize_instructions(instructions, &constants)?;

        // Tail-call elimination: a CallFn back to this very function
        // whose result flows untouched (through Moves and Jumps) to a
        // Return is a frame that never needs to exist.
        Self::eliminate_self_tail_calls(func_id, &mut instructions);
        // TCE strands the old call-result plumbing (a merge Move reading
        // the register the eliminated CallFn used to write) as dead code
        // past the new back-edge. The VM never executes it, but the JIT
        // builds SSA for every instruction — and a read of a
        // never-written register fails finalize, silently keeping every
        // tail-recursive function off native. Sweep again, after TCE.
        instructions = BytecodeOptimizer::sweep_unreachable(instructions);

        // OLANG_DUMP_FN=<name> prints the final instruction stream for one
        // function — the register-level view the JIT debug summary elides.
        if let Some(want) = std::env::var_os("OLANG_DUMP_FN")
            && *want == *func.name.as_str()
        {
            eprintln!(
                "[dump] fn '{}' ({} params):",
                func.name,
                func.parameters.len()
            );
            for (pc, inst) in instructions.iter().enumerate() {
                eprintln!("  {:4}: {:?}", pc, inst);
            }
        }

        Ok(CompiledBytecode {
            function_id: func_id,
            instructions,
            register_count: self.register_allocator.max_register_used(),
            local_count: 0,
            param_count: func.parameters.len(),
            // Carried from the declaration so the capability gate can
            // attribute this function's effects to the package that owns
            // its source, exactly as the interpreter does from its own
            // call stack.
            def_file: self.pending_def_file.clone(),
            param_names: func
                .parameters
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .into(),
            param_checks: self.pending_param_checks.clone(),
            return_check: self.pending_return_check.clone(),
            constants,
            span_table: self.emitter.take_spans(),
            debug_info: BytecodeDebugInfo {
                function_name: Some(func.name.clone()),
                ..Default::default()
            },
            optimization_level: 1,
            entry_point: 0,
        })
    }

    /// Rewrite self-calls in tail position to `TailCallSelf`. A call is
    /// in tail position when, from the instruction after it, execution
    /// reaches a `Return` of the call's result having passed only
    /// through `Move`s of that result and unconditional `Jump`s — the
    /// exact shape branch merges compile to. Runs after the optimizer,
    /// on resolved jump offsets.
    fn eliminate_self_tail_calls(func_id: FunctionId, instructions: &mut [Instruction]) {
        let mut rewrites = Vec::new();
        for (i, inst) in instructions.iter().enumerate() {
            if let Instruction::CallFn {
                dst,
                func_id: target,
                args,
                ..
            } = inst
                && *target == func_id
                && Self::result_flows_to_return(instructions, i + 1, *dst)
            {
                rewrites.push((i, *dst, args.clone()));
            }
        }
        if rewrites.is_empty() {
            return;
        }
        // The result-flow chain behind each rewritten call is unreachable
        // (a tail call never falls through), but it still *reads* the
        // call's result register — which now has no writer, and the JIT's
        // flow-insensitive qualification would refuse the function over a
        // read that can never happen. Nop the chain's private prefix; a
        // shared merge point (anything some other instruction jumps to)
        // stays, because live paths still flow through it.
        let mut jump_targets = std::collections::HashSet::new();
        for inst in instructions.iter() {
            match inst {
                Instruction::Jump { target }
                | Instruction::JumpIfTrue { target, .. }
                | Instruction::JumpIfFalse { target, .. } => {
                    jump_targets.insert(target.0 as usize);
                }
                _ => {}
            }
        }
        for (i, dst, args) in rewrites {
            instructions[i] = Instruction::TailCallSelf { args };
            let mut pc = i + 1;
            let mut cur = dst;
            let mut steps = 0usize;
            while pc < instructions.len() && !jump_targets.contains(&pc) {
                steps += 1;
                if steps > instructions.len() {
                    break;
                }
                match &instructions[pc] {
                    Instruction::Move { dst, src } | Instruction::TakeMove { dst, src }
                        if *src == cur =>
                    {
                        cur = *dst;
                        instructions[pc] = Instruction::Nop;
                        pc += 1;
                    }
                    Instruction::Nop => pc += 1,
                    Instruction::Jump { target } => {
                        let t = target.0 as usize;
                        instructions[pc] = Instruction::Nop;
                        pc = t;
                    }
                    Instruction::Return { value: Some(r) } if *r == cur => {
                        instructions[pc] = Instruction::Nop;
                        break;
                    }
                    _ => break,
                }
            }
        }
    }

    /// Does execution starting at `pc` reach `Return` of `cur` touching
    /// nothing else? Conservative: any other instruction, a conditional
    /// jump, or a cycle answers no.
    fn result_flows_to_return(
        instructions: &[Instruction],
        mut pc: usize,
        mut cur: Register,
    ) -> bool {
        let mut steps = 0usize;
        while pc < instructions.len() {
            steps += 1;
            if steps > instructions.len() {
                return false;
            }
            match &instructions[pc] {
                Instruction::Move { dst, src } | Instruction::TakeMove { dst, src }
                    if *src == cur =>
                {
                    cur = *dst;
                    pc += 1;
                }
                Instruction::Nop => pc += 1,
                Instruction::Jump { target } => pc = target.0 as usize,
                Instruction::Return { value: Some(r) } => return *r == cur,
                _ => return false,
            }
        }
        false
    }

    fn compile_expression(&mut self, expr: &Expr) -> Result<Register, BytecodeError> {
        match expr {
            Expr::Integer(value) => {
                let const_idx = self
                    .emitter
                    .add_constant(OvmValue::from_ast(Value::Integer(*value)));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            Expr::Float(value) => {
                let const_idx = self
                    .emitter
                    .add_constant(OvmValue::from_ast(Value::Float(*value)));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            Expr::Boolean(value) => {
                let const_idx = self
                    .emitter
                    .add_constant(OvmValue::from_ast(Value::Boolean(*value)));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            Expr::String(value) => {
                let const_idx =
                    self.emitter
                        .add_constant(OvmValue::from_ast(Value::String(Arc::new(
                            (**value).clone(),
                        ))));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            // A slot-resolved reference from the interpreter's resolver is
            // just a named identifier here — the VM has its own registers
            Expr::LocalRef { name, .. } => self.compile_expression(&Expr::Identifier(name.clone())),
            Expr::LocalAssign { name, value, .. } => self.compile_expression(&Expr::Assignment {
                target: name.clone(),
                value: value.clone(),
            }),

            Expr::Identifier(name) => {
                if let Some(&reg) = self.local_variables.get(name) {
                    // The variable already lives in a register — nothing to emit
                    Ok(reg)
                } else {
                    // A free identifier resolves through the function value's
                    // own closure — which the interpreter installs verbatim as
                    // the call environment, and closures are snapshots (a
                    // global mutated after declaration is not seen; verified
                    // before building on it). So a closure hit IS the value
                    // the name will resolve to, baked as a constant. A miss
                    // (the interpreter would fall back to the caller's scope
                    // chain, which is runtime state) refuses compilation.
                    match self.enclosing_closure.get(name) {
                        // A function value is wrapped verbatim (AstFunction),
                        // the same representation the lambda machinery uses,
                        // so it converts back unchanged and the native
                        // higher-order path can compile it.
                        Some(Value::Function(func)) => {
                            let const_idx = self
                                .emitter
                                .add_constant(OvmValue::new_ast_function(func.clone()));
                            let dst_reg = self.register_allocator.allocate_register();
                            self.emitter.emit_load_const(dst_reg, const_idx);
                            Ok(dst_reg)
                        }
                        Some(value) if BytecodeVm::round_trips(value) => {
                            let const_idx =
                                self.emitter.add_constant(OvmValue::from_ast(value.clone()));
                            let dst_reg = self.register_allocator.allocate_register();
                            self.emitter.emit_load_const(dst_reg, const_idx);
                            Ok(dst_reg)
                        }
                        _ => Err(BytecodeError::CompilationFailed(format!(
                            "Unresolved identifier '{}' (absent from the function's closure, or not tier-representable)",
                            name
                        ))),
                    }
                }
            }

            // Unary `-` and `!`. The VM has had Neg/Not instructions and an
            // execute_unary_op matching the interpreter's semantics exactly
            // (checked_neg with the same overflow message, `-x` on floats,
            // `!b` on booleans, type error otherwise) all along -- the
            // compiler simply never emitted them, so any function containing
            // a `-x` was refused and stayed on the interpreter.
            Expr::UnaryOp { op, operand } => {
                let operand_reg = self.compile_expression(operand)?;
                let dst_reg = self.register_allocator.allocate_register();
                let instruction = match op {
                    UnaryOp::Negate => Instruction::Neg {
                        dst: dst_reg,
                        src: operand_reg,
                    },
                    UnaryOp::Not => Instruction::Not {
                        dst: dst_reg,
                        src: operand_reg,
                    },
                };
                self.emitter.instructions.push(instruction);
                Ok(dst_reg)
            }

            Expr::BinaryOp { left, op, right } => {
                // `&&` / `||` short-circuit: the right operand is compiled
                // behind a conditional jump taken only when the left doesn't
                // already settle the result. The short-circuit test must be
                // "is exactly Boolean(false/true)" — NOT truthiness — to match
                // the interpreter (`0 && x` still evaluates x and then type-
                // errors in the And instruction, exactly like the tree-walk).
                // PatternEq never errors and always yields a Boolean, which
                // makes it safe to branch on.
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    let left_reg = self.compile_expression(left)?;
                    // The combine instruction reads the left register after
                    // the right operand has run — shield it from rhs writes.
                    let left_reg = self.shield_operand(left_reg, !Self::assignment_free(right));

                    let settle_value = matches!(op, BinaryOp::Or); // false for &&, true for ||
                    let settle_const = self
                        .emitter
                        .add_constant(OvmValue::new_boolean(settle_value));
                    let settle_reg = self.register_allocator.allocate_register();
                    self.emitter.emit_load_const(settle_reg, settle_const);

                    let settles = self.register_allocator.allocate_register();
                    self.emitter.instructions.push(Instruction::PatternEq {
                        dst: settles,
                        value: left_reg,
                        other: settle_reg,
                    });

                    // Preset the result to the settling value; overwritten on
                    // the right-hand path.
                    let dst_reg = self.register_allocator.allocate_register();
                    self.emitter.emit_load_const(dst_reg, settle_const);

                    let rhs_label = self.emitter.create_label();
                    let end_label = self.emitter.create_label();
                    self.emitter.emit_branch_if_false(settles, rhs_label);
                    self.emitter.emit_jump(end_label);

                    self.emitter.place_label(rhs_label);
                    let right_reg = self.compile_expression(right)?;
                    let combine = match op {
                        BinaryOp::And => Instruction::And {
                            dst: dst_reg,
                            lhs: left_reg,
                            rhs: right_reg,
                        },
                        _ => Instruction::Or {
                            dst: dst_reg,
                            lhs: left_reg,
                            rhs: right_reg,
                        },
                    };
                    self.emitter.instructions.push(combine);
                    self.emitter.place_label(end_label);
                    return Ok(dst_reg);
                }

                // A numeric literal operand rides in the instruction as an
                // immediate: no LoadConst, no constant register. A literal
                // has no side effects, so skipping its "evaluation" is
                // unobservable; a literal LEFT operand only fuses when the
                // operation commutes or the comparison can flip.
                let imm_of = |e: &Expr| match e {
                    Expr::Integer(n) => Some(OvmValue::new_integer(*n)),
                    Expr::Float(f) => Some(OvmValue::new_float(*f)),
                    _ => None,
                };
                let flipped = |op: &BinaryOp| match op {
                    BinaryOp::Add | BinaryOp::Multiply => Some(op.clone()),
                    BinaryOp::Equal | BinaryOp::NotEqual => Some(op.clone()),
                    BinaryOp::LessThan => Some(BinaryOp::GreaterThan),
                    BinaryOp::LessThanEqual => Some(BinaryOp::GreaterThanEqual),
                    BinaryOp::GreaterThan => Some(BinaryOp::LessThan),
                    BinaryOp::GreaterThanEqual => Some(BinaryOp::LessThanEqual),
                    _ => None,
                };
                let immediate_ops = matches!(
                    op,
                    BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::Modulo
                        | BinaryOp::Equal
                        | BinaryOp::NotEqual
                        | BinaryOp::LessThan
                        | BinaryOp::LessThanEqual
                        | BinaryOp::GreaterThan
                        | BinaryOp::GreaterThanEqual
                );
                if immediate_ops {
                    if let Some(imm) = imm_of(right) {
                        let left_reg = self.compile_expression(left)?;
                        let dst_reg = self.register_allocator.allocate_register();
                        self.emitter.instructions.push(Instruction::BinImm {
                            op: op.clone(),
                            dst: dst_reg,
                            lhs: left_reg,
                            imm,
                            swapped: false,
                        });
                        return Ok(dst_reg);
                    }
                    if let (Some(imm), None, Some(op)) = (imm_of(left), imm_of(right), flipped(op))
                    {
                        let right_reg = self.compile_expression(right)?;
                        let dst_reg = self.register_allocator.allocate_register();
                        self.emitter.instructions.push(Instruction::BinImm {
                            op,
                            dst: dst_reg,
                            lhs: right_reg,
                            imm,
                            swapped: true,
                        });
                        return Ok(dst_reg);
                    }
                }

                let left_reg = self.compile_expression(left)?;
                let left_reg = self.shield_operand(left_reg, !Self::assignment_free(right));
                let right_reg = self.compile_expression(right)?;
                let dst_reg = self.register_allocator.allocate_register();

                match op {
                    BinaryOp::Add => self.emitter.emit_add(dst_reg, left_reg, right_reg),
                    BinaryOp::Subtract => self.emitter.emit_sub(dst_reg, left_reg, right_reg),
                    BinaryOp::Multiply => self.emitter.emit_mul(dst_reg, left_reg, right_reg),
                    BinaryOp::Divide => self.emitter.emit_div(dst_reg, left_reg, right_reg),
                    BinaryOp::Modulo => self.emitter.emit_mod(dst_reg, left_reg, right_reg),
                    BinaryOp::Equal => self.emitter.emit_eq(dst_reg, left_reg, right_reg),
                    BinaryOp::NotEqual => self.emitter.emit_ne(dst_reg, left_reg, right_reg),
                    BinaryOp::LessThan => self.emitter.emit_lt(dst_reg, left_reg, right_reg),
                    BinaryOp::LessThanEqual => self.emitter.emit_le(dst_reg, left_reg, right_reg),
                    BinaryOp::GreaterThan => self.emitter.emit_gt(dst_reg, left_reg, right_reg),
                    BinaryOp::GreaterThanEqual => {
                        self.emitter.emit_ge(dst_reg, left_reg, right_reg)
                    }
                    _ => {
                        return Err(BytecodeError::CompilationFailed(format!(
                            "Unsupported binary operator: {:?}",
                            op
                        )));
                    }
                }

                Ok(dst_reg)
            }

            Expr::List(elements) => {
                let exprs: Vec<&Expr> = elements.iter().collect();
                let element_regs = self.compile_operands(&exprs)?;

                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_make_list(dst_reg, element_regs);
                Ok(dst_reg)
            }

            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Compile condition
                let condition_reg = self.compile_expression(condition)?;

                // Create labels for branches
                let _then_label = self.emitter.create_label();
                let else_label = self.emitter.create_label();
                let end_label = self.emitter.create_label();

                // Branch on condition
                self.emitter.emit_branch_if_false(condition_reg, else_label);

                // Compile then branch
                let then_reg = self.compile_expression(then_branch)?;
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_move(dst_reg, then_reg);
                self.emitter.emit_jump(end_label);

                // Else branch
                self.emitter.place_label(else_label);
                if let Some(else_expr) = else_branch {
                    let else_reg = self.compile_expression(else_expr)?;
                    self.emitter.emit_move(dst_reg, else_reg);
                } else {
                    // No else branch, use unit
                    let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                    self.emitter.emit_load_const(dst_reg, const_idx);
                }

                self.emitter.place_label(end_label);
                Ok(dst_reg)
            }

            Expr::Range {
                start,
                end,
                inclusive,
            } => {
                // Compile start and end expressions
                let start_reg = self.compile_expression(start)?;
                let start_reg = self.shield_operand(start_reg, !Self::assignment_free(end));
                let end_reg = self.compile_expression(end)?;

                // Create range value - for now, we'll create a constant range
                // In a full implementation, this would handle dynamic ranges
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter
                    .emit_make_range(dst_reg, start_reg, end_reg, *inclusive);
                Ok(dst_reg)
            }

            Expr::Call { callee, arguments } => {
                // Callee resolution mirrors the interpreter's scope order. A
                // name held by a LOCAL is a function value and the call goes
                // through CallValue — so a parameter named `len` shadows the
                // builtin, exactly as interpreted. Otherwise the name
                // resolves through the registries (user functions shadow
                // builtins), then enum constructors, then the closure as a
                // baked value. Non-name callees — `f(x)(y)`, an immediately
                // invoked lambda, `obj.handler(x)` — compile as expressions
                // and call through CallValue.
                // The interpreter evaluates the callee (and a method-call
                // receiver) BEFORE the arguments — shield their registers
                // from argument code that assigns.
                let args_may_assign = arguments.iter().any(|a| {
                    let expr = match a {
                        crate::ast::Argument::Positional(e) => e,
                        crate::ast::Argument::Named { value, .. } => value,
                    };
                    !Self::assignment_free(expr)
                });
                let callee_reg: Option<Register> = match callee.as_ref() {
                    Expr::Identifier(name) | Expr::LocalRef { name, .. } => {
                        self.local_variables.get(name).copied()
                    }
                    // A stdlib module call like `math.sqrt(x)`: the object is
                    // a *bare* identifier (a shadowing local would be a
                    // LocalRef), so this is the real module when the
                    // synthesized name is in the builtin allow-list. A
                    // `value.m(..)` whose receiver is a plain LOCAL compiles
                    // to CallMethod, which mirrors trait dispatch at runtime
                    // (restricting to locals keeps the interpreter's
                    // receiver re-evaluation quirk unobservable). Anything
                    // else refuses.
                    Expr::FieldAccess { object, field } => match object.as_ref() {
                        Expr::Identifier(module)
                            if !self.local_variables.contains_key(module)
                                && self
                                    .builtin_names
                                    .contains(&format!("{}.{}", module, field)) =>
                        {
                            None
                        }
                        // Any OTHER native-module call: the module resolves in
                        // the closure to a Module struct, and the function is
                        // one of its Builtin fields. The builtin value carries
                        // its full dispatch name ("db.execute"), which is
                        // exactly what the interpreter itself calls through —
                        // so the bridge runs the same implementation with the
                        // same name. Existence is validated here at compile
                        // time; a call to a missing module function refuses
                        // (falls through) and stays interpreted.
                        Expr::Identifier(module)
                            if !self.local_variables.contains_key(module)
                                && matches!(
                                    self.enclosing_closure.get(module),
                                    Some(Value::Struct { type_name, fields })
                                        if type_name == "Module"
                                            && matches!(
                                                fields.get(field.as_str()),
                                                Some(Value::Builtin(_))
                                            )
                                ) =>
                        {
                            let builtin_name = match self.enclosing_closure.get(module) {
                                Some(Value::Struct { fields, .. }) => match fields.get(field.as_str()) {
                                    Some(Value::Builtin(b)) => b.name.clone(),
                                    _ => unreachable!("guard checked the field is a builtin"),
                                },
                                _ => unreachable!("guard checked the module"),
                            };
                            // caps.allowed("x") under a static manifest is
                            // a constant here too (see the fold below for
                            // the reasoning); this arm is the one a normal
                            // `caps.allowed(...)` call actually reaches.
                            if builtin_name == "caps.allowed"
                                && arguments.len() == 1
                                && let crate::ast::Argument::Positional(arg) = &arguments[0]
                                && let Expr::String(cap_name) = arg
                                && let Some(verdict) = self.fold_cap_allowed(cap_name)
                            {
                                let dst_reg = self.register_allocator.allocate_register();
                                let idx = self
                                    .emitter
                                    .add_constant(OvmValue::new_boolean(verdict));
                                self.emitter.instructions.push(Instruction::LoadConst {
                                    dst: dst_reg,
                                    const_idx: idx,
                                });
                                return Ok(dst_reg);
                            }
                            let arg_regs = self.compile_call_args(arguments)?;
                            let dst_reg = self.register_allocator.allocate_register();
                            let pregranted = self.pregrant(&builtin_name);
                            self.emitter.instructions.push(Instruction::CallNamed {
                                dst: dst_reg,
                                function_name: builtin_name,
                                args: arg_regs,
                                pregranted,
                            });
                            return Ok(dst_reg);
                        }
                        receiver if Self::receiver_is_pure(receiver) => {
                            let object_reg = self.compile_expression(object)?;
                            let object_reg = self.shield_operand(object_reg, args_may_assign);
                            let arg_regs = self.compile_call_args(arguments)?;
                            let dst_reg = self.register_allocator.allocate_register();
                            self.emitter.instructions.push(Instruction::CallMethod {
                                dst: dst_reg,
                                object: object_reg,
                                method: field.clone(),
                                args: arg_regs,
                            });
                            return Ok(dst_reg);
                        }
                        _ => {
                            return Err(BytecodeError::CompilationFailed(
                                "Method calls on effectful receiver expressions are not compiled in the bytecode tier"
                                    .to_string(),
                            ))
                        }
                    },
                    _ => Some(self.compile_expression(callee)?),
                };
                let callee_reg = callee_reg.map(|reg| self.shield_operand(reg, args_may_assign));

                let arg_regs = self.compile_call_args(arguments)?;

                if let Some(callee_reg) = callee_reg {
                    let dst_reg = self.register_allocator.allocate_register();
                    self.emitter.instructions.push(Instruction::CallValue {
                        dst: dst_reg,
                        callee: callee_reg,
                        args: arg_regs,
                    });
                    return Ok(dst_reg);
                }

                let function_name = match callee.as_ref() {
                    Expr::Identifier(name) | Expr::LocalRef { name, .. } => name.clone(),
                    Expr::FieldAccess { object, field } => match object.as_ref() {
                        Expr::Identifier(module) => format!("{}.{}", module, field),
                        _ => unreachable!("non-identifier objects take the value path"),
                    },
                    _ => unreachable!("non-name callees take the value path"),
                };

                // caps.allowed("x") under a static manifest is a constant:
                // the table is fixed for the run and grants attenuate by
                // this function's provenance, both known right here. The
                // branch it guards then folds, the denied side goes dead,
                // and the JIT stops refusing functions for code that can
                // never run. Unknown capability names keep the runtime
                // call — and its error — untouched.
                if function_name == "caps.allowed"
                    && arguments.len() == 1
                    && let crate::ast::Argument::Positional(arg) = &arguments[0]
                    && let Expr::String(cap_name) = arg
                    && let Some(verdict) = self.fold_cap_allowed(cap_name)
                {
                    let dst_reg = self.register_allocator.allocate_register();
                    let idx = self.emitter.add_constant(OvmValue::new_boolean(verdict));
                    self.emitter.instructions.push(Instruction::LoadConst {
                        dst: dst_reg,
                        const_idx: idx,
                    });
                    return Ok(dst_reg);
                }

                let dst_reg = self.register_allocator.allocate_register();
                // A nested fn calling ITSELF: CallFn to its own id, with the
                // body's capture parameters appended so recursion keeps its
                // captures.
                if let Some((self_name, self_id, real_params, captures)) = &self.self_call
                    && *self_name == function_name
                {
                    let mut full_args = arg_regs;
                    for i in 0..*captures {
                        full_args.push(Register((real_params + i) as u32));
                    }
                    self.emitter.instructions.push(Instruction::CallFn {
                        dst: dst_reg,
                        func_id: *self_id,
                        args: full_args,
                        arg_moves: 0,
                    });
                    return Ok(dst_reg);
                }
                // User functions shadow builtins (same order as the runtime
                // path); resolving the id here removes the per-call name hash.
                // But the *closure* shadows the registry when it holds a
                // DIFFERENT function under this name — lexical scope is
                // the runtime rule, and a by-value compile (a rebuilt
                // lambda in the bridge) may close over one module's
                // private `insert` while the registry's last-noted
                // `insert` came from another. When closure and registry
                // agree (the overwhelmingly common case: the closure
                // snapshot simply contains the global), the registry's
                // direct CallFn stays.
                let closure_disagrees = match self.enclosing_closure.get(&function_name) {
                    Some(Value::Function(f)) => self
                        .known_function_values
                        .get(&function_name)
                        .is_none_or(|known| !std::sync::Arc::ptr_eq(&known.body, &f.body)),
                    Some(_) => true,
                    None => false,
                };
                if !closure_disagrees
                    && let Some(&func_id) = self.function_registry.get(&function_name)
                {
                    self.emitter.instructions.push(Instruction::CallFn {
                        dst: dst_reg,
                        func_id,
                        args: arg_regs,
                        arg_moves: 0,
                    });
                    return Ok(dst_reg);
                }
                if self.builtin_names.contains(&function_name) {
                    if let Some(builtin_id) =
                        BytecodeVm::float_math_id(&function_name, arg_regs.len())
                    {
                        self.emitter.instructions.push(Instruction::CallBuiltin {
                            dst: dst_reg,
                            builtin_id,
                            args: arg_regs,
                        });
                    } else {
                        let pregranted = self.pregrant(&function_name);
                        self.emitter.instructions.push(Instruction::CallNamed {
                            dst: dst_reg,
                            function_name,
                            args: arg_regs,
                            pregranted,
                        });
                    }
                    return Ok(dst_reg);
                }
                // An enum tuple-variant constructor from the closure —
                // `Circle(2.0)`. An argument-count mismatch refuses, and the
                // interpreter raises its arity error.
                if let Some(Value::EnumConstructor {
                    type_name,
                    variant_name,
                    arity,
                }) = self.enclosing_closure.get(&function_name)
                {
                    if *arity != arguments.len() {
                        return Err(BytecodeError::UnresolvedCallee(function_name));
                    }
                    self.emitter.instructions.push(Instruction::MakeEnum {
                        dst: dst_reg,
                        type_name: type_name.clone(),
                        variant_name: variant_name.clone(),
                        args: arg_regs,
                    });
                    return Ok(dst_reg);
                }
                // Last resort: the closure. A function bearing THIS name is
                // a user function the tier hasn't registered yet — report
                // UnresolvedCallee so the tier's dependency resolution
                // compiles it and retries (that channel is what makes
                // transitive and mutual recursion promote both functions).
                // Anything else — an alias holding a differently-named
                // function, a lambda, a non-callable — bakes as a value and
                // calls through CallValue, whose interpreter fallback owns
                // the error semantics.
                match self.enclosing_closure.get(&function_name) {
                    Some(Value::Function(f))
                        if f.name.as_deref() == Some(function_name.as_str()) =>
                    {
                        Err(BytecodeError::UnresolvedCallee(function_name))
                    }
                    Some(_) => {
                        let baked =
                            self.compile_expression(&Expr::Identifier(function_name.clone()))?;
                        self.emitter.instructions.push(Instruction::CallValue {
                            dst: dst_reg,
                            callee: baked,
                            args: arg_regs,
                        });
                        Ok(dst_reg)
                    }
                    None => Err(BytecodeError::UnresolvedCallee(function_name)),
                }
            }

            Expr::Block(statements) => {
                // A block evaluates its statements in order; its value is the
                // value of the last statement (Unit for an empty block).
                //
                // A block also scopes its bindings, exactly as the
                // interpreter does: names bound inside are dropped at the
                // end, so an inner `let x` shadows an outer `x` for the
                // block and no longer after it. Restoring the name→register
                // map is enough — every binding allocates a fresh register,
                // so the outer name still points at its own untouched
                // register.
                let saved_locals = self.local_variables.clone();
                let mut result_reg = None;
                for statement in statements.iter() {
                    result_reg = Some(self.compile_statement(statement)?);
                }
                self.local_variables = saved_locals;
                match result_reg {
                    Some(reg) => Ok(reg),
                    None => {
                        let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                        let dst_reg = self.register_allocator.allocate_register();
                        self.emitter.emit_load_const(dst_reg, const_idx);
                        Ok(dst_reg)
                    }
                }
            }

            // Struct literals validate against the declared field set at
            // compile time with the interpreter's exact rules (unknown
            // type, missing field, surprise field). A literal that would
            // fail refuses compilation, so the function stays interpreted
            // and the interpreter raises its own error — never a divergent
            // one. Field values compile in literal order, preserving
            // side-effect order. Declared field types travel with the
            // instruction so MakeStruct enforces them at run time, exactly as
            // the interpreter does.
            Expr::StructLiteral(literal) => {
                let declared = self.struct_defs.get(&literal.type_name).ok_or_else(|| {
                    BytecodeError::CompilationFailed(format!(
                        "struct type '{}' is unknown to the bytecode tier (undeclared, or                          redeclared with a different shape)",
                        literal.type_name
                    ))
                })?;
                let given: Vec<&String> = literal.fields.iter().map(|f| &f.name).collect();
                for required in declared {
                    if !given.contains(&required) {
                        return Err(BytecodeError::CompilationFailed(format!(
                            "struct '{}' literal is missing field '{}'",
                            literal.type_name, required
                        )));
                    }
                }
                for name in &given {
                    if !declared.contains(name) {
                        return Err(BytecodeError::CompilationFailed(format!(
                            "struct '{}' literal has surprise field '{}'",
                            literal.type_name, name
                        )));
                    }
                }

                // Field exprs compile in literal order (side-effect order);
                // the instruction stores their registers in SHAPE order.
                let exprs: Vec<&Expr> = literal.fields.iter().map(|f| &f.value).collect();
                let regs = self.compile_operands(&exprs)?;
                let mut pairs: Vec<(String, Register)> = literal
                    .fields
                    .iter()
                    .zip(regs)
                    .map(|(f, reg)| (f.name.clone(), reg))
                    .collect();
                let shape = crate::ovm::value::intern_shape(
                    &literal.type_name,
                    pairs.iter().map(|(n, _)| n.clone()).collect(),
                );
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                // Field types in shape order, parallel to field_regs. Fields
                // with an unenforceable annotation carry `None` and stay
                // dynamic.
                let field_types: std::sync::Arc<[Option<crate::ast::FieldTypeCheck>]> = {
                    let checks = self.struct_field_checks.get(&literal.type_name);
                    pairs
                        .iter()
                        .map(|(name, _)| checks.and_then(|c| c.get(name)).cloned())
                        .collect()
                };
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::MakeStruct {
                    dst: dst_reg,
                    shape,
                    field_regs: pairs.into_iter().map(|(_, r)| r).collect(),
                    field_types,
                });
                Ok(dst_reg)
            }

            // Anonymous objects are free-form: no validation, type name
            // "Object", exactly as the interpreter builds them.
            Expr::AnonymousObject { fields } => {
                let exprs: Vec<&Expr> = fields.iter().map(|f| &f.value).collect();
                let regs = self.compile_operands(&exprs)?;
                let mut pairs: Vec<(String, Register)> = fields
                    .iter()
                    .zip(regs)
                    .map(|(f, reg)| (f.name.clone(), reg))
                    .collect();
                let shape = crate::ovm::value::intern_shape(
                    "Object",
                    pairs.iter().map(|(n, _)| n.clone()).collect(),
                );
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                // Anonymous objects declare no field types: nothing to check.
                let field_types: std::sync::Arc<[Option<crate::ast::FieldTypeCheck>]> =
                    pairs.iter().map(|_| None).collect();
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::MakeStruct {
                    dst: dst_reg,
                    shape,
                    field_regs: pairs.into_iter().map(|(_, r)| r).collect(),
                    field_types,
                });
                Ok(dst_reg)
            }

            Expr::MapLiteral { entries } => {
                // Keys and values compile in written order, key before value
                // per entry — the interpreter's evaluation order.
                let mut flat = Vec::with_capacity(entries.len() * 2);
                for entry in entries {
                    flat.push(&entry.key);
                    flat.push(&entry.value);
                }
                let regs = self.compile_operands(&flat)?;
                let entry_regs: Vec<(Register, Register)> =
                    regs.chunks(2).map(|c| (c[0], c[1])).collect();
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::MakeMap {
                    dst: dst_reg,
                    entries: entry_regs,
                });
                Ok(dst_reg)
            }

            Expr::TemplateString { parts } => {
                // Interpolations compile in written order (side-effect
                // order); literal chunks ride in the instruction.
                let last_assigning = parts.iter().rposition(|p| {
                    matches!(p, crate::ast::TemplatePart::Interpolation(e)
                        if !Self::assignment_free(e))
                });
                let mut compiled = Vec::with_capacity(parts.len());
                for (i, part) in parts.iter().enumerate() {
                    match part {
                        crate::ast::TemplatePart::Literal(text) => {
                            compiled.push(TplPart::Literal(text.clone()));
                        }
                        crate::ast::TemplatePart::Interpolation(expr) => {
                            let reg = self.compile_expression(expr)?;
                            let reg =
                                self.shield_operand(reg, last_assigning.is_some_and(|j| i < j));
                            compiled.push(TplPart::Reg(reg));
                        }
                    }
                }
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::MakeTemplate {
                    dst: dst_reg,
                    parts: compiled,
                });
                Ok(dst_reg)
            }

            // A nested `fn` declaration is a named closure over the current
            // frame: compile it exactly as the equivalent lambda (runtime
            // captures via MakeClosure, declaration-closure constants baked)
            // and bind the name. A nested fn referencing ITSELF refuses
            // through the lambda machinery's bound-names rule, matching the
            // conservative treatment of names bound later.
            Expr::Lambda {
                parameters, body, ..
            } => self.compile_lambda(parameters, body, None),

            Expr::Tuple(items) => {
                // The empty tuple is Unit — `()` in source. The
                // interpreter reads it the same way, and a tier that
                // built a zero-element tuple here instead would disagree
                // with it on every `x == ()`.
                if items.is_empty() {
                    let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                    let dst_reg = self.register_allocator.allocate_register();
                    self.emitter.emit_load_const(dst_reg, const_idx);
                    return Ok(dst_reg);
                }
                let exprs: Vec<&Expr> = items.iter().collect();
                let element_regs = self.compile_operands(&exprs)?;
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::MakeTuple {
                    dst: dst_reg,
                    elements: element_regs,
                });
                Ok(dst_reg)
            }

            Expr::Pipeline { left, right } => {
                // `x |> f(a)` is `f(x, a)`; `x |> f` is `f(x)`
                let desugared = match right.as_ref() {
                    Expr::Call { callee, arguments } => {
                        let mut args = Vec::with_capacity(arguments.len() + 1);
                        args.push(crate::ast::Argument::Positional((**left).clone()));
                        args.extend(arguments.iter().cloned());
                        Expr::Call {
                            callee: callee.clone(),
                            arguments: args,
                        }
                    }
                    other => Expr::Call {
                        callee: Box::new(other.clone()),
                        arguments: vec![crate::ast::Argument::Positional((**left).clone())],
                    },
                };
                self.compile_expression(&desugared)
            }

            Expr::ResultOk(inner) | Expr::ResultErr(inner) => {
                let ok = matches!(expr, Expr::ResultOk(_));
                let value_reg = self.compile_expression(inner)?;
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::MakeResult {
                    dst: dst_reg,
                    value: value_reg,
                    ok,
                });
                Ok(dst_reg)
            }

            Expr::Match { value, arms } => {
                let scrutinee = self.compile_expression(value)?;
                // A failing arm's guard runs before the NEXT arm's pattern
                // test re-reads the scrutinee register — shield it from
                // guards that assign. (Arm bodies never precede a test.)
                let guards_free = arms.iter().all(|arm| {
                    arm.guard.as_deref().is_none_or(Self::assignment_free)
                        && Self::pattern_guards_assignment_free(&arm.pattern)
                });
                let scrutinee = self.shield_operand(scrutinee, !guards_free);
                let result_reg = self.register_allocator.allocate_register();
                let end_label = self.emitter.create_label();

                for arm in arms.iter() {
                    let next_arm = self.emitter.create_label();

                    // Pattern test; jumps to next_arm when it doesn't apply
                    self.compile_pattern_test(&arm.pattern, scrutinee, next_arm)?;

                    // A guard may live on the arm or inside a Guarded pattern
                    if let Some(guard) = &arm.guard {
                        let guard_reg = self.compile_expression(guard)?;
                        self.emitter.emit_branch_if_false(guard_reg, next_arm);
                    }

                    let body_reg = self.compile_expression(&arm.expression)?;
                    self.emitter.emit_move(result_reg, body_reg);
                    self.emitter.emit_jump(end_label);

                    self.emitter.place_label(next_arm);
                }

                // Falling past every arm is the interpreter's PatternMatchFailed
                self.emitter.instructions.push(Instruction::MatchFail);
                self.emitter.place_label(end_label);
                Ok(result_reg)
            }

            // `break value` changes what the loop evaluates to and `return`
            // unwinds the call — the bytecode loops don't model either;
            // refuse so the function stays on the interpreter (never diverge).
            Expr::Break(Some(_)) => Err(BytecodeError::CompilationFailed(
                "'break' with a value is not supported in the bytecode tier".to_string(),
            )),

            Expr::Return(_) => Err(BytecodeError::CompilationFailed(
                "'return' is not supported in the bytecode tier".to_string(),
            )),

            Expr::Break(None) => {
                let (_, break_target) = *self.loop_targets.last().ok_or_else(|| {
                    BytecodeError::CompilationFailed("'break' outside of a loop".to_string())
                })?;
                self.emitter.emit_jump(break_target);
                // Unreachable, but every expression must yield a register
                self.unit_register()
            }

            Expr::Continue => {
                let (continue_target, _) = *self.loop_targets.last().ok_or_else(|| {
                    BytecodeError::CompilationFailed("'continue' outside of a loop".to_string())
                })?;
                self.emitter.emit_jump(continue_target);
                self.unit_register()
            }

            Expr::ForLoop {
                variable,
                iterable,
                body,
            } => {
                // Iterate by index over a list or range, matching the
                // interpreter (which never materializes a range).
                // IterGet re-reads the source register EVERY pass, so a body
                // that reassigns the iterable variable must not share it.
                let source_reg = self.compile_expression(iterable)?;
                let source_reg = self.shield_operand(source_reg, !Self::assignment_free(body));

                let len_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::IterLen {
                    dst: len_reg,
                    src: source_reg,
                });

                let idx_reg = self.register_allocator.allocate_register();
                let zero = self.emitter.add_constant(OvmValue::new_integer(0));
                self.emitter.emit_load_const(idx_reg, zero);

                // The loop variable gets its own register, rebound each pass
                let var_reg = self.register_allocator.allocate_register();
                self.local_variables.insert(variable.clone(), var_reg);

                let loop_start = self.emitter.create_label();
                let loop_step = self.emitter.create_label();
                let loop_end = self.emitter.create_label();

                self.emitter.place_label(loop_start);
                let cond_reg = self.register_allocator.allocate_register();
                self.emitter.emit_lt(cond_reg, idx_reg, len_reg);
                self.emitter.emit_branch_if_false(cond_reg, loop_end);

                self.emitter.instructions.push(Instruction::IterGet {
                    dst: var_reg,
                    src: source_reg,
                    idx: idx_reg,
                });

                // `continue` jumps to the increment, not the test, so the
                // loop still advances
                self.loop_targets.push((loop_step, loop_end));
                let body_result = self.compile_expression(body);
                self.loop_targets.pop();
                body_result?;

                self.emitter.place_label(loop_step);
                self.emitter.instructions.push(Instruction::BinImm {
                    op: BinaryOp::Add,
                    dst: idx_reg,
                    lhs: idx_reg,
                    imm: OvmValue::new_integer(1),
                    swapped: false,
                });
                self.emitter.emit_jump(loop_start);

                self.emitter.place_label(loop_end);

                // For loops evaluate to Unit
                self.unit_register()
            }

            Expr::Assignment { target, value } => {
                let target_reg = match self.local_variables.get(target) {
                    Some(&reg) => reg,
                    None => {
                        return Err(BytecodeError::CompilationFailed(format!(
                            "Assignment to unresolved variable '{}' (globals not supported in bytecode tier)",
                            target
                        )));
                    }
                };
                // Fuse `x = col.set(x, i, v)` / `x = col.swap(x, i, j)`
                // into an in-place indexed write — the interpreter fuses
                // the identical shape, so the tiers stay observationally
                // identical including the O(1) cost. Guards mirror the
                // interpreter's: the real builtin (an unshadowed `col`),
                // three positional args naming the target first, and
                // assignment-free index/value expressions.
                if let Expr::Call { callee, arguments } = value.as_ref()
                    && let Expr::FieldAccess { object, field } = callee.as_ref()
                    && matches!(object.as_ref(), Expr::Identifier(m)
                        if m == "col" && !self.local_variables.contains_key(m))
                    && matches!(field.as_str(), "set" | "swap")
                    && self.builtin_names.contains(&format!("col.{}", field))
                    && arguments.len() == 3
                {
                    let exprs: Vec<&Expr> = arguments
                        .iter()
                        .filter_map(|a| match a {
                            crate::ast::Argument::Positional(e) => Some(e),
                            crate::ast::Argument::Named { .. } => None,
                        })
                        .collect();
                    let names_target = exprs.len() == 3
                        && (matches!(exprs[0], Expr::Identifier(n) if n == target)
                            || matches!(exprs[0], Expr::LocalRef { name, .. } if name == target));
                    if names_target
                        && Self::assignment_free(exprs[1])
                        && Self::assignment_free(exprs[2])
                    {
                        let a1 = self.compile_expression(exprs[1])?;
                        let a2 = self.compile_expression(exprs[2])?;
                        self.emitter.instructions.push(if field == "set" {
                            Instruction::ListSetAssign {
                                target: target_reg,
                                index: a1,
                                value: a2,
                            }
                        } else {
                            Instruction::ListSwapAssign {
                                target: target_reg,
                                i: a1,
                                j: a2,
                            }
                        });
                        return Ok(target_reg);
                    }
                }
                // Fuse the accumulate pattern `x = x + rhs` into AddAssign,
                // which appends in place when x holds the only reference to
                // its string. Only when the rhs provably contains no
                // assignment: fusing evaluates the rhs before reading x, so
                // an rhs that writes x would observe the wrong order.
                if let Expr::BinaryOp {
                    left,
                    op: crate::ast::BinaryOp::Add,
                    right,
                } = value.as_ref()
                    && (matches!(left.as_ref(),
                        Expr::Identifier(n) if n == target)
                        || matches!(left.as_ref(),
                            Expr::LocalRef { name, .. } if name == target))
                    && Self::assignment_free(right)
                {
                    let rhs_reg = self.compile_expression(right)?;
                    self.emitter.instructions.push(Instruction::AddAssign {
                        target: target_reg,
                        rhs: rhs_reg,
                    });
                    return Ok(target_reg);
                }
                // `x = f(x, ...)`: pass x by move — TakeMove into a temp
                // that becomes argument 0, so the callee's frame holds
                // the only reference and its in-place writes stay in
                // place. Mirrors the interpreter's move-call fusion; the
                // remaining arguments must be assignment-free for the
                // same evaluation-order reason.
                if let Expr::Call { callee, arguments } = value.as_ref()
                    && matches!(callee.as_ref(), Expr::Identifier(_) | Expr::LocalRef { .. })
                    && !arguments.is_empty()
                {
                    if std::env::var_os("OLANG_DEBUG_TAKEMOVE").is_some() {
                        eprintln!("[takemove] candidate: {} = call", target);
                    }
                    let mut exprs = Vec::with_capacity(arguments.len());
                    for a in arguments {
                        match a {
                            crate::ast::Argument::Positional(e) => exprs.push(e),
                            crate::ast::Argument::Named { .. } => {
                                exprs.clear();
                                break;
                            }
                        }
                    }
                    let names_target = !exprs.is_empty()
                        && (matches!(exprs[0], Expr::Identifier(n) if n == target)
                            || matches!(exprs[0], Expr::LocalRef { name, .. } if name == target));
                    if names_target
                        && exprs[1..].iter().all(|e| Self::assignment_free(e))
                        && !self.local_variables.contains_key("__moved_arg0__")
                    {
                        if std::env::var_os("OLANG_DEBUG_TAKEMOVE").is_some() {
                            eprintln!("[takemove] FIRES for {}", target);
                        }
                        let tmp = self.register_allocator.allocate_register();
                        self.emitter.instructions.push(Instruction::TakeMove {
                            dst: tmp,
                            src: target_reg,
                        });
                        self.local_variables
                            .insert("__moved_arg0__".to_string(), tmp);
                        let mut moved_args = arguments.clone();
                        moved_args[0] = crate::ast::Argument::Positional(Expr::Identifier(
                            "__moved_arg0__".to_string(),
                        ));
                        let rewritten = Expr::Call {
                            callee: callee.clone(),
                            arguments: moved_args,
                        };
                        let value_reg = self.compile_expression(&rewritten);
                        self.local_variables.remove("__moved_arg0__");
                        let value_reg = value_reg?;
                        if value_reg != target_reg {
                            self.emitter.emit_move(target_reg, value_reg);
                        }
                        return Ok(target_reg);
                    }
                }
                let value_reg = self.compile_expression(value)?;
                if value_reg != target_reg {
                    self.emitter.emit_move(target_reg, value_reg);
                }
                Ok(target_reg)
            }

            Expr::WhileLoop { condition, body } => {
                let loop_start = self.emitter.create_label();
                let loop_end = self.emitter.create_label();

                self.emitter.place_label(loop_start);
                let condition_reg = self.compile_expression(condition)?;
                self.emitter.emit_branch_if_false(condition_reg, loop_end);

                // `continue` re-tests the condition; `break` exits
                self.loop_targets.push((loop_start, loop_end));
                let body_result = self.compile_expression(body);
                self.loop_targets.pop();
                body_result?;

                self.emitter.emit_jump(loop_start);

                self.emitter.place_label(loop_end);

                // While loops evaluate to Unit
                let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                let dst_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(dst_reg, const_idx);
                Ok(dst_reg)
            }

            Expr::FieldAccess { object, field } => {
                let object_reg = self.compile_expression(object)?;
                let name_const = self
                    .emitter
                    .add_constant(OvmValue::new_string(field.clone()));
                let dst = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::GetField {
                    dst,
                    object: object_reg,
                    name_const,
                    cache: FieldCache::default(),
                });
                Ok(dst)
            }

            Expr::Index { object, index } => {
                let object_reg = self.compile_expression(object)?;
                let object_reg = self.shield_operand(object_reg, !Self::assignment_free(index));
                let index_reg = self.compile_expression(index)?;
                let dst = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::IndexGet {
                    dst,
                    object: object_reg,
                    index: index_reg,
                });
                Ok(dst)
            }

            other => {
                // Refuse to compile unsupported expressions — substituting a
                // Unit constant (e.g. for a recursive call site) silently
                // changed program results on promotion to the bytecode tier
                Err(BytecodeError::CompilationFailed(format!(
                    "Unsupported expression in bytecode tier: {:?}",
                    std::mem::discriminant(other)
                )))
            }
        }
    }

    /// Emit a test for `pattern` against `value_reg`, jumping to `fail_label`
    /// when it does not apply. Identifier patterns bind on the success path.
    ///
    /// Only the non-destructuring subset is supported; destructuring patterns
    /// (Ok/Err, lists, tuples, structs, enums) are rejected so the function
    /// stays on the interpreter rather than being miscompiled.
    /// Compile a lambda or a named nested fn to a function value. With
    /// `self_name`, the name is a self-binding: skipped in free-variable
    /// analysis, bound to the pending body's own id during its compile
    /// (so recursion is CallFn), and carried on the escaped AST form so
    /// interpreted copies recurse through call-time self-definition.
    fn compile_lambda(
        &mut self,
        parameters: &[crate::ast::Parameter],
        body: &Expr,
        self_name: Option<&str>,
    ) -> Result<Register, BytecodeError> {
        // Only lambdas that reference nothing but their own parameters.
        // Such a lambda is a compile-time constant: with no free
        // variables, an empty closure is equivalent to whatever the
        // interpreter would have captured.
        if parameters.iter().any(|p| p.default_value.is_some()) {
            return Err(BytecodeError::CompilationFailed(
                "Lambda with default parameter values is not supported in the bytecode tier"
                    .to_string(),
            ));
        }

        let bound: std::collections::HashSet<String> =
            parameters.iter().map(|p| p.name.clone()).collect();
        let mut free = std::collections::HashSet::new();
        if !Self::collect_free_vars(body, &bound, &mut free) {
            return Err(BytecodeError::CompilationFailed(
                "Lambda body uses constructs the bytecode tier cannot analyze".to_string(),
            ));
        }

        // Free variables split three ways. A name currently living
        // in a register (a parameter or an already-bound local) is a
        // RUNTIME capture: its value is snapshotted at the lambda
        // expression by MakeClosure, the interpreter's own
        // capture-by-value moment. A name in the declaration-time
        // closure resolves identically in both tiers and is baked
        // into the template. A name the enclosing function binds
        // only LATER (in scope for the interpreter's resolution
        // rules but with no register yet) refuses compilation.
        let mut runtime_captures: Vec<(String, Register)> = Vec::new();
        for name in &free {
            if Some(name.as_str()) == self_name {
                // The nested fn's own name: resolves to its own
                // compiled id (bound during the pending compile), and
                // the escaped form carries the name so interpreted
                // recursion works through call_function's
                // self-definition.
                continue;
            }
            if let Some(&reg) = self.local_variables.get(name) {
                runtime_captures.push((name.clone(), reg));
            } else if self.function_registry.contains_key(name) {
                // A known user function (forward or mutual recursion
                // through the lambda): the compiled body calls it
                // through the registry — but the lambda's AST form
                // must still carry the function in its closure, or an
                // escaped copy (bridged to a builtin, returned to
                // interpreted code) hits "Undefined variable". The
                // template example caught exactly that. Attached
                // below via known_function_values; a registered name
                // with no recorded value refuses.
                if !self.known_function_values.contains_key(name) {
                    return Err(BytecodeError::CompilationFailed(format!(
                        "Lambda references function '{}' with no recorded value",
                        name
                    )));
                }
            } else if self.enclosing_bound_names.contains(name) {
                return Err(BytecodeError::CompilationFailed(format!(
                    "Lambda captures '{}' before the enclosing function binds it",
                    name
                )));
            } else if !self.enclosing_closure.contains_key(name) {
                // Not a local, not registered, not in the closure. It
                // may still be a user function declared LATER (mutual
                // recursion through the lambda): report it as an
                // unresolved callee so the tier's dependency
                // resolution can register and compile it, then retry.
                // A name that isn't a known function rejects there.
                return Err(BytecodeError::UnresolvedCallee(name.clone()));
            }
        }
        // Deterministic capture order regardless of hash iteration
        runtime_captures.sort_by(|a, b| a.0.cmp(&b.0));

        // Attach only the entries the lambda actually references.
        // Attaching the full closure would defeat call_function's
        // empty-closure fast path: every call of a trivial lambda
        // would materialize the entire prelude into its environment.
        let captured: im::HashMap<String, Value> = free
            .iter()
            .filter(|name| !runtime_captures.iter().any(|(n, _)| n == *name))
            .filter_map(|name| {
                if let Some(value) = self.enclosing_closure.get(name) {
                    return Some((name.clone(), value.clone()));
                }
                // Registry-resolved function: carried as a value so
                // the escaped lambda resolves it interpreted too
                self.known_function_values
                    .get(name)
                    .map(|f| (name.clone(), Value::Function(f.clone())))
            })
            .collect();

        let function = crate::ast::Function {
            name: self_name.map(|n| n.to_string()),
            param_checks: crate::ast::param_checks_of(parameters, &[]),
            return_check: None,
            parameters: parameters.to_vec(),
            body: std::sync::Arc::new(body.clone()),
            // Values cloned from the enclosing snapshot — free
            // variables resolve identically in both tiers
            closure: std::sync::Arc::new(captured.clone()),
            param_bounds: Vec::new(),
            // The bytecode tier isn't instrumented for coverage (that runs
            // on the interpreter), so no def_file is threaded here.
            def_file: None,
        };

        if runtime_captures.is_empty() && self_name.is_none() {
            // No runtime state: the lambda is a compile-time constant
            let const_idx = self
                .emitter
                .add_constant(OvmValue::new_ast_function(function));
            let dst_reg = self.register_allocator.allocate_register();
            self.emitter.emit_load_const(dst_reg, const_idx);
            return Ok(dst_reg);
        }

        // Runtime captures: compile the body once as a standalone
        // function whose trailing parameters are the captured names
        // (call with [args..., captures...]), deferred to the VM
        // because the compiler's per-function state cannot nest.
        let capture_names: Vec<String> = runtime_captures.iter().map(|(n, _)| n.clone()).collect();
        let capture_regs: Vec<Register> = runtime_captures.iter().map(|(_, r)| *r).collect();

        let mut hidden_params = parameters.to_vec();
        for name in &capture_names {
            hidden_params.push(crate::ast::Parameter {
                name: name.clone(),
                type_annotation: None,
                default_value: None,
            });
        }
        let lambda_id = FunctionId::new();
        self.pending_lambdas.push((
            lambda_id,
            FunctionDecl {
                name_span: None,
                name: self_name.unwrap_or("<lambda>").to_string(),
                type_params: Vec::new(),
                type_param_bounds: Vec::new(),
                parameters: hidden_params,
                return_type: None,
                body: body.clone(),
            },
            std::sync::Arc::new(captured),
            self_name.map(|n| (n.to_string(), capture_names.len())),
        ));

        let template = crate::ovm::value::ClosureObject {
            template: function,
            capture_names,
            captured: Vec::new(),
            func_id: lambda_id,
        };
        let template_const = self
            .emitter
            .add_constant(OvmValue::new_closure(Arc::new(template)));
        let dst_reg = self.register_allocator.allocate_register();
        self.emitter.instructions.push(Instruction::MakeClosure {
            dst: dst_reg,
            template_const,
            captures: capture_regs,
        });
        Ok(dst_reg)
    }

    fn compile_pattern_test(
        &mut self,
        pattern: &crate::ast::Pattern,
        value_reg: Register,
        fail_label: Label,
    ) -> Result<(), BytecodeError> {
        use crate::ast::Pattern;

        match pattern {
            Pattern::Wildcard => Ok(()),

            Pattern::Identifier(name) => {
                // A bare name that is a declared unit enum variant is an
                // equality match, not a binding — mirroring the interpreter,
                // which checks (in order) that the name is a declared unit
                // variant AND resolves to a unit enum in the current scope.
                // A local shadowing the name makes it a binding again; a
                // variant declared after this function (absent from the
                // closure, resolvable only through the caller's runtime
                // scope) refuses, because no snapshot can answer it.
                if !self.local_variables.contains_key(name)
                    && self.unit_variant_names.contains(name)
                {
                    match self.enclosing_closure.get(name) {
                        Some(
                            variant @ Value::Enum {
                                variant_data: crate::ast::EnumVariantData::Unit,
                                ..
                            },
                        ) => {
                            let const_idx = self
                                .emitter
                                .add_constant(OvmValue::from_ast(variant.clone()));
                            let const_reg = self.register_allocator.allocate_register();
                            self.emitter.emit_load_const(const_reg, const_idx);
                            let test_reg = self.register_allocator.allocate_register();
                            self.emitter.instructions.push(Instruction::PatternEq {
                                dst: test_reg,
                                value: value_reg,
                                other: const_reg,
                            });
                            self.emitter.emit_branch_if_false(test_reg, fail_label);
                            return Ok(());
                        }
                        Some(_) => {} // shadowed by a non-variant: binds
                        None => {
                            return Err(BytecodeError::CompilationFailed(format!(
                                "unit variant '{}' resolves through runtime scope, not the closure",
                                name
                            )));
                        }
                    }
                }
                // Bind the name to its own register so later assignment to it
                // doesn't clobber the scrutinee
                let var_reg = self.register_allocator.allocate_register();
                self.emitter.emit_move(var_reg, value_reg);
                self.local_variables.insert(name.clone(), var_reg);
                Ok(())
            }

            Pattern::Literal(literal) => {
                if !BytecodeVm::round_trips(literal) {
                    return Err(BytecodeError::CompilationFailed(
                        "Unsupported literal pattern in bytecode tier".to_string(),
                    ));
                }
                let const_idx = self
                    .emitter
                    .add_constant(OvmValue::from_ast(literal.clone()));
                let const_reg = self.register_allocator.allocate_register();
                self.emitter.emit_load_const(const_reg, const_idx);

                let test_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::PatternEq {
                    dst: test_reg,
                    value: value_reg,
                    other: const_reg,
                });
                self.emitter.emit_branch_if_false(test_reg, fail_label);
                Ok(())
            }

            Pattern::Range {
                start,
                end,
                inclusive,
            } => {
                let bound = |p: &Pattern| match p {
                    Pattern::Literal(Value::Integer(n)) => Some(*n),
                    _ => None,
                };
                match (bound(start), bound(end)) {
                    (Some(lo), Some(hi)) => {
                        let test_reg = self.register_allocator.allocate_register();
                        self.emitter.instructions.push(Instruction::PatternInRange {
                            dst: test_reg,
                            value: value_reg,
                            lo,
                            hi,
                            inclusive: *inclusive,
                        });
                        self.emitter.emit_branch_if_false(test_reg, fail_label);
                        Ok(())
                    }
                    // Character ranges and anything non-literal stay interpreted
                    _ => Err(BytecodeError::CompilationFailed(
                        "Unsupported range pattern in bytecode tier".to_string(),
                    )),
                }
            }

            Pattern::Or { alternatives } => {
                // Alternatives may not bind, so that the success path has the
                // same bindings whichever one matched
                if alternatives.iter().any(Self::pattern_binds) {
                    return Err(BytecodeError::CompilationFailed(
                        "Or-patterns that bind variables are not supported in the bytecode tier"
                            .to_string(),
                    ));
                }

                let matched = self.emitter.create_label();
                for alternative in alternatives {
                    let try_next = self.emitter.create_label();
                    self.compile_pattern_test(alternative, value_reg, try_next)?;
                    self.emitter.emit_jump(matched);
                    self.emitter.place_label(try_next);
                }
                self.emitter.emit_jump(fail_label);
                self.emitter.place_label(matched);
                Ok(())
            }

            Pattern::Guarded { pattern, guard } => {
                self.compile_pattern_test(pattern, value_reg, fail_label)?;
                let guard_reg = self.compile_expression(guard)?;
                self.emitter.emit_branch_if_false(guard_reg, fail_label);
                Ok(())
            }

            Pattern::Ok(inner) | Pattern::Err(inner) => {
                let want_ok = matches!(pattern, Pattern::Ok(_));

                let test_reg = self.register_allocator.allocate_register();
                self.emitter
                    .instructions
                    .push(Instruction::PatternTestResult {
                        dst: test_reg,
                        value: value_reg,
                        want_ok,
                    });
                self.emitter.emit_branch_if_false(test_reg, fail_label);

                // Safe to extract now: the test above guarantees the shape
                let payload_reg = self.register_allocator.allocate_register();
                self.emitter.instructions.push(Instruction::ExtractResult {
                    dst: payload_reg,
                    value: value_reg,
                    want_ok,
                });
                self.compile_pattern_test(inner, payload_reg, fail_label)
            }

            Pattern::Tuple(patterns) => {
                let test_reg = self.register_allocator.allocate_register();
                self.emitter
                    .instructions
                    .push(Instruction::PatternTestTuple {
                        dst: test_reg,
                        value: value_reg,
                        len: patterns.len(),
                    });
                self.emitter.emit_branch_if_false(test_reg, fail_label);

                for (index, element) in patterns.iter().enumerate() {
                    let element_reg = self.register_allocator.allocate_register();
                    self.emitter.instructions.push(Instruction::ExtractElement {
                        dst: element_reg,
                        value: value_reg,
                        index,
                    });
                    self.compile_pattern_test(element, element_reg, fail_label)?;
                }
                Ok(())
            }

            Pattern::List { patterns, rest } => {
                // Without a rest binding the length must match exactly;
                // with one, the explicit patterns are a prefix
                let test_reg = self.register_allocator.allocate_register();
                self.emitter
                    .instructions
                    .push(Instruction::PatternTestList {
                        dst: test_reg,
                        value: value_reg,
                        min_len: patterns.len(),
                        exact: rest.is_none(),
                    });
                self.emitter.emit_branch_if_false(test_reg, fail_label);

                for (index, element) in patterns.iter().enumerate() {
                    let element_reg = self.register_allocator.allocate_register();
                    self.emitter.instructions.push(Instruction::ExtractElement {
                        dst: element_reg,
                        value: value_reg,
                        index,
                    });
                    self.compile_pattern_test(element, element_reg, fail_label)?;
                }

                if let Some(rest_name) = rest {
                    let rest_reg = self.register_allocator.allocate_register();
                    self.emitter.instructions.push(Instruction::ExtractRest {
                        dst: rest_reg,
                        value: value_reg,
                        from: patterns.len(),
                    });
                    self.local_variables.insert(rest_name.clone(), rest_reg);
                }
                Ok(())
            }

            Pattern::EnumVariant {
                variant_name,
                patterns,
            } => {
                // Test first (variant + payload arity, with the interpreter's
                // legacy plain-tuple acceptance), then extract each payload
                // element positionally — struct-variant payloads in
                // field-name order, exactly as the interpreter matches them.
                let test_reg = self.register_allocator.allocate_register();
                self.emitter
                    .instructions
                    .push(Instruction::PatternTestEnum {
                        dst: test_reg,
                        value: value_reg,
                        variant_name: variant_name.clone(),
                        pattern_count: patterns.len(),
                    });
                self.emitter.emit_branch_if_false(test_reg, fail_label);

                for (index, sub) in patterns.iter().enumerate() {
                    let payload_reg = self.register_allocator.allocate_register();
                    self.emitter
                        .instructions
                        .push(Instruction::ExtractEnumPayload {
                            dst: payload_reg,
                            value: value_reg,
                            index,
                        });
                    self.compile_pattern_test(sub, payload_reg, fail_label)?;
                }
                Ok(())
            }

            // Struct patterns: each named field must exist and its
            // subpattern match, checked in written order with short-circuit.
            // The interpreter ignores the pattern's type name and tolerates
            // extra fields in the value; so does this.
            Pattern::Struct { field_patterns, .. }
            | Pattern::AnonymousStruct { field_patterns } => {
                for (field_name, sub) in field_patterns {
                    let test_reg = self.register_allocator.allocate_register();
                    self.emitter
                        .instructions
                        .push(Instruction::PatternTestStructField {
                            dst: test_reg,
                            value: value_reg,
                            field_name: field_name.clone(),
                        });
                    self.emitter.emit_branch_if_false(test_reg, fail_label);

                    let name_const = self
                        .emitter
                        .add_constant(OvmValue::new_string(field_name.clone()));
                    let field_reg = self.register_allocator.allocate_register();
                    self.emitter.instructions.push(Instruction::GetField {
                        dst: field_reg,
                        object: value_reg,
                        name_const,
                        cache: FieldCache::default(),
                    });
                    self.compile_pattern_test(sub, field_reg, fail_label)?;
                }
                Ok(())
            }

            other => Err(BytecodeError::CompilationFailed(format!(
                "Unsupported pattern in bytecode tier: {:?}",
                std::mem::discriminant(other)
            ))),
        }
    }

    /// Collect every name `expr` binds or assigns, at any depth. Used to
    /// detect lambda free variables that would capture the enclosing
    /// function's runtime state rather than its declaration-time closure.
    /// Over-collection is safe (more rejections); under-collection is not.
    fn collect_bound_names(expr: &Expr, names: &mut std::collections::HashSet<String>) {
        use crate::ast::Statement;

        match expr {
            Expr::Assignment { target, value } => {
                names.insert(target.clone());
                Self::collect_bound_names(value, names);
            }
            Expr::LocalAssign { name, value, .. } => {
                names.insert(name.clone());
                Self::collect_bound_names(value, names);
            }
            Expr::Block(statements) => {
                for statement in statements {
                    let statement = statement.unwrapped();
                    match statement {
                        Statement::Expression(e) => Self::collect_bound_names(e, names),
                        Statement::LetDecl(decl) => {
                            Self::pattern_binding_names(&decl.pattern, names);
                            if let Some(value) = &decl.value {
                                Self::collect_bound_names(value, names);
                            }
                        }
                        Statement::FunctionDecl(decl) => {
                            names.insert(decl.name.clone());
                            for p in &decl.parameters {
                                names.insert(p.name.clone());
                            }
                            Self::collect_bound_names(&decl.body, names);
                        }
                        _ => {}
                    }
                }
            }
            Expr::ForLoop {
                variable,
                iterable,
                body,
            } => {
                names.insert(variable.clone());
                Self::collect_bound_names(iterable, names);
                Self::collect_bound_names(body, names);
            }
            Expr::Match { value, arms } => {
                Self::collect_bound_names(value, names);
                for arm in arms.iter() {
                    Self::pattern_binding_names(&arm.pattern, names);
                    if let Some(guard) = &arm.guard {
                        Self::collect_bound_names(guard, names);
                    }
                    Self::collect_bound_names(&arm.expression, names);
                }
            }
            Expr::Lambda {
                parameters, body, ..
            } => {
                for param in parameters {
                    names.insert(param.name.clone());
                }
                Self::collect_bound_names(body, names);
            }
            Expr::BinaryOp { left, right, .. } | Expr::BitwiseOp { left, right, .. } => {
                Self::collect_bound_names(left, names);
                Self::collect_bound_names(right, names);
            }
            Expr::UnaryOp { operand, .. } => Self::collect_bound_names(operand, names),
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::collect_bound_names(condition, names);
                Self::collect_bound_names(then_branch, names);
                if let Some(e) = else_branch {
                    Self::collect_bound_names(e, names);
                }
            }
            Expr::WhileLoop { condition, body } => {
                Self::collect_bound_names(condition, names);
                Self::collect_bound_names(body, names);
            }
            Expr::Loop { body } => Self::collect_bound_names(body, names),
            Expr::Call { callee, arguments } => {
                Self::collect_bound_names(callee, names);
                for argument in arguments {
                    match argument {
                        crate::ast::Argument::Positional(e) => Self::collect_bound_names(e, names),
                        crate::ast::Argument::Named { value, .. } => {
                            Self::collect_bound_names(value, names)
                        }
                    }
                }
            }
            Expr::Pipeline { left, right } => {
                Self::collect_bound_names(left, names);
                Self::collect_bound_names(right, names);
            }
            Expr::List(items) => {
                for item in items.iter() {
                    Self::collect_bound_names(item, names);
                }
            }
            Expr::Tuple(items) => {
                for item in items.iter() {
                    Self::collect_bound_names(item, names);
                }
            }
            Expr::Range { start, end, .. } => {
                Self::collect_bound_names(start, names);
                Self::collect_bound_names(end, names);
            }
            Expr::Index { object, index } => {
                Self::collect_bound_names(object, names);
                Self::collect_bound_names(index, names);
            }
            Expr::ResultOk(inner) | Expr::ResultErr(inner) => {
                Self::collect_bound_names(inner, names)
            }
            Expr::FieldAccess { object, .. } => Self::collect_bound_names(object, names),
            Expr::MapLiteral { entries } => {
                for entry in entries {
                    Self::collect_bound_names(&entry.key, names);
                    Self::collect_bound_names(&entry.value, names);
                }
            }
            Expr::TemplateString { parts } => {
                for part in parts {
                    if let crate::ast::TemplatePart::Interpolation(e) = part {
                        Self::collect_bound_names(e, names);
                    }
                }
            }
            Expr::StructLiteral(literal) => {
                for field in &literal.fields {
                    Self::collect_bound_names(&field.value, names);
                }
            }
            Expr::AnonymousObject { fields } => {
                for field in fields {
                    Self::collect_bound_names(&field.value, names);
                }
            }
            // Leaves and forms with no binding constructs worth descending
            // into: anything unhandled compiles to a rejection elsewhere, so
            // missing names here cannot reach a compiled lambda.
            _ => {}
        }
    }

    /// Collect the free variables of `expr` into `free`, given `bound` names.
    ///
    /// Returns false for any expression form it does not explicitly
    /// understand — deliberately a whitelist, so an unfamiliar construct
    /// makes the enclosing lambda ineligible rather than compiled with a
    /// closure that cannot satisfy it.
    fn collect_free_vars(
        expr: &Expr,
        bound: &std::collections::HashSet<String>,
        free: &mut std::collections::HashSet<String>,
    ) -> bool {
        use crate::ast::Statement;

        match expr {
            Expr::Integer(_)
            | Expr::Float(_)
            | Expr::String(_)
            | Expr::RawString(_)
            | Expr::Boolean(_)
            | Expr::Break(None)
            | Expr::Continue => true,

            Expr::Identifier(name) | Expr::LocalRef { name, .. } => {
                if !bound.contains(name) {
                    free.insert(name.clone());
                }
                true
            }

            Expr::LocalAssign { name, value, .. } => {
                bound.contains(name) && Self::collect_free_vars(value, bound, free)
            }

            Expr::BinaryOp { left, right, .. } | Expr::BitwiseOp { left, right, .. } => {
                Self::collect_free_vars(left, bound, free)
                    && Self::collect_free_vars(right, bound, free)
            }
            Expr::UnaryOp { operand, .. } => Self::collect_free_vars(operand, bound, free),

            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::collect_free_vars(condition, bound, free)
                    && Self::collect_free_vars(then_branch, bound, free)
                    && else_branch
                        .as_ref()
                        .is_none_or(|e| Self::collect_free_vars(e, bound, free))
            }

            Expr::List(items) => items
                .iter()
                .all(|e| Self::collect_free_vars(e, bound, free)),
            Expr::Tuple(items) => items
                .iter()
                .all(|e| Self::collect_free_vars(e, bound, free)),

            Expr::Range { start, end, .. } => {
                Self::collect_free_vars(start, bound, free)
                    && Self::collect_free_vars(end, bound, free)
            }

            Expr::Index { object, index } => {
                Self::collect_free_vars(object, bound, free)
                    && Self::collect_free_vars(index, bound, free)
            }

            Expr::ResultOk(inner) | Expr::ResultErr(inner) => {
                Self::collect_free_vars(inner, bound, free)
            }

            Expr::FieldAccess { object, .. } => Self::collect_free_vars(object, bound, free),

            Expr::MapLiteral { entries } => entries.iter().all(|e| {
                Self::collect_free_vars(&e.key, bound, free)
                    && Self::collect_free_vars(&e.value, bound, free)
            }),

            Expr::TemplateString { parts } => parts.iter().all(|p| match p {
                crate::ast::TemplatePart::Literal(_) => true,
                crate::ast::TemplatePart::Interpolation(e) => {
                    Self::collect_free_vars(e, bound, free)
                }
            }),

            Expr::StructLiteral(literal) => literal
                .fields
                .iter()
                .all(|f| Self::collect_free_vars(&f.value, bound, free)),
            Expr::AnonymousObject { fields } => fields
                .iter()
                .all(|f| Self::collect_free_vars(&f.value, bound, free)),

            // Assignment target must be lambda-local: assigning to a closure
            // name would rely on write-through semantics we don't replicate
            Expr::Assignment { target, value } => {
                bound.contains(target) && Self::collect_free_vars(value, bound, free)
            }

            Expr::WhileLoop { condition, body } => {
                Self::collect_free_vars(condition, bound, free)
                    && Self::collect_free_vars(body, bound, free)
            }

            Expr::Loop { body } => Self::collect_free_vars(body, bound, free),

            Expr::ForLoop {
                variable,
                iterable,
                body,
            } => {
                if !Self::collect_free_vars(iterable, bound, free) {
                    return false;
                }
                let mut scope = bound.clone();
                scope.insert(variable.clone());
                Self::collect_free_vars(body, &scope, free)
            }

            Expr::Call { callee, arguments } => {
                Self::collect_free_vars(callee, bound, free)
                    && arguments.iter().all(|argument| match argument {
                        crate::ast::Argument::Positional(e) => {
                            Self::collect_free_vars(e, bound, free)
                        }
                        crate::ast::Argument::Named { value, .. } => {
                            Self::collect_free_vars(value, bound, free)
                        }
                    })
            }

            Expr::Pipeline { left, right } => {
                Self::collect_free_vars(left, bound, free)
                    && Self::collect_free_vars(right, bound, free)
            }

            // A nested lambda's parameters bind within it; the rest of its
            // free variables bubble up
            Expr::Lambda {
                parameters, body, ..
            } => {
                if parameters.iter().any(|p| p.default_value.is_some()) {
                    return false;
                }
                let mut scope = bound.clone();
                for param in parameters {
                    scope.insert(param.name.clone());
                }
                Self::collect_free_vars(body, &scope, free)
            }

            Expr::Match { value, arms } => {
                if !Self::collect_free_vars(value, bound, free) {
                    return false;
                }
                arms.iter().all(|arm| {
                    let mut scope = bound.clone();
                    Self::pattern_binding_names(&arm.pattern, &mut scope);
                    arm.guard
                        .as_ref()
                        .is_none_or(|g| Self::collect_free_vars(g, &scope, free))
                        && Self::collect_free_vars(&arm.expression, &scope, free)
                })
            }

            Expr::Block(statements) => {
                let mut scope = bound.clone();
                for statement in statements {
                    let statement = statement.unwrapped();
                    match statement {
                        Statement::Expression(e) => {
                            if !Self::collect_free_vars(e, &scope, free) {
                                return false;
                            }
                        }
                        Statement::LetDecl(decl) => {
                            if let Some(value) = &decl.value
                                && !Self::collect_free_vars(value, &scope, free)
                            {
                                return false;
                            }
                            Self::pattern_binding_names(&decl.pattern, &mut scope);
                        }
                        Statement::FunctionDecl(decl) => {
                            let mut inner = scope.clone();
                            for p in &decl.parameters {
                                inner.insert(p.name.clone());
                            }
                            if !Self::collect_free_vars(&decl.body, &inner, free) {
                                return false;
                            }
                            scope.insert(decl.name.clone());
                        }
                        _ => return false,
                    }
                }
                true
            }

            _ => false,
        }
    }

    /// Collect the names a pattern binds.
    fn pattern_binding_names(
        pattern: &crate::ast::Pattern,
        names: &mut std::collections::HashSet<String>,
    ) {
        use crate::ast::Pattern;
        match pattern {
            Pattern::Identifier(name) | Pattern::Rest(name) => {
                names.insert(name.clone());
            }
            Pattern::Ok(inner) | Pattern::Err(inner) => Self::pattern_binding_names(inner, names),
            Pattern::Tuple(patterns) => {
                for p in patterns {
                    Self::pattern_binding_names(p, names);
                }
            }
            Pattern::List { patterns, rest } => {
                for p in patterns {
                    Self::pattern_binding_names(p, names);
                }
                if let Some(rest_name) = rest {
                    names.insert(rest_name.clone());
                }
            }
            Pattern::Or { alternatives } => {
                for p in alternatives {
                    Self::pattern_binding_names(p, names);
                }
            }
            Pattern::Guarded { pattern, .. } => Self::pattern_binding_names(pattern, names),
            Pattern::EnumVariant { patterns, .. } => {
                for p in patterns {
                    Self::pattern_binding_names(p, names);
                }
            }
            Pattern::Struct { field_patterns, .. }
            | Pattern::AnonymousStruct { field_patterns } => {
                for (_, p) in field_patterns {
                    Self::pattern_binding_names(p, names);
                }
            }
            Pattern::Literal(_) | Pattern::Wildcard | Pattern::Range { .. } => {}
        }
    }

    /// Whether a pattern introduces bindings.
    fn pattern_binds(pattern: &crate::ast::Pattern) -> bool {
        use crate::ast::Pattern;
        match pattern {
            Pattern::Identifier(_) | Pattern::Rest(_) => true,
            Pattern::Or { alternatives } => alternatives.iter().any(Self::pattern_binds),
            Pattern::Guarded { pattern, .. } => Self::pattern_binds(pattern),
            Pattern::Ok(inner) | Pattern::Err(inner) => Self::pattern_binds(inner),
            Pattern::Tuple(patterns) => patterns.iter().any(Self::pattern_binds),
            Pattern::List { patterns, rest } => {
                rest.is_some() || patterns.iter().any(Self::pattern_binds)
            }
            Pattern::EnumVariant { patterns, .. } => patterns.iter().any(Self::pattern_binds),
            Pattern::Struct { field_patterns, .. }
            | Pattern::AnonymousStruct { field_patterns } => {
                field_patterns.iter().any(|(_, p)| Self::pattern_binds(p))
            }
            _ => false,
        }
    }

    /// True when re-evaluating the expression is unobservable — the
    /// interpreter's method-dispatch fallthrough re-evaluates the receiver,
    /// so only receivers in this set may compile to CallMethod. Field
    /// access can error, but re-evaluating reproduces the same error with
    /// no side effects.
    fn receiver_is_pure(expr: &Expr) -> bool {
        match expr {
            Expr::Identifier(_)
            | Expr::LocalRef { .. }
            | Expr::Integer(_)
            | Expr::Float(_)
            | Expr::String(_)
            | Expr::RawString(_)
            | Expr::Boolean(_) => true,
            Expr::FieldAccess { object, .. } => Self::receiver_is_pure(object),
            _ => false,
        }
    }

    /// Allocate a register holding Unit.
    fn unit_register(&mut self) -> Result<Register, BytecodeError> {
        let const_idx = self.emitter.add_constant(OvmValue::new_unit());
        let dst_reg = self.register_allocator.allocate_register();
        self.emitter.emit_load_const(dst_reg, const_idx);
        Ok(dst_reg)
    }

    /// True when evaluating `e` provably performs no assignment — the
    /// precondition for fusing `x = x + rhs` (fusion reorders the read of
    /// x after the rhs). A whitelist: anything unlisted conservatively
    /// declines fusion and compiles the ordinary Add + Move. Calls and
    /// lambdas are safe — compiled functions cannot reach the caller's
    /// registers, and lambdas that assign captured names are rejected by
    /// the lambda compiler — but a block or match arm can assign
    /// directly, so those decline.
    pub(crate) fn assignment_free(e: &crate::ast::Expr) -> bool {
        use crate::ast::Expr as E;
        match e {
            E::Integer(_)
            | E::Float(_)
            | E::String(_)
            | E::Boolean(_)
            | E::Identifier(_)
            | E::LocalRef { .. } => true,
            E::BinaryOp { left, right, .. } => {
                Self::assignment_free(left) && Self::assignment_free(right)
            }
            E::UnaryOp { operand, .. } => Self::assignment_free(operand),
            E::FieldAccess { object, .. } => Self::assignment_free(object),
            E::Index { object, index } => {
                Self::assignment_free(object) && Self::assignment_free(index)
            }
            E::ResultOk(inner) | E::ResultErr(inner) | E::Try(inner) => {
                Self::assignment_free(inner)
            }
            E::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::assignment_free(condition)
                    && Self::assignment_free(then_branch)
                    && else_branch
                        .as_ref()
                        .is_none_or(|e| Self::assignment_free(e))
            }
            E::Call { callee, arguments } => {
                Self::assignment_free(callee)
                    && arguments.iter().all(|a| match a {
                        crate::ast::Argument::Positional(e) => Self::assignment_free(e),
                        crate::ast::Argument::Named { value, .. } => Self::assignment_free(value),
                    })
            }
            E::List(items) => items.iter().all(Self::assignment_free),
            E::Tuple(items) => items.iter().all(Self::assignment_free),
            // A template string is its interpolations; the missing arm
            // here silently disabled the append fusion for the extremely
            // common `xs = xs + [`...${i}...`]` accumulation shape,
            // which then copied the whole list per iteration.
            E::TemplateString { parts } => parts.iter().all(|p| match p {
                crate::ast::TemplatePart::Literal(_) => true,
                crate::ast::TemplatePart::Interpolation(e) => Self::assignment_free(e),
            }),
            _ => false,
        }
    }

    /// True when `reg` is a local variable's home register — the one class
    /// of register that assignment instructions write in place.
    fn is_variable_register(&self, reg: Register) -> bool {
        self.local_variables.values().any(|&r| r == reg)
    }

    /// Multi-operand instructions read their operand registers when they
    /// execute — after ALL operand code has run — while the interpreter
    /// captures each operand's value in evaluation order. Those orders
    /// agree unless an earlier operand's register is a variable's home
    /// register and a later operand assigns to it (`x + { x = 1  5 }`).
    /// In exactly that case, copy the value out to a fresh register at
    /// the operand's evaluation point; otherwise (the hot path) keep the
    /// register as-is and pay no Move.
    fn shield_operand(&mut self, reg: Register, later_may_assign: bool) -> Register {
        if !later_may_assign || !self.is_variable_register(reg) {
            return reg;
        }
        let dst = self.register_allocator.allocate_register();
        self.emitter.emit_move(dst, reg);
        dst
    }

    /// Compile expressions evaluated left-to-right whose registers feed a
    /// single instruction (call arguments, list/tuple elements), shielding
    /// each from assignments in the expressions after it.
    fn compile_operands(&mut self, exprs: &[&Expr]) -> Result<Vec<Register>, BytecodeError> {
        let last_assigning = exprs.iter().rposition(|e| !Self::assignment_free(e));
        let mut regs = Vec::with_capacity(exprs.len());
        for (i, expr) in exprs.iter().enumerate() {
            let reg = self.compile_expression(expr)?;
            let reg = self.shield_operand(reg, last_assigning.is_some_and(|j| i < j));
            regs.push(reg);
        }
        Ok(regs)
    }

    /// Compile positional call arguments via `compile_operands`; named
    /// arguments refuse compilation.
    fn compile_call_args(
        &mut self,
        arguments: &[crate::ast::Argument],
    ) -> Result<Vec<Register>, BytecodeError> {
        let mut exprs = Vec::with_capacity(arguments.len());
        for argument in arguments {
            match argument {
                crate::ast::Argument::Positional(expr) => exprs.push(expr),
                crate::ast::Argument::Named { .. } => {
                    return Err(BytecodeError::CompilationFailed(
                        "Named arguments are not supported in the bytecode tier".to_string(),
                    ));
                }
            }
        }
        self.compile_operands(&exprs)
    }

    /// True when every guard nested in the pattern (via `Guarded`) is
    /// assignment-free. Guards run between pattern tests, which re-read
    /// the scrutinee register — see `shield_operand`.
    fn pattern_guards_assignment_free(p: &crate::ast::Pattern) -> bool {
        use crate::ast::Pattern as P;
        match p {
            P::Guarded { pattern, guard } => {
                Self::assignment_free(guard) && Self::pattern_guards_assignment_free(pattern)
            }
            P::List { patterns, .. } | P::Tuple(patterns) => {
                patterns.iter().all(Self::pattern_guards_assignment_free)
            }
            P::EnumVariant { patterns, .. } => {
                patterns.iter().all(Self::pattern_guards_assignment_free)
            }
            P::Or { alternatives } => alternatives
                .iter()
                .all(Self::pattern_guards_assignment_free),
            P::Ok(inner) | P::Err(inner) => Self::pattern_guards_assignment_free(inner),
            P::Struct { field_patterns, .. } => field_patterns
                .iter()
                .all(|(_, p)| Self::pattern_guards_assignment_free(p)),
            P::AnonymousStruct { field_patterns } => field_patterns
                .iter()
                .all(|(_, p)| Self::pattern_guards_assignment_free(p)),
            P::Range { start, end, .. } => {
                Self::pattern_guards_assignment_free(start)
                    && Self::pattern_guards_assignment_free(end)
            }
            _ => true,
        }
    }

    /// Compile a statement inside a block, returning the register holding its
    /// value (let-declarations evaluate to Unit like in the interpreter).
    fn compile_statement(
        &mut self,
        statement: &crate::ast::Statement,
    ) -> Result<Register, BytecodeError> {
        match statement {
            crate::ast::Statement::Located { line, column, stmt } => {
                // Mirror the interpreter's span discipline: this
                // statement's span covers its instructions; a nested block
                // pushes deeper spans, and when it ends the enclosing span
                // resurfaces for whatever the parent compiles afterwards.
                self.span_stack.push((*line, *column));
                self.emitter.note_span(*line, *column);
                let result = self.compile_statement(stmt);
                self.span_stack.pop();
                if let Some(&(l, c)) = self.span_stack.last() {
                    self.emitter.note_span(l, c);
                }
                result
            }
            crate::ast::Statement::Expression(expr) => self.compile_expression(expr),
            crate::ast::Statement::LetDecl(let_decl) => {
                // An annotated let is a checked boundary the VM does not
                // yet enforce inline — refuse, fail-closed: the interpreter
                // runs the function and enforces it (v1; liftable later).
                if let_decl
                    .type_annotation
                    .as_ref()
                    .and_then(|ann| crate::ast::FieldTypeCheck::from_annotation(ann, &[]))
                    .is_some()
                {
                    return Err(BytecodeError::CompilationFailed(
                        "let bindings with type annotations run interpreted".to_string(),
                    ));
                }
                let value_reg = match &let_decl.value {
                    Some(expr) => self.compile_expression(expr)?,
                    None => {
                        let const_idx = self.emitter.add_constant(OvmValue::from_ast(Value::Unit));
                        let reg = self.register_allocator.allocate_register();
                        self.emitter.emit_load_const(reg, const_idx);
                        reg
                    }
                };
                // Bind through the pattern machinery — an identifier binds
                // (a fresh register, so later assignment doesn't clobber the
                // shared value register), destructuring extracts, and a
                // non-matching pattern raises the interpreter's
                // PatternMatchFailed, exactly as eval_let_decl does.
                let ok_label = self.emitter.create_label();
                let fail_label = self.emitter.create_label();
                self.compile_pattern_test(&let_decl.pattern, value_reg, fail_label)?;
                self.emitter.emit_jump(ok_label);
                self.emitter.place_label(fail_label);
                self.emitter.instructions.push(Instruction::MatchFail);
                self.emitter.place_label(ok_label);

                // A let evaluates to the bound value — the interpreter's
                // eval_let_decl returns it, observable when a block ends in
                // a let. (This used to yield Unit: a real divergence, caught
                // while extending let patterns.)
                Ok(value_reg)
            }
            crate::ast::Statement::FunctionDecl(decl) => {
                // A nested fn is a NAMED closure over the current frame:
                // compiled through the lambda machinery with its own name as
                // a self-binding, so recursion resolves to its own compiled
                // id and an escaped copy (which carries the name) recurses
                // interpreted via the call-time self-definition.
                let value_reg =
                    self.compile_lambda(&decl.parameters, &decl.body, Some(&decl.name))?;
                let var_reg = self.register_allocator.allocate_register();
                self.local_variables.insert(decl.name.clone(), var_reg);
                self.emitter.emit_move(var_reg, value_reg);
                // The declaration evaluates to the function value, as
                // eval_function_decl returns it
                Ok(var_reg)
            }
            other => Err(BytecodeError::CompilationFailed(format!(
                "Unsupported statement in bytecode tier: {:?}",
                std::mem::discriminant(other)
            ))),
        }
    }
}

// Implementation stubs for optimization components
impl Default for RegisterAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl RegisterAllocator {
    pub fn new() -> Self {
        Self {
            next_register: 0,
            free_registers: Vec::new(),
            max_registers: 0,
        }
    }

    pub fn reset(&mut self) {
        self.next_register = 0;
        self.free_registers.clear();
        self.max_registers = 0;
    }

    pub fn allocate_register(&mut self) -> Register {
        if let Some(reg) = self.free_registers.pop() {
            reg
        } else {
            let reg = Register(self.next_register);
            self.next_register += 1;
            self.max_registers = self.max_registers.max(self.next_register);
            reg
        }
    }

    pub fn free_register(&mut self, reg: Register) {
        self.free_registers.push(reg);
    }

    pub fn max_register_used(&self) -> u32 {
        self.max_registers
    }
}

impl Default for InstructionEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl InstructionEmitter {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            span_table: Vec::new(),
            label_positions: HashMap::new(),
            next_label_id: 0,
            constants: Vec::new(),
            constant_map: HashMap::new(),
            current_line: 0,
            debug_info: BytecodeDebugInfo::default(),
        }
    }

    pub fn reset(&mut self) {
        self.instructions.clear();
        self.label_positions.clear();
        self.next_label_id = 0;
        self.constants.clear();
        self.constant_map.clear();
        self.current_line = 0;
        self.debug_info = BytecodeDebugInfo::default();
    }

    pub fn add_constant(&mut self, value: OvmValue) -> u32 {
        let idx = self.constants.len() as u32;
        self.constants.push(value);
        idx
    }

    pub fn emit_load_const(&mut self, dst: Register, const_idx: u32) {
        self.instructions
            .push(Instruction::LoadConst { dst, const_idx });
    }

    pub fn emit_load_local(&mut self, dst: Register, local_idx: u32) {
        self.instructions
            .push(Instruction::LoadLocal { dst, local_idx });
    }

    pub fn emit_store_local(&mut self, src: Register, local_idx: u32) {
        self.instructions
            .push(Instruction::StoreLocal { src, local_idx });
    }

    pub fn emit_add(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Add { dst, lhs, rhs });
    }

    pub fn emit_sub(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Sub { dst, lhs, rhs });
    }

    pub fn emit_mul(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Mul { dst, lhs, rhs });
    }

    pub fn emit_div(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Div { dst, lhs, rhs });
    }

    pub fn emit_mod(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Mod { dst, lhs, rhs });
    }

    pub fn emit_eq(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Eq { dst, lhs, rhs });
    }

    pub fn emit_ne(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Ne { dst, lhs, rhs });
    }

    pub fn emit_lt(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Lt { dst, lhs, rhs });
    }

    pub fn emit_le(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Le { dst, lhs, rhs });
    }

    pub fn emit_gt(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Gt { dst, lhs, rhs });
    }

    pub fn emit_ge(&mut self, dst: Register, lhs: Register, rhs: Register) {
        self.instructions.push(Instruction::Ge { dst, lhs, rhs });
    }

    pub fn emit_make_list(&mut self, dst: Register, elements: Vec<Register>) {
        self.instructions
            .push(Instruction::MakeList { dst, elements });
    }

    pub fn emit_return(&mut self, value: Option<Register>) {
        self.instructions.push(Instruction::Return { value });
    }

    pub fn emit_branch_if_false(&mut self, condition: Register, target: Label) {
        self.instructions
            .push(Instruction::JumpIfFalse { condition, target });
    }

    pub fn emit_jump(&mut self, target: Label) {
        self.instructions.push(Instruction::Jump { target });
    }

    pub fn emit_move(&mut self, dst: Register, src: Register) {
        self.instructions.push(Instruction::Move { dst, src });
    }

    pub fn emit_make_range(
        &mut self,
        dst: Register,
        start: Register,
        end: Register,
        inclusive: bool,
    ) {
        self.instructions.push(Instruction::MakeRange {
            dst,
            start,
            end,
            inclusive,
        });
    }

    pub fn create_label(&mut self) -> Label {
        let label = Label(self.next_label_id);
        self.next_label_id += 1;
        label
    }

    /// Bind a label to the current instruction offset.
    pub fn place_label(&mut self, label: Label) {
        self.label_positions
            .insert(label.0, self.instructions.len());
    }

    /// Patch every jump target from a label id to the instruction offset the
    /// label was placed at. Must run after emission, before execution — a
    /// label id is meaningless as a program counter.
    pub fn resolve_labels(&mut self) -> Result<(), BytecodeError> {
        let resolve =
            |target: &mut Label, positions: &HashMap<u32, usize>| match positions.get(&target.0) {
                Some(&offset) => {
                    *target = Label(offset as u32);
                    Ok(())
                }
                None => Err(BytecodeError::CompilationFailed(format!(
                    "Jump references unplaced label {}",
                    target.0
                ))),
            };

        for instruction in &mut self.instructions {
            match instruction {
                Instruction::Jump { target }
                | Instruction::JumpIfTrue { target, .. }
                | Instruction::JumpIfFalse { target, .. } => {
                    resolve(target, &self.label_positions)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn emit_nop(&mut self) {
        self.instructions.push(Instruction::Nop);
    }

    pub fn has_return(&self) -> bool {
        self.instructions
            .iter()
            .any(|instr| matches!(instr, Instruction::Return { .. }))
    }

    pub fn take_instructions(&mut self) -> Vec<Instruction> {
        std::mem::take(&mut self.instructions)
    }

    /// Mark that instructions emitted from here belong to the statement
    /// at (line, column). Consecutive duplicates collapse; a marker at
    /// the same offset as the previous one replaces it.
    pub fn note_span(&mut self, line: u32, column: u32) {
        let at = self.instructions.len() as u32;
        if let Some(last) = self.span_table.last_mut() {
            if last.1 == line && last.2 == column {
                return;
            }
            if last.0 == at {
                *last = (at, line, column);
                return;
            }
        }
        self.span_table.push((at, line, column));
    }

    pub fn take_spans(&mut self) -> Vec<(u32, u32, u32)> {
        std::mem::take(&mut self.span_table)
    }

    pub fn take_constants(&mut self) -> Vec<OvmValue> {
        std::mem::take(&mut self.constants)
    }
}

impl Default for BytecodeOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

impl BytecodeOptimizer {
    pub fn new() -> Self {
        Self {}
    }

    /// No-op: correctness first. See the struct docs for why the previous
    /// pipeline was removed.
    pub fn optimize_instructions(
        &mut self,
        instructions: Vec<Instruction>,
        constants: &[OvmValue],
    ) -> Result<Vec<Instruction>, BytecodeError> {
        let instructions = Self::fold_constant_branches(instructions, constants);
        let instructions = Self::sweep_unreachable(instructions);
        Ok(Self::eliminate_dead_moves(instructions))
    }

    /// A conditional jump whose condition register was just loaded with a
    /// Boolean constant is not a decision: it becomes a plain Jump (or
    /// nothing). Folded constants come from real code — a `caps.allowed`
    /// query folded under a static manifest, a literal `while true` — and
    /// folding here is what lets `sweep_unreachable` retire the branch
    /// that can never run.
    fn fold_constant_branches(
        mut instructions: Vec<Instruction>,
        constants: &[OvmValue],
    ) -> Vec<Instruction> {
        use Instruction as I;
        for pc in 1..instructions.len() {
            let (cond, target, jump_on) = match &instructions[pc] {
                I::JumpIfTrue { condition, target } => (*condition, *target, true),
                I::JumpIfFalse { condition, target } => (*condition, *target, false),
                _ => continue,
            };
            // Only the immediately preceding instruction: farther-away
            // defs would need a dominance argument this pass does not
            // make. The compiler emits exactly this adjacent shape for
            // folded queries and literal conditions.
            let I::LoadConst { dst, const_idx } = &instructions[pc - 1] else {
                continue;
            };
            if *dst != cond {
                continue;
            }
            let Some(crate::ovm::value::ValueData::Boolean(b)) =
                constants.get(*const_idx as usize).map(|c| &c.data)
            else {
                continue;
            };
            instructions[pc] = if *b == jump_on {
                I::Jump { target }
            } else {
                I::Nop
            };
        }
        instructions
    }

    /// Replace instructions no path from the entry can reach with Nop.
    /// Dead branches stop counting against the JIT's qualification scan,
    /// so a function whose denied capability branch contains an
    /// uncompilable call still compiles — the code that cannot run no
    /// longer taxes the code that does.
    pub(crate) fn sweep_unreachable(mut instructions: Vec<Instruction>) -> Vec<Instruction> {
        use Instruction as I;
        let n = instructions.len();
        if n == 0 {
            return instructions;
        }
        let mut reachable = vec![false; n];
        let mut work = vec![0usize];
        while let Some(pc) = work.pop() {
            if pc >= n || reachable[pc] {
                continue;
            }
            reachable[pc] = true;
            match &instructions[pc] {
                I::Jump { target } => work.push(target.0 as usize),
                I::JumpIfTrue { target, .. } | I::JumpIfFalse { target, .. } => {
                    work.push(target.0 as usize);
                    work.push(pc + 1);
                }
                I::TailCallSelf { .. } => work.push(0),
                I::Return { .. } | I::MatchFail => {}
                _ => work.push(pc + 1),
            }
        }
        for (pc, inst) in instructions.iter_mut().enumerate() {
            if !reachable[pc] {
                *inst = I::Nop;
            }
        }
        instructions
    }

    /// Replace `Move`s whose destination is never read again with `Nop`.
    ///
    /// The motivating case is the merge of an `if` used as a statement:
    /// each branch ends `Move phi ← result; Jump end`, and when nothing
    /// reads the phi, that Move still *copies the value* — for an Arc-
    /// backed list it plants a second reference in a register that stays
    /// live until some future instruction happens to overwrite it, which
    /// turns every subsequent sole-owner in-place write into a whole-
    /// list copy. Liveness is a backward fixpoint over the resolved
    /// control flow; any instruction the model does not know is treated
    /// as reading every register, so unknown territory disables the
    /// optimization rather than miscompiling it.
    fn eliminate_dead_moves(mut instructions: Vec<Instruction>) -> Vec<Instruction> {
        // Killing one Move can strand the Move feeding it (`Move b←a;
        // Move c←b` where only the first survives a single sweep), so the
        // pass runs to its own fixpoint. Chains are short; the bound is a
        // backstop, not a budget.
        for _ in 0..8 {
            let (next, changed) = Self::eliminate_dead_moves_once(instructions);
            instructions = next;
            if !changed {
                break;
            }
        }
        instructions
    }

    fn eliminate_dead_moves_once(mut instructions: Vec<Instruction>) -> (Vec<Instruction>, bool) {
        use Instruction as I;
        let mut any_rewrite = false;
        let n = instructions.len();
        if n == 0 {
            return (instructions, false);
        }
        let mut nregs = 0usize;
        {
            let mut track = |r: &Register| nregs = nregs.max(r.0 as usize + 1);
            for inst in &instructions {
                Self::visit_registers(inst, &mut track);
            }
        }
        // live[pc] = registers possibly read at or after pc along some path.
        let mut live_in: Vec<Vec<bool>> = vec![vec![false; nregs]; n];
        let mut changed = true;
        while changed {
            changed = false;
            for pc in (0..n).rev() {
                let mut out = vec![false; nregs];
                let mut succ = |t: usize| {
                    if t < n {
                        for (i, b) in live_in[t].iter().enumerate() {
                            if *b {
                                out[i] = true;
                            }
                        }
                    }
                };
                match &instructions[pc] {
                    I::Jump { target } => succ(target.0 as usize),
                    I::JumpIfTrue { target, .. } | I::JumpIfFalse { target, .. } => {
                        succ(target.0 as usize);
                        succ(pc + 1);
                    }
                    I::TailCallSelf { .. } => succ(0),
                    I::Return { .. } | I::MatchFail => {}
                    _ => succ(pc + 1),
                }
                let mut new_in = out;
                match Self::uses_defs(&instructions[pc]) {
                    Some((uses, defs)) => {
                        for d in defs {
                            if (d as usize) < nregs {
                                new_in[d as usize] = false;
                            }
                        }
                        for u in uses {
                            if (u as usize) < nregs {
                                new_in[u as usize] = true;
                            }
                        }
                    }
                    // Unmodeled: assume it reads everything and defines
                    // nothing — the conservative direction.
                    None => {
                        for b in new_in.iter_mut() {
                            *b = true;
                        }
                    }
                }
                if new_in != live_in[pc] {
                    live_in[pc] = new_in;
                    changed = true;
                }
            }
        }
        for pc in 0..n {
            // Calls consume dead-after argument registers by move: filling
            // the mask here (rather than cloning at frame push) is what
            // keeps a handle passed into a callee from staying pinned in
            // the caller's frame for the callee's whole run. Only
            // registers appearing once in the argument list qualify — a
            // duplicated register must still be cloned for its second use.
            if let I::CallFn {
                args, arg_moves, ..
            } = &mut instructions[pc]
            {
                let mut mask = 0u64;
                for (i, r) in args.iter().enumerate().take(64) {
                    let unique = args.iter().filter(|a| a.0 == r.0).count() == 1;
                    let dead = !(pc + 1 < n && live_in[pc + 1][r.0 as usize]);
                    if unique && dead {
                        mask |= 1 << i;
                    }
                }
                *arg_moves = mask;
            }
            if std::env::var_os("OLANG_DEBUG_LIVENESS").is_some()
                && let I::Move { dst, src } = &instructions[pc]
            {
                let la = |r: u32| pc + 1 < n && live_in[pc + 1][r as usize];
                eprintln!(
                    "[live] pc={} Move dst=r{}(live_after={}) src=r{}(live_after={})",
                    pc,
                    dst.0,
                    la(dst.0),
                    src.0,
                    la(src.0)
                );
            }
            let rewrite = match &instructions[pc] {
                I::Move { dst, src } if dst != src => {
                    let live_after = |r: u32| pc + 1 < n && live_in[pc + 1][r as usize];
                    if !live_after(dst.0) {
                        // Nobody reads the destination: the Move (and the
                        // reference it would plant) can vanish entirely.
                        Some(I::Nop)
                    } else if !live_after(src.0) {
                        // The destination lives but the source is dead —
                        // the call-result pattern (`CallFn dst; Move x ←
                        // dst`): move the value instead of cloning it, so
                        // the dead register does not pin an Arc until
                        // something happens to overwrite it.
                        Some(I::TakeMove {
                            dst: *dst,
                            src: *src,
                        })
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(inst) = rewrite {
                instructions[pc] = inst;
                any_rewrite = true;
            }
        }
        (instructions, any_rewrite)
    }

    /// Registers an instruction reads and writes, or None when this model
    /// does not describe it (treated as a full barrier by the liveness).
    #[allow(clippy::type_complexity)]
    pub(crate) fn uses_defs(inst: &Instruction) -> Option<(Vec<u32>, Vec<u32>)> {
        use Instruction as I;
        let mut uses = Vec::new();
        let mut defs = Vec::new();
        match inst {
            I::LoadConst { dst, .. } => defs.push(dst.0),
            I::Move { dst, src } => {
                uses.push(src.0);
                defs.push(dst.0);
            }
            I::TakeMove { dst, src } => {
                uses.push(src.0);
                defs.push(dst.0);
                defs.push(src.0);
            }
            I::Nop | I::MatchFail => {}
            I::Jump { .. } => {}
            I::JumpIfTrue { condition, .. } | I::JumpIfFalse { condition, .. } => {
                uses.push(condition.0)
            }
            I::Return { value } => {
                if let Some(r) = value {
                    uses.push(r.0);
                }
            }
            I::Add { dst, lhs, rhs }
            | I::Sub { dst, lhs, rhs }
            | I::Mul { dst, lhs, rhs }
            | I::Div { dst, lhs, rhs }
            | I::Mod { dst, lhs, rhs }
            | I::Eq { dst, lhs, rhs }
            | I::Ne { dst, lhs, rhs }
            | I::Lt { dst, lhs, rhs }
            | I::Le { dst, lhs, rhs }
            | I::Gt { dst, lhs, rhs }
            | I::Ge { dst, lhs, rhs }
            | I::And { dst, lhs, rhs }
            | I::Or { dst, lhs, rhs } => {
                uses.push(lhs.0);
                uses.push(rhs.0);
                defs.push(dst.0);
            }
            I::PatternEq { dst, value, other } => {
                uses.push(value.0);
                uses.push(other.0);
                defs.push(dst.0);
            }
            I::Not { dst, src } | I::Neg { dst, src } => {
                uses.push(src.0);
                defs.push(dst.0);
            }
            I::BinImm { dst, lhs, .. } => {
                uses.push(lhs.0);
                defs.push(dst.0);
            }
            I::AddAssign { target, rhs } => {
                uses.push(target.0);
                uses.push(rhs.0);
                defs.push(target.0);
            }
            I::ListSetAssign {
                target,
                index,
                value,
            } => {
                uses.push(target.0);
                uses.push(index.0);
                uses.push(value.0);
                defs.push(target.0);
            }
            I::ListSwapAssign { target, i, j } => {
                uses.push(target.0);
                uses.push(i.0);
                uses.push(j.0);
                defs.push(target.0);
            }
            I::IndexGet { dst, object, index } => {
                uses.push(object.0);
                uses.push(index.0);
                defs.push(dst.0);
            }
            I::IterLen { dst, src } => {
                uses.push(src.0);
                defs.push(dst.0);
            }
            I::IterGet { dst, src, idx } => {
                uses.push(src.0);
                uses.push(idx.0);
                defs.push(dst.0);
            }
            I::MakeTemplate { dst, parts } => {
                for p in parts {
                    if let crate::ovm::bytecode::TplPart::Reg(r) = p {
                        uses.push(r.0);
                    }
                }
                defs.push(dst.0);
            }
            I::MakeList { dst, elements } | I::MakeTuple { dst, elements } => {
                for e in elements {
                    uses.push(e.0);
                }
                defs.push(dst.0);
            }
            I::CallFn { dst, args, .. } => {
                for a in args {
                    uses.push(a.0);
                }
                defs.push(dst.0);
            }
            I::CallNamed { dst, args, .. } => {
                for a in args {
                    uses.push(a.0);
                }
                defs.push(dst.0);
            }
            I::CallBuiltin { dst, args, .. } => {
                for a in args {
                    uses.push(a.0);
                }
                defs.push(dst.0);
            }
            I::CallValue { dst, callee, args } => {
                uses.push(callee.0);
                for a in args {
                    uses.push(a.0);
                }
                defs.push(dst.0);
            }
            I::TailCallSelf { args } => {
                for (i, a) in args.iter().enumerate() {
                    uses.push(a.0);
                    defs.push(i as u32);
                }
            }
            _ => return None,
        }
        Some((uses, defs))
    }

    /// Visit every register an instruction mentions (for sizing).
    fn visit_registers(inst: &Instruction, f: &mut impl FnMut(&Register)) {
        if let Some((uses, defs)) = Self::uses_defs(inst) {
            for u in uses.iter().chain(defs.iter()) {
                f(&Register(*u));
            }
        }
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Instruction::LoadConst { dst, const_idx } => {
                write!(f, "LOAD_CONST r{}, #{}", dst.0, const_idx)
            }
            Instruction::LoadLocal { dst, local_idx } => {
                write!(f, "LOAD_LOCAL r{}, l{}", dst.0, local_idx)
            }
            Instruction::StoreLocal { src, local_idx } => {
                write!(f, "STORE_LOCAL r{}, l{}", src.0, local_idx)
            }
            Instruction::Move { dst, src } => write!(f, "MOVE r{}, r{}", dst.0, src.0),
            Instruction::AddAssign { target, rhs } => {
                write!(f, "ADD_ASSIGN r{}, r{}", target.0, rhs.0)
            }
            Instruction::Add { dst, lhs, rhs } => {
                write!(f, "ADD r{}, r{}, r{}", dst.0, lhs.0, rhs.0)
            }
            Instruction::Sub { dst, lhs, rhs } => {
                write!(f, "SUB r{}, r{}, r{}", dst.0, lhs.0, rhs.0)
            }
            Instruction::Mul { dst, lhs, rhs } => {
                write!(f, "MUL r{}, r{}, r{}", dst.0, lhs.0, rhs.0)
            }
            Instruction::Return { value } => {
                if let Some(reg) = value {
                    write!(f, "RETURN r{}", reg.0)
                } else {
                    write!(f, "RETURN")
                }
            }
            Instruction::Nop => write!(f, "NOP"),
            _ => write!(f, "{:?}", self),
        }
    }
}

impl Default for BytecodeVm {
    fn default() -> Self {
        Self::new()
    }
}

/// The value view `FieldTypeCheck::check_value` wants: the outer type
/// name, which Result side is present (and its payload's name), and a
/// callable's (required, total) parameter counts where the value exposes
/// them — AstFunction and Closure carry full parameter info; FunctionObject
/// and CompiledFunction don't record defaults, so their arity stays
/// unchecked rather than wrongly strict.
#[allow(clippy::type_complexity)]
fn ovm_value_view(
    v: &OvmValue,
) -> (
    &str,
    Option<(bool, &str)>,
    Option<(usize, usize)>,
    Option<crate::ast::ScalarView<'_>>,
) {
    use crate::ovm::value::ValueData;
    let payload = match &v.data {
        ValueData::Result(r) => match (&r.ok, &r.err) {
            (Some(p), _) => Some((true, p.type_name())),
            (_, Some(p)) => Some((false, p.type_name())),
            _ => None,
        },
        _ => None,
    };
    let counts = |params: &[crate::ast::Parameter]| {
        (
            params.iter().filter(|p| p.default_value.is_none()).count(),
            params.len(),
        )
    };
    let arity = match &v.data {
        ValueData::AstFunction(f) => Some(counts(&f.parameters)),
        ValueData::Closure(c) => Some(counts(&c.template.parameters)),
        _ => None,
    };
    let scalar = match &v.data {
        ValueData::Integer(i) => Some(crate::ast::ScalarView::Int(*i)),
        ValueData::String(s) => Some(crate::ast::ScalarView::Str(s.as_str())),
        ValueData::Boolean(b) => Some(crate::ast::ScalarView::Bool(*b)),
        _ => None,
    };
    (v.type_name(), payload, arity, scalar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FunctionDecl, Parameter, TypeAnnotation};

    #[test]
    fn test_bytecode_vm_creation() {
        let vm = BytecodeVm::new();
        assert_eq!(vm.stats.instructions_executed, 0);
        assert_eq!(vm.stats.function_calls, 0);
    }

    #[test]
    fn test_simple_function_compilation() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a simple function: fn test() -> int { 42 }
        let func = FunctionDecl {
            name_span: None,
            name: "test".to_string(),
            type_params: Vec::new(),
            type_param_bounds: Vec::new(),
            parameters: Vec::new(),
            return_type: Some(TypeAnnotation::Int),
            body: Expr::Integer(42),
        };

        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function compilation should succeed");
        assert!(
            vm.has_bytecode(func_id),
            "VM should have bytecode for the function"
        );
    }

    #[test]
    fn test_function_with_parameters() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create function: fn add(a: int, b: int) -> int { a + b }
        let func = FunctionDecl {
            name_span: None,
            name: "add".to_string(),
            type_params: Vec::new(),
            type_param_bounds: Vec::new(),
            parameters: vec![
                Parameter {
                    name: "a".to_string(),
                    type_annotation: Some(TypeAnnotation::Int),
                    default_value: None,
                },
                Parameter {
                    name: "b".to_string(),
                    type_annotation: Some(TypeAnnotation::Int),
                    default_value: None,
                },
            ],
            return_type: Some(TypeAnnotation::Int),
            body: Expr::BinaryOp {
                left: Box::new(Expr::Identifier("a".to_string())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Identifier("b".to_string())),
            },
        };

        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function with parameters should compile");

        // Test execution with arguments
        let args = vec![
            OvmValue::from_ast(Value::Integer(5)),
            OvmValue::from_ast(Value::Integer(3)),
        ];

        let execution_result = vm.execute(func_id, &args);
        assert!(
            execution_result.is_ok(),
            "Function execution should succeed"
        );

        // The result should be 8 (5 + 3)
        let result_value = execution_result.unwrap();
        match result_value.to_ast() {
            Ok(Value::Integer(n)) => assert_eq!(n, 8, "Addition result should be 8"),
            Ok(value) => panic!("Expected integer result, but got: {:?}", value),
            Err(e) => panic!("Failed to convert result to AST: {:?}", e),
        }
    }

    #[test]
    fn test_arithmetic_operations() {
        let vm = BytecodeVm::new();

        let left = OvmValue::from_ast(Value::Integer(10));
        let right = OvmValue::from_ast(Value::Integer(3));

        // Test addition
        let result = vm.execute_binary_op(&left, &right, BinaryOp::Add);
        assert!(result.is_ok());
        match result.unwrap().to_ast() {
            Ok(Value::Integer(n)) => assert_eq!(n, 13),
            _ => panic!("Expected integer result"),
        }

        // Test division
        let result = vm.execute_binary_op(&left, &right, BinaryOp::Divide);
        assert!(result.is_ok());
        match result.unwrap().to_ast() {
            Ok(Value::Integer(n)) => assert_eq!(n, 3),
            _ => panic!("Expected integer result"),
        }

        // Test division by zero
        let zero = OvmValue::from_ast(Value::Integer(0));
        let result = vm.execute_binary_op(&left, &zero, BinaryOp::Divide);
        assert!(result.is_err(), "Division by zero should fail");
        match result.unwrap_err() {
            BytecodeError::DivisionByZero => {}
            _ => panic!("Expected division by zero error"),
        }
    }

    #[test]
    fn test_builtin_functions() {
        // Builtins are dispatched by name and delegate to the interpreter's
        // implementations.
        let mut vm = BytecodeVm::new();

        let list_arg = OvmValue::from_ast(Value::List(
            vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)].into(),
        ));

        let result = vm.execute_builtin_call("len", &[list_arg], false);
        assert!(result.is_ok());
        match result.unwrap().to_ast() {
            Ok(Value::Integer(n)) => assert_eq!(n, 3, "List length should be 3"),
            _ => panic!("Expected integer result"),
        }

        let int_arg = OvmValue::from_ast(Value::Integer(42));
        let result = vm.execute_builtin_call("to_string", &[int_arg], false);
        assert!(result.is_ok());
        match result.unwrap().to_ast() {
            Ok(Value::String(s)) => assert_eq!(*s, "42", "Should convert to string"),
            _ => panic!("Expected string result"),
        }
    }

    #[test]
    fn unrepresentable_builtin_results_are_rejected() {
        // A value that cannot round-trip through the OVM model must produce an
        // error rather than silently becoming Unit. Maps round-trip now, so
        // the unrepresentable specimen is a map holding an unrepresentable
        // VALUE (a promise-free stand-in: a map containing a map is fine, so
        // use a TypeInfo, which never converts).
        let mut vm = BytecodeVm::new();
        let mut fields = std::collections::HashMap::new();
        fields.insert("a".to_string(), Value::Integer(1));
        assert!(BytecodeVm::round_trips(&Value::Map(Arc::new(
            fields.clone()
        ))));
        let mut bad = std::collections::HashMap::new();
        bad.insert(
            "t".to_string(),
            Value::TypeInfo {
                name: "X".to_string(),
                definition: crate::ast::TypeDefinition::Struct { fields: Vec::new() },
            },
        );
        assert!(!BytecodeVm::round_trips(&Value::Map(Arc::new(bad))));
        assert!(BytecodeVm::round_trips(&Value::Integer(1)));
        assert!(BytecodeVm::round_trips(&Value::Ok(Box::new(
            Value::Integer(1)
        ))));
        // sanity: the delegation path still works for a representable result
        assert!(
            vm.execute_builtin_call("to_string", &[OvmValue::new_integer(7)], false)
                .is_ok()
        );
    }

    #[test]
    fn test_register_allocator() {
        let mut allocator = RegisterAllocator::new();

        let reg1 = allocator.allocate_register();
        let reg2 = allocator.allocate_register();
        let reg3 = allocator.allocate_register();

        assert_eq!(reg1.0, 0);
        assert_eq!(reg2.0, 1);
        assert_eq!(reg3.0, 2);
        assert_eq!(allocator.max_register_used(), 3);

        // Test register reuse
        allocator.free_register(reg2);
        let reg4 = allocator.allocate_register();
        assert_eq!(reg4.0, 1, "Should reuse freed register");
    }

    #[test]
    fn test_instruction_emitter() {
        let mut emitter = InstructionEmitter::new();

        let reg1 = Register(0);
        let reg2 = Register(1);
        let const_idx = emitter.add_constant(OvmValue::from_ast(Value::Integer(42)));

        emitter.emit_load_const(reg1, const_idx);
        emitter.emit_add(reg2, reg1, reg1);
        emitter.emit_return(Some(reg2));

        assert!(emitter.has_return(), "Should have return instruction");

        let instructions = emitter.take_instructions();
        assert_eq!(instructions.len(), 3, "Should have 3 instructions");

        let constants = emitter.take_constants();
        assert_eq!(constants.len(), 1, "Should have 1 constant");
    }

    #[test]
    fn test_bytecode_execution_arithmetic() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a simpler function first: fn simple() -> Int = 5 + 3
        let func = FunctionDecl {
            name_span: None,
            name: "simple".to_string(),
            type_params: vec![],
            type_param_bounds: Vec::new(),
            parameters: vec![],
            body: Expr::BinaryOp {
                op: BinaryOp::Add,
                left: Box::new(Expr::Integer(5)),
                right: Box::new(Expr::Integer(3)),
            },
            return_type: None,
        };

        // Compile the function
        let result = vm.compile_function(func_id, &func);
        assert!(
            result.is_ok(),
            "Function compilation should succeed: {:?}",
            result.err()
        );

        // Execute the function with no arguments
        let args = vec![];

        let result = vm.execute(func_id, &args);
        assert!(
            result.is_ok(),
            "Function execution should succeed: {:?}",
            result.err()
        );

        let result_value = result.unwrap();
        if let crate::ovm::value::ValueData::Integer(val) = result_value.data {
            assert_eq!(val, 8, "5 + 3 should equal 8");
        } else {
            panic!("Expected integer result, got: {:?}", result_value);
        }
    }

    #[test]
    fn test_bytecode_execution_with_optimizations() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a function with constant folding opportunity: fn const_expr() -> Int = 10 + 20 + 30
        let func = FunctionDecl {
            name_span: None,
            name: "const_expr".to_string(),
            type_params: vec![],
            type_param_bounds: Vec::new(),
            parameters: vec![],
            body: Expr::BinaryOp {
                op: BinaryOp::Add,
                left: Box::new(Expr::BinaryOp {
                    op: BinaryOp::Add,
                    left: Box::new(Expr::Integer(10)),
                    right: Box::new(Expr::Integer(20)),
                }),
                right: Box::new(Expr::Integer(30)),
            },
            return_type: None,
        };

        // Compile the function (should apply constant folding optimization)
        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function compilation should succeed");

        // Execute the function
        let args = vec![];
        let result = vm.execute(func_id, &args);
        assert!(result.is_ok(), "Function execution should succeed");

        let result_value = result.unwrap();
        if let crate::ovm::value::ValueData::Integer(val) = result_value.data {
            assert_eq!(val, 60, "10 + 20 + 30 should equal 60");
        } else {
            panic!("Expected integer result, got: {:?}", result_value);
        }
    }

    #[test]
    fn test_bytecode_execution_control_flow() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a function with conditional: fn simple_if() -> Int = if 10 > 5 then 10 else 5
        let func = FunctionDecl {
            name_span: None,
            name: "simple_if".to_string(),
            type_params: vec![],
            type_param_bounds: Vec::new(),
            parameters: vec![],
            body: Expr::If {
                condition: Box::new(Expr::BinaryOp {
                    op: BinaryOp::GreaterThan,
                    left: Box::new(Expr::Integer(10)),
                    right: Box::new(Expr::Integer(5)),
                }),
                then_branch: Box::new(Expr::Integer(10)),
                else_branch: Some(Box::new(Expr::Integer(5))),
            },
            return_type: None,
        };

        // Compile the function
        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function compilation should succeed");

        // Test the condition (10 > 5 is true, so should return 10)
        let args = vec![];
        let result = vm.execute(func_id, &args);
        assert!(result.is_ok(), "Function execution should succeed");

        let result_value = result.unwrap();
        if let crate::ovm::value::ValueData::Integer(val) = result_value.data {
            assert_eq!(val, 10, "if 10 > 5 then 10 else 5 should return 10");
        } else {
            panic!("Expected integer result, got: {:?}", result_value);
        }
    }

    #[test]
    fn test_bytecode_execution_range_operations() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // Create a function that works with ranges: fn range_test() -> Range = 1..10
        let func = FunctionDecl {
            name_span: None,
            name: "range_test".to_string(),
            type_params: vec![],
            type_param_bounds: Vec::new(),
            parameters: vec![],
            body: Expr::Range {
                start: Box::new(Expr::Integer(1)),
                end: Box::new(Expr::Integer(10)),
                inclusive: false,
            },
            return_type: None,
        };

        // Compile the function
        let result = vm.compile_function(func_id, &func);
        assert!(result.is_ok(), "Function compilation should succeed");

        // Execute the function
        let args = vec![];
        let result = vm.execute(func_id, &args);
        assert!(result.is_ok(), "Function execution should succeed");

        let result_value = result.unwrap();
        if let crate::ovm::value::ValueData::Range(range) = &result_value.data {
            assert_eq!(range.start, 1, "Range start should be 1");
            assert_eq!(range.end, 10, "Range end should be 10");
            assert!(!range.inclusive, "Range should not be inclusive");
        } else {
            panic!("Expected range result, got: {:?}", result_value);
        }
    }

    #[test]
    fn test_bytecode_execution_stats() {
        let mut vm = BytecodeVm::new();
        let func_id = FunctionId::new();

        // A string body: stays on bytecode (the JIT's pure-integer
        // whitelist declines it), so dispatch-loop stats keep counting.
        let func = FunctionDecl {
            name_span: None,
            name: "simple".to_string(),
            type_params: vec![],
            type_param_bounds: Vec::new(),
            parameters: vec![],
            body: Expr::String(std::sync::Arc::new("still on bytecode".to_string())),
            return_type: None,
        };

        // Compile and execute multiple times to generate stats
        vm.compile_function(func_id, &func).unwrap();

        for _ in 0..5 {
            let _ = vm.execute(func_id, &[]);
        }

        // Check that statistics are being tracked
        let stats = vm.get_stats();
        assert!(
            stats.instructions_executed > 0,
            "Should have executed instructions"
        );
        assert!(
            stats.function_calls >= 5,
            "Should have recorded function calls"
        );
    }

    #[test]
    fn test_bytecode_error_handling() {
        let mut vm = BytecodeVm::new();
        let invalid_func_id = FunctionId::new();

        // Try to execute a function that doesn't exist
        let result = vm.execute(invalid_func_id, &[]);
        assert!(
            result.is_err(),
            "Should fail to execute non-existent function"
        );

        // Try to execute with wrong number of arguments
        let func_id = FunctionId::new();
        let func = FunctionDecl {
            name_span: None,
            name: "two_param".to_string(),
            type_params: vec![],
            type_param_bounds: Vec::new(),
            parameters: vec![
                crate::ast::Parameter {
                    name: "x".to_string(),
                    type_annotation: None,
                    default_value: None,
                },
                crate::ast::Parameter {
                    name: "y".to_string(),
                    type_annotation: None,
                    default_value: None,
                },
            ],
            body: Expr::Identifier("x".to_string()),
            return_type: None,
        };

        vm.compile_function(func_id, &func).unwrap();

        // Execute with wrong number of args
        let result = vm.execute(func_id, &[OvmValue::new_integer(1)]); // Should need 2 args
        assert!(
            result.is_err(),
            "Should fail with wrong number of arguments"
        );
    }
}

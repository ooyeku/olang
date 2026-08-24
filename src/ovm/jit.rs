//! The baseline JIT: hot bytecode compiled to native machine code.
//!
//! This is the third tier, and it extends the correctness story unchanged:
//! interpreter → bytecode ("can't compile identically → stay interpreted")
//! → native ("can't compile natively → stay on bytecode"). A function
//! prequalifies at promotion time when every instruction falls in a pure
//! numeric/boolean whitelist — arithmetic, comparisons, branches, calls
//! to other olang functions, return. The actual compilation is
//! **type-specialized, lazy, and call-graph aware**: on a function's
//! first call, the JIT plans every function reachable through its CallFn
//! sites, runs kind inference to a global fixpoint across the group
//! (callee return kinds feed caller registers; masks only grow, so it
//! converges), and compiles the whole group with direct native-to-native
//! calls — helpers, chains, and mutual recursion all stay native. Each
//! compiled function guards its entry on the exact argument kinds it was
//! specialized for (one specialization per function); any other shape
//! runs on bytecode as before.
//!
//! Pure is the load-bearing word: a qualifying group has no side
//! effects, so *any* guard failure (argument-kind mismatch, integer
//! overflow, division by zero, depth exhaustion — anywhere in the native
//! call chain) simply abandons the native run and re-executes the
//! original call on bytecode, which produces the exact result or error
//! the VM would have produced anyway. The JIT never reproduces an error
//! message; it only ever declines.
//!
//! Codegen notes:
//! - Registers are typed by inference: i64 (integers, booleans as 0/1)
//!   or f64. Mixed int/float arithmetic promotes the integer side with
//!   fcvt_from_sint, exactly as the VM's execute_binary_op does.
//! - Checked integer arithmetic is hand-rolled flag math; float add/sub/
//!   mul are plain IEEE (as in the VM), while float division guards
//!   b == 0.0 because olang errors there rather than producing inf.
//!   Float modulo is refused — fmod has no exact IR equivalent.
//! - A register written with conflicting kinds is fine as long as nothing
//!   reads it (an `if` statement's dead result slot); liveness flows
//!   backwards through copies. Dead stores are skipped, but operation
//!   *guards* still run, because the VM would still error on an
//!   overflowing dead computation.
//! - Every call carries a depth budget clamped to the VM's own
//!   max_call_depth; exhaustion deopts, so runaway recursion errors the
//!   canonical way instead of smashing the native stack. Each internal
//!   function returns (value, status); status != 0 deopts the caller
//!   too, unwinding the native stack to the entry wrapper.

use crate::ast::BinaryOp;
use crate::ovm::FunctionId;
use crate::ovm::bytecode::{CompiledBytecode, Instruction};
use crate::ovm::value::{OvmValue, ValueData};

use std::collections::HashMap;
use std::sync::Arc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Type, Value as ClifValue, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

/// The C-ABI entry: (args pointer, depth budget, out pointer) -> status.
/// Status 0 means `*out` holds the result (raw bits; the caller knows the
/// kind); anything else means deopt. Float arguments and results travel
/// as their IEEE bit patterns in the i64 slots.
type NativeEntry = unsafe extern "C" fn(*const i64, i64, *mut ScratchCtx, *mut i64) -> i64;

/// Per-call allocation context, owned by the VM for exactly one native
/// call. Every struct a native body builds lands here (so deopt can
/// never leak), and `retained` carries the one Arc a struct return
/// hands back. Registered args make returning a parameter sound.
#[derive(Default)]
pub struct ScratchCtx {
    allocs: Vec<Arc<crate::ovm::value::StructObject>>,
    args: Vec<Arc<crate::ovm::value::StructObject>>,
    retained: Option<Arc<crate::ovm::value::StructObject>>,
    str_allocs: Vec<Arc<String>>,
    str_args: Vec<Arc<String>>,
    retained_str: Option<Arc<String>>,
    result_allocs: Vec<Arc<crate::ovm::value::ResultObject>>,
    result_args: Vec<Arc<crate::ovm::value::ResultObject>>,
    retained_result: Option<Arc<crate::ovm::value::ResultObject>>,
    list_allocs: Vec<Arc<Vec<OvmValue>>>,
    list_args: Vec<Arc<Vec<OvmValue>>>,
    retained_list: Option<Arc<Vec<OvmValue>>>,
    map_allocs: Vec<Arc<HashMap<String, OvmValue>>>,
    map_args: Vec<Arc<HashMap<String, OvmValue>>>,
    retained_map: Option<Arc<HashMap<String, OvmValue>>>,
    /// Scratch lengths at loop entry. Each back-edge truncates every
    /// family back to this mark, freeing the iteration's allocations —
    /// codegen only permits it when every heap value born in the loop
    /// provably dies in its iteration. None until a loop is entered;
    /// release without a mark is a no-op (fail-safe for loop entries
    /// codegen didn't instrument).
    mark: Option<[usize; 5]>,
}

impl ScratchCtx {
    /// Drop whatever the previous call left behind while keeping every
    /// buffer's capacity. The context is reused across boundary calls
    /// (see JitCache::scratch), so an allocating constructor stops
    /// paying a malloc/free per call for its bookkeeping vec.
    fn clear(&mut self) {
        self.allocs.clear();
        self.args.clear();
        self.retained = None;
        self.str_allocs.clear();
        self.str_args.clear();
        self.retained_str = None;
        self.result_allocs.clear();
        self.result_args.clear();
        self.retained_result = None;
        self.list_allocs.clear();
        self.list_args.clear();
        self.retained_list = None;
        self.map_allocs.clear();
        self.map_args.clear();
        self.retained_map = None;
    }
}

/// How the VM hands the JIT other functions' bytecode when planning a
/// call graph.
pub type BytecodeLookup<'a> = dyn Fn(FunctionId) -> Option<Arc<CompiledBytecode>> + 'a;

/// One callable's signature in the group-inference snapshot: parameter
/// kinds, current return mask, element kinds when it returns a tuple,
/// its shape when it returns a struct, its full Result kind when it
/// returns a Result, and its list kind when it returns a list.
type SigSnapshot = HashMap<
    usize,
    (
        Vec<Kind>,
        u16,
        Option<Vec<Kind>>,
        Option<u32>,
        Option<Kind>,
        Option<Kind>,
    ),
>;

/// A struct shape observed at the entry: the interned shape plus the
/// field kinds of the argument instance the specialization keys on.
/// Per-read helper guards keep same-shape/different-kind instances safe.
#[derive(Clone)]
pub struct ShapeSpec {
    pub shape: Arc<crate::ovm::value::StructShape>,
    pub field_kinds: Vec<Kind>,
}

/// Classify a Result argument by its present side's payload: scalars and
/// strings specialize, anything else refuses. The absent side is Absent.
pub fn classify_result(r: &crate::ovm::value::ResultObject) -> Option<Kind> {
    fn payload_of(v: Option<&OvmValue>) -> Option<Payload> {
        Some(match v.map(|v| &v.data) {
            None => Payload::Absent,
            Some(ValueData::Integer(_)) => Payload::Int,
            Some(ValueData::Float(_)) => Payload::Float,
            Some(ValueData::Boolean(_)) => Payload::Bool,
            Some(ValueData::String(_)) => Payload::Str,
            _ => return None,
        })
    }
    Some(Kind::Result(
        payload_of(r.ok.as_ref())?,
        payload_of(r.err.as_ref())?,
    ))
}

/// Classify a map argument by its values: a uniform scalar or string
/// payload specializes; an empty map is Map(Absent) — its guarded reads
/// all deopt, but construction and presence tests still compile.
pub fn classify_map(m: &HashMap<String, OvmValue>) -> Option<Kind> {
    let mut payload = Payload::Absent;
    for v in m.values() {
        let pv = match &v.data {
            ValueData::Integer(_) => Payload::Int,
            ValueData::Float(_) => Payload::Float,
            ValueData::Boolean(_) => Payload::Bool,
            ValueData::String(_) => Payload::Str,
            _ => return None,
        };
        payload = join_payload(payload, pv)?;
    }
    Some(Kind::Map(payload))
}

/// Classify a list argument by its elements: uniformly Float, Int, or
/// one struct shape. Empty or mixed lists refuse (per-read guards would
/// have nothing sound to specialize against).
pub fn classify_list(items: &[OvmValue]) -> Option<Kind> {
    let first = items.first()?;
    let want = match &first.data {
        ValueData::Float(_) => Kind::ListFloat,
        ValueData::Integer(_) => Kind::ListInt,
        ValueData::Struct(s) => Kind::ListStruct(s.shape.id),
        ValueData::String(_) => Kind::ListStr,
        _ => return None,
    };
    for v in items.iter().skip(1) {
        let ok = match (&v.data, want) {
            (ValueData::Float(_), Kind::ListFloat) => true,
            (ValueData::Integer(_), Kind::ListInt) => true,
            (ValueData::Struct(s), Kind::ListStruct(sid)) => s.shape.id == sid,
            (ValueData::String(_), Kind::ListStr) => true,
            _ => false,
        };
        if !ok {
            return None;
        }
    }
    Some(want)
}

/// Collect shape specs relevant to `v` (a struct, or a list of structs)
/// into `shapes` — used by call sites before a first (specializing) call.
pub fn note_shapes(v: &OvmValue, shapes: &mut HashMap<u32, ShapeSpec>) {
    match &v.data {
        ValueData::Struct(obj) => {
            if let Some((_, spec)) = observe_struct(obj) {
                shapes.entry(obj.shape.id).or_insert(spec);
            }
        }
        ValueData::List(items) => {
            if let Some(ValueData::Struct(first)) = items.first().map(|x| &x.data)
                && let Some((_, spec)) = observe_struct(first)
            {
                shapes.entry(first.shape.id).or_insert(spec);
            }
        }
        _ => {}
    }
}

fn observe_struct(obj: &crate::ovm::value::StructObject) -> Option<(Kind, ShapeSpec)> {
    let mut field_kinds = Vec::with_capacity(obj.values.len());
    for v in &obj.values {
        field_kinds.push(match &v.data {
            ValueData::Integer(_) => Kind::Int,
            ValueData::Float(_) => Kind::Float,
            ValueData::Boolean(_) => Kind::Bool,
            ValueData::String(_) => Kind::Str,
            _ => return None,
        });
    }
    Some((
        Kind::Struct(obj.shape.id),
        ShapeSpec {
            shape: obj.shape.clone(),
            field_kinds,
        },
    ))
}

const STATUS_OK: i64 = 0;
const MAX_PARAMS: usize = 16;
/// Sanity bound on how many functions one group may pull in.
const MAX_GROUP: usize = 32;

/// What one side of an observed Result holds. `Absent` means the side
/// was never seen at specialization time — reads of it are guarded and
/// the guard can only deopt, so any downstream claim is vacuous.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Payload {
    Absent,
    Int,
    Bool,
    Float,
    Str,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Int,
    Bool,
    Float,
    /// A struct argument of one interned shape, passed as a borrowed
    /// pointer (JIT calls are synchronous; the caller's slot outlives
    /// the call, so no refcount is touched).
    Struct(u32),
    /// A list argument passed as a borrowed pointer to its Vec, with
    /// every element observed as the given kind at specialization time.
    /// Per-read helper guards keep differently-typed elements safe.
    ListFloat,
    ListInt,
    ListStruct(u32),
    /// A list argument whose elements are uniformly strings. Element
    /// reads hand out borrowed pointers into the list, valid for the
    /// synchronous call like struct elements.
    ListStr,
    /// A string argument or field, passed as a borrowed pointer to the
    /// String behind its Arc. Concat allocates through the scratch
    /// context under the same straight-line discipline as structs.
    Str,
    /// A Result passed as a borrowed pointer to its ResultObject, with
    /// the payload kind of each side as observed (ok, err). Sides merge
    /// monotonically: Absent joins with anything, so `Ok(Int)` and
    /// `Err(Str)` flowing into one register give Result(Int, Str).
    /// Every payload read is guarded per side, so a mismatch deopts.
    Result(Payload, Payload),
    /// A string-keyed map with a uniform value payload, passed as a
    /// borrowed pointer to its HashMap. Absent = observed empty (every
    /// guarded read then deopts on execution). Reads are key-guarded, so
    /// a missing key or a value surprise deopts, never misreads.
    Map(Payload),
}

impl Kind {
    fn clif_type(self) -> Type {
        match self {
            Kind::Float => types::F64,
            _ => types::I64,
        }
    }
}

struct JittedFn {
    entry: NativeEntry,
    /// The inner (fast-convention) function, callable from later groups.
    clif_id: cranelift_module::FuncId,
    param_kinds: Vec<Kind>,
    ret_kind: Kind,
    /// Element kinds when this function returns a tuple (the entry
    /// wrapper then writes one out slot per element).
    ret_tuple: Option<Vec<Kind>>,
    /// Builtin natives this body (or any group member / native callee)
    /// baked direct calls to. A later user definition of one of these
    /// names demotes the whole entry — the bytecode rerun then resolves
    /// the name the ordinary way.
    baked_builtins: Arc<[String]>,
    /// Owns the bytecode this body was compiled from: MakeStruct sites
    /// bake pointers to shape Arcs living inside its instructions, so
    /// the native code must never outlive it (note-channel invalidation
    /// can drop the VM's own copy while this stays Ready).
    _bytecode: Arc<CompiledBytecode>,
}

enum Slot {
    /// Whitelist passed at registration; compiles on first call with the
    /// argument kinds that call observes.
    Pending,
    /// Never native — failed the whitelist, inference, or codegen.
    Refused,
    /// One compiled specialization; the entry guard is an exact match on
    /// its param_kinds.
    Ready(JittedFn),
}

/// Per-VM JIT state. The module owns the executable memory; function
/// pointers stay valid until the module (and thus the VM) drops.
pub struct JitCache {
    module: Option<JITModule>,
    table: Vec<Option<Slot>>,
    /// Builtin names shadowed by a user definition. Compiled code that
    /// baked one of these natives is demoted the moment the shadow
    /// appears (see note_shadow); specialization refuses them up front.
    shadowed: std::collections::HashSet<String>,
    /// Functions whose native body can never pay for the bytecode→native
    /// boundary (trivial constructors — see boundary_unprofitable). They
    /// still compile as group members, where callers reach them by direct
    /// native call; only `has` says no, so bytecode callers skip the
    /// marshalling entirely.
    boundary_skip: Vec<bool>,
    /// One scratch context reused for every boundary call. Native calls
    /// never nest (a native body reaches other natives by direct call,
    /// sharing the ctx it was handed), so a single buffer is safe — and
    /// its vecs keep their capacity between calls.
    scratch: ScratchCtx,
    pub compiled: u64,
    pub native_calls: u64,
}

// SAFETY: the module's executable memory is immutable after
// finalize_definitions; mutation only happens through &mut self on the
// owning (single-threaded) VM. Function pointers are plain code addresses.
unsafe impl Send for JitCache {}

impl Drop for JitCache {
    fn drop(&mut self) {
        // Cranelift's `JITModule` does NOT release its mmap'd executable
        // pages on a normal drop — `free_memory` must be called explicitly,
        // or every module leaks its compiled code. This is acute for a
        // program that `spawn`s a worker pool per tick: each worker clones
        // the interpreter, JIT-compiles its own copy of the hot functions,
        // and — without this — leaked the code pages when it finished (the
        // per-tick soak leak). Safe here: the cache owns the module and
        // every function pointer into it lives in `self.table`, which is
        // dropped alongside, so no dangling pointer can outlive the free.
        if let Some(module) = self.module.take() {
            unsafe { module.free_memory() }
        }
    }
}

impl std::fmt::Debug for JitCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JitCache")
            .field("compiled", &self.compiled)
            .field("native_calls", &self.native_calls)
            .finish()
    }
}

impl Default for JitCache {
    fn default() -> Self {
        Self::new()
    }
}

fn jit_debug() -> bool {
    std::env::var_os("OLANG_JIT_DEBUG").is_some()
}

// Field-kind codes shared between codegen and the host helper.
const FIELD_INT: u64 = 0;
const FIELD_FLOAT: u64 = 1;
const FIELD_BOOL: u64 = 2;
const FIELD_STR: u64 = 4;
/// An expect code no value matches: extraction of a side never observed
/// lowers to a guard that always deopts (and is dynamically unreachable,
/// because the pattern test on that side is always false).
const FIELD_NEVER: u64 = 0xFF;

/// Host helper the native code calls for `obj.field`. `obj` is a borrowed
/// StructObject pointer (valid for the duration of the synchronous native
/// call), `idx` a shape-resolved field index, `expect` a FIELD_* code.
/// Returns 0 and writes the raw bits on success; 1 to deopt on any
/// surprise (index out of range, unexpected field kind) — the bytecode
/// re-run then produces the canonical behavior.
///
/// # Safety
/// Called only from JIT code compiled by this module, with pointers the
/// entry guard extracted from live argument slots.
/// Host helper for the float-math builtins: calls the VM's own
/// eval_float_math, so native results are exact by construction (same
/// function, same bits). Unary ops receive b = 0.0, matching the VM's
/// unused-slot default. Total — never deopts.
extern "C" fn olang_jit_math(id: i64, a: f64, b: f64) -> f64 {
    crate::ovm::bytecode::BytecodeVm::eval_float_math(id as usize, a, b)
}

const EXPECT_STRUCT: u64 = 3;

/// Host helper for `list[i]`: negative indices count from the end
/// (matching the VM), bounds violations and element-kind surprises
/// deopt. Struct elements return a borrowed pointer into the Vec —
/// valid because the list is immutable for the duration of the
/// synchronous native call.
///
/// # Safety
/// Called only from JIT code with pointers extracted from live slots.
unsafe extern "C" fn olang_jit_index(
    list: *const Vec<OvmValue>,
    idx: i64,
    expect: u64,
    expect_shape: u64,
    out: *mut i64,
) -> i64 {
    unsafe {
        let items = &*list;
        let len = items.len() as i64;
        // Bit 8 of `expect`: negative indices wrap (subscripts do, `for`
        // iteration does not — the VM errors there, so we deopt).
        let wrap = expect & 0x100 != 0;
        let expect = expect & 0xFF;
        let adjusted = if idx < 0 && wrap { len + idx } else { idx };
        if adjusted < 0 || adjusted >= len {
            return 1;
        }
        match (&items[adjusted as usize].data, expect) {
            (ValueData::Integer(i), FIELD_INT) => {
                *out = *i;
                0
            }
            (ValueData::Float(f), FIELD_FLOAT) => {
                *out = f.to_bits() as i64;
                0
            }
            (ValueData::Struct(s), EXPECT_STRUCT) if s.shape.id as u64 == expect_shape => {
                *out = Arc::as_ptr(s) as i64;
                0
            }
            (ValueData::String(v), FIELD_STR) => {
                *out = Arc::as_ptr(v) as i64;
                0
            }
            _ => 1,
        }
    }
}

/// Host helper: a borrowed list's length (total for list kinds).
///
/// # Safety
/// Called only from JIT code with pointers extracted from live slots.
unsafe extern "C" fn olang_jit_len(list: *const Vec<OvmValue>) -> i64 {
    unsafe { (*list).len() as i64 }
}

/// Host helper for MakeStruct: build the object from raw field bits
/// (4-bit kind codes packed in `kinds`: 0 int, 1 float, 2 bool), push
/// the Arc into the call's scratch context — so a later deopt can never
/// leak it — and hand native code a borrowed pointer.
///
/// # Safety
/// Called only from JIT code; `shape` points at the Arc inside the
/// owning CompiledBytecode (kept alive by the JittedFn), `fields` at a
/// stack buffer of `n` slots.
unsafe extern "C" fn olang_jit_make_struct(
    ctx: *mut ScratchCtx,
    shape: *const Arc<crate::ovm::value::StructShape>,
    fields: *const i64,
    n: i64,
    kinds: i64,
) -> i64 {
    unsafe {
        let mut values = Vec::with_capacity(n as usize);
        for i in 0..n as usize {
            let bits = *fields.add(i);
            values.push(match (kinds >> (i * 4)) & 0xF {
                0 => OvmValue::new_integer(bits),
                2 => OvmValue::new_boolean(bits != 0),
                _ => OvmValue::new_float(f64::from_bits(bits as u64)),
            });
        }
        let ctx = &mut *ctx;
        // Everything allocated in one native call lives until the call
        // resolves; cap it so allocation-heavy loops deopt to bytecode
        // instead of holding unbounded memory. Null tells codegen to deopt.
        if ctx.allocs.len() >= 1_000_000 {
            return 0;
        }
        let obj = Arc::new(crate::ovm::value::StructObject {
            shape: (*shape).clone(),
            values,
        });
        let ptr = Arc::as_ptr(&obj) as i64;
        ctx.allocs.push(obj);
        ptr
    }
}

/// Host helper for returning a struct: resolve the borrowed pointer back
/// to an owned Arc — a scratch allocation or an entry argument — and
/// park it in `retained` for the VM to take. Unknown pointers (e.g. a
/// list element) deopt; the bytecode re-run returns it the ordinary way.
///
/// # Safety
/// Called only from JIT code with the call's own ctx.
unsafe extern "C" fn olang_jit_retain(ctx: *mut ScratchCtx, ptr: i64) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        // Called exactly once, at the entry boundary. The returned struct is
        // almost always the newest allocation; check it first, then scan.
        if let Some(last) = ctx.allocs.last()
            && Arc::as_ptr(last) as i64 == ptr
        {
            ctx.retained = Some(last.clone());
            return 0;
        }
        if let Some(a) = ctx.allocs.iter().find(|a| Arc::as_ptr(a) as i64 == ptr) {
            ctx.retained = Some(a.clone());
            return 0;
        }
        if let Some(a) = ctx.args.iter().find(|a| Arc::as_ptr(a) as i64 == ptr) {
            ctx.retained = Some(a.clone());
            return 0;
        }
        1
    }
}

/// Host helper for string comparisons: the same Rust operators the VM
/// uses, on borrowed strings. Op codes: 0 ==, 1 !=, 2 <, 3 <=, 4 >, 5 >=.
///
/// # Safety
/// Called only from JIT code with pointers extracted from live slots.
unsafe extern "C" fn olang_jit_str_cmp(a: *const String, b: *const String, op: i64) -> i64 {
    unsafe {
        let (a, b) = (&*a, &*b);
        (match op {
            0 => a == b,
            1 => a != b,
            2 => a < b,
            3 => a <= b,
            4 => a > b,
            _ => a >= b,
        }) as i64
    }
}

/// Host helper for string concatenation, scratch-owned like MakeStruct.
/// Null return = cap reached, deopt.
///
/// # Safety
/// Called only from JIT code with the call's own ctx and live pointers.
unsafe extern "C" fn olang_jit_str_concat(
    ctx: *mut ScratchCtx,
    a: *const String,
    b: *const String,
) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if ctx.str_allocs.len() >= 1_000_000 {
            return 0;
        }
        let s = Arc::new(format!("{}{}", *a, *b));
        let ptr = Arc::as_ptr(&s) as i64;
        ctx.str_allocs.push(s);
        ptr
    }
}

/// String twin of olang_jit_retain: resolve a returned borrowed pointer
/// to an owned Arc at the entry boundary.
///
/// # Safety
/// Called only from JIT code with the call's own ctx.
unsafe extern "C" fn olang_jit_str_retain(ctx: *mut ScratchCtx, ptr: i64) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if let Some(last) = ctx.str_allocs.last()
            && Arc::as_ptr(last) as i64 == ptr
        {
            ctx.retained_str = Some(last.clone());
            return 0;
        }
        if let Some(s) = ctx
            .str_allocs
            .iter()
            .chain(ctx.str_args.iter())
            .find(|s| Arc::as_ptr(s) as i64 == ptr)
        {
            ctx.retained_str = Some(s.clone());
            return 0;
        }
        1
    }
}

/// Host helper for PatternTestResult: does the borrowed Result have the
/// asked-for side? Total (any Result answers), so no guard status.
///
/// # Safety
/// Called only from JIT code with pointers extracted from live slots.
unsafe extern "C" fn olang_jit_result_test(
    res: *const crate::ovm::value::ResultObject,
    want_ok: i64,
) -> i64 {
    unsafe {
        let r = &*res;
        (if want_ok != 0 {
            r.ok.is_some()
        } else {
            r.err.is_some()
        }) as i64
    }
}

/// Host helper for ExtractResult: guarded payload read (FIELD_* expect
/// codes, same as olang_jit_field). String payloads hand out a borrowed
/// pointer — valid because the ResultObject is an entry argument or a
/// scratch allocation, both alive for the whole synchronous call.
///
/// # Safety
/// Called only from JIT code with pointers extracted from live slots.
unsafe extern "C" fn olang_jit_result_extract(
    res: *const crate::ovm::value::ResultObject,
    want_ok: i64,
    expect: u64,
    out: *mut i64,
) -> i64 {
    unsafe {
        let r = &*res;
        let side = if want_ok != 0 { &r.ok } else { &r.err };
        match side.as_ref().map(|v| &v.data) {
            Some(ValueData::Integer(i)) if expect == FIELD_INT => {
                *out = *i;
                0
            }
            Some(ValueData::Float(f)) if expect == FIELD_FLOAT => {
                *out = f.to_bits() as i64;
                0
            }
            Some(ValueData::Boolean(b)) if expect == FIELD_BOOL => {
                *out = *b as i64;
                0
            }
            Some(ValueData::String(s)) if expect == FIELD_STR => {
                *out = Arc::as_ptr(s) as i64;
                0
            }
            _ => 1,
        }
    }
}

/// Host helper for MakeResult: build Ok/Err around a scalar payload
/// (kind codes 0 int, 1 float, 2 bool — same as MakeStruct), push the
/// Arc into the scratch context, hand back a borrowed pointer. Null
/// return = cap reached, deopt.
///
/// # Safety
/// Called only from JIT code with the call's own ctx.
unsafe extern "C" fn olang_jit_make_result(
    ctx: *mut ScratchCtx,
    ok: i64,
    kind: i64,
    bits: i64,
) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if ctx.result_allocs.len() >= 1_000_000 {
            return 0;
        }
        let inner = match kind {
            0 => OvmValue::new_integer(bits),
            2 => OvmValue::new_boolean(bits != 0),
            _ => OvmValue::new_float(f64::from_bits(bits as u64)),
        };
        let (ok_side, err_side) = if ok != 0 {
            (Some(inner), None)
        } else {
            (None, Some(inner))
        };
        let obj = Arc::new(crate::ovm::value::ResultObject {
            ok: ok_side,
            err: err_side,
        });
        let ptr = Arc::as_ptr(&obj) as i64;
        ctx.result_allocs.push(obj);
        ptr
    }
}

/// Result twin of olang_jit_retain: resolve a returned borrowed pointer
/// to an owned Arc at the entry boundary.
///
/// # Safety
/// Called only from JIT code with the call's own ctx.
unsafe extern "C" fn olang_jit_result_retain(ctx: *mut ScratchCtx, ptr: i64) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if let Some(last) = ctx.result_allocs.last()
            && Arc::as_ptr(last) as i64 == ptr
        {
            ctx.retained_result = Some(last.clone());
            return 0;
        }
        if let Some(r) = ctx
            .result_allocs
            .iter()
            .chain(ctx.result_args.iter())
            .find(|r| Arc::as_ptr(r) as i64 == ptr)
        {
            ctx.retained_result = Some(r.clone());
            return 0;
        }
        1
    }
}

/// Host helper for MakeList: build a uniform scalar list from raw bits
/// (kind codes 0 int, 1 float — inference proved uniformity), push the
/// Arc into the scratch context, hand back a borrowed pointer. Null
/// return = cap reached, deopt.
///
/// # Safety
/// Called only from JIT code with the call's own ctx; `elems` points at
/// a stack buffer of `n` slots.
unsafe extern "C" fn olang_jit_make_list(
    ctx: *mut ScratchCtx,
    elems: *const i64,
    n: i64,
    kind: i64,
) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if ctx.list_allocs.len() >= 1_000_000 {
            return 0;
        }
        let mut values = Vec::with_capacity(n as usize);
        for i in 0..n as usize {
            let bits = *elems.add(i);
            values.push(if kind == 0 {
                OvmValue::new_integer(bits)
            } else {
                OvmValue::new_float(f64::from_bits(bits as u64))
            });
        }
        let list = Arc::new(values);
        let ptr = Arc::as_ptr(&list) as i64;
        ctx.list_allocs.push(list);
        ptr
    }
}

/// Host helper for list + list: clone both sides into a fresh
/// scratch-owned list, exactly the VM's concat. Element kinds were
/// proven equal by inference, so no per-element guard is needed. Null
/// return = cap reached, deopt.
///
/// # Safety
/// Called only from JIT code with the call's own ctx and live pointers.
unsafe extern "C" fn olang_jit_list_concat(
    ctx: *mut ScratchCtx,
    a: *const Vec<OvmValue>,
    b: *const Vec<OvmValue>,
) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if ctx.list_allocs.len() >= 1_000_000 {
            return 0;
        }
        let (a, b) = (&*a, &*b);
        let mut items = Vec::with_capacity(a.len() + b.len());
        items.extend(a.iter().cloned());
        items.extend(b.iter().cloned());
        let list = Arc::new(items);
        let ptr = Arc::as_ptr(&list) as i64;
        ctx.list_allocs.push(list);
        ptr
    }
}

/// List twin of olang_jit_retain: resolve a returned borrowed pointer
/// to an owned Arc at the entry boundary.
///
/// # Safety
/// Called only from JIT code with the call's own ctx.
unsafe extern "C" fn olang_jit_list_retain(ctx: *mut ScratchCtx, ptr: i64) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if let Some(last) = ctx.list_allocs.last()
            && Arc::as_ptr(last) as i64 == ptr
        {
            ctx.retained_list = Some(last.clone());
            return 0;
        }
        if let Some(l) = ctx
            .list_allocs
            .iter()
            .chain(ctx.list_args.iter())
            .find(|l| Arc::as_ptr(l) as i64 == ptr)
        {
            ctx.retained_list = Some(l.clone());
            return 0;
        }
        1
    }
}

/// Struct twin of olang_jit_make_list: each element arrives as a
/// borrowed struct pointer, resolved back to an owned Arc — a scratch
/// allocation (newest first, elements are usually just built) or an
/// entry argument. An unknown pointer deopts, like the cap.
///
/// # Safety
/// Called only from JIT code with the call's own ctx; `elems` points at
/// a stack buffer of `n` slots.
unsafe extern "C" fn olang_jit_make_list_structs(
    ctx: *mut ScratchCtx,
    elems: *const i64,
    n: i64,
) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if ctx.list_allocs.len() >= 1_000_000 {
            return 0;
        }
        let mut values = Vec::with_capacity(n as usize);
        for i in 0..n as usize {
            let ptr = *elems.add(i);
            let arc = ctx
                .allocs
                .iter()
                .rev()
                .find(|a| Arc::as_ptr(a) as i64 == ptr)
                .or_else(|| ctx.args.iter().find(|a| Arc::as_ptr(a) as i64 == ptr));
            match arc {
                Some(a) => values.push(OvmValue::new_struct(a.clone())),
                None => return 0,
            }
        }
        let list = Arc::new(values);
        let ptr = Arc::as_ptr(&list) as i64;
        ctx.list_allocs.push(list);
        ptr
    }
}

/// String twin of olang_jit_make_list: each element arrives as a
/// borrowed string pointer. A scratch allocation or entry argument
/// resolves to its owned Arc; anything else (a baked constant, a list
/// element) is content-cloned — strings are immutable values with
/// content equality, so identity is unobservable. Null = cap, deopt.
///
/// # Safety
/// Called only from JIT code with the call's own ctx; `elems` points at
/// a stack buffer of `n` slots holding live `*const String` values.
unsafe extern "C" fn olang_jit_make_list_strs(
    ctx: *mut ScratchCtx,
    elems: *const i64,
    n: i64,
) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if ctx.list_allocs.len() >= 1_000_000 {
            return 0;
        }
        let mut values = Vec::with_capacity(n as usize);
        for i in 0..n as usize {
            let ptr = *elems.add(i);
            let arc = ctx
                .str_allocs
                .iter()
                .rev()
                .find(|a| Arc::as_ptr(a) as i64 == ptr)
                .or_else(|| ctx.str_args.iter().find(|a| Arc::as_ptr(a) as i64 == ptr))
                .cloned()
                .unwrap_or_else(|| Arc::new((*(ptr as *const String)).clone()));
            values.push(OvmValue {
                data: ValueData::String(arc),
            });
        }
        let list = Arc::new(values);
        let ptr = Arc::as_ptr(&list) as i64;
        ctx.list_allocs.push(list);
        ptr
    }
}

/// Host helper for map_get on a map receiver: key-guarded read with the
/// FIELD_* expect codes. A missing key (the VM returns Unit) or a value
/// surprise deopts — bytecode then produces the canonical answer.
///
/// # Safety
/// Called only from JIT code with pointers extracted from live slots.
unsafe extern "C" fn olang_jit_map_get(
    map: *const HashMap<String, OvmValue>,
    key: *const String,
    expect: u64,
    out: *mut i64,
) -> i64 {
    unsafe {
        let m = &*map;
        let k = &*key;
        match m.get(k.as_str()).map(|v| &v.data) {
            Some(ValueData::Integer(i)) if expect == FIELD_INT => {
                *out = *i;
                0
            }
            Some(ValueData::Float(f)) if expect == FIELD_FLOAT => {
                *out = f.to_bits() as i64;
                0
            }
            Some(ValueData::Boolean(b)) if expect == FIELD_BOOL => {
                *out = *b as i64;
                0
            }
            Some(ValueData::String(s)) if expect == FIELD_STR => {
                *out = Arc::as_ptr(s) as i64;
                0
            }
            _ => 1,
        }
    }
}

/// Host helper for map_has_key on a map receiver: total, no guard.
///
/// # Safety
/// Called only from JIT code with pointers extracted from live slots.
unsafe extern "C" fn olang_jit_map_has(
    map: *const HashMap<String, OvmValue>,
    key: *const String,
) -> i64 {
    unsafe { (*map).contains_key((*key).as_str()) as i64 }
}

/// Host helper for map_set: clone-and-insert, exactly the VM's native
/// (maps are immutable values). The new map is scratch-owned. Value kind
/// codes: 0 int, 2 bool, 4 string (content-cloned), else float. Null =
/// cap reached, deopt.
///
/// # Safety
/// Called only from JIT code with the call's own ctx and live pointers.
unsafe extern "C" fn olang_jit_map_set(
    ctx: *mut ScratchCtx,
    map: *const HashMap<String, OvmValue>,
    key: *const String,
    kind: i64,
    bits: i64,
) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if ctx.map_allocs.len() >= 1_000_000 {
            return 0;
        }
        let mut new_map = (*map).clone();
        new_map.insert((*key).clone(), scalar_from_bits(kind, bits));
        let arc = Arc::new(new_map);
        let ptr = Arc::as_ptr(&arc) as i64;
        ctx.map_allocs.push(arc);
        ptr
    }
}

/// Host helper for MakeMap: string keys (content-cloned — strings are
/// immutable values), a uniform scalar or string value payload. The map
/// is scratch-owned; a duplicate key overwrites, exactly like the VM.
/// Null = cap reached, deopt.
///
/// # Safety
/// Called only from JIT code with the call's own ctx; `keys` and `vals`
/// point at stack buffers of `n` slots each.
unsafe extern "C" fn olang_jit_make_map(
    ctx: *mut ScratchCtx,
    keys: *const i64,
    vals: *const i64,
    n: i64,
    kind: i64,
) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if ctx.map_allocs.len() >= 1_000_000 {
            return 0;
        }
        let mut map = HashMap::with_capacity(n as usize);
        for i in 0..n as usize {
            let key = (*(*keys.add(i) as *const String)).clone();
            map.insert(key, scalar_from_bits(kind, *vals.add(i)));
        }
        let arc = Arc::new(map);
        let ptr = Arc::as_ptr(&arc) as i64;
        ctx.map_allocs.push(arc);
        ptr
    }
}

/// Decode a scalar-or-string value from its raw bits (codes above).
///
/// # Safety
/// For code 4, `bits` must be a live `*const String`.
unsafe fn scalar_from_bits(kind: i64, bits: i64) -> OvmValue {
    unsafe {
        match kind {
            0 => OvmValue::new_integer(bits),
            2 => OvmValue::new_boolean(bits != 0),
            4 => OvmValue {
                data: ValueData::String(Arc::new((*(bits as *const String)).clone())),
            },
            _ => OvmValue::new_float(f64::from_bits(bits as u64)),
        }
    }
}

/// Map twin of olang_jit_retain: resolve a returned borrowed pointer to
/// an owned Arc at the entry boundary.
///
/// # Safety
/// Called only from JIT code with the call's own ctx.
unsafe extern "C" fn olang_jit_map_retain(ctx: *mut ScratchCtx, ptr: i64) -> i64 {
    unsafe {
        let ctx = &mut *ctx;
        if let Some(last) = ctx.map_allocs.last()
            && Arc::as_ptr(last) as i64 == ptr
        {
            ctx.retained_map = Some(last.clone());
            return 0;
        }
        if let Some(m) = ctx
            .map_allocs
            .iter()
            .chain(ctx.map_args.iter())
            .find(|m| Arc::as_ptr(m) as i64 == ptr)
        {
            ctx.retained_map = Some(m.clone());
            return 0;
        }
        1
    }
}

/// Host helper at loop entry: remember every scratch family's length.
///
/// # Safety
/// Called only from JIT code with the call's own ctx.
unsafe extern "C" fn olang_jit_mark(ctx: *mut ScratchCtx) {
    unsafe {
        let ctx = &mut *ctx;
        ctx.mark = Some([
            ctx.allocs.len(),
            ctx.str_allocs.len(),
            ctx.result_allocs.len(),
            ctx.list_allocs.len(),
            ctx.map_allocs.len(),
        ]);
    }
}

/// Host helper at each loop back-edge: free the iteration's allocations
/// by truncating every family to the loop-entry mark. Sound because the
/// compilation gate proved no pointer into them survives the iteration;
/// values that escaped into another owner survive on their own Arc.
///
/// # Safety
/// Called only from JIT code with the call's own ctx.
unsafe extern "C" fn olang_jit_release(ctx: *mut ScratchCtx) {
    unsafe {
        let ctx = &mut *ctx;
        if let Some([a, s, r, l, m]) = ctx.mark {
            ctx.allocs.truncate(a);
            ctx.str_allocs.truncate(s);
            ctx.result_allocs.truncate(r);
            ctx.list_allocs.truncate(l);
            ctx.map_allocs.truncate(m);
        }
    }
}

unsafe extern "C" fn olang_jit_field(
    obj: *const crate::ovm::value::StructObject,
    idx: u64,
    expect: u64,
    out: *mut i64,
) -> i64 {
    unsafe {
        let obj = &*obj;
        match obj.values.get(idx as usize).map(|v| &v.data) {
            Some(ValueData::Integer(i)) if expect == FIELD_INT => {
                *out = *i;
                0
            }
            Some(ValueData::Float(f)) if expect == FIELD_FLOAT => {
                *out = f.to_bits() as i64;
                0
            }
            Some(ValueData::Boolean(b)) if expect == FIELD_BOOL => {
                *out = *b as i64;
                0
            }
            Some(ValueData::String(s)) if expect == FIELD_STR => {
                *out = Arc::as_ptr(s) as i64;
                0
            }
            _ => 1,
        }
    }
}

impl JitCache {
    pub fn new() -> Self {
        Self {
            module: None,
            table: Vec::new(),
            shadowed: std::collections::HashSet::new(),
            boundary_skip: Vec::new(),
            scratch: ScratchCtx::default(),
            compiled: 0,
            native_calls: 0,
        }
    }

    fn module(&mut self) -> Option<&mut JITModule> {
        if self.module.is_none() {
            // Speed over compile time; the functions are tiny.
            let mut builder = JITBuilder::with_flags(
                &[("opt_level", "speed")],
                cranelift_module::default_libcall_names(),
            )
            .ok()?;
            builder.symbol("olang_jit_field", olang_jit_field as *const u8);
            builder.symbol("olang_jit_math", olang_jit_math as *const u8);
            builder.symbol("olang_jit_index", olang_jit_index as *const u8);
            builder.symbol("olang_jit_len", olang_jit_len as *const u8);
            builder.symbol("olang_jit_make_struct", olang_jit_make_struct as *const u8);
            builder.symbol("olang_jit_retain", olang_jit_retain as *const u8);
            builder.symbol("olang_jit_str_cmp", olang_jit_str_cmp as *const u8);
            builder.symbol("olang_jit_str_concat", olang_jit_str_concat as *const u8);
            builder.symbol("olang_jit_str_retain", olang_jit_str_retain as *const u8);
            builder.symbol("olang_jit_result_test", olang_jit_result_test as *const u8);
            builder.symbol(
                "olang_jit_result_extract",
                olang_jit_result_extract as *const u8,
            );
            builder.symbol("olang_jit_make_result", olang_jit_make_result as *const u8);
            builder.symbol(
                "olang_jit_result_retain",
                olang_jit_result_retain as *const u8,
            );
            builder.symbol("olang_jit_make_list", olang_jit_make_list as *const u8);
            builder.symbol(
                "olang_jit_make_list_structs",
                olang_jit_make_list_structs as *const u8,
            );
            builder.symbol(
                "olang_jit_make_list_strs",
                olang_jit_make_list_strs as *const u8,
            );
            builder.symbol("olang_jit_map_get", olang_jit_map_get as *const u8);
            builder.symbol("olang_jit_map_has", olang_jit_map_has as *const u8);
            builder.symbol("olang_jit_map_set", olang_jit_map_set as *const u8);
            builder.symbol("olang_jit_make_map", olang_jit_make_map as *const u8);
            builder.symbol("olang_jit_map_retain", olang_jit_map_retain as *const u8);
            builder.symbol("olang_jit_mark", olang_jit_mark as *const u8);
            builder.symbol("olang_jit_release", olang_jit_release as *const u8);
            builder.symbol("olang_jit_list_concat", olang_jit_list_concat as *const u8);
            builder.symbol("olang_jit_list_retain", olang_jit_list_retain as *const u8);
            self.module = Some(JITModule::new(builder));
        }
        self.module.as_mut()
    }

    /// Called at bytecode registration: run the cheap syntactic whitelist
    /// so functions that can never compile pay nothing per call later.
    pub fn try_compile(&mut self, func_id: FunctionId, bytecode: &CompiledBytecode) {
        let idx = func_id.index();
        if self.table.len() <= idx {
            self.table.resize_with(idx + 1, || None);
        }
        if self.table[idx].is_some() {
            return;
        }
        if whitelist_ok(bytecode) {
            self.table[idx] = Some(Slot::Pending);
            if boundary_unprofitable(bytecode) {
                if self.boundary_skip.len() <= idx {
                    self.boundary_skip.resize(idx + 1, false);
                }
                self.boundary_skip[idx] = true;
                if jit_debug() {
                    eprintln!(
                        "[jit] fn#{} qualifies but stays bytecode at the call boundary (trivial constructor)",
                        idx
                    );
                }
            }
        } else {
            if jit_debug() {
                eprintln!(
                    "[jit] fn#{} does not qualify; instructions: {:?}",
                    idx,
                    bytecode
                        .instructions
                        .iter()
                        .map(instruction_name)
                        .collect::<Vec<_>>()
                );
            }
            self.table[idx] = Some(Slot::Refused);
        }
    }

    /// A user definition now shadows the builtin `name` (the VM calls
    /// this from its own shadow bookkeeping). Any compiled entry that
    /// baked a direct call to that native is stale: demote it so calls
    /// fall back to bytecode, which resolves the name the ordinary way.
    pub fn note_shadow(&mut self, name: &str) {
        if !self.shadowed.insert(name.to_string()) {
            return;
        }
        for slot in self.table.iter_mut() {
            if let Some(Slot::Ready(j)) = slot
                && j.baked_builtins.iter().any(|b| b == name)
            {
                *slot = Some(Slot::Refused);
            }
        }
    }

    /// True when a native run is possible or still decidable — callers
    /// use it to decide whether extracting argument values is worth it.
    /// Boundary-unprofitable bodies answer false even once compiled:
    /// their native form only exists for direct calls from group members.
    #[inline]
    pub fn has(&self, func_id: FunctionId) -> bool {
        let idx = func_id.index();
        matches!(
            self.table.get(idx),
            Some(Some(Slot::Pending)) | Some(Some(Slot::Ready(_)))
        ) && !self.boundary_skip.get(idx).copied().unwrap_or(false)
    }

    /// Run the native body if the argument kinds fit (compiling the
    /// specialization group on first use). None means "no native run
    /// happened" — the caller proceeds on bytecode exactly as before.
    #[inline]
    pub fn try_call(
        &mut self,
        func_id: FunctionId,
        bytecode: &Arc<CompiledBytecode>,
        args: &[OvmValue],
        remaining_depth: u32,
        lookup: &BytecodeLookup,
    ) -> Option<OvmValue> {
        let mut bits = [0i64; MAX_PARAMS];
        let mut kinds = [Kind::Int; MAX_PARAMS];
        if args.len() > MAX_PARAMS {
            return None;
        }
        let mut any_ref = false;
        for (i, arg) in args.iter().enumerate() {
            match &arg.data {
                ValueData::Integer(v) => {
                    bits[i] = *v;
                    kinds[i] = Kind::Int;
                }
                ValueData::Float(f) => {
                    bits[i] = f.to_bits() as i64;
                    kinds[i] = Kind::Float;
                }
                ValueData::Struct(obj) => {
                    bits[i] = Arc::as_ptr(obj) as i64;
                    kinds[i] = Kind::Struct(obj.shape.id);
                    any_ref = true;
                }
                ValueData::List(items) => {
                    kinds[i] = classify_list(items)?;
                    bits[i] = Arc::as_ptr(items) as i64;
                    any_ref = true;
                }
                ValueData::String(s) => {
                    bits[i] = Arc::as_ptr(s) as i64;
                    kinds[i] = Kind::Str;
                    any_ref = true;
                }
                ValueData::Result(r) => {
                    kinds[i] = classify_result(r)?;
                    bits[i] = Arc::as_ptr(r) as i64;
                    any_ref = true;
                }
                ValueData::Map(m) => {
                    kinds[i] = classify_map(m)?;
                    bits[i] = Arc::as_ptr(m) as i64;
                    any_ref = true;
                }
                _ => return None,
            }
        }
        // Shape specs are only needed to SPECIALIZE (first call); Ready
        // calls skip the allocation entirely. Per-read helper guards keep
        // same-shape/different-kind instances safe either way.
        let mut shapes: HashMap<u32, ShapeSpec> = HashMap::new();
        if matches!(self.table.get(func_id.index()), Some(Some(Slot::Pending))) {
            for arg in args {
                if let ValueData::Struct(obj) = &arg.data {
                    let (_, spec) = observe_struct(obj)?;
                    shapes.entry(obj.shape.id).or_insert(spec);
                }
                if let ValueData::List(items) = &arg.data
                    && let Some(ValueData::Struct(first)) = items.first().map(|v| &v.data)
                {
                    let (_, spec) = observe_struct(first)?;
                    shapes.entry(first.shape.id).or_insert(spec);
                }
            }
        }
        let mut struct_args: Vec<Arc<crate::ovm::value::StructObject>> = Vec::new();
        let mut str_args: Vec<Arc<String>> = Vec::new();
        let mut result_args: Vec<Arc<crate::ovm::value::ResultObject>> = Vec::new();
        let mut list_args: Vec<Arc<Vec<OvmValue>>> = Vec::new();
        let mut map_args: Vec<Arc<HashMap<String, OvmValue>>> = Vec::new();
        // The per-family sweep only matters when a reference-kind argument
        // exists; all-scalar calls (the common boundary) skip it whole.
        if any_ref {
            for arg in args {
                if let ValueData::Struct(obj) = &arg.data {
                    struct_args.push(obj.clone());
                }
                if let ValueData::String(s) = &arg.data {
                    str_args.push(s.clone());
                }
                if let ValueData::Result(r) = &arg.data {
                    result_args.push(r.clone());
                }
                if let ValueData::List(l) = &arg.data {
                    list_args.push(l.clone());
                }
                if let ValueData::Map(m) = &arg.data {
                    map_args.push(m.clone());
                }
            }
        }
        self.try_call_raw_with_shapes(
            func_id,
            bytecode,
            &bits[..args.len()],
            &kinds[..args.len()],
            remaining_depth,
            lookup,
            &shapes,
            &struct_args,
            &str_args,
            &result_args,
            &list_args,
            &map_args,
        )
    }

    /// True when the function's first native call hasn't happened yet —
    /// callers use it to know whether shape specs must be gathered.
    #[inline]
    /// True when `func_id` compiled and its return can be unmarshalled
    /// faithfully at the raw entry: tuple returns carry only scalar
    /// element kinds (the raw unmarshal reinterprets heap-kind tuple
    /// slots as numbers). OSR checks this once after specializing a
    /// region before trusting its live-out tuple.
    pub fn ready_tuple_ret_scalar(&self, func_id: FunctionId) -> bool {
        match self.table.get(func_id.index()).and_then(|s| s.as_ref()) {
            Some(Slot::Ready(j)) => match &j.ret_tuple {
                Some(ks) => ks
                    .iter()
                    .all(|k| matches!(k, Kind::Int | Kind::Bool | Kind::Float)),
                None => true,
            },
            _ => false,
        }
    }

    pub fn is_pending(&self, func_id: FunctionId) -> bool {
        matches!(self.table.get(func_id.index()), Some(Some(Slot::Pending)))
    }

    /// Like try_call, but the caller already extracted raw bits and kinds
    /// (no struct arguments on this path).
    pub fn try_call_raw(
        &mut self,
        func_id: FunctionId,
        bytecode: &Arc<CompiledBytecode>,
        bits: &[i64],
        kinds: &[Kind],
        remaining_depth: u32,
        lookup: &BytecodeLookup,
    ) -> Option<OvmValue> {
        let shapes = HashMap::new();
        self.try_call_raw_with_shapes(
            func_id,
            bytecode,
            bits,
            kinds,
            remaining_depth,
            lookup,
            &shapes,
            &[],
            &[],
            &[],
            &[],
            &[],
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn try_call_raw_with_shapes(
        &mut self,
        func_id: FunctionId,
        bytecode: &Arc<CompiledBytecode>,
        bits: &[i64],
        kinds: &[Kind],
        remaining_depth: u32,
        lookup: &BytecodeLookup,
        shapes: &HashMap<u32, ShapeSpec>,
        args_for_ctx: &[Arc<crate::ovm::value::StructObject>],
        str_args_for_ctx: &[Arc<String>],
        result_args_for_ctx: &[Arc<crate::ovm::value::ResultObject>],
        list_args_for_ctx: &[Arc<Vec<OvmValue>>],
        map_args_for_ctx: &[Arc<HashMap<String, OvmValue>>],
    ) -> Option<OvmValue> {
        let idx = func_id.index();
        match self.table.get(idx)? {
            Some(Slot::Ready(_)) => {}
            Some(Slot::Pending) => {
                // First call: specialize the whole reachable group on the
                // kinds this call carries.
                if self
                    .specialize_group(func_id, bytecode, kinds, lookup, shapes)
                    .is_none()
                {
                    if jit_debug() {
                        eprintln!("[jit] fn#{} refused (inference or codegen)", idx);
                    }
                    self.table[idx] = Some(Slot::Refused);
                    return None;
                }
            }
            _ => return None,
        }

        let Some(Slot::Ready(jitted)) = self.table.get(idx)?.as_ref() else {
            return None;
        };
        if kinds != jitted.param_kinds.as_slice() {
            return None;
        }
        let mut out = [0i64; MAX_TUPLE];
        let ctx = &mut self.scratch;
        ctx.clear();
        for arg in args_for_ctx {
            ctx.args.push(arg.clone());
        }
        for s in str_args_for_ctx {
            ctx.str_args.push(s.clone());
        }
        for r in result_args_for_ctx {
            ctx.result_args.push(r.clone());
        }
        for l in list_args_for_ctx {
            ctx.list_args.push(l.clone());
        }
        for m in map_args_for_ctx {
            ctx.map_args.push(m.clone());
        }
        let ctx_ptr: *mut ScratchCtx = ctx;
        let status = unsafe {
            (jitted.entry)(
                bits.as_ptr(),
                remaining_depth as i64,
                ctx_ptr,
                out.as_mut_ptr(),
            )
        };
        // One exit below: whatever the call left in the scratch context —
        // deopt leftovers or the temporaries around a retained return —
        // drops now, not at some later boundary call.
        let result = if status != STATUS_OK {
            None
        } else if let Some(tk) = &jitted.ret_tuple {
            let elems: Vec<OvmValue> = tk
                .iter()
                .zip(out.iter())
                .map(|(k, bits)| match k {
                    Kind::Int => OvmValue::new_integer(*bits),
                    Kind::Bool => OvmValue::new_boolean(*bits != 0),
                    _ => OvmValue::new_float(f64::from_bits(*bits as u64)),
                })
                .collect();
            Some(OvmValue::new_tuple(elems))
        } else {
            match jitted.ret_kind {
                Kind::Int => Some(OvmValue::new_integer(out[0])),
                Kind::Bool => Some(OvmValue::new_boolean(out[0] != 0)),
                Kind::Float => Some(OvmValue::new_float(f64::from_bits(out[0] as u64))),
                Kind::Struct(_) => ctx.retained.take().map(OvmValue::new_struct),
                Kind::Str => ctx.retained_str.take().map(|arc| OvmValue {
                    data: ValueData::String(arc),
                }),
                Kind::Result(..) => ctx.retained_result.take().map(|arc| OvmValue {
                    data: ValueData::Result(arc),
                }),
                Kind::ListInt | Kind::ListFloat | Kind::ListStruct(_) | Kind::ListStr => {
                    ctx.retained_list.take().map(|arc| OvmValue {
                        data: ValueData::List(arc),
                    })
                }
                Kind::Map(_) => ctx.retained_map.take().map(|arc| OvmValue {
                    data: ValueData::Map(arc),
                }),
            }
        };
        ctx.clear();
        if result.is_some() {
            self.native_calls += 1;
        }
        result
    }

    /// Plan, infer, and compile the call graph reachable from `entry_id`.
    /// On success every group member becomes Ready. On failure the entry
    /// alone becomes Refused (a helper may still compile later from its
    /// own first call, with its own kinds).
    fn specialize_group(
        &mut self,
        entry_id: FunctionId,
        entry_bytecode: &Arc<CompiledBytecode>,
        entry_kinds: &[Kind],
        lookup: &BytecodeLookup,
        shapes: &HashMap<u32, ShapeSpec>,
    ) -> Option<()> {
        if entry_kinds.len() != entry_bytecode.param_count {
            return None;
        }

        // ── plan + global inference fixpoint ──
        let mut plans: Vec<PlanFn> = vec![PlanFn::new(
            entry_id,
            Arc::clone(entry_bytecode),
            entry_kinds.to_vec(),
            lookup,
        )];
        let mut plan_pos: HashMap<usize, usize> = HashMap::new();
        plan_pos.insert(entry_id.index(), 0);

        let mut group_shapes: HashMap<u32, ShapeSpec> = HashMap::new();
        for (k, v) in shapes {
            group_shapes.insert(*k, v.clone());
        }

        loop {
            let mut changed = false;

            // Signature snapshot: kinds + current ret mask per function the
            // group can call (plans lag one iteration; monotone, converges).
            let mut sigs: SigSnapshot = HashMap::new();
            for p in &plans {
                sigs.insert(
                    p.func_id.index(),
                    (
                        p.param_kinds.clone(),
                        p.ret_mask,
                        p.ret_tuple.clone(),
                        p.ret_struct,
                        p.ret_result,
                        p.ret_list,
                    ),
                );
            }
            for (i, slot) in self.table.iter().enumerate() {
                if let Some(Slot::Ready(j)) = slot {
                    sigs.insert(
                        i,
                        (
                            j.param_kinds.clone(),
                            match &j.ret_tuple {
                                Some(_) => K_TUPLE,
                                None => kind_mask(j.ret_kind),
                            },
                            j.ret_tuple.clone(),
                            match j.ret_kind {
                                Kind::Struct(sid) => Some(sid),
                                _ => None,
                            },
                            match j.ret_kind {
                                k @ (Kind::Result(..) | Kind::Map(_)) => Some(k),
                                _ => None,
                            },
                            match j.ret_kind {
                                k @ (Kind::ListInt
                                | Kind::ListFloat
                                | Kind::ListStruct(_)
                                | Kind::ListStr) => Some(k),
                                _ => None,
                            },
                        ),
                    );
                }
            }

            let mut requests: Vec<(FunctionId, Vec<Kind>)> = Vec::new();
            for plan in plans.iter_mut() {
                if plan
                    .infer_pass(
                        &sigs,
                        &group_shapes,
                        &self.shadowed,
                        &mut requests,
                        &mut changed,
                    )
                    .is_none()
                {
                    if jit_debug() {
                        eprintln!("[jit] fn#{} infer_pass failed", plan.func_id.index());
                    }
                    return None;
                }
            }

            for plan in &plans {
                for (sid, spec) in &plan.made_shapes {
                    match group_shapes.get(sid) {
                        None => {
                            group_shapes.insert(*sid, spec.clone());
                            changed = true;
                        }
                        Some(prev) if prev.field_kinds != spec.field_kinds => return None,
                        _ => {}
                    }
                }
            }

            for (fid, kinds) in requests {
                let idx = fid.index();
                if let Some(pos) = plan_pos.get(&idx) {
                    if plans[*pos].param_kinds != kinds {
                        return None; // one specialization per function
                    }
                    continue;
                }
                match self.table.get(idx) {
                    Some(Some(Slot::Ready(j))) => {
                        if j.param_kinds != kinds {
                            return None;
                        }
                        continue; // already native; sigs covers it
                    }
                    Some(Some(Slot::Refused)) => return None,
                    _ => {}
                }
                if plans.len() >= MAX_GROUP {
                    return None;
                }
                let bytecode = lookup(fid)?;
                if !whitelist_ok(&bytecode) || bytecode.param_count != kinds.len() {
                    return None;
                }
                plan_pos.insert(idx, plans.len());
                plans.push(PlanFn::new(fid, bytecode, kinds, lookup));
                changed = true;
            }

            if !changed {
                break;
            }
        }

        // ── finalize each member's inference ──
        let mut inferences: Vec<Inference> = Vec::with_capacity(plans.len());
        for plan in &plans {
            match plan.finalize() {
                Some(inf) => {
                    // Declared types must be *statically discharged*: native
                    // code never runs the VM's per-call checks, so the JIT
                    // only compiles a specialization whose parameter kinds
                    // and inferred return kind provably satisfy their
                    // annotations. Anything else stays on bytecode, which
                    // enforces per call.
                    for (i, check) in plan.bytecode.param_checks.iter().enumerate() {
                        if let (Some(check), Some(&kind)) = (check, plan.param_kinds.get(i))
                            && !kind_discharges(check, kind, &group_shapes)
                        {
                            if jit_debug() {
                                eprintln!(
                                    "[jit] fn#{} refused: parameter {} annotation not statically satisfied",
                                    plan.func_id.index(),
                                    i
                                );
                            }
                            return None;
                        }
                    }
                    if let Some(check) = &plan.bytecode.return_check {
                        let ok = match (&inf.ret_tuple, inf.ret_kind) {
                            (Some(_), _) => check.accepts("Tuple"),
                            (None, k) => kind_discharges(check, k, &group_shapes),
                        };
                        if !ok {
                            if jit_debug() {
                                eprintln!(
                                    "[jit] fn#{} refused: return annotation not statically satisfied",
                                    plan.func_id.index()
                                );
                            }
                            return None;
                        }
                    }
                    inferences.push(inf)
                }
                None => {
                    if jit_debug() {
                        eprintln!(
                            "[jit] fn#{} finalize failed; writes={:?} exotic={:?}",
                            plan.func_id.index(),
                            plan.writes,
                            plan.exotic
                        );
                    }
                    return None;
                }
            }
        }

        // ── codegen: declare everything, then define everything ──
        // Snapshot previously compiled call targets before borrowing the
        // module (both live in self).
        // Which builtin natives this group bakes: its own whitelisted
        // CallNamed sites, plus (transitively) whatever already-compiled
        // callees baked — their machine code is called directly, so a
        // shadow of THEIR builtins must demote this group too.
        let mut baked: std::collections::HashSet<String> = std::collections::HashSet::new();
        for plan in &plans {
            for inst in plan.bytecode.instructions.iter() {
                match inst {
                    Instruction::CallNamed { function_name, .. }
                        if matches!(
                            function_name.as_str(),
                            "map_get" | "map_has_key" | "map_set"
                        ) =>
                    {
                        baked.insert(function_name.clone());
                    }
                    Instruction::CallFn { func_id, .. } => {
                        if let Some(Some(Slot::Ready(j))) = self.table.get(func_id.index()) {
                            baked.extend(j.baked_builtins.iter().cloned());
                        }
                    }
                    _ => {}
                }
            }
        }
        let baked: Arc<[String]> = baked.into_iter().collect::<Vec<_>>().into();

        let mut targets: HashMap<usize, (cranelift_module::FuncId, Kind, Option<Vec<Kind>>)> =
            HashMap::new();
        for (i, slot) in self.table.iter().enumerate() {
            if let Some(Slot::Ready(j)) = slot {
                targets.insert(i, (j.clif_id, j.ret_kind, j.ret_tuple.clone()));
            }
        }
        let module = self.module()?;
        // The imported field helper, declared once per group.
        let field_helper = {
            let mut sig = module.make_signature();
            for _ in 0..4 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_field", Linkage::Import, &sig)
                .ok()?
        };
        let math_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::F64));
            sig.params.push(AbiParam::new(types::F64));
            sig.returns.push(AbiParam::new(types::F64));
            module
                .declare_function("olang_jit_math", Linkage::Import, &sig)
                .ok()?
        };
        let len_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_len", Linkage::Import, &sig)
                .ok()?
        };
        let make_struct_helper = {
            let mut sig = module.make_signature();
            for _ in 0..5 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_make_struct", Linkage::Import, &sig)
                .ok()?
        };
        let str_cmp_helper = {
            let mut sig = module.make_signature();
            for _ in 0..3 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_str_cmp", Linkage::Import, &sig)
                .ok()?
        };
        let str_concat_helper = {
            let mut sig = module.make_signature();
            for _ in 0..3 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_str_concat", Linkage::Import, &sig)
                .ok()?
        };
        let str_retain_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_str_retain", Linkage::Import, &sig)
                .ok()?
        };
        let retain_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_retain", Linkage::Import, &sig)
                .ok()?
        };
        let index_helper = {
            let mut sig = module.make_signature();
            for _ in 0..5 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_index", Linkage::Import, &sig)
                .ok()?
        };
        let result_test_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_result_test", Linkage::Import, &sig)
                .ok()?
        };
        let result_extract_helper = {
            let mut sig = module.make_signature();
            for _ in 0..4 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_result_extract", Linkage::Import, &sig)
                .ok()?
        };
        let make_result_helper = {
            let mut sig = module.make_signature();
            for _ in 0..4 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_make_result", Linkage::Import, &sig)
                .ok()?
        };
        let result_retain_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_result_retain", Linkage::Import, &sig)
                .ok()?
        };
        let make_list_helper = {
            let mut sig = module.make_signature();
            for _ in 0..4 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_make_list", Linkage::Import, &sig)
                .ok()?
        };
        let list_concat_helper = {
            let mut sig = module.make_signature();
            for _ in 0..3 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_list_concat", Linkage::Import, &sig)
                .ok()?
        };
        let make_list_structs_helper = {
            let mut sig = module.make_signature();
            for _ in 0..3 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_make_list_structs", Linkage::Import, &sig)
                .ok()?
        };
        let make_list_strs_helper = {
            let mut sig = module.make_signature();
            for _ in 0..3 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_make_list_strs", Linkage::Import, &sig)
                .ok()?
        };
        let map_get_helper = {
            let mut sig = module.make_signature();
            for _ in 0..4 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_map_get", Linkage::Import, &sig)
                .ok()?
        };
        let map_has_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_map_has", Linkage::Import, &sig)
                .ok()?
        };
        let map_set_helper = {
            let mut sig = module.make_signature();
            for _ in 0..5 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_map_set", Linkage::Import, &sig)
                .ok()?
        };
        let make_map_helper = {
            let mut sig = module.make_signature();
            for _ in 0..5 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_make_map", Linkage::Import, &sig)
                .ok()?
        };
        let map_retain_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_map_retain", Linkage::Import, &sig)
                .ok()?
        };
        let mark_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_mark", Linkage::Import, &sig)
                .ok()?
        };
        let release_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_release", Linkage::Import, &sig)
                .ok()?
        };
        let list_retain_helper = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            module
                .declare_function("olang_jit_list_retain", Linkage::Import, &sig)
                .ok()?
        };
        let mut clif_ids = Vec::with_capacity(plans.len());
        for (plan, inf) in plans.iter().zip(&inferences) {
            let mut sig = module.make_signature();
            for k in &plan.param_kinds {
                sig.params.push(AbiParam::new(k.clif_type()));
            }
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            match &inf.ret_tuple {
                Some(tk) => {
                    for k in tk {
                        sig.returns.push(AbiParam::new(k.clif_type()));
                    }
                }
                None => sig.returns.push(AbiParam::new(inf.ret_kind.clif_type())),
            }
            sig.returns.push(AbiParam::new(types::I64));
            let name = format!("olang_jit_{}", plan.func_id.index());
            let id = module.declare_function(&name, Linkage::Local, &sig).ok()?;
            clif_ids.push(id);
        }

        // Call-site resolver: group members (by plan position) override
        // any snapshot entry.
        for ((plan, inf), clif_id) in plans.iter().zip(&inferences).zip(&clif_ids) {
            targets.insert(
                plan.func_id.index(),
                (*clif_id, inf.ret_kind, inf.ret_tuple.clone()),
            );
        }

        let mut fbc = FunctionBuilderContext::new();
        for ((plan, inf), clif_id) in plans.iter().zip(&inferences).zip(&clif_ids) {
            let mut ctx = module.make_context();
            ctx.func.signature = {
                let mut sig = module.make_signature();
                for k in &plan.param_kinds {
                    sig.params.push(AbiParam::new(k.clif_type()));
                }
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(types::I64));
                match &inf.ret_tuple {
                    Some(tk) => {
                        for k in tk {
                            sig.returns.push(AbiParam::new(k.clif_type()));
                        }
                    }
                    None => sig.returns.push(AbiParam::new(inf.ret_kind.clif_type())),
                }
                sig.returns.push(AbiParam::new(types::I64));
                sig
            };
            {
                let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
                if translate_body(
                    &mut builder,
                    module,
                    &targets,
                    &plan.bytecode,
                    inf,
                    &group_shapes,
                    Helpers {
                        field: field_helper,
                        math: math_helper,
                        index: index_helper,
                        len: len_helper,
                        make_struct: make_struct_helper,
                        str_cmp: str_cmp_helper,
                        str_concat: str_concat_helper,
                        result_test: result_test_helper,
                        result_extract: result_extract_helper,
                        make_result: make_result_helper,
                        make_list: make_list_helper,
                        make_list_structs: make_list_structs_helper,
                        make_list_strs: make_list_strs_helper,
                        list_concat: list_concat_helper,
                        map_get: map_get_helper,
                        map_has: map_has_helper,
                        map_set: map_set_helper,
                        make_map: make_map_helper,
                        mark: mark_helper,
                        release: release_helper,
                    },
                )
                .is_none()
                {
                    if jit_debug() {
                        eprintln!("[jit] fn#{} translate_body failed", plan.func_id.index());
                    }
                    return None;
                }
                builder.finalize(module.target_config());
            }
            if let Err(e) = module.define_function(*clif_id, &mut ctx) {
                if jit_debug() {
                    eprintln!(
                        "[jit] define fn#{} failed: {:?}\nIR:\n{}",
                        plan.func_id.index(),
                        e,
                        ctx.func
                    );
                }
                return None;
            }
            module.clear_context(&mut ctx);
        }

        // Entry wrappers (C ABI) for every member, so each is directly
        // callable from the VM later. Float params/results travel as raw
        // bits in the i64 slots — same bytes, no conversion.
        let mut entries = Vec::with_capacity(plans.len());
        for ((plan, inf), clif_id) in plans.iter().zip(&inferences).zip(&clif_ids) {
            let mut entry_sig = module.make_signature();
            for _ in 0..4 {
                entry_sig.params.push(AbiParam::new(types::I64));
            }
            entry_sig.returns.push(AbiParam::new(types::I64));
            let entry_name = format!("olang_jit_{}_entry", plan.func_id.index());
            let entry_fid = module
                .declare_function(&entry_name, Linkage::Export, &entry_sig)
                .ok()?;

            let mut ctx = module.make_context();
            ctx.func.signature = entry_sig;
            {
                let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
                let block = builder.create_block();
                builder.append_block_params_for_function_params(block);
                builder.switch_to_block(block);
                let args_ptr = builder.block_params(block)[0];
                let depth = builder.block_params(block)[1];
                let ctx_ptr = builder.block_params(block)[2];
                let out_ptr = builder.block_params(block)[3];

                let mut call_args = Vec::with_capacity(plan.param_kinds.len() + 1);
                for (i, k) in plan.param_kinds.iter().enumerate() {
                    call_args.push(builder.ins().load(
                        k.clif_type(),
                        MemFlagsData::trusted(),
                        args_ptr,
                        (i * 8) as i32,
                    ));
                }
                call_args.push(depth);
                call_args.push(ctx_ptr);

                let inner_ref = module.declare_func_in_func(*clif_id, builder.func);
                let call = builder.ins().call(inner_ref, &call_args);
                let results = builder.inst_results(call).to_vec();
                let n_vals = results.len() - 1;
                for (slot_i, v) in results[..n_vals].iter().enumerate() {
                    builder
                        .ins()
                        .store(MemFlagsData::trusted(), *v, out_ptr, (slot_i * 8) as i32);
                }
                let status = results[n_vals];
                if matches!(
                    inf.ret_kind,
                    Kind::Struct(_)
                        | Kind::Str
                        | Kind::Result(..)
                        | Kind::ListInt
                        | Kind::ListFloat
                        | Kind::ListStruct(_)
                        | Kind::ListStr
                        | Kind::Map(_)
                ) {
                    // Ownership boundary: resolve the pointer to an owned
                    // Arc in ctx.retained; unknown pointers deopt.
                    let ok_block = builder.create_block();
                    let fail_block = builder.create_block();
                    let done_block = builder.create_block();
                    builder.append_block_param(done_block, types::I64);
                    builder.ins().brif(status, fail_block, &[], ok_block, &[]);

                    builder.switch_to_block(ok_block);
                    let which = match inf.ret_kind {
                        Kind::Str => str_retain_helper,
                        Kind::Result(..) => result_retain_helper,
                        Kind::ListInt | Kind::ListFloat | Kind::ListStruct(_) | Kind::ListStr => {
                            list_retain_helper
                        }
                        Kind::Map(_) => map_retain_helper,
                        _ => retain_helper,
                    };
                    let retain_ref = module.declare_func_in_func(which, builder.func);
                    let rcall = builder.ins().call(retain_ref, &[ctx_ptr, results[0]]);
                    let rstatus = builder.inst_results(rcall)[0];
                    builder.ins().jump(done_block, &[rstatus.into()]);

                    builder.switch_to_block(fail_block);
                    builder.ins().jump(done_block, &[status.into()]);

                    builder.switch_to_block(done_block);
                    let final_status = builder.block_params(done_block)[0];
                    builder.ins().return_(&[final_status]);
                } else {
                    builder.ins().return_(&[status]);
                }
                builder.seal_all_blocks();
                builder.finalize(module.target_config());
            }
            module.define_function(entry_fid, &mut ctx).ok()?;
            module.clear_context(&mut ctx);
            entries.push(entry_fid);
            let _ = inf; // ret kind used via targets
        }

        module.finalize_definitions().ok()?;

        for (((plan, inf), clif_id), entry_fid) in
            plans.iter().zip(&inferences).zip(&clif_ids).zip(&entries)
        {
            let code = self.module.as_ref()?.get_finalized_function(*entry_fid);
            // SAFETY: signature matches the entry ABI; memory lives as
            // long as the module (owned by this cache).
            let entry: NativeEntry = unsafe { std::mem::transmute(code) };
            let idx = plan.func_id.index();
            if self.table.len() <= idx {
                self.table.resize_with(idx + 1, || None);
            }
            if jit_debug() {
                eprintln!(
                    "[jit] fn#{} compiled to native for {:?}",
                    idx, plan.param_kinds
                );
            }
            self.table[idx] = Some(Slot::Ready(JittedFn {
                entry,
                clif_id: *clif_id,
                param_kinds: plan.param_kinds.clone(),
                ret_kind: inf.ret_kind,
                ret_tuple: inf.ret_tuple.clone(),
                baked_builtins: baked.clone(),
                _bytecode: Arc::clone(&plan.bytecode),
            }));
            self.compiled += 1;
        }
        Some(())
    }
}

// ── prequalification: the syntactic whitelist ──────────────────────────

/// Every instruction the JIT knows how to translate, checked without any
/// type information — cheap enough to run at registration for every
/// promoted function. CallFn targets are resolved (and arity-checked)
/// later, at group-planning time.
fn whitelist_ok(bytecode: &CompiledBytecode) -> bool {
    if bytecode.param_count > MAX_PARAMS
        || bytecode.instructions.is_empty()
        || bytecode.entry_point >= bytecode.instructions.len()
    {
        return false;
    }
    bytecode.instructions.iter().all(|inst| match inst {
        Instruction::LoadConst { const_idx, .. } => matches!(
            bytecode.constants.get(*const_idx as usize).map(|c| &c.data),
            Some(
                ValueData::Integer(_)
                    | ValueData::Boolean(_)
                    | ValueData::Unit
                    | ValueData::Float(_)
                    | ValueData::String(_)
            )
        ),
        Instruction::Move { .. }
        | Instruction::TakeMove { .. }
        | Instruction::Add { .. }
        | Instruction::AddAssign { .. }
        | Instruction::Sub { .. }
        | Instruction::Mul { .. }
        | Instruction::Div { .. }
        | Instruction::Mod { .. }
        | Instruction::Neg { .. }
        | Instruction::Eq { .. }
        | Instruction::Ne { .. }
        | Instruction::Lt { .. }
        | Instruction::Le { .. }
        | Instruction::Gt { .. }
        | Instruction::Ge { .. }
        | Instruction::And { .. }
        | Instruction::Or { .. }
        | Instruction::Not { .. }
        | Instruction::Jump { .. }
        | Instruction::JumpIfTrue { .. }
        | Instruction::JumpIfFalse { .. }
        | Instruction::MatchFail
        | Instruction::Nop
        | Instruction::GetField { .. }
        | Instruction::IndexGet { .. }
        | Instruction::IterLen { .. }
        | Instruction::IterGet { .. }
        | Instruction::MakeTuple { .. }
        | Instruction::PatternTestTuple { .. }
        | Instruction::ExtractElement { .. }
        | Instruction::PatternTestResult { .. }
        | Instruction::ExtractResult { .. }
        // arg_moves is an optimization hint, not a semantic contract: a
        // masked register is dead after the call (the liveness pass
        // proved it), so native code cloning it instead of moving is
        // unobservable. The JIT ignores the mask.
        | Instruction::CallFn { .. } => true,
        // A tail self-call is a backward jump to the entry with a
        // parameter rebind — a native loop once compiled.
        Instruction::TailCallSelf { .. } => true,
        // Allocation is allowed only in straight-line code (constructors).
        // In a native loop every allocation would live until the call
        // ends — the scratch model's memory cost — and a cap-triggered
        // mid-loop deopt costs more than never compiling. Loops that
        // build structs stay on bytecode and call native constructors.
        Instruction::MakeStruct { field_regs, .. } => field_regs.len() <= 16,
        // Same allocation discipline as MakeStruct.
        Instruction::MakeResult { .. } => true,
        Instruction::MakeList { elements, .. } => elements.len() <= 64,
        Instruction::MakeMap { entries, .. } => entries.len() <= 64,
        // Named map natives. Reads don't allocate, so they compile in
        // loops; map_set allocates and follows the MakeStruct rule. A
        // user definition shadowing one of these refuses at inference
        // (and demotes already-compiled entries via note_shadow).
        Instruction::CallNamed {
            function_name,
            args,
            ..
        } => match function_name.as_str() {
            "map_get" | "map_has_key" => args.len() == 2,
            "map_set" => args.len() == 3,
            _ => false,
        },
        Instruction::BinImm { imm, .. } => {
            matches!(imm.data, ValueData::Integer(_) | ValueData::Float(_))
        }
        // Domain-constrained math builtins (sqrt, asin, acos, ln, log2,
        // log10) stay off the JIT: native code would compute a raw NaN
        // where the interpreter raises a domain error. Excluded here, they
        // run on bytecode, which routes out-of-domain inputs through the
        // interpreter's checked path. The domain-free builtins still compile.
        Instruction::CallBuiltin {
            builtin_id, args, ..
        } => {
            matches!(
                crate::ovm::bytecode::BytecodeVm::FLOAT_MATH.get(*builtin_id as usize),
                Some((_, arity)) if args.len() == *arity && *arity <= 2
            ) && !matches!(*builtin_id as usize, 0 | 10 | 11 | 18 | 19 | 20)
        }
        Instruction::Return { value } => value.is_some(),
        _ => false,
    })
}

/// How many non-plumbing instructions an allocating body must carry
/// before its native form can out-earn the call boundary. Measured on
/// `fn make(i) = [i, i * 2]` called 3M times from a bytecode loop: the
/// boundary (argument marshalling, scratch-context setup, retain
/// resolution, unmarshal) costs ~20ns per call, while each bytecode
/// instruction the native body replaces saves only a few ns — and the
/// allocation itself goes through the same runtime path on both tiers,
/// so it saves nothing. Crossover lands around eight compute ops.
const BOUNDARY_MIN_COMPUTE: usize = 8;

/// True for bodies that exist to allocate — a constructor's MakeStruct/
/// MakeList/MakeMap/MakeResult plus a handful of compute ops. Calling
/// such a body natively from bytecode is a net loss (see
/// BOUNDARY_MIN_COMPUTE), so `has` declines the boundary and the call
/// stays on bytecode. The body still compiles when a group needs it:
/// native callers reach it by direct call, which has no boundary.
fn boundary_unprofitable(bytecode: &CompiledBytecode) -> bool {
    let mut allocates = false;
    let mut compute = 0usize;
    for inst in bytecode.instructions.iter() {
        match inst {
            Instruction::MakeStruct { .. }
            | Instruction::MakeList { .. }
            | Instruction::MakeMap { .. }
            | Instruction::MakeResult { .. } => allocates = true,
            Instruction::CallNamed { function_name, .. } if function_name == "map_set" => {
                allocates = true
            }
            // Plumbing: moved values and constants cost next to nothing
            // on either tier. MakeTuple is not an allocation (tuples
            // return through out slots) but it is not compute either.
            Instruction::Move { .. }
            | Instruction::TakeMove { .. }
            | Instruction::LoadConst { .. }
            | Instruction::Return { .. }
            | Instruction::MakeTuple { .. } => {}
            _ => compute += 1,
        }
    }
    allocates && compute <= BOUNDARY_MIN_COMPUTE
}

fn has_backward_jump(bytecode: &CompiledBytecode) -> bool {
    bytecode.instructions.iter().enumerate().any(|(i, inst)| {
        let target = match inst {
            Instruction::Jump { target } => Some(target.0 as usize),
            Instruction::JumpIfTrue { target, .. } | Instruction::JumpIfFalse { target, .. } => {
                Some(target.0 as usize)
            }
            // A tail self-call loops back to the entry.
            Instruction::TailCallSelf { .. } => Some(bytecode.entry_point),
            _ => None,
        };
        matches!(target, Some(t) if t <= i)
    })
}

// ── qualification: register-kind inference ─────────────────────────────

// Kinds as a bitmask. A register may be WRITTEN with several kinds over
// its lifetime (e.g. the dead result slot of an `if` statement that is
// Unit on one path and Int on the other). What must be single-kinded is
// every register an instruction READS — reads carry *allowed-set*
// constraints, writes accumulate possibilities, and qualification
// demands each read register's write-set be a singleton inside its
// allowed set.
const K_INT: u16 = 1;
const K_BOOL: u16 = 2;
const K_UNIT: u16 = 4;
const K_FLOAT: u16 = 8;
const K_STRUCT: u16 = 16;
const K_LIST: u16 = 32;
const K_TUPLE: u16 = 64;
const K_STR: u16 = 128;
/// Result values ride borrowed `Arc<ResultObject>` pointers, like
/// structs — the ninth kind, and the reason the masks are u16.
const K_RESULT: u16 = 256;
/// Maps ride borrowed `Arc<HashMap<String, OvmValue>>` pointers.
const K_MAP: u16 = 512;
/// Largest tuple the JIT returns natively (multi-value return slots).
const MAX_TUPLE: usize = 4;
const K_NUM: u16 = K_INT | K_FLOAT;
const K_ANY: u16 =
    K_INT | K_BOOL | K_UNIT | K_FLOAT | K_STRUCT | K_LIST | K_TUPLE | K_STR | K_RESULT | K_MAP;

fn kind_mask(k: Kind) -> u16 {
    match k {
        Kind::Int => K_INT,
        Kind::Bool => K_BOOL,
        Kind::Float => K_FLOAT,
        Kind::Struct(_) => K_STRUCT,
        Kind::ListFloat | Kind::ListInt | Kind::ListStruct(_) | Kind::ListStr => K_LIST,
        Kind::Str => K_STR,
        Kind::Result(..) => K_RESULT,
        Kind::Map(_) => K_MAP,
    }
}

/// Does a value of `kind` provably satisfy `check`, for every value the
/// kind can classify? The shared question behind both parameter and
/// return discharge: native code never runs the VM's per-call checks,
/// so an annotation compiles only when its kind proves it.
fn kind_discharges(
    check: &crate::ast::FieldTypeCheck,
    kind: Kind,
    shapes: &HashMap<u32, ShapeSpec>,
) -> bool {
    match kind {
        Kind::Int => check.accepts("Int"),
        Kind::Float => check.accepts("Float"),
        Kind::Bool => check.accepts("Bool"),
        Kind::Str => check.accepts("String"),
        Kind::Result(okp, errp) => result_return_discharged(check, okp, errp),
        Kind::ListInt | Kind::ListFloat | Kind::ListStruct(_) | Kind::ListStr => {
            check.accepts("List")
        }
        Kind::Map(_) => check.accepts("Map"),
        Kind::Struct(sid) => shapes
            .get(&sid)
            .is_some_and(|s| check.accepts(&s.shape.type_name)),
    }
}

/// Static discharge for a Result-returning annotated function: the
/// inferred kind proves the annotation only when each observed side's
/// payload satisfies that side's check. Absent sides never occur inside
/// a compiled specialization (a call carrying the other side classifies
/// to different param kinds and stays on bytecode), so they impose
/// nothing. Anything but a plain Result annotation stays on bytecode.
fn result_return_discharged(
    check: &crate::ast::FieldTypeCheck,
    okp: Payload,
    errp: Payload,
) -> bool {
    use crate::ast::FieldTypeCheck;
    fn payload_name(p: Payload) -> Option<&'static str> {
        match p {
            Payload::Int => Some("Int"),
            Payload::Bool => Some("Bool"),
            Payload::Float => Some("Float"),
            Payload::Str => Some("String"),
            Payload::Absent => None,
        }
    }
    fn side_ok(c: &Option<Box<FieldTypeCheck>>, p: Payload) -> bool {
        match (c, payload_name(p)) {
            (_, None) => true, // side never occurs
            (None, _) => true, // side unchecked
            (Some(c), Some(n)) => c.accepts(n),
        }
    }
    match check {
        FieldTypeCheck::Result { ok, err } => side_ok(ok, okp) && side_ok(err, errp),
        FieldTypeCheck::Named(n) => n == "Result",
        _ => false,
    }
}

/// Absent is bottom, equal payloads join to themselves, conflicting
/// payloads refuse.
fn join_payload(a: Payload, b: Payload) -> Option<Payload> {
    match (a, b) {
        (Payload::Absent, p) | (p, Payload::Absent) => Some(p),
        (a, b) if a == b => Some(a),
        _ => None,
    }
}

/// Join two payload-carrying kinds side by side (Results by side, maps
/// by value payload). Anything else refuses — every other exotic kind
/// must match exactly.
fn join_exotic(a: Kind, b: Kind) -> Option<Kind> {
    match (a, b) {
        (Kind::Result(ao, ae), Kind::Result(bo, be)) => {
            Some(Kind::Result(join_payload(ao, bo)?, join_payload(ae, be)?))
        }
        (Kind::Map(ap), Kind::Map(bp)) => Some(Kind::Map(join_payload(ap, bp)?)),
        _ => None,
    }
}

fn mask_singleton(mask: u16) -> Option<Kind> {
    match mask {
        K_INT => Some(Kind::Int),
        K_BOOL => Some(Kind::Bool),
        K_FLOAT => Some(Kind::Float),
        K_STR => Some(Kind::Str),
        _ => None,
    }
}

/// One group member's inference state, carried across global fixpoint
/// iterations.
struct PlanFn {
    func_id: FunctionId,
    bytecode: Arc<CompiledBytecode>,
    param_kinds: Vec<Kind>,
    writes: Vec<u16>,
    allowed: Vec<u16>,
    was_read: Vec<bool>,
    /// The full kind of a struct- or list-holding register (masks only
    /// say "some struct"/"some list"; this carries which).
    exotic: Vec<Option<Kind>>,
    /// Element kinds of a tuple-holding register (scalars only, len <=
    /// MAX_TUPLE). Tuples never enter as parameters; they arise from
    /// MakeTuple and from calls to tuple-returning group members.
    tuples: HashMap<u32, Vec<Kind>>,
    /// Set when Return hands back a tuple register.
    ret_tuple: Option<Vec<Kind>>,
    /// Shapes this function synthesizes with MakeStruct, merged into the
    /// group's spec map each fixpoint iteration.
    made_shapes: HashMap<u32, ShapeSpec>,
    /// Set when Return hands back a struct register (the shape id).
    ret_struct: Option<u32>,
    /// Set when Return hands back a Result or Map register — the join
    /// of every return site's kind, so `Ok(n)`/`Err(msg)` paths (and
    /// maps of different observed payloads) merge.
    ret_result: Option<Kind>,
    /// Set when Return hands back a list register (the element kind).
    ret_list: Option<Kind>,
    ret_mask: u16,
    eq_pairs: Vec<(u32, u32)>,
    return_regs: Vec<u32>,
    /// The function's single loop region, when it has exactly one
    /// backward-jump target (see loop_shape).
    loop_region: Option<(usize, usize)>,
    /// Registers live at the loop head — the watermark gate's oracle.
    live_at_head: Vec<bool>,
    /// Registers defined anywhere inside the loop region.
    defined_in_region: Vec<bool>,
}

/// The finalized result codegen consumes.
struct Inference {
    reg_kind: Vec<Option<Kind>>,
    param_kinds: Vec<Kind>,
    ret_kind: Kind,
    tuples: HashMap<u32, Vec<Kind>>,
    ret_tuple: Option<Vec<Kind>>,
    ret_struct: Option<u32>,
    /// Some((head, last back-edge)) when codegen should watermark the
    /// loop: mark scratch lengths on entry, truncate on every taken
    /// back-edge. Set only when the finalize gate proved every heap
    /// value born in the region dies in its iteration.
    scratch_region: Option<(usize, usize)>,
}

/// Apply `f` to every register an instruction touches. Returns false
/// for an unmodeled instruction — callers must abort their transform.
pub(crate) fn for_each_reg(
    inst: &mut Instruction,
    mut f: impl FnMut(&mut crate::ovm::bytecode::Register),
) -> bool {
    use Instruction as I;
    match inst {
        I::LoadConst { dst, .. } => f(dst),
        I::Move { dst, src } | I::TakeMove { dst, src } => {
            f(dst);
            f(src);
        }
        I::AddAssign { target, rhs } => {
            f(target);
            f(rhs);
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
            f(dst);
            f(lhs);
            f(rhs);
        }
        I::Not { dst, src } | I::Neg { dst, src } => {
            f(dst);
            f(src);
        }
        I::BinImm { dst, lhs, .. } => {
            f(dst);
            f(lhs);
        }
        I::Jump { .. } | I::MatchFail | I::Nop => {}
        I::JumpIfTrue { condition, .. } | I::JumpIfFalse { condition, .. } => f(condition),
        I::CallFn { dst, args, .. }
        | I::CallNamed { dst, args, .. }
        | I::CallBuiltin { dst, args, .. } => {
            f(dst);
            args.iter_mut().for_each(&mut f);
        }
        I::Return { value } => {
            if let Some(v) = value {
                f(v);
            }
        }
        I::MakeStruct {
            dst, field_regs, ..
        } => {
            f(dst);
            field_regs.iter_mut().for_each(&mut f);
        }
        I::MakeTemplate { dst, parts } => {
            f(dst);
            for p in parts {
                if let crate::ovm::bytecode::TplPart::Reg(r) = p {
                    f(r);
                }
            }
        }
        I::MakeList { dst, elements } | I::MakeTuple { dst, elements } => {
            f(dst);
            elements.iter_mut().for_each(&mut f);
        }
        I::MakeMap { dst, entries } => {
            f(dst);
            for (k, v) in entries.iter_mut() {
                f(k);
                f(v);
            }
        }
        I::MakeResult { dst, value, .. }
        | I::PatternTestResult { dst, value, .. }
        | I::ExtractResult { dst, value, .. }
        | I::PatternTestTuple { dst, value, .. }
        | I::ExtractElement { dst, value, .. } => {
            f(dst);
            f(value);
        }
        I::IterLen { dst, src } => {
            f(dst);
            f(src);
        }
        I::IterGet { dst, src, idx } => {
            f(dst);
            f(src);
            f(idx);
        }
        I::GetField { dst, object, .. } => {
            f(dst);
            f(object);
        }
        I::IndexGet { dst, object, index } => {
            f(dst);
            f(object);
            f(index);
        }
        _ => return false,
    }
    true
}

/// Shift every jump target through `map` (absolute old pc -> new pc).
pub(crate) fn remap_targets(inst: &mut Instruction, map: &dyn Fn(u32) -> u32) {
    match inst {
        Instruction::Jump { target } => target.0 = map(target.0),
        Instruction::JumpIfTrue { target, .. } | Instruction::JumpIfFalse { target, .. } => {
            target.0 = map(target.0)
        }
        _ => {}
    }
}

/// Splice `replacement` over the single instruction at `at`, fixing the
/// function's jump targets, entry point, and span table. Targets that
/// pointed AT the replaced instruction land on the replacement's first
/// instruction; targets beyond it shift by the growth.
fn splice(b: &mut CompiledBytecode, at: usize, replacement: Vec<Instruction>) {
    let grow = replacement.len() as i64 - 1;
    b.instructions.splice(at..=at, replacement);
    let shift = |pc: u32| -> u32 {
        if (pc as usize) > at {
            (pc as i64 + grow) as u32
        } else {
            pc
        }
    };
    for (i, inst) in b.instructions.iter_mut().enumerate() {
        // Instructions inside the replacement were emitted with final
        // positions already; everything else remaps.
        if i >= at && i < at + (grow + 1) as usize {
            continue;
        }
        remap_targets(inst, &shift);
    }
    if b.entry_point > at {
        b.entry_point = (b.entry_point as i64 + grow) as usize;
    }
    for entry in b.span_table.iter_mut() {
        entry.0 = shift(entry.0);
    }
}

/// Inline calls to tiny callees: no type annotations, a handful of
/// instructions, and — since Campaign 7, T4 — calls of their own are no
/// longer a refusal: a callee whose calls themselves inline away (a
/// distance function calling a square helper) presents its expanded,
/// call-free form and inlines like any leaf, to `INLINE_DEPTH` levels.
/// Exposes cross-function structure (a constructor's fields, a helper's
/// arithmetic) to the scalar-replacement pass below. The transform only
/// exists on the JIT's planning clone — the VM's bytecode is untouched,
/// and every error path deopts to a clean rerun of the original, so
/// semantics and stack traces cannot drift.
const INLINE_MAX_CALLEE: usize = 24;
/// A transitively expanded callee may exceed the per-callee cap by its
/// own inlined helpers, up to this bound.
const INLINE_MAX_EXPANDED: usize = 48;
const INLINE_BUDGET: usize = 256;
/// How many levels of helper-within-helper expand before a call is a
/// call. Cycles and self-recursion bottom out here naturally: the
/// recursive call survives expansion, and a form that still contains
/// one is refused.
const INLINE_DEPTH: u32 = 2;

/// The callee as the inliner would splice it: itself when call-free and
/// small, its expanded form when its own calls inline away, None when
/// it must stay a call. Named and builtin calls (`map_get`, float math —
/// everything `whitelist_ok` admits) pass through unexpanded: the
/// splice mechanics carry them verbatim.
fn inlinable_form(
    callee: &Arc<CompiledBytecode>,
    lookup: &BytecodeLookup,
    depth: u32,
) -> Option<Arc<CompiledBytecode>> {
    let has_fn_call = callee
        .instructions
        .iter()
        .any(|i| matches!(i, Instruction::CallFn { .. }));
    if !has_fn_call {
        return (callee.instructions.len() <= INLINE_MAX_CALLEE).then(|| callee.clone());
    }
    if depth == 0 || callee.instructions.len() > INLINE_MAX_CALLEE {
        return None;
    }
    let mut form = (**callee).clone();
    inline_to_depth(&mut form, callee.function_id, lookup, depth - 1);
    if form
        .instructions
        .iter()
        .any(|i| matches!(i, Instruction::CallFn { .. }))
        || form.instructions.len() > INLINE_MAX_EXPANDED
    {
        return None;
    }
    Some(Arc::new(form))
}

fn inline_leaves(b: &mut CompiledBytecode, self_id: FunctionId, lookup: &BytecodeLookup) {
    inline_to_depth(b, self_id, lookup, INLINE_DEPTH);
}

fn inline_to_depth(
    b: &mut CompiledBytecode,
    self_id: FunctionId,
    lookup: &BytecodeLookup,
    depth: u32,
) {
    let mut budget = INLINE_BUDGET;
    'rescan: loop {
        for pc in 0..b.instructions.len() {
            let Instruction::CallFn {
                dst, func_id, args, ..
            } = &b.instructions[pc]
            else {
                continue;
            };
            let (dst, func_id, args) = (*dst, *func_id, args.clone());
            if func_id == self_id {
                continue;
            }
            let Some(callee) = lookup(func_id) else {
                continue;
            };
            if callee.instructions.is_empty()
                || callee.entry_point != 0
                || callee.param_count != args.len()
                || callee.param_checks.iter().any(|c| c.is_some())
                || callee.return_check.is_some()
                || !whitelist_ok(&callee)
                || has_backward_jump(&callee)
            {
                continue;
            }
            let Some(callee) = inlinable_form(&callee, lookup, depth) else {
                continue;
            };
            if callee.instructions.len() > budget {
                continue;
            }

            let reg_off = b.register_count;
            let const_off = b.constants.len() as u32;
            let body_start = pc + args.len();

            // Where each callee pc lands (Returns grow by one), so
            // intra-callee jumps can be remapped absolutely.
            let mut pos = Vec::with_capacity(callee.instructions.len());
            let mut cursor = body_start;
            for inst in &callee.instructions {
                pos.push(cursor);
                cursor += if matches!(inst, Instruction::Return { .. }) {
                    2
                } else {
                    1
                };
            }
            let cont = cursor as u32; // first instruction after the splice

            let mut rep: Vec<Instruction> = Vec::with_capacity(cursor - pc);
            for (i, arg) in args.iter().enumerate() {
                rep.push(Instruction::Move {
                    dst: crate::ovm::bytecode::Register(reg_off + i as u32),
                    src: *arg,
                });
            }
            let mut ok = true;
            for (ci, inst) in callee.instructions.iter().enumerate() {
                let mut inst = inst.clone();
                if !for_each_reg(&mut inst, |r| r.0 += reg_off) {
                    ok = false;
                    break;
                }
                match &mut inst {
                    Instruction::LoadConst { const_idx, .. } => *const_idx += const_off,
                    Instruction::GetField { name_const, .. } => *name_const += const_off,
                    _ => {}
                }
                if let Instruction::Return { value } = &inst {
                    let Some(v) = value else {
                        ok = false;
                        break;
                    };
                    rep.push(Instruction::Move { dst, src: *v });
                    rep.push(Instruction::Jump {
                        target: crate::ovm::bytecode::Label(cont),
                    });
                    let _ = ci;
                    continue;
                }
                remap_targets(&mut inst, &|t| pos[t as usize] as u32);
                rep.push(inst);
            }
            if !ok {
                continue;
            }

            budget -= callee.instructions.len();
            b.constants.extend(callee.constants.iter().cloned());
            b.register_count += callee.register_count;
            splice(b, pc, rep);
            continue 'rescan;
        }
        break;
    }
}

/// Single-def copy propagation and dead-move elimination: when both
/// sides of a Move are defined exactly once and every use of the copy
/// sits after the Move, the copy IS the source — uses rewrite to the
/// source and the Move (now unread) becomes a Nop. This is what lets
/// scalar replacement see through the parameter-binding Moves the
/// inliner emits.
fn propagate_copies(b: &mut CompiledBytecode) {
    let nregs = b.register_count as usize;
    let mut uses_scratch: Vec<u32> = Vec::new();
    let mut defs_scratch: Vec<u32> = Vec::new();
    loop {
        let mut def_counts = vec![0u32; nregs];
        // Parameters are defined at entry — an implicit def that must
        // count, or a reassigned parameter masquerades as single-def.
        for d in def_counts.iter_mut().take(b.param_count.min(nregs)) {
            *d = 1;
        }
        for inst in &b.instructions {
            uses_scratch.clear();
            defs_scratch.clear();
            if !inst_uses_defs(inst, &mut uses_scratch, &mut defs_scratch) {
                return;
            }
            for d in &defs_scratch {
                if (*d as usize) < nregs {
                    def_counts[*d as usize] += 1;
                }
            }
        }
        let mut changed = false;
        for pc in 0..b.instructions.len() {
            let Instruction::Move { dst: m, src } = b.instructions[pc] else {
                continue;
            };
            if m == src
                || def_counts.get(m.0 as usize).copied().unwrap_or(2) != 1
                || def_counts.get(src.0 as usize).copied().unwrap_or(2) != 1
            {
                continue;
            }
            // Every use of m must sit after the Move; a use before it
            // (a loop carry) would observe the previous iteration.
            let mut rewritable = true;
            for (i, inst) in b.instructions.iter().enumerate() {
                uses_scratch.clear();
                defs_scratch.clear();
                inst_uses_defs(inst, &mut uses_scratch, &mut defs_scratch);
                if uses_scratch.contains(&m.0) && i <= pc {
                    rewritable = false;
                    break;
                }
            }
            if !rewritable {
                continue;
            }
            // Rewrite reads of m to src everywhere except the Move's own
            // def; then the Move is dead.
            for (i, inst) in b.instructions.iter_mut().enumerate() {
                if i == pc {
                    continue;
                }
                for_each_reg(inst, |r| {
                    if *r == m {
                        *r = src;
                    }
                });
            }
            b.instructions[pc] = Instruction::Nop;
            changed = true;
        }
        if !changed {
            break;
        }
    }
}

/// Scalar replacement of aggregates: a MakeStruct whose register has a
/// single def and is read only by GetField (all after the construction)
/// never needs to exist. Construction becomes one Move per field into a
/// fresh snapshot register; each field read becomes a Move from its
/// snapshot. No allocation, no helper call, no guard — the fields are
/// plain registers the rest of the pipeline compiles as scalars.
fn scalar_replace(b: &mut CompiledBytecode) {
    'rescan: loop {
        let nregs = b.register_count as usize;
        let mut def_counts = vec![0u32; nregs];
        for d in def_counts.iter_mut().take(b.param_count.min(nregs)) {
            *d = 1;
        }
        let mut uses_scratch: Vec<u32> = Vec::new();
        let mut defs_scratch: Vec<u32> = Vec::new();
        for inst in &b.instructions {
            uses_scratch.clear();
            defs_scratch.clear();
            if !inst_uses_defs(inst, &mut uses_scratch, &mut defs_scratch) {
                return; // unmodeled instruction: leave the function alone
            }
            for d in &defs_scratch {
                if (*d as usize) < nregs {
                    def_counts[*d as usize] += 1;
                }
            }
        }
        for pc in 0..b.instructions.len() {
            let Instruction::MakeStruct {
                dst,
                shape,
                field_regs,
                ..
            } = &b.instructions[pc]
            else {
                continue;
            };
            let (d, shape, field_regs) = (*dst, shape.clone(), field_regs.clone());
            if field_regs.is_empty()
                || field_regs.len() > 8
                || def_counts.get(d.0 as usize).copied().unwrap_or(2) != 1
            {
                continue;
            }
            // Every use of d must be a GetField at a later position whose
            // name resolves in the shape.
            let mut eligible = true;
            let mut reads: Vec<(usize, usize)> = Vec::new(); // (pc, field idx)
            for (i, inst) in b.instructions.iter().enumerate() {
                uses_scratch.clear();
                defs_scratch.clear();
                inst_uses_defs(inst, &mut uses_scratch, &mut defs_scratch);
                if !uses_scratch.contains(&d.0) {
                    continue;
                }
                let Instruction::GetField {
                    object, name_const, ..
                } = inst
                else {
                    eligible = false;
                    break;
                };
                if object.0 != d.0 || i <= pc {
                    eligible = false;
                    break;
                }
                let Some(ValueData::String(name)) =
                    b.constants.get(*name_const as usize).map(|c| &c.data)
                else {
                    eligible = false;
                    break;
                };
                let Some(idx) = shape.field_names.iter().position(|f| f == name.as_str()) else {
                    eligible = false;
                    break;
                };
                reads.push((i, idx));
            }
            if !eligible {
                continue;
            }

            let snap_base = b.register_count;
            b.register_count += field_regs.len() as u32;
            for (read_pc, field_idx) in &reads {
                let Instruction::GetField { dst: g, .. } = b.instructions[*read_pc] else {
                    unreachable!("collected above");
                };
                b.instructions[*read_pc] = Instruction::Move {
                    dst: g,
                    src: crate::ovm::bytecode::Register(snap_base + *field_idx as u32),
                };
            }
            let rep: Vec<Instruction> = field_regs
                .iter()
                .enumerate()
                .map(|(j, src)| Instruction::Move {
                    dst: crate::ovm::bytecode::Register(snap_base + j as u32),
                    src: *src,
                })
                .collect();
            splice(b, pc, rep);
            continue 'rescan;
        }
        // The list twin: a MakeList read only by IndexGet with constant,
        // in-bounds indices (negatives wrap, exactly like the runtime)
        // never needs the Vec.
        for pc in 0..b.instructions.len() {
            let Instruction::MakeList { dst, elements } = &b.instructions[pc] else {
                continue;
            };
            let (d, elements) = (*dst, elements.clone());
            if elements.is_empty()
                || elements.len() > 8
                || def_counts.get(d.0 as usize).copied().unwrap_or(2) != 1
            {
                continue;
            }
            // A register holding a constant integer: single def, LoadConst.
            let const_int = |reg: crate::ovm::bytecode::Register| -> Option<i64> {
                if def_counts.get(reg.0 as usize).copied().unwrap_or(2) != 1 {
                    return None;
                }
                b.instructions.iter().find_map(|inst| match inst {
                    Instruction::LoadConst { dst, const_idx } if *dst == reg => {
                        match b.constants.get(*const_idx as usize).map(|c| &c.data) {
                            Some(ValueData::Integer(i)) => Some(*i),
                            _ => None,
                        }
                    }
                    _ => None,
                })
            };
            let mut eligible = true;
            let mut reads: Vec<(usize, usize)> = Vec::new();
            for (i, inst) in b.instructions.iter().enumerate() {
                uses_scratch.clear();
                defs_scratch.clear();
                inst_uses_defs(inst, &mut uses_scratch, &mut defs_scratch);
                if !uses_scratch.contains(&d.0) {
                    continue;
                }
                let Instruction::IndexGet { object, index, .. } = inst else {
                    eligible = false;
                    break;
                };
                if object.0 != d.0 || i <= pc {
                    eligible = false;
                    break;
                }
                let Some(raw) = const_int(*index) else {
                    eligible = false;
                    break;
                };
                let len = elements.len() as i64;
                let resolved = if raw < 0 { len + raw } else { raw };
                if resolved < 0 || resolved >= len {
                    eligible = false; // out of bounds: leave the error path
                    break;
                }
                reads.push((i, resolved as usize));
            }
            if !eligible {
                continue;
            }
            let snap_base = b.register_count;
            b.register_count += elements.len() as u32;
            for (read_pc, elem_idx) in &reads {
                let Instruction::IndexGet { dst: g, .. } = b.instructions[*read_pc] else {
                    unreachable!("collected above");
                };
                b.instructions[*read_pc] = Instruction::Move {
                    dst: g,
                    src: crate::ovm::bytecode::Register(snap_base + *elem_idx as u32),
                };
            }
            let rep: Vec<Instruction> = elements
                .iter()
                .enumerate()
                .map(|(j, src)| Instruction::Move {
                    dst: crate::ovm::bytecode::Register(snap_base + j as u32),
                    src: *src,
                })
                .collect();
            splice(b, pc, rep);
            continue 'rescan;
        }
        break;
    }
}

/// The registers an instruction reads and the register it defines —
/// the vocabulary of the loop-liveness analysis. Returns false for an
/// instruction it doesn't model, which the caller must treat as
/// "reads everything" (conservatively live). Only whitelisted
/// instructions reach planning, so the catch-all is a safety net.
fn inst_uses_defs(inst: &Instruction, uses: &mut Vec<u32>, defs: &mut Vec<u32>) -> bool {
    use Instruction as I;
    match inst {
        I::LoadConst { dst, .. } => defs.push(dst.0),
        I::Move { dst, src } => {
            uses.push(src.0);
            defs.push(dst.0);
        }
        I::TakeMove { dst, src } => {
            uses.push(src.0);
            defs.push(dst.0);
            // The source is Unit afterward — a definition, exactly as
            // the optimizer models it.
            defs.push(src.0);
        }
        I::TailCallSelf { args } => {
            for (i, a) in args.iter().enumerate() {
                uses.push(a.0);
                defs.push(i as u32);
            }
        }
        I::AddAssign { target, rhs } => {
            uses.push(target.0);
            uses.push(rhs.0);
            defs.push(target.0);
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
        I::Not { dst, src } | I::Neg { dst, src } => {
            uses.push(src.0);
            defs.push(dst.0);
        }
        I::BinImm { dst, lhs, .. } => {
            uses.push(lhs.0);
            defs.push(dst.0);
        }
        I::Jump { .. } | I::MatchFail | I::Nop => {}
        I::JumpIfTrue { condition, .. } | I::JumpIfFalse { condition, .. } => {
            uses.push(condition.0);
        }
        I::CallFn { dst, args, .. }
        | I::CallNamed { dst, args, .. }
        | I::CallBuiltin { dst, args, .. } => {
            uses.extend(args.iter().map(|a| a.0));
            defs.push(dst.0);
        }
        I::Return { value } => {
            if let Some(v) = value {
                uses.push(v.0);
            }
        }
        I::MakeStruct {
            dst, field_regs, ..
        } => {
            uses.extend(field_regs.iter().map(|r| r.0));
            defs.push(dst.0);
        }
        I::MakeList { dst, elements } | I::MakeTuple { dst, elements } => {
            uses.extend(elements.iter().map(|r| r.0));
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
        I::MakeMap { dst, entries } => {
            for (k, v) in entries {
                uses.push(k.0);
                uses.push(v.0);
            }
            defs.push(dst.0);
        }
        I::MakeResult { dst, value, .. }
        | I::PatternTestResult { dst, value, .. }
        | I::ExtractResult { dst, value, .. }
        | I::PatternTestTuple { dst, value, .. }
        | I::ExtractElement { dst, value, .. } => {
            uses.push(value.0);
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
        I::GetField { dst, object, .. } => {
            uses.push(object.0);
            defs.push(dst.0);
        }
        I::IndexGet { dst, object, index } => {
            uses.push(object.0);
            uses.push(index.0);
            defs.push(dst.0);
        }
        _ => return false,
    }
    true
}

/// The single loop's shape, when the function has exactly one backward-
/// jump target: `head..=last_back_edge` is the region the watermark
/// governs. `heads > 1` (nesting or sequential loops) disables the
/// watermark — allocation the old rules forbade then refuses at
/// finalize, and everything else compiles exactly as before.
pub(crate) struct LoopShape {
    /// Some((head, last back-edge pc)) when exactly one head exists;
    /// None for straight-line code and for several heads alike (the
    /// watermark only handles the single-loop shape either way).
    pub(crate) region: Option<(usize, usize)>,
}

pub(crate) fn loop_shape(bytecode: &CompiledBytecode) -> LoopShape {
    let mut heads: Vec<usize> = Vec::new();
    let mut last_edge: usize = 0;
    for (pc, inst) in bytecode.instructions.iter().enumerate() {
        let target = match inst {
            Instruction::Jump { target } => Some(target.0 as usize),
            Instruction::JumpIfTrue { target, .. } | Instruction::JumpIfFalse { target, .. } => {
                Some(target.0 as usize)
            }
            Instruction::TailCallSelf { .. } => Some(bytecode.entry_point),
            _ => None,
        };
        if let Some(t) = target
            && t <= pc
        {
            if !heads.contains(&t) {
                heads.push(t);
            }
            last_edge = last_edge.max(pc);
        }
    }
    match heads.len() {
        1 => LoopShape {
            region: Some((heads[0], last_edge)),
        },
        _ => LoopShape { region: None },
    }
}

/// Backward liveness to a fixpoint, returning the live-in set at `at`.
/// One bit per register; an unmodeled instruction makes every register
/// live (the conservative direction for this analysis, whose consumers
/// only act on proven-dead).
pub(crate) fn live_in_at(bytecode: &CompiledBytecode, nregs: usize, at: usize) -> Vec<bool> {
    let n = bytecode.instructions.len();
    let mut live_in: Vec<Vec<bool>> = vec![vec![false; nregs]; n];
    let mut uses: Vec<u32> = Vec::new();
    let mut defs: Vec<u32> = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        for pc in (0..n).rev() {
            let inst = &bytecode.instructions[pc];
            // live-out = union of successors' live-in
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
            match inst {
                Instruction::Jump { target } => succ(target.0 as usize),
                Instruction::JumpIfTrue { target, .. }
                | Instruction::JumpIfFalse { target, .. } => {
                    succ(target.0 as usize);
                    succ(pc + 1);
                }
                Instruction::TailCallSelf { .. } => succ(bytecode.entry_point),
                Instruction::Return { .. } | Instruction::MatchFail => {}
                _ => succ(pc + 1),
            }
            uses.clear();
            defs.clear();
            let modeled = inst_uses_defs(inst, &mut uses, &mut defs);
            let mut new_in = out;
            if modeled {
                for d in &defs {
                    if (*d as usize) < nregs {
                        new_in[*d as usize] = false;
                    }
                }
                for u in &uses {
                    if (*u as usize) < nregs {
                        new_in[*u as usize] = true;
                    }
                }
            } else {
                for b in new_in.iter_mut() {
                    *b = true;
                }
            }
            if new_in != live_in[pc] {
                live_in[pc] = new_in;
                changed = true;
            }
        }
    }
    live_in
        .get(at)
        .cloned()
        .unwrap_or_else(|| vec![true; nregs])
}

impl PlanFn {
    fn new(
        func_id: FunctionId,
        bytecode: Arc<CompiledBytecode>,
        param_kinds: Vec<Kind>,
        lookup: &BytecodeLookup,
    ) -> Self {
        // Canonicalize: AddAssign{t, r} is semantically Add{dst: t, lhs: t,
        // rhs: r}, and TakeMove is Move whose source the optimizer proved
        // dead afterward — native code keeping the source value alive is
        // unobservable. Rewriting both up front means inference and
        // codegen handle one shape each. 1:1, so jump targets are
        // untouched. The rewritten copy is what the JittedFn keeps alive
        // (its instructions carry the baked shape Arcs).
        let bytecode = if bytecode.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::AddAssign { .. } | Instruction::TakeMove { .. }
            )
        }) {
            let mut b = (*bytecode).clone();
            for inst in &mut b.instructions {
                match inst {
                    Instruction::AddAssign { target, rhs } => {
                        *inst = Instruction::Add {
                            dst: *target,
                            lhs: *target,
                            rhs: *rhs,
                        };
                    }
                    Instruction::TakeMove { dst, src } => {
                        *inst = Instruction::Move {
                            dst: *dst,
                            src: *src,
                        };
                    }
                    _ => {}
                }
            }
            Arc::new(b)
        } else {
            bytecode
        };
        // Inline tiny leaf callees, then scalar-replace structs that are
        // only ever field-read — both on this planning clone only. The
        // passes are no-ops on functions without the shapes they target.
        let bytecode = {
            let mut b = (*bytecode).clone();
            inline_leaves(&mut b, func_id, lookup);
            propagate_copies(&mut b);
            scalar_replace(&mut b);
            Arc::new(b)
        };
        let nregs = bytecode.register_count as usize;
        let shape = loop_shape(&bytecode);
        let (loop_region, live_at_head, defined_in_region) = match shape.region {
            Some((h, e)) => {
                let live = live_in_at(&bytecode, nregs, h);
                let mut defined = vec![false; nregs];
                let mut uses: Vec<u32> = Vec::new();
                let mut defs: Vec<u32> = Vec::new();
                for inst in &bytecode.instructions[h..=e] {
                    uses.clear();
                    defs.clear();
                    if inst_uses_defs(inst, &mut uses, &mut defs) {
                        for d in &defs {
                            if (*d as usize) < nregs {
                                defined[*d as usize] = true;
                            }
                        }
                    } else {
                        defined.iter_mut().for_each(|b| *b = true);
                    }
                }
                (Some((h, e)), live, defined)
            }
            None => (None, Vec::new(), Vec::new()),
        };
        let mut writes = vec![0u16; nregs];
        let mut exotic = vec![None; nregs];
        for (i, k) in param_kinds.iter().enumerate() {
            writes[i] = kind_mask(*k);
            if matches!(kind_mask(*k), K_STRUCT | K_LIST | K_RESULT | K_MAP) {
                exotic[i] = Some(*k);
            }
        }
        Self {
            func_id,
            bytecode,
            param_kinds,
            writes,
            allowed: vec![K_ANY; nregs],
            was_read: vec![false; nregs],
            exotic,
            tuples: HashMap::new(),
            ret_tuple: None,
            made_shapes: HashMap::new(),
            ret_struct: None,
            ret_result: None,
            ret_list: None,
            ret_mask: 0,
            eq_pairs: Vec::new(),
            return_regs: Vec::new(),
            loop_region,
            live_at_head,
            defined_in_region,
        }
    }

    /// One inference pass. `sigs` maps callable functions to their
    /// (param kinds, current ret mask); unknown callees whose argument
    /// kinds have resolved are pushed onto `requests` for planning.
    /// Returns None on a hard refusal.
    fn infer_pass(
        &mut self,
        sigs: &SigSnapshot,
        shapes: &HashMap<u32, ShapeSpec>,
        shadowed: &std::collections::HashSet<String>,
        requests: &mut Vec<(FunctionId, Vec<Kind>)>,
        global_changed: &mut bool,
    ) -> Option<()> {
        let bytecode = self.bytecode.clone();
        let mut changed = false;

        let const_mask = |idx: u32| -> u16 {
            match bytecode.constants.get(idx as usize).map(|c| &c.data) {
                Some(ValueData::Integer(_)) => K_INT,
                Some(ValueData::Boolean(_)) => K_BOOL,
                Some(ValueData::Unit) => K_UNIT,
                Some(ValueData::Float(_)) => K_FLOAT,
                Some(ValueData::String(_)) => K_STR,
                _ => 0,
            }
        };

        self.eq_pairs.clear();
        self.return_regs.clear();

        macro_rules! grow {
            ($slot:expr_2021, $bits:expr_2021) => {{
                let bits = $bits;
                let slot = &mut $slot;
                if *slot | bits != *slot {
                    *slot |= bits;
                    changed = true;
                }
            }};
        }
        macro_rules! narrow {
            ($r:expr_2021, $mask:expr_2021) => {{
                let r = $r as usize;
                let mask = $mask;
                if self.allowed[r] & mask != self.allowed[r] {
                    self.allowed[r] &= mask;
                    changed = true;
                }
                if !self.was_read[r] {
                    self.was_read[r] = true;
                    changed = true;
                }
            }};
        }

        for inst in &bytecode.instructions {
            match inst {
                Instruction::LoadConst { dst, const_idx } => {
                    grow!(self.writes[dst.0 as usize], const_mask(*const_idx));
                }
                // A tail self-call is a parallel Move of every argument
                // into its parameter register; a tuple-kinded argument
                // refuses (the entry rebind is scalar variables only).
                Instruction::TailCallSelf { args } => {
                    for (i, a) in args.iter().enumerate() {
                        if self.tuples.contains_key(&a.0) {
                            return None;
                        }
                        // Each argument is read (any kind it settles to
                        // is fine — the constraint that matters is the
                        // parameter register's own singleton, which the
                        // write below feeds), and its parameter register
                        // is written with the argument's kinds.
                        narrow!(a.0, u16::MAX);
                        let src_mask = self.writes[a.0 as usize];
                        grow!(self.writes[i], src_mask);
                    }
                }
                Instruction::Move { dst, src } => {
                    let src_mask = self.writes[src.0 as usize];
                    grow!(self.writes[dst.0 as usize], src_mask);
                    if let Some(tk) = self.tuples.get(&src.0).cloned() {
                        match self.tuples.get(&dst.0) {
                            None => {
                                self.tuples.insert(dst.0, tk);
                                changed = true;
                            }
                            Some(prev) if *prev != tk => return None,
                            _ => {}
                        }
                    }
                    if let Some(k) = self.exotic[src.0 as usize] {
                        match self.exotic[dst.0 as usize] {
                            None => {
                                self.exotic[dst.0 as usize] = Some(k);
                                changed = true;
                            }
                            Some(prev) if prev == k => {}
                            // Result kinds merge side by side; everything
                            // else is one exotic kind per register.
                            Some(prev) => match join_exotic(prev, k) {
                                Some(j) => {
                                    if j != prev {
                                        self.exotic[dst.0 as usize] = Some(j);
                                        changed = true;
                                    }
                                }
                                None => {
                                    if jit_debug() {
                                        eprintln!("[jit] exotic conflict on Move dst r{}", dst.0);
                                    }
                                    return None;
                                }
                            },
                        }
                    }
                    // Liveness flows backwards through copies: the move
                    // reads src only if someone reads dst, and the copied
                    // value must satisfy dst's constraint.
                    if self.was_read[dst.0 as usize] {
                        let dst_allowed = self.allowed[dst.0 as usize];
                        narrow!(src.0, dst_allowed);
                    }
                }
                Instruction::Add { dst, lhs, rhs }
                | Instruction::Sub { dst, lhs, rhs }
                | Instruction::Mul { dst, lhs, rhs }
                | Instruction::Div { dst, lhs, rhs }
                | Instruction::Mod { dst, lhs, rhs } => {
                    // String + string is concat: an allocation, under the
                    // same straight-line rule as MakeStruct. Mixed
                    // string/number Add (formatting) stays on bytecode.
                    if matches!(inst, Instruction::Add { .. })
                        && (self.writes[lhs.0 as usize] == 0 || self.writes[rhs.0 as usize] == 0)
                    {
                        // Operand kinds not yet resolved (e.g. a call dst
                        // mid-fixpoint): defer — narrowing now on a guess
                        // would be irreversible.
                        continue;
                    }
                    if matches!(inst, Instruction::Add { .. })
                        && self.writes[lhs.0 as usize] == K_LIST
                        && self.writes[rhs.0 as usize] == K_LIST
                    {
                        // List + list is concat, an allocation — inside a
                        // loop, finalize's watermark gate decides its fate.
                        // Element kinds may lag the fixpoint (a callee's
                        // list return resolves late): defer, don't refuse.
                        let (Some(lk), Some(rk)) =
                            (self.exotic[lhs.0 as usize], self.exotic[rhs.0 as usize])
                        else {
                            continue;
                        };
                        if lk != rk {
                            return None;
                        }
                        narrow!(lhs.0, K_LIST);
                        narrow!(rhs.0, K_LIST);
                        grow!(self.writes[dst.0 as usize], K_LIST);
                        if self.exotic[dst.0 as usize].is_none() {
                            self.exotic[dst.0 as usize] = Some(lk);
                            changed = true;
                        } else if self.exotic[dst.0 as usize] != Some(lk) {
                            return None;
                        }
                        continue;
                    }
                    if matches!(inst, Instruction::Add { .. })
                        && self.writes[lhs.0 as usize] == K_STR
                        && self.writes[rhs.0 as usize] == K_STR
                    {
                        narrow!(lhs.0, K_STR);
                        narrow!(rhs.0, K_STR);
                        grow!(self.writes[dst.0 as usize], K_STR);
                        continue;
                    }
                    narrow!(lhs.0, K_NUM);
                    narrow!(rhs.0, K_NUM);
                    let l = self.writes[lhs.0 as usize];
                    let r = self.writes[rhs.0 as usize];
                    if (l | r) & K_FLOAT != 0 {
                        grow!(self.writes[dst.0 as usize], K_FLOAT);
                    }
                    if l & K_INT != 0 && r & K_INT != 0 {
                        grow!(self.writes[dst.0 as usize], K_INT);
                    }
                }
                Instruction::Neg { dst, src } => {
                    narrow!(src.0, K_NUM);
                    let s = self.writes[src.0 as usize];
                    grow!(self.writes[dst.0 as usize], s & K_NUM);
                }
                Instruction::Eq { dst, lhs, rhs } | Instruction::Ne { dst, lhs, rhs } => {
                    narrow!(lhs.0, K_NUM | K_BOOL | K_STR);
                    narrow!(rhs.0, K_NUM | K_BOOL | K_STR);
                    self.eq_pairs.push((lhs.0, rhs.0));
                    grow!(self.writes[dst.0 as usize], K_BOOL);
                }
                Instruction::Lt { dst, lhs, rhs }
                | Instruction::Le { dst, lhs, rhs }
                | Instruction::Gt { dst, lhs, rhs }
                | Instruction::Ge { dst, lhs, rhs } => {
                    narrow!(lhs.0, K_NUM | K_STR);
                    narrow!(rhs.0, K_NUM | K_STR);
                    grow!(self.writes[dst.0 as usize], K_BOOL);
                }
                Instruction::And { dst, lhs, rhs } | Instruction::Or { dst, lhs, rhs } => {
                    narrow!(lhs.0, K_BOOL);
                    narrow!(rhs.0, K_BOOL);
                    grow!(self.writes[dst.0 as usize], K_BOOL);
                }
                Instruction::Not { dst, src } => {
                    narrow!(src.0, K_BOOL);
                    grow!(self.writes[dst.0 as usize], K_BOOL);
                }
                Instruction::BinImm {
                    op, dst, lhs, imm, ..
                } => {
                    let imm_mask = match imm.data {
                        ValueData::Integer(_) => K_INT,
                        ValueData::Float(_) => K_FLOAT,
                        _ => return None,
                    };
                    match op {
                        BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::Modulo => {
                            narrow!(lhs.0, K_NUM);
                            let l = self.writes[lhs.0 as usize];
                            if (l | imm_mask) & K_FLOAT != 0 {
                                grow!(self.writes[dst.0 as usize], K_FLOAT);
                            }
                            if l & K_INT != 0 && imm_mask == K_INT {
                                grow!(self.writes[dst.0 as usize], K_INT);
                            }
                        }
                        BinaryOp::Equal
                        | BinaryOp::NotEqual
                        | BinaryOp::LessThan
                        | BinaryOp::LessThanEqual
                        | BinaryOp::GreaterThan
                        | BinaryOp::GreaterThanEqual => {
                            narrow!(lhs.0, K_NUM);
                            grow!(self.writes[dst.0 as usize], K_BOOL);
                        }
                        _ => return None,
                    }
                }
                Instruction::GetField {
                    dst,
                    object,
                    name_const,
                    ..
                } => {
                    narrow!(object.0, K_STRUCT);
                    // The object's kind (a callee's return, resolving a
                    // fixpoint iteration late) or its shape spec (merged
                    // between iterations) may lag: defer, don't refuse.
                    let sid = match self.exotic[object.0 as usize] {
                        Some(Kind::Struct(sid)) => sid,
                        None => continue,
                        Some(_) => {
                            if jit_debug() {
                                eprintln!("[jit] GetField on non-struct r{}", object.0);
                            }
                            return None;
                        }
                    };
                    let Some(spec) = shapes.get(&sid) else {
                        continue;
                    };
                    let name = match self
                        .bytecode
                        .constants
                        .get(*name_const as usize)
                        .map(|c| &c.data)
                    {
                        Some(ValueData::String(n)) => n.clone(),
                        _ => return None,
                    };
                    let idx = spec
                        .shape
                        .field_names
                        .iter()
                        .position(|f| f == name.as_str())?;
                    grow!(
                        self.writes[dst.0 as usize],
                        kind_mask(spec.field_kinds[idx])
                    );
                }
                Instruction::IndexGet { dst, object, index } => {
                    narrow!(object.0, K_LIST);
                    narrow!(index.0, K_INT);
                    let elem = match self.exotic[object.0 as usize] {
                        Some(Kind::ListFloat) => Kind::Float,
                        Some(Kind::ListInt) => Kind::Int,
                        Some(Kind::ListStruct(sid)) => Kind::Struct(sid),
                        Some(Kind::ListStr) => Kind::Str,
                        // A callee's list return resolves a fixpoint
                        // iteration late: defer, don't refuse.
                        None => continue,
                        other => {
                            if jit_debug() {
                                eprintln!(
                                    "[jit] IndexGet object r{} not a known list: {:?}",
                                    object.0, other
                                );
                            }
                            return None;
                        }
                    };
                    grow!(self.writes[dst.0 as usize], kind_mask(elem));
                    if matches!(elem, Kind::Struct(_)) {
                        if self.exotic[dst.0 as usize].is_none() {
                            self.exotic[dst.0 as usize] = Some(elem);
                            changed = true;
                        } else if self.exotic[dst.0 as usize] != Some(elem) {
                            if jit_debug() {
                                eprintln!("[jit] exotic conflict on IndexGet dst r{}", dst.0);
                            }
                            return None;
                        }
                    }
                }
                Instruction::MakeTuple { dst, elements } => {
                    if elements.len() > MAX_TUPLE {
                        return None;
                    }
                    let mut kinds = Vec::with_capacity(elements.len());
                    let mut resolved = true;
                    for e in elements {
                        narrow!(e.0, K_NUM | K_BOOL);
                        match mask_singleton(self.writes[e.0 as usize]) {
                            Some(k) => kinds.push(k),
                            None => {
                                resolved = false;
                                break;
                            }
                        }
                    }
                    grow!(self.writes[dst.0 as usize], K_TUPLE);
                    if resolved {
                        match self.tuples.get(&dst.0) {
                            None => {
                                self.tuples.insert(dst.0, kinds);
                                changed = true;
                            }
                            Some(prev) if *prev != kinds => return None,
                            _ => {}
                        }
                    }
                }
                Instruction::PatternTestTuple { dst, value, len } => {
                    narrow!(value.0, K_TUPLE);
                    if let Some(tk) = self.tuples.get(&value.0)
                        && tk.len() != *len
                    {
                        return None; // statically false: stay on bytecode
                    }
                    grow!(self.writes[dst.0 as usize], K_BOOL);
                }
                Instruction::ExtractElement { dst, value, index } => {
                    narrow!(value.0, K_TUPLE);
                    if let Some(tk) = self.tuples.get(&value.0) {
                        let k = *tk.get(*index)?;
                        grow!(self.writes[dst.0 as usize], kind_mask(k));
                    }
                }
                Instruction::MakeResult { dst, value, ok } => {
                    // v1 payloads are scalars; a string payload narrows the
                    // value register to nothing and refuses at finalize.
                    narrow!(value.0, K_NUM | K_BOOL);
                    grow!(self.writes[dst.0 as usize], K_RESULT);
                    if let Some(pk) = mask_singleton(self.writes[value.0 as usize]) {
                        let payload = match pk {
                            Kind::Int => Payload::Int,
                            Kind::Bool => Payload::Bool,
                            Kind::Float => Payload::Float,
                            _ => return None,
                        };
                        let rk = if *ok {
                            Kind::Result(payload, Payload::Absent)
                        } else {
                            Kind::Result(Payload::Absent, payload)
                        };
                        match self.exotic[dst.0 as usize] {
                            None => {
                                self.exotic[dst.0 as usize] = Some(rk);
                                changed = true;
                            }
                            Some(prev) if prev == rk => {}
                            Some(prev) => {
                                let j = join_exotic(prev, rk)?;
                                if j != prev {
                                    self.exotic[dst.0 as usize] = Some(j);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
                Instruction::PatternTestResult { dst, value, .. } => {
                    narrow!(value.0, K_RESULT);
                    grow!(self.writes[dst.0 as usize], K_BOOL);
                }
                Instruction::MakeList { dst, elements } => {
                    // Uniform element kinds only (the kinds classify_list
                    // recognizes); empty literals have no element kind.
                    if elements.is_empty() {
                        return None;
                    }
                    grow!(self.writes[dst.0 as usize], K_LIST);
                    // Choose the scalar or struct path by the element
                    // masks, and only once they've all resolved —
                    // narrowing is irreversible, so a premature guess
                    // would poison the fixpoint.
                    if elements.iter().any(|e| self.writes[e.0 as usize] == 0) {
                        continue;
                    }
                    if elements.iter().all(|e| self.writes[e.0 as usize] == K_STR) {
                        for e in elements {
                            narrow!(e.0, K_STR);
                        }
                        if self.exotic[dst.0 as usize].is_none() {
                            self.exotic[dst.0 as usize] = Some(Kind::ListStr);
                            changed = true;
                        } else if self.exotic[dst.0 as usize] != Some(Kind::ListStr) {
                            return None;
                        }
                        continue;
                    }
                    let all_structs = elements
                        .iter()
                        .all(|e| self.writes[e.0 as usize] == K_STRUCT);
                    let lk = if all_structs {
                        let mut sid: Option<u32> = None;
                        let mut resolved = true;
                        for e in elements {
                            narrow!(e.0, K_STRUCT);
                            match self.exotic[e.0 as usize] {
                                Some(Kind::Struct(this)) => match sid {
                                    None => sid = Some(this),
                                    Some(prev) if prev != this => return None,
                                    _ => {}
                                },
                                Some(_) => return None,
                                None => {
                                    resolved = false;
                                    break;
                                }
                            }
                        }
                        if !resolved {
                            continue;
                        }
                        Kind::ListStruct(sid?)
                    } else {
                        if elements
                            .iter()
                            .any(|e| matches!(self.writes[e.0 as usize], K_STRUCT | K_STR))
                        {
                            return None; // struct/string/scalar mix can never type
                        }
                        let mut elem: Option<Kind> = None;
                        for e in elements {
                            narrow!(e.0, K_NUM);
                            match mask_singleton(self.writes[e.0 as usize]) {
                                Some(k @ (Kind::Int | Kind::Float)) => match elem {
                                    None => elem = Some(k),
                                    Some(prev) if prev != k => return None,
                                    _ => {}
                                },
                                _ => return None,
                            }
                        }
                        match elem {
                            Some(Kind::Int) => Kind::ListInt,
                            Some(Kind::Float) => Kind::ListFloat,
                            _ => return None,
                        }
                    };
                    if self.exotic[dst.0 as usize].is_none() {
                        self.exotic[dst.0 as usize] = Some(lk);
                        changed = true;
                    } else if self.exotic[dst.0 as usize] != Some(lk) {
                        return None;
                    }
                }
                Instruction::MakeMap { dst, entries } => {
                    grow!(self.writes[dst.0 as usize], K_MAP);
                    // Defer until every key and value mask resolves.
                    if entries.iter().any(|(k, v)| {
                        self.writes[k.0 as usize] == 0 || self.writes[v.0 as usize] == 0
                    }) {
                        continue;
                    }
                    let mut payload = Payload::Absent;
                    for (k, v) in entries {
                        narrow!(k.0, K_STR);
                        if self.writes[k.0 as usize] != K_STR {
                            return None; // stringified non-string keys stay on bytecode
                        }
                        narrow!(v.0, K_NUM | K_BOOL | K_STR);
                        let pv = match mask_singleton(self.writes[v.0 as usize]) {
                            Some(Kind::Int) => Payload::Int,
                            Some(Kind::Bool) => Payload::Bool,
                            Some(Kind::Float) => Payload::Float,
                            Some(Kind::Str) => Payload::Str,
                            _ => return None,
                        };
                        payload = join_payload(payload, pv)?;
                    }
                    let mk = Kind::Map(payload);
                    match self.exotic[dst.0 as usize] {
                        None => {
                            self.exotic[dst.0 as usize] = Some(mk);
                            changed = true;
                        }
                        Some(prev) if prev == mk => {}
                        Some(prev) => {
                            let j = join_exotic(prev, mk)?;
                            if j != prev {
                                self.exotic[dst.0 as usize] = Some(j);
                                changed = true;
                            }
                        }
                    }
                }
                Instruction::CallNamed {
                    dst,
                    function_name,
                    args,
                } => {
                    // Whitelisted map natives only — and only while no
                    // user definition shadows the name (a later shadow
                    // demotes compiled entries via note_shadow).
                    if shadowed.contains(function_name.as_str()) {
                        return None;
                    }
                    match function_name.as_str() {
                        "map_get" if args.len() == 2 => {
                            narrow!(args[0].0, K_MAP);
                            narrow!(args[1].0, K_STR);
                            if let Some(Kind::Map(p)) = self.exotic[args[0].0 as usize] {
                                let k = match p {
                                    Payload::Int => Kind::Int,
                                    Payload::Bool => Kind::Bool,
                                    Payload::Float => Kind::Float,
                                    Payload::Str => Kind::Str,
                                    // Observed empty: every read misses and
                                    // the guard deopts; Int keeps the dst
                                    // register typeable.
                                    Payload::Absent => Kind::Int,
                                };
                                grow!(self.writes[dst.0 as usize], kind_mask(k));
                            }
                            // exotic None: a callee's map resolves late — defer.
                        }
                        "map_has_key" if args.len() == 2 => {
                            narrow!(args[0].0, K_MAP);
                            narrow!(args[1].0, K_STR);
                            grow!(self.writes[dst.0 as usize], K_BOOL);
                        }
                        "map_set" if args.len() == 3 => {
                            narrow!(args[0].0, K_MAP);
                            narrow!(args[1].0, K_STR);
                            narrow!(args[2].0, K_NUM | K_BOOL | K_STR);
                            grow!(self.writes[dst.0 as usize], K_MAP);
                            let (Some(Kind::Map(p)), Some(vk)) = (
                                self.exotic[args[0].0 as usize],
                                mask_singleton(self.writes[args[2].0 as usize]),
                            ) else {
                                continue; // defer until both resolve
                            };
                            let pv = match vk {
                                Kind::Int => Payload::Int,
                                Kind::Bool => Payload::Bool,
                                Kind::Float => Payload::Float,
                                Kind::Str => Payload::Str,
                                _ => return None,
                            };
                            let mk = Kind::Map(join_payload(p, pv)?);
                            match self.exotic[dst.0 as usize] {
                                None => {
                                    self.exotic[dst.0 as usize] = Some(mk);
                                    changed = true;
                                }
                                Some(prev) if prev == mk => {}
                                Some(prev) => {
                                    let j = join_exotic(prev, mk)?;
                                    if j != prev {
                                        self.exotic[dst.0 as usize] = Some(j);
                                        changed = true;
                                    }
                                }
                            }
                        }
                        _ => return None,
                    }
                }
                Instruction::ExtractResult {
                    dst,
                    value,
                    want_ok,
                } => {
                    narrow!(value.0, K_RESULT);
                    if let Some(Kind::Result(okp, errp)) = self.exotic[value.0 as usize] {
                        let k = match if *want_ok { okp } else { errp } {
                            Payload::Int => Kind::Int,
                            Payload::Bool => Kind::Bool,
                            Payload::Float => Kind::Float,
                            Payload::Str => Kind::Str,
                            // A side never observed: its extraction is
                            // unreachable (the guarding test is always
                            // false) and its guard can only deopt, so any
                            // claim is vacuous — Int keeps the dead
                            // branch's registers typeable.
                            Payload::Absent => Kind::Int,
                        };
                        grow!(self.writes[dst.0 as usize], kind_mask(k));
                    }
                }
                Instruction::MakeStruct {
                    dst,
                    shape,
                    field_regs,
                    field_types,
                } => {
                    let mut kinds = Vec::with_capacity(field_regs.len());
                    let mut resolved = true;
                    for (i, r) in field_regs.iter().enumerate() {
                        narrow!(r.0, K_NUM | K_BOOL);
                        match mask_singleton(self.writes[r.0 as usize]) {
                            Some(k) => {
                                // A field whose provably-known kind contradicts
                                // its declared type can never satisfy the
                                // run-time check. Refuse the whole function so
                                // it stays on bytecode, which raises the same
                                // error the interpreter would — the JIT never
                                // builds a mistyped struct.
                                if let Some(Some(check)) = field_types.get(i) {
                                    use crate::ast::FieldTypeCheck;
                                    let ok = matches!(
                                        (check, k),
                                        (FieldTypeCheck::Int, Kind::Int)
                                            | (FieldTypeCheck::Float, Kind::Float)
                                            | (FieldTypeCheck::Bool, Kind::Bool)
                                    );
                                    if !ok {
                                        return None;
                                    }
                                }
                                kinds.push(k);
                            }
                            None => {
                                resolved = false;
                                break;
                            }
                        }
                    }
                    grow!(self.writes[dst.0 as usize], K_STRUCT);
                    let k = Kind::Struct(shape.id);
                    if self.exotic[dst.0 as usize].is_none() {
                        self.exotic[dst.0 as usize] = Some(k);
                        changed = true;
                    } else if self.exotic[dst.0 as usize] != Some(k) {
                        return None;
                    }
                    if resolved && !self.made_shapes.contains_key(&shape.id) {
                        self.made_shapes.insert(
                            shape.id,
                            ShapeSpec {
                                shape: shape.clone(),
                                field_kinds: kinds,
                            },
                        );
                        changed = true;
                    }
                }
                Instruction::IterLen { dst, src } => {
                    narrow!(src.0, K_LIST);
                    if !matches!(
                        self.exotic[src.0 as usize],
                        Some(Kind::ListFloat | Kind::ListInt | Kind::ListStruct(_) | Kind::ListStr)
                    ) {
                        return None; // ranges etc. stay on bytecode
                    }
                    grow!(self.writes[dst.0 as usize], K_INT);
                }
                Instruction::IterGet { dst, src, idx } => {
                    narrow!(src.0, K_LIST);
                    narrow!(idx.0, K_INT);
                    let elem = match self.exotic[src.0 as usize] {
                        Some(Kind::ListFloat) => Kind::Float,
                        Some(Kind::ListInt) => Kind::Int,
                        Some(Kind::ListStruct(sid)) => Kind::Struct(sid),
                        Some(Kind::ListStr) => Kind::Str,
                        _ => return None,
                    };
                    grow!(self.writes[dst.0 as usize], kind_mask(elem));
                    if matches!(elem, Kind::Struct(_)) {
                        if self.exotic[dst.0 as usize].is_none() {
                            self.exotic[dst.0 as usize] = Some(elem);
                            changed = true;
                        } else if self.exotic[dst.0 as usize] != Some(elem) {
                            return None;
                        }
                    }
                }
                Instruction::CallBuiltin { dst, args, .. } => {
                    for a in args.iter() {
                        narrow!(a.0, K_NUM);
                    }
                    grow!(self.writes[dst.0 as usize], K_FLOAT);
                }
                Instruction::Jump { .. } | Instruction::MatchFail | Instruction::Nop => {}
                Instruction::JumpIfTrue { condition, .. }
                | Instruction::JumpIfFalse { condition, .. } => {
                    narrow!(condition.0, K_BOOL);
                }
                Instruction::CallFn {
                    dst, func_id, args, ..
                } => {
                    if let Some((
                        param_kinds,
                        ret_mask,
                        ret_tuple,
                        ret_struct,
                        ret_result,
                        ret_list,
                    )) = sigs.get(&func_id.index())
                    {
                        if args.len() != param_kinds.len() {
                            return None;
                        }
                        for (i, a) in args.iter().enumerate() {
                            narrow!(a.0, kind_mask(param_kinds[i]));
                            if matches!(kind_mask(param_kinds[i]), K_STRUCT | K_LIST | K_RESULT)
                                && self.exotic[a.0 as usize] != Some(param_kinds[i])
                            {
                                return None; // exotic kinds must match exactly
                            }
                        }
                        let rm = *ret_mask;
                        grow!(self.writes[dst.0 as usize], rm);
                        if let Some(tk) = ret_tuple {
                            if let Some(prev) = self.tuples.get(&dst.0) {
                                if prev != tk {
                                    return None;
                                }
                            } else {
                                self.tuples.insert(dst.0, tk.clone());
                                changed = true;
                            }
                        }
                        if let Some(sid) = ret_struct {
                            // Heap-returning callees allocate into the ENTRY
                            // call's scratch context; in a loop, finalize's
                            // watermark gate proves the values die per
                            // iteration or refuses the function.
                            let k = Kind::Struct(*sid);
                            if self.exotic[dst.0 as usize].is_none() {
                                self.exotic[dst.0 as usize] = Some(k);
                                changed = true;
                            } else if self.exotic[dst.0 as usize] != Some(k) {
                                return None;
                            }
                        }
                        if let Some(lk) = ret_list {
                            if self.exotic[dst.0 as usize].is_none() {
                                self.exotic[dst.0 as usize] = Some(*lk);
                                changed = true;
                            } else if self.exotic[dst.0 as usize] != Some(*lk) {
                                return None;
                            }
                        }
                        if let Some(rk) = ret_result {
                            match self.exotic[dst.0 as usize] {
                                None => {
                                    self.exotic[dst.0 as usize] = Some(*rk);
                                    changed = true;
                                }
                                Some(prev) if prev == *rk => {}
                                Some(prev) => {
                                    let j = join_exotic(prev, *rk)?;
                                    if j != prev {
                                        self.exotic[dst.0 as usize] = Some(j);
                                        changed = true;
                                    }
                                }
                            }
                        }
                    } else {
                        // Unknown callee: once every argument register has
                        // resolved to a single numeric/bool kind, request it
                        // for planning. Until then, keep iterating.
                        let mut kinds = Vec::with_capacity(args.len());
                        let mut resolved = true;
                        for a in args {
                            if let Some(k) = self.exotic[a.0 as usize] {
                                kinds.push(k);
                                continue;
                            }
                            match mask_singleton(self.writes[a.0 as usize]) {
                                Some(k) => kinds.push(k),
                                None => {
                                    resolved = false;
                                    break;
                                }
                            }
                        }
                        if resolved {
                            requests.push((*func_id, kinds));
                        }
                    }
                }
                Instruction::Return { value } => {
                    let reg = (*value)?;
                    // Decide the return shape by RESOLVED state, never by a
                    // mid-fixpoint writes mask: narrowing on a premature
                    // mask is irreversible (allowed only shrinks) and
                    // poisoned struct returns on the first pass.
                    if let Some(Kind::Struct(sid)) = self.exotic[reg.0 as usize] {
                        narrow!(reg.0, K_STRUCT);
                        match self.ret_struct {
                            None => {
                                self.ret_struct = Some(sid);
                                changed = true;
                            }
                            Some(prev) if prev != sid => return None,
                            _ => {}
                        }
                        self.return_regs.push(reg.0);
                        grow!(self.ret_mask, K_STRUCT);
                    } else if let Some(rk @ (Kind::Result(..) | Kind::Map(_))) =
                        self.exotic[reg.0 as usize]
                    {
                        narrow!(reg.0, kind_mask(rk));
                        match self.ret_result {
                            None => {
                                self.ret_result = Some(rk);
                                changed = true;
                            }
                            Some(prev) if prev == rk => {}
                            Some(prev) => {
                                let j = join_exotic(prev, rk)?;
                                if j != prev {
                                    self.ret_result = Some(j);
                                    changed = true;
                                }
                            }
                        }
                        self.return_regs.push(reg.0);
                        grow!(self.ret_mask, kind_mask(rk));
                    } else if let Some(
                        lk
                        @ (Kind::ListInt | Kind::ListFloat | Kind::ListStruct(_) | Kind::ListStr),
                    ) = self.exotic[reg.0 as usize]
                    {
                        narrow!(reg.0, K_LIST);
                        match self.ret_list {
                            None => {
                                self.ret_list = Some(lk);
                                changed = true;
                            }
                            Some(prev) if prev != lk => return None,
                            _ => {}
                        }
                        self.return_regs.push(reg.0);
                        grow!(self.ret_mask, K_LIST);
                    } else if let Some(tk) = self.tuples.get(&reg.0).cloned() {
                        narrow!(reg.0, K_TUPLE);
                        match &self.ret_tuple {
                            None => {
                                self.ret_tuple = Some(tk);
                                changed = true;
                            }
                            Some(prev) if *prev != tk => return None,
                            _ => {}
                        }
                        self.return_regs.push(reg.0);
                        grow!(self.ret_mask, K_TUPLE);
                    } else if self.writes[reg.0 as usize] == K_STR {
                        narrow!(reg.0, K_STR);
                        self.return_regs.push(reg.0);
                        grow!(self.ret_mask, K_STR);
                    } else if self.writes[reg.0 as usize] == 0 {
                        // Unresolved (e.g. dst of a call whose return kind
                        // isn't known yet) — contribute nothing this pass;
                        // the fixpoint keeps iterating until it resolves.
                    } else {
                        narrow!(reg.0, K_NUM | K_BOOL);
                        self.return_regs.push(reg.0);
                        grow!(self.ret_mask, self.writes[reg.0 as usize]);
                    }
                }
                other => {
                    if jit_debug() {
                        eprintln!("[jit] infer_pass: unhandled {}", instruction_name(other));
                    }
                    return None;
                }
            }
        }

        if changed {
            *global_changed = true;
        }
        Some(())
    }

    /// After the global fixpoint: check singletons and produce the
    /// codegen-facing result.
    fn finalize(&self) -> Option<Inference> {
        let nregs = self.writes.len();
        let mut reg_kind: Vec<Option<Kind>> = vec![None; nregs];
        for (r, slot) in reg_kind.iter_mut().enumerate() {
            if self.was_read[r] {
                if matches!(self.writes[r], K_STRUCT | K_LIST | K_RESULT | K_MAP) {
                    let k = self.exotic[r]?;
                    if kind_mask(k) & self.allowed[r] == 0 {
                        return None;
                    }
                    *slot = Some(k);
                    continue;
                }
                if self.writes[r] == K_TUPLE {
                    // Tuple registers have no single Kind; codegen reads
                    // them through the tuples map. Requires resolution.
                    if !self.tuples.contains_key(&(r as u32)) || self.allowed[r] & K_TUPLE == 0 {
                        return None;
                    }
                    continue;
                }
                let k = mask_singleton(self.writes[r])?;
                if kind_mask(k) & self.allowed[r] == 0 {
                    return None;
                }
                *slot = Some(k);
            }
        }

        for (l, r) in &self.eq_pairs {
            let lk = reg_kind[*l as usize]?;
            let rk = reg_kind[*r as usize]?;
            let both_num =
                matches!(lk, Kind::Int | Kind::Float) && matches!(rk, Kind::Int | Kind::Float);
            let both_bool = lk == Kind::Bool && rk == Kind::Bool;
            let both_str = lk == Kind::Str && rk == Kind::Str;
            if !both_num && !both_bool && !both_str {
                return None;
            }
        }

        let (ret_kind, ret_tuple) = if self.ret_mask == K_STR {
            for r in &self.return_regs {
                if mask_singleton(self.writes[*r as usize]) != Some(Kind::Str) {
                    return None;
                }
            }
            (Kind::Str, None)
        } else if self.ret_mask == K_STRUCT {
            let sid = self.ret_struct?;
            for r in &self.return_regs {
                if self.exotic[*r as usize] != Some(Kind::Struct(sid)) {
                    return None;
                }
            }
            (Kind::Struct(sid), None)
        } else if self.ret_mask == K_TUPLE {
            let tk = self.ret_tuple.clone()?;
            for r in &self.return_regs {
                if self.tuples.get(r) != Some(&tk) {
                    return None;
                }
            }
            // The scalar slot is unused for tuple returns; Int is a
            // placeholder for signatures that never carry it.
            (Kind::Int, Some(tk))
        } else if self.ret_mask == K_LIST {
            let lk = self.ret_list?;
            for r in &self.return_regs {
                if self.exotic[*r as usize] != Some(lk) {
                    return None;
                }
            }
            (lk, None)
        } else if self.ret_mask == K_RESULT || self.ret_mask == K_MAP {
            let rk = self.ret_result?;
            for r in &self.return_regs {
                let k = self.exotic[*r as usize]?;
                // Every return site's kind must fold into the joined
                // return kind (rk was built from them, so join is a
                // consistency re-check, not new information).
                if join_exotic(k, rk) != Some(rk) {
                    return None;
                }
            }
            (rk, None)
        } else {
            let k = mask_singleton(self.ret_mask)?;
            for r in &self.return_regs {
                if reg_kind[*r as usize] != Some(k) {
                    return None;
                }
            }
            (k, None)
        };

        // ── the watermark gate ──
        //
        // Allocation inside a loop used to refuse wholesale. It is now
        // allowed exactly when the loop can be watermarked: a single
        // region, and no heap-kind register defined inside it is live at
        // the head — liveness at the head covers both a later iteration
        // reading the pointer and every after-loop path (a while-loop
        // exits THROUGH the head after the back-edge truncated). Values
        // that escaped into another owner survive on their own Arc; only
        // raw borrowed pointers can dangle, and the gate proves none do.
        let is_heap = |k: Option<Kind>| {
            matches!(
                k,
                Some(
                    Kind::Struct(_)
                        | Kind::Str
                        | Kind::Result(..)
                        | Kind::Map(_)
                        | Kind::ListInt
                        | Kind::ListFloat
                        | Kind::ListStruct(_)
                        | Kind::ListStr
                )
            )
        };
        let mut scratch_region = None;
        if has_backward_jump(&self.bytecode) {
            // The forms the old rules forbade alongside any backward
            // jump (the whitelist was function-global, so this check is
            // function-global too — nothing that compiled before stops).
            let newly_allowed = self.bytecode.instructions.iter().any(|inst| match inst {
                Instruction::MakeStruct { .. }
                | Instruction::MakeList { .. }
                | Instruction::MakeMap { .. }
                | Instruction::MakeResult { .. } => true,
                Instruction::Add { dst, .. } => is_heap(reg_kind[dst.0 as usize]),
                Instruction::CallFn { dst, .. } => is_heap(reg_kind[dst.0 as usize]),
                Instruction::CallNamed { function_name, .. } => function_name == "map_set",
                _ => false,
            });
            if newly_allowed {
                let (h, e) = self.loop_region?; // several loop heads: refuse, as before
                for (r, kind) in reg_kind.iter().enumerate() {
                    if self.defined_in_region.get(r).copied().unwrap_or(true)
                        && is_heap(*kind)
                        && self.live_at_head.get(r).copied().unwrap_or(true)
                    {
                        if jit_debug() {
                            eprintln!(
                                "[jit] fn#{} refused: r{} escapes its loop iteration",
                                self.func_id.index(),
                                r
                            );
                        }
                        return None;
                    }
                }
                scratch_region = Some((h, e));
            } else if let Some((h, e)) = self.loop_region {
                // Nothing newly allowed, but callees may still allocate
                // into the entry scratch (an int-returning callee's
                // internals): watermarking bounds that too, and is safe
                // here because every heap value the region can reference
                // lives below the mark.
                let region_allocates = self.bytecode.instructions[h..=e].iter().any(|inst| {
                    match inst {
                        Instruction::CallFn { .. } => true,
                        // Among the named builtins only map_set allocates;
                        // guarded reads never do, and a release per
                        // back-edge is measurable in a hot read loop.
                        Instruction::CallNamed { function_name, .. } => function_name == "map_set",
                        _ => false,
                    }
                });
                if region_allocates {
                    scratch_region = Some((h, e));
                }
            }
        }

        Some(Inference {
            reg_kind,
            param_kinds: self.param_kinds.clone(),
            ret_kind,
            tuples: self.tuples.clone(),
            ret_tuple,
            ret_struct: self.ret_struct,
            scratch_region,
        })
    }
}

fn instruction_name(inst: &Instruction) -> &'static str {
    match inst {
        Instruction::LoadConst { .. } => "LoadConst",
        Instruction::LoadLocal { .. } => "LoadLocal",
        Instruction::StoreLocal { .. } => "StoreLocal",
        Instruction::Move { .. } => "Move",
        Instruction::TakeMove { .. } => "TakeMove",
        Instruction::Add { .. } => "Add",
        Instruction::AddAssign { .. } => "AddAssign",
        Instruction::Sub { .. } => "Sub",
        Instruction::Mul { .. } => "Mul",
        Instruction::Div { .. } => "Div",
        Instruction::Mod { .. } => "Mod",
        Instruction::Neg { .. } => "Neg",
        Instruction::Eq { .. } => "Eq",
        Instruction::Ne { .. } => "Ne",
        Instruction::Lt { .. } => "Lt",
        Instruction::Le { .. } => "Le",
        Instruction::Gt { .. } => "Gt",
        Instruction::Ge { .. } => "Ge",
        Instruction::And { .. } => "And",
        Instruction::Or { .. } => "Or",
        Instruction::Not { .. } => "Not",
        Instruction::Jump { .. } => "Jump",
        Instruction::JumpIfTrue { .. } => "JumpIfTrue",
        Instruction::JumpIfFalse { .. } => "JumpIfFalse",
        Instruction::Call { .. } => "Call",
        Instruction::CallFn { .. } => "CallFn",
        Instruction::CallValue { .. } => "CallValue",
        Instruction::CallBuiltin { .. } => "CallBuiltin",
        Instruction::CallNamed { .. } => "CallNamed",
        Instruction::CallMethod { .. } => "CallMethod",
        Instruction::IterLen { .. } => "IterLen",
        Instruction::IterGet { .. } => "IterGet",
        Instruction::GetField { .. } => "GetField",
        Instruction::IndexGet { .. } => "IndexGet",
        Instruction::BinImm { .. } => "BinImm",
        Instruction::Return { .. } => "Return",
        Instruction::MatchFail => "MatchFail",
        Instruction::MakeResult { .. } => "MakeResult",
        Instruction::PatternTestResult { .. } => "PatternTestResult",
        Instruction::ExtractResult { .. } => "ExtractResult",
        Instruction::MakeList { .. } => "MakeList",
        Instruction::MakeMap { .. } => "MakeMap",
        _ => "other",
    }
}

// ── codegen ────────────────────────────────────────────────────────────

/// The imported host helpers, declared once per group.
#[derive(Clone, Copy)]
struct Helpers {
    field: cranelift_module::FuncId,
    math: cranelift_module::FuncId,
    index: cranelift_module::FuncId,
    len: cranelift_module::FuncId,
    make_struct: cranelift_module::FuncId,
    str_cmp: cranelift_module::FuncId,
    str_concat: cranelift_module::FuncId,
    result_test: cranelift_module::FuncId,
    result_extract: cranelift_module::FuncId,
    make_result: cranelift_module::FuncId,
    make_list: cranelift_module::FuncId,
    make_list_structs: cranelift_module::FuncId,
    make_list_strs: cranelift_module::FuncId,
    list_concat: cranelift_module::FuncId,
    map_get: cranelift_module::FuncId,
    map_has: cranelift_module::FuncId,
    map_set: cranelift_module::FuncId,
    make_map: cranelift_module::FuncId,
    mark: cranelift_module::FuncId,
    release: cranelift_module::FuncId,
}

struct Gen<'a> {
    inference: &'a Inference,
    deopt_block: cranelift_codegen::ir::Block,
}

impl Gen<'_> {
    fn kind(&self, reg: u32) -> Option<Kind> {
        self.inference.reg_kind.get(reg as usize).copied().flatten()
    }

    /// use_var for a live register; None if the register is dead (the
    /// caller decides whether that makes the instruction skippable).
    fn read(&self, builder: &mut FunctionBuilder, reg: u32) -> Option<ClifValue> {
        self.kind(reg)?;
        Some(builder.use_var(Variable::from_u32(reg)))
    }

    /// def_var unless the register is dead.
    fn write(&self, builder: &mut FunctionBuilder, reg: u32, value: ClifValue) {
        if self.kind(reg).is_some() {
            builder.def_var(Variable::from_u32(reg), value);
        }
    }

    /// Promote a value to f64 if its register kind is Int.
    fn to_float(&self, builder: &mut FunctionBuilder, value: ClifValue, k: Kind) -> ClifValue {
        match k {
            Kind::Float => value,
            _ => builder.ins().fcvt_from_sint(types::F64, value),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn translate_body(
    builder: &mut FunctionBuilder,
    module: &mut JITModule,
    targets: &HashMap<usize, (cranelift_module::FuncId, Kind, Option<Vec<Kind>>)>,
    bytecode: &CompiledBytecode,
    inference: &Inference,
    shapes: &HashMap<u32, ShapeSpec>,
    helpers: Helpers,
) -> Option<()> {
    let Helpers {
        field: field_helper,
        math: math_helper,
        index: index_helper,
        len: len_helper,
        make_struct: make_struct_helper,
        str_cmp: str_cmp_helper,
        str_concat: str_concat_helper,
        result_test: result_test_helper,
        result_extract: result_extract_helper,
        make_result: make_result_helper,
        make_list: make_list_helper,
        make_list_structs: make_list_structs_helper,
        make_list_strs: make_list_strs_helper,
        list_concat: list_concat_helper,
        map_get: map_get_helper,
        map_has: map_has_helper,
        map_set: map_set_helper,
        make_map: make_map_helper,
        mark: mark_helper,
        release: release_helper,
    } = helpers;
    let n = bytecode.instructions.len();
    let param_count = inference.param_kinds.len();
    let ptr_ty = module.target_config().pointer_type();

    // Block leaders: entry, every jump target, every instruction after a
    // conditional jump (the fallthrough edge needs a block).
    let mut is_leader = vec![false; n];
    is_leader[bytecode.entry_point] = true;
    is_leader[0] = true;
    for (i, inst) in bytecode.instructions.iter().enumerate() {
        match inst {
            Instruction::Jump { target } => {
                let t = target.0 as usize;
                if t >= n {
                    return None;
                }
                is_leader[t] = true;
            }
            Instruction::JumpIfTrue { target, .. } | Instruction::JumpIfFalse { target, .. } => {
                let t = target.0 as usize;
                if t >= n {
                    return None;
                }
                is_leader[t] = true;
                if i + 1 < n {
                    is_leader[i + 1] = true;
                }
            }
            _ => {}
        }
    }

    let entry_block = builder.create_block();
    builder.append_block_params_for_function_params(entry_block);
    let deopt_block = builder.create_block();
    let r#gen = Gen {
        inference,
        deopt_block,
    };

    let mut blocks = vec![None; n];
    for (i, leader) in is_leader.iter().enumerate() {
        if *leader {
            blocks[i] = Some(builder.create_block());
        }
    }

    // One typed variable per register, plus the depth budget and ctx.
    // Tuple registers get one variable per element, allocated past the
    // scalar space: register r's element i lives at
    // tuple_base + r*MAX_TUPLE + i.
    //
    // cranelift 0.134's declare_var allocates Variable indices
    // sequentially, so the dense index scheme above is preserved by
    // declaring every slot in index order; dead slots get an I64 filler
    // that is never read or written.
    let nregs = bytecode.register_count as usize;
    let depth_var = Variable::from_u32(nregs as u32);
    let ctx_var = Variable::from_u32(nregs as u32 + 1);
    let tuple_base = nregs as u32 + 2;
    let tuple_var =
        |r: u32, i: usize| Variable::from_u32(tuple_base + r * MAX_TUPLE as u32 + i as u32);
    let total_vars = match inference.tuples.keys().copied().max() {
        Some(r) => tuple_base as usize + (r as usize + 1) * MAX_TUPLE,
        None => nregs + 2,
    };
    let mut var_types = vec![types::I64; total_vars];
    for (r, slot) in var_types.iter_mut().enumerate().take(nregs) {
        if let Some(k) = r#gen.kind(r as u32) {
            *slot = k.clif_type();
        }
    }
    for (r, tk) in &inference.tuples {
        for (i, k) in tk.iter().enumerate() {
            var_types[tuple_var(*r, i).as_u32() as usize] = k.clif_type();
        }
    }
    for (i, ty) in var_types.iter().enumerate() {
        let declared = builder.declare_var(*ty);
        debug_assert_eq!(declared, Variable::from_u32(i as u32));
    }

    builder.switch_to_block(entry_block);
    let params: Vec<ClifValue> = builder.block_params(entry_block).to_vec();
    for (i, p) in params.iter().take(param_count).enumerate() {
        r#gen.write(builder, i as u32, *p);
    }
    // Initialize every other live register so use before first def can't
    // trip the SSA builder (bytecode never actually reads uninitialized
    // registers, but proving that is the verifier's job, not ours).
    for r in param_count..nregs {
        if let Some(k) = r#gen.kind(r as u32) {
            let zero = match k {
                Kind::Float => builder.ins().f64const(0.0),
                _ => builder.ins().iconst(types::I64, 0),
            };
            builder.def_var(Variable::from_u32(r as u32), zero);
        }
    }
    for (r, tk) in &inference.tuples {
        for (i, k) in tk.iter().enumerate() {
            let zero = match k {
                Kind::Float => builder.ins().f64const(0.0),
                _ => builder.ins().iconst(types::I64, 0),
            };
            builder.def_var(tuple_var(*r, i), zero);
        }
    }
    builder.def_var(depth_var, params[param_count]);
    builder.def_var(ctx_var, params[param_count + 1]);
    let first = blocks[bytecode.entry_point]
        .or(blocks[0])
        .expect("entry leader");
    builder.ins().jump(first, &[]);

    // The entry block just terminated; the loop's leader handling switches
    // into the first real block without adding a second jump.
    let mut terminated = true;

    for (i, inst) in bytecode.instructions.iter().enumerate() {
        if let Some(block) = blocks[i] {
            if !terminated {
                // Falling into the watermarked loop's head: remember every
                // scratch family's length so each back-edge can free the
                // iteration's allocations.
                if inference.scratch_region.is_some_and(|(h, _)| h == i) {
                    let ctx = builder.use_var(ctx_var);
                    let mark_ref = module.declare_func_in_func(mark_helper, builder.func);
                    builder.ins().call(mark_ref, &[ctx]);
                }
                builder.ins().jump(block, &[]);
            }
            builder.switch_to_block(block);
            terminated = false;
        }
        if terminated {
            // Dead code between a terminator and the next leader.
            continue;
        }

        match inst {
            Instruction::LoadConst { dst, const_idx } => {
                let Some(dk) = r#gen.kind(dst.0) else {
                    continue;
                };
                let val = match (&bytecode.constants[*const_idx as usize].data, dk) {
                    (ValueData::Integer(x), Kind::Int) => builder.ins().iconst(types::I64, *x),
                    (ValueData::Boolean(b), Kind::Bool) => {
                        builder.ins().iconst(types::I64, *b as i64)
                    }
                    (ValueData::Float(f), Kind::Float) => builder.ins().f64const(*f),
                    // The constant's Arc lives in the bytecode, which the
                    // JittedFn owns — a baked borrowed pointer is sound.
                    (ValueData::String(s), Kind::Str) => {
                        builder.ins().iconst(types::I64, Arc::as_ptr(s) as i64)
                    }
                    _ => return None,
                };
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::MatchFail => {
                // Unreachable on real paths; if control ever got here the
                // deopt re-run raises the canonical match-failure error.
                builder.ins().jump(deopt_block, &[]);
                terminated = true;
            }
            Instruction::Nop => {}
            Instruction::TailCallSelf { args } => {
                // Read every argument before writing any parameter — an
                // argument may be the very parameter register it rebinds.
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    let Some(v) = r#gen.read(builder, a.0) else {
                        if jit_debug() {
                            eprintln!("[jit] tailcall arg read failed: reg {}", a.0);
                        }
                        return None;
                    };
                    vals.push(v);
                }
                for (i, v) in vals.into_iter().enumerate() {
                    builder.def_var(Variable::from_u32(i as u32), v);
                }
                // The entry is always a leader; looping back re-enters
                // the body with the freshly bound parameters — a native
                // loop, no frame, no depth spent.
                let Some(entry) = blocks[bytecode.entry_point] else {
                    if jit_debug() {
                        eprintln!("[jit] tailcall: entry block missing");
                    }
                    return None;
                };
                builder.ins().jump(entry, &[]);
                terminated = true;
            }
            Instruction::Move { dst, src } => {
                if let Some(tk) = inference.tuples.get(&src.0) {
                    if inference.tuples.contains_key(&dst.0) {
                        for i in 0..tk.len() {
                            let v = builder.use_var(tuple_var(src.0, i));
                            builder.def_var(tuple_var(dst.0, i), v);
                        }
                    }
                    continue;
                }
                if r#gen.kind(dst.0).is_none() {
                    continue; // dead store, no observable effect
                }
                let val = r#gen.read(builder, src.0)?;
                builder.def_var(Variable::from_u32(dst.0), val);
            }
            Instruction::Add { dst, lhs, rhs }
            | Instruction::Sub { dst, lhs, rhs }
            | Instruction::Mul { dst, lhs, rhs }
            | Instruction::Div { dst, lhs, rhs }
            | Instruction::Mod { dst, lhs, rhs } => {
                let lk = r#gen.kind(lhs.0)?;
                let rk = r#gen.kind(rhs.0)?;
                if matches!(
                    lk,
                    Kind::ListInt | Kind::ListFloat | Kind::ListStruct(_) | Kind::ListStr
                ) {
                    if !matches!(inst, Instruction::Add { .. }) || rk != lk {
                        return None;
                    }
                    let a = builder.use_var(Variable::from_u32(lhs.0));
                    let b = builder.use_var(Variable::from_u32(rhs.0));
                    let ctx = builder.use_var(ctx_var);
                    let helper_ref = module.declare_func_in_func(list_concat_helper, builder.func);
                    let call = builder.ins().call(helper_ref, &[ctx, a, b]);
                    let ptr = builder.inst_results(call)[0];
                    let null = builder.ins().icmp_imm_s(IntCC::Equal, ptr, 0);
                    let ok_block = builder.create_block();
                    builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                    builder.switch_to_block(ok_block);
                    r#gen.write(builder, dst.0, ptr);
                    continue;
                }
                if lk == Kind::Str && rk == Kind::Str {
                    if !matches!(inst, Instruction::Add { .. }) {
                        return None;
                    }
                    let a = builder.use_var(Variable::from_u32(lhs.0));
                    let b = builder.use_var(Variable::from_u32(rhs.0));
                    let ctx = builder.use_var(ctx_var);
                    let helper_ref = module.declare_func_in_func(str_concat_helper, builder.func);
                    let call = builder.ins().call(helper_ref, &[ctx, a, b]);
                    let ptr = builder.inst_results(call)[0];
                    let null = builder.ins().icmp_imm_s(IntCC::Equal, ptr, 0);
                    let ok_block = builder.create_block();
                    builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                    builder.switch_to_block(ok_block);
                    r#gen.write(builder, dst.0, ptr);
                    continue;
                }
                let a = builder.use_var(Variable::from_u32(lhs.0));
                let b = builder.use_var(Variable::from_u32(rhs.0));
                let op = arith_op(inst);
                let val = emit_arith(builder, &r#gen, op, a, lk, b, rk)?;
                r#gen.write(builder, dst.0, val);
            }
            Instruction::Neg { dst, src } => {
                let sk = r#gen.kind(src.0)?;
                let a = builder.use_var(Variable::from_u32(src.0));
                let val = match sk {
                    Kind::Float => builder.ins().fneg(a),
                    Kind::Int => {
                        // checked_neg: only i64::MIN overflows.
                        let min = builder.ins().iconst(types::I64, i64::MIN);
                        let is_min = builder.ins().icmp(IntCC::Equal, a, min);
                        let cont = builder.create_block();
                        builder.ins().brif(is_min, deopt_block, &[], cont, &[]);
                        builder.switch_to_block(cont);
                        builder.ins().ineg(a)
                    }
                    _ => return None,
                };
                r#gen.write(builder, dst.0, val);
            }
            Instruction::Eq { dst, lhs, rhs }
            | Instruction::Ne { dst, lhs, rhs }
            | Instruction::Lt { dst, lhs, rhs }
            | Instruction::Le { dst, lhs, rhs }
            | Instruction::Gt { dst, lhs, rhs }
            | Instruction::Ge { dst, lhs, rhs } => {
                let lk = r#gen.kind(lhs.0)?;
                let rk = r#gen.kind(rhs.0)?;
                let a = builder.use_var(Variable::from_u32(lhs.0));
                let b = builder.use_var(Variable::from_u32(rhs.0));
                if lk == Kind::Str && rk == Kind::Str {
                    let opcode: i64 = match inst {
                        Instruction::Eq { .. } => 0,
                        Instruction::Ne { .. } => 1,
                        Instruction::Lt { .. } => 2,
                        Instruction::Le { .. } => 3,
                        Instruction::Gt { .. } => 4,
                        _ => 5,
                    };
                    let op_v = builder.ins().iconst(types::I64, opcode);
                    let helper_ref = module.declare_func_in_func(str_cmp_helper, builder.func);
                    let call = builder.ins().call(helper_ref, &[a, b, op_v]);
                    let val = builder.inst_results(call)[0];
                    r#gen.write(builder, dst.0, val);
                    continue;
                }
                if (lk == Kind::Str) != (rk == Kind::Str) {
                    return None; // string-vs-other comparisons stay on bytecode
                }
                let flag = if lk == Kind::Float || rk == Kind::Float {
                    let fa = r#gen.to_float(builder, a, lk);
                    let fb = r#gen.to_float(builder, b, rk);
                    builder.ins().fcmp(float_cc(inst), fa, fb)
                } else {
                    builder.ins().icmp(compare_cc(inst), a, b)
                };
                let val = builder.ins().uextend(types::I64, flag);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::And { dst, lhs, rhs } => {
                let a = r#gen.read(builder, lhs.0)?;
                let b = r#gen.read(builder, rhs.0)?;
                let val = builder.ins().band(a, b);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::Or { dst, lhs, rhs } => {
                let a = r#gen.read(builder, lhs.0)?;
                let b = r#gen.read(builder, rhs.0)?;
                let val = builder.ins().bor(a, b);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::Not { dst, src } => {
                let a = r#gen.read(builder, src.0)?;
                let one = builder.ins().iconst(types::I64, 1);
                let val = builder.ins().bxor(a, one);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::BinImm {
                op, dst, lhs, imm, ..
            } => {
                let lk = r#gen.kind(lhs.0)?;
                let a = builder.use_var(Variable::from_u32(lhs.0));
                let (b, bk) = match imm.data {
                    ValueData::Integer(x) => {
                        if lk == Kind::Float {
                            // The VM promotes the int immediate to float.
                            (builder.ins().f64const(x as f64), Kind::Float)
                        } else {
                            (builder.ins().iconst(types::I64, x), Kind::Int)
                        }
                    }
                    ValueData::Float(f) => (builder.ins().f64const(f), Kind::Float),
                    _ => return None,
                };
                match op {
                    BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Multiply
                    | BinaryOp::Divide
                    | BinaryOp::Modulo => {
                        let val = emit_arith(builder, &r#gen, imm_arith(op)?, a, lk, b, bk)?;
                        r#gen.write(builder, dst.0, val);
                    }
                    BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::LessThan
                    | BinaryOp::LessThanEqual
                    | BinaryOp::GreaterThan
                    | BinaryOp::GreaterThanEqual => {
                        let flag = if lk == Kind::Float || bk == Kind::Float {
                            let fa = r#gen.to_float(builder, a, lk);
                            let fb = r#gen.to_float(builder, b, bk);
                            builder.ins().fcmp(imm_float_cc(op)?, fa, fb)
                        } else {
                            builder.ins().icmp(imm_compare(op)?, a, b)
                        };
                        let val = builder.ins().uextend(types::I64, flag);
                        r#gen.write(builder, dst.0, val);
                    }
                    _ => return None,
                }
            }
            Instruction::CallBuiltin {
                dst,
                builtin_id,
                args,
            } => {
                let mut fargs = [None, None];
                for (i, a) in args.iter().enumerate().take(2) {
                    let k = r#gen.kind(a.0)?;
                    let v = builder.use_var(Variable::from_u32(a.0));
                    fargs[i] = Some(r#gen.to_float(builder, v, k));
                }
                let a = fargs[0]?;
                // The VM leaves the unused slot at 0.0 for unary ops.
                let b = fargs[1].unwrap_or_else(|| builder.ins().f64const(0.0));
                // sqrt/floor/ceil/trunc are bit-exact IEEE operations with
                // native instructions; everything else goes through the
                // imported helper, which IS the VM's eval_float_math.
                let name = crate::ovm::bytecode::BytecodeVm::FLOAT_MATH
                    .get(*builtin_id as usize)?
                    .0;
                let val = match name {
                    "math.sqrt" => builder.ins().sqrt(a),
                    "math.floor" => builder.ins().floor(a),
                    "math.ceil" => builder.ins().ceil(a),
                    "math.trunc" => builder.ins().trunc(a),
                    _ => {
                        let id_v = builder.ins().iconst(types::I64, *builtin_id as i64);
                        let helper_ref = module.declare_func_in_func(math_helper, builder.func);
                        let call = builder.ins().call(helper_ref, &[id_v, a, b]);
                        builder.inst_results(call)[0]
                    }
                };
                r#gen.write(builder, dst.0, val);
            }
            Instruction::MakeTuple { dst, elements } => {
                if !inference.tuples.contains_key(&dst.0) {
                    return None;
                }
                for (i, e) in elements.iter().enumerate() {
                    let v = r#gen.read(builder, e.0)?;
                    builder.def_var(tuple_var(dst.0, i), v);
                }
            }
            Instruction::PatternTestTuple { dst, value, len } => {
                // Inference proved the register holds a tuple of exactly
                // this arity, so the test is statically true.
                let tk = inference.tuples.get(&value.0)?;
                if tk.len() != *len {
                    return None;
                }
                let one = builder.ins().iconst(types::I64, 1);
                r#gen.write(builder, dst.0, one);
            }
            Instruction::ExtractElement { dst, value, index } => {
                let tk = inference.tuples.get(&value.0)?;
                if *index >= tk.len() {
                    return None;
                }
                let v = builder.use_var(tuple_var(value.0, *index));
                r#gen.write(builder, dst.0, v);
            }
            Instruction::MakeStruct {
                dst,
                shape,
                field_regs,
                // Type mismatches are caught by the inference pass, which
                // refuses to compile any function that could build a mistyped
                // struct; codegen only runs for structs already proven sound.
                field_types: _,
            } => {
                let n = field_regs.len();
                let slot =
                    builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        (n.max(1) * 8) as u32,
                        3,
                    ));
                let mut kinds_desc: i64 = 0;
                for (i, r) in field_regs.iter().enumerate() {
                    let k = r#gen.kind(r.0)?;
                    let v = builder.use_var(Variable::from_u32(r.0));
                    builder.ins().stack_store(ptr_ty, v, slot, (i * 8) as i32);
                    let code: i64 = match k {
                        Kind::Int => 0,
                        Kind::Float => 1,
                        Kind::Bool => 2,
                        _ => return None,
                    };
                    kinds_desc |= code << (i * 4);
                }
                let ctx = builder.use_var(ctx_var);
                // The Arc lives inside this instruction, which the owning
                // JittedFn keeps alive together with its bytecode.
                let shape_ptr = builder
                    .ins()
                    .iconst(types::I64, shape as *const Arc<_> as i64);
                let fields_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                let n_v = builder.ins().iconst(types::I64, n as i64);
                let kinds_v = builder.ins().iconst(types::I64, kinds_desc);
                let helper_ref = module.declare_func_in_func(make_struct_helper, builder.func);
                let call = builder
                    .ins()
                    .call(helper_ref, &[ctx, shape_ptr, fields_ptr, n_v, kinds_v]);
                let ptr = builder.inst_results(call)[0];
                let null = builder.ins().icmp_imm_s(IntCC::Equal, ptr, 0);
                let ok_block = builder.create_block();
                builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                r#gen.write(builder, dst.0, ptr);
            }
            Instruction::MakeList { dst, elements } => {
                let elem = match r#gen.kind(dst.0) {
                    Some(Kind::ListInt) => Kind::Int,
                    Some(Kind::ListFloat) => Kind::Float,
                    Some(Kind::ListStr) => {
                        let n = elements.len();
                        let slot = builder.create_sized_stack_slot(
                            cranelift_codegen::ir::StackSlotData::new(
                                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                (n.max(1) * 8) as u32,
                                3,
                            ),
                        );
                        for (i, e) in elements.iter().enumerate() {
                            if r#gen.kind(e.0)? != Kind::Str {
                                return None;
                            }
                            let v = builder.use_var(Variable::from_u32(e.0));
                            builder.ins().stack_store(ptr_ty, v, slot, (i * 8) as i32);
                        }
                        let ctx = builder.use_var(ctx_var);
                        let elems_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                        let n_v = builder.ins().iconst(types::I64, n as i64);
                        let helper_ref =
                            module.declare_func_in_func(make_list_strs_helper, builder.func);
                        let call = builder.ins().call(helper_ref, &[ctx, elems_ptr, n_v]);
                        let ptr = builder.inst_results(call)[0];
                        let null = builder.ins().icmp_imm_s(IntCC::Equal, ptr, 0);
                        let ok_block = builder.create_block();
                        builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                        builder.switch_to_block(ok_block);
                        r#gen.write(builder, dst.0, ptr);
                        continue;
                    }
                    Some(Kind::ListStruct(sid)) => {
                        // Element pointers are resolved to owned Arcs by
                        // the helper; an unknown pointer deopts.
                        let n = elements.len();
                        let slot = builder.create_sized_stack_slot(
                            cranelift_codegen::ir::StackSlotData::new(
                                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                (n.max(1) * 8) as u32,
                                3,
                            ),
                        );
                        for (i, e) in elements.iter().enumerate() {
                            if r#gen.kind(e.0)? != Kind::Struct(sid) {
                                return None;
                            }
                            let v = builder.use_var(Variable::from_u32(e.0));
                            builder.ins().stack_store(ptr_ty, v, slot, (i * 8) as i32);
                        }
                        let ctx = builder.use_var(ctx_var);
                        let elems_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                        let n_v = builder.ins().iconst(types::I64, n as i64);
                        let helper_ref =
                            module.declare_func_in_func(make_list_structs_helper, builder.func);
                        let call = builder.ins().call(helper_ref, &[ctx, elems_ptr, n_v]);
                        let ptr = builder.inst_results(call)[0];
                        let null = builder.ins().icmp_imm_s(IntCC::Equal, ptr, 0);
                        let ok_block = builder.create_block();
                        builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                        builder.switch_to_block(ok_block);
                        r#gen.write(builder, dst.0, ptr);
                        continue;
                    }
                    _ => return None,
                };
                let n = elements.len();
                let slot =
                    builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        (n.max(1) * 8) as u32,
                        3,
                    ));
                for (i, e) in elements.iter().enumerate() {
                    if r#gen.kind(e.0)? != elem {
                        return None;
                    }
                    let v = builder.use_var(Variable::from_u32(e.0));
                    builder.ins().stack_store(ptr_ty, v, slot, (i * 8) as i32);
                }
                let ctx = builder.use_var(ctx_var);
                let elems_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                let n_v = builder.ins().iconst(types::I64, n as i64);
                let kind_v = builder
                    .ins()
                    .iconst(types::I64, if elem == Kind::Int { 0 } else { 1 });
                let helper_ref = module.declare_func_in_func(make_list_helper, builder.func);
                let call = builder
                    .ins()
                    .call(helper_ref, &[ctx, elems_ptr, n_v, kind_v]);
                let ptr = builder.inst_results(call)[0];
                let null = builder.ins().icmp_imm_s(IntCC::Equal, ptr, 0);
                let ok_block = builder.create_block();
                builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                r#gen.write(builder, dst.0, ptr);
            }
            Instruction::MakeMap { dst, entries } => {
                let Some(Kind::Map(payload)) = r#gen.kind(dst.0) else {
                    return None;
                };
                let n = entries.len();
                // Two stack buffers in one slot: keys first, values after.
                let slot =
                    builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        ((n * 2).max(1) * 8) as u32,
                        3,
                    ));
                let mut code: i64 = 0;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if r#gen.kind(k.0)? != Kind::Str {
                        return None;
                    }
                    let kv = builder.use_var(Variable::from_u32(k.0));
                    builder.ins().stack_store(ptr_ty, kv, slot, (i * 8) as i32);
                    code = match r#gen.kind(v.0)? {
                        Kind::Int => 0,
                        Kind::Bool => 2,
                        Kind::Str => 4,
                        Kind::Float => 1,
                        _ => return None,
                    };
                    let vv = builder.use_var(Variable::from_u32(v.0));
                    builder
                        .ins()
                        .stack_store(ptr_ty, vv, slot, ((n + i) * 8) as i32);
                }
                let _ = payload;
                let ctx = builder.use_var(ctx_var);
                let keys_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                let vals_ptr = builder.ins().stack_addr(types::I64, slot, (n * 8) as i32);
                let n_v = builder.ins().iconst(types::I64, n as i64);
                let kind_v = builder.ins().iconst(types::I64, code);
                let helper_ref = module.declare_func_in_func(make_map_helper, builder.func);
                let call = builder
                    .ins()
                    .call(helper_ref, &[ctx, keys_ptr, vals_ptr, n_v, kind_v]);
                let ptr = builder.inst_results(call)[0];
                let null = builder.ins().icmp_imm_s(IntCC::Equal, ptr, 0);
                let ok_block = builder.create_block();
                builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                r#gen.write(builder, dst.0, ptr);
            }
            Instruction::CallNamed {
                dst,
                function_name,
                args,
            } => match function_name.as_str() {
                "map_get" => {
                    let Kind::Map(p) = r#gen.kind(args[0].0)? else {
                        return None;
                    };
                    if r#gen.kind(args[1].0)? != Kind::Str {
                        return None;
                    }
                    let (expect, load_ty) = match p {
                        Payload::Int => (FIELD_INT, types::I64),
                        Payload::Float => (FIELD_FLOAT, types::F64),
                        Payload::Bool => (FIELD_BOOL, types::I64),
                        Payload::Str => (FIELD_STR, types::I64),
                        // Observed empty: the guard always deopts, and the
                        // bytecode rerun returns the canonical Unit.
                        Payload::Absent => (FIELD_NEVER, types::I64),
                    };
                    let m = builder.use_var(Variable::from_u32(args[0].0));
                    let k = builder.use_var(Variable::from_u32(args[1].0));
                    let exp_v = builder.ins().iconst(types::I64, expect as i64);
                    let slot =
                        builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            8,
                            3,
                        ));
                    let out_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                    let helper_ref = module.declare_func_in_func(map_get_helper, builder.func);
                    let call = builder.ins().call(helper_ref, &[m, k, exp_v, out_ptr]);
                    let status = builder.inst_results(call)[0];
                    let ok_block = builder.create_block();
                    builder.ins().brif(status, deopt_block, &[], ok_block, &[]);
                    builder.switch_to_block(ok_block);
                    let val = builder.ins().stack_load(ptr_ty, load_ty, slot, 0);
                    r#gen.write(builder, dst.0, val);
                }
                "map_has_key" => {
                    let Kind::Map(_) = r#gen.kind(args[0].0)? else {
                        return None;
                    };
                    if r#gen.kind(args[1].0)? != Kind::Str {
                        return None;
                    }
                    let m = builder.use_var(Variable::from_u32(args[0].0));
                    let k = builder.use_var(Variable::from_u32(args[1].0));
                    let helper_ref = module.declare_func_in_func(map_has_helper, builder.func);
                    let call = builder.ins().call(helper_ref, &[m, k]);
                    let val = builder.inst_results(call)[0];
                    r#gen.write(builder, dst.0, val);
                }
                "map_set" => {
                    let Kind::Map(_) = r#gen.kind(args[0].0)? else {
                        return None;
                    };
                    if r#gen.kind(args[1].0)? != Kind::Str {
                        return None;
                    }
                    let code: i64 = match r#gen.kind(args[2].0)? {
                        Kind::Int => 0,
                        Kind::Bool => 2,
                        Kind::Str => 4,
                        Kind::Float => 1,
                        _ => return None,
                    };
                    // Raw bits regardless of clif type (F64 or I64).
                    let v = builder.use_var(Variable::from_u32(args[2].0));
                    let slot =
                        builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            8,
                            3,
                        ));
                    builder.ins().stack_store(ptr_ty, v, slot, 0);
                    let bits = builder.ins().stack_load(ptr_ty, types::I64, slot, 0);
                    let ctx = builder.use_var(ctx_var);
                    let m = builder.use_var(Variable::from_u32(args[0].0));
                    let k = builder.use_var(Variable::from_u32(args[1].0));
                    let kind_v = builder.ins().iconst(types::I64, code);
                    let helper_ref = module.declare_func_in_func(map_set_helper, builder.func);
                    let call = builder.ins().call(helper_ref, &[ctx, m, k, kind_v, bits]);
                    let ptr = builder.inst_results(call)[0];
                    let null = builder.ins().icmp_imm_s(IntCC::Equal, ptr, 0);
                    let ok_block = builder.create_block();
                    builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                    builder.switch_to_block(ok_block);
                    r#gen.write(builder, dst.0, ptr);
                }
                _ => return None,
            },
            Instruction::MakeResult { dst, value, ok } => {
                let k = r#gen.kind(value.0)?;
                let code: i64 = match k {
                    Kind::Int => 0,
                    Kind::Float => 1,
                    Kind::Bool => 2,
                    _ => return None,
                };
                // Round-trip through a stack slot so the payload's raw
                // bits travel the same way regardless of clif type.
                let v = builder.use_var(Variable::from_u32(value.0));
                let slot =
                    builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        8,
                        3,
                    ));
                builder.ins().stack_store(ptr_ty, v, slot, 0);
                let bits = builder.ins().stack_load(ptr_ty, types::I64, slot, 0);
                let ctx = builder.use_var(ctx_var);
                let ok_v = builder.ins().iconst(types::I64, *ok as i64);
                let kind_v = builder.ins().iconst(types::I64, code);
                let helper_ref = module.declare_func_in_func(make_result_helper, builder.func);
                let call = builder.ins().call(helper_ref, &[ctx, ok_v, kind_v, bits]);
                let ptr = builder.inst_results(call)[0];
                let null = builder.ins().icmp_imm_s(IntCC::Equal, ptr, 0);
                let ok_block = builder.create_block();
                builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                r#gen.write(builder, dst.0, ptr);
            }
            Instruction::PatternTestResult {
                dst,
                value,
                want_ok,
            } => {
                let Kind::Result(..) = r#gen.kind(value.0)? else {
                    return None;
                };
                let ptr = builder.use_var(Variable::from_u32(value.0));
                let want_v = builder.ins().iconst(types::I64, *want_ok as i64);
                let helper_ref = module.declare_func_in_func(result_test_helper, builder.func);
                let call = builder.ins().call(helper_ref, &[ptr, want_v]);
                let val = builder.inst_results(call)[0];
                r#gen.write(builder, dst.0, val);
            }
            Instruction::ExtractResult {
                dst,
                value,
                want_ok,
            } => {
                let Kind::Result(okp, errp) = r#gen.kind(value.0)? else {
                    return None;
                };
                let (expect, load_ty) = match if *want_ok { okp } else { errp } {
                    Payload::Int => (FIELD_INT, types::I64),
                    Payload::Float => (FIELD_FLOAT, types::F64),
                    Payload::Bool => (FIELD_BOOL, types::I64),
                    Payload::Str => (FIELD_STR, types::I64),
                    // Unreachable side (its test is always false): the
                    // guard below can only deopt, so the loaded value —
                    // typed by the vacuous Int claim — never escapes.
                    Payload::Absent => (FIELD_NEVER, types::I64),
                };
                let ptr = builder.use_var(Variable::from_u32(value.0));
                let want_v = builder.ins().iconst(types::I64, *want_ok as i64);
                let exp_v = builder.ins().iconst(types::I64, expect as i64);
                let slot =
                    builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        8,
                        3,
                    ));
                let out_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                let helper_ref = module.declare_func_in_func(result_extract_helper, builder.func);
                let call = builder
                    .ins()
                    .call(helper_ref, &[ptr, want_v, exp_v, out_ptr]);
                let status = builder.inst_results(call)[0];
                let ok_block = builder.create_block();
                builder.ins().brif(status, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                let val = builder.ins().stack_load(ptr_ty, load_ty, slot, 0);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::IterLen { dst, src } => {
                let list_ptr = builder.use_var(Variable::from_u32(src.0));
                let helper_ref = module.declare_func_in_func(len_helper, builder.func);
                let call = builder.ins().call(helper_ref, &[list_ptr]);
                let val = builder.inst_results(call)[0];
                r#gen.write(builder, dst.0, val);
            }
            Instruction::IterGet { dst, src, idx } => {
                let (expect, expect_shape, load_ty) = match r#gen.kind(src.0)? {
                    Kind::ListFloat => (FIELD_FLOAT, 0, types::F64),
                    Kind::ListInt => (FIELD_INT, 0, types::I64),
                    Kind::ListStruct(sid) => (EXPECT_STRUCT, sid as u64, types::I64),
                    Kind::ListStr => (FIELD_STR, 0, types::I64),
                    _ => return None,
                };
                let list_ptr = builder.use_var(Variable::from_u32(src.0));
                let idx_v = r#gen.read(builder, idx.0)?;
                let exp_v = builder.ins().iconst(types::I64, expect as i64);
                let shape_v = builder.ins().iconst(types::I64, expect_shape as i64);
                let slot =
                    builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        8,
                        3,
                    ));
                let out_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                let helper_ref = module.declare_func_in_func(index_helper, builder.func);
                let call = builder
                    .ins()
                    .call(helper_ref, &[list_ptr, idx_v, exp_v, shape_v, out_ptr]);
                let status = builder.inst_results(call)[0];
                let ok_block = builder.create_block();
                builder.ins().brif(status, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                let val = builder.ins().stack_load(ptr_ty, load_ty, slot, 0);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::IndexGet { dst, object, index } => {
                let (expect, expect_shape, load_ty) = match r#gen.kind(object.0)? {
                    Kind::ListFloat => (FIELD_FLOAT | 0x100, 0, types::F64),
                    Kind::ListInt => (FIELD_INT | 0x100, 0, types::I64),
                    Kind::ListStruct(sid) => (EXPECT_STRUCT | 0x100, sid as u64, types::I64),
                    Kind::ListStr => (FIELD_STR | 0x100, 0, types::I64),
                    _ => return None,
                };
                let list_ptr = builder.use_var(Variable::from_u32(object.0));
                let idx_v = r#gen.read(builder, index.0)?;
                let exp_v = builder.ins().iconst(types::I64, expect as i64);
                let shape_v = builder.ins().iconst(types::I64, expect_shape as i64);
                let slot =
                    builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        8,
                        3,
                    ));
                let out_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                let helper_ref = module.declare_func_in_func(index_helper, builder.func);
                let call = builder
                    .ins()
                    .call(helper_ref, &[list_ptr, idx_v, exp_v, shape_v, out_ptr]);
                let status = builder.inst_results(call)[0];
                let ok_block = builder.create_block();
                builder.ins().brif(status, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                let val = builder.ins().stack_load(ptr_ty, load_ty, slot, 0);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::GetField {
                dst,
                object,
                name_const,
                ..
            } => {
                let Kind::Struct(sid) = r#gen.kind(object.0)? else {
                    return None;
                };
                let spec = shapes.get(&sid)?;
                let name = match &bytecode.constants[*name_const as usize].data {
                    ValueData::String(n) => n.clone(),
                    _ => return None,
                };
                let idx = spec
                    .shape
                    .field_names
                    .iter()
                    .position(|f| f == name.as_str())?;
                let fk = spec.field_kinds[idx];
                let expect = match fk {
                    Kind::Int => FIELD_INT,
                    Kind::Float => FIELD_FLOAT,
                    Kind::Bool => FIELD_BOOL,
                    Kind::Str => FIELD_STR,
                    _ => return None,
                };

                let obj_ptr = builder.use_var(Variable::from_u32(object.0));
                let idx_v = builder.ins().iconst(types::I64, idx as i64);
                let exp_v = builder.ins().iconst(types::I64, expect as i64);
                let slot =
                    builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        8,
                        3,
                    ));
                let out_ptr = builder.ins().stack_addr(types::I64, slot, 0);
                let helper_ref = module.declare_func_in_func(field_helper, builder.func);
                let call = builder
                    .ins()
                    .call(helper_ref, &[obj_ptr, idx_v, exp_v, out_ptr]);
                let status = builder.inst_results(call)[0];
                let ok_block = builder.create_block();
                builder.ins().brif(status, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                let val = builder.ins().stack_load(ptr_ty, fk.clif_type(), slot, 0);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::Jump { target } => {
                let block = blocks[target.0 as usize]?;
                if let Some((h, _)) = inference.scratch_region
                    && target.0 as usize == h
                {
                    let ctx = builder.use_var(ctx_var);
                    if i < h {
                        // A forward entry into the loop: set the mark.
                        let mark_ref = module.declare_func_in_func(mark_helper, builder.func);
                        builder.ins().call(mark_ref, &[ctx]);
                    } else {
                        // The back-edge: this iteration's heap values are
                        // proven dead — free them before going around.
                        let release_ref = module.declare_func_in_func(release_helper, builder.func);
                        builder.ins().call(release_ref, &[ctx]);
                    }
                }
                builder.ins().jump(block, &[]);
                terminated = true;
            }
            Instruction::JumpIfTrue { condition, target } => {
                let cond = r#gen.read(builder, condition.0)?;
                let mut then_block = blocks[target.0 as usize]?;
                let else_block = blocks.get(i + 1).copied().flatten()?;
                if let Some((h, _)) = inference.scratch_region
                    && target.0 as usize == h
                    && i >= h
                {
                    // Bottom-tested loop: release only on the taken
                    // back-edge — the fall-through exits the loop and may
                    // still read this iteration's values.
                    let tramp = builder.create_block();
                    let entry = then_block;
                    builder.ins().brif(cond, tramp, &[], else_block, &[]);
                    builder.switch_to_block(tramp);
                    let ctx = builder.use_var(ctx_var);
                    let release_ref = module.declare_func_in_func(release_helper, builder.func);
                    builder.ins().call(release_ref, &[ctx]);
                    builder.ins().jump(entry, &[]);
                    terminated = true;
                    continue;
                }
                let _ = &mut then_block;
                builder.ins().brif(cond, then_block, &[], else_block, &[]);
                terminated = true;
            }
            Instruction::JumpIfFalse { condition, target } => {
                let cond = r#gen.read(builder, condition.0)?;
                let else_block = blocks[target.0 as usize]?;
                let then_block = blocks.get(i + 1).copied().flatten()?;
                if let Some((h, _)) = inference.scratch_region
                    && target.0 as usize == h
                    && i >= h
                {
                    let tramp = builder.create_block();
                    builder.ins().brif(cond, then_block, &[], tramp, &[]);
                    builder.switch_to_block(tramp);
                    let ctx = builder.use_var(ctx_var);
                    let release_ref = module.declare_func_in_func(release_helper, builder.func);
                    builder.ins().call(release_ref, &[ctx]);
                    builder.ins().jump(else_block, &[]);
                    terminated = true;
                    continue;
                }
                builder.ins().brif(cond, then_block, &[], else_block, &[]);
                terminated = true;
            }
            Instruction::CallFn {
                dst, func_id, args, ..
            } => {
                // A native-to-native call (group member or previously
                // compiled function): spend a unit of depth budget, deopt
                // when exhausted.
                let (callee_clif, _callee_ret, callee_tuple) =
                    targets.get(&func_id.index())?.clone();
                let depth = builder.use_var(depth_var);
                let one = builder.ins().iconst(types::I64, 1);
                let new_depth = builder.ins().isub(depth, one);
                let exhausted =
                    builder
                        .ins()
                        .icmp_imm_s(IntCC::SignedLessThanOrEqual, new_depth, 0);
                let cont = builder.create_block();
                builder.ins().brif(exhausted, deopt_block, &[], cont, &[]);
                builder.switch_to_block(cont);

                let mut call_args = Vec::with_capacity(args.len() + 1);
                for a in args {
                    call_args.push(r#gen.read(builder, a.0)?);
                }
                call_args.push(new_depth);
                call_args.push(builder.use_var(ctx_var));
                let callee_ref = module.declare_func_in_func(callee_clif, builder.func);
                let call = builder.ins().call(callee_ref, &call_args);
                let results = builder.inst_results(call).to_vec();
                let values = &results[..results.len() - 1];
                let status = results[results.len() - 1];

                // A deopt anywhere below unwinds the whole native call.
                let ok_block = builder.create_block();
                builder.ins().brif(status, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                match &callee_tuple {
                    Some(tk) => {
                        if inference.tuples.contains_key(&dst.0) {
                            for (i, v) in values.iter().take(tk.len()).enumerate() {
                                builder.def_var(tuple_var(dst.0, i), *v);
                            }
                        }
                    }
                    None => r#gen.write(builder, dst.0, values[0]),
                }
            }
            Instruction::Return { value } => {
                let reg = (*value)?;
                if r#gen.kind(reg.0) == Some(Kind::Str) && inference.ret_kind == Kind::Str {
                    let ptr = builder.use_var(Variable::from_u32(reg.0));
                    let ok = builder.ins().iconst(types::I64, STATUS_OK);
                    builder.ins().return_(&[ptr, ok]);
                    terminated = true;
                    continue;
                }
                if matches!(r#gen.kind(reg.0), Some(Kind::Struct(_)))
                    && inference.ret_struct.is_some()
                {
                    // Raw pointer out; the scratch list keeps it alive.
                    // Ownership transfers once, in the entry wrapper.
                    let ptr = builder.use_var(Variable::from_u32(reg.0));
                    let ok = builder.ins().iconst(types::I64, STATUS_OK);
                    builder.ins().return_(&[ptr, ok]);
                    terminated = true;
                    continue;
                }
                if matches!(r#gen.kind(reg.0), Some(Kind::Result(..)))
                    && matches!(inference.ret_kind, Kind::Result(..))
                {
                    let ptr = builder.use_var(Variable::from_u32(reg.0));
                    let ok = builder.ins().iconst(types::I64, STATUS_OK);
                    builder.ins().return_(&[ptr, ok]);
                    terminated = true;
                    continue;
                }
                if matches!(
                    r#gen.kind(reg.0),
                    Some(Kind::ListInt | Kind::ListFloat | Kind::ListStruct(_) | Kind::ListStr)
                ) && matches!(
                    inference.ret_kind,
                    Kind::ListInt | Kind::ListFloat | Kind::ListStruct(_) | Kind::ListStr
                ) {
                    let ptr = builder.use_var(Variable::from_u32(reg.0));
                    let ok = builder.ins().iconst(types::I64, STATUS_OK);
                    builder.ins().return_(&[ptr, ok]);
                    terminated = true;
                    continue;
                }
                if let Some(tk) = inference.tuples.get(&reg.0) {
                    let mut vals: Vec<ClifValue> = (0..tk.len())
                        .map(|i| builder.use_var(tuple_var(reg.0, i)))
                        .collect();
                    vals.push(builder.ins().iconst(types::I64, STATUS_OK));
                    builder.ins().return_(&vals);
                } else {
                    let val = r#gen.read(builder, reg.0)?;
                    let ok = builder.ins().iconst(types::I64, STATUS_OK);
                    builder.ins().return_(&[val, ok]);
                }
                terminated = true;
            }
            _ => return None,
        }
    }

    if !terminated {
        // Bytecode always ends in Return; be safe anyway.
        builder.ins().jump(deopt_block, &[]);
    }

    builder.switch_to_block(deopt_block);
    let mut vals: Vec<ClifValue> = Vec::new();
    match &inference.ret_tuple {
        Some(tk) => {
            for k in tk {
                vals.push(match k {
                    Kind::Float => builder.ins().f64const(0.0),
                    _ => builder.ins().iconst(types::I64, 0),
                });
            }
        }
        None => vals.push(match inference.ret_kind {
            Kind::Float => builder.ins().f64const(0.0),
            _ => builder.ins().iconst(types::I64, 0),
        }),
    }
    vals.push(builder.ins().iconst(types::I64, 1));
    builder.ins().return_(&vals);

    builder.seal_all_blocks();
    Some(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

fn arith_op(inst: &Instruction) -> Arith {
    match inst {
        Instruction::Add { .. } => Arith::Add,
        Instruction::Sub { .. } => Arith::Sub,
        Instruction::Mul { .. } => Arith::Mul,
        Instruction::Div { .. } => Arith::Div,
        Instruction::Mod { .. } => Arith::Mod,
        _ => unreachable!("arith_op on non-arith instruction"),
    }
}

fn imm_arith(op: &BinaryOp) -> Option<Arith> {
    Some(match op {
        BinaryOp::Add => Arith::Add,
        BinaryOp::Subtract => Arith::Sub,
        BinaryOp::Multiply => Arith::Mul,
        BinaryOp::Divide => Arith::Div,
        BinaryOp::Modulo => Arith::Mod,
        _ => return None,
    })
}

fn compare_cc(inst: &Instruction) -> IntCC {
    match inst {
        Instruction::Eq { .. } => IntCC::Equal,
        Instruction::Ne { .. } => IntCC::NotEqual,
        Instruction::Lt { .. } => IntCC::SignedLessThan,
        Instruction::Le { .. } => IntCC::SignedLessThanOrEqual,
        Instruction::Gt { .. } => IntCC::SignedGreaterThan,
        Instruction::Ge { .. } => IntCC::SignedGreaterThanOrEqual,
        _ => unreachable!("compare_cc on non-compare instruction"),
    }
}

fn float_cc(inst: &Instruction) -> FloatCC {
    match inst {
        Instruction::Eq { .. } => FloatCC::Equal,
        Instruction::Ne { .. } => FloatCC::NotEqual,
        Instruction::Lt { .. } => FloatCC::LessThan,
        Instruction::Le { .. } => FloatCC::LessThanOrEqual,
        Instruction::Gt { .. } => FloatCC::GreaterThan,
        Instruction::Ge { .. } => FloatCC::GreaterThanOrEqual,
        _ => unreachable!("float_cc on non-compare instruction"),
    }
}

fn imm_compare(op: &BinaryOp) -> Option<IntCC> {
    Some(match op {
        BinaryOp::Equal => IntCC::Equal,
        BinaryOp::NotEqual => IntCC::NotEqual,
        BinaryOp::LessThan => IntCC::SignedLessThan,
        BinaryOp::LessThanEqual => IntCC::SignedLessThanOrEqual,
        BinaryOp::GreaterThan => IntCC::SignedGreaterThan,
        BinaryOp::GreaterThanEqual => IntCC::SignedGreaterThanOrEqual,
        _ => return None,
    })
}

fn imm_float_cc(op: &BinaryOp) -> Option<FloatCC> {
    Some(match op {
        BinaryOp::Equal => FloatCC::Equal,
        BinaryOp::NotEqual => FloatCC::NotEqual,
        BinaryOp::LessThan => FloatCC::LessThan,
        BinaryOp::LessThanEqual => FloatCC::LessThanOrEqual,
        BinaryOp::GreaterThan => FloatCC::GreaterThan,
        BinaryOp::GreaterThanEqual => FloatCC::GreaterThanOrEqual,
        _ => return None,
    })
}

/// Arithmetic matching the VM's semantics exactly. Integer paths carry
/// overflow/zero guards that branch to deopt; float add/sub/mul are plain
/// IEEE; float division guards b == 0.0 (olang errors there). Float
/// modulo itself is refused — Rust's `%` is fmod, which has no exact IR
/// equivalent, and guessing is how divergence starts.
fn emit_arith(
    builder: &mut FunctionBuilder,
    r#gen: &Gen,
    op: Arith,
    a: ClifValue,
    ak: Kind,
    b: ClifValue,
    bk: Kind,
) -> Option<ClifValue> {
    if ak == Kind::Bool || bk == Kind::Bool {
        return None;
    }
    let float = ak == Kind::Float || bk == Kind::Float;
    if float {
        if op == Arith::Mod {
            return None;
        }
        let fa = r#gen.to_float(builder, a, ak);
        let fb = r#gen.to_float(builder, b, bk);
        let val = match op {
            Arith::Add => builder.ins().fadd(fa, fb),
            Arith::Sub => builder.ins().fsub(fa, fb),
            Arith::Mul => builder.ins().fmul(fa, fb),
            Arith::Div => {
                // olang: float division by zero is an error, not inf.
                let zero = builder.ins().f64const(0.0);
                let is_zero = builder.ins().fcmp(FloatCC::Equal, fb, zero);
                let cont = builder.create_block();
                builder
                    .ins()
                    .brif(is_zero, r#gen.deopt_block, &[], cont, &[]);
                builder.switch_to_block(cont);
                builder.ins().fdiv(fa, fb)
            }
            Arith::Mod => unreachable!(),
        };
        return Some(val);
    }

    let deopt = r#gen.deopt_block;
    let val = match op {
        Arith::Add => {
            let r = builder.ins().iadd(a, b);
            // signed overflow iff sign(a)==sign(b) and sign(r)!=sign(a):
            // ((a ^ r) & (b ^ r)) < 0
            let ax = builder.ins().bxor(a, r);
            let bx = builder.ins().bxor(b, r);
            let both = builder.ins().band(ax, bx);
            let overflow = builder.ins().icmp_imm_s(IntCC::SignedLessThan, both, 0);
            let cont = builder.create_block();
            builder.ins().brif(overflow, deopt, &[], cont, &[]);
            builder.switch_to_block(cont);
            r
        }
        Arith::Sub => {
            let r = builder.ins().isub(a, b);
            // overflow iff sign(a)!=sign(b) and sign(r)!=sign(a):
            // ((a ^ b) & (a ^ r)) < 0
            let ab = builder.ins().bxor(a, b);
            let ar = builder.ins().bxor(a, r);
            let both = builder.ins().band(ab, ar);
            let overflow = builder.ins().icmp_imm_s(IntCC::SignedLessThan, both, 0);
            let cont = builder.create_block();
            builder.ins().brif(overflow, deopt, &[], cont, &[]);
            builder.switch_to_block(cont);
            r
        }
        Arith::Mul => {
            let r = builder.ins().imul(a, b);
            // overflow iff the high 64 bits disagree with the sign
            // extension of the low 64: smulhi(a,b) != r >> 63
            let hi = builder.ins().smulhi(a, b);
            let sign = builder.ins().sshr_imm_s(r, 63);
            let overflow = builder.ins().icmp(IntCC::NotEqual, hi, sign);
            let cont = builder.create_block();
            builder.ins().brif(overflow, deopt, &[], cont, &[]);
            builder.switch_to_block(cont);
            r
        }
        Arith::Div | Arith::Mod => {
            // b == 0 and (a == i64::MIN && b == -1) both deopt (the VM's
            // zero-division and checked-overflow errors respectively).
            let zero_div = builder.ins().icmp_imm_s(IntCC::Equal, b, 0);
            let cont1 = builder.create_block();
            builder.ins().brif(zero_div, deopt, &[], cont1, &[]);
            builder.switch_to_block(cont1);

            let min = builder.ins().iconst(types::I64, i64::MIN);
            let a_min = builder.ins().icmp(IntCC::Equal, a, min);
            let b_neg1 = builder.ins().icmp_imm_s(IntCC::Equal, b, -1);
            let both = builder.ins().band(a_min, b_neg1);
            let cont2 = builder.create_block();
            builder.ins().brif(both, deopt, &[], cont2, &[]);
            builder.switch_to_block(cont2);

            match op {
                Arith::Div => builder.ins().sdiv(a, b),
                _ => builder.ins().srem(a, b),
            }
        }
    };
    Some(val)
}

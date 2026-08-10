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
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Type, Value as ClifValue, types};
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
}

/// How the VM hands the JIT other functions' bytecode when planning a
/// call graph.
pub type BytecodeLookup<'a> = dyn Fn(FunctionId) -> Option<Arc<CompiledBytecode>> + 'a;

/// One callable's signature in the group-inference snapshot: parameter
/// kinds, current return mask, and element kinds when it returns a tuple.
type SigSnapshot = HashMap<usize, (Vec<Kind>, u8, Option<Vec<Kind>>, Option<u32>)>;

/// A struct shape observed at the entry: the interned shape plus the
/// field kinds of the argument instance the specialization keys on.
/// Per-read helper guards keep same-shape/different-kind instances safe.
#[derive(Clone)]
pub struct ShapeSpec {
    pub shape: Arc<crate::ovm::value::StructShape>,
    pub field_kinds: Vec<Kind>,
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
        _ => return None,
    };
    for v in items.iter().skip(1) {
        let ok = match (&v.data, want) {
            (ValueData::Float(_), Kind::ListFloat) => true,
            (ValueData::Integer(_), Kind::ListInt) => true,
            (ValueData::Struct(s), Kind::ListStruct(sid)) => s.shape.id == sid,
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
    /// A string argument or field, passed as a borrowed pointer to the
    /// String behind its Arc. Concat allocates through the scratch
    /// context under the same straight-line discipline as structs.
    Str,
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
    pub compiled: u64,
    pub native_calls: u64,
}

// SAFETY: the module's executable memory is immutable after
// finalize_definitions; mutation only happens through &mut self on the
// owning (single-threaded) VM. Function pointers are plain code addresses.
unsafe impl Send for JitCache {}

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

    /// True when a native run is possible or still decidable — callers
    /// use it to decide whether extracting argument values is worth it.
    #[inline]
    pub fn has(&self, func_id: FunctionId) -> bool {
        matches!(
            self.table.get(func_id.index()),
            Some(Some(Slot::Pending)) | Some(Some(Slot::Ready(_)))
        )
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
                }
                ValueData::List(items) => {
                    kinds[i] = classify_list(items)?;
                    bits[i] = Arc::as_ptr(items) as i64;
                }
                ValueData::String(s) => {
                    bits[i] = Arc::as_ptr(s) as i64;
                    kinds[i] = Kind::Str;
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
        for arg in args {
            if let ValueData::Struct(obj) = &arg.data {
                struct_args.push(obj.clone());
            }
            if let ValueData::String(s) = &arg.data {
                str_args.push(s.clone());
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
        )
    }

    /// True when the function's first native call hasn't happened yet —
    /// callers use it to know whether shape specs must be gathered.
    #[inline]
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
        let mut ctx = ScratchCtx::default();
        for arg in args_for_ctx {
            ctx.args.push(arg.clone());
        }
        for s in str_args_for_ctx {
            ctx.str_args.push(s.clone());
        }
        let status = unsafe {
            (jitted.entry)(
                bits.as_ptr(),
                remaining_depth as i64,
                &mut ctx as *mut ScratchCtx,
                out.as_mut_ptr(),
            )
        };
        if status != STATUS_OK {
            return None;
        }
        self.native_calls += 1;
        if let Some(tk) = &jitted.ret_tuple {
            let elems: Vec<OvmValue> = tk
                .iter()
                .zip(out.iter())
                .map(|(k, bits)| match k {
                    Kind::Int => OvmValue::new_integer(*bits),
                    Kind::Bool => OvmValue::new_boolean(*bits != 0),
                    _ => OvmValue::new_float(f64::from_bits(*bits as u64)),
                })
                .collect();
            return Some(OvmValue::new_tuple(elems));
        }
        if let Kind::Struct(_) = jitted.ret_kind {
            let arc = ctx.retained.take()?;
            return Some(OvmValue::new_struct(arc));
        }
        if let Kind::Str = jitted.ret_kind {
            let arc = ctx.retained_str.take()?;
            return Some(OvmValue {
                data: ValueData::String(arc),
            });
        }
        let out = out[0];
        Some(match jitted.ret_kind {
            Kind::Int => OvmValue::new_integer(out),
            Kind::Bool => OvmValue::new_boolean(out != 0),
            Kind::Float => OvmValue::new_float(f64::from_bits(out as u64)),
            Kind::Struct(_) | Kind::Str => unreachable!("handled above"),
            Kind::ListFloat | Kind::ListInt | Kind::ListStruct(_) => {
                unreachable!("list returns are refused by inference")
            }
        })
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
                        ),
                    );
                }
            }

            let mut requests: Vec<(FunctionId, Vec<Kind>)> = Vec::new();
            for plan in plans.iter_mut() {
                if plan
                    .infer_pass(&sigs, &group_shapes, &mut requests, &mut changed)
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
                plans.push(PlanFn::new(fid, bytecode, kinds));
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
                Some(inf) => inferences.push(inf),
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
                    },
                )
                .is_none()
                {
                    if jit_debug() {
                        eprintln!("[jit] fn#{} translate_body failed", plan.func_id.index());
                    }
                    return None;
                }
                builder.finalize();
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
                        MemFlags::trusted(),
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
                        .store(MemFlags::trusted(), *v, out_ptr, (slot_i * 8) as i32);
                }
                let status = results[n_vals];
                if matches!(inf.ret_kind, Kind::Struct(_) | Kind::Str) {
                    // Ownership boundary: resolve the pointer to an owned
                    // Arc in ctx.retained; unknown pointers deopt.
                    let ok_block = builder.create_block();
                    let fail_block = builder.create_block();
                    let done_block = builder.create_block();
                    builder.append_block_param(done_block, types::I64);
                    builder.ins().brif(status, fail_block, &[], ok_block, &[]);

                    builder.switch_to_block(ok_block);
                    let which = if matches!(inf.ret_kind, Kind::Str) {
                        str_retain_helper
                    } else {
                        retain_helper
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
                builder.finalize();
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
        | Instruction::Add { .. }
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
        | Instruction::GetField { .. }
        | Instruction::IndexGet { .. }
        | Instruction::IterLen { .. }
        | Instruction::IterGet { .. }
        | Instruction::MakeTuple { .. }
        | Instruction::PatternTestTuple { .. }
        | Instruction::ExtractElement { .. }
        | Instruction::TupleGet { .. }
        | Instruction::CallFn { .. } => true,
        // Allocation is allowed only in straight-line code (constructors).
        // In a native loop every allocation would live until the call
        // ends — the scratch model's memory cost — and a cap-triggered
        // mid-loop deopt costs more than never compiling. Loops that
        // build structs stay on bytecode and call native constructors.
        Instruction::MakeStruct { field_regs, .. } => {
            field_regs.len() <= 16 && !has_backward_jump(bytecode)
        }
        Instruction::BinImm { imm, .. } => {
            matches!(imm.data, ValueData::Integer(_) | ValueData::Float(_))
        }
        Instruction::CallBuiltin {
            builtin_id, args, ..
        } => matches!(
            crate::ovm::bytecode::BytecodeVm::FLOAT_MATH.get(*builtin_id as usize),
            Some((_, arity)) if args.len() == *arity && *arity <= 2
        ),
        Instruction::Return { value } => value.is_some(),
        _ => false,
    })
}

fn has_backward_jump(bytecode: &CompiledBytecode) -> bool {
    bytecode.instructions.iter().enumerate().any(|(i, inst)| {
        let target = match inst {
            Instruction::Jump { target } => Some(target.0 as usize),
            Instruction::JumpIfTrue { target, .. } | Instruction::JumpIfFalse { target, .. } => {
                Some(target.0 as usize)
            }
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
const K_INT: u8 = 1;
const K_BOOL: u8 = 2;
const K_UNIT: u8 = 4;
const K_FLOAT: u8 = 8;
const K_STRUCT: u8 = 16;
const K_LIST: u8 = 32;
const K_TUPLE: u8 = 64;
const K_STR: u8 = 128;
/// Largest tuple the JIT returns natively (multi-value return slots).
const MAX_TUPLE: usize = 4;
const K_NUM: u8 = K_INT | K_FLOAT;
const K_ANY: u8 = K_INT | K_BOOL | K_UNIT | K_FLOAT | K_STRUCT | K_LIST | K_TUPLE | K_STR;

fn kind_mask(k: Kind) -> u8 {
    match k {
        Kind::Int => K_INT,
        Kind::Bool => K_BOOL,
        Kind::Float => K_FLOAT,
        Kind::Struct(_) => K_STRUCT,
        Kind::ListFloat | Kind::ListInt | Kind::ListStruct(_) => K_LIST,
        Kind::Str => K_STR,
    }
}

fn mask_singleton(mask: u8) -> Option<Kind> {
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
    writes: Vec<u8>,
    allowed: Vec<u8>,
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
    ret_mask: u8,
    eq_pairs: Vec<(u32, u32)>,
    return_regs: Vec<u32>,
}

/// The finalized result codegen consumes.
struct Inference {
    reg_kind: Vec<Option<Kind>>,
    param_kinds: Vec<Kind>,
    ret_kind: Kind,
    tuples: HashMap<u32, Vec<Kind>>,
    ret_tuple: Option<Vec<Kind>>,
    ret_struct: Option<u32>,
}

impl PlanFn {
    fn new(func_id: FunctionId, bytecode: Arc<CompiledBytecode>, param_kinds: Vec<Kind>) -> Self {
        let nregs = bytecode.register_count as usize;
        let mut writes = vec![0u8; nregs];
        let mut exotic = vec![None; nregs];
        for (i, k) in param_kinds.iter().enumerate() {
            writes[i] = kind_mask(*k);
            if matches!(kind_mask(*k), K_STRUCT | K_LIST) {
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
            ret_mask: 0,
            eq_pairs: Vec::new(),
            return_regs: Vec::new(),
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
        requests: &mut Vec<(FunctionId, Vec<Kind>)>,
        global_changed: &mut bool,
    ) -> Option<()> {
        let bytecode = self.bytecode.clone();
        let mut changed = false;

        let const_mask = |idx: u32| -> u8 {
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
                        if self.exotic[dst.0 as usize].is_none() {
                            self.exotic[dst.0 as usize] = Some(k);
                            changed = true;
                        } else if self.exotic[dst.0 as usize] != Some(k) {
                            if jit_debug() {
                                eprintln!("[jit] exotic conflict on Move dst r{}", dst.0);
                            }
                            return None; // one exotic kind per register
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
                        && self.writes[lhs.0 as usize] == K_STR
                        && self.writes[rhs.0 as usize] == K_STR
                    {
                        if has_backward_jump(&self.bytecode) {
                            return None;
                        }
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
                Instruction::BinImm { op, dst, lhs, imm } => {
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
                    let Some(Kind::Struct(sid)) = self.exotic[object.0 as usize] else {
                        if jit_debug() {
                            eprintln!("[jit] GetField on non-struct r{}", object.0);
                        }
                        return None;
                    };
                    let spec = shapes.get(&sid)?;
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
                Instruction::TupleGet { dst, tuple, index } => {
                    narrow!(tuple.0, K_TUPLE);
                    if let Some(tk) = self.tuples.get(&tuple.0) {
                        let k = *tk.get(*index as usize)?;
                        grow!(self.writes[dst.0 as usize], kind_mask(k));
                    }
                }
                Instruction::MakeStruct {
                    dst,
                    shape,
                    field_regs,
                } => {
                    let mut kinds = Vec::with_capacity(field_regs.len());
                    let mut resolved = true;
                    for r in field_regs {
                        narrow!(r.0, K_NUM | K_BOOL);
                        match mask_singleton(self.writes[r.0 as usize]) {
                            Some(k) => kinds.push(k),
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
                        Some(Kind::ListFloat | Kind::ListInt | Kind::ListStruct(_))
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
                Instruction::Jump { .. } | Instruction::MatchFail => {}
                Instruction::JumpIfTrue { condition, .. }
                | Instruction::JumpIfFalse { condition, .. } => {
                    narrow!(condition.0, K_BOOL);
                }
                Instruction::CallFn { dst, func_id, args } => {
                    if let Some((param_kinds, ret_mask, ret_tuple, ret_struct)) =
                        sigs.get(&func_id.index())
                    {
                        if args.len() != param_kinds.len() {
                            return None;
                        }
                        for (i, a) in args.iter().enumerate() {
                            narrow!(a.0, kind_mask(param_kinds[i]));
                            if matches!(kind_mask(param_kinds[i]), K_STRUCT | K_LIST)
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
                        if *ret_mask == K_STR && has_backward_jump(&self.bytecode) {
                            return None; // same discipline as struct returns
                        }
                        if let Some(sid) = ret_struct {
                            // Struct-returning callees allocate into the
                            // ENTRY call's scratch context. Straight-line
                            // callers are bounded (a few allocations per
                            // call); a caller with a loop would accumulate
                            // until the cap and waste the whole run on a
                            // mid-loop deopt — those stay on bytecode and
                            // drive native constructors call by call.
                            if has_backward_jump(&self.bytecode) {
                                return None;
                            }
                            let k = Kind::Struct(*sid);
                            if self.exotic[dst.0 as usize].is_none() {
                                self.exotic[dst.0 as usize] = Some(k);
                                changed = true;
                            } else if self.exotic[dst.0 as usize] != Some(k) {
                                return None;
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
                if matches!(self.writes[r], K_STRUCT | K_LIST) {
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
        } else {
            let k = mask_singleton(self.ret_mask)?;
            for r in &self.return_regs {
                if reg_kind[*r as usize] != Some(k) {
                    return None;
                }
            }
            (k, None)
        };

        Some(Inference {
            reg_kind,
            param_kinds: self.param_kinds.clone(),
            ret_kind,
            tuples: self.tuples.clone(),
            ret_tuple,
            ret_struct: self.ret_struct,
        })
    }
}

fn instruction_name(inst: &Instruction) -> &'static str {
    match inst {
        Instruction::LoadConst { .. } => "LoadConst",
        Instruction::LoadLocal { .. } => "LoadLocal",
        Instruction::StoreLocal { .. } => "StoreLocal",
        Instruction::Move { .. } => "Move",
        Instruction::Add { .. } => "Add",
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
    } = helpers;
    let n = bytecode.instructions.len();
    let param_count = inference.param_kinds.len();

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

    // One typed variable per LIVE register, plus the depth budget.
    let nregs = bytecode.register_count as usize;
    for r in 0..nregs {
        if let Some(k) = r#gen.kind(r as u32) {
            builder.declare_var(Variable::from_u32(r as u32), k.clif_type());
        }
    }
    let depth_var = Variable::from_u32(nregs as u32);
    builder.declare_var(depth_var, types::I64);
    let ctx_var = Variable::from_u32(nregs as u32 + 1);
    builder.declare_var(ctx_var, types::I64);
    // Tuple registers: one variable per element, allocated past the
    // scalar space. Register r's element i lives at
    // tuple_base + r*MAX_TUPLE + i.
    let tuple_base = nregs as u32 + 2;
    let tuple_var =
        |r: u32, i: usize| Variable::from_u32(tuple_base + r * MAX_TUPLE as u32 + i as u32);
    for (r, tk) in &inference.tuples {
        for (i, k) in tk.iter().enumerate() {
            builder.declare_var(tuple_var(*r, i), k.clif_type());
        }
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
                    let null = builder.ins().icmp_imm(IntCC::Equal, ptr, 0);
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
            Instruction::BinImm { op, dst, lhs, imm } => {
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
            Instruction::TupleGet { dst, tuple, index } => {
                let tk = inference.tuples.get(&tuple.0)?;
                if *index as usize >= tk.len() {
                    return None;
                }
                let v = builder.use_var(tuple_var(tuple.0, *index as usize));
                r#gen.write(builder, dst.0, v);
            }
            Instruction::MakeStruct {
                dst,
                shape,
                field_regs,
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
                    builder.ins().stack_store(v, slot, (i * 8) as i32);
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
                let null = builder.ins().icmp_imm(IntCC::Equal, ptr, 0);
                let ok_block = builder.create_block();
                builder.ins().brif(null, deopt_block, &[], ok_block, &[]);
                builder.switch_to_block(ok_block);
                r#gen.write(builder, dst.0, ptr);
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
                let val = builder.ins().stack_load(load_ty, slot, 0);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::IndexGet { dst, object, index } => {
                let (expect, expect_shape, load_ty) = match r#gen.kind(object.0)? {
                    Kind::ListFloat => (FIELD_FLOAT | 0x100, 0, types::F64),
                    Kind::ListInt => (FIELD_INT | 0x100, 0, types::I64),
                    Kind::ListStruct(sid) => (EXPECT_STRUCT | 0x100, sid as u64, types::I64),
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
                let val = builder.ins().stack_load(load_ty, slot, 0);
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
                let val = builder.ins().stack_load(fk.clif_type(), slot, 0);
                r#gen.write(builder, dst.0, val);
            }
            Instruction::Jump { target } => {
                let block = blocks[target.0 as usize]?;
                builder.ins().jump(block, &[]);
                terminated = true;
            }
            Instruction::JumpIfTrue { condition, target } => {
                let cond = r#gen.read(builder, condition.0)?;
                let then_block = blocks[target.0 as usize]?;
                let else_block = blocks.get(i + 1).copied().flatten()?;
                builder.ins().brif(cond, then_block, &[], else_block, &[]);
                terminated = true;
            }
            Instruction::JumpIfFalse { condition, target } => {
                let cond = r#gen.read(builder, condition.0)?;
                let else_block = blocks[target.0 as usize]?;
                let then_block = blocks.get(i + 1).copied().flatten()?;
                builder.ins().brif(cond, then_block, &[], else_block, &[]);
                terminated = true;
            }
            Instruction::CallFn { dst, func_id, args } => {
                // A native-to-native call (group member or previously
                // compiled function): spend a unit of depth budget, deopt
                // when exhausted.
                let (callee_clif, _callee_ret, callee_tuple) =
                    targets.get(&func_id.index())?.clone();
                let depth = builder.use_var(depth_var);
                let one = builder.ins().iconst(types::I64, 1);
                let new_depth = builder.ins().isub(depth, one);
                let exhausted = builder
                    .ins()
                    .icmp_imm(IntCC::SignedLessThanOrEqual, new_depth, 0);
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
            let overflow = builder.ins().icmp_imm(IntCC::SignedLessThan, both, 0);
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
            let overflow = builder.ins().icmp_imm(IntCC::SignedLessThan, both, 0);
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
            let sign = builder.ins().sshr_imm(r, 63);
            let overflow = builder.ins().icmp(IntCC::NotEqual, hi, sign);
            let cont = builder.create_block();
            builder.ins().brif(overflow, deopt, &[], cont, &[]);
            builder.switch_to_block(cont);
            r
        }
        Arith::Div | Arith::Mod => {
            // b == 0 and (a == i64::MIN && b == -1) both deopt (the VM's
            // zero-division and checked-overflow errors respectively).
            let zero_div = builder.ins().icmp_imm(IntCC::Equal, b, 0);
            let cont1 = builder.create_block();
            builder.ins().brif(zero_div, deopt, &[], cont1, &[]);
            builder.switch_to_block(cont1);

            let min = builder.ins().iconst(types::I64, i64::MIN);
            let a_min = builder.ins().icmp(IntCC::Equal, a, min);
            let b_neg1 = builder.ins().icmp_imm(IntCC::Equal, b, -1);
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

//! OVM Unified Value System
//!
//! Provides a unified value representation that works across all execution tiers
//! with integrated garbage collection, lazy evaluation, and optimization metadata.

use std::collections::HashMap;
use std::fmt;

use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use crate::ast::{Expr, Value};

/// Unified value representation for the OVM
#[derive(Debug)]
#[repr(C)]
pub struct OvmValue {
    /// GC and execution metadata
    pub header: ValueHeader,

    /// Actual value data
    pub data: ValueData,
}

/// Value metadata. Kept deliberately small and Copy: it is cloned on every
/// register read in the bytecode dispatch loop.
///
/// The mark bits, refcount, age, and force counters of the old tracing-GC
/// header are gone — Arc payloads handle reclamation, and the atomics made
/// every value clone measurably more expensive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct ValueHeader {
    /// Value type tag for fast type checking
    pub type_tag: TypeTag,

    /// Current execution tier for this value
    pub tier: ExecutionTier,

    /// Lazy evaluation state
    pub lazy_state: LazyState,
}

/// Type tags for fast runtime type checking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TypeTag {
    // Immediate types (no heap allocation)
    Integer = 0,
    Float = 1,
    Boolean = 2,
    Unit = 3,

    // GC-managed types
    String = 10,
    List = 11,
    Tuple = 12,
    Function = 13,
    Struct = 14,
    Range = 15,

    // Special types
    Builtin = 20,
    Promise = 21,

    // Lazy types
    Thunk = 30,
    Stream = 31,
    LazyList = 32,

    // Compiled representations
    CompiledFunction = 40,
    OptimizedValue = 41,

    // Error types
    Error = 50,
    Result = 51,

    // Values owned by OVM modules (ods arrays, ...)
    Native = 60,
}

/// Execution tier for values
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ExecutionTier {
    /// Interpreted execution
    Interpreter = 0,

    /// Bytecode VM execution
    Bytecode = 1,

    /// JIT-compiled native code
    Native = 2,

    /// Mixed execution (e.g., lazy boundaries)
    Hybrid = 3,
}

/// Lazy evaluation state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LazyState {
    /// Value is computed and available
    Eager = 0,

    /// Value is lazy and not yet computed
    Lazy = 1,

    /// Value is currently being computed
    Forcing = 2,

    /// Value was lazy but is now cached
    Cached = 3,

    /// Value represents a stream/sequence
    Stream = 4,
}

/// Value data variants
#[derive(Debug)]
pub enum ValueData {
    // Immediate values (stored inline, no GC needed)
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Unit,

    // Heap values, reference-counted. Arc replaces the old raw-pointer
    // GcPtr scheme, which leaked every allocation (Box::into_raw with no
    // dealloc anywhere) and required unsafe derefs at every use site.
    String(Arc<String>),
    List(Arc<Vec<OvmValue>>),
    Tuple(Arc<Vec<OvmValue>>),
    Function(Arc<FunctionObject>),
    /// An interpreter function held verbatim, so it converts back losslessly.
    /// Used for non-capturing lambdas the bytecode tier builds as constants
    /// and hands to builtins; FunctionObject drops parameter metadata and
    /// rewrites the closure, so it cannot round-trip.
    AstFunction(Arc<crate::ast::Function>),
    /// A map value, mirroring the interpreter's `Value::Map` exactly
    /// (string keys, arbitrary values, shared via Arc). Maps used to be
    /// crushed into a struct shape that could not convert back, which
    /// kept every map-touching function and every map-returning builtin
    /// off the tier.
    Map(Arc<HashMap<String, OvmValue>>),
    /// A proper enum value: type, variant, and payload, converting to and
    /// from the interpreter's `Value::Enum` losslessly. (Enums used to be
    /// crushed into a struct shape with a `__variant` field, which could
    /// not convert back — so no enum ever crossed the tier boundary.)
    Enum(Arc<EnumObject>),
    /// A lambda over *runtime* captures, built by the MakeClosure
    /// instruction: the template carries parameters, body, and the
    /// declaration-time closure part; `captured` holds the values
    /// snapshotted from the enclosing frame when the lambda expression
    /// evaluated — exactly the interpreter's capture-by-value moment.
    Closure(Arc<ClosureObject>),
    Struct(Arc<StructObject>),
    Range(Arc<RangeObject>),
    Builtin(Arc<BuiltinObject>),

    // Lazy values
    Thunk(Arc<ThunkObject>),
    Stream(Arc<StreamObject>),
    LazyList(Arc<LazyListObject>),

    // Async values
    Promise(Arc<PromiseObject>),

    // Compiled representations
    CompiledFunction(Arc<CompiledFunctionObject>),
    OptimizedValue(Arc<OptimizedValueObject>),

    // Error handling
    Error(Arc<ErrorObject>),
    Result {
        ok: Option<Box<OvmValue>>,
        err: Option<Box<OvmValue>>,
    },

    /// A value owned by an OVM module: the *same* Arc the interpreter's
    /// `Value::Native` holds, so the tier boundary is a refcount bump and
    /// the conversion is lossless by construction.
    Native(crate::native::NativeHandle),
}

/// Function object representation
#[derive(Debug)]
pub struct FunctionObject {
    pub name: Option<String>,
    pub parameters: Vec<String>,
    pub body: Expr,
    pub closure: HashMap<String, OvmValue>,
    pub compilation_tier: ExecutionTier,
    pub call_count: AtomicU32,
    pub optimization_data: OptimizationData,
}

/// An enum value in the OVM: mirrors `Value::Enum` exactly.
#[derive(Debug)]
pub struct EnumObject {
    pub type_name: String,
    pub variant_name: String,
    pub data: EnumData,
}

#[derive(Debug)]
pub enum EnumData {
    Unit,
    Tuple(Vec<OvmValue>),
    /// Struct-variant fields. Order preserved from the source value; the
    /// interpreter's positional pattern matching sorts by name, which the
    /// VM mirrors at match time, not here.
    Struct(Vec<(String, OvmValue)>),
}

/// A compiled lambda plus its runtime captures. Convertible back to an
/// interpreter `Function` (template closure + captured entries) so it can
/// cross the tier boundary losslessly, and executable natively via
/// `func_id`, whose bytecode takes the captures as hidden trailing
/// parameters: call with `[args..., captured...]`.
#[derive(Debug)]
pub struct ClosureObject {
    pub template: crate::ast::Function,
    /// Names of the runtime captures, parallel to `captured`.
    pub capture_names: Vec<String>,
    pub captured: Vec<OvmValue>,
    pub func_id: crate::ovm::FunctionId,
}

/// A struct's layout, interned globally: one shape per (type name, field
/// set), field names in sorted order, with a stable id. Two structs of the
/// same type share the same `Arc<StructShape>`, which is what lets a
/// GetField site cache "shape id → field index" and turn a repeat read
/// into an integer compare plus an array index — no hashing.
#[derive(Debug)]
pub struct StructShape {
    pub id: u32,
    pub type_name: String,
    /// Sorted — the canonical order `values` is stored in.
    pub field_names: Vec<String>,
    index: HashMap<String, u32, FnvBuildHasher>,
}

impl StructShape {
    #[inline]
    pub fn field_index(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }
}

impl PartialEq for StructShape {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// Intern a shape. Shapes live for the process; ids start at 1 so 0 can
/// mean "cold" in inline caches.
pub fn intern_shape(type_name: &str, mut field_names: Vec<String>) -> Arc<StructShape> {
    use std::sync::{Mutex, OnceLock};
    type Interner = Mutex<HashMap<(String, Vec<String>), Arc<StructShape>>>;
    static SHAPES: OnceLock<Interner> = OnceLock::new();
    static NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

    field_names.sort();
    let mut shapes = SHAPES.get_or_init(Default::default).lock().unwrap();
    let key = (type_name.to_string(), field_names);
    if let Some(shape) = shapes.get(&key) {
        return shape.clone();
    }
    let index = key
        .1
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), i as u32))
        .collect();
    let shape = Arc::new(StructShape {
        id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        type_name: key.0.clone(),
        field_names: key.1.clone(),
        index,
    });
    shapes.insert(key, shape.clone());
    shape
}

/// Struct object: an interned shape plus values in the shape's field order.
#[derive(Debug)]
pub struct StructObject {
    pub shape: Arc<StructShape>,
    pub values: Vec<OvmValue>,
}

impl StructObject {
    /// Build from unordered (name, value) pairs, interning the shape.
    pub fn from_pairs(type_name: &str, pairs: Vec<(String, OvmValue)>) -> Self {
        let shape = intern_shape(type_name, pairs.iter().map(|(n, _)| n.clone()).collect());
        let mut values: Vec<Option<OvmValue>> =
            (0..shape.field_names.len()).map(|_| None).collect();
        for (name, value) in pairs {
            if let Some(i) = shape.field_index(&name) {
                values[i as usize] = Some(value);
            }
        }
        let values = values
            .into_iter()
            .map(|v| v.unwrap_or_else(OvmValue::new_unit))
            .collect();
        Self { shape, values }
    }

    #[inline]
    pub fn field(&self, name: &str) -> Option<&OvmValue> {
        self.shape
            .field_index(name)
            .map(|i| &self.values[i as usize])
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &OvmValue)> {
        self.shape.field_names.iter().zip(self.values.iter())
    }

    pub fn type_name(&self) -> &str {
        &self.shape.type_name
    }
}

/// FNV-1a for struct field maps. Field lookups sit on the dispatch loop's
/// hot path (every GetField hashes the field name), the maps are tiny, and
/// the keys are short identifiers from source text - SipHash's per-lookup
/// cost is the wrong trade here. Not DoS-hardened; field names come from
/// program text, not external input.
pub struct FnvHasher(u64);

impl std::hash::Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut h = self.0;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = h;
    }
}

#[derive(Clone, Default)]
pub struct FnvBuildHasher;

impl std::hash::BuildHasher for FnvBuildHasher {
    type Hasher = FnvHasher;
    fn build_hasher(&self) -> FnvHasher {
        FnvHasher(0xcbf2_9ce4_8422_2325)
    }
}

pub type FieldMap = HashMap<String, OvmValue, FnvBuildHasher>;

/// Range object for OVM execution with proper iteration support
#[derive(Debug)]
pub struct RangeObject {
    pub start: i64,
    pub end: i64,
    pub inclusive: bool,
    pub current_position: Option<i64>, // For lazy iteration
}

impl RangeObject {
    /// Create a new range object
    pub fn new(start: i64, end: i64, inclusive: bool) -> Self {
        Self {
            start,
            end,
            inclusive,
            current_position: None,
        }
    }

    /// Check if the range contains a value
    pub fn contains(&self, value: i64) -> bool {
        if self.inclusive {
            value >= self.start && value <= self.end
        } else {
            value >= self.start && value < self.end
        }
    }

    /// Get the length of the range
    pub fn len(&self) -> usize {
        if self.start > self.end {
            return 0;
        }
        let diff = if self.inclusive {
            (self.end - self.start + 1).max(0)
        } else {
            (self.end - self.start).max(0)
        };
        diff as usize
    }

    /// Check if the range is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Convert to iterator (for list comprehensions, etc.)
    pub fn to_vec(&self) -> Vec<i64> {
        if self.start > self.end {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut current = self.start;

        if self.inclusive {
            while current <= self.end {
                result.push(current);
                current += 1;
            }
        } else {
            while current < self.end {
                result.push(current);
                current += 1;
            }
        }

        result
    }
}

/// Builtin function object for OVM-native execution
#[derive(Debug)]
pub struct BuiltinObject {
    pub name: String,
    pub arity: usize,
    pub function_ptr: fn(&[OvmValue]) -> Result<OvmValue, RuntimeError>,
    pub is_ovm_native: bool,
    pub performance_tier: ExecutionTier,
}

impl BuiltinObject {
    /// Create a new builtin object
    pub fn new(
        name: String,
        arity: usize,
        function_ptr: fn(&[OvmValue]) -> Result<OvmValue, RuntimeError>,
    ) -> Self {
        Self {
            name,
            arity,
            function_ptr,
            is_ovm_native: true,
            performance_tier: ExecutionTier::Interpreter,
        }
    }

    /// Execute the builtin function
    pub fn execute(&self, args: &[OvmValue]) -> Result<OvmValue, RuntimeError> {
        if args.len() != self.arity && self.arity != 0 {
            // 0 arity means variadic (any number of arguments)
            return Err(RuntimeError::Generic {
                message: format!(
                    "Arity mismatch for {}: expected {}, got {}",
                    self.name,
                    self.arity,
                    args.len()
                ),
            });
        }

        (self.function_ptr)(args)
    }
}

/// Thunk object for lazy evaluation
#[derive(Debug)]
pub struct ThunkObject {
    pub expression: Expr,
    pub environment: HashMap<String, OvmValue>,
    pub memoized_value: Mutex<Option<OvmValue>>,
    pub computation_cost: ComputationCost,
    pub dependencies: Vec<Arc<OvmValue>>,
}

/// Stream object for lazy sequences
#[derive(Debug)]
pub struct StreamObject {
    pub generator: Mutex<GeneratorFunction>,
    pub buffer: Mutex<Vec<OvmValue>>,
    pub buffer_position: Mutex<usize>,
    pub is_infinite: bool,
    pub chunk_size: usize,
}

/// Lazy list object
#[derive(Debug)]
pub struct LazyListObject {
    pub source: Box<OvmValue>,
    pub transformation: TransformationChain,
    pub materialized_prefix: Mutex<Vec<OvmValue>>,
    pub materialization_point: Mutex<usize>,
}

/// Promise object for async operations
#[derive(Debug)]
pub struct PromiseObject {
    pub state: PromiseState,
    pub value: Option<Box<OvmValue>>,
    pub error: Option<Box<OvmValue>>,
    pub callbacks: Vec<CallbackFunction>,
}

/// Compiled function object
#[derive(Debug)]
pub struct CompiledFunctionObject {
    pub original_function: Arc<FunctionObject>,
    /// Address of native code (stored as usize so OvmValue stays Send/Sync)
    pub compiled_code: usize,
    pub code_size: usize,
    pub optimization_level: OptimizationLevel,
    pub deoptimization_count: AtomicU32,
    pub gc_map: GcMap,
}

/// Optimized value object for specialized representations
#[derive(Debug)]
pub struct OptimizedValueObject {
    pub original_value: Box<OvmValue>,
    pub optimized_representation: OptimizedRepresentation,
    pub optimization_metadata: OptimizationMetadata,
}

/// Error object
#[derive(Debug, Clone)]
pub struct ErrorObject {
    pub message: String,
    pub error_type: String,
    pub stack_trace: Vec<String>,
}

/// Supporting types and enums

#[derive(Debug, Clone, Copy)]
pub enum PromiseState {
    Pending,
    Resolved,
    Rejected,
}

#[derive(Debug)]
pub enum GeneratorFunction {
    Range {
        start: i64,
        end: i64,
        step: i64,
    },
    Map {
        source: Box<OvmValue>,
        function: Arc<FunctionObject>,
    },
    Filter {
        source: Box<OvmValue>,
        predicate: Arc<FunctionObject>,
    },
    Custom {
        function: Arc<FunctionObject>,
    },
}

#[derive(Debug)]
pub enum TransformationChain {
    Identity,
    Map(Arc<FunctionObject>),
    Filter(Arc<FunctionObject>),
    Chain(Box<TransformationChain>, Box<TransformationChain>),
}

#[derive(Debug)]
pub struct CallbackFunction {
    pub function: Arc<FunctionObject>,
    pub is_error_handler: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum OptimizationLevel {
    None,
    Basic,
    Aggressive,
}

#[derive(Debug)]
pub struct OptimizationData {
    pub inline_cache: Vec<InlineCacheEntry>,
    pub type_feedback: TypeFeedback,
    pub call_site_data: Vec<CallSiteData>,
}

#[derive(Debug)]
pub struct InlineCacheEntry {
    pub call_site_id: u32,
    pub target_function: Arc<FunctionObject>,
    pub hit_count: u32,
}

#[derive(Debug, Clone)]
pub struct TypeFeedback {
    pub observed_types: Vec<TypeTag>,
    pub type_stability: f64,
}

#[derive(Debug, Clone)]
pub struct CallSiteData {
    pub call_count: u32,
    pub average_execution_time: std::time::Duration,
    pub memory_allocations: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum ComputationCost {
    Trivial,
    Light,
    Medium,
    Heavy,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct GcMap {
    pub root_offsets: Vec<usize>,
    pub safepoint_offsets: Vec<usize>,
}

#[derive(Debug)]
pub enum OptimizedRepresentation {
    PackedArray(Vec<i64>),                 // For homogeneous integer arrays
    BitSet(Vec<u64>),                      // For boolean arrays
    SparseArray(HashMap<usize, OvmValue>), // For sparse arrays
    String(String),                        // For optimized string representation
}

#[derive(Debug, Clone)]
pub struct OptimizationMetadata {
    pub optimization_type: OptimizationType,
    pub memory_savings: usize,
    pub access_pattern: AccessPattern,
}

#[derive(Debug, Clone, Copy)]
pub enum OptimizationType {
    MemoryLayout,
    AccessPattern,
    TypeSpecialization,
    ComputationMemoization,
}

#[derive(Debug, Clone)]
pub enum AccessPattern {
    Sequential,
    Random,
    Sparse,
    ReadOnly,
    WriteHeavy,
}

/// Runtime error type
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Type error: expected {expected}, found {found}")]
    TypeError { expected: String, found: String },

    #[error("Index out of bounds: {index} >= {length}")]
    IndexOutOfBounds { index: usize, length: usize },

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Null pointer dereference")]
    NullPointer,

    #[error("Stack overflow")]
    StackOverflow,

    #[error("Out of memory")]
    OutOfMemory,

    #[error("Lazy evaluation error: {message}")]
    LazyError { message: String },

    #[error("Promise error: {message}")]
    PromiseError { message: String },

    #[error("Concurrency error: {0}")]
    ConcurrencyError(String),

    #[error("Runtime error: {message}")]
    Generic { message: String },
}

impl RuntimeError {
    pub fn new(message: &str) -> Self {
        RuntimeError::Generic {
            message: message.to_string(),
        }
    }
}

// Implementation of core methods

impl Clone for OvmValue {
    fn clone(&self) -> Self {
        self.clone_simple()
    }
}

impl PartialEq for OvmValue {
    fn eq(&self, other: &Self) -> bool {
        // Compare type tags first for quick rejection
        if self.header.type_tag as u8 != other.header.type_tag as u8 {
            return false;
        }

        // Compare based on value data
        match (&self.data, &other.data) {
            (ValueData::Integer(a), ValueData::Integer(b)) => a == b,
            (ValueData::Float(a), ValueData::Float(b)) => a == b,
            (ValueData::Boolean(a), ValueData::Boolean(b)) => a == b,
            (ValueData::Unit, ValueData::Unit) => true,

            (ValueData::String(a), ValueData::String(b)) => a == b,

            (ValueData::List(a), ValueData::List(b)) => a == b,

            (ValueData::Tuple(a), ValueData::Tuple(b)) => a == b,

            (ValueData::Range(a), ValueData::Range(b)) => {
                a.start == b.start && a.end == b.end && a.inclusive == b.inclusive
            }

            (
                ValueData::Result {
                    ok: a_ok,
                    err: a_err,
                },
                ValueData::Result {
                    ok: b_ok,
                    err: b_err,
                },
            ) => match (a_ok, a_err, b_ok, b_err) {
                (Some(a_val), None, Some(b_val), None) => a_val.as_ref() == b_val.as_ref(),
                (None, Some(a_val), None, Some(b_val)) => a_val.as_ref() == b_val.as_ref(),
                (None, None, None, None) => true,
                _ => false,
            },

            // For complex types, fall back to pointer comparison for now
            (ValueData::Function(a), ValueData::Function(b)) => Arc::ptr_eq(a, b),
            (ValueData::AstFunction(a), ValueData::AstFunction(b)) => Arc::ptr_eq(a, b),
            (ValueData::Closure(a), ValueData::Closure(b)) => Arc::ptr_eq(a, b),
            (ValueData::Enum(a), ValueData::Enum(b)) => Arc::ptr_eq(a, b),
            (ValueData::Map(a), ValueData::Map(b)) => Arc::ptr_eq(a, b),
            (ValueData::Struct(a), ValueData::Struct(b)) => Arc::ptr_eq(a, b),
            (ValueData::Builtin(a), ValueData::Builtin(b)) => Arc::ptr_eq(a, b),
            (ValueData::Promise(a), ValueData::Promise(b)) => Arc::ptr_eq(a, b),
            (ValueData::Thunk(a), ValueData::Thunk(b)) => Arc::ptr_eq(a, b),
            (ValueData::LazyList(a), ValueData::LazyList(b)) => Arc::ptr_eq(a, b),
            (ValueData::Stream(a), ValueData::Stream(b)) => Arc::ptr_eq(a, b),
            (ValueData::CompiledFunction(a), ValueData::CompiledFunction(b)) => Arc::ptr_eq(a, b),
            (ValueData::OptimizedValue(a), ValueData::OptimizedValue(b)) => Arc::ptr_eq(a, b),
            (ValueData::Error(a), ValueData::Error(b)) => Arc::ptr_eq(a, b),

            // Different types are never equal
            _ => false,
        }
    }
}

impl Eq for OvmValue {}

impl OvmValue {
    /// Create a new integer value
    #[inline]
    pub fn new_integer(value: i64) -> Self {
        Self {
            header: ValueHeader::new(
                TypeTag::Integer,
                ExecutionTier::Interpreter,
                LazyState::Eager,
            ),
            data: ValueData::Integer(value),
        }
    }

    /// Create a new float value
    #[inline]
    pub fn new_float(value: f64) -> Self {
        Self {
            header: ValueHeader::new(TypeTag::Float, ExecutionTier::Interpreter, LazyState::Eager),
            data: ValueData::Float(value),
        }
    }

    /// Create a new boolean value
    #[inline]
    pub fn new_boolean(value: bool) -> Self {
        Self {
            header: ValueHeader::new(
                TypeTag::Boolean,
                ExecutionTier::Interpreter,
                LazyState::Eager,
            ),
            data: ValueData::Boolean(value),
        }
    }

    /// Create a new unit value
    pub fn new_unit() -> Self {
        Self {
            header: ValueHeader::new(TypeTag::Unit, ExecutionTier::Interpreter, LazyState::Eager),
            data: ValueData::Unit,
        }
    }

    /// Clone a value. Immediate values are copied; heap values share their
    /// GcPtr payload (previously they were silently replaced with Unit,
    /// destroying every string/list/function that went through a register
    /// copy or argument pass).
    #[inline]
    pub fn clone_simple(&self) -> Self {
        // Immediates dominate register traffic in a numeric kernel, and
        // copying one is a few bytes. Splitting them out keeps this arm
        // inlinable at the call site; the heap arms stay behind a call,
        // because a 20-way match is far past what LLVM will inline, and
        // leaving them here made *every* register copy a jump into it.
        match &self.data {
            ValueData::Integer(i) => {
                return Self {
                    header: self.header,
                    data: ValueData::Integer(*i),
                }
            }
            ValueData::Float(f) => {
                return Self {
                    header: self.header,
                    data: ValueData::Float(*f),
                }
            }
            ValueData::Boolean(b) => {
                return Self {
                    header: self.header,
                    data: ValueData::Boolean(*b),
                }
            }
            ValueData::Unit => {
                return Self {
                    header: self.header,
                    data: ValueData::Unit,
                }
            }
            _ => {}
        }
        self.clone_heap()
    }

    #[inline(never)]
    fn clone_heap(&self) -> Self {
        let data = match &self.data {
            ValueData::Integer(i) => ValueData::Integer(*i),
            ValueData::Float(f) => ValueData::Float(*f),
            ValueData::Boolean(b) => ValueData::Boolean(*b),
            ValueData::Unit => ValueData::Unit,
            ValueData::String(p) => ValueData::String(p.clone()),
            ValueData::Native(p) => ValueData::Native(p.clone()),
            ValueData::List(p) => ValueData::List(p.clone()),
            ValueData::Tuple(p) => ValueData::Tuple(p.clone()),
            ValueData::Function(p) => ValueData::Function(p.clone()),
            ValueData::AstFunction(p) => ValueData::AstFunction(p.clone()),
            ValueData::Closure(p) => ValueData::Closure(p.clone()),
            ValueData::Enum(p) => ValueData::Enum(p.clone()),
            ValueData::Map(p) => ValueData::Map(p.clone()),
            ValueData::Struct(p) => ValueData::Struct(p.clone()),
            ValueData::Range(p) => ValueData::Range(p.clone()),
            ValueData::Builtin(p) => ValueData::Builtin(p.clone()),
            ValueData::Thunk(p) => ValueData::Thunk(p.clone()),
            ValueData::Stream(p) => ValueData::Stream(p.clone()),
            ValueData::LazyList(p) => ValueData::LazyList(p.clone()),
            ValueData::Promise(p) => ValueData::Promise(p.clone()),
            ValueData::CompiledFunction(p) => ValueData::CompiledFunction(p.clone()),
            ValueData::OptimizedValue(p) => ValueData::OptimizedValue(p.clone()),
            ValueData::Error(p) => ValueData::Error(p.clone()),
            ValueData::Result { ok, err } => ValueData::Result {
                ok: ok.clone(),
                err: err.clone(),
            },
        };
        Self {
            header: self.header,
            data,
        }
    }

    /// Get the type tag of this value
    pub fn type_tag(&self) -> TypeTag {
        self.header.type_tag
    }

    /// Check if this value is lazy
    pub fn is_lazy(&self) -> bool {
        matches!(self.header.lazy_state, LazyState::Lazy | LazyState::Stream)
    }

    /// Check if this value is immediate (no heap allocation)
    pub fn is_immediate(&self) -> bool {
        matches!(
            self.header.type_tag,
            TypeTag::Integer | TypeTag::Float | TypeTag::Boolean | TypeTag::Unit
        )
    }

    /// Force evaluation of lazy values
    pub fn force(&mut self) -> Result<(), RuntimeError> {
        if !self.is_lazy() {
            return Ok(());
        }

        // Handle different lazy value types based on lazy state
        let lazy_state = self.header.lazy_state;
        match lazy_state {
            LazyState::Lazy => {
                // Extract the data temporarily to avoid borrowing issues
                match &self.data {
                    ValueData::Thunk(thunk_ptr) => {
                        let thunk_ptr_copy = thunk_ptr.clone();
                        self.force_thunk_impl(thunk_ptr_copy)?;
                    }
                    ValueData::LazyList(lazy_list_ptr) => {
                        let lazy_list_ptr_copy = lazy_list_ptr.clone();
                        self.force_lazy_list_impl(lazy_list_ptr_copy)?;
                    }
                    _ => {}
                }
            }
            LazyState::Stream => {
                // Streams remain lazy but may buffer more data
                if let ValueData::Stream(stream_ptr) = &self.data {
                    let stream_ptr_copy = stream_ptr.clone();
                    self.advance_stream_buffer_impl(stream_ptr_copy)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Force evaluation of a thunk with memoization
    fn force_thunk_impl(&mut self, thunk_ptr: Arc<ThunkObject>) -> Result<(), RuntimeError> {
        // Prevent infinite recursion
        if self.header.lazy_state == LazyState::Forcing {
            return Err(RuntimeError::new("Circular thunk dependency detected"));
        }

        // Set forcing state
        self.header.lazy_state = LazyState::Forcing;

        // Access the thunk safely
        let thunk_ref = &*thunk_ptr;

        // Check if already memoized (with lock)
        {
            let memoized_guard = thunk_ref.memoized_value.lock().map_err(|_| {
                RuntimeError::ConcurrencyError("Failed to acquire thunk lock".to_string())
            })?;

            if let Some(memoized) = &*memoized_guard {
                // Use memoized value - create a simple copy instead of clone
                match &memoized.data {
                    ValueData::Integer(i) => self.data = ValueData::Integer(*i),
                    ValueData::Float(f) => self.data = ValueData::Float(*f),
                    ValueData::Boolean(b) => self.data = ValueData::Boolean(*b),
                    ValueData::Unit => self.data = ValueData::Unit,
                    _ => self.data = ValueData::Unit, // Fallback for complex types
                }
                self.header.type_tag = memoized.header.type_tag;
                self.header.lazy_state = LazyState::Cached;
                return Ok(());
            }
        }

        // Evaluate the thunk
        let evaluated_value = self.evaluate_thunk_expression(thunk_ref)?;

        // Memoize the result in the thunk (with lock)
        {
            let mut memoized_guard = thunk_ref.memoized_value.lock().map_err(|_| {
                RuntimeError::ConcurrencyError("Failed to acquire thunk lock".to_string())
            })?;
            *memoized_guard = Some(evaluated_value.clone_simple());
        }

        // Update this value with the result
        match evaluated_value.data {
            ValueData::Integer(i) => self.data = ValueData::Integer(i),
            ValueData::Float(f) => self.data = ValueData::Float(f),
            ValueData::Boolean(b) => self.data = ValueData::Boolean(b),
            ValueData::Unit => self.data = ValueData::Unit,
            data => self.data = data, // For other types that can be moved
        }
        self.header.type_tag = evaluated_value.header.type_tag;
        self.header.lazy_state = LazyState::Cached;

        Ok(())
    }

    /// Force evaluation of a lazy list
    fn force_lazy_list_impl(
        &mut self,
        lazy_list_ptr: Arc<LazyListObject>,
    ) -> Result<(), RuntimeError> {
        // For now, materialize a reasonable prefix of the lazy list
        let chunk_size = 100; // Configurable chunk size

        let lazy_list_ref = &*lazy_list_ptr;

        // Use locks to safely access and modify the materialized data
        let mut materialized_guard = lazy_list_ref.materialized_prefix.lock().map_err(|_| {
            RuntimeError::ConcurrencyError("Failed to acquire lazy list lock".to_string())
        })?;
        let mut materialization_point_guard =
            lazy_list_ref.materialization_point.lock().map_err(|_| {
                RuntimeError::ConcurrencyError(
                    "Failed to acquire materialization point lock".to_string(),
                )
            })?;

        // If we haven't materialized anything yet, start materializing
        if materialized_guard.is_empty() {
            match &lazy_list_ref.transformation {
                TransformationChain::Identity => {
                    // Copy from source - simplified implementation
                    if let ValueData::List(source_list) = &lazy_list_ref.source.data {
                        let mut materialized = 0;
                        for item in source_list.iter().take(chunk_size) {
                            materialized_guard.push(item.clone());
                            materialized += 1;
                        }
                        *materialization_point_guard = materialized;
                    }
                }
                TransformationChain::Map(_map_fn) => {
                    // Apply map transformation - simplified for now
                    if let ValueData::List(source_list) = &lazy_list_ref.source.data {
                        let mut materialized = 0;
                        for item in source_list.iter().take(chunk_size) {
                            materialized_guard.push(item.clone());
                            materialized += 1;
                        }
                        *materialization_point_guard = materialized;
                    }
                }
                TransformationChain::Filter(_filter_fn) => {
                    // Apply filter transformation - simplified for now
                    if let ValueData::List(source_list) = &lazy_list_ref.source.data {
                        let mut materialized = 0;
                        for item in source_list.iter().take(chunk_size) {
                            materialized_guard.push(item.clone());
                            materialized += 1;
                        }
                        *materialization_point_guard = materialized;
                    }
                }
                TransformationChain::Chain(_first, _second) => {
                    // Apply chained transformations - simplified for now
                    if let ValueData::List(source_list) = &lazy_list_ref.source.data {
                        let mut materialized = 0;
                        for item in source_list.iter().take(chunk_size) {
                            materialized_guard.push(item.clone());
                            materialized += 1;
                        }
                        *materialization_point_guard = materialized;
                    }
                }
            }
        }

        // Convert the lazy list to a regular list with materialized items
        // For now, create a simplified result
        if !materialized_guard.is_empty() {
            // Take the first item as a representative result (simplified)
            let first_item = materialized_guard[0].clone();
            self.data = first_item.data;
            self.header = first_item.header;
        }
        self.header.type_tag = TypeTag::List;
        self.header.lazy_state = LazyState::Cached;

        Ok(())
    }

    /// Advance stream buffer for better performance
    fn advance_stream_buffer_impl(
        &mut self,
        stream_ptr: Arc<StreamObject>,
    ) -> Result<(), RuntimeError> {
        let stream_ref = &*stream_ptr;

        // Use locks to safely access and modify the stream data
        let mut buffer_guard = stream_ref.buffer.lock().map_err(|_| {
            RuntimeError::ConcurrencyError("Failed to acquire stream buffer lock".to_string())
        })?;
        let position_guard = stream_ref.buffer_position.lock().map_err(|_| {
            RuntimeError::ConcurrencyError("Failed to acquire stream position lock".to_string())
        })?;
        let mut generator_guard = stream_ref.generator.lock().map_err(|_| {
            RuntimeError::ConcurrencyError("Failed to acquire stream generator lock".to_string())
        })?;

        // Buffer more items if buffer is getting low
        let buffer_threshold = stream_ref.chunk_size / 2;
        if buffer_guard.len() - *position_guard < buffer_threshold {
            let items_to_generate = stream_ref.chunk_size;

            for _ in 0..items_to_generate {
                match &mut *generator_guard {
                    GeneratorFunction::Range { start, end, step } => {
                        // Honor the step direction — a descending range
                        // (negative step) never satisfies `start < end`
                        let in_range = if *step >= 0 {
                            *start < *end
                        } else {
                            *start > *end
                        };
                        if in_range {
                            let value = OvmValue::new_integer(*start);
                            buffer_guard.push(value);
                            *start += *step;
                        } else {
                            break; // End of range
                        }
                    }
                    GeneratorFunction::Map {
                        source: _source,
                        function: _function,
                    } => {
                        // Simplified - would need interpreter context for function calls
                        break;
                    }
                    GeneratorFunction::Filter {
                        source: _source,
                        predicate: _predicate,
                    } => {
                        // Simplified - would need interpreter context for predicate calls
                        break;
                    }
                    GeneratorFunction::Custom {
                        function: _function,
                    } => {
                        // Simplified - would need interpreter context for function calls
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Evaluate thunk expression (simplified version)
    fn evaluate_thunk_expression(&self, thunk: &ThunkObject) -> Result<OvmValue, RuntimeError> {
        // For now, return a simple computed value based on the expression
        // In a full implementation, this would use the interpreter with the thunk's environment
        match &thunk.expression {
            Expr::Integer(value) => Ok(OvmValue::new_integer(*value)),
            Expr::Float(value) => Ok(OvmValue::new_float(*value)),
            Expr::Boolean(value) => Ok(OvmValue::new_boolean(*value)),
            Expr::String(value) => Ok(OvmValue::new_string((**value).clone())),
            Expr::BinaryOp { left, op, right } => {
                // Simplified binary operation evaluation
                if let (Expr::Integer(a), Expr::Integer(b)) = (left.as_ref(), right.as_ref()) {
                    match op {
                        crate::ast::BinaryOp::Add => Ok(OvmValue::new_integer(a + b)),
                        crate::ast::BinaryOp::Subtract => Ok(OvmValue::new_integer(a - b)),
                        crate::ast::BinaryOp::Multiply => Ok(OvmValue::new_integer(a * b)),
                        crate::ast::BinaryOp::Divide => {
                            if *b != 0 {
                                Ok(OvmValue::new_integer(a / b))
                            } else {
                                Err(RuntimeError::new("Division by zero"))
                            }
                        }
                        _ => Ok(OvmValue::new_integer(*a)), // Fallback
                    }
                } else {
                    Ok(OvmValue::new_unit())
                }
            }
            _ => Ok(OvmValue::new_unit()), // Fallback for complex expressions
        }
    }

    /// Convert AST value to OVM value (simplified)
    fn _convert_ast_value_to_ovm(&self, _value: Value) -> OvmValue {
        // This method is currently unused but kept for future use
        OvmValue::new_unit()
    }

    /// Create a new string value using simplified GC allocation
    pub fn new_string(value: String) -> Self {
        let gc_ptr = Arc::new(value);

        Self {
            header: ValueHeader::new(
                TypeTag::String,
                ExecutionTier::Interpreter,
                LazyState::Eager,
            ),
            data: ValueData::String(gc_ptr),
        }
    }

    /// Create a new list value
    pub fn new_list(values: Vec<Self>) -> Self {
        let gc_ptr = Arc::new(values);

        Self {
            header: ValueHeader::new(TypeTag::List, ExecutionTier::Interpreter, LazyState::Eager),
            data: ValueData::List(gc_ptr),
        }
    }

    /// Create a builtin function value
    pub fn new_builtin(
        name: String,
        arity: usize,
        function_ptr: fn(&[OvmValue]) -> Result<OvmValue, RuntimeError>,
    ) -> Self {
        let builtin_obj = BuiltinObject::new(name, arity, function_ptr);

        // For now, we'll use a placeholder pointer - in a real implementation,
        // this would be allocated through the GC
        let gc_ptr = Arc::new(builtin_obj);

        Self {
            header: ValueHeader::new(
                TypeTag::Builtin,
                ExecutionTier::Interpreter,
                LazyState::Eager,
            ),
            data: ValueData::Builtin(gc_ptr),
        }
    }

    /// Check if this value is a builtin function
    pub fn is_builtin(&self) -> bool {
        matches!(self.data, ValueData::Builtin(_))
    }

    /// Get builtin function name if this is a builtin
    pub fn get_builtin_name(&self) -> Option<&str> {
        match &self.data {
            ValueData::Builtin(ptr) => Some(&ptr.name),
            _ => None,
        }
    }

    /// Execute builtin function if this value is a builtin
    pub fn execute_builtin(&self, args: &[OvmValue]) -> Result<OvmValue, RuntimeError> {
        match &self.data {
            ValueData::Builtin(ptr) => ptr.execute(args),
            _ => Err(RuntimeError::TypeError {
                expected: "builtin function".to_string(),
                found: format!("{:?}", self.header.type_tag),
            }),
        }
    }

    /// Enhanced from_ast conversion with better builtin support
    pub fn from_ast(ast_value: Value) -> Self {
        match ast_value {
            Value::Integer(n) => Self::new_integer(n),
            Value::Float(f) => Self::new_float(f),
            Value::Boolean(b) => Self::new_boolean(b),
            Value::String(s) => Self::new_string(s.as_ref().clone()),
            Value::Unit => Self::new_unit(),

            Value::List(items) => {
                let ovm_items: Vec<Self> = items
                    .iter()
                    .map(|item| Self::from_ast(item.clone()))
                    .collect();
                Self::new_list(ovm_items)
            }

            Value::Tuple(items) => {
                let ovm_items: Vec<Self> = items
                    .iter()
                    .map(|item| Self::from_ast(item.clone()))
                    .collect();
                Self::new_tuple(ovm_items)
            }

            // Wrapped verbatim: the old conversion deep-copied the whole
            // closure into a FunctionObject that could not convert back, so
            // no function value ever crossed the tier boundary. AstFunction
            // is lossless and O(1).
            Value::Function(func) => Self::new_ast_function(func),

            Value::Builtin(builtin) => {
                // Create a placeholder builtin function
                // In a real implementation, this would map to actual builtin functions
                Self::new_builtin(builtin.name.clone(), builtin.arity, |_args| {
                    Err(RuntimeError::Generic {
                        message: "Builtin function not implemented in OVM".to_string(),
                    })
                })
            }

            Value::Ok(value) => Self {
                header: ValueHeader::new(
                    TypeTag::Result,
                    ExecutionTier::Interpreter,
                    LazyState::Eager,
                ),
                data: ValueData::Result {
                    ok: Some(Box::new(Self::from_ast(*value))),
                    err: None,
                },
            },

            Value::Err(value) => Self {
                header: ValueHeader::new(
                    TypeTag::Result,
                    ExecutionTier::Interpreter,
                    LazyState::Eager,
                ),
                data: ValueData::Result {
                    ok: None,
                    err: Some(Box::new(Self::from_ast(*value))),
                },
            },

            Value::Range {
                start,
                end,
                inclusive,
            } => {
                // Implement proper range representation
                let range_obj = RangeObject {
                    start,
                    end,
                    inclusive,
                    current_position: None, // Will be set during iteration
                };

                let gc_ptr = Arc::new(range_obj);

                Self {
                    header: ValueHeader::new(
                        TypeTag::Range,
                        ExecutionTier::Interpreter,
                        LazyState::Eager,
                    ),
                    data: ValueData::Range(gc_ptr),
                }
            }

            Value::Struct { type_name, fields } => {
                let pairs: Vec<(String, OvmValue)> = fields
                    .into_iter()
                    .map(|(k, v)| (k, Self::from_ast(v)))
                    .collect();
                let struct_obj = StructObject::from_pairs(&type_name, pairs);

                let gc_ptr = Arc::new(struct_obj);

                Self {
                    header: ValueHeader::new(
                        TypeTag::Struct,
                        ExecutionTier::Interpreter,
                        LazyState::Eager,
                    ),
                    data: ValueData::Struct(gc_ptr),
                }
            }

            Value::Enum {
                type_name,
                variant_name,
                variant_data,
            } => {
                let data = match variant_data {
                    crate::ast::EnumVariantData::Unit => EnumData::Unit,
                    crate::ast::EnumVariantData::Tuple(values) => {
                        EnumData::Tuple(values.iter().map(|v| Self::from_ast(v.clone())).collect())
                    }
                    crate::ast::EnumVariantData::Struct(struct_fields) => EnumData::Struct(
                        struct_fields
                            .iter()
                            .map(|(k, v)| (k.clone(), Self::from_ast(v.clone())))
                            .collect(),
                    ),
                };
                Self::new_enum(Arc::new(EnumObject {
                    type_name,
                    variant_name,
                    data,
                }))
            }

            Value::Promise {
                state,
                value,
                error,
                ..
            } => {
                let ovm_state = match state {
                    crate::ast::PromiseState::Pending => PromiseState::Pending,
                    crate::ast::PromiseState::Resolved => PromiseState::Resolved,
                    crate::ast::PromiseState::Rejected => PromiseState::Rejected,
                };

                let promise_obj = PromiseObject {
                    state: ovm_state,
                    value: value.map(|v| Box::new(Self::from_ast(*v))),
                    error: error.map(|e| Box::new(Self::from_ast(*e))),
                    callbacks: Vec::new(),
                };

                let gc_ptr = Arc::new(promise_obj);

                Self {
                    header: ValueHeader::new(
                        TypeTag::Promise,
                        ExecutionTier::Interpreter,
                        LazyState::Eager,
                    ),
                    data: ValueData::Promise(gc_ptr),
                }
            }

            Value::Map(map) => Self::new_map(Arc::new(
                map.iter()
                    .map(|(k, v)| (k.clone(), Self::from_ast(v.clone())))
                    .collect(),
            )),

            Value::TypeInfo { name, .. } => {
                // For now, represent types as string names
                Self::new_string(name)
            }
            // A constructor is a callable; it never actually crosses into the
            // VM (round_trips excludes it), so a placeholder unit is fine
            Value::EnumConstructor { .. } => Self::new_unit(),

            // The same Arc, shared verbatim: crossing the boundary is a
            // refcount bump, never a conversion.
            Value::Native(handle) => Self {
                header: ValueHeader::new(
                    TypeTag::Native,
                    ExecutionTier::Interpreter,
                    LazyState::Eager,
                ),
                data: ValueData::Native(handle),
            },
        }
    }

    /// Create OVM value from AST value with GC integration and safepoint coordination
    pub fn from_ast_with_gc(
        ast_value: Value,
        safepoint_manager: &Arc<crate::ovm::gc::SafepointManager>,
    ) -> Result<Self, RuntimeError> {
        // First check for safepoint before allocation
        safepoint_manager.check_safepoint();

        // Convert from AST using the standard method
        let mut ovm_value = Self::from_ast(ast_value);

        ovm_value.header.tier = ExecutionTier::Interpreter; // Start at interpreter tier

        // Record allocation with safepoint manager
        let allocation_size = std::mem::size_of::<OvmValue>()
            + match &ovm_value.data {
                ValueData::String(s) => s.len(),
                ValueData::List(gc_ptr) => gc_ptr.len() * std::mem::size_of::<OvmValue>(),
                ValueData::Tuple(gc_ptr) => gc_ptr.len() * std::mem::size_of::<OvmValue>(),
                ValueData::Function(_) => std::mem::size_of::<FunctionObject>(),
                ValueData::AstFunction(_) => std::mem::size_of::<crate::ast::Function>(),
                ValueData::Closure(_) => std::mem::size_of::<ClosureObject>(),
                ValueData::Enum(_) => std::mem::size_of::<EnumObject>(),
                ValueData::Map(m) => m.len() * 64,
                ValueData::Struct(_) => std::mem::size_of::<StructObject>(),
                ValueData::Range(_) => std::mem::size_of::<RangeObject>(),
                ValueData::Promise(_) => std::mem::size_of::<PromiseObject>(),
                ValueData::Thunk(_) => std::mem::size_of::<ThunkObject>(),
                ValueData::LazyList(_) => std::mem::size_of::<LazyListObject>(),
                _ => 0,
            };

        safepoint_manager.record_allocation(allocation_size);

        Ok(ovm_value)
    }

    /// Wrap a map value.
    pub fn new_map(map: Arc<HashMap<String, OvmValue>>) -> Self {
        Self {
            header: ValueHeader::new(TypeTag::Struct, ExecutionTier::Bytecode, LazyState::Eager),
            data: ValueData::Map(map),
        }
    }

    /// Wrap an enum value.
    pub fn new_enum(obj: Arc<EnumObject>) -> Self {
        Self {
            header: ValueHeader::new(TypeTag::Struct, ExecutionTier::Bytecode, LazyState::Eager),
            data: ValueData::Enum(obj),
        }
    }

    /// Wrap a struct object built by MakeStruct.
    pub fn new_struct(obj: Arc<StructObject>) -> Self {
        Self {
            header: ValueHeader::new(TypeTag::Struct, ExecutionTier::Bytecode, LazyState::Eager),
            data: ValueData::Struct(obj),
        }
    }

    /// Wrap a runtime closure built by MakeClosure.
    pub fn new_closure(closure: Arc<ClosureObject>) -> Self {
        Self {
            header: ValueHeader::new(TypeTag::Function, ExecutionTier::Bytecode, LazyState::Eager),
            data: ValueData::Closure(closure),
        }
    }

    /// Wrap an interpreter function verbatim so it converts back unchanged.
    pub fn new_ast_function(func: crate::ast::Function) -> Self {
        Self {
            header: ValueHeader::new(
                TypeTag::Function,
                ExecutionTier::Interpreter,
                LazyState::Eager,
            ),
            data: ValueData::AstFunction(Arc::new(func)),
        }
    }

    /// Create an Ok/Err result value
    pub fn new_result(inner: Self, ok: bool) -> Self {
        let (ok_slot, err_slot) = if ok {
            (Some(Box::new(inner)), None)
        } else {
            (None, Some(Box::new(inner)))
        };
        Self {
            header: ValueHeader::new(
                TypeTag::Result,
                ExecutionTier::Interpreter,
                LazyState::Eager,
            ),
            data: ValueData::Result {
                ok: ok_slot,
                err: err_slot,
            },
        }
    }

    /// Create a new tuple value
    pub fn new_tuple(values: Vec<Self>) -> Self {
        let gc_ptr = Arc::new(values);

        Self {
            header: ValueHeader::new(TypeTag::Tuple, ExecutionTier::Interpreter, LazyState::Eager),
            data: ValueData::Tuple(gc_ptr),
        }
    }

    /// Convert to AST Value (for compatibility)
    pub fn to_ast(&self) -> Result<Value, RuntimeError> {
        match &self.data {
            ValueData::Integer(i) => Ok(Value::Integer(*i)),
            ValueData::Float(f) => Ok(Value::Float(*f)),
            ValueData::Boolean(b) => Ok(Value::Boolean(*b)),
            ValueData::Unit => Ok(Value::Unit),
            ValueData::String(gc_ptr) => Ok(Value::String(gc_ptr.clone())),
            ValueData::List(gc_ptr) => {
                let mut ast_values = Vec::with_capacity(gc_ptr.len());
                for ovm_val in gc_ptr.iter() {
                    ast_values.push(ovm_val.to_ast()?);
                }
                Ok(Value::List(ast_values.into()))
            }
            ValueData::Tuple(gc_ptr) => {
                let mut ast_values = Vec::with_capacity(gc_ptr.len());
                for ovm_val in gc_ptr.iter() {
                    ast_values.push(ovm_val.to_ast()?);
                }
                Ok(Value::Tuple(std::sync::Arc::new(ast_values)))
            }
            ValueData::AstFunction(func) => Ok(Value::Function((**func).clone())),
            ValueData::Map(m) => {
                let mut out = HashMap::new();
                for (k, v) in m.iter() {
                    out.insert(k.clone(), v.to_ast()?);
                }
                Ok(Value::Map(Arc::new(out)))
            }
            ValueData::Enum(e) => {
                let variant_data = match &e.data {
                    EnumData::Unit => crate::ast::EnumVariantData::Unit,
                    EnumData::Tuple(values) => crate::ast::EnumVariantData::Tuple(
                        values
                            .iter()
                            .map(|v| v.to_ast())
                            .collect::<Result<_, _>>()?,
                    ),
                    EnumData::Struct(fields) => crate::ast::EnumVariantData::Struct(
                        fields
                            .iter()
                            .map(|(k, v)| Ok((k.clone(), v.to_ast()?)))
                            .collect::<Result<_, RuntimeError>>()?,
                    ),
                };
                Ok(Value::Enum {
                    type_name: e.type_name.clone(),
                    variant_name: e.variant_name.clone(),
                    variant_data,
                })
            }
            // Rebuild the interpreter function the lambda evaluation would
            // have produced: the declaration-time closure with the runtime
            // captures layered on top. The body Arc is shared verbatim.
            ValueData::Closure(c) => {
                let mut closure_map = (*c.template.closure).clone();
                for (name, val) in c.capture_names.iter().zip(c.captured.iter()) {
                    closure_map.insert(name.clone(), val.to_ast()?);
                }
                Ok(Value::Function(crate::ast::Function {
                    name: None,
                    parameters: c.template.parameters.clone(),
                    body: c.template.body.clone(),
                    closure: Arc::new(closure_map),
                    param_bounds: Vec::new(),
                }))
            }
            ValueData::Function(_) => {
                // Functions return unit for now
                Ok(Value::Unit)
            }
            ValueData::Struct(gc_ptr) => {
                let mut fields = HashMap::new();
                for (name, val) in gc_ptr.iter() {
                    fields.insert(name.clone(), val.to_ast()?);
                }
                Ok(Value::Struct {
                    type_name: gc_ptr.type_name().to_string(),
                    fields,
                })
            }
            ValueData::Range(gc_ptr) => Ok(Value::Range {
                start: gc_ptr.start,
                end: gc_ptr.end,
                inclusive: gc_ptr.inclusive,
            }),
            ValueData::Builtin(_) => {
                // Builtins return unit for now
                Ok(Value::Unit)
            }
            ValueData::Thunk(_) => {
                // Thunks return unit for now
                Ok(Value::Unit)
            }
            ValueData::Stream(_) => {
                // Streams return unit for now
                Ok(Value::Unit)
            }
            ValueData::LazyList(_) => {
                // Lazy lists return unit for now
                Ok(Value::Unit)
            }
            ValueData::Promise(_) => {
                // Promises return unit for now
                Ok(Value::Unit)
            }
            ValueData::CompiledFunction(_) => {
                // Compiled functions return unit for now
                Ok(Value::Unit)
            }
            ValueData::OptimizedValue(_) => {
                // Optimized values return unit for now
                Ok(Value::Unit)
            }
            ValueData::Error(gc_ptr) => Ok(Value::Err(Box::new(Value::String(
                std::sync::Arc::new(gc_ptr.message.clone()),
            )))),
            ValueData::Result { ok, err } => {
                if let Some(ok_val) = ok {
                    Ok(Value::Ok(Box::new(ok_val.to_ast()?)))
                } else if let Some(err_val) = err {
                    Ok(Value::Err(Box::new(err_val.to_ast()?)))
                } else {
                    Err(RuntimeError::new("Result value has neither Ok nor Err"))
                }
            }
            ValueData::Native(handle) => Ok(Value::Native(handle.clone())),
        }
    }
}

impl ValueHeader {
    pub const fn new(type_tag: TypeTag, tier: ExecutionTier, lazy_state: LazyState) -> Self {
        Self {
            type_tag,
            tier,
            lazy_state,
        }
    }
}

impl fmt::Display for OvmValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.data {
            ValueData::Integer(i) => write!(f, "{}", i),
            ValueData::Float(fl) => write!(f, "{}", fl),
            ValueData::Boolean(b) => write!(f, "{}", b),
            ValueData::Unit => write!(f, "()"),
            ValueData::Native(handle) => write!(f, "{}", handle.0.display()),
            ValueData::String(gc_ptr) => write!(f, "\"{}\"", gc_ptr),
            ValueData::List(gc_ptr) => {
                write!(f, "[")?;
                for (i, val) in gc_ptr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", val)?;
                }
                write!(f, "]")
            }
            ValueData::Tuple(gc_ptr) => {
                write!(f, "(")?;
                for (i, val) in gc_ptr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", val)?;
                }
                write!(f, ")")
            }
            ValueData::Function(_) | ValueData::AstFunction(_) | ValueData::Closure(_) => {
                write!(f, "<function>")
            }
            ValueData::Enum(e) => write!(f, "{}::{}", e.type_name, e.variant_name),
            ValueData::Map(m) => write!(f, "<map: {} entries>", m.len()),
            ValueData::Struct(_) => write!(f, "<struct>"),
            ValueData::Range(gc_ptr) => {
                if gc_ptr.inclusive {
                    write!(f, "{}..={}", gc_ptr.start, gc_ptr.end)
                } else {
                    write!(f, "{}..{}", gc_ptr.start, gc_ptr.end)
                }
            }
            ValueData::Builtin(_) => write!(f, "<builtin>"),
            ValueData::Thunk(_) => write!(f, "<thunk>"),
            ValueData::Stream(_) => write!(f, "<stream>"),
            ValueData::LazyList(_) => write!(f, "<lazy-list>"),
            ValueData::Promise(_) => write!(f, "<promise>"),
            ValueData::CompiledFunction(_) => write!(f, "<compiled-function>"),
            ValueData::OptimizedValue(_) => write!(f, "<optimized-value>"),
            ValueData::Error(_) => write!(f, "<error>"),
            ValueData::Result { .. } => write!(f, "<result>"),
        }
    }
}

// Unsafe implementations for Send/Sync (needed for multi-threading)
// OvmValue is Send + Sync automatically now that heap payloads are Arc-based;
// assert it so a non-thread-safe field can't sneak back in.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OvmValue>()
};

impl Default for ValueHeader {
    fn default() -> Self {
        Self::new(TypeTag::Unit, ExecutionTier::Interpreter, LazyState::Eager)
    }
}

impl Default for OptimizationData {
    fn default() -> Self {
        Self {
            inline_cache: Vec::new(),
            type_feedback: TypeFeedback {
                observed_types: Vec::new(),
                type_stability: 0.0,
            },
            call_site_data: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_immediate_values() {
        let int_val = OvmValue::new_integer(42);
        assert_eq!(int_val.type_tag(), TypeTag::Integer);
        assert!(!int_val.is_lazy());
        assert!(int_val.is_immediate());

        let float_val = OvmValue::new_float(std::f64::consts::PI);
        assert_eq!(float_val.type_tag(), TypeTag::Float);

        let bool_val = OvmValue::new_boolean(true);
        assert_eq!(bool_val.type_tag(), TypeTag::Boolean);

        let unit_val = OvmValue::new_unit();
        assert_eq!(unit_val.type_tag(), TypeTag::Unit);
    }

    #[test]
    fn test_value_header() {
        let header = ValueHeader::new(
            TypeTag::Integer,
            ExecutionTier::Interpreter,
            LazyState::Eager,
        );
        assert_eq!(header.type_tag, TypeTag::Integer);
        assert_eq!(header.tier, ExecutionTier::Interpreter);
        assert_eq!(header.lazy_state, LazyState::Eager);
    }

    #[test]
    fn test_ast_conversion() {
        let ast_int = Value::Integer(42);
        let ovm_int = OvmValue::from_ast(ast_int);
        assert_eq!(ovm_int.type_tag(), TypeTag::Integer);

        let converted_back = ovm_int.to_ast().unwrap();
        assert_eq!(converted_back, Value::Integer(42));
    }
}

//! OVM Unified Value System
//!
//! Provides a unified value representation that works across all execution tiers
//! with integrated garbage collection, lazy evaluation, and optimization metadata.

use std::collections::HashMap;
use std::fmt;

use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use crate::ast::{Expr, Value};

/// Ok/Err payloads behind the Result variant's single Arc.
#[derive(Debug)]
pub struct ResultObject {
    pub ok: Option<OvmValue>,
    pub err: Option<OvmValue>,
}

/// Unified value representation for the OVM
#[derive(Debug)]
#[repr(C)]
pub struct OvmValue {
    /// Actual value data. The old ValueHeader (type tag, tier, lazy state)
    /// is gone: the tag is derivable from the data, the tier was never
    /// read, and no constructor ever produced a lazy value — it was 8
    /// bytes copied on every register move for nothing.
    pub data: ValueData,
}

/// Type tags for fast runtime type checking
/// Which tier produced or owns a compiled artifact. (Formerly part of
/// the per-value header; now only compilation metadata uses it.)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionTier {
    Interpreter = 0,
    Bytecode = 1,
}

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

    // Error types
    Error = 50,
    Result = 51,

    // Values owned by OVM modules (ods arrays, ...)
    Native = 60,
}

/// Lists at or under this length convert eagerly at the tier boundary;
/// longer ones wrap as `AstList`. Small enough that ordinary code keeps
/// the native fast paths; large enough that a collections handle (or any
/// big data list) never pays a whole-list conversion per call.
pub const AST_LIST_EAGER: usize = 64;

/// The typed-conversion cache: interpreter list (by Arc pointer) → its
/// typed OvmValue. Bounded; oldest entry evicted. Thread-local because
/// conversions happen on whichever thread runs the boundary.
const TYPED_CACHE_CAP: usize = 16;
type TypedCacheEntry = (usize, Arc<Vec<Value>>, OvmValue);
thread_local! {
    static TYPED_CACHE: std::cell::RefCell<std::collections::VecDeque<TypedCacheEntry>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

fn typed_cache_lookup(items: &Arc<Vec<Value>>) -> Option<OvmValue> {
    let key = Arc::as_ptr(items) as usize;
    TYPED_CACHE.with(|c| {
        c.borrow()
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, _, v)| v.clone())
    })
}

fn typed_cache_insert(items: &Arc<Vec<Value>>, typed: &OvmValue) {
    let key = Arc::as_ptr(items) as usize;
    TYPED_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= TYPED_CACHE_CAP {
            c.pop_front();
        }
        c.push_back((key, items.clone(), typed.clone()));
    });
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
    /// Homogeneous scalar lists in their native layout (Campaign:
    /// typed-list backing). A list of a million floats is a
    /// `Vec<f64>`, not a million boxed values: indexing yields an
    /// immediate, iteration streams contiguous memory, and the
    /// sole-owner in-place write and append paths work on raw
    /// scalars. Detected at the tier boundary by one early-exit scan;
    /// any operation that inserts a non-matching element rebuilds the
    /// boxed form (correctness first, the fast layout is an
    /// optimization).
    FloatList(Arc<Vec<f64>>),
    IntList(Arc<Vec<i64>>),
    /// An interpreter list held verbatim — the AstFunction idea applied
    /// to data (Campaign 7, T2). Crossing the tier boundary with a large
    /// list used to convert every element, both directions, per call:
    /// O(n) tax on O(log n) operations, and the reason the bundled
    /// collections were pinned to the interpreter. Wrapped, the boundary
    /// is O(1) each way; the fast list instructions (length, index, the
    /// in-place writes, iteration) read and write through the wrapper
    /// with per-element conversion, and any operation that wants the
    /// native layout materializes it once — a cost no larger than the
    /// O(n) work such an operation was about to do anyway. Small lists
    /// (at or under AST_LIST_EAGER) still convert eagerly at the
    /// boundary, so ordinary code never meets this variant.
    AstList(Arc<Vec<crate::ast::Value>>),
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

    // Error handling
    Error(Arc<ErrorObject>),
    /// Ok/Err behind one Arc so the variant is pointer-sized — the
    /// two-slot inline form made every OvmValue 24 bytes instead of 16.
    Result(Arc<ResultObject>),

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
    /// The interpreter-side closure map this object converts to, built
    /// once. The bridge's HOF cache keys a lambda on its closure Arc, so
    /// a fresh Arc per boundary crossing recompiled the same lambda on
    /// every callback a builtin made into it.
    pub ast_closure: std::sync::OnceLock<Arc<im::HashMap<String, crate::ast::Value>>>,
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

/// Error object
#[derive(Debug, Clone)]
pub struct ErrorObject {
    pub message: String,
    pub error_type: String,
    pub stack_trace: Vec<String>,
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
        if std::mem::discriminant(&self.data) != std::mem::discriminant(&other.data) {
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
            (ValueData::AstList(a), ValueData::AstList(b)) => a == b,
            // Mixed representations compare element-wise through a
            // conversion — an O(n) comparison was O(n) already.
            (ValueData::List(a), ValueData::AstList(b))
            | (ValueData::AstList(b), ValueData::List(a)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| *x == OvmValue::from_ast(y.clone()))
            }
            // Typed lists: element equality is strict per the language
            // ([1] != [1.0]), so a FloatList never equals an IntList,
            // and against boxed forms only the matching scalar arm hits.
            (ValueData::FloatList(a), ValueData::FloatList(b)) => a == b,
            (ValueData::IntList(a), ValueData::IntList(b)) => a == b,
            (ValueData::FloatList(_), ValueData::IntList(_))
            | (ValueData::IntList(_), ValueData::FloatList(_)) => false,
            (ValueData::FloatList(a), ValueData::List(b))
            | (ValueData::List(b), ValueData::FloatList(a)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| matches!(y.data, ValueData::Float(f) if f == *x))
            }
            (ValueData::IntList(a), ValueData::List(b))
            | (ValueData::List(b), ValueData::IntList(a)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| matches!(y.data, ValueData::Integer(n) if n == *x))
            }
            (ValueData::FloatList(a), ValueData::AstList(b))
            | (ValueData::AstList(b), ValueData::FloatList(a)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| matches!(y, Value::Float(f) if f == x))
            }
            (ValueData::IntList(a), ValueData::AstList(b))
            | (ValueData::AstList(b), ValueData::IntList(a)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| matches!(y, Value::Integer(n) if n == x))
            }

            (ValueData::Tuple(a), ValueData::Tuple(b)) => a == b,

            (ValueData::Range(a), ValueData::Range(b)) => {
                a.start == b.start && a.end == b.end && a.inclusive == b.inclusive
            }

            (ValueData::Result(a), ValueData::Result(b)) => match (&a.ok, &a.err, &b.ok, &b.err) {
                (Some(a_val), None, Some(b_val), None) => a_val == b_val,
                (None, Some(a_val), None, Some(b_val)) => a_val == b_val,
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
            (ValueData::Error(a), ValueData::Error(b)) => Arc::ptr_eq(a, b),

            // Different types are never equal
            _ => false,
        }
    }
}

impl Eq for OvmValue {}

impl OvmValue {
    /// Runtime type name, spelled exactly as `crate::ast::Value::type_name`
    /// so a struct field-type check compares the same strings on either
    /// tier: `Int`, `Float`, `Bool`, `String`, `List`, `Map`, `Tuple`,
    /// `Function`, `Range`, `Result`, `Unit`, or a struct/enum's
    /// declared type name.
    pub fn type_name(&self) -> &str {
        match &self.data {
            ValueData::Integer(_) => "Int",
            ValueData::Float(_) => "Float",
            ValueData::Boolean(_) => "Bool",
            ValueData::String(_) => "String",
            ValueData::List(_)
            | ValueData::AstList(_)
            | ValueData::FloatList(_)
            | ValueData::IntList(_) => "List",
            ValueData::Map(_) => "Map",
            ValueData::Tuple(_) => "Tuple",
            ValueData::Function(_) | ValueData::AstFunction(_) | ValueData::Closure(_) => {
                "Function"
            }
            ValueData::Builtin(_) => "Builtin",
            ValueData::Struct(s) => &s.shape.type_name,
            ValueData::Enum(e) => &e.type_name,
            ValueData::Range(_) => "Range",
            ValueData::Result(_) => "Result",
            ValueData::Unit => "Unit",
            ValueData::Error(_) => "Error",
            ValueData::Native(handle) => handle.0.type_name(),
        }
    }

    /// Create a new integer value
    #[inline]
    pub fn new_integer(value: i64) -> Self {
        Self {
            data: ValueData::Integer(value),
        }
    }

    /// Create a new float value
    #[inline]
    pub fn new_float(value: f64) -> Self {
        Self {
            data: ValueData::Float(value),
        }
    }

    /// Create a new boolean value
    #[inline]
    pub fn new_boolean(value: bool) -> Self {
        Self {
            data: ValueData::Boolean(value),
        }
    }

    /// Create a new unit value
    pub fn new_unit() -> Self {
        Self {
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
                    data: ValueData::Integer(*i),
                };
            }
            ValueData::Float(f) => {
                return Self {
                    data: ValueData::Float(*f),
                };
            }
            ValueData::Boolean(b) => {
                return Self {
                    data: ValueData::Boolean(*b),
                };
            }
            ValueData::Unit => {
                return Self {
                    data: ValueData::Unit,
                };
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
            ValueData::FloatList(p) => ValueData::FloatList(p.clone()),
            ValueData::IntList(p) => ValueData::IntList(p.clone()),
            ValueData::AstList(p) => ValueData::AstList(p.clone()),
            ValueData::Tuple(p) => ValueData::Tuple(p.clone()),
            ValueData::Function(p) => ValueData::Function(p.clone()),
            ValueData::AstFunction(p) => ValueData::AstFunction(p.clone()),
            ValueData::Closure(p) => ValueData::Closure(p.clone()),
            ValueData::Enum(p) => ValueData::Enum(p.clone()),
            ValueData::Map(p) => ValueData::Map(p.clone()),
            ValueData::Struct(p) => ValueData::Struct(p.clone()),
            ValueData::Range(p) => ValueData::Range(p.clone()),
            ValueData::Builtin(p) => ValueData::Builtin(p.clone()),
            ValueData::Error(p) => ValueData::Error(p.clone()),
            ValueData::Result(p) => ValueData::Result(p.clone()),
        };
        Self { data }
    }

    /// Get the type tag of this value, derived from the data itself
    /// (reproducing exactly the tags the removed constructors assigned).
    pub fn type_tag(&self) -> TypeTag {
        match &self.data {
            ValueData::Integer(_) => TypeTag::Integer,
            ValueData::Float(_) => TypeTag::Float,
            ValueData::Boolean(_) => TypeTag::Boolean,
            ValueData::Unit => TypeTag::Unit,
            ValueData::String(_) => TypeTag::String,
            ValueData::List(_)
            | ValueData::AstList(_)
            | ValueData::FloatList(_)
            | ValueData::IntList(_) => TypeTag::List,
            ValueData::Tuple(_) => TypeTag::Tuple,
            ValueData::Function(_) | ValueData::AstFunction(_) | ValueData::Closure(_) => {
                TypeTag::Function
            }
            ValueData::Range(_) => TypeTag::Range,
            ValueData::Builtin(_) => TypeTag::Builtin,
            ValueData::Native(_) => TypeTag::Native,
            ValueData::Result(_) => TypeTag::Result,
            ValueData::Enum(_) | ValueData::Map(_) | ValueData::Struct(_) | ValueData::Error(_) => {
                TypeTag::Struct
            }
        }
    }

    /// Lazy values no longer exist (no constructor ever produced one).
    pub fn is_lazy(&self) -> bool {
        false
    }

    /// Check if this value is immediate (no heap allocation)
    pub fn is_immediate(&self) -> bool {
        matches!(
            self.type_tag(),
            TypeTag::Integer | TypeTag::Float | TypeTag::Boolean | TypeTag::Unit
        )
    }

    /// Lazy forcing is gone with the header; nothing constructs lazy values.
    pub fn force(&mut self) -> Result<(), RuntimeError> {
        Ok(())
    }

    pub fn new_string(value: String) -> Self {
        let gc_ptr = Arc::new(value);

        Self {
            data: ValueData::String(gc_ptr),
        }
    }

    /// Create a new list value
    pub fn new_list(values: Vec<Self>) -> Self {
        let gc_ptr = Arc::new(values);

        Self {
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
                found: format!("{:?}", self.type_tag()),
            }),
        }
    }

    fn detect_float_list(items: &[Value]) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(items.len());
        for v in items {
            match v {
                Value::Float(f) => out.push(*f),
                _ => return None,
            }
        }
        Some(OvmValue {
            data: ValueData::FloatList(Arc::new(out)),
        })
    }

    fn detect_int_list(items: &[Value]) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(items.len());
        for v in items {
            match v {
                Value::Integer(n) => out.push(*n),
                _ => return None,
            }
        }
        Some(OvmValue {
            data: ValueData::IntList(Arc::new(out)),
        })
    }

    /// The boxed form of any list-shaped value: typed lists box their
    /// scalars, a boxed list hands back its own Arc. The JIT/OSR
    /// boundaries use this to classify and keep alive a typed list the
    /// native ABI reads in boxed layout — one O(n) pass, paid at a
    /// call that was about to run O(n * epochs) of work.
    pub fn to_boxed_list(&self) -> Option<Arc<Vec<OvmValue>>> {
        match &self.data {
            ValueData::List(items) => Some(items.clone()),
            ValueData::FloatList(v) => Some(Arc::new(
                v.iter().map(|&f| OvmValue::new_float(f)).collect(),
            )),
            ValueData::IntList(v) => Some(Arc::new(
                v.iter().map(|&n| OvmValue::new_integer(n)).collect(),
            )),
            ValueData::AstList(items) => Some(Arc::new(
                items
                    .iter()
                    .map(|v| OvmValue::from_ast(v.clone()))
                    .collect(),
            )),
            _ => None,
        }
    }

    pub fn new_float_list(values: Vec<f64>) -> Self {
        OvmValue {
            data: ValueData::FloatList(Arc::new(values)),
        }
    }

    pub fn new_int_list(values: Vec<i64>) -> Self {
        OvmValue {
            data: ValueData::IntList(Arc::new(values)),
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
                // Homogeneous scalar lists take the typed layout: the
                // detection scan is one early-exit pass and the typed
                // copy a contiguous fill. Large lists go through a
                // pointer-keyed cache, because the SAME interpreter list
                // can cross the boundary once per lambda call (a
                // captured 300k-element list in a mapped closure crosses
                // millions of times) — the AstList wrapper kept that
                // boundary O(1), and the typed layout must too. The
                // cached entry's keepalive Arc makes the cache sound: a
                // pinned allocation cannot be freed and reused, and the
                // interpreter's sole-owner in-place writes see the extra
                // reference and copy instead of mutating.
                if items.len() > AST_LIST_EAGER {
                    if let Some(hit) = typed_cache_lookup(&items) {
                        return hit;
                    }
                    let detected =
                        Self::detect_float_list(&items).or_else(|| Self::detect_int_list(&items));
                    if let Some(tl) = detected {
                        typed_cache_insert(&items, &tl);
                        return tl;
                    }
                    return OvmValue {
                        data: ValueData::AstList(items),
                    };
                }
                if let Some(fl) = Self::detect_float_list(&items) {
                    return fl;
                }
                if let Some(il) = Self::detect_int_list(&items) {
                    return il;
                }
                if items.len() <= AST_LIST_EAGER {
                    let ovm_items: Vec<Self> = items
                        .iter()
                        .map(|item| Self::from_ast(item.clone()))
                        .collect();
                    Self::new_list(ovm_items)
                } else {
                    OvmValue {
                        data: ValueData::AstList(items),
                    }
                }
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
                data: ValueData::Result(Arc::new(ResultObject {
                    ok: Some(Self::from_ast(*value)),
                    err: None,
                })),
            },

            Value::Err(value) => Self {
                data: ValueData::Result(Arc::new(ResultObject {
                    ok: None,
                    err: Some(Self::from_ast(*value)),
                })),
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
                    data: ValueData::Range(gc_ptr),
                }
            }

            Value::Struct { type_name, fields } => {
                // The fields are behind an Arc now, so this borrows and
                // clones each value rather than consuming the map — the
                // caller's struct may still be alive and shared.
                let pairs: Vec<(String, OvmValue)> = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::from_ast(v.clone())))
                    .collect();
                let struct_obj = StructObject::from_pairs(&type_name, pairs);

                let gc_ptr = Arc::new(struct_obj);

                Self {
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
        let ovm_value = Self::from_ast(ast_value);

        // Record allocation with safepoint manager
        let allocation_size = std::mem::size_of::<OvmValue>()
            + match &ovm_value.data {
                ValueData::String(s) => s.len(),
                ValueData::List(gc_ptr) => gc_ptr.len() * std::mem::size_of::<OvmValue>(),
                ValueData::FloatList(v) => v.len() * 8,
                ValueData::IntList(v) => v.len() * 8,
                ValueData::AstList(items) => items.len() * std::mem::size_of::<crate::ast::Value>(),
                ValueData::Tuple(gc_ptr) => gc_ptr.len() * std::mem::size_of::<OvmValue>(),
                ValueData::Function(_) => std::mem::size_of::<FunctionObject>(),
                ValueData::AstFunction(_) => std::mem::size_of::<crate::ast::Function>(),
                ValueData::Closure(_) => std::mem::size_of::<ClosureObject>(),
                ValueData::Enum(_) => std::mem::size_of::<EnumObject>(),
                ValueData::Map(m) => m.len() * 64,
                ValueData::Struct(_) => std::mem::size_of::<StructObject>(),
                ValueData::Range(_) => std::mem::size_of::<RangeObject>(),
                _ => 0,
            };

        safepoint_manager.record_allocation(allocation_size);

        Ok(ovm_value)
    }

    /// Wrap a map value.
    pub fn new_map(map: Arc<HashMap<String, OvmValue>>) -> Self {
        Self {
            data: ValueData::Map(map),
        }
    }

    /// Wrap an enum value.
    pub fn new_enum(obj: Arc<EnumObject>) -> Self {
        Self {
            data: ValueData::Enum(obj),
        }
    }

    /// Wrap a struct object built by MakeStruct.
    pub fn new_struct(obj: Arc<StructObject>) -> Self {
        Self {
            data: ValueData::Struct(obj),
        }
    }

    /// Wrap a runtime closure built by MakeClosure.
    pub fn new_closure(closure: Arc<ClosureObject>) -> Self {
        Self {
            data: ValueData::Closure(closure),
        }
    }

    /// Wrap an interpreter function verbatim so it converts back unchanged.
    pub fn new_ast_function(func: crate::ast::Function) -> Self {
        Self {
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
            data: ValueData::Result(Arc::new(ResultObject {
                ok: ok_slot.map(|b| *b),
                err: err_slot.map(|b| *b),
            })),
        }
    }

    /// Create a new tuple value
    pub fn new_tuple(values: Vec<Self>) -> Self {
        let gc_ptr = Arc::new(values);

        Self {
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
            ValueData::FloatList(v) => Ok(Value::List(Arc::new(
                v.iter().map(|&f| Value::Float(f)).collect(),
            ))),
            ValueData::IntList(v) => Ok(Value::List(Arc::new(
                v.iter().map(|&n| Value::Integer(n)).collect(),
            ))),
            ValueData::List(gc_ptr) => {
                let mut ast_values = Vec::with_capacity(gc_ptr.len());
                for ovm_val in gc_ptr.iter() {
                    ast_values.push(ovm_val.to_ast()?);
                }
                Ok(Value::List(ast_values.into()))
            }
            // The whole point: the boundary out is a pointer move.
            ValueData::AstList(items) => Ok(Value::List(items.clone())),
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
                if std::env::var_os("OLANG_DEBUG_CLOSURE").is_some() {
                    eprintln!(
                        "[closure->ast] params={:?} capture_names={:?} template_closure_keys={:?}",
                        c.template
                            .parameters
                            .iter()
                            .map(|p| &p.name)
                            .collect::<Vec<_>>(),
                        c.capture_names,
                        c.template.closure.keys().collect::<Vec<_>>()
                    );
                }
                let closure = match c.ast_closure.get() {
                    Some(existing) => existing.clone(),
                    None => {
                        let mut closure_map = (*c.template.closure).clone();
                        for (name, val) in c.capture_names.iter().zip(c.captured.iter()) {
                            closure_map.insert(name.clone(), val.to_ast()?);
                        }
                        let built = Arc::new(closure_map);
                        let _ = c.ast_closure.set(built.clone());
                        built
                    }
                };
                Ok(Value::Function(crate::ast::Function {
                    // The template's name survives the round trip: a
                    // nested fn's self-recursion binds through its name
                    // at call time, and a rebuilt `insert` that lost it
                    // could not call itself.
                    name: c.template.name.clone(),
                    param_checks: crate::ast::param_checks_of(&c.template.parameters, &[]),
                    return_check: None,
                    parameters: c.template.parameters.clone(),
                    body: c.template.body.clone(),
                    closure,
                    param_bounds: Vec::new(),
                    def_file: c.template.def_file.clone(),
                    parent_scope: 0,
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
                    fields: std::sync::Arc::new(fields),
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
            ValueData::Error(gc_ptr) => Ok(Value::Err(Box::new(Value::String(
                std::sync::Arc::new(gc_ptr.message.clone()),
            )))),
            ValueData::Result(r) => {
                if let Some(ok_val) = &r.ok {
                    Ok(Value::Ok(Box::new(ok_val.to_ast()?)))
                } else if let Some(err_val) = &r.err {
                    Ok(Value::Err(Box::new(err_val.to_ast()?)))
                } else {
                    Err(RuntimeError::new("Result value has neither Ok nor Err"))
                }
            }
            ValueData::Native(handle) => Ok(Value::Native(handle.clone())),
        }
    }
}

impl fmt::Display for OvmValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.data {
            ValueData::Integer(i) => write!(f, "{}", i),
            ValueData::Float(fl) => write!(f, "{}", crate::ast::format_float(*fl)),
            ValueData::Boolean(b) => write!(f, "{}", b),
            ValueData::Unit => write!(f, "()"),
            ValueData::Native(handle) => write!(f, "{}", handle.0.display()),
            ValueData::String(gc_ptr) => write!(f, "\"{}\"", gc_ptr),
            ValueData::FloatList(v) => {
                write!(f, "[")?;
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", crate::ast::format_float(*x))?;
                }
                write!(f, "]")
            }
            ValueData::IntList(v) => {
                write!(f, "[")?;
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", x)?;
                }
                write!(f, "]")
            }
            ValueData::AstList(items) => {
                write!(f, "[")?;
                for (i, val) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", OvmValue::from_ast(val.clone()))?;
                }
                write!(f, "]")
            }
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
            ValueData::Error(_) => write!(f, "<error>"),
            ValueData::Result(_) => write!(f, "<result>"),
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
    fn test_ast_conversion() {
        let ast_int = Value::Integer(42);
        let ovm_int = OvmValue::from_ast(ast_int);
        assert_eq!(ovm_int.type_tag(), TypeTag::Integer);

        let converted_back = ovm_int.to_ast().unwrap();
        assert_eq!(converted_back, Value::Integer(42));
    }
    #[test]
    fn ovm_value_is_sixteen_bytes() {
        // The P3 probe: the dead ValueHeader is gone. 16 bytes = the data
        // enum alone (8-byte payload + discriminant, padded). If this
        // grows, every register move pays for it — treat as a regression.
        assert_eq!(std::mem::size_of::<OvmValue>(), 16);
    }
}

//! OVM Unified Value System
//!
//! Provides a unified value representation that works across all execution tiers
//! with integrated garbage collection, lazy evaluation, and optimization metadata.

use std::collections::HashMap;
use std::fmt;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
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

/// Value header containing metadata for GC, execution, and optimization
#[derive(Debug)]
#[repr(C)]
pub struct ValueHeader {
    /// GC metadata (mark bits, generation, forwarding pointer)
    pub gc_bits: AtomicU32,

    /// Value type tag for fast type checking
    pub type_tag: TypeTag,

    /// Current execution tier for this value
    pub tier: ExecutionTier,

    /// Tier-specific optimization data
    pub optimization_data: u32,

    /// Lazy evaluation state
    pub lazy_state: LazyState,

    /// Force count for profiling lazy evaluation
    pub force_count: AtomicU32,

    /// Reference count for shared values (when not using GC)
    pub ref_count: AtomicU32,

    /// Mark bit for garbage collection
    pub gc_mark: bool,

    /// Age of the value
    pub age: u8,

    /// Size of the value
    pub size: u32,
}

impl Clone for ValueHeader {
    fn clone(&self) -> Self {
        Self {
            gc_bits: AtomicU32::new(self.gc_bits.load(Ordering::Relaxed)),
            type_tag: self.type_tag,
            tier: self.tier,
            optimization_data: self.optimization_data,
            lazy_state: self.lazy_state,
            force_count: AtomicU32::new(self.force_count.load(Ordering::Relaxed)),
            ref_count: AtomicU32::new(self.ref_count.load(Ordering::Relaxed)),
            gc_mark: self.gc_mark,
            age: self.age,
            size: self.size,
        }
    }
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

    // GC-managed values
    String(GcPtr<String>),
    List(GcPtr<ValueArray>),
    Tuple(GcPtr<ValueArray>),
    Function(GcPtr<FunctionObject>),
    Struct(GcPtr<StructObject>),
    Range(GcPtr<RangeObject>),
    Builtin(GcPtr<BuiltinObject>),

    // Lazy values
    Thunk(GcPtr<ThunkObject>),
    Stream(GcPtr<StreamObject>),
    LazyList(GcPtr<LazyListObject>),

    // Async values
    Promise(GcPtr<PromiseObject>),

    // Compiled representations
    CompiledFunction(GcPtr<CompiledFunctionObject>),
    OptimizedValue(GcPtr<OptimizedValueObject>),

    // Error handling
    Error(GcPtr<ErrorObject>),
    Result {
        ok: Option<Box<OvmValue>>,
        err: Option<Box<OvmValue>>,
    },
}

/// GC pointer type
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct GcPtr<T> {
    ptr: NonNull<T>,
    generation: u32,
}

// Manual impls: the derived ones bound on `T: Copy`/`T: Clone`, but copying
// the pointer itself never requires copying the pointee.
impl<T> Clone for GcPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for GcPtr<T> {}

unsafe impl<T> Send for GcPtr<T> {}
unsafe impl<T> Sync for GcPtr<T> {}

/// Array of values for lists and tuples
#[derive(Debug)]
pub struct ValueArray {
    pub length: usize,
    pub capacity: usize,
    pub data: *mut OvmValue,
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

/// Struct object representation
#[derive(Debug)]
pub struct StructObject {
    pub type_name: String,
    pub fields: HashMap<String, OvmValue>,
}

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
    pub dependencies: Vec<GcPtr<OvmValue>>,
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
    pub original_function: GcPtr<FunctionObject>,
    pub compiled_code: *const u8,
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
        function: GcPtr<FunctionObject>,
    },
    Filter {
        source: Box<OvmValue>,
        predicate: GcPtr<FunctionObject>,
    },
    Custom {
        function: GcPtr<FunctionObject>,
    },
}

#[derive(Debug)]
pub enum TransformationChain {
    Identity,
    Map(GcPtr<FunctionObject>),
    Filter(GcPtr<FunctionObject>),
    Chain(Box<TransformationChain>, Box<TransformationChain>),
}

#[derive(Debug)]
pub struct CallbackFunction {
    pub function: GcPtr<FunctionObject>,
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
    pub target_function: GcPtr<FunctionObject>,
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
            
            (ValueData::String(a), ValueData::String(b)) => unsafe {
                // Compare string contents
                a.as_ref() == b.as_ref()
            },
            
            (ValueData::List(a), ValueData::List(b)) => unsafe {
                let a_array = a.as_ref();
                let b_array = b.as_ref();
                
                if a_array.length != b_array.length {
                    return false;
                }
                
                if a_array.length == 0 {
                    return true;
                }
                
                // Validate pointers before creating slices
                if a_array.data.is_null() || b_array.data.is_null() {
                    return false;
                }
                
                // Additional bounds check to prevent buffer overflows
                if a_array.length > isize::MAX as usize || b_array.length > isize::MAX as usize {
                    return false;
                }
                
                let a_slice = std::slice::from_raw_parts(a_array.data, a_array.length);
                let b_slice = std::slice::from_raw_parts(b_array.data, b_array.length);
                
                a_slice.iter().zip(b_slice.iter()).all(|(x, y)| x == y)
            },
            
            (ValueData::Tuple(a), ValueData::Tuple(b)) => unsafe {
                let a_array = a.as_ref();
                let b_array = b.as_ref();
                
                if a_array.length != b_array.length {
                    return false;
                }
                
                if a_array.length == 0 {
                    return true;
                }
                
                // Validate pointers before creating slices
                if a_array.data.is_null() || b_array.data.is_null() {
                    return false;
                }
                
                // Additional bounds check to prevent buffer overflows
                if a_array.length > isize::MAX as usize || b_array.length > isize::MAX as usize {
                    return false;
                }
                
                let a_slice = std::slice::from_raw_parts(a_array.data, a_array.length);
                let b_slice = std::slice::from_raw_parts(b_array.data, b_array.length);
                
                a_slice.iter().zip(b_slice.iter()).all(|(x, y)| x == y)
            },
            
            (ValueData::Range(a), ValueData::Range(b)) => unsafe {
                let a_range = a.as_ref();
                let b_range = b.as_ref();
                
                a_range.start == b_range.start &&
                a_range.end == b_range.end &&
                a_range.inclusive == b_range.inclusive
            },
            
            (ValueData::Result { ok: a_ok, err: a_err }, ValueData::Result { ok: b_ok, err: b_err }) => {
                match (a_ok, a_err, b_ok, b_err) {
                    (Some(a_val), None, Some(b_val), None) => a_val.as_ref() == b_val.as_ref(),
                    (None, Some(a_val), None, Some(b_val)) => a_val.as_ref() == b_val.as_ref(),
                    (None, None, None, None) => true,
                    _ => false,
                }
            },
            
            // For complex types, fall back to pointer comparison for now
            (ValueData::Function(a), ValueData::Function(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (ValueData::Struct(a), ValueData::Struct(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (ValueData::Builtin(a), ValueData::Builtin(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (ValueData::Promise(a), ValueData::Promise(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (ValueData::Thunk(a), ValueData::Thunk(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (ValueData::LazyList(a), ValueData::LazyList(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (ValueData::Stream(a), ValueData::Stream(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (ValueData::CompiledFunction(a), ValueData::CompiledFunction(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (ValueData::OptimizedValue(a), ValueData::OptimizedValue(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (ValueData::Error(a), ValueData::Error(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            
            // Different types are never equal
            _ => false,
        }
    }
}

impl Eq for OvmValue {}

impl OvmValue {
    /// Create a new integer value
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
    pub fn new_float(value: f64) -> Self {
        Self {
            header: ValueHeader::new(TypeTag::Float, ExecutionTier::Interpreter, LazyState::Eager),
            data: ValueData::Float(value),
        }
    }

    /// Create a new boolean value
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
    pub fn clone_simple(&self) -> Self {
        let data = match &self.data {
            ValueData::Integer(i) => ValueData::Integer(*i),
            ValueData::Float(f) => ValueData::Float(*f),
            ValueData::Boolean(b) => ValueData::Boolean(*b),
            ValueData::Unit => ValueData::Unit,
            ValueData::String(p) => ValueData::String(*p),
            ValueData::List(p) => ValueData::List(*p),
            ValueData::Tuple(p) => ValueData::Tuple(*p),
            ValueData::Function(p) => ValueData::Function(*p),
            ValueData::Struct(p) => ValueData::Struct(*p),
            ValueData::Range(p) => ValueData::Range(*p),
            ValueData::Builtin(p) => ValueData::Builtin(*p),
            ValueData::Thunk(p) => ValueData::Thunk(*p),
            ValueData::Stream(p) => ValueData::Stream(*p),
            ValueData::LazyList(p) => ValueData::LazyList(*p),
            ValueData::Promise(p) => ValueData::Promise(*p),
            ValueData::CompiledFunction(p) => ValueData::CompiledFunction(*p),
            ValueData::OptimizedValue(p) => ValueData::OptimizedValue(*p),
            ValueData::Error(p) => ValueData::Error(*p),
            ValueData::Result { ok, err } => ValueData::Result {
                ok: ok.clone(),
                err: err.clone(),
            },
        };
        Self {
            header: self.header.clone(),
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

        // Increment force count for profiling
        self.header.force_count.fetch_add(1, Ordering::Relaxed);

        // Handle different lazy value types based on lazy state
        let lazy_state = self.header.lazy_state;
        match lazy_state {
            LazyState::Lazy => {
                // Extract the data temporarily to avoid borrowing issues
                match &self.data {
                    ValueData::Thunk(thunk_ptr) => {
                        let thunk_ptr_copy = GcPtr { ptr: thunk_ptr.ptr, generation: thunk_ptr.generation };
                        self.force_thunk_impl(thunk_ptr_copy)?;
                    }
                    ValueData::LazyList(lazy_list_ptr) => {
                        let lazy_list_ptr_copy = GcPtr { ptr: lazy_list_ptr.ptr, generation: lazy_list_ptr.generation };
                        self.force_lazy_list_impl(lazy_list_ptr_copy)?;
                    }
                    _ => {}
                }
            }
            LazyState::Stream => {
                // Streams remain lazy but may buffer more data
                if let ValueData::Stream(stream_ptr) = &self.data {
                    let stream_ptr_copy = GcPtr { ptr: stream_ptr.ptr, generation: stream_ptr.generation };
                    self.advance_stream_buffer_impl(stream_ptr_copy)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Force evaluation of a thunk with memoization
    fn force_thunk_impl(&mut self, thunk_ptr: GcPtr<ThunkObject>) -> Result<(), RuntimeError> {
        // Prevent infinite recursion
        if self.header.lazy_state == LazyState::Forcing {
            return Err(RuntimeError::new("Circular thunk dependency detected"));
        }

        // Set forcing state
        self.header.lazy_state = LazyState::Forcing;

        // Access the thunk safely
        let thunk_ref = unsafe { thunk_ptr.as_ref() };
        
        // Check if already memoized (with lock)
        {
            let memoized_guard = thunk_ref.memoized_value.lock()
                .map_err(|_| RuntimeError::ConcurrencyError("Failed to acquire thunk lock".to_string()))?;
            
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
            let mut memoized_guard = thunk_ref.memoized_value.lock()
                .map_err(|_| RuntimeError::ConcurrencyError("Failed to acquire thunk lock".to_string()))?;
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
    fn force_lazy_list_impl(&mut self, lazy_list_ptr: GcPtr<LazyListObject>) -> Result<(), RuntimeError> {
        // For now, materialize a reasonable prefix of the lazy list
        let chunk_size = 100; // Configurable chunk size
        
        let lazy_list_ref = unsafe { lazy_list_ptr.as_ref() };
        
        // Use locks to safely access and modify the materialized data
        let mut materialized_guard = lazy_list_ref.materialized_prefix.lock()
            .map_err(|_| RuntimeError::ConcurrencyError("Failed to acquire lazy list lock".to_string()))?;
        let mut materialization_point_guard = lazy_list_ref.materialization_point.lock()
            .map_err(|_| RuntimeError::ConcurrencyError("Failed to acquire materialization point lock".to_string()))?;
        
        // If we haven't materialized anything yet, start materializing
        if materialized_guard.is_empty() {
            match &lazy_list_ref.transformation {
                TransformationChain::Identity => {
                    // Copy from source - simplified implementation
                    if let ValueData::List(source_list) = &lazy_list_ref.source.data {
                        let source_array = unsafe { source_list.as_ref() };
                        let mut materialized = 0;
                        for i in 0..source_array.length.min(chunk_size) {
                            unsafe {
                                let item = source_array.data.add(i).read();
                                materialized_guard.push(item);
                                materialized += 1;
                            }
                        }
                        *materialization_point_guard = materialized;
                    }
                }
                TransformationChain::Map(_map_fn) => {
                    // Apply map transformation - simplified for now
                    if let ValueData::List(source_list) = &lazy_list_ref.source.data {
                        let source_array = unsafe { source_list.as_ref() };
                        let mut materialized = 0;
                        for i in 0..source_array.length.min(chunk_size) {
                            unsafe {
                                let item = source_array.data.add(i).read();
                                // For now, just copy the item (would need interpreter context for actual function call)
                                materialized_guard.push(item);
                                materialized += 1;
                            }
                        }
                        *materialization_point_guard = materialized;
                    }
                }
                TransformationChain::Filter(_filter_fn) => {
                    // Apply filter transformation - simplified for now  
                    if let ValueData::List(source_list) = &lazy_list_ref.source.data {
                        let source_array = unsafe { source_list.as_ref() };
                        let mut materialized = 0;
                        for i in 0..source_array.length.min(chunk_size) {
                            if materialized >= chunk_size {
                                break;
                            }
                            unsafe {
                                let item = source_array.data.add(i).read();
                                // For now, include all items (would need interpreter context for actual predicate)
                                materialized_guard.push(item);
                                materialized += 1;
                            }
                        }
                        *materialization_point_guard = materialized;
                    }
                }
                TransformationChain::Chain(_first, _second) => {
                    // Apply chained transformations - simplified for now
                    if let ValueData::List(source_list) = &lazy_list_ref.source.data {
                        let source_array = unsafe { source_list.as_ref() };
                        let mut materialized = 0;
                        for i in 0..source_array.length.min(chunk_size) {
                            unsafe {
                                let item = source_array.data.add(i).read();
                                materialized_guard.push(item);
                                materialized += 1;
                            }
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
    fn advance_stream_buffer_impl(&mut self, stream_ptr: GcPtr<StreamObject>) -> Result<(), RuntimeError> {
        let stream_ref = unsafe { stream_ptr.as_ref() };
        
        // Use locks to safely access and modify the stream data
        let mut buffer_guard = stream_ref.buffer.lock()
            .map_err(|_| RuntimeError::ConcurrencyError("Failed to acquire stream buffer lock".to_string()))?;
        let position_guard = stream_ref.buffer_position.lock()
            .map_err(|_| RuntimeError::ConcurrencyError("Failed to acquire stream position lock".to_string()))?;
        let mut generator_guard = stream_ref.generator.lock()
            .map_err(|_| RuntimeError::ConcurrencyError("Failed to acquire stream generator lock".to_string()))?;
        
        // Buffer more items if buffer is getting low
        let buffer_threshold = stream_ref.chunk_size / 2;
        if buffer_guard.len() - *position_guard < buffer_threshold {
            let items_to_generate = stream_ref.chunk_size;
            
            for _ in 0..items_to_generate {
                match &mut *generator_guard {
                    GeneratorFunction::Range { start, end, step } => {
                        // Honor the step direction — a descending range
                        // (negative step) never satisfies `start < end`
                        let in_range = if *step >= 0 { *start < *end } else { *start > *end };
                        if in_range {
                            let value = OvmValue::new_integer(*start);
                            buffer_guard.push(value);
                            *start += *step;
                        } else {
                            break; // End of range
                        }
                    }
                    GeneratorFunction::Map { source: _source, function: _function } => {
                        // Simplified - would need interpreter context for function calls
                        break;
                    }
                    GeneratorFunction::Filter { source: _source, predicate: _predicate } => {
                        // Simplified - would need interpreter context for predicate calls
                        break;
                    }
                    GeneratorFunction::Custom { function: _function } => {
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
        // Allocate string on heap and create GC pointer
        let heap_string = Box::into_raw(Box::new(value));
        let gc_ptr = GcPtr::new(heap_string);

        Self {
            header: ValueHeader::new(
                TypeTag::String,
                ExecutionTier::Interpreter,
                LazyState::Eager,
            ),
            data: ValueData::String(gc_ptr),
        }
    }

    /// Create a new list value using simplified GC allocation
    pub fn new_list(values: Vec<Self>) -> Self {
        // Convert Vec<OvmValue> to raw array allocation
        let len = values.len();
        let cap = len;
        let data_ptr = if len == 0 {
            std::ptr::null_mut()
        } else {
            let mut boxed_values = values.into_boxed_slice();
            let ptr = boxed_values.as_mut_ptr();
            std::mem::forget(boxed_values); // Prevent deallocation
            ptr
        };

        let value_array = ValueArray {
            length: len,
            capacity: cap,
            data: data_ptr,
        };

        let heap_array = Box::into_raw(Box::new(value_array));
        let gc_ptr = GcPtr::new(heap_array);

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
        let ptr = Box::into_raw(Box::new(builtin_obj));
        let gc_ptr = GcPtr::new(ptr);

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
            ValueData::Builtin(ptr) => unsafe { Some(&ptr.as_ref().name) },
            _ => None,
        }
    }

    /// Execute builtin function if this value is a builtin
    pub fn execute_builtin(&self, args: &[OvmValue]) -> Result<OvmValue, RuntimeError> {
        match &self.data {
            ValueData::Builtin(ptr) => unsafe { ptr.as_ref().execute(args) },
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

            Value::Function(func) => {
                // Create function object
                let func_obj = FunctionObject {
                    name: func.name.clone(),
                    parameters: func.parameters.iter().map(|p| p.name.clone()).collect(),
                    body: (*func.body).clone(),
                    closure: func
                        .closure
                        .iter()
                        .map(|(k, v)| (k.clone(), Self::from_ast(v.clone())))
                        .collect(),
                    compilation_tier: ExecutionTier::Interpreter,
                    call_count: AtomicU32::new(0),
                    optimization_data: OptimizationData::default(),
                };

                let ptr = Box::into_raw(Box::new(func_obj));
                let gc_ptr = GcPtr::new(ptr);

                Self {
                    header: ValueHeader::new(
                        TypeTag::Function,
                        ExecutionTier::Interpreter,
                        LazyState::Eager,
                    ),
                    data: ValueData::Function(gc_ptr),
                }
            }

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

                let boxed_range = match std::panic::catch_unwind(|| Box::new(range_obj)) {
                    Ok(boxed) => boxed,
                    Err(_) => {
                        // Box allocation failed, create a simple range value instead
                        return Self::new_unit(); // Fallback to unit value
                    }
                };
                let ptr = Box::into_raw(boxed_range);
                let gc_ptr = GcPtr::new(ptr);

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
                // Create struct object
                let struct_fields: HashMap<String, OvmValue> = fields
                    .into_iter()
                    .map(|(k, v)| (k, Self::from_ast(v)))
                    .collect();

                let struct_obj = StructObject {
                    type_name,
                    fields: struct_fields,
                };

                let boxed_struct = match std::panic::catch_unwind(|| Box::new(struct_obj)) {
                    Ok(boxed) => boxed,
                    Err(_) => return Self::new_unit(),
                };
                let ptr = Box::into_raw(boxed_struct);
                let gc_ptr = GcPtr::new(ptr);

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
                // For now, create a simple struct-like representation for enums
                // In a full implementation, we would have proper enum value support
                let mut fields = HashMap::new();
                fields.insert("__variant".to_string(), Self::new_string(variant_name.clone()));

                match variant_data {
                    crate::ast::EnumVariantData::Unit => {
                        // Unit variant has no additional fields
                    }
                    crate::ast::EnumVariantData::Tuple(values) => {
                        // Add tuple values as indexed fields
                        for (i, value) in values.iter().enumerate() {
                            fields.insert(format!("_{}", i), Self::from_ast(value.clone()));
                        }
                    }
                    crate::ast::EnumVariantData::Struct(struct_fields) => {
                        // Add struct fields directly
                        for (name, value) in struct_fields {
                            fields.insert(name, Self::from_ast(value.clone()));
                        }
                    }
                }

                let struct_obj = StructObject {
                    type_name: format!("{}::{}", type_name, variant_name),
                    fields,
                };

                let ptr = Box::into_raw(Box::new(struct_obj));
                let gc_ptr = GcPtr::new(ptr);

                Self {
                    header: ValueHeader::new(
                        TypeTag::Struct,
                        ExecutionTier::Interpreter,
                        LazyState::Eager,
                    ),
                    data: ValueData::Struct(gc_ptr),
                }
            }

            Value::Promise {
                state,
                value,
                error,
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

                let ptr = Box::into_raw(Box::new(promise_obj));
                let gc_ptr = GcPtr::new(ptr);

                Self {
                    header: ValueHeader::new(
                        TypeTag::Promise,
                        ExecutionTier::Interpreter,
                        LazyState::Eager,
                    ),
                    data: ValueData::Promise(gc_ptr),
                }
            }

            Value::Map(map) => {
                // Convert HashMap<String, Value> to OVM representation
                // For now, create a simple struct-like representation
                let mut fields = HashMap::new();
                for (key, value) in map.iter() {
                    fields.insert(key.clone(), Self::from_ast(value.clone()));
                }

                let struct_obj = StructObject {
                    type_name: "Map".to_string(),
                    fields,
                };

                let ptr = Box::into_raw(Box::new(struct_obj));
                let gc_ptr = GcPtr::new(ptr);

                Self {
                    header: ValueHeader::new(
                        TypeTag::Struct,
                        ExecutionTier::Interpreter,
                        LazyState::Eager,
                    ),
                    data: ValueData::Struct(gc_ptr),
                }
            }

            Value::TypeInfo { name, .. } => {
                // For now, represent types as string names
                Self::new_string(name)
            }
        }
    }

    /// Create OVM value from AST value with GC integration and safepoint coordination
    pub fn from_ast_with_gc(
        ast_value: Value, 
        safepoint_manager: &Arc<crate::ovm::gc::SafepointManager>
    ) -> Result<Self, RuntimeError> {
        // First check for safepoint before allocation
        safepoint_manager.check_safepoint();
        
        // Convert from AST using the standard method
        let mut ovm_value = Self::from_ast(ast_value);
        
        // Update GC metadata for proper tracking
        ovm_value.header.gc_bits.store(1, std::sync::atomic::Ordering::Relaxed); // Mark as allocated
        ovm_value.header.tier = ExecutionTier::Interpreter; // Start at interpreter tier
        
        // Record allocation with safepoint manager
        let allocation_size = std::mem::size_of::<OvmValue>() + 
            match &ovm_value.data {
                ValueData::String(s) => unsafe { s.as_ref().len() },
                ValueData::List(gc_ptr) => {
                    unsafe { (*gc_ptr.as_ptr()).length * std::mem::size_of::<OvmValue>() }
                }
                ValueData::Tuple(gc_ptr) => {
                    unsafe { (*gc_ptr.as_ptr()).length * std::mem::size_of::<OvmValue>() }
                }
                ValueData::Function(_) => std::mem::size_of::<FunctionObject>(),
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

    /// Create a new tuple value
    pub fn new_tuple(values: Vec<Self>) -> Self {
        let array = ValueArray {
            length: values.len(),
            capacity: values.len(),
            data: {
                let mut data = Vec::with_capacity(values.len());
                data.extend(values);
                Box::into_raw(data.into_boxed_slice()) as *mut Self
            },
        };

        let ptr = Box::into_raw(Box::new(array));
        let gc_ptr = GcPtr::new(ptr);

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
            ValueData::String(gc_ptr) => {
                // Convert GC string back to Arc<String>
                unsafe {
                    let string_ref = gc_ptr.as_ref();
                    Ok(Value::String(std::sync::Arc::new(string_ref.clone())))
                }
            }
            ValueData::List(gc_ptr) => {
                // Convert GC list back to Arc<[Value]>
                unsafe {
                    let array_ref = gc_ptr.as_ref();
                    let mut ast_values = Vec::with_capacity(array_ref.length);

                    if array_ref.length > 0 && !array_ref.data.is_null() && array_ref.length <= isize::MAX as usize {
                        let data_slice =
                            std::slice::from_raw_parts(array_ref.data, array_ref.length);
                        for ovm_val in data_slice {
                            ast_values.push(ovm_val.to_ast()?);
                        }
                    }

                    Ok(Value::List(ast_values.into()))
                }
            }
            ValueData::Tuple(gc_ptr) => {
                // Convert GC tuple back to Arc<Vec<Value>>
                unsafe {
                    let array_ref = gc_ptr.as_ref();
                    let mut ast_values = Vec::with_capacity(array_ref.length);

                    if array_ref.length > 0 && !array_ref.data.is_null() && array_ref.length <= isize::MAX as usize {
                        let data_slice =
                            std::slice::from_raw_parts(array_ref.data, array_ref.length);
                        for ovm_val in data_slice {
                            ast_values.push(ovm_val.to_ast()?);
                        }
                    }

                    Ok(Value::Tuple(std::sync::Arc::new(ast_values)))
                }
            }
            ValueData::Function(_) => {
                // Functions return unit for now
                Ok(Value::Unit)
            }
            ValueData::Struct(gc_ptr) => unsafe {
                let struct_ref = gc_ptr.as_ref();
                let mut fields = HashMap::new();
                for (name, val) in &struct_ref.fields {
                    fields.insert(name.clone(), val.to_ast()?);
                }
                Ok(Value::Struct {
                    type_name: struct_ref.type_name.clone(),
                    fields,
                })
            },
            ValueData::Range(gc_ptr) => {
                // Convert Range back to AST Range
                unsafe {
                    let range_ref = gc_ptr.as_ref();
                    Ok(Value::Range {
                        start: range_ref.start,
                        end: range_ref.end,
                        inclusive: range_ref.inclusive,
                    })
                }
            }
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
            ValueData::Error(gc_ptr) => unsafe {
                let error_ref = gc_ptr.as_ref();
                Ok(Value::Err(Box::new(Value::String(std::sync::Arc::new(
                    error_ref.message.clone(),
                )))))
            },
            ValueData::Result { ok, err } => {
                if let Some(ok_val) = ok {
                    Ok(Value::Ok(Box::new(ok_val.to_ast()?)))
                } else if let Some(err_val) = err {
                    Ok(Value::Err(Box::new(err_val.to_ast()?)))
                } else {
                    Err(RuntimeError::new("Result value has neither Ok nor Err"))
                }
            }
        }
    }
}

impl ValueHeader {
    pub fn new(type_tag: TypeTag, tier: ExecutionTier, lazy_state: LazyState) -> Self {
        Self {
            gc_bits: AtomicU32::new(0),
            type_tag,
            tier,
            optimization_data: 0,
            lazy_state,
            force_count: AtomicU32::new(0),
            ref_count: AtomicU32::new(1),
            gc_mark: false,
            age: 0,
            size: 0,
        }
    }

    pub fn mark_for_gc(&self) {
        // Set mark bit for garbage collection
        self.gc_bits.fetch_or(0x1, Ordering::Relaxed);
    }

    pub fn is_marked(&self) -> bool {
        (self.gc_bits.load(Ordering::Relaxed) & 0x1) != 0
    }

    pub fn clear_mark(&self) {
        // Clear mark bit
        self.gc_bits.fetch_and(!0x1, Ordering::Relaxed);
    }
}

impl<T> GcPtr<T> {
    pub fn new(ptr: *mut T) -> Self {
        Self {
            ptr: NonNull::new(ptr).expect("GcPtr::new called with null pointer"),
            generation: 0,
        }
    }
    
    pub fn try_new(ptr: *mut T) -> Result<Self, RuntimeError> {
        Ok(Self {
            ptr: NonNull::new(ptr).ok_or_else(|| RuntimeError::NullPointer)?,
            generation: 0,
        })
    }
    
    pub fn new_unchecked(ptr: *mut T) -> Self {
        Self {
            ptr: unsafe { NonNull::new_unchecked(ptr) },
            generation: 0,
        }
    }

    pub fn as_ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }

    pub unsafe fn as_ref(&self) -> &T {
        self.ptr.as_ref()
    }

    pub unsafe fn as_mut(&mut self) -> &mut T {
        self.ptr.as_mut()
    }
}

impl fmt::Display for OvmValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.data {
            ValueData::Integer(i) => write!(f, "{}", i),
            ValueData::Float(fl) => write!(f, "{}", fl),
            ValueData::Boolean(b) => write!(f, "{}", b),
            ValueData::Unit => write!(f, "()"),
            ValueData::String(gc_ptr) => unsafe {
                let string_ref = gc_ptr.as_ref();
                write!(f, "\"{}\"", string_ref)
            },
            ValueData::List(gc_ptr) => unsafe {
                let array_ref = gc_ptr.as_ref();
                write!(f, "[")?;

                if array_ref.length > 0 && !array_ref.data.is_null() && array_ref.length <= isize::MAX as usize {
                    let data_slice = std::slice::from_raw_parts(array_ref.data, array_ref.length);
                    for (i, val) in data_slice.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", val)?;
                    }
                }

                write!(f, "]")
            },
            ValueData::Tuple(gc_ptr) => unsafe {
                let array_ref = gc_ptr.as_ref();
                write!(f, "(")?;

                if array_ref.length > 0 && !array_ref.data.is_null() && array_ref.length <= isize::MAX as usize {
                    let data_slice = std::slice::from_raw_parts(array_ref.data, array_ref.length);
                    for (i, val) in data_slice.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", val)?;
                    }
                }

                write!(f, ")")
            },
            ValueData::Function(_) => write!(f, "<function>"),
            ValueData::Struct(_) => write!(f, "<struct>"),
            ValueData::Range(gc_ptr) => unsafe {
                let range_ref = gc_ptr.as_ref();
                if range_ref.inclusive {
                    write!(f, "{}..={}", range_ref.start, range_ref.end)
                } else {
                    write!(f, "{}..{}", range_ref.start, range_ref.end)
                }
            },
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
unsafe impl Send for OvmValue {}
unsafe impl Sync for OvmValue {}

impl Default for ValueHeader {
    fn default() -> Self {
        Self {
            gc_bits: AtomicU32::new(0),
            type_tag: TypeTag::Unit,
            tier: ExecutionTier::Interpreter,
            optimization_data: 0,
            lazy_state: LazyState::Eager,
            force_count: AtomicU32::new(0),
            ref_count: AtomicU32::new(1),
            gc_mark: false,
            age: 0,
            size: 0,
        }
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
        assert!(!header.is_marked());

        header.mark_for_gc();
        assert!(header.is_marked());

        header.clear_mark();
        assert!(!header.is_marked());
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

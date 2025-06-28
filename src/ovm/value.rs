//! OVM Unified Value System
//!
//! Provides a unified value representation that works across all execution tiers
//! with integrated garbage collection, lazy evaluation, and optimization metadata.

use std::collections::HashMap;
use std::fmt;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::ast::{Expr, Value as AstValue};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcPtr<T> {
    ptr: NonNull<T>,
    generation: u32,
}

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
    pub memoized_value: Option<OvmValue>,
    pub computation_cost: ComputationCost,
    pub dependencies: Vec<GcPtr<OvmValue>>,
}

/// Stream object for lazy sequences
#[derive(Debug)]
pub struct StreamObject {
    pub generator: GeneratorFunction,
    pub buffer: Vec<OvmValue>,
    pub buffer_position: usize,
    pub is_infinite: bool,
    pub chunk_size: usize,
}

/// Lazy list object
#[derive(Debug)]
pub struct LazyListObject {
    pub source: Box<OvmValue>,
    pub transformation: TransformationChain,
    pub materialized_prefix: Vec<OvmValue>,
    pub materialization_point: usize,
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

    #[error("Runtime error: {message}")]
    Generic { message: String },
}

// Implementation of core methods

impl Clone for OvmValue {
    fn clone(&self) -> Self {
        self.clone_simple()
    }
}

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

    /// Simple clone for basic value types
    pub fn clone_simple(&self) -> Self {
        match &self.data {
            ValueData::Integer(i) => Self::new_integer(*i),
            ValueData::Float(f) => Self::new_float(*f),
            ValueData::Boolean(b) => Self::new_boolean(*b),
            ValueData::Unit => Self::new_unit(),
            _ => {
                // For complex types, create a unit value as fallback
                Self::new_unit()
            }
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

        // Handle different lazy value types
        match self.header.lazy_state {
            LazyState::Lazy => {
                match &self.data {
                    ValueData::Thunk(_) => {
                        // TODO: Implement thunk forcing
                        Ok(())
                    }
                    ValueData::LazyList(_) => {
                        // TODO: Implement lazy list forcing
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }
            LazyState::Stream => {
                // Streams remain lazy but may buffer more data
                Ok(())
            }
            _ => Ok(()),
        }
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
    pub fn from_ast(ast_value: AstValue) -> Self {
        match ast_value {
            AstValue::Integer(n) => Self::new_integer(n),
            AstValue::Float(f) => Self::new_float(f),
            AstValue::Boolean(b) => Self::new_boolean(b),
            AstValue::String(s) => Self::new_string(s.as_ref().clone()),
            AstValue::Unit => Self::new_unit(),

            AstValue::List(items) => {
                let ovm_items: Vec<Self> = items
                    .iter()
                    .map(|item| Self::from_ast(item.clone()))
                    .collect();
                Self::new_list(ovm_items)
            }

            AstValue::Tuple(items) => {
                let ovm_items: Vec<Self> = items
                    .iter()
                    .map(|item| Self::from_ast(item.clone()))
                    .collect();
                Self::new_tuple(ovm_items)
            }

            AstValue::Function(func) => {
                // Create function object
                let func_obj = FunctionObject {
                    name: func.name.clone(),
                    parameters: func.parameters.iter().map(|p| p.name.clone()).collect(),
                    body: func.body,
                    closure: func
                        .closure
                        .into_iter()
                        .map(|(k, v)| (k, Self::from_ast(v)))
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

            AstValue::Builtin(builtin) => {
                // Create a placeholder builtin function
                // In a real implementation, this would map to actual builtin functions
                Self::new_builtin(builtin.name.clone(), builtin.arity, |_args| {
                    Err(RuntimeError::Generic {
                        message: "Builtin function not implemented in OVM".to_string(),
                    })
                })
            }

            AstValue::Ok(value) => Self {
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

            AstValue::Err(value) => Self {
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

            AstValue::Range {
                start,
                end: _,
                inclusive: _,
            } => {
                // For ranges, create a simple representation for now
                // TODO: Implement proper range representation
                Self::new_integer(start)
            }

            AstValue::Struct { type_name, fields } => {
                // Create struct object
                let struct_fields: HashMap<String, OvmValue> = fields
                    .into_iter()
                    .map(|(k, v)| (k, Self::from_ast(v)))
                    .collect();

                let struct_obj = StructObject {
                    type_name,
                    fields: struct_fields,
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

            AstValue::Promise {
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
        }
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
    pub fn to_ast(&self) -> Result<AstValue, RuntimeError> {
        match &self.data {
            ValueData::Integer(i) => Ok(AstValue::Integer(*i)),
            ValueData::Float(f) => Ok(AstValue::Float(*f)),
            ValueData::Boolean(b) => Ok(AstValue::Boolean(*b)),
            ValueData::Unit => Ok(AstValue::Unit),
            ValueData::String(gc_ptr) => {
                // Convert GC string back to Arc<String>
                unsafe {
                    let string_ref = gc_ptr.as_ref();
                    Ok(AstValue::String(std::sync::Arc::new(string_ref.clone())))
                }
            }
            ValueData::List(gc_ptr) => {
                // Convert GC list back to Arc<[Value]>
                unsafe {
                    let array_ref = gc_ptr.as_ref();
                    let mut ast_values = Vec::with_capacity(array_ref.length);

                    if array_ref.length > 0 && !array_ref.data.is_null() {
                        let data_slice =
                            std::slice::from_raw_parts(array_ref.data, array_ref.length);
                        for ovm_val in data_slice {
                            ast_values.push(ovm_val.to_ast()?);
                        }
                    }

                    Ok(AstValue::List(ast_values.into()))
                }
            }
            ValueData::Tuple(gc_ptr) => {
                // Convert GC tuple back to Arc<Vec<Value>>
                unsafe {
                    let array_ref = gc_ptr.as_ref();
                    let mut ast_values = Vec::with_capacity(array_ref.length);

                    if array_ref.length > 0 && !array_ref.data.is_null() {
                        let data_slice =
                            std::slice::from_raw_parts(array_ref.data, array_ref.length);
                        for ovm_val in data_slice {
                            ast_values.push(ovm_val.to_ast()?);
                        }
                    }

                    Ok(AstValue::Tuple(std::sync::Arc::new(ast_values)))
                }
            }
            ValueData::Function(_) => {
                // Functions return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::Struct(_) => {
                // Structs return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::Builtin(_) => {
                // Builtins return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::Thunk(_) => {
                // Thunks return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::Stream(_) => {
                // Streams return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::LazyList(_) => {
                // Lazy lists return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::Promise(_) => {
                // Promises return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::CompiledFunction(_) => {
                // Compiled functions return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::OptimizedValue(_) => {
                // Optimized values return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::Error(_) => {
                // Errors return unit for now
                Ok(AstValue::Unit)
            }
            ValueData::Result { ok: _, err: _ } => {
                // Results return unit for now
                Ok(AstValue::Unit)
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
            ptr: NonNull::new(ptr).unwrap(),
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

                if array_ref.length > 0 && !array_ref.data.is_null() {
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

                if array_ref.length > 0 && !array_ref.data.is_null() {
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
        let ast_int = AstValue::Integer(42);
        let ovm_int = OvmValue::from_ast(ast_int);
        assert_eq!(ovm_int.type_tag(), TypeTag::Integer);

        let converted_back = ovm_int.to_ast().unwrap();
        assert_eq!(converted_back, AstValue::Integer(42));
    }
}

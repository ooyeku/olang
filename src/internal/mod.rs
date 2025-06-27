use crate::ast::{Function, Value};
use crate::interpreter::{Interpreter, InterpreterError};
use std::sync::{Arc, Mutex};

/// Thread-safe version of Function for lazy evaluation
/// This avoids the Rc<> issues in the original Function struct
#[derive(Debug, Clone)]
pub(crate) struct ThreadSafeFunction {
    pub name: Option<String>,
    pub parameters: Vec<crate::ast::Parameter>,
    pub body_code: String, // Store as string to avoid Rc<> issues
    pub closure: std::collections::HashMap<String, Value>,
}

impl ThreadSafeFunction {
    /// Create a thread-safe function from a regular function
    pub fn from_function(func: &Function) -> Self {
        Self {
            name: func.name.clone(),
            parameters: func.parameters.clone(),
            body_code: serde_json::to_string(&func.body).unwrap_or_else(|_| "null".to_string()),
            closure: func.closure.clone(),
        }
    }

    /// Convert back to a regular function for evaluation
    pub fn to_function(&self) -> Function {
        let body =
            serde_json::from_str(&self.body_code).unwrap_or_else(|_| crate::ast::Expr::Integer(0)); // Fallback to 0 instead of 42

        Function {
            name: self.name.clone(),
            parameters: self.parameters.clone(),
            body,
            closure: self.closure.clone(),
        }
    }
}

// Thread safety implementations
unsafe impl Send for ThreadSafeFunction {}
unsafe impl Sync for ThreadSafeFunction {}

/// Internal representation that can hold either eager or lazy values
/// This is not visible to users - they only see the unified Value interface
#[derive(Debug, Clone)]
pub(crate) enum InternalValue {
    Eager(Value),    // Current eager values
    Lazy(LazyValue), // New lazy values
}

/// Lazy value variants for different types of deferred computation
#[derive(Clone)]
pub(crate) enum LazyValue {
    /// A generic thunk that can be evaluated later
    Thunk(Arc<dyn Fn(&mut Interpreter) -> Result<Value, InterpreterError> + Send + Sync>),
    /// Lazy range generation
    Range {
        start: i64,
        end: i64,
        step: i64,
        inclusive: bool,
    },
    /// Lazy mapped list - now using thread-safe function
    MappedList {
        source: Arc<InternalValue>,
        mapper: Arc<ThreadSafeFunction>,
    },
    /// Lazy filtered list - now using thread-safe function
    FilteredList {
        source: Arc<InternalValue>,
        predicate: Arc<ThreadSafeFunction>,
    },
    /// Lazy concatenated lists
    ConcatList {
        first: Arc<InternalValue>,
        second: Arc<InternalValue>,
    },
    /// Fused map+filter operation for optimization - now using thread-safe functions
    MapFiltered {
        source: Arc<InternalValue>,
        mapper: Arc<ThreadSafeFunction>,
        predicate: Arc<ThreadSafeFunction>,
    },
}

/// Thread-safe wrapper for internal values
/// This provides the transparent interface while hiding lazy evaluation details
#[derive(Debug, Clone)]
pub(crate) struct ValueHandle {
    inner: Arc<Mutex<InternalValue>>,
}

impl ValueHandle {
    /// Create a new handle with an eager value
    pub fn new_eager(value: Value) -> Self {
        Self {
            inner: Arc::new(Mutex::new(InternalValue::Eager(value))),
        }
    }

    /// Create a new handle with a lazy value
    pub fn new_lazy(lazy: LazyValue) -> Self {
        Self {
            inner: Arc::new(Mutex::new(InternalValue::Lazy(lazy))),
        }
    }

    /// Get the value, evaluating it if lazy and caching the result
    pub fn get(&self, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
        let mut guard = self.inner.lock().unwrap();
        match &*guard {
            InternalValue::Eager(value) => Ok(value.clone()),
            InternalValue::Lazy(lazy) => {
                // Evaluate and cache
                let value = lazy.evaluate(interpreter)?;
                *guard = InternalValue::Eager(value.clone());
                Ok(value)
            }
        }
    }

    /// Force evaluation without caching (for memory pressure scenarios)
    pub fn force(&self, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
        let guard = self.inner.lock().unwrap();
        match &*guard {
            InternalValue::Eager(value) => Ok(value.clone()),
            InternalValue::Lazy(lazy) => lazy.evaluate(interpreter),
        }
    }

    /// Check if this handle contains a lazy value
    pub fn is_lazy(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        matches!(&*guard, InternalValue::Lazy(_))
    }

    /// Get the internal value for optimization purposes
    pub fn get_internal(&self) -> Arc<InternalValue> {
        let guard = self.inner.lock().unwrap();
        Arc::new(guard.clone())
    }
}

impl std::fmt::Debug for LazyValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LazyValue::Thunk(_) => write!(f, "LazyValue::Thunk(<function>)"),
            LazyValue::Range {
                start,
                end,
                step,
                inclusive,
            } => f
                .debug_struct("LazyValue::Range")
                .field("start", start)
                .field("end", end)
                .field("step", step)
                .field("inclusive", inclusive)
                .finish(),
            LazyValue::MappedList { source, mapper } => f
                .debug_struct("LazyValue::MappedList")
                .field("source", source)
                .field("mapper", &mapper.name)
                .finish(),
            LazyValue::FilteredList { source, predicate } => f
                .debug_struct("LazyValue::FilteredList")
                .field("source", source)
                .field("predicate", &predicate.name)
                .finish(),
            LazyValue::ConcatList { first, second } => f
                .debug_struct("LazyValue::ConcatList")
                .field("first", first)
                .field("second", second)
                .finish(),
            LazyValue::MapFiltered {
                source,
                mapper,
                predicate,
            } => f
                .debug_struct("LazyValue::MapFiltered")
                .field("source", source)
                .field("mapper", &mapper.name)
                .field("predicate", &predicate.name)
                .finish(),
        }
    }
}

impl LazyValue {
    /// Evaluate this lazy value to produce an eager value
    pub fn evaluate(&self, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
        match self {
            LazyValue::Thunk(thunk) => thunk(interpreter),
            LazyValue::Range {
                start,
                end,
                step,
                inclusive,
            } => self.evaluate_range(*start, *end, *step, *inclusive),
            LazyValue::MappedList { source, mapper } => {
                self.evaluate_mapped_list(source, mapper, interpreter)
            }
            LazyValue::FilteredList { source, predicate } => {
                self.evaluate_filtered_list(source, predicate, interpreter)
            }
            LazyValue::ConcatList { first, second } => {
                self.evaluate_concat_list(first, second, interpreter)
            }
            LazyValue::MapFiltered {
                source,
                mapper,
                predicate,
            } => self.evaluate_map_filtered(source, mapper, predicate, interpreter),
        }
    }

    /// Evaluate a lazy range
    fn evaluate_range(
        &self,
        start: i64,
        end: i64,
        step: i64,
        inclusive: bool,
    ) -> Result<Value, InterpreterError> {
        let mut values = Vec::new();
        let mut current = start;

        if step > 0 {
            while current < end || (inclusive && current == end) {
                values.push(Value::Integer(current));
                if current == end && inclusive {
                    break;
                }
                current += step;
            }
        } else if step < 0 {
            while current > end || (inclusive && current == end) {
                values.push(Value::Integer(current));
                if current == end && inclusive {
                    break;
                }
                current += step;
            }
        }

        Ok(Value::List(Arc::from(values)))
    }

    /// Evaluate a lazy mapped list
    fn evaluate_mapped_list(
        &self,
        source: &Arc<InternalValue>,
        mapper: &Arc<ThreadSafeFunction>,
        interpreter: &mut Interpreter,
    ) -> Result<Value, InterpreterError> {
        // First force the source to get the actual list
        let source_value = InternalValue::force(source, interpreter)?;

        match source_value {
            Value::List(items) => {
                let mut results = Vec::new();
                for item in items.iter() {
                    let result = interpreter.call_function(
                        Value::Function((**mapper).to_function()),
                        vec![item.clone()],
                    )?;
                    results.push(result);
                }
                Ok(Value::List(Arc::from(results)))
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                // Convert range to vector and map
                let end_val = if inclusive { end + 1 } else { end };
                let mut results = Vec::new();
                for i in start..end_val {
                    let result = interpreter.call_function(
                        Value::Function((**mapper).to_function()),
                        vec![Value::Integer(i)],
                    )?;
                    results.push(result);
                }
                Ok(Value::List(Arc::from(results)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "Cannot map over non-list or non-range value".to_string(),
            }),
        }
    }

    /// Evaluate a lazy filtered list
    fn evaluate_filtered_list(
        &self,
        source: &Arc<InternalValue>,
        predicate: &Arc<ThreadSafeFunction>,
        interpreter: &mut Interpreter,
    ) -> Result<Value, InterpreterError> {
        let source_value = InternalValue::force(source, interpreter)?;

        match source_value {
            Value::List(items) => {
                let mut results = Vec::new();
                for item in items.iter() {
                    let pred_result = interpreter.call_function(
                        Value::Function((**predicate).to_function()),
                        vec![item.clone()],
                    )?;

                    if let Value::Boolean(true) = pred_result {
                        results.push(item.clone());
                    }
                }
                Ok(Value::List(Arc::from(results)))
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                // Convert range to vector and filter
                let end_val = if inclusive { end + 1 } else { end };
                let mut results = Vec::new();
                for i in start..end_val {
                    let item = Value::Integer(i);
                    let pred_result = interpreter.call_function(
                        Value::Function((**predicate).to_function()),
                        vec![item.clone()],
                    )?;

                    if let Value::Boolean(true) = pred_result {
                        results.push(item);
                    }
                }
                Ok(Value::List(Arc::from(results)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "Cannot filter non-list or non-range value".to_string(),
            }),
        }
    }

    /// Evaluate a lazy concatenated list
    fn evaluate_concat_list(
        &self,
        first: &Arc<InternalValue>,
        second: &Arc<InternalValue>,
        interpreter: &mut Interpreter,
    ) -> Result<Value, InterpreterError> {
        let first_value = InternalValue::force(first, interpreter)?;
        let second_value = InternalValue::force(second, interpreter)?;

        match (first_value, second_value) {
            (Value::List(first_items), Value::List(second_items)) => {
                let mut result = first_items.to_vec();
                result.extend(second_items.iter().cloned());
                Ok(Value::List(Arc::from(result)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "Cannot concatenate non-list values".to_string(),
            }),
        }
    }

    /// Evaluate a fused map+filter operation
    fn evaluate_map_filtered(
        &self,
        source: &Arc<InternalValue>,
        mapper: &Arc<ThreadSafeFunction>,
        predicate: &Arc<ThreadSafeFunction>,
        interpreter: &mut Interpreter,
    ) -> Result<Value, InterpreterError> {
        let source_value = InternalValue::force(source, interpreter)?;

        match source_value {
            Value::List(items) => {
                let mut results = Vec::new();
                for item in items.iter() {
                    // First apply the predicate
                    let pred_result = interpreter.call_function(
                        Value::Function((**predicate).to_function()),
                        vec![item.clone()],
                    )?;

                    if let Value::Boolean(true) = pred_result {
                        // Then apply the mapper
                        let mapped_result = interpreter.call_function(
                            Value::Function((**mapper).to_function()),
                            vec![item.clone()],
                        )?;
                        results.push(mapped_result);
                    }
                }
                Ok(Value::List(Arc::from(results)))
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                // Convert range to vector, filter, then map
                let end_val = if inclusive { end + 1 } else { end };
                let mut results = Vec::new();
                for i in start..end_val {
                    let item = Value::Integer(i);
                    // First apply the predicate
                    let pred_result = interpreter.call_function(
                        Value::Function((**predicate).to_function()),
                        vec![item.clone()],
                    )?;

                    if let Value::Boolean(true) = pred_result {
                        // Then apply the mapper
                        let mapped_result = interpreter
                            .call_function(Value::Function((**mapper).to_function()), vec![item])?;
                        results.push(mapped_result);
                    }
                }
                Ok(Value::List(Arc::from(results)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "Cannot map/filter non-list or non-range value".to_string(),
            }),
        }
    }

    /// Attempt to fuse this lazy value with another operation
    pub fn try_fuse(&self, other: &LazyValue) -> Option<LazyValue> {
        match (self, other) {
            // Fuse map followed by filter
            (
                LazyValue::MappedList { source, mapper },
                LazyValue::FilteredList { predicate, .. },
            ) => Some(LazyValue::MapFiltered {
                source: source.clone(),
                mapper: mapper.clone(),
                predicate: predicate.clone(),
            }),
            // Fuse filter followed by map
            (
                LazyValue::FilteredList { source, predicate },
                LazyValue::MappedList { mapper, .. },
            ) => Some(LazyValue::MapFiltered {
                source: source.clone(),
                mapper: mapper.clone(),
                predicate: predicate.clone(),
            }),
            _ => None,
        }
    }
}

impl InternalValue {
    /// Force evaluation of an internal value
    pub fn force(
        internal: &Arc<InternalValue>,
        interpreter: &mut Interpreter,
    ) -> Result<Value, InterpreterError> {
        match internal.as_ref() {
            InternalValue::Eager(value) => Ok(value.clone()),
            InternalValue::Lazy(lazy) => lazy.evaluate(interpreter),
        }
    }

    /// Convert a regular Value to an InternalValue
    pub fn from_value(value: Value) -> Self {
        InternalValue::Eager(value)
    }

    /// Convert an InternalValue to a ValueHandle
    pub fn into_handle(self) -> ValueHandle {
        ValueHandle {
            inner: Arc::new(Mutex::new(self)),
        }
    }
}

/// Configuration for lazy evaluation behavior
#[derive(Debug, Clone)]
pub struct LazyConfig {
    pub lazy_by_default: bool,
    pub lazy_threshold: usize,
    pub chunk_size: usize,
    pub memory_threshold_mb: usize,
    pub fusion_enabled: bool,
}

impl Default for LazyConfig {
    fn default() -> Self {
        Self {
            lazy_by_default: true,
            lazy_threshold: 100,
            chunk_size: 1024,
            memory_threshold_mb: 100,
            fusion_enabled: true,
        }
    }
}

/// Check if memory pressure suggests forcing lazy values
pub(crate) fn check_memory_pressure(threshold_mb: usize) -> bool {
    // Simple memory pressure check - in production this would be more sophisticated
    // For now, just return false to avoid forcing unless explicitly needed
    false
}

/// Force point functions - these functions require eager evaluation
pub(crate) fn is_force_point(function_name: &str) -> bool {
    matches!(
        function_name,
        "println"
            | "print"
            | "len"
            | "head"
            | "tail"
            | "sum"
            | "average"
            | "min"
            | "max"
            | "reverse"
            | "sort"
            | "join"
            | "contains"
            | "to_string"
            | "starts_with"
            | "ends_with"
    )
}

/// Lazy-aware function variants - these can work with lazy values
pub(crate) fn is_lazy_function(function_name: &str) -> bool {
    matches!(
        function_name,
        "map"
            | "filter"
            | "take"
            | "skip"
            | "concat"
            | "zip"
            | "enumerate"
            | "chunk"
            | "flatten"
            | "group_by"
            | "find"
    )
}

/// Create a lazy map operation
pub(crate) fn create_lazy_map(source: ValueHandle, mapper: crate::ast::Function) -> LazyValue {
    LazyValue::MappedList {
        source: source.get_internal(),
        mapper: Arc::new(ThreadSafeFunction::from_function(&mapper)),
    }
}

/// Create a lazy filter operation  
pub(crate) fn create_lazy_filter(
    source: ValueHandle,
    predicate: crate::ast::Function,
) -> LazyValue {
    LazyValue::FilteredList {
        source: source.get_internal(),
        predicate: Arc::new(ThreadSafeFunction::from_function(&predicate)),
    }
}

/// Create a lazy range
pub(crate) fn create_lazy_range(start: i64, end: i64, step: i64, inclusive: bool) -> LazyValue {
    LazyValue::Range {
        start,
        end,
        step,
        inclusive,
    }
}

/// Fusion optimization - combine compatible lazy operations
pub(crate) fn try_fuse_operations(
    lazy_val: &LazyValue,
    new_op: &str,
    function: Option<crate::ast::Function>,
) -> Option<LazyValue> {
    match (lazy_val, new_op) {
        // Map + Filter -> MapFiltered
        (LazyValue::MappedList { source, mapper }, "filter") => {
            function.map(|predicate| LazyValue::MapFiltered {
                source: source.clone(),
                mapper: mapper.clone(),
                predicate: Arc::new(ThreadSafeFunction::from_function(&predicate)),
            })
        }
        // Filter + Map -> MapFiltered
        (LazyValue::FilteredList { source, predicate }, "map") => {
            function.map(|mapper| LazyValue::MapFiltered {
                source: source.clone(),
                mapper: Arc::new(ThreadSafeFunction::from_function(&mapper)),
                predicate: predicate.clone(),
            })
        }
        _ => None,
    }
}

/// Take first N elements from a lazy sequence
pub(crate) fn create_lazy_take(source: ValueHandle, n: usize) -> ValueHandle {
    let source_internal = source.get_internal();
    let lazy_val = LazyValue::Thunk(Arc::new(move |interpreter| {
        let source_value = InternalValue::force(&source_internal, interpreter)?;
        match source_value {
            crate::ast::Value::List(items) => {
                let taken: Vec<_> = items.iter().take(n).cloned().collect();
                Ok(crate::ast::Value::List(taken.into()))
            }
            _ => Err(crate::interpreter::InterpreterError::TypeError {
                message: "take: argument must be a list".to_string(),
            }),
        }
    }));
    ValueHandle::new_lazy(lazy_val)
}

/// Skip first N elements from a lazy sequence  
pub(crate) fn create_lazy_skip(source: ValueHandle, n: usize) -> ValueHandle {
    let source_internal = source.get_internal();
    let lazy_val = LazyValue::Thunk(Arc::new(move |interpreter| {
        let source_value = InternalValue::force(&source_internal, interpreter)?;
        match source_value {
            crate::ast::Value::List(items) => {
                let skipped: Vec<_> = items.iter().skip(n).cloned().collect();
                Ok(crate::ast::Value::List(skipped.into()))
            }
            _ => Err(crate::interpreter::InterpreterError::TypeError {
                message: "skip: argument must be a list".to_string(),
            }),
        }
    }));
    ValueHandle::new_lazy(lazy_val)
}

/// Utility functions for working with lazy values
pub(crate) mod utils {
    use super::*;

    /// Check if a Value should be treated as lazy based on configuration
    pub fn should_be_lazy(value: &Value, config: &LazyConfig) -> bool {
        if !config.lazy_by_default {
            return false;
        }

        match value {
            Value::List(items) => items.len() >= config.lazy_threshold,
            _ => false,
        }
    }

    /// Convert a Value to a ValueHandle, making it lazy if appropriate
    pub fn value_to_handle(value: Value, config: &LazyConfig) -> ValueHandle {
        if should_be_lazy(&value, config) {
            // For now, just wrap as eager - lazy conversion happens at operation level
            ValueHandle::new_eager(value)
        } else {
            ValueHandle::new_eager(value)
        }
    }

    /// Force evaluation of a value if it's lazy
    pub fn force_if_lazy(value: Value) -> Value {
        // Since we're working with the public Value interface,
        // this is a no-op for now. The lazy evaluation happens
        // at the interpreter level with ValueHandle
        value
    }
}

// Include tests
mod tests;

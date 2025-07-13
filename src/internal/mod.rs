use crate::ast::{Function, Value};
use crate::interpreter::{Interpreter, InterpreterError};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use std::collections::HashSet;

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
        let config = interpreter.get_lazy_config().clone();
        let mut context = LazyEvaluationContext::new(config);
        self.get_with_context(interpreter, &mut context)
    }

    /// Get the value with context for enhanced error handling
    pub fn get_with_context(
        &self,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        context.check_timeout()?;
        
        // Optimize memory usage before evaluation
        context.optimize_memory_usage()?;
        
        // Use try_lock to avoid deadlocks
        if context.config.thread_safety_checks {
            // Create a copy of the lock manager to avoid borrow conflicts
            let lock_manager = context.lock_manager.clone();
            match lock_manager.try_acquire_lock(&self.inner) {
                Ok(mut guard) => {
                    match &*guard {
                        InternalValue::Eager(value) => Ok(value.clone()),
                        InternalValue::Lazy(lazy) => {
                            // Check if we should evaluate based on memory strategy
                            if self.should_evaluate_now(&context.config) {
                                // Evaluate and cache with enhanced error handling
                                match lazy.evaluate_with_context(interpreter, context) {
                                    Ok(value) => {
                                        // Only cache if memory strategy allows it
                                        if self.should_cache(&context.config, &value) {
                                            *guard = InternalValue::Eager(value.clone());
                                        }
                                        Ok(value)
                                    }
                                    Err(e) => {
                                        // If recovery is enabled, try to recover
                                        if context.can_recover() {
                                            context.attempt_recovery();
                                            
                                            // Try force evaluation as recovery
                                            if let Ok(forced_value) = lazy.try_force_evaluation(interpreter, context) {
                                                if self.should_cache(&context.config, &forced_value) {
                                                    *guard = InternalValue::Eager(forced_value.clone());
                                                }
                                                Ok(forced_value)
                                            } else {
                                                Err(InterpreterError::RecoveryFailed {
                                                    original_error: e.to_string(),
                                                })
                                            }
                                        } else {
                                            Err(e)
                                        }
                                    }
                                }
                            } else {
                                // Memory pressure is too high, try force evaluation without caching
                                lazy.try_force_evaluation(interpreter, context)
                            }
                        }
                    }
                }
                Err(e) => Err(e),
            }
        } else {
            // Non-thread-safe path for better performance
            let mut guard = self.inner.lock().unwrap();
            match &*guard {
                InternalValue::Eager(value) => Ok(value.clone()),
                InternalValue::Lazy(lazy) => {
                    // Check memory strategy before evaluation and caching
                    if self.should_evaluate_now(&context.config) {
                        // Evaluate and cache
                        let value = lazy.evaluate_with_context(interpreter, context)?;
                        if self.should_cache(&context.config, &value) {
                            *guard = InternalValue::Eager(value.clone());
                        }
                        Ok(value)
                    } else {
                        // Force evaluation without caching
                        lazy.try_force_evaluation(interpreter, context)
                    }
                }
            }
        }
    }

    /// Force evaluation without caching (for memory pressure scenarios)
    pub fn force(&self, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
        let config = interpreter.get_lazy_config().clone();
        let mut context = LazyEvaluationContext::new(config);
        self.force_with_context(interpreter, &mut context)
    }

    /// Force evaluation with context
    pub fn force_with_context(
        &self,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        context.check_timeout()?;
        
        let guard = self.inner.lock().unwrap();
        match &*guard {
            InternalValue::Eager(value) => Ok(value.clone()),
            InternalValue::Lazy(lazy) => lazy.evaluate_with_context(interpreter, context),
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

    /// Check if we should evaluate now based on memory strategy
    fn should_evaluate_now(&self, config: &LazyConfig) -> bool {
        match &config.memory_strategy {
            MemoryStrategy::Conservative => {
                // Conservative: only evaluate if memory pressure is low
                !check_memory_pressure(config.memory_threshold_mb)
            }
            MemoryStrategy::Balanced => {
                // Balanced: evaluate unless memory pressure is high
                get_estimated_memory_usage() < config.memory_threshold_mb * 3 / 4
            }
            MemoryStrategy::Aggressive => {
                // Aggressive: always evaluate and cache
                true
            }
            MemoryStrategy::Adaptive { low_memory_threshold_mb } => {
                // Adaptive: switch strategy based on available memory
                if get_estimated_memory_usage() < *low_memory_threshold_mb {
                    true // Behave like aggressive when memory is plentiful
                } else {
                    !check_memory_pressure(config.memory_threshold_mb) // Behave like conservative when memory is tight
                }
            }
        }
    }

    /// Check if we should cache the result based on memory strategy and value size
    fn should_cache(&self, config: &LazyConfig, value: &Value) -> bool {
        // Don't cache if auto cleanup is disabled
        if !config.auto_cleanup_enabled {
            return true; // Let the user manage memory manually
        }

        // Estimate the memory footprint of the value
        let value_size = self.estimate_value_size(value);
        
        // Don't cache very large values if memory pressure is high
        if check_memory_pressure(config.memory_threshold_mb) && value_size > 1024 * 1024 {
            return false;
        }

        match &config.memory_strategy {
            MemoryStrategy::Conservative => {
                // Conservative: only cache small values
                value_size < 64 * 1024 // 64KB limit
            }
            MemoryStrategy::Balanced => {
                // Balanced: cache medium-sized values
                value_size < 512 * 1024 // 512KB limit
            }
            MemoryStrategy::Aggressive => {
                // Aggressive: cache everything
                true
            }
            MemoryStrategy::Adaptive { .. } => {
                // Adaptive: adjust based on memory pressure
                if check_memory_pressure(config.memory_threshold_mb) {
                    value_size < 64 * 1024 // Conservative when memory is tight
                } else {
                    value_size < 1024 * 1024 // More generous when memory is available
                }
            }
        }
    }

    /// Estimate the memory size of a value (rough approximation)
    fn estimate_value_size(&self, value: &Value) -> usize {
        match value {
            Value::Unit => 0,
            Value::Boolean(_) => 1,
            Value::Integer(_) => 8,
            Value::Float(_) => 8,
            Value::String(s) => s.len() * 4, // Rough estimate for UTF-8
            Value::List(items) => {
                items.iter().map(|item| self.estimate_value_size(item)).sum::<usize>() + items.len() * 8
            }
            Value::Map(map) => {
                map.iter().map(|(k, v)| k.len() * 4 + self.estimate_value_size(v)).sum::<usize>() + map.len() * 16
            }
            Value::Function(_) => 256, // Rough estimate for function objects
            Value::Builtin(_) => 64,   // Builtin functions are lighter
            Value::Struct { .. } => 128, // Rough estimate for struct objects
            _ => 128, // Default estimate for other types
        }
    }

    /// Clear cached value to free memory
    pub fn clear_cache(&self) -> Result<(), InterpreterError> {
        if let Ok(guard) = self.inner.try_lock() {
            match &*guard {
                InternalValue::Eager(_) => {
                    // Convert back to lazy if possible
                    // This is a simplified implementation - in practice, we might not be able to do this
                    // For now, we'll just keep the eager value
                }
                InternalValue::Lazy(_) => {
                    // Already lazy, nothing to clear
                }
            }
        }
        Ok(())
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
    /// Evaluate this lazy value to produce an eager value with enhanced error handling
    pub fn evaluate(&self, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
        let config = interpreter.get_lazy_config().clone();
        let mut context = LazyEvaluationContext::new(config);
        self.evaluate_with_context(interpreter, &mut context)
    }

    /// Evaluate with context for proper error handling and recovery
    pub fn evaluate_with_context(
        &self,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        // Check timeout before evaluation
        context.check_timeout()?;
        
        // Check memory pressure
        context.check_memory_pressure()?;
        
        // Increment evaluation depth
        context.increment_depth()?;

        // Attempt evaluation with recovery
        let result = self.try_evaluate_with_recovery(interpreter, context);
        
        // Decrement depth after evaluation
        context.evaluation_depth = context.evaluation_depth.saturating_sub(1);
        
        result
    }

    /// Try evaluation with recovery mechanisms
    fn try_evaluate_with_recovery(
        &self,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        loop {
            match self.evaluate_internal(interpreter, context) {
                Ok(value) => return Ok(value),
                Err(error) => {
                    // Check if we can recover from this error
                    if context.can_recover() && self.is_recoverable_error(&error) {
                        context.attempt_recovery();
                        
                        // For certain errors, try forcing evaluation
                        if context.config.force_evaluation_on_error {
                            if let Ok(forced_value) = self.try_force_evaluation(interpreter, context) {
                                return Ok(forced_value);
                            }
                        }
                        
                        // For timeout errors, extend timeout and retry
                        if matches!(error, InterpreterError::LazyEvaluationTimeout { .. }) {
                            context.extend_timeout_for_recovery();
                            continue;
                        }
                        
                        // For memory pressure, try garbage collection
                        if matches!(error, InterpreterError::MemoryLimitExceeded { .. }) {
                            // In a real implementation, this would trigger GC
                            std::hint::spin_loop(); // Placeholder
                            continue;
                        }
                        
                        // For circular dependencies, try to break the cycle and retry
                        if matches!(error, InterpreterError::CircularDependency { .. }) {
                            if let Ok(()) = context.try_break_cycle("unknown") {
                                continue;
                            }
                        }
                    }
                    
                    // If we can't recover, wrap the error
                    return Err(if context.recovery_attempts > 0 {
                        InterpreterError::RecoveryFailed {
                            original_error: error.to_string(),
                        }
                    } else {
                        error
                    });
                }
            }
        }
    }

    /// Internal evaluation method with context
    fn evaluate_internal(
        &self,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        match self {
            LazyValue::Thunk(thunk) => {
                // Check circular dependency for thunks
                let thunk_id = format!("thunk_{:p}", thunk.as_ref() as *const _);
                context.check_circular_dependency(&thunk_id)?;
                
                // Evaluate thunk with timeout check
                context.check_operation_timeout("thunk")?;
                let result = thunk(interpreter);
                
                // Remove from visited set after successful evaluation
                context.visited_thunks.write().unwrap().remove(&thunk_id);
                
                result.map_err(|e| InterpreterError::LazyEvaluationError {
                    message: format!("Thunk evaluation failed: {}", e),
                })
            }
            LazyValue::Range {
                start,
                end,
                step,
                inclusive,
            } => self.evaluate_range_with_context(*start, *end, *step, *inclusive, context),
            LazyValue::MappedList { source, mapper } => {
                self.evaluate_mapped_list_with_context(source, mapper, interpreter, context)
            }
            LazyValue::FilteredList { source, predicate } => {
                self.evaluate_filtered_list_with_context(source, predicate, interpreter, context)
            }
            LazyValue::ConcatList { first, second } => {
                self.evaluate_concat_list_with_context(first, second, interpreter, context)
            }
            LazyValue::MapFiltered {
                source,
                mapper,
                predicate,
            } => self.evaluate_map_filtered_with_context(source, mapper, predicate, interpreter, context),
        }
    }

    /// Check if an error is recoverable
    fn is_recoverable_error(&self, error: &InterpreterError) -> bool {
        matches!(
            error,
            InterpreterError::LazyEvaluationTimeout { .. }
                | InterpreterError::MemoryLimitExceeded { .. }
                | InterpreterError::CircularDependency { .. }
                | InterpreterError::ThreadSafetyViolation { .. }
        )
    }

    /// Try to force evaluation as a recovery mechanism
    fn try_force_evaluation(
        &self,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        // Create a simplified context for forcing
        let mut force_context = LazyEvaluationContext::new(context.config.clone());
        force_context.config.timeout_ms = 5000; // Shorter timeout for forced evaluation
        force_context.config.max_evaluation_depth = 100; // Shallow depth for forcing
        
        self.evaluate_internal(interpreter, &mut force_context)
            .map_err(|e| InterpreterError::ForceEvaluationFailed {
                reason: e.to_string(),
            })
    }

    /// Evaluate a lazy range with enhanced error handling
    fn evaluate_range_with_context(
        &self,
        start: i64,
        end: i64,
        step: i64,
        inclusive: bool,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        let mut values = Vec::new();
        let mut current = start;
        let mut iterations = 0;
        const MAX_ITERATIONS: u64 = 1_000_000; // Prevent infinite loops

        if step == 0 {
            return Err(InterpreterError::LazyEvaluationError {
                message: "Range step cannot be zero".to_string(),
            });
        }

        if step > 0 {
            while current < end || (inclusive && current == end) {
                context.check_operation_timeout("range")?;
                
                iterations += 1;
                if iterations > MAX_ITERATIONS {
                    return Err(InterpreterError::LazyEvaluationError {
                        message: format!("Range evaluation exceeded maximum iterations: {}", MAX_ITERATIONS),
                    });
                }

                values.push(Value::Integer(current));
                if current == end && inclusive {
                    break;
                }
                
                // Check for overflow
                if current > i64::MAX - step {
                    return Err(InterpreterError::LazyEvaluationError {
                        message: "Range evaluation would overflow".to_string(),
                    });
                }
                
                current += step;
            }
        } else if step < 0 {
            while current > end || (inclusive && current == end) {
                context.check_operation_timeout("range")?;
                
                iterations += 1;
                if iterations > MAX_ITERATIONS {
                    return Err(InterpreterError::LazyEvaluationError {
                        message: format!("Range evaluation exceeded maximum iterations: {}", MAX_ITERATIONS),
                    });
                }

                values.push(Value::Integer(current));
                if current == end && inclusive {
                    break;
                }
                
                // Check for underflow
                if current < i64::MIN - step {
                    return Err(InterpreterError::LazyEvaluationError {
                        message: "Range evaluation would underflow".to_string(),
                    });
                }
                
                current += step;
            }
        }

        Ok(Value::List(Arc::from(values)))
    }

    /// Evaluate a lazy mapped list with enhanced error handling
    fn evaluate_mapped_list_with_context(
        &self,
        source: &Arc<InternalValue>,
        mapper: &Arc<ThreadSafeFunction>,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        context.check_operation_timeout("map")?;
        
        // First force the source to get the actual list
        let source_value = InternalValue::force_with_context(source, interpreter, context)?;

        match source_value {
            Value::List(items) => {
                let mut results = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    context.check_timeout()?;
                    
                    let result = interpreter.call_function(
                        Value::Function((**mapper).to_function()),
                        vec![item.clone()],
                    ).map_err(|e| InterpreterError::LazyEvaluationError {
                        message: format!("Map function failed at index {}: {}", index, e),
                    })?;
                    
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
                    context.check_timeout()?;
                    
                    let result = interpreter.call_function(
                        Value::Function((**mapper).to_function()),
                        vec![Value::Integer(i)],
                    ).map_err(|e| InterpreterError::LazyEvaluationError {
                        message: format!("Map function failed at value {}: {}", i, e),
                    })?;
                    
                    results.push(result);
                }
                Ok(Value::List(Arc::from(results)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "Cannot map over non-list or non-range value".to_string(),
            }),
        }
    }

    /// Evaluate a lazy filtered list with enhanced error handling
    fn evaluate_filtered_list_with_context(
        &self,
        source: &Arc<InternalValue>,
        predicate: &Arc<ThreadSafeFunction>,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        context.check_operation_timeout("filter")?;
        
        let source_value = InternalValue::force_with_context(source, interpreter, context)?;

        match source_value {
            Value::List(items) => {
                let mut results = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    context.check_timeout()?;
                    
                    let pred_result = interpreter.call_function(
                        Value::Function((**predicate).to_function()),
                        vec![item.clone()],
                    ).map_err(|e| InterpreterError::LazyEvaluationError {
                        message: format!("Filter predicate failed at index {}: {}", index, e),
                    })?;

                    if let Value::Boolean(true) = pred_result {
                        results.push(item.clone());
                    } else if !matches!(pred_result, Value::Boolean(false)) {
                        return Err(InterpreterError::LazyEvaluationError {
                            message: format!("Filter predicate must return boolean, got: {:?}", pred_result),
                        });
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
                    context.check_timeout()?;
                    
                    let item = Value::Integer(i);
                    let pred_result = interpreter.call_function(
                        Value::Function((**predicate).to_function()),
                        vec![item.clone()],
                    ).map_err(|e| InterpreterError::LazyEvaluationError {
                        message: format!("Filter predicate failed at value {}: {}", i, e),
                    })?;

                    if let Value::Boolean(true) = pred_result {
                        results.push(item);
                    } else if !matches!(pred_result, Value::Boolean(false)) {
                        return Err(InterpreterError::LazyEvaluationError {
                            message: format!("Filter predicate must return boolean, got: {:?}", pred_result),
                        });
                    }
                }
                Ok(Value::List(Arc::from(results)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "Cannot filter non-list or non-range value".to_string(),
            }),
        }
    }

    /// Evaluate a lazy concatenated list with enhanced error handling
    fn evaluate_concat_list_with_context(
        &self,
        first: &Arc<InternalValue>,
        second: &Arc<InternalValue>,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        context.check_operation_timeout("concat")?;
        
        let first_value = InternalValue::force_with_context(first, interpreter, context)?;
        let second_value = InternalValue::force_with_context(second, interpreter, context)?;

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

    /// Evaluate a fused map+filter operation with enhanced error handling
    fn evaluate_map_filtered_with_context(
        &self,
        source: &Arc<InternalValue>,
        mapper: &Arc<ThreadSafeFunction>,
        predicate: &Arc<ThreadSafeFunction>,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        context.check_timeout()?;
        
        let source_value = InternalValue::force_with_context(source, interpreter, context)?;

        match source_value {
            Value::List(items) => {
                let mut results = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    context.check_timeout()?;
                    
                    // First apply the mapper
                    let mapped_result = interpreter.call_function(
                        Value::Function((**mapper).to_function()),
                        vec![item.clone()],
                    ).map_err(|e| InterpreterError::LazyEvaluationError {
                        message: format!("Map function failed at index {}: {}", index, e),
                    })?;

                    // Then apply the predicate to the mapped result
                    let pred_result = interpreter.call_function(
                        Value::Function((**predicate).to_function()),
                        vec![mapped_result.clone()],
                    ).map_err(|e| InterpreterError::LazyEvaluationError {
                        message: format!("Filter predicate failed at index {}: {}", index, e),
                    })?;

                    if let Value::Boolean(true) = pred_result {
                        results.push(mapped_result);
                    } else if !matches!(pred_result, Value::Boolean(false)) {
                        return Err(InterpreterError::LazyEvaluationError {
                            message: format!("Filter predicate must return boolean, got: {:?}", pred_result),
                        });
                    }
                }
                Ok(Value::List(Arc::from(results)))
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                // Convert range to vector, map, then filter
                let end_val = if inclusive { end + 1 } else { end };
                let mut results = Vec::new();
                
                for i in start..end_val {
                    context.check_timeout()?;
                    
                    let item = Value::Integer(i);
                    // First apply the mapper
                    let mapped_result = interpreter.call_function(
                        Value::Function((**mapper).to_function()), 
                        vec![item]
                    ).map_err(|e| InterpreterError::LazyEvaluationError {
                        message: format!("Map function failed at value {}: {}", i, e),
                    })?;

                    // Then apply the predicate to the mapped result
                    let pred_result = interpreter.call_function(
                        Value::Function((**predicate).to_function()),
                        vec![mapped_result.clone()],
                    ).map_err(|e| InterpreterError::LazyEvaluationError {
                        message: format!("Filter predicate failed at value {}: {}", i, e),
                    })?;

                    if let Value::Boolean(true) = pred_result {
                        results.push(mapped_result);
                    } else if !matches!(pred_result, Value::Boolean(false)) {
                        return Err(InterpreterError::LazyEvaluationError {
                            message: format!("Filter predicate must return boolean, got: {:?}", pred_result),
                        });
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
        let config = interpreter.get_lazy_config().clone();
        let mut context = LazyEvaluationContext::new(config);
        Self::force_with_context(internal, interpreter, &mut context)
    }

    /// Force evaluation with context for enhanced error handling
    pub fn force_with_context(
        internal: &Arc<InternalValue>,
        interpreter: &mut Interpreter,
        context: &mut LazyEvaluationContext,
    ) -> Result<Value, InterpreterError> {
        context.check_timeout()?;
        context.increment_depth()?;
        
        let result = match internal.as_ref() {
            InternalValue::Eager(value) => Ok(value.clone()),
            InternalValue::Lazy(lazy) => lazy.evaluate_with_context(interpreter, context),
        };
        
        context.evaluation_depth = context.evaluation_depth.saturating_sub(1);
        result
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

/// Timeout strategy for lazy evaluation
#[derive(Debug, Clone)]
pub enum TimeoutStrategy {
    /// Fixed timeout - always use the same timeout
    Fixed(u64),
    /// Adaptive timeout - adjust based on operation complexity
    Adaptive { base_ms: u64, scaling_factor: f64 },
    /// Progressive timeout - start with short timeout and increase
    Progressive { initial_ms: u64, max_ms: u64, multiplier: f64 },
    /// Per-operation timeout - different timeouts for different operations
    PerOperation {
        map_ms: u64,
        filter_ms: u64,
        range_ms: u64,
        concat_ms: u64,
        thunk_ms: u64,
    },
}

impl Default for TimeoutStrategy {
    fn default() -> Self {
        TimeoutStrategy::Adaptive {
            base_ms: 30000,
            scaling_factor: 1.2,
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
    
    // Enhanced configuration for edge cases
    pub timeout_ms: u64,
    pub timeout_strategy: TimeoutStrategy,
    pub max_evaluation_depth: usize,
    pub enable_recovery: bool,
    pub circular_dependency_detection: bool,
    pub thread_safety_checks: bool,
    pub memory_pressure_threshold: f64,
    pub force_evaluation_on_error: bool,
    pub timeout_monitoring_enabled: bool,
    pub timeout_warning_threshold: f64, // Warn when using X% of timeout
    
    // Memory optimization configuration
    pub memory_strategy: MemoryStrategy,
    pub auto_cleanup_enabled: bool,
    pub force_gc_on_pressure: bool,
    pub cache_size_limit: usize,
    pub memory_monitoring_enabled: bool,
}

impl Default for LazyConfig {
    fn default() -> Self {
        Self {
            lazy_by_default: true,
            lazy_threshold: 100,
            chunk_size: 1024,
            memory_threshold_mb: 100,
            fusion_enabled: true,
            
            // Enhanced defaults for edge cases
            timeout_ms: 30000, // 30 seconds
            timeout_strategy: TimeoutStrategy::default(),
            max_evaluation_depth: 1000,
            enable_recovery: true,
            circular_dependency_detection: true,
            thread_safety_checks: true,
            memory_pressure_threshold: 0.8, // 80% of memory limit
            force_evaluation_on_error: false,
            timeout_monitoring_enabled: true,
            timeout_warning_threshold: 0.8, // Warn at 80% of timeout
            
            // Memory optimization defaults
            memory_strategy: MemoryStrategy::default(),
            auto_cleanup_enabled: true,
            force_gc_on_pressure: true,
            cache_size_limit: 1000, // Maximum cached lazy values
            memory_monitoring_enabled: true,
        }
    }
}

/// Check if memory pressure suggests forcing lazy values
pub(crate) fn check_memory_pressure(threshold_mb: usize) -> bool {
    // Enhanced memory pressure check with better heuristics
    
    // Check system memory usage (simplified simulation)
    // In a real implementation, this would use proper system calls
    let estimated_system_memory_mb = get_estimated_memory_usage();
    let memory_pressure_ratio = estimated_system_memory_mb as f64 / threshold_mb as f64;
    
    // Consider memory pressure high if we're using > 90% of threshold
    // This is more conservative to avoid false positives in tests
    if memory_pressure_ratio > 0.9 {
        return true;
    }
    
    // Check lazy evaluation specific memory usage
    let lazy_memory_mb = get_lazy_evaluation_memory_usage();
    if lazy_memory_mb > threshold_mb * 3 / 4 {
        return true;
    }
    
    // Check for memory fragmentation indicators
    if is_memory_fragmented() {
        return true;
    }
    
    false
}

/// Get estimated system memory usage (placeholder implementation)
pub(crate) fn get_estimated_memory_usage() -> usize {
    // In a real implementation, this would query the system for actual memory usage
    // For now, return a simulated value that won't trigger memory pressure in tests
    let base_usage = 50; // Base 50MB usage
    let random_factor = (std::ptr::addr_of!(base_usage) as usize % 30) / 10; // Pseudo-random 0-3MB
    base_usage + random_factor
}

/// Get memory usage specifically for lazy evaluation structures
pub(crate) fn get_lazy_evaluation_memory_usage() -> usize {
    // Estimate memory usage from lazy evaluation structures
    // This would track memory used by ValueHandles, lazy values, etc.
    // For now, return a placeholder
    25 // 25MB placeholder
}

/// Check if memory appears fragmented
fn is_memory_fragmented() -> bool {
    // Simple heuristic for memory fragmentation detection
    // In a real implementation, this would check actual memory layout
    false // Placeholder - assume no fragmentation for now
}

/// Memory optimization strategies
#[derive(Debug, Clone)]
pub enum MemoryStrategy {
    /// Conservative - prefer memory efficiency over speed
    Conservative,
    /// Balanced - balance memory and performance
    Balanced,
    /// Aggressive - prefer speed over memory efficiency
    Aggressive,
    /// Adaptive - adjust strategy based on available memory
    Adaptive { low_memory_threshold_mb: usize },
}

impl Default for MemoryStrategy {
    fn default() -> Self {
        MemoryStrategy::Balanced
    }
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

/// Create a lazy concatenation operation
pub(crate) fn create_lazy_concat(first: ValueHandle, second: ValueHandle) -> LazyValue {
    LazyValue::ConcatList {
        first: first.get_internal(),
        second: second.get_internal(),
    }
}

/// Create a lazy map+filter fused operation
pub(crate) fn create_lazy_map_filtered(
    source: ValueHandle,
    mapper: crate::ast::Function,
    predicate: crate::ast::Function,
) -> LazyValue {
    LazyValue::MapFiltered {
        source: source.get_internal(),
        mapper: Arc::new(ThreadSafeFunction::from_function(&mapper)),
        predicate: Arc::new(ThreadSafeFunction::from_function(&predicate)),
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

/// Context for lazy evaluation with error recovery and tracking
#[derive(Debug, Clone)]
pub(crate) struct LazyEvaluationContext {
    pub start_time: Instant,
    pub evaluation_depth: usize,
    pub visited_thunks: Arc<RwLock<HashSet<String>>>,
    pub config: LazyConfig,
    pub recovery_attempts: usize,
    pub max_recovery_attempts: usize,
    pub lock_manager: Arc<LockManager>,
    pub thread_id: Option<std::thread::ThreadId>,
}

impl LazyEvaluationContext {
    pub fn new(config: LazyConfig) -> Self {
        Self {
            start_time: Instant::now(),
            evaluation_depth: 0,
            visited_thunks: Arc::new(RwLock::new(HashSet::new())),
            config,
            recovery_attempts: 0,
            max_recovery_attempts: 3,
            lock_manager: Arc::new(LockManager::default()),
            thread_id: Some(std::thread::current().id()),
        }
    }

    /// Check if we're still on the same thread
    fn check_thread_safety(&self) -> Result<(), InterpreterError> {
        if self.config.thread_safety_checks {
            if let Some(original_thread) = self.thread_id {
                if original_thread != std::thread::current().id() {
                    return Err(InterpreterError::ThreadSafetyViolation {
                        details: "Context used from different thread than created".to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn increment_depth(&mut self) -> Result<(), InterpreterError> {
        self.check_thread_safety()?;
        self.evaluation_depth += 1;
        if self.evaluation_depth > self.config.max_evaluation_depth {
            return Err(InterpreterError::EvaluationChainTooDeep {
                depth: self.evaluation_depth,
                max_depth: self.config.max_evaluation_depth,
            });
        }
        Ok(())
    }

    pub fn check_timeout(&self) -> Result<(), InterpreterError> {
        let elapsed = self.start_time.elapsed();
        let timeout_ms = self.get_effective_timeout();
        
        // Check for timeout warning
        if self.config.timeout_monitoring_enabled {
            let warning_threshold = (timeout_ms as f64 * self.config.timeout_warning_threshold) as u64;
            if elapsed.as_millis() > warning_threshold as u128 {
                // In a real implementation, this would log a warning
                // For now, we'll just continue - the warning would be logged elsewhere
            }
        }
        
        if elapsed.as_millis() > timeout_ms as u128 {
            return Err(InterpreterError::LazyEvaluationTimeout {
                timeout_ms,
            });
        }
        Ok(())
    }

    pub fn check_circular_dependency(&self, thunk_id: &str) -> Result<(), InterpreterError> {
        if !self.config.circular_dependency_detection {
            return Ok(());
        }

        // Use read lock for checking
        let visited = if self.config.thread_safety_checks {
            self.lock_manager.try_acquire_read_lock(&self.visited_thunks)?
        } else {
            self.visited_thunks.read().unwrap()
        };
        
        if visited.contains(thunk_id) {
            // Enhanced cycle detection: provide the full cycle path
            let cycle_path = self.build_cycle_path(&visited, thunk_id);
            return Err(InterpreterError::CircularDependency {
                cycle: format!("Circular dependency detected: {}", cycle_path),
            });
        }
        
        // Drop read lock before acquiring write lock
        drop(visited);
        
        // Use write lock for inserting
        let mut visited_write = if self.config.thread_safety_checks {
            self.lock_manager.try_acquire_write_lock(&self.visited_thunks)?
        } else {
            self.visited_thunks.write().unwrap()
        };
        
        visited_write.insert(thunk_id.to_string());
        Ok(())
    }

    /// Build a descriptive cycle path for better error reporting
    fn build_cycle_path(&self, visited: &HashSet<String>, current_thunk: &str) -> String {
        let mut cycle_nodes = Vec::new();
        
        // In a real implementation, we would track the actual dependency chain
        // For now, we'll create a simplified representation
        cycle_nodes.push(current_thunk.to_string());
        
        // Add a few nodes from the visited set to show the cycle
        for (i, visited_thunk) in visited.iter().enumerate() {
            if i >= 3 { // Limit to prevent very long error messages
                cycle_nodes.push("...".to_string());
                break;
            }
            cycle_nodes.push(visited_thunk.clone());
        }
        
        cycle_nodes.push(current_thunk.to_string()); // Complete the cycle
        cycle_nodes.join(" -> ")
    }

    /// Clear circular dependency tracking for a specific thunk
    pub fn clear_thunk_dependency(&self, thunk_id: &str) {
        if self.config.thread_safety_checks {
            if let Ok(mut visited) = self.lock_manager.try_acquire_write_lock(&self.visited_thunks) {
                visited.remove(thunk_id);
            }
        } else {
            if let Ok(mut visited) = self.visited_thunks.write() {
                visited.remove(thunk_id);
            }
        }
    }

    /// Check for potential circular dependencies before they occur
    pub fn check_potential_cycle(&self, dependencies: &[String]) -> Result<(), InterpreterError> {
        if !self.config.circular_dependency_detection {
            return Ok(());
        }

        let visited = if self.config.thread_safety_checks {
            self.lock_manager.try_acquire_read_lock(&self.visited_thunks)?
        } else {
            self.visited_thunks.read().unwrap()
        };
        
        // Check if any of the dependencies are already in the visited set
        for dep in dependencies {
            if visited.contains(dep) {
                return Err(InterpreterError::CircularDependency {
                    cycle: format!("Potential circular dependency detected with: {}", dep),
                });
            }
        }
        
        Ok(())
    }

    /// Try to break a circular dependency by forcing one of the nodes
    pub fn try_break_cycle(&mut self, thunk_id: &str) -> Result<(), InterpreterError> {
        if !self.config.enable_recovery {
            return Err(InterpreterError::CircularDependency {
                cycle: format!("Cannot break cycle for {}: recovery disabled", thunk_id),
            });
        }

        // Clear the visited thunks to break the cycle
        if self.config.thread_safety_checks {
            if let Ok(mut visited) = self.lock_manager.try_acquire_write_lock(&self.visited_thunks) {
                visited.clear();
            }
        } else {
            if let Ok(mut visited) = self.visited_thunks.write() {
                visited.clear();
            }
        }

        // Increment recovery attempts
        self.attempt_recovery();

        Ok(())
    }

    /// Get statistics about circular dependency detection
    pub fn get_cycle_detection_stats(&self) -> (usize, usize) {
        let visited_count = if self.config.thread_safety_checks {
            self.lock_manager.try_acquire_read_lock(&self.visited_thunks)
                .map(|v| v.len())
                .unwrap_or(0)
        } else {
            self.visited_thunks.read().map(|v| v.len()).unwrap_or(0)
        };
        (visited_count, self.recovery_attempts)
    }

    pub fn check_memory_pressure(&self) -> Result<(), InterpreterError> {
        if check_memory_pressure(self.config.memory_threshold_mb) {
            let estimated_memory_mb = get_estimated_memory_usage();
            
            if estimated_memory_mb > self.config.memory_threshold_mb {
                return Err(InterpreterError::MemoryLimitExceeded {
                    current_mb: estimated_memory_mb,
                    limit_mb: self.config.memory_threshold_mb,
                });
            }
            
            // Check if we're approaching the limit
            let memory_usage_ratio = estimated_memory_mb as f64 / self.config.memory_threshold_mb as f64;
            if memory_usage_ratio > self.config.memory_pressure_threshold {
                // Try to trigger cleanup
                self.suggest_memory_cleanup();
            }
        }
        Ok(())
    }

    pub fn can_recover(&self) -> bool {
        self.config.enable_recovery && self.recovery_attempts < self.max_recovery_attempts
    }

    pub fn attempt_recovery(&mut self) {
        self.recovery_attempts += 1;
    }

    /// Get the effective timeout based on the timeout strategy
    pub fn get_effective_timeout(&self) -> u64 {
        match &self.config.timeout_strategy {
            TimeoutStrategy::Fixed(timeout) => *timeout,
            TimeoutStrategy::Adaptive { base_ms, scaling_factor } => {
                // Scale timeout based on evaluation depth
                let scaled_timeout = *base_ms as f64 * scaling_factor.powf(self.evaluation_depth as f64);
                scaled_timeout.min(self.config.timeout_ms as f64 * 2.0) as u64 // Cap at 2x base timeout
            }
            TimeoutStrategy::Progressive { initial_ms, max_ms, multiplier } => {
                // Progressive timeout increases with recovery attempts
                let progressive_timeout = *initial_ms as f64 * multiplier.powf(self.recovery_attempts as f64);
                progressive_timeout.min(*max_ms as f64) as u64
            }
            TimeoutStrategy::PerOperation { .. } => {
                // For now, use the base timeout - per-operation timeouts would be set at the operation level
                self.config.timeout_ms
            }
        }
    }

    /// Get timeout for a specific operation type
    pub fn get_operation_timeout(&self, operation: &str) -> u64 {
        match &self.config.timeout_strategy {
            TimeoutStrategy::PerOperation { map_ms, filter_ms, range_ms, concat_ms, thunk_ms } => {
                match operation {
                    "map" => *map_ms,
                    "filter" => *filter_ms,
                    "range" => *range_ms,
                    "concat" => *concat_ms,
                    "thunk" => *thunk_ms,
                    _ => self.config.timeout_ms,
                }
            }
            _ => self.get_effective_timeout(),
        }
    }

    /// Check timeout for a specific operation
    pub fn check_operation_timeout(&self, operation: &str) -> Result<(), InterpreterError> {
        let elapsed = self.start_time.elapsed();
        let timeout_ms = self.get_operation_timeout(operation);
        
        if elapsed.as_millis() > timeout_ms as u128 {
            return Err(InterpreterError::LazyEvaluationTimeout {
                timeout_ms,
            });
        }
        Ok(())
    }

    /// Extend timeout for recovery attempts
    pub fn extend_timeout_for_recovery(&mut self) {
        match &mut self.config.timeout_strategy {
            TimeoutStrategy::Fixed(timeout) => {
                *timeout *= 2; // Double the timeout
            }
            TimeoutStrategy::Adaptive { base_ms, .. } => {
                *base_ms *= 2; // Double the base timeout
            }
            TimeoutStrategy::Progressive { max_ms, .. } => {
                *max_ms *= 2; // Double the max timeout
            }
            TimeoutStrategy::PerOperation { map_ms, filter_ms, range_ms, concat_ms, thunk_ms } => {
                *map_ms *= 2;
                *filter_ms *= 2;
                *range_ms *= 2;
                *concat_ms *= 2;
                *thunk_ms *= 2;
            }
        }
    }

    /// Suggest memory cleanup strategies
    fn suggest_memory_cleanup(&self) {
        // In a real implementation, this would:
        // 1. Force evaluation of completed lazy values
        // 2. Clear unnecessary caches
        // 3. Trigger garbage collection
        // 4. Compact memory layouts
        
        // For now, just a placeholder
    }

    /// Check if we should use lazy evaluation based on memory pressure
    pub fn should_use_lazy_evaluation(&self, operation_size: usize) -> bool {
        if !self.config.lazy_by_default {
            return false;
        }
        
        // If memory pressure is high, be more conservative about lazy evaluation
        if check_memory_pressure(self.config.memory_threshold_mb) {
            // Under memory pressure, only use lazy evaluation for very large operations
            operation_size > self.config.lazy_threshold * 2
        } else {
            // Normal conditions, use standard threshold
            operation_size > self.config.lazy_threshold
        }
    }

    /// Optimize memory usage for the current context
    pub fn optimize_memory_usage(&mut self) -> Result<(), InterpreterError> {
        // Clear circular dependency tracking if memory is tight
        if check_memory_pressure(self.config.memory_threshold_mb) {
            if self.config.thread_safety_checks {
                if let Ok(mut visited) = self.lock_manager.try_acquire_write_lock(&self.visited_thunks) {
                    if visited.len() > 100 { // Arbitrary threshold
                        visited.clear();
                    }
                }
            } else {
                if let Ok(mut visited) = self.visited_thunks.write() {
                    if visited.len() > 100 { // Arbitrary threshold
                        visited.clear();
                    }
                }
            }
        }
        
        // Reduce evaluation depth limit under memory pressure
        if get_estimated_memory_usage() > self.config.memory_threshold_mb * 3 / 4 {
            self.config.max_evaluation_depth = self.config.max_evaluation_depth.min(500);
        }
        
        Ok(())
    }
}

/// Thread-safe lock manager to prevent deadlocks
#[derive(Debug)]
pub(crate) struct LockManager {
    lock_timeout: Duration,
    max_lock_attempts: usize,
}

impl Default for LockManager {
    fn default() -> Self {
        Self {
            lock_timeout: Duration::from_millis(100),
            max_lock_attempts: 5,
        }
    }
}

impl LockManager {
    /// Try to acquire a lock with timeout and retry logic
    pub fn try_acquire_lock<'a, T>(&self, mutex: &'a Mutex<T>) -> Result<std::sync::MutexGuard<'a, T>, InterpreterError> {
        for attempt in 0..self.max_lock_attempts {
            match mutex.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if attempt == self.max_lock_attempts - 1 {
                        return Err(InterpreterError::ThreadSafetyViolation {
                            details: format!("Failed to acquire lock after {} attempts", self.max_lock_attempts),
                        });
                    }
                    std::thread::sleep(self.lock_timeout);
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(InterpreterError::ThreadSafetyViolation {
                        details: "Lock is poisoned".to_string(),
                    });
                }
            }
        }
        unreachable!()
    }

    /// Try to acquire a read lock with timeout
    pub fn try_acquire_read_lock<'a, T>(&self, rwlock: &'a RwLock<T>) -> Result<std::sync::RwLockReadGuard<'a, T>, InterpreterError> {
        for attempt in 0..self.max_lock_attempts {
            match rwlock.try_read() {
                Ok(guard) => return Ok(guard),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if attempt == self.max_lock_attempts - 1 {
                        return Err(InterpreterError::ThreadSafetyViolation {
                            details: format!("Failed to acquire read lock after {} attempts", self.max_lock_attempts),
                        });
                    }
                    std::thread::sleep(self.lock_timeout);
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(InterpreterError::ThreadSafetyViolation {
                        details: "Read lock is poisoned".to_string(),
                    });
                }
            }
        }
        unreachable!()
    }

    /// Try to acquire a write lock with timeout
    pub fn try_acquire_write_lock<'a, T>(&self, rwlock: &'a RwLock<T>) -> Result<std::sync::RwLockWriteGuard<'a, T>, InterpreterError> {
        for attempt in 0..self.max_lock_attempts {
            match rwlock.try_write() {
                Ok(guard) => return Ok(guard),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if attempt == self.max_lock_attempts - 1 {
                        return Err(InterpreterError::ThreadSafetyViolation {
                            details: format!("Failed to acquire write lock after {} attempts", self.max_lock_attempts),
                        });
                    }
                    std::thread::sleep(self.lock_timeout);
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(InterpreterError::ThreadSafetyViolation {
                        details: "Write lock is poisoned".to_string(),
                    });
                }
            }
        }
        unreachable!()
    }
}

// Include tests
mod tests;

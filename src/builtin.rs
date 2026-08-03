use crate::ast::{BuiltinFunction, Value};
use crate::interpreter::InterpreterError;
use crate::parallel::should_parallelize;
use rayon::prelude::*;
use std::collections::HashMap;

pub struct BuiltinFunctions {
    functions: HashMap<String, BuiltinFunction>,
}

impl Default for BuiltinFunctions {
    fn default() -> Self {
        Self::new()
    }
}

impl BuiltinFunctions {
    pub fn new() -> Self {
        let mut functions = HashMap::new();

        // Core I/O functions
        functions.insert(
            "println".to_string(),
            BuiltinFunction {
                name: "println".to_string(),
                arity: 0, // 0 means variadic (any number of arguments)
            },
        );

        functions.insert(
            "print".to_string(),
            BuiltinFunction {
                name: "print".to_string(),
                arity: 1,
            },
        );

        // List functions
        functions.insert(
            "map".to_string(),
            BuiltinFunction {
                name: "map".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "filter".to_string(),
            BuiltinFunction {
                name: "filter".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "reduce".to_string(),
            BuiltinFunction {
                name: "reduce".to_string(),
                arity: 3,
            },
        );

        functions.insert(
            "fold".to_string(),
            BuiltinFunction {
                name: "fold".to_string(),
                arity: 3,
            },
        );

        functions.insert(
            "len".to_string(),
            BuiltinFunction {
                name: "len".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "head".to_string(),
            BuiltinFunction {
                name: "head".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "tail".to_string(),
            BuiltinFunction {
                name: "tail".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "cons".to_string(),
            BuiltinFunction {
                name: "cons".to_string(),
                arity: 2,
            },
        );

        // Type conversion functions
        functions.insert(
            "to_string".to_string(),
            BuiltinFunction {
                name: "to_string".to_string(),
                arity: 1,
            },
        );

        // Utility functions
        functions.insert(
            "range".to_string(),
            BuiltinFunction {
                name: "range".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "zip".to_string(),
            BuiltinFunction {
                name: "zip".to_string(),
                arity: 2,
            },
        );

        // Type introspection
        functions.insert(
            "typeof".to_string(),
            BuiltinFunction {
                name: "typeof".to_string(),
                arity: 1,
            },
        );

        // List manipulation utilities
        functions.insert(
            "reverse".to_string(),
            BuiltinFunction {
                name: "reverse".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "sort".to_string(),
            BuiltinFunction {
                name: "sort".to_string(),
                arity: 1,
            },
        );

        // String/List conversion utilities
        functions.insert(
            "join".to_string(),
            BuiltinFunction {
                name: "join".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "split".to_string(),
            BuiltinFunction {
                name: "split".to_string(),
                arity: 2,
            },
        );

        // Search and query utilities
        functions.insert(
            "contains".to_string(),
            BuiltinFunction {
                name: "contains".to_string(),
                arity: 2,
            },
        );

        // === New Built-ins (v0.5 draft) ===
        functions.insert(
            "sum".to_string(),
            BuiltinFunction {
                name: "sum".to_string(),
                arity: 1,
            },
        );
        functions.insert(
            "average".to_string(),
            BuiltinFunction {
                name: "average".to_string(),
                arity: 1,
            },
        );
        functions.insert(
            "min".to_string(),
            BuiltinFunction {
                name: "min".to_string(),
                arity: 1,
            },
        );
        functions.insert(
            "max".to_string(),
            BuiltinFunction {
                name: "max".to_string(),
                arity: 1,
            },
        );
        functions.insert(
            "clamp".to_string(),
            BuiltinFunction {
                name: "clamp".to_string(),
                arity: 3,
            },
        );
        functions.insert(
            "flatten".to_string(),
            BuiltinFunction {
                name: "flatten".to_string(),
                arity: 1,
            },
        );
        functions.insert(
            "chunk".to_string(),
            BuiltinFunction {
                name: "chunk".to_string(),
                arity: 2,
            },
        );
        functions.insert(
            "enumerate".to_string(),
            BuiltinFunction {
                name: "enumerate".to_string(),
                arity: 1,
            },
        );
        functions.insert(
            "find".to_string(),
            BuiltinFunction {
                name: "find".to_string(),
                arity: 2,
            },
        );
        functions.insert(
            "starts_with".to_string(),
            BuiltinFunction {
                name: "starts_with".to_string(),
                arity: 2,
            },
        );
        functions.insert(
            "ends_with".to_string(),
            BuiltinFunction {
                name: "ends_with".to_string(),
                arity: 2,
            },
        );
        functions.insert(
            "group_by".to_string(),
            BuiltinFunction {
                name: "group_by".to_string(),
                arity: 2,
            },
        );

        // Parallel control functions
        functions.insert(
            "set_parallel".to_string(),
            BuiltinFunction {
                name: "set_parallel".to_string(),
                arity: 1,
            },
        );

        // Lazy evaluation functions
        functions.insert(
            "take".to_string(),
            BuiltinFunction {
                name: "take".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "skip".to_string(),
            BuiltinFunction {
                name: "skip".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "force".to_string(),
            BuiltinFunction {
                name: "force".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "lazy".to_string(),
            BuiltinFunction {
                name: "lazy".to_string(),
                arity: 1,
            },
        );

        // List concatenation function
        functions.insert(
            "concat".to_string(),
            BuiltinFunction {
                name: "concat".to_string(),
                arity: 2,
            },
        );

        // Fused map+filter function
        functions.insert(
            "map_filtered".to_string(),
            BuiltinFunction {
                name: "map_filtered".to_string(),
                arity: 3,
            },
        );

        // Map functions
        functions.insert(
            "map_get".to_string(),
            BuiltinFunction {
                name: "map_get".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "map_set".to_string(),
            BuiltinFunction {
                name: "map_set".to_string(),
                arity: 3,
            },
        );

        functions.insert(
            "map_has_key".to_string(),
            BuiltinFunction {
                name: "map_has_key".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "map_keys".to_string(),
            BuiltinFunction {
                name: "map_keys".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "map_values".to_string(),
            BuiltinFunction {
                name: "map_values".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "map_remove".to_string(),
            BuiltinFunction {
                name: "map_remove".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "map_len".to_string(),
            BuiltinFunction {
                name: "map_len".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "map_clear".to_string(),
            BuiltinFunction {
                name: "map_clear".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "map_merge".to_string(),
            BuiltinFunction {
                name: "map_merge".to_string(),
                arity: 2,
            },
        );

        // Result type utility functions
        functions.insert(
            "unwrap".to_string(),
            BuiltinFunction {
                name: "unwrap".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "unwrap_or".to_string(),
            BuiltinFunction {
                name: "unwrap_or".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "unwrap_or_else".to_string(),
            BuiltinFunction {
                name: "unwrap_or_else".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "is_ok".to_string(),
            BuiltinFunction {
                name: "is_ok".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "is_err".to_string(),
            BuiltinFunction {
                name: "is_err".to_string(),
                arity: 1,
            },
        );

        functions.insert(
            "result_map".to_string(),
            BuiltinFunction {
                name: "result_map".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "result_map_err".to_string(),
            BuiltinFunction {
                name: "result_map_err".to_string(),
                arity: 2,
            },
        );

        Self { functions }
    }

    pub fn get_functions(&self) -> &HashMap<String, BuiltinFunction> {
        &self.functions
    }

    pub fn call(
        builtins: &BuiltinFunctions,
        name: &str,
        arguments: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        // Check if this is a force point - functions that require eager evaluation
        if crate::internal::is_force_point(name) {
            // Force evaluation of any lazy arguments before calling the function
            let forced_args: Result<Vec<_>, _> = arguments
                .into_iter()
                .map(|arg| {
                    // For now, just return the argument as-is since we're working with the public Value interface
                    // In a full implementation, this would force lazy ValueHandle values
                    Ok(arg)
                })
                .collect();
            let arguments = forced_args?;
            
            // Continue with the original function call logic
            return Self::call_internal(builtins, name, arguments, interpreter);
        }

        // Check if this is a lazy function that can work with lazy values
        if crate::internal::is_lazy_function(name) {
            // For lazy functions, we can pass through lazy values
            return Self::call_internal(builtins, name, arguments, interpreter);
        }

        // Default case - call the function normally
        Self::call_internal(builtins, name, arguments, interpreter)
    }

    fn call_internal(
        builtins: &BuiltinFunctions,
        name: &str,
        arguments: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        // Handle filesystem functions
        if let Some(fs_function) = name.strip_prefix("fs.") {
            // Remove "fs." prefix
            return crate::stdlib::fs::call_fs_function(fs_function, arguments).map_err(|e| {
                InterpreterError::RuntimeError {
                    message: e.to_string(),
                }
            });
        }

        // Handle HTTP functions
        if let Some(http_function) = name.strip_prefix("http.") {
            // Remove "http." prefix
            return crate::stdlib::http::call_http_function(http_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Handle math functions
        if let Some(math_function) = name.strip_prefix("math.") {
            // Remove "math." prefix
            return crate::stdlib::math::call_math_function(math_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Handle dates functions
        if let Some(dates_function) = name.strip_prefix("dates.") {
            // Remove "dates." prefix
            return crate::stdlib::dates::call_dates_function(dates_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Handle random functions
        if let Some(random_function) = name.strip_prefix("random.") {
            // Remove "random." prefix
            return crate::stdlib::random::call_random_function(random_function, arguments)
                .map_err(|e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                });
        }

        // Handle csv functions
        if let Some(csv_function) = name.strip_prefix("csv.") {
            // Remove "csv." prefix
            return crate::stdlib::csv::call_csv_function(csv_function, arguments).map_err(|e| {
                InterpreterError::RuntimeError {
                    message: e.to_string(),
                }
            });
        }

        // Handle json functions
        if let Some(json_function) = name.strip_prefix("json.") {
            // Remove "json." prefix
            return crate::stdlib::json::call_json_function(json_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Handle base64 functions
        if let Some(base64_function) = name.strip_prefix("base64.") {
            // Remove "base64." prefix
            return crate::stdlib::base64::call_base64_function(base64_function, arguments)
                .map_err(|e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                });
        }

        // Handle os functions
        if let Some(os_function) = name.strip_prefix("os.") {
            // Remove "os." prefix
            return crate::stdlib::os::call_os_function(os_function, arguments).map_err(|e| {
                InterpreterError::RuntimeError {
                    message: e.to_string(),
                }
            });
        }

        // Handle crypto functions
        if let Some(crypto_function) = name.strip_prefix("crypto.") {
            // Remove "crypto." prefix
            return crate::stdlib::crypto::call_crypto_function(crypto_function, arguments)
                .map_err(|e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                });
        }

        // Handle testing functions
        if let Some(testing_function) = name.strip_prefix("testing.") {
            // Remove "testing." prefix
            return crate::stdlib::testing::call_testing_function(testing_function, arguments)
                .map_err(|e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                });
        }

        match name {
            "println" => builtins.println(arguments),
            "print" => builtins.print(arguments),
            "map" => builtins.map(arguments, interpreter),
            "filter" => builtins.filter(arguments, interpreter),
            "reduce" => builtins.reduce(arguments, interpreter),
            "fold" => builtins.fold(arguments, interpreter),
            "len" => builtins.len(arguments),
            "head" => builtins.head(arguments),
            "tail" => builtins.tail(arguments),
            "cons" => builtins.cons(arguments),
            "to_string" => builtins.to_string(arguments),
            "to_int" => builtins.to_int(arguments),
            "to_float" => builtins.to_float(arguments),
            "range" => builtins.range(arguments),
            "zip" => builtins.zip(arguments),
            "typeof" => builtins.type_of(arguments),
            "reverse" => builtins.reverse(arguments),
            "sort" => builtins.sort(arguments),
            "join" => builtins.join(arguments),
            "split" => builtins.split(arguments),
            "contains" => builtins.contains(arguments),
            "sum" => builtins.sum(arguments),
            "average" => builtins.average(arguments),
            "min" => builtins.min_value(arguments),
            "max" => builtins.max_value(arguments),
            "clamp" => builtins.clamp(arguments),
            "flatten" => builtins.flatten(arguments),
            "chunk" => builtins.chunk(arguments),
            "enumerate" => builtins.enumerate_list(arguments),
            "find" => builtins.find(arguments, interpreter),
            "starts_with" => builtins.starts_with(arguments),
            "ends_with" => builtins.ends_with(arguments),
            "group_by" => builtins.group_by(arguments, interpreter),
            "set_parallel" => builtins.set_parallel(arguments),
            "take" => builtins.take_lazy(arguments, interpreter),
            "skip" => builtins.skip_lazy(arguments, interpreter),
            "force" => builtins.force_value(arguments, interpreter),
            "lazy" => builtins.make_lazy(arguments, interpreter),
            "concat" => builtins.concat_lazy(arguments, interpreter),
            "map_filtered" => builtins.map_filtered(arguments, interpreter),
            "map_get" => builtins.map_get(arguments),
            "map_set" => builtins.map_set(arguments),
            "map_has_key" => builtins.map_has_key(arguments),
            "map_keys" => builtins.map_keys(arguments),
            "map_values" => builtins.map_values(arguments),
            "map_remove" => builtins.map_remove(arguments),
            "map_len" => builtins.map_len(arguments),
            "map_clear" => builtins.map_clear(arguments),
            "map_merge" => builtins.map_merge(arguments),
            "unwrap" => builtins.unwrap_result(arguments),
            "unwrap_or" => builtins.unwrap_or(arguments),
            "unwrap_or_else" => builtins.unwrap_or_else(arguments, interpreter),
            "is_ok" => builtins.is_ok(arguments),
            "is_err" => builtins.is_err(arguments),
            "result_map" => builtins.result_map(arguments, interpreter),
            "result_map_err" => builtins.result_map_err(arguments, interpreter),
            _ => Err(InterpreterError::RuntimeError {
                message: format!("Unknown builtin function: {}", name),
            }),
        }
    }

    fn println(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.is_empty() {
            println!();
        } else {
            // Force evaluation of any lazy values before printing
            let output = args
                .iter()
                .map(|arg| match arg {
                    // Print raw string content without quotes for a nicer UX
                    Value::String(s) => s.as_str().to_string(),
                    _ => format!("{}", arg),
                })
                .collect::<Vec<String>>()
                .join(" ");
            println!("{}", output);
        }
        Ok(Value::Unit)
    }

    fn print(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::String(s) => print!("{}", s.as_str()),
            other => print!("{}", other),
        }
        Ok(Value::Unit)
    }

    fn map(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let list = &args[0];
        let function = &args[1];

        // Work with references instead of copying the entire list
        let list_ref = match list {
            Value::List(items) => items.as_ref(),
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                // For ranges, we still need to materialize, but only once
                let end_val = if *inclusive { end.saturating_add(1) } else { *end };
                let range_size = end_val.saturating_sub(*start).max(0) as usize;
                
                // MEMORY MONITORING: Check if range is too large before creating
                if range_size > 10_000_000 {
                    return Err(InterpreterError::RuntimeError {
                        message: format!(
                            "Range size ({}) too large, this could cause memory issues. Maximum range size is 10000000.",
                            range_size
                        ),
                    });
                }
                
                let range_vec: Vec<Value> = (*start..end_val).map(Value::Integer).collect();
                
                // AGGRESSIVE MEMORY MANAGEMENT: Cleanup after large range operations
                if range_size > 50 {
                    interpreter.force_memory_cleanup();
                }
                
                return Self::process_map_range(range_vec, function, interpreter);
            }
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map: first argument must be a list or range".to_string(),
                })
            }
        };

        // Check if we should use lazy evaluation
        let config = interpreter.get_lazy_config();
        if config.lazy_by_default && list_ref.len() > config.lazy_threshold {
            if let Value::Function(func) = function {
                let source_handle = crate::internal::utils::value_to_handle(list.clone(), config);
                let mut lazy_val = crate::internal::create_lazy_map(source_handle.clone(), func.clone());
                // Fusion logic: if the source is already a lazy value, try to fuse
                if config.fusion_enabled {
                    if let crate::internal::InternalValue::Lazy(ref prev_lazy) = *source_handle.get_internal() {
                        if let Some(fused) = crate::internal::try_fuse_operations(prev_lazy, "map", Some(func.clone())) {
                            lazy_val = fused;
                        }
                    }
                }
                let lazy_handle = crate::internal::ValueHandle::new_lazy(lazy_val);
                return lazy_handle.get(interpreter);
            }
        }

        // Sequential evaluation - parallel disabled because interpreter cloning is too expensive
        // (cloning full interpreter state for each element defeats parallelization benefits)
        let mut result = Vec::with_capacity(list_ref.len());
        for item in list_ref.iter() {
            let value = interpreter.call_function_optimized(function, vec![item.clone()])?;
            result.push(value);
        }
        
        Ok(Value::List(result.into()))
    }

    fn process_map_range(
        range_vec: Vec<Value>,
        function: &Value,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        // Sequential only - parallel disabled due to interpreter cloning overhead
        let mut result = Vec::with_capacity(range_vec.len());
        for item in range_vec.iter() {
            let value = interpreter.call_function_optimized(function, vec![item.clone()])?;
            result.push(value);
        }
        Ok(Value::List(result.into()))
    }

    fn process_filter_range(
        range_vec: Vec<Value>,
        function: &Value,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        // Sequential only - parallel disabled due to interpreter cloning overhead
        let mut result = Vec::with_capacity(range_vec.len());
        for item in range_vec.iter() {
            let pred = interpreter.call_function_optimized(function, vec![item.clone()])?;
            if let Value::Boolean(true) = pred {
                result.push(item.clone())
            }
        }
        Ok(Value::List(result.into()))
    }

    fn filter(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let list = &args[0];
        let function = &args[1];

        // Work with references instead of copying the entire list
        let list_ref = match list {
            Value::List(items) => items.as_ref(),
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                // For ranges, we still need to materialize, but only once
                let end_val = if *inclusive { end.saturating_add(1) } else { *end };
                let range_size = end_val.saturating_sub(*start).max(0) as usize;
                
                // MEMORY MONITORING: Check if range is too large before creating
                if range_size > 10_000_000 {
                    return Err(InterpreterError::RuntimeError {
                        message: format!(
                            "Range size ({}) too large, this could cause memory issues. Maximum range size is 10000000.",
                            range_size
                        ),
                    });
                }
                
                let range_vec: Vec<Value> = (*start..end_val).map(Value::Integer).collect();
                
                // AGGRESSIVE MEMORY MANAGEMENT: Cleanup after large range operations
                if range_size > 50 {
                    interpreter.force_memory_cleanup();
                }
                
                return Self::process_filter_range(range_vec, function, interpreter);
            }
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "filter: first argument must be a list or range".to_string(),
                })
            }
        };

        // Check if we should use lazy evaluation
        let config = interpreter.get_lazy_config();
        if config.lazy_by_default && list_ref.len() > config.lazy_threshold {
            if let Value::Function(func) = function {
                let source_handle = crate::internal::utils::value_to_handle(list.clone(), config);
                let mut lazy_val = crate::internal::create_lazy_filter(source_handle.clone(), func.clone());
                // Fusion logic: if the source is already a lazy value, try to fuse
                if config.fusion_enabled {
                    if let crate::internal::InternalValue::Lazy(ref prev_lazy) = *source_handle.get_internal() {
                        if let Some(fused) = crate::internal::try_fuse_operations(prev_lazy, "filter", Some(func.clone())) {
                            lazy_val = fused;
                        }
                    }
                }
                let lazy_handle = crate::internal::ValueHandle::new_lazy(lazy_val);
                return lazy_handle.get(interpreter);
            }
        }

        // Sequential evaluation - parallel disabled due to interpreter cloning overhead
        let mut result = Vec::with_capacity(list_ref.len());
        for item in list_ref.iter() {
            let pred = interpreter.call_function_optimized(function, vec![item.clone()])?;
            if let Value::Boolean(true) = pred {
                result.push(item.clone())
            }
        }
        
        Ok(Value::List(result.into()))
    }

    fn reduce(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 3 {
            return Err(InterpreterError::ArityMismatch {
                expected: 3,
                got: args.len(),
            });
        }
        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "reduce: first argument must be a list".to_string(),
                })
            }
        };
        let list = list_rc.as_ref();
        let mut acc = args[1].clone();
        let function = &args[2];
        for item in list.iter() {
            acc = interpreter.call_function(function.clone(), vec![acc, item.clone()])?;
        }
        Ok(acc)
    }

    fn fold(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        self.reduce(args, interpreter)
    }

    fn len(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::List(items) => Ok(Value::Integer(items.len() as i64)),
            Value::String(s) => Ok(Value::Integer(s.chars().count() as i64)),
            Value::Tuple(items) => Ok(Value::Integer(items.len() as i64)),
            _ => Err(InterpreterError::TypeError {
                message: "len: argument must be a list, string, or tuple".to_string(),
            }),
        }
    }

    fn head(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::List(items) => {
                if items.is_empty() {
                    Err(InterpreterError::RuntimeError {
                        message: "head: cannot get head of empty list".to_string(),
                    })
                } else {
                    Ok(items[0].clone())
                }
            }
            _ => Err(InterpreterError::TypeError {
                message: "head: argument must be a list".to_string(),
            }),
        }
    }

    fn tail(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::List(items) => {
                if items.is_empty() {
                    Err(InterpreterError::RuntimeError {
                        message: "tail: cannot get tail of empty list".to_string(),
                    })
                } else {
                    Ok(Value::List(items[1..].to_vec().into()))
                }
            }
            _ => Err(InterpreterError::TypeError {
                message: "tail: argument must be a list".to_string(),
            }),
        }
    }

    fn cons(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let head = &args[0];
        let tail_rc = match &args[1] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "cons: second argument must be a list".to_string(),
                })
            }
        };
        let tail = tail_rc.as_ref();

        let mut new_list = vec![head.clone()];
        new_list.extend(tail.iter().cloned());
        Ok(Value::List(new_list.into()))
    }

    fn to_string(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        Ok(Value::String(args[0].to_string().into()))
    }

    fn to_int(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::Integer(n) => Ok(Value::Integer(*n)),
            Value::Float(x) => Ok(Value::Integer(*x as i64)),
            Value::String(s) => {
                s.parse::<i64>()
                    .map(Value::Integer)
                    .map_err(|_| InterpreterError::TypeError {
                        message: format!("Cannot convert string '{}' to integer", s),
                    })
            }
            _ => Err(InterpreterError::TypeError {
                message: "to_int: cannot convert value to integer".to_string(),
            }),
        }
    }

    fn to_float(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::Integer(n) => Ok(Value::Float(*n as f64)),
            Value::Float(x) => Ok(Value::Float(*x)),
            Value::String(s) => {
                s.parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| InterpreterError::TypeError {
                        message: format!("Cannot convert string '{}' to float", s),
                    })
            }
            _ => Err(InterpreterError::TypeError {
                message: "to_float: cannot convert value to float".to_string(),
            }),
        }
    }

    fn range(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        match args.len() {
            1 => {
                // range(n) -> [0, 1, 2, ..., n-1]
                let end = match &args[0] {
                    Value::Integer(n) => *n,
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "range: argument must be an integer".to_string(),
                        })
                    }
                };

                // For small ranges, generate eagerly
                if end <= 100 {
                    let mut result = Vec::new();
                    for i in 0..end {
                        result.push(Value::Integer(i));
                    }
                    Ok(Value::List(result.into()))
                } else {
                    // For large ranges, this could be made lazy in the future
                    // For now, still generate eagerly but with a warning for very large ranges
                    if end > 100000 {
                        crate::log::get_logger().warn("builtin", &format!("Generating very large range ({}), consider using lazy evaluation", end));
                    }
                    let mut result = Vec::new();
                    for i in 0..end {
                        result.push(Value::Integer(i));
                    }
                    Ok(Value::List(result.into()))
                }
            }
            2 => {
                // range(start, end) -> [start, start+1, ..., end-1]
                let start = match &args[0] {
                    Value::Integer(n) => *n,
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "range: arguments must be integers".to_string(),
                        })
                    }
                };

                let end = match &args[1] {
                    Value::Integer(n) => *n,
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "range: arguments must be integers".to_string(),
                        })
                    }
                };

                let mut result = Vec::new();
                for i in start..end {
                    result.push(Value::Integer(i));
                }
                Ok(Value::List(result.into()))
            }
            3 => {
                // range(start, end, step) -> [start, start+step, start+2*step, ...]
                let start = match &args[0] {
                    Value::Integer(n) => *n,
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "range: arguments must be integers".to_string(),
                        })
                    }
                };

                let end = match &args[1] {
                    Value::Integer(n) => *n,
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "range: arguments must be integers".to_string(),
                        })
                    }
                };

                let step = match &args[2] {
                    Value::Integer(n) => *n,
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "range: arguments must be integers".to_string(),
                        })
                    }
                };

                if step == 0 {
                    return Err(InterpreterError::RuntimeError {
                        message: "range: step cannot be zero".to_string(),
                    });
                }

                let mut result = Vec::new();
                if step > 0 {
                    let mut i = start;
                    while i < end {
                        result.push(Value::Integer(i));
                        i += step;
                    }
                } else {
                    let mut i = start;
                    while i > end {
                        result.push(Value::Integer(i));
                        i += step; // step is negative
                    }
                }
                Ok(Value::List(result.into()))
            }
            _ => Err(InterpreterError::ArityMismatch {
                expected: 2, // We'll say 2 for the error, but we actually accept 1-3
                got: args.len(),
            }),
        }
    }

    fn zip(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let list1_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "zip: arguments must be lists".to_string(),
                })
            }
        };
        let list1 = list1_rc.as_ref();

        let list2_rc = match &args[1] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "zip: arguments must be lists".to_string(),
                })
            }
        };
        let list2 = list2_rc.as_ref();

        let min_len = std::cmp::min(list1.len(), list2.len());
        let mut result = Vec::new();

        for i in 0..min_len {
            result.push(Value::Tuple(
                vec![list1[i].clone(), list2[i].clone()].into(),
            ));
        }

        Ok(Value::List(result.into()))
    }

    fn type_of(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        Ok(Value::String(args[0].type_name().into()))
    }

    fn reverse(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "reverse: argument must be a list".to_string(),
                })
            }
        };
        let list = list_rc.as_ref();

        // Use parallel processing for larger lists
        if should_parallelize(list.len()) {
            // PARALLEL VERSION - collect in reverse order using parallel iterator
            let result: Vec<Value> = list.par_iter().rev().cloned().collect();
            Ok(Value::List(result.into()))
        } else {
            // SEQUENTIAL VERSION for small lists
            let mut result = Vec::new();
            for item in list.iter().rev() {
                result.push(item.clone());
            }
            Ok(Value::List(result.into()))
        }
    }

    fn sort(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "sort: argument must be a list".to_string(),
                })
            }
        };
        let list = list_rc.as_ref();

        let mut result: Vec<Value> = list.to_vec();

        // Use parallel sorting for larger lists
        if should_parallelize(result.len()) {
            // PARALLEL VERSION - use rayon's parallel sort
            result.par_sort_by(|a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
                (Value::Float(x), Value::Float(y)) => {
                    x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                }
                (Value::String(x), Value::String(y)) => x.cmp(y),
                (Value::Integer(x), Value::Float(y)) => (*x as f64)
                    .partial_cmp(y)
                    .unwrap_or(std::cmp::Ordering::Equal),
                (Value::Float(x), Value::Integer(y)) => x
                    .partial_cmp(&(*y as f64))
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => std::cmp::Ordering::Equal,
            });
        } else {
            // SEQUENTIAL VERSION for small lists
            result.sort_by(|a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
                (Value::Float(x), Value::Float(y)) => {
                    x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                }
                (Value::String(x), Value::String(y)) => x.cmp(y),
                (Value::Integer(x), Value::Float(y)) => (*x as f64)
                    .partial_cmp(y)
                    .unwrap_or(std::cmp::Ordering::Equal),
                (Value::Float(x), Value::Integer(y)) => x
                    .partial_cmp(&(*y as f64))
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => std::cmp::Ordering::Equal,
            });
        }

        Ok(Value::List(result.into()))
    }

    fn join(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "join: first argument must be a list".to_string(),
                })
            }
        };
        let list = list_rc.as_ref();

        let separator_rc = match &args[1] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "join: second argument must be a string".to_string(),
                })
            }
        };
        let separator = separator_rc.as_ref();

        let result = list
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<String>>();
        Ok(Value::String(result.join(separator).into()))
    }

    fn split(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let string_rc = match &args[0] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "split: first argument must be a string".to_string(),
                })
            }
        };
        let string = string_rc.as_ref();

        let separator_rc = match &args[1] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "split: second argument must be a string".to_string(),
                })
            }
        };
        let separator = separator_rc.as_ref();

        let result = string
            .split(separator)
            .map(|s| Value::String(s.to_string().into()))
            .collect::<Vec<Value>>();
        Ok(Value::List(result.into()))
    }

    fn contains(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "contains: first argument must be a list".to_string(),
                })
            }
        };
        let list = list_rc.as_ref();
        let item = &args[1];

        // Use parallel processing for larger lists to speed up search
        if should_parallelize(list.len()) {
            // PARALLEL VERSION - parallel search using any()
            let result = list.par_iter().any(|i| i == item);
            Ok(Value::Boolean(result))
        } else {
            // SEQUENTIAL VERSION for small lists
            let result = list.iter().any(|i| i == item);
            Ok(Value::Boolean(result))
        }
    }

    fn sum(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        let (list_values, _should_use_parallel) = match &args[0] {
            Value::List(items) => (items.as_ref().to_vec(), should_parallelize(items.len())),
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                // Convert range to vector of integers
                let end_val = if *inclusive { end.saturating_add(1) } else { *end };
                let values: Vec<Value> = (*start..end_val).map(Value::Integer).collect();
                let use_parallel = should_parallelize(values.len());
                (values, use_parallel)
            }
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "sum: argument must be a list or range".to_string(),
                })
            }
        };

        // Check if we should use parallel processing - be more aggressive for mathematical operations
        let should_use_parallel = list_values.len() >= 5; // Very low threshold for sum operations - prioritize multi-threading

        if should_use_parallel {
            // PARALLEL VERSION - parallel sum with fold and reduce
            let result = list_values
                .par_iter()
                .try_fold(
                    || (0i64, 0.0f64, false), // (int_acc, float_acc, is_float)
                    |mut acc, item| match item {
                        Value::Integer(n) => {
                            if acc.2 {
                                acc.1 += *n as f64;
                            } else {
                                acc.0 = acc.0.checked_add(*n).ok_or_else(|| {
                                    InterpreterError::RuntimeError {
                                        message: "sum: integer overflow".to_string(),
                                    }
                                })?;
                            }
                            Ok(acc)
                        }
                        Value::Float(f) => {
                            if !acc.2 {
                                // Zero the int accumulator when promoting so the
                                // reduce step doesn't add it a second time
                                acc.1 = acc.0 as f64;
                                acc.0 = 0;
                                acc.2 = true;
                            }
                            acc.1 += f;
                            Ok(acc)
                        }
                        _ => Err(InterpreterError::TypeError {
                            message: "sum: list must contain only numbers".to_string(),
                        }),
                    },
                )
                .try_reduce(
                    || (0i64, 0.0f64, false),
                    |mut acc1, acc2| {
                        if acc1.2 || acc2.2 {
                            acc1.1 += acc1.0 as f64 + acc2.1 + acc2.0 as f64;
                            acc1.0 = 0;
                            acc1.2 = true;
                        } else {
                            acc1.0 = acc1.0.checked_add(acc2.0).ok_or_else(|| {
                                InterpreterError::RuntimeError {
                                    message: "sum: integer overflow".to_string(),
                                }
                            })?;
                        }
                        Ok(acc1)
                    },
                )?;

            if result.2 {
                Ok(Value::Float(result.1))
            } else {
                Ok(Value::Integer(result.0))
            }
        } else {
            // SEQUENTIAL VERSION (for small lists)
            let mut int_acc: i64 = 0;
            let mut float_acc: f64 = 0.0;
            let mut is_float = false;
            for item in list_values.iter() {
                match item {
                    Value::Integer(n) => {
                        if is_float {
                            float_acc += *n as f64;
                        } else {
                            int_acc = int_acc.checked_add(*n).ok_or_else(|| {
                                InterpreterError::RuntimeError {
                                    message: "sum: integer overflow".to_string(),
                                }
                            })?;
                        }
                    }
                    Value::Float(f) => {
                        if !is_float {
                            float_acc = int_acc as f64;
                            is_float = true;
                        }
                        float_acc += *f;
                    }
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "sum: list must contain only numbers".to_string(),
                        })
                    }
                }
            }
            if is_float {
                Ok(Value::Float(float_acc))
            } else {
                Ok(Value::Integer(int_acc))
            }
        }
    }

    fn average(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }
        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "average: argument must be a list".to_string(),
                })
            }
        };
        if list_rc.is_empty() {
            return Ok(Value::Err(Box::new(Value::String(
                "EmptyList".to_string().into(),
            ))));
        }
        let sum_val = self.sum(vec![Value::List(list_rc.clone())])?;
        match sum_val {
            Value::Integer(s) => Ok(Value::Float(s as f64 / list_rc.len() as f64)),
            Value::Float(f) => Ok(Value::Float(f / list_rc.len() as f64)),
            _ => unreachable!(),
        }
    }

    fn min_value(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }
        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "min: argument must be a list".to_string(),
                })
            }
        };
        if list_rc.is_empty() {
            return Ok(Value::Err(Box::new(Value::String(
                "EmptyList".to_string().into(),
            ))));
        }
        use std::cmp::Ordering;
        let mut min = list_rc[0].clone();
        for item in list_rc.iter().skip(1) {
            if let Some(ord) = item.compare_for_sort(&min) {
                if ord == Ordering::Less {
                    min = item.clone();
                }
            }
        }
        Ok(min)
    }

    fn max_value(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }
        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "max: argument must be a list".to_string(),
                })
            }
        };
        if list_rc.is_empty() {
            return Ok(Value::Err(Box::new(Value::String(
                "EmptyList".to_string().into(),
            ))));
        }
        use std::cmp::Ordering;
        let mut max = list_rc[0].clone();
        for item in list_rc.iter().skip(1) {
            if let Some(ord) = item.compare_for_sort(&max) {
                if ord == Ordering::Greater {
                    max = item.clone();
                }
            }
        }
        Ok(max)
    }

    fn clamp(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 3 {
            return Err(InterpreterError::ArityMismatch {
                expected: 3,
                got: args.len(),
            });
        }
        let (x, lo, hi) = (&args[0], &args[1], &args[2]);
        match (x, lo, hi) {
            (Value::Integer(v), Value::Integer(l), Value::Integer(h)) => {
                Ok(Value::Integer((*v).max(*l).min(*h)))
            }
            (Value::Float(v), Value::Float(l), Value::Float(h)) => {
                Ok(Value::Float((*v).max(*l).min(*h)))
            }
            (Value::Integer(v), Value::Float(l), Value::Float(h)) => {
                let vf = *v as f64;
                Ok(Value::Float(vf.max(*l).min(*h)))
            }
            (Value::Float(v), Value::Integer(l), Value::Integer(h)) => {
                let lf = *l as f64;
                let hf = *h as f64;
                Ok(Value::Float((*v).max(lf).min(hf)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "clamp: all arguments must be numeric and comparable".to_string(),
            }),
        }
    }

    fn flatten(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }
        let outer_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "flatten: argument must be a list".to_string(),
                })
            }
        };
        let outer_list = outer_rc.as_ref();

        // Use parallel processing for larger lists
        if should_parallelize(outer_list.len()) {
            // PARALLEL VERSION - parallel flatten
            let result: Vec<Value> = outer_list
                .par_iter()
                .flat_map(|elem| match elem {
                    Value::List(inner_rc) => inner_rc.iter().cloned().collect::<Vec<_>>(),
                    other => vec![other.clone()],
                })
                .collect();
            Ok(Value::List(result.into()))
        } else {
            // SEQUENTIAL VERSION for small lists
            let mut result = Vec::new();
            for elem in outer_list.iter() {
                match elem {
                    Value::List(inner_rc) => result.extend(inner_rc.iter().cloned()),
                    other => result.push(other.clone()),
                }
            }
            Ok(Value::List(result.into()))
        }
    }

    fn chunk(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }
        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "chunk: first argument must be a list".to_string(),
                })
            }
        };
        let size = match &args[1] {
            Value::Integer(n) if *n > 0 => *n as usize,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "chunk: size must be a positive integer".to_string(),
                })
            }
        };
        let mut chunks = Vec::new();
        let mut idx = 0;
        let list = list_rc.as_ref();
        while idx < list.len() {
            let end = (idx + size).min(list.len());
            chunks.push(Value::List(list[idx..end].to_vec().into()));
            idx = end;
        }
        Ok(Value::List(chunks.into()))
    }

    fn enumerate_list(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }
        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "enumerate: argument must be a list".to_string(),
                })
            }
        };
        let mut result = Vec::new();
        for (idx, item) in list_rc.iter().enumerate() {
            result.push(Value::Tuple(
                vec![Value::Integer(idx as i64), item.clone()].into(),
            ));
        }
        Ok(Value::List(result.into()))
    }

    fn find(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }
        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "find: first argument must be a list".to_string(),
                })
            }
        };
        let predicate = &args[1];
        for item in list_rc.iter() {
            let res = interpreter.call_function(predicate.clone(), vec![item.clone()])?;
            if matches!(res, Value::Boolean(true)) {
                return Ok(Value::Ok(Box::new(item.clone())));
            }
        }
        Ok(Value::Err(Box::new(Value::String(
            "NotFound".to_string().into(),
        ))))
    }

    fn starts_with(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }
        let text = match &args[0] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "starts_with: first argument must be a string".to_string(),
                })
            }
        };
        let prefix = match &args[1] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "starts_with: second argument must be a string".to_string(),
                })
            }
        };
        Ok(Value::Boolean(text.starts_with(prefix.as_str())))
    }

    fn ends_with(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }
        let text = match &args[0] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "ends_with: first argument must be a string".to_string(),
                })
            }
        };
        let suffix = match &args[1] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "ends_with: second argument must be a string".to_string(),
                })
            }
        };
        Ok(Value::Boolean(text.ends_with(suffix.as_str())))
    }

    fn group_by(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }
        let list_rc = match &args[0] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "group_by: first argument must be a list".to_string(),
                })
            }
        };
        let key_fn = &args[1];
        use std::collections::HashMap;
        let mut groups: HashMap<String, Vec<Value>> = HashMap::new();
        for item in list_rc.iter() {
            let key_val = interpreter.call_function(key_fn.clone(), vec![item.clone()])?;
            let key_str = key_val.to_string();
            groups.entry(key_str).or_default().push(item.clone());
        }
        // Convert groups to list of tuples (key, list)
        let mut out = Vec::new();
        for (k, v) in groups.into_iter() {
            out.push(Value::Tuple(
                vec![Value::String(k.into()), Value::List(v.into())].into(),
            ));
        }
        Ok(Value::List(out.into()))
    }

    fn set_parallel(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::Boolean(enabled) => {
                crate::parallel::set_parallel_enabled(*enabled);
                Ok(Value::Unit)
            }
            _ => Err(InterpreterError::TypeError {
                message: "set_parallel: argument must be boolean".to_string(),
            }),
        }
    }

    fn take_lazy(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }
        let list = &args[0];
        let n = match &args[1] {
            Value::Integer(i) if *i >= 0 => *i as usize,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "take: second argument must be a non-negative integer".to_string(),
                })
            }
        };
        let config = interpreter.get_lazy_config();
        let source_handle = crate::internal::utils::value_to_handle(list.clone(), config);
        let mut lazy_val = {
            // Use the existing utility for take
            let source_internal = source_handle.get_internal();
            crate::internal::LazyValue::Thunk(std::sync::Arc::new(move |interpreter| {
                let source_value = crate::internal::InternalValue::force(&source_internal, interpreter)?;
                match source_value {
                    Value::List(items) => {
                        let taken: Vec<_> = items.iter().take(n).cloned().collect();
                        Ok(Value::List(taken.into()))
                    }
                    _ => Err(crate::interpreter::InterpreterError::TypeError {
                        message: "take: argument must be a list".to_string(),
                    }),
                }
            }))
        };
        // Fusion logic
        if config.fusion_enabled {
            if let crate::internal::InternalValue::Lazy(ref prev_lazy) = *source_handle.get_internal() {
                if let Some(fused) = crate::internal::try_fuse_operations(prev_lazy, "take", None) {
                    lazy_val = fused;
                }
            }
        }
        let lazy_handle = crate::internal::ValueHandle::new_lazy(lazy_val);
        lazy_handle.get(interpreter)
    }

    fn skip_lazy(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }
        let list = &args[0];
        let n = match &args[1] {
            Value::Integer(i) if *i >= 0 => *i as usize,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "skip: second argument must be a non-negative integer".to_string(),
                })
            }
        };
        let config = interpreter.get_lazy_config();
        let source_handle = crate::internal::utils::value_to_handle(list.clone(), config);
        let mut lazy_val = {
            // Use the existing utility for skip
            let source_internal = source_handle.get_internal();
            crate::internal::LazyValue::Thunk(std::sync::Arc::new(move |interpreter| {
                let source_value = crate::internal::InternalValue::force(&source_internal, interpreter)?;
                match source_value {
                    Value::List(items) => {
                        let skipped: Vec<_> = items.iter().skip(n).cloned().collect();
                        Ok(Value::List(skipped.into()))
                    }
                    _ => Err(crate::interpreter::InterpreterError::TypeError {
                        message: "skip: argument must be a list".to_string(),
                    }),
                }
            }))
        };
        // Fusion logic
        if config.fusion_enabled {
            if let crate::internal::InternalValue::Lazy(ref prev_lazy) = *source_handle.get_internal() {
                if let Some(fused) = crate::internal::try_fuse_operations(prev_lazy, "skip", None) {
                    lazy_val = fused;
                }
            }
        }
        let lazy_handle = crate::internal::ValueHandle::new_lazy(lazy_val);
        lazy_handle.get(interpreter)
    }

    fn force_value(
        &self,
        args: Vec<Value>,
        _interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        // Force evaluation of any lazy values
        // Since we're working with the public Value interface, this is mostly a no-op
        // The real lazy forcing happens at the ValueHandle level
        Ok(crate::internal::utils::force_if_lazy(args[0].clone()))
    }

    fn make_lazy(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        // Wrap the value in a lazy container if appropriate
        let config = interpreter.get_lazy_config();
        let value = &args[0];

        if crate::internal::utils::should_be_lazy(value, config) {
            // Create a lazy thunk that just returns the value
            let lazy_handle = crate::internal::ValueHandle::new_lazy(
                crate::internal::LazyValue::Thunk(std::sync::Arc::new({
                    let val = value.clone();
                    move |_| Ok(val.clone())
                })),
            );
            lazy_handle.get(interpreter)
        } else {
            Ok(value.clone())
        }
    }

    fn concat_lazy(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let list1 = &args[0];
        let list2 = &args[1];

        // Check if we should use lazy evaluation
        let config = interpreter.get_lazy_config();
        if config.lazy_by_default {
            // Use lazy concatenation for any list size when lazy is enabled
            let first_handle = crate::internal::utils::value_to_handle(list1.clone(), config);
            let second_handle = crate::internal::utils::value_to_handle(list2.clone(), config);
            let lazy_concat = crate::internal::create_lazy_concat(first_handle, second_handle);
            let lazy_handle = crate::internal::ValueHandle::new_lazy(lazy_concat);
            return lazy_handle.get(interpreter);
        }

        // Fall back to eager evaluation
        let (a, b) = match (list1, list2) {
            (Value::List(a), Value::List(b)) => (a, b),
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "concat: arguments must be lists".to_string(),
                })
            }
        };
        let mut combined: Vec<Value> = a.as_ref().to_vec();
        combined.extend(b.iter().cloned());
        Ok(Value::List(combined.into()))
    }

    fn map_filtered(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 3 {
            return Err(InterpreterError::ArityMismatch {
                expected: 3,
                got: args.len(),
            });
        }
        let list = &args[0];
        let predicate = &args[1];
        let function = &args[2];

        let list_values = match list {
            Value::List(items) => items.as_ref().to_vec(),
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                // Convert range to vector of integers
                let end_val = if *inclusive { end.saturating_add(1) } else { *end };
                (*start..end_val).map(Value::Integer).collect()
            }
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_filtered: first argument must be a list or range".to_string(),
                })
            }
        };

        // Check if we should use lazy evaluation
        let config = interpreter.get_lazy_config();
        if config.lazy_by_default && list_values.len() > config.lazy_threshold {
            if let (Value::Function(func), Value::Function(pred)) = (function, predicate) {
                let source_handle = crate::internal::utils::value_to_handle(list.clone(), config);
                let mut lazy_val = crate::internal::create_lazy_map_filtered(source_handle.clone(), func.clone(), pred.clone());
                // Fusion logic: if the source is already a lazy value, try to fuse
                if config.fusion_enabled {
                    if let crate::internal::InternalValue::Lazy(ref prev_lazy) = *source_handle.get_internal() {
                        if let Some(fused) = crate::internal::try_fuse_operations(prev_lazy, "map_filtered", Some(func.clone())) {
                            lazy_val = fused;
                        }
                    }
                }
                let lazy_handle = crate::internal::ValueHandle::new_lazy(lazy_val);
                return lazy_handle.get(interpreter);
            }
        }

        // Sequential evaluation - parallel disabled due to interpreter cloning overhead
        let mut result = Vec::new();
        for item in list_values.iter() {
            let pred = interpreter.call_function(predicate.clone(), vec![item.clone()])?;
            if matches!(pred, Value::Boolean(true)) {
                let func_res = interpreter.call_function(function.clone(), vec![item.clone()])?;
                result.push(func_res);
            }
        }
        Ok(Value::List(result.into()))
    }

    fn map_get(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let map = match &args[0] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_get: first argument must be a map".to_string(),
                })
            }
        };

        let key = match &args[1] {
            Value::String(s) => s.as_ref(),
            Value::Integer(i) => &i.to_string(),
            Value::Float(f) => &f.to_string(),
            Value::Boolean(b) => &b.to_string(),
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_get: key must be string, integer, float, or boolean".to_string(),
                })
            }
        };

        Ok(map.get(key).cloned().unwrap_or(Value::Unit))
    }

    fn map_set(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 3 {
            return Err(InterpreterError::ArityMismatch {
                expected: 3,
                got: args.len(),
            });
        }

        let map_ref = match &args[0] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_set: first argument must be a map".to_string(),
                })
            }
        };

        let key = match &args[1] {
            Value::String(s) => s.as_ref().clone(),
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Boolean(b) => b.to_string(),
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_set: key must be string, integer, float, or boolean".to_string(),
                })
            }
        };

        let value = args[2].clone();

        // Create a new map with the updated value
        let mut new_map = (**map_ref).clone();
        new_map.insert(key, value);

        Ok(Value::Map(std::sync::Arc::new(new_map)))
    }

    fn map_has_key(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let map = match &args[0] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_has_key: first argument must be a map".to_string(),
                })
            }
        };

        let key = match &args[1] {
            Value::String(s) => s.as_ref(),
            Value::Integer(i) => &i.to_string(),
            Value::Float(f) => &f.to_string(),
            Value::Boolean(b) => &b.to_string(),
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_has_key: key must be string, integer, float, or boolean".to_string(),
                })
            }
        };

        Ok(Value::Boolean(map.contains_key(key)))
    }

    fn map_keys(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        let map = match &args[0] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_keys: argument must be a map".to_string(),
                })
            }
        };

        let keys: Vec<Value> = map
            .keys()
            .map(|k| Value::String(std::sync::Arc::new(k.clone())))
            .collect();

        Ok(Value::List(std::sync::Arc::from(keys)))
    }

    fn map_values(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        let map = match &args[0] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_values: argument must be a map".to_string(),
                })
            }
        };

        let values: Vec<Value> = map.values().cloned().collect();

        Ok(Value::List(std::sync::Arc::from(values)))
    }

    fn map_remove(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let map_ref = match &args[0] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_remove: first argument must be a map".to_string(),
                })
            }
        };

        let key = match &args[1] {
            Value::String(s) => s.as_ref(),
            Value::Integer(i) => &i.to_string(),
            Value::Float(f) => &f.to_string(),
            Value::Boolean(b) => &b.to_string(),
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_remove: key must be string, integer, float, or boolean".to_string(),
                })
            }
        };

        // Create a new map without the specified key
        let mut new_map = (**map_ref).clone();
        new_map.remove(key);

        Ok(Value::Map(std::sync::Arc::new(new_map)))
    }

    fn map_len(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        let map = match &args[0] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_len: argument must be a map".to_string(),
                })
            }
        };

        Ok(Value::Integer(map.len() as i64))
    }

    fn map_clear(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::Map(_) => {
                // Return an empty map
                Ok(Value::Map(std::sync::Arc::new(std::collections::HashMap::new())))
            }
            _ => Err(InterpreterError::TypeError {
                message: "map_clear: argument must be a map".to_string(),
            }),
        }
    }

    fn map_merge(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let map1 = match &args[0] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_merge: first argument must be a map".to_string(),
                })
            }
        };

        let map2 = match &args[1] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_merge: second argument must be a map".to_string(),
                })
            }
        };

        // Create a new map that merges both maps (map2 values overwrite map1 values)
        let mut new_map = (**map1).clone();
        for (key, value) in map2.iter() {
            new_map.insert(key.clone(), value.clone());
        }

        Ok(Value::Map(std::sync::Arc::new(new_map)))
    }

    // Result type utility functions

    /// Unwrap a Result value, panicking on Err
    /// Usage: unwrap(result) -> T
    fn unwrap_result(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::Ok(inner) => Ok(*inner.clone()),
            Value::Err(err) => Err(InterpreterError::RuntimeError {
                message: format!("Unwrap failed on error: {}", err),
            }),
            _ => Err(InterpreterError::TypeError {
                message: "unwrap: argument must be a Result type (Ok or Err)".to_string(),
            }),
        }
    }

    /// Unwrap a Result value with a default value for Err
    /// Usage: unwrap_or(result, default_value) -> T
    fn unwrap_or(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::Ok(inner) => Ok(*inner.clone()),
            Value::Err(_) => Ok(args[1].clone()),
            _ => Err(InterpreterError::TypeError {
                message: "unwrap_or: first argument must be a Result type (Ok or Err)".to_string(),
            }),
        }
    }

    /// Unwrap a Result value with a function to handle Err
    /// Usage: unwrap_or_else(result, error_handler_fn) -> T
    fn unwrap_or_else(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::Ok(inner) => Ok(*inner.clone()),
            Value::Err(err) => {
                let handler = &args[1];
                interpreter.call_function(handler.clone(), vec![*err.clone()])
            }
            _ => Err(InterpreterError::TypeError {
                message: "unwrap_or_else: first argument must be a Result type (Ok or Err)".to_string(),
            }),
        }
    }

    /// Check if a Result is Ok
    /// Usage: is_ok(result) -> Bool
    fn is_ok(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::Ok(_) => Ok(Value::Boolean(true)),
            Value::Err(_) => Ok(Value::Boolean(false)),
            _ => Err(InterpreterError::TypeError {
                message: "is_ok: argument must be a Result type (Ok or Err)".to_string(),
            }),
        }
    }

    /// Check if a Result is Err
    /// Usage: is_err(result) -> Bool
    fn is_err(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }

        match &args[0] {
            Value::Ok(_) => Ok(Value::Boolean(false)),
            Value::Err(_) => Ok(Value::Boolean(true)),
            _ => Err(InterpreterError::TypeError {
                message: "is_err: argument must be a Result type (Ok or Err)".to_string(),
            }),
        }
    }

    /// Map a function over the Ok value of a Result
    /// Usage: result_map(result, fn) -> Result<U, E>
    fn result_map(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let function = &args[1];

        match &args[0] {
            Value::Ok(inner) => {
                let mapped_value = interpreter.call_function(function.clone(), vec![*inner.clone()])?;
                Ok(Value::Ok(Box::new(mapped_value)))
            }
            Value::Err(err) => Ok(Value::Err(err.clone())),
            _ => Err(InterpreterError::TypeError {
                message: "result_map: first argument must be a Result type (Ok or Err)".to_string(),
            }),
        }
    }

    /// Map a function over the Err value of a Result
    /// Usage: result_map_err(result, fn) -> Result<T, F>
    fn result_map_err(
        &self,
        args: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let function = &args[1];

        match &args[0] {
            Value::Ok(inner) => Ok(Value::Ok(inner.clone())),
            Value::Err(err) => {
                let mapped_error = interpreter.call_function(function.clone(), vec![*err.clone()])?;
                Ok(Value::Err(Box::new(mapped_error)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "result_map_err: first argument must be a Result type (Ok or Err)".to_string(),
            }),
        }
    }


}

impl Clone for BuiltinFunctions {
    fn clone(&self) -> Self {
        Self {
            functions: self.functions.clone(),
        }
    }
}

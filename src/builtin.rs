use crate::ast::{BuiltinFunction, Value};
use crate::interpreter::InterpreterError;
use crate::parallel::should_parallelize;
#[cfg(feature = "native")]
use rayon::prelude::*;

// Sequential stand-ins for rayon's API on the wasm (playground) build, so
// the `should_parallelize(..)` call sites compile unchanged. They are never
// hot: should_parallelize is always false without the native feature.
#[cfg(not(feature = "native"))]
trait SeqParIter {
    type Item;
    fn par_iter(&self) -> std::slice::Iter<'_, Self::Item>;
}
#[cfg(not(feature = "native"))]
impl<T> SeqParIter for [T] {
    type Item = T;
    fn par_iter(&self) -> std::slice::Iter<'_, T> {
        self.iter()
    }
}
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
            "par_map".to_string(),
            BuiltinFunction {
                name: "par_map".to_string(),
                arity: 2,
            },
        );

        functions.insert(
            "par_filter".to_string(),
            BuiltinFunction {
                name: "par_filter".to_string(),
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

        // to_int / to_float were dispatched in call() but never registered
        // here, so they worked from the tier but were "undefined variable"
        // as plain identifiers
        functions.insert(
            "to_int".to_string(),
            BuiltinFunction {
                name: "to_int".to_string(),
                arity: 1,
            },
        );
        functions.insert(
            "to_float".to_string(),
            BuiltinFunction {
                name: "to_float".to_string(),
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

        functions.insert(
            "implements".to_string(),
            BuiltinFunction {
                name: "implements".to_string(),
                arity: 2,
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

        // Display rendering and iteration helpers
        functions.insert(
            "show".to_string(),
            BuiltinFunction {
                name: "show".to_string(),
                arity: 1,
            },
        );
        functions.insert(
            "entries".to_string(),
            BuiltinFunction {
                name: "entries".to_string(),
                arity: 1,
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
        Self::call_internal(builtins, name, arguments, interpreter)
    }

    fn call_internal(
        builtins: &BuiltinFunctions,
        name: &str,
        arguments: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        // The capability gate: one branch when no manifest is loaded;
        // otherwise the effectful modules check the caller's grant before
        // dispatch. Purity is never gated — see caps::check.
        if let Some(message) = interpreter.capability_denial(name) {
            return Err(InterpreterError::RuntimeError { message });
        }

        // The Open Timeline: in replay mode a recorded nondeterministic
        // call returns its logged result and the real effect is skipped;
        // in record mode the real call runs and its result is logged just
        // below. One branch when no timeline is attached.
        match interpreter.timeline_replay(name) {
            Some(Ok(value)) => return Ok(value),
            Some(Err(message)) => return Err(InterpreterError::RuntimeError { message }),
            None => {}
        }
        let timeline_active = interpreter.timeline_recording();
        if timeline_active {
            let result = Self::call_internal_inner(builtins, name, arguments, interpreter)?;
            interpreter.timeline_record(name, &result);
            return Ok(result);
        }

        Self::call_internal_inner(builtins, name, arguments, interpreter)
    }

    fn call_internal_inner(
        builtins: &BuiltinFunctions,
        name: &str,
        arguments: Vec<Value>,
        interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        // Handle filesystem functions
        #[cfg(feature = "native")]
        if let Some(fs_function) = name.strip_prefix("fs.") {
            // Remove "fs." prefix
            return crate::stdlib::fs::call_fs_function(fs_function, arguments).map_err(|e| {
                InterpreterError::RuntimeError {
                    message: e.to_string(),
                }
            });
        }

        // Handle HTTP functions
        #[cfg(feature = "native")]
        if let Some(http_function) = name.strip_prefix("http.") {
            // `serve` runs a blocking server that calls back into an olang
            // handler on every request, so it needs the interpreter — it
            // cannot go through the interpreter-less dispatch below.
            if http_function == "serve" {
                return crate::stdlib::http::serve_blocking(arguments, interpreter);
            }
            return crate::stdlib::http::call_http_function(http_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Without the native feature (the browser playground), whole module
        // families don't exist: say so plainly instead of "unknown function".
        #[cfg(not(feature = "native"))]
        for gated in ["fs.", "http.", "os.", "db."] {
            if name.starts_with(gated) {
                return Err(InterpreterError::RuntimeError {
                    message: format!(
                        "{} is not available in the playground (no filesystem, network, \
                         processes, or database in the browser sandbox)",
                        name
                    ),
                });
            }
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

        // Handle chan functions
        if let Some(chan_function) = name.strip_prefix("chan.") {
            return crate::stdlib::chan::call_chan_function(chan_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Handle toml functions
        if let Some(toml_function) = name.strip_prefix("toml.") {
            return crate::stdlib::toml_mod::call_toml_function(toml_function, arguments).map_err(
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

        // Handle dom functions (browser-only; native errors clearly)
        if let Some(dom_function) = name.strip_prefix("dom.") {
            return crate::stdlib::dom::call_dom_function(dom_function, arguments).map_err(|e| {
                InterpreterError::RuntimeError {
                    message: e.to_string(),
                }
            });
        }

        // Handle time functions
        if let Some(time_function) = name.strip_prefix("time.") {
            return crate::stdlib::time::call_time_function(time_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Handle os functions
        #[cfg(feature = "native")]
        if let Some(os_function) = name.strip_prefix("os.") {
            // Remove "os." prefix
            return crate::stdlib::os::call_os_function(os_function, arguments).map_err(|e| {
                InterpreterError::RuntimeError {
                    message: e.to_string(),
                }
            });
        }

        // Handle proc functions (child processes + pipelines)
        #[cfg(feature = "native")]
        if let Some(proc_function) = name.strip_prefix("proc.") {
            return crate::stdlib::proc::call_proc_function(proc_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Handle crypto functions
        if let Some(crypto_function) = name.strip_prefix("crypto.") {
            // Remove "crypto." prefix
            return crate::stdlib::crypto::call_crypto_function(crypto_function, arguments)
                .map_err(|e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                });
        }

        // Handle db (SQLite) functions
        #[cfg(feature = "native")]
        if let Some(db_function) = name.strip_prefix("db.") {
            return crate::stdlib::db::call_db_function(db_function, arguments).map_err(|e| {
                InterpreterError::RuntimeError {
                    message: e.to_string(),
                }
            });
        }

        // Handle col (collections) functions — higher-order, so they take
        // the interpreter to invoke their function arguments
        if let Some(col_function) = name.strip_prefix("col.") {
            return crate::stdlib::collections::call_collections_function(
                col_function,
                arguments,
                interpreter,
            );
        }

        // Handle str functions
        if let Some(str_function) = name.strip_prefix("str.") {
            return crate::stdlib::string::call_string_function(str_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Handle re functions
        if let Some(re_function) = name.strip_prefix("re.") {
            return crate::stdlib::regex_mod::call_regex_function(re_function, arguments).map_err(
                |e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                },
            );
        }

        // Handle testing functions
        if let Some(testing_function) = name.strip_prefix("testing.") {
            // Remove "testing." prefix
            return crate::stdlib::testing::call_testing_function(testing_function, arguments)
                .map_err(|e| InterpreterError::RuntimeError {
                    message: e.to_string(),
                });
        }

        // OVM extension modules (ods, ...): "<module>.<func>" dispatches
        // through the module registry. After the fixed prefixes above so
        // no existing module name can be shadowed.
        if let Some((module_name, module_function)) = name.split_once('.')
            && let Some(module) = crate::native::module_named(module_name)
        {
            return module
                .dispatch(module_function, arguments)
                .map_err(|message| InterpreterError::RuntimeError { message });
        }

        match name {
            "println" => builtins.println(arguments),
            "print" => builtins.print(arguments),
            "map" => builtins.map(arguments, interpreter),
            "filter" => builtins.filter(arguments, interpreter),
            "par_map" => builtins.par_map(arguments, interpreter),
            "par_filter" => builtins.par_filter(arguments, interpreter),
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
            "implements" => {
                if arguments.len() != 2 {
                    return Err(InterpreterError::ArityMismatch {
                        expected: 2,
                        got: arguments.len(),
                    });
                }
                let trait_name = match &arguments[1] {
                    Value::String(s) => s.to_string(),
                    other => {
                        return Err(InterpreterError::TypeError {
                            message: format!(
                                "implements: second argument must be a trait name string, got {}",
                                other.type_name()
                            ),
                        });
                    }
                };
                Ok(Value::Boolean(
                    interpreter.value_implements(&arguments[0], &trait_name),
                ))
            }
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
            "show" => builtins.show(arguments),
            "entries" => builtins.entries(arguments),
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
            crate::output::emit_line("");
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
            crate::output::emit_line(&output);
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
            Value::String(s) => crate::output::emit(s.as_str()),
            other => crate::output::emit(&format!("{}", other)),
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
                let end_val = if *inclusive {
                    end.saturating_add(1)
                } else {
                    *end
                };
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

                return Self::process_map_range(range_vec, function, interpreter);
            }
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map: first argument must be a list or range".to_string(),
                });
            }
        };

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
                let end_val = if *inclusive {
                    end.saturating_add(1)
                } else {
                    *end
                };
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
                });
            }
        };

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

    /// Materialize `par_map`/`par_filter`'s first argument, mirroring the
    /// sequential builtins' checks and messages.
    fn parallel_input(op: &str, value: &Value) -> Result<Vec<Value>, InterpreterError> {
        match value {
            Value::List(items) => Ok(items.as_ref().to_vec()),
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                let end_val = if *inclusive {
                    end.saturating_add(1)
                } else {
                    *end
                };
                let range_size = end_val.saturating_sub(*start).max(0) as usize;
                if range_size > 10_000_000 {
                    return Err(InterpreterError::RuntimeError {
                        message: format!(
                            "Range size ({}) too large, this could cause memory issues. Maximum range size is 10000000.",
                            range_size
                        ),
                    });
                }
                Ok((*start..end_val).map(Value::Integer).collect())
            }
            _ => Err(InterpreterError::TypeError {
                message: format!("{}: first argument must be a list or range", op),
            }),
        }
    }

    /// Apply `function` to every item, fanning contiguous chunks out to one
    /// interpreter clone per worker thread (this is the fix for why `map`
    /// went sequential: cloning per *element* was ruinous; cloning per
    /// *worker* is O(cores) and im-map clones are cheap). Order is
    /// preserved. On error, the one reported is the error the sequential
    /// twin would have hit first (lowest index) — but unlike the sequential
    /// twin, later elements may already have been evaluated.
    ///
    /// Every chunk runs on a clone — including the single-threaded
    /// fallback — so `function` always sees spawn's snapshot semantics:
    /// mutations to enclosing state never reach the caller, on any machine.
    fn parallel_apply(
        items: &[Value],
        function: &Value,
        interpreter: &crate::interpreter::Interpreter,
    ) -> Result<Vec<Value>, InterpreterError> {
        if items.is_empty() {
            return Ok(Vec::new());
        }

        #[cfg(feature = "native")]
        {
            let workers = crate::parallel::get_config()
                .max_threads
                .clamp(1, items.len());
            if workers > 1 {
                let chunk_size = items.len().div_ceil(workers);
                let joined: Vec<Result<Vec<Value>, (usize, InterpreterError)>> =
                    std::thread::scope(|scope| {
                        let handles: Vec<_> = items
                            .chunks(chunk_size)
                            .enumerate()
                            .map(|(chunk_idx, chunk)| {
                                let mut worker = interpreter.thread_safe_clone();
                                let function = function.clone();
                                scope.spawn(move || {
                                    let mut out = Vec::with_capacity(chunk.len());
                                    for (i, item) in chunk.iter().enumerate() {
                                        match worker
                                            .call_function_optimized(&function, vec![item.clone()])
                                        {
                                            Ok(v) => out.push(v),
                                            Err(e) => return Err((chunk_idx * chunk_size + i, e)),
                                        }
                                    }
                                    Ok(out)
                                })
                            })
                            .collect();
                        handles
                            .into_iter()
                            .map(|h| {
                                h.join().unwrap_or_else(|_| {
                                    Err((
                                        usize::MAX,
                                        InterpreterError::RuntimeError {
                                            message: "parallel worker thread panicked".to_string(),
                                        },
                                    ))
                                })
                            })
                            .collect()
                    });

                let mut chunks_ok = Vec::new();
                let mut first_err: Option<(usize, InterpreterError)> = None;
                for result in joined {
                    match result {
                        Ok(values) => chunks_ok.push(values),
                        Err((idx, e)) => {
                            if first_err.as_ref().is_none_or(|(seen, _)| idx < *seen) {
                                first_err = Some((idx, e));
                            }
                        }
                    }
                }
                if let Some((_, e)) = first_err {
                    return Err(e);
                }
                return Ok(chunks_ok.concat());
            }
        }

        // One core, or the playground (no threads): same snapshot
        // semantics on a single clone.
        let mut worker = interpreter.thread_safe_clone();
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push(worker.call_function_optimized(function, vec![item.clone()])?);
        }
        Ok(out)
    }

    fn par_map(
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
        let items = Self::parallel_input("par_map", &args[0])?;
        let results = Self::parallel_apply(&items, &args[1], interpreter)?;
        Ok(Value::List(results.into()))
    }

    fn par_filter(
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
        let items = Self::parallel_input("par_filter", &args[0])?;
        let verdicts = Self::parallel_apply(&items, &args[1], interpreter)?;
        // Same acceptance rule as filter: keep on Boolean(true), drop on
        // anything else.
        let kept: Vec<Value> = items
            .into_iter()
            .zip(verdicts)
            .filter_map(|(item, verdict)| matches!(verdict, Value::Boolean(true)).then_some(item))
            .collect();
        Ok(Value::List(kept.into()))
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
                });
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
                });
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
            // `as i64` saturates NaN to 0 and infinities to i64::MIN/MAX,
            // silently inventing a number. A non-finite float has no integer
            // value, so reject it instead.
            Value::Float(x) => {
                if x.is_finite() {
                    Ok(Value::Integer(*x as i64))
                } else {
                    Err(InterpreterError::TypeError {
                        message: format!(
                            "to_int: cannot convert {} to an integer",
                            crate::ast::format_float(*x)
                        ),
                    })
                }
            }
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
                        });
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
                        crate::log::get_logger().warn(
                            "builtin",
                            &format!(
                                "Generating very large range ({}), consider using lazy evaluation",
                                end
                            ),
                        );
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
                        });
                    }
                };

                let end = match &args[1] {
                    Value::Integer(n) => *n,
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "range: arguments must be integers".to_string(),
                        });
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
                        });
                    }
                };

                let end = match &args[1] {
                    Value::Integer(n) => *n,
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "range: arguments must be integers".to_string(),
                        });
                    }
                };

                let step = match &args[2] {
                    Value::Integer(n) => *n,
                    _ => {
                        return Err(InterpreterError::TypeError {
                            message: "range: arguments must be integers".to_string(),
                        });
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
                });
            }
        };
        let list1 = list1_rc.as_ref();

        let list2_rc = match &args[1] {
            Value::List(items) => items,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "zip: arguments must be lists".to_string(),
                });
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
                });
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
                });
            }
        };
        let list = list_rc.as_ref();

        // Sorting needs a total order over comparable elements. Numbers
        // (Int/Float) compare with each other and strings compare with
        // strings, but a list mixing numbers and strings (or containing
        // booleans, lists, structs, …) has no meaningful order — the old
        // comparator silently declared such pairs Equal and returned a
        // half-sorted list. Reject a heterogeneous list up front.
        #[derive(PartialEq)]
        enum SortClass {
            Numeric,
            Str,
            Other,
        }
        fn class_of(v: &Value) -> SortClass {
            match v {
                Value::Integer(_) | Value::Float(_) => SortClass::Numeric,
                Value::String(_) => SortClass::Str,
                _ => SortClass::Other,
            }
        }
        if let Some(first) = list.first() {
            let cls = class_of(first);
            if cls == SortClass::Other {
                return Err(InterpreterError::TypeError {
                    message: format!("sort: cannot compare values of type {}", first.type_name()),
                });
            }
            for item in list.iter() {
                if class_of(item) != cls {
                    return Err(InterpreterError::TypeError {
                        message: "sort: cannot compare values of different types (mix of numbers and strings)".to_string(),
                    });
                }
            }
        }

        let mut result: Vec<Value> = list.to_vec();

        fn sort_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
            match (a, b) {
                // total_cmp (not partial_cmp) gives floats a *total* order:
                // partial_cmp returns None for NaN, and the old
                // unwrap_or(Equal) made NaN compare equal to everything,
                // which is inconsistent and leaves even the finite elements
                // unsorted. total_cmp keeps finite values correctly ordered
                // and sinks NaN to one end.
                (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
                (Value::Float(x), Value::Float(y)) => x.total_cmp(y),
                (Value::String(x), Value::String(y)) => x.cmp(y),
                (Value::Integer(x), Value::Float(y)) => (*x as f64).total_cmp(y),
                (Value::Float(x), Value::Integer(y)) => x.total_cmp(&(*y as f64)),
                _ => std::cmp::Ordering::Equal,
            }
        }

        // Parallel sort for larger lists; always sequential without rayon.
        #[cfg(feature = "native")]
        if should_parallelize(result.len()) {
            result.par_sort_by(sort_cmp);
        } else {
            result.sort_by(sort_cmp);
        }
        #[cfg(not(feature = "native"))]
        result.sort_by(sort_cmp);

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
                });
            }
        };
        let list = list_rc.as_ref();

        let separator_rc = match &args[1] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "join: second argument must be a string".to_string(),
                });
            }
        };
        let separator = separator_rc.as_ref();

        let result = list
            .iter()
            .map(|item| match item {
                // Render string elements by their content, not their quoted
                // debug form — `join(["a", "b"], ",")` is `a,b`, not `"a","b"`.
                Value::String(s) => s.as_ref().clone(),
                other => other.to_string(),
            })
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
                });
            }
        };
        let string = string_rc.as_ref();

        let separator_rc = match &args[1] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "split: second argument must be a string".to_string(),
                });
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
                });
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
                let end_val = if *inclusive {
                    end.saturating_add(1)
                } else {
                    *end
                };
                let values: Vec<Value> = (*start..end_val).map(Value::Integer).collect();
                let use_parallel = should_parallelize(values.len());
                (values, use_parallel)
            }
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "sum: argument must be a list or range".to_string(),
                });
            }
        };

        // Parallelism follows the configured threshold like every other
        // builtin — no operation-specific hardcoded cutoffs.
        let should_use_parallel = crate::parallel::should_parallelize(list_values.len());

        if should_use_parallel {
            // PARALLEL VERSION - parallel sum with fold and reduce.
            // (rayon's try_fold/try_reduce have no sequential lookalike, so
            // this arm is compiled out on the wasm build, where
            // should_parallelize is always false.)
            #[cfg(not(feature = "native"))]
            unreachable!("should_parallelize is false without the native feature");
            #[cfg(feature = "native")]
            {
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
                        });
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
                });
            }
        };
        if list_rc.is_empty() {
            return Err(InterpreterError::TypeError {
                message: "average: cannot take average of an empty list".to_string(),
            });
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
                });
            }
        };
        if list_rc.is_empty() {
            return Err(InterpreterError::TypeError {
                message: "min: cannot take min of an empty list".to_string(),
            });
        }
        use std::cmp::Ordering;
        let mut min = list_rc[0].clone();
        for item in list_rc.iter().skip(1) {
            if let Some(ord) = item.compare_for_sort(&min)
                && ord == Ordering::Less
            {
                min = item.clone();
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
                });
            }
        };
        if list_rc.is_empty() {
            return Err(InterpreterError::TypeError {
                message: "max: cannot take max of an empty list".to_string(),
            });
        }
        use std::cmp::Ordering;
        let mut max = list_rc[0].clone();
        for item in list_rc.iter().skip(1) {
            if let Some(ord) = item.compare_for_sort(&max)
                && ord == Ordering::Greater
            {
                max = item.clone();
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
                });
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
                });
            }
        };
        let size = match &args[1] {
            Value::Integer(n) if *n > 0 => *n as usize,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "chunk: size must be a positive integer".to_string(),
                });
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
                });
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
                });
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
                });
            }
        };
        let prefix = match &args[1] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "starts_with: second argument must be a string".to_string(),
                });
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
                });
            }
        };
        let suffix = match &args[1] {
            Value::String(s) => s,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "ends_with: second argument must be a string".to_string(),
                });
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
                });
            }
        };
        let key_fn = &args[1];
        use std::collections::HashMap;
        // Group by the key's VALUE, preserving its type (an Int key stays an
        // Int, a String key stays a bare string), in first-seen order so the
        // result is deterministic. The HashMap holds only a type-tagged
        // canonical string to locate a group in O(1); the emitted key is the
        // original value. (Tagging keeps `0` and `"0"` in separate groups.)
        fn canonical_key(v: &Value) -> String {
            match v {
                Value::Integer(i) => format!("i:{i}"),
                Value::Float(f) => format!("f:{}", f.to_bits()),
                Value::String(s) => format!("s:{s}"),
                Value::Boolean(b) => format!("b:{b}"),
                Value::Unit => "u:".to_string(),
                other => format!("o:{other}"),
            }
        }
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut order: Vec<(Value, Vec<Value>)> = Vec::new();
        for item in list_rc.iter() {
            let key_val = interpreter.call_function(key_fn.clone(), vec![item.clone()])?;
            let canon = canonical_key(&key_val);
            match index.get(&canon) {
                Some(&pos) => order[pos].1.push(item.clone()),
                None => {
                    index.insert(canon, order.len());
                    order.push((key_val, vec![item.clone()]));
                }
            }
        }
        let out: Vec<Value> = order
            .into_iter()
            .map(|(k, v)| Value::Tuple(vec![k, Value::List(v.into())].into()))
            .collect();
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
        _interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }
        let n = match &args[1] {
            Value::Integer(i) if *i >= 0 => *i as usize,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "take: second argument must be a non-negative integer".to_string(),
                });
            }
        };
        match &args[0] {
            Value::List(items) => {
                let taken: Vec<_> = items.iter().take(n).cloned().collect();
                Ok(Value::List(taken.into()))
            }
            _ => Err(InterpreterError::TypeError {
                message: "take: argument must be a list".to_string(),
            }),
        }
    }

    fn skip_lazy(
        &self,
        args: Vec<Value>,
        _interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }
        let n = match &args[1] {
            Value::Integer(i) if *i >= 0 => *i as usize,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "skip: second argument must be a non-negative integer".to_string(),
                });
            }
        };
        match &args[0] {
            Value::List(items) => {
                let skipped: Vec<_> = items.iter().skip(n).cloned().collect();
                Ok(Value::List(skipped.into()))
            }
            _ => Err(InterpreterError::TypeError {
                message: "skip: argument must be a list".to_string(),
            }),
        }
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

        // olang evaluates eagerly: by the time a builtin sees its argument it
        // is already a value, so `force` is the identity. Kept for source
        // compatibility; the book documents this honestly.
        Ok(args[0].clone())
    }

    fn make_lazy(
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
        // A builtin receives its argument already evaluated, so `lazy` cannot
        // defer anything — it is the identity, and always was (the previous
        // implementation built a thunk and immediately forced it). Kept for
        // source compatibility; the book documents this honestly.
        Ok(args[0].clone())
    }

    fn concat_lazy(
        &self,
        args: Vec<Value>,
        _interpreter: &mut crate::interpreter::Interpreter,
    ) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let list1 = &args[0];
        let list2 = &args[1];

        let (a, b) = match (list1, list2) {
            (Value::List(a), Value::List(b)) => (a, b),
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "concat: arguments must be lists".to_string(),
                });
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
                let end_val = if *inclusive {
                    end.saturating_add(1)
                } else {
                    *end
                };
                (*start..end_val).map(Value::Integer).collect()
            }
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_filtered: first argument must be a list or range".to_string(),
                });
            }
        };

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

    /// A read-only field-map view over a map or any struct-like value.
    /// Lets the `map_*` accessors work uniformly on maps, anonymous objects,
    /// structs, and parsed JSON objects — anything with named fields reads the
    /// same way, so dynamic key access on parsed JSON is possible.
    fn field_map(value: &Value) -> Option<&HashMap<String, Value>> {
        match value {
            Value::Map(m) => Some(m.as_ref()),
            Value::Struct { fields, .. } => Some(fields),
            _ => None,
        }
    }

    /// Display rendering: strings render bare (no quotes), everything else
    /// exactly as `to_string`. `to_string` keeps its repr form — `show` is
    /// what you want when building output for people.
    fn show(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }
        let text = match &args[0] {
            Value::String(s) => s.as_ref().clone(),
            other => other.to_string(),
        };
        Ok(Value::String(std::sync::Arc::new(text)))
    }

    /// The (key, value) pairs of a map or any struct-like value, as a list
    /// of tuples sorted by key — deterministic order, made for
    /// `for (k, v) in entries(m)`.
    fn entries(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 1 {
            return Err(InterpreterError::ArityMismatch {
                expected: 1,
                got: args.len(),
            });
        }
        let map = match Self::field_map(&args[0]) {
            Some(map) => map,
            None => {
                return Err(InterpreterError::TypeError {
                    message: "entries: argument must be a map or object".to_string(),
                });
            }
        };
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        let pairs: Vec<Value> = keys
            .into_iter()
            .map(|k| {
                Value::Tuple(std::sync::Arc::from(vec![
                    Value::String(std::sync::Arc::new(k.clone())),
                    map.get(k).cloned().unwrap_or(Value::Unit),
                ]))
            })
            .collect();
        Ok(Value::List(std::sync::Arc::from(pairs)))
    }

    fn map_get(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let map = match Self::field_map(&args[0]) {
            Some(map) => map,
            None => {
                return Err(InterpreterError::TypeError {
                    message: "map_get: first argument must be a map or object".to_string(),
                });
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
                });
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

        let key = match &args[1] {
            Value::String(s) => s.as_ref().clone(),
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Boolean(b) => b.to_string(),
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_set: key must be string, integer, float, or boolean".to_string(),
                });
            }
        };

        let value = args[2].clone();

        // Writers mirror the readers' struct-likeness: updating a map yields
        // a map; updating an object, struct, or parsed JSON object yields a
        // new value of the same kind with the field set.
        match &args[0] {
            Value::Map(map_ref) => {
                let mut new_map = (**map_ref).clone();
                new_map.insert(key, value);
                Ok(Value::Map(std::sync::Arc::new(new_map)))
            }
            Value::Struct { type_name, fields } => {
                let mut new_fields = fields.clone();
                new_fields.insert(key, value);
                Ok(Value::Struct {
                    type_name: type_name.clone(),
                    fields: new_fields,
                })
            }
            _ => Err(InterpreterError::TypeError {
                message: "map_set: first argument must be a map or object".to_string(),
            }),
        }
    }

    fn map_has_key(&self, args: Vec<Value>) -> Result<Value, InterpreterError> {
        if args.len() != 2 {
            return Err(InterpreterError::ArityMismatch {
                expected: 2,
                got: args.len(),
            });
        }

        let map = match Self::field_map(&args[0]) {
            Some(map) => map,
            None => {
                return Err(InterpreterError::TypeError {
                    message: "map_has_key: first argument must be a map or object".to_string(),
                });
            }
        };

        let key = match &args[1] {
            Value::String(s) => s.as_ref(),
            Value::Integer(i) => &i.to_string(),
            Value::Float(f) => &f.to_string(),
            Value::Boolean(b) => &b.to_string(),
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_has_key: key must be string, integer, float, or boolean"
                        .to_string(),
                });
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

        let map = match Self::field_map(&args[0]) {
            Some(map) => map,
            None => {
                return Err(InterpreterError::TypeError {
                    message: "map_keys: argument must be a map or object".to_string(),
                });
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

        let map = match Self::field_map(&args[0]) {
            Some(map) => map,
            None => {
                return Err(InterpreterError::TypeError {
                    message: "map_values: argument must be a map or object".to_string(),
                });
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

        let key = match &args[1] {
            Value::String(s) => s.as_ref().clone(),
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Boolean(b) => b.to_string(),
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_remove: key must be string, integer, float, or boolean"
                        .to_string(),
                });
            }
        };

        // Like map_set, removal works on any struct-like value and returns a
        // new value of the same kind without the key.
        match &args[0] {
            Value::Map(map_ref) => {
                let mut new_map = (**map_ref).clone();
                new_map.remove(&key);
                Ok(Value::Map(std::sync::Arc::new(new_map)))
            }
            Value::Struct { type_name, fields } => {
                let mut new_fields = fields.clone();
                new_fields.remove(&key);
                Ok(Value::Struct {
                    type_name: type_name.clone(),
                    fields: new_fields,
                })
            }
            _ => Err(InterpreterError::TypeError {
                message: "map_remove: first argument must be a map or object".to_string(),
            }),
        }
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
                });
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
                Ok(Value::Map(std::sync::Arc::new(
                    std::collections::HashMap::new(),
                )))
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
                });
            }
        };

        let map2 = match &args[1] {
            Value::Map(map) => map,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "map_merge: second argument must be a map".to_string(),
                });
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
                message: "unwrap_or_else: first argument must be a Result type (Ok or Err)"
                    .to_string(),
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
                let mapped_value =
                    interpreter.call_function(function.clone(), vec![*inner.clone()])?;
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
                let mapped_error =
                    interpreter.call_function(function.clone(), vec![*err.clone()])?;
                Ok(Value::Err(Box::new(mapped_error)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "result_map_err: first argument must be a Result type (Ok or Err)"
                    .to_string(),
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

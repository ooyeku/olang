use crate::analyze::AnalysisError;
use crate::ast::{
    Argument, AsyncFunctionDecl, BinaryOp, Expr, Function, FunctionDecl, LetDecl, MatchArm, Pattern, Program,
    Statement, TypeDecl, UnaryOp, UseDecl, Value, ShareDecl, PromiseType, EnumVariantData, ErrorTypeDecl, BuiltinFunction,
};
use crate::async_runtime::AsyncRuntime;
use crate::builtin::BuiltinFunctions;
use crate::internal::{LazyConfig, check_memory_pressure};
use crate::ovm::gc::SafepointManager;
use crate::type_checker::TypeChecker;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum InterpreterError {
    #[error("Undefined variable: {name}")]
    UndefinedVariable { name: String },
    #[error("Type error: {message}")]
    TypeError { message: String },
    #[error("Runtime error: {message}")]
    RuntimeError { message: String },
    #[error("Arity mismatch: expected {expected}, got {got}")]
    ArityMismatch { expected: usize, got: usize },
    #[error("Pattern match failed")]
    PatternMatchFailed,
    
    // Enhanced lazy evaluation error types
    #[error("Lazy evaluation error: {message}")]
    LazyEvaluationError { message: String },
    #[error("Lazy evaluation timeout: operation exceeded {timeout_ms}ms")]
    LazyEvaluationTimeout { timeout_ms: u64 },
    #[error("Circular dependency detected in lazy evaluation: {cycle}")]
    CircularDependency { cycle: String },
    #[error("Memory limit exceeded during lazy evaluation: {current_mb}MB > {limit_mb}MB")]
    MemoryLimitExceeded { current_mb: usize, limit_mb: usize },
    #[error("Thread safety violation in lazy evaluation: {details}")]
    ThreadSafetyViolation { details: String },
    #[error("Lazy evaluation recovery failed: {original_error}")]
    RecoveryFailed { original_error: String },
    #[error("Force evaluation failed: {reason}")]
    ForceEvaluationFailed { reason: String },
    #[error("Lazy thunk corrupted: {thunk_id}")]
    ThunkCorrupted { thunk_id: String },
    #[error("Lazy evaluation chain too deep: {depth} > {max_depth}")]
    EvaluationChainTooDeep { depth: usize, max_depth: usize },
}

/// Configuration for module resolution debugging
#[derive(Debug, Clone)]
pub struct ModuleDebugConfig {
    pub enable_resolution_tracing: bool,
    pub log_search_paths: bool,
    pub show_resolution_timing: bool,
    pub verbose_error_messages: bool,
}

impl Default for ModuleDebugConfig {
    fn default() -> Self {
        Self {
            enable_resolution_tracing: std::env::var("OLANG_DEBUG_MODULES").is_ok(),
            log_search_paths: true,
            show_resolution_timing: false,
            verbose_error_messages: true,
        }
    }
}

pub struct Environment {
    variables: HashMap<String, Value>,
    parent: Option<Box<Environment>>,
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

impl Environment {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            parent: None,
        }
    }

    pub fn with_parent(parent: Environment) -> Self {
        Self {
            variables: HashMap::new(),
            parent: Some(Box::new(parent)),
        }
    }

    pub fn define(&mut self, name: String, value: Value) {
        self.variables.insert(name, value);
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(value) = self.variables.get(name) {
            Some(value.clone())
        } else if let Some(parent) = &self.parent {
            parent.get(name)
        } else {
            None
        }
    }

    pub fn set(&mut self, name: &str, value: Value) -> Result<(), InterpreterError> {
        if self.variables.contains_key(name) {
            self.variables.insert(name.to_string(), value);
            Ok(())
        } else if let Some(parent) = &mut self.parent {
            parent.set(name, value)
        } else {
            Err(InterpreterError::UndefinedVariable {
                name: name.to_string(),
            })
        }
    }

    /// Get all variables in this environment (excluding parent environments)
    pub fn get_all_variables(&self) -> &HashMap<String, Value> {
        &self.variables
    }
}

/// Olang interpreter with optional type checking
pub struct Interpreter {
    environment: Environment,
    builtin_functions: BuiltinFunctions,
    type_checker: Option<TypeChecker>,
    async_runtime: AsyncRuntime,
    lazy_config: LazyConfig,
    safepoint_manager: Arc<SafepointManager>,
    pub module_debug_config: ModuleDebugConfig,
    
    // Enhanced module system
    module_cache: HashMap<String, ModuleCacheEntry>,
    dependency_tracker: ModuleDependencyTracker,
    current_module_path: Option<String>, // For tracking current module during loading
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        let mut interpreter = Self {
            environment: Environment::new(),
            builtin_functions: BuiltinFunctions::new(),
            type_checker: None,
            async_runtime: AsyncRuntime::new(),
            lazy_config: LazyConfig::default(),
            safepoint_manager: Arc::new(SafepointManager::new()),
            module_debug_config: ModuleDebugConfig::default(),
            
            // Enhanced module system
            module_cache: HashMap::new(),
            dependency_tracker: ModuleDependencyTracker::new(),
            current_module_path: None, // For tracking current module during loading
        };

        // Register built-in functions
        interpreter.register_builtins();
        interpreter
    }

    /// Create a new interpreter with type checking enabled
    pub fn with_type_checking() -> Self {
        let mut interpreter = Self::new();
        interpreter.type_checker = Some(TypeChecker::new());
        interpreter
    }

    /// Enable or disable type checking
    pub fn set_type_checking(&mut self, enabled: bool) {
        if enabled {
            self.type_checker = Some(TypeChecker::new());
        } else {
            self.type_checker = None;
        }
    }

    /// Register built-in functions in the environment
    fn register_builtins(&mut self) {
        for (name, func) in self.builtin_functions.get_functions() {
            self.environment
                .define(name.clone(), Value::Builtin(func.clone()));
        }

        // Register stdlib modules
        let stdlib = crate::stdlib::get_stdlib();
        for (module_name, module_value) in stdlib {
            self.environment.define(module_name, module_value);
        }
    }

    /// Evaluate a program
    pub fn eval_program(&mut self, program: Program) -> Result<Value, InterpreterError> {
        // Optional type checking with proper error propagation
        if let Some(ref mut type_checker) = self.type_checker {
            if let Err(type_errors) = type_checker.check_program(&program) {
                // Convert type checking errors to proper InterpreterError
                let error_messages: Vec<String> = type_errors.iter().map(|e| format!("{:?}", e)).collect();
                let combined_message = error_messages.join("; ");
                
                return Err(InterpreterError::TypeError {
                    message: format!("Type checking failed: {}", combined_message),
                });
            }
        }

        let mut last_value = Value::Unit;
        for statement in program.statements {
            last_value = self.eval_statement(statement)?;
        }
        Ok(last_value)
    }

    pub fn eval_statement(&mut self, statement: Statement) -> Result<Value, InterpreterError> {
        // Safepoint poll for GC coordination
        self.safepoint_poll()?;

        match statement {
            Statement::Expression(expr) => self.eval_expr(expr),
            Statement::LetDecl(let_decl) => self.eval_let_decl(let_decl),
            Statement::FunctionDecl(func_decl) => self.eval_function_decl(func_decl),
            Statement::AsyncFunctionDecl(async_func_decl) => {
                self.eval_async_function_decl(async_func_decl)
            }
            Statement::TypeDecl(type_decl) => self.eval_type_decl(type_decl),
            Statement::ErrorTypeDecl(error_type_decl) => self.eval_error_type_decl(error_type_decl),
            Statement::ShareDecl(share_decl) => self.eval_share_decl(share_decl),
            Statement::UseDecl(use_decl) => self.eval_use_decl(use_decl),
        }
    }

    fn eval_error_type_decl(
        &mut self,
        _error_type_decl: crate::ast::ErrorTypeDecl,
    ) -> Result<Value, InterpreterError> {
        // For now, error type declarations don't produce runtime values
        // In a full implementation, we'd store error type information for later use
        Ok(Value::Unit)
    }

    fn eval_let_decl(&mut self, let_decl: LetDecl) -> Result<Value, InterpreterError> {
        let value = if let Some(expr) = let_decl.value {
            self.eval_expr(expr)?
        } else {
            Value::Unit
        };

        // Use pattern matching to bind variables from the pattern
        let mut bindings = HashMap::new();
        if !self.pattern_matches_bind(&let_decl.pattern, &value, &mut bindings)? {
            return Err(InterpreterError::PatternMatchFailed);
        }

        // Bind all variables from the pattern
        for (var_name, var_value) in bindings {
            self.environment.define(var_name, var_value);
        }

        Ok(value)
    }

    fn eval_function_decl(&mut self, func_decl: FunctionDecl) -> Result<Value, InterpreterError> {
        let closure = self.environment.variables.clone();
        let function = Function {
            name: Some(func_decl.name.clone()),
            parameters: func_decl.parameters,
            body: func_decl.body,
            closure,
        };

        let function_value = Value::Function(function);

        // Define the function in the current environment so it can be called recursively
        self.environment
            .define(func_decl.name, function_value.clone());

        Ok(function_value)
    }

    fn eval_expr(&mut self, expr: Expr) -> Result<Value, InterpreterError> {
        match expr {
            Expr::Integer(n) => Ok(Value::Integer(n)),
            Expr::Float(x) => Ok(Value::Float(x)),
            Expr::String(s) => Ok(Value::String(std::sync::Arc::new((*s).clone()))),
            Expr::Boolean(b) => Ok(Value::Boolean(b)),
            Expr::List(items_rc) => {
                let mut values = Vec::new();
                for item in items_rc.iter() {
                    values.push(self.eval_expr(item.clone())?);
                }
                Ok(Value::List(std::sync::Arc::from(values)))
            }
            Expr::Tuple(items_rc) => {
                let mut values = Vec::new();
                for item in items_rc.iter() {
                    values.push(self.eval_expr(item.clone())?);
                }
                Ok(Value::Tuple(std::sync::Arc::new(values)))
            }
            Expr::Identifier(name) => self
                .environment
                .get(&name)
                .ok_or(InterpreterError::UndefinedVariable { name }),
            Expr::Call { callee, arguments } => {
                let callee_value = self.eval_expr(*callee)?;
                
                // Enhanced named argument resolution
                let arg_values = self.resolve_arguments(&callee_value, arguments)?;
                self.call_function(callee_value, arg_values)
            }
            Expr::Lambda {
                parameters, body, ..
            } => {
                // Capture all accessible variables from the environment chain
                let closure = self.collect_all_accessible_variables();
                Ok(Value::Function(Function {
                    name: None,
                    parameters: parameters.clone(),
                    body: *body,
                    closure,
                }))
            }
            Expr::Pipeline { left, right } => {
                let left_value = self.eval_expr(*left)?;
                match *right {
                    Expr::Call { callee, arguments } => {
                        // Enhanced named argument resolution for pipelines
                        let callee_value = self.eval_expr(*callee)?;
                        
                        // Resolve arguments excluding the piped value
                        let pipeline_arguments = arguments;
                        let additional_args = self.resolve_arguments(&callee_value, pipeline_arguments)?;
                        
                        // Prepend the piped value as the first argument
                        let mut final_args = vec![left_value];
                        final_args.extend(additional_args);
                        
                        self.call_function(callee_value, final_args)
                    }
                    Expr::Identifier(name) => {
                        let function_value = self.environment.get(&name).ok_or_else(|| {
                            InterpreterError::UndefinedVariable { name: name.clone() }
                        })?;
                        self.call_function(function_value, vec![left_value])
                    }
                    _ => Err(InterpreterError::RuntimeError {
                        message:
                            "Pipeline right side must be a function call or function identifier"
                                .to_string(),
                    }),
                }
            }
            Expr::Match { value, arms } => {
                let value = self.eval_expr(*value)?;
                self.eval_match(value, arms)
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = self.eval_expr(*condition)?;
                let condition_bool = self.to_boolean(&condition)?;

                if condition_bool {
                    self.eval_expr(*then_branch)
                } else if let Some(else_expr) = else_branch {
                    self.eval_expr(*else_expr)
                } else {
                    Ok(Value::Unit)
                }
            }
            Expr::Block(statements) => {
                let mut result = Value::Unit;
                for statement in statements {
                    result = self.eval_statement(statement)?;
                }
                Ok(result)
            }
            Expr::BinaryOp { left, op, right } => {
                let left = self.eval_expr(*left)?;
                let right = self.eval_expr(*right)?;
                self.eval_binary_op(left, op, right)
            }
            Expr::UnaryOp { op, operand } => {
                let operand = self.eval_expr(*operand)?;
                self.eval_unary_op(op, operand)
            }
            Expr::Range {
                start,
                end,
                inclusive,
            } => {
                let start_val = self.eval_expr(*start)?;
                let end_val = self.eval_expr(*end)?;
                self.eval_range(start_val, end_val, inclusive)
            }
            Expr::StructLiteral(struct_literal) => self.eval_struct_literal(struct_literal),
            Expr::AnonymousObject { fields } => self.eval_anonymous_object(fields),
            Expr::MapLiteral { entries } => self.eval_map_literal(entries),
            Expr::FieldAccess { object, field } => self.eval_field_access(object, field),
            Expr::ResultOk(expr) => {
                let value = self.eval_expr(*expr)?;
                Ok(Value::Ok(Box::new(value)))
            }
            Expr::ResultErr(expr) => {
                let value = self.eval_expr(*expr)?;
                Ok(Value::Err(Box::new(value)))
            }
            Expr::Try(expr) => {
                let value = self.eval_expr(*expr)?;
                match value {
                    Value::Ok(inner) => Ok(*inner),
                    Value::Err(err) => Err(InterpreterError::RuntimeError {
                        message: format!("Tried to unwrap error: {:?}", err),
                    }),
                    _ => Err(InterpreterError::TypeError {
                        message: "Try operator can only be used on Result values".to_string(),
                    }),
                }
            }
            Expr::TryCatch {
                try_block,
                catch_var,
                catch_block,
            } => {
                let try_result = self.eval_expr(*try_block)?;
                match try_result {
                    Value::Ok(inner) => Ok(*inner),
                    Value::Err(err) => {
                        // Create new scope for catch block with error variable
                        let parent = self.environment.clone();
                        self.environment = Environment::with_parent(parent);
                        self.environment.define(catch_var, *err);

                        let result = self.eval_expr(*catch_block);

                        // Restore parent environment
                        if let Some(parent) = self.environment.parent.take() {
                            self.environment = *parent;
                        }

                        result
                    }
                    _ => Err(InterpreterError::TypeError {
                        message: "Try-catch can only be used on Result values".to_string(),
                    }),
                }
            }
            Expr::ForLoop {
                variable,
                iterable,
                body,
            } => self.eval_for_loop(&variable, &iterable, &body),
            Expr::WhileLoop { condition, body } => self.eval_while_loop(&condition, &body),
            Expr::Loop { body } => self.eval_loop(&body),
            Expr::Break => Err(InterpreterError::RuntimeError {
                message: "break".to_string(),
            }),
            Expr::Continue => Err(InterpreterError::RuntimeError {
                message: "continue".to_string(),
            }),
            Expr::Assignment { target, value } => {
                let val = self.eval_expr(*value)?;
                self.environment.set(&target, val.clone()).or_else(|_| {
                    // If variable not defined, define it
                    self.environment.define(target.clone(), val.clone());
                    Ok(())
                })?;
                Ok(val)
            }
            Expr::RawString(s) => {
                Ok(Value::String(std::sync::Arc::new(s.as_str().to_string())))
            }
            Expr::TemplateString { parts } => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        crate::ast::TemplatePart::Literal(s) => result.push_str(&s),
                        crate::ast::TemplatePart::Interpolation(expr) => {
                            let val = self.eval_expr(*expr)?;
                            // For template interpolation, we want raw values without quotes
                            match val {
                                Value::String(s) => result.push_str(&s),
                                Value::Integer(n) => result.push_str(&n.to_string()),
                                Value::Float(x) => result.push_str(&x.to_string()),
                                Value::Boolean(b) => result.push_str(&b.to_string()),
                                other => result.push_str(&format!("{}", other)),
                            }
                        }
                    }
                }
                Ok(Value::String(result.into()))
            }
            Expr::BitwiseOp { left, op, right } => {
                let left_val = self.eval_expr(*left)?;
                let right_val = self.eval_expr(*right)?;
                
                match (left_val, right_val) {
                    (Value::Integer(l), Value::Integer(r)) => {
                        let result = match op {
                            crate::ast::BitwiseOp::And => l & r,
                            crate::ast::BitwiseOp::Or => l | r,
                            crate::ast::BitwiseOp::Xor => l ^ r,
                            crate::ast::BitwiseOp::Shl => l << r,
                            crate::ast::BitwiseOp::Shr => l >> r,
                        };
                        Ok(Value::Integer(result))
                    }
                    _ => Err(InterpreterError::TypeError {
                        message: "integer operands required for bitwise operation".to_string(),
                    }),
                }
            }
            Expr::Spread(expr) => {
                // For now, just evaluate the inner expression
                // Spread semantics would be handled at the call site
                self.eval_expr(*expr)
            }
            Expr::Rest(expr) => {
                // For now, just evaluate the inner expression
                // Rest semantics would be handled in pattern matching
                self.eval_expr(*expr)
            }
            Expr::Index { object, index } => {
                let object_value = self.eval_expr(*object)?;
                let index_value = self.eval_expr(*index)?;

                match (object_value, index_value) {
                    (Value::List(list), Value::Integer(idx)) => {
                        let index = if idx < 0 {
                            // Negative indexing from end
                            (list.len() as i64 + idx) as usize
                        } else {
                            idx as usize
                        };

                        if index < list.len() {
                            Ok(list[index].clone())
                        } else {
                            Err(InterpreterError::RuntimeError {
                                message: format!(
                                    "Index {} out of bounds for list of length {}",
                                    idx,
                                    list.len()
                                ),
                            })
                        }
                    }
                    (Value::Tuple(tuple), Value::Integer(idx)) => {
                        let index = if idx < 0 {
                            // Negative indexing from end
                            (tuple.len() as i64 + idx) as usize
                        } else {
                            idx as usize
                        };

                        if index < tuple.len() {
                            Ok(tuple[index].clone())
                        } else {
                            Err(InterpreterError::RuntimeError {
                                message: format!(
                                    "Index {} out of bounds for tuple of length {}",
                                    idx,
                                    tuple.len()
                                ),
                            })
                        }
                    }
                    (Value::String(string), Value::Integer(idx)) => {
                        let chars: Vec<char> = string.chars().collect();
                        let index = if idx < 0 {
                            // Negative indexing from end
                            (chars.len() as i64 + idx) as usize
                        } else {
                            idx as usize
                        };

                        if index < chars.len() {
                            Ok(Value::String(chars[index].to_string().into()))
                        } else {
                            Err(InterpreterError::RuntimeError {
                                message: format!(
                                    "Index {} out of bounds for string of length {}",
                                    idx,
                                    chars.len()
                                ),
                            })
                        }
                    }
                    (_, Value::Integer(_)) => Err(InterpreterError::TypeError {
                        message: "Only lists, tuples, and strings can be indexed".to_string(),
                    }),
                    (_, _) => Err(InterpreterError::TypeError {
                        message: "Index must be an integer".to_string(),
                    }),
                }
            }
            // Async expressions - enhanced implementations
            Expr::Async {
                parameters,
                body,
                return_type: _return_type,
            } => {
                // Create async function with enhanced async capabilities
                let closure = self.environment.variables.clone();
                let function = Function {
                    name: None,
                    parameters,
                    body: *body,
                    closure,
                };
                
                // Return a function that when called returns a promise
                Ok(Value::Function(function))
            }
            Expr::Await { expression } => {
                // Enhanced await implementation with proper promise resolution
                let value = self.eval_expr(*expression)?;
                match value {
                    Value::Promise {
                        state: crate::ast::PromiseState::Resolved,
                        value: Some(resolved_value),
                        ..
                    } => Ok(*resolved_value),
                    Value::Promise {
                        state: crate::ast::PromiseState::Rejected,
                        error: Some(error_value),
                        ..
                    } => Err(InterpreterError::RuntimeError {
                        message: format!("Promise rejected: {:?}", error_value),
                    }),
                    Value::Promise {
                        state: crate::ast::PromiseState::Pending,
                        ..
                    } => {
                        // In a full async implementation, this would wait for resolution
                        // For now, we'll return an error but in real async this would suspend
                        Err(InterpreterError::RuntimeError {
                            message: "Cannot await pending promise (async scheduling not implemented)".to_string(),
                        })
                    }
                    // If not a promise, treat as already resolved value
                    _ => Ok(value),
                }
            }
            Expr::Promise {
                promise_type,
                value,
                delay,
            } => {
                let evaluated_value = self.eval_expr(*value)?;
                match promise_type {
                    PromiseType::Resolve => Ok(self.async_runtime.promise_resolve(evaluated_value)),
                    PromiseType::Reject => Ok(self.async_runtime.promise_reject(evaluated_value)),
                    PromiseType::Delay => {
                        // Enhanced delay implementation
                        if let Some(delay_expr) = delay {
                            let delay_value = self.eval_expr(*delay_expr)?;
                            match delay_value {
                                Value::Integer(ms) if ms >= 0 => {
                                    let (_, delayed_promise) = self.async_runtime.create_delayed_promise(ms as u64, evaluated_value);
                                    Ok(delayed_promise)
                                }
                                Value::Integer(_) => {
                                    Err(InterpreterError::RuntimeError {
                                        message: "Delay must be a non-negative integer".to_string(),
                                    })
                                }
                                _ => {
                                    Err(InterpreterError::TypeError {
                                        message: "Delay must be an integer representing milliseconds".to_string(),
                                    })
                                }
                            }
                        } else {
                            // Default delay of 0ms (immediate resolution)
                            Ok(self.async_runtime.promise_resolve(evaluated_value))
                        }
                    }
                }
            }
            Expr::All(expressions) => {
                // Enhanced Promise.all implementation
                let mut results = Vec::new();
                let mut all_resolved = true;
                let mut any_rejected = false;
                let mut rejection_error = None;
                
                for expr in expressions {
                    let value = self.eval_expr(expr)?;
                    match value {
                        Value::Promise {
                            state: crate::ast::PromiseState::Resolved,
                            value: Some(resolved_value),
                            ..
                        } => {
                            results.push(*resolved_value);
                        }
                        Value::Promise {
                            state: crate::ast::PromiseState::Rejected,
                            error: Some(error_value),
                            ..
                        } => {
                            any_rejected = true;
                            rejection_error = Some(*error_value);
                            break;
                        }
                        Value::Promise {
                            state: crate::ast::PromiseState::Pending,
                            ..
                        } => {
                            all_resolved = false;
                            // In full async implementation, would wait for all promises
                            results.push(Value::Unit); // Placeholder
                        }
                        // Non-promise values are treated as already resolved
                        _ => {
                            results.push(value);
                        }
                    }
                }
                
                if any_rejected {
                    Ok(self.async_runtime.promise_reject(rejection_error.unwrap_or(Value::Unit)))
                } else if all_resolved {
                    Ok(self.async_runtime.promise_resolve(Value::List(std::sync::Arc::from(results))))
                } else {
                    // Some promises still pending - in full implementation would return pending promise
                    Ok(Value::Promise {
                        state: crate::ast::PromiseState::Pending,
                        value: None,
                        error: None,
                    })
                }
            }
            Expr::Race(expressions) => {
                // Enhanced Promise.race implementation
                if expressions.is_empty() {
                    return Ok(Value::Promise {
                        state: crate::ast::PromiseState::Pending,
                        value: None,
                        error: None,
                    });
                }
                
                // Evaluate all expressions and return the first resolved/rejected promise
                for expr in expressions {
                    let value = self.eval_expr(expr)?;
                    match value {
                        Value::Promise {
                            state: crate::ast::PromiseState::Resolved,
                            ..
                        } | Value::Promise {
                            state: crate::ast::PromiseState::Rejected,
                            ..
                        } => {
                            // Return first resolved or rejected promise
                            return Ok(value);
                        }
                        Value::Promise {
                            state: crate::ast::PromiseState::Pending,
                            ..
                        } => {
                            // Continue to next promise
                            continue;
                        }
                        // Non-promise values are treated as already resolved
                        _ => {
                            return Ok(self.async_runtime.promise_resolve(value));
                        }
                    }
                }
                
                // All promises are pending
                Ok(Value::Promise {
                    state: crate::ast::PromiseState::Pending,
                    value: None,
                    error: None,
                })
            }
            Expr::Spawn(expression) => {
                // Enhanced spawn implementation - evaluate expression asynchronously
                // For now, just evaluate the expression and wrap in resolved promise
                // In full implementation, would execute in separate task
                let result = self.eval_expr(*expression)?;
                Ok(self.async_runtime.promise_resolve(result))
            }
        }
    }

    fn eval_async_function_decl(
        &mut self,
        async_func_decl: crate::ast::AsyncFunctionDecl,
    ) -> Result<Value, InterpreterError> {
        // For now, treat async functions like regular functions
        // In full implementation, would mark as async
        let closure = self.environment.variables.clone();
        let function = Function {
            name: Some(async_func_decl.name.clone()),
            parameters: async_func_decl.parameters,
            body: async_func_decl.body,
            closure,
        };

        let function_value = Value::Function(function);

        // Define the function in the current environment so it can be called recursively
        self.environment
            .define(async_func_decl.name, function_value.clone());

        Ok(function_value)
    }

    pub fn call_function(
        &mut self,
        callee: Value,
        arguments: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        match callee {
            Value::Function(func) => {
                // Count required parameters (those without default values)
                let required_params = func.parameters.iter()
                    .filter(|p| p.default_value.is_none())
                    .count();
                
                // Check if we have enough arguments for required parameters
                if arguments.len() < required_params {
                    return Err(InterpreterError::ArityMismatch {
                        expected: required_params,
                        got: arguments.len(),
                    });
                }
                
                // Check if we have too many arguments
                if arguments.len() > func.parameters.len() {
                    return Err(InterpreterError::ArityMismatch {
                        expected: func.parameters.len(),
                        got: arguments.len(),
                    });
                }

                // Create new environment with current environment as parent
                let mut new_env = Environment::with_parent(self.environment.clone());
                
                // Add closure variables to the new environment (only if not empty)
                if !func.closure.is_empty() {
                    for (k, v) in func.closure.iter() {
                        new_env.define(k.clone(), v.clone());
                    }
                }

                // If this is a named function, add it to its own scope for recursion
                if let Some(name) = &func.name {
                    new_env.define(name.clone(), Value::Function(func.clone()));
                }

                // Add parameters to environment
                for (i, param) in func.parameters.iter().enumerate() {
                    let value = if i < arguments.len() {
                        arguments[i].clone()
                    } else if param.default_value.is_some() {
                        // For simplicity, use Unit for default values for now
                        Value::Unit
                    } else {
                        return Err(InterpreterError::RuntimeError {
                            message: format!("Missing argument for parameter {}", param.name),
                        });
                    };
                    
                    new_env.define(param.name.clone(), value);
                }

                // Save current environment and switch to new one
                let saved_env = std::mem::replace(&mut self.environment, new_env);
                
                // Evaluate function body in the new environment
                let result = self.eval_expr(func.body);
                
                // Restore original environment
                self.environment = saved_env;
                
                result
            }
            Value::Builtin(builtin) => {
                let name = builtin.name.clone();
                let builtin_functions = self.builtin_functions.clone();
                BuiltinFunctions::call(&builtin_functions, &name, arguments, self)
            }
            _ => Err(InterpreterError::TypeError {
                message: "Cannot call non-function value".to_string(),
            }),
        }
    }

    /// Create a thread-safe clone for parallel operations
    pub fn thread_safe_clone(&self) -> Self {
        Self {
            environment: self.environment.clone(),
            builtin_functions: self.builtin_functions.clone(),
            type_checker: self.type_checker.clone(),
            async_runtime: AsyncRuntime::new(),
            lazy_config: self.lazy_config.clone(),
            safepoint_manager: self.safepoint_manager.clone(),
            module_debug_config: self.module_debug_config.clone(),
            
            // Enhanced module system
            module_cache: self.module_cache.clone(),
            dependency_tracker: self.dependency_tracker.clone(),
            current_module_path: self.current_module_path.clone(), // For tracking current module during loading
        }
    }

    /// Call function in a thread-safe manner (immutable)
    pub fn call_function_safe(
        &self,
        function: Value,
        args: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        // Create a local copy of interpreter state for this thread
        let mut local_interpreter = self.thread_safe_clone();
        local_interpreter.call_function(function, args)
    }

    fn eval_match(&mut self, value: Value, arms: Vec<MatchArm>) -> Result<Value, InterpreterError> {
        for arm in arms {
            let mut bindings = HashMap::new();
            if self.pattern_matches_bind(&arm.pattern, &value, &mut bindings)? {
                // Pattern matched, now check guard clause if present
                let guard_passed = if let Some(guard_expr) = &arm.guard {
                    // Create scope with pattern bindings for guard evaluation
                    let parent = self.environment.clone();
                    self.environment = Environment::with_parent(parent);
                    for (k, v) in &bindings {
                        self.environment.define(k.clone(), v.clone());
                    }
                    
                    let guard_result = self.eval_expr(*guard_expr.clone());
                    
                    // Restore parent environment
                    if let Some(parent) = self.environment.parent.take() {
                        self.environment = *parent;
                    }
                    
                    match guard_result {
                        Ok(guard_value) => self.to_boolean(&guard_value)?,
                        Err(_) => false, // Guard evaluation failed, treat as false
                    }
                } else {
                    true // No guard clause, pattern match is sufficient
                };
                
                if guard_passed {
                    // Execute the match arm expression with pattern bindings
                    let parent = self.environment.clone();
                    self.environment = Environment::with_parent(parent);
                    for (k, v) in bindings {
                        self.environment.define(k, v);
                    }
                    let result = self.eval_expr(arm.expression);
                    if let Some(parent) = self.environment.parent.take() {
                        self.environment = *parent;
                    }
                    return result;
                }
                // Pattern matched but guard failed, continue to next arm
            }
        }
        Err(InterpreterError::PatternMatchFailed)
    }

    fn pattern_matches_bind(
        &self,
        pattern: &Pattern,
        value: &Value,
        bindings: &mut HashMap<String, Value>,
    ) -> Result<bool, InterpreterError> {
        match (pattern, value) {
            (Pattern::Literal(lit), val) => Ok(lit == val),
            (Pattern::Identifier(name), val) => {
                bindings.insert(name.clone(), val.clone());
                Ok(true)
            }
            (Pattern::Wildcard, _) => Ok(true),
            (Pattern::List { patterns, rest }, Value::List(values)) => {
                if let Some(rest_name) = rest {
                    // Rest pattern: [a, b, ...rest]
                    if patterns.len() > values.len() {
                        return Ok(false); // Not enough values for required patterns
                    }
                    
                    // Match the explicit patterns
                    for (i, pattern) in patterns.iter().enumerate() {
                        if !self.pattern_matches_bind(pattern, &values[i], bindings)? {
                            return Ok(false);
                        }
                    }
                    
                    // Bind the rest of the values to the rest variable
                    let rest_values: Vec<Value> = values[patterns.len()..].to_vec();
                    bindings.insert(rest_name.clone(), Value::List(rest_values.into()));
                    Ok(true)
                } else {
                    // No rest pattern: exact length match required
                    if patterns.len() != values.len() {
                        return Ok(false);
                    }
                    for (p, v) in patterns.iter().zip(values.iter()) {
                        if !self.pattern_matches_bind(p, v, bindings)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
            }
            (Pattern::Tuple(patterns), Value::Tuple(values)) => {
                if patterns.len() != values.len() {
                    return Ok(false);
                }
                for (p, v) in patterns.iter().zip(values.iter()) {
                    if !self.pattern_matches_bind(p, v, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            // Result pattern matching
            (Pattern::Ok(inner_pattern), Value::Ok(inner_value)) => {
                self.pattern_matches_bind(inner_pattern, inner_value, bindings)
            }
            (Pattern::Err(inner_pattern), Value::Err(inner_value)) => {
                self.pattern_matches_bind(inner_pattern, inner_value, bindings)
            }
            (Pattern::Ok(_), _) => Ok(false), // Ok pattern doesn't match non-Ok values
            (Pattern::Err(_), _) => Ok(false), // Err pattern doesn't match non-Err values
            // Enum variant patterns - now with proper enum value handling
            (
                Pattern::EnumVariant {
                    variant_name,
                    patterns,
                },
                Value::Enum {
                    variant_name: val_variant,
                    variant_data,
                    ..
                },
            ) => {
                // Check if variant names match
                if variant_name != val_variant {
                    return Ok(false);
                }

                // Match based on the variant data type
                match variant_data {
                    EnumVariantData::Unit => {
                        // Unit variants should have no patterns
                        Ok(patterns.is_empty())
                    }
                    EnumVariantData::Tuple(values) => {
                        // Tuple variants should match against the contained values
                        if patterns.len() != values.len() {
                            return Ok(false);
                        }
                        for (p, v) in patterns.iter().zip(values.iter()) {
                            if !self.pattern_matches_bind(p, v, bindings)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                    EnumVariantData::Struct(fields) => {
                        // For struct variants, patterns should match field values
                        // This is a simplified implementation - real struct matching would be more complex
                        if patterns.len() != fields.len() {
                            return Ok(false);
                        }
                        // For now, just match values in order
                        let field_values: Vec<_> = fields.values().collect();
                        for (p, v) in patterns.iter().zip(field_values.iter()) {
                            if !self.pattern_matches_bind(p, *v, bindings)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                }
            }
            // Fallback for old tuple-based enum handling (for compatibility)
            (
                Pattern::EnumVariant {
                    variant_name: _,
                    patterns,
                },
                Value::Tuple(values),
            ) => {
                // Keep backward compatibility with tuple-based enum handling
                if patterns.len() != values.len() {
                    return Ok(false);
                }
                for (p, v) in patterns.iter().zip(values.iter()) {
                    if !self.pattern_matches_bind(p, v, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            // Struct patterns
            (
                Pattern::Struct {
                    type_name: _,
                    field_patterns,
                },
                Value::Struct { fields, .. },
            ) => {
                for (field_name, pattern) in field_patterns {
                    if let Some(field_value) = fields.get(field_name) {
                        if !self.pattern_matches_bind(pattern, field_value, bindings)? {
                            return Ok(false);
                        }
                    } else {
                        return Ok(false); // Field not found
                    }
                }
                Ok(true)
            }
            // Anonymous struct patterns
            (
                Pattern::AnonymousStruct { field_patterns },
                Value::Struct { fields, .. },
            ) => {
                for (field_name, pattern) in field_patterns {
                    if let Some(field_value) = fields.get(field_name) {
                        if !self.pattern_matches_bind(pattern, field_value, bindings)? {
                            return Ok(false);
                        }
                    } else {
                        return Ok(false); // Field not found
                    }
                }
                Ok(true)
            }
            // Range patterns for integers
            (Pattern::Range { start, end, inclusive }, Value::Integer(n)) => {
                let start_val = match start.as_ref() {
                    Pattern::Literal(Value::Integer(s)) => *s,
                    _ => return Ok(false),
                };
                let end_val = match end.as_ref() {
                    Pattern::Literal(Value::Integer(e)) => *e,
                    _ => return Ok(false),
                };

                if *inclusive {
                    Ok(*n >= start_val && *n <= end_val)
                } else {
                    Ok(*n >= start_val && *n < end_val)
                }
            }
            // Range patterns for single-character strings
            (Pattern::Range { start, end, inclusive }, Value::String(s)) => {
                let start_char = match start.as_ref() {
                    Pattern::Literal(Value::String(ref sv)) => sv.chars().next().unwrap_or('\0'),
                    _ => return Ok(false),
                };
                let end_char = match end.as_ref() {
                    Pattern::Literal(Value::String(ref ev)) => ev.chars().next().unwrap_or('\0'),
                    _ => return Ok(false),
                };
                let ch = s.chars().next().unwrap_or('\0');

                if *inclusive {
                    Ok(ch >= start_char && ch <= end_char)
                } else {
                    Ok(ch >= start_char && ch < end_char)
                }
            }
            // Or patterns
            (Pattern::Or { alternatives }, val) => {
                for alt_pattern in alternatives {
                    let mut alt_bindings = HashMap::new();
                    if self.pattern_matches_bind(alt_pattern, val, &mut alt_bindings)? {
                        // Merge bindings from the matching alternative
                        bindings.extend(alt_bindings);
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            // Guarded patterns (guards are handled at a higher level)
            (Pattern::Guarded { pattern, .. }, val) => {
                // For guarded patterns, just check if the inner pattern matches
                // The guard will be evaluated separately in eval_match
                self.pattern_matches_bind(pattern, val, bindings)
            }
            // Rest patterns (standalone rest patterns should not appear in normal matching)
            (Pattern::Rest(_), _) => {
                // This should not happen in well-formed patterns as rest patterns 
                // are only valid inside list patterns
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn eval_binary_op(
        &self,
        left: Value,
        op: BinaryOp,
        right: Value,
    ) -> Result<Value, InterpreterError> {
        match (left, op, right) {
            (Value::Integer(a), BinaryOp::Add, Value::Integer(b)) => Ok(Value::Integer(a + b)),
            (Value::Float(a), BinaryOp::Add, Value::Float(b)) => Ok(Value::Float(a + b)),
            (Value::Integer(a), BinaryOp::Add, Value::Float(b)) => Ok(Value::Float(a as f64 + b)),
            (Value::Float(a), BinaryOp::Add, Value::Integer(b)) => Ok(Value::Float(a + b as f64)),
            (Value::Integer(a), BinaryOp::Subtract, Value::Integer(b)) => Ok(Value::Integer(a - b)),
            (Value::Float(a), BinaryOp::Subtract, Value::Float(b)) => Ok(Value::Float(a - b)),
            (Value::Integer(a), BinaryOp::Subtract, Value::Float(b)) => {
                Ok(Value::Float(a as f64 - b))
            }
            (Value::Float(a), BinaryOp::Subtract, Value::Integer(b)) => {
                Ok(Value::Float(a - b as f64))
            }
            (Value::Integer(a), BinaryOp::Multiply, Value::Integer(b)) => Ok(Value::Integer(a * b)),
            (Value::Float(a), BinaryOp::Multiply, Value::Float(b)) => Ok(Value::Float(a * b)),
            (Value::Integer(a), BinaryOp::Multiply, Value::Float(b)) => {
                Ok(Value::Float(a as f64 * b))
            }
            (Value::Float(a), BinaryOp::Multiply, Value::Integer(b)) => {
                Ok(Value::Float(a * b as f64))
            }
            (Value::Integer(a), BinaryOp::Divide, Value::Integer(b)) => {
                if b == 0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    Ok(Value::Integer(a / b))
                }
            }
            (Value::Float(a), BinaryOp::Divide, Value::Float(b)) => {
                if b == 0.0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a / b))
                }
            }
            (Value::Integer(a), BinaryOp::Divide, Value::Float(b)) => {
                if b == 0.0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a as f64 / b))
                }
            }
            (Value::Float(a), BinaryOp::Divide, Value::Integer(b)) => {
                if b == 0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a / b as f64))
                }
            }
            (Value::Integer(a), BinaryOp::Modulo, Value::Integer(b)) => {
                if b == 0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Modulo by zero".to_string(),
                    })
                } else {
                    Ok(Value::Integer(a % b))
                }
            }
            (Value::Float(a), BinaryOp::Modulo, Value::Float(b)) => {
                if b == 0.0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Modulo by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a % b))
                }
            }
            (Value::Integer(a), BinaryOp::Modulo, Value::Float(b)) => {
                if b == 0.0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Modulo by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a as f64 % b))
                }
            }
            (Value::Float(a), BinaryOp::Modulo, Value::Integer(b)) => {
                if b == 0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Modulo by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a % b as f64))
                }
            }
            (Value::Integer(a), BinaryOp::Equal, Value::Integer(b)) => Ok(Value::Boolean(a == b)),
            (Value::Float(a), BinaryOp::Equal, Value::Float(b)) => Ok(Value::Boolean(a == b)),
            (Value::String(a), BinaryOp::Equal, Value::String(b)) => Ok(Value::Boolean(*a == *b)),
            (Value::Boolean(a), BinaryOp::Equal, Value::Boolean(b)) => Ok(Value::Boolean(a == b)),
            (Value::Integer(a), BinaryOp::NotEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a != b))
            }
            (Value::Float(a), BinaryOp::NotEqual, Value::Float(b)) => Ok(Value::Boolean(a != b)),
            (Value::String(a), BinaryOp::NotEqual, Value::String(b)) => {
                Ok(Value::Boolean(*a != *b))
            }
            (Value::Boolean(a), BinaryOp::NotEqual, Value::Boolean(b)) => {
                Ok(Value::Boolean(a != b))
            }
            (Value::Integer(a), BinaryOp::Equal, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) == b))
            }
            (Value::Float(a), BinaryOp::Equal, Value::Integer(b)) => {
                Ok(Value::Boolean(a == b as f64))
            }
            (Value::Integer(a), BinaryOp::NotEqual, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) != b))
            }
            (Value::Float(a), BinaryOp::NotEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a != b as f64))
            }
            (Value::Integer(a), BinaryOp::LessThan, Value::Integer(b)) => Ok(Value::Boolean(a < b)),
            (Value::Float(a), BinaryOp::LessThan, Value::Float(b)) => Ok(Value::Boolean(a < b)),
            (Value::Integer(a), BinaryOp::LessThan, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) < b))
            }
            (Value::Float(a), BinaryOp::LessThan, Value::Integer(b)) => {
                Ok(Value::Boolean(a < b as f64))
            }
            (Value::Integer(a), BinaryOp::LessThanEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a <= b))
            }
            (Value::Float(a), BinaryOp::LessThanEqual, Value::Float(b)) => {
                Ok(Value::Boolean(a <= b))
            }
            (Value::Integer(a), BinaryOp::LessThanEqual, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) <= b))
            }
            (Value::Float(a), BinaryOp::LessThanEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a <= b as f64))
            }
            (Value::Integer(a), BinaryOp::GreaterThan, Value::Integer(b)) => {
                Ok(Value::Boolean(a > b))
            }
            (Value::Float(a), BinaryOp::GreaterThan, Value::Float(b)) => Ok(Value::Boolean(a > b)),
            (Value::Integer(a), BinaryOp::GreaterThan, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) > b))
            }
            (Value::Float(a), BinaryOp::GreaterThan, Value::Integer(b)) => {
                Ok(Value::Boolean(a > b as f64))
            }
            (Value::Integer(a), BinaryOp::GreaterThanEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a >= b))
            }
            (Value::Float(a), BinaryOp::GreaterThanEqual, Value::Float(b)) => {
                Ok(Value::Boolean(a >= b))
            }
            (Value::Integer(a), BinaryOp::GreaterThanEqual, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) >= b))
            }
            (Value::Float(a), BinaryOp::GreaterThanEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a >= b as f64))
            }
            (Value::Boolean(a), BinaryOp::And, Value::Boolean(b)) => Ok(Value::Boolean(a && b)),
            (Value::Boolean(a), BinaryOp::Or, Value::Boolean(b)) => Ok(Value::Boolean(a || b)),
            (Value::String(a), BinaryOp::Add, Value::String(b)) => {
                let mut s = (*a).clone();
                s.push_str(&b);
                Ok(Value::String(std::sync::Arc::new(s)))
            }
            (Value::String(a), BinaryOp::Add, Value::Integer(b)) => {
                let mut s = (*a).clone();
                s.push_str(&b.to_string());
                Ok(Value::String(std::sync::Arc::new(s)))
            }
            (Value::Integer(a), BinaryOp::Add, Value::String(b)) => {
                let mut s = a.to_string();
                s.push_str(&b);
                Ok(Value::String(std::sync::Arc::new(s)))
            }
            (Value::String(a), BinaryOp::Add, Value::Float(b)) => {
                let mut s = (*a).clone();
                s.push_str(&b.to_string());
                Ok(Value::String(std::sync::Arc::new(s)))
            }
            (Value::Float(a), BinaryOp::Add, Value::String(b)) => {
                let mut s = a.to_string();
                s.push_str(&b);
                Ok(Value::String(std::sync::Arc::new(s)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "Invalid binary operation".to_string(),
            }),
        }
    }

    fn eval_unary_op(&self, op: UnaryOp, operand: Value) -> Result<Value, InterpreterError> {
        match (op, operand) {
            (UnaryOp::Negate, Value::Integer(n)) => Ok(Value::Integer(-n)),
            (UnaryOp::Not, Value::Boolean(b)) => Ok(Value::Boolean(!b)),
            _ => Err(InterpreterError::TypeError {
                message: "Invalid unary operation".to_string(),
            }),
        }
    }

    fn to_boolean(&self, value: &Value) -> Result<bool, InterpreterError> {
        match value {
            Value::Boolean(b) => Ok(*b),
            Value::Integer(n) => Ok(*n != 0),
            Value::Float(x) => Ok(*x != 0.0),
            Value::String(s) => Ok(!s.is_empty()),
            Value::List(items) => Ok(!items.is_empty()),
            Value::Tuple(items) => Ok(!items.is_empty()),
            Value::Range { start, end, inclusive } => {
                if *inclusive {
                    Ok(start <= end)
                } else {
                    Ok(start < end)
                }
            }
            Value::Unit => Ok(false),
            _ => Ok(true),
        }
    }

    fn eval_range(
        &self,
        start: Value,
        end: Value,
        inclusive: bool,
    ) -> Result<Value, InterpreterError> {
        let start_int = match start {
            Value::Integer(n) => n,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "Range start must be an integer".to_string(),
                })
            }
        };

        let end_int = match end {
            Value::Integer(n) => n,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "Range end must be an integer".to_string(),
                })
            }
        };

        // Return a proper Range value instead of expanding to a list
        Ok(Value::Range {
            start: start_int,
            end: end_int,
            inclusive,
        })
    }

    fn eval_type_decl(
        &mut self,
        _type_decl: crate::ast::TypeDecl,
    ) -> Result<Value, InterpreterError> {
        // For now, type declarations don't produce runtime values
        // In a full implementation, we'd store type information for later use
        Ok(Value::Unit)
    }

    fn eval_struct_literal(
        &mut self,
        struct_literal: crate::ast::StructLiteral,
    ) -> Result<Value, InterpreterError> {
        let mut fields = std::collections::HashMap::new();

        for field_value in struct_literal.fields {
            let value = self.eval_expr(field_value.value)?;
            fields.insert(field_value.name, value);
        }

        Ok(Value::Struct {
            type_name: struct_literal.type_name,
            fields,
        })
    }

    fn eval_anonymous_object(
        &mut self,
        field_values: Vec<crate::ast::FieldValue>,
    ) -> Result<Value, InterpreterError> {
        let mut fields = std::collections::HashMap::new();

        for field_value in field_values {
            let value = self.eval_expr(field_value.value)?;
            fields.insert(field_value.name, value);
        }

        // Use a generic type name for anonymous objects
        Ok(Value::Struct {
            type_name: "Object".to_string(),
            fields,
        })
    }

    fn eval_map_literal(
        &mut self,
        entries: Vec<crate::ast::MapEntry>,
    ) -> Result<Value, InterpreterError> {
        let mut map = std::collections::HashMap::new();

        for entry in entries {
            let key = self.eval_expr(entry.key)?;
            let value = self.eval_expr(entry.value)?;

            // Convert key to string (maps in Olang use string keys)
            let key_str = match key {
                Value::String(s) => s.as_ref().clone(),
                Value::Integer(i) => i.to_string(),
                Value::Float(f) => f.to_string(),
                Value::Boolean(b) => b.to_string(),
                _ => {
                    return Err(InterpreterError::TypeError {
                        message: "Map keys must be strings, integers, floats, or booleans"
                            .to_string(),
                    })
                }
            };

            map.insert(key_str, value);
        }

        Ok(Value::Map(std::sync::Arc::new(map)))
    }

    fn eval_field_access(
        &mut self,
        object: Box<crate::ast::Expr>,
        field: String,
    ) -> Result<Value, InterpreterError> {
        let object_value = self.eval_expr(*object)?;

        match object_value {
            Value::Struct { fields, type_name } => {
                if type_name == "Module" {
                    // Handle module function access (e.g., fs.read_file)
                    fields
                        .get(&field)
                        .cloned()
                        .ok_or_else(|| InterpreterError::TypeError {
                            message: format!("Function '{}' not found in module", field),
                        })
                } else {
                    // Handle regular struct field access
                    fields
                        .get(&field)
                        .cloned()
                        .ok_or_else(|| InterpreterError::TypeError {
                            message: format!("Field '{}' not found", field),
                        })
                }
            }
            _ => Err(InterpreterError::TypeError {
                message: format!("Cannot access field '{}' on non-struct value", field),
            }),
        }
    }

    /// Get a reference to the current environment for REPL inspection
    pub fn get_environment(&self) -> &Environment {
        &self.environment
    }

    /// Get all user-defined variables (excluding built-ins)
    pub fn get_user_variables(&self) -> HashMap<String, &Value> {
        let mut user_vars = HashMap::new();
        let builtin_names: std::collections::HashSet<String> = self
            .builtin_functions
            .get_functions()
            .keys()
            .cloned()
            .collect();

        for (name, value) in &self.environment.variables {
            if !builtin_names.contains(name) {
                user_vars.insert(name.clone(), value);
            }
        }
        user_vars
    }

    /// Get all built-in functions
    pub fn get_builtin_functions(&self) -> &HashMap<String, BuiltinFunction> {
        self.builtin_functions.get_functions()
    }

    /// Clear user-defined variables (keep built-ins and stdlib modules)
    pub fn clear_user_environment(&mut self) {
        let builtin_names: std::collections::HashSet<String> = self
            .builtin_functions
            .get_functions()
            .keys()
            .cloned()
            .collect();

        // Identify stdlib modules before the retain operation
        let stdlib_names: std::collections::HashSet<String> = self
            .environment
            .variables
            .iter()
            .filter_map(|(name, value)| {
                if Self::is_stdlib_module_static(name, value) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();

        self.environment.variables.retain(|name, _| {
            // Keep builtin functions
            builtin_names.contains(name) ||
            // Keep stdlib modules
            stdlib_names.contains(name)
        });
    }

    /// Check if a variable is a standard library module (static version)
    fn is_stdlib_module_static(name: &str, value: &Value) -> bool {
        // Check if it's a known stdlib module name with Module type
        matches!(
            name,
            "dates"
                | "math"
                | "http"
                | "fs"
                | "random"
                | "json"
                | "csv"
                | "base64"
                | "os"
                | "crypto"
        ) && matches!(value, Value::Struct { type_name, .. } if type_name == "Module")
    }

    /// Define a variable in the current environment (for REPL use)
    pub fn define_variable(&mut self, name: String, value: Value) {
        self.environment.define(name, value);
    }

    /// Get the current lazy evaluation configuration
    pub fn get_lazy_config(&self) -> &LazyConfig {
        &self.lazy_config
    }

    /// Update the lazy evaluation configuration
    pub fn set_lazy_config(&mut self, config: LazyConfig) {
        self.lazy_config = config;
    }

    /// Check if memory pressure detection suggests forcing lazy values
    pub fn should_force_evaluation(&self) -> bool {
        check_memory_pressure(self.lazy_config.memory_threshold_mb)
    }

    /// Perform safepoint poll for GC coordination
    /// This should be called periodically during evaluation
    pub fn safepoint_poll(&self) -> Result<(), InterpreterError> {
        self.safepoint_manager
            .safepoint_poll()
            .map_err(|e| InterpreterError::RuntimeError {
                message: format!("Safepoint coordination failed: {}", e),
            })
    }

    /// Register this thread with the safepoint manager
    pub fn register_thread(&self) {
        self.safepoint_manager.register_thread();
    }

    /// Unregister this thread from the safepoint manager
    pub fn unregister_thread(&self) {
        self.safepoint_manager.unregister_thread();
    }

    /// Get the safepoint manager for external coordination
    pub fn get_safepoint_manager(&self) -> Arc<SafepointManager> {
        Arc::clone(&self.safepoint_manager)
    }

    /// Collect all accessible variables from the current environment and its parent chain
    fn collect_all_accessible_variables(&self) -> HashMap<String, Value> {
        let mut all_variables = HashMap::new();
        let mut current_env = &self.environment;

        // Traverse the environment chain from parent to current
        // This ensures that current environment variables override parent ones
        let mut env_chain = Vec::new();
        while let Some(env) = current_env.parent.as_ref() {
            env_chain.push(current_env);
            current_env = env;
        }
        env_chain.push(current_env); // Add the root environment

        // Add variables from root to current (parents first, current last)
        for env in env_chain.iter().rev() {
            for (name, value) in &env.variables {
                all_variables.insert(name.clone(), value.clone());
            }
        }

        all_variables
    }

    fn eval_for_loop(
        &mut self,
        variable: &str,
        iterable: &Expr,
        body: &Expr,
    ) -> Result<Value, InterpreterError> {
        let iterable_value = self.eval_expr(iterable.clone())?;

        match iterable_value {
            Value::List(items) => {
                let mut last_value = Value::Unit;
                let parent_env = std::mem::replace(&mut self.environment, Environment::new());
                self.environment.parent = Some(Box::new(parent_env));

                for item in items.iter() {
                    // Safepoint poll for GC coordination during iteration
                    self.safepoint_poll()?;

                    self.environment.define(variable.to_string(), item.clone());
                    last_value = self.eval_expr(body.clone())?;
                }

                // Restore parent environment
                if let Some(parent) = self.environment.parent.take() {
                    self.environment = *parent;
                }

                Ok(last_value)
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                let mut last_value = Value::Unit;
                let parent_env = std::mem::replace(&mut self.environment, Environment::new());
                self.environment.parent = Some(Box::new(parent_env));

                let range_end = if inclusive { end + 1 } else { end };
                for i in start..range_end {
                    // Safepoint poll for GC coordination during iteration
                    self.safepoint_poll()?;

                    self.environment
                        .define(variable.to_string(), Value::Integer(i));
                    last_value = self.eval_expr(body.clone())?;
                }

                // Restore parent environment
                if let Some(parent) = self.environment.parent.take() {
                    self.environment = *parent;
                }

                Ok(last_value)
            }
            _ => Err(InterpreterError::TypeError {
                message: format!("Cannot iterate over {:?}", iterable_value),
            }),
        }
    }

    fn eval_while_loop(
        &mut self,
        condition: &Expr,
        body: &Expr,
    ) -> Result<Value, InterpreterError> {
        let mut last_value = Value::Unit;

        loop {
            // Safepoint poll for GC coordination at start of each iteration
            self.safepoint_poll()?;

            let condition_value = self.eval_expr(condition.clone())?;
            let condition_bool = self.to_boolean(&condition_value)?;

            if !condition_bool {
                break;
            }

            last_value = self.eval_expr(body.clone())?;
        }

        Ok(last_value)
    }

    fn eval_loop(&mut self, body: &Expr) -> Result<Value, InterpreterError> {
        loop {
            // Safepoint poll for GC coordination at start of each iteration
            self.safepoint_poll()?;

            let _ = self.eval_expr(body.clone())?;
        }
    }

    /// Resolve arguments (both positional and named) for function calls
    fn resolve_arguments(&mut self, callee: &Value, arguments: Vec<Argument>) -> Result<Vec<Value>, InterpreterError> {
        // Get function parameter information if available
        let parameters = match callee {
            Value::Function(func) => Some(&func.parameters),
            _ => None, // For builtin functions and other callables, use positional-only
        };
        
        let mut resolved_args = Vec::new();
        let mut named_args = HashMap::new();
        let mut positional_count = 0;
        
        // First pass: collect positional and named arguments
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    if !named_args.is_empty() {
                        return Err(InterpreterError::RuntimeError {
                            message: "Positional arguments cannot come after named arguments".to_string(),
                        });
                    }
                    resolved_args.push(self.eval_expr(expr)?);
                    positional_count += 1;
                }
                Argument::Named { name, value } => {
                    let evaluated_value = self.eval_expr(value)?;
                    if named_args.contains_key(&name) {
                        return Err(InterpreterError::RuntimeError {
                            message: format!("Duplicate named argument: {}", name),
                        });
                    }
                    named_args.insert(name, evaluated_value);
                }
            }
        }
        
        // Second pass: resolve named arguments to correct positions (if we have parameter info)
        if let Some(params) = parameters {
            // Check for conflicts between positional and named arguments
            for (arg_name, _) in &named_args {
                if let Some(param_index) = params.iter().position(|p| &p.name == arg_name) {
                    if param_index < positional_count {
                        return Err(InterpreterError::RuntimeError {
                            message: format!("Argument '{}' specified both positionally and by name", arg_name),
                        });
                    }
                }
            }
            
            // Extend resolved_args to cover all parameters, filling with named args or defaults
            while resolved_args.len() < params.len() {
                let param_index = resolved_args.len();
                let param = &params[param_index];
                
                if let Some(named_value) = named_args.remove(&param.name) {
                    // Use named argument value
                    resolved_args.push(named_value);
                } else if let Some(default_expr) = &param.default_value {
                    // Use default value
                    let default_value = self.eval_expr(default_expr.clone())?;
                    resolved_args.push(default_value);
                } else {
                    // Missing required argument
                    return Err(InterpreterError::RuntimeError {
                        message: format!("Missing required argument: {}", param.name),
                    });
                }
            }
            
            // Check for unrecognized named arguments
            if !named_args.is_empty() {
                let unrecognized: Vec<String> = named_args.keys().cloned().collect();
                return Err(InterpreterError::RuntimeError {
                    message: format!("Unrecognized named argument(s): {}", unrecognized.join(", ")),
                });
            }
        } else {
            // For builtin functions, just append named arguments as positional
            for (_, value) in named_args {
                resolved_args.push(value);
            }
        }
        
        Ok(resolved_args)
    }

    /// Get a cached module if it exists and is still valid
    fn get_cached_module(&self, module_path: &str) -> Result<Option<ModuleCacheEntry>, InterpreterError> {
        if let Some(entry) = self.module_cache.get(module_path) {
            if let Some(file_path) = self.resolve_module_path(module_path).ok() {
                if let Ok(metadata) = std::fs::metadata(&file_path) {
                    if let Ok(modified) = metadata.modified() {
                        if entry.last_modified.map_or(true, |lm| modified > lm) {
                            return Ok(None); // Invalidated
                        }
                    }
                }
            }
            Ok(Some(entry.clone()))
        } else {
            Ok(None)
        }
    }
    
    /// Cache a module for future use
    fn cache_module(&mut self, module_path: String, module: Value, file_path: Option<std::path::PathBuf>, dependencies: Vec<String>) -> Result<(), InterpreterError> {
        let last_modified = if let Some(ref path) = file_path {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .map_err(|e| InterpreterError::RuntimeError {
                    message: format!("Failed to get file modification time: {}", e),
                })
                .ok()
        } else {
            None
        };
        
        let cache_entry = ModuleCacheEntry {
            module,
            file_path,
            last_modified,
            dependencies,
        };
        
        self.module_cache.insert(module_path.clone(), cache_entry);
        crate::log::get_logger().debug("interpreter", &format!("Cached module: {}", module_path));
        Ok(())
    }
    
    /// Bind module imports to current environment
    fn bind_module_imports(&mut self, module: &Value, items: &Option<Vec<String>>) -> Result<(), InterpreterError> {
        match items {
            Some(item_list) => {
                // Import specific items: use module { func1, func2 }
                for item in item_list {
                    if let Some(value) = self.get_module_export(module, item) {
                        self.environment.define(item.clone(), value);
                        crate::log::get_logger().debug("interpreter", &format!("Imported {} from module", item));
                    } else {
                        // Debug: show what exports are available
                        if let Value::Struct { fields, .. } = module {
                            let available: Vec<_> = fields.keys().collect();
                            crate::log::get_logger().debug("interpreter", &format!("Available exports: {:?}", available));
                        }
                        return Err(InterpreterError::UndefinedVariable { 
                            name: format!("Export '{}' not found in module", item) 
                        });
                    }
                }
            }
            None => {
                // This shouldn't happen with the new system, but handle gracefully
                return Err(InterpreterError::RuntimeError {
                    message: "Wildcard imports not supported in new module system".to_string(),
                });
            }
        }
        Ok(())
    }
    
    /// Clear module cache (useful for testing or when modules change)
    pub fn clear_module_cache(&mut self) {
        self.module_cache.clear();
        self.dependency_tracker = ModuleDependencyTracker::new();
        crate::log::get_logger().debug("interpreter", "Module cache cleared");
    }
    
    /// Get dependency information for a module
    pub fn get_module_dependencies(&self, module_path: &str) -> Vec<String> {
        self.dependency_tracker.dependencies.get(module_path).cloned().unwrap_or_default()
    }
    
    /// Get modules that depend on a given module
    pub fn get_module_dependents(&self, module_path: &str) -> Vec<String> {
        self.dependency_tracker.dependents.get(module_path).cloned().unwrap_or_default()
    }
    
    /// Check if a module dependency would create a circular dependency
    pub fn would_create_circular_dependency(&self, from: &str, to: &str) -> bool {
        self.dependency_tracker.check_circular_dependency(from, to)
    }
    
    /// Get all modules in the dependency chain for a given module
    pub fn get_dependency_chain(&self, module_path: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        self.collect_dependency_chain(module_path, &mut chain, &mut visited);
        chain
    }
    
    /// Recursively collect dependency chain
    fn collect_dependency_chain(&self, module_path: &str, chain: &mut Vec<String>, visited: &mut HashSet<String>) {
        if visited.contains(module_path) {
            return;
        }
        
        visited.insert(module_path.to_string());
        
        if let Some(dependencies) = self.dependency_tracker.dependencies.get(module_path) {
            for dep in dependencies {
                self.collect_dependency_chain(dep, chain, visited);
                if !chain.contains(dep) {
                    chain.push(dep.clone());
                }
            }
        }
    }
    
    /// Invalidate a module and all its dependents from cache
    pub fn invalidate_module(&mut self, module_path: &str) {
        let dependents = self.get_module_dependents(module_path);
        
        // Remove the module from cache
        self.module_cache.remove(module_path);
        crate::log::get_logger().debug("interpreter", &format!("Invalidated module: {}", module_path));
        
        // Recursively invalidate dependents
        for dependent in dependents {
            self.invalidate_module(&dependent);
        }
    }
    
    /// Check if a module is cached
    pub fn is_module_cached(&self, module_path: &str) -> bool {
        self.module_cache.contains_key(module_path)
    }
    
    /// Get module cache statistics
    pub fn get_module_cache_stats(&self) -> (usize, usize, usize) {
        let cached_modules = self.module_cache.len();
        let total_dependencies = self.dependency_tracker.dependencies.values().map(|v| v.len()).sum();
        let total_dependents = self.dependency_tracker.dependents.values().map(|v| v.len()).sum();
        
        (cached_modules, total_dependencies, total_dependents)
    }
    
    /// Export module dependency information for debugging
    pub fn export_dependency_graph(&self) -> HashMap<String, Vec<String>> {
        self.dependency_tracker.dependencies.clone()
    }

    fn eval_share_decl(&mut self, share: ShareDecl) -> Result<Value, InterpreterError> {
        match share {
            ShareDecl::Function(func) => self.eval_function_decl(func),
            ShareDecl::Let(letd) => self.eval_let_decl(letd),
            ShareDecl::Type(typed) => self.eval_type_decl(typed),
            ShareDecl::Use(use_decl) => self.eval_transitive_share(use_decl),
        }
    }

    /// Handle transitive sharing: share use module { items }
    fn eval_transitive_share(&mut self, use_decl: UseDecl) -> Result<Value, InterpreterError> {
        // Load the module and import the specified items
        let module_path = use_decl.path.join(".");
        let module = self.load_module_from_file(&module_path)?;
        
        // Import items into current environment (makes them available locally)
        self.bind_module_imports(&module, &Some(use_decl.items.clone()))?;
        
        // Note: The re-sharing is handled in load_module_from_file when processing ShareDecl::Use
        // This just ensures the items are available in the current module's environment
        
        Ok(Value::Unit)
    }

    /// Load a module from the file system or standard library
    fn load_module_from_file(&mut self, module_path: &str) -> Result<Value, InterpreterError> {
        // Check cache first
        if let Ok(Some(cached)) = self.get_cached_module(module_path) {
            return Ok(cached.module);
        }

        // Determine the file path
        let file_path = self.resolve_module_path(module_path)?;
        
        // Handle standard library modules
        if file_path.to_string_lossy().starts_with("__stdlib__/") {
            let stdlib_name = file_path.file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: format!("Invalid stdlib module path: {}", file_path.display())
                })?;
                
            let stdlib = crate::stdlib::get_stdlib();
            if let Some(module) = stdlib.get(stdlib_name) {
                // Cache the stdlib module
                self.cache_module(module_path.to_string(), module.clone(), None, Vec::new())?;
                return Ok(module.clone());
            } else {
                return Err(InterpreterError::RuntimeError {
                    message: format!("Standard library module '{}' not found", stdlib_name)
                });
            }
        }
        
        // Read and parse the module file
        let content = std::fs::read_to_string(&file_path)
            .map_err(|e| InterpreterError::RuntimeError { 
                message: format!("Failed to read module file {}: {}", file_path.display(), e) 
            })?;
            
        // Parse the module
        let parser = crate::parser::Parser::new();
        let program = parser.parse(&content)
            .map_err(|e| InterpreterError::RuntimeError { 
                message: format!("Failed to parse module {}: {:?}", file_path.display(), e) 
            })?;
            
        // Create a new environment for the module with builtins
        let mut module_env = Environment::new();
        
        // Add builtin functions to module environment
        for (name, func) in self.builtin_functions.get_functions() {
            module_env.define(name.clone(), Value::Builtin(func.clone()));
        }
        
        // Add stdlib modules to module environment
        for (name, module) in crate::stdlib::get_stdlib() {
            module_env.define(name, module);
        }
        
        // Save current environment and module path
        let saved_env = std::mem::replace(&mut self.environment, module_env);
        let saved_module_path = self.current_module_path.clone();
        self.current_module_path = Some(module_path.to_string());
        
        // Execute the module and collect exports
        let mut exports = std::collections::HashMap::new();
        let mut dependencies = Vec::new();
        
        for statement in program.statements {
            match statement {
                crate::ast::Statement::ShareDecl(share_decl) => {
                    match share_decl {
                        ShareDecl::Function(func_decl) => {
                            let value = self.eval_function_decl(func_decl.clone())?;
                            exports.insert(func_decl.name, value);
                        }
                        ShareDecl::Let(let_decl) => {
                            let value = self.eval_let_decl(let_decl.clone())?;
                            if let Pattern::Identifier(name) = let_decl.pattern {
                                exports.insert(name, value);
                            }
                        }
                        ShareDecl::Type(type_decl) => {
                            self.eval_type_decl(type_decl.clone())?;
                            // Export type information as a special Type value
                            let type_info = Value::TypeInfo {
                                name: type_decl.name.clone(),
                                definition: type_decl.definition,
                            };
                            exports.insert(type_decl.name, type_info);
                        }
                        ShareDecl::Use(use_decl) => {
                            // Handle transitive sharing: re-export items from another module
                            let module_path = use_decl.path.join(".");
                            let module = self.load_module_from_file(&module_path)?;
                            
                            // Re-export the specified items
                            for item_name in &use_decl.items {
                                if let Some(value) = self.get_module_export(&module, item_name) {
                                    exports.insert(item_name.clone(), value);
                                }
                            }
                        }
                    }
                }
                crate::ast::Statement::UseDecl(use_decl) => {
                    dependencies.push(use_decl.path.join("."));
                    self.eval_statement(crate::ast::Statement::UseDecl(use_decl))?;
                }
                _ => {
                    self.eval_statement(statement)?;
                }
            }
        }
        
        // Restore original environment and module path
        self.environment = saved_env;
        self.current_module_path = saved_module_path;
        
        // Create module struct
        let module = Value::Struct {
            type_name: "Module".to_string(),
            fields: exports,
        };
        
        // Cache the module
        let last_modified = if let Ok(metadata) = std::fs::metadata(&file_path) {
            metadata.modified().ok()
        } else { None };
        self.module_cache.insert(module_path.to_string(), ModuleCacheEntry {
            module: module.clone(),
            file_path: Some(file_path),
            last_modified,
            dependencies,
        });
        
        Ok(module)
    }
    
    /// Resolve module path to actual file path using dependency-free discovery
    /// 
    /// Discovery Rules (Feature 4):
    /// 1. Relative to current file - `use sibling_file { function }`
    /// 2. Relative to project root - `use utils.helpers { function }`
    /// 3. Standard library - `use std.io { println }` (built-in)
    /// 4. Current directory first - Always check same folder first
    fn resolve_module_path(&self, module_path: &str) -> Result<std::path::PathBuf, InterpreterError> {
        let debug_config = &self.module_debug_config;
        let _start_time = if debug_config.show_resolution_timing {
            Some(Instant::now())
        } else {
            None
        };

        if debug_config.enable_resolution_tracing {
            crate::log::get_logger().debug("interpreter", &format!("Resolving module: '{}' using dependency-free discovery", module_path));
        }

        // Try discovery algorithms in order of priority
        if let Ok(path) = self.discover_module_same_directory(module_path) {
            if debug_config.enable_resolution_tracing {
                crate::log::get_logger().debug("interpreter", &format!("Found in same directory: {}", path.display()));
            }
            return Ok(path);
        }

        if let Ok(path) = self.discover_module_project_root(module_path) {
            if debug_config.enable_resolution_tracing {
                crate::log::get_logger().debug("interpreter", &format!("Found relative to project root: {}", path.display()));
            }
            return Ok(path);
        }

        if let Ok(path) = self.discover_module_stdlib(module_path) {
            if debug_config.enable_resolution_tracing {
                crate::log::get_logger().debug("interpreter", &format!("Found in standard library: {}", path.display()));
            }
            return Ok(path);
        }

        // Enhanced error with discovery information
        self.create_module_not_found_error(module_path, debug_config)
    }

    /// 1. Check same directory as current file
    fn discover_module_same_directory(&self, module_path: &str) -> Result<std::path::PathBuf, InterpreterError> {
        let current_file_dir = if let Some(current_module) = &self.current_module_path {
            // If we're loading from within a module, use that module's directory
            if let Ok(cached) = self.get_cached_module(current_module) {
                if let Some(cached_entry) = cached {
                    if let Some(ref file_path) = cached_entry.file_path {
                        file_path.parent().unwrap_or_else(|| std::path::Path::new(".")).to_path_buf()
                    } else {
                        std::env::current_dir().map_err(|e| InterpreterError::RuntimeError { 
                            message: format!("Failed to get current directory: {}", e) 
                        })?
                    }
                } else {
                    std::env::current_dir().map_err(|e| InterpreterError::RuntimeError { 
                        message: format!("Failed to get current directory: {}", e) 
                    })?
                }
            } else {
                std::env::current_dir().map_err(|e| InterpreterError::RuntimeError { 
                    message: format!("Failed to get current directory: {}", e) 
                })?
            }
        } else {
            // No current module context, use current working directory
            std::env::current_dir().map_err(|e| InterpreterError::RuntimeError { 
                message: format!("Failed to get current directory: {}", e) 
            })?
        };

        self.try_resolve_in_directory(&current_file_dir, module_path)
    }

    /// 2. Check relative to project root
    fn discover_module_project_root(&self, module_path: &str) -> Result<std::path::PathBuf, InterpreterError> {
        let project_root = self.detect_project_root()?;
        self.try_resolve_in_directory(&project_root, module_path)
    }

    /// 3. Check standard library
    fn discover_module_stdlib(&self, module_path: &str) -> Result<std::path::PathBuf, InterpreterError> {
        // Check if this is a stdlib module
        let stdlib = crate::stdlib::get_stdlib();
        if stdlib.contains_key(module_path) {
            // Return a special path that indicates this is a stdlib module
            // We'll handle this specially in load_module_from_file
            return Ok(std::path::PathBuf::from(format!("__stdlib__/{}", module_path)));
        }

        // Check for std.* prefix
        if module_path.starts_with("std.") {
            let stdlib_name = &module_path[4..]; // Remove "std." prefix
            if stdlib.contains_key(stdlib_name) {
                return Ok(std::path::PathBuf::from(format!("__stdlib__/{}", stdlib_name)));
            }
        }

        Err(InterpreterError::RuntimeError {
            message: format!("Not a standard library module: {}", module_path)
        })
    }

    /// Detect project root by looking for common project indicators
    fn detect_project_root(&self) -> Result<std::path::PathBuf, InterpreterError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| InterpreterError::RuntimeError { 
                message: format!("Failed to get current directory: {}", e) 
            })?;

        // Look for project indicators in current and parent directories
        let mut dir = current_dir;
        loop {
            // Check for common project files
            let indicators = vec![
                "Cargo.toml",     // Rust project
                "package.json",   // Node.js project
                "requirements.txt", // Python project
                "go.mod",         // Go project
                ".git",           // Git repository
                "olang.toml",     // Future Olang project file
                "main.ol",        // Olang entry point
            ];

            for indicator in indicators {
                if dir.join(indicator).exists() {
                    return Ok(dir);
                }
            }

            // Move to parent directory
            if let Some(parent) = dir.parent() {
                dir = parent.to_path_buf();
            } else {
                // Reached filesystem root, use original current directory
                return std::env::current_dir()
                    .map_err(|e| InterpreterError::RuntimeError { 
                        message: format!("Failed to get current directory: {}", e) 
                    });
            }
        }
    }

    /// Try to resolve a module in a specific directory
    fn try_resolve_in_directory(&self, base_dir: &std::path::Path, module_path: &str) -> Result<std::path::PathBuf, InterpreterError> {
        // Handle dotted paths by splitting on . and joining with /
        let file_parts: Vec<String> = module_path.split('.').map(|s| s.to_string()).collect();
        let file_name = format!("{}.ol", file_parts.last().unwrap_or(&String::new()));
        let dir_path = if file_parts.len() > 1 {
            file_parts[..file_parts.len()-1].join("/")
        } else {
            String::new()
        };

        // Generate candidates in order of preference
        let mut candidates = Vec::new();
        
        // Direct file path
        if !dir_path.is_empty() {
            let target_dir = base_dir.join(&dir_path);
            candidates.push(target_dir.join(&file_name));
            
            // Feature 5: Check for automatic index files in directories
            candidates.push(target_dir.join("index.ol"));
            candidates.push(target_dir.join("mod.ol"));
            
            // Feature 5: Check if this is a directory import and generate index if needed
            if target_dir.exists() && target_dir.is_dir() {
                if let Ok(index_path) = self.ensure_directory_index(&target_dir) {
                    candidates.insert(0, index_path); // Prioritize generated index
                }
            }
        } else {
            candidates.push(base_dir.join(&file_name));
            
            // Feature 5: Check if importing a directory directly (e.g., use utils { ... })
            let target_dir = base_dir.join(module_path);
            if target_dir.exists() && target_dir.is_dir() {
                if let Ok(index_path) = self.ensure_directory_index(&target_dir) {
                    candidates.insert(0, index_path);
                }
                candidates.push(target_dir.join("index.ol"));
                candidates.push(target_dir.join("mod.ol"));
            }
        }

        // Try src/ subdirectory as well
        if !dir_path.is_empty() {
            let src_target_dir = base_dir.join("src").join(&dir_path);
            candidates.push(src_target_dir.join(&file_name));
            candidates.push(src_target_dir.join("mod.ol"));
            candidates.push(src_target_dir.join("index.ol"));
            
            // Feature 5: Auto-generate index for src subdirectories
            if src_target_dir.exists() && src_target_dir.is_dir() {
                if let Ok(index_path) = self.ensure_directory_index(&src_target_dir) {
                    candidates.push(index_path);
                }
            }
        } else {
            candidates.push(base_dir.join("src").join(&file_name));
            
            // Feature 5: Check src directory for direct imports
            let src_target_dir = base_dir.join("src").join(module_path);
            if src_target_dir.exists() && src_target_dir.is_dir() {
                if let Ok(index_path) = self.ensure_directory_index(&src_target_dir) {
                    candidates.push(index_path);
                }
            }
        }

        // Check each candidate
        for candidate in &candidates {
            if candidate.exists() && candidate.is_file() {
                return Ok(candidate.clone());
            }
        }

        Err(InterpreterError::RuntimeError {
            message: format!("Module '{}' not found in directory {}", module_path, base_dir.display())
        })
    }

    /// Feature 5: Ensure a directory has an index.ol file that aggregates all shared exports
    fn ensure_directory_index(&self, dir_path: &std::path::Path) -> Result<std::path::PathBuf, InterpreterError> {
        let index_path = dir_path.join("index.ol");
        
        // Scan directory for .ol files (excluding index.ol itself)
        let ol_files = self.scan_directory_for_modules(dir_path)?;
        
        if ol_files.is_empty() {
            return Err(InterpreterError::RuntimeError {
                message: format!("No .ol files found in directory {}", dir_path.display())
            });
        }
        
        // Check if index file needs updating
        let should_regenerate = self.should_regenerate_index(&index_path, &ol_files)?;
        
        if should_regenerate {
            self.generate_directory_index(dir_path, &ol_files, &index_path)?;
            crate::log::get_logger().debug("interpreter", &format!("Generated index file: {}", index_path.display()));
        }
        
        Ok(index_path)
    }
    
    /// Feature 5: Scan directory for .ol files and return their paths
    fn scan_directory_for_modules(&self, dir_path: &std::path::Path) -> Result<Vec<std::path::PathBuf>, InterpreterError> {
        let mut ol_files = Vec::new();
        
        if !dir_path.exists() || !dir_path.is_dir() {
            return Ok(ol_files);
        }
        
        let entries = std::fs::read_dir(dir_path)
            .map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to read directory {}: {}", dir_path.display(), e)
            })?;
            
        for entry in entries {
            let entry = entry.map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to read directory entry: {}", e)
            })?;
            
            let path = entry.path();
            if path.is_file() {
                if let Some(extension) = path.extension() {
                    if extension == "ol" {
                        // Skip index.ol and mod.ol to avoid circular references
                        if let Some(filename) = path.file_name() {
                            if filename != "index.ol" && filename != "mod.ol" {
                                ol_files.push(path);
                            }
                        }
                    }
                }
            }
        }
        
        // Sort for consistent ordering
        ol_files.sort();
        Ok(ol_files)
    }
    
    /// Feature 5: Check if index file needs regeneration
    fn should_regenerate_index(&self, index_path: &std::path::Path, ol_files: &[std::path::PathBuf]) -> Result<bool, InterpreterError> {
        // If index doesn't exist, need to generate
        if !index_path.exists() {
            return Ok(true);
        }
        
        // Get index file modification time
        let index_modified = std::fs::metadata(index_path)
            .and_then(|m| m.modified())
            .map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to get index file metadata: {}", e)
            })?;
        
        // Check if any .ol file is newer than the index
        for ol_file in ol_files {
            let file_modified = std::fs::metadata(ol_file)
                .and_then(|m| m.modified())
                .map_err(|e| InterpreterError::RuntimeError {
                    message: format!("Failed to get file metadata for {}: {}", ol_file.display(), e)
                })?;
                
            if file_modified > index_modified {
                return Ok(true);
            }
        }
        
        Ok(false)
    }
    
    /// Feature 5: Generate index.ol file that aggregates all shared exports
    fn generate_directory_index(&self, dir_path: &std::path::Path, ol_files: &[std::path::PathBuf], index_path: &std::path::Path) -> Result<(), InterpreterError> {
        let mut index_content = String::new();
        
        // Header comment
        index_content.push_str("// Auto-generated index file for directory: ");
        index_content.push_str(&dir_path.display().to_string());
        index_content.push_str("\n// This file automatically aggregates exports from all modules in this directory\n");
        index_content.push_str("// Generated by Olang Feature 5: Automatic Index Files\n\n");
        
        // Collect all modules and their exports
        let mut all_exports = Vec::new();
        for ol_file in ol_files {
            let exports = self.extract_shared_exports(ol_file)?;
            let module_name = ol_file.file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: format!("Invalid filename: {}", ol_file.display())
                })?;
            
            if !exports.is_empty() {
                all_exports.push((module_name.to_string(), exports));
            }
        }
        
        // For now, let's create a simple aggregation approach
        // Import all modules at the top
        for (module_name, exports) in &all_exports {
            index_content.push_str(&format!("use {} {{ {} }}\n", 
                module_name, 
                exports.join(", ")
            ));
        }
        
        index_content.push_str("\n");
        index_content.push_str("// All exports are automatically available through the individual module imports above\n");
        index_content.push_str("// This allows 'use utils { function_name }' to work by importing from this index\n");
        
        // Write the index file
        std::fs::write(index_path, index_content)
            .map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to write index file {}: {}", index_path.display(), e)
            })?;
            
        Ok(())
    }
    
    /// Feature 5: Extract shared exports from a .ol file
    fn extract_shared_exports(&self, file_path: &std::path::PathBuf) -> Result<Vec<String>, InterpreterError> {
        let content = std::fs::read_to_string(file_path)
            .map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to read file {}: {}", file_path.display(), e)
            })?;
        
        let parser = crate::parser::Parser::new();
        let program = parser.parse(&content)
            .map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to parse file {}: {:?}", file_path.display(), e)
            })?;
        
        let mut exports = Vec::new();
        
        for statement in program.statements {
            match statement {
                crate::ast::Statement::ShareDecl(share_decl) => {
                    match share_decl {
                        ShareDecl::Function(func_decl) => {
                            exports.push(func_decl.name);
                        }
                        ShareDecl::Let(let_decl) => {
                            if let Pattern::Identifier(name) = let_decl.pattern {
                                exports.push(name);
                            }
                        }
                        ShareDecl::Type(type_decl) => {
                            exports.push(type_decl.name);
                        }
                        ShareDecl::Use(use_decl) => {
                            // Add re-shared items to exports
                            for item_name in &use_decl.items {
                                exports.push(item_name.clone());
                            }
                        }
                    }
                }
                _ => {} // Ignore non-share declarations
            }
        }
        
        Ok(exports)
    }

    /// Create comprehensive error message for module not found
    fn create_module_not_found_error(&self, module_path: &str, debug_config: &ModuleDebugConfig) -> Result<std::path::PathBuf, InterpreterError> {
        if debug_config.verbose_error_messages {
            let mut search_info = Vec::new();
            
            // Show what directories were searched
            if let Ok(current_dir) = std::env::current_dir() {
                search_info.push(format!("Same directory: {}", current_dir.display()));
            }
            
            if let Ok(project_root) = self.detect_project_root() {
                search_info.push(format!("Project root: {}", project_root.display()));
            }
            
            // Check if it might be a typo of a stdlib module
            let stdlib = crate::stdlib::get_stdlib();
            let stdlib_suggestions: Vec<String> = stdlib.keys()
                .filter(|name| {
                    // Simple similarity check
                    let distance = levenshtein_distance(module_path, name);
                    distance <= 2 && distance > 0
                })
                .map(|s| s.to_string())
                .collect();

            let mut error_msg = format!(
                "Module '{}' not found.\n\nSearched locations:\n  - {}",
                module_path,
                search_info.join("\n  - ")
            );

            if !stdlib_suggestions.is_empty() {
                error_msg.push_str(&format!(
                    "\n\nDid you mean one of these standard library modules?\n  - {}",
                    stdlib_suggestions.join("\n  - ")
                ));
            }

            // Show available stdlib modules if it looks like a stdlib reference
            if module_path.starts_with("std.") || stdlib_suggestions.is_empty() {
                let stdlib_modules: Vec<String> = stdlib.keys().cloned().collect();
                error_msg.push_str(&format!(
                    "\n\nAvailable standard library modules:\n  - {}",
                    stdlib_modules.join("\n  - ")
                ));
            }
            
            Err(InterpreterError::RuntimeError { message: error_msg })
        } else {
            Err(InterpreterError::RuntimeError { 
                message: format!("Module file not found: {}", module_path) 
            })
        }
    }

    /// Get an export from a loaded module
    fn get_module_export(&self, module: &Value, export_name: &str) -> Option<Value> {
        match module {
            Value::Struct { fields, .. } => {
                fields.get(export_name).cloned()
            }
            _ => None,
        }
    }

    fn eval_use_decl(&mut self, use_decl: UseDecl) -> Result<Value, InterpreterError> {
        let module_path = use_decl.path.join(".");
        let module = self.load_module_from_file(&module_path)?;
        self.bind_module_imports(&module, &Some(use_decl.items))?;
        Ok(Value::Unit)
    }
}

impl Clone for Environment {
    fn clone(&self) -> Self {
        Self {
            variables: self.variables.clone(),
            parent: self.parent.clone(),
        }
    }
}

/// Module cache entry with metadata
#[derive(Debug, Clone)]
pub struct ModuleCacheEntry {
    pub module: Value,
    pub file_path: Option<std::path::PathBuf>,
    pub last_modified: Option<std::time::SystemTime>,
    pub dependencies: Vec<String>,
}

/// Module dependency tracking
#[derive(Debug, Clone)]
pub struct ModuleDependencyTracker {
    pub dependencies: HashMap<String, Vec<String>>, // module -> its dependencies
    pub dependents: HashMap<String, Vec<String>>,   // module -> modules that depend on it
}

impl ModuleDependencyTracker {
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
        }
    }
    
    pub fn add_dependency(&mut self, module: String, dependency: String) {
        self.dependencies.entry(module.clone()).or_insert_with(Vec::new).push(dependency.clone());
        self.dependents.entry(dependency).or_insert_with(Vec::new).push(module);
    }
    
    pub fn check_circular_dependency(&self, module: &str, dependency: &str) -> bool {
        self.has_path(dependency, module)
    }
    
    fn has_path(&self, from: &str, to: &str) -> bool {
        if from == to {
            return true;
        }
        
        if let Some(deps) = self.dependencies.get(from) {
            for dep in deps {
                if self.has_path(dep, to) {
                    return true;
                }
            }
        }
        
        false
    }
}

/// Calculate Levenshtein distance between two strings for similarity checking
fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.len();
    let len2 = s2.len();
    
    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }
    
    let s1_chars: Vec<char> = s1.chars().collect();
    let s2_chars: Vec<char> = s2.chars().collect();
    
    let mut dp = vec![vec![0; len2 + 1]; len1 + 1];
    
    // Initialize base cases
    for i in 0..=len1 {
        dp[i][0] = i;
    }
    for j in 0..=len2 {
        dp[0][j] = j;
    }
    
    // Fill the DP table
    for i in 1..=len1 {
        for j in 1..=len2 {
            let cost = if s1_chars[i - 1] == s2_chars[j - 1] { 0 } else { 1 };
            dp[i][j] = std::cmp::min(
                std::cmp::min(dp[i - 1][j] + 1, dp[i][j - 1] + 1),
                dp[i - 1][j - 1] + cost
            );
        }
    }
    
    dp[len1][len2]
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_module_cache_creation_and_retrieval() {
        let mut interpreter = Interpreter::new();
        
        // Test initial state
        assert_eq!(interpreter.module_cache.len(), 0);
        assert!(!interpreter.is_module_cached("test_module"));
        
        // Cache a module
        let test_module = Value::Struct {
            type_name: "Module".to_string(),
            fields: HashMap::new(),
        };
        
        interpreter.cache_module(
            "test_module".to_string(), 
            test_module.clone(), 
            None, 
            Vec::new()
        ).unwrap();
        
        // Verify module is cached
        assert!(interpreter.is_module_cached("test_module"));
        let cached = interpreter.get_cached_module("test_module").unwrap().unwrap();
        assert_eq!(cached.dependencies.len(), 0);
        
        // Test cache statistics
        let (cached_modules, _, _) = interpreter.get_module_cache_stats();
        assert_eq!(cached_modules, 1);
    }
    
    #[test]
    fn test_module_dependency_tracking() {
        let mut tracker = ModuleDependencyTracker::new();
        
        // Add some dependencies: A -> B, A -> C, B -> D
        tracker.add_dependency("A".to_string(), "B".to_string());
        tracker.add_dependency("A".to_string(), "C".to_string());
        tracker.add_dependency("B".to_string(), "D".to_string());
        
        // Test dependency queries
        assert_eq!(tracker.dependencies.get("A").unwrap().len(), 2);
        assert!(tracker.dependencies.get("A").unwrap().contains(&"B".to_string()));
        assert!(tracker.dependencies.get("A").unwrap().contains(&"C".to_string()));
        
        // Test dependent queries
        assert_eq!(tracker.dependents.get("B").unwrap().len(), 1);
        assert!(tracker.dependents.get("B").unwrap().contains(&"A".to_string()));
        
        // Test circular dependency detection
        assert!(!tracker.check_circular_dependency("A", "D")); // A -> B -> D (no cycle)
        assert!(tracker.check_circular_dependency("D", "A"));  // D -> A would create cycle
    }
    
    // Removed test_circular_dependency_prevention as it used the old import system
    
    #[test]
    fn test_module_cache_invalidation() {
        let mut interpreter = Interpreter::new();
        
        // Cache some modules with dependencies
        let module_a = Value::Struct {
            type_name: "Module".to_string(),
            fields: HashMap::new(),
        };
        let module_b = Value::Struct {
            type_name: "Module".to_string(),
            fields: HashMap::new(),
        };
        
        interpreter.cache_module("module_a".to_string(), module_a, None, Vec::new()).unwrap();
        interpreter.cache_module("module_b".to_string(), module_b, None, Vec::new()).unwrap();
        
        // Set up dependency: B depends on A
        interpreter.dependency_tracker.add_dependency("module_b".to_string(), "module_a".to_string());
        
        // Verify both modules are cached
        assert!(interpreter.is_module_cached("module_a"));
        assert!(interpreter.is_module_cached("module_b"));
        
        // Invalidate module A
        interpreter.invalidate_module("module_a");
        
        // Verify both A and its dependent B are invalidated
        assert!(!interpreter.is_module_cached("module_a"));
        assert!(!interpreter.is_module_cached("module_b"));
    }
    
    #[test]
    fn test_dependency_chain_analysis() {
        let mut interpreter = Interpreter::new();
        
        // Create dependency chain: A -> B -> C -> D
        interpreter.dependency_tracker.add_dependency("A".to_string(), "B".to_string());
        interpreter.dependency_tracker.add_dependency("B".to_string(), "C".to_string());
        interpreter.dependency_tracker.add_dependency("C".to_string(), "D".to_string());
        
        let chain = interpreter.get_dependency_chain("A");
        
        // Should include all dependencies in the chain
        assert!(chain.contains(&"B".to_string()));
        assert!(chain.contains(&"C".to_string()));
        assert!(chain.contains(&"D".to_string()));
        
        // Test dependency graph export
        let graph = interpreter.export_dependency_graph();
        assert!(graph.contains_key("A"));
        assert!(graph.contains_key("B"));
        assert!(graph.contains_key("C"));
    }
    
    #[test]
    fn test_module_cache_clearing() {
        let mut interpreter = Interpreter::new();
        
        // Cache some modules
        let test_module = Value::Struct {
            type_name: "Module".to_string(),
            fields: HashMap::new(),
        };
        
        interpreter.cache_module("module1".to_string(), test_module.clone(), None, Vec::new()).unwrap();
        interpreter.cache_module("module2".to_string(), test_module, None, Vec::new()).unwrap();
        interpreter.dependency_tracker.add_dependency("module1".to_string(), "module2".to_string());
        
        // Verify modules are cached and dependencies exist
        assert!(interpreter.is_module_cached("module1"));
        assert!(interpreter.is_module_cached("module2"));
        assert!(!interpreter.dependency_tracker.dependencies.is_empty());
        
        // Clear cache
        interpreter.clear_module_cache();
        
        // Verify everything is cleared
        assert!(!interpreter.is_module_cached("module1"));
        assert!(!interpreter.is_module_cached("module2"));
        assert!(interpreter.dependency_tracker.dependencies.is_empty());
        assert!(interpreter.dependency_tracker.dependents.is_empty());
    }
    
    #[test]
    fn test_would_create_circular_dependency() {
        let mut interpreter = Interpreter::new();
        
        // Set up a simple dependency chain: A -> B -> C
        interpreter.dependency_tracker.add_dependency("A".to_string(), "B".to_string());
        interpreter.dependency_tracker.add_dependency("B".to_string(), "C".to_string());
        
        // Test various potential circular dependencies
        assert!(!interpreter.would_create_circular_dependency("A", "D")); // No cycle
        assert!(!interpreter.would_create_circular_dependency("D", "E")); // No cycle
        assert!(interpreter.would_create_circular_dependency("C", "A"));  // Would create cycle
        assert!(interpreter.would_create_circular_dependency("C", "B"));  // Would create cycle
        assert!(interpreter.would_create_circular_dependency("B", "A"));  // Would create cycle
    }
}


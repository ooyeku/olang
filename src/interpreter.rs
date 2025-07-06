use crate::ast::{
    Argument, AsyncFunctionDecl, BinaryOp, BuiltinFunction, EnumVariantData, ErrorTypeDecl, ExportDecl, Expr,
    Function, FunctionDecl, ImportDecl, LetDecl, MapEntry, MatchArm, Pattern, Program, PromiseType,
    Statement, UnaryOp, Value,
};
use crate::async_runtime::AsyncRuntime;
use crate::builtin::BuiltinFunctions;
use crate::internal::{check_memory_pressure, LazyConfig};
use crate::ovm::gc::SafepointManager;
use crate::type_checker::TypeChecker;
use std::collections::HashMap;

use std::sync::Arc;
use std::time::{Duration, Instant};
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
            Statement::ImportDecl(import_decl) => self.eval_import_decl(import_decl),
            Statement::ExportDecl(export_decl) => self.eval_export_decl(export_decl),
            Statement::ErrorTypeDecl(error_type_decl) => self.eval_error_type_decl(error_type_decl),
        }
    }

    fn eval_error_type_decl(
        &mut self,
        _error_type_decl: ErrorTypeDecl,
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
                let mut arg_values = Vec::new();

                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => {
                            arg_values.push(self.eval_expr(expr)?);
                        }
                        Argument::Named { name: _, value } => {
                            // For now, treat named arguments as positional
                            // TODO: Implement proper named argument resolution
                            arg_values.push(self.eval_expr(value)?);
                        }
                    }
                }

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
                        let mut new_args = vec![left_value];
                        for arg in arguments {
                            match arg {
                                Argument::Positional(expr) => {
                                    new_args.push(self.eval_expr(expr)?);
                                }
                                Argument::Named { name: _, value } => {
                                    // For now, treat named arguments as positional
                                    // TODO: Implement proper named argument resolution in pipelines
                                    new_args.push(self.eval_expr(value)?);
                                }
                            }
                        }
                        let callee_value = self.eval_expr(*callee)?;
                        self.call_function(callee_value, new_args)
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
            // Async expressions - placeholder implementations for now
            Expr::Async {
                parameters,
                body,
                return_type: _return_type,
            } => {
                // Create async function like regular function but mark as async
                let closure = self.environment.variables.clone();
                Ok(Value::Function(Function {
                    name: None,
                    parameters,
                    body: *body,
                    closure,
                }))
            }
            Expr::Await { expression } => {
                // For now, just evaluate the expression directly
                // In full implementation, this would handle promise resolution
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
                    } => Err(InterpreterError::RuntimeError {
                        message: "Cannot await pending promise".to_string(),
                    }),
                    _ => Ok(value), // If not a promise, return as-is
                }
            }
            Expr::Promise {
                promise_type,
                value,
                delay: _,
            } => {
                let evaluated_value = self.eval_expr(*value)?;
                match promise_type {
                    PromiseType::Resolve => Ok(self.async_runtime.promise_resolve(evaluated_value)),
                    PromiseType::Reject => Ok(self.async_runtime.promise_reject(evaluated_value)),
                    PromiseType::Delay => {
                        // For now, just resolve immediately
                        // In full implementation, would use delay
                        Ok(self.async_runtime.promise_resolve(evaluated_value))
                    }
                }
            }
            Expr::All(expressions) => {
                // Evaluate all expressions and return as list
                let mut results = Vec::new();
                for expr in expressions {
                    results.push(self.eval_expr(expr)?);
                }
                Ok(Value::List(std::sync::Arc::from(results)))
            }
            Expr::Race(expressions) => {
                // For now, just return the first expression result
                // In full implementation, would race promises
                if let Some(first_expr) = expressions.into_iter().next() {
                    self.eval_expr(first_expr)
                } else {
                    Err(InterpreterError::RuntimeError {
                        message: "Race expression requires at least one argument".to_string(),
                    })
                }
            }
            Expr::Spawn(expression) => {
                // For now, just evaluate the expression
                // In full implementation, would spawn async task
                self.eval_expr(*expression)
            }
        }
    }

    fn eval_async_function_decl(
        &mut self,
        async_func_decl: AsyncFunctionDecl,
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

                // Create new environment with closure
                let mut new_env = Environment::new();
                for (k, v) in func.closure.iter() {
                    new_env.define(k.clone(), v.clone());
                }

                // If this is a named function, add it to its own scope for recursion
                if let Some(name) = &func.name {
                    new_env.define(name.clone(), Value::Function(func.clone()));
                }

                // Add parameters to environment, using defaults for missing arguments
                for (i, param) in func.parameters.iter().enumerate() {
                    let value = if i < arguments.len() {
                        // Use provided argument
                        arguments[i].clone()
                    } else if let Some(default_expr) = &param.default_value {
                        // Use default value - evaluate it in the current environment
                        self.eval_expr(default_expr.clone())?
                    } else {
                        // This should not happen due to our arity check above
                        return Err(InterpreterError::RuntimeError {
                            message: format!("Missing argument for parameter {}", param.name),
                        });
                    };
                    
                    new_env.define(param.name.clone(), value);
                }

                let mut new_interpreter = Interpreter {
                    environment: new_env,
                    builtin_functions: self.builtin_functions.clone(),
                    type_checker: self.type_checker.clone(),
                    async_runtime: AsyncRuntime::new(),
                    lazy_config: self.lazy_config.clone(),
                    safepoint_manager: self.safepoint_manager.clone(),
                    module_debug_config: self.module_debug_config.clone(),
                };
                new_interpreter.eval_expr(func.body)
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

    fn eval_import_decl(&mut self, import_decl: ImportDecl) -> Result<Value, InterpreterError> {
        // Enhanced module loading with file system support
        let module_path = &import_decl.module_path;
        
        // Try to load from file system first
        if let Ok(module) = self.load_module_from_file(module_path) {
            // Handle specific imports vs wildcard
            match &import_decl.items {
                Some(items) => {
                    // Import specific items: import { func1, func2 } from "module"
                    for item in items {
                        if let Some(value) = self.get_module_export(&module, item) {
                            self.environment.define(item.clone(), value);
                        } else {
                            return Err(InterpreterError::UndefinedVariable { 
                                name: format!("{}::{}", module_path, item) 
                            });
                        }
                    }
                }
                None => {
                    // Wildcard import: import * from "module"
                    if let Value::Struct { fields, .. } = module {
                        for (name, value) in fields {
                            self.environment.define(name, value);
                        }
                    }
                }
            }
            Ok(Value::Unit)
        } else {
            // Fallback to stdlib modules
            if let Some(module) = self.get_stdlib_module(module_path) {
                // Define the module in the environment
                self.environment.define(module_path.clone(), module);
                crate::log::get_logger().debug("interpreter", &format!("Imported stdlib module: {}", module_path));
                Ok(Value::Unit)
            } else {
                Err(InterpreterError::RuntimeError { 
                    message: format!("Module not found: {}", module_path) 
                })
            }
        }
    }

    /// Load a module from the file system
    fn load_module_from_file(&mut self, module_path: &str) -> Result<Value, InterpreterError> {
        // Determine the file path
        let file_path = self.resolve_module_path(module_path)?;
        
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
            
        // Create a new environment for the module
        let mut module_env = Environment::new();
        
        // Add stdlib modules to module environment
        for (name, module) in crate::stdlib::get_stdlib() {
            module_env.define(name, module);
        }
        
        // Save current environment
        let saved_env = std::mem::replace(&mut self.environment, module_env);
        
        // Execute the module and collect exports
        let mut exports = std::collections::HashMap::new();
        
        for statement in program.statements {
            match statement {
                crate::ast::Statement::ExportDecl(export_decl) => {
                    let value = self.eval_expr(export_decl.value)?;
                    exports.insert(export_decl.name, value);
                }
                _ => {
                    self.eval_statement(statement)?;
                }
            }
        }
        
        // Restore original environment
        self.environment = saved_env;
        
        // Return module as a struct with exports
        Ok(Value::Struct {
            type_name: "Module".to_string(),
            fields: exports,
        })
    }
    
    /// Resolve module path to actual file path with enhanced debugging
    fn resolve_module_path(&self, module_path: &str) -> Result<std::path::PathBuf, InterpreterError> {
        use std::path::PathBuf;
        
        let debug_config = &self.module_debug_config;
        let start_time = if debug_config.show_resolution_timing {
            Some(Instant::now())
        } else {
            None
        };

        if debug_config.enable_resolution_tracing {
            crate::log::get_logger().debug("interpreter", &format!("Resolving module: '{}'", module_path));
        }
        
        let current_dir = std::env::current_dir()
            .map_err(|e| InterpreterError::RuntimeError { 
                message: format!("Failed to get current directory: {}", e) 
            })?;
        
        // Enhanced candidate resolution with debugging
        let candidates = vec![
            // Relative to current directory
            current_dir.join(format!("{}.ol", module_path)),
            current_dir.join(format!("{}/mod.ol", module_path)),
            current_dir.join(format!("{}/index.ol", module_path)),
            
            // Relative to src directory
            current_dir.join("src").join(format!("{}.ol", module_path)),
            current_dir.join("src").join(format!("{}/mod.ol", module_path)),
            current_dir.join("src").join(format!("{}/index.ol", module_path)),
            
            // Absolute path if it looks like one
            PathBuf::from(format!("{}.ol", module_path)),
        ];

        if debug_config.log_search_paths {
            crate::log::get_logger().trace("interpreter", "Module search paths:");
            for (i, candidate) in candidates.iter().enumerate() {
                let status = if candidate.exists() { "exists" } else { "missing" };
                crate::log::get_logger().trace("interpreter", &format!("  {}. {} {}", i + 1, status, candidate.display()));
            }
        }
        
        for candidate in &candidates {
            if candidate.exists() && candidate.is_file() {
                if debug_config.enable_resolution_tracing {
                    crate::log::get_logger().debug("interpreter", &format!("Found module file: {}", candidate.display()));
                    if let Some(start) = start_time {
                        crate::log::get_logger().trace("interpreter", &format!("Module resolution time: {:?}", start.elapsed()));
                    }
                }
                return Ok(candidate.clone());
            }
        }

        // Enhanced error with debug information
        if debug_config.verbose_error_messages {
            let search_paths: Vec<String> = candidates.iter()
                .map(|p| p.display().to_string())
                .collect();
            
            Err(InterpreterError::RuntimeError { 
                message: format!(
                    "Module '{}' not found.\nSearched paths:\n  - {}",
                    module_path,
                    search_paths.join("\n  - ")
                )
            })
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
    
    /// Get a stdlib module by name
    fn get_stdlib_module(&self, name: &str) -> Option<Value> {
        let stdlib = crate::stdlib::get_stdlib();
        stdlib.get(name).cloned()
    }

    fn eval_export_decl(&mut self, export_decl: ExportDecl) -> Result<Value, InterpreterError> {
        // Evaluate the export value and store it
        let value = self.eval_expr(export_decl.value)?;
        self.environment
            .define(export_decl.name.clone(), value.clone());
        crate::log::get_logger().debug("interpreter", &format!("Export: {} = {:?}", export_decl.name, value));
        Ok(value)
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
}

impl Clone for Environment {
    fn clone(&self) -> Self {
        Self {
            variables: self.variables.clone(),
            parent: self.parent.clone(),
        }
    }
}

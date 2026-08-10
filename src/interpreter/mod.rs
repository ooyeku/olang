use crate::ast::{
    Argument, BinaryOp, BuiltinFunction, EnumVariantData, Expr, Function, FunctionDecl, LetDecl,
    MatchArm, Parameter, Program, PromiseType, ShareDecl, Statement, TestDecl, TypeAnnotation,
    UseDecl, Value,
};
use crate::async_runtime::AsyncRuntime;
use crate::builtin::BuiltinFunctions;
use crate::ovm::gc::SafepointManager;
use crate::type_checker::TypeChecker;
use im::HashMap as ImHashMap;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Integer range iterator that can't overflow (a plain `start..end + 1` panics
/// when `end == i64::MAX`).
struct RangeIter {
    next: i64,
    end: i64,
    inclusive: bool,
    done: bool,
}

impl Iterator for RangeIter {
    type Item = i64;

    fn next(&mut self) -> Option<i64> {
        if self.done {
            return None;
        }
        let current = self.next;
        let last = if self.inclusive {
            self.end
        } else {
            self.end - 1
        };
        if current >= last {
            self.done = true;
        } else {
            self.next = current + 1;
        }
        Some(current)
    }
}

mod errors;
mod modules;
mod ops;

/// The recursion limit that turns runaway recursion into a clean
/// "maximum call depth exceeded" error. The playground (wasm) build uses a
/// far lower value: wasm's call stack is much smaller than a native
/// thread's, and ~1000 deep interpreter frames overflow it *physically* —
/// a hard trap that poisons the whole wasm instance so every later program
/// also traps — well before a 1000 limit would fire. A low limit makes the
/// depth guard win the race, so the playground reports the error and stays
/// alive.
#[cfg(feature = "native")]
pub const DEFAULT_MAX_CALL_DEPTH: usize = 1000;
#[cfg(not(feature = "native"))]
pub const DEFAULT_MAX_CALL_DEPTH: usize = 400;
mod patterns;
pub use errors::{InterpreterError, IntuitiveErrorFormatter};

mod environment;
pub use environment::{Environment, ModuleDebugConfig};

/// Olang interpreter with optional type checking
pub struct Interpreter {
    environment: Environment,
    builtin_functions: BuiltinFunctions,
    type_checker: Option<TypeChecker>,
    async_runtime: AsyncRuntime,
    safepoint_manager: Arc<SafepointManager>,
    pub module_debug_config: ModuleDebugConfig,

    // Enhanced module system
    module_cache: HashMap<String, ModuleCacheEntry>,
    dependency_tracker: ModuleDependencyTracker,
    current_module_path: Option<String>, // For tracking current module during loading
    module_loading_stack: Vec<String>, // Feature 7: Track modules currently being loaded for circular detection

    // Feature 8: Smart caching system
    smart_cache_config: SmartCacheConfig,
    cache_statistics: CacheStatistics,

    // Feature 9: Intuitive error messages
    error_formatter: IntuitiveErrorFormatter,

    // MEMORY PROTECTION: Prevent exponential memory growth
    call_depth: usize,
    max_call_depth: usize,

    // MEMORY MONITORING: Track memory usage to prevent corruption
    memory_allocations: usize,
    max_memory_allocations: usize,

    // AGGRESSIVE MEMORY MANAGEMENT: Track large allocations and force cleanup
    large_allocation_count: usize,
    last_cleanup_operation: usize,

    /// Optional bytecode tier: hot functions are compiled and executed on the
    /// OVM instead of walking the AST. Disabled unless explicitly enabled.
    /// Boxed deliberately: the tier owns the whole VM (compiler, caches,
    /// execution state — 1.4 KB), and `call_function` moves it out of the
    /// interpreter and back on every call so the tier can borrow the
    /// interpreter for builtins. Inline, that was ~2.8 KB of memcpy per
    /// interpreted call; behind a box it is two pointer moves.
    bytecode_tier: Option<Box<crate::ovm::tier::BytecodeTier>>,

    /// Trait method implementations, keyed by (type name, method name) ->
    /// the concrete function. Populated by `impl` blocks; consulted when a
    /// `value.method(args)` call's field isn't a struct field.
    trait_impls: HashMap<(String, String), Function>,
    /// Default method bodies from `trait` declarations, keyed by
    /// (trait name, method name). Used when an impl doesn't override them.
    trait_defaults: HashMap<(String, String), Function>,
    /// Which traits each type implements, so a type can reach its trait's
    /// default methods: type name -> set of trait names.
    type_traits: HashMap<String, Vec<String>>,

    /// Names of declared unit enum variants (e.g. `North`, `AtEnd`). A bare
    /// identifier pattern is a variant equality test only when its name is one
    /// of these — otherwise it is a fresh binding. Without this, a binding
    /// sub-pattern like `b` in `Concat(a, b)` would be misread as a variant
    /// test whenever some `b` already in scope happened to hold a unit variant.
    unit_variant_names: HashSet<String>,

    /// Test-runner mode (`olang test`): when on, `test` blocks record their
    /// outcome here and execution continues past failures instead of
    /// aborting. When off (normal runs), a failing test block is an error —
    /// the long-standing inline behavior.
    test_mode: bool,
    test_results: Vec<TestOutcome>,

    /// Declared struct types: name -> field names. Construction of a
    /// declared struct validates its field set; an undeclared struct-literal
    /// name is an error. (Field VALUES are dynamic — declarations fix shape,
    /// not types.)
    struct_defs: HashMap<String, Vec<String>>,

    /// Package dependency map: dependency name -> the directory whose `.ol`
    /// files it exposes. A `use foo.bar` whose first segment is a dependency
    /// name resolves inside that directory rather than relative to the
    /// current file. Populated by the package manager before execution.
    dependency_map: HashMap<String, std::path::PathBuf>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        // Feature 8: Initialize smart caching
        let smart_cache_config = SmartCacheConfig::default();

        let mut interpreter = Self {
            environment: Environment::new(),
            builtin_functions: BuiltinFunctions::new(),
            type_checker: None,
            async_runtime: AsyncRuntime::new(),
            safepoint_manager: Arc::new(SafepointManager::new()),
            module_debug_config: ModuleDebugConfig::default(),

            // Enhanced module system
            module_cache: HashMap::new(),
            dependency_tracker: ModuleDependencyTracker::new(),
            current_module_path: None, // For tracking current module during loading
            module_loading_stack: Vec::new(), // Feature 7: Track modules currently being loaded for circular detection

            // Feature 8: Smart caching system
            smart_cache_config,
            cache_statistics: CacheStatistics::default(),
            error_formatter: IntuitiveErrorFormatter::default(),

            // MEMORY PROTECTION: Initialize recursion depth tracking
            call_depth: 0,
            max_call_depth: DEFAULT_MAX_CALL_DEPTH,

            // MEMORY MONITORING: Initialize memory tracking
            memory_allocations: 0,
            max_memory_allocations: 10000, // Prevent excessive allocations

            // AGGRESSIVE MEMORY MANAGEMENT: Initialize tracking
            large_allocation_count: 0,
            last_cleanup_operation: 0,
            bytecode_tier: None,
            trait_impls: HashMap::new(),
            trait_defaults: HashMap::new(),
            type_traits: HashMap::new(),
            unit_variant_names: HashSet::new(),
            test_mode: false,
            test_results: Vec::new(),
            struct_defs: HashMap::new(),
            dependency_map: HashMap::new(),
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

    /// Set the current file path for module resolution context
    /// This allows relative module imports to be resolved correctly
    /// when running a file from a different directory
    pub fn set_current_file(&mut self, file_path: &std::path::Path) {
        // Store the file path as the current module context
        if let Some(path_str) = file_path.to_str() {
            self.current_module_path = Some(path_str.to_string());

            // Calculate content hash if the file exists (to prevent cache invalidation)
            let content_hash = if file_path.exists() {
                self.calculate_file_hash(file_path).unwrap_or_default()
            } else {
                String::new()
            };

            // Also cache a dummy entry so discover_module_same_directory can find the directory
            let cache_entry = ModuleCacheEntry {
                module: Value::Unit,
                file_path: Some(file_path.to_path_buf()),
                last_modified: Some(crate::clock::system_now()),
                dependencies: vec![],
                content_hash,
                compilation_time: std::time::Duration::default(),
                access_count: 0,
                last_accessed: crate::clock::system_now(),
                cache_generation: 0,
                memory_size: 0,
            };
            self.module_cache.insert(path_str.to_string(), cache_entry);
        }
    }

    /// Clear the current file context
    pub fn clear_current_file(&mut self) {
        self.current_module_path = None;
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
        if let Some(ref mut type_checker) = self.type_checker
            && let Err(type_errors) = type_checker.check_program(&program)
        {
            // Convert type checking errors to proper InterpreterError
            let error_messages: Vec<String> =
                type_errors.iter().map(|e| format!("{:?}", e)).collect();
            let combined_message = error_messages.join("; ");

            return Err(InterpreterError::TypeError {
                message: format!("Type checking failed: {}", combined_message),
            });
        }

        let mut last_value = Value::Unit;
        for statement in &program.statements {
            last_value = self.eval_statement(statement)?;
        }
        Ok(last_value)
    }

    pub fn eval_statement(&mut self, statement: &Statement) -> Result<Value, InterpreterError> {
        // Safepoint poll for GC coordination
        self.safepoint_poll()?;

        match statement {
            Statement::Expression(expr) => self.eval_expr(expr),
            Statement::LetDecl(let_decl) => self.eval_let_decl(let_decl),
            Statement::FunctionDecl(func_decl) => self.eval_function_decl(func_decl.clone()),
            Statement::AsyncFunctionDecl(async_func_decl) => {
                self.eval_async_function_decl(async_func_decl.clone())
            }
            Statement::TypeDecl(type_decl) => self.eval_type_decl(type_decl.clone()),
            Statement::ErrorTypeDecl(error_type_decl) => {
                self.eval_error_type_decl(error_type_decl.clone())
            }
            Statement::ShareDecl(share_decl) => self.eval_share_decl(share_decl.clone()),
            Statement::UseDecl(use_decl) => self.eval_use_decl(use_decl.clone()),
            Statement::TestDecl(test_decl) => self.eval_test_decl(test_decl.clone()),
            Statement::TraitDecl(trait_decl) => self.eval_trait_decl(trait_decl.clone()),
            Statement::ImplDecl(impl_decl) => self.eval_impl_decl(impl_decl.clone()),
        }
    }

    /// Record a trait's default method bodies. The trait itself introduces no
    /// runtime binding; it is a contract that `impl` blocks fulfil.
    fn eval_trait_decl(
        &mut self,
        trait_decl: crate::ast::TraitDecl,
    ) -> Result<Value, InterpreterError> {
        let closure = self.environment.flat_snapshot();
        for method in &trait_decl.methods {
            if let Some(body) = &method.default_body {
                let function = Function {
                    name: Some(method.name.clone()),
                    parameters: method.parameters.clone(),
                    body: Arc::new(body.clone()),
                    closure: Arc::new(closure.clone()),
                    param_bounds: Vec::new(),
                };
                if let Some(tier) = self.bytecode_tier.as_mut() {
                    tier.note_trait_default(
                        trait_decl.name.clone(),
                        method.name.clone(),
                        function.clone(),
                    );
                }
                self.trait_defaults
                    .insert((trait_decl.name.clone(), method.name.clone()), function);
            }
        }
        Ok(Value::Unit)
    }

    /// Register the methods of an `impl Trait for Type` block for runtime
    /// dispatch, and note that Type implements Trait.
    fn eval_impl_decl(
        &mut self,
        impl_decl: crate::ast::ImplDecl,
    ) -> Result<Value, InterpreterError> {
        let closure = self.environment.flat_snapshot();
        for method in &impl_decl.methods {
            let function = Function {
                name: Some(method.name.clone()),
                parameters: method.parameters.clone(),
                body: Arc::new(method.body.clone()),
                closure: Arc::new(closure.clone()),
                param_bounds: Vec::new(),
            };
            // Tell the tier this method body exists under its bare name. A
            // method dispatched by receiver type (`s.area()`) reaches the tier
            // keyed only by "area"; when two impls define `area`, the tier's
            // ambiguity guard sees two distinct bodies and keeps the name on
            // the interpreter — so dispatch stays correct. A single-impl
            // method is unambiguous and still promotes. (Before struct
            // arguments became tier-representable, these never compiled, which
            // hid the need to note them.)
            if let Some(tier) = self.bytecode_tier.as_mut() {
                tier.note_function(method.name.clone(), function.clone());
                tier.note_trait_impl(
                    impl_decl.type_name.clone(),
                    method.name.clone(),
                    function.clone(),
                );
            }
            self.trait_impls
                .insert((impl_decl.type_name.clone(), method.name.clone()), function);
        }
        if let Some(tier) = self.bytecode_tier.as_mut() {
            tier.note_type_trait(impl_decl.type_name.clone(), impl_decl.trait_name.clone());
        }
        let traits = self
            .type_traits
            .entry(impl_decl.type_name.clone())
            .or_default();
        if !traits.contains(&impl_decl.trait_name) {
            traits.push(impl_decl.trait_name.clone());
        }
        Ok(Value::Unit)
    }

    /// Map trait bounds on type parameters to the positions of parameters
    /// annotated with those type variables. `<T: Show>(x: T)` yields
    /// `[(0, ["Show"])]`. A parameter must be annotated with the bare type
    /// variable (`x: T`) for the bound to attach.
    fn resolve_param_bounds(
        type_param_bounds: &[(String, Vec<String>)],
        parameters: &[Parameter],
    ) -> Vec<(usize, Vec<String>)> {
        if type_param_bounds.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (index, param) in parameters.iter().enumerate() {
            let annotated = match &param.type_annotation {
                Some(TypeAnnotation::TypeVariable(name)) => Some(name),
                Some(TypeAnnotation::Custom(name)) => Some(name),
                _ => None,
            };
            if let Some(type_var) = annotated {
                for (bound_var, traits) in type_param_bounds {
                    if bound_var == type_var {
                        out.push((index, traits.clone()));
                    }
                }
            }
        }
        out
    }

    /// Public: whether a value's runtime type implements a trait. Backs the
    /// `implements(value, "Trait")` builtin.
    pub fn value_implements(&self, value: &Value, trait_name: &str) -> bool {
        self.type_implements(&value.type_name(), trait_name)
    }

    /// Whether a runtime type name satisfies a trait — either via an explicit
    /// `impl Trait for Type`, or because the type is the trait's own name
    /// (allowing a bound to name a concrete type too).
    fn type_implements(&self, type_name: &str, trait_name: &str) -> bool {
        if type_name == trait_name {
            return true;
        }
        self.type_traits
            .get(type_name)
            .is_some_and(|traits| traits.iter().any(|t| t == trait_name))
    }

    /// Check every trait bound of a function against the supplied arguments.
    /// Returns a clear error naming the argument, its type, and the unmet
    /// trait when a bound is violated.
    fn check_param_bounds(
        &self,
        func: &Function,
        arguments: &[Value],
    ) -> Result<(), InterpreterError> {
        for (index, traits) in &func.param_bounds {
            if let Some(arg) = arguments.get(*index) {
                let type_name = arg.type_name();
                for trait_name in traits {
                    if !self.type_implements(&type_name, trait_name) {
                        let fn_name = func.name.as_deref().unwrap_or("<lambda>");
                        return Err(InterpreterError::TypeError {
                            message: format!(
                                "{}: argument {} of type {} does not implement trait {}",
                                fn_name,
                                index + 1,
                                type_name,
                                trait_name
                            ),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve a method for `type_name`: a concrete impl first, then any
    /// default from a trait that type implements.
    fn lookup_method(&self, type_name: &str, method: &str) -> Option<Function> {
        if let Some(f) = self
            .trait_impls
            .get(&(type_name.to_string(), method.to_string()))
        {
            return Some(f.clone());
        }
        if let Some(traits) = self.type_traits.get(type_name) {
            for trait_name in traits {
                if let Some(f) = self
                    .trait_defaults
                    .get(&(trait_name.clone(), method.to_string()))
                {
                    return Some(f.clone());
                }
            }
        }
        None
    }

    fn eval_error_type_decl(
        &mut self,
        error_type_decl: crate::ast::ErrorTypeDecl,
    ) -> Result<Value, InterpreterError> {
        // An error declaration behaves like an enum: each bare variant binds
        // as a unit value, each payload variant as a constructor taking the
        // payload fields positionally. The resulting values are ordinary
        // enum values, so `Err(NotFound)` and `match r { Err(Invalid(m)) =>
        // ... }` use the existing Result and pattern machinery.
        for variant in &error_type_decl.variants {
            let value = if variant.fields.is_empty() {
                self.unit_variant_names.insert(variant.name.clone());
                if let Some(tier) = self.bytecode_tier.as_mut() {
                    tier.note_unit_variant(variant.name.clone());
                }
                Value::Enum {
                    type_name: error_type_decl.name.clone(),
                    variant_name: variant.name.clone(),
                    variant_data: EnumVariantData::Unit,
                }
            } else {
                Value::EnumConstructor {
                    type_name: error_type_decl.name.clone(),
                    variant_name: variant.name.clone(),
                    arity: variant.fields.len(),
                }
            };
            self.environment.define(variant.name.clone(), value);
        }
        Ok(Value::Unit)
    }

    fn eval_let_decl(&mut self, let_decl: &LetDecl) -> Result<Value, InterpreterError> {
        let value = if let Some(expr) = &let_decl.value {
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
        // Convert ImHashMap to regular HashMap for closure storage
        // O(1): the environment's flat map is persistent, adopt it directly
        let closure = self.environment.flat_snapshot();

        // Resolve identifiers to frame slots once, at declaration — the
        // call path then indexes instead of probing names (with per-use
        // verification and name fallback, so this can never change results)
        let param_names: Vec<String> = func_decl
            .parameters
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let resolved_body = crate::resolve::Resolver::resolve_function_body(
            &func_decl.body,
            Some(&func_decl.name),
            &param_names,
        );

        // Resolve trait bounds to parameter positions: a parameter annotated
        // with a bounded type variable (`x: T` where `T: Show`) records
        // (index, required traits), checked against the argument at call time.
        let param_bounds =
            Self::resolve_param_bounds(&func_decl.type_param_bounds, &func_decl.parameters);

        let function = Function {
            name: Some(func_decl.name.clone()),
            parameters: func_decl.parameters,
            body: Arc::new(resolved_body),
            closure: Arc::new(closure),
            param_bounds,
        };

        // Let the bytecode tier know this function exists, so a promoted
        // function that calls it can have it compiled too
        if let Some(tier) = self.bytecode_tier.as_mut() {
            tier.note_function(func_decl.name.clone(), function.clone());
        }

        let function_value = Value::Function(function);

        // Define the function in the current environment so it can be called recursively
        self.environment
            .define(func_decl.name, function_value.clone());

        Ok(function_value)
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, InterpreterError> {
        match expr {
            Expr::Integer(n) => Ok(Value::Integer(*n)),
            Expr::Float(x) => Ok(Value::Float(*x)),
            Expr::String(s) => Ok(Value::String(s.clone())),
            Expr::Boolean(b) => Ok(Value::Boolean(*b)),
            Expr::List(items_rc) => {
                // MEMORY MONITORING: Track list creation to prevent memory corruption
                self.track_allocation(items_rc.len())?;

                let mut values = Vec::with_capacity(items_rc.len()); // Pre-allocate
                for item in items_rc.iter() {
                    values.push(self.eval_expr(item)?);
                }

                // AGGRESSIVE MEMORY MANAGEMENT: Cleanup after large list creation
                if items_rc.len() > 10 {
                    self.force_memory_cleanup();
                }

                Ok(Value::List(std::sync::Arc::from(values)))
            }
            Expr::Tuple(items_rc) => {
                let mut values = Vec::new();
                for item in items_rc.iter() {
                    values.push(self.eval_expr(item)?);
                }
                Ok(Value::Tuple(std::sync::Arc::new(values)))
            }
            Expr::Identifier(name) => self
                .environment
                .get(name)
                .ok_or_else(|| InterpreterError::UndefinedVariable { name: name.clone() }),
            Expr::LocalRef { name, depth, slot } => self
                .environment
                .get_slot(name, *depth, *slot)
                .ok_or_else(|| InterpreterError::UndefinedVariable { name: name.clone() }),
            Expr::LocalAssign {
                name,
                depth,
                slot,
                value,
            } => {
                let val = self.eval_expr(value)?;
                match self.environment.set_slot(name, *depth, *slot, val.clone()) {
                    Ok(()) => Ok(val),
                    Err(_) => {
                        // Same behavior as unresolved assignment to a new name
                        self.environment.define(name.clone(), val.clone());
                        Ok(val)
                    }
                }
            }
            Expr::Call { callee, arguments } => {
                // Method-call dispatch: `receiver.method(args)` where `method`
                // is not a struct field resolves to a trait implementation for
                // the receiver's runtime type, with the receiver passed as
                // `self`. Struct fields take precedence, preserving field
                // access that happens to hold a callable.
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    let receiver = self.eval_expr(object)?;
                    let is_struct_field = matches!(
                        &receiver,
                        Value::Struct { fields, .. } if fields.contains_key(field)
                    );
                    if !is_struct_field
                        && let Some(method) = self.lookup_method(&receiver.type_name(), field)
                    {
                        let mut arg_values = vec![receiver];
                        for arg in arguments {
                            let expr = match arg {
                                Argument::Positional(e) => e,
                                Argument::Named { value, .. } => value,
                            };
                            arg_values.push(self.eval_expr(expr)?);
                        }
                        return self.call_function(Value::Function(method), arg_values);
                    }
                }

                let callee_value = self.eval_expr(callee)?;

                // Enhanced named argument resolution
                let arg_values = self.resolve_arguments(&callee_value, arguments)?;
                self.call_function(callee_value, arg_values)
            }
            Expr::Lambda {
                parameters, body, ..
            } => {
                // Capture all accessible variables from the environment chain
                let closure = self.collect_all_accessible_variables();
                let param_names: Vec<String> = parameters.iter().map(|p| p.name.clone()).collect();
                let resolved_body =
                    crate::resolve::Resolver::resolve_function_body(body, None, &param_names);
                Ok(Value::Function(Function {
                    name: None,
                    parameters: parameters.clone(),
                    body: Arc::new(resolved_body),
                    closure: Arc::new(closure),
                    param_bounds: Vec::new(),
                }))
            }
            Expr::Pipeline { left, right } => {
                let left_value = self.eval_expr(left)?;
                match right.as_ref() {
                    Expr::Call { callee, arguments } => {
                        // Enhanced named argument resolution for pipelines
                        let callee_value = self.eval_expr(callee)?;

                        // The piped value fills the first parameter, so resolve
                        // the explicit arguments against the remaining ones —
                        // otherwise `5 |> add(3)` errors on "missing" param.
                        let resolve_target = match &callee_value {
                            Value::Function(func) if !func.parameters.is_empty() => {
                                let mut shifted = func.clone();
                                shifted.parameters.remove(0);
                                Value::Function(shifted)
                            }
                            other => other.clone(),
                        };
                        let additional_args = self.resolve_arguments(&resolve_target, arguments)?;

                        // Prepend the piped value as the first argument
                        let mut final_args = vec![left_value];
                        final_args.extend(additional_args);

                        self.call_function(callee_value, final_args)
                    }
                    Expr::Identifier(name) => {
                        let function_value = self.environment.get(name).ok_or_else(|| {
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
                let value = self.eval_expr(value)?;
                self.eval_match(value, arms)
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = self.eval_expr(condition)?;
                let condition_bool = self.to_boolean(&condition)?;

                if condition_bool {
                    self.eval_expr(then_branch)
                } else if let Some(else_expr) = else_branch {
                    self.eval_expr(else_expr)
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
                // The logical operators short-circuit: the right operand is
                // evaluated only when the left doesn't already settle the
                // result. `false && x` is false and `true || x` is true
                // without touching `x` — so guards like
                // `i < len && ok(at(i))` are safe. The non-short-circuiting
                // path still runs through eval_binary_op for its type checks.
                match op {
                    BinaryOp::And => {
                        let left = self.eval_expr(left)?;
                        if matches!(left, Value::Boolean(false)) {
                            return Ok(Value::Boolean(false));
                        }
                        let right = self.eval_expr(right)?;
                        self.eval_binary_op(left, op.clone(), right)
                    }
                    BinaryOp::Or => {
                        let left = self.eval_expr(left)?;
                        if matches!(left, Value::Boolean(true)) {
                            return Ok(Value::Boolean(true));
                        }
                        let right = self.eval_expr(right)?;
                        self.eval_binary_op(left, op.clone(), right)
                    }
                    _ => {
                        let left = self.eval_expr(left)?;
                        let right = self.eval_expr(right)?;
                        self.eval_binary_op(left, op.clone(), right)
                    }
                }
            }
            Expr::UnaryOp { op, operand } => {
                let operand = self.eval_expr(operand)?;
                self.eval_unary_op(op.clone(), operand)
            }
            Expr::Range {
                start,
                end,
                inclusive,
            } => {
                let start_val = self.eval_expr(start)?;
                let end_val = self.eval_expr(end)?;
                self.eval_range(start_val, end_val, *inclusive)
            }
            Expr::StructLiteral(struct_literal) => self.eval_struct_literal(struct_literal),
            Expr::AnonymousObject { fields } => self.eval_anonymous_object(fields),
            Expr::MapLiteral { entries } => self.eval_map_literal(entries),
            Expr::FieldAccess { object, field } => self.eval_field_access(object, field),
            Expr::ResultOk(expr) => {
                let value = self.eval_expr(expr)?;
                Ok(Value::Ok(Box::new(value)))
            }
            Expr::ResultErr(expr) => {
                let value = self.eval_expr(expr)?;
                Ok(Value::Err(Box::new(value)))
            }
            Expr::Try(expr) => {
                let value = self.eval_expr(expr)?;
                match value {
                    Value::Ok(inner) => Ok(*inner),
                    // Early-return: unwind to the enclosing function call,
                    // which returns this Err to its caller.
                    err @ Value::Err(_) => Err(InterpreterError::ErrPropagation(err)),
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
                let try_result = self.eval_expr(try_block)?;
                match try_result {
                    Value::Ok(inner) => Ok(*inner),
                    Value::Err(err) => {
                        // Create new scope for catch block with error variable
                        let parent = self.environment.clone();
                        self.environment = Environment::with_parent(parent);
                        self.environment.define(catch_var.clone(), *err);

                        let result = self.eval_expr(catch_block);

                        // Restore parent environment
                        if let Some(parent) = self.environment.parent.take() {
                            self.environment =
                                Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                        }

                        result
                    }
                    // A non-Result value passes through unchanged: the try
                    // block succeeded with a plain value. This is what makes
                    // `try { await task } catch (e) { fallback }` work when
                    // the task succeeds (await yields the bare value) as well
                    // as when it fails (await yields Err).
                    other => Ok(other),
                }
            }
            Expr::ForLoop {
                variable,
                iterable,
                body,
            } => self.eval_for_loop(variable, iterable, body),
            Expr::ParForLoop {
                variable,
                iterable,
                body,
            } => self.eval_par_for_loop(variable, iterable, body),
            Expr::WhileLoop { condition, body } => self.eval_while_loop(condition, body),
            Expr::Loop { body } => self.eval_loop(body),
            Expr::Break(value) => {
                let v = match value {
                    Some(e) => self.eval_expr(e)?,
                    None => Value::Unit,
                };
                Err(InterpreterError::BreakSignal(v))
            }
            Expr::Continue => Err(InterpreterError::ContinueSignal),
            Expr::Return(value) => {
                let v = match value {
                    Some(e) => self.eval_expr(e)?,
                    None => Value::Unit,
                };
                Err(InterpreterError::ReturnSignal(v))
            }
            Expr::Assignment { target, value } => {
                let val = self.eval_expr(value)?;
                self.environment.set(target, val.clone()).or_else(|_| {
                    // If variable not defined, define it
                    self.environment.define(target.clone(), val.clone());
                    Ok(())
                })?;
                Ok(val)
            }
            Expr::RawString(s) => Ok(Value::String(std::sync::Arc::new(s.as_str().to_string()))),
            Expr::TemplateString { parts } => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        crate::ast::TemplatePart::Literal(s) => result.push_str(s),
                        crate::ast::TemplatePart::Interpolation(expr) => {
                            let val = self.eval_expr(expr)?;
                            // For template interpolation, we want raw values without quotes
                            match val {
                                Value::String(s) => result.push_str(&s),
                                Value::Integer(n) => result.push_str(&n.to_string()),
                                Value::Float(x) => result.push_str(&crate::ast::format_float(x)),
                                Value::Boolean(b) => result.push_str(&b.to_string()),
                                other => result.push_str(&format!("{}", other)),
                            }
                        }
                    }
                }
                Ok(Value::String(result.into()))
            }
            Expr::BitwiseOp { left, op, right } => {
                let left_val = self.eval_expr(left)?;
                let right_val = self.eval_expr(right)?;

                match (left_val, right_val) {
                    (Value::Integer(l), Value::Integer(r)) => {
                        let shift_amount = |r: i64| {
                            u32::try_from(r).ok().filter(|s| *s < 64).ok_or_else(|| {
                                InterpreterError::RuntimeError {
                                    message: format!(
                                        "Shift amount {} out of range (must be 0..64)",
                                        r
                                    ),
                                }
                            })
                        };
                        let result = match op {
                            crate::ast::BitwiseOp::And => l & r,
                            crate::ast::BitwiseOp::Or => l | r,
                            crate::ast::BitwiseOp::Xor => l ^ r,
                            crate::ast::BitwiseOp::Shl => l.wrapping_shl(shift_amount(r)?),
                            crate::ast::BitwiseOp::Shr => l.wrapping_shr(shift_amount(r)?),
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
                self.eval_expr(expr)
            }
            Expr::Rest(expr) => {
                // For now, just evaluate the inner expression
                // Rest semantics would be handled in pattern matching
                self.eval_expr(expr)
            }
            Expr::Index { object, index } => {
                let object_value = self.eval_expr(object)?;
                let index_value = self.eval_expr(index)?;

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
                        let string_len = string.chars().count();
                        let index = if idx < 0 {
                            // Negative indexing from end
                            if (-idx) as usize > string_len {
                                return Err(InterpreterError::RuntimeError {
                                    message: format!(
                                        "Index {} out of bounds for string of length {}",
                                        idx, string_len
                                    ),
                                });
                            }
                            string_len - ((-idx) as usize)
                        } else {
                            idx as usize
                        };

                        if let Some(ch) = string.chars().nth(index) {
                            // More efficient: create single-char string directly
                            Ok(Value::String(ch.to_string().into()))
                        } else {
                            Err(InterpreterError::RuntimeError {
                                message: format!(
                                    "Index {} out of bounds for string of length {}",
                                    idx, string_len
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
                // Convert ImHashMap to regular HashMap for closure storage
                // O(1): adopt the persistent flat map directly
                let closure = self.environment.flat_snapshot();
                let function = Function {
                    name: None,
                    parameters: parameters.clone(),
                    body: Arc::new((**body).clone()),
                    closure: Arc::new(closure),
                    param_bounds: Vec::new(),
                };

                // Return a function that when called returns a promise
                Ok(Value::Function(function))
            }
            Expr::Await { expression } => {
                let value = self.eval_expr(expression)?;
                let (deadline, outcome) = self.settle_info(value)?;
                Self::sleep_until_epoch_ms(deadline);
                match outcome {
                    Ok(v) => Ok(v),
                    // Rejection is a value, not a crash: awaiting a rejected
                    // promise (a failed spawn task, Promise.reject, or a
                    // rejecting all/race) yields `Err(e)`, composing with
                    // match, unwrap_or, `?`, and try/catch like every other
                    // fallible result in the language. Previously this raised
                    // a hard error nothing could catch — one failed worker
                    // killed the whole program.
                    Err(e) => Ok(Value::Err(Box::new(e))),
                }
            }
            Expr::Promise {
                promise_type,
                value,
                delay,
            } => {
                let evaluated_value = self.eval_expr(value)?;
                match promise_type {
                    PromiseType::Resolve => Ok(self.async_runtime.promise_resolve(evaluated_value)),
                    PromiseType::Reject => Ok(self.async_runtime.promise_reject(evaluated_value)),
                    PromiseType::Delay => {
                        // Enhanced delay implementation
                        if let Some(delay_expr) = delay {
                            let delay_value = self.eval_expr(delay_expr)?;
                            match delay_value {
                                Value::Integer(ms) if ms >= 0 => {
                                    // The interpreter is synchronous — there is
                                    // no scheduler to resolve this later. Carry
                                    // the deadline in the value so `await` can
                                    // sleep out the remainder; the old path
                                    // registered with a runtime nothing drains,
                                    // leaking an entry per delay and making
                                    // every await of it error.
                                    let deadline = crate::clock::system_now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_millis() as u64)
                                        .unwrap_or(0)
                                        .saturating_add(ms as u64);
                                    Ok(Value::Promise {
                                        state: crate::ast::PromiseState::Pending,
                                        value: Some(Box::new(evaluated_value)),
                                        error: None,
                                        resolve_at_epoch_ms: Some(deadline),
                                        task_id: None,
                                    })
                                }
                                Value::Integer(_) => Err(InterpreterError::RuntimeError {
                                    message: "Delay must be a non-negative integer".to_string(),
                                }),
                                _ => Err(InterpreterError::TypeError {
                                    message: "Delay must be an integer representing milliseconds"
                                        .to_string(),
                                }),
                            }
                        } else {
                            // Default delay of 0ms (immediate resolution)
                            Ok(self.async_runtime.promise_resolve(evaluated_value))
                        }
                    }
                }
            }
            Expr::All(list_expr) => {
                // Await every promise in the list, resolving to the list of
                // their values. Delayed promises carry a deadline, so
                // "concurrent" fan-out sleeps once until the *latest* deadline
                // (total time = the longest delay, not their sum) — correct
                // concurrent timing even on a single thread. Rejects as soon
                // as any input has already rejected.
                let promises = self.eval_promise_collection(list_expr, "all")?;
                let mut settled = Vec::with_capacity(promises.len());
                for value in promises {
                    settled.push(self.settle_info(value)?);
                }

                // A rejection short-circuits the whole thing.
                for (_, outcome) in &settled {
                    if let Err(err) = outcome {
                        return Ok(self.async_runtime.promise_reject(err.clone()));
                    }
                }

                // Sleep once until the last deadline, then collect values.
                let deadline = settled.iter().map(|(t, _)| *t).max().unwrap_or(0);
                Self::sleep_until_epoch_ms(deadline);
                let results: Vec<Value> = settled
                    .into_iter()
                    .map(|(_, outcome)| outcome.unwrap_or(Value::Unit))
                    .collect();
                Ok(self
                    .async_runtime
                    .promise_resolve(Value::List(std::sync::Arc::from(results))))
            }
            Expr::Race(list_expr) => {
                // Settle to whichever promise finishes first: the minimum
                // deadline wins (already-resolved promises settle at t=0).
                let promises = self.eval_promise_collection(list_expr, "race")?;
                if promises.is_empty() {
                    return Ok(Value::Promise {
                        state: crate::ast::PromiseState::Pending,
                        value: None,
                        error: None,
                        resolve_at_epoch_ms: None,
                        task_id: None,
                    });
                }

                let mut settled = Vec::with_capacity(promises.len());
                for value in promises {
                    settled.push(self.settle_info(value)?);
                }

                // The earliest to settle wins (ties: first in the list).
                let (deadline, outcome) = settled
                    .into_iter()
                    .min_by_key(|(t, _)| *t)
                    .expect("non-empty");
                Self::sleep_until_epoch_ms(deadline);
                match outcome {
                    Ok(v) => Ok(self.async_runtime.promise_resolve(v)),
                    Err(e) => Ok(self.async_runtime.promise_reject(e)),
                }
            }
            Expr::Spawn(expression) => {
                // Real background execution: the expression evaluates on its
                // own OS thread against a thread-safe clone of this
                // interpreter — the same worker pattern http.serve uses. The
                // clone snapshots current bindings, so `spawn` captures by
                // value, exactly like closures do. `await` joins the thread
                // (memoized, so a cloned promise can be awaited repeatedly).
                let mut worker = self.thread_safe_clone();
                let expr = expression.as_ref().clone();
                let task_id = spawn_registry::next_id();
                let handle = std::thread::Builder::new()
                    .name(format!("olang-spawn-{}", task_id))
                    .stack_size(64 * 1024 * 1024)
                    .spawn(move || worker.eval_expr(&expr).map_err(|e| e.to_string()))
                    .map_err(|e| InterpreterError::RuntimeError {
                        message: format!("spawn: could not start thread: {}", e),
                    })?;
                spawn_registry::register(task_id, handle);
                Ok(Value::Promise {
                    state: crate::ast::PromiseState::Pending,
                    value: None,
                    error: None,
                    resolve_at_epoch_ms: None,
                    task_id: Some(task_id),
                })
            }

            // Test assertions
            Expr::AssertEq {
                actual,
                expected,
                message,
            } => {
                let actual_val = self.eval_expr(actual)?;
                let expected_val = self.eval_expr(expected)?;
                if actual_val != expected_val {
                    let msg = message.clone().unwrap_or_else(|| {
                        format!("Assertion failed: {:?} != {:?}", actual_val, expected_val)
                    });
                    return Err(InterpreterError::RuntimeError { message: msg });
                }
                Ok(Value::Unit)
            }
            Expr::AssertNe {
                actual,
                expected,
                message,
            } => {
                let actual_val = self.eval_expr(actual)?;
                let expected_val = self.eval_expr(expected)?;
                if actual_val == expected_val {
                    let msg = message.clone().unwrap_or_else(|| {
                        format!("Assertion failed: {:?} == {:?}", actual_val, expected_val)
                    });
                    return Err(InterpreterError::RuntimeError { message: msg });
                }
                Ok(Value::Unit)
            }
            Expr::Assert { condition, message } => {
                let condition_val = self.eval_expr(condition)?;
                match condition_val {
                    Value::Boolean(true) => Ok(Value::Unit),
                    Value::Boolean(false) => {
                        let msg = message
                            .clone()
                            .unwrap_or_else(|| "Assertion failed: condition is false".to_string());
                        Err(InterpreterError::RuntimeError { message: msg })
                    }
                    _ => {
                        let msg = message.clone().unwrap_or_else(|| {
                            format!(
                                "Assertion failed: condition is not boolean: {:?}",
                                condition_val
                            )
                        });
                        Err(InterpreterError::RuntimeError { message: msg })
                    }
                }
            }
            Expr::AssertTrue {
                expression,
                message,
            } => {
                let val = self.eval_expr(expression)?;
                match val {
                    Value::Boolean(true) => Ok(Value::Unit),
                    _ => {
                        let msg = message.clone().unwrap_or_else(|| {
                            format!("Assertion failed: expected true, got {:?}", val)
                        });
                        Err(InterpreterError::RuntimeError { message: msg })
                    }
                }
            }
            Expr::AssertFalse {
                expression,
                message,
            } => {
                let val = self.eval_expr(expression)?;
                match val {
                    Value::Boolean(false) => Ok(Value::Unit),
                    _ => {
                        let msg = message.clone().unwrap_or_else(|| {
                            format!("Assertion failed: expected false, got {:?}", val)
                        });
                        Err(InterpreterError::RuntimeError { message: msg })
                    }
                }
            }
        }
    }

    fn eval_async_function_decl(
        &mut self,
        async_func_decl: crate::ast::AsyncFunctionDecl,
    ) -> Result<Value, InterpreterError> {
        // For now, treat async functions like regular functions
        // In full implementation, would mark as async
        // Convert ImHashMap to regular HashMap for closure storage
        // O(1): the environment's flat map is persistent, adopt it directly
        let closure = self.environment.flat_snapshot();
        let function = Function {
            name: Some(async_func_decl.name.clone()),
            parameters: async_func_decl.parameters,
            body: Arc::new(async_func_decl.body),
            closure: Arc::new(closure),
            param_bounds: Vec::new(),
        };

        let function_value = Value::Function(function);

        // Define the function in the current environment so it can be called recursively
        self.environment
            .define(async_func_decl.name, function_value.clone());

        Ok(function_value)
    }

    /// Enable promotion of hot functions to the OVM bytecode tier.
    ///
    /// Promotion never changes program behavior: anything the tier can't
    /// compile (closures, unsupported expressions, unresolved callees) stays
    /// interpreted. See `crate::ovm::tier` and the differential test suite.
    pub fn enable_bytecode_tier(&mut self, threshold: u32, verbose: bool) {
        self.bytecode_tier = Some(Box::new(
            crate::ovm::tier::BytecodeTier::new(threshold).with_verbose(verbose),
        ));
    }

    pub fn bytecode_tier_stats(&self) -> Option<crate::ovm::tier::TierStats> {
        self.bytecode_tier.as_ref().map(|t| t.stats())
    }

    /// Seed the declaration-level state a *bridge* interpreter needs to run
    /// user code faithfully. The bytecode VM bridges builtins it cannot run
    /// natively (e.g. `fold`) back to a throwaway interpreter; when such a
    /// builtin invokes a user lambda that dispatches a trait method
    /// (`x.tag()`) or constructs a declared struct, that interpreter must
    /// carry the same trait/struct/variant tables the program declared, or the
    /// call diverges from the tree-walk (a spurious "Field not found"). The VM
    /// mirrors these facts as declarations evaluate and installs them here.
    pub fn seed_bridge_state(
        &mut self,
        trait_impls: HashMap<(String, String), Function>,
        trait_defaults: HashMap<(String, String), Function>,
        type_traits: HashMap<String, Vec<String>>,
        struct_defs: HashMap<String, Vec<String>>,
        unit_variant_names: HashSet<String>,
    ) {
        self.trait_impls = trait_impls;
        self.trait_defaults = trait_defaults;
        self.type_traits = type_traits;
        self.struct_defs = struct_defs;
        self.unit_variant_names = unit_variant_names;
    }

    pub fn call_function(
        &mut self,
        callee: Value,
        arguments: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        // MEMORY PROTECTION: Check recursion depth to prevent exponential memory growth
        if self.call_depth >= self.max_call_depth {
            return Err(InterpreterError::RuntimeError {
                message: format!(
                    "Maximum call depth ({}) exceeded - possible infinite recursion or very deep call stack",
                    self.max_call_depth
                ),
            });
        }

        match callee {
            Value::Function(func) => {
                // Increment call depth for user functions
                self.call_depth += 1;

                // MEMORY CLEANUP: Reset memory tracking for each new function call
                self.reset_memory_tracking();
                // Count required parameters (those without default values)
                let required_params = func
                    .parameters
                    .iter()
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

                // Enforce trait bounds at the call boundary: an argument whose
                // type does not implement a bounded parameter's trait fails
                // here with a clear message, not deep inside the body.
                if !func.param_bounds.is_empty() {
                    self.check_param_bounds(&func, &arguments)?;
                }

                // Hot-function promotion: run on the bytecode tier when the
                // function is eligible, otherwise fall through to the AST walk
                if self.bytecode_tier.is_some() && arguments.len() == func.parameters.len() {
                    let mut tier = self.bytecode_tier.take();
                    let outcome = tier
                        .as_mut()
                        .map(|t| t.try_call(&func, &arguments))
                        .unwrap_or(crate::ovm::tier::TierOutcome::Fallback);
                    self.bytecode_tier = tier;

                    if let crate::ovm::tier::TierOutcome::Ran(result) = outcome {
                        self.call_depth -= 1;
                        return result
                            .map_err(|message| InterpreterError::RuntimeError { message });
                    }
                }

                // Create new environment with current environment as parent
                let mut new_env = Environment::with_parent(self.environment.clone());

                // Adopt the closure as the environment's flat map in O(1) —
                // the persistent map is shared, not copied. This was a loop
                // defining every closure entry (the whole prelude, ~200
                // entries) on every single call.
                if !func.closure.is_empty() {
                    new_env.variables = func.closure.clone();
                }

                // If this is a named function, add it to its own scope for recursion
                if let Some(name) = &func.name {
                    new_env.define_local(name.clone(), Value::Function(func.clone()));
                }

                // Parameters are defined over the shared map; copy-on-write
                // clones only the touched structure
                for (i, param) in func.parameters.iter().enumerate() {
                    let value = if i < arguments.len() {
                        arguments[i].clone()
                    } else if let Some(default_expr) = &param.default_value {
                        self.eval_expr(default_expr)?
                    } else {
                        return Err(InterpreterError::RuntimeError {
                            message: format!("Missing argument for parameter {}", param.name),
                        });
                    };

                    new_env.define_local(param.name.clone(), value);
                }

                // MEMORY OPTIMIZED: Use scoped evaluation instead of environment replacement
                let result = match self.eval_expr_with_env(&func.body, new_env) {
                    // `?` hit an Err inside this body: the function returns
                    // that Err to its caller — the early-return semantics.
                    Err(InterpreterError::ErrPropagation(err)) => Ok(err),
                    // `return v` inside this body: the function's value is v.
                    Err(InterpreterError::ReturnSignal(v)) => Ok(v),
                    other => other,
                };

                // Decrement call depth when function completes
                self.call_depth -= 1;

                // AGGRESSIVE MEMORY MANAGEMENT: Cleanup after function calls
                if self.memory_allocations > 5000 {
                    self.force_memory_cleanup();
                }

                result
            }
            Value::Builtin(builtin) => {
                let name = builtin.name.clone();
                let builtin_functions = self.builtin_functions.clone();
                BuiltinFunctions::call(&builtin_functions, &name, arguments, self)
            }
            // Applying a tuple-variant constructor builds the enum value
            Value::EnumConstructor {
                type_name,
                variant_name,
                arity,
            } => {
                // call_depth is only incremented in the Function arm, so
                // there is nothing to unwind here
                if arguments.len() != arity {
                    return Err(InterpreterError::ArityMismatch {
                        expected: arity,
                        got: arguments.len(),
                    });
                }
                Ok(Value::Enum {
                    type_name,
                    variant_name,
                    variant_data: crate::ast::EnumVariantData::Tuple(arguments),
                })
            }
            _ => Err(InterpreterError::TypeError {
                message: "Cannot call non-function value".to_string(),
            }),
        }
    }

    /// Feature 9: Get the intuitive error formatter
    pub fn get_error_formatter(&self) -> &IntuitiveErrorFormatter {
        &self.error_formatter
    }

    /// Feature 9: Format an interpreter error with enhanced context and suggestions
    pub fn format_error(&self, error: &InterpreterError) -> String {
        self.error_formatter.format_error(error)
    }

    /// Create a thread-safe clone for parallel operations
    pub fn thread_safe_clone(&self) -> Self {
        Self {
            environment: self.environment.clone(),
            builtin_functions: self.builtin_functions.clone(),
            type_checker: self.type_checker.clone(),
            async_runtime: AsyncRuntime::new(),
            safepoint_manager: self.safepoint_manager.clone(),
            module_debug_config: self.module_debug_config.clone(),

            // Enhanced module system
            module_cache: self.module_cache.clone(),
            dependency_tracker: self.dependency_tracker.clone(),
            current_module_path: self.current_module_path.clone(), // For tracking current module during loading
            module_loading_stack: Vec::new(), // Feature 7: Each thread gets its own loading stack

            // Feature 8: Smart caching system
            smart_cache_config: self.smart_cache_config.clone(),
            cache_statistics: self.cache_statistics.clone(),

            // Feature 9: Intuitive error messages
            error_formatter: self.error_formatter.clone(),

            // MEMORY PROTECTION: Initialize fresh recursion tracking for each thread
            call_depth: 0,
            max_call_depth: self.max_call_depth,

            // MEMORY MONITORING: Initialize fresh memory tracking for each thread
            memory_allocations: 0,
            max_memory_allocations: self.max_memory_allocations,

            // AGGRESSIVE MEMORY MANAGEMENT: Initialize fresh tracking for each thread
            large_allocation_count: 0,
            last_cleanup_operation: 0,

            // Each thread profiles independently; the VM is not shared, so
            // the clone gets a fresh, quiet tier with the same promotion
            // policy, and the declaration knowledge the parent accumulated
            // (functions, struct shapes, traits, variants) is replayed into
            // it — the worker never re-evaluates the declarations, so
            // without the replay nothing could promote. (This used to be
            // None — worker threads ran the pure tree-walker and silently
            // lost the tier's speed.)
            bytecode_tier: self.bytecode_tier.as_ref().map(|t| {
                let mut tier = crate::ovm::tier::BytecodeTier::new(t.threshold());
                for (name, fields) in &self.struct_defs {
                    tier.note_struct(name.clone(), fields.clone());
                }
                for name in &self.unit_variant_names {
                    tier.note_unit_variant(name.clone());
                }
                for ((type_name, method), func) in &self.trait_impls {
                    tier.note_function(method.clone(), func.clone());
                    tier.note_trait_impl(type_name.clone(), method.clone(), func.clone());
                }
                for ((trait_name, method), func) in &self.trait_defaults {
                    tier.note_trait_default(trait_name.clone(), method.clone(), func.clone());
                }
                for (type_name, traits) in &self.type_traits {
                    for trait_name in traits {
                        tier.note_type_trait(type_name.clone(), trait_name.clone());
                    }
                }
                for (name, value) in self.environment.get_all_variables() {
                    if let Value::Function(func) = value {
                        tier.note_function(name, func);
                    }
                }
                Box::new(tier)
            }),
            trait_impls: self.trait_impls.clone(),
            trait_defaults: self.trait_defaults.clone(),
            type_traits: self.type_traits.clone(),
            unit_variant_names: self.unit_variant_names.clone(),
            test_mode: false,
            test_results: Vec::new(),
            struct_defs: self.struct_defs.clone(),
            dependency_map: self.dependency_map.clone(),
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

    /// Optimized function call that reuses function references where possible
    /// This reduces cloning for repeated calls with the same function
    pub fn call_function_optimized(
        &mut self,
        function: &Value,
        args: Vec<Value>,
    ) -> Result<Value, InterpreterError> {
        // Clone only when necessary
        match function {
            Value::Function(_) => {
                // For user functions, we still need to clone for now
                // TODO: Implement reference-based calling
                self.call_function(function.clone(), args)
            }
            Value::Builtin(builtin) => {
                // For builtin functions, we can optimize
                let name = builtin.name.clone();
                let builtin_functions = self.builtin_functions.clone();
                BuiltinFunctions::call(&builtin_functions, &name, args, self)
            }
            _ => Err(InterpreterError::TypeError {
                message: "Cannot call non-function value".to_string(),
            }),
        }
    }

    /// MEMORY OPTIMIZED: Evaluate expression with scoped variables instead of environment replacement
    /// This completely avoids expensive environment moving operations
    fn eval_expr_with_env(
        &mut self,
        expr: &Expr,
        temp_env: Environment,
    ) -> Result<Value, InterpreterError> {
        // Swap the environment in and out. temp_env's parent already chains
        // to the caller's environment, so name resolution is identical to the
        // previous overlay approach (locals -> closure -> caller chain) —
        // but without iterating every closure entry twice per call, which
        // was a full lookup + clone + define + restore of ~200 prelude
        // entries on every single function call.
        let saved = std::mem::replace(&mut self.environment, temp_env);
        let result = self.eval_expr(expr);
        self.environment = saved;
        result
    }

    /// MEMORY MONITORING: Track memory allocations to prevent corruption
    fn track_allocation(&mut self, size: usize) -> Result<(), InterpreterError> {
        self.memory_allocations += size;

        // AGGRESSIVE MEMORY MANAGEMENT: Track large allocations
        if size > 100 {
            self.large_allocation_count += 1;

            // Force cleanup after every 5 large allocations
            if self.large_allocation_count - self.last_cleanup_operation >= 5 {
                self.force_memory_cleanup();
                self.last_cleanup_operation = self.large_allocation_count;
            }
        }

        if self.memory_allocations > self.max_memory_allocations {
            return Err(InterpreterError::RuntimeError {
                message: format!(
                    "Memory allocation limit ({}) exceeded. Current allocations: {}. This prevents memory corruption.",
                    self.max_memory_allocations, self.memory_allocations
                ),
            });
        }
        Ok(())
    }

    /// MEMORY CLEANUP: Reset memory tracking between function calls to prevent accumulation
    fn reset_memory_tracking(&mut self) {
        self.memory_allocations = 0;
        // Don't reset call_depth - it needs to be preserved for proper decrementing
    }

    /// AGGRESSIVE MEMORY MANAGEMENT: Force garbage collection and cleanup
    pub fn force_memory_cleanup(&mut self) {
        // Clear module cache to free large amounts of memory
        self.clear_module_cache();

        // Reset all memory tracking
        self.memory_allocations = 0;

        // Don't clear user environment - it breaks variable scoping
        // self.clear_user_environment();

        // Perform intelligent cache cleanup to free memory
        let _ = self.perform_intelligent_cache_cleanup();
    }

    fn eval_match(&mut self, value: Value, arms: &[MatchArm]) -> Result<Value, InterpreterError> {
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

                    let guard_result = self.eval_expr(guard_expr);

                    // Restore parent environment
                    if let Some(parent) = self.environment.parent.take() {
                        self.environment =
                            Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                    }

                    // Surface guard errors instead of silently treating them
                    // as "no match" (which hid typos like undefined variables)
                    self.to_boolean(&guard_result?)?
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
                    let result = self.eval_expr(&arm.expression);
                    if let Some(parent) = self.environment.parent.take() {
                        self.environment =
                            Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                    }
                    return result;
                }
                // Pattern matched but guard failed, continue to next arm
            }
        }
        Err(InterpreterError::PatternMatchFailed)
    }

    fn eval_type_decl(
        &mut self,
        type_decl: crate::ast::TypeDecl,
    ) -> Result<Value, InterpreterError> {
        use crate::ast::{EnumVariantData, TypeDefinition};

        // Enum declarations bind each variant into scope so it can be
        // constructed. Unit variants become `Enum` values directly; tuple
        // variants become constructor callables (`Circle(radius)`).
        //
        // Type parameters (`enum Option<T>`) are erased at runtime — the
        // language is dynamically typed, so a generic variant constructs for
        // any argument type. The static side is the type checker's concern.
        if let TypeDefinition::Enum { variants } = &type_decl.definition {
            for variant in variants {
                let value = match &variant.data {
                    None => {
                        // Remember unit-variant names so a pattern can tell a
                        // variant test from a fresh binding by name.
                        self.unit_variant_names.insert(variant.name.clone());
                        if let Some(tier) = self.bytecode_tier.as_mut() {
                            tier.note_unit_variant(variant.name.clone());
                        }
                        Value::Enum {
                            type_name: type_decl.name.clone(),
                            variant_name: variant.name.clone(),
                            variant_data: EnumVariantData::Unit,
                        }
                    }
                    Some(fields) => Value::EnumConstructor {
                        type_name: type_decl.name.clone(),
                        variant_name: variant.name.clone(),
                        arity: fields.len(),
                    },
                };
                self.environment.define(variant.name.clone(), value);
            }
        }

        // Struct declarations register their shape: construction validates
        // the field-name set against it (values stay dynamic).
        if let TypeDefinition::Struct { fields } = &type_decl.definition {
            let field_names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
            // The bytecode tier validates literals at compile time against
            // the same shape
            if let Some(tier) = self.bytecode_tier.as_mut() {
                tier.note_struct(type_decl.name.clone(), field_names.clone());
            }
            self.struct_defs.insert(type_decl.name.clone(), field_names);
        }

        Ok(Value::Unit)
    }

    fn eval_struct_literal(
        &mut self,
        struct_literal: &crate::ast::StructLiteral,
    ) -> Result<Value, InterpreterError> {
        // A struct literal names a declared type, and construction validates
        // the field-name set against the declaration — missing or surprise
        // fields are errors, and an undeclared name is an error (use an
        // anonymous `{ ... }` object for free-form records). Field VALUES are
        // not type-checked: olang is dynamically typed; declarations fix
        // shape, not types.
        let declared = match self.struct_defs.get(&struct_literal.type_name) {
            Some(fields) => fields.clone(),
            None => {
                return Err(InterpreterError::TypeError {
                    message: format!(
                        "unknown struct type '{}' — declare it with `type {} = struct {{ ... }}`, \
                         or use an anonymous object `{{ ... }}` for a free-form record",
                        struct_literal.type_name, struct_literal.type_name
                    ),
                });
            }
        };

        let given: Vec<&String> = struct_literal.fields.iter().map(|f| &f.name).collect();
        for required in &declared {
            if !given.contains(&required) {
                return Err(InterpreterError::TypeError {
                    message: format!(
                        "struct '{}' is missing field '{}' (declared fields: {})",
                        struct_literal.type_name,
                        required,
                        declared.join(", ")
                    ),
                });
            }
        }
        for name in &given {
            if !declared.contains(name) {
                return Err(InterpreterError::TypeError {
                    message: format!(
                        "struct '{}' has no field '{}' (declared fields: {})",
                        struct_literal.type_name,
                        name,
                        declared.join(", ")
                    ),
                });
            }
        }

        let mut fields = std::collections::HashMap::new();
        for field_value in &struct_literal.fields {
            let value = self.eval_expr(&field_value.value)?;
            fields.insert(field_value.name.clone(), value);
        }

        Ok(Value::Struct {
            type_name: struct_literal.type_name.clone(),
            fields,
        })
    }

    fn eval_anonymous_object(
        &mut self,
        field_values: &[crate::ast::FieldValue],
    ) -> Result<Value, InterpreterError> {
        let mut fields = std::collections::HashMap::new();

        for field_value in field_values {
            let value = self.eval_expr(&field_value.value)?;
            fields.insert(field_value.name.clone(), value);
        }

        // Use a generic type name for anonymous objects
        Ok(Value::Struct {
            type_name: "Object".to_string(),
            fields,
        })
    }

    fn eval_map_literal(
        &mut self,
        entries: &[crate::ast::MapEntry],
    ) -> Result<Value, InterpreterError> {
        let mut map = std::collections::HashMap::new();

        for entry in entries {
            let key = self.eval_expr(&entry.key)?;
            let value = self.eval_expr(&entry.value)?;

            // Convert key to string (maps in Olang use string keys)
            let key_str = match key {
                Value::String(s) => s.as_ref().clone(),
                Value::Integer(i) => i.to_string(),
                Value::Float(f) => crate::ast::format_float(f),
                Value::Boolean(b) => b.to_string(),
                _ => {
                    return Err(InterpreterError::TypeError {
                        message: "Map keys must be strings, integers, floats, or booleans"
                            .to_string(),
                    });
                }
            };

            map.insert(key_str, value);
        }

        Ok(Value::Map(std::sync::Arc::new(map)))
    }

    fn eval_field_access(
        &mut self,
        object: &crate::ast::Expr,
        field: &str,
    ) -> Result<Value, InterpreterError> {
        let object_value = self.eval_expr(object)?;

        match object_value {
            Value::Struct { fields, type_name } => {
                if type_name == "Module" {
                    // Handle module function access (e.g., fs.read_file)
                    fields
                        .get(field)
                        .cloned()
                        .ok_or_else(|| InterpreterError::TypeError {
                            message: format!("Function '{}' not found in module", field),
                        })
                } else {
                    // Handle regular struct field access
                    fields
                        .get(field)
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
    /// If `name` is bound to a module (native stdlib, embedded, or a package),
    /// return its member function names, sorted. Used by `:help <module>` so
    /// imported modules are discoverable. Returns None for non-module bindings.
    pub fn module_members(&self, name: &str) -> Option<Vec<String>> {
        match self.environment.get(name)? {
            Value::Struct { type_name, fields } if type_name == "Module" => {
                let mut names: Vec<String> = fields.keys().cloned().collect();
                names.sort();
                Some(names)
            }
            _ => None,
        }
    }

    pub fn get_user_variables(&self) -> HashMap<String, &Value> {
        let mut user_vars = HashMap::new();
        let builtin_names: std::collections::HashSet<String> = self
            .builtin_functions
            .get_functions()
            .keys()
            .cloned()
            .collect();

        // Dereference Arc to iterate over ImHashMap
        for (name, value) in self.environment.variables.iter() {
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

        // Use Arc::make_mut for copy-on-write mutation of the ImHashMap
        let vars = Arc::make_mut(&mut self.environment.variables);
        vars.retain(|name, _| {
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
    fn collect_all_accessible_variables(&self) -> ImHashMap<String, Value> {
        // Union the chain innermost-first: im's union prefers entries from
        // self on collision, so inner scopes shadow outer ones. Structural
        // sharing makes this near-O(1) for the common shallow chains,
        // versus copying every entry of every scope.
        // NOTE: im::HashMap::union is unusable here — its collision bias
        // depends on which map is LARGER (it swaps sides internally as a
        // size optimization), so "inner scope wins" silently became
        // "bigger scope wins" and a captured variable could resolve to an
        // ancestor frame's stale value. Insert explicitly instead: existing
        // entries always win, so inner scopes shadow outer ones.
        let mut all_variables = self.environment.flat_snapshot();
        let mut current_env = &self.environment;
        while let Some(parent) = current_env.parent.as_ref() {
            // Within a scope, call-frame locals shadow its flat map
            for (name, value) in parent.locals.iter().rev() {
                if !all_variables.contains_key(name) {
                    all_variables.insert(name.clone(), value.clone());
                }
            }
            for (name, value) in parent.variables.iter() {
                if !all_variables.contains_key(name) {
                    all_variables.insert(name.clone(), value.clone());
                }
            }
            current_env = parent;
        }

        all_variables
    }

    fn eval_for_loop(
        &mut self,
        variable: &str,
        iterable: &Expr,
        body: &Expr,
    ) -> Result<Value, InterpreterError> {
        let iterable_value = self.eval_expr(iterable)?;

        match iterable_value {
            Value::List(items) => {
                let parent_env = std::mem::take(&mut self.environment);
                self.environment.parent = Some(Arc::new(parent_env));
                self.environment.is_frame = true;

                let result = self.run_loop_body(body, items.iter().cloned(), Some(variable));

                // Restore parent environment (also on error, so a failing body
                // doesn't leak the loop scope into subsequent statements)
                if let Some(parent) = self.environment.parent.take() {
                    self.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                }

                result
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                let parent_env = std::mem::take(&mut self.environment);
                self.environment.parent = Some(Arc::new(parent_env));
                self.environment.is_frame = true;

                let items = RangeIter {
                    next: start,
                    end,
                    inclusive,
                    done: if inclusive { start > end } else { start >= end },
                };
                let result = self.run_loop_body(body, items.map(Value::Integer), Some(variable));

                // Restore parent environment
                if let Some(parent) = self.environment.parent.take() {
                    self.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                }

                result
            }
            // Strings iterate by character, each a 1-character string —
            // consistent with 'a' literals being strings.
            Value::String(s) => {
                let parent_env = std::mem::take(&mut self.environment);
                self.environment.parent = Some(Arc::new(parent_env));
                self.environment.is_frame = true;

                let chars: Vec<Value> = s
                    .chars()
                    .map(|c| Value::String(Arc::new(c.to_string())))
                    .collect();
                let result = self.run_loop_body(body, chars.into_iter(), Some(variable));

                if let Some(parent) = self.environment.parent.take() {
                    self.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
                }

                result
            }
            _ => Err(InterpreterError::TypeError {
                message: format!("Cannot iterate over {:?}", iterable_value),
            }),
        }
    }

    /// Run a loop body over an iterator of items, honoring break/continue.
    /// `par for`: fan the iterations across worker threads. Spawn-style
    /// snapshot semantics — each worker runs against a clone of the
    /// interpreter, so mutations to enclosing state are not visible to
    /// the caller; effects (println, fs) are real. Implicit barrier at
    /// the end; the loop evaluates to Unit. If several iterations fail,
    /// the error reported is the one the sequential loop would have hit
    /// first. `break` and `return` cannot cross the parallel boundary
    /// and are errors; `continue` works within an iteration.
    fn eval_par_for_loop(
        &mut self,
        variable: &str,
        iterable: &Expr,
        body: &Expr,
    ) -> Result<Value, InterpreterError> {
        let iterable_value = self.eval_expr(iterable)?;
        let items: Vec<Value> = match iterable_value {
            Value::List(items) => items.to_vec(),
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                let stop = if inclusive {
                    end.saturating_add(1)
                } else {
                    end
                };
                (start..stop).map(Value::Integer).collect()
            }
            Value::String(s) => s
                .chars()
                .map(|c| Value::String(Arc::new(c.to_string())))
                .collect(),
            other => {
                return Err(InterpreterError::TypeError {
                    message: format!(
                        "par for: cannot iterate over {} (lists, ranges, and strings)",
                        other.type_name()
                    ),
                });
            }
        };
        if items.is_empty() {
            return Ok(Value::Unit);
        }

        let run_one = |worker: &mut Interpreter, item: Value| -> Result<(), InterpreterError> {
            let parent_env = std::mem::take(&mut worker.environment);
            worker.environment.parent = Some(Arc::new(parent_env));
            worker.environment.is_frame = true;
            worker.environment.define(variable.to_string(), item);
            let result = match worker.eval_expr(body) {
                Ok(_) | Err(InterpreterError::ContinueSignal) => Ok(()),
                Err(InterpreterError::BreakSignal(_)) => Err(InterpreterError::RuntimeError {
                    message: "break cannot cross a par for boundary".to_string(),
                }),
                Err(InterpreterError::ReturnSignal(_)) => Err(InterpreterError::RuntimeError {
                    message: "return cannot cross a par for boundary".to_string(),
                }),
                Err(e) => Err(e),
            };
            if let Some(parent) = worker.environment.parent.take() {
                worker.environment = Arc::try_unwrap(parent).unwrap_or_else(|arc| (*arc).clone());
            }
            result
        };

        #[cfg(feature = "native")]
        {
            let workers = crate::parallel::get_config()
                .max_threads
                .clamp(1, items.len());
            if workers > 1 {
                let chunk_size = items.len().div_ceil(workers);
                let joined: Vec<Result<(), (usize, InterpreterError)>> =
                    std::thread::scope(|scope| {
                        let handles: Vec<_> = items
                            .chunks(chunk_size)
                            .enumerate()
                            .map(|(chunk_idx, chunk)| {
                                let mut worker = self.thread_safe_clone();
                                scope.spawn(move || {
                                    for (i, item) in chunk.iter().enumerate() {
                                        run_one(&mut worker, item.clone())
                                            .map_err(|e| (chunk_idx * chunk_size + i, e))?;
                                    }
                                    Ok(())
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
                                            message: "par for worker thread panicked".to_string(),
                                        },
                                    ))
                                })
                            })
                            .collect()
                    });
                let mut first_err: Option<(usize, InterpreterError)> = None;
                for r in joined {
                    if let Err((idx, e)) = r
                        && first_err.as_ref().map(|(fi, _)| idx < *fi).unwrap_or(true)
                    {
                        first_err = Some((idx, e));
                    }
                }
                if let Some((_, e)) = first_err {
                    return Err(e);
                }
                return Ok(Value::Unit);
            }
        }

        // Single worker (or the playground): sequential, same semantics.
        for item in items {
            run_one(self, item)?;
        }
        Ok(Value::Unit)
    }

    fn run_loop_body(
        &mut self,
        body: &Expr,
        items: impl Iterator<Item = Value>,
        variable: Option<&str>,
    ) -> Result<Value, InterpreterError> {
        let mut last_value = Value::Unit;
        for item in items {
            // Safepoint poll for GC coordination during iteration
            self.safepoint_poll()?;

            if let Some(name) = variable {
                self.environment.define(name.to_string(), item);
            }
            match self.eval_expr(body) {
                Ok(v) => last_value = v,
                // `break value` makes the loop evaluate to that value; a
                // bare break keeps the last body value (Unit carries both).
                Err(InterpreterError::BreakSignal(v)) => {
                    if !matches!(v, Value::Unit) {
                        last_value = v;
                    }
                    break;
                }
                Err(InterpreterError::ContinueSignal) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(last_value)
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

            let condition_value = self.eval_expr(condition)?;
            let condition_bool = self.to_boolean(&condition_value)?;

            if !condition_bool {
                break;
            }

            match self.eval_expr(body) {
                Ok(v) => last_value = v,
                Err(InterpreterError::BreakSignal(v)) => {
                    if !matches!(v, Value::Unit) {
                        last_value = v;
                    }
                    break;
                }
                Err(InterpreterError::ContinueSignal) => continue,
                Err(e) => return Err(e),
            }
        }

        Ok(last_value)
    }

    fn eval_loop(&mut self, body: &Expr) -> Result<Value, InterpreterError> {
        let mut last_value = Value::Unit;
        loop {
            // Safepoint poll for GC coordination at start of each iteration
            self.safepoint_poll()?;

            match self.eval_expr(body) {
                Ok(v) => last_value = v,
                Err(InterpreterError::BreakSignal(v)) => {
                    return Ok(if matches!(v, Value::Unit) {
                        last_value
                    } else {
                        v
                    });
                }
                Err(InterpreterError::ContinueSignal) => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Resolve arguments (both positional and named) for function calls
    fn resolve_arguments(
        &mut self,
        callee: &Value,
        arguments: &[Argument],
    ) -> Result<Vec<Value>, InterpreterError> {
        // Get function parameter information if available
        let parameters = match callee {
            Value::Function(func) => Some(&func.parameters),
            _ => None, // For builtin functions and other callables, use positional-only
        };

        let mut resolved_args = Vec::new();
        // Vec keeps source order — a HashMap would append named args to
        // builtins in nondeterministic order
        let mut named_args: Vec<(String, Value)> = Vec::new();
        let mut positional_count = 0;

        // First pass: collect positional and named arguments
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    if !named_args.is_empty() {
                        return Err(InterpreterError::RuntimeError {
                            message: "Positional arguments cannot come after named arguments"
                                .to_string(),
                        });
                    }
                    resolved_args.push(self.eval_expr(expr)?);
                    positional_count += 1;
                }
                Argument::Named { name, value } => {
                    let evaluated_value = self.eval_expr(value)?;
                    if named_args.iter().any(|(n, _)| n == name) {
                        return Err(InterpreterError::RuntimeError {
                            message: format!("Duplicate named argument: {}", name),
                        });
                    }
                    named_args.push((name.clone(), evaluated_value));
                }
            }
        }

        // Second pass: resolve named arguments to correct positions (if we have parameter info)
        if let Some(params) = parameters {
            // Check for conflicts between positional and named arguments
            for (arg_name, _) in &named_args {
                if let Some(param_index) = params.iter().position(|p| &p.name == arg_name)
                    && param_index < positional_count
                {
                    return Err(InterpreterError::RuntimeError {
                        message: format!(
                            "Argument '{}' specified both positionally and by name",
                            arg_name
                        ),
                    });
                }
            }

            // Extend resolved_args to cover all parameters, filling with named args or defaults
            while resolved_args.len() < params.len() {
                let param_index = resolved_args.len();
                let param = &params[param_index];

                if let Some(pos) = named_args.iter().position(|(n, _)| n == &param.name) {
                    // Use named argument value
                    resolved_args.push(named_args.remove(pos).1);
                } else if let Some(default_expr) = &param.default_value {
                    // Use default value
                    let default_value = self.eval_expr(default_expr)?;
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
                let unrecognized: Vec<String> = named_args.iter().map(|(n, _)| n.clone()).collect();
                return Err(InterpreterError::RuntimeError {
                    message: format!(
                        "Unrecognized named argument(s): {}",
                        unrecognized.join(", ")
                    ),
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

    fn eval_share_decl(&mut self, share: ShareDecl) -> Result<Value, InterpreterError> {
        match share {
            ShareDecl::Function(func) => self.eval_function_decl(func),
            ShareDecl::Let(letd) => self.eval_let_decl(&letd),
            ShareDecl::Type(typed) => self.eval_type_decl(typed),
            ShareDecl::Use(use_decl) => self.eval_transitive_share(use_decl),
            // Traits and impls register in global registries, not the
            // environment, so `share` is cosmetic — declaring them already
            // makes them available to any module that loads this one.
            ShareDecl::Trait(trait_decl) => self.eval_trait_decl(trait_decl),
            ShareDecl::Impl(impl_decl) => self.eval_impl_decl(impl_decl),
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

    /// Install the package dependency map (name -> source directory), so
    /// `use` paths rooted at a dependency name resolve inside it.
    /// Evaluate the argument of `Promise.all`/`race` — any expression that
    /// yields a list — into a vector of promise values. Accepts a literal
    /// list or a variable holding one.
    fn eval_promise_collection(
        &mut self,
        list_expr: &Expr,
        which: &str,
    ) -> Result<Vec<Value>, InterpreterError> {
        match self.eval_expr(list_expr)? {
            Value::List(items) => Ok(items.iter().cloned().collect()),
            other => Err(InterpreterError::TypeError {
                message: format!(
                    "Promise.{} expects a list of promises, got {}",
                    which,
                    other.type_name()
                ),
            }),
        }
    }

    /// Normalize a promise value into `(settle_epoch_ms, Ok(value) | Err(err))`
    /// so `await`, `Promise.all`, and `Promise.race` share one resolution
    /// model. Already-resolved/rejected promises settle at t=0; a delayed
    /// promise settles at its deadline carrying its value. A pending promise
    /// with no deadline can never settle in a synchronous interpreter and is
    /// an error. Non-promise values are treated as resolved.
    fn settle_info(&self, value: Value) -> Result<(u64, Result<Value, Value>), InterpreterError> {
        match value {
            // A spawn-backed promise settles by joining its thread. The
            // result is memoized in the registry, so awaiting a clone of the
            // same promise again returns the same value.
            Value::Promise {
                state: crate::ast::PromiseState::Pending,
                task_id: Some(id),
                ..
            } => match spawn_registry::join(id) {
                Some(Ok(v)) => Ok((0, Ok(v))),
                Some(Err(msg)) => Ok((
                    0,
                    Err(Value::String(std::sync::Arc::new(format!(
                        "spawned task failed: {}",
                        msg
                    )))),
                )),
                None => Err(InterpreterError::RuntimeError {
                    message: format!("spawned task {} is unknown to this process", id),
                }),
            },
            Value::Promise {
                state: crate::ast::PromiseState::Resolved,
                value: Some(v),
                ..
            } => Ok((0, Ok(*v))),
            Value::Promise {
                state: crate::ast::PromiseState::Rejected,
                error: Some(e),
                ..
            } => Ok((0, Err(*e))),
            Value::Promise {
                state: crate::ast::PromiseState::Pending,
                value,
                resolve_at_epoch_ms: Some(deadline),
                ..
            } => Ok((deadline, Ok(value.map(|v| *v).unwrap_or(Value::Unit)))),
            Value::Promise {
                state: crate::ast::PromiseState::Pending,
                ..
            } => Err(InterpreterError::RuntimeError {
                message: "Cannot await pending promise (no deadline to resolve it)".to_string(),
            }),
            // A plain value is an already-resolved result.
            other => Ok((0, Ok(other))),
        }
    }

    /// Sleep until the given epoch-millisecond deadline (no-op if already
    /// past). Time already elapsed since the promise was created counts
    /// against the delay, like a real timer.
    fn sleep_until_epoch_ms(deadline: u64) {
        if deadline == 0 {
            return;
        }
        let now = crate::clock::system_now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(u64::MAX);
        if deadline > now {
            crate::clock::sleep_ms(deadline - now);
        }
    }

    fn eval_test_decl(&mut self, test_decl: TestDecl) -> Result<Value, InterpreterError> {
        if !self.test_mode {
            // Inline behavior (normal runs): the body executes in place and a
            // failing assertion aborts, like any other error.
            for statement in &test_decl.body {
                self.eval_statement(statement)?;
            }
            return Ok(Value::Unit);
        }

        // Runner behavior (`olang test`): record the outcome and keep going,
        // so one failing block doesn't hide the others.
        let mut error = None;
        for statement in &test_decl.body {
            if let Err(e) = self.eval_statement(statement) {
                error = Some(e.to_string());
                break;
            }
        }
        self.test_results.push(TestOutcome {
            name: test_decl.name,
            error,
        });
        Ok(Value::Unit)
    }

    /// Turn on test-runner mode: `test` blocks record outcomes (readable via
    /// `take_test_results`) and execution continues past failing blocks.
    pub fn enable_test_mode(&mut self) {
        self.test_mode = true;
    }

    /// The outcomes of every `test` block run so far, clearing the record.
    pub fn take_test_results(&mut self) -> Vec<TestOutcome> {
        std::mem::take(&mut self.test_results)
    }
}

/// The result of one `test "name" { ... }` block under `olang test`.
#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub name: String,
    /// `None` when the block passed; the failure message otherwise.
    pub error: Option<String>,
}

mod spawn_registry;

mod module_cache;
pub use module_cache::{
    CacheCleanupStats, CacheStatistics, ModuleCacheEntry, ModuleDependencyTracker, SmartCacheConfig,
};

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

        interpreter
            .cache_module(
                "test_module".to_string(),
                test_module.clone(),
                None,
                Vec::new(),
            )
            .unwrap();

        // Verify module is cached
        assert!(interpreter.is_module_cached("test_module"));
        let cached = interpreter
            .get_cached_module("test_module")
            .unwrap()
            .unwrap();
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
        assert!(
            tracker
                .dependencies
                .get("A")
                .unwrap()
                .contains(&"B".to_string())
        );
        assert!(
            tracker
                .dependencies
                .get("A")
                .unwrap()
                .contains(&"C".to_string())
        );

        // Test dependent queries
        assert_eq!(tracker.dependents.get("B").unwrap().len(), 1);
        assert!(
            tracker
                .dependents
                .get("B")
                .unwrap()
                .contains(&"A".to_string())
        );

        // Test circular dependency detection
        assert!(!tracker.check_circular_dependency("A", "D")); // A -> B -> D (no cycle)
        assert!(tracker.check_circular_dependency("D", "A")); // D -> A would create cycle
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

        interpreter
            .cache_module("module_a".to_string(), module_a, None, Vec::new())
            .unwrap();
        interpreter
            .cache_module("module_b".to_string(), module_b, None, Vec::new())
            .unwrap();

        // Set up dependency: B depends on A
        interpreter
            .dependency_tracker
            .add_dependency("module_b".to_string(), "module_a".to_string());

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
        interpreter
            .dependency_tracker
            .add_dependency("A".to_string(), "B".to_string());
        interpreter
            .dependency_tracker
            .add_dependency("B".to_string(), "C".to_string());
        interpreter
            .dependency_tracker
            .add_dependency("C".to_string(), "D".to_string());

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

        interpreter
            .cache_module("module1".to_string(), test_module.clone(), None, Vec::new())
            .unwrap();
        interpreter
            .cache_module("module2".to_string(), test_module, None, Vec::new())
            .unwrap();
        interpreter
            .dependency_tracker
            .add_dependency("module1".to_string(), "module2".to_string());

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
        interpreter
            .dependency_tracker
            .add_dependency("A".to_string(), "B".to_string());
        interpreter
            .dependency_tracker
            .add_dependency("B".to_string(), "C".to_string());

        // Test various potential circular dependencies
        assert!(!interpreter.would_create_circular_dependency("A", "D")); // No cycle
        assert!(!interpreter.would_create_circular_dependency("D", "E")); // No cycle
        assert!(interpreter.would_create_circular_dependency("C", "A")); // Would create cycle
        assert!(interpreter.would_create_circular_dependency("C", "B")); // Would create cycle
        assert!(interpreter.would_create_circular_dependency("B", "A")); // Would create cycle
    }
}

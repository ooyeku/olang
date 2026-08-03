//! OVM Integration Layer
//!
//! Provides seamless integration between the main interpreter with the Olang interpreter
//! and the Olang Virtual Machine (OVM) for enhanced performance.

use crate::ast::{Argument, FunctionDecl, Program, Value};
use crate::interpreter::{Interpreter, InterpreterError};
use crate::ovm::{FunctionId, OlangVirtualMachine, OvmConfig, OvmError, OvmValue};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::fs;
use sha2::{Sha256, Digest};

/// Enhanced interpreter that can use either traditional interpretation or OVM execution
pub struct OvmInterpreter {
    /// Traditional interpreter for compatibility
    classic_interpreter: Interpreter,

    /// OVM instance for high-performance execution
    ovm: Option<OlangVirtualMachine>,

    /// Configuration for OVM integration
    integration_config: IntegrationConfig,

    /// Function registry mapping names to OVM function IDs
    function_registry: HashMap<String, FunctionId>,

    /// Ahead-of-time function analysis results
    function_analysis: HashMap<String, FunctionAnalysis>,

    /// Compiled state cache for functions
    compiled_state: HashMap<String, CompiledState>,

    /// Performance statistics
    execution_stats: Arc<Mutex<ExecutionStats>>,

    /// Disk cache of known functions (keyed by hash) loaded from previous runs
    cached_functions: HashMap<String, FunctionDecl>,

    /// Whether the cache has been loaded this session
    cache_loaded: bool,
}

/// Configuration for OVM integration behavior
#[derive(Debug, Clone)]
pub struct IntegrationConfig {
    /// Whether to use OVM by default for new expressions
    pub use_ovm_by_default: bool,

    /// Threshold for switching to OVM (based on complexity)
    pub ovm_complexity_threshold: usize,

    /// Whether to enable automatic function compilation
    pub auto_compile_functions: bool,

    /// Whether to enable lazy evaluation through OVM
    pub enable_ovm_lazy_eval: bool,

    /// Fallback to classic interpreter on OVM errors
    pub fallback_on_error: bool,

    /// Whether to enable OVM builtin execution
    pub enable_ovm_builtins: bool,

    /// Enable persistence of compiled artifacts across sessions
    pub ovm_cache_enabled: bool,

    /// Enable parallel execution of independent expressions
    pub enable_parallel: bool,

    /// Maximum parallelism (threads) to use when parallel is enabled
    pub max_parallelism: Option<usize>,

    /// List of builtin functions that should use OVM
    pub ovm_preferred_builtins: Vec<String>,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            use_ovm_by_default: true,
            ovm_complexity_threshold: 1,
            auto_compile_functions: true,
            enable_ovm_lazy_eval: true,
            fallback_on_error: true,
            enable_ovm_builtins: true,
            ovm_cache_enabled: true,
            enable_parallel: true,
            max_parallelism: None,
            ovm_preferred_builtins: vec![
                // Simple mathematical builtins that can benefit from OVM
                "len".to_string(),
                "typeof".to_string(),
                "to_string".to_string(),
                "to_int".to_string(),
                "to_float".to_string(),
                "sum".to_string(),
                "average".to_string(),
                "min".to_string(),
                "max".to_string(),
                "clamp".to_string(),
                "reverse".to_string(),
                "sort".to_string(),
                "contains".to_string(),
                "starts_with".to_string(),
                "ends_with".to_string(),
                "flatten".to_string(),
                // Simple list operations
                "head".to_string(),
                "tail".to_string(),
                "cons".to_string(),
                // Pipeline operations like map and filter are removed to use classic interpreter
                "reduce".to_string(),
                "fold".to_string(),
                "take".to_string(),
                "skip".to_string(),
            ],
        }
    }
}

/// Statistics for tracking execution performance
#[derive(Debug, Default)]
pub struct ExecutionStats {
    pub classic_executions: u64,
    pub ovm_executions: u64,
    pub fallback_executions: u64,
    pub compilation_count: u64,
    pub average_classic_time_ms: f64,
    pub average_ovm_time_ms: f64,
}

/// Ahead-of-time function analysis result
#[derive(Debug, Clone, Default)]
pub struct FunctionAnalysis {
    pub eligible: bool,
    pub cost: usize,
    pub reason: Option<String>,
}

/// Compiled state of a function in the current interpreter session
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompiledState {
    Pending,
    NotEligible,
    Compiled(FunctionId),
}

impl Default for CompiledState {
    fn default() -> Self {
        CompiledState::Pending
    }
}

/// Detailed OVM status information
#[derive(Debug, Clone)]
pub struct OvmStatus {
    pub initialized: bool,
    pub running: bool,
    pub memory_usage: usize,
    pub uptime: std::time::Duration,
}

/// Simple parallel executor using the global rayon pool
struct ParallelExecutor;
impl ParallelExecutor {
    fn initialize_if_needed(max_threads: Option<usize>) {
        if let Some(n) = max_threads {
            let _ = crate::parallel::initialize_parallelization(Some(n));
        }
    }
}

impl OvmInterpreter {
    /// Create a new OVM-integrated interpreter
    pub fn new() -> Self {
        Self {
            classic_interpreter: Interpreter::new(),
            ovm: None,
            integration_config: IntegrationConfig::default(),
            function_registry: HashMap::new(),
            function_analysis: HashMap::new(),
            compiled_state: HashMap::new(),
            execution_stats: Arc::new(Mutex::new(ExecutionStats::default())),
            cached_functions: HashMap::new(),
            cache_loaded: false,
        }
    }

    /// Create with custom integration configuration
    pub fn with_config(integration_config: IntegrationConfig) -> Self {
        Self {
            classic_interpreter: Interpreter::new(),
            ovm: None,
            integration_config,
            function_registry: HashMap::new(),
            function_analysis: HashMap::new(),
            compiled_state: HashMap::new(),
            execution_stats: Arc::new(Mutex::new(ExecutionStats::default())),
            cached_functions: HashMap::new(),
            cache_loaded: false,
        }
    }

    /// Initialize the OVM with given configuration
    pub fn initialize_ovm(&mut self, ovm_config: OvmConfig) -> Result<(), IntegrationError> {
        // Stop any existing OVM first
        if let Some(ovm) = &mut self.ovm {
            let _ = ovm.stop();
        }
        self.ovm = None;

        let mut ovm =
            OlangVirtualMachine::new(ovm_config).map_err(IntegrationError::OvmInitError)?;

        ovm.start().map_err(IntegrationError::OvmInitError)?;

        self.ovm = Some(ovm);

        // Log cache directory for visibility
        if self.integration_config.ovm_cache_enabled {
            let cache_dir = Self::cache_dir_path();
            crate::log::get_logger().info(
                "ovm_cache",
                &format!("Cache enabled. Directory: {}", cache_dir.display()),
            );
        }

        // Load cached function metadata if enabled
        if self.integration_config.ovm_cache_enabled {
            let _ = self.load_cached_functions();
            // For safety, do not preload cached functions by default.
            // Preloading can be explicitly enabled via OLANG_OVM_PRELOAD=1
            let preload = std::env::var("OLANG_OVM_PRELOAD")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false);
            if preload {
                let _ = self.preload_cached_functions();
            } else {
                crate::log::get_logger().info(
                    "ovm_cache",
                    "Skipping preload of cached functions (set OLANG_OVM_PRELOAD=1 to enable)",
                );
            }
        }

        Ok(())
    }

    /// Initialize OVM with default configuration
    pub fn initialize_ovm_default(&mut self) -> Result<(), IntegrationError> {
        match self.initialize_ovm(OvmConfig::default()) {
            Ok(()) => Ok(()),
            Err(e) => {
                crate::log::get_logger().error("ovm_integration", &format!("OVM initialization failed: {}", e));
                Err(e)
            }
        }
    }

    /// Check if OVM is available and running
    pub fn is_ovm_available(&self) -> bool {
        self.ovm.is_some()
    }

    /// Evaluate a program using the best available execution method
    pub fn eval_program(&mut self, program: Program) -> Result<Value, IntegrationError> {
        // Proactively persist any function declarations to cache (even if we end up using classic)
        if self.integration_config.ovm_cache_enabled {
            for stmt in &program.statements {
                if let crate::ast::Statement::FunctionDecl(fd) = stmt {
                    // Log any error; do not fail execution due to cache issues
                    if let Err(e) = self.save_function_to_cache(fd) {
                        crate::log::get_logger().warn(
                            "ovm_cache",
                            &format!("Cache save failed for function '{}': {}", fd.name, e),
                        );
                    }
                }
            }
        }

        let complexity = self.estimate_complexity(&program);
        let should_use_ovm = self.should_use_ovm(&program, complexity);

        if should_use_ovm {
            self.eval_program_ovm(program)
        } else {
            self.eval_program_classic(program)
        }
    }

    /// Evaluate using classic interpreter
    pub fn eval_program_classic(&mut self, program: Program) -> Result<Value, IntegrationError> {
        let start = std::time::Instant::now();
        let result = self
            .classic_interpreter
            .eval_program(program)
            .map_err(IntegrationError::ClassicInterpreterError)?;
        let duration = start.elapsed();

        self.update_classic_stats(duration);
        Ok(result)
    }

    /// Evaluate using OVM with enhanced builtin support
    pub fn eval_program_ovm(&mut self, program: Program) -> Result<Value, IntegrationError> {
        if self.ovm.is_none() {
            return Err(IntegrationError::OvmNotInitialized);
        }

        let start = std::time::Instant::now();
        let auto_compile = self.integration_config.auto_compile_functions;

        // Ahead-of-time: analyze all functions and pre-compile eligible ones
        self.analyze_program_functions(&program);
        if auto_compile {
            for stmt in &program.statements {
                if let crate::ast::Statement::FunctionDecl(fd) = stmt {
                    if let Some(analysis) = self.function_analysis.get(&fd.name) {
                        if analysis.eligible {
                            // Compile once if not compiled yet
                            let already_compiled = matches!(self.compiled_state.get(&fd.name), Some(CompiledState::Compiled(_)));
                            if !already_compiled {
                                if let Some(ovm) = &mut self.ovm {
                                    if let Ok(func_id) = ovm.register_function(fd.clone()) {
                                        self.function_registry.insert(fd.name.clone(), func_id);
                                        self.compiled_state.insert(fd.name.clone(), CompiledState::Compiled(func_id));
                                        self.increment_compilation_count();
                                        // Persist to disk cache for future runs
                                        let _ = self.save_function_to_cache(fd);
                                    }
                                }
                            }
                        } else {
                            self.compiled_state.insert(fd.name.clone(), CompiledState::NotEligible);
                        }
                    }
                }
            }
        }

        // Parallel settings from config/env
        let enable_parallel = self.integration_config.enable_parallel
            || std::env::var("OVM_ENABLE_PARALLEL").map(|v| v == "1" || v.to_lowercase() == "true").unwrap_or(false);
        let max_threads = self.integration_config.max_parallelism
            .or_else(|| std::env::var("OVM_PARALLELISM").ok().and_then(|s| s.parse::<usize>().ok()));
        if enable_parallel {
            ParallelExecutor::initialize_if_needed(max_threads);
        }

        // Execute each statement through OVM (sequential by default)
        let mut last_value = Value::Unit;
        for statement in program.statements {
            match statement {
                crate::ast::Statement::Expression(expr) => {
                    // Fast-path: if this is a call to a precompiled function and args are variable-free, run directly in OVM
                    if let Some(res) = self.try_fastpath_compiled_call(&expr) {
                        last_value = res?;
                        continue;
                    }
                    // Enhanced builtin routing logic with variable environment synchronization
                    if self.should_use_ovm_for_expression(&expr) {
                        // Before routing to OVM, ensure all variables in the expression are accessible
                        if self.expression_needs_classic_variables(&expr) {
                            // Fallback to classic interpreter for expressions that reference variables
                            last_value = self
                                .classic_interpreter
                                .eval_statement(&crate::ast::Statement::Expression(expr))
                                .map_err(IntegrationError::ClassicInterpreterError)?;
                            self.increment_fallback_count();
                        } else {
                            // Use OVM for enhanced execution
                            let ovm_value = self
                                .ovm
                                .as_mut()
                                .ok_or(IntegrationError::OvmNotInitialized)?
                                .execute_expression(expr)
                                .map_err(IntegrationError::OvmExecutionError)?;
                            last_value = self.convert_ovm_to_ast_value(ovm_value)?;
                        }
                    } else {
                        // Fallback to classic interpreter for complex builtins and pipelines
                        last_value = self
                            .classic_interpreter
                            .eval_statement(&crate::ast::Statement::Expression(expr))
                            .map_err(IntegrationError::ClassicInterpreterError)?;
                        self.increment_fallback_count();
                    }
                }
                crate::ast::Statement::FunctionDecl(func_decl) => {
                    if auto_compile {
                        let already_compiled = matches!(self.compiled_state.get(&func_decl.name), Some(CompiledState::Compiled(_)));
                        if !already_compiled {
                            let func_id = self
                                .ovm
                                .as_mut()
                                .ok_or(IntegrationError::OvmNotInitialized)?
                                .register_function(func_decl.clone())
                                .map_err(IntegrationError::OvmExecutionError)?;
                            self.function_registry
                                .insert(func_decl.name.clone(), func_id);
                            self.compiled_state.insert(func_decl.name.clone(), CompiledState::Compiled(func_id));
                            self.increment_compilation_count();
                            // Persist to disk cache for future runs
                            let _ = self.save_function_to_cache(&func_decl);
                        }
                    }

                    // Also register with classic interpreter for compatibility
                    last_value = self
                        .classic_interpreter
                        .eval_statement(&crate::ast::Statement::FunctionDecl(func_decl))
                        .map_err(IntegrationError::ClassicInterpreterError)?;
                }
                other_statement => {
                    // Handle other statement types with classic interpreter
                    last_value = self
                        .classic_interpreter
                        .eval_statement(&other_statement)
                        .map_err(IntegrationError::ClassicInterpreterError)?;
                }
            }
        }

        let duration = start.elapsed();
        self.update_ovm_stats(duration);
        Ok(last_value)
    }

    /// Force garbage collection in OVM if available
    pub fn force_gc(&mut self) -> Result<(), IntegrationError> {
        if let Some(ovm) = &mut self.ovm {
            ovm.force_gc()
                .map_err(IntegrationError::OvmExecutionError)?;
        }
        Ok(())
    }

    /// Get performance statistics
    pub fn get_stats(&self) -> ExecutionStats {
        if let Ok(stats) = self.execution_stats.lock() {
            stats.clone()
        } else {
            ExecutionStats::default()
        }
    }

    /// Get the classic interpreter for direct access
    pub fn get_classic_interpreter(&mut self) -> &mut Interpreter {
        &mut self.classic_interpreter
    }

    /// Check if OVM is healthy and running
    pub fn is_ovm_healthy(&self) -> bool {
        // For now, consider OVM healthy if it exists
        // In a future version, we could expose a proper health check method from OVM
        self.ovm.is_some()
    }

    /// Restart OVM if it has stopped
    pub fn ensure_ovm_running(&mut self) -> Result<(), IntegrationError> {
        if self.ovm.is_none() {
            // Initialize OVM if not present
            self.initialize_ovm_default()?;
        }
        Ok(())
    }

    /// Get detailed OVM status
    pub fn get_ovm_status(&self) -> OvmStatus {
        if let Some(_ovm) = &self.ovm {
            OvmStatus {
                initialized: true,
                running: true,                     // Assume running if OVM exists
                memory_usage: 0,                   // Would need additional OVM methods for this
                uptime: std::time::Duration::ZERO, // Would need additional OVM methods for this
            }
        } else {
            OvmStatus {
                initialized: false,
                running: false,
                memory_usage: 0,
                uptime: std::time::Duration::ZERO,
            }
        }
    }

    /// Register a function for OVM compilation
    pub fn register_function(
        &mut self,
        func_decl: FunctionDecl,
    ) -> Result<FunctionId, IntegrationError> {
        let ovm = self
            .ovm
            .as_mut()
            .ok_or(IntegrationError::OvmNotInitialized)?;

        let func_id = ovm
            .register_function(func_decl.clone())
            .map_err(IntegrationError::OvmExecutionError)?;

        self.function_registry
            .insert(func_decl.name.clone(), func_id);
        self.increment_compilation_count();

        Ok(func_id)
    }

    // Private helper methods

    fn should_use_ovm(&self, _program: &Program, complexity: usize) -> bool {
        self.is_ovm_available()
            && (self.integration_config.use_ovm_by_default
                || complexity >= self.integration_config.ovm_complexity_threshold)
    }

    fn estimate_complexity(&self, program: &Program) -> usize {
        // Simple complexity estimation based on AST nodes
        program.statements.len() * 2 // Placeholder implementation
    }

    fn convert_ovm_to_ast_value(&self, ovm_value: OvmValue) -> Result<Value, IntegrationError> {
        // Convert OVM value back to AST value
        ovm_value.to_ast().map_err(|e| {
            IntegrationError::ConversionError(format!("OVM to AST conversion failed: {:?}", e))
        })
    }

    fn update_classic_stats(&self, duration: std::time::Duration) {
        if let Ok(mut stats) = self.execution_stats.lock() {
            stats.classic_executions += 1;
            let duration_ms = duration.as_secs_f64() * 1000.0;
            stats.average_classic_time_ms = (stats.average_classic_time_ms
                * (stats.classic_executions - 1) as f64
                + duration_ms)
                / stats.classic_executions as f64;
        }
    }

    fn update_ovm_stats(&self, duration: std::time::Duration) {
        if let Ok(mut stats) = self.execution_stats.lock() {
            stats.ovm_executions += 1;
            let duration_ms = duration.as_secs_f64() * 1000.0;
            stats.average_ovm_time_ms =
                (stats.average_ovm_time_ms * (stats.ovm_executions - 1) as f64 + duration_ms)
                    / stats.ovm_executions as f64;
        }
    }

    fn increment_fallback_count(&self) {
        if let Ok(mut stats) = self.execution_stats.lock() {
            stats.fallback_executions += 1;
        }
    }

    fn increment_compilation_count(&self) {
        if let Ok(mut stats) = self.execution_stats.lock() {
            stats.compilation_count += 1;
        }
    }

    // === Persistent cache helpers ===
    fn cache_dir_path() -> PathBuf {
        // Allow overriding the cache directory via environment variables
        // Primary: OLANG_OVM_CACHE_DIR, Fallback: OVM_CACHE_DIR
        if let Ok(dir) = std::env::var("OLANG_OVM_CACHE_DIR") {
            return PathBuf::from(dir);
        }
        if let Ok(dir) = std::env::var("OVM_CACHE_DIR") {
            return PathBuf::from(dir);
        }
        if let Some(home) = dirs::home_dir() {
            home.join(".olang").join("ovm-cache")
        } else {
            PathBuf::from(".olang/ovm-cache")
        }
    }

    fn ensure_cache_dir() -> Result<PathBuf, IntegrationError> {
        let dir = Self::cache_dir_path();
        if let Err(e) = fs::create_dir_all(&dir) {
            return Err(IntegrationError::ConversionError(format!(
                "Failed to create cache dir {}: {}",
                dir.display(), e
            )));
        }
        Ok(dir)
    }

    fn function_key(fd: &FunctionDecl) -> Result<String, IntegrationError> {
        let bytes = bincode::serialize(fd).map_err(|e| IntegrationError::ConversionError(format!(
            "Failed to serialize function '{}': {:?}", fd.name, e
        )))?;
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let hash = hasher.finalize();
        Ok(format!("{:x}", hash))
    }

    fn save_function_to_cache(&self, fd: &FunctionDecl) -> Result<(), IntegrationError> {
        if !self.integration_config.ovm_cache_enabled { return Ok(()); }
        let dir = Self::ensure_cache_dir()?;
        let key = Self::function_key(fd)?;
        let filename = format!("{}-{}.olfunc", fd.name, &key[..16]);
        let path = dir.join(&filename);
        let bytes = bincode::serialize(fd).map_err(|e| IntegrationError::ConversionError(format!(
            "Failed to serialize function for cache: {:?}", e
        )))?;
        // Atomic write: write to temp file then rename
        let tmp_path = dir.join(format!("{}.tmp", filename));
        match fs::write(&tmp_path, &bytes) {
            Ok(()) => {
                if let Err(e) = fs::rename(&tmp_path, &path) {
                    let _ = fs::remove_file(&tmp_path);
                    let msg = format!("Failed to move cache file into place ({} -> {}): {}", tmp_path.display(), path.display(), e);
                    crate::log::get_logger().warn("ovm_cache", &msg);
                    return Err(IntegrationError::ConversionError(msg));
                }
                crate::log::get_logger().info(
                    "ovm_cache",
                    &format!("Saved function '{}' to {}", fd.name, path.display()),
                );
                Ok(())
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                let msg = format!("Failed to write cache temp file {}: {}", tmp_path.display(), e);
                crate::log::get_logger().warn("ovm_cache", &msg);
                Err(IntegrationError::ConversionError(msg))
            }
        }
    }

    fn load_cached_functions(&mut self) -> Result<(), IntegrationError> {
        if self.cache_loaded || !self.integration_config.ovm_cache_enabled { return Ok(()); }
        let dir = Self::ensure_cache_dir()?;
        let entries = fs::read_dir(&dir).map_err(|e| IntegrationError::ConversionError(format!(
            "Failed to read cache dir {}: {}", dir.display(), e
        )))?;
        let mut loaded = 0usize;
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "olfunc" || ext == "bin" {
                    match fs::read(&path) {
                        Ok(bytes) => {
                            match bincode::deserialize::<FunctionDecl>(&bytes) {
                                Ok(fd) => {
                                    if let Ok(key) = Self::function_key(&fd) {
                                        self.cached_functions.insert(key, fd);
                                        loaded += 1;
                                    }
                                }
                                Err(e) => {
                                    crate::log::get_logger().warn(
                                        "ovm_cache",
                                        &format!(
                                            "Failed to deserialize cache file {}: {:?}",
                                            path.display(), e
                                        ),
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            crate::log::get_logger().warn(
                                "ovm_cache",
                                &format!("Failed to read cache file {}: {}", path.display(), e),
                            );
                        }
                    }
                }
            }
        }
        self.cache_loaded = true;
        crate::log::get_logger().info(
            "ovm_cache",
            &format!("Loaded {} cached function(s) from {}", loaded, dir.display()),
        );
        Ok(())
    }

    fn preload_cached_functions(&mut self) -> Result<(), IntegrationError> {
        if self.ovm.is_none() || !self.integration_config.ovm_cache_enabled { return Ok(()); }
        // Register any cached function with the OVM to avoid re-parsing/resolution cost later
        let mut registered = 0usize;
        for fd in self.cached_functions.values().cloned() {
            if let Some(ovm) = &mut self.ovm {
                if let Ok(func_id) = ovm.register_function(fd.clone()) {
                    self.function_registry.insert(fd.name.clone(), func_id);
                    self.compiled_state.insert(fd.name.clone(), CompiledState::Compiled(func_id));
                    registered += 1;
                    // Do not increment compilation_count here; these are preloaded
                }
            }
        }
        crate::log::get_logger().info(
            "ovm_cache",
            &format!("Preloaded {} cached function(s) into OVM", registered),
        );
        Ok(())
    }

    /// Enhanced expression routing logic
    fn should_use_ovm_for_expression(&self, expr: &crate::ast::Expr) -> bool {
        // First check if the expression needs classic variables - if so, don't use OVM
        if self.expression_needs_classic_variables(expr) {
            return false;
        }

        match expr {
            // Function calls - enhanced builtin routing
            crate::ast::Expr::Call { callee, .. } => {
                match callee.as_ref() {
                    crate::ast::Expr::Identifier(name) => {
                        if self.is_builtin_function(name) {
                            // Check if this builtin should use OVM
                            self.should_use_ovm_for_builtin(name)
                        } else {
                            // User functions can use OVM
                            self.integration_config.use_ovm_by_default
                        }
                    }
                    crate::ast::Expr::FieldAccess { .. } => {
                        // Module functions (like math.sqrt) can use OVM
                        true
                    }
                    _ => false, // Other complex callees use classic interpreter for safety
                }
            }

            // Pipeline expressions can benefit from OVM optimization - but only if no variables
            crate::ast::Expr::Pipeline { .. } => true,

            // Identifiers, loops should use classic interpreter for environment consistency
            crate::ast::Expr::Identifier(_) => false,
            crate::ast::Expr::ForLoop { .. } => false,
            crate::ast::Expr::WhileLoop { .. } => false,
            crate::ast::Expr::Loop { .. } => false,

            // Simple expressions can use OVM
            crate::ast::Expr::Integer(_) => true,
            crate::ast::Expr::Float(_) => true,
            crate::ast::Expr::String(_) => true,
            crate::ast::Expr::Boolean(_) => true,
            crate::ast::Expr::List(_) => true,
            crate::ast::Expr::Tuple(_) => true,
            crate::ast::Expr::BinaryOp { .. } => true,
            crate::ast::Expr::UnaryOp { .. } => true,

            // Complex expressions that can benefit from OVM optimization
            crate::ast::Expr::Lambda { .. } => true,
            crate::ast::Expr::Match { .. } => false, // Pattern matching uses classic for now
            crate::ast::Expr::StructLiteral(_) => true,
            crate::ast::Expr::FieldAccess { .. } => true,
            crate::ast::Expr::Index { .. } => true,

            // Default to OVM if configured
            _ => self.integration_config.use_ovm_by_default,
        }
    }

    /// Determine if a builtin function should use OVM execution
    fn should_use_ovm_for_builtin(&self, builtin_name: &str) -> bool {
        // Check if OVM builtins are enabled
        if !self.integration_config.enable_ovm_builtins {
            return false;
        }

        // Check if this builtin is in the OVM-preferred list
        self.integration_config
            .ovm_preferred_builtins
            .contains(&builtin_name.to_string())
    }

    /// Get OVM execution statistics
    pub fn get_ovm_builtin_stats(&self) -> HashMap<String, u32> {
        // In a full implementation, this would track per-builtin execution counts
        HashMap::new()
    }

    /// Force enable/disable OVM builtins at runtime
    pub fn set_ovm_builtins_enabled(&mut self, enabled: bool) {
        self.integration_config.enable_ovm_builtins = enabled;
    }

    /// Add a builtin to the OVM-preferred list
    pub fn add_ovm_preferred_builtin(&mut self, builtin_name: String) {
        if !self
            .integration_config
            .ovm_preferred_builtins
            .contains(&builtin_name)
        {
            self.integration_config
                .ovm_preferred_builtins
                .push(builtin_name);
        }
    }

    /// Remove a builtin from the OVM-preferred list
    pub fn remove_ovm_preferred_builtin(&mut self, builtin_name: &str) {
        self.integration_config
            .ovm_preferred_builtins
            .retain(|name| name != builtin_name);
    }

    /// Check if a name corresponds to a builtin function
    fn is_builtin_function(&self, name: &str) -> bool {
        // List of builtin functions that should use classic interpreter
        matches!(
            name,
            "println"
                | "print"
                | "typeof"
                | "len"
                | "head"
                | "tail"
                | "cons"
                | "to_string"
                | "to_int"
                | "to_float"
                | "range"
                | "zip"
                | "map"
                | "filter"
                | "reduce"
                | "fold"
                | "reverse"
                | "sort"
                | "join"
                | "split"
                | "contains"
                | "sum"
                | "average"
                | "min"
                | "max"
                | "clamp"
                | "flatten"
                | "chunk"
                | "enumerate"
                | "find"
                | "starts_with"
                | "ends_with"
                | "group_by"
                | "set_parallel"
                | "take"
                | "skip"
                | "force"
                | "lazy"
        )
    }

    /// Check if an expression references variables that need classic interpreter resolution
    fn expression_needs_classic_variables(&self, expr: &crate::ast::Expr) -> bool {
        match expr {
            crate::ast::Expr::Identifier(name) => {
                // Builtin functions don't need variable resolution
                !self.is_builtin_function(name)
            }
            crate::ast::Expr::Call { callee, arguments } => {
                // Check callee and arguments for variable references
                if self.expression_needs_classic_variables(callee) {
                    return true;
                }
                for arg in arguments {
                    let arg_expr = match arg {
                        Argument::Positional(expr) => expr,
                        Argument::Named { value, .. } => value,
                    };
                    if self.expression_needs_classic_variables(arg_expr) {
                        return true;
                    }
                }
                false
            }
            crate::ast::Expr::BinaryOp { left, right, .. } => {
                self.expression_needs_classic_variables(left) || self.expression_needs_classic_variables(right)
            }
            crate::ast::Expr::UnaryOp { operand, .. } => {
                self.expression_needs_classic_variables(operand)
            }
            crate::ast::Expr::List(elements) => {
                elements.iter().any(|e| self.expression_needs_classic_variables(e))
            }
            crate::ast::Expr::Tuple(elements) => {
                elements.iter().any(|e| self.expression_needs_classic_variables(e))
            }
            crate::ast::Expr::Index { object, index } => {
                self.expression_needs_classic_variables(object) || self.expression_needs_classic_variables(index)
            }
            crate::ast::Expr::FieldAccess { object, .. } => {
                self.expression_needs_classic_variables(object)
            }
            crate::ast::Expr::Pipeline { left, right } => {
                self.expression_needs_classic_variables(left) || self.expression_needs_classic_variables(right)
            }
            // Literals don't need variable resolution
            crate::ast::Expr::Integer(_) | crate::ast::Expr::Float(_) | crate::ast::Expr::String(_) | crate::ast::Expr::Boolean(_) => false,
            // For other expressions, be conservative and assume they might need variables
            _ => true,
        }
    }

    /// Variant that treats certain identifiers as allowed (e.g., function parameters)
    fn expression_needs_classic_variables_with_allow(&self, expr: &crate::ast::Expr, allowed: &std::collections::HashSet<String>) -> bool {
        match expr {
            crate::ast::Expr::Identifier(name) => {
                // Allowed identifiers (params) and builtins do not require classic resolution
                !(allowed.contains(name) || self.is_builtin_function(name))
            }
            crate::ast::Expr::Call { callee, arguments } => {
                if self.expression_needs_classic_variables_with_allow(callee, allowed) {
                    return true;
                }
                for arg in arguments {
                    let arg_expr = match arg {
                        Argument::Positional(expr) => expr,
                        Argument::Named { value, .. } => value,
                    };
                    if self.expression_needs_classic_variables_with_allow(arg_expr, allowed) {
                        return true;
                    }
                }
                false
            }
            crate::ast::Expr::BinaryOp { left, right, .. } => {
                self.expression_needs_classic_variables_with_allow(left, allowed)
                    || self.expression_needs_classic_variables_with_allow(right, allowed)
            }
            crate::ast::Expr::UnaryOp { operand, .. } => {
                self.expression_needs_classic_variables_with_allow(operand, allowed)
            }
            crate::ast::Expr::List(elements) => {
                elements.iter().any(|e| self.expression_needs_classic_variables_with_allow(e, allowed))
            }
            crate::ast::Expr::Tuple(elements) => {
                elements.iter().any(|e| self.expression_needs_classic_variables_with_allow(e, allowed))
            }
            crate::ast::Expr::Index { object, index } => {
                self.expression_needs_classic_variables_with_allow(object, allowed)
                    || self.expression_needs_classic_variables_with_allow(index, allowed)
            }
            crate::ast::Expr::FieldAccess { object, .. } => {
                self.expression_needs_classic_variables_with_allow(object, allowed)
            }
            crate::ast::Expr::Pipeline { left, right } => {
                self.expression_needs_classic_variables_with_allow(left, allowed)
                    || self.expression_needs_classic_variables_with_allow(right, allowed)
            }
            crate::ast::Expr::Integer(_)
            | crate::ast::Expr::Float(_)
            | crate::ast::Expr::String(_)
            | crate::ast::Expr::Boolean(_) => false,
            _ => true,
        }
    }

    /// Determine if an expression is OVM-friendly given a set of allowed identifiers (params)
    fn is_ovm_friendly_expr_with_params(&self, expr: &crate::ast::Expr, allowed: &std::collections::HashSet<String>) -> bool {
        // Reject constructs we don't support in OVM yet
        match expr {
            crate::ast::Expr::Match { .. }
            | crate::ast::Expr::ForLoop { .. }
            | crate::ast::Expr::WhileLoop { .. }
            | crate::ast::Expr::Loop { .. } => return false,
            _ => {}
        }
        // If needs classic variables beyond allowed, not friendly
        if self.expression_needs_classic_variables_with_allow(expr, allowed) {
            return false;
        }
        // Recursively check children where applicable
        match expr {
            crate::ast::Expr::Call { callee, arguments } => {
                if !self.is_ovm_friendly_expr_with_params(callee, allowed) { return false; }
                for arg in arguments {
                    let e = match arg { Argument::Positional(e) => e, Argument::Named { value, .. } => value };
                    if !self.is_ovm_friendly_expr_with_params(e, allowed) { return false; }
                }
                true
            }
            crate::ast::Expr::BinaryOp { left, right, .. } => {
                self.is_ovm_friendly_expr_with_params(left, allowed)
                    && self.is_ovm_friendly_expr_with_params(right, allowed)
            }
            crate::ast::Expr::UnaryOp { operand, .. } => self.is_ovm_friendly_expr_with_params(operand, allowed),
            crate::ast::Expr::List(items) => items.iter().all(|e| self.is_ovm_friendly_expr_with_params(e, allowed)),
            crate::ast::Expr::Tuple(items) => items.iter().all(|e| self.is_ovm_friendly_expr_with_params(e, allowed)),
            crate::ast::Expr::Pipeline { left, right } => {
                self.is_ovm_friendly_expr_with_params(left, allowed)
                    && self.is_ovm_friendly_expr_with_params(right, allowed)
            }
            _ => true,
        }
    }

    /// Estimate cost as approximate node count times a small constant
    fn estimate_expr_cost(&self, expr: &crate::ast::Expr) -> usize {
        fn count(expr: &crate::ast::Expr) -> usize {
            match expr {
                crate::ast::Expr::Call { callee, arguments } => {
                    let mut c = 1 + count(callee);
                    for arg in arguments {
                        let e = match arg { Argument::Positional(e) => e, Argument::Named { value, .. } => value };
                        c += count(e);
                    }
                    c
                }
                crate::ast::Expr::BinaryOp { left, right, .. } => 1 + count(left) + count(right),
                crate::ast::Expr::UnaryOp { operand, .. } => 1 + count(operand),
                crate::ast::Expr::List(items) => 1 + items.iter().map(count).sum::<usize>(),
                crate::ast::Expr::Tuple(items) => 1 + items.iter().map(count).sum::<usize>(),
                crate::ast::Expr::Pipeline { left, right } => 1 + count(left) + count(right),
                crate::ast::Expr::If { condition, then_branch, else_branch } => {
                    1 + count(condition) + count(then_branch) + else_branch.as_ref().map(|e| count(e)).unwrap_or(0)
                }
                _ => 1,
            }
        }
        count(expr)
    }


    /// Analyze a function declaration and produce FunctionAnalysis
    fn analyze_function(&self, func_decl: &FunctionDecl) -> FunctionAnalysis {
        let mut allowed = std::collections::HashSet::new();
        for p in &func_decl.parameters { allowed.insert(p.name.clone()); }
        let eligible = self.is_ovm_friendly_expr_with_params(&func_decl.body, &allowed);
        let cost = self.estimate_expr_cost(&func_decl.body);
        FunctionAnalysis { eligible, cost, reason: if eligible { None } else { Some("Not OVM-friendly body".to_string()) } }
    }

    /// Analyze all functions in a program and populate caches
    fn analyze_program_functions(&mut self, program: &Program) {
        for stmt in &program.statements {
            if let crate::ast::Statement::FunctionDecl(fd) = stmt {
                let analysis = self.analyze_function(fd);
                self.function_analysis.insert(fd.name.clone(), analysis);
                // Initialize compiled state if missing
                self.compiled_state.entry(fd.name.clone()).or_insert(CompiledState::Pending);
            }
        }
    }

    /// Try to bypass routing for direct calls to compiled functions
    fn try_fastpath_compiled_call(&mut self, expr: &crate::ast::Expr) -> Option<Result<Value, IntegrationError>> {
        use crate::ast::Expr as E;
        if let E::Call { callee, arguments: _ } = expr {
            if let E::Identifier(name) = callee.as_ref() {
                if let Some(CompiledState::Compiled(_fid)) = self.compiled_state.get(name).copied() {
                    // Only fast-path when the call expression has no external identifiers
                    let allowed = std::collections::HashSet::new();
                    if self.expression_needs_classic_variables_with_allow(expr, &allowed) {
                        return None;
                    }
                    let ovm_value = self
                        .ovm
                        .as_mut()
                        .ok_or(IntegrationError::OvmNotInitialized)
                        .and_then(|ovm| ovm.execute_expression(expr.clone()).map_err(IntegrationError::OvmExecutionError));
                    return Some(ovm_value.and_then(|v| self.convert_ovm_to_ast_value(v)));
                }
            }
        }
        None
    }
}

impl Default for OvmInterpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for OvmInterpreter {
    fn drop(&mut self) {
        // Ensure OVM is properly shut down
        if let Some(ovm) = &mut self.ovm {
            let _ = ovm.stop();
        }
    }
}

/// Integration-specific error types
#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("Classic interpreter error: {0}")]
    ClassicInterpreterError(#[from] InterpreterError),

    #[error("OVM initialization error: {0}")]
    OvmInitError(#[from] OvmError),

    #[error("OVM execution error: {0}")]
    OvmExecutionError(OvmError),

    #[error("OVM not initialized")]
    OvmNotInitialized,

    #[error("Value conversion error: {0}")]
    ConversionError(String),
}

/// Result type for integration operations
pub type IntegrationResult<T> = Result<T, IntegrationError>;

// Implement Clone for ExecutionStats to make it accessible
impl Clone for ExecutionStats {
    fn clone(&self) -> Self {
        Self {
            classic_executions: self.classic_executions,
            ovm_executions: self.ovm_executions,
            fallback_executions: self.fallback_executions,
            compilation_count: self.compilation_count,
            average_classic_time_ms: self.average_classic_time_ms,
            average_ovm_time_ms: self.average_ovm_time_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expr, Statement};

    #[test]
    fn test_ovm_interpreter_creation() {
        let interpreter = OvmInterpreter::new();
        assert!(!interpreter.is_ovm_available());
    }

    #[test]
    fn test_ovm_initialization() {
        let mut interpreter = OvmInterpreter::new();
        assert!(interpreter.initialize_ovm_default().is_ok());
        assert!(interpreter.is_ovm_available());
    }

    #[test]
    fn test_classic_fallback() {
        let mut interpreter = OvmInterpreter::new();
        let program = Program {
            statements: vec![Statement::Expression(Expr::Integer(42))],
        };

        // Should use classic interpreter when OVM not available
        let result = interpreter.eval_program(program).expect("Expected successful program evaluation");
        assert_eq!(result, Value::Integer(42));

        let stats = interpreter.get_stats();
        assert_eq!(stats.classic_executions, 1);
        assert_eq!(stats.ovm_executions, 0);
    }

    #[test]
    fn test_builtin_routing_configuration() {
        let mut interpreter = OvmInterpreter::new();

        // Test default configuration
        assert!(interpreter.integration_config.enable_ovm_builtins);
        assert!(interpreter
            .integration_config
            .ovm_preferred_builtins
            .contains(&"len".to_string()));

        // Test runtime configuration changes
        interpreter.set_ovm_builtins_enabled(false);
        assert!(!interpreter.integration_config.enable_ovm_builtins);

        interpreter.add_ovm_preferred_builtin("custom_builtin".to_string());
        assert!(interpreter
            .integration_config
            .ovm_preferred_builtins
            .contains(&"custom_builtin".to_string()));

        interpreter.remove_ovm_preferred_builtin("len");
        assert!(!interpreter
            .integration_config
            .ovm_preferred_builtins
            .contains(&"len".to_string()));
    }

    #[test]
    fn test_builtin_function_detection() {
        let interpreter = OvmInterpreter::new();

        // Test builtin detection
        assert!(interpreter.is_builtin_function("len"));
        assert!(interpreter.is_builtin_function("map"));
        assert!(interpreter.is_builtin_function("println"));
        assert!(!interpreter.is_builtin_function("not_a_builtin"));

        // Test OVM builtin preferences
        assert!(interpreter.should_use_ovm_for_builtin("len"));
        assert!(interpreter.should_use_ovm_for_builtin("sum"));
        assert!(!interpreter.should_use_ovm_for_builtin("map")); // Not in OVM preferred list
    }

    #[test]
    fn test_expression_routing() {
        let interpreter = OvmInterpreter::new();

        // Test simple expressions route to OVM
        let simple_expr = Expr::Integer(42);
        assert!(interpreter.should_use_ovm_for_expression(&simple_expr));

        // Test complex expressions route to classic
        let identifier_expr = Expr::Identifier("some_var".to_string());
        assert!(!interpreter.should_use_ovm_for_expression(&identifier_expr));

        // Test OVM-preferred builtin call routes to OVM
        let len_call = Expr::Call {
            callee: Box::new(Expr::Identifier("len".to_string())),
            arguments: vec![Argument::Positional(Expr::List(std::sync::Arc::from(
                [Expr::Integer(1), Expr::Integer(2)] as [Expr; 2],
            )))],
        };
        assert!(interpreter.should_use_ovm_for_expression(&len_call));

        // Test non-OVM builtin call routes to classic
        let map_call = Expr::Call {
            callee: Box::new(Expr::Identifier("map".to_string())),
            arguments: vec![
                Argument::Positional(Expr::List(std::sync::Arc::from(
                    [Expr::Integer(1), Expr::Integer(2)] as [Expr; 2]
                ))),
                Argument::Positional(Expr::Lambda {
                    parameters: vec![crate::ast::Parameter {
                        name: "x".to_string(),
                        type_annotation: None,
                        default_value: None,
                    }],
                    body: Box::new(Expr::BinaryOp {
                        left: Box::new(Expr::Identifier("x".to_string())),
                        op: crate::ast::BinaryOp::Add,
                        right: Box::new(Expr::Integer(1)),
                    }),
                    return_type: None,
                }),
            ],
        };
        assert!(!interpreter.should_use_ovm_for_expression(&map_call));
    }
}

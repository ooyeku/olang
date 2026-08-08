use crate::analyze::AnalysisReport;
use crate::ast::{
    Argument, BinaryOp, BuiltinFunction, EnumVariantData, Expr, Function, FunctionDecl, LetDecl,
    MatchArm, Parameter, Pattern, Program, PromiseType, ShareDecl, Statement, TestDecl,
    TypeAnnotation, UnaryOp, UseDecl, Value,
};
use crate::async_runtime::AsyncRuntime;
use crate::builtin::BuiltinFunctions;
use crate::internal::{check_memory_pressure, LazyConfig};
use crate::ovm::gc::SafepointManager;
use crate::type_checker::TypeChecker;
use im::HashMap as ImHashMap;
use sha2::Digest; // For SHA256 hashing
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error; // Persistent/immutable HashMap for O(1) cloning

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
    // Loop control flow signals — intercepted by the loop evaluators, an error
    // only if they escape to top level (i.e. used outside a loop)
    #[error("'break' used outside of a loop")]
    BreakSignal,
    #[error("'continue' used outside of a loop")]
    ContinueSignal,

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

    // Feature 7: Circular dependency detection
    #[error("Circular dependency detected: {cycle_path}")]
    CircularDependencyDetected { cycle_path: String },

    // Feature 9: Enhanced multi-file error messages
    #[error("Module '{module_path}' not found")]
    ModuleNotFound {
        module_path: String,
        searched_paths: Vec<String>,
        available_modules: Vec<String>,
        suggestions: Vec<String>,
    },
    #[error("Function '{function_name}' not found in module '{module_path}'")]
    FunctionNotFoundInModule {
        function_name: String,
        module_path: String,
        available_functions: Vec<String>,
        suggestions: Vec<String>,
        file_path: Option<String>,
    },
    #[error("Import error in '{file_path}' at line {line}")]
    ImportError {
        file_path: String,
        line: usize,
        column: Option<usize>,
        import_path: String,
        reason: String,
        suggestions: Vec<String>,
    },
    #[error("Type mismatch in '{file_path}' from module '{module_path}'")]
    MultiFileTypeMismatch {
        file_path: String,
        module_path: String,
        expected_type: String,
        actual_type: String,
        function_name: Option<String>,
        suggestions: Vec<String>,
    },
    #[error("Dependency chain error: {chain:?}")]
    DependencyChainError {
        chain: Vec<String>,
        root_error: String,
        suggested_fix: String,
    },
}

/// Feature 9: Enhanced error message formatter for multi-file development
#[derive(Clone)]
pub struct IntuitiveErrorFormatter {
    pub use_colors: bool,
    pub show_suggestions: bool,
    pub show_context: bool,
    pub max_suggestions: usize,
    pub max_list_items: usize,
}

impl Default for IntuitiveErrorFormatter {
    fn default() -> Self {
        Self {
            use_colors: true,
            show_suggestions: true,
            show_context: true,
            max_suggestions: 5,
            max_list_items: 10,
        }
    }
}

impl IntuitiveErrorFormatter {
    /// Format an interpreter error with enhanced context and suggestions
    pub fn format_error(&self, error: &InterpreterError) -> String {
        match error {
            InterpreterError::ModuleNotFound {
                module_path,
                searched_paths,
                available_modules,
                suggestions,
            } => self.format_module_not_found_error(
                module_path,
                searched_paths,
                available_modules,
                suggestions,
            ),
            InterpreterError::FunctionNotFoundInModule {
                function_name,
                module_path,
                available_functions,
                suggestions,
                file_path,
            } => self.format_function_not_found_error(
                function_name,
                module_path,
                available_functions,
                suggestions,
                file_path,
            ),
            InterpreterError::ImportError {
                file_path,
                line,
                column,
                import_path,
                reason,
                suggestions,
            } => {
                self.format_import_error(file_path, *line, column, import_path, reason, suggestions)
            }
            InterpreterError::MultiFileTypeMismatch {
                file_path,
                module_path,
                expected_type,
                actual_type,
                function_name,
                suggestions,
            } => self.format_type_mismatch_error(
                file_path,
                module_path,
                expected_type,
                actual_type,
                function_name,
                suggestions,
            ),
            InterpreterError::DependencyChainError {
                chain,
                root_error,
                suggested_fix,
            } => self.format_dependency_chain_error(chain, root_error, suggested_fix),
            _ => {
                // Default formatting for other errors
                format!("{}", error)
            }
        }
    }

    fn format_module_not_found_error(
        &self,
        module_path: &str,
        searched_paths: &[String],
        available_modules: &[String],
        suggestions: &[String],
    ) -> String {
        let mut result = String::new();

        // Main error message
        result.push_str(&format!("Error: Cannot find module '{}'\n", module_path));

        // Show where we looked
        if !searched_paths.is_empty() {
            result.push_str("\nSearched in:\n");
            for path in searched_paths {
                let safe = self.sanitize_path(path);
                result.push_str(&format!("  • {}\n", safe));
            }
        }

        // Show suggestions if available
        if !suggestions.is_empty() {
            result.push_str("\nDid you mean:\n");
            for suggestion in suggestions.iter().take(self.max_suggestions) {
                result.push_str(&format!("  • {}\n", suggestion));
            }
        }

        // Show available modules if any
        if !available_modules.is_empty() {
            result.push_str("\nAvailable modules:\n");
            for module in available_modules.iter().take(self.max_list_items) {
                result.push_str(&format!("  • {}\n", module));
            }
            if available_modules.len() > self.max_list_items {
                result.push_str(&format!(
                    "  ... and {} more\n",
                    available_modules.len() - self.max_list_items
                ));
            }
        }

        // Helpful guidance
        result.push_str("\nHelp:\n");
        result.push_str("  • Check the module path spelling\n");
        result.push_str("  • Ensure the module file exists in the correct directory\n");
        result.push_str("  • For relative imports, check you're in the right directory\n");

        result
    }

    fn format_function_not_found_error(
        &self,
        function_name: &str,
        module_path: &str,
        available_functions: &[String],
        suggestions: &[String],
        file_path: &Option<String>,
    ) -> String {
        let mut result = String::new();

        // Main error message with context
        if let Some(path) = file_path.as_ref() {
            let safe_path = self.sanitize_path(path);
            result.push_str(&format!(
                "Error: Cannot find '{}' in {}\n",
                function_name, module_path
            ));
            result.push_str(&format!("  --> {}\n", safe_path));
        } else {
            result.push_str(&format!(
                "Error: Cannot find '{}' in {}\n",
                function_name, module_path
            ));
        }

        // Show import context
        result.push_str("\nIn import statement:\n");
        result.push_str(&format!("  use {} {{ {} }}\n", module_path, function_name));
        result.push_str(&format!(
            "             {}\n",
            "^".repeat(function_name.len())
        ));

        // Show suggestions
        if !suggestions.is_empty() {
            result.push_str("\nDid you mean:\n");
            for suggestion in suggestions.iter().take(self.max_suggestions) {
                result.push_str(&format!("  • {}\n", suggestion));
            }
        }

        // Show available functions
        if !available_functions.is_empty() {
            result.push_str(&format!("\nAvailable functions in {}:\n", module_path));
            for func in available_functions.iter().take(self.max_list_items) {
                result.push_str(&format!("  • {}\n", func));
            }
            if available_functions.len() > self.max_list_items {
                result.push_str(&format!(
                    "  ... and {} more\n",
                    available_functions.len() - self.max_list_items
                ));
            }
        }

        result.push_str("\nHelp:\n");
        result.push_str("  • Check the function name spelling\n");
        result.push_str("  • Ensure the function is marked with 'share' in the module\n");
        result.push_str(&format!(
            "  • Try: use {} {{ available_function_name }}\n",
            module_path
        ));

        result
    }

    fn format_import_error(
        &self,
        file_path: &str,
        line: usize,
        column: &Option<usize>,
        import_path: &str,
        reason: &str,
        suggestions: &[String],
    ) -> String {
        let mut result = String::new();
        let safe_path = self.sanitize_path(file_path);

        // Location information
        if let Some(col) = column {
            result.push_str("Error: Import failed\n");
            result.push_str(&format!("  --> {}:{}:{}\n", safe_path, line, col));
        } else {
            result.push_str("Error: Import failed\n");
            result.push_str(&format!("  --> {}:{}\n", safe_path, line));
        }

        result.push_str(&format!("\nFailed to import: {}\n", import_path));
        result.push_str(&format!("Reason: {}\n", reason));

        // Show suggestions
        if !suggestions.is_empty() {
            result.push_str("\nSuggestions:\n");
            for suggestion in suggestions.iter().take(self.max_suggestions) {
                result.push_str(&format!("  • {}\n", suggestion));
            }
        }

        result
    }

    fn format_type_mismatch_error(
        &self,
        file_path: &str,
        module_path: &str,
        expected_type: &str,
        actual_type: &str,
        function_name: &Option<String>,
        suggestions: &[String],
    ) -> String {
        let mut result = String::new();

        if let Some(func_name) = function_name {
            result.push_str(&format!(
                "Error: Type mismatch in function '{}'\n",
                func_name
            ));
        } else {
            result.push_str("Error: Type mismatch\n");
        }

        result.push_str(&format!(
            "  --> {} (from module {})\n",
            file_path, module_path
        ));
        result.push_str(&format!("\nExpected: {}\n", expected_type));
        result.push_str(&format!("Found:    {}\n", actual_type));

        if !suggestions.is_empty() {
            result.push_str("\nSuggestions:\n");
            for suggestion in suggestions {
                result.push_str(&format!("  • {}\n", suggestion));
            }
        }

        result
    }

    fn format_dependency_chain_error(
        &self,
        chain: &[String],
        root_error: &str,
        suggested_fix: &str,
    ) -> String {
        let mut result = String::new();

        result.push_str("Error: Dependency chain failure\n\n");
        result.push_str("Dependency chain:\n");
        for (i, module) in chain.iter().enumerate() {
            if i == chain.len() - 1 {
                result.push_str(&format!("  {} {} (error here)\n", "└─", module));
            } else {
                result.push_str(&format!(
                    "  {} {}\n",
                    if i == 0 { "┌─" } else { "├─" },
                    module
                ));
            }
        }

        result.push_str(&format!("\nRoot cause: {}\n", root_error));
        result.push_str(&format!("\nSuggested fix: {}\n", suggested_fix));

        result
    }

    /// Sanitize file paths to avoid leaking sensitive directories
    fn sanitize_path(&self, raw: &str) -> String {
        if let Ok(home) = std::env::var("HOME") {
            if raw.starts_with(&home) {
                return raw.replacen(&home, "~", 1);
            }
        }
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            if raw.starts_with(&userprofile) {
                return raw.replacen(&userprofile, "~", 1);
            }
        }
        raw.to_string()
    }

    /// Calculate Levenshtein distance for "did you mean" suggestions
    #[allow(clippy::needless_range_loop)] // matrix DP is clearest indexed
    pub fn levenshtein_distance(a: &str, b: &str) -> usize {
        let len_a = a.len();
        let len_b = b.len();

        if len_a == 0 {
            return len_b;
        }
        if len_b == 0 {
            return len_a;
        }

        let mut matrix = vec![vec![0; len_b + 1]; len_a + 1];

        for i in 0..=len_a {
            matrix[i][0] = i;
        }
        for j in 0..=len_b {
            matrix[0][j] = j;
        }

        for i in 1..=len_a {
            for j in 1..=len_b {
                let cost = if a.chars().nth(i - 1) == b.chars().nth(j - 1) {
                    0
                } else {
                    1
                };
                matrix[i][j] = (matrix[i - 1][j] + 1)
                    .min(matrix[i][j - 1] + 1)
                    .min(matrix[i - 1][j - 1] + cost);
            }
        }

        matrix[len_a][len_b]
    }

    /// Generate "did you mean" suggestions based on available options
    pub fn generate_suggestions(&self, target: &str, available: &[String]) -> Vec<String> {
        let mut suggestions: Vec<(String, usize)> = available
            .iter()
            .map(|option| (option.clone(), Self::levenshtein_distance(target, option)))
            .filter(|(_, distance)| *distance <= 3 && *distance > 0) // Only suggest if reasonably close
            .collect();

        suggestions.sort_by_key(|(_, distance)| *distance);
        suggestions
            .into_iter()
            .take(self.max_suggestions)
            .map(|(suggestion, _)| suggestion)
            .collect()
    }
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

/// Environment with Arc-based immutable collections for O(1) cloning
/// This is the core optimization - previous version cloned entire HashMap on every scope entry
#[derive(Clone)]
pub struct Environment {
    variables: Arc<ImHashMap<String, Value>>,
    /// Frame bindings (parameters, the function's own name, and — in frame
    /// environments — every runtime binding), probed by string compare
    /// before the map. Hashed HAMT traffic was the dominant interpreter
    /// cost twice over: closure copies per call, then let/assignment
    /// inserts per loop iteration.
    locals: Vec<(String, Value)>,
    /// Whether this is a transient frame (function call, loop body, match
    /// arm, catch block). Frames store new bindings in `locals` — a push
    /// and in-place overwrites — instead of the persistent map. The root
    /// environment is not a frame: top-level definitions go to the map so
    /// they persist and are cheap to snapshot into closures.
    is_frame: bool,
    parent: Option<Arc<Environment>>,
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

impl Environment {
    pub fn new() -> Self {
        Self {
            variables: Arc::new(ImHashMap::new()),
            locals: Vec::new(),
            is_frame: false,
            parent: None,
        }
    }

    /// Create child environment with parent reference - O(1) now!
    pub fn with_parent(parent: Environment) -> Self {
        Self {
            variables: Arc::new(ImHashMap::new()),
            locals: Vec::new(),
            is_frame: true,
            parent: Some(Arc::new(parent)),
        }
    }

    /// Create child environment from Arc parent - even cheaper
    pub fn with_parent_arc(parent: Arc<Environment>) -> Self {
        Self {
            variables: Arc::new(ImHashMap::new()),
            locals: Vec::new(),
            is_frame: true,
            parent: Some(parent),
        }
    }

    /// Define a variable. In a frame, bindings live in the probed locals
    /// vector (in-place overwrite on rebind, e.g. a `let` re-executed each
    /// loop iteration); at the root they go to the persistent map.
    pub fn define(&mut self, name: String, value: Value) {
        // A later binding must shadow an existing frame local of the same name
        if let Some(slot) = self.locals.iter_mut().rev().find(|(n, _)| *n == name) {
            slot.1 = value;
            return;
        }
        if self.is_frame {
            self.locals.push((name, value));
            return;
        }
        Arc::make_mut(&mut self.variables).insert(name, value);
    }

    /// Define a call-frame binding (a parameter or the function's own name)
    /// without touching the shared persistent map.
    pub fn define_local(&mut self, name: String, value: Value) {
        self.locals.push((name, value));
    }

    /// Fetch a resolved slot: hop `depth` parents, verify the slot holds
    /// `name`, and fall back to a normal lookup on any mismatch — static
    /// resolution can be stale (e.g. a conditionally-executed `let` shifted
    /// later slots), and the fallback keeps that a performance event, not a
    /// correctness event.
    pub fn get_slot(&self, name: &str, depth: u16, slot: u16) -> Option<Value> {
        let mut env = self;
        for _ in 0..depth {
            env = env.parent.as_deref()?;
        }
        if let Some((slot_name, value)) = env.locals.get(slot as usize) {
            if slot_name == name {
                return Some(value.clone());
            }
        }
        self.get(name)
    }

    /// Assign through a resolved slot, with the same verify-and-fall-back
    /// contract as get_slot.
    pub fn set_slot(
        &mut self,
        name: &str,
        depth: u16,
        slot: u16,
        value: Value,
    ) -> Result<(), InterpreterError> {
        // Walk mutably: hop through Arc parents with make_mut
        if depth == 0 {
            if let Some((slot_name, slot_value)) = self.locals.get_mut(slot as usize) {
                if slot_name == name {
                    *slot_value = value;
                    return Ok(());
                }
            }
            return self.set(name, value);
        }
        let Some(parent) = self.parent.as_mut() else {
            return self.set(name, value);
        };
        Arc::make_mut(parent).set_slot(name, depth - 1, slot, value)
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some((_, value)) = self.locals.iter().rev().find(|(n, _)| n == name) {
            Some(value.clone())
        } else if let Some(value) = self.variables.get(name) {
            Some(value.clone())
        } else if let Some(parent) = &self.parent {
            parent.get(name)
        } else {
            None
        }
    }

    pub fn set(&mut self, name: &str, value: Value) -> Result<(), InterpreterError> {
        if let Some(slot) = self.locals.iter_mut().rev().find(|(n, _)| n == name) {
            slot.1 = value;
            Ok(())
        } else if self.variables.contains_key(name) {
            Arc::make_mut(&mut self.variables).insert(name.to_string(), value);
            Ok(())
        } else if let Some(parent) = &mut self.parent {
            // Write through to the ancestor that owns the variable; shadowing
            // it locally would silently discard the assignment when this
            // scope is popped (e.g. `x = x + 1` inside a for-loop body)
            Arc::make_mut(parent).set(name, value)
        } else {
            Err(InterpreterError::UndefinedVariable {
                name: name.to_string(),
            })
        }
    }

    /// Get all variables in this environment (excluding parent environments)
    /// Returns a clone for compatibility with existing code
    pub fn get_all_variables(&self) -> HashMap<String, Value> {
        let mut all: HashMap<String, Value> = self
            .variables
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (name, value) in &self.locals {
            all.insert(name.clone(), value.clone());
        }
        all
    }

    /// This environment's own bindings as a persistent map: the flat map
    /// with call-frame locals overlaid. O(1) when there are no locals.
    fn flat_snapshot(&self) -> ImHashMap<String, Value> {
        let mut snapshot = (*self.variables).clone();
        for (name, value) in &self.locals {
            snapshot.insert(name.clone(), value.clone());
        }
        snapshot
    }

    /// Reserve capacity - no-op for ImHashMap (it grows automatically)
    pub fn reserve(&mut self, _additional: usize) {
        // ImHashMap handles capacity automatically
    }

    /// Remove a variable from this environment (for scoped cleanup)
    pub fn remove_variable(&mut self, name: &str) {
        self.locals.retain(|(n, _)| n != name);
        Arc::make_mut(&mut self.variables).remove(name);
    }

    /// Get ownership of all variables in this environment (for scoped operations)
    pub fn into_variables(self) -> HashMap<String, Value> {
        Arc::try_unwrap(self.variables)
            .unwrap_or_else(|arc| (*arc).clone())
            .into_iter()
            .collect()
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
    module_loading_stack: Vec<String>, // Feature 7: Track modules currently being loaded for circular detection

    // Feature 8: Smart caching system
    smart_cache_config: SmartCacheConfig,
    persistent_cache_manager: Option<PersistentCacheManager>,
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
    bytecode_tier: Option<crate::ovm::tier::BytecodeTier>,

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
        let persistent_cache_manager = if smart_cache_config.enable_persistent_cache {
            PersistentCacheManager::new(smart_cache_config.clone()).ok()
        } else {
            None
        };

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
            module_loading_stack: Vec::new(), // Feature 7: Track modules currently being loaded for circular detection

            // Feature 8: Smart caching system
            smart_cache_config,
            persistent_cache_manager,
            cache_statistics: CacheStatistics::default(),
            error_formatter: IntuitiveErrorFormatter::default(),

            // MEMORY PROTECTION: Initialize recursion depth tracking
            call_depth: 0,
            max_call_depth: 1000, // Reasonable limit to prevent stack overflow

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
                last_modified: Some(std::time::SystemTime::now()),
                dependencies: vec![],
                content_hash,
                compilation_time: std::time::Duration::default(),
                access_count: 0,
                last_accessed: std::time::SystemTime::now(),
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
        if let Some(ref mut type_checker) = self.type_checker {
            if let Err(type_errors) = type_checker.check_program(&program) {
                // Convert type checking errors to proper InterpreterError
                let error_messages: Vec<String> =
                    type_errors.iter().map(|e| format!("{:?}", e)).collect();
                let combined_message = error_messages.join("; ");

                return Err(InterpreterError::TypeError {
                    message: format!("Type checking failed: {}", combined_message),
                });
            }
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
            self.trait_impls
                .insert((impl_decl.type_name.clone(), method.name.clone()), function);
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
        _error_type_decl: crate::ast::ErrorTypeDecl,
    ) -> Result<Value, InterpreterError> {
        // For now, error type declarations don't produce runtime values
        // In a full implementation, we'd store error type information for later use
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
                    if !is_struct_field {
                        if let Some(method) = self.lookup_method(&receiver.type_name(), field) {
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
                    _ => Err(InterpreterError::TypeError {
                        message: "Try-catch can only be used on Result values".to_string(),
                    }),
                }
            }
            Expr::ForLoop {
                variable,
                iterable,
                body,
            } => self.eval_for_loop(variable, iterable, body),
            Expr::WhileLoop { condition, body } => self.eval_while_loop(condition, body),
            Expr::Loop { body } => self.eval_loop(body),
            Expr::Break => Err(InterpreterError::BreakSignal),
            Expr::Continue => Err(InterpreterError::ContinueSignal),
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
                    Err(e) => Err(InterpreterError::RuntimeError {
                        message: format!("Promise rejected: {:?}", e),
                    }),
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
                                    let deadline = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_millis() as u64)
                                        .unwrap_or(0)
                                        .saturating_add(ms as u64);
                                    Ok(Value::Promise {
                                        state: crate::ast::PromiseState::Pending,
                                        value: Some(Box::new(evaluated_value)),
                                        error: None,
                                        resolve_at_epoch_ms: Some(deadline),
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
                // Enhanced spawn implementation - evaluate expression asynchronously
                // For now, just evaluate the expression and wrap in resolved promise
                // In full implementation, would execute in separate task
                let result = self.eval_expr(expression)?;
                Ok(self.async_runtime.promise_resolve(result))
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
        self.bytecode_tier =
            Some(crate::ovm::tier::BytecodeTier::new(threshold).with_verbose(verbose));
    }

    pub fn bytecode_tier_stats(&self) -> Option<crate::ovm::tier::TierStats> {
        self.bytecode_tier.as_ref().map(|t| t.stats())
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
                let result = self.eval_expr_with_env(&func.body, new_env);

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
            lazy_config: self.lazy_config.clone(),
            safepoint_manager: self.safepoint_manager.clone(),
            module_debug_config: self.module_debug_config.clone(),

            // Enhanced module system
            module_cache: self.module_cache.clone(),
            dependency_tracker: self.dependency_tracker.clone(),
            current_module_path: self.current_module_path.clone(), // For tracking current module during loading
            module_loading_stack: Vec::new(), // Feature 7: Each thread gets its own loading stack

            // Feature 8: Smart caching system
            smart_cache_config: self.smart_cache_config.clone(),
            persistent_cache_manager: None, // Each thread manages its own cache connections
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

            // Each thread profiles independently; the VM is not shared
            bytecode_tier: None,
            trait_impls: self.trait_impls.clone(),
            trait_defaults: self.trait_defaults.clone(),
            type_traits: self.type_traits.clone(),
            unit_variant_names: self.unit_variant_names.clone(),
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

    fn pattern_matches_bind(
        &self,
        pattern: &Pattern,
        value: &Value,
        bindings: &mut HashMap<String, Value>,
    ) -> Result<bool, InterpreterError> {
        match (pattern, value) {
            (Pattern::Literal(lit), val) => Ok(lit == val),
            (Pattern::Identifier(name), val) => {
                // A bare name that is a *declared* unit enum variant is a
                // variant pattern — match it by equality — rather than a fresh
                // binding that captures everything. Without this,
                // `match dir { North => .., East => .. }` would have `North`
                // bind and shadow every other arm. The declared-variant check
                // (not "does some in-scope value happen to be a unit variant")
                // is essential: a binding sub-pattern such as `b` in
                // `Concat(a, b)` must still bind even when a `b` already in
                // scope holds a unit variant.
                if self.unit_variant_names.contains(name) {
                    if let Some(variant @ Value::Enum { .. }) = self.environment.get(name) {
                        if matches!(
                            variant,
                            Value::Enum {
                                variant_data: EnumVariantData::Unit,
                                ..
                            }
                        ) {
                            return Ok(&variant == val);
                        }
                    }
                }
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
                        if patterns.len() != fields.len() {
                            return Ok(false);
                        }
                        // Positional patterns carry no field names, so match in
                        // field-name order — HashMap iteration order would make
                        // multi-field matches succeed or fail nondeterministically
                        let mut field_values: Vec<(&String, &Value)> = fields.iter().collect();
                        field_values.sort_by_key(|(name, _)| name.as_str());
                        for (p, (_, v)) in patterns.iter().zip(field_values.iter()) {
                            if !self.pattern_matches_bind(p, v, bindings)? {
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
            (Pattern::AnonymousStruct { field_patterns }, Value::Struct { fields, .. }) => {
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
            (
                Pattern::Range {
                    start,
                    end,
                    inclusive,
                },
                Value::Integer(n),
            ) => {
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
            (
                Pattern::Range {
                    start,
                    end,
                    inclusive,
                },
                Value::String(s),
            ) => {
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
            (Value::Integer(a), BinaryOp::Add, Value::Integer(b)) => a
                .checked_add(b)
                .map(Value::Integer)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: "Integer overflow in addition".to_string(),
                }),
            (Value::Float(a), BinaryOp::Add, Value::Float(b)) => Ok(Value::Float(a + b)),
            (Value::Integer(a), BinaryOp::Add, Value::Float(b)) => Ok(Value::Float(a as f64 + b)),
            (Value::Float(a), BinaryOp::Add, Value::Integer(b)) => Ok(Value::Float(a + b as f64)),
            (Value::Integer(a), BinaryOp::Subtract, Value::Integer(b)) => a
                .checked_sub(b)
                .map(Value::Integer)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: "Integer overflow in subtraction".to_string(),
                }),
            (Value::Float(a), BinaryOp::Subtract, Value::Float(b)) => Ok(Value::Float(a - b)),
            (Value::Integer(a), BinaryOp::Subtract, Value::Float(b)) => {
                Ok(Value::Float(a as f64 - b))
            }
            (Value::Float(a), BinaryOp::Subtract, Value::Integer(b)) => {
                Ok(Value::Float(a - b as f64))
            }
            (Value::Integer(a), BinaryOp::Multiply, Value::Integer(b)) => a
                .checked_mul(b)
                .map(Value::Integer)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: "Integer overflow in multiplication".to_string(),
                }),
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
                    // checked_div also rejects i64::MIN / -1, which overflows
                    a.checked_div(b).map(Value::Integer).ok_or_else(|| {
                        InterpreterError::RuntimeError {
                            message: "Integer overflow in division".to_string(),
                        }
                    })
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
                    // checked_rem also rejects i64::MIN % -1, which overflows
                    a.checked_rem(b).map(Value::Integer).ok_or_else(|| {
                        InterpreterError::RuntimeError {
                            message: "Integer overflow in modulo".to_string(),
                        }
                    })
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
            // Enum values compare structurally: same variant and payloads.
            // Value derives PartialEq, so this is the natural equality.
            (left @ Value::Enum { .. }, BinaryOp::Equal, right @ Value::Enum { .. }) => {
                Ok(Value::Boolean(left == right))
            }
            (left @ Value::Enum { .. }, BinaryOp::NotEqual, right @ Value::Enum { .. }) => {
                Ok(Value::Boolean(left != right))
            }
            (Value::Unit, BinaryOp::Equal, Value::Unit) => Ok(Value::Boolean(true)),
            (Value::Unit, BinaryOp::NotEqual, Value::Unit) => Ok(Value::Boolean(false)),
            (Value::Integer(a), BinaryOp::NotEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a != b))
            }
            (Value::Float(a), BinaryOp::NotEqual, Value::Float(b)) => Ok(Value::Boolean(a != b)),
            (Value::String(a), BinaryOp::NotEqual, Value::String(b)) => {
                Ok(Value::Boolean(*a != *b))
            }
            // Strings order lexicographically, matching the bytecode tier — so
            // character-range checks like `c >= "0" && c <= "9"` work and
            // strings sort. Ordering is by Unicode scalar value.
            (Value::String(a), BinaryOp::LessThan, Value::String(b)) => Ok(Value::Boolean(*a < *b)),
            (Value::String(a), BinaryOp::LessThanEqual, Value::String(b)) => {
                Ok(Value::Boolean(*a <= *b))
            }
            (Value::String(a), BinaryOp::GreaterThan, Value::String(b)) => {
                Ok(Value::Boolean(*a > *b))
            }
            (Value::String(a), BinaryOp::GreaterThanEqual, Value::String(b)) => {
                Ok(Value::Boolean(*a >= *b))
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
            (Value::List(a), BinaryOp::Add, Value::List(b)) => {
                let mut items = Vec::with_capacity(a.len() + b.len());
                items.extend(a.iter().cloned());
                items.extend(b.iter().cloned());
                Ok(Value::List(std::sync::Arc::from(items)))
            }
            _ => Err(InterpreterError::TypeError {
                message: "Invalid binary operation".to_string(),
            }),
        }
    }

    fn eval_unary_op(&self, op: UnaryOp, operand: Value) -> Result<Value, InterpreterError> {
        match (op, operand) {
            (UnaryOp::Negate, Value::Integer(n)) => {
                n.checked_neg()
                    .map(Value::Integer)
                    .ok_or_else(|| InterpreterError::RuntimeError {
                        message: "Integer overflow in negation".to_string(),
                    })
            }
            (UnaryOp::Negate, Value::Float(x)) => Ok(Value::Float(-x)),
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
            Value::Range {
                start,
                end,
                inclusive,
            } => {
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

        // Struct and union declarations don't yet produce runtime bindings;
        // struct values are built via struct-literal syntax.
        Ok(Value::Unit)
    }

    fn eval_struct_literal(
        &mut self,
        struct_literal: &crate::ast::StructLiteral,
    ) -> Result<Value, InterpreterError> {
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
            _ => Err(InterpreterError::TypeError {
                message: format!("Cannot iterate over {:?}", iterable_value),
            }),
        }
    }

    /// Run a loop body over an iterator of items, honoring break/continue.
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
                Err(InterpreterError::BreakSignal) => break,
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
                Err(InterpreterError::BreakSignal) => break,
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
                Err(InterpreterError::BreakSignal) => return Ok(last_value),
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
                if let Some(param_index) = params.iter().position(|p| &p.name == arg_name) {
                    if param_index < positional_count {
                        return Err(InterpreterError::RuntimeError {
                            message: format!(
                                "Argument '{}' specified both positionally and by name",
                                arg_name
                            ),
                        });
                    }
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

    /// Feature 8: Enhanced cached module retrieval with smart validation
    fn get_cached_module(
        &mut self,
        module_path: &str,
    ) -> Result<Option<ModuleCacheEntry>, InterpreterError> {
        // Check if we have a cached entry first (read-only check)
        let has_entry = self.module_cache.contains_key(module_path);
        if !has_entry {
            self.cache_statistics.cache_misses += 1;
            return self.load_from_persistent_cache(module_path);
        }

        // Get file path from the cached entry itself to avoid recursion
        // (calling resolve_module_path here would cause infinite recursion)
        let (should_validate, current_hash) =
            if let Some(entry) = self.module_cache.get(module_path) {
                if let Some(ref path) = entry.file_path {
                    if self.smart_cache_config.enable_content_hashing && path.exists() {
                        (true, Some(self.calculate_file_hash(path)?))
                    } else {
                        (false, None)
                    }
                } else {
                    (false, None)
                }
            } else {
                (false, None)
            };

        // Now safely access the cache entry
        if let Some(entry) = self.module_cache.get_mut(module_path) {
            // Update access statistics
            entry.access_count += 1;
            entry.last_accessed = SystemTime::now();
            self.cache_statistics.cache_hits += 1;

            // Validate if needed
            if should_validate {
                if let Some(hash) = current_hash {
                    if hash != entry.content_hash {
                        // Content changed, invalidate cache
                        self.cache_statistics.invalidations += 1;
                        self.module_cache.remove(module_path);
                        return Ok(None);
                    }
                }
            }

            Ok(Some(entry.clone()))
        } else {
            Ok(None)
        }
    }

    /// Feature 8: Load module from persistent cache
    fn load_from_persistent_cache(
        &mut self,
        module_path: &str,
    ) -> Result<Option<ModuleCacheEntry>, InterpreterError> {
        if let Some(ref cache_manager) = self.persistent_cache_manager {
            if let Some(mut entry) = cache_manager.load_cache_entry(module_path)? {
                // Validate persistent cache entry
                if self.is_persistent_cache_valid(&entry, module_path)? {
                    // Update access statistics
                    entry.access_count += 1;
                    entry.last_accessed = SystemTime::now();
                    self.cache_statistics.persistent_loads += 1;

                    // Store in memory cache for faster access
                    self.module_cache
                        .insert(module_path.to_string(), entry.clone());
                    Ok(Some(entry))
                } else {
                    // Persistent cache is stale
                    Ok(None)
                }
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    /// Feature 8: Validate persistent cache entry
    fn is_persistent_cache_valid(
        &self,
        entry: &ModuleCacheEntry,
        _module_path: &str,
    ) -> Result<bool, InterpreterError> {
        if let Some(file_path) = &entry.file_path {
            if self.smart_cache_config.enable_content_hashing {
                let current_hash = self.calculate_file_hash(file_path)?;
                Ok(current_hash == entry.content_hash)
            } else {
                // Fallback to timestamp validation
                if let Ok(metadata) = std::fs::metadata(file_path) {
                    if let Ok(modified) = metadata.modified() {
                        Ok(entry.last_modified.is_some_and(|lm| modified <= lm))
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(false)
                }
            }
        } else {
            // Stdlib modules are always valid
            Ok(true)
        }
    }

    /// Cache a module for future use with smart caching enhancements
    fn cache_module(
        &mut self,
        module_path: String,
        module: Value,
        file_path: Option<std::path::PathBuf>,
        dependencies: Vec<String>,
    ) -> Result<(), InterpreterError> {
        self.cache_module_with_options(
            module_path,
            module,
            file_path,
            dependencies,
            None,
            None,
            Duration::default(),
        )
    }

    /// Feature 8: Enhanced cache_module with smart caching options
    #[allow(clippy::too_many_arguments)] // caching knobs; a config struct is future work
    fn cache_module_with_options(
        &mut self,
        module_path: String,
        module: Value,
        file_path: Option<std::path::PathBuf>,
        dependencies: Vec<String>,
        ast_cache: Option<Program>,
        analysis_cache: Option<AnalysisReport>,
        compilation_time: Duration,
    ) -> Result<(), InterpreterError> {
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

        // Feature 8: Calculate content hash for smart invalidation
        let content_hash = if let Some(ref path) = file_path {
            self.calculate_file_hash(path)?
        } else {
            // For stdlib modules, use module path as hash
            self.calculate_string_hash(&module_path)
        };

        // Feature 8: Estimate memory usage
        let memory_size = self.estimate_module_memory_size(&module, &ast_cache, &analysis_cache);

        let cache_entry = ModuleCacheEntry {
            module,
            file_path,
            last_modified,
            dependencies,
            // Feature 8: Smart caching fields
            content_hash,
            compilation_time,
            access_count: 1,
            last_accessed: SystemTime::now(),
            cache_generation: 1,
            memory_size,
        };

        self.module_cache.insert(module_path.clone(), cache_entry);
        crate::log::get_logger().debug(
            "interpreter",
            &format!(
                "Smart cached module: {} ({}KB)",
                module_path,
                memory_size / 1024
            ),
        );
        Ok(())
    }

    /// Feature 8: Calculate SHA-256 hash of file content for change detection
    fn calculate_file_hash(&self, file_path: &std::path::Path) -> Result<String, InterpreterError> {
        use std::io::Read;

        // Virtual modules (embedded olang builtins, native stdlib structs)
        // have no file on disk. Their source is fixed for the life of the
        // binary, so hash the path — a stable, always-available identity.
        let as_str = file_path.to_string_lossy();
        if as_str.starts_with("__embedded__/") || as_str.starts_with("__stdlib__/") {
            return Ok(self.calculate_string_hash(&as_str));
        }

        let mut file =
            std::fs::File::open(file_path).map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to open file for hashing: {}", e),
            })?;

        let mut hasher = sha2::Sha256::new();
        let mut buffer = [0u8; 4096];

        loop {
            let bytes_read =
                file.read(&mut buffer)
                    .map_err(|e| InterpreterError::RuntimeError {
                        message: format!("Failed to read file for hashing: {}", e),
                    })?;

            if bytes_read == 0 {
                break;
            }

            hasher.update(&buffer[..bytes_read]);
        }

        Ok(format!("{:x}", hasher.finalize()))
    }

    /// Feature 8: Calculate hash of string content
    fn calculate_string_hash(&self, content: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Feature 8: Estimate memory usage of cached module data
    fn estimate_module_memory_size(
        &self,
        module: &Value,
        ast_cache: &Option<Program>,
        analysis_cache: &Option<AnalysisReport>,
    ) -> usize {
        let mut size = 0;

        // Estimate module value size (simplified)
        match module {
            Value::Struct { fields, .. } => {
                size += fields.len() * 64; // rough estimate per field
            }
            _ => size += 64, // base size
        }

        // Add AST cache size estimate
        if let Some(ast) = ast_cache {
            size += ast.statements.len() * 256; // rough estimate per statement
        }

        // Add analysis cache size estimate
        if let Some(_analysis) = analysis_cache {
            size += 1024; // rough estimate for analysis data
        }

        size
    }

    /// Bind module imports to current environment
    fn bind_module_imports(
        &mut self,
        module: &Value,
        items: &Option<Vec<crate::ast::UseItem>>,
    ) -> Result<(), InterpreterError> {
        match items {
            Some(item_list) => {
                // Check for wildcard imports
                let has_wildcard = item_list
                    .iter()
                    .any(|item| matches!(item, crate::ast::UseItem::Wildcard));

                if has_wildcard {
                    // Import all shared objects from the module
                    if let Value::Struct { fields, .. } = module {
                        for (name, value) in fields {
                            self.environment.define(name.clone(), value.clone());
                            crate::log::get_logger().debug(
                                "interpreter",
                                &format!("Wildcard imported {} from module", name),
                            );
                        }
                    } else {
                        return Err(InterpreterError::RuntimeError {
                            message: "Cannot perform wildcard import on non-struct module"
                                .to_string(),
                        });
                    }
                } else {
                    // Import specific items: use module { func1, func2 }
                    for item in item_list {
                        if let crate::ast::UseItem::Specific(item_name) = item {
                            if let Some(value) = self.get_module_export(module, item_name) {
                                // Importing an enum type name also brings its
                                // variant constructors into scope, so the ADT is
                                // usable for construction (`Text(..)`) and not
                                // only as a type reference. Variants are bare
                                // names with no qualified form to reach otherwise.
                                if let Value::TypeInfo {
                                    definition: crate::ast::TypeDefinition::Enum { variants },
                                    ..
                                } = &value
                                {
                                    for variant in variants {
                                        if let Some(ctor) =
                                            self.get_module_export(module, &variant.name)
                                        {
                                            self.environment.define(variant.name.clone(), ctor);
                                        }
                                    }
                                }
                                self.environment.define(item_name.clone(), value);
                                crate::log::get_logger().debug(
                                    "interpreter",
                                    &format!("Imported {} from module", item_name),
                                );
                            } else {
                                // Feature 9: Enhanced function not found error with suggestions
                                let mut available_functions = Vec::new();
                                let current_file = self.current_module_path.clone();

                                // Collect available functions from the module
                                if let Value::Struct { fields, .. } = module {
                                    available_functions.extend(fields.keys().cloned());
                                }

                                // Generate suggestions for the missing function
                                let suggestions = self
                                    .error_formatter
                                    .generate_suggestions(item_name, &available_functions);

                                // Get the module path from the current context
                                let module_path = self
                                    .current_module_path
                                    .clone()
                                    .unwrap_or_else(|| "unknown_module".to_string());

                                return Err(InterpreterError::FunctionNotFoundInModule {
                                    function_name: item_name.clone(),
                                    module_path,
                                    available_functions,
                                    suggestions,
                                    file_path: current_file,
                                });
                            }
                        }
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

    /// Feature 8: Enhanced module cache clearing with persistent cache cleanup
    pub fn clear_module_cache(&mut self) {
        self.module_cache.clear();
        self.dependency_tracker = ModuleDependencyTracker::new();
        self.cache_statistics = CacheStatistics::default();
        crate::log::get_logger().debug("interpreter", "Smart module cache cleared");

        // Optionally clear persistent cache
        if let Some(ref mut cache_manager) = self.persistent_cache_manager {
            if cache_manager.cleanup_cache().is_ok() {
                crate::log::get_logger().debug("interpreter", "Persistent cache cleaned up");
            }
        }
    }

    /// Feature 8: Get comprehensive cache statistics
    pub fn get_smart_cache_statistics(&mut self) -> CacheStatistics {
        // Update memory usage statistics
        self.cache_statistics.total_memory_usage = self.calculate_total_cache_memory();

        // Calculate cache efficiency
        let total_requests = self.cache_statistics.cache_hits + self.cache_statistics.cache_misses;
        self.cache_statistics.cache_efficiency = if total_requests > 0 {
            (self.cache_statistics.cache_hits as f64 / total_requests as f64) * 100.0
        } else {
            0.0
        };

        // Calculate average compilation time
        if !self.module_cache.is_empty() {
            let total_time: Duration = self
                .module_cache
                .values()
                .map(|entry| entry.compilation_time)
                .sum();
            self.cache_statistics.average_compilation_time =
                total_time / self.module_cache.len() as u32;
        }

        self.cache_statistics.clone()
    }

    /// Feature 8: Calculate total memory usage of cached data
    fn calculate_total_cache_memory(&self) -> usize {
        self.module_cache
            .values()
            .map(|entry| entry.memory_size)
            .sum()
    }

    /// Feature 8: Intelligent cache cleanup based on usage patterns
    pub fn perform_intelligent_cache_cleanup(
        &mut self,
    ) -> Result<CacheCleanupStats, InterpreterError> {
        let mut cleanup_stats = CacheCleanupStats::default();
        let start_time = Instant::now();

        // Get cache size limits
        let max_memory_mb = self.smart_cache_config.max_cache_size_mb;
        let max_entries = self.smart_cache_config.max_cache_entries;
        let current_memory = self.calculate_total_cache_memory();
        let current_entries = self.module_cache.len();

        // Check if cleanup is needed
        if current_memory > max_memory_mb * 1024 * 1024 || current_entries > max_entries {
            // Collect entries with their scores for sorting
            let mut entries_with_scores: Vec<_> = self
                .module_cache
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        self.calculate_cache_priority_score(v),
                        v.memory_size,
                    )
                })
                .collect();
            entries_with_scores
                .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

            // Remove least important entries
            let target_entries = max_entries * 80 / 100; // Keep 80% of max
            let entries_to_remove = current_entries.saturating_sub(target_entries);

            for (module_path, _score, memory_size) in
                entries_with_scores.iter().take(entries_to_remove)
            {
                cleanup_stats.bytes_freed += memory_size;
                cleanup_stats.files_removed += 1;
                self.module_cache.remove(module_path);
            }
        }

        // Clean up persistent cache
        if let Some(ref mut cache_manager) = self.persistent_cache_manager {
            let persistent_stats = cache_manager.cleanup_cache()?;
            cleanup_stats.bytes_freed += persistent_stats.bytes_freed;
            cleanup_stats.files_removed += persistent_stats.files_removed;
        }

        cleanup_stats.cleanup_time = start_time.elapsed();
        self.cache_statistics.cleanup_operations += 1;

        Ok(cleanup_stats)
    }

    /// Feature 8: Calculate priority score for cache entry (higher = keep longer)
    fn calculate_cache_priority_score(&self, entry: &ModuleCacheEntry) -> f64 {
        let _now = SystemTime::now();
        let hours_since_access = entry
            .last_accessed
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs() as f64
            / 3600.0;

        let recency_score = 1.0 / (1.0 + hours_since_access);
        let frequency_score = (entry.access_count as f64).ln();
        let compilation_cost_score = entry.compilation_time.as_millis() as f64 / 1000.0;

        // Combine scores: recent + frequent + expensive to compile = higher priority
        recency_score * 0.4 + frequency_score * 0.4 + compilation_cost_score * 0.2
    }

    /// Feature 8: Prepare cache directory structure (persistent caching disabled for thread safety)
    pub fn save_cache_to_persistent_storage(&mut self) -> Result<(), InterpreterError> {
        if let Some(ref mut cache_manager) = self.persistent_cache_manager {
            for (module_path, entry) in &self.module_cache {
                cache_manager.save_cache_entry(module_path, entry)?;
                self.cache_statistics.persistent_saves += 1;
            }
        }
        Ok(())
    }

    /// Get dependency information for a module
    pub fn get_module_dependencies(&self, module_path: &str) -> Vec<String> {
        self.dependency_tracker
            .dependencies
            .get(module_path)
            .cloned()
            .unwrap_or_default()
    }

    /// Get modules that depend on a given module
    pub fn get_module_dependents(&self, module_path: &str) -> Vec<String> {
        self.dependency_tracker
            .dependents
            .get(module_path)
            .cloned()
            .unwrap_or_default()
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
    fn collect_dependency_chain(
        &self,
        module_path: &str,
        chain: &mut Vec<String>,
        visited: &mut HashSet<String>,
    ) {
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
        crate::log::get_logger().debug(
            "interpreter",
            &format!("Invalidated module: {}", module_path),
        );

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
        let total_dependencies = self
            .dependency_tracker
            .dependencies
            .values()
            .map(|v| v.len())
            .sum();
        let total_dependents = self
            .dependency_tracker
            .dependents
            .values()
            .map(|v| v.len())
            .sum();

        (cached_modules, total_dependencies, total_dependents)
    }

    /// Export module dependency information for debugging
    pub fn export_dependency_graph(&self) -> HashMap<String, Vec<String>> {
        self.dependency_tracker.dependencies.clone()
    }

    /// Feature 7: Get current module loading stack (for debugging)
    pub fn get_module_loading_stack(&self) -> &Vec<String> {
        &self.module_loading_stack
    }

    /// Feature 7: Check if module loading stack is empty
    pub fn is_module_loading_stack_empty(&self) -> bool {
        self.module_loading_stack.is_empty()
    }

    /// Feature 7: Detect all potential circular dependencies in the current dependency graph
    pub fn detect_all_circular_dependencies(&self) -> Vec<Vec<String>> {
        self.dependency_tracker.find_all_cycles()
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

    /// Load a module from the file system or standard library
    fn load_module_from_file(&mut self, module_path: &str) -> Result<Value, InterpreterError> {
        // Feature 7: Check for circular dependency before loading
        if self.module_loading_stack.contains(&module_path.to_string()) {
            // Build cycle path for clear error message
            let cycle_start = self
                .module_loading_stack
                .iter()
                .position(|m| m == module_path)
                .unwrap_or(0);
            let mut cycle_path = self.module_loading_stack[cycle_start..].to_vec();
            cycle_path.push(module_path.to_string());

            return Err(InterpreterError::CircularDependencyDetected {
                cycle_path: cycle_path.join(" → "),
            });
        }

        // Check cache first
        if let Ok(Some(cached)) = self.get_cached_module(module_path) {
            return Ok(cached.module);
        }

        // Determine the file path
        let file_path = self.resolve_module_path(module_path)?;

        // Handle standard library modules
        if file_path.to_string_lossy().starts_with("__stdlib__/") {
            let stdlib_name = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: format!("Invalid stdlib module path: {}", file_path.display()),
                })?;

            let stdlib = crate::stdlib::get_stdlib();
            if let Some(module) = stdlib.get(stdlib_name) {
                // Cache the stdlib module
                self.cache_module(module_path.to_string(), module.clone(), None, Vec::new())?;
                return Ok(module.clone());
            } else {
                return Err(InterpreterError::RuntimeError {
                    message: format!("Standard library module '{}' not found", stdlib_name),
                });
            }
        }

        // Feature 8: Start timing for compilation metrics
        let start_time = Instant::now();

        // The module source is either an embedded olang builtin (compiled into
        // the binary) or a file on disk.
        let content = if let Some(name) = file_path
            .to_string_lossy()
            .strip_prefix("__embedded__/")
            .map(|s| s.to_string())
        {
            crate::stdlib::embedded::source(&name)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: format!("Embedded module '{}' not found", name),
                })?
                .to_string()
        } else {
            std::fs::read_to_string(&file_path).map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to read module file {}: {}", file_path.display(), e),
            })?
        };

        // Parse the module
        let parser = crate::parser::Parser::new();
        let program = parser
            .parse(&content)
            .map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to parse module {}: {:?}", file_path.display(), e),
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

        // Feature 7: Add module to loading stack to track circular dependencies
        self.module_loading_stack.push(module_path.to_string());
        let stack_size_before = self.module_loading_stack.len();

        // Save current environment and module path
        let saved_env = std::mem::replace(&mut self.environment, module_env);
        let saved_module_path = self.current_module_path.clone();

        // Set current module path to the FULL file path (not just module name)
        // This allows nested imports to resolve relative to this file's directory
        let file_path_str = file_path.to_string_lossy().to_string();
        self.current_module_path = Some(file_path_str.clone());

        // Pre-cache a placeholder entry so nested imports can find this module's directory
        // The full module value will be added after the module is fully loaded
        let content_hash = if file_path.exists() {
            self.calculate_file_hash(&file_path).unwrap_or_default()
        } else {
            String::new()
        };
        let placeholder_entry = ModuleCacheEntry {
            module: Value::Unit, // Placeholder until full load
            file_path: Some(file_path.clone()),
            last_modified: file_path.metadata().ok().and_then(|m| m.modified().ok()),
            dependencies: vec![],
            content_hash,
            compilation_time: std::time::Duration::default(),
            access_count: 0,
            last_accessed: SystemTime::now(),
            cache_generation: 0,
            memory_size: 0,
        };
        self.module_cache.insert(file_path_str, placeholder_entry);

        // Execute the module and collect exports
        let result = {
            let mut exports = std::collections::HashMap::new();
            let mut dependencies = Vec::new();

            // Process all statements in the module
            for statement in &program.statements {
                match statement {
                    crate::ast::Statement::ShareDecl(share_decl) => {
                        match share_decl {
                            ShareDecl::Function(func_decl) => {
                                let value = self.eval_function_decl(func_decl.clone())?;
                                exports.insert(func_decl.name.clone(), value);
                            }
                            ShareDecl::Let(let_decl) => {
                                let value = self.eval_let_decl(let_decl)?;
                                if let Pattern::Identifier(name) = &let_decl.pattern {
                                    exports.insert(name.clone(), value);
                                }
                            }
                            ShareDecl::Type(type_decl) => {
                                self.eval_type_decl(type_decl.clone())?;
                                // Export type information as a special Type value
                                let type_info = Value::TypeInfo {
                                    name: type_decl.name.clone(),
                                    definition: type_decl.definition.clone(),
                                };
                                exports.insert(type_decl.name.clone(), type_info);

                                // For an enum, also export each variant
                                // constructor. `eval_type_decl` bound them into
                                // the module environment; exporting them lets an
                                // importer *construct* a shared ADT (`Text("x")`),
                                // not just pattern-match it. Patterns use bare
                                // names and never needed the import, but there is
                                // no qualified `Type::Variant` syntax, so the bare
                                // constructor must cross the module boundary.
                                if let crate::ast::TypeDefinition::Enum { variants } =
                                    &type_decl.definition
                                {
                                    for variant in variants {
                                        if let Some(ctor) = self.environment.get(&variant.name) {
                                            exports.insert(variant.name.clone(), ctor);
                                        }
                                    }
                                }
                            }
                            // Traits/impls register globally (not as exports);
                            // loading this module is enough to make them apply.
                            ShareDecl::Trait(trait_decl) => {
                                self.eval_trait_decl(trait_decl.clone())?;
                            }
                            ShareDecl::Impl(impl_decl) => {
                                self.eval_impl_decl(impl_decl.clone())?;
                            }
                            ShareDecl::Use(use_decl) => {
                                // Handle transitive sharing: re-export items from another module
                                let dep_module_path = use_decl.path.join(".");
                                let module = self.load_module_from_file(&dep_module_path)?;

                                // Re-export the specified items
                                for item in &use_decl.items {
                                    match item {
                                        crate::ast::UseItem::Specific(name) => {
                                            if let Some(value) =
                                                self.get_module_export(&module, name)
                                            {
                                                exports.insert(name.clone(), value);
                                            }
                                        }
                                        crate::ast::UseItem::Wildcard => {
                                            // For wildcard re-exports, import all exports from the module
                                            if let Value::Struct { fields, .. } = &module {
                                                for (name, value) in fields {
                                                    exports.insert(name.clone(), value.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    crate::ast::Statement::UseDecl(use_decl) => {
                        dependencies.push(use_decl.path.join("."));
                        self.eval_statement(&crate::ast::Statement::UseDecl(use_decl.clone()))?;
                    }
                    _ => {
                        self.eval_statement(statement)?;
                    }
                }
            }

            // Create module struct
            // Re-close every exported function over the *complete* module
            // environment. A function's closure is snapshotted when it is
            // declared, so a shared function that calls a private helper
            // declared later in the file wouldn't otherwise see it. After the
            // whole module has run, module_env holds every binding (shared and
            // private); rebinding exported functions' closures to it gives them
            // full visibility of their siblings — matching how top-level
            // functions in a single file can call one another regardless of
            // order.
            let module_scope = self.environment.flat_snapshot();
            for value in exports.values_mut() {
                if let Value::Function(func) = value {
                    func.closure = Arc::new(module_scope.clone());
                }
            }

            let module = Value::Struct {
                type_name: "Module".to_string(),
                fields: exports,
            };

            // Feature 8: Cache the module with smart caching enhancements
            let compilation_time = start_time.elapsed();
            self.cache_module_with_options(
                module_path.to_string(),
                module.clone(),
                Some(file_path),
                dependencies,
                Some(program.clone()), // Cache the parsed AST
                None,                  // Analysis cache can be added later
                compilation_time,
            )?;

            Ok(module)
        };

        // Restore original environment and module path
        self.environment = saved_env;
        self.current_module_path = saved_module_path;

        // Feature 7: Clean up loading stack (ensure we only remove the module we added)
        if self.module_loading_stack.len() >= stack_size_before {
            self.module_loading_stack.truncate(stack_size_before - 1);
        }

        result
    }

    /// Resolve module path to actual file path using dependency-free discovery
    ///
    /// Discovery Rules (Feature 4):
    /// 1. Relative to current file - `use sibling_file { function }`
    /// 2. Relative to project root - `use utils.helpers { function }`
    /// 3. Standard library - `use std.io { println }` (built-in)
    /// 4. Current directory first - Always check same folder first
    fn resolve_module_path(
        &mut self,
        module_path: &str,
    ) -> Result<std::path::PathBuf, InterpreterError> {
        let debug_config = self.module_debug_config.clone();
        let _start_time = if debug_config.show_resolution_timing {
            Some(Instant::now())
        } else {
            None
        };

        if debug_config.enable_resolution_tracing {
            crate::log::get_logger().debug(
                "interpreter",
                &format!(
                    "Resolving module: '{}' using dependency-free discovery",
                    module_path
                ),
            );
        }

        // Embedded olang-source stdlib modules (builtin packages compiled
        // into the binary) resolve to a special path handled below.
        if crate::stdlib::embedded::is_embedded(module_path) {
            return Ok(std::path::PathBuf::from(format!(
                "__embedded__/{}",
                module_path
            )));
        }

        // Package dependencies win first: `use foo.bar` where `foo` is a
        // declared dependency resolves inside that dependency's directory.
        if let Ok(path) = self.discover_module_dependency(module_path) {
            if debug_config.enable_resolution_tracing {
                crate::log::get_logger().debug(
                    "interpreter",
                    &format!("Found in dependency: {}", path.display()),
                );
            }
            return Ok(path);
        }

        // Try discovery algorithms in order of priority
        if let Ok(path) = self.discover_module_same_directory(module_path) {
            if debug_config.enable_resolution_tracing {
                crate::log::get_logger().debug(
                    "interpreter",
                    &format!("Found in same directory: {}", path.display()),
                );
            }
            return Ok(path);
        }

        if let Ok(path) = self.discover_module_project_root(module_path) {
            if debug_config.enable_resolution_tracing {
                crate::log::get_logger().debug(
                    "interpreter",
                    &format!("Found relative to project root: {}", path.display()),
                );
            }
            return Ok(path);
        }

        if let Ok(path) = self.discover_module_stdlib(module_path) {
            if debug_config.enable_resolution_tracing {
                crate::log::get_logger().debug(
                    "interpreter",
                    &format!("Found in standard library: {}", path.display()),
                );
            }
            return Ok(path);
        }

        // Enhanced error with discovery information
        self.create_module_not_found_error(module_path, &debug_config)
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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(u64::MAX);
        if deadline > now {
            std::thread::sleep(std::time::Duration::from_millis(deadline - now));
        }
    }

    pub fn set_dependency_map(&mut self, map: HashMap<String, std::path::PathBuf>) {
        self.dependency_map = map;
    }

    /// Merge entries into the dependency map without clearing existing ones,
    /// so several packages can be made available by path (e.g. `:pkg load`).
    pub fn add_to_dependency_map(&mut self, map: HashMap<String, std::path::PathBuf>) {
        self.dependency_map.extend(map);
    }

    /// If the first segment of the module path is a declared dependency,
    /// resolve the remaining path inside that dependency's directory. A bare
    /// `use foo` resolves to the dependency's package root (its index.ol,
    /// mod.ol, or foo.ol).
    fn discover_module_dependency(
        &mut self,
        module_path: &str,
    ) -> Result<std::path::PathBuf, InterpreterError> {
        let mut parts = module_path.split('.');
        let head = parts.next().unwrap_or("");
        let dep_dir = self.dependency_map.get(head).cloned().ok_or_else(|| {
            InterpreterError::RuntimeError {
                message: format!("'{}' is not a dependency", head),
            }
        })?;

        let rest: Vec<&str> = parts.collect();
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if rest.is_empty() {
            // `use foo` -> the package's public root
            candidates.push(dep_dir.join("index.ol"));
            candidates.push(dep_dir.join("mod.ol"));
            candidates.push(dep_dir.join(format!("{}.ol", head)));
            candidates.push(dep_dir.join("src").join("index.ol"));
        } else {
            // `use foo.bar.baz` -> foo/{bar/baz.ol, bar/baz/index.ol}
            let sub = rest.join("/");
            candidates.push(dep_dir.join(format!("{}.ol", sub)));
            candidates.push(dep_dir.join(&sub).join("index.ol"));
            candidates.push(dep_dir.join(&sub).join("mod.ol"));
            candidates.push(dep_dir.join("src").join(format!("{}.ol", sub)));
        }

        for candidate in candidates {
            if candidate.exists() {
                return Ok(candidate);
            }
        }
        Err(InterpreterError::RuntimeError {
            message: format!(
                "module '{}' not found in dependency '{}'",
                module_path, head
            ),
        })
    }

    /// 1. Check same directory as current file
    fn discover_module_same_directory(
        &mut self,
        module_path: &str,
    ) -> Result<std::path::PathBuf, InterpreterError> {
        let current_file_dir = if let Some(current_module) = self.current_module_path.clone() {
            // If we're loading from within a module, use that module's directory
            if let Ok(cached) = self.get_cached_module(&current_module) {
                if let Some(cached_entry) = cached {
                    if let Some(ref file_path) = cached_entry.file_path {
                        file_path
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new("."))
                            .to_path_buf()
                    } else {
                        std::env::current_dir().map_err(|e| InterpreterError::RuntimeError {
                            message: format!("Failed to get current directory: {}", e),
                        })?
                    }
                } else {
                    std::env::current_dir().map_err(|e| InterpreterError::RuntimeError {
                        message: format!("Failed to get current directory: {}", e),
                    })?
                }
            } else {
                std::env::current_dir().map_err(|e| InterpreterError::RuntimeError {
                    message: format!("Failed to get current directory: {}", e),
                })?
            }
        } else {
            // No current module context, use current working directory
            std::env::current_dir().map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to get current directory: {}", e),
            })?
        };

        self.try_resolve_in_directory(&current_file_dir, module_path)
    }

    /// 2. Check relative to project root
    fn discover_module_project_root(
        &self,
        module_path: &str,
    ) -> Result<std::path::PathBuf, InterpreterError> {
        let project_root = self.detect_project_root()?;
        self.try_resolve_in_directory(&project_root, module_path)
    }

    /// 3. Check standard library
    fn discover_module_stdlib(
        &self,
        module_path: &str,
    ) -> Result<std::path::PathBuf, InterpreterError> {
        // Check if this is a stdlib module
        let stdlib = crate::stdlib::get_stdlib();
        if stdlib.contains_key(module_path) {
            // Return a special path that indicates this is a stdlib module
            // We'll handle this specially in load_module_from_file
            return Ok(std::path::PathBuf::from(format!(
                "__stdlib__/{}",
                module_path
            )));
        }

        // Check for std.* prefix
        if let Some(stdlib_name) = module_path.strip_prefix("std.") {
            // Remove "std." prefix
            if stdlib.contains_key(stdlib_name) {
                return Ok(std::path::PathBuf::from(format!(
                    "__stdlib__/{}",
                    stdlib_name
                )));
            }
        }

        Err(InterpreterError::RuntimeError {
            message: format!("Not a standard library module: {}", module_path),
        })
    }

    /// Detect project root by looking for common project indicators
    fn detect_project_root(&self) -> Result<std::path::PathBuf, InterpreterError> {
        let current_dir = std::env::current_dir().map_err(|e| InterpreterError::RuntimeError {
            message: format!("Failed to get current directory: {}", e),
        })?;

        // Look for project indicators in current and parent directories
        let mut dir = current_dir;
        loop {
            // Check for common project files
            let indicators = vec![
                "Cargo.toml",       // Rust project
                "package.json",     // Node.js project
                "requirements.txt", // Python project
                "go.mod",           // Go project
                ".git",             // Git repository
                "olang.toml",       // Future Olang project file
                "main.ol",          // Olang entry point
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
                return std::env::current_dir().map_err(|e| InterpreterError::RuntimeError {
                    message: format!("Failed to get current directory: {}", e),
                });
            }
        }
    }

    /// Try to resolve a module in a specific directory
    fn try_resolve_in_directory(
        &self,
        base_dir: &std::path::Path,
        module_path: &str,
    ) -> Result<std::path::PathBuf, InterpreterError> {
        // Handle dotted paths by splitting on . and joining with /
        let file_parts: Vec<String> = module_path.split('.').map(|s| s.to_string()).collect();
        let file_name = format!("{}.ol", file_parts.last().unwrap_or(&String::new()));
        let dir_path = if file_parts.len() > 1 {
            file_parts[..file_parts.len() - 1].join("/")
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
        } else {
            candidates.push(base_dir.join(&file_name));

            // Importing a directory directly (e.g., `use utils { ... }`)
            // resolves to a user-written index.ol / mod.ol — never a generated
            // one.
            let target_dir = base_dir.join(module_path);
            if target_dir.exists() && target_dir.is_dir() {
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
        } else {
            candidates.push(base_dir.join("src").join(&file_name));
        }

        // Check each candidate
        for candidate in &candidates {
            if candidate.exists() && candidate.is_file() {
                return Ok(candidate.clone());
            }
        }

        Err(InterpreterError::RuntimeError {
            message: format!(
                "Module '{}' not found in directory {}",
                module_path,
                base_dir.display()
            ),
        })
    }

    /// Feature 9: Enhanced module not found error with detailed context and suggestions
    fn create_module_not_found_error(
        &self,
        module_path: &str,
        _debug_config: &ModuleDebugConfig,
    ) -> Result<std::path::PathBuf, InterpreterError> {
        let mut searched_paths = Vec::new();
        let mut available_modules = Vec::new();

        // Collect search paths
        if let Ok(current_dir) = std::env::current_dir() {
            searched_paths.push(format!("Same directory: {}", current_dir.display()));
        }

        if let Ok(project_root) = self.detect_project_root() {
            searched_paths.push(format!("Project root: {}", project_root.display()));
        }

        searched_paths.push("Standard library modules".to_string());

        // Get available modules from stdlib
        let stdlib = crate::stdlib::get_stdlib();
        available_modules.extend(stdlib.keys().map(|s| s.to_string()));

        // Get available modules from current directory (if any .ol files exist)
        if let Ok(current_dir) = std::env::current_dir() {
            if let Ok(entries) = std::fs::read_dir(&current_dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.ends_with(".ol") && name != "main.ol" {
                            available_modules.push(name.trim_end_matches(".ol").to_string());
                        }
                    }
                }
            }
        }

        // Generate suggestions using the error formatter
        let suggestions = self
            .error_formatter
            .generate_suggestions(module_path, &available_modules);

        Err(InterpreterError::ModuleNotFound {
            module_path: module_path.to_string(),
            searched_paths,
            available_modules,
            suggestions,
        })
    }

    /// Get an export from a loaded module
    fn get_module_export(&self, module: &Value, export_name: &str) -> Option<Value> {
        match module {
            Value::Struct { fields, .. } => fields.get(export_name).cloned(),
            _ => None,
        }
    }

    fn eval_use_decl(&mut self, use_decl: UseDecl) -> Result<Value, InterpreterError> {
        let module_path = use_decl.path.join(".");
        let module = self.load_module_from_file(&module_path)?;
        self.bind_module_imports(&module, &Some(use_decl.items))?;

        // Also bind the module's own name as a namespace, so an imported
        // module is inspectable and callable as `name.fn(...)` — matching the
        // native stdlib modules (`col`, `math`, ...), which are always bound.
        if let Some(leaf) = use_decl.path.last() {
            // Don't clobber an existing binding of the same name (e.g. a
            // native module the user also referenced).
            if self.environment.get(leaf).is_none() {
                self.environment.define(leaf.clone(), module.clone());
            }
        }

        Ok(Value::Unit)
    }

    fn eval_test_decl(&mut self, test_decl: TestDecl) -> Result<Value, InterpreterError> {
        // For now, we'll just evaluate the test body and return Unit
        // In a full implementation, this would be part of the test runner
        for statement in &test_decl.body {
            self.eval_statement(statement)?;
        }
        Ok(Value::Unit)
    }
}

/// Enhanced module cache entry with smart caching features
#[derive(Debug, Clone)]
pub struct ModuleCacheEntry {
    pub module: Value,
    pub file_path: Option<std::path::PathBuf>,
    pub last_modified: Option<SystemTime>,
    pub dependencies: Vec<String>,

    // Feature 8: Smart caching enhancements (simplified for thread safety)
    pub content_hash: String,       // SHA-256 hash of file content
    pub compilation_time: Duration, // Time taken to compile this module
    pub access_count: u64,          // How often this module is accessed
    pub last_accessed: SystemTime,  // When this module was last accessed
    pub cache_generation: u64,      // Cache generation for cleanup
    pub memory_size: usize,         // Estimated memory usage of cached data
}

/// Enhanced module dependency tracking with smart invalidation
#[derive(Debug, Clone)]
pub struct ModuleDependencyTracker {
    pub dependencies: HashMap<String, Vec<String>>, // module -> its dependencies
    pub dependents: HashMap<String, Vec<String>>,   // module -> modules that depend on it

    // Feature 8: Smart dependency tracking
    pub dependency_timestamps: HashMap<String, SystemTime>, // module -> when it was last compiled
    pub dependency_hashes: HashMap<String, String>,         // module -> content hash
    pub invalidation_queue: Vec<String>,                    // modules pending invalidation
    pub dependency_graph_hash: String,                      // hash of entire dependency graph
}

impl Default for ModuleDependencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleDependencyTracker {
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
            dependency_timestamps: HashMap::new(),
            dependency_hashes: HashMap::new(),
            invalidation_queue: Vec::new(),
            dependency_graph_hash: String::new(),
        }
    }

    pub fn add_dependency(&mut self, module: String, dependency: String) {
        self.dependencies
            .entry(module.clone())
            .or_default()
            .push(dependency.clone());
        self.dependents.entry(dependency).or_default().push(module);
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

    /// Feature 7: Find all cycles in the dependency graph using DFS
    pub fn find_all_cycles(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut current_path = Vec::new();

        for module in self.dependencies.keys() {
            if !visited.contains(module) {
                self.dfs_find_cycles(
                    module,
                    &mut visited,
                    &mut rec_stack,
                    &mut current_path,
                    &mut cycles,
                );
            }
        }

        cycles
    }

    /// DFS helper for cycle detection
    fn dfs_find_cycles(
        &self,
        module: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        current_path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(module.to_string());
        rec_stack.insert(module.to_string());
        current_path.push(module.to_string());

        if let Some(deps) = self.dependencies.get(module) {
            for dep in deps {
                if !visited.contains(dep) {
                    self.dfs_find_cycles(dep, visited, rec_stack, current_path, cycles);
                } else if rec_stack.contains(dep) {
                    // Found a cycle - extract the cycle from current_path
                    if let Some(cycle_start) = current_path.iter().position(|m| m == dep) {
                        let mut cycle = current_path[cycle_start..].to_vec();
                        cycle.push(dep.to_string()); // Complete the cycle
                        cycles.push(cycle);
                    }
                }
            }
        }

        current_path.pop();
        rec_stack.remove(module);
    }
}

/// Smart cache configuration and management
#[derive(Debug, Clone)]
pub struct SmartCacheConfig {
    pub enable_persistent_cache: bool,
    pub cache_directory: std::path::PathBuf,
    pub max_cache_size_mb: usize,
    pub max_cache_entries: usize,
    pub cache_cleanup_interval: Duration,
    pub enable_content_hashing: bool,
    pub enable_ast_caching: bool,
    pub enable_analysis_caching: bool,
    pub cache_compression: bool,
}

impl Default for SmartCacheConfig {
    fn default() -> Self {
        Self {
            enable_persistent_cache: true,
            cache_directory: std::env::temp_dir().join("olang_cache"),
            max_cache_size_mb: 100,
            max_cache_entries: 1000,
            cache_cleanup_interval: Duration::from_secs(300), // 5 minutes
            enable_content_hashing: true,
            enable_ast_caching: true,
            enable_analysis_caching: true,
            cache_compression: false,
        }
    }
}

/// Cache statistics for monitoring and optimization
#[derive(Debug, Default, Clone)]
pub struct CacheStatistics {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub invalidations: u64,
    pub persistent_loads: u64,
    pub persistent_saves: u64,
    pub cleanup_operations: u64,
    pub total_memory_usage: usize,
    pub average_compilation_time: Duration,
    pub cache_efficiency: f64, // hit_rate percentage
}

/// Persistent cache manager for disk-based caching
#[derive(Debug)]
pub struct PersistentCacheManager {
    config: SmartCacheConfig,
    _cache_generation: u64,
    last_cleanup: Instant,
}

impl PersistentCacheManager {
    pub fn new(config: SmartCacheConfig) -> Result<Self, InterpreterError> {
        if config.enable_persistent_cache {
            std::fs::create_dir_all(&config.cache_directory).map_err(|e| {
                InterpreterError::RuntimeError {
                    message: format!("Failed to create cache directory: {}", e),
                }
            })?;
        }

        Ok(Self {
            config,
            _cache_generation: 1,
            last_cleanup: Instant::now(),
        })
    }

    /// Load cache entry from persistent storage
    pub fn load_cache_entry(
        &self,
        module_path: &str,
    ) -> Result<Option<ModuleCacheEntry>, InterpreterError> {
        if !self.config.enable_persistent_cache {
            return Ok(None);
        }

        let cache_file = self.get_cache_file_path(module_path);
        if !cache_file.exists() {
            return Ok(None);
        }

        let _data = std::fs::read(&cache_file).map_err(|e| InterpreterError::RuntimeError {
            message: format!("Failed to read cache file: {}", e),
        })?;

        // Feature 8: Simplified - persistent caching disabled for thread safety
        // Just return None to indicate no cached entry

        Ok(None)
    }

    /// Save cache entry to persistent storage
    pub fn save_cache_entry(
        &mut self,
        module_path: &str,
        _entry: &ModuleCacheEntry,
    ) -> Result<(), InterpreterError> {
        if !self.config.enable_persistent_cache {
            return Ok(());
        }

        // Feature 8: Simplified - persistent caching disabled for thread safety
        // Only create cache directory structure, no actual serialization
        let cache_file = self.get_cache_file_path(module_path);
        if let Some(parent) = cache_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to create cache directory: {}", e),
            })?;
        }

        Ok(())
    }

    /// Clean up old cache entries
    pub fn cleanup_cache(&mut self) -> Result<CacheCleanupStats, InterpreterError> {
        if !self.config.enable_persistent_cache {
            return Ok(CacheCleanupStats::default());
        }

        let now = Instant::now();
        if now.duration_since(self.last_cleanup) < self.config.cache_cleanup_interval {
            return Ok(CacheCleanupStats::default());
        }

        let mut stats = CacheCleanupStats::default();
        let cache_dir = &self.config.cache_directory;

        if !cache_dir.exists() {
            return Ok(stats);
        }

        let entries = std::fs::read_dir(cache_dir).map_err(|e| InterpreterError::RuntimeError {
            message: format!("Failed to read cache directory: {}", e),
        })?;

        let mut cache_files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| InterpreterError::RuntimeError {
                message: format!("Failed to read cache entry: {}", e),
            })?;

            if entry.path().extension().and_then(|s| s.to_str()) == Some("cache") {
                cache_files.push(entry.path());
            }
        }

        // Sort by modification time (oldest first)
        cache_files.sort_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });

        // Calculate total cache size
        let mut total_size = 0;
        for file in &cache_files {
            if let Ok(metadata) = std::fs::metadata(file) {
                total_size += metadata.len() as usize;
            }
        }

        let max_size_bytes = self.config.max_cache_size_mb * 1024 * 1024;
        let max_entries = self.config.max_cache_entries;

        // Remove entries if over limits
        let mut files_to_remove = Vec::new();

        // Remove by count limit
        if cache_files.len() > max_entries {
            files_to_remove.extend(&cache_files[..cache_files.len() - max_entries]);
        }

        // Remove by size limit
        if total_size > max_size_bytes {
            let mut current_size = total_size;
            for file in &cache_files {
                if current_size <= max_size_bytes {
                    break;
                }
                if !files_to_remove.contains(&file) {
                    files_to_remove.push(file);
                    if let Ok(metadata) = std::fs::metadata(file) {
                        current_size -= metadata.len() as usize;
                    }
                }
            }
        }

        // Actually remove the files
        for file in files_to_remove {
            if let Ok(metadata) = std::fs::metadata(file) {
                stats.bytes_freed += metadata.len() as usize;
            }
            if std::fs::remove_file(file).is_ok() {
                stats.files_removed += 1;
            }
        }

        self.last_cleanup = now;
        stats.cleanup_time = now.elapsed();

        Ok(stats)
    }

    fn get_cache_file_path(&self, module_path: &str) -> std::path::PathBuf {
        let sanitized = module_path.replace(['/', '\\', '.'], "_");
        self.config
            .cache_directory
            .join(format!("{}.cache", sanitized))
    }
}

#[derive(Debug, Default)]
pub struct CacheCleanupStats {
    pub files_removed: usize,
    pub bytes_freed: usize,
    pub cleanup_time: Duration,
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
        assert!(tracker
            .dependencies
            .get("A")
            .unwrap()
            .contains(&"B".to_string()));
        assert!(tracker
            .dependencies
            .get("A")
            .unwrap()
            .contains(&"C".to_string()));

        // Test dependent queries
        assert_eq!(tracker.dependents.get("B").unwrap().len(), 1);
        assert!(tracker
            .dependents
            .get("B")
            .unwrap()
            .contains(&"A".to_string()));

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

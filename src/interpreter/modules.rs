//! The module system: loading and executing module files, selective and
//! wildcard imports, dependency-map resolution (packages), the module
//! cache, and dependency/circularity tracking.

use super::{
    CacheCleanupStats, CacheStatistics, Environment, Interpreter, InterpreterError,
    ModuleCacheEntry, ModuleDebugConfig, ModuleDependencyTracker,
};
use crate::analyze::AnalysisReport;
use crate::ast::{Pattern, Program, ShareDecl, UseDecl, Value};
use crate::clock::{Instant, system_now};
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

impl Interpreter {
    /// Feature 8: Enhanced cached module retrieval with smart validation
    pub(crate) fn get_cached_module(
        &mut self,
        module_path: &str,
    ) -> Result<Option<ModuleCacheEntry>, InterpreterError> {
        // Check if we have a cached entry first (read-only check)
        let has_entry = self.module_cache.contains_key(module_path);
        if !has_entry {
            self.cache_statistics.cache_misses += 1;
            return Ok(None);
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
            entry.last_accessed = system_now();
            self.cache_statistics.cache_hits += 1;

            // Validate if needed
            if should_validate
                && let Some(hash) = current_hash
                && hash != entry.content_hash
            {
                // Content changed, invalidate cache
                self.cache_statistics.invalidations += 1;
                self.module_cache.remove(module_path);
                return Ok(None);
            }

            Ok(Some(entry.clone()))
        } else {
            Ok(None)
        }
    }

    /// Cache a module for future use with smart caching enhancements
    pub(crate) fn cache_module(
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
    pub(crate) fn cache_module_with_options(
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
            last_accessed: system_now(),
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
    pub(crate) fn calculate_file_hash(
        &self,
        file_path: &std::path::Path,
    ) -> Result<String, InterpreterError> {
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
    pub(crate) fn calculate_string_hash(&self, content: &str) -> String {
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
    pub(crate) fn bind_module_imports(
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
                            match self.get_module_export(module, item_name) {
                                Some(value) => {
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
                                }
                                _ => {
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

    /// Enhanced module cache clearing.
    pub fn clear_module_cache(&mut self) {
        self.module_cache.clear();
        self.dependency_tracker = ModuleDependencyTracker::new();
        self.cache_statistics = CacheStatistics::default();
        crate::log::get_logger().debug("interpreter", "Smart module cache cleared");
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

        cleanup_stats.cleanup_time = start_time.elapsed();
        self.cache_statistics.cleanup_operations += 1;

        Ok(cleanup_stats)
    }

    /// Feature 8: Calculate priority score for cache entry (higher = keep longer)
    fn calculate_cache_priority_score(&self, entry: &ModuleCacheEntry) -> f64 {
        let _now = system_now();
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

    /// Load a module from the file system or standard library
    pub(crate) fn load_module_from_file(
        &mut self,
        module_path: &str,
    ) -> Result<Value, InterpreterError> {
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
                message: format!("Failed to parse module {}:\n{}", file_path.display(), e),
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
            last_accessed: system_now(),
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
    pub(crate) fn resolve_module_path(
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
    pub(crate) fn discover_module_dependency(
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
            match self.get_cached_module(&current_module) {
                Ok(cached) => {
                    if let Some(cached_entry) = cached {
                        if let Some(ref file_path) = cached_entry.file_path {
                            file_path
                                .parent()
                                .unwrap_or_else(|| std::path::Path::new("."))
                                .to_path_buf()
                        } else {
                            crate::clock::current_dir().map_err(|e| {
                                InterpreterError::RuntimeError {
                                    message: format!("Failed to get current directory: {}", e),
                                }
                            })?
                        }
                    } else {
                        crate::clock::current_dir().map_err(|e| InterpreterError::RuntimeError {
                            message: format!("Failed to get current directory: {}", e),
                        })?
                    }
                }
                _ => crate::clock::current_dir().map_err(|e| InterpreterError::RuntimeError {
                    message: format!("Failed to get current directory: {}", e),
                })?,
            }
        } else {
            // No current module context, use current working directory
            crate::clock::current_dir().map_err(|e| InterpreterError::RuntimeError {
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
        let current_dir =
            crate::clock::current_dir().map_err(|e| InterpreterError::RuntimeError {
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
                return crate::clock::current_dir().map_err(|e| InterpreterError::RuntimeError {
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
        if let Ok(current_dir) = crate::clock::current_dir() {
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
        if let Ok(current_dir) = crate::clock::current_dir()
            && let Ok(entries) = std::fs::read_dir(&current_dir)
        {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str()
                    && name.ends_with(".ol")
                    && name != "main.ol"
                {
                    available_modules.push(name.trim_end_matches(".ol").to_string());
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
    pub(crate) fn get_module_export(&self, module: &Value, export_name: &str) -> Option<Value> {
        match module {
            Value::Struct { fields, .. } => fields.get(export_name).cloned(),
            _ => None,
        }
    }

    pub(crate) fn eval_use_decl(&mut self, use_decl: UseDecl) -> Result<Value, InterpreterError> {
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
}

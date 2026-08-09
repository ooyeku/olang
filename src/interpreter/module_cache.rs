//! Module caching machinery: cache entries, dependency tracking, smart
//! cache configuration, and the persistent on-disk cache.

use super::InterpreterError;
use crate::ast::Value;
use crate::clock::Instant;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

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
            // The playground has no disk: no persistent cache, and
            // std::env::temp_dir() would panic there ("no filesystem").
            enable_persistent_cache: cfg!(feature = "native"),
            #[cfg(feature = "native")]
            cache_directory: std::env::temp_dir().join("olang_cache"),
            #[cfg(not(feature = "native"))]
            cache_directory: std::path::PathBuf::from("olang_cache"),
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

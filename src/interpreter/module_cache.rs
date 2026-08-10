//! Module caching machinery: cache entries, dependency tracking, and smart
//! cache configuration.

use crate::ast::Value;
use std::collections::HashMap;
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

#[derive(Debug, Default)]
pub struct CacheCleanupStats {
    pub files_removed: usize,
    pub bytes_freed: usize,
    pub cleanup_time: Duration,
}

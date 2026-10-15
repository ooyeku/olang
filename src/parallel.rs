use parking_lot::RwLock;
use std::sync::LazyLock;

/// Global configuration for parallelization
#[derive(Clone, Debug)]
pub struct ParallelConfig {
    /// Minimum list size to trigger parallelization
    pub min_parallel_size: usize,
    /// Maximum number of threads to use
    pub max_threads: usize,
    /// Whether parallelization is enabled
    pub enabled: bool,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            min_parallel_size: 1000,
            #[cfg(feature = "native")]
            max_threads: num_cpus::get(),
            #[cfg(not(feature = "native"))]
            max_threads: 1,
            enabled: true,
        }
    }
}

static PARALLEL_CONFIG: LazyLock<RwLock<ParallelConfig>> =
    LazyLock::new(|| RwLock::new(ParallelConfig::default()));

/// Get current parallel configuration
pub fn get_config() -> ParallelConfig {
    PARALLEL_CONFIG.read().clone()
}

/// Set parallel configuration
pub fn set_config(config: ParallelConfig) {
    *PARALLEL_CONFIG.write() = config;
}

/// Check if a list should be processed in parallel
pub fn should_parallelize(list_size: usize) -> bool {
    // Without the native feature (wasm playground) there is no thread pool:
    // every "parallel" path degrades to its sequential twin.
    #[cfg(not(feature = "native"))]
    {
        let _ = list_size;
        false
    }
    #[cfg(feature = "native")]
    {
        let config = get_config();
        config.enabled && list_size >= config.min_parallel_size
    }
}

/// The thread count the environment asks for: `OLANG_PARALLELISM`.
/// `None` when it is not set to a positive integer.
pub fn parallelism_from_env() -> Option<usize> {
    std::env::var("OLANG_PARALLELISM")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
}

/// Initialize the parallel processing system
pub fn initialize_parallelization(
    threads: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "native")]
    if let Some(num_threads) = threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build_global()?;

        let mut config = get_config();
        config.max_threads = num_threads;
        set_config(config);
    }
    #[cfg(not(feature = "native"))]
    let _ = threads;
    Ok(())
}

/// Set the minimum threshold for parallel processing
pub fn set_parallel_threshold(threshold: usize) {
    let mut config = get_config();
    config.min_parallel_size = threshold;
    set_config(config);
}

/// Enable or disable parallelization
pub fn set_parallel_enabled(enabled: bool) {
    let mut config = get_config();
    config.enabled = enabled;
    set_config(config);
}

#[cfg(test)]
mod tests {
    use super::parallelism_from_env;

    #[test]
    fn olang_parallelism_is_read() {
        // SAFETY: nothing else in the test binary reads this variable.
        unsafe { std::env::remove_var("OLANG_PARALLELISM") };
        assert_eq!(parallelism_from_env(), None);
        unsafe { std::env::set_var("OLANG_PARALLELISM", "5") };
        assert_eq!(parallelism_from_env(), Some(5));
        unsafe { std::env::set_var("OLANG_PARALLELISM", "0") };
        assert_eq!(parallelism_from_env(), None, "zero is no count");
        unsafe { std::env::remove_var("OLANG_PARALLELISM") };
    }
}

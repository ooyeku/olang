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

/// Initialize the parallel processing system
/// The thread count the environment asks for: `OLANG_PARALLELISM`, or
/// `OVM_PARALLELISM`, its older name, kept as an alias. `None` when
/// neither is set to a positive integer.
pub fn parallelism_from_env() -> Option<usize> {
    ["OLANG_PARALLELISM", "OVM_PARALLELISM"]
        .iter()
        .find_map(|v| std::env::var(v).ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
}

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
    fn olang_parallelism_is_read_and_the_old_name_is_its_alias() {
        // SAFETY: nothing else in the test binary reads these variables.
        unsafe {
            std::env::remove_var("OLANG_PARALLELISM");
            std::env::remove_var("OVM_PARALLELISM");
        }
        assert_eq!(parallelism_from_env(), None);
        unsafe { std::env::set_var("OVM_PARALLELISM", "3") };
        assert_eq!(parallelism_from_env(), Some(3));
        unsafe { std::env::set_var("OLANG_PARALLELISM", "5") };
        assert_eq!(parallelism_from_env(), Some(5), "the new name wins");
        unsafe { std::env::set_var("OLANG_PARALLELISM", "0") };
        assert_eq!(parallelism_from_env(), None, "zero is no count");
        unsafe {
            std::env::remove_var("OLANG_PARALLELISM");
            std::env::remove_var("OVM_PARALLELISM");
        }
    }
}

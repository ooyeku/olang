//! When builtins run in parallel: the worker pool's size and the list
//! length below which work stays on one thread.

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

/// Initialize the parallel processing system: the pool's size, when one
/// is asked for (`--ovm-parallelism`, `OLANG_PARALLELISM`). No thread is
/// started here; the pool is made by the first parallel operation
/// ([`install`]).
pub fn initialize_parallelization(
    threads: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "native")]
    if let Some(num_threads) = threads {
        let mut config = get_config();
        config.max_threads = num_threads.max(1);
        set_config(config);
    }
    #[cfg(not(feature = "native"))]
    let _ = threads;
    Ok(())
}

/// The pool's size when nothing caps it: `OLANG_PARALLELISM`, else a
/// thread a core.
#[cfg(feature = "native")]
fn default_threads() -> usize {
    parallelism_from_env().unwrap_or_else(num_cpus::get).max(1)
}

/// Cap the parallel pool at `n` threads from the program itself
/// (`task.set_parallelism(n)`); `None` gives back the default
/// (`OLANG_PARALLELISM`, else a thread a core). Not passed to children,
/// as an environment variable would be. The pool is remade at its next
/// use; an idle one is ended now. Answers the count now in effect.
pub fn set_parallelism(n: Option<usize>) -> usize {
    #[cfg(feature = "native")]
    {
        let threads = n.map(|n| n.max(1)).unwrap_or_else(default_threads);
        let mut config = get_config();
        config.max_threads = threads;
        set_config(config);
        pool::resize(threads);
        threads
    }
    #[cfg(not(feature = "native"))]
    {
        let _ = n;
        1
    }
}

/// Run `f` in olang's parallel pool, so the rayon operations in it
/// (`par_iter`, `par_sort_by`, ...) run on its threads. The pool is made
/// by the first call, sized by [`ParallelConfig::max_threads`], and
/// ended once it has been idle for [`pool::IDLE`]: a program that does no
/// parallel work starts no pool, and one that did, keeps none while it
/// waits (OL-552). Called from a pool thread, `f` just runs.
#[cfg(feature = "native")]
pub fn install<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    pool::install(f)
}

/// Without the native feature there is no pool: `f` runs here.
#[cfg(not(feature = "native"))]
pub fn install<R>(f: impl FnOnce() -> R) -> R {
    f()
}

/// How many threads the pool has now (0 when there is none): for tests
/// and diagnostics.
pub fn pool_threads() -> usize {
    #[cfg(feature = "native")]
    {
        pool::live()
    }
    #[cfg(not(feature = "native"))]
    {
        0
    }
}

#[cfg(feature = "native")]
pub(crate) mod pool {
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// How long the pool lives without work before its threads end.
    pub const IDLE: Duration = Duration::from_secs(3);

    struct Slot {
        pool: Option<Arc<rayon::ThreadPool>>,
        size: usize,
        /// calls in the pool now: it is never ended under one
        users: usize,
        last: Instant,
        /// a thread is watching for the pool going idle
        reaper: bool,
    }

    static SLOT: std::sync::LazyLock<Mutex<Slot>> = std::sync::LazyLock::new(|| {
        Mutex::new(Slot {
            pool: None,
            size: 0,
            users: 0,
            last: Instant::now(),
            reaper: false,
        })
    });

    struct Using;
    impl Drop for Using {
        fn drop(&mut self) {
            let mut s = SLOT.lock();
            s.users -= 1;
            s.last = Instant::now();
        }
    }

    pub fn live() -> usize {
        let s = SLOT.lock();
        if s.pool.is_some() { s.size } else { 0 }
    }

    pub fn resize(threads: usize) {
        let gone = {
            let mut s = SLOT.lock();
            if s.size == threads || s.users > 0 {
                None
            } else {
                s.pool.take()
            }
        };
        drop(gone);
    }

    pub fn install<R: Send>(f: impl FnOnce() -> R + Send) -> R {
        if rayon::current_thread_index().is_some() {
            return f();
        }
        let want = super::get_config().max_threads.max(1);
        let pool = {
            let mut s = SLOT.lock();
            if s.pool.is_none() || s.size != want {
                let built = rayon::ThreadPoolBuilder::new()
                    .num_threads(want)
                    .thread_name(|i| format!("olang-par-{}", i))
                    .build();
                match built {
                    Ok(p) => {
                        s.pool = Some(Arc::new(p));
                        s.size = want;
                    }
                    // no threads to be had: the work runs on this one
                    // (rayon's own pool, made on demand, if it can be)
                    Err(_) if s.pool.is_none() => {
                        drop(s);
                        return f();
                    }
                    Err(_) => {}
                }
            }
            s.users += 1;
            s.last = Instant::now();
            if !s.reaper {
                s.reaper = std::thread::Builder::new()
                    .name("olang-par-idle".into())
                    .spawn(reap)
                    .is_ok();
            }
            s.pool.clone().expect("the pool was just made")
        };
        let _using = Using;
        pool.install(f)
    }

    /// Ends the pool once no call has used it for [`IDLE`], then itself.
    fn reap() {
        loop {
            let wait = {
                let mut s = SLOT.lock();
                let idle = s.last.elapsed();
                if s.users == 0 && (idle >= IDLE || s.pool.is_none()) {
                    let gone = s.pool.take();
                    s.reaper = false;
                    drop(s);
                    drop(gone); // its threads end as they see it gone
                    return;
                }
                IDLE.saturating_sub(idle).max(Duration::from_millis(50))
            };
            std::thread::sleep(wait);
        }
    }
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

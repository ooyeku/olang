//! One clock for both worlds.
//!
//! On native builds these are thin wrappers over `std::time`. On the
//! playground build (wasm32 without the "native" feature) std's clocks
//! panic — "time not implemented on this platform" — so the host page
//! injects two imports instead: `host_now_ms` (performance.now, monotonic)
//! and `host_epoch_ms` (Date.now). `Instant` is rebuilt on top of them and
//! `system_now()` reconstructs a real `SystemTime` as an offset from
//! `UNIX_EPOCH`, which is pure arithmetic and works everywhere.

#[cfg(feature = "native")]
pub use std::time::Instant;

/// The current wall-clock time as a `SystemTime`, safely on every target.
#[cfg(feature = "native")]
pub fn system_now() -> std::time::SystemTime {
    std::time::SystemTime::now()
}

/// Milliseconds since the Unix epoch.
#[cfg(feature = "native")]
pub fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `std::env::current_dir` that errors instead of panicking on wasm
/// (std's unsupported backend panics with "no filesystem on this platform").
pub fn current_dir() -> std::io::Result<std::path::PathBuf> {
    #[cfg(feature = "native")]
    {
        std::env::current_dir()
    }
    #[cfg(not(feature = "native"))]
    {
        Err(std::io::Error::other(
            "no filesystem in the playground sandbox",
        ))
    }
}

/// Block the current thread for `ms` milliseconds. The playground has no
/// scheduler to sleep on, so it spins on the host clock instead — it runs
/// inside a worker the page can terminate, never on the UI thread.
pub fn sleep_ms(ms: u64) {
    #[cfg(feature = "native")]
    std::thread::sleep(std::time::Duration::from_millis(ms));
    #[cfg(not(feature = "native"))]
    {
        let start = Instant::now();
        while (start.elapsed().as_millis() as u64) < ms {
            std::hint::spin_loop();
        }
    }
}

/// Wall-clock "now" in UTC on every target.
pub fn utc_now() -> chrono::DateTime<chrono::Utc> {
    #[cfg(feature = "native")]
    {
        chrono::Utc::now()
    }
    #[cfg(not(feature = "native"))]
    {
        chrono::DateTime::from_timestamp_millis(epoch_ms()).unwrap_or_default()
    }
}

/// Wall-clock "now" with the local offset where the platform has one.
/// The playground has no timezone database, so it reports UTC.
pub fn local_now_fixed() -> chrono::DateTime<chrono::FixedOffset> {
    #[cfg(feature = "native")]
    {
        chrono::Local::now().fixed_offset()
    }
    #[cfg(not(feature = "native"))]
    {
        utc_now().fixed_offset()
    }
}

#[cfg(not(feature = "native"))]
mod hosted {
    use std::time::Duration;

    #[cfg(target_arch = "wasm32")]
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_now_ms() -> f64;
        fn host_epoch_ms() -> f64;
    }

    // A non-wasm build without the native feature only exists for
    // `cargo check`-style verification; give it inert clocks rather than
    // undefined symbols.
    #[cfg(not(target_arch = "wasm32"))]
    unsafe fn host_now_ms() -> f64 {
        0.0
    }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe fn host_epoch_ms() -> f64 {
        0.0
    }

    /// Monotonic instant backed by the host's `performance.now()`.
    #[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
    pub struct Instant {
        ms: f64,
    }

    impl Instant {
        pub fn now() -> Self {
            Self {
                ms: unsafe { host_now_ms() },
            }
        }

        pub fn elapsed(&self) -> Duration {
            Self::now().duration_since(*self)
        }

        pub fn duration_since(&self, earlier: Instant) -> Duration {
            Duration::from_secs_f64((self.ms - earlier.ms).max(0.0) / 1000.0)
        }
    }

    pub fn epoch_ms() -> i64 {
        unsafe { host_epoch_ms() as i64 }
    }

    pub fn system_now() -> std::time::SystemTime {
        std::time::UNIX_EPOCH + Duration::from_millis(epoch_ms().max(0) as u64)
    }
}

#[cfg(not(feature = "native"))]
pub use hosted::{Instant, epoch_ms, system_now};

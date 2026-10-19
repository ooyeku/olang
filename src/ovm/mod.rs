//! OVM — olang's bytecode execution tier.
//!
//! Three modules remain after the 0.24 cleanup:
//!
//! - [`bytecode`]: the register-based VM and its compiler
//! - [`tier`]: hot-function promotion from the interpreter
//! - [`value`]: the reference-counted runtime value model
//! - [`gc`]: safepoint flags the interpreter polls in loops
//!
//! The former routing layer (`OlangVirtualMachine`, `ovm_integration`) and
//! the speculative engines (pipeline, SIMD, lazy, fusion, adaptive, JIT
//! scaffolding, region allocator) were deleted: measured against the
//! rewritten interpreter, the routing layer was ~70% overhead and none of
//! the engines were wired into execution. See docs/ovm.md for the
//! architecture that actually runs.

/// Whether the environment variable `$name` is set, read once a process:
/// a debug switch on a hot path (`getenv` takes the environment's lock
/// and scans it, and a VM call or a conversion that asked it each time
/// spent a tenth of a frame there).
macro_rules! env_on {
    ($name:literal) => {{
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var_os($name).is_some())
    }};
}

pub mod bytecode; // Register-based bytecode VM
pub mod gc; // Safepoint coordination flags
#[cfg(feature = "native")]
pub mod jit; // Baseline JIT: pure-integer hot functions to native code
#[cfg(feature = "native")]
pub mod osr; // On-stack replacement: hot loop regions to native, mid-frame
pub mod tier; // Hot-function promotion to the bytecode tier
pub mod value;
pub mod warm; // Warm start: cross-run tier profiles // Reference-counted runtime values

pub use value::OvmValue;

/// Unique identifier for functions compiled to bytecode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FunctionId(u64);

impl Default for FunctionId {
    fn default() -> Self {
        Self::new()
    }
}

impl FunctionId {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        Self(COUNTER.fetch_add(1, Ordering::SeqCst))
    }

    /// Ids are small dense integers from the global counter, so they double
    /// as direct indices into per-VM tables (no hashing on the call path).
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// The number behind the id: the compile cache writes it and maps it
    /// to this run's id on the way back (`crate::compile_cache`).
    pub fn raw(self) -> u64 {
        self.0
    }
}

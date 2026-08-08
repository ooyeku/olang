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

pub mod bytecode; // Register-based bytecode VM
pub mod gc; // Safepoint coordination flags
pub mod tier; // Hot-function promotion to the bytecode tier
pub mod value; // Reference-counted runtime values

pub use value::OvmValue;

/// Unique identifier for functions compiled to bytecode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}

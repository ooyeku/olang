//! Re-export olang's playground boundary so its `#[no_mangle]` exports
//! (olang_alloc / olang_dealloc / olang_run / olang_result_free) land in
//! this cdylib. See src/playground.rs in the main crate for the contract.
//!
//! On a native build (workspace-wide `cargo build`/`cargo test`, where
//! feature unification turns olang's "native" feature on) the playground
//! module doesn't exist and the glob simply re-exports less; the cdylib
//! it produces there is unused.

pub use olang::*;

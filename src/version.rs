//! Version information for the Olang project.
//!  
//! This module provides the current version of the Olang language,
//! which is automatically pulled from `Cargo.toml` at compile time.

/// The current language version, automatically fetched from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Convenience helper that returns the version string.
#[inline]
pub const fn current() -> &'static str {
    VERSION
}

//!/ Version information for the Olang project.
//!  
//! Keeping the version string in a single, centralised place makes it trivial
//! to bump versions across the whole code-base (REPL banner, docs, CLI flags
//! etc.).  We intentionally keep the version in the short "major.minor" form
//! (two-segment semantic versioning) so that patch versions can continue to be
//! tracked in Cargo.toml without leaking into user-facing output.

/// The **current language version** in `major.minor` format.
///
/// *NOTE*: Bump this constant (and the value in `Cargo.toml`) whenever a new
/// release is cut.
pub const VERSION: &str = "0.16";

/// Convenience helper that returns the version string. Kept as a function to
/// make it possible to pass a function pointer where a `'static` string slice
/// wouldn't be accepted.
#[inline]
pub const fn current() -> &'static str {
    VERSION
}

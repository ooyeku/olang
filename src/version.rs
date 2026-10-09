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

/// The commit the binary was built from (build.rs; empty outside a checkout).
pub const BUILD_COMMIT: &str = env!("OLANG_BUILD_COMMIT");
/// The branch checked out then.
pub const BUILD_BRANCH: &str = env!("OLANG_BUILD_BRANCH");
/// `true` when the checkout had uncommitted changes to tracked files then,
/// `false` when it had none, empty when git could not say.
pub const BUILD_DIRTY: &str = env!("OLANG_BUILD_DIRTY");
/// When the build script last ran (UTC, RFC 3339): the last change to the
/// sources, the lock or the commit.
pub const BUILD_DATE: &str = env!("OLANG_BUILD_DATE");
/// `release` or `debug`.
pub const BUILD_PROFILE: &str = env!("OLANG_BUILD_PROFILE");
/// The target triple.
pub const BUILD_TARGET: &str = env!("OLANG_BUILD_TARGET");
/// `rustc -V` of the compiler that built it.
pub const BUILD_RUSTC: &str = env!("OLANG_BUILD_RUSTC");
/// `name version` of the crates a bug report needs, `;`-separated; a
/// version ending `+vendored` is a patched copy in vendor/.
pub const BUILD_CRATES: &str = env!("OLANG_BUILD_CRATES");

/// The Cargo features this binary was built with.
pub fn features() -> Vec<&'static str> {
    let mut out = Vec::new();
    if cfg!(feature = "native") {
        out.push("native");
    }
    if cfg!(feature = "full") {
        out.push("full");
    }
    if cfg!(feature = "gui") {
        out.push("gui");
    }
    if cfg!(feature = "regex-module") {
        out.push("regex-module");
    }
    if cfg!(feature = "rsa-crypto") {
        out.push("rsa-crypto");
    }
    if cfg!(feature = "alloc-count") {
        out.push("alloc-count");
    }
    out
}

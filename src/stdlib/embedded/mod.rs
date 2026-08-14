//! Embedded olang-source stdlib modules — "builtin packages" written in
//! olang and compiled into the binary via `include_str!`.
//!
//! Unlike the native modules (which wrap Rust crates for FFI: fs, http,
//! crypto, db, ...), these are pure olang. They resolve like any package
//! `use`, but their source lives in the binary rather than on disk, so they
//! are always available with no filesystem package. This is how the pure
//! parts of the stdlib can be self-hosted while native primitives stay Rust.

/// Registry of embedded modules: `use <name>` loads this source. Add a module
/// by dropping a `.ol` file beside this file and listing it here.
const MODULES: &[(&str, &str)] = &[
    ("colx", include_str!("colx.ol")),
    ("mathx", include_str!("mathx.ol")),
    ("ui", include_str!("ui.ol")),
    ("viz", include_str!("viz.ol")),
];

/// The olang source of an embedded module, if one is registered under `name`.
pub fn source(name: &str) -> Option<&'static str> {
    MODULES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
}

/// Whether `name` is an embedded module.
pub fn is_embedded(name: &str) -> bool {
    MODULES.iter().any(|(n, _)| *n == name)
}

/// All embedded module names (for REPL help/completion).
pub fn names() -> Vec<&'static str> {
    MODULES.iter().map(|(n, _)| *n).collect()
}

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
    ("dash", include_str!("dash.ol")),
    ("cli", include_str!("cli.ol")),
    ("term", include_str!("term.ol")),
];

/// The olang source of an embedded module, if one is registered under `name`.
pub fn source(name: &str) -> Option<&'static str> {
    MODULES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
}

/// Process-wide cache of parsed embedded ASTs. The source is `'static` and
/// never changes, so its AST is reusable across every interpreter instance.
static PARSED: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<&'static str, std::sync::Arc<crate::ast::Program>>>,
> = std::sync::OnceLock::new();

/// The parsed AST of embedded module `name`, parsed once per process and
/// reused thereafter. Without this, every `use viz` in a fresh interpreter
/// re-parses the package — and a fresh interpreter is created for every CLI
/// invocation, every `olang test` file, and every playground run, so the
/// cold-start parse was being paid over and over. Returns `Err` with the
/// parse message if the (compiled-in, normally infallible) source fails to
/// parse, and `Ok(None)` if `name` is not an embedded module.
pub fn parsed(name: &str) -> Result<Option<std::sync::Arc<crate::ast::Program>>, String> {
    let Some((static_name, src)) = MODULES.iter().find(|(n, _)| *n == name) else {
        return Ok(None);
    };
    let cache = PARSED.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(program) = cache.lock().unwrap().get(static_name) {
        return Ok(Some(program.clone()));
    }
    let program = crate::parser::Parser::new()
        .parse(src)
        .map_err(|e| e.to_string())?;
    let arc = std::sync::Arc::new(program);
    cache.lock().unwrap().insert(static_name, arc.clone());
    Ok(Some(arc))
}

/// Whether `name` is an embedded module.
pub fn is_embedded(name: &str) -> bool {
    MODULES.iter().any(|(n, _)| *n == name)
}

/// All embedded module names (for REPL help/completion).
pub fn names() -> Vec<&'static str> {
    MODULES.iter().map(|(n, _)| *n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_memoizes_and_covers_every_module() {
        // A non-module name is Ok(None), not an error.
        assert!(parsed("definitely-not-a-module").unwrap().is_none());

        // Every registered package parses, and a second call returns the
        // very same Arc — proof the AST is cached, not re-parsed.
        for name in names() {
            let first = parsed(name).unwrap().expect("embedded module parses");
            let second = parsed(name).unwrap().expect("cached on second call");
            assert!(
                std::sync::Arc::ptr_eq(&first, &second),
                "{name} should be memoized (same Arc), not re-parsed"
            );
            assert!(
                !first.statements.is_empty(),
                "{name} should have statements"
            );
        }
    }
}

//! Every function the book names must exist.
//!
//! Running the code blocks proves the *examples* work. It says nothing
//! about the far larger surface the prose names in passing — "use
//! `fs.read_file`", "`col.frequencies` counts occurrences" — which is
//! where a rename or a removal quietly leaves a lie behind. The 1.0 audit
//! found one (`fs.read`, which has been `fs.read_file` for a long time)
//! by extracting every reference and resolving it; this keeps that check
//! from having to be redone by hand.
//!
//! False positives are the failure mode that gets a guard like this
//! switched off, so the extractor is deliberately conservative: it only
//! considers a known module prefix, and it skips the shapes that look
//! like references but are not — file names (`ods.md`, `olang-dom.js`)
//! and glob prose (`str.parse_*`, `testing.assert_*`).

use olang::{Interpreter, Parser};
use std::collections::BTreeSet;

/// Modules whose members are worth checking. Both kinds are here: those
/// always in scope, and the embedded olang packages that need `use`
/// first — the resolver below tries both, so the distinction does not
/// have to be maintained by hand.
const MODULES: &[&str] = &[
    "base64",
    "caps",
    "cell",
    "chan",
    "col",
    "collections",
    "crypto",
    "csv",
    "dates",
    "db",
    "dom",
    "fs",
    "http",
    "json",
    "math",
    "meta",
    "ods",
    "os",
    "plot",
    "proc",
    "random",
    "re",
    "stats",
    "str",
    "task",
    "testing",
    "time",
    "toml",
    "viz",
];

/// Pull `module.function` references out of prose and code alike.
fn references(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes: Vec<char> = text.chars().collect();
    for module in MODULES {
        let pat: Vec<char> = format!("{}.", module).chars().collect();
        let mut i = 0;
        while i + pat.len() < bytes.len() {
            if bytes[i..i + pat.len()] == pat[..] {
                // Must start a word: `ods.` inside `docs/ods.md` is a path.
                let prev_ok = i == 0
                    || !(bytes[i - 1].is_alphanumeric()
                        || bytes[i - 1] == '_'
                        || bytes[i - 1] == '/'
                        || bytes[i - 1] == '.');
                let mut j = i + pat.len();
                let start = j;
                while j < bytes.len()
                    && (bytes[j].is_ascii_lowercase()
                        || bytes[j].is_ascii_digit()
                        || bytes[j] == '_')
                {
                    j += 1;
                }
                let name: String = bytes[start..j].iter().collect();
                // Skip glob prose (`str.parse_*`) and file extensions.
                let is_glob = name.ends_with('_');
                let is_file = matches!(name.as_str(), "md" | "js" | "ol" | "rs" | "toml" | "lock");
                if prev_ok && !name.is_empty() && !is_glob && !is_file {
                    found.insert(format!("{}.{}", module, name));
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
    }
    found
}

/// Named on purpose while not existing. The design record discusses
/// options the project has *not* taken, and naming them concretely is
/// what makes that record useful — but each one has to be listed here,
/// with its reason, so an accidental reference to a removed function can
/// never hide among them.
const DELIBERATELY_HYPOTHETICAL: &[(&str, &str)] = &[(
    "ods.fma",
    "ods.md's design record weighs fused kernels as a future option, \
     pre-approved if the stated gate trips; it is not claimed to exist",
)];

/// Does `reference` resolve — either already in scope, or after importing
/// its module (which is how the embedded olang packages are reached)?
fn resolves(reference: &str) -> bool {
    let module = reference.split('.').next().unwrap_or("");
    for source in [
        format!("let probe = {}\n", reference),
        format!("use {}\nlet probe = {}\n", module, reference),
    ] {
        let Ok(program) = Parser::new().parse(&source) else {
            continue;
        };
        if Interpreter::new().eval_program(program).is_ok() {
            return true;
        }
    }
    false
}

#[test]
fn every_function_the_book_names_exists() {
    let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let mut files: Vec<_> = std::fs::read_dir(&docs)
        .expect("docs/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();

    let mut broken: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for path in &files {
        let text = std::fs::read_to_string(path).expect("readable");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        // The roadmap describes work not yet done, so it names functions
        // that are supposed not to exist yet.
        if name == "roadmap.md" {
            continue;
        }
        for reference in references(&text) {
            checked += 1;
            if DELIBERATELY_HYPOTHETICAL
                .iter()
                .any(|(name, _)| *name == reference)
            {
                continue;
            }
            if !resolves(&reference) {
                broken.push(format!("{}: {}", name, reference));
            }
        }
    }

    assert!(
        checked > 200,
        "extractor found only {checked} references — broken?"
    );
    assert!(
        broken.is_empty(),
        "the book names {} function(s) that do not resolve:\n  {}",
        broken.len(),
        broken.join("\n  ")
    );
}

/// The extractor has to reject the shapes that merely look like calls, or
/// the guard becomes noise and gets switched off.
#[test]
fn the_extractor_ignores_prose_that_is_not_a_reference() {
    let found = references(
        "See [the data stack](ods.md) and `olang-dom.js`; use `str.parse_*` \
         or str.trim, and docs/ods.md lists more.",
    );
    assert!(
        found.contains("str.trim"),
        "real references must be found: {found:?}"
    );
    for not_a_reference in ["ods.md", "dom.js", "str.parse_"] {
        assert!(
            !found.contains(not_a_reference),
            "{not_a_reference} is not a function reference: {found:?}"
        );
    }
}

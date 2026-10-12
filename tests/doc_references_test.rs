//! The book and the binary name the same functions, both ways.
//!
//! Running the code blocks proves the *examples* work. It says nothing
//! about the far larger surface the prose names in passing — "use
//! `fs.read_file`", "`col.frequencies` counts occurrences" — which is
//! where a rename or a removal quietly leaves a lie behind. The 1.0 audit
//! found one (`fs.read`, which has been `fs.read_file` for a long time)
//! by extracting every reference and resolving it; this keeps that check
//! from having to be redone by hand.
//!
//! The other direction matters as much: a member a module has that the
//! reference never mentions is surface nobody promised and nobody can
//! find. `every_member_a_module_has_is_documented` reads every module's
//! members from the binary and looks for each in stdlib.md; what is
//! deliberately left out is on [`INTERNAL`], with its reason.
//!
//! The module list is the binary's own (the native modules, the data
//! stack, the embedded packages and `collections`' submodules), so a new
//! module is checked the day it lands.
//!
//! False positives are the failure mode that gets a guard like this
//! switched off, so the extractor is deliberately conservative: it only
//! considers a known module prefix, and it skips the shapes that look
//! like references but are not — file names (`ods.md`, `olang-dom.js`)
//! and glob prose (`str.parse_*`, `testing.assert_*`).

use olang::ast::Value;
use olang::{Interpreter, Parser};
use std::collections::BTreeSet;

/// `collections`' submodules, which the book also names by their short
/// names after `use collections { heap }`.
const COLLECTIONS: &[&str] = &["alg", "bitset", "deque", "dsu", "heap", "table"];

/// Evaluates `source` on a fresh interpreter.
fn eval(source: &str) -> Option<Value> {
    let program = Parser::new().parse(source).ok()?;
    Interpreter::new().eval_program(program).ok()
}

/// Every module a program can reach: the always-in-scope ones, the
/// embedded packages that need `use`, and `collections`' submodules (as
/// `collections.heap`). Names beginning `__` are internal hooks.
fn modules() -> Vec<String> {
    let mut names: BTreeSet<String> = olang::stdlib::get_stdlib()
        .keys()
        .filter(|k| !k.starts_with("__"))
        .cloned()
        .collect();
    for name in olang::stdlib::embedded::names() {
        if COLLECTIONS.contains(&name) {
            names.insert(format!("collections.{}", name));
        } else {
            names.insert(name.to_string());
        }
    }
    names.into_iter().collect()
}

/// The `use` line a module path needs, if any.
fn use_line(module: &str) -> String {
    let root = module.split('.').next().unwrap_or(module);
    if olang::stdlib::embedded::is_embedded(root) && root != "collections" {
        format!("use {}\n", root)
    } else {
        String::new()
    }
}

/// The prefixes the extractor looks for: every module, and the
/// collections' short names.
fn prefixes() -> Vec<String> {
    let mut out: Vec<String> = modules().into_iter().filter(|m| !m.contains('.')).collect();
    out.extend(COLLECTIONS.iter().map(|s| s.to_string()));
    out
}

/// Pull `module.function` references (and `module.sub.function` chains)
/// out of prose and code alike.
fn references(text: &str, modules: &[String]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes: Vec<char> = text.chars().collect();
    for module in modules {
        let pat: Vec<char> = format!("{}.", module).chars().collect();
        let mut i = 0;
        while i + pat.len() < bytes.len() {
            if bytes[i..i + pat.len()] == pat[..] {
                // Must start a word: `ods.` inside `docs/ods.md` is a path.
                let prev_ok = i == 0
                    || !(bytes[i - 1].is_alphanumeric()
                        || bytes[i - 1] == '_'
                        || bytes[i - 1] == '/'
                        || bytes[i - 1] == '-'
                        || bytes[i - 1] == '.');
                let start = i + pat.len();
                let mut j = start;
                let ident = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_';
                while j < bytes.len() {
                    if ident(bytes[j]) {
                        j += 1;
                    } else if bytes[j] == '.'
                        && j > start
                        && j + 1 < bytes.len()
                        && bytes[j + 1].is_ascii_lowercase()
                    {
                        // `stats.norm.cdf`: a chain through a submodule.
                        j += 1;
                    } else {
                        break;
                    }
                }
                let name: String = bytes[start..j].iter().collect();
                let last = name.rsplit('.').next().unwrap_or("");
                // Skip glob prose (`str.parse_*`) and file extensions.
                let is_glob = name.ends_with('_') || bytes.get(j) == Some(&'*');
                let is_file = matches!(
                    last,
                    "md" | "js" | "json" | "ol" | "rs" | "toml" | "lock" | "html" | "olt"
                );
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
const DELIBERATELY_HYPOTHETICAL: &[(&str, &str)] = &[
    (
        "ods.fma",
        "ods.md's design record weighs fused kernels as a future option, \
         pre-approved if the stated gate trips; it is not claimed to exist",
    ),
    (
        "fs.reed_file",
        "migrating.md's example of a misspelled member and the error \
         naming the nearest one",
    ),
];

/// What `reference` is, by `typeof`: in scope, after `use` of its module,
/// or (for a collections short name) after `use collections { heap }`.
fn kind_of(reference: &str) -> Option<String> {
    let root = reference.split('.').next().unwrap_or("");
    let mut sources = vec![
        format!("typeof({})\n", reference),
        format!("use {}\ntypeof({})\n", root, reference),
    ];
    if COLLECTIONS.contains(&root) {
        sources.push(format!(
            "use collections {{ {} }}\ntypeof({})\n",
            root, reference
        ));
    }
    let mut kind = None;
    for source in sources {
        match eval(&source) {
            Some(Value::String(ref s)) if s.as_str() != "Unit" => return Some(s.to_string()),
            Some(Value::String(ref s)) => kind = Some(s.to_string()),
            _ => {}
        }
    }
    kind
}

/// A reference resolves when it names a function or a builtin. A
/// submodule (`collections.heap`, `stats.norm`) is named in prose as a
/// place, and resolves too; anything else — `()` above all, which a
/// missing member reads as since 0.88 — does not.
fn resolves(reference: &str) -> bool {
    matches!(
        kind_of(reference).as_deref(),
        Some("Function" | "Builtin" | "Module")
    )
}

fn book() -> Vec<(String, String)> {
    let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let mut files: Vec<_> = std::fs::read_dir(&docs)
        .expect("docs/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().to_string(),
                std::fs::read_to_string(path).expect("readable"),
            )
        })
        .collect()
}

#[test]
fn every_function_the_book_names_exists() {
    let prefixes = prefixes();
    let mut broken: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for (name, text) in book() {
        // The roadmap describes work not yet done, so it names functions
        // that are supposed not to exist yet.
        if name == "roadmap.md" {
            continue;
        }
        for reference in references(&text, &prefixes) {
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
        "the book names {} function(s) that do not resolve to one:\n  {}",
        broken.len(),
        broken.join("\n  ")
    );
}

/// A missing member reads as `()`; that is not a function, so the check
/// must not count it as resolved (it did, from 0.88 until this test).
#[test]
fn a_missing_member_does_not_resolve() {
    assert!(!resolves("fs.bogus_function_xyz"));
    assert!(!resolves("collections.heap.bogus"));
    assert!(resolves("fs.read_file"));
    assert!(resolves("heap.push"));
    assert!(resolves("stats.norm.cdf"));
    assert!(resolves("cli.parse"));
}

/// Members a module has that the reference leaves out on purpose.
const INTERNAL: &[(&str, &str)] = &[];

/// The member names of the module at `path`, with each one's `typeof`.
fn members(path: &str) -> Vec<(String, String)> {
    let source = format!(
        "{}let m = {}\nmap(map_keys(m), (k) => (k, typeof(map_get(m, k))))\n",
        use_line(path),
        path
    );
    let Some(Value::List(ref items)) = eval(&source) else {
        panic!("could not list the members of {path}");
    };
    items
        .iter()
        .map(|item| match item {
            Value::Tuple(pair) => match (&pair[0], &pair[1]) {
                (Value::String(k), Value::String(t)) => (k.to_string(), t.to_string()),
                other => panic!("unexpected member entry {other:?}"),
            },
            other => panic!("unexpected member entry {other:?}"),
        })
        .collect()
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `needle` occurs in `hay` as a whole name (not inside a longer
/// identifier, and not as the tail of a dotted path).
fn has_name(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(at) = hay[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let before = hay[..start].chars().next_back();
        let after = hay[end..].chars().next();
        if !before.is_some_and(|c| is_ident(c) || c == '.') && !after.is_some_and(is_ident) {
            return true;
        }
        from = start + needle.len().max(1);
    }
    false
}

/// The text of stdlib.md's sections about `root`: every heading that
/// names it in backticks (`` ## `str` — strings ``), and the collections
/// chapter for `collections`.
fn sections_about(stdlib: &str, root: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    let tick = format!("`{}`", root);
    for line in stdlib.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            inside = heading.contains(&tick)
                || (root == "collections" && heading.contains("Collections"));
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn every_member_a_module_has_is_documented() {
    let stdlib = book()
        .into_iter()
        .find(|(name, _)| name == "stdlib.md")
        .expect("stdlib.md")
        .1;
    let mut missing = Vec::new();
    let mut checked = 0usize;
    let mut queue = modules();
    while let Some(module) = queue.pop() {
        let root = module.split('.').next().unwrap_or(&module).to_string();
        let short = module.rsplit('.').next().unwrap_or(&module).to_string();
        let section = sections_about(&stdlib, &root);
        for (name, kind) in members(&module) {
            let full = format!("{}.{}", module, name);
            if kind == "Module" {
                queue.push(full.clone());
            }
            checked += 1;
            if INTERNAL.iter().any(|(n, _)| *n == full) {
                continue;
            }
            let documented = has_name(&stdlib, &full)
                || has_name(&stdlib, &format!("{}.{}", short, name))
                || has_name(&section, &format!("`{}", name))
                || has_name(&section, &format!("{}(", name));
            if !documented {
                missing.push(full);
            }
        }
    }
    assert!(checked > 500, "only {checked} members listed — broken?");
    missing.sort();
    assert!(
        missing.is_empty(),
        "{} member(s) a module has that stdlib.md does not document \
         (document each, or list it in INTERNAL with the reason):\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

/// The extractor has to reject the shapes that merely look like calls, or
/// the guard becomes noise and gets switched off.
#[test]
fn the_extractor_ignores_prose_that_is_not_a_reference() {
    let found = references(
        "See [the data stack](ods.md) and `olang-dom.js`; use `str.parse_*` \
         or str.trim, `stats.norm.cdf`, and docs/ods.md lists more.",
        &prefixes(),
    );
    assert!(
        found.contains("str.trim"),
        "real references must be found: {found:?}"
    );
    assert!(
        found.contains("stats.norm.cdf"),
        "a chain through a submodule is one reference: {found:?}"
    );
    for not_a_reference in ["ods.md", "dom.js", "str.parse_"] {
        assert!(
            !found.contains(not_a_reference),
            "{not_a_reference} is not a function reference: {found:?}"
        );
    }
}

#[test]
fn every_module_is_listed() {
    let modules = modules();
    for expected in [
        "gui",
        "vt",
        "pty",
        "bytes",
        "compress",
        "vec",
        "bigint",
        "asset",
        "runtime",
        "colx",
        "mathx",
        "cli",
        "term",
        "ui",
        "dash",
        "viz",
        "ods",
        "stats",
        "plot",
        "collections.heap",
    ] {
        assert!(
            modules.iter().any(|m| m == expected),
            "{expected} missing from {modules:?}"
        );
    }
    assert!(!modules.iter().any(|m| m.starts_with("__")));
}

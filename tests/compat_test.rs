//! The 1.x compatibility promise, held as a test (docs/stability.md,
//! "The 1.x compatibility check"; tools/release.sh runs it before every tag).
//!
//! Two checks, both reading `tests/compat/`:
//!
//! - `compat_programs_keep_their_output`: every program in
//!   `tests/compat/programs/` covers a Stable area (the core language,
//!   patterns, functions, types, errors, numbers, modules, macros, the
//!   stdlib modules, concurrency, the embedded packages, the scope
//!   rules) and must keep parsing and running with the same stdout and
//!   exit status on every 1.x release, on both tiers. Its expected
//!   output is `<name>.out` beside it.
//! - `compat_surface_still_stands`: `tests/compat/surface/<x.y>.txt` lists
//!   every name the stdlib had at that release (the global builtins and
//!   each module's members, the Experimental modules left out). Every
//!   name listed must still be there; new names are fine.
//!
//! `tests/compat/package/` is a package whose public surface is
//! `@stable`, which `tools/compat-check.sh` holds to 1.0 with `olang check
//! --against`.
//!
//! A program's first comment lines may carry a header: `// compat: exit 1`
//! (the exit status it must end with, default 0) and `// compat: test`
//! (run with `olang test`; the `.out` lines must then appear in order
//! among the runner's report, which is not contract). A program prints
//! only what is contract: values and `raise` messages, never error text,
//! and a report's fields rather than a whole record a later release may
//! add to.
//!
//! Adding a program: write it, run `OLANG_COMPAT_BLESS=1 cargo test
//! --profile fasttest --test compat_test` to write its `.out` (only a
//! missing `.out` is ever written), and read the `.out` before
//! committing it. `OLANG_COMPAT_SNAPSHOT=1.1` writes a new minor's
//! surface, once (it refuses to overwrite one); `tools/release.sh` does
//! it when it cuts an x.y.0.
//!
//! An intended break is never silent. A 1.x release may change behavior
//! only as a bug fix, or an Experimental surface in a minor release
//! (docs/stability.md, Versioning). When such a change alters a
//! program's output or removes a surface name:
//!
//! 1. add a `[[break]]` entry to `tests/compat/allow.toml`: `what` (the
//!    fixture as this test names it, or `surface:<name>`), `version`,
//!    `issue`, `kind` (`bug-fix`, `experimental` or `major`) and `reason`;
//! 2. update the `.out` in the same commit if you like:
//!    `tools/compat-check.sh` accepts an expectation changed since v1.0.0
//!    only when an entry names it;
//! 3. name the issue in the release's CHANGELOG section: `tools/release.sh`
//!    refuses the release otherwise, refuses a `major` break outside a
//!    new major version and an `experimental` one in a patch, and prints
//!    every intended break of the release it cuts.
//!
//! Never edit an existing `.out` or snapshot just to make this pass. The
//! allow-list is reviewed like code: an entry is a decision about what
//! olang promises.

use olang::ast::Value;
use olang::{Interpreter, Parser};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn compat_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat")
}

// ── the allow-list ──────────────────────────────────────────────────

/// One reviewed, intended break (`[[break]]` in allow.toml).
#[derive(Debug)]
struct Allowed {
    what: String,
    version: String,
    issue: String,
    kind: String,
    reason: String,
}

/// The kinds of break a 1.x release may make (docs/stability.md,
/// Versioning): a bug fix, a change to an Experimental surface, or a new
/// major version.
const KINDS: &[&str] = &["bug-fix", "experimental", "major"];

fn allow_list() -> Vec<Allowed> {
    let path = compat_dir().join("allow.toml");
    let text = std::fs::read_to_string(&path).expect("tests/compat/allow.toml");
    let doc: toml::Value = toml::from_str(&text).expect("allow.toml is valid TOML");
    let mut out = Vec::new();
    let entries = match doc.get("break") {
        None => return out,
        Some(toml::Value::Array(a)) => a.clone(),
        Some(_) => panic!("allow.toml: `break` must be an array of tables ([[break]])"),
    };
    for (i, e) in entries.iter().enumerate() {
        let field = |k: &str| -> String {
            match e.get(k).and_then(|v| v.as_str()) {
                Some(s) if !s.trim().is_empty() => s.to_string(),
                _ => panic!(
                    "allow.toml: break #{} has no `{k}`; every entry needs what, version, issue, kind and reason",
                    i + 1
                ),
            }
        };
        let a = Allowed {
            what: field("what"),
            version: field("version"),
            issue: field("issue"),
            kind: field("kind"),
            reason: field("reason"),
        };
        assert!(
            KINDS.contains(&a.kind.as_str()),
            "allow.toml: break `{}` has kind `{}`; it must be one of {KINDS:?}",
            a.what,
            a.kind
        );
        out.push(a);
    }
    out
}

fn allowed<'a>(list: &'a [Allowed], what: &str) -> Option<&'a Allowed> {
    list.iter().find(|a| a.what == what)
}

#[test]
fn compat_allow_list_is_well_formed() {
    // parsing checks every field; this names each entry so a review sees
    // them in the test output
    for a in allow_list() {
        println!(
            "allowed break: {} in {} ({}, {}): {}",
            a.what, a.version, a.kind, a.issue, a.reason
        );
    }
}

// ── the programs ────────────────────────────────────────────────────

/// A fixture: the program to run, the folder to run it in, its name
/// (as allow.toml names it), and its expected output file.
struct Fixture {
    name: String,
    file: PathBuf,
    dir: PathBuf,
    expected: PathBuf,
}

fn fixtures() -> Vec<Fixture> {
    let programs = compat_dir().join("programs");
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&programs)
        .expect("tests/compat/programs")
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        if path.is_dir() && path.join("main.ol").is_file() {
            out.push(Fixture {
                name: format!("programs/{stem}"),
                file: path.join("main.ol"),
                dir: path.clone(),
                expected: programs.join(format!("{stem}.out")),
            });
        } else if path.extension().is_some_and(|e| e == "ol") {
            out.push(Fixture {
                name: format!("programs/{stem}.ol"),
                file: path.clone(),
                dir: programs.clone(),
                expected: programs.join(format!("{stem}.out")),
            });
        }
    }
    out
}

/// What a fixture's `// compat:` header line asks: run it with `olang
/// test` (`test`), and the exit status it must end with (`exit N`).
struct Header {
    test: bool,
    exit: i32,
}

fn header(source: &str) -> Header {
    let mut h = Header {
        test: false,
        exit: 0,
    };
    for line in source.lines().take_while(|l| l.starts_with("//")) {
        let Some(rest) = line.strip_prefix("// compat:") else {
            continue;
        };
        for part in rest.split(',') {
            let part = part.trim();
            if part == "test" {
                h.test = true;
            } else if let Some(n) = part.strip_prefix("exit ") {
                h.exit = n
                    .trim()
                    .parse()
                    .expect("`// compat: exit N` takes a number");
            }
        }
    }
    h
}

/// Runs `olang [flags] [test] file` in the fixture's folder: stdout and
/// the exit status. Error text goes to stderr and is not contract.
fn run(fx: &Fixture, h: &Header, flags: &[&str]) -> (String, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    cmd.args(flags);
    if h.test {
        cmd.arg("test");
    }
    let out = cmd
        .arg(fx.file.file_name().unwrap())
        .current_dir(&fx.dir)
        .env("NO_COLOR", "1")
        .env_remove("OLANG_COMPAT_VAR")
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Whether `got` keeps the promise `expected` makes: the same text, or
/// for a test fixture, the expected lines in order among the runner's.
fn keeps(expected: &str, got: &str, test: bool) -> bool {
    if !test {
        return expected == got;
    }
    let mut lines = got.lines();
    expected
        .lines()
        .all(|want| lines.by_ref().any(|line| line == want))
}

#[test]
fn compat_programs_keep_their_output() {
    let allow = allow_list();
    let all = fixtures();
    assert!(
        all.len() >= 10,
        "tests/compat/programs has only {} programs",
        all.len()
    );
    let bless = std::env::var("OLANG_COMPAT_BLESS").is_ok_and(|v| v == "1");
    let mut broken = Vec::new();
    for fx in &all {
        let source = std::fs::read_to_string(&fx.file).unwrap();
        let h = header(&source);
        let (got, code) = run(fx, &h, &[]);
        if !fx.expected.exists() {
            // A new fixture: write what it prints, for review, only when
            // asked. An existing .out is never rewritten here.
            if bless && code == h.exit {
                std::fs::write(&fx.expected, &got).unwrap();
                println!("wrote {}", fx.expected.display());
                continue;
            }
            broken.push(format!(
                "{}: no {} (OLANG_COMPAT_BLESS=1 writes it for a new fixture)",
                fx.name,
                fx.expected.display()
            ));
            continue;
        }
        let expected = std::fs::read_to_string(&fx.expected).unwrap();
        let mut problems = Vec::new();
        if code != h.exit {
            problems.push(format!("exit status {code}, expected {}", h.exit));
        }
        if !keeps(&expected, &got, h.test) {
            problems.push(format!(
                "stdout differs\n--- expected\n{expected}\n--- got\n{got}"
            ));
        }
        // the tiers agree: the tree-walker gives the same result
        if !h.test {
            let (slow, slow_code) = run(fx, &h, &["--no-ovm"]);
            if slow != got || slow_code != code {
                problems.push(format!(
                    "--no-ovm differs from the default tiers\n--- default\n{got}\n--- --no-ovm\n{slow}"
                ));
            }
        }
        if problems.is_empty() {
            continue;
        }
        match allowed(&allow, &fx.name) {
            Some(a) => println!(
                "{}: changed, allowed in {} ({}, {}): {}",
                fx.name, a.version, a.kind, a.issue, a.reason
            ),
            None => broken.push(format!("{}: {}", fx.name, problems.join("\n"))),
        }
    }
    assert!(
        broken.is_empty(),
        "{} compatibility program{} broke the 1.x promise (docs/stability.md). \
         Fix the regression; a break that is intended goes in tests/compat/allow.toml, reviewed.\n\n{}",
        broken.len(),
        if broken.len() == 1 { "" } else { "s" },
        broken.join("\n\n")
    );
}

// ── the surface ─────────────────────────────────────────────────────

/// The Experimental modules (docs/stability.md): a minor release may
/// change or remove their members, so the surface leaves them out.
/// `toad` is in no tier yet (OL-629), so it is not promised here either.
const EXPERIMENTAL: &[&str] = &[
    "caps", "dom", "gui", "peg", "pty", "sim", "toad", "tty", "vt",
];

/// Single members outside the contract or on an Experimental surface
/// inside a Stable module: `http.serve` and what its handlers use (the
/// request/response API is Experimental; only the client is Stable).
const NOT_CONTRACT: &[&str] = &[
    "http.serve",
    "http.defer",
    "http.held",
    "http.hold",
    "http.notify",
    "http.notify_all",
    "http.respond",
    "http.response",
    "http.response_with_headers",
    "http.shutdown",
];

const COLLECTIONS: &[&str] = &["alg", "bitset", "deque", "dsu", "heap", "table"];

fn eval(source: &str) -> Option<Value> {
    let program = Parser::new().parse(source).ok()?;
    Interpreter::new().eval_program(program).ok()
}

fn use_line(module: &str) -> String {
    let root = module.split('.').next().unwrap_or(module);
    if olang::stdlib::embedded::is_embedded(root) && root != "collections" {
        format!("use {}\n", root)
    } else {
        String::new()
    }
}

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
        .filter_map(|item| match item {
            Value::Tuple(pair) => match (&pair[0], &pair[1]) {
                (Value::String(k), Value::String(t)) => Some((k.to_string(), t.to_string())),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Every name a program can reach today: `builtin <name>` for each
/// global builtin, `<module>.<member>` for each member of each module
/// that is not Experimental (submodules followed).
fn surface_now() -> BTreeSet<String> {
    let mut names: BTreeSet<String> = olang::builtin::BuiltinFunctions::global_names()
        .into_iter()
        .filter(|n| !n.starts_with("__"))
        .map(|n| format!("builtin {n}"))
        .collect();
    let mut modules: BTreeSet<String> = olang::stdlib::get_stdlib()
        .keys()
        .filter(|k| !k.starts_with("__"))
        .cloned()
        .collect();
    for name in olang::stdlib::embedded::names() {
        if COLLECTIONS.contains(&name) {
            modules.insert(format!("collections.{name}"));
        } else {
            modules.insert(name.to_string());
        }
    }
    let mut queue: Vec<String> = modules
        .into_iter()
        .filter(|m| !EXPERIMENTAL.contains(&m.split('.').next().unwrap_or(m)))
        .collect();
    while let Some(module) = queue.pop() {
        names.insert(format!("module {module}"));
        for (member, kind) in members(&module) {
            if member.starts_with("__") {
                continue;
            }
            let full = format!("{module}.{member}");
            if NOT_CONTRACT.contains(&full.as_str()) {
                continue;
            }
            if kind == "Module" && !names.contains(&format!("module {full}")) {
                queue.push(full.clone());
            }
            names.insert(full);
        }
    }
    names
}

#[test]
fn compat_surface_still_stands() {
    let dir = compat_dir().join("surface");
    let now = surface_now();
    if let Ok(v) = std::env::var("OLANG_COMPAT_SNAPSHOT") {
        let ok = v.split('.').count() == 2 && v.split('.').all(|p| p.parse::<u32>().is_ok());
        assert!(
            ok,
            "OLANG_COMPAT_SNAPSHOT is a major.minor, like 1.1, not `{v}`"
        );
        let path = dir.join(format!("{v}.txt"));
        assert!(
            !path.exists(),
            "{} exists: a release's surface is written once and never rewritten",
            path.display()
        );
        let mut text = format!(
            "# The stdlib surface of olang {v}: every global builtin and module member,\n\
             # the Experimental modules left out. Every later 1.x keeps each name\n\
             # (tests/compat_test.rs). Written once by OLANG_COMPAT_SNAPSHOT={v}; never edited.\n"
        );
        for n in &now {
            text.push_str(n);
            text.push('\n');
        }
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, text).unwrap();
        println!("wrote {} ({} names)", path.display(), now.len());
    }
    let allow = allow_list();
    let mut snapshots: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("tests/compat/surface")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    snapshots.sort();
    assert!(
        !snapshots.is_empty(),
        "tests/compat/surface has no snapshot"
    );
    let mut missing = Vec::new();
    for snap in &snapshots {
        let release = snap.file_stem().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(snap).unwrap();
        for name in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            if now.contains(name) {
                continue;
            }
            match allowed(&allow, &format!("surface:{name}")) {
                Some(a) => println!(
                    "surface:{name} (from {release}) is gone, allowed in {} ({}, {}): {}",
                    a.version, a.kind, a.issue, a.reason
                ),
                None => missing.push(format!("{name}  (in {release})")),
            }
        }
    }
    assert!(
        missing.is_empty(),
        "{} name{} of an earlier release's surface no longer exist{} (docs/stability.md: \
         the stdlib is append-mostly; a rename keeps the old name as an alias). Restore \
         {}, or list an intended removal in tests/compat/allow.toml as `surface:<name>`:\n  {}",
        missing.len(),
        if missing.len() == 1 { "" } else { "s" },
        if missing.len() == 1 { "s" } else { "" },
        if missing.len() == 1 { "it" } else { "them" },
        missing.join("\n  ")
    );
}

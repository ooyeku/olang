//! Whole-program tier agreement: every program in the corpus must produce
//! the same output on the tree-walking interpreter and on the compiled
//! tiers.
//!
//! `docs/ovm.md` makes the promise the three-tier design rests on:
//!
//! > a lower tier that cannot reproduce the interpreter's result exactly
//! > refuses to run the function rather than diverging
//!
//! That promise is the entire basis for trusting a runtime that silently
//! swaps engines underneath a program. Until this file existed it was
//! tested only by `bytecode_differential_test.rs`, which calls one
//! hand-written function with hand-written arguments — valuable, but it
//! cannot catch a divergence that needs a whole program to express.
//!
//! One did. Adding the `()` literal revealed that the interpreter read an
//! empty tuple as Unit while the bytecode compiler built an actual
//! zero-element tuple, so a hot function comparing `x == ()` answered
//! differently from a cold one. Nothing failed; the fast tier simply won.
//! It was found by accident, while adding an unrelated feature.
//!
//! So: run the real binary over the real corpus, twice, and diff. Every
//! runnable example from the book, plus the standalone example programs.
//! Comparison is over stdout, stderr and exit status together, because a
//! divergence that turns a value into an error is exactly the kind this
//! is looking for.
//!
//! The compiled side uses `--ovm-tier=1`, promoting every function on its
//! first call rather than waiting for it to get hot. A threshold that
//! never trips would make this file a very slow way to test nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

/// A program to check, and where it came from, so a failure names it.
struct Case {
    origin: String,
    code: String,
    /// Where to run it. Examples run in their own directory so relative
    /// imports and checked-in data resolve; doc snippets are
    /// self-contained and run in a scratch directory.
    in_place: Option<PathBuf>,
}

/// Every `olang` block in the book that is meant to run. `no-run` blocks
/// are parse-only by convention (they touch the filesystem or network),
/// and running them here would test the environment, not the tiers.
fn doc_cases() -> Vec<Case> {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let mut cases = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&docs)
        .expect("docs/ must exist")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();

    for path in files {
        let content = std::fs::read_to_string(&path).expect("readable doc");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let mut in_block = false;
        let mut skip = false;
        let mut start = 0usize;
        let mut code = String::new();

        for (idx, line) in content.lines().enumerate() {
            if let Some(info) = line.strip_prefix("```") {
                if in_block {
                    if !skip {
                        cases.push(Case {
                            origin: format!("{}:{}", name, start),
                            code: std::mem::take(&mut code),
                            in_place: None,
                        });
                    } else {
                        code.clear();
                    }
                    in_block = false;
                } else {
                    let info = info.trim();
                    if info.split_whitespace().next() == Some("olang") {
                        in_block = true;
                        skip = info.contains("no-run");
                        start = idx + 2;
                    }
                }
            } else if in_block {
                code.push_str(line);
                code.push('\n');
            }
        }
    }
    cases
}

/// Standalone example programs that run to completion on their own.
/// Servers and anything needing checked-in data are excluded — this file
/// tests tier agreement, not the examples' environments, and a program
/// that cannot run tells us nothing about whether two engines agree.
fn example_cases() -> Vec<Case> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut cases = Vec::new();
    for name in [
        "nbody",
        "minilisp",
        "parser",
        "regex",
        "template",
        "markdown",
        "jsonschema",
        "scheduler",
        "parmap",
        "workflow",
        "timeseries",
        "macros",
    ] {
        let dir = root.join(name);
        for entry in ["main.ol", "index.ol"] {
            let path = dir.join(entry);
            if path.exists()
                && let Ok(code) = std::fs::read_to_string(&path)
            {
                cases.push(Case {
                    origin: format!("examples/{}/{}", name, entry),
                    code,
                    in_place: Some(path.clone()),
                });
                break;
            }
        }
    }
    cases
}

/// Mask the things that legitimately differ between a tree-walker and a
/// compiled tier: elapsed times and throughputs. A program running 17×
/// faster on the compiled tier is the *point*, not a divergence. Anything
/// else — a value, an error, an ordering — is compared verbatim.
fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                i += 1;
            }
            let rest: String = bytes[i..].iter().take(20).collect();
            let number: String = bytes[start..i].iter().collect();
            // Only a number that is *labelled* as a duration or a rate;
            // a bare number is data and must still be compared.
            if rest.starts_with("ms")
                || rest.starts_with("x\n")
                || rest.starts_with(" interactions/sec")
                || rest.starts_with("s\n")
            {
                out.push_str("<T>");
            } else {
                out.push_str(&number);
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Run one program with the given extra flags, from a scratch directory
/// so any file a program writes cannot collide with another case.
fn run(case: &Case, dir: &Path, flags: &[&str]) -> (String, String, Option<i32>) {
    let (file, cwd) = match &case.in_place {
        Some(path) => (path.clone(), path.parent().unwrap().to_path_buf()),
        None => {
            let file = dir.join("case.ol");
            std::fs::write(&file, &case.code).unwrap();
            (file, dir.to_path_buf())
        }
    };
    let out = Command::new(olang())
        .args(flags)
        .arg("run")
        .arg(&file)
        .current_dir(&cwd)
        .output()
        .expect("olang runs");
    (
        normalise(&String::from_utf8_lossy(&out.stdout)),
        // Paths differ between the two runs' scratch dirs only if a
        // program prints one; normalise the directory out so an absolute
        // path in an error message is not a false mismatch.
        String::from_utf8_lossy(&out.stderr).replace(&*dir.to_string_lossy(), "<dir>"),
        out.status.code(),
    )
}

fn check(cases: Vec<Case>, label: &str) {
    assert!(
        !cases.is_empty(),
        "{label}: collected no programs — extraction broken?"
    );
    let base = std::env::temp_dir().join(format!("olang_tier_{}_{}", std::process::id(), label));
    let mut diverged = Vec::new();

    for (i, case) in cases.iter().enumerate() {
        let interp_dir = base.join(format!("{i}_interp"));
        let tiered_dir = base.join(format!("{i}_tiered"));
        std::fs::create_dir_all(&interp_dir).unwrap();
        std::fs::create_dir_all(&tiered_dir).unwrap();

        let interpreted = run(case, &interp_dir, &["--no-ovm"]);
        // Promote on the first call, so the compiled tier actually runs
        // rather than waiting for a hotness threshold this program may
        // never reach.
        let tiered = run(case, &tiered_dir, &["--ovm-tier=1"]);

        if interpreted != tiered {
            // Name the field that actually differs, and show it whole —
            // a truncated report of a divergence is how a divergence
            // stays unfixed.
            let mut what = Vec::new();
            if interpreted.0 != tiered.0 {
                what.push(format!(
                    "  stdout differs:\n    interpreter: {:?}\n    compiled:    {:?}",
                    interpreted.0, tiered.0
                ));
            }
            if interpreted.1 != tiered.1 {
                what.push(format!(
                    "  stderr differs:\n    interpreter: {:?}\n    compiled:    {:?}",
                    interpreted.1, tiered.1
                ));
            }
            if interpreted.2 != tiered.2 {
                what.push(format!(
                    "  exit differs: interpreter={:?} compiled={:?}",
                    interpreted.2, tiered.2
                ));
            }
            diverged.push(format!("\n── {} ──\n{}", case.origin, what.join("\n")));
        }
    }
    let _ = std::fs::remove_dir_all(&base);

    assert!(
        diverged.is_empty(),
        "{} program(s) produced different results on the interpreter and the \
         compiled tiers. Either the compiled tier is wrong, or it should have \
         refused to compile the function rather than running it — see \
         docs/ovm.md.\n{}",
        diverged.len(),
        diverged.join("")
    );
}

#[test]
fn every_runnable_book_example_agrees_across_tiers() {
    check(doc_cases(), "docs");
}

#[test]
fn every_standalone_example_program_agrees_across_tiers() {
    check(example_cases(), "examples");
}

/// The harness has to be able to see a divergence, or it is decoration.
/// This asserts the comparison itself: two programs that genuinely differ
/// must compare unequal on the same inputs the real check uses.
#[test]
fn the_comparison_can_actually_detect_a_difference() {
    let dir = std::env::temp_dir().join(format!("olang_tier_selftest_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let one = Case {
        origin: "self".into(),
        code: "println(\"one\")\n".into(),
        in_place: None,
    };
    let two = Case {
        origin: "self".into(),
        code: "println(\"two\")\n".into(),
        in_place: None,
    };
    let a = run(&one, &dir, &["--no-ovm"]);
    let b = run(&two, &dir, &["--ovm-tier=1"]);
    assert_ne!(a, b, "the comparison cannot distinguish different output");

    let same = Case {
        origin: "self".into(),
        code: "println(\"same\")\n".into(),
        in_place: None,
    };
    let same_a = run(&same, &dir, &["--no-ovm"]);
    let same_b = run(&same, &dir, &["--ovm-tier=1"]);
    assert_eq!(same_a, same_b, "identical programs must compare equal");
    let _ = std::fs::remove_dir_all(&dir);
}

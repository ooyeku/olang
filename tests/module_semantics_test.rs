//! Two module-system semantics pinned by the web-sdk build:
//!
//! 1. A `share use` re-export keeps the closure its own module gave
//!    it. Re-closing it over the re-exporting module's scope stripped
//!    access to the source module's private helpers — an aggregator
//!    index re-exporting `action` from a module with private cells
//!    died with "Undefined variable" at the first call.
//!
//! 2. Test blocks are inert outside `olang test`. They used to execute
//!    inline on every normal run — so `use` of any tested library ran
//!    its whole suite (prints, asserts, side effects) at import time.

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run_in(dir: &std::path::Path, rel: &str) -> (bool, String) {
    let out = Command::new(olang())
        .arg("run")
        .arg(rel)
        .current_dir(dir)
        .output()
        .expect("spawn olang");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn reexports_keep_their_module_closure() {
    let base = std::env::temp_dir().join(format!("olang_reexp_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("lib")).unwrap();
    // inner: a share fn leaning on a private module-level cell.
    std::fs::write(
        base.join("lib/inner.ol"),
        "let counter = cell.new(0)\n\
         share fn bump() = {\n\
             cell.update(counter, (n) => n + 1)\n\
             cell.get(counter)\n\
         }\n",
    )
    .unwrap();
    // index: re-exports bump without declaring it.
    std::fs::write(base.join("index.ol"), "share use lib.inner { bump }\n").unwrap();
    std::fs::write(
        base.join("olang.toml"),
        "[package]\nname = \"agg\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        base.join("main.ol"),
        "use agg { bump }\nprintln(to_string(bump() + bump()))\n",
    )
    .unwrap();
    let (ok, out) = run_in(&base, "main.ol");
    assert!(ok, "re-exported fn must reach its private cell:\n{out}");
    assert!(
        out.contains("3"),
        "1 + 2 through the private counter:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_blocks_are_inert_outside_the_runner() {
    let base = std::env::temp_dir().join(format!("olang_inert_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("lib")).unwrap();
    std::fs::write(
        base.join("lib/noisy.ol"),
        "share fn f(x) = x + 1\n\
         test \"screams\" {\n\
             println(\"TEST-SIDE-EFFECT\")\n\
             assert_eq(f(1), 999)\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        base.join("main.ol"),
        "use lib.noisy { f }\n\
         test \"local\" { println(\"LOCAL-TEST\") }\n\
         println(\"result=\" + to_string(f(41)))\n",
    )
    .unwrap();
    // A normal run: no test output, no failing assertion, program runs.
    let (ok, out) = run_in(&base, "main.ol");
    assert!(
        ok,
        "the failing test block must not abort a normal run:\n{out}"
    );
    assert!(out.contains("result=42"), "{out}");
    assert!(
        !out.contains("TEST-SIDE-EFFECT") && !out.contains("LOCAL-TEST"),
        "test blocks must be inert outside `olang test`:\n{out}"
    );

    // The runner still runs them (and reports the failure).
    let out = Command::new(olang())
        .arg("test")
        .arg(&base)
        .output()
        .expect("spawn olang test");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("TEST-SIDE-EFFECT"), "runner executes: {text}");
    assert!(
        text.contains("failed"),
        "runner reports the bad assert: {text}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

//! `olang test --coverage` — line coverage from the test runner. An
//! end-to-end test: it invokes the built `olang` binary on a two-file
//! fixture (a library and a test that exercises part of it) and checks
//! that coverage is attributed to the file the code lives in, with the
//! uncovered lines correctly identified.

use std::process::Command;

fn olang_bin() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn tmp(sub: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("olang_cov_{}_{}", sub, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn coverage_attributes_lines_across_files() {
    let dir = tmp("cross");

    // A library with three functions; the test below calls only two, so the
    // third's body is the coverage gap. Using `calc` (not `mathx`) avoids
    // colliding with the embedded package of that name.
    std::fs::write(
        dir.join("calc.ol"),
        "share fn double(x) = x * 2\n\
         \n\
         share fn triple(x) = {\n\
         \x20   let a = x * 3\n\
         \x20   a\n\
         }\n\
         \n\
         share fn never_called(x) = {\n\
         \x20   let b = x - 1\n\
         \x20   b\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("calc_test.ol"),
        "use calc { double, triple }\n\
         \n\
         test \"double works\" {\n\
         \x20   assert(double(4) == 8)\n\
         }\n\
         \n\
         test \"triple works\" {\n\
         \x20   assert(triple(2) == 6)\n\
         }\n",
    )
    .unwrap();

    let out = Command::new(olang_bin())
        .args(["test", dir.to_str().unwrap(), "--coverage-lines"])
        .output()
        .expect("run olang test --coverage-lines");
    let stdout = String::from_utf8_lossy(&out.stdout);

    // The suite itself passes.
    assert!(out.status.success(), "test run failed: {stdout}");
    assert!(stdout.contains("2 passed, 0 failed"), "stdout: {stdout}");

    // A coverage section is printed, and the library file appears in it —
    // its lines attributed to calc.ol, not to the test file that ran them.
    assert!(stdout.contains("coverage"), "no coverage section: {stdout}");
    assert!(stdout.contains("calc.ol"), "library not reported: {stdout}");

    // The only uncovered code is `never_called`'s body (its two block lines,
    // 9-10). The report lists exactly that range, proving the gap is found
    // and correctly located.
    assert!(
        stdout.contains("uncovered 9-10"),
        "expected the never-called body (lines 9-10) as the gap: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

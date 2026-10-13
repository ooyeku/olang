//! The command line's traps (the freeze audit): a mistyped command is
//! suggested, not read as a file; run options are taken after `run`; what
//! follows the program file is the program's; a missing path exits 2; and
//! `olang test --fail-if-none` fails a run that ran nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_cli_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.ol"), "println(os.args())\n").unwrap();
    std::fs::write(dir.join("t.ol"), "fn f() = 1\n").unwrap();
    dir
}

fn olang(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .output()
        .expect("run olang");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn a_mistyped_command_is_suggested_and_exits_2() {
    let ws = workspace("typo");
    let (out, code) = olang(&ws, &["chek", "."]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("no such file or command: chek — did you mean `olang check`?"),
        "{out}"
    );
    let (out, code) = olang(&ws, &["--ovm-stats", "json", "a.ol"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("--ovm-stats takes its value after `=`"),
        "{out}"
    );
}

#[test]
fn a_missing_path_exits_2_for_every_command() {
    let ws = workspace("missing");
    for args in [
        &["nosuch.ol"][..],
        &["run", "nosuch.ol"],
        &["check", "nosuch"],
        &["test", "nosuch"],
        &["fmt", "nosuch.ol"],
        &["build", "nosuch.ol"],
        &["doctor", "nosuch"],
        &["inspect", "nosuch"],
    ] {
        let (out, code) = olang(&ws, args);
        assert_eq!(code, 2, "{args:?}: {out}");
        assert!(
            out.contains("no such file or directory: nosuch"),
            "{args:?}: {out}"
        );
    }
}

#[test]
fn run_options_are_taken_after_run_and_the_programs_after_the_file() {
    let ws = workspace("run");
    for mode in [&["run"][..], &["--no-ovm", "run"]] {
        let mut args = mode.to_vec();
        args.extend([
            "--deny",
            "net",
            "--max-depth",
            "50",
            "a.ol",
            "x",
            "--deny",
            "y",
        ]);
        let (out, code) = olang(&ws, &args);
        assert_eq!(code, 0, "{out}");
        assert_eq!(out.trim(), "[\"a.ol\", \"x\", \"--deny\", \"y\"]");
    }
    // `--version` after the file is the program's
    let (out, code) = olang(&ws, &["a.ol", "--version", "--verbose"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.trim(), "[\"a.ol\", \"--version\", \"--verbose\"]");
    let (out, _) = olang(&ws, &["--version"]);
    assert!(out.starts_with("olang "), "{out}");
}

#[test]
fn fail_if_none_fails_a_run_that_ran_no_test() {
    let ws = workspace("none");
    let (out, code) = olang(&ws, &["test", "t.ol"]);
    assert_eq!(code, 0, "{out}");
    let (out, code) = olang(&ws, &["test", "t.ol", "--fail-if-none"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("no test block ran"), "{out}");
    std::fs::write(ws.join("u_test.ol"), "test \"one\" { assert_eq(1, 1) }\n").unwrap();
    let (out, code) = olang(&ws, &["test", "u_test.ol", "--fail-if-none"]);
    assert_eq!(code, 0, "{out}");
    let (out, code) = olang(
        &ws,
        &["test", "u_test.ol", "--fail-if-none", "--only", "zzz"],
    );
    assert_eq!(code, 1, "{out}");
}

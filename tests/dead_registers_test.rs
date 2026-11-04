//! A register whose value is dead lets it go (OL-411).
//!
//! On the bytecode tier a variable lives in a register, and so does every
//! temporary. A write nobody read again was dropped as a dead move, which
//! left the old value in place; a loop's body kept its last value in its
//! registers after the loop. Either held a dead value until the function
//! returned: `x = ()` freed nothing, and a long-running loop that never
//! returns (an app's event loop) held the previous turn's state.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_deadreg_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, source: &str) -> String {
    std::fs::write(dir.join("main.ol"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("main.ol")
        .current_dir(dir)
        .env("OLANG_COMPILE_CACHE", "0")
        .output()
        .expect("run olang");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Each case prints the KB it still holds once its big value is dead; a
/// list of 100,000 strings is about 7,800 KB.
const CASES: &str = r#"fn heap() = runtime.memory().heap
fn mk(n) = { let mut out = []; for i in 0..n { out = out + [to_string(i)] }; out }
fn t_reassign() = {
    let h0 = heap()
    let mut x = mk(100000)
    x = ()
    println(to_string((heap() - h0) / 1024))
}
fn t_built_here() = {
    let h0 = heap()
    let mut x = [1]
    for i in 0..100000 { x = x + [to_string(i)] }
    x = ()
    println(to_string((heap() - h0) / 1024))
}
fn t_while() = {
    let h0 = heap()
    let mut i = 0
    while i < 3 { let tmp = mk(100000); i = i + 1 }
    println(to_string((heap() - h0) / 1024))
}
fn t_for() = {
    let h0 = heap()
    for k in 0..3 { let tmp = mk(100000) }
    println(to_string((heap() - h0) / 1024))
}
fn t_nested() = {
    let h0 = heap()
    let mut n = 0
    for k in 0..2 { for j in 0..2 { let tmp = mk(50000); n = n + len(tmp) } }
    println(to_string((heap() - h0) / 1024))
}
t_reassign()
t_built_here()
t_while()
t_for()
t_nested()
"#;

#[test]
fn a_dead_value_is_let_go_before_the_function_returns() {
    let ws = workspace("cases");
    let out = run(&ws, CASES);
    let held: Vec<i64> = out.lines().map(|l| l.trim().parse().unwrap()).collect();
    let names = [
        "x = ()",
        "x built in place, then ()",
        "a while body's let",
        "a for body's let",
        "a nested loop's let",
    ];
    assert_eq!(held.len(), names.len(), "{out}");
    for (name, kb) in names.iter().zip(&held) {
        assert!(
            *kb < 512,
            "{name}: still holds {kb} KB after its value is dead\n{out}"
        );
    }
}

/// A `for` loop's variable is the loop's own, as on the interpreter: an
/// outer binding of its name is the outer one again after the loop, and
/// a name only the loop bound is gone.
#[test]
fn a_for_variable_does_not_outlive_its_loop() {
    let ws = workspace("forvar");
    let out = run(
        &ws,
        "fn outer() = {\n  let x = 5\n  for x in [1, 2] { }\n  x\n}\n\
         fn counted() = {\n  let i = 7\n  for i in 0..3 { }\n  i\n}\n\
         fn gone() = {\n  for y in [1, 2] { }\n  y\n}\n\
         println(outer())\nprintln(counted())\nprintln(is_err(attempt(() => gone())))\n",
    );
    assert_eq!(out, "5\n7\ntrue\n");
}

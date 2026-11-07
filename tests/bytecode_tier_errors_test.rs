//! Two ways the bytecode tier and the JIT went wrong where the
//! interpreter did not, each checked on both tiers through the binary.

use std::process::Command;

fn run(src: &str, args: &[&str]) -> (bool, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.ol");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .arg(&file)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn both_tiers(src: &str) -> String {
    let (ok, vm, err) = run(src, &[]);
    assert!(ok, "bytecode tier: {err}");
    let (ok, interp, err) = run(src, &["--no-ovm"]);
    assert!(ok, "interpreter: {err}");
    assert_eq!(vm, interp, "the tiers disagree");
    vm
}

/// OL-518: an Int division or modulo by zero in a function `attempt`'s
/// closure calls through a function value is the `Err`, as every other
/// runtime error there is; on the bytecode tier it ended the run.
#[test]
fn attempt_catches_division_by_zero_through_a_function_value() {
    let out = both_tiers(
        r#"
fn int_div_fn(s) = s / 0
fn int_mod_fn(s) = s % 0
fn run(f, s) = attempt(() => f(s))
let up = (s) => s / 0
println(run(up, 4))
println(run(int_div_fn, 4))
println(run(int_mod_fn, 4))
let rec = { h: int_div_fn }
println(attempt(() => rec.h(3)))
"#,
    );
    assert_eq!(
        out,
        "Err(\"Division by zero\")\nErr(\"Division by zero\")\nErr(\"Modulo by zero\")\nErr(\"Division by zero\")\n"
    );
}

/// OL-519: a group the JIT could not finish left its members' symbols
/// declared and unbodied, and the next group's finalize panicked ("can't
/// resolve symbol olang_jit_N_gM") on the members it had defined.
#[test]
fn a_failed_jit_group_does_not_break_the_next_one() {
    let out = both_tiers(
        r#"
fn l_struct_int(n) = {
    let p = { a: 3 }
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + p.a
        i = i + 1
    }
    acc
}
fn l_rec_int(n) = {
    let p = { a: 3, xs: [1, 2, 3, 4, 5] }
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + p.a
        i = i + 1
    }
    acc
}
fn l_index(n) = {
    let xs = [1, 2, 3, 4, 5]
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + xs[i % 5]
        i = i + 1
    }
    acc
}
fn t(f) = println(f())
fn main() = {
    t(() => l_struct_int(100000))
    t(() => l_rec_int(100000))
    t(() => l_index(100000))
}
main()
"#,
    );
    assert_eq!(out, "300000\n300000\n300000\n");
}

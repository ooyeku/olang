//! The W2 rulings, pinned as differentials against `--no-ovm`: a Float is
//! always finite (overflow raises on every tier, math functions included,
//! and out-of-range text does not parse), and Series division between Int
//! columns is true division.

use std::process::Command;

fn run(source: &str, no_ovm: bool) -> (String, i32) {
    let dir = std::env::temp_dir().join("olang_w2_rulings_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let path = dir.join(format!("t{:x}.ol", h.finish()));
    std::fs::write(&path, source).expect("write");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    if no_ovm {
        cmd.arg("--no-ovm");
    }
    let out = cmd.arg("run").arg(&path).output().expect("run");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

fn agree(source: &str) -> (String, i32) {
    let (tiered, rc_t) = run(source, false);
    let (oracle, rc_o) = run(source, true);
    assert_eq!(tiered, oracle, "tiers diverged on:\n{source}");
    assert_eq!(rc_t, rc_o);
    (tiered, rc_t)
}

#[test]
fn float_overflow_raises_on_every_tier() {
    for (expr, op) in [
        ("1e308 * 10.0", "multiplication"),
        ("1e308 + 1e308", "addition"),
        ("(0.0 - 1e308) - 1e308", "subtraction"),
        ("1e308 / 0.1", "division"),
    ] {
        let (out, rc) = agree(&format!("println(to_string({expr}))"));
        assert_ne!(rc, 0, "{expr} must fail: {out}");
        assert!(
            out.contains(&format!("Float overflow in {op}")),
            "{expr}: {out}"
        );
        assert!(!out.contains("inf"), "{expr} leaked inf: {out}");
    }
}

#[test]
fn a_hot_float_loop_overflows_the_same_way_under_the_jit() {
    // grow() is called enough to be compiled before the overflowing call.
    let (out, rc) = agree(
        "fn grow(x, n) = {\n\
             let mut v = x\n\
             for i in range(0, n) { v = v * 2.0 }\n\
             v\n\
         }\n\
         let mut acc = 0.0\n\
         for i in range(0, 3000) { acc = acc + grow(1.5, 20) }\n\
         println(to_string(acc > 0.0))\n\
         println(to_string(grow(1e300, 100)))",
    );
    assert_ne!(rc, 0, "{out}");
    assert_eq!(out.lines().next(), Some("true"));
    assert!(out.contains("Float overflow in multiplication"), "{out}");
}

#[test]
fn math_functions_and_parsing_never_produce_inf() {
    let (out, rc) = agree(
        "println(show(str.parse_float(\"1e999\")))\n\
         println(show(str.parse_float(\"1e300\")))\n\
         println(to_string(math.pow(10.0, 300.0) > 0.0))",
    );
    assert_eq!(rc, 0, "{out}");
    assert!(
        out.contains("Err(\"'1e999' is out of range for a float\")"),
        "{out}"
    );
    assert!(out.contains("Ok(1e300)"), "{out}");
    for src in [
        "println(to_string(math.pow(10.0, 400.0)))",
        "println(to_string(math.exp(1000.0)))",
        "println(to_string(to_float(\"1e999\")))",
    ] {
        let (out, rc) = agree(src);
        assert_ne!(rc, 0, "{src}: {out}");
        assert!(!out.contains("inf"), "{src} leaked inf: {out}");
    }
}

#[test]
fn an_out_of_range_float_literal_is_a_syntax_error() {
    let (out, rc) = run("println(to_string(1e400))", false);
    assert_ne!(rc, 0);
    assert!(out.contains("out of range"), "{out}");
    let (out, rc) = run("println(to_string(1.7e308))", false);
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out.trim(), "1.7e308");
}

#[test]
fn series_division_between_int_columns_is_true_division() {
    let (out, rc) = agree(
        "let counts = ods.series([1, 2, 3])\n\
         println(to_string(ods.to_list(counts / 2)))\n\
         println(to_string(ods.to_list(counts / ods.series([2, 4, 4]))))\n\
         println(to_string(ods.to_list(6 / counts)))\n\
         println(to_string(ods.to_list(ods.cast(counts / 2, \"Int\"))))\n\
         println(to_string(ods.to_list(counts * 2)))\n\
         println(typeof(ods.to_list(counts / 2)[0]))\n\
         // The climate shape: growth between two Int measure columns.\n\
         let now = ods.series([110, 250, 300])\n\
         let then = ods.series([100, 200, 300])\n\
         println(to_string(ods.to_list((now - then) / then)))",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        out,
        "[0.5, 1.0, 1.5]\n[0.5, 0.5, 0.75]\n[6.0, 3.0, 2.0]\n[0, 1, 1]\n[2, 4, 6]\nFloat\n[0.1, 0.25, 0.0]\n"
    );
}

#[test]
fn series_division_by_zero_still_raises() {
    let (out, rc) =
        agree("println(to_string(ods.to_list(ods.series([1, 2]) / ods.series([1, 0]))))");
    assert_ne!(rc, 0, "{out}");
    assert!(out.contains("Division by zero"), "{out}");
}

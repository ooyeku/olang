//! `assert_close(actual, expected, tolerance, message?)` (roadmap W8):
//! the tolerance-based numeric assertion beside the existing asserts,
//! with the same everywhere-an-expression reach and raising semantics.
//! Every runtime case is a differential against `--no-ovm`.

use std::process::Command;

fn run(source: &str, no_ovm: bool) -> (String, i32) {
    let dir = std::env::temp_dir().join("olang_assert_close_tests");
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

fn assert_tiers_agree(source: &str) -> (String, i32) {
    let (tiered, rc_t) = run(source, false);
    let (oracle, rc_o) = run(source, true);
    assert_eq!(tiered, oracle, "tiers diverged on:\n{source}");
    assert_eq!(rc_t, rc_o);
    (tiered, rc_t)
}

#[test]
fn passes_within_tolerance_and_mixes_ints_and_floats() {
    let (out, rc) = assert_tiers_agree(
        "assert_close(0.1 + 0.2, 0.3, 0.000001)\n\
         assert_close(10, 10.4, 0.5)\n\
         assert_close(3, 3, 0)\n\
         assert_close(-1.5, -1.4, 0.2)\n\
         println(\"all close\")",
    );
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("all close"));
}

#[test]
fn fails_with_the_difference_in_the_message() {
    let (out, rc) = assert_tiers_agree("assert_close(1.0, 2.0, 0.5)");
    assert_ne!(rc, 0);
    assert!(out.contains("is not within"), "{out}");
    assert!(out.contains("difference"), "{out}");
}

#[test]
fn a_custom_message_replaces_the_default() {
    let (out, _) = assert_tiers_agree("assert_close(1.0, 2.0, 0.5, \"way off\")");
    assert!(out.contains("way off"), "{out}");
    assert!(!out.contains("is not within"), "{out}");
}

#[test]
fn nan_fails_and_negative_tolerance_errors() {
    let (out, rc) = assert_tiers_agree(
        "let nan = 1e308 * 10.0 - 1e308 * 10.0\n\
         assert_close(nan, 0.0, 1.0)",
    );
    // Whatever the current float-overflow stance produces, the two
    // tiers must agree; if the arithmetic yields NaN the assert fails.
    assert_ne!(rc, 0, "{out}");

    let (out, rc) = assert_tiers_agree("assert_close(1.0, 1.0, -0.1)");
    assert_ne!(rc, 0);
    assert!(out.contains("tolerance must be non-negative"), "{out}");
}

#[test]
fn non_numeric_arguments_error_by_name() {
    let (out, rc) = assert_tiers_agree("assert_close(\"a\", 1.0, 0.1)");
    assert_ne!(rc, 0);
    assert!(out.contains("actual must be a number"), "{out}");
}

#[test]
fn works_in_expression_positions_and_test_blocks() {
    let (out, rc) = assert_tiers_agree(
        "let checked = map([1.0, 2.0, 3.0], (x) => {\n\
             assert_close(x * 2.0 / 2.0, x, 0.0000001)\n\
             x\n\
         })\n\
         println(len(checked))\n\
         let arm = match 5 { n => { assert_close(to_float(n), 5.0, 0.1) ; \"ok\" } }\n\
         println(arm)",
    );
    assert_eq!(rc, 0, "{out}");

    // And under the test runner.
    let dir = std::env::temp_dir().join("olang_assert_close_testblock");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("close_test.ol"),
        "fn near(x) = x + 0.0000001\n\
         test \"assert_close inside a test block\" {\n\
             assert_close(near(1.0), 1.0, 0.001)\n\
         }\n",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("test")
        .arg(&dir)
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("1 passed"), "{text}");
}

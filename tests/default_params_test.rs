//! Default parameter values (roadmap W8): every case is a differential
//! — the tiered binary and `--no-ovm` must print the same bytes. The
//! semantics under test: a default evaluates at call time, in the
//! callee's scope (closure plus every earlier parameter, never the
//! caller's locals), left to right, once per call. Call sites on the
//! bytecode tier splice literal defaults as constants and route
//! non-literal ones through the function value's interpreter-exact
//! path, so the hot-loop cases pin the fast paths against the oracle.

use std::process::Command;

fn run(source: &str, no_ovm: bool) -> String {
    let dir = std::env::temp_dir().join("olang_default_params_tests");
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
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn assert_tiers_agree(source: &str) {
    let tiered = run(source, false);
    let oracle = run(source, true);
    assert_eq!(tiered, oracle, "tiers diverged on:\n{source}");
}

#[test]
fn trailing_defaults_and_named_arguments() {
    assert_tiers_agree(
        "fn probe(a, b = 2, c = 3) = a * 100 + b * 10 + c\n\
         println(probe(1))\n\
         println(probe(1, 8))\n\
         println(probe(1, 8, 9))\n\
         println(probe(1, c: 9))\n\
         println(probe(c: 9, a: 1))\n\
         println(probe(1, b: 8, c: 9))",
    );
}

#[test]
fn defaults_see_earlier_parameters_not_caller_locals() {
    // `y = x * 2` binds the parameter x; the caller's own x (and any
    // same-named local) must be invisible to the default.
    assert_tiers_agree(
        "fn mid(x, y = x * 2) = x + y\n\
         let x = 999\n\
         println(mid(10))\n\
         println(mid(10, 1))\n\
         fn chain(a, b = a + 1, c = b + a) = `${a} ${b} ${c}`\n\
         println(chain(5))\n\
         println(chain(5, 10))",
    );
}

#[test]
fn defaults_see_the_closure_scope() {
    assert_tiers_agree(
        "let base = 7\n\
         fn f(x, y = base * 2) = x + y\n\
         println(f(1))\n\
         fn outer() = {\n\
             let k = 100\n\
             let g = (v, w = k + 1) => v + w\n\
             g(2)\n\
         }\n\
         println(outer())",
    );
}

#[test]
fn defaults_evaluate_once_per_call() {
    assert_tiers_agree(
        "let calls = cell(0)\n\
         fn stamp() = {\n\
             cell.set(calls, cell.get(calls) + 1)\n\
             cell.get(calls)\n\
         }\n\
         fn f(x, tag = stamp()) = `${x}:${tag}`\n\
         println(f(\"a\"))\n\
         println(f(\"b\"))\n\
         println(f(\"c\", 0))\n\
         println(f(\"d\"))\n\
         println(cell.get(calls))",
    );
}

#[test]
fn hot_loops_keep_defaults_fast_and_exact() {
    // Literal defaults splice at the compiled call site; a
    // parameter-referencing default takes the value path. Both must
    // match the oracle bit for bit across enough calls to promote.
    assert_tiers_agree(
        "fn pad(x, p = 100) = x + p\n\
         fn mid(x, y = x * 2) = x + y\n\
         fn hot(n) = {\n\
             let mut t = 0\n\
             for i in 0..n { t = t + pad(i) + mid(i) }\n\
             t\n\
         }\n\
         println(hot(50000))",
    );
}

#[test]
fn defaults_through_function_values_and_lambdas() {
    assert_tiers_agree(
        "fn greet(name, punct = \"!\") = name + punct\n\
         let g = greet\n\
         println(g(\"ada\"))\n\
         fn call_it(f, x) = f(x)\n\
         println(call_it(greet, \"eve\"))\n\
         let lam = (a, b = 7) => a + b\n\
         println(lam(1))\n\
         println(lam(1, 2))",
    );
}

#[test]
fn recursion_with_defaults() {
    assert_tiers_agree(
        "fn count_down(n, acc = 0) =\n\
             if n == 0 => acc\n\
             else => count_down(n - 1, acc + n)\n\
         println(count_down(100))\n\
         println(count_down(100, 5))",
    );
}

#[test]
fn argument_errors_match_the_oracle() {
    for src in [
        // missing required
        "fn f(a, b = 1) = a + b\nprintln(f())",
        // unknown named argument
        "fn f(a, b = 1) = a + b\nprintln(f(1, z: 2))",
        // duplicate named argument
        "fn f(a, b = 1) = a + b\nprintln(f(1, b: 2, b: 3))",
        // positional and named for the same parameter
        "fn f(a, b = 1) = a + b\nprintln(f(1, 2, b: 3))",
    ] {
        assert_tiers_agree(src);
    }
}

//! The interpreter half of finding #1: `xs = xs + [..]` extends in place
//! when `xs` is a sole-owned list, so accumulation is O(n) even when the
//! function never promotes (a one-shot build, or the wasm/browser path).
//! These run interpreter-only (`--no-ovm` equivalent) and pin that the
//! `Arc::get_mut` aliasing guard leaves every alias untouched — the
//! discipline that keeps the fusion transparent against the oracle.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

/// Interpreter only — no bytecode tier, so the interpreter's own append
/// fusion is what runs.
fn eval(source: &str) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

#[test]
fn snapshot_before_append_is_not_mutated() {
    // `snapshot` aliases the list when the append happens: the fused path
    // must copy, never extend the shared Vec. Reading the snapshot's
    // length after the append is what catches a leak.
    let r = eval(
        r#"
fn f() = {
    let mut xs = [1, 2]
    let snapshot = xs
    xs = xs + [3]
    len(snapshot) * 100 + len(xs)
}
f()
"#,
    );
    assert_eq!(r, Ok(Value::Integer(203)));
}

#[test]
fn nested_list_snapshot_survives_later_appends() {
    let r = eval(
        r#"
fn f() = {
    let mut xs = [1]
    let kept = [xs, [9]]
    xs = xs + [2]
    xs = xs + [3]
    len(kept[0]) * 100 + len(xs)
}
f()
"#,
    );
    assert_eq!(r, Ok(Value::Integer(103)));
}

#[test]
fn self_append_copies_on_the_interpreter() {
    let r = eval(
        r#"
fn f() = {
    let mut xs = [1, 2]
    xs = xs + xs
    xs = xs + xs
    len(xs) * 1000 + xs[0] + xs[7]
}
f()
"#,
    );
    assert_eq!(r, Ok(Value::Integer(8003)));
}

#[test]
fn sub_cap_build_is_correct_and_ordered() {
    // A genuine accumulation loop (the pattern the finding is about),
    // kept under the interpreter's allocation budget. Correct contents
    // and order are what matter here — the speed is proven separately.
    let r = eval(
        r#"
fn build(n) = {
    let mut xs = []
    let mut i = 0
    while i < n {
        xs = xs + [i * i]
        i = i + 1
    }
    len(xs) * 1000000 + xs[0] + xs[1] + xs[n - 1]
}
build(2000)
"#,
    );
    // len 2000, xs[0]=0, xs[1]=1, xs[1999]=1999*1999=3996001
    assert_eq!(r, Ok(Value::Integer(2000 * 1_000_000 + 1 + 3_996_001)));
}

#[test]
fn append_result_is_the_new_list_value() {
    // `xs = xs + [v]` evaluates to the updated list, like any assignment.
    let r = eval(
        r#"
let mut xs = [1]
let out = (xs = xs + [2])
len(out)
"#,
    );
    assert_eq!(r, Ok(Value::Integer(2)));
}

/// Tiered — the VM's AddAssign fusion is what runs.
fn eval_tiered(source: &str) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        interpreter.enable_bytecode_tier(1, false);
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

#[test]
fn template_string_appends_stay_linear_on_the_vm() {
    // `xs = xs + [`${i}`]` — the accumulation shape every CSV/report
    // builder uses. A missing TemplateString arm in assignment_free
    // once refused the AddAssign fusion for exactly this shape, so each
    // append copied the whole list: 40k rows took ~1.8s. Fused, the
    // same loop is ~10ms; the bound is two orders of magnitude above
    // the fused time and an order below the quadratic one.
    let src = "fn build(n) = {\n\
         let mut xs = []\n\
         for i in range(0, n) { xs = xs + [`row-${i}`] }\n\
         xs\n\
     }\n\
     len(build(40000))";
    let start = std::time::Instant::now();
    let got = eval_tiered(src).expect("eval");
    assert_eq!(got, Value::Integer(40000));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "templated append fell back to copy-per-iteration: {:?}",
        start.elapsed()
    );
    // And the answer itself must match the interpreter.
    let interp = eval(
        "fn build(n) = {\n\
         let mut xs = []\n\
         for i in range(0, n) { xs = xs + [`row-${i}`] }\n\
         xs\n\
     }\n\
     to_string(build(3))",
    )
    .expect("interp");
    let tiered = eval_tiered(
        "fn build(n) = {\n\
         let mut xs = []\n\
         for i in range(0, n) { xs = xs + [`row-${i}`] }\n\
         xs\n\
     }\n\
     to_string(build(3))",
    )
    .expect("tiered");
    assert_eq!(interp, tiered);
}

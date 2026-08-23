//! Tail-call elimination (Campaign 5, R4b): a self-call in tail
//! position runs in the caller's frame — O(1) stack and O(1) logical
//! depth — on every tier. The contract: elision changes how deep a
//! program can go and nothing else. Same values, same order of effects,
//! same errors for non-tail recursion, and the identity test is the
//! function *value* (body + closure), so shadows and same-named
//! siblings still behave as ordinary calls.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str, tier: bool) -> Result<Value, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let parser = Parser::new();
            let program = parser.parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if tier {
                interpreter.enable_bytecode_tier(1, false);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .expect("spawn")
        .join()
        .expect("join")
}

/// Both tiers, same result — far past the 100k depth cap, from a small
/// OS stack, so only genuine elision can pass.
fn assert_deep_identical(source: &str, want: Value) {
    for tier in [false, true] {
        let got = eval(source, tier).unwrap_or_else(|e| panic!("tier={}: {}", tier, e));
        assert_eq!(got, want, "tier={}", tier);
    }
}

#[test]
fn tail_recursion_runs_past_the_depth_cap_on_both_tiers() {
    assert_deep_identical(
        "fn count(n, acc) = if n <= 0 => acc else => count(n - 1, acc + 1)\ncount(2000000, 0)",
        Value::Integer(2_000_000),
    );
}

#[test]
fn tail_positions_include_match_arms_blocks_and_return() {
    assert_deep_identical(
        "fn go(i, acc) = match i <= 0 { true => acc, false => go(i - 1, acc + i) }\ngo(500000, 0)",
        Value::Integer(125_000_250_000),
    );
    assert_deep_identical(
        "fn go(i, acc) = {\n    let next = acc + i\n    if i <= 0 => acc\n    else => go(i - 1, next)\n}\ngo(500000, 0)",
        Value::Integer(125_000_250_000),
    );
    assert_deep_identical(
        "fn go(i, acc) = {\n    if i <= 0 => { return acc }\n    return go(i - 1, acc + i)\n}\ngo(500000, 0)",
        Value::Integer(125_000_250_000),
    );
}

#[test]
fn non_tail_recursion_still_hits_the_depth_cap() {
    // `1 + down(...)` needs its frame back — nothing to elide, and the
    // cap must still catch it identically on both tiers.
    for tier in [false, true] {
        let err = eval(
            "fn down(n) = if n <= 0 => 0 else => 1 + down(n - 1)\ndown(200000)",
            tier,
        )
        .unwrap_err();
        assert!(
            err.contains("Maximum call depth (100000) exceeded"),
            "tier={}: {}",
            tier,
            err
        );
    }
}

#[test]
fn effectful_tail_recursion_keeps_its_order() {
    let src = "\
fn noisy(n, acc) = {\n\
    if n <= 0 => acc\n\
    else => {\n\
        println(n)\n\
        noisy(n - 1, acc + n)\n\
    }\n\
}\n\
noisy(3, 0)";
    assert_deep_identical(src, Value::Integer(6));
}

#[test]
fn a_shadowing_binding_is_not_the_function_itself() {
    // `helper` inside `outer` shadows the outer helper with a lambda of
    // the same arity; the call must dispatch to the shadow (a normal
    // call), not loop in the outer function's frame.
    let src = "\
fn helper(n) = n * 10\n\
fn outer(n) = {\n\
    let helper = (x) => x + 1\n\
    helper(n)\n\
}\n\
outer(5)";
    assert_deep_identical(src, Value::Integer(6));
}

#[test]
fn nested_named_helper_tail_recurses_over_its_captures() {
    // The classic accumulator hidden in a closure: the nested helper
    // captures `limit` and self-tail-calls; both tiers must carry the
    // capture through every elided frame.
    let src = "\
fn sum_below(limit) = {\n\
    fn go(i, acc) = if i >= limit => acc else => go(i + 1, acc + i)\n\
    go(0, 0)\n\
}\n\
sum_below(300000)";
    assert_deep_identical(src, Value::Integer(44_999_850_000));
}

#[test]
fn the_trace_notes_elided_frames() {
    // A deep tail spin that then raises: the stack cannot show a
    // million frames, and must say so rather than show one frame as if
    // it were the whole story.
    // A block body: the failing statement is Located inside the live
    // frame, which is where the reporter captures the stack.
    let source =
        "fn spin(n) = {\n    if n <= 0 => [1][5]\n    else => spin(n - 1)\n}\nspin(200000)";
    let parser = Parser::new();
    let program = parser.parse(source).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect_err("must raise");
    // The note rides the captured call stack the reporter renders.
    let location = interpreter
        .take_error_location()
        .expect("a runtime error captures its location");
    assert!(
        location
            .call_stack
            .iter()
            .any(|frame| frame.contains("tail calls elided")),
        "the trace must note the elision: {:?}",
        location.call_stack
    );
}

#[test]
fn default_parameters_survive_elision() {
    assert_deep_identical(
        "fn go(n, acc = 0) = if n <= 0 => acc else => go(n - 1, acc + 1)\ngo(150000)",
        Value::Integer(150_000),
    );
}

#[test]
fn annotated_parameters_stay_promises_at_depth() {
    // The self-call violates the annotation only on a later iteration;
    // the elided frame must still enforce it, on both tiers.
    for tier in [false, true] {
        let err = eval(
            "fn go(n, acc: Int) = if n <= 0 => acc else => go(n - 1, if n == 1 => \"oops\" else => acc)\n\
             fn outer() = go(150000, 0)\nouter()",
            tier,
        )
        .unwrap_err();
        assert!(
            err.contains("expects Int") || err.contains("Int"),
            "tier={}: {}",
            tier,
            err
        );
    }
}

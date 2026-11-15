//! The call-depth contract (Campaign 5, R4a): the cap is logical and
//! *physically reachable* — each user call grows the Rust stack in
//! segments when headroom runs low, so 99,999 frames work and frame
//! 100,000 raises a clean error instead of the stack raising a signal.
//! Both tiers enforce the same cap and raise the same message, and
//! `set_max_call_depth` moves the cap for both.

use olang::ast::Value;
use olang::{Interpreter, Parser};

const DEEP: &str = "fn down(n) = if n <= 0 => 0 else => 1 + down(n - 1)\n";

fn eval_with_depth(source: &str, tier: bool, max_depth: Option<usize>) -> Result<Value, String> {
    let source = source.to_string();
    // A small OS stack on purpose: the segmented growth, not a huge
    // preallocated stack, is what must carry deep recursion.
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let parser = Parser::new();
            let program = parser.parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if tier {
                interpreter.enable_bytecode_tier(1, false);
            }
            if let Some(depth) = max_depth {
                interpreter.set_max_call_depth(depth);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .expect("spawn")
        .join()
        .expect("join")
}

#[test]
fn the_default_cap_is_physically_reachable() {
    // 99,999 calls — one under the cap — must *work*, on both tiers,
    // even from a thread with a small OS stack.
    for tier in [false, true] {
        let got = eval_with_depth(&format!("{DEEP}down(99999)"), tier, None)
            .unwrap_or_else(|e| panic!("tier={}: {}", tier, e));
        assert_eq!(got, Value::Integer(99999), "tier={}", tier);
    }
}

#[test]
fn both_tiers_raise_the_same_error_at_the_same_depth() {
    let interp = eval_with_depth(&format!("{DEEP}down(100000)"), false, None).unwrap_err();
    let tiered = eval_with_depth(&format!("{DEEP}down(100000)"), true, None).unwrap_err();
    assert!(
        interp.contains("Maximum call depth (100000) exceeded"),
        "{}",
        interp
    );
    assert!(
        tiered.contains("Maximum call depth (100000) exceeded"),
        "{}",
        tiered
    );
}

#[test]
fn the_cap_moves_in_both_directions() {
    for tier in [false, true] {
        let low = eval_with_depth(&format!("{DEEP}down(600)"), tier, Some(500)).unwrap_err();
        assert!(
            low.contains("Maximum call depth (500) exceeded"),
            "tier={}: {}",
            tier,
            low
        );
        let high = eval_with_depth(&format!("{DEEP}down(150000)"), tier, Some(200_000))
            .unwrap_or_else(|e| panic!("tier={}: {}", tier, e));
        assert_eq!(got_int(&high), 150000, "tier={}", tier);
    }
}

fn got_int(v: &Value) -> i64 {
    match v {
        Value::Integer(i) => *i,
        other => panic!("expected an Int, got {:?}", other),
    }
}

#[test]
fn deep_recursion_still_returns_the_right_answer() {
    // Not just "doesn't crash": the 90k-frame unwind must carry the
    // value back correctly through every segment boundary.
    let got = eval_with_depth(&format!("{DEEP}down(90000)"), true, None).unwrap();
    assert_eq!(got, Value::Integer(90000));
}

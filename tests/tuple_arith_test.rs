//! Arithmetic on tuples, element by element (`ops::tuple_arith`): the
//! same answers from the interpreter and from a function promoted to the
//! bytecode tier, whose elements go through each tier's own numbers.
use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str, tier: bool) -> Result<String, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if tier {
                interpreter.enable_bytecode_tier(2, false);
            }
            interpreter
                .eval_program(program)
                .map(|v| match v {
                    Value::String(ref s) => s.to_string(),
                    other => format!("{other:?}"),
                })
                .map_err(|e| e.to_string())
        })
        .expect("spawn")
        .join()
        .expect("join")
}

/// `expr` inside a function called often enough to be promoted, shown.
fn both(expr: &str) -> (Result<String, String>, Result<String, String>) {
    let src = format!("fn f(k) = {expr}\nlet r = range(0, 20) |> map((k) => f(k))\nshow(r[19])");
    (eval(&src, false), eval(&src, true))
}

fn agree(expr: &str, want: &str) {
    let (a, b) = both(expr);
    assert_eq!(a.as_deref(), Ok(want), "interpreted: {expr}");
    assert_eq!(b.as_deref(), Ok(want), "promoted: {expr}");
}

fn both_refuse(expr: &str, says: &str) {
    let (a, b) = both(expr);
    for (tier, r) in [("interpreted", a), ("promoted", b)] {
        let e = r.expect_err(&format!("{tier}: {expr} should fail"));
        assert!(e.contains(says), "{tier}: {expr}: {e}");
    }
}

#[test]
fn element_by_element() {
    agree("(1, 2) + (3, 4)", "(4, 6)");
    agree("(10, 20) - (1, 2)", "(9, 18)");
    agree("(2, 3) * (4, 5)", "(8, 15)");
    agree("(7.0, 9.0) / (2.0, 3.0)", "(3.5, 3.0)");
    agree("(7, 9) % (4, 5)", "(3, 4)");
    agree("((1, 2), 3) + ((10, 20), 30)", "((11, 22), 33)");
}

#[test]
fn a_number_scales() {
    agree("(1.0, 2.0) * 0.5", "(0.5, 1.0)");
    agree("2 * (1, 2, 3)", "(2, 4, 6)");
    agree("(7.0, 9.0) / 2.0", "(3.5, 4.5)");
    agree("(7, 9) % 4", "(3, 1)");
}

#[test]
fn elements_keep_their_number_rules() {
    agree("(1, 2) + (0.5, 0.5)", "(1.5, 2.5)");
    both_refuse("(9223372036854775807, 1) + (1, 1)", "overflow");
}

#[test]
fn what_is_not_arithmetic_is_refused() {
    both_refuse("(1, 2) + (1, 2, 3)", "tuples of 2 and 3 elements");
    both_refuse("2.0 / (1.0, 2.0)", "cannot apply '/'");
    both_refuse("(1, 2) + 1", "cannot apply '+'");
}

#[test]
fn a_position_steps_as_written() {
    agree(
        "{ let p = (0.0, 0.0)\n let v = (1.0, 2.0)\n p + v * 0.5 }",
        "(0.5, 1.0)",
    );
}

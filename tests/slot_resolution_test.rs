//! Slot resolution must never change program results — only how fast
//! identifiers resolve. These cases target the places where the static
//! model can diverge from runtime frame layout, exercising the
//! verify-and-fall-back contract.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

#[test]
fn straight_line_locals_resolve() {
    let src = r#"
fn work(a, b) = {
    let c = a + b
    let d = c * 2
    d + a
}
work(3, 4)
"#;
    assert_eq!(eval(src), Value::Integer(17));
}

#[test]
fn conditional_let_shifts_later_slots() {
    // `let early` only pushes when the branch is taken, so `late`'s runtime
    // slot differs between calls — the name check at the slot must catch
    // the shifted case and fall back.
    let src = r#"
fn work(flag) = {
    if flag => {
        let early = 100
        early
    }
    let late = 5
    late * 10
}
work(true) + work(false)
"#;
    assert_eq!(eval(src), Value::Integer(100));
}

#[test]
fn let_reads_outer_before_binding() {
    // `let x = x + 1`: the value expression must see the parameter, not the
    // slot being bound
    let src = r#"
fn bump(x) = {
    let x = x + 1
    x
}
bump(41)
"#;
    assert_eq!(eval(src), Value::Integer(42));
}

#[test]
fn loop_variable_shadows_function_local() {
    let src = r#"
fn work() = {
    let mut i = 100
    let mut total = 0
    for i in 0..5 {
        total = total + i
    }
    total + i
}
work()
"#;
    // 0+1+2+3+4 = 10, plus the untouched outer i
    assert_eq!(eval(src), Value::Integer(110));
}

#[test]
fn match_binding_shadows_parameter() {
    let src = r#"
fn work(n) = match n * 2 {
    n => n + 1
}
work(10)
"#;
    assert_eq!(eval(src), Value::Integer(21));
}

#[test]
fn reference_through_nested_frames() {
    // From inside a match arm inside a for loop, references reach the
    // function frame two hops out
    let src = r#"
fn work(base) = {
    let mut total = 0
    for i in 0..3 {
        total = total + match i % 2 {
            0 => base,
            _ => i
        }
    }
    total
}
work(100)
"#;
    // i=0: base(100), i=1: 1, i=2: base(100)
    assert_eq!(eval(src), Value::Integer(201));
}

#[test]
fn rebinding_in_loop_keeps_slot() {
    let src = r#"
fn work(n) = {
    let mut total = 0
    let mut i = 0
    while i < n {
        let squared = i * i
        total = total + squared
        i = i + 1
    }
    total
}
work(5)
"#;
    assert_eq!(eval(src), Value::Integer(30));
}

#[test]
fn destructuring_lets_stay_correct() {
    let src = r#"
fn work() = {
    let [a, b, c] = [1, 2, 3]
    let d = a + b
    d + c
}
work()
"#;
    assert_eq!(eval(src), Value::Integer(6));
}

#[test]
fn lambda_inside_resolved_body() {
    // The lambda body resolves against its own frame; its captures resolve
    // by name from the closure
    let src = r#"
fn work(k) = {
    let items = [1, 2, 3]
    sum(map(items, (x) => x * 2)) + k
}
work(10)
"#;
    assert_eq!(eval(src), Value::Integer(22));
}

#[test]
fn nested_function_declaration_resolves_independently() {
    let src = r#"
fn outer(a) = {
    fn inner(b) = b * 10
    inner(a) + a
}
outer(5)
"#;
    assert_eq!(eval(src), Value::Integer(55));
}

#[test]
fn recursion_through_resolved_self_slot() {
    let src = r#"
fn fact(n) = if n <= 1 => 1 else => n * fact(n - 1)
fact(6)
"#;
    assert_eq!(eval(src), Value::Integer(720));
}

#[test]
fn catch_variable_resolves_in_catch_frame() {
    let src = r#"
fn work(x) = {
    let fallback = 7
    try { Err("boom") } catch (e) { fallback + x }
}
work(1)
"#;
    assert_eq!(eval(src), Value::Integer(8));
}

//! Syntax and operator extensions surfaced by dogfooding a template engine:
//! list concatenation with `+`, `else if` chains, and multi-line `use` import
//! lists with a trailing comma.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

fn ints(v: Value) -> Vec<i64> {
    match v {
        Value::List(items) => items
            .iter()
            .map(|x| match x {
                Value::Integer(n) => *n,
                other => panic!("expected int, got {:?}", other),
            })
            .collect(),
        other => panic!("expected list, got {:?}", other),
    }
}

fn s(v: Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn list_plus_concatenates() {
    assert_eq!(ints(eval("[1, 2] + [3, 4]")), vec![1, 2, 3, 4]);
}

#[test]
fn list_plus_with_empty_operands() {
    assert_eq!(ints(eval("[] + [1]")), vec![1]);
    assert_eq!(ints(eval("[1] + []")), vec![1]);
    assert_eq!(ints(eval("[] + []")), Vec::<i64>::new());
}

#[test]
fn list_plus_builds_up_in_a_loop() {
    // The pattern the engine's lexer relies on: append one element per step.
    let src = r#"
let mut acc = []
let mut i = 0
while i < 4 {
    acc = acc + [i * i]
    i = i + 1
}
acc
"#;
    assert_eq!(ints(eval(src)), vec![0, 1, 4, 9]);
}

#[test]
fn else_if_chain_selects_the_right_arm() {
    let src = r#"
fn classify(n) = if n > 0 => "pos"
    else if n < 0 => "neg"
    else => "zero"
classify(-2) + "/" + classify(0) + "/" + classify(5)
"#;
    assert_eq!(s(eval(src)), "neg/zero/pos");
}

#[test]
fn else_if_chain_of_several_arms() {
    let src = r#"
fn grade(x) = if x >= 90 => "A"
    else if x >= 80 => "B"
    else if x >= 70 => "C"
    else => "F"
grade(95) + grade(85) + grade(72) + grade(40)
"#;
    assert_eq!(s(eval(src)), "ABCF");
}

#[test]
fn plain_if_else_still_parses_before_a_declaration() {
    // Guards the grammar change: an `if/else` function body followed by
    // another declaration must still parse.
    let src = r#"
fn a(x) = if x => "y" else => "n"
fn b(x) = a(x)
b(true)
"#;
    assert_eq!(s(eval(src)), "y");
}

#[test]
fn multiline_use_list_with_trailing_comma_parses() {
    // Parsing only — the module need not resolve; we just assert the import
    // list spanning lines with a trailing comma is accepted by the grammar.
    let src = "use lib.things {\n    alpha,\n    beta,\n    gamma,\n}\n";
    assert!(Parser::new().parse(src).is_ok());
}

#[test]
fn single_line_use_list_still_parses() {
    let src = "use lib.things { alpha, beta }\n";
    assert!(Parser::new().parse(src).is_ok());
}

#[test]
fn let_with_a_forgotten_equals_is_a_parse_error() {
    // `let scores #{...}` used to silently parse as an uninitialized `let`
    // plus a stray expression statement, leaving `scores` bound to Unit.
    // A let must either have an initializer or end the statement.
    assert!(
        Parser::new()
            .parse("let scores #{ \"ada\": 99 }\n")
            .is_err()
    );
    assert!(Parser::new().parse("let x [1, 2, 3]\n").is_err());
    assert!(Parser::new().parse("let y \"oops\"\n").is_err());
}

#[test]
fn uninitialized_let_still_parses_and_is_unit() {
    assert!(Parser::new().parse("let pending\n").is_ok());
    assert!(
        Parser::new()
            .parse("let pending // fill in later\n")
            .is_ok()
    );
    assert!(Parser::new().parse("let typed: Int\n").is_ok());
    // Unbound-until-assigned semantics are unchanged.
    let src = "let pending\ntypeof(pending)";
    match eval(src) {
        Value::String(s) => assert_eq!(s.to_string(), "Unit"),
        other => panic!("expected type name, got {:?}", other),
    }
}

#[test]
fn assertions_accept_multiline_arguments() {
    // A multi-line assert_eq used to fall out of the assertion grammar and
    // parse as a call to an undefined `assert_eq` function.
    let src = "test \"multiline\" {\n    assert_eq(\n        1 + 1,\n        2\n    )\n    assert_eq(\"a\" + \"b\",\n        \"ab\", \"concat\")\n    assert(\n        true\n    )\n}\n\"done\"";
    match eval(src) {
        Value::String(s) => assert_eq!(s.to_string(), "done"),
        other => panic!("expected done, got {:?}", other),
    }
}

#[test]
fn multiline_assertion_failures_still_fail() {
    let src = "test \"fails\" {\n    assert_eq(\n        1,\n        2\n    )\n}";
    let program = Parser::new().parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    assert!(interpreter.eval_program(program).is_err());
}

#[test]
fn let_mut_is_one_statement_with_no_stray_binding() {
    // `let mut x = 1` must not leave a binding named `mut` behind.
    let src = "let mut x = 1\nx = x + 1\nx";
    assert_eq!(eval(src), Value::Integer(2));
    let parser = Parser::new();
    let program = parser.parse("let mut x = 1\ntypeof(mut)").expect("parse");
    let mut interpreter = Interpreter::new();
    assert!(
        interpreter.eval_program(program).is_err(),
        "`mut` must not be bound by `let mut x`"
    );
}

#[test]
fn fat_arrow_may_start_a_new_line() {
    // Dogfooding viz: a long `if` condition reads better with the `=>`
    // wrapped to the next line. The grammar now tolerates NL before it.
    assert_eq!(
        s(eval(
            r#"
let x = 5
if x > 0
    => "positive"
    else => "non-positive"
"#
        )),
        "positive"
    );
    // Long multi-term condition, arrow on its own line.
    assert_eq!(
        s(eval(
            r#"
let a = 3
let b = 4
if a > 0 && b > 0 && a + b > 5
    => "both, big"
    else => "no"
"#
        )),
        "both, big"
    );
    // The original same-line form is unchanged.
    assert_eq!(s(eval(r#"if 1 > 0 => "a" else => "b""#)), "a");
    // `else if` chains still parse with a wrapped arrow.
    assert_eq!(
        s(eval(
            r#"
let n = 2
if n == 1
    => "one"
    else => if n == 2
        => "two"
        else => "many"
"#
        )),
        "two"
    );
}

// ── keyword-prefixed identifiers ────────────────────────────────────
// Every reserved and contextual keyword used to be able to swallow the
// head of an identifier: `breaker` lexed as `break` + `er`, `returns =
// 5` as `return s`, and `useful = 1` died in `use_decl`. Keywords now
// consume only at a word boundary.

#[test]
fn identifiers_may_start_with_keywords() {
    let v = eval(
        "let mut useful = 1\n\
         useful = useful + 1\n\
         let mut typed = 2\n\
         typed = typed + 1\n\
         let mut shared = 3\n\
         shared = shared + 1\n\
         let mut formal = 4\n\
         formal = formal + 1\n\
         let mut input = 5\n\
         input = input + 1\n\
         let mut matcher = 6\n\
         matcher = matcher + 1\n\
         let mut iffy = 7\n\
         iffy = iffy + 1\n\
         let mut looped = 8\n\
         looped = looped + 1\n\
         [useful, typed, shared, formal, input, matcher, iffy, looped]",
    );
    assert_eq!(ints(v), vec![2, 3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn functions_may_be_named_with_keyword_prefixes() {
    let v = eval(
        "fn breaker(n) = {\n\
             let mut i = 0\n\
             while true { if i >= n => { break } i = i + 1 }\n\
             i\n\
         }\n\
         fn returns(x) = x + 1\n\
         fn continued(x) = x * 2\n\
         [breaker(7), returns(9), continued(3)]",
    );
    assert_eq!(ints(v), vec![7, 10, 6]);
}

#[test]
fn keywords_still_work_with_space_separated_operands() {
    // The boundary guard must not require the operand to touch the
    // keyword: `return x`, `break 5`, `if b => ...` all keep a space.
    let v = eval(
        "fn f(n) = {\n\
             let mut i = 0\n\
             let mut got = 0\n\
             while true {\n\
                 if i >= n => { got = loop { break i + 100 } break }\n\
                 i = i + 1\n\
             }\n\
             return got\n\
         }\n\
         [f(3)]",
    );
    assert_eq!(ints(v), vec![103]);
}

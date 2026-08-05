//! Coverage for the `col` (collections) module — the first higher-order
//! stdlib module. Verifies each operation and that key functions/predicates
//! are actually invoked through the interpreter.

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

#[test]
fn min_max_by_use_the_key_function() {
    assert_eq!(
        eval(r#"col.min_by([{n: 3}, {n: 1}, {n: 2}], (x) => x.n).n"#),
        Value::Integer(1)
    );
    assert_eq!(
        eval(r#"col.max_by([{n: 3}, {n: 1}, {n: 2}], (x) => x.n).n"#),
        Value::Integer(3)
    );
}

#[test]
fn sort_by_orders_by_key() {
    assert_eq!(
        ints(eval(r#"col.sort_by([5, 2, 8, 1], (x) => x)"#)),
        vec![1, 2, 5, 8]
    );
    // descending via negated key
    assert_eq!(
        ints(eval(r#"col.sort_by([5, 2, 8, 1], (x) => 0 - x)"#)),
        vec![8, 5, 2, 1]
    );
}

#[test]
fn count_by_and_frequencies_have_clean_keys() {
    // string keys must not be double-quoted
    assert_eq!(
        eval(r#"map_get(col.count_by([{t: "a"}, {t: "b"}, {t: "a"}], (x) => x.t), "a")"#),
        Value::Integer(2)
    );
    assert_eq!(
        eval(r#"map_get(col.frequencies(["x", "y", "x", "x"]), "x")"#),
        Value::Integer(3)
    );
}

#[test]
fn partition_splits_by_predicate() {
    let src = r#"
let parts = col.partition([1, 2, 3, 4, 5], (x) => x % 2 == 0)
let evens = parts[0]
let odds = parts[1]
[len(evens), len(odds)]
"#;
    assert_eq!(ints(eval(src)), vec![2, 3]);
}

#[test]
fn flat_map_flattens_one_level() {
    assert_eq!(
        ints(eval(r#"col.flat_map([1, 2, 3], (x) => [x, x * 10])"#)),
        vec![1, 10, 2, 20, 3, 30]
    );
}

#[test]
fn take_and_drop_while() {
    assert_eq!(
        ints(eval(r#"col.take_while([1, 2, 3, 9, 1], (x) => x < 4)"#)),
        vec![1, 2, 3]
    );
    assert_eq!(
        ints(eval(r#"col.drop_while([1, 2, 3, 9, 1], (x) => x < 4)"#)),
        vec![9, 1]
    );
}

#[test]
fn all_any_short_circuit() {
    assert_eq!(
        eval(r#"col.all([2, 4, 6], (x) => x % 2 == 0)"#),
        Value::Boolean(true)
    );
    assert_eq!(
        eval(r#"col.all([2, 3, 6], (x) => x % 2 == 0)"#),
        Value::Boolean(false)
    );
    assert_eq!(
        eval(r#"col.any([1, 3, 4], (x) => x % 2 == 0)"#),
        Value::Boolean(true)
    );
    assert_eq!(
        eval(r#"col.any([1, 3, 5], (x) => x % 2 == 0)"#),
        Value::Boolean(false)
    );
    // vacuous truth
    assert_eq!(eval(r#"col.all([], (x) => false)"#), Value::Boolean(true));
    assert_eq!(eval(r#"col.any([], (x) => true)"#), Value::Boolean(false));
}

#[test]
fn sum_by_totals_a_projection() {
    assert_eq!(
        eval(r#"col.sum_by([{n: 10}, {n: 20}, {n: 5}], (x) => x.n)"#),
        Value::Integer(35)
    );
}

#[test]
fn unique_preserves_first_seen_order() {
    assert_eq!(
        ints(eval(r#"col.unique([3, 1, 3, 2, 1, 3])"#)),
        vec![3, 1, 2]
    );
}

#[test]
fn window_produces_sliding_slices() {
    let src = r#"
let ws = col.window([1, 2, 3, 4], 2)
[len(ws), ws[0][0], ws[0][1], ws[2][1]]
"#;
    assert_eq!(ints(eval(src)), vec![3, 1, 2, 4]);
    // window larger than the list yields nothing
    assert_eq!(eval(r#"len(col.window([1, 2], 5))"#), Value::Integer(0));
}

#[test]
fn zip_with_combines_to_shortest() {
    assert_eq!(
        ints(eval(
            r#"col.zip_with([1, 2, 3], [10, 20], (a, b) => a + b)"#
        )),
        vec![11, 22]
    );
}

#[test]
fn last_returns_final_element() {
    assert_eq!(eval(r#"col.last([7, 8, 9])"#), Value::Integer(9));
}

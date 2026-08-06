//! The embedded olang-source stdlib (`colx`) is compiled into the binary and
//! usable via `use colx` with no filesystem package. These tests verify it
//! loads, and — differentially — that every olang-implemented function agrees
//! with the native Rust `col` builtin it mirrors.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

#[test]
fn embedded_module_loads_without_a_filesystem_package() {
    // `use colx` resolves to source embedded in the binary.
    let src = "use colx { unique }\nunique([1, 1, 2, 3, 2])";
    assert_eq!(
        eval(src),
        Value::List(vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)].into())
    );
}

/// Run an expression against both the olang `colx` function and the Rust
/// `col` builtin and assert they produce the same value.
fn assert_agrees(colx_call: &str, col_call: &str) {
    let olang_side = eval(&format!("use colx {{ * }}\n{}", colx_call));
    let rust_side = eval(col_call);
    assert_eq!(
        olang_side, rust_side,
        "olang `{}` != rust `{}`",
        colx_call, col_call
    );
}

#[test]
fn unique_agrees() {
    assert_agrees(
        "unique([3, 1, 3, 2, 1, 2])",
        "col.unique([3, 1, 3, 2, 1, 2])",
    );
    assert_agrees("unique([])", "col.unique([])");
}

#[test]
fn sum_by_agrees() {
    assert_agrees(
        "sum_by([1, 2, 3, 4], (x) => x * x)",
        "col.sum_by([1, 2, 3, 4], (x) => x * x)",
    );
}

#[test]
fn all_and_any_agree() {
    assert_agrees(
        "all([2, 4, 6], (x) => x % 2 == 0)",
        "col.all([2, 4, 6], (x) => x % 2 == 0)",
    );
    assert_agrees(
        "all([2, 3, 6], (x) => x % 2 == 0)",
        "col.all([2, 3, 6], (x) => x % 2 == 0)",
    );
    assert_agrees(
        "any([1, 3, 4], (x) => x % 2 == 0)",
        "col.any([1, 3, 4], (x) => x % 2 == 0)",
    );
    assert_agrees(
        "any([1, 3, 5], (x) => x % 2 == 0)",
        "col.any([1, 3, 5], (x) => x % 2 == 0)",
    );
}

#[test]
fn take_while_agrees() {
    // The case the differential test caught: state must be threaded purely.
    assert_agrees(
        "take_while([1, 2, 3, 10, 4], (x) => x < 5)",
        "col.take_while([1, 2, 3, 10, 4], (x) => x < 5)",
    );
    assert_agrees(
        "take_while([9, 1, 2], (x) => x < 5)",
        "col.take_while([9, 1, 2], (x) => x < 5)",
    );
}

#[test]
fn partition_agrees() {
    assert_agrees(
        "partition([1, 2, 3, 4, 5], (x) => x % 2 == 0)",
        "col.partition([1, 2, 3, 4, 5], (x) => x % 2 == 0)",
    );
}

#[test]
fn count_by_agrees() {
    // Compare the count for a specific key (maps compare structurally, but
    // extracting a key avoids any ordering concerns).
    assert_agrees(
        "map_get(count_by([\"a\", \"b\", \"a\", \"a\"], (x) => x), \"a\")",
        "map_get(col.count_by([\"a\", \"b\", \"a\", \"a\"], (x) => x), \"a\")",
    );
}

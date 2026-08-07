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

#[test]
fn drop_while_agrees() {
    assert_agrees(
        "drop_while([1, 2, 3, 10, 4], (x) => x < 5)",
        "col.drop_while([1, 2, 3, 10, 4], (x) => x < 5)",
    );
    assert_agrees(
        "drop_while([9, 1, 2], (x) => x < 5)",
        "col.drop_while([9, 1, 2], (x) => x < 5)",
    );
}

#[test]
fn flat_map_agrees() {
    assert_agrees(
        "flat_map([1, 2, 3], (x) => [x, x * 10])",
        "col.flat_map([1, 2, 3], (x) => [x, x * 10])",
    );
    // non-list results are kept as single elements
    assert_agrees(
        "flat_map([1, 2], (x) => x + 1)",
        "col.flat_map([1, 2], (x) => x + 1)",
    );
}

#[test]
fn frequencies_agrees() {
    assert_agrees(
        "map_get(frequencies([1, 2, 2, 3, 2]), \"2\")",
        "map_get(col.frequencies([1, 2, 2, 3, 2]), \"2\")",
    );
}

#[test]
fn last_agrees() {
    assert_agrees("last([5, 6, 7])", "col.last([5, 6, 7])");
    assert_agrees("last([42])", "col.last([42])");
}

#[test]
fn min_max_by_agree_with_a_key_function() {
    // Key that inverts order, over records — exercises real key functions.
    assert_agrees(
        "min_by([{v: 3}, {v: 1}, {v: 2}], (r) => r.v).v",
        "col.min_by([{v: 3}, {v: 1}, {v: 2}], (r) => r.v).v",
    );
    assert_agrees(
        "max_by([{v: 3}, {v: 1}, {v: 2}], (r) => r.v).v",
        "col.max_by([{v: 3}, {v: 1}, {v: 2}], (r) => r.v).v",
    );
    // string keys
    assert_agrees(
        "min_by([\"bb\", \"a\", \"ccc\"], (s) => len(s))",
        "col.min_by([\"bb\", \"a\", \"ccc\"], (s) => len(s))",
    );
}

#[test]
fn sort_by_agrees_and_is_stable() {
    assert_agrees(
        "sort_by([3, 1, 4, 1, 5, 9, 2, 6], (x) => x)",
        "col.sort_by([3, 1, 4, 1, 5, 9, 2, 6], (x) => x)",
    );
    // descending by a negated key
    assert_agrees(
        "sort_by([1, 2, 3], (x) => 0 - x)",
        "col.sort_by([1, 2, 3], (x) => 0 - x)",
    );
    // stability: equal keys keep input order (first field is the tiebreak we
    // observe; both implementations must agree)
    assert_agrees(
        "sort_by([{k: 1, id: \"a\"}, {k: 1, id: \"b\"}, {k: 0, id: \"c\"}], (r) => r.k)",
        "col.sort_by([{k: 1, id: \"a\"}, {k: 1, id: \"b\"}, {k: 0, id: \"c\"}], (r) => r.k)",
    );
}

#[test]
fn window_agrees() {
    assert_agrees(
        "window([1, 2, 3, 4, 5], 3)",
        "col.window([1, 2, 3, 4, 5], 3)",
    );
    assert_agrees("window([1, 2], 3)", "col.window([1, 2], 3)"); // size > len -> []
    assert_agrees("window([1, 2, 3], 1)", "col.window([1, 2, 3], 1)");
}

#[test]
fn zip_with_agrees() {
    assert_agrees(
        "zip_with([1, 2, 3], [10, 20, 30], (x, y) => x + y)",
        "col.zip_with([1, 2, 3], [10, 20, 30], (x, y) => x + y)",
    );
    // truncates to the shorter list
    assert_agrees(
        "zip_with([1, 2, 3, 4], [10, 20], (x, y) => x * y)",
        "col.zip_with([1, 2, 3, 4], [10, 20], (x, y) => x * y)",
    );
}

#[test]
fn colx_mirrors_every_col_function() {
    // Every function the olang `colx` module exports must be callable, so the
    // embedded mirror stays complete. We check the known set is present by
    // importing all and referencing each (a missing export fails to resolve).
    let names = [
        "all",
        "any",
        "count_by",
        "drop_while",
        "flat_map",
        "frequencies",
        "last",
        "max_by",
        "min_by",
        "partition",
        "sort_by",
        "sum_by",
        "take_while",
        "unique",
        "window",
        "zip_with",
    ];
    for name in names {
        // Referencing the imported name yields a function value; an absent
        // export would be an "undefined variable" error.
        let src = format!("use colx {{ * }}\ntypeof({})", name);
        let result = eval(&src);
        assert_eq!(
            result,
            Value::String("Function".to_string().into()),
            "colx is missing `{}`",
            name
        );
    }
}

#[test]
fn use_binds_the_module_as_a_namespace() {
    // After `use colx { ... }`, the module name itself is bound — inspectable
    // and callable as `colx.fn(...)` — matching the native stdlib modules.
    // The selective import still binds the named items too.
    let src = r#"
use colx { unique }
[typeof(colx), colx.sort_by([3, 1, 2], (x) => x), unique([1, 1, 2])]
"#;
    match eval(src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::String("Module".to_string().into()));
            assert_eq!(
                items[1],
                Value::List(vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)].into())
            );
            assert_eq!(
                items[2],
                Value::List(vec![Value::Integer(1), Value::Integer(2)].into())
            );
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn module_members_lists_functions_of_a_bound_module() {
    // Backs `:help <module>` — after use, the module's members are queryable.
    let program = Parser::new().parse("use colx { unique }").unwrap();
    let mut interp = Interpreter::new();
    interp.eval_program(program).unwrap();
    let members = interp
        .module_members("colx")
        .expect("colx is a bound module");
    assert!(members.contains(&"unique".to_string()));
    assert!(members.contains(&"sort_by".to_string()));
    assert_eq!(members.len(), 16);
    // A non-module name yields None.
    assert!(interp.module_members("nope").is_none());
}

//! The 1.0-freeze number and order rulings: `min`/`max` raise on values
//! `<` cannot order (OL-338, OL-391), `col.unique` dedupes by `==`
//! (OL-339), the aggregates raise a Float overflow instead of answering
//! inf (OL-355), an Int and a Float order exactly, as `==` compares them
//! (OL-359), and lists order element by element, as tuples do (OL-487).
//!
//! Every case runs three ways and must agree: the interpreter (the
//! reference), the bytecode VM (each function compiled directly), and
//! the tiered interpreter at threshold 1 (bytecode promoted at its first
//! call and the JIT taking what it can).

use olang::ast::{Statement, Value};
use olang::ovm::FunctionId;
use olang::ovm::OvmValue;
use olang::ovm::bytecode::BytecodeVm;
use olang::{Interpreter, Parser};
use std::sync::Arc;

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn eval(source: &str, tier: Option<u32>) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        if let Some(threshold) = tier {
            interpreter.enable_bytecode_tier(threshold, false);
        }
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

/// `main()` compiled on the bytecode VM with every function of the
/// program; refusing to compile is a failure of the test.
fn on_vm(source: &str) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
        let mut vm = BytecodeVm::new();
        let mut ids = Vec::new();
        let mut main = None;
        for statement in &program.statements {
            if let Statement::FunctionDecl(func) = statement.unwrapped() {
                let id = FunctionId::new();
                vm.register_function(func.name.clone(), id);
                if func.name == "main" {
                    main = Some(id);
                }
                ids.push((id, func.clone()));
            }
        }
        for (id, func) in &ids {
            if let Err(e) = vm.compile_function(*id, func) {
                panic!("`{}` refused by the bytecode tier: {}", func.name, e);
            }
        }
        let main = main.expect("a main function");
        vm.execute(main, &[] as &[OvmValue])
            .map_err(|e| e.to_string())
            .and_then(|v| v.into_ast().map_err(|e| format!("{:?}", e)))
    })
}

/// The interpreter's answer, which the tiered run gives too (message for
/// message) and the VM gives too (an error as an error). A program that
/// calls a `col.` function needs the interpreter's modules to compile, so
/// it is checked tiered only.
fn agree(source: &str) -> Result<Value, String> {
    let interpreted = eval(source, None);
    let tiered = eval(source, Some(1));
    assert_eq!(
        tiered, interpreted,
        "tiered diverged from the interpreter\n{source}"
    );
    if source.contains("col.") {
        return interpreted;
    }
    let vm = on_vm(source);
    match (&interpreted, &vm) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "the VM diverged\n{source}"),
        (Err(_), Err(_)) => {}
        _ => panic!("the VM diverged: {interpreted:?} vs {vm:?}\n{source}"),
    }
    interpreted
}

fn list(items: Vec<Value>) -> Value {
    Value::List(Arc::new(items))
}

fn bools(bs: &[bool]) -> Value {
    list(bs.iter().map(|b| Value::Boolean(*b)).collect())
}

fn ints(ns: &[i64]) -> Value {
    list(ns.iter().map(|n| Value::Integer(*n)).collect())
}

// ── OL-338, OL-391: min and max ─────────────────────────────────────
#[test]
fn min_and_max_raise_on_values_they_cannot_order() {
    for call in [
        "min([3, \"a\"])",
        "max([3, \"a\"])",
        "min([\"a\", 3])",
        "max([\"a\", 3])",
        "min([[1], 2])",
        "min(3, \"a\")",
        "min(\"a\", 1)",
        "max(\"a\", 1)",
        "col.min([1, \"a\"])",
    ] {
        let src = format!("fn main() = {call}\nmain()");
        let err = agree(&src).expect_err(call);
        assert!(err.contains("cannot order"), "{call}: {err}");
        assert!(
            err.contains("min:") || err.contains("max:"),
            "{call}: {err}"
        );
    }
}

#[test]
fn min_and_max_still_order_what_they_can() {
    let src = r#"
fn main() = [
    min([3, 1.5, 2]) == 1.5,
    max([3, 1.5, 2]) == 3,
    min("b", "a") == "a",
    max(["a", "c", "b"]) == "c",
    min([(2, "a"), (1, "z")]) == (1, "z"),
    max([[1, 2], [1, 3], [0, 9]]) == [1, 3],
    min([true, false]) == false,
    min([1, 1.0]) == 1,
]
main()
"#;
    assert_eq!(agree(src), Ok(bools(&[true; 8])));
}

// ── OL-339: col.unique by `==` ──────────────────────────────────────
#[test]
fn col_unique_dedupes_by_equality() {
    let src = r#"
fn main() = [
    col.unique([1, 1.0, 2]) == [1, 2],
    len(col.unique([1, "1"])) == 2,
    len(col.unique([[1], [1.0], [1], [2]])) == 3,
    len(col.unique([#{ "a": 1 }, #{ "a": 1 }, #{ "a": "1" }])) == 2,
    col.unique([1.5, 1.5, 2]) == [1.5, 2],
]
main()
"#;
    assert_eq!(agree(src), Ok(bools(&[true; 5])));
}

// ── OL-355: aggregates overflow as `+` does ─────────────────────────
#[test]
fn float_aggregates_raise_on_overflow() {
    for (call, name) in [
        ("sum([1e308, 1e308])", "sum:"),
        ("average([1e308, 1e308])", "average:"),
        ("col.sum_by([1e308, 1e308], (x) => x)", "col.sum_by:"),
        ("sum([1, 1e308, 1e308])", "sum:"),
    ] {
        let src = format!("fn main() = {call}\nmain()");
        let err = agree(&src).expect_err(call);
        assert!(
            err.contains(name) && err.contains("Float overflow in addition"),
            "{call}: {err}"
        );
        assert!(!err.contains("inf"), "{call} leaked inf: {err}");
    }
}

#[test]
fn average_of_large_ints_is_their_mean() {
    let src = "fn main() = average([9223372036854775807, 9223372036854775807])\nmain()";
    assert_eq!(agree(src), Ok(Value::Float(9223372036854775807.0)));
    let src = "fn main() = average([1, \"a\"])\nmain()";
    let err = agree(src).expect_err("not numbers");
    assert!(err.contains("average:"), "{err}");
}

// ── OL-359: Int and Float order exactly ─────────────────────────────
#[test]
fn int_float_ordering_agrees_with_equality_past_2_53() {
    // Through function arguments (registers on the VM and the JIT) and
    // through literals (immediates).
    let src = r#"
fn rel(a, b) = [a == b, a <= b, b < a, a > b, a < b, b >= a]
fn main() = rel(9007199254740993, 9007199254740992.0)
main()
"#;
    assert_eq!(
        agree(src),
        Ok(bools(&[false, false, true, true, false, false]))
    );
    let src = r#"
fn main() = [
    9223372036854775807 < 9223372036854775807.0,
    9223372036854775807.0 > 9223372036854775807,
    9007199254740993 > 9007199254740992.0,
    9007199254740992.0 < 9007199254740993,
    -9223372036854775807 - 1 <= -9223372036854775808.0,
    2 < 2.5,
    3 > 2.5,
]
main()
"#;
    assert_eq!(agree(src), Ok(bools(&[true; 7])));
}

#[test]
fn a_hot_mixed_comparison_stays_exact_under_the_jit() {
    // lt() is called enough to be compiled before the deciding calls.
    let src = r#"
fn lt(a, b) = a < b
fn ge_big(x) = x >= 9007199254740993
fn main() = {
    let mut n = 0
    for i in 0..2000 {
        n = n + (if lt(i, 1000.5) => 1 else => 0)
    }
    [n == 1001, lt(9007199254740992.0, 9007199254740993), lt(9007199254740993, 9007199254740992.0), ge_big(9007199254740992.0)]
}
main()
"#;
    assert_eq!(agree(src), Ok(bools(&[true, true, false, false])));
}

#[test]
fn sort_orders_mixed_numbers_exactly() {
    let src = "fn main() = sort([9007199254740993, 9007199254740992.0, 1])\nmain()";
    assert_eq!(
        agree(src),
        Ok(list(vec![
            Value::Integer(1),
            Value::Float(9007199254740992.0),
            Value::Integer(9007199254740993),
        ]))
    );
}

// ── OL-487: lists order lexicographically ───────────────────────────
#[test]
fn lists_order_element_by_element() {
    let src = r#"
fn lt(a, b) = a < b
fn main() = [
    [1, 2] < [1, 3],
    [1, 3] > [1, 2],
    [1] < [1, 0],
    [] < [0],
    [1, 2] <= [1, 2],
    [1, 2.5] < [1, 3],
    ["a", "b"] < ["a", "c"],
    [[1, 2], [3]] < [[1, 2], [4]],
    lt([2.5], [3]),
    lt([1, 2], [1, 3]),
    ([2] < [1, 9]) == false,
]
main()
"#;
    assert_eq!(agree(src), Ok(bools(&[true; 11])));
}

#[test]
fn sort_and_sort_by_order_lists() {
    let src = "fn main() = sort([[2, 1], [1, 9], [1, 2], [1]])\nmain()";
    assert_eq!(
        agree(src),
        Ok(list(vec![
            ints(&[1]),
            ints(&[1, 2]),
            ints(&[1, 9]),
            ints(&[2, 1]),
        ]))
    );
    let src = "fn main() = col.sort_by([[10, 2], [9, 3]], (r) => r) |> map((r) => r[1])\nmain()";
    assert_eq!(agree(src), Ok(ints(&[3, 2])));
}

#[test]
fn an_unequal_list_pair_without_an_order_raises() {
    let src = "fn main() = [1, \"a\"] < [\"b\", 2]\nmain()";
    let err = agree(src).expect_err("no order");
    assert!(
        err.contains("cannot order Int and String inside a list"),
        "{err}"
    );
    let src = "fn main() = sort([[1, \"a\"], [1, 2]])\nmain()";
    let err = agree(src).expect_err("no order");
    assert!(
        err.contains("sort: cannot order ") && err.contains(" inside a list"),
        "{err}"
    );
    // A list and a tuple are different kinds: no order between them.
    let src = "fn main() = [1, 2] < (1, 2)\nmain()";
    agree(src).expect_err("a list and a tuple");
}

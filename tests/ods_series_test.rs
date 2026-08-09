//! Phase 1 integration tests: Series behaves identically from both tiers
//! and agrees with olang-list reference computations (docs/design/ods.md).
//!
//! Promoted functions here use float constants deliberately: the baseline
//! JIT's pure-integer whitelist declines them, so these tests exercise the
//! bytecode tier's module hook regardless of JIT progress.

#![cfg(feature = "ods")]

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

fn eval(source: &str, tier_threshold: Option<u32>) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        if let Some(threshold) = tier_threshold {
            interpreter.enable_bytecode_tier(threshold, false);
        }
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

fn assert_tier_transparent(source: &str) -> Result<Value, String> {
    let interpreted = eval(source, None);
    let promoted = eval(source, Some(2));
    match (&interpreted, &promoted) {
        (Ok(a), Ok(b)) => assert_eq!(
            a, b,
            "promotion changed the result\n  source: {}",
            source
        ),
        (Err(_), Err(_)) => {}
        _ => panic!(
            "promotion changed success/failure\n  interpreted: {:?}\n  promoted: {:?}\n  source: {}",
            interpreted, promoted, source
        ),
    }
    interpreted
}

#[test]
fn series_arithmetic_is_tier_transparent() {
    // scale promotes (arithmetic + float constants); the series crosses
    // into the VM and its Mul/Add instructions must hit the module hook.
    let result = assert_tier_transparent(
        r#"
        fn scale(s) = s * 2.0 + 1.0
        let xs = ods.series([1.0, 2.0, 3.0])
        let a = scale(xs)
        let b = scale(xs)
        let c = scale(xs)
        ods.to_list(c)
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![Value::Float(3.0), Value::Float(5.0), Value::Float(7.0)].into()
        ))
    );
}

#[test]
fn series_reductions_match_list_references() {
    // The same numbers through list builtins and through Series kernels.
    let result = assert_tier_transparent(
        r#"
        let raw = [4.0, 1.0, 3.0, 2.0]
        let s = ods.series(raw)
        let list_mean = sum(raw) / len(raw)
        [ods.sum(s) == sum(raw), ods.mean(s) == list_mean, ods.min(s), ods.max(s)]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Float(1.0),
                Value::Float(4.0),
            ]
            .into()
        ))
    );
}

#[test]
fn nulls_propagate_and_reductions_skip_them() {
    // There is no unit literal in the grammar; a missing map key is the
    // canonical runtime source of Unit — and the realistic way a null
    // enters a dataset.
    let result = assert_tier_transparent(
        r#"
        let missing = map_get(#{}, "absent")
        let s = ods.series([1.0, missing, 3.0])
        let doubled = s * 2.0
        [
            ods.null_count(doubled),
            ods.get(doubled, 1),
            ods.mean(s),
            ods.null_count(ods.fill_null(s, 0.0))
        ]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::Integer(1),
                Value::Unit,
                Value::Float(2.0),
                Value::Integer(0),
            ]
            .into()
        ))
    );
}

#[test]
fn comparison_masks_and_filter() {
    let result = assert_tier_transparent(
        r#"
        let s = ods.series([5, 1, 4, 2, 3])
        let big = ods.filter(s, s > 2)
        [ods.to_list(big), ods.to_list(ods.sort(s)), ods.len(ods.filter(s, s == 4))]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::List(vec![Value::Integer(5), Value::Integer(4), Value::Integer(3)].into()),
                Value::List(
                    vec![
                        Value::Integer(1),
                        Value::Integer(2),
                        Value::Integer(3),
                        Value::Integer(4),
                        Value::Integer(5),
                    ]
                    .into()
                ),
                Value::Integer(1),
            ]
            .into()
        ))
    );
}

#[test]
fn series_equality_is_structural_between_series() {
    let result = assert_tier_transparent(
        r#"
        let a = ods.series([1.0, 2.0])
        let b = ods.series([1.0, 2.0])
        let c = ods.series([1.0, 9.0])
        [a == b, a == c, a != c]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(true),
            ]
            .into()
        ))
    );
}

#[test]
fn range_constructor_and_pipeline_flow() {
    let result = assert_tier_transparent(
        r#"
        let s = ods.series(1..6)
        let total = 1..6 |> ods.series() |> ods.sum()
        [ods.len(s), total, ods.to_list(ods.cumsum(s))]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::Integer(5),
                Value::Integer(15),
                Value::List(
                    vec![
                        Value::Integer(1),
                        Value::Integer(3),
                        Value::Integer(6),
                        Value::Integer(10),
                        Value::Integer(15),
                    ]
                    .into()
                ),
            ]
            .into()
        ))
    );
}

#[test]
fn quantile_argsort_take_dot() {
    let result = assert_tier_transparent(
        r#"
        let s = ods.series([4.0, 1.0, 3.0, 2.0])
        let idx = ods.argsort(s)
        [
            ods.quantile(s, 0.5),
            ods.to_list(idx),
            ods.to_list(ods.take(s, idx)) == ods.to_list(ods.sort(s)),
            ods.dot(s, s)
        ]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::Float(2.5),
                Value::List(
                    vec![
                        Value::Integer(1),
                        Value::Integer(3),
                        Value::Integer(2),
                        Value::Integer(0),
                    ]
                    .into()
                ),
                Value::Boolean(true),
                Value::Float(30.0),
            ]
            .into()
        ))
    );
}

#[test]
fn division_by_zero_errors_in_both_tiers() {
    let result = assert_tier_transparent(
        r#"
        fn halve(s) = s / 0.0
        let a = halve(ods.series([1.0]))
        let b = halve(ods.series([1.0]))
        let c = halve(ods.series([1.0]))
        c
        "#,
    );
    assert!(result.is_err(), "series / 0.0 must be an error");
}

#[test]
fn typeof_display_and_zeros_linspace() {
    let result = assert_tier_transparent(
        r#"
        let s = ods.series([1, 2])
        let l = ods.linspace(0.0, 1.0, 3)
        [typeof(s), to_string(s), ods.to_list(l), ods.sum(ods.zeros(4))]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::String("Series".to_string().into()),
                Value::String("Series[Int; 2] [1, 2]".to_string().into()),
                Value::List(vec![Value::Float(0.0), Value::Float(0.5), Value::Float(1.0)].into()),
                Value::Float(0.0),
            ]
            .into()
        ))
    );
}

#[test]
fn scalar_on_the_left_broadcasts() {
    let result = assert_tier_transparent(
        r#"
        let s = ods.series([1.0, 2.0, 4.0])
        [ods.to_list(10.0 - s), ods.to_list(2 * s)]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::List(vec![Value::Float(9.0), Value::Float(8.0), Value::Float(6.0)].into()),
                Value::List(vec![Value::Float(2.0), Value::Float(4.0), Value::Float(8.0)].into()),
            ]
            .into()
        ))
    );
}

#[test]
fn elementwise_eq_masks_via_ods_eq() {
    let result = eval(
        r#"
        let a = ods.series([1, 2, 3])
        let b = ods.series([1, 9, 3])
        ods.to_list(ods.eq(a, b))
        "#,
        None,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(true),
            ]
            .into()
        ))
    );
}

#[test]
fn type_errors_are_informative() {
    let err = eval("ods.sum(3)", None).unwrap_err();
    assert!(err.contains("must be a Series"), "got: {}", err);
    // Strings became a real dtype in Phase 3; nested lists stay invalid.
    let err = eval("ods.series([[1, 2]])", None).unwrap_err();
    assert!(err.contains("Int, Float, Bool"), "got: {}", err);
    let err = eval("ods.series([\"a\", 1])", None).unwrap_err();
    assert!(err.contains("cannot mix String"), "got: {}", err);
    let err = eval("ods.quantile(ods.series([1.0]), 2.0)", None).unwrap_err();
    assert!(err.contains("[0, 1]"), "got: {}", err);
}

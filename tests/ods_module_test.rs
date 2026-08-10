//! Phase 0 seam tests for the ods OVM module (docs/design/ods.md).
//!
//! These pin the Native-value plumbing before any real engine exists:
//! the tier boundary shares one Arc (no conversion), operator
//! interception behaves identically in both tiers, and typeof / display /
//! equality / module dispatch all see the same value.

use olang::ast::Value;
use olang::native::NativeHandle;
use olang::ods::OdsProbe;
use olang::{Interpreter, OvmValue, Parser};

/// Run a closure on a thread with the same generous stack the CLI gives
/// the interpreter (test threads default to ~2 MB).
fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

/// Evaluate a program, optionally with the bytecode tier enabled.
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

/// Assert a program produces the same result with and without promotion,
/// and return the interpreted result for direct assertions.
fn assert_tier_transparent(source: &str) -> Result<Value, String> {
    let interpreted = eval(source, None);
    let promoted = eval(source, Some(2));

    match (&interpreted, &promoted) {
        (Ok(a), Ok(b)) => assert_eq!(
            a, b,
            "promotion changed the result\n  interpreted: {:?}\n  promoted:    {:?}\n  source: {}",
            a, b, source
        ),
        (Err(_), Err(_)) => {}
        _ => panic!(
            "promotion changed success/failure\n  interpreted: {:?}\n  promoted:    {:?}\n  source: {}",
            interpreted, promoted, source
        ),
    }
    interpreted
}

// ---------------------------------------------------------------------
// The load-bearing property: the tier boundary is an Arc clone.
// ---------------------------------------------------------------------

#[test]
fn native_value_crosses_tier_boundary_as_same_arc() {
    let handle = NativeHandle::new(OdsProbe { tag: 42 });
    let before = Value::Native(handle.clone());

    let crossed = OvmValue::from_ast(before);
    let back = crossed.to_ast().expect("to_ast");

    match back {
        Value::Native(after) => assert!(
            handle.ptr_eq(&after),
            "the tier boundary must share the allocation, not convert it"
        ),
        other => panic!("native value came back as {:?}", other),
    }
}

#[test]
fn native_values_round_trip_by_policy() {
    // round_trips is the single gate deciding what may cross the boundary;
    // if Native ever drops out of it, natives silently stay interpreted.
    let value = Value::Native(NativeHandle::new(OdsProbe { tag: 1 }));
    assert!(olang::ovm::bytecode::BytecodeVm::round_trips(&value));
}

// ---------------------------------------------------------------------
// Both tiers observe the same module: dispatch, typeof, display,
// equality, operator interception.
// ---------------------------------------------------------------------

#[test]
fn probe_arithmetic_is_tier_transparent() {
    // bump is pure arithmetic, so it promotes; the probe is created on the
    // interpreter and crosses into the VM, whose Add instruction must hit
    // the same module hook the interpreter uses.
    let result = assert_tier_transparent(
        r#"
        fn bump(p) = p + 3
        let a = bump(ods.probe(5))
        let b = bump(a)
        let c = bump(b)
        ods.probe_tag(c)
        "#,
    );
    assert_eq!(result, Ok(Value::Integer(14)));
}

#[test]
fn probe_addition_is_commutative_across_tiers() {
    let result = assert_tier_transparent(
        r#"
        fn bump_left(p) = 10 + p
        let x = bump_left(ods.probe(1))
        let y = bump_left(x)
        let z = bump_left(y)
        ods.probe_tag(z)
        "#,
    );
    assert_eq!(result, Ok(Value::Integer(31)));
}

#[test]
fn probe_equality_is_tier_transparent() {
    let result = assert_tier_transparent(
        r#"
        fn same(a, b) = a == b
        let p = ods.probe(7)
        let q = ods.probe(7)
        let r = ods.probe(8)
        let x = same(p, q)
        let y = same(p, q)
        let z = same(p, r)
        [x, y, z, p != r]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(true),
            ]
            .into()
        ))
    );
}

#[test]
fn probe_typeof_and_display() {
    let result = assert_tier_transparent(
        r#"
        let p = ods.probe(9)
        [typeof(p), to_string(p)]
        "#,
    );
    assert_eq!(
        result,
        Ok(Value::List(
            vec![
                Value::String("OdsProbe".to_string().into()),
                Value::String("<ods.probe 9>".to_string().into()),
            ]
            .into()
        ))
    );
}

#[test]
fn unsupported_probe_operation_errors_in_both_tiers() {
    // The module declines Multiply; both tiers must fail (the harness
    // treats matching Err/Err as transparent).
    let result = assert_tier_transparent(
        r#"
        fn shrink(p) = p * 2
        let a = shrink(ods.probe(5))
        let b = shrink(ods.probe(5))
        let c = shrink(ods.probe(5))
        ods.probe_tag(c)
        "#,
    );
    assert!(result.is_err(), "probe * Int must be a type error");
}

#[test]
fn ods_version_reports_phase() {
    let result = eval("ods.version()", None).expect("version");
    match result {
        Value::String(s) => assert!(s.contains("phase"), "got {}", s),
        other => panic!("ods.version() returned {:?}", other),
    }
}

#[test]
fn unknown_ods_function_reports_module_error() {
    let err = eval("ods.no_such_fn(1)", None);
    assert!(err.is_err());
}

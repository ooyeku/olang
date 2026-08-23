//! bigint — arbitrary-precision integers (Campaign 5, N1).
//!
//! The contract under test: BigInt follows the Int rules exactly
//! (truncating division, dividend-signed remainder, the same
//! zero-divisor errors), an Int operand promotes on contact, Floats
//! never mix implicitly, both tiers agree operator for operator, and
//! the Int overflow error names the way out.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str, tier_threshold: Option<u32>) -> Result<Value, String> {
    let parser = Parser::new();
    let program = parser.parse(source).map_err(|e| e.to_string())?;
    let mut interpreter = Interpreter::new();
    if let Some(threshold) = tier_threshold {
        interpreter.enable_bytecode_tier(threshold, false);
    }
    interpreter.eval_program(program).map_err(|e| e.to_string())
}

/// Both tiers must produce the same value — or the same failure.
fn assert_tier_transparent(source: &str) -> Result<Value, String> {
    let interpreted = eval(source, None);
    let promoted = eval(source, Some(2));
    assert_eq!(
        interpreted, promoted,
        "promotion changed the result\n  source: {}",
        source
    );
    interpreted
}

fn shows(source: &str) -> String {
    match assert_tier_transparent(source) {
        Ok(Value::String(s)) => s.as_ref().clone(),
        other => panic!(
            "expected a string result, got {:?}\n  source: {}",
            other, source
        ),
    }
}

#[test]
fn arithmetic_beyond_int_range() {
    assert_eq!(
        shows("to_string(bigint.pow(bigint.of(2), 200))"),
        "1606938044258990275541962092341162602522202993782792835301376"
    );
    // 50! — the classic overflow victim.
    assert_eq!(
        shows("to_string(fold(range(1, 51), bigint.of(1), (acc, i) => acc * i))"),
        "30414093201713378043612608166064768844377641568960512000000000000"
    );
}

#[test]
fn int_operands_promote_on_contact() {
    assert_eq!(shows("to_string(bigint.of(2) + 1)"), "3");
    assert_eq!(shows("to_string(1 + bigint.of(2))"), "3");
    assert_eq!(shows("to_string(10 - bigint.of(2))"), "8");
    assert_eq!(
        assert_tier_transparent("bigint.of(5) == 5").unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        assert_tier_transparent("3 < bigint.of(5) && bigint.of(5) <= 5").unwrap(),
        Value::Boolean(true)
    );
}

#[test]
fn division_follows_the_int_rules_exactly() {
    // Truncation toward zero and dividend-signed remainder, on both
    // signs — promoted arithmetic must change range, never answers.
    for (expr, want) in [
        ("-7 / 2", "-3"),
        ("-7 % 2", "-1"),
        ("7 / -2", "-3"),
        ("7 % -2", "1"),
    ] {
        let int_result = shows(&format!("to_string({})", expr));
        let big_result = shows(&format!(
            "to_string(bigint.of({}) {})",
            expr.split_whitespace().next().unwrap(),
            expr.split_once(' ').unwrap().1
        ));
        assert_eq!(int_result, want);
        assert_eq!(big_result, want, "BigInt disagrees with Int on {}", expr);
    }
}

#[test]
fn zero_divisors_raise_the_same_errors() {
    let div = eval("bigint.of(1) / 0", None).unwrap_err();
    assert!(div.contains("Division by zero"), "{}", div);
    let rem = eval("bigint.of(1) % 0", None).unwrap_err();
    assert!(rem.contains("Modulo by zero"), "{}", rem);
}

#[test]
fn floats_never_mix_implicitly() {
    let err = eval("bigint.of(2) * 0.5", None).unwrap_err();
    assert!(
        err.contains("do not mix implicitly") && err.contains("bigint.to_float"),
        "the refusal must say why and point at the door: {}",
        err
    );
    // The sanctioned door works.
    assert_eq!(
        assert_tier_transparent("bigint.to_float(bigint.of(4)) * 0.5").unwrap(),
        Value::Float(2.0)
    );
}

#[test]
fn round_trips_and_parse() {
    assert_eq!(
        shows("match bigint.to_int(bigint.of(42)) { Ok(i) => to_string(i), Err(m) => m }"),
        "42"
    );
    assert_eq!(
        shows(
            "match bigint.to_int(bigint.pow(bigint.of(2), 100)) { Ok(i) => to_string(i), Err(m) => m }"
        ),
        "bigint.to_int: 1267650600228229401496703205376 does not fit in a 64-bit Int"
    );
    assert_eq!(
        shows(
            "match bigint.parse(\"123456789012345678901234567890\") { Ok(b) => to_string(b), Err(m) => m }"
        ),
        "123456789012345678901234567890"
    );
    assert!(
        shows("match bigint.parse(\"nope\") { Ok(b) => to_string(b), Err(m) => m }")
            .contains("not a decimal integer")
    );
}

#[test]
fn number_theory_kernels() {
    // 561 is a Carmichael number: 7^560 ≡ 1 (mod 561).
    assert_eq!(
        shows("to_string(bigint.mod_pow(bigint.of(7), bigint.of(560), bigint.of(561)))"),
        "1"
    );
    assert_eq!(shows("to_string(bigint.gcd(bigint.of(48), 18))"), "6");
    assert_eq!(shows("to_string(bigint.gcd(bigint.of(-48), -18))"), "6");
    assert_eq!(shows("to_string(bigint.abs(bigint.of(-7)))"), "7");
    assert_eq!(shows("to_string(bigint.neg(bigint.of(7)))"), "-7");
    assert_eq!(shows("to_string(0 - bigint.of(7))"), "-7");
}

#[test]
fn typeof_display_and_equality() {
    assert_eq!(shows("typeof(bigint.of(1))"), "BigInt");
    assert_eq!(
        assert_tier_transparent("bigint.of(\"100\") == bigint.of(100)").unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        assert_tier_transparent("bigint.of(1) == ()").unwrap(),
        Value::Boolean(false)
    );
    // Template interpolation shows the digits, same as println.
    assert_eq!(
        shows("let b = bigint.pow(bigint.of(10), 30)\n`${b}`"),
        "1000000000000000000000000000000"
    );
}

#[test]
fn the_overflow_error_names_the_way_out() {
    let err = eval("9223372036854775807 + 1", None).unwrap_err();
    assert!(
        err.contains("Integer overflow") && err.contains("bigint.of"),
        "the Int overflow error must point at bigint: {}",
        err
    );
    // And the pointed-at path actually computes the value.
    assert_eq!(
        shows("to_string(bigint.of(9223372036854775807) + 1)"),
        "9223372036854775808"
    );
}

#[test]
fn misuse_raises_rather_than_returns() {
    for bad in [
        "bigint.of(1.5)",
        "bigint.of(true)",
        "bigint.pow(bigint.of(2), -1)",
        "bigint.mod_pow(bigint.of(2), bigint.of(3), bigint.of(0))",
        "bigint.of(\"12x\")",
    ] {
        assert!(eval(bad, None).is_err(), "{} must raise", bad);
    }
}

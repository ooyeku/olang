//! Numbers read back in (roadmap lane W2).
//!
//! The runtime used to print floats the parser could not read — `1e21`
//! out of `to_string`, while `1e20` in source was a parse error — and
//! JSON silently converted oversized integers to floats. These tests pin
//! the repaired contract: every printed finite float is a valid literal
//! that parses back bit-for-bit, and JSON numbers are lossless or loud.

use olang::ast::{Value, format_float};
use olang::interpreter::Interpreter;
use olang::parser::Parser;
use proptest::prelude::*;

fn eval(source: &str) -> Result<Value, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    Interpreter::new()
        .eval_program(program)
        .map_err(|e| e.to_string())
}

// ── scientific-notation literals ──────────────────────────────────────

#[test]
fn exponent_only_floats_parse() {
    for (src, want) in [
        ("1e20", 1e20),
        ("1E20", 1e20),
        ("2.5e-3", 2.5e-3),
        ("1E+6", 1e6),
        ("1e-4", 1e-4),
        ("9.999999999999999e-10", 9.999999999999999e-10),
        ("1_000e3", 1_000e3),
    ] {
        match eval(src) {
            Ok(Value::Float(got)) => assert_eq!(got, want, "{src}"),
            other => panic!("{src} did not evaluate to a Float: {other:?}"),
        }
    }
}

#[test]
fn neighbors_of_the_float_grammar_are_untouched() {
    // Hex with an `e` digit is still an integer.
    assert!(matches!(eval("0x1e2"), Ok(Value::Integer(482))));
    // Ranges still parse — the `.` rule did not loosen.
    assert!(matches!(
        eval("let mut s = 0\nfor i in 1..4 { s = s + i }\ns"),
        Ok(Value::Integer(6))
    ));
    // A malformed exponent is still an error, not a partial parse.
    assert!(eval("1e").is_err());
    assert!(eval("1e+").is_err());
}

// ── print/parse round-trip ────────────────────────────────────────────

proptest! {
    /// Every finite f64, printed by the runtime's own formatter, parses
    /// back to the identical bits. This is the round-trip the lane is
    /// named for: printed values are source.
    #[test]
    fn printed_floats_parse_back_bit_for_bit(bits in any::<u64>()) {
        let x = f64::from_bits(bits);
        prop_assume!(x.is_finite());
        let printed = format_float(x);
        match eval(&printed) {
            Ok(Value::Float(back)) => {
                prop_assert_eq!(
                    back.to_bits(), x.to_bits(),
                    "{} reparsed as {}", printed, back
                );
            }
            other => {
                return Err(TestCaseError::fail(format!(
                    "printed float {printed:?} did not parse back: {other:?}"
                )));
            }
        }
    }
}

#[test]
fn the_observed_offenders_round_trip() {
    // The two printed forms the roadmap row cites, plus the boundaries
    // of format_float's exponent window.
    for x in [
        1e21,
        9.999999999999999e-10,
        1e16,
        1e-4,
        9.999e15,
        -1e21,
        -0.0,
    ] {
        let printed = format_float(x);
        match eval(&printed) {
            Ok(Value::Float(back)) => assert_eq!(back.to_bits(), x.to_bits(), "{printed}"),
            other => panic!("{printed} did not parse back: {other:?}"),
        }
    }
}

// ── JSON numeric fidelity ─────────────────────────────────────────────

#[test]
fn json_integers_within_i64_are_lossless() {
    // 2^53 + 1: exactly where f64 starts losing integers.
    let v = eval("unwrap(json.parse(\"9007199254740993\"))").unwrap();
    assert!(matches!(v, Value::Integer(9007199254740993)), "{v:?}");
    let v = eval("unwrap(json.parse(\"-9223372036854775808\"))").unwrap();
    assert!(matches!(v, Value::Integer(i64::MIN)), "{v:?}");
}

#[test]
fn json_oversized_integers_refuse_the_lossy_read() {
    for src in [
        "json.parse(\"99999999999999999999999999\")",
        "json.parse(\"-99999999999999999999999999\")",
        "json.parse(\"18446744073709551615\")", // fits u64, not i64
        "json.parse(\"{\\\"n\\\": 99999999999999999999999999}\")", // nested
    ] {
        match eval(src).unwrap() {
            Value::Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("does not fit Int"), "{src}: {msg}");
                assert!(msg.contains("bigint"), "no bigint pointer: {msg}");
            }
            other => panic!("{src} read losslessly?! {other:?}"),
        }
    }
}

#[test]
fn json_floats_keep_the_ieee_reading_and_never_go_infinite() {
    // A decimal is a float even when it lands on an integer value.
    let v = eval("unwrap(json.parse(\"1.0\"))").unwrap();
    assert!(matches!(v, Value::Float(f) if f == 1.0), "{v:?}");
    let v = eval("unwrap(json.parse(\"1.5e300\"))").unwrap();
    assert!(matches!(v, Value::Float(f) if f == 1.5e300), "{v:?}");
    // Overflowing float text errors instead of producing inf.
    match eval("json.parse(\"1e400\")").unwrap() {
        Value::Err(e) => assert!(e.to_string().contains("does not fit Float"), "{e:?}"),
        other => panic!("1e400 produced {other:?}"),
    }
}

#[test]
fn json_round_trips_the_i64_edge() {
    let v = eval(
        "let s = unwrap(json.stringify(#{\"x\": 9007199254740993}))\n\
         let back = unwrap(json.parse(s))\n\
         map_get(back, \"x\")",
    )
    .unwrap();
    assert!(matches!(v, Value::Integer(9007199254740993)), "{v:?}");
}

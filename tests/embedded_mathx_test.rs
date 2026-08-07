//! `mathx` is the pure subset of `math`, written in olang and embedded in the
//! binary. Each function is differential-tested against the native `math`
//! builtin: exact equality for the integer/rational/rounding operations,
//! and a tolerance for `sqrt` (Newton's method converges to within ~1 ulp of
//! the correctly-rounded native sqrt, not to the exact same bits).

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

/// The olang `mathx` function and the native `math` function must produce the
/// exact same value.
fn assert_exact(mathx_call: &str, math_call: &str) {
    let olang_side = eval(&format!("use mathx {{ * }}\n{}", mathx_call));
    let native = eval(math_call);
    assert_eq!(
        olang_side, native,
        "mathx `{}` != math `{}`",
        mathx_call, math_call
    );
}

fn as_f64(v: &Value) -> f64 {
    match v {
        Value::Float(f) => *f,
        Value::Integer(n) => *n as f64,
        other => panic!("expected number, got {:?}", other),
    }
}

/// For a transcendental like sqrt: agree to within a tight tolerance.
fn assert_close(mathx_call: &str, math_call: &str) {
    let olang_side = as_f64(&eval(&format!("use mathx {{ * }}\n{}", mathx_call)));
    let native = as_f64(&eval(math_call));
    assert!(
        (olang_side - native).abs() < 1e-10,
        "mathx `{}` = {} vs math `{}` = {} (diff too large)",
        mathx_call,
        olang_side,
        math_call,
        native
    );
}

#[test]
fn constants_match() {
    assert_exact("PI", "math.PI");
    assert_exact("E", "math.E");
    assert_exact("TAU", "math.TAU");
}

#[test]
fn sign_and_magnitude_agree() {
    for x in ["0 - 5", "7", "0"] {
        assert_exact(&format!("abs({x})"), &format!("math.abs({x})"));
        assert_exact(&format!("sign({x})"), &format!("math.sign({x})"));
    }
    assert_exact("min(3, 7)", "math.min(3, 7)");
    assert_exact("max(3, 7)", "math.max(3, 7)");
    assert_exact("min(9, 2)", "math.min(9, 2)");
}

#[test]
fn number_theory_agrees() {
    for (a, b) in [("12", "8"), ("0 - 12", "8"), ("17", "5"), ("0", "9")] {
        assert_exact(&format!("gcd({a}, {b})"), &format!("math.gcd({a}, {b})"));
        assert_exact(&format!("lcm({a}, {b})"), &format!("math.lcm({a}, {b})"));
    }
    for n in ["0", "1", "5", "10"] {
        assert_exact(&format!("factorial({n})"), &format!("math.factorial({n})"));
    }
}

#[test]
fn rounding_agrees_including_negatives() {
    let xs = [
        "2.3",
        "2.7",
        "0.0 - 2.3",
        "0.0 - 2.7",
        "2.5",
        "0.0 - 2.5",
        "3.0",
        "0.0",
    ];
    for x in xs {
        assert_exact(&format!("floor({x})"), &format!("math.floor({x})"));
        assert_exact(&format!("ceil({x})"), &format!("math.ceil({x})"));
        assert_exact(&format!("trunc({x})"), &format!("math.trunc({x})"));
        assert_exact(&format!("round({x})"), &format!("math.round({x})"));
        assert_exact(&format!("fract({x})"), &format!("math.fract({x})"));
    }
}

#[test]
fn angle_conversion_agrees() {
    for d in ["0.0", "90.0", "180.0", "45.0"] {
        assert_exact(&format!("radians({d})"), &format!("math.radians({d})"));
    }
    for r in ["0.0", "1.5707963267948966", "3.141592653589793"] {
        assert_exact(&format!("degrees({r})"), &format!("math.degrees({r})"));
    }
}

#[test]
fn sqrt_agrees_within_tolerance() {
    for x in ["2.0", "9.0", "0.5", "1000.0", "0.0", "1.0"] {
        assert_close(&format!("sqrt({x})"), &format!("math.sqrt({x})"));
    }
}

#[test]
fn mathx_loads_and_binds_as_a_namespace() {
    let src = "use mathx { gcd }\n[typeof(mathx), mathx.factorial(5), gcd(12, 8)]";
    match eval(src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::String("Module".to_string().into()));
            assert_eq!(items[1], Value::Integer(120));
            assert_eq!(items[2], Value::Integer(4));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

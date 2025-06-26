use crate::ast::Value;
use std::collections::HashMap;

/// Error types for mathematical operations
#[derive(Debug, thiserror::Error)]
pub enum MathError {
    #[error("Domain error: {message}")]
    DomainError { message: String },
    #[error("Range error: {message}")]
    RangeError { message: String },
    #[error("Division by zero")]
    DivisionByZero,
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
}

/// Creates the math module with all mathematical functions and constants
pub fn create_math_module() -> Value {
    let mut module = HashMap::new();

    // Mathematical constants
    module.insert("PI".to_string(), Value::Float(std::f64::consts::PI));
    module.insert("E".to_string(), Value::Float(std::f64::consts::E));
    module.insert("TAU".to_string(), Value::Float(std::f64::consts::TAU));
    module.insert("SQRT_2".to_string(), Value::Float(std::f64::consts::SQRT_2));
    module.insert("SQRT_3".to_string(), Value::Float(3_f64.sqrt()));
    module.insert("LN_2".to_string(), Value::Float(std::f64::consts::LN_2));
    module.insert("LN_10".to_string(), Value::Float(std::f64::consts::LN_10));
    module.insert("LOG2_E".to_string(), Value::Float(std::f64::consts::LOG2_E));
    module.insert(
        "LOG10_E".to_string(),
        Value::Float(std::f64::consts::LOG10_E),
    );

    // Basic mathematical functions
    module.insert("abs".to_string(), create_builtin_function("abs", 1));
    module.insert("min".to_string(), create_builtin_function("min", 2));
    module.insert("max".to_string(), create_builtin_function("max", 2));
    module.insert("pow".to_string(), create_builtin_function("pow", 2));
    module.insert("sqrt".to_string(), create_builtin_function("sqrt", 1));
    module.insert("cbrt".to_string(), create_builtin_function("cbrt", 1));

    // Rounding functions
    module.insert("floor".to_string(), create_builtin_function("floor", 1));
    module.insert("ceil".to_string(), create_builtin_function("ceil", 1));
    module.insert("round".to_string(), create_builtin_function("round", 1));
    module.insert("trunc".to_string(), create_builtin_function("trunc", 1));
    module.insert("fract".to_string(), create_builtin_function("fract", 1));

    // Trigonometric functions
    module.insert("sin".to_string(), create_builtin_function("sin", 1));
    module.insert("cos".to_string(), create_builtin_function("cos", 1));
    module.insert("tan".to_string(), create_builtin_function("tan", 1));
    module.insert("asin".to_string(), create_builtin_function("asin", 1));
    module.insert("acos".to_string(), create_builtin_function("acos", 1));
    module.insert("atan".to_string(), create_builtin_function("atan", 1));
    module.insert("atan2".to_string(), create_builtin_function("atan2", 2));

    // Hyperbolic functions
    module.insert("sinh".to_string(), create_builtin_function("sinh", 1));
    module.insert("cosh".to_string(), create_builtin_function("cosh", 1));
    module.insert("tanh".to_string(), create_builtin_function("tanh", 1));

    // Logarithmic and exponential functions
    module.insert("exp".to_string(), create_builtin_function("exp", 1));
    module.insert("exp2".to_string(), create_builtin_function("exp2", 1));
    module.insert("ln".to_string(), create_builtin_function("ln", 1));
    module.insert("log".to_string(), create_builtin_function("log", 2));
    module.insert("log2".to_string(), create_builtin_function("log2", 1));
    module.insert("log10".to_string(), create_builtin_function("log10", 1));

    // Utility functions
    module.insert("sign".to_string(), create_builtin_function("sign", 1));
    module.insert("degrees".to_string(), create_builtin_function("degrees", 1));
    module.insert("radians".to_string(), create_builtin_function("radians", 1));
    module.insert("gcd".to_string(), create_builtin_function("gcd", 2));
    module.insert("lcm".to_string(), create_builtin_function("lcm", 2));
    module.insert(
        "factorial".to_string(),
        create_builtin_function("factorial", 1),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("math.{}", name),
        arity,
    })
}

/// Main dispatcher for math function calls
pub fn call_math_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "abs" => math_abs(args),
        "min" => math_min(args),
        "max" => math_max(args),
        "pow" => math_pow(args),
        "sqrt" => math_sqrt(args),
        "cbrt" => math_cbrt(args),
        "floor" => math_floor(args),
        "ceil" => math_ceil(args),
        "round" => math_round(args),
        "trunc" => math_trunc(args),
        "fract" => math_fract(args),
        "sin" => math_sin(args),
        "cos" => math_cos(args),
        "tan" => math_tan(args),
        "asin" => math_asin(args),
        "acos" => math_acos(args),
        "atan" => math_atan(args),
        "atan2" => math_atan2(args),
        "sinh" => math_sinh(args),
        "cosh" => math_cosh(args),
        "tanh" => math_tanh(args),
        "exp" => math_exp(args),
        "exp2" => math_exp2(args),
        "ln" => math_ln(args),
        "log" => math_log(args),
        "log2" => math_log2(args),
        "log10" => math_log10(args),
        "sign" => math_sign(args),
        "degrees" => math_degrees(args),
        "radians" => math_radians(args),
        "gcd" => math_gcd(args),
        "lcm" => math_lcm(args),
        "factorial" => math_factorial(args),
        _ => Err(format!("Unknown math function: {}", name).into()),
    }
}

/// Get numeric value from Olang Value
fn get_numeric(value: &Value) -> Result<f64, Box<dyn std::error::Error>> {
    match value {
        Value::Integer(n) => Ok(*n as f64),
        Value::Float(f) => Ok(*f),
        _ => Err("Expected numeric value".into()),
    }
}

/// Absolute value
/// Usage: math.abs(-5) -> 5
fn math_abs(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("abs expects 1 argument, got {}", args.len()).into());
    }

    match &args[0] {
        Value::Integer(n) => Ok(Value::Integer(n.abs())),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        _ => Err("abs: argument must be a number".into()),
    }
}

/// Minimum of two numbers
/// Usage: math.min(3, 7) -> 3
fn math_min(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("min expects 2 arguments, got {}", args.len()).into());
    }

    let a = get_numeric(&args[0])?;
    let b = get_numeric(&args[1])?;

    let result = a.min(b);
    match (&args[0], &args[1]) {
        (Value::Integer(_), Value::Integer(_)) => Ok(Value::Integer(result as i64)),
        _ => Ok(Value::Float(result)),
    }
}

/// Maximum of two numbers
/// Usage: math.max(3, 7) -> 7
fn math_max(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("max expects 2 arguments, got {}", args.len()).into());
    }

    let a = get_numeric(&args[0])?;
    let b = get_numeric(&args[1])?;

    let result = a.max(b);
    match (&args[0], &args[1]) {
        (Value::Integer(_), Value::Integer(_)) => Ok(Value::Integer(result as i64)),
        _ => Ok(Value::Float(result)),
    }
}

/// Power function
/// Usage: math.pow(2, 3) -> 8
fn math_pow(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("pow expects 2 arguments, got {}", args.len()).into());
    }

    let base = get_numeric(&args[0])?;
    let exponent = get_numeric(&args[1])?;

    Ok(Value::Float(base.powf(exponent)))
}

/// Square root
/// Usage: math.sqrt(16) -> 4.0
fn math_sqrt(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("sqrt expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    if n < 0.0 {
        return Err("sqrt: cannot take square root of negative number".into());
    }

    Ok(Value::Float(n.sqrt()))
}

/// Cube root
/// Usage: math.cbrt(27) -> 3.0
fn math_cbrt(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("cbrt expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.cbrt()))
}

/// Floor function
/// Usage: math.floor(3.7) -> 3.0
fn math_floor(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("floor expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.floor()))
}

/// Ceiling function
/// Usage: math.ceil(3.2) -> 4.0
fn math_ceil(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("ceil expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.ceil()))
}

/// Round to nearest integer
/// Usage: math.round(3.6) -> 4.0
fn math_round(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("round expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.round()))
}

/// Truncate to integer part
/// Usage: math.trunc(3.7) -> 3.0
fn math_trunc(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("trunc expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.trunc()))
}

/// Fractional part
/// Usage: math.fract(3.7) -> 0.7
fn math_fract(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("fract expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.fract()))
}

/// Sine function
/// Usage: math.sin(math.PI / 2) -> 1.0
fn math_sin(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("sin expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.sin()))
}

/// Cosine function
/// Usage: math.cos(0) -> 1.0
fn math_cos(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("cos expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.cos()))
}

/// Tangent function
/// Usage: math.tan(math.PI / 4) -> 1.0
fn math_tan(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("tan expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.tan()))
}

/// Arcsine function
/// Usage: math.asin(1) -> π/2
fn math_asin(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("asin expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    if !(-1.0..=1.0).contains(&n) {
        return Err("asin: input must be in range [-1, 1]".into());
    }

    Ok(Value::Float(n.asin()))
}

/// Arccosine function
/// Usage: math.acos(0) -> π/2
fn math_acos(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("acos expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    if !(-1.0..=1.0).contains(&n) {
        return Err("acos: input must be in range [-1, 1]".into());
    }

    Ok(Value::Float(n.acos()))
}

/// Arctangent function
/// Usage: math.atan(1) -> π/4
fn math_atan(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("atan expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.atan()))
}

/// Two-argument arctangent
/// Usage: math.atan2(y, x) -> angle from x-axis to point (x,y)
fn math_atan2(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("atan2 expects 2 arguments, got {}", args.len()).into());
    }

    let y = get_numeric(&args[0])?;
    let x = get_numeric(&args[1])?;
    Ok(Value::Float(y.atan2(x)))
}

/// Hyperbolic sine
/// Usage: math.sinh(1) -> 1.175...
fn math_sinh(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("sinh expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.sinh()))
}

/// Hyperbolic cosine
/// Usage: math.cosh(0) -> 1.0
fn math_cosh(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("cosh expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.cosh()))
}

/// Hyperbolic tangent
/// Usage: math.tanh(0) -> 0.0
fn math_tanh(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("tanh expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.tanh()))
}

/// Natural exponential
/// Usage: math.exp(1) -> e
fn math_exp(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("exp expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.exp()))
}

/// Base-2 exponential
/// Usage: math.exp2(3) -> 8.0
fn math_exp2(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("exp2 expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.exp2()))
}

/// Natural logarithm
/// Usage: math.ln(math.E) -> 1.0
fn math_ln(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("ln expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    if n <= 0.0 {
        return Err("ln: input must be positive".into());
    }

    Ok(Value::Float(n.ln()))
}

/// Logarithm with custom base
/// Usage: math.log(100, 10) -> 2.0
fn math_log(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("log expects 2 arguments, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    let base = get_numeric(&args[1])?;

    if n <= 0.0 {
        return Err("log: input must be positive".into());
    }
    if base <= 0.0 || base == 1.0 {
        return Err("log: base must be positive and not equal to 1".into());
    }

    Ok(Value::Float(n.log(base)))
}

/// Base-2 logarithm
/// Usage: math.log2(8) -> 3.0
fn math_log2(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("log2 expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    if n <= 0.0 {
        return Err("log2: input must be positive".into());
    }

    Ok(Value::Float(n.log2()))
}

/// Base-10 logarithm
/// Usage: math.log10(100) -> 2.0
fn math_log10(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("log10 expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    if n <= 0.0 {
        return Err("log10: input must be positive".into());
    }

    Ok(Value::Float(n.log10()))
}

/// Sign function
/// Usage: math.sign(-5) -> -1
fn math_sign(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("sign expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    let result = if n > 0.0 {
        1.0
    } else if n < 0.0 {
        -1.0
    } else {
        0.0
    };
    Ok(Value::Integer(result as i64))
}

/// Convert radians to degrees
/// Usage: math.degrees(math.PI) -> 180.0
fn math_degrees(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("degrees expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.to_degrees()))
}

/// Convert degrees to radians
/// Usage: math.radians(180) -> π
fn math_radians(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("radians expects 1 argument, got {}", args.len()).into());
    }

    let n = get_numeric(&args[0])?;
    Ok(Value::Float(n.to_radians()))
}

/// Greatest common divisor
/// Usage: math.gcd(12, 8) -> 4
fn math_gcd(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("gcd expects 2 arguments, got {}", args.len()).into());
    }

    let a = match &args[0] {
        Value::Integer(n) => *n,
        _ => return Err("gcd: arguments must be integers".into()),
    };

    let b = match &args[1] {
        Value::Integer(n) => *n,
        _ => return Err("gcd: arguments must be integers".into()),
    };

    fn gcd_impl(a: i64, b: i64) -> i64 {
        if b == 0 {
            a.abs()
        } else {
            gcd_impl(b, a % b)
        }
    }

    Ok(Value::Integer(gcd_impl(a, b)))
}

/// Least common multiple
/// Usage: math.lcm(4, 6) -> 12
fn math_lcm(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("lcm expects 2 arguments, got {}", args.len()).into());
    }

    let a = match &args[0] {
        Value::Integer(n) => *n,
        _ => return Err("lcm: arguments must be integers".into()),
    };

    let b = match &args[1] {
        Value::Integer(n) => *n,
        _ => return Err("lcm: arguments must be integers".into()),
    };

    if a == 0 || b == 0 {
        return Ok(Value::Integer(0));
    }

    // Use the identity: lcm(a,b) = |a*b| / gcd(a,b)
    fn gcd_impl(a: i64, b: i64) -> i64 {
        if b == 0 {
            a.abs()
        } else {
            gcd_impl(b, a % b)
        }
    }

    let gcd = gcd_impl(a, b);
    let lcm = (a.abs() / gcd) * b.abs();

    Ok(Value::Integer(lcm))
}

/// Factorial function
/// Usage: math.factorial(5) -> 120
fn math_factorial(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("factorial expects 1 argument, got {}", args.len()).into());
    }

    let n = match &args[0] {
        Value::Integer(n) => *n,
        _ => return Err("factorial: argument must be an integer".into()),
    };

    if n < 0 {
        return Err("factorial: argument must be non-negative".into());
    }

    if n > 20 {
        return Err("factorial: argument too large (maximum 20)".into());
    }

    let mut result = 1i64;
    for i in 1..=n {
        result *= i;
    }

    Ok(Value::Integer(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Value;
    use std::f64::consts;
    use std::sync::Arc;

    // Helper function to create test values
    fn int_val(n: i64) -> Value {
        Value::Integer(n)
    }

    fn float_val(f: f64) -> Value {
        Value::Float(f)
    }

    fn string_val(s: &str) -> Value {
        Value::String(Arc::new(s.to_string()))
    }

    // Helper function to assert float equality with tolerance
    fn assert_float_eq(actual: &Value, expected: f64, tolerance: f64) {
        match actual {
            Value::Float(f) => {
                // Handle special cases for infinity and NaN
                if expected.is_infinite()
                    && f.is_infinite()
                    && expected.is_sign_positive() == f.is_sign_positive()
                {
                    return; // Both are the same type of infinity
                }
                if expected.is_nan() && f.is_nan() {
                    return; // Both are NaN
                }
                assert!(
                    (f - expected).abs() < tolerance,
                    "Expected {}, got {}, difference: {}",
                    expected,
                    f,
                    (f - expected).abs()
                );
            }
            _ => panic!("Expected float value, got {:?}", actual),
        }
    }

    fn assert_int_eq(actual: &Value, expected: i64) {
        match actual {
            Value::Integer(i) => assert_eq!(*i, expected),
            _ => panic!("Expected integer value, got {:?}", actual),
        }
    }

    #[test]
    fn test_math_constants() {
        let module = create_math_module();
        if let Value::Struct { fields, .. } = module {
            assert_float_eq(fields.get("PI").unwrap(), consts::PI, 1e-15);
            assert_float_eq(fields.get("E").unwrap(), consts::E, 1e-15);
            assert_float_eq(fields.get("TAU").unwrap(), consts::TAU, 1e-15);
            assert_float_eq(fields.get("SQRT_2").unwrap(), consts::SQRT_2, 1e-15);
            assert_float_eq(fields.get("SQRT_3").unwrap(), 3_f64.sqrt(), 1e-15);
            assert_float_eq(fields.get("LN_2").unwrap(), consts::LN_2, 1e-15);
            assert_float_eq(fields.get("LN_10").unwrap(), consts::LN_10, 1e-15);
            assert_float_eq(fields.get("LOG2_E").unwrap(), consts::LOG2_E, 1e-15);
            assert_float_eq(fields.get("LOG10_E").unwrap(), consts::LOG10_E, 1e-15);
        } else {
            panic!("Expected struct for math module");
        }
    }

    #[test]
    fn test_abs() {
        // Test with integers
        let result = math_abs(vec![int_val(-5)]).unwrap();
        assert_int_eq(&result, 5);

        let result = math_abs(vec![int_val(5)]).unwrap();
        assert_int_eq(&result, 5);

        let result = math_abs(vec![int_val(0)]).unwrap();
        assert_int_eq(&result, 0);

        // Test with floats
        let result = math_abs(vec![float_val(-std::f64::consts::PI)]).unwrap();
        assert_float_eq(&result, std::f64::consts::PI, 1e-15);

        let result = math_abs(vec![float_val(2.71)]).unwrap();
        assert_float_eq(&result, 2.71, 1e-15);

        // Test edge cases (i64::MIN causes overflow in Rust, so we expect it to panic in debug mode)
        // In release mode it would wrap around, but this is expected behavior

        // Test errors
        assert!(math_abs(vec![]).is_err());
        assert!(math_abs(vec![int_val(1), int_val(2)]).is_err());
        assert!(math_abs(vec![string_val("hello")]).is_err());
    }

    #[test]
    fn test_min_max() {
        // Test with integers
        let result = math_min(vec![int_val(3), int_val(7)]).unwrap();
        assert_int_eq(&result, 3);

        let result = math_max(vec![int_val(3), int_val(7)]).unwrap();
        assert_int_eq(&result, 7);

        // Test with floats
        let result = math_min(vec![float_val(3.5), float_val(2.1)]).unwrap();
        assert_float_eq(&result, 2.1, 1e-15);

        let result = math_max(vec![float_val(3.5), float_val(2.1)]).unwrap();
        assert_float_eq(&result, 3.5, 1e-15);

        // Test mixed types (should promote to float)
        let result = math_min(vec![int_val(3), float_val(2.5)]).unwrap();
        assert_float_eq(&result, 2.5, 1e-15);

        let result = math_max(vec![int_val(3), float_val(2.5)]).unwrap();
        assert_float_eq(&result, 3.0, 1e-15);

        // Test edge cases
        let result = math_min(vec![float_val(f64::INFINITY), float_val(1.0)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15);

        let result = math_max(vec![float_val(f64::NEG_INFINITY), float_val(1.0)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15);

        // Test errors
        assert!(math_min(vec![int_val(1)]).is_err());
        assert!(math_max(vec![int_val(1), int_val(2), int_val(3)]).is_err());
    }

    #[test]
    fn test_pow() {
        // Basic tests
        let result = math_pow(vec![int_val(2), int_val(3)]).unwrap();
        assert_float_eq(&result, 8.0, 1e-15);

        let result = math_pow(vec![float_val(2.0), float_val(0.5)]).unwrap();
        assert_float_eq(&result, 2_f64.sqrt(), 1e-15);

        // Edge cases
        let result = math_pow(vec![int_val(0), int_val(0)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15); // 0^0 = 1 by convention

        let result = math_pow(vec![int_val(1), float_val(f64::INFINITY)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15);

        // Negative base with fractional exponent
        let result = math_pow(vec![int_val(-2), float_val(0.5)]).unwrap();
        if let Value::Float(f) = result {
            assert!(
                f.is_nan(),
                "Expected NaN for negative base with fractional exponent"
            );
        } else {
            panic!("Expected float result");
        }
    }

    #[test]
    fn test_sqrt() {
        let result = math_sqrt(vec![int_val(16)]).unwrap();
        assert_float_eq(&result, 4.0, 1e-15);

        let result = math_sqrt(vec![float_val(2.0)]).unwrap();
        assert_float_eq(&result, 2_f64.sqrt(), 1e-15);

        let result = math_sqrt(vec![int_val(0)]).unwrap();
        assert_float_eq(&result, 0.0, 1e-15);

        // Test error for negative input
        assert!(math_sqrt(vec![int_val(-1)]).is_err());
        assert!(math_sqrt(vec![float_val(-0.1)]).is_err());
    }

    #[test]
    fn test_cbrt() {
        let result = math_cbrt(vec![int_val(27)]).unwrap();
        assert_float_eq(&result, 3.0, 1e-15);

        let result = math_cbrt(vec![int_val(-8)]).unwrap();
        assert_float_eq(&result, -2.0, 1e-15);

        let result = math_cbrt(vec![int_val(0)]).unwrap();
        assert_float_eq(&result, 0.0, 1e-15);
    }

    #[test]
    fn test_rounding_functions() {
        // Test floor
        let result = math_floor(vec![float_val(3.7)]).unwrap();
        assert_float_eq(&result, 3.0, 1e-15);

        let result = math_floor(vec![float_val(-3.7)]).unwrap();
        assert_float_eq(&result, -4.0, 1e-15);

        // Test ceil
        let result = math_ceil(vec![float_val(3.2)]).unwrap();
        assert_float_eq(&result, 4.0, 1e-15);

        let result = math_ceil(vec![float_val(-3.2)]).unwrap();
        assert_float_eq(&result, -3.0, 1e-15);

        // Test round
        let result = math_round(vec![float_val(3.6)]).unwrap();
        assert_float_eq(&result, 4.0, 1e-15);

        let result = math_round(vec![float_val(3.4)]).unwrap();
        assert_float_eq(&result, 3.0, 1e-15);

        let result = math_round(vec![float_val(-3.6)]).unwrap();
        assert_float_eq(&result, -4.0, 1e-15);

        // Test trunc
        let result = math_trunc(vec![float_val(3.9)]).unwrap();
        assert_float_eq(&result, 3.0, 1e-15);

        let result = math_trunc(vec![float_val(-3.9)]).unwrap();
        assert_float_eq(&result, -3.0, 1e-15);

        // Test fract
        let result = math_fract(vec![float_val(3.7)]).unwrap();
        assert_float_eq(&result, 0.7, 1e-15);

        let result = math_fract(vec![float_val(-3.7)]).unwrap();
        assert_float_eq(&result, -0.7, 1e-15);
    }

    #[test]
    fn test_trigonometric_functions() {
        // Test basic trig functions
        let result = math_sin(vec![float_val(consts::PI / 2.0)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15);

        let result = math_cos(vec![float_val(0.0)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15);

        let result = math_tan(vec![float_val(consts::PI / 4.0)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15);

        // Test inverse trig functions
        let result = math_asin(vec![float_val(1.0)]).unwrap();
        assert_float_eq(&result, consts::PI / 2.0, 1e-15);

        let result = math_acos(vec![float_val(0.0)]).unwrap();
        assert_float_eq(&result, consts::PI / 2.0, 1e-15);

        let result = math_atan(vec![float_val(1.0)]).unwrap();
        assert_float_eq(&result, consts::PI / 4.0, 1e-15);

        // Test atan2
        let result = math_atan2(vec![float_val(1.0), float_val(1.0)]).unwrap();
        assert_float_eq(&result, consts::PI / 4.0, 1e-15);

        let result = math_atan2(vec![float_val(1.0), float_val(0.0)]).unwrap();
        assert_float_eq(&result, consts::PI / 2.0, 1e-15);

        // Test domain errors for inverse trig
        assert!(math_asin(vec![float_val(2.0)]).is_err());
        assert!(math_asin(vec![float_val(-2.0)]).is_err());
        assert!(math_acos(vec![float_val(2.0)]).is_err());
        assert!(math_acos(vec![float_val(-2.0)]).is_err());

        // Test trigonometric identities
        let angle = consts::PI / 6.0; // 30 degrees
        let sin_result = math_sin(vec![float_val(angle)]).unwrap();
        let cos_result = math_cos(vec![float_val(angle)]).unwrap();

        if let (Value::Float(sin_val), Value::Float(cos_val)) = (sin_result, cos_result) {
            // sin²θ + cos²θ = 1
            let identity_result = sin_val * sin_val + cos_val * cos_val;
            assert_float_eq(&Value::Float(identity_result), 1.0, 1e-15);
        }
    }

    #[test]
    fn test_hyperbolic_functions() {
        let result = math_sinh(vec![float_val(0.0)]).unwrap();
        assert_float_eq(&result, 0.0, 1e-15);

        let result = math_cosh(vec![float_val(0.0)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15);

        let result = math_tanh(vec![float_val(0.0)]).unwrap();
        assert_float_eq(&result, 0.0, 1e-15);

        // Test hyperbolic identity: cosh²x - sinh²x = 1
        let x = 1.0;
        let sinh_result = math_sinh(vec![float_val(x)]).unwrap();
        let cosh_result = math_cosh(vec![float_val(x)]).unwrap();

        if let (Value::Float(sinh_val), Value::Float(cosh_val)) = (sinh_result, cosh_result) {
            let identity_result = cosh_val * cosh_val - sinh_val * sinh_val;
            assert_float_eq(&Value::Float(identity_result), 1.0, 1e-15);
        }
    }

    #[test]
    fn test_exponential_logarithmic() {
        // Test exp
        let result = math_exp(vec![float_val(1.0)]).unwrap();
        assert_float_eq(&result, consts::E, 1e-15);

        let result = math_exp(vec![float_val(0.0)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15);

        // Test exp2
        let result = math_exp2(vec![float_val(3.0)]).unwrap();
        assert_float_eq(&result, 8.0, 1e-15);

        // Test ln
        let result = math_ln(vec![float_val(consts::E)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15);

        let result = math_ln(vec![float_val(1.0)]).unwrap();
        assert_float_eq(&result, 0.0, 1e-15);

        // Test log with custom base
        let result = math_log(vec![float_val(100.0), float_val(10.0)]).unwrap();
        assert_float_eq(&result, 2.0, 1e-15);

        let result = math_log(vec![float_val(8.0), float_val(2.0)]).unwrap();
        assert_float_eq(&result, 3.0, 1e-15);

        // Test log2
        let result = math_log2(vec![float_val(8.0)]).unwrap();
        assert_float_eq(&result, 3.0, 1e-15);

        // Test log10
        let result = math_log10(vec![float_val(1000.0)]).unwrap();
        assert_float_eq(&result, 3.0, 1e-15);

        // Test domain errors for logarithms
        assert!(math_ln(vec![float_val(0.0)]).is_err());
        assert!(math_ln(vec![float_val(-1.0)]).is_err());
        assert!(math_log2(vec![float_val(0.0)]).is_err());
        assert!(math_log10(vec![float_val(-1.0)]).is_err());
        assert!(math_log(vec![float_val(10.0), float_val(0.0)]).is_err());
        assert!(math_log(vec![float_val(10.0), float_val(1.0)]).is_err());
        assert!(math_log(vec![float_val(10.0), float_val(-1.0)]).is_err());

        // Test exp/ln inverse relationship
        let x = 2.5;
        let exp_result = math_exp(vec![float_val(x)]).unwrap();
        if let Value::Float(exp_val) = exp_result {
            let ln_result = math_ln(vec![float_val(exp_val)]).unwrap();
            assert_float_eq(&ln_result, x, 1e-15);
        }
    }

    #[test]
    fn test_utility_functions() {
        // Test sign
        let result = math_sign(vec![float_val(5.0)]).unwrap();
        assert_int_eq(&result, 1);

        let result = math_sign(vec![float_val(-3.0)]).unwrap();
        assert_int_eq(&result, -1);

        let result = math_sign(vec![float_val(0.0)]).unwrap();
        assert_int_eq(&result, 0);

        // Test degrees/radians conversion
        let result = math_degrees(vec![float_val(consts::PI)]).unwrap();
        assert_float_eq(&result, 180.0, 1e-13);

        let result = math_radians(vec![float_val(180.0)]).unwrap();
        assert_float_eq(&result, consts::PI, 1e-15);

        // Test round-trip conversion
        let angle_deg = 45.0;
        let rad_result = math_radians(vec![float_val(angle_deg)]).unwrap();
        if let Value::Float(rad_val) = rad_result {
            let deg_result = math_degrees(vec![float_val(rad_val)]).unwrap();
            assert_float_eq(&deg_result, angle_deg, 1e-13);
        }
    }

    #[test]
    fn test_gcd() {
        let result = math_gcd(vec![int_val(12), int_val(8)]).unwrap();
        assert_int_eq(&result, 4);

        let result = math_gcd(vec![int_val(17), int_val(13)]).unwrap();
        assert_int_eq(&result, 1);

        let result = math_gcd(vec![int_val(0), int_val(5)]).unwrap();
        assert_int_eq(&result, 5);

        let result = math_gcd(vec![int_val(-12), int_val(8)]).unwrap();
        assert_int_eq(&result, 4);

        let result = math_gcd(vec![int_val(-12), int_val(-8)]).unwrap();
        assert_int_eq(&result, 4);

        // Test with large numbers
        let result = math_gcd(vec![int_val(1071), int_val(462)]).unwrap();
        assert_int_eq(&result, 21);

        // Test errors
        assert!(math_gcd(vec![float_val(1.5), int_val(2)]).is_err());
        assert!(math_gcd(vec![int_val(1)]).is_err());
    }

    #[test]
    fn test_lcm() {
        let result = math_lcm(vec![int_val(4), int_val(6)]).unwrap();
        assert_int_eq(&result, 12);

        let result = math_lcm(vec![int_val(3), int_val(5)]).unwrap();
        assert_int_eq(&result, 15);

        let result = math_lcm(vec![int_val(0), int_val(5)]).unwrap();
        assert_int_eq(&result, 0);

        let result = math_lcm(vec![int_val(12), int_val(0)]).unwrap();
        assert_int_eq(&result, 0);

        let result = math_lcm(vec![int_val(-4), int_val(6)]).unwrap();
        assert_int_eq(&result, 12);

        // Test relationship: gcd(a,b) * lcm(a,b) = |a * b|
        let a = 12;
        let b = 8;
        let gcd_result = math_gcd(vec![int_val(a), int_val(b)]).unwrap();
        let lcm_result = math_lcm(vec![int_val(a), int_val(b)]).unwrap();

        if let (Value::Integer(gcd_val), Value::Integer(lcm_val)) = (gcd_result, lcm_result) {
            assert_eq!(gcd_val * lcm_val, (a * b).abs());
        }

        // Test errors
        assert!(math_lcm(vec![float_val(1.5), int_val(2)]).is_err());
        assert!(math_lcm(vec![int_val(1)]).is_err());
    }

    #[test]
    fn test_factorial() {
        let result = math_factorial(vec![int_val(0)]).unwrap();
        assert_int_eq(&result, 1);

        let result = math_factorial(vec![int_val(1)]).unwrap();
        assert_int_eq(&result, 1);

        let result = math_factorial(vec![int_val(5)]).unwrap();
        assert_int_eq(&result, 120);

        let result = math_factorial(vec![int_val(10)]).unwrap();
        assert_int_eq(&result, 3628800);

        // Test maximum allowed value
        let result = math_factorial(vec![int_val(20)]).unwrap();
        assert_int_eq(&result, 2432902008176640000);

        // Test errors
        assert!(math_factorial(vec![int_val(-1)]).is_err());
        assert!(math_factorial(vec![int_val(21)]).is_err()); // Too large
        assert!(math_factorial(vec![float_val(5.5)]).is_err());
        assert!(math_factorial(vec![]).is_err());
        assert!(math_factorial(vec![int_val(1), int_val(2)]).is_err());
    }

    #[test]
    fn test_function_dispatcher() {
        // Test successful function calls
        let result = call_math_function("abs", vec![int_val(-5)]).unwrap();
        assert_int_eq(&result, 5);

        let result = call_math_function("sqrt", vec![int_val(16)]).unwrap();
        assert_float_eq(&result, 4.0, 1e-15);

        // Test unknown function
        assert!(call_math_function("unknown_func", vec![int_val(1)]).is_err());

        // Test with error conditions
        assert!(call_math_function("sqrt", vec![int_val(-1)]).is_err());
        assert!(call_math_function("ln", vec![int_val(0)]).is_err());
    }

    #[test]
    fn test_get_numeric() {
        assert_eq!(get_numeric(&int_val(42)).unwrap(), 42.0);
        assert_eq!(
            get_numeric(&float_val(std::f64::consts::PI)).unwrap(),
            std::f64::consts::PI
        );
        assert!(get_numeric(&string_val("hello")).is_err());
    }

    #[test]
    fn test_edge_cases_and_special_values() {
        // Test with infinity
        let result = math_abs(vec![float_val(f64::INFINITY)]).unwrap();
        assert_float_eq(&result, f64::INFINITY, 0.0);

        let result = math_abs(vec![float_val(f64::NEG_INFINITY)]).unwrap();
        assert_float_eq(&result, f64::INFINITY, 0.0);

        // Test exp with large values (should be very large or infinite)
        let result = math_exp(vec![float_val(700.0)]).unwrap();
        if let Value::Float(f) = result {
            assert!(
                f.is_infinite() || f > 1e100,
                "Expected very large or infinite value, got {}",
                f
            );
        }

        // Test very small values
        let result = math_ln(vec![float_val(f64::MIN_POSITIVE)]).unwrap();
        if let Value::Float(f) = result {
            assert!(f.is_finite());
        }

        // Test with zero
        let result = math_pow(vec![float_val(0.0), float_val(0.0)]).unwrap();
        assert_float_eq(&result, 1.0, 1e-15); // 0^0 = 1 by convention

        // Test sign with special values
        let result = math_sign(vec![float_val(f64::INFINITY)]).unwrap();
        assert_int_eq(&result, 1);

        let result = math_sign(vec![float_val(f64::NEG_INFINITY)]).unwrap();
        assert_int_eq(&result, -1);
    }

    #[test]
    fn test_mathematical_relationships() {
        // Test that pow(x, 1/n) and nth root are consistent
        let x = 27.0;
        let cube_root = math_cbrt(vec![float_val(x)]).unwrap();
        let pow_result = math_pow(vec![float_val(x), float_val(1.0 / 3.0)]).unwrap();

        if let (Value::Float(cbrt_val), Value::Float(pow_val)) = (cube_root, pow_result) {
            assert_float_eq(&Value::Float(cbrt_val), pow_val, 1e-14);
        }

        // Test that exp(ln(x)) = x for positive x
        let x = 5.0;
        let ln_result = math_ln(vec![float_val(x)]).unwrap();
        if let Value::Float(ln_val) = ln_result {
            let exp_result = math_exp(vec![float_val(ln_val)]).unwrap();
            assert_float_eq(&exp_result, x, 1e-15);
        }

        // Test that log_b(x) = ln(x) / ln(b)
        let x = 100.0;
        let base = 10.0;
        let log_result = math_log(vec![float_val(x), float_val(base)]).unwrap();
        let ln_x = math_ln(vec![float_val(x)]).unwrap();
        let ln_base = math_ln(vec![float_val(base)]).unwrap();

        if let (Value::Float(log_val), Value::Float(ln_x_val), Value::Float(ln_base_val)) =
            (log_result, ln_x, ln_base)
        {
            let manual_log = ln_x_val / ln_base_val;
            assert_float_eq(&Value::Float(log_val), manual_log, 1e-15);
        }
    }

    #[test]
    fn test_type_preservation() {
        // min/max should preserve integer type when both inputs are integers
        let result = math_min(vec![int_val(3), int_val(7)]).unwrap();
        assert!(matches!(result, Value::Integer(_)));

        let result = math_max(vec![int_val(3), int_val(7)]).unwrap();
        assert!(matches!(result, Value::Integer(_)));

        // But should promote to float if either input is float
        let result = math_min(vec![int_val(3), float_val(7.0)]).unwrap();
        assert!(matches!(result, Value::Float(_)));

        // abs should preserve type
        let result = math_abs(vec![int_val(-5)]).unwrap();
        assert!(matches!(result, Value::Integer(_)));

        let result = math_abs(vec![float_val(-5.0)]).unwrap();
        assert!(matches!(result, Value::Float(_)));

        // Most other functions should return floats
        let result = math_sqrt(vec![int_val(16)]).unwrap();
        assert!(matches!(result, Value::Float(_)));

        // Sign should return integer
        let result = math_sign(vec![float_val(5.5)]).unwrap();
        assert!(matches!(result, Value::Integer(_)));
    }
}

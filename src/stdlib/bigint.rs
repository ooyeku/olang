//! bigint — arbitrary-precision integers as first-class values.
//!
//! `typeof` says `BigInt`, display is the decimal digits, and the
//! `bigint` OvmModule below gives the value its operators on both
//! tiers: `+ - * / %` and the comparisons, with a plain Int operand
//! promoted to BigInt on contact. Division truncates toward zero and
//! the remainder takes the dividend's sign — exactly the Int rules, so
//! promoting a computation never changes its answers, only its range.
//!
//! Floats never mix implicitly: a Float has 53 bits of mantissa, so
//! `big * 0.5` would silently round the very digits the caller reached
//! for BigInt to keep. The operator refuses with a pointer to
//! `bigint.to_float`, and `bigint.of` likewise takes Int and String but
//! not Float.

use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use num_bigint::BigInt;
use num_traits::{Pow, Signed, ToPrimitive, Zero};
use std::collections::HashMap;
use std::sync::Arc;

/// A *misused* call — wrong arity, wrong argument type. Raised rather
/// than returned as `Value::Err`, per the stdlib conventions: a caller's
/// bug must abort, never come back as a handleable Result.
#[derive(Debug)]
struct Misuse(String);

impl std::fmt::Display for Misuse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Misuse {}

fn misuse(msg: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(Misuse(msg.into()))
}

#[derive(Debug)]
pub struct BigIntObject(pub BigInt);

impl NativeObject for BigIntObject {
    fn module(&self) -> &'static str {
        "bigint"
    }
    fn type_name(&self) -> &'static str {
        "BigInt"
    }
    fn display(&self) -> String {
        self.0.to_string()
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<BigIntObject>()
            .map(|o| self.0 == o.0)
            .unwrap_or(false)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Wrap a BigInt as an olang value.
pub fn bigint_value(b: BigInt) -> Value {
    Value::Native(NativeHandle::new(BigIntObject(b)))
}

fn as_bigint(v: &Value) -> Option<&BigInt> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<BigIntObject>().map(|b| &b.0),
        _ => None,
    }
}

/// A BigInt operand, or an Int promoted to one. `None` means the value
/// is neither, and the operator should decline.
fn as_bigint_operand(v: &Value) -> Option<BigInt> {
    match v {
        Value::Integer(i) => Some(BigInt::from(*i)),
        _ => as_bigint(v).cloned(),
    }
}

pub fn create_bigint_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [
        ("of", 1),
        ("parse", 1),
        ("to_int", 1),
        ("to_float", 1),
        ("abs", 1),
        ("neg", 1),
        ("pow", 2),
        ("mod_pow", 3),
        ("gcd", 2),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("bigint.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

/// One argument that must already be (or promote to) a BigInt.
fn want_big(func: &str, args: &[Value], at: usize) -> Result<BigInt, Box<dyn std::error::Error>> {
    match args.get(at).and_then(as_bigint_operand) {
        Some(b) => Ok(b),
        None => Err(misuse(format!(
            "{}: argument {} must be a BigInt or Int, got {}",
            func,
            at + 1,
            args.get(at)
                .map(|v| v.type_name())
                .unwrap_or_else(|| "nothing".to_string())
        ))),
    }
}

pub fn call_bigint_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let want_arity = |n: usize| -> Result<(), Box<dyn std::error::Error>> {
        if args.len() != n {
            return Err(misuse(format!(
                "{} expects {} argument{}, got {}",
                name,
                n,
                if n == 1 { "" } else { "s" },
                args.len()
            )));
        }
        Ok(())
    };
    match name {
        // The constructor: Int and digit-string in, BigInt out. A Float
        // is refused — its integer part may already have rounded — and
        // a malformed string *raises*, because `of` is for values the
        // caller controls; `parse` is the Result-returning twin for
        // data.
        "of" => {
            want_arity(1)?;
            match &args[0] {
                Value::Integer(i) => Ok(bigint_value(BigInt::from(*i))),
                Value::String(s) => match s.trim().parse::<BigInt>() {
                    Ok(b) => Ok(bigint_value(b)),
                    Err(_) => Err(misuse(format!(
                        "of: {:?} is not a decimal integer — for data you don't control, bigint.parse returns a Result instead of raising",
                        s.as_ref()
                    ))),
                },
                other if as_bigint(other).is_some() => Ok(other.clone()),
                Value::Float(_) => Err(misuse(
                    "of: a Float has 53 bits of precision and may already have rounded — convert from an Int or a digit string",
                )),
                other => Err(misuse(format!(
                    "of: argument must be an Int or a String of digits, got {}",
                    other.type_name()
                ))),
            }
        }
        "parse" => {
            want_arity(1)?;
            match &args[0] {
                Value::String(s) => Ok(match s.trim().parse::<BigInt>() {
                    Ok(b) => Value::Ok(Box::new(bigint_value(b))),
                    Err(_) => Value::Err(Box::new(Value::String(Arc::new(format!(
                        "bigint.parse: {:?} is not a decimal integer",
                        s.as_ref()
                    ))))),
                }),
                other => Err(misuse(format!(
                    "parse: argument must be a String, got {}",
                    other.type_name()
                ))),
            }
        }
        // The way back down. A Result, because whether the value fits
        // in 64 bits is a property of data, not of the call.
        "to_int" => {
            want_arity(1)?;
            let b = want_big("to_int", &args, 0)?;
            Ok(match b.to_i64() {
                Some(i) => Value::Ok(Box::new(Value::Integer(i))),
                None => Value::Err(Box::new(Value::String(Arc::new(format!(
                    "bigint.to_int: {} does not fit in a 64-bit Int",
                    b
                ))))),
            })
        }
        // Explicitly lossy, which is the point: the only sanctioned
        // door between BigInt and Float.
        "to_float" => {
            want_arity(1)?;
            let b = want_big("to_float", &args, 0)?;
            Ok(Value::Float(b.to_f64().unwrap_or(f64::INFINITY)))
        }
        "abs" => {
            want_arity(1)?;
            Ok(bigint_value(want_big("abs", &args, 0)?.abs()))
        }
        "neg" => {
            want_arity(1)?;
            Ok(bigint_value(-want_big("neg", &args, 0)?))
        }
        "pow" => {
            want_arity(2)?;
            let base = want_big("pow", &args, 0)?;
            let exp = match &args[1] {
                Value::Integer(e) if *e >= 0 => *e as u64,
                Value::Integer(_) => {
                    return Err(misuse(
                        "pow: a negative exponent leaves the integers — use bigint.to_float first",
                    ));
                }
                other => {
                    return Err(misuse(format!(
                        "pow: exponent must be a non-negative Int, got {}",
                        other.type_name()
                    )));
                }
            };
            Ok(bigint_value(Pow::pow(base, exp)))
        }
        // Modular exponentiation without materializing base^exp — the
        // whole reason it is a single builtin.
        "mod_pow" => {
            want_arity(3)?;
            let base = want_big("mod_pow", &args, 0)?;
            let exp = want_big("mod_pow", &args, 1)?;
            let modulus = want_big("mod_pow", &args, 2)?;
            if exp.is_negative() {
                return Err(misuse("mod_pow: exponent must be non-negative"));
            }
            if modulus.is_zero() {
                return Err(misuse("mod_pow: modulus must not be zero"));
            }
            Ok(bigint_value(base.modpow(&exp, &modulus)))
        }
        "gcd" => {
            want_arity(2)?;
            let mut a = want_big("gcd", &args, 0)?.abs();
            let mut b = want_big("gcd", &args, 1)?.abs();
            while !b.is_zero() {
                let r = &a % &b;
                a = b;
                b = r;
            }
            Ok(bigint_value(a))
        }
        _ => Err(misuse(format!("Unknown bigint function: {}", name))),
    }
}

/// The operator surface for BigInt values, shared by both tiers through
/// `native::binary_op_hook`. An Int operand promotes to BigInt; the
/// arithmetic then follows the Int rules exactly (truncating division,
/// dividend-signed remainder, the same zero-divisor errors), so widening
/// a computation changes its range and nothing else. A Float operand is
/// refused by name rather than declined, because the generic type error
/// could not say *why* the mix is disallowed.
pub struct BigIntModule;

impl crate::native::OvmModule for BigIntModule {
    fn name(&self) -> &'static str {
        "bigint"
    }
    // The `bigint` namespace is registered by the stdlib; this OvmModule
    // exists purely to give BigInt values operators.
    fn namespaces(&self) -> Vec<(String, Value)> {
        Vec::new()
    }
    fn dispatch(&self, func: &str, _args: Vec<Value>) -> Result<Value, String> {
        Err(format!(
            "bigint.{func} dispatches through the stdlib, not the native registry"
        ))
    }
    fn binary_op(
        &self,
        op: &crate::ast::BinaryOp,
        lhs: &Value,
        rhs: &Value,
    ) -> Option<Result<Value, String>> {
        use crate::ast::BinaryOp as B;
        // Only claim operations where at least one side is a BigInt.
        if as_bigint(lhs).is_none() && as_bigint(rhs).is_none() {
            return None;
        }
        if matches!(lhs, Value::Float(_)) || matches!(rhs, Value::Float(_)) {
            return Some(Err(
                "bigint: BigInt and Float do not mix implicitly — a Float has 53 bits of \
                 precision, so the product would silently round. Convert one side: \
                 bigint.to_float(b) to accept the rounding, or bigint.of(i) to stay exact"
                    .to_string(),
            ));
        }
        let (a, b) = match (as_bigint_operand(lhs), as_bigint_operand(rhs)) {
            (Some(a), Some(b)) => (a, b),
            _ => return None,
        };
        Some(match op {
            B::Add => Ok(bigint_value(a + b)),
            B::Subtract => Ok(bigint_value(a - b)),
            B::Multiply => Ok(bigint_value(a * b)),
            B::Divide => {
                if b.is_zero() {
                    Err("Division by zero".to_string())
                } else {
                    Ok(bigint_value(a / b))
                }
            }
            B::Modulo => {
                if b.is_zero() {
                    Err("Modulo by zero".to_string())
                } else {
                    Ok(bigint_value(a % b))
                }
            }
            B::Equal => Ok(Value::Boolean(a == b)),
            B::NotEqual => Ok(Value::Boolean(a != b)),
            B::LessThan => Ok(Value::Boolean(a < b)),
            B::LessThanEqual => Ok(Value::Boolean(a <= b)),
            B::GreaterThan => Ok(Value::Boolean(a > b)),
            B::GreaterThanEqual => Ok(Value::Boolean(a >= b)),
            B::And | B::Or => return None,
        })
    }
}

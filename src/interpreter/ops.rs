//! Operator semantics: binary and unary operations, truthiness, and range
//! construction. The single source of truth the OVM must agree with.

use super::{Interpreter, InterpreterError};
use crate::ast::{BinaryOp, UnaryOp, Value};

/// A float result must be finite: overflow raises, like every other
/// operation whose IEEE result would be `inf` or `NaN`.
fn float_result(r: f64, op: &str) -> Result<Value, InterpreterError> {
    if r.is_finite() {
        Ok(Value::Float(r))
    } else {
        Err(InterpreterError::RuntimeError {
            message: crate::ast::float_overflow_message(op),
        })
    }
}

impl Interpreter {
    pub(crate) fn eval_binary_op(
        &self,
        left: Value,
        op: BinaryOp,
        right: Value,
    ) -> Result<Value, InterpreterError> {
        // `??` with both sides already evaluated (the evaluator itself
        // evaluates the right only when the left is `()`).
        if matches!(op, BinaryOp::Coalesce) {
            return Ok(if matches!(left, Value::Unit) {
                right
            } else {
                left
            });
        }
        // Native extension values (OVM modules) get first refusal on any
        // operation touching them; no arm below can apply to one. The VM's
        // execute_binary_op calls the same hook on the same Arc-shared
        // value, which is what keeps the tiers observationally identical.
        if (matches!(left, Value::Native(_)) || matches!(right, Value::Native(_)))
            && let Some(result) = crate::native::binary_op_hook(&op, &left, &right)
        {
            return result.map_err(|message| InterpreterError::RuntimeError { message });
        }
        // `x == ()` / `x != ()` is the presence test and is total (0.68):
        // Unit is the absence value — what a missing map key, an absent
        // substring, and a JSON null all produce — so asking "is this
        // nothing?" must be answerable about every value, not only about
        // values that happen to be absent. Equality between two present
        // but unrelated kinds remains a type error below; ordering or
        // arithmetic against Unit falls through to the type error too.
        if matches!(left, Value::Unit) || matches!(right, Value::Unit) {
            let both_unit = matches!(left, Value::Unit) && matches!(right, Value::Unit);
            match op {
                BinaryOp::Equal => return Ok(Value::Boolean(both_unit)),
                BinaryOp::NotEqual => return Ok(Value::Boolean(!both_unit)),
                _ => {}
            }
        }
        // A list sum whose left side nothing else holds (a call's result:
        // `cell.take(c) + [v]`, `f(x) + ys`) extends it in place; a shared
        // one is copied once, as before.
        if matches!(op, BinaryOp::Add)
            && matches!(left, Value::List(_))
            && matches!(right, Value::List(_))
        {
            let mut items = left.into_items().expect("a list");
            if let Value::List(b) = &right {
                items.extend(b.iter().cloned());
            }
            return Ok(Value::List(std::sync::Arc::new(items)));
        }
        match (left, op, right) {
            (Value::Integer(a), BinaryOp::Add, Value::Integer(b)) => a
                .checked_add(b)
                .map(Value::Integer)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: "Integer overflow in addition (bigint.of gives arbitrary precision)"
                        .to_string(),
                }),
            (Value::Float(a), BinaryOp::Add, Value::Float(b)) => float_result(a + b, "addition"),
            (Value::Integer(a), BinaryOp::Add, Value::Float(b)) => {
                float_result(a as f64 + b, "addition")
            }
            (Value::Float(a), BinaryOp::Add, Value::Integer(b)) => {
                float_result(a + b as f64, "addition")
            }
            (Value::Integer(a), BinaryOp::Subtract, Value::Integer(b)) => a
                .checked_sub(b)
                .map(Value::Integer)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message:
                        "Integer overflow in subtraction (bigint.of gives arbitrary precision)"
                            .to_string(),
                }),
            (Value::Float(a), BinaryOp::Subtract, Value::Float(b)) => {
                float_result(a - b, "subtraction")
            }
            (Value::Integer(a), BinaryOp::Subtract, Value::Float(b)) => {
                float_result(a as f64 - b, "subtraction")
            }
            (Value::Float(a), BinaryOp::Subtract, Value::Integer(b)) => {
                float_result(a - b as f64, "subtraction")
            }
            (Value::Integer(a), BinaryOp::Multiply, Value::Integer(b)) => a
                .checked_mul(b)
                .map(Value::Integer)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message:
                        "Integer overflow in multiplication (bigint.of gives arbitrary precision)"
                            .to_string(),
                }),
            (Value::Float(a), BinaryOp::Multiply, Value::Float(b)) => {
                float_result(a * b, "multiplication")
            }
            (Value::Integer(a), BinaryOp::Multiply, Value::Float(b)) => {
                float_result(a as f64 * b, "multiplication")
            }
            (Value::Float(a), BinaryOp::Multiply, Value::Integer(b)) => {
                float_result(a * b as f64, "multiplication")
            }
            (Value::Integer(a), BinaryOp::Divide, Value::Integer(b)) => {
                if b == 0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    // checked_div also rejects i64::MIN / -1, which overflows
                    a.checked_div(b).map(Value::Integer).ok_or_else(|| {
                        InterpreterError::RuntimeError {
                            message:
                                "Integer overflow in division (bigint.of gives arbitrary precision)"
                                    .to_string(),
                        }
                    })
                }
            }
            (Value::Float(a), BinaryOp::Divide, Value::Float(b)) => {
                if b == 0.0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    float_result(a / b, "division")
                }
            }
            (Value::Integer(a), BinaryOp::Divide, Value::Float(b)) => {
                if b == 0.0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    float_result(a as f64 / b, "division")
                }
            }
            (Value::Float(a), BinaryOp::Divide, Value::Integer(b)) => {
                if b == 0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    float_result(a / b as f64, "division")
                }
            }
            (Value::Integer(a), BinaryOp::Modulo, Value::Integer(b)) => {
                if b == 0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Modulo by zero".to_string(),
                    })
                } else {
                    // checked_rem also rejects i64::MIN % -1, which overflows
                    a.checked_rem(b).map(Value::Integer).ok_or_else(|| {
                        InterpreterError::RuntimeError {
                            message:
                                "Integer overflow in modulo (bigint.of gives arbitrary precision)"
                                    .to_string(),
                        }
                    })
                }
            }
            (Value::Float(a), BinaryOp::Modulo, Value::Float(b)) => {
                if b == 0.0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Modulo by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a % b))
                }
            }
            (Value::Integer(a), BinaryOp::Modulo, Value::Float(b)) => {
                if b == 0.0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Modulo by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a as f64 % b))
                }
            }
            (Value::Float(a), BinaryOp::Modulo, Value::Integer(b)) => {
                if b == 0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Modulo by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a % b as f64))
                }
            }
            (Value::Integer(a), BinaryOp::Equal, Value::Integer(b)) => Ok(Value::Boolean(a == b)),
            (Value::Float(a), BinaryOp::Equal, Value::Float(b)) => Ok(Value::Boolean(a == b)),
            (Value::String(ref a), BinaryOp::Equal, Value::String(ref b)) => {
                Ok(Value::Boolean(*a == *b))
            }
            (Value::Boolean(a), BinaryOp::Equal, Value::Boolean(b)) => Ok(Value::Boolean(a == b)),
            // A container holding a function, a builtin or a range has no
            // equality, as they have none at the top level; a Result
            // compares as its payload does.
            (
                ref left @ (Value::Enum(_)
                | Value::List(_)
                | Value::Tuple(_)
                | Value::Map(_)
                | Value::Struct { .. }
                | Value::Ok(_)
                | Value::Err(_)),
                BinaryOp::Equal | BinaryOp::NotEqual,
                ref right @ (Value::Enum(_)
                | Value::List(_)
                | Value::Tuple(_)
                | Value::Map(_)
                | Value::Struct { .. }
                | Value::Ok(_)
                | Value::Err(_)),
            ) if !same_collection(left, right)
                && let Some(message) = uncomparable_error(left, right) =>
            {
                Err(InterpreterError::TypeError { message })
            }
            // Enum values compare structurally: same variant and payloads.
            (left @ Value::Enum(_), BinaryOp::Equal, right @ Value::Enum(_)) => {
                Ok(Value::Boolean(loose_eq(&left, &right)))
            }
            (left @ Value::Enum(_), BinaryOp::NotEqual, right @ Value::Enum(_)) => {
                Ok(Value::Boolean(!loose_eq(&left, &right)))
            }
            // A Result compares structurally, its kind and payload.
            (
                left @ (Value::Ok(_) | Value::Err(_)),
                BinaryOp::Equal,
                right @ (Value::Ok(_) | Value::Err(_)),
            ) => Ok(Value::Boolean(loose_eq(&left, &right))),
            (
                left @ (Value::Ok(_) | Value::Err(_)),
                BinaryOp::NotEqual,
                right @ (Value::Ok(_) | Value::Err(_)),
            ) => Ok(Value::Boolean(!loose_eq(&left, &right))),
            // Collections and structs compare structurally, like enums:
            // same shape, equal elements. Value's derived PartialEq is the
            // natural recursive equality.
            (left @ Value::List(_), BinaryOp::Equal, right @ Value::List(_)) => {
                Ok(Value::Boolean(loose_eq(&left, &right)))
            }
            (left @ Value::List(_), BinaryOp::NotEqual, right @ Value::List(_)) => {
                Ok(Value::Boolean(!loose_eq(&left, &right)))
            }
            (left @ Value::Tuple(_), BinaryOp::Equal, right @ Value::Tuple(_)) => {
                Ok(Value::Boolean(loose_eq(&left, &right)))
            }
            (left @ Value::Tuple(_), BinaryOp::NotEqual, right @ Value::Tuple(_)) => {
                Ok(Value::Boolean(!loose_eq(&left, &right)))
            }
            // The map kinds compare by contents with one another: a parsed
            // JSON object, an anonymous record, and a `#{}` map holding the
            // same keys and values are equal, as every map function already
            // treats them alike. `typeof` keeps telling them apart.
            (
                left @ (Value::Map(_) | Value::Struct { .. }),
                BinaryOp::Equal,
                right @ (Value::Map(_) | Value::Struct { .. }),
            ) => Ok(Value::Boolean(loose_eq(&left, &right))),
            (
                left @ (Value::Map(_) | Value::Struct { .. }),
                BinaryOp::NotEqual,
                right @ (Value::Map(_) | Value::Struct { .. }),
            ) => Ok(Value::Boolean(!loose_eq(&left, &right))),
            (Value::Unit, BinaryOp::Equal, Value::Unit) => Ok(Value::Boolean(true)),
            (Value::Unit, BinaryOp::NotEqual, Value::Unit) => Ok(Value::Boolean(false)),
            (Value::Integer(a), BinaryOp::NotEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a != b))
            }
            (Value::Float(a), BinaryOp::NotEqual, Value::Float(b)) => Ok(Value::Boolean(a != b)),
            (Value::String(ref a), BinaryOp::NotEqual, Value::String(ref b)) => {
                Ok(Value::Boolean(*a != *b))
            }
            // Tuples order lexicographically (`tuple_ordering`), as a sort
            // by several keys needs: `(a.year, a.name) < (b.year, b.name)`.
            (
                Value::Tuple(ref a),
                op @ (BinaryOp::LessThan
                | BinaryOp::LessThanEqual
                | BinaryOp::GreaterThan
                | BinaryOp::GreaterThanEqual),
                Value::Tuple(ref b),
            ) => {
                let ord = tuple_ordering(a, b)?;
                Ok(Value::Boolean(match op {
                    BinaryOp::LessThan => ord.is_lt(),
                    BinaryOp::LessThanEqual => ord.is_le(),
                    BinaryOp::GreaterThan => ord.is_gt(),
                    _ => ord.is_ge(),
                }))
            }
            // Strings order lexicographically, matching the bytecode tier — so
            // character-range checks like `c >= "0" && c <= "9"` work and
            // strings sort. Ordering is by Unicode scalar value.
            (Value::String(ref a), BinaryOp::LessThan, Value::String(ref b)) => {
                Ok(Value::Boolean(*a < *b))
            }
            (Value::String(ref a), BinaryOp::LessThanEqual, Value::String(ref b)) => {
                Ok(Value::Boolean(*a <= *b))
            }
            (Value::String(ref a), BinaryOp::GreaterThan, Value::String(ref b)) => {
                Ok(Value::Boolean(*a > *b))
            }
            (Value::String(ref a), BinaryOp::GreaterThanEqual, Value::String(ref b)) => {
                Ok(Value::Boolean(*a >= *b))
            }
            (Value::Boolean(a), BinaryOp::NotEqual, Value::Boolean(b)) => {
                Ok(Value::Boolean(a != b))
            }
            (Value::Integer(a), BinaryOp::Equal, Value::Float(b)) => {
                Ok(Value::Boolean(int_float_eq(a, b)))
            }
            (Value::Float(a), BinaryOp::Equal, Value::Integer(b)) => {
                Ok(Value::Boolean(int_float_eq(b, a)))
            }
            (Value::Integer(a), BinaryOp::NotEqual, Value::Float(b)) => {
                Ok(Value::Boolean(!int_float_eq(a, b)))
            }
            (Value::Float(a), BinaryOp::NotEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(!int_float_eq(b, a)))
            }
            (Value::Integer(a), BinaryOp::LessThan, Value::Integer(b)) => Ok(Value::Boolean(a < b)),
            (Value::Float(a), BinaryOp::LessThan, Value::Float(b)) => Ok(Value::Boolean(a < b)),
            (Value::Integer(a), BinaryOp::LessThan, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) < b))
            }
            (Value::Float(a), BinaryOp::LessThan, Value::Integer(b)) => {
                Ok(Value::Boolean(a < b as f64))
            }
            (Value::Integer(a), BinaryOp::LessThanEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a <= b))
            }
            (Value::Float(a), BinaryOp::LessThanEqual, Value::Float(b)) => {
                Ok(Value::Boolean(a <= b))
            }
            (Value::Integer(a), BinaryOp::LessThanEqual, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) <= b))
            }
            (Value::Float(a), BinaryOp::LessThanEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a <= b as f64))
            }
            (Value::Integer(a), BinaryOp::GreaterThan, Value::Integer(b)) => {
                Ok(Value::Boolean(a > b))
            }
            (Value::Float(a), BinaryOp::GreaterThan, Value::Float(b)) => Ok(Value::Boolean(a > b)),
            (Value::Integer(a), BinaryOp::GreaterThan, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) > b))
            }
            (Value::Float(a), BinaryOp::GreaterThan, Value::Integer(b)) => {
                Ok(Value::Boolean(a > b as f64))
            }
            (Value::Integer(a), BinaryOp::GreaterThanEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a >= b))
            }
            (Value::Float(a), BinaryOp::GreaterThanEqual, Value::Float(b)) => {
                Ok(Value::Boolean(a >= b))
            }
            (Value::Integer(a), BinaryOp::GreaterThanEqual, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) >= b))
            }
            (Value::Float(a), BinaryOp::GreaterThanEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a >= b as f64))
            }
            (Value::Boolean(a), BinaryOp::And, Value::Boolean(b)) => Ok(Value::Boolean(a && b)),
            (Value::Boolean(a), BinaryOp::Or, Value::Boolean(b)) => Ok(Value::Boolean(a || b)),
            (Value::String(ref a), BinaryOp::Add, Value::String(ref b)) => {
                let mut s = String::clone(a);
                s.push_str(b);
                Ok(Value::String(std::sync::Arc::new(s)))
            }
            // Mixing a number and a string under `+` is a type error, not a
            // silent stringify (Python-3 style). Concatenation stays
            // string+string; convert the number with to_string(...) first.
            (Value::String(_), BinaryOp::Add, Value::Integer(_)) => {
                Err(InterpreterError::TypeError {
                    message: "cannot add String and Int; use to_string(...) to convert".to_string(),
                })
            }
            (Value::Integer(_), BinaryOp::Add, Value::String(_)) => {
                Err(InterpreterError::TypeError {
                    message: "cannot add Int and String; use to_string(...) to convert".to_string(),
                })
            }
            (Value::String(_), BinaryOp::Add, Value::Float(_)) => {
                Err(InterpreterError::TypeError {
                    message: "cannot add String and Float; use to_string(...) to convert"
                        .to_string(),
                })
            }
            (Value::Float(_), BinaryOp::Add, Value::String(_)) => {
                Err(InterpreterError::TypeError {
                    message: "cannot add Float and String; use to_string(...) to convert"
                        .to_string(),
                })
            }
            (Value::List(ref a), BinaryOp::Add, Value::List(ref b)) => {
                let mut items = Vec::with_capacity(a.len() + b.len());
                items.extend(a.iter().cloned());
                items.extend(b.iter().cloned());
                Ok(Value::List(std::sync::Arc::from(items)))
            }
            // Equality across types is an answer, not an error: a String is
            // never the Boolean true, an Int never a String. (Int and Float
            // compare numerically above.) A request field that arrived as
            // the wrong type used to end the handler here.
            (l, BinaryOp::Equal, r) if l.type_name() != r.type_name() => Ok(Value::Boolean(false)),
            (l, BinaryOp::NotEqual, r) if l.type_name() != r.type_name() => {
                Ok(Value::Boolean(true))
            }
            (l, op, r) => Err(InterpreterError::TypeError {
                message: format!(
                    "Invalid binary operation: cannot apply '{}' to {} and {}",
                    op.symbol(),
                    l.type_name(),
                    r.type_name()
                ),
            }),
        }
    }

    pub(crate) fn eval_unary_op(
        &self,
        op: UnaryOp,
        operand: Value,
    ) -> Result<Value, InterpreterError> {
        match (op, operand) {
            (UnaryOp::Negate, Value::Integer(n)) => {
                n.checked_neg()
                    .map(Value::Integer)
                    .ok_or_else(|| InterpreterError::RuntimeError {
                        message:
                            "Integer overflow in negation (bigint.of gives arbitrary precision)"
                                .to_string(),
                    })
            }
            (UnaryOp::Negate, Value::Float(x)) => Ok(Value::Float(-x)),
            (UnaryOp::Not, Value::Boolean(b)) => Ok(Value::Boolean(!b)),
            (op, operand) => Err(InterpreterError::TypeError {
                message: format!(
                    "Invalid unary operation: cannot apply '{}' to {}",
                    op.symbol(),
                    operand.type_name()
                ),
            }),
        }
    }

    /// A condition's answer (`if`, `while`, a match guard): a Bool, as
    /// `&&`, `||` and `!` take. Any other value raises — an empty list
    /// or `()` is not false; say what is meant (`len(xs) > 0`, `x != ()`).
    pub(crate) fn to_boolean(&self, value: &Value) -> Result<bool, InterpreterError> {
        match value {
            Value::Boolean(b) => Ok(*b),
            other => Err(InterpreterError::TypeError {
                message: condition_error(&other.type_name()),
            }),
        }
    }

    pub(crate) fn eval_range(
        &self,
        start: Value,
        end: Value,
        inclusive: bool,
    ) -> Result<Value, InterpreterError> {
        let start_int = match start {
            Value::Integer(n) => n,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "Range start must be an integer".to_string(),
                });
            }
        };

        let end_int = match end {
            Value::Integer(n) => n,
            _ => {
                return Err(InterpreterError::TypeError {
                    message: "Range end must be an integer".to_string(),
                });
            }
        };

        // Return a proper Range value instead of expanding to a list
        Ok(Value::Range {
            start: start_int,
            end: end_int,
            inclusive,
        })
    }
}

/// The map kinds a value may be compared as: a `#{}` map, a parsed JSON
/// object, or an anonymous `{ ... }` record. A declared struct is not one:
/// its name is part of its identity.
fn map_like(v: &Value) -> Option<&crate::ast::ValueMap> {
    match v {
        Value::Map(m) => Some(m),
        Value::Struct { type_name, fields }
            if type_name == "JsonObject" || type_name == "Object" =>
        {
            Some(fields)
        }
        _ => None,
    }
}

/// What `assert_eq` asks: equal as `==` says — the map kinds alike (a
/// parsed JSON object equals the `#{}` map with its keys and values), Int
/// and Float numerically at the top — and total, where `==` refuses some
/// kinds: a Result is compared through its payload, so
/// `assert_eq(json.parse(s), Ok(#{ "a": 1 }))` holds too.
pub fn assert_eq_holds(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Integer(x), Value::Float(y)) => (*x as f64) == *y,
        (Value::Float(x), Value::Integer(y)) => *x == (*y as f64),
        _ => deep_eq(a, b),
    }
}

/// `loose_eq` carried through Results, at every depth.
fn deep_eq(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    if let (Some(x), Some(y)) = (map_like(a), map_like(b)) {
        return x.len() == y.len()
            && x.iter()
                .all(|(k, v)| y.get(k).is_some_and(|w| deep_eq(v, w)));
    }
    match (a, b) {
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(v, w)| deep_eq(v, w))
        }
        (Value::Tuple(x), Value::Tuple(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(v, w)| deep_eq(v, w))
        }
        (
            Value::Struct {
                type_name: tn,
                fields: fx,
            },
            Value::Struct {
                type_name: tm,
                fields: fy,
            },
        ) => {
            tn == tm
                && fx.len() == fy.len()
                && fx
                    .iter()
                    .all(|(k, v)| fy.get(k).is_some_and(|w| deep_eq(v, w)))
        }
        (Value::Ok(x), Value::Ok(y)) | (Value::Err(x), Value::Err(y)) => deep_eq(x, y),
        _ => false,
    }
}

/// Structural equality that treats the map kinds alike and compares Int
/// with Float numerically, at every depth. What `==` means for maps,
/// lists, tuples, and records on both tiers.
/// The order `<` and `sort` give two tuples: element by element, the
/// first unequal pair deciding by `<`'s own rule for it (numbers with
/// numbers, strings with strings, tuples again), a tuple that runs out
/// first being the smaller. Equal elements need no order of their own, so
/// `(true, 1) < (true, 2)`; an unequal pair `<` cannot order is the error
/// `<` raises for it.
pub fn tuple_ordering(a: &[Value], b: &[Value]) -> Result<std::cmp::Ordering, InterpreterError> {
    for (x, y) in a.iter().zip(b.iter()) {
        if loose_eq(x, y) {
            continue;
        }
        return value_ordering(x, y);
    }
    Ok(a.len().cmp(&b.len()))
}

/// `<`'s order of two values it can order, for a tuple's elements.
fn value_ordering(x: &Value, y: &Value) -> Result<std::cmp::Ordering, InterpreterError> {
    use std::cmp::Ordering;
    let num = |v: &Value| match v {
        Value::Integer(n) => Some(*n as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    };
    match (x, y) {
        (Value::Integer(a), Value::Integer(b)) => Ok(a.cmp(b)),
        (Value::String(a), Value::String(b)) => Ok(a.cmp(b)),
        (Value::Tuple(a), Value::Tuple(b)) => tuple_ordering(a, b),
        _ => match (num(x), num(y)) {
            (Some(a), Some(b)) => Ok(a.partial_cmp(&b).unwrap_or(Ordering::Equal)),
            _ => Err(InterpreterError::TypeError {
                message: format!(
                    "cannot order {} and {} inside a tuple: `<` orders numbers, strings and tuples",
                    x.type_name(),
                    y.type_name()
                ),
            }),
        },
    }
}

/// What a condition that is not a Bool raises, on every tier.
pub fn condition_error(type_name: &str) -> String {
    format!(
        "a condition must be a Bool, got {}: say what is meant (`len(xs) > 0`, \
         `x != ()`, `s != \"\"`)",
        type_name
    )
}

/// The first value `==` cannot compare that `v` holds, at any depth: a
/// function, a builtin or a range compares with nothing at the top
/// level, and a container holding one does not compare either.
pub fn uncomparable(v: &Value) -> Option<&'static str> {
    match v {
        Value::Function(_) => Some("Function"),
        Value::Builtin(_) => Some("Builtin"),
        Value::Range { .. } => Some("Range"),
        Value::List(items) | Value::Tuple(items) => items.iter().find_map(uncomparable),
        Value::Map(m) => m.values().find_map(uncomparable),
        Value::Struct { fields, .. } => fields.values().find_map(uncomparable),
        Value::Enum(e) => match &e.variant_data {
            crate::ast::EnumVariantData::Unit => None,
            crate::ast::EnumVariantData::Tuple(items) => items.iter().find_map(uncomparable),
            crate::ast::EnumVariantData::Struct(fields) => fields.values().find_map(uncomparable),
        },
        Value::Ok(inner) | Value::Err(inner) => uncomparable(inner),
        _ => None,
    }
}

/// The error `==` raises on a pair holding a value it cannot compare.
pub fn uncomparable_error(a: &Value, b: &Value) -> Option<String> {
    uncomparable(a).or_else(|| uncomparable(b)).map(|kind| {
        format!(
            "cannot compare with ==: a {} has no equality, and a list, map or record \
             holding one has none either; compare the parts that hold data",
            kind
        )
    })
}

/// `a == b` as the operator answers it, for a builtin that searches
/// (`contains`, `col.index_of`): an Int and a Float equal when exactly
/// so, `()` equal only to `()`, a function, builtin or range (or a
/// container holding one) refused, everything else structurally.
pub fn search_eq(a: &Value, b: &Value) -> Result<bool, String> {
    match (a, b) {
        (Value::Integer(i), Value::Float(f)) | (Value::Float(f), Value::Integer(i)) => {
            return Ok(int_float_eq(*i, *f));
        }
        (Value::Unit, other) | (other, Value::Unit) => return Ok(matches!(other, Value::Unit)),
        _ => {}
    }
    match uncomparable_error(a, b) {
        Some(message) => Err(message),
        None => Ok(loose_eq(a, b)),
    }
}

/// `i == f` exactly: the Float is that integer, not merely the f64
/// nearest it (`9007199254740993 == 9007199254740992.0` is false).
pub fn int_float_eq(i: i64, f: f64) -> bool {
    f.fract() == 0.0
        && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&f)
        && f as i64 == i
}

/// The same list, tuple or map, by address: equal to itself without a
/// walk (and without the uncomparable scan, as before it).
fn same_collection(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
            std::sync::Arc::ptr_eq(x, y)
        }
        (Value::Map(x), Value::Map(y)) => std::sync::Arc::ptr_eq(x, y),
        _ => false,
    }
}

pub fn loose_eq(a: &Value, b: &Value) -> bool {
    // A collection is equal to itself without a walk.
    match (a, b) {
        (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y))
            if std::sync::Arc::ptr_eq(x, y) =>
        {
            return true;
        }
        (Value::Map(x), Value::Map(y)) if std::sync::Arc::ptr_eq(x, y) => return true,
        _ => {}
    }
    if let (Some(x), Some(y)) = (map_like(a), map_like(b)) {
        return x.len() == y.len()
            && x.iter()
                .all(|(k, v)| y.get(k).is_some_and(|w| loose_eq(v, w)));
    }
    match (a, b) {
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(v, w)| loose_eq(v, w))
        }
        (Value::Tuple(x), Value::Tuple(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(v, w)| loose_eq(v, w))
        }
        // Inside a collection an Int and a Float stay distinct ([1] and
        // [1.0] are different lists), as they always were; only the
        // top-level `1 == 1.0` compares numerically, in the Equal arms.
        (
            Value::Struct {
                type_name: tn,
                fields: fx,
            },
            Value::Struct {
                type_name: tm,
                fields: fy,
            },
        ) => {
            tn == tm
                && fx.len() == fy.len()
                && fx
                    .iter()
                    .all(|(k, v)| fy.get(k).is_some_and(|w| loose_eq(v, w)))
        }
        (Value::Ok(x), Value::Ok(y)) | (Value::Err(x), Value::Err(y)) => loose_eq(x, y),
        (Value::Enum(x), Value::Enum(y)) => {
            x.type_name == y.type_name
                && x.variant_name == y.variant_name
                && match (&x.variant_data, &y.variant_data) {
                    (
                        crate::ast::EnumVariantData::Tuple(p),
                        crate::ast::EnumVariantData::Tuple(q),
                    ) => p.len() == q.len() && p.iter().zip(q.iter()).all(|(v, w)| loose_eq(v, w)),
                    (p, q) => p == q,
                }
        }
        _ => a == b,
    }
}

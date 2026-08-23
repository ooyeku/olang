//! Operator semantics: binary and unary operations, truthiness, and range
//! construction. The single source of truth the OVM must agree with.

use super::{Interpreter, InterpreterError};
use crate::ast::{BinaryOp, UnaryOp, Value};

impl Interpreter {
    pub(crate) fn eval_binary_op(
        &self,
        left: Value,
        op: BinaryOp,
        right: Value,
    ) -> Result<Value, InterpreterError> {
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
        match (left, op, right) {
            (Value::Integer(a), BinaryOp::Add, Value::Integer(b)) => a
                .checked_add(b)
                .map(Value::Integer)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message: "Integer overflow in addition (bigint.of gives arbitrary precision)"
                        .to_string(),
                }),
            (Value::Float(a), BinaryOp::Add, Value::Float(b)) => Ok(Value::Float(a + b)),
            (Value::Integer(a), BinaryOp::Add, Value::Float(b)) => Ok(Value::Float(a as f64 + b)),
            (Value::Float(a), BinaryOp::Add, Value::Integer(b)) => Ok(Value::Float(a + b as f64)),
            (Value::Integer(a), BinaryOp::Subtract, Value::Integer(b)) => a
                .checked_sub(b)
                .map(Value::Integer)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message:
                        "Integer overflow in subtraction (bigint.of gives arbitrary precision)"
                            .to_string(),
                }),
            (Value::Float(a), BinaryOp::Subtract, Value::Float(b)) => Ok(Value::Float(a - b)),
            (Value::Integer(a), BinaryOp::Subtract, Value::Float(b)) => {
                Ok(Value::Float(a as f64 - b))
            }
            (Value::Float(a), BinaryOp::Subtract, Value::Integer(b)) => {
                Ok(Value::Float(a - b as f64))
            }
            (Value::Integer(a), BinaryOp::Multiply, Value::Integer(b)) => a
                .checked_mul(b)
                .map(Value::Integer)
                .ok_or_else(|| InterpreterError::RuntimeError {
                    message:
                        "Integer overflow in multiplication (bigint.of gives arbitrary precision)"
                            .to_string(),
                }),
            (Value::Float(a), BinaryOp::Multiply, Value::Float(b)) => Ok(Value::Float(a * b)),
            (Value::Integer(a), BinaryOp::Multiply, Value::Float(b)) => {
                Ok(Value::Float(a as f64 * b))
            }
            (Value::Float(a), BinaryOp::Multiply, Value::Integer(b)) => {
                Ok(Value::Float(a * b as f64))
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
                    Ok(Value::Float(a / b))
                }
            }
            (Value::Integer(a), BinaryOp::Divide, Value::Float(b)) => {
                if b == 0.0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a as f64 / b))
                }
            }
            (Value::Float(a), BinaryOp::Divide, Value::Integer(b)) => {
                if b == 0 {
                    Err(InterpreterError::RuntimeError {
                        message: "Division by zero".to_string(),
                    })
                } else {
                    Ok(Value::Float(a / b as f64))
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
            (Value::String(a), BinaryOp::Equal, Value::String(b)) => Ok(Value::Boolean(*a == *b)),
            (Value::Boolean(a), BinaryOp::Equal, Value::Boolean(b)) => Ok(Value::Boolean(a == b)),
            // Enum values compare structurally: same variant and payloads.
            // Value derives PartialEq, so this is the natural equality.
            (left @ Value::Enum { .. }, BinaryOp::Equal, right @ Value::Enum { .. }) => {
                Ok(Value::Boolean(left == right))
            }
            (left @ Value::Enum { .. }, BinaryOp::NotEqual, right @ Value::Enum { .. }) => {
                Ok(Value::Boolean(left != right))
            }
            // Collections and structs compare structurally, like enums:
            // same shape, equal elements. Value's derived PartialEq is the
            // natural recursive equality.
            (left @ Value::List(_), BinaryOp::Equal, right @ Value::List(_)) => {
                Ok(Value::Boolean(left == right))
            }
            (left @ Value::List(_), BinaryOp::NotEqual, right @ Value::List(_)) => {
                Ok(Value::Boolean(left != right))
            }
            (left @ Value::Tuple(_), BinaryOp::Equal, right @ Value::Tuple(_)) => {
                Ok(Value::Boolean(left == right))
            }
            (left @ Value::Tuple(_), BinaryOp::NotEqual, right @ Value::Tuple(_)) => {
                Ok(Value::Boolean(left != right))
            }
            (left @ Value::Map(_), BinaryOp::Equal, right @ Value::Map(_)) => {
                Ok(Value::Boolean(left == right))
            }
            (left @ Value::Map(_), BinaryOp::NotEqual, right @ Value::Map(_)) => {
                Ok(Value::Boolean(left != right))
            }
            (left @ Value::Struct { .. }, BinaryOp::Equal, right @ Value::Struct { .. }) => {
                Ok(Value::Boolean(left == right))
            }
            (left @ Value::Struct { .. }, BinaryOp::NotEqual, right @ Value::Struct { .. }) => {
                Ok(Value::Boolean(left != right))
            }
            (Value::Unit, BinaryOp::Equal, Value::Unit) => Ok(Value::Boolean(true)),
            (Value::Unit, BinaryOp::NotEqual, Value::Unit) => Ok(Value::Boolean(false)),
            (Value::Integer(a), BinaryOp::NotEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a != b))
            }
            (Value::Float(a), BinaryOp::NotEqual, Value::Float(b)) => Ok(Value::Boolean(a != b)),
            (Value::String(a), BinaryOp::NotEqual, Value::String(b)) => {
                Ok(Value::Boolean(*a != *b))
            }
            // Strings order lexicographically, matching the bytecode tier — so
            // character-range checks like `c >= "0" && c <= "9"` work and
            // strings sort. Ordering is by Unicode scalar value.
            (Value::String(a), BinaryOp::LessThan, Value::String(b)) => Ok(Value::Boolean(*a < *b)),
            (Value::String(a), BinaryOp::LessThanEqual, Value::String(b)) => {
                Ok(Value::Boolean(*a <= *b))
            }
            (Value::String(a), BinaryOp::GreaterThan, Value::String(b)) => {
                Ok(Value::Boolean(*a > *b))
            }
            (Value::String(a), BinaryOp::GreaterThanEqual, Value::String(b)) => {
                Ok(Value::Boolean(*a >= *b))
            }
            (Value::Boolean(a), BinaryOp::NotEqual, Value::Boolean(b)) => {
                Ok(Value::Boolean(a != b))
            }
            (Value::Integer(a), BinaryOp::Equal, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) == b))
            }
            (Value::Float(a), BinaryOp::Equal, Value::Integer(b)) => {
                Ok(Value::Boolean(a == b as f64))
            }
            (Value::Integer(a), BinaryOp::NotEqual, Value::Float(b)) => {
                Ok(Value::Boolean((a as f64) != b))
            }
            (Value::Float(a), BinaryOp::NotEqual, Value::Integer(b)) => {
                Ok(Value::Boolean(a != b as f64))
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
            (Value::String(a), BinaryOp::Add, Value::String(b)) => {
                let mut s = (*a).clone();
                s.push_str(&b);
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
            (Value::List(a), BinaryOp::Add, Value::List(b)) => {
                let mut items = Vec::with_capacity(a.len() + b.len());
                items.extend(a.iter().cloned());
                items.extend(b.iter().cloned());
                Ok(Value::List(std::sync::Arc::from(items)))
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

    pub(crate) fn to_boolean(&self, value: &Value) -> Result<bool, InterpreterError> {
        match value {
            Value::Boolean(b) => Ok(*b),
            Value::Integer(n) => Ok(*n != 0),
            Value::Float(x) => Ok(*x != 0.0),
            Value::String(s) => Ok(!s.is_empty()),
            Value::List(items) => Ok(!items.is_empty()),
            Value::Tuple(items) => Ok(!items.is_empty()),
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                if *inclusive {
                    Ok(start <= end)
                } else {
                    Ok(start < end)
                }
            }
            Value::Unit => Ok(false),
            _ => Ok(true),
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

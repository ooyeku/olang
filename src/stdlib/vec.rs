//! `vec` — vector arithmetic over lists of numbers.
//!
//! The kernels every hand-written numeric loop in the examples spelled
//! out: dot products, axpy updates, norms. Lists of Int or Float go in;
//! Float lists (or a Float) come out. Length mismatches are errors, not
//! truncation.

use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub fn create_vec_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [
        ("dot", 2),
        ("add", 2),
        ("sub", 2),
        ("scale", 2),
        ("sum", 1),
        ("norm", 1),
        ("mean", 1),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("vec.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

fn floats(args: &[Value], i: usize, func: &str) -> Result<Vec<f64>, Box<dyn std::error::Error>> {
    match args.get(i) {
        Some(Value::List(items)) => items
            .iter()
            .enumerate()
            .map(|(j, v)| match v {
                Value::Integer(n) => Ok(*n as f64),
                Value::Float(x) => Ok(*x),
                other => Err(format!(
                    "vec.{}: element {} is {}, expected a number",
                    func,
                    j,
                    other.type_name()
                )
                .into()),
            })
            .collect(),
        Some(other) => Err(format!(
            "vec.{}: expected a list of numbers, got {}",
            func,
            other.type_name()
        )
        .into()),
        None => Err(format!("vec.{}: missing argument", func).into()),
    }
}

fn scalar(args: &[Value], i: usize, func: &str) -> Result<f64, Box<dyn std::error::Error>> {
    match args.get(i) {
        Some(Value::Integer(n)) => Ok(*n as f64),
        Some(Value::Float(x)) => Ok(*x),
        Some(other) => {
            Err(format!("vec.{}: expected a number, got {}", func, other.type_name()).into())
        }
        None => Err(format!("vec.{}: missing argument", func).into()),
    }
}

fn same_len(func: &str, a: &[f64], b: &[f64]) -> Result<(), Box<dyn std::error::Error>> {
    if a.len() != b.len() {
        return Err(format!("vec.{}: lengths differ ({} and {})", func, a.len(), b.len()).into());
    }
    Ok(())
}

fn list(values: Vec<f64>) -> Value {
    Value::List(Arc::from(
        values.into_iter().map(Value::Float).collect::<Vec<_>>(),
    ))
}

fn finite(x: f64, func: &str) -> Result<Value, Box<dyn std::error::Error>> {
    if x.is_finite() {
        Ok(Value::Float(x))
    } else {
        Err(crate::ast::float_overflow_message(&format!("vec.{}", func)).into())
    }
}

pub fn call_vec_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "dot" => {
            let (a, b) = (floats(&args, 0, name)?, floats(&args, 1, name)?);
            same_len(name, &a, &b)?;
            finite(a.iter().zip(&b).map(|(x, y)| x * y).sum(), name)
        }
        "add" | "sub" => {
            let (a, b) = (floats(&args, 0, name)?, floats(&args, 1, name)?);
            same_len(name, &a, &b)?;
            let out: Vec<f64> = if name == "add" {
                a.iter().zip(&b).map(|(x, y)| x + y).collect()
            } else {
                a.iter().zip(&b).map(|(x, y)| x - y).collect()
            };
            if let Some(bad) = out.iter().find(|x| !x.is_finite()) {
                let _ = bad;
                return Err(crate::ast::float_overflow_message(&format!("vec.{}", name)).into());
            }
            Ok(list(out))
        }
        "scale" => {
            let a = floats(&args, 0, name)?;
            let k = scalar(&args, 1, name)?;
            let out: Vec<f64> = a.iter().map(|x| x * k).collect();
            if out.iter().any(|x| !x.is_finite()) {
                return Err(crate::ast::float_overflow_message("vec.scale").into());
            }
            Ok(list(out))
        }
        "sum" => finite(floats(&args, 0, name)?.iter().sum(), name),
        "norm" => finite(
            floats(&args, 0, name)?
                .iter()
                .map(|x| x * x)
                .sum::<f64>()
                .sqrt(),
            name,
        ),
        "mean" => {
            let a = floats(&args, 0, name)?;
            if a.is_empty() {
                return Err("vec.mean: the list is empty".into());
            }
            finite(a.iter().sum::<f64>() / a.len() as f64, name)
        }
        _ => Err(format!("Unknown vec function: {}", name).into()),
    }
}

//! The `time` module: clocks and sleeping.
//!
//! `dates` handles calendars and formatting; this module is about measuring
//! and pacing — epoch milliseconds for timestamps, a monotonic clock for
//! durations (it never goes backwards, unlike wall time), and a blocking
//! sleep.

use crate::ast::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;

/// The monotonic clock's origin: the first time anything asked for it.
static MONOTONIC_ORIGIN: OnceLock<Instant> = OnceLock::new();

pub fn create_time_module() -> Value {
    let mut module = HashMap::new();

    module.insert("now_ms".to_string(), create_builtin_function("now_ms", 0));
    module.insert(
        "monotonic_ms".to_string(),
        create_builtin_function("monotonic_ms", 0),
    );
    module.insert("sleep".to_string(), create_builtin_function("sleep", 1));

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("time.{}", name),
        arity,
    })
}

pub fn call_time_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "now_ms" => time_now_ms(args),
        "monotonic_ms" => time_monotonic_ms(args),
        "sleep" => time_sleep(args),
        _ => Err(format!("Unknown time function: {}", name).into()),
    }
}

/// Milliseconds since the Unix epoch, as an integer.
/// Usage: time.now_ms() -> Int
fn time_now_ms(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("now_ms expects 0 arguments, got {}", args.len()).into());
    }
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(Value::Integer(ms))
}

/// Milliseconds on a monotonic clock (origin: first use in this process).
/// Never goes backwards — the right clock for measuring durations.
/// Usage: time.monotonic_ms() -> Int
fn time_monotonic_ms(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("monotonic_ms expects 0 arguments, got {}", args.len()).into());
    }
    let origin = MONOTONIC_ORIGIN.get_or_init(Instant::now);
    Ok(Value::Integer(origin.elapsed().as_millis() as i64))
}

/// Block for the given number of milliseconds.
/// Usage: time.sleep(50) -> ()
fn time_sleep(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("sleep expects 1 argument (ms), got {}", args.len()).into());
    }
    let ms = match &args[0] {
        Value::Integer(n) if *n >= 0 => *n as u64,
        Value::Integer(_) => return Err("sleep: milliseconds must be non-negative".into()),
        other => {
            return Err(format!(
                "sleep: milliseconds must be an integer, got {}",
                other.type_name()
            )
            .into())
        }
    };
    std::thread::sleep(std::time::Duration::from_millis(ms));
    Ok(Value::Unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ms_is_a_plausible_epoch() {
        let v = time_now_ms(vec![]).unwrap();
        match v {
            Value::Integer(ms) => assert!(ms > 1_600_000_000_000, "epoch ms sanity"),
            other => panic!("expected integer, got {:?}", other),
        }
    }

    #[test]
    fn monotonic_never_goes_backwards() {
        let a = match time_monotonic_ms(vec![]).unwrap() {
            Value::Integer(n) => n,
            _ => unreachable!(),
        };
        let b = match time_monotonic_ms(vec![]).unwrap() {
            Value::Integer(n) => n,
            _ => unreachable!(),
        };
        assert!(b >= a);
    }

    #[test]
    fn sleep_rejects_bad_arguments() {
        assert!(time_sleep(vec![Value::Integer(-1)]).is_err());
        assert!(time_sleep(vec![Value::Boolean(true)]).is_err());
    }

    #[test]
    fn sleep_actually_sleeps() {
        let before = Instant::now();
        time_sleep(vec![Value::Integer(30)]).unwrap();
        assert!(before.elapsed().as_millis() >= 25);
    }
}

//! The `sim` module as olang sees it (S1):
//!
//! ```text
//! let w = sim.world(#{ "seed": 7, "start": 1767225600000 })
//! let run = w.run_fn(main, #{ "for": sim.hours(6) })
//! run.outcome    // "done", "failed", "deadline", "deadlock" or "step_limit"
//! run.events     // [#{ t, task, kind, detail }, ...] in order
//! ```
//!
//! A world is a value (its configuration); `run_fn` runs a function as
//! the world's main program and answers a report map. Nodes, the network,
//! the disk and crashes are S2 and S3: `World` grows methods then.

use super::{Config, Report};
use crate::ast::{BuiltinFunction, Value, ValueMap};
use crate::interpreter::{Interpreter, InterpreterError};
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::sync::Arc;

/// A world's configuration, as an olang value. Running it (`run_fn`)
/// makes a fresh world each time, so one value runs the same world as
/// often as it is asked.
#[derive(Debug)]
struct WorldObject {
    cfg: Config,
}

impl NativeObject for WorldObject {
    fn module(&self) -> &'static str {
        "sim.World"
    }
    fn type_name(&self) -> &'static str {
        "World"
    }
    fn display(&self) -> String {
        format!("<world seed {}>", self.cfg.seed)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<WorldObject>()
            .is_some_and(|o| {
                o.cfg.seed == self.cfg.seed
                    && o.cfg.start_ms == self.cfg.start_ms
                    && o.cfg.max_steps == self.cfg.max_steps
            })
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn field(&self, name: &str) -> Option<Result<Value, String>> {
        match name {
            "seed" => Some(Ok(Value::Integer(self.cfg.seed as i64))),
            "start" => Some(Ok(Value::Integer(self.cfg.start_ms))),
            _ => None,
        }
    }
}

fn builtin(name: &str, arity: usize) -> Value {
    Value::Builtin(BuiltinFunction {
        name: format!("sim.{name}"),
        arity,
    })
}

pub fn create_sim_module() -> Value {
    let mut module = ValueMap::default();
    module.insert("world".to_string(), builtin("world", 1));
    for unit in ["ms", "seconds", "minutes", "hours"] {
        module.insert(unit.to_string(), builtin(unit, 1));
    }
    let mut world = ValueMap::default();
    world.insert("run_fn".to_string(), builtin("World.run_fn", 3));
    module.insert(
        "World".to_string(),
        Value::Struct {
            type_name: "Module".to_string(),
            fields: Arc::new(world),
        },
    );
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

fn misuse(message: impl Into<String>) -> InterpreterError {
    InterpreterError::runtime(message)
}

/// Dispatch `sim.<name>`.
pub fn call(
    name: &str,
    args: Vec<Value>,
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    match name {
        "world" => world(&args),
        "ms" => duration(&args, "ms", 1),
        "seconds" => duration(&args, "seconds", 1_000),
        "minutes" => duration(&args, "minutes", 60_000),
        "hours" => duration(&args, "hours", 3_600_000),
        "World.run_fn" => run_fn(args, interpreter),
        _ => Err(misuse(format!("Unknown sim function: sim.{name}"))),
    }
}

/// `sim.ms(n)`, `sim.seconds(n)`, ...: a duration as virtual ms.
fn duration(args: &[Value], name: &str, scale: i64) -> Result<Value, InterpreterError> {
    let ms = match args {
        [Value::Integer(n)] => n.checked_mul(scale),
        [Value::Float(f)] if f.is_finite() => Some((f * scale as f64).round() as i64),
        _ => None,
    };
    match ms {
        Some(ms) if ms >= 0 => Ok(Value::Integer(ms)),
        _ => Err(misuse(format!(
            "sim.{name} expects a non-negative number, got {}",
            args.first()
                .map(|a| a.type_name())
                .unwrap_or_else(|| "nothing".into())
        ))),
    }
}

fn fields_of(v: &Value) -> Option<&ValueMap> {
    match v {
        Value::Map(m) => Some(m),
        Value::Struct { fields, .. } => Some(fields),
        _ => None,
    }
}

fn int_field(opts: &ValueMap, key: &str, what: &str) -> Result<Option<i64>, InterpreterError> {
    match opts.get(key) {
        None | Some(Value::Unit) => Ok(None),
        Some(Value::Integer(n)) => Ok(Some(*n)),
        Some(other) => Err(misuse(format!(
            "{what}: `{key}` must be an Int, got {}",
            other.type_name()
        ))),
    }
}

/// `sim.world(#{ seed, start, max_steps })`.
fn world(args: &[Value]) -> Result<Value, InterpreterError> {
    let mut cfg = Config::default();
    match args {
        [] | [Value::Unit] => {}
        [opts] => {
            let Some(opts) = fields_of(opts) else {
                return Err(misuse(format!(
                    "sim.world expects #{{ seed, start }}, got {}",
                    opts.type_name()
                )));
            };
            for key in opts.keys() {
                if !matches!(key.as_str(), "seed" | "start" | "max_steps") {
                    return Err(misuse(format!(
                        "sim.world: unknown option `{key}` (S1 knows seed, start and \
                         max_steps; net, disk and nodes arrive in S2 and S3)"
                    )));
                }
            }
            if let Some(seed) = int_field(opts, "seed", "sim.world")? {
                cfg.seed = seed as u64;
            }
            if let Some(start) = int_field(opts, "start", "sim.world")? {
                cfg.start_ms = start;
            }
            if let Some(steps) = int_field(opts, "max_steps", "sim.world")? {
                if steps <= 0 {
                    return Err(misuse("sim.world: `max_steps` must be positive"));
                }
                cfg.max_steps = steps as u64;
            }
        }
        _ => return Err(misuse("sim.world expects one argument: #{ seed, start }")),
    }
    Ok(Value::Native(NativeHandle::new(WorldObject { cfg })))
}

/// `w.run_fn(f, #{ for })`: run `f()` as the world's main program.
fn run_fn(args: Vec<Value>, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
    let mut args = args.into_iter();
    let (Some(world), Some(f)) = (args.next(), args.next()) else {
        return Err(misuse("run_fn expects a function: w.run_fn(f, #{ for })"));
    };
    let opts = args.next();
    let Value::Native(handle) = &world else {
        return Err(misuse("run_fn is a method of a world (sim.world)"));
    };
    let Some(world) = handle.0.as_any().downcast_ref::<WorldObject>() else {
        return Err(misuse("run_fn is a method of a world (sim.world)"));
    };
    if !matches!(f, Value::Function(_) | Value::Builtin(_)) {
        return Err(misuse(format!(
            "run_fn expects a function to run as the world's main program, got {}",
            f.type_name()
        )));
    }
    let mut cfg = world.cfg.clone();
    if let Some(opts) = opts.as_ref().filter(|o| !matches!(o, Value::Unit)) {
        let Some(opts) = fields_of(opts) else {
            return Err(misuse(format!(
                "run_fn: options are #{{ for }}, got {}",
                opts.type_name()
            )));
        };
        for key in opts.keys() {
            if key != "for" {
                return Err(misuse(format!(
                    "run_fn: unknown option `{key}` (S1 knows `for`)"
                )));
            }
        }
        if let Some(ms) = int_field(opts, "for", "run_fn")? {
            if ms < 0 {
                return Err(misuse("run_fn: `for` must be non-negative"));
            }
            cfg.run_for = Some(ms as u64);
        }
    }
    // The world runs on the interpreter tier: every builtin through the
    // one dispatch point that sees it (as record and replay run).
    let kept = interpreter.suspend_bytecode_tier();
    let report = super::run(cfg, || interpreter.call_function(f, Vec::new()));
    interpreter.resume_bytecode_tier(kept);
    let report = report.map_err(misuse)?;
    Ok(report_value(&report))
}

/// A report as olang data.
pub fn report_value(r: &Report) -> Value {
    let s = |t: &str| Value::String(Arc::new(t.to_string()));
    let mut m = ValueMap::default();
    m.insert("seed".to_string(), Value::Integer(r.seed as i64));
    m.insert("outcome".to_string(), s(r.outcome));
    m.insert("value".to_string(), r.value.clone().unwrap_or(Value::Unit));
    m.insert(
        "error".to_string(),
        r.error.as_deref().map(s).unwrap_or(Value::Unit),
    );
    m.insert("now".to_string(), Value::Integer(r.now as i64));
    m.insert("steps".to_string(), Value::Integer(r.steps as i64));
    m.insert("tasks".to_string(), Value::Integer(r.tasks as i64));
    m.insert(
        "failed_tasks".to_string(),
        Value::Integer(r.failed_tasks as i64),
    );
    let events = r
        .events
        .iter()
        .map(|e| {
            let mut ev = ValueMap::default();
            ev.insert("t".to_string(), Value::Integer(e.t as i64));
            ev.insert("task".to_string(), Value::Integer(e.task as i64));
            ev.insert("kind".to_string(), s(e.kind));
            ev.insert("detail".to_string(), s(&e.detail));
            Value::Map(Arc::new(ev))
        })
        .collect::<Vec<_>>();
    m.insert("events".to_string(), Value::List(Arc::new(events)));
    Value::Map(Arc::new(m))
}

//! `task` — background threads and how to collect their results.
//!
//! `spawn f(x)` starts `f(x)` on a real OS thread and hands back a **task
//! handle**. `task.join(t)` blocks until that thread finishes and returns
//! what it produced, or `Err(e)` if it failed. That is the whole model:
//!
//! ```text
//! let t = spawn fetch("users")
//! match task.join(t) {
//!     Err(e) => println("failed: " + show(e))
//!     v      => println(v.name)
//! }
//! ```
//!
//! Joining several is just `map`, since every task is already running:
//! `tasks |> map(task.join)`.
//!
//! ## Joining is not cancelling
//!
//! `task.join_timeout(t, ms)` bounds how long you *wait*. It does not stop
//! the task: an OS thread cannot be cancelled from outside without leaving
//! whatever it was touching in an unknown state, so olang does not pretend
//! otherwise. A timed-out task keeps running to completion; its result is
//! memoized, so a later `task.join` on the same handle still collects it.
//! When you need work that genuinely stops, have the task poll a condition
//! it can see — a channel, or a value it re-reads each iteration.
//!
//! ## Lifetime
//!
//! The handle owns the task's registry entry through an `Arc`'d guard, so
//! a completed task's memoized result is freed when the last handle to it
//! drops. A program that spawns a worker pool every tick does not
//! accumulate finished tasks. A handle dropped without ever being joined
//! is fire-and-forget: the thread detaches and runs to completion.

use crate::ast::Value;
use crate::interpreter::InterpreterError;
use crate::interpreter::spawn_registry::{self, SpawnGuard};
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// A handle to one spawned thread. The `Arc<SpawnGuard>` is what ties the
/// registry entry's lifetime to the handle's: every clone of the value
/// shares one guard, and the last drop reclaims the entry.
struct TaskObject {
    id: u64,
    _guard: Arc<SpawnGuard>,
}

impl std::fmt::Debug for TaskObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<task {}>", self.id)
    }
}

impl NativeObject for TaskObject {
    fn module(&self) -> &'static str {
        "task"
    }
    fn type_name(&self) -> &'static str {
        "Task"
    }
    fn display(&self) -> String {
        format!("<task {}>", self.id)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        // Identity: two handles are equal iff they name the same thread.
        other
            .as_any()
            .downcast_ref::<TaskObject>()
            .map(|o| o.id == self.id)
            .unwrap_or(false)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Wrap a freshly registered task id as an olang value. Called by the
/// interpreter's `spawn`.
pub fn handle(id: u64) -> Value {
    Value::Native(NativeHandle::new(TaskObject {
        id,
        _guard: Arc::new(SpawnGuard::new(id)),
    }))
}

pub fn create_task_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [
        ("join", 1),
        ("join_timeout", 2),
        ("list", 0),
        ("parked", 0),
        ("watch", 2),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("task.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

fn task_id(value: &Value) -> Result<u64, String> {
    match value {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<TaskObject>()
                .map(|t| t.id)
                .ok_or_else(|| format!("task: expected a task, got a {} handle", h.0.type_name()))
        }
        other => Err(format!(
            "task: expected a task (from `spawn`), got {}",
            other.type_name()
        )),
    }
}

fn raise(message: String) -> InterpreterError {
    InterpreterError::runtime(message)
}

/// A task's failure is a *value*, not a crash: one worker falling over
/// must not take the program with it. `Err(e)` composes with `match`,
/// `unwrap_or`, `?`, and `try`/`catch` like any other fallible result.
fn settle(outcome: Result<Value, String>) -> Value {
    match outcome {
        Ok(v) => v,
        Err(e) => Value::Err(Box::new(Value::String(Arc::new(e)))),
    }
}

pub fn call_task_function(name: &str, args: Vec<Value>) -> Result<Value, InterpreterError> {
    match name {
        "join" => task_join(args),
        "join_timeout" => task_join_timeout(args),
        "watch" => task_watch(args),
        "list" => task_list(args),
        "parked" => task_parked(args),
        _ => Err(raise(format!("Unknown task function: {}", name))),
    }
}

/// Every task a live handle still watches, as `#{ id, state,
/// elapsed_ms }` maps — the introspection the REPL and the profiler
/// read. States: "running", "joining" (a thread is mid-collect), and
/// "done" (finished; the memoized result may or may not be collected).
fn task_list(args: Vec<Value>) -> Result<Value, InterpreterError> {
    if !args.is_empty() {
        return Err(raise("task.list takes no arguments".to_string()));
    }
    let rows = spawn_registry::list()
        .into_iter()
        .map(|(id, state, elapsed_ms)| {
            let mut m = HashMap::new();
            m.insert("id".to_string(), Value::Integer(id as i64));
            m.insert(
                "state".to_string(),
                Value::String(Arc::new(state.to_string())),
            );
            m.insert("elapsed_ms".to_string(), Value::Integer(elapsed_ms as i64));
            Value::Map(Arc::new(m))
        })
        .collect();
    Ok(Value::List(Arc::new(rows)))
}

/// Every thread currently blocked in an unbounded wait, as
/// `#{ thread, on, waited_ms }` maps — the live view behind the stall
/// detector's report.
fn task_parked(args: Vec<Value>) -> Result<Value, InterpreterError> {
    if !args.is_empty() {
        return Err(raise("task.parked takes no arguments".to_string()));
    }
    let rows = crate::stdlib::chan::parked_sites()
        .into_iter()
        .map(|(thread, what, waited_ms)| {
            let mut m = HashMap::new();
            m.insert("thread".to_string(), Value::String(Arc::new(thread)));
            m.insert("on".to_string(), Value::String(Arc::new(what)));
            m.insert("waited_ms".to_string(), Value::Integer(waited_ms as i64));
            Value::Map(Arc::new(m))
        })
        .collect();
    Ok(Value::List(Arc::new(rows)))
}

/// `task.watch(t, c)`: the channel dies with the task. When `t` ends —
/// by returning or by raising — `c` is closed, so a `chan.recv` on it
/// returns `Err` instead of waiting forever for a sender that no longer
/// exists. Ownership is declared here, not inferred: any task may hold
/// either end of a channel, so only the program can say which task's
/// death should close it. A task that already ended closes `c` at once.
fn task_watch(args: Vec<Value>) -> Result<Value, InterpreterError> {
    if args.len() != 2 {
        return Err(raise(
            "task.watch expects two arguments: the task and the channel".to_string(),
        ));
    }
    let id = task_id(&args[0]).map_err(raise)?;
    if !crate::stdlib::chan::is_channel(&args[1]) {
        return Err(raise(format!(
            "task.watch: expected a channel, got {}",
            args[1].type_name()
        )));
    }
    if !spawn_registry::watch(id, args[1].clone()) {
        crate::stdlib::chan::close_value(&args[1]);
    }
    Ok(Value::Unit)
}

fn task_join(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let _parked = crate::profile::blocked();
    if args.len() != 1 {
        return Err(raise(
            "task.join expects one argument: the task".to_string(),
        ));
    }
    let id = task_id(&args[0]).map_err(raise)?;
    match spawn_registry::join(id) {
        Some(outcome) => Ok(settle(outcome)),
        None => Err(raise(
            "task.join: this task is no longer available (its handle was already released)"
                .to_string(),
        )),
    }
}

fn task_join_timeout(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let ms = match args.as_slice() {
        [_, Value::Integer(ms)] if *ms >= 0 => *ms as u64,
        _ => {
            return Err(raise(
                "task.join_timeout expects a task and a non-negative Int of milliseconds"
                    .to_string(),
            ));
        }
    };
    let id = task_id(&args[0]).map_err(raise)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    match spawn_registry::join_until(id, deadline) {
        // Finished in time. Wrapped in Ok so the caller can tell "the task
        // produced Err(e)" from "we gave up waiting" — both are Err
        // otherwise, and they mean very different things.
        Some(Some(outcome)) => Ok(Value::Ok(Box::new(settle(outcome)))),
        Some(None) => Ok(Value::Err(Box::new(Value::String(Arc::new(
            "timed out".to_string(),
        ))))),
        None => Err(raise(
            "task.join_timeout: this task is no longer available (its handle was already released)"
                .to_string(),
        )),
    }
}

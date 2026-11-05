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
//! ## Joining is not cancelling; interrupting is
//!
//! `task.join_timeout(t, ms)` bounds how long you *wait*. It does not stop
//! the task: a timed-out task keeps running; its result is memoized, so a
//! later `task.join` on the same handle still collects it.
//!
//! `task.interrupt(t, reason?)` stops it. The task raises "interrupted"
//! (or "interrupted: reason") at its next poll: the next statement, loop
//! iteration or call, on every tier, or at once when it is waiting in
//! `chan.recv`, `chan.ask`, a full `chan.send`, `task.join`, `time.sleep`
//! or a `proc` read. The error unwinds like any other, so an `attempt`
//! can catch it and clean up, and it is asked once: code that catches it
//! runs on. A task inside a system call that cannot be woken raises once
//! the call returns; `task.list` shows it as "interrupting" until then.
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
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [
        ("join", 1),
        ("join_timeout", 2),
        ("interrupt", 1),
        ("set_qos", 1),
        ("qos", 0),
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
        type_name: "Module".into(),
        fields: std::sync::Arc::new(module).into(),
    }
}

pub(crate) fn task_id(value: &Value) -> Result<u64, String> {
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
/// A finished task as a `Result`: `Ok(value)` when it returned (a value
/// that is itself an `Err` included), `Err(message)` when it raised.
pub(crate) fn settle(outcome: Result<Value, String>) -> Value {
    match outcome {
        Ok(v) => Value::Ok(Box::new(v)),
        Err(e) => Value::Err(Box::new(Value::String(Arc::new(e)))),
    }
}

pub fn call_task_function(name: &str, args: Vec<Value>) -> Result<Value, InterpreterError> {
    match name {
        "join" => task_join(args),
        "join_timeout" => task_join_timeout(args),
        "interrupt" => task_interrupt(args),
        "set_qos" => task_set_qos(args),
        "qos" => task_qos(args),
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
    // a test's way to a Rust panic inside a task (OLANG_TEST_PANIC)
    crate::contain::inject("task.list");
    if !args.is_empty() {
        return Err(raise("task.list takes no arguments".to_string()));
    }
    let rows = spawn_registry::list()
        .into_iter()
        .map(|(id, state, elapsed_ms)| {
            let mut m = crate::ast::ValueMap::default();
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
    crate::contain::inject("task.parked");
    if !args.is_empty() {
        return Err(raise("task.parked takes no arguments".to_string()));
    }
    let rows = crate::stdlib::chan::parked_sites()
        .into_iter()
        .map(|(thread, what, waited_ms)| {
            let mut m = crate::ast::ValueMap::default();
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
    match spawn_registry::join(id).map_err(raise)? {
        Some(outcome) => Ok(settle(outcome)),
        None => Err(raise(
            "task.join: this task is no longer available (its handle was already released)"
                .to_string(),
        )),
    }
}

// ── QoS classes (OL-507) ────────────────────────────────────────────
/// The macOS quality-of-service classes, by name, with the codes
/// `pthread_set_qos_class_self_np` takes. The scheduler gives a thread
/// that does less urgent work a lower class: it then yields the CPU (and
/// on Apple silicon the performance cores) to the higher ones.
pub const QOS_CLASSES: [(&str, u8); 5] = [
    ("user-interactive", 0x21),
    ("user-initiated", 0x19),
    ("default", 0x15),
    ("utility", 0x11),
    ("background", 0x09),
];

fn qos_code(name: &str) -> Option<u8> {
    QOS_CLASSES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
}

fn qos_name(code: u8) -> &'static str {
    QOS_CLASSES
        .iter()
        .find(|(_, c)| *c == code)
        .map(|(n, _)| *n)
        .unwrap_or("default")
}

/// Give the calling thread QoS class `code`. Does nothing off macOS.
pub fn apply_qos(code: u8) {
    #[cfg(target_os = "macos")]
    {
        use libc::qos_class_t::*;
        let class = match code {
            0x21 => QOS_CLASS_USER_INTERACTIVE,
            0x19 => QOS_CLASS_USER_INITIATED,
            0x11 => QOS_CLASS_UTILITY,
            0x09 => QOS_CLASS_BACKGROUND,
            _ => QOS_CLASS_DEFAULT,
        };
        // SAFETY: sets the calling thread's own class; no pointers.
        unsafe {
            libc::pthread_set_qos_class_self_np(class, 0);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = code;
}

/// What a spawned task inherits: the spawner's class, unless it is the
/// default one.
pub fn spawner_qos() -> Option<u8> {
    let code = current_qos();
    (code != 0x15).then_some(code)
}

/// A new task's first act: take the class its spawner had.
pub fn inherit_qos(code: Option<u8>) {
    if let Some(code) = code {
        apply_qos(code);
    }
}

/// The calling thread's QoS class code (`default` off macOS).
fn current_qos() -> u8 {
    #[cfg(target_os = "macos")]
    {
        let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
        let mut rel: libc::c_int = 0;
        // SAFETY: reads the calling thread's class into two locals.
        unsafe {
            libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut rel);
        }
        let code = class as u32 as u8;
        if qos_name(code) == "default" {
            0x15
        } else {
            code
        }
    }
    #[cfg(not(target_os = "macos"))]
    0x15
}

fn qos_arg(value: &Value) -> Result<u8, InterpreterError> {
    match value {
        Value::String(s) => qos_code(s).ok_or_else(|| {
            raise(format!(
                "task.set_qos: no QoS class {:?} (user-interactive, user-initiated, default, utility, background)",
                s.as_str()
            ))
        }),
        other => Err(raise(format!(
            "task.set_qos: the class is a String, got {}",
            other.type_name()
        ))),
    }
}

/// `task.set_qos(class)` gives the calling thread a QoS class;
/// `task.set_qos(t, class)` gives it to task `t`, which takes it at its
/// next poll, or at once when it waits. Answers the class.
fn task_set_qos(args: Vec<Value>) -> Result<Value, InterpreterError> {
    match args.as_slice() {
        [class] => {
            let code = qos_arg(class)?;
            apply_qos(code);
            Ok(Value::String(Arc::new(qos_name(code).to_string())))
        }
        [t, class] => {
            let code = qos_arg(class)?;
            let id = task_id(t).map_err(raise)?;
            spawn_registry::stop_handle(id)
                .ok_or_else(|| {
                    raise(
                        "task.set_qos: this task is no longer available (its handle was already released)"
                            .to_string(),
                    )
                })?
                .request_qos(code);
            Ok(Value::String(Arc::new(qos_name(code).to_string())))
        }
        _ => Err(raise(
            "task.set_qos expects a QoS class, or a task and a QoS class".to_string(),
        )),
    }
}

/// `task.qos()`: the calling thread's QoS class.
fn task_qos(args: Vec<Value>) -> Result<Value, InterpreterError> {
    if !args.is_empty() {
        return Err(raise("task.qos takes no arguments".to_string()));
    }
    Ok(Value::String(Arc::new(qos_name(current_qos()).to_string())))
}

/// `task.interrupt(t, reason?)`: ask task `t` to stop. It raises
/// "interrupted" (with ": reason" when given) at its next poll, or at once
/// in a wait. `true` when the task was running and is now asked, `false`
/// when it had already ended.
fn task_interrupt(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let reason = match args.as_slice() {
        [_] => None,
        [_, Value::String(s)] => Some(s.as_ref().clone()),
        [_, Value::Unit] => None,
        [_, other] => Some(other.to_string()),
        _ => {
            return Err(raise(
                "task.interrupt expects a task and, optionally, a reason".to_string(),
            ));
        }
    };
    let id = task_id(&args[0]).map_err(raise)?;
    match spawn_registry::interrupt(id, reason) {
        Some(asked) => Ok(Value::Boolean(asked)),
        None => Err(raise(
            "task.interrupt: this task is no longer available (its handle was already released)"
                .to_string(),
        )),
    }
}

fn task_join_timeout(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let ms = match args.as_slice() {
        [_, limit] => crate::stdlib::timeout_ms_arg(limit),
        _ => None,
    }
    .ok_or_else(|| {
        raise(
            "task.join_timeout expects a task and a non-negative number of milliseconds (or #{ timeout_ms })"
                .to_string(),
        )
    })?;
    let id = task_id(&args[0]).map_err(raise)?;
    let deadline = crate::clock::Instant::now() + std::time::Duration::from_millis(ms);
    match spawn_registry::join_until(id, deadline).map_err(raise)? {
        // Finished in time: `Ok` of what `task.join` answers (`Ok(v)`, or
        // `Err(failure)` when it raised); `Err("timed out")` is the wait
        // given up, never the task's own answer.
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

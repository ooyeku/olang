//! `cell` — the one mutable location olang has.
//!
//! Values are immutable and closures capture by value, which is what makes
//! `spawn` and `par_map` safe without locks. That leaves one thing
//! genuinely hard to express: state that outlives an expression and is
//! updated in place — a counter, an accumulator, a memo table. Threading it
//! through every call is the honest workaround, and for deep call chains it
//! is a poor one. A cell is the escape hatch, deliberately narrow:
//!
//! ```text
//! let counter = cell(0)              // cell.new(0) spelled shorter
//! cell.set(counter, cell.get(counter) + 1)
//! cell.update(counter, (n) => n + 1)
//! ```
//!
//! ## Confinement
//!
//! A cell belongs to the thread that created it. Reading or writing one
//! from another thread is an error naming both threads. This is what keeps
//! the language's central guarantee intact: no two threads can reach the
//! same mutable location, so the absence of data races is still structural
//! rather than a matter of discipline.
//!
//! Enforcement is at *access*, not at the thread boundary, because `spawn`
//! and `par_map` snapshot the whole environment rather than an enumerated
//! capture list — there is no list of "what crossed" to inspect. Checking
//! on use is both sound and precise: a cell that merely sits in scope while
//! a task runs is harmless and stays legal, and only an actual cross-thread
//! access fails. `chan.send` is the one crossing where the value *is* in
//! hand, so that one is refused at the send, where the mistake is.
//!
//! ## Re-entrancy
//!
//! `cell.update(c, f)` marks the cell in-update while `f` runs; touching
//! the same cell from inside `f` is an error rather than a write that `f`'s
//! return value silently overwrites. The lock is never held across `f`, so
//! the failure is a diagnosable error, not a deadlock.
//!
//! ## Determinism
//!
//! Cell mutation is deterministic within a thread and unreachable across
//! threads, so cells need no timeline recording — a replayed run performs
//! the same mutations in the same order.

use crate::ast::Value;
use crate::interpreter::InterpreterError;
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Mutex;
use std::thread::ThreadId;

const POISONED: &str = "cell is unusable: the thread holding it panicked mid-update";

/// A cell's contents plus its re-entrancy flag. The mutex is uncontended
/// by construction (only the owning thread reaches it) and exists to
/// satisfy `Send + Sync`, which every `Value` must be.
struct CellState {
    value: Value,
    updating: bool,
}

struct CellObject {
    owner: ThreadId,
    /// The owning thread's name, captured at creation for the error
    /// message. `spawn` names its threads `olang-spawn-N`; the main
    /// thread is `main`.
    owner_label: String,
    state: Mutex<CellState>,
}

/// How a thread names itself in an escape error.
fn thread_label() -> String {
    match std::thread::current().name() {
        Some("main") => "the main thread".to_string(),
        Some(name) => format!("thread '{}'", name),
        None => "an unnamed thread".to_string(),
    }
}

impl CellObject {
    fn escaped(&self) -> String {
        format!(
            "cell escaped its thread: this cell was created on {} and cannot be \
             read or written from {}. A cell belongs to one thread — send the \
             value through a channel, or return it from the task",
            self.owner_label,
            thread_label()
        )
    }

    fn reentrant(verb: &str) -> String {
        format!(
            "cell is being updated: `cell.update` is already running on this cell, \
             so it cannot be {} from inside the update function — compute the new \
             value from the argument instead",
            verb
        )
    }

    fn own_thread(&self) -> Result<(), String> {
        if std::thread::current().id() == self.owner {
            Ok(())
        } else {
            Err(self.escaped())
        }
    }
}

impl std::fmt::Debug for CellObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<cell>")
    }
}

impl NativeObject for CellObject {
    fn module(&self) -> &'static str {
        "cell"
    }
    fn type_name(&self) -> &'static str {
        "Cell"
    }
    fn display(&self) -> String {
        // Showing the contents needs the same standing as reading them, so
        // a cell printed from the wrong thread renders opaquely instead of
        // leaking the value through the back door that `println` would
        // otherwise be.
        if self.own_thread().is_err() {
            return "<cell: not on its owning thread>".to_string();
        }
        match self.state.lock() {
            Ok(st) if st.updating => "<cell: updating>".to_string(),
            Ok(st) => format!("cell({})", st.value),
            Err(_) => "<cell>".to_string(),
        }
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        // Identity, not contents: a cell is a location. Two cells holding
        // equal values are still two places to write.
        other
            .as_any()
            .downcast_ref::<CellObject>()
            .map(|o| std::ptr::eq(self, o))
            .unwrap_or(false)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn confined_to(&self) -> Option<ThreadId> {
        Some(self.owner)
    }
}

/// The `cell` module. It is *callable*: `cell(0)` is `cell.new(0)`, which
/// is why the constructor reads as a one-word noun at the use site while
/// the operations stay namespaced.
pub fn create_cell_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [("new", 1), ("get", 1), ("set", 2), ("update", 2)] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("cell.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Pull the cell back out of a handle.
fn cell_of(value: &Value) -> Result<&CellObject, String> {
    match value {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<CellObject>()
                .ok_or_else(|| format!("cell: expected a cell, got a {} handle", h.0.type_name()))
        }
        other => Err(format!(
            "cell: expected a cell (from `cell(v)`), got {}",
            other.type_name()
        )),
    }
}

/// The name of the thread-confined value inside `value`, if there is one.
/// Used by `chan.send`, the one thread crossing that holds the value being
/// sent — a confined value nested in a list or a struct field is exactly
/// the case a shallow check would wave through.
///
/// Asks each native whether it is confined rather than naming the types it
/// knows about, so a new confined native is refused here without anyone
/// remembering to come back and add it.
pub fn confined_within(value: &Value) -> Option<&'static str> {
    fn first<'a>(mut it: impl Iterator<Item = &'a Value>) -> Option<&'static str> {
        it.find_map(confined_within)
    }
    match value {
        Value::Native(h) => h.0.confined_to().map(|_| h.0.type_name()),
        Value::List(items) | Value::Tuple(items) => first(items.iter()),
        Value::Map(entries) => first(entries.values()),
        Value::Struct { fields, .. } => first(fields.values()),
        Value::Ok(inner) | Value::Err(inner) => confined_within(inner),
        Value::Enum { variant_data, .. } => match variant_data {
            crate::ast::EnumVariantData::Tuple(items) => first(items.iter()),
            crate::ast::EnumVariantData::Struct(fields) => first(fields.values()),
            crate::ast::EnumVariantData::Unit => None,
        },
        _ => None,
    }
}

/// Dispatch. Returns the interpreter's own error type rather than a
/// `String` so that an error raised by `cell.update`'s callback propagates
/// verbatim instead of being flattened into text and re-wrapped — which
/// would stack a second "Runtime error:" prefix on it.
pub fn call_cell_function(
    name: &str,
    args: Vec<Value>,
    interpreter: &mut crate::interpreter::Interpreter,
) -> Result<Value, InterpreterError> {
    match name {
        "new" => cell_new(args).map_err(raise),
        "get" => cell_get(args).map_err(raise),
        "set" => cell_set(args).map_err(raise),
        "update" => cell_update(args, interpreter),
        _ => Err(raise(format!("Unknown cell function: {}", name))),
    }
}

fn raise(message: String) -> InterpreterError {
    InterpreterError::RuntimeError { message }
}

fn cell_new(mut args: Vec<Value>) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("cell expects one argument: the initial value".to_string());
    }
    let value = args.pop().expect("len checked");
    Ok(Value::Native(NativeHandle::new(CellObject {
        owner: std::thread::current().id(),
        owner_label: thread_label(),
        state: Mutex::new(CellState {
            value,
            updating: false,
        }),
    })))
}

fn cell_get(args: Vec<Value>) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("cell.get expects one argument: the cell".to_string());
    }
    let cell = cell_of(&args[0])?;
    cell.own_thread()?;
    let state = cell.state.lock().map_err(|_| POISONED.to_string())?;
    if state.updating {
        return Err(CellObject::reentrant("read"));
    }
    Ok(state.value.clone())
}

fn cell_set(mut args: Vec<Value>) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("cell.set expects two arguments: the cell and the new value".to_string());
    }
    let value = args.pop().expect("len checked");
    let cell = cell_of(&args[0])?;
    cell.own_thread()?;
    let mut state = cell.state.lock().map_err(|_| POISONED.to_string())?;
    if state.updating {
        return Err(CellObject::reentrant("written"));
    }
    state.value = value;
    Ok(Value::Unit)
}

/// `cell.update(c, f)` — read, apply, store, and return the new value.
///
/// The lock is released before `f` runs, so a re-entrant access reports an
/// error rather than deadlocking; the `updating` flag is what makes that
/// access detectable. It is cleared whether `f` returns or raises.
fn cell_update(
    mut args: Vec<Value>,
    interpreter: &mut crate::interpreter::Interpreter,
) -> Result<Value, InterpreterError> {
    if args.len() != 2 {
        return Err(raise(
            "cell.update expects two arguments: the cell and a function".to_string(),
        ));
    }
    let function = args.pop().expect("len checked");
    let cell = cell_of(&args[0]).map_err(raise)?;
    cell.own_thread().map_err(raise)?;

    let current = {
        let mut state = cell.state.lock().map_err(|_| raise(POISONED.to_string()))?;
        if state.updating {
            return Err(raise(CellObject::reentrant("updated")));
        }
        state.updating = true;
        state.value.clone()
    };

    let applied = interpreter.call_function_optimized(&function, vec![current]);

    // Cleared whether the function returned or raised, so one failed update
    // does not leave the cell permanently unreadable.
    let mut state = cell.state.lock().map_err(|_| raise(POISONED.to_string()))?;
    state.updating = false;
    let next = applied?;
    state.value = next.clone();
    Ok(next)
}

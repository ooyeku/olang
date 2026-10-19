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
//! ## Growing a table in place
//!
//! `cell.get` answers a copy that shares the cell's value, so a map or a
//! list taken out with `get` and extended copies it whole — a memo table
//! grown with `cell.set(c, map_set(cell.get(c), k, v))` is quadratic in
//! its size (2,000 inserts, 57 ms). `cell.take(c)` moves the value out,
//! leaving `()`, so nothing else holds it and the extension is in place:
//!
//! ```text
//! cell.set(memo, map_set(cell.take(memo), key, value))   // in place
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
//! ## The bytecode VM's values
//!
//! A cell the VM writes holds the VM's own value, unconverted, tagged with
//! the VM that wrote it; that VM reads it back as it is. A table grown in
//! a cell (`cell.set(c, map_set(cell.take(c), k, v))`, a UI's memo) was
//! converted to the interpreter's form at every write and back at every
//! read — a frame's layout paid a subtree's conversion each time it found
//! one. The tree-walker, and any other VM, reads such a cell converted
//! (a closure's bytecode is its VM's own), as a value crossing the tiers
//! is.
//!
//! ## Determinism
//!
//! Cell mutation is deterministic within a thread and unreachable across
//! threads, so cells need no timeline recording — a replayed run performs
//! the same mutations in the same order.

use crate::ast::Value;
use crate::interpreter::InterpreterError;
use crate::native::{NativeHandle, NativeObject};
use crate::ovm::value::{OvmValue, ValueData};
use std::any::Any;
use std::sync::Mutex;
use std::thread::ThreadId;

const POISONED: &str = "cell is unusable: the thread holding it panicked mid-update";

/// A cell's contents plus its re-entrancy flag. The mutex is uncontended
/// by construction (only the owning thread reaches it) and exists to
/// satisfy `Send + Sync`, which every `Value` must be.
struct CellState {
    value: Value,
    /// The value as a bytecode VM set it, with that VM's mark
    /// (`next_vm_owner`); while it is here, `value` is `()`.
    vm: Option<(OvmValue, u64)>,
    updating: bool,
}

impl CellState {
    /// The value in the interpreter's form.
    fn ast(&self) -> Result<Value, String> {
        match &self.vm {
            Some((v, _)) => v.to_ast().map_err(|e| format!("cell: {}", e)),
            None => Ok(self.value.clone()),
        }
    }

    /// The value moved out, in the interpreter's form; the cell holds `()`.
    fn take_ast(&mut self) -> Result<Value, String> {
        match self.vm.take() {
            Some((v, _)) => v.to_ast().map_err(|e| format!("cell: {}", e)),
            None => Ok(std::mem::replace(&mut self.value, Value::Unit)),
        }
    }

    fn set_ast(&mut self, v: Value) {
        self.vm = None;
        self.value = v;
    }
}

/// A mark for a new bytecode VM: the cells it writes are its own to read
/// back unconverted.
pub fn next_vm_owner() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
        // A cell that holds itself (`cell.set(c, [c])`) is shown while it
        // is being shown: its lock is this very display's, so it is taken
        // only if free — waiting for it would wait forever (the REPL's
        // reply, a `print`, hung on it).
        match self.state.try_lock() {
            Ok(st) if st.updating => "<cell: updating>".to_string(),
            Ok(st) => match st.ast() {
                Ok(v) => format!("cell({})", v),
                Err(_) => "<cell>".to_string(),
            },
            Err(std::sync::TryLockError::WouldBlock) => "cell(…)".to_string(),
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
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [
        ("new", 1),
        ("get", 1),
        ("take", 1),
        ("set", 2),
        ("update", 2),
    ] {
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
        fields: std::sync::Arc::new(module),
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
        Value::Enum(e) => match &e.variant_data {
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
        "take" => cell_take(args).map_err(raise),
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
            vm: None,
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
    state.ast()
}

/// `cell.take(c)` — the value, moved out; the cell holds `()` until it
/// is set again. Nothing else then holds the value, so extending it
/// (`map_set`, `+ [x]`) is in place rather than a copy.
fn cell_take(args: Vec<Value>) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("cell.take expects one argument: the cell".to_string());
    }
    let cell = cell_of(&args[0])?;
    cell.own_thread()?;
    let mut state = cell.state.lock().map_err(|_| POISONED.to_string())?;
    if state.updating {
        return Err(CellObject::reentrant("taken"));
    }
    state.take_ast()
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
    state.set_ast(value);
    Ok(Value::Unit)
}

/// `cell.new`, `cell.get`, `cell.take` and `cell.set` as the bytecode VM
/// marked `vm` calls them, on its own values: what it sets is kept as it
/// is, and what it set it reads back as it is. `None` for anything else
/// (another operation, not a cell, the wrong thread, an update running):
/// the bridge then runs the call, and says its errors.
pub fn vm_call(op: &str, args: &mut [OvmValue], vm: u64) -> Option<Result<OvmValue, String>> {
    if op == "new" {
        if args.len() != 1 {
            return None;
        }
        let v = std::mem::replace(&mut args[0], OvmValue::new_unit());
        let cell = CellObject {
            owner: std::thread::current().id(),
            owner_label: thread_label(),
            state: Mutex::new(CellState {
                value: Value::Unit,
                vm: Some((v, vm)),
                updating: false,
            }),
        };
        return Some(Ok(OvmValue {
            data: ValueData::Native(NativeHandle::new(cell)),
        }));
    }
    let want = match op {
        "get" | "take" => 1,
        "set" => 2,
        _ => return None,
    };
    if args.len() != want {
        return None;
    }
    let handle = match &args[0].data {
        ValueData::Native(h) => h.clone(),
        _ => return None,
    };
    let cell = handle.0.as_any().downcast_ref::<CellObject>()?;
    if cell.own_thread().is_err() {
        return None;
    }
    let mut state = cell.state.lock().ok()?;
    if state.updating {
        return None;
    }
    // Another VM's value, or the interpreter's: converted, as a value
    // crossing the tiers is (a closure's bytecode belongs to its VM).
    fn mine(slot: Option<(OvmValue, u64)>, vm: u64) -> Result<OvmValue, String> {
        match slot {
            Some((v, o)) if o == vm => Ok(v),
            Some((v, _)) => v
                .to_ast()
                .map(OvmValue::from_ast)
                .map_err(|e| format!("cell: {}", e)),
            None => Ok(OvmValue::new_unit()),
        }
    }
    Some(match op {
        "get" => match &state.vm {
            Some((v, o)) if *o == vm => Ok(v.clone()),
            Some(_) => mine(state.vm.clone(), vm),
            None => Ok(OvmValue::from_ast(state.value.clone())),
        },
        "take" => {
            if state.vm.is_some() {
                mine(state.vm.take(), vm)
            } else {
                Ok(OvmValue::from_ast(std::mem::replace(
                    &mut state.value,
                    Value::Unit,
                )))
            }
        }
        _ => {
            let v = std::mem::replace(&mut args[1], OvmValue::new_unit());
            let old_value = std::mem::replace(&mut state.value, Value::Unit);
            let old_vm = state.vm.replace((v, vm));
            // what the cell held is let go once the lock is (a value
            // holding this very cell takes it on its way out)
            drop(state);
            drop(old_value);
            drop(old_vm);
            Ok(OvmValue::new_unit())
        }
    })
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
        let current = state.ast().map_err(raise)?;
        state.updating = true;
        current
    };

    let applied = interpreter.call_function_optimized(&function, vec![current]);

    // Cleared whether the function returned or raised, so one failed update
    // does not leave the cell permanently unreadable.
    let mut state = cell.state.lock().map_err(|_| raise(POISONED.to_string()))?;
    state.updating = false;
    let next = applied?;
    state.set_ast(next.clone());
    Ok(next)
}

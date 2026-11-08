//! `shared` — one immutable value, readable from every thread.
//!
//! `shared.new(v)` makes a reference any thread can read and replace:
//! `shared.get(s)` answers the value it holds now, `shared.swap(s, f)`
//! replaces it with `f` of it, `shared.compare_and_set(s, old, new)`
//! replaces it only if it is still `old`, and `shared.set(s, v)` replaces
//! it outright (OL-466).
//!
//! This keeps the language's rule that no two threads share mutable
//! data: the value inside never changes. What changes is which value the
//! reference points at, and every reader gets a whole value, the old one
//! or the new one, never a mix. A reader holds its value as long as it
//! likes; a later swap does not touch it.
//!
//! Reads never wait for a writer's work: the reference is an `Arc` behind
//! a lock held only to copy or replace that pointer. `swap` computes
//! `f(current)` outside it and installs the result only if no other
//! writer got in first, trying again with the newer value if one did, so
//! `f` may run more than once and should not have effects.

use crate::ast::Value;
use crate::interpreter::InterpreterError;
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::sync::Arc;

pub struct SharedObject {
    current: parking_lot::RwLock<Arc<Value>>,
}

impl std::fmt::Debug for SharedObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<shared>")
    }
}

impl SharedObject {
    fn load(&self) -> Arc<Value> {
        self.current.read().clone()
    }

    /// Install `new` if the value is still `seen` (the same `Arc`).
    fn install_if(&self, seen: &Arc<Value>, new: Value) -> Result<(), Value> {
        let mut w = self.current.write();
        if Arc::ptr_eq(&w, seen) {
            *w = Arc::new(new);
            Ok(())
        } else {
            Err(new)
        }
    }
}

impl NativeObject for SharedObject {
    fn module(&self) -> &'static str {
        "shared"
    }
    fn type_name(&self) -> &'static str {
        "Shared"
    }
    fn display(&self) -> String {
        format!("<shared: {}>", self.load())
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        // Identity: two references are two places to point.
        other
            .as_any()
            .downcast_ref::<SharedObject>()
            .is_some_and(|o| std::ptr::eq(self, o))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    // Under the write lock no reader can copy the pointer, and only when
    // the reference is its value's sole holder is that value its own: a
    // `swap` in progress still holds the one it read.
    fn trace(&self, visit: &mut dyn FnMut(crate::native::Traced<'_>)) -> bool {
        let Some(current) = self.current.try_write() else {
            return false;
        };
        if Arc::strong_count(&current) != 1 {
            return false;
        }
        visit(crate::native::Traced::Value(&current));
        true
    }

    fn break_cycle(&self) {
        let old = match self.current.try_write() {
            Some(mut w) => std::mem::replace(&mut *w, Arc::new(Value::Unit)),
            None => return,
        };
        drop(old);
    }
}

pub fn create_shared_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [
        ("new", 1),
        ("get", 1),
        ("set", 2),
        ("swap", 2),
        ("compare_and_set", 3),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("shared.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".into(),
        fields: Arc::new(module).into(),
    }
}

fn shared_of<'a>(v: &'a Value, who: &str) -> Result<&'a SharedObject, String> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<SharedObject>().ok_or_else(|| {
            format!(
                "{who}: expected a shared reference (from shared.new), got a {}",
                h.0.type_name()
            )
        }),
        other => Err(format!(
            "{who}: expected a shared reference (from shared.new), got {}",
            other.type_name()
        )),
    }
}

/// A value every thread may read: nothing confined to one thread inside.
fn crossable(v: &Value, who: &str) -> Result<(), String> {
    match crate::stdlib::cell::confined_within(v) {
        Some(kind) => Err(format!(
            "{who}: a {kind} cannot be shared between threads — it belongs to the \
             thread that created it. Share what it holds instead"
        )),
        None => Ok(()),
    }
}

fn raise(message: String) -> InterpreterError {
    InterpreterError::RuntimeError { message }
}

pub fn call_shared_function(
    name: &str,
    args: Vec<Value>,
    interpreter: &mut crate::interpreter::Interpreter,
) -> Result<Value, InterpreterError> {
    let want = match name {
        "new" | "get" => 1,
        "set" | "swap" => 2,
        "compare_and_set" => 3,
        _ => return Err(raise(format!("Unknown shared function: {name}"))),
    };
    if args.len() != want {
        return Err(raise(format!(
            "shared.{name} expects {want} argument{}, got {}",
            if want == 1 { "" } else { "s" },
            args.len()
        )));
    }
    let mut args = args;
    match name {
        "new" => {
            let v = args.pop().expect("len checked");
            crossable(&v, "shared.new").map_err(raise)?;
            let handle = NativeHandle::new(SharedObject {
                current: parking_lot::RwLock::new(Arc::new(v)),
            });
            crate::cycles::track(&handle);
            Ok(Value::Native(handle))
        }
        "get" => Ok((*shared_of(&args[0], "shared.get").map_err(raise)?.load()).clone()),
        "set" => {
            let v = args.pop().expect("len checked");
            crossable(&v, "shared.set").map_err(raise)?;
            let s = shared_of(&args[0], "shared.set").map_err(raise)?;
            *s.current.write() = Arc::new(v);
            Ok(Value::Unit)
        }
        "swap" => {
            let f = args.pop().expect("len checked");
            let s = shared_of(&args[0], "shared.swap").map_err(raise)?;
            loop {
                let seen = s.load();
                let new = interpreter.call_function(f.clone(), vec![(*seen).clone()])?;
                crossable(&new, "shared.swap").map_err(raise)?;
                match s.install_if(&seen, new.clone()) {
                    Ok(()) => return Ok(new),
                    Err(_) => continue,
                }
            }
        }
        "compare_and_set" => {
            let new = args.pop().expect("len checked");
            let expected = args.pop().expect("len checked");
            crossable(&new, "shared.compare_and_set").map_err(raise)?;
            let s = shared_of(&args[0], "shared.compare_and_set").map_err(raise)?;
            let seen = s.load();
            // The value it holds, or one equal to it: a value has no
            // identity a program can rely on, so equal is enough.
            if !(seen.same(&expected) || crate::interpreter::ops::loose_eq(&seen, &expected)) {
                return Ok(Value::Boolean(false));
            }
            Ok(Value::Boolean(s.install_if(&seen, new).is_ok()))
        }
        _ => unreachable!("checked above"),
    }
}

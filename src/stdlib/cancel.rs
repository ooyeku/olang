//! `cancel` — cancellation tokens.
//!
//! A token is a flag that starts clear and can be set once, from any
//! thread: `cancel.token()`, `cancel.set(t)`, `cancel.is_set(t)`. It is
//! how one task tells another that its work is no longer wanted (a newer
//! revision arrived, a query was superseded). Reading it is one atomic
//! load, so work can check it as often as it likes.
//!
//! A token is not shared mutable data in the sense the language forbids:
//! it holds no value, only "asked to stop", and it goes one way. So it
//! crosses `spawn` and channels like any value.
//!
//! **Stopping code that does not check.** `runtime.call_cancellable(f,
//! args, t)` runs `f` so that setting `t` stops it wherever it is: the
//! tree-walker at its next statement, the VM at its next back edge or
//! call, native code at its interrupt polls, and the long builtins (the
//! file walks, `os.exec`, http reads) between their steps. It is the
//! interrupt `runtime.call_budget` uses, aimed at the threads running a
//! cancellable call on that token (`interrupt::cancel_*`).

use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A token: the flag, behind the `Arc` the interrupt's watch list shares.
#[derive(Debug, Default)]
pub struct Token {
    set: AtomicBool,
}

impl Token {
    pub fn is_set(&self) -> bool {
        self.set.load(Ordering::Acquire)
    }
}

/// The token handle: what `cancel.token()` answers.
#[derive(Debug)]
pub struct TokenObject(pub Arc<Token>);

impl NativeObject for TokenObject {
    fn module(&self) -> &'static str {
        "cancel"
    }
    fn type_name(&self) -> &'static str {
        "CancelToken"
    }
    fn display(&self) -> String {
        if self.0.is_set() {
            "<cancel token: set>".to_string()
        } else {
            "<cancel token>".to_string()
        }
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        // Identity: two tokens are two flags.
        other
            .as_any()
            .downcast_ref::<TokenObject>()
            .is_some_and(|o| Arc::ptr_eq(&self.0, &o.0))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn create_cancel_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [("token", 0), ("set", 1), ("is_set", 1)] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("cancel.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

/// The token a value is, or the error naming what it is instead.
pub fn token_of(value: &Value, who: &str) -> Result<Arc<Token>, String> {
    match value {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<TokenObject>()
                .map(|t| t.0.clone())
                .ok_or_else(|| {
                    format!(
                        "{who}: expected a cancel token (from cancel.token()), got a {}",
                        h.0.type_name()
                    )
                })
        }
        other => Err(format!(
            "{who}: expected a cancel token (from cancel.token()), got {}",
            other.type_name()
        )),
    }
}

pub fn call_cancel_function(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match (name, args.as_slice()) {
        ("token", []) => Ok(Value::Native(NativeHandle::new(
            TokenObject(Arc::default()),
        ))),
        ("set", [t]) => {
            let token = token_of(t, "cancel.set")?;
            if !token.set.swap(true, Ordering::AcqRel) {
                crate::interrupt::cancel_set(&token);
            }
            Ok(Value::Unit)
        }
        ("is_set", [t]) => Ok(Value::Boolean(token_of(t, "cancel.is_set")?.is_set())),
        ("token", _) => Err(format!(
            "cancel.token expects no arguments, got {}",
            args.len()
        )),
        ("set" | "is_set", _) => Err(format!(
            "cancel.{name} expects one argument, a cancel token; got {}",
            args.len()
        )),
        _ => Err(format!("Unknown cancel function: {name}")),
    }
}

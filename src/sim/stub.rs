//! `sim` where there is no simulator: the playground (wasm32, no threads,
//! no blocking waits). The same names the runtime asks, answering that no
//! world is running; the surface itself is not in this build's stdlib
//! (`stdlib/mod.rs` registers `sim` on native builds only).

use crate::ast::{Expr, Value};
use crate::interpreter::{Interpreter, InterpreterError};
use rand::rngs::StdRng;
use std::sync::{Arc, Mutex};

/// No world runs here.
pub fn in_world() -> bool {
    false
}

/// The virtual clock: none.
pub fn virtual_epoch_ms() -> Option<i64> {
    None
}

/// A task's seeded stream: none.
pub fn task_rng() -> Option<Arc<Mutex<StdRng>>> {
    None
}

/// Nothing is intercepted.
pub fn intercept(
    _name: &str,
    _args: &[Value],
    _interpreter: &mut Interpreter,
) -> Option<Result<Value, InterpreterError>> {
    None
}

/// Never reached (`in_world` is false), kept for the one call site.
pub fn spawn(_interpreter: &Interpreter, _expr: &Expr) -> Result<Value, InterpreterError> {
    Err(InterpreterError::RuntimeError {
        message: "sim: worlds need a native build".to_string(),
    })
}

/// Seeds a sim test runs under `olang test`: none here.
pub fn test_seeds() -> u64 {
    1
}

pub mod surface {
    use crate::ast::Value;
    use crate::interpreter::{Interpreter, InterpreterError};

    /// `sim.*` in a build without the simulator.
    pub fn call(
        name: &str,
        _args: Vec<Value>,
        _interpreter: &mut Interpreter,
    ) -> Result<Value, InterpreterError> {
        Err(InterpreterError::RuntimeError {
            message: format!("sim.{name}: worlds need a native build"),
        })
    }
}

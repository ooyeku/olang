//! Interrupting a running evaluation from another thread.
//!
//! `olang repl --serve` (src/repl_serve.rs) reads its requests on a
//! thread of their own while the main thread evaluates; an `interrupt`
//! request sets one flag here, and every tier polls it where a program
//! can spin:
//!
//! - the tree-walker at every statement and every loop iteration (its
//!   safepoint poll);
//! - the bytecode VM at every backward jump, every self tail call and
//!   every call's entry;
//! - native code at its back edges and self tail calls, but only when
//!   the process was armed before the code was compiled (`arm`): an
//!   ordinary run compiles exactly the code it always did. A native body
//!   that sees the flag deopts, which re-runs the call on the VM, which
//!   raises the error at its first poll.
//!
//! The error is an ordinary runtime error, "interrupted", so the
//! evaluation unwinds as any failure does and the session's state stays
//! as it was at that point. The flag stays set until the next evaluation
//! clears it, so a program that catches the error and carries on meets
//! it again at its next poll.

use std::sync::atomic::{AtomicBool, Ordering};

static ARMED: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicBool = AtomicBool::new(false);

/// The message every tier raises.
pub const MESSAGE: &str = "interrupted";

/// Compile the native tier's polls from now on (the REPL server arms the
/// process before it evaluates anything).
pub fn arm() {
    ARMED.store(true, Ordering::SeqCst);
}

/// Whether native code compiled now must poll.
#[inline]
pub fn armed() -> bool {
    ARMED.load(Ordering::Relaxed)
}

/// Ask the running evaluation to stop.
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// Forget a request (at the start of an evaluation).
pub fn clear() {
    REQUESTED.store(false, Ordering::SeqCst);
}

/// Whether a stop was asked for: one relaxed load, a predicted-false
/// branch at each poll.
#[inline(always)]
pub fn pending() -> bool {
    REQUESTED.load(Ordering::Relaxed)
}

/// Where the flag lives, for native code to read (a byte: 0 or 1).
pub fn flag_address() -> usize {
    REQUESTED.as_ptr() as usize
}

/// The interrupt as the tree-walker's error, built out of line: the
/// polls stay a load and a branch where they sit.
#[cold]
#[inline(never)]
pub fn interpreter_error() -> crate::interpreter::InterpreterError {
    crate::interpreter::InterpreterError::RuntimeError { message: MESSAGE.to_string() }
}

/// The interrupt as the VM's error, built out of line.
#[cold]
#[inline(never)]
pub fn vm_error() -> crate::ovm::bytecode::BytecodeError {
    crate::ovm::bytecode::BytecodeError::RuntimeError(MESSAGE.to_string())
}

/// Whether an error message is the interrupt's.
pub fn is_interrupt(message: &str) -> bool {
    message == MESSAGE || message.ends_with(": interrupted")
}

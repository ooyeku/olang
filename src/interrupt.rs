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
//!
//! **Budgets.** `runtime.call_budget(f, args, ms)` (a plugin host's
//! call into code it does not trust to finish) runs `f` with a deadline:
//! one watchdog thread, started on the first budget, sets the same flag
//! when the deadline passes — aimed at the thread that made the call, so
//! the rest of the program (a window's tasks on other threads) polls
//! past it. A request with no target (the REPL's interrupt) stops every
//! thread's evaluation, as before. The poll's fast path is unchanged: one
//! relaxed load, false while nothing is asked.
//!
//! **Cancellation.** `runtime.call_cancellable(f, args, token)` runs `f`
//! watching a `cancel` token: setting the token stops the threads running
//! a cancellable call on it, through the same flag and polls. A thread
//! watching nothing, or a token not set, polls past it.

use crate::clock::Instant;
use crate::stdlib::cancel::Token;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

static ARMED: AtomicBool = AtomicBool::new(false);
/// The polls' flag: true while anything may be asking some thread to
/// stop (an interrupt, a budget's trip, a set token a cancellable call
/// watches). Only `pending`'s slow path asks whether it is this thread.
/// Written only under `FLAG_LOCK`, by `refresh` or with the interrupt.
static REQUESTED: AtomicBool = AtomicBool::new(false);
/// An interrupt or a budget's trip stands, for `TARGET`.
static INTERRUPT: AtomicBool = AtomicBool::new(false);
static FLAG_LOCK: Mutex<()> = Mutex::new(());
/// The thread a request is for (a number of `thread_no`), 0 for every one.
static TARGET: AtomicU64 = AtomicU64::new(0);
static NEXT_THREAD: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static THREAD_NO: u64 = NEXT_THREAD.fetch_add(1, Ordering::Relaxed);
    /// The tokens this thread's cancellable calls watch, outermost first.
    static WATCHING: std::cell::RefCell<Vec<Arc<Token>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Every cancellable call running: (its thread, its token).
static WATCHES: Mutex<Vec<(u64, Arc<Token>)>> = Mutex::new(Vec::new());

/// Set the polls' flag from what stands: the interrupt, or a watched
/// token that is set.
fn refresh() {
    let _g = FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cancelled = WATCHES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|(_, t)| t.is_set());
    REQUESTED.store(
        INTERRUPT.load(Ordering::SeqCst) || cancelled,
        Ordering::SeqCst,
    );
}

/// Raise the interrupt for `target` (0: every thread).
fn raise_interrupt(target: u64) {
    let _g = FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    TARGET.store(target, Ordering::SeqCst);
    INTERRUPT.store(true, Ordering::SeqCst);
    REQUESTED.store(true, Ordering::SeqCst);
}

/// This thread's number (never 0): what a budget's trip is aimed at.
pub fn thread_no() -> u64 {
    THREAD_NO.with(|n| *n)
}

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

/// Ask the running evaluation to stop (every thread's).
pub fn request() {
    raise_interrupt(0);
}

/// Forget a request (at the start of an evaluation). A cancellable call
/// still running keeps its token's say.
pub fn clear() {
    INTERRUPT.store(false, Ordering::SeqCst);
    TARGET.store(0, Ordering::SeqCst);
    refresh();
}

/// Whether a stop was asked for this thread: one relaxed load, a
/// predicted-false branch at each poll; only a request standing asks
/// whom it is for.
#[inline(always)]
pub fn pending() -> bool {
    REQUESTED.load(Ordering::Relaxed) && aimed_here()
}

#[cold]
#[inline(never)]
fn aimed_here() -> bool {
    if INTERRUPT.load(Ordering::SeqCst) {
        let t = TARGET.load(Ordering::SeqCst);
        if t == 0 || t == thread_no() {
            return true;
        }
    }
    WATCHING.with(|w| w.borrow().iter().any(|t| t.is_set()))
}

// ── cancellation ────────────────────────────────────────────────────
/// The message a cancelled call's error carries once caught.
pub const CANCEL_MESSAGE: &str = "cancelled";

/// A cancellable call begun on this thread: once `token` is set, this
/// thread's evaluation is stopped as an interrupt is, until
/// `cancel_end`. Arms the process, so native code compiled from now on
/// polls too.
pub fn cancel_begin(token: Arc<Token>) {
    arm();
    WATCHING.with(|w| w.borrow_mut().push(token.clone()));
    WATCHES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((thread_no(), token));
    refresh();
}

/// The cancellable call begun last on this thread ended: whether its
/// token was set.
pub fn cancel_end() -> bool {
    let me = thread_no();
    let token = WATCHING.with(|w| w.borrow_mut().pop());
    if let Some(token) = &token {
        let mut watches = WATCHES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = watches
            .iter()
            .rposition(|(t, k)| *t == me && Arc::ptr_eq(k, token))
        {
            watches.remove(i);
        }
    }
    refresh();
    token.is_some_and(|t| t.is_set())
}

/// `cancel.set(token)` just set it: the threads watching it stop at their
/// next poll.
pub fn cancel_set(_token: &Arc<Token>) {
    refresh();
}

/// Whether this thread runs inside a cancellable call whose token is
/// set, or was asked to stop otherwise: what a long builtin checks
/// between its steps (the same answer the evaluators' polls get).
pub fn should_stop() -> bool {
    pending()
}

/// This thread's stop question, for helper threads working on its behalf
/// (a file walk's workers): it asks what `should_stop` would ask here.
#[derive(Clone)]
pub struct StopProbe {
    thread: u64,
    tokens: Vec<Arc<Token>>,
}

impl StopProbe {
    pub fn here() -> StopProbe {
        StopProbe {
            thread: thread_no(),
            tokens: WATCHING.with(|w| w.borrow().clone()),
        }
    }

    #[inline]
    pub fn should_stop(&self) -> bool {
        if !REQUESTED.load(Ordering::Relaxed) {
            return false;
        }
        if INTERRUPT.load(Ordering::SeqCst) {
            let t = TARGET.load(Ordering::SeqCst);
            if t == 0 || t == self.thread {
                return true;
            }
        }
        self.tokens.iter().any(|t| t.is_set())
    }
}

// ── budgets ─────────────────────────────────────────────────────────
/// The message a call over its budget raises (as `interrupted` does).
pub const BUDGET_MESSAGE: &str = "interrupted: over its time budget";

struct Watch {
    /// Bumped by every begin and end: a deadline the watchdog read is
    /// stale once it changes.
    generation: u64,
    /// The budgets running, outermost first: (deadline, thread).
    stack: Vec<(Instant, u64)>,
    /// The generation that tripped, when the top budget did.
    tripped: Option<u64>,
}

fn watch() -> &'static (Mutex<Watch>, Condvar) {
    static W: OnceLock<(Mutex<Watch>, Condvar)> = OnceLock::new();
    W.get_or_init(|| {
        std::thread::Builder::new()
            .name("olang-budget".into())
            .spawn(watchdog)
            .expect("the budget watchdog starts");
        (
            Mutex::new(Watch {
                generation: 0,
                stack: Vec::new(),
                tripped: None,
            }),
            Condvar::new(),
        )
    })
}

fn watchdog() {
    let (lock, cv) = watch();
    let mut w = lock.lock().unwrap();
    loop {
        match w.stack.last().copied() {
            None => {
                w = cv.wait(w).unwrap();
            }
            Some((deadline, thread)) => {
                let now = Instant::now();
                if now >= deadline {
                    if w.tripped != Some(w.generation) {
                        w.tripped = Some(w.generation);
                        raise_interrupt(thread);
                    }
                    w = cv.wait(w).unwrap();
                } else {
                    w = cv.wait_timeout(w, deadline - now).unwrap().0;
                }
            }
        }
    }
}

/// A budget begun: the calling thread's evaluation is stopped (as an
/// interrupt) once `ms` pass, unless `budget_end` comes first. Arms the
/// process, so native code compiled from now on polls too.
pub fn budget_begin(ms: u64) {
    arm();
    let (lock, cv) = watch();
    let mut w = lock.lock().unwrap();
    w.generation += 1;
    let deadline = Instant::now() + Duration::from_millis(ms.max(1));
    w.stack.push((deadline, thread_no()));
    cv.notify_all();
}

/// The budget begun last on this thread ended: whether it ran out. A
/// trip aimed here is forgotten, so the code after the call runs on.
pub fn budget_end() -> bool {
    let (lock, cv) = watch();
    let mut w = lock.lock().unwrap();
    let tripped = w.tripped == Some(w.generation);
    w.stack.pop();
    w.generation += 1;
    if tripped {
        w.tripped = None;
        if TARGET.load(Ordering::SeqCst) == thread_no() {
            INTERRUPT.store(false, Ordering::SeqCst);
            TARGET.store(0, Ordering::SeqCst);
            refresh();
        }
    }
    cv.notify_all();
    tripped
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
    crate::interpreter::InterpreterError::RuntimeError {
        message: MESSAGE.to_string(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_trips_only_its_own_thread() {
        budget_begin(5);
        let other = std::thread::spawn(|| {
            crate::clock::sleep(Duration::from_millis(30));
            pending()
        });
        let start = Instant::now();
        while !pending() && start.elapsed() < Duration::from_secs(2) {
            std::hint::spin_loop();
        }
        assert!(pending(), "the budget's thread sees the trip");
        assert!(!other.join().unwrap(), "another thread polls past it");
        assert!(budget_end(), "the budget says it ran out");
        assert!(!pending(), "the trip is forgotten once the budget ends");
        budget_begin(10_000);
        assert!(!budget_end(), "a budget that did not run out says so");
    }
}

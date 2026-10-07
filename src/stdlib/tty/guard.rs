//! The restore guard: the terminal comes back however the process ends.
//!
//! `tty.enter` *arms* the guard with a snapshot of everything it is about
//! to change — the terminal's original settings and the bytes that undo
//! each mode it turns on. Restoring is one operation, [`restore`]: take
//! the armed snapshot (an atomic swap, so exactly one caller gets it),
//! write its leave bytes, put the settings back. Every exit path calls
//! it, and none of them depends on the olang program cooperating:
//!
//! | how the process ends | what restores |
//! |---|---|
//! | `tty.leave(h)` | `leave`, then the handle is spent |
//! | the program finishes, or raises uncaught | the CLI, as soon as the program's evaluation returns — *before* any error is printed, so the error lands on the main screen |
//! | `os.exit(n)` | `os.exit`, before it exits |
//! | any other `exit(3)` (a stall abort, a broken pipe) | an `atexit` handler (Unix) |
//! | a Rust panic, in any thread | the panic hook, before the panic message (also under `panic = "abort"`) |
//! | SIGTERM, SIGHUP, SIGINT | the signal action: restore, then die of the signal as the default action would — unless the program trapped the signal with `os.on_interrupt` / `os.on_shutdown`, in which case the signal is the program's (it arrives as a `signal` event too) |
//! | SIGTSTP (ctrl+z with `raw: false`, `kill -TSTP`, `tty.suspend`) | the signal action restores and stops; SIGCONT re-enters |
//!
//! The snapshot is plain data read with an atomic load, and restoring is
//! `write(2)` and `tcsetattr(3)`, both async-signal-safe, so the signal
//! actions and the `atexit` handler restore directly, whatever the
//! program's threads were doing. The guard is installed once, on the
//! first `tty.enter`; a program that never enters pays nothing.
//!
//! What cannot be guarded: SIGKILL and a power cut (nothing runs), and a
//! stack overflow (the runtime aborts without a hook). `reset` in the
//! shell is the answer there.
// The signal path (SIG_*, take_signals, resume) is Unix's; Windows keys
// and resizes arrive as console records instead.
#![cfg_attr(windows, allow(dead_code))]

use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

/// What restoring needs, and what re-entering after a stop needs.
pub(crate) struct Snapshot {
    /// Bytes that undo every mode `enter` turned on.
    pub leave: Vec<u8>,
    /// Bytes that turn them on again (after SIGCONT).
    pub enter: Vec<u8>,
    #[cfg(unix)]
    pub saved: libc::termios,
    #[cfg(unix)]
    pub raw: Option<libc::termios>,
    #[cfg(windows)]
    pub raw: bool,
    #[cfg(windows)]
    pub mouse: bool,
}

/// The armed snapshot: non-null exactly while the terminal is entered.
static ARMED: AtomicPtr<Snapshot> = AtomicPtr::new(std::ptr::null_mut());
/// A snapshot restored by a stop (SIGTSTP), waiting for SIGCONT.
static SUSPENDED: AtomicPtr<Snapshot> = AtomicPtr::new(std::ptr::null_mut());

/// Signals seen since the reader last looked, as bits (see `SIG_*`).
static PENDING: AtomicU32 = AtomicU32::new(0);
pub(crate) const SIG_WINCH: u32 = 1;
pub(crate) const SIG_CONT: u32 = 2;
pub(crate) const SIG_TSTP: u32 = 4;
pub(crate) const SIG_TERM: u32 = 8;
pub(crate) const SIG_HUP: u32 = 16;
pub(crate) const SIG_INT: u32 = 32;

/// The signals seen since the last call, cleared.
pub(crate) fn take_signals() -> u32 {
    PENDING.swap(0, Ordering::SeqCst)
}

/// Arm the guard with `snap`. The caller then changes the terminal; a
/// restore that races the change restores to the original settings,
/// which is what they still are.
///
/// Snapshots are never freed: one is a few hundred bytes per `enter`,
/// and never freeing is what makes every reader of the pointers (signal
/// actions, the panic hook, the reader thread after SIGCONT) safe
/// without a lock.
pub(crate) fn arm(snap: Snapshot) {
    let p: *mut Snapshot = Box::leak(Box::new(snap));
    ARMED.store(p, Ordering::SeqCst);
}

/// Is the terminal entered right now?
pub(crate) fn armed() -> bool {
    !ARMED.load(Ordering::SeqCst).is_null()
}

/// Restore the terminal if it is entered; idempotent. Safe to call from
/// any thread, a panic hook, or `atexit`. Returns whether it restored.
pub fn restore() -> bool {
    // A snapshot a stop already restored is simply dropped from view.
    SUSPENDED.store(std::ptr::null_mut(), Ordering::SeqCst);
    let p = ARMED.swap(std::ptr::null_mut(), Ordering::SeqCst);
    if p.is_null() {
        return false;
    }
    // SAFETY: snapshots are leaked, never freed (see `arm`).
    apply_leave(unsafe { &*p });
    true
}

/// Undo `snap`'s modes. Async-signal-safe on Unix.
fn apply_leave(snap: &Snapshot) {
    #[cfg(unix)]
    {
        super::unix::write_fd(libc::STDOUT_FILENO, &snap.leave);
        // SAFETY: tcsetattr on stdin with a termios read from it.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &snap.saved);
        }
    }
    #[cfg(windows)]
    super::windows::leave(snap);
}

/// Re-apply `snap`'s modes (after a stop).
fn apply_enter(snap: &Snapshot) {
    #[cfg(unix)]
    {
        if let Some(raw) = &snap.raw {
            // SAFETY: as in apply_leave.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, raw);
            }
        }
        super::unix::write_fd(libc::STDOUT_FILENO, &snap.enter);
    }
    #[cfg(windows)]
    super::windows::enter(snap);
}

/// After SIGCONT: re-enter what a stop restored. `true` if this call
/// re-entered (and so should announce it); a second caller finds nothing.
/// A process stopped by SIGSTOP (not through us) keeps its snapshot
/// armed; its raw settings are re-asserted in case the shell reset them.
pub(crate) fn resume() -> bool {
    let p = SUSPENDED.swap(std::ptr::null_mut(), Ordering::SeqCst);
    if !p.is_null() {
        // SAFETY: snapshots are leaked, never freed (see `arm`).
        apply_enter(unsafe { &*p });
        ARMED.store(p, Ordering::SeqCst);
        return true;
    }
    #[cfg(unix)]
    {
        let a = ARMED.load(Ordering::SeqCst);
        if !a.is_null() {
            // SAFETY: snapshots are leaked, never freed (see `arm`).
            if let Some(raw) = unsafe { &(*a).raw } {
                unsafe {
                    libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, raw);
                }
            }
        }
    }
    false
}

/// Install the guard's process-wide pieces, once: the panic hook, and on
/// Unix the `atexit` handler and the signal actions.
pub(crate) fn install() -> Result<(), String> {
    use std::sync::OnceLock;
    static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                // The terminal first, so the panic message is readable
                // on the main screen.
                restore();
                previous(info);
            }));
            #[cfg(unix)]
            unix_install()?;
            Ok(())
        })
        .clone()
}

#[cfg(unix)]
extern "C" fn at_exit() {
    restore_in_handler();
}

/// The async-signal-safe restore: no allocation, no locks, no free (the
/// process is ending, or stopping with the snapshot parked).
#[cfg(unix)]
fn restore_in_handler() -> *mut Snapshot {
    let p = ARMED.swap(std::ptr::null_mut(), Ordering::SeqCst);
    if !p.is_null() {
        // SAFETY: snapshots are leaked, never freed (see `arm`); reading
        // one allocates nothing.
        apply_leave(unsafe { &*p });
    }
    p
}

#[cfg(unix)]
fn unix_install() -> Result<(), String> {
    use signal_hook::consts::signal::*;
    use signal_hook::low_level;

    super::unix::wake_pipe().map_err(|e| format!("tty: cannot create the wake pipe: {e}"))?;
    // SAFETY: atexit with a plain extern "C" fn.
    unsafe {
        libc::atexit(at_exit);
    }

    // SAFETY (all registrations): every action below is async-signal-safe:
    // atomics, write(2) to a pipe and the terminal, tcsetattr(3), and
    // signal-hook's emulate_default_handler (which is documented for use
    // inside a handler).
    let fatal = |sig: i32, bit: u32| {
        move || {
            note(bit);
            if crate::stdlib::os::signals_trapped() {
                return; // the program's own handler owns this signal
            }
            restore_in_handler();
            let _ = low_level::emulate_default_handler(sig);
        }
    };
    let reg = |sig: i32, action: Box<dyn Fn() + Send + Sync>| -> Result<(), String> {
        unsafe { low_level::register(sig, action) }
            .map(|_| ())
            .map_err(|e| format!("tty: cannot install the signal guard: {e}"))
    };
    reg(SIGTERM, Box::new(fatal(SIGTERM, SIG_TERM)))?;
    reg(SIGHUP, Box::new(fatal(SIGHUP, SIG_HUP)))?;
    reg(SIGINT, Box::new(fatal(SIGINT, SIG_INT)))?;
    reg(
        SIGTSTP,
        Box::new(|| {
            note(SIG_TSTP);
            let p = restore_in_handler();
            if !p.is_null() {
                SUSPENDED.store(p, Ordering::SeqCst);
            }
            // The default action of SIGTSTP: stop (as SIGSTOP).
            let _ = low_level::emulate_default_handler(SIGTSTP);
        }),
    )?;
    reg(SIGCONT, Box::new(|| note(SIG_CONT)))?;
    reg(SIGWINCH, Box::new(|| note(SIG_WINCH)))?;
    Ok(())
}

/// Record a signal for the reader and wake it.
#[cfg(unix)]
fn note(bit: u32) {
    PENDING.fetch_or(bit, Ordering::SeqCst);
    super::unix::wake();
}

/// Record a signal for the reader and wake it (Windows has no signals;
/// kept so the shared code reads the same).
#[cfg(not(unix))]
#[allow(dead_code)]
fn note(bit: u32) {
    PENDING.fetch_or(bit, Ordering::SeqCst);
}

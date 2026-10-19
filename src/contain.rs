//! Containing a panic: code that must never take the process down (a
//! callback the platform makes into olang, a request a server answers,
//! the `gui` engine's builtins) runs under [`catch`], and a panic inside
//! it becomes an `Err` carrying what panicked and where.
//!
//! Everywhere else a panic still ends the process as it always has: the
//! binary unwinds (so `catch` can work), but olang's panic hook
//! ([`crate::stdio::set_panic_hook`]) aborts at once after saying the
//! panic when no `catch` is around it on that thread ([`active`]). A
//! panic on a task's thread is never left to unwind silently while the
//! program waits on that task.
//!
//! A test (or a person chasing a report) can make named places panic on
//! purpose: [`arm`], or `OLANG_TEST_PANIC=site,site` in the environment,
//! and [`inject`] at each place panics the first time it is reached.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, OnceLock};

thread_local! {
    /// How many `catch`es this thread is inside.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    /// What the hook saw of the last contained panic: its message and
    /// where it happened.
    static SEEN: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Whether a panic on this thread now would be caught by a [`catch`].
pub fn active() -> bool {
    DEPTH.with(|d| d.get() > 0)
}

/// The panic hook's part: keep what a contained panic said (`text`, with
/// its place) for the `catch` that will take it.
pub fn note(text: String) {
    SEEN.with(|s| *s.borrow_mut() = Some(text));
}

/// Run `f`; a panic inside it is caught and answered as `Err(what
/// panicked, at file:line)`. `what` names the place for that text.
pub fn catch<R>(what: &str, f: impl FnOnce() -> R) -> Result<R, String> {
    catch_named(|| what.to_string(), f)
}

/// [`catch`], the place's name made only when something panicked (a hot
/// path pays no allocation).
pub fn catch_named<R>(what: impl FnOnce() -> String, f: impl FnOnce() -> R) -> Result<R, String> {
    DEPTH.with(|d| d.set(d.get() + 1));
    let r = catch_unwind(AssertUnwindSafe(f));
    DEPTH.with(|d| d.set(d.get() - 1));
    r.map_err(|payload| {
        let seen = SEEN.with(|s| s.borrow_mut().take());
        let message = seen.unwrap_or_else(|| {
            if let Some(m) = payload.downcast_ref::<&str>() {
                (*m).to_string()
            } else if let Some(m) = payload.downcast_ref::<String>() {
                m.clone()
            } else {
                "a panic".to_string()
            }
        });
        format!("{}: {message}", what())
    })
}

/// The message and place of a panic, as the hook says it.
pub fn describe(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = if let Some(m) = payload.downcast_ref::<&str>() {
        (*m).to_string()
    } else if let Some(m) = payload.downcast_ref::<String>() {
        m.clone()
    } else {
        "a panic".to_string()
    };
    match info.location() {
        Some(l) => format!("{message} (at {}:{}:{})", l.file(), l.line(), l.column()),
        None => message,
    }
}

fn armed() -> &'static Mutex<HashSet<String>> {
    static ARMED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ARMED.get_or_init(|| {
        let from_env = std::env::var("OLANG_TEST_PANIC")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Mutex::new(from_env)
    })
}

/// Make `site` panic the next time it is reached ([`inject`]).
pub fn arm(site: &str) {
    ARMED_BY_CALL.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut a) = armed().lock() {
        a.insert(site.to_string());
    }
}

/// Whether any place may be armed (a test's): when not, [`inject`] is
/// never needed (a hot path asks this before it names its place).
#[inline]
pub fn any_armed() -> bool {
    // one load after the first: nothing armed is the common case
    static ANY: OnceLock<bool> = OnceLock::new();
    *ANY.get_or_init(|| std::env::var_os("OLANG_TEST_PANIC").is_some())
        || ARMED_BY_CALL.load(std::sync::atomic::Ordering::Relaxed)
}

/// A place a test can make panic: panics once when `site` is armed.
#[inline]
pub fn inject(site: &str) {
    if !any_armed() {
        return;
    }
    let hit = armed().lock().map(|mut a| a.remove(site)).unwrap_or(false);
    if hit {
        panic!("a panic injected at {site} (a test's)");
    }
}

static ARMED_BY_CALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_inside_catch_is_an_error_with_its_place() {
        let r: Result<(), String> = catch("here", || panic!("boom"));
        let e = r.unwrap_err();
        assert!(e.starts_with("here: boom"), "{e}");
        assert!(!active());
        assert_eq!(catch("there", || 7).unwrap(), 7);
    }

    #[test]
    fn an_armed_site_panics_once() {
        arm("contain.test.site");
        let first = catch("t", || inject("contain.test.site"));
        assert!(first.is_err());
        let second = catch("t", || inject("contain.test.site"));
        assert!(second.is_ok());
    }
}

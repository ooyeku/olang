//! Whether a real window can be seen, for the gui_macos real-window tests
//! (OL-410).
//!
//! A locked screen, a session that isn't the console's (another user
//! switched in) or a sleeping display presents no frames, so a window
//! test can't see what it tests. Each test asks [`skip_if_unseen`] before
//! it opens its window, holds the display awake ([`keep_awake`]), and
//! calls [`guard`] after olang's panic hook: a failure, or the test's own
//! time running out ([`timed_out`]), while the screen can't be seen is
//! said as a skip with its reason, not a SIGABRT that fails an unattended
//! verify run.
//!
//! Not a test target: cargo only builds `tests/*.rs` and `tests/*/main.rs`.

#![allow(dead_code)]

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::NSString;
use std::ffi::c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGSessionCopyCurrentDictionary() -> *mut c_void;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// The session's flag `key` (a CFBoolean or CFNumber), or `None` when the
/// session has no such key or no session dictionary answers (no window
/// server: an ssh login, a launch daemon).
fn session_flag(key: &str) -> Option<bool> {
    // SAFETY: CGSessionCopyCurrentDictionary returns a CFDictionary we own
    // (or null); a CFDictionary is toll-free bridged to NSDictionary, and
    // it is released once read.
    unsafe {
        let dict = CGSessionCopyCurrentDictionary();
        if dict.is_null() {
            return None;
        }
        let d = &*(dict as *const AnyObject);
        let k = NSString::from_str(key);
        let v: Option<Retained<AnyObject>> = msg_send![d, objectForKey: &*k];
        let flag = v.map(|v| {
            let b: bool = msg_send![&*v, boolValue];
            b
        });
        CFRelease(dict);
        flag
    }
}

/// Why a window can't be seen now, or `None` when it can.
/// `GUI_TEST_UNSEEN=<why>` answers that reason instead, so the skip can be
/// tried without locking the screen.
pub fn why_unseen() -> Option<String> {
    if let Ok(why) = std::env::var("GUI_TEST_UNSEEN")
        && !why.is_empty()
    {
        return Some(why);
    }
    Some(why_screen_unseen()?.to_string())
}

fn why_screen_unseen() -> Option<&'static str> {
    // SAFETY: none of these take or return pointers we keep.
    unsafe {
        let dict = CGSessionCopyCurrentDictionary();
        if dict.is_null() {
            return Some("no window server session (not a desktop login)");
        }
        CFRelease(dict);
    }
    if session_flag("CGSSessionScreenIsLocked") == Some(true) {
        return Some("the screen is locked");
    }
    if session_flag("kCGSSessionOnConsoleKey") == Some(false) {
        return Some("this session isn't the one on the screen");
    }
    // SAFETY: plain CoreGraphics queries, no pointers.
    if unsafe { CGDisplayIsAsleep(CGMainDisplayID()) } != 0 {
        return Some("the display is asleep");
    }
    None
}

fn say_skipped(test: &str, why: &str) {
    println!(
        "{test}: skipped ({why}, so its window presents no frames; it needs an unlocked, awake display)"
    );
}

/// Said and `true` when a window can't be seen now: the test returns
/// before it opens one.
pub fn skip_if_unseen(test: &str) -> bool {
    match why_unseen() {
        Some(why) => {
            say_skipped(test, &why);
            true
        }
        None => false,
    }
}

/// The display kept awake for as long as this process lives: an idle
/// display sleeps and stops presenting frames.
pub fn keep_awake() {
    let _ = std::process::Command::new("/usr/bin/caffeinate")
        .args(["-d", "-w", &std::process::id().to_string()])
        .spawn();
}

/// After olang's panic hook is set: a panic while the screen can't be
/// seen (it locked or slept as the test ran, so a first frame or a later
/// one never came) ends the process as a skip, said with why. Any other
/// panic goes to the hook before it, as it did.
pub fn guard(test: &'static str) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(why) = why_unseen() {
            say_skipped(test, &why);
            eprintln!("{test}: (what failed: {info})");
            std::process::exit(0);
        }
        prev(info)
    }));
}

/// The test's own time ran out: a skip when the screen can't be seen,
/// else the failure it was (status 2).
pub fn timed_out(test: &str) -> ! {
    if let Some(why) = why_unseen() {
        say_skipped(test, &why);
        std::process::exit(0);
    }
    eprintln!("{test}: timed out");
    std::process::exit(2);
}

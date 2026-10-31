//! What is held: every key and mouse button down in any of the process's
//! windows, and what was pressed and released since it was last read.
//! Windows hand keys to the focused node as chords; a game wants the
//! keyboard as a set of switches, read once a step (Toad's input reads
//! this). A press and release between two reads still counts once each.
//! A window losing focus lets go of everything (no key stays stuck down).
//! A scripted run (`scripted`) hears only its script: the devices'
//! presses are ignored while it is on, so a demo or a test plays the same
//! whoever is at the machine.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static SCRIPTED: AtomicBool = AtomicBool::new(false);

/// Hear only the script's hands (`press`, `release`), not the devices'.
pub fn scripted(on: bool) {
    SCRIPTED.store(on, Ordering::SeqCst);
}

pub fn is_scripted() -> bool {
    SCRIPTED.load(Ordering::SeqCst)
}

/// A device's key or button went down (a window's, a gamepad's).
pub fn device_press(name: &str) {
    if !is_scripted() {
        press(name);
    }
}

/// A device's key or button came up.
pub fn device_release(name: &str) {
    if !is_scripted() {
        release(name);
    }
}

static POINTER: Mutex<Option<(f32, f32)>> = Mutex::new(None);

/// The pointer moved to `(x, y)` in a window (points from its top left).
pub fn device_pointer(x: f32, y: f32) {
    if !is_scripted() {
        *POINTER.lock().unwrap_or_else(|e| e.into_inner()) = Some((x, y));
    }
}

/// Where the pointer last was in a window, if it has been in one.
pub fn pointer() -> Option<(f32, f32)> {
    *POINTER.lock().unwrap_or_else(|e| e.into_inner())
}

/// A device let go of everything (a window lost focus, a pad went away).
pub fn device_release_all() {
    if !is_scripted() {
        release_all();
    }
}

#[derive(Default)]
struct Held {
    down: HashSet<String>,
    pressed: HashMap<String, u32>,
    released: HashMap<String, u32>,
}

static HELD: Mutex<Option<Held>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Held) -> T) -> T {
    let mut g = HELD.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Held::default))
}

/// `name` went down (a key repeating is not a new press).
pub fn press(name: &str) {
    with(|h| {
        if h.down.insert(name.to_string()) {
            *h.pressed.entry(name.to_string()).or_default() += 1;
        }
    });
}

/// `name` came up.
pub fn release(name: &str) {
    with(|h| {
        if h.down.remove(name) {
            *h.released.entry(name.to_string()).or_default() += 1;
        }
    });
}

/// Exactly `names` are down, and nothing went down or up: where a
/// replay left the keys (a rewind, a reload), so a key held across it is
/// not pressed again.
pub fn hold(names: &[String]) {
    with(|h| {
        h.down = names.iter().cloned().collect();
        h.pressed.clear();
        h.released.clear();
    });
}

/// Everything comes up (a window lost focus).
pub fn release_all() {
    with(|h| {
        for k in std::mem::take(&mut h.down) {
            *h.released.entry(k).or_default() += 1;
        }
    });
}

/// What is down now, and what was pressed and released since the last
/// read (each once however often), sorted.
pub fn read() -> (Vec<String>, Vec<String>, Vec<String>) {
    with(|h| {
        let mut down: Vec<String> = h.down.iter().cloned().collect();
        let mut pressed: Vec<String> = h.pressed.drain().map(|(k, _)| k).collect();
        let mut released: Vec<String> = h.released.drain().map(|(k, _)| k).collect();
        down.sort();
        pressed.sort();
        released.sort();
        (down, pressed, released)
    })
}

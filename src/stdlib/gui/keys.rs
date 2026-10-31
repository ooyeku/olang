//! What is held: every key and mouse button down in any of the process's
//! windows, and what was pressed and released since it was last read.
//! Windows hand keys to the focused node as chords; a game wants the
//! keyboard as a set of switches, read once a step (Toad's input reads
//! this). A press and release between two reads still counts once each.
//! A window losing focus lets go of everything (no key stays stuck down).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

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

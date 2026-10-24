//! How long the engine's locks are held and waited for (`GUI_HOLDS=1`, a
//! measurement's): a window's state held by `gui.apply`, the text system
//! held by a display list, the main thread waiting for a window's state,
//! `gui.measure` waiting for the text system. Off, a site costs one load
//! of a flag.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// 0 not read yet, 1 off, 2 on.
static ON: AtomicU8 = AtomicU8::new(0);

/// Whether holds are measured.
pub fn on() -> bool {
    match ON.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var_os("GUI_HOLDS").is_some_and(|v| v != "0");
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

type Samples = HashMap<&'static str, Vec<f32>>;

static SAMPLES: Mutex<Option<Samples>> = Mutex::new(None);

/// A hold (or a wait) at `site`, measured.
pub fn record(site: &'static str, d: Duration) {
    if !on() {
        return;
    }
    let mut s = SAMPLES.lock().unwrap_or_else(|e| e.into_inner());
    let v = s.get_or_insert_with(HashMap::new).entry(site).or_default();
    if v.len() < 1 << 20 {
        v.push(d.as_secs_f32() * 1000.0);
    }
}

/// A clock started where measuring, else nothing (no clock read).
pub fn start() -> Option<Instant> {
    on().then(Instant::now)
}

/// The time since `t`, recorded at `site`.
pub fn since(site: &'static str, t: Option<Instant>) {
    if let Some(t) = t {
        record(site, t.elapsed());
    }
}

/// Every site's samples (ms), taken (the next measurement starts empty).
pub fn take() -> Vec<(&'static str, Vec<f32>)> {
    let mut s = SAMPLES.lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<_> = s.take().unwrap_or_default().into_iter().collect();
    out.sort_by_key(|(k, _)| *k);
    out
}

/// One line a site: how many, p50, p99 and the most (ms).
pub fn summary(samples: &[(&'static str, Vec<f32>)]) -> String {
    let mut out = String::new();
    for (site, v) in samples {
        let mut v = v.clone();
        if v.is_empty() {
            continue;
        }
        v.sort_by(|a, b| a.total_cmp(b));
        let at = |q: f32| v[((v.len() - 1) as f32 * q).round() as usize];
        out.push_str(&format!(
            "{site}: n {} p50 {:.3} p99 {:.3} max {:.3} ms\n",
            v.len(),
            at(0.5),
            at(0.99),
            at(1.0)
        ));
    }
    out
}

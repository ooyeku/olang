//! Warm start: the tier profile a run leaves behind, and the head start
//! the next run of the same program takes from it.
//!
//! olang's domain is file-first scripts — processes are born cold and
//! die young, which is where JIT economics are worst: every run
//! re-learns which functions matter and re-specializes them at first
//! call, on the critical path. A finished run therefore records what it
//! learned — which named functions ran native, on which scalar argument
//! kinds, and how often — keyed by a hash of the program's source. The
//! next run of byte-identical source replays that knowledge: proven-hot
//! functions compile and specialize at declaration time, so their first
//! call (a server's first request, a pipeline's first batch) runs
//! native instead of paying the compile right then.
//!
//! Profiles are *hints*, never authority. A stale or wrong profile
//! costs at most a wasted compile of a function the run never calls —
//! and the source hash makes even that rare, since any edit
//! invalidates the key. Nothing here can change a result: the tier's
//! own qualification and specialization run exactly as they would
//! have, just earlier.
//!
//! Stored under `~/.olang/warm/`, one small TOML per program
//! (`OLANG_WARM_DIR` overrides the location, which is how tests
//! isolate; `OLANG_WARM=0` disables both sides for A/B measurement).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// A function that finished a previous run native, with what the JIT
/// knew about it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WarmFn {
    pub name: String,
    /// The specialization's parameter kinds, when every one is a plain
    /// scalar (`Int` / `Float` / `Bool`). Pointer-carrying kinds
    /// (structs, lists, strings) depend on runtime identities that do
    /// not survive the process, so those functions record an empty list
    /// — they still pre-compile to bytecode, and specialize on their
    /// first call as usual.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Native calls observed — the evidence the pre-compile is not
    /// wasted work.
    #[serde(default)]
    pub native_calls: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WarmProfile {
    #[serde(default)]
    pub functions: Vec<WarmFn>,
}

/// Native calls a function must have made before its knowledge is worth
/// replaying. Below this, first-call specialization was cheap enough
/// the first time.
pub const WARM_MIN_CALLS: u64 = 32;

fn disabled() -> bool {
    matches!(std::env::var("OLANG_WARM").as_deref(), Ok("0"))
}

fn warm_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("OLANG_WARM_DIR") {
        return Some(PathBuf::from(dir));
    }
    crate::home::warm()
}

fn key_for(source: &str) -> String {
    let mut h = Sha256::new();
    h.update(source.as_bytes());
    let digest = h.finalize();
    digest
        .iter()
        .take(8)
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// The profile `olang build` embedded in the running binary, when this
/// process is a built artifact that carried one. A sidecar profile for
/// the same source wins over it — a real run on this machine is fresher
/// evidence than build-time knowledge — so this is `load`'s fallback,
/// which is exactly what gives a built binary a warm first run on a
/// machine that has never seen it.
static EMBEDDED: std::sync::OnceLock<WarmProfile> = std::sync::OnceLock::new();

/// Install the profile a built binary carries (called once at startup by
/// the embedded runner; a second call is ignored).
pub fn set_embedded(profile: WarmProfile) {
    let _ = EMBEDDED.set(profile);
}

/// The profile a previous run of byte-identical source left, if any —
/// falling back to the one embedded at build time.
pub fn load(source: &str) -> Option<WarmProfile> {
    if disabled() {
        return None;
    }
    if let Some(dir) = warm_dir() {
        let path = dir.join(format!("{}.toml", key_for(source)));
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(profile) = toml::from_str(&text)
        {
            return Some(profile);
        }
    }
    EMBEDDED.get().cloned()
}

/// Persist what this run learned. Failures are swallowed: a read-only
/// home directory must never fail a program that ran correctly.
pub fn save(source: &str, profile: &WarmProfile) {
    if disabled() || profile.functions.is_empty() {
        return;
    }
    let Some(dir) = warm_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(text) = toml::to_string(profile) {
        let _ = std::fs::write(dir.join(format!("{}.toml", key_for(source))), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_round_trip_keyed_by_content() {
        let dir = std::env::temp_dir().join(format!("olang_warm_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: test-only env mutation; the single test in this module.
        unsafe { std::env::set_var("OLANG_WARM_DIR", &dir) };

        let profile = WarmProfile {
            functions: vec![WarmFn {
                name: "hot".into(),
                kinds: vec!["Int".into(), "Float".into()],
                native_calls: 4096,
            }],
        };
        save("let x = 1\n", &profile);
        assert_eq!(load("let x = 1\n"), Some(profile.clone()));
        // A one-byte edit is a different program.
        assert_eq!(load("let x = 2\n"), None);

        // Disabled cuts both directions.
        unsafe { std::env::set_var("OLANG_WARM", "0") };
        assert_eq!(load("let x = 1\n"), None);
        unsafe { std::env::remove_var("OLANG_WARM") };
        unsafe { std::env::remove_var("OLANG_WARM_DIR") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}

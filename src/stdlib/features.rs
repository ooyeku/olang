//! `runtime.features(profile?)`: what a runtime has — its modules and
//! the host powers behind them — so a program decides "runs in the
//! browser" by a check, not by guessing from a program's text.
//!
//! The native binary answers for itself and for the two browser
//! profiles it can serve (`runtime.wasm()`, `runtime.wasm("full")`);
//! a browser runtime answers for itself and for the other profiles. tests/browser_agreement_test.rs
//! holds the native answer for a profile to that profile's own.

use crate::ast::Value;
use std::sync::Arc;

/// The modules only the native binary has: the operating system's, the
/// network's, SQLite's, the terminal's and the windows'.
const NATIVE_ONLY: &[&str] = &["db", "fs", "gui", "http", "os", "proc", "pty", "tty", "vt"];

/// The modules only the full profile adds to the browser's.
const FULL_ONLY: &[&str] = &["re"];

/// The functions of modules every profile has that reach a power a
/// browser profile lacks: a host that offers to run a block in the
/// browser asks `needs` rather than the module list (`ods` is there,
/// `ods.open_csv` reads a file). `spawn` is the form, not a function.
const NEEDS: &[(&str, &str)] = &[
    ("ods.read_csv_file", "fs"),
    ("ods.read_jsonl_file", "fs"),
    ("ods.open_csv", "fs"),
    ("ods.open_jsonl", "fs"),
    ("ods.write_csv", "fs"),
    ("ods.write_jsonl", "fs"),
    ("ods.write_frame", "fs"),
    ("ods.read_frame", "fs"),
    ("ods.frame_info", "fs"),
    ("gg.save", "fs"),
    ("spawn", "threads"),
    ("task.join", "threads"),
    ("task.join_timeout", "threads"),
    ("task.watch", "threads"),
];

/// The profile this runtime is.
pub fn own_profile() -> &'static str {
    if cfg!(feature = "native") {
        "native"
    } else if cfg!(feature = "regex-module") {
        "full"
    } else {
        "browser"
    }
}

/// The module names a program can call in `profile`.
fn modules_of(profile: &str) -> Vec<String> {
    let own: Vec<String> = crate::stdlib::native_members().keys().cloned().collect();
    let mut names: Vec<String> = if profile == own_profile() {
        own
    } else if own_profile() != "native" {
        // a browser runtime describing another profile: its own set, the
        // full one's extra modules added or taken away, the native
        // binary's added
        let mut v: Vec<String> = own
            .into_iter()
            .filter(|m| profile != "browser" || !FULL_ONLY.contains(&m.as_str()))
            .collect();
        if profile != "browser" {
            v.extend(FULL_ONLY.iter().map(|s| s.to_string()));
        }
        if profile == "native" {
            v.extend(NATIVE_ONLY.iter().map(|s| s.to_string()));
        }
        v
    } else {
        // the native binary describing a browser profile: its own set
        // without what the browser cannot host
        own.into_iter()
            .filter(|m| !NATIVE_ONLY.contains(&m.as_str()))
            .filter(|m| profile == "full" || !FULL_ONLY.contains(&m.as_str()))
            .chain(
                (profile == "full")
                    .then(|| FULL_ONLY.iter().map(|s| s.to_string()))
                    .into_iter()
                    .flatten(),
            )
            .collect()
    };
    names.extend(
        crate::stdlib::embedded::names()
            .into_iter()
            .filter(|n| crate::stdlib::embedded::is_public(n))
            .map(String::from),
    );
    // a module named `__…` is the runtime's own (a test fixture), not one a program calls
    names.retain(|n| !n.starts_with("__"));
    names.sort();
    names.dedup();
    names
}

/// `#{ profile, modules, threads, fs, net, processes, db, gui, dom, jit,
/// needs }`. Any runtime answers for any profile.
pub fn features(profile: Option<&str>) -> Result<Value, String> {
    let profile = match profile {
        None => own_profile(),
        Some(p @ ("native" | "browser" | "full")) => p,
        Some(other) => {
            return Err(format!(
                "runtime.features: profile is \"native\", \"browser\" or \"full\", got \"{other}\""
            ));
        }
    };
    let native = profile == "native";
    let mut m = crate::ast::ValueMap::default();
    m.insert(
        "profile".into(),
        Value::String(Arc::new(profile.to_string())),
    );
    m.insert(
        "modules".into(),
        Value::List(Arc::new(
            modules_of(profile)
                .into_iter()
                .map(|n| Value::String(Arc::new(n)))
                .collect(),
        )),
    );
    for (key, on) in [
        ("threads", native),
        ("fs", native),
        ("net", native),
        ("processes", native),
        ("db", native),
        // the released binary has windows; a browser runtime asked of
        // the native profile answers for that binary
        (
            "gui",
            native && (cfg!(feature = "gui") || own_profile() != "native"),
        ),
        ("dom", !native),
        ("jit", native),
    ] {
        m.insert(key.into(), Value::Boolean(on));
    }
    // what this profile's modules hold but cannot run: `#{ "ods.open_csv":
    // "fs", … }`, empty where every power is there
    let mut needs = crate::ast::ValueMap::default();
    if !native {
        for (f, power) in NEEDS {
            needs.insert((*f).into(), Value::String(Arc::new((*power).to_string())));
        }
    }
    m.insert("needs".into(), Value::Map(Arc::new(needs)));
    Ok(Value::Map(Arc::new(m)))
}

/// The builtin's call: no argument, or a profile's name.
pub fn call(args: &[Value]) -> Result<Value, String> {
    match args {
        [] => features(None),
        [Value::String(p)] => features(Some(p.as_str())),
        [other] => Err(format!(
            "runtime.features: profile must be a string, got {}",
            other.type_name()
        )),
        _ => Err(format!(
            "runtime.features expects 0 or 1 argument(s), got {}",
            args.len()
        )),
    }
}

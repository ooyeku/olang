//! `cargo xtask <task>` — repository tasks that need more than a Makefile
//! line, chiefly the browser runtime the `olang` binary embeds.
//!
//! `cargo xtask wasm` builds `olang-playground` for
//! `wasm32-unknown-unknown` in release mode, in both profiles: the full
//! one (the whole stdlib, `re` and RSA included), staged at
//! `target/wasm-full/olang.wasm`, and the browser one the web SDK ships
//! (no `re`, no RSA), left where the root crate's `build.rs` reads it —
//! `target/wasm32-unknown-unknown/release/olang_playground.wasm`. The
//! next `cargo build` or `cargo install --path .` of `olang` carries both
//! (`runtime.wasm()`, `runtime.wasm("full")`). `-j N` is passed to cargo.
//!
//! Each artifact is checked for std's unsupported-platform panics (the
//! clock, sleep, the environment, paths, pipes, processes): their panic
//! sites survive in a wasm only when something can reach them, and a
//! panic there kills the browser instance. A build that carries one
//! fails, naming it.
//!
//! Nested cargo inside `build.rs` would rebuild the runtime on every
//! native build and fight the outer build for the target directory; a
//! task the developer (and `make wasm`, `make install`, CI) runs first is
//! the cleaner shape.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const WASM_TARGET: &str = "wasm32-unknown-unknown";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("wasm") => wasm(&jobs(&args)),
        Some("install") => install(),
        Some("help") | None => {
            eprintln!(
                "cargo xtask wasm [-j N]     build the browser runtimes the olang binary embeds"
            );
            eprintln!(
                "cargo xtask install         build the runtime, then install olang and otc with it"
            );
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown task '{other}'; tasks: wasm, install");
            ExitCode::FAILURE
        }
    }
}

/// `-j N` / `--jobs N` as given, for the inner cargo.
fn jobs(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-j" || a == "--jobs" {
            if let Some(n) = it.next() {
                out.push("-j".to_string());
                out.push(n.clone());
            }
        } else if let Some(n) = a.strip_prefix("-j") {
            out.push("-j".to_string());
            out.push(n.to_string());
        }
    }
    out
}

/// std's panics for what `wasm32-unknown-unknown` lacks, by the source
/// file each is raised in: present in an artifact only when reachable.
const UNSUPPORTED: &[(&str, &str)] = &[
    (
        "sys/time/unsupported.rs",
        "the clock (Instant::now, SystemTime::now): use crate::clock",
    ),
    (
        "sys/thread/unsupported.rs",
        "thread::sleep: use crate::clock::sleep",
    ),
    ("sys/env/unsupported.rs", "std::env::vars"),
    ("sys/paths/unsupported.rs", "split_paths or temp_dir"),
    ("sys/pipe/unsupported.rs", "a pipe"),
    ("sys/process/unsupported.rs", "process::id"),
    ("sys/random/unsupported.rs", "std's random source"),
];

/// The unsupported-platform panics `wasm` can reach.
fn unreachable_std(wasm: &Path) -> Result<(), String> {
    let bytes = std::fs::read(wasm).map_err(|e| format!("cannot read {}: {e}", wasm.display()))?;
    let found: Vec<String> = UNSUPPORTED
        .iter()
        .filter(|(file, _)| bytes.windows(file.len()).any(|w| w == file.as_bytes()))
        .map(|(file, what)| format!("  {file}: {what}"))
        .collect();
    if found.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} can reach std code that panics in a browser (the panic kills the instance):\n{}",
            wasm.display(),
            found.join("\n")
        ))
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits inside the repository")
        .to_path_buf()
}

fn cargo(root: &Path, args: &[&str]) -> bool {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    match Command::new(cargo).args(args).current_dir(root).status() {
        Ok(status) => status.success(),
        Err(e) => {
            eprintln!("cannot run cargo: {e}");
            false
        }
    }
}

/// The one-liner: the runtime, then `cargo install --path . --locked
/// --force` for olang and otc, so the installed binaries carry it.
fn install() -> ExitCode {
    if wasm(&[]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }
    let root = repo_root();
    for (path, bin) in [(".", "olang"), ("otc", "otc")] {
        if !cargo(
            &root,
            &[
                "install", "--path", path, "--bin", bin, "--locked", "--force",
            ],
        ) {
            return ExitCode::FAILURE;
        }
    }
    println!(
        "installed olang and otc with the browser runtime embedded (into Cargo's bin folder, usually ~/.cargo/bin)"
    );
    println!("try: olang --version");
    ExitCode::SUCCESS
}

/// Whether the active toolchain has the wasm target's standard library;
/// adds it through rustup when it is missing, and says what to do when
/// that cannot be done — rather than leaving rustc's "can't find crate
/// for `core`" wall.
fn wasm_target_ready() -> bool {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let sysroot = Command::new(&rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    let has = |root: &Option<PathBuf>| {
        root.as_ref()
            .is_some_and(|r| r.join("lib/rustlib").join(WASM_TARGET).is_dir())
    };
    if has(&sysroot) {
        return true;
    }
    eprintln!("the browser runtime needs Rust's {WASM_TARGET} target; adding it with rustup...");
    let added = Command::new("rustup")
        .args(["target", "add", WASM_TARGET])
        .status()
        .is_ok_and(|s| s.success());
    if !added {
        eprintln!(
            "could not add the {WASM_TARGET} target.\n\
             With rustup (https://rustup.rs), run:  rustup target add {WASM_TARGET}\n\
             then run this command again. Rust installed another way (e.g. Homebrew's) has no\n\
             wasm target; `cargo install --path .` installs olang without the browser runtime."
        );
    }
    added
}

fn wasm(jobs: &[String]) -> ExitCode {
    if !wasm_target_ready() {
        return ExitCode::FAILURE;
    }
    let root = repo_root();
    let artifact = root
        .join("target")
        .join(WASM_TARGET)
        .join("release")
        .join("olang_playground.wasm");
    let build = |features: &[&str]| -> bool {
        let mut args = vec![
            "build",
            "-p",
            "olang-playground",
            "--target",
            WASM_TARGET,
            "--release",
        ];
        args.extend_from_slice(features);
        args.extend(jobs.iter().map(String::as_str));
        cargo(&root, &args)
    };
    // The full profile: the whole stdlib, for a playground.
    if !build(&["--features", "full"]) {
        return ExitCode::FAILURE;
    }
    if let Err(e) = unreachable_std(&artifact) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    let dest = root.join("target/wasm-full/olang.wasm");
    if let Some(dir) = dest.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::copy(&artifact, &dest) {
        eprintln!("cannot stage {}: {e}", dest.display());
        return ExitCode::FAILURE;
    }
    println!("staged {}", dest.display());
    // The browser profile the SDK ships. Built last so the artifact path
    // holds this profile afterwards.
    if !build(&[]) {
        return ExitCode::FAILURE;
    }
    if let Err(e) = unreachable_std(&artifact) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    match std::fs::metadata(&artifact) {
        Ok(meta) => {
            println!(
                "{} ({} KB) — the next `cargo build` of olang embeds it and the full one",
                artifact.display(),
                meta.len() / 1024
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("built, but {} is missing: {e}", artifact.display());
            ExitCode::FAILURE
        }
    }
}

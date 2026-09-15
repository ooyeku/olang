//! `cargo xtask <task>` — repository tasks that need more than a Makefile
//! line, chiefly the browser runtime the `olang` binary embeds.
//!
//! `cargo xtask wasm` builds `olang-playground` for
//! `wasm32-unknown-unknown` in release mode (the browser profile the
//! web SDK ships: no `re`, no RSA) and leaves the artifact where the
//! root crate's `build.rs` reads it — `target/wasm32-unknown-unknown/
//! release/olang_playground.wasm` — so the next `cargo build` or
//! `cargo install --path .` of `olang` carries the runtime. With
//! `--full` it also builds the website's profile (the whole stdlib) and
//! stages it at `website/static/playground/olang.wasm`.
//!
//! Nested cargo inside `build.rs` would rebuild the runtime on every
//! native build and fight the outer build for the target directory; a
//! task the developer (and `make wasm`, `setup.sh`, CI) runs first is
//! the cleaner shape.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const WASM_TARGET: &str = "wasm32-unknown-unknown";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("wasm") => wasm(args.iter().any(|a| a == "--full")),
        Some("install") => install(),
        Some("help") | None => {
            eprintln!(
                "cargo xtask wasm [--full]   build the browser runtime the olang binary embeds"
            );
            eprintln!(
                "cargo xtask install         build the runtime, then install olang and otc with it"
            );
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown task '{other}'; tasks: wasm");
            ExitCode::FAILURE
        }
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
    if wasm(false) != ExitCode::SUCCESS {
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
    println!("installed olang and otc with the browser runtime embedded");
    ExitCode::SUCCESS
}

fn wasm(full: bool) -> ExitCode {
    let root = repo_root();
    let artifact = root
        .join("target")
        .join(WASM_TARGET)
        .join("release")
        .join("olang_playground.wasm");
    if full {
        // The website's playground carries the whole stdlib.
        if !cargo(
            &root,
            &[
                "build",
                "-p",
                "olang-playground",
                "--features",
                "full",
                "--target",
                WASM_TARGET,
                "--release",
            ],
        ) {
            return ExitCode::FAILURE;
        }
        let dest = root.join("website/static/playground/olang.wasm");
        if let Some(dir) = dest.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::copy(&artifact, &dest) {
            eprintln!("cannot stage {}: {e}", dest.display());
            return ExitCode::FAILURE;
        }
        println!("staged {}", dest.display());
    }
    // The browser profile the SDK ships — the bytes the binary embeds.
    // Built last so the artifact path holds this profile afterwards.
    if !cargo(
        &root,
        &[
            "build",
            "-p",
            "olang-playground",
            "--target",
            WASM_TARGET,
            "--release",
        ],
    ) {
        return ExitCode::FAILURE;
    }
    match std::fs::metadata(&artifact) {
        Ok(meta) => {
            println!(
                "{} ({} KB) — the next `cargo build` of olang embeds it",
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

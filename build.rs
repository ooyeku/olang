//! Embeds the browser runtime in the `olang` binary.
//!
//! The runtime is the playground wasm — olang itself compiled for
//! `wasm32-unknown-unknown`, which `cargo xtask wasm` (or `make wasm`)
//! builds first and leaves at `target/wasm32-unknown-unknown/release/
//! olang_playground.wasm`. This script copies those bytes into
//! `OUT_DIR` and hands the path to `include_bytes!` through the
//! `OLANG_RUNTIME_WASM` environment variable (`src/runtime_wasm.rs`).
//! When the artifact is absent — a fresh checkout that has not built
//! it, or the wasm build of olang itself, which must not embed a copy
//! of itself — an empty file is embedded and `runtime.wasm()` answers
//! `Err`, telling the program how to build a binary that carries it.
//! `OLANG_WASM` names another artifact to embed instead.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let dest = out_dir.join("olang_runtime.wasm");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-env-changed=OLANG_WASM");
    let artifact = env::var("OLANG_WASM")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            manifest_dir
                .join("target")
                .join("wasm32-unknown-unknown")
                .join("release")
                .join("olang_playground.wasm")
        });
    println!("cargo:rerun-if-changed={}", artifact.display());
    let target_is_wasm = env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
    let native = env::var("CARGO_FEATURE_NATIVE").is_ok();
    let bytes = if !target_is_wasm && native {
        fs::read(&artifact).unwrap_or_default()
    } else {
        Vec::new()
    };
    let unchanged = fs::read(&dest).map(|have| have == bytes).unwrap_or(false);
    if !unchanged {
        fs::write(&dest, &bytes).expect("write the embedded runtime into OUT_DIR");
    }
    println!("cargo:rustc-env=OLANG_RUNTIME_WASM={}", dest.display());
}

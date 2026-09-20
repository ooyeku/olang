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
//!
//! The gzip and brotli forms and the content hash are made here too, and
//! embedded beside the raw bytes. Compressing at first use cost a served
//! app 64 MB of freed-but-resident scratch memory and a third of a
//! second of boot; a build pays it once.

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
    let gz_dest = out_dir.join("olang_runtime.wasm.gz");
    let br_dest = out_dir.join("olang_runtime.wasm.br");
    let hash_dest = out_dir.join("olang_runtime.wasm.hash");
    let unchanged = fs::read(&dest).map(|have| have == bytes).unwrap_or(false)
        && gz_dest.exists()
        && br_dest.exists()
        && hash_dest.exists();
    if !unchanged {
        let (gz, br, hash) = if bytes.is_empty() {
            (Vec::new(), Vec::new(), String::new())
        } else {
            (gzip(&bytes), brotli(&bytes), hash16(&bytes))
        };
        fs::write(&gz_dest, gz).expect("write the gzipped runtime into OUT_DIR");
        fs::write(&br_dest, br).expect("write the brotli runtime into OUT_DIR");
        fs::write(&hash_dest, hash).expect("write the runtime hash into OUT_DIR");
        fs::write(&dest, &bytes).expect("write the embedded runtime into OUT_DIR");
    }
    println!("cargo:rustc-env=OLANG_RUNTIME_WASM={}", dest.display());
    println!(
        "cargo:rustc-env=OLANG_RUNTIME_WASM_GZ={}",
        gz_dest.display()
    );
    println!(
        "cargo:rustc-env=OLANG_RUNTIME_WASM_BR={}",
        br_dest.display()
    );
    println!(
        "cargo:rustc-env=OLANG_RUNTIME_WASM_HASH={}",
        hash_dest.display()
    );
}

/// Gzip at level 6 — what `Content-Encoding: gzip` carries.
fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
    encoder.write_all(bytes).expect("gzip the runtime");
    encoder.finish().expect("gzip the runtime")
}

/// Brotli at quality 5, window 22 — a quarter of the raw bytes on the wire.
fn brotli(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut out = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
        writer.write_all(bytes).expect("brotli the runtime");
        writer.flush().expect("brotli the runtime");
    }
    out
}

/// The first sixteen hex digits of the SHA-256: the runtime's ETag and the
/// name in its content-addressed URL.
fn hash16(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hex = format!("{:x}", Sha256::digest(bytes));
    hex[..16].to_string()
}

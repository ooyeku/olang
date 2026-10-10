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
    // The executable's pointers are fixed up by chained fixups, not by
    // the rebase and bind opcodes a macOS 11 minimum otherwise selects:
    // dyld then applies them as pages are first touched instead of
    // walking every one before `main`. Started by Launch Services (an
    // application), the runtime reached `main` ~3.5 ms later than a C
    // executable linking the same frameworks; with them, as soon.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64")
    {
        println!("cargo:rustc-link-arg-bins=-Wl,-fixup_chains");
    }
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
    build_info(&manifest_dir);
}

/// What `runtime.build()` reports (src/version.rs): the commit the binary
/// was built from (and whether the checkout had uncommitted changes then),
/// when, the profile and target, the compiler, and the versions of the
/// crates that matter to a bug report (the gui engine's and a few of the
/// stdlib's), read from Cargo.lock. Built again when the sources, the lock
/// or the checkout's commit change — so the date is the last time any of
/// them did, and the commit is the one the sources were built at.
fn build_info(manifest_dir: &std::path::Path) {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git").args(args).current_dir(manifest_dir).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let git_dir = manifest_dir.join(".git");
    if git_dir.is_dir() {
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        if let Ok(head) = fs::read_to_string(git_dir.join("HEAD"))
            && let Some(r) = head.trim().strip_prefix("ref: ")
        {
            println!("cargo:rerun-if-changed={}", git_dir.join(r).display());
        }
        println!("cargo:rerun-if-changed={}", git_dir.join("packed-refs").display());
    }
    println!("cargo:rerun-if-changed={}", manifest_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", manifest_dir.join("Cargo.lock").display());
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).map(|s| !s.is_empty());
    let secs = env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let rustc_v = std::process::Command::new(rustc)
        .arg("-V")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    // name version[ vendored] for each crate named, in this order
    const CRATES: &[&str] = &[
        "winit", "wgpu", "accesskit", "accesskit_winit", "accesskit_macos", "parley", "swash", "skrifa", "tiny-skia",
        "resvg", "softbuffer", "muda", "rfd", "arboard", "objc2", "objc2-app-kit", "zune-jpeg", "image-webp", "gif",
        "cranelift-codegen", "rusqlite", "reqwest", "regex", "rustyline", "rayon", "serde_json",
    ];
    let lock = fs::read_to_string(manifest_dir.join("Cargo.lock")).unwrap_or_default();
    let mut found: Vec<(String, String, bool)> = Vec::new();
    for block in lock.split("[[package]]") {
        let field = |k: &str| {
            block.lines().find_map(|l| {
                l.trim().strip_prefix(k).and_then(|r| r.trim().strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_string())
            })
        };
        if let (Some(name), Some(version)) = (field("name "), field("version ")) {
            if CRATES.contains(&name.as_str()) {
                let local = !block.contains("source =");
                found.push((name, version, local));
            }
        }
    }
    let mut crates = Vec::new();
    for want in CRATES {
        let vs: Vec<String> = found
            .iter()
            .filter(|(n, _, _)| n == want)
            .map(|(_, v, local)| if *local { format!("{v}+vendored") } else { v.clone() })
            .collect();
        if !vs.is_empty() {
            crates.push(format!("{} {}", want, vs.join("/")));
        }
    }
    println!("cargo:rustc-env=OLANG_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=OLANG_BUILD_BRANCH={branch}");
    println!(
        "cargo:rustc-env=OLANG_BUILD_DIRTY={}",
        match dirty {
            Some(true) => "true",
            Some(false) => "false",
            None => "",
        }
    );
    println!("cargo:rustc-env=OLANG_BUILD_DATE={}", rfc3339(secs));
    println!("cargo:rustc-env=OLANG_BUILD_PROFILE={}", env::var("PROFILE").unwrap_or_default());
    println!("cargo:rustc-env=OLANG_BUILD_TARGET={}", env::var("TARGET").unwrap_or_default());
    println!("cargo:rustc-env=OLANG_BUILD_RUSTC={rustc_v}");
    println!("cargo:rustc-env=OLANG_BUILD_CRATES={}", crates.join(";"));
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ` (UTC; the civil date
/// by Howard Hinnant's days-from-civil, inverted).
fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
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

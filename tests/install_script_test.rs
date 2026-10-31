//! install.sh, the one-line installer: it parses as POSIX sh, and the
//! oldest Rust it asks for before a source build is the one Cargo.toml
//! (and otc's) declare.

use std::path::Path;

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("reading {rel}: {e}"))
}

fn rust_version(manifest: &str) -> String {
    read(manifest)
        .lines()
        .find_map(|l| l.strip_prefix("rust-version = \""))
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{manifest} declares no rust-version"))
        .to_string()
}

#[test]
fn installer_min_rust_matches_the_manifests() {
    let script = read("install.sh");
    let min = script
        .lines()
        .find_map(|l| l.strip_prefix("MIN_RUST=\""))
        .and_then(|v| v.strip_suffix('"'))
        .expect("install.sh sets MIN_RUST");
    assert_eq!(
        min,
        rust_version("Cargo.toml"),
        "install.sh MIN_RUST vs Cargo.toml"
    );
    assert_eq!(
        min,
        rust_version("otc/Cargo.toml"),
        "install.sh MIN_RUST vs otc/Cargo.toml"
    );
}

#[test]
fn installer_parses_as_posix_sh() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let status = std::process::Command::new("sh")
        .arg("-n")
        .arg(&path)
        .status();
    if let Ok(status) = status {
        assert!(status.success(), "sh -n install.sh failed");
    }
}

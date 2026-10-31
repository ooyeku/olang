//! The installers. install.sh parses as POSIX sh, passes shellcheck,
//! and does the right thing on every pretend computer in
//! tests/install/harness.sh under each shell here. The oldest Rust it
//! asks for before a source build is the one Cargo.toml (and otc's)
//! declare, and install.sh and install.ps1 carry the same signing key.

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

/// The shell-level harness (tests/install/harness.sh): install.sh run
/// against pretend computers and a pretend GitHub, under every POSIX
/// shell this computer has.
#[test]
fn installer_works_on_pretend_computers_under_each_shell() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let harness = root.join("tests/install/harness.sh");
    let mut ran = 0;
    for shell in ["sh", "dash", "bash"] {
        let found = std::process::Command::new("sh")
            .args(["-c", &format!("command -v {shell}")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !found {
            continue;
        }
        let out = std::process::Command::new("sh")
            .arg(&harness)
            .arg(shell)
            .output()
            .expect("running tests/install/harness.sh");
        assert!(
            out.status.success(),
            "install.sh under {shell}:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        ran += 1;
    }
    assert!(ran > 0, "no POSIX shell found to run the harness");
}

/// shellcheck, when it is installed, finds nothing in the installer or
/// its harness.
#[test]
fn installer_passes_shellcheck() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Ok(out) = std::process::Command::new("shellcheck")
        .args(["-s", "sh", "install.sh", "tests/install/harness.sh"])
        .current_dir(root)
        .output()
    else {
        return; // not installed here
    };
    assert!(
        out.status.success(),
        "shellcheck:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// The release signing key: install.sh, install.ps1 and
/// dist/signing/olang-release.pub carry the same public key, and a key
/// always comes with the release it starts at.
#[test]
fn installers_agree_on_the_signing_key() {
    let sh = read("install.sh");
    let key = shell_value(&sh, "SIGNING_KEY");
    let from = shell_value(&sh, "SIGNED_FROM");
    if !key.is_empty() {
        assert!(
            key.starts_with("-----BEGIN PUBLIC KEY-----")
                && key.ends_with("-----END PUBLIC KEY-----"),
            "install.sh SIGNING_KEY is not a PEM public key"
        );
        assert!(
            from.starts_with('v'),
            "install.sh has a SIGNING_KEY but no SIGNED_FROM release"
        );
    }
    let pub_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/signing/olang-release.pub");
    if pub_file.exists() {
        assert_eq!(
            read("dist/signing/olang-release.pub").trim(),
            key,
            "dist/signing/olang-release.pub vs install.sh SIGNING_KEY"
        );
    }
    let ps = read("install.ps1");
    assert_eq!(
        ps_value(&ps, "SigningKey"),
        key,
        "install.ps1 $SigningKey vs install.sh"
    );
    assert_eq!(
        ps_value(&ps, "SignedFrom"),
        from,
        "install.ps1 $SignedFrom vs install.sh"
    );
}

/// NAME="value" in sh, possibly spanning lines.
fn shell_value(script: &str, name: &str) -> String {
    let start = script
        .find(&format!("\n{name}=\""))
        .unwrap_or_else(|| panic!("install.sh sets no {name}"))
        + name.len()
        + 3;
    let end = script[start..].find('"').expect("closing quote") + start;
    script[start..end].replace("\r\n", "\n")
}

/// $Name = 'value' in PowerShell, possibly spanning lines.
fn ps_value(script: &str, name: &str) -> String {
    let start = script
        .find(&format!("\n${name} = '"))
        .unwrap_or_else(|| panic!("install.ps1 sets no ${name}"))
        + name.len()
        + 6;
    let end = script[start..].find('\'').expect("closing quote") + start;
    script[start..end].replace("\r\n", "\n")
}

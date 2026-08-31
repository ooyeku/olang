//! `otc update` and `otc toolchain` — release installation and
//! side-by-side version management, rustup-shaped but smaller.
//!
//! Releases install into `~/.olang/toolchains/<version>/bin/` and the
//! shims in `~/.olang/bin/` are symlinks to the default toolchain, so
//! switching versions is one atomic relink and the running binaries are
//! never overwritten in place. Downloads come from the project's GitHub
//! releases and are verified against the release's `SHA256SUMS` before
//! anything is installed. Nothing checks the network unless explicitly
//! asked: `otc update` and `otc update --check` are the only calls that
//! reach out, consistent with the language's capability posture.
//!
//! Installations owned by other package managers (Homebrew, cargo) are
//! never modified; `otc doctor` reports how they shadow each other.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const REPO: &str = "ooyeku/olang";

#[derive(Subcommand)]
pub enum ToolchainCommand {
    /// List installed toolchains and mark the default
    List,
    /// Download and install a specific release (e.g. 0.79.0)
    Install { version: String },
    /// Make an installed toolchain the default
    Default { version: String },
    /// Remove an installed toolchain (never the default)
    Remove { version: String },
}

/// `otc update [--check]`: install the latest release and make it the
/// default, or with `--check` just report what the latest is.
pub fn update(check: bool) -> Result<()> {
    let latest = latest_version()?;
    let current = olang::VERSION;
    if check {
        println!("current: {}", current);
        println!("latest:  {}", latest);
        if normalize(&latest) == current {
            println!("up to date.");
        } else {
            println!("run `otc update` to install {}.", latest);
        }
        return Ok(());
    }
    if normalize(&latest) == current && default_toolchain()?.as_deref() == Some(current) {
        println!("already up to date ({}).", current);
        return Ok(());
    }
    install_version(&normalize(&latest))?;
    set_default(&normalize(&latest))?;
    print_path_hint()?;
    Ok(())
}

pub fn toolchain(cmd: ToolchainCommand) -> Result<()> {
    match cmd {
        ToolchainCommand::List => {
            let dir = toolchains_dir()?;
            let default = default_toolchain()?;
            let mut versions: Vec<String> = match std::fs::read_dir(&dir) {
                Ok(entries) => entries
                    .flatten()
                    .filter(|e| e.path().join("bin").join("olang").is_file())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect(),
                Err(_) => Vec::new(),
            };
            versions.sort();
            if versions.is_empty() {
                println!("no toolchains installed under {}", dir.display());
                println!("(`otc update` installs the latest release)");
                return Ok(());
            }
            for v in versions {
                let marker = if Some(v.as_str()) == default.as_deref() {
                    "(default)"
                } else {
                    ""
                };
                println!("  {} {}", v, marker);
            }
            Ok(())
        }
        ToolchainCommand::Install { version } => {
            install_version(&normalize(&version))?;
            println!(
                "installed {}; `otc toolchain default {}` switches to it.",
                normalize(&version),
                normalize(&version)
            );
            Ok(())
        }
        ToolchainCommand::Default { version } => {
            let v = normalize(&version);
            let dir = toolchains_dir()?.join(&v);
            if !dir.join("bin").join("olang").is_file() {
                bail!("{} is not installed (otc toolchain install {})", v, v);
            }
            set_default(&v)?;
            print_path_hint()?;
            Ok(())
        }
        ToolchainCommand::Remove { version } => {
            let v = normalize(&version);
            if default_toolchain()?.as_deref() == Some(v.as_str()) {
                bail!("{} is the default toolchain; switch defaults first", v);
            }
            let dir = toolchains_dir()?.join(&v);
            if !dir.exists() {
                bail!("{} is not installed", v);
            }
            std::fs::remove_dir_all(&dir)?;
            println!("removed {}.", v);
            Ok(())
        }
    }
}

fn normalize(v: &str) -> String {
    v.trim_start_matches('v').to_string()
}

fn toolchains_dir() -> Result<PathBuf> {
    olang::home::toolchains().context("no home directory could be determined")
}

fn bin_dir() -> Result<PathBuf> {
    olang::home::bin().context("no home directory could be determined")
}

/// The default toolchain, read from where the olang shim points.
fn default_toolchain() -> Result<Option<String>> {
    let shim = bin_dir()?.join("olang");
    let Ok(target) = std::fs::read_link(&shim) else {
        return Ok(None);
    };
    // .../toolchains/<version>/bin/olang
    Ok(target
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .map(|v| v.to_string_lossy().into_owned()))
}

fn set_default(version: &str) -> Result<()> {
    let bins = bin_dir()?;
    std::fs::create_dir_all(&bins)?;
    for name in ["olang", "otc"] {
        let target = toolchains_dir()?.join(version).join("bin").join(name);
        let shim = bins.join(name);
        let staged = bins.join(format!(".{}.new", name));
        let _ = std::fs::remove_file(&staged);
        std::os::unix::fs::symlink(&target, &staged)
            .with_context(|| format!("linking {}", shim.display()))?;
        std::fs::rename(&staged, &shim)?;
    }
    println!("default toolchain is now {}.", version);
    Ok(())
}

fn print_path_hint() -> Result<()> {
    let bins = bin_dir()?;
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d == bins))
        .unwrap_or(false);
    if !on_path {
        println!();
        println!("add the shims to your PATH:");
        println!("    export PATH=\"{}:$PATH\"", bins.display());
    }
    Ok(())
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(format!("otc/{}", olang::VERSION))
        .build()?)
}

fn latest_version() -> Result<String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    let resp: serde_json::Value = client()?
        .get(&url)
        .send()
        .context("could not reach github.com")?
        .error_for_status()?
        .json()?;
    resp["tag_name"]
        .as_str()
        .map(|s| s.to_string())
        .context("release response carried no tag_name")
}

fn platform() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        (os, arch) => bail!(
            "no prebuilt release for {}/{}; build from source (see docs/installation.md)",
            os,
            arch
        ),
    })
}

fn install_version(version: &str) -> Result<()> {
    let plat = platform()?;
    let dest = toolchains_dir()?.join(version);
    if dest.join("bin").join("olang").is_file() {
        println!("{} is already installed.", version);
        return Ok(());
    }
    let base = format!("https://github.com/{}/releases/download/v{}", REPO, version);
    let tarball = format!("olang-{}-{}.tar.gz", version, plat);

    println!("downloading {} ...", tarball);
    let client = client()?;
    let bytes = client
        .get(format!("{}/{}", base, tarball))
        .send()
        .context("download failed")?
        .error_for_status()
        .with_context(|| format!("no {} in release v{}", tarball, version))?
        .bytes()?;
    let sums = client
        .get(format!("{}/SHA256SUMS", base))
        .send()?
        .error_for_status()?
        .text()?;

    let expected = sums
        .lines()
        .find_map(|l| {
            l.strip_suffix(tarball.as_str())
                .map(|h| h.trim().to_string())
        })
        .with_context(|| format!("{} is not listed in SHA256SUMS", tarball))?;
    let actual = hex(&Sha256::digest(&bytes));
    if actual != expected {
        bail!(
            "checksum mismatch for {} (expected {}, got {})",
            tarball,
            expected,
            actual
        );
    }
    println!("checksum verified.");

    // Extract into a staging directory, then rename into place so a
    // half-finished install never looks installed.
    let staging = toolchains_dir()?.join(format!(".staging-{}", version));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let tar_path = staging.join(&tarball);
    std::fs::write(&tar_path, &bytes)?;
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(&tar_path)
        .arg("-C")
        .arg(&staging)
        .status()
        .context("running tar")?;
    if !status.success() {
        bail!("tar extraction failed");
    }
    let unpacked = staging.join(format!("olang-{}-{}", version, plat));
    let bin = dest.join("bin");
    std::fs::create_dir_all(&bin)?;
    for name in ["olang", "otc"] {
        std::fs::rename(unpacked.join(name), bin.join(name))
            .with_context(|| format!("staging {}", name))?;
    }
    let _ = std::fs::remove_dir_all(&staging);

    // Smoke-test what was installed before anyone links to it.
    let out = std::process::Command::new(bin.join("olang"))
        .arg("--version")
        .output()
        .context("running the installed olang")?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !text.contains(version) {
        let _ = std::fs::remove_dir_all(&dest);
        bail!(
            "installed binary reports '{}', expected {}",
            text.trim(),
            version
        );
    }
    println!("installed {} to {}.", version, dest.display());
    Ok(())
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

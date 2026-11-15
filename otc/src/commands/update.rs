//! `otc update` and `otc toolchain` — release installation and
//! side-by-side version management, rustup-shaped but smaller.
//!
//! Releases install into `~/.olang/toolchains/<version>/bin/` and the
//! shims in `~/.olang/bin/` are symlinks to the default toolchain, so
//! switching versions is one atomic relink and the running binaries are
//! never overwritten in place. Downloads come from the project's GitHub
//! releases and are verified against the release's `SHA256SUMS` before
//! anything is installed, and `SHA256SUMS` against its signature
//! (`SHA256SUMS.sig`) by olang's release key, pinned here as
//! [`RELEASE_SIGNING_KEY`] (OL-491). Nothing checks the network unless explicitly
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

/// The public half of olang's release signing key, an ECDSA P-256 key
/// as PEM: the key `install.sh` pins as `SIGNING_KEY` and `install.ps1`
/// as `$SigningKey` (tests/install_script_test.rs checks the copies
/// agree). The release workflow signs each release's `SHA256SUMS` with
/// the private half (dist/signing/README.md).
///
/// PLACEHOLDER: empty until the owner sets up release signing and pastes
/// the real public key here, as into the installers. While it is empty
/// `otc update` checks the checksum only and says the release is
/// unverified.
pub const RELEASE_SIGNING_KEY: &str = "";

/// The first signed release (`v1.0.0`), as `install.sh`'s `SIGNED_FROM`:
/// an older release carries no signature and installs on its checksum.
pub const SIGNED_FROM: &str = "";

/// What the check of a release's `SHA256SUMS` signature found.
#[derive(Debug, PartialEq)]
enum Signature {
    /// It verifies with the pinned key.
    Verified,
    /// No key is pinned yet: signing is not set up.
    NotSetUp,
    /// The release is older than the first signed one and has none.
    OlderThanSigning,
    /// It has none, and `OLANG_ALLOW_UNSIGNED=1` lets it through.
    AllowedUnsigned,
}

/// Whether `sums` (a release's `SHA256SUMS`) may be trusted: its
/// signature `sig` (`None` when the release has no `SHA256SUMS.sig`)
/// must verify with `key`, the pinned public key. A release older than
/// `signed_from` may lack one; a bad signature is refused always.
fn check_signature(
    key: &str,
    signed_from: &str,
    version: &str,
    sums: &[u8],
    sig: Option<&[u8]>,
    allow_unsigned: bool,
) -> Result<Signature> {
    if key.trim().is_empty() {
        return Ok(Signature::NotSetUp);
    }
    let Some(sig) = sig else {
        let older = match (
            semver::Version::parse(normalize(version).as_str()),
            semver::Version::parse(normalize(signed_from).as_str()),
        ) {
            (Ok(v), Ok(from)) => v < from,
            _ => false,
        };
        if older {
            return Ok(Signature::OlderThanSigning);
        }
        if allow_unsigned {
            return Ok(Signature::AllowedUnsigned);
        }
        bail!(
            "release {version} has no SHA256SUMS.sig, so otc cannot tell that it came from olang; \
             nothing was installed. Every release from {signed_from} on is signed: please report \
             this at https://github.com/{REPO}/issues"
        );
    };
    verify_p256(key, sums, sig).map_err(|e| {
        anyhow::anyhow!(
            "the signature of release {version}'s SHA256SUMS does not verify with olang's release \
             key ({e}), so nothing was installed. The files on the download server may have been \
             changed by someone other than olang: please report this at https://github.com/{REPO}/issues"
        )
    })?;
    Ok(Signature::Verified)
}

/// The DER prefix of an ECDSA P-256 SubjectPublicKeyInfo: the algorithm
/// (id-ecPublicKey, prime256v1) and the bit string's header. The 65-byte
/// uncompressed point follows it.
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// Verify `sig`, a DER ECDSA signature over SHA-256 (what `openssl dgst
/// -sha256 -sign` writes), of `msg` with `pem`, a P-256 public key.
fn verify_p256(pem: &str, msg: &[u8], sig: &[u8]) -> Result<()> {
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
    use base64::Engine;
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("-----"))
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body)
        .context("the pinned key is not base64 PEM")?;
    let point = der
        .strip_prefix(&P256_SPKI_PREFIX[..])
        .filter(|p| p.len() == 65)
        .context("the pinned key is not an ECDSA P-256 public key")?;
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, point)
        .verify(msg, sig)
        .map_err(|_| anyhow::anyhow!("bad signature"))
}

#[derive(Subcommand)]
pub enum ToolchainCommand {
    /// List every olang on this machine: the toolchains, the shims'
    /// default, and those on PATH, with the one that runs marked
    List {
        /// `text` (the list) or `json` (the same as data)
        #[arg(long, value_name = "FORMAT", default_value = "text")]
        format: String,
    },
    /// Download and install a specific release (e.g. 0.79.0)
    Install { version: String },
    /// Make an installed toolchain the default
    Default { version: String },
    /// Remove an installed toolchain (never the default)
    Remove { version: String },
    /// Print the olang a project runs: the toolchain its pin resolves to
    /// (olang.toml `[package] olang`, olang.lock's `olang`)
    Which {
        /// The project's folder (default: the one this is run in)
        dir: Option<String>,
    },
    /// Register a local build as a toolchain: VERSION's olang becomes
    /// PATH (an olang binary, or a checkout holding target/release/olang)
    Link { version: String, path: String },
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
        ToolchainCommand::List { format } => {
            let json = super::json_format("otc toolchain list", &format)?;
            // every olang on the machine, which one runs and why (OL-577)
            super::toolchains::list(json, default_toolchain()?)
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
        ToolchainCommand::Which { dir } => {
            let start = match dir {
                Some(d) => PathBuf::from(d),
                None => std::env::current_dir()?,
            };
            let root = olang::pkg::manifest::Manifest::find_root(&start)
                .with_context(|| format!("no olang.toml at or above {}", start.display()))?;
            use olang::pkg::toolchain::{Resolved, for_project, install_hint};
            match for_project(&root) {
                Resolved::Installed { bin, .. } => println!("{}", bin.display()),
                Resolved::Running { .. } | Resolved::Unpinned => {
                    println!(
                        "{}",
                        std::env::current_exe()?.with_file_name("olang").display()
                    )
                }
                Resolved::Missing { pin } => bail!(
                    "olang.toml pins olang {}, and no installed olang satisfies it ({} installs one)",
                    pin,
                    install_hint(&pin)
                ),
            }
            Ok(())
        }
        ToolchainCommand::Link { version, path } => {
            let v = normalize(&version);
            let given = PathBuf::from(&path);
            let bin = if given.is_dir() {
                given.join("target").join("release").join("olang")
            } else {
                given
            };
            let bin = bin
                .canonicalize()
                .with_context(|| format!("no olang at {}", bin.display()))?;
            if !bin.is_file() {
                bail!("{} is not an olang binary", bin.display());
            }
            let dir = toolchains_dir()?.join(&v).join("bin");
            std::fs::create_dir_all(&dir)?;
            let link = dir.join("olang");
            let _ = std::fs::remove_file(&link);
            link_file(&bin, &link).with_context(|| format!("linking {}", link.display()))?;
            println!("toolchain {} is {}.", v, bin.display());
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
        link_file(&target, &staged).with_context(|| format!("linking {}", shim.display()))?;
        std::fs::rename(&staged, &shim)?;
    }
    println!("default toolchain is now {}.", version);
    Ok(())
}

/// A symlink on Unix. Windows lets only administrators and developer
/// mode make symlinks, so there the file is copied instead.
fn link_file(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(not(unix))]
    {
        std::fs::copy(target, link).map(|_| ())
    }
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
    // A static musl build updates to the static build, as the installer
    // chose it for a reason (Alpine, or a Linux older than glibc 2.28).
    let musl = cfg!(target_env = "musl");
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("linux", "x86_64") if musl => "linux-x64-musl",
        ("linux", "aarch64") if musl => "linux-arm64-musl",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("windows", _) => bail!(
            "otc update does not update a Windows install yet; run the installer again: \
             irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex"
        ),
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
    let sig = if RELEASE_SIGNING_KEY.trim().is_empty() {
        None
    } else {
        let resp = client
            .get(format!("{}/SHA256SUMS.sig", base))
            .send()
            .context("download of SHA256SUMS.sig failed")?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            None
        } else {
            Some(resp.error_for_status()?.bytes()?.to_vec())
        }
    };
    match check_signature(
        RELEASE_SIGNING_KEY,
        SIGNED_FROM,
        version,
        sums.as_bytes(),
        sig.as_deref(),
        std::env::var("OLANG_ALLOW_UNSIGNED").as_deref() == Ok("1"),
    )? {
        Signature::Verified => println!("signature verified: signed by olang's release key."),
        Signature::NotSetUp => println!(
            "signature NOT verified: this otc has no release key pinned yet, so only the \
             checksum below is checked, which proves the download is intact but not who \
             published it (dist/signing/README.md)."
        ),
        Signature::OlderThanSigning => println!(
            "release {} is older than olang's release signatures (they start at {}), \
             so only its checksum is checked.",
            version, SIGNED_FROM
        ),
        Signature::AllowedUnsigned => println!(
            "release {} has no signature; going on because OLANG_ALLOW_UNSIGNED=1.",
            version
        ),
    }

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

#[cfg(test)]
mod tests {
    //! The signature check, over a test-only key pair (tests/fixtures/
    //! release-signing: its private half was never kept). No network.
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/release-signing")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn key() -> String {
        String::from_utf8(fixture("test-release.pub")).unwrap()
    }

    #[test]
    fn a_good_signature_verifies() {
        let sums = fixture("SHA256SUMS");
        let sig = fixture("SHA256SUMS.sig");
        assert_eq!(
            check_signature(&key(), "v1.0.0", "1.2.0", &sums, Some(&sig), false).unwrap(),
            Signature::Verified
        );
    }

    #[test]
    fn changed_sums_or_another_keys_signature_is_refused() {
        let sums = fixture("SHA256SUMS");
        let sig = fixture("SHA256SUMS.sig");
        let mut changed = sums.clone();
        changed[0] = if changed[0] == b'0' { b'1' } else { b'0' };
        let err = check_signature(&key(), "v1.0.0", "1.2.0", &changed, Some(&sig), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not verify"), "{err}");
        assert!(err.contains("nothing was installed"), "{err}");

        let other = fixture("SHA256SUMS.other-key.sig");
        assert!(check_signature(&key(), "v1.0.0", "1.2.0", &sums, Some(&other), true).is_err());
        // garbage is refused, never a panic
        assert!(check_signature(&key(), "v1.0.0", "1.2.0", &sums, Some(b"nope"), false).is_err());
    }

    #[test]
    fn a_missing_signature_is_refused_from_the_first_signed_release_on() {
        let sums = fixture("SHA256SUMS");
        let err = check_signature(&key(), "v1.0.0", "1.0.0", &sums, None, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("has no SHA256SUMS.sig"), "{err}");
        assert_eq!(
            check_signature(&key(), "v1.0.0", "0.99.0", &sums, None, false).unwrap(),
            Signature::OlderThanSigning
        );
        assert_eq!(
            check_signature(&key(), "v1.0.0", "1.1.0", &sums, None, true).unwrap(),
            Signature::AllowedUnsigned
        );
    }

    #[test]
    fn with_no_pinned_key_only_the_checksum_is_checked() {
        let sums = fixture("SHA256SUMS");
        assert_eq!(
            check_signature("", "", "1.0.0", &sums, Some(b"anything"), false).unwrap(),
            Signature::NotSetUp
        );
    }

    #[test]
    fn a_key_that_is_not_p256_is_refused() {
        let sums = fixture("SHA256SUMS");
        let sig = fixture("SHA256SUMS.sig");
        let not_p256 = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=\n-----END PUBLIC KEY-----";
        assert!(verify_p256(not_p256, &sums, &sig).is_err());
    }
}

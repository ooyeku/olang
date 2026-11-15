//! olang's releases on GitHub: the list (`otc toolchain list
//! --available`, cached an hour), and installing one, verified.
//!
//! Releases install into `~/.olang/toolchains/<version>/bin/`. Downloads
//! come from the project's GitHub releases and are verified against the
//! release's `SHA256SUMS` before anything is installed, and `SHA256SUMS`
//! against its signature (`SHA256SUMS.sig`) by olang's release key, pinned
//! here as [`RELEASE_SIGNING_KEY`] (OL-491). Only the commands that plainly
//! ask for a release reach the network. `OLANG_GITHUB_API` and
//! `OLANG_GITHUB` say where GitHub is (the tests point them at a local
//! fixture server).

use crate::say;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const REPO: &str = "ooyeku/olang";

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
pub const RELEASE_SIGNING_KEY: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE36cXQoV1JHERlNekWav8gfY3uw6J
LvVlnGRVnXmoiR0E/OuqLIfeejOe9XvwIJjf+PwFwPWl6p0ghxHXjUBWIg==
-----END PUBLIC KEY-----";

/// The first signed release (`v1.0.0`), as `install.sh`'s `SIGNED_FROM`:
/// an older release carries no signature and installs on its checksum.
pub const SIGNED_FROM: &str = "v1.0.0";

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

/// One release of olang.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Release {
    /// The version, without the tag's `v`.
    pub version: String,
    /// The day it was published (`YYYY-MM-DD`).
    pub date: String,
    pub prerelease: bool,
}

/// The releases, newest first, and where the answer came from.
pub struct Listing {
    pub releases: Vec<Release>,
    /// Seconds since the answer was fetched.
    pub age_s: u64,
    /// Read from the cache rather than GitHub.
    pub cached: bool,
    /// Why GitHub was not asked or could not answer, when it was not.
    pub note: Option<String>,
}

/// How a listing may use the network.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fetch {
    /// The cache when under an hour old, else GitHub (the cache when
    /// GitHub cannot be reached).
    Cached,
    /// The cache only (`--offline`).
    Offline,
}

const CACHE_FOR_S: u64 = 3600;

fn api_base() -> String {
    std::env::var("OLANG_GITHUB_API")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://api.github.com".to_string())
        .trim_end_matches('/')
        .to_string()
}

fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_file() -> Option<PathBuf> {
    olang::home::cache().map(|c| c.join("releases.json"))
}

fn read_cache() -> Option<(u64, Vec<Release>)> {
    let text = std::fs::read_to_string(cache_file()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let at = v.get("fetched_at")?.as_u64()?;
    let releases = serde_json::from_value(v.get("releases")?.clone()).ok()?;
    Some((at, releases))
}

fn write_cache(releases: &[Release]) {
    let Some(f) = cache_file() else { return };
    if let Some(p) = f.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let text = serde_json::json!({ "format": 1, "fetched_at": now_s(), "releases": releases });
    let _ = std::fs::write(f, text.to_string());
}

/// Ask GitHub for every release (network).
fn fetch_releases() -> Result<Vec<Release>> {
    let client = client()?;
    let mut out = Vec::new();
    for page in 1..=5 {
        let url = format!(
            "{}/repos/{}/releases?per_page=100&page={}",
            api_base(),
            REPO,
            page
        );
        let resp = client
            .get(&url)
            .send()
            .with_context(|| format!("could not reach {}", api_base()))?
            .error_for_status()
            .with_context(|| format!("{url} answered with an error"))?;
        let items: Vec<serde_json::Value> =
            resp.json().context("the releases list was not JSON")?;
        let n = items.len();
        for r in items {
            if r["draft"].as_bool() == Some(true) {
                continue;
            }
            let Some(tag) = r["tag_name"].as_str() else {
                continue;
            };
            out.push(Release {
                version: normalize(tag),
                date: r["published_at"]
                    .as_str()
                    .or(r["created_at"].as_str())
                    .unwrap_or("")
                    .chars()
                    .take(10)
                    .collect(),
                prerelease: r["prerelease"].as_bool().unwrap_or(false),
            });
        }
        if n < 100 {
            break;
        }
    }
    out.sort_by(|a, b| version_cmp(&b.version, &a.version));
    Ok(out)
}

/// Versions in semver order (text where one does not parse).
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    olang::pkg::shelf::version_order(a, b)
}

/// The releases, as `how` allows.
pub fn list(how: Fetch) -> Result<Listing> {
    let cached = read_cache();
    let fresh = cached
        .as_ref()
        .is_some_and(|(at, _)| now_s().saturating_sub(*at) < CACHE_FOR_S);
    if how == Fetch::Offline || fresh {
        return match cached {
            Some((at, releases)) => Ok(Listing {
                releases,
                age_s: now_s().saturating_sub(at),
                cached: true,
                note: (how == Fetch::Offline)
                    .then(|| "offline: the releases as last fetched".to_string()),
            }),
            None => Err(crate::commands::fail(
                "no releases have been fetched yet, so there is nothing to show offline",
                "otc toolchain list --available",
            )),
        };
    }
    match fetch_releases() {
        Ok(releases) => {
            write_cache(&releases);
            Ok(Listing {
                releases,
                age_s: 0,
                cached: false,
                note: None,
            })
        }
        Err(e) => match cached {
            Some((at, releases)) => Ok(Listing {
                releases,
                age_s: now_s().saturating_sub(at),
                cached: true,
                note: Some(format!(
                    "GitHub could not be reached ({e:#}); the releases as last fetched"
                )),
            }),
            None => Err(crate::commands::fail(
                format!("GitHub could not be reached, and no list of releases is cached: {e:#}"),
                "otc toolchain list --available (once the network is back)",
            )),
        },
    }
}

/// Seconds as a person says an age: `3 min`, `2 h`, `5 days`.
pub fn age(s: u64) -> String {
    match s {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

pub fn normalize(v: &str) -> String {
    v.trim().trim_start_matches('v').to_string()
}

/// Whether release `a` is older than `b` (both as semver; false when
/// either does not read).
pub fn is_older(a: &str, b: &str) -> bool {
    match (
        semver::Version::parse(&normalize(a)),
        semver::Version::parse(&normalize(b)),
    ) {
        (Ok(a), Ok(b)) => a < b,
        _ => false,
    }
}

/// A toolchain's name as given, normalized, and checked to be one
/// folder's name under the toolchains folder: `otc toolchain remove ..`
/// removed the toolchains folder's parent, `~/.olang`, whole.
pub fn toolchain_name(v: &str) -> Result<String> {
    let v = normalize(v);
    if v.is_empty() || v == "." || v == ".." || v.contains(['/', '\\']) || v.starts_with('.') {
        bail!(
            "\"{}\" is not a toolchain's name (a version such as 1.0.0, or a name)",
            v
        );
    }
    Ok(v)
}

/// The checksum `SHA256SUMS` lists for exactly `file` (`<hex>  <file>`,
/// or `<hex> *<file>`): a line was taken when it merely ended with the
/// name, so `x-olang-…tar.gz` answered for `olang-…tar.gz`.
fn listed_sum(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, name) = l.trim_end().split_once(' ')?;
        let name = name.trim_start_matches(' ');
        let name = name.strip_prefix('*').unwrap_or(name);
        (name == file && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

pub fn toolchains_dir() -> Result<PathBuf> {
    olang::home::toolchains()
        .context("no home directory could be determined (set HOME or OLANG_HOME)")
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(format!("otc/{}", olang::VERSION))
        .build()?)
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

/// Download, verify and install release `version` (network). An
/// installed one is left as it is.
pub fn install_version(version: &str) -> Result<()> {
    let plat = platform()?;
    let dest = toolchains_dir()?.join(version);
    if dest.join("bin").join("olang").is_file() {
        say!("{} is already installed.", version);
        return Ok(());
    }
    let base = format!(
        "{}/{}/releases/download/v{}",
        olang::pkg::github::base(),
        REPO,
        version
    );
    let tarball = format!("olang-{}-{}.tar.gz", version, plat);

    say!("downloading {} ...", tarball);
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
        Signature::Verified => say!("signature verified: signed by olang's release key."),
        Signature::NotSetUp => say!(
            "signature NOT verified: this otc has no release key pinned yet, so only the \
             checksum below is checked, which proves the download is intact but not who \
             published it (dist/signing/README.md)."
        ),
        Signature::OlderThanSigning => say!(
            "release {} is older than olang's release signatures (they start at {}), \
             so only its checksum is checked.",
            version,
            SIGNED_FROM
        ),
        Signature::AllowedUnsigned => say!(
            "release {} has no signature; going on because OLANG_ALLOW_UNSIGNED=1.",
            version
        ),
    }

    let expected = listed_sum(&sums, &tarball)
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
    say!("checksum verified.");

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
    say!("installed {} to {}.", version, dest.display());
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
    fn the_checksum_is_the_named_files_alone() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let sums =
            format!("{a}  x-olang-1.0.0-macos-arm64.tar.gz\n{b}  olang-1.0.0-macos-arm64.tar.gz\n");
        assert_eq!(
            super::listed_sum(&sums, "olang-1.0.0-macos-arm64.tar.gz").as_deref(),
            Some(b.as_str())
        );
        assert_eq!(
            super::listed_sum(&format!("{a} *olang.tar.gz\n"), "olang.tar.gz").as_deref(),
            Some(a.as_str())
        );
        assert_eq!(
            super::listed_sum("zz  olang.tar.gz\n", "olang.tar.gz"),
            None
        );
        assert_eq!(
            super::listed_sum(&format!("{a}  x-olang.tar.gz\n"), "olang.tar.gz"),
            None
        );
    }

    #[test]
    fn a_toolchain_name_is_one_folder_and_update_never_goes_down() {
        for bad in ["..", ".", "", "../x", "a/b", "v..", ".hidden"] {
            assert!(super::toolchain_name(bad).is_err(), "{bad}");
        }
        assert_eq!(super::toolchain_name("v1.2.0").unwrap(), "1.2.0");
        assert_eq!(super::toolchain_name("dev").unwrap(), "dev");
        assert!(super::is_older("v1.0.0", "1.0.1"));
        assert!(super::is_older("1.0.0-rc.9", "1.0.0"));
        assert!(!super::is_older("v1.1.0", "1.0.0"));
        assert!(!super::is_older("1.0.0", "1.0.0"));
        assert!(!super::is_older("nonsense", "1.0.0"));
    }

    #[test]
    fn a_key_that_is_not_p256_is_refused() {
        let sums = fixture("SHA256SUMS");
        let sig = fixture("SHA256SUMS.sig");
        let not_p256 = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=\n-----END PUBLIC KEY-----";
        assert!(verify_p256(not_p256, &sums, &sig).is_err());
    }
}

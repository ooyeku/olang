//! Content-addressed cache for fetched dependency sources, and the git/path
//! fetching that populates it.
//!
//! Git dependencies are cloned once into `~/.olang/cache/git/<sha>/` keyed by
//! their resolved commit, so a revision is fetched a single time and shared
//! across every project. Path dependencies are used in place (never copied).
//! Fetching shells out to the `git` CLI — no native git dependency, matching
//! how the rest of the toolchain stays lean.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Errors during fetching or cache operations.
#[derive(Debug)]
pub enum CacheError {
    Io(std::io::Error),
    Git(String),
    NoHome,
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Io(e) => write!(f, "{}", e),
            CacheError::Git(e) => write!(f, "git: {}", e),
            CacheError::NoHome => write!(f, "cannot locate home directory for the cache"),
        }
    }
}

impl std::error::Error for CacheError {}

impl From<std::io::Error> for CacheError {
    fn from(e: std::io::Error) -> Self {
        CacheError::Io(e)
    }
}

/// The cache root, `~/.olang/cache` (overridable via `OLANG_CACHE` for tests).
pub fn cache_root() -> Result<PathBuf, CacheError> {
    if let Ok(dir) = std::env::var("OLANG_CACHE") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| CacheError::NoHome)?;
    Ok(PathBuf::from(home).join(".olang").join("cache"))
}

/// Run a git command in `dir`, returning trimmed stdout or an error carrying
/// stderr.
fn git(dir: Option<&Path>, args: &[&str]) -> Result<String, CacheError> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd.args(args);
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(CacheError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A git reference to resolve: a tag, an exact rev, a branch, or the default
/// branch (None of the above).
#[derive(Debug, Clone)]
pub enum GitRef {
    Tag(String),
    Rev(String),
    Branch(String),
    Default,
}

impl GitRef {
    /// The refspec to resolve into a commit SHA.
    fn spec(&self) -> &str {
        match self {
            GitRef::Tag(t) => t,
            GitRef::Rev(r) => r,
            GitRef::Branch(b) => b,
            GitRef::Default => "HEAD",
        }
    }
}

/// A fetched dependency: where its source lives, and the exact commit if git.
pub struct Fetched {
    pub dir: PathBuf,
    pub rev: Option<String>,
}

/// Clone (or reuse) a git dependency at a specific ref, returning the cached
/// checkout directory and the resolved commit SHA.
///
/// The cache is keyed by (url, resolved-sha), so the same commit is only ever
/// stored once. A previously cached checkout is reused without touching the
/// network.
pub fn fetch_git(url: &str, git_ref: &GitRef) -> Result<Fetched, CacheError> {
    let root = cache_root()?;
    let bare_dir = root.join("git-db").join(url_hash(url));
    std::fs::create_dir_all(&bare_dir)?;

    // Maintain a bare mirror per URL, updated on demand, so ref resolution is
    // cheap and offline after the first fetch.
    if !bare_dir.join("HEAD").exists() {
        git(
            None,
            &[
                "clone",
                "--bare",
                "--quiet",
                url,
                &bare_dir.to_string_lossy(),
            ],
        )?;
    } else {
        // Refresh refs (ignore failure so offline reuse still works). The
        // refspecs are explicit because a `clone --bare` repo has no fetch
        // refspec configured — a plain `fetch --all` would update nothing
        // and branch refs would stay frozen at clone time forever.
        let _ = git(
            Some(&bare_dir),
            &[
                "fetch",
                "--quiet",
                "--force",
                url,
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ],
        );
    }

    // Resolve the ref to an exact commit SHA.
    let rev = git(Some(&bare_dir), &["rev-parse", git_ref.spec()])?;

    // Check out that commit into a content-addressed directory.
    let checkout = root.join("git").join(&rev);
    if !checkout.exists() {
        std::fs::create_dir_all(&checkout)?;
        // Use a worktree-free checkout: archive the tree at the rev.
        let archive = git(Some(&bare_dir), &["archive", "--format=tar", &rev])?;
        // git archive to stdout then untar is awkward via strings; do it with a
        // pipe instead.
        checkout_via_archive(&bare_dir, &rev, &checkout)?;
        let _ = archive; // (the string form is unused; see checkout_via_archive)
    }

    Ok(Fetched {
        dir: checkout,
        rev: Some(rev),
    })
}

/// Extract the tree at `rev` from the bare repo into `dest` using
/// `git archive | tar -x`, which needs no working tree.
fn checkout_via_archive(bare: &Path, rev: &str, dest: &Path) -> Result<(), CacheError> {
    use std::process::Stdio;
    let mut archive = Command::new("git")
        .current_dir(bare)
        .args(["archive", "--format=tar", rev])
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = archive
        .stdout
        .take()
        .ok_or_else(|| CacheError::Git("failed to capture git archive output".to_string()))?;
    let tar = Command::new("tar")
        .current_dir(dest)
        .args(["-xf", "-"])
        .stdin(stdout)
        .output()?;
    let status = archive.wait()?;
    if !status.success() {
        return Err(CacheError::Git("git archive failed".to_string()));
    }
    if !tar.status.success() {
        return Err(CacheError::Git(
            String::from_utf8_lossy(&tar.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

/// sha256 of a URL, for stable per-repo cache directory names.
fn url_hash(url: &str) -> String {
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    hex::encode(h.finalize())[..16].to_string()
}

/// sha256 of a package's source tree: every `.ol` file, hashed in sorted path
/// order so the digest is stable and order-independent. Used as the lockfile
/// checksum to detect tampering on re-fetch.
pub fn checksum_dir(dir: &Path) -> Result<String, CacheError> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_ol_files(dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for file in &files {
        let rel = file.strip_prefix(dir).unwrap_or(file);
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0u8]);
        hasher.update(std::fs::read(file)?);
        hasher.update([0u8]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn collect_ol_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), CacheError> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // Skip nested caches / VCS dirs
        let name = entry.file_name();
        if name == ".git" || name == "target" {
            continue;
        }
        if path.is_dir() {
            collect_ol_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "ol") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_stable_and_content_sensitive() {
        let dir = std::env::temp_dir().join(format!("olang_pkg_ck_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.ol"), "share fn a() = 1").unwrap();
        std::fs::write(dir.join("b.ol"), "share fn b() = 2").unwrap();

        let c1 = checksum_dir(&dir).unwrap();
        let c2 = checksum_dir(&dir).unwrap();
        assert_eq!(c1, c2, "checksum must be deterministic");

        std::fs::write(dir.join("b.ol"), "share fn b() = 3").unwrap();
        let c3 = checksum_dir(&dir).unwrap();
        assert_ne!(c1, c3, "changing content must change the checksum");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

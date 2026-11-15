//! The library shelf: every library this machine holds, by name, with as
//! many versions of each as needed (docs/otc-design.md).
//!
//! A name on the shelf has at most one **local** folder (a working copy,
//! referenced where it is, never copied) and any number of **copies**:
//! read-only trees the shelf owns under `~/.olang/shelf/<name>/<version>/`,
//! each recording where it came from (GitHub, another git repository, a
//! folder snapshotted at a commit, or a starter library shipped in this
//! binary), its commit and a checksum of its sources. A project records
//! only the name (`my-lib = { shelf = "my-lib" }`, perhaps with a version
//! or a repository); the lockfile records the commit and checksum of the
//! copy it got, so another machine resolves the same code.
//!
//! Which copy a name gives a project that asks for no version is the
//! shelf's **rule 3**: the machine's default (`otc shelf use`), else the
//! local folder, else the newest copy held ([`Shelf::pick`]).
//!
//! Stored as one TOML file at `~/.olang/shelf.toml` (`OLANG_SHELF`
//! overrides the file, which is how tests isolate; the copies live in a
//! `shelf/` directory beside it). A library with only a local folder is
//! written as a plain string (`greeter = "/path"`), so the simple case
//! reads as it always did; any other is a table with a `path` key naming
//! what the name gives, for readers such as the web SDK.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The newest shelf.toml format this olang reads and writes. A newer
/// shelf is refused by name, never rewritten without what it holds.
pub const SHELF_VERSION: u32 = 2;

fn shelf_version() -> u32 {
    SHELF_VERSION
}

/// One held copy of a library.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShelfCopy {
    /// The library's version: its `[package] version`, else the tag's.
    pub version: String,
    /// `github`, `git`, `folder` (a local repository at a commit) or
    /// `starter` (shipped in this olang).
    pub source: String,
    /// `owner/repo`, a git URL, or the folder the snapshot was made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// What was asked for: a tag, a branch or a commit.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The full commit id the tree is (none for a starter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// sha256 of the copy's `.ol` files (`cache::checksum_dir`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    /// The copy's folder, relative to the shelf's home.
    pub dir: String,
    /// The day it was fetched (`YYYY-MM-DD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched: Option<String>,
}

impl ShelfCopy {
    /// The copy's folder.
    pub fn path(&self) -> PathBuf {
        Shelf::home().join(&self.dir)
    }

    /// The label a person names this copy by: its version, and the
    /// commit's first 12 digits after it when another copy shares the
    /// version.
    pub fn short_commit(&self) -> Option<String> {
        self.commit.as_ref().map(|c| c.chars().take(12).collect())
    }

    /// Whether `label` (a version, a commit prefix, or `version@commit`)
    /// names this copy.
    pub fn answers(&self, label: &str) -> bool {
        let label = label.trim().trim_start_matches('v');
        if label == self.version {
            return true;
        }
        if label.len() >= 7
            && label.bytes().all(|b| b.is_ascii_hexdigit())
            && self.commit.as_deref().is_some_and(|c| c.starts_with(label))
        {
            return true;
        }
        false
    }
}

/// A library on the shelf.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "LibraryToml", into = "LibraryToml")]
pub struct Library {
    /// A registered local folder (referenced, never copied).
    pub local: Option<PathBuf>,
    /// The machine's default for the bare name: `local`, a version, or a
    /// commit prefix (`otc shelf use`).
    pub default: Option<String>,
    /// The copies held, in the order they were made.
    pub copies: Vec<ShelfCopy>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum LibraryToml {
    Folder(PathBuf),
    Full {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        copies: Vec<ShelfCopy>,
    },
}

impl From<LibraryToml> for Library {
    fn from(t: LibraryToml) -> Self {
        match t {
            LibraryToml::Folder(p) => Library {
                local: Some(p),
                ..Default::default()
            },
            LibraryToml::Full {
                local,
                default,
                copies,
                ..
            } => Library {
                local,
                default,
                copies,
            },
        }
    }
}

impl From<Library> for LibraryToml {
    fn from(l: Library) -> Self {
        if l.copies.is_empty()
            && l.default.is_none()
            && let Some(p) = &l.local
        {
            return LibraryToml::Folder(p.clone());
        }
        let path = l.pick().map(|(p, _)| p.path());
        LibraryToml::Full {
            path,
            local: l.local,
            default: l.default,
            copies: l.copies,
        }
    }
}

/// What a name gives: its local folder, or one of its copies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pick<'a> {
    Local(&'a Path),
    Copy(&'a ShelfCopy),
}

impl<'a> Pick<'a> {
    pub fn path(&self) -> PathBuf {
        match self {
            Pick::Local(p) => p.to_path_buf(),
            Pick::Copy(c) => c.path(),
        }
    }
    /// The commit it is, for a copy that has one (a starter has none, a
    /// local folder follows its files).
    pub fn commit(&self) -> Option<&'a str> {
        match *self {
            Pick::Local(_) => None,
            Pick::Copy(c) => c.commit.as_deref(),
        }
    }
    pub fn version(&self) -> Option<String> {
        match self {
            Pick::Local(p) => folder_version(p),
            Pick::Copy(c) => Some(c.version.clone()),
        }
    }
}

/// Which part of rule 3 chose a pick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Why {
    /// `otc shelf use` set it.
    Default,
    /// The registered local folder.
    Local,
    /// The newest copy held.
    Remote,
}

impl Why {
    pub fn word(self) -> &'static str {
        match self {
            Why::Default => "default",
            Why::Local => "local",
            Why::Remote => "remote",
        }
    }
}

impl Library {
    /// Rule 3: the default, else the local folder, else the newest copy.
    pub fn pick(&self) -> Option<(Pick<'_>, Why)> {
        if let Some(d) = &self.default {
            if d == "local" {
                if let Some(p) = &self.local {
                    return Some((Pick::Local(p), Why::Default));
                }
            } else if let Some(c) = self.copies.iter().rev().find(|c| c.answers(d)) {
                return Some((Pick::Copy(c), Why::Default));
            }
        }
        if let Some(p) = &self.local {
            return Some((Pick::Local(p), Why::Local));
        }
        newest(self.copies.iter()).map(|c| (Pick::Copy(c), Why::Remote))
    }

    /// The newest copy (or the local folder) whose version `req` allows.
    /// With `from` (a dependency naming its repository), only that
    /// repository's copies; otherwise the local folder counts too, and a
    /// tie goes to it, then to the copy made last.
    pub fn select(&self, req: &str, from: Option<&str>) -> Option<Pick<'_>> {
        let copies = self
            .copies
            .iter()
            .filter(|c| from.is_none_or(|f| same_origin(c.from.as_deref(), f)))
            .filter(|c| version_matches(req, &c.version));
        let best = newest(copies);
        let local = self
            .local
            .as_deref()
            .filter(|_| from.is_none())
            .filter(|p| folder_version(p).is_some_and(|v| version_matches(req, &v)));
        match (best, local) {
            (Some(c), Some(l)) => {
                let lv = folder_version(l).unwrap_or_default();
                if version_order(&lv, &c.version) == std::cmp::Ordering::Less {
                    Some(Pick::Copy(c))
                } else {
                    Some(Pick::Local(l))
                }
            }
            (Some(c), None) => Some(Pick::Copy(c)),
            (None, Some(l)) => Some(Pick::Local(l)),
            (None, None) => None,
        }
    }

    /// The copy at commit `rev` (a prefix of 7 or more digits).
    pub fn by_commit(&self, rev: &str) -> Option<&ShelfCopy> {
        self.copies.iter().rev().find(|c| {
            c.commit
                .as_deref()
                .is_some_and(|c| c.starts_with(rev) || rev.starts_with(c))
        })
    }

    /// The newest copy of `from` made for the ref `reference` (a tag or a
    /// branch).
    pub fn by_ref(&self, from: Option<&str>, reference: &str) -> Option<&ShelfCopy> {
        self.copies.iter().rev().find(|c| {
            c.reference.as_deref() == Some(reference)
                && from.is_none_or(|f| same_origin(c.from.as_deref(), f))
        })
    }
}

/// Whether two origins (`owner/repo`, URLs, folders) are one.
pub fn same_origin(a: Option<&str>, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_ascii_lowercase()
    };
    a.is_some_and(|a| norm(a) == norm(b))
}

fn newest<'a>(copies: impl Iterator<Item = &'a ShelfCopy>) -> Option<&'a ShelfCopy> {
    // max_by keeps the last of equals: the copy made last wins a tie
    copies.max_by(|a, b| version_order(&a.version, &b.version))
}

/// Versions in order: by semver where both parse, else by text.
pub fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    match (
        semver::Version::parse(a.trim_start_matches('v')),
        semver::Version::parse(b.trim_start_matches('v')),
    ) {
        (Ok(x), Ok(y)) => x.cmp_precedence(&y).then_with(|| x.cmp(&y)),
        (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
        (Err(_), Ok(_)) => std::cmp::Ordering::Less,
        _ => a.cmp(b),
    }
}

/// Whether version `v` meets the requirement `req` as a manifest writes
/// it. A bare version is a prefix: `"1.1"` is any 1.1.x, `"1"` any 1.x,
/// `"1.1.0"` exactly that (prereleases only when named). One with an
/// operator (`^1.1`, `~1.1.2`, `>=1.0`, `=1.1.0`, `*`) is a semver
/// requirement.
pub fn version_matches(req: &str, v: &str) -> bool {
    let req = req.trim();
    let Ok(version) = semver::Version::parse(v.trim().trim_start_matches('v')) else {
        return req == v.trim();
    };
    if req.is_empty() || req == "*" {
        return version.pre.is_empty();
    }
    if req.starts_with(['^', '~', '>', '<', '=']) || req.contains(',') || req.contains('*') {
        return semver::VersionReq::parse(req).is_ok_and(|r| r.matches(&version));
    }
    let req = req.trim_start_matches('v');
    if let Ok(exact) = semver::Version::parse(req) {
        return exact == version;
    }
    let parts: Vec<&str> = req.split('.').collect();
    if parts.len() > 2 || parts.iter().any(|p| p.parse::<u64>().is_err()) {
        return false;
    }
    let n: Vec<u64> = parts.iter().map(|p| p.parse().unwrap_or(0)).collect();
    version.pre.is_empty() && version.major == n[0] && (n.len() < 2 || version.minor == n[1])
}

/// A folder's `[package] version`, if it has one.
pub fn folder_version(dir: &Path) -> Option<String> {
    crate::pkg::manifest::Manifest::load(dir)
        .ok()
        .map(|m| m.package.version.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Shelf {
    /// The format, `version = 2`.
    #[serde(default = "shelf_version")]
    pub version: u32,
    /// Every library held, by name.
    #[serde(default)]
    pub libraries: BTreeMap<String, Library>,
}

impl Default for Shelf {
    fn default() -> Self {
        Shelf {
            version: SHELF_VERSION,
            libraries: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub enum ShelfError {
    Io(String),
    Parse(String),
    /// The path being registered is not a directory, or carries no code.
    NotALibrary(String),
}

impl std::fmt::Display for ShelfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShelfError::Io(e) => write!(f, "{}", e),
            ShelfError::Parse(e) => write!(
                f,
                "invalid shelf.toml: {} (`otc doctor` says more; `otc doctor --fix` repairs an old layout)",
                e
            ),
            ShelfError::NotALibrary(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for ShelfError {}

/// The tables of a format-1 shelf this format folds into copies: a shelf
/// still holding them is moved by `otc doctor --fix`.
pub const OLD_TABLES: &[&str] = &["revs", "repos"];

impl Shelf {
    /// Where the shelf lives: `$OLANG_SHELF`, else `~/.olang/shelf.toml`.
    pub fn file() -> PathBuf {
        if let Ok(p) = std::env::var("OLANG_SHELF")
            && !p.is_empty()
        {
            return PathBuf::from(p);
        }
        crate::home::shelf_manifest().unwrap_or_else(|| PathBuf::from(".olang-shelf.toml"))
    }

    /// Where the shelf's copies live: a `shelf/` directory beside the file.
    pub fn home() -> PathBuf {
        Self::file()
            .parent()
            .map(|p| p.join("shelf"))
            .unwrap_or_else(|| PathBuf::from("shelf"))
    }

    /// Whether the shelf file is in the old layout `otc doctor --fix`
    /// moves (format 1 with pinned libraries, or copies under `pins/`).
    pub fn old_layout() -> bool {
        let text = std::fs::read_to_string(Self::file()).unwrap_or_default();
        let old_tables = toml::from_str::<toml::Table>(&text)
            .is_ok_and(|t| OLD_TABLES.iter().any(|k| t.contains_key(*k)));
        // old starters: folders the shelf made, registered as if local
        let home = Self::home();
        let local_in_home = toml::from_str::<Shelf>(&text).is_ok_and(|s| {
            s.libraries
                .values()
                .any(|l| l.local.as_ref().is_some_and(|p| p.starts_with(&home)))
        });
        old_tables || home.join("pins").is_dir() || local_in_home
    }

    /// Load the shelf; a missing file is an empty shelf, not an error.
    pub fn load() -> Result<Shelf, ShelfError> {
        let path = Self::file();
        if !path.exists() {
            return Ok(Shelf::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| ShelfError::Io(e.to_string()))?;
        Self::parse(&path, &text)
    }

    fn parse(path: &Path, text: &str) -> Result<Shelf, ShelfError> {
        // the version first, alone: a newer format may not parse as this one
        #[derive(Deserialize)]
        struct Head {
            #[serde(default = "shelf_version")]
            version: u32,
        }
        if let Ok(head) = toml::from_str::<Head>(text)
            && head.version > SHELF_VERSION
        {
            return Err(ShelfError::Io(format!(
                "{} is format version {}, written by a newer olang; this olang reads version {}: upgrade olang (otc update)",
                path.display(),
                head.version,
                SHELF_VERSION
            )));
        }
        if let Ok(t) = toml::from_str::<toml::Table>(text)
            && OLD_TABLES.iter().any(|k| t.contains_key(*k))
        {
            return Err(ShelfError::Io(format!(
                "{} is in the old shelf layout (libraries pinned under shelf/pins/): `otc doctor --fix` moves it to this olang's",
                path.display()
            )));
        }
        toml::from_str(text).map_err(|e| ShelfError::Parse(e.to_string()))
    }

    pub fn save(&self) -> Result<(), ShelfError> {
        let path = Self::file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ShelfError::Io(e.to_string()))?;
        }
        let mut out = self.clone();
        out.version = SHELF_VERSION;
        let text = toml::to_string_pretty(&out).map_err(|e| ShelfError::Parse(e.to_string()))?;
        // written whole, then renamed: a reader never sees half a shelf
        let staged = path.with_extension("toml.new");
        std::fs::write(&staged, text).map_err(|e| ShelfError::Io(e.to_string()))?;
        std::fs::rename(&staged, &path).map_err(|e| ShelfError::Io(e.to_string()))
    }

    /// The library `name`, if held.
    pub fn library(&self, name: &str) -> Option<&Library> {
        self.libraries.get(name)
    }

    /// The directory a bare shelf name resolves to (rule 3), if held.
    pub fn resolve(&self, name: &str) -> Option<PathBuf> {
        self.pick(name).map(|(p, _)| p.path())
    }

    /// Rule 3 for `name`: what it gives and why.
    pub fn pick(&self, name: &str) -> Option<(Pick<'_>, Why)> {
        self.libraries.get(name)?.pick()
    }

    /// The commit a bare name gives, when what it gives is a copy with
    /// one: the lock records it, so another machine resolves the same.
    pub fn rev_of(&self, name: &str) -> Option<&str> {
        self.pick(name).and_then(|(p, _)| p.commit())
    }

    /// Register a local folder for a library. The path must be a
    /// directory that looks like an olang library — an `index.ol` or an
    /// `olang.toml` naming it. Returns the name it registered under: the
    /// manifest's package name when one exists, the directory name
    /// otherwise, `explicit_name` overriding both. A default the person
    /// set to a copy is cleared: registering a folder says "use this".
    pub fn add(&mut self, path: &Path, explicit_name: Option<&str>) -> Result<String, ShelfError> {
        let dir = std::fs::canonicalize(path).map_err(|e| {
            ShelfError::NotALibrary(format!("'{}' is not reachable: {}", path.display(), e))
        })?;
        if !dir.is_dir() {
            return Err(ShelfError::NotALibrary(format!(
                "'{}' is not a directory",
                dir.display()
            )));
        }
        let name = library_name(&dir, explicit_name)?;
        let lib = self.libraries.entry(name.clone()).or_default();
        lib.local = Some(dir);
        lib.default = None;
        Ok(name)
    }

    /// Hold the tree at `tree` as a copy of library `name`: copied under
    /// the shelf's home, made read-only, its checksum taken. A copy of the
    /// same commit (or, for a starter, the same version) is replaced.
    /// Returns the copy as recorded.
    pub fn add_copy(
        &mut self,
        name: &str,
        tree: &Path,
        mut copy: ShelfCopy,
    ) -> Result<ShelfCopy, ShelfError> {
        check_library(tree)?;
        if !Self::is_name(name) {
            return Err(ShelfError::NotALibrary(format!(
                "{name:?} cannot name a library on the shelf (a plain name: letters, digits, - and _)"
            )));
        }
        let lib = self.libraries.entry(name.to_string()).or_default();
        // the same commit (or starter version) is one copy, replaced
        let same = |c: &ShelfCopy| match (&c.commit, &copy.commit) {
            (Some(a), Some(b)) => a == b,
            (None, None) => c.source == copy.source && c.version == copy.version,
            _ => false,
        };
        if let Some(i) = lib.copies.iter().position(same) {
            let old = lib.copies.remove(i);
            remove_tree(&old.path());
        }
        let taken = |dir: &str| lib.copies.iter().any(|c| c.dir == dir);
        let mut leaf = copy.version.clone();
        if taken(&format!("{name}/{leaf}")) || leaf.is_empty() {
            leaf = copy
                .short_commit()
                .unwrap_or_else(|| format!("{}-{}", copy.version, lib.copies.len()));
        }
        let rel = format!("{name}/{leaf}");
        let dest = Self::home().join(&rel);
        remove_tree(&dest);
        copy_tree(tree, &dest)
            .map_err(|e| ShelfError::Io(format!("copying to {}: {e}", dest.display())))?;
        copy.checksum = crate::pkg::cache::checksum_dir(&dest).ok();
        copy.dir = rel;
        if copy.fetched.is_none() {
            copy.fetched = Some(today());
        }
        make_read_only(&dest);
        lib.copies.push(copy.clone());
        Ok(copy)
    }

    /// Register a library as it stands at one commit of its repository:
    /// the tree at `rev` becomes a copy (source `folder`) and the name's
    /// default, so the name no longer follows the checkout. Returns the
    /// registered name and the full commit id.
    pub fn add_at_rev(
        &mut self,
        path: &Path,
        explicit_name: Option<&str>,
        rev: &str,
    ) -> Result<(String, String), ShelfError> {
        let repo = std::fs::canonicalize(path).map_err(|e| {
            ShelfError::NotALibrary(format!("'{}' is not reachable: {}", path.display(), e))
        })?;
        let (staging, sha) = Self::snapshot(&repo, rev)?;
        let result = (|| {
            let name = match explicit_name {
                Some(n) => n.to_string(),
                None => crate::pkg::manifest::Manifest::load(&staging)
                    .ok()
                    .map(|m| m.package.name)
                    .or_else(|| repo.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "library".to_string()),
            };
            let copy = self.add_copy(
                &name,
                &staging,
                ShelfCopy {
                    version: folder_version(&staging).unwrap_or_else(|| "0.0.0".to_string()),
                    source: "folder".to_string(),
                    from: Some(repo.display().to_string()),
                    reference: Some(rev.to_string()),
                    commit: Some(sha.clone()),
                    checksum: None,
                    dir: String::new(),
                    fetched: None,
                },
            )?;
            let lib = self.libraries.get_mut(&name).expect("just added");
            lib.default = copy.short_commit();
            Ok(name)
        })();
        let _ = std::fs::remove_dir_all(&staging);
        result.map(|name| (name, sha))
    }

    /// The tree of the library at `repo` as it stands at commit `rev`,
    /// unpacked into a staging directory: the directory and the full
    /// commit id. The library may sit below the repository's root; its
    /// own subtree is what is taken.
    fn snapshot(repo: &Path, rev: &str) -> Result<(PathBuf, String), ShelfError> {
        if rev.starts_with('-') {
            return Err(ShelfError::NotALibrary(format!(
                "{rev:?} is not a commit (a ref cannot start with '-')"
            )));
        }
        let git = |args: &[&str]| -> Result<std::process::Output, ShelfError> {
            std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .output()
                .map_err(|e| ShelfError::Io(format!("git: {}", e)))
        };
        let resolved = git(&["rev-parse", "--verify", &format!("{rev}^{{commit}}")])?;
        if !resolved.status.success() {
            return Err(ShelfError::NotALibrary(format!(
                "'{}' has no commit '{}': {} (`git -C {} log` shows its commits)",
                repo.display(),
                rev,
                String::from_utf8_lossy(&resolved.stderr).trim(),
                repo.display()
            )));
        }
        let sha = String::from_utf8_lossy(&resolved.stdout).trim().to_string();
        let prefix = git(&["rev-parse", "--show-prefix"])?;
        let prefix = String::from_utf8_lossy(&prefix.stdout).trim().to_string();
        let tree = if prefix.is_empty() {
            sha.clone()
        } else {
            format!("{}:{}", sha, prefix.trim_end_matches('/'))
        };
        // Run from the repository's top level: from inside a subdirectory
        // `git archive` narrows to that subdirectory as well, so
        // `<sha>:<prefix>` named a path under the prefix twice and the
        // archive came back empty.
        let top = git(&["rev-parse", "--show-toplevel"])?;
        let top = String::from_utf8_lossy(&top.stdout).trim().to_string();
        let archive = std::process::Command::new("git")
            .arg("-C")
            .arg(if top.is_empty() {
                repo
            } else {
                Path::new(&top)
            })
            .args(["archive", "--format=tar", &tree])
            .output()
            .map_err(|e| ShelfError::Io(format!("git: {}", e)))?;
        if !archive.status.success() {
            return Err(ShelfError::Io(format!(
                "git archive {}: {}",
                tree,
                String::from_utf8_lossy(&archive.stderr).trim()
            )));
        }
        let staging = Self::home().join(format!(".staging-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| ShelfError::Io(e.to_string()))?;
        let staging = std::fs::canonicalize(&staging).unwrap_or(staging);
        untar(&archive.stdout, &staging)?;
        Ok((staging, sha))
    }

    /// Whether a lock's `rev` and shelf name can name a copy: a commit id
    /// (7 to 64 hex digits) and a plain name, never a path. A lock is
    /// committed, and a clone's lock is not trusted to point outside the
    /// shelf.
    pub fn is_pin(name: &str, rev: &str) -> bool {
        (7..=64).contains(&rev.len())
            && rev.bytes().all(|b| b.is_ascii_hexdigit())
            && Self::is_name(name)
    }

    /// A plain library name: no path, no leading dot.
    pub fn is_name(name: &str) -> bool {
        !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\']) && name != ".."
    }

    /// The folder repository a library's snapshots can be made again
    /// from: the one a `folder` copy was made from, else its local folder.
    pub fn repo_of(&self, name: &str) -> Option<PathBuf> {
        let lib = self.libraries.get(name)?;
        lib.copies
            .iter()
            .rev()
            .find(|c| c.source == "folder")
            .and_then(|c| c.from.as_ref().map(PathBuf::from))
            .or_else(|| lib.local.clone())
    }

    /// The copy of library `name` at commit `rev`, whatever the name gives
    /// now. One that is not held is made again from the library's folder
    /// repository when the shelf knows one (`folder` copies); a GitHub
    /// copy is fetched by the caller (`pkg::github`). `Ok(None)`: not held
    /// and nothing to make it from.
    pub fn pinned_copy(&mut self, name: &str, rev: &str) -> Result<Option<PathBuf>, ShelfError> {
        if !Self::is_pin(name, rev) {
            return Err(ShelfError::NotALibrary(format!(
                "{:?} at {:?} does not name a commit of a shelf library",
                name, rev
            )));
        }
        if let Some(c) = self.libraries.get(name).and_then(|l| l.by_commit(rev))
            && c.path().is_dir()
        {
            return Ok(Some(c.path()));
        }
        let Some(repo) = self.repo_of(name) else {
            return Ok(None);
        };
        if !repo.exists() {
            return Ok(None);
        }
        let (staging, sha) = Self::snapshot(&repo, rev)?;
        if !sha.starts_with(rev) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(ShelfError::NotALibrary(format!(
                "'{}' names commit {} as {}, not the commit the lock pins",
                repo.display(),
                &rev[..rev.len().min(12)],
                sha
            )));
        }
        let made = self.add_copy(
            name,
            &staging,
            ShelfCopy {
                version: folder_version(&staging).unwrap_or_else(|| "0.0.0".to_string()),
                source: "folder".to_string(),
                from: Some(repo.display().to_string()),
                reference: Some(rev.to_string()),
                commit: Some(sha),
                checksum: None,
                dir: String::new(),
                fetched: None,
            },
        );
        let _ = std::fs::remove_dir_all(&staging);
        let made = made?;
        // made again, kept: the shelf file records it
        let _ = self.save();
        Ok(Some(made.path()))
    }

    /// The copy at commit `rev` of library `name`, read-only: what the
    /// shelf holds now, made again from nothing.
    pub fn held_copy(&self, name: &str, rev: &str) -> Option<PathBuf> {
        self.libraries
            .get(name)?
            .by_commit(rev)
            .map(ShelfCopy::path)
            .filter(|p| p.is_dir())
    }

    /// Load the shelf, seeding a brand-new one with the starter
    /// libraries. Seeding happens only when the shelf *file* does not
    /// exist yet — a shelf whose starters were deliberately removed
    /// stays exactly as its owner left it.
    pub fn load_or_seed() -> Result<Shelf, ShelfError> {
        if Self::file().exists() {
            return Self::load();
        }
        let mut shelf = Shelf::default();
        for lib in crate::pkg::starter::STARTERS {
            // A failed materialization skips that library rather than
            // failing the shelf: the shelf must work on a read-only
            // home, just with nothing pre-stocked.
            if shelf.restore_starter(lib.name).is_err() {
                shelf.libraries.remove(lib.name);
            }
        }
        shelf.save()?;
        Ok(shelf)
    }

    /// Materialize (or refresh) one starter library as a copy. Unknown
    /// names error, naming what exists.
    pub fn restore_starter(&mut self, name: &str) -> Result<PathBuf, ShelfError> {
        let lib = crate::pkg::starter::get(name).ok_or_else(|| {
            ShelfError::NotALibrary(format!(
                "'{}' is not a starter library (they are: {})",
                name,
                crate::pkg::starter::STARTERS
                    .iter()
                    .map(|s| s.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        let staging = Self::home().join(format!(".staging-starter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        crate::pkg::starter::materialize(lib, &staging)
            .map_err(|e| ShelfError::Io(e.to_string()))?;
        let made = self.add_copy(
            lib.name,
            &staging,
            ShelfCopy {
                version: crate::VERSION.to_string(),
                source: "starter".to_string(),
                from: None,
                reference: None,
                commit: None,
                checksum: None,
                dir: String::new(),
                fetched: None,
            },
        );
        let _ = std::fs::remove_dir_all(&staging);
        Ok(made?.path())
    }

    /// Is this path one the shelf itself made (a copy), as opposed to a
    /// user directory the shelf merely points at? Removal deletes such
    /// directories — the shelf owns them.
    pub fn owns(&self, path: &Path) -> bool {
        path.starts_with(Self::home())
    }

    /// Take library `name` off the shelf, or only its copy `label` (a
    /// version, a commit prefix, or `local` for the folder). Copies'
    /// folders are deleted; a local folder is only forgotten. Returns what
    /// was removed, as words.
    pub fn remove(&mut self, name: &str, label: Option<&str>) -> Result<Vec<String>, ShelfError> {
        let Some(lib) = self.libraries.get_mut(name) else {
            return Err(ShelfError::NotALibrary(format!(
                "'{name}' is not on your shelf (`otc shelf list` shows what is)"
            )));
        };
        let mut gone = Vec::new();
        match label {
            None => {
                for c in &lib.copies {
                    remove_tree(&c.path());
                    gone.push(format!("{name} {} ({})", c.version, c.source));
                }
                if let Some(l) = &lib.local {
                    gone.push(format!(
                        "{name} local folder {} (forgotten, not deleted)",
                        l.display()
                    ));
                }
                self.libraries.remove(name);
                let _ = std::fs::remove_dir(Self::home().join(name));
            }
            Some("local") => {
                let Some(l) = lib.local.take() else {
                    return Err(ShelfError::NotALibrary(format!(
                        "'{name}' has no local folder on your shelf (`otc shelf list {name}` shows what it has)"
                    )));
                };
                gone.push(format!(
                    "{name} local folder {} (forgotten, not deleted)",
                    l.display()
                ));
                if lib.default.as_deref() == Some("local") {
                    lib.default = None;
                }
            }
            Some(label) => {
                let before = lib.copies.len();
                lib.copies.retain(|c| {
                    if c.answers(label) {
                        remove_tree(&c.path());
                        gone.push(format!("{name} {} ({})", c.version, c.source));
                        false
                    } else {
                        true
                    }
                });
                if lib.copies.len() == before {
                    return Err(ShelfError::NotALibrary(format!(
                        "'{name}' has no copy {label} (`otc shelf list {name}` shows its versions)"
                    )));
                }
                if lib.default.as_deref().is_some_and(|d| d == label) {
                    lib.default = None;
                }
            }
        }
        if let Some(lib) = self.libraries.get(name)
            && lib.local.is_none()
            && lib.copies.is_empty()
        {
            self.libraries.remove(name);
        }
        Ok(gone)
    }
}

/// The name a library at `dir` registers under, checking it is one.
fn library_name(dir: &Path, explicit: Option<&str>) -> Result<String, ShelfError> {
    let manifest_name = crate::pkg::manifest::Manifest::load(dir)
        .ok()
        .map(|m| m.package.name);
    if !dir.join("index.ol").exists() && manifest_name.is_none() {
        return Err(ShelfError::NotALibrary(format!(
            "'{}' has neither an index.ol nor an olang.toml — `use` would find nothing there \
             (scaffold one with `otc new <name> --lib`)",
            dir.display()
        )));
    }
    let name = explicit
        .map(str::to_string)
        .or(manifest_name)
        .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "library".to_string());
    if !Shelf::is_name(&name) {
        return Err(ShelfError::NotALibrary(format!(
            "{name:?} cannot name a library on the shelf (pass --name <plain-name>)"
        )));
    }
    Ok(name)
}

/// Why `dir` is not an olang library, or Ok: it must hold an `olang.toml`
/// with a `[package]`, or an `index.ol`.
pub fn check_library(dir: &Path) -> Result<(), ShelfError> {
    let manifest = dir.join("olang.toml");
    if manifest.is_file() {
        let text = std::fs::read_to_string(&manifest).unwrap_or_default();
        let has_package = toml::from_str::<toml::Table>(&text)
            .is_ok_and(|t| t.get("package").is_some_and(|p| p.is_table()));
        if has_package {
            return Ok(());
        }
    }
    if dir.join("index.ol").is_file() {
        return Ok(());
    }
    Err(ShelfError::NotALibrary(
        "it is not an olang library: it has no olang.toml with a [package] and no index.ol"
            .to_string(),
    ))
}

/// Today, as `YYYY-MM-DD` (UTC).
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

/// Unpack a tar archive's bytes into `dest`.
pub fn untar(bytes: &[u8], dest: &Path) -> Result<(), ShelfError> {
    let mut child = std::process::Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(dest)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ShelfError::Io(format!("tar: {}", e)))?;
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("piped");
        stdin
            .write_all(bytes)
            .map_err(|e| ShelfError::Io(e.to_string()))?;
    }
    if !child
        .wait()
        .map_err(|e| ShelfError::Io(e.to_string()))?
        .success()
    {
        return Err(ShelfError::Io(
            "tar could not unpack the archive".to_string(),
        ));
    }
    Ok(())
}

/// Copy a tree (files and folders; `.git` left out).
pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let kind = entry.file_type()?;
        let target = to.join(&name);
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Make a copy read-only: files and folders lose their write bits.
pub fn make_read_only(dir: &Path) {
    set_writable(dir, false);
}

fn set_writable(path: &Path, writable: bool) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    if meta.is_dir() {
        // a folder is opened for writing before its children change
        if writable {
            set_mode(path, &meta, true);
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for e in entries.flatten() {
                set_writable(&e.path(), writable);
            }
        }
        if !writable {
            set_mode(path, &meta, false);
        }
    } else {
        set_mode(path, &meta, writable);
    }
}

fn set_mode(path: &Path, meta: &std::fs::Metadata, writable: bool) {
    let mut perms = meta.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perms.mode();
        perms.set_mode(if writable {
            mode | 0o200
        } else {
            mode & !0o222
        });
    }
    #[cfg(not(unix))]
    perms.set_readonly(!writable);
    let _ = std::fs::set_permissions(path, perms);
}

/// Delete a copy, read-only or not.
pub fn remove_tree(dir: &Path) {
    if dir.exists() {
        set_writable(dir, true);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_written_bare_is_a_prefix() {
        assert!(version_matches("1.1", "1.1.0"));
        assert!(version_matches("1.1", "1.1.9"));
        assert!(!version_matches("1.1", "1.2.0"));
        assert!(version_matches("1", "1.9.0"));
        assert!(!version_matches("1", "2.0.0"));
        assert!(version_matches("1.1.0", "1.1.0"));
        assert!(!version_matches("1.1.0", "1.1.1"));
        assert!(!version_matches("1.1", "1.1.0-rc.1"));
        assert!(version_matches("^1.1", "1.2.0"));
        assert!(version_matches("~1.1.2", "1.1.5"));
        assert!(!version_matches("~1.1.2", "1.2.0"));
        assert!(version_matches(">=1.0", "3.0.0"));
        assert!(version_matches("v1.1", "1.1.3"));
        assert!(!version_matches("banana", "1.0.0"));
    }

    #[test]
    fn rule_three_is_default_then_local_then_newest() {
        let copy = |v: &str, c: &str| ShelfCopy {
            version: v.to_string(),
            source: "github".to_string(),
            from: Some("o/geometry".to_string()),
            reference: Some(format!("v{v}")),
            commit: Some(c.to_string()),
            checksum: None,
            dir: format!("geometry/{v}"),
            fetched: None,
        };
        let mut lib = Library {
            local: None,
            default: None,
            copies: vec![copy("1.1.0", "aaaaaaaa1"), copy("1.2.0", "bbbbbbbb2")],
        };
        let (p, why) = lib.pick().unwrap();
        assert_eq!((p.version().unwrap().as_str(), why), ("1.2.0", Why::Remote));
        lib.local = Some(PathBuf::from("/nowhere/geometry"));
        assert_eq!(lib.pick().unwrap().1, Why::Local);
        lib.default = Some("1.1.0".to_string());
        let (p, why) = lib.pick().unwrap();
        assert_eq!(
            (p.version().unwrap().as_str(), why),
            ("1.1.0", Why::Default)
        );
        assert_eq!(lib.select("1.1", None).unwrap().version().unwrap(), "1.1.0");
        assert_eq!(
            lib.select("^1", Some("O/Geometry.git")).unwrap().commit(),
            Some("bbbbbbbb2")
        );
        assert!(lib.by_commit("bbbbbbbb").is_some());
    }

    #[test]
    fn a_folder_only_library_stays_a_string() {
        let mut shelf = Shelf::default();
        shelf.libraries.insert(
            "greeter".to_string(),
            Library {
                local: Some(PathBuf::from("/code/greeter")),
                ..Default::default()
            },
        );
        let text = toml::to_string_pretty(&shelf).unwrap();
        assert!(text.contains("greeter = \"/code/greeter\""), "{text}");
        let back: Shelf = toml::from_str(&text).unwrap();
        assert_eq!(back, shelf);
        assert!(Shelf::parse(Path::new("s"), "version = 1\n[libraries]\nx = \"/a\"\n").is_ok());
        let old = "version = 1\n[libraries]\nx = \"/a\"\n[revs]\nx = \"abc1234\"\n";
        let err = Shelf::parse(Path::new("s"), old).unwrap_err().to_string();
        assert!(err.contains("otc doctor --fix"), "{err}");
    }

    /// One test rather than several: the shelf location comes from a
    /// process-global environment variable, and parallel tests would
    /// race on it.
    #[test]
    fn shelf_round_trips_and_validates() {
        let dir = std::env::temp_dir().join(format!("olang_shelf_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: test-only env mutation, single-threaded within this test.
        unsafe { std::env::set_var("OLANG_SHELF", dir.join("shelf.toml")) };

        // Empty shelf loads from nothing.
        let mut shelf = Shelf::load().unwrap();
        assert!(shelf.libraries.is_empty());

        // A directory with no index.ol and no manifest is refused.
        let junk = dir.join("junk");
        std::fs::create_dir_all(&junk).unwrap();
        assert!(shelf.add(&junk, None).is_err());

        // A library shape registers under its manifest name.
        let lib = dir.join("greeting-lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(
            lib.join("olang.toml"),
            "[package]\nname = \"greetings\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(lib.join("index.ol"), "share fn hi() = \"hi\"\n").unwrap();
        let name = shelf.add(&lib, None).unwrap();
        assert_eq!(name, "greetings");

        // A copy of it, held beside the folder, read-only.
        let copy = shelf
            .add_copy(
                "greetings",
                &lib,
                ShelfCopy {
                    version: "0.1.0".to_string(),
                    source: "github".to_string(),
                    from: Some("me/greetings".to_string()),
                    reference: Some("v0.1.0".to_string()),
                    commit: Some("0123456789abcdef".to_string()),
                    checksum: None,
                    dir: String::new(),
                    fetched: None,
                },
            )
            .unwrap();
        assert_eq!(copy.dir, "greetings/0.1.0");
        assert!(copy.checksum.is_some());
        assert!(
            std::fs::metadata(copy.path().join("index.ol"))
                .unwrap()
                .permissions()
                .readonly()
        );
        shelf.save().unwrap();

        // Reload sees both; the bare name gives the local folder.
        let again = Shelf::load().unwrap();
        assert_eq!(
            again.resolve("greetings").unwrap(),
            std::fs::canonicalize(&lib).unwrap()
        );
        assert_eq!(again.library("greetings").unwrap().copies.len(), 1);
        let mut again = again;
        let gone = again.remove("greetings", Some("0.1.0")).unwrap();
        assert_eq!(gone.len(), 1);
        assert!(!copy.path().exists(), "a removed copy's folder goes");
        assert!(lib.exists(), "a local folder is never deleted");

        unsafe { std::env::remove_var("OLANG_SHELF") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}

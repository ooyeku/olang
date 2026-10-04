//! The files a built application carries (`olang build --app`): its
//! modules, every package's manifest, and the assets the packages
//! declare, keyed by the absolute paths they had on the build machine.
//! Module resolution and `asset.read` look here first and at the disk
//! second, so an application resolves its `use`s exactly as it did when
//! it was built, on a machine that has none of its files.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

struct Vfs {
    files: HashMap<PathBuf, Arc<[u8]>>,
    dirs: HashSet<PathBuf>,
    dependencies: HashMap<String, PathBuf>,
}

static VFS: OnceLock<Vfs> = OnceLock::new();

/// Install the application's files (once, before it runs).
pub fn install(files: HashMap<PathBuf, Arc<[u8]>>, dependencies: HashMap<String, PathBuf>) {
    let mut dirs = HashSet::new();
    for path in files.keys() {
        let mut at = path.parent();
        while let Some(d) = at {
            if !dirs.insert(d.to_path_buf()) {
                break;
            }
            at = d.parent();
        }
    }
    note_package_roots(&dependencies);
    let _ = VFS.set(Vfs {
        files,
        dirs,
        dependencies,
    });
}

/// Is an application's files installed?
pub fn active() -> bool {
    VFS.get().is_some()
}

/// The dependency map the application was built with.
pub fn dependencies() -> Option<HashMap<String, PathBuf>> {
    VFS.get().map(|v| v.dependencies.clone())
}

/// `a/b/../c/./d` as `a/c/d`: `.` dropped and `..` taken back, without
/// touching the filesystem (a symlink keeps its name).
pub fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The bytes of `path` from the application, if it carries the file.
pub fn read(path: &Path) -> Option<Arc<[u8]>> {
    VFS.get()?.files.get(&normalize(path)).cloned()
}

/// `path` exists, in the application or on disk.
pub fn exists(path: &Path) -> bool {
    VFS.get().is_some_and(|v| {
        let p = normalize(path);
        v.files.contains_key(&p) || v.dirs.contains(&p)
    }) || path.exists()
}

/// `path` is a file, in the application or on disk.
pub fn is_file(path: &Path) -> bool {
    VFS.get()
        .is_some_and(|v| v.files.contains_key(&normalize(path)))
        || path.is_file()
}

/// `path` is a directory, in the application or on disk.
pub fn is_dir(path: &Path) -> bool {
    VFS.get().is_some_and(|v| v.dirs.contains(&normalize(path))) || path.is_dir()
}

/// The text of `path`: the application's copy, else the disk's.
pub fn read_to_string(path: &Path) -> std::io::Result<String> {
    match read(path) {
        Some(bytes) => String::from_utf8(bytes.to_vec())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        None => std::fs::read_to_string(path),
    }
}

/// The bytes of `path`: the application's copy, else the disk's.
pub fn read_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    match read(path) {
        Some(bytes) => Ok(bytes.to_vec()),
        None => std::fs::read(path),
    }
}

/// The entries directly in `dir` that the application carries.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let Some(v) = VFS.get() else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = v
        .files
        .keys()
        .chain(v.dirs.iter())
        .filter(|p| p.parent() == Some(dir))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The root directory of each package the running program knows by name
/// (its own and its dependencies'): what `asset.read(package, path)`
/// resolves against. Filled wherever an interpreter's dependency map is.
static ROOTS: std::sync::RwLock<Option<HashMap<String, PathBuf>>> = std::sync::RwLock::new(None);

/// Learn package roots (merged with those already known).
pub fn note_package_roots(map: &HashMap<String, PathBuf>) {
    if let Ok(mut r) = ROOTS.write() {
        let roots = r.get_or_insert_with(HashMap::new);
        for (k, v) in map {
            roots.insert(k.clone(), v.clone());
        }
    }
}

/// The root of package `name`, if the program knows it.
pub fn package_root(name: &str) -> Option<PathBuf> {
    ROOTS.read().ok()?.as_ref()?.get(name).cloned()
}

/// The application section of a built binary: an index and the bytes.
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct AppIndex {
    /// `(path, offset, length)` into the data that follows the index.
    pub files: Vec<(String, u64, u64)>,
    pub dependencies: HashMap<String, String>,
}

/// Serialize an application section: `[index_len u64][index json][data]`.
pub fn encode(files: &[(PathBuf, Vec<u8>)], dependencies: &HashMap<String, PathBuf>) -> Vec<u8> {
    let mut index = AppIndex::default();
    let mut data = Vec::new();
    for (path, bytes) in files {
        index.files.push((
            normalize(path).to_string_lossy().to_string(),
            data.len() as u64,
            bytes.len() as u64,
        ));
        data.extend_from_slice(bytes);
    }
    index.dependencies = dependencies
        .iter()
        .map(|(k, v)| (k.clone(), normalize(v).to_string_lossy().to_string()))
        .collect();
    let json = serde_json::to_vec(&index).unwrap_or_default();
    let mut out = Vec::with_capacity(8 + json.len() + data.len());
    out.extend_from_slice(&(json.len() as u64).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&data);
    out
}

/// Read an application section back: the files and the dependency map.
pub type Decoded = (HashMap<PathBuf, Arc<[u8]>>, HashMap<String, PathBuf>);

pub fn decode(section: &[u8]) -> Option<Decoded> {
    let n = u64::from_le_bytes(section.get(0..8)?.try_into().ok()?) as usize;
    let index: AppIndex = serde_json::from_slice(section.get(8..8usize.checked_add(n)?)?).ok()?;
    let data = section.get(8 + n..)?;
    let mut files = HashMap::new();
    for (path, offset, len) in index.files {
        let (o, l) = (offset as usize, len as usize);
        let bytes = data.get(o..o.checked_add(l)?)?;
        files.insert(PathBuf::from(path), Arc::from(bytes));
    }
    let deps = index
        .dependencies
        .into_iter()
        .map(|(k, v)| (k, PathBuf::from(v)))
        .collect();
    Some((files, deps))
}

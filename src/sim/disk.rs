//! A node's disk (S3): an in-memory file system in which every file has
//! two states, what the node sees and what would survive the power going
//! out. Pure bookkeeping with no threads and no world in it — `fs.rs`
//! (the `fs.*` builtins) and `vfs.rs` (SQLite) drive it under the world's
//! lock — so every rule here is unit-tested on its own.
//!
//! **The model.** A file is an inode with its current bytes, its durable
//! bytes, and the writes made since it was last synced (`pending`), in
//! order. The namespace is two maps of path to entry: the current one and
//! the durable one. Writes, truncations, creations, removals and renames
//! change the current state only; durability comes from a sync:
//!
//! - **`sync(file)`** makes the file's bytes durable *and* its own entry
//!   (its name and the directories above it). Many real file systems do
//!   persist a new file's entry with its first fsync; POSIX does not
//!   promise it. A program that wants the stricter rule syncs the
//!   directory too, and loses nothing by it here.
//! - **`sync(dir)`** makes the directory's entries durable: what was
//!   created, removed or renamed directly in it.
//!
//! **A crash** (power lost: `w.crash`) keeps the durable state only: the
//! durable namespace comes back, each surviving file with its durable
//! bytes, and an inode no durable name reaches is gone. With
//! `torn_writes`, a crash may also keep part of what was never synced:
//! each pending write lands whole, lands torn (only its first sectors, at
//! 512-byte boundaries), or not at all, in the order it was made — the
//! disk wrote some of its cache before the power went. A process that is
//! killed (`w.kill`, `os.exit`) loses no write: what it wrote is in the
//! operating system's cache, and stays pending until a sync or a crash.
//!
//! **Faults** are drawn by the caller from the world's seeded streams and
//! passed in (a sync that lies is a `sync` told to lie), so this module
//! stays a pure function of what it is asked.

use rand::{Rng, RngExt};
use std::collections::BTreeMap;

/// A sector: the unit a torn write is cut at.
pub const SECTOR: u64 = 512;

/// How a disk misbehaves (`sim.disk(#{ ... })`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiskOpts {
    /// The probability that a sync (file or directory) reports success
    /// and makes nothing durable.
    pub fsync_lies: f64,
    /// A crash may keep part of what was not synced (see the module doc).
    pub torn_writes: bool,
    /// Writes that would take the disk past this many bytes fail with
    /// "No space left on device".
    pub full_at: Option<u64>,
    /// Multiplies the time a sync takes (1.0: one to three ms).
    pub slow: f64,
}

impl Default for DiskOpts {
    fn default() -> Self {
        DiskOpts {
            fsync_lies: 0.0,
            torn_writes: false,
            full_at: None,
            slow: 1.0,
        }
    }
}

impl DiskOpts {
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0..=1.0).contains(&self.fsync_lies) {
            return Err("`fsync_lies` is a probability in 0..1".into());
        }
        if !(self.slow.is_finite() && self.slow >= 0.0) {
            return Err("`slow` is a non-negative factor".into());
        }
        Ok(())
    }
}

/// One write the disk has not been told to keep.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    Write { at: u64, bytes: Vec<u8> },
    Truncate(u64),
}

#[derive(Debug, Clone, Default)]
struct Inode {
    data: Vec<u8>,
    durable: Vec<u8>,
    pending: Vec<Op>,
    /// Virtual wall-clock ms of the last change (`fs.file_info`).
    modified: i64,
}

impl Inode {
    fn apply(data: &mut Vec<u8>, op: &Op) {
        match op {
            Op::Write { at, bytes } => {
                let at = *at as usize;
                let end = at + bytes.len();
                if data.len() < end {
                    data.resize(end, 0);
                }
                data[at..end].copy_from_slice(bytes);
            }
            Op::Truncate(len) => data.resize(*len as usize, 0),
        }
    }

    fn change(&mut self, op: Op, now: i64) {
        Self::apply(&mut self.data, &op);
        self.pending.push(op);
        self.modified = now;
    }
}

/// What a path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    File(u64),
    Dir,
}

/// What a disk has done, for a run's report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskStats {
    pub writes: u64,
    pub syncs: u64,
    /// Syncs that said yes and kept nothing.
    pub lies: u64,
    /// Pending writes a crash threw away.
    pub lost: u64,
    /// Pending writes a crash kept only in part.
    pub torn: u64,
    /// Writes refused for want of space.
    pub full: u64,
    pub crashes: u64,
}

/// A node's disk.
#[derive(Debug, Clone)]
pub struct Disk {
    pub opts: DiskOpts,
    inodes: BTreeMap<u64, Inode>,
    next_ino: u64,
    names: BTreeMap<String, Entry>,
    durable: BTreeMap<String, Entry>,
    /// Bumped by every crash or kill: a file a dead process opened is
    /// not this process's (`vfs.rs` checks it).
    pub boot: u64,
    pub stats: DiskStats,
}

pub type Res<T> = Result<T, String>;

pub const NOT_FOUND: &str = "No such file or directory (os error 2)";
pub const EXISTS: &str = "File exists (os error 17)";
pub const NOT_DIR: &str = "Not a directory (os error 20)";
pub const IS_DIR: &str = "Is a directory (os error 21)";
pub const NOT_EMPTY: &str = "Directory not empty (os error 66)";
pub const NO_SPACE: &str = "No space left on device (os error 28)";

/// `path` made absolute against `cwd` and normalized: no `.`, no `..`,
/// no doubled or trailing `/`. `/` is the root.
pub fn normalize(cwd: &str, path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let joined;
    let full = if path.starts_with('/') {
        path
    } else {
        joined = format!("{cwd}/{path}");
        &joined
    };
    for part in full.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The directory above `path` (`/` for `/` itself).
pub fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

fn under(dir: &str, path: &str) -> bool {
    if dir == "/" {
        path != "/"
    } else {
        path.len() > dir.len() && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'/'
    }
}

fn child_of(dir: &str, path: &str) -> bool {
    under(dir, path) && parent(path) == dir
}

impl Disk {
    pub fn new(opts: DiskOpts) -> Disk {
        let mut names = BTreeMap::new();
        names.insert("/".to_string(), Entry::Dir);
        Disk {
            opts,
            inodes: BTreeMap::new(),
            next_ino: 1,
            durable: names.clone(),
            names,
            boot: 0,
            stats: DiskStats::default(),
        }
    }

    /// A file the node starts with, durable already (its directories
    /// made as needed).
    pub fn seed_file(&mut self, path: &str, bytes: Vec<u8>) {
        let path = normalize("/", path);
        let mut dir = parent(&path).to_string();
        let mut dirs = Vec::new();
        while dir != "/" {
            dirs.push(dir.clone());
            dir = parent(&dir).to_string();
        }
        for d in dirs {
            self.names.insert(d.clone(), Entry::Dir);
            self.durable.insert(d, Entry::Dir);
        }
        let ino = self.next_ino;
        self.next_ino += 1;
        self.inodes.insert(
            ino,
            Inode {
                durable: bytes.clone(),
                data: bytes,
                ..Inode::default()
            },
        );
        self.names.insert(path.clone(), Entry::File(ino));
        self.durable.insert(path, Entry::File(ino));
    }

    /// A directory the node starts with, durable already.
    pub fn seed_dir(&mut self, path: &str) {
        let path = normalize("/", path);
        self.names.insert(path.clone(), Entry::Dir);
        self.durable.insert(path, Entry::Dir);
    }

    pub fn entry(&self, path: &str) -> Option<Entry> {
        self.names.get(path).copied()
    }

    pub fn exists(&self, path: &str) -> bool {
        self.names.contains_key(path)
    }

    pub fn is_file(&self, path: &str) -> bool {
        matches!(self.entry(path), Some(Entry::File(_)))
    }

    pub fn is_dir(&self, path: &str) -> bool {
        matches!(self.entry(path), Some(Entry::Dir))
    }

    fn file(&self, path: &str) -> Res<u64> {
        match self.entry(path) {
            Some(Entry::File(ino)) => Ok(ino),
            Some(Entry::Dir) => Err(IS_DIR.into()),
            None => Err(NOT_FOUND.into()),
        }
    }

    fn need_dir(&self, path: &str) -> Res<()> {
        match self.entry(path) {
            Some(Entry::Dir) => Ok(()),
            Some(Entry::File(_)) => Err(NOT_DIR.into()),
            None => Err(NOT_FOUND.into()),
        }
    }

    pub fn read(&self, path: &str) -> Res<&[u8]> {
        let ino = self.file(path)?;
        Ok(&self.inodes[&ino].data)
    }

    /// Bytes in use: every live inode's current length.
    pub fn used(&self) -> u64 {
        self.inodes.values().map(|i| i.data.len() as u64).sum()
    }

    fn room_for(&mut self, grow: u64) -> Res<()> {
        if let Some(limit) = self.opts.full_at
            && grow > 0
            && self.used() + grow > limit
        {
            self.stats.full += 1;
            return Err(NO_SPACE.into());
        }
        Ok(())
    }

    /// Open (and with `create`, make) the file at `path`: its inode.
    pub fn open(&mut self, path: &str, create: bool, exclusive: bool) -> Res<u64> {
        match self.entry(path) {
            Some(Entry::File(ino)) if !exclusive => Ok(ino),
            Some(Entry::File(_)) => Err(EXISTS.into()),
            Some(Entry::Dir) => Err(IS_DIR.into()),
            None if !create => Err(NOT_FOUND.into()),
            None => {
                self.need_dir(parent(path))?;
                let ino = self.next_ino;
                self.next_ino += 1;
                self.inodes.insert(ino, Inode::default());
                self.names.insert(path.to_string(), Entry::File(ino));
                Ok(ino)
            }
        }
    }

    pub fn len_of(&self, ino: u64) -> u64 {
        self.inodes
            .get(&ino)
            .map(|i| i.data.len() as u64)
            .unwrap_or(0)
    }

    pub fn modified(&self, path: &str) -> Option<i64> {
        match self.entry(path)? {
            Entry::File(ino) => Some(self.inodes[&ino].modified),
            Entry::Dir => None,
        }
    }

    /// Read up to `buf.len()` bytes of inode `ino` at `at`: how many.
    pub fn read_at(&self, ino: u64, at: u64, buf: &mut [u8]) -> usize {
        let Some(inode) = self.inodes.get(&ino) else {
            return 0;
        };
        let at = at as usize;
        if at >= inode.data.len() {
            return 0;
        }
        let n = buf.len().min(inode.data.len() - at);
        buf[..n].copy_from_slice(&inode.data[at..at + n]);
        n
    }

    pub fn write_at(&mut self, ino: u64, at: u64, bytes: &[u8], now: i64) -> Res<()> {
        let len = self.len_of(ino);
        self.room_for((at + bytes.len() as u64).saturating_sub(len))?;
        let inode = self.inodes.get_mut(&ino).ok_or(NOT_FOUND)?;
        inode.change(
            Op::Write {
                at,
                bytes: bytes.to_vec(),
            },
            now,
        );
        self.stats.writes += 1;
        Ok(())
    }

    pub fn truncate(&mut self, ino: u64, len: u64, now: i64) -> Res<()> {
        let have = self.len_of(ino);
        self.room_for(len.saturating_sub(have))?;
        let inode = self.inodes.get_mut(&ino).ok_or(NOT_FOUND)?;
        inode.change(Op::Truncate(len), now);
        self.stats.writes += 1;
        Ok(())
    }

    /// `fs.write_file` (replace) or `fs.append_file` (append).
    pub fn write(&mut self, path: &str, bytes: &[u8], append: bool, now: i64) -> Res<()> {
        let existing = match self.entry(path) {
            Some(Entry::File(ino)) => Some(ino),
            Some(Entry::Dir) => return Err(IS_DIR.into()),
            None => {
                self.need_dir(parent(path))?;
                None
            }
        };
        let old = existing.map(|i| self.len_of(i)).unwrap_or(0);
        let grow = if append {
            bytes.len() as u64
        } else {
            (bytes.len() as u64).saturating_sub(old)
        };
        self.room_for(grow)?;
        let ino = self.open(path, true, false)?;
        if append {
            self.write_at(ino, old, bytes, now)
        } else {
            self.truncate(ino, 0, now)?;
            self.write_at(ino, 0, bytes, now)
        }
    }

    pub fn list(&self, dir: &str) -> Res<Vec<String>> {
        self.need_dir(dir)?;
        Ok(self
            .names
            .keys()
            .filter(|p| child_of(dir, p))
            .map(|p| p.rsplit('/').next().unwrap_or("").to_string())
            .collect())
    }

    /// Every file under `dir`, as absolute paths, sorted.
    pub fn walk(&self, dir: &str) -> Res<Vec<String>> {
        self.need_dir(dir)?;
        Ok(self
            .names
            .iter()
            .filter(|(p, e)| under(dir, p) && matches!(e, Entry::File(_)))
            .map(|(p, _)| p.clone())
            .collect())
    }

    /// Every path on the disk, files and directories.
    pub fn paths(&self) -> impl Iterator<Item = (&String, &Entry)> {
        self.names.iter()
    }

    pub fn mkdir(&mut self, path: &str) -> Res<()> {
        if self.exists(path) {
            return Err(EXISTS.into());
        }
        self.need_dir(parent(path))?;
        self.names.insert(path.to_string(), Entry::Dir);
        Ok(())
    }

    pub fn mkdir_all(&mut self, path: &str) -> Res<()> {
        match self.entry(path) {
            Some(Entry::Dir) => return Ok(()),
            Some(Entry::File(_)) => return Err(EXISTS.into()),
            None => {}
        }
        if path != "/" {
            self.mkdir_all(parent(path))?;
        }
        self.names.insert(path.to_string(), Entry::Dir);
        Ok(())
    }

    pub fn unlink(&mut self, path: &str) -> Res<()> {
        self.file(path)?;
        self.names.remove(path);
        self.forget_unreachable();
        Ok(())
    }

    pub fn rmdir(&mut self, path: &str) -> Res<()> {
        self.need_dir(path)?;
        if path == "/" || self.names.keys().any(|p| under(path, p)) {
            return Err(NOT_EMPTY.into());
        }
        self.names.remove(path);
        Ok(())
    }

    pub fn rmdir_all(&mut self, path: &str) -> Res<()> {
        self.need_dir(path)?;
        if path == "/" {
            return Err("refusing to remove the root".into());
        }
        self.names.retain(|p, _| p != path && !under(path, p));
        self.forget_unreachable();
        Ok(())
    }

    /// `rename(2)`: a file or a directory (with what is under it) moves;
    /// a file at `to` is replaced.
    pub fn rename(&mut self, from: &str, to: &str) -> Res<()> {
        let entry = self.entry(from).ok_or(NOT_FOUND)?;
        if from == to {
            return Ok(());
        }
        self.need_dir(parent(to))?;
        match (entry, self.entry(to)) {
            (Entry::File(_), Some(Entry::Dir)) => return Err(IS_DIR.into()),
            (Entry::Dir, Some(Entry::File(_))) => return Err(NOT_DIR.into()),
            (Entry::Dir, Some(Entry::Dir)) if self.names.keys().any(|p| under(to, p)) => {
                return Err(NOT_EMPTY.into());
            }
            _ => {}
        }
        if entry == Entry::Dir && under(from, to) {
            return Err("Invalid argument (os error 22)".into());
        }
        let moved: Vec<(String, Entry)> = self
            .names
            .iter()
            .filter(|(p, _)| *p == from || under(from, p))
            .map(|(p, e)| (format!("{to}{}", &p[from.len()..]), *e))
            .collect();
        self.names.retain(|p, _| p != from && !under(from, p));
        for (p, e) in moved {
            self.names.insert(p, e);
        }
        self.forget_unreachable();
        Ok(())
    }

    pub fn copy(&mut self, from: &str, to: &str, now: i64) -> Res<()> {
        let bytes = self.read(from)?.to_vec();
        self.write(to, &bytes, false, now)
    }

    /// Inodes neither namespace reaches are gone (an open handle in
    /// `vfs.rs` keeps reading an unlinked file through its own copy of
    /// nothing: SQLite never reads a file it deleted).
    fn forget_unreachable(&mut self) {
        let live: std::collections::BTreeSet<u64> = self
            .names
            .values()
            .chain(self.durable.values())
            .filter_map(|e| match e {
                Entry::File(i) => Some(*i),
                Entry::Dir => None,
            })
            .collect();
        self.inodes.retain(|i, _| live.contains(i));
    }

    fn persist_entry(&mut self, path: &str) {
        if let Some(e) = self.entry(path) {
            self.durable.insert(path.to_string(), e);
        }
        let mut dir = parent(path).to_string();
        loop {
            self.durable.insert(dir.clone(), Entry::Dir);
            if dir == "/" {
                break;
            }
            dir = parent(&dir).to_string();
        }
    }

    /// Make inode `ino` durable, and its name `path` with it when `path`
    /// still names it. `lie`: the sync says yes and keeps nothing.
    pub fn sync_ino(&mut self, ino: u64, path: Option<&str>, lie: bool) {
        self.stats.syncs += 1;
        if lie {
            self.stats.lies += 1;
            return;
        }
        if let Some(inode) = self.inodes.get_mut(&ino) {
            inode.durable = inode.data.clone();
            inode.pending.clear();
        }
        if let Some(path) = path
            && self.entry(path) == Some(Entry::File(ino))
        {
            self.persist_entry(path);
        }
    }

    /// `fs.sync(path)`: a file's bytes and entry, or a directory's
    /// entries.
    pub fn sync(&mut self, path: &str, lie: bool) -> Res<()> {
        match self.entry(path).ok_or(NOT_FOUND)? {
            Entry::File(ino) => {
                self.sync_ino(ino, Some(path), lie);
                Ok(())
            }
            Entry::Dir => {
                self.sync_dir(path, lie);
                Ok(())
            }
        }
    }

    /// A directory's entries made durable: what was created, removed or
    /// renamed directly in it. (A file's bytes still need their own sync.)
    pub fn sync_dir(&mut self, dir: &str, lie: bool) {
        self.stats.syncs += 1;
        if lie {
            self.stats.lies += 1;
            return;
        }
        let gone: Vec<String> = self
            .durable
            .keys()
            .filter(|p| child_of(dir, p) && !self.names.contains_key(*p))
            .cloned()
            .collect();
        for p in gone {
            self.durable.retain(|d, _| *d != p && !under(&p, d));
        }
        let here: Vec<(String, Entry)> = self
            .names
            .iter()
            .filter(|(p, _)| child_of(dir, p))
            .map(|(p, e)| (p.clone(), *e))
            .collect();
        for (p, e) in here {
            // A file renamed in keeps the bytes it had durable before.
            self.durable.insert(p, e);
        }
        if self.is_dir(dir) {
            self.persist_entry(dir);
        }
        self.forget_unreachable();
    }

    /// Is anything waiting for a sync?
    pub fn dirty(&self) -> bool {
        self.names != self.durable || self.inodes.values().any(|i| !i.pending.is_empty())
    }

    /// The power goes out: only the durable state is left (with
    /// `torn_writes`, plus whatever part of the pending writes `rng`
    /// lets land). A summary of what was lost.
    pub fn crash(&mut self, rng: &mut impl Rng) -> String {
        self.stats.crashes += 1;
        self.boot += 1;
        let mut lost = 0u64;
        let mut torn = 0u64;
        let mut kept = 0u64;
        for inode in self.inodes.values_mut() {
            let mut data = std::mem::take(&mut inode.durable);
            for op in std::mem::take(&mut inode.pending) {
                if !self.opts.torn_writes {
                    lost += 1;
                    continue;
                }
                let draw: f64 = rng.random();
                if draw < 1.0 / 3.0 {
                    lost += 1;
                } else if draw < 2.0 / 3.0 {
                    Inode::apply(&mut data, &op);
                    kept += 1;
                } else {
                    match op {
                        Op::Write { at, bytes } => {
                            // Up to a sector boundary past `at`: the first
                            // sectors landed, the rest did not.
                            let end = at + bytes.len() as u64;
                            let first = (at / SECTOR + 1) * SECTOR;
                            let bounds: Vec<u64> = (0..)
                                .map(|k| first + k * SECTOR)
                                .take_while(|b| *b < end)
                                .collect();
                            if bounds.is_empty() {
                                lost += 1;
                                continue;
                            }
                            let cut = bounds[rng.random_range(0..bounds.len())];
                            let part = Op::Write {
                                at,
                                bytes: bytes[..(cut - at) as usize].to_vec(),
                            };
                            Inode::apply(&mut data, &part);
                            torn += 1;
                        }
                        trunc => {
                            Inode::apply(&mut data, &trunc);
                            kept += 1;
                        }
                    }
                }
            }
            inode.durable = data.clone();
            inode.data = data;
        }
        let names_lost = self.names != self.durable;
        self.names = self.durable.clone();
        self.forget_unreachable();
        self.stats.lost += lost;
        self.stats.torn += torn;
        let mut parts = Vec::new();
        if lost > 0 {
            parts.push(format!("{lost} unsynced write(s) lost"));
        }
        if torn > 0 {
            parts.push(format!("{torn} torn"));
        }
        if kept > 0 {
            parts.push(format!("{kept} landed"));
        }
        if names_lost {
            parts.push("unsynced names lost".to_string());
        }
        if parts.is_empty() {
            "nothing unsynced".to_string()
        } else {
            parts.join(", ")
        }
    }

    /// What would be left of `path` if the power went now, as bytes:
    /// its durable contents under its durable name (S4: properties read
    /// a node's disk this way). `None` when the name is not durable.
    pub fn durable_read(&self, path: &str) -> Option<&[u8]> {
        match self.durable.get(path)? {
            Entry::File(ino) => self.inodes.get(ino).map(|i| i.durable.as_slice()),
            Entry::Dir => None,
        }
    }

    /// The durable names directly in `dir`.
    pub fn durable_list(&self, dir: &str) -> Option<Vec<String>> {
        if !matches!(self.durable.get(dir), Some(Entry::Dir)) {
            return None;
        }
        Some(
            self.durable
                .keys()
                .filter(|p| child_of(dir, p))
                .map(|p| p.rsplit('/').next().unwrap_or("").to_string())
                .collect(),
        )
    }

    /// The disk as a power cut now would leave it, with no tearing: a
    /// copy holding only what is durable (for reading a database's
    /// durable state with SQLite, `props.rs`).
    pub fn durable_image(&self) -> Disk {
        let mut image = Disk {
            opts: DiskOpts::default(),
            inodes: BTreeMap::new(),
            next_ino: self.next_ino,
            names: self.durable.clone(),
            durable: self.durable.clone(),
            boot: 0,
            stats: DiskStats::default(),
        };
        for e in self.durable.values() {
            if let Entry::File(ino) = e
                && let Some(i) = self.inodes.get(ino)
            {
                image.inodes.insert(
                    *ino,
                    Inode {
                        data: i.durable.clone(),
                        durable: i.durable.clone(),
                        pending: Vec::new(),
                        modified: i.modified,
                    },
                );
            }
        }
        image
    }

    /// The process dies but the machine stays up: nothing written is
    /// lost; files the dead process held are no longer its.
    pub fn kill(&mut self) {
        self.boot += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    #[test]
    fn paths_normalize_against_the_working_directory() {
        assert_eq!(normalize("/", "a/b"), "/a/b");
        assert_eq!(normalize("/srv", "./data/../x.db"), "/srv/x.db");
        assert_eq!(normalize("/srv", "/etc//hosts/"), "/etc/hosts");
        assert_eq!(normalize("/", "../.."), "/");
        assert_eq!(parent("/a/b"), "/a");
        assert_eq!(parent("/a"), "/");
    }

    #[test]
    fn an_unsynced_write_is_lost_and_a_synced_one_survives() {
        let mut d = Disk::new(DiskOpts::default());
        d.write("/a", b"one", false, 0).unwrap();
        d.sync("/a", false).unwrap();
        d.write("/a", b"two", false, 0).unwrap();
        d.write("/b", b"new", false, 0).unwrap();
        assert_eq!(d.read("/a").unwrap(), b"two");
        assert!(d.dirty());
        d.crash(&mut rng());
        assert_eq!(d.read("/a").unwrap(), b"one");
        assert!(!d.exists("/b"), "a file never synced is gone");
        assert!(!d.dirty());
        // the replacement is a truncation and a write; the new file too
        assert_eq!(d.stats.lost, 4);
    }

    #[test]
    fn a_lying_sync_keeps_nothing() {
        let mut d = Disk::new(DiskOpts::default());
        d.write("/a", b"x", false, 0).unwrap();
        d.sync("/a", true).unwrap();
        d.crash(&mut rng());
        assert!(!d.exists("/a"));
        assert_eq!(d.stats.lies, 1);
    }

    #[test]
    fn appends_are_pending_until_synced() {
        let mut d = Disk::new(DiskOpts::default());
        d.write("/log", b"a\n", true, 0).unwrap();
        d.sync("/log", false).unwrap();
        d.write("/log", b"b\n", true, 0).unwrap();
        assert_eq!(d.read("/log").unwrap(), b"a\nb\n");
        d.crash(&mut rng());
        assert_eq!(d.read("/log").unwrap(), b"a\n");
    }

    #[test]
    fn a_kill_loses_nothing() {
        let mut d = Disk::new(DiskOpts::default());
        d.write("/a", b"x", false, 0).unwrap();
        d.kill();
        assert_eq!(d.read("/a").unwrap(), b"x");
        assert_eq!(d.boot, 1);
        // still pending: a later crash loses it
        d.crash(&mut rng());
        assert!(!d.exists("/a"));
    }

    #[test]
    fn a_rename_is_durable_once_its_directory_is_synced() {
        let mut d = Disk::new(DiskOpts::default());
        d.mkdir("/d").unwrap();
        d.write("/d/tmp", b"v2", false, 0).unwrap();
        d.sync("/d/tmp", false).unwrap();
        d.write("/d/f", b"v1", false, 0).unwrap();
        d.sync("/d/f", false).unwrap();
        d.rename("/d/tmp", "/d/f").unwrap();
        let mut copy = d.clone();
        copy.crash(&mut rng());
        assert_eq!(
            copy.read("/d/f").unwrap(),
            b"v1",
            "the rename was not durable"
        );
        assert!(copy.exists("/d/tmp"));
        d.sync("/d", false).unwrap();
        d.crash(&mut rng());
        assert_eq!(d.read("/d/f").unwrap(), b"v2");
        assert!(!d.exists("/d/tmp"));
    }

    #[test]
    fn torn_writes_keep_whole_sectors_of_what_was_pending() {
        let opts = DiskOpts {
            torn_writes: true,
            ..DiskOpts::default()
        };
        let mut seen_torn = false;
        let mut seen_whole = false;
        let mut seen_lost = false;
        for seed in 0..64 {
            let mut d = Disk::new(opts);
            d.write("/f", &[1u8; 2048], false, 0).unwrap();
            d.sync("/f", false).unwrap();
            let ino = match d.entry("/f") {
                Some(Entry::File(i)) => i,
                _ => unreachable!(),
            };
            d.write_at(ino, 0, &[2u8; 2048], 0).unwrap();
            d.crash(&mut StdRng::seed_from_u64(seed));
            let data = d.read("/f").unwrap();
            assert_eq!(data.len(), 2048);
            let twos = data.iter().take_while(|b| **b == 2).count();
            assert_eq!(twos % SECTOR as usize, 0, "cut at a sector");
            assert!(data[twos..].iter().all(|b| *b == 1));
            match twos {
                0 => seen_lost = true,
                2048 => seen_whole = true,
                _ => seen_torn = true,
            }
        }
        assert!(seen_torn && seen_whole && seen_lost);
    }

    #[test]
    fn a_full_disk_refuses_the_write_that_would_overflow_it() {
        let mut d = Disk::new(DiskOpts {
            full_at: Some(10),
            ..DiskOpts::default()
        });
        d.write("/a", b"12345678", false, 0).unwrap();
        assert_eq!(d.write("/a", b"9abc", true, 0), Err(NO_SPACE.to_string()));
        d.write("/a", b"short", false, 0).unwrap();
        assert_eq!(d.stats.full, 1);
    }

    #[test]
    fn directories_list_walk_and_refuse_what_posix_refuses() {
        let mut d = Disk::new(DiskOpts::default());
        assert_eq!(d.write("/no/such", b"x", false, 0), Err(NOT_FOUND.into()));
        d.mkdir_all("/a/b").unwrap();
        d.write("/a/b/x", b"1", false, 0).unwrap();
        d.write("/a/y", b"2", false, 0).unwrap();
        assert_eq!(d.list("/a").unwrap(), vec!["b", "y"]);
        assert_eq!(d.walk("/a").unwrap(), vec!["/a/b/x", "/a/y"]);
        assert_eq!(d.rmdir("/a"), Err(NOT_EMPTY.into()));
        assert_eq!(d.mkdir("/a"), Err(EXISTS.into()));
        d.rename("/a", "/c").unwrap();
        assert_eq!(d.read("/c/b/x").unwrap(), b"1");
        d.rmdir_all("/c").unwrap();
        assert!(!d.exists("/c/y"));
    }

    #[test]
    fn seeded_files_are_durable_from_the_start() {
        let mut d = Disk::new(DiskOpts::default());
        d.seed_file("/repo/lib/x.ol", b"let x = 1".to_vec());
        d.crash(&mut rng());
        assert_eq!(d.read("/repo/lib/x.ol").unwrap(), b"let x = 1");
        assert!(d.is_dir("/repo/lib"));
    }
}

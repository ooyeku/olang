//! SQLite inside a world (S3): a VFS, registered with SQLite as
//! `olang-sim`, whose files are a node's in-memory disk (`disk.rs`). A
//! connection a node opens (`db.open` in a world: `dbs.rs`) reads and
//! writes its database, its rollback journal, its WAL and its temporary
//! files here, and nowhere else: no call in this file reaches the real
//! file system.
//!
//! **Which disk.** A path handed to SQLite carries its node's disk as a
//! prefix: `/@sim<key>/data/x.db` is `/data/x.db` on the disk registered
//! under `key` (`register`). SQLite derives the journal (`-journal`) and
//! WAL (`-wal`) names from the database's, so they land on the same
//! disk; a relative name (an `ATTACH`) is resolved against the calling
//! node's working directory. A temporary file (no name) is the handle's
//! own memory.
//!
//! **Where the faults are.** `xSync` may lie (`fsync_lies`): it answers
//! `SQLITE_OK` and keeps nothing. `xWrite`, `xTruncate` and `xSync` are
//! crash points: when the node crashes at random (`#{ crash }` on the
//! node), the disk *freezes* at the call — it and every later write fail
//! — and the node crashes as the `db` call returns (`dbs.rs`), so the
//! disk is exactly as it was at that instant. A write past `full_at` is
//! `SQLITE_FULL`. Each sync costs one to three ms (times `slow`) of
//! virtual time, which the caller pays after the `db` call (`take_debt`):
//! SQLite cannot be paused in the middle of a call, so a sync is never a
//! yield point itself.
//!
//! **Memory.** Locks and the WAL index (`xShm*`) are process memory: per
//! node and per boot, gone with a crash. One connection per `db.open`,
//! never a shared cache; two connections of one node see each other's
//! locks as two processes' would.
//!
//! **The unsafe code** is the `extern "C"` callbacks below and nothing
//! else. Each one: (1) runs its body under `catch_unwind`, so a panic is
//! an `SQLITE_IOERR`, never an unwind across the FFI boundary; (2) reads
//! its `sqlite3_file` as a [`File`], whose `h` is a `Box<Handle>` made by
//! `x_open` and freed only by `x_close` (SQLite calls no method on a file
//! after closing it, and none on a file whose open failed: `pMethods` is
//! set last); (3) touches the buffers SQLite hands it within the lengths
//! it gives. Shared-memory regions are boxed slices that live as long as
//! the [`Shm`] that made them, which every handle that mapped one holds.

use super::disk::{self, Disk};
use rand::RngExt;
use rand::rngs::StdRng;
use rusqlite::ffi;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

/// The VFS's name, as `sqlite3_open_v2` is told it.
pub const VFS_NAME: &str = "olang-sim";

/// A node's disk and what goes with it: its seeded fault streams, the
/// process memory SQLite keeps beside it, and the events that happened
/// on it since the world last collected them.
pub struct NodeDisk {
    pub disk: Disk,
    /// Lies, sync latencies, torn writes.
    pub rng: StdRng,
    /// Crash points (`#{ crash: #{ p } }` on the node).
    pub crash_rng: StdRng,
    pub crash_p: f64,
    pub mem: Mem,
    /// What happened (kind, detail, the fault's visit), for the world's
    /// event stream.
    pub events: Vec<(&'static str, String, Option<super::plan::Key>)>,
    /// Set when the node is to crash at a disk point: every later write
    /// fails, and the `db` call's wrapper crashes the node. With the
    /// visit it fell on.
    pub crash_at: Option<(String, super::plan::Key)>,
    pub key: u64,
    /// S4: this disk's fault sites (crash points, lying syncs), keyed by
    /// its node's number and the world's clock.
    pub sites: super::plan::Sites,
    pub place: u64,
    pub clock: Arc<AtomicU64>,
    /// The node's crash plan's restart delay (what a crash point carries).
    pub restart_after: Option<u64>,
}

pub type Shared = Arc<Mutex<NodeDisk>>;

impl NodeDisk {
    pub fn new(disk: Disk, rng: StdRng, crash_rng: StdRng, crash_p: f64) -> NodeDisk {
        NodeDisk {
            disk,
            rng,
            crash_rng,
            crash_p,
            mem: Mem::default(),
            events: Vec::new(),
            crash_at: None,
            key: 0,
            sites: super::plan::Sites::drawn(),
            place: 0,
            clock: Arc::new(AtomicU64::new(0)),
            restart_after: None,
        }
    }

    fn now(&self) -> u64 {
        self.clock.load(Ordering::Relaxed)
    }

    /// Does this sync lie? The visit's key when it does.
    pub fn draw_lie(&mut self) -> Option<super::plan::Key> {
        let p = self.disk.opts.fsync_lies;
        if p <= 0.0 {
            return None;
        }
        let drawn = self.rng.random::<f64>() < p;
        let now = self.now();
        let d = self
            .sites
            .visit(super::plan::Site::FsyncLie, self.place, now, drawn, None);
        d.fire.then_some(d.key)
    }

    /// How long a sync takes, in µs of virtual time.
    pub fn sync_latency_us(&mut self) -> u64 {
        let base: u64 = self.rng.random_range(1_000..=3_000);
        (base as f64 * self.disk.opts.slow).round() as u64
    }

    /// Does the node crash here? The visit's key (carrying the restart
    /// delay) when it does.
    pub fn draw_crash(&mut self, restart_after: Option<u64>) -> Option<super::plan::Key> {
        if self.crash_p <= 0.0 {
            return None;
        }
        let drawn = self.crash_rng.random::<f64>() < self.crash_p;
        let now = self.now();
        let d = self.sites.visit(
            super::plan::Site::CrashPoint,
            self.place,
            now,
            drawn,
            restart_after,
        );
        d.fire.then_some(d.key)
    }

    /// One sync of inode `ino` (named `path`), with its faults drawn:
    /// whether it lied, and what it cost (µs).
    pub fn sync_file(&mut self, ino: u64, path: &str) -> (bool, u64) {
        let lie = self.draw_lie();
        let cost = self.sync_latency_us();
        self.disk.sync_ino(ino, Some(path), lie.is_some());
        self.events.push(if lie.is_some() {
            ("fsync_lie", path.to_string(), lie)
        } else {
            ("fsync", path.to_string(), None)
        });
        (lie.is_some(), cost)
    }

    pub fn sync_dir(&mut self, dir: &str) -> (bool, u64) {
        let lie = self.draw_lie();
        let cost = self.sync_latency_us();
        self.disk.sync_dir(dir, lie.is_some());
        self.events.push(if lie.is_some() {
            ("fsync_lie", format!("{dir} (directory)"), lie)
        } else {
            ("fsync", format!("{dir} (directory)"), None)
        });
        (lie.is_some(), cost)
    }

    /// The process is gone (crashed or killed): its locks and its WAL
    /// index go with it.
    pub fn forget_process(&mut self) {
        self.mem = Mem::default();
        self.crash_at = None;
    }
}

pub fn lock(nd: &Shared) -> MutexGuard<'_, NodeDisk> {
    nd.lock().unwrap_or_else(|e| e.into_inner())
}

// ─── which disk a name is on ─────────────────────────────────────────

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

fn keys() -> &'static Mutex<HashMap<u64, Weak<Mutex<NodeDisk>>>> {
    static KEYS: OnceLock<Mutex<HashMap<u64, Weak<Mutex<NodeDisk>>>>> = OnceLock::new();
    KEYS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Make `nd` reachable by name (its key goes into the paths SQLite is
/// given). Weak: the world owns its disks.
pub fn register(nd: &Shared) -> u64 {
    let key = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    lock(nd).key = key;
    keys()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, Arc::downgrade(nd));
    key
}

pub fn unregister(key: u64) {
    keys()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key);
}

fn find(key: u64) -> Option<Shared> {
    keys()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .and_then(Weak::upgrade)
}

/// The name SQLite is given for `path` on the disk under `key`.
pub fn sqlite_path(key: u64, path: &str) -> String {
    format!("/@sim{key}{path}")
}

/// `/@sim<key>/a/b` → (key, "/a/b").
fn split_key(name: &str) -> Option<(u64, String)> {
    let rest = name.strip_prefix("/@sim")?;
    let digits = rest.find('/').unwrap_or(rest.len());
    let key = rest[..digits].parse().ok()?;
    Some((key, disk::normalize("/", &rest[digits..])))
}

// ─── the virtual time a call owes ────────────────────────────────────

thread_local! {
    static DEBT_US: Cell<u64> = const { Cell::new(0) };
}

fn owe(us: u64) {
    DEBT_US.with(|d| d.set(d.get().saturating_add(us)));
}

/// What the calls since the last `take_debt` on this thread cost, in
/// whole virtual ms (rounded up), and the slate wiped.
pub fn take_debt_ms() -> u64 {
    let us = DEBT_US.with(|d| d.replace(0));
    us.div_ceil(1_000)
}

// ─── process memory: locks and the WAL index ─────────────────────────

/// A node's SQLite memory for one boot.
#[derive(Default)]
pub struct Mem {
    /// path → (handle → lock level), as `xLock` keeps them.
    locks: BTreeMap<String, BTreeMap<u64, c_int>>,
    /// path → the WAL index every handle on that database maps.
    shm: BTreeMap<String, Arc<Shm>>,
}

/// One database's shared memory: regions SQLite maps, and its eight
/// lock slots.
pub struct Shm {
    inner: Mutex<ShmInner>,
}

struct ShmInner {
    regions: Vec<(*mut u8, usize)>,
    shared: [BTreeSet<u64>; ffi::SQLITE_SHM_NLOCK as usize],
    exclusive: [Option<u64>; ffi::SQLITE_SHM_NLOCK as usize],
}

// SAFETY: the region pointers are owned by `ShmInner` (made from boxed
// slices in `x_shm_map`, freed only in its `Drop`) and every access to
// the vector is under the mutex; SQLite serializes its own use of the
// mapped memory with the shm locks, as it does across processes.
unsafe impl Send for ShmInner {}

impl Drop for ShmInner {
    fn drop(&mut self) {
        for (p, len) in self.regions.drain(..) {
            // SAFETY: made by `Box::into_raw` of a boxed slice of `len`
            // bytes in `x_shm_map`, and freed only here, once.
            unsafe {
                drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, len)));
            }
        }
    }
}

impl Shm {
    fn new() -> Shm {
        Shm {
            inner: Mutex::new(ShmInner {
                regions: Vec::new(),
                shared: Default::default(),
                exclusive: [None; ffi::SQLITE_SHM_NLOCK as usize],
            }),
        }
    }

    fn inner(&self) -> MutexGuard<'_, ShmInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn release(&self, id: u64) {
        let mut s = self.inner();
        for i in 0..s.shared.len() {
            s.shared[i].remove(&id);
            if s.exclusive[i] == Some(id) {
                s.exclusive[i] = None;
            }
        }
    }
}

// ─── the file ────────────────────────────────────────────────────────

/// SQLite's `sqlite3_file`, extended: `base` first, as SQLite requires.
#[repr(C)]
struct File {
    base: ffi::sqlite3_file,
    h: *mut Handle,
}

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

struct Handle {
    /// The node's disk; `None` for a temporary file (`temp` holds it).
    nd: Option<Shared>,
    temp: Vec<u8>,
    path: String,
    ino: u64,
    /// The disk's boot when this was opened: after a crash or a kill the
    /// file is no longer this process's, and every call on it fails.
    boot: u64,
    id: u64,
    delete_on_close: bool,
    shm: Option<Arc<Shm>>,
    /// A journal or WAL this handle created: its first sync syncs its
    /// directory too, so the new name is durable — what SQLite's unix VFS
    /// does for the pager, which counts on it.
    dirsync: bool,
}

/// Run a callback's body; a panic is `fallback`, never an unwind into C.
fn guard(fallback: c_int, f: impl FnOnce() -> c_int) -> c_int {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// The handle behind a file SQLite opened with `x_open`.
///
/// # Safety
/// `file` is a file `x_open` succeeded on and `x_close` has not closed.
unsafe fn handle<'a>(file: *mut ffi::sqlite3_file) -> &'a mut Handle {
    // SAFETY: per the contract, `file` is our `File` and `h` is live.
    unsafe { &mut *(*(file as *mut File)).h }
}

/// The node's disk, locked, when this handle still belongs to its boot.
fn live(h: &Handle) -> Option<MutexGuard<'_, NodeDisk>> {
    let g = lock(h.nd.as_ref()?);
    (g.disk.boot == h.boot).then_some(g)
}

fn now() -> i64 {
    crate::clock::epoch_ms()
}

unsafe extern "C" fn x_close(file: *mut ffi::sqlite3_file) -> c_int {
    guard(ffi::SQLITE_IOERR_CLOSE, || {
        // SAFETY: SQLite closes a file once; the box is ours (x_open).
        let h = unsafe { Box::from_raw((*(file as *mut File)).h) };
        // SAFETY: as above; nothing reads `h` through the file again.
        unsafe { (*(file as *mut File)).h = std::ptr::null_mut() };
        if let Some(shm) = &h.shm {
            shm.release(h.id);
        }
        if let Some(mut g) = live(&h) {
            if let Some(locks) = g.mem.locks.get_mut(&h.path) {
                locks.remove(&h.id);
            }
            if h.delete_on_close && g.crash_at.is_none() {
                let _ = g.disk.unlink(&h.path);
            }
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_read(
    file: *mut ffi::sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    off: ffi::sqlite3_int64,
) -> c_int {
    guard(ffi::SQLITE_IOERR_READ, || {
        // SAFETY: a live file (SQLite's contract); `buf` holds `amt` bytes.
        let h = unsafe { handle(file) };
        let out = unsafe { std::slice::from_raw_parts_mut(buf as *mut u8, amt.max(0) as usize) };
        let n = if h.nd.is_none() {
            let at = off as usize;
            let n = h.temp.len().saturating_sub(at).min(out.len());
            if n > 0 {
                out[..n].copy_from_slice(&h.temp[at..at + n]);
            }
            n
        } else {
            let Some(g) = live(h) else {
                return ffi::SQLITE_IOERR_READ;
            };
            g.disk.read_at(h.ino, off as u64, out)
        };
        if n < out.len() {
            out[n..].fill(0);
            return ffi::SQLITE_IOERR_SHORT_READ;
        }
        ffi::SQLITE_OK
    })
}

/// The checks every change makes first: the file is this boot's, the
/// disk is not frozen for a crash, and this is not where the node
/// crashes. `Err(code)` stops the call.
fn may_change(g: &mut NodeDisk, h: &Handle, what: &str, code: c_int) -> Result<(), c_int> {
    if g.disk.boot != h.boot || g.crash_at.is_some() {
        return Err(code);
    }
    let restart = g.restart_after;
    if let Some(key) = g.draw_crash(restart) {
        g.crash_at = Some((format!("{what} {}", h.path), key));
        return Err(code);
    }
    Ok(())
}

unsafe extern "C" fn x_write(
    file: *mut ffi::sqlite3_file,
    buf: *const c_void,
    amt: c_int,
    off: ffi::sqlite3_int64,
) -> c_int {
    guard(ffi::SQLITE_IOERR_WRITE, || {
        // SAFETY: a live file; `buf` holds `amt` bytes.
        let h = unsafe { handle(file) };
        let bytes = unsafe { std::slice::from_raw_parts(buf as *const u8, amt.max(0) as usize) };
        let off = off as u64;
        let Some(nd) = h.nd.clone() else {
            let end = off as usize + bytes.len();
            if h.temp.len() < end {
                h.temp.resize(end, 0);
            }
            h.temp[off as usize..end].copy_from_slice(bytes);
            return ffi::SQLITE_OK;
        };
        let mut g = lock(&nd);
        if let Err(code) = may_change(&mut g, h, "xWrite", ffi::SQLITE_IOERR_WRITE) {
            return code;
        }
        match g.disk.write_at(h.ino, off, bytes, now()) {
            Ok(()) => ffi::SQLITE_OK,
            Err(e) if e == disk::NO_SPACE => {
                g.events.push(("disk_full", h.path.clone(), None));
                ffi::SQLITE_FULL
            }
            Err(_) => ffi::SQLITE_IOERR_WRITE,
        }
    })
}

unsafe extern "C" fn x_truncate(file: *mut ffi::sqlite3_file, size: ffi::sqlite3_int64) -> c_int {
    guard(ffi::SQLITE_IOERR_TRUNCATE, || {
        // SAFETY: a live file.
        let h = unsafe { handle(file) };
        let Some(nd) = h.nd.clone() else {
            h.temp.resize(size.max(0) as usize, 0);
            return ffi::SQLITE_OK;
        };
        let mut g = lock(&nd);
        if let Err(code) = may_change(&mut g, h, "xTruncate", ffi::SQLITE_IOERR_TRUNCATE) {
            return code;
        }
        match g.disk.truncate(h.ino, size.max(0) as u64, now()) {
            Ok(()) => ffi::SQLITE_OK,
            Err(e) if e == disk::NO_SPACE => ffi::SQLITE_FULL,
            Err(_) => ffi::SQLITE_IOERR_TRUNCATE,
        }
    })
}

unsafe extern "C" fn x_sync(file: *mut ffi::sqlite3_file, _flags: c_int) -> c_int {
    guard(ffi::SQLITE_IOERR_FSYNC, || {
        // SAFETY: a live file.
        let h = unsafe { handle(file) };
        let Some(nd) = h.nd.clone() else {
            return ffi::SQLITE_OK;
        };
        let mut g = lock(&nd);
        if let Err(code) = may_change(&mut g, h, "xSync", ffi::SQLITE_IOERR_FSYNC) {
            return code;
        }
        let (_, cost) = g.sync_file(h.ino, &h.path);
        owe(cost);
        if std::mem::take(&mut h.dirsync) {
            let (_, cost) = g.sync_dir(disk::parent(&h.path));
            owe(cost);
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_file_size(
    file: *mut ffi::sqlite3_file,
    size: *mut ffi::sqlite3_int64,
) -> c_int {
    guard(ffi::SQLITE_IOERR_FSTAT, || {
        // SAFETY: a live file; `size` is SQLite's out-parameter.
        let h = unsafe { handle(file) };
        let n = if h.nd.is_none() {
            h.temp.len() as u64
        } else {
            match live(h) {
                Some(g) => g.disk.len_of(h.ino),
                None => return ffi::SQLITE_IOERR_FSTAT,
            }
        };
        unsafe { *size = n as ffi::sqlite3_int64 };
        ffi::SQLITE_OK
    })
}

/// The levels other handles hold on `h`'s file.
fn others(g: &NodeDisk, h: &Handle) -> Vec<c_int> {
    g.mem
        .locks
        .get(&h.path)
        .map(|m| {
            m.iter()
                .filter(|(id, _)| **id != h.id)
                .map(|(_, l)| *l)
                .collect()
        })
        .unwrap_or_default()
}

unsafe extern "C" fn x_lock(file: *mut ffi::sqlite3_file, level: c_int) -> c_int {
    guard(ffi::SQLITE_IOERR_LOCK, || {
        // SAFETY: a live file.
        let h = unsafe { handle(file) };
        if h.nd.is_none() {
            return ffi::SQLITE_OK;
        }
        let Some(mut g) = live(h) else {
            return ffi::SQLITE_IOERR_LOCK;
        };
        let mine = g
            .mem
            .locks
            .get(&h.path)
            .and_then(|m| m.get(&h.id))
            .copied()
            .unwrap_or(ffi::SQLITE_LOCK_NONE);
        if mine >= level {
            return ffi::SQLITE_OK;
        }
        let others = others(&g, h);
        let max_other = others
            .iter()
            .copied()
            .max()
            .unwrap_or(ffi::SQLITE_LOCK_NONE);
        let granted = match level {
            ffi::SQLITE_LOCK_SHARED => {
                if max_other >= ffi::SQLITE_LOCK_PENDING {
                    return ffi::SQLITE_BUSY;
                }
                ffi::SQLITE_LOCK_SHARED
            }
            ffi::SQLITE_LOCK_RESERVED => {
                if max_other >= ffi::SQLITE_LOCK_RESERVED {
                    return ffi::SQLITE_BUSY;
                }
                ffi::SQLITE_LOCK_RESERVED
            }
            _ => {
                // PENDING on the way to EXCLUSIVE: new readers are kept
                // out while the ones already in finish.
                if others.iter().any(|l| *l >= ffi::SQLITE_LOCK_RESERVED) {
                    return ffi::SQLITE_BUSY;
                }
                let readers = others.iter().any(|l| *l >= ffi::SQLITE_LOCK_SHARED);
                if readers || level == ffi::SQLITE_LOCK_PENDING {
                    g.mem
                        .locks
                        .entry(h.path.clone())
                        .or_default()
                        .insert(h.id, ffi::SQLITE_LOCK_PENDING);
                    return if readers {
                        ffi::SQLITE_BUSY
                    } else {
                        ffi::SQLITE_OK
                    };
                }
                ffi::SQLITE_LOCK_EXCLUSIVE
            }
        };
        g.mem
            .locks
            .entry(h.path.clone())
            .or_default()
            .insert(h.id, granted);
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_unlock(file: *mut ffi::sqlite3_file, level: c_int) -> c_int {
    guard(ffi::SQLITE_IOERR_UNLOCK, || {
        // SAFETY: a live file.
        let h = unsafe { handle(file) };
        if let Some(mut g) = live(h)
            && let Some(m) = g.mem.locks.get_mut(&h.path)
        {
            if level <= ffi::SQLITE_LOCK_NONE {
                m.remove(&h.id);
            } else if let Some(l) = m.get_mut(&h.id) {
                *l = (*l).min(level);
            }
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_check_reserved_lock(file: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    guard(ffi::SQLITE_IOERR_LOCK, || {
        // SAFETY: a live file; `out` is SQLite's out-parameter.
        let h = unsafe { handle(file) };
        let held = match live(h) {
            Some(g) => others(&g, h)
                .iter()
                .any(|l| *l >= ffi::SQLITE_LOCK_RESERVED),
            None => false,
        };
        unsafe { *out = held as c_int };
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_file_control(
    _file: *mut ffi::sqlite3_file,
    _op: c_int,
    _arg: *mut c_void,
) -> c_int {
    ffi::SQLITE_NOTFOUND
}

unsafe extern "C" fn x_sector_size(_file: *mut ffi::sqlite3_file) -> c_int {
    disk::SECTOR as c_int
}

/// No promises: no atomic writes, no safe appends, no powersafe
/// overwrite. SQLite then journals as it must for the worst disk.
unsafe extern "C" fn x_device_characteristics(_file: *mut ffi::sqlite3_file) -> c_int {
    0
}

unsafe extern "C" fn x_shm_map(
    file: *mut ffi::sqlite3_file,
    region: c_int,
    size: c_int,
    extend: c_int,
    out: *mut *mut c_void,
) -> c_int {
    guard(ffi::SQLITE_IOERR_SHMMAP, || {
        // SAFETY: a live file; `out` is SQLite's out-parameter.
        let h = unsafe { handle(file) };
        unsafe { *out = std::ptr::null_mut() };
        let shm = match &h.shm {
            Some(s) => s.clone(),
            None => {
                let Some(mut g) = live(h) else {
                    return ffi::SQLITE_IOERR_SHMMAP;
                };
                let s = g
                    .mem
                    .shm
                    .entry(h.path.clone())
                    .or_insert_with(|| Arc::new(Shm::new()))
                    .clone();
                drop(g);
                h.shm = Some(s.clone());
                s
            }
        };
        let mut s = shm.inner();
        let region = region.max(0) as usize;
        while s.regions.len() <= region {
            if extend == 0 {
                return ffi::SQLITE_OK;
            }
            let len = size.max(0) as usize;
            let bytes = vec![0u8; len].into_boxed_slice();
            s.regions.push((Box::into_raw(bytes) as *mut u8, len));
        }
        // SAFETY: the region lives as long as `shm`, which this handle
        // holds until x_shm_unmap or x_close.
        unsafe { *out = s.regions[region].0 as *mut c_void };
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_shm_lock(
    file: *mut ffi::sqlite3_file,
    offset: c_int,
    n: c_int,
    flags: c_int,
) -> c_int {
    guard(ffi::SQLITE_IOERR_SHMLOCK, || {
        // SAFETY: a live file.
        let h = unsafe { handle(file) };
        if live(h).is_none() {
            return ffi::SQLITE_IOERR_SHMLOCK;
        }
        let Some(shm) = &h.shm else {
            return ffi::SQLITE_IOERR_SHMLOCK;
        };
        let mut s = shm.inner();
        let slots = offset.max(0) as usize..(offset + n).max(0) as usize;
        if slots.end > s.shared.len() {
            return ffi::SQLITE_IOERR_SHMLOCK;
        }
        let me = h.id;
        if flags & ffi::SQLITE_SHM_UNLOCK != 0 {
            for i in slots {
                s.shared[i].remove(&me);
                if s.exclusive[i] == Some(me) {
                    s.exclusive[i] = None;
                }
            }
            return ffi::SQLITE_OK;
        }
        let exclusive = flags & ffi::SQLITE_SHM_EXCLUSIVE != 0;
        for i in slots.clone() {
            if s.exclusive[i].is_some_and(|o| o != me) {
                return ffi::SQLITE_BUSY;
            }
            if exclusive && s.shared[i].iter().any(|o| *o != me) {
                return ffi::SQLITE_BUSY;
            }
        }
        for i in slots {
            if exclusive {
                s.exclusive[i] = Some(me);
            } else {
                s.shared[i].insert(me);
            }
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_shm_barrier(_file: *mut ffi::sqlite3_file) {
    std::sync::atomic::fence(Ordering::SeqCst);
}

unsafe extern "C" fn x_shm_unmap(file: *mut ffi::sqlite3_file, delete: c_int) -> c_int {
    guard(ffi::SQLITE_IOERR_SHMMAP, || {
        // SAFETY: a live file.
        let h = unsafe { handle(file) };
        if let Some(shm) = h.shm.take() {
            shm.release(h.id);
        }
        if delete != 0
            && let Some(mut g) = live(h)
        {
            g.mem.shm.remove(&h.path);
        }
        ffi::SQLITE_OK
    })
}

static METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
    iVersion: 2,
    xClose: Some(x_close),
    xRead: Some(x_read),
    xWrite: Some(x_write),
    xTruncate: Some(x_truncate),
    xSync: Some(x_sync),
    xFileSize: Some(x_file_size),
    xLock: Some(x_lock),
    xUnlock: Some(x_unlock),
    xCheckReservedLock: Some(x_check_reserved_lock),
    xFileControl: Some(x_file_control),
    xSectorSize: Some(x_sector_size),
    xDeviceCharacteristics: Some(x_device_characteristics),
    xShmMap: Some(x_shm_map),
    xShmLock: Some(x_shm_lock),
    xShmBarrier: Some(x_shm_barrier),
    xShmUnmap: Some(x_shm_unmap),
    xFetch: None,
    xUnfetch: None,
};

// ─── the VFS ─────────────────────────────────────────────────────────

/// A name SQLite gives us, as text.
///
/// # Safety
/// `name` is null or a NUL-terminated string SQLite owns for the call.
unsafe fn text<'a>(name: *const c_char) -> Option<&'a str> {
    if name.is_null() {
        return None;
    }
    // SAFETY: per the contract.
    unsafe { CStr::from_ptr(name) }.to_str().ok()
}

unsafe extern "C" fn x_open(
    _vfs: *mut ffi::sqlite3_vfs,
    name: ffi::sqlite3_filename,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    guard(ffi::SQLITE_CANTOPEN, || {
        let f = file as *mut File;
        // SAFETY: SQLite hands us `szOsFile` bytes for our `File`.
        unsafe {
            (*f).base.pMethods = std::ptr::null();
            (*f).h = std::ptr::null_mut();
        }
        let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        let delete_on_close = flags & ffi::SQLITE_OPEN_DELETEONCLOSE != 0;
        // SAFETY: SQLite's name for the call (or null: a temporary file).
        let handle = match unsafe { text(name) } {
            None => Handle {
                nd: None,
                temp: Vec::new(),
                path: String::new(),
                ino: 0,
                boot: 0,
                id,
                delete_on_close: true,
                shm: None,
                dirsync: false,
            },
            Some(name) => {
                let Some((key, path)) = split_key(name) else {
                    return ffi::SQLITE_CANTOPEN;
                };
                let Some(nd) = find(key) else {
                    return ffi::SQLITE_CANTOPEN;
                };
                let mut g = lock(&nd);
                if g.crash_at.is_some() {
                    return ffi::SQLITE_CANTOPEN;
                }
                let create = flags & ffi::SQLITE_OPEN_CREATE != 0;
                let exclusive = flags & ffi::SQLITE_OPEN_EXCLUSIVE != 0;
                let journal = flags
                    & (ffi::SQLITE_OPEN_MAIN_JOURNAL
                        | ffi::SQLITE_OPEN_SUPER_JOURNAL
                        | ffi::SQLITE_OPEN_WAL)
                    != 0;
                let dirsync = create && journal;
                let ino = match g.disk.open(&path, create, exclusive) {
                    Ok(ino) => ino,
                    Err(_) => return ffi::SQLITE_CANTOPEN,
                };
                let boot = g.disk.boot;
                drop(g);
                Handle {
                    nd: Some(nd),
                    temp: Vec::new(),
                    path,
                    ino,
                    boot,
                    id,
                    delete_on_close,
                    shm: None,
                    dirsync,
                }
            }
        };
        // SAFETY: as above; `pMethods` last, so a failed open has none.
        unsafe {
            (*f).h = Box::into_raw(Box::new(handle));
            (*f).base.pMethods = &METHODS;
            if !out_flags.is_null() {
                *out_flags = flags;
            }
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_delete(
    _vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    sync_dir: c_int,
) -> c_int {
    guard(ffi::SQLITE_IOERR_DELETE, || {
        // SAFETY: SQLite's name for the call.
        let Some((key, path)) = unsafe { text(name) }.and_then(split_key) else {
            return ffi::SQLITE_IOERR_DELETE;
        };
        let Some(nd) = find(key) else {
            return ffi::SQLITE_IOERR_DELETE;
        };
        let mut g = lock(&nd);
        if g.crash_at.is_some() {
            return ffi::SQLITE_IOERR_DELETE;
        }
        let restart = g.restart_after;
        if let Some(key) = g.draw_crash(restart) {
            g.crash_at = Some((format!("xDelete {path}"), key));
            return ffi::SQLITE_IOERR_DELETE;
        }
        if g.disk.unlink(&path).is_err() {
            return ffi::SQLITE_IOERR_DELETE_NOENT;
        }
        if sync_dir != 0 {
            let (_, cost) = g.sync_dir(disk::parent(&path));
            owe(cost);
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_access(
    _vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    _flags: c_int,
    out: *mut c_int,
) -> c_int {
    guard(ffi::SQLITE_IOERR_ACCESS, || {
        // SAFETY: SQLite's name and out-parameter for the call.
        let found = unsafe { text(name) }
            .and_then(split_key)
            .and_then(|(key, path)| Some(lock(&find(key)?).disk.exists(&path)))
            .unwrap_or(false);
        unsafe { *out = found as c_int };
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_full_pathname(
    _vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    n_out: c_int,
    out: *mut c_char,
) -> c_int {
    guard(ffi::SQLITE_CANTOPEN, || {
        // SAFETY: SQLite's name for the call.
        let Some(name) = (unsafe { text(name) }) else {
            return ffi::SQLITE_CANTOPEN;
        };
        let full = match split_key(name) {
            Some((key, path)) => sqlite_path(key, &path),
            // A name without a disk (an ATTACH): the calling node's, as
            // its working directory reads it.
            None => match super::node_disk_here() {
                Some((key, cwd)) => sqlite_path(key, &disk::normalize(&cwd, name)),
                None => return ffi::SQLITE_CANTOPEN,
            },
        };
        let bytes = full.as_bytes();
        if bytes.len() + 1 > n_out.max(0) as usize {
            return ffi::SQLITE_CANTOPEN;
        }
        // SAFETY: `out` holds `n_out` bytes; we write len + 1 of them.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, bytes.len());
            *out.add(bytes.len()) = 0;
        }
        ffi::SQLITE_OK
    })
}

unsafe extern "C" fn x_dl_open(_vfs: *mut ffi::sqlite3_vfs, _name: *const c_char) -> *mut c_void {
    std::ptr::null_mut()
}

unsafe extern "C" fn x_dl_error(_vfs: *mut ffi::sqlite3_vfs, n: c_int, out: *mut c_char) {
    let message = b"extensions are not loaded inside a world\0";
    if n > 0 && !out.is_null() {
        let len = message.len().min(n as usize);
        // SAFETY: `out` holds `n` bytes; the last one written is a NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(message.as_ptr(), out as *mut u8, len);
            *out.add(len - 1) = 0;
        }
    }
}

unsafe extern "C" fn x_dl_sym(
    _vfs: *mut ffi::sqlite3_vfs,
    _handle: *mut c_void,
    _symbol: *const c_char,
) -> Option<unsafe extern "C" fn(*mut ffi::sqlite3_vfs, *mut c_void, *const c_char)> {
    None
}

unsafe extern "C" fn x_dl_close(_vfs: *mut ffi::sqlite3_vfs, _handle: *mut c_void) {}

/// Randomness from the world's seed (the calling task's stream), so a
/// world's SQLite draws the same numbers every run.
unsafe extern "C" fn x_randomness(
    _vfs: *mut ffi::sqlite3_vfs,
    n: c_int,
    out: *mut c_char,
) -> c_int {
    guard(0, || {
        // SAFETY: `out` holds `n` bytes.
        let buf = unsafe { std::slice::from_raw_parts_mut(out as *mut u8, n.max(0) as usize) };
        match super::task_rng() {
            Some(rng) => {
                let mut rng = rng.lock().unwrap_or_else(|e| e.into_inner());
                rng.fill(buf);
            }
            None => buf.fill(0),
        }
        n
    })
}

/// SQLite's busy handler sleeps here: virtual time, owed by the caller.
unsafe extern "C" fn x_sleep(_vfs: *mut ffi::sqlite3_vfs, us: c_int) -> c_int {
    owe(us.max(0) as u64);
    us
}

/// Julian day 0 in Unix ms is -210866760000000.
const JULIAN_EPOCH_MS: i64 = 210_866_760_000_000;

unsafe extern "C" fn x_current_time(_vfs: *mut ffi::sqlite3_vfs, out: *mut f64) -> c_int {
    // SAFETY: SQLite's out-parameter.
    unsafe { *out = (now() + JULIAN_EPOCH_MS) as f64 / 86_400_000.0 };
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_current_time_int64(
    _vfs: *mut ffi::sqlite3_vfs,
    out: *mut ffi::sqlite3_int64,
) -> c_int {
    // SAFETY: SQLite's out-parameter.
    unsafe { *out = now() + JULIAN_EPOCH_MS };
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_get_last_error(
    _vfs: *mut ffi::sqlite3_vfs,
    _n: c_int,
    _out: *mut c_char,
) -> c_int {
    0
}

/// Register the VFS with SQLite (once per process): its name.
pub fn registered() -> Result<&'static str, String> {
    static DONE: OnceLock<Result<(), String>> = OnceLock::new();
    DONE.get_or_init(|| {
        let name = std::ffi::CString::new(VFS_NAME).expect("no NUL in the name");
        let vfs = Box::leak(Box::new(ffi::sqlite3_vfs {
            iVersion: 2,
            szOsFile: std::mem::size_of::<File>() as c_int,
            mxPathname: 1024,
            pNext: std::ptr::null_mut(),
            zName: name.into_raw(),
            pAppData: std::ptr::null_mut(),
            xOpen: Some(x_open),
            xDelete: Some(x_delete),
            xAccess: Some(x_access),
            xFullPathname: Some(x_full_pathname),
            xDlOpen: Some(x_dl_open),
            xDlError: Some(x_dl_error),
            xDlSym: Some(x_dl_sym),
            xDlClose: Some(x_dl_close),
            xRandomness: Some(x_randomness),
            xSleep: Some(x_sleep),
            xCurrentTime: Some(x_current_time),
            xGetLastError: Some(x_get_last_error),
            xCurrentTimeInt64: Some(x_current_time_int64),
            xSetSystemCall: None,
            xGetSystemCall: None,
            xNextSystemCall: None,
        }));
        // SAFETY: the struct is leaked (it lives as long as the process,
        // as SQLite requires of a registered VFS) and fully initialized.
        let rc = unsafe { ffi::sqlite3_vfs_register(vfs, 0) };
        if rc == ffi::SQLITE_OK {
            Ok(())
        } else {
            Err(format!("sim: SQLite refused the world's VFS (code {rc})"))
        }
    })
    .clone()
    .map(|_| VFS_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rusqlite::{Connection, OpenFlags};

    fn node_disk(opts: disk::DiskOpts) -> Shared {
        let nd = Arc::new(Mutex::new(NodeDisk::new(
            Disk::new(opts),
            StdRng::seed_from_u64(1),
            StdRng::seed_from_u64(2),
            0.0,
        )));
        register(&nd);
        nd
    }

    fn open(nd: &Shared, path: &str) -> Connection {
        let key = lock(nd).key;
        Connection::open_with_flags_and_vfs(
            sqlite_path(key, path),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            registered().unwrap(),
        )
        .expect("open")
    }

    fn count(c: &Connection) -> i64 {
        c.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap_or(-1)
    }

    fn crash(nd: &Shared, seed: u64) {
        let mut g = lock(nd);
        g.disk.crash(&mut StdRng::seed_from_u64(seed));
        g.forget_process();
    }

    #[test]
    fn a_database_lives_on_the_node_disk_and_nowhere_else() {
        let nd = node_disk(disk::DiskOpts::default());
        let c = open(&nd, "/x.db");
        c.execute_batch("CREATE TABLE t (v); INSERT INTO t VALUES (1), (2);")
            .unwrap();
        assert_eq!(count(&c), 2);
        assert!(lock(&nd).disk.exists("/x.db"));
        assert!(!std::path::Path::new("/x.db").exists());
        assert!(!std::path::Path::new("/@sim1").exists());
    }

    /// Rows committed before a crash, read back after it: (rows, ok).
    fn commit_then_crash(mode: &str, sync: &str, torn: bool, seed: u64) -> (i64, String) {
        let nd = node_disk(disk::DiskOpts {
            torn_writes: torn,
            ..Default::default()
        });
        {
            let c = open(&nd, "/x.db");
            c.pragma_update(None, "journal_mode", mode).unwrap();
            c.pragma_update(None, "synchronous", sync).unwrap();
            c.execute_batch("CREATE TABLE t (v TEXT)").unwrap();
            for i in 0..20 {
                c.execute("INSERT INTO t VALUES (?1)", [format!("{i:0>400}")])
                    .unwrap();
            }
            // a transaction in flight: written, never committed
            c.execute_batch("BEGIN; DELETE FROM t; INSERT INTO t VALUES ('x');")
                .unwrap();
            crash(&nd, seed);
            // the dead process's connection cannot commit, and closing
            // it writes nothing
            assert!(
                c.execute_batch("INSERT INTO t VALUES ('y'); COMMIT")
                    .is_err()
            );
        }
        let c = open(&nd, "/x.db");
        let ok: String = c
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        (count(&c), ok)
    }

    #[test]
    fn committed_rows_survive_a_crash_when_sqlite_syncs_what_it_must() {
        for torn in [false, true] {
            for seed in 0..6 {
                // WAL with FULL syncs the log at every commit; DELETE
                // needs EXTRA, which syncs the directory after the
                // journal is unlinked: that unlink is the commit.
                for (mode, sync) in [("WAL", "FULL"), ("DELETE", "EXTRA")] {
                    let (rows, ok) = commit_then_crash(mode, sync, torn, seed);
                    assert_eq!(
                        (rows, ok.as_str()),
                        (20, "ok"),
                        "{mode} {sync} torn {torn} seed {seed}"
                    );
                }
            }
        }
    }

    #[test]
    fn delete_mode_at_full_can_roll_back_the_last_commit_and_only_it() {
        // SQLite documents it: at synchronous=FULL in DELETE mode the
        // journal's unlink is not synced, so a power cut right after a
        // commit finds the journal hot and rolls that commit back. Never
        // a torn database, never more than the last transaction.
        for torn in [false, true] {
            let (rows, ok) = commit_then_crash("DELETE", "FULL", torn, 1);
            assert_eq!(ok, "ok");
            assert_eq!(rows, 19, "torn {torn}");
        }
    }

    #[test]
    fn wal_at_normal_loses_the_commits_since_the_last_checkpoint() {
        // synchronous=NORMAL in WAL mode syncs the log only at a
        // checkpoint: a power cut loses what was committed since.
        let (rows, ok) = commit_then_crash("WAL", "NORMAL", false, 1);
        assert_eq!(ok, "ok");
        assert!(rows < 20, "{rows}");
    }

    #[test]
    fn a_lying_fsync_loses_committed_rows() {
        let nd = node_disk(disk::DiskOpts {
            fsync_lies: 1.0,
            ..Default::default()
        });
        {
            let c = open(&nd, "/x.db");
            c.execute_batch("CREATE TABLE t (v); INSERT INTO t VALUES (1);")
                .unwrap();
            assert_eq!(count(&c), 1);
            crash(&nd, 1);
        }
        let c = open(&nd, "/x.db");
        assert_eq!(count(&c), -1, "the table never reached the disk");
        assert!(lock(&nd).disk.stats.lies > 0);
    }

    #[test]
    fn a_full_disk_is_sqlite_full() {
        let nd = node_disk(disk::DiskOpts {
            full_at: Some(64 * 1024),
            ..Default::default()
        });
        let c = open(&nd, "/x.db");
        c.execute_batch("CREATE TABLE t (v TEXT)").unwrap();
        let mut failed = None;
        for i in 0..1000 {
            if let Err(e) = c.execute("INSERT INTO t VALUES (?1)", [format!("{i:0>1000}")]) {
                failed = Some(e.to_string());
                break;
            }
        }
        let failed = failed.expect("the disk filled");
        assert!(failed.contains("full"), "{failed}");
    }

    #[test]
    fn two_connections_of_one_node_lock_each_other_out() {
        let nd = node_disk(disk::DiskOpts::default());
        let a = open(&nd, "/x.db");
        let b = open(&nd, "/x.db");
        a.execute_batch("CREATE TABLE t (v)").unwrap();
        b.busy_timeout(std::time::Duration::from_millis(0)).unwrap();
        a.execute_batch("BEGIN IMMEDIATE; INSERT INTO t VALUES (1);")
            .unwrap();
        assert!(b.execute("INSERT INTO t VALUES (2)", []).is_err());
        a.execute_batch("COMMIT").unwrap();
        b.execute("INSERT INTO t VALUES (2)", []).unwrap();
        assert_eq!(count(&a), 2);
    }
}

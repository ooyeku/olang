//! Compiled bytecode kept between runs.
//!
//! The parse cache (`crate::parse_cache`) keeps a module's tree between
//! launches; this keeps what a function compiled to. olang Studio's first
//! frame compiled ~550 functions to bytecode (~65 ms) at every launch,
//! from trees that had not changed since the last one.
//!
//! **What an entry is.** One compile of one function: its bytecode and
//! the bytecode of the lambdas it carries (compiled with it as one
//! group), each with its constants written as *where they came from*
//! rather than as values — a literal, or a name the compile looked up in
//! its scope (a module value, a function, a lambda's captures) and looks
//! up again when the entry is used. A function id is written as the
//! number it had in the run that compiled it, with what it named: a
//! member of the group, or a name in the registry.
//!
//! **Keying.** An entry is keyed by a hash of everything a compile reads
//! that is not its scope: the build of olang (version,
//! commit, checkout state, build date — the parse cache's stamp), the
//! function's declaration (name, parameters, body: the module's text
//! as parsed), its type checks, its file, and whether callees compile
//! at their first call.
//!
//! **Exactness.** What the compile read from its scope is recorded
//! beside the code: every question it asked of the registries, the
//! closure, the module and the capability table, with a hash of the
//! answer as far as the compile looked at it (whether a name is a
//! function and of which name; a Boolean's value, since a constant
//! condition folds; a known function's parameters, defaults included,
//! since a short call splices them; a struct's fields).
//! An entry is used only when every question, asked again of this run's
//! scope at the same point, gets the same answer — then the compile
//! would have made the same code, and the constants, looked up again,
//! are the values it would have baked. Anything else compiles afresh and
//! replaces the entry. A compile that met anything the entry cannot
//! express (a constant with no recorded origin, an id with no name)
//! is not kept.
//!
//! **Bounds.** A module's entries share one file (a pack), read whole
//! the first time one of its functions compiles; a key keeps up to four
//! entries (one function compiled under scopes that differ from run to
//! run). Packs are written on a writer thread off the program's, each
//! read again, merged and renamed over — a reader meets the old pack or
//! the new one, never half of either, and takes no lock; two processes
//! writing one pack lose at most what the other added (compiled again
//! next run). An entry no run has written or used for two weeks is
//! dropped when its pack is next written; a pack holds 4,096 entries at
//! most; the directory is held under a size (64 MB,
//! `OLANG_COMPILE_CACHE_MB`), the packs least recently written removed
//! first. A pack whose header, index or an entry's checksum does not
//! hold — truncated, garbled, another build's — is not trusted: what it
//! should have held compiles afresh and is written again.
//!
//! Stored under `~/.olang/state/compiled/` (`OLANG_HOME` moves it);
//! `OLANG_COMPILE_CACHE=0` turns it off. Native code is not kept: see
//! docs/ovm.md.

use crate::ast::{Expr, FieldTypeCheck, Parameter};
use crate::ovm::bytecode::Instruction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

const MAGIC: &[u8; 4] = b"olcc";
/// The entry layout; part of every key.
const FORMAT: u16 = 1;

/// A compile's key: the first 16 bytes of a SHA-256.
pub type Key = [u8; 16];

/// Whether entries are read and written: on, unless
/// `OLANG_COMPILE_CACHE=0` (read once a process) or there is no home.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        if cfg!(target_arch = "wasm32") {
            return false;
        }
        !matches!(std::env::var("OLANG_COMPILE_CACHE").as_deref(), Ok("0")) && dir().is_some()
    })
}

fn dir() -> Option<PathBuf> {
    crate::home::state().map(|s| s.join("compiled"))
}

fn cap_bytes() -> u64 {
    std::env::var("OLANG_COMPILE_CACHE_MB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(64)
        * 1024
        * 1024
}

/// The build an entry must have been written by (the parse cache's).
fn stamp() -> String {
    format!(
        "{} {} {} {}",
        crate::version::VERSION,
        crate::version::BUILD_COMMIT,
        crate::version::BUILD_DIRTY,
        crate::version::BUILD_DATE
    )
}

/// Builds a key: the build's stamp, the format, then whatever the caller
/// feeds (postcard encodings, written straight into the hash).
pub struct KeyHasher(Sha256);

impl KeyHasher {
    pub fn new() -> Self {
        let mut h = Sha256::new();
        h.update(MAGIC);
        h.update(FORMAT.to_le_bytes());
        h.update(stamp().as_bytes());
        KeyHasher(h)
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.0.update((b.len() as u64).to_le_bytes());
        self.0.update(b);
    }

    /// A value's postcard encoding; false when it cannot be encoded.
    pub fn value<T: Serialize + ?Sized>(&mut self, v: &T) -> bool {
        struct W<'a>(&'a mut Sha256);
        impl std::io::Write for W<'_> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.update(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        self.0.update([0xA5]);
        postcard::to_io(v, W(&mut self.0)).is_ok()
    }

    pub fn finish(self) -> Key {
        let full = self.0.finalize();
        let mut k = [0u8; 16];
        k.copy_from_slice(&full[..16]);
        k
    }
}

impl Default for KeyHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// A stable hash of an answer (SipHash with fixed keys: the same in
/// every run of one build, and the build is in every key).
pub fn answer<T: std::hash::Hash + ?Sized>(v: &T) -> u64 {
    use std::hash::Hasher;
    // `new` has fixed keys: the same hash in every process
    let mut h = std::hash::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------
// What an entry holds
// ---------------------------------------------------------------------

/// One question a compile asked of its scope, with its answer's hash.
/// `tag` names the question (`crate::ovm::bytecode` defines them); a
/// `SEGMENT` record starts a group member's own questions, a `LAZY` one
/// is an id the compile gave a callee (replayed, not asked).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rec {
    pub tag: u8,
    pub a: String,
    pub b: String,
    pub h: u64,
}

/// What a function id in the code named.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IdRef {
    /// A member of the group (0 is the function, then its lambdas).
    Member(u32),
    /// The id the registry holds for a name (looked up after the
    /// questions are answered, an owed callee's included).
    Registry(String),
}

/// A literal constant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Plain {
    Int(i64),
    Float(f64),
    Bool(bool),
    Unit,
    Str(String),
    List(Vec<Plain>),
    Tuple(Vec<Plain>),
    EmptyMap,
}

/// A lambda the compile built as a value: its parts, and the names its
/// closure took from the scope (looked up again).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LambdaSrc {
    pub self_name: Option<String>,
    pub params: Vec<Parameter>,
    pub body: Expr,
    pub captured: Vec<String>,
}

/// Where a constant came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConstSrc {
    Plain(Plain),
    /// A free identifier: closure, run, module, then a known function.
    Ident(String),
    /// A called name's value (`callee_in_scope`).
    Callee(String),
    /// The known function of a name.
    Known(String),
    /// What the function sees lexically under a name (a unit variant).
    Lexical(String),
    /// A lambda with no runtime captures.
    Lambda(LambdaSrc),
    /// A closure's template: its lambda, the runtime captures' names,
    /// and the id its body was compiled under.
    Closure {
        src: LambdaSrc,
        capture_names: Vec<String>,
        func_id: u64,
    },
}

/// One compiled function of a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub old_id: u64,
    pub instructions: Vec<Instruction>,
    pub register_count: u32,
    pub local_count: u32,
    pub param_count: usize,
    pub def_file: Option<String>,
    pub param_names: Vec<String>,
    pub param_checks: Vec<Option<FieldTypeCheck>>,
    pub return_check: Option<FieldTypeCheck>,
    pub constants: Vec<ConstSrc>,
    pub span_table: Vec<(u32, u32, u32)>,
    pub callee_names: Vec<(u32, String)>,
    pub function_name: Option<String>,
    pub optimization_level: u8,
    pub entry_point: usize,
}

/// A compile: its questions, its members, what its ids named.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub log: Vec<Rec>,
    pub members: Vec<Member>,
    pub ids: Vec<(u64, IdRef)>,
}

// ---------------------------------------------------------------------
// Serde helpers for the instruction fields that hold process state
// ---------------------------------------------------------------------

/// A struct shape as its type and fields, interned again when read.
pub mod shape_serde {
    use crate::ovm::value::StructShape;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    pub fn serialize<S: Serializer>(shape: &Arc<StructShape>, s: S) -> Result<S::Ok, S::Error> {
        (&shape.type_name, &shape.field_names).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<StructShape>, D::Error> {
        let (type_name, fields): (String, Vec<String>) = Deserialize::deserialize(d)?;
        Ok(crate::ovm::value::intern_shape(&type_name, fields))
    }
}

/// An immediate operand: a literal number (anything else fails to
/// encode, and the compile is not kept).
pub mod imm_serde {
    use super::Plain;
    use crate::ovm::OvmValue;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &OvmValue, s: S) -> Result<S::Ok, S::Error> {
        match super::plain_of(v) {
            Some(p) => p.serialize(s),
            None => Err(serde::ser::Error::custom("an immediate that is not a literal")),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OvmValue, D::Error> {
        let p: Plain = Deserialize::deserialize(d)?;
        Ok(super::value_of(&p))
    }
}

/// A literal constant's form, or `None` for anything else.
pub fn plain_of(v: &crate::ovm::OvmValue) -> Option<Plain> {
    use crate::ovm::value::ValueData as D;
    Some(match &v.data {
        D::Integer(i) => Plain::Int(*i),
        D::Float(f) => Plain::Float(*f),
        D::Boolean(b) => Plain::Bool(*b),
        D::Unit => Plain::Unit,
        D::String(s) => Plain::Str((**s).clone()),
        D::List(items) => Plain::List(items.iter().map(plain_of).collect::<Option<_>>()?),
        D::IntList(items) => Plain::List(items.iter().map(|i| Plain::Int(*i)).collect()),
        D::FloatList(items) => Plain::List(items.iter().map(|f| Plain::Float(*f)).collect()),
        D::Tuple(items) => Plain::Tuple(items.iter().map(plain_of).collect::<Option<_>>()?),
        D::Map(m) if m.is_empty() => Plain::EmptyMap,
        _ => return None,
    })
}

/// The value a literal constant stands for (lists boxed, as constants
/// are).
pub fn value_of(p: &Plain) -> crate::ovm::OvmValue {
    use crate::ovm::OvmValue;
    match p {
        Plain::Int(i) => OvmValue::new_integer(*i),
        Plain::Float(f) => OvmValue::new_float(*f),
        Plain::Bool(b) => OvmValue::new_boolean(*b),
        Plain::Unit => OvmValue::new_unit(),
        Plain::Str(s) => OvmValue::new_string(s.clone()),
        Plain::List(items) => OvmValue::new_list(items.iter().map(value_of).collect()),
        Plain::Tuple(items) => OvmValue::new_tuple(items.iter().map(value_of).collect()),
        Plain::EmptyMap => OvmValue::new_map(Arc::new(crate::ovm::value::OvmMap::default())),
    }
}

// ---------------------------------------------------------------------
// Packs: one file a module
// ---------------------------------------------------------------------
//
// A module's entries share one file (a pack), named by the build and
// the module's path, read whole the first time one of its functions
// compiles: a launch reads a file a module, not one a function (Studio:
// 122 reads for ~600 compiles; a read of a small file costs ~30 µs on
// APFS). Inside, an index (key, when last written or used, offset,
// length, checksum) and the entries, each decoded when first asked for.
//
// Layout: magic `olcp`, format, stamp (u16 length, bytes), count (u32),
// then `count` index rows of 16 + 8 + 8 + 8 + 8 bytes, then the bodies.

const PACK_MAGIC: &[u8; 4] = b"olcp";
const ROW: usize = 16 + 8 + 8 + 8 + 8;
/// A pack holds at most this many entries; past it, the least recently
/// written or used go first.
const PACK_MAX: usize = 4096;
/// A key holds at most this many entries, newest first: one function
/// compiled under scopes that differ from run to run (which callees the
/// registry had met yet, which name was ambiguous yet) keeps a code for
/// each, and a run uses the first whose questions it answers alike.
const VARIANTS: usize = 4;
/// An entry no run has written or used for this long is dropped when its
/// pack is next written.
const STALE_SECS: u64 = 14 * 86_400;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The pack of the module `def_file` (`None`: code with no file).
fn pack_path(def_file: Option<&str>) -> Option<PathBuf> {
    let mut h = Sha256::new();
    h.update(stamp().as_bytes());
    h.update([0]);
    h.update(def_file.unwrap_or("\u{0}").as_bytes());
    let full = h.finalize();
    let name: String = full[..12].iter().map(|b| format!("{:02x}", b)).collect();
    dir().map(|d| d.join(format!("{}.olcp", name)))
}

struct Row {
    time: u64,
    off: usize,
    len: usize,
    sum: [u8; 8],
}

/// A pack as read: its bytes, its index, what has been decoded.
struct Pack {
    bytes: Vec<u8>,
    body: usize,
    /// Each key's entries, newest first.
    rows: HashMap<Key, Vec<Row>>,
}

impl Pack {
    fn empty() -> Pack {
        Pack {
            bytes: Vec::new(),
            body: 0,
            rows: HashMap::new(),
        }
    }

    /// A pack's header and index, if they are whole and of this build;
    /// a body is checked when it is read.
    fn parse(bytes: Vec<u8>) -> Option<Pack> {
        let stamp = stamp();
        let mut at = 0usize;
        let take = |at: &mut usize, n: usize| -> Option<std::ops::Range<usize>> {
            let end = at.checked_add(n)?;
            if end > bytes.len() {
                return None;
            }
            let r = *at..end;
            *at = end;
            Some(r)
        };
        let u64_at = |r: std::ops::Range<usize>| u64::from_le_bytes(bytes[r].try_into().unwrap_or([0; 8]));
        if bytes[take(&mut at, 4)?] != PACK_MAGIC[..] || bytes[take(&mut at, 2)?] != FORMAT.to_le_bytes() {
            return None;
        }
        let n = u16::from_le_bytes(bytes[take(&mut at, 2)?].try_into().ok()?) as usize;
        if bytes[take(&mut at, n)?] != *stamp.as_bytes() {
            return None;
        }
        let count = u32::from_le_bytes(bytes[take(&mut at, 4)?].try_into().ok()?) as usize;
        let index = take(&mut at, count.checked_mul(ROW)?)?;
        let body = at;
        let span = bytes.len() - body;
        let mut rows: HashMap<Key, Vec<Row>> = HashMap::with_capacity(count);
        for i in 0..count {
            let r = index.start + i * ROW;
            let key: Key = bytes[r..r + 16].try_into().ok()?;
            let time = u64_at(r + 16..r + 24);
            let off = u64_at(r + 24..r + 32) as usize;
            let len = u64_at(r + 32..r + 40) as usize;
            let sum: [u8; 8] = bytes[r + 40..r + 48].try_into().ok()?;
            if off.checked_add(len)? > span {
                return None;
            }
            rows.entry(key).or_default().push(Row { time, off, len, sum });
        }
        Some(Pack { bytes, body, rows })
    }

    /// The body of `key`'s `i`th entry (newest first), if it is whole.
    fn body_at(&self, key: &Key, i: usize) -> Option<&[u8]> {
        let row = self.rows.get(key)?.get(i)?;
        let body = &self.bytes[self.body + row.off..self.body + row.off + row.len];
        (Sha256::digest(body)[..8] == row.sum).then_some(body)
    }

    /// The bodies of `key`'s entries that are whole, newest first, with
    /// when each was last written or used.
    fn bodies_of(&self, key: &Key) -> Vec<(u64, &[u8])> {
        let Some(rows) = self.rows.get(key) else {
            return Vec::new();
        };
        rows.iter()
            .filter_map(|row| {
                let body = &self.bytes[self.body + row.off..self.body + row.off + row.len];
                (Sha256::digest(body)[..8] == row.sum).then_some((row.time, body))
            })
            .collect()
    }

    fn read(path: &std::path::Path) -> Option<Pack> {
        Pack::parse(std::fs::read(path).ok()?)
    }
}

/// Write a pack: `rows` (key, time, body), newest kept when there are
/// too many.
fn encode_pack(mut rows: Vec<(Key, u64, Vec<u8>)>) -> Vec<u8> {
    if rows.len() > PACK_MAX {
        // stable: a key's entries keep their order
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        rows.truncate(PACK_MAX);
    }
    let stamp = stamp();
    let total: usize = rows.iter().map(|r| r.2.len()).sum();
    let mut out = Vec::with_capacity(4 + 2 + 2 + stamp.len() + 4 + rows.len() * ROW + total);
    out.extend_from_slice(PACK_MAGIC);
    out.extend_from_slice(&FORMAT.to_le_bytes());
    out.extend_from_slice(&(stamp.len() as u16).to_le_bytes());
    out.extend_from_slice(stamp.as_bytes());
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    let mut off = 0u64;
    for (key, time, body) in &rows {
        out.extend_from_slice(key);
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend_from_slice(&Sha256::digest(body)[..8]);
        off += body.len() as u64;
    }
    for (_, _, body) in &rows {
        out.extend_from_slice(body);
    }
    out
}

/// An entry's body: its postcard encoding.
fn encode(entry: &Entry) -> Option<Vec<u8>> {
    postcard::to_stdvec(entry).ok()
}

fn decode(body: &[u8]) -> Option<Entry> {
    postcard::from_bytes(body).ok()
}

/// What this process has read of each pack, the entries it stored, by
/// key (newest first), and the newest entry of each key of a pack read
/// ahead (`prefetch`), decoded and waiting to be taken.
#[derive(Default)]
struct Memo {
    packs: HashMap<PathBuf, Arc<Pack>>,
    entries: HashMap<Key, Vec<Arc<Entry>>>,
    ahead: HashMap<Key, Entry>,
    asked: HashSet<PathBuf>,
}

fn memo() -> &'static Mutex<Memo> {
    static M: OnceLock<Mutex<Memo>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(Memo::default()))
}

/// The entries kept for `key` in the pack of `def_file`, newest first
/// (not yet checked against the scope): those this process stored, then
/// the pack's, each read and decoded as it is asked for.
pub fn load(def_file: Option<&str>, key: &Key) -> impl Iterator<Item = Arc<Entry>> {
    let (stored, ahead, pack, path) = pack_of(def_file, key).unwrap_or((Vec::new(), None, None, None));
    let rows = pack.as_ref().and_then(|p| p.rows.get(key)).map_or(0, Vec::len);
    // the newest, when read ahead, is decoded already
    let first = if ahead.is_some() { 1 } else { 0 };
    // Used: an entry not written or used for a day is marked used, so
    // the pack keeps it when it is next written.
    if let (Some(path), Some(p)) = (path, pack.as_ref())
        && p.rows
            .get(key)
            .is_some_and(|rows| rows.iter().any(|r| now_secs().saturating_sub(r.time) > 86_400))
    {
        send(Job::Used(path, *key));
    }
    let key = *key;
    // decoded at each use, not kept: the caller takes its parts
    stored
        .into_iter()
        .chain(ahead.map(Arc::new))
        .chain((first..rows).filter_map(move |i| {
            let p = pack.as_ref()?;
            decode(p.body_at(&key, i)?).map(Arc::new)
        }))
}

#[allow(clippy::type_complexity)]
fn pack_of(
    def_file: Option<&str>,
    key: &Key,
) -> Option<(Vec<Arc<Entry>>, Option<Entry>, Option<Arc<Pack>>, Option<PathBuf>)> {
    let path = pack_path(def_file)?;
    {
        let mut m = memo().lock().ok()?;
        let stored = m.entries.get(key).cloned().unwrap_or_default();
        if let Some(p) = m.packs.get(&path).cloned() {
            let ahead = m.ahead.remove(key);
            return Some((stored, ahead, Some(p), Some(path)));
        }
    }
    // read outside the lock (the reader thread may be reading another)
    let read = Arc::new(Pack::read(&path).unwrap_or_else(Pack::empty));
    let mut m = memo().lock().ok()?;
    let stored = m.entries.get(key).cloned().unwrap_or_default();
    let pack = m.packs.entry(path.clone()).or_insert(read).clone();
    let ahead = m.ahead.remove(key);
    Some((stored, ahead, Some(pack), Some(path)))
}

/// Read the pack of `def_file` ahead, on a thread of its own: a module is
/// loaded (and evaluated) well before its functions first compile, and
/// its pack, read and decoded meanwhile, is then waiting in memory.
pub fn prefetch(def_file: Option<&str>) {
    if !enabled() {
        return;
    }
    let Some(path) = pack_path(def_file) else {
        return;
    };
    {
        let Ok(mut m) = memo().lock() else {
            return;
        };
        if m.packs.contains_key(&path) || !m.asked.insert(path.clone()) {
            return;
        }
    }
    static TX: OnceLock<Option<Mutex<std::sync::mpsc::Sender<PathBuf>>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();
        std::thread::Builder::new()
            .name("olang-compile-cache-read".to_string())
            .spawn(move || {
                while let Ok(path) = rx.recv() {
                    read_ahead(path);
                }
            })
            .ok()?;
        Some(Mutex::new(tx))
    });
    if let Some(tx) = tx
        && let Ok(tx) = tx.lock()
    {
        let _ = tx.send(path);
    }
}

fn read_ahead(path: PathBuf) {
    let Some(pack) = Pack::read(&path) else {
        return;
    };
    let pack = Arc::new(pack);
    {
        let Ok(mut m) = memo().lock() else {
            return;
        };
        if m.packs.contains_key(&path) {
            // the program got there first
            return;
        }
        m.packs.insert(path, pack.clone());
    }
    // each key's newest entry, decoded; taken (not copied) when used
    let mut batch = Vec::new();
    for key in pack.rows.keys() {
        if let Some(entry) = pack.body_at(key, 0).and_then(decode) {
            batch.push((*key, entry));
        }
        if batch.len() >= 32 {
            flush_ahead(&mut batch);
        }
    }
    flush_ahead(&mut batch);
}

fn flush_ahead(batch: &mut Vec<(Key, Entry)>) {
    if let Ok(mut m) = memo().lock() {
        for (key, entry) in batch.drain(..) {
            m.ahead.entry(key).or_insert(entry);
        }
    }
}

/// Keep the entry `make` makes under `key` in the pack of `def_file`:
/// made, encoded and written on the writer thread (a compile pays for
/// none of it), and found by this process's later compiles once made. A
/// key is written as many times a process as it holds entries, at most
/// (a compile whose scope keeps changing is not rewritten on every
/// call).
pub fn store(
    def_file: Option<&str>,
    key: Key,
    make: impl FnOnce() -> Option<Entry> + Send + 'static,
) {
    static WRITTEN: OnceLock<Mutex<(HashMap<Key, usize>, usize)>> = OnceLock::new();
    let written = WRITTEN.get_or_init(|| Mutex::new((HashMap::new(), 0)));
    let Ok(mut w) = written.lock() else {
        return;
    };
    // a bound on what one process writes, and on each key
    let (counts, total) = &mut *w;
    let n = counts.entry(key).or_insert(0);
    if *total >= 20_000 || *n >= VARIANTS {
        return;
    }
    *n += 1;
    *total += 1;
    drop(w);
    let Some(path) = pack_path(def_file) else {
        return;
    };
    send(Job::Write(path, key, Box::new(make)));
}

/// An entry made on the writer thread: kept in memory for this process,
/// and its encoding for the pack.
fn made(key: Key, make: Maker) -> Option<Vec<u8>> {
    let entry = make()?;
    let body = encode(&entry)?;
    if let Ok(mut m) = memo().lock() {
        m.entries.entry(key).or_default().insert(0, Arc::new(entry));
    }
    crate::boot_trace::add(crate::boot_trace::Counter::CompileCacheStored, 0);
    Some(body)
}

type Maker = Box<dyn FnOnce() -> Option<Entry> + Send>;

/// Forget what this process read and wrote (tests: a "next run" in the
/// same process).
pub fn forget() {
    if let Ok(mut m) = memo().lock() {
        *m = Memo::default();
    }
}

enum Job {
    Write(PathBuf, Key, Maker),
    Used(PathBuf, Key),
    Flush(std::sync::mpsc::Sender<()>),
}

static SENDER_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn sender() -> Option<&'static Mutex<std::sync::mpsc::Sender<Job>>> {
    static TX: OnceLock<Option<Mutex<std::sync::mpsc::Sender<Job>>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("olang-compile-cache".to_string())
            .spawn(move || writer(rx))
            .ok()?;
        SENDER_STARTED.store(true, std::sync::atomic::Ordering::Release);
        Some(Mutex::new(tx))
    })
    .as_ref()
}

fn send(job: Job) {
    if let Some(tx) = sender()
        && let Ok(tx) = tx.lock()
    {
        let _ = tx.send(job);
    }
}

/// Changes waiting to be written to one pack.
#[derive(Default)]
struct Pending {
    new: Vec<(Key, Vec<u8>)>,
    used: HashSet<Key>,
}

/// The writer: changes are gathered while they keep coming (a launch
/// compiles in a burst) and each pack touched is written once — at a
/// quiet moment, after half a second of changes at most, or when the
/// process asks (`flush`).
fn writer(rx: std::sync::mpsc::Receiver<Job>) {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::{Duration, Instant};
    let mut pending: HashMap<PathBuf, Pending> = HashMap::new();
    let mut since: Option<Instant> = None;
    let mut evicted = false;
    loop {
        let job = if pending.is_empty() {
            match rx.recv() {
                Ok(j) => Some(j),
                Err(_) => return,
            }
        } else {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(j) => Some(j),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    write_packs(&mut pending);
                    return;
                }
            }
        };
        // a timeout: nothing came for 100 ms
        let quiet = job.is_none();
        let mut flush = None;
        match job {
            Some(Job::Write(path, key, make)) => {
                if let Some(body) = made(key, make) {
                    pending.entry(path).or_default().new.push((key, body));
                    since.get_or_insert_with(Instant::now);
                }
            }
            Some(Job::Used(path, key)) => {
                pending.entry(path).or_default().used.insert(key);
                since.get_or_insert_with(Instant::now);
            }
            Some(Job::Flush(done)) => flush = Some(done),
            None => {}
        }
        let due = flush.is_some()
            || quiet
            || since.is_some_and(|t| t.elapsed() >= Duration::from_millis(500));
        if due && !pending.is_empty() {
            write_packs(&mut pending);
            since = None;
            if !evicted {
                evicted = true;
                evict(cap_bytes());
            }
        }
        if let Some(done) = flush {
            let _ = done.send(());
        }
    }
}


/// Write each pack with its changes: read it again first (another
/// process may have written it since), add what is new, refresh what was
/// used, drop what no run has wanted for two weeks, write beside and
/// rename over — a reader meets the old pack or the new one, never half.
/// Two processes writing one pack: the last rename wins, whole; what the
/// other added is compiled and kept again by its next run.
fn write_packs(pending: &mut HashMap<PathBuf, Pending>) {
    let Some(dir) = dir() else {
        pending.clear();
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        pending.clear();
        return;
    }
    let now = now_secs();
    for (path, changes) in pending.drain() {
        let old = Pack::read(&path).unwrap_or_else(Pack::empty);
        // each key's entries, newest first: what this process made, then
        // what the pack held (a copy of a new one dropped)
        let mut rows: HashMap<Key, Vec<(u64, Vec<u8>)>> = HashMap::new();
        for (key, body) in changes.new.into_iter().rev() {
            let v = rows.entry(key).or_default();
            if !v.iter().any(|(_, b)| *b == body) {
                v.push((now, body));
            }
        }
        for key in old.rows.keys() {
            let used = changes.used.contains(key);
            for (time, body) in old.bodies_of(key) {
                if now.saturating_sub(time) > STALE_SECS && !used {
                    continue;
                }
                let v = rows.entry(*key).or_default();
                if !v.iter().any(|(_, b)| b == body) {
                    v.push((if used { now } else { time }, body.to_vec()));
                }
            }
        }
        let mut flat = Vec::new();
        for (key, mut v) in rows {
            v.truncate(VARIANTS);
            flat.extend(v.into_iter().map(|(t, b)| (key, t, b)));
        }
        let bytes = encode_pack(flat);
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("tmp.{}.{}", std::process::id(), n));
        if std::fs::write(&tmp, &bytes).is_ok() {
            if std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// Hold the directory under `cap` bytes: the packs least recently
/// written go first, down to three quarters of it. A temporary file
/// older than a minute (its writer gone) goes too.
pub fn evict(cap: u64) {
    let Some(dir) = dir() else {
        return;
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let mut total = 0u64;
    for e in read.flatten() {
        let path = e.path();
        let Ok(meta) = e.metadata() else {
            continue;
        };
        let modified = meta.modified().unwrap_or(now);
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.contains(".tmp.") {
            if now.duration_since(modified).is_ok_and(|d| d.as_secs() > 60) {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        if !name.ends_with(".olcp") {
            continue;
        }
        total += meta.len();
        files.push((modified, meta.len(), path));
    }
    if total <= cap {
        return;
    }
    files.sort_by_key(|f| f.0);
    let goal = cap / 4 * 3;
    for (_, len, path) in files {
        if total <= goal {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Wait (a moment at most) for what the writer has been handed: a run
/// that ends keeps what it compiled. Called as the process exits.
pub fn flush() {
    if !SENDER_STARTED.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let Some(tx) = sender() else {
        return;
    };
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    if let Ok(tx) = tx.lock() {
        let _ = tx.send(Job::Flush(done_tx));
    }
    let _ = done_rx.recv_timeout(std::time::Duration::from_millis(500));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> Entry {
        Entry {
            log: vec![Rec {
                tag: 3,
                a: name.into(),
                b: String::new(),
                h: 7,
            }],
            members: vec![],
            ids: vec![(4, IdRef::Registry("f".into()))],
        }
    }

    // A pack is read back whole; a damaged one, or a damaged entry in
    // one, is not trusted.
    #[test]
    fn a_pack_round_trips_and_a_damaged_one_is_refused() {
        let rows = vec![
            ([1u8; 16], 10, encode(&entry("len")).unwrap()),
            ([2u8; 16], 20, encode(&entry("map")).unwrap()),
        ];
        let bytes = encode_pack(rows);
        let pack = Pack::parse(bytes.clone()).expect("parses");
        let back = decode(pack.bodies_of(&[2u8; 16])[0].1).expect("decodes");
        assert_eq!(back.log[0].a, "map");
        assert!(pack.bodies_of(&[3u8; 16]).is_empty());
        // a truncation anywhere: the header or index fails, or the entry
        // it cuts into does
        for cut in 0..bytes.len() {
            if let Some(p) = Pack::parse(bytes[..cut].to_vec()) {
                let whole = !p.bodies_of(&[1u8; 16]).is_empty() && !p.bodies_of(&[2u8; 16]).is_empty();
                assert!(!whole, "cut at {cut} read as whole");
            }
        }
        // a flipped byte in a body: that entry's checksum fails
        let mut flipped = bytes.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0x55;
        let p = Pack::parse(flipped).expect("the index is intact");
        assert!(p.bodies_of(&[2u8; 16]).is_empty());
        assert!(Pack::parse(b"garbage that is not a pack at all".to_vec()).is_none());
    }

    // The newest entries are kept when a pack would hold too many.
    #[test]
    fn a_full_pack_keeps_the_newest() {
        let rows: Vec<(Key, u64, Vec<u8>)> = (0..PACK_MAX + 10)
            .map(|i| {
                let mut k = [0u8; 16];
                k[..8].copy_from_slice(&(i as u64).to_le_bytes());
                (k, i as u64, vec![1, 2, 3])
            })
            .collect();
        let pack = Pack::parse(encode_pack(rows)).expect("parses");
        assert_eq!(pack.rows.len(), PACK_MAX);
        assert!(pack.rows.values().flatten().all(|r| r.time >= 10));
    }
}

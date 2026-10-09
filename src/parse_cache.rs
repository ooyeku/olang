//! Parsed modules kept between runs.
//!
//! A program on a UI framework loads tens of thousands of lines of
//! modules that do not change from one launch to the next, and parsing
//! them was most of its cold start (olang Studio: ~175 ms of a ~390 ms
//! launch). The parsed tree of each module is kept on disk as the program
//! image keeps one for the browser (`crate::olb`: postcard behind a
//! header), and read back the next time the same bytes are loaded by the
//! same build of olang.
//!
//! One file a module, named by its path: the cache is as large as the
//! set of modules ever loaded, never a growing history. An entry is used
//! only when the build that wrote it is this one (its version, commit,
//! checkout state and build date — any change to olang's sources moves
//! the date) and the module's text hashes to what it held then; anything
//! else is parsed and written again. A module that uses macros is never
//! kept: its expansion depends on other files. Failures are silent — a
//! read-only home never fails a program.
//!
//! Stored under `~/.olang/state/parsed/` (`OLANG_HOME` moves it);
//! `OLANG_PARSE_CACHE=0` turns it off for A/B measurement.

use crate::ast::Program;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"olpc";

fn disabled() -> bool {
    matches!(std::env::var("OLANG_PARSE_CACHE").as_deref(), Ok("0"))
}

fn dir() -> Option<PathBuf> {
    crate::home::state().map(|s| s.join("parsed"))
}

/// The build an entry must have been written by.
fn stamp() -> String {
    format!(
        "{} {} {} {}",
        crate::version::VERSION,
        crate::version::BUILD_COMMIT,
        crate::version::BUILD_DIRTY,
        crate::version::BUILD_DATE
    )
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

fn entry(path: &Path) -> Option<PathBuf> {
    let key = sha(path.to_string_lossy().as_bytes());
    let name: String = key.iter().take(10).map(|b| format!("{:02x}", b)).collect();
    dir().map(|d| d.join(format!("{}.olpc", name)))
}

/// Whether a module's text may be kept: no macro anywhere in it (its
/// expansion reads other files), judged by the text, conservatively.
pub fn cacheable(source: &str) -> bool {
    !source.contains('@') && !source.contains("meta")
}

/// The tree `path` parsed to when it held `source`, from a run of this
/// build, or `None`.
pub fn load(path: &Path, source: &str) -> Option<Program> {
    if disabled() || !cacheable(source) {
        return None;
    }
    let bytes = std::fs::read(entry(path)?).ok()?;
    let stamp = stamp();
    let head = 4 + 2 + stamp.len() + 32;
    if bytes.len() < head || &bytes[0..4] != MAGIC {
        return None;
    }
    let n = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if n != stamp.len() || &bytes[6..6 + n] != stamp.as_bytes() {
        return None;
    }
    if bytes[6 + n..head] != sha(source.as_bytes()) {
        return None;
    }
    postcard::from_bytes(&bytes[head..]).ok()
}

/// Keep `program`, the tree `path` parsed to holding `source`. Written
/// beside and renamed over: a reader never meets half an entry.
pub fn store(path: &Path, source: &str, program: &Program) {
    if disabled() || !cacheable(source) {
        return;
    }
    let (Some(dir), Some(file)) = (dir(), entry(path)) else {
        return;
    };
    let Ok(body) = postcard::to_stdvec(program) else {
        return;
    };
    let stamp = stamp();
    let mut out = Vec::with_capacity(4 + 2 + stamp.len() + 32 + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(stamp.len() as u16).to_le_bytes());
    out.extend_from_slice(stamp.as_bytes());
    out.extend_from_slice(&sha(source.as_bytes()));
    out.extend_from_slice(&body);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let tmp = file.with_extension(format!("olpc.{}", std::process::id()));
    if std::fs::write(&tmp, &out).is_ok() && std::fs::rename(&tmp, &file).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // What is read back is the tree the parser made: an entry is the
    // program image's encoding of it.
    #[test]
    fn a_tree_round_trips() {
        let src = "use a.b { c }\n/// doc\nshare fn f(x, y = 2) = { let z = x + y\n match z { 0 => \"a\", n => to_string(n) } }\nlet T = #{ \"k\": [1, 2.5, (3, ())] }\ntest \"t\" { assert_eq(f(1), \"3\") }\n";
        let p = crate::parser::Parser::new().parse(src).expect("parses");
        let bytes = postcard::to_stdvec(&p).expect("encodes");
        let back: Program = postcard::from_bytes(&bytes).expect("decodes");
        assert_eq!(back, p);
    }

    #[test]
    fn a_module_with_macros_is_never_kept() {
        assert!(cacheable("fn f() = 1"));
        assert!(!cacheable("@derive fn f() = 1"));
        assert!(!cacheable("meta fn m(x) = x"));
    }
}

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
//! kept: its expansion depends on other files. Which modules do is the
//! parser's answer (`Parser::parse_module`), not a guess from the text:
//! an `@` in a string (`lsp@p1`) or the word `meta` in a comment kept a
//! module out, and a quarter of olang Studio's modules were parsed on
//! every launch for it. Failures are silent — a read-only home never
//! fails a program.
//!
//! Stored under `~/.olang/state/parsed/` (`OLANG_HOME` moves it);
//! `OLANG_PARSE_CACHE=0` turns it off for A/B measurement. Held as the
//! compile cache is: an entry no run has written or read for two weeks
//! is dropped, and past `OLANG_PARSE_CACHE_MB` (64) the entries least
//! recently used go first, down to three quarters of it, once a run
//! that writes one (OL-256: every module version ever parsed stayed,
//! 233 MB in 23,296 files).

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

/// The tree `path` parsed to when it held `source`, from a run of this
/// build, or `None`. Only a macro-free text was ever kept, and the entry
/// holds the text's hash: the same text is macro-free still.
pub fn load(path: &Path, source: &str) -> Option<Program> {
    if disabled() {
        return None;
    }
    let file = entry(path)?;
    let bytes = std::fs::read(&file).ok()?;
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
    let program = postcard::from_bytes(&bytes[head..]).ok()?;
    touch(&file);
    Some(program)
}

/// An entry read is used: its time moves to now, at most once a day, so
/// the eviction's least recently used is least recently read too.
fn touch(file: &Path) {
    let now = crate::clock::system_now();
    let stale = std::fs::metadata(file)
        .and_then(|m| m.modified())
        .is_ok_and(|t| now.duration_since(t).is_ok_and(|d| d.as_secs() > DAY));
    if stale && let Ok(f) = std::fs::File::options().append(true).open(file) {
        let _ = f.set_modified(now);
    }
}

const DAY: u64 = 24 * 60 * 60;

fn cap_bytes() -> u64 {
    std::env::var("OLANG_PARSE_CACHE_MB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(64)
        * 1024
        * 1024
}

/// Hold the directory to its policy: entries unused for two weeks go,
/// then, past `cap` bytes, the least recently used down to three
/// quarters of it; a temporary file older than a minute (its writer
/// gone) goes too.
fn evict(dir: &Path, cap: u64) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let now = crate::clock::system_now();
    let age = |t: std::time::SystemTime| now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
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
        if name.contains(".olpc.") {
            if age(modified) > 60 {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        if !name.ends_with(".olpc") {
            continue;
        }
        if age(modified) > 14 * DAY {
            let _ = std::fs::remove_file(&path);
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

/// Keep `program`, the tree `path` parsed to holding `source` — a text
/// with no macro in it (`Parser::parse_module` says so; the caller asks
/// it). Written beside and renamed over: a reader never meets half an
/// entry.
pub fn store(path: &Path, source: &str, program: &Program) {
    if disabled() {
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
    // once a run, off the program's path
    static EVICTED: std::sync::Once = std::sync::Once::new();
    EVICTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("olang-parse-cache".into())
            .spawn(move || evict(&dir, cap_bytes()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // What is read back is the tree the parser made: an entry is the
    // program image's encoding of it.
    #[test]
    fn eviction_drops_the_old_then_the_least_recently_used() {
        let dir = std::env::temp_dir().join(format!("olang-parse-evict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = std::time::SystemTime::now();
        let put = |name: &str, len: usize, days_ago: u64| {
            let p = dir.join(name);
            std::fs::write(&p, vec![0u8; len]).unwrap();
            let f = std::fs::File::options().append(true).open(&p).unwrap();
            f.set_modified(now - std::time::Duration::from_secs(days_ago * DAY + 5))
                .unwrap();
        };
        put("old.olpc", 10, 15);
        put("a.olpc", 400, 3);
        put("b.olpc", 400, 2);
        put("c.olpc", 400, 1);
        put("x.olpc.123", 10, 1);
        put("other.txt", 10, 30);
        evict(&dir, 1000);
        let left: std::collections::BTreeSet<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        // the two-week-old entry and the orphaned temp file go, then the
        // least recently used until three quarters of the cap
        assert_eq!(
            left,
            ["c.olpc", "other.txt"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tree_round_trips() {
        let src = "use a.b { c }\n/// doc\nshare fn f(x, y = 2) = { let z = x + y\n match z { 0 => \"a\", n => to_string(n) } }\nlet T = #{ \"k\": [1, 2.5, (3, ())] }\ntest \"t\" { assert_eq(f(1), \"3\") }\n";
        let p = crate::parser::Parser::new().parse(src).expect("parses");
        let bytes = postcard::to_stdvec(&p).expect("encodes");
        let back: Program = postcard::from_bytes(&bytes).expect("decodes");
        assert_eq!(back, p);
    }

    // Which texts may be kept is the parser's verdict: an `@` in a
    // string or `meta` in a comment is plain text, a macro call or a
    // `meta fn` is not.
    #[test]
    fn only_a_text_without_macros_is_kept() {
        let plain = |src: &str| {
            crate::parser::Parser::new()
                .parse_module(src, None)
                .map(|(_, plain)| plain)
                .unwrap_or(false)
        };
        assert!(plain("fn f() = 1"));
        assert!(plain(
            "let tag = \"lsp@p1\"\n// meta: a comment\nfn f() = tag"
        ));
        assert!(!plain("meta fn m(x) = x"));
    }
}

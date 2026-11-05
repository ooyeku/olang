//! The `str` functions that work in bytes rather than characters, and
//! the ones that answer a whole text's numbers in one pass.
//!
//! Every other `str` position counts characters, which is what a reader
//! means by "the fifth letter" — and costs a walk from the start to find
//! it. An editor, a diff or a rope holds byte offsets instead: they are
//! O(1) to slice at and stable under the encoding. These functions are
//! the bridge: `str.byte_len` and `str.slice_bytes` work in bytes,
//! `str.char_to_byte` / `str.byte_to_char` convert, `str.utf16_len`
//! counts what an LSP client or a browser counts, and `str.chunks` cuts
//! a text into pieces of at most so many bytes in one pass.
//!
//! A byte offset that falls inside a character raises: a slice there
//! would not be text, and no caller can recover from its own off-by-one.

use crate::ast::{Value, ValueMap};
use std::sync::Arc;

type R = Result<Value, Box<dyn std::error::Error>>;

/// The names `str` dispatches here, with the most arguments each takes.
pub(crate) const MAX_ARGS: &[(&str, usize)] = &[
    ("byte_len", 1),
    ("slice_bytes", 3),
    ("char_to_byte", 2),
    ("byte_to_char", 2),
    ("utf16_len", 1),
    ("chunks", 3),
    ("encode", 2),
    ("lines", 2),
    ("line_starts", 1),
];

/// The fewest arguments each takes (what the module registers, which
/// `olang check` holds a call to).
pub(crate) const MIN_ARGS: &[(&str, usize)] = &[
    ("byte_len", 1),
    ("slice_bytes", 3),
    ("char_to_byte", 2),
    ("byte_to_char", 2),
    ("utf16_len", 1),
    ("chunks", 2),
    ("encode", 2),
    ("lines", 1),
    ("line_starts", 1),
];

/// Calls one of this file's functions, or `None` when `name` is not one.
pub(crate) fn call(name: &str, args: &[Value]) -> Option<R> {
    Some(match name {
        "byte_len" => byte_len(args),
        "slice_bytes" => slice_bytes(args),
        "char_to_byte" => char_to_byte_fn(args),
        "byte_to_char" => byte_to_char_fn(args),
        "utf16_len" => utf16_len_fn(args),
        "chunks" => chunks(args),
        "encode" => encode(args),
        "lines" => lines(args),
        "line_starts" => line_starts(args),
        _ => return None,
    })
}

// ── arguments ───────────────────────────────────────────────────────

pub(crate) fn arg_str<'a>(args: &'a [Value], i: usize, func: &str) -> Result<&'a str, String> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.as_str()),
        Some(other) => Err(format!(
            "str.{func}: argument {} must be a string, got {}",
            i + 1,
            other.type_name()
        )),
        None => Err(format!("str.{func}: missing argument {}", i + 1)),
    }
}

/// A position argument: an Int that is not negative.
fn arg_pos(args: &[Value], i: usize, func: &str) -> Result<usize, String> {
    match args.get(i) {
        Some(Value::Integer(n)) if *n >= 0 => Ok(*n as usize),
        Some(Value::Integer(n)) => Err(format!(
            "str.{func}: position {n} is negative: a position counts from 0"
        )),
        Some(other) => Err(format!(
            "str.{func}: argument {} must be an integer, got {}",
            i + 1,
            other.type_name()
        )),
        None => Err(format!("str.{func}: missing argument {}", i + 1)),
    }
}

/// The options map at `i`, if one was passed: `()` is no options, an
/// unknown key raises naming the ones there are.
pub(crate) fn arg_opts<'a>(
    args: &'a [Value],
    i: usize,
    func: &str,
    known: &[&str],
) -> Result<Option<&'a ValueMap>, String> {
    match args.get(i) {
        None | Some(Value::Unit) => Ok(None),
        Some(Value::Map(m)) => {
            if let Some(k) = m.keys().find(|k| !known.contains(&k.as_str())) {
                return Err(format!(
                    "str.{func}: unknown option \"{k}\" (it takes {})",
                    known.join(", ")
                ));
            }
            Ok(Some(m))
        }
        Some(other) => Err(format!(
            "str.{func}: argument {} must be a map of options, got {}",
            i + 1,
            other.type_name()
        )),
    }
}

pub(crate) fn opt_bool(
    opts: Option<&ValueMap>,
    key: &str,
    func: &str,
    default: bool,
) -> Result<bool, String> {
    match opts.and_then(|m| m.get(key)) {
        None | Some(Value::Unit) => Ok(default),
        Some(Value::Boolean(b)) => Ok(*b),
        Some(other) => Err(format!(
            "str.{func}: option {key} must be true or false, got {}",
            other.type_name()
        )),
    }
}

pub(crate) fn string(s: &str) -> Value {
    Value::String(Arc::new(s.to_string()))
}

/// Where `b` falls, said for a caller who passed it: the character it
/// cuts and the boundaries either side.
fn not_a_boundary(func: &str, s: &str, b: usize) -> String {
    let before = (0..b).rev().find(|i| s.is_char_boundary(*i)).unwrap_or(0);
    let after = (b..=s.len()).find(|i| s.is_char_boundary(*i)).unwrap_or(s.len());
    format!(
        "str.{func}: byte {b} is inside the character {:?} (bytes {before}..{after}); a byte offset must fall between characters",
        &s[before..after]
    )
}

// ── counting ────────────────────────────────────────────────────────

/// How many characters start in `bytes`: every byte that is not a
/// continuation byte (10xxxxxx). A filter-count the compiler vectorises.
pub(crate) fn count_chars(bytes: &[u8]) -> usize {
    bytes.iter().filter(|b| (**b as i8) >= -0x40).count()
}

/// UTF-16 code units: one per character, two for a character outside
/// the Basic Multilingual Plane — which is exactly the characters whose
/// UTF-8 lead byte is 11110xxx.
pub(crate) fn utf16_units(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .map(|b| ((*b as i8) >= -0x40) as usize + (*b >= 0xF0) as usize)
        .sum()
}

// ── the functions ───────────────────────────────────────────────────

fn byte_len(args: &[Value]) -> R {
    Ok(Value::Integer(arg_str(args, 0, "byte_len")?.len() as i64))
}

fn utf16_len_fn(args: &[Value]) -> R {
    let s = arg_str(args, 0, "utf16_len")?;
    Ok(Value::Integer(utf16_units(s.as_bytes()) as i64))
}

/// `str.slice_bytes(s, a, b)`: bytes `a` to `b`, by the slices' one rule
/// (past the end stops at the end, `b` at or before `a` is `""`, a
/// negative position raises), and an offset inside a character raises.
fn slice_bytes(args: &[Value]) -> R {
    let s = arg_str(args, 0, "slice_bytes")?;
    let a = arg_pos(args, 1, "slice_bytes")?.min(s.len());
    let b = arg_pos(args, 2, "slice_bytes")?.min(s.len());
    for at in [a, b] {
        if !s.is_char_boundary(at) {
            return Err(not_a_boundary("slice_bytes", s, at).into());
        }
    }
    if b <= a {
        return Ok(string(""));
    }
    if a == 0 && b == s.len() {
        return Ok(args[0].clone());
    }
    Ok(string(&s[a..b]))
}

/// `str.char_to_byte(s, i)`: the byte offset where character `i` starts;
/// `str.length(s)` and past it answer `str.byte_len(s)`.
fn char_to_byte_fn(args: &[Value]) -> R {
    let s = arg_str(args, 0, "char_to_byte")?;
    let i = arg_pos(args, 1, "char_to_byte")?;
    Ok(Value::Integer(super::string::char_to_byte(s, 0, 0, i) as i64))
}

/// `str.byte_to_char(s, b)`: how many characters come before byte `b`;
/// past the end answers `str.length(s)`, inside a character raises.
fn byte_to_char_fn(args: &[Value]) -> R {
    let s = arg_str(args, 0, "byte_to_char")?;
    let b = arg_pos(args, 1, "byte_to_char")?.min(s.len());
    if !s.is_char_boundary(b) {
        return Err(not_a_boundary("byte_to_char", s, b).into());
    }
    Ok(Value::Integer(count_chars(&s.as_bytes()[..b]) as i64))
}

/// `str.chunks(s, max_bytes, #{prefer})`: `s` cut into pieces of at most
/// `max_bytes` bytes, each cut between characters, in one pass. With
/// `prefer` (a string, usually "\n"), a piece ends just after the last
/// `prefer` inside its window when there is one. Joined back, the pieces
/// are `s`. A character longer than `max_bytes` is a piece of its own.
fn chunks(args: &[Value]) -> R {
    let s = arg_str(args, 0, "chunks")?;
    let max = match args.get(1) {
        Some(Value::Integer(n)) if *n > 0 => *n as usize,
        Some(Value::Integer(n)) => {
            return Err(format!("str.chunks: max_bytes must be at least 1, got {n}").into());
        }
        Some(other) => {
            return Err(format!(
                "str.chunks: max_bytes must be an integer, got {}",
                other.type_name()
            )
            .into());
        }
        None => return Err("str.chunks: missing argument 2 (max_bytes)".into()),
    };
    let opts = arg_opts(args, 2, "chunks", &["prefer"])?;
    let prefer: Option<&str> = match opts.and_then(|m| m.get("prefer")) {
        None | Some(Value::Unit) => None,
        Some(Value::String(p)) if !p.is_empty() => Some(p.as_str()),
        Some(Value::String(_)) => {
            return Err("str.chunks: option prefer must not be empty".into());
        }
        Some(other) => {
            return Err(format!(
                "str.chunks: option prefer must be a string, got {}",
                other.type_name()
            )
            .into());
        }
    };
    let bytes = s.as_bytes();
    let mut out: Vec<Value> = Vec::with_capacity(s.len() / max + 1);
    let mut pos = 0;
    while pos < bytes.len() {
        let mut end = (pos + max).min(bytes.len());
        if end < bytes.len() {
            if let Some(p) = prefer {
                let window = &bytes[pos..end];
                let found = if p.len() == 1 {
                    memchr::memrchr(p.as_bytes()[0], window)
                } else {
                    memchr::memmem::rfind(window, p.as_bytes())
                };
                if let Some(i) = found {
                    let cut = pos + i + p.len();
                    if cut <= end {
                        end = cut;
                    }
                }
            }
            while end > pos && !s.is_char_boundary(end) {
                end -= 1;
            }
            if end == pos {
                // One character wider than the window: it goes alone.
                end = pos + 1;
                while !s.is_char_boundary(end) {
                    end += 1;
                }
            }
        }
        out.push(string(&s[pos..end]));
        pos = end;
    }
    Ok(Value::List(Arc::new(out)))
}

/// `str.encode(s, encoding)`: the text as Bytes in "utf-8", "utf-16le",
/// "utf-16be" or "latin1" — a `Result`, `Err` naming the first
/// character Latin-1 has no byte for (`bytes.decode` is the way back).
fn encode(args: &[Value]) -> R {
    let s = arg_str(args, 0, "encode")?;
    let enc = super::bytes::encoding_of(arg_str(args, 1, "encode")?, "str.encode")?;
    Ok(match super::bytes::encode(s, enc) {
        Ok(b) => Value::Ok(Box::new(super::bytes::to_value(b))),
        Err(e) => Value::Err(Box::new(string(&e))),
    })
}

// ── lines (OL-472) ──────────────────────────────────────────────────

/// `str.lines(s, #{keep_ends})`: the lines of `s`. A line ends at "\n"
/// or "\r\n". By default the ends are dropped and a final line break
/// starts no line (Rust's `lines`): "a\r\nb\n" is ["a", "b"]. With
/// `keep_ends: true` every line keeps its own terminator, so joined back
/// they are `s` exactly: ["a\r\n", "b\n"] — what an exact patch needs
/// ("\ No newline at end of file" is a last line without one).
fn lines(args: &[Value]) -> R {
    let s = arg_str(args, 0, "lines")?;
    let opts = arg_opts(args, 1, "lines", &["keep_ends"])?;
    let keep = opt_bool(opts, "keep_ends", "lines", false)?;
    let out: Vec<Value> = if keep {
        s.split_inclusive('\n').map(string).collect()
    } else {
        s.lines().map(string).collect()
    };
    Ok(Value::List(Arc::new(out)))
}

/// `str.line_starts(s)`: the byte offset where each line starts — 0, and
/// one past every "\n". An editor's line index: a text ending in a line
/// break has an empty last line, so there is always one more start than
/// there are "\n"s. Line `n` is `str.slice_bytes(s, starts[n], starts[n + 1])`.
fn line_starts(args: &[Value]) -> R {
    let s = arg_str(args, 0, "line_starts")?;
    let mut out = Vec::with_capacity(s.len() / 32 + 1);
    out.push(Value::Integer(0));
    out.extend(memchr::memchr_iter(b'\n', s.as_bytes()).map(|i| Value::Integer(i as i64 + 1)));
    Ok(Value::List(Arc::new(out)))
}


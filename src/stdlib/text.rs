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
    ("stats", 1),
    ("word_bounds", 1),
    ("line_breaks", 2),
    ("bidi_runs", 2),
    ("normalize", 2),
    ("fold", 1),
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
    ("stats", 1),
    ("word_bounds", 1),
    ("line_breaks", 1),
    ("bidi_runs", 1),
    ("normalize", 2),
    ("fold", 1),
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
        "stats" => stats_fn(args),
        "word_bounds" => word_bounds(args),
        "line_breaks" => line_breaks(args),
        "bidi_runs" => bidi_runs(args),
        "normalize" => normalize(args),
        "fold" => fold(args),
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

// ── one-pass numbers (OL-452) ───────────────────────────────────────

/// A text's numbers, each what its own function would answer.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Stats {
    pub bytes: usize,
    pub chars: usize,
    /// `str.line_starts`'s count: one more than the "\n"s.
    pub lines: usize,
    pub graphemes: usize,
    pub utf16: usize,
    /// `str.width`.
    pub width: usize,
    /// Characters after the last "\n" (all of them without one).
    pub last_line_len: usize,
}

/// Graphemes and width of an ASCII stretch: every byte is a cluster but
/// a "\r\n" pair, and a cluster is a cell unless it holds a control.
fn ascii_clusters(b: &[u8]) -> (usize, usize) {
    let crlf = b.windows(2).filter(|w| w[0] == b'\r' && w[1] == b'\n').count();
    let printable = b.iter().filter(|&&c| (0x20..0x7F).contains(&c)).count();
    (b.len() - crlf, printable)
}

/// Graphemes and width of any stretch, by UAX #29.
fn segmented_clusters(s: &str) -> (usize, usize) {
    use unicode_segmentation::UnicodeSegmentation;
    let mut n = 0;
    let mut w = 0;
    for g in s.graphemes(true) {
        n += 1;
        w += super::string::cell_width_of(g);
    }
    (n, w)
}

/// `s`'s numbers in passes over memory. ASCII stretches are counted a
/// byte at a time (vectorised); the rest is segmented, each stretch
/// taking the ASCII character either side of it, which is the most a
/// cluster can reach into ASCII (a base before combining marks, a
/// character after a Prepend), and a "\r\n" whole.
pub(crate) fn stats(s: &str) -> Stats {
    let b = s.as_bytes();
    let newlines = memchr::memchr_iter(b'\n', b).count();
    let last_start = memchr::memrchr(b'\n', b).map(|i| i + 1).unwrap_or(0);
    let (graphemes, width) = if b.is_ascii() {
        ascii_clusters(b)
    } else {
        let (mut n, mut w) = (0, 0);
        let mut i = 0;
        while i < b.len() {
            // the ASCII run from i
            let run_end = b[i..].iter().position(|c| !c.is_ascii()).map_or(b.len(), |p| i + p);
            if run_end == b.len() {
                let (a, c) = ascii_clusters(&b[i..]);
                n += a;
                w += c;
                break;
            }
            // keep the run's last character (and a "\r\n") for the segment
            let mut keep = run_end;
            if keep > i {
                keep -= 1;
                if keep > i && b[keep] == b'\n' && b[keep - 1] == b'\r' {
                    keep -= 1;
                }
            }
            let (a, c) = ascii_clusters(&b[i..keep]);
            n += a;
            w += c;
            // the non-ASCII stretch, and the ASCII character after it
            let mut end = b[run_end..].iter().position(|c| c.is_ascii()).map_or(b.len(), |p| run_end + p);
            if end < b.len() {
                end += if b[end] == b'\r' && b.get(end + 1) == Some(&b'\n') { 2 } else { 1 };
            }
            let (a, c) = segmented_clusters(&s[keep..end]);
            n += a;
            w += c;
            i = end;
        }
        (n, w)
    };
    Stats {
        bytes: b.len(),
        chars: count_chars(b),
        lines: newlines + 1,
        graphemes,
        utf16: utf16_units(b),
        width,
        last_line_len: count_chars(&b[last_start..]),
    }
}

/// `str.stats(s)`: `#{bytes, chars, lines, graphemes, utf16, width,
/// last_line_len}` in one call, at memory speed for ASCII.
fn stats_fn(args: &[Value]) -> R {
    let st = stats(arg_str(args, 0, "stats")?);
    let mut m = ValueMap::default();
    for (k, v) in [
        ("bytes", st.bytes),
        ("chars", st.chars),
        ("lines", st.lines),
        ("graphemes", st.graphemes),
        ("utf16", st.utf16),
        ("width", st.width),
        ("last_line_len", st.last_line_len),
    ] {
        m.insert(k.to_string(), Value::Integer(v as i64));
    }
    Ok(Value::Map(Arc::new(m)))
}

// ── Unicode text algorithms (OL-470) ────────────────────────────────

fn offsets(xs: impl Iterator<Item = usize>) -> Value {
    Value::List(Arc::new(xs.map(|i| Value::Integer(i as i64)).collect()))
}

/// `str.word_bounds(s)`: the byte offsets of every UAX #29 word
/// boundary, 0 and the end included. Between two neighbours is a word,
/// a run of spaces or one punctuation mark: what word motion and a word
/// diff step over. `str.words` (split on whitespace) is the simple form.
fn word_bounds(args: &[Value]) -> R {
    use unicode_segmentation::UnicodeSegmentation;
    let s = arg_str(args, 0, "word_bounds")?;
    let starts = s.split_word_bound_indices().map(|(i, _)| i);
    if s.is_empty() {
        return Ok(offsets(std::iter::once(0)));
    }
    Ok(offsets(starts.chain(std::iter::once(s.len()))))
}

/// `str.line_breaks(s, #{hard})`: the byte offsets where UAX #14 allows a
/// line to break (a wrap may cut there), the end of the text included.
/// With `hard: true`, only the breaks that must happen (after a line
/// break character).
fn line_breaks(args: &[Value]) -> R {
    use unicode_linebreak::BreakOpportunity;
    let s = arg_str(args, 0, "line_breaks")?;
    let opts = arg_opts(args, 1, "line_breaks", &["hard"])?;
    let hard = opt_bool(opts, "hard", "line_breaks", false)?;
    Ok(offsets(
        unicode_linebreak::linebreaks(s)
            .filter(|(_, o)| !hard || *o == BreakOpportunity::Mandatory)
            .map(|(i, _)| i),
    ))
}

/// `str.bidi_runs(s, #{direction, visual})`: the text's runs of one
/// UAX #9 embedding level, `#{start, end, level}` in bytes (an odd level
/// is right-to-left). Each paragraph's direction comes from its first
/// strong character unless `direction` is "ltr" or "rtl". In logical
/// order by default; `visual: true` gives each paragraph's runs in the
/// order a line shows them (each paragraph taken as one line).
fn bidi_runs(args: &[Value]) -> R {
    use unicode_bidi::{BidiInfo, Level};
    let s = arg_str(args, 0, "bidi_runs")?;
    let opts = arg_opts(args, 1, "bidi_runs", &["direction", "visual"])?;
    let visual = opt_bool(opts, "visual", "bidi_runs", false)?;
    let level = match opts.and_then(|m| m.get("direction")) {
        None | Some(Value::Unit) => None,
        Some(Value::String(d)) if d.as_str() == "auto" => None,
        Some(Value::String(d)) if d.as_str() == "ltr" => Some(Level::ltr()),
        Some(Value::String(d)) if d.as_str() == "rtl" => Some(Level::rtl()),
        Some(other) => {
            return Err(format!(
                "str.bidi_runs: option direction must be \"ltr\", \"rtl\" or \"auto\", got {}",
                match other {
                    Value::String(d) => format!("{d:?}"),
                    v => v.type_name(),
                }
            )
            .into());
        }
    };
    let info = BidiInfo::new(s, level);
    let run = |start: usize, end: usize, level: Level| {
        let mut m = ValueMap::default();
        m.insert("start".to_string(), Value::Integer(start as i64));
        m.insert("end".to_string(), Value::Integer(end as i64));
        m.insert("level".to_string(), Value::Integer(level.number() as i64));
        Value::Map(Arc::new(m))
    };
    let mut out = Vec::new();
    for para in &info.paragraphs {
        if visual {
            let (levels, runs) = info.visual_runs(para, para.range.clone());
            for r in runs {
                out.push(run(r.start, r.end, levels[r.start]));
            }
        } else {
            let mut start = para.range.start;
            for i in para.range.clone() {
                if s.is_char_boundary(i) && info.levels[i] != info.levels[start] {
                    out.push(run(start, i, info.levels[start]));
                    start = i;
                }
            }
            if start < para.range.end {
                out.push(run(start, para.range.end, info.levels[start]));
            }
        }
    }
    Ok(Value::List(Arc::new(out)))
}

/// `str.normalize(s, form)`: the text in Unicode normalization form
/// "nfc", "nfd", "nfkc" or "nfkd". NFC is what to compare and store:
/// "é" typed as one character and as "e" with a combining accent are
/// equal once both are NFC.
fn normalize(args: &[Value]) -> R {
    let s = arg_str(args, 0, "normalize")?;
    let form = arg_str(args, 1, "normalize")?;
    let out = match form.to_ascii_lowercase().as_str() {
        "nfc" => icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(s),
        "nfkc" => icu_normalizer::ComposingNormalizerBorrowed::new_nfkc().normalize(s),
        "nfd" => icu_normalizer::DecomposingNormalizerBorrowed::new_nfd().normalize(s),
        "nfkd" => icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd().normalize(s),
        _ => {
            return Err(format!(
                "str.normalize: unknown form {form:?} (it takes \"nfc\", \"nfd\", \"nfkc\" or \"nfkd\")"
            )
            .into());
        }
    };
    Ok(match out {
        std::borrow::Cow::Borrowed(b) if b.len() == s.len() => args[0].clone(),
        other => string(&other),
    })
}

/// `str.fold(s)`: Unicode full case folding, the form to compare text
/// ignoring case in ("Straße" and "STRASSE" fold alike, and "İ" to
/// "i̇" rather than to_lower's guess). Not for display.
fn fold(args: &[Value]) -> R {
    let s = arg_str(args, 0, "fold")?;
    Ok(string(&icu_casemap::CaseMapper::new().fold_string(s)))
}

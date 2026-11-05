//! `bytes` — immutable binary data.
//!
//! olang's strings are UTF-8 text, which left binary data — an image, a
//! gzip, a non-UTF-8 file, a hash's raw output — with no value to be.
//! `Bytes` is that value: an immutable byte sequence carried as a native
//! handle, so it crosses the tier boundary as a refcount bump like every
//! other native value, compares structurally, subscripts (`b[i]` is the
//! byte at `i` as an Int, negative indices from the end), and answers
//! `len`.
//!
//! ```text
//! let b = bytes.from_list([104, 105])
//! len(b)                     // 2
//! b[0]                       // 104
//! unwrap(bytes.to_string(b)) // "hi"
//! ```
//!
//! Conventions (docs/stdlib.md): construction from a list validates its
//! elements and raises on misuse (rule 3); `bytes.to_string` returns a
//! `Result` because "these bytes are not UTF-8" is the caller's decision
//! (rule 2); everything else is infallible and returns its value.
//! `fs.read_bytes` / `fs.write_bytes` live in `fs` and carry its
//! capability gates.

use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::sync::Arc;

/// A byte sequence: a range of a shared buffer, so `bytes.slice` is a view
/// that copies nothing. The buffer is owned for bytes a program made, or
/// borrowed for bytes the binary already holds for the life of the
/// process (the embedded browser runtime), so handing those to a program
/// is not a copy of them either.
///
/// `#[repr(C)]` with the data pointer and the length first: native code
/// reads a byte as two loads and a bounds check (jit.rs, `ListBytes`).
#[repr(C)]
pub struct BytesObject {
    /// The first byte of the range, inside `buf` (which keeps it alive and
    /// never changes: the buffer is immutable once made).
    ptr: *const u8,
    len: usize,
    buf: Buf,
}

// The pointer is into `buf`, owned by this object and never written.
unsafe impl Send for BytesObject {}
unsafe impl Sync for BytesObject {}

/// What keeps `ptr`'s bytes alive; read only through `ptr`.
#[derive(Clone)]
#[allow(dead_code)]
enum Buf {
    Static(&'static [u8]),
    Owned(Arc<Vec<u8>>),
}

impl BytesObject {
    /// Where native code finds the data pointer and the length.
    pub const PTR_OFFSET: i32 = std::mem::offset_of!(BytesObject, ptr) as i32;
    pub const LEN_OFFSET: i32 = std::mem::offset_of!(BytesObject, len) as i32;

    pub fn new(bytes: Vec<u8>) -> BytesObject {
        let buf = Arc::new(bytes);
        BytesObject {
            ptr: buf.as_ptr(),
            len: buf.len(),
            buf: Buf::Owned(buf),
        }
    }

    fn from_static(bytes: &'static [u8]) -> BytesObject {
        BytesObject {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
            buf: Buf::Static(bytes),
        }
    }

    /// The bytes.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `ptr..ptr + len` lies inside `buf`, which this object
        // holds and which is never mutated or moved (an Arc'd Vec or a
        // static slice).
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// `from..to` of these bytes, sharing their buffer.
    fn view(&self, from: usize, to: usize) -> BytesObject {
        debug_assert!(from <= to && to <= self.len);
        BytesObject {
            // SAFETY: from <= len, so the pointer stays in the range.
            ptr: unsafe { self.ptr.add(from) },
            len: to - from,
            buf: self.buf.clone(),
        }
    }
}

impl std::fmt::Debug for BytesObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl NativeObject for BytesObject {
    fn module(&self) -> &'static str {
        "bytes"
    }
    fn type_name(&self) -> &'static str {
        "Bytes"
    }
    fn display(&self) -> String {
        // A hex preview, capped: value displays are for humans and logs,
        // and a megabyte of hex is neither.
        const PREVIEW: usize = 16;
        let head: String = self
            .as_slice()
            .iter()
            .take(PREVIEW)
            .map(|b| format!("{b:02x}"))
            .collect();
        if self.len > PREVIEW {
            format!("b\"{head}…\" ({} bytes)", self.len)
        } else {
            format!("b\"{head}\"")
        }
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<BytesObject>()
            .map(|o| self.as_slice() == o.as_slice())
            .unwrap_or(false)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn length(&self) -> Option<usize> {
        Some(self.len)
    }
    fn hash64(&self) -> Option<u64> {
        Some(xxhash_rust::xxh3::xxh3_64(self.as_slice()))
    }
    fn index(&self, key: &Value) -> Option<Result<Value, String>> {
        let i = match key {
            Value::Integer(i) => *i,
            other => {
                return Some(Err(format!(
                    "Bytes takes an integer position, got {}",
                    other.type_name()
                )));
            }
        };
        let n = self.len as i64;
        // A negative position is out of range, as a list's is.
        let idx = i;
        if idx < 0 || idx >= n {
            return Some(Err(format!(
                "Index {i} out of bounds for Bytes of length {n}"
            )));
        }
        Some(Ok(Value::Integer(self.as_slice()[idx as usize] as i64)))
    }
}

/// Wrap raw bytes as an olang value.
pub fn to_value(bytes: Vec<u8>) -> Value {
    Value::Native(NativeHandle::new(BytesObject::new(bytes)))
}

/// Wrap bytes that live as long as the process, without copying them.
pub fn static_value(bytes: &'static [u8]) -> Value {
    Value::Native(NativeHandle::new(BytesObject::from_static(bytes)))
}

/// The byte at `i` of a Bytes value, for the VM's subscript: `None` for
/// anything that is not Bytes (the native's own `index` then answers).
#[inline]
pub fn byte_at(h: &NativeHandle, i: i64) -> Option<Result<i64, String>> {
    let b = h.0.as_any().downcast_ref::<BytesObject>()?;
    let s = b.as_slice();
    Some(if i >= 0 && (i as usize) < s.len() {
        Ok(s[i as usize] as i64)
    } else {
        Err(format!(
            "Index {i} out of bounds for Bytes of length {}",
            s.len()
        ))
    })
}

/// Pull the byte slice back out of a value, or say what it was.
pub fn bytes_of(value: &Value) -> Result<&[u8], String> {
    match value {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<BytesObject>()
                .map(|b| b.as_slice())
                .ok_or_else(|| format!("expected Bytes, got a {} handle", h.0.type_name()))
        }
        other => Err(format!("expected Bytes, got {}", other.type_name())),
    }
}

pub fn create_bytes_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [
        ("from_list", 1),
        ("to_list", 1),
        ("from_string", 1),
        ("to_string", 1),
        ("len", 1),
        ("slice", 3),
        ("concat", 2),
        ("to_hex", 1),
        ("from_hex", 1),
        ("to_string_lossy", 1),
        ("decode", 2),
        ("join", 2),
        ("builder", 0),
        ("push", 2),
        ("build", 1),
        ("index_of", 3),
        ("compare", 2),
        ("pack", 2),
        ("unpack", 3),
        ("get_u8", 2),
        ("get_i8", 2),
        ("get_u16", 3),
        ("get_i16", 3),
        ("get_u32", 3),
        ("get_i32", 3),
        ("get_u64", 3),
        ("get_i64", 3),
        ("get_f32", 3),
        ("get_f64", 3),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("bytes.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

pub fn call_bytes_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "from_list" => from_list(args),
        "to_list" => to_list(args),
        "from_string" => from_string(args),
        "to_string" => to_string(args),
        "len" => len(args),
        "slice" => slice(args),
        "concat" => concat(args),
        "to_hex" => to_hex(args),
        "from_hex" => from_hex(args),
        "to_string_lossy" => to_string_lossy(args),
        "decode" => decode_fn(args),
        "join" => join(args),
        "builder" => builder(args),
        "push" => push(args),
        "build" => build(args),
        "index_of" => index_of(args),
        "compare" => compare(args),
        "pack" => pack(args),
        "unpack" => unpack(args),
        _ if name.starts_with("get_") => get_scalar(&name[4..], args),
        _ => Err(format!("Unknown bytes function: {}", name).into()),
    }
}

fn one_arg<'a>(args: &'a [Value], name: &str) -> Result<&'a Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "bytes.{name} expects 1 argument, got {}",
            args.len()
        ));
    }
    Ok(&args[0])
}

fn from_list(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let arg = one_arg(&args, "from_list")?;
    let items = match arg {
        Value::List(items) => items,
        other => {
            return Err(format!(
                "bytes.from_list: argument must be a list of integers 0..=255, got {}",
                other.type_name()
            )
            .into());
        }
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        match item {
            Value::Integer(n) if (0..=255).contains(n) => out.push(*n as u8),
            Value::Integer(n) => {
                return Err(format!(
                    "bytes.from_list: element {i} is {n}, outside a byte's range 0..=255"
                )
                .into());
            }
            other => {
                return Err(format!(
                    "bytes.from_list: element {i} must be an integer, got {}",
                    other.type_name()
                )
                .into());
            }
        }
    }
    Ok(to_value(out))
}

fn to_list(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let b = bytes_of(one_arg(&args, "to_list")?).map_err(|e| format!("bytes.to_list: {e}"))?;
    Ok(Value::List(Arc::new(
        b.iter().map(|&x| Value::Integer(x as i64)).collect(),
    )))
}

fn from_string(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    match one_arg(&args, "from_string")? {
        Value::String(s) => Ok(to_value(s.as_bytes().to_vec())),
        other => Err(format!(
            "bytes.from_string: argument must be a string, got {}",
            other.type_name()
        )
        .into()),
    }
}

/// UTF-8 decode. A `Result`, not a raise: whether these bytes are text is
/// a property of the data, and the caller decides what a non-text answer
/// means.
fn to_string(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let b = bytes_of(one_arg(&args, "to_string")?).map_err(|e| format!("bytes.to_string: {e}"))?;
    Ok(match std::str::from_utf8(b) {
        Ok(s) => Value::Ok(Box::new(Value::String(Arc::new(s.to_string())))),
        Err(e) => Value::Err(Box::new(Value::String(Arc::new(format!(
            "bytes are not valid UTF-8: {e}"
        ))))),
    })
}

/// Lowercase hex, two digits a byte (heddle-sql drew a blob's bytes
/// through a 256-entry table in olang: ~90 ms a MB).
fn to_hex(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let b = bytes_of(one_arg(&args, "to_hex")?).map_err(|e| format!("bytes.to_hex: {e}"))?;
    let mut out = String::with_capacity(b.len() * 2);
    for &x in b.iter() {
        out.push(DIGITS[(x >> 4) as usize] as char);
        out.push(DIGITS[(x & 15) as usize] as char);
    }
    Ok(Value::String(Arc::new(out)))
}

/// Hex text (either case, an even number of digits) as Bytes — a
/// `Result`, since text may not be hex.
fn from_hex(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let s = match one_arg(&args, "from_hex")? {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "bytes.from_hex: argument must be a string, got {}",
                other.type_name()
            )
            .into());
        }
    };
    let digit = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let text = s.as_bytes();
    if text.len() % 2 != 0 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "bytes.from_hex: an odd number of digits ({})",
            text.len()
        ))))));
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    for (i, pair) in text.chunks(2).enumerate() {
        match (digit(pair[0]), digit(pair[1])) {
            (Some(h), Some(l)) => out.push(h << 4 | l),
            _ => {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "bytes.from_hex: not a hex digit at {}",
                    if digit(pair[0]).is_none() {
                        i * 2
                    } else {
                        i * 2 + 1
                    }
                ))))));
            }
        }
    }
    Ok(Value::Ok(Box::new(to_value(out))))
}

fn len(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let b = bytes_of(one_arg(&args, "len")?).map_err(|e| format!("bytes.len: {e}"))?;
    Ok(Value::Integer(b.len() as i64))
}

/// Half-open, by the slices' one rule (`str.substring`): a negative
/// position raises, one past the end stops at the end. A view: the
/// answer shares the bytes it was cut from, so slicing copies nothing.
fn slice(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!("bytes.slice expects 3 arguments, got {}", args.len()).into());
    }
    let b = bytes_obj(&args[0]).map_err(|e| format!("bytes.slice: {e}"))?;
    let (from, to) = match (&args[1], &args[2]) {
        (Value::Integer(f), Value::Integer(t)) => (*f, *t),
        _ => return Err("bytes.slice: from and to must be integers".into()),
    };
    if from < 0 || to < 0 {
        return Err(format!(
            "bytes.slice: position {} is negative: a position counts from 0",
            if from < 0 { from } else { to }
        )
        .into());
    }
    let n = b.len as i64;
    let from = from.min(n) as usize;
    let to = (to.min(n) as usize).max(from);
    Ok(Value::Native(NativeHandle::new(b.view(from, to))))
}

/// The Bytes object behind a value, or what it was instead.
fn bytes_obj(value: &Value) -> Result<&BytesObject, String> {
    match value {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<BytesObject>()
                .ok_or_else(|| format!("expected Bytes, got a {} handle", h.0.type_name()))
        }
        other => Err(format!("expected Bytes, got {}", other.type_name())),
    }
}

fn concat(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("bytes.concat expects 2 arguments, got {}", args.len()).into());
    }
    let a = bytes_of(&args[0]).map_err(|e| format!("bytes.concat: {e}"))?;
    let b = bytes_of(&args[1]).map_err(|e| format!("bytes.concat: {e}"))?;
    let mut out = Vec::with_capacity(a.len() + b.len());
    out.extend_from_slice(a);
    out.extend_from_slice(b);
    Ok(to_value(out))
}

// ── text in other encodings (OL-433) ────────────────────────────────
//
// `to_string` is strict UTF-8, which is right for data and wrong for
// opening any file a person has: one stray byte from an old editor and
// the whole file was an `Err`. These decode what text there is.

/// The encodings `bytes.decode`, `str.encode` and `fs.read_file` name.
pub const ENCODINGS: &[&str] = &["utf-8", "utf-16le", "utf-16be", "latin1"];

/// An encoding's name as a caller writes it ("UTF-8", "utf8", "latin-1",
/// "iso-8859-1"), or the misuse said.
pub fn encoding_of(name: &str, func: &str) -> Result<&'static str, String> {
    let n = name.to_ascii_lowercase().replace('_', "-");
    Ok(match n.as_str() {
        "utf-8" | "utf8" => "utf-8",
        "utf-16le" | "utf16le" => "utf-16le",
        "utf-16be" | "utf16be" => "utf-16be",
        "latin1" | "latin-1" | "iso-8859-1" => "latin1",
        _ => {
            return Err(format!(
                "{func}: unknown encoding \"{name}\" (it takes {})",
                ENCODINGS.join(", ")
            ));
        }
    })
}

/// `b` decoded as `encoding`. With `lossy`, each sequence that is not
/// text becomes U+FFFD and the answer is always `Ok`; without, the first
/// one is an `Err` saying where it is.
pub fn decode(b: &[u8], encoding: &str, lossy: bool) -> Result<String, String> {
    match encoding {
        "utf-8" => {
            if lossy {
                Ok(String::from_utf8_lossy(b).into_owned())
            } else {
                std::str::from_utf8(b)
                    .map(str::to_string)
                    .map_err(|e| format!("bytes are not valid UTF-8: {e}"))
            }
        }
        "latin1" => Ok(b.iter().map(|&x| x as char).collect()),
        "utf-16le" | "utf-16be" => {
            let le = encoding == "utf-16le";
            let odd = b.len() % 2 == 1;
            if odd && !lossy {
                return Err(format!(
                    "bytes are not valid UTF-16: an odd number of bytes ({})",
                    b.len()
                ));
            }
            let units = b.chunks_exact(2).map(|p| {
                if le {
                    u16::from_le_bytes([p[0], p[1]])
                } else {
                    u16::from_be_bytes([p[0], p[1]])
                }
            });
            let mut out = String::with_capacity(b.len() / 2);
            for (i, c) in char::decode_utf16(units).enumerate() {
                match c {
                    Ok(c) => out.push(c),
                    Err(_) if lossy => out.push(char::REPLACEMENT_CHARACTER),
                    Err(e) => {
                        return Err(format!(
                            "bytes are not valid UTF-16: the unpaired surrogate {:#06x} near byte {}",
                            e.unpaired_surrogate(),
                            i * 2
                        ));
                    }
                }
            }
            if odd {
                out.push(char::REPLACEMENT_CHARACTER);
            }
            Ok(out)
        }
        other => Err(format!("unknown encoding {other}")),
    }
}

/// `s` in `encoding`: always for the UTF encodings, an `Err` naming the
/// first character Latin-1 has no byte for.
pub fn encode(s: &str, encoding: &str) -> Result<Vec<u8>, String> {
    Ok(match encoding {
        "utf-8" => s.as_bytes().to_vec(),
        "utf-16le" => s.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        "utf-16be" => s.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        "latin1" => {
            let mut out = Vec::with_capacity(s.len());
            for (i, c) in s.chars().enumerate() {
                if (c as u32) > 0xFF {
                    return Err(format!(
                        "Latin-1 has no byte for {c:?} (U+{:04X}, character {i})",
                        c as u32
                    ));
                }
                out.push(c as u8);
            }
            out
        }
        other => return Err(format!("unknown encoding {other}")),
    })
}

fn ok_text(r: Result<String, String>) -> Value {
    match r {
        Ok(s) => Value::Ok(Box::new(Value::String(Arc::new(s)))),
        Err(e) => Value::Err(Box::new(Value::String(Arc::new(e)))),
    }
}

/// `bytes.to_string_lossy(b)`: UTF-8, each bad sequence U+FFFD.
fn to_string_lossy(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let b = bytes_of(one_arg(&args, "to_string_lossy")?)
        .map_err(|e| format!("bytes.to_string_lossy: {e}"))?;
    Ok(Value::String(Arc::new(String::from_utf8_lossy(b).into_owned())))
}

/// `bytes.decode(b, encoding, #{lossy})`: a `Result`, always `Ok` with
/// `lossy: true`.
fn decode_fn(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !(2..=3).contains(&args.len()) {
        return Err(format!("bytes.decode expects 2 or 3 arguments, got {}", args.len()).into());
    }
    let b = bytes_of(&args[0]).map_err(|e| format!("bytes.decode: {e}"))?;
    let enc = match &args[1] {
        Value::String(s) => encoding_of(s, "bytes.decode")?,
        other => {
            return Err(format!(
                "bytes.decode: the encoding must be a string, got {}",
                other.type_name()
            )
            .into());
        }
    };
    let lossy = lossy_option(args.get(2), "bytes.decode", &["lossy"])?;
    Ok(ok_text(decode(b, enc, lossy)))
}

/// The `lossy` key of an options map (`()` or absent is no options), an
/// unknown key raising.
pub fn lossy_option(opts: Option<&Value>, func: &str, known: &[&str]) -> Result<bool, String> {
    match opts {
        None | Some(Value::Unit) => Ok(false),
        Some(Value::Map(m)) => {
            if let Some(k) = m.keys().find(|k| !known.contains(&k.as_str())) {
                return Err(format!(
                    "{func}: unknown option \"{k}\" (it takes {})",
                    known.join(", ")
                ));
            }
            match m.get("lossy") {
                None | Some(Value::Unit) => Ok(false),
                Some(Value::Boolean(b)) => Ok(*b),
                Some(other) => Err(format!(
                    "{func}: option lossy must be true or false, got {}",
                    other.type_name()
                )),
            }
        }
        Some(other) => Err(format!(
            "{func}: the options must be a map, got {}",
            other.type_name()
        )),
    }
}

/// `bytes.join(parts, sep)`: the parts one after another, `sep` between
/// them; one allocation of the whole. Each part is Bytes or a String.
fn join(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let (parts, sep) = match args.as_slice() {
        [Value::List(parts)] => (parts, &[][..]),
        [Value::List(parts), sep] => (
            parts,
            piece(sep).map_err(|e| format!("bytes.join: the separator: {e}"))?,
        ),
        [other, ..] if !matches!(other, Value::List(_)) => {
            return Err(format!(
                "bytes.join: the parts must be a list of Bytes, got {}",
                other.type_name()
            )
            .into());
        }
        _ => return Err(format!("bytes.join expects 1 or 2 arguments, got {}", args.len()).into()),
    };
    let mut pieces = Vec::with_capacity(parts.len());
    for (i, p) in parts.iter().enumerate() {
        pieces.push(piece(p).map_err(|e| format!("bytes.join: part {i}: {e}"))?);
    }
    let total: usize =
        pieces.iter().map(|p| p.len()).sum::<usize>() + sep.len() * pieces.len().saturating_sub(1);
    let mut out = Vec::with_capacity(total);
    for (i, p) in pieces.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(sep);
        }
        out.extend_from_slice(p);
    }
    Ok(to_value(out))
}

/// The bytes a part stands for: Bytes as they are, a String as its UTF-8.
fn piece(v: &Value) -> Result<&[u8], String> {
    match v {
        Value::String(s) => Ok(s.as_bytes()),
        other => bytes_of(other)
            .map_err(|_| format!("expected Bytes or a String, got {}", other.type_name())),
    }
}

// ── the builder ─────────────────────────────────────────────────────

/// `bytes.builder()`: a buffer that grows in place. It is a mutable
/// location, so like a cell it belongs to the thread that made it.
struct BuilderObject {
    owner: std::thread::ThreadId,
    buf: std::sync::Mutex<Vec<u8>>,
    _live: crate::native::ConfinedLive,
}

impl std::fmt::Debug for BuilderObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<bytes builder>")
    }
}

impl NativeObject for BuilderObject {
    fn module(&self) -> &'static str {
        "bytes"
    }
    fn type_name(&self) -> &'static str {
        "BytesBuilder"
    }
    fn display(&self) -> String {
        match self.buf.try_lock() {
            Ok(b) => format!("<bytes builder: {} bytes>", b.len()),
            Err(_) => "<bytes builder>".to_string(),
        }
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<BuilderObject>()
            .is_some_and(|o| std::ptr::eq(self, o))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn confined_to(&self) -> Option<std::thread::ThreadId> {
        Some(self.owner)
    }
    fn length(&self) -> Option<usize> {
        if std::thread::current().id() != self.owner {
            return None;
        }
        self.buf.lock().ok().map(|b| b.len())
    }
}

fn builder_of<'a>(v: &'a Value, who: &str) -> Result<&'a BuilderObject, String> {
    let b = match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<BuilderObject>(),
        _ => None,
    }
    .ok_or_else(|| {
        format!(
            "bytes.{who}: expected a builder (from bytes.builder()), got {}",
            match v {
                Value::Native(h) => h.0.type_name().to_string(),
                other => other.type_name(),
            }
        )
    })?;
    if std::thread::current().id() != b.owner {
        return Err(format!(
            "bytes.{who}: a builder belongs to the thread that made it"
        ));
    }
    Ok(b)
}

fn builder(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("bytes.builder expects no arguments, got {}", args.len()).into());
    }
    Ok(Value::Native(NativeHandle::new(BuilderObject {
        owner: std::thread::current().id(),
        buf: std::sync::Mutex::new(Vec::new()),
        _live: crate::native::ConfinedLive::new(),
    })))
}

/// `bytes.push(builder, x)`: append Bytes, a String's UTF-8, one byte
/// (an Int 0..=255) or a list of them, in place. Answers the builder.
fn push(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let [bld, x] = args.as_slice() else {
        return Err(format!(
            "bytes.push expects a builder and what to append, got {} arguments",
            args.len()
        )
        .into());
    };
    let b = builder_of(bld, "push")?;
    let mut buf = b
        .buf
        .lock()
        .map_err(|_| "bytes.push: the builder is unusable")?;
    match x {
        Value::Integer(n) if (0..=255).contains(n) => buf.push(*n as u8),
        Value::Integer(n) => {
            return Err(format!("bytes.push: {n} is outside a byte's range 0..=255").into());
        }
        Value::List(items) => {
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::Integer(n) if (0..=255).contains(n) => buf.push(*n as u8),
                    other => {
                        return Err(format!(
                            "bytes.push: element {i} must be an Int 0..=255, got {}",
                            other
                        )
                        .into());
                    }
                }
            }
        }
        other => buf.extend_from_slice(piece(other).map_err(|e| format!("bytes.push: {e}"))?),
    }
    drop(buf);
    Ok(bld.clone())
}

/// `bytes.build(builder)`: what it holds, as Bytes; the builder is empty
/// afterwards. Moves the buffer: no copy.
fn build(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let [bld] = args.as_slice() else {
        return Err(format!(
            "bytes.build expects a builder, got {} arguments",
            args.len()
        )
        .into());
    };
    let b = builder_of(bld, "build")?;
    let mut buf = b
        .buf
        .lock()
        .map_err(|_| "bytes.build: the builder is unusable")?;
    Ok(to_value(std::mem::take(&mut *buf)))
}

// ── searching and comparing ─────────────────────────────────────────

/// `bytes.index_of(b, needle, from)`: where `needle` (Bytes, a String's
/// UTF-8, or one byte) first occurs at or after `from`, or `()`.
fn index_of(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let (hay, needle, from) = match args.as_slice() {
        [h, n] => (h, n, 0i64),
        [h, n, Value::Integer(f)] => (h, n, *f),
        [_, _, other] => {
            return Err(format!(
                "bytes.index_of: from must be an Int, got {}",
                other.type_name()
            )
            .into());
        }
        _ => {
            return Err(format!(
                "bytes.index_of expects 2 or 3 arguments, got {}",
                args.len()
            )
            .into());
        }
    };
    let hay = bytes_of(hay).map_err(|e| format!("bytes.index_of: {e}"))?;
    if from < 0 {
        return Err(format!("bytes.index_of: position {from} is negative").into());
    }
    let from = (from as usize).min(hay.len());
    let found = match needle {
        Value::Integer(n) if (0..=255).contains(n) => memchr::memchr(*n as u8, &hay[from..]),
        Value::Integer(n) => {
            return Err(format!("bytes.index_of: {n} is outside a byte's range 0..=255").into());
        }
        other => {
            let n = piece(other).map_err(|e| format!("bytes.index_of: {e}"))?;
            memchr::memmem::find(&hay[from..], n)
        }
    };
    Ok(found
        .map(|i| Value::Integer((from + i) as i64))
        .unwrap_or(Value::Unit))
}

/// `bytes.compare(a, b)`: -1, 0 or 1, byte by byte (a prefix first).
fn compare(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let [a, b] = args.as_slice() else {
        return Err(format!("bytes.compare expects 2 arguments, got {}", args.len()).into());
    };
    let a = bytes_of(a).map_err(|e| format!("bytes.compare: {e}"))?;
    let b = bytes_of(b).map_err(|e| format!("bytes.compare: {e}"))?;
    Ok(Value::Integer(match a.cmp(b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

// ── typed reads, pack and unpack ────────────────────────────────────

#[derive(Clone, Copy)]
enum Scalar {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
}

impl Scalar {
    fn named(name: &str) -> Option<Scalar> {
        Some(match name {
            "u8" => Scalar::U8,
            "i8" => Scalar::I8,
            "u16" => Scalar::U16,
            "i16" => Scalar::I16,
            "u32" => Scalar::U32,
            "i32" => Scalar::I32,
            "u64" => Scalar::U64,
            "i64" => Scalar::I64,
            "f32" => Scalar::F32,
            "f64" => Scalar::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Scalar::U8 | Scalar::I8 => 1,
            Scalar::U16 | Scalar::I16 => 2,
            Scalar::U32 | Scalar::I32 | Scalar::F32 => 4,
            Scalar::U64 | Scalar::I64 | Scalar::F64 => 8,
        }
    }

    /// Read one from `b` (exactly `size` bytes).
    fn read(self, b: &[u8], big: bool) -> Value {
        let mut a = [0u8; 8];
        a[..b.len()].copy_from_slice(b);
        if big {
            a[..b.len()].reverse();
        }
        let u = u64::from_le_bytes(a);
        match self {
            Scalar::U8 => Value::Integer(u as u8 as i64),
            Scalar::I8 => Value::Integer(u as u8 as i8 as i64),
            Scalar::U16 => Value::Integer(u as u16 as i64),
            Scalar::I16 => Value::Integer(u as u16 as i16 as i64),
            Scalar::U32 => Value::Integer(u as u32 as i64),
            Scalar::I32 => Value::Integer(u as u32 as i32 as i64),
            // the 64 bits as an Int: a u64 past i64::MAX reads negative
            Scalar::U64 | Scalar::I64 => Value::Integer(u as i64),
            Scalar::F32 => Value::Float(f32::from_bits(u as u32) as f64),
            Scalar::F64 => Value::Float(f64::from_bits(u)),
        }
    }

    /// Append `v` to `out`, or say why it does not fit.
    fn write(self, v: &Value, big: bool, out: &mut Vec<u8>) -> Result<(), String> {
        let bits: u64 = match (self, v) {
            (Scalar::F32, Value::Integer(n)) => (*n as f32).to_bits() as u64,
            (Scalar::F32, Value::Float(x)) => (*x as f32).to_bits() as u64,
            (Scalar::F64, Value::Integer(n)) => (*n as f64).to_bits(),
            (Scalar::F64, Value::Float(x)) => x.to_bits(),
            (_, Value::Integer(n)) => {
                let n = *n;
                let fits = match self {
                    Scalar::U8 => (0..=u8::MAX as i64).contains(&n),
                    Scalar::I8 => (i8::MIN as i64..=i8::MAX as i64).contains(&n),
                    Scalar::U16 => (0..=u16::MAX as i64).contains(&n),
                    Scalar::I16 => (i16::MIN as i64..=i16::MAX as i64).contains(&n),
                    Scalar::U32 => (0..=u32::MAX as i64).contains(&n),
                    Scalar::I32 => (i32::MIN as i64..=i32::MAX as i64).contains(&n),
                    _ => true,
                };
                if !fits {
                    return Err(format!("{n} does not fit in a {}", self.name()));
                }
                n as u64
            }
            (_, other) => {
                return Err(format!(
                    "a {} takes an Int, got {}",
                    self.name(),
                    other.type_name()
                ));
            }
        };
        let le = bits.to_le_bytes();
        let n = self.size();
        if big {
            out.extend(le[..n].iter().rev());
        } else {
            out.extend_from_slice(&le[..n]);
        }
        Ok(())
    }

    fn name(self) -> &'static str {
        match self {
            Scalar::U8 => "u8",
            Scalar::I8 => "i8",
            Scalar::U16 => "u16",
            Scalar::I16 => "i16",
            Scalar::U32 => "u32",
            Scalar::I32 => "i32",
            Scalar::U64 => "u64",
            Scalar::I64 => "i64",
            Scalar::F32 => "f32",
            Scalar::F64 => "f64",
        }
    }
}

/// The byte order an argument names: "le" (the default) or "be".
fn order(v: Option<&Value>, who: &str) -> Result<bool, String> {
    match v {
        None => Ok(false),
        Some(Value::String(s)) if s.as_str() == "le" => Ok(false),
        Some(Value::String(s)) if s.as_str() == "be" => Ok(true),
        Some(other) => Err(format!(
            "{who}: the byte order is \"le\" or \"be\", got {other}"
        )),
    }
}

/// `bytes.get_u16(b, at)` and its kin: the number at byte `at`, little
/// endian unless "be" is given.
fn get_scalar(kind: &str, args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let who = format!("bytes.get_{kind}");
    let ty = Scalar::named(kind).ok_or_else(|| format!("Unknown bytes function: get_{kind}"))?;
    let (b, at, ord) = match args.as_slice() {
        [b, Value::Integer(at)] => (b, *at, None),
        [b, Value::Integer(at), o] if ty.size() > 1 => (b, *at, Some(o)),
        _ => {
            return Err(format!(
                "{who} expects Bytes and a position{}",
                if ty.size() > 1 {
                    ", and optionally \"le\" or \"be\""
                } else {
                    ""
                }
            )
            .into());
        }
    };
    let big = order(ord, &who)?;
    let s = bytes_of(b).map_err(|e| format!("{who}: {e}"))?;
    let n = ty.size();
    if at < 0 || at as usize + n > s.len() {
        return Err(format!(
            "{who}: {n} bytes at {at} run past the end of {} bytes",
            s.len()
        )
        .into());
    }
    Ok(ty.read(&s[at as usize..at as usize + n], big))
}

/// A layout: whitespace-separated types, each `u8` … `f64` with an
/// optional `be` or `le` suffix (little endian by default).
fn layout(fmt: &Value, who: &str) -> Result<Vec<(Scalar, bool)>, String> {
    let Value::String(f) = fmt else {
        return Err(format!(
            "{who}: the layout must be a string such as \"u16 u32be f64\""
        ));
    };
    f.split_whitespace()
        .map(|t| {
            let (name, big) = if let Some(n) = t.strip_suffix("be") {
                (n, true)
            } else if let Some(n) = t.strip_suffix("le") {
                (n, false)
            } else {
                (t, false)
            };
            Scalar::named(name)
                .map(|s| (s, big))
                .ok_or_else(|| format!("{who}: unknown type {t:?} in the layout (u8, i8, u16, i16, u32, i32, u64, i64, f32 or f64, with be or le)"))
        })
        .collect()
}

/// `bytes.pack(layout, values)`: the values written one after another as
/// the layout's types.
fn pack(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let [fmt, Value::List(values)] = args.as_slice() else {
        return Err("bytes.pack expects a layout and a list of values".into());
    };
    let types = layout(fmt, "bytes.pack")?;
    if types.len() != values.len() {
        return Err(format!(
            "bytes.pack: the layout has {} types and the list {} values",
            types.len(),
            values.len()
        )
        .into());
    }
    let mut out = Vec::with_capacity(types.iter().map(|(t, _)| t.size()).sum());
    for (i, ((ty, big), v)) in types.iter().zip(values.iter()).enumerate() {
        ty.write(v, *big, &mut out)
            .map_err(|e| format!("bytes.pack: value {i}: {e}"))?;
    }
    Ok(to_value(out))
}

/// `bytes.unpack(layout, b, at)`: the values the layout's types read
/// from `b` from byte `at` (0 when left out), as a list.
fn unpack(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let (fmt, b, at) = match args.as_slice() {
        [f, b] => (f, b, 0i64),
        [f, b, Value::Integer(at)] => (f, b, *at),
        _ => return Err("bytes.unpack expects a layout, Bytes and optionally a position".into()),
    };
    let types = layout(fmt, "bytes.unpack")?;
    let s = bytes_of(b).map_err(|e| format!("bytes.unpack: {e}"))?;
    let need: usize = types.iter().map(|(t, _)| t.size()).sum();
    if at < 0 || at as usize + need > s.len() {
        return Err(format!(
            "bytes.unpack: the layout reads {need} bytes at {at}, past the end of {} bytes",
            s.len()
        )
        .into());
    }
    let mut pos = at as usize;
    let mut out = Vec::with_capacity(types.len());
    for (ty, big) in types {
        out.push(ty.read(&s[pos..pos + ty.size()], big));
        pos += ty.size();
    }
    Ok(Value::List(Arc::new(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: &[u8]) -> Value {
        to_value(v.to_vec())
    }

    #[test]
    fn pack_and_unpack_round_trip_both_orders() {
        let fmt = Value::String(Arc::new("u8 i16 u32be i64 f32 f64be".into()));
        let vals = Value::List(Arc::new(vec![
            Value::Integer(255),
            Value::Integer(-2),
            Value::Integer(0x0102_0304),
            Value::Integer(i64::MIN),
            Value::Float(1.5),
            Value::Float(-0.25),
        ]));
        let packed = pack(vec![fmt.clone(), vals.clone()]).unwrap();
        assert_eq!(bytes_of(&packed).unwrap().len(), 1 + 2 + 4 + 8 + 4 + 8);
        assert_eq!(&bytes_of(&packed).unwrap()[3..7], &[1, 2, 3, 4]);
        assert_eq!(unpack(vec![fmt, packed]).unwrap(), vals);
    }

    #[test]
    fn a_slice_is_a_view_of_its_source() {
        let src = b(&[1, 2, 3, 4, 5]);
        let s = slice(vec![src.clone(), Value::Integer(1), Value::Integer(4)]).unwrap();
        assert_eq!(bytes_of(&s).unwrap(), &[2, 3, 4]);
        let inner = slice(vec![s, Value::Integer(1), Value::Integer(9)]).unwrap();
        assert_eq!(bytes_of(&inner).unwrap(), &[3, 4]);
        let (Value::Native(a), Value::Native(c)) = (&src, &inner) else {
            unreachable!()
        };
        let pa =
            a.0.as_any()
                .downcast_ref::<BytesObject>()
                .unwrap()
                .as_slice()
                .as_ptr();
        let pc =
            c.0.as_any()
                .downcast_ref::<BytesObject>()
                .unwrap()
                .as_slice()
                .as_ptr();
        assert_eq!(unsafe { pa.add(2) }, pc, "no copy");
    }
}

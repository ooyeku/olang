//! The native columnar file format: a text header over binary columns.
//!
//! CSV and JSON lines are interchange with the outside world, and both
//! pay for it — every load re-parses text and re-infers types, and
//! neither can record that a column was Int rather than Float. This
//! format is what a Frame writes when the reader is going to be olang:
//! the types survive, the load is a read rather than a parse, and a
//! single column can be fetched without touching the others.
//!
//! Arrow and Parquet were the alternatives and were declined. Both would
//! be a dependency, and both are opaque — you cannot look at the file and
//! see what is in it. That is the wrong trade for a project whose case is
//! that the artifact should be inspectable, so the header here is UTF-8
//! text, one line per column, readable with `head`:
//!
//! ```text
//! olang-columns 1
//! rows 3
//! columns 3
//! col "region" String enc=dict nulls=0 bytes=42
//! col "amount" Float enc=plain nulls=1 bytes=32
//! col "qty" Int enc=plain nulls=0 bytes=24
//! data
//! <binary payload>
//! ```
//!
//! The header carries byte *lengths* rather than offsets, so a column's
//! position is the sum of those before it. That keeps the header
//! writable in one pass and, more importantly, keeps it truthful: an
//! offset table can disagree with the payload, a length table read in
//! order cannot drift without the total failing to match.
//!
//! ## Column encoding
//!
//! Each column is, in order: a validity bitmap when and only when the
//! header says `nulls` is non-zero; then the values.
//!
//! | dtype | values |
//! |---|---|
//! | `Int` | `rows` × i64, little-endian |
//! | `Float` | `rows` × f64, little-endian |
//! | `Bool` | bitmap of `rows` bits |
//! | `String`, `enc=plain` | `rows + 1` × u64 offsets, then the UTF-8 blob |
//! | `String`, `enc=dict` | u64 entry count, `entries + 1` × u64 offsets, the blob, then `rows` × u32 codes |
//!
//! A String column is written whichever of those two ways is smaller,
//! and the header says which. Repetition is the normal case in a
//! table — a `region` column is four distinct values across a million
//! rows — and storing each row's text separately makes the file *larger*
//! than the CSV it came from, because a u64 offset costs more than the
//! four characters it points at. The choice is made by comparing the two
//! sizes arithmetically rather than by a cardinality threshold, so it is
//! exact and there is no tuning constant to be wrong.
//!
//! The bitmap is LSB-first, one bit per row, `1` meaning present — the
//! same convention Arrow uses, chosen so that anyone who has read one
//! format can read this one. A null slot still occupies its fixed-width
//! value position, holding zero; that costs eight bytes per null and buys
//! random access by row index, which is the whole point of a column.

use super::series::series_of;
use crate::ast::Value;
use olang_ods::{DType, Frame, Scalar, Series};
use std::collections::HashMap;

const MAGIC: &str = "olang-columns";
const VERSION: u32 = 1;

// ── writing ───────────────────────────────────────────────────────────

/// Set bit `i` of a validity bitmap sized for `rows`.
fn bitmap(rows: usize, present: impl Fn(usize) -> bool) -> Vec<u8> {
    let mut bits = vec![0u8; rows.div_ceil(8)];
    for i in 0..rows {
        if present(i) {
            bits[i / 8] |= 1 << (i % 8);
        }
    }
    bits
}

/// How a column's values are laid out. Recorded per column in the
/// header, so a file describes its own encoding and a later version can
/// add one without the reader guessing.
#[derive(Clone, Copy, PartialEq)]
pub enum Encoding {
    Plain,
    Dict,
}

impl Encoding {
    fn name(self) -> &'static str {
        match self {
            Encoding::Plain => "plain",
            Encoding::Dict => "dict",
        }
    }

    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "plain" => Ok(Encoding::Plain),
            "dict" => Ok(Encoding::Dict),
            other => Err(bad(format!("unknown column encoding '{}'", other))),
        }
    }
}

/// The strings of a String column, in row order, with `None` for null.
fn string_values(series: &Series) -> Vec<Option<String>> {
    (0..series.len())
        .map(|i| match series.scalar_at(i) {
            Scalar::Str(s) => Some(s.to_string()),
            _ => None,
        })
        .collect()
}

/// Which of the two String layouts is smaller for these values. Both
/// sizes are computed in closed form so only the winner is ever built.
fn choose_encoding(values: &[Option<String>]) -> Encoding {
    let rows = values.len();
    let mut distinct: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut total = 0usize;
    for value in values.iter().flatten() {
        if distinct.insert(value.as_str()) {
            total += value.len();
        }
    }
    let blob: usize = values.iter().flatten().map(|v| v.len()).sum();
    let plain = 8 * (rows + 1) + blob;
    let dict = 8 + 8 * (distinct.len() + 1) + total + 4 * rows;
    if dict < plain {
        Encoding::Dict
    } else {
        Encoding::Plain
    }
}

/// One column's bytes, its encoding, and how many of its rows were null.
fn encode_column(series: &Series) -> (Vec<u8>, Encoding, usize) {
    let rows = series.len();
    let nulls = series.null_count();
    let mut encoding = Encoding::Plain;
    let mut out = Vec::new();
    if nulls > 0 {
        out.extend_from_slice(&bitmap(rows, |i| {
            !matches!(series.scalar_at(i), Scalar::Null)
        }));
    }
    match series.dtype() {
        DType::I64 => {
            for i in 0..rows {
                let v = match series.scalar_at(i) {
                    Scalar::I64(x) => x,
                    _ => 0,
                };
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        DType::F64 => {
            for i in 0..rows {
                let v = match series.scalar_at(i) {
                    Scalar::F64(x) => x,
                    _ => 0.0,
                };
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        DType::Bool => {
            out.extend_from_slice(&bitmap(rows, |i| {
                matches!(series.scalar_at(i), Scalar::Bool(true))
            }));
        }
        DType::Str => {
            let values = string_values(series);
            encoding = choose_encoding(&values);
            match encoding {
                // Offsets first so the blob can be read in one go, and so
                // any row is reachable without scanning those before it.
                Encoding::Plain => {
                    let mut blob = Vec::new();
                    let mut offsets = Vec::with_capacity((rows + 1) * 8);
                    offsets.extend_from_slice(&0u64.to_le_bytes());
                    for value in &values {
                        if let Some(text) = value {
                            blob.extend_from_slice(text.as_bytes());
                        }
                        offsets.extend_from_slice(&(blob.len() as u64).to_le_bytes());
                    }
                    out.extend_from_slice(&offsets);
                    out.extend_from_slice(&blob);
                }
                // Each distinct string once, then one small code per row.
                Encoding::Dict => {
                    let mut codes: Vec<u32> = Vec::with_capacity(rows);
                    let mut index: HashMap<&str, u32> = HashMap::new();
                    let mut entries: Vec<&str> = Vec::new();
                    for value in &values {
                        let text = value.as_deref().unwrap_or("");
                        let code = match index.get(text) {
                            Some(code) => *code,
                            None => {
                                let code = entries.len() as u32;
                                index.insert(text, code);
                                entries.push(text);
                                code
                            }
                        };
                        codes.push(code);
                    }
                    let mut blob = Vec::new();
                    let mut offsets = Vec::with_capacity((entries.len() + 1) * 8);
                    offsets.extend_from_slice(&0u64.to_le_bytes());
                    for entry in &entries {
                        blob.extend_from_slice(entry.as_bytes());
                        offsets.extend_from_slice(&(blob.len() as u64).to_le_bytes());
                    }
                    out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
                    out.extend_from_slice(&offsets);
                    out.extend_from_slice(&blob);
                    for code in codes {
                        out.extend_from_slice(&code.to_le_bytes());
                    }
                }
            }
        }
    }
    (out, encoding, nulls)
}

/// The whole file: header text, then every column's bytes in order.
pub fn encode(frame: &Frame) -> Vec<u8> {
    let mut blocks = Vec::with_capacity(frame.n_cols());
    for series in frame.columns() {
        blocks.push(encode_column(series));
    }
    let mut header = format!(
        "{} {}\nrows {}\ncolumns {}\n",
        MAGIC,
        VERSION,
        frame.n_rows(),
        frame.n_cols()
    );
    for ((name, series), (bytes, encoding, nulls)) in
        frame.names().iter().zip(frame.columns()).zip(&blocks)
    {
        header.push_str(&format!(
            "col {} {} enc={} nulls={} bytes={}\n",
            // JSON-quoted so that a column name holding a space, a quote,
            // or a newline cannot break the line-oriented header.
            serde_json::Value::String(name.clone()),
            series.dtype(),
            encoding.name(),
            nulls,
            bytes.len()
        ));
    }
    header.push_str("data\n");

    let mut out = header.into_bytes();
    for (bytes, _, _) in blocks {
        out.extend_from_slice(&bytes);
    }
    out
}

// ── reading ───────────────────────────────────────────────────────────

/// What the header says about one column.
#[derive(Clone)]
pub struct ColumnSpec {
    pub name: String,
    pub dtype: DType,
    pub encoding: Encoding,
    pub nulls: usize,
    pub bytes: usize,
}

pub struct Header {
    pub rows: usize,
    pub columns: Vec<ColumnSpec>,
    /// Where the payload starts, in bytes from the beginning of the file.
    pub data_at: usize,
}

fn bad(what: impl std::fmt::Display) -> String {
    format!("not a readable olang-columns file: {}", what)
}

/// Read the JSON string that starts at the head of `rest`, returning it
/// and whatever follows.
fn quoted(rest: &str) -> Result<(String, &str), String> {
    let body = rest
        .strip_prefix('"')
        .ok_or_else(|| bad("expected a quoted column name"))?;
    let mut escaped = false;
    for (i, ch) in body.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            let literal = &rest[..=i + 1];
            let name: String = serde_json::from_str(literal)
                .map_err(|err| bad(format!("column name: {}", err)))?;
            return Ok((name, &rest[i + 2..]));
        }
    }
    Err(bad("unterminated column name"))
}

fn dtype_of(name: &str) -> Result<DType, String> {
    match name {
        "Int" => Ok(DType::I64),
        "Float" => Ok(DType::F64),
        "Bool" => Ok(DType::Bool),
        "String" => Ok(DType::Str),
        other => Err(bad(format!("unknown column type '{}'", other))),
    }
}

fn field(token: Option<&str>, key: &str) -> Result<usize, String> {
    token
        .and_then(|t| t.strip_prefix(key))
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| bad(format!("expected {}<number>", key)))
}

/// Parse the text header. Reads no payload, so it is what `frame_info`
/// uses to describe a file without loading it.
pub fn parse_header(bytes: &[u8]) -> Result<Header, String> {
    // The header is ASCII text; find its end without requiring the whole
    // file to be valid UTF-8, since everything after `data` is binary.
    let marker = b"\ndata\n";
    let end = bytes
        .windows(marker.len())
        .position(|w| w == marker)
        .ok_or_else(|| bad("no 'data' marker"))?
        + marker.len();
    let text = std::str::from_utf8(&bytes[..end]).map_err(|_| bad("header is not text"))?;

    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default();
    let version = first
        .strip_prefix(MAGIC)
        .map(str::trim)
        .ok_or_else(|| bad(format!("expected it to begin with '{}'", MAGIC)))?;
    // A version this build does not know is a clear message rather than a
    // misparse of a layout that has moved.
    if version.parse::<u32>().ok() != Some(VERSION) {
        return Err(bad(format!(
            "version {} — this build reads version {}",
            version, VERSION
        )));
    }

    let count = |line: Option<&str>, key: &str| -> Result<usize, String> {
        line.and_then(|l| l.strip_prefix(key))
            .and_then(|v| v.trim().parse().ok())
            .ok_or_else(|| bad(format!("expected a '{}' line", key.trim())))
    };
    let rows = count(lines.next(), "rows ")?;
    let n_cols = count(lines.next(), "columns ")?;

    let mut columns = Vec::with_capacity(n_cols);
    for _ in 0..n_cols {
        let line = lines.next().ok_or_else(|| bad("header ends early"))?;
        let rest = line
            .strip_prefix("col ")
            .ok_or_else(|| bad(format!("expected a 'col' line, got '{}'", line)))?;
        let (name, rest) = quoted(rest)?;
        let mut parts = rest.split_whitespace();
        let dtype = dtype_of(parts.next().unwrap_or_default())?;
        let encoding = Encoding::parse(
            parts
                .next()
                .and_then(|t| t.strip_prefix("enc="))
                .ok_or_else(|| bad("expected enc=<encoding>"))?,
        )?;
        let nulls = field(parts.next(), "nulls=")?;
        let bytes = field(parts.next(), "bytes=")?;
        if nulls > rows {
            return Err(bad(format!(
                "column '{}' claims {} nulls in {} rows",
                name, nulls, rows
            )));
        }
        columns.push(ColumnSpec {
            name,
            dtype,
            encoding,
            nulls,
            bytes,
        });
    }
    Ok(Header {
        rows,
        columns,
        data_at: end,
    })
}

fn present_at(bits: &[u8], i: usize) -> bool {
    bits.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0)
}

fn take<const N: usize>(block: &[u8], at: usize) -> Result<[u8; N], String> {
    block
        .get(at..at + N)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| bad("a column block is shorter than its header says"))
}

fn decode_column(spec: &ColumnSpec, rows: usize, block: &[u8]) -> Result<Series, String> {
    let (valid, values) = if spec.nulls > 0 {
        let n = rows.div_ceil(8);
        if block.len() < n {
            return Err(bad("a validity bitmap is truncated"));
        }
        (Some(&block[..n]), &block[n..])
    } else {
        (None, block)
    };
    let present = |i: usize| valid.is_none_or(|bits| present_at(bits, i));

    Ok(match spec.dtype {
        DType::I64 => {
            let mut out = Vec::with_capacity(rows);
            for i in 0..rows {
                let raw = i64::from_le_bytes(take::<8>(values, i * 8)?);
                out.push(present(i).then_some(raw));
            }
            Series::from_i64_options(out)
        }
        DType::F64 => {
            let mut out = Vec::with_capacity(rows);
            for i in 0..rows {
                let raw = f64::from_le_bytes(take::<8>(values, i * 8)?);
                out.push(present(i).then_some(raw));
            }
            Series::from_f64_options(out)
        }
        DType::Bool => {
            if values.len() < rows.div_ceil(8) {
                return Err(bad("a boolean column is truncated"));
            }
            let mut out = Vec::with_capacity(rows);
            for i in 0..rows {
                out.push(present(i).then(|| present_at(values, i)));
            }
            Series::from_bool_options(out)
        }
        DType::Str => {
            // Both layouts are an offset table over a blob; they differ
            // in how many entries the table has and in what maps a row to
            // one, so the slicing is shared and only that map differs.
            let (entries, table_at, code_rows) = match spec.encoding {
                Encoding::Plain => (rows, 0, None),
                Encoding::Dict => {
                    let count = u64::from_le_bytes(take::<8>(values, 0)?) as usize;
                    (count, 8, Some(()))
                }
            };
            let table = (entries + 1) * 8;
            let codes_at = table_at + table;
            if values.len() < codes_at {
                return Err(bad("a string column's offsets are truncated"));
            }
            let blob_end = values.len() - if code_rows.is_some() { rows * 4 } else { 0 };
            if blob_end < codes_at {
                return Err(bad("a dictionary column's codes are truncated"));
            }
            let blob = &values[codes_at..blob_end];
            let offset = |i: usize| -> Result<usize, String> {
                Ok(u64::from_le_bytes(take::<8>(values, table_at + i * 8)?) as usize)
            };
            let entry = |i: usize| -> Result<String, String> {
                let (start, end) = (offset(i)?, offset(i + 1)?);
                if end < start || end > blob.len() {
                    return Err(bad("a string offset points outside the blob"));
                }
                Ok(std::str::from_utf8(&blob[start..end])
                    .map_err(|_| bad("a string column holds invalid UTF-8"))?
                    .to_string())
            };
            let mut out = Vec::with_capacity(rows);
            for i in 0..rows {
                if !present(i) {
                    out.push(None);
                    continue;
                }
                let slot = match spec.encoding {
                    Encoding::Plain => i,
                    Encoding::Dict => {
                        let code =
                            u32::from_le_bytes(take::<4>(values, blob_end + i * 4)?) as usize;
                        if code >= entries {
                            return Err(bad("a dictionary code is out of range"));
                        }
                        code
                    }
                };
                out.push(Some(entry(slot)?));
            }
            Series::from_str_options(out)
        }
    })
}

/// Decode a file. `wanted` of `None` reads every column; otherwise only
/// the named ones are decoded — the rest are skipped over by the byte
/// lengths in the header, which is what a columnar layout is *for*.
pub fn decode(bytes: &[u8], wanted: Option<&[String]>) -> Result<Frame, String> {
    let header = parse_header(bytes)?;
    let payload = &bytes[header.data_at..];

    if let Some(names) = wanted {
        let known: Vec<&str> = header.columns.iter().map(|c| c.name.as_str()).collect();
        if let Some(missing) = names.iter().find(|n| !known.contains(&n.as_str())) {
            return Err(format!(
                "no column '{}' in this file. It has: {}",
                missing,
                known.join(", ")
            ));
        }
    }

    // Walk every column so the offsets stay right, but decode only the
    // ones asked for.
    let mut at = 0usize;
    let mut decoded: Vec<(String, Series)> = Vec::new();
    for spec in &header.columns {
        let end = at + spec.bytes;
        let block = payload.get(at..end).ok_or_else(|| {
            bad(format!(
                "column '{}' runs past the end of the file",
                spec.name
            ))
        })?;
        let take_it = wanted.is_none_or(|names| names.contains(&spec.name));
        if take_it {
            decoded.push((spec.name.clone(), decode_column(spec, header.rows, block)?));
        }
        at = end;
    }
    if at != payload.len() {
        return Err(bad(format!(
            "{} trailing bytes after the last column",
            payload.len() - at
        )));
    }

    // Preserve the caller's requested order, not the file's.
    let pairs = match wanted {
        None => decoded,
        Some(names) => names
            .iter()
            .filter_map(|n| decoded.iter().find(|(name, _)| name == n).cloned())
            .collect(),
    };
    Frame::new(pairs).map_err(|err| err.to_string())
}

/// The schema as a Frame, read from the header alone.
pub fn info(bytes: &[u8]) -> Result<Frame, String> {
    let header = parse_header(bytes)?;
    let mut names = Vec::new();
    let mut dtypes = Vec::new();
    let mut nulls = Vec::new();
    let mut sizes = Vec::new();
    for spec in &header.columns {
        names.push(Some(spec.name.clone()));
        dtypes.push(Some(spec.dtype.to_string()));
        nulls.push(Some(spec.nulls as i64));
        sizes.push(Some(spec.bytes as i64));
    }
    Frame::new(vec![
        ("column".to_string(), Series::from_str_options(names)),
        ("dtype".to_string(), Series::from_str_options(dtypes)),
        ("nulls".to_string(), Series::from_i64_options(nulls)),
        ("bytes".to_string(), Series::from_i64_options(sizes)),
    ])
    .map_err(|err| err.to_string())
}

/// The column list a caller passed to `read_frame`, if they passed one.
pub fn wanted_columns(value: Option<&Value>) -> Result<Option<Vec<String>>, String> {
    match value {
        None | Some(Value::Unit) => Ok(None),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(s) => Ok(s.as_ref().clone()),
                other => Err(format!(
                    "ods.read_frame: each column name must be a String, got {}",
                    other.type_name()
                )),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(other) => {
            // A bare Series would be a plausible mistake here.
            let hint = if series_of(other).is_some() {
                " (pass a list of names, not a column)"
            } else {
                ""
            };
            Err(format!(
                "ods.read_frame: the second argument is a list of column names, got {}{}",
                other.type_name(),
                hint
            ))
        }
    }
}

//! Frame: the language-facing face of the engine's columnar table.
//!
//! Verbs are pipeline-friendly (`df |> ods.filter(mask) |> ods.head(5)`),
//! and the bridges compose with the existing stdlib: `ods.read_csv`
//! takes CSV *text* (pair it with `fs.read_file`), and
//! `ods.frame_from_records` takes a list of maps — exactly what
//! `json.parse` produces for a JSON array of objects.

use super::series::{make_series_value, scalar_to_value, series_from_list, series_of};
use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use olang_ods::{AggOp, AggSpec, Frame, JoinHow, Scalar, Series};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub struct OdsFrame(pub Frame);

impl OdsFrame {
    pub fn into_value(frame: Frame) -> Value {
        Value::Native(NativeHandle::new(OdsFrame(frame)))
    }
}

impl NativeObject for OdsFrame {
    fn module(&self) -> &'static str {
        "ods"
    }

    fn type_name(&self) -> &'static str {
        "Frame"
    }

    fn display(&self) -> String {
        super::table::render(&self.0)
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other.as_any().downcast_ref::<OdsFrame>().is_some_and(|o| {
            self.0.names() == o.0.names()
                && self
                    .0
                    .columns()
                    .iter()
                    .zip(o.0.columns())
                    .all(|(a, b)| a.series_eq(b))
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `f["amount"]` is the column; `f[mask]` is the rows the mask keeps.
    ///
    /// Two meanings, disjoint by the key's type, which is what keeps this
    /// from being pandas' `df[x]` — there, one subscript means column for
    /// a string, row filter for a mask, positional for a slice, and an
    /// error for an integer, which is why `.loc` and `.iloc` had to be
    /// invented. Here an integer is refused outright and named its verb.
    fn index(&self, key: &Value) -> Option<Result<Value, String>> {
        Some(match key {
            Value::String(name) => match self.0.column(name) {
                Ok(col) => Ok(make_series_value(col.clone())),
                // A column name written into the source that is not there
                // is a mistake in the program, not input the caller chose,
                // so it raises — and it says what is available, which is
                // the fact the author needs.
                Err(_) => Err(format!(
                    "no column '{}' in this Frame. It has: {}",
                    name,
                    self.0.names().join(", ")
                )),
            },
            Value::Native(_) => match super::series::series_of(key) {
                Some(mask) => self
                    .0
                    .filter(mask)
                    .map(OdsFrame::into_value)
                    .map_err(|err| err.to_string()),
                None => Err(format!(
                    "a Frame is indexed by a column name or a Bool mask, got a {}",
                    key.type_name()
                )),
            },
            Value::Integer(_) => Err(
                "a Frame is indexed by column, not by row position. For rows, \
                 use ods.head(f, n) or ods.take(f, indices)"
                    .to_string(),
            ),
            other => Err(format!(
                "a Frame is indexed by a column name or a Bool mask, got {}",
                other.type_name()
            )),
        })
    }
}

pub fn frame_of(value: &Value) -> Option<&Frame> {
    match value {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsFrame>().map(|f| &f.0),
        _ => None,
    }
}

/// (name, arity) of the frame functions in the `ods` namespace.
/// `filter` and `take` are shared with Series and dispatch by the type
/// of their first argument (handled in series.rs).
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("frame", 1),
    ("read_csv", 1),
    ("read_csv_file", 1),
    ("read_jsonl", 1),
    ("read_jsonl_file", 1),
    ("open_csv", 1),
    ("open_jsonl", 1),
    ("next_chunk", 2),
    ("rows_read", 1),
    ("at_end", 1),
    ("to_csv", 1),
    ("write_csv", 2),
    ("to_jsonl", 1),
    ("write_jsonl", 2),
    ("frame_from_records", 1),
    ("to_records", 1),
    ("columns", 1),
    ("column", 2),
    ("n_rows", 1),
    ("n_cols", 1),
    ("select", 2),
    ("with_column", 3),
    ("sort_by", 3),
    ("head", 2),
    ("describe", 1),
    ("schema", 1),
    ("group_by", 3),
    ("join", 4),
    ("join_left", 4),
];

fn want_frame<'a>(func: &str, args: &'a [Value], idx: usize) -> Result<&'a Frame, String> {
    args.get(idx).and_then(frame_of).ok_or_else(|| {
        format!(
            "ods.{}: argument {} must be a Frame, got {}",
            func,
            idx + 1,
            args.get(idx).map(|v| v.type_name()).unwrap_or_default()
        )
    })
}

fn want_string(func: &str, args: &[Value], idx: usize) -> Result<String, String> {
    match args.get(idx) {
        Some(Value::String(s)) => Ok(s.as_ref().clone()),
        other => Err(format!(
            "ods.{}: argument {} must be a String, got {}",
            func,
            idx + 1,
            other.map(|v| v.type_name()).unwrap_or_default()
        )),
    }
}

fn string_list(func: &str, args: &[Value], idx: usize) -> Result<Vec<String>, String> {
    match args.get(idx) {
        Some(Value::String(s)) => Ok(vec![s.as_ref().clone()]),
        Some(Value::List(items)) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.as_ref().clone()),
                other => Err(format!(
                    "ods.{}: expected column names as Strings, got {}",
                    func,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "ods.{}: argument {} must be a String or list of Strings, got {}",
            func,
            idx + 1,
            other.map(|v| v.type_name()).unwrap_or_default()
        )),
    }
}

fn e(err: olang_ods::OdsError) -> String {
    err.to_string()
}

/// Functions whose final declared argument may be omitted. `head` is the
/// one verb typed constantly at the REPL, where `ods.head(f)` is what a
/// hand reaches for; the default is stated here rather than buried in the
/// arm so the arity check and the default cannot drift apart.
const OPTIONAL_TAIL: &[(&str, usize)] = &[("head", DEFAULT_HEAD)];
const DEFAULT_HEAD: usize = 10;

pub fn dispatch(func: &str, mut args: Vec<Value>) -> Result<Value, String> {
    let expected = FUNCTIONS
        .iter()
        .find(|(n, _)| *n == func)
        .map(|(_, a)| *a)
        .expect("caller checked membership");
    if let Some((_, default)) = OPTIONAL_TAIL.iter().find(|(n, _)| *n == func)
        && args.len() + 1 == expected
    {
        args.push(Value::Integer(*default as i64));
    }
    if args.len() != expected {
        return Err(format!(
            "ods.{} expects {} argument{}, got {}",
            func,
            expected,
            if expected == 1 { "" } else { "s" },
            args.len()
        ));
    }
    match func {
        "frame" => {
            let pairs = match &args[0] {
                Value::List(items) => items
                    .iter()
                    .map(|item| match item {
                        Value::List(pair) if pair.len() == 2 => {
                            let name = match &pair[0] {
                                Value::String(s) => s.as_ref().clone(),
                                other => {
                                    return Err(format!(
                                        "ods.frame: column name must be a String, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let col = match series_of(&pair[1]) {
                                Some(s) => s.clone(),
                                None => match &pair[1] {
                                    Value::List(vals) => series_from_list(vals)?,
                                    other => {
                                        return Err(format!(
                                        "ods.frame: column value must be a Series or list, got {}",
                                        other.type_name()
                                    ))
                                    }
                                },
                            };
                            Ok((name, col))
                        }
                        other => Err(format!(
                            "ods.frame expects a list of [name, column] pairs, got element {}",
                            other.type_name()
                        )),
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                other => {
                    return Err(format!(
                        "ods.frame expects a list of [name, column] pairs, got {}",
                        other.type_name()
                    ))
                }
            };
            Frame::new(pairs).map(OdsFrame::into_value).map_err(e)
        }
        "read_csv" => {
            let text = want_string(func, &args, 0)?;
            // `read_csv` takes CSV *text*, so a path parses as a one-line
            // file: one column named after the path, zero rows, and no
            // error anywhere. Silence is the worst outcome here, and the
            // shape is unmistakable, so name the sibling that was meant.
            // The check stays a string test — probing the filesystem from
            // a function with no `fs` grant is the very hole the split
            // between these two functions exists to prevent.
            let trimmed = text.trim();
            if !text.contains('\n')
                && ["csv", "tsv", "txt"]
                    .iter()
                    .any(|ext| trimmed.to_lowercase().ends_with(&format!(".{}", ext)))
            {
                return Err(format!(
                    "ods.read_csv: {:?} looks like a path, but read_csv takes \
                     CSV text — use ods.read_csv_file({:?}) to read the file, \
                     or ods.open_csv({:?}) to stream it",
                    trimmed, trimmed, trimmed
                ));
            }
            read_csv(&text)
        }
        // The file-reading twin. Gated under `fs` (see caps::required):
        // without that, `ods` would be a latent filesystem capability, the
        // same hole `db.open` had before 0.60.
        "read_csv_file" => {
            let path = want_string(func, &args, 0)?;
            match std::fs::read_to_string(&path) {
                // A missing or unreadable file is a failure the caller can
                // handle, so it is a Result rather than a raise — the 0.64
                // rule. A malformed CSV *inside* a readable file is also a
                // Result, for the same reason: the caller chose the file.
                Err(err) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "ods.read_csv_file: {}: {}",
                    path, err
                )))))),
                Ok(text) => match read_csv(&text) {
                    Ok(frame) => Ok(Value::Ok(Box::new(frame))),
                    Err(msg) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                        "{} (reading {})",
                        msg, path
                    )))))),
                },
            }
        }
        "read_jsonl" => {
            let text = want_string(func, &args, 0)?;
            read_jsonl(&text)
        }
        "read_jsonl_file" => {
            let path = want_string(func, &args, 0)?;
            match std::fs::read_to_string(&path) {
                Err(err) => Ok(open_error("read_jsonl_file", &path, err)),
                Ok(text) => read_jsonl(&text),
            }
        }
        "open_csv" => {
            let path = want_string(func, &args, 0)?;
            open_csv(&path)
        }
        "open_jsonl" => {
            let path = want_string(func, &args, 0)?;
            open_jsonl(&path)
        }
        "next_chunk" => {
            let r = reader_of(func, &args)?;
            let n = match args.get(1) {
                Some(Value::Integer(n)) if *n >= 0 => *n as usize,
                _ => return Err(format!("ods.{}: argument 2 must be a row count", func)),
            };
            next_chunk(r, n)
        }
        "rows_read" => {
            let r = reader_of(func, &args)?;
            r.own_thread()?;
            let st = r
                .state
                .lock()
                .map_err(|_| "reader is unusable".to_string())?;
            Ok(Value::Integer(st.delivered as i64))
        }
        "at_end" => {
            let r = reader_of(func, &args)?;
            r.own_thread()?;
            let st = r
                .state
                .lock()
                .map_err(|_| "reader is unusable".to_string())?;
            Ok(Value::Boolean(st.done))
        }
        // Serialization cannot fail — every Scalar has a text form — so it
        // returns the string outright.
        "to_csv" => {
            let f = want_frame(func, &args, 0)?;
            Ok(Value::String(Arc::new(to_csv(f))))
        }
        "write_csv" => {
            let f = want_frame(func, &args, 0)?;
            let path = want_string(func, &args, 1)?;
            match std::fs::write(&path, to_csv(f)) {
                Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
                Err(err) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "ods.write_csv: {}: {}",
                    path, err
                )))))),
            }
        }
        "to_jsonl" => {
            let f = want_frame(func, &args, 0)?;
            Ok(Value::String(Arc::new(to_jsonl(f))))
        }
        "write_jsonl" => {
            let f = want_frame(func, &args, 0)?;
            let path = want_string(func, &args, 1)?;
            match std::fs::write(&path, to_jsonl(f)) {
                Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
                Err(err) => Ok(open_error("write_jsonl", &path, err)),
            }
        }
        "frame_from_records" => {
            let records = match &args[0] {
                Value::List(items) => items,
                other => {
                    return Err(format!(
                        "ods.frame_from_records expects a list of maps, got {}",
                        other.type_name()
                    ));
                }
            };
            frame_from_records(records)
        }
        "to_records" => {
            let f = want_frame(func, &args, 0)?;
            let mut out = Vec::with_capacity(f.n_rows());
            for row in 0..f.n_rows() {
                let mut m = HashMap::new();
                for (name, col) in f.names().iter().zip(f.columns()) {
                    m.insert(name.clone(), scalar_to_value(col.scalar_at(row)));
                }
                out.push(Value::Map(Arc::new(m)));
            }
            Ok(Value::List(out.into()))
        }
        "columns" => {
            let f = want_frame(func, &args, 0)?;
            Ok(Value::List(
                f.names()
                    .iter()
                    .map(|n| Value::String(Arc::new(n.clone())))
                    .collect::<Vec<_>>()
                    .into(),
            ))
        }
        "column" => {
            let f = want_frame(func, &args, 0)?;
            let name = want_string(func, &args, 1)?;
            f.column(&name)
                .map(|c| make_series_value(c.clone()))
                .map_err(e)
        }
        "n_rows" => Ok(Value::Integer(want_frame(func, &args, 0)?.n_rows() as i64)),
        "n_cols" => Ok(Value::Integer(want_frame(func, &args, 0)?.n_cols() as i64)),
        "select" => {
            let f = want_frame(func, &args, 0)?;
            let names = string_list(func, &args, 1)?;
            f.select(&names).map(OdsFrame::into_value).map_err(e)
        }
        "with_column" => {
            let f = want_frame(func, &args, 0)?;
            let name = want_string(func, &args, 1)?;
            let col = match series_of(&args[2]) {
                Some(s) => s.clone(),
                None => match &args[2] {
                    Value::List(vals) => series_from_list(vals)?,
                    other => {
                        return Err(format!(
                            "ods.with_column: column must be a Series or list, got {}",
                            other.type_name()
                        ));
                    }
                },
            };
            f.with_column(&name, col)
                .map(OdsFrame::into_value)
                .map_err(e)
        }
        "sort_by" => {
            let f = want_frame(func, &args, 0)?;
            let name = want_string(func, &args, 1)?;
            let descending = match &args[2] {
                Value::Boolean(b) => *b,
                other => {
                    return Err(format!(
                        "ods.sort_by: descending flag must be a Bool, got {}",
                        other.type_name()
                    ));
                }
            };
            f.sort_by(&name, descending)
                .map(OdsFrame::into_value)
                .map_err(e)
        }
        "head" => {
            let f = want_frame(func, &args, 0)?;
            let n = match &args[1] {
                Value::Integer(n) if *n >= 0 => *n as usize,
                other => {
                    return Err(format!(
                        "ods.head: n must be a non-negative Int, got {}",
                        other
                    ));
                }
            };
            Ok(OdsFrame::into_value(f.head(n)))
        }
        // Orientation: what is in this table, and what shape is it in.
        // Both return Frames rather than maps so they print as tables and
        // can themselves be sorted, filtered, and written out.
        "describe" => {
            let f = want_frame(func, &args, 0)?;
            describe(f)
        }
        "schema" => {
            let f = want_frame(func, &args, 0)?;
            schema(f)
        }
        "group_by" => {
            let f = want_frame(func, &args, 0)?;
            let keys = string_list(func, &args, 1)?;
            let aggs = parse_aggs(&args[2])?;
            f.group_by(&keys, &aggs)
                .map(OdsFrame::into_value)
                .map_err(e)
        }
        "join" | "join_left" => {
            let l = want_frame(func, &args, 0)?;
            let r = want_frame(func, &args, 1)?;
            let left_on = want_string(func, &args, 2)?;
            let right_on = want_string(func, &args, 3)?;
            let how = if func == "join" {
                JoinHow::Inner
            } else {
                JoinHow::Left
            };
            l.join(r, &left_on, &right_on, how)
                .map(OdsFrame::into_value)
                .map_err(e)
        }
        _ => unreachable!("caller checked membership"),
    }
}

/// Frame-typed overloads for the names shared with Series.
pub fn dispatch_shared(func: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let f = frame_of(args.first()?)?;
    match func {
        "filter" => Some(match args.get(1).and_then(series_of) {
            Some(mask) => f.filter(mask).map(OdsFrame::into_value).map_err(e),
            None => Err("ods.filter: mask must be a Bool Series".to_string()),
        }),
        "take" => Some(match args.get(1).and_then(series_of) {
            Some(idx) => f.take(idx).map(OdsFrame::into_value).map_err(e),
            None => Err("ods.take: indices must be an Int Series".to_string()),
        }),
        _ => None,
    }
}

/// Agg specs come as a list of `[out_name, op, col]` triples (or
/// `[out_name, "count"]` pairs, since count needs no column).
fn parse_aggs(value: &Value) -> Result<Vec<AggSpec>, String> {
    let items = match value {
        Value::List(items) => items,
        other => {
            return Err(format!(
                "ods.group_by: aggregations must be a list of [name, op, column] triples, got {}",
                other.type_name()
            ));
        }
    };
    items
        .iter()
        .map(|item| {
            let parts = match item {
                Value::List(parts) if parts.len() == 2 || parts.len() == 3 => parts,
                other => {
                    return Err(format!(
                        "ods.group_by: each aggregation is [name, op, column] (or [name, \"count\"]), got {}",
                        other.type_name()
                    ))
                }
            };
            let as_str = |v: &Value, what: &str| match v {
                Value::String(s) => Ok(s.as_ref().clone()),
                other => Err(format!(
                    "ods.group_by: {} must be a String, got {}",
                    what,
                    other.type_name()
                )),
            };
            let out_name = as_str(&parts[0], "aggregation name")?;
            let op_name = as_str(&parts[1], "aggregation op")?;
            let op = AggOp::parse(&op_name).ok_or_else(|| {
                format!(
                    "ods.group_by: unknown aggregation '{}' (count, sum, mean, min, max)",
                    op_name
                )
            })?;
            let col = if parts.len() == 3 {
                as_str(&parts[2], "aggregation column")?
            } else if op == AggOp::Count {
                String::new()
            } else {
                return Err(format!(
                    "ods.group_by: aggregation '{}' needs a column",
                    op_name
                ));
            };
            Ok(AggSpec { out_name, op, col })
        })
        .collect()
}

// ---------------------------------------------------------------------
// Bridges
// ---------------------------------------------------------------------

/// Parse CSV text (with a header row) into a Frame, inferring each
/// column's type: all-Int → Int, numeric → Float, true/false → Bool,
/// otherwise String. Empty cells are null.
fn read_csv(text: &str) -> Result<Value, String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|err| format!("ods.read_csv: {}", err))?
        .iter()
        .map(|h| h.to_string())
        .collect();
    let mut cells: Vec<Vec<String>> = vec![Vec::new(); headers.len()];
    for record in reader.records() {
        let record = record.map_err(|err| format!("ods.read_csv: {}", err))?;
        for (c, cell) in cells.iter_mut().enumerate() {
            cell.push(record.get(c).unwrap_or("").to_string());
        }
    }
    let pairs = headers
        .into_iter()
        .zip(cells)
        .map(|(name, raw)| (name, infer_column(raw)))
        .collect();
    Frame::new(pairs).map(OdsFrame::into_value).map_err(e)
}

/// Serialize a Frame as CSV with a header row.
///
/// Round-trips through `read_csv`: a null becomes an empty cell, which is
/// exactly what `read_csv` reads back as null, and floats use the same
/// `format_float` the rest of the language prints with, so a value shown
/// in a REPL and a value written to a file agree.
fn to_csv(f: &Frame) -> String {
    let mut w = csv::WriterBuilder::new().from_writer(Vec::new());
    // `csv` only fails here on an IO error from the sink, and the sink is
    // an in-memory Vec — so these writes cannot fail in practice. Errors
    // are still threaded rather than unwrapped, and collapse to whatever
    // was serialized before the (impossible) failure.
    let _ = w.write_record(f.names());
    let cols = f.columns();
    let mut record: Vec<String> = Vec::with_capacity(cols.len());
    for row in 0..f.n_rows() {
        record.clear();
        for col in cols {
            record.push(match col.scalar_at(row) {
                Scalar::Null => String::new(),
                Scalar::I64(v) => v.to_string(),
                Scalar::F64(v) => crate::ast::format_float(v),
                Scalar::Bool(v) => v.to_string(),
                Scalar::Str(v) => v,
            });
        }
        let _ = w.write_record(&record);
    }
    w.into_inner()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

fn infer_column(raw: Vec<String>) -> Series {
    let non_empty = || raw.iter().filter(|s| !s.is_empty());
    if non_empty().count() == 0 {
        return Series::from_str_options(vec![None; raw.len()]);
    }
    if non_empty().all(|s| s.parse::<i64>().is_ok()) {
        return Series::from_i64_options(raw.iter().map(|s| s.parse::<i64>().ok()).collect());
    }
    if non_empty().all(|s| s.parse::<f64>().is_ok()) {
        return Series::from_f64_options(raw.iter().map(|s| s.parse::<f64>().ok()).collect());
    }
    if non_empty().all(|s| s == "true" || s == "false") {
        return Series::from_bool_options(
            raw.iter()
                .map(|s| match s.as_str() {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                })
                .collect(),
        );
    }
    Series::from_str_options(
        raw.into_iter()
            .map(|s| if s.is_empty() { None } else { Some(s) })
            .collect(),
    )
}

/// A list of maps (what `json.parse` gives for an array of objects)
/// becomes a Frame: columns are the sorted union of keys, missing keys
/// are null.
fn frame_from_records(records: &[Value]) -> Result<Value, String> {
    records_to_frame(records).map(OdsFrame::into_value)
}

/// One JSON-lines line: a single JSON object, which is one row.
fn parse_jsonl_line(line: &str) -> Result<Value, String> {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(serde_json::Value::Object(obj)) => Ok(crate::stdlib::json::json_to_olang_value(
            serde_json::Value::Object(obj),
        )),
        // A row has named fields; a bare array or number has none, so
        // there is no honest column to put it in.
        Ok(other) => Err(format!(
            "each line must be a JSON object, got {}",
            match other {
                serde_json::Value::Array(_) => "an array",
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "a boolean",
                serde_json::Value::Number(_) => "a number",
                _ => "a string",
            }
        )),
        Err(err) => Err(err.to_string()),
    }
}

/// JSON-lines text: one JSON object per line, blank lines skipped.
fn read_jsonl(text: &str) -> Result<Value, String> {
    let mut records = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_jsonl_line(line) {
            Ok(rec) => records.push(rec),
            // The line number is the whole diagnostic for a large file.
            Err(msg) => {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "ods.read_jsonl: line {}: {}",
                    i + 1,
                    msg
                ))))));
            }
        }
    }
    records_to_frame(&records).map(|f| Value::Ok(Box::new(OdsFrame::into_value(f))))
}

/// A Frame as JSON-lines text: one object per row, nulls omitted rather
/// than written as `null`, which is what makes the output round-trip
/// through `read_jsonl` to the same Frame.
fn to_jsonl(frame: &Frame) -> String {
    let mut out = String::new();
    for row in 0..frame.n_rows() {
        let mut obj = serde_json::Map::new();
        for (name, col) in frame.names().iter().zip(frame.columns()) {
            let cell = match col.scalar_at(row) {
                Scalar::Null => continue,
                Scalar::F64(x) => serde_json::Number::from_f64(x)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
                Scalar::I64(x) => serde_json::Value::Number(x.into()),
                Scalar::Bool(b) => serde_json::Value::Bool(b),
                Scalar::Str(s) => serde_json::Value::String(s.to_string()),
            };
            obj.insert(name.clone(), cell);
        }
        out.push_str(&serde_json::Value::Object(obj).to_string());
        out.push('\n');
    }
    out
}

fn records_to_frame(records: &[Value]) -> Result<Frame, String> {
    // json.parse yields objects as structs (type "JsonObject"); literal
    // maps arrive as maps. Accept both record shapes.
    fn record_fields(rec: &Value) -> Result<Vec<(&String, &Value)>, String> {
        match rec {
            Value::Map(m) => Ok(m.iter().collect()),
            Value::Struct { fields, .. } => Ok(fields.iter().collect()),
            other => Err(format!(
                "ods.frame_from_records: each record must be a map, got {}",
                other.type_name()
            )),
        }
    }
    let mut names: Vec<String> = Vec::new();
    for rec in records {
        for (k, _) in record_fields(rec)? {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    names.sort();
    let mut pairs = Vec::with_capacity(names.len());
    for name in names {
        let cells: Vec<Value> = records
            .iter()
            .map(|rec| match rec {
                Value::Map(m) => m.get(&name).cloned().unwrap_or(Value::Unit),
                Value::Struct { fields, .. } => fields.get(&name).cloned().unwrap_or(Value::Unit),
                _ => unreachable!("validated above"),
            })
            .collect();
        let col = series_from_list(&cells).map_err(|err| format!("column '{}': {}", name, err))?;
        pairs.push((name, col));
    }
    Frame::new(pairs).map_err(e)
}

/// Per-column summary statistics, one row per column.
///
/// The numeric statistics are null for String and Bool columns rather
/// than absent: a Frame has one type per column, so the alternative is
/// two shapes of result depending on the input, and a caller that has to
/// branch on the shape of a summary is worse off than one reading nulls.
fn describe(frame: &Frame) -> Result<Value, String> {
    let n = frame.n_cols();
    let mut names = Vec::with_capacity(n);
    let mut dtypes = Vec::with_capacity(n);
    let mut counts = Vec::with_capacity(n);
    let mut nulls = Vec::with_capacity(n);
    // One Vec per statistic, each holding Value::Unit where the column is
    // not numeric — which is how `series_from_list` spells a null.
    let mut stats: Vec<Vec<Value>> = (0..7).map(|_| Vec::with_capacity(n)).collect();

    for (name, col) in frame.names().iter().zip(frame.columns()) {
        names.push(Value::String(Arc::new(name.clone())));
        dtypes.push(Value::String(Arc::new(col.dtype().to_string())));
        let null_count = col.null_count();
        counts.push(Value::Integer((col.len() - null_count) as i64));
        nulls.push(Value::Integer(null_count as i64));

        let numeric = matches!(col.dtype(), olang_ods::DType::F64 | olang_ods::DType::I64);
        let values: [Option<f64>; 7] = if numeric {
            let scalar_f64 = |s: Scalar| match s {
                Scalar::F64(x) => Some(x),
                Scalar::I64(x) => Some(x as f64),
                _ => None,
            };
            [
                col.mean(false).ok().flatten(),
                col.std(false).ok().flatten(),
                col.min().ok().and_then(scalar_f64),
                col.quantile(0.25).ok().flatten(),
                col.quantile(0.5).ok().flatten(),
                col.quantile(0.75).ok().flatten(),
                col.max().ok().and_then(scalar_f64),
            ]
        } else {
            [None; 7]
        };
        for (slot, value) in stats.iter_mut().zip(values) {
            slot.push(value.map(Value::Float).unwrap_or(Value::Unit));
        }
    }

    let labels = ["mean", "std", "min", "q25", "median", "q75", "max"];
    let mut pairs = vec![
        ("column".to_string(), series_from_list(&names)?),
        ("dtype".to_string(), series_from_list(&dtypes)?),
        ("count".to_string(), series_from_list(&counts)?),
        ("nulls".to_string(), series_from_list(&nulls)?),
    ];
    for (label, column) in labels.iter().zip(stats) {
        pairs.push((label.to_string(), series_from_list(&column)?));
    }
    Frame::new(pairs).map(OdsFrame::into_value).map_err(e)
}

/// Name, type, and null count per column — `describe` without the
/// arithmetic, for a Frame too wide to summarize comfortably.
fn schema(frame: &Frame) -> Result<Value, String> {
    let mut names = Vec::with_capacity(frame.n_cols());
    let mut dtypes = Vec::with_capacity(frame.n_cols());
    let mut nulls = Vec::with_capacity(frame.n_cols());
    for (name, col) in frame.names().iter().zip(frame.columns()) {
        names.push(Value::String(Arc::new(name.clone())));
        dtypes.push(Value::String(Arc::new(col.dtype().to_string())));
        nulls.push(Value::Integer(col.null_count() as i64));
    }
    Frame::new(vec![
        ("column".to_string(), series_from_list(&names)?),
        ("dtype".to_string(), series_from_list(&dtypes)?),
        ("nulls".to_string(), series_from_list(&nulls)?),
    ])
    .map(OdsFrame::into_value)
    .map_err(e)
}

// ── streaming ─────────────────────────────────────────────────────
//
// A bounded-memory reader: it holds an open file and its position, and
// hands back one Frame per call. Reading a file larger than memory is
// then an ordinary olang loop.
//
// This is a *handle* rather than a fold taking a callback, and both of
// the reasons are forced rather than chosen. A callback cannot
// accumulate: `(chunk) => { total = total + chunk }` is refused by the
// capture rule, because the write lands on the closure's snapshot. And a
// fold taking an olang lambda cannot live in this module at all —
// `OvmModule::dispatch` receives `(func, args)` and no interpreter, which
// is exactly what lets both tiers dispatch identically. A handle needs
// neither, and it is also the only shape that is not quadratic: a fold
// over chunks re-read from offset zero would rescan the prefix every
// time.
//
// Like a cell, a reader is confined to the thread that created it. It is
// a mutable location — the file position — and the guarantee that no two
// threads reach one of those is what makes olang's parallelism lock-free.

/// An open file, its position, and whatever the format needs to keep.
///
/// One type covers both formats so that `next_chunk`, `rows_read`, and
/// `at_end` are the *same* three verbs whatever was opened — a program
/// that streams a CSV reads identically to one that streams JSON lines,
/// and neither has to name its source again after `open_`.
///
/// The mutex is uncontended by construction (only the owning thread ever
/// reaches it) and exists to satisfy the `Send + Sync` every `Value` must
/// have.
struct RowReader {
    path: String,
    owner: std::thread::ThreadId,
    owner_label: String,
    state: std::sync::Mutex<ReaderState>,
}

struct ReaderState {
    source: Source,
    /// Rows handed out so far, for `ods.rows_read`.
    delivered: usize,
    done: bool,
}

/// Where the rows come from. The two arms differ only in how a batch of
/// rows becomes a Frame; everything around them is shared.
enum Source {
    Csv {
        headers: Vec<String>,
        records: csv::StringRecordsIntoIter<std::io::BufReader<std::fs::File>>,
    },
    Jsonl {
        lines: std::io::Lines<std::io::BufReader<std::fs::File>>,
        /// Columns seen so far. JSON lines carry no header, so without
        /// this the empty chunk that ends the loop would come back with
        /// no columns and a pipeline written against a chunk would need
        /// a special case for its last iteration.
        columns: Vec<String>,
    },
}

impl Source {
    fn label(&self) -> &'static str {
        match self {
            Source::Csv { .. } => "csv",
            Source::Jsonl { .. } => "jsonl",
        }
    }
}

impl std::fmt::Debug for RowReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl NativeObject for RowReader {
    fn module(&self) -> &'static str {
        "ods"
    }
    fn type_name(&self) -> &'static str {
        "Reader"
    }
    fn display(&self) -> String {
        let (kind, at) = match self.state.lock() {
            Ok(st) => (st.source.label(), st.delivered),
            Err(_) => ("?", 0),
        };
        format!("<{} reader {} @ row {}>", kind, self.path, at)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        // Identity, like a cell: two readers over the same path are two
        // distinct positions and are not interchangeable.
        other
            .as_any()
            .downcast_ref::<RowReader>()
            .is_some_and(|o| std::ptr::eq(self, o))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn confined_to(&self) -> Option<std::thread::ThreadId> {
        Some(self.owner)
    }
}

impl RowReader {
    fn own_thread(&self) -> Result<(), String> {
        if std::thread::current().id() == self.owner {
            return Ok(());
        }
        Err(format!(
            "reader escaped its thread: this reader was opened on {} and \
             cannot be read from {}. A reader holds a file position, which is \
             mutable state — open one per thread, or send the rows through a \
             channel",
            self.owner_label,
            match std::thread::current().name() {
                Some("main") => "the main thread".to_string(),
                Some(name) => format!("thread '{}'", name),
                None => "an unnamed thread".to_string(),
            }
        ))
    }
}

fn reader_of<'a>(func: &str, args: &'a [Value]) -> Result<&'a RowReader, String> {
    match args.first() {
        Some(Value::Native(h)) => {
            h.0.as_any()
                .downcast_ref::<RowReader>()
                .ok_or_else(|| format!("ods.{}: argument 1 must be a Reader", func))
        }
        other => Err(format!(
            "ods.{}: argument 1 must be a Reader (from ods.open_csv or \
             ods.open_jsonl), got {}",
            func,
            other.map(|v| v.type_name()).unwrap_or_default()
        )),
    }
}

/// The thread label used in confinement messages, computed once at open
/// time because the *owning* thread's name is what a later error needs.
fn thread_label() -> String {
    match std::thread::current().name() {
        Some("main") => "the main thread".to_string(),
        Some(name) => format!("thread '{}'", name),
        None => "an unnamed thread".to_string(),
    }
}

fn open_error(func: &str, path: &str, err: impl std::fmt::Display) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(format!(
        "ods.{}: {}: {}",
        func, path, err
    )))))
}

fn into_reader(path: &str, source: Source) -> Value {
    let reader = RowReader {
        path: path.to_string(),
        owner: std::thread::current().id(),
        owner_label: thread_label(),
        state: std::sync::Mutex::new(ReaderState {
            source,
            delivered: 0,
            done: false,
        }),
    };
    Value::Ok(Box::new(Value::Native(NativeHandle::new(reader))))
}

fn open_csv(path: &str) -> Result<Value, String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) => return Ok(open_error("open_csv", path, err)),
    };
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(std::io::BufReader::new(file));
    // The header is read eagerly so that an unreadable file fails at
    // `open_csv` rather than at the first `next_chunk`, where the caller
    // has already committed to a loop.
    let headers: Vec<String> = match rdr.headers() {
        Ok(h) => h.iter().map(|s| s.to_string()).collect(),
        Err(err) => return Ok(open_error("open_csv", path, err)),
    };
    Ok(into_reader(
        path,
        Source::Csv {
            headers,
            records: rdr.into_records(),
        },
    ))
}

fn open_jsonl(path: &str) -> Result<Value, String> {
    use std::io::BufRead;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) => return Ok(open_error("open_jsonl", path, err)),
    };
    // Unlike CSV there is no header to validate here: JSON lines carry
    // their own keys, so the columns are whatever the chunk contains.
    Ok(into_reader(
        path,
        Source::Jsonl {
            lines: std::io::BufReader::new(file).lines(),
            columns: Vec::new(),
        },
    ))
}

/// Pull up to `n` rows. The Frame always carries the source's columns, so
/// a pipeline written against it keeps working on the final short chunk
/// and on the empty one that ends the loop.
fn next_chunk(reader: &RowReader, n: usize) -> Result<Value, String> {
    reader.own_thread()?;
    if n == 0 {
        return Err(
            "ods.next_chunk: chunk size must be at least 1 (a zero-row chunk \
             would end the loop without reading anything)"
                .to_string(),
        );
    }
    let mut st = reader
        .state
        .lock()
        .map_err(|_| "reader is unusable: the thread reading it panicked".to_string())?;
    let path = reader.path.clone();

    let (frame, taken, exhausted) = match &mut st.source {
        Source::Csv { headers, records } => {
            let mut cells: Vec<Vec<String>> = vec![Vec::new(); headers.len()];
            let mut taken = 0;
            let mut done = false;
            while taken < n {
                match records.next() {
                    None => {
                        done = true;
                        break;
                    }
                    Some(Err(err)) => {
                        return Ok(open_error("next_chunk", &path, err));
                    }
                    Some(Ok(record)) => {
                        for (c, cell) in cells.iter_mut().enumerate() {
                            cell.push(record.get(c).unwrap_or("").to_string());
                        }
                        taken += 1;
                    }
                }
            }
            // Each chunk infers its own column types, which is the price
            // of not reading the file twice: a column that is all-Int in
            // one chunk and has a decimal in the next comes back Int then
            // Float. Callers that need one type across the whole file
            // should say so with a cast.
            let pairs = headers
                .iter()
                .cloned()
                .zip(cells)
                .map(|(name, raw)| (name, infer_column(raw)))
                .collect();
            (Frame::new(pairs).map_err(e)?, taken, done)
        }
        Source::Jsonl { lines, columns } => {
            let mut records = Vec::with_capacity(n);
            let mut done = false;
            while records.len() < n {
                match lines.next() {
                    None => {
                        done = true;
                        break;
                    }
                    Some(Err(err)) => return Ok(open_error("next_chunk", &path, err)),
                    Some(Ok(line)) => {
                        // Blank lines are skipped rather than refused: a
                        // trailing newline is how most writers finish a
                        // JSON-lines file, and refusing it would make the
                        // common file the failing case.
                        if line.trim().is_empty() {
                            continue;
                        }
                        match parse_jsonl_line(&line) {
                            Ok(record) => records.push(record),
                            Err(msg) => {
                                return Ok(open_error("next_chunk", &path, msg));
                            }
                        }
                    }
                }
            }
            let taken = records.len();
            let frame = if records.is_empty() {
                // Nothing left to infer from, so reuse the schema the
                // earlier chunks established.
                let pairs = columns
                    .iter()
                    .map(|name| (name.clone(), infer_column(Vec::new())))
                    .collect();
                Frame::new(pairs).map_err(e)?
            } else {
                let frame = records_to_frame(&records)?;
                columns.clone_from(&frame.names().to_vec());
                frame
            };
            (frame, taken, done)
        }
    };
    st.delivered += taken;
    st.done = st.done || exhausted;
    Ok(Value::Ok(Box::new(OdsFrame::into_value(frame))))
}

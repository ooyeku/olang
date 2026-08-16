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
        let cols: Vec<String> = self
            .0
            .names()
            .iter()
            .zip(self.0.columns())
            .map(|(n, c)| format!("{}: {}", n, c.dtype()))
            .collect();
        format!(
            "Frame[{} x {}]({})",
            self.0.n_rows(),
            self.0.n_cols(),
            cols.join(", ")
        )
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
    ("to_csv", 1),
    ("write_csv", 2),
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

pub fn dispatch(func: &str, args: Vec<Value>) -> Result<Value, String> {
    let expected = FUNCTIONS
        .iter()
        .find(|(n, _)| *n == func)
        .map(|(_, a)| *a)
        .expect("caller checked membership");
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
    Frame::new(pairs).map(OdsFrame::into_value).map_err(e)
}

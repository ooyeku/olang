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
    ("read_frame", 2),
    ("write_frame", 2),
    ("frame_info", 1),
    ("concat", 1),
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
    ("tail", 2),
    ("rename", 2),
    ("drop", 2),
    ("distinct", 2),
    ("drop_null", 2),
    ("describe", 1),
    ("schema", 1),
    ("group_by", 3),
    ("join", 4),
    ("join_left", 4),
    ("join_full", 4),
    ("join_semi", 4),
    ("join_anti", 4),
    ("pivot", 5),
    ("unpivot", 3),
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

/// Functions whose final declared argument may be omitted, and what it
/// defaults to. `ods.head(f)` is what a hand reaches for at the REPL, and
/// `ods.read_frame(path)` should not demand a column list to mean "all of
/// them". Each default is stated here rather than buried in its arm, so
/// the arity check and the default cannot drift apart.
fn default_tail(func: &str, args: &[Value]) -> Option<Value> {
    match func {
        "head" | "tail" => Some(Value::Integer(DEFAULT_HEAD)),
        // Unit means "every column", which is what these verbs mean
        // without a subset. An empty list would be ambiguous with
        // "consider no columns", which is not a useful request.
        "distinct" | "drop_null" => Some(Value::Unit),
        // Unit reads as "unspecified", which `wanted_columns` turns into
        // every column. An empty list would be ambiguous with asking for
        // no columns at all.
        "read_frame" => Some(Value::Unit),
        // Unit means "every column that is not an id column", which is
        // what unpivot means without a list.
        "unpivot" => Some(Value::Unit),
        // Both sides of a join usually call the key the same thing, and
        // repeating it was pure ceremony. The default is the *other*
        // side's name rather than a constant, which is why this takes the
        // arguments.
        "join" | "join_left" | "join_full" | "join_semi" | "join_anti" => args.get(2).cloned(),
        _ => None,
    }
}
const DEFAULT_HEAD: i64 = 10;

pub fn dispatch(func: &str, mut args: Vec<Value>) -> Result<Value, String> {
    let expected = FUNCTIONS
        .iter()
        .find(|(n, _)| *n == func)
        .map(|(_, a)| *a)
        .expect("caller checked membership");
    if args.len() + 1 == expected
        && let Some(default) = default_tail(func, &args)
    {
        args.push(default);
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
        // The native columnar format: types survive the round trip, the
        // load is a read rather than a parse, and a single column can be
        // fetched without touching the others.
        "write_frame" => {
            let f = want_frame(func, &args, 0)?;
            let path = want_string(func, &args, 1)?;
            match std::fs::write(&path, super::columns::encode(f)) {
                Ok(()) => Ok(Value::Ok(Box::new(Value::Unit))),
                Err(err) => Ok(open_error("write_frame", &path, err)),
            }
        }
        "read_frame" => {
            let path = want_string(func, &args, 0)?;
            // Misuse of the argument raises; a bad *file* is a Result,
            // because the caller chose the file but wrote the argument.
            let wanted = super::columns::wanted_columns(args.get(1))?;
            match std::fs::read(&path) {
                Err(err) => Ok(open_error("read_frame", &path, err)),
                Ok(bytes) => match super::columns::decode(&bytes, wanted.as_deref()) {
                    Ok(frame) => Ok(Value::Ok(Box::new(OdsFrame::into_value(frame)))),
                    Err(msg) => Ok(open_error("read_frame", &path, msg)),
                },
            }
        }
        "frame_info" => {
            let path = want_string(func, &args, 0)?;
            match std::fs::read(&path) {
                Err(err) => Ok(open_error("frame_info", &path, err)),
                Ok(bytes) => match super::columns::info(&bytes) {
                    Ok(frame) => Ok(Value::Ok(Box::new(OdsFrame::into_value(frame)))),
                    Err(msg) => Ok(open_error("frame_info", &path, msg)),
                },
            }
        }
        // Row-binding. Streaming turns one pass into many partial
        // results, and without this there is no way to put them back
        // together except a detour through row-shaped values — which is
        // exactly what the columnar representation exists to avoid.
        "concat" => {
            let frames = match &args[0] {
                Value::List(items) => items,
                other => {
                    return Err(format!(
                        "ods.concat: expects a list of Frames, got {}",
                        other.type_name()
                    ));
                }
            };
            concat(frames)
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
        "tail" => {
            let f = want_frame(func, &args, 0)?;
            let n = match &args[1] {
                Value::Integer(n) if *n >= 0 => (*n as usize).min(f.n_rows()),
                other => {
                    return Err(format!(
                        "ods.tail: n must be a non-negative Int, got {}",
                        other
                    ));
                }
            };
            let start = f.n_rows() - n;
            let idx = Series::from_i64((start..f.n_rows()).map(|i| i as i64).collect());
            f.take(&idx).map(OdsFrame::into_value).map_err(e)
        }
        // Renaming is what makes a join survivable: `join` suffixes a
        // collision `_right`, and until now there was no way to give it
        // the name the rest of the pipeline wants.
        "rename" => {
            let f = want_frame(func, &args, 0)?;
            let renames = match &args[1] {
                Value::Map(m) => m.as_ref().clone(),
                other => {
                    return Err(format!(
                        "ods.rename: expects a map of old name to new name, got {}",
                        other.type_name()
                    ));
                }
            };
            let mut names: Vec<String> = f.names().to_vec();
            for (old, new) in renames.iter() {
                let new = match new {
                    Value::String(s) => s.as_ref().clone(),
                    other => {
                        return Err(format!(
                            "ods.rename: the new name for '{}' must be a String, got {}",
                            old,
                            other.type_name()
                        ));
                    }
                };
                let at = names.iter().position(|n| n == old).ok_or_else(|| {
                    format!(
                        "ods.rename: no column '{}' in this Frame. It has: {}",
                        old,
                        f.names().join(", ")
                    )
                })?;
                // A rename onto a name that is staying would make two
                // columns indistinguishable, which Frame::new refuses
                // anyway — refused here so the message names the cause.
                if let Some(clash) = names
                    .iter()
                    .enumerate()
                    .find(|(i, n)| *i != at && *n == &new)
                {
                    return Err(format!(
                        "ods.rename: '{}' would collide with the existing column '{}'",
                        new, clash.1
                    ));
                }
                names[at] = new;
            }
            let pairs = names
                .into_iter()
                .zip(f.columns().iter().cloned())
                .collect::<Vec<_>>();
            Frame::new(pairs).map(OdsFrame::into_value).map_err(e)
        }
        // The complement of `select`. Naming what to remove is shorter
        // and more robust than listing everything to keep, which silently
        // drops a column added upstream.
        "drop" => {
            let f = want_frame(func, &args, 0)?;
            let drops = string_list(func, &args, 1)?;
            for name in &drops {
                if !f.names().iter().any(|n| n == name) {
                    return Err(format!(
                        "ods.drop: no column '{}' in this Frame. It has: {}",
                        name,
                        f.names().join(", ")
                    ));
                }
            }
            let keep: Vec<String> = f
                .names()
                .iter()
                .filter(|n| !drops.contains(n))
                .cloned()
                .collect();
            f.select(&keep).map(OdsFrame::into_value).map_err(e)
        }
        "distinct" => {
            let f = want_frame(func, &args, 0)?;
            let cols = subset_columns(func, f, args.get(1))?;
            let mut seen = std::collections::HashSet::new();
            let mut keep = Vec::new();
            for row in 0..f.n_rows() {
                if seen.insert(row_key(&cols, row)) {
                    keep.push(row as i64);
                }
            }
            let idx = Series::from_i64(keep);
            f.take(&idx).map(OdsFrame::into_value).map_err(e)
        }
        "drop_null" => {
            let f = want_frame(func, &args, 0)?;
            let cols = subset_columns(func, f, args.get(1))?;
            // Nullness is the validity bitmap, so ask it directly: a
            // column with no bitmap has no nulls and drops out of the
            // check entirely, and the columns that remain answer with a
            // bit test instead of materializing a Scalar per cell (which
            // cloned every string it passed over).
            let masks: Vec<&olang_ods::Bitmap> = cols.iter().filter_map(|c| c.validity()).collect();
            let n_rows = f.n_rows();
            let keep: Vec<i64> = if masks.is_empty() {
                (0..n_rows as i64).collect()
            } else {
                (0..n_rows)
                    .filter(|&row| masks.iter().all(|m| m.get(row)))
                    .map(|row| row as i64)
                    .collect()
            };
            let idx = Series::from_i64(keep);
            f.take(&idx).map(OdsFrame::into_value).map_err(e)
        }
        "group_by" => {
            let f = want_frame(func, &args, 0)?;
            let keys = string_list(func, &args, 1)?;
            let aggs = parse_aggs(&args[2])?;
            f.group_by(&keys, &aggs)
                .map(OdsFrame::into_value)
                .map_err(e)
        }
        // Long to wide. The aggregation is required rather than
        // defaulted: when a cell has more than one row behind it, which
        // reduction applies is the caller's decision, and picking one
        // silently is how a wrong number gets into a report.
        "pivot" => {
            let f = want_frame(func, &args, 0)?;
            let index = string_list(func, &args, 1)?;
            let columns = want_string(func, &args, 2)?;
            let values = want_string(func, &args, 3)?;
            let op_name = want_string(func, &args, 4)?;
            let op = AggOp::parse(&op_name).ok_or_else(|| {
                format!(
                    "ods.pivot: '{}' is not an aggregation. Expected one of \
                     count, sum, mean, min, max",
                    op_name
                )
            })?;
            f.pivot(&index, &columns, &values, op, &crate::ast::format_float)
                .map(OdsFrame::into_value)
                .map_err(e)
        }
        // Wide to long.
        "unpivot" => {
            let f = want_frame(func, &args, 0)?;
            let id = string_list(func, &args, 1)?;
            let value_columns = match &args[2] {
                // Everything that is not an id column, in the Frame's own
                // order — the common case, and the one that survives a
                // new column arriving upstream.
                Value::Unit => f
                    .names()
                    .iter()
                    .filter(|n| !id.contains(n))
                    .cloned()
                    .collect(),
                other => {
                    let named = string_list(func, std::slice::from_ref(other), 0)?;
                    for name in &named {
                        f.column(name).map_err(|_| {
                            format!(
                                "ods.unpivot: no column '{}' in this Frame. It has: {}",
                                name,
                                f.names().join(", ")
                            )
                        })?;
                    }
                    named
                }
            };
            f.unpivot(&id, &value_columns, "name", "value")
                .map(OdsFrame::into_value)
                .map_err(e)
        }
        "join" | "join_left" | "join_full" | "join_semi" | "join_anti" => {
            let l = want_frame(func, &args, 0)?;
            let r = want_frame(func, &args, 1)?;
            let left_on = want_string(func, &args, 2)?;
            let right_on = want_string(func, &args, 3)?;
            let how = match func {
                "join" => JoinHow::Inner,
                "join_left" => JoinHow::Left,
                "join_full" => JoinHow::Full,
                "join_semi" => JoinHow::Semi,
                _ => JoinHow::Anti,
            };
            l.join(r, &left_on, &right_on, how)
                .map(OdsFrame::into_value)
                .map_err(e)
        }
        _ => unreachable!("caller checked membership"),
    }
}

/// Frame-typed overloads for the names shared with Series.
/// Build a Frame from columns that are already Series. `value_counts`
/// returns a Frame from the Series layer, and this is the one place that
/// knows how a Frame becomes a Value.
pub fn from_columns(pairs: Vec<(String, Series)>) -> Result<Value, String> {
    Frame::new(pairs).map(OdsFrame::into_value).map_err(e)
}

/// Choose `n` distinct row positions out of `len`, in increasing order.
///
/// Shared by the Frame and Series forms of `sample` so they cannot drift
/// apart. Three decisions worth naming:
///
/// Sampling is **without replacement** — asking for 100 rows gives 100
/// distinct rows, which is what "a sample of the data" means to a reader
/// looking at one. Asking for more rows than exist yields all of them
/// rather than an error, matching `head` and `tail`.
///
/// Rows come back in their **original order**, not draw order. A sample
/// is meant to be looked at, and the alternative — shuffling as a side
/// effect of sampling — makes a sample of a sorted frame unreadable.
///
/// Randomness comes from the same seeded stream `random.seed(n)`
/// governs, as `ods.stats`' Monte Carlo already does. So a sample is
/// reproducible exactly when the program says it is, and there is one
/// seed to set rather than one per verb.
/// Turn a uniform in [0, 1) into an index below `span`. The clamp cannot
/// trigger for a well-formed uniform; it is there so a float bound can
/// never become an out-of-range index.
fn index_from(u: f64, span: usize) -> usize {
    ((u * span as f64) as usize).min(span - 1)
}

pub fn sample_indices(func: &str, len: usize, n: Option<&Value>) -> Result<Series, String> {
    let want = match n {
        Some(Value::Integer(k)) if *k >= 0 => (*k as usize).min(len),
        Some(Value::Integer(k)) => {
            return Err(format!(
                "ods.{}: n must be a non-negative Int, got {}",
                func, k
            ));
        }
        other => {
            return Err(format!(
                "ods.{}: n must be an Int, got {}",
                func,
                other.map(|v| v.type_name()).unwrap_or_default()
            ));
        }
    };
    // A small sample out of a large frame — `ods.sample(f, 5)` on ten
    // million rows, which is the gesture this verb exists for — draws by
    // rejecting collisions, so the memory it touches is proportional to
    // the sample rather than to the frame. Below the ratio, collisions
    // stop being rare and the shuffle is both cheaper and bounded.
    let mut pool: Vec<i64> = if want > 0 && want.saturating_mul(4) <= len {
        let mut chosen: std::collections::HashSet<i64> =
            std::collections::HashSet::with_capacity(want);
        while chosen.len() < want {
            // One batch per round rather than one draw per candidate:
            // the RNG is behind a lock, and at this ratio a round
            // usually finishes the job.
            for u in crate::stdlib::random::draw_uniforms(want - chosen.len()) {
                chosen.insert(index_from(u, len) as i64);
            }
        }
        chosen.into_iter().collect()
    } else {
        // Partial Fisher-Yates: `want` swaps, not `len`.
        let draws = crate::stdlib::random::draw_uniforms(want);
        let mut pool: Vec<i64> = (0..len as i64).collect();
        for (i, u) in draws.iter().enumerate() {
            pool.swap(i, i + index_from(*u, len - i));
        }
        pool.truncate(want);
        pool
    };
    // Sorting is what puts the rows back in the frame's own order, and
    // it is also what makes the result independent of HashSet iteration
    // order above.
    pool.sort_unstable();
    Ok(Series::from_i64(pool))
}

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
        "sample" => Some(
            sample_indices(func, f.n_rows(), args.get(1))
                .and_then(|idx| f.take(&idx).map(OdsFrame::into_value).map_err(e)),
        ),
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
/// Below this many rows, per-column work (type inference, record
/// gather) stays on one core; the hand-off costs more than it saves.
const PAR_COLUMNS_MIN_ROWS: usize = 100_000;

/// Below this many lines, JSONL parses on one core.
const PAR_JSONL_MIN_LINES: usize = 10_000;

/// How many workers the data stack may use for `n` independent items,
/// honoring `set_parallel` — 1 means "stay sequential".
fn stack_workers(n: usize) -> usize {
    // OLANG_ODS_WORKERS caps the data-stack fan-out — the tuning knob
    // the parallel-scaling measurements use.
    if let Some(cap) = std::env::var("OLANG_ODS_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        return cap.clamp(1, n.max(1));
    }
    #[cfg(feature = "native")]
    {
        let cfg = crate::parallel::get_config();
        if cfg.enabled {
            cfg.max_threads.clamp(1, n.max(1))
        } else {
            1
        }
    }
    #[cfg(not(feature = "native"))]
    {
        let _ = n;
        1
    }
}

/// Map `f` over `items` on `workers` threads, preserving order: items
/// split into contiguous chunks, chunk results concatenated in chunk
/// order, so the output is identical to the sequential map. The
/// data-stack twin of `parallel_apply` for host-side work (parsing,
/// column inference) rather than olang kernels.
/// Promote the calling worker thread to user-initiated QoS on macOS.
/// Threads inherit their spawner's QoS class; when the process runs at
/// a background class, the scheduler confines its threads to
/// efficiency cores and data-parallel fan-out collapses to ~2x. A
/// data-stack worker is user-initiated work by definition — the caller
/// is waiting on it.
#[cfg(target_os = "macos")]
fn promote_worker_qos() {
    // SAFETY: setting the current thread's QoS class has no memory
    // preconditions; 0x21 is QOS_CLASS_USER_INITIATED.
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }
    unsafe {
        let _ = pthread_set_qos_class_self_np(0x21, 0);
    }
}

#[cfg(not(target_os = "macos"))]
fn promote_worker_qos() {}

fn parallel_map_ordered<T: Send, R: Send>(
    items: Vec<T>,
    workers: usize,
    f: impl Fn(T) -> R + Sync,
) -> Vec<R> {
    if workers <= 1 || items.len() <= 1 {
        return items.into_iter().map(f).collect();
    }
    let chunk_size = items.len().div_ceil(workers);
    let mut chunks: Vec<Vec<T>> = Vec::with_capacity(workers);
    let mut it = items.into_iter();
    loop {
        let chunk: Vec<T> = it.by_ref().take(chunk_size).collect();
        if chunk.is_empty() {
            break;
        }
        chunks.push(chunk);
    }
    let f = &f;
    std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || {
                    promote_worker_qos();
                    chunk.into_iter().map(f).collect::<Vec<R>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("data-stack worker panicked"))
            .collect()
    })
}

/// Byte offsets that split `text` into about `parts` runs of whole CSV
/// records: each split lands just after a newline that is *outside*
/// quotes (RFC 4180 — a doubled `""` toggles the quote state twice, so
/// it nets out). One sequential pass over the bytes; the parse of each
/// run then fans out.
/// Files at or above this size parse across cores; below it the split
/// and merge cost more than the parse they save.
const PAR_CSV_MIN_BYTES: usize = 4 * 1024 * 1024;

/// Record-aligned split offsets for an UNQUOTED body: jump to each
/// equal-stride target and take the next newline. O(parts) memchr
/// probes on small windows — where the quote-aware splitter below
/// walks every byte, which on a 40 MB body cost more than the whole
/// parallel scan it was setting up.
fn unquoted_record_splits(text: &str, parts: usize) -> Vec<usize> {
    let bytes = text.as_bytes();
    let stride = bytes.len().div_ceil(parts.max(1)).max(1);
    let mut splits = vec![0usize];
    let mut target = stride;
    while target < bytes.len() {
        match memchr::memchr(b'\n', &bytes[target..]) {
            Some(off) => {
                let pos = target + off + 1;
                if pos < bytes.len() {
                    splits.push(pos);
                }
                target = pos + stride;
            }
            None => break,
        }
    }
    splits.push(bytes.len());
    splits
}

fn csv_record_splits(text: &str, parts: usize) -> Vec<usize> {
    let bytes = text.as_bytes();
    let stride = bytes.len().div_ceil(parts.max(1)).max(1);
    let mut splits = vec![0usize];
    let mut next_target = stride;
    let mut in_quotes = false;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' => in_quotes = !in_quotes,
            b'\n' if !in_quotes && i + 1 >= next_target => {
                if i + 1 < bytes.len() {
                    splits.push(i + 1);
                }
                next_target = (i + 1) + stride;
            }
            _ => {}
        }
    }
    splits.push(bytes.len());
    splits
}

// ---------------------------------------------------------------------
// The fused CSV scan: delimiters found and fields parsed in one pass
// ---------------------------------------------------------------------

// ---------------------------------------------------------------------
// Delimiter scanning
// ---------------------------------------------------------------------

/// Positions of every `,` and `\n` in `bytes`, in order. On aarch64 the
/// classification is NEON: 16 bytes per compare, the 0xFF/0x00 lane
/// mask narrowed to a 64-bit nibble mask with the `vshrn` idiom (4 bits
/// per byte), and set nibbles iterated with trailing_zeros. Delimiters
/// arrive every ~7 bytes in a typical CSV, which is exactly the density
/// where memchr's find-one-at-a-time loop pays per-call overhead per
/// hit; classifying a block at a time amortizes it.
#[cfg(target_arch = "aarch64")]
struct DelimIter<'a> {
    bytes: &'a [u8],
    /// Start of the block `mask` describes.
    base: usize,
    /// Nibble mask for that block: 4 identical bits per matched byte.
    mask: u64,
    /// Next unclassified position.
    next: usize,
}

#[cfg(target_arch = "aarch64")]
impl<'a> DelimIter<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        DelimIter {
            bytes,
            base: 0,
            mask: 0,
            next: 0,
        }
    }

    #[inline]
    fn classify(bytes: &[u8], at: usize) -> u64 {
        use std::arch::aarch64::*;
        // SAFETY: caller guarantees at + 16 <= bytes.len(); NEON is
        // baseline on aarch64, and vld1q_u8 is alignment-free.
        unsafe {
            let v = vld1q_u8(bytes.as_ptr().add(at));
            let hit = vorrq_u8(
                vceqq_u8(v, vdupq_n_u8(b',')),
                vceqq_u8(v, vdupq_n_u8(b'\n')),
            );
            let narrowed = vshrn_n_u16::<4>(vreinterpretq_u16_u8(hit));
            vget_lane_u64::<0>(vreinterpret_u64_u8(narrowed))
        }
    }
}

#[cfg(target_arch = "aarch64")]
impl<'a> Iterator for DelimIter<'a> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        loop {
            if self.mask != 0 {
                let tz = self.mask.trailing_zeros() as usize;
                // Clear the whole nibble — all 4 bits mark one byte.
                self.mask &= !(0xFu64 << (tz & !3));
                return Some(self.base + (tz >> 2));
            }
            if self.next + 16 <= self.bytes.len() {
                self.mask = Self::classify(self.bytes, self.next);
                self.base = self.next;
                self.next += 16;
                continue;
            }
            // Scalar tail (under 16 bytes remain).
            if self.next >= self.bytes.len() {
                return None;
            }
            let pos = self.next;
            self.next += 1;
            let b = self.bytes[pos];
            if b == b',' || b == b'\n' {
                return Some(pos);
            }
        }
    }
}

#[cfg(not(target_arch = "aarch64"))]
struct DelimIter<'a>(memchr::Memchr2<'a>);

#[cfg(not(target_arch = "aarch64"))]
impl<'a> DelimIter<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        DelimIter(memchr::memchr2_iter(b',', b'\n', bytes))
    }
}

#[cfg(not(target_arch = "aarch64"))]
impl<'a> Iterator for DelimIter<'a> {
    type Item = usize;
    #[inline]
    fn next(&mut self) -> Option<usize> {
        self.0.next()
    }
}

/// One column's outcome for one chunk of the fused scan.
enum ChunkCol {
    /// Every cell in this chunk was empty — compatible with any type.
    AllNull { n: usize },
    I64 {
        values: Vec<i64>,
        valid: Option<Vec<bool>>,
    },
    F64 {
        values: Vec<f64>,
        valid: Option<Vec<bool>>,
    },
    Bool {
        values: Vec<bool>,
        valid: Option<Vec<bool>>,
    },
    /// Spans into the body (already rebased); an empty span is a null.
    Str { spans: Vec<(u32, u32)> },
}

impl ChunkCol {
    fn len(&self) -> usize {
        match self {
            ChunkCol::AllNull { n } => *n,
            ChunkCol::I64 { values, .. } => values.len(),
            ChunkCol::F64 { values, .. } => values.len(),
            ChunkCol::Bool { values, .. } => values.len(),
            ChunkCol::Str { spans } => spans.len(),
        }
    }
}

/// A fast i64 parse for the hot loop: sign plus up to 18 digits needs
/// no overflow check (i64::MAX has 19); longer digit runs go through
/// the stdlib's checked parse. Accepts and refuses exactly what
/// `str::parse::<i64>` does.
#[inline]
fn parse_i64_fast(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let (neg, digits) = match b.first()? {
        b'-' => (true, &b[1..]),
        b'+' => (false, &b[1..]),
        _ => (false, b),
    };
    if digits.is_empty() || digits.len() > 18 {
        return s.parse::<i64>().ok();
    }
    let mut v: i64 = 0;
    for &d in digits {
        let d = d.wrapping_sub(b'0');
        if d > 9 {
            return None;
        }
        v = v * 10 + d as i64;
    }
    Some(if neg { -v } else { v })
}

/// The per-column speculative builder the fused scan drives.
enum Builder {
    /// No non-empty cell yet; `nulls` counts the empties.
    Undecided {
        nulls: usize,
    },
    I64 {
        values: Vec<i64>,
        valid: Option<Vec<bool>>,
    },
    F64 {
        values: Vec<f64>,
        valid: Option<Vec<bool>>,
    },
    Bool {
        values: Vec<bool>,
        valid: Option<Vec<bool>>,
    },
    Str {
        spans: Vec<(u32, u32)>,
    },
}

impl Builder {
    #[inline]
    fn push_null(&mut self) {
        match self {
            Builder::Undecided { nulls } => *nulls += 1,
            Builder::I64 { values, valid } => {
                Self::mark_null(valid, values.len());
                values.push(0);
            }
            Builder::F64 { values, valid } => {
                Self::mark_null(valid, values.len());
                values.push(0.0);
            }
            Builder::Bool { values, valid } => {
                Self::mark_null(valid, values.len());
                values.push(false);
            }
            Builder::Str { spans } => spans.push((0, 0)),
        }
    }

    #[inline]
    fn mark_null(valid: &mut Option<Vec<bool>>, len: usize) {
        valid.get_or_insert_with(|| vec![true; len]).push(false);
    }

    #[inline]
    fn push_valid(valid: &mut Option<Vec<bool>>) {
        if let Some(v) = valid.as_mut() {
            v.push(true);
        }
    }
}

/// The first non-empty cell decides a column's speculation, and its
/// value is the builder's first entry.
fn seed_builder(cell: &str, nulls: usize, span: (u32, u32), est_rows: usize) -> Builder {
    let cap = est_rows.max(nulls + 1);
    let valid = (nulls > 0).then(|| {
        let mut v = Vec::with_capacity(cap);
        v.extend(std::iter::repeat_n(false, nulls));
        v.push(true);
        v
    });
    if let Some(v) = parse_i64_fast(cell) {
        let mut values = Vec::with_capacity(cap);
        values.extend(std::iter::repeat_n(0i64, nulls));
        values.push(v);
        Builder::I64 { values, valid }
    } else if let Ok(f) = cell.parse::<f64>() {
        let mut values = Vec::with_capacity(cap);
        values.extend(std::iter::repeat_n(0.0f64, nulls));
        values.push(f);
        Builder::F64 { values, valid }
    } else if cell == "true" || cell == "false" {
        let mut values = Vec::with_capacity(cap);
        values.extend(std::iter::repeat_n(false, nulls));
        values.push(cell == "true");
        Builder::Bool { values, valid }
    } else {
        let mut spans = Vec::with_capacity(cap);
        spans.extend(std::iter::repeat_n((0u32, 0u32), nulls));
        spans.push(span);
        Builder::Str { spans }
    }
}

/// Feed one cell to its builder. Returns true when the column just
/// demoted to text — the caller stops feeding it and re-scans for its
/// spans at the end of the chunk.
#[inline]
fn emit_cell(b: &mut Builder, cell: &str, span: (u32, u32), est_rows: usize) -> bool {
    if cell.is_empty() {
        b.push_null();
        return false;
    }
    match b {
        Builder::Undecided { nulls } => {
            let n = *nulls;
            *b = seed_builder(cell, n, span, est_rows);
        }
        Builder::I64 { values, valid } => {
            if let Some(v) = parse_i64_fast(cell) {
                Builder::push_valid(valid);
                values.push(v);
            } else if let Ok(f) = cell.parse::<f64>() {
                // Upgrade in place: an i64 converts to exactly the
                // double its decimal text would parse to, so the
                // accumulated values re-read losslessly.
                let mut fv: Vec<f64> = std::mem::take(values)
                    .into_iter()
                    .map(|x| x as f64)
                    .collect();
                let mut vd = valid.take();
                Builder::push_valid(&mut vd);
                fv.push(f);
                *b = Builder::F64 {
                    values: fv,
                    valid: vd,
                };
            } else {
                *b = Builder::Str { spans: Vec::new() };
                return true;
            }
        }
        Builder::F64 { values, valid } => {
            if let Ok(f) = cell.parse::<f64>() {
                Builder::push_valid(valid);
                values.push(f);
            } else {
                *b = Builder::Str { spans: Vec::new() };
                return true;
            }
        }
        Builder::Bool { values, valid } => match cell {
            "true" => {
                Builder::push_valid(valid);
                values.push(true);
            }
            "false" => {
                Builder::push_valid(valid);
                values.push(false);
            }
            _ => {
                *b = Builder::Str { spans: Vec::new() };
                return true;
            }
        },
        Builder::Str { spans } => spans.push(span),
    }
    false
}

/// Scan one record-aligned chunk of the body, parsing fields into
/// per-column builders as the separators arrive. Returns `None` when
/// the chunk's shape disqualifies the fast path (a short or long row).
fn fused_scan_chunk(
    body: &str,
    range: std::ops::Range<usize>,
    n_cols: usize,
) -> Option<Vec<ChunkCol>> {
    let base = range.start as u32;
    let chunk = &body[range];
    let bytes = chunk.as_bytes();
    // Pre-size builders from a bytes-per-row estimate: without it a
    // 50k-row chunk column reallocates through ~16 doublings.
    let est_rows = bytes.len() / (n_cols * 4).max(8) + 8;
    let mut builders: Vec<Builder> = (0..n_cols)
        .map(|_| Builder::Undecided { nulls: 0 })
        .collect();
    let mut demoted: Vec<bool> = vec![false; n_cols];
    let mut any_demoted = false;

    let mut field_start = 0usize;
    let mut col = 0usize;
    for pos in DelimIter::new(bytes) {
        if bytes[pos] == b',' {
            if col + 1 >= n_cols {
                return None; // more fields than the header declares
            }
            if !demoted[col] {
                let cell = &chunk[field_start..pos];
                let span = (base + field_start as u32, (pos - field_start) as u32);
                if emit_cell(&mut builders[col], cell, span, est_rows) {
                    demoted[col] = true;
                    any_demoted = true;
                }
            }
            col += 1;
        } else {
            let mut end = pos;
            if end > field_start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            // Blank lines skip, exactly as the csv crate skips them.
            if col == 0 && end == field_start {
                field_start = pos + 1;
                continue;
            }
            if col != n_cols - 1 {
                return None; // short row
            }
            if !demoted[col] {
                let cell = &chunk[field_start..end];
                let span = (base + field_start as u32, (end - field_start) as u32);
                if emit_cell(&mut builders[col], cell, span, est_rows) {
                    demoted[col] = true;
                    any_demoted = true;
                }
            }
            col = 0;
        }
        field_start = pos + 1;
    }
    // A final line without a trailing newline.
    if field_start < bytes.len() || col > 0 {
        let mut end = bytes.len();
        if end > field_start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        if !(col == 0 && end == field_start) {
            if col != n_cols - 1 {
                return None;
            }
            if !demoted[col] {
                let cell = &chunk[field_start..end];
                let span = (base + field_start as u32, (end - field_start) as u32);
                if emit_cell(&mut builders[col], cell, span, est_rows) {
                    demoted[col] = true;
                    any_demoted = true;
                }
            }
        }
    }

    // Columns that demoted mid-chunk re-scan this chunk for their
    // spans — paid only on a real type conflict.
    if any_demoted {
        let span_cols = rescan_spans(chunk, base, n_cols, &demoted)?;
        for (c, spans) in span_cols.into_iter().enumerate() {
            if demoted[c] {
                builders[c] = Builder::Str { spans };
            }
        }
    }

    Some(
        builders
            .into_iter()
            .map(|b| match b {
                Builder::Undecided { nulls } => ChunkCol::AllNull { n: nulls },
                Builder::I64 { values, valid } => ChunkCol::I64 { values, valid },
                Builder::F64 { values, valid } => ChunkCol::F64 { values, valid },
                Builder::Bool { values, valid } => ChunkCol::Bool { values, valid },
                Builder::Str { spans } => ChunkCol::Str { spans },
            })
            .collect(),
    )
}

/// Extract the spans of selected columns from one chunk — the fallback
/// for mid-chunk demotions and cross-chunk type conflicts.
fn rescan_spans(
    chunk: &str,
    base: u32,
    n_cols: usize,
    wanted: &[bool],
) -> Option<Vec<Vec<(u32, u32)>>> {
    let bytes = chunk.as_bytes();
    let mut cols: Vec<Vec<(u32, u32)>> = (0..n_cols).map(|_| Vec::new()).collect();
    let mut field_start = 0usize;
    let mut col = 0usize;
    for pos in memchr::memchr2_iter(b',', b'\n', bytes) {
        if bytes[pos] == b',' {
            if col + 1 >= n_cols {
                return None;
            }
            if wanted[col] {
                cols[col].push((base + field_start as u32, (pos - field_start) as u32));
            }
            col += 1;
        } else {
            let mut end = pos;
            if end > field_start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            if col == 0 && end == field_start {
                field_start = pos + 1;
                continue;
            }
            if col != n_cols - 1 {
                return None;
            }
            if wanted[col] {
                cols[col].push((base + field_start as u32, (end - field_start) as u32));
            }
            col = 0;
        }
        field_start = pos + 1;
    }
    if field_start < bytes.len() || col > 0 {
        let mut end = bytes.len();
        if end > field_start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        if !(col == 0 && end == field_start) {
            if col != n_cols - 1 {
                return None;
            }
            if wanted[col] {
                cols[col].push((base + field_start as u32, (end - field_start) as u32));
            }
        }
    }
    Some(cols)
}

/// Append a chunk's validity onto a column's, materializing the column
/// bitmap lazily (only once some chunk carries nulls).
fn append_validity(
    col_valid: &mut Option<Vec<bool>>,
    chunk_valid: Option<Vec<bool>>,
    prior_len: usize,
    chunk_len: usize,
) {
    match (col_valid.as_mut(), chunk_valid) {
        (None, None) => {}
        (Some(cv), None) => cv.extend(std::iter::repeat_n(true, chunk_len)),
        (None, Some(chv)) => {
            let mut cv = vec![true; prior_len];
            cv.extend(chv);
            *col_valid = Some(cv);
        }
        (Some(cv), Some(chv)) => cv.extend(chv),
    }
}

/// Reconcile one column's chunk outcomes into a Series, with the
/// cascade's semantics: Int and Float mix as Float, Bool mixes with
/// nothing numeric, any conflict lands on text (chunks that parsed as
/// numbers re-derive their spans from the raw bytes). Returns `None`
/// only if a re-scan hits a shape violation, which the fused scan has
/// already ruled out — kept as an honest bail rather than a panic.
fn merge_column(
    body_arc: &std::sync::Arc<str>,
    chunks: Vec<ChunkCol>,
    chunk_ranges: &[std::ops::Range<usize>],
    col: usize,
    n_cols: usize,
) -> Option<Series> {
    use ChunkCol as C;
    #[derive(Clone, Copy, PartialEq)]
    enum T {
        Null,
        I,
        F,
        B,
        S,
    }
    let mut t = T::Null;
    for c in &chunks {
        let ct = match c {
            C::AllNull { .. } => T::Null,
            C::I64 { .. } => T::I,
            C::F64 { .. } => T::F,
            C::Bool { .. } => T::B,
            C::Str { .. } => T::S,
        };
        t = match (t, ct) {
            (T::Null, x) | (x, T::Null) => x,
            (a, b) if a == b => a,
            (T::I, T::F) | (T::F, T::I) => T::F,
            _ => T::S,
        };
    }

    let total: usize = chunks.iter().map(|c| c.len()).sum();
    match t {
        T::Null => Some(Series::from_str_options(vec![None; total])),
        T::I => {
            let mut values = Vec::with_capacity(total);
            let mut valid: Option<Vec<bool>> = None;
            for c in chunks {
                match c {
                    C::AllNull { n } => {
                        valid
                            .get_or_insert_with(|| vec![true; values.len()])
                            .extend(std::iter::repeat_n(false, n));
                        values.extend(std::iter::repeat_n(0i64, n));
                    }
                    C::I64 {
                        values: v,
                        valid: cv,
                    } => {
                        append_validity(&mut valid, cv, values.len(), v.len());
                        values.extend(v);
                    }
                    _ => unreachable!("reconciled I64"),
                }
            }
            Some(Series::from_i64_with_validity(values, valid))
        }
        T::F => {
            let mut values = Vec::with_capacity(total);
            let mut valid: Option<Vec<bool>> = None;
            for c in chunks {
                match c {
                    C::AllNull { n } => {
                        valid
                            .get_or_insert_with(|| vec![true; values.len()])
                            .extend(std::iter::repeat_n(false, n));
                        values.extend(std::iter::repeat_n(0.0f64, n));
                    }
                    C::I64 {
                        values: v,
                        valid: cv,
                    } => {
                        append_validity(&mut valid, cv, values.len(), v.len());
                        values.extend(v.into_iter().map(|x| x as f64));
                    }
                    C::F64 {
                        values: v,
                        valid: cv,
                    } => {
                        append_validity(&mut valid, cv, values.len(), v.len());
                        values.extend(v);
                    }
                    _ => unreachable!("reconciled F64"),
                }
            }
            Some(Series::from_f64_with_validity(values, valid))
        }
        T::B => {
            let mut values = Vec::with_capacity(total);
            let mut valid: Option<Vec<bool>> = None;
            for c in chunks {
                match c {
                    C::AllNull { n } => {
                        valid
                            .get_or_insert_with(|| vec![true; values.len()])
                            .extend(std::iter::repeat_n(false, n));
                        values.extend(std::iter::repeat_n(false, n));
                    }
                    C::Bool {
                        values: v,
                        valid: cv,
                    } => {
                        append_validity(&mut valid, cv, values.len(), v.len());
                        values.extend(v);
                    }
                    _ => unreachable!("reconciled Bool"),
                }
            }
            Some(Series::from_bool_with_validity(values, valid))
        }
        T::S => {
            let mut spans: Vec<(u32, u32)> = Vec::with_capacity(total);
            let mut valid: Vec<bool> = Vec::with_capacity(total);
            let mut any_null = false;
            let wanted: Vec<bool> = (0..n_cols).map(|c| c == col).collect();
            for (ci, c) in chunks.into_iter().enumerate() {
                let chunk_spans = match c {
                    C::Str { spans } => spans,
                    C::AllNull { n } => vec![(0u32, 0u32); n],
                    _ => {
                        let range = chunk_ranges[ci].clone();
                        let mut got = rescan_spans(
                            &body_arc[range.clone()],
                            range.start as u32,
                            n_cols,
                            &wanted,
                        )?;
                        std::mem::take(&mut got[col])
                    }
                };
                for &(_, l) in &chunk_spans {
                    let ok = l > 0;
                    valid.push(ok);
                    any_null |= !ok;
                }
                spans.extend(chunk_spans);
            }
            Some(Series::from_str_spans(
                body_arc.clone(),
                spans,
                any_null.then_some(valid),
            ))
        }
    }
}

/// The borrowed read path, fused: one scan per record-aligned chunk
/// both finds delimiters and parses each field into its column's
/// speculative typed builder while the bytes are hot in cache. Chunk
/// outcomes reconcile per column with the cascade's exact semantics
/// (empty = null at every type, i64 then f64 then bool then text,
/// Bool never mixing with numbers). Returns `None` when the body is
/// not one this path may decide — a short or long row — leaving the
/// general parser in charge of error reporting.
fn read_csv_borrowed(
    body_arc: &std::sync::Arc<str>,
    headers: &[String],
) -> Option<Result<Value, String>> {
    let body: &str = body_arc;
    let n_cols = headers.len();
    if n_cols == 0 {
        return None;
    }

    let workers = if body.len() >= PAR_CSV_MIN_BYTES {
        stack_workers(usize::MAX)
    } else {
        1
    };
    let timing = std::env::var_os("OLANG_ODS_TIMING").is_some();
    let t0 = std::time::Instant::now();

    let chunk_ranges: Vec<std::ops::Range<usize>> = if workers > 1 {
        unquoted_record_splits(body, workers)
            .windows(2)
            .map(|w| w[0]..w[1])
            .collect()
    } else {
        std::iter::once(0..body.len()).collect()
    };
    let scanned: Vec<Option<Vec<ChunkCol>>> =
        parallel_map_ordered(chunk_ranges.clone(), workers, |range| {
            fused_scan_chunk(body, range, n_cols)
        });
    // Any chunk declining (bad row shape) declines the whole path.
    let mut per_chunk: Vec<Vec<ChunkCol>> = Vec::with_capacity(scanned.len());
    for c in scanned {
        per_chunk.push(c?);
    }
    if timing {
        eprintln!("[ods-timing] fused-scan={:?}", t0.elapsed());
    }
    let t1 = std::time::Instant::now();

    // Transpose to per-column chunk lists and reconcile each column —
    // columns are independent, so large tables merge one per core.
    let mut col_chunks: Vec<Vec<ChunkCol>> = (0..n_cols).map(|_| Vec::new()).collect();
    for chunk in per_chunk {
        for (c, col) in chunk.into_iter().enumerate() {
            col_chunks[c].push(col);
        }
    }
    let n_rows: usize = col_chunks
        .first()
        .map(|c| c.iter().map(|k| k.len()).sum())
        .unwrap_or(0);
    let merge_workers = if n_rows >= PAR_COLUMNS_MIN_ROWS {
        stack_workers(n_cols)
    } else {
        1
    };
    let ranges_ref = &chunk_ranges;
    let merged = parallel_map_ordered(
        headers
            .iter()
            .cloned()
            .zip(col_chunks.into_iter().enumerate())
            .collect(),
        merge_workers,
        |(name, (c, chunks))| {
            let s = merge_column(body_arc, chunks, ranges_ref, c, n_cols);
            (name, s)
        },
    );
    let mut pairs = Vec::with_capacity(merged.len());
    for (name, s) in merged {
        pairs.push((name, s?));
    }
    if timing {
        eprintln!("[ods-timing] merge={:?}", t1.elapsed());
    }
    Some(Frame::new(pairs).map(OdsFrame::into_value).map_err(e))
}

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

    let n_cols = headers.len();
    let body_start = reader.position().byte() as usize;

    // Fast path: an unquoted file needs no owned cell at all. Rows split
    // into borrowed slices, columns infer from those slices, and only a
    // column that really is text allocates — where the general path
    // below allocates every cell before anything knows its type. Any
    // shape this path declines (a quote, a short row) falls through to
    // the general parser, whose errors carry the true line number.
    // The body becomes the shared backing buffer for any column that
    // really is text: string cells are spans into it, so the one copy
    // here replaces a per-cell allocation downstream.
    if !text.as_bytes()[body_start..].contains(&b'"')
        && !headers.is_empty()
        && u32::try_from(text.len() - body_start).is_ok()
    {
        let body_arc: std::sync::Arc<str> = std::sync::Arc::from(&text[body_start..]);
        if let Some(frame) = read_csv_borrowed(&body_arc, &headers) {
            return frame;
        }
    }

    // A large file parses across all cores: split at record boundaries,
    // parse each run with its own reader, and concatenate the runs' cells
    // in order — cell-identical to one reader over the whole text. Any
    // parse error falls back to the sequential reader, whose error
    // carries the true line number.
    let workers = if text.len() >= PAR_CSV_MIN_BYTES {
        stack_workers(usize::MAX)
    } else {
        1
    };
    let mut cells: Vec<Vec<String>> = vec![Vec::new(); n_cols];
    let mut parallel_ok = false;
    if workers > 1 {
        let body_start = reader.position().byte() as usize;
        let splits = csv_record_splits(&text[body_start..], workers);
        let runs: Vec<&str> = splits
            .windows(2)
            .map(|w| &text[body_start + w[0]..body_start + w[1]])
            .collect();
        let parsed: Vec<Result<Vec<Vec<String>>, ()>> =
            parallel_map_ordered(runs, workers, |run| {
                let mut r = csv::ReaderBuilder::new()
                    .has_headers(false)
                    .from_reader(run.as_bytes());
                let mut cols: Vec<Vec<String>> = vec![Vec::new(); n_cols];
                for record in r.records() {
                    let record = record.map_err(|_| ())?;
                    for (c, col) in cols.iter_mut().enumerate() {
                        col.push(record.get(c).unwrap_or("").to_string());
                    }
                }
                Ok(cols)
            });
        if parsed.iter().all(|p| p.is_ok()) {
            for chunk in parsed {
                let chunk = chunk.expect("checked ok above");
                for (c, col) in chunk.into_iter().enumerate() {
                    cells[c].extend(col);
                }
            }
            parallel_ok = true;
        }
    }
    if !parallel_ok {
        for col in cells.iter_mut() {
            col.clear();
        }
        for record in reader.records() {
            let record = record.map_err(|err| format!("ods.read_csv: {}", err))?;
            for (c, cell) in cells.iter_mut().enumerate() {
                cell.push(record.get(c).unwrap_or("").to_string());
            }
        }
    }
    // Column type inference re-scans every cell up to four times; on a
    // large file each column infers on its own core. Columns are
    // independent and the zip order is preserved, so the frame is
    // identical to the sequential build.
    let n_rows = cells.first().map(|c| c.len()).unwrap_or(0);
    let workers = if n_rows >= PAR_COLUMNS_MIN_ROWS {
        stack_workers(headers.len())
    } else {
        1
    };
    let pairs = parallel_map_ordered(
        headers.into_iter().zip(cells).collect(),
        workers,
        |(name, raw)| (name, infer_column(raw)),
    );
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

/// Infer a column's type from its cells and build it, in one pass per
/// candidate type.
///
/// The candidate order is unchanged — all-integer, then all-float, then
/// all-boolean, then string — and so is every result. What changed is
/// that each attempt *keeps what it parses* instead of testing with one
/// scan and rebuilding with a second: an integer column used to parse
/// every cell twice. A failed attempt still costs only the cells before
/// the first non-conforming one, because the loop stops there exactly
/// as `all` short-circuited.
fn infer_column<S: AsRef<str> + Into<String>>(raw: Vec<S>) -> Series {
    infer_column_chunks(vec![raw])
}

fn infer_column_chunks<S: AsRef<str> + Into<String>>(chunks: Vec<Vec<S>>) -> Series {
    let raw_len: usize = chunks.iter().map(|c| c.len()).sum();
    let raw = || chunks.iter().flat_map(|c| c.iter());
    // An all-empty column has no type to infer; it reads as nulls.
    if raw().all(|s| s.as_ref().is_empty()) {
        return Series::from_str_options(vec![None; raw_len]);
    }

    let mut ints: Vec<Option<i64>> = Vec::with_capacity(raw_len);
    if raw().all(|s| {
        let s = s.as_ref();
        if s.is_empty() {
            ints.push(None);
            return true;
        }
        match s.parse::<i64>() {
            Ok(v) => {
                ints.push(Some(v));
                true
            }
            Err(_) => false,
        }
    }) {
        return Series::from_i64_options(ints);
    }
    drop(ints);

    let mut floats: Vec<Option<f64>> = Vec::with_capacity(raw_len);
    if raw().all(|s| {
        let s = s.as_ref();
        if s.is_empty() {
            floats.push(None);
            return true;
        }
        match s.parse::<f64>() {
            Ok(v) => {
                floats.push(Some(v));
                true
            }
            Err(_) => false,
        }
    }) {
        return Series::from_f64_options(floats);
    }
    drop(floats);

    let mut bools: Vec<Option<bool>> = Vec::with_capacity(raw_len);
    if raw().all(|s| match s.as_ref() {
        "" => {
            bools.push(None);
            true
        }
        "true" => {
            bools.push(Some(true));
            true
        }
        "false" => {
            bools.push(Some(false));
            true
        }
        _ => false,
    }) {
        return Series::from_bool_options(bools);
    }
    drop(bools);

    // A column that really is text: pack the owned cells.
    Series::from_str_options(
        chunks
            .into_iter()
            .flatten()
            .map(|s| {
                let s: String = s.into();
                if s.is_empty() { None } else { Some(s) }
            })
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
        Ok(serde_json::Value::Object(obj)) => {
            crate::stdlib::json::json_to_olang_value(serde_json::Value::Object(obj))
        }
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
///
/// Every line is an independent document, so a large file parses across
/// all cores; lines rejoin in order and the *lowest*-numbered bad line
/// reports, exactly as a sequential loop would. The rows then go
/// straight from parsed JSON into column vectors — the row-shaped
/// intermediate this used to build (one boxed map per line) tripled the
/// file's size in allocations and cost more than the parse itself.
fn read_jsonl(text: &str) -> Result<Value, String> {
    let lines: Vec<(usize, &str)> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();
    let workers = if lines.len() >= PAR_JSONL_MIN_LINES {
        stack_workers(lines.len())
    } else {
        1
    };
    let parsed = parallel_map_ordered(lines, workers, |(i, line)| {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(serde_json::Value::Object(obj)) => Ok(obj),
            // A row has named fields; a bare array or number has none,
            // so there is no honest column to put it in.
            Ok(other) => Err((
                i,
                format!(
                    "each line must be a JSON object, got {}",
                    match other {
                        serde_json::Value::Array(_) => "an array",
                        serde_json::Value::Null => "null",
                        serde_json::Value::Bool(_) => "a boolean",
                        serde_json::Value::Number(_) => "a number",
                        _ => "a string",
                    }
                ),
            )),
            Err(err) => Err((i, err.to_string())),
        }
    });
    let mut records = Vec::with_capacity(parsed.len());
    for item in parsed {
        match item {
            Ok(rec) => records.push(rec),
            // The line number is the whole diagnostic for a large file.
            // Results are in line order, so the first Err is the lowest.
            Err((i, msg)) => {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "ods.read_jsonl: line {}: {}",
                    i + 1,
                    msg
                ))))));
            }
        }
    }

    // Columns are the sorted union of keys, missing keys are null —
    // the same shape frame_from_records gives the same file.
    let mut name_set: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for rec in &records {
        for key in rec.keys() {
            name_set.insert(key.as_str());
        }
    }
    let mut names: Vec<String> = name_set.into_iter().map(String::from).collect();
    names.sort();

    // Consume the rows into per-column cell vectors, chunk-parallel:
    // each worker scatters its contiguous run of rows, and the runs
    // concatenate per column in chunk order, so cells stay in row order.
    let col_workers = if records.len() >= PAR_COLUMNS_MIN_ROWS {
        stack_workers(records.len())
    } else {
        1
    };
    let chunk_size = records.len().div_ceil(col_workers.max(1)).max(1);
    let mut chunks: Vec<Vec<serde_json::Map<String, serde_json::Value>>> = Vec::new();
    let mut it = records.into_iter();
    loop {
        let chunk: Vec<_> = it.by_ref().take(chunk_size).collect();
        if chunk.is_empty() {
            break;
        }
        chunks.push(chunk);
    }
    let names_ref = &names;
    let scattered: Vec<Result<Vec<Vec<Value>>, String>> =
        parallel_map_ordered(chunks, col_workers, |chunk| {
            let mut cols: Vec<Vec<Value>> = names_ref
                .iter()
                .map(|_| Vec::with_capacity(chunk.len()))
                .collect();
            for mut rec in chunk {
                for (k, name) in names_ref.iter().enumerate() {
                    cols[k].push(match rec.remove(name) {
                        Some(v) => crate::stdlib::json::json_to_olang_value(v)?,
                        None => Value::Unit,
                    });
                }
            }
            Ok(cols)
        });
    let n_cols = names.len();
    let mut columns: Vec<Vec<Value>> = (0..n_cols).map(|_| Vec::new()).collect();
    for chunk_cols in scattered {
        for (k, col) in chunk_cols?.drain(..).enumerate() {
            columns[k].extend(col);
        }
    }
    let built = parallel_map_ordered(
        names.into_iter().zip(columns).collect::<Vec<_>>(),
        stack_workers(n_cols),
        |(name, cells)| {
            series_from_list(&cells)
                .map(|col| (name.clone(), col))
                .map_err(|err| format!("column '{}': {}", name, err))
        },
    );
    let mut pairs = Vec::with_capacity(built.len());
    for item in built {
        pairs.push(item?);
    }
    Frame::new(pairs)
        .map(|f| Value::Ok(Box::new(OdsFrame::into_value(f))))
        .map_err(e)
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
    // Each output column gathers and type-checks independently, so on a
    // large record set the columns build on their own cores; name order
    // is preserved and the first failing column (in name order) reports,
    // the same as the sequential loop.
    let workers = if records.len() >= PAR_COLUMNS_MIN_ROWS {
        stack_workers(names.len())
    } else {
        1
    };
    let built = parallel_map_ordered(names, workers, |name| {
        let cells: Vec<Value> = records
            .iter()
            .map(|rec| match rec {
                Value::Map(m) => m.get(&name).cloned().unwrap_or(Value::Unit),
                Value::Struct { fields, .. } => fields.get(&name).cloned().unwrap_or(Value::Unit),
                _ => unreachable!("validated above"),
            })
            .collect();
        series_from_list(&cells)
            .map(|col| (name.clone(), col))
            .map_err(|err| format!("column '{}': {}", name, err))
    });
    let mut pairs = Vec::with_capacity(built.len());
    for item in built {
        pairs.push(item?);
    }
    Frame::new(pairs).map_err(e)
}

/// The columns a subset-taking verb should consider: the named ones, or
/// every column when the argument was omitted.
fn subset_columns<'a>(
    func: &str,
    frame: &'a Frame,
    arg: Option<&Value>,
) -> Result<Vec<&'a Series>, String> {
    match arg {
        None | Some(Value::Unit) => Ok(frame.columns().iter().collect()),
        Some(Value::List(items)) => {
            let mut cols = Vec::with_capacity(items.len());
            for item in items.iter() {
                let name = match item {
                    Value::String(s) => s.as_ref().clone(),
                    other => {
                        return Err(format!(
                            "ods.{}: each column name must be a String, got {}",
                            func,
                            other.type_name()
                        ));
                    }
                };
                cols.push(frame.column(&name).map_err(|_| {
                    format!(
                        "ods.{}: no column '{}' in this Frame. It has: {}",
                        func,
                        name,
                        frame.names().join(", ")
                    )
                })?);
            }
            Ok(cols)
        }
        Some(other) => Err(format!(
            "ods.{}: the second argument is a list of column names, got {}",
            func,
            other.type_name()
        )),
    }
}

/// A row's identity across `cols`, as bytes that cannot be confused.
///
/// Each field is length-prefixed rather than separator-joined, so no
/// value can impersonate a field boundary — `["a,b", "c"]` and
/// `["a", "b,c"]` are different rows and must hash differently.
fn row_key(cols: &[&Series], row: usize) -> Vec<u8> {
    let mut key = Vec::new();
    for col in cols {
        let field = match col.scalar_at(row) {
            Scalar::Null => None,
            Scalar::F64(x) => Some(crate::ast::format_float(x)),
            Scalar::I64(x) => Some(x.to_string()),
            Scalar::Bool(b) => Some(b.to_string()),
            Scalar::Str(s) => Some(s.to_string()),
        };
        match field {
            // A distinct tag for null, so it is one value rather than
            // colliding with the empty string.
            None => key.push(0u8),
            Some(text) => {
                key.push(1u8);
                key.extend_from_slice(&(text.len() as u64).to_le_bytes());
                key.extend_from_slice(text.as_bytes());
            }
        }
    }
    key
}

/// Stack Frames vertically. Every Frame must carry the same column
/// names; the first Frame's order is the result's order.
///
/// Columns are matched by *name*, not position, because two Frames that
/// happen to share a shape but not a meaning is the failure this is most
/// likely to be handed. A missing or extra column is refused and named,
/// rather than filled with nulls: a schema that drifted between chunks is
/// a bug in the producer, and quietly padding it would hide that.
fn concat(frames: &[Value]) -> Result<Value, String> {
    let mut parts = Vec::with_capacity(frames.len());
    for (i, value) in frames.iter().enumerate() {
        parts.push(frame_of(value).ok_or_else(|| {
            format!(
                "ods.concat: item {} must be a Frame, got {}",
                i + 1,
                value.type_name()
            )
        })?);
    }
    let Some((first, rest)) = parts.split_first() else {
        // An empty list carries no schema, so there is no Frame it could
        // honestly produce — not even an empty one.
        return Err("ods.concat: needs at least one Frame".to_string());
    };
    let names = first.names().to_vec();
    for (i, frame) in rest.iter().enumerate() {
        let mut theirs = frame.names().to_vec();
        let mut ours = names.clone();
        theirs.sort();
        ours.sort();
        if theirs != ours {
            return Err(format!(
                "ods.concat: Frame {} has columns [{}] but the first has [{}]",
                i + 2,
                frame.names().join(", "),
                names.join(", ")
            ));
        }
    }

    let mut pairs = Vec::with_capacity(names.len());
    for name in &names {
        // Collect the column from every Frame as language values, which
        // is what lets `series_from_list` apply the same widening rule a
        // literal list gets: an Int column meeting a Float one becomes
        // Float, and anything less compatible is refused there.
        let mut cells = Vec::new();
        for frame in &parts {
            let col = frame.column(name).map_err(e)?;
            for row in 0..col.len() {
                cells.push(scalar_to_value(col.scalar_at(row)));
            }
        }
        let series = series_from_list(&cells)
            .map_err(|err| format!("ods.concat: column '{}': {}", name, err))?;
        pairs.push((name.clone(), series));
    }
    Frame::new(pairs).map(OdsFrame::into_value).map_err(e)
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
                    .map(|name| (name.clone(), infer_column(Vec::<String>::new())))
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

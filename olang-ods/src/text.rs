//! Text columns: the kernels behind a String column's methods.
//!
//! Every kernel works over `StrCol` spans and writes a new `StrCol` (one
//! buffer and a span a row) or a typed column — no `String` per row.
//! What only narrows a cell (`trim`, `substring`, `split_n`) writes no
//! text at all: its spans point into the column's own buffer. A
//! Category's kernels run once per level and gather by code, so lowering
//! a million rows of six neighbourhoods lowers six strings.

use crate::temporal::{self, Format};
use crate::{Bitmap, DType, OdsError, Series, StrCol};
use std::sync::Arc;

type Result<T> = std::result::Result<T, OdsError>;

/// A text column's cells: a String's, or a Category's levels and codes.
enum Text<'a> {
    Str(&'a StrCol),
    Cat(&'a [u32], &'a StrCol),
}

fn text_of<'a>(func: &str, s: &'a Series) -> Result<(Text<'a>, Option<&'a Bitmap>)> {
    match s {
        Series::Str { col, validity } => Ok((Text::Str(col), validity.as_ref())),
        Series::Cat {
            codes,
            levels,
            validity,
        } => Ok((Text::Cat(codes, levels), validity.as_ref())),
        other => Err(OdsError::TypeMismatch(format!(
            "{}: needs a String Series, got a {} Series{}",
            func,
            other.dtype(),
            match other.dtype() {
                DType::Date | DType::Datetime => " (its parts are year(), month(), …; cast(s, \"String\") writes it as text)",
                _ => " (cast(s, \"String\") makes it text)",
            }
        ))),
    }
}

/// The text of `n` cells, `f` writing each (valid) cell's text.
fn build(n: usize, mut cell: impl FnMut(usize, &mut String) -> bool) -> (StrCol, Vec<bool>) {
    let mut buf = String::new();
    let mut spans = Vec::with_capacity(n);
    let mut ok = Vec::with_capacity(n);
    for i in 0..n {
        let start = buf.len();
        let keep = cell(i, &mut buf);
        if !keep {
            buf.truncate(start);
        }
        ok.push(keep);
        spans.push((start as u32, (buf.len() - start) as u32));
    }
    assert!(
        u32::try_from(buf.len()).is_ok(),
        "text column exceeds 4 GB of backing text"
    );
    (StrCol::from_spans(Arc::from(buf), spans), ok)
}

fn validity_from(ok: &[bool]) -> Option<Bitmap> {
    (!ok.iter().all(|&b| b)).then(|| Bitmap::from_bools(ok))
}

/// New text per cell: `f(cell, out)` appends the cell's new text and
/// says whether it has one (false: null). A null stays null.
pub fn transform(func: &str, s: &Series, mut f: impl FnMut(&str, &mut String) -> bool) -> Result<Series> {
    let (t, validity) = text_of(func, s)?;
    let valid = |i: usize| validity.is_none_or(|v| v.get(i));
    match t {
        Text::Str(col) => {
            let (col, ok) = build(col.len(), |i, out| valid(i) && f(col.get(i), out));
            Ok(Series::Str {
                validity: validity_from(&ok),
                col,
            })
        }
        Text::Cat(codes, levels) => {
            // Each level once, then a span a row.
            let (lv, lok) = build(levels.len(), |i, out| f(levels.get(i), out));
            let ls = lv.spans();
            let mut ok = Vec::with_capacity(codes.len());
            let spans: Vec<(u32, u32)> = codes
                .iter()
                .enumerate()
                .map(|(i, &c)| {
                    let keep = valid(i) && lok[c as usize];
                    ok.push(keep);
                    if keep { ls[c as usize] } else { (0, 0) }
                })
                .collect();
            Ok(Series::Str {
                col: StrCol::from_spans(lv.buf_arc(), spans),
                validity: validity_from(&ok),
            })
        }
    }
}

/// Each cell narrowed to the part `f` names (a byte offset and length
/// within it; None: null), its spans into the same text.
fn reslice(func: &str, s: &Series, f: impl Fn(&str) -> Option<(usize, usize)>) -> Result<Series> {
    let (t, validity) = text_of(func, s)?;
    let valid = |i: usize| validity.is_none_or(|v| v.get(i));
    let cut = |col: &StrCol, i: usize| -> Option<(u32, u32)> {
        let (a, _) = col.spans()[i];
        f(col.get(i)).map(|(o, l)| (a + o as u32, l as u32))
    };
    let (col, spans, ok): (&StrCol, Vec<(u32, u32)>, Vec<bool>) = match t {
        Text::Str(col) => {
            let mut ok = Vec::with_capacity(col.len());
            let spans = (0..col.len())
                .map(|i| match valid(i).then(|| cut(col, i)).flatten() {
                    Some(sp) => {
                        ok.push(true);
                        sp
                    }
                    None => {
                        ok.push(false);
                        (0, 0)
                    }
                })
                .collect();
            (col, spans, ok)
        }
        Text::Cat(codes, levels) => {
            let per: Vec<Option<(u32, u32)>> = (0..levels.len()).map(|i| cut(levels, i)).collect();
            let mut ok = Vec::with_capacity(codes.len());
            let spans = codes
                .iter()
                .enumerate()
                .map(|(i, &c)| match valid(i).then(|| per[c as usize]).flatten() {
                    Some(sp) => {
                        ok.push(true);
                        sp
                    }
                    None => {
                        ok.push(false);
                        (0, 0)
                    }
                })
                .collect();
            (levels, spans, ok)
        }
    };
    Ok(Series::Str {
        col: StrCol::from_spans(col.buf_arc(), spans),
        validity: validity_from(&ok),
    })
}

/// A value per cell (a Category's once per level); `None` is null.
fn per_cell<T: Copy + Default>(
    func: &str,
    s: &Series,
    f: impl Fn(&str) -> Option<T>,
) -> Result<(Vec<T>, Option<Bitmap>)> {
    let (t, validity) = text_of(func, s)?;
    let valid = |i: usize| validity.is_none_or(|v| v.get(i));
    let mut ok = Vec::with_capacity(s.len());
    let vals: Vec<T> = match t {
        Text::Str(col) => (0..col.len())
            .map(|i| match valid(i).then(|| f(col.get(i))).flatten() {
                Some(v) => {
                    ok.push(true);
                    v
                }
                None => {
                    ok.push(false);
                    T::default()
                }
            })
            .collect(),
        Text::Cat(codes, levels) => {
            let per: Vec<Option<T>> = levels.iter().map(&f).collect();
            codes
                .iter()
                .enumerate()
                .map(|(i, &c)| match valid(i).then(|| per[c as usize]).flatten() {
                    Some(v) => {
                        ok.push(true);
                        v
                    }
                    None => {
                        ok.push(false);
                        T::default()
                    }
                })
                .collect()
        }
    };
    Ok((vals, validity_from(&ok)))
}

fn mask(func: &str, s: &Series, f: impl Fn(&str) -> bool) -> Result<Series> {
    let (v, validity) = per_cell(func, s, |t| Some(f(t)))?;
    Ok(Series::Bool {
        values: Arc::new(v),
        validity,
    })
}

#[inline]
fn push_ascii_mapped(out: &mut String, t: &str, f: fn(&u8) -> u8) {
    // SAFETY: `t` is ASCII and `f` maps ASCII to ASCII.
    unsafe { out.as_mut_vec() }.extend(t.as_bytes().iter().map(f));
}

pub fn lower(s: &Series) -> Result<Series> {
    transform("lower", s, |t, out| {
        if t.is_ascii() {
            push_ascii_mapped(out, t, u8::to_ascii_lowercase);
        } else {
            out.push_str(&t.to_lowercase());
        }
        true
    })
}

pub fn upper(s: &Series) -> Result<Series> {
    transform("upper", s, |t, out| {
        if t.is_ascii() {
            push_ascii_mapped(out, t, u8::to_ascii_uppercase);
        } else {
            out.push_str(&t.to_uppercase());
        }
        true
    })
}

/// Whitespace off both ends (`start`, `end` choose which).
pub fn trim(s: &Series, start: bool, end: bool) -> Result<Series> {
    let func = match (start, end) {
        (true, true) => "trim",
        (true, false) => "trim_start",
        _ => "trim_end",
    };
    reslice(func, s, |t| {
        let a = if start { t.len() - t.trim_start().len() } else { 0 };
        let rest = &t[a..];
        let l = if end { rest.trim_end().len() } else { rest.len() };
        Some((a, l))
    })
}

/// Every occurrence of `from` replaced by `to`.
pub fn replace(s: &Series, from: &str, to: &str) -> Result<Series> {
    if from.is_empty() {
        return Err(OdsError::InvalidArgument(
            "replace: the text to replace is empty".to_string(),
        ));
    }
    transform("replace", s, |t, out| {
        let mut last = 0;
        for (at, _) in t.match_indices(from) {
            out.push_str(&t[last..at]);
            out.push_str(to);
            last = at + from.len();
        }
        out.push_str(&t[last..]);
        true
    })
}

/// Whether `needle` occurs in `hay`: a scan for its first byte and a
/// compare, which for the short needles and cells of a column beats the
/// setup of a general substring search.
#[inline]
fn has_bytes(hay: &[u8], needle: &[u8]) -> bool {
    let Some((&first, rest)) = needle.split_first() else {
        return true;
    };
    if hay.len() < needle.len() {
        return false;
    }
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        match hay[i..=last].iter().position(|&b| b == first) {
            None => return false,
            Some(p) => {
                i += p;
                if &hay[i + 1..i + needle.len()] == rest {
                    return true;
                }
                i += 1;
            }
        }
    }
    false
}

/// Whether `pat` occurs in each cell (`case`: false ignores case).
pub fn contains(s: &Series, pat: &str, case: bool) -> Result<Series> {
    if case {
        if pat.len() <= 32 {
            let p = pat.as_bytes();
            return mask("contains", s, move |t| has_bytes(t.as_bytes(), p));
        }
        return mask("contains", s, |t| t.contains(pat));
    }
    if pat.is_ascii() {
        let p = pat.as_bytes();
        return mask("contains", s, move |t| {
            p.is_empty()
                || t.as_bytes()
                    .windows(p.len())
                    .any(|w| w.eq_ignore_ascii_case(p))
        });
    }
    let p = pat.to_lowercase();
    mask("contains", s, move |t| t.to_lowercase().contains(&p))
}

pub fn starts_with(s: &Series, pat: &str) -> Result<Series> {
    mask("starts_with", s, |t| t.starts_with(pat))
}

pub fn ends_with(s: &Series, pat: &str) -> Result<Series> {
    mask("ends_with", s, |t| t.ends_with(pat))
}

/// Each cell's length in characters.
pub fn len_chars(s: &Series) -> Result<Series> {
    let (v, validity) = per_cell("len_chars", s, |t| {
        Some(if t.is_ascii() {
            t.len() as i64
        } else {
            t.chars().count() as i64
        })
    })?;
    Ok(Series::I64 {
        values: Arc::new(v),
        validity,
    })
}

/// The byte offset of character `k` (or the end).
fn char_byte(t: &str, k: usize) -> usize {
    if t.is_ascii() {
        return k.min(t.len());
    }
    t.char_indices().nth(k).map(|(b, _)| b).unwrap_or(t.len())
}

/// Characters `a` up to `b` (to the end when None): `str.substring`'s
/// rule, a position past the end stopping there.
pub fn substring(s: &Series, a: usize, b: Option<usize>) -> Result<Series> {
    reslice("substring", s, |t| {
        let start = char_byte(t, a);
        let end = match b {
            Some(b) if b <= a => start,
            Some(b) => char_byte(t, b),
            None => t.len(),
        };
        Some((start, end - start))
    })
}

/// Piece `i` of each cell split at `sep` (a negative `i` counts from
/// the end); null where there is no such piece.
pub fn split_n(s: &Series, sep: &str, i: i64) -> Result<Series> {
    if sep.is_empty() {
        return Err(OdsError::InvalidArgument(
            "split_n: the separator is empty".to_string(),
        ));
    }
    reslice("split_n", s, |t| {
        let base = t.as_ptr() as usize;
        let piece = if i >= 0 {
            t.split(sep).nth(i as usize)
        } else {
            t.rsplit(sep).nth((-i - 1) as usize)
        }?;
        Some((piece.as_ptr() as usize - base, piece.len()))
    })
}

/// Padded to `width` characters with `fill` repeated, on the left
/// (`start`) or right; a longer cell is left as it is.
pub fn pad(s: &Series, width: usize, fill: &str, start: bool) -> Result<Series> {
    let func = if start { "pad_start" } else { "pad_end" };
    if fill.is_empty() {
        return Err(OdsError::InvalidArgument(format!("{}: the fill is empty", func)));
    }
    transform(func, s, |t, out| {
        let have = if t.is_ascii() { t.len() } else { t.chars().count() };
        let pad = |out: &mut String| {
            for c in fill.chars().cycle().take(width.saturating_sub(have)) {
                out.push(c);
            }
        };
        if start {
            pad(out);
            out.push_str(t);
        } else {
            out.push_str(t);
            pad(out);
        }
        true
    })
}

/// Each cell read as an Int (surrounding space ignored); null where it
/// is not one.
pub fn to_int(s: &Series) -> Result<Series> {
    let (v, validity) = per_cell("to_int", s, |t| t.trim().parse::<i64>().ok())?;
    Ok(Series::I64 {
        values: Arc::new(v),
        validity,
    })
}

/// Each cell read as a Float; null where it is not one.
pub fn to_float(s: &Series) -> Result<Series> {
    let (v, validity) = per_cell("to_float", s, |t| t.trim().parse::<f64>().ok())?;
    Ok(Series::F64 {
        values: Arc::new(v),
        validity,
    })
}

/// Each cell read as a Datetime: ISO 8601 without a format, else the
/// format's; null where it does not read.
pub fn to_datetime(s: &Series, fmt: Option<&str>) -> Result<Series> {
    let f = fmt.map(Format::new).transpose().map_err(|e| {
        OdsError::InvalidArgument(format!("to_datetime: {}", e))
    })?;
    let (v, validity) = per_cell("to_datetime", s, |t| match &f {
        None => temporal::parse_iso(t.trim()).map(|(us, _)| us),
        Some(f) => f.parse(t.trim()),
    })?;
    Ok(Series::Datetime {
        values: Arc::new(v),
        validity,
    })
}

/// Each cell read as a Date (a datetime's day): ISO 8601 without a
/// format, else the format's; null where it does not read.
pub fn to_date(s: &Series, fmt: Option<&str>) -> Result<Series> {
    let f = fmt
        .map(Format::new)
        .transpose()
        .map_err(|e| OdsError::InvalidArgument(format!("to_date: {}", e)))?;
    let (v, validity) = per_cell("to_date", s, |t| {
        let t = t.trim();
        let day = match &f {
            None => temporal::parse_iso_date(t)
                .or_else(|| temporal::parse_iso(t).map(|(us, _)| temporal::day_of_us(us))),
            Some(f) => f.parse(t).map(temporal::day_of_us),
        }?;
        i32::try_from(day).ok()
    })?;
    Ok(Series::Date {
        values: Arc::new(v),
        validity,
    })
}

/// A Date or Datetime written in a format (`strftime`'s directives).
pub fn format(s: &Series, fmt: &str) -> Result<Series> {
    if matches!(s, Series::F64 { .. } | Series::I64 { .. }) {
        let spec = crate::numfmt::Spec::parse(fmt)
            .map_err(|e| OdsError::InvalidArgument(format!("format: {}", e)))?;
        return format_numbers(s, &spec);
    }
    let f = Format::new(fmt).map_err(|e| OdsError::InvalidArgument(format!("format: {}", e)))?;
    let validity = s.validity();
    let valid = |i: usize| validity.is_none_or(|v| v.get(i));
    let us_at: Box<dyn Fn(usize) -> i64 + '_> = match s {
        Series::Date { values, .. } => Box::new(|i| values[i] as i64 * temporal::US_PER_DAY),
        Series::Datetime { values, .. } => Box::new(|i| values[i]),
        other => {
            return Err(OdsError::TypeMismatch(format!(
                "format: needs a Date or Datetime column, got {}",
                other.dtype()
            )));
        }
    };
    let (col, ok) = build(s.len(), |i, out| {
        if !valid(i) {
            return false;
        }
        out.push_str(&f.format(us_at(i)));
        true
    });
    Ok(Series::Str {
        col,
        validity: validity_from(&ok),
    })
}

/// Whether every non-null cell of a text column reads as an ISO date
/// (`Some(Date)`) or datetime (`Some(Datetime)`): what `dates: "auto"`
/// asks of a column. An all-null column is neither.
pub fn iso_kind(s: &Series) -> Option<DType> {
    let Series::Str { col, validity } = s else {
        return None;
    };
    let mut any = false;
    let mut time = false;
    for i in 0..col.len() {
        if validity.as_ref().is_some_and(|v| !v.get(i)) {
            continue;
        }
        let t = col.get(i).trim();
        if temporal::parse_iso_date(t).is_some() {
            any = true;
            continue;
        }
        match temporal::parse_iso(t) {
            Some((_, has_time)) => {
                any = true;
                time |= has_time;
            }
            None => return None,
        }
    }
    any.then_some(if time { DType::Datetime } else { DType::Date })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scalar;

    fn texts(s: &Series) -> Vec<Option<String>> {
        (0..s.len())
            .map(|i| match s.scalar_at(i) {
                Scalar::Str(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn kernels_keep_nulls_and_spans() {
        let s = Series::from_str_options(vec![Some(" Old Town ".into()), None, Some("ÉTÉ x".into())]);
        assert_eq!(
            texts(&lower(&trim(&s, true, true).unwrap()).unwrap()),
            vec![Some("old town".into()), None, Some("été x".into())]
        );
        assert_eq!(
            texts(&split_n(&s, " ", -1).unwrap()),
            vec![Some("".into()), None, Some("x".into())]
        );
        assert_eq!(texts(&substring(&s, 1, Some(4)).unwrap())[2], Some("TÉ ".into()));
        assert_eq!(
            texts(&pad(&Series::from_str_values(vec!["7".into()]), 3, "0", true).unwrap()),
            vec![Some("007".into())]
        );
        let m = contains(&s, "old", false).unwrap();
        assert_eq!(m.scalar_at(0), Scalar::Bool(true));
        assert_eq!(m.scalar_at(1), Scalar::Null);
        let d = to_date(&Series::from_str_values(vec!["2024-03-01".into(), "x".into()]), None).unwrap();
        assert_eq!(d.scalar_at(0), Scalar::Date(19783));
        assert_eq!(d.scalar_at(1), Scalar::Null);
    }
}

/// A number column as text by a format spec (`crate::numfmt`): one pass,
/// one text buffer. A null stays null.
pub fn format_numbers(s: &Series, spec: &crate::numfmt::Spec) -> Result<Series> {
    let validity = s.validity();
    let valid = |i: usize| validity.is_none_or(|v| v.get(i));
    let (col, ok) = match s {
        Series::F64 { values, .. } => build(s.len(), |i, out| {
            if !valid(i) {
                return false;
            }
            spec.format_into(values[i], out);
            true
        }),
        Series::I64 { values, .. } => build(s.len(), |i, out| {
            if !valid(i) {
                return false;
            }
            spec.format_i64(values[i], out);
            true
        }),
        other => {
            return Err(OdsError::TypeMismatch(format!(
                "format: needs a number column, got {}",
                other.dtype()
            )));
        }
    };
    Ok(Series::Str {
        col,
        validity: validity_from(&ok),
    })
}

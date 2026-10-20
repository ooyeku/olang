//! Categories: a column of `u32` codes into an ordered list of text
//! levels (`Series::Cat`), and what is built from them — indicator
//! columns (`dummies`) and the pairs of two columns (`interact`).
//!
//! A category's levels are its order: sorted text (numbers by value,
//! dates by date) unless the levels are given, in which case they are
//! the given order. Sorting, `<` and `min`/`max` follow it; equality is
//! by text. Every kernel works on codes: a value's level is looked up
//! once per distinct value, never per row.

use crate::frame::{FxMap, group_ids};
use crate::{Bitmap, CmpOp, DType, OdsError, Scalar, Series, StrCol};
use std::sync::Arc;

type Result<T> = std::result::Result<T, OdsError>;

/// Whether two level lists are the same list (shared, or equal text).
pub fn same_levels(a: &StrCol, b: &StrCol) -> bool {
    (Arc::ptr_eq(&a.buf_arc(), &b.buf_arc()) && a.spans() == b.spans())
        || (a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y))
}

/// `codes ⋈ text` as a mask: equality by text (a text that is no level
/// equals nothing), order by the levels' (where `text` must be one).
pub fn compare_codes(codes: &[u32], levels: &StrCol, text: &str, op: CmpOp) -> Result<Vec<bool>> {
    let pos = levels.iter().position(|l| l == text).map(|p| p as u32);
    Ok(match (op, pos) {
        (CmpOp::Eq, None) => vec![false; codes.len()],
        (CmpOp::Ne, None) => vec![true; codes.len()],
        (_, None) => {
            return Err(OdsError::InvalidArgument(format!(
                "\"{}\" is not a level of this Category, so it has no place in its order \
                 (levels: {})",
                text,
                preview(levels)
            )));
        }
        (op, Some(p)) => codes
            .iter()
            .map(|&c| match op {
                CmpOp::Eq => c == p,
                CmpOp::Ne => c != p,
                CmpOp::Lt => c < p,
                CmpOp::Le => c <= p,
                CmpOp::Gt => c > p,
                CmpOp::Ge => c >= p,
            })
            .collect(),
    })
}

fn preview(levels: &StrCol) -> String {
    let mut out: Vec<String> = levels.iter().take(8).map(|l| format!("{:?}", l)).collect();
    if levels.len() > 8 {
        out.push(format!("… {} in all", levels.len()));
    }
    out.join(", ")
}

/// A value's text as a category level.
fn level_text(v: Scalar, fmt_f64: &dyn Fn(f64) -> String) -> String {
    match v {
        Scalar::Str(s) => s,
        Scalar::I64(x) => x.to_string(),
        Scalar::F64(x) => fmt_f64(x),
        Scalar::Bool(b) => b.to_string(),
        Scalar::Date(d) => crate::temporal::format_date(d as i64),
        Scalar::Datetime(t) => crate::temporal::format_datetime(t),
        Scalar::Null => String::new(),
    }
}

/// `s` as a Category. Without `levels`, its distinct values in their
/// natural order (text, numbers, dates, `false` before `true`); with
/// them, those levels in that order, and a value that is none of them
/// null. A Category given levels is re-levelled.
pub fn to_category(
    s: &Series,
    levels: Option<&[String]>,
    fmt_f64: &dyn Fn(f64) -> String,
) -> Result<Series> {
    if let Some(given) = levels {
        let mut index: FxMap<&str, u32> = FxMap::default();
        for (i, l) in given.iter().enumerate() {
            if index.insert(l.as_str(), i as u32).is_some() {
                return Err(OdsError::InvalidArgument(format!(
                    "to_category: the level \"{}\" is given twice",
                    l
                )));
            }
        }
        // Each distinct value is looked up once.
        let (ids, n_groups) = group_ids(&[s])?;
        let first = first_rows(&ids, n_groups);
        let map: Vec<Option<u32>> = first
            .iter()
            .map(|&r| match s.scalar_at(r) {
                Scalar::Null => None,
                v => index.get(level_text(v, fmt_f64).as_str()).copied(),
            })
            .collect();
        return Ok(from_group_map(&ids, &map, StrCol::from_strings(given.iter())));
    }
    if let Series::Cat { .. } = s {
        return Ok(s.clone());
    }
    let (ids, n_groups) = group_ids(&[s])?;
    let first = first_rows(&ids, n_groups);
    // The distinct values (nulls aside), in the column's own order.
    let mut groups: Vec<usize> = (0..n_groups)
        .filter(|&g| !matches!(s.scalar_at(first[g]), Scalar::Null))
        .collect();
    let distinct = s.gather(&groups.iter().map(|&g| first[g]).collect::<Vec<_>>());
    let order: Vec<usize> = match distinct.dtype() {
        DType::Bool => {
            let mut o: Vec<usize> = (0..distinct.len()).collect();
            o.sort_by_key(|&i| matches!(distinct.scalar_at(i), Scalar::Bool(true)));
            o
        }
        _ => match distinct.argsort()? {
            Series::I64 { values, .. } => values.iter().map(|&i| i as usize).collect(),
            _ => unreachable!("argsort gives Int indices"),
        },
    };
    groups = order.iter().map(|&i| groups[i]).collect();
    let mut map: Vec<Option<u32>> = vec![None; n_groups];
    for (code, &g) in groups.iter().enumerate() {
        map[g] = Some(code as u32);
    }
    let levels = StrCol::from_strings(
        groups
            .iter()
            .map(|&g| level_text(s.scalar_at(first[g]), fmt_f64)),
    );
    Ok(from_group_map(&ids, &map, levels))
}

fn first_rows(ids: &[u32], n_groups: usize) -> Vec<usize> {
    let mut first = vec![usize::MAX; n_groups];
    for (row, &g) in ids.iter().enumerate() {
        let f = &mut first[g as usize];
        if *f == usize::MAX {
            *f = row;
        }
    }
    first
}

/// Codes from each row's group through `map` (None: null).
fn from_group_map(ids: &[u32], map: &[Option<u32>], levels: StrCol) -> Series {
    let mut codes = Vec::with_capacity(ids.len());
    let mut any_null = false;
    for &g in ids {
        match map[g as usize] {
            Some(c) => codes.push(c),
            None => {
                any_null = true;
                codes.push(u32::MAX);
            }
        }
    }
    let validity = any_null.then(|| {
        Bitmap::from_bools(&codes.iter().map(|&c| c != u32::MAX).collect::<Vec<_>>())
    });
    if any_null {
        for c in codes.iter_mut() {
            if *c == u32::MAX {
                *c = 0;
            }
        }
    }
    Series::category(codes, levels, validity)
}

/// A Category's levels, in order, as a String column.
pub fn levels(s: &Series) -> Result<Series> {
    match s {
        Series::Cat { levels, .. } => Ok(Series::Str {
            col: levels.clone(),
            validity: None,
        }),
        _ => Err(not_category("levels", s)),
    }
}

/// A Category's codes (0 for its first level) as an Int column; a null
/// stays null.
pub fn codes(s: &Series) -> Result<Series> {
    match s {
        Series::Cat {
            codes, validity, ..
        } => Ok(Series::I64 {
            values: Arc::new(codes.iter().map(|&c| c as i64).collect()),
            validity: validity.clone(),
        }),
        _ => Err(not_category("codes", s)),
    }
}

fn not_category(what: &str, s: &Series) -> OdsError {
    OdsError::TypeMismatch(format!(
        "{}: needs a Category column, got {} (make one with to_category())",
        what,
        s.dtype()
    ))
}

/// The indicator columns of a column: one per level (its first left
/// out when `drop_first`), named `prefix_level` (the level alone with
/// no prefix), 1 where the row holds that level. A null row is 0 in
/// every column. `dtype` is Float, Int or Bool.
pub fn dummies(
    s: &Series,
    prefix: Option<&str>,
    drop_first: bool,
    dtype: DType,
    fmt_f64: &dyn Fn(f64) -> String,
) -> Result<Vec<(String, Series)>> {
    if !matches!(dtype, DType::F64 | DType::I64 | DType::Bool) {
        return Err(OdsError::InvalidArgument(format!(
            "dummies: dtype is \"Float\", \"Int\" or \"Bool\", got {}",
            dtype
        )));
    }
    let cat = to_category(s, None, fmt_f64)?;
    let Series::Cat {
        codes,
        levels,
        validity,
    } = &cat
    else {
        unreachable!("to_category gives a Category")
    };
    let k = levels.len();
    let skip = usize::from(drop_first && k > 0);
    let valid = |i: usize| validity.as_ref().is_none_or(|v| v.get(i));
    // Every column zeroed (pages the allocator hands over zeroed), then
    // one pass over the codes sets each row's one.
    fn scatter<T: Copy + Default>(
        codes: &[u32],
        k: usize,
        skip: usize,
        one: T,
        valid: impl Fn(usize) -> bool,
    ) -> Vec<Vec<T>> {
        let mut cols: Vec<Vec<T>> = (skip..k).map(|_| vec![T::default(); codes.len()]).collect();
        for (i, &c) in codes.iter().enumerate() {
            let c = c as usize;
            if c >= skip && valid(i) {
                cols[c - skip][i] = one;
            }
        }
        cols
    }
    let cols: Vec<Series> = match dtype {
        DType::F64 => scatter(codes, k, skip, 1.0f64, valid)
            .into_iter()
            .map(Series::from_f64)
            .collect(),
        DType::I64 => scatter(codes, k, skip, 1i64, valid)
            .into_iter()
            .map(Series::from_i64)
            .collect(),
        _ => scatter(codes, k, skip, true, valid)
            .into_iter()
            .map(Series::from_bool)
            .collect(),
    };
    Ok(levels
        .iter()
        .skip(skip)
        .zip(cols)
        .map(|(level, col)| {
            let name = match prefix {
                Some(p) => format!("{}_{}", p, level),
                None => level.to_string(),
            };
            (name, col)
        })
        .collect())
}

/// Each row's pair `a:b` as a Category whose levels are the pairs that
/// occur, `a`'s order first and `b`'s within it. A null on either side
/// is null.
pub fn interact(a: &Series, b: &Series, sep: &str, fmt_f64: &dyn Fn(f64) -> String) -> Result<Series> {
    if a.len() != b.len() {
        return Err(OdsError::LengthMismatch {
            left: a.len(),
            right: b.len(),
        });
    }
    let ca = to_category(a, None, fmt_f64)?;
    let cb = to_category(b, None, fmt_f64)?;
    let (
        Series::Cat {
            codes: xa,
            levels: la,
            ..
        },
        Series::Cat {
            codes: xb,
            levels: lb,
            ..
        },
    ) = (&ca, &cb)
    else {
        unreachable!("to_category gives a Category")
    };
    let valid = |i: usize| {
        ca.validity().is_none_or(|v| v.get(i)) && cb.validity().is_none_or(|v| v.get(i))
    };
    let nb = lb.len().max(1) as u64;
    let n = a.len();
    let mut seen: Vec<u64> = (0..n)
        .filter(|&i| valid(i))
        .map(|i| xa[i] as u64 * nb + xb[i] as u64)
        .collect();
    seen.sort_unstable();
    seen.dedup();
    let index: FxMap<u64, u32> = seen.iter().enumerate().map(|(i, &p)| (p, i as u32)).collect();
    let levels = StrCol::from_strings(seen.iter().map(|&p| {
        format!(
            "{}{}{}",
            la.get((p / nb) as usize),
            sep,
            lb.get((p % nb) as usize)
        )
    }));
    let mut any_null = false;
    let codes: Vec<u32> = (0..n)
        .map(|i| {
            if valid(i) {
                index[&(xa[i] as u64 * nb + xb[i] as u64)]
            } else {
                any_null = true;
                0
            }
        })
        .collect();
    let validity =
        any_null.then(|| Bitmap::from_bools(&(0..n).map(valid).collect::<Vec<_>>()));
    Ok(Series::category(codes, levels, validity))
}

/// Categories laid end to end: the first's levels, then each new level
/// of the next ones in their order.
pub fn stack(parts: &[&Series]) -> Result<Series> {
    let mut levels: Vec<String> = Vec::new();
    let mut index: FxMap<String, u32> = FxMap::default();
    let mut codes = Vec::new();
    let mut valid = Vec::new();
    for p in parts {
        let Series::Cat {
            codes: c,
            levels: l,
            validity,
        } = p
        else {
            return Err(not_category("stacking categories", p));
        };
        let remap: Vec<u32> = l
            .iter()
            .map(|t| {
                *index.entry(t.to_string()).or_insert_with(|| {
                    levels.push(t.to_string());
                    (levels.len() - 1) as u32
                })
            })
            .collect();
        for (i, &x) in c.iter().enumerate() {
            let ok = validity.as_ref().is_none_or(|v| v.get(i));
            codes.push(if ok { remap[x as usize] } else { 0 });
            valid.push(ok);
        }
    }
    let validity = (!valid.iter().all(|&v| v)).then(|| Bitmap::from_bools(&valid));
    Ok(Series::category(codes, StrCol::from_strings(levels), validity))
}

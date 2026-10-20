//! Column kernels that are not elementwise: constructors, positions,
//! differences and running products, fills that look along the column,
//! the mode, membership answers and recoding. Elementwise work (math,
//! conditionals, `coalesce`, `clip`, …) is the expression engine's
//! (`fuse`); these are the kernels around it.

use crate::fuse::{BinOp, Expr, Math, Op};
use crate::{Bitmap, OdsError, Scalar, Series, StrCol};
use std::collections::HashMap;
use std::sync::Arc;

type Result<T> = std::result::Result<T, OdsError>;

/// What `recode` gives a value its map does not name.
#[derive(Clone, Debug)]
pub enum Unmapped {
    /// The value itself (the default).
    Keep,
    /// Null.
    Null,
    /// One value for every unmapped position.
    Value(Scalar),
}

/// `n` copies of `v`; a null `v` is a Float column of nulls.
pub fn full(n: usize, v: &Scalar) -> Series {
    match v {
        Scalar::F64(x) => Series::from_f64(vec![*x; n]),
        Scalar::I64(x) => Series::from_i64(vec![*x; n]),
        Scalar::Bool(x) => Series::from_bool(vec![*x; n]),
        Scalar::Str(t) => {
            // One copy of the text, every span pointing at it.
            let len = u32::try_from(t.len()).expect("a cell under 4 GB");
            Series::Str {
                col: StrCol::from_spans(Arc::from(t.as_str()), vec![(0, len); n]),
                validity: None,
            }
        }
        Scalar::Null => Series::from_f64_options(vec![None; n]),
    }
}

/// `start, start + step, …` up to but excluding `stop`, as Ints.
pub fn arange_i64(start: i64, stop: i64, step: i64) -> Result<Series> {
    if step == 0 {
        return Err(OdsError::InvalidArgument(
            "arange: the step cannot be 0".to_string(),
        ));
    }
    let mut out = Vec::new();
    let mut x = start;
    while (step > 0 && x < stop) || (step < 0 && x > stop) {
        out.push(x);
        match x.checked_add(step) {
            Some(y) => x = y,
            None => break,
        }
    }
    Ok(Series::from_i64(out))
}

/// The Float form of [`arange_i64`]: element `i` is `start + i * step`
/// (not a running sum, so the last element carries no drift).
pub fn arange_f64(start: f64, stop: f64, step: f64) -> Result<Series> {
    if step == 0.0 || !step.is_finite() || !start.is_finite() || !stop.is_finite() {
        return Err(OdsError::InvalidArgument(
            "arange: start, stop and step must be finite, and the step not 0".to_string(),
        ));
    }
    let n = ((stop - start) / step).ceil().max(0.0);
    if n > 1e10 {
        return Err(OdsError::InvalidArgument(format!(
            "arange: {} elements is too many",
            n
        )));
    }
    let n = n as usize;
    Ok(Series::from_f64(
        (0..n).map(|i| start + i as f64 * step).collect(),
    ))
}

fn gather_validity(s: &Series, idx: &[usize]) -> Option<Bitmap> {
    s.validity().map(|v| {
        let bits: Vec<bool> = idx.iter().map(|&i| v.get(i)).collect();
        Bitmap::from_bools(&bits)
    })
}

/// A gather that keeps a fully valid column bitmap-free.
fn gather_all(s: &Series, idx: &[usize]) -> Series {
    let validity = gather_validity(s, idx).filter(|b| !b.all_valid());
    match s {
        Series::F64 { values, .. } => Series::F64 {
            values: Arc::new(idx.iter().map(|&i| values[i]).collect()),
            validity,
        },
        Series::I64 { values, .. } => Series::I64 {
            values: Arc::new(idx.iter().map(|&i| values[i]).collect()),
            validity,
        },
        Series::Bool { values, .. } => Series::Bool {
            values: Arc::new(idx.iter().map(|&i| values[i]).collect()),
            validity,
        },
        Series::Str { col, .. } => Series::Str {
            col: col.gather(idx),
            validity,
        },
    }
}

impl Series {
    /// A column of this one's length holding `v`.
    pub fn full_like(&self, v: &Scalar) -> Series {
        full(self.len(), v)
    }

    /// Each element `times` times in a row (`[a, b]` → `[a, a, b, b]`).
    pub fn repeat(&self, times: usize) -> Series {
        let idx: Vec<usize> = (0..self.len())
            .flat_map(|i| std::iter::repeat_n(i, times))
            .collect();
        gather_all(self, &idx)
    }

    /// Each element as many times as `counts` says, position by
    /// position.
    pub fn repeat_each(&self, counts: &[i64]) -> Result<Series> {
        if counts.len() != self.len() {
            return Err(OdsError::LengthMismatch {
                left: self.len(),
                right: counts.len(),
            });
        }
        if let Some(c) = counts.iter().find(|&&c| c < 0) {
            return Err(OdsError::InvalidArgument(format!(
                "repeat: a count cannot be negative, got {}",
                c
            )));
        }
        let idx: Vec<usize> = counts
            .iter()
            .enumerate()
            .flat_map(|(i, &c)| std::iter::repeat_n(i, c as usize))
            .collect();
        Ok(gather_all(self, &idx))
    }

    /// The whole column `n` times end to end (`[a, b]` → `[a, b, a, b]`).
    pub fn tile(&self, n: usize) -> Series {
        let len = self.len();
        let idx: Vec<usize> = (0..n).flat_map(|_| 0..len).collect();
        gather_all(self, &idx)
    }

    /// Valid positions.
    pub fn count(&self) -> usize {
        self.len() - self.null_count()
    }

    /// The first element, null included (Null for an empty column).
    pub fn first(&self) -> Scalar {
        if self.is_empty() {
            Scalar::Null
        } else {
            self.scalar_at(0)
        }
    }

    /// The last element, null included.
    pub fn last(&self) -> Scalar {
        if self.is_empty() {
            Scalar::Null
        } else {
            self.scalar_at(self.len() - 1)
        }
    }

    /// The position of the largest (or smallest) valid value; the first
    /// such position on a tie. NaN is skipped, as a null is. None when
    /// nothing is valid.
    pub fn arg_extreme(&self, want_max: bool) -> Result<Option<usize>> {
        let valid = |i: usize| self.validity().is_none_or(|v| v.get(i));
        Ok(match self {
            Series::F64 { values, .. } => {
                let mut best: Option<usize> = None;
                for (i, &x) in values.iter().enumerate() {
                    if !valid(i) || x.is_nan() {
                        continue;
                    }
                    best = match best {
                        Some(b) if (want_max && x > values[b]) || (!want_max && x < values[b]) => {
                            Some(i)
                        }
                        None => Some(i),
                        keep => keep,
                    };
                }
                best
            }
            Series::I64 { values, .. } => {
                let mut best: Option<usize> = None;
                for (i, &x) in values.iter().enumerate() {
                    if !valid(i) {
                        continue;
                    }
                    best = match best {
                        Some(b) if (want_max && x > values[b]) || (!want_max && x < values[b]) => {
                            Some(i)
                        }
                        None => Some(i),
                        keep => keep,
                    };
                }
                best
            }
            Series::Str { col, .. } => {
                let mut best: Option<usize> = None;
                for i in 0..col.len() {
                    if !valid(i) {
                        continue;
                    }
                    let x = col.get(i);
                    best = match best {
                        Some(b) if (want_max && x > col.get(b)) || (!want_max && x < col.get(b)) => {
                            Some(i)
                        }
                        None => Some(i),
                        keep => keep,
                    };
                }
                best
            }
            Series::Bool { values, .. } => {
                // true is the larger: argmax is the first true.
                (0..values.len())
                    .filter(|&i| valid(i))
                    .find(|&i| values[i] == want_max)
                    .or_else(|| (0..values.len()).find(|&i| valid(i)))
            }
        })
    }

    /// Indices that sort the column, stable, nulls last in their order;
    /// `descending` reverses the order of the values (ties still keep
    /// their row order).
    pub fn argsort_by(&self, descending: bool) -> Result<Series> {
        if !descending {
            return self.argsort();
        }
        if let Some(order) = self.packed_order(true) {
            return Ok(Series::from_i64(order.into_iter().map(|i| i as i64).collect()));
        }
        let n = self.len();
        let valid = |i: usize| self.validity().is_none_or(|v| v.get(i));
        let (mut idx, nulls): (Vec<usize>, Vec<usize>) = (0..n).partition(|&i| valid(i));
        let par = crate::par_worth(crate::ParClass::Sort, n);
        macro_rules! sort {
            ($cmp:expr) => {{
                #[cfg(feature = "parallel")]
                {
                    use rayon::prelude::*;
                    if par {
                        idx.par_sort_by($cmp);
                    } else {
                        idx.sort_by($cmp);
                    }
                }
                #[cfg(not(feature = "parallel"))]
                {
                    let _ = par;
                    idx.sort_by($cmp);
                }
            }};
        }
        match self {
            Series::F64 { values, .. } => sort!(|&a, &b| values[b].total_cmp(&values[a])),
            Series::I64 { values, .. } => sort!(|&a, &b| values[b].cmp(&values[a])),
            Series::Str { col, .. } => sort!(|&a, &b| col.get(b).cmp(col.get(a))),
            Series::Bool { .. } => {
                return Err(OdsError::TypeMismatch(
                    "argsort is not defined for Bool series".to_string(),
                ));
            }
        }
        idx.extend(nulls);
        Ok(Series::from_i64(idx.into_iter().map(|i| i as i64).collect()))
    }

    /// The sorted column; `descending` puts the largest first. Nulls go
    /// last either way.
    pub fn sort_by_dir(&self, descending: bool, par: bool) -> Result<Series> {
        let sorted = self.sort(par)?;
        if !descending {
            return Ok(sorted);
        }
        let valid = sorted.len() - sorted.null_count();
        let mut idx: Vec<usize> = (0..valid).rev().collect();
        idx.extend(valid..sorted.len());
        Ok(gather_all(&sorted, &idx))
    }

    /// `x[i] - x[i - lag]` (a negative `lag` looks ahead); the first
    /// `lag` positions are null. Int stays Int.
    pub fn diff(&self, lag: i64) -> Result<Series> {
        let prev = self.shift(lag)?;
        Expr::bin(BinOp::Sub, Expr::col(self.clone()), Expr::col(prev)).eval(self.len(), false)
    }

    /// `x[i] / x[i - lag] - 1`, as a Float. A zero divisor at a valid
    /// position raises, as `/` does.
    pub fn pct_change(&self, lag: i64) -> Result<Series> {
        let prev = self.shift(lag)?;
        Expr::bin(
            BinOp::Sub,
            Expr::bin(BinOp::Div, Expr::col(self.clone()), Expr::col(prev)),
            Expr::lit(Scalar::F64(1.0)),
        )
        .eval(self.len(), false)
    }

    /// The running product over valid positions; nulls stay null and
    /// the product carries across them. Int is checked.
    pub fn cumprod(&self) -> Result<Series> {
        let valid = |i: usize| self.validity().is_none_or(|v| v.get(i));
        match self {
            Series::F64 { values, validity } => {
                let mut acc = 1.0;
                let out: Vec<f64> = values
                    .iter()
                    .enumerate()
                    .map(|(i, &x)| {
                        if valid(i) {
                            acc *= x;
                        }
                        acc
                    })
                    .collect();
                Ok(Series::F64 {
                    values: Arc::new(out),
                    validity: validity.clone(),
                })
            }
            Series::I64 { values, validity } => {
                let mut acc: i64 = 1;
                let mut out = Vec::with_capacity(values.len());
                for (i, &x) in values.iter().enumerate() {
                    if valid(i) {
                        acc = acc
                            .checked_mul(x)
                            .ok_or(OdsError::IntegerOverflow("multiplication"))?;
                    }
                    out.push(acc);
                }
                Ok(Series::I64 {
                    values: Arc::new(out),
                    validity: validity.clone(),
                })
            }
            _ => Err(OdsError::TypeMismatch(format!(
                "cumprod is not defined for {} series",
                self.dtype()
            ))),
        }
    }

    /// The most frequent valid value; on a tie, the smallest. Null for a
    /// column with no valid value. One pass of the dictionary encoding
    /// `group_by` uses, then a count per distinct value.
    pub fn mode(&self) -> Result<Scalar> {
        let (ids, n_groups) = crate::frame::group_ids(&[self])?;
        let valid = |i: usize| self.validity().is_none_or(|v| v.get(i));
        let mut counts = vec![0usize; n_groups];
        let mut first = vec![usize::MAX; n_groups];
        for (i, &g) in ids.iter().enumerate() {
            if valid(i) {
                let g = g as usize;
                counts[g] += 1;
                if first[g] == usize::MAX {
                    first[g] = i;
                }
            }
        }
        let mut best: Option<usize> = None;
        for g in 0..n_groups {
            if counts[g] == 0 {
                continue;
            }
            best = match best {
                None => Some(g),
                Some(b) => {
                    let (vb, vg) = (self.scalar_at(first[b]), self.scalar_at(first[g]));
                    let smaller = match (&vg, &vb) {
                        (Scalar::F64(x), Scalar::F64(y)) => x.total_cmp(y).is_lt(),
                        (Scalar::I64(x), Scalar::I64(y)) => x < y,
                        (Scalar::Str(x), Scalar::Str(y)) => x < y,
                        (Scalar::Bool(x), Scalar::Bool(y)) => x < y,
                        _ => false,
                    };
                    if counts[g] > counts[b] || (counts[g] == counts[b] && smaller) {
                        Some(g)
                    } else {
                        Some(b)
                    }
                }
            };
        }
        Ok(best.map(|g| self.scalar_at(first[g])).unwrap_or(Scalar::Null))
    }

    /// Whether any valid element of a Bool column is true (`all`: none
    /// is false). Nulls are skipped, so an all-null column is `any`
    /// false and `all` true, as an empty one is.
    pub fn any_all(&self, all: bool) -> Result<bool> {
        let Series::Bool { values, validity } = self else {
            return Err(OdsError::TypeMismatch(format!(
                "{} needs a Bool mask, got a {} series",
                if all { "all" } else { "any" },
                self.dtype()
            )));
        };
        let valid = |i: usize| validity.as_ref().is_none_or(|v| v.get(i));
        Ok(if all {
            !(0..values.len()).any(|i| valid(i) && !values[i])
        } else {
            (0..values.len()).any(|i| valid(i) && values[i])
        })
    }

    /// Where `v` would go in this sorted column to keep it sorted: the
    /// first position whose value is not less than `v` (`right`: greater
    /// than). The column's nulls, last after a sort, are not searched.
    pub fn searchsorted(&self, v: &Scalar, right: bool) -> Result<usize> {
        let n = self.len() - self.null_count();
        let pos = |less: &dyn Fn(usize) -> bool| -> usize {
            // The first index for which `less(i)` is false.
            let (mut lo, mut hi) = (0usize, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                if less(mid) {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        let to_f = |s: &Scalar| match s {
            Scalar::F64(x) => Some(*x),
            Scalar::I64(x) => Some(*x as f64),
            _ => None,
        };
        Ok(match (self, v) {
            (Series::I64 { values, .. }, Scalar::I64(x)) => {
                pos(&|i| if right { values[i] <= *x } else { values[i] < *x })
            }
            (Series::I64 { values, .. }, Scalar::F64(x)) => {
                pos(&|i| if right { (values[i] as f64) <= *x } else { (values[i] as f64) < *x })
            }
            (Series::F64 { values, .. }, Scalar::F64(_) | Scalar::I64(_)) => {
                let x = to_f(v).expect("numeric");
                pos(&|i| if right { values[i] <= x } else { values[i] < x })
            }
            (Series::Str { col, .. }, Scalar::Str(x)) => {
                pos(&|i| if right { col.get(i) <= x.as_str() } else { col.get(i) < x.as_str() })
            }
            _ => {
                return Err(OdsError::TypeMismatch(format!(
                    "searchsorted: cannot search a {} series for {:?}",
                    self.dtype(),
                    v
                )));
            }
        })
    }

    /// Nulls filled from the nearest valid value before (`forward`) or
    /// after; `limit` caps how many in a row one value fills.
    pub fn fill_direction(&self, forward: bool, limit: Option<usize>) -> Series {
        let Some(v) = self.validity() else {
            return self.clone();
        };
        let n = self.len();
        // The source row of each position (itself when valid), and the
        // validity after the fill, one pass.
        let mut words = v.words().to_vec();
        let mut src: Vec<usize> = Vec::with_capacity(n);
        let mut last: Option<usize> = None;
        let mut run = 0usize;
        if forward && limit.is_none() {
            // Values carried straight across, whole valid words as blocks.
            fn carry<T: Copy + Default>(vals: &[T], v: &Bitmap, words: &mut [u64]) -> Vec<T> {
                let n = vals.len();
                let mut out = Vec::with_capacity(n);
                let mut last: Option<T> = None;
                for (w, &bits) in v.words().iter().enumerate() {
                    let lo = w * 64;
                    let hi = (lo + 64).min(n);
                    if bits == u64::MAX && hi - lo == 64 {
                        out.extend_from_slice(&vals[lo..hi]);
                        last = Some(vals[hi - 1]);
                        continue;
                    }
                    for i in lo..hi {
                        if (bits >> (i - lo)) & 1 == 1 {
                            last = Some(vals[i]);
                            out.push(vals[i]);
                        } else {
                            if last.is_some() {
                                words[w] |= 1 << (i - lo);
                            }
                            out.push(last.unwrap_or_default());
                        }
                    }
                }
                out
            }
            let finish = |words: Vec<u64>| {
                let bm = Bitmap::from_words(n, words);
                (!bm.all_valid()).then_some(bm)
            };
            match self {
                Series::F64 { values, .. } => {
                    let out = carry(values, v, &mut words);
                    return Series::F64 {
                        values: Arc::new(out),
                        validity: finish(words),
                    };
                }
                Series::I64 { values, .. } => {
                    let out = carry(values, v, &mut words);
                    return Series::I64 {
                        values: Arc::new(out),
                        validity: finish(words),
                    };
                }
                Series::Bool { values, .. } => {
                    let out = carry(values, v, &mut words);
                    return Series::Bool {
                        values: Arc::new(out),
                        validity: finish(words),
                    };
                }
                Series::Str { .. } => {}
            }
            // Text: the source row of each position, then a gather.
            for (w, &bits) in v.words().iter().enumerate() {
                let lo = w * 64;
                let hi = (lo + 64).min(n);
                if bits == u64::MAX && hi - lo == 64 {
                    src.extend(lo..hi);
                    last = Some(hi - 1);
                    continue;
                }
                for i in lo..hi {
                    if (bits >> (i - lo)) & 1 == 1 {
                        last = Some(i);
                        src.push(i);
                    } else if let Some(l) = last {
                        words[i / 64] |= 1 << (i % 64);
                        src.push(l);
                    } else {
                        src.push(i);
                    }
                }
            }
            return self.gather_filled(&src, words);
        }
        let mut step = |i: usize, src: &mut Vec<usize>| {
            if v.get(i) {
                last = Some(i);
                run = 0;
                src.push(i);
            } else {
                run += 1;
                match last {
                    Some(l) if limit.is_none_or(|lim| run <= lim) => {
                        words[i / 64] |= 1 << (i % 64);
                        src.push(l);
                    }
                    _ => src.push(i),
                }
            }
        };
        if forward {
            for i in 0..n {
                step(i, &mut src);
            }
        } else {
            for i in (0..n).rev() {
                step(i, &mut src);
            }
            src.reverse();
        }
        self.gather_filled(&src, words)
    }

    /// The values at `src`, under the validity `words`.
    fn gather_filled(&self, src: &[usize], words: Vec<u64>) -> Series {
        let n = self.len();
        let validity = {
            let bm = Bitmap::from_words(n, words);
            (!bm.all_valid()).then_some(bm)
        };
        match self {
            Series::F64 { values, .. } => Series::F64 {
                values: Arc::new(src.iter().map(|&i| values[i]).collect()),
                validity,
            },
            Series::I64 { values, .. } => Series::I64 {
                values: Arc::new(src.iter().map(|&i| values[i]).collect()),
                validity,
            },
            Series::Bool { values, .. } => Series::Bool {
                values: Arc::new(src.iter().map(|&i| values[i]).collect()),
                validity,
            },
            Series::Str { col, .. } => Series::Str {
                col: col.gather(src),
                validity,
            },
        }
    }

    /// Nulls between two valid values filled on the straight line
    /// between them, by position; leading and trailing nulls stay null.
    /// `limit` caps how many in a row are filled (the first of each
    /// run). The result is Float.
    pub fn interpolate(&self, limit: Option<usize>) -> Result<Series> {
        let n = self.len();
        let x: Vec<f64> = match self {
            Series::F64 { values, .. } => values.as_ref().clone(),
            Series::I64 { values, .. } => values.iter().map(|&v| v as f64).collect(),
            _ => {
                return Err(OdsError::TypeMismatch(format!(
                    "interpolate needs a numeric series, got {}",
                    self.dtype()
                )));
            }
        };
        let Some(v) = self.validity() else {
            return Ok(Series::from_f64(x));
        };
        let mut out = x;
        let mut words = v.words().to_vec();
        let mut prev: Option<usize> = None;
        let mut i = 0;
        while i < n {
            if v.get(i) {
                prev = Some(i);
                i += 1;
                continue;
            }
            let start = i;
            while i < n && !v.get(i) {
                i += 1;
            }
            // A gap start..i, between prev and i (if both exist).
            if let (Some(p), true) = (prev, i < n) {
                let (a, b) = (out[p], out[i]);
                let span = (i - p) as f64;
                for (k, j) in (start..i).enumerate() {
                    if limit.is_some_and(|l| k >= l) {
                        break;
                    }
                    out[j] = a + (b - a) * ((j - p) as f64 / span);
                    words[j / 64] |= 1 << (j % 64);
                }
            }
        }
        let bm = Bitmap::from_words(n, words);
        Ok(Series::F64 {
            values: Arc::new(out),
            validity: (!bm.all_valid()).then_some(bm),
        })
    }

    /// Each value replaced through the map `from[k] → to[k]`; a value it
    /// does not name becomes what `unmapped` says. Nulls stay null.
    pub fn recode(&self, from: &Series, to: &Series, unmapped: &Unmapped) -> Result<Series> {
        if from.len() != to.len() {
            return Err(OdsError::LengthMismatch {
                left: from.len(),
                right: to.len(),
            });
        }
        let n = self.len();
        // Each distinct value is looked up once: the column's dictionary
        // encoding, then a gather.
        let (ids, n_groups) = crate::frame::group_ids(&[self])?;
        let mut rep = vec![usize::MAX; n_groups];
        for (i, &g) in ids.iter().enumerate() {
            if rep[g as usize] == usize::MAX {
                rep[g as usize] = i;
            }
        }
        let num = |s: &Scalar| match s {
            Scalar::F64(x) => Some(*x),
            Scalar::I64(x) => Some(*x as f64),
            _ => None,
        };
        // The first `from` entry naming a value wins.
        let lookup = |v: &Scalar| -> Option<usize> {
            (0..from.len()).find(|&k| match (v, from.scalar_at(k)) {
                (Scalar::Null, _) => false,
                (Scalar::Str(a), Scalar::Str(b)) => *a == b,
                (Scalar::Bool(a), Scalar::Bool(b)) => *a == b,
                (Scalar::I64(a), Scalar::I64(b)) => *a == b,
                (a, b) => match (num(a), num(&b)) {
                    (Some(x), Some(y)) => x == y,
                    _ => false,
                },
            })
        };
        let small = from.len() <= 16;
        let mut by_key: HashMap<String, usize> = HashMap::new();
        if !small {
            for k in (0..from.len()).rev() {
                by_key.insert(key_text(&from.scalar_at(k)), k);
            }
        }
        let group_hit: Vec<Option<usize>> = rep
            .iter()
            .map(|&r| {
                let v = self.scalar_at(r);
                if matches!(v, Scalar::Null) {
                    None
                } else if small {
                    lookup(&v)
                } else {
                    by_key.get(&key_text(&v)).copied()
                }
            })
            .collect();
        let hit: Vec<Option<usize>> = ids.iter().map(|&g| group_hit[g as usize]).collect();
        // The mapped values, null where unmapped, then the fallback over
        // them: one coalesce, whatever the types.
        let idx: Vec<usize> = hit.iter().map(|h| h.unwrap_or(0)).collect();
        let mut mapped = if to.is_empty() {
            full(n, &Scalar::Null)
        } else {
            gather_all(to, &idx)
        };
        let mut ok: Vec<bool> = hit.iter().map(|h| h.is_some()).collect();
        if let Some(v) = mapped.validity() {
            for (i, b) in ok.iter_mut().enumerate() {
                *b = *b && v.get(i);
            }
        }
        set_validity(
            &mut mapped,
            (!ok.iter().all(|&b| b)).then(|| Bitmap::from_bools(&ok)),
        );
        // Every valid value found in the map: nothing falls back.
        if mapped.null_count() == self.null_count() {
            return Ok(mapped);
        }
        let fallback = match unmapped {
            Unmapped::Null => return Ok(mapped),
            Unmapped::Keep => Expr::col(self.clone()),
            // Only unmapped valid positions take it; nulls stay null.
            Unmapped::Value(v) => Expr::op(
                    Op::Where,
                    vec![
                        Expr::op(Op::IsNull, vec![Expr::col(self.clone())]),
                        Expr::lit(Scalar::Null),
                        Expr::lit(v.clone()),
                    ],
                ),
        };
        Expr::op(Op::Coalesce, vec![Expr::col(mapped), fallback]).eval(n, false)
    }

    /// The column with every null replaced by its mean (`median`: its
    /// median). An Int column stays Int when the statistic is whole,
    /// and becomes Float otherwise.
    pub fn fill_stat(&self, median: bool) -> Result<Series> {
        if self.validity().is_none() {
            return Ok(self.clone());
        }
        let stat = if median {
            self.quantile(0.5)?
        } else {
            self.mean(false)?
        };
        let Some(stat) = stat else {
            return Ok(self.clone());
        };
        let fill = match self {
            Series::I64 { .. } if stat.fract() == 0.0 => Scalar::I64(stat as i64),
            _ => Scalar::F64(stat),
        };
        Expr::op(Op::Coalesce, vec![Expr::col(self.clone()), Expr::lit(fill)]).eval(self.len(), false)
    }

    /// Apply one math function (the expression engine's kernel).
    pub fn math(&self, m: Math) -> Result<Series> {
        Expr::op(Op::Math(m), vec![Expr::col(self.clone())]).eval(self.len(), false)
    }
}

/// A recode key as text that numbers of either kind share (`2` is `2.0`).
fn key_text(s: &Scalar) -> String {
    match s {
        Scalar::F64(x) if x.fract() == 0.0 && x.abs() < 9.0e15 => format!("n{}", *x as i64),
        Scalar::F64(x) => format!("n{}", x),
        Scalar::I64(x) => format!("n{}", x),
        Scalar::Bool(b) => format!("b{}", b),
        Scalar::Str(t) => format!("s{}", t),
        Scalar::Null => "null".to_string(),
    }
}

fn set_validity(s: &mut Series, v: Option<Bitmap>) {
    match s {
        Series::F64 { validity, .. }
        | Series::I64 { validity, .. }
        | Series::Bool { validity, .. }
        | Series::Str { validity, .. } => *validity = v,
    }
}


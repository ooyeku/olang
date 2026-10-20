//! Rolling windows in O(n): running sums (compensated), monotonic
//! deques for the extremes, and a sorted window for the median.
//!
//! Element `i` reduces the window of `w` rows ending at `i` (`center`:
//! the window around `i`, as pandas places it — rows `i - w/2` to
//! `i - w/2 + w - 1`, so an even window leans back by half a row). A
//! window that runs past either end is reduced only when `min_periods`
//! is given and it holds at least that many valid values; without it, a
//! window must be whole and hold one valid value (the long-standing
//! rule: the first `w - 1` elements of a trailing window are null).
//!
//! Sums, means, variances and standard deviations slide: each step adds
//! the row entering and subtracts the row leaving, with Neumaier
//! compensation, so a window of any size costs O(1) per row. The error
//! is a few units in the last place of the window's sum, far below any
//! measurement's own precision; `exact: true` recomputes every window
//! from scratch (O(n × w)) for a result equal to `ods.sum` of the same
//! rows. An Int column's sum is exact either way (an i128 total).

use crate::{AggOp, DType, OdsError, Series};
use std::collections::VecDeque;

type Result<T> = std::result::Result<T, OdsError>;

/// The options of a rolling window.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rolling {
    /// Centre the window on its row.
    pub center: bool,
    /// The fewest valid values a window reduces (`None`: the window must
    /// be whole).
    pub min_periods: Option<usize>,
    /// Recompute each window's sum from scratch.
    pub exact: bool,
}

/// A compensated running sum (Neumaier), with the non-finite values
/// counted apart so one NaN or infinity does not stay in the total
/// after it leaves the window.
#[derive(Default, Clone, Copy)]
struct Running {
    sum: f64,
    comp: f64,
    nan: usize,
    pos_inf: usize,
    neg_inf: usize,
}

impl Running {
    #[inline]
    fn add(&mut self, x: f64, sign: f64) {
        if !x.is_finite() {
            let d = if sign > 0.0 { 1isize } else { -1 };
            let slot = if x.is_nan() {
                &mut self.nan
            } else if x > 0.0 {
                &mut self.pos_inf
            } else {
                &mut self.neg_inf
            };
            *slot = (*slot as isize + d) as usize;
            return;
        }
        let x = x * sign;
        let t = self.sum + x;
        if self.sum.abs() >= x.abs() {
            self.comp += (self.sum - t) + x;
        } else {
            self.comp += (x - t) + self.sum;
        }
        self.sum = t;
    }

    fn value(&self) -> f64 {
        if self.nan > 0 || (self.pos_inf > 0 && self.neg_inf > 0) {
            f64::NAN
        } else if self.pos_inf > 0 {
            f64::INFINITY
        } else if self.neg_inf > 0 {
            f64::NEG_INFINITY
        } else {
            self.sum + self.comp
        }
    }

    fn finite(&self) -> bool {
        self.nan == 0 && self.pos_inf == 0 && self.neg_inf == 0
    }
}

/// The window bounds `[lo, hi)` of output `i`, and whether the window
/// lies wholly inside the column.
#[inline]
fn bounds(i: usize, n: usize, w: usize, off: usize) -> (usize, usize, bool) {
    let end = i + off + 1; // one past the window's last row, unclipped
    let lo = end.saturating_sub(w);
    let hi = end.min(n);
    (lo, hi, end >= w && end <= n)
}

impl Series {
    /// A rolling aggregate with the default options (trailing, whole
    /// windows, running sums).
    pub fn rolling(&self, window: usize, op: AggOp) -> Result<Series> {
        self.rolling_with(window, op, Rolling::default())
    }

    /// A rolling aggregate: count, sum, mean, min, max, median, std or
    /// var over each window. Nulls inside a window are skipped, as the
    /// whole-column reductions skip them.
    pub fn rolling_with(&self, window: usize, op: AggOp, opts: Rolling) -> Result<Series> {
        if window == 0 {
            return Err(OdsError::InvalidArgument(
                "rolling: window must be at least 1".to_string(),
            ));
        }
        if !matches!(
            op,
            AggOp::Count
                | AggOp::Sum
                | AggOp::Mean
                | AggOp::Min
                | AggOp::Max
                | AggOp::Median
                | AggOp::Std
                | AggOp::Var
        ) {
            return Err(OdsError::InvalidArgument(format!(
                "rolling: {} is not a rolling aggregation (count, sum, mean, min, max, median, std, var)",
                op.name()
            )));
        }
        let numeric = matches!(self.dtype(), DType::F64 | DType::I64);
        if op != AggOp::Count && !numeric {
            return Err(OdsError::InvalidArgument(format!(
                "rolling: {:?} needs a numeric column, got {}",
                op,
                self.dtype()
            )));
        }
        let n = self.len();
        let w = window;
        let off = if opts.center { (w - 1) - w / 2 } else { 0 };
        let valid = |i: usize| self.validity().is_none_or(|v| v.get(i));
        // Whether a window of `c` valid values (`whole`: inside the
        // column) is reported.
        let keep = |c: usize, whole: bool, least: usize| match opts.min_periods {
            Some(m) => c >= m.max(least),
            None => whole && c >= least,
        };
        if op == AggOp::Count {
            let mut out = Vec::with_capacity(n);
            let (mut lo, mut hi, mut c) = (0usize, 0usize, 0usize);
            for i in 0..n {
                let (l, h, whole) = bounds(i, n, w, off);
                while hi < h {
                    c += valid(hi) as usize;
                    hi += 1;
                }
                while lo < l {
                    c -= valid(lo) as usize;
                    lo += 1;
                }
                out.push(keep(c, whole, 0).then_some(c as i64));
            }
            return Ok(Series::from_i64_options(out));
        }
        // A Float column without nulls: the same arithmetic over the
        // slice, every row valid (the common case, and twice as fast).
        if let Series::F64 { values, validity: None } = self
            && matches!(op, AggOp::Sum | AggOp::Mean | AggOp::Min | AggOp::Max | AggOp::Std | AggOp::Var)
            && !opts.exact
        {
            return Ok(rolling_dense(values, w, off, op, opts.min_periods));
        }
        let f = |i: usize| -> Option<f64> {
            if !valid(i) {
                return None;
            }
            Some(match self {
                Series::F64 { values, .. } => values[i],
                Series::I64 { values, .. } => values[i] as f64,
                _ => unreachable!("numeric checked"),
            })
        };
        let int = |i: usize| -> Option<i64> {
            match self {
                Series::I64 { values, .. } if valid(i) => Some(values[i]),
                _ => None,
            }
        };
        let is_int = self.dtype() == DType::I64;
        let mut out: Vec<Option<f64>> = Vec::with_capacity(n);
        let mut out_int: Vec<Option<i64>> = Vec::new();
        match op {
            AggOp::Sum | AggOp::Mean if is_int => {
                // Exact: an i128 total.
                let (mut lo, mut hi, mut c, mut t) = (0usize, 0usize, 0usize, 0i128);
                for i in 0..n {
                    let (l, h, whole) = bounds(i, n, w, off);
                    while hi < h {
                        if let Some(x) = int(hi) {
                            t += x as i128;
                            c += 1;
                        }
                        hi += 1;
                    }
                    while lo < l {
                        if let Some(x) = int(lo) {
                            t -= x as i128;
                            c -= 1;
                        }
                        lo += 1;
                    }
                    let ok = keep(c, whole, 1);
                    if op == AggOp::Sum {
                        out_int.push(match ok {
                            true => Some(i64::try_from(t).map_err(|_| {
                                OdsError::IntegerOverflow("addition")
                            })?),
                            false => None,
                        });
                    } else {
                        out.push(ok.then(|| t as f64 / c as f64));
                    }
                }
                if op == AggOp::Sum {
                    return Ok(Series::from_i64_options(out_int));
                }
            }
            AggOp::Sum | AggOp::Mean | AggOp::Std | AggOp::Var if opts.exact => {
                for i in 0..n {
                    let (l, h, whole) = bounds(i, n, w, off);
                    let (mut c, mut total) = (0usize, 0.0f64);
                    for x in (l..h).filter_map(f) {
                        c += 1;
                        total += x;
                    }
                    let least = if matches!(op, AggOp::Std | AggOp::Var) { 2 } else { 1 };
                    if !keep(c, whole, 1) || c < least {
                        out.push(None);
                        continue;
                    }
                    out.push(Some(match op {
                        AggOp::Sum => total,
                        AggOp::Mean => total / c as f64,
                        _ => {
                            let mean = total / c as f64;
                            let ss: f64 = (l..h).filter_map(f).map(|x| (x - mean) * (x - mean)).sum();
                            let var = ss / (c - 1) as f64;
                            if op == AggOp::Std { var.sqrt() } else { var }
                        }
                    }));
                }
            }
            AggOp::Sum | AggOp::Mean => {
                let (mut lo, mut hi, mut c) = (0usize, 0usize, 0usize);
                let mut r = Running::default();
                for i in 0..n {
                    let (l, h, whole) = bounds(i, n, w, off);
                    while hi < h {
                        if let Some(x) = f(hi) {
                            r.add(x, 1.0);
                            c += 1;
                        }
                        hi += 1;
                    }
                    while lo < l {
                        if let Some(x) = f(lo) {
                            r.add(x, -1.0);
                            c -= 1;
                        }
                        lo += 1;
                    }
                    if c == 0 {
                        // An empty window starts clean: no drift carries
                        // across a gap.
                        r = Running::default();
                    }
                    out.push(keep(c, whole, 1).then(|| {
                        let s = r.value();
                        if op == AggOp::Sum { s } else { s / c as f64 }
                    }));
                }
            }
            AggOp::Std | AggOp::Var => {
                // Sums of (x - k) and (x - k)², k a value of the column,
                // so a large common offset does not cancel away the
                // spread.
                let k = (0..n).filter_map(f).find(|x| x.is_finite()).unwrap_or(0.0);
                let (mut lo, mut hi, mut c) = (0usize, 0usize, 0usize);
                let (mut s1, mut s2) = (Running::default(), Running::default());
                for i in 0..n {
                    let (l, h, whole) = bounds(i, n, w, off);
                    while hi < h {
                        if let Some(x) = f(hi) {
                            s1.add(x - k, 1.0);
                            s2.add((x - k) * (x - k), 1.0);
                            c += 1;
                        }
                        hi += 1;
                    }
                    while lo < l {
                        if let Some(x) = f(lo) {
                            s1.add(x - k, -1.0);
                            s2.add((x - k) * (x - k), -1.0);
                            c -= 1;
                        }
                        lo += 1;
                    }
                    if c == 0 {
                        s1 = Running::default();
                        s2 = Running::default();
                    }
                    out.push((keep(c, whole, 1) && c >= 2).then(|| {
                        if !s1.finite() || !s2.finite() {
                            return f64::NAN;
                        }
                        let (a, b) = (s1.value(), s2.value());
                        let var = ((b - a * a / c as f64) / (c - 1) as f64).max(0.0);
                        if op == AggOp::Std { var.sqrt() } else { var }
                    }));
                }
            }
            AggOp::Min | AggOp::Max => {
                let want_max = op == AggOp::Max;
                // Indices of the window's candidates, their values
                // monotone (decreasing for max): the front is the answer.
                let mut dq: VecDeque<usize> = VecDeque::new();
                let (mut lo, mut hi, mut c, mut nan) = (0usize, 0usize, 0usize, 0usize);
                let better = |a: f64, b: f64| if want_max { a >= b } else { a <= b };
                let better_i = |a: i64, b: i64| if want_max { a >= b } else { a <= b };
                for i in 0..n {
                    let (l, h, whole) = bounds(i, n, w, off);
                    while hi < h {
                        if valid(hi) {
                            c += 1;
                            if let Some(x) = int(hi) {
                                while dq.back().is_some_and(|&j| better_i(x, int(j).expect("valid"))) {
                                    dq.pop_back();
                                }
                                dq.push_back(hi);
                            } else if let Some(x) = f(hi) {
                                if x.is_nan() {
                                    nan += 1;
                                } else {
                                    while dq.back().is_some_and(|&j| better(x, f(j).expect("valid"))) {
                                        dq.pop_back();
                                    }
                                    dq.push_back(hi);
                                }
                            }
                        }
                        hi += 1;
                    }
                    while lo < l {
                        if valid(lo) {
                            c -= 1;
                            if f(lo).is_some_and(|x| x.is_nan()) {
                                nan -= 1;
                            }
                        }
                        lo += 1;
                    }
                    while dq.front().is_some_and(|&j| j < lo) {
                        dq.pop_front();
                    }
                    let ok = keep(c, whole, 1);
                    if is_int {
                        out_int.push(if ok { dq.front().and_then(|&j| int(j)) } else { None });
                    } else {
                        // A window of NaNs alone is NaN; NaN is skipped
                        // otherwise, as `f64::min` skips it.
                        out.push(ok.then(|| match dq.front() {
                            Some(&j) => f(j).expect("valid"),
                            None => {
                                let _ = nan;
                                f64::NAN
                            }
                        }));
                    }
                }
                if is_int {
                    return Ok(Series::from_i64_options(out_int));
                }
            }
            AggOp::Median => {
                let mut sorted: Vec<f64> = Vec::with_capacity(w);
                let (mut lo, mut hi) = (0usize, 0usize);
                for i in 0..n {
                    let (l, h, whole) = bounds(i, n, w, off);
                    while hi < h {
                        if let Some(x) = f(hi) {
                            let at = sorted.partition_point(|y| y.total_cmp(&x).is_lt());
                            sorted.insert(at, x);
                        }
                        hi += 1;
                    }
                    while lo < l {
                        if let Some(x) = f(lo) {
                            let at = sorted.partition_point(|y| y.total_cmp(&x).is_lt());
                            sorted.remove(at);
                        }
                        lo += 1;
                    }
                    out.push(keep(sorted.len(), whole, 1).then(|| {
                        let h = (sorted.len() - 1) as f64 * 0.5;
                        let (a, b) = (h.floor() as usize, h.ceil() as usize);
                        sorted[a] + (sorted[b] - sorted[a]) * (h - a as f64)
                    }));
                }
            }
            _ => unreachable!("checked above"),
        }
        // Mean, median, std and var are fractions; the rest keep the
        // column's own type, as the whole-column reductions do.
        Ok(match op {
            AggOp::Sum | AggOp::Min | AggOp::Max if is_int => {
                Series::from_i64_options(out.into_iter().map(|o| o.map(|x| x as i64)).collect())
            }
            _ => Series::from_f64_options(out),
        })
    }
}

/// `rolling_with` over a Float column with no nulls: every row of a
/// window counts, so the count is the window's width.
fn rolling_dense(v: &[f64], w: usize, off: usize, op: AggOp, min_periods: Option<usize>) -> Series {
    let n = v.len();
    let mut out = vec![0.0f64; n];
    let mut ok = vec![false; n];
    let least = if matches!(op, AggOp::Std | AggOp::Var) { 2 } else { 1 };
    let keep = |c: usize, whole: bool| {
        c >= least
            && match min_periods {
                Some(m) => c >= m,
                None => whole,
            }
    };
    let (mut lo, mut hi) = (0usize, 0usize);
    match op {
        AggOp::Sum | AggOp::Mean => {
            let mut r = Running::default();
            for i in 0..n {
                let (l, h, whole) = bounds(i, n, w, off);
                while hi < h {
                    r.add(v[hi], 1.0);
                    hi += 1;
                }
                while lo < l {
                    r.add(v[lo], -1.0);
                    lo += 1;
                }
                let c = h - l;
                if keep(c, whole) {
                    let s = r.value();
                    out[i] = if op == AggOp::Sum { s } else { s / c as f64 };
                    ok[i] = true;
                }
            }
        }
        AggOp::Std | AggOp::Var => {
            let k = v.iter().copied().find(|x| x.is_finite()).unwrap_or(0.0);
            let (mut s1, mut s2) = (Running::default(), Running::default());
            for i in 0..n {
                let (l, h, whole) = bounds(i, n, w, off);
                while hi < h {
                    let d = v[hi] - k;
                    s1.add(d, 1.0);
                    s2.add(d * d, 1.0);
                    hi += 1;
                }
                while lo < l {
                    let d = v[lo] - k;
                    s1.add(d, -1.0);
                    s2.add(d * d, -1.0);
                    lo += 1;
                }
                let c = h - l;
                if keep(c, whole) {
                    out[i] = if !s1.finite() || !s2.finite() {
                        f64::NAN
                    } else {
                        let (a, b) = (s1.value(), s2.value());
                        let var = ((b - a * a / c as f64) / (c - 1) as f64).max(0.0);
                        if op == AggOp::Std { var.sqrt() } else { var }
                    };
                    ok[i] = true;
                }
            }
        }
        _ => {
            let want_max = op == AggOp::Max;
            let mut dq: VecDeque<usize> = VecDeque::with_capacity(w.min(n));
            for i in 0..n {
                let (l, h, whole) = bounds(i, n, w, off);
                while hi < h {
                    let x = v[hi];
                    if !x.is_nan() {
                        while let Some(&j) = dq.back() {
                            if (want_max && x >= v[j]) || (!want_max && x <= v[j]) {
                                dq.pop_back();
                            } else {
                                break;
                            }
                        }
                        dq.push_back(hi);
                    }
                    hi += 1;
                }
                lo = l;
                while dq.front().is_some_and(|&j| j < lo) {
                    dq.pop_front();
                }
                if keep(h - l, whole) {
                    // A window of NaNs alone is NaN.
                    out[i] = dq.front().map_or(f64::NAN, |&j| v[j]);
                    ok[i] = true;
                }
            }
        }
    }
    if ok.iter().all(|&b| b) {
        Series::from_f64(out)
    } else {
        Series::from_f64_with_validity(out, Some(ok))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scalar;

    fn f64s(s: &Series) -> Vec<Option<f64>> {
        (0..s.len())
            .map(|i| match s.scalar_at(i) {
                Scalar::F64(x) => Some(x),
                Scalar::I64(x) => Some(x as f64),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn running_and_exact_agree() {
        let v: Vec<f64> = (0..500).map(|i| ((i * 37) % 101) as f64 * 0.37 + 1e6).collect();
        let s = Series::from_f64(v);
        for op in [AggOp::Sum, AggOp::Mean, AggOp::Std, AggOp::Var] {
            let a = f64s(&s.rolling_with(30, op, Rolling::default()).unwrap());
            let b = f64s(&s.rolling_with(30, op, Rolling { exact: true, ..Default::default() }).unwrap());
            for (x, y) in a.iter().zip(&b) {
                match (x, y) {
                    (Some(x), Some(y)) => assert!((x - y).abs() <= 1e-6 * y.abs().max(1.0), "{op:?} {x} {y}"),
                    (None, None) => {}
                    _ => panic!("{op:?} nulls differ"),
                }
            }
        }
    }

    #[test]
    fn centred_windows_follow_pandas() {
        let s = Series::from_f64(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let c = Rolling { center: true, ..Default::default() };
        assert_eq!(f64s(&s.rolling_with(4, AggOp::Sum, c).unwrap()), vec![None, None, Some(10.0), Some(14.0), Some(18.0), None]);
        let m = Rolling { center: true, min_periods: Some(1), exact: false };
        assert_eq!(f64s(&s.rolling_with(4, AggOp::Sum, m).unwrap()), vec![Some(3.0), Some(6.0), Some(10.0), Some(14.0), Some(18.0), Some(15.0)]);
        assert_eq!(f64s(&s.rolling_with(3, AggOp::Max, Rolling::default()).unwrap()), vec![None, None, Some(3.0), Some(4.0), Some(5.0), Some(6.0)]);
        assert_eq!(f64s(&s.rolling_with(3, AggOp::Median, c).unwrap()), vec![None, Some(2.0), Some(3.0), Some(4.0), Some(5.0), None]);
    }

    #[test]
    fn a_nan_leaves_the_window_with_its_row() {
        let s = Series::from_f64(vec![1.0, f64::NAN, 2.0, 3.0, 4.0]);
        let r = f64s(&s.rolling(2, AggOp::Sum).unwrap());
        assert!(r[1].unwrap().is_nan() && r[2].unwrap().is_nan());
        assert_eq!(r[3], Some(5.0));
        assert_eq!(r[4], Some(7.0));
    }
}

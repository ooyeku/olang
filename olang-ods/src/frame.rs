//! Frame: a columnar table — named, equal-length Series.
//!
//! The verb set is the tidyverse core: `select`, `with_column`,
//! `filter`/`take` (delegating to the column kernels), `sort_by`,
//! `group_by` + aggregate, and hash `join` (inner/left). Semantics
//! follow the column layer: aggregations skip nulls, a null key forms
//! its own group in `group_by` (R/Polars behavior) but never matches in
//! joins (SQL behavior), and groups keep first-seen order so results
//! are deterministic without a sort.
//!
//! Grouping hashes with an inline Fx-style hasher (multiply-xor) —
//! SipHash's DoS resistance buys nothing against our own group keys and
//! costs most of the group-by budget at 10M rows.

use crate::{Bitmap, OdsError, Scalar, Series};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

type Result<T> = std::result::Result<T, OdsError>;

// ---------------------------------------------------------------------
// Fx-style hashing
// ---------------------------------------------------------------------

#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash = (self.hash.rotate_left(5) ^ b as u64).wrapping_mul(SEED);
        }
    }

    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.hash = (self.hash.rotate_left(5) ^ n).wrapping_mul(SEED);
    }

    #[inline]
    fn write_i64(&mut self, n: i64) {
        self.write_u64(n as u64);
    }

    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

// ---------------------------------------------------------------------
// Frame
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Frame {
    names: Vec<String>,
    cols: Vec<Series>,
}

/// One aggregation in a `group_by`: `out_name = op(col)`.
#[derive(Clone, Debug)]
pub struct AggSpec {
    pub out_name: String,
    pub op: AggOp,
    pub col: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggOp {
    /// Rows in the group (nulls included — R's `n()`).
    Count,
    Sum,
    Mean,
    Min,
    Max,
}

impl AggOp {
    pub fn parse(name: &str) -> Option<AggOp> {
        Some(match name {
            "count" => AggOp::Count,
            "sum" => AggOp::Sum,
            "mean" => AggOp::Mean,
            "min" => AggOp::Min,
            "max" => AggOp::Max,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinHow {
    Inner,
    Left,
    /// Every row from both sides. Unmatched rows on either side keep
    /// their own columns and take nulls in the other side's.
    Full,
    /// Left rows that have at least one match, once each, left columns
    /// only — the existence question, without the multiplication an
    /// inner join would apply when the right side has duplicates.
    Semi,
    /// Left rows that have no match, left columns only.
    Anti,
}

impl Frame {
    pub fn new(pairs: Vec<(String, Series)>) -> Result<Frame> {
        let mut names = Vec::with_capacity(pairs.len());
        let mut cols = Vec::with_capacity(pairs.len());
        for (name, col) in pairs {
            if names.contains(&name) {
                return Err(OdsError::InvalidArgument(format!(
                    "frame: duplicate column name '{}'",
                    name
                )));
            }
            if let Some(first) = cols.first() {
                let first: &Series = first;
                if col.len() != first.len() {
                    return Err(OdsError::LengthMismatch {
                        left: first.len(),
                        right: col.len(),
                    });
                }
            }
            names.push(name);
            cols.push(col);
        }
        Ok(Frame { names, cols })
    }

    pub fn n_rows(&self) -> usize {
        self.cols.first().map(|c| c.len()).unwrap_or(0)
    }

    pub fn n_cols(&self) -> usize {
        self.cols.len()
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn columns(&self) -> &[Series] {
        &self.cols
    }

    pub fn column(&self, name: &str) -> Result<&Series> {
        self.index_of(name).map(|i| &self.cols[i])
    }

    fn index_of(&self, name: &str) -> Result<usize> {
        self.names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| OdsError::InvalidArgument(format!("frame: no column named '{}'", name)))
    }

    pub fn select(&self, names: &[String]) -> Result<Frame> {
        let mut pairs = Vec::with_capacity(names.len());
        for name in names {
            pairs.push((name.clone(), self.column(name)?.clone()));
        }
        Frame::new(pairs)
    }

    /// Replace `name` if it exists, else append it.
    pub fn with_column(&self, name: &str, col: Series) -> Result<Frame> {
        if !self.cols.is_empty() && col.len() != self.n_rows() {
            return Err(OdsError::LengthMismatch {
                left: self.n_rows(),
                right: col.len(),
            });
        }
        let mut out = self.clone();
        match out.names.iter().position(|n| n == name) {
            Some(i) => out.cols[i] = col,
            None => {
                out.names.push(name.to_string());
                out.cols.push(col);
            }
        }
        Ok(out)
    }

    pub fn take(&self, indices: &Series) -> Result<Frame> {
        // Resolve the indices ONCE (negatives, bounds) and share the
        // resolved list across every column — per-column take used to
        // redo the same 1M-element resolution six times on a six-column
        // frame. Each column then gathers independently, one column per
        // core on a large frame. Column order is preserved; the result
        // is identical to the sequential gather.
        let idx: &[i64] = match indices {
            Series::I64 {
                values,
                validity: None,
            } => values,
            Series::I64 { .. } => {
                return Err(OdsError::InvalidArgument(
                    "take indices must not contain nulls".to_string(),
                ));
            }
            _ => {
                return Err(OdsError::TypeMismatch(format!(
                    "take indices must be an Int series, got Series[{}]",
                    indices.dtype()
                )));
            }
        };
        let resolved = crate::resolve_take_indices(idx, self.n_rows())?;
        #[cfg(feature = "parallel")]
        let cols = if resolved.len() >= PAR_TAKE_ROWS
            && self.cols.len() > 1
            && crate::parallel_enabled()
        {
            use rayon::prelude::*;
            self.cols
                .par_iter()
                .map(|c| c.gather(&resolved))
                .collect::<Vec<_>>()
        } else {
            self.cols.iter().map(|c| c.gather(&resolved)).collect()
        };
        #[cfg(not(feature = "parallel"))]
        let cols: Vec<Series> = self.cols.iter().map(|c| c.gather(&resolved)).collect();
        Ok(Frame {
            names: self.names.clone(),
            cols,
        })
    }

    pub fn filter(&self, mask: &Series) -> Result<Frame> {
        // The mask converts to keep-indices ONCE and every column
        // gathers against that one list — per-column filter used to
        // rescan the mask per column. Columns run one per core on a
        // large frame; order preserved, result identical to sequential.
        if self.n_rows() != mask.len() {
            return Err(OdsError::LengthMismatch {
                left: self.n_rows(),
                right: mask.len(),
            });
        }
        let keep = crate::mask_keep_indices(mask)?;
        #[cfg(feature = "parallel")]
        let cols =
            if mask.len() >= PAR_TAKE_ROWS && self.cols.len() > 1 && crate::parallel_enabled() {
                use rayon::prelude::*;
                self.cols
                    .par_iter()
                    .map(|c| c.gather(&keep))
                    .collect::<Vec<_>>()
            } else {
                self.cols.iter().map(|c| c.gather(&keep)).collect()
            };
        #[cfg(not(feature = "parallel"))]
        let cols: Vec<Series> = self.cols.iter().map(|c| c.gather(&keep)).collect();
        Ok(Frame {
            names: self.names.clone(),
            cols,
        })
    }

    pub fn head(&self, n: usize) -> Frame {
        let keep: Vec<i64> = (0..self.n_rows().min(n) as i64).collect();
        let idx = Series::from_i64(keep);
        // take on in-range prefix indices cannot fail
        self.take(&idx).expect("prefix take")
    }

    /// Sort rows by one column; nulls last regardless of direction.
    pub fn sort_by(&self, name: &str, descending: bool) -> Result<Frame> {
        let key = self.column(name)?;
        let idx = key.argsort()?;
        let idx = if descending {
            // Reverse only the valid prefix: nulls stay last.
            let n_valid = key.len() - key.null_count();
            match &idx {
                Series::I64 { values, .. } => {
                    let mut v: Vec<i64> = values.as_ref().clone();
                    v[..n_valid].reverse();
                    Series::from_i64(v)
                }
                _ => unreachable!("argsort returns a dense Int series"),
            }
        } else {
            idx
        };
        self.take(&idx)
    }

    // -----------------------------------------------------------------
    // group_by
    // -----------------------------------------------------------------

    pub fn group_by(&self, keys: &[String], aggs: &[AggSpec]) -> Result<Frame> {
        if keys.is_empty() {
            return Err(OdsError::InvalidArgument(
                "group_by needs at least one key column".to_string(),
            ));
        }
        let key_cols: Vec<&Series> = keys.iter().map(|k| self.column(k)).collect::<Result<_>>()?;
        let n = self.n_rows();

        // Group ids per row, first-seen order.
        let (group_ids, n_groups) = if key_cols.len() == 1 {
            group_ids_single(key_cols[0])
        } else {
            group_ids_multi(&key_cols, n)
        };

        // Key output columns: first row index of each group.
        let first_row = first_row_per_group(&group_ids, n_groups);
        let first_idx = Series::from_i64(first_row.clone());

        let mut pairs: Vec<(String, Series)> = Vec::with_capacity(keys.len() + aggs.len());
        for (k, col) in keys.iter().zip(key_cols.iter()) {
            pairs.push((k.clone(), col.take(&first_idx)?));
        }
        for spec in aggs {
            let out = match spec.op {
                AggOp::Count => {
                    let mut counts = vec![0i64; n_groups];
                    for &g in &group_ids {
                        counts[g as usize] += 1;
                    }
                    Series::from_i64(counts)
                }
                _ => aggregate_column(self.column(&spec.col)?, &group_ids, n_groups, spec.op)?,
            };
            pairs.push((spec.out_name.clone(), out));
        }
        Frame::new(pairs)
    }

    // -----------------------------------------------------------------
    // join
    // -----------------------------------------------------------------

    /// Hash join on one key column per side. Null keys never match; a
    /// left join keeps unmatched (and null-key) left rows with nulls on
    /// the right. Right columns keep their names, except the right key
    /// (dropped) and collisions (suffixed `_right`).
    pub fn join(
        &self,
        right: &Frame,
        left_on: &str,
        right_on: &str,
        how: JoinHow,
    ) -> Result<Frame> {
        let lkey = self.column(left_on)?;
        let rkey = right.column(right_on)?;

        // Right key -> row indices (duplicates multiply, standard SQL).
        let mut table: FxMap<Key, Vec<usize>> = FxMap::default();
        for i in 0..rkey.len() {
            if let Some(k) = key_at(rkey, i) {
                table.entry(k).or_default().push(i);
            }
        }

        // Semi and anti ask about existence rather than combination, so
        // they answer with a subset of the left frame — no right columns,
        // and no multiplication when a key repeats on the right.
        if matches!(how, JoinHow::Semi | JoinHow::Anti) {
            let want_match = how == JoinHow::Semi;
            let rows: Vec<i64> = (0..lkey.len())
                .filter(|&i| {
                    // A null key matches nothing, which puts a null-keyed
                    // left row in the anti result and never in the semi
                    // one — the same rule the other kinds follow.
                    key_at(lkey, i).is_some_and(|k| table.contains_key(&k)) == want_match
                })
                .map(|i| i as i64)
                .collect();
            return self.take(&Series::from_i64(rows));
        }

        // The probe: each left row is looked up independently in the
        // read-only table, so the loop parallelizes across left rows on a
        // large frame — the join's dominant cost. Chunks are concatenated in
        // row order, so the result is bit-identical to the sequential path.
        // A full join probes as a left join and then appends what the
        // right side had left over.
        let probe_how = if how == JoinHow::Full {
            JoinHow::Left
        } else {
            how
        };
        let (left_idx, right_idx) = join_probe(lkey, &table, probe_how, lkey.len());

        if how == JoinHow::Full {
            let mut matched = vec![false; rkey.len()];
            for r in right_idx.iter().flatten() {
                matched[*r] = true;
            }
            // Right rows nothing matched, in row order — including any
            // with a null key, since a null matched nothing.
            let orphans: Vec<usize> = (0..rkey.len()).filter(|&r| !matched[r]).collect();

            let left_rows: Vec<Option<usize>> = left_idx
                .iter()
                .map(|&i| Some(i as usize))
                .chain(orphans.iter().map(|_| None))
                .collect();
            let right_rows: Vec<Option<usize>> = right_idx
                .iter()
                .copied()
                .chain(orphans.iter().map(|&r| Some(r)))
                .collect();

            let mut pairs: Vec<(String, Series)> = Vec::new();
            for (name, col) in self.names.iter().zip(self.cols.iter()) {
                if name == left_on {
                    // The key column must carry the right side's value on
                    // an orphan row. Without this the one column that says
                    // which row this is would be null exactly where the
                    // reader needs it.
                    pairs.push((
                        name.clone(),
                        coalesce(
                            &gather_optional(col, &left_rows),
                            &gather_optional(rkey, &right_rows),
                        )?,
                    ));
                } else {
                    pairs.push((name.clone(), gather_optional(col, &left_rows)));
                }
            }
            for (name, col) in right.names.iter().zip(right.cols.iter()) {
                if name == right_on {
                    continue;
                }
                let out_name = if self.names.contains(name) {
                    format!("{}_right", name)
                } else {
                    name.clone()
                };
                pairs.push((out_name, gather_optional(col, &right_rows)));
            }
            return Frame::new(pairs);
        }

        let lidx = Series::from_i64(left_idx);
        let mut pairs: Vec<(String, Series)> = Vec::new();
        for (name, col) in self.names.iter().zip(self.cols.iter()) {
            pairs.push((name.clone(), col.take(&lidx)?));
        }
        for (name, col) in right.names.iter().zip(right.cols.iter()) {
            if name == right_on {
                continue;
            }
            let out_name = if self.names.contains(name) {
                format!("{}_right", name)
            } else {
                name.clone()
            };
            pairs.push((out_name, gather_optional(col, &right_idx)));
        }
        Frame::new(pairs)
    }
}

// ---------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------

/// How a rank distributes the positions that tie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankMethod {
    /// 1, 2, 2, 4 — the competition ranking, and what "rank" means
    /// unqualified.
    Min,
    /// 1, 3, 3, 4 — ties take the last position they span.
    Max,
    /// 1, 2.5, 2.5, 4 — ties share the mean of the positions they span.
    Average,
    /// 1, 2, 3, 4 — ties broken by row order, so every rank is distinct.
    Ordinal,
    /// 1, 2, 2, 3 — the next distinct value gets the next integer.
    Dense,
}

impl RankMethod {
    pub fn parse(name: &str) -> Option<RankMethod> {
        Some(match name {
            "min" => RankMethod::Min,
            "max" => RankMethod::Max,
            "average" => RankMethod::Average,
            "ordinal" => RankMethod::Ordinal,
            "dense" => RankMethod::Dense,
            _ => return None,
        })
    }
}

impl Series {
    /// Move values `by` positions down the column, filling what is
    /// vacated with nulls. A negative `by` moves them up.
    ///
    /// Nulls rather than a wrapped value, because a window has an edge:
    /// the row before the first row does not exist, and saying so is
    /// the honest answer. `s - ods.shift(s, 1)` is then a difference
    /// whose first element is null, which is correct.
    pub fn shift(&self, by: i64) -> Result<Series> {
        let n = self.len() as i64;
        let idx: Vec<Option<usize>> = (0..n)
            .map(|i| {
                let from = i - by;
                (0..n).contains(&from).then_some(from as usize)
            })
            .collect();
        Ok(gather_optional(self, &idx))
    }

    /// The running maximum (or minimum), the shape `cumsum` already has:
    /// element `i` reduces `0..=i`. A null contributes nothing and takes
    /// the running value so far, so the result has no nulls after the
    /// first valid element.
    pub fn cum_extreme(&self, want_max: bool) -> Result<Series> {
        if matches!(self.dtype(), crate::DType::Str | crate::DType::Bool) {
            return Err(OdsError::InvalidArgument(format!(
                "cum_max/cum_min: needs a numeric column, got {}",
                self.dtype()
            )));
        }
        let n = self.len();
        let better = |a: f64, b: f64| if want_max { a > b } else { a < b };
        let mut running: Option<f64> = None;
        let mut out: Vec<Option<f64>> = Vec::with_capacity(n);
        for i in 0..n {
            let v = match self.scalar_at(i) {
                Scalar::F64(x) => Some(x),
                Scalar::I64(x) => Some(x as f64),
                _ => None,
            };
            if let Some(x) = v
                && running.is_none_or(|r| better(x, r))
            {
                running = Some(x);
            }
            out.push(running);
        }
        Ok(match self.dtype() {
            crate::DType::I64 => {
                Series::from_i64_options(out.into_iter().map(|o| o.map(|x| x as i64)).collect())
            }
            _ => Series::from_f64_options(out),
        })
    }

    /// The rank of each element, smallest first. Nulls rank as null:
    /// they are skipped by every other reduction, and giving them a
    /// position would silently place them somewhere.
    pub fn rank(&self, method: RankMethod) -> Result<Series> {
        let n = self.len();
        // Order the valid positions. `argsort` puts nulls last, which is
        // exactly the tail this then ignores.
        let order = self.argsort()?;
        let positions: Vec<usize> = (0..order.len())
            .filter_map(|i| match order.scalar_at(i) {
                Scalar::I64(row) => Some(row as usize),
                _ => None,
            })
            .filter(|&row| !matches!(self.scalar_at(row), Scalar::Null))
            .collect();

        let mut out: Vec<Option<f64>> = vec![None; n];
        let mut i = 0;
        let mut dense = 0i64;
        while i < positions.len() {
            // The run of equal values starting at i.
            let mut j = i + 1;
            while j < positions.len()
                && scalars_equal(self.scalar_at(positions[j]), self.scalar_at(positions[i]))
            {
                j += 1;
            }
            dense += 1;
            for (k, &row) in positions[i..j].iter().enumerate() {
                out[row] = Some(match method {
                    RankMethod::Min => (i + 1) as f64,
                    RankMethod::Max => j as f64,
                    RankMethod::Average => ((i + 1 + j) as f64) / 2.0,
                    RankMethod::Ordinal => (i + k + 1) as f64,
                    RankMethod::Dense => dense as f64,
                });
            }
            i = j;
        }
        // Only `average` can produce a half, so the others stay Int and
        // can be used as indices without a cast.
        Ok(if method == RankMethod::Average {
            Series::from_f64_options(out)
        } else {
            Series::from_i64_options(out.into_iter().map(|o| o.map(|x| x as i64)).collect())
        })
    }

    /// A trailing-window aggregate: element `i` reduces the `window`
    /// elements ending at `i`. The first `window - 1` elements are null,
    /// because the window is not yet full and reporting a partial
    /// reduction as if it were whole is how a chart lies at its left
    /// edge.
    ///
    /// Nulls inside a window are skipped, exactly as the whole-column
    /// reductions skip them, so a window with two valid elements of
    /// three means the mean of two.
    ///
    /// Cost is one reduction per element — O(n × window). The
    /// incremental alternative accumulates float drift that a fresh sum
    /// does not, which would make a rolling mean disagree with the
    /// `sum` of the same window. Correctness first; if a large window on
    /// a large column ever shows up in a measurement, that is the point
    /// to revisit it.
    pub fn rolling(&self, window: usize, op: AggOp) -> Result<Series> {
        if window == 0 {
            return Err(OdsError::InvalidArgument(
                "rolling: window must be at least 1".to_string(),
            ));
        }
        let n = self.len();
        if op != AggOp::Count && matches!(self.dtype(), crate::DType::Str | crate::DType::Bool) {
            return Err(OdsError::InvalidArgument(format!(
                "rolling: {:?} needs a numeric column, got {}",
                op,
                self.dtype()
            )));
        }
        let value_at = |i: usize| match self.scalar_at(i) {
            Scalar::F64(x) => Some(x),
            Scalar::I64(x) => Some(x as f64),
            _ => None,
        };
        let reduce = |lo: usize, hi: usize| -> Option<f64> {
            let vals = (lo..hi).filter_map(value_at);
            match op {
                AggOp::Count => Some((lo..hi).filter(|&i| value_at(i).is_some()).count() as f64),
                AggOp::Sum => {
                    let mut any = false;
                    let mut total = 0.0;
                    for v in vals {
                        any = true;
                        total += v;
                    }
                    // An all-null window sums to null, not to zero: no
                    // data is not the same measurement as zero.
                    any.then_some(total)
                }
                AggOp::Mean => {
                    let (mut total, mut count) = (0.0, 0usize);
                    for v in vals {
                        total += v;
                        count += 1;
                    }
                    (count > 0).then(|| total / count as f64)
                }
                AggOp::Min => vals.fold(None, |a: Option<f64>, v| Some(a.map_or(v, |x| x.min(v)))),
                AggOp::Max => vals.fold(None, |a: Option<f64>, v| Some(a.map_or(v, |x| x.max(v)))),
            }
        };
        let out: Vec<Option<f64>> = (0..n)
            .map(|i| {
                (i + 1 >= window)
                    .then(|| reduce(i + 1 - window, i + 1))
                    .flatten()
            })
            .collect();
        // Count is always a count; mean is always a fraction; the rest
        // keep the column's own type, as the whole-column reductions do.
        Ok(match op {
            AggOp::Count => {
                Series::from_i64_options(out.into_iter().map(|o| o.map(|x| x as i64)).collect())
            }
            AggOp::Mean => Series::from_f64_options(out),
            _ if self.dtype() == crate::DType::I64 => {
                Series::from_i64_options(out.into_iter().map(|o| o.map(|x| x as i64)).collect())
            }
            _ => Series::from_f64_options(out),
        })
    }
}

/// Value equality for ranking, where two nulls never reach this and NaN
/// is not a concern (argsort has already ordered them).
fn scalars_equal(a: Scalar, b: Scalar) -> bool {
    match (a, b) {
        (Scalar::F64(x), Scalar::F64(y)) => x == y,
        (Scalar::I64(x), Scalar::I64(y)) => x == y,
        (Scalar::Bool(x), Scalar::Bool(y)) => x == y,
        (Scalar::Str(x), Scalar::Str(y)) => x == y,
        _ => false,
    }
}

// ---------------------------------------------------------------------
// Reshape
// ---------------------------------------------------------------------

impl Frame {
    /// Long to wide: one row per distinct `index` combination, one column
    /// per distinct value of `columns`, each cell the `op` aggregate of
    /// `values` over the rows that share both.
    ///
    /// The aggregation is a `group_by` — literally, the same call — so
    /// pivoting and grouping cannot disagree about how a column reduces
    /// or what happens to nulls. What this adds is the scatter.
    ///
    /// `fmt_f64` renders a float on its way to becoming a column name,
    /// for the same reason `cast` takes one: the engine has no opinion on
    /// how olang spells a float, and a second opinion would drift.
    pub fn pivot(
        &self,
        index: &[String],
        columns: &str,
        values: &str,
        op: AggOp,
        fmt_f64: &dyn Fn(f64) -> String,
    ) -> Result<Frame> {
        if index.is_empty() {
            return Err(OdsError::InvalidArgument(
                "pivot: needs at least one index column".to_string(),
            ));
        }
        for name in index {
            if name == columns || name == values {
                return Err(OdsError::InvalidArgument(format!(
                    "pivot: '{}' cannot be both an index column and the {} column",
                    name,
                    if name == columns { "columns" } else { "values" }
                )));
            }
        }

        // One group per (index..., columns) cell, aggregated exactly as
        // group_by would. CELL is a name a caller cannot collide with,
        // since it never reaches the output.
        const CELL: &str = "\u{0}cell";
        let mut keys: Vec<String> = index.to_vec();
        keys.push(columns.to_string());
        let cells = self.group_by(
            &keys,
            &[AggSpec {
                out_name: CELL.to_string(),
                op,
                col: values.to_string(),
            }],
        )?;

        let index_cols: Vec<&Series> = index
            .iter()
            .map(|n| cells.column(n))
            .collect::<Result<_>>()?;
        let column_col = cells.column(columns)?;
        let cell_col = cells.column(CELL)?;

        // Rows and output columns, both in first-seen order — the order
        // the data presented them, which is what group_by already
        // guarantees for its keys.
        let (row_ids, n_rows) = if index_cols.len() == 1 {
            group_ids_single(index_cols[0])
        } else {
            group_ids_multi(&index_cols, cells.n_rows())
        };
        let (col_ids, n_cols) = group_ids_single(column_col);

        // A null cannot name a column, and calling it "null" would
        // collide with a genuine "null" string. Say so instead.
        for i in 0..column_col.len() {
            if matches!(column_col.scalar_at(i), Scalar::Null) {
                return Err(OdsError::InvalidArgument(format!(
                    "pivot: '{}' has a null, and a null cannot name a column. \
                     Drop or fill those rows first",
                    columns
                )));
            }
        }

        let out_names: Vec<String> = first_row_per_group(&col_ids, n_cols)
            .iter()
            .map(|&r| scalar_name(column_col.scalar_at(r as usize), fmt_f64))
            .collect();
        for name in &out_names {
            if index.contains(name) {
                return Err(OdsError::InvalidArgument(format!(
                    "pivot: the value '{}' in '{}' would name a column that already \
                     exists as an index column",
                    name, columns
                )));
            }
        }

        // Scatter: every group is one cell, so each lands in exactly one
        // slot. Slots nothing wrote stay null — the combination did not
        // occur, which is a different fact from a null value in it.
        let mut slots: Vec<Vec<Option<usize>>> = vec![vec![None; n_rows]; n_cols];
        for r in 0..cells.n_rows() {
            slots[col_ids[r] as usize][row_ids[r] as usize] = Some(r);
        }

        let first_rows = Series::from_i64(first_row_per_group(&row_ids, n_rows));
        let mut pairs: Vec<(String, Series)> = Vec::with_capacity(index.len() + n_cols);
        for (name, col) in index.iter().zip(index_cols.iter()) {
            pairs.push((name.clone(), col.take(&first_rows)?));
        }
        for (name, slot) in out_names.into_iter().zip(slots) {
            pairs.push((name, gather_optional(cell_col, &slot)));
        }
        Frame::new(pairs)
    }

    /// Wide to long: keep `id` columns as they are, and turn each of
    /// `value_columns` into rows of a `name`/`value` pair.
    ///
    /// The value columns are stacked into one column, so they must share
    /// a type. Mixing them would mean choosing a common type on the
    /// caller's behalf, which is `cast`'s job and its explicit decision.
    pub fn unpivot(
        &self,
        id: &[String],
        value_columns: &[String],
        name_out: &str,
        value_out: &str,
    ) -> Result<Frame> {
        if value_columns.is_empty() {
            return Err(OdsError::InvalidArgument(
                "unpivot: needs at least one value column".to_string(),
            ));
        }
        let cols: Vec<&Series> = value_columns
            .iter()
            .map(|n| self.column(n))
            .collect::<Result<_>>()?;
        let dtype = cols[0].dtype();
        if let Some(pos) = cols.iter().position(|c| c.dtype() != dtype) {
            return Err(OdsError::InvalidArgument(format!(
                "unpivot: the value columns stack into one column, so they must share \
                 a type — '{}' is {} but '{}' is {}. Cast them first",
                value_columns[0],
                dtype,
                value_columns[pos],
                cols[pos].dtype()
            )));
        }
        for name in id {
            if value_columns.contains(name) {
                return Err(OdsError::InvalidArgument(format!(
                    "unpivot: '{}' cannot be both an id column and a value column",
                    name
                )));
            }
        }
        for reserved in [name_out, value_out] {
            if id.contains(&reserved.to_string()) {
                return Err(OdsError::InvalidArgument(format!(
                    "unpivot: an id column is already called '{}', which is where the \
                     output goes",
                    reserved
                )));
            }
        }

        // Column-major: all of the first value column's rows, then the
        // second's. Reading down the output follows the input's columns
        // in the order the caller named them.
        let n = self.n_rows();
        let repeat = Series::from_i64((0..value_columns.len()).flat_map(|_| 0..n as i64).collect());
        let mut pairs: Vec<(String, Series)> = Vec::with_capacity(id.len() + 2);
        for name in id {
            pairs.push((name.clone(), self.column(name)?.take(&repeat)?));
        }
        pairs.push((
            name_out.to_string(),
            Series::from_str_values(
                value_columns
                    .iter()
                    .flat_map(|name| std::iter::repeat_n(name.clone(), n))
                    .collect(),
            ),
        ));
        pairs.push((value_out.to_string(), stack_same_dtype(&cols)));
        Frame::new(pairs)
    }
}

/// Lay same-typed columns end to end. `unpivot` has already established
/// that they share a type; this is the copy that follows.
fn stack_same_dtype(cols: &[&Series]) -> Series {
    let scalars = || {
        cols.iter()
            .flat_map(|c| (0..c.len()).map(|i| c.scalar_at(i)))
    };
    match cols[0].dtype() {
        crate::DType::F64 => Series::from_f64_options(
            scalars()
                .map(|v| match v {
                    Scalar::F64(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::I64 => Series::from_i64_options(
            scalars()
                .map(|v| match v {
                    Scalar::I64(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::Bool => Series::from_bool_options(
            scalars()
                .map(|v| match v {
                    Scalar::Bool(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::Str => Series::from_str_options(
            scalars()
                .map(|v| match v {
                    Scalar::Str(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
    }
}

/// A scalar as a column name. Only reached for `pivot`, where the values
/// of one column become the names of many.
fn scalar_name(v: Scalar, fmt_f64: &dyn Fn(f64) -> String) -> String {
    match v {
        Scalar::Str(s) => s,
        Scalar::I64(x) => x.to_string(),
        Scalar::F64(x) => fmt_f64(x),
        Scalar::Bool(b) => b.to_string(),
        // Refused before this is reached.
        Scalar::Null => "null".to_string(),
    }
}

// ---------------------------------------------------------------------
// Join probe
// ---------------------------------------------------------------------

/// Left frames at or above this many rows probe in parallel. Below it, the
/// thread hand-off costs more than the probe saves.
#[cfg(feature = "parallel")]
const PAR_JOIN_ROWS: usize = 50_000;

/// Below this many gathered rows, Frame::take stays sequential.
#[cfg(feature = "parallel")]
const PAR_TAKE_ROWS: usize = 100_000;

/// Probe left rows `[lo, hi)` against the built right-key table, producing
/// `(left_idx, right_idx)` match pairs in left-row order (a matched left row
/// repeats once per right match; an unmatched row is kept with `None` only
/// for a left join). Pure and read-only over `table`/`lkey`, so it is safe
/// to run over disjoint ranges on separate threads.
fn probe_join(
    lkey: &Series,
    table: &FxMap<Key, Vec<usize>>,
    how: JoinHow,
    lo: usize,
    hi: usize,
) -> (Vec<i64>, Vec<Option<usize>>) {
    let mut left_idx: Vec<i64> = Vec::new();
    let mut right_idx: Vec<Option<usize>> = Vec::new();
    for i in lo..hi {
        match key_at(lkey, i).and_then(|k| table.get(&k)) {
            Some(matches) => {
                for &r in matches {
                    left_idx.push(i as i64);
                    right_idx.push(Some(r));
                }
            }
            None => {
                if how == JoinHow::Left {
                    left_idx.push(i as i64);
                    right_idx.push(None);
                }
            }
        }
    }
    (left_idx, right_idx)
}

/// Probe all `n` left rows, in parallel over row-chunks when the frame is
/// large and the `parallel` feature is on. Chunks are concatenated in row
/// order, so the output is identical to the sequential probe.
fn join_probe(
    lkey: &Series,
    table: &FxMap<Key, Vec<usize>>,
    how: JoinHow,
    n: usize,
) -> (Vec<i64>, Vec<Option<usize>>) {
    #[cfg(feature = "parallel")]
    if n >= PAR_JOIN_ROWS && crate::parallel_enabled() {
        use rayon::prelude::*;
        let threads = rayon::current_num_threads().max(1);
        let chunk = n.div_ceil(threads).max(1);
        let ranges: Vec<(usize, usize)> = (0..n)
            .step_by(chunk)
            .map(|s| (s, (s + chunk).min(n)))
            .collect();
        let parts: Vec<(Vec<i64>, Vec<Option<usize>>)> = ranges
            .par_iter()
            .map(|&(lo, hi)| probe_join(lkey, table, how, lo, hi))
            .collect();
        let total: usize = parts.iter().map(|(l, _)| l.len()).sum();
        let mut left_idx = Vec::with_capacity(total);
        let mut right_idx = Vec::with_capacity(total);
        for (li, ri) in parts {
            left_idx.extend(li);
            right_idx.extend(ri);
        }
        return (left_idx, right_idx);
    }
    probe_join(lkey, table, how, 0, n)
}

/// The row index where each group first appears, indexed by group id.
/// `group_ids_single`/`_multi` assign ids in first-seen order, so this is
/// also sorted ascending — but the verbs below reorder groups, and rely
/// on being able to ask for a specific group's first row.
fn first_row_per_group(group_ids: &[u32], n_groups: usize) -> Vec<i64> {
    let mut first_row: Vec<i64> = vec![0; n_groups];
    let mut seen = vec![false; n_groups];
    for (row, &g) in group_ids.iter().enumerate() {
        let g = g as usize;
        if !seen[g] {
            seen[g] = true;
            first_row[g] = row as i64;
        }
    }
    first_row
}

/// Column-level distinct verbs.
///
/// All three go through `group_ids_single` — the same pass `group_by`
/// uses — so they cannot disagree with it about what counts as one
/// value. Two consequences worth stating, because both differ from `==`
/// on the corresponding scalars: a null is a value (it forms its own
/// group, the R/Polars convention), and all NaNs are one value, since
/// keys are float bit patterns with NaN canonicalized.
impl Series {
    /// The distinct values, in first-seen order.
    pub fn unique(&self) -> Result<Series> {
        let (ids, n_groups) = group_ids_single(self);
        self.take(&Series::from_i64(first_row_per_group(&ids, n_groups)))
    }

    /// How many distinct values there are. Counting does not need the
    /// values themselves, so this skips the gather `unique` pays for.
    pub fn n_unique(&self) -> usize {
        group_ids_single(self).1
    }

    /// Each distinct value paired with how many rows carry it, most
    /// frequent first. Ties break by first appearance rather than
    /// arbitrarily, so the result is deterministic for a given input.
    pub fn value_counts(&self) -> Result<(Series, Series)> {
        let (ids, n_groups) = group_ids_single(self);
        let mut counts = vec![0i64; n_groups];
        for &g in &ids {
            counts[g as usize] += 1;
        }
        let first = first_row_per_group(&ids, n_groups);
        let mut order: Vec<usize> = (0..n_groups).collect();
        order.sort_by(|&a, &b| counts[b].cmp(&counts[a]).then(first[a].cmp(&first[b])));
        let rows = Series::from_i64(order.iter().map(|&g| first[g]).collect());
        let counts = Series::from_i64(order.iter().map(|&g| counts[g]).collect());
        Ok((self.take(&rows)?, counts))
    }
}

// ---------------------------------------------------------------------
// Group keys
// ---------------------------------------------------------------------

/// A hashable group/join key. Floats key by bit pattern (all NaNs are
/// one group).
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    I(i64),
    F(u64),
    B(bool),
    S(String),
}

fn key_at(s: &Series, i: usize) -> Option<Key> {
    match s.scalar_at(i) {
        Scalar::Null => None,
        Scalar::I64(x) => Some(Key::I(x)),
        Scalar::F64(x) => Some(Key::F(if x.is_nan() {
            f64::NAN.to_bits()
        } else {
            x.to_bits()
        })),
        Scalar::Bool(b) => Some(Key::B(b)),
        Scalar::Str(s) => Some(Key::S(s)),
    }
}

const NULL_GROUP: u64 = u64::MAX;

/// Group ids for a single key column, first-seen order. The dense i64
/// and string paths avoid per-row Scalar boxing — group_by's hot loop.
fn group_ids_single(key: &Series) -> (Vec<u32>, usize) {
    let n = key.len();
    let mut ids = Vec::with_capacity(n);
    let mut next: u32 = 0;
    let mut null_id: Option<u32> = None;

    match key {
        Series::I64 { values, validity } => {
            let mut map: FxMap<i64, u32> = FxMap::default();
            for (i, &v) in values.iter().enumerate() {
                let valid = validity.as_ref().map(|b| b.get(i)).unwrap_or(true);
                let id = if !valid {
                    *null_id.get_or_insert_with(|| {
                        let id = next;
                        next += 1;
                        id
                    })
                } else {
                    *map.entry(v).or_insert_with(|| {
                        let id = next;
                        next += 1;
                        id
                    })
                };
                ids.push(id);
            }
        }
        Series::Str { col, validity } => {
            let mut map: FxMap<&str, u32> = FxMap::default();
            for (i, v) in col.iter().enumerate() {
                let valid = validity.as_ref().map(|b| b.get(i)).unwrap_or(true);
                let id = if !valid {
                    *null_id.get_or_insert_with(|| {
                        let id = next;
                        next += 1;
                        id
                    })
                } else {
                    *map.entry(v).or_insert_with(|| {
                        let id = next;
                        next += 1;
                        id
                    })
                };
                ids.push(id);
            }
        }
        _ => {
            let mut map: FxMap<Key, u32> = FxMap::default();
            for i in 0..n {
                let id = match key_at(key, i) {
                    None => *null_id.get_or_insert_with(|| {
                        let id = next;
                        next += 1;
                        id
                    }),
                    Some(k) => *map.entry(k).or_insert_with(|| {
                        let id = next;
                        next += 1;
                        id
                    }),
                };
                ids.push(id);
            }
        }
    }
    let _ = NULL_GROUP;
    (ids, next as usize)
}

/// Cap on the dense composite table (`u32` slots): 4M slots = 16 MB
/// transient. Above it — or on cardinality overflow — the hash path
/// below handles the pathological keyspace.
const COMPOSITE_DENSE_MAX: usize = 1 << 22;

fn group_ids_multi(key_cols: &[&Series], n: usize) -> (Vec<u32>, usize) {
    // Fast path: dictionary-encode each key column independently (a
    // per-type hash pass, no per-row allocation), then combine the
    // per-row ids arithmetically and densify through an array — the
    // composite pass becomes multiply-add plus an index, instead of
    // allocating and hashing a Vec<Option<Key>> per row. First-seen
    // order is preserved: dense ids are assigned in row order. Nulls
    // group exactly as before — each column's encoder gives null its
    // own id, so the composite classes are identical to the hash path's.
    let encoded: Vec<(Vec<u32>, usize)> = key_cols.iter().map(|c| group_ids_single(c)).collect();
    let mut product: usize = 1;
    let mut dense_ok = true;
    for (_, card) in &encoded {
        match product.checked_mul((*card).max(1)) {
            Some(p) if p <= COMPOSITE_DENSE_MAX => product = p,
            _ => {
                dense_ok = false;
                break;
            }
        }
    }
    if dense_ok {
        let mut dense: Vec<u32> = vec![u32::MAX; product];
        let mut ids = Vec::with_capacity(n);
        let mut next: u32 = 0;
        for i in 0..n {
            let mut code: usize = 0;
            for (col_ids, card) in &encoded {
                code = code * (*card).max(1) + col_ids[i] as usize;
            }
            let slot = &mut dense[code];
            if *slot == u32::MAX {
                *slot = next;
                next += 1;
            }
            ids.push(*slot);
        }
        return (ids, next as usize);
    }

    // The composite key per row — a Scalar clone per key column — is the
    // expensive half of the hash pass, and each row's is independent, so
    // large frames build them across all cores. The id-assigning insert
    // loop below stays sequential and in row order, which is what makes
    // group ids come out in first-seen order — identical to a fully
    // sequential pass.
    let build = |i: usize| -> Vec<Option<Key>> { key_cols.iter().map(|c| key_at(c, i)).collect() };
    #[cfg(feature = "parallel")]
    let composites: Vec<Vec<Option<Key>>> = if n >= PAR_GROUPBY_ROWS && crate::parallel_enabled() {
        use rayon::prelude::*;
        (0..n).into_par_iter().map(build).collect()
    } else {
        (0..n).map(build).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let composites: Vec<Vec<Option<Key>>> = (0..n).map(build).collect();

    let mut map: FxMap<Vec<Option<Key>>, u32> = FxMap::default();
    let mut ids = Vec::with_capacity(n);
    let mut next: u32 = 0;
    for composite in composites {
        let id = *map.entry(composite).or_insert_with(|| {
            let id = next;
            next += 1;
            id
        });
        ids.push(id);
    }
    (ids, next as usize)
}

// ---------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------

/// Per-group accumulators from one scatter pass: (sum, count, min, max).
type F64Acc = (Vec<f64>, Vec<i64>, Vec<f64>, Vec<f64>);
type I64Acc = (Vec<i64>, Vec<i64>, Vec<i64>, Vec<i64>);

/// Below this row count, group-by aggregation stays sequential — the thread
/// hand-off costs more than the scatter saves.
#[cfg(feature = "parallel")]
const PAR_GROUPBY_ROWS: usize = 100_000;
/// Above this group count, aggregation stays sequential regardless of rows:
/// each thread holds its own `n_groups`-sized partial accumulators, so a very
/// high-cardinality group-by would blow memory. Typical group-bys have far
/// fewer groups than rows and fall well under this.
#[cfg(feature = "parallel")]
const PAR_GROUPBY_MAX_GROUPS: usize = 4_000_000;

/// Scatter-accumulate a float column into per-group (sum, count, min, max).
/// On a large frame the scatter runs across all cores: each thread reduces a
/// disjoint row-chunk into its own partials, and the partials merge in a
/// **fixed chunk order** — so the result is deterministic run to run.
/// `count`, `min`, and `max` are bit-identical to the sequential path; the
/// float `sum` (and the `mean` derived from it) can differ in the last ULPs
/// because addition is reordered.
fn accumulate_f64(
    values: &[f64],
    validity: Option<&Bitmap>,
    group_ids: &[u32],
    n_groups: usize,
) -> F64Acc {
    let scatter = |lo: usize, hi: usize| {
        let mut acc = vec![0.0f64; n_groups];
        let mut cnt = vec![0i64; n_groups];
        let mut mins = vec![f64::INFINITY; n_groups];
        let mut maxs = vec![f64::NEG_INFINITY; n_groups];
        for i in lo..hi {
            if validity.map(|b| b.get(i)).unwrap_or(true) {
                let g = group_ids[i] as usize;
                let v = values[i];
                acc[g] += v;
                cnt[g] += 1;
                if v < mins[g] {
                    mins[g] = v;
                }
                if v > maxs[g] {
                    maxs[g] = v;
                }
            }
        }
        (acc, cnt, mins, maxs)
    };

    #[cfg(feature = "parallel")]
    if values.len() >= PAR_GROUPBY_ROWS
        && n_groups <= PAR_GROUPBY_MAX_GROUPS
        && crate::parallel_enabled()
    {
        use rayon::prelude::*;
        let n = values.len();
        let threads = rayon::current_num_threads().max(1);
        let chunk = n.div_ceil(threads).max(1);
        let ranges: Vec<(usize, usize)> = (0..n)
            .step_by(chunk)
            .map(|s| (s, (s + chunk).min(n)))
            .collect();
        let partials: Vec<F64Acc> = ranges.par_iter().map(|&(lo, hi)| scatter(lo, hi)).collect();
        let mut acc = vec![0.0f64; n_groups];
        let mut cnt = vec![0i64; n_groups];
        let mut mins = vec![f64::INFINITY; n_groups];
        let mut maxs = vec![f64::NEG_INFINITY; n_groups];
        for (pa, pc, pmin, pmax) in &partials {
            for g in 0..n_groups {
                acc[g] += pa[g];
                cnt[g] += pc[g];
                if pmin[g] < mins[g] {
                    mins[g] = pmin[g];
                }
                if pmax[g] > maxs[g] {
                    maxs[g] = pmax[g];
                }
            }
        }
        return (acc, cnt, mins, maxs);
    }

    scatter(0, values.len())
}

/// The integer counterpart of [`accumulate_f64`]. Every reduction here is
/// associative, so the parallel result is bit-identical to sequential. Sum
/// overflow is a checked error, in both the per-chunk scatter and the merge.
fn accumulate_i64(
    values: &[i64],
    validity: Option<&Bitmap>,
    group_ids: &[u32],
    n_groups: usize,
) -> Result<I64Acc> {
    let scatter = |lo: usize, hi: usize| -> Result<I64Acc> {
        let mut acc = vec![0i64; n_groups];
        let mut cnt = vec![0i64; n_groups];
        let mut mins = vec![i64::MAX; n_groups];
        let mut maxs = vec![i64::MIN; n_groups];
        for i in lo..hi {
            if validity.map(|b| b.get(i)).unwrap_or(true) {
                let g = group_ids[i] as usize;
                let v = values[i];
                acc[g] = acc[g]
                    .checked_add(v)
                    .ok_or(OdsError::IntegerOverflow("addition"))?;
                cnt[g] += 1;
                if v < mins[g] {
                    mins[g] = v;
                }
                if v > maxs[g] {
                    maxs[g] = v;
                }
            }
        }
        Ok((acc, cnt, mins, maxs))
    };

    #[cfg(feature = "parallel")]
    if values.len() >= PAR_GROUPBY_ROWS
        && n_groups <= PAR_GROUPBY_MAX_GROUPS
        && crate::parallel_enabled()
    {
        use rayon::prelude::*;
        let n = values.len();
        let threads = rayon::current_num_threads().max(1);
        let chunk = n.div_ceil(threads).max(1);
        let ranges: Vec<(usize, usize)> = (0..n)
            .step_by(chunk)
            .map(|s| (s, (s + chunk).min(n)))
            .collect();
        let partials: Vec<I64Acc> = ranges
            .par_iter()
            .map(|&(lo, hi)| scatter(lo, hi))
            .collect::<Result<Vec<_>>>()?;
        let mut acc = vec![0i64; n_groups];
        let mut cnt = vec![0i64; n_groups];
        let mut mins = vec![i64::MAX; n_groups];
        let mut maxs = vec![i64::MIN; n_groups];
        for (pa, pc, pmin, pmax) in &partials {
            for g in 0..n_groups {
                acc[g] = acc[g]
                    .checked_add(pa[g])
                    .ok_or(OdsError::IntegerOverflow("addition"))?;
                cnt[g] += pc[g];
                if pmin[g] < mins[g] {
                    mins[g] = pmin[g];
                }
                if pmax[g] > maxs[g] {
                    maxs[g] = pmax[g];
                }
            }
        }
        return Ok((acc, cnt, mins, maxs));
    }

    scatter(0, values.len())
}

fn aggregate_column(col: &Series, group_ids: &[u32], n_groups: usize, op: AggOp) -> Result<Series> {
    match col {
        Series::F64 { values, validity } => {
            let (acc, cnt, mins, maxs) =
                accumulate_f64(values, validity.as_ref(), group_ids, n_groups);
            finish_f64(op, acc, cnt, mins, maxs)
        }
        Series::I64 { values, validity } => {
            let (acc, cnt, mins, maxs) =
                accumulate_i64(values, validity.as_ref(), group_ids, n_groups)?;
            match op {
                AggOp::Sum => Ok(Series::from_i64_options(
                    cnt.iter()
                        .zip(acc)
                        .map(|(&c, a)| (c > 0).then_some(a))
                        .collect(),
                )),
                AggOp::Mean => Ok(Series::from_f64_options(
                    cnt.iter()
                        .zip(acc)
                        .map(|(&c, a)| (c > 0).then(|| a as f64 / c as f64))
                        .collect(),
                )),
                AggOp::Min => Ok(Series::from_i64_options(
                    cnt.iter()
                        .zip(mins)
                        .map(|(&c, m)| (c > 0).then_some(m))
                        .collect(),
                )),
                AggOp::Max => Ok(Series::from_i64_options(
                    cnt.iter()
                        .zip(maxs)
                        .map(|(&c, m)| (c > 0).then_some(m))
                        .collect(),
                )),
                AggOp::Count => unreachable!("handled by caller"),
            }
        }
        _ => Err(OdsError::TypeMismatch(format!(
            "cannot aggregate {} over a {} column",
            match op {
                AggOp::Sum => "sum",
                AggOp::Mean => "mean",
                AggOp::Min => "min",
                AggOp::Max => "max",
                AggOp::Count => "count",
            },
            col.dtype()
        ))),
    }
}

fn finish_f64(
    op: AggOp,
    acc: Vec<f64>,
    cnt: Vec<i64>,
    mins: Vec<f64>,
    maxs: Vec<f64>,
) -> Result<Series> {
    let out = match op {
        AggOp::Sum => Series::from_f64_options(
            cnt.iter()
                .zip(acc)
                .map(|(&c, a)| (c > 0).then_some(a))
                .collect(),
        ),
        AggOp::Mean => Series::from_f64_options(
            cnt.iter()
                .zip(acc)
                .map(|(&c, a)| (c > 0).then(|| a / c as f64))
                .collect(),
        ),
        AggOp::Min => Series::from_f64_options(
            cnt.iter()
                .zip(mins)
                .map(|(&c, m)| (c > 0).then_some(m))
                .collect(),
        ),
        AggOp::Max => Series::from_f64_options(
            cnt.iter()
                .zip(maxs)
                .map(|(&c, m)| (c > 0).then_some(m))
                .collect(),
        ),
        AggOp::Count => unreachable!("handled by caller"),
    };
    Ok(out)
}

/// Gather with optional indices (join's right side): None -> null.
fn gather_optional(col: &Series, idx: &[Option<usize>]) -> Series {
    match col {
        Series::F64 { values, .. } => Series::from_f64_options(
            idx.iter()
                .map(|o| o.filter(|&i| col_valid(col, i)).map(|i| values[i]))
                .collect(),
        ),
        Series::I64 { values, .. } => Series::from_i64_options(
            idx.iter()
                .map(|o| o.filter(|&i| col_valid(col, i)).map(|i| values[i]))
                .collect(),
        ),
        Series::Bool { values, .. } => Series::from_bool_options(
            idx.iter()
                .map(|o| o.filter(|&i| col_valid(col, i)).map(|i| values[i]))
                .collect(),
        ),
        Series::Str { col: sc, .. } => {
            let resolved: Vec<Option<usize>> = idx
                .iter()
                .map(|o| o.filter(|&i| col_valid(col, i)))
                .collect();
            if resolved.iter().all(|o| o.is_some()) {
                let indices: Vec<usize> = resolved.into_iter().flatten().collect();
                Series::Str {
                    col: sc.gather(&indices),
                    validity: None,
                }
            } else {
                let valid: Vec<bool> = resolved.iter().map(|o| o.is_some()).collect();
                Series::Str {
                    col: sc.gather_opt(&resolved),
                    validity: Some(Bitmap::from_bools(&valid)),
                }
            }
        }
    }
}

/// `a` where it is present, `b` where it is null. Used for a full join's
/// key column, where each output row has the value from exactly one side.
///
/// The two sides must have the same type. They already do whenever the
/// join found anything at all — keys hash by type, so an Int column and
/// a Float column never match each other — but a full join returns rows
/// even when nothing matched, which is exactly when a mismatch would
/// otherwise be silently papered over.
fn coalesce(a: &Series, b: &Series) -> Result<Series> {
    if a.dtype() != b.dtype() {
        return Err(OdsError::InvalidArgument(format!(
            "join: a full join needs both key columns to have the same type, \
             got {} on the left and {} on the right",
            a.dtype(),
            b.dtype()
        )));
    }
    let pick = |i: usize| match a.scalar_at(i) {
        Scalar::Null => b.scalar_at(i),
        v => v,
    };
    let n = a.len();
    Ok(match a.dtype() {
        crate::DType::F64 => Series::from_f64_options(
            (0..n)
                .map(|i| match pick(i) {
                    Scalar::F64(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::I64 => Series::from_i64_options(
            (0..n)
                .map(|i| match pick(i) {
                    Scalar::I64(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::Bool => Series::from_bool_options(
            (0..n)
                .map(|i| match pick(i) {
                    Scalar::Bool(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::Str => Series::from_str_options(
            (0..n)
                .map(|i| match pick(i) {
                    Scalar::Str(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
    })
}

fn col_valid(col: &Series, i: usize) -> bool {
    !matches!(col.scalar_at(i), Scalar::Null)
}

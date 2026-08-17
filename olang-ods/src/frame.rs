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
        let cols = self
            .cols
            .iter()
            .map(|c| c.take(indices))
            .collect::<Result<Vec<_>>>()?;
        Ok(Frame {
            names: self.names.clone(),
            cols,
        })
    }

    pub fn filter(&self, mask: &Series) -> Result<Frame> {
        let cols = self
            .cols
            .iter()
            .map(|c| c.filter(mask))
            .collect::<Result<Vec<_>>>()?;
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
// Join probe
// ---------------------------------------------------------------------

/// Left frames at or above this many rows probe in parallel. Below it, the
/// thread hand-off costs more than the probe saves.
#[cfg(feature = "parallel")]
const PAR_JOIN_ROWS: usize = 50_000;

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
    if n >= PAR_JOIN_ROWS {
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
        Series::Str { values, validity } => {
            let mut map: FxMap<&str, u32> = FxMap::default();
            for (i, v) in values.iter().enumerate() {
                let valid = validity.as_ref().map(|b| b.get(i)).unwrap_or(true);
                let id = if !valid {
                    *null_id.get_or_insert_with(|| {
                        let id = next;
                        next += 1;
                        id
                    })
                } else {
                    *map.entry(v.as_str()).or_insert_with(|| {
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

fn group_ids_multi(key_cols: &[&Series], n: usize) -> (Vec<u32>, usize) {
    let mut map: FxMap<Vec<Option<Key>>, u32> = FxMap::default();
    let mut ids = Vec::with_capacity(n);
    let mut next: u32 = 0;
    for i in 0..n {
        let composite: Vec<Option<Key>> = key_cols.iter().map(|c| key_at(c, i)).collect();
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
    if values.len() >= PAR_GROUPBY_ROWS && n_groups <= PAR_GROUPBY_MAX_GROUPS {
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
    if values.len() >= PAR_GROUPBY_ROWS && n_groups <= PAR_GROUPBY_MAX_GROUPS {
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
        Series::Str { values, .. } => Series::from_str_options(
            idx.iter()
                .map(|o| o.filter(|&i| col_valid(col, i)).map(|i| values[i].clone()))
                .collect(),
        ),
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

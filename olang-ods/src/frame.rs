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

use crate::{OdsError, Scalar, Series};
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
        let mut first_row: Vec<usize> = vec![0; n_groups];
        let mut seen = vec![false; n_groups];
        for (row, &g) in group_ids.iter().enumerate() {
            let g = g as usize;
            if !seen[g] {
                seen[g] = true;
                first_row[g] = row;
            }
        }
        let first_idx = Series::from_i64(first_row.iter().map(|&r| r as i64).collect());

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

        let mut left_idx: Vec<i64> = Vec::new();
        let mut right_idx: Vec<Option<usize>> = Vec::new();
        for i in 0..lkey.len() {
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

fn aggregate_column(col: &Series, group_ids: &[u32], n_groups: usize, op: AggOp) -> Result<Series> {
    match col {
        Series::F64 { values, validity } => {
            let mut acc = vec![0.0f64; n_groups];
            let mut cnt = vec![0i64; n_groups];
            let mut mins = vec![f64::INFINITY; n_groups];
            let mut maxs = vec![f64::NEG_INFINITY; n_groups];
            for (i, (&v, &g)) in values.iter().zip(group_ids.iter()).enumerate() {
                if validity.as_ref().map(|b| b.get(i)).unwrap_or(true) {
                    let g = g as usize;
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
            finish_f64(op, acc, cnt, mins, maxs)
        }
        Series::I64 { values, validity } => {
            let mut acc = vec![0i64; n_groups];
            let mut cnt = vec![0i64; n_groups];
            let mut mins = vec![i64::MAX; n_groups];
            let mut maxs = vec![i64::MIN; n_groups];
            for (i, (&v, &g)) in values.iter().zip(group_ids.iter()).enumerate() {
                if validity.as_ref().map(|b| b.get(i)).unwrap_or(true) {
                    let g = g as usize;
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

fn col_valid(col: &Series, i: usize) -> bool {
    !matches!(col.scalar_at(i), Scalar::Null)
}

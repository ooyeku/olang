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

pub(crate) type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

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
    /// The middle value (the mean of the two middles for an even count),
    /// as `ods.median` answers it. Float.
    Median,
    /// Sample standard deviation (ddof = 1), as `ods.std`. Float; null
    /// with fewer than two values.
    Std,
    /// Sample variance (ddof = 1), as `ods.var`. Float.
    Var,
    /// The group's first row's value, null included. Any column type.
    First,
    /// The group's last row's value, null included. Any column type.
    Last,
    /// How many distinct values the group holds, as `ods.n_unique`
    /// counts them (null is one value). Any column type. Int.
    NUnique,
    /// Whether any valid value is true (a Bool column; a number counts
    /// when it is not zero). Bool.
    Any,
    /// Whether no valid value is false. Bool.
    All,
    /// The row position (in the whole column) of the group's largest
    /// valid value, the first on a tie; null when it has none. Int.
    ArgMax,
    /// The row position of the smallest valid value. Int.
    ArgMin,
    /// The `q` quantile by linear interpolation, as `ods.quantile`. The
    /// payload is `q`'s bits (`AggOp::quantile(q)`). Float.
    Quantile(u64),
}

impl AggOp {
    /// Every aggregation's name, in the order errors list them.
    pub const NAMES: &'static [&'static str] = &[
        "count", "sum", "mean", "min", "max", "median", "std", "var", "first", "last", "n_unique",
        "any", "all", "argmax", "argmin",
    ];

    /// The `q` quantile.
    pub fn quantile(q: f64) -> AggOp {
        AggOp::Quantile(q.to_bits())
    }

    /// A quantile's `q`.
    pub fn q(self) -> Option<f64> {
        match self {
            AggOp::Quantile(b) => Some(f64::from_bits(b)),
            _ => None,
        }
    }

    pub fn parse(name: &str) -> Option<AggOp> {
        Some(match name {
            "count" => AggOp::Count,
            "sum" => AggOp::Sum,
            "mean" => AggOp::Mean,
            "min" => AggOp::Min,
            "max" => AggOp::Max,
            "median" => AggOp::Median,
            "std" => AggOp::Std,
            "var" => AggOp::Var,
            "first" => AggOp::First,
            "last" => AggOp::Last,
            "n_unique" => AggOp::NUnique,
            "any" => AggOp::Any,
            "all" => AggOp::All,
            "argmax" => AggOp::ArgMax,
            "argmin" => AggOp::ArgMin,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            AggOp::Count => "count",
            AggOp::Sum => "sum",
            AggOp::Mean => "mean",
            AggOp::Min => "min",
            AggOp::Max => "max",
            AggOp::Median => "median",
            AggOp::Std => "std",
            AggOp::Var => "var",
            AggOp::First => "first",
            AggOp::Last => "last",
            AggOp::NUnique => "n_unique",
            AggOp::Any => "any",
            AggOp::All => "all",
            AggOp::ArgMax => "argmax",
            AggOp::ArgMin => "argmin",
            AggOp::Quantile(_) => "quantile",
        }
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
        // Group ids per row, first-seen order; each group's rows are
        // built once, for the aggregations that need a group's values
        // together (a median, a variance, a count of distinct values).
        let g = crate::group::Grouping::new(&key_cols)?;
        let mut pairs: Vec<(String, Series)> = Vec::with_capacity(keys.len() + aggs.len());
        for (k, col) in keys.iter().zip(key_cols.iter()) {
            pairs.push((k.clone(), g.keys_of(col)?));
        }
        for spec in aggs {
            // Count here is the group's rows (R's `n()`), nulls included.
            let out = match spec.op {
                AggOp::Count => g.size(),
                op => g.aggregate(self.column(&spec.col)?, op)?,
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
        self.join_by(
            right,
            &JoinSpec {
                left_on: &[left_on.to_string()],
                right_on: &[right_on.to_string()],
                how,
                suffix: "_right",
                validate: Validate::None,
            },
        )
    }

    /// Hash join on one or more key columns per side (`left_on[j]`
    /// matches `right_on[j]`). Null keys never match. A right column
    /// whose name the left side has takes `suffix`; the right keys are
    /// dropped (a full join's left keys carry the right side's value on
    /// a row only the right side has). `validate` raises when a side
    /// that should hold each key once holds one twice.
    pub fn join_by(&self, right: &Frame, spec: &JoinSpec) -> Result<Frame> {
        let (left_on, right_on, how) = (spec.left_on, spec.right_on, spec.how);
        if left_on.is_empty() || left_on.len() != right_on.len() {
            return Err(OdsError::InvalidArgument(format!(
                "join: {} key column(s) on the left but {} on the right",
                left_on.len(),
                right_on.len()
            )));
        }
        let lcols: Vec<&Series> = left_on.iter().map(|k| self.column(k)).collect::<Result<_>>()?;
        let rcols: Vec<&Series> = right_on.iter().map(|k| right.column(k)).collect::<Result<_>>()?;
        let (lcodes, rcodes, card) = encode_join_keys(&lcols, &rcols);

        // The right side's rows by key code: group `c`'s rows are
        // `rrows[offsets[c]..offsets[c + 1]]`, in row order.
        let mut offsets = vec![0usize; card + 1];
        for &c in &rcodes {
            if c != NO_KEY {
                offsets[c as usize + 1] += 1;
            }
        }
        for c in 0..card {
            offsets[c + 1] += offsets[c];
        }
        let mut next = offsets.clone();
        let mut rrows = vec![0u32; offsets[card]];
        for (r, &c) in rcodes.iter().enumerate() {
            if c != NO_KEY {
                rrows[next[c as usize]] = r as u32;
                next[c as usize] += 1;
            }
        }

        let describe = |cols: &[&Series], row: usize| -> String {
            cols.iter()
                .map(|c| match c.scalar_at(row) {
                    Scalar::Str(s) => format!("{:?}", s),
                    Scalar::F64(x) => x.to_string(),
                    Scalar::I64(x) => x.to_string(),
                    Scalar::Bool(b) => b.to_string(),
                    Scalar::Date(d) => crate::temporal::format_date(d as i64),
                    Scalar::Datetime(t) => crate::temporal::format_datetime(t),
                    Scalar::Null => "()".to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        if matches!(spec.validate, Validate::ManyToOne | Validate::OneToOne)
            && let Some(c) = (0..card).find(|&c| offsets[c + 1] - offsets[c] > 1)
        {
            let r = rrows[offsets[c]] as usize;
            return Err(OdsError::InvalidArgument(format!(
                "join: validate \"{}\" wants each key once on the right, but key ({}) is on {} \
                 rows of it — this join would repeat left rows",
                spec.validate.name(),
                describe(&rcols, r),
                offsets[c + 1] - offsets[c]
            )));
        }
        if matches!(spec.validate, Validate::OneToMany | Validate::OneToOne) {
            let (ids, n_groups) = group_ids(&lcols)?;
            let mut seen = vec![u32::MAX; n_groups];
            for (i, &g) in ids.iter().enumerate() {
                if lcols.iter().any(|c| c.validity().is_some_and(|v| !v.get(i))) {
                    continue;
                }
                if seen[g as usize] != u32::MAX {
                    return Err(OdsError::InvalidArgument(format!(
                        "join: validate \"{}\" wants each key once on the left, but key ({}) is \
                         on rows {} and {} of it",
                        spec.validate.name(),
                        describe(&lcols, i),
                        seen[g as usize],
                        i
                    )));
                }
                seen[g as usize] = i as u32;
            }
        }

        // Semi and anti ask about existence rather than combination, so
        // they answer with a subset of the left frame — no right columns,
        // and no multiplication when a key repeats on the right.
        if matches!(how, JoinHow::Semi | JoinHow::Anti) {
            let want_match = how == JoinHow::Semi;
            let rows: Vec<i64> = (0..lcodes.len())
                .filter(|&i| (lcodes[i] != NO_KEY) == want_match)
                .map(|i| i as i64)
                .collect();
            return self.take(&Series::from_i64(rows));
        }

        // The probe; a full join probes as a left join and then appends
        // what the right side had left over.
        let probe_how = if how == JoinHow::Full { JoinHow::Left } else { how };
        let table = JoinTable { lcodes: &lcodes, offsets: &offsets, rrows: &rrows };
        let (left_idx, right_idx) = join_probe(&table, probe_how, lcodes.len());

        let right_name = |name: &String| -> String {
            if self.names.contains(name) {
                format!("{}{}", name, spec.suffix)
            } else {
                name.clone()
            }
        };

        if how == JoinHow::Full {
            let mut matched = vec![false; right.n_rows()];
            for r in right_idx.iter().flatten() {
                matched[*r] = true;
            }
            // Right rows nothing matched, in row order — including any
            // with a null key, since a null matched nothing.
            let orphans: Vec<usize> = (0..right.n_rows()).filter(|&r| !matched[r]).collect();
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
                match left_on.iter().position(|k| k == name) {
                    // A key column carries the right side's value on an
                    // orphan row: without it the one column that says
                    // which row this is would be null exactly where the
                    // reader needs it.
                    Some(j) => pairs.push((
                        name.clone(),
                        coalesce(
                            &gather_optional(col, &left_rows),
                            &gather_optional(rcols[j], &right_rows),
                        )?,
                    )),
                    None => pairs.push((name.clone(), gather_optional(col, &left_rows))),
                }
            }
            for (name, col) in right.names.iter().zip(right.cols.iter()) {
                if right_on.contains(name) {
                    continue;
                }
                pairs.push((right_name(name), gather_optional(col, &right_rows)));
            }
            return Frame::new(pairs);
        }

        let lidx = Series::from_i64(left_idx);
        let mut pairs: Vec<(String, Series)> = Vec::new();
        for (name, col) in self.names.iter().zip(self.cols.iter()) {
            pairs.push((name.clone(), col.take(&lidx)?));
        }
        for (name, col) in right.names.iter().zip(right.cols.iter()) {
            if right_on.contains(name) {
                continue;
            }
            pairs.push((right_name(name), gather_optional(col, &right_idx)));
        }
        Frame::new(pairs)
    }
}

/// Which right row an as-of join takes for a left row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsOf {
    /// The last right row at or before the left row's value.
    Backward,
    /// The first right row at or after it.
    Forward,
    /// Whichever of those two is closer (backward on a tie).
    Nearest,
}

impl AsOf {
    pub fn parse(name: &str) -> Option<AsOf> {
        Some(match name {
            "backward" => AsOf::Backward,
            "forward" => AsOf::Forward,
            "nearest" => AsOf::Nearest,
            _ => return None,
        })
    }
}

impl Frame {
    /// An as-of join: each left row takes the right row whose `on` value
    /// is the nearest at or before its own (`direction`), among the right
    /// rows with equal `by` keys — the last price before each trade, the
    /// reading in force at each event. `on` is a number or ISO text on
    /// both sides; `tolerance` (numbers only) bounds the distance. Every
    /// left row is kept, in order, with nulls where nothing qualifies.
    /// The right side need not be sorted.
    pub fn asof_join(
        &self,
        right: &Frame,
        on: &str,
        by: &[String],
        direction: AsOf,
        tolerance: Option<f64>,
        suffix: &str,
    ) -> Result<Frame> {
        let (lon, ron) = (self.column(on)?, right.column(on)?);
        // A Date or Datetime compares as its integers (a Date against a
        // Datetime as its midnight); a tolerance is days for a Date,
        // seconds for a Datetime.
        let temporal = |s: &Series| matches!(s.dtype(), crate::DType::Date | crate::DType::Datetime);
        let as_time = |s: &Series| -> Result<Series> {
            let t = if lon.dtype() != ron.dtype() {
                s.cast(crate::DType::Datetime, &|x| x.to_string())?
            } else {
                s.clone()
            };
            Ok(t.physical().expect("temporal"))
        };
        let (lphys, rphys);
        let (lon, ron, tolerance) = if temporal(lon) && temporal(ron) {
            lphys = as_time(lon)?;
            rphys = as_time(ron)?;
            let micros = lon.dtype() == crate::DType::Datetime || ron.dtype() == crate::DType::Datetime;
            (&lphys, &rphys, tolerance.map(|t| if micros { t * 1e6 } else { t }))
        } else {
            (lon, ron, tolerance)
        };
        let numeric = |s: &Series| matches!(s, Series::F64 { .. } | Series::I64 { .. });
        let comparable = (numeric(lon) && numeric(ron))
            || (lon.dtype() == crate::DType::Str && ron.dtype() == crate::DType::Str);
        if !comparable {
            return Err(OdsError::TypeMismatch(format!(
                "asof_join: '{}' must be numbers or ISO text on both sides, got {} and {}",
                on,
                lon.dtype(),
                ron.dtype()
            )));
        }
        if tolerance.is_some() && !numeric(lon) {
            return Err(OdsError::InvalidArgument(
                "asof_join: a tolerance needs a numeric 'on' column".to_string(),
            ));
        }
        let valid = |s: &Series, i: usize| s.validity().is_none_or(|v| v.get(i));
        // The right rows by `by` key, each group's rows sorted by `on`
        // (stable, so equal values keep their row order).
        let (lcodes, rcodes, card) = if by.is_empty() {
            (vec![0u32; self.n_rows()], vec![0u32; right.n_rows()], 1)
        } else {
            let lb: Vec<&Series> = by.iter().map(|k| self.column(k)).collect::<Result<_>>()?;
            let rb: Vec<&Series> = by.iter().map(|k| right.column(k)).collect::<Result<_>>()?;
            encode_join_keys(&lb, &rb)
        };
        let order = ron.argsort()?;
        let Series::I64 { values: order, .. } = order else {
            unreachable!("argsort gives Int positions")
        };
        let mut groups: Vec<Vec<u32>> = vec![Vec::new(); card];
        for &r in order.iter() {
            let r = r as usize;
            if rcodes[r] != NO_KEY && valid(ron, r) {
                groups[rcodes[r] as usize].push(r as u32);
            }
        }
        let lvalid: Vec<bool> = (0..self.n_rows())
            .map(|l| lcodes[l] != NO_KEY && valid(lon, l))
            .collect();
        let t = AsofTable { lvalid: &lvalid, lcodes: &lcodes, groups: &groups, direction, tolerance };
        let right_idx: Vec<Option<usize>> = match (lon, ron) {
            (Series::I64 { values: a, .. }, Series::I64 { values: b, .. }) => {
                t.pick(|l| a[l], |r| b[r], Some(|x: i64, y: i64| (x as f64 - y as f64).abs()))
            }
            (Series::Str { col: a, .. }, Series::Str { col: b, .. }) => {
                t.pick(|l| a.get(l), |r| b.get(r), None::<fn(&str, &str) -> f64>)
            }
            _ => {
                let num = |s: &Series, i: usize| -> f64 {
                    match s {
                        Series::F64 { values, .. } => values[i],
                        Series::I64 { values, .. } => values[i] as f64,
                        _ => f64::NAN,
                    }
                };
                t.pick(|l| num(lon, l), |r| num(ron, r), Some(|x: f64, y: f64| (x - y).abs()))
            }
        };

        let mut pairs: Vec<(String, Series)> = self
            .names
            .iter()
            .cloned()
            .zip(self.cols.iter().cloned())
            .collect();
        for (name, col) in right.names.iter().zip(right.cols.iter()) {
            if name == on || by.contains(name) {
                continue;
            }
            let out = if self.names.contains(name) {
                format!("{}{}", name, suffix)
            } else {
                name.clone()
            };
            pairs.push((out, gather_optional(col, &right_idx)));
        }
        Frame::new(pairs)
    }
}

/// The built right side of an as-of join.
struct AsofTable<'a> {
    lvalid: &'a [bool],
    lcodes: &'a [u32],
    /// Each `by` group's right rows, sorted by `on`.
    groups: &'a [Vec<u32>],
    direction: AsOf,
    tolerance: Option<f64>,
}

impl AsofTable<'_> {
    /// Each left row's right row. A left side sorted by `on` (the usual
    /// case: events in time) walks each group's rows once, a cursor per
    /// group; otherwise each row is two binary searches.
    fn pick<K: PartialOrd + Copy>(
        &self,
        lk: impl Fn(usize) -> K,
        rk: impl Fn(usize) -> K,
        dist: Option<impl Fn(K, K) -> f64>,
    ) -> Vec<Option<usize>> {
        let n = self.lvalid.len();
        let mut prev: Option<K> = None;
        let sorted = (0..n).filter(|&l| self.lvalid[l]).all(|l| {
            let x = lk(l);
            let ok = prev.is_none_or(|p| p <= x);
            prev = Some(x);
            ok
        });
        // Per group: how many rows are at or below the value (`le`), and
        // how many strictly below (`lt`).
        let mut le = vec![0usize; self.groups.len()];
        let mut lt = vec![0usize; self.groups.len()];
        (0..n)
            .map(|l| {
                if !self.lvalid[l] {
                    return None;
                }
                let g = self.lcodes[l] as usize;
                let rows = &self.groups[g];
                let x = lk(l);
                let want_back = self.direction != AsOf::Forward;
                let want_fwd = self.direction != AsOf::Backward;
                let (after, at_or_after) = if sorted {
                    if want_back {
                        while le[g] < rows.len() && rk(rows[le[g]] as usize) <= x {
                            le[g] += 1;
                        }
                    }
                    if want_fwd {
                        while lt[g] < rows.len() && rk(rows[lt[g]] as usize) < x {
                            lt[g] += 1;
                        }
                    }
                    (le[g], lt[g])
                } else {
                    (
                        if want_back { rows.partition_point(|&r| rk(r as usize) <= x) } else { 0 },
                        if want_fwd { rows.partition_point(|&r| rk(r as usize) < x) } else { rows.len() },
                    )
                };
                let back = (after > 0).then(|| rows[after - 1] as usize);
                let fwd = rows.get(at_or_after).map(|&r| r as usize);
                let d = |r: usize| dist.as_ref().map(|f| f(x, rk(r)));
                let chosen = match self.direction {
                    AsOf::Backward => back,
                    AsOf::Forward => fwd,
                    AsOf::Nearest => match (back, fwd) {
                        (Some(b), Some(f)) => Some(match (d(b), d(f)) {
                            (Some(db), Some(df)) if df < db => f,
                            _ => b,
                        }),
                        (b, f) => b.or(f),
                    },
                };
                chosen.filter(|&r| match (self.tolerance, d(r)) {
                    (Some(t), Some(dr)) => dr <= t,
                    _ => true,
                })
            })
            .collect()
    }
}

/// A join's keys and options.
#[derive(Clone, Debug)]
pub struct JoinSpec<'a> {
    pub left_on: &'a [String],
    pub right_on: &'a [String],
    pub how: JoinHow,
    /// Appended to a right column's name that the left side has.
    pub suffix: &'a str,
    pub validate: Validate,
}

/// Which side of a join must hold each key once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validate {
    None,
    /// Both sides.
    OneToOne,
    /// The right side (a lookup table): the join never repeats a left row.
    ManyToOne,
    /// The left side.
    OneToMany,
}

impl Validate {
    /// `"one_to_one"` (`"1:1"`), `"many_to_one"` (`"m:1"`),
    /// `"one_to_many"` (`"1:m"`), or `"many_to_many"` (`"m:m"`, no check).
    pub fn parse(name: &str) -> Option<Validate> {
        Some(match name {
            "one_to_one" | "1:1" => Validate::OneToOne,
            "many_to_one" | "m:1" => Validate::ManyToOne,
            "one_to_many" | "1:m" => Validate::OneToMany,
            "many_to_many" | "m:m" => Validate::None,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Validate::None => "many_to_many",
            Validate::OneToOne => "one_to_one",
            Validate::ManyToOne => "many_to_one",
            Validate::OneToMany => "one_to_many",
        }
    }
}

/// A row whose key is null, or (on the left) matches nothing.
const NO_KEY: u32 = u32::MAX;

/// One key column per side encoded against the right side's values:
/// each right row's code (dense, first-seen), each left row's code of
/// the equal right value (`NO_KEY` when there is none or the key is
/// null), and the number of codes. The common types encode without a
/// per-row allocation; the rest through `Key`, whose types never match
/// across (an Int key never equals a Float one).
fn encode_join_pair(l: &Series, r: &Series) -> (Vec<u32>, Vec<u32>, usize) {
    fn run<K: std::hash::Hash + Eq>(
        nl: usize,
        nr: usize,
        lk: impl Fn(usize) -> Option<K>,
        rk: impl Fn(usize) -> Option<K>,
    ) -> (Vec<u32>, Vec<u32>, usize) {
        let mut dict: FxMap<K, u32> = FxMap::default();
        let mut rcodes = Vec::with_capacity(nr);
        for i in 0..nr {
            rcodes.push(match rk(i) {
                None => NO_KEY,
                Some(k) => {
                    let next = dict.len() as u32;
                    *dict.entry(k).or_insert(next)
                }
            });
        }
        let lcodes = (0..nl)
            .map(|i| lk(i).and_then(|k| dict.get(&k).copied()).unwrap_or(NO_KEY))
            .collect();
        (lcodes, rcodes, dict.len())
    }
    let valid = |s: &Series, i: usize| s.validity().is_none_or(|v| v.get(i));
    let (nl, nr) = (l.len(), r.len());
    match (l, r) {
        // Two categories of one level list match by code; a category
        // and text (or another list) by text.
        (
            Series::Cat {
                codes: a,
                levels: la,
                ..
            },
            Series::Cat {
                codes: b,
                levels: lb,
                ..
            },
        ) if crate::category::same_levels(la, lb) => {
            return run(
                nl,
                nr,
                |i| valid(l, i).then(|| a[i]),
                |i| valid(r, i).then(|| b[i]),
            );
        }
        (Series::Cat { .. }, _) | (_, Series::Cat { .. }) => {
            return encode_join_pair(&l.key_view(), &r.key_view());
        }
        (Series::Date { values: a, .. }, Series::Date { values: b, .. }) => {
            return run(
                nl,
                nr,
                |i| valid(l, i).then(|| a[i]),
                |i| valid(r, i).then(|| b[i]),
            );
        }
        (Series::Datetime { values: a, .. }, Series::Datetime { values: b, .. }) => {
            return run(
                nl,
                nr,
                |i| valid(l, i).then(|| a[i]),
                |i| valid(r, i).then(|| b[i]),
            );
        }
        _ => {}
    }
    match (l, r) {
        (Series::I64 { values: a, .. }, Series::I64 { values: b, .. }) => run(
            nl,
            nr,
            |i| valid(l, i).then(|| a[i]),
            |i| valid(r, i).then(|| b[i]),
        ),
        (Series::Str { col: a, .. }, Series::Str { col: b, .. }) => run(
            nl,
            nr,
            |i| valid(l, i).then(|| a.get(i)),
            |i| valid(r, i).then(|| b.get(i)),
        ),
        (Series::F64 { values: a, .. }, Series::F64 { values: b, .. }) => {
            let bits = |x: f64| if x.is_nan() { f64::NAN.to_bits() } else { x.to_bits() };
            run(
                nl,
                nr,
                |i| valid(l, i).then(|| bits(a[i])),
                |i| valid(r, i).then(|| bits(b[i])),
            )
        }
        _ => run(nl, nr, |i| key_at(l, i), |i| key_at(r, i)),
    }
}

/// Every key column of a join encoded into one code per row: the
/// columns' codes combined a column at a time and made dense again, so
/// a code stays a `u32` however many keys there are.
fn encode_join_keys(lcols: &[&Series], rcols: &[&Series]) -> (Vec<u32>, Vec<u32>, usize) {
    let (mut lc, mut rc, mut card) = encode_join_pair(lcols[0], rcols[0]);
    for (l, r) in lcols.iter().zip(rcols).skip(1) {
        let (l2, r2, c2) = encode_join_pair(l, r);
        let pair = |a: u32, b: u32| (a as u64) * (c2 as u64) + b as u64;
        let mut dict: FxMap<u64, u32> = FxMap::default();
        for (a, b) in rc.iter_mut().zip(&r2) {
            *a = if *a == NO_KEY || *b == NO_KEY {
                NO_KEY
            } else {
                let next = dict.len() as u32;
                *dict.entry(pair(*a, *b)).or_insert(next)
            };
        }
        for (a, b) in lc.iter_mut().zip(&l2) {
            *a = if *a == NO_KEY || *b == NO_KEY {
                NO_KEY
            } else {
                dict.get(&pair(*a, *b)).copied().unwrap_or(NO_KEY)
            };
        }
        card = dict.len();
    }
    (lc, rc, card)
}

/// The built right side of a join, by key code.
struct JoinTable<'a> {
    lcodes: &'a [u32],
    offsets: &'a [usize],
    rrows: &'a [u32],
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
        // Numbers and Bools: one copy of the kept values, the validity
        // shifted a word at a time.
        let n = self.len();
        let k = (by.unsigned_abs() as usize).min(n);
        fn moved<T: Copy + Default>(v: &[T], by: i64, k: usize) -> Vec<T> {
            let n = v.len();
            let mut out = Vec::with_capacity(n);
            if by >= 0 {
                out.extend(std::iter::repeat_n(T::default(), k));
                out.extend_from_slice(&v[..n - k]);
            } else {
                out.extend_from_slice(&v[k..]);
                out.extend(std::iter::repeat_n(T::default(), k));
            }
            out
        }
        let validity = || shifted_validity(self.validity(), n, by);
        match self {
            Series::F64 { values, .. } => {
                return Ok(Series::F64 {
                    values: std::sync::Arc::new(moved(values, by, k)),
                    validity: validity(),
                });
            }
            Series::I64 { values, .. } => {
                return Ok(Series::I64 {
                    values: std::sync::Arc::new(moved(values, by, k)),
                    validity: validity(),
                });
            }
            Series::Bool { values, .. } => {
                return Ok(Series::Bool {
                    values: std::sync::Arc::new(moved(values, by, k)),
                    validity: validity(),
                });
            }
            Series::Date { values, .. } => {
                return Ok(Series::Date {
                    values: std::sync::Arc::new(moved(values, by, k)),
                    validity: validity(),
                });
            }
            Series::Datetime { values, .. } => {
                return Ok(Series::Datetime {
                    values: std::sync::Arc::new(moved(values, by, k)),
                    validity: validity(),
                });
            }
            Series::Cat { codes, levels, .. } => {
                return Ok(Series::Cat {
                    codes: std::sync::Arc::new(moved(codes, by, k)),
                    levels: levels.clone(),
                    validity: validity(),
                });
            }
            Series::Str { .. } => {}
        }
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
        if !matches!(self.dtype(), crate::DType::F64 | crate::DType::I64) {
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

    // `rolling` lives in window.rs.
}

/// Value equality for ranking, where two nulls never reach this and NaN
/// is not a concern (argsort has already ordered them).
fn scalars_equal(a: Scalar, b: Scalar) -> bool {
    match (a, b) {
        (Scalar::F64(x), Scalar::F64(y)) => x == y,
        (Scalar::I64(x), Scalar::I64(y)) => x == y,
        (Scalar::Bool(x), Scalar::Bool(y)) => x == y,
        (Scalar::Str(x), Scalar::Str(y)) => x == y,
        (Scalar::Date(x), Scalar::Date(y)) => x == y,
        (Scalar::Datetime(x), Scalar::Datetime(y)) => x == y,
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

/// Columns of one type laid end to end (a Category's levels merged).
pub fn stack(cols: &[&Series]) -> Series {
    if cols.len() == 1 {
        return cols[0].clone();
    }
    stack_same_dtype(cols)
}

/// Lay same-typed columns end to end. `unpivot` has already established
/// that they share a type; this is the copy that follows.
fn stack_same_dtype(cols: &[&Series]) -> Series {
    if cols[0].dtype() == crate::DType::Category {
        return crate::category::stack(cols).expect("categories, checked");
    }
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
        crate::DType::Date => Series::from_date_options(
            scalars()
                .map(|v| match v {
                    Scalar::Date(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::Datetime => Series::from_datetime_options(
            scalars()
                .map(|v| match v {
                    Scalar::Datetime(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::Category => unreachable!("stacked above"),
    }
}

/// A scalar as a column name. Only reached for `pivot`, where the values
/// of one column become the names of many.
pub(crate) fn scalar_name(v: Scalar, fmt_f64: &dyn Fn(f64) -> String) -> String {
    match v {
        Scalar::Str(s) => s,
        Scalar::I64(x) => x.to_string(),
        Scalar::F64(x) => fmt_f64(x),
        Scalar::Bool(b) => b.to_string(),
        Scalar::Date(d) => crate::temporal::format_date(d as i64),
        Scalar::Datetime(t) => crate::temporal::format_datetime(t),
        // Refused before this is reached.
        Scalar::Null => "null".to_string(),
    }
}

// ---------------------------------------------------------------------
// Join probe
// ---------------------------------------------------------------------

/// Left frames at or above this many rows probe in parallel. Below it, the
/// thread hand-off costs more than the probe saves (benches/crossover.rs:
/// 0.88x at 50,000 rows, 1.25x at 250,000).
#[cfg(feature = "parallel")]
const PAR_JOIN_ROWS: usize = 200_000;

/// Below this many gathered rows, Frame::take (and filter) stays
/// sequential: a column apiece gains nothing on a narrow frame below it
/// (0.86x at 100,000 rows, 1.04x at 500,000).
#[cfg(feature = "parallel")]
const PAR_TAKE_ROWS: usize = 500_000;

/// Probe left rows `[lo, hi)` against the built right side, producing
/// `(left_idx, right_idx)` match pairs in left-row order (a matched left
/// row repeats once per right match, in right-row order; an unmatched
/// row is kept with `None` only for a left join). Read-only over the
/// table, so it runs over disjoint ranges on separate threads.
fn probe_join(t: &JoinTable, how: JoinHow, lo: usize, hi: usize) -> (Vec<i64>, Vec<Option<usize>>) {
    let mut left_idx: Vec<i64> = Vec::new();
    let mut right_idx: Vec<Option<usize>> = Vec::new();
    for i in lo..hi {
        let c = t.lcodes[i];
        if c != NO_KEY {
            for &r in &t.rrows[t.offsets[c as usize]..t.offsets[c as usize + 1]] {
                left_idx.push(i as i64);
                right_idx.push(Some(r as usize));
            }
        } else if how == JoinHow::Left {
            left_idx.push(i as i64);
            right_idx.push(None);
        }
    }
    (left_idx, right_idx)
}

/// Probe all `n` left rows, in parallel over row-chunks when the frame is
/// large and the `parallel` feature is on. Chunks are concatenated in row
/// order, so the output is identical to the sequential probe.
fn join_probe(t: &JoinTable, how: JoinHow, n: usize) -> (Vec<i64>, Vec<Option<usize>>) {
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
            .map(|&(lo, hi)| probe_join(t, how, lo, hi))
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
    probe_join(t, how, 0, n)
}

/// The row index where each group first appears, indexed by group id.
/// `group_ids_single`/`_multi` assign ids in first-seen order, so this is
/// also sorted ascending — but the verbs below reorder groups, and rely
/// on being able to ask for a specific group's first row.
pub(crate) fn first_row_per_group(group_ids: &[u32], n_groups: usize) -> Vec<i64> {
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

/// A column's summary, as a host draws it over a table's column: its
/// nulls and distinct values, and a number's range, mean and histogram or
/// any other column's most frequent values. Over the first `upto` rows.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnSummary {
    /// The rows summarised (at most `upto`).
    pub rows: usize,
    pub nulls: usize,
    /// Distinct values, nulls not among them.
    pub distinct: usize,
    /// An Int or Float column.
    pub numeric: bool,
    /// A number's least, most and mean over its finite values; None when
    /// it has none.
    pub range: Option<(f64, f64, f64)>,
    /// A number's histogram: equal-width bins over [min, max] (every
    /// value in the first when they are equal); nulls not counted.
    pub hist: Vec<u64>,
    /// Any other column's most frequent values: (a row holding it, its
    /// count), most first, ties by first appearance.
    pub top: Vec<(usize, usize)>,
}

impl Series {
    /// The column's summary over its first `upto` rows: `bins` histogram
    /// bins for a number, the `top` most frequent values otherwise.
    pub fn summarize(&self, upto: usize, bins: usize, top: usize) -> ColumnSummary {
        let m = self.len().min(upto);
        let valid = |i: usize| self.validity().map(|v| v.get(i)).unwrap_or(true);
        let nulls = match self.validity() {
            None => 0,
            Some(_) => (0..m).filter(|&i| !valid(i)).count(),
        };
        let mut out = ColumnSummary {
            rows: m,
            nulls,
            distinct: 0,
            numeric: false,
            range: None,
            hist: Vec::new(),
            top: Vec::new(),
        };
        match self {
            // A date's range and histogram over its integers; the host
            // writes them as dates (it knows the column's type).
            Series::Date { .. } | Series::Datetime { .. } => {
                return self
                    .physical()
                    .expect("temporal")
                    .summarize(upto, bins, top);
            }
            Series::Cat { codes, levels, .. } => {
                let mut counts: Vec<(usize, usize)> = vec![(usize::MAX, 0); levels.len()];
                for i in 0..m {
                    if valid(i) {
                        let slot = &mut counts[codes[i] as usize];
                        if slot.0 == usize::MAX {
                            slot.0 = i;
                        }
                        slot.1 += 1;
                    }
                }
                let mut all: Vec<(usize, usize)> =
                    counts.into_iter().filter(|c| c.1 > 0).collect();
                out.distinct = all.len();
                all.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                all.truncate(top);
                out.top = all;
            }
            Series::F64 { .. } | Series::I64 { .. } => {
                out.numeric = true;
                let bins = bins.max(1);
                let at = |i: usize| -> f64 {
                    match self {
                        Series::F64 { values, .. } => values[i],
                        Series::I64 { values, .. } => values[i] as f64,
                        _ => unreachable!(),
                    }
                };
                // distinct values by sorting their bits: a hash of float
                // bits clusters (the fast hasher mixes the low bits), a
                // sort does not care
                let mut seen: Vec<u64> = Vec::with_capacity(m - nulls);
                let (mut lo, mut hi, mut sum, mut n) =
                    (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0usize);
                for i in 0..m {
                    if !valid(i) {
                        continue;
                    }
                    match self {
                        Series::I64 { values, .. } => seen.push(values[i] as u64),
                        _ => {
                            let x = at(i);
                            seen.push(if x.is_nan() {
                                f64::NAN.to_bits()
                            } else {
                                (x + 0.0).to_bits()
                            });
                        }
                    }
                    let x = at(i);
                    if x.is_finite() {
                        lo = lo.min(x);
                        hi = hi.max(x);
                        sum += x;
                        n += 1;
                    }
                }
                seen.sort_unstable();
                seen.dedup();
                out.distinct = seen.len();
                out.hist = vec![0; bins];
                if n > 0 {
                    out.range = Some((lo, hi, sum / n as f64));
                    let span = hi - lo;
                    for i in 0..m {
                        if !valid(i) {
                            continue;
                        }
                        let x = at(i);
                        if !x.is_finite() {
                            continue;
                        }
                        let b = if span > 0.0 {
                            (((x - lo) / span) * bins as f64) as usize
                        } else {
                            0
                        };
                        out.hist[b.min(bins - 1)] += 1;
                    }
                }
            }
            Series::Str { col, .. } => {
                let mut counts: FxMap<&str, (usize, usize)> =
                    FxMap::with_capacity_and_hasher((m / 4).min(1 << 18), Default::default());
                for i in 0..m {
                    if valid(i) {
                        counts.entry(col.get(i)).or_insert((i, 0)).1 += 1;
                    }
                }
                out.distinct = counts.len();
                let mut all: Vec<(usize, usize)> = counts.into_values().collect();
                all.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                all.truncate(top);
                out.top = all;
            }
            Series::Bool { values, .. } => {
                // (first row, count) of false and of true
                let mut slots: [Option<(usize, usize)>; 2] = [None, None];
                for i in 0..m {
                    if valid(i) {
                        slots[values[i] as usize].get_or_insert((i, 0)).1 += 1;
                    }
                }
                let mut all: Vec<(usize, usize)> = slots.into_iter().flatten().collect();
                out.distinct = all.len();
                all.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                all.truncate(top);
                out.top = all;
            }
        }
        out
    }
}

// ---------------------------------------------------------------------
// Group keys
// ---------------------------------------------------------------------

/// A hashable group/join key. Floats key by bit pattern (all NaNs are
/// one group).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    I(i64),
    F(u64),
    B(bool),
    S(String),
    D(i32),
    T(i64),
}

pub(crate) fn key_at(s: &Series, i: usize) -> Option<Key> {
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
        Scalar::Date(d) => Some(Key::D(d)),
        Scalar::Datetime(t) => Some(Key::T(t)),
    }
}

const NULL_GROUP: u64 = u64::MAX;

/// Group ids for a single key column, first-seen order. The dense i64
/// and string paths avoid per-row Scalar boxing — group_by's hot loop.
/// Parallel dictionary encoding: chunks build local dictionaries and
/// local ids concurrently; the local dictionaries then merge into the
/// global one in chunk order — which reproduces sequential first-seen
/// id order exactly, because a key's global first occurrence lies in
/// the earliest chunk containing it, and both chunk order and
/// within-chunk order are first-seen — and a final parallel pass
/// translates local ids through each chunk's translation table.
#[cfg(feature = "parallel")]
fn group_ids_par<K>(n: usize, key_of: impl Fn(usize) -> Option<K> + Sync) -> (Vec<u32>, usize)
where
    K: std::hash::Hash + Eq + Copy + Send + Sync,
{
    use rayon::prelude::*;
    let workers = rayon::current_num_threads().max(1);
    let chunk = n.div_ceil(workers).max(1);
    let ranges: Vec<(usize, usize)> = (0..n.div_ceil(chunk))
        .map(|w| (w * chunk, ((w + 1) * chunk).min(n)))
        .collect();

    let locals: Vec<(Vec<u32>, Vec<Option<K>>)> = ranges
        .par_iter()
        .map(|&(lo, hi)| {
            let mut map: FxMap<Option<K>, u32> = FxMap::default();
            let mut order: Vec<Option<K>> = Vec::new();
            let mut local = Vec::with_capacity(hi - lo);
            for i in lo..hi {
                let k = key_of(i);
                let next = order.len() as u32;
                let id = *map.entry(k).or_insert_with(|| {
                    order.push(k);
                    next
                });
                local.push(id);
            }
            (local, order)
        })
        .collect();

    let mut global: FxMap<Option<K>, u32> = FxMap::default();
    let mut next: u32 = 0;
    let trans: Vec<Vec<u32>> = locals
        .iter()
        .map(|(_, order)| {
            order
                .iter()
                .map(|k| {
                    *global.entry(*k).or_insert_with(|| {
                        let id = next;
                        next += 1;
                        id
                    })
                })
                .collect()
        })
        .collect();

    let mut ids = vec![0u32; n];
    ids.par_chunks_mut(chunk)
        .zip(locals.par_iter().zip(trans.par_iter()))
        .for_each(|(out, ((local, _), tr))| {
            for (o, &l) in out.iter_mut().zip(local.iter()) {
                *o = tr[l as usize];
            }
        });
    (ids, next as usize)
}

pub(crate) fn group_ids_single(key: &Series) -> (Vec<u32>, usize) {
    let n = key.len();
    match key {
        // A date groups as its integer.
        Series::Date { .. } | Series::Datetime { .. } => {
            return group_ids_single(&key.physical().expect("temporal"));
        }
        // A category's codes are already a dictionary: first-seen order
        // through a table as long as its levels, no hashing.
        Series::Cat {
            codes,
            levels,
            validity,
        } => {
            let null = levels.len();
            let mut map = vec![u32::MAX; null + 1];
            let mut next = 0u32;
            let mut ids = Vec::with_capacity(n);
            for (i, &c) in codes.iter().enumerate() {
                let k = if validity.as_ref().is_none_or(|v| v.get(i)) {
                    c as usize
                } else {
                    null
                };
                let slot = &mut map[k];
                if *slot == u32::MAX {
                    *slot = next;
                    next += 1;
                }
                ids.push(*slot);
            }
            return (ids, next as usize);
        }
        _ => {}
    }

    // The hot key types encode across all cores on large columns; the
    // sequential arms below serve small columns and the rare types.
    #[cfg(feature = "parallel")]
    if n >= PAR_GROUPBY_ROWS && crate::parallel_enabled() {
        match key {
            Series::I64 { values, validity } => {
                let valid = |i: usize| validity.as_ref().map(|b| b.get(i)).unwrap_or(true);
                return group_ids_par(n, |i| valid(i).then(|| values[i]));
            }
            Series::Str { col, validity } => {
                let valid = |i: usize| validity.as_ref().map(|b| b.get(i)).unwrap_or(true);
                return group_ids_par(n, |i| valid(i).then(|| col.get(i)));
            }
            _ => {}
        }
    }

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

pub(crate) fn group_ids_multi(key_cols: &[&Series], n: usize) -> (Vec<u32>, usize) {
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
/// hand-off costs more than the scatter saves (benches/crossover.rs: 0.40x
/// at 100,000 rows, 0.75x at 250,000, 1.17x at 500,000).
#[cfg(feature = "parallel")]
const PAR_GROUPBY_ROWS: usize = 500_000;
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

pub(crate) fn aggregate_column(col: &Series, group_ids: &[u32], n_groups: usize, op: AggOp) -> Result<Series> {
    match col {
        // The earliest and latest of a group's dates.
        Series::Date { .. } | Series::Datetime { .. } if matches!(op, AggOp::Min | AggOp::Max) => {
            let phys = col.physical().expect("temporal");
            Series::from_physical(col.dtype(), aggregate_column(&phys, group_ids, n_groups, op)?)
        }
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
                _ => unreachable!("handled by caller"),
            }
        }
        _ => Err(OdsError::TypeMismatch(format!(
            "cannot aggregate {} over a {} column",
            op.name(),
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
        _ => unreachable!("handled by caller"),
    };
    Ok(out)
}

/// Each group's rows, in row order, by a counting sort over the group
/// ids: group `g`'s rows are `rows[offsets[g]..offsets[g + 1]]`.
pub(crate) fn rows_by_group(group_ids: &[u32], n_groups: usize) -> (Vec<usize>, Vec<u32>) {
    let mut offsets = vec![0usize; n_groups + 1];
    for &g in group_ids {
        offsets[g as usize + 1] += 1;
    }
    for g in 0..n_groups {
        offsets[g + 1] += offsets[g];
    }
    let mut next = offsets.clone();
    let mut rows = vec![0u32; group_ids.len()];
    for (i, &g) in group_ids.iter().enumerate() {
        rows[next[g as usize]] = i as u32;
        next[g as usize] += 1;
    }
    (offsets, rows)
}

/// The last row index of each group.
pub(crate) fn last_row_per_group(group_ids: &[u32], n_groups: usize) -> Vec<i64> {
    let mut last = vec![0i64; n_groups];
    for (i, &g) in group_ids.iter().enumerate() {
        last[g as usize] = i as i64;
    }
    last
}

/// One value per group, each computed from that group's rows alone — so
/// fanning the groups out over threads gives exactly the sequential
/// answer: no group's arithmetic depends on how the others are split.
pub(crate) fn per_group<T: Send>(
    n_rows: usize,
    n_groups: usize,
    f: impl Fn(usize) -> T + Sync + Send,
) -> Vec<T> {
    #[cfg(feature = "parallel")]
    if n_rows >= PAR_GROUPBY_ROWS && n_groups > 1 && crate::parallel_enabled() {
        use rayon::prelude::*;
        return (0..n_groups).into_par_iter().map(f).collect();
    }
    let _ = n_rows;
    (0..n_groups).map(f).collect()
}

/// The aggregations that need a group's values together: median, std,
/// var (numeric columns; nulls skipped) and n_unique (any column; null
/// is one value, as `Series::n_unique` counts it).
pub(crate) fn aggregate_by_rows(
    col: &Series,
    group_ids: &[u32],
    groups: &(Vec<usize>, Vec<u32>),
    op: AggOp,
) -> Result<Series> {
    let (offsets, rows) = groups;
    let n_groups = offsets.len() - 1;
    let n = group_ids.len();
    let rows_of = |g: usize| &rows[offsets[g]..offsets[g + 1]];
    if op == AggOp::NUnique {
        // Value ids over the whole column, then each group's distinct
        // ids: a mark per value id says which group last counted it.
        let (value_ids, n_values) = group_ids_single(col);
        let mut mark = vec![u32::MAX; n_values];
        let mut counts = vec![0i64; n_groups];
        for g in 0..n_groups {
            for &r in rows_of(g) {
                let v = value_ids[r as usize] as usize;
                if mark[v] != g as u32 {
                    mark[v] = g as u32;
                    counts[g] += 1;
                }
            }
        }
        return Ok(Series::from_i64(counts));
    }
    let value: Box<dyn Fn(u32) -> Option<f64> + Sync + Send + '_> = match col {
        Series::F64 { values, validity } => Box::new(move |r: u32| {
            let r = r as usize;
            validity
                .as_ref()
                .is_none_or(|v| v.get(r))
                .then(|| values[r])
        }),
        Series::I64 { values, validity } => Box::new(move |r: u32| {
            let r = r as usize;
            validity
                .as_ref()
                .is_none_or(|v| v.get(r))
                .then(|| values[r] as f64)
        }),
        _ => {
            return Err(OdsError::TypeMismatch(format!(
                "cannot aggregate {} over a {} column",
                op.name(),
                col.dtype()
            )));
        }
    };
    let out = per_group(n, n_groups, |g| {
        let mut vals: Vec<f64> = rows_of(g).iter().filter_map(|&r| value(r)).collect();
        match op {
            AggOp::Median => {
                if vals.is_empty() {
                    return None;
                }
                // `Series::quantile(0.5)`'s arithmetic, so a group's
                // median is the column's median of the same rows.
                // The two middle order statistics by selection, not a
                // full sort: the same values a sort would put there.
                let h = (vals.len() - 1) as f64 * 0.5;
                let (lo, hi) = (h.floor() as usize, h.ceil() as usize);
                let (_, &mut a, rest) = vals.select_nth_unstable_by(lo, f64::total_cmp);
                let b = if hi == lo {
                    a
                } else {
                    *rest
                        .iter()
                        .min_by(|x, y| x.total_cmp(y))
                        .expect("an even count has an upper middle")
                };
                Some(a + (b - a) * (h - lo as f64))
            }
            AggOp::Quantile(_) => {
                if vals.is_empty() {
                    return None;
                }
                // `Series::quantile`'s interpolation over the group's
                // sorted values.
                let q = op.q().expect("a quantile");
                vals.sort_unstable_by(f64::total_cmp);
                let h = (vals.len() - 1) as f64 * q;
                let (lo, hi) = (h.floor() as usize, h.ceil() as usize);
                Some(vals[lo] + (vals[hi] - vals[lo]) * (h - lo as f64))
            }
            _ => {
                if vals.len() < 2 {
                    return None;
                }
                let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                let ss: f64 = vals.iter().map(|x| (x - mean) * (x - mean)).sum();
                let var = ss / (vals.len() - 1) as f64;
                Some(if op == AggOp::Std { var.sqrt() } else { var })
            }
        }
    });
    Ok(Series::from_f64_options(out))
}

/// Gather with optional indices (join's right side): None -> null.
pub(crate) fn gather_optional(col: &Series, idx: &[Option<usize>]) -> Series {
    // Values and validity written apart: no Option per row.
    fn dense<T: Copy + Default>(values: &[T], col: &Series, idx: &[Option<usize>]) -> (Vec<T>, Option<Bitmap>) {
        let mut out = Vec::with_capacity(idx.len());
        let mut valid = Vec::with_capacity(idx.len());
        for o in idx {
            match o.filter(|&i| col_valid(col, i)) {
                Some(i) => {
                    out.push(values[i]);
                    valid.push(true);
                }
                None => {
                    out.push(T::default());
                    valid.push(false);
                }
            }
        }
        let bm = (!valid.iter().all(|&v| v)).then(|| Bitmap::from_bools(&valid));
        (out, bm)
    }
    match col {
        Series::Date { .. } | Series::Datetime { .. } | Series::Cat { .. } => {
            let valid: Vec<bool> = idx
                .iter()
                .map(|o| o.is_some_and(|i| col_valid(col, i)))
                .collect();
            let rows: Vec<usize> = idx.iter().map(|o| o.unwrap_or(0)).collect();
            let validity = (!valid.iter().all(|&v| v)).then(|| Bitmap::from_bools(&valid));
            if col.is_empty() {
                // Nothing to gather from: every slot is null.
                return match col {
                    Series::Date { .. } => Series::from_date_options(vec![None; idx.len()]),
                    Series::Datetime { .. } => Series::from_datetime_options(vec![None; idx.len()]),
                    _ => Series::from_str_options(vec![None; idx.len()])
                        .cast(crate::DType::Category, &|x| x.to_string())
                        .expect("text to a category"),
                };
            }
            col.gather_under(&rows, validity)
        }
        Series::F64 { values, .. } => {
            let (v, b) = dense(values, col, idx);
            Series::F64 { values: std::sync::Arc::new(v), validity: b }
        }
        Series::I64 { values, .. } => {
            let (v, b) = dense(values, col, idx);
            Series::I64 { values: std::sync::Arc::new(v), validity: b }
        }
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
pub(crate) fn coalesce(a: &Series, b: &Series) -> Result<Series> {
    if a.dtype() != b.dtype() {
        return Err(OdsError::InvalidArgument(format!(
            "join: a full join needs both key columns to have the same type, \
             got {} on the left and {} on the right",
            a.dtype(),
            b.dtype()
        )));
    }
    if a.dtype() == crate::DType::Category {
        let text = coalesce(&a.key_view(), &b.key_view())?;
        return crate::category::to_category(&text, None, &|x| x.to_string());
    }
    let pick = |i: usize| match a.scalar_at(i) {
        Scalar::Null => b.scalar_at(i),
        v => v,
    };
    let n = a.len();
    Ok(match a.dtype() {
        crate::DType::Date => Series::from_date_options(
            (0..n)
                .map(|i| match pick(i) {
                    Scalar::Date(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::Datetime => Series::from_datetime_options(
            (0..n)
                .map(|i| match pick(i) {
                    Scalar::Datetime(x) => Some(x),
                    _ => None,
                })
                .collect(),
        ),
        crate::DType::Category => unreachable!("decoded above"),
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
    col.validity().is_none_or(|v| v.get(i))
}

// ---------------------------------------------------------------------
// Group aggregation broadcast back (`over`) and multi-key sorting
// ---------------------------------------------------------------------

/// Each row's group id over `keys` (first-seen order) and the number of
/// groups: the dictionary encoding `group_by` runs, reusable.
pub fn group_ids(keys: &[&Series]) -> Result<(Vec<u32>, usize)> {
    let Some(first) = keys.first() else {
        return Err(OdsError::InvalidArgument(
            "a grouping needs at least one key column".to_string(),
        ));
    };
    let n = first.len();
    if let Some(bad) = keys.iter().find(|k| k.len() != n) {
        return Err(OdsError::LengthMismatch {
            left: n,
            right: bad.len(),
        });
    }
    Ok(if keys.len() == 1 {
        group_ids_single(first)
    } else {
        group_ids_multi(keys, n)
    })
}

/// `op` of `col` within each group of `keys`, given back to every row
/// of the group (pandas' `transform`, Polars' `over`). `Count` counts
/// the group's valid values, as `ods.count` does.
pub fn over(col: &Series, keys: &[&Series], op: AggOp) -> Result<Series> {
    let g = crate::group::Grouping::new(keys)?;
    g.broadcast(&g.aggregate(col, op)?)
}

/// A key's dense rank per row: equal values share a rank, nulls rank
/// after every value whatever the direction, and `descending` reverses
/// the values' order.
fn key_ranks(key: &Series, descending: bool) -> Result<Vec<u32>> {
    let n = key.len();
    let order = match key {
        Series::Bool { values, validity } => {
            // false < true; nulls last.
            let valid = |i: usize| validity.as_ref().is_none_or(|v| v.get(i));
            let mut idx: Vec<usize> = (0..n).filter(|&i| valid(i) && !values[i]).collect();
            idx.extend((0..n).filter(|&i| valid(i) && values[i]));
            idx.extend((0..n).filter(|&i| !valid(i)));
            idx
        }
        _ => match key.argsort()? {
            Series::I64 { values, .. } => values.iter().map(|&i| i as usize).collect(),
            _ => unreachable!("argsort gives Int indices"),
        },
    };
    let valid = |i: usize| key.validity().is_none_or(|v| v.get(i));
    let same = |a: usize, b: usize| match key {
        Series::F64 { values, .. } => values[a].total_cmp(&values[b]).is_eq(),
        Series::I64 { values, .. } => values[a] == values[b],
        Series::Bool { values, .. } => values[a] == values[b],
        Series::Str { col, .. } => col.get(a) == col.get(b),
        Series::Date { values, .. } => values[a] == values[b],
        Series::Datetime { values, .. } => values[a] == values[b],
        Series::Cat { codes, .. } => codes[a] == codes[b],
    };
    let mut ranks = vec![u32::MAX; n];
    let mut r = 0u32;
    let mut prev: Option<usize> = None;
    for &i in &order {
        if !valid(i) {
            break;
        }
        if let Some(p) = prev
            && !same(p, i)
        {
            r += 1;
        }
        ranks[i] = r;
        prev = Some(i);
    }
    if descending {
        for x in ranks.iter_mut() {
            if *x != u32::MAX {
                *x = r - *x;
            }
        }
    }
    Ok(ranks)
}

/// Row order sorting by several keys, each ascending or descending:
/// stable (ties keep their row order), nulls last in every key.
pub fn sort_order(keys: &[(&Series, bool)]) -> Result<Vec<usize>> {
    let Some((first, _)) = keys.first() else {
        return Err(OdsError::InvalidArgument(
            "sort_by needs at least one key".to_string(),
        ));
    };
    let n = first.len();
    let ranks: Vec<Vec<u32>> = keys
        .iter()
        .map(|(k, d)| {
            if k.len() != n {
                return Err(OdsError::LengthMismatch {
                    left: n,
                    right: k.len(),
                });
            }
            key_ranks(k, *d)
        })
        .collect::<Result<_>>()?;
    // The keys' ranks folded into one dense rank, a key at a time: a
    // sort of packed (rank so far, next rank) pairs per key. Then one
    // sort of (rank, row) pairs, the row breaking ties (stable).
    let mut combined: Vec<u32> = ranks[0].clone();
    for r in &ranks[1..] {
        let mut pairs: Vec<(u64, u32)> = (0..n)
            .map(|i| (((combined[i] as u64) << 32) | r[i] as u64, i as u32))
            .collect();
        sort_pairs(&mut pairs, n);
        let mut next = 0u32;
        for k in 0..pairs.len() {
            if k > 0 && pairs[k].0 != pairs[k - 1].0 {
                next += 1;
            }
            combined[pairs[k].1 as usize] = next;
        }
    }
    let mut pairs: Vec<(u64, u32)> = (0..n).map(|i| (combined[i] as u64, i as u32)).collect();
    sort_pairs(&mut pairs, n);
    Ok(pairs.into_iter().map(|(_, i)| i as usize).collect())
}

fn sort_pairs(pairs: &mut [(u64, u32)], n: usize) {
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        if crate::par_worth(crate::ParClass::Sort, n) {
            pairs.par_sort_unstable();
            return;
        }
    }
    let _ = n;
    pairs.sort_unstable();
}

impl Frame {
    /// Rows sorted by several key columns (`(name, descending)`), stable,
    /// nulls last in each.
    pub fn sort_by_keys(&self, keys: &[(&Series, bool)]) -> Result<Frame> {
        let order = sort_order(keys)?;
        if keys.first().is_some_and(|(k, _)| k.len() != self.n_rows()) {
            return Err(OdsError::LengthMismatch {
                left: self.n_rows(),
                right: keys[0].0.len(),
            });
        }
        let idx = Series::from_i64(order.into_iter().map(|i| i as i64).collect());
        self.take(&idx)
    }
}

/// A validity moved `by` positions down (up when negative), the
/// vacated positions null: word by word, no per-element pass.
fn shifted_validity(v: Option<&Bitmap>, n: usize, by: i64) -> Option<Bitmap> {
    if by == 0 {
        return v.cloned();
    }
    let nw = n.div_ceil(64);
    let src = |k: i64| -> u64 {
        if k < 0 || k as usize >= nw {
            return 0;
        }
        let k = k as usize;
        match v {
            Some(b) => b.words()[k],
            None => {
                if (k + 1) * 64 <= n {
                    u64::MAX
                } else {
                    (1u64 << (n - k * 64)) - 1
                }
            }
        }
    };
    let words: Vec<u64> = (0..nw)
        .map(|w| {
            // Output bit 64w + j comes from source bit 64w + j - by.
            let s = 64 * w as i64 - by;
            let q = s.div_euclid(64);
            let r = s.rem_euclid(64) as u32;
            if r == 0 {
                src(q)
            } else {
                (src(q) >> r) | (src(q + 1) << (64 - r))
            }
        })
        .collect();
    let bm = Bitmap::from_words(n, words);
    (!bm.all_valid()).then_some(bm)
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    #[test]
    fn an_asof_join_takes_the_row_in_force() {
        let quotes = Frame::new(vec![
            ("t".into(), Series::from_i64(vec![1, 5, 3, 9])),
            ("sym".into(), Series::from_str_values(vec!["a".into(), "a".into(), "b".into(), "a".into()])),
            ("px".into(), Series::from_f64(vec![10.0, 50.0, 30.0, 90.0])),
        ])
        .unwrap();
        let trades = Frame::new(vec![
            ("t".into(), Series::from_i64(vec![0, 4, 5, 8, 4])),
            ("sym".into(), Series::from_str_values(vec!["a".into(), "a".into(), "a".into(), "a".into(), "b".into()])),
        ])
        .unwrap();
        let px = |d: AsOf, tol: Option<f64>| -> Vec<Scalar> {
            let j = trades.asof_join(&quotes, "t", &["sym".to_string()], d, tol, "_right").unwrap();
            let c = j.column("px").unwrap();
            (0..c.len()).map(|i| c.scalar_at(i)).collect()
        };
        use Scalar::{F64, Null};
        assert_eq!(px(AsOf::Backward, None), vec![Null, F64(10.0), F64(50.0), F64(50.0), F64(30.0)]);
        assert_eq!(px(AsOf::Forward, None), vec![F64(10.0), F64(50.0), F64(50.0), F64(90.0), Null]);
        assert_eq!(px(AsOf::Nearest, None), vec![F64(10.0), F64(50.0), F64(50.0), F64(90.0), F64(30.0)]);
        assert_eq!(px(AsOf::Backward, Some(1.0)), vec![Null, Null, F64(50.0), Null, F64(30.0)]);
    }

    #[test]
    fn a_columns_summary_counts_ranges_and_ranks() {
        let n = Series::from_f64_options(vec![Some(1.0), None, Some(3.0), Some(1.0)]);
        let s = n.summarize(10, 4, 6);
        assert_eq!((s.nulls, s.distinct, s.numeric), (1, 2, true));
        assert_eq!(s.range, Some((1.0, 3.0, 5.0 / 3.0)));
        assert_eq!(s.hist, vec![2, 0, 0, 1]);
        let t = Series::from_str_options(vec![
            Some("b".into()),
            Some("a".into()),
            None,
            Some("a".into()),
            Some("b".into()),
            Some("c".into()),
        ]);
        let s = t.summarize(10, 4, 2);
        assert_eq!((s.nulls, s.distinct), (1, 3));
        // ties by first appearance: b (row 0) before a (row 1)
        assert_eq!(s.top, vec![(0, 2), (1, 2)]);
        // the first rows only
        let s = t.summarize(2, 4, 6);
        assert_eq!((s.rows, s.distinct, s.nulls), (2, 2, 0));
        let same = Series::from_i64(vec![4, 4]).summarize(10, 3, 6);
        assert_eq!(same.hist, vec![2, 0, 0]);
    }
}

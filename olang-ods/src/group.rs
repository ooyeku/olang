//! Groupings as values, and the verbs built on them: aggregation by
//! any `AggOp`, a group's rows, crosstabs, runs and keyed lookup.
//!
//! A [`Grouping`] is the dictionary encoding `group_by` runs (group ids
//! per row, first-seen order, in parallel on a large column), kept so
//! that every aggregation, transform and split over the same keys
//! reuses it. Each group's rows (a counting sort over the ids) are
//! built at most once, on the first aggregation that needs a group's
//! values together.

use crate::frame::{
    FxMap, Key, aggregate_by_rows, aggregate_column, first_row_per_group, group_ids, key_at,
    last_row_per_group, rows_by_group, scalar_name,
};
use crate::{AggOp, DType, Frame, OdsError, Scalar, Series};
use std::sync::OnceLock;

type Result<T> = std::result::Result<T, OdsError>;

/// Below this many rows a split, or a per-group pass, stays on one core.
#[cfg(feature = "parallel")]
const PAR_GROUP_ROWS: usize = 200_000;

/// Group ids over some key columns, and what follows from them.
#[derive(Debug)]
pub struct Grouping {
    ids: Vec<u32>,
    n_groups: usize,
    rows: OnceLock<(Vec<usize>, Vec<u32>)>,
    first: OnceLock<Vec<i64>>,
}

impl Grouping {
    /// The grouping of `keys` (one or more equal-length columns). A null
    /// is a key value of its own, as in `group_by`.
    pub fn new(keys: &[&Series]) -> Result<Grouping> {
        let (ids, n_groups) = group_ids(keys)?;
        Ok(Grouping::from_ids(ids, n_groups))
    }

    /// A grouping from ids already computed (each below `n_groups`).
    pub fn from_ids(ids: Vec<u32>, n_groups: usize) -> Grouping {
        Grouping {
            ids,
            n_groups,
            rows: OnceLock::new(),
            first: OnceLock::new(),
        }
    }

    pub fn n_groups(&self) -> usize {
        self.n_groups
    }

    /// Rows grouped.
    pub fn n_rows(&self) -> usize {
        self.ids.len()
    }

    /// Each row's group id.
    pub fn ids(&self) -> &[u32] {
        &self.ids
    }

    /// The first row of each group.
    pub fn first_rows(&self) -> &[i64] {
        self.first
            .get_or_init(|| first_row_per_group(&self.ids, self.n_groups))
    }

    fn rows(&self) -> &(Vec<usize>, Vec<u32>) {
        self.rows
            .get_or_init(|| rows_by_group(&self.ids, self.n_groups))
    }

    /// Group `g`'s rows, in row order.
    pub fn group_rows(&self, g: usize) -> &[u32] {
        let (offsets, rows) = self.rows();
        &rows[offsets[g]..offsets[g + 1]]
    }

    /// Each group's value of a key column: its first row's.
    pub fn keys_of(&self, col: &Series) -> Result<Series> {
        col.take(&Series::from_i64(self.first_rows().to_vec()))
    }

    /// Rows per group (nulls included). Int.
    pub fn size(&self) -> Series {
        let mut counts = vec![0i64; self.n_groups];
        for &g in &self.ids {
            counts[g as usize] += 1;
        }
        Series::from_i64(counts)
    }

    /// `op` of `col` within each group: one value per group, in group
    /// order. `Count` counts valid values (`size` counts rows).
    pub fn aggregate(&self, col: &Series, op: AggOp) -> Result<Series> {
        if col.len() != self.ids.len() {
            return Err(OdsError::LengthMismatch {
                left: self.ids.len(),
                right: col.len(),
            });
        }
        let (ids, n) = (&self.ids, self.n_groups);
        Ok(match op {
            AggOp::Count => match col.validity() {
                None => self.size(),
                Some(v) => {
                    let mut counts = vec![0i64; n];
                    for (i, &g) in ids.iter().enumerate() {
                        if v.get(i) {
                            counts[g as usize] += 1;
                        }
                    }
                    Series::from_i64(counts)
                }
            },
            AggOp::Sum | AggOp::Mean if col.dtype() == DType::Bool => {
                // A mask's sum counts its trues, its mean is their share.
                aggregate_column(&bool_as_int(col), ids, n, op)?
            }
            AggOp::Sum | AggOp::Mean | AggOp::Min | AggOp::Max => {
                aggregate_column(col, ids, n, op)?
            }
            AggOp::Median | AggOp::Std | AggOp::Var | AggOp::NUnique | AggOp::Quantile(_) => {
                if let Some(q) = op.q()
                    && !(0.0..=1.0).contains(&q)
                {
                    return Err(OdsError::InvalidArgument(format!(
                        "quantile expects q in [0, 1], got {}",
                        q
                    )));
                }
                aggregate_by_rows(col, ids, self.rows(), op)?
            }
            AggOp::First => self.keys_of(col)?,
            AggOp::Last => col.take(&Series::from_i64(last_row_per_group(ids, n)))?,
            AggOp::Any | AggOp::All => any_all(col, ids, n, op == AggOp::All)?,
            AggOp::ArgMax | AggOp::ArgMin => arg_extreme(col, ids, n, op == AggOp::ArgMax)?,
        })
    }

    /// A per-group column given back to every row (pandas' `transform`).
    pub fn broadcast(&self, per_group: &Series) -> Result<Series> {
        if per_group.len() != self.n_groups {
            return Err(OdsError::LengthMismatch {
                left: self.n_groups,
                right: per_group.len(),
            });
        }
        let idx: Vec<usize> = self.ids.iter().map(|&g| g as usize).collect();
        Ok(per_group.gather(&idx))
    }

    /// The Frame cut into one Frame per group, in group order (each
    /// keeps its rows' order).
    pub fn split(&self, f: &Frame) -> Result<Vec<Frame>> {
        if f.n_rows() != self.ids.len() {
            return Err(OdsError::LengthMismatch {
                left: self.ids.len(),
                right: f.n_rows(),
            });
        }
        let one = |g: usize| -> Result<Frame> {
            let idx: Vec<i64> = self.group_rows(g).iter().map(|&r| r as i64).collect();
            f.take(&Series::from_i64(idx))
        };
        // The rows are built once, before any thread asks for them.
        let _ = self.rows();
        #[cfg(feature = "parallel")]
        if f.n_rows() >= PAR_GROUP_ROWS && self.n_groups > 1 && crate::parallel_enabled() {
            use rayon::prelude::*;
            return (0..self.n_groups).into_par_iter().map(one).collect();
        }
        (0..self.n_groups).map(one).collect()
    }
}

/// `col` gathered at `idx`, a `None` giving a null (a lookup's miss).
pub fn take_opt(col: &Series, idx: &[Option<usize>]) -> Series {
    crate::frame::gather_optional(col, idx)
}

/// A Bool column as 0/1 Ints, nulls kept.
fn bool_as_int(col: &Series) -> Series {
    match col {
        Series::Bool { values, validity } => Series::I64 {
            values: std::sync::Arc::new(values.iter().map(|&b| b as i64).collect()),
            validity: validity.clone(),
        },
        other => other.clone(),
    }
}

fn any_all(col: &Series, ids: &[u32], n: usize, all: bool) -> Result<Series> {
    let truth: Box<dyn Fn(usize) -> Option<bool>> = match col {
        Series::Bool { values, validity } => Box::new(move |i| {
            validity
                .as_ref()
                .is_none_or(|v| v.get(i))
                .then(|| values[i])
        }),
        Series::I64 { values, validity } => Box::new(move |i| {
            validity
                .as_ref()
                .is_none_or(|v| v.get(i))
                .then(|| values[i] != 0)
        }),
        Series::F64 { values, validity } => Box::new(move |i| {
            validity
                .as_ref()
                .is_none_or(|v| v.get(i))
                .then(|| values[i] != 0.0)
        }),
        _ => {
            return Err(OdsError::TypeMismatch(format!(
                "{} needs a Bool (or numeric) column, got {}",
                if all { "all" } else { "any" },
                col.dtype()
            )));
        }
    };
    // `any` starts false and finds a true; `all` starts true and finds a
    // false. Nulls are skipped, so an all-null group is any false and
    // all true, as `ods.any`/`ods.all` answer an empty column.
    let mut out = vec![all; n];
    for (i, &g) in ids.iter().enumerate() {
        if let Some(t) = truth(i)
            && t != all
        {
            out[g as usize] = !all;
        }
    }
    Ok(Series::from_bool(out))
}

fn arg_extreme(col: &Series, ids: &[u32], n: usize, want_max: bool) -> Result<Series> {
    let mut best: Vec<Option<usize>> = vec![None; n];
    let better = |x: f64, y: f64| if want_max { x > y } else { x < y };
    match col {
        Series::F64 { values, validity } => {
            for (i, &g) in ids.iter().enumerate() {
                let x = values[i];
                if x.is_nan() || !validity.as_ref().is_none_or(|v| v.get(i)) {
                    continue;
                }
                let b = &mut best[g as usize];
                if b.is_none_or(|j| better(x, values[j])) {
                    *b = Some(i);
                }
            }
        }
        Series::I64 { values, validity } => {
            for (i, &g) in ids.iter().enumerate() {
                if !validity.as_ref().is_none_or(|v| v.get(i)) {
                    continue;
                }
                let x = values[i];
                let b = &mut best[g as usize];
                if b.is_none_or(|j| {
                    if want_max {
                        x > values[j]
                    } else {
                        x < values[j]
                    }
                }) {
                    *b = Some(i);
                }
            }
        }
        _ => {
            return Err(OdsError::TypeMismatch(format!(
                "{} needs a numeric column, got {}",
                if want_max { "argmax" } else { "argmin" },
                col.dtype()
            )));
        }
    }
    Ok(Series::from_i64_options(
        best.into_iter().map(|b| b.map(|i| i as i64)).collect(),
    ))
}

// ---------------------------------------------------------------------
// Crosstab
// ---------------------------------------------------------------------

/// How a crosstab's cells are made shares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalize {
    /// Counts (or the aggregate) as they are.
    None,
    /// Each cell over the table's total.
    All,
    /// Each cell over its row's total.
    Rows,
    /// Each cell over its column's total.
    Columns,
}

/// The distinct values of `col` (nulls left out), in ascending order,
/// and each row's position among them (`u32::MAX` for a null).
fn sorted_levels(col: &Series) -> Result<(Series, Vec<u32>)> {
    let g = Grouping::new(&[col])?;
    let firsts = g.first_rows();
    let levels = col.take(&Series::from_i64(firsts.to_vec()))?;
    // Ascending order of the levels; a null level sorts last and is
    // dropped.
    let order: Vec<usize> = match levels.argsort()? {
        Series::I64 { values, .. } => values.iter().map(|&i| i as usize).collect(),
        _ => unreachable!("argsort gives Int positions"),
    };
    let null_group = (0..levels.len()).find(|&k| matches!(levels.scalar_at(k), Scalar::Null));
    let mut rank = vec![u32::MAX; g.n_groups()];
    let mut kept: Vec<i64> = Vec::new();
    for &k in &order {
        if Some(k) == null_group {
            continue;
        }
        rank[k] = kept.len() as u32;
        kept.push(firsts[k]);
    }
    let pos: Vec<u32> = g.ids().iter().map(|&k| rank[k as usize]).collect();
    Ok((col.take(&Series::from_i64(kept))?, pos))
}

/// A two-way table of `a` against `b`: one row per value of `a`, one
/// column per value of `b` (both ascending, nulls left out), each cell
/// the rows that share both (`values` given: `op` of those values).
/// A combination that never occurs counts 0 (an aggregate: null).
/// `row_name` names the first column, the `a` values.
pub fn crosstab(
    a: &Series,
    b: &Series,
    values: Option<&Series>,
    op: AggOp,
    normalize: Normalize,
    row_name: &str,
    fmt_f64: &dyn Fn(f64) -> String,
) -> Result<Frame> {
    if a.len() != b.len() {
        return Err(OdsError::LengthMismatch {
            left: a.len(),
            right: b.len(),
        });
    }
    if let Some(v) = values
        && v.len() != a.len()
    {
        return Err(OdsError::LengthMismatch {
            left: a.len(),
            right: v.len(),
        });
    }
    let counting = values.is_none() || op == AggOp::Count;
    if normalize != Normalize::None && !(counting || op == AggOp::Sum) {
        return Err(OdsError::InvalidArgument(format!(
            "crosstab: normalize makes shares of counts or sums, not of {}",
            op.name()
        )));
    }
    let (rows, ra) = sorted_levels(a)?;
    let (cols, rb) = sorted_levels(b)?;
    let (nr, nc) = (rows.len(), cols.len());
    let cells = nr
        .checked_mul(nc)
        .filter(|&c| c < u32::MAX as usize)
        .ok_or_else(|| OdsError::InvalidArgument("crosstab: too many cells".to_string()))?;
    // Each row's cell; a row with a null on either side goes to a spare
    // cell past the table, which is dropped.
    let ids: Vec<u32> = ra
        .iter()
        .zip(&rb)
        .map(|(&i, &j)| {
            if i == u32::MAX || j == u32::MAX {
                cells as u32
            } else {
                i * nc as u32 + j
            }
        })
        .collect();
    let g = Grouping::from_ids(ids, cells + 1);
    let cell: Series = match values {
        None => g.size(),
        Some(v) => g.aggregate(v, op)?,
    };
    let cell_f64 = |k: usize| -> Option<f64> {
        match cell.scalar_at(k) {
            Scalar::F64(x) => Some(x),
            Scalar::I64(x) => Some(x as f64),
            _ => None,
        }
    };
    let mut pairs: Vec<(String, Series)> = Vec::with_capacity(nc + 1);
    pairs.push((row_name.to_string(), rows));
    let row_total: Vec<f64> = (0..nr)
        .map(|i| (0..nc).filter_map(|j| cell_f64(i * nc + j)).sum())
        .collect();
    let col_total: Vec<f64> = (0..nc)
        .map(|j| (0..nr).filter_map(|i| cell_f64(i * nc + j)).sum())
        .collect();
    let total: f64 = row_total.iter().sum();
    for (j, &col_tot) in col_total.iter().enumerate() {
        let name = scalar_name(cols.scalar_at(j), fmt_f64);
        if name == row_name {
            return Err(OdsError::InvalidArgument(format!(
                "crosstab: the value '{}' would name a column that is already the row column",
                name
            )));
        }
        let idx: Vec<usize> = (0..nr).map(|i| i * nc + j).collect();
        let column = match normalize {
            Normalize::None => {
                let c = cell.gather(&idx);
                // A cell with no rows counts zero.
                if counting {
                    c.fill_null(Scalar::I64(0))?
                } else {
                    c
                }
            }
            _ => Series::from_f64(
                (0..nr)
                    .map(|i| {
                        let x = cell_f64(i * nc + j).unwrap_or(0.0);
                        let d = match normalize {
                            Normalize::All => total,
                            Normalize::Rows => row_total[i],
                            _ => col_tot,
                        };
                        if d == 0.0 { 0.0 } else { x / d }
                    })
                    .collect(),
            ),
        };
        pairs.push((name, column));
    }
    Frame::new(pairs)
}

// ---------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------

/// Whether rows `i` and `i - 1` hold the same value (two nulls are the
/// same; a null and a value are not).
fn same_as_previous(s: &Series, i: usize) -> bool {
    let valid = |k: usize| s.validity().is_none_or(|v| v.get(k));
    match (valid(i), valid(i - 1)) {
        (false, false) => true,
        (true, true) => match s {
            Series::F64 { values, .. } => values[i].to_bits() == values[i - 1].to_bits(),
            Series::I64 { values, .. } => values[i] == values[i - 1],
            Series::Bool { values, .. } => values[i] == values[i - 1],
            Series::Str { col, .. } => col.get(i) == col.get(i - 1),
            Series::Date { values, .. } => values[i] == values[i - 1],
            Series::Datetime { values, .. } => values[i] == values[i - 1],
            Series::Cat { codes, .. } => codes[i] == codes[i - 1],
        },
        _ => false,
    }
}

/// A run number per row: 0 for the first row, one more wherever any of
/// `cols` changes from the row before (pandas' `(s != s.shift()).cumsum()`,
/// Polars' `rle_id`). Int.
pub fn run_id(cols: &[&Series]) -> Result<Series> {
    let Some(first) = cols.first() else {
        return Err(OdsError::InvalidArgument(
            "run_id needs at least one column".to_string(),
        ));
    };
    let n = first.len();
    if let Some(bad) = cols.iter().find(|c| c.len() != n) {
        return Err(OdsError::LengthMismatch {
            left: n,
            right: bad.len(),
        });
    }
    let mut out = Vec::with_capacity(n);
    let mut id = 0i64;
    for i in 0..n {
        if i > 0 && !cols.iter().all(|c| same_as_previous(c, i)) {
            id += 1;
        }
        out.push(id);
    }
    Ok(Series::from_i64(out))
}

/// The runs of a column: for a Bool mask, each run of trues (a null
/// ends a run); otherwise every run of equal values, with its `value`.
/// Columns `start` and `end` (the run's first and last row) and
/// `length`.
pub fn runs(s: &Series) -> Result<Frame> {
    let n = s.len();
    let (mut start, mut end, mut len) = (Vec::new(), Vec::new(), Vec::new());
    if let Series::Bool { values, validity } = s {
        let on = |i: usize| values[i] && validity.as_ref().is_none_or(|v| v.get(i));
        let mut i = 0;
        while i < n {
            if on(i) {
                let s0 = i;
                while i < n && on(i) {
                    i += 1;
                }
                start.push(s0 as i64);
                end.push(i as i64 - 1);
                len.push((i - s0) as i64);
            } else {
                i += 1;
            }
        }
        return Frame::new(vec![
            ("start".to_string(), Series::from_i64(start)),
            ("end".to_string(), Series::from_i64(end)),
            ("length".to_string(), Series::from_i64(len)),
        ]);
    }
    let mut firsts = Vec::new();
    let mut i = 0;
    while i < n {
        let s0 = i;
        i += 1;
        while i < n && same_as_previous(s, i) {
            i += 1;
        }
        firsts.push(s0 as i64);
        start.push(s0 as i64);
        end.push(i as i64 - 1);
        len.push((i - s0) as i64);
    }
    Frame::new(vec![
        ("value".to_string(), s.take(&Series::from_i64(firsts))?),
        ("start".to_string(), Series::from_i64(start)),
        ("end".to_string(), Series::from_i64(end)),
        ("length".to_string(), Series::from_i64(len)),
    ])
}

// ---------------------------------------------------------------------
// Keyed lookup
// ---------------------------------------------------------------------

/// A hash index over one column: value → its first row. Built once and
/// asked many times (`ods.lookup`'s cache).
#[derive(Debug)]
pub struct KeyIndex {
    map: FxMap<Key, usize>,
    dtype: DType,
}

impl KeyIndex {
    pub fn new(col: &Series) -> KeyIndex {
        let mut map: FxMap<Key, usize> = FxMap::default();
        map.reserve(col.len());
        for i in 0..col.len() {
            if let Some(k) = key_at(col, i) {
                map.entry(k).or_insert(i);
            }
        }
        KeyIndex {
            map,
            dtype: col.dtype(),
        }
    }

    /// The first row holding `key`. A number finds an equal number of
    /// the column's own type (`3` finds `3.0` in a Float column).
    pub fn get(&self, key: &Scalar) -> Option<usize> {
        let k = match (key, self.dtype) {
            (Scalar::Null, _) => return None,
            (Scalar::I64(x), DType::F64) => Key::F((*x as f64).to_bits()),
            (Scalar::F64(x), DType::I64) if x.fract() == 0.0 && x.abs() < 9.0e15 => {
                Key::I(*x as i64)
            }
            (Scalar::I64(x), _) => Key::I(*x),
            (Scalar::F64(x), _) => Key::F(if x.is_nan() {
                f64::NAN.to_bits()
            } else {
                x.to_bits()
            }),
            (Scalar::Bool(b), _) => Key::B(*b),
            (Scalar::Str(s), DType::Date) => match crate::temporal::parse_iso(s.trim()) {
                Some((us, _)) => Key::D(crate::temporal::day_of_us(us) as i32),
                None => return None,
            },
            (Scalar::Str(s), DType::Datetime) => match crate::temporal::parse_iso(s.trim()) {
                Some((us, _)) => Key::T(us),
                None => return None,
            },
            (Scalar::Str(s), _) => Key::S(s.clone()),
            (Scalar::Date(d), _) => Key::D(*d),
            (Scalar::Datetime(t), _) => Key::T(*t),
        };
        self.map.get(&k).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ints(s: &Series) -> Vec<Option<i64>> {
        (0..s.len())
            .map(|i| match s.scalar_at(i) {
                Scalar::I64(x) => Some(x),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_grouping_aggregates_and_broadcasts() {
        let k = Series::from_str_values(vec!["a".into(), "b".into(), "a".into(), "b".into()]);
        let x = Series::from_f64(vec![1.0, 5.0, 3.0, 2.0]);
        let g = Grouping::new(&[&k]).unwrap();
        assert_eq!(g.n_groups(), 2);
        assert_eq!(
            ints(&g.aggregate(&x, AggOp::ArgMax).unwrap()),
            vec![Some(2), Some(1)]
        );
        let q = g.aggregate(&x, AggOp::quantile(0.5)).unwrap();
        assert_eq!(q.scalar_at(0), Scalar::F64(2.0));
        let m = g.aggregate(&x, AggOp::Sum).unwrap();
        let b = g.broadcast(&m).unwrap();
        assert_eq!(b.scalar_at(3), Scalar::F64(7.0));
        let mask = Series::from_bool(vec![true, false, false, false]);
        assert_eq!(
            ints(&g.aggregate(&mask, AggOp::Sum).unwrap()),
            vec![Some(1), Some(0)]
        );
        let parts = g
            .split(&Frame::new(vec![("x".into(), x)]).unwrap())
            .unwrap();
        assert_eq!(parts[1].n_rows(), 2);
    }

    #[test]
    fn runs_and_run_ids() {
        let m = Series::from_bool(vec![true, true, false, true, false, false, true]);
        let r = runs(&m).unwrap();
        assert_eq!(
            ints(r.column("start").unwrap()),
            vec![Some(0), Some(3), Some(6)]
        );
        assert_eq!(
            ints(r.column("end").unwrap()),
            vec![Some(1), Some(3), Some(6)]
        );
        assert_eq!(
            ints(&run_id(&[&m]).unwrap()),
            vec![
                Some(0),
                Some(0),
                Some(1),
                Some(2),
                Some(3),
                Some(3),
                Some(4)
            ]
        );
    }

    #[test]
    fn crosstab_counts_with_zeros() {
        let a = Series::from_i64(vec![1, 0, 1, 1]);
        let b = Series::from_str_values(vec!["y".into(), "x".into(), "x".into(), "y".into()]);
        let t = crosstab(&a, &b, None, AggOp::Count, Normalize::None, "a", &|x| {
            x.to_string()
        })
        .unwrap();
        assert_eq!(t.names(), &["a", "x", "y"]);
        assert_eq!(ints(t.column("y").unwrap()), vec![Some(0), Some(2)]);
    }
}

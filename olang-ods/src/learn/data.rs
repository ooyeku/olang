//! From a Frame to numbers: the design matrix, its encoding, and class
//! labels.
//!
//! A model remembers how it encoded its inputs ([`Design`]), so a
//! prediction encodes new rows the same way: a String column becomes one
//! 0/1 column per *training* level (the first dropped for models with an
//! intercept), a Bool becomes 0/1, an Int widens. A level the training
//! data never had encodes as all zeros — the baseline — rather than an
//! error, so scoring a frame with a new category still works.
//!
//! A null (or a NaN) in any used column makes the row invalid: fits drop
//! it (and say how many they dropped), predictions answer null there.

use super::formula::{Formula, Response, TermCode};
use crate::{Frame, OdsError, Series};
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, OdsError>;

pub fn bad(msg: impl Into<String>) -> OdsError {
    OdsError::InvalidArgument(msg.into())
}

/// A column-major dense matrix: `cols[j][i]` is row `i` of feature `j`.
#[derive(Clone, Debug, Default)]
pub struct Matrix {
    pub n: usize,
    pub cols: Vec<Vec<f64>>,
}

impl Matrix {
    pub fn new(n: usize, cols: Vec<Vec<f64>>) -> Matrix {
        debug_assert!(cols.iter().all(|c| c.len() == n));
        Matrix { n, cols }
    }

    pub fn k(&self) -> usize {
        self.cols.len()
    }

    /// The rows at `idx`, in that order.
    pub fn take(&self, idx: &[usize]) -> Matrix {
        Matrix {
            n: idx.len(),
            cols: self
                .cols
                .iter()
                .map(|c| idx.iter().map(|&i| c[i]).collect())
                .collect(),
        }
    }

    /// Row-major copy (`n × k`), what distance kernels stream over.
    pub fn row_major(&self) -> Vec<f64> {
        let k = self.k();
        let mut out = vec![0.0; self.n * k];
        for (j, c) in self.cols.iter().enumerate() {
            for (i, v) in c.iter().enumerate() {
                out[i * k + j] = *v;
            }
        }
        out
    }

    pub fn row_into(&self, i: usize, out: &mut [f64]) {
        for (j, c) in self.cols.iter().enumerate() {
            out[j] = c[i];
        }
    }
}

/// How one input column becomes features.
#[derive(Clone, Debug)]
pub enum Input {
    /// Float or Int, used as is.
    Numeric(String),
    /// Bool as 0/1.
    Flag(String),
    /// A String column, one 0/1 feature per level (minus the first when
    /// `drop_first`).
    Categorical {
        name: String,
        levels: Vec<String>,
        drop_first: bool,
    },
    /// A formula's term: its own columns, coded patsy's way.
    Term(Arc<TermCode>),
}

impl Input {
    pub fn name(&self) -> &str {
        match self {
            Input::Numeric(n) | Input::Flag(n) => n,
            Input::Categorical { name, .. } => name,
            Input::Term(t) => &t.label,
        }
    }

    /// The Frame columns it reads.
    pub fn sources(&self) -> Vec<String> {
        match self {
            Input::Term(t) => t.sources(),
            other => vec![other.name().to_string()],
        }
    }
}

/// The encoding a model learned from its training frame.
#[derive(Clone, Debug)]
pub struct Design {
    pub inputs: Vec<Input>,
    /// One name per feature column: the column's own, or `col_level`.
    pub features: Vec<String>,
    /// A formula's response, when the model was fit from one: how to
    /// read (or compute) the target from a new Frame.
    pub response: Option<Response>,
}

/// The sorted distinct non-null strings of a String column.
pub fn levels_of(s: &Series) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    if let Series::Str { col, .. } = s {
        for i in 0..s.len() {
            if is_valid(s, i) {
                set.insert(col.get(i));
            }
        }
    }
    set.into_iter().map(|x| x.to_string()).collect()
}

#[inline]
pub fn is_valid(s: &Series, i: usize) -> bool {
    s.validity().map(|v| v.get(i)).unwrap_or(true)
}

/// A numeric (or Bool) column as f64 with a validity vector (a NaN is
/// invalid too). Strings are refused.
pub fn numeric(s: &Series, what: &str) -> Result<(Vec<f64>, Vec<bool>)> {
    let n = s.len();
    let (vals, mut valid): (Vec<f64>, Vec<bool>) = match s {
        Series::F64 { values, .. } => (values.as_ref().clone(), vec![true; n]),
        Series::I64 { values, .. } => (values.iter().map(|&v| v as f64).collect(), vec![true; n]),
        Series::Bool { values, .. } => (
            values.iter().map(|&b| if b { 1.0 } else { 0.0 }).collect(),
            vec![true; n],
        ),
        Series::Str { .. } | Series::Cat { .. } => {
            return Err(bad(format!(
                "{}: needs a numeric column, got {}",
                what,
                s.dtype()
            )));
        }
        Series::Date { .. } | Series::Datetime { .. } => {
            return Err(bad(format!(
                "{}: needs a numeric column, got a {} (use its parts: year(), month(), weekday(), …)",
                what,
                s.dtype()
            )));
        }
    };
    if let Some(bm) = s.validity() {
        for (i, v) in valid.iter_mut().enumerate() {
            *v = bm.get(i);
        }
    }
    for (i, v) in vals.iter().enumerate() {
        if v.is_nan() {
            valid[i] = false;
        }
    }
    Ok((vals, valid))
}

impl Design {
    /// Learn the encoding of `cols` in `frame`.
    pub fn learn(frame: &Frame, cols: &[String], drop_first: bool) -> Result<Design> {
        if cols.is_empty() {
            return Err(bad("learn: no input columns"));
        }
        let mut inputs = Vec::with_capacity(cols.len());
        let mut features = Vec::new();
        for name in cols {
            let s = frame.column(name)?;
            match s {
                Series::F64 { .. } | Series::I64 { .. } => {
                    features.push(name.clone());
                    inputs.push(Input::Numeric(name.clone()));
                }
                Series::Bool { .. } => {
                    features.push(name.clone());
                    inputs.push(Input::Flag(name.clone()));
                }
                Series::Date { .. } | Series::Datetime { .. } => {
                    return Err(bad(format!(
                        "learn: column '{}' is a {}; give the model its parts \
                         (year(), month(), weekday(), …) or days since a start",
                        name,
                        s.dtype()
                    )));
                }
                Series::Str { .. } | Series::Cat { .. } => {
                    // A Category's levels are its own, in its order (its
                    // first is the one dropped); text's are sorted.
                    let levels = match s {
                        Series::Cat { levels, .. } => levels.iter().map(str::to_string).collect(),
                        _ => levels_of(s),
                    };
                    if levels.is_empty() {
                        return Err(bad(format!("learn: column '{}' has no values", name)));
                    }
                    let skip = usize::from(drop_first);
                    for l in levels.iter().skip(skip) {
                        features.push(format!("{}_{}", name, l));
                    }
                    inputs.push(Input::Categorical {
                        name: name.clone(),
                        levels,
                        drop_first,
                    });
                }
            }
        }
        Ok(Design {
            inputs,
            features,
            response: None,
        })
    }

    /// The design a formula describes over `frame`.
    pub fn from_formula(frame: &Frame, f: &Formula) -> Result<Design> {
        let (terms, response) = super::formula::code(f, frame)?;
        let mut features = Vec::new();
        let mut inputs = Vec::with_capacity(terms.len());
        for t in terms {
            features.extend(t.features());
            inputs.push(Input::Term(Arc::new(t)));
        }
        Ok(Design {
            inputs,
            features,
            response: Some(response),
        })
    }

    pub fn k(&self) -> usize {
        self.features.len()
    }

    /// Encode `frame` the trained way. Returns the matrix (invalid rows
    /// hold zeros) and which rows are complete.
    pub fn encode(&self, frame: &Frame) -> Result<(Matrix, Vec<bool>)> {
        let n = frame.n_rows();
        let mut valid = vec![true; n];
        let mut cols: Vec<Vec<f64>> = Vec::with_capacity(self.k());
        for input in &self.inputs {
            if let Input::Term(t) = input {
                t.encode(frame, &mut cols, &mut valid, false)?;
                continue;
            }
            let s = frame.column(input.name())?;
            match input {
                Input::Term(_) => unreachable!(),
                Input::Numeric(name) | Input::Flag(name) => {
                    if matches!(s, Series::Str { .. } | Series::Cat { .. }) {
                        return Err(bad(format!(
                            "learn: column '{}' was numeric when the model was fit, now {}",
                            name,
                            s.dtype()
                        )));
                    }
                    let (vals, ok) = numeric(s, name)?;
                    for (v, o) in valid.iter_mut().zip(&ok) {
                        *v &= *o;
                    }
                    cols.push(vals);
                }
                Input::Categorical {
                    name,
                    levels,
                    drop_first,
                } => {
                    let text = s.key_view();
                    let Series::Str { col, .. } = text.as_ref() else {
                        return Err(bad(format!(
                            "learn: column '{}' was categorical when the model was fit, now {}",
                            name,
                            s.dtype()
                        )));
                    };
                    let index: std::collections::HashMap<&str, usize> = levels
                        .iter()
                        .enumerate()
                        .map(|(i, l)| (l.as_str(), i))
                        .collect();
                    let skip = usize::from(*drop_first);
                    let width = levels.len() - skip;
                    let mut block = vec![vec![0.0f64; n]; width];
                    for i in 0..n {
                        if !is_valid(s, i) {
                            valid[i] = false;
                            continue;
                        }
                        if let Some(&pos) = index.get(col.get(i))
                            && pos >= skip
                        {
                            block[pos - skip][i] = 1.0;
                        }
                    }
                    cols.extend(block);
                }
            }
        }
        Ok((Matrix::new(n, cols), valid))
    }
}

// ---------------------------------------------------------------------
// Class labels
// ---------------------------------------------------------------------

/// The distinct classes of a classification target, sorted the way
/// sklearn sorts them (false < true, ascending Ints, byte-order
/// Strings). Class `i` of a fit is `Labels[i]`.
#[derive(Clone, Debug, PartialEq)]
pub enum Labels {
    Bool(Vec<bool>),
    Int(Vec<i64>),
    Str(Vec<String>),
}

pub const NO_CLASS: usize = usize::MAX;

impl Labels {
    pub fn len(&self) -> usize {
        match self {
            Labels::Bool(v) => v.len(),
            Labels::Int(v) => v.len(),
            Labels::Str(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn name(&self, i: usize) -> String {
        match self {
            Labels::Bool(v) => v[i].to_string(),
            Labels::Int(v) => v[i].to_string(),
            Labels::Str(v) => v[i].clone(),
        }
    }

    pub fn names(&self) -> Vec<String> {
        (0..self.len()).map(|i| self.name(i)).collect()
    }

    /// A Series of labels for class indices (`NO_CLASS` is null).
    pub fn series(&self, idx: &[usize]) -> Series {
        match self {
            Labels::Bool(v) => Series::from_bool_options(
                idx.iter().map(|&i| (i != NO_CLASS).then(|| v[i])).collect(),
            ),
            Labels::Int(v) => Series::from_i64_options(
                idx.iter().map(|&i| (i != NO_CLASS).then(|| v[i])).collect(),
            ),
            Labels::Str(v) => Series::from_str_options(
                idx.iter()
                    .map(|&i| (i != NO_CLASS).then(|| v[i].clone()))
                    .collect(),
            ),
        }
    }

    /// Learn the classes of `s` and encode it. Nulls (and unusable
    /// Floats) encode as `NO_CLASS`.
    pub fn learn(s: &Series) -> Result<(Labels, Vec<usize>)> {
        let labels = match s {
            Series::Bool { values, .. } => {
                let mut seen = [false, false];
                for i in 0..s.len() {
                    if is_valid(s, i) {
                        seen[values[i] as usize] = true;
                    }
                }
                Labels::Bool(
                    [false, true]
                        .into_iter()
                        .filter(|b| seen[*b as usize])
                        .collect(),
                )
            }
            Series::I64 { values, .. } => {
                let set: std::collections::BTreeSet<i64> = (0..s.len())
                    .filter(|&i| is_valid(s, i))
                    .map(|i| values[i])
                    .collect();
                Labels::Int(set.into_iter().collect())
            }
            Series::F64 { values, .. } => {
                let mut set = std::collections::BTreeSet::new();
                for i in 0..s.len() {
                    if is_valid(s, i) && !values[i].is_nan() {
                        let v = values[i];
                        if v.fract() != 0.0 || v.abs() > 9.0e15 {
                            return Err(bad(
                                "learn: a classification target must be String, Bool or Int \
                                 (a Float column with fractions is a regression target)",
                            ));
                        }
                        set.insert(v as i64);
                    }
                }
                Labels::Int(set.into_iter().collect())
            }
            Series::Str { .. } => Labels::Str(levels_of(s)),
            Series::Cat { .. } => Labels::Str(levels_of(&s.key_view())),
            Series::Date { .. } | Series::Datetime { .. } => {
                return Err(bad(format!(
                    "learn: a classification target must be String, Bool or Int, got {}",
                    s.dtype()
                )));
            }
        };
        let codes = labels.encode(s);
        Ok((labels, codes))
    }

    /// Encode `s` by these classes; a value not among them (or a null)
    /// is `NO_CLASS`.
    pub fn encode(&self, s: &Series) -> Vec<usize> {
        if let Some(text) = s.decoded() {
            return self.encode(&text);
        }
        let n = s.len();
        let mut out = vec![NO_CLASS; n];
        for (i, o) in out.iter_mut().enumerate() {
            if !is_valid(s, i) {
                continue;
            }
            *o = match (self, s) {
                (Labels::Bool(ls), Series::Bool { values, .. }) => {
                    ls.iter().position(|b| *b == values[i]).unwrap_or(NO_CLASS)
                }
                (Labels::Int(ls), Series::I64 { values, .. }) => {
                    ls.binary_search(&values[i]).unwrap_or(NO_CLASS)
                }
                (Labels::Int(ls), Series::F64 { values, .. }) => {
                    let v = values[i];
                    if v.is_nan() || v.fract() != 0.0 {
                        NO_CLASS
                    } else {
                        ls.binary_search(&(v as i64)).unwrap_or(NO_CLASS)
                    }
                }
                (Labels::Str(ls), Series::Str { col, .. }) => ls
                    .binary_search_by(|l| l.as_str().cmp(col.get(i)))
                    .unwrap_or(NO_CLASS),
                _ => NO_CLASS,
            };
        }
        out
    }
}

/// Indices of the rows where every flag is true.
pub fn kept(valid: &[bool]) -> Vec<usize> {
    valid
        .iter()
        .enumerate()
        .filter_map(|(i, &v)| v.then_some(i))
        .collect()
}

/// The positive class of a binary target given as a Series: `pos` when
/// the caller named one, else the larger of the two sorted classes
/// (true, 1, the second string) — sklearn's convention.
pub fn binary_target(y: &Series, pos: Option<&crate::Scalar>) -> Result<Vec<Option<bool>>> {
    use crate::Scalar;
    let n = y.len();
    if let Some(p) = pos {
        return Ok((0..n)
            .map(|i| match y.scalar_at(i) {
                Scalar::Null => None,
                v => Some(scalar_eq(&v, p)),
            })
            .collect());
    }
    let (labels, codes) = Labels::learn(y)?;
    if labels.len() > 2 {
        return Err(bad(format!(
            "learn: a binary metric needs two classes, the target has {} ({})",
            labels.len(),
            labels.names().join(", ")
        )));
    }
    // One class only: true when it is the "positive-looking" one.
    let one_pos = labels.len() == 1
        && match &labels {
            Labels::Bool(v) => v[0],
            Labels::Int(v) => v[0] == 1,
            Labels::Str(_) => false,
        };
    Ok(codes
        .into_iter()
        .map(|c| {
            if c == NO_CLASS {
                None
            } else if labels.len() == 1 {
                Some(one_pos)
            } else {
                Some(c == 1)
            }
        })
        .collect())
}

fn scalar_eq(a: &crate::Scalar, b: &crate::Scalar) -> bool {
    use crate::Scalar::*;
    match (a, b) {
        (F64(x), I64(y)) | (I64(y), F64(x)) => *x == *y as f64,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_with_training_levels_and_flags_nulls() {
        let train = Frame::new(vec![
            (
                "c".to_string(),
                Series::from_str_values(vec!["b".into(), "a".into(), "c".into()]),
            ),
            ("x".to_string(), Series::from_f64(vec![1.0, 2.0, 3.0])),
        ])
        .unwrap();
        let d = Design::learn(&train, &["c".to_string(), "x".to_string()], true).unwrap();
        assert_eq!(d.features, vec!["c_b", "c_c", "x"]);
        let test = Frame::new(vec![
            (
                "c".to_string(),
                Series::from_str_options(vec![Some("c".into()), Some("zz".into()), None]),
            ),
            ("x".to_string(), Series::from_f64(vec![5.0, f64::NAN, 1.0])),
        ])
        .unwrap();
        let (m, valid) = d.encode(&test).unwrap();
        assert_eq!(m.cols[0], vec![0.0, 0.0, 0.0]);
        assert_eq!(m.cols[1], vec![1.0, 0.0, 0.0]);
        assert_eq!(valid, vec![true, false, false]);
    }

    #[test]
    fn class_labels_sort_like_sklearn() {
        let (l, codes) = Labels::learn(&Series::from_i64(vec![3, 1, 3, 2])).unwrap();
        assert_eq!(l, Labels::Int(vec![1, 2, 3]));
        assert_eq!(codes, vec![2, 0, 2, 1]);
    }
}

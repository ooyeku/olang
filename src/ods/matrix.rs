//! Matrices (docs/ods.md): the `Matrix` value,
//! `ods.matrix`, `ods.corr` and `ods.cov`, the `mat` namespace, and `@`.
//!
//! A Matrix is the engine's `olang_ods::matrix::Matrix`: column-major
//! Floats whose columns are shared `Arc`s, so `ods.matrix(f, cols)` over
//! null-free Float columns copies nothing, and a transpose is a flag.
//! Its methods are `mat`'s functions (`X.t()`, `X.gram()`, `X.mean()`),
//! resolved like a Series' (L2).

use super::frame::{OdsFrame, frame_of};
use super::series::{make_series_value, series_of};
use crate::ast::{BinaryOp, BuiltinFunction, UnaryOp, Value, ValueMap};
use crate::native::{NativeHandle, NativeObject};
use olang_ods::matrix::{self, Arith, CorrMethod, Matrix, Metric};
use olang_ods::{Frame, Series};
use std::any::Any;
use std::sync::Arc;

type R<T> = Result<T, String>;

/// The `ods` functions this file answers.
pub const ODS_FUNCTIONS: &[(&str, usize)] = &[("matrix", 3), ("corr", 3), ("cov", 2)];

/// The `mat` namespace.
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("of", 1),
    ("identity", 1),
    ("t", 1),
    ("gram", 1),
    ("matmul", 2),
    ("col", 2),
    ("row", 2),
    ("shape", 1),
    ("mean", 2),
    ("std", 3),
    ("solve", 2),
    ("inv", 1),
    ("cholesky", 1),
    ("eigh", 1),
    ("cdist", 3),
    ("to_frame", 2),
    ("to_list", 1),
    ("sum", 2),
];

/// The column functions a Matrix takes, as `ods.<name>(X)` and `X.<name>()`:
/// each applied to every column, as to a Series.
pub const COLUMN_FUNCTIONS: &[(&str, usize)] = &[
    ("abs", 1),
    ("exp", 1),
    ("exp2", 1),
    ("expm1", 1),
    ("ln", 1),
    ("ln1p", 1),
    ("log2", 1),
    ("log10", 1),
    ("sqrt", 1),
    ("cbrt", 1),
    ("sin", 1),
    ("cos", 1),
    ("tan", 1),
    ("asin", 1),
    ("acos", 1),
    ("atan", 1),
    ("sinh", 1),
    ("cosh", 1),
    ("tanh", 1),
    ("sigmoid", 1),
    ("logit", 1),
    ("sign", 1),
    ("floor", 1),
    ("ceil", 1),
    ("trunc", 1),
    ("pow", 2),
    ("round", 2),
    ("clip", 3),
];

// ── The value ───────────────────────────────────────────────────────
#[derive(Debug)]
pub struct OdsMatrix(pub Matrix);

impl OdsMatrix {
    pub fn into_value(m: Matrix) -> Value {
        Value::Native(NativeHandle::new(OdsMatrix(m)))
    }
}

/// Column names of a matrix shown as a table: `0`, `1`, … as pandas
/// labels a DataFrame of an array.
fn index_names(k: usize) -> Vec<String> {
    (0..k).map(|j| j.to_string()).collect()
}

fn as_frame(m: &Matrix, names: Option<Vec<String>>) -> R<Frame> {
    let names = names.unwrap_or_else(|| index_names(m.ncols()));
    let cols = m.columns();
    Frame::new(
        names
            .into_iter()
            .zip(cols)
            .map(|(n, c)| {
                (
                    n,
                    Series::F64 {
                        values: c,
                        validity: None,
                    },
                )
            })
            .collect(),
    )
    .map_err(|e| e.to_string())
}

impl NativeObject for OdsMatrix {
    fn module(&self) -> &'static str {
        "mat"
    }

    fn type_name(&self) -> &'static str {
        "Matrix"
    }

    /// A grid: the Frame table of its columns, under its shape, without
    /// the dtype row (every cell is a Float).
    fn display(&self) -> String {
        let (r, c) = self.0.shape();
        let shape = format!("Matrix[{} x {}]", r, c);
        if c == 0 {
            return format!("{} (no columns)", shape);
        }
        let Ok(f) = as_frame(&self.0, None) else {
            return shape;
        };
        let text = super::table::render(&f);
        let mut lines: Vec<&str> = text.lines().collect();
        lines[0] = &shape;
        if lines.len() > 3 {
            lines.remove(3);
        }
        lines.join("\n")
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<OdsMatrix>()
            .is_some_and(|o| o.0 == self.0)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `len(X)`: the row count.
    fn length(&self) -> Option<usize> {
        Some(self.0.rows())
    }
}

pub fn matrix_of(v: &Value) -> Option<&Matrix> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsMatrix>().map(|m| &m.0),
        _ => None,
    }
}

/// A Matrix as the Frame the notebook kernel shows: columns `0`, `1`, ….
pub fn matrix_frame(v: &Value) -> Option<Value> {
    let m = matrix_of(v)?;
    as_frame(m, None).ok().map(OdsFrame::into_value)
}

pub fn namespace() -> Value {
    let mut module = ValueMap::default();
    for (name, arity) in FUNCTIONS.iter().chain(COLUMN_FUNCTIONS) {
        module.insert(
            name.to_string(),
            Value::Builtin(BuiltinFunction {
                name: format!("mat.{}", name),
                arity: *arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

pub fn handles(func: &str) -> bool {
    FUNCTIONS
        .iter()
        .chain(COLUMN_FUNCTIONS)
        .any(|(n, _)| *n == func)
}

fn par() -> bool {
    crate::parallel::should_parallelize(usize::MAX) && olang_ods::parallel_enabled()
}

// ── Conversions ─────────────────────────────────────────────────────
/// A Float column with no nulls, shared when it is one already.
fn dense_column(s: &Series, name: &str, func: &str) -> R<Arc<Vec<f64>>> {
    let (vals, _) = matrix::float_values(s, name).map_err(|e| format!("{}: {}", func, e))?;
    let nulls = s.null_count();
    if nulls > 0 {
        return Err(format!(
            "{}: column '{}' has {} null{}; drop them first, or pass nulls: \"drop\" to leave out the rows that have any",
            func,
            name,
            nulls,
            if nulls == 1 { "" } else { "s" }
        ));
    }
    Ok(vals)
}

/// Named columns of a matrix: from a Frame (the named ones, or every
/// numeric one) or a list of Series.
fn named_columns<'a>(
    func: &str,
    data: &'a Value,
    cols: Option<&Value>,
) -> R<Vec<(String, &'a Series)>> {
    if let Some(f) = frame_of(data) {
        let names: Vec<String> = match cols {
            None | Some(Value::Unit) => f
                .names()
                .iter()
                .zip(f.columns())
                .filter(|(_, s)| {
                    matches!(
                        s,
                        Series::F64 { .. } | Series::I64 { .. } | Series::Bool { .. }
                    )
                })
                .map(|(n, _)| n.clone())
                .collect(),
            Some(Value::String(s)) => vec![s.to_string()],
            Some(Value::List(items)) => items
                .iter()
                .map(|v| match v {
                    Value::String(s) => Ok(s.to_string()),
                    other => Err(format!(
                        "{}: the columns are names (Strings), got {}",
                        func,
                        other.type_name()
                    )),
                })
                .collect::<R<_>>()?,
            Some(other) => {
                return Err(format!(
                    "{}: the columns are a name or a list of names, got {}",
                    func,
                    other.type_name()
                ));
            }
        };
        if names.is_empty() {
            return Err(format!("{}: the Frame has no numeric columns", func));
        }
        return names
            .into_iter()
            .map(|n| {
                let s = f.column(&n).map_err(|_| {
                    format!(
                        "{}: no column '{}' in this Frame. It has: {}",
                        func,
                        n,
                        f.names().join(", ")
                    )
                })?;
                Ok((n, s))
            })
            .collect();
    }
    if let Value::List(items) = data {
        if !matches!(cols, None | Some(Value::Unit)) {
            return Err(format!("{}: a list of Series takes no column names", func));
        }
        return items
            .iter()
            .enumerate()
            .map(|(j, v)| match series_of(v) {
                Some(s) => Ok((j.to_string(), s)),
                None => Err(format!(
                    "{}: a list of Series, but item {} is {}",
                    func,
                    j,
                    v.type_name()
                )),
            })
            .collect();
    }
    Err(format!(
        "{}: needs a Frame or a list of Series, got {}",
        func,
        data.type_name()
    ))
}

/// `ods.matrix(f, cols?, nulls: "raise" | "drop")`.
fn build(func: &str, data: &Value, cols: Option<&Value>, nulls: &str) -> R<Matrix> {
    let named = named_columns(func, data, cols)?;
    let n = named.first().map(|(_, s)| s.len()).unwrap_or(0);
    if let Some((name, s)) = named.iter().find(|(_, s)| s.len() != n) {
        return Err(format!(
            "{}: the columns differ in length ({} has {}, the first {})",
            func,
            name,
            s.len(),
            n
        ));
    }
    match nulls {
        "raise" => {
            let cols = named
                .iter()
                .map(|(name, s)| dense_column(s, name, func))
                .collect::<R<Vec<_>>>()?;
            Matrix::from_columns(n, cols).map_err(|e| e.to_string())
        }
        "drop" => {
            let parts = named
                .iter()
                .map(|(name, s)| {
                    matrix::float_column(s, name).map_err(|e| format!("{}: {}", func, e))
                })
                .collect::<R<Vec<_>>>()?;
            if parts.iter().all(|(_, m)| m.is_none()) {
                return Matrix::from_columns(n, parts.into_iter().map(|(v, _)| v).collect())
                    .map_err(|e| e.to_string());
            }
            let keep: Vec<usize> = (0..n)
                .filter(|&i| parts.iter().all(|(_, m)| m.as_ref().is_none_or(|m| m[i])))
                .collect();
            let cols = parts
                .iter()
                .map(|(v, _)| Arc::new(keep.iter().map(|&i| v[i]).collect::<Vec<f64>>()))
                .collect();
            Matrix::from_columns(keep.len(), cols).map_err(|e| e.to_string())
        }
        other => Err(format!(
            "{}: nulls is \"raise\" or \"drop\", got \"{}\"",
            func, other
        )),
    }
}

/// A Matrix argument: a Matrix, a Frame's numeric columns, a list of
/// Series (columns), a list of lists (rows), or a Series (one column).
fn want_matrix(func: &str, v: Option<&Value>, what: &str) -> R<Matrix> {
    let Some(v) = v else {
        return Err(format!("{}: missing {}", func, what));
    };
    if let Some(m) = matrix_of(v) {
        return Ok(m.clone());
    }
    if let Some(s) = series_of(v) {
        let c = dense_column(s, "the Series", func)?;
        return Matrix::from_columns(s.len(), vec![c]).map_err(|e| e.to_string());
    }
    if frame_of(v).is_some() {
        return build(func, v, None, "raise");
    }
    if let Value::List(items) = v {
        if items.iter().all(|i| series_of(i).is_some()) && !items.is_empty() {
            return build(func, v, None, "raise");
        }
        if items.iter().all(|i| matches!(i, Value::List(_))) {
            let rows = items
                .iter()
                .map(|r| floats(func, r))
                .collect::<R<Vec<_>>>()?;
            return Matrix::from_rows(&rows).map_err(|e| e.to_string());
        }
    }
    Err(format!(
        "{}: {} must be a Matrix (or a Frame, a list of Series, a list of rows), got {}",
        func,
        what,
        v.type_name()
    ))
}

/// A vector: a Series or a list of numbers.
fn floats(func: &str, v: &Value) -> R<Vec<f64>> {
    if let Some(s) = series_of(v) {
        return Ok(dense_column(s, "the Series", func)?.as_ref().clone());
    }
    match v {
        Value::List(items) => items
            .iter()
            .map(|x| match x {
                Value::Float(f) => Ok(*f),
                Value::Integer(i) => Ok(*i as f64),
                other => Err(format!(
                    "{}: a vector holds numbers, got {}",
                    func,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{}: needs a Series or a list of numbers, got {}",
            func,
            other.type_name()
        )),
    }
}

fn is_vector(v: &Value) -> bool {
    series_of(v).is_some()
        || matches!(v, Value::List(items) if items.iter().all(|x| matches!(x, Value::Float(_) | Value::Integer(_))))
}

fn float_series(v: Vec<f64>) -> Value {
    make_series_value(Series::from_f64(v))
}

fn int_arg(func: &str, v: Option<&Value>, what: &str) -> R<i64> {
    match v {
        Some(Value::Integer(i)) => Ok(*i),
        Some(other) => Err(format!(
            "{}: {} must be an Int, got {}",
            func,
            what,
            other.type_name()
        )),
        None => Err(format!("{}: missing {}", func, what)),
    }
}

fn index_arg(func: &str, v: Option<&Value>, len: usize, what: &str) -> R<usize> {
    let i = int_arg(func, v, what)?;
    let j = if i < 0 { i + len as i64 } else { i };
    if j < 0 || j as usize >= len {
        return Err(format!(
            "{}: {} {} is out of range (there are {})",
            func, what, i, len
        ));
    }
    Ok(j as usize)
}

// ── `@` ─────────────────────────────────────────────────────────────
/// `a @ b` with a Matrix on either side.
pub fn matmul(lhs: &Value, rhs: &Value) -> Option<R<Value>> {
    let (a, b) = (matrix_of(lhs), matrix_of(rhs));
    if a.is_none() && b.is_none() {
        return None;
    }
    Some(matmul_values("`@`", lhs, rhs))
}

fn matmul_values(func: &str, lhs: &Value, rhs: &Value) -> R<Value> {
    match (matrix_of(lhs), matrix_of(rhs)) {
        (Some(a), Some(b)) => a
            .matmul(b, par())
            .map(OdsMatrix::into_value)
            .map_err(|e| e.to_string()),
        (Some(a), None) if is_vector(rhs) => {
            let v = match series_of(rhs) {
                Some(s) => dense_column(s, "the Series", func)?,
                None => Arc::new(floats(func, rhs)?),
            };
            a.matvec_shared(&v, par())
                .map(float_series)
                .map_err(|e| e.to_string())
        }
        // vᵀX: a row vector times a matrix.
        (None, Some(b)) if is_vector(lhs) => {
            let v = match series_of(lhs) {
                Some(s) => dense_column(s, "the Series", func)?,
                None => Arc::new(floats(func, lhs)?),
            };
            b.t()
                .matvec_shared(&v, par())
                .map(float_series)
                .map_err(|_| {
                    format!(
                        "{}: a vector of {} times a {}×{} matrix (it needs {})",
                        func,
                        v.len(),
                        b.rows(),
                        b.ncols(),
                        b.rows()
                    )
                })
        }
        _ => {
            let a = want_matrix(func, Some(lhs), "the left side")?;
            let b = want_matrix(func, Some(rhs), "the right side")?;
            a.matmul(&b, par())
                .map(OdsMatrix::into_value)
                .map_err(|e| e.to_string())
        }
    }
}

// ── + - * / and -X ──────────────────────────────────────────────────
/// `a op b` element by element with a Matrix on either side: the other a
/// Matrix of the same shape (or a single row or column of it, broadcast),
/// a number, or a row of numbers — a Series or list as long as the matrix
/// is wide, applied to every row (a bias). `None` when neither side is a
/// Matrix or `op` is not `+ - * /`.
pub fn elementwise(op: &BinaryOp, lhs: &Value, rhs: &Value) -> Option<R<Value>> {
    let arith = match op {
        BinaryOp::Add => Arith::Add,
        BinaryOp::Subtract => Arith::Sub,
        BinaryOp::Multiply => Arith::Mul,
        BinaryOp::Divide => Arith::Div,
        _ => return None,
    };
    let (a, b) = (matrix_of(lhs), matrix_of(rhs));
    let out = match (a, b) {
        (None, None) => return None,
        (Some(a), Some(b)) => matrix::arith(a, arith, b, par()).map_err(|e| e.to_string()),
        (Some(a), None) => beside(a, arith, rhs, false),
        (None, Some(b)) => beside(b, arith, lhs, true),
    };
    Some(out.map(OdsMatrix::into_value))
}

/// A Matrix and the other side of `op`, the other written first when `first`.
fn beside(m: &Matrix, op: Arith, other: &Value, first: bool) -> R<Matrix> {
    let s = op.symbol();
    match other {
        Value::Float(x) => return Ok(matrix::arith_scalar(m, op, *x, first, par())),
        Value::Integer(i) => return Ok(matrix::arith_scalar(m, op, *i as f64, first, par())),
        _ => {}
    }
    if !is_vector(other) {
        return Err(format!(
            "Invalid binary operation: cannot apply '{}' to {} and {}",
            s,
            if first {
                other.type_name()
            } else {
                "Matrix".to_string()
            },
            if first {
                "Matrix".to_string()
            } else {
                other.type_name()
            }
        ));
    }
    let row = floats(&format!("`{}`", s), other)?;
    let (r, c) = m.shape();
    if row.len() != c {
        return Err(format!(
            "`{}`: a row of {} beside a {}×{} matrix: a Series or list is applied to every row, so it is as long as the matrix is wide ({}); a column is an n × 1 Matrix (ods.matrix([s]))",
            s,
            row.len(),
            r,
            c,
            c
        ));
    }
    let row = Matrix::from_rows(&[row]).map_err(|e| e.to_string())?;
    let (a, b) = if first { (&row, m) } else { (m, &row) };
    matrix::arith(a, op, b, par()).map_err(|e| e.to_string())
}

/// `-X`.
pub fn negate(op: &UnaryOp, v: &Value) -> Option<R<Value>> {
    let m = matrix_of(v)?;
    match op {
        UnaryOp::Negate => Some(Ok(OdsMatrix::into_value(matrix::arith_scalar(
            m,
            Arith::Mul,
            -1.0,
            false,
            par(),
        )))),
        UnaryOp::Not => Some(Err(
            "Invalid unary operation: cannot apply '!' to Matrix".to_string()
        )),
    }
}

/// A column function over a Matrix (`ods.tanh(X)`, `X.clip(lo: 0.0)`):
/// `call` runs it on each column as a Series, the other arguments as they
/// are; the columns make the Matrix it answers. `None` when the first
/// argument is no Matrix or `func` is not one of `COLUMN_FUNCTIONS`.
pub fn over_columns(
    func: &str,
    args: &[Value],
    call: impl Fn(&str, Vec<Value>) -> R<Value>,
) -> Option<R<Value>> {
    if !COLUMN_FUNCTIONS.iter().any(|(n, _)| *n == func) {
        return None;
    }
    let m = matrix_of(args.first()?)?;
    if args[1..].iter().any(|a| matrix_of(a).is_some()) {
        return Some(Err(format!(
            "ods.{}: a Matrix's {} takes numbers beside it, not another Matrix",
            func, func
        )));
    }
    let (r, c) = m.shape();
    let out = (|| -> R<Value> {
        let cols = (0..c)
            .map(|j| {
                let col = make_series_value(Series::F64 {
                    values: m.column(j),
                    validity: None,
                });
                let mut a = Vec::with_capacity(args.len());
                a.push(col);
                a.extend(args[1..].iter().cloned());
                let v = call(func, a)?;
                let s = series_of(&v).ok_or_else(|| {
                    format!(
                        "ods.{}: a column gave {}, not a Series",
                        func,
                        v.type_name()
                    )
                })?;
                dense_column(s, "a column", &format!("ods.{}", func))
            })
            .collect::<R<Vec<_>>>()?;
        Matrix::from_columns(r, cols)
            .map(OdsMatrix::into_value)
            .map_err(|e| e.to_string())
    })();
    Some(out)
}

// ── ods.matrix, ods.corr, ods.cov ───────────────────────────────────
pub fn ods_handles(func: &str) -> bool {
    ODS_FUNCTIONS.iter().any(|(n, _)| *n == func)
}

pub fn ods_dispatch(func: &str, args: &[Value]) -> R<Value> {
    let full = format!("ods.{}", func);
    let data = args
        .first()
        .ok_or_else(|| format!("{}: needs a Frame", full))?;
    match func {
        "matrix" => {
            let nulls = match args.get(2) {
                None | Some(Value::Unit) => "raise".to_string(),
                Some(Value::String(s)) => s.to_string(),
                Some(other) => {
                    return Err(format!(
                        "{}: nulls is \"raise\" or \"drop\", got {}",
                        full,
                        other.type_name()
                    ));
                }
            };
            build(&full, data, args.get(1), &nulls).map(OdsMatrix::into_value)
        }
        "corr" | "cov" => {
            let method = if func == "corr" {
                match args.get(2) {
                    None | Some(Value::Unit) => CorrMethod::Pearson,
                    Some(Value::String(s)) if s.as_str() == "pearson" => CorrMethod::Pearson,
                    Some(Value::String(s)) if s.as_str() == "spearman" => CorrMethod::Spearman,
                    Some(Value::String(s)) => {
                        return Err(format!(
                            "{}: method is \"pearson\" or \"spearman\", got \"{}\"",
                            full, s
                        ));
                    }
                    Some(other) => {
                        return Err(format!(
                            "{}: method is \"pearson\" or \"spearman\", got {}",
                            full,
                            other.type_name()
                        ));
                    }
                }
            } else {
                CorrMethod::Pearson
            };
            let named = named_columns(&full, data, args.get(1))?;
            let parts = named
                .iter()
                .map(|(n, s)| matrix::float_column(s, n).map_err(|e| format!("{}: {}", full, e)))
                .collect::<R<Vec<_>>>()?;
            let k = parts.len();
            let vals = matrix::corr_cov(&parts, method, func == "corr", par());
            let names: Vec<String> = named.iter().map(|(n, _)| n.clone()).collect();
            let mut out = vec![("term".to_string(), Series::from_str_values(names.clone()))];
            for (j, n) in names.iter().enumerate() {
                if n == "term" {
                    return Err(format!(
                        "{}: a column is called 'term', the table's label column; rename it first",
                        full
                    ));
                }
                out.push((
                    n.clone(),
                    Series::from_f64((0..k).map(|i| vals[i * k + j]).collect()),
                ));
            }
            Frame::new(out)
                .map(OdsFrame::into_value)
                .map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown ods function: {}", full)),
    }
}

// ── mat.* ───────────────────────────────────────────────────────────
pub fn dispatch(func: &str, args: Vec<Value>) -> R<Value> {
    let full = format!("mat.{}", func);
    let f = full.as_str();
    if let Some(r) = over_columns(func, &args, super::call) {
        return r;
    }
    if COLUMN_FUNCTIONS.iter().any(|(n, _)| *n == func) {
        return Err(format!(
            "{}: needs a Matrix, got {} (ods.{} takes a Series)",
            f,
            args.first()
                .map(|v| v.type_name())
                .unwrap_or_else(|| "nothing".to_string()),
            func
        ));
    }
    let a0 = args.first();
    match func {
        "of" => match a0 {
            Some(Value::List(rows)) if rows.iter().all(|r| matches!(r, Value::List(_))) => {
                let rows = rows.iter().map(|r| floats(f, r)).collect::<R<Vec<_>>>()?;
                Matrix::from_rows(&rows)
                    .map(OdsMatrix::into_value)
                    .map_err(|e| e.to_string())
            }
            other => Err(format!(
                "{}: needs a list of rows (lists of numbers), got {}",
                f,
                other
                    .map(|v| v.type_name())
                    .unwrap_or_else(|| "nothing".to_string())
            )),
        },
        "identity" => {
            let n = int_arg(f, a0, "n")?;
            if n < 0 {
                return Err(format!("{}: n must be >= 0, got {}", f, n));
            }
            Ok(OdsMatrix::into_value(Matrix::identity(n as usize)))
        }
        "t" => Ok(OdsMatrix::into_value(want_matrix(f, a0, "the matrix")?.t())),
        "gram" => Ok(OdsMatrix::into_value(
            want_matrix(f, a0, "the matrix")?.gram(par()),
        )),
        "matmul" => matmul_values(
            f,
            a0.ok_or_else(|| format!("{}: missing the left side", f))?,
            args.get(1)
                .ok_or_else(|| format!("{}: missing the right side", f))?,
        ),
        "col" => {
            let m = want_matrix(f, a0, "the matrix")?;
            let j = index_arg(f, args.get(1), m.ncols(), "column")?;
            Ok(make_series_value(Series::F64 {
                values: m.column(j),
                validity: None,
            }))
        }
        "row" => {
            let m = want_matrix(f, a0, "the matrix")?;
            let i = index_arg(f, args.get(1), m.rows(), "row")?;
            Ok(Value::List(Arc::new(
                m.row(i).into_iter().map(Value::Float).collect(),
            )))
        }
        "shape" => {
            let m = want_matrix(f, a0, "the matrix")?;
            let (r, c) = m.shape();
            Ok(Value::List(Arc::new(vec![
                Value::Integer(r as i64),
                Value::Integer(c as i64),
            ])))
        }
        "mean" | "std" | "sum" => {
            let m = want_matrix(f, a0, "the matrix")?;
            let axis = match args.get(1) {
                None | Some(Value::Unit) => 0,
                v => int_arg(f, v, "axis")?,
            };
            if !(0..=1).contains(&axis) {
                return Err(format!(
                    "{}: axis is 0 (one value per column) or 1 (one per row), got {}",
                    f, axis
                ));
            }
            let out = if func == "sum" {
                m.sum(axis as usize, par())
            } else if func == "mean" {
                m.mean(axis as usize, par())
            } else {
                let ddof = match args.get(2) {
                    None | Some(Value::Unit) => 1,
                    v => int_arg(f, v, "ddof")?,
                };
                if ddof < 0 {
                    return Err(format!("{}: ddof must be >= 0, got {}", f, ddof));
                }
                m.std(axis as usize, ddof as usize, par())
            };
            out.map(float_series).map_err(|e| e.to_string())
        }
        "solve" => {
            let a = want_matrix(f, a0, "the matrix")?;
            let b = args
                .get(1)
                .ok_or_else(|| format!("{}: missing the right-hand side", f))?;
            if is_vector(b) {
                let v = floats(f, b)?;
                let n = v.len();
                let rhs = Matrix::from_columns(n, vec![Arc::new(v)]).map_err(|e| e.to_string())?;
                let x = matrix::solve(&a, &rhs).map_err(|e| e.to_string())?;
                return Ok(make_series_value(Series::F64 {
                    values: x.column(0),
                    validity: None,
                }));
            }
            let rhs = want_matrix(f, Some(b), "the right-hand side")?;
            matrix::solve(&a, &rhs)
                .map(OdsMatrix::into_value)
                .map_err(|e| e.to_string())
        }
        "inv" => matrix::inv(&want_matrix(f, a0, "the matrix")?)
            .map(OdsMatrix::into_value)
            .map_err(|e| e.to_string()),
        "cholesky" => matrix::cholesky(&want_matrix(f, a0, "the matrix")?)
            .map(OdsMatrix::into_value)
            .map_err(|e| e.to_string()),
        "eigh" => {
            let (vals, vecs) =
                matrix::eigh(&want_matrix(f, a0, "the matrix")?).map_err(|e| e.to_string())?;
            let mut m = ValueMap::default();
            m.insert("values".to_string(), float_series(vals));
            m.insert("vectors".to_string(), OdsMatrix::into_value(vecs));
            Ok(Value::Map(Arc::new(m)))
        }
        "cdist" => {
            let x = want_matrix(f, a0, "the points")?;
            let c = want_matrix(f, args.get(1), "the second points")?;
            let metric = match args.get(2) {
                None | Some(Value::Unit) => Metric::SqEuclidean,
                Some(Value::String(s)) => Metric::parse(s).ok_or_else(|| {
                    format!(
                        "{}: metric is one of {}, got \"{}\"",
                        f,
                        Metric::NAMES.join(", "),
                        s
                    )
                })?,
                Some(other) => {
                    return Err(format!(
                        "{}: metric is a name ({}), got {}",
                        f,
                        Metric::NAMES.join(", "),
                        other.type_name()
                    ));
                }
            };
            matrix::cdist(&x, &c, metric, par())
                .map(OdsMatrix::into_value)
                .map_err(|e| e.to_string())
        }
        "to_frame" => {
            let m = want_matrix(f, a0, "the matrix")?;
            let names = match args.get(1) {
                None | Some(Value::Unit) => None,
                Some(Value::List(items)) => Some(
                    items
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => Ok(s.to_string()),
                            other => Err(format!(
                                "{}: the names are Strings, got {}",
                                f,
                                other.type_name()
                            )),
                        })
                        .collect::<R<Vec<_>>>()?,
                ),
                Some(other) => {
                    return Err(format!(
                        "{}: the names are a list of Strings, got {}",
                        f,
                        other.type_name()
                    ));
                }
            };
            if let Some(n) = &names
                && n.len() != m.ncols()
            {
                return Err(format!(
                    "{}: {} names for {} columns",
                    f,
                    n.len(),
                    m.ncols()
                ));
            }
            as_frame(&m, names).map(OdsFrame::into_value)
        }
        "to_list" => {
            let m = want_matrix(f, a0, "the matrix")?;
            Ok(Value::List(Arc::new(
                (0..m.rows())
                    .map(|i| {
                        Value::List(Arc::new(m.row(i).into_iter().map(Value::Float).collect()))
                    })
                    .collect(),
            )))
        }
        _ => Err(format!("unknown mat function: {}", f)),
    }
}

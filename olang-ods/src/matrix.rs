//! Dense matrices over columns: the `Matrix` value and the small dense
//! routines (`mat.*`) that work on it.
//!
//! A [`Matrix`] is column-major `f64`, and its columns are `Arc`s: built
//! from a Frame's null-free Float columns it *shares* them, so
//! `ods.matrix(frame, cols)` copies nothing. A transpose is a flag, so
//! `X.t()` is free and `X.t() @ y` reads `X` once, the way `Xᵀy` should.
//!
//! The shapes analysis meets are tall and skinny: millions of rows, tens
//! of columns. The O(rows) kernels are written for that shape:
//!
//! - **`X @ b`** walks the rows in L1-sized chunks, accumulating every
//!   output column of a chunk before moving on (an axpy per input column
//!   into a buffer that stays in cache), in parallel over pieces of rows.
//! - **`Xᵀ Y`** (and `X.gram()`, its symmetric case) is one pass over the
//!   rows: each chunk adds all of its column pairs' dot products to a
//!   piece's accumulator, and the pieces are summed in order, so the
//!   result does not depend on the thread count.
//! - **`cdist`** is blocked over row tiles, parallel over tiles.
//!
//! The k×k routines (LU solve and inverse, Cholesky, and `eigh` by
//! Householder tridiagonalization and implicit QL) are
//! textbook and exact enough at these orders; none needs BLAS, which
//! keeps the browser build small.

use crate::{OdsError, Series};
#[cfg(feature = "parallel")]
use rayon::prelude::*;
use std::sync::Arc;

type Result<T> = std::result::Result<T, OdsError>;

fn bad(msg: impl Into<String>) -> OdsError {
    OdsError::InvalidArgument(msg.into())
}

/// Rows a kernel's inner loop takes at a time: its buffers stay in L1.
const CHUNK: usize = 256;
/// Rows one parallel task takes: a fixed size, so a parallel sum adds the
/// same partial sums in the same order as a sequential one.
const PIECE: usize = 16_384;
/// Multiply-adds below which a kernel stays on one thread.
pub const PAR_WORK: usize = 1 << 21;

/// A dense `rows × cols` matrix of Floats, stored as columns.
#[derive(Clone, Debug)]
pub struct Matrix {
    /// The length of each stored column.
    n: usize,
    cols: Vec<Arc<Vec<f64>>>,
    /// The matrix is the transpose of what is stored.
    t: bool,
}

impl PartialEq for Matrix {
    fn eq(&self, other: &Matrix) -> bool {
        if self.shape() != other.shape() {
            return false;
        }
        let (r, c) = self.shape();
        (0..c).all(|j| (0..r).all(|i| self.get(i, j) == other.get(i, j)))
    }
}

impl Matrix {
    /// A matrix of these columns, each `n` long.
    pub fn from_columns(n: usize, cols: Vec<Arc<Vec<f64>>>) -> Result<Matrix> {
        if let Some(c) = cols.iter().find(|c| c.len() != n) {
            return Err(OdsError::LengthMismatch {
                left: n,
                right: c.len(),
            });
        }
        Ok(Matrix { n, cols, t: false })
    }

    /// A matrix of these rows (each the same length).
    pub fn from_rows(rows: &[Vec<f64>]) -> Result<Matrix> {
        let k = rows.first().map(|r| r.len()).unwrap_or(0);
        if let Some(r) = rows.iter().find(|r| r.len() != k) {
            return Err(bad(format!(
                "mat: the rows must be the same length, got {} and {}",
                k,
                r.len()
            )));
        }
        let cols = (0..k)
            .map(|j| Arc::new(rows.iter().map(|r| r[j]).collect::<Vec<f64>>()))
            .collect();
        Ok(Matrix {
            n: rows.len(),
            cols,
            t: false,
        })
    }

    /// From a row-major buffer of `rows × cols`.
    pub fn from_row_major(rows: usize, cols: usize, data: &[f64]) -> Matrix {
        debug_assert_eq!(data.len(), rows * cols);
        let out = (0..cols)
            .map(|j| Arc::new((0..rows).map(|i| data[i * cols + j]).collect::<Vec<f64>>()))
            .collect();
        Matrix {
            n: rows,
            cols: out,
            t: false,
        }
    }

    /// The `n × n` identity.
    pub fn identity(n: usize) -> Matrix {
        let cols = (0..n)
            .map(|j| {
                let mut c = vec![0.0; n];
                c[j] = 1.0;
                Arc::new(c)
            })
            .collect();
        Matrix { n, cols, t: false }
    }

    /// `(rows, cols)`.
    pub fn shape(&self) -> (usize, usize) {
        if self.t {
            (self.cols.len(), self.n)
        } else {
            (self.n, self.cols.len())
        }
    }

    pub fn rows(&self) -> usize {
        self.shape().0
    }

    pub fn ncols(&self) -> usize {
        self.shape().1
    }

    #[inline]
    pub fn get(&self, i: usize, j: usize) -> f64 {
        if self.t {
            self.cols[i][j]
        } else {
            self.cols[j][i]
        }
    }

    /// Whether the stored columns are the matrix's own (no transpose).
    pub fn is_transposed(&self) -> bool {
        self.t
    }

    /// The transpose: the same storage, flagged.
    pub fn t(&self) -> Matrix {
        Matrix {
            n: self.n,
            cols: self.cols.clone(),
            t: !self.t,
        }
    }

    /// The same matrix with its own columns stored (a transpose gathers).
    pub fn materialize(&self) -> Matrix {
        if !self.t {
            return self.clone();
        }
        let (r, c) = self.shape();
        let cols = (0..c)
            .map(|j| Arc::new((0..r).map(|i| self.cols[i][j]).collect::<Vec<f64>>()))
            .collect();
        Matrix {
            n: r,
            cols,
            t: false,
        }
    }

    /// Column `j`: shared when stored, gathered for a transpose.
    pub fn column(&self, j: usize) -> Arc<Vec<f64>> {
        if self.t {
            Arc::new(self.cols.iter().map(|c| c[j]).collect())
        } else {
            self.cols[j].clone()
        }
    }

    /// Every column, as `column` gives them.
    pub fn columns(&self) -> Vec<Arc<Vec<f64>>> {
        if self.t {
            self.materialize().cols
        } else {
            self.cols.clone()
        }
    }

    /// Row `i`.
    pub fn row(&self, i: usize) -> Vec<f64> {
        if self.t {
            self.cols[i].as_ref().clone()
        } else {
            self.cols.iter().map(|c| c[i]).collect()
        }
    }

    /// Row-major copy.
    pub fn to_row_major(&self) -> Vec<f64> {
        let (r, c) = self.shape();
        let mut out = vec![0.0; r * c];
        if self.t {
            for (i, row) in self.cols.iter().enumerate() {
                out[i * c..(i + 1) * c].copy_from_slice(row);
            }
        } else {
            for (j, col) in self.cols.iter().enumerate() {
                for (i, v) in col.iter().enumerate() {
                    out[i * c + j] = *v;
                }
            }
        }
        out
    }

    // -----------------------------------------------------------------
    // Products
    // -----------------------------------------------------------------

    /// `self @ b` for a vector `b` of length `cols`.
    pub fn matvec(&self, b: &[f64], par: bool) -> Result<Vec<f64>> {
        let (r, c) = self.shape();
        if b.len() != c {
            return Err(bad(format!(
                "`@`: a {}×{} matrix times a vector of {} (it needs {})",
                r,
                c,
                b.len(),
                c
            )));
        }
        if self.t {
            // Sᵀb: one pass over the stored columns.
            return Ok(cross(
                &self.cols,
                &[Arc::new(b.to_vec())],
                self.n,
                false,
                par,
            ));
        }
        let m = self.matmul_dense(b, 1, par)?;
        Ok(m.cols[0].as_ref().clone())
    }

    /// `self @ b` for a shared vector: `Xᵀy` reads `y` where it is.
    pub fn matvec_shared(&self, b: &Arc<Vec<f64>>, par: bool) -> Result<Vec<f64>> {
        if self.t && b.len() == self.n {
            return Ok(cross(
                &self.cols,
                std::slice::from_ref(b),
                self.n,
                false,
                par,
            ));
        }
        self.matvec(b, par)
    }

    /// `self @ other`.
    pub fn matmul(&self, other: &Matrix, par: bool) -> Result<Matrix> {
        let (r, c) = self.shape();
        let (r2, c2) = other.shape();
        if c != r2 {
            return Err(bad(format!(
                "`@`: the shapes do not chain: {}×{} @ {}×{} (the left's columns must equal the right's rows)",
                r, c, r2, c2
            )));
        }
        match (self.t, other.t) {
            // Sᵀ B: one pass over the rows S and B share.
            (true, false) => {
                let same = same_cols(&self.cols, &other.cols);
                let g = cross(&self.cols, &other.cols, self.n, same, par);
                Ok(Matrix::from_row_major(
                    self.cols.len(),
                    other.cols.len(),
                    &g,
                ))
            }
            // Sᵀ Tᵀ = (T S)ᵀ.
            (true, true) => Ok(other.t().matmul(&self.t(), par)?.t()),
            // A B, B dense (its rows are A's columns: few).
            (false, _) => {
                let dense = other.to_row_major();
                self.matmul_dense(&dense, c2, par)
            }
        }
    }

    /// `self @ B` for a stored (untransposed) self and `B` row-major
    /// `cols × m`.
    fn matmul_dense(&self, b: &[f64], m: usize, par: bool) -> Result<Matrix> {
        if self.t {
            return self.materialize().matmul_dense(b, m, par);
        }
        let n = self.n;
        let k = self.cols.len();
        let cols: Vec<&[f64]> = self.cols.iter().map(|c| c.as_slice()).collect();
        let mut outs: Vec<Vec<f64>> = (0..m).map(|_| vec![0.0; n]).collect();
        // One piece of rows: every output column for those rows, a chunk
        // at a time.
        fill_pieces(
            &mut outs,
            par && n * k * m.max(1) >= PAR_WORK,
            |start, parts| {
                let len = parts.first().map(|p| p.len()).unwrap_or(0);
                let mut s = 0;
                while s < len {
                    let e = (s + CHUNK).min(len);
                    let (lo, hi) = (start + s, start + e);
                    for (o, outc) in parts.iter_mut().enumerate() {
                        let buf = &mut outc[s..e];
                        // Four columns a pass: the buffer is read and written
                        // a quarter as often.
                        let mut j = 0;
                        while j + 4 <= k {
                            let (w0, w1, w2, w3) = (
                                b[j * m + o],
                                b[(j + 1) * m + o],
                                b[(j + 2) * m + o],
                                b[(j + 3) * m + o],
                            );
                            let (c0, c1, c2, c3) = (
                                &cols[j][lo..hi],
                                &cols[j + 1][lo..hi],
                                &cols[j + 2][lo..hi],
                                &cols[j + 3][lo..hi],
                            );
                            let len = buf.len();
                            let (c0, c1, c2, c3) = (&c0[..len], &c1[..len], &c2[..len], &c3[..len]);
                            for r in 0..len {
                                buf[r] += (w0 * c0[r] + w1 * c1[r]) + (w2 * c2[r] + w3 * c3[r]);
                            }
                            j += 4;
                        }
                        while j < k {
                            let w = b[j * m + o];
                            for (x, v) in buf.iter_mut().zip(&cols[j][lo..hi]) {
                                *x += w * v;
                            }
                            j += 1;
                        }
                    }
                    s = e;
                }
            },
        );
        Ok(Matrix {
            n,
            cols: outs.into_iter().map(Arc::new).collect(),
            t: false,
        })
    }

    /// `XᵀX` (`cols × cols`), one pass over the rows.
    pub fn gram(&self, par: bool) -> Matrix {
        self.t()
            .matmul(self, par)
            .expect("a matrix chains with its transpose")
    }

    // -----------------------------------------------------------------
    // Reductions along an axis
    // -----------------------------------------------------------------

    /// Means down the columns (`axis` 0, one per column) or across the
    /// rows (`axis` 1, one per row).
    pub fn mean(&self, axis: usize, par: bool) -> Result<Vec<f64>> {
        let along_stored = self.stored_axis(axis)?;
        Ok(if along_stored {
            self.cols.iter().map(|c| sum(c) / self.n as f64).collect()
        } else {
            let k = self.cols.len() as f64;
            row_sums(&self.cols, self.n, par)
                .into_iter()
                .map(|s| s / k)
                .collect()
        })
    }

    /// Standard deviations along an axis, with `ddof` (1: the sample's,
    /// as `ods.std`).
    pub fn std(&self, axis: usize, ddof: usize, par: bool) -> Result<Vec<f64>> {
        let along_stored = self.stored_axis(axis)?;
        let means = self.mean(axis, par)?;
        Ok(if along_stored {
            let d = self.n as f64 - ddof as f64;
            self.cols
                .iter()
                .zip(&means)
                .map(|(c, m)| {
                    if d <= 0.0 {
                        return f64::NAN;
                    }
                    (c.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / d).sqrt()
                })
                .collect()
        } else {
            let d = self.cols.len() as f64 - ddof as f64;
            let mut ss = vec![0.0; self.n];
            for c in &self.cols {
                for ((s, v), m) in ss.iter_mut().zip(c.iter()).zip(&means) {
                    *s += (v - m) * (v - m);
                }
            }
            ss.into_iter()
                .map(|s| if d <= 0.0 { f64::NAN } else { (s / d).sqrt() })
                .collect()
        })
    }

    /// Whether `axis` (of the matrix) runs down the stored columns.
    fn stored_axis(&self, axis: usize) -> Result<bool> {
        match axis {
            0 | 1 => Ok((axis == 0) != self.t),
            other => Err(bad(format!(
                "mat: axis is 0 (down the columns) or 1 (across the rows), got {}",
                other
            ))),
        }
    }
}

fn same_cols(a: &[Arc<Vec<f64>>], b: &[Arc<Vec<f64>>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y))
}

/// Fill output columns (each `n` long) a fixed piece of rows at a time:
/// `f(start, parts)` gets the piece's slice of every column. In parallel
/// when `par`; each piece is written once, so the result is the
/// sequential one.
fn fill_pieces(outs: &mut [Vec<f64>], par: bool, f: impl Fn(usize, &mut [&mut [f64]]) + Sync) {
    let n = outs.first().map(|o| o.len()).unwrap_or(0);
    let n_pieces = n.div_ceil(PIECE).max(1);
    let mut pieces: Vec<(usize, Vec<&mut [f64]>)> = (0..n_pieces)
        .map(|p| (p * PIECE, Vec::with_capacity(outs.len())))
        .collect();
    for o in outs.iter_mut() {
        for (p, chunk) in o.chunks_mut(PIECE).enumerate() {
            pieces[p].1.push(chunk);
        }
    }
    #[cfg(feature = "parallel")]
    {
        if par && n_pieces > 1 {
            pieces.par_iter_mut().for_each(|(s, parts)| f(*s, parts));
            return;
        }
    }
    let _ = par;
    for (s, parts) in pieces.iter_mut() {
        f(*s, parts);
    }
}

/// Cut `0..n` into fixed pieces and run `f` on each, in parallel when
/// `par`; results in order.
fn run_pieces<T: Send>(n: usize, par: bool, f: impl Fn(usize, usize) -> T + Sync) -> Vec<T> {
    let starts: Vec<usize> = (0..n.max(1)).step_by(PIECE).collect();
    let one = |&s: &usize| f(s, (s + PIECE).min(n));
    #[cfg(feature = "parallel")]
    {
        if par && starts.len() > 1 {
            return starts.par_iter().map(one).collect();
        }
    }
    let _ = par;
    starts.iter().map(one).collect()
}

#[inline]
fn dot(a: &[f64], b: &[f64]) -> f64 {
    crate::learn::linalg::dot(a, b)
}

fn sum(a: &[f64]) -> f64 {
    let mut acc = [0.0f64; 4];
    let (c, r) = a.as_chunks::<4>();
    for x in c {
        for l in 0..4 {
            acc[l] += x[l];
        }
    }
    (acc[0] + acc[1]) + (acc[2] + acc[3]) + r.iter().sum::<f64>()
}

fn row_sums(cols: &[Arc<Vec<f64>>], n: usize, par: bool) -> Vec<f64> {
    let pieces = run_pieces(n, par && n * cols.len() >= PAR_WORK, |s, e| {
        let mut out = vec![0.0; e - s];
        for c in cols {
            for (o, v) in out.iter_mut().zip(&c[s..e]) {
                *o += v;
            }
        }
        out
    });
    pieces.concat()
}

/// `AᵀB` for column sets `a` and `b` of `n` rows: row-major
/// `a.len() × b.len()`. `sym` (a and b the same columns) computes the
/// upper triangle and mirrors it.
pub fn cross(a: &[Arc<Vec<f64>>], b: &[Arc<Vec<f64>>], n: usize, sym: bool, par: bool) -> Vec<f64> {
    let ka = a.len();
    let kb = b.len();
    let piece = |s: usize, e: usize| -> Vec<f64> {
        let mut acc = vec![0.0; ka * kb];
        let mut c0 = s;
        while c0 < e {
            let c1 = (c0 + CHUNK).min(e);
            for i in 0..ka {
                let ai = &a[i][c0..c1];
                let from = if sym { i } else { 0 };
                for j in from..kb {
                    acc[i * kb + j] += dot(ai, &b[j][c0..c1]);
                }
            }
            c0 = c1;
        }
        acc
    };
    let work = n * ka * kb / if sym { 2 } else { 1 };
    let pieces = run_pieces(n, par && work >= PAR_WORK, piece);
    let mut out = vec![0.0; ka * kb];
    for p in pieces {
        for (o, v) in out.iter_mut().zip(p) {
            *o += v;
        }
    }
    if sym {
        for i in 0..ka {
            for j in 0..i {
                out[i * kb + j] = out[j * kb + i];
            }
        }
    }
    out
}

// ---------------------------------------------------------------------
// k × k routines
// ---------------------------------------------------------------------

fn square(a: &Matrix, what: &str) -> Result<usize> {
    let (r, c) = a.shape();
    if r != c {
        return Err(bad(format!(
            "{}: needs a square matrix, got {}×{}",
            what, r, c
        )));
    }
    Ok(r)
}

/// LU with partial pivoting of a row-major `n × n` matrix, in place.
/// Returns the row permutation, or None when the matrix is singular.
fn lu(a: &mut [f64], n: usize) -> Option<Vec<usize>> {
    let mut perm: Vec<usize> = (0..n).collect();
    let scale = a.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let tiny = scale * n as f64 * f64::EPSILON;
    for k in 0..n {
        let p = (k..n).max_by(|&i, &j| a[i * n + k].abs().total_cmp(&a[j * n + k].abs()))?;
        if a[p * n + k].abs() <= tiny || !a[p * n + k].is_finite() {
            return None;
        }
        if p != k {
            for j in 0..n {
                a.swap(k * n + j, p * n + j);
            }
            perm.swap(k, p);
        }
        let d = a[k * n + k];
        for i in k + 1..n {
            let f = a[i * n + k] / d;
            a[i * n + k] = f;
            if f != 0.0 {
                for j in k + 1..n {
                    a[i * n + j] -= f * a[k * n + j];
                }
            }
        }
    }
    Some(perm)
}

fn lu_solve(lu: &[f64], perm: &[usize], n: usize, b: &[f64]) -> Vec<f64> {
    let mut x: Vec<f64> = perm.iter().map(|&p| b[p]).collect();
    for i in 0..n {
        let mut s = x[i];
        for j in 0..i {
            s -= lu[i * n + j] * x[j];
        }
        x[i] = s;
    }
    for i in (0..n).rev() {
        let mut s = x[i];
        for j in i + 1..n {
            s -= lu[i * n + j] * x[j];
        }
        x[i] = s / lu[i * n + i];
    }
    x
}

/// `lu`'s solution for every column of `b` (row-major `n × m`) at once:
/// each step is an axpy over a contiguous row, which vectorizes.
fn lu_solve_many(lu: &[f64], perm: &[usize], n: usize, b: &[f64], m: usize) -> Vec<f64> {
    let mut x = vec![0.0; n * m];
    for (i, &p) in perm.iter().enumerate() {
        x[i * m..(i + 1) * m].copy_from_slice(&b[p * m..(p + 1) * m]);
    }
    for i in 0..n {
        let (done, rest) = x.split_at_mut(i * m);
        let xi = &mut rest[..m];
        for j in 0..i {
            let l = lu[i * n + j];
            if l != 0.0 {
                for (a, v) in xi.iter_mut().zip(&done[j * m..(j + 1) * m]) {
                    *a -= l * v;
                }
            }
        }
    }
    for i in (0..n).rev() {
        let (head, tail) = x.split_at_mut((i + 1) * m);
        let xi = &mut head[i * m..];
        for j in i + 1..n {
            let u = lu[i * n + j];
            if u != 0.0 {
                for (a, v) in xi.iter_mut().zip(&tail[(j - i - 1) * m..(j - i) * m]) {
                    *a -= u * v;
                }
            }
        }
        let d = lu[i * n + i];
        for a in xi.iter_mut() {
            *a /= d;
        }
    }
    x
}

/// `x` with `a x = b`, `b` a matrix of right-hand sides.
pub fn solve(a: &Matrix, b: &Matrix) -> Result<Matrix> {
    let n = square(a, "mat.solve")?;
    if b.rows() != n {
        return Err(bad(format!(
            "mat.solve: a {}×{} system needs {} rows on the right, got {}",
            n,
            n,
            n,
            b.rows()
        )));
    }
    let mut m = a.to_row_major();
    let perm = lu(&mut m, n).ok_or_else(|| bad("mat.solve: the matrix is singular"))?;
    if b.ncols() == 1 {
        let x = lu_solve(&m, &perm, n, &b.column(0));
        return Ok(Matrix {
            n,
            cols: vec![Arc::new(x)],
            t: false,
        });
    }
    let k = b.ncols();
    let x = lu_solve_many(&m, &perm, n, &b.to_row_major(), k);
    Ok(Matrix::from_row_major(n, k, &x))
}

/// `a⁻¹`.
pub fn inv(a: &Matrix) -> Result<Matrix> {
    let n = square(a, "mat.inv")?;
    solve(a, &Matrix::identity(n)).map_err(|_| bad("mat.inv: the matrix is singular"))
}

fn symmetric(a: &Matrix, what: &str) -> Result<(usize, Vec<f64>)> {
    let n = square(a, what)?;
    let m = a.to_row_major();
    let scale = m.iter().fold(0.0f64, |s, v| s.max(v.abs())).max(1e-300);
    for i in 0..n {
        for j in 0..i {
            if (m[i * n + j] - m[j * n + i]).abs() > 1e-10 * scale {
                return Err(bad(format!(
                    "{}: needs a symmetric matrix; [{}, {}] is {} but [{}, {}] is {}",
                    what,
                    i,
                    j,
                    m[i * n + j],
                    j,
                    i,
                    m[j * n + i]
                )));
            }
        }
    }
    Ok((n, m))
}

/// The lower Cholesky factor `L` (`a = L Lᵀ`) of a symmetric positive
/// definite matrix.
pub fn cholesky(a: &Matrix) -> Result<Matrix> {
    let (n, m) = symmetric(a, "mat.cholesky")?;
    let l = crate::learn::linalg::cholesky(&m, n)
        .ok_or_else(|| bad("mat.cholesky: the matrix is not positive definite"))?;
    Ok(Matrix::from_row_major(n, n, &l))
}

/// Eigenvalues (largest first) and unit eigenvectors (the columns of a
/// matrix) of a symmetric matrix: Householder reduction to tridiagonal
/// form, then the implicit QL algorithm (EISPACK's tred2 and tql2, as
/// JAMA writes them), O(k³). Each vector's largest component (by
/// magnitude) is positive, so the signs are deterministic.
pub fn eigh(a: &Matrix) -> Result<(Vec<f64>, Matrix)> {
    let (n, m) = symmetric(a, "mat.eigh")?;
    if n == 0 {
        return Ok((Vec::new(), Matrix::from_row_major(0, 0, &[])));
    }
    let (vals, vecs) = sym_eigen(&m, n);
    // Largest first.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| vals[j].total_cmp(&vals[i]));
    let values: Vec<f64> = order.iter().map(|&i| vals[i]).collect();
    let mut out = vec![0.0; n * n];
    for (new, &old) in order.iter().enumerate() {
        let mut big = 0usize;
        for i in 0..n {
            if vecs[i * n + old].abs() > vecs[big * n + old].abs() + 1e-12 {
                big = i;
            }
        }
        let sign = if vecs[big * n + old] < 0.0 { -1.0 } else { 1.0 };
        for i in 0..n {
            out[i * n + new] = sign * vecs[i * n + old];
        }
    }
    Ok((values, Matrix::from_row_major(n, n, &out)))
}

/// tred2 + tql2 on a row-major symmetric `n × n` matrix: eigenvalues
/// (unordered) and eigenvectors as the columns of a row-major matrix.
fn sym_eigen(a: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut v = a.to_vec();
    let mut d = vec![0.0f64; n];
    let mut e = vec![0.0f64; n];
    let ix = |i: usize, j: usize| i * n + j;
    // Householder reduction to tridiagonal form.
    for j in 0..n {
        d[j] = v[ix(n - 1, j)];
    }
    for i in (1..n).rev() {
        let mut scale = 0.0;
        let mut h = 0.0;
        for dk in d.iter().take(i) {
            scale += dk.abs();
        }
        if scale == 0.0 {
            e[i] = d[i - 1];
            for j in 0..i {
                d[j] = v[ix(i - 1, j)];
                v[ix(i, j)] = 0.0;
                v[ix(j, i)] = 0.0;
            }
        } else {
            for dk in d.iter_mut().take(i) {
                *dk /= scale;
                h += *dk * *dk;
            }
            let mut f = d[i - 1];
            let mut g = h.sqrt();
            if f > 0.0 {
                g = -g;
            }
            e[i] = scale * g;
            h -= f * g;
            d[i - 1] = f - g;
            for ej in e.iter_mut().take(i) {
                *ej = 0.0;
            }
            for j in 0..i {
                f = d[j];
                v[ix(j, i)] = f;
                g = e[j] + v[ix(j, j)] * f;
                for k in j + 1..i {
                    g += v[ix(k, j)] * d[k];
                    e[k] += v[ix(k, j)] * f;
                }
                e[j] = g;
            }
            f = 0.0;
            for j in 0..i {
                e[j] /= h;
                f += e[j] * d[j];
            }
            let hh = f / (h + h);
            for j in 0..i {
                e[j] -= hh * d[j];
            }
            for j in 0..i {
                f = d[j];
                g = e[j];
                for k in j..i {
                    v[ix(k, j)] -= f * e[k] + g * d[k];
                }
                d[j] = v[ix(i - 1, j)];
                v[ix(i, j)] = 0.0;
            }
        }
        d[i] = h;
    }
    // Accumulate the transformations.
    for i in 0..n - 1 {
        v[ix(n - 1, i)] = v[ix(i, i)];
        v[ix(i, i)] = 1.0;
        let h = d[i + 1];
        if h != 0.0 {
            for k in 0..=i {
                d[k] = v[ix(k, i + 1)] / h;
            }
            for j in 0..=i {
                let mut g = 0.0;
                for k in 0..=i {
                    g += v[ix(k, i + 1)] * v[ix(k, j)];
                }
                for k in 0..=i {
                    v[ix(k, j)] -= g * d[k];
                }
            }
        }
        for k in 0..=i {
            v[ix(k, i + 1)] = 0.0;
        }
    }
    for j in 0..n {
        d[j] = v[ix(n - 1, j)];
        v[ix(n - 1, j)] = 0.0;
    }
    v[ix(n - 1, n - 1)] = 1.0;
    e[0] = 0.0;
    // The QL sweeps rotate pairs of eigenvector columns: work on the
    // transpose, so each rotation runs along two contiguous rows.
    let mut w = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            w[j * n + i] = v[i * n + j];
        }
    }
    let mut v = w;
    // Implicit QL on the tridiagonal matrix.
    for i in 1..n {
        e[i - 1] = e[i];
    }
    e[n - 1] = 0.0;
    let mut f = 0.0f64;
    let mut tst1 = 0.0f64;
    let eps = f64::EPSILON;
    for l in 0..n {
        tst1 = tst1.max(d[l].abs() + e[l].abs());
        let mut m = l;
        while m < n {
            if e[m].abs() <= eps * tst1 {
                break;
            }
            m += 1;
        }
        if m > l {
            for _iter in 0..60 {
                let mut g = d[l];
                let mut p = (d[l + 1] - g) / (2.0 * e[l]);
                let mut r = p.hypot(1.0);
                if p < 0.0 {
                    r = -r;
                }
                d[l] = e[l] / (p + r);
                d[l + 1] = e[l] * (p + r);
                let dl1 = d[l + 1];
                let mut h = g - d[l];
                for di in d.iter_mut().skip(l + 2) {
                    *di -= h;
                }
                f += h;
                p = d[m];
                let mut c = 1.0;
                let mut c2 = c;
                let mut c3 = c;
                let el1 = e[l + 1];
                let mut s = 0.0;
                let mut s2 = 0.0;
                for i in (l..m).rev() {
                    c3 = c2;
                    c2 = c;
                    s2 = s;
                    g = c * e[i];
                    h = c * p;
                    r = p.hypot(e[i]);
                    e[i + 1] = s * r;
                    s = e[i] / r;
                    c = p / r;
                    p = c * d[i] - s * g;
                    d[i + 1] = h + s * (c * g + s * d[i]);
                    let (lo, hi) = v.split_at_mut((i + 1) * n);
                    let (row_i, row_i1) = (&mut lo[i * n..], &mut hi[..n]);
                    for (vi, vi1) in row_i.iter_mut().zip(row_i1.iter_mut()) {
                        let t = *vi1;
                        *vi1 = s * *vi + c * t;
                        *vi = c * *vi - s * t;
                    }
                }
                p = -s * s2 * c3 * el1 * e[l] / dl1;
                e[l] = s * p;
                d[l] = c * p;
                if e[l].abs() <= eps * tst1 {
                    break;
                }
            }
        }
        d[l] += f;
        e[l] = 0.0;
    }
    let mut out = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            out[j * n + i] = v[i * n + j];
        }
    }
    (d, out)
}

// ---------------------------------------------------------------------
// Distances
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    SqEuclidean,
    Euclidean,
    Cityblock,
    Chebyshev,
    Cosine,
}

impl Metric {
    pub const NAMES: &'static [&'static str] = &[
        "sqeuclidean",
        "euclidean",
        "cityblock",
        "manhattan",
        "chebyshev",
        "cosine",
    ];

    pub fn parse(name: &str) -> Option<Metric> {
        Some(match name {
            "sqeuclidean" => Metric::SqEuclidean,
            "euclidean" => Metric::Euclidean,
            "cityblock" | "manhattan" => Metric::Cityblock,
            "chebyshev" => Metric::Chebyshev,
            "cosine" => Metric::Cosine,
            _ => return None,
        })
    }
}

#[inline]
fn distance(x: &[f64], c: &[f64], metric: Metric, nx: f64, nc: f64) -> f64 {
    match metric {
        Metric::SqEuclidean | Metric::Euclidean => {
            let mut acc = [0.0f64; 4];
            let (cx, rx) = x.as_chunks::<4>();
            let (cc, rc) = c.as_chunks::<4>();
            for (a, b) in cx.iter().zip(cc) {
                for l in 0..4 {
                    let d = a[l] - b[l];
                    acc[l] += d * d;
                }
            }
            let mut s = (acc[0] + acc[1]) + (acc[2] + acc[3]);
            for (a, b) in rx.iter().zip(rc) {
                s += (a - b) * (a - b);
            }
            if metric == Metric::Euclidean {
                s.sqrt()
            } else {
                s
            }
        }
        Metric::Cityblock => x.iter().zip(c).map(|(a, b)| (a - b).abs()).sum(),
        Metric::Chebyshev => x
            .iter()
            .zip(c)
            .fold(0.0f64, |m, (a, b)| m.max((a - b).abs())),
        Metric::Cosine => {
            let d = dot(x, c);
            if nx == 0.0 || nc == 0.0 {
                f64::NAN
            } else {
                1.0 - d / (nx * nc)
            }
        }
    }
}

/// Pairwise distances between the rows of `x` (`n × k`) and of `c`
/// (`m × k`): an `n × m` matrix.
pub fn cdist(x: &Matrix, c: &Matrix, metric: Metric, par: bool) -> Result<Matrix> {
    let (n, k) = x.shape();
    let (m, k2) = c.shape();
    if k != k2 {
        return Err(bad(format!(
            "mat.cdist: the rows have {} and {} values; they must have the same",
            k, k2
        )));
    }
    let xr = x.to_row_major();
    let cr = c.to_row_major();
    let norms = |r: &[f64], rows: usize| -> Vec<f64> {
        if metric != Metric::Cosine {
            return Vec::new();
        }
        (0..rows)
            .map(|i| dot(&r[i * k..(i + 1) * k], &r[i * k..(i + 1) * k]).sqrt())
            .collect()
    };
    let nxs = norms(&xr, n);
    let ncs = norms(&cr, m);
    let mut cols: Vec<Vec<f64>> = (0..m).map(|_| vec![0.0; n]).collect();
    // A task is one centre against a piece of rows: the centre stays in
    // registers, the rows stream once, and every output written once.
    let mut tasks: Vec<(usize, usize, &mut [f64])> = Vec::new();
    for (j, col) in cols.iter_mut().enumerate() {
        for (p, chunk) in col.chunks_mut(PIECE).enumerate() {
            tasks.push((j, p * PIECE, chunk));
        }
    }
    let run = |(j, s, out): &mut (usize, usize, &mut [f64])| {
        let cj = &cr[*j * k..(*j + 1) * k];
        let nc = ncs.get(*j).copied().unwrap_or(0.0);
        for (r, o) in out.iter_mut().enumerate() {
            let i = *s + r;
            let nx = nxs.get(i).copied().unwrap_or(0.0);
            *o = distance(&xr[i * k..(i + 1) * k], cj, metric, nx, nc);
        }
    };
    let par = par && n * m * k.max(1) >= PAR_WORK / 4 && tasks.len() > 1;
    #[cfg(feature = "parallel")]
    {
        if par {
            tasks.par_iter_mut().for_each(run);
        } else {
            tasks.iter_mut().for_each(run);
        }
    }
    #[cfg(not(feature = "parallel"))]
    {
        let _ = par;
        tasks.iter_mut().for_each(run);
    }
    drop(tasks);
    Ok(Matrix {
        n,
        cols: cols.into_iter().map(Arc::new).collect(),
        t: false,
    })
}

// ---------------------------------------------------------------------
// Correlation and covariance of columns
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrMethod {
    Pearson,
    Spearman,
}

/// A column's floats and, when some are not, which rows are usable.
pub type FloatColumn = (Arc<Vec<f64>>, Option<Vec<bool>>);

/// A numeric column as Floats and which rows are usable (valid, not NaN).
pub fn float_column(s: &Series, name: &str) -> Result<FloatColumn> {
    let (vals, validity) = float_values(s, name)?;
    let nan = vals.iter().any(|v| v.is_nan());
    let mask = if validity.is_some() || nan {
        Some(
            (0..vals.len())
                .map(|i| validity.map(|b| b.get(i)).unwrap_or(true) && !vals[i].is_nan())
                .collect(),
        )
    } else {
        None
    };
    Ok((vals, mask))
}

/// A numeric column's values as Floats (shared when they are Floats)
/// and its validity, as stored.
pub fn float_values<'a>(
    s: &'a Series,
    name: &str,
) -> Result<(Arc<Vec<f64>>, Option<&'a crate::Bitmap>)> {
    Ok(match s {
        Series::F64 { values, validity } => (values.clone(), validity.as_ref()),
        Series::I64 { values, validity } => (
            Arc::new(values.iter().map(|&v| v as f64).collect()),
            validity.as_ref(),
        ),
        Series::Bool { values, validity } => (
            Arc::new(values.iter().map(|&v| if v { 1.0 } else { 0.0 }).collect()),
            validity.as_ref(),
        ),
        other => {
            return Err(OdsError::TypeMismatch(format!(
                "column '{}' is {}; a matrix takes Float, Int or Bool columns",
                name,
                other.dtype()
            )));
        }
    })
}

/// Average ranks (ties share the mean of their positions), 1-based.
fn ranks(v: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_unstable_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut out = vec![0.0; v.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i + 1;
        while j < idx.len() && v[idx[j]] == v[idx[i]] {
            j += 1;
        }
        let r = (i + 1 + j) as f64 / 2.0;
        for &p in &idx[i..j] {
            out[p] = r;
        }
        i = j;
    }
    out
}

fn centred(v: &[f64]) -> Vec<f64> {
    let m = sum(v) / v.len() as f64;
    v.iter().map(|x| x - m).collect()
}

/// One pair over the rows where both are usable (pandas' pairwise
/// complete observations).
fn pair_stat(
    a: &[f64],
    b: &[f64],
    ma: Option<&Vec<bool>>,
    mb: Option<&Vec<bool>>,
    method: CorrMethod,
    corr: bool,
) -> f64 {
    let (mut xa, mut xb) = (Vec::new(), Vec::new());
    for i in 0..a.len() {
        if ma.is_none_or(|m| m[i]) && mb.is_none_or(|m| m[i]) {
            xa.push(a[i]);
            xb.push(b[i]);
        }
    }
    let n = xa.len();
    if n < 2 {
        return f64::NAN;
    }
    if method == CorrMethod::Spearman {
        xa = ranks(&xa);
        xb = ranks(&xb);
    }
    let ca = centred(&xa);
    let cb = centred(&xb);
    let sab = dot(&ca, &cb);
    if corr {
        let d = (dot(&ca, &ca) * dot(&cb, &cb)).sqrt();
        if d == 0.0 {
            f64::NAN
        } else {
            (sab / d).clamp(-1.0, 1.0)
        }
    } else {
        sab / (n - 1) as f64
    }
}

/// The correlation (`corr`) or sample covariance matrix of columns,
/// row-major `k × k`. Null-free columns take one pass (centred columns'
/// Gram matrix); a column with nulls (or NaN) pairs over the rows both
/// have, as pandas does.
pub fn corr_cov(cols: &[FloatColumn], method: CorrMethod, corr: bool, par: bool) -> Vec<f64> {
    let k = cols.len();
    let n = cols.first().map(|c| c.0.len()).unwrap_or(0);
    let mut out = vec![f64::NAN; k * k];
    if cols.iter().all(|c| c.1.is_none()) {
        if n < 2 {
            return out;
        }
        let prep = |c: &FloatColumn| -> Arc<Vec<f64>> {
            let v = if method == CorrMethod::Spearman {
                ranks(&c.0)
            } else {
                c.0.as_ref().clone()
            };
            Arc::new(centred(&v))
        };
        #[cfg(feature = "parallel")]
        let centred_cols: Vec<Arc<Vec<f64>>> = if par && n * k >= PAR_WORK / 4 {
            cols.par_iter().map(prep).collect()
        } else {
            cols.iter().map(prep).collect()
        };
        #[cfg(not(feature = "parallel"))]
        let centred_cols: Vec<Arc<Vec<f64>>> = cols.iter().map(prep).collect();
        let g = cross(&centred_cols, &centred_cols, n, true, par);
        for i in 0..k {
            for j in 0..k {
                out[i * k + j] = if corr {
                    let d = (g[i * k + i] * g[j * k + j]).sqrt();
                    if i == j && d > 0.0 {
                        1.0
                    } else if d == 0.0 {
                        f64::NAN
                    } else {
                        (g[i * k + j] / d).clamp(-1.0, 1.0)
                    }
                } else {
                    g[i * k + j] / (n - 1) as f64
                };
            }
        }
        return out;
    }
    let pairs: Vec<(usize, usize)> = (0..k).flat_map(|i| (i..k).map(move |j| (i, j))).collect();
    let at = |&(i, j): &(usize, usize)| {
        pair_stat(
            &cols[i].0,
            &cols[j].0,
            cols[i].1.as_ref(),
            cols[j].1.as_ref(),
            method,
            corr,
        )
    };
    #[cfg(feature = "parallel")]
    let vals: Vec<f64> = if par && n * pairs.len() >= PAR_WORK / 4 {
        pairs.par_iter().map(at).collect()
    } else {
        pairs.iter().map(at).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let vals: Vec<f64> = pairs.iter().map(at).collect();
    for (&(i, j), v) in pairs.iter().zip(vals) {
        let v = if corr && i == j && !v.is_nan() {
            1.0
        } else {
            v
        };
        out[i * k + j] = v;
        out[j * k + i] = v;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(rows: &[&[f64]]) -> Matrix {
        Matrix::from_rows(&rows.iter().map(|r| r.to_vec()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn products_agree_however_stored() {
        let a = m(&[&[1.0, 2.0], &[3.0, 4.0], &[5.0, 6.0]]);
        let b = m(&[&[1.0, 0.0, 2.0], &[0.0, 1.0, 1.0]]);
        let ab = a.matmul(&b, false).unwrap();
        assert_eq!(ab.shape(), (3, 3));
        assert_eq!(ab.row(2), vec![5.0, 6.0, 16.0]);
        // (AB)ᵀ = BᵀAᵀ, through every storage pairing
        let btat = b.t().matmul(&a.t(), false).unwrap();
        assert_eq!(btat, ab.t());
        assert_eq!(a.t().materialize().t(), a);
        let g = a.gram(false);
        assert_eq!(g.to_row_major(), vec![35.0, 44.0, 44.0, 56.0]);
        assert_eq!(
            a.matvec(&[1.0, -1.0], false).unwrap(),
            vec![-1.0, -1.0, -1.0]
        );
        assert!(a.matmul(&a, false).is_err());
    }

    #[test]
    fn parallel_gram_is_the_sequential_one() {
        let n = 200_000;
        let cols: Vec<Arc<Vec<f64>>> = (0..6)
            .map(|j| Arc::new((0..n).map(|i| ((i * (j + 3)) % 97) as f64 * 0.37).collect()))
            .collect();
        let x = Matrix::from_columns(n, cols).unwrap();
        assert_eq!(x.gram(true), x.gram(false));
        let b = [0.5, -1.0, 2.0, 0.0, 1.5, 3.0];
        assert_eq!(x.matvec(&b, true).unwrap(), x.matvec(&b, false).unwrap());
    }

    #[test]
    fn solves_and_factors() {
        let a = m(&[&[4.0, 1.0], &[1.0, 3.0]]);
        let x = solve(&a, &m(&[&[1.0], &[2.0]])).unwrap();
        let back = a.matmul(&x, false).unwrap();
        assert!((back.get(0, 0) - 1.0).abs() < 1e-12 && (back.get(1, 0) - 2.0).abs() < 1e-12);
        let ai = inv(&a).unwrap();
        let id = a.matmul(&ai, false).unwrap();
        assert!((id.get(0, 1)).abs() < 1e-12 && (id.get(1, 1) - 1.0).abs() < 1e-12);
        let l = cholesky(&a).unwrap();
        let llt = l.matmul(&l.t(), false).unwrap();
        assert!((llt.get(0, 1) - 1.0).abs() < 1e-12);
        let (vals, vecs) = eigh(&a).unwrap();
        assert!(vals[0] > vals[1]);
        let av = a.matvec(&vecs.column(0), false).unwrap();
        assert!((av[0] - vals[0] * vecs.get(0, 0)).abs() < 1e-10);
        assert!(solve(&m(&[&[1.0, 2.0], &[2.0, 4.0]]), &m(&[&[1.0], &[1.0]])).is_err());
        assert!(cholesky(&m(&[&[1.0, 2.0], &[0.0, 1.0]])).is_err());
    }

    #[test]
    fn eigh_matches_jacobi() {
        let n = 12;
        let mut a = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..=i {
                let v = ((i * 7 + j * 3) % 11) as f64 - 5.0 + if i == j { 20.0 } else { 0.0 };
                a[i * n + j] = v;
                a[j * n + i] = v;
            }
        }
        let m = Matrix::from_row_major(n, n, &a);
        let (vals, vecs) = eigh(&m).unwrap();
        let (jv, _) = crate::learn::linalg::eigh(&a, n);
        for (x, y) in vals.iter().zip(&jv) {
            assert!((x - y).abs() < 1e-9, "{x} vs {y}");
        }
        // A v = λ v, and the vectors are orthonormal
        for (j, &val) in vals.iter().enumerate().take(n) {
            let col = vecs.column(j);
            let av = m.matvec(&col, false).unwrap();
            for i in 0..n {
                assert!((av[i] - val * col[i]).abs() < 1e-9);
            }
        }
        let vtv = vecs.gram(false);
        for i in 0..n {
            for j in 0..n {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((vtv.get(i, j) - want).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn distances() {
        let x = m(&[&[0.0, 0.0], &[3.0, 4.0]]);
        let c = m(&[&[0.0, 0.0], &[1.0, 1.0], &[3.0, 0.0]]);
        let d = cdist(&x, &c, Metric::Euclidean, false).unwrap();
        assert_eq!(d.shape(), (2, 3));
        assert_eq!(d.row(1), vec![5.0, (13.0f64).sqrt(), 4.0]);
        let d = cdist(&x, &c, Metric::Cityblock, false).unwrap();
        assert_eq!(d.row(1), vec![7.0, 5.0, 4.0]);
    }

    #[test]
    fn correlation_pairwise_and_spearman() {
        let a = Arc::new(vec![1.0, 2.0, 3.0, 4.0]);
        let b = Arc::new(vec![2.0, 4.0, 6.0, 8.5]);
        let c = Arc::new(vec![1.0, 8.0, 27.0, 64.0]);
        let cols = vec![(a.clone(), None), (b, None), (c.clone(), None)];
        let r = corr_cov(&cols, CorrMethod::Spearman, true, false);
        assert_eq!(r[2], 1.0);
        let p = corr_cov(&cols, CorrMethod::Pearson, true, false);
        assert!(p[1] > 0.99 && p[1] < 1.0);
        let cov = corr_cov(&cols[..1], CorrMethod::Pearson, false, false);
        assert!((cov[0] - 5.0 / 3.0).abs() < 1e-12);
        // a null pairs over the rows both have
        let masked = vec![(a, Some(vec![true, true, false, true])), (c, None)];
        let r = corr_cov(&masked, CorrMethod::Spearman, true, false);
        assert_eq!(r[1], 1.0);
    }
}

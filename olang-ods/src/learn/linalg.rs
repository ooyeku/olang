//! Small dense linear algebra: the k×k systems the models solve.
//!
//! Matrices are row-major `Vec<f64>` with their order `n` beside them.
//! k is a column count (tens, at most a few hundred), so textbook
//! Cholesky and cyclic Jacobi are exact enough and fast enough; nothing
//! here is on an O(rows) path.

/// The dot product, four accumulators wide (autovectorizes).
#[inline]
pub fn dot(a: &[f64], b: &[f64]) -> f64 {
    let mut acc = [0.0f64; 4];
    let (ca, ra) = a.as_chunks::<4>();
    let (cb, rb) = b.as_chunks::<4>();
    for (x, y) in ca.iter().zip(cb) {
        for l in 0..4 {
            acc[l] += x[l] * y[l];
        }
    }
    let rem: f64 = ra.iter().zip(rb).map(|(x, y)| x * y).sum();
    (acc[0] + acc[1]) + (acc[2] + acc[3]) + rem
}

/// Cholesky factor L (lower, row-major) of a symmetric positive-definite
/// `n×n` matrix; `None` when it is not positive definite.
pub fn cholesky(a: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut l = vec![0.0f64; n * n];
    for i in 0..n {
        for j in 0..=i {
            let mut s = a[i * n + j];
            for m in 0..j {
                s -= l[i * n + m] * l[j * n + m];
            }
            if i == j {
                if s <= 0.0 || !s.is_finite() {
                    return None;
                }
                l[i * n + i] = s.sqrt();
            } else {
                l[i * n + j] = s / l[j * n + j];
            }
        }
    }
    Some(l)
}

/// Solve (L Lᵀ) x = b.
pub fn chol_solve(l: &[f64], n: usize, b: &[f64]) -> Vec<f64> {
    let mut z = vec![0.0f64; n];
    for i in 0..n {
        let mut s = b[i];
        for m in 0..i {
            s -= l[i * n + m] * z[m];
        }
        z[i] = s / l[i * n + i];
    }
    let mut x = vec![0.0f64; n];
    for i in (0..n).rev() {
        let mut s = z[i];
        for m in i + 1..n {
            s -= l[m * n + i] * x[m];
        }
        x[i] = s / l[i * n + i];
    }
    x
}

/// (L Lᵀ)⁻¹, row-major.
pub fn chol_inverse(l: &[f64], n: usize) -> Vec<f64> {
    let mut inv = vec![0.0f64; n * n];
    let mut e = vec![0.0f64; n];
    for j in 0..n {
        e.iter_mut().for_each(|v| *v = 0.0);
        e[j] = 1.0;
        let col = chol_solve(l, n, &e);
        for i in 0..n {
            inv[i * n + j] = col[i];
        }
    }
    inv
}

/// Factor a symmetric matrix that should be positive definite, adding a
/// growing ridge to the diagonal when rounding (or a flat direction)
/// makes it fail. Returns the factor and the ridge used.
pub fn cholesky_jittered(a: &[f64], n: usize) -> Option<(Vec<f64>, f64)> {
    if let Some(l) = cholesky(a, n) {
        return Some((l, 0.0));
    }
    let scale = (0..n)
        .map(|i| a[i * n + i].abs())
        .fold(0.0f64, f64::max)
        .max(1e-300);
    let mut jitter = scale * 1e-12;
    for _ in 0..12 {
        let mut b = a.to_vec();
        for i in 0..n {
            b[i * n + i] += jitter;
        }
        if let Some(l) = cholesky(&b, n) {
            return Some((l, jitter));
        }
        jitter *= 10.0;
    }
    None
}

/// Symmetric eigen-decomposition by cyclic Jacobi rotations.
/// Returns eigenvalues in descending order and the matching unit
/// eigenvectors as columns of a row-major `n×n` matrix.
pub fn eigh(a: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut m = a.to_vec();
    let mut v = vec![0.0f64; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    let norm: f64 = m.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-300);
    for _sweep in 0..100 {
        let off: f64 = (0..n)
            .flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j)))
            .map(|(i, j)| m[i * n + j] * m[i * n + j])
            .sum::<f64>()
            .sqrt();
        if off <= 1e-15 * norm {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let apq = m[p * n + q];
                if apq.abs() <= 1e-300 {
                    continue;
                }
                let app = m[p * n + p];
                let aqq = m[q * n + q];
                let theta = (aqq - app) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let mkp = m[k * n + p];
                    let mkq = m[k * n + q];
                    m[k * n + p] = c * mkp - s * mkq;
                    m[k * n + q] = s * mkp + c * mkq;
                }
                for k in 0..n {
                    let mpk = m[p * n + k];
                    let mqk = m[q * n + k];
                    m[p * n + k] = c * mpk - s * mqk;
                    m[q * n + k] = s * mpk + c * mqk;
                }
                for k in 0..n {
                    let vkp = v[k * n + p];
                    let vkq = v[k * n + q];
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| m[j * n + j].total_cmp(&m[i * n + i]));
    let vals: Vec<f64> = order.iter().map(|&i| m[i * n + i]).collect();
    let mut vecs = vec![0.0f64; n * n];
    for (new, &old) in order.iter().enumerate() {
        for k in 0..n {
            vecs[k * n + new] = v[k * n + old];
        }
    }
    (vals, vecs)
}

/// Log-determinant from a Cholesky factor.
pub fn chol_logdet(l: &[f64], n: usize) -> f64 {
    (0..n).map(|i| l[i * n + i].ln()).sum::<f64>() * 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cholesky_solves() {
        let a = vec![4.0, 2.0, 0.6, 2.0, 5.0, 1.0, 0.6, 1.0, 3.0];
        let l = cholesky(&a, 3).unwrap();
        let x = chol_solve(&l, 3, &[1.0, 2.0, 3.0]);
        for i in 0..3 {
            let r: f64 = (0..3).map(|j| a[i * 3 + j] * x[j]).sum();
            assert!((r - [1.0, 2.0, 3.0][i]).abs() < 1e-12);
        }
        let inv = chol_inverse(&l, 3);
        for i in 0..3 {
            for j in 0..3 {
                let r: f64 = (0..3).map(|k| a[i * 3 + k] * inv[k * 3 + j]).sum();
                assert!((r - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn eigh_reconstructs() {
        let a = vec![4.0, 1.0, 2.0, 1.0, 3.0, 0.5, 2.0, 0.5, 6.0];
        let (vals, vecs) = eigh(&a, 3);
        assert!(vals[0] >= vals[1] && vals[1] >= vals[2]);
        for c in 0..3 {
            for i in 0..3 {
                let av: f64 = (0..3).map(|k| a[i * 3 + k] * vecs[k * 3 + c]).sum();
                assert!((av - vals[c] * vecs[i * 3 + c]).abs() < 1e-10);
            }
        }
        assert!((vals.iter().sum::<f64>() - 13.0).abs() < 1e-10);
    }
}

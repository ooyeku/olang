//! Small numerical helpers the grammar's stats need: quantiles, the
//! normal and Student t quantile functions, a k×k solve. Pure Rust, no
//! dependency (the browser build carries this module).

/// Quantile of sorted data, R's type 7 (linear between order statistics).
pub fn quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return f64::NAN;
    }
    if n == 1 {
        return sorted[0];
    }
    let h = (n - 1) as f64 * q.clamp(0.0, 1.0);
    let lo = h.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo])
}

pub fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Sample standard deviation (n − 1).
pub fn sd(xs: &[f64]) -> f64 {
    let n = xs.len();
    if n < 2 {
        return f64::NAN;
    }
    let m = mean(xs);
    (xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (n - 1) as f64).sqrt()
}

pub fn sorted_finite(xs: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

/// The standard normal quantile (Acklam's rational approximation with one
/// Halley step: relative error below 1e-15 in double precision).
pub fn qnorm(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383_577_518_672_69e2,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    let pl = 0.02425;
    let x = if p < pl {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - pl {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    // one Halley refinement against the exact cdf
    let e = pnorm(x) - p;
    let u = e * (2.0 * std::f64::consts::PI).sqrt() * (x * x / 2.0).exp();
    x - u / (1.0 + x * u / 2.0)
}

/// The standard normal cdf.
pub fn pnorm(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// erfc to about 1e-15: the Taylor series of erf near zero, a continued
/// fraction (Lentz) in the tails.
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    if z < 2.5 {
        // erf(z) = 2/√π Σ (−1)ⁿ z^(2n+1) / (n! (2n+1))
        let mut term = z;
        let mut sum = z;
        let z2 = z * z;
        let mut n = 0.0;
        while term.abs() > 1e-17 * sum.abs() && n < 200.0 {
            n += 1.0;
            term *= -z2 / n;
            sum += term / (2.0 * n + 1.0);
        }
        let erf = sum * 2.0 / std::f64::consts::PI.sqrt();
        return if x >= 0.0 { 1.0 - erf } else { 1.0 + erf };
    }
    // erfc(z) = e^(−z²)/√π · 1/(z + (1/2)/(z + 1/(z + (3/2)/(z + …))))
    let tiny = 1e-300;
    let mut f = z;
    let mut c = z;
    let mut d = 0.0;
    for k in 1..300 {
        let a = k as f64 / 2.0;
        d = z + a * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = z + a / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() < 1e-16 {
            break;
        }
    }
    let r = (-z * z).exp() / std::f64::consts::PI.sqrt() / f;
    if x >= 0.0 { r } else { 2.0 - r }
}

fn ln_gamma(x: f64) -> f64 {
    // Lanczos approximation (g = 7, n = 9)
    const G: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.5203681218851,
        -1259.1392167224028,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507343278686905,
        -0.13857109526572012,
        9.984_369_578_019_572e-6,
        1.5056327351493116e-7,
    ];
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = G[0];
    let t = x + 7.5;
    for (i, g) in G.iter().enumerate().skip(1) {
        a += g / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Regularized incomplete beta I_x(a, b) (continued fraction, Lentz).
fn inc_beta(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * beta_cf(x, a, b) / a
    } else {
        1.0 - bt * beta_cf(1.0 - x, b, a) / b
    }
}

fn beta_cf(x: f64, a: f64, b: f64) -> f64 {
    let tiny = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < tiny {
        d = tiny;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..300 {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = 1.0 + aa / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = 1.0 + aa / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-14 {
            break;
        }
    }
    h
}

/// Student t cdf with `df` degrees of freedom.
pub fn pt(t: f64, df: f64) -> f64 {
    let x = df / (df + t * t);
    let tail = 0.5 * inc_beta(x, df / 2.0, 0.5);
    if t > 0.0 { 1.0 - tail } else { tail }
}

/// Student t quantile: bisection on the cdf, started from the normal's.
pub fn qt(p: f64, df: f64) -> f64 {
    if !df.is_finite() || df <= 0.0 || df > 1e6 {
        return qnorm(p);
    }
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    let (mut lo, mut hi) = (-1.0, 1.0);
    while pt(lo, df) > p {
        lo *= 2.0;
    }
    while pt(hi, df) < p {
        hi *= 2.0;
    }
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if pt(mid, df) < p {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-12 * (1.0 + mid.abs()) {
            break;
        }
    }
    0.5 * (lo + hi)
}

/// Solve the symmetric positive (semi)definite system `a · x = b` (k×k,
/// row-major) by Gaussian elimination with partial pivoting. None when
/// singular.
pub fn solve(a: &[f64], b: &[f64], k: usize) -> Option<Vec<f64>> {
    let mut m: Vec<f64> = a.to_vec();
    let mut r: Vec<f64> = b.to_vec();
    for col in 0..k {
        let mut piv = col;
        for row in col + 1..k {
            if m[row * k + col].abs() > m[piv * k + col].abs() {
                piv = row;
            }
        }
        if m[piv * k + col].abs() < 1e-12 {
            return None;
        }
        if piv != col {
            for j in 0..k {
                m.swap(col * k + j, piv * k + j);
            }
            r.swap(col, piv);
        }
        let d = m[col * k + col];
        for row in col + 1..k {
            let f = m[row * k + col] / d;
            if f != 0.0 {
                for j in col..k {
                    m[row * k + j] -= f * m[col * k + j];
                }
                r[row] -= f * r[col];
            }
        }
    }
    let mut x = vec![0.0; k];
    for i in (0..k).rev() {
        let mut s = r[i];
        for j in i + 1..k {
            s -= m[i * k + j] * x[j];
        }
        x[i] = s / m[i * k + i];
    }
    Some(x)
}

/// The inverse of a k×k matrix (row-major), None when singular.
/// log det of a symmetric positive-definite `k`×`k` matrix (Cholesky), or
/// None when it is not positive definite.
pub fn logdet_spd(a: &[f64], k: usize) -> Option<f64> {
    let mut l = vec![0.0; k * k];
    let mut out = 0.0;
    for j in 0..k {
        let mut d = a[j * k + j];
        for t in 0..j {
            d -= l[j * k + t] * l[j * k + t];
        }
        if d.is_nan() || d <= 0.0 {
            return None;
        }
        let dj = d.sqrt();
        l[j * k + j] = dj;
        out += 2.0 * dj.ln();
        for i in j + 1..k {
            let mut v = a[i * k + j];
            for t in 0..j {
                v -= l[i * k + t] * l[j * k + t];
            }
            l[i * k + j] = v / dj;
        }
    }
    Some(out)
}

pub fn inverse(a: &[f64], k: usize) -> Option<Vec<f64>> {
    let mut out = vec![0.0; k * k];
    for j in 0..k {
        let mut e = vec![0.0; k];
        e[j] = 1.0;
        let col = solve(a, &e, k)?;
        for i in 0..k {
            out[i * k + j] = col[i];
        }
    }
    Some(out)
}

/// A small deterministic generator (xorshift64*): jitter is reproducible
/// across runs and tiers.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_f64(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64) / ((1u64 << 53) as f64)
    }
    /// Uniform in [-1, 1).
    pub fn sym(&mut self) -> f64 {
        self.next_f64() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_match_r() {
        // R: qnorm(0.975) = 1.959963984540054; qt(0.975, 10) = 2.228138851986274
        assert!((qnorm(0.975) - 1.959963984540054).abs() < 1e-12);
        assert!(qnorm(0.5).abs() < 1e-15);
        assert!((pnorm(-3.0) - 0.0013498980316301).abs() < 1e-15);
        assert!((pnorm(1.0) - 0.8413447460685429).abs() < 1e-14);
        assert!((qnorm(0.001) + 3.090232306167813).abs() < 1e-6);
        assert!((qt(0.975, 10.0) - 2.228138851986274).abs() < 1e-6);
        assert!((qt(0.975, 3.0) - 3.182446305284263).abs() < 1e-6);
        assert!((qt(0.05, 25.0) + 1.708140761251899).abs() < 1e-6);
        // type 7: quantile(1:10, 0.25) = 3.25
        let v: Vec<f64> = (1..=10).map(|i| i as f64).collect();
        assert!((quantile_sorted(&v, 0.25) - 3.25).abs() < 1e-12);
    }

    #[test]
    fn solve_small_system() {
        let a = [4.0, 1.0, 1.0, 3.0];
        let x = solve(&a, &[1.0, 2.0], 2).unwrap();
        assert!((x[0] - 1.0 / 11.0).abs() < 1e-12 && (x[1] - 7.0 / 11.0).abs() < 1e-12);
        assert!(solve(&[1.0, 2.0, 2.0, 4.0], &[1.0, 1.0], 2).is_none());
    }
}

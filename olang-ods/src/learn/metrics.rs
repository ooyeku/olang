//! Evaluation metrics over dense slices (the glue drops nulls first).
//!
//! Definitions follow sklearn.metrics, including its edge conventions:
//! MAPE divides by max(|y|, ε); `roc_curve` drops collinear points and
//! starts at threshold +∞; AUC is the Mann–Whitney statistic with ties
//! counted half; log loss clips probabilities to [ε, 1 − ε].

/// Root mean squared error.
pub fn rmse(y: &[f64], p: &[f64]) -> f64 {
    mse(y, p).sqrt()
}

pub fn mse(y: &[f64], p: &[f64]) -> f64 {
    y.iter().zip(p).map(|(a, b)| (a - b) * (a - b)).sum::<f64>() / y.len() as f64
}

pub fn mae(y: &[f64], p: &[f64]) -> f64 {
    y.iter().zip(p).map(|(a, b)| (a - b).abs()).sum::<f64>() / y.len() as f64
}

/// Mean absolute percentage error as a fraction (sklearn's convention:
/// 0.1 is 10%).
pub fn mape(y: &[f64], p: &[f64]) -> f64 {
    let eps = f64::EPSILON;
    y.iter()
        .zip(p)
        .map(|(a, b)| (a - b).abs() / a.abs().max(eps))
        .sum::<f64>()
        / y.len() as f64
}

pub fn r2(y: &[f64], p: &[f64]) -> f64 {
    let m = y.iter().sum::<f64>() / y.len() as f64;
    let ss_res: f64 = y.iter().zip(p).map(|(a, b)| (a - b) * (a - b)).sum();
    let ss_tot: f64 = y.iter().map(|a| (a - m) * (a - m)).sum();
    if ss_tot == 0.0 {
        if ss_res == 0.0 { 1.0 } else { 0.0 }
    } else {
        1.0 - ss_res / ss_tot
    }
}

pub fn accuracy(y: &[usize], p: &[usize]) -> f64 {
    y.iter().zip(p).filter(|(a, b)| a == b).count() as f64 / y.len() as f64
}

/// `m[t][p]`: rows with true class t predicted as p.
pub fn confusion(y: &[usize], p: &[usize], k: usize) -> Vec<Vec<usize>> {
    let mut m = vec![vec![0usize; k]; k];
    for (a, b) in y.iter().zip(p) {
        m[*a][*b] += 1;
    }
    m
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Average {
    /// The positive class only.
    Binary(usize),
    Macro,
    Micro,
    Weighted,
}

/// Per-class (precision, recall, f1, support).
pub fn per_class(y: &[usize], p: &[usize], k: usize) -> Vec<(f64, f64, f64, usize)> {
    let m = confusion(y, p, k);
    (0..k)
        .map(|c| {
            let tp = m[c][c] as f64;
            let pred: f64 = (0..k).map(|t| m[t][c] as f64).sum();
            let actual: usize = m[c].iter().sum();
            let prec = if pred > 0.0 { tp / pred } else { 0.0 };
            let rec = if actual > 0 { tp / actual as f64 } else { 0.0 };
            let f1 = if prec + rec > 0.0 {
                2.0 * prec * rec / (prec + rec)
            } else {
                0.0
            };
            (prec, rec, f1, actual)
        })
        .collect()
}

/// Precision, recall and F1 under one averaging (sklearn's
/// `precision_recall_fscore_support`, zero_division = 0).
pub fn prf(y: &[usize], p: &[usize], k: usize, avg: Average) -> (f64, f64, f64) {
    let pc = per_class(y, p, k);
    match avg {
        Average::Binary(c) => (pc[c].0, pc[c].1, pc[c].2),
        Average::Macro => {
            let kf = k as f64;
            (
                pc.iter().map(|v| v.0).sum::<f64>() / kf,
                pc.iter().map(|v| v.1).sum::<f64>() / kf,
                pc.iter().map(|v| v.2).sum::<f64>() / kf,
            )
        }
        Average::Weighted => {
            let total: f64 = pc.iter().map(|v| v.3 as f64).sum();
            let w = |f: fn(&(f64, f64, f64, usize)) -> f64| {
                pc.iter().map(|v| f(v) * v.3 as f64).sum::<f64>() / total
            };
            (w(|v| v.0), w(|v| v.1), w(|v| v.2))
        }
        Average::Micro => {
            // Single-label: micro precision = recall = F1 = accuracy.
            let a = accuracy(y, p);
            (a, a, a)
        }
    }
}

/// Binary log loss (y ∈ {0,1}, p = P(1)).
pub fn log_loss(y: &[bool], p: &[f64]) -> f64 {
    let eps = f64::EPSILON;
    y.iter()
        .zip(p)
        .map(|(&t, &q)| {
            let q = q.clamp(eps, 1.0 - eps);
            if t { -q.ln() } else { -(1.0 - q).ln() }
        })
        .sum::<f64>()
        / y.len() as f64
}

/// Multiclass log loss: `probs[c][i]`, each row renormalized as sklearn does.
pub fn log_loss_multi(y: &[usize], probs: &[Vec<f64>]) -> f64 {
    let eps = f64::EPSILON;
    let n = y.len();
    let mut s = 0.0;
    for i in 0..n {
        let row: Vec<f64> = probs.iter().map(|c| c[i].clamp(eps, 1.0 - eps)).collect();
        let total: f64 = row.iter().sum();
        s -= (row[y[i]] / total).ln();
    }
    s / n as f64
}

/// Brier score: mean (p − y)².
pub fn brier(y: &[bool], p: &[f64]) -> f64 {
    y.iter()
        .zip(p)
        .map(|(&t, &q)| {
            let d = q - if t { 1.0 } else { 0.0 };
            d * d
        })
        .sum::<f64>()
        / y.len() as f64
}

/// Area under the ROC curve: the probability a random positive outscores
/// a random negative, ties counted half (rank-sum form, O(n log n)).
pub fn roc_auc(y: &[bool], s: &[f64]) -> Option<f64> {
    let n = y.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_unstable_by(|&a, &b| s[a].total_cmp(&s[b]));
    let mut ranks = vec![0.0; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && s[idx[j + 1]] == s[idx[i]] {
            j += 1;
        }
        let r = (i + j) as f64 / 2.0 + 1.0;
        for &t in &idx[i..=j] {
            ranks[t] = r;
        }
        i = j + 1;
    }
    let np = y.iter().filter(|&&v| v).count() as f64;
    let nn = n as f64 - np;
    if np == 0.0 || nn == 0.0 {
        return None;
    }
    let rsum: f64 = (0..n).filter(|&i| y[i]).map(|i| ranks[i]).sum();
    Some((rsum - np * (np + 1.0) / 2.0) / (np * nn))
}

/// Cumulative (fps, tps, thresholds) at each distinct score, descending.
fn binary_clf_curve(y: &[bool], s: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut idx: Vec<usize> = (0..y.len()).collect();
    idx.sort_by(|&a, &b| s[b].total_cmp(&s[a]));
    let mut fps = Vec::new();
    let mut tps = Vec::new();
    let mut thr = Vec::new();
    let (mut tp, mut fp) = (0.0, 0.0);
    for (pos, &i) in idx.iter().enumerate() {
        if y[i] {
            tp += 1.0;
        } else {
            fp += 1.0;
        }
        let last = pos + 1 == idx.len() || s[idx[pos + 1]] != s[i];
        if last {
            fps.push(fp);
            tps.push(tp);
            thr.push(s[i]);
        }
    }
    (fps, tps, thr)
}

/// sklearn's `roc_curve(drop_intermediate=True)`: (fpr, tpr, thresholds),
/// starting at (0, 0) with threshold +∞.
pub fn roc_curve(y: &[bool], s: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let (fps, tps, thr) = binary_clf_curve(y, s);
    let m = fps.len();
    // Keep the corners: a point where the slope changes.
    let keep: Vec<usize> = (0..m)
        .filter(|&i| {
            if i == 0 || i + 1 == m {
                return true;
            }
            let d2f = fps[i + 1] - 2.0 * fps[i] + fps[i - 1];
            let d2t = tps[i + 1] - 2.0 * tps[i] + tps[i - 1];
            d2f != 0.0 || d2t != 0.0
        })
        .collect();
    let np = tps.last().copied().unwrap_or(0.0);
    let nn = fps.last().copied().unwrap_or(0.0);
    let mut fpr = vec![0.0];
    let mut tpr = vec![0.0];
    let mut th = vec![f64::INFINITY];
    for &i in &keep {
        fpr.push(if nn > 0.0 { fps[i] / nn } else { f64::NAN });
        tpr.push(if np > 0.0 { tps[i] / np } else { f64::NAN });
        th.push(thr[i]);
    }
    (fpr, tpr, th)
}

/// sklearn's `precision_recall_curve`: (precision, recall, thresholds)
/// in increasing threshold order, ending at precision 1, recall 0 (the
/// final point has no threshold).
pub fn pr_curve(y: &[bool], s: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let (fps, tps, thr) = binary_clf_curve(y, s);
    let np = tps.last().copied().unwrap_or(0.0);
    let m = fps.len();
    let mut prec: Vec<f64> = (0..m)
        .map(|i| {
            let ps = tps[i] + fps[i];
            if ps > 0.0 { tps[i] / ps } else { 0.0 }
        })
        .collect();
    let mut rec: Vec<f64> = (0..m)
        .map(|i| if np > 0.0 { tps[i] / np } else { 1.0 })
        .collect();
    let mut th = thr;
    prec.reverse();
    rec.reverse();
    th.reverse();
    prec.push(1.0);
    rec.push(0.0);
    (prec, rec, th)
}

/// Average precision: Σ (Rₙ − Rₙ₋₁)·Pₙ over the PR curve.
pub fn average_precision(y: &[bool], s: &[f64]) -> f64 {
    let (p, r, _) = pr_curve(y, s);
    // Curve is in decreasing recall order after the reverse.
    let mut ap = 0.0;
    for i in 0..p.len() - 1 {
        ap += (r[i] - r[i + 1]) * p[i];
    }
    ap
}

/// Calibration bins: (bin lower edge, upper edge, mean predicted, fraction
/// positive, count) for each non-empty bin. `quantile` puts equal counts
/// in each bin; otherwise bins are equal-width on [0, 1].
pub fn calibration(
    y: &[bool],
    p: &[f64],
    bins: usize,
    quantile: bool,
) -> Vec<(f64, f64, f64, f64, usize)> {
    let bins = bins.max(1);
    let edges: Vec<f64> = if quantile {
        let mut s = p.to_vec();
        s.sort_unstable_by(f64::total_cmp);
        let n = s.len();
        (0..=bins)
            .map(|b| {
                let q = b as f64 / bins as f64 * (n - 1) as f64;
                let lo = q.floor() as usize;
                let hi = (lo + 1).min(n - 1);
                s[lo] + (s[hi] - s[lo]) * (q - lo as f64)
            })
            .collect()
    } else {
        (0..=bins).map(|b| b as f64 / bins as f64).collect()
    };
    // sklearn: bin = searchsorted(edges[1:-1], p) — a value on an edge
    // belongs to the bin below it.
    let inner = &edges[1..bins];
    let mut sum_p = vec![0.0; bins];
    let mut sum_y = vec![0.0; bins];
    let mut cnt = vec![0usize; bins];
    for (t, q) in y.iter().zip(p) {
        let b = inner.partition_point(|e| *e < *q);
        sum_p[b] += q;
        sum_y[b] += if *t { 1.0 } else { 0.0 };
        cnt[b] += 1;
    }
    (0..bins)
        .filter(|&b| cnt[b] > 0)
        .map(|b| {
            (
                edges[b],
                edges[b + 1],
                sum_p[b] / cnt[b] as f64,
                sum_y[b] / cnt[b] as f64,
                cnt[b],
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auc_matches_hand_count() {
        let y = [false, false, true, true];
        let s = [0.1, 0.4, 0.35, 0.8];
        assert_eq!(roc_auc(&y, &s), Some(0.75));
        let (fpr, tpr, th) = roc_curve(&y, &s);
        assert_eq!(fpr, vec![0.0, 0.0, 0.5, 0.5, 1.0]);
        assert_eq!(tpr, vec![0.0, 0.5, 0.5, 1.0, 1.0]);
        assert_eq!(th[1], 0.8);
    }

    #[test]
    fn pr_curve_matches_sklearn_doc_example() {
        // sklearn docs: y=[0,0,1,1], s=[0.1,0.4,0.35,0.8]
        let (p, r, t) = pr_curve(&[false, false, true, true], &[0.1, 0.4, 0.35, 0.8]);
        assert_eq!(t, vec![0.1, 0.35, 0.4, 0.8]);
        let want_p = [0.5, 2.0 / 3.0, 0.5, 1.0, 1.0];
        let want_r = [1.0, 1.0, 0.5, 0.5, 0.0];
        for i in 0..5 {
            assert!((p[i] - want_p[i]).abs() < 1e-12 && (r[i] - want_r[i]).abs() < 1e-12);
        }
        assert!(
            (average_precision(&[false, false, true, true], &[0.1, 0.4, 0.35, 0.8])
                - 0.8333333333333333)
                .abs()
                < 1e-12
        );
    }

    #[test]
    fn prf_macro() {
        let y = [0, 1, 2, 0, 1, 2];
        let p = [0, 2, 1, 0, 0, 1];
        let (pr, rc, f1) = prf(&y, &p, 3, Average::Macro);
        assert!((pr - 0.2222222222222222).abs() < 1e-12);
        assert!((rc - 0.3333333333333333).abs() < 1e-12);
        assert!((f1 - 0.26666666666666666).abs() < 1e-12);
    }
}

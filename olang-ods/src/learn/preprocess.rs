//! Splitting and preprocessing at the index level: seeded (optionally
//! stratified) train/test splits, k-fold and stratified k-fold ids,
//! rolling-origin time splits, scaler and imputer statistics, and the
//! polynomial feature expansion.

use super::rng::Pcg;

/// numpy's default ("linear") quantile of sorted data.
pub fn quantile_sorted(s: &[f64], q: f64) -> f64 {
    if s.is_empty() {
        return f64::NAN;
    }
    let pos = q * (s.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(s.len() - 1);
    s[lo] + (s[hi] - s[lo]) * (pos - lo as f64)
}

/// Split `n` rows: `n_test` of them to the test side, the rest train,
/// each side in the rows' original order. With `strata` the test rows
/// are allocated to each class in proportion (largest remainder), so
/// both sides keep the class balance.
pub fn split(n: usize, n_test: usize, shuffle: bool, strata: Option<&[usize]>, rng: &mut Pcg) -> (Vec<usize>, Vec<usize>) {
    let n_test = n_test.min(n);
    let mut is_test = vec![false; n];
    match strata {
        None => {
            if shuffle {
                for i in rng.choose(n, n_test) {
                    is_test[i] = true;
                }
            } else {
                for t in is_test.iter_mut().skip(n - n_test) {
                    *t = true;
                }
            }
        }
        Some(cls) => {
            let k = cls.iter().copied().max().map(|m| m + 1).unwrap_or(0);
            let mut groups: Vec<Vec<usize>> = vec![Vec::new(); k];
            for (i, &c) in cls.iter().enumerate() {
                groups[c].push(i);
            }
            // Largest-remainder allocation of n_test over the classes.
            let exact: Vec<f64> = groups.iter().map(|g| g.len() as f64 * n_test as f64 / n as f64).collect();
            let mut alloc: Vec<usize> = exact.iter().map(|e| e.floor() as usize).collect();
            let mut left = n_test - alloc.iter().sum::<usize>();
            let mut order: Vec<usize> = (0..k).collect();
            order.sort_by(|&a, &b| (exact[b] - exact[b].floor()).total_cmp(&(exact[a] - exact[a].floor())).then(a.cmp(&b)));
            for &c in &order {
                if left == 0 {
                    break;
                }
                if alloc[c] < groups[c].len() {
                    alloc[c] += 1;
                    left -= 1;
                }
            }
            for (c, g) in groups.iter().enumerate() {
                if shuffle {
                    for p in rng.choose(g.len(), alloc[c]) {
                        is_test[g[p]] = true;
                    }
                } else {
                    for &i in &g[g.len() - alloc[c]..] {
                        is_test[i] = true;
                    }
                }
            }
        }
    }
    let train = (0..n).filter(|&i| !is_test[i]).collect();
    let test = (0..n).filter(|&i| is_test[i]).collect();
    (train, test)
}

/// sklearn's `KFold`: fold ids 0..k, the first n % k folds one larger,
/// over a shuffled order when `shuffle`.
pub fn kfold(n: usize, k: usize, shuffle: bool, rng: &mut Pcg) -> Vec<usize> {
    let order: Vec<usize> = if shuffle { rng.permutation(n) } else { (0..n).collect() };
    let mut ids = vec![0usize; n];
    let base = n / k;
    let extra = n % k;
    let mut pos = 0;
    for f in 0..k {
        let size = base + usize::from(f < extra);
        for &i in &order[pos..pos + size] {
            ids[i] = f;
        }
        pos += size;
    }
    ids
}

/// sklearn's `StratifiedKFold` allocation: each class's rows spread over
/// the folds as evenly as the class size allows, the folds' sizes as
/// even as the data allows.
pub fn stratified_kfold(cls: &[usize], k: usize, shuffle: bool, rng: &mut Pcg) -> Vec<usize> {
    let n = cls.len();
    let nc = cls.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    // y_order = classes sorted; allocation[f][c] = count of class c in
    // y_order[f::k].
    let mut sorted: Vec<usize> = cls.to_vec();
    sorted.sort_unstable();
    let mut alloc = vec![vec![0usize; nc]; k];
    for (i, &c) in sorted.iter().enumerate() {
        alloc[i % k][c] += 1;
    }
    let mut ids = vec![0usize; n];
    for c in 0..nc {
        let mut members: Vec<usize> = (0..n).filter(|&i| cls[i] == c).collect();
        if shuffle {
            rng.shuffle(&mut members);
        }
        let mut pos = 0;
        for (f, a) in alloc.iter().enumerate() {
            for &i in &members[pos..pos + a[c]] {
                ids[i] = f;
            }
            pos += a[c];
        }
    }
    ids
}

/// Rolling-origin splits (sklearn's `TimeSeriesSplit`): `n_splits`
/// consecutive test windows of `test_size` rows at the end of the
/// series; each train window is everything before its test window less
/// `gap` rows, capped at `max_train` rows when given. Ranges are
/// half-open (start, end).
pub fn time_splits(
    n: usize,
    n_splits: usize,
    test_size: Option<usize>,
    gap: usize,
    max_train: Option<usize>,
) -> Result<Vec<((usize, usize), (usize, usize))>, String> {
    let test_size = test_size.unwrap_or(n / (n_splits + 1));
    if test_size == 0 || n_splits == 0 || n_splits * test_size + gap >= n {
        return Err(format!(
            "time_splits: {} rows cannot hold {} test windows of {} rows (gap {}) after a training window",
            n, n_splits, test_size, gap
        ));
    }
    let first = n - n_splits * test_size;
    Ok((0..n_splits)
        .map(|s| {
            let ts = first + s * test_size;
            let train_end = ts - gap;
            let train_start = max_train.map(|m| train_end.saturating_sub(m)).unwrap_or(0);
            ((train_start, train_end), (ts, ts + test_size))
        })
        .collect())
}

/// Polynomial terms of `k` inputs up to `degree`: each term is a list of
/// input indices (a multiset). Order matches sklearn's
/// `PolynomialFeatures` (by degree, then combinations with replacement).
pub fn poly_terms(k: usize, degree: usize, interaction_only: bool) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    fn rec(start: usize, k: usize, left: usize, inter: bool, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if left == 0 {
            out.push(cur.clone());
            return;
        }
        for j in start..k {
            cur.push(j);
            rec(if inter { j + 1 } else { j }, k, left - 1, inter, cur, out);
            cur.pop();
        }
    }
    for d in 1..=degree {
        rec(0, k, d, interaction_only, &mut Vec::new(), &mut out);
    }
    out
}

/// A term's name: `a`, `a^2`, `a*b`, `a^2*b`.
pub fn poly_name(term: &[usize], names: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < term.len() {
        let mut j = i;
        while j + 1 < term.len() && term[j + 1] == term[i] {
            j += 1;
        }
        let pw = j - i + 1;
        parts.push(if pw == 1 { names[term[i]].clone() } else { format!("{}^{}", names[term[i]], pw) });
        i = j + 1;
    }
    parts.join("*")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kfold_sizes() {
        let ids = kfold(10, 3, false, &mut Pcg::new(1, 0));
        assert_eq!(ids, vec![0, 0, 0, 0, 1, 1, 1, 2, 2, 2]);
    }

    #[test]
    fn stratified_split_keeps_balance() {
        let cls: Vec<usize> = (0..100).map(|i| usize::from(i < 20)).collect();
        let (tr, te) = split(100, 25, true, Some(&cls), &mut Pcg::new(3, 0));
        assert_eq!(te.len(), 25);
        assert_eq!(tr.len(), 75);
        assert_eq!(te.iter().filter(|&&i| cls[i] == 1).count(), 5);
    }

    #[test]
    fn stratified_kfold_spreads_each_class() {
        let cls: Vec<usize> = (0..30).map(|i| i % 3).collect();
        let ids = stratified_kfold(&cls, 5, true, &mut Pcg::new(9, 0));
        for f in 0..5 {
            for c in 0..3 {
                assert_eq!((0..30).filter(|&i| ids[i] == f && cls[i] == c).count(), 2);
            }
        }
    }

    #[test]
    fn time_splits_match_sklearn() {
        // TimeSeriesSplit(n_splits=3) on 10 rows: test sizes 2, starts 4, 6, 8.
        let s = time_splits(10, 3, None, 0, None).unwrap();
        assert_eq!(s, vec![((0, 4), (4, 6)), ((0, 6), (6, 8)), ((0, 8), (8, 10))]);
    }

    #[test]
    fn poly_order() {
        let names = vec!["a".to_string(), "b".to_string()];
        let t: Vec<String> = poly_terms(2, 2, false).iter().map(|t| poly_name(t, &names)).collect();
        assert_eq!(t, vec!["a", "b", "a^2", "a*b", "b^2"]);
        let t: Vec<String> = poly_terms(2, 3, true).iter().map(|t| poly_name(t, &names)).collect();
        assert_eq!(t, vec!["a", "b", "a*b"]);
    }
}

//! CART decision trees: exact best splits over sorted feature values,
//! the splitter sklearn's `DecisionTree*` uses.
//!
//! A tree is stored flat (one entry per node, children by index), so a
//! forest of them is a few vectors and prediction is a tight loop. Leaf
//! values are class probabilities (classification) or the mean
//! (regression). Sample weights carry a bootstrap's counts, exactly as
//! sklearn's forests pass them, so a forest's trees are ordinary trees.

use super::data::Matrix;
use super::rng::Pcg;

pub const LEAF: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Criterion {
    Gini,
    Entropy,
    Mse,
}

#[derive(Clone, Debug)]
pub struct TreeParams {
    pub criterion: Criterion,
    pub max_depth: Option<usize>,
    pub min_samples_split: usize,
    pub min_samples_leaf: usize,
    /// Features considered per split (≤ k).
    pub max_features: usize,
    pub min_impurity_decrease: f64,
}

/// A fitted tree, flat.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub feature: Vec<u32>,
    pub threshold: Vec<f64>,
    pub left: Vec<u32>,
    pub right: Vec<u32>,
    /// `n_out` values per node: class probabilities, or one mean.
    pub value: Vec<f64>,
    pub n_out: usize,
    pub impurity: Vec<f64>,
    pub weighted_n: Vec<f64>,
    pub n_samples: Vec<u32>,
    /// Weighted impurity decrease per feature (unnormalized).
    pub importance: Vec<f64>,
}

impl Tree {
    pub fn n_nodes(&self) -> usize {
        self.feature.len()
    }

    pub fn depth(&self) -> usize {
        fn go(t: &Tree, i: usize) -> usize {
            if t.feature[i] == LEAF {
                0
            } else {
                1 + go(t, t.left[i] as usize).max(go(t, t.right[i] as usize))
            }
        }
        if self.n_nodes() == 0 { 0 } else { go(self, 0) }
    }

    pub fn n_leaves(&self) -> usize {
        self.feature.iter().filter(|&&f| f == LEAF).count()
    }

    /// The leaf a row lands in.
    #[inline]
    pub fn leaf_of(&self, x: &Matrix, i: usize) -> usize {
        let mut node = 0usize;
        loop {
            let f = self.feature[node];
            if f == LEAF {
                return node;
            }
            node = if x.cols[f as usize][i] <= self.threshold[node] {
                self.left[node] as usize
            } else {
                self.right[node] as usize
            };
        }
    }

    #[inline]
    pub fn leaf_values(&self, leaf: usize) -> &[f64] {
        &self.value[leaf * self.n_out..(leaf + 1) * self.n_out]
    }

    /// Add `scale ×` this tree's leaf values for each row into `out`
    /// (`out[c][i]`).
    pub fn accumulate(&self, x: &Matrix, out: &mut [Vec<f64>], scale: f64) {
        for i in 0..x.n {
            let leaf = self.leaf_of(x, i);
            for (c, o) in out.iter_mut().enumerate() {
                o[i] += scale * self.value[leaf * self.n_out + c];
            }
        }
    }
}

/// What a tree learns from.
pub enum Target<'a> {
    Classes { y: &'a [usize], n_classes: usize },
    Values(&'a [f64]),
}

impl Target<'_> {
    fn n_out(&self) -> usize {
        match self {
            Target::Classes { n_classes, .. } => *n_classes,
            Target::Values(_) => 1,
        }
    }
}

/// Node statistics: weighted class counts, or (Σw, Σwy, Σwy²).
#[derive(Clone)]
struct Stats {
    w: f64,
    counts: Vec<f64>,
    sy: f64,
    syy: f64,
}

impl Stats {
    fn new(n_out: usize) -> Stats {
        Stats {
            w: 0.0,
            counts: vec![0.0; n_out],
            sy: 0.0,
            syy: 0.0,
        }
    }

    fn clear(&mut self) {
        self.w = 0.0;
        self.counts.iter_mut().for_each(|c| *c = 0.0);
        self.sy = 0.0;
        self.syy = 0.0;
    }

    #[inline]
    fn add(&mut self, target: &Target, i: usize, w: f64) {
        self.w += w;
        match target {
            Target::Classes { y, .. } => self.counts[y[i]] += w,
            Target::Values(y) => {
                self.sy += w * y[i];
                self.syy += w * y[i] * y[i];
            }
        }
    }

    fn impurity(&self, crit: Criterion) -> f64 {
        if self.w <= 0.0 {
            return 0.0;
        }
        match crit {
            Criterion::Gini => {
                1.0 - self
                    .counts
                    .iter()
                    .map(|c| (c / self.w) * (c / self.w))
                    .sum::<f64>()
            }
            Criterion::Entropy => -self
                .counts
                .iter()
                .filter(|&&c| c > 0.0)
                .map(|c| {
                    let p = c / self.w;
                    p * p.log2()
                })
                .sum::<f64>(),
            Criterion::Mse => {
                let m = self.sy / self.w;
                (self.syy / self.w - m * m).max(0.0)
            }
        }
    }

    fn value(&self, out: &mut Vec<f64>, regression: bool) {
        if regression {
            out.push(if self.w > 0.0 { self.sy / self.w } else { 0.0 });
        } else {
            for c in &self.counts {
                out.push(if self.w > 0.0 { c / self.w } else { 0.0 });
            }
        }
    }
}

/// sklearn's FEATURE_THRESHOLD: values closer than this are one value.
const FEATURE_THRESHOLD: f64 = 1e-7;

/// Grow a tree on the rows `rows` (each with weight `w[row]`).
pub fn grow(
    x: &Matrix,
    target: &Target,
    w: &[f64],
    rows: Vec<usize>,
    params: &TreeParams,
    rng: &mut Pcg,
) -> Tree {
    let k = x.k();
    let n_out = target.n_out();
    let regression = matches!(target, Target::Values(_));
    let mut tree = Tree {
        n_out,
        importance: vec![0.0; k],
        ..Tree::default()
    };
    let mut rows = rows;
    let total_w: f64 = rows.iter().map(|&i| w[i]).sum();
    // (start, end, depth, parent, is_left)
    let mut stack: Vec<(usize, usize, usize, usize, bool)> =
        vec![(0, rows.len(), 0, usize::MAX, false)];
    let mut node_stats = Stats::new(n_out);
    let mut left = Stats::new(n_out);
    let mut right = Stats::new(n_out);
    let mut scratch: Vec<(f64, usize)> = Vec::with_capacity(rows.len());
    let mut features: Vec<usize> = (0..k).collect();
    // A 0/1 column (a one-hot level, a Bool) has one threshold, 0.5: its
    // split is found by one pass over the node's rows, with no sort.
    let binary: Vec<bool> = x
        .cols
        .iter()
        .map(|c| c.iter().all(|&v| v == 0.0 || v == 1.0))
        .collect();
    while let Some((start, end, depth, parent, is_left)) = stack.pop() {
        let id = tree.feature.len();
        if parent != usize::MAX {
            if is_left {
                tree.left[parent] = id as u32;
            } else {
                tree.right[parent] = id as u32;
            }
        }
        node_stats.clear();
        for &i in &rows[start..end] {
            node_stats.add(target, i, w[i]);
        }
        let impurity = node_stats.impurity(params.criterion);
        tree.feature.push(LEAF);
        tree.threshold.push(f64::NAN);
        tree.left.push(LEAF);
        tree.right.push(LEAF);
        tree.impurity.push(impurity);
        tree.weighted_n.push(node_stats.w);
        tree.n_samples.push((end - start) as u32);
        node_stats.value(&mut tree.value, regression);

        let n_node = end - start;
        let stop = params.max_depth.is_some_and(|d| depth >= d)
            || n_node < params.min_samples_split
            || n_node < 2 * params.min_samples_leaf
            || impurity <= 1e-12;
        if stop {
            continue;
        }

        // Best split over a random subset of features (all of them in
        // random order when max_features = k). As in sklearn, features
        // found constant here do not count toward max_features.
        let mut best: Option<(f64, usize, f64, usize)> = None; // (proxy, feature, threshold, n_left)
        let mut visited = 0usize;
        let mut drawn = 0usize;
        while drawn < k && visited < params.max_features {
            let j = drawn + rng.below(k - drawn);
            features.swap(drawn, j);
            let f = features[drawn];
            drawn += 1;
            let col = &x.cols[f];
            if binary[f] {
                left.clear();
                let mut nl = 0usize;
                for &i in &rows[start..end] {
                    if col[i] == 0.0 {
                        left.add(target, i, w[i]);
                        nl += 1;
                    }
                }
                if nl == 0 || nl == n_node {
                    continue;
                }
                visited += 1;
                if nl < params.min_samples_leaf || n_node - nl < params.min_samples_leaf {
                    continue;
                }
                right.clone_from(&node_stats);
                right.w -= left.w;
                for (r, l) in right.counts.iter_mut().zip(&left.counts) {
                    *r -= l;
                }
                right.sy -= left.sy;
                right.syy -= left.syy;
                if left.w <= 0.0 || right.w <= 0.0 {
                    continue;
                }
                let proxy = proxy_gain(&left, &right, params.criterion);
                if best.is_none_or(|b| proxy > b.0) {
                    best = Some((proxy, f, 0.5, nl));
                }
                continue;
            }
            scratch.clear();
            scratch.extend(rows[start..end].iter().map(|&i| (col[i], i)));
            scratch.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            if scratch[n_node - 1].0 <= scratch[0].0 + FEATURE_THRESHOLD {
                continue;
            }
            visited += 1;
            left.clear();
            right.clone_from(&node_stats);
            for p in 0..n_node - 1 {
                let (v, i) = scratch[p];
                left.add(target, i, w[i]);
                // right = node − left
                right.w -= w[i];
                match target {
                    Target::Classes { y, .. } => right.counts[y[i]] -= w[i],
                    Target::Values(y) => {
                        right.sy -= w[i] * y[i];
                        right.syy -= w[i] * y[i] * y[i];
                    }
                }
                let next = scratch[p + 1].0;
                if next <= v + FEATURE_THRESHOLD {
                    continue;
                }
                let nl = p + 1;
                if nl < params.min_samples_leaf || n_node - nl < params.min_samples_leaf {
                    continue;
                }
                if left.w <= 0.0 || right.w <= 0.0 {
                    continue;
                }
                let proxy = proxy_gain(&left, &right, params.criterion);
                if best.is_none_or(|b| proxy > b.0) {
                    let mut thr = v / 2.0 + next / 2.0;
                    if thr == next || !thr.is_finite() {
                        thr = v;
                    }
                    best = Some((proxy, f, thr, nl));
                }
            }
        }
        let Some((_, f, thr, _)) = best else {
            continue;
        };
        // Partition rows[start..end] by the split.
        let col = &x.cols[f];
        let seg = &mut rows[start..end];
        let mut lo = 0usize;
        for p in 0..seg.len() {
            if col[seg[p]] <= thr {
                seg.swap(lo, p);
                lo += 1;
            }
        }
        let mid = start + lo;
        left.clear();
        right.clear();
        for &i in &rows[start..mid] {
            left.add(target, i, w[i]);
        }
        for &i in &rows[mid..end] {
            right.add(target, i, w[i]);
        }
        let il = left.impurity(params.criterion);
        let ir = right.impurity(params.criterion);
        let decrease = node_stats.w * impurity - left.w * il - right.w * ir;
        if decrease / total_w < params.min_impurity_decrease {
            continue;
        }
        tree.feature[id] = f as u32;
        tree.threshold[id] = thr;
        tree.importance[f] += decrease;
        // Right pushed first so the left subtree is built (and numbered)
        // first, as sklearn's depth-first builder does.
        stack.push((mid, end, depth + 1, id, false));
        stack.push((start, mid, depth + 1, id, true));
    }
    tree
}

#[inline]
fn proxy_gain(l: &Stats, r: &Stats, crit: Criterion) -> f64 {
    match crit {
        Criterion::Gini => {
            let sl: f64 = l.counts.iter().map(|c| c * c).sum();
            let sr: f64 = r.counts.iter().map(|c| c * c).sum();
            sl / l.w + sr / r.w
        }
        Criterion::Entropy => -(l.w * l_impurity(l, crit) + r.w * l_impurity(r, crit)),
        Criterion::Mse => l.sy * l.sy / l.w + r.sy * r.sy / r.w,
    }
}

#[inline]
fn l_impurity(s: &Stats, crit: Criterion) -> f64 {
    s.impurity(crit)
}

/// Normalized importances (sum 1) from a set of trees' raw decreases,
/// sklearn's `feature_importances_`: each tree normalized, then averaged.
pub fn importances(trees: &[&Tree], k: usize) -> Vec<f64> {
    let mut out = vec![0.0; k];
    let mut used = 0usize;
    for t in trees {
        let total: f64 = t.importance.iter().sum();
        if total > 0.0 {
            for (o, v) in out.iter_mut().zip(&t.importance) {
                *o += v / total;
            }
            used += 1;
        }
    }
    if used > 0 {
        for o in out.iter_mut() {
            *o /= used as f64;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(crit: Criterion) -> TreeParams {
        TreeParams {
            criterion: crit,
            max_depth: None,
            min_samples_split: 2,
            min_samples_leaf: 1,
            max_features: 2,
            min_impurity_decrease: 0.0,
        }
    }

    #[test]
    fn learns_xor_exactly() {
        let a = vec![0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0];
        let b = vec![0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let y = vec![0usize, 1, 1, 0, 0, 1, 1, 0];
        let x = Matrix::new(8, vec![a, b]);
        let t = grow(
            &x,
            &Target::Classes {
                y: &y,
                n_classes: 2,
            },
            &[1.0; 8],
            (0..8).collect(),
            &params(Criterion::Gini),
            &mut Pcg::new(1, 0),
        );
        for i in 0..8 {
            let leaf = t.leaf_of(&x, i);
            assert_eq!(t.leaf_values(leaf)[y[i]], 1.0);
        }
        assert_eq!(t.depth(), 2);
    }

    #[test]
    fn regression_tree_fits_steps() {
        let a: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let y: Vec<f64> = a
            .iter()
            .map(|v| {
                if *v < 30.0 {
                    1.0
                } else if *v < 70.0 {
                    5.0
                } else {
                    -2.0
                }
            })
            .collect();
        let x = Matrix::new(100, vec![a]);
        let mut p = params(Criterion::Mse);
        p.max_features = 1;
        p.max_depth = Some(2);
        let t = grow(
            &x,
            &Target::Values(&y),
            &[1.0; 100],
            (0..100).collect(),
            &p,
            &mut Pcg::new(1, 0),
        );
        for i in 0..100 {
            assert_eq!(t.leaf_values(t.leaf_of(&x, i))[0], y[i]);
        }
        assert!(t.threshold[0] == 29.5 || t.threshold[0] == 69.5);
    }
}

//! Random forests: bootstrap-weighted CART trees with per-split feature
//! subsampling, grown in parallel.
//!
//! Tree `t` draws everything — its bootstrap and its feature choices —
//! from stream `t` of the forest's seed, so the forest is the same on
//! one thread or eighteen, in any order. The out-of-bag score averages
//! each row's predictions from the trees that did not draw it.

use super::data::Matrix;
use super::rng::Pcg;
use super::tree::{Target, Tree, TreeParams, grow};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

#[derive(Clone, Debug)]
pub struct Forest {
    pub trees: Vec<Tree>,
    /// Out-of-bag predictions (`oob[c][i]`; NaN where a row was in every
    /// bootstrap), when asked for.
    pub oob: Option<Vec<Vec<f64>>>,
    /// Accuracy (classification) or R² (regression) of the OOB
    /// predictions over the rows some tree left out.
    pub oob_score: Option<f64>,
}

pub struct ForestParams {
    pub n_trees: usize,
    pub bootstrap: bool,
    /// Rows per bootstrap (≤ n); `None` = n.
    pub max_samples: Option<usize>,
    pub oob: bool,
    pub tree: TreeParams,
    pub seed: u64,
}

fn one_tree(x: &Matrix, target: &Target, base_w: &[f64], p: &ForestParams, t: usize) -> (Tree, Vec<f64>) {
    let n = x.n;
    let mut rng = Pcg::child(p.seed, t as u64);
    let mut counts = vec![0.0f64; n];
    if p.bootstrap {
        let draws = p.max_samples.unwrap_or(n).min(n);
        for _ in 0..draws {
            counts[rng.below(n)] += 1.0;
        }
    } else {
        counts.iter_mut().for_each(|c| *c = 1.0);
    }
    let w: Vec<f64> = counts.iter().zip(base_w).map(|(c, b)| c * b).collect();
    let rows: Vec<usize> = (0..n).filter(|&i| counts[i] > 0.0).collect();
    let tree = grow(x, target, &w, rows, &p.tree, &mut rng);
    (tree, counts)
}

pub fn fit(x: &Matrix, target: &Target, base_w: &[f64], p: &ForestParams, par: bool) -> Forest {
    let build = |t: usize| one_tree(x, target, base_w, p, t);
    let grown: Vec<(Tree, Vec<f64>)> = {
        #[cfg(feature = "parallel")]
        {
            if par {
                (0..p.n_trees).into_par_iter().map(build).collect()
            } else {
                (0..p.n_trees).map(build).collect()
            }
        }
        #[cfg(not(feature = "parallel"))]
        {
            let _ = par;
            (0..p.n_trees).map(build).collect()
        }
    };
    let oob = if p.oob && p.bootstrap {
        let n_out = grown.first().map(|g| g.0.n_out).unwrap_or(1);
        let mut acc = vec![vec![0.0f64; x.n]; n_out];
        let mut hits = vec![0usize; x.n];
        for (tree, counts) in &grown {
            for i in 0..x.n {
                if counts[i] == 0.0 {
                    let leaf = tree.leaf_of(x, i);
                    for (c, a) in acc.iter_mut().enumerate() {
                        a[i] += tree.value[leaf * tree.n_out + c];
                    }
                    hits[i] += 1;
                }
            }
        }
        for a in acc.iter_mut() {
            for (v, h) in a.iter_mut().zip(&hits) {
                *v = if *h > 0 { *v / *h as f64 } else { f64::NAN };
            }
        }
        Some(acc)
    } else {
        None
    };
    let oob_score = oob.as_ref().map(|acc| {
        let rows: Vec<usize> = (0..x.n).filter(|&i| !acc[0][i].is_nan()).collect();
        match target {
            Target::Classes { y, .. } => {
                let hits = rows
                    .iter()
                    .filter(|&&i| {
                        let best = (0..acc.len()).max_by(|&a, &b| acc[a][i].total_cmp(&acc[b][i]).then(b.cmp(&a))).unwrap();
                        best == y[i]
                    })
                    .count();
                hits as f64 / rows.len().max(1) as f64
            }
            Target::Values(y) => {
                let yy: Vec<f64> = rows.iter().map(|&i| y[i]).collect();
                let pp: Vec<f64> = rows.iter().map(|&i| acc[0][i]).collect();
                super::metrics::r2(&yy, &pp)
            }
        }
    });
    Forest {
        trees: grown.into_iter().map(|g| g.0).collect(),
        oob,
        oob_score,
    }
}

impl Forest {
    /// Mean of the trees' leaf values per row: `out[c][i]`.
    pub fn predict(&self, x: &Matrix, par: bool) -> Vec<Vec<f64>> {
        let n_out = self.trees.first().map(|t| t.n_out).unwrap_or(1);
        let nt = self.trees.len().max(1) as f64;
        let rows = |range: std::ops::Range<usize>| -> Vec<Vec<f64>> {
            let mut out = vec![vec![0.0; range.len()]; n_out];
            for (r, i) in range.enumerate() {
                for t in &self.trees {
                    let leaf = t.leaf_of(x, i);
                    for c in 0..n_out {
                        out[c][r] += t.value[leaf * n_out + c];
                    }
                }
            }
            for o in out.iter_mut() {
                o.iter_mut().for_each(|v| *v /= nt);
            }
            out
        };
        const BLOCK: usize = 2048;
        let blocks: Vec<std::ops::Range<usize>> = (0..x.n.div_ceil(BLOCK))
            .map(|b| b * BLOCK..((b + 1) * BLOCK).min(x.n))
            .collect();
        let parts: Vec<Vec<Vec<f64>>> = {
            #[cfg(feature = "parallel")]
            {
                if par {
                    blocks.into_par_iter().map(rows).collect()
                } else {
                    blocks.into_iter().map(rows).collect()
                }
            }
            #[cfg(not(feature = "parallel"))]
            {
                let _ = par;
                blocks.into_iter().map(rows).collect()
            }
        };
        let mut out = vec![Vec::with_capacity(x.n); n_out];
        for part in parts {
            for (o, p) in out.iter_mut().zip(part) {
                o.extend(p);
            }
        }
        out
    }
}

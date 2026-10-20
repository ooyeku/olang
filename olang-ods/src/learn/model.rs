//! Models over Frames: fit from named columns, predict on a Frame,
//! explain with named terms.
//!
//! This is the layer the language binds. A [`Model`] owns its encoding
//! ([`Design`]), its target's name and classes, and the fitted body;
//! `predict` encodes a new Frame the training way and answers a Series
//! (values, class labels, a positive-class probability) or a Frame
//! (class probabilities, intervals, components). Rows with a null in a
//! used column predict null.

use super::boost::{self, BoostParams, Booster, HistParams, Loss};
use super::cluster::{self, Gmm, GmmParams, KMeans, KMeansParams, Pca};
use super::data::{Design, Input, Labels, Matrix, NO_CLASS, Result, bad, is_valid, kept, numeric};
use super::forest::{self, Forest, ForestParams};
use super::linear::{self, LogitFit, OlsFit, SoftmaxFit};
use super::metrics;
use super::neighbors::{GaussianNb, Knn, gaussian_nb};
use super::preprocess;
use super::rng::Pcg;
use super::tree::{self, Criterion, Target, Tree, TreeParams};
use crate::{Frame, Scalar, Series, dist};

// ---------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskHint {
    Auto,
    Regression,
    Classification,
}

#[derive(Clone, Debug)]
pub enum ClassWeight {
    None,
    Balanced,
    /// Weight per class label (as text).
    Given(Vec<(String, f64)>),
}

/// What every supervised fit names.
#[derive(Clone, Debug)]
pub struct Common {
    pub y: String,
    /// Inputs; empty means every other column.
    pub x: Vec<String>,
    /// A column of row weights.
    pub weights: Option<String>,
    pub task: TaskHint,
    pub class_weight: ClassWeight,
}

#[derive(Clone, Copy, Debug)]
pub enum MaxFeatures {
    /// sqrt for classification forests, all otherwise (sklearn's defaults).
    Default,
    All,
    Sqrt,
    Log2,
    Count(usize),
    Fraction(f64),
}

impl MaxFeatures {
    fn resolve(self, k: usize, forest_classifier: bool) -> usize {
        let v = match self {
            MaxFeatures::Default => {
                if forest_classifier {
                    (k as f64).sqrt().floor() as usize
                } else {
                    k
                }
            }
            MaxFeatures::All => k,
            MaxFeatures::Sqrt => (k as f64).sqrt().floor() as usize,
            MaxFeatures::Log2 => (k as f64).log2().floor() as usize,
            MaxFeatures::Count(c) => c,
            MaxFeatures::Fraction(f) => (f * k as f64).floor() as usize,
        };
        v.clamp(1, k.max(1))
    }
}

#[derive(Clone, Debug)]
pub struct TreeOpts {
    /// None: gini for classification, squared error for regression.
    pub criterion: Option<Criterion>,
    pub max_depth: Option<usize>,
    pub min_samples_split: usize,
    pub min_samples_leaf: usize,
    pub max_features: MaxFeatures,
    pub min_impurity_decrease: f64,
}

impl Default for TreeOpts {
    fn default() -> Self {
        TreeOpts {
            criterion: None,
            max_depth: None,
            min_samples_split: 2,
            min_samples_leaf: 1,
            max_features: MaxFeatures::Default,
            min_impurity_decrease: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Algo {
    Linear {
        intercept: bool,
    },
    Ridge {
        alpha: f64,
        intercept: bool,
    },
    ElasticNet {
        alpha: f64,
        l1_ratio: f64,
        intercept: bool,
        max_iter: usize,
        tol: f64,
    },
    Logistic {
        /// L2 strength on the coefficients (1/C); 0 = unpenalized.
        lambda: f64,
        max_iter: usize,
        tol: f64,
    },
    Tree {
        opts: TreeOpts,
        seed: u64,
    },
    Forest {
        opts: TreeOpts,
        n_trees: usize,
        bootstrap: bool,
        max_samples: Option<f64>,
        oob: bool,
        seed: u64,
    },
    Boost {
        params: BoostParams,
    },
    Knn {
        k: usize,
        distance: bool,
    },
    NaiveBayes {
        var_smoothing: f64,
    },
}

impl Algo {
    pub fn name(&self) -> &'static str {
        match self {
            Algo::Linear { .. } => "linear",
            Algo::Ridge { .. } => "ridge",
            Algo::ElasticNet { l1_ratio, .. } if *l1_ratio == 1.0 => "lasso",
            Algo::ElasticNet { .. } => "elastic_net",
            Algo::Logistic { .. } => "logistic",
            Algo::Tree { .. } => "tree",
            Algo::Forest { .. } => "forest",
            Algo::Boost { .. } => "boost",
            Algo::Knn { .. } => "knn",
            Algo::NaiveBayes { .. } => "naive_bayes",
        }
    }
}

pub fn default_boost(seed: u64) -> BoostParams {
    BoostParams {
        loss: Loss::Squared,
        n_trees: 100,
        learning_rate: 0.1,
        tree: HistParams {
            max_depth: Some(3),
            max_leaf_nodes: None,
            min_samples_leaf: 1,
            l2: 0.0,
            min_hessian: 1e-3,
        },
        subsample: 1.0,
        max_bins: 255,
        early_stopping: false,
        validation_fraction: 0.1,
        n_iter_no_change: 10,
        tol: 1e-7,
        seed,
    }
}

// ---------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Task {
    Regression,
    Classification(Labels),
    Unsupervised,
}

#[derive(Clone, Debug)]
pub enum Body {
    Ols(OlsFit),
    Penalized { coef: Vec<f64>, intercept: bool, n_iter: usize },
    Logit(LogitFit),
    Softmax(SoftmaxFit),
    Tree(Tree),
    Forest(Forest),
    Boost(Booster),
    Knn { knn: Knn, y_cls: Vec<usize>, y_reg: Vec<f64> },
    NaiveBayes(GaussianNb),
    KMeans(KMeans),
    Pca(Pca),
    Gmm(Gmm),
    Scaler { method: String, center: Vec<f64>, scale: Vec<f64> },
    OneHot,
    Imputer { fills: Vec<Scalar>, strategy: String },
}

#[derive(Clone, Debug)]
pub struct Model {
    pub algo: &'static str,
    pub design: Design,
    pub y: Option<String>,
    pub task: Task,
    /// Rows the fit used, and rows it dropped for nulls.
    pub n: usize,
    pub dropped: usize,
    pub body: Body,
}

/// Feature values per row out of a model: what `predict` answers before
/// it is shaped into a Series or Frame.
pub enum Raw {
    Values(Vec<f64>),
    /// `p[c][i]`.
    Probs(Vec<Vec<f64>>),
    Clusters(Vec<usize>),
    /// `s[c][i]`: PCA scores.
    Scores(Vec<Vec<f64>>),
}

/// What `predict` hands back.
pub enum Output {
    Series(Series),
    Frame(Frame),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PredictKind {
    /// Values for regression, labels for classification and clusters,
    /// scores for PCA.
    Auto,
    /// Class probabilities: the positive class's for a binary model, a
    /// Frame of one column per class otherwise.
    Prob,
    /// Always a Frame of one column per class.
    ProbFrame,
    /// The linear predictor (log-odds for a logit).
    Link,
    /// An OLS interval: (prediction interval?, level).
    Interval(bool, f64),
}

fn null_where(vals: Vec<f64>, valid: &[bool]) -> Series {
    if valid.iter().all(|v| *v) {
        Series::from_f64(vals)
    } else {
        Series::from_f64_options(vals.into_iter().zip(valid).map(|(v, ok)| ok.then_some(v)).collect())
    }
}

fn argmax_rows(p: &[Vec<f64>], n: usize) -> Vec<usize> {
    (0..n)
        .map(|i| {
            let mut best = 0;
            for c in 1..p.len() {
                if p[c][i] > p[best][i] {
                    best = c;
                }
            }
            best
        })
        .collect()
}

/// The prepared training data of a supervised fit.
struct Prepared {
    design: Design,
    x: Matrix,
    task: Task,
    y_reg: Vec<f64>,
    y_cls: Vec<usize>,
    w: Option<Vec<f64>>,
    n: usize,
    dropped: usize,
}

fn other_columns(frame: &Frame, c: &Common) -> Vec<String> {
    frame
        .names()
        .iter()
        .filter(|n| **n != c.y && Some(n.as_str()) != c.weights.as_deref())
        .cloned()
        .collect()
}

fn prepare(frame: &Frame, c: &Common, drop_first: bool, task_hint: TaskHint) -> Result<Prepared> {
    let x_names = if c.x.is_empty() { other_columns(frame, c) } else { c.x.clone() };
    if x_names.iter().any(|n| *n == c.y) {
        return Err(bad(format!("learn: the target '{}' is also an input", c.y)));
    }
    let design = Design::learn(frame, &x_names, drop_first)?;
    let (x_all, mut valid) = design.encode(frame)?;
    let ys = frame.column(&c.y)?;
    let hint = if task_hint == TaskHint::Auto { c.task } else { task_hint };
    let classify = match hint {
        TaskHint::Classification => true,
        TaskHint::Regression => false,
        TaskHint::Auto => !matches!(ys, Series::F64 { .. }),
    };
    let mut y_reg = Vec::new();
    let mut codes = Vec::new();
    let task = if classify {
        let (labels, cd) = Labels::learn(ys)?;
        for (v, c) in valid.iter_mut().zip(&cd) {
            *v &= *c != NO_CLASS;
        }
        codes = cd;
        Task::Classification(labels)
    } else {
        let (vals, ok) = numeric(ys, &c.y)?;
        for (v, o) in valid.iter_mut().zip(&ok) {
            *v &= *o;
        }
        y_reg = vals;
        Task::Regression
    };
    let mut w_all: Option<Vec<f64>> = None;
    if let Some(wn) = &c.weights {
        let (wv, ok) = numeric(frame.column(wn)?, wn)?;
        for (v, o) in valid.iter_mut().zip(&ok) {
            *v &= *o;
        }
        if wv.iter().zip(&valid).any(|(w, ok)| *ok && *w < 0.0) {
            return Err(bad("learn: weights must be >= 0"));
        }
        w_all = Some(wv);
    }
    let rows = kept(&valid);
    let n = rows.len();
    let dropped = frame.n_rows() - n;
    if n == 0 {
        return Err(bad("learn: no complete rows to fit on"));
    }
    let x = if dropped == 0 { x_all } else { x_all.take(&rows) };
    let y_reg = if classify { Vec::new() } else { rows.iter().map(|&i| y_reg[i]).collect() };
    let y_cls: Vec<usize> = if classify { rows.iter().map(|&i| codes[i]).collect() } else { Vec::new() };
    let mut w: Option<Vec<f64>> = w_all.map(|w| rows.iter().map(|&i| w[i]).collect());
    if let Task::Classification(labels) = &task {
        let cw: Option<Vec<f64>> = match &c.class_weight {
            ClassWeight::None => None,
            ClassWeight::Balanced => {
                let k = labels.len();
                let mut counts = vec![0.0; k];
                for &c in &y_cls {
                    counts[c] += 1.0;
                }
                Some(counts.iter().map(|&cnt| if cnt > 0.0 { n as f64 / (k as f64 * cnt) } else { 0.0 }).collect())
            }
            ClassWeight::Given(pairs) => {
                let names = labels.names();
                let mut v = vec![1.0; names.len()];
                for (l, wgt) in pairs {
                    let pos = names.iter().position(|n| n == l).ok_or_else(|| {
                        bad(format!("learn: class_weight names '{}', which is not a class ({})", l, names.join(", ")))
                    })?;
                    v[pos] = *wgt;
                }
                Some(v)
            }
        };
        if let Some(cw) = cw {
            let base = w.take().unwrap_or_else(|| vec![1.0; n]);
            w = Some(base.iter().zip(&y_cls).map(|(b, &c)| b * cw[c]).collect());
        }
    }
    Ok(Prepared {
        design,
        x,
        task,
        y_reg,
        y_cls,
        w,
        n,
        dropped,
    })
}

/// Fit a supervised model.
pub fn fit(frame: &Frame, c: &Common, algo: &Algo, par: bool) -> Result<Model> {
    let drop_first = matches!(
        algo,
        Algo::Linear { intercept: true }
            | Algo::Ridge { intercept: true, .. }
            | Algo::ElasticNet { intercept: true, .. }
            | Algo::Logistic { .. }
    );
    let hint = match algo {
        Algo::Linear { .. } | Algo::Ridge { .. } | Algo::ElasticNet { .. } => TaskHint::Regression,
        Algo::Logistic { .. } | Algo::NaiveBayes { .. } => TaskHint::Classification,
        _ => TaskHint::Auto,
    };
    let p = prepare(frame, c, drop_first, hint)?;
    let w = p.w.as_deref();
    let body = match algo {
        Algo::Linear { intercept } => Body::Ols(linear::ols(&p.x, &p.y_reg, w, *intercept, par)?),
        Algo::Ridge { alpha, intercept } => {
            if w.is_some() {
                return Err(bad("ridge: weights are not supported"));
            }
            let r = linear::ridge(&p.x, &p.y_reg, *alpha, *intercept, par)?;
            Body::Penalized {
                coef: r.coef,
                intercept: *intercept,
                n_iter: r.n_iter,
            }
        }
        Algo::ElasticNet {
            alpha,
            l1_ratio,
            intercept,
            max_iter,
            tol,
        } => {
            if w.is_some() {
                return Err(bad("elastic_net: weights are not supported"));
            }
            let r = linear::elastic_net(&p.x, &p.y_reg, *alpha, *l1_ratio, *intercept, *max_iter, *tol, par)?;
            Body::Penalized {
                coef: r.coef,
                intercept: *intercept,
                n_iter: r.n_iter,
            }
        }
        Algo::Logistic { lambda, max_iter, tol } => {
            let Task::Classification(labels) = &p.task else { unreachable!() };
            if labels.len() < 2 {
                return Err(bad("logistic: the target has one class"));
            }
            if labels.len() == 2 {
                let y: Vec<f64> = p.y_cls.iter().map(|&c| c as f64).collect();
                Body::Logit(linear::logistic(&p.x, &y, w, *lambda, *max_iter, *tol, par)?)
            } else {
                Body::Softmax(linear::softmax(&p.x, &p.y_cls, labels.len(), w, *lambda, *max_iter, *tol, par)?)
            }
        }
        Algo::Tree { opts, seed } => {
            let (target, n_out_cls) = target_of(&p);
            let params = tree_params(opts, p.x.k(), &p.task, false);
            let ones;
            let wv = match w {
                Some(w) => w,
                None => {
                    ones = vec![1.0; p.n];
                    &ones
                }
            };
            let _ = n_out_cls;
            let t = tree::grow(&p.x, &target, wv, (0..p.n).collect(), &params, &mut Pcg::new(*seed, 0));
            Body::Tree(t)
        }
        Algo::Forest {
            opts,
            n_trees,
            bootstrap,
            max_samples,
            oob,
            seed,
        } => {
            let (target, _) = target_of(&p);
            let classifier = matches!(p.task, Task::Classification(_));
            let params = tree_params(opts, p.x.k(), &p.task, classifier);
            let base = w.map(|w| w.to_vec()).unwrap_or_else(|| vec![1.0; p.n]);
            let fp = ForestParams {
                n_trees: (*n_trees).max(1),
                bootstrap: *bootstrap,
                max_samples: max_samples.map(|f| {
                    if f <= 1.0 {
                        ((f * p.n as f64).round() as usize).max(1)
                    } else {
                        f as usize
                    }
                }),
                oob: *oob,
                tree: params,
                seed: *seed,
            };
            Body::Forest(forest::fit(&p.x, &target, &base, &fp, par))
        }
        Algo::Boost { params } => {
            let mut bp = params.clone();
            let y: Vec<f64> = match &p.task {
                Task::Regression => {
                    bp.loss = Loss::Squared;
                    p.y_reg.clone()
                }
                Task::Classification(labels) => {
                    if labels.len() != 2 {
                        return Err(bad(format!(
                            "boost: classification is binary; the target has {} classes ({})",
                            labels.len(),
                            labels.names().join(", ")
                        )));
                    }
                    bp.loss = Loss::LogLoss;
                    p.y_cls.iter().map(|&c| c as f64).collect()
                }
                Task::Unsupervised => unreachable!(),
            };
            Body::Boost(boost::fit(&p.x, &y, w, &bp, par))
        }
        Algo::Knn { k, distance } => Body::Knn {
            knn: Knn::new(&p.x, *k, *distance),
            y_cls: p.y_cls.clone(),
            y_reg: p.y_reg.clone(),
        },
        Algo::NaiveBayes { var_smoothing } => {
            let Task::Classification(labels) = &p.task else { unreachable!() };
            Body::NaiveBayes(gaussian_nb(&p.x, &p.y_cls, labels.len(), w, *var_smoothing))
        }
    };
    Ok(Model {
        algo: algo.name(),
        design: p.design,
        y: Some(c.y.clone()),
        task: p.task,
        n: p.n,
        dropped: p.dropped,
        body,
    })
}

fn target_of(p: &Prepared) -> (Target<'_>, usize) {
    match &p.task {
        Task::Classification(l) => (
            Target::Classes {
                y: &p.y_cls,
                n_classes: l.len(),
            },
            l.len(),
        ),
        _ => (Target::Values(&p.y_reg), 1),
    }
}

fn tree_params(o: &TreeOpts, k: usize, task: &Task, forest_classifier: bool) -> TreeParams {
    let classify = matches!(task, Task::Classification(_));
    TreeParams {
        criterion: o.criterion.unwrap_or(if classify { Criterion::Gini } else { Criterion::Mse }),
        max_depth: o.max_depth,
        min_samples_split: o.min_samples_split.max(2),
        min_samples_leaf: o.min_samples_leaf.max(1),
        max_features: o.max_features.resolve(k, forest_classifier),
        min_impurity_decrease: o.min_impurity_decrease,
    }
}

// ---------------------------------------------------------------------
// Unsupervised fits and transformers
// ---------------------------------------------------------------------

fn complete(frame: &Frame, cols: &[String], drop_first: bool) -> Result<(Design, Matrix, usize)> {
    let cols: Vec<String> = if cols.is_empty() { frame.names().to_vec() } else { cols.to_vec() };
    let design = Design::learn(frame, &cols, drop_first)?;
    let (x, valid) = design.encode(frame)?;
    let rows = kept(&valid);
    let dropped = frame.n_rows() - rows.len();
    if rows.is_empty() {
        return Err(bad("learn: no complete rows"));
    }
    let x = if dropped == 0 { x } else { x.take(&rows) };
    Ok((design, x, dropped))
}

fn unsupervised(algo: &'static str, design: Design, n: usize, dropped: usize, body: Body) -> Model {
    Model {
        algo,
        design,
        y: None,
        task: Task::Unsupervised,
        n,
        dropped,
        body,
    }
}

pub fn fit_kmeans(frame: &Frame, cols: &[String], p: &KMeansParams, par: bool) -> Result<Model> {
    let (design, x, dropped) = complete(frame, cols, false)?;
    if p.k == 0 || p.k > x.n {
        return Err(bad(format!("kmeans: k must be between 1 and the row count ({})", x.n)));
    }
    let n = x.n;
    Ok(unsupervised("kmeans", design, n, dropped, Body::KMeans(cluster::kmeans(&x, p, par))))
}

pub fn fit_pca(frame: &Frame, cols: &[String], n_components: Option<usize>, scale: bool, par: bool) -> Result<Model> {
    let (design, x, dropped) = complete(frame, cols, false)?;
    let nc = n_components.unwrap_or(x.k()).min(x.k());
    let n = x.n;
    Ok(unsupervised("pca", design, n, dropped, Body::Pca(cluster::pca(&x, nc, scale, par))))
}

pub fn fit_gmm(frame: &Frame, cols: &[String], p: &GmmParams, par: bool) -> Result<Model> {
    let (design, x, dropped) = complete(frame, cols, false)?;
    if p.k == 0 || p.k > x.n {
        return Err(bad(format!("gmm: k must be between 1 and the row count ({})", x.n)));
    }
    let n = x.n;
    Ok(unsupervised("gmm", design, n, dropped, Body::Gmm(cluster::gmm(&x, p, par))))
}

/// A fitted scaler over numeric columns: "standard" (mean, population
/// sd), "minmax" (min, range) or "robust" (median, IQR). A zero scale
/// becomes 1, as in sklearn.
pub fn fit_scaler(frame: &Frame, cols: &[String], method: &str) -> Result<Model> {
    let cols = numeric_columns(frame, cols)?;
    let mut center = Vec::new();
    let mut scale = Vec::new();
    for name in &cols {
        let (v, ok) = numeric(frame.column(name)?, name)?;
        let mut vals: Vec<f64> = v.into_iter().zip(ok).filter_map(|(x, o)| o.then_some(x)).collect();
        if vals.is_empty() {
            return Err(bad(format!("scaler: column '{}' has no values", name)));
        }
        let n = vals.len() as f64;
        let (c, s) = match method {
            "standard" => {
                let m = vals.iter().sum::<f64>() / n;
                let sd = (vals.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n).sqrt();
                (m, sd)
            }
            "minmax" => {
                let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
                let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                (lo, hi - lo)
            }
            "robust" => {
                vals.sort_unstable_by(f64::total_cmp);
                let med = preprocess::quantile_sorted(&vals, 0.5);
                let iqr = preprocess::quantile_sorted(&vals, 0.75) - preprocess::quantile_sorted(&vals, 0.25);
                (med, iqr)
            }
            other => {
                return Err(bad(format!(
                    "scaler: method must be \"standard\", \"minmax\" or \"robust\", got \"{}\"",
                    other
                )));
            }
        };
        center.push(c);
        scale.push(if s == 0.0 || !s.is_finite() { 1.0 } else { s });
    }
    let design = Design {
        inputs: cols.iter().map(|c| Input::Numeric(c.clone())).collect(),
        features: cols.clone(),
    };
    let n = frame.n_rows();
    Ok(unsupervised(
        "scaler",
        design,
        n,
        0,
        Body::Scaler {
            method: method.to_string(),
            center,
            scale,
        },
    ))
}

fn numeric_columns(frame: &Frame, cols: &[String]) -> Result<Vec<String>> {
    if !cols.is_empty() {
        return Ok(cols.to_vec());
    }
    Ok(frame
        .names()
        .iter()
        .zip(frame.columns())
        .filter(|(_, s)| matches!(s, Series::F64 { .. } | Series::I64 { .. }))
        .map(|(n, _)| n.clone())
        .collect())
}

/// A fitted one-hot encoder over String columns (every String column
/// when `cols` is empty).
pub fn fit_one_hot(frame: &Frame, cols: &[String], drop_first: bool) -> Result<Model> {
    let cols: Vec<String> = if cols.is_empty() {
        frame
            .names()
            .iter()
            .zip(frame.columns())
            .filter(|(_, s)| matches!(s, Series::Str { .. }))
            .map(|(n, _)| n.clone())
            .collect()
    } else {
        cols.to_vec()
    };
    for c in &cols {
        if !matches!(frame.column(c)?, Series::Str { .. }) {
            return Err(bad(format!("one_hot: column '{}' is not a String column", c)));
        }
    }
    let design = Design::learn(frame, &cols, drop_first)?;
    let n = frame.n_rows();
    Ok(unsupervised("one_hot", design, n, 0, Body::OneHot))
}

/// A fitted imputer: "mean", "median", "most_frequent" or "constant"
/// (with `fill`). Strings take most_frequent or constant.
pub fn fit_imputer(frame: &Frame, cols: &[String], strategy: &str, fill: Option<Scalar>) -> Result<Model> {
    let cols: Vec<String> = if cols.is_empty() { frame.names().to_vec() } else { cols.to_vec() };
    let mut fills = Vec::new();
    for name in &cols {
        let s = frame.column(name)?;
        let f = match strategy {
            "constant" => fill.clone().ok_or_else(|| bad("imputer: strategy \"constant\" needs a fill value"))?,
            "most_frequent" => most_frequent(s),
            "mean" | "median" => {
                if matches!(s, Series::Str { .. } | Series::Bool { .. }) {
                    return Err(bad(format!(
                        "imputer: \"{}\" needs a numeric column; '{}' is {} (use \"most_frequent\")",
                        strategy,
                        name,
                        s.dtype()
                    )));
                }
                let (v, ok) = numeric(s, name)?;
                let mut vals: Vec<f64> = v.into_iter().zip(ok).filter_map(|(x, o)| o.then_some(x)).collect();
                if vals.is_empty() {
                    Scalar::Null
                } else if strategy == "mean" {
                    Scalar::F64(vals.iter().sum::<f64>() / vals.len() as f64)
                } else {
                    vals.sort_unstable_by(f64::total_cmp);
                    Scalar::F64(preprocess::quantile_sorted(&vals, 0.5))
                }
            }
            other => {
                return Err(bad(format!(
                    "imputer: strategy must be \"mean\", \"median\", \"most_frequent\" or \"constant\", got \"{}\"",
                    other
                )));
            }
        };
        fills.push(f);
    }
    let design = Design {
        inputs: cols.iter().map(|c| Input::Numeric(c.clone())).collect(),
        features: cols.clone(),
    };
    let n = frame.n_rows();
    Ok(unsupervised(
        "imputer",
        design,
        n,
        0,
        Body::Imputer {
            fills,
            strategy: strategy.to_string(),
        },
    ))
}

/// The most common non-null value (the smallest on a tie, as sklearn).
fn most_frequent(s: &Series) -> Scalar {
    let mut counts: std::collections::BTreeMap<String, (usize, Scalar)> = std::collections::BTreeMap::new();
    let mut nums: Vec<(f64, Scalar)> = Vec::new();
    for i in 0..s.len() {
        let v = s.scalar_at(i);
        match &v {
            Scalar::Null => {}
            Scalar::F64(x) if x.is_nan() => {}
            Scalar::F64(x) => nums.push((*x, v.clone())),
            Scalar::I64(x) => nums.push((*x as f64, v.clone())),
            Scalar::Bool(b) => nums.push((if *b { 1.0 } else { 0.0 }, v.clone())),
            Scalar::Str(t) => {
                let e = counts.entry(t.clone()).or_insert((0, v.clone()));
                e.0 += 1;
            }
        }
    }
    if !nums.is_empty() {
        nums.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut best = (0usize, Scalar::Null);
        let mut i = 0;
        while i < nums.len() {
            let mut j = i;
            while j + 1 < nums.len() && nums[j + 1].0 == nums[i].0 {
                j += 1;
            }
            if j - i + 1 > best.0 {
                best = (j - i + 1, nums[i].1.clone());
            }
            i = j + 1;
        }
        return best.1;
    }
    let mut best = (0usize, Scalar::Null);
    for (_, (c, v)) in counts {
        if c > best.0 {
            best = (c, v);
        }
    }
    best.1
}

/// Polynomial features of numeric `cols` up to `degree`, appended to (or
/// replacing, when `!keep`) the inputs. Stateless.
pub fn poly(frame: &Frame, cols: &[String], degree: usize, interaction_only: bool) -> Result<Frame> {
    if degree == 0 {
        return Err(bad("poly: degree must be >= 1"));
    }
    let cols = numeric_columns(frame, cols)?;
    let mut data = Vec::new();
    let mut valid = vec![true; frame.n_rows()];
    for c in &cols {
        let (v, ok) = numeric(frame.column(c)?, c)?;
        for (a, b) in valid.iter_mut().zip(&ok) {
            *a &= *b;
        }
        data.push(v);
    }
    let mut out = frame.clone();
    for term in preprocess::poly_terms(cols.len(), degree, interaction_only) {
        if term.len() == 1 {
            continue;
        }
        let name = preprocess::poly_name(&term, &cols);
        let vals: Vec<f64> = (0..frame.n_rows())
            .map(|i| term.iter().map(|&j| data[j][i]).product())
            .collect();
        out = out.with_column(&name, null_where(vals, &valid))?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// Prediction
// ---------------------------------------------------------------------

impl Model {
    pub fn classes(&self) -> Option<&Labels> {
        match &self.task {
            Task::Classification(l) => Some(l),
            _ => None,
        }
    }

    pub fn features(&self) -> &[String] {
        &self.design.features
    }

    pub fn inputs(&self) -> Vec<String> {
        self.design.inputs.iter().map(|i| i.name().to_string()).collect()
    }

    /// Raw predictions for an encoded matrix.
    pub fn raw(&self, x: &Matrix, par: bool) -> Result<Raw> {
        Ok(match &self.body {
            Body::Ols(f) => Raw::Values(linear::linear_predict(x, &f.coef, f.intercept)),
            Body::Penalized { coef, intercept, .. } => {
                let c = if *intercept { coef.clone() } else { coef[1..].to_vec() };
                Raw::Values(linear::linear_predict(x, &c, *intercept))
            }
            Body::Logit(f) => {
                let eta = linear::linear_predict(x, &f.coef, true);
                let p1: Vec<f64> = eta.iter().map(|&e| linear::sigmoid(e)).collect();
                let p0 = p1.iter().map(|p| 1.0 - p).collect();
                Raw::Probs(vec![p0, p1])
            }
            Body::Softmax(f) => Raw::Probs(linear::softmax_predict(x, &f.coef)),
            Body::Tree(t) => {
                let mut out = vec![vec![0.0; x.n]; t.n_out];
                t.accumulate(x, &mut out, 1.0);
                match self.task {
                    Task::Classification(_) => Raw::Probs(out),
                    _ => Raw::Values(out.pop().unwrap()),
                }
            }
            Body::Forest(f) => {
                let mut out = f.predict(x, par);
                match self.task {
                    Task::Classification(_) => Raw::Probs(out),
                    _ => Raw::Values(out.pop().unwrap()),
                }
            }
            Body::Boost(b) => {
                let r = b.raw(x, par);
                match b.loss {
                    Loss::Squared => Raw::Values(r),
                    Loss::LogLoss => {
                        let p1: Vec<f64> = r.iter().map(|&e| linear::sigmoid(e)).collect();
                        let p0 = p1.iter().map(|p| 1.0 - p).collect();
                        Raw::Probs(vec![p0, p1])
                    }
                }
            }
            Body::Knn { knn, y_cls, y_reg } => match &self.task {
                Task::Classification(l) => Raw::Probs(knn.predict_proba(x, y_cls, l.len(), par)),
                _ => Raw::Values(knn.predict_values(x, y_reg, par)),
            },
            Body::NaiveBayes(nb) => Raw::Probs(nb.predict_proba(x)),
            Body::KMeans(k) => Raw::Clusters(k.predict(x, par).0),
            Body::Pca(p) => Raw::Scores(p.transform(x)),
            Body::Gmm(g) => Raw::Probs(g.predict_proba(x)),
            Body::Scaler { .. } | Body::OneHot | Body::Imputer { .. } => {
                return Err(bad(format!(
                    "predict: a {} transforms a frame (learn.transform) rather than predicting",
                    self.algo
                )));
            }
        })
    }

    /// The linear predictor, for the linear models.
    fn link(&self, x: &Matrix) -> Result<Vec<f64>> {
        match &self.body {
            Body::Ols(f) => Ok(linear::linear_predict(x, &f.coef, f.intercept)),
            Body::Logit(f) => Ok(linear::linear_predict(x, &f.coef, true)),
            Body::Boost(b) => Ok(b.raw(x, false)),
            Body::Penalized { coef, intercept, .. } => {
                let c = if *intercept { coef.clone() } else { coef[1..].to_vec() };
                Ok(linear::linear_predict(x, &c, *intercept))
            }
            _ => Err(bad(format!("predict: kind \"link\" needs a linear or logistic model, not {}", self.algo))),
        }
    }

    /// Predict on a Frame.
    pub fn predict(&self, frame: &Frame, kind: PredictKind, par: bool) -> Result<Output> {
        let (x, valid) = self.design.encode(frame)?;
        if let PredictKind::Interval(prediction, level) = kind {
            let Body::Ols(f) = &self.body else {
                return Err(bad(format!("predict: intervals need a linear (OLS) model, not {}", self.algo)));
            };
            let fit = linear::linear_predict(&x, &f.coef, f.intercept);
            let se_mean = f.mean_se(&x);
            let tq = dist::t_ppf(0.5 + level / 2.0, f.df_resid)?;
            let half: Vec<f64> = se_mean
                .iter()
                .map(|s| tq * if prediction { (s * s + f.sigma2).sqrt() } else { *s })
                .collect();
            let lo: Vec<f64> = fit.iter().zip(&half).map(|(a, b)| a - b).collect();
            let hi: Vec<f64> = fit.iter().zip(&half).map(|(a, b)| a + b).collect();
            return Ok(Output::Frame(Frame::new(vec![
                ("fit".to_string(), null_where(fit, &valid)),
                ("se".to_string(), null_where(se_mean, &valid)),
                ("lo".to_string(), null_where(lo, &valid)),
                ("hi".to_string(), null_where(hi, &valid)),
            ])?));
        }
        if kind == PredictKind::Link {
            return Ok(Output::Series(null_where(self.link(&x)?, &valid)));
        }
        let raw = self.raw(&x, par)?;
        Ok(match raw {
            Raw::Values(v) => {
                if matches!(kind, PredictKind::Prob | PredictKind::ProbFrame) {
                    return Err(bad(format!("predict: kind \"prob\" needs a classifier; {} is a regression", self.algo)));
                }
                Output::Series(null_where(v, &valid))
            }
            Raw::Clusters(c) => {
                let opts: Vec<Option<i64>> = c.iter().zip(&valid).map(|(c, ok)| ok.then_some(*c as i64)).collect();
                Output::Series(Series::from_i64_options(opts))
            }
            Raw::Scores(s) => {
                let cols = s
                    .into_iter()
                    .enumerate()
                    .map(|(c, v)| (format!("pc{}", c + 1), null_where(v, &valid)))
                    .collect();
                Output::Frame(Frame::new(cols)?)
            }
            Raw::Probs(p) => {
                let names: Vec<String> = match (&self.task, &self.body) {
                    (Task::Classification(l), _) => l.names(),
                    (_, Body::Gmm(g)) => (0..g.k).map(|c| c.to_string()).collect(),
                    _ => (0..p.len()).map(|c| c.to_string()).collect(),
                };
                match kind {
                    PredictKind::Prob if p.len() == 2 => Output::Series(null_where(p[1].clone(), &valid)),
                    PredictKind::Prob | PredictKind::ProbFrame => {
                        let cols = p
                            .into_iter()
                            .zip(&names)
                            .map(|(v, n)| (n.clone(), null_where(v, &valid)))
                            .collect();
                        Output::Frame(Frame::new(cols)?)
                    }
                    _ => {
                        let mut cls = argmax_rows(&p, x.n);
                        for (c, ok) in cls.iter_mut().zip(&valid) {
                            if !ok {
                                *c = NO_CLASS;
                            }
                        }
                        match &self.task {
                            Task::Classification(l) => Output::Series(l.series(&cls)),
                            _ => Output::Series(Series::from_i64_options(
                                cls.iter().map(|&c| (c != NO_CLASS).then_some(c as i64)).collect(),
                            )),
                        }
                    }
                }
            }
        })
    }

    /// Apply a fitted transformer (scaler, one-hot encoder, imputer, PCA).
    pub fn transform(&self, frame: &Frame) -> Result<Frame> {
        match &self.body {
            Body::Scaler { center, scale, .. } => {
                let mut out = frame.clone();
                for (j, input) in self.design.inputs.iter().enumerate() {
                    let name = input.name();
                    let (v, ok) = numeric(frame.column(name)?, name)?;
                    let vals: Vec<f64> = v.iter().map(|x| (x - center[j]) / scale[j]).collect();
                    out = out.with_column(name, null_where(vals, &ok))?;
                }
                Ok(out)
            }
            Body::OneHot => {
                let (x, valid) = self.design.encode(frame)?;
                let drop: Vec<String> = self.inputs();
                let mut pairs: Vec<(String, Series)> = frame
                    .names()
                    .iter()
                    .zip(frame.columns())
                    .filter(|(n, _)| !drop.contains(n))
                    .map(|(n, s)| (n.clone(), s.clone()))
                    .collect();
                for (name, col) in self.design.features.iter().zip(x.cols) {
                    pairs.push((name.clone(), null_where(col, &valid)));
                }
                Frame::new(pairs)
            }
            Body::Imputer { fills, .. } => {
                let mut out = frame.clone();
                for (input, fill) in self.design.inputs.iter().zip(fills) {
                    let name = input.name();
                    let s = frame.column(name)?;
                    if matches!(fill, Scalar::Null) {
                        continue;
                    }
                    let filled = fill_nan_and_null(s, fill)?;
                    out = out.with_column(name, filled)?;
                }
                Ok(out)
            }
            Body::Pca(_) => match self.predict(frame, PredictKind::Auto, false)? {
                Output::Frame(f) => Ok(f),
                Output::Series(_) => unreachable!(),
            },
            _ => Err(bad(format!("transform: a {} model predicts (learn.predict) rather than transforming", self.algo))),
        }
    }
}

fn fill_nan_and_null(s: &Series, fill: &Scalar) -> Result<Series> {
    // NaNs count as missing, as they do everywhere in learn.
    let s = match s {
        Series::F64 { values, validity } if values.iter().any(|v| v.is_nan()) => {
            let opts: Vec<Option<f64>> = values
                .iter()
                .enumerate()
                .map(|(i, v)| (validity.as_ref().is_none_or(|b| b.get(i)) && !v.is_nan()).then_some(*v))
                .collect();
            Series::from_f64_options(opts)
        }
        other => other.clone(),
    };
    let fill = match (&s, fill) {
        (Series::I64 { .. }, Scalar::F64(f)) => {
            // An Int column filled with a fractional mean becomes Float.
            if f.fract() == 0.0 {
                Scalar::I64(*f as i64)
            } else {
                let as_f: Vec<Option<f64>> = (0..s.len())
                    .map(|i| match s.scalar_at(i) {
                        Scalar::I64(v) => Some(v as f64),
                        _ => None,
                    })
                    .collect();
                return Series::from_f64_options(as_f).fill_null(Scalar::F64(*f));
            }
        }
        _ => fill.clone(),
    };
    s.fill_null(fill)
}

// ---------------------------------------------------------------------
// Explanation: importances and summaries
// ---------------------------------------------------------------------

fn frame_of(cols: Vec<(&str, Series)>) -> Result<Frame> {
    Frame::new(cols.into_iter().map(|(n, s)| (n.to_string(), s)).collect())
}

impl Model {
    /// Term names: "intercept" first for models that fit one.
    pub fn terms(&self) -> Vec<String> {
        let mut t = Vec::new();
        let has_intercept = match &self.body {
            Body::Ols(f) => f.intercept,
            Body::Penalized { intercept, .. } => *intercept,
            Body::Logit(_) | Body::Softmax(_) => true,
            _ => false,
        };
        if has_intercept {
            t.push("intercept".to_string());
        }
        t.extend(self.design.features.iter().cloned());
        t
    }

    /// The estimates by term, for models with coefficients.
    pub fn params(&self) -> Option<Vec<(String, f64)>> {
        let terms = self.terms();
        let coef: Vec<f64> = match &self.body {
            Body::Ols(f) => f.coef.clone(),
            Body::Penalized { coef, intercept, .. } => {
                if *intercept {
                    coef.clone()
                } else {
                    coef[1..].to_vec()
                }
            }
            Body::Logit(f) => f.coef.clone(),
            _ => return None,
        };
        Some(terms.into_iter().zip(coef).collect())
    }

    /// Feature importances, largest first: impurity decrease for trees
    /// (sklearn's normalized `feature_importances_`; split gain for
    /// boosting), |coefficient| for linear models.
    pub fn importance(&self) -> Result<Frame> {
        let k = self.design.features.len();
        let imp: Vec<f64> = match &self.body {
            Body::Tree(t) => tree::importances(&[t], k),
            Body::Forest(f) => tree::importances(&f.trees.iter().collect::<Vec<_>>(), k),
            Body::Boost(b) => {
                let mut raw = vec![0.0; k];
                for t in &b.trees {
                    for (r, v) in raw.iter_mut().zip(&t.importance) {
                        *r += v;
                    }
                }
                let total: f64 = raw.iter().sum();
                raw.iter().map(|v| if total > 0.0 { v / total } else { 0.0 }).collect()
            }
            Body::Ols(_) | Body::Penalized { .. } | Body::Logit(_) => {
                let p = self.params().unwrap();
                p.iter()
                    .filter(|(n, _)| n != "intercept")
                    .map(|(_, v)| v.abs())
                    .collect()
            }
            Body::Softmax(f) => (0..k)
                .map(|j| f.coef.iter().map(|b| b[j + 1].abs()).sum::<f64>() / f.coef.len() as f64)
                .collect(),
            _ => {
                return Err(bad(format!(
                    "importance: a {} model has no built-in importances; use learn.permutation_importance",
                    self.algo
                )));
            }
        };
        let mut order: Vec<usize> = (0..k).collect();
        order.sort_by(|&a, &b| imp[b].total_cmp(&imp[a]).then(a.cmp(&b)));
        frame_of(vec![
            ("feature", Series::from_str_values(order.iter().map(|&i| self.design.features[i].clone()).collect())),
            ("importance", Series::from_f64(order.iter().map(|&i| imp[i]).collect())),
        ])
    }

    /// The model as a table: coefficients with inference for the linear
    /// models, importances for trees, centres, components, ...
    pub fn summary(&self, level: f64) -> Result<Frame> {
        let terms = self.terms();
        let z = dist::norm_ppf(0.5 + level / 2.0, 0.0, 1.0)?;
        match &self.body {
            Body::Ols(f) => {
                let tq = dist::t_ppf(0.5 + level / 2.0, f.df_resid)?;
                frame_of(vec![
                    ("term", Series::from_str_values(terms)),
                    ("estimate", Series::from_f64(f.coef.clone())),
                    ("se", Series::from_f64(f.se.clone())),
                    ("t", Series::from_f64(f.t.clone())),
                    ("p", Series::from_f64(f.p.clone())),
                    ("lo", Series::from_f64(f.coef.iter().zip(&f.se).map(|(c, s)| c - tq * s).collect())),
                    ("hi", Series::from_f64(f.coef.iter().zip(&f.se).map(|(c, s)| c + tq * s).collect())),
                ])
            }
            Body::Logit(f) => frame_of(vec![
                ("term", Series::from_str_values(terms)),
                ("estimate", Series::from_f64(f.coef.clone())),
                ("se", Series::from_f64(f.se.clone())),
                ("z", Series::from_f64(f.z.clone())),
                ("p", Series::from_f64(f.p.clone())),
                ("lo", Series::from_f64(f.coef.iter().zip(&f.se).map(|(c, s)| c - z * s).collect())),
                ("hi", Series::from_f64(f.coef.iter().zip(&f.se).map(|(c, s)| c + z * s).collect())),
                ("odds_ratio", Series::from_f64(f.coef.iter().map(|c| c.exp()).collect())),
            ]),
            Body::Penalized { .. } => {
                let p = self.params().unwrap();
                frame_of(vec![
                    ("term", Series::from_str_values(p.iter().map(|v| v.0.clone()).collect())),
                    ("estimate", Series::from_f64(p.iter().map(|v| v.1).collect())),
                ])
            }
            Body::Softmax(f) => {
                let labels = self.classes().unwrap();
                let mut cls = Vec::new();
                let mut term = Vec::new();
                let mut est = Vec::new();
                for (c, b) in f.coef.iter().enumerate() {
                    for (t, v) in terms.iter().zip(b) {
                        cls.push(labels.name(c));
                        term.push(t.clone());
                        est.push(*v);
                    }
                }
                frame_of(vec![
                    ("class", Series::from_str_values(cls)),
                    ("term", Series::from_str_values(term)),
                    ("estimate", Series::from_f64(est)),
                ])
            }
            Body::Tree(_) | Body::Forest(_) | Body::Boost(_) => self.importance(),
            Body::NaiveBayes(nb) => {
                let labels = self.classes().unwrap();
                let mut cls = Vec::new();
                let mut feat = Vec::new();
                let mut mean = Vec::new();
                let mut var = Vec::new();
                for c in 0..nb.prior.len() {
                    for (j, fname) in self.design.features.iter().enumerate() {
                        cls.push(labels.name(c));
                        feat.push(fname.clone());
                        mean.push(nb.theta[c][j]);
                        var.push(nb.var[c][j]);
                    }
                }
                frame_of(vec![
                    ("class", Series::from_str_values(cls)),
                    ("feature", Series::from_str_values(feat)),
                    ("mean", Series::from_f64(mean)),
                    ("var", Series::from_f64(var)),
                ])
            }
            Body::KMeans(k) => self.centers_frame(&k.centers, k.k, Some(&k.labels)),
            Body::Gmm(g) => {
                let mut f = self.centers_frame(&g.means, g.k, None)?;
                f = f.with_column("weight", Series::from_f64(g.weights.clone()))?;
                Ok(f)
            }
            Body::Pca(p) => {
                let nc = p.n_components;
                let mut cum = 0.0;
                let mut cols: Vec<(String, Series)> = vec![
                    ("component".to_string(), Series::from_str_values((1..=nc).map(|c| format!("pc{}", c)).collect())),
                    ("explained_variance".to_string(), Series::from_f64(p.explained_variance.clone())),
                    ("ratio".to_string(), Series::from_f64(p.explained_variance_ratio.clone())),
                    (
                        "cumulative".to_string(),
                        Series::from_f64(
                            p.explained_variance_ratio
                                .iter()
                                .map(|r| {
                                    cum += r;
                                    cum
                                })
                                .collect(),
                        ),
                    ),
                ];
                for (j, fname) in self.design.features.iter().enumerate() {
                    cols.push((fname.clone(), Series::from_f64((0..nc).map(|c| p.components[c * p.dims + j]).collect())));
                }
                Frame::new(cols)
            }
            Body::Knn { .. } => frame_of(vec![("feature", Series::from_str_values(self.design.features.clone()))]),
            Body::Scaler { center, scale, .. } => frame_of(vec![
                ("column", Series::from_str_values(self.design.features.clone())),
                ("center", Series::from_f64(center.clone())),
                ("scale", Series::from_f64(scale.clone())),
            ]),
            Body::OneHot => {
                let mut col = Vec::new();
                let mut lvl = Vec::new();
                let mut feat = Vec::new();
                for input in &self.design.inputs {
                    if let Input::Categorical { name, levels, drop_first } = input {
                        for (i, l) in levels.iter().enumerate() {
                            col.push(name.clone());
                            lvl.push(l.clone());
                            feat.push(if i == 0 && *drop_first { None } else { Some(format!("{}_{}", name, l)) });
                        }
                    }
                }
                frame_of(vec![
                    ("column", Series::from_str_values(col)),
                    ("level", Series::from_str_values(lvl)),
                    ("feature", Series::from_str_options(feat)),
                ])
            }
            Body::Imputer { fills, .. } => frame_of(vec![
                ("column", Series::from_str_values(self.design.features.clone())),
                (
                    "fill",
                    Series::from_str_options(fills.iter().map(|f| scalar_string(f)).collect()),
                ),
            ]),
        }
    }

    fn centers_frame(&self, centers: &[f64], k: usize, labels: Option<&[usize]>) -> Result<Frame> {
        let d = self.design.features.len();
        let mut cols: Vec<(String, Series)> = vec![("cluster".to_string(), Series::from_i64((0..k as i64).collect()))];
        if let Some(l) = labels {
            let mut size = vec![0i64; k];
            for &c in l {
                size[c] += 1;
            }
            cols.push(("size".to_string(), Series::from_i64(size)));
        }
        for (j, fname) in self.design.features.iter().enumerate() {
            cols.push((fname.clone(), Series::from_f64((0..k).map(|c| centers[c * d + j]).collect())));
        }
        Frame::new(cols)
    }
}

fn scalar_string(s: &Scalar) -> Option<String> {
    match s {
        Scalar::Null => None,
        Scalar::F64(v) => Some(v.to_string()),
        Scalar::I64(v) => Some(v.to_string()),
        Scalar::Bool(v) => Some(v.to_string()),
        Scalar::Str(v) => Some(v.clone()),
    }
}

// ---------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------

/// Whether a larger value of `metric` is better.
pub fn greater_is_better(metric: &str) -> Result<bool> {
    Ok(match metric {
        "r2" | "accuracy" | "precision" | "recall" | "f1" | "auc" | "roc_auc" | "average_precision" => true,
        "rmse" | "mse" | "mae" | "mape" | "log_loss" | "brier" => false,
        other => {
            return Err(bad(format!(
                "learn: unknown metric \"{}\" (rmse, mse, mae, mape, r2, accuracy, precision, recall, f1, \
                 log_loss, auc, average_precision, brier)",
                other
            )));
        }
    })
}

impl Model {
    /// Score the model on `frame` (which carries the target column).
    pub fn score(&self, frame: &Frame, metric: &str, par: bool) -> Result<f64> {
        greater_is_better(metric)?;
        let yname = self.y.as_ref().ok_or_else(|| bad("score: an unsupervised model has no target"))?;
        let (x, mut valid) = self.design.encode(frame)?;
        let ys = frame.column(yname)?;
        let raw = self.raw(&x, par)?;
        match (&self.task, raw) {
            (Task::Regression, Raw::Values(p)) => {
                let (yv, ok) = numeric(ys, yname)?;
                for (v, o) in valid.iter_mut().zip(&ok) {
                    *v &= *o;
                }
                let rows = kept(&valid);
                let y: Vec<f64> = rows.iter().map(|&i| yv[i]).collect();
                let p: Vec<f64> = rows.iter().map(|&i| p[i]).collect();
                if y.is_empty() {
                    return Err(bad("score: no complete rows"));
                }
                Ok(match metric {
                    "rmse" => metrics::rmse(&y, &p),
                    "mse" => metrics::mse(&y, &p),
                    "mae" => metrics::mae(&y, &p),
                    "mape" => metrics::mape(&y, &p),
                    "r2" => metrics::r2(&y, &p),
                    other => return Err(bad(format!("score: \"{}\" is a classification metric; this is a regression", other))),
                })
            }
            (Task::Classification(labels), Raw::Probs(p)) => {
                let codes = labels.encode(ys);
                for (v, c) in valid.iter_mut().zip(&codes) {
                    *v &= *c != NO_CLASS;
                }
                let rows = kept(&valid);
                if rows.is_empty() {
                    return Err(bad("score: no complete rows with a known class"));
                }
                let y: Vec<usize> = rows.iter().map(|&i| codes[i]).collect();
                let probs: Vec<Vec<f64>> = p.iter().map(|c| rows.iter().map(|&i| c[i]).collect()).collect();
                let pred = argmax_rows(&probs, rows.len());
                let k = labels.len();
                let avg = if k == 2 { metrics::Average::Binary(1) } else { metrics::Average::Macro };
                Ok(match metric {
                    "accuracy" => metrics::accuracy(&y, &pred),
                    "precision" => metrics::prf(&y, &pred, k, avg).0,
                    "recall" => metrics::prf(&y, &pred, k, avg).1,
                    "f1" => metrics::prf(&y, &pred, k, avg).2,
                    "log_loss" => {
                        if k == 2 {
                            let yb: Vec<bool> = y.iter().map(|&c| c == 1).collect();
                            metrics::log_loss(&yb, &probs[1])
                        } else {
                            metrics::log_loss_multi(&y, &probs)
                        }
                    }
                    "auc" | "roc_auc" | "average_precision" | "brier" => {
                        if k != 2 {
                            return Err(bad(format!("score: \"{}\" needs a binary classifier", metric)));
                        }
                        let yb: Vec<bool> = y.iter().map(|&c| c == 1).collect();
                        match metric {
                            "average_precision" => metrics::average_precision(&yb, &probs[1]),
                            "brier" => metrics::brier(&yb, &probs[1]),
                            _ => metrics::roc_auc(&yb, &probs[1]).unwrap_or(f64::NAN),
                        }
                    }
                    other => return Err(bad(format!("score: \"{}\" is a regression metric; this is a classifier", other))),
                })
            }
            _ => Err(bad(format!("score: a {} model is not scored against a target", self.algo))),
        }
    }

    /// Permutation importance of each input column: the drop in `metric`
    /// (sign-adjusted so larger is more important) when that column's
    /// values are shuffled, `repeats` times. Shuffles come from stream
    /// (column, repeat) of `seed`.
    pub fn permutation_importance(&self, frame: &Frame, metric: &str, repeats: usize, seed: u64, par: bool) -> Result<Frame> {
        let sign = if greater_is_better(metric)? { 1.0 } else { -1.0 };
        let base = self.score(frame, metric, par)?;
        let inputs = self.inputs();
        let n = frame.n_rows();
        let one = |(ci, name): (usize, &String)| -> Result<(f64, f64)> {
            let col = frame.column(name)?;
            let mut drops = Vec::with_capacity(repeats);
            for r in 0..repeats {
                let mut rng = Pcg::new(seed, (ci * 10_007 + r) as u64 + 1);
                let perm: Vec<i64> = rng.permutation(n).into_iter().map(|i| i as i64).collect();
                let shuffled = col.take(&Series::from_i64(perm))?;
                let f = frame.with_column(name, shuffled)?;
                let s = self.score(&f, metric, false)?;
                drops.push(sign * (base - s));
            }
            let m = drops.iter().sum::<f64>() / repeats as f64;
            let sd = (drops.iter().map(|d| (d - m) * (d - m)).sum::<f64>() / repeats as f64).sqrt();
            Ok((m, sd))
        };
        let results: Vec<Result<(f64, f64)>> = {
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;
                if par {
                    inputs.par_iter().enumerate().map(one).collect()
                } else {
                    inputs.iter().enumerate().map(one).collect()
                }
            }
            #[cfg(not(feature = "parallel"))]
            {
                inputs.iter().enumerate().map(one).collect()
            }
        };
        let results: Vec<(f64, f64)> = results.into_iter().collect::<Result<_>>()?;
        let mut order: Vec<usize> = (0..inputs.len()).collect();
        order.sort_by(|&a, &b| results[b].0.total_cmp(&results[a].0).then(a.cmp(&b)));
        frame_of(vec![
            ("feature", Series::from_str_values(order.iter().map(|&i| inputs[i].clone()).collect())),
            ("importance", Series::from_f64(order.iter().map(|&i| results[i].0).collect())),
            ("std", Series::from_f64(order.iter().map(|&i| results[i].1).collect())),
        ])
    }

    /// One-dimensional partial dependence: the average prediction (the
    /// positive-class probability for a binary classifier, one column
    /// per class otherwise) with `feature` set to each grid value for
    /// every row.
    pub fn partial_dependence(&self, frame: &Frame, feature: &str, grid: Option<Vec<Scalar>>, points: usize, par: bool) -> Result<Frame> {
        let col = frame.column(feature)?;
        let grid: Vec<Scalar> = match grid {
            Some(g) => g,
            None => match col {
                Series::Str { .. } => super::data::levels_of(col).into_iter().map(Scalar::Str).collect(),
                Series::Bool { .. } => vec![Scalar::Bool(false), Scalar::Bool(true)],
                _ => {
                    let (v, ok) = numeric(col, feature)?;
                    let mut vals: Vec<f64> = v.into_iter().zip(ok).filter_map(|(x, o)| o.then_some(x)).collect();
                    vals.sort_unstable_by(f64::total_cmp);
                    let mut d = vals.clone();
                    d.dedup();
                    if d.len() <= points {
                        d.into_iter().map(Scalar::F64).collect()
                    } else {
                        // sklearn: an even grid between the 5th and 95th percentiles.
                        let lo = preprocess::quantile_sorted(&vals, 0.05);
                        let hi = preprocess::quantile_sorted(&vals, 0.95);
                        (0..points)
                            .map(|i| Scalar::F64(lo + (hi - lo) * i as f64 / (points - 1).max(1) as f64))
                            .collect()
                    }
                }
            },
        };
        let n = frame.n_rows();
        let mut outs: Vec<Vec<f64>> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for g in &grid {
            let constant = constant_like(col, g, n)?;
            let f = frame.with_column(feature, constant)?;
            let (x, valid) = self.design.encode(&f)?;
            let raw = self.raw(&x, par)?;
            let rows = kept(&valid);
            let mean = |v: &[f64]| rows.iter().map(|&i| v[i]).sum::<f64>() / rows.len().max(1) as f64;
            let vals: Vec<f64> = match &raw {
                Raw::Values(v) => {
                    names = vec!["mean".to_string()];
                    vec![mean(v)]
                }
                Raw::Probs(p) if p.len() == 2 => {
                    names = vec!["mean".to_string()];
                    vec![mean(&p[1])]
                }
                Raw::Probs(p) => {
                    names = self.classes().map(|l| l.names()).unwrap_or_default();
                    p.iter().map(|c| mean(c)).collect()
                }
                _ => return Err(bad("partial_dependence: needs a regression or a classifier")),
            };
            outs.push(vals);
        }
        let mut cols: Vec<(String, Series)> = vec![(feature.to_string(), scalars_series(&grid))];
        for (c, name) in names.iter().enumerate() {
            cols.push((name.clone(), Series::from_f64(outs.iter().map(|o| o[c]).collect())));
        }
        Frame::new(cols)
    }
}

fn constant_like(col: &Series, v: &Scalar, n: usize) -> Result<Series> {
    Ok(match (col, v) {
        (Series::Str { .. }, Scalar::Str(s)) => Series::from_str_values(vec![s.clone(); n]),
        (Series::Bool { .. }, Scalar::Bool(b)) => Series::from_bool(vec![*b; n]),
        (Series::I64 { .. }, Scalar::I64(i)) => Series::from_i64(vec![*i; n]),
        (_, Scalar::F64(f)) => Series::from_f64(vec![*f; n]),
        (_, Scalar::I64(i)) => Series::from_f64(vec![*i as f64; n]),
        _ => return Err(bad("partial_dependence: a grid value does not fit the column's type")),
    })
}

fn scalars_series(v: &[Scalar]) -> Series {
    match v.first() {
        Some(Scalar::Str(_)) => Series::from_str_values(
            v.iter()
                .map(|s| if let Scalar::Str(t) = s { t.clone() } else { String::new() })
                .collect(),
        ),
        Some(Scalar::Bool(_)) => Series::from_bool(v.iter().map(|s| matches!(s, Scalar::Bool(true))).collect()),
        Some(Scalar::I64(_)) => Series::from_i64(v.iter().map(|s| if let Scalar::I64(i) = s { *i } else { 0 }).collect()),
        _ => Series::from_f64(
            v.iter()
                .map(|s| match s {
                    Scalar::F64(f) => *f,
                    Scalar::I64(i) => *i as f64,
                    _ => f64::NAN,
                })
                .collect(),
        ),
    }
}

/// Row-level validity helper for callers that hold a Series.
pub fn valid_rows(s: &Series) -> Vec<bool> {
    (0..s.len()).map(|i| is_valid(s, i)).collect()
}

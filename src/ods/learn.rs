//! The `learn` namespace: machine learning over Frames.
//!
//! Thin glue over `olang_ods::learn` (the kernels): argument checking,
//! options maps, and shaping results as olang values. Every function
//! takes its data positionally and its options in one trailing map,
//! which may be left out; an unknown option is an error naming the ones
//! the function knows. The keyword layer is `use ml` (ml.ol).
//!
//! A fit returns a Map — `fit.params.sqft`, `fit.r2`, `fit.coef` (a
//! Frame) read like fields — whose `model` entry is the native fitted
//! model and whose `spec` entry is how to fit it again (`learn.fit(spec,
//! frame)`), which is what cross-validation, learning curves and grid
//! search refit natively, in parallel over folds.
//!
//! Randomness: every function that draws takes `seed`. Without one it
//! draws a seed from the `random` module's stream (so `random.seed(k)`
//! still makes a program reproducible) and records it in the model's
//! spec, so a refit draws the same.

use super::frame::{OdsFrame, frame_of};
use super::series::{make_series_value, series_from_list, series_of, value_to_scalar};
use crate::ast::{BuiltinFunction, Value, ValueMap};
use crate::native::{NativeHandle, NativeObject};
use olang_ods::learn::boost::{BoostParams, HistParams};
use olang_ods::learn::cluster::{GmmParams, KMeansParams};
use olang_ods::learn::data::{Labels, NO_CLASS, binary_target, numeric};
use olang_ods::learn::metrics::{self, Average};
use olang_ods::learn::model::{self as lm, default_boost};
use olang_ods::learn::tree::Criterion;
use olang_ods::learn::{Algo, ClassWeight, Common, MaxFeatures, Model, Output, Pcg, PredictKind, TaskHint, TreeOpts};
use olang_ods::learn::{preprocess, rng::splitmix64};
use olang_ods::{Frame, Scalar, Series};
use std::any::Any;
use std::sync::Arc;

#[cfg(feature = "native")]
use rayon::prelude::*;

// ---------------------------------------------------------------------
// The native model value
// ---------------------------------------------------------------------

#[derive(Debug)]
pub struct LearnModel {
    pub model: Arc<Model>,
}

impl NativeObject for LearnModel {
    fn module(&self) -> &'static str {
        "learn"
    }

    fn type_name(&self) -> &'static str {
        "Model"
    }

    fn display(&self) -> String {
        let m = &self.model;
        format!(
            "<learn.{} on {} rows, {} feature{}>",
            m.algo,
            m.n,
            m.design.features.len(),
            if m.design.features.len() == 1 { "" } else { "s" }
        )
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<LearnModel>()
            .is_some_and(|o| Arc::ptr_eq(&o.model, &self.model))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The fitted model in a value: the native itself, or a fit's Map.
fn model_of(v: &Value) -> Option<&Arc<Model>> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<LearnModel>().map(|m| &m.model),
        Value::Map(m) => m.get("model").and_then(model_of),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Namespace
// ---------------------------------------------------------------------

/// (name, fewest arguments).
pub const FUNCTIONS: &[(&str, usize)] = &[
    // splitting
    ("split", 1),
    ("kfold", 2),
    ("folds", 2),
    ("time_splits", 2),
    // preprocessing
    ("scaler", 1),
    ("one_hot", 1),
    ("imputer", 1),
    ("transform", 2),
    ("poly", 3),
    // supervised models
    ("linear", 2),
    ("ridge", 2),
    ("lasso", 2),
    ("elastic_net", 2),
    ("logistic", 2),
    ("tree", 2),
    ("forest", 2),
    ("boost", 2),
    ("knn", 2),
    ("naive_bayes", 2),
    // unsupervised
    ("kmeans", 3),
    ("gmm", 3),
    ("pca", 1),
    ("silhouette", 3),
    // the model interface
    ("predict", 2),
    ("importance", 1),
    ("summary", 1),
    ("params", 1),
    ("score", 3),
    ("spec", 2),
    ("fit", 2),
    // evaluation
    ("cross_validate", 2),
    ("permutation_importance", 2),
    ("partial_dependence", 3),
    ("learning_curve", 2),
    ("grid_search", 3),
    // metrics
    ("rmse", 2),
    ("mse", 2),
    ("mae", 2),
    ("mape", 2),
    ("r2", 2),
    ("accuracy", 2),
    ("precision", 2),
    ("recall", 2),
    ("f1", 2),
    ("log_loss", 2),
    ("brier", 2),
    ("auc", 2),
    ("average_precision", 2),
    ("roc", 2),
    ("pr_curve", 2),
    ("confusion", 2),
    ("classification_report", 2),
    ("at_threshold", 2),
    ("threshold_sweep", 2),
    ("calibration", 2),
];

pub fn namespace() -> Value {
    let mut module = ValueMap::default();
    for (name, arity) in FUNCTIONS {
        module.insert(
            name.to_string(),
            Value::Builtin(BuiltinFunction {
                name: format!("learn.{}", name),
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
    FUNCTIONS.iter().any(|(n, _)| *n == func)
}

// ---------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------

type R<T> = Result<T, String>;

fn e(err: olang_ods::OdsError) -> String {
    err.to_string()
}

fn tname(v: Option<&Value>) -> String {
    v.map(|v| v.type_name().to_string()).unwrap_or_else(|| "nothing".to_string())
}

fn want_frame<'a>(func: &str, args: &'a [Value], idx: usize) -> R<&'a Frame> {
    args.get(idx).and_then(frame_of).ok_or_else(|| {
        format!("learn.{}: argument {} must be a Frame, got {}", func, idx + 1, tname(args.get(idx)))
    })
}

fn want_series<'a>(func: &str, args: &'a [Value], idx: usize) -> R<&'a Series> {
    args.get(idx).and_then(series_of).ok_or_else(|| {
        format!("learn.{}: argument {} must be a Series, got {}", func, idx + 1, tname(args.get(idx)))
    })
}

fn want_model<'a>(func: &str, args: &'a [Value], idx: usize) -> R<&'a Arc<Model>> {
    args.get(idx).and_then(model_of).ok_or_else(|| {
        format!(
            "learn.{}: argument {} must be a fitted model (what learn.linear, learn.forest, ... return), got {}",
            func,
            idx + 1,
            tname(args.get(idx))
        )
    })
}

fn want_str(func: &str, args: &[Value], idx: usize) -> R<String> {
    match args.get(idx) {
        Some(Value::String(s)) => Ok(s.as_ref().clone()),
        other => Err(format!("learn.{}: argument {} must be a String, got {}", func, idx + 1, tname(other))),
    }
}

/// Column names: a String, a list of Strings, or `()` for "the rest".
fn names_arg(func: &str, v: Option<&Value>) -> R<Vec<String>> {
    match v {
        None | Some(Value::Unit) => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![s.as_ref().clone()]),
        Some(Value::List(items)) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.as_ref().clone()),
                other => Err(format!("learn.{}: column names must be Strings, got {}", func, other.type_name())),
            })
            .collect(),
        Some(other) => Err(format!(
            "learn.{}: columns must be a name, a list of names or () for every other column, got {}",
            func,
            other.type_name()
        )),
    }
}

fn map_value(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(Arc::new(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect()))
}

fn frame_value(f: Frame) -> Value {
    OdsFrame::into_value(f)
}

fn strings(v: &[String]) -> Value {
    Value::List(Arc::new(v.iter().map(|s| Value::String(Arc::new(s.clone()))).collect()))
}

fn int_series(idx: &[usize]) -> Series {
    Series::from_i64(idx.iter().map(|&i| i as i64).collect())
}

fn take(f: &Frame, idx: &[usize]) -> R<Frame> {
    f.take(&int_series(idx)).map_err(e)
}

/// Engine policy: whether the program allows parallelism at all. The
/// learning kernels then fan out past their own measured thresholds,
/// which count work (rows × features × trees ...), not rows: the
/// runtime's list threshold (10,000 elements under `olang run`) is
/// about per-element olang closures and would keep a 4,000-row kNN
/// query — 64 million distances — on one core.
fn par(_n: usize) -> bool {
    crate::parallel::should_parallelize(usize::MAX) && olang_ods::parallel_enabled()
}

/// A trailing options map, checked against the keys a function knows.
struct Opts<'a> {
    func: String,
    map: Option<&'a ValueMap>,
}

impl<'a> Opts<'a> {
    fn new(func: &str, v: Option<&'a Value>, allowed: &[&str]) -> R<Opts<'a>> {
        let map = match v {
            None | Some(Value::Unit) => None,
            Some(Value::Map(m)) => Some(m.as_ref()),
            Some(other) => {
                return Err(format!(
                    "learn.{}: the options must be a map (#{{...}}), got {}",
                    func,
                    other.type_name()
                ));
            }
        };
        if let Some(m) = map {
            for k in m.keys() {
                if !allowed.contains(&k.as_str()) {
                    return Err(format!(
                        "learn.{}: unknown option '{}' — the options are {}",
                        func,
                        k,
                        if allowed.is_empty() { "none".to_string() } else { allowed.join(", ") }
                    ));
                }
            }
        }
        Ok(Opts { func: func.to_string(), map })
    }

    fn get(&self, k: &str) -> Option<&'a Value> {
        self.map.and_then(|m| m.get(k)).filter(|v| !matches!(v, Value::Unit))
    }

    fn f64(&self, k: &str, d: f64) -> R<f64> {
        Ok(self.opt_f64(k)?.unwrap_or(d))
    }

    fn opt_f64(&self, k: &str) -> R<Option<f64>> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::Float(x)) => Ok(Some(*x)),
            Some(Value::Integer(x)) => Ok(Some(*x as f64)),
            Some(other) => Err(format!("learn.{}: option '{}' must be a number, got {}", self.func, k, other.type_name())),
        }
    }

    fn usize(&self, k: &str, d: usize) -> R<usize> {
        Ok(self.opt_usize(k)?.unwrap_or(d))
    }

    fn opt_usize(&self, k: &str) -> R<Option<usize>> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::Integer(x)) if *x >= 0 => Ok(Some(*x as usize)),
            Some(other) => Err(format!(
                "learn.{}: option '{}' must be a non-negative Int, got {}",
                self.func,
                k,
                match other {
                    Value::Integer(x) => x.to_string(),
                    o => o.type_name().to_string(),
                }
            )),
        }
    }

    fn bool(&self, k: &str, d: bool) -> R<bool> {
        match self.get(k) {
            None => Ok(d),
            Some(Value::Boolean(b)) => Ok(*b),
            Some(other) => Err(format!("learn.{}: option '{}' must be a Bool, got {}", self.func, k, other.type_name())),
        }
    }

    fn opt_str(&self, k: &str) -> R<Option<String>> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.as_ref().clone())),
            Some(other) => Err(format!("learn.{}: option '{}' must be a String, got {}", self.func, k, other.type_name())),
        }
    }

    fn str(&self, k: &str, d: &str) -> R<String> {
        Ok(self.opt_str(k)?.unwrap_or_else(|| d.to_string()))
    }

    fn one_of(&self, k: &str, d: &str, choices: &[&str]) -> R<String> {
        let v = self.str(k, d)?;
        if choices.contains(&v.as_str()) {
            Ok(v)
        } else {
            Err(format!(
                "learn.{}: option '{}' must be one of {}, got \"{}\"",
                self.func,
                k,
                choices.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", "),
                v
            ))
        }
    }

    /// The seed: given, or drawn from the `random` module's stream.
    fn seed(&self) -> R<u64> {
        match self.get("seed") {
            Some(Value::Integer(s)) => Ok(*s as u64),
            None => Ok(draw_seed()),
            Some(other) => Err(format!("learn.{}: option 'seed' must be an Int, got {}", self.func, other.type_name())),
        }
    }
}

fn draw_seed() -> u64 {
    let u = crate::stdlib::random::draw_uniforms(1)[0];
    (u * 9_007_199_254_740_992.0) as u64
}

/// The options map with its seed made explicit, so a spec refits the
/// same way.
fn with_seed(opts: Option<&Value>, seed: u64) -> Value {
    let mut m: ValueMap = match opts {
        Some(Value::Map(m)) => m.as_ref().clone(),
        _ => ValueMap::default(),
    };
    m.insert("seed".to_string(), Value::Integer(seed as i64));
    Value::Map(Arc::new(m))
}

// ---------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------

pub fn dispatch(func: &str, args: Vec<Value>) -> R<Value> {
    let min = FUNCTIONS
        .iter()
        .find(|(n, _)| *n == func)
        .map(|(_, a)| *a)
        .ok_or_else(|| format!("unknown learn function: learn.{}", func))?;
    let max = max_args(func);
    if args.len() < min || args.len() > max {
        return Err(if min == max {
            format!("learn.{} expects {} argument{}, got {}", func, min, if min == 1 { "" } else { "s" }, args.len())
        } else {
            format!("learn.{} expects {} to {} arguments, got {}", func, min, max, args.len())
        });
    }
    match func {
        "split" => split(&args),
        "kfold" => kfold(&args),
        "folds" => folds(&args),
        "time_splits" => time_splits(&args),
        "scaler" | "one_hot" | "imputer" => transformer(func, &args),
        "transform" => {
            let m = want_model(func, &args, 0)?;
            let f = want_frame(func, &args, 1)?;
            m.transform(f).map(frame_value).map_err(e)
        }
        "poly" => poly(&args),
        "linear" | "ridge" | "lasso" | "elastic_net" | "logistic" | "tree" | "forest" | "boost" | "knn"
        | "naive_bayes" => supervised(func, &args),
        "kmeans" | "gmm" | "pca" => unsupervised(func, &args),
        "silhouette" => silhouette(&args),
        "predict" => predict(&args),
        "importance" => {
            let m = want_model(func, &args, 0)?;
            m.importance().map(frame_value).map_err(e)
        }
        "summary" => {
            let m = want_model(func, &args, 0)?;
            let o = Opts::new("summary", args.get(1), &["level"])?;
            m.summary(o.f64("level", 0.95)?).map(frame_value).map_err(e)
        }
        "params" => {
            let m = want_model(func, &args, 0)?;
            Ok(params_value(m))
        }
        "score" => {
            let m = want_model(func, &args, 0)?;
            let f = want_frame(func, &args, 1)?;
            let metric = want_str(func, &args, 2)?;
            m.score(f, &metric, par(f.n_rows())).map(Value::Float).map_err(e)
        }
        "spec" => spec(&args),
        "fit" => {
            let f = want_frame(func, &args, 1)?;
            let s = spec_of(func, &args[0])?;
            fit_spec(&s, f)
        }
        "cross_validate" => cross_validate(&args),
        "permutation_importance" => permutation_importance(&args),
        "partial_dependence" => partial_dependence(&args),
        "learning_curve" => learning_curve(&args),
        "grid_search" => grid_search(&args),
        _ => metric(func, &args),
    }
}

/// The most arguments a function takes: its minimum plus a trailing
/// options map (and the optional middle arguments some have).
fn max_args(func: &str) -> usize {
    match func {
        "transform" | "params" | "importance" => FUNCTIONS.iter().find(|(n, _)| *n == func).unwrap().1,
        "split" | "scaler" | "one_hot" | "imputer" | "pca" => 3,
        "linear" | "ridge" | "lasso" | "elastic_net" | "logistic" | "tree" | "forest" | "boost" | "knn"
        | "naive_bayes" => 4,
        "spec" => 4,
        "fit" | "score" => FUNCTIONS.iter().find(|(n, _)| *n == func).unwrap().1,
        _ => FUNCTIONS.iter().find(|(n, _)| *n == func).unwrap().1 + 1,
    }
}

// ---------------------------------------------------------------------
// Splitting
// ---------------------------------------------------------------------

fn test_count(o: &Opts, n: usize) -> R<usize> {
    match o.get("test") {
        None => Ok((0.25 * n as f64).ceil() as usize),
        Some(Value::Float(f)) if *f > 0.0 && *f < 1.0 => Ok((f * n as f64).ceil() as usize),
        Some(Value::Integer(k)) if *k >= 0 && (*k as usize) <= n => Ok(*k as usize),
        Some(other) => Err(format!(
            "learn.{}: option 'test' must be a fraction in (0, 1) or a row count up to {}, got {}",
            o.func,
            n,
            match other {
                Value::Float(f) => f.to_string(),
                Value::Integer(i) => i.to_string(),
                v => v.type_name().to_string(),
            }
        )),
    }
}

/// Class codes of a stratification column.
fn strata(f: &Frame, col: &str) -> R<Vec<usize>> {
    let s = f.column(col).map_err(e)?;
    let (labels, codes) = Labels::learn(s).map_err(e)?;
    if codes.contains(&NO_CLASS) {
        // Nulls form their own stratum.
        let k = labels.len();
        return Ok(codes.into_iter().map(|c| if c == NO_CLASS { k } else { c }).collect());
    }
    Ok(codes)
}

fn split(args: &[Value]) -> R<Value> {
    let f = want_frame("split", args, 0)?;
    let o = Opts::new("split", args.get(1), &["test", "stratify", "shuffle", "seed"])?;
    let n = f.n_rows();
    let n_test = test_count(&o, n)?;
    let shuffle = o.bool("shuffle", true)?;
    let mut rng = Pcg::new(o.seed()?, 0);
    let st = match o.opt_str("stratify")? {
        Some(c) => Some(strata(f, &c)?),
        None => None,
    };
    let (tr, te) = preprocess::split(n, n_test, shuffle, st.as_deref(), &mut rng);
    Ok(map_value(vec![("train", frame_value(take(f, &tr)?)), ("test", frame_value(take(f, &te)?))]))
}

fn fold_ids(func: &str, args: &[Value]) -> R<(Vec<usize>, usize)> {
    let k = match args.get(1) {
        Some(Value::Integer(k)) if *k >= 2 => *k as usize,
        other => return Err(format!("learn.{}: k must be an Int >= 2, got {}", func, tname(other))),
    };
    let o = Opts::new(func, args.get(2), &["shuffle", "seed", "stratify"])?;
    let shuffle = o.bool("shuffle", true)?;
    let mut rng = Pcg::new(o.seed()?, 0);
    let (n, st) = match &args[0] {
        Value::Integer(n) if *n >= 0 => (*n as usize, None),
        v => {
            let f = frame_of(v).ok_or_else(|| {
                format!("learn.{}: argument 1 must be a Frame or a row count, got {}", func, v.type_name())
            })?;
            let st = match o.opt_str("stratify")? {
                Some(c) => Some(strata(f, &c)?),
                None => None,
            };
            (f.n_rows(), st)
        }
    };
    if o.opt_str("stratify")?.is_some() && st.is_none() {
        return Err(format!("learn.{}: 'stratify' needs a Frame, not a row count", func));
    }
    if k > n {
        return Err(format!("learn.{}: {} folds need at least {} rows, got {}", func, k, k, n));
    }
    Ok((
        match st {
            Some(s) => preprocess::stratified_kfold(&s, k, shuffle, &mut rng),
            None => preprocess::kfold(n, k, shuffle, &mut rng),
        },
        k,
    ))
}

fn kfold(args: &[Value]) -> R<Value> {
    let (ids, _) = fold_ids("kfold", args)?;
    Ok(make_series_value(int_series(&ids)))
}

fn folds(args: &[Value]) -> R<Value> {
    let f = want_frame("folds", args, 0)?;
    let (ids, k) = fold_ids("folds", args)?;
    let mut out = Vec::with_capacity(k);
    for fold in 0..k {
        let tr: Vec<usize> = (0..ids.len()).filter(|&i| ids[i] != fold).collect();
        let te: Vec<usize> = (0..ids.len()).filter(|&i| ids[i] == fold).collect();
        out.push(map_value(vec![
            ("fold", Value::Integer(fold as i64)),
            ("train", frame_value(take(f, &tr)?)),
            ("test", frame_value(take(f, &te)?)),
        ]));
    }
    Ok(Value::List(Arc::new(out)))
}

fn time_splits(args: &[Value]) -> R<Value> {
    let o = Opts::new("time_splits", args.get(2), &["test_size", "gap", "max_train"])?;
    let n_splits = match args.get(1) {
        Some(Value::Integer(k)) if *k >= 1 => *k as usize,
        other => return Err(format!("learn.time_splits: n_splits must be an Int >= 1, got {}", tname(other))),
    };
    let (n, frame) = match &args[0] {
        Value::Integer(n) if *n >= 0 => (*n as usize, None),
        v => {
            let f = frame_of(v).ok_or_else(|| {
                format!("learn.time_splits: argument 1 must be a Frame or a row count, got {}", v.type_name())
            })?;
            (f.n_rows(), Some(f))
        }
    };
    let splits = preprocess::time_splits(n, n_splits, o.opt_usize("test_size")?, o.usize("gap", 0)?, o.opt_usize("max_train")?)?;
    let mut out = Vec::new();
    for ((a, b), (c, d)) in splits {
        let tr: Vec<usize> = (a..b).collect();
        let te: Vec<usize> = (c..d).collect();
        out.push(match frame {
            Some(f) => map_value(vec![("train", frame_value(take(f, &tr)?)), ("test", frame_value(take(f, &te)?))]),
            None => map_value(vec![
                ("train", make_series_value(int_series(&tr))),
                ("test", make_series_value(int_series(&te))),
            ]),
        });
    }
    Ok(Value::List(Arc::new(out)))
}

// ---------------------------------------------------------------------
// Preprocessing
// ---------------------------------------------------------------------

fn transformer(func: &str, args: &[Value]) -> R<Value> {
    let f = want_frame(func, args, 0)?;
    let cols = names_arg(func, args.get(1))?;
    let m = match func {
        "scaler" => {
            let o = Opts::new(func, args.get(2), &["method"])?;
            lm::fit_scaler(f, &cols, &o.one_of("method", "standard", &["standard", "minmax", "robust"])?)
        }
        "one_hot" => {
            let o = Opts::new(func, args.get(2), &["drop_first"])?;
            lm::fit_one_hot(f, &cols, o.bool("drop_first", false)?)
        }
        _ => {
            let o = Opts::new(func, args.get(2), &["strategy", "fill"])?;
            let strategy = o.one_of("strategy", "mean", &["mean", "median", "most_frequent", "constant"])?;
            let fill = o.get("fill").and_then(value_to_scalar);
            lm::fit_imputer(f, &cols, &strategy, fill)
        }
    }
    .map_err(e)?;
    let spec = spec_value(func, Value::Unit, args.get(1).cloned().unwrap_or(Value::Unit), args.get(2).cloned().unwrap_or(Value::Unit));
    Ok(model_value(m, spec))
}

fn poly(args: &[Value]) -> R<Value> {
    let f = want_frame("poly", args, 0)?;
    let cols = names_arg("poly", args.get(1))?;
    let degree = match args.get(2) {
        Some(Value::Integer(d)) if *d >= 1 => *d as usize,
        other => return Err(format!("learn.poly: degree must be an Int >= 1, got {}", tname(other))),
    };
    let o = Opts::new("poly", args.get(3), &["interaction_only"])?;
    lm::poly(f, &cols, degree, o.bool("interaction_only", false)?).map(frame_value).map_err(e)
}

// ---------------------------------------------------------------------
// Supervised fits
// ---------------------------------------------------------------------

const COMMON: &[&str] = &["weights", "task", "class_weight"];

fn allowed(func: &str) -> Vec<&'static str> {
    let own: &[&str] = match func {
        "linear" => &["intercept"],
        "ridge" => &["alpha", "intercept"],
        "lasso" => &["alpha", "intercept", "max_iter", "tol"],
        "elastic_net" => &["alpha", "l1_ratio", "intercept", "max_iter", "tol"],
        "logistic" => &["penalty", "C", "max_iter", "tol"],
        "tree" => &["criterion", "max_depth", "min_samples_split", "min_samples_leaf", "max_features", "min_impurity_decrease", "seed"],
        "forest" => &[
            "criterion",
            "max_depth",
            "min_samples_split",
            "min_samples_leaf",
            "max_features",
            "min_impurity_decrease",
            "n_trees",
            "bootstrap",
            "max_samples",
            "oob",
            "seed",
        ],
        "boost" => &[
            "n_trees",
            "learning_rate",
            "max_depth",
            "max_leaf_nodes",
            "min_samples_leaf",
            "l2",
            "subsample",
            "max_bins",
            "early_stopping",
            "validation_fraction",
            "n_iter_no_change",
            "tol",
            "seed",
        ],
        "knn" => &["k", "weights_by"],
        "naive_bayes" => &["var_smoothing"],
        _ => &[],
    };
    COMMON.iter().chain(own).copied().collect()
}

fn tree_opts(o: &Opts) -> R<TreeOpts> {
    let criterion = match o.opt_str("criterion")?.as_deref() {
        None => None,
        Some("gini") => Some(Criterion::Gini),
        Some("entropy") => Some(Criterion::Entropy),
        Some("squared_error") | Some("mse") => Some(Criterion::Mse),
        Some(other) => {
            return Err(format!(
                "learn.{}: criterion must be \"gini\", \"entropy\" or \"squared_error\", got \"{}\"",
                o.func, other
            ));
        }
    };
    let max_features = match o.get("max_features") {
        None => MaxFeatures::Default,
        Some(Value::String(s)) => match s.as_str() {
            "sqrt" => MaxFeatures::Sqrt,
            "log2" => MaxFeatures::Log2,
            "all" => MaxFeatures::All,
            other => {
                return Err(format!(
                    "learn.{}: max_features must be \"sqrt\", \"log2\", \"all\", an Int or a fraction, got \"{}\"",
                    o.func, other
                ));
            }
        },
        Some(Value::Integer(k)) if *k >= 1 => MaxFeatures::Count(*k as usize),
        Some(Value::Float(f)) if *f > 0.0 && *f <= 1.0 => MaxFeatures::Fraction(*f),
        Some(other) => return Err(format!("learn.{}: bad max_features {}", o.func, other.type_name())),
    };
    Ok(TreeOpts {
        criterion,
        max_depth: o.opt_usize("max_depth")?,
        min_samples_split: o.usize("min_samples_split", 2)?,
        min_samples_leaf: o.usize("min_samples_leaf", 1)?,
        max_features,
        min_impurity_decrease: o.f64("min_impurity_decrease", 0.0)?,
    })
}

fn class_weight(o: &Opts) -> R<ClassWeight> {
    match o.get("class_weight") {
        None => Ok(ClassWeight::None),
        Some(Value::String(s)) if s.as_str() == "balanced" => Ok(ClassWeight::Balanced),
        Some(Value::Map(m)) => m
            .iter()
            .map(|(k, v)| match v {
                Value::Float(x) => Ok((k.clone(), *x)),
                Value::Integer(x) => Ok((k.clone(), *x as f64)),
                other => Err(format!("learn.{}: class weights must be numbers, got {}", o.func, other.type_name())),
            })
            .collect::<R<Vec<_>>>()
            .map(ClassWeight::Given),
        Some(other) => Err(format!(
            "learn.{}: class_weight must be \"balanced\" or a map from class to weight, got {}",
            o.func,
            other.type_name()
        )),
    }
}

fn supervised(func: &str, args: &[Value]) -> R<Value> {
    let f = want_frame(func, args, 0)?;
    let y = want_str(func, args, 1)?;
    let x = names_arg(func, args.get(2))?;
    let keys = allowed(func);
    let o = Opts::new(func, args.get(3), &keys)?;
    let task = match o.opt_str("task")?.as_deref() {
        None => TaskHint::Auto,
        Some("regression") => TaskHint::Regression,
        Some("classification") => TaskHint::Classification,
        Some(other) => {
            return Err(format!(
                "learn.{}: task must be \"regression\" or \"classification\", got \"{}\"",
                func, other
            ));
        }
    };
    let common = Common {
        y: y.clone(),
        x,
        weights: o.opt_str("weights")?,
        task,
        class_weight: class_weight(&o)?,
    };
    let needs_seed = matches!(func, "tree" | "forest" | "boost");
    let seed = if needs_seed { o.seed()? } else { 0 };
    let intercept = o.bool("intercept", true)?;
    let algo = match func {
        "linear" => Algo::Linear { intercept },
        "ridge" => Algo::Ridge {
            alpha: o.f64("alpha", 1.0)?,
            intercept,
        },
        "lasso" | "elastic_net" => Algo::ElasticNet {
            alpha: o.f64("alpha", 1.0)?,
            l1_ratio: if func == "lasso" { 1.0 } else { o.f64("l1_ratio", 0.5)? },
            intercept,
            max_iter: o.usize("max_iter", 10_000)?,
            tol: o.f64("tol", 1e-8)?,
        },
        "logistic" => {
            let lambda = match (o.opt_f64("penalty")?, o.opt_f64("C")?) {
                (Some(_), Some(_)) => return Err("learn.logistic: give 'penalty' or 'C', not both".to_string()),
                (Some(l), None) => l,
                (None, Some(c)) if c > 0.0 => 1.0 / c,
                (None, Some(_)) => return Err("learn.logistic: C must be > 0".to_string()),
                (None, None) => 0.0,
            };
            Algo::Logistic {
                lambda,
                max_iter: o.usize("max_iter", 100)?,
                tol: o.f64("tol", 1e-10)?,
            }
        }
        "tree" => Algo::Tree { opts: tree_opts(&o)?, seed },
        "forest" => Algo::Forest {
            opts: tree_opts(&o)?,
            n_trees: o.usize("n_trees", 100)?,
            bootstrap: o.bool("bootstrap", true)?,
            max_samples: o.opt_f64("max_samples")?,
            oob: o.bool("oob", false)?,
            seed,
        },
        "boost" => {
            let mut p: BoostParams = default_boost(seed);
            p.n_trees = o.usize("n_trees", 100)?;
            p.learning_rate = o.f64("learning_rate", 0.1)?;
            p.tree = HistParams {
                max_depth: match o.get("max_depth") {
                    None => Some(3),
                    Some(_) => o.opt_usize("max_depth")?,
                },
                max_leaf_nodes: o.opt_usize("max_leaf_nodes")?,
                min_samples_leaf: o.usize("min_samples_leaf", 1)?.max(1),
                l2: o.f64("l2", 0.0)?,
                min_hessian: 1e-3,
            };
            p.subsample = o.f64("subsample", 1.0)?;
            if !(p.subsample > 0.0 && p.subsample <= 1.0) {
                return Err("learn.boost: subsample must lie in (0, 1]".to_string());
            }
            p.max_bins = o.usize("max_bins", 255)?.clamp(2, 256);
            p.early_stopping = o.bool("early_stopping", false)?;
            p.validation_fraction = o.f64("validation_fraction", 0.1)?;
            p.n_iter_no_change = o.usize("n_iter_no_change", 10)?;
            p.tol = o.f64("tol", 1e-7)?;
            Algo::Boost { params: p }
        }
        "knn" => Algo::Knn {
            k: o.usize("k", 5)?.max(1),
            distance: o.one_of("weights_by", "uniform", &["uniform", "distance"])? == "distance",
        },
        "naive_bayes" => Algo::NaiveBayes {
            var_smoothing: o.f64("var_smoothing", 1e-9)?,
        },
        _ => unreachable!(),
    };
    let model = lm::fit(f, &common, &algo, par(f.n_rows())).map_err(e)?;
    let opts = if needs_seed { with_seed(args.get(3), seed) } else { args.get(3).cloned().unwrap_or(Value::Unit) };
    let spec = spec_value(func, Value::String(Arc::new(y)), args.get(2).cloned().unwrap_or(Value::Unit), opts);
    Ok(model_value(model, spec))
}

fn unsupervised(func: &str, args: &[Value]) -> R<Value> {
    let f = want_frame(func, args, 0)?;
    let cols = names_arg(func, args.get(1))?;
    let p = par(f.n_rows());
    let (model, opts) = match func {
        "pca" => {
            let o = Opts::new(func, args.get(2), &["n_components", "scale"])?;
            (
                lm::fit_pca(f, &cols, o.opt_usize("n_components")?, o.bool("scale", false)?, p),
                args.get(2).cloned().unwrap_or(Value::Unit),
            )
        }
        _ => {
            let k = match args.get(2) {
                Some(Value::Integer(k)) if *k >= 1 => *k as usize,
                other => return Err(format!("learn.{}: k must be an Int >= 1, got {}", func, tname(other))),
            };
            if func == "kmeans" {
                let o = Opts::new(func, args.get(3), &["n_init", "max_iter", "tol", "seed"])?;
                let seed = o.seed()?;
                (
                    lm::fit_kmeans(
                        f,
                        &cols,
                        &KMeansParams {
                            k,
                            n_init: o.usize("n_init", 10)?,
                            max_iter: o.usize("max_iter", 300)?,
                            tol: o.f64("tol", 1e-4)?,
                            seed,
                        },
                        p,
                    ),
                    with_k(with_seed(args.get(3), seed), k),
                )
            } else {
                let o = Opts::new(func, args.get(3), &["max_iter", "tol", "reg_covar", "seed"])?;
                let seed = o.seed()?;
                (
                    lm::fit_gmm(
                        f,
                        &cols,
                        &GmmParams {
                            k,
                            max_iter: o.usize("max_iter", 100)?,
                            tol: o.f64("tol", 1e-3)?,
                            reg_covar: o.f64("reg_covar", 1e-6)?,
                            seed,
                        },
                        p,
                    ),
                    with_k(with_seed(args.get(3), seed), k),
                )
            }
        }
    };
    let model = model.map_err(e)?;
    let spec = spec_value(func, Value::Unit, args.get(1).cloned().unwrap_or(Value::Unit), opts);
    Ok(model_value(model, spec))
}

fn with_k(opts: Value, k: usize) -> Value {
    let mut m = match &opts {
        Value::Map(m) => m.as_ref().clone(),
        _ => ValueMap::default(),
    };
    m.insert("k".to_string(), Value::Integer(k as i64));
    Value::Map(Arc::new(m))
}

fn silhouette(args: &[Value]) -> R<Value> {
    let f = want_frame("silhouette", args, 0)?;
    let cols = names_arg("silhouette", args.get(1))?;
    let labels = want_series("silhouette", args, 2)?;
    let o = Opts::new("silhouette", args.get(3), &["sample", "seed"])?;
    if labels.len() != f.n_rows() {
        return Err(format!("learn.silhouette: {} labels for {} rows", labels.len(), f.n_rows()));
    }
    let cols = if cols.is_empty() { f.names().to_vec() } else { cols };
    let design = olang_ods::learn::Design::learn(f, &cols, false).map_err(e)?;
    let (x, mut valid) = design.encode(f).map_err(e)?;
    let (_, codes) = Labels::learn(labels).map_err(e)?;
    for (v, c) in valid.iter_mut().zip(&codes) {
        *v &= *c != NO_CLASS;
    }
    let mut rows: Vec<usize> = (0..f.n_rows()).filter(|&i| valid[i]).collect();
    if let Some(m) = o.opt_usize("sample")?
        && m < rows.len()
    {
        let mut rng = Pcg::new(o.seed()?, 0);
        let pick = rng.choose(rows.len(), m);
        let mut chosen: Vec<usize> = pick.into_iter().map(|p| rows[p]).collect();
        chosen.sort_unstable();
        rows = chosen;
    }
    if rows.len() < 2 {
        return Err("learn.silhouette: needs at least two labelled rows".to_string());
    }
    Ok(Value::Float(olang_ods::learn::cluster::silhouette(&x, &codes, &rows, par(rows.len()))))
}

// ---------------------------------------------------------------------
// The model as a value
// ---------------------------------------------------------------------

fn spec_value(kind: &str, y: Value, x: Value, opts: Value) -> Value {
    map_value(vec![
        ("kind", Value::String(Arc::new(kind.to_string()))),
        ("y", y),
        ("x", x),
        ("opts", opts),
    ])
}

fn params_value(m: &Model) -> Value {
    match m.params() {
        Some(p) => Value::Map(Arc::new(p.into_iter().map(|(k, v)| (k, Value::Float(v))).collect())),
        None => Value::Map(Arc::new(ValueMap::default())),
    }
}

fn floats(v: &[f64]) -> Value {
    make_series_value(Series::from_f64(v.to_vec()))
}

fn model_value(model: Model, spec: Value) -> Value {
    use olang_ods::learn::Body;
    let m = Arc::new(model);
    let mut out: Vec<(&str, Value)> = vec![
        ("model", Value::Native(NativeHandle::new(LearnModel { model: m.clone() }))),
        ("kind", Value::String(Arc::new(m.algo.to_string()))),
    ];
    if let Some(y) = &m.y {
        out.push(("y", Value::String(Arc::new(y.clone()))));
    }
    out.push(("inputs", strings(&m.inputs())));
    out.push(("features", strings(&m.design.features)));
    if let Some(l) = m.classes() {
        out.push(("classes", strings(&l.names())));
    }
    out.push(("n", Value::Integer(m.n as i64)));
    out.push(("dropped", Value::Integer(m.dropped as i64)));
    let summary = |level: f64| m.summary(level).map(frame_value).unwrap_or(Value::Unit);
    let n = m.n as f64;
    match &m.body {
        Body::Ols(f) => {
            let p = f.coef.len() as f64;
            out.push(("terms", strings(&m.terms())));
            out.push(("params", params_value(&m)));
            out.push(("coef", summary(0.95)));
            out.push(("r2", Value::Float(f.r2)));
            out.push(("adj_r2", Value::Float(f.adj_r2)));
            out.push(("sigma", Value::Float(f.sigma2.sqrt())));
            out.push(("log_lik", Value::Float(f.log_lik)));
            out.push(("aic", Value::Float(-2.0 * f.log_lik + 2.0 * p)));
            out.push(("bic", Value::Float(-2.0 * f.log_lik + p * n.ln())));
            out.push(("df_resid", Value::Float(f.df_resid)));
            out.push(("fitted", floats(&f.fitted)));
        }
        Body::Penalized { n_iter, .. } => {
            out.push(("terms", strings(&m.terms())));
            out.push(("params", params_value(&m)));
            out.push(("coef", summary(0.95)));
            out.push(("n_iter", Value::Integer(*n_iter as i64)));
        }
        Body::Logit(f) => {
            let p = f.coef.len() as f64;
            out.push(("terms", strings(&m.terms())));
            out.push(("params", params_value(&m)));
            out.push(("coef", summary(0.95)));
            out.push(("log_lik", Value::Float(f.log_lik)));
            out.push(("null_log_lik", Value::Float(f.null_log_lik)));
            out.push(("pseudo_r2", Value::Float(1.0 - f.log_lik / f.null_log_lik)));
            out.push(("aic", Value::Float(-2.0 * f.log_lik + 2.0 * p)));
            out.push(("bic", Value::Float(-2.0 * f.log_lik + p * n.ln())));
            out.push(("n_iter", Value::Integer(f.n_iter as i64)));
            out.push(("converged", Value::Boolean(f.converged)));
        }
        Body::Softmax(f) => {
            out.push(("terms", strings(&m.terms())));
            out.push(("coef", summary(0.95)));
            out.push(("log_lik", Value::Float(f.log_lik)));
            out.push(("n_iter", Value::Integer(f.n_iter as i64)));
            out.push(("converged", Value::Boolean(f.converged)));
        }
        Body::Tree(t) => {
            out.push(("depth", Value::Integer(t.depth() as i64)));
            out.push(("n_leaves", Value::Integer(t.n_leaves() as i64)));
            out.push(("n_nodes", Value::Integer(t.n_nodes() as i64)));
            out.push(("importance", summary(0.95)));
        }
        Body::Forest(fr) => {
            out.push(("n_trees", Value::Integer(fr.trees.len() as i64)));
            out.push(("importance", summary(0.95)));
            if let Some(score) = fr.oob_score {
                out.push(("oob_score", Value::Float(score)));
            }
        }
        Body::Boost(b) => {
            out.push(("n_trees", Value::Integer(b.trees.len() as i64)));
            out.push(("best_iter", Value::Integer(b.best_iter as i64)));
            out.push(("train_loss", floats(&b.train_loss)));
            if !b.valid_loss.is_empty() {
                out.push(("valid_loss", floats(&b.valid_loss)));
            }
            out.push(("importance", summary(0.95)));
        }
        Body::Knn { knn, .. } => {
            out.push(("k", Value::Integer(knn.k as i64)));
        }
        Body::NaiveBayes(nb) => {
            let names = m.classes().map(|l| l.names()).unwrap_or_default();
            out.push((
                "priors",
                Value::Map(Arc::new(names.into_iter().zip(&nb.prior).map(|(k, v)| (k, Value::Float(*v))).collect())),
            ));
            out.push(("table", summary(0.95)));
        }
        Body::KMeans(k) => {
            out.push(("k", Value::Integer(k.k as i64)));
            out.push(("labels", make_series_value(Series::from_i64(k.labels.iter().map(|&l| l as i64).collect()))));
            out.push(("centers", summary(0.95)));
            out.push(("inertia", Value::Float(k.inertia)));
            out.push(("n_iter", Value::Integer(k.n_iter as i64)));
        }
        Body::Pca(p) => {
            out.push(("n_components", Value::Integer(p.n_components as i64)));
            out.push(("explained_variance", floats(&p.explained_variance)));
            out.push(("explained_variance_ratio", floats(&p.explained_variance_ratio)));
            out.push(("components", summary(0.95)));
        }
        Body::Gmm(g) => {
            let pcount = g.n_parameters() as f64;
            let ll = g.log_likelihood * n;
            out.push(("k", Value::Integer(g.k as i64)));
            out.push(("means", summary(0.95)));
            out.push(("weights", floats(&g.weights)));
            out.push(("log_lik", Value::Float(ll)));
            out.push(("aic", Value::Float(-2.0 * ll + 2.0 * pcount)));
            out.push(("bic", Value::Float(-2.0 * ll + pcount * n.ln())));
            out.push(("n_iter", Value::Integer(g.n_iter as i64)));
            out.push(("converged", Value::Boolean(g.converged)));
        }
        Body::Scaler { .. } | Body::OneHot | Body::Imputer { .. } => {
            out.push(("table", summary(0.95)));
        }
    }
    out.push(("spec", spec));
    map_value(out)
}

// ---------------------------------------------------------------------
// Prediction
// ---------------------------------------------------------------------

fn predict(args: &[Value]) -> R<Value> {
    let m = want_model("predict", args, 0)?;
    let f = want_frame("predict", args, 1)?;
    let o = Opts::new("predict", args.get(2), &["kind", "level"])?;
    let level = o.f64("level", 0.95)?;
    if !(level > 0.0 && level < 1.0) {
        return Err("learn.predict: level must lie in (0, 1)".to_string());
    }
    let kind = match o.str("kind", "auto")?.as_str() {
        "auto" | "class" | "value" | "response" | "label" => PredictKind::Auto,
        "prob" => PredictKind::Prob,
        "probs" => PredictKind::ProbFrame,
        "link" => PredictKind::Link,
        "confidence" => PredictKind::Interval(false, level),
        "prediction" => PredictKind::Interval(true, level),
        other => {
            return Err(format!(
                "learn.predict: kind must be \"auto\", \"class\", \"prob\", \"probs\", \"link\", \"confidence\" or \"prediction\", got \"{}\"",
                other
            ));
        }
    };
    match m.predict(f, kind, par(f.n_rows())).map_err(e)? {
        Output::Series(s) => Ok(make_series_value(s)),
        Output::Frame(f) => Ok(frame_value(f)),
    }
}

// ---------------------------------------------------------------------
// Specs and refitting
// ---------------------------------------------------------------------

const KINDS: &[&str] = &[
    "linear",
    "ridge",
    "lasso",
    "elastic_net",
    "logistic",
    "tree",
    "forest",
    "boost",
    "knn",
    "naive_bayes",
    "kmeans",
    "gmm",
    "pca",
    "scaler",
    "one_hot",
    "imputer",
];

/// `learn.spec(kind, y, x?, opts?)`: how to fit, without fitting. The
/// seed is fixed now, so every refit of the spec draws the same.
fn spec(args: &[Value]) -> R<Value> {
    let kind = want_str("spec", args, 0)?;
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("learn.spec: unknown kind \"{}\" — the kinds are {}", kind, KINDS.join(", ")));
    }
    let y = args.get(1).cloned().unwrap_or(Value::Unit);
    let x = args.get(2).cloned().unwrap_or(Value::Unit);
    let opts = args.get(3).cloned().unwrap_or(Value::Unit);
    let needs_seed = matches!(kind.as_str(), "tree" | "forest" | "boost" | "kmeans" | "gmm");
    let opts = if needs_seed {
        let has = matches!(&opts, Value::Map(m) if m.get("seed").is_some_and(|v| !matches!(v, Value::Unit)));
        if has { opts } else { with_seed(Some(&opts), draw_seed()) }
    } else {
        opts
    };
    Ok(spec_value(&kind, y, x, opts))
}

struct Spec {
    kind: String,
    y: Value,
    x: Value,
    opts: Value,
}

fn spec_of(func: &str, v: &Value) -> R<Spec> {
    let m = match v {
        Value::Map(m) if m.get("model").is_some() => match m.get("spec") {
            Some(Value::Map(s)) => s.clone(),
            _ => return Err(format!("learn.{}: the model carries no spec", func)),
        },
        Value::Map(m) if m.get("kind").is_some() => m.clone(),
        other => {
            return Err(format!(
                "learn.{}: argument 1 must be a model or a spec (learn.spec), got {}",
                func,
                other.type_name()
            ));
        }
    };
    let kind = match m.get("kind") {
        Some(Value::String(s)) => s.as_ref().clone(),
        _ => return Err(format!("learn.{}: a spec's kind must be a String", func)),
    };
    Ok(Spec {
        kind,
        y: m.get("y").cloned().unwrap_or(Value::Unit),
        x: m.get("x").cloned().unwrap_or(Value::Unit),
        opts: m.get("opts").cloned().unwrap_or(Value::Unit),
    })
}

fn fit_spec(s: &Spec, f: &Frame) -> R<Value> {
    let fv = frame_value(f.clone());
    match s.kind.as_str() {
        "kmeans" | "gmm" => {
            let k = match &s.opts {
                Value::Map(m) => m.get("k").cloned(),
                _ => None,
            }
            .ok_or_else(|| format!("learn.fit: a {} spec needs k in its options", s.kind))?;
            let mut o = match &s.opts {
                Value::Map(m) => m.as_ref().clone(),
                _ => ValueMap::default(),
            };
            o.shift_remove("k");
            dispatch(&s.kind, vec![fv, s.x.clone(), k, Value::Map(Arc::new(o))])
        }
        "pca" | "scaler" | "one_hot" | "imputer" => dispatch(&s.kind, vec![fv, s.x.clone(), s.opts.clone()]),
        _ => dispatch(&s.kind, vec![fv, s.y.clone(), s.x.clone(), s.opts.clone()]),
    }
}

fn with_opts(s: &Spec, overrides: &[(String, Value)]) -> Spec {
    let mut m = match &s.opts {
        Value::Map(m) => m.as_ref().clone(),
        _ => ValueMap::default(),
    };
    for (k, v) in overrides {
        m.insert(k.clone(), v.clone());
    }
    Spec {
        kind: s.kind.clone(),
        y: s.y.clone(),
        x: s.x.clone(),
        opts: Value::Map(Arc::new(m)),
    }
}

fn fitted(v: &Value) -> R<Arc<Model>> {
    model_of(v).cloned().ok_or_else(|| "learn: the fit returned no model".to_string())
}

// ---------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------

fn spec_seed(s: &Spec) -> u64 {
    match &s.opts {
        Value::Map(m) => match m.get("seed") {
            Some(Value::Integer(v)) => *v as u64,
            _ => 0,
        },
        _ => 0,
    }
}

/// The fold ids cross-validation uses: stratified by the target for a
/// classifier, shuffled with `seed`.
fn cv_folds(s: &Spec, f: &Frame, k: usize, seed: u64, shuffle: bool) -> R<Vec<usize>> {
    let mut rng = Pcg::new(seed, 0);
    let classify = classification_spec(s, f);
    if classify && let Value::String(y) = &s.y {
        return Ok(preprocess::stratified_kfold(&strata(f, y)?, k, shuffle, &mut rng));
    }
    Ok(preprocess::kfold(f.n_rows(), k, shuffle, &mut rng))
}

fn classification_spec(s: &Spec, f: &Frame) -> bool {
    let Value::String(y) = &s.y else { return false };
    let task = match &s.opts {
        Value::Map(m) => match m.get("task") {
            Some(Value::String(t)) => Some(t.as_ref().clone()),
            _ => None,
        },
        _ => None,
    };
    match task.as_deref() {
        Some("classification") => return true,
        Some("regression") => return false,
        _ => {}
    }
    match s.kind.as_str() {
        "logistic" | "naive_bayes" => true,
        "linear" | "ridge" | "lasso" | "elastic_net" => false,
        _ => f.column(y).is_ok_and(|c| !matches!(c, Series::F64 { .. })),
    }
}

fn default_metrics(s: &Spec, f: &Frame) -> Vec<String> {
    if classification_spec(s, f) {
        let binary = match &s.y {
            Value::String(y) => f.column(y).ok().and_then(|c| Labels::learn(c).ok()).is_some_and(|(l, _)| l.len() == 2),
            _ => false,
        };
        if binary {
            vec!["accuracy".to_string(), "auc".to_string()]
        } else {
            vec!["accuracy".to_string()]
        }
    } else {
        vec!["rmse".to_string(), "r2".to_string()]
    }
}

fn metric_list(o: &Opts, s: &Spec, f: &Frame) -> R<Vec<String>> {
    let ms = match o.get("metrics").or(o.get("metric")) {
        None => default_metrics(s, f),
        Some(v) => names_arg(&o.func, Some(v))?,
    };
    for m in &ms {
        lm::greater_is_better(m).map_err(e)?;
    }
    Ok(ms)
}

/// Run `jobs` (independent fits) in parallel when allowed; results in
/// job order either way.
fn run_jobs<T: Send>(n_jobs: usize, rows: usize, job: impl Fn(usize) -> R<T> + Sync + Send) -> R<Vec<T>> {
    #[cfg(feature = "native")]
    {
        if par(rows) && n_jobs > 1 {
            return (0..n_jobs).into_par_iter().map(job).collect();
        }
    }
    let _ = rows;
    (0..n_jobs).map(job).collect()
}

fn cross_validate(args: &[Value]) -> R<Value> {
    let s = spec_of("cross_validate", &args[0])?;
    let f = want_frame("cross_validate", args, 1)?;
    let o = Opts::new("cross_validate", args.get(2), &["folds", "metrics", "metric", "seed", "shuffle"])?;
    let k = o.usize("folds", 5)?;
    if k < 2 || k > f.n_rows() {
        return Err(format!("learn.cross_validate: folds must be between 2 and the row count, got {}", k));
    }
    let ms = metric_list(&o, &s, f)?;
    let seed = match o.get("seed") {
        Some(_) => o.seed()?,
        None => splitmix64(spec_seed(&s) ^ 0xC0FFEE),
    };
    let ids = cv_folds(&s, f, k, seed, o.bool("shuffle", true)?)?;
    let rows = f.n_rows();
    let results = run_jobs(k, rows, |fold| {
        let tr: Vec<usize> = (0..rows).filter(|&i| ids[i] != fold).collect();
        let te: Vec<usize> = (0..rows).filter(|&i| ids[i] == fold).collect();
        let train = take(f, &tr)?;
        let test = take(f, &te)?;
        let m = fitted(&fit_spec(&s, &train)?)?;
        let scores = ms
            .iter()
            .map(|metric| m.score(&test, metric, false).map_err(e))
            .collect::<R<Vec<f64>>>()?;
        Ok((tr.len(), te.len(), scores))
    })?;
    let mut cols: Vec<(String, Series)> = vec![
        ("fold".to_string(), Series::from_i64((0..k as i64).collect())),
        ("n_train".to_string(), Series::from_i64(results.iter().map(|r| r.0 as i64).collect())),
        ("n_test".to_string(), Series::from_i64(results.iter().map(|r| r.1 as i64).collect())),
    ];
    for (j, m) in ms.iter().enumerate() {
        cols.push((m.clone(), Series::from_f64(results.iter().map(|r| r.2[j]).collect())));
    }
    Frame::new(cols).map(frame_value).map_err(e)
}

fn permutation_importance(args: &[Value]) -> R<Value> {
    let m = want_model("permutation_importance", args, 0)?;
    let f = want_frame("permutation_importance", args, 1)?;
    let o = Opts::new("permutation_importance", args.get(2), &["metric", "repeats", "seed"])?;
    let metric = match o.opt_str("metric")? {
        Some(mm) => mm,
        None => match m.classes() {
            Some(l) if l.len() == 2 => "auc".to_string(),
            Some(_) => "accuracy".to_string(),
            None => "r2".to_string(),
        },
    };
    let repeats = o.usize("repeats", 5)?.max(1);
    m.permutation_importance(f, &metric, repeats, o.seed()?, par(f.n_rows()))
        .map(frame_value)
        .map_err(e)
}

fn partial_dependence(args: &[Value]) -> R<Value> {
    let m = want_model("partial_dependence", args, 0)?;
    let f = want_frame("partial_dependence", args, 1)?;
    let feature = want_str("partial_dependence", args, 2)?;
    let o = Opts::new("partial_dependence", args.get(3), &["grid", "points", "sample", "seed"])?;
    let grid = match o.get("grid") {
        None => None,
        Some(Value::List(items)) => Some(
            items
                .iter()
                .map(|v| value_to_scalar(v).ok_or_else(|| "learn.partial_dependence: grid values must be scalars".to_string()))
                .collect::<R<Vec<Scalar>>>()?,
        ),
        Some(v) => match series_of(v) {
            Some(s) => Some((0..s.len()).map(|i| s.scalar_at(i)).collect()),
            None => return Err("learn.partial_dependence: grid must be a list or Series".to_string()),
        },
    };
    let frame = match o.opt_usize("sample")? {
        Some(n) if n < f.n_rows() => {
            let mut idx = Pcg::new(o.seed()?, 0).choose(f.n_rows(), n);
            idx.sort_unstable();
            take(f, &idx)?
        }
        _ => f.clone(),
    };
    m.partial_dependence(&frame, &feature, grid, o.usize("points", 50)?.max(2), par(frame.n_rows()))
        .map(frame_value)
        .map_err(e)
}

fn learning_curve(args: &[Value]) -> R<Value> {
    let s = spec_of("learning_curve", &args[0])?;
    let f = want_frame("learning_curve", args, 1)?;
    let o = Opts::new("learning_curve", args.get(2), &["sizes", "folds", "metric", "seed"])?;
    let sizes: Vec<f64> = match o.get("sizes") {
        None => vec![0.1, 0.325, 0.55, 0.775, 1.0],
        Some(Value::List(items)) => items
            .iter()
            .map(|v| match v {
                Value::Float(x) if *x > 0.0 && *x <= 1.0 => Ok(*x),
                Value::Integer(x) if *x >= 1 => Ok(*x as f64),
                _ => Err("learn.learning_curve: sizes are fractions in (0, 1] or row counts".to_string()),
            })
            .collect::<R<_>>()?,
        Some(_) => return Err("learn.learning_curve: sizes must be a list".to_string()),
    };
    let k = o.usize("folds", 5)?;
    let metric = match o.opt_str("metric")? {
        Some(mm) => mm,
        None => default_metrics(&s, f)[0].clone(),
    };
    lm::greater_is_better(&metric).map_err(e)?;
    let seed = match o.get("seed") {
        Some(_) => o.seed()?,
        None => splitmix64(spec_seed(&s) ^ 0x1EA2),
    };
    let ids = cv_folds(&s, f, k, seed, true)?;
    let rows = f.n_rows();
    let jobs: Vec<(usize, usize)> = (0..sizes.len()).flat_map(|si| (0..k).map(move |fold| (si, fold))).collect();
    let results = run_jobs(jobs.len(), rows, |j| {
        let (si, fold) = jobs[j];
        // Shuffled train order, so a prefix is a random subset.
        let mut tr: Vec<usize> = Pcg::new(seed, 2 + fold as u64).permutation(rows).into_iter().filter(|&i| ids[i] != fold).collect();
        let te: Vec<usize> = (0..rows).filter(|&i| ids[i] == fold).collect();
        let want = if sizes[si] <= 1.0 { (sizes[si] * tr.len() as f64).round() as usize } else { sizes[si] as usize };
        tr.truncate(want.clamp(2, tr.len()));
        tr.sort_unstable();
        let train = take(f, &tr)?;
        let test = take(f, &te)?;
        let m = fitted(&fit_spec(&s, &train)?)?;
        Ok((tr.len(), m.score(&train, &metric, false).map_err(e)?, m.score(&test, &metric, false).map_err(e)?))
    })?;
    let mut n_train = Vec::new();
    let (mut trm, mut trs, mut tem, mut tes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for si in 0..sizes.len() {
        let part: Vec<&(usize, f64, f64)> = results.iter().enumerate().filter(|(j, _)| jobs[*j].0 == si).map(|(_, r)| r).collect();
        n_train.push((part.iter().map(|r| r.0).sum::<usize>() / part.len()) as i64);
        let (m1, s1) = mean_sd(&part.iter().map(|r| r.1).collect::<Vec<_>>());
        let (m2, s2) = mean_sd(&part.iter().map(|r| r.2).collect::<Vec<_>>());
        trm.push(m1);
        trs.push(s1);
        tem.push(m2);
        tes.push(s2);
    }
    Frame::new(vec![
        ("n_train".to_string(), Series::from_i64(n_train)),
        ("train".to_string(), Series::from_f64(trm)),
        ("train_sd".to_string(), Series::from_f64(trs)),
        ("test".to_string(), Series::from_f64(tem)),
        ("test_sd".to_string(), Series::from_f64(tes)),
    ])
    .map(frame_value)
    .map_err(e)
}

fn mean_sd(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    let m = v.iter().sum::<f64>() / n;
    let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n).sqrt();
    (m, sd)
}

fn grid_search(args: &[Value]) -> R<Value> {
    let s = spec_of("grid_search", &args[0])?;
    let f = want_frame("grid_search", args, 1)?;
    let grid = match &args[2] {
        Value::Map(m) => m.clone(),
        other => return Err(format!("learn.grid_search: the grid must be a map of option to values, got {}", other.type_name())),
    };
    let o = Opts::new("grid_search", args.get(3), &["folds", "metric", "seed"])?;
    let k = o.usize("folds", 5)?;
    let metric = match o.opt_str("metric")? {
        Some(mm) => mm,
        None => default_metrics(&s, f)[0].clone(),
    };
    let higher = lm::greater_is_better(&metric).map_err(e)?;
    // The Cartesian product, first key varying slowest.
    let keys: Vec<String> = grid.keys().cloned().collect();
    let values: Vec<Vec<Value>> = grid
        .values()
        .map(|v| match v {
            Value::List(items) if !items.is_empty() => Ok(items.as_ref().clone()),
            other => Err(format!("learn.grid_search: each grid entry must be a non-empty list, got {}", other.type_name())),
        })
        .collect::<R<_>>()?;
    let mut combos: Vec<Vec<Value>> = vec![Vec::new()];
    for vals in &values {
        combos = combos
            .into_iter()
            .flat_map(|c| {
                vals.iter().map(move |v| {
                    let mut c2 = c.clone();
                    c2.push(v.clone());
                    c2
                })
            })
            .collect();
    }
    let seed = match o.get("seed") {
        Some(_) => o.seed()?,
        None => splitmix64(spec_seed(&s) ^ 0x6A1D),
    };
    let ids = cv_folds(&s, f, k, seed, true)?;
    let rows = f.n_rows();
    let jobs: Vec<(usize, usize)> = (0..combos.len()).flat_map(|c| (0..k).map(move |fold| (c, fold))).collect();
    let scores = run_jobs(jobs.len(), rows, |j| {
        let (c, fold) = jobs[j];
        let over: Vec<(String, Value)> = keys.iter().cloned().zip(combos[c].iter().cloned()).collect();
        let sp = with_opts(&s, &over);
        let tr: Vec<usize> = (0..rows).filter(|&i| ids[i] != fold).collect();
        let te: Vec<usize> = (0..rows).filter(|&i| ids[i] == fold).collect();
        let m = fitted(&fit_spec(&sp, &take(f, &tr)?)?)?;
        m.score(&take(f, &te)?, &metric, false).map_err(e)
    })?;
    let mut means = Vec::new();
    let mut sds = Vec::new();
    for c in 0..combos.len() {
        let v: Vec<f64> = (0..k).map(|fold| scores[c * k + fold]).collect();
        let (m, sd) = mean_sd(&v);
        means.push(m);
        sds.push(sd);
    }
    let best = (0..combos.len())
        .max_by(|&a, &b| {
            let (x, y) = (means[a], means[b]);
            let ord = if higher { x.total_cmp(&y) } else { y.total_cmp(&x) };
            ord.then(b.cmp(&a))
        })
        .unwrap_or(0);
    let mut cols: Vec<(String, Series)> = Vec::new();
    for (ki, key) in keys.iter().enumerate() {
        let items: Vec<Value> = combos.iter().map(|c| c[ki].clone()).collect();
        let s = series_from_list(&items).unwrap_or_else(|_| {
            Series::from_str_values(items.iter().map(|v| value_to_scalar(v).and_then(super::table::scalar_text).unwrap_or_default()).collect())
        });
        cols.push((key.clone(), s));
    }
    cols.push((format!("{}_mean", metric), Series::from_f64(means.clone())));
    cols.push((format!("{}_sd", metric), Series::from_f64(sds)));
    let results = Frame::new(cols).map_err(e)?;
    let best_params: ValueMap = keys.iter().cloned().zip(combos[best].iter().cloned()).collect();
    let best_spec = with_opts(&s, &keys.iter().cloned().zip(combos[best].iter().cloned()).collect::<Vec<_>>());
    let model = fit_spec(&best_spec, f)?;
    Ok(map_value(vec![
        ("results", frame_value(results)),
        ("best", Value::Map(Arc::new(best_params))),
        ("best_score", Value::Float(means[best])),
        ("metric", Value::String(Arc::new(metric))),
        ("model", model),
    ]))
}

// ---------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------

fn same_len(func: &str, a: &Series, b: &Series) -> R<()> {
    if a.len() != b.len() {
        return Err(format!("learn.{}: y has {} rows, the predictions {}", func, a.len(), b.len()));
    }
    Ok(())
}

/// Numeric pairs where both are present.
fn num_pairs(func: &str, y: &Series, p: &Series) -> R<(Vec<f64>, Vec<f64>)> {
    same_len(func, y, p)?;
    let (yv, yo) = numeric(y, func).map_err(e)?;
    let (pv, po) = numeric(p, func).map_err(e)?;
    let mut a = Vec::with_capacity(yv.len());
    let mut b = Vec::with_capacity(yv.len());
    for i in 0..yv.len() {
        if yo[i] && po[i] {
            a.push(yv[i]);
            b.push(pv[i]);
        }
    }
    if a.is_empty() {
        return Err(format!("learn.{}: no rows where both are present", func));
    }
    Ok((a, b))
}

/// Class codes of y and predictions over the union of their labels.
fn class_pairs(func: &str, y: &Series, p: &Series) -> R<(Labels, Vec<usize>, Vec<usize>)> {
    same_len(func, y, p)?;
    let (ly, _) = Labels::learn(y).map_err(e)?;
    let (lp, _) = Labels::learn(p).map_err(e)?;
    let labels = match (ly, lp) {
        (Labels::Bool(a), Labels::Bool(b)) => Labels::Bool(union(a, b)),
        (Labels::Int(a), Labels::Int(b)) => Labels::Int(union(a, b)),
        (Labels::Str(a), Labels::Str(b)) => Labels::Str(union(a, b)),
        _ => {
            return Err(format!(
                "learn.{}: y is {} but the predictions are {} — compare like with like",
                func,
                y.dtype(),
                p.dtype()
            ));
        }
    };
    let cy = labels.encode(y);
    let cp = labels.encode(p);
    let keep: Vec<usize> = (0..cy.len()).filter(|&i| cy[i] != NO_CLASS && cp[i] != NO_CLASS).collect();
    if keep.is_empty() {
        return Err(format!("learn.{}: no rows where both are present", func));
    }
    Ok((labels, keep.iter().map(|&i| cy[i]).collect(), keep.iter().map(|&i| cp[i]).collect()))
}

fn union<T: Ord + Clone>(mut a: Vec<T>, b: Vec<T>) -> Vec<T> {
    a.extend(b);
    a.sort();
    a.dedup();
    a
}

/// (y as bool, score) where both are present, for the probability metrics.
fn binary_pairs(func: &str, y: &Series, p: &Series, o: &Opts) -> R<(Vec<bool>, Vec<f64>)> {
    same_len(func, y, p)?;
    let pos = o.get("pos").and_then(value_to_scalar);
    let yb = binary_target(y, pos.as_ref()).map_err(e)?;
    let (pv, po) = numeric(p, func).map_err(e)?;
    let mut a = Vec::new();
    let mut b = Vec::new();
    for i in 0..yb.len() {
        if let Some(t) = yb[i]
            && po[i]
        {
            a.push(t);
            b.push(pv[i]);
        }
    }
    if a.is_empty() {
        return Err(format!("learn.{}: no rows where both are present", func));
    }
    Ok((a, b))
}

fn average(func: &str, o: &Opts, labels: &Labels) -> R<Average> {
    let default = if labels.len() <= 2 { "binary" } else { "macro" };
    match o.str("average", default)?.as_str() {
        "binary" => {
            if labels.len() > 2 {
                return Err(format!(
                    "learn.{}: average \"binary\" needs two classes; there are {} (use \"macro\", \"micro\" or \"weighted\")",
                    func,
                    labels.len()
                ));
            }
            let pos = match o.get("pos").and_then(value_to_scalar) {
                None => labels.len() - 1,
                Some(sc) => {
                    let name = match sc {
                        Scalar::Str(s) => s,
                        Scalar::I64(i) => i.to_string(),
                        Scalar::Bool(b) => b.to_string(),
                        Scalar::F64(f) => (f as i64).to_string(),
                        Scalar::Null => String::new(),
                    };
                    labels
                        .names()
                        .iter()
                        .position(|n| *n == name)
                        .ok_or_else(|| format!("learn.{}: pos {} is not a class", func, name))?
                }
            };
            Ok(Average::Binary(pos))
        }
        "macro" => Ok(Average::Macro),
        "micro" => Ok(Average::Micro),
        "weighted" => Ok(Average::Weighted),
        other => Err(format!(
            "learn.{}: average must be \"binary\", \"macro\", \"micro\" or \"weighted\", got \"{}\"",
            func, other
        )),
    }
}

fn metric(func: &str, args: &[Value]) -> R<Value> {
    let y = want_series(func, args, 0)?;
    // A Frame of class probabilities is accepted where a multiclass
    // metric wants one (log_loss).
    if func == "log_loss"
        && let Some(pf) = args.get(1).and_then(frame_of)
    {
        let (labels, codes) = Labels::learn(y).map_err(e)?;
        let names = labels.names();
        let mut probs = Vec::new();
        for n in &names {
            let (v, _) = numeric(pf.column(n).map_err(|_| {
                format!("learn.log_loss: the probability frame has no column for class '{}'", n)
            })?, "log_loss")
            .map_err(e)?;
            probs.push(v);
        }
        let keep: Vec<usize> = (0..codes.len()).filter(|&i| codes[i] != NO_CLASS).collect();
        let yk: Vec<usize> = keep.iter().map(|&i| codes[i]).collect();
        let pk: Vec<Vec<f64>> = probs.iter().map(|c| keep.iter().map(|&i| c[i]).collect()).collect();
        return Ok(Value::Float(metrics::log_loss_multi(&yk, &pk)));
    }
    let p = want_series(func, args, 1)?;
    let opts_keys: &[&str] = match func {
        "precision" | "recall" | "f1" => &["average", "pos"],
        "log_loss" | "brier" | "auc" | "average_precision" | "roc" | "pr_curve" => &["pos"],
        "at_threshold" => &["threshold", "pos"],
        "threshold_sweep" => &["pos"],
        "calibration" => &["bins", "strategy", "pos"],
        _ => &[],
    };
    let o = Opts::new(func, args.get(2), opts_keys)?;
    match func {
        "rmse" | "mse" | "mae" | "mape" | "r2" => {
            let (a, b) = num_pairs(func, y, p)?;
            Ok(Value::Float(match func {
                "rmse" => metrics::rmse(&a, &b),
                "mse" => metrics::mse(&a, &b),
                "mae" => metrics::mae(&a, &b),
                "mape" => metrics::mape(&a, &b),
                _ => metrics::r2(&a, &b),
            }))
        }
        "accuracy" => {
            let (_, a, b) = class_pairs(func, y, p)?;
            Ok(Value::Float(metrics::accuracy(&a, &b)))
        }
        "precision" | "recall" | "f1" => {
            let (labels, a, b) = class_pairs(func, y, p)?;
            let avg = average(func, &o, &labels)?;
            let (pr, rc, f1) = metrics::prf(&a, &b, labels.len(), avg);
            Ok(Value::Float(match func {
                "precision" => pr,
                "recall" => rc,
                _ => f1,
            }))
        }
        "confusion" => {
            let (labels, a, b) = class_pairs(func, y, p)?;
            let k = labels.len();
            let m = metrics::confusion(&a, &b, k);
            let names = labels.names();
            let mut cols = vec![("actual".to_string(), Series::from_str_values(names.clone()))];
            for (c, n) in names.iter().enumerate() {
                cols.push((n.clone(), Series::from_i64((0..k).map(|t| m[t][c] as i64).collect())));
            }
            Frame::new(cols).map(frame_value).map_err(e)
        }
        "classification_report" => {
            let (labels, a, b) = class_pairs(func, y, p)?;
            let pc = metrics::per_class(&a, &b, labels.len());
            Frame::new(vec![
                ("class".to_string(), Series::from_str_values(labels.names())),
                ("precision".to_string(), Series::from_f64(pc.iter().map(|v| v.0).collect())),
                ("recall".to_string(), Series::from_f64(pc.iter().map(|v| v.1).collect())),
                ("f1".to_string(), Series::from_f64(pc.iter().map(|v| v.2).collect())),
                ("support".to_string(), Series::from_i64(pc.iter().map(|v| v.3 as i64).collect())),
            ])
            .map(frame_value)
            .map_err(e)
        }
        "log_loss" | "brier" | "auc" | "average_precision" => {
            let (a, b) = binary_pairs(func, y, p, &o)?;
            Ok(Value::Float(match func {
                "log_loss" => metrics::log_loss(&a, &b),
                "brier" => metrics::brier(&a, &b),
                "average_precision" => metrics::average_precision(&a, &b),
                _ => metrics::roc_auc(&a, &b).ok_or("learn.auc: y has one class only; AUC needs both")?,
            }))
        }
        "roc" => {
            let (a, b) = binary_pairs(func, y, p, &o)?;
            let (fpr, tpr, th) = metrics::roc_curve(&a, &b);
            Frame::new(vec![
                ("fpr".to_string(), Series::from_f64(fpr)),
                ("tpr".to_string(), Series::from_f64(tpr)),
                ("threshold".to_string(), Series::from_f64(th)),
            ])
            .map(frame_value)
            .map_err(e)
        }
        "pr_curve" => {
            let (a, b) = binary_pairs(func, y, p, &o)?;
            let (pr, rc, th) = metrics::pr_curve(&a, &b);
            // The last point (recall 0) has no threshold: null there.
            let mut tho: Vec<Option<f64>> = th.into_iter().map(Some).collect();
            tho.push(None);
            Frame::new(vec![
                ("precision".to_string(), Series::from_f64(pr)),
                ("recall".to_string(), Series::from_f64(rc)),
                ("threshold".to_string(), Series::from_f64_options(tho)),
            ])
            .map(frame_value)
            .map_err(e)
        }
        "at_threshold" => {
            let (a, b) = binary_pairs(func, y, p, &o)?;
            let t = o.f64("threshold", 0.5)?;
            let (mut tp, mut fp, mut fnn, mut tn) = (0i64, 0i64, 0i64, 0i64);
            for (yv, s) in a.iter().zip(&b) {
                match (*yv, *s >= t) {
                    (true, true) => tp += 1,
                    (false, true) => fp += 1,
                    (true, false) => fnn += 1,
                    (false, false) => tn += 1,
                }
            }
            let prec = if tp + fp > 0 { tp as f64 / (tp + fp) as f64 } else { 0.0 };
            let rec = if tp + fnn > 0 { tp as f64 / (tp + fnn) as f64 } else { 0.0 };
            let f1 = if prec + rec > 0.0 { 2.0 * prec * rec / (prec + rec) } else { 0.0 };
            let n = a.len() as f64;
            Ok(map_value(vec![
                ("threshold", Value::Float(t)),
                ("tp", Value::Integer(tp)),
                ("fp", Value::Integer(fp)),
                ("fn", Value::Integer(fnn)),
                ("tn", Value::Integer(tn)),
                ("precision", Value::Float(prec)),
                ("recall", Value::Float(rec)),
                ("f1", Value::Float(f1)),
                ("accuracy", Value::Float((tp + tn) as f64 / n)),
                ("flagged", Value::Float((tp + fp) as f64 / n)),
            ]))
        }
        "threshold_sweep" => {
            let (a, b) = binary_pairs(func, y, p, &o)?;
            // One sort: walk the distinct scores from high to low.
            let mut idx: Vec<usize> = (0..a.len()).collect();
            idx.sort_by(|&i, &j| b[j].total_cmp(&b[i]));
            let np = a.iter().filter(|v| **v).count() as f64;
            let n = a.len() as f64;
            let (mut tp, mut fp) = (0.0, 0.0);
            let (mut th, mut pr, mut rc, mut f1, mut acc, mut fl) = (vec![], vec![], vec![], vec![], vec![], vec![]);
            for (pos, &i) in idx.iter().enumerate() {
                if a[i] {
                    tp += 1.0;
                } else {
                    fp += 1.0;
                }
                if pos + 1 == idx.len() || b[idx[pos + 1]] != b[i] {
                    let p_ = tp / (tp + fp);
                    let r_ = if np > 0.0 { tp / np } else { 0.0 };
                    th.push(b[i]);
                    pr.push(p_);
                    rc.push(r_);
                    f1.push(if p_ + r_ > 0.0 { 2.0 * p_ * r_ / (p_ + r_) } else { 0.0 });
                    acc.push((tp + (n - np - fp)) / n);
                    fl.push((tp + fp) / n);
                }
            }
            Frame::new(vec![
                ("threshold".to_string(), Series::from_f64(th)),
                ("precision".to_string(), Series::from_f64(pr)),
                ("recall".to_string(), Series::from_f64(rc)),
                ("f1".to_string(), Series::from_f64(f1)),
                ("accuracy".to_string(), Series::from_f64(acc)),
                ("flagged".to_string(), Series::from_f64(fl)),
            ])
            .map(frame_value)
            .map_err(e)
        }
        "calibration" => {
            let (a, b) = binary_pairs(func, y, p, &o)?;
            let bins = o.usize("bins", 10)?.max(1);
            let q = o.one_of("strategy", "uniform", &["uniform", "quantile"])? == "quantile";
            let rows = metrics::calibration(&a, &b, bins, q);
            Frame::new(vec![
                ("lo".to_string(), Series::from_f64(rows.iter().map(|r| r.0).collect())),
                ("hi".to_string(), Series::from_f64(rows.iter().map(|r| r.1).collect())),
                ("predicted".to_string(), Series::from_f64(rows.iter().map(|r| r.2).collect())),
                ("observed".to_string(), Series::from_f64(rows.iter().map(|r| r.3).collect())),
                ("count".to_string(), Series::from_i64(rows.iter().map(|r| r.4 as i64).collect())),
            ])
            .map(frame_value)
            .map_err(e)
        }
        _ => Err(format!("unknown learn function: learn.{}", func)),
    }
}


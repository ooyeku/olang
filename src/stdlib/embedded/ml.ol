// ml — machine learning with keyword arguments, bundled as a builtin
// package (`use ml`).
//
// Every function here is a thin keyword front on the native `learn`
// module: the work (training, prediction, scoring, resampling) runs in
// Rust, never per row in olang. What this layer adds is the calling
// convention — named options with defaults — and the one thing a native
// cannot do: call an olang function, which `cross_validate` and
// `learning_curve` do once per fold when handed a `(train) => model`.
//
//   use ml
//   let s = ml.split(homes, test: 0.25, seed: 7)
//   let fit = ml.linear(s.train, y: "price", x: ["sqft", "beds", "hood"])
//   println(fit.params.sqft)
//   let pred = ml.predict(fit, s.test)
//   println(ml.rmse(s.test["price"], pred))
//
// A fit is a map: `fit.params`, `fit.coef` (a Frame), `fit.r2`, ...;
// `fit.model` is the native model `learn` predicts with, `fit.spec` how
// to fit it again.

// ── plumbing ───────────────────────────────────────────────────────────

// An options map without the options left at ().
fn opts(m) = {
    let mut out = #{}
    for k in map_keys(m) {
        let v = map_get(m, k)
        if typeof(v) != "Unit" => {
            out = map_set(out, k, v)
        }
    }
    out
}

// ── splitting ──────────────────────────────────────────────────────────

/// `#{ train, test }`: a seeded split, `test` a fraction or a row count,
/// stratified by a column's classes when `stratify` names one.
share fn split(data, test = 0.25, stratify = (), shuffle = true, seed = ()) =
    learn.split(data, opts(#{ "test": test, "stratify": stratify, "shuffle": shuffle, "seed": seed }))

/// An Int Series of fold ids (0..k) for the rows of `data` (a Frame or a
/// row count), stratified when `stratify` names a column.
share fn kfold(data, k = 5, stratify = (), shuffle = true, seed = ()) =
    learn.kfold(data, k, opts(#{ "stratify": stratify, "shuffle": shuffle, "seed": seed }))

/// The k folds as a list of `#{ fold, train, test }` Frames.
share fn folds(data, k = 5, stratify = (), shuffle = true, seed = ()) =
    learn.folds(data, k, opts(#{ "stratify": stratify, "shuffle": shuffle, "seed": seed }))

/// Rolling-origin splits for ordered data: a list of `#{ train, test }`.
share fn time_splits(data, n_splits = 5, test_size = (), gap = 0, max_train = ()) =
    learn.time_splits(data, n_splits, opts(#{ "test_size": test_size, "gap": gap, "max_train": max_train }))

// ── preprocessing ──────────────────────────────────────────────────────

/// A scaler fitted on `data` (`"standard"`, `"minmax"` or `"robust"`);
/// apply it to any frame with `ml.transform`.
share fn scaler(data, cols = (), method = "standard") =
    learn.scaler(data, cols, #{ "method": method })

/// A one-hot encoder of String columns (all of them when `cols` is ()).
share fn one_hot(data, cols = (), drop_first = false) =
    learn.one_hot(data, cols, #{ "drop_first": drop_first })

/// An imputer: `"mean"`, `"median"`, `"most_frequent"` or `"constant"`
/// (with `fill`).
share fn imputer(data, cols = (), strategy = "mean", fill = ()) =
    learn.imputer(data, cols, opts(#{ "strategy": strategy, "fill": fill }))

/// Apply a fitted scaler, encoder, imputer or PCA to a frame.
share fn transform(model, data) = learn.transform(model, data)

/// Fit a transformer on `data` and apply it there: `#{ model, data }`.
share fn fit_transform(model_fn, data) = {
    let m = model_fn(data)
    #{ "model": m, "data": learn.transform(m, data) }
}

/// Polynomial (and interaction) columns of numeric `cols`, appended.
share fn poly(data, cols = (), degree = 2, interaction_only = false) =
    learn.poly(data, cols, degree, #{ "interaction_only": interaction_only })

// ── linear models ──────────────────────────────────────────────────────

/// Ordinary least squares with standard errors, t, p and intervals.
share fn linear(data, y, x = (), intercept = true, weights = ()) =
    learn.linear(data, y, x, opts(#{ "intercept": intercept, "weights": weights }))

/// Ridge regression (L2): ‖y − Xβ‖² + alpha‖β‖².
share fn ridge(data, y, x = (), alpha = 1.0, intercept = true) =
    learn.ridge(data, y, x, #{ "alpha": alpha, "intercept": intercept })

/// The lasso (L1) by coordinate descent.
share fn lasso(data, y, x = (), alpha = 1.0, intercept = true, max_iter = (), tol = ()) =
    learn.lasso(data, y, x, opts(#{ "alpha": alpha, "intercept": intercept, "max_iter": max_iter, "tol": tol }))

/// Elastic net: alpha·l1_ratio of L1 plus alpha·(1 − l1_ratio)/2 of L2.
share fn elastic_net(data, y, x = (), alpha = 1.0, l1_ratio = 0.5, intercept = true, max_iter = (), tol = ()) =
    learn.elastic_net(data, y, x, opts(#{ "alpha": alpha, "l1_ratio": l1_ratio, "intercept": intercept,
                                          "max_iter": max_iter, "tol": tol }))

/// Logistic regression (binary, or multinomial for 3+ classes) by
/// Newton's method; `C` or `penalty` adds an L2 penalty.
share fn logistic(data, y, x = (), C = (), penalty = (), class_weight = (), weights = (), max_iter = ()) =
    learn.logistic(data, y, x, opts(#{ "C": C, "penalty": penalty, "class_weight": class_weight,
                                       "weights": weights, "max_iter": max_iter }))

// ── trees and ensembles ────────────────────────────────────────────────

/// A CART decision tree (classification for a String, Bool or Int
/// target, regression for a Float one; `task` overrides).
share fn tree(data, y, x = (), max_depth = (), min_samples_split = (), min_samples_leaf = (),
              criterion = (), max_features = (), class_weight = (), task = (), seed = ()) =
    learn.tree(data, y, x, opts(#{ "max_depth": max_depth, "min_samples_split": min_samples_split,
                                   "min_samples_leaf": min_samples_leaf, "criterion": criterion,
                                   "max_features": max_features, "class_weight": class_weight,
                                   "task": task, "seed": seed }))

/// A random forest: `trees` bootstrapped trees grown in parallel, each
/// split choosing among `max_features` (√k for classification).
share fn forest(data, y, x = (), trees = 100, max_depth = (), max_features = (), min_samples_leaf = (),
                min_samples_split = (), bootstrap = true, max_samples = (), oob = false, criterion = (),
                class_weight = (), task = (), seed = ()) =
    learn.forest(data, y, x, opts(#{ "n_trees": trees, "max_depth": max_depth, "max_features": max_features,
                                     "min_samples_leaf": min_samples_leaf, "min_samples_split": min_samples_split,
                                     "bootstrap": bootstrap, "max_samples": max_samples, "oob": oob,
                                     "criterion": criterion, "class_weight": class_weight, "task": task,
                                     "seed": seed }))

/// Gradient-boosted trees over binned features (squared loss, or log
/// loss for a binary target). `early_stopping` holds out `validation`
/// of the rows at random and stops after `patience` rounds without
/// improvement. An explicit validation set stops on it instead (and
/// turns early stopping on): `validation` a Frame (its rows appended to
/// the data's to fit, held out), `validation_rows` (row indices of the
/// data), or `validation_tail: n` (the last n rows: a time series' most
/// recent). `m.valid_loss` is the loss on them each round (`m.best_iter` the least).
share fn boost(data, y, x = (), trees = 100, learning_rate = 0.1, max_depth = 3, max_leaf_nodes = (),
               min_samples_leaf = (), l2 = (), subsample = 1.0, bins = 255, early_stopping = false,
               validation = 0.1, validation_rows = (), validation_tail = (), patience = 10, task = (),
               weights = (), seed = ()) = {
    let held_frame = typeof(validation) == "Frame"
    let fit_on = if held_frame => ods.concat([data, validation]) else => data
    let rows = if held_frame => range(ods.n_rows(data), ods.n_rows(fit_on)) else => validation_rows
    let explicit = held_frame || validation_rows != () || validation_tail != ()
    learn.boost(fit_on, y, x, opts(#{ "n_trees": trees, "learning_rate": learning_rate, "max_depth": max_depth,
                                      "max_leaf_nodes": max_leaf_nodes, "min_samples_leaf": min_samples_leaf,
                                      "l2": l2, "subsample": subsample, "max_bins": bins,
                                      "early_stopping": early_stopping || explicit,
                                      "validation_fraction": if held_frame => () else => validation,
                                      "validation_rows": rows, "validation_tail": validation_tail,
                                      "n_iter_no_change": patience, "task": task, "weights": weights,
                                      "seed": seed }))
}

// ── other models ───────────────────────────────────────────────────────

/// k-nearest neighbours; `weights_by: "distance"` weighs by 1/d.
share fn knn(data, y, x = (), k = 5, weights_by = "uniform", task = ()) =
    learn.knn(data, y, x, opts(#{ "k": k, "weights_by": weights_by, "task": task }))

/// Gaussian naive Bayes.
share fn naive_bayes(data, y, x = (), var_smoothing = ()) =
    learn.naive_bayes(data, y, x, opts(#{ "var_smoothing": var_smoothing }))

/// k-means with k-means++ starts; the best of `n_init` by inertia.
share fn kmeans(data, k, cols = (), n_init = 10, max_iter = 300, seed = ()) =
    learn.kmeans(data, cols, k, opts(#{ "n_init": n_init, "max_iter": max_iter, "seed": seed }))

/// A Gaussian mixture (full covariances) fitted by EM.
share fn gmm(data, k, cols = (), max_iter = 100, seed = ()) =
    learn.gmm(data, cols, k, opts(#{ "max_iter": max_iter, "seed": seed }))

/// Principal components (`n` of them, all by default), on standardized
/// columns when `scale`.
share fn pca(data, cols = (), n = (), scale = false) =
    learn.pca(data, cols, opts(#{ "n_components": n, "scale": scale }))

/// The mean silhouette coefficient of a labelling.
share fn silhouette(data, labels, cols = (), sample = (), seed = ()) =
    learn.silhouette(data, cols, labels, opts(#{ "sample": sample, "seed": seed }))

// ── the model interface ────────────────────────────────────────────────

/// Predictions for `data`: values, class labels or clusters by default;
/// `kind: "prob"` (positive-class probability, or a Frame for 3+
/// classes), `"probs"`, `"link"`, `"confidence"` / `"prediction"` (an
/// OLS interval Frame at `level`). `trees: k`: a boosted model's
/// prediction from its first k trees (staged predictions).
share fn predict(model, data, kind = "auto", level = 0.95, trees = ()) =
    learn.predict(model, data, opts(#{ "kind": kind, "level": level, "trees": trees }))

/// The positive-class probability (binary) or a Frame of class
/// probabilities.
share fn prob(model, data) = learn.predict(model, data, #{ "kind": "prob" })

share fn importance(model) = learn.importance(model)

share fn summary(model, level = 0.95) = learn.summary(model, #{ "level": level })

share fn params(model) = learn.params(model)

/// One metric of the model on a frame that carries the target.
share fn score(model, data, metric) = learn.score(model, data, metric)

/// How to fit, without fitting: for `cross_validate`, `learning_curve`
/// and `grid_search`, which refit it natively. `options` is a map or an
/// object (`{ n_trees: 200 }`); an unknown key raises.
share fn spec(kind, y = (), x = (), options = #{}) = learn.spec(kind, y, x, options)

/// Fit a spec (or refit a model's) on new data.
share fn fit(spec, data) = learn.fit(spec, data)

// ── evaluation ─────────────────────────────────────────────────────────

fn default_metrics(m) =
    if map_has_key(m, "classes") =>
        if len(m.classes) == 2 => ["accuracy", "auc"] else => ["accuracy"]
    else => ["rmse", "r2"]

/// A Frame of per-fold scores. `fit` is a model, a spec, or a function
/// `(train) => model` — the first two refit natively (in parallel), the
/// function is called once per fold.
share fn cross_validate(fit, data, folds = 5, metrics = (), stratify = (), seed = ()) = {
    if typeof(fit) != "Function" => {
        learn.cross_validate(fit, data, opts(#{ "folds": folds, "metrics": metrics, "seed": seed }))
    } else => {
        let parts = learn.folds(data, folds, opts(#{ "stratify": stratify, "seed": seed }))
        let mut rows = []
        let mut names = metrics
        for p in parts {
            let m = fit(p.train)
            if typeof(names) == "Unit" => { names = default_metrics(m) }
            let mut row = #{ "fold": p.fold, "n_train": ods.n_rows(p.train), "n_test": ods.n_rows(p.test) }
            for name in names {
                row = map_set(row, name, learn.score(m, p.test, name))
            }
            rows = rows + [row]
        }
        ods.frame_from_records(rows)
    }
}

/// The drop in a metric when each input is shuffled: a Frame of
/// feature, importance, std.
share fn permutation_importance(model, data, metric = (), repeats = 5, seed = ()) =
    learn.permutation_importance(model, data, opts(#{ "metric": metric, "repeats": repeats, "seed": seed }))

/// Average prediction as `feature` sweeps a grid: a Frame to plot.
share fn partial_dependence(model, data, feature, grid = (), points = 50, sample = (), seed = ()) =
    learn.partial_dependence(model, data, feature, opts(#{ "grid": grid, "points": points, "sample": sample, "seed": seed }))

/// Train and test scores as the training set grows: a Frame of n_train,
/// train, train_sd, test, test_sd. `fit` is a model or a spec.
share fn learning_curve(fit, data, sizes = (), folds = 5, metric = (), seed = ()) =
    learn.learning_curve(fit, data, opts(#{ "sizes": sizes, "folds": folds, "metric": metric, "seed": seed }))

/// Cross-validated search over a grid of options (`{ max_depth: [3,
/// 5, 8] }`, or the same as a map): `#{ results, best, best_score, metric, model }`, the model
/// refit on all of `data` with the best options.
share fn grid_search(fit, data, grid, folds = 5, metric = (), seed = ()) =
    learn.grid_search(fit, data, grid, opts(#{ "folds": folds, "metric": metric, "seed": seed }))

// ── metrics ────────────────────────────────────────────────────────────

share fn rmse(y, pred) = learn.rmse(y, pred)
share fn mse(y, pred) = learn.mse(y, pred)
share fn mae(y, pred) = learn.mae(y, pred)
share fn mape(y, pred) = learn.mape(y, pred)
share fn r2(y, pred) = learn.r2(y, pred)
share fn accuracy(y, pred) = learn.accuracy(y, pred)

share fn precision(y, pred, average = (), pos = ()) =
    learn.precision(y, pred, opts(#{ "average": average, "pos": pos }))
share fn recall(y, pred, average = (), pos = ()) =
    learn.recall(y, pred, opts(#{ "average": average, "pos": pos }))
share fn f1(y, pred, average = (), pos = ()) =
    learn.f1(y, pred, opts(#{ "average": average, "pos": pos }))

/// Log loss of probabilities: a Series (binary) or a Frame of class
/// probabilities.
share fn log_loss(y, p, pos = ()) =
    if typeof(p) == "Frame" => learn.log_loss(y, p) else => learn.log_loss(y, p, opts(#{ "pos": pos }))
share fn brier(y, p, pos = ()) = learn.brier(y, p, opts(#{ "pos": pos }))
share fn auc(y, p, pos = ()) = learn.auc(y, p, opts(#{ "pos": pos }))
share fn average_precision(y, p, pos = ()) = learn.average_precision(y, p, opts(#{ "pos": pos }))
share fn roc(y, p, pos = ()) = learn.roc(y, p, opts(#{ "pos": pos }))
share fn pr_curve(y, p, pos = ()) = learn.pr_curve(y, p, opts(#{ "pos": pos }))
share fn confusion(y, pred) = learn.confusion(y, pred)
share fn classification_report(y, pred) = learn.classification_report(y, pred)
share fn at_threshold(y, p, threshold = 0.5, pos = ()) =
    learn.at_threshold(y, p, opts(#{ "threshold": threshold, "pos": pos }))
share fn threshold_sweep(y, p, pos = ()) = learn.threshold_sweep(y, p, opts(#{ "pos": pos }))
share fn calibration(y, p, bins = 10, strategy = "uniform", pos = ()) =
    learn.calibration(y, p, opts(#{ "bins": bins, "strategy": strategy, "pos": pos }))

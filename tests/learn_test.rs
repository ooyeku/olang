//! The `learn` module and `use ml` from olang source, pinned against
//! scikit-learn 1.9 / numpy / scipy on the fixtures in
//! tests/fixtures/learn (reference.json holds the expected values; the
//! script that wrote both is described in that folder's README).
//!
//! Deterministic algorithms (OLS, penalized regression, logistic and
//! multinomial regression, naive Bayes, trees, kNN, PCA, scalers,
//! k-means' inertia, metrics) must match to the solvers' tolerances.
//! Randomized ensembles draw from a different generator than sklearn's,
//! so their *test metrics* must land within a band of sklearn's.

use olang::ast::Value;
use olang::{Interpreter, Parser};
use serde_json::Value as Json;

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn fixtures() -> String {
    format!("{}/tests/fixtures/learn", env!("CARGO_MANIFEST_DIR"))
}

fn eval_tier(source: &str, tier: Option<u32>) -> Result<Value, String> {
    let source = source.replace("FIX", &fixtures());
    with_big_stack(move || {
        let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        if let Some(t) = tier {
            interpreter.enable_bytecode_tier(t, false);
        }
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

fn eval(source: &str) -> Value {
    eval_tier(source, None).unwrap_or_else(|e| panic!("{e}\n--- source ---\n{source}"))
}

fn reference() -> Json {
    let text =
        std::fs::read_to_string(format!("{}/reference.json", fixtures())).expect("reference.json");
    serde_json::from_str(&text).expect("json")
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Float(x) => *x,
        Value::Integer(x) => *x as f64,
        other => panic!("expected a number, got {:?}", other),
    }
}

fn nums(v: &Value) -> Vec<f64> {
    match v {
        Value::List(items) => items.iter().map(num).collect(),
        other => panic!("expected a list, got {:?}", other),
    }
}

fn jnums(j: &Json) -> Vec<f64> {
    j.as_array()
        .expect("array")
        .iter()
        .flat_map(|v| match v {
            Json::Array(_) => jnums(v),
            other => vec![other.as_f64().expect("number")],
        })
        .collect()
}

fn jnum(j: &Json) -> f64 {
    j.as_f64().expect("number")
}

fn close(got: f64, want: f64, tol: f64, what: &str) {
    assert!(
        (got - want).abs() <= tol * (1.0 + want.abs()),
        "{what}: got {got}, want {want} (tol {tol})"
    );
}

fn all_close(got: &[f64], want: &[f64], tol: f64, what: &str) {
    assert_eq!(
        got.len(),
        want.len(),
        "{what}: length {} vs {}",
        got.len(),
        want.len()
    );
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        close(*g, *w, tol, &format!("{what}[{i}]"));
    }
}

const READ: &str = r#"
let reg = unwrap(ods.read_csv_file("FIX/reg.csv"))
let cls = unwrap(ods.read_csv_file("FIX/cls.csv"))
let multi = unwrap(ods.read_csv_file("FIX/multi.csv"))
let blobs = unwrap(ods.read_csv_file("FIX/blobs.csv"))
let scores = unwrap(ods.read_csv_file("FIX/scores.csv"))
fn l(s) = ods.to_list(s)
fn col(f, c) = ods.to_list(f[c])
fn flat_map(xs, f) = flatten(map(xs, f))
fn drop(xs, n) = skip(xs, n)
fn index_of(xs, v) = {
    let mut at = -1
    for (i, x) in enumerate(xs) {
        if x == v && at < 0 => { at = i }
    }
    at
}
"#;

fn run(body: &str) -> Value {
    eval(&format!("{READ}\n{body}"))
}

fn items(v: &Value) -> Vec<Value> {
    match v {
        Value::List(items) => items.as_ref().clone(),
        other => panic!("expected a list, got {:?}", other),
    }
}

#[test]
fn ols_matches_numpy_with_inference_and_intervals() {
    let r = reference();
    let v = items(&run(r#"
let fit = learn.linear(reg, "y", ["x1", "x2", "x3", "hood"])
let q = ods.frame([["x1", [0.5]], ["x2", [1.0]], ["x3", [0.0]], ["hood", ["north"]]])
let pi = learn.predict(fit, q, #{ "kind": "prediction" })
let ci = learn.predict(fit, q, #{ "kind": "confidence" })
[col(fit.coef, "estimate"), col(fit.coef, "se"), col(fit.coef, "t"), col(fit.coef, "p"),
 [fit.r2, fit.adj_r2, fit.aic, fit.bic],
 [col(pi, "fit")[0], col(ci, "lo")[0], col(ci, "hi")[0], col(pi, "lo")[0], col(pi, "hi")[0]],
 fit.terms]
"#));
    let o = &r["ols"];
    all_close(&nums(&v[0]), &jnums(&o["coef"]), 1e-10, "coef");
    all_close(&nums(&v[1]), &jnums(&o["se"]), 1e-9, "se");
    all_close(&nums(&v[2]), &jnums(&o["t"]), 1e-9, "t");
    // p-values span 1e-158..0.5: compare in log space.
    for (g, w) in nums(&v[3]).iter().zip(jnums(&o["p"])) {
        close(g.ln(), w.ln(), 1e-6, "log p");
    }
    let fitv = nums(&v[4]);
    close(fitv[0], jnum(&o["r2"]), 1e-12, "r2");
    close(fitv[1], jnum(&o["adj_r2"]), 1e-12, "adj_r2");
    close(fitv[2], jnum(&o["aic"]), 1e-10, "aic");
    close(fitv[3], jnum(&o["bic"]), 1e-10, "bic");
    let iv = nums(&v[5]);
    close(iv[0], jnum(&o["pred"]), 1e-10, "prediction");
    all_close(&iv[1..3], &jnums(&o["conf"]), 1e-9, "confidence interval");
    all_close(&iv[3..5], &jnums(&o["pi"]), 1e-9, "prediction interval");
    assert_eq!(
        format!("{:?}", v[6]).matches("hood_").count(),
        2,
        "one-hot drops the first level for a model with an intercept"
    );
}

#[test]
fn ridge_lasso_and_elastic_net_match_sklearn() {
    let r = reference();
    let v = items(&run(r#"
let x = ["x1", "x2", "x3", "hood"]
fn est(m) = col(learn.summary(m), "estimate")
[est(learn.ridge(reg, "y", x, #{ "alpha": 5.0 })),
 est(learn.lasso(reg, "y", x, #{ "alpha": 0.1 })),
 est(learn.elastic_net(reg, "y", x, #{ "alpha": 0.2, "l1_ratio": 0.3 }))]
"#));
    all_close(&nums(&v[0]), &jnums(&r["ridge"]), 1e-10, "ridge");
    all_close(&nums(&v[1]), &jnums(&r["lasso"]), 1e-7, "lasso");
    all_close(&nums(&v[2]), &jnums(&r["enet"]), 1e-7, "elastic net");
}

#[test]
fn logistic_regression_matches_sklearn_and_statsmodels_inference() {
    let r = reference();
    let v = items(&run(r#"
let x = ["a", "b", "c", "d", "seg"]
let fit = learn.logistic(cls, "churn", x)
let p = take(l(learn.predict(fit, cls, #{ "kind": "prob" })), 10)
fn est(m) = col(learn.summary(m), "estimate")
[col(fit.coef, "estimate"), col(fit.coef, "se"), col(fit.coef, "p"), p,
 est(learn.logistic(cls, "churn", x, #{ "C": 0.5 })),
 est(learn.logistic(cls, "churn", x, #{ "class_weight": "balanced" })),
 fit.classes, fit.converged]
"#));
    let o = &r["logit"];
    all_close(&nums(&v[0]), &jnums(&o["coef"]), 1e-6, "coef");
    all_close(&nums(&v[1]), &jnums(&o["se"]), 1e-6, "se");
    for (g, w) in nums(&v[2]).iter().zip(jnums(&o["p"])) {
        close(g.ln(), w.ln(), 1e-5, "log p");
    }
    all_close(&nums(&v[3]), &jnums(&o["prob"]), 1e-7, "probabilities");
    all_close(&nums(&v[4]), &jnums(&r["logit_c"]), 1e-6, "C = 0.5");
    all_close(
        &nums(&v[5]),
        &jnums(&r["logit_bal"]),
        1e-5,
        "balanced class weights",
    );
    assert_eq!(
        format!("{:?}", v[6]),
        format!("{:?}", eval(r#"["no", "yes"]"#))
    );
    assert_eq!(v[7], Value::Boolean(true));
}

#[test]
fn multinomial_logistic_matches_sklearn() {
    let r = reference();
    let v = items(&run(r#"
let x = ["u", "v", "w"]
let fit = learn.logistic(multi, "label", x)
let pf = learn.predict(fit, ods.head(multi, 10), #{ "kind": "prob" })
let pc = learn.predict(learn.logistic(multi, "label", x, #{ "C": 0.3 }), ods.head(multi, 10), #{ "kind": "probs" })
fn rows(f) = flat_map(range(0, 10), (i) => [col(f, "a")[i], col(f, "b")[i], col(f, "c")[i]])
[rows(pf), col(fit.coef, "estimate"), rows(pc)]
"#));
    all_close(
        &nums(&v[0]),
        &jnums(&r["softmax_prob"]),
        1e-6,
        "probabilities",
    );
    all_close(
        &nums(&v[1]),
        &jnums(&r["softmax_coef"]),
        1e-5,
        "centred coefficients",
    );
    all_close(&nums(&v[2]), &jnums(&r["softmax_c_prob"]), 1e-6, "C = 0.3");
}

#[test]
fn naive_bayes_trees_and_neighbours_match_sklearn() {
    let r = reference();
    let v = items(&run(r#"
let cx = ["a", "b", "c", "d"]
let rx = ["x1", "x2", "x3"]
let nb = learn.naive_bayes(cls, "churn", cx)
let nbm = learn.predict(learn.naive_bayes(multi, "label", ["u", "v", "w"]), ods.head(multi, 5), #{ "kind": "probs" })
let tc = learn.tree(cls, "churn", cx, #{ "max_depth": 3, "seed": 1 })
let te = learn.tree(cls, "churn", cx, #{ "max_depth": 3, "criterion": "entropy", "seed": 1 })
let tr = learn.tree(reg, "y", rx, #{ "max_depth": 4, "seed": 1 })
fn imp(m) = {
    let f = learn.importance(m)
    let names = col(f, "feature")
    let vals = col(f, "importance")
    map(m.features, (n) => vals[index_of(names, n)])
}
let rtr = ods.head(reg, 300)
let rte = ods.tail(reg, 100)
let ctr = ods.head(cls, 400)
let cte = ods.tail(cls, 100)
[take(l(learn.predict(nb, cls, #{ "kind": "prob" })), 10),
 flat_map(range(0, 5), (i) => [col(nbm, "a")[i], col(nbm, "b")[i], col(nbm, "c")[i]]),
 l(learn.predict(tc, cls, #{ "kind": "prob" })), imp(tc),
 l(learn.predict(te, cls, #{ "kind": "prob" })),
 take(l(learn.predict(tr, reg)), 20), imp(tr),
 l(learn.predict(learn.knn(rtr, "y", rx, #{ "k": 7 }), rte)),
 l(learn.predict(learn.knn(rtr, "y", rx, #{ "k": 7, "weights_by": "distance" }), rte)),
 l(learn.predict(learn.knn(ctr, "churn", cx, #{ "k": 9 }), cte, #{ "kind": "prob" }))]
"#));
    all_close(&nums(&v[0]), &jnums(&r["nb_prob"]), 1e-9, "naive Bayes");
    all_close(
        &nums(&v[1]),
        &jnums(&r["nb_multi_prob"]),
        1e-9,
        "naive Bayes, 3 classes",
    );
    all_close(
        &nums(&v[2]),
        &jnums(&r["tree_cls"]["prob"]),
        1e-12,
        "gini tree",
    );
    all_close(
        &nums(&v[3]),
        &jnums(&r["tree_cls"]["imp"]),
        1e-9,
        "gini tree importances",
    );
    all_close(
        &nums(&v[4]),
        &jnums(&r["tree_cls_entropy"]),
        1e-12,
        "entropy tree",
    );
    all_close(
        &nums(&v[5]),
        &jnums(&r["tree_reg"]["pred"]),
        1e-12,
        "regression tree",
    );
    all_close(
        &nums(&v[6]),
        &jnums(&r["tree_reg"]["imp"]),
        1e-9,
        "regression tree importances",
    );
    all_close(&nums(&v[7]), &jnums(&r["knn_reg"]), 1e-12, "kNN regression");
    all_close(
        &nums(&v[8]),
        &jnums(&r["knn_reg_dist"]),
        1e-12,
        "kNN, distance weights",
    );
    all_close(
        &nums(&v[9]),
        &jnums(&r["knn_cls_prob"]),
        1e-12,
        "kNN classifier",
    );
}

#[test]
fn scalers_pca_kmeans_and_mixtures_match_sklearn() {
    let r = reference();
    let v = items(&run(r#"
let rx = ["x1", "x2", "x3"]
fn first3(f) = flat_map(range(0, 3), (i) => map(rx, (c) => col(f, c)[i]))
let p = learn.pca(reg, rx)
let scores = learn.transform(p, ods.head(reg, 3))
let km = learn.kmeans(blobs, ["p", "q"], 4, #{ "seed": 1 })
let g = learn.gmm(blobs, ["p", "q"], 4, #{ "seed": 1 })
[first3(learn.transform(learn.scaler(reg, rx), reg)),
 first3(learn.transform(learn.scaler(reg, rx, #{ "method": "minmax" }), reg)),
 first3(learn.transform(learn.scaler(reg, rx, #{ "method": "robust" }), reg)),
 l(p.explained_variance), l(p.explained_variance_ratio),
 flat_map(range(0, 3), (i) => map(rx, (c) => col(p.components, c)[i])),
 flat_map(range(0, 3), (i) => [col(scores, "pc1")[i], col(scores, "pc2")[i], col(scores, "pc3")[i]]),
 [km.inertia, learn.silhouette(blobs, ["p", "q"], km.labels), g.log_lik / 600.0, g.bic],
 sort(l(g.weights))]
"#));
    let s = &r["scalers"];
    all_close(
        &nums(&v[0]),
        &jnums(&s["standard"]),
        1e-12,
        "standard scaler",
    );
    all_close(&nums(&v[1]), &jnums(&s["minmax"]), 1e-12, "min-max scaler");
    all_close(&nums(&v[2]), &jnums(&s["robust"]), 1e-12, "robust scaler");
    let p = &r["pca"];
    all_close(&nums(&v[3]), &jnums(&p["ev"]), 1e-10, "explained variance");
    all_close(
        &nums(&v[4]),
        &jnums(&p["ratio"]),
        1e-10,
        "explained variance ratio",
    );
    all_close(
        &nums(&v[5]),
        &jnums(&p["components"]),
        1e-8,
        "components (sklearn's sign)",
    );
    all_close(&nums(&v[6]), &jnums(&p["scores"]), 1e-8, "scores");
    let c = nums(&v[7]);
    close(c[0], jnum(&r["kmeans_inertia"]), 1e-9, "k-means inertia");
    close(c[1], jnum(&r["silhouette"]), 1e-9, "silhouette");
    close(
        c[2],
        jnum(&r["gmm"]["score"]),
        1e-4,
        "mixture log-likelihood per row",
    );
    close(c[3], jnum(&r["gmm"]["bic"]), 1e-4, "mixture BIC");
    all_close(
        &nums(&v[8]),
        &jnums(&r["gmm"]["weights"]),
        5e-3,
        "mixture weights",
    );
}

#[test]
fn binary_metrics_match_sklearn() {
    let r = reference();
    let b = &r["binary"];
    let v = items(&run(r#"
let y = scores["y"]
let p = scores["p"]
let roc = learn.roc(y, p)
let pr = learn.pr_curve(y, p)
let cal = learn.calibration(y, p)
let calq = learn.calibration(y, p, #{ "bins": 5, "strategy": "quantile" })
let t = learn.at_threshold(y, p)
let yhat = p >= 0.5
let yb = y == 1
[[learn.auc(y, p), learn.log_loss(y, p), learn.brier(y, p), learn.average_precision(y, p)],
 col(roc, "fpr"), col(roc, "tpr"), drop(col(roc, "threshold"), 1),
 col(pr, "precision"), col(pr, "recall"), take(col(pr, "threshold"), ods.n_rows(pr) - 1),
 col(cal, "observed"), col(cal, "predicted"), col(calq, "observed"), col(calq, "predicted"),
 [t.accuracy, t.precision, t.recall, t.f1],
 [learn.accuracy(yb, yhat), learn.precision(yb, yhat), learn.recall(yb, yhat), learn.f1(yb, yhat)]]
"#));
    all_close(
        &nums(&v[0]),
        &[
            jnum(&b["auc"]),
            jnum(&b["logloss"]),
            jnum(&b["brier"]),
            jnum(&b["ap"]),
        ],
        1e-12,
        "auc, log loss, brier, average precision",
    );
    all_close(&nums(&v[1]), &jnums(&b["roc_fpr"]), 1e-12, "roc fpr");
    all_close(&nums(&v[2]), &jnums(&b["roc_tpr"]), 1e-12, "roc tpr");
    all_close(&nums(&v[3]), &jnums(&b["roc_th"]), 1e-12, "roc thresholds");
    all_close(&nums(&v[4]), &jnums(&b["pr_p"]), 1e-12, "pr precision");
    all_close(&nums(&v[5]), &jnums(&b["pr_r"]), 1e-12, "pr recall");
    all_close(&nums(&v[6]), &jnums(&b["pr_th"]), 1e-12, "pr thresholds");
    all_close(
        &nums(&v[7]),
        &jnums(&b["cal_true"]),
        1e-12,
        "calibration observed",
    );
    all_close(
        &nums(&v[8]),
        &jnums(&b["cal_pred"]),
        1e-12,
        "calibration predicted",
    );
    all_close(
        &nums(&v[9]),
        &jnums(&b["calq_true"]),
        1e-12,
        "quantile calibration observed",
    );
    all_close(
        &nums(&v[10]),
        &jnums(&b["calq_pred"]),
        1e-12,
        "quantile calibration predicted",
    );
    let want = [
        jnum(&b["acc"]),
        jnum(&b["prec"]),
        jnum(&b["rec"]),
        jnum(&b["f1"]),
    ];
    all_close(&nums(&v[11]), &want, 1e-12, "at_threshold");
    all_close(&nums(&v[12]), &want, 1e-12, "label metrics");
}

#[test]
fn multiclass_metrics_match_sklearn() {
    let r = reference();
    let m = &r["multi_metrics"];
    let v = items(&run(r#"
let fit = learn.logistic(multi, "label", ["u", "v", "w"])
let y = multi["label"]
let p = learn.predict(fit, multi)
let c = learn.confusion(y, p)
[[learn.accuracy(y, p), learn.f1(y, p), learn.f1(y, p, #{ "average": "micro" }),
  learn.f1(y, p, #{ "average": "weighted" }), learn.precision(y, p), learn.recall(y, p),
  learn.log_loss(y, learn.predict(fit, multi, #{ "kind": "probs" }))],
 flat_map(range(0, 3), (i) => [col(c, "a")[i], col(c, "b")[i], col(c, "c")[i]])]
"#));
    all_close(
        &nums(&v[0]),
        &[
            jnum(&m["acc"]),
            jnum(&m["f1_macro"]),
            jnum(&m["f1_micro"]),
            jnum(&m["f1_weighted"]),
            jnum(&m["prec_macro"]),
            jnum(&m["rec_macro"]),
            jnum(&m["logloss"]),
        ],
        1e-9,
        "multiclass metrics",
    );
    all_close(
        &nums(&v[1]),
        &jnums(&m["confusion"]),
        0.0,
        "confusion matrix",
    );
}

#[test]
fn ensembles_score_within_a_band_of_sklearn() {
    let r = reference();
    let v = nums(&run(r#"
let cx = ["a", "b", "c", "d"]
let rx = ["x1", "x2", "x3"]
let ctr = ods.head(cls, 400)
let cte = ods.tail(cls, 100)
let rtr = ods.head(reg, 300)
let rte = ods.tail(reg, 100)
let rf = learn.forest(ctr, "churn", cx, #{ "n_trees": 300, "oob": true, "seed": 1 })
let gb = learn.boost(ctr, "churn", cx, #{ "seed": 1 })
let gbp = learn.predict(gb, cte, #{ "kind": "prob" })
[learn.auc(cte["churn"], learn.predict(rf, cte, #{ "kind": "prob" })), rf.oob_score,
 learn.auc(cte["churn"], gbp), learn.log_loss(cte["churn"], gbp),
 learn.rmse(rte["y"], learn.predict(learn.boost(rtr, "y", rx, #{ "seed": 1 }), rte)),
 learn.rmse(rte["y"], learn.predict(learn.forest(rtr, "y", rx, #{ "n_trees": 200, "seed": 1 }), rte))]
"#));
    close(v[0], jnum(&r["rfc"]["auc"]), 0.03, "forest AUC");
    close(v[1], jnum(&r["rfc"]["oob"]), 0.04, "forest OOB accuracy");
    close(v[2], jnum(&r["gbc"]["auc"]), 0.03, "boosted AUC");
    close(v[3], jnum(&r["gbc"]["logloss"]), 0.1, "boosted log loss");
    close(v[4], jnum(&r["gbr_rmse"]), 0.05, "boosted RMSE");
    close(v[5], jnum(&r["rfr_rmse"]), 0.05, "forest RMSE");
}

#[test]
fn seeded_results_do_not_depend_on_threads_or_runs() {
    let program = r#"
let x = ["a", "b", "c", "d"]
let rf = learn.forest(cls, "churn", x, #{ "n_trees": 40, "seed": 9 })
let gb = learn.boost(cls, "churn", x, #{ "n_trees": 30, "subsample": 0.7, "seed": 9 })
let km = learn.kmeans(blobs, ["p", "q"], 4, #{ "seed": 9 })
let s = learn.split(cls, #{ "test": 0.2, "stratify": "churn", "seed": 9 })
[l(learn.predict(rf, cls, #{ "kind": "prob" })), l(learn.predict(gb, cls, #{ "kind": "prob" })),
 l(km.labels), col(s.test, "a"), l(learn.kfold(cls, 5, #{ "seed": 9 }))]
"#;
    let par = run(program);
    let again = run(program);
    let seq = run(&format!("set_parallel(false)\n{program}"));
    assert_eq!(par, again, "a seeded run repeats");
    assert_eq!(par, seq, "one thread or many, the same models");
}

#[test]
fn splits_folds_and_time_splits() {
    let v = items(&run(r#"
let s = learn.split(cls, #{ "test": 0.2, "stratify": "churn", "seed": 3 })
fn yes(f) = len(filter(col(f, "churn"), (c) => c == "yes"))
let ids = l(learn.kfold(cls, 5, #{ "stratify": "churn", "seed": 3 }))
let sizes = map(range(0, 5), (k) => len(filter(ids, (i) => i == k)))
let ts = learn.time_splits(10, 3)
let folds = learn.folds(reg, 4, #{ "seed": 1 })
[ods.n_rows(s.train), ods.n_rows(s.test), yes(s.test) * 1.0 / ods.n_rows(s.test), yes(cls) * 1.0 / ods.n_rows(cls),
 sizes, l(ts[2].train), l(ts[2].test), map(folds, (f) => ods.n_rows(f.test)),
 l(learn.kfold(10, 3, #{ "shuffle": false }))]
"#));
    assert_eq!(num(&v[0]), 400.0);
    assert_eq!(num(&v[1]), 100.0);
    close(num(&v[2]), num(&v[3]), 0.01, "stratified test share");
    assert_eq!(nums(&v[4]), vec![100.0; 5]);
    assert_eq!(nums(&v[5]), (0..8).map(|i| i as f64).collect::<Vec<_>>());
    assert_eq!(nums(&v[6]), vec![8.0, 9.0]);
    assert_eq!(nums(&v[7]), vec![100.0; 4]);
    assert_eq!(
        nums(&v[8]),
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0]
    );
}

#[test]
fn preprocessing_one_hot_imputer_poly() {
    let v = items(&run(r#"
let f = ods.frame([["a", [1.0, (), 3.0, 4.0]], ["s", ["x", (), "y", "x"]], ["n", [1, 2, 3, 4]]])
let oh = learn.transform(learn.one_hot(f, ["s"]), f)
let im = learn.transform(learn.imputer(f, ["a"], #{ "strategy": "median" }), f)
let mf = learn.transform(learn.imputer(f, ["s"], #{ "strategy": "most_frequent" }), f)
let p = learn.poly(f, ["a", "n"], 2)
[ods.columns(oh), col(oh, "s_x"), col(im, "a"), col(mf, "s"), ods.columns(p), col(p, "a*n")]
"#));
    assert_eq!(
        format!("{:?}", v[0]),
        format!("{:?}", eval(r#"["a", "n", "s_x", "s_y"]"#))
    );
    assert_eq!(
        format!("{:?}", v[1]),
        format!("{:?}", eval("[1.0, (), 0.0, 1.0]"))
    );
    assert_eq!(nums(&v[2]), vec![1.0, 3.0, 3.0, 4.0]);
    assert_eq!(
        format!("{:?}", v[3]),
        format!("{:?}", eval(r#"["x", "x", "y", "x"]"#))
    );
    assert_eq!(
        format!("{:?}", v[4]),
        format!("{:?}", eval(r#"["a", "s", "n", "a^2", "a*n", "n^2"]"#))
    );
    assert_eq!(
        format!("{:?}", v[5]),
        format!("{:?}", eval("[1.0, (), 9.0, 16.0]"))
    );
}

#[test]
fn evaluation_cross_validates_searches_and_explains() {
    let v = items(&run(r#"
use ml
let x = ["a", "b", "c", "d", "seg"]
let lr = ml.logistic(cls, y: "churn", x: x)
let cv = ml.cross_validate(lr, cls, folds: 5, seed: 2)
let cvf = ml.cross_validate((d) => ml.logistic(d, y: "churn", x: x), cls, folds: 5, stratify: "churn", seed: 2)
let gs = ml.grid_search(ml.spec("tree", y: "churn", x: x, options: #{ "seed": 1 }), cls,
                        #{ "max_depth": [1, 3, 12] }, metric: "auc", seed: 2)
let pi = ml.permutation_importance(lr, cls, seed: 1)
let pd = ml.partial_dependence(lr, cls, "a", points: 5)
let lc = ml.learning_curve(lr, cls, sizes: [0.2, 1.0], metric: "accuracy")
[ods.columns(cv), ods.n_rows(cv), col(cv, "auc"), ods.n_rows(cvf), gs.best.max_depth,
 ods.n_rows(gs.results), col(pi, "feature")[0], col(pd, "mean"), l(lc["n_train"])]
"#));
    assert_eq!(
        format!("{:?}", v[0]),
        format!(
            "{:?}",
            eval(r#"["fold", "n_train", "n_test", "accuracy", "auc"]"#)
        )
    );
    assert_eq!(num(&v[1]), 5.0);
    assert!(
        nums(&v[2]).iter().all(|a| *a > 0.7 && *a < 0.95),
        "{:?}",
        v[2]
    );
    assert_eq!(num(&v[3]), 5.0);
    assert_eq!(
        num(&v[4]),
        3.0,
        "depth 3 beats a stump and an overgrown tree"
    );
    assert_eq!(num(&v[5]), 3.0);
    assert_eq!(v[6], Value::String(std::sync::Arc::new("a".to_string())));
    let pd = nums(&v[7]);
    assert!(
        pd.windows(2).all(|w| w[1] > w[0]),
        "a's coefficient is positive: {:?}",
        pd
    );
    assert_eq!(nums(&v[8]), vec![80.0, 400.0]);
}

#[test]
fn the_keyword_layer_is_the_native_module() {
    let v = items(&run(r#"
use ml
let a = ml.linear(reg, y: "y", x: ["x1", "x2"])
let b = learn.linear(reg, "y", ["x1", "x2"])
let s = ml.split(reg, test: 0.3, seed: 4)
let t = learn.split(reg, #{ "test": 0.3, "seed": 4 })
[a.params == b.params, col(s.test, "x1") == col(t.test, "x1"),
 ml.rmse(reg["y"], ml.predict(a, reg)) == learn.rmse(reg["y"], learn.predict(b, reg))]
"#));
    assert!(v.iter().all(|b| *b == Value::Boolean(true)), "{:?}", v);
}

#[test]
fn nulls_drop_rows_and_predict_null() {
    let v = items(&run(r#"
let f = ods.frame([["x", [1.0, 2.0, (), 4.0, 5.0, 6.0]], ["y", [2.1, 3.9, 6.0, 8.2, 9.9, 12.1]]])
let fit = learn.linear(f, "y", ["x"])
[fit.n, fit.dropped, l(learn.predict(fit, f))[2]]
"#));
    assert_eq!(num(&v[0]), 5.0);
    assert_eq!(num(&v[1]), 1.0);
    assert_eq!(v[2], Value::Unit);
}

#[test]
fn misuse_is_named() {
    let unknown = eval_tier(
        &format!("{READ}\nlearn.forest(cls, \"churn\", (), #{{ \"trees\": 10 }})"),
        None,
    )
    .expect_err("unknown option");
    assert!(
        unknown.contains("unknown option 'trees'") && unknown.contains("n_trees"),
        "{unknown}"
    );
    let metric = eval_tier(
        &format!("{READ}\nlearn.score(learn.linear(reg, \"y\", [\"x1\"]), reg, \"auc\")"),
        None,
    )
    .expect_err("wrong metric");
    assert!(metric.contains("classification metric"), "{metric}");
    let missing = eval_tier(
        &format!("{READ}\nlearn.predict(learn.linear(reg, \"y\", [\"x1\"]), ods.frame([[\"x2\", [1.0]]]))"),
        None,
    )
    .expect_err("missing column");
    assert!(missing.contains("x1"), "{missing}");
}

#[test]
fn tiers_agree_on_a_modelling_workflow() {
    let source = format!(
        "{READ}\n{}",
        r#"
fn evaluate(seed) = {
    let s = learn.split(cls, #{ "test": 0.25, "stratify": "churn", "seed": seed })
    let m = learn.boost(s.train, "churn", ["a", "b", "c", "d", "seg"], #{ "n_trees": 20, "seed": seed })
    learn.auc(s.test["churn"], learn.predict(m, s.test, #{ "kind": "prob" }))
}
map(range(0, 6), evaluate)
"#
    );
    let a = eval_tier(&source, None).expect("interpreter");
    let b = eval_tier(&source, Some(2)).expect("bytecode tier");
    assert_eq!(a, b);
}

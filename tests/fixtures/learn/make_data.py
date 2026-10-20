"""Fixture datasets and sklearn/numpy reference values for olang's learn tests.

Run: venv/bin/python make_data.py <fixture dir>
Writes reg.csv, cls.csv, multi.csv, blobs.csv, scores.csv and reference.json.
"""
import json, sys, os
import numpy as np
from scipy import stats
from sklearn import linear_model as lm, tree, neighbors, naive_bayes, cluster, decomposition, metrics, mixture, ensemble, preprocessing

out = sys.argv[1]
rng = np.random.default_rng(20261020)
ref = {}

def write(name, cols):
    keys = list(cols)
    n = len(cols[keys[0]])
    with open(os.path.join(out, name), "w") as f:
        f.write(",".join(keys) + "\n")
        for i in range(n):
            row = []
            for k in keys:
                v = cols[k][i]
                if isinstance(v, (float, np.floating)):
                    row.append(repr(float(v)))
                else:
                    row.append(str(v))
            f.write(",".join(row) + "\n")

# ---- regression ----
n = 400
x1 = rng.normal(0, 1, n); x2 = rng.normal(2, 1.5, n); x3 = rng.uniform(-1, 1, n)
hood = rng.choice(["east", "north", "west"], n)
eff = {"east": 0.0, "north": 1.2, "west": -0.7}
y = 3 + 2 * x1 - 1.5 * x2 + 0.5 * x3 + np.array([eff[h] for h in hood]) + rng.normal(0, 1, n)
write("reg.csv", {"x1": x1, "x2": x2, "x3": x3, "hood": hood, "y": y})
X = np.column_stack([x1, x2, x3, (hood == "north").astype(float), (hood == "west").astype(float)])
Xi = np.column_stack([np.ones(n), X])
beta, *_ = np.linalg.lstsq(Xi, y, rcond=None)
res = y - Xi @ beta
df = n - Xi.shape[1]
s2 = res @ res / df
cov = s2 * np.linalg.inv(Xi.T @ Xi)
se = np.sqrt(np.diag(cov))
tv = beta / se
pv = 2 * stats.t.sf(np.abs(tv), df)
r2 = 1 - res @ res / ((y - y.mean()) @ (y - y.mean()))
x0 = np.array([1, 0.5, 1.0, 0.0, 1.0, 0.0])
se_mean = np.sqrt(x0 @ cov @ x0)
tq = stats.t.ppf(0.975, df)
ll = -n / 2 * (np.log(2 * np.pi) + np.log(res @ res / n) + 1)
ref["ols"] = {"coef": beta.tolist(), "se": se.tolist(), "t": tv.tolist(), "p": pv.tolist(), "r2": r2,
              "adj_r2": 1 - (1 - r2) * (n - 1) / df, "aic": -2 * ll + 2 * 6, "bic": -2 * ll + 6 * np.log(n),
              "pred": float(x0 @ beta), "conf": [float(x0 @ beta - tq * se_mean), float(x0 @ beta + tq * se_mean)],
              "pi": [float(x0 @ beta - tq * np.sqrt(se_mean**2 + s2)), float(x0 @ beta + tq * np.sqrt(se_mean**2 + s2))]}
r = lm.Ridge(alpha=5.0).fit(X, y)
ref["ridge"] = [float(r.intercept_)] + r.coef_.tolist()
l = lm.Lasso(alpha=0.1, tol=1e-12, max_iter=100000).fit(X, y)
ref["lasso"] = [float(l.intercept_)] + l.coef_.tolist()
en = lm.ElasticNet(alpha=0.2, l1_ratio=0.3, tol=1e-12, max_iter=100000).fit(X, y)
ref["enet"] = [float(en.intercept_)] + en.coef_.tolist()
Xn = np.column_stack([x1, x2, x3])
dt = tree.DecisionTreeRegressor(max_depth=4, random_state=0).fit(Xn, y)
ref["tree_reg"] = {"pred": dt.predict(Xn[:20]).tolist(), "imp": dt.feature_importances_.tolist()}
kn = neighbors.KNeighborsRegressor(n_neighbors=7).fit(Xn[:300], y[:300])
ref["knn_reg"] = kn.predict(Xn[300:]).tolist()
knd = neighbors.KNeighborsRegressor(n_neighbors=7, weights="distance").fit(Xn[:300], y[:300])
ref["knn_reg_dist"] = knd.predict(Xn[300:]).tolist()
ss = preprocessing.StandardScaler().fit(Xn); mm = preprocessing.MinMaxScaler().fit(Xn); rb = preprocessing.RobustScaler().fit(Xn)
ref["scalers"] = {"standard": ss.transform(Xn[:3]).tolist(), "minmax": mm.transform(Xn[:3]).tolist(), "robust": rb.transform(Xn[:3]).tolist()}
p = decomposition.PCA().fit(Xn)
ref["pca"] = {"ev": p.explained_variance_.tolist(), "ratio": p.explained_variance_ratio_.tolist(),
              "components": p.components_.tolist(), "scores": p.transform(Xn[:3]).tolist()}
gb = ensemble.GradientBoostingRegressor(n_estimators=100, max_depth=3, learning_rate=0.1, random_state=0).fit(Xn[:300], y[:300])
ref["gbr_rmse"] = float(np.sqrt(np.mean((gb.predict(Xn[300:]) - y[300:]) ** 2)))
rf = ensemble.RandomForestRegressor(n_estimators=200, random_state=0).fit(Xn[:300], y[:300])
ref["rfr_rmse"] = float(np.sqrt(np.mean((rf.predict(Xn[300:]) - y[300:]) ** 2)))
ref["ols3_rmse"] = float(np.sqrt(np.mean((lm.LinearRegression().fit(Xn[:300], y[:300]).predict(Xn[300:]) - y[300:]) ** 2)))

# ---- binary classification ----
n = 500
a = rng.normal(0, 1, n); b = rng.normal(0, 1, n); c = rng.uniform(0, 3, n); d = rng.normal(1, 2, n)
seg = rng.choice(["basic", "plus", "pro"], n, p=[0.5, 0.3, 0.2])
segeff = {"basic": 0.0, "plus": 0.6, "pro": -0.8}
eta = -0.5 + 1.2 * a - 0.8 * b + 0.4 * c + np.array([segeff[s] for s in seg])
yb = rng.uniform(size=n) < 1 / (1 + np.exp(-eta))
ylab = np.where(yb, "yes", "no")
write("cls.csv", {"a": a, "b": b, "c": c, "d": d, "seg": seg, "churn": ylab})
Xc = np.column_stack([a, b, c, d, (seg == "plus").astype(float), (seg == "pro").astype(float)])
lr = lm.LogisticRegression(penalty=None, tol=1e-12, max_iter=10000).fit(Xc, yb)
co = np.concatenate([lr.intercept_, lr.coef_[0]])
pp = lr.predict_proba(Xc)[:, 1]
Xci = np.column_stack([np.ones(n), Xc])
H = Xci.T @ (Xci * (pp * (1 - pp))[:, None])
sel = np.sqrt(np.diag(np.linalg.inv(H)))
ref["logit"] = {"coef": co.tolist(), "se": sel.tolist(), "p": (2 * stats.norm.sf(np.abs(co / sel))).tolist(), "prob": pp[:10].tolist()}
lr2 = lm.LogisticRegression(C=0.5, tol=1e-12, max_iter=10000).fit(Xc, yb)
ref["logit_c"] = np.concatenate([lr2.intercept_, lr2.coef_[0]]).tolist()
lr3 = lm.LogisticRegression(penalty=None, class_weight="balanced", tol=1e-12, max_iter=10000).fit(Xc, yb)
ref["logit_bal"] = np.concatenate([lr3.intercept_, lr3.coef_[0]]).tolist()
Xcn = np.column_stack([a, b, c, d])
nb = naive_bayes.GaussianNB().fit(Xcn, yb)
ref["nb_prob"] = nb.predict_proba(Xcn[:10])[:, 1].tolist()
dtc = tree.DecisionTreeClassifier(max_depth=3, random_state=0).fit(Xcn, yb)
ref["tree_cls"] = {"prob": dtc.predict_proba(Xcn)[:, 1].tolist(), "imp": dtc.feature_importances_.tolist()}
dte = tree.DecisionTreeClassifier(max_depth=3, criterion="entropy", random_state=0).fit(Xcn, yb)
ref["tree_cls_entropy"] = dte.predict_proba(Xcn)[:, 1].tolist()
knc = neighbors.KNeighborsClassifier(n_neighbors=9).fit(Xcn[:400], yb[:400])
ref["knn_cls_prob"] = knc.predict_proba(Xcn[400:])[:, 1].tolist()
rfc = ensemble.RandomForestClassifier(n_estimators=300, random_state=0, oob_score=True).fit(Xcn[:400], yb[:400])
ref["rfc"] = {"auc": metrics.roc_auc_score(yb[400:], rfc.predict_proba(Xcn[400:])[:, 1]), "oob": rfc.oob_score_,
              "acc": metrics.accuracy_score(yb[400:], rfc.predict(Xcn[400:]))}
gbc = ensemble.GradientBoostingClassifier(n_estimators=100, max_depth=3, random_state=0).fit(Xcn[:400], yb[:400])
ref["gbc"] = {"auc": metrics.roc_auc_score(yb[400:], gbc.predict_proba(Xcn[400:])[:, 1]),
              "logloss": metrics.log_loss(yb[400:], gbc.predict_proba(Xcn[400:])[:, 1])}
ref["logit_test_auc"] = metrics.roc_auc_score(yb[400:], lm.LogisticRegression(penalty=None, max_iter=10000).fit(Xcn[:400], yb[:400]).predict_proba(Xcn[400:])[:, 1])

# ---- multiclass ----
n = 450
u = rng.normal(0, 1, n); v = rng.normal(0, 1, n); w = rng.normal(0, 1, n)
logits = np.column_stack([np.zeros(n), 1.0 + 1.5 * u - v, -0.5 - u + 2 * w])
pr = np.exp(logits); pr /= pr.sum(1, keepdims=True)
cls = np.array([rng.choice(["a", "b", "c"], p=row) for row in pr])
write("multi.csv", {"u": u, "v": v, "w": w, "label": cls})
Xm = np.column_stack([u, v, w])
mn = lm.LogisticRegression(penalty=None, tol=1e-12, max_iter=10000).fit(Xm, cls)
ref["softmax_prob"] = mn.predict_proba(Xm[:10]).tolist()
cm = mn.coef_ - mn.coef_.mean(0); im = mn.intercept_ - mn.intercept_.mean()
ref["softmax_coef"] = np.column_stack([im, cm]).tolist()
mn2 = lm.LogisticRegression(C=0.3, tol=1e-12, max_iter=10000).fit(Xm, cls)
ref["softmax_c_prob"] = mn2.predict_proba(Xm[:10]).tolist()
nbm = naive_bayes.GaussianNB().fit(Xm, cls)
ref["nb_multi_prob"] = nbm.predict_proba(Xm[:5]).tolist()
pm = mn.predict(Xm)
ref["multi_metrics"] = {"acc": metrics.accuracy_score(cls, pm),
    "f1_macro": metrics.f1_score(cls, pm, average="macro"), "f1_micro": metrics.f1_score(cls, pm, average="micro"),
    "f1_weighted": metrics.f1_score(cls, pm, average="weighted"), "prec_macro": metrics.precision_score(cls, pm, average="macro"),
    "rec_macro": metrics.recall_score(cls, pm, average="macro"),
    "confusion": metrics.confusion_matrix(cls, pm, labels=["a", "b", "c"]).tolist(),
    "logloss": metrics.log_loss(cls, mn.predict_proba(Xm))}

# ---- blobs ----
centers = np.array([[0, 0], [6, 1], [2, 7], [8, 8]])
pts = np.vstack([c + rng.normal(0, 1.0, (150, 2)) for c in centers])
write("blobs.csv", {"p": pts[:, 0], "q": pts[:, 1]})
km = cluster.KMeans(n_clusters=4, n_init=10, random_state=0).fit(pts)
ref["kmeans_inertia"] = float(km.inertia_)
ref["silhouette"] = float(metrics.silhouette_score(pts, km.labels_))
gm = mixture.GaussianMixture(n_components=4, random_state=0).fit(pts)
ref["gmm"] = {"weights": sorted(gm.weights_.tolist()), "score": float(gm.score(pts)), "bic": float(gm.bic(pts))}

# ---- scores (binary metrics) ----
n = 300
yt = rng.uniform(size=n) < 0.35
sc = np.clip(0.35 + 0.25 * (yt - 0.35) + rng.normal(0, 0.18, n), 0.001, 0.999)
sc = np.round(sc, 2)  # ties on purpose
write("scores.csv", {"y": yt.astype(int), "p": sc})
fpr, tpr, th = metrics.roc_curve(yt, sc)
pre, rec, th2 = metrics.precision_recall_curve(yt, sc)
pt, pp2 = __import__("sklearn.calibration", fromlist=["calibration_curve"]).calibration_curve(yt, sc, n_bins=10)
ptq, ppq = __import__("sklearn.calibration", fromlist=["calibration_curve"]).calibration_curve(yt, sc, n_bins=5, strategy="quantile")
yhat = sc >= 0.5
ref["binary"] = {"auc": metrics.roc_auc_score(yt, sc), "logloss": metrics.log_loss(yt, sc), "brier": metrics.brier_score_loss(yt, sc),
    "ap": metrics.average_precision_score(yt, sc), "roc_n": len(fpr), "roc_fpr": fpr.tolist(), "roc_tpr": tpr.tolist(), "roc_th": th[1:].tolist(),
    "pr_n": len(pre), "pr_p": pre.tolist(), "pr_r": rec.tolist(), "pr_th": th2.tolist(),
    "cal_true": pt.tolist(), "cal_pred": pp2.tolist(), "calq_true": ptq.tolist(), "calq_pred": ppq.tolist(),
    "acc": metrics.accuracy_score(yt, yhat), "prec": metrics.precision_score(yt, yhat), "rec": metrics.recall_score(yt, yhat), "f1": metrics.f1_score(yt, yhat),
    "mape": metrics.mean_absolute_percentage_error(sc + 0.1, sc), "r2": metrics.r2_score(sc + rng.normal(0,0.01,n)*0, sc*0.9+0.03)}
with open(os.path.join(out, "reference.json"), "w") as f:
    json.dump(ref, f, indent=1)
print("ok")

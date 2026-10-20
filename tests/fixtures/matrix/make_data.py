"""Writes data.csv and reference.json for tests/matrix_formula_test.rs:
what NumPy, SciPy and pandas compute on a seeded dataset. Run with a
Python that has numpy, scipy and pandas: `python make_data.py .`
The tests never run Python.

The formula references are least squares on designs built here by hand
with patsy's coding (treatment contrasts; an interaction whose margins
are absent spans every cell but one), so a fit's fitted values, R² and
the coefficients outside a recoded block agree whatever names a coding
gives the block's columns."""

import json
import sys

import numpy as np
import pandas as pd
from scipy.spatial.distance import cdist
from scipy.stats import rankdata

out = sys.argv[1] if len(sys.argv) > 1 else "."
rng = np.random.default_rng(20261020)
n = 400
x1 = rng.normal(0.0, 1.0, n).round(6)
x2 = rng.normal(2.0, 0.5, n).round(6)
x3 = (x1 * 0.3 + rng.normal(0.0, 1.0, n)).round(6)
hood = rng.choice(["east", "north", "south", "west"], n)
kind = rng.choice(["flat", "house", "loft"], n)
hour = rng.integers(0, 4, n)
eff_h = {"east": 0.0, "north": 1.5, "south": -0.7, "west": 0.4}
eff_kh = {(k, h): ((ord(k[0]) % 7) - 3) * 0.3 + h * 0.25 * (1 if k == "loft" else -1) for k in ["flat", "house", "loft"] for h in range(4)}
price = (10.0 + 2.0 * x1 - 1.5 * x2 + 0.8 * x1 ** 2 + np.array([eff_h[h] for h in hood])
         + np.array([eff_kh[(k, h)] for k, h in zip(kind, hour)]) + rng.normal(0.0, 0.3, n)).round(6)
w = x2.copy()
w[::17] = np.nan
df = pd.DataFrame({"x1": x1, "x2": x2, "x3": x3, "w": w, "hood": hood, "kind": kind, "hour": hour, "price": price})
df.to_csv(f"{out}/data.csv", index=False, na_rep="")

X = df[["x1", "x2", "x3"]].to_numpy()
y = price
G = X.T @ X
b = np.array([0.5, -1.0, 2.0])
L = np.linalg.cholesky(G)
vals, vecs = np.linalg.eigh(G)
order = np.argsort(-vals)
vals, vecs = vals[order], vecs[:, order]
for j in range(vecs.shape[1]):
    big = np.argmax(np.abs(vecs[:, j]))
    if vecs[big, j] < 0:
        vecs[:, j] = -vecs[:, j]
P, C = X[:60], X[:5]
num = df[["x1", "x2", "x3", "w"]]


def ranked_pairwise(frame):
    k = frame.shape[1]
    r = np.zeros((k, k))
    for i in range(k):
        for j in range(k):
            m = frame.iloc[:, i].notna() & frame.iloc[:, j].notna()
            a = rankdata(frame.iloc[:, i][m])
            c = rankdata(frame.iloc[:, j][m])
            r[i, j] = np.corrcoef(a, c)[0, 1]
    return r


def dummies(col, levels, drop_first):
    lv = levels[1:] if drop_first else levels
    return np.column_stack([(col == l).astype(float) for l in lv])


def ols(D, y):
    beta, *_ = np.linalg.lstsq(D, y, rcond=None)
    fitted = D @ beta
    sse = ((y - fitted) ** 2).sum()
    sst = ((y - y.mean()) ** 2).sum()
    return beta, fitted, 1 - sse / sst


hoods = sorted(set(hood))
kinds = sorted(set(kind))
hours = sorted(set(hour))
cells = np.column_stack([((kind == k) & (hour == h)).astype(float) for k in kinds for h in hours])[:, 1:]
D1 = np.column_stack([np.ones(n), x1, x2, x1 ** 2, dummies(hood, hoods, True), cells])
beta1, fit1, r2_1 = ols(D1, price)
q = df.head(6)
qcells = np.column_stack([((q.kind == k) & (q.hour == h)).astype(float) for k in kinds for h in hours])[:, 1:]
Q1 = np.column_stack([np.ones(6), q.x1, q.x2, q.x1 ** 2, dummies(q.hood.to_numpy(), hoods, True), qcells])
kd = dummies(kind, kinds, True)
D2 = np.column_stack([np.ones(n), x1, kd, kd * x1[:, None]])
beta2, fit2, r2_2 = ols(D2, np.log(price + 10.0))
D3 = np.column_stack([dummies(hood, hoods, False), x1])
beta3, fit3, _ = ols(D3, price)

ref = {
    "gram": G.ravel().tolist(),
    "xb": (X @ b)[:10].tolist(),
    "xty": (X.T @ y).tolist(),
    "solve": np.linalg.solve(G, X.T @ y).tolist(),
    "inv": np.linalg.inv(G).ravel().tolist(),
    "cholesky": L.ravel().tolist(),
    "eig_values": vals.tolist(),
    "eig_vectors": vecs.ravel().tolist(),
    "mean0": X.mean(axis=0).tolist(),
    "std0": X.std(axis=0, ddof=1).tolist(),
    "mean1": X.mean(axis=1)[:10].tolist(),
    "cdist_sqeuclidean": cdist(P, C, "sqeuclidean").T.ravel().tolist(),
    "cdist_euclidean": cdist(P, C, "euclidean").T.ravel().tolist(),
    "cdist_cityblock": cdist(P, C, "cityblock").T.ravel().tolist(),
    "cdist_cosine": cdist(P, C, "cosine").T.ravel().tolist(),
    "corr_pearson": num.corr().to_numpy().ravel().tolist(),
    "corr_spearman": ranked_pairwise(num).ravel().tolist(),
    "corr_spearman_pandas": num.corr(method="spearman").to_numpy().ravel().tolist(),
    "cov": num.cov().to_numpy().ravel().tolist(),
    "f1_head": beta1[:4].tolist(),
    "f1_hood": beta1[4:7].tolist(),
    "f1_r2": r2_1,
    "f1_k": D1.shape[1],
    "f1_fitted": fit1[:10].tolist(),
    "f1_predict": (Q1 @ beta1).tolist(),
    "f2_beta": beta2.tolist(),
    "f2_r2": r2_2,
    "f3_beta": beta3.tolist(),
}
with open(f"{out}/reference.json", "w") as fh:
    json.dump(ref, fh, indent=1)

"""The NumPy side of benchmarks/matrix.ol: the same data, the same
operations through NumPy, SciPy and pandas; milliseconds, the fastest of
seven rounds.

    python3 benchmarks/matrix_numpy.py
"""

import time

import numpy as np
import pandas as pd
from scipy.spatial.distance import cdist


def best(f):
    f()
    t = float("inf")
    for _ in range(7):
        t0 = time.perf_counter()
        f()
        t = min(t, (time.perf_counter() - t0) * 1e3)
    return t


def row(name, ms):
    print(f"{name:<45} {ms:>10.3f} ms")


def columns(n, k):
    i = np.arange(n)
    return pd.DataFrame({f"x{j}": np.sin(i * (0.001 * (j + 1))) + ((i * (j + 7)) % 97) * 0.01 for j in range(k)})


n = 1_000_000
f = columns(n, 10)
X = f.to_numpy()
b = np.arange(10) * 0.1 - 0.4
y = f["x0"].to_numpy() * 2.0 - f["x3"].to_numpy()
B = np.array([[j, 1.0, -j] for j in range(10)], dtype=float)
g20 = (lambda a: a.T @ a)(columns(2000, 20).to_numpy())
g100 = (lambda a: a.T @ a)(columns(5000, 100).to_numpy())
pts = columns(10000, 10).to_numpy()
centres = columns(8, 10).to_numpy()
sample = columns(2000, 10).to_numpy()
small = columns(100000, 10)

print(f"numpy {np.__version__}, pandas {pd.__version__}, n = {n}")
row("df.to_numpy() (10 columns)", best(lambda: f.to_numpy()))
row("X @ b (1M x 10)", best(lambda: X @ b))
row("X @ B (1M x 10 @ 10 x 3)", best(lambda: X @ B))
row("X.T @ y (1M x 10)", best(lambda: X.T @ y))
row("X.T @ X (1M x 10)", best(lambda: X.T @ X))
row("np.linalg.eigh 20 x 20", best(lambda: np.linalg.eigh(g20)))
row("np.linalg.eigh 100 x 100", best(lambda: np.linalg.eigh(g100)))
row("np.linalg.solve 100 x 100", best(lambda: np.linalg.solve(g100, np.ones(100))))
row("np.linalg.inv 100 x 100", best(lambda: np.linalg.inv(g100)))
row("cdist 10,000 x 8 sqeuclidean (k = 10)", best(lambda: cdist(pts, centres, "sqeuclidean")))
row("cdist 2,000 x 2,000 euclidean (k = 10)", best(lambda: cdist(sample, sample)))
row("df.corr() pearson (1M x 10)", best(lambda: f.corr()))
row("df.cov() (1M x 10)", best(lambda: f.cov()))
row("df.corr(spearman) (100k x 10)", best(lambda: small.corr(method="spearman")))

# The formula's design, built as patsy codes it, and least squares.
i = np.arange(n)
g = f.assign(kind=(i % 5 % 3).astype(str), hour=i % 24, y=f.x0 * 2.0 - f.x1 + f.x2)


def formula_fit():
    cell = g.kind + ":" + g.hour.astype(str)
    D = np.column_stack([np.ones(n), g.x0, g.x1, g.x0 ** 2,
                         pd.get_dummies(cell, drop_first=True, dtype=float).to_numpy()])
    return np.linalg.lstsq(D, g.y.to_numpy(), rcond=None)


row("get_dummies + lstsq (1M, 75 columns)", best(formula_fit))

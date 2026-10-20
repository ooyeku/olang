"""The NumPy/pandas side of benches/expr.rs: the same kernels on the same
shapes (10M Float columns, 10% nulls where the Rust row has them, 1000
groups), milliseconds, the fastest of seven rounds. Run with any Python
that has NumPy and pandas: `python3 olang-ods/benches/expr.py [n]`."""

import sys
import time

import numpy as np
import pandas as pd


def best(f):
    f()
    t = float("inf")
    for _ in range(7):
        t0 = time.perf_counter()
        f()
        t = min(t, (time.perf_counter() - t0) * 1e3)
    return t


def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 10_000_000
    rng = np.random.default_rng(1)
    a = rng.uniform(-500, 500, n)
    b = rng.uniform(-500, 500, n)
    c = rng.uniform(-500, 500, n)
    an = a.copy()
    an[rng.random(n) < 0.1] = np.nan
    bn = b.copy()
    bn[rng.random(n) < 0.1] = np.nan
    pa, pan, pbn = pd.Series(a), pd.Series(an), pd.Series(bn)
    ia = rng.integers(0, 1000, n)
    ib = rng.integers(0, 1000, n)
    g = rng.integers(0, 1000, n)
    rows = [
        ("a * b", lambda: a * b),
        ("a * 2.5", lambda: a * 2.5),
        ("a * b (10% nulls, pandas)", lambda: pan * pbn),
        ("exp", lambda: np.exp(a)),
        ("sqrt(abs(a))", lambda: np.sqrt(np.abs(a))),
        ("sigmoid", lambda: 1.0 / (1.0 + np.exp(-a))),
        ("ln(abs(a) + 1)", lambda: np.log(np.abs(a) + 1.0)),
        ("(a*b + c)*0.5 - 1", lambda: (a * b + c) * 0.5 - 1.0),
        ("same, 10% nulls (pandas)", lambda: (pan * pbn + c) * 0.5 - 1.0),
        ("i*3 + j (Int)", lambda: ia * 3 + ib),
        ("sum(a*b)", lambda: np.sum(a * b)),
        ("dot(a, b)", lambda: np.dot(a, b)),
        ("mean((a-b)^2)", lambda: np.mean((a - b) ** 2)),
        ("mean(a > 0)", lambda: np.mean(a > 0)),
        ("log loss", lambda: -np.mean(
            (b > 0) * np.log(1 / (1 + np.exp(-a / 100)))
            + (1 - (b > 0)) * np.log(1 - 1 / (1 + np.exp(-a / 100))))),
        ("where(a > 0, a, b)", lambda: np.where(a > 0, a, b)),
        ("clip(a, -100, 100)", lambda: np.clip(a, -100, 100)),
        ("coalesce(a?, b) (pandas)", lambda: pan.fillna(pd.Series(b))),
        ("fill_null(a?, 0.0) (pandas)", lambda: pan.fillna(0.0)),
        ("between(a, -100, 100) (pandas)", lambda: pa.between(-100, 100)),
        ("ffill (pandas)", lambda: pan.ffill()),
        ("interpolate (pandas)", lambda: pan.interpolate()),
        ("diff (pandas)", lambda: pa.diff()),
        ("pct_change (pandas)", lambda: pd.Series(np.abs(c) + 1).pct_change()),
        ("cumprod", lambda: np.cumprod(a)),
        ("argmax", lambda: np.argmax(a)),
        ("mode (Int, pandas)", lambda: pd.Series(ia).mode()),
        ("searchsorted x 1000", None),
        ("sort descending", lambda: -np.sort(-a)),
        ("argsort descending (stable)", lambda: np.argsort(-a, kind="stable")),
        ("recode (pandas map)", None),
        ("over mean (pandas transform)", lambda: pa.groupby(g).transform("mean")),
        ("over median (pandas transform)", lambda: pa.groupby(g).transform("median")),
        ("sort_by [g asc, a desc] (pandas)", None),
    ]
    sa = np.sort(a)
    probes = np.arange(1000) - 500.0
    labels = pd.Series(np.array([f"k{k}" for k in range(1000)])[ia])
    mapping = {f"k{k}": f"v{k}" for k in range(1000)}
    frame = pd.DataFrame({"g": g, "a": a})
    extra = {
        "searchsorted x 1000": lambda: [np.searchsorted(sa, p) for p in probes],
        "recode (pandas map)": lambda: labels.map(mapping),
        "sort_by [g asc, a desc] (pandas)": lambda: frame.sort_values(
            ["g", "a"], ascending=[True, False], kind="stable"),
    }
    print(f"n = {n}; NumPy {np.__version__}, pandas {pd.__version__}; milliseconds")
    for name, f in rows:
        f = f or extra[name]
        print(f"{name:<34} {best(f):>9.2f}")


main()

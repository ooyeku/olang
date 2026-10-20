"""The pandas side of benchmarks/frames.ol: grouping, joins, crosstabs,
time buckets and rolling windows on the same data, milliseconds, the
fastest of seven rounds.

    python3 benchmarks/frames_pandas.py DATA.csv [n]

writes DATA.csv first when it does not exist (n rows, default 1,000,000:
an hourly timestamp, two Int keys of 1,000 and 50 values, two Floats),
which frames.ol then reads."""

import os
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


def make(path, n):
    i = np.arange(n)
    ts = pd.Timestamp("1990-01-01") + pd.to_timedelta(i, unit="h")
    df = pd.DataFrame({
        "ts": ts.strftime("%Y-%m-%dT%H:%M"),
        "k": (i * 7919) % 1000,
        "k2": (i * 31) % 50,
        "x": np.sin(i * 0.001) * 100 + 500 + (i % 97),
        "y": np.cos(i * 0.37) * 10,
    })
    df.to_csv(path, index=False)


def main():
    path = sys.argv[1]
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 1_000_000
    if not os.path.exists(path):
        make(path, n)
    df = pd.read_csv(path)
    dt = df.assign(ts=pd.to_datetime(df["ts"]))
    right = pd.DataFrame({
        "k": np.repeat(np.arange(1000), 50),
        "k2": np.tile(np.arange(50), 1000),
    })
    right["w"] = right["k"] * 0.5 + right["k2"]
    right["key"] = right["k"] * 50 + right["k2"]
    ri = right.set_index("key")["w"]
    qs = [(i * 7) % 50_000 for i in range(10_000)]
    g = df.groupby("k")
    rows = [
        ("group_by agg: sum, mean, std, median, count", lambda: g.agg(
            s=("x", "sum"), m=("x", "mean"), sd=("x", "std"), med=("x", "median"), n=("x", "size"))),
        ("group_by agg: sum(x * y) / sum(y)", lambda: df.assign(xy=df.x * df.y).groupby("k").agg(
            a=("xy", "sum"), b=("y", "sum")).eval("a / b")),
        ("transform: mean over k", lambda: g["x"].transform("mean")),
        ("x - mean over k (a with column)", lambda: df.assign(d=df.x - g["x"].transform("mean"))),
        ("group_by apply, 1,000 groups", lambda: g.apply(lambda t: t["x"].max() - t["x"].min(), include_groups=False)),
        ("crosstab k2 x (k % 20)", lambda: pd.crosstab(df.k2, df.k % 20)),
        ("join on [k, k2], many_to_one", lambda: df.merge(right, on=["k", "k2"], how="left", validate="many_to_one")),
        ("resample 1d, sum and mean (parsed)", lambda: dt.resample("1D", on="ts").agg({"x": "sum", "y": "mean"})),
        ("resample 1d from text (parse + bucket)", lambda: df.assign(ts=pd.to_datetime(df.ts)).resample("1D", on="ts").agg({"x": "sum", "y": "mean"})),
        ("resample 1mo (parsed)", lambda: dt.resample("MS", on="ts")["x"].sum()),
        ("rolling 365 mean", lambda: df.x.rolling(365).mean()),
        ("rolling 365 std", lambda: df.x.rolling(365).std()),
        ("rolling 365 max", lambda: df.x.rolling(365).max()),
        ("rolling 24 median, centred", lambda: df.x.rolling(24, center=True).median()),
        ("lookup: 10,000 scalar keys, one at a time", lambda: [ri.at[q] for q in qs]),
    ]
    print(f"pandas {pd.__version__}, n = {len(df):,}")
    for name, f in rows:
        print(f"{name:45s} {best(f):10.2f} ms")


if __name__ == "__main__":
    main()

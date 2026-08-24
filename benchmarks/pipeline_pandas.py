# pipeline_pandas.py — the DP4 end-to-end benchmark, pandas edition.
#
# Stage-for-stage identical to benchmarks/pipeline.ol; prints the same
# `STAGE <name> <ms> <checksum>` lines so the runner can refuse any
# result whose answers disagree across engines.

import time

import pandas as pd

started = time.perf_counter()
t = started


def stage(name, t0, checksum):
    now = time.perf_counter()
    print(f"STAGE {name} {round((now - t0) * 1000)} {checksum}")
    return now


# 1. load
sales = pd.read_csv("benchmarks/data/sales.csv")
t = stage("load", t, len(sales))

# 2. clean — drop null-price rows, derive revenue.
clean = sales.dropna().copy()
clean["revenue"] = clean["units"].astype(float) * clean["price"]
t = stage("clean", t, f"{round(clean['revenue'].sum()):.1f}")

# 3. filter — the bulk orders.
bulk = clean[clean["units"] >= 5]
t = stage("filter", t, len(bulk))

# 4. group — revenue and volume per region x category.
grouped = (
    bulk.groupby(["region", "category"], as_index=False)
    .agg(revenue=("revenue", "sum"), avg_units=("units", "mean"), orders=("revenue", "size"))
)
t = stage("group", t, f"{len(grouped)}:{round(grouped['revenue'].max()):.1f}")

# 5. join — attach each region's manager.
dim = pd.read_csv("benchmarks/data/regions.csv")
managed = grouped.merge(dim, on="region", how="inner")
t = stage("join", t, len(managed))

# 6. sort — best cell first.
ranked = managed.sort_values("revenue", ascending=False)
top = f"{ranked.iloc[0]['region']}/{ranked.iloc[0]['category']}"
t = stage("sort", t, top)

# 7. daily — revenue per day, 7-day trailing mean.
daily = bulk.groupby("date", as_index=False)["revenue"].sum().sort_values("date")
smooth = daily["revenue"].rolling(7).mean()
t = stage("daily", t, f"{round(smooth.sum()):.1f}")

# 8. write
ranked.to_csv("benchmarks/data/out_pandas.csv", index=False)
t = stage("write", t, len(ranked))

print(f"TOTAL {round((time.perf_counter() - started) * 1000)}")

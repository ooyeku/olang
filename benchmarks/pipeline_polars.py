# pipeline_polars.py — the DP4 end-to-end benchmark, Polars edition.
#
# Stage-for-stage identical to benchmarks/pipeline.ol; prints the same
# `STAGE <name> <ms> <checksum>` lines so the runner can refuse any
# result whose answers disagree across engines. Eager API, default
# thread pool — Polars as it ships.

import time

import polars as pl

started = time.perf_counter()
t = started


def stage(name, t0, checksum):
    now = time.perf_counter()
    print(f"STAGE {name} {round((now - t0) * 1000)} {checksum}")
    return now


# 1. load
sales = pl.read_csv("benchmarks/data/sales.csv")
t = stage("load", t, sales.height)

# 2. clean — drop null-price rows, derive revenue.
clean = sales.drop_nulls().with_columns(
    (pl.col("units").cast(pl.Float64) * pl.col("price")).alias("revenue")
)
t = stage("clean", t, f"{round(clean['revenue'].sum()):.1f}")

# 3. filter — the bulk orders.
bulk = clean.filter(pl.col("units") >= 5)
t = stage("filter", t, bulk.height)

# 4. group — revenue and volume per region x category.
grouped = bulk.group_by(["region", "category"]).agg(
    pl.col("revenue").sum().alias("revenue"),
    pl.col("units").mean().alias("avg_units"),
    pl.len().alias("orders"),
)
t = stage("group", t, f"{grouped.height}:{round(grouped['revenue'].max()):.1f}")

# 5. join — attach each region's manager.
dim = pl.read_csv("benchmarks/data/regions.csv")
managed = grouped.join(dim, on="region", how="inner")
t = stage("join", t, managed.height)

# 6. sort — best cell first.
ranked = managed.sort("revenue", descending=True)
top = f"{ranked[0, 'region']}/{ranked[0, 'category']}"
t = stage("sort", t, top)

# 7. daily — revenue per day, 7-day trailing mean.
daily = bulk.group_by("date").agg(pl.col("revenue").sum()).sort("date")
smooth = daily["revenue"].rolling_mean(window_size=7)
t = stage("daily", t, f"{round(smooth.sum()):.1f}")

# 8. write
ranked.write_csv("benchmarks/data/out_polars.csv")
t = stage("write", t, ranked.height)

print(f"TOTAL {round((time.perf_counter() - started) * 1000)}")

"""The pandas side of benches/dtypes.rs: the same kernels on the same
shapes (1M rows), milliseconds, the fastest of seven rounds. Run with any
Python that has pandas: `python3 olang-ods/benches/dtypes.py [n]`."""

import sys
import time

import numpy as np
import pandas as pd

NAMES = ["Northgate", "Riverside", "Old Town", "Hillcrest", "Docklands", "Meadowbrook", "Ashford",
         "Brookside", "Cedar Park", "Elmwood", "Fairview", "Glenmore", "Harbourside", "Ironbridge",
         "Kingsway", "Lakeview", "Millbrook", "Northfield", "Oakridge", "Parkside"]


def best(f):
    f()
    t = float("inf")
    for _ in range(7):
        t0 = time.perf_counter()
        f()
        t = min(t, (time.perf_counter() - t0) * 1e3)
    return t


def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 1_000_000
    rng = np.random.default_rng(1)
    start = 18262 * 86400
    secs = start + rng.integers(0, 4 * 365 * 86400, n)
    ts = pd.Series(pd.to_datetime(secs, unit="s"))
    iso = pd.Series(ts.dt.strftime("%Y-%m-%dT%H:%M:%S"))
    days = pd.Series(ts.dt.strftime("%Y-%m-%d"))
    dmy = pd.Series(ts.dt.strftime("%d/%m/%Y %H:%M"))
    words = pd.Series([f" {NAMES[k % 20]} {k // 20 % 10000} " for k in rng.integers(0, 2**40, n)])
    hoods = pd.Series([NAMES[k] for k in rng.integers(0, 20, n)])
    cat = hoods.astype("category")
    codes = rng.integers(0, 1000, n)
    g = pd.Categorical.from_codes(codes, [f"level {i:04}" for i in range(1000)])
    v = rng.integers(0, 1000, n).astype(float)
    fc = pd.DataFrame({"g": g, "v": v})
    fs = pd.DataFrame({"g": pd.Series(g).astype(str), "v": v})
    ordered = cat.cat.as_ordered()
    dates = ts.dt.normalize()
    ints = pd.Series(rng.integers(0, 100_000, n))
    print(f"pandas {pd.__version__}; string dtype {words.dtype}; n = {n}; milliseconds")
    rows = [
        ("to_datetime, ISO", lambda: pd.to_datetime(iso, format="ISO8601")),
        ("to_date, ISO", lambda: pd.to_datetime(days, format="%Y-%m-%d")),
        ('to_datetime, "%d/%m/%Y %H:%M"', lambda: pd.to_datetime(dmy, format="%d/%m/%Y %H:%M")),
        ("hour", lambda: ts.dt.hour),
        ("weekday", lambda: ts.dt.weekday),
        ("year", lambda: ts.dt.year),
        ("month", lambda: ts.dt.month),
        ('floor("1h")', lambda: ts.dt.floor("h")),
        ('floor("1mo")', lambda: ts.dt.to_period("M").dt.to_timestamp()),
        ("lower", lambda: words.str.lower()),
        ("upper", lambda: words.str.upper()),
        ("trim", lambda: words.str.strip()),
        ('contains("ark")', lambda: words.str.contains("ark", regex=False)),
        ('contains("ark", case: false)', lambda: words.str.contains("ARK", case=False, regex=False)),
        ('replace("o", "0")', lambda: words.str.replace("o", "0", regex=False)),
        ("len_chars", lambda: words.str.len()),
        ('split_n(" ", 2)', lambda: words.str.split(" ").str[2]),
        ("to_category", lambda: hoods.astype("category")),
        ("dummies, 20 levels", lambda: pd.get_dummies(cat, prefix="h", drop_first=True, dtype=float)),
        ("lower, a Category", lambda: cat.str.lower()),
        ("group_by(Category) sum", lambda: fc.groupby("g", observed=True, sort=False)["v"].sum()),
        ("group_by(String) sum", lambda: fs.groupby("g", sort=False)["v"].sum()),
        ('col(cat) == "Riverside"', lambda: cat == "Riverside"),
        ('col(cat) < "Kingsway"', lambda: ordered < "Kingsway"),
        ('col(text) + "!"', lambda: words + "!"),
        ('format("%b %d %Y")', lambda: dates.dt.strftime("%b %d %Y")),
        ('format("05d")', lambda: ints.map("{:05d}".format)),
    ]
    for name, f in rows:
        print(f"{name:<34} {best(f):>9.2f}")


if __name__ == "__main__":
    main()

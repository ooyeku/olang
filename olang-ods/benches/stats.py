"""NumPy / SciPy timings for olang-ods/benches/stats.rs (docs/performance.md).

Run: python benches/stats.py [n]. Milliseconds, the fastest of seven.
statsmodels is not in this environment; its acf is NumPy's FFT (as
statsmodels' acovf(fft=True) does), timed here; its Holt-Winters fit is
not timed.
"""

import sys
import time

import numpy as np
from scipy import optimize, stats


def best(f):
    f()
    out = float("inf")
    for _ in range(7):
        t0 = time.perf_counter()
        f()
        out = min(out, (time.perf_counter() - t0) * 1e3)
    return out


n = int(sys.argv[1]) if len(sys.argv) > 1 else 1_000_000
g = np.random.default_rng(1)
x = g.normal(0.0, 1.0, n)
u = g.uniform(0.0, 1.0, n)
print(f"n = {n}; milliseconds (NumPy {np.__version__})")
for name, f in [
    ("norm.cdf", lambda: stats.norm.cdf(x)),
    ("norm.pdf", lambda: stats.norm.pdf(x)),
    ("norm.ppf", lambda: stats.norm.ppf(u)),
    ("t.cdf (df 10)", lambda: stats.t.cdf(x, 10)),
    ("gamma.cdf (shape 2)", lambda: stats.gamma.cdf(u, 2.0)),
    ("rng.normal", lambda: np.random.default_rng(7).normal(0.0, 1.0, n)),
    ("rng.uniform", lambda: np.random.default_rng(7).uniform(0.0, 1.0, n)),
    ("rng.poisson (lam 3)", lambda: np.random.default_rng(7).poisson(3.0, n)),
    ("rng.integers", lambda: np.random.default_rng(7).integers(0, 100, n)),
    ("rng.choice (50 weights)", lambda: np.random.default_rng(7).choice(50, n, p=np.arange(1, 51) / 1275.0)),
]:
    print(f"{name:<34} {best(f):>9.2f}")

small = x[:10_000]
def boot_mean():
    idx = np.random.default_rng(1).integers(0, 10_000, (10_000, 10_000))
    m = small[idx].mean(axis=1)
    np.percentile(m, [2.5, 97.5])
def boot_median():
    r = np.random.default_rng(1)
    m = np.array([np.median(small[r.integers(0, 10_000, 10_000)]) for _ in range(2_000)])
    np.percentile(m, [2.5, 97.5])
print(f"{'bootstrap mean 10k x 10k':<34} {best(boot_mean):>9.2f}")
print(f"{'bootstrap median 10k x 2k':<34} {best(boot_median):>9.2f}")
import pandas as pd
ps = pd.Series(x)
print(f"{'format \",.2f\" (pandas map)':<34} {best(lambda: ps.map('{:,.2f}'.format)):>9.2f}")
print(f"{'format \".1%\" (pandas map)':<34} {best(lambda: ps.map('{:.1%}'.format)):>9.2f}")
print(f"{'mad':<34} {best(lambda: stats.median_abs_deviation(x, scale='normal')):>9.2f}")

walk = np.empty(n)
acc = 0.0
for i in range(n):
    acc = acc * 0.9 + x[i]
    walk[i] = acc


def acf_fft(v, nlags):  # statsmodels acovf(fft=True) / acf
    v = v - v.mean()
    m = len(v)
    size = 1 << (2 * m - 1).bit_length()
    f = np.fft.rfft(v, size)
    acov = np.fft.irfft(f * np.conjugate(f), size)[: nlags + 1] / m
    return acov / acov[0]


print(f"{'acf (100 lags)':<34} {best(lambda: acf_fft(walk, 100)):>9.2f}")
hours = walk[:26_280]
print(f"{'acf 26,280 h (336 lags)':<34} {best(lambda: acf_fft(hours, 336)):>9.2f}")

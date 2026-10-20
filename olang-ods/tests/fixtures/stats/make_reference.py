"""Reference values for olang-ods/tests/stats_reference.rs.

Run with a Python that has NumPy and SciPy (the values in reference.json
came from NumPy 2.5.1 and SciPy 1.18.0):

    python make_reference.py reference.json

The tests never run Python. statsmodels was not installed where these
were made, so its functions are written out below from its source (each
says which): the autocovariances, Bartlett's interval, Yule-Walker
partial autocorrelations (one linear solve per order, not
Levinson-Durbin, so the check is independent of the kernel's method),
seasonal_decompose's filter, and TTestIndPower's power. Holt-Winters is
the recursion olang documents (Winters 1960, the season updated from the
new level), fitted here by SciPy's bounded L-BFGS-B from many starts.
"""

import json
import sys

import numpy as np
from scipy import optimize, stats

rng = np.random.default_rng(20261020)
out = {}

# ── distributions ──────────────────────────────────────────────────────
families = {
    "norm": ([1.5, -0.3], stats.norm(1.5, 2.0 ** 0.5 * 1.3), [-2.0, 0.0, 1.5, 3.7]),
    "t": ([4.5], stats.t(4.5), [-2.5, 0.0, 0.7, 3.0]),
    "chi2": ([3.0], stats.chi2(3.0), [0.1, 1.0, 2.5, 9.0]),
    "f": ([3.0, 11.0], stats.f(3.0, 11.0), [0.2, 1.0, 2.5, 6.0]),
    "lognorm": ([0.4, 0.7], stats.lognorm(0.7, scale=np.exp(0.4)), [0.2, 1.0, 1.5, 6.0]),
    "beta": ([2.5, 4.0], stats.beta(2.5, 4.0), [0.05, 0.3, 0.5, 0.9]),
    "gamma": ([2.2, 1.7], stats.gamma(2.2, scale=1.7), [0.3, 1.0, 3.9, 12.0]),
    "poisson": ([3.4], stats.poisson(3.4), [0, 1, 3, 8]),
    "binom": ([17, 0.31], stats.binom(17, 0.31), [0, 3, 5, 12]),
}
families["norm"] = ([1.5, 1.3], stats.norm(1.5, 1.3), [-2.0, 0.0, 1.5, 3.7])
ps = [0.001, 0.025, 0.4, 0.5, 0.9, 0.999]
dist = {}
for name, (params, d, xs) in families.items():
    discrete = name in ("poisson", "binom")
    dens = d.pmf(xs) if discrete else d.pdf(xs)
    dist[name] = {
        "params": params,
        "x": xs,
        "density": dens.tolist(),
        "cdf": d.cdf(xs).tolist(),
        "p": ps,
        "ppf": d.ppf(ps).tolist(),
    }
out["dist"] = dist

# ── tests ──────────────────────────────────────────────────────────────
a = np.round(rng.normal(10.0, 2.0, 37), 3)
b = np.round(rng.normal(9.0, 3.0, 29), 3)
t = stats.ttest_ind(a, b, equal_var=False)
ci = t.confidence_interval(0.95)
na, nb = len(a), len(b)
pooled = np.sqrt(((na - 1) * a.var(ddof=1) + (nb - 1) * b.var(ddof=1)) / (na + nb - 2))
tg = stats.ttest_ind(a, b, equal_var=False, alternative="greater")
t1 = stats.ttest_1samp(a, 9.5)
ci1 = t1.confidence_interval(0.9)
out["t_test"] = {
    "a": a.tolist(),
    "b": b.tolist(),
    "t": float(t.statistic),
    "df": float(t.df),
    "p": float(t.pvalue),
    "lo": float(ci.low),
    "hi": float(ci.high),
    "d": float((a.mean() - b.mean()) / pooled),
    "p_greater": float(tg.pvalue),
    "lo_greater": float(tg.confidence_interval(0.95).low),
    "one_mu": 9.5,
    "one_t": float(t1.statistic),
    "one_p": float(t1.pvalue),
    "one_lo90": float(ci1.low),
    "one_hi90": float(ci1.high),
    "one_d": float((a.mean() - 9.5) / a.std(ddof=1)),
}

# Mann-Whitney with ties, Spearman
x = np.round(rng.normal(0.0, 1.0, 40), 1)
y = np.round(rng.normal(0.3, 1.0, 33), 1)
mw = stats.mannwhitneyu(x, y, alternative="two-sided", method="asymptotic")
mwl = stats.mannwhitneyu(x, y, alternative="less", method="asymptotic")
u = np.round(rng.normal(0.0, 1.0, 50), 1)
v = np.round(u * 0.6 + rng.normal(0.0, 1.0, 50), 1)
sp = stats.spearmanr(u, v)
out["rank"] = {
    "x": x.tolist(),
    "y": y.tolist(),
    "u": float(mw.statistic),
    "p": float(mw.pvalue),
    "p_less": float(mwl.pvalue),
    "s_x": u.tolist(),
    "s_y": v.tolist(),
    "rho": float(sp.statistic),
    "rho_p": float(sp.pvalue),
}

# robust
r = np.round(np.concatenate([rng.normal(50.0, 5.0, 60), [120.0, -30.0, 95.0]]), 2)
q1, q3 = np.percentile(r, [25, 75])
out["robust"] = {
    "x": r.tolist(),
    "mad_normal": float(stats.median_abs_deviation(r, scale="normal")),
    "mad_raw": float(stats.median_abs_deviation(r)),
    "iqr": float(stats.iqr(r)),
    "fence_lo": float(q1 - 2.0 * (q3 - q1)),
    "fence_hi": float(q3 + 2.0 * (q3 - q1)),
    "z": stats.zscore(r, ddof=1).tolist(),
    "z_robust": ((r - np.median(r)) / stats.median_abs_deviation(r, scale="normal")).tolist(),
}

# ── power (statsmodels TTestIndPower / TTestPower, written out) ────────
def power_t(effect, n, alpha, alternative, two_sample, ratio=1.0):
    if two_sample:
        n2 = n * ratio
        df = n + n2 - 2
        nobs = 1.0 / (1.0 / n + 1.0 / n2)
    else:
        df = n - 1
        nobs = n
    nc = effect * np.sqrt(nobs)
    if alternative == "two-sided":
        upp = stats.t.isf(alpha / 2, df)
        low = stats.t.ppf(alpha / 2, df)
        # SciPy's nct.cdf is NaN far in its tail (where it is ~0)
        return stats.nct.sf(upp, df, nc) + np.nan_to_num(stats.nct.cdf(low, df, nc))
    if alternative == "greater":
        return stats.nct.sf(stats.t.isf(alpha, df), df, nc)
    return stats.nct.cdf(stats.t.ppf(alpha, df), df, nc)

cases = []
for effect, n, alpha, alt, two, ratio in [
    (0.5, 64, 0.05, "two-sided", True, 1.0),
    (0.2, 200, 0.05, "two-sided", True, 1.0),
    (0.8, 10, 0.01, "greater", True, 1.0),
    (0.3, 50, 0.05, "two-sided", False, 1.0),
    (0.4, 40, 0.05, "two-sided", True, 2.0),
    (-0.5, 30, 0.05, "less", True, 1.0),
    (1.5, 6, 0.05, "two-sided", True, 1.0),
]:
    cases.append({
        "effect": effect, "n": n, "alpha": alpha, "alternative": alt,
        "two_sample": two, "ratio": ratio,
        "power": float(power_t(effect, n, alpha, alt, two, ratio)),
    })
solved = []
for effect, power, alt, two in [(0.5, 0.8, "two-sided", True), (0.2, 0.9, "two-sided", True),
                                (0.35, 0.8, "greater", False)]:
    n = optimize.brentq(lambda n: power_t(effect, n, 0.05, alt, two) - power, 2.0001, 2000.0, xtol=1e-12)
    solved.append({"effect": effect, "power": power, "alternative": alt, "two_sample": two, "n": float(n)})
out["power"] = {"cases": cases, "solved": solved}

# ── time series ────────────────────────────────────────────────────────
n = 500
tt = np.arange(n)
series = 20.0 + 0.03 * tt + 4.0 * np.sin(2 * np.pi * tt / 12) + rng.normal(0.0, 1.0, n)
series = np.round(series, 4)

def acovf(x):  # statsmodels acovf(adjusted=False, demean=True)
    x = x - x.mean()
    m = len(x)
    return np.array([np.dot(x[: m - k], x[k:]) / m for k in range(m)])

nlags = 30
acv = acovf(series)
acf = acv[: nlags + 1] / acv[0]
var = np.ones(nlags + 1) / n
var[0] = 0
var[1] = 1.0 / n
var[2:] *= 1 + 2 * np.cumsum(acf[1:-1] ** 2)
z = stats.norm.ppf(0.975)
half = z * np.sqrt(var)

def pacf_yw(x, k_max, adjusted):  # statsmodels yule_walker, one order at a time
    x = x - x.mean()
    m = len(x)
    r = np.array([np.dot(x[: m - k], x[k:]) for k in range(k_max + 1)])
    r = r / (m - np.arange(k_max + 1)) if adjusted else r / m
    out = [1.0]
    for k in range(1, k_max + 1):
        R = np.array([[r[abs(i - j)] for j in range(k)] for i in range(k)])
        out.append(float(np.linalg.solve(R, r[1 : k + 1])[-1]))
    return out

def decompose(x, period, model):  # statsmodels seasonal_decompose, two-sided
    if period % 2 == 0:
        filt = np.array([0.5] + [1.0] * (period - 1) + [0.5]) / period
    else:
        filt = np.repeat(1.0 / period, period)
    trend = np.convolve(x, filt[::-1], mode="valid")
    pad = (len(x) - len(trend)) // 2
    trend = np.concatenate([[np.nan] * pad, trend, [np.nan] * (len(x) - len(trend) - pad)])
    det = x - trend if model == "additive" else x / trend
    avg = np.array([np.nanmean(det[i::period]) for i in range(period)])
    avg = avg - avg.mean() if model == "additive" else avg / avg.mean()
    seasonal = np.tile(avg, len(x) // period + 1)[: len(x)]
    resid = det - seasonal if model == "additive" else det / seasonal
    return trend, seasonal, resid

def nan_null(v):
    return [None if np.isnan(t) else float(t) for t in v]

tr, se, re = decompose(series, 12, "additive")
trm, sem, rem = decompose(series, 12, "multiplicative")

def hw_run(y, m, rates, trend=True, mul=False):
    a, b, g = rates
    l0 = y[:m].mean()
    sl = (y[m : 2 * m].mean() - l0) / m if trend else 0.0
    mid = (m - 1) / 2.0
    line = l0 + sl * (np.arange(m) - mid)
    s = list(y[:m] / line if mul else y[:m] - line)
    lv = l0 + sl * mid
    fitted = [np.nan] * len(y)
    sse = 0.0
    for t in range(m, len(y)):
        si = s[t % m]
        base = lv + sl
        pred = base * si if mul else base + si
        fitted[t] = pred
        sse += (y[t] - pred) ** 2
        last = lv
        lv = a * (y[t] / si if mul else y[t] - si) + (1 - a) * base
        if trend:
            sl = b * (lv - last) + (1 - b) * sl
        s[t % m] = g * (y[t] / lv if mul else y[t] - lv) + (1 - g) * si
    fc = [(lv + k * sl) * s[(len(y) + k - 1) % m] if mul else lv + k * sl + s[(len(y) + k - 1) % m]
          for k in range(1, 13)]
    return fitted, sse, fc

fixed_rates = (0.3, 0.05, 0.2)
fitted, sse, fc = hw_run(series, 12, fixed_rates)
best = None
for start in [(0.1, 0.01, 0.1), (0.5, 0.1, 0.5), (0.2, 0.0, 0.3), (0.9, 0.5, 0.9), (0.05, 0.2, 0.05)]:
    res = optimize.minimize(lambda p: hw_run(series, 12, p)[1], start, method="L-BFGS-B",
                            bounds=[(0, 1)] * 3)
    if best is None or res.fun < best.fun:
        best = res
pos = series + 10.0
fitted_m, sse_m, fc_m = hw_run(pos, 12, fixed_rates, mul=True)

out["ts"] = {
    "y": series.tolist(),
    "acf": acf.tolist(),
    "acf_lo": (acf - half).tolist(),
    "acf_hi": (acf + half).tolist(),
    "pacf_yw": pacf_yw(series, 20, True),
    "pacf_ywm": pacf_yw(series, 20, False),
    "pacf_half": float(z / np.sqrt(n)),
    "trend": nan_null(tr), "seasonal": se.tolist(), "resid": nan_null(re),
    "trend_mul": nan_null(trm), "seasonal_mul": sem.tolist(), "resid_mul": nan_null(rem),
    "hw_rates": list(fixed_rates),
    "hw_fitted": nan_null(np.array(fitted)),
    "hw_sse": float(sse),
    "hw_forecast": fc,
    "hw_best_sse": float(best.fun),
    "hw_mul_fitted": nan_null(np.array(fitted_m)),
    "hw_mul_sse": float(sse_m),
    "hw_mul_forecast": fc_m,
}

with open(sys.argv[1] if len(sys.argv) > 1 else "reference.json", "w") as f:
    json.dump(out, f, indent=1)

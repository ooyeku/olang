# Data ergonomics: the data stack at pandas parity

Status: proposal (2026-10-18). Scope: `ods`, `stats`, `plot`, `random`, plus a new `learn` module and a few small language hooks. Evidence: every code cell of the seven olmd notebooks (`../olmd/*/*.olmd`) and their four generators (`*/data/gen.ol`), each compared with the idiomatic Python (pandas, NumPy, SciPy, statsmodels, scikit-learn, seaborn).

## Contents

- [The problem in numbers](#the-problem-in-numbers)
- [Design principles](#design-principles)
- [The gaps, consolidated](#the-gaps-consolidated)
- [Prerequisites in the language and runtime (L)](#prerequisites-in-the-language-and-runtime-l)
- [The library](#the-library)
  - [1. Columns: dtypes, math, nulls, conditionals](#1-columns-dtypes-math-nulls-conditionals)
  - [2. Text, dates and categories](#2-text-dates-and-categories)
  - [3. Expressions (`ods.col`)](#3-expressions-odscol)
  - [4. Frames: shape, group, join, time](#4-frames-shape-group-join-time)
  - [5. Matrices](#5-matrices)
  - [6. `learn`: models, preprocessing, validation, metrics](#6-learn-models-preprocessing-validation-metrics)
  - [7. `stats`: inference and time series](#7-stats-inference-and-time-series)
  - [8. `random`: generators](#8-random-generators)
  - [9. `plot`: grouping, dates, reference marks](#9-plot-grouping-dates-reference-marks)
  - [10. I/O and formatting](#10-io-and-formatting)
- [Before and after: the olmd notebooks](#before-and-after-the-olmd-notebooks)
- [Performance plan](#performance-plan)
- [Milestones](#milestones)
- [Acceptance criteria](#acceptance-criteria)
- [Decisions to make](#decisions-to-make)

## The problem in numbers

The four data-science notebooks are 1.3–2× longer than equivalent pandas or scikit-learn code. That is not because olang is verbose. It is because a handful of missing primitives force every analysis to rebuild them by hand.

| Notebook | Code lines | Ceremony lines¹ | Hand-written algorithms |
|---|---|---|---|
| housing | 221 | 39 | `choose`, `predict`, rmse/mae/r², k-fold assignment, CV loop, one-hot |
| churn | 207 | 31 | logistic regression (gradient descent), `sigmoid`, `log_loss`, AUC, confusion, ROC, calibration, permutation importance |
| energy | 213 | 26 | `choose`, interpolation, Holt-Winters, ACF, centred rolling mean, run-length events, grid search |
| segments | 193 | 34 | k-means++, Lloyd, silhouette, power-iteration PCA, crosstab |
| sales, experiment, workspace | 36, 58, 38 | 1, 5, 1 | describe-by-hand, power by simulation |

¹ `to_list`, `ods.series(map …)`, `ods.cast`, `to_float`, `to_int`.

The inventory counted 158 workaround sites across 13 categories:

| Category | Sites | Pattern occurrences |
|---|---|---|
| Models, metrics and stats that are missing | 47 | ~45 helper functions |
| Reshape, group, join and lookup | 41 | |
| Type ceremony | 30 | 45 `ods.cast` and 41 `to_float` |
| List round-trips | 27 | 21 `to_list` and 25 `ods.series(map…)` |
| Linear algebra | 22 | 9 `fold(enumerate(xs), …)` standing in for `X @ b` |
| Text, dates and categoricals | 22 | |
| Elementwise math | 21 | 19 string-named `ods.map` calls and 7 `* -1.0` |
| Conditionals and nulls | 20 | |

The rest are plotting, randomness, I/O and notebook UI.

What olang already does better is listed under [What to keep](#what-to-keep), and this design keeps all of it:
- pipelines with `|>`
- explicit `Result`s
- `test` blocks that check analyses against known truth
- params and `--for`
- capabilities in front matter
- `${…}` prose
- one-line reactive controls
- the columnar engine's speed (sum, std, sort and OLS beat NumPy; group-by and ETL are on a par with Polars)

## Design principles

1. **Every high-level call lowers to a native columnar kernel.** No API added here runs an olang closure per row. Per-row olang code stays possible (`to_list` → `map` → `series`), but it is the escape hatch, never the path a common task needs. A closure runs at most once per *group* or once per *call*.
2. **Fusion is the default, not a special case.** Elementwise arithmetic, comparisons, math functions, null handling and conditionals build one expression. It is evaluated chunk by chunk (L1-resident), in parallel past the measured thresholds, and fused into a reduction when one follows. A user never decides whether something fuses.
3. **One idea, one name, three spellings.** `ods.exp(s)`, `s |> ods.exp` and `s.exp()` are the same function. Methods are sugar resolved to the module function (L2), so docs, help and the checker describe one thing.
4. **Columns know their meaning.** Dates, datetimes and categories are dtypes, so `.hour()`, `.weekday()`, one-hot encoding, axes and joins do the right thing without parsing strings.
5. **Models are values with names.** A fit carries its terms, so `fit.params.sqft`, `fit.predict(frame)` and `fit.coef` (a Frame) replace positional indices. Adding a feature cannot silently shift a coefficient.
6. **Keyword arguments for options, and defaults that are right for analysis.** `learn.split(df, test: 0.25, stratify: "churned", seed: 7)`, not a trailing `#{…}`.
7. **Additive under the 1.0 rules.** Every existing function keeps its documented behaviour. New dtypes, verbs, options and modules are "after" items (`docs/stability.md`, append-mostly). The only changes to existing behaviour are a raise becoming a value: `ods.sum(bool_mask)` and unary minus on a Series.
8. **The browser build stays small.** Pure Rust, with no BLAS, Arrow, faer or Polars dependency. Each milestone states its wasm size cost. The models are sized for the shapes analysis meets: many rows, tens of columns, k×k solves.

## The gaps, consolidated

Each gap gets an id, and the library sections below cite them. Rank is frequency × pain.

| Id | Gap | Worst sites | Rank |
|---|---|---|---|
| G1 | **No matrix**: `X @ b` is `fold(enumerate(xs), xs[0]*0.0 + b[0], …)`. Distances, correlation matrices, eigenvectors and projections are all built by hand. | churn 166/175, housing 46/408, energy 241/325, segments 96–113/257–272 | 1 |
| G2 | **Models without names**: no `predict`, no fitted values or residuals, 30 lookups of the form `ods.get(fit.coef, 9)`, query vectors in hand-matched order | housing 359–367/406/443–456, energy 323–325 | 2 |
| G3 | **Missing models**: logistic regression, k-means, silhouette, PCA, Holt-Winters fitting, scaling | churn 162–174 (most of the runtime), segments (~110 lines), energy 217–234 | 3 |
| G4 | **No metrics library**: rmse, mae, r², mape, AUC, ROC, confusion, precision/recall/F1, calibration, log loss, permutation importance | housing 49–54, churn 42–67/291–362, energy 49 | 4 |
| G5 | **No `where`/`select`/`clip`/`coalesce`/`interpolate`/fill by method or by group**: `choose` built from float masks (NaN poisoning), interpolation that breaks on two gaps in a row | housing 40–43/114–116, energy 40–43/67–69/83–88, segments 103–113 | 5 |
| G6 | **Math by string name**: `ods.map(s, "exp")`; no unary minus, so `* -1.0` | 19 + 7 lines, in every DS notebook | 6 |
| G7 | **No `.str`/`.dt` operations**: lower/trim/replace through closures; hour, weekday and month count from substrings or a calendar join; `date_part` returns Strings | housing 94–99/190–192, energy 71–84/319 | 7 |
| G8 | **No one-hot / categorical**: 4 sites, 82 dummy columns (71 interaction dummies) | housing 201, churn 133–137, energy 237/320 | 8 |
| G9 | **Group-by ergonomics**: no transform/`over`, only 5 aggregations (no first, std, median, n_unique), positional `[out, op, col]` triples, no per-group apply, no crosstab, no run grouping | housing 110–117/317, energy 105–108/153–159/348–354, segments 224–228/322–325, churn 367–375 | 9 |
| G10 | **List round-trips**: 46 sites, often over 10k–26k rows | everywhere | 10 (mostly removed by G5–G9) |
| G11 | **Type ceremony**: Int→Float before arithmetic, `ln` or `lm`; Bool→Int to count; Int→String for bar labels; Int vs Float sliders | 86 lines | 11 |
| G12 | **Splitting and validation**: k-fold by an argsort-of-shuffle trick, no stratification, backtests by `head(tail())`, grid search by triple-nested flatten | housing 302–314, churn 139, energy 246–275 | 12 |
| G13 | **Time-series toolkit**: Holt-Winters on lists, ACF by 192 full correlation passes, centred rolling via shift (off by one for even windows), decomposition by join plus re-sort, no resample | energy throughout | 13 |
| G14 | **Plotting**: colour by group as one filtered entry per group, fake 1..n x axes for months, Int casts for categorical axes, no reference lines, error bars, heatmap values or ACF stems | energy 138, segments 290, housing 160/168, sales 83 | 14 |
| G15 | **One global RNG**: reseeding to keep fits independent; no weighted choice, poisson or lognormal (copied 3 times) | experiment 77–86, segments 159, all generators | 15 |
| G16 | **Keyed lookup, argmax/argmin, descending sort, multi-key sort and join** | housing 404/454, churn 282, segments 207 | 16 |
| G17 | **I/O**: CSV without options (no date parsing or dtypes), `db.query` returns maps, generators build CSV text by hand with O(n²) appends | workspace 84–92, all generators | 17 |
| G18 | **Formatting**: `pct`, `money`, `fixed2` redefined in 6 notebooks; the brush's `()` replaced by sentinels | 9 definitions | 18 |

## Prerequisites in the language and runtime (L)

These are small changes, but the ergonomics depend on them. Each is additive.

| Id | Change | Why | Where |
|---|---|---|---|
| L1 | **Keyword arguments and defaults for natives.** A native registers its parameter names and default values. The call path maps `name: value` onto positions before dispatch. | `learn.split(df, test: 0.25, stratify: "y")`, `plot.xy(..., color: seg)`, and dropping the mandatory `#{}` | `src/native.rs` registry, the call path in `interpreter` and `ovm` |
| L2 | **Methods on native values.** `recv.m(args)` on a Series, Frame, Expr, Matrix or Model resolves to the receiver's home module, `ods.m(recv, args)`, when no `impl` matches. Each native names its module, and an explicit method table can add aliases. | `s.exp()`, `col("x").fill_null(0.0)`, `fit.predict(df)` | `lookup_method` fallback; one table per native |
| L3 | **Unary `-`/`!` and bitwise `& \| ^` reach the native operator hook.** `%` gets an arm for Series. | `-s`, `mask_a & mask_b`, `~`-style negation | `interpreter/ops.rs:411`, the `BitwiseOp` node, `binary_op_hook` |
| L4 | **`@` as a binary operator** (matrix product), resolved only through the native hook. Plain values raise "`@` needs a Matrix or Series". | `X @ b`; one grammar rule | `grammar.pest`, all three tiers |
| L5 | **Ergonomic coercions.** `len(series)` works. `ods.sum/mean(bool_mask)` count (no raise). Int Series and Int scalars join fused chains, widening to Float where the expression needs it. `ods.*` math and `stats.*` accept Int. | Removes about 80 of the 86 ceremony lines (G11) | `NativeObject::length`, `lib.rs:905`, `fuse` |
| L6 | **Field access on natives** (`f.price` for a column, `fit.params`) through a `NativeObject::field` hook. | `fit.params.sqft`, `split.train` | interpreter and OVM `GetField` fallback |

L2 makes every `ods` function a method, so this design never adds wrappers in olang. Verify first that `impl Trait for Series` dispatches (no test covers it today). L2 replaces it either way.

## The library

The signatures below use L1 keywords. `Series`/`Frame`/`Expr` arguments accept either where noted ("S|E": a Series, or an Expr inside a Frame verb).

### 1. Columns: dtypes, math, nulls, conditionals

**Dtypes** (G7, G8, G11). New to the existing `F64`, `I64`, `Bool` and `Str`:

| Dtype | Storage | Notes |
|---|---|---|
| `Date` | `i32` days since 1970-01-01 | built from `dates.Date` values, `to_date(fmt?)`, CSV `dates:` |
| `Datetime` | `i64` microseconds, naive or UTC | `to_datetime(fmt?)`; an optional `tz` is a later step |
| `Category` | `u32` codes plus a shared `StrCol` dictionary, with an optional order | the representation `.olc` already writes; `to_category(levels?)` |

`Bool` stays one byte per value (it vectorizes better than bits for the fused path).

**Constructors** (G1, P3):
- `ods.full(n, v)`, `ods.ones(n)`, `ods.zeros(n)` (exists)
- `ods.arange(n)` / `ods.arange(a, b, step)`
- `ods.full_like(s, v)`
- `ods.repeat(s, times)`, `ods.tile(s, n)` (energy 214)

**Elementwise math** (G6). Each is also a method:
- `abs exp exp2 expm1 ln ln1p log2 log10 sqrt cbrt sin cos tan …` (the `ods.map` list as functions)
- `round(s, digits = 0)` keeps the Float dtype; `floor`/`ceil`/`trunc` also keep it
- `sign`, `pow(s, p)`, `sigmoid`, `logit`, `clip(s, lo = (), hi = ())`
- `-s` (L3)

`ods.map(s, "name")` stays as written.

**Null and conditional** (G5):

| Function | Meaning |
|---|---|
| `ods.where(mask, a, b)` | elementwise choice; keeps the common dtype; a null mask element gives null |
| `ods.select(masks, values, default)` | first true wins (np.select) |
| `ods.coalesce(a, b, …)` | first non-null |
| `s.fill_null(v \| Series \| method)` | `method` is `"ffill"`, `"bfill"`, `"linear"`, `"mean"` or `"median"`, with an optional `limit:` |
| `s.interpolate(limit: ())` | linear between valid neighbours; correct over runs of gaps |
| `s.between(lo, hi, closed: "both")` | Bool mask; a `()` bound is open, so a `ui.brush` value of `()` means "everything" |
| `s.is_null()`, `s.not_null()`, `s.is_nan()` | masks |
| `s.recode(map, default: keep)` | value → value through a map (housing `hood_of`, energy `holiday→sunday`) |

**Reductions and positions** (G16):
- `argmax`, `argmin`
- `sum`, `mean`, `std`, `var`, `median`, `quantile`, `mode`
- `count`, `n_unique`, `first`, `last`, `any`, `all`
- `sort(s, descending: false)`, `argsort(s, descending: false)`
- `diff(s, lag = 1)`, `pct_change(s, lag = 1)`, `cumprod`
- `rank` (exists)
- `searchsorted(s, v)` (segments 128)

### 2. Text, dates and categories

Methods resolve by dtype (L2). Text methods on a `Str` column, date methods on `Date`/`Datetime`. A method that doesn't fit the dtype raises with the dtype and the method name. Every kernel is native over `StrCol` spans or the integer storage, with no per-element olang code.

**Text** (G7): `lower`, `upper`, `trim`, `trim_start`, `trim_end`, `replace(from, to)`, `replace_re(re, to)`, `contains(text, case: true)`, `starts_with`, `ends_with`, `len_chars`, `substring(a, b)`, `split(sep)` → list column, deferred, or `split_n(sep, i)`, `pad_start`, `pad_end`, `to_int()`, `to_float()` (null on failure), `to_date(fmt?)`, `to_datetime(fmt?)`.

**Dates** (G7, G13): `year`, `quarter`, `month`, `day`, `weekday`, `day_of_year`, `week`, `hour`, `minute`, `second`, `date()` (Datetime → Date), `day_name()`, `month_name()`, `floor("1d" | "1h" | "1mo" | …)`, `months_between(a, b)`, `add_days(n)`, `add_months(n)`, `is_weekend()`.
- Date arithmetic: `d - d` gives Int days; `d + n`.

**Categories** (G8):
- `to_category(levels: ())`, `levels()`, `codes()`
- `ods.dummies(s | frame, columns, drop_first: true, prefix: name, dtype: "Float")` → a Frame of indicator columns
- `ods.interact(a, b)`: a Category of `a:b` pairs, for energy's 71 interaction dummies

### 3. Expressions (`ods.col`)

A native `Expr` names columns without repeating the Frame variable, and is the unit of fusion.

```olang
use ods { col, lit }

houses
    |> ods.with({ ppsf: col("price") / col("sqft"), age: 2026 - col("year_built") })
    |> ods.filter(col("ppsf").between(lo, hi) & (col("sqft") <= 7000))
    |> ods.sort_by(["neighbourhood", "price"], descending: [false, true])
```

- **Where they apply.** Every column function and method of §1 and §2 also applies to an `Expr`, giving an `Expr`. Operators come through the native hook, so `col("a") * 2 + col("b")` is an Expr.
- **Evaluation.** An Expr is resolved against a Frame by `ods.with(f, { name: expr, … })`, `ods.filter(f, expr)`, `ods.select(f, [names | exprs])`, `ods.sort_by(f, exprs)` and `group_by(…).agg(…)`. The whole object of new columns compiles to one fused plan, evaluated in chunks and in parallel. A column several outputs read is read once.
- **`expr.over(keys)`** broadcasts a group aggregation back to every row, i.e. pandas' `transform`. It covers housing's group-wise fill and energy's weekday effect:
  ```olang
  ods.with(f, { year_built: col("year_built").fill_null(col("year_built").mean().over("neighbourhood").round()) })
  ```
- **Plain Series still work.** A bare Series anywhere an Expr goes is a literal column.
- **No new grammar.** `{ name: expr }` is the existing anonymous-object literal (ordered); `#{ "name": expr }` also works.

### 4. Frames: shape, group, join, time

**Shape and access** (G16):
- `ods.with(f, {…})`, `ods.rename(f, {…})`, `ods.slice(f, start, len)`
- `f["a"]`; `f[["a", "b"]]` (a List subscript selects columns); `f.a` (L6) as a read-only shorthand
- `ods.row(f, i)` → Map
- `ods.lookup(f, key_col, key, value_col)` → scalar or `()`. It builds a hash once and caches it per Frame identity (housing 404).
- `ods.sort_by(f, keys, descending: Bool | [Bool])` with several keys
- `ods.frame({ a: s1, b: [1, 2] })` from an object or map

**Group-by** (G9):

```olang
let by = sales |> ods.group_by(["region", "month"])     // a GroupBy native: group ids computed once
by.agg({ revenue: col("amount").sum(), orders: ods.count(), median_basket: col("amount").median() })
by.mean()                    // every numeric column (segments 224–228)
by.size()
by.transform(col("x").mean())   // == col("x").mean().over(keys)
by.apply((g) => fit_one(g))     // one olang call per group; results concat or collect
```

- **Aggregations:** count, sum, mean, min, max, first, last, median, quantile(q), std, var, n_unique, any, all, and argmax/argmin as row positions.
- **Compatibility:** the existing `group_by(f, keys, [[out, op, col]])` keeps working; it's the same engine.

**Crosstab, runs and pivot** (G9):
- `ods.crosstab(a, b, values: (), agg: "count", normalize: ())`
- `ods.runs(mask)`: a Frame of `start`, `end` and `length` for each true run (energy 348–354)
- `ods.run_id(mask)`: an Int Series to group by
- `pivot`/`unpivot` exist; `pivot` gains `fill:`

**Joins** (G16):
- `on:` takes a name or a list of names
- `how: "inner" | "left" | "full" | "semi" | "anti"` on `ods.join` (the five functions stay)
- `suffix:`
- `validate: "one_to_one" | "many_to_one"`, which raises on a key fan-out, the bug class behind energy's join-and-re-sort

**Time** (G13):
- `ods.resample(f, on, every, aggs)`, e.g. `"1d"`, `"1h"`, `"1mo"`. It is `group_by` on `col(on).floor(every)`, sorted by time (sales 75, housing 168, energy 105).
- `ods.rolling(s, w, agg, center: false, min_periods: w)`:
  - aggregations count, sum, mean, min, max, std, median;
  - `center:` is correct for even windows;
  - an O(n) running sum where exactness allows (Kahan-compensated), keeping the documented exact path as `exact: true`.
- `ods.asof_join` is deferred.

### 5. Matrices

A native `Matrix` (G1): column-major `F64`. It is a **view over column Arcs**, so `ods.matrix(frame, cols)` copies nothing when the columns are null-free `F64`.

| Call | Result |
|---|---|
| `ods.matrix(f, cols)`, `ods.matrix(list_of_series)` | Matrix (n × k); raises on nulls unless `nulls: "drop"` |
| `X @ b` (L4) | Series when `b` is a list or Series of length k; Matrix when `b` is a Matrix |
| `X.t()`, `X.gram()` (`XᵀX`, parallel), `X.col(j)`, `X.row(i)` | Matrix, Matrix, Series, List |
| `X.mean(axis: 0)`, `X.std(axis: 0)` | Series by column |
| `mat.solve(A, b)`, `mat.inv(A)`, `mat.cholesky(A)` | small k×k; reuses the in-crate Cholesky |
| `mat.eigh(A)` | symmetric eigenpairs (Jacobi; k ≤ a few hundred) |
| `ods.corr(f, cols?, method: "pearson" \| "spearman")`, `ods.cov` | Frame (labelled), one parallel pass (housing 209–213) |
| `mat.cdist(X, C, metric: "sqeuclidean")` | Matrix n × m, blocked (segments 96–113) |

`mat` is a namespace of small dense routines. SVD and QR wait for a demand that k×k Cholesky and Jacobi do not meet (`docs/ods.md` already defers faer).

### 6. `learn`: models, preprocessing, validation, metrics

A new native module, in scope like `ods`. Every model is a native `Model` with named terms.

**The model interface.** Each model has these fields (L6) and methods (L2):
- `fit.terms`: a list of names, the intercept first
- `fit.params`: Map term → estimate, so `fit.params.sqft` works
- `fit.coef`: a Frame of term, estimate, se, stat, p, lo, hi, plus odds_ratio for a logit
- `fit.predict(frame | matrix, kind: "response" | "link" | "class")`
- `fit.fitted`, `fit.resid`
- `fit.summary()`: a Frame
- `fit.n`, `fit.df_resid`, `fit.r2` (and adj_r2), `fit.aic`

**Design from a Frame.** Each model takes `y:` and `x:` as column names:
- A `Category` or `Str` column is one-hot encoded with the first level dropped.
- `Bool` becomes 0/1, and `Int` widens.
- Rows with a null in any used column are dropped, and that count is reported in `fit.dropped`.

`fit.predict(frame)` encodes new data with the training levels, so housing 406's hand-ordered vector disappears.

**Formula form.** `learn.linear(f, "price ~ sqft + bedrooms + C(hood) + C(kind):C(hour)")` is the same design, parsed natively; `C()` marks a term categorical. Both forms are first-class.

| Function | Purpose | Gap |
|---|---|---|
| `learn.linear(f, y:, x:, weights: ())` | OLS (exists as `stats.lm`, which stays) with names, fitted values, residuals, `conf_int(level)`, `predict(…, interval: "confidence" \| "prediction", level: 0.95)` | G2, H28/H36 |
| `learn.logistic(f, y:, x:, weights: () \| "balanced", penalty: 0.0)` | IRLS (Newton on the Gram matrix, parallel); SE, z and p; converges in about 8 iterations, with no learning rate | G3, churn |
| `learn.kmeans(X \| f, k, starts: 10, max_iter: 300, seed:)` | k-means++ init, Lloyd steps with blocked parallel assignment; labels (Int), centres (Frame), inertia | G3, segments |
| `learn.pca(X \| f, n, scale: true)` | eigh of the k×k covariance; components, explained variance, `transform(f)` | G3, segments |
| `learn.scale(f, cols, method: "standard" \| "minmax" \| "robust", log: [cols])` | a fitted Scaler, so `scaler.transform(test)` uses the training parameters | G3, churn 142–146 |
| `learn.split(f, test: 0.25, stratify: (), seed:)` | `{ train, test }` | G12 |
| `learn.kfold(n \| f, k, shuffle: true, stratify: (), seed:)` | Int fold-id Series | G12, housing 302 |
| `learn.time_splits(n, initial:, horizon:, step:)` | rolling-origin `[{ train, test }]` index ranges | G12, energy 264 |
| `learn.cross_validate(f, (train) => model, folds:, metrics: ["rmse", "r2"])` | a Frame per fold; one olang call per fold, not per row | G12, housing 306 |
| `learn.grid(params…)` | Cartesian product as a list of objects (`col.product` underneath) | energy 246 |
| `learn.permutation_importance(fit, f, y:, metric: "auc", repeats: 5, seed:)` | Frame, importance ± sd; the shuffles are native | G4, churn 357 |
| `learn.silhouette(X, labels, sample: (), seed:)` | Float, blocked distances | G3, segments 188 |

**Metrics.** Native; they take `y` and a score or prediction, Series or Expr:
- `learn.rmse`, `mae`, `mape`, `r2`
- `learn.log_loss(y, p, weights: ())`
- `learn.auc(y, p)`: Mann–Whitney with ties
- `learn.roc(y, p)` → Frame of fpr, tpr and threshold, with ties collapsed
- `learn.pr_curve(y, p)`
- `learn.confusion(y, p, threshold: 0.5)` → `{ tp, fp, fn, tn, precision, recall, f1, accuracy, flagged }`
- `learn.threshold_sweep(y, p, metric: "f1")` → Frame, from one sort and no per-threshold pass
- `learn.calibration(y, p, bins: 10)` → Frame

### 7. `stats`: inference and time series

- **Distributions are vectorized:**
  - `stats.norm.cdf(s | x, …)` accepts a Series.
  - Each `*.sample` takes `rng:`.
  - `stats.poisson`, `lognorm`, `binom`, `beta` and `gamma` join the existing four.
- **Tests:**
  - `stats.t_test` adds `ci` (the CI of the difference) and `d` (Cohen's d) to its Map.
  - New: `stats.power_t(effect, n, alpha, alternative)` and its inverse `stats.n_for_power(...)`, which make experiment 77–86's 240 simulated tests one call.
  - New: `stats.mann_whitney`, `stats.spearman`.
- **Robust statistics:** `stats.mad(s, scale: "normal")` (energy 327), `stats.iqr`, `stats.zscore(s, robust: false)`, `stats.tukey_fences(s, k: 1.5)` → `[lo, hi]` (housing 103).
- **Time series:** `stats.ts`.
  - `acf(s, lags, ci: 0.95)` → a Frame of lag, acf, lo and hi. It is FFT-based, O(n log n), replacing energy 196's 192 full passes.
  - `pacf`.
  - `decompose(s, period, model: "additive")` → a Frame of trend, seasonal and resid.
  - `holt_winters(s, season:, trend: "add", seasonal: "add", fit: true)`: the rates are optimized by bounded Nelder–Mead on the SSE. It returns a model with `forecast(h)`, `fitted`, `params` and `sse` (energy 217–248).
  - `seasonal_naive(s, season, h)`.

### 8. `random`: generators

G15:
- `random.rng(seed)` → a native `Rng` (PCG64), independent of the global stream, which stays.
  - Draws: `rng.normal(n, mu, sd)`, `uniform`, `integers`, `poisson`, `lognormal`, `binomial` → Series.
  - Sampling: `rng.choice(values, n, weights: (), replace: true)`, `rng.shuffle(s)`, `rng.permutation(n)`.
  - `rng.spawn(k)`: k independent child streams, deterministic under `par_map`.
- `ods.sample`, `learn.split`, `learn.kmeans` and the rest take `seed:` or `rng:`.
- The generators no longer need their copied `pick`, `poisson`, `lognormal` and `clamp`. They draw whole columns at once instead of appending rows one by one (O(n²)).

### 9. `plot`: grouping, dates, reference marks

G14. Every chart function gains L1 keywords. The old trailing `opts` map still works.

- **Frame-first forms:** `plot.scatter(f, x: "pc1", y: "pc2", color: "cluster")`, `plot.line(f, x: "month", y: "revenue")`, `plot.box(f, x: "neighbourhood", y: "price")`, `plot.hist(f, x: "v", color: "arm", bins: 40)`.
  - `color:` splits a column into series natively, so there are no records and no per-group filters.
  - A continuous `color:` uses the scale, through the per-point colour the chart description already carries.
- **Axes:**
  - A `Date`/`Datetime` x gets time ticks.
  - A `Category`/`Str`/`Int` x on bar and box charts is categorical, with no cast.
- **Marks on any xy chart:** `hline:`, `vline:`, `band:`, `error: (lo, hi)`, and `labels:` for points.
- **Chart kinds:** `plot.stem` (for the ACF), `plot.residuals(fit)`, `plot.coef(fit)` (estimates with intervals).
- **Heatmaps:** `plot.heatmap(frame | matrix, annotate: true, fmt: ".2f", center: 0.0)`. The centred scale already exists since 3d4ec98.
- **Studio:** draws all of this from the chart description, which gains `series.color`, `axis.kind` (`time`/`category`) and `marks`.

### 10. I/O and formatting

- **Reading CSV:** `ods.read_csv(text, sep: ",", dates: ["sale_date"], dtypes: { … }, nulls: ["", "NA"])`, and the same keywords on `read_csv_file`. `dates: "auto"` recognises ISO dates and datetimes.
- **Databases:** `db.query_frame(conn, sql, args)` → Frame, and `db.insert_frame(conn, table, f, create: true)` (workspace 84–92).
- **Writing:** `ods.write_csv(f, path, float_fmt: ())`. The generators build Series and write once.
- **Formatting:** `str.pct(x, digits = 1)`, `str.money(x, symbol = "$", digits = 0)`, `str.fmt(x, spec)` (`".2f"`, `",.0f"`, `".1%"`). Prose interpolation pipes through them: `${auc |> str.fmt(".3f")}`. This removes the copies of `pct`, `money` and `fixed2` in six notebooks.
- **Controls:** `ui.brush(chart)` returns `()` for "no range", and `s.between(range)` treats `()` as unbounded. The sentinels go.

## Before and after: the olmd notebooks

Line counts are code lines only. The Python columns are what an experienced pandas/sklearn user writes.

**Housing, cleaning** (8 cells today: closures over 10k rows, three casts, `choose`, join-to-fill):

```olang
let ppsf = col("price") / col("sqft")
let (lo, hi) = stats.tukey_fences(raw["price"] / raw["sqft"], k: 3.0)
let houses = raw
    |> ods.with({
        neighbourhood: col("neighbourhood").trim().lower().recode(hood_of).to_category(),
        sale_date: col("sale_date").replace("/", "-").to_date(),
    })
    |> ods.distinct()
    |> ods.drop_null(["sqft"])
    |> ods.filter(ppsf.between(lo, hi) & (col("sqft") <= 7000))
    |> ods.with({
        year_built: col("year_built").fill_null(col("year_built").mean().over("neighbourhood").round()),
        garage: col("garage").fill_null("median"),
        lot_sqft: col("lot_sqft").fill_null(col("sqft") * (col("lot_sqft") / col("sqft")).median()),
    })
```
That is 17 lines against 24 today; the pandas equivalent is about 14. Every step is native and fused.

**Housing, model** (`predict`, positional coefficients, the hand-ordered quote vector, the CV loop):

```olang
let fit = learn.linear(train, y: "price", x: ["sqft", "bedrooms", "bathrooms", "garage", "lot_sqft", "km_to_centre", "age", "neighbourhood"])
let rmse = learn.rmse(test["price"], fit.predict(test))
let cv = learn.cross_validate(data, (tr) => learn.linear(tr, y: "price", x: feats), folds: learn.kfold(data, 5, seed: seed), metrics: ["rmse", "r2"])
let quote = fit.predict(ods.frame({ sqft: [q_sqft], bedrooms: [q_bedrooms], neighbourhood: [q_hood], … }), interval: "prediction")
fit.coef        // term, estimate, se, t, p, lo, hi
fit.params.sqft
```
One-hot encoding, intervals, named terms and CV are no longer hand-written: 39 lines of cells come down to about 10, against about 12 for statsmodels/sklearn.

**Churn** (descent, sigmoid, log loss, AUC, confusion, ROC, calibration, importance):

```olang
let parts = learn.split(customers, test: 0.25, stratify: "churned", seed: seed)
let feats = ["tenure_months", "monthly_charges", "support_tickets", "usage_hours", "late_payments", "autopay", "contract", "segment"]
let plain = learn.logistic(parts.train, y: "churned", x: feats)
let balanced = learn.logistic(parts.train, y: "churned", x: feats, weights: "balanced")
let p = plain.predict(parts.test)
let calls = learn.confusion(parts.test["churned"], p, threshold: threshold)
plot.line(learn.roc(parts.test["churned"], p), x: "fpr", y: "tpr")
learn.permutation_importance(plain, parts.test, y: "churned", metric: "auc", seed: seed)
```
- The `iterations` and `learning_rate` params disappear.
- Coefficients come with standard errors and are already per-unit (no back-transform).
- Most of the 2.3 s (now 0.35 s) is gone: IRLS on a 15k × 12 Gram matrix is milliseconds.
- About 70 lines of helpers and fitting become about 10; sklearn needs about 12.

**Energy, features and models**:

```olang
let hourly = raw |> ods.with({ ts: col("timestamp").to_datetime(), demand_mw: col("demand_mw").interpolate() })
    |> ods.with({
        hour: col("ts").hour(), weekday: col("ts").weekday(), day: col("ts").date(),
        daytype: ods.select([col("holiday"), col("ts").weekday() == 7, col("ts").weekday() == 6], ["holiday", "sunday", "saturday"], "workday"),
        heating: (15.5 - col("temp_c")).clip(lo: 0.0),
    })
let daily = hourly |> ods.resample("ts", "1d", { energy: col("demand_mw").sum(), holiday: col("holiday").first(), temp: col("temp_c").mean() })
let hw = stats.ts.holt_winters(daily["energy"], season: 7)
let acf = stats.ts.acf(hourly["demand_mw"], 192)
let events = hourly |> ods.filter(col("robust_z").abs() > z_cut) |> ods.runs("index")
```
- A per-element closure over 26k records goes, and so do the calendar join and the substring parses.
- The 71 interaction dummies become `C(kind):C(hour)`.
- The 7-line run fold becomes `runs`.
- Holt-Winters and its triple-nested grid become one fitted call.

**Segments** (k-means++, Lloyd, silhouette, PCA):

```olang
let z = learn.scale(customers, FEATURES, log: ["orders", "avg_basket", "days_since_last"]).transform(customers)
let fits = map(KS, (k) => learn.kmeans(z, k, starts: 8, seed: seed))
let elbow = ods.frame({ k: KS, inertia: map(fits, (f) => f.inertia), silhouette: map(fits, (f) => learn.silhouette(z, f.labels, sample: 600, seed: seed)) })
let pcs = learn.pca(z, 2).transform(z)
plot.scatter(ods.with(pcs, { cluster: fit.labels }), x: "pc1", y: "pc2", color: "cluster")
ods.crosstab(fit.labels, customers["persona"])
```
About 110 lines of algorithms become about 6, against about 7 for sklearn.

| Notebook | Today | Target | Python reference (estimate) |
|---|---|---|---|
| housing | 221 | ≤ 120 | ~130 |
| churn | 207 | ≤ 90 | ~100 |
| energy | 213 | ≤ 110 | ~120 |
| segments | 193 | ≤ 70 | ~80 |

The Python references will be written out and counted when D5 rewrites the notebooks; see [Acceptance criteria](#acceptance-criteria).

## Performance plan

The engine already competes with NumPy, sklearn's OLS and Polars. The design must not cost that, and it should widen the lead where olang can.

1. **One expression engine.** Grow `fuse::Expr` into the evaluator behind `Expr` and every elementwise Series operation:
   - **Node types:** arithmetic (all of `+ - * / % pow neg`), comparisons, `& | !`, every math function, `where`/`select`/`clip`/`coalesce`/`fill_null`/`between`/`is_null`, `cast`, and the date-component and recode kernels.
   - **Nulls fuse.** Validity is combined per chunk, as a word-wise AND of bitmaps; today any null breaks the chain.
   - **Int fuses** and widens where the expression needs Float (L5).
   - **A reduction after an expression fuses into it**: `learn.rmse`, `sum(a*b)`, `mean(mask)` read each column once and allocate nothing.
   - **Shared subexpressions** in one `ods.with` object evaluate once.
   - **Parallelism** follows `ParClass` per plan. A plan's cost is the sum of its nodes, so a long plan crosses the threshold sooner than its single operations would.
2. **No closures in kernels.** `recode` takes a Map and builds a dictionary once. A `Category` maps by code. Per-group `apply` calls olang once per group, with the group handed over as a Frame view (no copy).
3. **Group ids are a reusable value.** A `GroupBy` and `expr.over(keys)` compute the id vector once (the existing parallel dictionary encoding) and every aggregation reuses it.
4. **The matrix is columns.** `ods.matrix` aliases column Arcs. `X @ b` is a fused axpy across columns; `gram` is the existing parallel `XᵀX`. Models are tall-skinny:
   - **IRLS** iterates on `XᵀWX` (k×k) with a Cholesky solve.
   - **k-means** assignment is a blocked n×k distance in L1-sized tiles, parallel over row blocks.
   - **PCA** is eigh of k×k.
   - **Silhouette** is blocked pairwise on a sample.
   - None needs BLAS at these shapes, which keeps the wasm build small.
5. **Text stays in spans.** String kernels write a new `StrCol` (one buffer plus spans), with no per-element `String`. `lower`/`upper` take an ASCII fast path, and `contains`/`replace` use memchr-style search.
6. **Dates are integers.** Components come from civil-from-days arithmetic over `i32`/`i64`: branch-light and autovectorized. Parsing ISO strings has a fixed-width fast path.
7. **Benchmarks are part of each milestone.** Each new kernel gets a row in `docs/performance.md` against NumPy, pandas or sklearn (and Polars where it has the op), with tier agreement and the measured parallel threshold. A milestone does not land with a kernel slower than its Python reference.
8. **Budgets:**
   - The browser runtime grows by at most the measured size of each module (target: `learn` + `mat` + dtypes ≤ 150 KB brotli in total). The roadmap's 400 KB browser target is unaffected, since it is about the parser and compiler.
   - `Expr` evaluation of a single op costs within 5% of today's eager kernel.

## Milestones

Each lands with tests (tier agreement, doc examples, the `doc_references` coverage test), a docs section in `ods.md`/`stdlib.md` and benchmark rows.

| Id | Content | Gaps closed |
|---|---|---|
| **D0** | L1–L6. Verify `impl` on natives. Methods for every existing `ods`/`stats`/`plot` function. | G6 (`-s`), G11 most, the precondition for everything |
| **D1** | Columns (§1), the expression engine (performance plan item 1), `ods.col`/`with`/`filter`/`select`/`sort_by` with Exprs. Fusing nulls, Int and reductions. | G5, G6, G10 (most), G11, G16 |
| **D2** | Dtypes Date/Datetime/Category, the text and date methods, `dummies`, `interact`, CSV `dates:`/`dtypes:`, `resample`, the `rolling` options | G7, G8, G13 (part), G17 (part) |
| **D3** | GroupBy object with more aggregations, `over`, `transform`, `apply`, `crosstab`, `runs`, multi-key joins with `validate`, `lookup`, `slice`, `row` | G9, G16 |
| **D4** | `Matrix`, `@`, `mat.*`, `ods.corr` Frame; `learn`: linear (named), logistic, scale, split, kfold, time_splits, cross_validate, metrics, permutation_importance, kmeans, silhouette, pca, formula parsing | G1, G2, G3, G4, G12 |
| **D5** | `stats` additions (vectorized distributions, power, mad/fences, `ts.*`), `random.rng`, `plot` keywords, Frame-first plots, colour by, marks and date axes (Studio's chart description too), `str.fmt`/`pct`/`money`, `db.query_frame`/`insert_frame` | G13, G14, G15, G17, G18 |
| **D6** | Rewrite the four DS notebooks and their generators. Write the Python reference notebooks beside them (`olmd/<name>/reference.py`, run with a pinned environment, never by the tests). Measure lines and time. Update README and the comparison. | acceptance |

D0 and D1 deliver most of the felt difference. D4 deletes the most code.

## Acceptance criteria

For each olmd notebook after D6:

1. **Lines:** code lines are at most those of its Python reference.
2. **No escape hatches** in analysis cells:
   - no `ods.to_list` → `map` → `ods.series` round-trip;
   - no `ods.cast` or `to_float` for arithmetic's sake;
   - no string-named math;
   - no positional coefficient lookup.

   A `check` rule in olmd's `olang.toml` (`--rules`) enforces this.
3. **Time:** a full `olang test` runs no slower than today, and no slower than the Python reference run end to end (both measured warm, interleaved, on this Mac).
4. **Truth:** the planted-truth `test` blocks still pass, with the same or tighter tolerances. Logistic regression and Holt-Winters now estimate rather than iterate a fixed number of times, so churn and energy should tighten.
5. **Studio renders every new value natively:** a Date axis, colour by group, model summaries as tables, Category columns in table headers.

## Decisions to make

1. **Weekday numbering.** Scalar `dates.weekday` is 0 = Sunday. Column `.weekday()` could follow it, or follow ISO (1 = Monday … 7 = Sunday, as Python's `isoweekday` and Polars do). The proposal is ISO for columns, with `dates.iso_weekday` added for scalars. The mismatch is documented in both places.
2. **Formula strings.** Should they be first-class, a convenience over `y:`/`x:`, or left out? The proposal is to include them: energy's interactions read best as a formula. They are parsed natively, with errors pointing into the string.
3. **Methods everywhere (L2).** This gives three spellings of one function. The alternative is pipes only, with no method sugar, which means fewer surfaces but less fluency for long chains (`col("x").trim().lower().recode(m)`). The proposal is to adopt L2: methods are resolved names, not a second API.
4. **`learn` as its own module vs inside `stats`.** The proposal is separate. `stats` is inference (tests, intervals, distributions); `learn` is predictive modelling and evaluation. That matches how people search ("sklearn" vs "statsmodels").
5. **Datetime time zones.** Naive and UTC only at first? The proposal is yes, with `tz` added on demand.
6. **Rolling sums: exact vs running.** Keep exact as the default, which is documented, or switch to compensated running sums with `exact: true` as opt-in. The proposal is to switch: O(n) matters for energy's 365-day windows, and Kahan-compensated error is far below the data's own precision.

## What to keep

These are not up for change. Every proposal above composes with them:
- **`|>` pipelines:** the subject comes first, so every new function pipes.
- **Explicit `Result`s** for I/O.
- **`test` blocks checking planted truths:** `learn`'s deterministic seeds make them sharper.
- **Notebook features:** params and `--for`, capabilities, `${…}` prose, one-line `ui` controls.
- **Native speed:** nothing here runs olang per row.

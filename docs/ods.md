# The Data Stack

olang builds its data tools into the language: `ods` (typed columns and
tables), `stats` (statistical inference), and `plot` (charts as SVG
text). No import, no package, no flag — the three modules are in scope
in every build, including the browser playground. This chapter teaches
the whole stack: what a columnar model buys, every Series and Frame
verb, a complete inference workflow, and how the stack composes with
the rest of the language.

Every plain `olang` block here is executed by the test suite on every
change; blocks marked `no-run` are parse-checked only (they touch the
file system).

Part of [the olang book](README.md) ·
[Tour](tour.md) · [Language](language.md) · [Stdlib](stdlib.md)

---

## Table of Contents

- [Why columns](#why-columns)
- [Series — the typed column](#series--the-typed-column)
- [Frames — tables of named columns](#frames--tables-of-named-columns)
- [`stats` — from description to inference](#stats--from-description-to-inference)
- [`plot` — charts as SVG text](#plot--charts-as-svg-text)
- [The stack and the language](#the-stack-and-the-language)
- [Performance characteristics](#performance-characteristics)
- [The design record](#the-design-record)
- [Why eager evaluation](#why-eager-evaluation)
- [What ods is not](#what-ods-is-not)

## Why columns

Consider computing revenue over a CSV of sales. The general-purpose
answer is a list of records — one map per row — and a fold:

```olang
let rows = [
    #{ "region": "east", "amount": 25.5, "qty": 10 },
    #{ "region": "west", "amount": 320.0, "qty": 3 },
]
let total = rows
    |> map((r) => map_get(r, "amount") * to_float(map_get(r, "qty")))
    |> fold(0.0, (acc, v) => acc + v)
println(to_string(total))
```

This works, and for a hundred rows it is the right tool. But look at
what the machine is being asked to do per row: hash two string keys,
box two values, call two closures. The *shape* of the data — every
`amount` is a float, every `qty` an integer — is rediscovered on every
access, because a list of maps has no place to record it.

A **columnar** model turns the table sideways. Instead of many rows
each holding a few values, it stores a few *columns* each holding many
values of one type, in one contiguous native buffer. Now "amount times
qty" is a single vectorized multiply over two arrays — no hashing, no
boxing, no per-element interpretation — and the memory access pattern
is exactly the sequential streaming that hardware is built to prefetch.

The difference is not subtle. When
[`examples/dataproc/`](../examples/dataproc/) was rewritten from the
records-and-fold pipeline above to the Frame pipeline this chapter
teaches, the same 200,000-row CSV job went from 3.0 seconds to 0.06
seconds — a 50× change from representation alone, in the same binary
(measured in [the design record](#the-design-record)). That measurement is
the design rationale for the entire stack: data work is dominated by
representation, so the representation belongs in the runtime, beneath
the language, where every olang program gets it for free — the same
relationship NumPy has to Python, except built in rather than bolted
on.

Two ideas carry everything that follows:

- **Operators are vectorized.** Arithmetic and comparison on a column
  run native kernels over the whole buffer; a comparison yields a mask
  for filtering.
- **Nulls are first-class.** Missing data — an empty CSV cell, an
  absent JSON key, a `()` in a source list — becomes a proper null
  that propagates through arithmetic and is skipped by reductions,
  never silently coerced to zero.

## Series — the typed column

A **Series** is a one-dimensional, typed, null-aware column of `Int`,
`Float`, `Bool`, or `String` values.

### Creating one

`ods.series` builds a Series from a list or a range, inferring the
element type: an all-`Int` list makes an Int series; any `Float` among
numerics widens the whole column to Float; `()` marks a null and
defers to the rest.

```olang
let a = ods.series([1, 2, 3])            // Int
let b = ods.series([1, 2.5, 3])          // Float — ints widen
let c = ods.series(1..6)                 // ranges work directly
let d = ods.series(["east", "west"])     // String
println(typeof(a) + " of length " + to_string(ods.len(c)))
```

Mixing strings with numbers, or booleans with numbers, is an error —
a column has one type; that is the point of a column. Two more
constructors cover numeric scaffolding: `ods.zeros(n)` is `n` float
zeros, and `ods.linspace(a, b, n)` is `n` evenly spaced floats from
`a` to `b` inclusive — the usual x-axis for a chart.

```olang
let x = ods.linspace(0.0, 1.0, 5)
println(to_string(ods.to_list(x)))       // [0, 0.25, 0.5, 0.75, 1]
```

### Arithmetic is elementwise

The ordinary operators `+ - * /` work on Series, elementwise, with a
scalar on either side broadcasting across the column. Each expression
is one native kernel over the buffer, not an interpreted loop:

```olang
let prices = ods.series([12.5, 8.0, 15.25, 4.0])
let taxed = prices * 1.07
let spread = prices - ods.mean(prices)
println(to_string(ods.mean(taxed)))
println(to_string(ods.sum(spread) < 0.0000001))   // centered: ~0
```

### Comparisons make masks

`< <= > >=` between a Series and a scalar (or another Series) produce
a Bool series — a **mask**. Masks are how selection works:
`ods.filter(s, mask)` keeps the elements where the mask is true.

```olang
let prices = ods.series([12.5, 8.0, 15.25, 4.0])
let expensive = ods.filter(prices, prices > 10.0)
println(to_string(ods.to_list(expensive)))        // [12.5, 15.25]
```

Equality needs one distinction. Between two Series, `==` stays
*structural*, like every other olang collection — same dtype, same
length, same values, same null pattern — because "are these the same
column?" is the question `==` answers everywhere else in the language.
The *elementwise* equality masks have their own names, `ods.eq(a, b)`
and `ods.ne(a, b)`; and `==`/`!=` against a *scalar* produce masks,
since there is no structural reading of "a column equals 2":

```olang
let s = ods.series([1, 2, 2, 3])
println(to_string(s == ods.series([1, 2, 2, 3])))       // true (structural)
println(to_string(ods.to_list(s == 2)))                 // mask vs a scalar
println(to_string(ods.to_list(ods.eq(s, ods.series([1, 0, 2, 0])))))
```

### Nulls

A null enters a Series wherever a source value was `()` — which is
exactly what a missed `map_get`, an empty CSV cell, or a missing JSON
key produces. From there, two rules govern its life:

- **Arithmetic propagates nulls.** `null * 2` is null; an operation
  cannot invent a value it does not have.
- **Reductions skip nulls.** The mean of `[1.0, null, 3.0]` is `2.0`,
  computed over the two values that exist.

```olang
let missing = map_get(#{}, "absent")              // Unit → null
let s = ods.series([1.0, missing, 3.0])
println(to_string(ods.null_count(s)))             // 1
println(to_string(ods.null_count(s * 2.0)))       // propagates: still 1
println(to_string(ods.mean(s)))                   // skips: 2
```

Three tools manage them. `ods.null_count(s)` counts, `ods.is_null(s)`
is a mask of the null positions, and `ods.fill_null(s, v)` returns a
copy with every null replaced:

```olang
let missing = map_get(#{}, "absent")
let s = ods.series([1.0, missing, 3.0])
let filled = ods.fill_null(s, 0.0)
println(to_string(ods.to_list(filled)))           // [1, 0, 3]
println(to_string(ods.to_list(ods.is_null(s))))   // [false, true, false]
```

This is the honest treatment of missing data: nothing is coerced, and
the one place a default value appears is the place you wrote one.

### Reductions

`ods.sum`, `ods.mean`, `ods.var`, `ods.std`, `ods.min`, `ods.max`, and
`ods.quantile(s, q)` reduce a column to a number, skipping nulls.
`var` and `std` are the *sample* statistics (the n−1 divisor), and
`quantile` interpolates linearly between order statistics, matching
NumPy's default. Two running forms complete the set: `ods.cumsum(s)`
is the running sum as a new Series, and `ods.dot(a, b)` is the inner
product.

```olang
let s = ods.series([4.0, 1.0, 7.0, 2.0])
println(to_string(ods.min(s)) + " .. " + to_string(ods.max(s)))
println(to_string(ods.quantile(s, 0.5)))          // median: 3
println(to_string(ods.to_list(ods.cumsum(s))))    // [4, 5, 12, 14]
println(to_string(ods.dot(s, s)))                 // 4²+1²+7²+2²
```

### Order and selection

`ods.sort(s)` returns an ascending copy, nulls last. `ods.argsort(s)`
returns the *indices* that would sort — an Int series — and
`ods.take(s, idx)` gathers by index. The pair composes into the
classic idiom: sort one column by another.

```olang
let names = ods.series(["carol", "alice", "bob"])
let scores = ods.series([72, 91, 84])
let order = ods.argsort(scores)
println(to_string(ods.to_list(ods.take(names, order))))
// scores ascending: [carol, bob, alice]
```

### Back to language values

`ods.to_list(s)` converts a Series back to an ordinary list (nulls
become `()`), `ods.get(s, i)` reads one element — a negative index
counts from the end — and `ods.len(s)` is the length:

```olang
let s = ods.series([10, 20, 30])
println(to_string(ods.get(s, 0)) + " " + to_string(ods.get(s, -1)))
println(to_string(ods.len(s)))
```

The conversion functions mark the boundary of the columnar world:
cross it at the edges of a computation, not inside a loop. A pipeline
that calls `to_list`, maps a lambda, and rebuilds a Series has paid
the per-element cost the stack exists to avoid — prefer the vectorized
operators wherever they express the computation.

## Frames — tables of named columns

A **Frame** is a table: named, equal-length Series. Its verbs all take
the frame first, so they chain with `|>` like everything else in the
language.

### Building one

Three constructors, one per source shape. `ods.frame` takes explicit
`[name, column]` pairs (each column a Series or a plain list):

```olang
let f = ods.frame([
    ["region", ["east", "west", "east"]],
    ["amount", [25.5, 320.0, 80.0]],
])
println(to_string(ods.n_rows(f)) + " x " + to_string(ods.n_cols(f)))
```

`ods.read_csv` parses CSV *text* — pair it with `fs.read_file` for a
file on disk — inferring each column's type: all-integer columns
become Int, numeric become Float, `true`/`false` become Bool, anything
else String. Empty cells become nulls, not empty strings:

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,,4\n")
println(to_string(ods.columns(sales)))
println(to_string(ods.null_count(ods.column(sales, "amount"))))   // 1
```

And `ods.frame_from_records` takes a list of maps — exactly what
`json.parse` yields for a JSON array of objects — with columns formed
from the sorted union of keys and missing keys becoming nulls:

```olang
let records = unwrap(json.parse("[{\"name\": \"ada\", \"score\": 99}, {\"name\": \"bob\"}]"))
let f = ods.frame_from_records(records)
println(to_string(ods.columns(f)))                                // [name, score]
println(to_string(ods.null_count(ods.column(f, "score"))))        // 1
```

The inverse, `ods.to_records(f)`, turns a Frame back into a list of
maps — the exit ramp for row-at-a-time output like printing a report
or serializing JSON.

### Looking at one

`ods.columns(f)` lists the column names, `ods.column(f, name)`
extracts one column as a Series, `ods.n_rows` and `ods.n_cols` give
the dimensions. Extracting a column and computing on it is the
fundamental Frame move — the table organizes the columns; the Series
operations do the work.

### Shaping columns and rows

`ods.select(f, names)` keeps named columns; `ods.with_column(f, name,
col)` returns a Frame with a column added or replaced — which,
combined with column arithmetic, is how derived columns happen:

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,80.0,4\n")
let full = ods.with_column(sales, "revenue",
    ods.column(sales, "amount") * ods.column(sales, "qty"))
println(to_string(ods.columns(full)))
```

On the row axis, `ods.filter(f, mask)` keeps rows where a Bool series
is true (build the mask from any column), `ods.take(f, idx)` gathers
rows by index, `ods.head(f, n)` keeps the first `n`, and
`ods.sort_by(f, col, descending)` sorts the whole table by one column,
nulls last either way:

```olang
let sales = ods.read_csv("region,amount\neast,25.5\nwest,320.0\neast,80.0\n")
let amount = ods.column(sales, "amount")
let big = sales |> ods.filter(amount > 50.0) |> ods.sort_by("amount", true)
println(to_string(ods.to_list(ods.column(big, "region"))))    // [west, east]
```

### Grouped aggregation

`ods.group_by(f, keys, aggs)` is the census verb: partition rows by
one or more key columns, then compute aggregates within each group.
The aggregation spec is a list of `[out_name, op, column]` triples,
with ops `count`, `sum`, `mean`, `min`, `max` — and `count` may omit
the column, since it counts rows. Groups keep first-seen order, and a
null key forms its own group (the R and Polars convention) rather
than disappearing:

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,80.0,4\n")
let summary = sales
    |> ods.group_by("region", [["total", "sum", "amount"], ["n", "count"]])
    |> ods.sort_by("total", true)
for rec in ods.to_records(summary) {
    println(map_get(rec, "region") + ": " + to_string(map_get(rec, "total"))
        + " over " + to_string(map_get(rec, "n")) + " sales")
}
```

### Joins

`ods.join(a, b, on_a, on_b)` is an inner hash join on one key column
from each side; `ods.join_left` keeps every left row, filling the
right side with nulls where nothing matched. Null keys never match
(the SQL convention), and a column-name collision on the right gains
a `_right` suffix:

```olang
let totals = ods.frame([["region", ["east", "west"]], ["total", [105.5, 320.0]]])
let rates = ods.frame([["name", ["east", "west"]], ["rate", [0.07, 0.09]]])
let joined = ods.join(totals, rates, "region", "name")
let tax = ods.column(joined, "total") * ods.column(joined, "rate")
println(to_string(ods.to_list(tax)))
```

## `stats` — from description to inference

Description says what the data at hand looks like; inference asks what
can honestly be concluded from it. The `stats` module covers both, and
every statistic it produces is pinned against scipy reference values
in the test suite. Results come back as ordinary maps, destructured
with `map_get` like any other map.

### Describing a column

`stats.describe(s)` is the first look: count, null count, mean,
standard deviation, minimum, quartiles, and maximum in one map.

```olang
let missing = map_get(#{}, "absent")
let s = ods.series([5.1, 4.9, 6.2, 5.7, missing, 5.5])
let d = stats.describe(s)
println("n=" + to_string(map_get(d, "count"))
    + " nulls=" + to_string(map_get(d, "null_count"))
    + " median=" + to_string(map_get(d, "median")))
```

`stats.corr(a, b)` and `stats.cov(a, b)` measure how two columns move
together — Pearson correlation and sample covariance, computed over
pairwise-complete observations (a row drops when either side is null).

### Hypothesis tests

`stats.t_test` has two forms, selected by its second argument. With
two Series it is Welch's two-sample test — "do these two groups have
the same mean?" — which does not assume equal variances and is the
form to reach for by default. With a number it is the one-sample test
against that null mean. Both return `t`, `df`, and `p_value`:

```olang
let a = ods.series([5.1, 4.9, 6.2, 5.7, 5.5, 4.8, 5.9, 6.1])
let b = ods.series([4.2, 4.8, 4.5, 5.0, 4.4, 4.1, 4.9])
let t = stats.t_test(a, b)
println("p = " + to_string(map_get(t, "p_value")))
println("means: " + to_string(map_get(t, "mean_a"))
    + " vs " + to_string(map_get(t, "mean_b")))
```

Read the p-value for what it is: the probability of a gap at least
this large *if the groups truly had the same mean*. It is not the
probability the hypothesis is true, and crossing an arbitrary
threshold is not a fact about the world — report the value, the group
means, and the sample sizes, and let the reader weigh them.

`stats.chi2_test(observed, expected)` is the goodness-of-fit test for
counts: did these category frequencies plausibly come from that
expected distribution?

```olang
let observed = ods.series([18.0, 22.0, 20.0])
let expected = ods.series([20.0, 20.0, 20.0])
let r = stats.chi2_test(observed, expected)
println(to_string(map_get(r, "p_value") > 0.05))   // consistent with uniform
```

### Regression — a complete workflow

`stats.lm(y, xs)` fits ordinary least squares with an intercept, where
`xs` is one predictor Series or a list of them. The example below runs
the workflow end to end on simulated data — simulation is the honest
way to *learn* an estimator, because you know the truth it is trying
to recover:

```olang
random.seed(7)
let n = 500
let x1 = stats.norm.sample(n, 0.0, 1.0)
let x2 = stats.norm.sample(n, 0.0, 1.0)
let noise = stats.norm.sample(n, 0.0, 1.0)
let y = x1 * 2.0 + x2 * -1.0 + noise + 3.0    // truth: b0=3, b1=2, b2=-1

let fit = stats.lm(y, [x1, x2])
let coef = map_get(fit, "coef")
println("intercept ≈ " + to_string(math.round(ods.get(coef, 0))))   // 3
println("b1 ≈ " + to_string(math.round(ods.get(coef, 1))))          // 2
println("b2 ≈ " + to_string(math.round(ods.get(coef, 2))))          // -1
println("r2 = " + to_string(map_get(fit, "r2") > 0.5))
```

The result map carries the full coefficient table as parallel Series —
`coef`, `se` (standard errors), `t`, and `p_value`, with the intercept
at index 0 and predictors following in the order given — plus `r2`,
`adj_r2`, `n`, and `df_resid`. Rows containing any null drop before
fitting.

Read the outputs honestly. A coefficient's `p_value` asks only whether
the data could plausibly have arisen with that coefficient at zero —
it says nothing about the effect being large enough to matter, which
is the coefficient's own job to show. And `r2` is the fraction of
variance in `y` the predictors explain *in this sample*; a high `r2`
is not evidence of causation, and a low one may still accompany a
precisely estimated, genuinely important effect. (In the simulation
above, `r2` is capped by the noise term — no fit could reach 1.0, and
that is a property of the data, not a failure of the model.)
`adj_r2` corrects for the temptation to inflate `r2` by adding
predictors.

### Distributions

Four families — `stats.norm` (parameters `mu, sigma`), `stats.t`
(`df`), `stats.chi2` (`df`), `stats.f` (`d1, d2`) — each with four
functions:

| Function | Question it answers |
|---|---|
| `pdf(x, ...)` | how dense is the distribution at `x`? |
| `cdf(x, ...)` | what fraction of it lies at or below `x`? |
| `ppf(q, ...)` | which value has fraction `q` below it? (inverse cdf) |
| `sample(n, ...)` | draw `n` values as a Series |

`cdf` and `ppf` are inverses, and `ppf` is where critical values come
from — the 1.96 of textbook fame is one call away:

```olang
println(to_string(stats.norm.ppf(0.975, 0.0, 1.0)))    // 1.9599...
println(to_string(stats.norm.cdf(1.96, 0.0, 1.0)))     // ≈ 0.975
```

Sampling draws from the `random` module's seeded stream, so
`random.seed(k)` makes a simulation exactly reproducible — the
property that keeps randomized code testable:

```olang
random.seed(42)
let first = stats.norm.sample(5, 0.0, 1.0)
random.seed(42)
println(to_string(first == stats.norm.sample(5, 0.0, 1.0)))   // true
```

## `plot` — charts as SVG text

A `plot` function returns a complete, standalone SVG document as a
String. That one decision is the module's philosophy: a chart that is
*text* composes with everything the language already has —
`fs.write_file` puts it on disk, an `http.serve` handler serves it,
the playground displays it — and needs no rendering dependency,
which is also why the whole stack runs in the browser build.

| Function | Draws |
|---|---|
| `plot.line(x, y, opts)` | one line through xy points |
| `plot.scatter(x, y, opts)` | one point cloud |
| `plot.lines(x, pairs, opts)` | several lines with a legend; `pairs` is `[[label, y], ...]` (up to 8) |
| `plot.bar(labels, values, opts)` | one bar per category; `labels` is a Series or list |
| `plot.hist(s, bins, opts)` | binned counts of one numeric Series |

Options ride in a single map — `title`, `x_label`, `y_label`, `width`,
`height` — and passing `#{}` accepts the defaults, which follow a
colorblind-validated palette assigned in fixed series order. An
unknown option key is an error, because it is always a typo. Null
handling matches what a chart can honestly draw: xy plots drop a point
when either coordinate is null; `bar` refuses null values outright,
since a bar of unknown height is a lie — `fill_null` or `filter`
first, so the decision is visible in the code.

```olang
let x = ods.linspace(0.0, 6.28, 60)
let sin = ods.series(map(ods.to_list(x), (v) => math.sin(v)))
let cos = ods.series(map(ods.to_list(x), (v) => math.cos(v)))
let svg = plot.lines(x, [["sin", sin], ["cos", cos]],
    #{ "title": "two waves", "x_label": "t" })
println(to_string(str.contains(svg, "<svg")))

random.seed(3)
let draws = stats.norm.sample(2000, 0.0, 1.0)
let hist = plot.hist(draws, 30, #{ "title": "standard normal" })
println(to_string(str.length(hist) > 500))
```

```olang no-run
// The usual ending: a chart on disk, viewable in any browser.
unwrap(fs.write_file("waves.svg", svg))
```

## The stack and the language

Nothing in ods is a sublanguage. Frames chain with `|>` because every
verb takes its data first; results come back as maps and lists the
rest of the program consumes; and the parallelism constructs fan
statistical work across cores exactly as they fan any other work.

That last point deserves a demonstration, because Monte Carlo methods
are the natural meeting place of the two: each resample is independent
CPU-bound work — precisely what [`par_map`](stdlib.md#higher-order-functions)
distributes. Here is a bootstrap confidence interval for a mean, with
the resampling fanned across cores in chunks (each worker seeding its
own stream so the chunks explore different resamples):

```olang
random.seed(11)
let data = ods.to_list(stats.norm.sample(300, 50.0, 8.0))

let boot = par_map(range(0, 4), (chunk) => {
    random.seed(100 + chunk)
    let mut means = []
    let mut rep = 0
    while rep < 50 {
        means = means + [ods.mean(ods.series(random.choices(data, len(data))))]
        rep = rep + 1
    }
    means
})
let means = ods.series(flatten(boot))
println("95% CI: [" + to_string(math.round(ods.quantile(means, 0.025)))
    + ", " + to_string(math.round(ods.quantile(means, 0.975))) + "]")
```

Two complete worked programs extend these patterns to full scale:

- [`examples/dataproc/`](../examples/dataproc/) — the CSV → Frame →
  `group_by` → JSON pipeline, the 50× story of the opening section as
  a running program.
- [`examples/statlab/`](../examples/statlab/) — a full statistical
  study: 10,000 simulated subjects, a Welch t-test cross-validated by
  a 1,000-round permutation test and a bootstrap CI (both fanned over
  `par_map`), a three-predictor regression, and charts of the results,
  with a `test` block pinning every inference.

## Performance characteristics

The stack's performance claims are measured, not asserted — the full
methodology, benchmark code, and every recorded revision live in
[the design record below](#the-design-record). The shape of the results,
on 10M-element columns against NumPy and 10M-row tables against Polars:

| Operation | ods | Reference | Standing |
|---|---|---|---|
| `sum` / `mean` (10M floats) | 1.01 ms (0.41 ms parallel) | NumPy 1.12 ms | parity sequential, ~2.7× ahead parallel |
| `std` | 2.06 ms | NumPy 5.11 ms | ~2.5× ahead |
| `a * b + 1.0` elementwise | 2.42 ms parallel | NumPy 3.24 ms | ahead as executed; sequential trails (fusion deliberately deferred) |
| `sort` | 24.1 ms parallel | NumPy 324.3 ms | far ahead |
| null-aware `mean` (10% nulls) | 4.71 ms | NumPy `nanmean` 7.84 ms | ~1.7× ahead |
| OLS fit, 1M rows × 20 predictors | 36.4 ms parallel | NumPy `lstsq` 137.8 ms | ~3.8× ahead |
| group-by, 10M rows, 1k groups | 27.2 ms (one thread) | Polars 24.0 ms (18 threads) | within 1.13× |

Reductions on large columns parallelize automatically under the
language's standard policy — the same machinery as `par_map`, tuned by
`set_parallel(n)` — with no change to any result. The practical
guidance is simpler than the table: keep computations in Series and
Frame operations, cross to lists at the edges, and column size stops
being something to think about.

## The design record

This section is the stack's design register — the problem it was built
to solve, the decisions that shaped it, and the measurements that gated
each phase. It is kept in the book because the discipline it records is
part of the product: **benchmarks are the spec**, and a claim that was
never measured is a claim this chapter does not make.

### The problem

Before ods, olang was suitable for light data work and structurally
unsuitable for real statistics. The gaps, in order of severity:

1. **No numeric arrays.** A "list of floats" was `Arc<[Value]>` — every
   element a 16-byte tagged enum. Ten million floats meant ten million
   boxed values with no cache locality and no SIMD; no interpreter,
   bytecode, or JIT work can fix a representation problem.
2. **No linear algebra** — no solve/least-squares, so no regression or
   PCA without O(n³) loops over lists.
3. **No statistical machinery** — samples without distribution functions
   (pdf/cdf/ppf) mean no tests, intervals, or p-values.
4. **No missing-data story.** Real datasets have holes; a stats stack
   that discovers nulls late retrofits them into every kernel.

ods closes these gaps in Rust, under the language, so every olang-level
statistics library is fast *by construction* — the relationship NumPy
has to Python, built in rather than bolted on.

### The core decision: one array, both tiers

Everything hangs on the representation. The engine defines one value
kind — a contiguous, typed, 64-byte-aligned buffer with an Arrow-style
validity bitmap for nulls (absent when there are none) and a
vector-or-matrix shape:

```rust
pub struct OdsArray {
    dtype: DType,                 // F64 | I64 | Bool | Str
    buf: Buffer,                  // contiguous, 64-byte aligned, Arc-shared
    validity: Option<Bitmap>,     // Arrow-style null bitmap; None = no nulls
    shape: Shape,                 // Vector(len) | Matrix(rows, cols)
}
```

Three rules follow from it:

- **Nulls are first-class from day one.** The bitmap costs nothing when
  absent and cannot be retrofitted later without touching every kernel;
  every kernel is null-aware from its first version.
- **Copy-on-write mutation.** olang values are immutable and Arc-shared;
  kernels use `Arc::make_mut` — when the refcount is 1, mutate in place.
  Functional semantics, in-place performance: `xs |> fill_null(0.0) |>
  cumsum()` allocates once, not three times.
- **The tier boundary is an `Arc` clone.** The interpreter and the OVM
  hold the *same* array. There is no conversion, so there is nothing to
  convert lossily — the failure mode that once kept maps and enums off
  the bytecode tier is unrepresentable here.

The engine lives in its own workspace crate (`olang-ods`) with no
dependency on olang — pure buffers and kernels — and everything is pure
Rust, which is why the full stack runs in the
[browser playground](wasm.md).

### Where the speed comes from

Assembled levers, in order of payoff — no clever code where a crate or
the compiler already wins:

| Lever | Buys | How |
|---|---|---|
| Contiguous typed buffers | 10–100× over boxed lists | The representation above; this is most of the win |
| LLVM autovectorization | SIMD for free | Kernels are tight branch-free loops over `&[f64]` |
| Explicit SIMD (`pulp`) | AVX/NEON where autovec fails | Only for kernels that measurably need it |
| Rayon | Near-linear scaling on large arrays | Kernels split above the language's standard parallel threshold |
| In-crate solvers + `statrs` | Regression, distributions | Normal equations with a small Cholesky for OLS; pdf/cdf/ppf machinery never hand-rolled |
| Copy-on-write | Fused-allocation pipelines | `Arc::make_mut` in every kernel's output path |

One invariant keeps the layers coherent: **the engine contains no
statistics, and the stats layer contains no loops.** `stats.t_test` is a
few lines composing kernels; finding an element-wise `for` in the stats
layer means a primitive is missing from the engine — it moves down.

### Benchmarks are the spec

Each phase of the stack shipped only when its numbers met a target
recorded *in advance* — and a miss meant the target was met later or
revised here with the reason, never shipped with an asterisk. Apple
Silicon macOS, criterion benches, versus pinned NumPy 2.5 / Polars 1.43,
single-threaded best-of-15 unless noted. 10M elements/rows:

| # | Benchmark | ods seq | ods par | Reference | Verdict |
|---|---|---|---|---|---|
| B1 | `sum` | 1.01 ms | 0.41 ms | NumPy 1.12 ms | **met** — seq parity, par 2.7× ahead |
| B1 | `std` | 2.06 ms | — | NumPy 5.11 ms | **met** — 2.5× ahead |
| B2 | `a * b + 1.0` | 6.19 ms | 2.42 ms | NumPy 3.24 ms | **met as executed** (par is the 10M default) |
| B3 | `sort` | 121.5 ms | 24.1 ms | NumPy 324.3 ms | **met** — 2.7× / 13× ahead |
| B4 | OLS, 1M × 20 | 90 ms | 36.4 ms | NumPy `lstsq` 137.8 ms | **met** — up to 3.8× ahead |
| B5 | null-aware `mean`, 10% nulls | 4.71 ms | — | NumPy `nanmean` 7.84 ms | **met** — 1.7× ahead (target revised, note below) |
| B6 | group-by, 1k groups | 27.2 ms (1 thread) | — | Polars 24.0 ms (18 threads) | **met** — within 1.13× |

Revisions and notes, recorded per the rule:

- **B5's original target** ("within 1.5× of dense `mean`") implicitly
  assumed null *clusters*; the benchmark's scattered pattern leaves no
  dense 64-element blocks, and closing further would need
  multiply-by-mask kernels that are unsound when masked slots hold
  inf/NaN. Revised to the user-facing bar — beat `nanmean` — and met.
- **B2's sequential gap** is allocation plus memory bandwidth from
  materializing the intermediate — the identical two-pass cost NumPy
  pays. Fusion is the structural fix and is deliberately deferred
  ([below](#why-eager-evaluation)).
- **Sequential `sum` needed care:** strict FP ordering forbids LLVM from
  vectorizing a naive fold, so the dense kernels use eight accumulators
  (summation order differs from left-to-right by design, like NumPy's
  pairwise sum).
- **Why single-threaded group-by hangs with 18-thread Polars:** the hot
  loop is one Fx-hashed id per row plus vec-indexed accumulators; at 1k
  groups the accumulators live in L1 and the pass is memory-bound on the
  key column. Partitioned parallel grouping remains available headroom.
- **Inference is pinned, not eyeballed.** Every distribution value, test
  statistic, p-value, and regression coefficient is asserted against
  scipy/NumPy reference constants recorded in the test suites, with the
  generating snippets noted inline. `stats.*.sample` draws from the
  `random` module's stream, so `random.seed(k)` makes sampling
  reproducible — pinned by test.
- **The 50× headline** at the top of this chapter is B-series
  discipline applied end-to-end: `examples/dataproc` rewritten from
  records-and-fold to the Frame pipeline, same binary, 3.0 s → 0.06 s.

Deferrals recorded with reopening conditions: **faer-backed linear
algebra** (the in-crate Cholesky is textbook-correct for
regression-sized systems; the big decompositions — SVD, PCA, QR —
arrive with `faer` when a real demand creates them) and **lazy
evaluation**, next.

## Why eager evaluation

Every ods operation evaluates eagerly: what a pipeline does is what it
says, in order. This was not a default taken by inertia — lazy
evaluation and expression fusion were evaluated on the numbers when the
stack's last phase landed, and declined. The reasoning is recorded here
so the next visit starts from evidence instead of enthusiasm.

**The problem fusion solves.** Eager elementwise kernels materialize
every intermediate: in `xs * 2.0 + noise`, the multiply writes a
10M-element buffer the add immediately reads back and discards — three
memory streams where a fused kernel needs two. Measured on B2, a fused
single pass would land around 3–4 ms sequential against the eager
engine's 6.19 ms.

**Why that didn't buy lazy.** Two facts frame it: the eager engine
already beats the competition (parallel ods runs this exact benchmark
1.3× ahead of NumPy — which pays the identical two-pass cost, because
NumPy doesn't fuse either), and the win is bounded — one memory stream
saved per intermediate, roughly a third of the traffic for a two-op
chain. Nobody's workload is 10×'d.

The three places fusion could live, and the verdicts:

- **Lazy frames at the surface** (Polars' answer — `lazy() |> ... |>
  collect()` builds an optimized query plan): the highest ceiling, since
  predicate pushdown and column pruning beat kernel fusion by orders of
  magnitude on real pipelines. Rejected regardless: it is a second
  evaluation model for users to learn ("when is my frame *real*?"), an
  optimizer to build and test, and a debugging story that fights
  olang's errors-happen-here simplicity.
- **Peephole fusion in the bytecode tier** — recognizing
  `Mul t, a, b; Add dst, t, c` with `t` dead and calling a fused kernel;
  zero surface change. The *right* eventual home, deliberately waiting
  for the JIT's instruction-level machinery to be still; the liveness
  and pattern analysis should be built once.
- **Named fused kernels** (`ods.fma(a, b, c)`) — the boring option:
  an afternoon each, testable against the composed form by construction.
  Pre-approved whenever the gate below trips.

**The gate.** This decision reopens when a real workload — an example
program, a user report, or a benchmark modeling one — spends more than
~20% of its runtime in sequential elementwise Series chains of length
≥ 2. First response: ship the specific fused kernel and measure. Only
sustained demand across many patterns justifies the peephole, and only
optimizer-class wins would ever justify lazy frames.

Until then: the eager engine is simple, measured, and ahead. Boring is
a feature.

## What ods is not

The stack's boundaries are chosen, not pending. Knowing them tells
you when to reach for a different tool:

- **Not N-dimensional.** Series are 1-D and Frames are 2-D, which
  covers statistics. General N-D broadcasting is the largest
  complexity tax in array systems and serves a different domain (deep
  learning) — out of scope entirely, as is GPU execution.
- **Not a dtype zoo.** Int, Float, Bool, String. Dates, categoricals,
  and narrower floats wait for a concrete demand, not a checklist.
- **Not lazy.** Every operation evaluates eagerly, so what a pipeline
  does is what it says, in order. A lazy query optimizer is a second
  evaluation model the language declines to carry on today's evidence;
  the reasoning and the measured gate for revisiting are recorded in
  [Why eager evaluation](#why-eager-evaluation) above.
- **Not a plotting toolkit.** `plot` draws the statistical staples
  well, with strong defaults, as text. Interactive charts, animation,
  and grammar-of-graphics layering belong to dedicated tools that can
  consume the data ods exports.

Within those boundaries, the ambition is deliberate: for columns,
tables, inference, and charts, the built-in stack should be the
obvious tool — fast by construction, honest about nulls, and shaped
like the language it lives in.

---

Next: [the stdlib reference](stdlib.md) for the surrounding modules, or
[olang in the Browser](wasm.md) for the same stack running as
WebAssembly.

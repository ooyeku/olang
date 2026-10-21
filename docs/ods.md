# The data stack

Part of [the olang book](README.md) · [A tour of olang](tour.md) ·
[Standard library reference](stdlib.md) · [Writing fast olang](performance.md)

This chapter describes olang's data stack: typed columns and tables,
expressions over them, matrices, statistical inference, time series,
machine learning, and charts. It walks through each part with examples;
[the standard library reference](stdlib.md#ods--series-and-frames)
lists every function and option, and [Writing fast
olang](performance.md#bulk-data-belongs-in-ods) holds the measurements.

The modules are:

| Module | What it holds | In scope |
|---|---|---|
| `ods` | Series, Frames, expressions (`ods.col`), GroupBy, file formats, `ods.matrix` | always |
| `mat` | dense linear algebra over a `Matrix` | always |
| `stats` | descriptive statistics, tests, power, distributions, `stats.ts` | always |
| `learn` | models, formulas, splits and metrics | always |
| `random` | the seeded stream and `random.rng` generators | always |
| `plot` | charts as SVG text | always |
| `ml` | `learn` with keyword forms and olang-function hooks | `use ml` |
| `gg` | a grammar of graphics | `use gg` |
| `viz` | chart specs as values, for SVG or a canvas | `use viz` |

All of them run in every build, the browser build included. Every
plain `olang` block in this chapter runs as written; blocks marked
`no-run` touch the file system.

## Table of contents

- [The columnar model](#the-columnar-model)
- [Series — the typed column](#series--the-typed-column)
- [Frames — tables of named columns](#frames--tables-of-named-columns)
- [Expressions — `ods.col`](#expressions--odscol)
- [Matrices — `ods.matrix` and `mat`](#matrices--odsmatrix-and-mat)
- [`stats` — from description to inference](#stats--from-description-to-inference)
- [`learn` — machine learning](#learn--machine-learning)
- [`plot` — charts as SVG text](#plot--charts-as-svg-text)
- [`gg` — a grammar of graphics](#gg--a-grammar-of-graphics)
- [The stack and the language](#the-stack-and-the-language)

## The columnar model

A Series stores one column of one type in a contiguous native buffer,
with a validity bitmap for its nulls. A Frame is a table of named,
equal-length Series. Operations run native kernels over whole columns,
so a computation written as column operations does no per-row
interpretation; across large columns they also run in parallel.
[`examples/data-processing/dataproc/`](../examples/data-processing/dataproc/)
holds the same job written as a fold over a list of maps (`records.ol`)
and as Frame operations (`main.ol`); on 200,000 generated rows the
second is about eighty times faster.

Two rules carry everything that follows:

- **Operators are vectorized.** Arithmetic and comparison on a column
  run native kernels over the whole buffer; a comparison yields a mask
  for filtering.
- **Nulls are values of their own.** Missing data — an empty CSV cell,
  an absent JSON key, a `()` in a source list — becomes a null that
  propagates through arithmetic and is skipped by reductions. Nothing
  is coerced to zero.


## Series — the typed column

A **Series** is a one-dimensional, typed, null-aware column of `Int`,
`Float`, `Bool`, `String`, `Date`, `Datetime` or `Category` values
(the last three are [below](#dates-and-datetimes)).

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
println(`${typeof(a)} of length ${ods.len(c)}`)
```

Mixing strings with numbers, or booleans with numbers, is an error —
a column has one type; that is the point of a column. Two more
constructors cover numeric scaffolding: `ods.zeros(n)` is `n` float
zeros, and `ods.linspace(a, b, n)` is `n` evenly spaced floats from
`a` to `b` inclusive — the usual x-axis for a chart.

```olang
let x = ods.linspace(0.0, 1.0, 5)
println(to_string(ods.to_list(x)))       // [0.0, 0.25, 0.5, 0.75, 1.0]
```

The rest of the scaffolding is NumPy's. `ods.full(n, v)` is `n` copies
of any value (a text value is stored once), `ods.ones(n)` is `n` float
ones, and `ods.full_like(s, v)` is a column of `v` as long as `s`.
`ods.arange(stop)`, `ods.arange(start, stop)` and `ods.arange(start,
stop, step)` count up to, not including, `stop`: an Int column from Int
arguments, a Float column (element `i` is `start + i * step`, so the end
carries no drift) otherwise. `ods.repeat(s, k)` repeats each element `k`
times in a row (or as many times as a list of counts says, element by
element), and `ods.tile(s, k)` repeats the whole column end to end:

```olang
println(to_string(ods.to_list(ods.arange(0, 10, 3))))                 // [0, 3, 6, 9]
println(to_string(ods.series(["a", "b"]).repeat(2).to_list()))          // ["a", "a", "b", "b"]
println(to_string(ods.series([1, 2]).tile(2).to_list()))               // [1, 2, 1, 2]
println(to_string(ods.to_list(ods.full(2, "x"))))                       // ["x", "x"]
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

**Division is true division.** `/` between two Int Series, or between
an Int Series and an Int, yields a Float Series — unlike the scalar
`/`, where `7 / 2` is `3`. A ratio of measure columns (`deaths /
population`) is the usual meaning in data work, and a truncated one
would be a wrong answer with no error. `ods.cast(a / b, "Int")` gives
the integer quotient when that is what is meant. Addition, subtraction, and multiplication of Int Series
stay Int, and checked. Division by zero raises, as everywhere.

```olang
let counts = ods.series([1, 2, 3])
println(to_string(ods.to_list(counts / 2)))                      // [0.5, 1.0, 1.5]
println(to_string(ods.to_list(counts / ods.series([2, 4, 4]))))  // [0.5, 0.5, 0.75]
println(to_string(ods.to_list(ods.cast(counts / 2, "Int"))))     // [0, 1, 1]
```

**`+` joins text.** On a String Series, `+` concatenates elementwise —
with another String Series, or with a String on either side, which every
row takes. A null cell stays null, as it does in arithmetic. It is the
way to build a composite key or a label column:

```olang
let f = ods.read_csv("city,state\nAustin,TX\nReno,NV\n")
let key = f["city"] + ", " + f["state"]
println(to_string(ods.to_list(key)))           // ["Austin, TX", "Reno, NV"]
println(to_string(ods.to_list("<" + f["state"])))   // ["<TX", "<NV"]
```

Anything else beside a String Series (`s + 1`) is an error naming the
cast: `ods.cast(s, "String")` turns a number column into text first. A
Category joins as its text, and in an expression `+` joins the same way
(`"n=" + col("n").cast("String")`, `col("city") + "!"`).

`%` is the remainder with the sign of the dividend, as the scalar `%`
is: `ods.series([7, -7]) % 3` is `[1, -1]`, and a zero divisor raises
"Modulo by zero".

**Operators and column functions fuse.** `a * b + c`, `ods.exp(x) *
2.0`, `ods.where(m, a, b) - 1.0`: nothing is computed while the
expression is built, and the first time the result is observed — a
reduction, an index, a display, any verb — the whole expression runs
in one pass, a couple of thousand rows at a time, so the intermediate
columns live in the processor's cache instead of main memory. Nulls,
Int columns and every elementwise function take part. An operation
that can raise — a division, a remainder, checked Int arithmetic, a
function with a restricted domain like `ln` — is evaluated where it is
written (fused with everything before it), so its error is raised by
the line that causes it. A reduction of an expression (`ods.sum(a *
b)`, `ods.mean(mask)`) reads the columns once and makes no column at
all. None of this is visible except by the clock: the values are the
ones the operations give one at a time.

Series arithmetic runs IEEE kernels: a Float Series can hold `inf` or
`NaN` after an overflowing multiply, where scalar arithmetic raises
([Floats](language.md#floats)). `ods.to_list` hands
such values over as they are; `ods.is_null` does not count them.

### Elementwise math

Every function of `math.*` that maps a number to a number has a column
form: `ods.exp(s)`, `ods.ln(s)`, `ods.sqrt(s)` and the rest are one
native pass over the column, computing what `math.exp` and friends
compute (they call the same `f64` routines), bit for bit, with nulls
propagating. The full set is `abs exp exp2 expm1 ln ln1p log2 log10
sqrt cbrt sin cos tan asin acos atan sinh cosh tanh sigmoid logit sign
floor ceil trunc`. Three more take a second argument: `ods.pow(s, p)`
(`p` a number or a column), `ods.round(s, digits)` (half away from
zero; `digits` defaults to 0 and may be negative, rounding to tens or
hundreds), and `ods.clip`, below.

```olang
let t = ods.linspace(0.0, 6.28, 8)
let wave = t.sin() * 2.0 + 1.0                       // one fused pass
println(to_string(wave.mean()))
let p = ods.sigmoid(ods.series([-2.0, 0.0, 2.0]))    // the module form is the same call
println(to_string(p.round(digits: 3).to_list()))     // [0.119, 0.5, 0.881]
```

Two details matter in data work. A function whose answer is a whole
number keeps an Int column Int (`abs`, `sign`, `floor`, `ceil`, `trunc`,
`round`); the others widen it to Float, so an Int column needs no cast
before `ods.ln`. And a function with a restricted domain (`sqrt`, `ln`,
`log2`, `log10`, `ln1p`, `asin`, `acos`, `logit`) raises the scalar
function's error at the first valid value outside it, exactly as a loop
over `math.*` would; a null never raises. `sigmoid` is `1 / (1 + e^-x)`
computed without overflow for large `|x|`, and `logit` is its inverse.

`ods.map(s, name)` applies a math function by name: `ods.map(s,
"exp")` is `ods.exp(s)` with an Int column widened to Float, and it
also knows `fract`, `degrees` and `radians`.

### Comparisons make masks

`< <= > >=` between a Series and a scalar (or another Series) produce
a Bool series — a **mask**. Masks are how selection works:
`ods.filter(s, mask)` keeps the elements where the mask is true.

```olang
let prices = ods.series([12.5, 8.0, 15.25, 4.0])
let expensive = ods.filter(prices, prices > 10.0)
println(to_string(ods.to_list(expensive)))        // [12.5, 15.25]
```

Equality needs one distinction. Between two Series, `==` is
*structural*, as it is for every other olang collection: same dtype,
same length, same values, same null pattern. The *elementwise*
equality masks are `ods.eq(a, b)` and `ods.ne(a, b)`. `==` and `!=`
against a *scalar* produce masks:

```olang
let s = ods.series([1, 2, 2, 3])
println(to_string(s == ods.series([1, 2, 2, 3])))       // true (structural)
println(to_string(ods.to_list(s == 2)))                 // mask vs a scalar
println(to_string(ods.to_list(ods.eq(s, ods.series([1, 0, 2, 0])))))
```

### Choosing values: `where`, `select`, `clip`, `between`

`ods.where(mask, a, b)` takes `a` where the mask is true and `b` where
it is false, each a column or a value; a null in the mask gives a null.
The result has the type both sides meet at (Int and Float meet at
Float). `ods.select(masks, values, default)` is the many-way form: the
first mask that holds picks its value, a null mask counting as not
holding, and the default (`()` if omitted) fills the rest:

```olang
let temp = ods.series([10.0, 18.0, 25.0, ()])
let kind = ods.select([temp < 15.5, temp > 22.0], ["heating", "cooling"], default: "neither")
println(to_string(ods.to_list(kind)))     // ["heating", "neither", "cooling", "neither"]
println(to_string(ods.to_list(ods.where(temp < 15.5, 15.5 - temp, 0.0))))   // [5.5, 0.0, 0.0, ()]
```

`ods.clip(s, lo, hi)` limits a column to a range; either bound may be
`()` or left out, which leaves that side open (`s.clip(lo: 0.0)` is
`max(s, 0)`, `s.clip(hi: 1.0)` caps it). `ods.between(s, lo, hi)` is the
mask of values in the range, both ends included; `closed`, one of
`"both"` (the default), `"left"`, `"right"` or `"neither"`, says which
ends are. A `()` bound is open there too, and so
the range may come as one value — a `[lo, hi]` pair, or `()` for no
range at all, which is what a `ui.brush` answers before anything is
dragged:

```olang
let s = ods.series([-3.0, 0.5, 8.0])
println(to_string(s.clip(0.0, 5.0).to_list()))                  // [0.0, 0.5, 5.0]
println(to_string(s.clip(hi: 1.0).to_list()))                   // [-3.0, 0.5, 1.0]
println(to_string(s.between(0.0, ()).to_list()))                // [false, true, true]
println(to_string(s.between([0.0, 8.0], closed: "left").to_list()))   // [false, true, false]
println(to_string(ods.to_list(ods.between(s, ()))))             // [true, true, true]
```

### Nulls

A null enters a Series wherever a source value was `()` — which is
exactly what reading an absent key (`m.absent`), an empty CSV cell, or a
missing JSON key produces. From there, two rules govern its life:

- **Arithmetic propagates nulls.** `null * 2` is null; an operation
  cannot invent a value it does not have.
- **Reductions skip nulls.** The mean of `[1.0, null, 3.0]` is `2.0`,
  computed over the two values that exist.

```olang
let s = ods.series([1.0, (), 3.0])                // () becomes a null
println(to_string(ods.null_count(s)))             // 1
println(to_string(ods.null_count(s * 2.0)))       // propagates: still 1
println(to_string(ods.mean(s)))                   // skips the null: 2.0
```

Three tools manage them. `ods.null_count(s)` counts, `ods.is_null(s)`
is a mask of the null positions, and `ods.fill_null(s, v)` returns a
copy with every null replaced:

```olang
let s = ods.series([1.0, (), 3.0])
let filled = ods.fill_null(s, 0.0)
println(to_string(ods.to_list(filled)))           // [1.0, 0.0, 3.0]
println(to_string(ods.to_list(ods.is_null(s))))   // [false, true, false]
```

`ods.not_null(s)` is the mask the other way round, and `ods.is_nan(s)`
marks the NaNs of a Float column (a NaN is a value, not a null: it is
what `0.0 / 0.0` gives in a column, and reductions do not skip it).

The fill can be a column as well as a value — each null takes the
other column's value at that row — and `ods.coalesce(a, b, …)` takes
the first non-null of any number of columns and values. A value of the
other numeric type widens the column (an Int column filled with `2.5`
becomes Float). Or the fill can be a method, named by a String:

| Method | Each null becomes |
|---|---|
| `"ffill"` | the last valid value before it |
| `"bfill"` | the next valid value after it |
| `"linear"` | the straight line between the valid values either side, by position (`ods.interpolate(s)`) |
| `"mean"`, `"median"` | the column's mean or median |

`"ffill"`, `"bfill"` and `"linear"` take a third argument, `limit`, the
most nulls in a row one value fills. Leading nulls have nothing before them
to carry forward and stay null, as trailing ones do for `"bfill"` and
`"linear"`; the interpolation is right over a run of several gaps, not
just one:

```olang
let gaps = ods.series([1.0, (), (), 4.0, ()])
println(to_string(gaps.fill_null("ffill").to_list()))            // [1.0, 1.0, 1.0, 4.0, 4.0]
println(to_string(gaps.fill_null("ffill", limit: 1).to_list()))  // [1.0, 1.0, (), 4.0, 4.0]
println(to_string(gaps.interpolate().to_list()))                 // [1.0, 2.0, 3.0, 4.0, ()]
println(to_string(ods.to_list(ods.coalesce(gaps, ods.series([9.0, 8.0, 7.0, 6.0, 5.0])))))
```

(A method name is always a method: to fill a text column with the word
`"mean"`, write `ods.coalesce(s, "mean")`.)

### Reductions

`ods.sum`, `ods.mean`, `ods.var`, `ods.std`, `ods.min`, `ods.max`,
`ods.median`, and `ods.quantile(s, q)` reduce a column to a number,
skipping nulls. `var` and `std` are the *sample* statistics (the n−1
divisor), and `quantile` interpolates linearly between order
statistics, matching NumPy's default. `median` is `quantile(s, 0.5)`,
and so is the `median` row of `ods.describe`. Two more complete the set:
`ods.cumsum(s)` is the running sum as a new Series, and `ods.dot(a, b)`
is the inner product.

```olang
let s = ods.series([4.0, 1.0, 7.0, 2.0])
println(`${ods.min(s)} .. ${ods.max(s)}`)
println(to_string(ods.median(s)))                 // 3.0
println(to_string(ods.to_list(ods.cumsum(s))))    // [4.0, 5.0, 12.0, 14.0]
println(to_string(ods.dot(s, s)))                 // 4²+1²+7²+2²
```

A Bool mask sums to its count of true, so `ods.mean(mask)` is the share
of rows where it holds — `ods.mean(s > 2.0)` is a rate, with no cast.
`ods.count(s)` counts the values that are not null, `ods.first(s)` and
`ods.last(s)` are the ends (null included), `ods.mode(s)` is the most
frequent value (the smallest on a tie), and `ods.any(mask)` and
`ods.all(mask)` ask whether some or every valid element is true.
`ods.argmax(s)` and `ods.argmin(s)` are positions: the first row of the
largest or smallest value, skipping nulls and NaN, or `()` when there is
none.

```olang
let s = ods.series([4.0, 1.0, 7.0, (), 7.0])
println(`${s.argmax()} ${s.argmin()} ${s.count()}`)                   // 2 1 4
println(`${(s > 2.0).mean()} ${(s > 6.0).any()} ${s.mode()}`)        // 0.75 true 7.0
println(`${(s > 2.0).sum()} ${ods.mean(s > 2.0)}`)                   // 3 0.75
```

A reduction of an expression does not build the expression's column:
`ods.sum(a * b)`, `ods.mean((y - p) * (y - p))` and `ods.dot(a, b)`
read their inputs once, chunk by chunk.

### Windows

Four verbs look at a column's neighbourhood rather than the whole of it.

`ods.shift(s, by)` moves values down the column, filling what is vacated
with nulls; a negative `by` moves them up. `ods.diff(s, lag)` is `s - ods.shift(s, lag)` (`lag`
defaults to 1; an Int column stays Int), `ods.pct_change(s, lag)` is the
relative change `s / ods.shift(s, lag) - 1`, and `ods.cumprod(s)` is the
running product, the shape `ods.cumsum` has:

```olang
let s = ods.series([4, 1, 7, 1, 9])
println(to_string(ods.to_list(ods.shift(s, 1))))       // [(), 4, 1, 7, 1]
println(to_string(s.diff().to_list()))                 // [(), -3, 6, -6, 8]
println(to_string(s.diff(lag: 2).to_list()))           // [(), (), 3, 0, 2]
println(to_string(ods.series([1, 2, 3]).cumprod().to_list()))   // [1, 2, 6]
```

`ods.cum_max(s)` and `ods.cum_min(s)` are the running maximum and
minimum.

`ods.rank(s, method)` ranks smallest first, and a null ranks as null.
The method decides what happens to values that tie, and defaults to
`"min"`, competition ranking:

| method | ties among four values |
|---|---|
| `"min"` (default) | 1, 2, 2, 4 — competition ranking |
| `"max"` | 1, 3, 3, 4 — ties take the last position they span |
| `"average"` | 1, 2.5, 2.5, 4 — ties share the mean of their positions |
| `"ordinal"` | 1, 2, 3, 4 — broken by row order, every rank distinct |
| `"dense"` | 1, 2, 2, 3 — the next distinct value gets the next integer |

Only `"average"` can produce a half, so every other method returns an
Int column that can be used as indices without a cast.

`ods.rolling(s, window, agg)` is a window aggregate: element `i` reduces
the `window` elements ending at `i`, with `agg` one of `count`, `sum`,
`mean`, `min`, `max`, `median`, `std` or `var`. The first `window - 1`
elements are null, because the window is not yet full. Nulls inside a window are skipped, exactly as the whole-column
reductions skip them, so a window holding two valid values of three
gives the mean of two:

```olang
let s = ods.series([1.0, 2.0, 3.0, 4.0, 5.0])
println(to_string(ods.to_list(ods.rolling(s, 3, "mean"))))    // [(), (), 2.0, 3.0, 4.0]
println(to_string(ods.to_list(ods.rolling(s, 3, "max"))))     // [(), (), 3.0, 4.0, 5.0]
println(to_string(s.rolling(2, "sum", center: true).to_list()))   // [(), 3.0, 5.0, 7.0, 9.0]
println(to_string(s.rolling(3, "mean", min_periods: 1).to_list()))  // [1.0, 1.5, 2.0, 3.0, 4.0]
```

Three options, all named:
- `center: true` puts the window around its row rather than behind it,
  where pandas puts it: rows `i - window/2` to `i - window/2 + window - 1`,
  so an even window leans back by half a row. (A trailing window moved
  up with `shift` leans the other way, and is a row off for an even
  window.)
- `min_periods: k` reports a window — a partial one at either edge
  included — once it holds `k` valid values. Without it a window must be
  whole and hold one valid value.
- `exact: true` computes each window afresh.

By default a window slides: each step adds the row entering and
subtracts the row leaving, so a window of any width costs the same —
O(n) for the column — where a fresh reduction per row costs O(n ×
window). Sums, means, variances and standard deviations keep running
totals with Neumaier compensation (a NaN or an infinity counted apart,
so it leaves the window with its row), the extremes a monotone queue,
and the median a sorted window. A running sum can differ from a fresh
one in the last few bits of the window's sum — far below any measured
quantity's own precision — and `exact: true` is the fresh sum, equal to
`ods.sum` of the same rows, at O(n × window). An Int column's sum is
exact either way (its running total is a 128-bit integer).

### Distinct values

`ods.unique(s)` gives the distinct values in first-seen order,
`ods.n_unique(s)` counts them without building the column, and
`ods.value_counts(s)` returns a two-column Frame — `value` and
`count` — with the most frequent first, ties breaking by first
appearance so the result is deterministic:

```olang
let s = ods.series(["west", "east", "east", "north", "east"])
println(to_string(ods.to_list(ods.unique(s))))    // ["west", "east", "north"]
println(to_string(ods.n_unique(s)))               // 3
println(to_string(ods.to_list(ods.value_counts(s)["count"])))    // [3, 1, 1]
```

All three identify values the way `ods.group_by` identifies keys, which
differs from `==` on scalars in two ways: **a null is a value**,
forming its own entry rather than being skipped, and **all NaNs are one
value**.

### Changing a column's type

`ods.cast(s, type)` converts a column, where `type` is one of
`"Float"`, `"Int"`, `"Bool"`, `"String"`, `"Date"`, `"Datetime"` or
`"Category"` — the names `ods.schema` reports. Nulls stay null. Text
casts to a Date or a Datetime as ISO 8601, a Date's Int is its days
since 1970-01-01 and a Datetime's its microseconds.

A value the target type cannot hold becomes **null**, not an error and
not a wrong number, so one unparseable row does not fail a load, and
`ods.null_count` counts exactly what was lost:

```olang
let text = ods.series(["1", " 2 ", "oops", "4"])
let ints = ods.cast(text, "Int")
println(to_string(ods.to_list(ints)))       // [1, 2, (), 4]
println(to_string(ods.null_count(ints)))    // 1
```

Float to Int truncates toward zero, and a float too large for an Int,
or infinite, or NaN, becomes null rather than the saturated value a
raw hardware conversion would produce. Strings parse with surrounding
whitespace ignored; `"true"` and `"false"` parse to Bool in any case.
Casting to String spells a float exactly as `to_string` does.

Float to Bool raises and points at writing the comparison instead
(`s != 0.0`).

### Taking a sample

`ods.sample(s, n)` draws `n` rows at random, and works the same way on
a Frame — `ods.sample(f, n)` samples whole rows, keeping the columns
aligned.

Sampling is **without replacement**, so `n` draws are `n` distinct
rows; asking for more rows than exist returns all of them, as `head`
and `tail` do. Rows come back in the frame's **original order**, not
draw order. Without a seed, rows are drawn from the stream
`random.seed(k)` governs:

```olang
let f = ods.read_csv("id,v\n1,a\n2,b\n3,c\n4,d\n5,e\n")
random.seed(20260817)
let first = ods.to_list(ods.sample(f, 3)["id"])
random.seed(20260817)
println(to_string(first == ods.to_list(ods.sample(f, 3)["id"])))    // true
```

That stream is shared by the whole process, so the global seed does
**not** make sampling reproducible inside `par_map` or `par for`:
several threads drawing from one stream interleave in whatever order
they reach it. For that, give the sample its own seed or generator:
`ods.sample(f, n, seed: 7)`, or `ods.sample(f, n, rng: g)` with a
[`random.rng`](stdlib.md#generators-randomrng) — and inside `par_map`
one child of `g.spawn(k)` per task.

```olang
let f = ods.frame({ id: ods.arange(0, 100) })
println(ods.sample(f, 5, seed: 3)["id"].to_list() == ods.sample(f, 5, seed: 3)["id"].to_list())   // true
let kids = random.rng(9).spawn(3)
let picks = par_map(kids, (g) => f.sample(2, rng: g)["id"].to_list())
println(len(picks))   // 3
```

### Order and selection

`ods.sort(s)` returns an ascending copy, nulls last. `ods.argsort(s)`
returns the *indices* that would sort — an Int series — and
`ods.take(s, idx)` gathers by index (an Int Series or a list of Ints,
`ods.take(s, [2, 0])`). Together they sort one column by another:

```olang
let names = ods.series(["carol", "alice", "bob"])
let scores = ods.series([72, 91, 84])
println(names.take(scores.argsort()).to_list())   // ["carol", "bob", "alice"]
```

Both take a second argument, `descending`: `s.sort(descending: true)`
puts the largest first, and `s.argsort(descending: true)` gives the
order that does,
ties keeping their row order. Nulls go last either way.
`ods.searchsorted(s, v)` is where `v` would go in a sorted column to keep
it sorted — the first position whose value is not less than `v`, or
with `side: "right"`, greater than `v`; `v` may be a list
or a Series, giving a position for each:

```olang
let edges = ods.series([0.0, 10.0, 20.0, 50.0])
println(to_string(edges.searchsorted(15.0)))                    // 2
println(to_string(edges.searchsorted(20.0, side: "right")))     // 3
println(to_string(ods.searchsorted(edges, [5.0, 60.0])))        // [1, 4]
println(to_string(ods.series([3, 9, 1]).sort(descending: true).to_list()))   // [9, 3, 1]
```

### Recoding values

`ods.recode(s, mapping)` replaces each value through a mapping: a map or
an object (whose keys are text, read as numbers for a number column) or
a list of `[from, to]` pairs. A value the mapping does not name keeps
itself; a third argument replaces those instead, with `()` making them
null. Each distinct value is looked up once, so a mapping over a
million rows of a dozen spellings costs a dozen lookups:

```olang
let raw = ods.series(["N", "s", "S", "x"])
let names = { N: "North", s: "South", S: "South" }
println(to_string(ods.to_list(ods.recode(raw, names))))            // ["North", "South", "South", "x"]
println(to_string(raw.recode(names, default: "Other").to_list()))  // ["North", "South", "South", "Other"]
println(to_string(ods.to_list(ods.recode(ods.series([1, 2]), [[1, 10]]))))   // [10, 2]
```

### Text

A String column's methods are its text kernels. Each runs over the
column's buffer and writes a new one — no `String` a row — and a null
stays null:

```olang
let raw = ods.series(["  Old Town ", "riverside", "OLD TOWN", ()])
println(to_string(raw.trim().lower().to_list()))   // ["old town", "riverside", "old town", ()]
let s = ods.series(["L103773", "L108142"])
println(to_string(s.substring(1).to_int().to_list()))        // [103773, 108142]
println(to_string(ods.series(["a,b,c"]).split_n(",", -1).to_list()))   // ["c"]
println(to_string(ods.series(["7"]).pad_start(3, "0").to_list()))     // ["007"]
```

`lower`, `upper`, `trim` (and `trim_start`, `trim_end`), `replace(from,
to)`, `replace_re(pattern, to)` (the `re` module's patterns; `$1` names a
group), `contains(text, case: true)`, `starts_with`, `ends_with`,
`len_chars`, `substring(start, end)`, `split_n(sep, i)` (piece `i`, a
negative `i` counting from the end), `pad_start(width, fill)` and
`pad_end` are the `str` functions of the same names over a column, and
agree with them cell for cell. `format(spec)` of a number column is the
column as text by a number spec (`str.fmt`'s, Python's mini-language:
`".2f"`, `",.0f"`, `".1%"`, `"$,.0f"`, `"02d"` zero-padded, `"#x"`),
one pass into one buffer — the table and label
form of a measure:

```olang
let share = ods.series([0.1234, 0.5, ()])
println(to_string(share.format(".1%").to_list()))   // ["12.3%", "50.0%", ()]
println(to_string(ods.series([1234567.0]).format("$,.0f").to_list()))   // ["$1,234,567"]
```

The same spec reads in prose and on a scalar: `str.fmt(x, ".2f")`,
`str.pct(x)`, `str.money(x)`. `to_int()` and `to_float()` read numbers,
and a cell that is not one is null, as `cast` makes it. What only narrows
a cell — a trim, a substring, a piece — writes no text at all: its cells
point into the column's own buffer.

### Dates and datetimes

A **Date** column holds days since 1970-01-01, a **Datetime** column
microseconds since 1970-01-01T00:00:00 (naive, or UTC: a time read with
an offset is converted). They come from text, from `dates` values, or
from a CSV:

```olang
let days = ods.series(["2024-03-01", "2024/03/02", ()]).replace("/", "-").to_date()
println(days)                                       // Series[Date; 3] [2024-03-01, 2024-03-02, null]
let raw = ods.series(["21/05/2024 13:45", "01/06/2024 08:00"])
let ts = raw.to_datetime("%d/%m/%Y %H:%M")
println(to_string(ts.hour().to_list()))             // [13, 8]
println(ods.series([unwrap(dates.date(2026, 8, 7))]))   // Series[Date; 1] [2026-08-07]
```

Without a format, `to_date()` and `to_datetime()` read ISO 8601 —
`2024-03-01`, and `2024-03-01T13:45`, with seconds, a fraction and `Z` or
an offset if it has them — through a fixed-width path that is a few
nanoseconds a cell. With one they read `strptime`'s directives (`%Y %y %m
%d %e %j %H %k %I %l %p %P %M %S %f %z %Z %s %b %B %h %a %A %u %w %F %T
%D %R %c %x %X`; a month's or a day's name in full or its first three
letters). A cell that does not read is null. `%C %G %U %V %W` only write.

A date's parts are integer arithmetic on that storage, one pass over the
column:

| Method | Answer |
|---|---|
| `year()` `quarter()` `month()` `day()` | Int |
| `weekday()` | Int, ISO: 1 Monday … 7 Sunday |
| `day_of_year()` `week()` | Int (the ISO week, 1–53) |
| `hour()` `minute()` `second()` | Int (a Datetime's) |
| `is_weekend()` | Bool |
| `date()` | a Datetime's day, as a Date |
| `day_name()` `month_name()` | a Category whose levels are the calendar's, so it sorts Monday first |
| `floor(every)` | the start of its hour, day, week (Monday), month, quarter or year: `"1h"`, `"15m"`, `"1d"`, `"1w"`, `"1mo"`, `"1q"`, `"1y"` |
| `add_days(n)` `add_months(n)` | the same type; a month added to January 31st is February's last day; `n` an Int, or an Int column or expression (`col("d").add_months(col("term"))`) |
| `months_between(a, b)` | Int, the calendar months from `b` to `a` (the days ignored) |
| `format(fmt)` | text, in `strftime`'s directives as Python's C locale writes them: `%b` `Jan` and `%B` `January`, `%a` `Sun` and `%A` `Sunday`, `%d` `07` and `%e` ` 7`, `%U` `%W` weeks (Sunday or Monday first), `%G-W%V-%u` the ISO week, `%c` `Sun Jan  7 15:04:05 2024` |

A column's `weekday()` is ISO, 1 for Monday to 7 for Sunday, as Python's
`isoweekday` and Polars count. `dates.weekday` of a single Date counts
0 for Sunday; `dates.iso_weekday` is the scalar that agrees with the
column.

Arithmetic and comparisons do what they mean: `d + n` and `d - n` move a
Date by days, `d - d` is an Int of days between, `t - t` is a Float of
seconds between two Datetimes, and a comparison or a subtraction takes a
Date, a Datetime or ISO text (`t - t.min()`, whose minimum is its text):

```olang
use ods { col }
let sales = ods.read_csv("day,amount\n2024-01-30,10.0\n2024-02-02,25.0\n2024-02-29,40.0\n", dates: ["day"])
let febr = sales.filter(col("day") >= "2024-02-01")
println((febr.day - febr.day.shift(1)).to_list())   // [(), 27]
println(sales.day.add_months(1).to_list())          // [2024-02-29, 2024-03-02, 2024-03-29]
let monthly = sales
    .with({ month: col("day").floor("1mo") })
    .group_by("month")
    .agg({ total: col("amount").sum() })
println(monthly)
```

The parts, `floor`, `add_months`, `months_between` and the arithmetic are
expression-engine operations: in `ods.with` they fuse with the operators
and comparisons around them, so `col("ts").weekday() == 7` is one pass.
A Date or Datetime column sorts, groups, joins, takes `unique` and
`value_counts`, and its `min` and `max` are dates; an element read back
(`d[0]`, `to_list`) is a `Date` value, and a Datetime's is its ISO text
(the `dates` module's moments are text).

### Categories

A **Category** is a column of small codes into a list of levels — a
handful of neighbourhoods over a million rows stored as a code a row and
six strings. Its levels are its order: sorted, unless given.

```olang
let size = ods.series(["m", "s", "l", "m", ()]).to_category(levels: ["s", "m", "l"])
println(to_string(size.codes().to_list()))   // [1, 0, 2, 1, ()]
println(to_string(size.sort().to_list()))    // ["s", "m", "m", "l", ()]
println(to_string((size >= "m").to_list()))  // [true, false, true, true, ()]
println(ods.dummies(size, prefix: "size"))   // size_m, size_l: 0.0 or 1.0 a row
```

`to_category(levels)` takes the levels in their order; a value that is
none of them is null. Without them a column's distinct values are the
levels: text sorted, numbers by value, dates by date. `levels()` and
`codes()` read them back. A Category sorts and compares by its levels'
order, and is equal by text, so `size == "m"` and `ods.is_in(size,
["m", "l"])` read as they would on text — the same in an expression
(`ods.filter(f, col("size") >= "m")`, `ods.between(col("size"), "s",
"m")`) and in `ods.eq` and `ods.ne`. Ordering against text that is not
a level is an error naming the levels; `==` against it is false. Text methods run once per level.
It groups by its codes with no hashing, joins by its text, and stays a
Category through `filter`, `take`, `sort_by`, `concat` (whose levels it
merges) and the native file format.

`ods.dummies(s | frame, columns, drop_first: true, prefix, dtype:
"Float")` gives the indicator columns: one per level, named
`prefix_level`, the first left out by default (the regression
convention; `drop_first: false` keeps it), a null row 0 in all of them.
Given a Frame it replaces each String and Category column (or each one
named) by its indicators, where it stands. `ods.interact(a, b)` is the
Category of each row's pair, `a:b`, its levels the pairs that occur — an
interaction's dummies in one call:

```olang
let calendar = ods.frame([["daytype", ["workday", "sunday", "workday"]], ["hour", [8, 8, 18]]])
let slot = ods.interact(calendar["daytype"], calendar["hour"])
println(to_string(slot.levels().to_list()))   // ["sunday:8", "workday:8", "workday:18"]
println(ods.columns(ods.dummies(slot, prefix: "slot")))
```

`learn`'s models encode a Category column by its levels: the first level
is the dropped baseline, and the coefficients come out in level order.

### Back to language values

`ods.to_list(s)` converts a Series back to an ordinary list (nulls
become `()`), `ods.get(s, i)` reads one element — a negative index is
out of range, as a list's is — and `ods.len(s)` is the length:

```olang
let s = ods.series([10, 20, 30])
println(`${ods.get(s, 0)} ${ods.get(s, ods.len(s) - 1)}`)
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

`ods.frame` takes explicit `[name, column]` pairs (each column a Series
or a plain list):

```olang
let f = ods.frame([
    ["region", ["east", "west", "east"]],
    ["amount", [25.5, 320.0, 80.0]],
])
println(`${ods.n_rows(f)} x ${ods.n_cols(f)}`)
```

An object (or a map) does the same with less punctuation, its entries
the columns in order:

```olang
let f = ods.frame({ region: ["east", "west", "east"], amount: ods.series([25.5, 320.0, 80.0]) })
println(to_string(ods.columns(f)))    // ["region", "amount"]
```

`ods.read_csv` parses CSV *text*, inferring each column's type:
all-integer columns become Int, numeric become Float, `true`/`false`
become Bool, anything else String. Empty cells become nulls, not empty
strings:

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,,4\n")
println(to_string(ods.columns(sales)))
println(to_string(ods.null_count(sales["amount"])))   // 1
```

Four options change how it reads. `sep` is the separator (one
character), `nulls` the cells that read as null (`[""]` unless given),
`dtypes` an object naming a column's type — `"String"` keeps zero-padded
codes as text, which inference would read as numbers — and `dates` names
the columns that hold ISO dates, or `"auto"` takes every text column
whose cells all are. A column `dates` names that holds a cell that is no
ISO date is an error naming the cell, rather than a column quietly half
null:

```olang
let text = "zip;opened;flag
007;2024-03-01;NA
042;2024-03-02;y
"
let shops = ods.read_csv(text, sep: ";", nulls: ["NA"], dtypes: { zip: "String", flag: "Category" }, dates: "auto")
println(ods.schema(shops))
```

`read_jsonl` takes `dates` and `dtypes` too, and `ods.write_csv(f, path,
float_fmt: ".2f")` (or `ods.to_csv`) writes floats to a fixed number of
places. Dates go out as ISO 8601, so `dates: "auto"` reads them back.

`ods.read_csv` takes text, not a path: a single-line argument ending in
`.csv`, `.tsv`, or `.txt` is refused, naming
[`ods.read_csv_file`](#files-in-files-out) as the function that reads a
file. The check reads the string only; `read_csv` never touches the
file system.

### Files in, files out

`ods.read_csv_file` reads straight from disk, and `ods.write_csv` writes
a Frame back. The functions that touch the file system (these two, the
JSON-lines and native-format readers and writers, and the streaming
readers) demand the `fs` [capability](packages.md#capabilities): read
level to read, write level to write. Everything else in `ods` is pure
and needs no grant.

Reading returns a `Result`, because a missing or malformed file is a
failure the caller can handle:

```olang no-run
use ods { col }
let sales = unwrap(ods.read_csv_file("sales.csv"))
let by_region = sales.group_by("region").agg({ total: col("amount").sum() })
unwrap(ods.write_csv(by_region, "summary.csv"))
```

`ods.to_csv` is the serializer on its own, returning the text rather than
writing it — for a Frame that is going into an HTTP response, a database,
or a pipe rather than a file. It cannot fail, so it returns the string
outright:

```olang
let f = ods.frame({ k: ["a", "b"], v: [1, 2] })
print(ods.to_csv(f))
```

It is the exact inverse of `ods.read_csv`: a null becomes an empty cell,
which is what `read_csv` reads back as null, and a value containing a
comma or a quote is quoted so the column count survives the trip.

`ods.frame_from_records` takes a list of maps or objects — what
`json.parse` yields for a JSON array of objects — with columns formed
from the union of keys in the order first met (the first record's, then
each new key where it first appears) and missing keys becoming nulls:

```olang
let records = unwrap(json.parse("[{\"name\": \"ada\", \"score\": 99}, {\"name\": \"bob\"}]"))
let f = ods.frame_from_records(records)
println(to_string(ods.columns(f)))                                // ["name", "score"]
println(to_string(ods.null_count(f["score"])))        // 1
```

The inverse, `ods.to_records(f)`, turns a Frame back into a list of
maps — the exit ramp for row-at-a-time output like printing a report
or serializing JSON.

### Files larger than memory

`ods.read_csv_file` holds the whole table at once, which is the right
shape until the file no longer fits. `ods.open_csv` returns a reader that
hands back one chunk at a time, each chunk an ordinary Frame with the
file's columns:

```olang no-run
let r = unwrap(ods.open_csv("events.csv"))
let mut total = 0.0
loop {
    let chunk = unwrap(ods.next_chunk(r, 50000))
    if ods.n_rows(chunk) == 0 => break
    total = total + ods.sum(chunk["amount"])
}
println(to_string(total))
```

Only the current chunk is resident, so the memory a program uses is set
by the chunk size rather than the file size. A chunk is a Frame, so
every transform, join, and aggregation in this chapter applies to it.

The reader ends when a chunk comes back with zero rows. That final empty
chunk still carries the file's columns, so a pipeline written against a
chunk does not need a special case for it. `ods.rows_read(r)` reports how
many rows have been delivered so far and `ods.at_end(r)` whether the file
is exhausted; asking for fewer than one row per chunk is refused, since a
zero-row chunk is the loop's own stopping signal.

A reader is not a value like a Frame. It holds a position in a file —
mutable state — and so it belongs to the thread that opened it. Reaching
one from a `spawn`ed task or sending one down a channel is refused, as
it is for a [`cell`](language.md#cells-the-one-mutable-location). To
process chunks in parallel, read on one thread and send the chunks:

```olang no-run
let r = unwrap(ods.open_csv("events.csv"))
let mut totals = []
loop {
    let chunk = unwrap(ods.next_chunk(r, 50000))
    if ods.n_rows(chunk) == 0 => break
    totals = totals + [spawn ods.sum(chunk["amount"])]
}
```

`ods.open_csv` reaches the filesystem, so like `read_csv_file` it demands
the `fs` capability at read level.

### JSON lines

JSON lines hold one JSON object per line. `ods.read_jsonl` parses the text and `ods.read_jsonl_file`
reads it from disk. Columns are the union of the keys, and a record
missing one gets a null, as in `frame_from_records`:

```olang
let f = unwrap(ods.read_jsonl("{\"name\": \"ada\", \"score\": 99}\n{\"name\": \"bob\"}\n"))
println(to_string(ods.columns(f)))                            // ["name", "score"]
println(to_string(ods.null_count(f["score"])))    // 1
```

Blank lines are skipped. A malformed line, or one holding something
other than an object, is an `Err` naming the line number.

`ods.to_jsonl` serializes a Frame back, one object per row. It omits
nulls rather than writing them, so the round trip gives back the same
Frame.
`ods.write_jsonl` writes it to a file and demands `fs` at write level.

`ods.open_jsonl` streams it, driven by the same verbs as `open_csv`:

```olang no-run
let r = unwrap(ods.open_jsonl("events.jsonl"))
let mut total = 0.0
loop {
    let chunk = unwrap(ods.next_chunk(r, 50000))
    if ods.n_rows(chunk) == 0 => break
    total = total + ods.sum(chunk["amount"])
}
```

`ods.rows_read` and `ods.at_end` work on either reader, and a function
that takes a reader takes both. JSON lines carry no header, so a reader
remembers the columns its earlier chunks established and hands them back
on the empty chunk that ends the loop, as the CSV reader does.

### The native format

CSV and JSON lines re-parse text and re-infer types on every load, and
neither records what a column *was*: a column of zero-padded codes
written to CSV comes back as integers, with the padding gone.

```olang
let codes = ods.frame([["zip", ["007", "042", "100"]]])
println(to_string(ods.to_list(ods.read_csv(ods.to_csv(codes))["zip"])))   // [7, 42, 100]
```

`ods.write_frame` and `ods.read_frame` are the pair to use when the
reader is going to be olang. Types survive exactly, the load is a read
rather than a parse, and a single column can be fetched without touching
the others:

```olang no-run
unwrap(ods.write_frame(sales, "sales.olc"))
let all = unwrap(ods.read_frame("sales.olc"))
let some = unwrap(ods.read_frame("sales.olc", ["region", "revenue"]))
```

The second argument names the columns to decode, in the order wanted;
everything else is stepped over using the byte lengths in the header.
Over 500,000 rows and six columns, a CSV load takes 138 ms and the same
data from a columnar file 50 ms, or 26 ms for one column. The header is
UTF-8 text, readable with `head`:

```text
olang-columns 1
rows 500000
columns 6
col "id" Int enc=plain nulls=0 bytes=4000000
col "region" String enc=dict nulls=0 bytes=2000066
col "amount" Float enc=plain nulls=0 bytes=4000000
data
```

`ods.frame_info(path)` reads exactly that much and returns it as a Frame,
so a program can ask what is in a file — its columns, their types, how
many nulls each holds, and how many bytes each occupies — without loading
any of it.

A Date is four bytes a row and a Datetime eight, and a Category is
written as its dictionary — its levels in their order, then a code a
row — so the file keeps a category's order as well as its values.

`enc` is how a String column is stored: `plain`, each row's text, or
`dict`, a dictionary of its distinct values and a small code per row.
The writer computes both sizes and takes the smaller.

### Looking at one

Printing a Frame prints a table: the shape line, the column names, each
column's type, and the rows:

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\n")
println(to_string(sales))
```

```text
Frame[2 x 3]
┌────────┬────────┬─────┐
│ region │ amount │ qty │
│ String │  Float │ Int │
├────────┼────────┼─────┤
│ east   │   25.5 │  10 │
│ west   │  320.0 │   3 │
└────────┴────────┴─────┘
```

Numeric columns are right-aligned and text columns left-aligned, so a
column of numbers can be read down its last digit; nulls print as `—`.
A Frame larger than the terminal is capped four ways — twenty rows,
twenty-eight characters per cell, a hundred characters of width, and
however many columns fit in it — and every cap that bites is reported
under the table. A long Frame keeps both ends and elides the middle.

`ods.head(f, n)` takes the first `n` rows (`n` defaults to 10).
`ods.columns(f)` lists the column
names, and `ods.n_rows` and `ods.n_cols` give the dimensions.

`ods.describe(f)` summarizes every column at once — count, nulls, mean,
standard deviation, and the five-number summary. It returns a *Frame*,
not a map, so it prints as a table and can itself be sorted, filtered, or
written to a file:

```olang
let f = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\n")
println(to_string(ods.describe(f)))
```

The numeric statistics are null for String and Bool columns, so the
summary has the same shape whatever the input. `ods.schema(f)` is the same thing without the arithmetic — name, type,
and null count per column — for a Frame too wide to summarize
comfortably.

### Reaching a column

`f["amount"]` is the column as a Series, and so is `f.amount` (a field
read; see [Methods, names and fields](#methods-names-and-fields)):

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\n")
println((sales["amount"] * sales.qty).to_list())   // [255.0, 960.0]
```
 A Series takes a position:
`s[0]` is its first element and a position outside it (a negative one
included) raises, the same rule lists and strings follow, so a column
reads like the list it stands in for.

A Frame also takes a mask, which is how filtering reads:

```olang
let sales = ods.read_csv("region,amount\neast,25.5\nwest,320.0\neast,80.0\n")
let big = sales[sales["amount"] > 50.0]
println(to_string(ods.to_list(big["region"])))
```

A list of names selects those columns, in that order —
`f[["region", "amount"]]` is `ods.select(f, ["region", "amount"])`.

The subscript's meaning follows the key's *type*: a String selects a
column, a list of Strings selects columns, a Bool Series selects rows. A
row *position* is refused, naming `ods.row`, `ods.slice` and `ods.take`
instead.

A column name that is not in the Frame raises, and says what columns the
Frame has. Subscripting is read-only: there is no `f["a"] = x`. Frames
are values and every verb returns a new one; add or replace a column
with `ods.with` or `ods.with_column`.

### Combining conditions

A real filter has more than one condition. `&`, `|` and `^` combine
masks elementwise (and, or, exclusive or), and `!` inverts one; `&&`
and `||` short-circuit on a single Bool and do not apply to columns. As
in Rust and pandas, `&` and `|` bind tighter than a
comparison, so each comparison they join goes in parentheses. `ods.all_of` and `ods.any_of` take
a *list* of masks, for a filter built up as data:

```olang
let f = ods.read_csv("quality,kwh\nok,5.0\nfail,7.0\nok,-1.0\nok,9.0\n")
let usable = f[ods.eq(f.quality, "ok") & (f.kwh > 0.0)]
println(to_string(usable.kwh.to_list()))                     // [5.0, 9.0]
println(to_string(f[!(f.kwh > 0.0) | (f.kwh > 8.0)].kwh.to_list()))   // [-1.0, 9.0]
println(to_string(ods.all_of([ods.ne(f.quality, "fail"), f.kwh > 0.0]).sum()))   // 2
```

`ods.eq` and `ods.ne` are equality as a *mask*, taking either another
Series or a plain value (`==` between two Series is structural).

Combination is three-valued, as in SQL; `filter` treats a null as false. One `false`
settles an `&` (or an `all_of`) even when the other side is unknown,
and one `true` settles an `|` (or an `any_of`); otherwise an unknown
propagates. A comparison
against a null is itself unknown, which is why the mask above does not
need a null check in front of it.

Two more masks name the conditions a chain of `eq`s spells badly.
`ods.is_in(s, values)` is true where the element is one of `values` (a
list or a Series; a number matches either kind of number, so `2` finds
`2.0`), and `ods.contains(s, text)` is true where a String element
contains `text`, case-sensitive — a column of any other type is an
error naming it. Both compose with `&`, `filter` and a Frame's mask
subscript like any comparison:

```olang
let f = ods.read_csv("region,product,amount\neast,gadget,120.0\nnorth,gadget,300.0\nwest,widget,150.0\nwest,gadgetry,90.0\n")
let picked = f[f.region.is_in(["east", "west"])
               & (f.amount >= 100.0)
               & f["product"].contains("gad")]
println(to_string(ods.to_list(picked["amount"])))            // [120.0]
```

A null element is unknown to both, as it is to a comparison, so a
filter drops it. `is_in` keeps nulls when `values` holds `()` itself:
`ods.is_in(f["region"], ["east", ()])` is "east, or not known".

### Stacking Frames

`ods.concat(frames)` puts Frames on top of one another, for example the
partial results of a streamed aggregation:

```olang
let a = ods.read_csv("x,y\n1,a\n")
let b = ods.read_csv("x,y\n2,b\n")
println(to_string(ods.to_list(ods.concat([a, b])["x"])))     // [1, 2]
```

Columns are matched by *name*, not position. A missing or extra column
is refused and named rather than padded with nulls. An Int column
meeting a Float one widens to Float.

### Shaping columns and rows

`ods.select(f, names)` keeps named columns; `ods.with_column(f, name,
col)` returns a Frame with one column added or replaced, and
[`ods.with`](#expressions--odscol) adds several from expressions:

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,80.0,4\n")
let full = sales.with_column("revenue", sales.amount * sales.qty)
println(full.columns())    // ["region", "amount", "qty", "revenue"]
```

`ods.drop(f, names)` names the columns to remove rather than the ones
to keep, so it keeps a column added upstream later.

`ods.rename(f, mapping)` takes a map or an object of old name to new
name. It refuses a name that is not in the Frame and a target name that
already exists. A common use is the `_right` suffix a colliding join
leaves behind:

```olang
let sales = ods.frame([["id", [1, 2]], ["amount", [10.0, 20.0]]])
let costs = ods.frame([["id", [1, 2]], ["amount", [3.0, 4.0]]])
let j = sales.join(costs, "id").rename({ amount_right: "cost" })
println((j.amount - j.cost).to_list())    // [7.0, 16.0]
```

On the row axis, `ods.filter(f, mask)` keeps rows where a Bool series
is true (build the mask from any column), `ods.take(f, idx)` gathers
rows by index (an Int Series or a list), `ods.head(f, n)` keeps the first `n` and `ods.tail(f, n)`
the last `n` (both defaulting to ten, and both taking a Series as well,
whose first or last `n` values they return), and `ods.sort_by(f, col,
descending)` sorts the whole table by one column, nulls last either way:

```olang
let sales = ods.read_csv("region,amount\neast,25.5\nwest,320.0\neast,80.0\n")
let big = sales.filter(sales.amount > 50.0).sort_by("amount", descending: true)
println(big.region.to_list())    // ["west", "east"]
```

Asking `tail` for more rows than the Frame holds returns the whole
Frame rather than an error.

`ods.slice(f, start, n)` is `n` rows from `start` (a negative start
counts from the end; a Series slices the same way), and `ods.row(f, i)`
is one row as a map of column to value.

`ods.lookup(f, key_col, key, value_col)` reads one value by key: the
`value_col` of the first row whose `key_col` is `key`, or `()` when no
row has it. A list or Series of keys gives a Series, null where a key is
missing. The first lookup builds a hash of `key_col` and keeps it with
the Frame, so a loop of lookups against one table hashes it once:

```olang
let hoods = ods.frame({ hood: ["Old Town", "Hillcrest"], km: [1.5, 6.0] })
println(to_string(ods.lookup(hoods, "hood", "Hillcrest", "km")))      // 6.0
println(to_string(ods.lookup(hoods, "hood", "Nowhere", "km")))        // ()
println(to_string(hoods.lookup("hood", ["Old Town", "x"], "km").to_list()))   // [1.5, ()]
println(to_string(ods.row(hoods, -1)))
println(to_string(ods.slice(hoods, 1, 1)["hood"].to_list()))           // ["Hillcrest"]
```

### Duplicates and missing rows

Two verbs work on whole rows rather than columns. `ods.distinct(f)`
removes duplicate rows, keeping the first occurrence of each in its
original order. `ods.drop_null(f)` removes the rows
that are null anywhere. Both take an optional list of column names to
consider instead of every column:

```olang
let f = ods.read_csv("region,day,amount\neast,mon,1.0\neast,tue,2.0\nwest,mon,3.0\n")
println(to_string(ods.n_rows(ods.distinct(f))))              // 3 — no exact repeats
println(to_string(ods.to_list(ods.distinct(f, ["region"])["day"])))    // ["mon", "mon"]
```

Restricting `distinct` to a subset keeps the *first whole row* for each
distinct combination of those columns, so the columns outside the
subset come along unchanged rather than being aggregated. When the
question is which values occur rather than which rows, `ods.group_by`
is the verb that answers it.

Rows compare by their values, field by field: a row of `["a,b", "c"]`
and a row of `["a", "b,c"]` are distinct, and a null is distinct from
an empty string (the CSV reader reads both an empty cell and a quoted
`""` as null).

To fill nulls in a Frame's column, fill the column in an expression:

```olang
use ods { col }
let f = ods.frame({ amount: [1.0, (), 3.0] })
let filled = f.with({ amount: col("amount").fill_null(0.0) })
println(filled.amount.to_list())    // [1.0, 0.0, 3.0]
```

### Grouped aggregation

`ods.group_by(f, keys)` partitions the rows by one or more key columns
and gives back a **GroupBy**: the group of every row, computed once, for
the verbs that follow to reuse. `by.agg({ name: expr, … })` aggregates
with expressions, one output column per entry:

```olang
use ods { col }
let sales = ods.frame({
    region: ["east", "west", "east", "west", "east"],
    amount: [25.5, 320.0, 80.0, 40.0, 12.0],
    qty: [10, 3, 4, 2, 6],
})
let by = sales.group_by("region")
println(len(by))                     // 2 groups
let summary = by.agg({
    revenue: col("amount").sum(),
    orders: ods.count(),
    typical: col("amount").median(),
    p90: col("amount").quantile(0.9),
    per_unit: col("amount").sum() / col("qty").sum(),
    biggest_row: col("amount").argmax(),
})
println(summary)
```

An aggregation is a reduction of a column expression: `count` (its
valid values; `ods.count()` alone counts the group's rows), `sum`,
`mean`, `min`, `max`, `median`, `quantile(q)`, `std`, `var`, `first`,
`last`, `n_unique`, `any`, `all`, and `argmax`/`argmin` (the row
position, in the whole Frame, of the group's extreme — `ods.take(f,
…)` fetches those rows). The column can be any expression
(`(col("price") * col("qty")).sum()`), and arithmetic over aggregations
is computed per group, as `per_unit` is above. A key column stands for
the group's key. The expressions an aggregation reads are evaluated
together, in one fused pass over the rows, and each group's reduction
then runs over the group ids already made.

The GroupBy answers more than `agg`:
- `by.sum()`, `by.mean()`, `by.median()`, `by.std()`, … — the reduction
  of every numeric column (`count`, `first`, `last` and `n_unique`: of
  every column; `any` and `all`: of every Bool column), keys first;
- `by.size()` — the rows in each group, as a `size` column;
- `by.transform(expr)` — the aggregation given back to every row, the
  same as `expr.over(keys)` (an object of them gives a Frame);
- `by.apply((g) => …)` — the function called once per group with the
  group's rows as a Frame. Frames that come back are stacked under their
  group's keys; records (an object or map) become one row each beside
  the keys; plain values a `value` column; anything else, the list of
  results;
- `by.groups()` — a list of `(key, frame)` pairs, in group order (a
  key of several columns is a tuple), for a loop over the groups.

```olang
use ods { col }
let sales = ods.frame({ region: ["east", "west", "east"], amount: [25.5, 320.0, 80.0] })
let by = sales.group_by("region")
println(by.mean())
println(by.transform(col("amount").mean()).to_list())     // [52.75, 320.0, 52.75]
let spans = by.apply((g) => { n: ods.n_rows(g), spread: ods.max(g["amount"]) - ods.min(g["amount"]) })
println(spans)
for (region, rows) in by.groups() {
    println(`${region}: ${ods.n_rows(rows)} rows`)
}
```

`median`, `std` and `var` reduce each group's non-null values (sample
`std`/`var`, null below two values; all three Float). `first` and
`last` take the group's first and last row's value as it stands, a null
included, on a column of any type, and `n_unique` counts a group's
distinct values with null as one value. A Bool column sums to its count
of true and averages to its share. Groups keep first-seen order, and a
null key forms its own group:

```olang
use ods { col }
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,80.0,4\n,9.0,1\n")
let summary = sales
    .group_by("region")
    .agg({ total: col("amount").sum(), n: ods.count(), qtys: col("qty").n_unique() })
    .sort_by("total", descending: true)
for rec in summary.to_records() {
    println(`${rec.region}: ${rec.total} over ${rec.n} sales`)
}
```

`ods.group_by(f, keys, aggs)` with an object of aggregations is
`ods.group_by(f, keys).agg(aggs)`. It also takes a list of
`[out_name, op, column]` triples, with ops `count` (the group's rows,
nulls included, so the column may be left out), `sum`, `mean`, `min`,
`max`, `median`, `std`, `var`, `first`, `last`, `n_unique`, `any`,
`all`, `argmax` and `argmin`.

A grouped float `sum` or `mean` on a large Frame adds in an order fixed
by the chunk layout, so a given input always gives the same result,
which may differ from a one-thread run in the last bits; every other
aggregation is the same on any number of threads. `apply` calls the
function once per group, in group order.
[Writing fast olang](performance.md#frames-grouping-joins-and-time)
has the measurements.

### Crosstabs and runs

`ods.crosstab(a, b)` counts the rows of each pair of values: one row per
value of `a`, one column per value of `b`, both in ascending order
(nulls left out), and 0 where a pair never occurs. `values:` and `agg:`
aggregate a third column instead of counting (null where a pair never
occurs), `normalize:` makes the counts shares — `"all"` (or `true`) of
the table's total, `"rows"` of each row's, `"columns"` of each column's
— and `name:` names the first column (default `row`):

```olang
let cluster = ods.series([0, 1, 0, 1, 1])
let persona = ods.series(["regular", "lapsed", "regular", "regular", "lapsed"])
let table = ods.crosstab(cluster, persona, name: "cluster")
println(table)
let shares = ods.crosstab(cluster, persona, normalize: "rows")
println(shares["regular"].to_list())    // [1.0, 0.333333…]
```

`ods.runs(mask)` finds the runs of consecutive true rows of a Bool
column: a Frame with each run's `start` and `end` row (inclusive) and
`length`. Of any other column it gives every run of equal values, with
its `value`. `ods.run_id(s)` numbers the runs instead — 0 for the first
row, one more wherever the value changes (wherever any of several
columns changes, given a list) — which is the key a `group_by` of
consecutive rows needs:

```olang
use ods { col }
let z = ods.series([0.1, 4.2, 5.0, 0.3, -6.1, -4.4, 0.2])
println(ods.runs(z.abs() > 3.0))
let kind = ods.select([z > 3.0, z < -3.0], ["spike", "drop"])
let events = ods.frame({ hour: ods.arange(7), z: z, kind: kind })
    .with({ event: ods.run_id(col("kind")) })
    .filter(col("kind").not_null())
    .group_by("event")
    .agg({ kind: col("kind").first(), start: col("hour").first(), hours: ods.count(), peak: col("z").abs().max() })
println(events)
```

### Reshaping between long and wide

The same data has two shapes. **Long** is one row per observation, which
is what a database returns and what `group_by` and the plotting verbs
want. **Wide** is one row per subject and one column per category, which
is what a person reads in a table. `ods.pivot` and `ods.unpivot` move
between them.

`ods.pivot(f, index, columns, values, agg)` spreads: the distinct values
of `index` become rows, the distinct values of `columns` become new
columns, and each cell is the `agg` reduction of `values` over the rows
that share both, computed as `group_by` computes it.

```olang
let sales = ods.read_csv(
    "region,quarter,amount\neast,Q1,10.0\neast,Q2,20.0\nwest,Q1,5.0\neast,Q1,3.0\n")
let wide = ods.pivot(sales, "region", "quarter", "amount", "sum")
println(to_string(ods.columns(wide)))          // ["region", "Q1", "Q2"]
println(to_string(ods.to_list(wide["Q1"])))    // [13.0, 5.0] — east's two Q1 rows summed
println(to_string(ods.to_list(wide["Q2"])))    // [20.0, ()] — west never reported Q2
```

The aggregation is required. A cell no row reached is null unless
`fill:` says what it is instead (`ods.pivot(sales, "region", "quarter",
"amount", "sum", fill: 0.0)`).

A null in the `columns` column is refused, pointing at `drop_null`, and
so is a value that would name a column the index already has.

`ods.unpivot(f, ids, value_columns)` gathers, the other direction: the
`ids` stay as they are and every other column becomes rows of a
`name`/`value` pair. Omitting `value_columns` takes every column that is
not an id.

```olang
let wide = ods.frame([["region", ["east", "west"]], ["Q1", [13.0, 5.0]], ["Q2", [20.0, 1.0]]])
let long = ods.unpivot(wide, "region")
println(to_string(ods.columns(long)))            // ["region", "name", "value"]
println(to_string(ods.to_list(long["name"])))    // ["Q1", "Q1", "Q2", "Q2"]
```

The value columns stack into one column, so they share a type: Int
columns beside Float ones widen to Float, as `ods.concat` widens them;
any other mix (text beside numbers, a Date beside a Float) is refused,
naming the two columns, and needs an `ods.cast` first.

`unpivot` after `pivot` returns the long form with the combinations that
never occurred present as nulls; `ods.drop_null` removes them.

### Time buckets: `resample`

`ods.resample(f, on, every, aggs)` groups the rows by the start of the
time bucket each falls in and aggregates them, sorted by time: the
`group_by` of a floored time column. `on` is a Date or Datetime column
(its buckets come back as its own type), or ISO dates or date-times as
text (`2024-03-01`, `2024-03-01T13:45`, `2024-03-01 13:45:10`); `every` is a count and a unit — `"30s"`,
`"15m"`, `"1h"`, `"1d"`, `"1w"` (weeks start on Monday), `"1mo"`,
`"1q"`, `"1y"`; `aggs` is what `by.agg` takes. `by:` adds keys to group
by before the time. The bucket column keeps the name `on` and is
written in the input's shape — a date for a bucket of a day or more,
the date and time otherwise:

```olang
use ods { col }
let hourly = ods.frame({
    ts: ["2024-01-01T01:00", "2024-01-01T13:00", "2024-01-02T05:00", "2024-02-03T00:00"],
    mw: [10.0, 20.0, 30.0, 40.0],
    site: ["a", "b", "a", "a"],
})
println(ods.resample(hourly, "ts", "1d", { energy: col("mw").sum(), hours: ods.count() }))
println(ods.resample(hourly, "ts", "1mo", { energy: col("mw").sum() }, by: "site"))
```

`ods.floor(s, every)` (or `col("ts").floor("1w")`) is the bucket
column itself, each value's bucket start in the same shape — the key a
join on weeks or months needs (a Date or Datetime column's `floor` is
the same buckets, as dates):

```olang
let days = ods.series(["2024-03-14", "2024-03-17", "2024-03-18"])
println(to_string(days.floor("1w").to_list()))     // ["2024-03-11", "2024-03-11", "2024-03-18"]
println(to_string(days.floor("1mo").to_list()))    // ["2024-03-01", "2024-03-01", "2024-03-01"]
```

Text timestamps are parsed once into integer buckets; a Date or Datetime
column needs no parse.

### Joins

`ods.join(a, b, on_a, on_b)` is an inner hash join on one key column
from each side; `ods.join_left` keeps every left row, filling the
right side with nulls where nothing matched. Null keys never match
(the SQL convention), and a column-name collision on the right gains
a `_right` suffix:

```olang
let totals = ods.frame([["region", ["east", "west"]], ["total", [105.5, 320.0]]])
let rates = ods.frame([["name", ["east", "west"]], ["rate", [0.07, 0.09]]])
let joined = totals.join(rates, "region", "name")
println((joined.total * joined.rate).to_list())
```

Three more kinds complete the set. `ods.join_full(a, b, on)` keeps every
row from both sides, filling the other side's columns with nulls where
nothing matched; its key column takes whichever side has a value, so a
row that came only from the right is still identified by its key rather
than being null. A full join refuses key columns of different types,
where the other kinds simply find nothing.

`ods.join_semi(a, b, on)` and `ods.join_anti(a, b, on)` ask about
existence rather than combination: semi keeps the rows of `a` that have
a match, anti keeps those that do not, and both return `a`'s columns
alone. Where `b` has three rows for a key, an inner join returns three
rows and a semi join one:

```olang
let orders = ods.frame([["customer", ["ada", "bob"]], ["total", [10.0, 20.0]]])
let payments = ods.frame([["customer", ["ada", "ada"]], ["paid", [4.0, 6.0]]])
println(to_string(ods.n_rows(ods.join(orders, payments, "customer"))))       // 2
println(to_string(ods.n_rows(ods.join_semi(orders, payments, "customer"))))  // 1
println(to_string(ods.to_list(ods.join_anti(orders, payments, "customer")["customer"])))
```

An inner or left join keeps the left Frame's row order (a left row
matched twice is followed by its second match), so there is nothing to
re-sort after one; a full join appends the right side's unmatched rows
after them.

Every kind follows the same null rule: a null key matches nothing. So a
null-keyed left row appears in a left, full, or anti join and never in
an inner or semi one, and semi and anti always partition the left frame
between them.

`ods.join` takes the kind by name as well — `how:` one of `"inner"`
(the default), `"left"`, `"full"`, `"semi"` or `"anti"` — and every join
takes these:
- `on` a list of names joins on several keys at once (`right_on` a list
  of the same length when the right side calls them otherwise);
- `suffix:` replaces `_right` on a colliding right column;
- `validate:` says which side holds each key once — `"many_to_one"` (the
  right: a lookup table), `"one_to_many"` (the left) or `"one_to_one"`
  (both) — and raises, naming the key, when it does not, so a duplicated
  key in a lookup table is an error rather than multiplied rows.

```olang
let sales = ods.frame({ region: ["n", "s", "n"], week: [1, 1, 2], units: [5, 7, 9] })
let promos = ods.frame({ region: ["n", "s"], week: [2, 1], discount: [0.1, 0.2] })
let j = ods.join(sales, promos, ["region", "week"], how: "left", validate: "many_to_one")
println(j["discount"].to_list())           // [(), 0.2, 0.1]
let dup = ods.frame({ region: ["n", "n"], week: [2, 2], discount: [0.1, 0.3] })
println(attempt(() => ods.join(sales, dup, ["region", "week"], validate: "many_to_one")))
```

`ods.asof_join(left, right, on, by)` matches on the nearest value
rather than an equal one: each left row takes the right row whose `on`
is the last at or before its own (`direction: "forward"` the first at or
after, `"nearest"` the closer), among the right rows with the same `by`
keys — the reading in force at each event, the last quote before each
trade. `on` is numbers, Dates or Datetimes, or ISO text on both sides;
`tolerance:` (numbers: days for a Date, seconds for a Datetime) bounds
the distance, and every left row stays, in order, with nulls
where nothing qualifies. The right side need not be sorted:

```olang
let readings = ods.frame({ t: [0, 60, 120], site: ["a", "a", "a"], temp: [10.0, 12.5, 11.0] })
let events = ods.frame({ t: [30, 125, 5], site: ["a", "a", "b"] })
println(ods.asof_join(events, readings, "t", by: "site")["temp"].to_list())               // [10.0, 11.0, ()]
println(ods.asof_join(events, readings, "t", direction: "nearest")["temp"].to_list())     // [10.0, 11.0, 10.0]
```

A join on several keys costs little more than a join on one. On a large
left Frame the probe runs across every core, and the result is the
same as a one-thread run.

## Expressions — `ods.col`

`ods.col(name)` is an expression (an Expr) for a column of whatever
Frame it is given to: the Frame verbs below resolve it against their
Frame, so a pipeline can name columns of Frames it has not bound to a
variable:

```olang
use ods { col }
let houses = ods.read_csv("hood,price,sqft\nnorth,300000,1500\nsouth,210000,1400\nnorth,520000,2600\n")
let priced = houses
    .with({ ppsf: col("price") / col("sqft"), big: col("sqft") > 2000 })
    .filter(col("ppsf") > 160.0)
    .sort_by(["hood", "ppsf"], [false, true])
println(priced)
```

Everything that takes a column takes an expression: the operators
(`col("a") * 2 + col("b")`, `-col("x")`, `col("a") > 0 & col("b") < 5`),
every function of the sections above, as a method or by the module
(`col("x").exp()`, `col("x").fill_null("ffill")`, `ods.rank(col("x"))`,
`col("x").shift(1)`), and each of those gives an expression back. A
reduction gives a value every row shares: `col("x") - col("x").mean()`
centres a column, and a Date's or Datetime's `min`, `max`, `first`,
`last` or `mode` stays one, so `col("t") - col("t").min()` is seconds
since the first. `ods.lit(v)` is a
value as an expression, and a plain Series where an expression goes is a
literal column of the Frame's length.

The verbs that resolve expressions:

| Verb | What it does |
|---|---|
| `ods.with(f, { name: expr, … })` | adds each column, or replaces the one of that name. Every expression sees the Frame as it was; the whole object is one fused pass, a column several outputs read is read once, and a shared subexpression is computed once. A map (`#{ "name": expr }`) works too, and so does a list of named expressions. |
| `ods.filter(f, expr)` | the rows where a Bool expression holds (a null does not) |
| `ods.select(f, [names and exprs])` | those columns, in that order: a name, an expression (named by `ods.alias(expr, name)`, or else by the first column it reads), or an object of named expressions |
| `ods.sort_by(f, keys, descending)` | sorted by several keys: names or expressions, each ascending unless `descending` (a Bool for all, or a list of one per key) says otherwise. Ties keep their row order; nulls go last in every key. |

### Within groups: `over`

`ods.over(agg, keys)` computes an aggregation within each group of the
key columns and gives it back to every row of the group — pandas'
`transform`, Polars' `over`. The aggregation is any that `by.agg` takes
(see [Grouped aggregation](#grouped-aggregation)) — `ods.sum`,
`ods.mean`, `ods.quantile(e, q)`, `ods.count()` for the group's rows,
or arithmetic over several — and the keys are a column name or a list
of them. A group-wise fill is one line (the
filled column is Float, since a group's mean is):

```olang
use ods { col }
let homes = ods.frame([
    ["hood", ["north", "south", "north", "south", "north"]],
    ["built", [1990, 2004, (), (), 2000]],
])
let filled = homes.with({
    built: col("built").fill_null(col("built").mean().over("hood").round()),
    share: col("built") / ods.over(ods.sum(col("built")), "hood"),   // the module form
})
println(to_string(ods.to_list(filled["built"])))   // [1990.0, 2004.0, 1995.0, 2004.0, 2000.0]
```

The groups are the ones `ods.group_by` forms (a null key is a group of
its own).

### How expressions run

An expression is a description until a verb resolves it, so building
one costs nothing and printing one shows what it says
(`to_string(col("a") * 2)` is `(col("a") * 2)`). An Expr is not a lazy
Frame: the verb it is given to resolves it at once. The elementwise
parts — operators, math, `where`, `coalesce`, `clip`, `between`, the
null masks, casts — compile into one plan with the rest of the verb's
expressions, which runs chunk by chunk and across cores on large
Frames. A function that is not elementwise (a rank, a shift, a
reduction, `over`) runs once on its own column and joins the plan as an
input. Errors name the column that is missing and the columns there
are. [The expression engine](performance.md#the-expression-engine)
describes the cost model.

## Matrices — `ods.matrix` and `mat`

A `Matrix` is a dense table of Floats addressed by position rather than
by name: what `X @ b`, a Gram matrix, a table of distances or an
eigen-decomposition needs. It is column-major, and its columns are the
Frame's own. `ods.matrix(f, cols)` over Float columns with no nulls
shares them, so building one takes microseconds at any row count. A
transpose is a flag on the same storage, so `X.t() @ y` reads `X` once.

```olang
let f = ods.frame({
    sqft: [800.0, 1200.0, 1500.0, 2100.0, 950.0],
    age: [30.0, 12.0, 5.0, 2.0, 40.0],
    price: [210.0, 300.0, 360.0, 480.0, 220.0],
})
let X = ods.matrix(f, ["sqft", "age"])
println(X.shape())                              // [5, 2]
let b = mat.solve(X.gram(), X.t() @ f.price)    // least squares by the normal equations
println(len(X @ b))                             // 5
let e = mat.eigh(X.gram())
println(e.values.get(0) > e.values.get(1))      // true: largest first
println(mat.cdist(X, mat.of([[1000.0, 10.0]]), metric: "euclidean").shape())   // [5, 1]
println(ods.corr(f)["term"].to_list())          // ["sqft", "age", "price"]
```

- **Building one.** `ods.matrix(f)` takes every numeric column,
  `ods.matrix(f, ["a", "b"])` the ones named, and
  `ods.matrix([s1, s2])` a list of Series. Int and Bool columns are
  converted to Float. A null raises, naming the column and the count,
  unless `nulls: "drop"` leaves out every row that has one.
  `mat.of(rows)` builds a small matrix from a list of rows, and
  `mat.identity(n)` the identity.
- **Products.** `X @ b` is a Series when `b` is a list or Series of
  `X`'s width, and a Matrix when `b` is a Matrix. `v @ X` is `vᵀX`. A
  shape mismatch names both shapes.
- **Methods.** `X.t()`, `X.gram()` (`XᵀX`), `X.col(j)` (a Series,
  shared with the Matrix), `X.row(i)` (a list), `X.shape()`, `len(X)`
  (the row count), `X.mean(axis: 0)` and `X.std(axis: 0, ddof: 1)` (a
  Series by column; axis 1 gives one value per row), `X.to_frame(names)`
  and `mat.to_list(X)`.
- **Small dense routines.** `mat.solve(A, b)` uses LU with partial
  pivoting, and `b` may be a vector or a Matrix of right-hand sides.
  There are also `mat.inv(A)` and `mat.cholesky(A)` (the lower factor).
  `mat.eigh(A)` gives `#{ values, vectors }` of a symmetric matrix,
  largest first. Each vector's largest component is positive, so the
  signs do not depend on the run. NumPy's `eigh` orders smallest first.
- **Distances.** `mat.cdist(X, C, metric: "sqeuclidean")` gives the
  distances between the rows of `X` and the rows of `C` as an `n × m`
  Matrix. The other metrics are `"euclidean"`, `"cityblock"`,
  `"chebyshev"` and `"cosine"`.
- **Correlation.** `ods.corr(f, cols?, method: "pearson" | "spearman")`
  and `ods.cov(f, cols?)` return labelled Frames: a `term` column, then
  one column per variable, the way `df.corr()` prints. Columns without
  nulls take one pass: the Gram matrix of the centred columns. A column
  with nulls pairs over the rows both columns have, as pandas does.
- **Display.** A Matrix prints as a grid under `Matrix[n x k]`. The
  notebook and the REPL table views show it as a Frame with columns
  `0`, `1`, ….

None of it needs BLAS, so `mat` runs in the browser build too. Products
and the Gram matrix are summed in a fixed order, so a parallel result is
the one-thread result bit for bit; the timings against NumPy are in
[Matrices and formulas](performance.md#matrices-and-formulas).

## `stats` — from description to inference

The `stats` module covers description and inference. Its statistics,
p-values, distributions and coefficients are pinned against SciPy and
NumPy reference values in the test suite. Results come back as maps,
read with `.`.

### Describing a column

`stats.describe(s)` is the first look: count, null count, mean,
standard deviation, minimum, quartiles, and maximum in one map.

```olang
let s = ods.series([5.1, 4.9, 6.2, 5.7, (), 5.5])
let d = stats.describe(s)
println(`n=${d.count} nulls=${d.null_count} median=${d.median}`)
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
println(`p = ${t.p_value}`)
println(`means: ${t.mean_a} vs ${t.mean_b}`)
```

The map also carries the effect, not only its test: `diff` (the
difference of the means), its standard error `se`, `ci` (its interval
at `level`, 0.95 by default) and `d`, Cohen's d — the difference in
pooled standard deviations, the effect size power is planned in.
`alternative: "greater"` or `"less"` makes the test, and the interval,
one-sided.

```olang
let a = ods.series([5.1, 4.9, 6.2, 5.7, 5.5, 4.8, 5.9, 6.1])
let b = ods.series([4.2, 4.8, 4.5, 5.0, 4.4, 4.1, 4.9])
let t = stats.t_test(a, b)
let (lo, hi) = t.ci
println(lo < t.diff && t.diff < hi)   // true
println(t.d > 1.0)                    // true
```

The p-value is the probability of a gap at least this large *if the
groups had the same mean*, not the probability that the hypothesis is
true.

`stats.chi2_test(observed, expected)` is the goodness-of-fit test for
counts: did these category frequencies plausibly come from that
expected distribution?

```olang
let observed = ods.series([18.0, 22.0, 20.0])
let expected = ods.series([20.0, 20.0, 20.0])
let r = stats.chi2_test(observed, expected)
println(to_string(r.p_value > 0.05))   // consistent with uniform
```

### Power: how big must a study be?

`stats.power_t(effect, n)` is the chance a two-sample t-test at `alpha`
(0.05) detects a true standardized difference `effect` (Cohen's d) with
`n` in each group — computed from the noncentral t distribution, as
statsmodels does, not simulated. `stats.n_for_power(effect, power:
0.8)` inverts it. `kind: "one-sample"` (or `"paired"`) and
`alternative:` cover the other designs, `ratio:` unequal groups:

```olang
println(stats.power_t(0.5, 64) |> str.fmt(".3f"))          // 0.801
println(math.ceil(stats.n_for_power(0.5, power: 0.8)))     // 64
println(math.ceil(stats.n_for_power(0.2, power: 0.9)))     // 527
```

### Without the normal assumption

`stats.mann_whitney(a, b)` compares two samples by rank (its `effect`
is the chance a value of `a` beats one of `b`), and `stats.spearman(a,
b)` correlates ranks. For spread and outliers, `stats.mad(s)` is a
standard deviation the outliers cannot drag (the median absolute
deviation, scaled to a normal's sd), `stats.iqr(s)` the interquartile
range, `stats.tukey_fences(s, k: 1.5)` the pair of bounds outside which
a value is an outlier, and `stats.zscore(s, robust: true)` scores by
median and MAD:

```olang
let x = ods.series([10.0, 11.0, 9.5, 10.5, 10.2, 9.8, 48.0])
let (lo, hi) = stats.tukey_fences(x)
println(x.filter(x > hi).to_list() |> to_string)          // [48.0]
println(stats.zscore(x, robust: true).max() > 10.0)        // true
```

`stats.bootstrap(s, stat)` resamples natively — `"mean"`, `"median"`,
`"std"`, a quantile — and gives the percentile interval; a list of two
samples compares them (`compare: "diff"` or `"ratio"`). Each resample
draws from a stream of its own seed, so the answer does not depend on
the thread count:

```olang
let x = stats.norm.sample(300, 50.0, 8.0, rng: 11)
let b = stats.bootstrap(x, "mean", n: 2000, seed: 3)
let (lo, hi) = b.ci
println(`mean ${b.estimate |> str.fmt(".1f")}, 95% CI [${lo |> str.fmt(".1f")}, ${hi |> str.fmt(".1f")}]`)
```

### Regression — a complete workflow

`stats.lm(y, xs)` fits ordinary least squares with an intercept, where
`xs` is one predictor Series or a list of them. The example below fits
simulated data whose true coefficients are known:

```olang
random.seed(7)
let n = 500
let x1 = stats.norm.sample(n, 0.0, 1.0)
let x2 = stats.norm.sample(n, 0.0, 1.0)
let noise = stats.norm.sample(n, 0.0, 1.0)
let y = x1 * 2.0 + x2 * -1.0 + noise + 3.0    // truth: b0=3, b1=2, b2=-1

let fit = stats.lm(y, [x1, x2])
let coef = fit.coef
println(`intercept ≈ ${math.round(ods.get(coef, 0))}`)   // true value: 3
println(`b1 ≈ ${math.round(ods.get(coef, 1))}`)          // true value: 2
println(`b2 ≈ ${math.round(ods.get(coef, 2))}`)          // true value: -1
println(`r2 = ${fit.r2 > 0.5}`)
```

The result map carries the full coefficient table as parallel Series —
`coef`, `se` (standard errors), `t`, and `p_value`, with the intercept
at index 0 and predictors following in the order given — plus `r2`,
`adj_r2`, `n`, and `df_resid`. Rows containing any null drop before
fitting.

A coefficient's `p_value` tests whether it is zero, not whether the
effect is large. `r2` is the share of the variance in `y` the
predictors explain in this sample, and `adj_r2` adjusts it for the
number of predictors. For formulas, categorical terms and prediction,
use [`learn`](#learn--machine-learning).

### Distributions

Nine families — `stats.norm` (parameters `mean, std`), `stats.t`
(`df`), `stats.chi2` (`df`), `stats.f` (`d1, d2`), `stats.lognorm`
(`mu, sigma`), `stats.beta` (`a, b`), `stats.gamma` (`shape, scale`),
and the counts `stats.poisson` (`lam`) and `stats.binom` (`trials, p`)
— each with four functions (a count's density is its `pmf`):

| Function | Question it answers |
|---|---|
| `pdf(x, ...)` | how dense is the distribution at `x`? |
| `cdf(x, ...)` | what fraction of it lies at or below `x`? |
| `ppf(q, ...)` | which value has fraction `q` below it? (inverse cdf) |
| `sample(n, ...)` | draw `n` values as a Series |

`cdf` and `ppf` are inverses, and `ppf` gives critical values:

```olang
println(to_string(stats.norm.ppf(0.975, 0.0, 1.0)))    // 1.9599...
println(to_string(stats.norm.cdf(1.96, 0.0, 1.0)))     // ≈ 0.975
```

Each takes a Series as well as a number, and answers a Series — one
native pass over the column, in parallel when it is large:

```olang
let z = ods.series([-1.96, 0.0, 1.96])
println(stats.norm.cdf(z).to_list() |> map((p) => str.fmt(p, ".3f")))   // ["0.025", "0.500", "0.975"]
println(stats.poisson.pmf(ods.series([0, 1, 2]), 2.0).sum() |> str.fmt(".4f"))  // 0.6767
```

`sample(n, …, rng: g)` draws from a generator of its own
([`random.rng`](stdlib.md#generators-randomrng)), or an Int seed.
Without one, sampling draws from the `random` module's seeded stream, so
`random.seed(k)` makes a simulation reproducible:

```olang
random.seed(42)
let first = stats.norm.sample(5, 0.0, 1.0)
random.seed(42)
println(to_string(first == stats.norm.sample(5, 0.0, 1.0)))   // true
```

### Time series: `stats.ts`

`stats.ts` reads a column as a series in time: `acf` (by FFT, with
Bartlett's interval) and `pacf`, `decompose` into trend, season and
residual, Holt-Winters smoothing fitted by Nelder–Mead, and the
seasonal naive forecast to beat. Give `time:` (a Date or Datetime
column) and the answers carry it: a forecast is a Frame of the times
that follow.

```olang
let days = ods.arange(0, 84)
let stamp = ods.full(84, "2026-01-05").to_date().add_days(days)
let weekly = (days % 7).cast("Float")
let demand = days * 0.2 + 50.0 + weekly * weekly
let hw = stats.ts.holt_winters(demand, season: 7, time: stamp)
let next = hw.forecast(7)
println(next["time"].min())                         // 2026-03-30
println(next.n_rows())                              // 7
let corr = stats.ts.acf(demand - demand.rolling(7, "mean").fill_null("bfill"), 14)
println(corr.filter(ods.col("lag") == 7)["acf"][0] > 0.9)   // true
```

## `learn` — machine learning

`stats.lm` answers an inference question about one regression. `learn`
is for predictive modelling: split the data, fit a model, predict
held-out rows, and measure the result. The models are named values
rather than coefficient vectors. A fit knows its inputs by name and
encodes a new Frame the way it encoded the training one, so a String
column, a new category or a reordered frame cannot shift a coefficient.
The [reference](stdlib.md#learn-and-ml--machine-learning) lists every
function. This section walks through one workflow.

### A classification workflow

Simulated churn, where the truth is known: churn rises with `tickets`,
falls with `tenure`, and the `pro` plan churns less.

```olang
use ml
let g = random.rng(11)
let n = 2000
let tenure = g.normal(n, 24.0, 8.0)
let tickets = g.normal(n, 2.0, 1.0)
let plan = ods.where(g.normal(n) > 0.6, "pro", "basic")
let eta = tickets * 1.1 - tenure * 0.08 + 0.5 - ods.where(plan == "pro", 0.8, 0.0)
let f = ods.frame({ tenure: tenure, tickets: tickets, plan: plan, churn: eta + g.normal(n) > 0.0 })

let s = ml.split(f, test: 0.25, stratify: "churn", seed: 3)
let lr = ml.logistic(s.train, y: "churn")
println(lr.params.tickets > 0.0 && lr.params.plan_pro < 0.0)     // true
let p = ml.prob(lr, s.test)
println(ml.auc(s.test.churn, p) > 0.8)                            // true

let gb = ml.boost(s.train, y: "churn", trees: 200, seed: 3)
println(ml.auc(s.test.churn, ml.prob(gb, s.test)) > 0.8)          // true
println(ml.cross_validate(lr, f, folds: 5).n_rows())              // 5
```

`lr.coef` is the full table (estimate, se, z, p, the interval and the
odds ratio per term). `ml.importance(gb)` ranks the inputs.
`ml.roc(y, p)`, `ml.pr_curve(y, p)` and `ml.calibration(y, p)` return
Frames that plot directly, and `ml.at_threshold(y, p, threshold: 0.3)`
gives the counts at an operating point.

How the pieces fit:

- **Fits are maps.** `fit.params`, `fit.coef`, `fit.r2`, `fit.importance`
  and `fit.oob_score` are entries, so they read like fields without
  method calls. `fit.model` is the native model, and `fit.spec` is how
  to fit it again: its `kind`, `y`, `x` and `options` (`fit.spec.options.seed`
  is the seed a forest was grown with).
- **Evaluation refits natively.** `ml.cross_validate(fit, f)`,
  `ml.learning_curve` and `ml.grid_search` take a model or a spec
  (`ml.spec("forest", y: "churn", options: { n_trees: 200 })`; options
  are a map or an object, and an unknown key is an error) and
  refit it fold by fold in Rust, in parallel. `ml.cross_validate` also
  takes a `(train) => model` function, which runs once per fold.
- **Seeds make results exact.** Each random step draws from its own
  PCG stream of the seed: one per tree, per k-means start, per
  permutation repeat. A seeded program gives the same models on one
  thread or eighteen. Without a seed, one is drawn from the `random`
  stream, so `random.seed` still pins a whole program.
- **Preprocessing is fitted.** `ml.scaler(train)`, `ml.one_hot(train)`
  and `ml.imputer(train)` learn from the training rows, and
  `ml.transform(t, test)` applies what they learned. The models do
  their own one-hot encoding, so the encoder is for code that needs the
  dummy columns themselves.

### Formulas

Every supervised model takes a formula where `y` and `x` go, in R's and
patsy's notation: `"price ~ sqft + C(hood)"`. `+` adds a term, `a:b` is
an interaction, `a * b` is `a + b + a:b`, `C(x)` treats a column as
categorical, `I(…)` is arithmetic, `log(x)` and the other math
functions are terms, and `- 1` drops the intercept. A String or
Category column named bare is categorical. The models are also a
Frame's methods:

```olang
use ods { col }
let h = ods.frame({ sqft: ods.linspace(600.0, 2600.0, 60), hood: ods.tile(ods.series(["east", "west", "north"]), 20) })
let homes = h.with({ price: col("sqft") * 0.2 + ods.where(col("hood") == "east", 30.0, 0.0) + 50.0 })
let fit = homes.linear("price ~ sqft + C(hood)")
println(fit.params.sqft |> str.fmt(".2f"))            // 0.20
println(fit.params["hood_west"] |> str.fmt(".1f"))    // -30.0: east is the baseline
println(fit.predict(homes.head(2)).to_list())         // [200.0, 176.77966101694938]
```

A categorical term is coded with the first level as the baseline, and
`fit.predict(new)` encodes new rows with the training levels. The full
notation (nesting, powers, the response as an expression) is in the
[reference](stdlib.md#learn-and-ml--machine-learning).

### What the models are

- **Linear models.** OLS (the same estimates, standard errors and
  intervals as statsmodels), ridge, and the lasso and elastic net by
  coordinate descent on the Gram matrix (sklearn's objectives). Each
  sweep is O(k²) whatever the row count. Logistic regression uses
  Newton's method (IRLS) on the weighted Gram matrix, so it needs no
  learning rate and converges in a handful of iterations. With more
  than two classes it is multinomial.
- **Trees.** CART with exact best splits (sklearn's splitter). Random
  forests weight each tree's bootstrap and grow the trees in parallel.
  Gradient boosting bins each feature once into at most 255 quantile
  bins and grows each tree from histograms of gradient and Hessian
  sums. A node's larger child takes its histogram by subtraction, as
  in LightGBM and sklearn's `HistGradientBoosting`. Leaves split
  best-first, bounded by `max_depth` and `max_leaf_nodes`.
- **The rest.** Exact k-nearest neighbours: blocked distances, one
  vectorized column at a time. Gaussian naive Bayes. k-means with
  k-means++ seeding, where assignment is the parallel part. Gaussian
  mixtures by EM, initialized from k-means. PCA by eigen-decomposition
  of the covariance matrix.

None of them needs BLAS at these shapes (many rows, tens of columns),
so `learn` is pure Rust and ships in the browser build.

[Against scikit-learn](performance.md#against-scikit-learn) compares fit
times and scores.

## `plot` — charts as SVG text

A `plot` function returns a complete, standalone SVG document as a
String: `fs.write_file` puts it on disk, an `http.serve` handler serves
it, and `dom.set_html` puts it on a page.

| Function | Draws |
|---|---|
| `plot.line(x, y, opts)` | one line through xy points |
| `plot.scatter(x, y, opts)` | one point cloud |
| `plot.area(x, y, opts)` | a line with the region beneath it filled |
| `plot.lines(x, pairs, opts)` | several lines with a legend; `pairs` is `[[label, y], ...]` (up to 8) |
| `plot.xy(entries, opts)` | layered marks over shared scales; each entry is `[label, mark, x, y]` with mark `"line"`, `"area"`, or `"scatter"` — the multi-series form (shared axes, one legend) |
| `plot.bar(labels, values, opts)` | one bar per category; `labels` is a Series or list |
| `plot.bars(labels, pairs, opts)` | grouped (side-by-side) bars, one group per category |
| `plot.stacked(labels, pairs, opts)` | stacked bars — non-negative values only (a negative part misleads) |
| `plot.hist(s, bins, opts)` | binned counts of one numeric Series |
| `plot.heatmap(x_labels, y_labels, rows, opts)` | a matrix of values as colored cells on a sequential scale |
| `plot.box(pairs, opts)` | five-number-summary boxes: whiskers to min/max, quartile box, median line |

Options are named, or ride in a single trailing map — `title`, `x_label`, `y_label`, `width`,
`height`, `theme`, `paper`, `ink`, `responsive`, `interactive`,
`colors`, `vary`, `scale` — and leaving them out accepts the defaults, which follow a
colorblind-validated ten-hue palette assigned in fixed series order.
`theme: "dark"` re-tunes every color for a dark surface (mint leads);
`responsive: true` drops the fixed pixel size so the SVG fills its
container (the browser case — the viewBox keeps the aspect ratio).
`paper` and `ink` take the frame's colors from the page instead of the
theme: `paper` is the background as any CSS color (`"transparent"` lets
the page show through), and `ink` is the color of every label, tick,
axis, and grid line — the lines at reduced opacity — so `ink:
"currentColor"` with a transparent paper gives one chart that sits
correctly on a light and a dark page. An unknown option key is an
error.

`colors: ["#5aa9e6", ...]` gives a chart its own palette (series take
the list in order, cycling). `vary: true` makes a single-series bar
chart color each *category* from the palette. Heatmaps
pick a ramp with `scale`: `"auto"` (the theme's sequential scale),
`"ocean"`, `"ember"`, `"thermal"`, or `"diverging"` (signed data:
cold through the surface color to warm). `plot.ramp(name, t)` gives
one color of a ramp for `t` in [0, 1].

Nulls: xy plots drop a point when either coordinate is null; the bar
family rejects null values (call `fill_null` or `filter` first); `box`
drops nulls.

```olang
let x = ods.linspace(0.0, 6.28, 60)
let svg = plot.lines(x, [["sin", x.sin()], ["cos", x.cos()]], title: "two waves", x_label: "t")
println(str.contains(svg, "<svg"))     // true

let draws = stats.norm.sample(2000, 0.0, 1.0, rng: 3)
let hist = draws.hist(bins: 30, title: "standard normal")
println(str.length(hist) > 500)        // true
```

```olang no-run
// The usual ending: a chart on disk, viewable in any browser.
unwrap(fs.write_file("waves.svg", svg))
```

### A chart's description

Every chart also says what it draws, as data: right after its opening
`<svg>` tag stands `<metadata id="olang-chart">`, a JSON description
(escaped for XML) that an SVG viewer ignores and a host reads to draw
the chart itself (a notebook host hovers, zooms and brushes a chart
from it). `plot.describe(svg)` answers it as a Map, `Err` for text
that carries none; a `viz` chart, compiled to `plot`, carries one too.

| Key | What it says |
|---|---|
| `v`, `type` | `1`; `"xy"` (line, scatter, area, lines, xy), `"bar"` (bar, bars, stacked), `"hist"`, `"heatmap"`, `"box"` |
| `title`, `x_label`, `y_label`, `width`, `height`, `theme` | the options the chart was drawn with |
| `palettes` | `#{ light, dark }`: the series' colours in the order `plot` assigns them in each theme (the chart's own `colors` in both) |
| `x`, `y` | `#{ lo, hi }`: an axis's extent as drawn (its nice ticks' ends); `y` only for bars, boxes and a histogram's counts (its `x` the bins' range) |
| `series` (xy) | `[#{ name, mark, x, y }]`, the points as drawn (a null dropped); a series coloured per point (`plot.xy`'s fifth element, `viz`'s `color_by`) has `pc: true` and its `colors` |
| `labels`, `series`, `layout`, `vary` (bar) | the categories, `[#{ name, values }]`, `"single"`, `"grouped"` or `"stacked"`, and whether one series' bars vary in colour |
| `edges`, `counts` (hist) | the bins' `k + 1` edges and their `k` counts |
| `xl`, `yl`, `rows`, `scale` (heatmap) | the column and row labels, the cells by row, the ramp's name |
| `series` (box) | `[#{ name, min, q1, median, q3, max }]` |
| `plot` | `#{ left, top, w, h }`: where the plot stands in the chart's own units (an `x` extent's `lo` drawn at `left`, its `hi` at `left + w`) — what a page's brush maps the pointer through |

```olang
let d = unwrap(plot.describe(plot.line(ods.series([1, 2, 3]), ods.series([2.0, 4.0, 3.0]), title: "t")))
println(d.type + " " + d.title + " " + to_string(d.series[0].y))   // xy t [2.0, 4.0, 3.0]
```

In the browser the same charts land on the page with
`dom.set_html(el, svg)`; see
[olang in the browser](wasm.md#the-dom-module).

### The `viz` grammar

`use viz` is an embedded olang package where a chart is a *value*: a spec map holding data,
a mark, and column-name encodings. Data is records (a list of maps or objects —
what `dom.fetch_json` or `ods.to_records` delivers) or a Frame;
`color` names a column whose distinct values split the rows into one
series per value; bar marks sum rows sharing a category (`"stack":
true` stacks them); `layers` lists several xy marks over shared
scales. `viz.chart(spec)` compiles a spec to plot SVG — pure and
testable natively — and `viz.draw(canvas, spec)` compiles the same xy
specs to canvas instead, for point counts that would drown a DOM in
SVG nodes. On the canvas target, point and line marks ride
[`dom.draw_points`](wasm.md#graphics-the-draw-list) — one packed
binary buffer per series with the data→pixel affine applied host-side
— and a spec whose data is a **Frame** (with no `color` split) keeps
its columns as Series end to end, so a 50,000-row frame renders with
no per-row olang work:

```olang
use viz
let rows = [
    { day: 1, value: 3.0, kind: "a" }, { day: 2, value: 5.0, kind: "a" },
    { day: 1, value: 2.0, kind: "b" }, { day: 2, value: 6.0, kind: "b" },
]
let svg = viz.chart({ data: rows, mark: "line", x: "day", y: "value", color: "kind" })
println(str.contains(svg, "<svg"))   // true
```

Marks: `line`, `area`, `scatter`/`point` (layerable,
color-splittable), `bar`, `hist` (its values the `y` column, with
`bins`; `color` splits it into grouped bars over shared bins, one group's
counts beside another's), and `box` (one box
per distinct `x`). All plot options (`colors`, `vary`, `scale`,
`theme`, `interactive`, sizes, labels) ride in the spec itself. A
spec's keys are checked before anything renders: `w`/`h` are accepted as
the short forms of `width`/`height`, `font_size` and `font` pass through
to the plot text, and an unknown key is an error naming it.

**Continuous color: `color_by`.** Where `color` splits rows into
discrete series, `"color_by": "column"` maps each point's value onto
a ramp (`"scale"` picks it, `"thermal"` by default). It works on both
targets: the SVG renderer colors each mark individually, and the canvas
target draws at most 24 bulk `draw_points` calls, one per ramp step.

An interactive line carries its own hover surface: one small
`class="vertex"` circle per point, with the same `data-*` datum a
scatter mark carries, so tooltips and mark events work on a line chart
without overlaying a scatter of the same series.

**Interactive charts are event delegation.** With `"interactive":
true`, every mark carries its datum as `data-*` attributes — scatter
points (`s`/`x`/`y`), bars (`s`/`label`/`value`), heatmap cells
(`xl`/`yl`/`value`), boxes (the five-number summary) — and dom events
deliver the target's data map. In the browser, three helpers package
the common moves:
`viz.tooltip(el)` shows the hovered mark's datum in a floating tip;
`viz.on_mark(el, event, handler)` fires the handler with the mark's
data map only when a mark was hit (the click-to-filter primitive);
and `viz.brush(el, handler)` reports a pressed-dragged-released
horizontal range as width fractions, to map onto the data domain and
re-render for brush-to-zoom.

## `gg` — a grammar of graphics

`use gg` builds a chart the way ggplot2 does: data, aesthetic mappings
from columns to x, y, colour and the rest, then layers (geoms with
their stats), scales, facets, coordinates, labels and a theme. A plot
is a value built in steps, each taking the plot first so it pipes.
`gg.svg(p)` renders it to SVG text and `gg.save(p, path)` writes it; a
plot that is a notebook cell's or the REPL's value is drawn by the
editor, with values on hover.

```olang
use gg
let g = random.rng(5)
let f = ods.frame({
    size: g.uniform(120, 40.0, 200.0),
    kind: ods.tile(ods.series(["flat", "house", "loft"]), 40).to_category(levels: ["flat", "loft", "house"]),
})
let homes = f.with({ price: ods.col("size") * 2.5 + g.normal(120, 0.0, 30.0) })
let p = gg.plot(homes, gg.aes(x: "size", y: "price", color: "kind"))
    |> gg.point(alpha: 0.6)
    |> gg.smooth(method: "lm", se: false)
    |> gg.facet_wrap("kind")
    |> gg.scale_y_continuous(labels: "dollar")
    |> gg.labs(title: "Price by size", x: "m²", y: "price")
println(str.starts_with(gg.svg(p), "<svg"))   // true
```

A mapping names a column, holds a Series, or names a variable a stat
computes (`gg.after_stat("density")`). A Category keeps its levels'
order on an axis, in a legend and across facet panels, and text that
reads as ISO dates makes a date axis. The statistics behind the geoms
(densities, smoothers, bins, summaries) run natively. The reference
lists every geom, position, scale and theme:
[`gg` — a grammar of graphics](stdlib.md#gg--a-grammar-of-graphics).

`plot` draws one chart from one call with fixed defaults; `gg` composes
layered, faceted charts.

## The stack and the language

The data stack uses the language's own forms: every verb takes its data
first, so it pipes and chains; results are maps, lists and native values
the rest of the program reads; and `par_map` fans statistical work
across cores as it does any other work.

### Methods, names and fields

Every function is also a method of the values it takes first, every
parameter has a name, and a Frame's columns read as fields
([Methods of native values](language.md#methods-of-native-values)).
The three spellings are one function:

```olang
let sales = ods.read_csv("region,amount\neast,25.5\nwest,320.0\neast,80.0\nwest,12.0\n")
let a = ods.head(ods.sort_by(sales, "amount", true), 2)
let b = sales |> ods.sort_by("amount", descending: true) |> ods.head(2)
let c = sales.sort_by("amount", descending: true).head(2)
println(to_string(a == b && b == c))                          // true
println(to_string(sales.amount.rolling(2, agg: "mean").to_list()))
println(to_string(len(sales)))                                // 4 rows
```

- **Names.** `ods`, `stats`, `plot` and `learn` register their
  parameters, so a call names what it means and skips what it does not:
  `stats.norm.cdf(x, std: 2.0)` takes the mean's default.
  The options a function keeps in a trailing map are names too —
  `learn.split(f, test: 0.25, seed: 7)`, `plot.hist(s, bins: 30,
  title: "draws")` — and a chart's options map may be left off
  altogether. An unknown name raises, listing the ones the function
  takes; `olang check` reports it before the run.
- **Methods.** A Series has the column functions and borrows `stats`
  and `plot` (`s.describe()`, `s.hist()`); a Frame has the table verbs;
  a `learn` fit has the model's (`fit.predict(test)`); an `ods.col`
  expression has a Series' column functions (`col("x").mean().over("g")`,
  `col("y").fill_null("ffill")`). An `impl` for
  `Series` or `Frame` wins over the module's function of the same name.
  The editor completes a value's methods after `s.`.
- **Fields.** `f.amount` is `f["amount"]`. A method's name always means
  the method: a column called `sum` is `f["sum"]`.
- **`len`** counts a Series' values and a Frame's rows.
- **Operators.** `-s` negates a column, `!m` inverts a mask, and `m1 &
  m2`, `m1 | m2`, `m1 ^ m2` combine masks with nulls as unknowns — on
  Series and on expressions alike.
- **`@`** is the dot product of two Series (and a Matrix's product).

A leading `.` continues a chain on the next line, so a pipeline of
methods reads top to bottom
([Source structure](language.md#source-structure)). Independent work fans out with
`par_map`; each task takes its own generator from `random.rng(seed).spawn(k)`,
so the result does not depend on scheduling:

```olang
use ods { col }
let kids = random.rng(7).spawn(4)
let means = par_map(kids, (g) => {
    let f = ods.frame({ x: g.normal(10000, 50.0, 8.0) })
    f.with({ z: (col("x") - col("x").mean()) / col("x").std() })
        .filter(col("z").abs() < 2.0)
        .x
        .mean()
})
println(len(means))   // 4
```

Two complete worked programs extend these patterns to full scale:

- [`examples/data-processing/dataproc/`](../examples/data-processing/dataproc/) — the CSV → Frame →
  `group_by` → JSON pipeline, with the opening section's representation
  comparison shipped as two runnable programs (`main.ol` and
  `records.ol`) and a seeded data generator (`gen.ol`).
- [`examples/data-processing/statlab/`](../examples/data-processing/statlab/) — a full statistical
  study: 10,000 simulated subjects, a Welch t-test checked by a
  permutation test and a bootstrap CI, a three-predictor regression,
  and charts of the results, with a `test` block pinning every
  inference.

---

Next: [the standard library reference](stdlib.md#ods--series-and-frames)
for every function of these modules, or [Writing fast
olang](performance.md#bulk-data-belongs-in-ods) for the measurements.

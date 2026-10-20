# The data stack

Part of [the olang book](README.md) · [A tour of olang](tour.md) ·
[Language reference](language.md) · [Standard library reference](stdlib.md)

The data stack consists of three modules: `ods` (typed columns and tables),
`stats` (statistical inference), and `plot` (charts as SVG text). The three
modules are in scope in every build, including the browser playground, and
require no import. This chapter describes the columnar model the stack is
built on, every Series and Frame operation, a complete inference workflow,
how the stack composes with the rest of the language, its measured
performance, and the reasoning behind its design.

Every plain `olang` block is executed by the test suite; blocks marked
`no-run` are parse-checked only, because they access the file system.

## Table of contents

- [The columnar model](#why-columns)
- [Series — the typed column](#series--the-typed-column)
- [Frames — tables of named columns](#frames--tables-of-named-columns)
- [Expressions — `ods.col`](#expressions--odscol)
- [`stats` — from description to inference](#stats--from-description-to-inference)
- [`plot` — charts as SVG text](#plot--charts-as-svg-text)
- [The stack and the language](#the-stack-and-the-language)
- [Performance characteristics](#performance-characteristics)
- [The design record](#the-design-record)
- [Why eager evaluation](#why-eager-evaluation)
- [What ods is not](#what-ods-is-not)

## Why columns

Consider computing revenue over a CSV of sales. The general-purpose answer is
a list of records — one map per row — and a fold:

```olang
let rows = [
    #{ "region": "east", "amount": 25.5, "qty": 10 },
    #{ "region": "west", "amount": 320.0, "qty": 3 },
]
let total = rows
    |> map((r) => r.amount * to_float(r.qty))
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

The effect is large, and it is reproducible from the repository.
[`examples/data-processing/dataproc/`](../examples/data-processing/dataproc/) contains the same job in both
representations: `records.ol` is the records-and-fold pipeline above, and
`main.ol` is the Frame pipeline this chapter describes. The checked-in data
is a nine-row sample; `gen.ol` generates a seeded input of any size. On a
generated 200,000-row input (`olang gen.ol 200000`), the fold pipeline runs
in about 5 seconds and the Frame pipeline in about 0.06 seconds — a
difference of roughly eighty-fold from the change in representation alone,
in the same binary, with both programs producing identical totals. Because
data work is dominated by representation, the columnar representation is
implemented in the runtime, beneath the language, so that every olang
program uses it without additional code. This is the same relationship that
NumPy has to Python, with the stack built into the runtime rather than
provided as a separate library.

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
`/`, where `7 / 2` is `3`. The scalar rule is the language's; the
column rule is the data stack's, and they differ deliberately: in data
work a ratio between measure columns is the norm (`(gdp - then) /
then`, `deaths / population`), and a truncating quotient is a wrong
answer that looks like a right one — the climate example's growth
rates all read `0` before this rule, with no error and no hint.
`ods.cast(a / b, "Int")` recovers the integer quotient when that is
what is meant. Addition, subtraction, and multiplication of Int Series
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
cast: `ods.cast(s, "String")` turns a number column into text first.

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

Series arithmetic runs IEEE kernels for throughput: a Float Series
can hold `inf` or `NaN` after an overflowing multiply, where the
scalar language raises (language.md, "Floats"). `ods.to_list` hands
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

`ods.map(s, name)` is the older spelling, and still works: `ods.map(s,
"exp")` is `ods.exp(s)` with an Int column widened to Float (it also
knows `fract`, `degrees` and `radians`). Generating a 50,000-point cloud
this way, for [`dom.draw_points`](wasm.md#graphics-the-draw-list),
runs about **240× faster** than the closure form, which is why large
scatter and line art in the gallery is cheap to build.

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
let missing = #{}.absent                          // Unit → null
let s = ods.series([1.0, missing, 3.0])
println(to_string(ods.null_count(s)))             // 1
println(to_string(ods.null_count(s * 2.0)))       // propagates: still 1
println(to_string(ods.mean(s)))                   // skips the null: 2.0
```

Three tools manage them. `ods.null_count(s)` counts, `ods.is_null(s)`
is a mask of the null positions, and `ods.fill_null(s, v)` returns a
copy with every null replaced:

```olang
let missing = #{}.absent
let s = ods.series([1.0, missing, 3.0])
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

This is the honest treatment of missing data: nothing is coerced, and
the one place a default value appears is the place you wrote one.

### Reductions

`ods.sum`, `ods.mean`, `ods.var`, `ods.std`, `ods.min`, `ods.max`,
`ods.median`, and `ods.quantile(s, q)` reduce a column to a number,
skipping nulls. `var` and `std` are the *sample* statistics (the n−1
divisor), and `quantile` interpolates linearly between order
statistics, matching NumPy's default. `median` is defined as
`quantile(s, 0.5)` rather than computed separately, so the two can
never disagree — and neither can the `median` row of `ods.describe`,
which is the same call again. Two running forms complete the set:
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
with nulls; a negative `by` moves them up. Nulls rather than a wrapped
value, because a window has an edge and the row before the first row
does not exist. `ods.diff(s, lag)` is `s - ods.shift(s, lag)` (`lag`
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
minimum, the shape `ods.cumsum` already had.

`ods.rank(s, method)` ranks smallest first. Nulls rank as null — they
are skipped by every other reduction, and giving them a position would
place them somewhere without saying so. The method decides only what
happens to values that tie, and defaults to `"min"`, the competition
ranking that "rank" means unqualified:

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
elements are null, because the window is not yet full and reporting a
partial reduction as a whole one is how a chart lies at its left edge.
Nulls inside a window are skipped, exactly as the whole-column
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
exact either way (its running total is a 128-bit integer). On a
million rows a 365-row mean is 3.8 ms sliding and about 240 ms exact;
pandas takes 3.9 ms.

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

All three go through the pass `ods.group_by` uses to identify keys, so
they cannot disagree with it about what counts as one value. Two
consequences are worth stating because both differ from `==` on the
corresponding scalars: **a null is a value**, forming its own entry
rather than being skipped (the R and Polars convention, and the same
rule `group_by` follows), and **all NaNs are one value**, since keys
are float bit patterns with NaN canonicalized.

### Changing a column's type

`ods.cast(s, type)` converts a column, where `type` is one of
`"Float"`, `"Int"`, `"Bool"`, or `"String"` — the names `ods.schema`
reports. Nulls stay null.

The rule that matters is what happens to a value the target type
cannot hold: it becomes **null**, not an error and not a wrong number.
That is the shape ETL wants — one unparseable row in a million should
not fail a load — and it stays visible, since `ods.null_count` counts
exactly what was lost:

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

One pair is refused rather than guessed at: Float to Bool has no
defensible reading for a value like `0.5`, so it raises and points at
writing the comparison instead (`s != 0.0`).

### Taking a sample

`ods.sample(s, n)` draws `n` rows at random, and works the same way on
a Frame — `ods.sample(f, n)` samples whole rows, keeping the columns
aligned.

Three properties are worth knowing. Sampling is **without
replacement**, so `n` draws are `n` distinct rows; asking for more
rows than exist returns all of them rather than raising, as `head` and
`tail` do. Rows come back in the frame's **original order**, not draw
order, because a sample is meant to be read and shuffling it as a side
effect would make a sample of sorted data unreadable. And randomness
comes from the same stream `random.seed(k)` governs, so a sample is
reproducible exactly when the program says so:

```olang
let f = ods.read_csv("id,v\n1,a\n2,b\n3,c\n4,d\n5,e\n")
random.seed(20260817)
let first = ods.to_list(ods.sample(f, 3)["id"])
random.seed(20260817)
println(to_string(first == ods.to_list(ods.sample(f, 3)["id"])))    // true
```

That stream is shared by the whole process, which is what makes one
`random.seed` govern every sampling verb. It also means a seed does
**not** make sampling reproducible inside `par_map` or `par for`:
several threads drawing from one stream interleave in whatever order
they reach it. Sample before the fan-out, or seed nothing and treat
the result as genuinely random.

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
let names = #{ "N": "North", "s": "South", "S": "South" }
println(to_string(ods.to_list(ods.recode(raw, names))))            // ["North", "South", "South", "x"]
println(to_string(raw.recode(names, default: "Other").to_list()))  // ["North", "South", "South", "Other"]
println(to_string(ods.to_list(ods.recode(ods.series([1, 2]), [[1, 10]]))))   // [10, 2]
```

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

Three constructors, one per source shape. `ods.frame` takes explicit
`[name, column]` pairs (each column a Series or a plain list):

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

Because it takes text, `ods.read_csv("sales.csv")` would once have
parsed the *path* — one column named `sales.csv`, no rows, and no error
anywhere. A single-line argument ending in `.csv`, `.tsv`, or `.txt` is
refused instead, naming [`ods.read_csv_file`](#files-in-files-out) as
the function that was meant. The check reads the string rather than
asking the filesystem whether the path exists, because `read_csv` holds
no `fs` grant — and a function that probes the disk without one is
exactly the leak that keeping these two functions separate prevents.

### Files in, files out

`ods.read_csv_file` reads straight from disk, and `ods.write_csv` writes
a Frame back. Both are the only two points in the whole data stack that
touch the filesystem, so both demand the `fs`
[capability](packages.md#capabilities) — read level to read, write level
to write. Everything else in `ods` is pure and needs no grant, which is
what stops `ods` from becoming a filesystem capability by the back door.

Reading returns a `Result`, because a missing or malformed file is a
failure the caller can handle:

```olang no-run
let sales = unwrap(ods.read_csv_file("sales.csv"))
let by_region = ods.group_by(sales, ["region"], [["total", "sum", "amount"]])
unwrap(ods.write_csv(by_region, "summary.csv"))
```

`ods.to_csv` is the serializer on its own, returning the text rather than
writing it — for a Frame that is going into an HTTP response, a database,
or a pipe rather than a file. It cannot fail, so it returns the string
outright:

```olang
let f = ods.frame_from_records([#{ "k": "a", "v": 1 }, #{ "k": "b", "v": 2 }])
print(ods.to_csv(f))
```

It is the exact inverse of `ods.read_csv`: a null becomes an empty cell,
which is what `read_csv` reads back as null, and a value containing a
comma or a quote is quoted so the column count survives the trip.

And `ods.frame_from_records` takes a list of maps — exactly what
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
by the chunk size rather than the file size: the loop above costs the
same on a one-million-row file as on a two-hundred-thousand-row one.
Nothing else changes — a chunk is a Frame, so every transform, join, and
aggregation in this chapter applies to it unmodified.

The reader ends when a chunk comes back with zero rows. That final empty
chunk still carries the file's columns, so a pipeline written against a
chunk does not need a special case for it. `ods.rows_read(r)` reports how
many rows have been delivered so far and `ods.at_end(r)` whether the file
is exhausted; asking for fewer than one row per chunk is refused, since a
zero-row chunk is the loop's own stopping signal.

A reader is not a value like a Frame. It holds a position in a file —
mutable state — and so it belongs to the thread that opened it. Reaching
one from a `spawn`ed task or sending one down a channel is refused, the
same rule and for the same reason as
[`cell`](language.md#cells-the-one-mutable-location). To
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

The other format a data pipeline meets constantly is JSON lines: one JSON
object per line, which is what log shippers, event queues, and export
jobs emit. `ods.read_jsonl` parses the text and `ods.read_jsonl_file`
reads it from disk. Columns are the union of the keys, and a record
missing one gets a null — the same rule as `frame_from_records`, because
this is that function with a parser in front of it:

```olang
let f = unwrap(ods.read_jsonl("{\"name\": \"ada\", \"score\": 99}\n{\"name\": \"bob\"}\n"))
println(to_string(ods.columns(f)))                            // ["name", "score"]
println(to_string(ods.null_count(f["score"])))    // 1
```

Blank lines are skipped, because a trailing newline is how nearly every
writer finishes the format. A malformed line, or one holding something
other than an object, is an `Err` naming the line number — on a
million-line file that number is the whole diagnostic.

`ods.to_jsonl` serializes a Frame back, one object per row. It omits
nulls rather than writing them, which is what makes the round trip land
on the same Frame: `read_jsonl` turns a missing key into a null, so
writing `null` would be a second spelling of the same thing.
`ods.write_jsonl` writes it to a file and demands `fs` at write level.

`ods.open_jsonl` streams it, and this is where the shape of the reader
earns itself — it is driven by *the same three verbs*:

```olang no-run
let r = unwrap(ods.open_jsonl("events.jsonl"))
let mut total = 0.0
loop {
    let chunk = unwrap(ods.next_chunk(r, 50000))
    if ods.n_rows(chunk) == 0 => break
    total = total + ods.sum(chunk["amount"])
}
```

That is the CSV loop with one word changed. Nothing after the `open_`
names the format, `ods.rows_read` and `ods.at_end` work on either, and a
function that takes a reader takes both. Streaming a 300,000-row JSON
lines file peaks at 30.6MB against 471MB for reading it whole.

JSON lines carry no header, so a reader remembers the columns its earlier
chunks established and hands them back on the empty chunk that ends the
loop — which keeps the promise the CSV reader makes from its header, and
means a pipeline written against a chunk needs no special case for its
last iteration.

### The native format

CSV and JSON lines are interchange with the outside world, and both pay
for it. Every load re-parses text and re-infers types, and neither can
record what a column *was*: a column of zero-padded codes written to CSV
comes back as integers, with the padding gone.

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
That is what a columnar layout on disk buys, and it is measurable: over
500,000 rows and six columns, a CSV load takes 138ms and the same data
from a columnar file takes 50ms — 26ms for one column. The file is also
slightly smaller than the CSV, 15MB against 17MB.

Arrow and Parquet were the alternatives and were declined. Both would be
a dependency, and both are opaque — you cannot look at the file and see
what is in it, which is the wrong trade for a language whose case is that
the artifact should be inspectable. So the header is UTF-8 text, readable
with `head`:

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

`enc` is how a String column is stored. Repetition is the normal case in
a table: a `region` column is a handful of distinct values across a
million rows, and writing each row's text separately costs more in
offsets than the text itself is worth — enough to make the file *larger*
than the CSV it came from. So a repeating column is written once as a
dictionary of its distinct values plus one small code per row. The writer
picks between the two by computing both sizes and taking the smaller,
which means there is no threshold to tune and no case where the choice is
merely a guess.

### Looking at one

Printing a Frame prints a table. This is the first thing an exploratory
session does, so it shows the data rather than describing it — the shape
line, the column names, each column's type, and the rows:

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
under the table. A long Frame keeps both ends and elides the middle,
which is what makes the result of a `sort_by` readable at a glance.

`ods.head(f, n)` takes the first `n` rows, and `n` defaults to 10, so
`ods.head(f)` is the whole gesture. `ods.columns(f)` lists the column
names, and `ods.n_rows` and `ods.n_cols` give the dimensions.

`ods.describe(f)` summarizes every column at once — count, nulls, mean,
standard deviation, and the five-number summary. It returns a *Frame*,
not a map, so it prints as a table and can itself be sorted, filtered, or
written to a file:

```olang
let f = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\n")
println(to_string(ods.describe(f)))
```

A Frame has one type per column, so the numeric statistics are null for
String and Bool columns rather than absent — the alternative is a result
whose *shape* depends on the input, and a caller that has to branch on
the shape of a summary is worse off than one reading nulls.
`ods.schema(f)` is the same thing without the arithmetic — name, type,
and null count per column — for a Frame too wide to summarize
comfortably.

### Reaching a column

Extracting a column and computing on it is the fundamental Frame move —
the table organizes the columns; the Series operations do the work — so
it has syntax:

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\n")
let full = ods.with_column(sales, "revenue", sales["amount"] * sales["qty"])
println(to_string(ods.to_list(full["revenue"])))
```

`f["amount"]` is the column as a Series. A Series takes a position:
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

Three meanings for one subscript, told apart by the key's *type* — a
String selects a column, a list of Strings selects columns, a Bool
Series selects rows. A row *position* is refused outright, naming
`ods.row`, `ods.slice` and `ods.take` instead. That refusal
is the point: pandas spells column selection, row filtering, positional
slicing, and an error all `df[x]`, which is why `.loc` and `.iloc` had to
be invented on top. Here every subscript has exactly one reading.

A column name that is not in the Frame raises rather than returning a
`Result`, and says what the Frame does have. A literal name written into
the source is a claim about the data's shape; when the claim is wrong the
program is wrong, which is the 0.64 rule for misuse.

Subscripting is read-only. There is no `f["a"] = x`: Frames are values,
every verb returns a new one, and an assignment that silently produced a
copy would be a trap. Use `ods.with_column`.

### Combining conditions

A real filter has more than one condition. `&`, `|` and `^` combine
masks elementwise (and, or, exclusive or), and `!` inverts one; `&&`
cannot serve, because the language compiles it to a conditional jump so
that it can short-circuit, which has no elementwise reading over a
column. As in Rust and pandas, `&` and `|` bind tighter than a
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
Series or a plain value — the `==` operator is structural between two
Series, because "are these the same column?" is what `==` answers
everywhere else in the language.

Combination is three-valued, the same logic SQL uses and the same that
`filter` already assumes when it treats a null as false. One `false`
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

`ods.concat(frames)` puts Frames on top of one another. It is what makes
streaming aggregation possible: each chunk produces its own partial
result, and those have to be stacked and re-reduced before the answer is
whole.

```olang
let a = ods.read_csv("x,y\n1,a\n")
let b = ods.read_csv("x,y\n2,b\n")
println(to_string(ods.to_list(ods.concat([a, b])["x"])))     // [1, 2]
```

Columns are matched by *name*, not position, because two Frames that
happen to share a shape but not a meaning is the mistake this is most
likely to be handed. A missing or extra column is refused and named
rather than padded with nulls: a schema that drifted between chunks is a
bug in whatever produced it, and quietly filling the gap would hide that.
An Int column meeting a Float one widens to Float, which is the rule a
literal list already follows.

### Shaping columns and rows

`ods.select(f, names)` keeps named columns; `ods.with_column(f, name,
col)` returns a Frame with a column added or replaced — which,
combined with column arithmetic, is how derived columns happen:

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,80.0,4\n")
let full = ods.with_column(sales, "revenue", sales["amount"] * sales["qty"])
println(to_string(ods.columns(full)))
```

`ods.drop(f, names)` is the complement of `select`: it names the
columns to remove rather than the ones to keep. The distinction matters
when the Frame's shape is not fully known at the point the code is
written — a `drop` survives a new column arriving upstream, where the
equivalent `select` would silently discard it.

`ods.rename(f, mapping)` takes a Map of old name to new name and
renames as many columns as it is given in one pass. It refuses a name
that is not in the Frame, and refuses a target name that already
exists, since two columns of one name could not afterwards be told
apart. Its most common use is the `_right` suffix a colliding join
leaves behind:

```olang
let sales = ods.frame([["id", [1, 2]], ["amount", [10.0, 20.0]]])
let costs = ods.frame([["id", [1, 2]], ["amount", [3.0, 4.0]]])
let j = ods.join(sales, costs, "id") |> ods.rename(#{"amount_right": "cost"})
println(to_string(ods.to_list(j["amount"] - j["cost"])))    // [7.0, 16.0]
```

On the row axis, `ods.filter(f, mask)` keeps rows where a Bool series
is true (build the mask from any column), `ods.take(f, idx)` gathers
rows by index, `ods.head(f, n)` keeps the first `n` and `ods.tail(f, n)`
the last `n` (both defaulting to ten, and both taking a Series as well,
whose first or last `n` values they return), and `ods.sort_by(f, col,
descending)` sorts the whole table by one column, nulls last either way:

```olang
let sales = ods.read_csv("region,amount\neast,25.5\nwest,320.0\neast,80.0\n")
let big = sales |> ods.filter(sales["amount"] > 50.0) |> ods.sort_by("amount", true)
println(to_string(ods.to_list(big["region"])))    // ["west", "east"]
```

Asking `tail` for more rows than the Frame holds returns the whole
Frame rather than an error, which is what makes `ods.tail(f, 1000)` a
usable way to say "the end of this, however long it is".

`ods.slice(f, start, n)` is `n` rows from `start` (a negative start
counts from the end; a Series slices the same way), and `ods.row(f, i)`
is one row as a map of column to value. `ods.rename` takes an object as
well as a map.

`ods.lookup(f, key_col, key, value_col)` reads one value by key: the
`value_col` of the first row whose `key_col` is `key`, or `()` when no
row has it. A list or Series of keys gives a Series, null where a key is
missing. The first lookup builds a hash of `key_col` and keeps it with
the Frame, so a loop of lookups against one table hashes it once (10,000
single lookups against a 50,000-row table: 2 ms, where pandas' `.at`
takes 5):

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
removes duplicate rows, keeping the first occurrence of each — order
is preserved, because a reader scanning the result expects the rows in
the order the data presented them. `ods.drop_null(f)` removes the rows
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

Row identity is computed over the raw column values with each field
length-prefixed rather than joined by a separator, so no value can
impersonate a field boundary: a row of `["a,b", "c"]` and a row of
`["a", "b,c"]` are distinct, and a null is distinct from an empty
string. Note that the CSV reader cannot express that last distinction —
it reads both an empty cell and a quoted `""` as null — so it is
visible only on Frames built in memory.

Frame-level null filling is not a separate verb: `ods.fill_null`
operates on a Series, and combining it with subscript and
`with_column` fills a named column without a second spelling of the
same operation.

```olang
let f = ods.frame([["amount", [1.0, (), 3.0]]])
let filled = ods.with_column(f, "amount", ods.fill_null(f["amount"], 0.0))
println(to_string(ods.to_list(filled["amount"])))    // [1.0, 0.0, 3.0]
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
let by = sales |> ods.group_by("region")
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

`ods.group_by(f, keys, aggs)` with an object of aggregations is
`ods.group_by(f, keys).agg(aggs)`. The older form takes a list of
`[out_name, op, column]` triples, with ops `count` (the group's rows,
nulls included, so the column may be left out), `sum`, `mean`, `min`,
`max`, `median`, `std`, `var`, `first`, `last`, `n_unique`, `any`,
`all`, `argmax` and `argmin`; it is the same engine. `median`, `std` and
`var` are the Series reductions of the same names over each group's
non-null values (sample `std`/`var`, null below two values; all three
Float). `first` and `last` take the group's first and last row's value
as it stands — a null included — on a column of any type, and
`n_unique` counts a group's distinct values the way `ods.n_unique`
does, null as one value. A Bool column sums to its count of true and
averages to its share. Groups keep first-seen order, and a null key
forms its own group (the R and Polars convention) rather than
disappearing:

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,80.0,4\n")
let summary = sales
    |> ods.group_by("region", [["total", "sum", "amount"], ["n", "count"]])
    |> ods.sort_by("total", true)
for rec in ods.to_records(summary) {
    println(`${rec.region}: ${rec.total} over ${rec.n} sales`)
}

let spread = ods.group_by(sales, "region", [
    ["median", "median", "amount"], ["sd", "std", "amount"],
    ["first_qty", "first", "qty"], ["qtys", "n_unique", "qty"]])
println(spread)
```

On a large frame (500,000+ rows) the group ids come from a parallel
dictionary encoding, and the aggregation pass runs across every CPU
core: each thread accumulates `sum`/`count`/`min`/`max` into per-group
partials over a fixed range of rows, and the partials are merged in
chunk order. Integer results are bit-identical to the sequential path;
float `sum` and `mean` reorder their additions, but the order is fixed
by the chunk layout, so a given input always reproduces the same
result. `median`, `quantile`, `std` and `var` gather each group's rows
once (a counting sort by group) and reduce each group on its own, the
groups spread across the cores on a large frame; a group's answer never
depends on the split, so it is identical on one thread or many. `apply`
cuts the Frame into its groups the same way, in parallel on a large one,
and calls the function once per group in group order. At a million rows
and 1,000 groups: five aggregations 3.2 ms (pandas `agg` 13.9), a ratio
of two sums 0.7 ms (6.7), `transform` 0.6 ms (1.9), `apply` 10.9 ms
(42); [Writing fast olang](performance.md#frames-grouping-joins-and-time)
has the rest.

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
    |> ods.with({ event: ods.run_id(col("kind")) })
    |> ods.filter(col("kind").not_null())
    |> ods.group_by("event")
    |> ods.agg({ kind: col("kind").first(), start: col("hour").first(), hours: ods.count(), peak: col("z").abs().max() })
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
that share both. The aggregation is a `group_by` — the same call — so
pivoting and grouping cannot disagree about how a column reduces or what
happens to nulls; what `pivot` adds is the scatter.

```olang
let sales = ods.read_csv(
    "region,quarter,amount\neast,Q1,10.0\neast,Q2,20.0\nwest,Q1,5.0\neast,Q1,3.0\n")
let wide = ods.pivot(sales, "region", "quarter", "amount", "sum")
println(to_string(ods.columns(wide)))          // ["region", "Q1", "Q2"]
println(to_string(ods.to_list(wide["Q1"])))    // [13.0, 5.0] — east's two Q1 rows summed
println(to_string(ods.to_list(wide["Q2"])))    // [20.0, ()] — west never reported Q2
```

The aggregation is required rather than defaulted. When a cell has more
than one row behind it, which reduction applies is the caller's
decision, and choosing one silently is how a wrong number reaches a
report. A cell no row reached is null — the combination did not occur,
which is a different fact from a null value in it — unless `fill:` says
what it is instead (`ods.pivot(sales, "region", "quarter", "amount",
"sum", fill: 0.0)`).

Two shapes are refused rather than guessed at. A null in the `columns`
column cannot name a column, and calling it `"null"` would collide with
a genuine `"null"` string, so it says so and points at `drop_null`. And
a value that would name a column the index already has is refused rather
than silently overwriting it.

`ods.unpivot(f, ids, value_columns)` gathers, the other direction: the
`ids` stay as they are and every other column becomes rows of a
`name`/`value` pair. Omitting `value_columns` takes every column that is
not an id — which is the form that survives a new column arriving
upstream, where naming the value columns would leave it behind.

```olang
let wide = ods.frame([["region", ["east", "west"]], ["Q1", [13.0, 5.0]], ["Q2", [20.0, 1.0]]])
let long = ods.unpivot(wide, "region")
println(to_string(ods.columns(long)))            // ["region", "name", "value"]
println(to_string(ods.to_list(long["name"])))    // ["Q1", "Q1", "Q2", "Q2"]
```

The value columns stack into one column, so they must share a type.
Mixing them would mean choosing a common type on the caller's behalf,
which is `ods.cast`'s job and its explicit decision, so `unpivot`
refuses and names the two columns that disagree.

The round trip is lossless in one direction and explicit in the other:
`unpivot` after `pivot` returns the long form with the combinations that
never occurred present as nulls. Dropping them is then the caller's
call — `ods.drop_null` — rather than a decision the verb made silently.

### Time buckets: `resample`

`ods.resample(f, on, every, aggs)` groups the rows by the start of the
time bucket each falls in and aggregates them, sorted by time: the
`group_by` of a floored time column. `on` is a column of ISO dates or
date-times as text (`2024-03-01`, `2024-03-01T13:45`,
`2024-03-01 13:45:10`); `every` is a count and a unit — `"30s"`,
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
join on weeks or months needs:

```olang
let days = ods.series(["2024-03-14", "2024-03-17", "2024-03-18"])
println(to_string(days.floor("1w").to_list()))     // ["2024-03-11", "2024-03-11", "2024-03-18"]
println(to_string(days.floor("1mo").to_list()))    // ["2024-03-01", "2024-03-01", "2024-03-01"]
```

Each value is parsed once into an integer bucket (no text per row), the
grouping and the sort run on those integers, and only the buckets are
written back as text. A million hourly rows to daily sums and means
take 15 ms; pandas takes 16 on a column already parsed to datetimes and
65 from the same text.

### Joins

`ods.join(a, b, on_a, on_b)` is an inner hash join on one key column
from each side; `ods.join_left` keeps every left row, filling the
right side with nulls where nothing matched. Null keys never match
(the SQL convention), and a column-name collision on the right gains
a `_right` suffix (which `ods.rename` above exists to undo):

```olang
let totals = ods.frame([["region", ["east", "west"]], ["total", [105.5, 320.0]]])
let rates = ods.frame([["name", ["east", "west"]], ["rate", [0.07, 0.09]]])
let joined = ods.join(totals, rates, "region", "name")
let tax = joined["total"] * joined["rate"]
println(to_string(ods.to_list(tax)))
```

Three more kinds complete the set. `ods.join_full(a, b, on)` keeps every
row from both sides, filling the other side's columns with nulls where
nothing matched; its key column takes whichever side has a value, so a
row that came only from the right is still identified by its key rather
than being null in the one column that says what it is. Because a full
join returns rows even when nothing matched, it is the one kind where
mismatched key types would be silently papered over — so it refuses
them, where the other kinds simply find nothing.

`ods.join_semi(a, b, on)` and `ods.join_anti(a, b, on)` ask about
existence rather than combination: semi keeps the rows of `a` that have
a match, anti keeps those that do not, and both return `a`'s columns
alone. The distinction from an inner join is the multiplication — where
`b` has three rows for a key, an inner join returns three rows and a
semi join returns one:

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
  of the same length when the right side calls them otherwise), where a
  single key would have needed a concatenated string column;
- `suffix:` replaces `_right` on a colliding right column;
- `validate:` says which side holds each key once — `"many_to_one"` (the
  right: a lookup table), `"one_to_many"` (the left) or `"one_to_one"`
  (both) — and raises, naming the key, when it does not. A duplicated
  key in a lookup table multiplies the left rows it matches; the check
  turns that silent fan-out into an error at the join that caused it.

```olang
let sales = ods.frame({ region: ["n", "s", "n"], week: [1, 1, 2], units: [5, 7, 9] })
let promos = ods.frame({ region: ["n", "s"], week: [2, 1], discount: [0.1, 0.2] })
let j = ods.join(sales, promos, ["region", "week"], how: "left", validate: "many_to_one")
println(j["discount"].to_list())           // [(), 0.2, 0.1]
let dup = ods.frame({ region: ["n", "n"], week: [2, 2], discount: [0.1, 0.3] })
println(attempt(() => ods.join(sales, dup, ["region", "week"], validate: "many_to_one")))
```

Each side's key columns are encoded against the right side's values (an
integer per row, a column at a time), so a join on several keys costs
little more than a join on one, and no row's key is ever built as text.
A million-row left join on two Int keys against 50,000 rows takes 12 ms
(pandas `merge`: 23).

On a large left frame (50,000+ rows) the join's probe runs across every
CPU core — each left row is looked up independently, and the chunks are
concatenated in row order, so the parallel result is identical to the
sequential one. This is where olang's lack of a GIL shows: the whole join
uses the machine, no ceremony. Small joins stay single-threaded (the
thread hand-off would cost more than it saves).

## Expressions — `ods.col`

`ods.col(name)` is an expression for a column of a Frame not yet named
by a variable: the Frame verbs below resolve it against the Frame they
are given. It is what lets a pipeline say "price per square foot"
without repeating the Frame:

```olang
use ods { col }
let houses = ods.read_csv("hood,price,sqft\nnorth,300000,1500\nsouth,210000,1400\nnorth,520000,2600\n")
let priced = houses
    |> ods.with({ ppsf: col("price") / col("sqft"), big: col("sqft") > 2000 })
    |> ods.filter(col("ppsf") > 160.0)
    |> ods.sort_by(["hood", "ppsf"], [false, true])
println(priced)
```

Everything that takes a column takes an expression: the operators
(`col("a") * 2 + col("b")`, `-col("x")`, `col("a") > 0 & col("b") < 5`),
every function of the sections above, as a method or by the module
(`col("x").exp()`, `col("x").fill_null("ffill")`, `ods.rank(col("x"))`,
`col("x").shift(1)`), and each of those gives an expression back. A
reduction gives a value every row shares: `col("x") - col("x").mean()`
centres a column. `ods.lit(v)` is a
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

The group ids come from the same parallel dictionary encoding as
`ods.group_by`, so the two never disagree about what counts as one
group (a null key is a group of its own).

### How expressions run

An expression is a description until a verb resolves it, so building
one costs nothing and printing one shows what it says
(`to_string(col("a") * 2)` is `(col("a") * 2)`). When a verb resolves
it, the elementwise parts — operators, math, `where`, `coalesce`,
`clip`, `between`, the null masks, casts — compile into one plan with
the rest of the verb's expressions, which runs chunk by chunk and
across cores past the size where that pays. A function that is not
elementwise (a rank, a shift, a reduction, `over`) runs once on its own
column and joins the plan as an input. Errors name the column that is
missing and the columns there are.

## `stats` — from description to inference

Description says what the data at hand looks like; inference asks what
can honestly be concluded from it. The `stats` module covers both, and
every statistic it produces is pinned against scipy reference values
in the test suite. Results come back as ordinary maps, read with
`.` like any other map.

### Describing a column

`stats.describe(s)` is the first look: count, null count, mean,
standard deviation, minimum, quartiles, and maximum in one map.

```olang
let missing = #{}.absent
let s = ods.series([5.1, 4.9, 6.2, 5.7, missing, 5.5])
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
println(to_string(r.p_value > 0.05))   // consistent with uniform
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
random.seed(11)
let n = 2000
let tenure = stats.norm.sample(n, 24.0, 8.0)
let tickets = stats.norm.sample(n, 2.0, 1.0)
let plan = ods.series(map(ods.to_list(stats.norm.sample(n, 0.0, 1.0)), (z) => if z > 0.6 => "pro" else => "basic"))
let eta = ods.to_list(tickets * 1.1 - tenure * 0.08 + 0.5)
let pl = ods.to_list(plan)
let u = ods.to_list(stats.norm.sample(n, 0.0, 1.0))
let churn = ods.series(map(range(0, n), (i) => eta[i] - (if pl[i] == "pro" => 0.8 else => 0.0) + u[i] > 0.0))
let f = ods.frame([["tenure", tenure], ["tickets", tickets], ["plan", plan], ["churn", churn]])

let s = ml.split(f, test: 0.25, stratify: "churn", seed: 3)
let lr = ml.logistic(s.train, y: "churn")
println(lr.params.tickets > 0.0 && lr.params.plan_pro < 0.0)     // true
let p = ml.prob(lr, s.test)
println(ml.auc(s.test["churn"], p) > 0.8)                         // true

let gb = ml.boost(s.train, y: "churn", trees: 200, seed: 3)
println(ml.auc(s.test["churn"], ml.prob(gb, s.test)) > 0.8)       // true
println(ods.n_rows(ml.cross_validate(lr, f, folds: 5)))           // 5
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
  to fit it again.
- **Evaluation refits natively.** `ml.cross_validate(fit, f)`,
  `ml.learning_curve` and `ml.grid_search` take a model or a spec
  (`ml.spec("forest", y: "churn", options: #{ "n_trees": 200 })`) and
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

### Against scikit-learn

Fit times on 80,000 training rows (sklearn's `make_friedman1` with 10
features for regression, `make_classification` with 20 for
classification), and the test score on the other 20,000 rows: RMSE for
regression, AUC for classification. These were measured on an Apple M5
Pro (18 cores) against scikit-learn 1.9 with numpy 2. Each time is the
best of three runs, one for the slowest.

| Model | olang `learn` | scikit-learn | Score (olang / sklearn) |
|---|---:|---:|---|
| OLS, 10 features | 1.6 ms | 5.1 ms (`LinearRegression`) | 2.636 / 2.636 |
| Lasso, alpha 0.01 | 1.5 ms | 1.5 ms | 2.637 / 2.637 |
| Logistic, C = 1 | 17 ms | 19 ms (lbfgs) | 0.9618 / 0.9618 |
| Decision tree, depth 8 (classification) | 188 ms | 673 ms | 0.9627 / 0.9623 |
| Random forest, 100 trees (classification) | 525 ms | 1,416 ms (`n_jobs=-1`), 18,759 ms (one core) | 0.9757 / 0.9753 |
| Random forest, 100 trees (regression) | 1,004 ms | 2,498 ms (`n_jobs=-1`) | 1.205 / 1.206 |
| Boosting, 100 trees of depth 3 (regression) | 168 ms | 13,711 ms (`GradientBoosting`), 186 ms (`HistGradientBoosting`) | 1.259 / 1.307, 1.276 |
| Boosting, 300 trees of 31 leaves (regression) | 459 ms | 2,360 ms (`HistGradientBoosting`) | 1.046 / 1.054 |
| Boosting, 100 trees of depth 3 (classification) | 194 ms | 27,281 ms (`GradientBoosting`), 210 ms (`HistGradientBoosting`) | 0.9732 / 0.9740, 0.9736 |
| Boosting, 300 trees of 31 leaves (classification) | 525 ms | 2,395 ms (`HistGradientBoosting`) | 0.9751 / 0.9753 |
| k-nearest neighbours, k = 5 (fit and predict 20,000) | 349 ms | 235 ms | 0.9742 / 0.9742 |
| k-means, k = 8, 10 starts (100,000 rows) | 599 ms | 902 ms | inertia 62,572 / 62,571 |
| Gaussian naive Bayes | 10 ms | 8 ms | 0.9344 / 0.9344 |
| PCA, 20 columns (100,000 rows) | 3.7 ms | 1.9 ms | the same components |

At 20,000 rows the picture is the same. kNN is at parity there (17 ms
against 17 ms), and the forest takes 64 ms against sklearn's 214 ms.

- **Boosting.** It is the histogram method, so the fair comparison is
  `HistGradientBoosting`. It is about as fast at 100 small trees and
  4–5 times faster at 300 trees of 31 leaves. Against the exact
  `GradientBoosting` it is 80–140 times faster, at equal or better
  test scores.
- **Forests and trees.** Trees grow in parallel, and each split's sort
  is cheap, so forests are 2.5–3 times faster than sklearn on all
  cores.
- **Where sklearn wins.** kNN at 80,000 × 20,000 rows: sklearn's
  brute-force search is a BLAS matrix product, and `learn` is pure
  Rust with no BLAS, so it ships in the browser build. Naive Bayes and
  PCA: both take a few milliseconds either way.

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
| `plot.area(x, y, opts)` | a line with the region beneath it filled |
| `plot.lines(x, pairs, opts)` | several lines with a legend; `pairs` is `[[label, y], ...]` (up to 8) |
| `plot.xy(entries, opts)` | layered marks over shared scales; each entry is `[label, mark, x, y]` with mark `"line"`, `"area"`, or `"scatter"` — the multi-series form (shared axes, one legend) |
| `plot.bar(labels, values, opts)` | one bar per category; `labels` is a Series or list |
| `plot.bars(labels, pairs, opts)` | grouped (side-by-side) bars, one group per category |
| `plot.stacked(labels, pairs, opts)` | stacked bars — non-negative values only (a negative part misleads) |
| `plot.hist(s, bins, opts)` | binned counts of one numeric Series |
| `plot.heatmap(x_labels, y_labels, rows, opts)` | a matrix of values as colored cells on a sequential scale |
| `plot.box(pairs, opts)` | five-number-summary boxes: whiskers to min/max, quartile box, median line |

Options ride in a single map — `title`, `x_label`, `y_label`, `width`,
`height`, `theme`, `paper`, `ink`, `responsive`, `interactive`,
`colors`, `vary`, `scale` — and passing `#{}` accepts the defaults, which follow a
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
error, because it is always a typo.

**Color is a first-class option.** `colors: ["#5aa9e6", ...]` gives a
chart its own palette (series take the list in order, cycling) —
dashboards read best when each card owns a hue. `vary: true` makes a
single-series bar chart color each *category* from the palette —
categorical identity, the classic statistical-graphics look. Heatmaps
pick a ramp with `scale`: `"auto"` (the theme's sequential scale),
`"ocean"`, `"ember"`, `"thermal"`, or `"diverging"` (signed data:
cold through the surface color to warm) — hand-tuned multi-stop
gradients in the spirit of the scientific colormaps. `plot.ramp(name,
t)` exposes the same ramps to olang code: one color for `t` in
[0, 1], which is how `viz`'s continuous color encoding is built. Null handling matches what a chart can honestly draw: xy plots
drop a point when either coordinate is null; the bar family rejects
null values, because a bar has no defined height for a missing value.
Call `fill_null` or `filter` first, so that the decision is explicit;
`box` drops nulls, since a distribution summary honestly tolerates
missing observations.

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

### A chart's description

Every chart also says what it draws, as data: right after its opening
`<svg>` tag stands `<metadata id="olang-chart">`, a JSON description
(escaped for XML) that an SVG viewer ignores and a host reads to draw
the chart itself — olang Studio's notebooks hover, zoom and brush a
chart from it. `plot.describe(svg)` answers it as a Map, `Err` for text
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
let d = unwrap(plot.describe(plot.line(ods.series([1, 2, 3]), ods.series([2.0, 4.0, 3.0]), #{ "title": "t" })))
println(d.type + " " + d.title + " " + to_string(d.series[0].y))   // xy t [2.0, 4.0, 3.0]
```

In the browser the same charts land on the page directly — the whole
stack runs in the wasm build, so `dom.set_html(el, svg)` is the
entire rendering step. The tracker's `/charts.html` draws live
analytics that way (one `dom.fetch_json`, then frames and charts),
and `/gallery.html` is the showcase: computed art and statistical
pieces, every one rendered by `plot`. See
[olang in the browser](wasm.md#the-dom-module).

### The `viz` grammar

A spec's keys are checked before anything renders: `w`/`h` are
accepted as the short forms of `width`/`height`, `font_size` and
`font` pass through to the plot text, and an unknown key is an error
naming it — never a chart quietly drawn at the default size.

One level up from the chart functions sits `use viz` — an embedded
olang package where a chart is a *value*: a spec map holding data,
a mark, and column-name encodings. Data is records (a list of maps —
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
no per-row olang work at all:

```olang
use viz
let rows = [
    #{ "day": 1, "value": 3.0, "kind": "a" }, #{ "day": 2, "value": 5.0, "kind": "a" },
    #{ "day": 1, "value": 2.0, "kind": "b" }, #{ "day": 2, "value": 6.0, "kind": "b" }
]
let svg = viz.chart(#{ "data": rows, "mark": "line",
    "x": "day", "y": "value", "color": "kind" })
println(to_string(str.contains(svg, "<svg")))
```

Marks: `line`, `area`, `scatter`/`point` (layerable,
color-splittable), `bar`, `hist` (its values the `y` column, with
`bins`; `color` splits it into grouped bars over shared bins, one group's
counts beside another's), and `box` (one box
per distinct `x`). All plot options (`colors`, `vary`, `scale`,
`theme`, `interactive`, sizes, labels) ride in the spec itself.

**Continuous color: `color_by`.** Where `color` splits rows into
discrete series, `"color_by": "column"` maps each point's value onto
a ramp (`"scale"` picks it, `"thermal"` by default) — the
`aes(color = value)` of the grammar. It works on both targets: the
SVG renderer colors each mark individually, and the canvas target
buckets the 24 quantized ramp steps into at most 24 bulk
`draw_points` calls, so even a hundred-thousand-point cloud keeps the
binary path.

An interactive line carries its own hover surface: one small
`class="vertex"` circle per point, with the same `data-*` datum a
scatter mark carries, so tooltips and mark events work on a line chart
without overlaying a scatter of the same series.

**Interactive charts are event delegation.** With `"interactive":
true`, every mark carries its datum as `data-*` attributes — scatter
points (`s`/`x`/`y`), bars (`s`/`label`/`value`), heatmap cells
(`xl`/`yl`/`value`), boxes (the five-number summary) — and dom events
deliver the target's data map, so hover and click cost no new
machinery. In the browser, three helpers package the common moves:
`viz.tooltip(el)` shows the hovered mark's datum in a floating tip;
`viz.on_mark(el, event, handler)` fires the handler with the mark's
data map only when a mark was hit (the click-to-filter primitive);
and `viz.brush(el, handler)` reports a pressed-dragged-released
horizontal range as width fractions — map them onto your data domain
and re-render for brush-to-zoom. The tracker's `/charts.html`
cross-filters every card from clicked status bars, and the gallery's
forecast piece zooms by brushing.

## The stack and the language

Nothing in ods is a sublanguage. Frames chain with `|>` because every
verb takes its data first; results come back as maps and lists the
rest of the program consumes; and the parallelism constructs fan
statistical work across cores exactly as they fan any other work.

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
println(`95% CI: [${math.round(ods.quantile(means, 0.025))}, ${math.round(ods.quantile(means, 0.975))}]`)
```

Two complete worked programs extend these patterns to full scale:

- [`examples/data-processing/dataproc/`](../examples/data-processing/dataproc/) — the CSV → Frame →
  `group_by` → JSON pipeline, with the opening section's representation
  comparison shipped as two runnable programs (`main.ol` and
  `records.ol`) and a seeded data generator (`gen.ol`).
- [`examples/data-processing/statlab/`](../examples/data-processing/statlab/) — a full statistical
  study: 10,000 simulated subjects, a Welch t-test cross-validated by
  a 1,000-round permutation test and a bootstrap CI (both fanned over
  `par_map`), a three-predictor regression, and charts of the results,
  with a `test` block pinning every inference.

## Performance characteristics

Every performance figure in this section is produced by a benchmark in the
repository; the methodology, benchmark code, and recorded revisions are in
[the design record below](#the-design-record). The results, on 10M-element
columns against NumPy and 10M-row tables against Polars, are as follows:

| Operation | ods | Reference | Standing |
|---|---|---|---|
| `sum` / `mean` (10M floats) | 1.01 ms (0.41 ms parallel) | NumPy 1.12 ms | parity sequential, ~2.7× ahead parallel |
| `std` | 2.06 ms | NumPy 5.11 ms | ~2.5× ahead |
| `a * b + 1.0` elementwise | 2.42 ms parallel | NumPy 3.24 ms | ahead as executed; chains now fuse into one pass (E7, below) |
| `sort` | 24.1 ms parallel | NumPy 324.3 ms | far ahead |
| null-aware `mean` (10% nulls) | 4.71 ms | NumPy `nanmean` 7.84 ms | ~1.7× ahead |
| OLS fit, 1M rows × 20 predictors | 36.4 ms parallel | NumPy `lstsq` 137.8 ms | ~3.8× ahead |
| group-by, 10M rows, 1k groups | 27.2 ms (one thread) | Polars 24.0 ms (18 threads) | within 1.13× |
| group-by after parallel dictionary encoding (E9) | 5 ms in the pipeline below | Polars 8 ms | ahead as executed |

Large columns parallelize automatically — the same pool as `par_map`,
switched off with `set_parallel(false)` and sized by
`OLANG_PARALLELISM` — with no change to any result. Each kernel class
fans out only past the size where that pays for waking the pool:
500,000 elements for arithmetic between columns, 2M for arithmetic with
a scalar, 1M for a reduction, 100,000 for a sort, 200,000 to 500,000
rows for a join, a filter or a group-by; below them, and inside a
`par_map` stage, a kernel runs on its caller's thread
([measured thresholds](performance.md#automatic-parallelism-in-ods)). The practical
guidance is simpler than the table: keep computations in Series and
Frame operations, cross to lists at the edges, and column size stops
being something to think about. The end-to-end standing is measured by
[`benchmarks/run.sh`](../benchmarks/), a checksum-locked eight-stage
pipeline (load, clean, filter, group, join, sort, aggregate, write)
run identically against pandas and Polars; the full pipeline runs
within ~25% of Polars and about 7× ahead of pandas.

## The pipeline benchmark

The table above measures kernels. This one measures the whole gesture —
an end-to-end ETL pass over a 1M-row CSV, written three times with
identical semantics: in olang on this stack, in pandas, and in Polars
(`benchmarks/` in the repository). Every stage prints a checksum, and
the runner refuses a result where any checksum disagrees across engines
or repetitions, so speed is only ever compared on byte-identical
answers.

Medians of seven runs. Apple M5 Pro (6P+12E, 24 GB), macOS 27.0,
olang 0.78 (post-release engine work) versus pandas 3.0.5 and
Polars 1.44.1 on CPython 3.14.5.
Each engine runs as it ships: ods under the language's automatic
parallelism policy, pandas single-threaded, Polars on its default
thread pool. Times are per-stage, measured inside each process, so
startup is excluded equally.

| Stage | ods | pandas | Polars |
|---|---|---|---|
| load (1M-row CSV) | 10 ms | 174 ms | 7 ms |
| clean (drop nulls, derive) | 12 ms | 54 ms | 6 ms |
| filter | 9 ms | 9 ms | 5 ms |
| group (2 keys, 3 aggs) | 4 ms | 50 ms | 8 ms |
| join (dimension table) | 0 ms | 1 ms | 1 ms |
| sort | 0 ms | 0 ms | 0 ms |
| daily (group, sort, rolling 7) | 2 ms | 21 ms | 6 ms |
| write CSV | 0 ms | 1 ms | 1 ms |
| **whole pipeline** | **38 ms** | **310 ms** | **34 ms** |

Read plainly: on this workload ods is ahead of pandas end to end
(8.2×) and within 12% of Polars — with the group and daily stages
ahead of it — a decade of columnar engineering
with SIMD kernels throughout. The CSV reader is *fused*: one scan per
record-aligned chunk both finds delimiters (NEON block classification
on aarch64) and parses each field into its column's speculative typed
builder while the bytes are hot in cache; string columns keep spans
into the file body itself. A finding worth its sentence: most of what
looked like a parallel-scaling ceiling here was a sequential
quote-aware record-boundary pass running before the parallel scan —
the unquoted path now finds boundaries with O(workers) newline probes,
and the "ceiling" vanished.

The first publication of this table (0.71.0) showed 223 ms, and the
per-stage columns were read as the to-do list they are. Each of the
three worst rows has since closed most of its gap:

- `drop_null` asks each column's validity bitmap instead of
  materializing a value per cell, so a column with no nulls drops out
  of the check entirely: clean 69 ms → 20 ms.
- Frame `filter` runs one column per core, the way `take` already did:
  39 ms → 13 ms.
- An unquoted file now parses into *borrowed slices of the original
  text*. Only a column that really is text allocates; a numeric column
  is parsed straight out of the file and never becomes a `String` at
  all, where every cell used to be allocated before anything knew its
  type: load 76 ms → 52 ms. A quote anywhere, or a row whose field
  count disagrees with the header, falls through to the general parser
  unchanged — quoting rules and the line numbers in parse errors are
  its business, not the fast path's.

What remains in `load` is the half that is genuinely text: three of
this file's six columns are strings, and each cell is still an owned
`String` because that is what `Series::Str` holds. Interning them into
one arena with offsets — how Polars stores strings — is a change to
the `Series` representation, recorded here as the next piece of work
rather than attempted in passing.

To reproduce: `benchmarks/run.sh [reps] [rows]` — the dataset is
generated deterministically, the engine scripts are stage-for-stage
identical, and the runner enforces answer equality before reporting a
single number.

## The design record

This section records the design of the stack: the problem it was built to
solve, the decisions that shaped it, and the measurements that gated each
phase. Every performance claim in this chapter corresponds to a benchmark in
the repository; claims that were not measured are not made.

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

The stack's performance comes from the following techniques, listed in order
of impact. Each relies on an existing crate or compiler capability where one
is available:

| Lever | Buys | How |
|---|---|---|
| Contiguous typed buffers | 10–100× over boxed lists | The representation above; this is most of the win |
| LLVM autovectorization | SIMD for free | Kernels are tight branch-free loops over `&[f64]` |
| Explicit SIMD (`pulp`) | AVX/NEON where autovec fails | Only for kernels that measurably need it |
| Rayon | Near-linear scaling on large arrays | Kernels split above their class's measured threshold (`olang_ods::ParClass`) |
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
- **The representation comparison** at the top of this chapter is
  B-series discipline applied end-to-end, and it ships as runnable code:
  `examples/data-processing/dataproc` contains both pipelines (`records.ol` and
  `main.ol`) and a seeded generator (`gen.ol`). At the 0.40 rewrite the
  measured change was 3.0 s → 0.06 s on 200,000 rows; re-measured on the
  0.60 binary with the shipped programs, it is about 5 s → 0.06 s.

Deferrals recorded with reopening conditions: **faer-backed linear
algebra** (the in-crate Cholesky is textbook-correct for
regression-sized systems; the big decompositions — SVD, PCA, QR —
arrive with `faer` when a real demand creates them), **SIMD numeric
parsing in the CSV reader** (the reader is fused, delimiter scanning
is NEON-classified on aarch64, and the file reads directly into the
shared `Arc<str>` body with zero copies anywhere; load is 10 ms to
Polars' 7 on the 1M-row table, and what remains is vectorized digit
parsing and finer-grained merge scatter — reopen if a workload's load
stage, not its compute, is the bottleneck), and **lazy evaluation**,
next.

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

**Why this did not lead to lazy evaluation.** Two considerations bound the
benefit. First, the eager engine is already competitive on this benchmark:
parallel ods runs it about 1.3 times faster than NumPy, which pays the same
two-pass cost because it does not fuse either. Second, the potential gain is
limited to one saved memory stream per intermediate — roughly a third of the
memory traffic for a two-operation chain — so the improvement is
incremental, not order-of-magnitude.

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

**Amended — Campaign 8 (E7) ships a fourth option the list above did
not enumerate: invisible chain fusion at the operator seam.** An
operator chain like `a * b + 1.0` no longer materializes per operator;
each `+`/`-`/`*` on fusable operands builds a small deferred
expression, and the first observation of the result — an index, a
reduction, a display, any verb — evaluates the whole chain in one
chunked pass, intermediates living in cache instead of main memory.
This is none of the rejected designs: there is no `lazy()`/`collect()`
surface, no query plan, no second evaluation model — a user cannot
tell it exists except by the clock. That invisibility is guaranteed by
keeping the fused shape provably identical to the eager kernels:
Add/Sub/Mul only (infallible IEEE ops, each output element computing
the same operations in the same order — bit-identical), null-free
Float leaves of equal length, Float scalars. Everything outside that
shape — division (which must error at its own expression, and does),
null-bearing series, mixed dtypes, Int scalars — takes the eager path
unchanged, and chains deeper than 16 operators materialize as they
grow. Measured on the flagship shape (a four-operator chain over 5M
elements): 2.7× over the eager engine, identical checksum.

**Amended again — the expression engine (2026-10).** The shape the
fusion is proven on grew to everything elementwise: nulls (validity
combined a 64-bit word at a time), Int columns (checked, as the eager
kernels check), every math function, `where`/`select`/`coalesce`/
`clip`/`between`, the null masks and the numeric casts, and reductions
that follow an expression. The invisibility rule is kept by a simpler
argument than "only infallible operations": an operation that can
raise is evaluated where it is written, fused with what came before
it, so its error is raised there; only what cannot raise is deferred.
`ods.col` expressions are the same engine reached by name, and a
Frame verb's expressions compile into one plan. The measurements are
in [performance.md](performance.md#the-expression-engine).

For everything else, the record stands: the eager engine is simple,
measured, and ahead. Boring is a feature.

## What ods is not

The stack's boundaries are chosen, not pending. Knowing them tells
you when to reach for a different tool:

- **Not N-dimensional.** Series are 1-D and Frames are 2-D, which
  covers statistics. General N-D broadcasting is the largest
  complexity tax in array systems and serves a different domain (deep
  learning) — out of scope entirely, as is GPU execution.
- **Not a dtype zoo.** Int, Float, Bool, String. Dates, categoricals,
  and narrower floats wait for a concrete demand, not a checklist.
- **Not lazy.** No query optimizer, no `collect()`, no second
  evaluation model. The one internal exception is invisible by
  construction: elementwise expressions defer just long enough to
  evaluate in one fused pass, with results and errors identical to the
  eager kernels — the reasoning and boundaries are recorded in
  [Why eager evaluation](#why-eager-evaluation) above. An `ods.col`
  expression is not a lazy Frame: it is resolved by the verb it is
  given to, immediately.
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
[olang in the browser](wasm.md) for the same stack running as
WebAssembly.

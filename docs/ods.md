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

### Elementwise math: `ods.map`

Arithmetic covers `+ - * /`; `ods.map(series, name)` covers the rest —
the whole `math.*` family applied across a column in one native pass.
The second argument is a function *name* (a String, not a closure)
precisely so the loop stays in the kernel: `ods.map(xs, "sin")` is the
vectorized form of `map(xs, (v) => math.sin(v))` with no per-element
boundary crossing. Results are bit-identical to the scalar function
(the kernel calls the same `f64` methods), nulls propagate, and
domain-restricted functions (`sqrt`, `ln`, `asin`, …) raise the same
error the scalar form would. Compose it with arithmetic for a full
vectorized expression — one kernel per term:

```olang
let t = ods.linspace(0.0, 6.28, 8)
let wave = ods.map(t, "sin") * 2.0 + 1.0          // sin(t)*2 + 1, native
println(to_string(ods.mean(wave)))
```

The functions: `sin cos tan asin acos atan sinh cosh tanh exp exp2 ln
log2 log10 sqrt cbrt floor ceil round trunc fract abs degrees
radians` — all `f64 → f64`, producing a Float series (Int series
widen). Because a Series feeds
[`dom.draw_points`](wasm.md#graphics-the-draw-list) directly, generating
a 50,000-point cloud this way runs entirely in native code — about
**240× faster** than the closure form, and the whole reason large
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
println(to_string(ods.mean(s)))                   // skips the null: 2.0
```

Three tools manage them. `ods.null_count(s)` counts, `ods.is_null(s)`
is a mask of the null positions, and `ods.fill_null(s, v)` returns a
copy with every null replaced:

```olang
let missing = map_get(#{}, "absent")
let s = ods.series([1.0, missing, 3.0])
let filled = ods.fill_null(s, 0.0)
println(to_string(ods.to_list(filled)))           // [1.0, 0.0, 3.0]
println(to_string(ods.to_list(ods.is_null(s))))   // [false, true, false]
```

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

### Windows

Four verbs look at a column's neighbourhood rather than the whole of it.

`ods.shift(s, by)` moves values down the column, filling what is vacated
with nulls; a negative `by` moves them up. Nulls rather than a wrapped
value, because a window has an edge and the row before the first row
does not exist. That makes a difference fall out of the operators
already there, which is why there is no separate `diff` verb:

```olang
let s = ods.series([4, 1, 7, 1, 9])
println(to_string(ods.to_list(ods.shift(s, 1))))       // [(), 4, 1, 7, 1]
println(to_string(ods.to_list(s - ods.shift(s, 1))))   // [(), -3, 6, -6, 8]
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

`ods.rolling(s, window, agg)` is a trailing-window aggregate: element
`i` reduces the `window` elements ending at `i`, with `agg` one of
`count`, `sum`, `mean`, `min`, `max`. The first `window - 1` elements
are null, because the window is not yet full and reporting a partial
reduction as a whole one is how a chart lies at its left edge. Nulls
inside a window are skipped, exactly as the whole-column reductions skip
them, so a window holding two valid values of three gives the mean of
two:

```olang
let s = ods.series([1.0, 2.0, 3.0, 4.0, 5.0])
println(to_string(ods.to_list(ods.rolling(s, 3, "mean"))))    // [(), (), 2.0, 3.0, 4.0]
println(to_string(ods.to_list(ods.rolling(s, 3, "max"))))     // [(), (), 3.0, 4.0, 5.0]
```

`rolling` computes each window afresh, so it costs one reduction per
element — O(n × window). The incremental alternative, which keeps a
running total and subtracts the element leaving the window, accumulates
float drift a fresh sum does not, and would make a rolling mean disagree
with the `mean` of the same window. Correctness first; if a wide window
over a long column shows up in a measurement, that is the point to
revisit it.

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

### Back to language values

`ods.to_list(s)` converts a Series back to an ordinary list (nulls
become `()`), `ods.get(s, i)` reads one element — a negative index
counts from the end — and `ods.len(s)` is the length:

```olang
let s = ods.series([10, 20, 30])
println(`${ods.get(s, 0)} ${ods.get(s, -1)}`)
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
from the sorted union of keys and missing keys becoming nulls:

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
same rule and for the same reason as [`cell`](concurrency.md#cell). To
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
`s[0]` is its first element and `s[-1]` its last, the same rule lists and
strings follow, so a column reads like the list it stands in for.

A Frame also takes a mask, which is how filtering reads:

```olang
let sales = ods.read_csv("region,amount\neast,25.5\nwest,320.0\neast,80.0\n")
let big = sales[sales["amount"] > 50.0]
println(to_string(ods.to_list(big["region"])))
```

Two meanings for one subscript, told apart by the key's *type* — a
String selects a column, a Bool Series selects rows. A row *position* is
refused outright, naming `ods.head` and `ods.take` instead. That refusal
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

A real filter has more than one condition, and `&&` cannot serve: the
language compiles it to a conditional jump so that it can short-circuit,
which has no elementwise reading over a column. `ods.all_of` and
`ods.any_of` take a *list* of masks, because a filter usually has three
or four conditions rather than two, and `ods.not` inverts one:

```olang
let f = ods.read_csv("quality,kwh\nok,5.0\nfail,7.0\nok,-1.0\nok,9.0\n")
let usable = f[ods.all_of([ods.eq(f["quality"], "ok"), f["kwh"] > 0.0])]
println(to_string(ods.to_list(usable["kwh"])))               // [5.0, 9.0]
```

`ods.eq` and `ods.ne` are equality as a *mask*, taking either another
Series or a plain value — the `==` operator is structural between two
Series, because "are these the same column?" is what `==` answers
everywhere else in the language.

Combination is three-valued, the same logic SQL uses and the same that
`filter` already assumes when it treats a null as false. One `false`
settles an `all_of` even when another entry is unknown, and one `true`
settles an `any_of`; otherwise an unknown propagates. A comparison
against a null is itself unknown, which is why the mask above does not
need a null check in front of it.

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
the last `n` (both defaulting to ten), and `ods.sort_by(f, col,
descending)` sorts the whole table by one column, nulls last either way:

```olang
let sales = ods.read_csv("region,amount\neast,25.5\nwest,320.0\neast,80.0\n")
let big = sales |> ods.filter(sales["amount"] > 50.0) |> ods.sort_by("amount", true)
println(to_string(ods.to_list(big["region"])))    // ["west", "east"]
```

Asking `tail` for more rows than the Frame holds returns the whole
Frame rather than an error, which is what makes `ods.tail(f, 1000)` a
usable way to say "the end of this, however long it is".

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
    println(`${map_get(rec, "region")}: ${map_get(rec, "total")} over ${map_get(rec, "n")} sales`)
}
```

On a large frame (100,000+ rows with under a few million groups) the
aggregation pass runs across every CPU core: each thread accumulates
`sum`/`count`/`min`/`max` into per-group partials over a fixed range of
rows, and the partials are merged in chunk order. Integer results are
bit-identical to the sequential path; float `sum` and `mean` reorder
their additions, but the order is fixed by the chunk layout, so a given
input always reproduces the same result. The group-id hashing pass — the
larger cost — is still sequential; parallelizing it is the next step.

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
which is a different fact from a null value in it.

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

Every kind follows the same null rule: a null key matches nothing. So a
null-keyed left row appears in a left, full, or anti join and never in
an inner or semi one, and semi and anti always partition the left frame
between them.

On a large left frame (50,000+ rows) the join's probe runs across every
CPU core — each left row is looked up independently, and the chunks are
concatenated in row order, so the parallel result is identical to the
sequential one. This is where olang's lack of a GIL shows: the whole join
uses the machine, no ceremony. Small joins stay single-threaded (the
thread hand-off would cost more than it saves).

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
println(`n=${map_get(d, "count")} nulls=${map_get(d, "null_count")} median=${map_get(d, "median")}`)
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
println(`p = ${map_get(t, "p_value")}`)
println(`means: ${map_get(t, "mean_a")} vs ${map_get(t, "mean_b")}`)
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
println(`intercept ≈ ${math.round(ods.get(coef, 0))}`)   // true value: 3
println(`b1 ≈ ${math.round(ods.get(coef, 1))}`)          // true value: 2
println(`b2 ≈ ${math.round(ods.get(coef, 2))}`)          // true value: -1
println(`r2 = ${map_get(fit, "r2") > 0.5}`)
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
| `plot.area(x, y, opts)` | a line with the region beneath it filled |
| `plot.lines(x, pairs, opts)` | several lines with a legend; `pairs` is `[[label, y], ...]` (up to 8) |
| `plot.xy(entries, opts)` | layered marks over shared scales; each entry is `[label, mark, x, y]` with mark `"line"`, `"area"`, or `"scatter"` |
| `plot.bar(labels, values, opts)` | one bar per category; `labels` is a Series or list |
| `plot.bars(labels, pairs, opts)` | grouped (side-by-side) bars, one group per category |
| `plot.stacked(labels, pairs, opts)` | stacked bars — non-negative values only (a negative part misleads) |
| `plot.hist(s, bins, opts)` | binned counts of one numeric Series |
| `plot.heatmap(x_labels, y_labels, rows, opts)` | a matrix of values as colored cells on a sequential scale |
| `plot.box(pairs, opts)` | five-number-summary boxes: whiskers to min/max, quartile box, median line |

Options ride in a single map — `title`, `x_label`, `y_label`, `width`,
`height`, `theme`, `responsive`, `interactive`, `colors`, `vary`,
`scale` — and passing `#{}` accepts the defaults, which follow a
colorblind-validated ten-hue palette assigned in fixed series order.
`theme: "dark"` re-tunes every color for a dark surface (mint leads);
`responsive: true` drops the fixed pixel size so the SVG fills its
container (the browser case — the viewBox keeps the aspect ratio). An
unknown option key is an error, because it is always a typo.

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

In the browser the same charts land on the page directly — the whole
stack runs in the wasm build, so `dom.set_html(el, svg)` is the
entire rendering step. The tracker's `/charts.html` draws live
analytics that way (one `dom.fetch_json`, then frames and charts),
and `/gallery.html` is the showcase: computed art and statistical
pieces, every one rendered by `plot`. See
[olang in the Browser](wasm.md#the-dom-module).

### The `viz` grammar

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
color-splittable), `bar`, `hist` (with `bins`), and `box` (one box
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

Reductions on large columns parallelize automatically under the
language's standard policy — the same machinery as `par_map`, tuned by
`set_parallel(n)` — with no change to any result. The practical
guidance is simpler than the table: keep computations in Series and
Frame operations, cross to lists at the edges, and column size stops
being something to think about.

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
  construction: elementwise operator chains defer just long enough to
  evaluate in one fused pass, with results and errors identical to the
  eager kernels — the reasoning and boundaries are recorded in
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

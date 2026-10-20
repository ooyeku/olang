# Writing fast olang

Part of [the olang book](README.md) ·
[The execution model: OVM and JIT](ovm.md) ·
[Command-line tooling](tooling.md) ·
[The data stack](ods.md)

olang's tiers require no configuration: a plain `olang program.ol` runs
the interpreter, promotes hot functions to the bytecode VM, and compiles
qualifying code to native machine code. Every tier reproduces the
interpreter's results exactly or declines. What the tiers *cannot* do is
rewrite a program into a shape they optimize well. This chapter states
the shapes that reach the fast path, how to see which tier is running
your code, and how to verify the result — knowledge drawn from building
the repository's own benchmarks and data workstreams, where each idiom
below made a measured difference of between 2× and several hundred times.

## Table of contents

- [Put hot loops in functions](#put-hot-loops-in-functions)
- [The rebind forms](#the-rebind-forms)
- [Homogeneous lists are typed lists](#homogeneous-lists-are-typed-lists)
- [What declines the fast path](#what-declines-the-fast-path)
- [Reading the tier's decisions](#reading-the-tiers-decisions)
- [Verifying a hot program](#verifying-a-hot-program)
- [Bulk data belongs in ods](#bulk-data-belongs-in-ods)
- [Automatic parallelism in ods](#automatic-parallelism-in-ods)
- [The expression engine](#the-expression-engine)
- [What a program costs in memory](#what-a-program-costs-in-memory)

## Put hot loops in functions

Function bodies are the unit of promotion and compilation. A hot loop at
the top level of a script is promoted mid-flight with more limited
machinery than the same loop inside a function, where whole-function
compilation and on-stack replacement both apply. The transformation is
mechanical:

```olang no-run
// Slower: top-level loop.
let mut total = 0.0
for i in 0..n { total = total + values[i] }

// Faster: the same loop inside a function.
fn sum_values(values, n) = {
    let mut total = 0.0
    for i in 0..n { total = total + values[i] }
    total
}
let total = sum_values(values, n)
```

Moving the repository's n-body benchmark loop from the top level into a
function changed nothing about the algorithm and made it 24× faster,
because the loop nest became an on-stack-replacement region compiled as
a whole. When a script has one hot section, wrap that section in a
function and call it once.

## The rebind forms

olang values have value semantics: updating a list or map produces a new
value, and the runtime makes the update cheap by mutating in place
whenever the value is unshared. The compiler recognizes this in one
specific spelling — the **rebind form**, where the updated variable is
reassigned from an operation whose first argument names it:

```olang
fn build() = {
    let mut xs = [0.0, 0.0, 0.0]
    xs = col.set(xs, 1, 2.5)      // write one element in place
    let mut s = ""
    s = s + "ab"                  // extend the string in place
    let mut m = #{}
    m = map_set(m, "k", 1)        // insert into the map in place
    let mut acc = []
    acc = acc + [xs[1]]           // append one element in place
    `${xs[1]} ${s} ${m.k} ${acc[0]}`
}
println(build())
```

Each of these compiles to a fused instruction that takes the value out
of its variable, mutates it when nothing else holds a reference, and
copies it when something does — so aliases are never affected, and the
observable semantics are identical to the copying reading. Breaking the
form breaks the fusion silently: `let ys = col.set(xs, i, v)` (a new
name) or `m2 = map_set(m, k, v)` must copy, and a loop of such copies is
O(n) per operation instead of O(1). The same applies to reading the
variable in a position the fusion cannot see; keep updates in the shape
`x = op(x, ...)`.

Two spellings that are not literally the rebind form are read as it,
because they are what people write and mean exactly the same thing:

| Written | Read as |
|---|---|
| `let next = out + [x]; out = next` — a temporary that is never read again, rebound at once | `out = out + [x]` |
| a function or lambda whose result is one of its parameters extended: `(m, k) => map_set(m, k, v)`, `(acc, x) => if keep(x) => acc + [x] else => acc`, the same through a `match` | `m = map_set(m, k, v)` as its final expression, which answers the value assigned |

The second is every reducer: `fold` hands its accumulator over rather
than sharing it, so a `fold` that builds a list or a map extends it in
place — 20,000 keys through `fold` and `map_set` took 3.5 s in 0.86.0
and take 5 ms, which is what the loop took. A temporary that *is* read
again holds a second reference, and a prepend (`out = [x] + out`) moves
every element; neither can happen in place, and `olang check` says so
where the loop is written (class `copy` of
[`[check] promote`](tooling.md#project-rules)).

One ordering rule follows from the semantics: the arguments after the
first are evaluated *before* the value is taken, so
`m = map_set(m, k, map_get(m, k) + 1)` reads the live map and then
updates it, exactly as the interpreter does.

## Homogeneous lists are typed lists

A list whose elements are all `Float` or all `Int` is carried as a
packed native vector (`Vec<f64>` / `Vec<i64>`) and crosses into compiled
code as a raw pointer read by direct indexing. Loops over such lists —
indexing, iteration, `col.set` writes, appends — run at native speed. A
single element of another type demotes the list to the general boxed
representation, correctly but slowly. The practical rules:

- Keep numeric columns homogeneous. `[1, 2.5]` is a mixed list; convert
  once with `to_float` at construction rather than mixing.
- Accumulators that start as `[]` and receive only floats or only
  integers take the typed layout from the first append.
- A list of float lists (a feature matrix, `cols[j][i]`) is also
  recognized and read natively.

Domain-checked math (`math.sqrt`, `math.ln`, and the other constrained
functions) compiles into these loops behind a guard: in-domain inputs
run native, an out-of-domain input falls back and raises the standard
error.

## What declines the fast path

The tiers refuse rather than approximate. The refusals most often met in
practice:

- **Allocation per iteration.** A loop that builds a struct, map, or
  multi-element list on every pass stays on bytecode by design; hoist
  construction out of the loop, or accumulate with the rebind forms.
- **Bridged builtins in the hot path.** The native set covers the
  collection core, the higher-order loops, `math`, and `str`; other
  builtins convert their arguments at the tier boundary per call. Maps
  and long lists cross as wrappers in O(1); a short list of records or a
  struct converts one level per call. Move such calls out of inner
  loops.
- **What the tier refused.** A function the bytecode compiler refuses
  runs on the tree-walker, correctly and several times slower, and its
  callers compile around it (they call it through the bridge). `olang
  check --tier` lists every refusal with its reason before the program
  runs; `OLANG_TIER_STATS=1` prints them as `tier-refused:` lines after
  it; in the browser they are `window.olangProfile.report().refused`.
- **Shadowed builtins.** Defining a function named after a builtin (or
  binding one to a local) forces dynamic resolution and disqualifies the
  fused forms for that name, as correctness requires.
- **Effectful constructs** — `spawn`, global assignment, and method
  calls on effectful receiver expressions — are interpreter-owned. See
  [Known limitations](ovm.md#known-limitations) for the complete list.

## Reading the tier's decisions

The engine reports its decisions rather than leaving them to guesswork.

- `olang profile program.ol` attributes time to functions *per tier*, so
  a hot function still running interpreted is visible directly.
- The REPL's `:ovm` report shows promotion counts, native compilation,
  on-stack-replacement entries, and rejection reasons for the session.
- `OLANG_TIER_STATS=1` prints machine-readable counters on exit.
- `OLANG_JIT_DEBUG=1` and `OLANG_OSR_DEBUG=1` print each compilation and
  region decision, naming the instruction or register that caused a
  refusal — the fastest route from "this loop is slow" to "this is the
  line that disqualifies it".

The diagnostic variables are development aids; the
[tooling chapter](tooling.md#environment-variables) lists them all.

## Verifying a hot program

Optimized paths carry proof obligations, and two are available at run
time. `--verify-tiers <rate>` re-executes a sample of native results on
the bytecode VM and compares bit for bit — a sampled rate of a few
percent provides continuous spot-checking for a few percent of overhead
([tooling](tooling.md#--verify-tiers--live-tier-verification)). And any
observable difference between `olang program.ol` and
`olang --no-ovm program.ol` is by definition an engine bug worth
reporting, never a behavior to code around.

## Bulk data belongs in ods

The idioms above make scalar loops fast; they do not make olang a
vector engine. For column-scale numeric work — millions of rows,
aggregation, joins, sorting — the [data stack](ods.md) executes
operations in parallel native code and holds its own measured standings
against NumPy, pandas, and Polars. The division of labor is simple:
express per-element logic in olang functions, keep bulk transformation
in `Series` and `Frame` operations, and cross between the two at the
edges rather than inside loops.

### Methods and names cost nothing

`s.rank(method: "min")`, `ods.rank(s, method: "min")` and `ods.rank(s,
"min")` are one call. The bytecode compiler lays a named call out as the
positional one at compile time, and a method resolves through one table
built at startup (two hashes of short strings). Per call, on a
three-element Series so that the dispatch is all there is to measure
(300,000 calls in a function, best of two runs, Apple M5 Pro):

| Call | Bytecode tier | Interpreter (`--no-ovm`) |
|---|---:|---:|
| `ods.len(s)` | 113 ns | 320 ns |
| `s.len()` | 117 ns | 180 ns |
| `ods.rank(s, "min")` | 290 ns | 490 ns |
| `ods.rank(s, method: "min")` | 290 ns | 507 ns |
| `s.rank(method: "min")` | 353 ns | 427 ns |

A method is as cheap as the module call on the bytecode tier and cheaper
in the interpreter (it calls the function by name rather than reading it
from the module first). A method with names binds them when it runs,
once its receiver says which function it is: one small vector more. On a
column of any size the kernel is the cost.

## Automatic parallelism in ods

A `Series` or `Frame` operation fans out across the thread pool on its
own, but only where that pays. Waking the pool's workers and joining
them costs tens to a couple of hundred microseconds whatever the work,
so each kernel class has a threshold below which it runs on its
caller's thread (`olang_ods::ParClass`, and the frame kernels' own):

| Kernel class | Parallel from | Sequential at the threshold |
|---|---|---|
| Arithmetic between columns (`a * b`, a fused chain over two or more columns) | 500,000 elements | ~0.1 ms |
| Arithmetic with a scalar (`s + 1.5`, a chain over one column) | 2,000,000 | ~0.23 ms |
| Reductions (`sum`, `mean`, `var`, `std`, `dot`, a regression's Gram matrix) | 1,000,000 | ~0.09 ms (`sum`) |
| Sorts (`sort`, `argsort`, `sort_by`) | 100,000 | ~1 ms |
| `join` probe | 200,000 rows | ~1.7 ms |
| `group_by` | 500,000 rows | ~1.2 ms |
| `filter` and `take` across columns | 500,000 rows | ~1.5 ms |

A kernel already running on a pool worker (inside a `par_map` stage)
never fans out again, and with `OLANG_PARALLELISM=1` nothing does.
`set_parallel(false)` turns all of it off; the host's own threshold
(the REPL's `:parallel threshold`) applies on top. String kernels are
always sequential; comparisons, math and the rest of the elementwise
work follow [the expression engine](#the-expression-engine)'s cost model.

The thresholds are read off `benches/crossover.rs` in olang-ods, which
times every kernel with its sequential path and its parallel path
forced, across sizes (`cargo bench -p olang-ods --bench crossover`).
Measured on an Apple M5 Pro (6P+12E, 18 threads, 24 GB), macOS 27.0;
microseconds per call, sequential / parallel:

| Kernel | 50,000 | 250,000 | 500,000 | 1M | 10M |
|---|---|---|---|---|---|
| `mul` | 9.3 / 7.3 | 48.4 / 48.2 | 95.8 / 76.9 | 194 / 120 | 1,943 / 1,033 |
| `a * b + 1.0` fused | 15.5 / 15.7 | 77.5 / 73.3 | 157 / 123 | 317 / 203 | 3,166 / 1,648 |
| `s + 1.5` | 6.4 / 12.4 | 32.4 / 61.8 | 51.4 / 92.5 | 99.1 / 150 | 1,288 / 686 |
| `sum` | 4.1 / 4.2 | 20.8 / 53.0 | 41.7 / 54.5 | 85.7 / 48.1 | 954 / 365 |
| `sort` | 388 / 282 | 2,463 / 727 | 5,352 / 1,365 | 11,249 / 2,507 | 126,423 / 23,939 |
| `group_by`, 1,000 groups | 145 / 145 | 620 / 619 | 1,193 / 1,029 | 2,417 / 1,393 | 22,441 / 7,960 |
| `join` against 1,000 rows | 444 / 444 | 2,160 / 1,682 | 4,706 / 2,956 | 9,641 / 5,600 | 94,150 / 66,053 |

In 1.0.0-rc.2 every Series kernel went parallel at the language's
list threshold (10,000 elements), and arithmetic called its operator
through a function pointer at every element. At 15,000 elements the
parallel `ods.mean(s * t)` took ten times the sequential one, and a
notebook built of medium columns ran several times slower on all cores
than on one. `benchmarks/ods_parallel.ol` measures the operations as a
program calls them (milliseconds per call; the machine's 18 threads,
then `OLANG_PARALLELISM=1`):

| Operation | n | rc.2, all threads | rc.2, one | now, all threads | now, one |
|---|---|---|---|---|---|
| `ods.mean(s * t)` | 15,000 | 0.150 | 0.015 | 0.004 | 0.005 |
| `ods.mean(s / t)` | 15,000 | 0.201 | 0.023 | 0.005 | 0.005 |
| `ods.dot(s, t)` | 15,000 | 0.175 | 0.015 | 0.011 | 0.011 |
| `ods.mean(s * t)` | 50,000 | 0.198 | 0.034 | 0.014 | 0.014 |
| `ods.mean(s / t)` | 50,000 | 0.297 | 0.069 | 0.016 | 0.016 |
| `ods.mean(s * t)` | 1M | 0.420 | 0.536 | 0.252 | 0.281 |
| `ods.mean(s / t)` | 1M | 0.771 | 1.234 | 0.329 | 0.357 |
| `group_by` | 1M | 1.40 | 5.27 | 1.33 | 4.22 |
| `ods.mean(s * t)` | 10M | 2.16 | 5.49 | 1.53 | 3.01 |
| `ods.mean(s / t)` | 10M | 4.12 | 12.02 | 2.48 | 3.98 |
| `ods.len(s + 1.5)` | 10M | 1.40 | 4.02 | 0.70 | 1.31 |
| `group_by` | 10M | 8.95 | 49.05 | 8.10 | 39.33 |

The repository's notebooks under `olmd/`, `olang test` wall time
(fastest of three):

| Notebook | rc.2, all threads | rc.2, one | now, all threads | now, one |
|---|---|---|---|---|
| churn | 2.22 s (18.1 s system) | 0.53 s | 0.35 s | 0.35 s |
| segments | 0.78 s | 0.78 s | 0.63 s | 0.63 s |
| energy | 0.20 s | 0.21 s | 0.21 s | 0.21 s |
| housing, workspace, experiment, sales | unchanged (0.01–0.16 s) | | | |

`learn`'s models parallelize by work rather than by rows. The runtime's
list threshold (10,000 elements under `olang run`) is about olang
closures, so the model kernels ignore it and use their own measured
thresholds:
- trees of a forest: always, one stream per tree;
- histogram building: 64,000 cells;
- gradient and loss passes, and logistic regression's elementwise
  steps: 32,768 rows, summed in fixed pieces so the result is the same
  on any thread count;
- a Gram matrix: 50,000 rows × columns;
- k-means assignment: a million row × centre × feature operations;
- kNN queries: 2 million query × training pairs.

The comparison with scikit-learn is in
[the data stack](ods.md#against-scikit-learn).

## The expression engine

Elementwise column work — operators, the math functions, `where`,
`select`, `coalesce`, `fill_null` with a value, `clip`, `between`, the
null masks, the numeric casts, and `ods.col` expressions — runs through
one engine (`olang_ods::fuse`). An expression compiles into a list of
steps, common subexpressions once, and runs 2,048 rows at a time, so
every intermediate stays in L1 and only the inputs and outputs touch
memory. Nulls are combined a 64-bit word at a time. A reduction after
an expression (`ods.sum(a * b)`, `ods.mean(mask)`, `ods.dot`) folds the
chunks and makes no column. A single operation takes the eager kernel's
own path, so it costs what it did.

Measured with `cargo bench -p olang-ods --bench expr` (the engine) and
`olang-ods/benches/expr.py` (the same shapes in NumPy 2.5.1 and pandas
3.0.5), 10M rows, 10% nulls where marked, 1,000 groups; milliseconds,
fastest of seven, Apple M5 Pro (18 threads). "Eager" is the kernel per
operation, materializing each intermediate, as ods did before.

| Kernel | ods, one thread | ods, parallel | eager, one thread | NumPy / pandas |
|---|---|---|---|---|
| `a * b` | 1.79 | 0.98 | 1.81 | 1.96 |
| `a * 2.5` | 1.23 | 0.61 | 1.24 | 1.28 |
| `a * b`, nulls | 1.82 | 1.05 | 1.84 | 1.85 (pandas) |
| `exp(a)` | 39.7 | 3.24 | 43.5 (`map`) | 40.4 |
| `sqrt(abs(a))` | 4.29 | 0.94 | | 3.44 |
| `sigmoid(a)` | 43.3 | 3.30 | | 44.3 |
| `ln(abs(a) + 1)` | 18.0 | 1.64 | | 18.4 |
| `(a*b + c)*0.5 - 1` | 5.05 | 1.48 | 6.15 | 5.69 |
| the same, nulls | 5.83 | 1.63 | 6.21 | 6.31 (pandas) |
| `i*3 + j`, Int (checked) | 5.24 | 1.19 | 21.3 | 4.89 (unchecked) |
| `sum(a * b)` | 2.28 | 0.70 | 2.84 | 2.95 |
| `dot(a, b)` | 2.28 | 0.70 | 7.15 | 0.92 (BLAS) |
| `mean((a - b)^2)` (an RMSE) | 3.15 | 0.75 | | 4.15 |
| `mean(a > 0)` | 1.02 | 0.35 | | 4.76 |
| log loss (`sigmoid`, two `ln`, a mean) | 124 | 8.88 | | 179 |
| `where(a > 0, a, b)` | 3.98 | 1.18 | | 4.52 |
| `clip(a, -100, 100)` | 2.28 | 0.73 | | 2.24 |
| `between(a, -100, 100)` | 1.30 | 0.39 | | 3.64 (pandas) |
| `coalesce(a, b)`, nulls | 4.76 | 1.03 | | 7.17 (pandas `fillna`) |
| `fill_null(a, 0.0)`, nulls | 4.22 | 0.65 | 9.49 | 5.79 (pandas) |

The column kernels around the engine, against pandas (they are not
elementwise, so one thread):

| Kernel | ods | NumPy / pandas |
|---|---|---|
| `fill_null(s, "ffill")`, nulls | 8.35 | 10.4 |
| `interpolate(s)`, nulls | 11.7 | 166 |
| `diff(s)` | 3.16 | 5.11 |
| `pct_change(s)` | 5.11 | 8.09 |
| `cumprod(s)` | 6.51 | 19.5 |
| `argmax(s)` | 4.43 | 4.63 |
| `mode(s)`, Int, 1,000 values | 5.77 | 14.9 |
| `searchsorted`, 1,000 probes | 0.06 | 0.46 |
| `sort(s, true)` | 115 (24.8 parallel) | 281 |
| `argsort(s, true)`, stable | 165 (38.8 parallel) | 920 |
| `recode`, 1,000-key text map | 21.3 | 177 (`map`) |
| `over(mean, g)` | 22.3 (7.5 parallel) | 40.9 (`transform`) |
| `over(median, g)` | 95.5 (21.4 parallel) | 126 (`transform`) |
| `sort_by(f, ["g", "a"], [false, true])` | 901 (386 parallel) | 4,498 (`sort_values`, stable) |

The transcendental functions run at NumPy's speed on one thread (both
call a scalar `exp`/`log` per element); checked Int arithmetic pays a
few percent over NumPy's wrapping arithmetic for raising on overflow.
Everything else is at parity or ahead on one thread, and the parallel
column is what a program gets past the threshold.

**When an expression fans out.** A plan's cost is the sum of its
operations, a transcendental function counting twelve and a division,
`where`, `coalesce`, `clip` or `between` two. A plan of one operation
keeps its kernel class's threshold above; a longer one fans out from
`2,000,000 / cost` rows, but never below 65,536 nor above its class's
threshold. So `exp(a)` goes parallel from 166,000 rows, a four-operation
chain over columns from 500,000, and a log loss from 65,536. Measured at
the thresholds (one thread / parallel, milliseconds): `exp` at 150,000
rows 0.58 / 0.19; the four-operation chain at 600,000 rows 0.27 / 0.14;
log loss at 65,536 rows 0.82 / 0.27. A parallel run cuts the rows into
a few pieces a worker (16 to 128 chunks each) and computes what the
sequential run does, bit for bit; a reduction's pieces have a fixed
size per plan, and their partial sums are added in order, so its result
is the same on any number of threads.

## What a program costs in memory

A declared function costs its syntax tree and one table entry — a few
kilobytes for a one-line function. Its closure is the scope it was
declared in, and consecutive top-level declarations share one: the
scope as it stood before the first of the run. Anything else at the top
level between two declarations (a `let`, a `use`, a type) starts a new
run. The builtins and stdlib modules every file starts from are one
map shared by every module and interpreter in the process. The meaning
is unchanged from a snapshot per declaration — a
function sees the siblings declared before it as they were then, a
sibling shadows a builtin of its name, and a later redefinition does not
reach back — because each run keeps a table of its members by position,
consulted after the closure by both tiers.

The table compares 0.85.0 with 0.86.0, measured on an Apple M5 Pro
(6P+12E, 24 GB), macOS 27.0:

| Program | Resident, 0.85.0 | Resident, 0.86.0 | `runtime.memory()` heap |
|---|---|---|---|
| idle (`time.sleep`) | 10 MB | 10 MB | 0.3 MB |
| the same after `runtime.wasm()` | 99 MB | 10 MB | 0.3 MB |
| one file of 4,000 one-line functions | 137 MB | 32 MB | 12 MB |
| the same as an imported module | 140 MB | 39 MB | 12 MB |
| an application of 56 files and three packages, served with 18 http workers | 950 MB | 445 MB | 97 MB |
| twenty thousand closures created in a function, on the interpreter | 2.9 GB of heap | 82 MB | 60 MB |

The served application has since fallen to 330 MB resident with the flat
expression grammar described below.

A value is 40 bytes (176 in 0.85.0): a function, an enum value, a
constructor and a type are each one shared allocation the value points
at, so a list slot and a scope-map entry are a quarter of what they
were. A closure captures the bindings its body can name — not every
binding in scope, which on the interpreter was a three-hundred-entry
map rebuilt per closure — and every closure made from one lambda
expression shares one resolved body.

The difference between resident memory and the heap is memory that was
freed and is still resident, and most of it was the parser's. The
grammar's token queue cost about a hundred bytes per byte of source
while every operand opened and closed a rule per precedence level; a
binary expression is now parsed flat and its tree built by precedence
climbing, which cut the queue by two thirds, and a source of 16 KB or
more is parsed a run of top-level statements at a time, which bounds it
by a chunk (640 KB for a 90 KB file, from 10 MB). Booting the served
application above made 436 allocations of 4 MB or more in 0.86.0 and
makes three — single statements too large to split — and the system
allocator's cache of freed large blocks, which it keeps resident, fell
from 122 MB to 8: the process's physical footprint at boot is 143 MB,
from 303. It does not grow with the life of the process.

[`runtime.memory()`](stdlib.md#runtime--what-this-binary-carries) reports
the heap by share — the loaded program, the running program's values,
each task and http worker — and is cheap enough to serve from a
`/profile` route.

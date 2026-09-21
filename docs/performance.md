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
    `${xs[1]} ${s} ${map_get(m, "k")} ${acc[0]}`
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

| Program | Resident, 0.85.0 | Resident, 0.86.0 | `runtime.memory()` heap |
|---|---|---|---|
| idle (`time.sleep`) | 10 MB | 10 MB | 0.3 MB |
| the same after `runtime.wasm()` | 99 MB | 10 MB | 0.3 MB |
| one file of 4,000 one-line functions | 137 MB | 32 MB | 12 MB |
| the same as an imported module | 140 MB | 39 MB | 12 MB |
| an application of 56 files and three packages, served with 18 http workers | 950 MB | 445 MB | 97 MB |
| twenty thousand closures created in a function, on the interpreter | 2.9 GB of heap | 82 MB | 60 MB |

A value is 40 bytes (176 in 0.85.0): a function, an enum value, a
constructor and a type are each one shared allocation the value points
at, so a list slot and a scope-map entry are a quarter of what they
were. A closure captures the bindings its body can name — not every
binding in scope, which on the interpreter was a three-hundred-entry
map rebuilt per closure — and every closure made from one lambda
expression shares one resolved body.

The difference between resident memory and the heap is memory that was
freed and is still resident. The grammar's token queue costs about a
hundred bytes per byte of source, so a source of 16 KB or more is
parsed a run of top-level statements at a time and the queue is bounded
by a chunk; that lowers the peak. What remains is the system
allocator's own cache of freed large blocks — about 120 MB for the
served application above, which macOS returns under memory pressure and
`MallocLargeCache=0` removes at a third more boot time. It does not
grow with the life of the process.

[`runtime.memory()`](stdlib.md#runtime--what-this-binary-carries) reports
the heap by share — the loaded program, the running program's values,
each task and http worker — and is cheap enough to serve from a
`/profile` route.

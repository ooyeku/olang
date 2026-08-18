# A Tour of olang

This tour teaches olang by building one small program: a report over a
field station's temperature log. Each stop introduces one idea — values,
functions, pattern matching, types, errors, parallelism, the data stack,
testing — and applies it to the same data, so by the end you have seen a
complete, realistic olang program grow from a list of strings into a
statistical report with a chart.

Every code block runs exactly as shown: the test suite executes them all
on every change, so the tour cannot drift from the language.

Part of [the olang book](README.md) ·
[Language](language.md) · [Standard Library](stdlib.md) ·
[Packages](packages.md)

## Installing

Build from source with Cargo:

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
cargo install --path .
```

Two ways to run:

```bash
olang program.ol          # run a file (args after the file reach os.args())
olang                     # start the REPL (:help lists commands)
```

Or run nothing at all: the website's **/playground** runs the whole
language — including everything on this tour — as WebAssembly, sandboxed
in your browser.

Performance is not something you configure. olang executes on three
tiers: a tree-walking interpreter defines the semantics, hot functions
are promoted to a bytecode VM, and hot numeric functions compile further
to native machine code. Each tier either agrees with the interpreter
exactly or declines and falls back — so the fast path can never change
what a program means. You write the program; the runtime decides how to
run it.

## Hello

```olang
println("hello, olang")
```

Newlines end statements — no semicolons needed. `//` starts a comment.

## Values and bindings

Our program starts with data: a log of temperature readings, one line
per measurement, as it might arrive from a file or an HTTP response.
`let` binds names to values; backtick strings interpolate with `${...}`:

```olang
let station = "high-meadow"
let log = [
    "north,2026-08-01,21.5",
    "north,2026-08-02,19.0",
    "north,2026-08-03,23.5",
    "south,2026-08-01,24.0",
    "south,2026-08-02,18.5",
]
println(`${len(log)} readings from ${station}`)
```

Lists, tuples, maps, and objects are all literals:

```olang
let list = [1, 2, 3]                 // ordered, heterogeneous
let pair = (40, 2)                   // fixed shape, positional
let scores = #{ "ada": 99 }          // map: arbitrary string keys
let user = { name: "ada", age: 36 }  // object: fixed fields, dot access

println(to_string(list[0] + pair[0] + pair[1]))
println(`${user.name} scored ${map_get(scores, "ada")}`)
```

### Values never change

This is the first big idea, and everything later builds on it: **olang
values are immutable**. Operations that look like modification return a
new value and leave the original alone:

```olang
let base = [1, 2]
let grown = base + [3]           // a new list
println(to_string(base))         // [1, 2] — untouched
println(to_string(grown))        // [1, 2, 3]

let m = #{ "a": 1 }
let m2 = map_set(m, "b", 2)      // a new map
println(`${map_len(m)} then ${map_len(m2)}`)
```

A binding declared `let mut` can be reassigned; a plain `let` cannot,
and the values they point at cannot be mutated in place either. Programs
therefore
flow data through transformations rather than editing shared state —
which is also, as a later stop shows, exactly what makes olang's
parallelism safe.

```olang
let mut count = 0
count = count + 1        // rebinding the name, not mutating a value
println(to_string(count))
```

## Everything is an expression

`if` chains produce values; blocks yield their last expression. There
is no statement/expression divide to work around — a conditional is
just a value you bind:

```olang
let temp = 23.5
let feel = if temp < 10.0 => "cold"
    else if temp < 20.0 => "mild"
    else => "warm"
println(feel)
```

## Functions and pipelines

Functions are `fn name(args) = expression`; a block `{ ... }` is an
expression too, so multi-statement bodies are `fn f(x) = { ... }`. The
pipeline operator `|>` feeds a value into the next call's first
argument, so a chain of transformations reads top-to-bottom.

Time to extract something from the log. Each line is
`station,day,temp`; splitting on commas and taking the third field
gives the temperature:

```olang
let log = [
    "north,2026-08-01,21.5",
    "north,2026-08-02,19.0",
    "north,2026-08-03,23.5",
    "south,2026-08-01,24.0",
    "south,2026-08-02,18.5",
]

fn temp_of(line) = unwrap(str.parse_float(split(line, ",")[2]))

let warm_days = log
    |> map(temp_of)
    |> filter((t) => t > 20.0)
    |> len

println(`${warm_days} warm readings`)
println(`average ${average(log |> map(temp_of))}`)
```

Functions are first-class: `temp_of` is passed to `map` by name, and
the lambda `(t) => t > 20.0` is a function literal. Most of the
standard library takes its data as the first argument precisely so it
chains with `|>`.

(That `unwrap` trusts the input blindly — a malformed line would abort
the program. The error-handling stop fixes this properly.)

## Pattern matching

`match` tests a value against patterns in order and produces the first
matching arm's value. Patterns destructure as they match, which is what
makes them stronger than a chain of `if`s: the *shape* of the data
picks the arm, and the pieces arrive already named.

A log line has a shape — three comma-separated fields — and `match` on
the split list captures all three at once:

```olang
fn describe(line) = match split(line, ",") {
    [station, day, temp] => station + " measured " + temp + " on " + day,
    _ => "malformed: " + line
}

println(describe("north,2026-08-01,21.5"))
println(describe("not a reading"))
```

Guards, or-patterns, and ranges cover the classification side:

```olang
fn band(t) = match t {
    t if t < 0.0 => "freezing",
    t if t < 18.0 => "cool",
    t if t < 25.0 => "comfortable",
    _ => "hot"
}
println(band(-4.0) + ", " + band(21.5) + ", " + band(31.0))
```

`match` is checked: every value you send it must find an arm (the `_`
wildcard catches the rest). The
[language reference](language.md#pattern-matching) lists every pattern
kind — literals, tuples, lists with `...rest`, structs, enum variants,
ranges, guards.

## Data with shape: structs and enums

Strings-with-commas only carry a program so far. A `struct` declares a
record type with named fields, and construction is validated — a
missing or misspelled field is an error, not a silent `Unit`:

```olang
type Reading = struct { station: String, day: String, temp: Float }

fn parse(line) = {
    let parts = split(line, ",")
    Reading {
        station: parts[0],
        day: parts[1],
        temp: unwrap(str.parse_float(parts[2])),
    }
}

let r = parse("north,2026-08-01,21.5")
println(`${r.station}: ${r.temp}`)
println(typeof(r))   // Reading
```

An `enum` declares a closed set of alternatives, each optionally
carrying data — and `match` is how you take them apart. Enums and
`match` are two halves of one feature: the enum promises which cases
exist, the `match` handles them:

```olang
type Sky = enum { Clear, Overcast, Rain(Float) }   // Rain carries mm

fn describe(sky) = match sky {
    Clear => "clear skies",
    Overcast => "grey",
    Rain(mm) => to_string(mm) + "mm of rain"
}

let week = [Clear, Rain(12.5), Overcast]
println(week |> map(describe) |> join(", "))
```

## Behavior with dispatch: traits

A trait declares methods; `impl Trait for Type` provides them, and a
method call dispatches on the receiver's runtime type. Traits may give
default method bodies that impls can override — the mechanism for "same
operation, per-type behavior":

```olang
type Reading = struct { station: String, temp: Float }
type Gap = struct { station: String }        // a day with no data

trait Report {
    fn line(self) -> String
    fn tagged(self) -> String = "* " + self.line()   // default method
}

impl Report for Reading {
    fn line(self) = `${self.station} at ${self.temp}`
}
impl Report for Gap {
    fn line(self) = self.station + " (no reading)"
    fn tagged(self) = "! " + self.line()             // override
}

for entry in [Reading { station: "north", temp: 21.5 }, Gap { station: "south" }] {
    println(entry.tagged())
}
```

The list holds two different types; each element dispatches to its own
implementation. This is olang's polymorphism — no inheritance, no class
hierarchy, just types opting in to capabilities.

## Errors are values

olang has no exceptions. A function that can fail returns a `Result`:
`Ok(value)` on success, `Err(error)` on failure — and the caller
decides, with ordinary code, what failure means. `str.parse_float` is
such a function, which is why the earlier `unwrap` was a loaded gun:
`unwrap` extracts an `Ok` and *aborts* on `Err`.

The honest version of our parser returns a `Result` itself and uses
`?` to propagate failure upward — `expr?` unwraps an `Ok` or returns
the `Err` to the caller immediately:

```olang
type Reading = struct { station: String, day: String, temp: Float }

fn parse(line) = {
    let parts = split(line, ",")
    if len(parts) != 3 => return Err("malformed line: " + line)
    let t = str.parse_float(parts[2])?      // Err propagates to the caller
    Ok(Reading { station: parts[0], day: parts[1], temp: t })
}

println(to_string(is_ok(parse("north,2026-08-01,21.5"))))
println(to_string(parse("junk")))
println(to_string(parse("north,2026-08-02,warm")))   // bad float: Err
```

Because errors are values, "process what you can, report what you
can't" is a fold over `Result`s — no exception handler, no nesting:

```olang
type Reading = struct { station: String, day: String, temp: Float }
fn parse(line) = {
    let parts = split(line, ",")
    if len(parts) != 3 => return Err("malformed line: " + line)
    let t = str.parse_float(parts[2])?
    Ok(Reading { station: parts[0], day: parts[1], temp: t })
}

let log = ["north,2026-08-01,21.5", "junk", "south,2026-08-01,24.0"]
let readings = log |> map(parse) |> filter(is_ok) |> map(unwrap)
let bad = log |> map(parse) |> filter(is_err) |> len

println(`${len(readings)} parsed, ${bad} rejected`)
```

For consuming a single `Result` there is a whole toolkit — `match` on
`Ok(v)`/`Err(e)`, `unwrap_or(r, default)`, `match` on the two arms —
and `error` declarations give failures structure beyond strings. The
[language reference](language.md#error-handling) covers all of it.

## Parallelism, without a GIL

Suppose each reading needs real computation — a calibration model, a
simulation, anything CPU-heavy. olang fans work across every core with
three constructs, all built on OS threads (there is no global
interpreter lock):

- `spawn expr` starts evaluating on a background thread and returns a
  task handle; `task.join` collects its result.
- `par_map(xs, f)` is `map` fanned across the cores: same arguments,
  same results in the same order.
- `par for x in xs { ... }` is the parallel loop — for per-element
  *effects* rather than values, with an implicit barrier at the end.

```olang
fn calibrate(t) = {              // stand-in for heavy per-reading work
    let mut acc = 0.0
    for i in 0..2000 { acc = acc + t * 0.001 }
    t + acc / 2000.0
}

let temps = [21.5, 19.0, 23.5, 24.0, 18.5]
let calibrated = temps |> par_map(calibrate)     // all cores, order kept
println(to_string(len(calibrated)))

let a = spawn calibrate(21.5)
let b = spawn calibrate(24.0)                    // both run concurrently
println(to_string(task.join(a) < task.join(b)))
```

### The snapshot rule

Here is the one thing about olang concurrency you must internalize.
Every parallel construct — `spawn`, `par_map`, `par for`, and ordinary
closures too — **captures its environment by value**: the worker sees a
snapshot of the bindings as they were when it started, and writing to
them inside the worker updates only the worker's own copy. Watch:

```olang
let factor = 10
let scaled = par_map([1, 2, 3], (n) => n * factor)   // reads the snapshot
println(to_string(sum(scaled)))                       // 60
```

Reading captured values is exactly what you want and costs nothing.
*Writing* one is the trap, and the language closes it: assigning to a
binding captured from an enclosing scope is refused before the program
runs, because the write could only reach the worker's own copy.

```text
cannot assign to 'tally': it is captured from an enclosing scope, and
functions capture by value — the outer 'tally' would not change. Return
the new value, or hold the state in a cell
```

A `par for` body is refused on the same grounds and says so in its own
words, pointing at `par_map` and `chan` instead of a cell — a cell
belongs to the thread that made it, so it cannot cross to a worker.

This is not a limitation to route around; it is the design that makes
"fan it across the cores" a safe, one-word decision. No worker can see
another's writes, so there are no data races, no locks, and no
heisenbugs — the compiler-level guarantee is simply that there is
nothing shared to corrupt. When you want values back, use the construct
that returns them: `par_map` collects results in order, `spawn` hands
you a task handle for `task.join`, and aggregation is a `fold` over what
came back.

A failing worker doesn't kill the batch, either — joining a failed task
yields an `Err` value, handled with the same Result toolkit as any
other failure. The [language reference](language.md#concurrency)
has the full semantics; [`examples/parmap/`](../examples/parmap/)
measures the speedup on real kernels.

## The data stack

Our report wants statistics, and olang builds the tools in: `ods`
(typed columns and tables), `stats` (inference), and `plot` (SVG
charts) — no import, no flag, available in the playground too.

A **Series** is a typed, null-aware column. Operators on it are
vectorized — one native kernel over the whole column instead of an
interpreted loop — and comparisons yield masks for filtering:

```olang
let temps = ods.series([21.5, 19.0, 23.5, 24.0, 18.5])
let fahrenheit = temps * 1.8 + 32.0
println(`mean ${ods.mean(fahrenheit)}F, max ${ods.max(fahrenheit)}F`)

let warm = ods.filter(temps, temps > 20.0)
println(to_string(ods.to_list(warm)))
```

A **Frame** is a table of named Series — and `ods.read_csv` turns our
log, unchanged, into one. Group, aggregate, and the report writes
itself:

```olang
let log = [
    "north,2026-08-01,21.5",
    "north,2026-08-02,19.0",
    "north,2026-08-03,23.5",
    "south,2026-08-01,24.0",
    "south,2026-08-02,18.5",
]
let f = ods.read_csv("station,day,temp\n" + join(log, "\n"))

let summary = f
    |> ods.group_by("station", [["avg", "mean", "temp"], ["n", "count"]])
    |> ods.sort_by("avg", true)

for rec in ods.to_records(summary) {
    println(`${map_get(rec, "station")}: ${map_get(rec, "avg")}`)
}
```

From there, `stats.t_test` asks whether the two stations really differ,
`stats.lm` fits a trend line, and `plot.line` renders the chart — as a
complete SVG document in a string, ready to write to disk or serve over
HTTP:

```olang
let north = ods.series([21.5, 19.0, 23.5])
let south = ods.series([24.0, 18.5, 22.0])
let t = stats.t_test(north, south)
println(`p = ${map_get(t, "p_value") < 1.0}`)

let x = ods.series([1.0, 2.0, 3.0])
let svg = plot.line(x, north, #{ "title": "north station" })
println(to_string(str.contains(svg, "<svg")))
```

[The Data Stack](ods.md) is the full chapter — every Series and Frame
verb, the complete `stats` and `plot` modules, and the performance
story; [`examples/statlab/`](../examples/statlab/) is a complete
statistical study built on this stack.

## Testing

Tests live next to the code they guard, in `test "name" { ... }`
blocks, with assertions that abort on failure. Our parser has earned
one:

```olang
type Reading = struct { station: String, day: String, temp: Float }
fn parse(line) = {
    let parts = split(line, ",")
    if len(parts) != 3 => return Err("malformed line: " + line)
    let t = str.parse_float(parts[2])?
    Ok(Reading { station: parts[0], day: parts[1], temp: t })
}

test "parse handles good and bad lines" {
    let r = unwrap(parse("north,2026-08-01,21.5"))
    assert_eq(r.station, "north")
    assert_eq(r.temp, 21.5)
    assert_true(is_err(parse("junk")))
    assert_true(is_err(parse("north,2026-08-02,warm")))
}
println("tests passed")
```

`olang test` discovers and runs every `test` block under a directory,
reports each file's passes and failures, and exits non-zero when
anything fails — it slots straight into CI. See
[Tooling](tooling.md#olang-test).

## The whole program

Every idea from the tour, in one runnable program: parse with `Result`,
keep the good lines, aggregate per station on a Frame, and print the
report:

```olang
type Reading = struct { station: String, day: String, temp: Float }

fn parse(line) = {
    let parts = split(line, ",")
    if len(parts) != 3 => return Err("malformed line: " + line)
    let t = str.parse_float(parts[2])?
    Ok(Reading { station: parts[0], day: parts[1], temp: t })
}

fn band(t) = match t {
    t if t < 18.0 => "cool",
    t if t < 25.0 => "comfortable",
    _ => "hot"
}

let log = [
    "north,2026-08-01,21.5",
    "north,2026-08-02,19.0",
    "north,2026-08-03,23.5",
    "south,2026-08-01,24.0",
    "not,a,reading,at,all",
    "south,2026-08-02,18.5",
]

let readings = log |> map(parse) |> filter(is_ok) |> map(unwrap)
let rejected = len(log) - len(readings)

let rows = readings
    |> map((r) => `${r.station},${r.day},${r.temp}`)
let f = ods.read_csv("station,day,temp\n" + join(rows, "\n"))
let summary = f |> ods.group_by("station", [["avg", "mean", "temp"], ["n", "count"]])

println("=== station report ===")
for rec in ods.to_records(summary) {
    let avg = map_get(rec, "avg")
    println(str.fmt("{}: {} readings, avg {} ({})",
        map_get(rec, "station"), map_get(rec, "n"), avg, band(avg)))
}
println(str.fmt("{} malformed lines rejected", rejected))
```

In a real deployment the log would arrive via `fs.read_file` or
`http.get`, the chart would leave via `fs.write_file`, and the whole
thing might sit behind `http.serve` — the same code, wrapped in I/O.
[`examples/`](../examples/) is full of programs that take that step.

## The playground

The website's **/playground** is the entire language compiled to
WebAssembly: interpreter, bytecode tier, and the full data stack,
running sandboxed in your browser. No code leaves the page. It is the
fastest way to try everything on this tour — and because it is the
same engine, not a lookalike, what works there works installed.

The playground engine also powers olang in the browser as a *frontend*
language: the `dom` module lets an olang program drive a real page —
queries, events, fetch — which is how
[`examples/app/`](../examples/app/) serves an issue tracker whose
frontend is itself written in olang.
[olang in the Browser](wasm.md) tells that story whole, architecture
to application.

## Where to next

- [The Language Reference](language.md) — every construct, precisely.
- [The Standard Library](stdlib.md) — every builtin and module.
- [The Data Stack](ods.md) — Series, Frames, inference, and charts, in depth.
- [olang in the Browser](wasm.md) — the same language as a frontend language.
- [Packages](packages.md) — multi-file programs and dependencies.
- [`examples/`](../examples/) — complete programs: a task CLI, a template
  engine, a regex engine, a parser combinator library, a Lisp interpreter
  written in olang (`minilisp/`), a full-stack issue tracker (`app/`), a
  parallel statistical study on the data stack (`statlab/`), and more.
  Run them all with `cd examples && olang run_all.ol`.

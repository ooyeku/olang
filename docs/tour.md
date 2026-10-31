# A tour of olang

The best way to get a feel for a language is to build something with it.
In this tour we build one small program: a report on a weather station's
temperature log. We start with a few lines of text, and each stop adds
one idea and puts it to work on the same data. By the end you will have
a complete program that parses the log, copes with bad lines, and prints
a summary per station.

Every code block runs exactly as shown, so feel free to paste them into
a file or the REPL as you go.

Part of [the olang book](README.md) ·
[Language](language.md) · [Standard library](stdlib.md) ·
[Packages](packages.md)

## Table of contents

- [Running a program](#running-a-program)
- [Hello](#hello)
- [Values and bindings](#values-and-bindings)
- [Everything is an expression](#everything-is-an-expression)
- [Functions and pipelines](#functions-and-pipelines)
- [Pattern matching](#pattern-matching)
- [Data with shape: structs and enums](#data-with-shape-structs-and-enums)
- [Behavior with dispatch: traits](#behavior-with-dispatch-traits)
- [Errors are values](#errors-are-values)
- [Using every core](#using-every-core)
- [The data stack](#the-data-stack)
- [Testing](#testing)
- [The whole program](#the-whole-program)
- [Where to next](#where-to-next)

## Running a program

Once olang is installed ([Installation](installation.md)), save a program
in a file ending in `.ol` and pass it to `olang`. Run `olang` on its own
and you get the REPL, an interactive prompt where you can type olang and
see the answer straight away:

```bash
olang program.ol          # run a file (arguments after it reach os.args())
olang                     # start the REPL (:help lists commands)
```

## Hello

```olang
println("hello, olang")
```

A new line ends a statement, so there are no semicolons to remember.
`//` starts a comment that runs to the end of the line.

## Values and bindings

Our program starts with data: a temperature log with one reading per
line, the way it might arrive from a file or a web request. `let` gives
a value a name. Strings written in backticks can include values with
`${...}`:

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

olang has four ways to group values, and each is written the way it
looks. A list is an ordered row of anything. A tuple is a small fixed
group whose positions mean something. A map links string keys to values,
and an object has fixed field names you reach with a dot:

```olang
let list = [1, 2, 3]                 // ordered, heterogeneous
let pair = (40, 2)                   // fixed shape, positional
let scores = #{ "ada": 99 }          // map: arbitrary string keys
let user = { name: "ada", age: 36 }  // object: fixed fields, dot access

println(to_string(list[0] + pair[0] + pair[1]))
println(`${user.name} scored ${scores.ada}`)
```

### Values never change

This is the first big idea in olang, and a lot of what comes later rests
on it: once a value exists, it never changes. Anything that looks like
an edit actually makes a new value and leaves the original as it was:

```olang
let base = [1, 2]
let grown = base + [3]           // a new list
println(to_string(base))         // [1, 2] — untouched
println(to_string(grown))        // [1, 2, 3]

let m = #{ "a": 1 }
let m2 = #{ ...m, "b": 2 }       // a new map
println(`${map_len(m)} then ${map_len(m2)}`)
```

If you need a name that points at different values over time, declare
it with `let mut`. You are still not changing a value in place; you are
pointing the name at a new one. In practice this means olang programs
pass data through a series of transformations instead of editing shared
state, and that turns out to be exactly what makes parallel code safe,
as you'll see later in the tour.

```olang
let mut count = 0
count = count + 1        // rebinding the name, not mutating a value
println(to_string(count))
```

## Everything is an expression

In olang, an `if` produces a value, and a block `{ ... }` produces the
value of its last line. So a decision is just something you can give a
name to:

```olang
let temp = 23.5
let feel = if temp < 10.0 => "cold"
    else if temp < 20.0 => "mild"
    else => "warm"
println(feel)
```

## Functions and pipelines

A function is written `fn name(args) = expression`. When the body needs
more than one line, the expression is a block: `fn f(x) = { ... }`. The
pipe operator `|>` passes the value on its left into the call on its
right, as the first argument, so a chain of steps reads from top to
bottom.

Let's pull something useful out of the log. Each line has the form
`station,day,temp`, so splitting it on commas and taking the third piece
gives us the temperature:

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

Functions are values like any other. Here `temp_of` is handed to `map`
by name, and `(t) => t > 20.0` is a small function written on the spot,
often called a lambda. Most of the standard library takes its data as
the first argument so that it chains neatly with `|>`.

One caveat: `unwrap` trusts the input completely, so a malformed line
would stop the program. We'll fix that properly when we get to errors.

## Pattern matching

`match` compares a value against a list of patterns, in order, and gives
back the result of the first one that fits. What makes it more useful
than a chain of `if`s is that a pattern can take the value apart while
it checks it. The shape of the data picks the branch, and the pieces
arrive already named.

A log line has a shape, three fields separated by commas, so a single
pattern on the split line can check that and name all three parts:

```olang
fn describe(line) = match split(line, ",") {
    [station, day, temp] => station + " measured " + temp + " on " + day,
    _ => "malformed: " + line
}

println(describe("north,2026-08-01,21.5"))
println(describe("not a reading"))
```

A guard, written `if` after a pattern, adds a condition. That suits
sorting temperatures into bands:

```olang
fn band(t) = match t {
    t if t < 0.0 => "freezing",
    t if t < 18.0 => "cool",
    t if t < 25.0 => "comfortable",
    _ => "hot"
}
println(band(-4.0) + ", " + band(21.5) + ", " + band(31.0))
```

If a value matches none of the patterns, that is an error when the
program runs, so a `match` usually ends with `_`, which matches
anything. When olang knows the type being matched, `olang check` warns
you about cases you forgot. The [language
reference](language.md#pattern-matching) lists every kind of pattern,
including tuples, lists with a `...rest`, structs, enum variants and
ranges.

## Data with shape: structs and enums

Strings with commas in them only get a program so far. A `struct`
declares a record type with named fields, and olang checks each one when
you build it: a missing or misspelled field is an error on the spot,
instead of turning up later as an empty value.

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

An `enum` declares a fixed set of alternatives, and each one can carry
data of its own. Enums and `match` are made for each other: the enum
says which cases exist, and the `match` handles each of them.

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

Sometimes different kinds of data need to answer the same question in
their own way. A trait names a set of methods, and `impl Trait for Type`
supplies them for one type. When you call a method, olang picks the
version that belongs to the value's type. A trait can also provide a
default version of a method, which a type is free to replace:

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

The list holds two different types, and each one answers with its own
code. That is all there is to polymorphism in olang. There are no
classes and no inheritance; a type simply opts in to the traits it
supports.

## Errors are values

olang doesn't use exceptions for failures you can expect, like a number
that doesn't parse. Instead, a function that can fail returns a
`Result`, which is either `Ok(value)` when things worked or
`Err(error)` when they didn't. The caller decides what a failure means,
using ordinary code. `str.parse_float` is one of these functions, which
is why our earlier `unwrap` was risky: `unwrap` takes the value out of
an `Ok`, and stops the program if it finds an `Err`.

A more honest parser returns a `Result` of its own. Inside it, `?`
after an expression means "take the value if this worked, and otherwise
return the error to my caller right now":

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

Because errors are plain values, "process what you can and count what
you can't" needs no special machinery. You filter the results like any
other list:

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

There are plenty of other ways to handle a single `Result`: a `match`
on `Ok(v)` and `Err(e)`, `unwrap_or(r, default)` to fall back on a
value, and `error` declarations when a plain string isn't enough to
describe what went wrong. The [language
reference](language.md#error-handling) covers them all.

## Using every core

Suppose each reading needed real work, such as running a calibration
model or a simulation. olang can spread that work across all your
processor's cores. It uses real operating-system threads and, unlike
Python, has no global lock that lets only one of them run at a time.
There are three tools for it:

- `spawn expr` starts working on `expr` in the background and hands you
  a task; `task.join` waits for its result.
- `par_map(xs, f)` is `map` spread across the cores. It takes the same
  arguments and gives the same results, in the same order.
- `par for x in xs { ... }` is a parallel loop, for when each item needs
  doing rather than returning a value. It waits for every item to finish
  before moving on.

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
println(to_string(unwrap(task.join(a)) < unwrap(task.join(b))))
```

### The snapshot rule

One rule governs all of this. Every parallel tool, and every ordinary
closure too, captures the values it uses by copying them. A worker sees
a snapshot of the names around it as they were when it started:

```olang
let factor = 10
let scaled = par_map([1, 2, 3], (n) => n * factor)   // reads the snapshot
println(to_string(sum(scaled)))                       // 60
```

Reading from the snapshot is exactly what you want, and it costs
nothing. Writing to it would be a trap, since the change could only ever
reach the worker's own copy. So olang refuses it before the program
even starts, and explains why:

```text
cannot assign to 'tally': it is captured from an enclosing scope, and
functions capture by value — the outer 'tally' would not change. Return
the new value, or hold the state in a cell (`let tally = cell(...)`, then
`cell.set(tally, ...)`)
```

A `par for` body that tries the same thing gets a similar message,
pointing you to `par_map` or a channel instead. (A cell can't help
there, because a cell belongs to the thread that made it.)

This rule is why making a calculation parallel is usually a one-word
change. No worker can see another worker's writes, so there are no data
races and no locks to get wrong. When you want results back, use the
tool that returns them: `par_map` collects them in order, `spawn` gives
you a task to `task.join`, and you can combine what comes back with a
`fold`.

If one worker fails, the rest carry on. Joining a failed task gives you
an `Err`, which you handle like any other error. The [language
reference](language.md#concurrency) has the full details, and
[`examples/concurrency/parmap/`](../examples/concurrency/parmap/)
measures the speedup on real workloads.

## The data stack

Our report needs some statistics, and olang has the tools built in:
`ods` for columns and tables, `stats` for statistical tests, and `plot`
for charts. All three are available without an import.

A Series is a column of values of one type, which may include missing
values. Arithmetic on a Series works on the whole column at once, in
native code, instead of looping in olang. Comparing a Series gives you a
column of true and false values, called a mask, which you can use to
filter. And anything you can do to a Series with a function, you can
also do with a method:

```olang
let temps = ods.series([21.5, 19.0, 23.5, 24.0, 18.5])
let fahrenheit = temps * 1.8 + 32.0
println(`mean ${fahrenheit.mean()}F, max ${fahrenheit.max()}F`)

let warm = temps.filter(temps > 20.0)
println(warm.to_list())                  // [21.5, 23.5, 24.0]
```

A Frame is a table made of named Series. `ods.read_csv` turns our log,
as it is, into one. `group_by` splits the rows into groups, and `agg`
computes one new column per entry, using expressions written with
`col`. As here, each step of a chain can go on a line of its own:

```olang
use ods { col }

let log = [
    "north,2026-08-01,21.5",
    "north,2026-08-02,19.0",
    "north,2026-08-03,23.5",
    "south,2026-08-01,24.0",
    "south,2026-08-02,18.5",
]
let f = ods.read_csv("station,day,temp\n" + join(log, "\n"))

let summary = f
    .group_by("station")
    .agg({ avg: col("temp").mean(), n: ods.count() })
    .sort_by("avg", descending: true)

for rec in summary.to_records() {
    println(`${rec.station}: ${rec.avg}`)
}
```

From there, `stats.t_test` can tell us whether the two stations really
differ or just look different by chance. `plot.line` draws a chart and
returns it as a complete SVG document in a string, ready to save to a
file or send from a web server:

```olang
let north = ods.series([21.5, 19.0, 23.5])
let south = ods.series([24.0, 18.5, 22.0])
let t = stats.t_test(north, south)
println(`difference ${t.diff}, p = ${t.p_value > 0.05}`)

let svg = plot.line(ods.series([1.0, 2.0, 3.0]), north, title: "north station")
println(str.contains(svg, "<svg"))
```

[The data stack](ods.md) has the whole story: every Series and Frame
operation, expressions, matrices, statistics and time series, machine
learning with model formulas, and a grammar of graphics for charts.
[`examples/data-processing/statlab/`](../examples/data-processing/statlab/)
is a complete statistical study built on it.

## Testing

Tests live right next to the code they check, in `test "name" { ... }`
blocks. Inside, assertions stop the test if something is wrong. Our
parser has earned a test of its own:

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

`olang test` finds and runs every `test` block in a folder, reports what
passed and failed in each file, and exits with an error code if anything
failed, so it fits straight into automated checks. See
[Tooling](tooling.md#olang-test).

## The whole program

Here is everything from the tour in one program. It parses each line
into a `Result`, keeps the good ones, groups them by station in a Frame,
and prints the report:

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

let summary = ods.frame_from_records(readings)
    .group_by("station")
    .agg({ avg: ods.col("temp").mean(), n: ods.count() })

println("=== station report ===")
for rec in summary.to_records() {
    println(str.fmt("{}: {} readings, avg {} ({})",
        rec.station, rec.n, rec.avg, band(rec.avg)))
}
println(str.fmt("{} malformed lines rejected", rejected))
```

In real use, the log would come from `fs.read_file` or `http.get`, a
chart might be written out with `fs.write_file`, and the whole thing
could sit behind `http.serve` as a small web service. The core of the
program wouldn't change; it would just be wrapped in input and output.
[`examples/`](../examples/) has plenty of programs that take that step.

## Where to next

- [The language reference](language.md) gives the precise rules for
  every part of the language.
- [The standard library](stdlib.md) documents every built-in function
  and module.
- [The data stack](ods.md) goes deep on Series, Frames, statistics and
  charts.
- [olang in the browser](wasm.md) shows the same language driving a web
  page.
- [Packages](packages.md) covers programs that span several files, and
  dependencies.
- [`examples/`](../examples/) has complete programs to read: a task
  manager for the command line, a template engine, a regex engine, a
  parser combinator library, a Lisp interpreter written in olang
  (`minilisp/`), a full-stack issue tracker (`app/`), a parallel
  statistical study (`statlab/`), and more. Run them all with
  `cd examples && olang run_all.ol`.

# Introduction

olang is a dynamically typed, functional programming language implemented in
Rust. This chapter describes what the language is, the goals that shape its
design, and the kinds of programs it is suited to. It assumes general
programming experience but no prior exposure to olang.

Part of [the olang book](README.md).

## What olang is

An olang program is built from expressions. Functions are values, data is
immutable, and computation is usually expressed as a pipeline that transforms
data through a sequence of small functions. The language provides pattern
matching, algebraic data types, and `Result`-based error handling, and it
includes a substantial standard library: text and collection utilities,
file and network access, an embedded SQL database, a data-analysis stack, and
more.

The following program reads a small dataset, groups it, and reports a summary.
It illustrates the language's typical shape: literal data, a pipeline of
transformations, and pattern matching.

```olang
type Reading = struct { station: String, celsius: Float }

let readings = [
    Reading { station: "north", celsius: 21.5 },
    Reading { station: "north", celsius: 19.0 },
    Reading { station: "south", celsius: 24.0 },
]

let total = readings
    |> map((r) => r.celsius)
    |> fold(0.0, (acc, c) => acc + c)

println(`mean: ${total / to_float(len(readings))}`)
```

## Design goals

olang is organized around a small number of goals. They explain most of the
language's specific choices, and they recur throughout the book.

### Openness

A program's structure, its compiled artifacts, and its execution are all
available to olang programs as data.

- A program's abstract syntax tree is a stable, documented format, returned by
  `meta.parse` as ordinary olang values. Program-analysis tools are written in
  olang.
- A binary produced by `olang build` embeds its own source, a checksum, and
  the capabilities it may use. `olang inspect` reads these back.
- A run can be recorded to a portable trace and replayed exactly with `olang
  record` and `olang replay`.

These properties depend on other design decisions — a stabilized syntax,
immutable values, and a narrow boundary around side effects — and are
described in full in the [Openness](openness.md) chapter.

### Predictable execution

olang runs on three tiers: a tree-walking interpreter, a bytecode virtual
machine, and a native-code JIT. The interpreter defines the language's
semantics. The faster tiers are optimizations: each either reproduces the
interpreter's result exactly or declines to run the function and falls back.
A program therefore means the same thing regardless of which tier executes
it, and performance requires no configuration. This model is described in
[Architecture and internals](internals.md) and
[The execution model: OVM and JIT](ovm.md).

### Safe concurrency

Because values are immutable and closures capture by value, concurrent code
cannot share mutable state through ordinary variables. `spawn` runs a function
on an operating-system thread, and `par_map`, `par_filter`, and `par for`
distribute work across cores without a global interpreter lock. Communication
between threads is explicit, through channels. See the concurrency section of
the [Language reference](language.md).

### Gradual typing

Code without type annotations runs fully dynamically. Annotations are
optional, and each one is enforced at runtime on every tier; the `olang check`
static checker additionally reports violations it can prove before the program
runs. Types can be added to a program incrementally, one function or field at
a time. See [Types and gradual typing](types.md).

## When to use olang

olang is well suited to scripts, data-analysis tasks, teaching, and
experimentation. The standard library and data stack make many such programs
self-contained, and the tooling — a test runner, formatter, static checker,
and single-file executable builder — supports small projects without
additional dependencies.

olang is a young language. Long-running services and projects with many
third-party dependencies should be treated as experimental. The
[Stability and compatibility](stability.md) chapter is the authoritative
statement of what is stable and what is not.

## How to read this book

If you are new to the language, continue with [Installation](installation.md)
and then [A tour of olang](tour.md), which teaches the language by building one
program from start to finish. The [Language reference](language.md) and
[Common pitfalls](pitfalls.md) are useful to keep at hand afterward. The
[book index](README.md) lists suggested reading paths for other goals,
including library use and contributing to the implementation.

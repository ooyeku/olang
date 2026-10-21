# Introduction

olang is a dynamically typed, functional programming language implemented in
Rust. This chapter describes what the language is, what comes with it, and
how this book is organized. It assumes general programming experience but no
prior exposure to olang.

Part of [the olang book](README.md).

## What olang is

An olang program is built from expressions. Functions are values, data is
immutable, and computation is usually written as a pipeline that passes data
through a sequence of small functions. The language has pattern matching,
algebraic data types, traits, and `Result`-based error handling, and it ships
with a large standard library: text and collections, files and processes,
HTTP, an embedded SQL database, a data-analysis stack, a terminal toolkit,
desktop windows, and a browser runtime.

The following program declares a record type, computes a mean with a
pipeline, and classifies it with `match`:

```olang
type Reading = struct { station: String, celsius: Float }

let readings = [
    Reading { station: "north", celsius: 21.5 },
    Reading { station: "north", celsius: 19.0 },
    Reading { station: "south", celsius: 24.0 },
]

fn feel(c) = match c {
    c if c < 18.0 => "cool",
    c if c < 25.0 => "comfortable",
    _ => "hot"
}

let mean = readings |> map((r) => r.celsius) |> average
println(`mean ${mean}, ${feel(mean)}`)   // mean 21.5, comfortable
```

## What comes with it

**Three execution tiers.** A tree-walking interpreter defines the language's
semantics. Hot functions are compiled to a bytecode VM, and hot numeric code
further to native code by a JIT. Each faster tier either reproduces the
interpreter's result exactly or declines and falls back, so a program means
the same thing whichever tier runs it, and nothing needs configuring.
[Writing fast olang](performance.md) describes the program shapes the tiers
optimize; [The execution model](ovm.md) describes the machinery and its
measured performance.

**Concurrency without shared mutable state.** Values are immutable and
closures capture by value. `spawn` runs a function on an operating-system
thread; `par_map`, `par_filter` and `par for` spread work across cores with
no global interpreter lock; channels carry messages between threads. See
[Concurrency](language.md#concurrency).

**Gradual typing.** Code without annotations runs fully dynamically. Each
annotation is enforced at runtime on every tier, and `olang check` reports
the violations it can prove before the program runs, so types can be added
one function or field at a time. See [Types and gradual typing](types.md).

**A built-in data stack.** Typed columns and data frames (`ods`), matrices,
statistical inference and time series (`stats`), machine learning with model
formulas (`learn`, `ml`), and charts (`plot`, and the grammar of graphics
`gg`) run as native code beneath the language. See [The data stack](ods.md).

**Programs as data.** `meta.parse` returns a program's syntax tree as
ordinary olang values, so linters and codemods are olang programs. A binary
built with `olang build` carries its own source, manifest and capability
grant, which `olang inspect` reads back and verifies. `olang record` logs a
run's nondeterministic inputs and `olang replay` reproduces the run exactly.
See [`meta`](stdlib.md#meta--the-program-as-data-the-open-ast),
[The transparent binary](packages.md#the-transparent-binary) and
[Record and replay](tooling.md#olang-record-and-olang-replay--the-open-timeline).

**One toolchain.** `olang` runs, tests, formats, checks, profiles, builds and
documents programs, and serves the language server; `otc` scaffolds projects
and manages dependencies. See [Command-line tooling](tooling.md) and
[Packages and dependencies](packages.md).

The same language runs in the browser as WebAssembly, where an olang program
drives the page ([olang in the browser](wasm.md)), and the
[web SDK](web-sdk.md) builds full-stack applications on that.

## Platforms and stability

olang 1.0 promises compatibility on macOS. Linux and Windows are unsupported
previews. The third-party package ecosystem is small, so programs rely
chiefly on the standard library. [Stability and
compatibility](stability.md) is the authoritative statement of what is
stable, including the [platforms](stability.md#platforms) it covers.

## How to read this book

If you are new to the language, continue with [Installation](installation.md)
and then [A tour of olang](tour.md), which teaches the language by building
one program from start to finish. The [Language reference](language.md) and
[Common pitfalls](pitfalls.md) are useful to keep at hand afterward. The
[book's contents](README.md) list the remaining chapters and suggest reading
paths for other goals.

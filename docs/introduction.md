# Introduction

olang is a programming language for small tools, data work and web apps.
Programs are short and easy to read, they run quickly, and olang keeps
enough of a record of each run that you can play it back later. This
chapter explains what the language is like, what makes it unusual, and
how the rest of the book is laid out. You don't need to know olang
already. Some programming experience helps, but the first sections
assume very little.

Part of [the olang book](README.md).

## What olang is

Here is a small olang program. It takes five daily temperatures, counts
the warm days and works out the average:

```olang
let temps = [21.5, 19.0, 23.5, 24.0, 18.5]
let warm = temps |> filter((t) => t > 20.0)

println(`${len(warm)} of ${len(temps)} days were warm`)
println(`average ${average(temps)}`)
```

```text
3 of 5 days were warm
average 21.3
```

The `|>` is a pipe. It hands the list on its left to the step on its
right, so a longer calculation reads top to bottom as a series of small
steps. That style runs through the whole language. Values never change
once they are made: filtering `temps` gives you a new list and leaves
the old one alone. Functions are values you can pass around, like
numbers or strings.

olang is what programmers call a functional language, and it borrows
the good parts of that tradition: pattern matching to take data apart by
its shape, record and enum types, and errors returned as ordinary values
instead of thrown. It is also dynamically typed, which means you don't
have to write types down. You can add them later, one function at a
time, if you want olang to check your work.

A lot comes in the box. Besides text, lists and files, the standard
library has an HTTP client and server, an embedded SQL database, data
tables with statistics and machine learning, charts, a toolkit for
terminal programs, desktop windows, and a runtime that runs olang in a
web browser. Most programs need nothing else.

## Why you might want it

Plenty of languages are pleasant to write. What sets olang apart is
that it makes programs easier to trust once they exist. Four features
do most of that work.

### Any run can be replayed

Programs that touch the outside world are hard to debug, because the
outside world changes. A random number, the time of day, a file someone
edited, or a slow network reply can each make a bug appear once and
never again.

olang handles this with what we call the Open Timeline. `olang record`
runs your program normally and writes everything it received from
outside (random draws, clock readings, file contents, network replies,
database answers) to a trace file ending in `.olt`. `olang replay` runs
the program again from that trace, and because olang programs are
otherwise fully predictable, the replay is exact. Here is a program that
rolls three dice:

```olang no-run
let rolls = [random.randint(1, 6), random.randint(1, 6), random.randint(1, 6)]
println(`rolled ${rolls}, total ${sum(rolls)}`)
```

Run it twice and you get two different rolls. Record it once, delete
the program, and replay the trace:

```bash
$ olang record dice.ol
rolled [5, 2, 6], total 13
$ rm dice.ol
$ olang replay dice.olt
rolled [5, 2, 6], total 13
olang replay: clean — 3 recorded event(s) reproduced
```

The trace carries the program's source inside it, so it replays on
another machine, next month, with the original file gone. A run that
crashes is recorded too, which means a colleague can send you the trace
of a failure and you can watch it fail as many times as you need. If the
program has changed so that it no longer asks for the same things in the
same order, replay stops at that point and tells you, instead of quietly
producing a different run. [Record and
replay](tooling.md#olang-record-and-olang-replay--the-open-timeline) has
the details.

### Programs ask permission

Most languages let any line of code, including code in a library you
downloaded, read your files or talk to the network. olang lets a project
say what it is allowed to do. These permissions are called capabilities,
and they live in the project's `olang.toml`:

```toml
[capabilities]
fs = "read"     # may read files, but not write them
net = false     # no network
proc = false    # may not start other programs
```

A call that isn't allowed stops the program with a message that names
what was refused and why. You can tighten things further from the
command line with `--deny`, and you can give a single library less than
the rest of the program, so a small text-formatting dependency can be
kept off the network even if a future version of it tries.

You don't have to work the grant out by hand. `olang --trace-caps`
runs a program, watches what it actually used, and prints the tightest
`[capabilities]` block that would have been enough. Code can also ask
what it was granted and choose a different path:

```olang
let plan = if caps.allowed("net") => "fetch fresh prices" else => "use the cached prices"
println(plan)
```

Run normally, that prints `fetch fresh prices`. Run with `olang --deny
net`, it prints `use the cached prices`. The model is settled, but the
exact surface is still marked experimental in 1.0, so details may
shift in a minor release. See [Capabilities](packages.md#capabilities).

### It gets faster on its own

olang runs every program on three engines, which the book calls tiers.
Everything starts on an interpreter, which walks through the program
directly and is the reference for what any program means. Functions that
run often are moved to a bytecode virtual machine, a faster engine that
runs a compact, pre-digested form of the code. The busiest numeric code
goes further still, compiled to native machine code by a JIT (a
"just-in-time" compiler, which compiles while the program runs).

The faster tiers are held to a strict rule: each one either produces
exactly the interpreter's result or declines and hands the work back. So
there is nothing to configure and no "fast mode" that behaves slightly
differently. You write the program once and olang finds the speed where
it safely can.

Capabilities help here too. When a program runs under a fixed grant, the
compiler knows which branches can never be taken and leaves them out, so
sandboxed code can run faster than the same code without the sandbox.
[Writing fast olang](performance.md) shows the program shapes that reach
the fast tiers, and [The execution model](ovm.md) explains the machinery.

### A built program shows its work

`olang build` turns a program into a single executable you can copy to
another Mac and run. That file carries its complete source code, its
`olang.toml` and lockfile, and its capability grant, with a checksum (a
fingerprint of the contents) over all of them. `olang inspect` reads
these back and verifies them, so you can see exactly what a binary
contains, what it is allowed to do, and whether it was built from the
source you think it was. See [The transparent
binary](packages.md#the-transparent-binary).

## What else comes with it

Because values never change, olang can spread work across all your
processor's cores safely. `spawn` runs a function on its own thread,
`par_map` is a `map` that uses every core, and channels pass messages
between threads. Unlike Python, there is no global lock holding threads
back. A closure (a function that remembers values from where it was
made) takes a copy of what it uses, so two threads can never trip over
each other's data. See [Concurrency](language.md#concurrency).

Types are optional and gradual. Code without type annotations runs
dynamically. Each annotation you add is checked while the program runs,
on every tier, and `olang check` reports the mistakes it can prove
before the program runs at all. See [Types and gradual
typing](types.md).

The data stack is built in. `ods` gives you typed columns and data
tables (frames), `stats` covers statistical tests and time series,
`learn` and `ml` do machine learning with model formulas, and `plot` and
`gg` draw charts. All of it runs as native code underneath the language.
See [The data stack](ods.md).

Programs are data, too. `meta.parse` turns olang source into ordinary
olang values, so tools such as linters and code rewriters can be
written in olang itself. See
[`meta`](stdlib.md#meta--the-program-as-data-the-open-ast).

There is one toolchain. The `olang` command runs, tests, formats,
checks, profiles, builds and documents programs, and serves the language
server your editor talks to. Its companion `otc` creates projects and
manages their dependencies, including a personal shelf of libraries you
can use by name from any project. See [Command-line tooling](tooling.md)
and [Packages and dependencies](packages.md).

And the same language runs in the browser as WebAssembly, where an olang
program can drive the page ([olang in the browser](wasm.md)). The [web
SDK](web-sdk.md) builds on that to write a whole web application, server
and browser, in olang.

## Platforms and stability

olang 1.0 makes its compatibility promise on macOS, where every release
is tested. Linux and Windows are previews for now: Linux binaries are
released and pass their tests, and Windows has its own installer and
builds, but hasn't been verified on real machines yet (WSL works today).
The ecosystem of third-party packages is still small, so most programs
lean on the standard library, which is broad for that reason. [Stability and compatibility](stability.md) is the official
statement of what is stable, including the
[platforms](stability.md#platforms) it covers.

## How to read this book

If you are new, go on to [Installation](installation.md) and then [A
tour of olang](tour.md), which teaches the language by building one
small program from start to finish. After that, [Common
pitfalls](pitfalls.md) lists the things that tend to surprise
newcomers, and the [Language reference](language.md) is there for when
you want the precise rules. The [book's contents](README.md) list every
chapter and suggest routes through them for other goals, such as data
analysis or web apps.

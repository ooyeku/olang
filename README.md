<p align="center">
  <img src="branding/banner.svg" alt="olang" width="840">
</p>

# olang

olang is a programming language for small tools, data work and web apps.
You write short, readable programs, and olang runs them quickly. It also
keeps an exact record of what a program did, and lets you decide what
each program is allowed to touch on your computer.

Here is a complete program. It takes four sales, adds them up by region
and prints a table:

```olang
use ods { col }

let sales = ods.frame({
    region: ["east", "west", "east", "west"],
    amount: [25.5, 320.0, 80.0, 40.0],
})
let summary = sales
    .group_by("region")
    .agg({ revenue: col("amount").sum(), orders: ods.count() })
    .sort_by("revenue", descending: true)
println(summary)
```

```text
Frame[2 x 3]
┌────────┬─────────┬────────┐
│ region │ revenue │ orders │
│ String │   Float │    Int │
├────────┼─────────┼────────┤
│ west   │   360.0 │      2 │
│ east   │   105.5 │      2 │
└────────┴─────────┴────────┘
```

There was nothing to install first. Tables, statistics, charts, a
database, a web server and a test runner all come with olang itself.

## What makes it different

You can replay any run. A program that rolls dice gives a different
answer each time, which is exactly what makes a bug in it hard to chase.
`olang record` saves everything the run took in from the outside world
(the random numbers, the clock, files, network replies) into one small
trace file. `olang replay` plays the run back exactly, as often as you
like, even on another machine where the program itself is gone:

```bash
$ olang record dice.ol
rolled [5, 2, 6], total 13
$ rm dice.ol
$ olang replay dice.olt
rolled [5, 2, 6], total 13
olang replay: clean — 3 recorded event(s) reproduced
```

You can also decide what a program may touch. A project can list, in
its `olang.toml`, whether its code may read or write files, use the
network, start other programs or open databases. Once it does, anything
not granted is refused at the moment it is tried, including by a library
you pulled in, so a dependency that suddenly wants the network cannot
quietly have it. `olang --trace-caps` watches a run and writes the
smallest grant that would have been enough.

Programs get faster on their own. Every program starts on a simple
interpreter. Functions that run a lot move to a bytecode virtual machine,
and number-heavy ones are compiled to native machine code. Each faster
tier either gives exactly the interpreter's answer or steps aside, so you
never have to choose a mode or wonder whether optimisation changed your
results. A program running under a strict grant is often faster still,
because the compiler can leave out the code paths it knows are forbidden.

And a built program shows its work. `olang build` turns a program into a
single executable that carries its own source, its dependency list and
its permission grant. `olang inspect` reads all of that back out and
checks it against a checksum, so you can see what a binary contains and
what it is allowed to do before you run it.

The [introduction](docs/introduction.md) walks through each of these
with an example.

## A taste of the language

olang is functional at heart. Values never change once made, functions
are ordinary values, and data usually flows through a pipeline of small
steps. Pattern matching takes data apart by its shape:

```olang
type Shape = enum { Circle(Float), Rect(Float, Float) }

fn area(s) = match s {
    Circle(r) => 3.14159 * r * r,
    Rect(w, h) => w * h
}

let total = [Circle(1.0), Rect(2.0, 3.0), Circle(0.5)]
    |> map(area)
    |> sum

println(`total area: ${str.fixed(total, 2)}`)   // total area: 9.93
```

Types are optional. You can add annotations one function at a time, and
`olang check` reports the mistakes it can prove before the program runs.
Because nothing is shared and mutable, spreading work over every core is
a one-word change: `map` becomes `par_map`.

## Installation

On a Mac or Linux, open a terminal, paste this line and press Return:

```bash
curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh
```

On Windows, run `irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex` in PowerShell.
The installer picks the right download for your computer, checks it
against the release's checksums, and needs no Rust and no administrator
password. Then
quit and reopen your terminal (or run `source ~/.zshrc`, or whichever
file the installer names) and run `olang --version`. With Homebrew,
`brew install ooyeku/olang/olang` works too.
[Installation](docs/installation.md) covers every other way to install,
the platforms olang runs on, and what to do when something goes wrong.
Once it is installed:

```bash
olang script.ol            # run a program; arguments reach os.args()
olang                      # start the REPL
olang test                 # run test blocks under the current directory
olang check                # report provable type violations
otc new myapp              # scaffold a project
```

## Documentation

[The olang book](docs/README.md) is the documentation. If you are new,
read the [introduction](docs/introduction.md) and then
[A tour of olang](docs/tour.md), which builds one small program from
start to finish. After that, the [language reference](docs/language.md),
the [standard library](docs/stdlib.md), [the data stack](docs/ods.md)
and [the command line](docs/tooling.md) are there when you need them.
[Stability and compatibility](docs/stability.md) says what 1.0
promises. Every olang example in the book and in this file is run by
the test suite, so they work as written.

## Examples

The [`examples/`](examples/) folder holds complete programs you can read
and run: command-line tools, a template engine, a regular-expression
engine, a small Lisp written in olang, HTTP services, a full-stack issue
tracker whose front end runs in the browser, data analyses, and a
harbor-operations simulator that runs for a long time
([`concurrency/demo/`](examples/concurrency/demo/)). To run them all,
`cd examples && olang run_all.ol`.

## License

MIT

<p align="center">
  <img src="branding/mascot.svg" alt="Ollie, the olang otter" width="180"><br>
  <sub>Ollie, the olang otter</sub>
</p>

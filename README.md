<p align="center">
  <img src="branding/banner.svg" alt="olang — pipelines, pattern matching, batteries included" width="840">
</p>

A batteries-included dynamic functional language: pipelines, pattern
matching, algebraic data types, immutable values, `Result`-based errors —
with a practical standard library, a source-based package manager, and a
conservative bytecode accelerator that never changes what your program
means.

```olang
type Shape = enum { Circle(Float), Rect(Float, Float) }

fn area(s) = match s {
    Circle(r) => 3.14159 * r * r,
    Rect(w, h) => w * h
}

let total = [Circle(1.0), Rect(2.0, 3.0), Circle(0.5)]
    |> map(area)
    |> fold(0.0, (acc, a) => acc + a)

println(`total area: ${total}`)
```

**[The olang book](docs/README.md)** is the authoritative documentation —
every code block in it (and in this README) is executed by the test suite.
**[Stability](docs/stability.md)** is the authoritative statement of what
is stable, evolving, experimental, and reserved. Where any other text
disagrees with those two, they win.

## Install and run

```bash
cargo install --path .

olang script.ol            # run a program (args reach os.args())
olang                      # REPL
olang test                 # run `test` blocks under the current directory
olang fmt --check .        # formatter (whitespace hygiene, AST-safe)
```

## What's in the language

- **Expressions everywhere** — `if`, `match`, blocks, and loops produce
  values; `break value` and `return` for early exits.
- **Pattern matching** — literals, tuples, lists with `...rest`, structs,
  enum variants, ranges, or-patterns, guards.
- **Algebraic data types** — enums with payload constructors, validated
  struct declarations (shape is checked; values are dynamic), traits with
  runtime dispatch, `error` declarations.
- **Immutability and capture-by-value closures** — values never mutate in
  place; closures snapshot their environment.
- **Errors as values** — `Result`, `?` propagation, `try`/`catch`.
- **Concurrency, honestly labeled** — `spawn` runs on a real OS thread
  (await joins it); `Promise.delay`/`all`/`race` are deterministic,
  deadline-based timing simulation.
- **Modules and packages** — `use`/`share`, plus `olang.toml` packages
  with lockfiles, checksums, a content-addressed cache, and Minimal
  Version Selection ([docs/packages.md](docs/packages.md)).

## The standard library

Sixteen native modules — `str`, `col`, `math`, `json`, `csv`, `re`,
`dates`, `time`, `random`, `crypto`, `base64`, `fs`, `os`, `http` (client
and a keep-alive server), `db` (SQLite), `testing` — plus two olang-source
modules (`colx`, `mathx`) compiled into the binary and differential-tested
against their native twins. Reference: [docs/stdlib.md](docs/stdlib.md).

## Execution model

A tree-walking interpreter is the semantic authority. Hot functions are
promoted to a register bytecode tier (the OVM) — and anything the OVM
cannot compile *identically* is refused and stays interpreted. Falling
back is always correct; diverging is never acceptable. Details:
[docs/internals.md](docs/internals.md), [docs/ovm.md](docs/ovm.md).

## Examples

[`examples/`](examples/) holds real programs — a task CLI, log analyzer,
template engine, workflow engine, parser combinators, a regex engine, a
JSON Schema validator, a markdown converter, an HTTP notes API — all run
by the self-hosted harness (`olang run_all.ol`) and in CI.

## Maturity

olang is a young language with an unusual amount of testing discipline
(45 test binaries; doc examples, tier agreement, and differential stdlib
tests in CI). It is well suited to scripts, teaching, and
experimentation; treat long-running services and dependency-heavy
projects as adventurous. The honest, current capability statement always
lives in [docs/stability.md](docs/stability.md).

## License

MIT

<p align="center">
  <img src="branding/mascot.svg" alt="Ollie, the olang otter, floating with the o> pebble" width="180"><br>
  <sub>Ollie, the olang otter</sub>
</p>

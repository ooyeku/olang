<p align="center">
  <img src="branding/banner.svg" alt="olang — pipelines, pattern matching, batteries included" width="840">
</p>

**olang is the Open Language** — and it means the word literally, at three
layers no other language offers together:

- **Open code** — a program's own structure is a stable, public data
  format your olang code reads and transforms (`meta.parse`), so linters
  and codemods are olang scripts, not compiler changes.
- **Open artifacts** — a compiled binary carries its own source and a
  checksum, and declares exactly what it may touch (`olang inspect`,
  per-dependency capabilities): never a black box, never a silent
  over-reacher.
- **Open execution** — any run records and replays bit-for-bit, anywhere
  (`olang --record` / `replay`): a bug report becomes a file.

These fall out of decisions olang already made — early-stabilized syntax,
immutable values, a small explicit effect boundary — and an incumbent
cannot follow (Python can't freeze its AST; Go won't embed source; no
mainstream runtime is deterministic enough to promise replay). See
**[Openness](docs/openness.md)** for the whole story.

Under that identity, olang is a batteries-included dynamic functional
language: pipelines, pattern matching, algebraic data types, immutable
values, `Result`-based errors — with a built-in data stack (Series,
Frames, statistical inference, SVG charts), real no-GIL parallelism, a
source-based package manager, and a three-tier runtime — interpreter →
bytecode VM → native JIT — where every tier below the interpreter must
agree with it exactly or refuse.

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

Or don't install anything: the website's **playground** runs the whole
language — interpreter, bytecode tier, and the full data stack —
compiled to WebAssembly, sandboxed in your browser (`cd website && bun
run dev`, then /playground; no code leaves the page).

```bash
cargo install --path .

olang script.ol            # run a program (args reach os.args())
olang                      # REPL
olang test                 # run `test` blocks under the current directory
olang fmt --check .        # formatter (whitespace hygiene, AST-safe)
olang check                # static checker: provable annotation violations
olang --watch script.ol    # rerun on every save (the edit-run loop)
```

## What's in the language

- **Expressions everywhere** — `if`, `match`, blocks, and loops produce
  values; `break value` and `return` for early exits.
- **Pattern matching** — literals, tuples, lists with `...rest`, structs,
  enum variants, ranges, or-patterns, guards.
- **Algebraic data types** — enums with payload constructors, validated
  struct declarations (shape and annotated field types are checked at
  construction), traits with runtime dispatch, `error` declarations.
- **Gradually typed** — unannotated code is fully dynamic at zero cost;
  every annotation is enforced at runtime on every tier, and
  `olang check` (plus the language server) reports provable violations —
  down to element types — before the program runs
  ([docs/types.md](docs/types.md)).
- **Immutability and capture-by-value closures** — values never mutate in
  place; closures snapshot their environment.
- **Errors as values** — `Result`, `?` propagation, `try`/`catch`.
- **Concurrency, honestly labeled** — `spawn` runs on a real OS thread
  (await joins it); `par_map`/`par_filter` fan pipelines — and `par for`
  fans loop iterations — across every core with no GIL (9–13× measured
  on compute-heavy kernels), all with spawn's snapshot semantics;
  `Promise.delay`/`all`/`race` are deterministic, deadline-based timing
  simulation.
- **Modules and packages** — `use`/`share`, plus `olang.toml` packages
  with lockfiles, checksums, a content-addressed cache, and Minimal
  Version Selection ([docs/packages.md](docs/packages.md)).

## The standard library

Twenty-one native modules — `str`, `col`, `math`, `json`, `toml`, `csv`, `re`,
`dates`, `time`, `random`, `crypto`, `base64`, `fs`, `os`, `http` (client
and a keep-alive server), `db` (SQLite), `testing`, `dom` (the browser,
in the wasm build), and the data stack (`ods`, `stats`, `plot`) — plus
two olang-source modules (`colx`, `mathx`) compiled into the binary and
differential-tested against their native twins. Reference:
[docs/stdlib.md](docs/stdlib.md); the browser story — olang as a
frontend language over the wasm build — has its own chapter,
[docs/wasm.md](docs/wasm.md).

## The data stack

`ods`, `stats`, and `plot` are part of the language — no import, no
flag, in every build including the browser playground. Series are typed,
null-aware columns with vectorized operators; Frames add the table verbs
(CSV, `group_by`, joins); `stats` covers distributions, t-tests, χ², and
OLS, every statistic pinned against scipy reference values; `plot`
renders standalone SVG charts as text.

```olang
let prices = ods.series([12.5, 8.0, 15.25, 4.0])
let taxed = prices * 1.07                        // vectorized operators
println(to_string(ods.mean(taxed)))

let x = ods.series([1.0, 2.0, 3.0, 4.0, 5.0])
let y = ods.series([2.1, 3.9, 6.2, 8.1, 9.8])
let fit = stats.lm(y, x)
println(to_string(map_get(fit, "r2") > 0.99))
```

Measured, not asserted: reductions at NumPy parity sequentially and
2.7× ahead in parallel; a 10M-row, 1k-group aggregation in 27.2 ms
single-threaded against 24.0 ms for Polars on 18 threads; a 1M×20 OLS
3.8× ahead of `numpy.linalg.lstsq`. The stack's chapter —
teaching, full benchmark tables, and every recorded deferral:
[docs/ods.md](docs/ods.md).

## Execution model

Three tiers. A tree-walking interpreter is the semantic authority. Hot
functions are promoted to a register bytecode VM (the OVM) — anything
the OVM cannot compile *identically* is refused and stays interpreted.
Hot numeric functions go one tier further: a Cranelift JIT compiles them
to native machine code — type-specialized, lazy, and call-graph-aware
(helpers, chains, and mutual recursion compile together with
native-to-native calls; struct field access rides borrowed pointers) —
and every guard failure deopts to bytecode, which owns all errors.
Falling back is always correct; diverging is never acceptable.

The bytecode tier covers the language people actually write — 158 of
the 164 functions in the example corpus promote (the six holdouts are
async and global assignment, by design). The JIT covers ints, floats,
structs, lists, tuples, and strings: each either compiles or refuses by
a tested rule (allocating loops, for instance, deliberately stay on
bytecode driving native constructors). Measured: fib(30) at 4 ms, level
with Node and Bun; N-body at 26 ms, 15× ahead of CPython; integer
kernels 20–30× over bytecode; float kernels ~4.5×. Details and measured
tables: [docs/internals.md](docs/internals.md),
[docs/ovm.md](docs/ovm.md).

## Examples

[`examples/`](examples/) holds real programs. The flagship is
[`demo/`](examples/demo/) — **Harborline**, a long-running
harbor-operations simulator (threaded unload crews over channels,
SQLite ledger, tariff expression trees, an RSA-signed hmac digest
chain, daily self-checked invariants) built to genuinely soak-test the
language; [docs/demo.md](docs/demo.md) reads it as a design study for
robust olang systems. Alongside it — a task CLI, log analyzer,
template engine, workflow engine, parser combinators, a regex engine, a
JSON Schema validator, a markdown converter, an HTTP notes API, a small
Lisp interpreter written in olang (`minilisp/`), a full-stack issue
tracker (`app/`: SQLite JSON API plus its own browser frontend, all
served by `olang main.ol`), a CSV→Frame→aggregate pipeline (`dataproc/`),
a full statistical study — permutation test and bootstrap fanned over
`par_map`, OLS, SVG charts, 10,000 subjects in under half a second
(`statlab/`) — and data-parallel prime counting that measures its own
speedup with `par_map` and `par for` (`parmap/`) — all run by the
self-hosted harness
(`olang run_all.ol`) and in CI.

## Maturity

olang is a young language with an unusual amount of testing discipline
(950+ tests across 49 binaries; doc examples, tier and JIT agreement, and
differential stdlib tests in CI). It is well suited to scripts, teaching, and
experimentation; treat long-running services and dependency-heavy
projects as adventurous. The honest, current capability statement always
lives in [docs/stability.md](docs/stability.md).

## License

MIT

<p align="center">
  <img src="branding/mascot.svg" alt="Ollie, the olang otter, floating with the o> pebble" width="180"><br>
  <sub>Ollie, the olang otter</sub>
</p>

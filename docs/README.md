# The olang Book

The complete documentation for **olang — the Open Language**. olang means
the word literally, at three layers: **open code** (a program's structure
is a stable, public data format you read with `meta.parse`), **open
artifacts** (a compiled binary carries its own source and declares what it
may touch — `olang inspect`, capabilities), and **open execution** (any
run records and replays bit-for-bit — the Open Timeline). No other
language offers all three; [the Openness chapter](openness.md) is the map.

Under that identity, olang is a minimal, expressive language with
first-class functions, pipelines, pattern matching, and a built-in data
stack, implemented in Rust with a three-tier runtime — a tree-walking
interpreter (the semantic authority), a bytecode VM, and a Cranelift JIT
that compiles hot numeric functions to native machine code. The same
language, data stack included, runs in the browser as WebAssembly on the
website's /playground — where the `dom` module also makes it a frontend
language.

Every olang code block in this book is executed by the test suite on every
change (`tests/doc_examples_test.rs`) — the documentation cannot drift from
the implementation.

## For users — writing olang

| Chapter | What it covers |
|---|---|
| **[Openness](openness.md)** | What *Open Language* means, mechanically — open code (`meta.parse`), open artifacts (`olang inspect` + capabilities), open execution (record/replay). olang's defining identity, and the map to where each pillar is documented |
| **[A Tour of olang](tour.md)** | The language, taught by building one small program end-to-end — from a list of strings to a parallel statistical report |
| **[The Language Reference](language.md)** | Every construct, precisely: values and mutability, evaluation order, operators, control flow, pattern matching, functions and closure semantics, types, traits, errors, concurrency, modules, testing |
| **[Common Pitfalls](pitfalls.md)** | The sharp edges collected in one place — missing map keys, integer division, operator-precedence surprises, closure capture, truthiness, indexing, and the other behaviors that trip up newcomers, each with the idiom that avoids it |
| **[Types](types.md)** | Gradual typing end to end: annotations as enforced promises, the three rules, runtime enforcement on every tier, the `olang check` static checker and its element-type analysis, and how to adopt types incrementally |
| **[The Standard Library](stdlib.md)** | Every global builtin and all thirty modules — native (`str`, `os`, `fs`, `http`, `db`, `proc`, …), the embedded olang packages (`cli`, `term`, `ui`, `viz`, `dash`, …), and the data stack — each with its design rationale and examples |
| **[The Data Stack](ods.md)** | `ods`, `stats`, and `plot` in depth: why columns beat rows (measured), every Series and Frame verb, a complete inference workflow, charts as SVG text, the stack's performance characteristics, and the design record — how it was built, benchmarked against NumPy/scipy/Polars, and why it evaluates eagerly |
| **[olang in the Browser](wasm.md)** | The same language end to end: the WebAssembly build, the `dom` module, structured events, canvas draw-lists, the `ui` view layer, routing and storage, Web Workers, and a guided reading of four complete olang frontends |
| **[Packages](packages.md)** | Scaffolding projects and libraries, manifests, dependencies, the lockfile model, versioning, the registry |
| **[Building Robust Systems](demo.md)** | `examples/demo` — Harborline, the long-running harbor simulator — read as a design study: state threading, ADTs that make illegal states unrepresentable, Result discipline, testable concurrency, self-auditing invariants, and determinism as a feature |

Complete worked programs live in [`examples/`](../examples/). The
flagship is [`demo/`](../examples/demo/) — **Harborline**, a
long-running harbor-operations simulator with eleven library modules,
built to soak-test the language and documented as
[its own chapter](demo.md). Around it: a task CLI, a log analyzer, a
template engine, a regex engine, a parser combinator library, a
state-machine engine, a JSON Schema validator, a small Lisp interpreter
written in olang (`minilisp/`), a full-stack issue tracker whose
frontend is olang in the browser (`app/`), a Frame-based data pipeline
(`dataproc/`), a parallel statistical study on the data stack
(`statlab/`), and more. Run them all with `cd examples && olang
run_all.ol`.

## For developers — working on olang

| Chapter | What it covers |
|---|---|
| **[Editors](editors.md)** | The built-in language server (`olang lsp`): diagnostics, completions, hover, go-to-definition, formatting; the VS Code and Zed extensions |
| **[Internals](internals.md)** | The implementation as a design study: why an interpreter is the authority, the fail-closed refusal ladder, how promotion/inference/deopt work, the value model, modules, how to add things |
| **[The OVM](ovm.md)** | The bytecode and JIT tiers in depth: the register machine's design, the JIT whitelist and specialization, measured performance, deliberate refusals |
| **[Stability](stability.md)** | What is stable, what is experimental, and how the language evolves from here |
| **[Tooling](tooling.md)** | The command-line toolbox: `olang test` (runner, with `--coverage`), `olang fmt` (formatter), `olang check` (static checker), `olang build` (standalone single-file executables), `olang doc` (API docs from `///` comments), `olang bench` (reproducible timings), and the Open Timeline (`--record` / `replay` for deterministic re-execution) |
| **[Roadmap](roadmap.md)** | Development history and direction — where decisions, verdicts, and deferred work are recorded |

## Reading order

Want to know what makes olang *olang*? Start with
[Openness](openness.md) — the three-pillar identity — then the Tour.
New to the language itself? **Tour → Language Reference** (skim, then keep
as reference) **→ examples/**, with [Common Pitfalls](pitfalls.md) close
at hand for the behaviors that surprise. Ready to build something real?
[Building Robust Systems](demo.md) walks the flagship example's
architecture pattern by pattern. Building something specific? Go straight to the
[stdlib](stdlib.md) chapter for your domain — data work has its own
chapter in [The Data Stack](ods.md), and frontends in
[olang in the Browser](wasm.md). Adding type annotations to a program?
[Types](types.md) is the whole story. Contributing?
**Internals → Stability**, then the reference chapters as needed.

## Conventions used throughout

- ` ```olang ` blocks are runnable as-is and verified by CI.
- ` ```olang no-run ` blocks are parse-checked but not executed (they need
  files, a network, a browser, or multiple modules).
- Fallible operations return `Result` — examples unwrap deliberately.

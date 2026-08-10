# The olang Book

The complete documentation for olang: a minimal, expressive language with
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
| **[A Tour of olang](tour.md)** | The language, taught by building one small program end-to-end — from a list of strings to a parallel statistical report |
| **[The Language Reference](language.md)** | Every construct, precisely: values and mutability, evaluation order, operators, control flow, pattern matching, functions and closure semantics, types, traits, errors, concurrency, modules, testing |
| **[The Standard Library](stdlib.md)** | Every global builtin and all twenty-two modules, each with its design rationale and examples — including the ods data stack (Series, Frames, stats, SVG charts) and the browser `dom` module |
| **[Packages](packages.md)** | Scaffolding projects and libraries, manifests, dependencies, the lockfile model, versioning, the registry |

Complete worked programs live in [`examples/`](../examples/) — a task CLI,
a log analyzer, a template engine, a regex engine, a parser combinator
library, a state-machine engine, a JSON Schema validator, a small Lisp
interpreter written in olang (`minilisp/`), a full-stack issue tracker
whose frontend is olang in the browser (`app/`), a Frame-based data
pipeline (`dataproc/`), a parallel statistical study on the data stack
(`statlab/`), and more. Run them all with `cd examples && olang
run_all.ol`.

## For developers — working on olang

| Chapter | What it covers |
|---|---|
| **[Editors](editors.md)** | The built-in language server (`olang lsp`): diagnostics, completions, hover, go-to-definition, formatting; the VS Code and Zed extensions |
| **[Internals](internals.md)** | The implementation as a design study: why an interpreter is the authority, the fail-closed refusal ladder, how promotion/inference/deopt work, the value model, modules, how to add things |
| **[The OVM](ovm.md)** | The bytecode and JIT tiers in depth: the register machine's design, the JIT whitelist and specialization, measured performance, deliberate refusals |
| **[Stability](stability.md)** | What is stable, what is experimental, and how the language evolves from here |
| **[Tooling](tooling.md)** | `olang test` (the test runner) and `olang fmt` (the formatter) |
| **[Roadmap](roadmap.md)** | Development history and direction — where decisions, verdicts, and deferred work are recorded |
| **[Design: the ods data stack](design/ods.md)** | How Series, Frames, stats, and plot were designed and measured — benchmarks vs NumPy, scipy, and Polars, with every deferral recorded |
| **[Design: lazy evaluation](design/ods-lazy.md)** | Why ods evaluates eagerly, and the measured gate for revisiting |

## Reading order

New to olang? **Tour → Language Reference** (skim, then keep as reference)
**→ examples/**. Building something specific? Go straight to the
[stdlib](stdlib.md) chapter for your domain. Contributing?
**Internals → Stability**, then the reference chapters as needed.

## Conventions used throughout

- ` ```olang ` blocks are runnable as-is and verified by CI.
- ` ```olang no-run ` blocks are parse-checked but not executed (they need
  files, a network, a browser, or multiple modules).
- Fallible operations return `Result` — examples unwrap deliberately.

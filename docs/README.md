# The olang Book

The complete documentation for olang: a minimal, expressive language with
first-class functions, pipelines, and pattern matching, implemented in Rust
with a tree-walking interpreter and a bytecode acceleration tier.

Every olang code block in this book is executed by the test suite on every
change (`tests/doc_examples_test.rs`) — the documentation cannot drift from
the implementation.

## For users — writing olang

| Chapter | What it covers |
|---|---|
| **[A Tour of olang](tour.md)** | Install to first program in fifteen minutes |
| **[The Language Reference](language.md)** | Every construct, precisely: literals, operators, control flow, pattern matching, functions, types, traits, errors, async, modules, testing |
| **[The Standard Library](stdlib.md)** | Every global builtin and all seventeen modules, with examples |
| **[Packages](packages.md)** | Manifests, dependencies, lockfiles, versioning, the registry |

Complete worked programs live in [`examples/`](../examples/) — a task CLI,
a log analyzer, a template engine, a regex engine, a parser combinator
library, a state-machine engine, a JSON Schema validator, and more. Run them
all with `cd examples && olang run_all.ol`.

## For developers — working on olang

| Chapter | What it covers |
|---|---|
| **[Internals](internals.md)** | Architecture: grammar → parser → interpreter → bytecode tier; the value model; modules; how to add things |
| **[The OVM](ovm.md)** | The bytecode tier in depth: design, measured speedups, limitations |
| **[Stability](stability.md)** | What is stable, what is experimental, and how the language evolves from here |
| **[Roadmap](roadmap.md)** | What comes next, each item grounded in real friction from the example programs |

## Reading order

New to olang? **Tour → Language Reference** (skim, then keep as reference)
**→ examples/**. Building something specific? Go straight to the
[stdlib](stdlib.md) chapter for your domain. Contributing?
**Internals → Stability**, then the reference chapters as needed.

## Conventions used throughout

- ` ```olang ` blocks are runnable as-is and verified by CI.
- ` ```olang no-run ` blocks are parse-checked but not executed (they need
  files, a network, or multiple modules).
- Fallible operations return `Result` — examples unwrap deliberately.

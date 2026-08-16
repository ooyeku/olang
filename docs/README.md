# The olang book

This is the reference documentation for olang, a dynamically typed functional
programming language implemented in Rust. The book covers the language, its
standard library, its toolchain, and its implementation.

Every olang code block in this book is compiled and executed by the test
suite (`tests/doc_examples_test.rs`) on each change, so the examples stay
consistent with the implementation.

For the conventions used throughout the book, see the
[documentation style guide](STYLE.md).

## Contents

### Part I — Getting started

| Chapter | Contents |
|---|---|
| [Introduction](introduction.md) | What olang is, its design goals, and when to use it. |
| [Installation](installation.md) | Installing the compiler, running programs, the REPL, and the browser playground. |
| [A tour of olang](tour.md) | The language taught by building one program end to end, from a list of strings to a parallel statistical report. |

### Part II — The language

| Chapter | Contents |
|---|---|
| [Language reference](language.md) | Every construct in detail: values and mutability, evaluation order, operators and precedence, control flow, pattern matching, functions and closures, user-defined types, traits, errors, concurrency, and modules. |
| [Types and gradual typing](types.md) | Type annotations as enforced promises, runtime enforcement on every tier, the `olang check` static checker, element-type analysis, and incremental adoption. |
| [Common pitfalls](pitfalls.md) | Behaviors that surprise newcomers — missing map keys, integer division, operator precedence, closure capture, truthiness, indexing — each with the idiom that avoids it. |

### Part III — The standard library

| Chapter | Contents |
|---|---|
| [Standard library reference](stdlib.md) | The global builtins and every module: native modules (`str`, `os`, `fs`, `http`, `db`, and others), the embedded olang packages, and the program-as-data interface (`meta`). |
| [The data stack](ods.md) | `ods`, `stats`, and `plot`: typed columns and data frames, statistical inference, SVG charts, performance characteristics, and the design rationale. |

### Part IV — Building and running programs

| Chapter | Contents |
|---|---|
| [Packages and dependencies](packages.md) | Project and library structure, manifests, the lockfile, version resolution, the registry, and the capability model. |
| [Command-line tooling](tooling.md) | The `olang` command line: running, testing, formatting, checking, building standalone executables, generating documentation, benchmarking, and recording and replaying runs. |
| [olang in the browser](wasm.md) | The WebAssembly build, the `dom` module, structured events, canvas draw-lists, the `ui` view layer, routing and storage, and Web Workers. |
| [Openness](openness.md) | Open code (`meta`), open artifacts (`olang inspect` and capabilities), and open execution (record and replay), described mechanically. |
| [Editor support](editors.md) | The language server (`olang lsp`) and the VS Code and Zed extensions. |

### Part V — Implementation and project

| Chapter | Contents |
|---|---|
| [Architecture and internals](internals.md) | The three-tier execution model, why the interpreter is the semantic authority, the refusal ladder, promotion and deoptimization, the value model, and the repository layout. |
| [The execution model: OVM and JIT](ovm.md) | The bytecode VM and the Cranelift JIT in depth: the register machine, the compilation whitelist, type specialization, measured performance, and the deliberate limits. |
| [Case study: building robust systems](demo.md) | A design study of the flagship example, `examples/demo` (Harborline): state threading, algebraic data types, `Result` discipline, testable concurrency, and self-checking invariants. |
| [Stability and compatibility](stability.md) | What is stable, what is experimental, and how the language evolves. |
| [Roadmap](roadmap.md) | The plan of record for reaching 1.0: the locked design decisions and the campaigns that implement them. |

## Reading paths

- **Learning the language.** Read the [Introduction](introduction.md) and
  [Installation](installation.md), work through [A tour of olang](tour.md),
  then keep the [Language reference](language.md) and
  [Common pitfalls](pitfalls.md) at hand while reading the programs in
  [`examples/`](../examples/).
- **Using a specific part of the library.** Go directly to the
  [Standard library reference](stdlib.md), or to
  [The data stack](ods.md) for data analysis and
  [olang in the browser](wasm.md) for frontend work.
- **Adding type annotations.** [Types and gradual typing](types.md) is the
  complete reference.
- **Contributing to the implementation.** Read
  [Architecture and internals](internals.md) and
  [The execution model](ovm.md), then
  [Stability and compatibility](stability.md).

## Example programs

Complete programs live in [`examples/`](../examples/). The largest is
[`demo/`](../examples/demo/) (Harborline), a long-running harbor-operations
simulator built to exercise the language across many features and documented
as a [case study](demo.md). The directory also includes a task-management
CLI, a log analyzer, a template engine, a regular-expression engine, a parser
combinator library, a JSON Schema validator, a Lisp interpreter written in
olang, a full-stack issue tracker whose frontend runs in the browser, and
several data-analysis programs. Run the whole set with
`cd examples && olang run_all.ol`.

## Conventions

- ` ```olang ` blocks are executed by the test suite and must run as written.
- ` ```olang no-run ` blocks are parse-checked but not executed; they require
  files, a network, a browser, or multiple modules.
- Fallible operations return a `Result`; examples call `unwrap` deliberately
  to keep them short.

The [Stability and compatibility](stability.md) chapter is the authoritative
statement of what is stable. Where any other text disagrees with it, that
chapter is correct.

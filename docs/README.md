# The olang book

This is the documentation for olang, a dynamically typed functional
programming language implemented in Rust. It covers the language, its
standard library and data stack, its tools, the browser runtime, and the
implementation.

Every olang code block in the book is run by the test suite, so the examples
match the implementation.

## Contents

### Part I — Getting started

| Chapter | Contents |
|---|---|
| [Introduction](introduction.md) | What olang is, what comes with it, and how this book is organized. |
| [Installation](installation.md) | Installing `olang` and `otc`, running programs, and the REPL. |
| [A tour of olang](tour.md) | The language taught by building one program, from a list of strings to a statistical report. |
| [Common pitfalls](pitfalls.md) | Behaviors that surprise newcomers, each with the idiom that avoids it. |
| [Stability and compatibility](stability.md) | What 1.0 promises: what is stable, what is experimental, the platforms it covers, and what a version number means. |

### Part II — The language

| Chapter | Contents |
|---|---|
| [Language reference](language.md) | Every construct: values and mutability, operators and precedence, collections, control flow, pattern matching, functions, pipelines and method chains, user-defined types, traits, errors, concurrency, and modules. |
| [Types and gradual typing](types.md) | Annotations enforced at runtime on every tier, the `olang check` static checker, element types, and adding types incrementally. |
| [Macros](macros.md) | Extending olang in olang: `meta fn`, `@` invocation, decorators, compile-time evaluation, and the rules expansion follows. |

### Part III — The standard library

| Chapter | Contents |
|---|---|
| [Standard library reference](stdlib.md) | The global builtins and every module: native modules, the embedded olang modules and packages, and the program-as-data interface (`meta`). |
| [The data stack](ods.md) | Series, Frames, expressions, grouping, dates and categories, matrices, statistics and time series, machine learning with model formulas, and charts with `plot` and `gg`. |

### Part IV — Tools

| Chapter | Contents |
|---|---|
| [Command-line tooling](tooling.md) | The `olang` command line: running, testing, formatting, checking, fixing, profiling, benchmarking, building, documenting, and recording and replaying runs. |
| [Packages and dependencies](packages.md) | Projects and `otc`: manifests, dependencies, the library shelf, the lockfile, toolchains, capabilities, and the transparent binary. |
| [Notebooks](notebooks.md) | The `.olmd` format: markdown with olang cells, cell options, params, capabilities, controls, and pinned output. |
| [Writing fast olang](performance.md) | The program shapes the execution tiers optimize, bulk data on the data stack, reading the tiers' decisions, and measured performance. |
| [Editor support](editors.md) | The language server (`olang lsp`) and the VS Code and Zed extensions. |

### Part V — The browser

| Chapter | Contents |
|---|---|
| [olang in the browser](wasm.md) | The WebAssembly runtime, the `dom` module, events, canvas draw-lists, the `ui` view layer, routing and storage, and Web Workers. |
| [The web SDK](web-sdk.md) | Full-stack web applications: one route table for server and browser, views as data, the store, forms, the bundle, and server push. |

### Part VI — Going deeper

| Chapter | Contents |
|---|---|
| [Architecture and internals](internals.md) | The pipeline, the interpreter as the authority, refusal and deoptimization, the value model, modules, testing, and the repository layout. |
| [The execution model: OVM and JIT](ovm.md) | The bytecode VM and the Cranelift JIT in depth: what each compiles, how each stays faithful, the compile cache, and measured performance. |

## Reading paths

- **Learning the language.** Read the [Introduction](introduction.md) and
  [Installation](installation.md), work through [A tour of olang](tour.md)
  and [Common pitfalls](pitfalls.md), then keep the
  [Language reference](language.md) at hand while reading the programs in
  [`examples/`](../examples/).
- **Analysing data.** [The data stack](ods.md), with the reference tables
  in the [Standard library reference](stdlib.md#ods--series-and-frames).
- **Building a project.** [Packages and dependencies](packages.md) and
  [Command-line tooling](tooling.md).
- **Building for the web.** [olang in the browser](wasm.md), then
  [The web SDK](web-sdk.md).
- **Making a program faster.** [Writing fast olang](performance.md), then
  [The execution model](ovm.md).
- **Working on the implementation.** [Architecture and
  internals](internals.md) and [The execution model](ovm.md).

## Conventions

- ` ```olang ` blocks are run by the test suite and must run as written.
- ` ```olang no-run ` blocks are parse-checked but not run; they need files,
  a network, a browser, or several modules.
- Fallible operations return a `Result`; examples call `unwrap` to stay
  short.
- [Stability and compatibility](stability.md) is the authoritative statement
  of what is stable. Where any other text disagrees with it, that chapter is
  correct.

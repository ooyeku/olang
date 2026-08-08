# olang Internals

How the implementation works, for people changing it. Read alongside
[the OVM chapter](ovm.md) (the bytecode tier in depth) and
[Stability](stability.md) (what you may and may not change).

Part of [the olang book](README.md) ·
[Language](language.md) · [Standard Library](stdlib.md)

## The pipeline

A program moves through four stages:

```text
source (.ol)
   │  pest PEG grammar          grammar.pest
   ▼
parse pairs ──► AST             src/parser.rs → src/ast.rs
   │  static passes             src/analyze.rs, src/type_checker.rs
   ▼
tree-walking evaluation         src/interpreter.rs
   │  hot functions promoted
   ▼
register bytecode (OVM)         src/ovm/ — on by default
```

**`grammar.pest`** is the single source of truth for syntax. pest generates
the parser from it at build time; every syntax change starts here. The
expression hierarchy in the grammar *is* the operator-precedence table
documented in the [language reference](language.md#operators-and-precedence).

**`src/parser.rs`** walks pest's pairs into the AST (`src/ast.rs`). The AST
is uninterpreted structure: `Statement`, `Expr`, `Pattern`, `TypeDefinition`,
plus `Value` — the runtime value enum that the interpreter produces.

**`src/analyze.rs`** runs cheap static checks (unused/undefined hints);
**`src/type_checker.rs`** holds the optional annotation checker. Neither
gates execution today — the runtime is dynamic (see
[Stability](stability.md)).

**`src/interpreter.rs`** is the reference semantics: a tree-walking
evaluator over `Value` with a persistent-map `Environment` (cheap closure
snapshots, copy-on-write scoping).

**`src/ovm/`** is the acceleration tier: functions that get called often are
compiled to register bytecode and re-executed there. It is on by default.

## The two-tier execution model

The single most important invariant: **the interpreter and the OVM must
agree**. A function may run on either tier depending on call counts, so any
semantic divergence becomes a bug that appears and disappears with warmup.
Several real bugs of this class have been found and fixed (string ordering,
list concatenation); when you touch either tier's operators or value
semantics, touch both — or make the OVM refuse to compile the construct.

Refusal is the designed escape hatch: the OVM compiler returns
`CompilationFailed` for anything it does not support (globals, closures over
modules, `return`/`break value`, and more — see
[ovm.md](ovm.md#known-limitations)), and
the function transparently stays on the interpreter. **Falling back is always
correct; diverging is never acceptable.**

`--no-ovm` runs pure interpreter; `--ovm-stats` shows what was promoted.
When investigating a behavioral difference, run both modes first.

## The value model

`Value` (in `src/ast.rs`) is a clone-friendly enum: `Integer`, `Float`,
`String(Arc<String>)`, `Boolean`, `List(Arc<[Value]>)`, `Tuple`,
`Map(Arc<HashMap>)`, `Struct { type_name, fields }` (also used for anonymous
objects, modules, and parsed JSON objects), `Enum` / `EnumConstructor`,
`Function` (parameters, body, closure snapshot), `Builtin`, `Ok` / `Err`,
`Range`, `Promise`, `Unit`.

Conventions the code relies on:

- **Immutability**: values are never mutated in place; operations build new
  values. `Arc` makes the copies cheap.
- **Struct-likeness**: anything with named fields is a `Struct` under the
  hood — which is why `map_get` and friends read objects, structs, and JSON
  uniformly.
- **Closures capture by value**: a `Function` carries a snapshot of its
  environment's flat map (O(1), shared, persistent).
- **Control flow as signals**: `break`/`continue`/`?` unwind via dedicated
  `InterpreterError` variants (`BreakSignal`, `ContinueSignal`,
  `ErrPropagation`) that the loop evaluators and the function-call boundary
  intercept. They are errors only if they escape to top level.

## Modules

`use` resolves a dot path to a file (relative to the importing file, then
the project root, then package roots from the dependency map, then embedded
modules). Loading a module executes it in a fresh environment, collects its
`share`d exports into a `Struct { type_name: "Module" }`, then re-closes
every exported function over the *complete* module scope — so exports can
call private helpers regardless of declaration order.

Two subtleties worth knowing before touching this code:

- **Enum exports carry constructors.** Sharing an enum exports its variant
  constructors too, and importing the type binds them (there is no
  `Type::Variant` syntax, so the bare names must travel).
- **Function names must not collide across modules.** Interpreter
  environments and the OVM function registry are name-keyed; module loading
  takes care to keep same-named functions from different modules distinct.

The native stdlib (`src/stdlib/*.rs`) is registered as always-in-scope
modules. The *embedded* stdlib (`src/stdlib/embedded/*.ol`) is olang source
compiled into the binary — `colx` and `mathx` mirror `col` and `math` and
are differential-tested against them, so the language is exercised by its
own standard library.

## Errors

`InterpreterError` (thiserror) covers user-visible failures; the REPL and
CLI format them with source context and suggestions
(`src/error_formatter.rs`, `src/help.rs`). Language-level fallibility is
`Value::Ok`/`Value::Err` — the interpreter only raises hard errors for
genuine violations (type errors, undefined names, arity, overflow).

## Testing strategy

~40 integration test binaries under `tests/` plus unit tests. The layers:

| Layer | Where | What it protects |
|---|---|---|
| Doc examples | `doc_examples_test.rs` | every `olang` block in README + book chapters parses and runs |
| Example programs | `example_programs_test.rs` | the curated `examples/*.ol` keep working |
| Self-hosted harness | `examples/run_all.ol` | every example (incl. packages) runs in a real subprocess |
| Differential | `embedded_stdlib_test.rs` | `colx`/`mathx` agree with `col`/`math` |
| Tier consistency | `bytecode_tier_test.rs` and friends | OVM results match the interpreter |
| Feature regression | one file per fixed bug area | fixed bugs stay fixed |

The dogfooding methodology that produced much of the current hardening:
build a real program in olang (a CLI, an engine, a validator), fix every
root cause it surfaces, land the program as an example with regression
tests. Repeat in a new domain.

Gates for every change: `cargo test --release`, `cargo clippy --release
--all-targets` (zero warnings), `cargo fmt --check`, and — for anything
user-visible — verify through the installed binary, in both tiers.

## How to add things

**A global builtin**: implement in `src/builtin.rs`, register it in the
builtins table, dispatch in `call_builtin`. Add a regression test and a row
+ example in [stdlib.md](stdlib.md).

**A stdlib module function**: implement in `src/stdlib/<module>.rs`, add to
the module's registration *and* its dispatch match. Return `Result` values
(`Value::Ok`/`Value::Err`) for anything fallible. Document it.

**Syntax**: extend `grammar.pest`, build the AST in `src/parser.rs`, handle
it in the interpreter — and decide explicitly whether the OVM compiles it or
refuses (never silently diverges). Add cases to `analyze.rs` match arms
(they are exhaustive). Document it in [language.md](language.md) with a
runnable example — the doc test will hold you to it. Check
[Stability](stability.md) first: syntax changes must be additive.

**An example program**: a directory under `examples/` with `olang.toml` and
`main.ol` — `run_all.ol` discovers it automatically. Register it in
`examples/README.md`.

## Repository map

```text
grammar.pest              syntax (pest PEG)
src/
  main.rs                 CLI (file mode, REPL, flags)
  parser.rs, ast.rs       parsing → AST + Value
  interpreter.rs          reference semantics
  analyze.rs              static hints
  type_checker.rs         optional annotation checking
  builtin.rs              global builtins
  stdlib/                 native modules + embedded/ (olang-source)
  ovm/                    bytecode tier: compiler, VM, tier manager
  pkg/                    package manager (see packages.md)
  repl.rs, help.rs        interactive mode
docs/                     this book
examples/                 runnable programs + run_all.ol harness
tests/                    integration suites
```

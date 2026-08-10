# olang Internals

How the implementation works — written both for people changing it and
for people who want to study how a language gets built. olang's runtime
is organized around a small number of load-bearing design decisions,
and this chapter presents each one as what it is: a choice, with
reasons, alternatives, and consequences. Read alongside
[the OVM chapter](ovm.md) (the bytecode and JIT tiers in depth) and
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
tree-walking evaluation         src/interpreter/
   │  hot functions promoted
   ▼
register bytecode (OVM)         src/ovm/ — on by default
   │  hot numeric functions compiled
   ▼
native machine code (JIT)       src/ovm/jit.rs — Cranelift, on by default
```

**`grammar.pest`** is the single source of truth for syntax. pest generates
the parser from it at build time; every syntax change starts here. The
expression hierarchy in the grammar *is* the operator-precedence table
documented in the [language reference](language.md#operators-and-precedence)
— precedence is defined once, in the grammar, and documented from it,
rather than existing separately in a Pratt parser's table where the two
could drift.

**`src/parser.rs`** walks pest's pairs into the AST (`src/ast.rs`). The AST
is uninterpreted structure: `Statement`, `Expr`, `Pattern`, `TypeDefinition`,
plus `Value` — the runtime value enum that the interpreter produces.

**`src/analyze.rs`** runs cheap static checks (unused/undefined hints);
**`src/type_checker.rs`** holds the optional annotation checker. Neither
gates execution — the runtime is dynamic (see [Stability](stability.md)).

**`src/interpreter/`** is the reference semantics: a tree-walking
evaluator over `Value` with a persistent-map `Environment` (cheap closure
snapshots, copy-on-write scoping). It is split into semantic components —
core eval (`mod.rs`), environments, modules, patterns, operators, errors,
and the spawn registry.

**`src/ovm/`** is the acceleration machinery: functions that get called
are compiled to register bytecode and re-executed there, and bytecode
that qualifies for the JIT whitelist compiles further to native machine
code via Cranelift (`src/ovm/jit.rs`). Both are on by default.

## Why an interpreter is the authority

The first design decision everything else hangs on: **the tree-walking
interpreter defines the language, and every faster tier must agree with
it exactly.**

Why give the *slowest* component the crown? Because a dynamic language's
semantics live overwhelmingly in its corners — which error a bad index
raises and with what message, what `for` does with a range versus a
list, how `Int` and `Float` mix inside a structural comparison, the
precise moment a closure snapshots its environment. No specification
document keeps up with that level of detail, but an implementation does:
a tree-walker is essentially an executable specification, one small
`match` arm per construct, cheap to write, easy to audit, and — the
crucial property — easy to *trust*.

With a trusted oracle in place, "is the optimizer correct?" stops being
a matter of argument and becomes a testable property: run the program
both ways, compare. The differential suites
(`tests/bytecode_differential_test.rs`, `tests/bytecode_tier_test.rs`,
`tests/jit_test.rs`) do exactly that, over arithmetic edge cases,
overflow errors, aliasing, recursion, and every guard boundary the JIT
has. This is the same shape every mature tiered VM (V8, JSC, LuaJIT,
PyPy) converged on: keep the simplest tier forever, because it is the
semantic reference, the deopt target, and the one component you can
reason about when the others disagree.

The practical rule this buys: any observable difference between
`--no-ovm` (pure interpreter) and the default tiered execution is, by
definition, a bug in the tier — never a "new behavior". When
investigating anything surprising, run both modes first.

## The refusal ladder

The second decision: **when a tier cannot reproduce the interpreter
exactly, it must refuse — never approximate.** Fail-closed, with a rung
per tier:

- The bytecode compiler returns `CompilationFailed` for anything outside
  its supported subset (global assignment, `return` crossing bytecode
  loops, async, and more — see [ovm.md](ovm.md#known-limitations)), and
  the function transparently stays on the interpreter forever.
- The JIT only prequalifies functions whose every bytecode instruction
  is on its whitelist; anything else stays on bytecode.
- At runtime, a compiled function that hits any guard — unexpected
  argument kinds, overflow, division by zero, call-depth exhaustion —
  **deopts**: the native run is abandoned and the call re-executes on
  the tier below, which owns every error message.

Why is refusal the right default? Consider the alternative. An optimizer
that *approximates* an unsupported construct produces divergences that
appear and disappear with warmup — the same function gives different
answers on its first and thousandth call. That is the worst bug class a
language runtime can have: non-deterministic, unreproducible in
isolation, and invisible to unit tests that don't happen to cross the
promotion threshold. Refusal converts that entire class into a
performance question ("why didn't this promote?") which is observable
(`--ovm-stats`), harmless, and fixable incrementally.

The ladder also makes the system *growable*. Supporting a new construct
in a tier is purely additive: implement it exactly, extend the
differential tests, and functions that use it start promoting. Nothing
can regress semantically, because the failure mode of an incomplete
implementation is a refusal, not a wrong answer. **Falling back is
always correct; diverging is never acceptable** — the whole
architecture is that sentence, enforced.

## How promotion, inference, and deopt work

The tier manager (`src/ovm/tier.rs`) promotes eligible functions to
bytecode at their first call. When a promoted function calls another
user function, the compiler reports the unresolved callee rather than
giving up; the tier compiles that callee and retries — names are
registered with the VM *before* compilation (and withdrawn on failure),
which is what lets mutually recursive functions resolve each other.

JIT compilation goes further and is **type-specialized, lazy, and
call-graph aware**. On the first call of a prequalified function, the
JIT plans every function reachable through its call sites and runs
*kind inference* to a global fixpoint across the group: every register
is proven to hold `i64` or `f64`, callee return kinds feed caller
registers, and since a register's possible-kinds mask only ever grows,
the analysis converges. Mixed int/float arithmetic promotes the integer
side exactly as the VM does; a register may hold mixed kinds only if
nothing ever reads it (liveness flows backwards through copies). The
whole group then compiles together with direct native-to-native calls,
so helpers, call chains, and mutual recursion stay native rather than
bouncing through the VM at every boundary.

Each compiled function guards its entry on the exact argument kinds it
was specialized for; any other shape runs on bytecode. And this is
where a third design decision quietly pays for the other two: **the JIT
whitelist admits only pure operations**. Because a qualifying function
has no side effects, a deopt can simply *re-execute the whole call* on
bytecode — no partially-performed effects to undo, no resume-point
bookkeeping, no on-stack replacement machinery. Purity turns
deoptimization from the hardest problem in JIT engineering into a
retry. The JIT never reproduces an error message either; it only ever
declines or deopts, and bytecode produces the canonical error. Every
native call carries a depth budget clamped to the VM's own call-depth
limit, so runaway recursion errors identically instead of overflowing
the native stack.

## The scratch-ownership model

Native code that *allocates* — struct construction, string
concatenation — poses a memory-safety question the pure-arithmetic JIT
never faced: heap values are reference-counted `Arc`s, and a deopt can
abandon a native run at any guard. Who owns a value the native code
built but never got to return?

The answer is a **scratch list owned by the VM, per native call**: every
heap value a native invocation constructs is registered in that list at
creation. If the call completes, ownership of the return value transfers
once, at the entry boundary, where a retain helper resolves the borrowed
pointer into an owned `Arc`; the scratch list then drops whatever else
was built. If the call deopts at any point, the scratch list drops
everything. Leaking and dangling are both structurally impossible — not
prevented by careful code, but unrepresentable in the ownership scheme.

Arguments travel the other direction on the same discipline: structs,
lists, and strings pass into native code as **borrowed** pointers. JIT
calls are synchronous and the caller's slot outlives the call, so no
refcount is touched at all on the read path — specialization resolves
field indices per interned shape at compile time, and every read goes
through one guarded helper that deopts on any surprise, so no layout
assumption can leak into the VM.

Two refusal rules, set by measurement, keep the model where it wins:
allocation compiles only in straight-line code (constructors), while
**allocating loops stay on bytecode and drive the native constructors
call by call** — an unbounded native loop of allocations would hold its
scratch memory until call end; and mixed string/number `+` (formatting)
stays on bytecode. The [OVM chapter](ovm.md#the-jit) covers the full
whitelist and the measured results.

## The value model

`Value` (in `src/ast.rs`) is a clone-friendly enum: `Integer`, `Float`,
`String(Arc<String>)`, `Boolean`, `List(Arc<[Value]>)`, `Tuple`,
`Map(Arc<HashMap>)`, `Struct { type_name, fields }` (also used for anonymous
objects, modules, and parsed JSON objects), `Enum` / `EnumConstructor`,
`Function` (parameters, body, closure snapshot), `Builtin`, `Ok` / `Err`,
`Range`, `Promise`, `Unit`.

Conventions the code relies on:

- **Immutability**: values are never mutated in place; operations build new
  values. `Arc` makes the copies cheap — and immutable-plus-refcounted is
  what lets values cross thread and tier boundaries as a refcount bump
  rather than a deep copy.
- **Struct-likeness**: anything with named fields is a `Struct` under the
  hood — which is why `map_get` and friends read objects, structs, and JSON
  uniformly.
- **Closures capture by value**: a `Function` carries a snapshot of its
  environment's flat map (O(1), shared, persistent). This one
  representation choice simultaneously gives the language its
  [capture semantics](language.md#closures-capture-by-value), makes
  `spawn`/`par_map` safe without locks, and lets the bytecode compiler
  bake closure lookups as constants.
- **Control flow as signals**: `break`/`continue`/`?` unwind via dedicated
  `InterpreterError` variants (`BreakSignal`, `ContinueSignal`,
  `ErrPropagation`) that the loop evaluators and the function-call boundary
  intercept. They are errors only if they escape to top level.

Memory management is **reference counting, no tracing collector**.
olang's values are overwhelmingly immutable and acyclic (nothing in the
language constructs a cycle of values), so deterministic reclamation
when the last reference drops is both sufficient and simpler than a GC —
there are no pauses to tune and no object graphs to trace.

## Modules

`use` resolves a dot path to a file (embedded modules first, then package
dependencies, then relative to the importing file, then the project root,
then the native stdlib — the order is documented in the
[language reference](language.md#how-use-resolves)). Loading a module
executes it in a fresh environment, collects its `share`d exports into a
`Struct { type_name: "Module" }`, then re-closes every exported function
over the *complete* module scope — so exports can call private helpers
regardless of declaration order.

Two subtleties worth knowing before touching this code:

- **Enum exports carry constructors.** Sharing an enum exports its variant
  constructors too, and importing the type binds them (there is no
  `Type::Variant` syntax, so the bare names must travel).
- **Function names must not collide across modules.** Interpreter
  environments and the OVM function registry are name-keyed; module loading
  takes care to keep same-named functions from different modules distinct.

The native stdlib (`src/stdlib/*.rs`) is registered as always-in-scope
modules. The *embedded* stdlib (`src/stdlib/embedded/*.ol`) is olang source
compiled into the binary — `colx` and `mathx` mirror `col` and a subset of
`math` and are differential-tested against them, so the language is
exercised by its own standard library.

A third registration path exists for native *values*: `src/native.rs` is
the module registry through which a Rust component registers stdlib-style
namespaces, native value types, and operator behavior into *both*
execution tiers at once. Its proving instance is the ods data stack
(`src/ods/` — Series, Frames, stats, plot — over the pure-Rust
`olang-ods/` workspace crate). Native values cross the tier boundary as
one shared Arc — a refcount bump, never a conversion — so the lossy
round-trip failure mode is unrepresentable for them.

## Errors

`InterpreterError` (thiserror) covers user-visible failures; the REPL and
CLI format them with source context and suggestions
(`src/interpreter/errors.rs`, `src/help.rs`). Language-level fallibility is
`Value::Ok`/`Value::Err` — the interpreter only raises hard errors for
genuine violations (type errors, undefined names, arity, overflow). The
two layers mirror the language's own philosophy: expected failures are
values a program handles; contract violations are errors a programmer
fixes.

## Testing strategy

The suite is layered so that each guarantee the book makes has a test
that owns it:

| Layer | Where | What it protects |
|---|---|---|
| Doc examples | `doc_examples_test.rs` | every `olang` block in README + book chapters parses and runs |
| Example programs | `example_programs_test.rs` | the curated `examples/*.ol` keep working |
| Self-hosted harness | `examples/run_all.ol` | every example (incl. packages) runs in a real subprocess |
| Differential | `embedded_stdlib_test.rs` | `colx`/`mathx` agree with `col`/`math` |
| Tier consistency | `bytecode_tier_test.rs` and friends | OVM results match the interpreter |
| JIT parity | `jit_test.rs` | every guard edge agrees byte-for-byte, tiered vs interpreted |
| Engine properties | `ods_*_test.rs` (+ `olang-ods` unit tests) | Series/Frame/stats kernels against naive references and scipy constants |
| Feature regression | one file per fixed bug area | fixed bugs stay fixed |

Behind the layers sits a working method: **dogfooding as the discovery
mechanism**. Build a real program in olang (a CLI, an engine, a
validator, a web app), fix every root cause it surfaces, land the
program as an example with regression tests, repeat in a new domain.
The examples directory is that method's sediment — each program exists
because building it made the language better, and keeps running in CI
so it stays true.

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
  interpreter/            reference semantics (core eval, environment,
                          modules, patterns, ops, errors, spawn registry)
  analyze.rs              static hints
  type_checker.rs         optional annotation checking
  builtin.rs              global builtins
  stdlib/                 native modules + embedded/ (olang-source)
  native.rs               module registry for native values (both tiers)
  ods/                    the data stack modules: series, frame, stats, plot
  ovm/                    bytecode tier: compiler (bytecode.rs), value
                          model (value.rs), tier manager (tier.rs),
                          Cranelift JIT (jit.rs), NaN-boxing primitives
                          (nanbox.rs)
  parallel.rs             par_map / par_filter / par for worker config
  playground.rs           the wasm session engine (playground + dom)
  pkg/                    package manager (see packages.md)
  tools/                  olang test (test_runner.rs), olang fmt (fmt.rs)
  repl.rs, help.rs        interactive mode
  clock.rs, output.rs     native/wasm seams (time, print routing)
olang-ods/                pure-Rust engine crate behind src/ods/
playground/               cdylib crate: the language as wasm for the website
website/                  static site incl. /playground
docs/                     this book
examples/                 runnable programs + run_all.ol harness
tests/                    integration suites
```

# OVM — the Olang Virtual Machine

Part of [the olang book](README.md) ·
[Internals](internals.md) · [Stability](stability.md)

This document describes how Olang executes code, what parts of the OVM are
real today, and what is still scaffolding. It is deliberately explicit about
the second category: knowing what *isn't* implemented is more useful than a
feature list that overstates the system.

## Execution model

Olang has two execution tiers.

| Tier | What it is | When it runs |
|---|---|---|
| **Interpreter** | Tree-walking evaluator over the AST | Always; the default and the semantics reference |
| **Bytecode** | Register-based VM (`src/ovm/bytecode.rs`) | Eligible functions, on by default (compiled at first call) |

The interpreter is the source of truth. The bytecode tier is an optimization
that must be **observationally identical** to it — see
[Correctness policy](#correctness-policy).

There is no JIT tier in operation. See [Not implemented](#not-implemented).

### Promotion

The tier is **on by default**: eligible functions are compiled to bytecode
on their first call, so a hot loop inside a function called once still runs
on the VM. There is nothing to configure.

```bash
olang program.ol                   # fast by default
olang --ovm-stats program.ol       # report promoted / rejected / call counts
olang -v program.ol                # log each promotion decision
olang --ovm-tier=50 program.ol     # raise the promotion threshold
olang --no-ovm program.ol          # pure tree-walking interpreter
```

`--no-ovm` is the escape hatch and the semantics reference: it disables the
tier entirely. `--ovm-tier=N` delays promotion until the Nth call (its value
needs the `=` so a bare flag doesn't swallow the filename). A function that
fails compilation is marked ineligible and keeps running on the interpreter —
which is why the default can never change program behavior.

When a function calls another user function, the compiler reports the
unresolved callee rather than giving up; the tier compiles that callee and
retries. Names are registered with the VM *before* compilation, which is what
lets mutually recursive functions resolve each other — and are withdrawn if
compilation fails, so nothing later resolves a call against a name that has no
bytecode.

The decision logic lives in `src/ovm/tier.rs`.

### What can be promoted

A function is eligible when its body uses only the subset the VM implements:

- integer, float, boolean, string, list, and range literals
- arithmetic, comparison, and logical operators
- `if`/`else` expressions
- `match` expressions over literals, wildcards, bindings, integer ranges,
  or-patterns, guards, and destructuring of `Ok`/`Err`, lists (including
  `...rest`), tuples, enum variants (a bare unit-variant name is an
  equality match, not a binding — the interpreter's rule), and struct /
  anonymous-object patterns — nested to any depth
- `Ok(..)` / `Err(..)` construction
- field access (`p.x`) and indexing (`xs[i]`, negatives from the end)
- enum values end to end: unit variants bake as closure constants,
  tuple-variant constructors (`Circle(2.0)`) compile to a `MakeEnum`
  instruction with arity checked at compile time, `==`/`!=` compare
  structurally, and enums round-trip the tier boundary losslessly (they
  used to be crushed into a struct shape that could not convert back)
- struct literals and anonymous objects. Literals validate against the
  declared field set at *compile* time with the interpreter's exact rules
  (unknown type, missing field, surprise field all refuse, so the
  interpreter raises its own error); a type redeclared with a different
  shape invalidates compiled functions and its literals refuse from then
  on, keeping the interpreter's live registry the authority
- calls to the pure `math` module functions (`math.sqrt`, `math.sin`,
  `math.pow`, ...) — recognized as builtins when the module is a bare
  identifier, so a numeric kernel promotes instead of falling back on its
  first `sqrt`
- pipelines (`|>`), desugared to the equivalent call
- free identifiers that resolve in the function's own closure — global
  constants, module-level bindings, and named functions passed as values —
  baked as constants. Sound because the interpreter installs exactly that
  closure as the call environment, and closures are declaration-time
  snapshots: a global mutated after the function's declaration is not seen
  by either tier (pinned by a test)
- lambdas, including those capturing the enclosing function's *runtime*
  state (a parameter, a local). Free variables resolving in the
  declaration-time closure travel with the lambda as that snapshot;
  runtime captures compile via a `MakeClosure` instruction that reads the
  captured registers at the lambda expression — the interpreter's own
  capture-by-value moment — with the lambda body compiled once, taking
  the captures as hidden trailing parameters. The resulting closure runs
  natively through `map`/`filter` and converts losslessly to an
  interpreter function when it crosses the tier boundary
- blocks, `let` bindings, assignment to locals, `while` and `for` loops,
  `break`, and `continue`
- calls through function *values*: parameters and locals holding
  functions (`f(x)` where `f` is a parameter — locals shadow builtins,
  exactly as interpreted), curried calls (`g(a)(b)`), immediately invoked
  lambdas, and aliased functions from the closure. Compiled function
  values run in the VM; anything declined runs through the bridge
  interpreter, which owns arity errors, default parameters, and the
  non-callable error. Function values also round-trip the tier boundary
  now (wrapped verbatim), so higher-order user functions promote
- calls to the pure `str` module functions (`str.length`, `str.char_at`,
  `str.split`, ...) and `show`, over the same bridge as `to_string`
- calls to itself (recursion), to other user functions (compiled on demand,
  including mutual recursion), and to 43 builtins (see [Builtins](#builtins))

Anything else causes the function to stay interpreted:

- *assigning* to a global (reads bake as snapshot constants; a write
  would need the interpreter's environment)
- method calls (`value.m(..)` where `value` is not a stdlib module):
  they dispatch on the value's runtime type through trait impls, which a
  compile-time field read cannot replicate
- a free identifier absent from the function's closure — the interpreter
  would resolve it through the caller's runtime scope chain, which no
  compile-time snapshot can represent
- maps, async, and struct-variant enum construction (unit and tuple
  variants compile)
- a lambda capturing a name the enclosing function binds only *later*
  (no register holds it yet at the lambda expression); calling a
  lambda-valued expression directly
- struct and enum patterns, and or-patterns that bind variables (each
  alternative would otherwise leave different bindings on the success path)
- calling a function that itself cannot be compiled (the rejection propagates
  to every caller)
- default parameter values
- arguments or return values that don't round-trip through the OVM value
  model (functions, maps, promises, enums — and any struct holding one of
  those). Plain structs, objects, and parsed JSON objects *do* round-trip
  when every field does, so field-reading functions promote

None of these are errors. They are compile-time rejections that fall back to
the interpreter, which is why enabling the tier can never break a program.

## Correctness policy

**Promotion may never change what a program does.** Everything the VM cannot
handle is rejected at compile time rather than approximated at runtime. This
rule is enforced by two test suites:

- `tests/bytecode_differential_test.rs` (30 groups) runs programs through
  *both* the interpreter and the VM and asserts identical results —
  arithmetic and overflow errors, float semantics, comparisons, branches,
  recursion, loops, strings, lists, ranges, arity errors, and a set of
  aliasing cases specific to the register-window design. It also asserts the
  inverse: unsupported features must be *rejected*, never miscompiled.
- `tests/bytecode_tier_test.rs` (100+ tests) runs whole programs with and
  without the tier enabled and asserts the observable results match,
  including mixed programs where some functions are promoted and others are
  not, transitive and mutual recursion, and function redefinition.

If you extend the VM, extend the differential suite in the same change. A
divergence found by these tests is a bytecode bug by definition — the
interpreter defines the language.

## Value model

`OvmValue` (`src/ovm/value.rs`) is the VM's runtime value. Immediate values
(integer, float, boolean, unit) are stored inline; heap values (string, list,
tuple, function, struct, range, thunk, stream) hold `Arc` payloads.

**Memory is managed by reference counting.** There is no tracing collector.
Values are reclaimed deterministically when the last reference drops, which
suits a language whose values are overwhelmingly immutable and acyclic.

`src/ovm/gc.rs` holds only the safepoint flags the interpreter polls in
loops; the former tracing-GC machinery, region allocator, and the `:gc` REPL
command were deleted in the 0.24 cleanup — with reference counting there is
nothing to force.

The value header carries only a type tag, execution tier, and lazy state. It
is deliberately small and `Copy`, because it is cloned on every register read
in the dispatch loop.

## Bytecode VM

A register machine. Key design points:

- **Register windows.** Variables *are* registers: parameters occupy registers
  `0..n` in declaration order and `let` bindings allocate their own register.
  Reading a variable compiles to nothing at all — it names the register the
  value already lives in. There is no separate locals array to copy through.
- **Two-pass label resolution.** Jumps are emitted against label ids, then a
  resolution pass patches every target to an instruction offset before
  execution. A jump to an unplaced label is a compilation error.
- **Per-call frames.** Each call gets its own register file, saved and
  restored around nested execution, so recursion works.
- **Native operand dispatch.** Arithmetic and comparisons match on the value
  representation directly; there is no conversion to and from the AST value
  type inside the loop.
- **Bounded recursion.** The VM enforces the same 1000-frame call-depth limit
  as the interpreter, so runaway recursion reports an error instead of
  overflowing the host stack.
- **Index-based iteration.** `for` compiles to an index loop over `IterLen`
  and `IterGet`, which handle both lists and ranges. A range is never
  materialized, so iterating `0..10000000` costs no memory — matching the
  interpreter.
- **Guarded extraction.** Destructuring emits a shape test before any
  extraction, so an extractor can never see a value it doesn't fit. Nested
  patterns recurse on the extracted register.
- **Lambdas as constants.** A lambda whose free variables all resolve in
  the enclosing function's declaration-time closure is built once at compile
  time and stored in the constant pool as an `AstFunction` — an interpreter
  function held verbatim, which converts back losslessly. It carries a
  closure filtered to exactly its free set: attaching the full snapshot
  would defeat the interpreter's empty-closure fast path on every trivial
  lambda call.
- **Total pattern tests.** `match` uses `PatternEq` and `PatternInRange`
  rather than the ordinary comparison instructions, because a pattern of the
  wrong type must simply not match where an ordinary comparison would raise a
  type error. Falling past every arm emits `MatchFail`, the interpreter's
  pattern-match failure.

There are currently **no optimization passes**. The previous pipeline (dead
code elimination, register renaming, peephole rewrites, control-flow
optimization) was removed because each was incorrect: DCE deleted live control
flow and stores, register renaming rewrote only three opcodes, and the
control-flow pass treated label ids as addresses. Passes may return
individually, each validated against the differential suite.

## Builtins

The VM does not reimplement builtins — it calls the interpreter's own
implementations through `BuiltinFunctions::call`. Reimplementation would be a
second source of truth that could drift from the semantics the differential
tests hold the VM to; delegating makes them identical by construction.

43 builtins are enabled:

| Group | Builtins |
|---|---|
| Conversion | `to_string`, `to_int`, `to_float`, `typeof`, `len` |
| List access | `head`, `tail`, `cons`, `concat`, `reverse`, `sort`, `take`, `skip`, `flatten`, `zip`, `enumerate`, `chunk`, `range` |
| Aggregation | `sum`, `min`, `max`, `average`, `contains` |
| Higher-order | `map`, `filter`, `reduce`, `fold`, `find`, `map_filtered`, `result_map`, `result_map_err`, `unwrap_or_else` |
| Strings | `split`, `join`, `starts_with`, `ends_with` |
| Results | `is_ok`, `is_err`, `unwrap`, `unwrap_or` |
| Numeric | `clamp` |
| Output | `print`, `println` |

The higher-order builtins became available when compiled lambdas gained
closures: a lambda the VM builds itself can be handed to a delegated builtin
as a function value. `map`, `filter`, and `sum` go further: when the
collection is a list and the function argument compiles (checked once and
cached per function value), the loop runs *inside* the VM — one bytecode
call per element, no conversion at the boundary — and a mapped list flows
into `sum` without leaving the VM's value model. Anything declined bridges
to the interpreter, which stays the semantic authority; a native loop never
falls back mid-flight, so element errors propagate exactly as the
interpreter would. One category remains deliberately excluded:

- **Map-returning builtins** (`map_set`, `group_by`, ...) produce values that
  do not survive the round trip back to an AST value — a `Map` would come back
  as a `Struct`. `execute_builtin_call` checks representability and errors
  rather than silently returning a corrupted value.

`BytecodeVm::round_trips` is the single definition of which values survive the
boundary; the tier uses it to decide whether a call's arguments and result can
cross. Keeping a second copy in the tier caused a silent regression once — it
omitted `Ok`/`Err`, so every call passing a `Result` fell back to the
interpreter despite Results converting fine.

A user function shadows a builtin of the same name, matching the interpreter's
environment lookup: declaring `fn clamp(...)` makes calls to `clamp` resolve to
the user's definition in compiled code too.

## Performance

Measured on an Apple Silicon laptop, release build. Reproduce with
`cargo run --release --example tier_compare` and the workloads described in
the benchmark suite.

The 0.23 cycle changed the performance story twice. First the bytecode tier
delivered 26–120× over the interpreter on call-heavy code. Then a rewrite of
the interpreter's call path — swapping environments instead of overlaying and
restoring every closure entry on each call, adopting closures as persistent
maps in O(1), and keeping call-frame bindings in a probed vector — made the
*interpreter* 50–90× faster on those same workloads, collapsing the tier's
relative advantage:

| Workload | Interpreter | Bytecode tier | Tier speedup |
|---|---|---|---|
| `fib(27)` (recursive calls) | 0.27 s | 0.18 s | ~1.5× |
| 100k-iteration `while` loop | 26.6 ms | 3.5 ms | ~7.5× |
| 1M-iteration `Result` construct + match loop | 1.32 s | 1.11 s | ~1.2× |
| 2000 × 500-element `map`/`filter`/`fold` pipeline | 1.25 s | 1.01 s | ~1.2× |

(For scale: before the call-path rewrite the interpreter took 23.6 s, 64 s,
and 93 s on the first, third, and fourth rows.)

The honest summary: the tier's headline numbers were largely measuring
interpreter overhead that is now gone. Loop-heavy code still benefits
meaningfully (~7.5×) because the VM avoids per-iteration AST dispatch;
call-heavy code benefits modestly. Per-call interpreter cost is now ~0.65 µs
(down from ~33 µs).

Since the tier is on by default, these are the numbers a plain `olang
program.ol` gets — no flags. `otc ovm --compare <file>` runs a program both
ways, verifies the results agree, and reports the timings and promotion
statistics.

`cargo bench` measures the interpreter itself (`benches/interpreter_bench.rs`,
ten representative programs). Use it when changing the evaluator; use
`tier_compare` when changing the VM.

## Known limitations

These are real gaps, not oversights:

1. **Lambdas capturing enclosing runtime state fall back.** A lambda
   referencing an enclosing function's parameter or local — `(x) => x * k`
   where `k` is a parameter — keeps its enclosing function interpreted.
   Lifting this needs per-call closure construction at runtime. Struct and
   enum patterns are also uncompiled.
2. **Map-returning builtins are unavailable** (see [Builtins](#builtins)).
   A function calling one stays interpreted, and so does every function that
   calls it.
3. **Builtin calls cost a value round trip.** Delegation converts arguments
   and results between the OVM and AST value models, so a function dominated
   by builtin work sees a much smaller speedup than one dominated by
   arithmetic and control flow. Delegated lambdas also execute their bodies
   in the interpreter — the VM accelerates the code *around* a pipeline, not
   inside its lambdas.

## Not implemented

- **JIT compilation.** There is no native-code tier. The earlier Cranelift
  scaffolding emitted placeholder functions whose execution would have been
  undefined behavior; it was disabled in 0.23 and deleted in 0.24. A real
  JIT would be a fresh implementation compiling from bytecode.
- **Tracing garbage collection.** Memory is reference-counted; `gc.rs` holds
  only the safepoint flags the interpreter polls.
- **Automatic SIMD vectorization** and pipeline fusion: the speculative
  engines were deleted rather than finished — explicit bulk stdlib
  operations are the honest route if vectorization matters later.

## Source map

| File | Role |
|---|---|
| `src/interpreter.rs` | Tree-walking evaluator; the semantics reference |
| `src/resolve.rs` | Slot resolution for function bodies |
| `src/ovm/tier.rs` | Promotion decisions and eligibility |
| `src/ovm/bytecode.rs` | Compiler, instruction set, and dispatch loop |
| `src/ovm/value.rs` | `OvmValue`, the reference-counted value model |
| `src/ovm/gc.rs` | Safepoint flags the interpreter polls in loops |

Deleted in the 0.24 cleanup (~12,000 lines): the `OlangVirtualMachine`
routing layer and `ovm_integration` (measured ~70% slower than the plain
interpreter, and the default path until 0.24), the placeholder JIT
scaffolding, the tracing-GC/region-allocator remnants, and the pipeline,
SIMD, lazy, fusion, and adaptive engines — none of which were wired into
execution.

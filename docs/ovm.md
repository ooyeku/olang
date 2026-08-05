# OVM — the Olang Virtual Machine

This document describes how Olang executes code, what parts of the OVM are
real today, and what is still scaffolding. It is deliberately explicit about
the second category: knowing what *isn't* implemented is more useful than a
feature list that overstates the system.

## Execution model

Olang has two execution tiers.

| Tier | What it is | When it runs |
|---|---|---|
| **Interpreter** | Tree-walking evaluator over the AST | Always; the default and the semantics reference |
| **Bytecode** | Register-based VM (`src/ovm/bytecode.rs`) | Hot functions, when `--ovm-tier` is enabled |

The interpreter is the source of truth. The bytecode tier is an optimization
that must be **observationally identical** to it — see
[Correctness policy](#correctness-policy).

There is no JIT tier in operation. See [Not implemented](#not-implemented).

### Promotion

With `--ovm-tier[=N]`, the interpreter counts calls to each named user
function. On the Nth call (default 50) it tries to compile the function to
bytecode. If compilation succeeds, that and all later calls execute on the VM;
if it fails, the function is marked permanently ineligible and keeps running on
the interpreter.

```bash
olang --ovm-tier program.ol        # promote after 50 calls
olang --ovm-tier=10 program.ol     # promote sooner
olang --ovm-tier --ovm-stats p.ol  # report promoted / rejected / call counts
olang --ovm-tier -v program.ol     # log each promotion decision
```

Note that `--ovm-tier` takes its value with `=` (`--ovm-tier=10`), so that a
bare `--ovm-tier` doesn't swallow the following filename.

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
  `...rest`), and tuples — nested to any depth
- `Ok(..)` / `Err(..)` construction
- pipelines (`|>`), desugared to the equivalent call
- lambdas whose free variables all resolve in the enclosing function's
  declaration-time closure — which covers lambdas calling other user
  functions, builtins, and stdlib modules, and lambdas referencing global
  constants. The lambda carries that closure verbatim (the same snapshot the
  interpreter layers over the call-site chain), so resolution is identical in
  both tiers
- blocks, `let` bindings, assignment to locals, `while` and `for` loops,
  `break`, and `continue`
- calls to itself (recursion), to other user functions (compiled on demand,
  including mutual recursion), and to 43 builtins (see [Builtins](#builtins))

Anything else causes the function to stay interpreted:

- referencing a global or captured variable — the VM has no environment
- structs, maps, async
- lambdas capturing the enclosing function's *runtime* state: a parameter,
  a local, or any name the enclosing function assigns (the interpreter would
  capture the runtime value, which a declaration-time snapshot cannot
  represent); calling a lambda-valued expression directly
- struct and enum patterns, and or-patterns that bind variables (each
  alternative would otherwise leave different bindings on the success path)
- calling a function that itself cannot be compiled (the rejection propagates
  to every caller)
- default parameter values
- arguments or return values that don't round-trip through the OVM value
  model (functions, structs, maps, promises)

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
- `tests/bytecode_tier_test.rs` (80 tests) runs whole programs with and
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

`src/ovm/gc.rs` and `src/ovm/memory.rs` are accounting layers only —
allocation statistics, safepoint flags, and lifecycle for the `:gc` REPL
command and `--ovm-stats`. `GarbageCollector::force_collection` reports stats;
it does not trace or sweep.

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
- **Lambdas as constants.** A non-capturing lambda has no runtime
  dependencies, so it is built once at compile time and stored in the
  constant pool as an `AstFunction` — an interpreter function held verbatim,
  which converts back losslessly. `FunctionObject` cannot serve here: it
  drops parameter metadata and rewrites the closure.
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

34 builtins are enabled:

| Group | Builtins |
|---|---|
| Conversion | `to_string`, `to_int`, `to_float`, `typeof`, `len` |
| List access | `head`, `tail`, `cons`, `concat`, `reverse`, `sort`, `take`, `skip`, `flatten`, `zip`, `enumerate`, `chunk`, `range` |
| Aggregation | `sum`, `min`, `max`, `average`, `contains` |
| Strings | `split`, `join`, `starts_with`, `ends_with` |
| Results | `is_ok`, `is_err`, `unwrap`, `unwrap_or` |
| Numeric | `clamp` |
| Output | `print`, `println` |

Two categories are deliberately excluded:

- **Higher-order builtins** (`map`, `filter`, `reduce`, `fold`, `find`,
  `group_by`, ...) take a function argument, and function values cannot cross
  into the VM. A call passing a named function is rejected at compile time
  anyway, since the callee is not a local.
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
2. **Higher-order and map-returning builtins are unavailable** (see
   [Builtins](#builtins)). A function calling one stays interpreted, and so
   does every function that calls it.
3. **Builtin calls cost a value round trip.** Delegation converts arguments
   and results between the OVM and AST value models, so a function dominated
   by builtin work sees a much smaller speedup than one dominated by
   arithmetic and control flow.
4. **The tier is opt-in.** It should default to on once coverage is wide
   enough that the check is worth paying on every call.

## Not implemented

Named here because earlier documentation claimed otherwise:

- **JIT compilation.** `src/ovm/optimization.rs` contains Cranelift
  scaffolding, but the code generation emits placeholder functions that return
  constant integers cast to pointers. Executing them would be undefined
  behavior, so the tier is disabled: `has_compiled_function` returns `false`
  and execution falls back to bytecode or the interpreter. Cranelift remains a
  dependency for the eventual real implementation.
- **Concurrent generational garbage collection.** Removed. See
  [Value model](#value-model).
- **Automatic SIMD vectorization.** `src/ovm/simd.rs` pattern-matches specific
  closure shapes; it is not wired into the execution path.
- **Fusion, lazy, pipeline, and adaptive engines.** Present as modules,
  bypassed by the execution path.

## Source map

| File | Role |
|---|---|
| `src/interpreter.rs` | Tree-walking evaluator; the semantics reference |
| `src/ovm/tier.rs` | Promotion decisions and eligibility |
| `src/ovm/bytecode.rs` | Compiler, instruction set, and dispatch loop |
| `src/ovm/value.rs` | `OvmValue`, the reference-counted value model |
| `src/ovm/gc.rs` | Safepoint flags and allocation accounting |
| `src/ovm/memory.rs` | Memory statistics and lifecycle |
| `src/ovm/execution.rs` | Tiered execution engine and tier transitions |
| `src/ovm_integration.rs` | Routing between the interpreter and the OVM |
| `src/ovm/optimization.rs` | JIT scaffolding (disabled) |

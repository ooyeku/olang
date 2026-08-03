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

The decision logic lives in `src/ovm/tier.rs`.

### What can be promoted

A function is eligible when its body uses only the subset the VM implements:

- integer, float, boolean, string, list, and range literals
- arithmetic, comparison, and logical operators
- `if`/`else` expressions
- blocks, `let` bindings, assignment to locals, and `while` loops
- calls to **itself** (recursion) and to VM-implemented builtins (`len`,
  `to_string`)

Anything else causes the function to stay interpreted:

- referencing a global or captured variable — the VM has no environment
- `match`, lambdas, pipelines, `for` loops, structs, maps, async
- calling another user function (see [Known limitations](#known-limitations))
- default parameter values
- arguments or return values that don't round-trip through the OVM value
  model (functions, structs, maps, promises)

None of these are errors. They are compile-time rejections that fall back to
the interpreter, which is why enabling the tier can never break a program.

## Correctness policy

**Promotion may never change what a program does.** Everything the VM cannot
handle is rejected at compile time rather than approximated at runtime. This
rule is enforced by two test suites:

- `tests/bytecode_differential_test.rs` (29 groups) runs programs through
  *both* the interpreter and the VM and asserts identical results —
  arithmetic and overflow errors, float semantics, comparisons, branches,
  recursion, loops, strings, lists, ranges, arity errors, and a set of
  aliasing cases specific to the register-window design. It also asserts the
  inverse: unsupported features must be *rejected*, never miscompiled.
- `tests/bytecode_tier_test.rs` (14 tests) runs whole programs with and
  without the tier enabled and asserts the observable results match,
  including mixed programs where some functions are promoted and others are
  not.

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

There are currently **no optimization passes**. The previous pipeline (dead
code elimination, register renaming, peephole rewrites, control-flow
optimization) was removed because each was incorrect: DCE deleted live control
flow and stores, register renaming rewrote only three opcodes, and the
control-flow pass treated label ids as addresses. Passes may return
individually, each validated against the differential suite.

## Performance

Measured on an Apple Silicon laptop, release build. Reproduce with
`cargo run --release --example tier_compare`.

| Workload | Interpreter | Bytecode tier | Speedup |
|---|---|---|---|
| `fib(20)` (recursive calls) | 837 ms | 6.9 ms | ~121× |
| 100k-iteration `while` loop | 27.9 ms | 4.2 ms | ~6.6× |

End to end through the CLI, `fib(27)`:

```
olang -b --no-ovm program.ol              # 23.6 s
olang -b --no-ovm --ovm-tier program.ol   #  0.20 s
```

Call-heavy code benefits most, because a promoted recursive function runs its
entire call tree inside the VM. Loop-heavy code benefits less: the remaining
overhead is per-instruction dispatch and value copies.

`cargo bench` measures the interpreter itself (`benches/interpreter_bench.rs`,
ten representative programs). Use it when changing the evaluator; use
`tier_compare` when changing the VM.

## Known limitations

These are real gaps, not oversights:

1. **Only self-recursive and leaf functions are promoted.** A call to another
   user function requires that callee to be compiled too, and the tier
   compiles one function at a time. `fib` and `fact` qualify; a function
   calling a helper does not. Lifting this (transitive compilation) is the
   highest-value next step.
2. **The expression subset is narrow.** No `match`, `for`, lambdas, or
   pipelines — which excludes a lot of idiomatic Olang.
3. **Only two builtins are implemented in the VM** (`len`, `to_string`). A
   function calling any other builtin stays interpreted.
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

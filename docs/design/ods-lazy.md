# ods: lazy evaluation and expression fusion — an evaluation

Design document · Status: **evaluated, deferred behind a decision gate** ·
Companion to [ods](ods.md) (Phase 4 required this document before any
lazy work).

This document answers one question: **should ods evaluate lazily and fuse
expression chains, and if so, at which layer?** The answer today is *no —
with a measured gate for reopening*. The reasoning is recorded so the
next visit starts from evidence instead of enthusiasm.

## The problem fusion solves

Eager elementwise kernels materialize every intermediate. In
`ys = xs * 2.0 + noise`, the multiply allocates and writes a 10M-element
buffer that the add immediately reads back and throws away: three memory
streams where a fused kernel needs two, plus an allocation.

Measured (B2, `a * b + 1.0` at 10M f64, Apple Silicon):

| Path | Time | What it does |
|---|---|---|
| ods sequential | 6.19 ms | two passes, one intermediate allocation |
| ods parallel | 2.42 ms | same shape, rayon |
| NumPy | 3.24 ms | the *same* two passes — NumPy doesn't fuse either |
| Fused single pass (estimated) | ~3–4 ms seq | one read of each input, one write, no intermediate |

Two facts frame the whole question:

1. **The eager engine already beats the competition.** Parallel ods is
   1.3× ahead of NumPy on this exact benchmark; NumPy pays the identical
   two-pass cost. Fusion would extend the lead, not close a gap.
2. **The win is bounded.** Fusion saves one memory stream per
   intermediate — roughly a third of the traffic for a two-op chain,
   less proportionally as chains shorten. Nobody's workload is 10×'d.

## The three places fusion could live

### A. Lazy frames at the language surface (Polars' answer)

`df |> lazy() |> filter(...) |> select(...) |> collect()` builds a query
plan; an optimizer fuses, pushes predicates past joins, prunes columns.

- **For:** the ceiling is highest here — predicate pushdown and column
  pruning routinely beat kernel fusion by orders of magnitude on real
  pipelines, because the fastest element is the one never read.
- **Against:** it is a second evaluation model for users to learn
  (when is my frame *real*?), an optimizer to build and test, and a
  debugging story ("why is my error reported at collect()?") that fights
  olang's if-it-errors-it-errors-here simplicity. This is the largest
  single piece of engineering discussed anywhere in the ods design.

### B. Peephole fusion in the bytecode tier

Series operators already flow through VM instructions
(`Mul dst, a, b` → module hook). A peephole could recognize
`Mul t, a, b; Add dst, t, c` with `t` dead afterwards and call a fused
kernel — **zero surface change**, semantics pinned by the existing tier
transparency suite.

- **For:** invisible, safe to ship incrementally, and the JIT work is
  building exactly the machinery (register liveness, instruction
  pattern recognition) this needs. A natural follow-on once the JIT's
  dust settles.
- **Against:** it only fires in *promoted* code (top-level pipeline
  scripts stay interpreted), only for operator chains (not `ods.*`
  calls), and the interpreter must behave identically — meaning every
  fused kernel exists twice or the peephole is bytecode-only slippage
  the correctness policy has to explain.

### C. Explicit fused kernels (the boring option)

Ship the two or three chains that dominate real workloads as named
kernels: `ods.fma(a, b, c)` (a·b + c), `ods.axpy(k, x, y)` (k·x + y),
`ods.clamp_scale(...)` if demanded. One pass, no intermediate, no
magic — and the pipe syntax keeps them readable.

- **For:** an afternoon each, testable against the composed form by
  construction, works in both tiers and the playground identically.
- **Against:** doesn't generalize; each pattern is a request.

## Decision

**Defer all three.** Concretely:

- **A (lazy frames)** is rejected for the foreseeable future. ods wins
  its benchmarks eagerly; the complexity is the toy-maker (the design
  doc's own non-goal table said so, and the measurements since have
  only supported it). It gets rebuilt from evidence or not at all.
- **B (peephole)** is the *right* eventual home, and deliberately waits
  for the JIT to stabilize — the liveness and pattern machinery should
  be built once, there. Revisit when the JIT's instruction-level
  analysis is no longer moving weekly.
- **C (fused kernels)** is pre-approved whenever the gate below trips —
  it needs no further design.

## The gate

Reopen this document when **a real workload (an example program, a user
report, or a benchmark that models one) spends more than ~20% of its
runtime in sequential elementwise Series chains of length ≥ 2**. First
response: add the specific fused kernel (option C) and measure. Only
sustained, spreading demand across many patterns justifies B, and only
optimizer-class wins (pushdown, pruning) ever justify A.

Until then: the eager engine is simple, measured, and ahead. Boring is
a feature.

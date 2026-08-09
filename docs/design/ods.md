# ODS — the Olang Data Stack

Design document · Status: **proposed** · Owner: olang core

Companion reading: [The OVM](../ovm.md) · [Roadmap](../roadmap.md) ·
[Internals](../internals.md)

This document specifies **ods**, a native numerical-computation engine built
into the OVM, and the module system that hosts it. It is a design document,
not user documentation: the olang code in it does not run yet, and none of
its blocks are executed by `tests/doc_examples_test.rs`.

The register follows the rest of the book: explicit about what is and is not
being built, with acceptance criteria that are measurements, not adjectives.

---

## Problem statement

olang today is suitable for light data work — CSV pipelines, seeded
simulations, descriptive statistics over small lists — and structurally
unsuitable for real statistics and data science. The gaps, in order of
severity:

1. **No numeric arrays.** A "list of floats" is `Arc<[Value]>`: every element
   a 16-byte tagged enum. Ten million floats is ten million boxed values with
   no cache locality, no SIMD, and no memory-bandwidth-bound loop possible in
   any tier. This is not a constant-factor problem; it is a representation
   problem, and no interpreter, bytecode, or JIT work can fix it.
2. **No linear algebra.** No matrix type, no solve/decompose/least-squares —
   so no regression, no PCA, no GLMs without O(n³) loops over lists.
3. **No statistical machinery.** `random.gauss` draws samples, but there are
   no distribution functions (pdf/cdf/ppf), so no tests, intervals, or
   p-values without hand-rolled erf approximations.
4. **No missing-data story.** Real datasets have holes. A stats stack that
   discovers nulls in version three retrofits them into every kernel.

ods closes these gaps in Rust, under the language, so that every olang-level
statistics or math library is fast *by construction* — the same relationship
NumPy has to the scientific Python ecosystem, and the tidyverse has to R,
except built into the language rather than bolted onto it.

## Goals and non-goals

**Goals**

- A contiguous, typed, null-aware array representation that both execution
  tiers share by reference — zero conversion cost at the tier boundary.
- Elementwise arithmetic, reductions, and selection at **NumPy parity** on
  large arrays (both are memory-bound; parity is the correctness bar for the
  representation, not a stretch goal).
- Linear algebra and statistical distributions via best-in-class pure-Rust
  crates, not hand-rolled kernels.
- A general **OVM module** contract, with ods as its first and proving
  instance.
- Pure Rust throughout, so the entire stack works in the wasm playground.

**Non-goals (deliberate, not deferred-by-accident)**

| Non-goal | Why |
|---|---|
| N-dimensional tensors | 1-D and 2-D cover statistics. General N-D broadcasting is NumPy's largest complexity tax and a different product (deep learning) |
| A dtype zoo | `f64`, `i64`, `bool` in v1; strings arrive with Frame (Phase 3). `f32`, dates, categoricals wait for a concrete demand |
| Hand-written linear algebra or special functions | `faer` and `statrs` exist; our value-add is integration and language surface, not a worse LAPACK |
| Lazy evaluation / expression fusion / query optimizer in v1–v3 | Eager kernels first. Lazy engines are where non-toy projects go to become toys. The bytecode-instruction design below leaves the door open |
| Plotting in Rust | Plot output is SVG/HTML text emitted by an olang-level module; the aesthetic layer iterates in olang, not in the engine |
| GPU execution | Out of scope entirely for this design |

## The core decision: one array, both tiers

Everything hangs on the representation. The engine defines one value kind:

```rust
pub struct OdsArray {
    dtype: DType,                 // F64 | I64 | Bool          (v1)
    buf: Buffer,                  // contiguous, 64-byte aligned, Arc-shared
    validity: Option<Bitmap>,     // Arrow-style null bitmap; None = no nulls
    shape: Shape,                 // Vector(len) | Matrix(rows, cols)
}
```

Design rules:

- **Nulls are first-class from day one.** The validity bitmap costs nothing
  when absent (`None`) and cannot be retrofitted later without touching every
  kernel. Every kernel is written null-aware from its first version, even
  when v1 semantics are simply "nulls propagate."
- **Copy-on-write mutation.** olang values are immutable and Arc-shared; ods
  keeps that story at the language level but uses `Arc::make_mut` inside
  kernels — when the refcount is 1, mutate in place. Functional semantics,
  in-place performance; `xs |> fill_null(0.0) |> cumsum()` allocates once,
  not three times. This is Polars' model.
- **The tier boundary is an `Arc` clone.** The interpreter's
  `Value::Native(...)` and the OVM's `ValueData::Native(...)` hold the *same*
  `Arc`. There is no conversion, so there is nothing to convert lossily. Maps
  and enums were each kept off the tier for releases by round-trip-lossy
  shapes ([ovm.md](../ovm.md), "crushed into a struct shape that could not
  convert back"); this design makes that failure mode unrepresentable.

## The OVM module contract

An OVM module is a Rust component that plugs native types and native
functions into both tiers. The contract is deliberately small:

```rust
pub trait OvmModule: Send + Sync {
    /// Stable name, e.g. "ods". Doubles as the feature-flag name.
    fn name(&self) -> &str;

    /// Stdlib-style namespaces to register, e.g. ("ods", {...}) —
    /// same shape as stdlib::get_stdlib() entries.
    fn namespaces(&self) -> Vec<(String, Value)>;

    /// Called by both tiers' builtin dispatch for this module's functions.
    fn dispatch(&self, func: &str, args: &[Value]) -> Result<Value, OlangError>;

    /// Called by both tiers when a binary operator has a Native operand.
    /// Returning None means "not mine" and falls through to the normal
    /// type-error path.
    fn binary_op(&self, op: BinaryOp, lhs: &Value, rhs: &Value)
        -> Option<Result<Value, OlangError>>;
}
```

Notes on each seam:

- **Registration** formalizes the pattern `math`/`csv`/`crypto` already
  follow: `stdlib::get_stdlib()` gains module-contributed namespaces, and the
  `builtin.rs` dispatch chain gains one registry lookup. No new machinery.
- **Native values** enter through one new variant per tier:
  `Value::Native(Arc<dyn NativeObject>)` in `src/ast.rs` and a mirror in
  `src/ovm/value.rs`. The `dyn` dispatch is paid **once per kernel call**,
  then the kernel runs over the whole buffer — amortized O(1/n) per element,
  which is the entire vectorization bet. `NativeObject` carries the
  obligations `Value` already imposes: `type_name` (for `typeof`), display,
  structural equality, and serialization (`Value` derives
  `Serialize`/`PartialEq`, so `Native` needs manual impls written once
  against the trait).
- **Operator interception** is the ergonomics jackpot. Both tiers already
  match on operand variants for `+ - * / == < ...`; each gains one arm: if
  either operand is `Native`, offer the op to the owning module. This is what
  makes `ys = xs * 2.0 + noise` three vectorized kernels instead of
  `ods.add(ods.mul(xs, 2.0), noise)`. Without it nobody will love the surface.
- **Correctness policy is inherited unchanged.** A module's functions behave
  identically from either tier because both tiers call the same Rust function
  on the same value. There is nothing tier-specific to test beyond "the value
  crosses the boundary as the same Arc" — pinned by one test.

**Crate layout** *(revised during Phase 0)*. The original plan put the whole
module in a workspace crate, with core depending on it for registration —
but that inverts: the glue implements a core trait over core's `Value`, so a
crate implementing `OvmModule` must depend on core, and core cannot depend
back on it. Resolution: the trait, registry, and module *glue* live in core
(`src/native.rs`, `src/ods/`, feature `ods`); the numerical *engine* arrives
in Phase 1 as a workspace crate (`olang-ods`) with **no dependency on olang**
— pure buffers and kernels, carrying the heavy dependencies (`faer`,
`statrs`, `pulp`) and its own criterion benches. Core depends on the engine,
thinly. Everything is pure Rust, so the wasm playground gets the full stack —
NumPy-class arrays in the browser.

## The engine: where the speed comes from

Assembled levers, in order of payoff. No clever code where a crate or the
compiler already wins:

| Lever | Buys | How |
|---|---|---|
| Contiguous typed buffers | 10–100× over boxed lists | The `OdsArray` representation; this is most of the win |
| LLVM autovectorization | SIMD for free | Kernels are tight, branch-free loops over `&[f64]`; the compiler does the rest |
| Explicit SIMD | AVX2/AVX-512/NEON where autovec fails | `pulp` runtime dispatch, only for kernels that measurably need it (null-aware reductions, comparisons) |
| Rayon | Near-linear scaling on large arrays | Already a dependency. Kernels split above a size threshold that respects the configured parallel threshold — the hard-coded `sum` parallelism was pruned in 0.29 (roadmap C4); it does not come back through ods |
| `faer` | World-class dense linear algebra | Pure Rust, competitive with OpenBLAS/MKL, wasm-compatible. Solve, Cholesky, QR, SVD, least-squares — regression and PCA handed to us |
| `statrs` | Distributions and special functions | pdf/cdf/ppf for normal/t/chi²/F/..., erf/gamma/beta machinery we never hand-roll |
| Copy-on-write | Fused-allocation pipelines | `Arc::make_mut` in every kernel's output path |

## The layer cake

```
L3  olang packages           domain libraries written in olang, composing L2
L2  stdlib: stats, la, plot  thin Rust tables; tidyverse verbs; |> friendly
L1  language surface         Series (1-D), Matrix (2-D), later Frame;
                             operators, constructors, conversions
L0  ods engine (Rust crate)  buffers, kernels, faer, statrs. No opinions,
                             just speed
```

The invariant that keeps the stack coherent: **L0 contains no statistics,
and L2 contains no loops.** `stats.t_test` is ~15 lines calling L0 kernels.
Finding a `for` over elements in L2 means a primitive is missing from L0 —
move it down. This is how "everything uses ods under the hood" is enforced
by construction rather than by review.

### Proposed surface (illustrative, not final)

```text
// Series: 1-D typed array with nulls
let xs = series([1.0, 2.0, 3.0])
let noise = random.gauss_series(0.0, 1.0, 1_000_000)
let ys = xs * 2.0 + 1.0                    // operators → vectorized kernels

ys |> mean()                               // null-aware reductions
ys |> quantile(0.95)
xs |> mask(xs > 2.0)                       // comparison → bool series → filter

// Statistics (Phase 2)
let fit = stats.lm(ys, design_matrix)      // OLS via faer: coef, se, r2, p
stats.t_test(a, b)
stats.norm.ppf(0.975)

// Frame (Phase 3): the tidyverse verbs
let df = csv.read("trials.csv") |> frame()
df |> filter((r) => r.dose > 0.0)
   |> group_by("arm")
   |> summarize(#{ mu: mean("response"), n: count() })
```

Surface decisions deferred to their phase: exact constructor names,
`Series`/`Matrix` type names in the type checker, list↔series conversion
builtins, and how comparison operators yield boolean masks.

## Benchmarks are the spec

The roadmap already pins performance claims to measurements (fib vs CPython,
[ovm.md](../ovm.md) "Measured"). ods adopts the same discipline: **the
acceptance criteria below are the definition of done for each phase.**
Numbers are collected by `benches/` (criterion) against pinned NumPy/Polars
versions on the reference machine, and recorded in this document when a
phase lands.

| # | Benchmark | Target | Rationale |
|---|---|---|---|
| B1 | `sum` / `mean` / `std` of 10M f64 | **Parity with NumPy** (±10%) | Memory-bound; missing parity means the representation is wrong, not that we need more tricks |
| B2 | Elementwise `a * b + c`, 10M f64 | Parity with NumPy (±10%) | Same argument |
| B3 | `sort` / `argsort`, 10M f64 | Within 2× of NumPy | Comparison sorts are compute-bound; 2× is honest for a first cut |
| B4 | OLS fit, 1M × 20 design matrix | Within 2× of `numpy.linalg.lstsq` | faer should deliver this or better |
| B5 | Null-aware `mean` with 10% nulls, 10M | Within 1.5× of B1's time | Validity bitmap must be nearly free |
| B6 (v2) | groupby-aggregate, 10M rows, 1k groups | Within 3× of Polars | Honest for an eager engine; still far ahead of pandas |

A phase whose benchmarks miss target does not ship with an asterisk; it
ships when the target is met or the target is revised *in this document
with the reason recorded*.

## Phases

Each phase is independently shippable and defensible; no phase begins until
the previous phase's gate is met.

| Phase | Contents | Gate |
|---|---|---|
| **0 — the seam** | `OvmModule` trait + registry; `Value::Native` / `ValueData::Native` through both tiers (`typeof`, display, equality, serialization); operator-interception arms; feature flag; empty ods module registered | All existing suites green with the feature on and off; a pinned test shows a native value crossing the tier boundary as the same Arc |
| **1 — Series** | f64/i64/bool arrays with validity bitmaps: constructors (`series(list)`, `ods.zeros`, `ods.linspace`, range conversion), operator arithmetic with scalar broadcasting, reductions (`sum mean var std min max quantile`), `sort argsort take mask filter cumsum dot` | B1, B2, B3, B5 met; benchmark table published in this doc; property tests pin every kernel against a naive interpreter-level reference implementation |
| **2 — stats** | `describe`, correlation/covariance, distributions (normal, t, chi², F: pdf/cdf/ppf/sample), one- and two-sample t-tests, chi² test, `stats.lm` (OLS via faer: coefficients, SE, R², p-values) | B4 met; results pinned against published reference values (R/scipy outputs recorded as constants in tests) |
| **3 — Frame** | Columnar table = named Series + string columns; `select filter with group_by agg join sort`; CSV/JSON bridges to the existing stdlib modules; the tidyverse verb layer | B6 met; `examples/dataproc` rewritten on Frame with a measured speedup recorded |
| **4 — plot + lazy** | `plot` as an SVG-text emitter (composes with the playground); *evaluate* lazy expression fusion — bytecode already flows ods ops through instructions, so peephole fusion is possible without surface changes | Explicitly gated on 1–3 being done; lazy work requires its own design doc |

## Relationship to the JIT (roadmap P2)

ods and the planned Cranelift baseline JIT are complementary, not competing:

- ods makes **array-shaped** workloads fast without any JIT — kernels are
  already native code, and no dispatch-loop improvement changes B1–B6.
- The JIT makes **scalar-loop** code fast — fib, N-body, tight recursion —
  which no array representation helps.

Together they close the two halves of the performance story. Neither
obsoletes the other, and neither blocks the other: Phase 0–2 of ods touches
the dispatch seams, not the execution loop the JIT will replace.

## Risks and open questions

- **`Value` trait-object obligations.** `Value` derives
  `Serialize`/`Deserialize`/`PartialEq`; `Native` needs careful manual impls
  (serialize as a typed payload, equality via the trait). One-time cost,
  isolated in Phase 0.
- **Type checker surface.** How `Series`/`Matrix` appear in annotations, and
  whether `[Float] -> Series` conversions are implicit anywhere (proposed:
  never implicit). Decided in Phase 1.
- **Operator semantics for masks.** `xs > 2.0` yielding a bool series
  changes the meaning of comparison operators for one type. Precedent is
  NumPy/R/Polars and it is scoped to Native operands, but it deserves a
  short section in the language reference when it lands.
- **Parallel-threshold configuration.** Kernels must respect the same
  configuration story the rest of the runtime uses (roadmap C4). Phase 1
  picks the mechanism.
- **Name.** `ods` collides with OpenDocument Spreadsheet in mindshare. The
  language-visible names (`series`, `frame`, `stats.`) matter more than the
  engine's internal name; revisit only if it causes real confusion.

# Roadmap

Where olang goes from 0.27. Every item here traces to concrete friction hit
while building the [example programs](../examples/) — nothing is speculative.
All of it is additive, per the [stability policy](stability.md): documented
syntax and behavior do not change; the language grows and gets faster.

Part of [the olang book](README.md) ·
[Stability](stability.md) · [Internals](internals.md)

Statuses: **planned** → **in progress** → **landed** (with the release).

## Tier 1 — completing what exists

Small, high-value items that finish surfaces the language already has.

| # | Feature | Grounding | Status |
|---|---|---|---|
| 1 | **Write-side struct-likeness**: `map_set` / `map_remove` accept objects, structs, and parsed JSON (returning a new value of the same kind), matching the read-side uniformity of `map_get`/`map_keys` | Workflow engine kept contexts as raw maps solely because parsed JSON couldn't be updated | landed (0.27) |
| 2 | **`for` pattern destructuring** — `for (i, x) in enumerate(xs)`, plus an `entries(m)` builtin so maps iterate as `(key, value)` pairs | `pair[0]`/`pair[1]` indexing in markdown, run_all, and workflow | landed (0.27) |
| 3 | **Strings iterate**: `for ch in "abc"` yields 1-character strings | `str.chars` detours in the regex engine and parser library | landed (0.27) |
| 4 | **`show(v)` builtin** — display rendering: strings bare, everything else as `to_string`. `to_string` keeps its repr form (strings quoted) unchanged | Quote-stripping workarounds in workflow, jsonschema, and the `join` fix | landed (0.27) |

## Tier 2 — control flow and errors

| # | Feature | Grounding | Status |
|---|---|---|---|
| 5 | **`return expr`** — early exit from a function; reuses the `?` unwind machinery | `let mut go = true` loop flags in the template and markdown parsers | landed (0.27) |
| 6 | **Implement `error` declarations** — variants become constructors producing `Err`-matchable values (`match r { Err(NotFound) => ... }`); moves the construct from Reserved to Stable | The webserver's hand-rolled 400/404/500 taxonomy | landed (0.27) |
| 7 | **`break value`** — `loop { ... break x }` as an expression | pairs with `return` | landed (0.27) |

## Tier 3 — stdlib gaps

| # | Feature | Grounding | Status |
|---|---|---|---|
| 8 | **`time` module** — `now_ms`, `monotonic_ms`, `sleep(ms)` | run_all times in whole seconds via `dates.timestamp(dates.now())`; no sub-second clock exists | landed (0.27) |
| 9 | **`fs.walk` / `fs.glob`** | run_all hand-rolled two-level directory discovery over flat `list_dir` | landed (0.27) |
| 10 | **`os.exec` options** — optional third argument `#{ "cwd": ..., "stdin": ..., "env": ... }`; 2-arg form unchanged | run_all wraps every exec in `os.chdir` | landed (0.27) |
| 11 | **`str.fmt("{} of {}", a, b)`** — display-form placeholders | long `+`/`pad_end` chains in every program that prints tables | landed (0.27) |
| 12 | **db transactions** — `db.begin` / `db.commit` / `db.rollback` | the webserver's create-then-select pattern is unsafe under future concurrency | landed (0.27) |

## Tier 4 — the two big lanes

| # | Feature | Grounding | Status |
|---|---|---|---|
| 13 | **OVM coverage expansion**, starting with `&&`/`||` as conditional jumps — today any function containing them falls back to the tree-walker permanently. Then strings, field access, closures. Every expansion lands with tier-agreement tests | verified: the OVM compiler had no And/Or lowering; guards are everywhere post-dogfooding | `&&`/`||` (0.27), field access + indexing (0.31), pure `math.*` (0.32), then the 0.33–0.36 arc: native `map`/`filter`/`sum` loops, baked globals, capturing lambdas (`MakeClosure`), struct construction with compile-time shape checks, first-class enums (construction, patterns, round-trip), function values as callees (`CallValue`), the pure `str` module, plus a register slab, interned struct shapes with inline caches, and immediate operands. Then 0.37–0.38: trait method dispatch, first-class maps, template strings, nested (and self-recursive) fns, tuples + tuple-pattern lets, bridged calls to every native stdlib module, and ten native collection builtins. 158 of 164 example-corpus functions promote (the six holdouts are async and global assignment, by design); N-body runs ahead of both CPython and Ruby. Lane closed — the continuation is [the performance campaign](#the-performance-campaign-039) below |
| 14 | **`http.serve` keep-alive**, then bounded concurrency — the handler contract is fixed; the execution model grows (as [stability](stability.md) already carves out) | sequential + Connection: close was the original model | keep-alive landed (0.27); bounded worker pool landed (0.29), load-tested in pure olang (examples/loadtest) — exact shared-state consistency under concurrent load |

## Tooling track

| # | Feature | Grounding | Status |
|---|---|---|---|
| 15 | **`olang test`** — discover and run `test` blocks across a package in isolation, with reporting | examples now carry self-check `test` blocks; make the pattern first-class | landed (0.28) |
| 16 | **`olang fmt`** — enforce the book's conventions mechanically | the fixed idioms (`= {` bodies, `=>` arms) are formatter-shaped | landed (0.28): whitespace hygiene with an AST-identity safety gate; layout normalization is future work |

## 0.29 — consolidation

An external review (August 2026) scored the language surface and semantic
discipline well but called the platform unevenly finished, and prescribed
consolidation over features. 0.29 addresses its bottom line, item by item.
Version is `0.29.0-dev` until every item lands.

| # | Item | The verified problem | The fix | Status |
|---|---|---|---|---|
| C1 | **Make the struct story real, types explicitly dynamic** | `NeverDeclared { surprise: 42 }` constructs; declared structs accept missing/extra fields — declarations are decoration | Constructing a *declared* struct validates the exact field-name set; an undeclared struct-literal name is an error; anonymous `{ ... }` objects stay free-form. Field *values* stay dynamic, stated once, plainly, in the book | landed (0.29) |
| C2 | **Finish or rename async; prune lazy scaffolding** | `spawn` evaluates eagerly and wraps a resolved promise; lazy handles are built then immediately forced; memory estimates are placeholders | `spawn` runs on a real thread via the existing thread-safe-clone machinery (pending promise, joined at `await`/`all`/`race`) — with a written fallback: if differential tests show shared-state divergence, keep deterministic semantics and rename the book section honestly. Documented `lazy`/`force` behavior stays; the immediately-forced internal handle plumbing and placeholder estimates are deleted | landed (0.29) — the fallback was NOT needed: spawn runs real threads |
| C3 | **Fix package resolution** | The resolver keeps an already-selected version whenever it is ≥ the new requirement's floor — never verifying it *satisfies* the requirement; the `Conflict` error is unreachable | Selection re-verifies every requirement (in-loop and at fixpoint); incompatible ranges produce `Conflict` naming the package and both requirements; MVS tests cover both the conflict and correct-floor cases | landed (0.29) |
| C4 | **Prune placeholder subsystems** | `sum` hard-codes parallelism at ≥5 elements, bypassing configuration; GC/memory accounting carries placeholder estimates; dead scaffolding surrounds the interpreter core | Numeric builtins respect the configured parallel threshold (sequential by default); an enumerate-then-delete audit removes scaffolding that has no observable behavior — suite green after every deletion | landed (0.29) |
| C5 | **Split the interpreter into semantic components** | `interpreter.rs` is 5,477 lines — evaluation, module system, patterns, operators, and caching in one file | Mechanical zero-behavior split into `src/interpreter/` (core eval, modules, patterns, ops); public API unchanged; move-only commits, full suite as the proof | landed (0.29): 5,628 → 2,629-line core + 7 components |
| C6 | **One authoritative capability document** | The README still says "v0.23, experimental", "11 modules", and calls `http.serve` a placeholder — while the book documents 18 modules and a working keep-alive server | [Stability](stability.md) becomes the single capability authority; the README is rewritten short and accurate, deferring to the book (its code blocks already run in CI) | landed (0.29) |

Deliberately out of scope, with reasons:

- **Enforcing `let mut`** — the book documents that every binding is
  assignable and `mut` marks intent; changing that now would break the
  stability contract for a stylistic gain.
- **Pipeline precedence** (`1 + 2 |> f` parses as `1 + f(2)`) — a real wart,
  but the precedence table is documented stable; revisiting it is a
  major-version conversation, recorded here so it isn't forgotten.

## The performance campaign (0.39+)

The 0.33–0.38 arc took the bytecode tier as far as bytecode tiers go:
ahead of CPython and Ruby on struct/float and map work, the whole
language covered (158 of 164 corpus functions promote), every step
checksum-verified. The profile now shows essentially all remaining time
in instruction dispatch itself — the ceiling of the tier's *class*. The
next order of magnitude requires the levers every language that crossed
this gap used (V8, LuaJIT, PyPy, JSC). In planned order:

| # | Lever | Grounding | Expected | Status |
|---|---|---|---|---|
| P1 | **Parallel pipelines** — a native `par_map` (and a parallel `for`) in the VM: worker threads over Arc-shared immutable inputs, one VM per worker. olang already has real OS threads with no GIL, immutable values, and capture-by-value closures, so data-parallel map is trivially safe in a way CPython structurally cannot offer | `spawn` is load-tested (examples/loadtest); N-body's force loop is embarrassingly parallel; pargrep already hand-rolls this pattern | ~core-count multiplier (≈8× on an M-series) on data-parallel workloads, independent of single-thread speed | **landed (0.40)**: `par_map`/`par_filter` — chunked fan-out, one interpreter clone per worker, each with its own bytecode tier (the same change gave `spawn` threads the tier they had silently been running without); spawn-style snapshot semantics, first-in-order error reporting, differential-tested against `map`/`filter`. Measured 9–13× vs sequential `map` on compute-heavy kernels (examples/parmap). Parallel `for` remains open |
| P2 | **Baseline JIT via Cranelift** — compile hot bytecode to native machine code, type-specialized: observe operand types during bytecode execution, emit int-/float-specialized code with guards that deoptimize to bytecode on type surprise. The tier system, promotion machinery, `CompiledBytecode` IR, and fallback contract are the exact substrate a JIT needs; "can't compile natively → stay on bytecode" extends the correctness story unchanged. fib(30) is the acceptance test, then floats + shape-checked field access for N-body | The dispatch loop is ~100% of remaining hot time; the fib gap to CPython (2×) and to the JITs (20–60×) is dispatch cost by construction. The 0.23 Cranelift scaffolding was deleted for being placeholder UB — the dependency choice stands, the implementation starts honest this time | 10–50× on hot numeric/monomorphic code: fib 89ms → low single-digit ms, N-body 400ms → tens of ms | **baseline landed (0.40)**: pure integer/boolean functions (arithmetic, comparisons, branches, self-recursion, loop kernels) compile to native via Cranelift at promotion time; every guard (non-int args, overflow, div-zero, depth) deopts to bytecode, which owns all errors — the JIT only ever declines. fib(30) 89ms → 5ms (18×, level with Node/Bun), integer kernels 20–30×, pipeline 27ms → 13ms. Remaining: float specialization (unlocks N-body), cross-function calls, heap values — sequenced with P3 |
| P3 | **Value-model slimming (NaN-boxing)** — `OvmValue` is ~32 bytes (a nearly-dead header plus a 16-byte enum) copied on every register move; NaN-boxed 8-byte values triple effective bandwidth through the dispatch loop and are what JIT-compiled code wants in registers. High-blast-radius rewrite gated on the differential suite; the header alone can go first as a cheap probe | Every profile since the register-slab rung shows value movement as the bulk of dispatch work; sequenced after P2 so the JIT reveals where boxing actually hurts | 2–3× broad, larger in JIT-compiled code | planned |

Explicitly rejected for this campaign: threaded dispatch (~1.5× measured
ceiling, and the compare+branch fusion experiment showed this dispatch
loop's layout fragility — recorded in the 0.36 changelog), and removing
the interpreter (it is the semantic oracle, the declaration engine, and
the deopt target; every tiered VM that made this jump kept its bottom
tier).

## Explicitly not planned

- **Qualified `Type::Variant` syntax** — variant constructors already travel
  with imports; a second access form would split the idiom.
- **Union type declarations** — enums cover the use cases; the annotation
  forms stay reserved.
- **Mutable closure capture** — capture-by-value is load-bearing for modules
  and OVM promotion. State lives in explicit stores; if demand appears, an
  explicit `cell` module beats changing capture semantics.
- **Macros / metaprogramming** — ten real programs never wanted one.

## Process

Each item lands the way everything since 0.25 has: root-cause
implementation, regression tests, both execution tiers agreeing (or the OVM
explicitly refusing), the book updated in the same commit — the doc tests
hold the two together — and a CHANGELOG entry saying why.

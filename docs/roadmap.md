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
| P1 | **Parallel pipelines** — a native `par_map` (and a parallel `for`) in the VM: worker threads over Arc-shared immutable inputs, one VM per worker. olang already has real OS threads with no GIL, immutable values, and capture-by-value closures, so data-parallel map is trivially safe in a way CPython structurally cannot offer | `spawn` is load-tested (examples/loadtest); N-body's force loop is embarrassingly parallel; pargrep already hand-rolls this pattern | ~core-count multiplier (≈8× on an M-series) on data-parallel workloads, independent of single-thread speed | **landed (0.40)**: `par_map`/`par_filter` — chunked fan-out, one interpreter clone per worker, each with its own bytecode tier (the same change gave `spawn` threads the tier they had silently been running without); spawn-style snapshot semantics, first-in-order error reporting, differential-tested against `map`/`filter`. Measured 9–13× vs sequential `map` on compute-heavy kernels (examples/parmap). Parallel `for` landed in 0.43 as the `par for` language construct — same snapshot semantics, implicit barrier, interpreter-owned with the tier refusing it fail-closed |
| P2 | **Baseline JIT via Cranelift** — compile hot bytecode to native machine code, type-specialized: observe operand types during bytecode execution, emit int-/float-specialized code with guards that deoptimize to bytecode on type surprise. The tier system, promotion machinery, `CompiledBytecode` IR, and fallback contract are the exact substrate a JIT needs; "can't compile natively → stay on bytecode" extends the correctness story unchanged. fib(30) is the acceptance test, then floats + shape-checked field access for N-body | The dispatch loop is ~100% of remaining hot time; the fib gap to CPython (2×) and to the JITs (20–60×) is dispatch cost by construction. The 0.23 Cranelift scaffolding was deleted for being placeholder UB — the dependency choice stands, the implementation starts honest this time | 10–50× on hot numeric/monomorphic code: fib 89ms → low single-digit ms, N-body 400ms → tens of ms | **baseline landed (0.40)**: pure integer/boolean functions (arithmetic, comparisons, branches, self-recursion, loop kernels) compile to native via Cranelift at promotion time; every guard (non-int args, overflow, div-zero, depth) deopts to bytecode, which owns all errors — the JIT only ever declines. fib(30) 89ms → 5ms (18×, level with Node/Bun), integer kernels 20–30×, pipeline 27ms → 13ms. Float specialization landed next (runtime-observed kinds, mixed promotion, ~4.5× on float kernels), then call-graph groups: helpers, chains, and mutual recursion compile together with native-to-native calls (split-fib 94ms → 5ms), then **struct field access** on the safe Arc model — borrowed pointers (calls are synchronous; no refcount touched), per-interned-shape specialization, one guarded read helper that deopts on any surprise (struct kernel 143ms → 18ms, 8×; N-body 350 → ~322ms). The remaining rungs — math builtins, list indexing and `for` iteration, tuple returns, struct construction, strings — landed across 0.41–0.42; the lane is complete (fib(30) 4 ms, N-body 48 ms) |
| P3 | **Value-model slimming (NaN-boxing)** — `OvmValue` is ~32 bytes (a nearly-dead header plus a 16-byte enum) copied on every register move; NaN-boxed 8-byte values triple effective bandwidth through the dispatch loop and are what JIT-compiled code wants in registers. High-blast-radius rewrite gated on the differential suite; the header alone can go first as a cheap probe — **probe landed (0.40)**: header removed, Result packed, NativeHandle thinned; OvmValue 32 -> 16 bytes, N-body -13% from layout alone. **Probe verdict: full NaN-boxing deferred.** The probe was the gate, and its measurement re-prices the rung: halving value size cut the most value-bound workload 13%, so halving again projects ~10% more — not the 2-3x estimated from the 32-byte baseline — while heap-pointer packing requires manual refcounting (Arc::into_raw at every register move), the exact GcPtr failure mode this codebase already measured, leaked, and deleted in 0.23. The nanbox primitives stay (proven, pinned, ready if the JIT's register model later wants them); the effort redirects to the lever the data does support: JIT struct field access on the safe Arc model — N-body's actual blocker | Every profile since the register-slab rung shows value movement as the bulk of dispatch work; sequenced after P2 so the JIT reveals where boxing actually hurts | 2–3× broad, larger in JIT-compiled code | **probe landed, full NaN-boxing deferred (0.40)** — the verdict and its evidence are recorded at left, so the rung is not re-attempted without new data; the redirected effort (JIT struct field access) landed under P2 |

Explicitly rejected for this campaign: threaded dispatch (~1.5× measured
ceiling, and the compare+branch fusion experiment showed this dispatch
loop's layout fragility — recorded in the 0.36 changelog), and removing
the interpreter (it is the semantic oracle, the declaration engine, and
the deopt target; every tiered VM that made this jump kept its bottom
tier).

### Where the campaign stands, and the next rungs

As of 0.40 all three levers were resolved in baseline form: P1 landed,
P2 landed through struct field access, and P3's probe landed with the
full rewrite deferred on its own measurement. The rungs that remained,
in order — all landed by 0.43:

- ~~**`math.sqrt` (and the rest of the pure `math` builtins) in the JIT
  whitelist**~~ — landed post-0.40: all 25 float-math builtins compile
  (sqrt/floor/ceil/trunc as native IEEE instructions, the rest through
  a helper that *is* the VM's own eval_float_math, exact by
  construction). Measuring it exposed the real remaining blocker in
  N-body's hot loops: list indexing and tuple extraction — the heap
  rung below.
- ~~**JIT heap values (list indexing and `for` iteration)**~~ — landed
  post-0.40: lists pass as borrowed pointers classified by element kind
  (float, int, one struct shape); `xs[i]`, `IterLen`, and `IterGet`
  compile through guarded helpers that reproduce the VM's indexing
  semantics (subscripts wrap negatives, iteration does not) and deopt
  on any surprise. **N-body: 332 -> 56ms** — the force loops compile
  whole, closing the campaign's original acceptance target (400ms ->
  tens of ms). Tuple extraction landed next (multi-value
  native returns), then struct construction (scratch-owned MakeStruct
  with entry-boundary ownership transfer; constructors compile, 
  allocating loops deliberately stay on bytecode driving them);
  strings landed last — parameters,
  fields, and constants as borrowed pointers, equality and lexicographic
  ordering through helpers running the VM's own operators, concat
  scratch-owned under the straight-line discipline. **The JIT lane the
  campaign opened is now closed**; parallel `for` landed as a
  language construct (`par for x in xs { ... }` — worker snapshots,
  implicit barrier, first-sequential-error reporting, interpreter-owned
  with the tier refusing it fail-closed). **P1, P2, and P3 are all
  resolved; the campaign is complete** apart from its recorded
  deferrals.

## The data campaign (0.40)

Run alongside the performance campaign and shipped whole: all four
phases of [the ods design record](ods.md#the-design-record) landed against measured
gates — Series (reductions at NumPy parity, 2.7× ahead in parallel),
stats (1M×20 OLS 3.8× ahead of `numpy.linalg.lstsq`, every statistic
pinned to scipy constants), Frame (10M-row group-by within 1.13× of
18-thread Polars, single-threaded), and plot (standalone SVG charts as
text). 0.40 made the stack unconditional: no cargo feature, no import,
in every build including the wasm playground. Deferred with recorded
reopening gates: lazy evaluation
([Why eager evaluation](ods.md#why-eager-evaluation)) and faer-backed
linear algebra (both recorded in the ods chapter's design record).

## The migration campaign — toolchain and dependencies

The tree builds on rustc 1.95 / edition 2021 with several dependencies
two or more majors behind. A full survey (toolchain, every direct
dependency, live crates.io versions, August 2026) sorted the upgrades
into four rungs by user-visible payoff and blast radius. The honest
finding up front: most of it is hygiene — the rungs below are ordered
so the ones olang programs can actually *feel* land first, and each
names its gate.

| ID | Item | Why / evidence | Expected gain | Status |
|---|---|---|---|---|
| M1 | **Toolchain + edition 2024 + drop `once_cell`** — `rustup update`, `cargo fix --edition` on both crates, replace `once_cell::sync::Lazy` with `std::sync::LazyLock` (parallel.rs, native.rs registry, http), adopt let-chains where the dispatch ladders earn it | Edition 2024 has been stable since 1.85; this codebase's interpreter and glue are unusually rich in nested `if let` ladders; `LazyLock` makes a whole dependency deletable | Maintainer ergonomics only — **zero olang-visible change**, one dependency fewer; mechanical, with 1,000+ tests as the net | landed |
| M2 | **Leaf minors in one batch** — pest, regex, csv, serde, tokio, rayon, clap, base64, thiserror 2, miette 7, rustyline 18, rusqlite 0.40, crypto crates | All non-breaking or trivially-breaking; live-version survey shows regex/csv carry ongoing perf work, miette 7 renders better diagnostics, rustyline is five majors of REPL line-editing, rusqlite bundles a years-newer SQLite engine | olang-visible: faster `re`/`csv` paths, a newer SQLite under the `db` module, prettier error reports, a better REPL | landed |
| M3 | **Cranelift 0.121 → 0.134** — its own PR, JIT-agent-owned | 13 releases of instruction selection and aarch64 codegen work land directly under every JIT'd olang function; moderate API churn | The one upgrade that can move the published numbers (fib 4 ms, N-body 48 ms) with zero olang changes. Gate: full JIT parity suite green, benchmark table re-measured and recorded either way | landed |
| M4 | **The three deliberate breaks** — reqwest 0.13/hyper 1 (client only; `http.serve` is hand-rolled and unaffected), rand 0.10, getrandom 0.4 | Each has a named blast radius: reqwest is an API migration; **rand changes seeded streams** (`StdRng` is not stream-stable across majors — `random.seed(42)` reproduces within a version, not across the upgrade; a CHANGELOG behavior note is part of the rung); getrandom redesigned the custom-backend registration the wasm playground uses | Maintained HTTP stack under `http.request`; RNG/entropy plumbing current. Gate: differential suite, playground harness, and an explicit seeded-stream note in the CHANGELOG | landed |

Recorded for later, not a rung: `im` 15.1 (the interpreter's O(1)
environment cloning) is unmaintained with no newer version to migrate
to — the eventual move is the maintained fork `imbl`, near-drop-in,
gated on any actual breakage or advisory rather than scheduled.

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

## The distribution campaign

The language works; this campaign is about people being able to *get*
it. The unit of work is a channel, the honest measure is "what does a
user type to install olang", and the recurring constraint is which
channels need an account that doesn't exist yet. Everything below is
prepped in-repo (workflow, formula template, packaging, runbook — see
[RELEASING.md](../RELEASING.md)); "prepped" means the next `v*` tag or
a single documented manual step lights the channel up.

| ID | Rung | What ships / how | Status |
|---|---|---|---|
| D1 | **CI release binaries** — `.github/workflows/release.yml` on every `v*` tag: `olang` + `otc` for macos-arm64, macos-x64, linux-x64 (stripped, tarred with LICENSE/README), the playground wasm, and a `SHA256SUMS`, attached to a GitHub Release automatically | prepped — fires on the next tag, no account beyond the existing one |
| D2 | **Homebrew tap** — `brew install ooyeku/olang/olang` from a `ooyeku/homebrew-olang` repo; formula template + per-release checksum procedure in [dist/homebrew/](../dist/homebrew/) | prepped — needs the tap repo created (a repo, not an account) and D1's first release to point at |
| D3 | **VS Code .vsix** — publish-ready extension (bundled, iconed, licensed); `npx vsce package` or `make dist` emits a clean installable .vsix, shareable with zero accounts | prepped — packaging verified locally |
| D4 | **Zed registry submission** — one PR to `zed-industries/extensions` (submodule + `extensions.toml` entry with `path = "editors/zed"`); grammar rev is SHA-pinned and pushed; process in [editors/zed/PUBLISHING.md](../editors/zed/PUBLISHING.md) | prepped — PR not yet opened; no account needed beyond GitHub |
| D5 | **VS Code Marketplace** — `vsce publish` under publisher `ooyeku`; steps in [editors/vscode/PUBLISHING.md](../editors/vscode/PUBLISHING.md) | blocked-on-account — needs the (free) Azure DevOps publisher created; D3 is the account-free fallback meanwhile |

## The viz campaign's findings — where the language should grow next

The five-stage visualization campaign (plot → viz grammar →
interactivity → binary bulk path → dash kit, all under 0.53.0's
[Unreleased]) was deliberately run as a stress test: build something
that competes with D3/ggplot and write down every place the language
pushed back. These are the findings, ordered by how much viz-and-data
work they would unlock. Each is a candidate lane, not a promise.

1. **List building is quadratic. Done end-to-end (0.54.0+).** `xs = xs
   + [v]` in a loop copied the whole list per iteration. It now
   appends in place on **every tier**. The bytecode tier extended the
   AddAssign fusion (the 0.51.0 string precedent): a promoted
   60,000-element build went 3807 ms → 2 ms. Then the interpreter was
   finished too — `Value::List` moved from `Arc<[Value]>` (a
   fixed-size boxed slice) to `Arc<Vec<Value>>` (which grows), a
   change that cost only a handful of edits because reads deref
   identically, and the assignment path gained the same fusion (list-
   literal rhs, `Arc::get_mut` sole-owner guard). An *unpromoted*
   interpreter build of 8,000 elements went 215 ms → 1 ms — so the
   cold path, the wasm/browser path, and the hot path are all O(n)
   now. The aliasing guard is identical across tiers (a snapshot, a
   nested list, or a captured closure is copied, never mutated) and
   pinned by interpreter-only and cross-tier differential tests. The
   only remaining cap is the interpreter's global 10,000-allocation
   safety budget, which bounds a *single* unpromoted loop regardless
   — promoted functions run on the tier without it.
2. **No vectorized Series transforms. Done (0.54.0+).** `ods.map(xs,
   "sin")` applies any of the `math.*` unary functions across a
   column in one native kernel pass — the vectorized form of
   `map(xs, (v) => math.sin(v))` with no per-element boundary
   crossing. Results are bit-identical (the kernel calls the same
   `f64` methods), nulls propagate, and domain errors match the
   scalar form. It composes with the existing elementwise arithmetic
   into full expressions (`ods.map(xs, "sin") * 2.0 + 1.0`, one kernel
   per term) and feeds `dom.draw_points` directly. Measured: a
   sin·2+1 transform over 50,000 points went **238 ms → 1 ms** (~240×,
   bit-identical checksum), and it runs in the *interpreter* kernel —
   no promotion needed — so it also lifts the browser boot cost of
   finding #3. The gallery's 50k-point curtain is now generated this
   way.
3. **The browser tier ceiling. Root cause found and fixed (0.57.0+).**
   Wasm sessions run interpreter + bytecode (the JIT emits native code
   and cannot exist there), and the suspected cost was per-lambda-call
   overhead. Profiling found the boundary conversion was already gone
   (finding #2 era) and tier-resident lambda loops already run natively —
   the real cliff was elsewhere: the tier dispatches `CallNamed` by name
   and marks any name shared by two distinct bodies **ambiguous**, never
   tiering it. Embedded packages register their *private* helpers (`viz`'s
   `col`, `opt`, `groups`, `distinct`, `fmt`, …) into the global name
   space, so any program reusing one of those common names — the gallery
   included — silently ran that function, and its hot map loop, on the
   *interpreter*. The tier now dispatches an ambiguous name **by body
   identity** (via `hof_function_id`, keyed on the body pointer, compiled
   under the function's own closure), so a name-colliding function
   promotes normally: a `use viz` program mapping `map_get` over records
   went **~1480 ms → ~170 ms (native, ~8.6×)** and **~1480 ms → ~140 ms
   (wasm, ~10×)**, correctness identical on both tiers (the right body
   still runs). Also landed: a fused `map`/`filter` arm that hoists the
   bytecode fetch and per-call checks out of the element loop (+~16%
   native on tier-resident builtin-lambda loops). Deeper structural work
   (lambda-body inlining, promoting top-level script code) remains a
   future rung.
4. **Embedded packages importing each other. Done (0.54.0+).** The
   capability was already there — `resolve_module_path` checks
   `is_embedded` before touching the filesystem, so `use ui` inside an
   embedded module resolves natively *and* in wasm — it just wasn't
   used or pinned. `dash` now does `use ui { esc }` instead of
   re-implementing the HTML escape, so the discipline lives in one
   place, and a test pins that loading `dash` transitively loads `ui`
   and the shared escape runs (verified in the browser: the board's
   dash cards render). The pattern is available to every embedded
   package as they multiply.
5. **Session state has a primitive. Done (0.55.0+).** `dom.state_set(key,
   value)` / `dom.state_get(key)` name the DOM-resident-state pattern:
   a blessed, JSON-typed, page-lifetime store (a Map or list
   round-trips; a missing key reads as Unit; not persisted — that is
   `storage_*`). The charts page's cross-filter now rides it instead
   of a hidden input, and the pattern is one call each rather than
   query-a-hidden-input-and-parse-JSON. The DOM-resident *discipline*
   stays — it is still why the back button and bookmarks are free —
   this just names it.
6. **Syntax friction. Mostly done (0.55.0+).** Unary minus (`-x`) in
   fact already worked in every position — the finding was stale. The
   `=>` of an `if` may now start a new line, so a long condition wraps
   cleanly (`if a && b && c\n    => ...`) — the friction that bit
   most. The third item, **struct fields without type annotations**,
   is deferred on purpose: the field grammar is shared across struct,
   enum, error, and anonymous-struct contexts, so making types
   optional there means a separate rule plus AST/checker/validation
   changes through the gradual-typing pipeline — real risk for a case
   the anonymous-object (`#{ .. }`) and map workaround already covers
   idiomatically.
7. **Integer-aware ticks. Done (0.55.0+).** `nice_ticks` now detects
   whole-valued data and floors a sub-1 step to 1, so a bar chart of
   counts gets 0,1,2,3 instead of 0,0.5,1,…. Float data is untouched.
   The charts page's status bars read as whole numbers live.


## The Toolsmith campaign — building and shipping real tools in olang

olang is a general-purpose language, and the recent web/data-viz work
is one domain it happens to be good at — not its identity. This
campaign deliberately balances that by making olang excellent at the
*other* end of the general-purpose spectrum: command-line tools and
systems programs. One coherent arc — **write** the tool ergonomically,
**ship** it as a fast single binary, **maintain** it with first-class
tooling — and each lane's first consumer is olang's own toolchain, the
dogfooding loop that has carried the whole project. Grouped by the
three pillars it draws from: **C** (CLI & systems), **D** (runtime &
performance), **E** (tooling & DX).

| # | Lane | Pillar | What ships | Status |
|---|---|---|---|---|
| T1 | **`cli` — declarative argument parsing** | C | An embedded package: spec-driven flags (short/long, typed, defaults, `required`, env fallback), positional args, subcommands, auto-generated `--help`/usage, and precise errors. `cli.parse(spec, argv)` → `Ok(values)` \| `Err(msg)`; `cli.help(spec)` renders usage. The pattern olang's own `main.rs` hand-rolls today | **landed (0.56.0+)** — `taskcli` runs on it |
| T2 | **`term` — the terminal toolkit** | C | What `viz` is for the browser, for the terminal: fg/bg color + bold/dim/underline with TTY auto-detection (plain when piped), aligned tables and key/value blocks, progress bars and spinners, and `prompt`/`confirm`/`select` input. The test runner's report and `bench`'s tables are the first consumers | **landed (0.56.0+)** — colors gate on TTY/`NO_COLOR`/`CLICOLOR_FORCE`; `os.is_tty()`/`os.flush()` added; `taskcli` colorizes its output |
| T3 | **Process & pipe depth** | C | Beyond the existing `os.exec`: streaming stdin/stdout, process pipelines, exit-code plumbing, and signal handling (SIGINT → graceful shutdown) for long-running tools and servers | **landed (0.57.0+)** — a new `proc` module: `spawn` returns a live `Process` handle you drive with `write`/`read_line`/`wait`/`kill` (stdout and stderr drained off-thread so a chatty child never deadlocks); `pipeline` chains stages' stdout→stdin like the shell's `a \| b \| c`, reporting per-stage exit codes. Signals land in `os`: `on_interrupt`/`interrupted`/`reset_interrupt` trap Ctrl-C for graceful shutdown. Flagship dogfood: [`examples/watch`](../examples/watch/) streams a command or runs a pipeline until Ctrl-C |
| T4 | **`olang build` — AOT to a standalone binary** | D | The headline. Ship an olang tool as one self-contained executable, no olang install required. Rung A: bundle source + runtime into a self-extracting binary. Rung B: embed precompiled bytecode (skip parse at startup). Rung C: AOT-compile hot functions via Cranelift's `ObjectModule` (the JIT already proves the codegen; this emits `.o` and links). olang itself ships this way | **Rungs A + B landed (0.56.0 / 0.57.0+)** — `olang build prog.ol -o tool` copies the runtime and appends the program with a magic footer; startup detects it and runs it with the full process argv. No linker; single-source tools using stdlib + embedded packages bundle cleanly (`examples/greet.ol`). **Rung A** appended raw source, parsed at startup. **Rung B** now appends the *parsed AST* (serialized) and the built tool deserializes it instead of re-parsing — ~20× faster than the parser, cutting ~10 ms off startup for a 400-function tool (the source travels too, only so runtime error snippets still render; old rung-A bundles still run). **Rung C (Cranelift `ObjectModule` AOT) — assessed, deferred.** Measurement re-prices it to ~zero benefit: the payoff would be removing per-invocation JIT warmup, but that warmup is unmeasurable (a tiny hot function that promotes and JIT-compiles runs 22.1 ms vs 21.9 ms with the JIT off). Worse, the JIT is *speculative* — it specializes on runtime-observed argument shapes (`[Float]`/`[Int]`) at first call, which an ahead-of-time compile has no way to reproduce — and emitting/linking a `.o` reintroduces the C linker that rungs A/B deliberately avoid. A native built binary already runs the JIT at runtime, so AOT adds nothing on top. Not re-attempted without new data (the redirected effort is embedding the used embedded packages' ASTs, which skips their per-invocation parse — a no-linker extension of rung B) |
| T5 | **Startup & the wasm tier** | D | Cold-start matters for CLI tools invoked repeatedly *and* for browser boot (viz finding #3): bytecode caching, lazy stdlib init, measured with `olang bench` | **landed (0.57.0+)** — embedded packages (`cli`, `term`, `viz`, …) now parse once per *process* into a shared AST cache, instead of once per `use` in every fresh interpreter. Since a fresh interpreter is created for every CLI run, every `olang test` file, and every playground run, this turns the repeated cold-start parse into a one-time cost: a 30-file `olang test` where each file uses `cli`+`term` dropped from ~111ms to ~44ms (2.5×), and the playground's second-and-later runs skip embedded parsing entirely. Precompiled bytecode caching remains a future rung |
| T6 | **`olang doc` — API reference generator** | E | Establish a `///` doc-comment convention (the parser keeps them on declarations), then `olang doc` extracts them to a browsable, suite-themed reference. The stdlib reference generates from source instead of being hand-maintained | **landed (0.56.0+)** — `///` item + `//!` module doc comments (already valid olang comments) → themed HTML or `--md` Markdown; the `cli`/`term` packages carry them |
| T7 | **`olang test --coverage`** | E | Line/function coverage from the existing test runner — the last piece of a credible test story that already has discovery, reporting, and `olang bench` beside it | **landed (0.57.0+)** — `--coverage` reports covered/total lines per file plus an overall figure; `--coverage-lines` also lists each file's uncovered ranges. Attributed to the file the code lives in (a new `def_file` on function values, pushed/popped across calls), so a helper exercised by another file's test is credited correctly. Runs on the interpreter tier so the AST-level hook sees every statement; the executable-line denominator is derived from the parsed AST, so a fully-exercised file reads exactly 100%. A report, never a gate |

The through-line: after this campaign you can write a polished CLI
tool in olang, ship it as a single fast binary, and document and test
it with first-class tooling — a complete general-purpose story that has
nothing to do with the web. The application-framework family (`ui` /
`viz` / `dash` and the server-side `web` batteries) continues in
parallel as one domain track among several, no longer the center of
gravity.

The campaign's flagship dogfood is
[`examples/survey`](../examples/survey/) — a codebase surveyor
(colored summaries, a per-language bar chart, aligned tables, a live
progress bar) that exercises `cli` + `term` + `fs` in one
self-contained file, builds into a standalone binary with `olang
build`, and documents itself with `olang doc`: the terminal
counterpart of the tracker web suite.

## The adaptive-engine campaign — from baseline JIT to feedback-driven

The performance campaign closed with the OVM as a clean two-tier
engine: a register bytecode VM and a type-specialized, call-graph-aware
Cranelift JIT that puts fib(30) level with Node and Bun. What it is
not, yet, is *adaptive*: compilation decisions come from a static
whitelist and one entry-guarded specialization per function, not from
observed behavior. The gap to the adaptive engines (V8, JSC, HotSpot)
is now a known, well-trodden list — profiling feedback, OSR,
polymorphic inline caches, speculation with side-exit deopt,
feedback-driven inlining, background tiering — and olang starts with
two structural advantages those engines never had: **values are
immutable and acyclic**, so shapes cannot transition and an inline
cache, once filled, is valid forever (no invalidation machinery, no
write barriers); and **the VM is register-based**, so the state
mapping that makes OSR and deopt hard in stack machines is "copy N
registers".

The Harborline soak (examples/demo) is this campaign's acceptance
harness: a long-running, allocation-heavy, idiomatic program whose
top-level tick loop the JIT currently never reaches at all.

| # | Lane | What ships | Grounding | Expected | Status |
|---|---|---|---|---|---|
| A1 | **Profiling substrate** — compact feedback vectors on `CompiledBytecode`: per-site operand-kind masks, call-target ids, branch bias, loop trip counts. Recording is one OR in the dispatch loop; semantics-free by construction | Every later lane consumes feedback; the dispatch loop already touches every site. Adaptive means observed truth replacing static inference | Negligible overhead; the prerequisite for A2–A6 | not started |
| A2 | **OSR — on-stack replacement** — back-edge counters; at threshold, compile the function with an OSR entry at the loop header and enter mid-loop by materializing the native frame from the register file | Promotion counts *calls*, so a hot loop in a once-called function — including the top level of every script, Harborline's own tick loop among them — never crosses to native. The register file makes the state map a table copy | Extends the JIT to the program class people actually write: top-level loops, long-running ticks, `main`-shaped scripts | not started |
| A3 | **Polymorphic specialization + inline caches** — compiled variants keyed by argument-kind signature (bounded, then generic bytecode fallback) through a per-function dispatch table; per-site ICs for struct field access (monomorphic direct offset → polymorphic ≤4 shapes → megamorphic helper) and for HOF call sites (`map(f)` direct-calls a monomorphic `f`) | "One specialization per function" is the engine's biggest performance cliff — any polymorphic call site falls off native entirely. Interned, non-transitioning shapes mean ICs never invalidate, skipping the hard 40% of what ICs cost JS engines; the tier's identity dispatch (viz finding #3) already laid the HOF groundwork | Removes the cliffs; steadier native residency on mixed-shape code | not started |
| A4 | **Feedback-driven inlining, then escape analysis** — inline hot monomorphic targets under a budget (generalizing the existing leaf inliner via A1's call feedback), unlocking SROA across call boundaries; lambdas inline into the native `map`/`filter` loops (pipeline fusion for free); then true escape analysis so non-escaping structs/lists in a compiled group become registers | The state-threading idiom the language itself teaches (one world record, `patch`/fold transitions — the Harborline architecture) allocates a record per transition; inline-then-scalarize turns those into register writes. This is where idiomatic olang currently pays most against the JS engines | The campaign's headline speedup on allocation-heavy idiomatic code; directly attacks the allocation pressure behind the runtime's memory heuristics | not started |
| A5 | **Speculation with side-exit deopt** — compile assuming the feedback (never-overflowed adds unchecked+guard, never-taken branches pruned, stable list element kinds kept unboxed) with deopt metadata that transfers to bytecode at the exact program point on guard failure | Today's "deopt" is a guarded helper per operation; real side exits shrink code, free the register allocator, and make checked integer arithmetic nearly free on proven-safe sites. The register VM keeps the native-state→register-file map tractable | Real percentages on numeric work; smaller, better-allocated native code | not started |
| A6 | **Background tiering + runtime co-design** — compile off-thread (Cranelift is thread-friendly) and swap code in via the function table so compilation never pauses execution; recompile at a higher budget once feedback stabilizes (baseline → optimizing); extend the borrowed-pointer discipline to refcount elision within compiled groups; expand the native builtin set (`fold`/`reduce` still bridge); revisit the parked NaN-boxing *for the JIT's internal representation only* once A2/A3 change its calculus; replace the aggressive-memory-management heuristics with real accounting (the Harborline leak hunt is the entry point) | Pauses and bridge round-trips are the residual tax; the NaN-box deferral was explicitly gated on new data, and an IC/OSR world is new data. The mid-load cache-clearing bug (0.58 fixes) showed the heuristics layer is the least principled part of the runtime | Pause-free compilation; a true two-tier JIT; the runtime's memory story on the same footing as its correctness story | not started |

**The constitutional requirement.** Speculation adds an obligation the
current harness does not cover: *deopt-path equivalence*. Before A5
lands, the suite gains stress modes — a `--jit-stress` that randomly
fails guards, forces OSR entries, and forces deopt at every side exit —
and the entire differential suite runs under them (the V8
`--stress-opt` playbook). "Tiers agree" must keep meaning something as
the effective tier count grows; every lane extends the fail-closed
ladder, never bypasses it.

Explicitly rejected for this campaign: a **tracing JIT** (method-JIT +
ICs fits the architecture and its differential-testing story; tracing
would forfeit the oracle discipline for marginal gain on this workload
profile), and a **tracing GC** (immutable acyclic values make
refcounting correct, not merely convenient — A4 and A6 remove its cost
where it matters instead).

Sequencing: A1→A2 first (reach), A3 (cliffs), A4 (speed), A5–A6
(polish into a production adaptive engine). Expected arc: A1–A2 make
the JIT reach idiomatic programs; A4 is projected to close roughly half
the remaining gap to the JS engines on allocation-heavy code, which is
where olang currently pays most.

## The openness campaign — what "Open Language" means, mechanically

olang stabilized its syntax early, on purpose. The unclaimed prize on
that decision is that *openness can be a mechanical fact, not a slogan* —
made real at three layers: open artifacts (a binary you can read and
bound), open execution (a run you can replay), and open code (the
grammar as a data format you can program against). Two layers have
shipped; the table records what remains.

| # | Lane | What ships | Status |
|---|---|---|---|
| O1 | **The transparent binary + capabilities** — a built binary carries its own source, manifest, lockfile, and checksum (`olang inspect`), and a `[capabilities]` manifest gates the effectful stdlib surface with per-dependency attenuation. You cannot ship an olang program as a black box, and nothing you install can exceed its manifest | **landed (0.58+)** — `olang inspect`, `[capabilities]`, per-dependency attenuation, `--deny`; enforced at the module boundary on the interpreter tier |
| O2 | **The Open Timeline** — `--record` logs a run's nondeterministic inputs; `olang replay` reproduces it bit-for-bit from a portable, source-embedding `.olt` trace. Deterministic execution + immutable values + a small effect boundary make record/replay sound by construction | **landed (0.58+)** — record/replay for `random`/`time`/`os`/`fs`/`http`/`crypto`, portable traces, crash capture, divergence detection; interpreter-tier, single-thread |
| O3 | **`replay --why` — value provenance** — during replay, the interpreter (the semantic oracle) carries where each value came from, answering "this 4162.85 is line 212's revenue + line 208's total, which came from …" as a chain back to the recorded inputs. Time-travel debugging's payoff: not just *what* happened, but *why this number* | not started |
| O4 | **Timeline reach** — extend the recorded set to `db` reads (a replay stub handle serving recorded query results) and `chan`/thread interleavings (a recorded schedule), so a fully database-backed or concurrent program replays end to end | not started |
| O5 | **The Open AST — the grammar as a stable public data format** — a `meta` module exposes the parsed program as ordinary olang values (`kind`-tagged maps), so linters, codemods, and import extractors are written *in olang*. The frozen syntax is what lets the AST shapes be published | **landed (0.58+)** — `meta.parse(source)` → the program as walkable node maps; `otc deps` is four lines over it; `examples/metatool` lints bare `unwrap`s. `olang check --rules` (project rules run inside the checker) is the next rung |

The through-line: **open code, open artifacts, open execution.** No
incumbent can follow all three — Python cannot freeze its AST, Go will
not embed source, and no mainstream runtime is deterministic enough to
promise replay. Each lane grows out of a decision olang already made
(early syntax stability, immutable values, a clean effect boundary)
rather than a system it would have to invent.

**Status: the campaign's three-pillar thesis is complete.** Open
artifacts (O1), open execution (O2), and open code (O5) have all landed —
no other language offers all three. The remaining lanes are enhancements,
recorded as deferrals rather than gaps: **O3 (`replay --why`)** waits on
value-provenance instrumentation through the interpreter (a large change,
reopened when time-travel debugging is prioritized); **O4 (timeline
reach)** waits on a `db`-read replay stub and a recorded thread schedule
(the latter is genuinely hard — a deterministic scheduler — and is why
Harborline's worker pool replays as a *detected divergence* today, not a
clean run); **O5's `olang check --rules`** rung wires project-authored
`meta` rules into the checker. Each reopens on demand; none blocks the
identity the campaign set out to establish.

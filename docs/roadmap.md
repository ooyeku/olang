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

1. **List building is quadratic.** `xs = xs + [v]` in a loop copies the
   whole list per iteration. **Bytecode tier: done (0.54.0+).** The
   AddAssign fusion (the 0.51.0 string precedent) now extends to lists
   — a hot list-builder appends in place when it holds the only
   reference to its Vec, with the same `Arc::get_mut` aliasing guard
   the strings use (a snapshot taken before an append is never
   mutated, self-append copies). A promoted 60,000-element build went
   3807 ms → 2 ms, an O(n²)→O(n) collapse; differential tests pin the
   aliasing cases across tiers. **Remaining: the interpreter.** Its
   list is `Value::List(Arc<[Value]>)` — a fixed-size boxed slice that
   cannot grow in place — so an *unpromoted* one-shot build (the
   gallery's boot-time generators) is still quadratic. Making the
   interpreter fast means moving to `Arc<Vec<Value>>` and mirroring
   the fusion in the assignment path — a ~220-site representation
   change, tracked as its own lane.
2. **No vectorized Series transforms.** `math.sin` over 50,000 points
   means Series → list → 50,000 lambda calls → Series. Elementwise
   Series math (`ods.map`, or `sin`/`exp`/arithmetic lifted over
   Series in the ods kernel) would make point-cloud generation as
   native as the aggregations already are — and it composes with
   `dom.draw_points`, which already takes Series directly.
3. **The browser tier ceiling.** Wasm sessions run interpreter +
   bytecode (the JIT emits native code and cannot exist there), so the
   gallery's ~250k boot-time lambda calls cost visible seconds.
   Finding 2 removes most of that particular cost; the longer lane is
   whether the bytecode tier itself can specialize hot lambda loops
   harder under wasm.
4. **Embedded packages cannot import each other.** `dash` re-implements
   `ui.esc` because `use ui` inside an embedded module doesn't
   resolve. Allowing embedded→embedded imports keeps the builtin
   packages honest as they multiply.
5. **Session state wants a primitive.** Closures capture by value, so
   every browser app stores state in the DOM (hidden inputs, data-ui
   attributes, the URL). The discipline is sound — it made the back
   button and bookmarks free — but a deliberate session-scoped store
   (even just a blessed `dom.state_get/set` over one hidden root)
   would name the pattern instead of leaving each app to rediscover
   it.
6. **Syntax friction found by writing lots of olang:** no unary minus
   (`0.0 - x` everywhere trigonometry appears); `=>` must share the
   `if`'s line, which bites exactly when conditions get long; struct
   fields require type annotations, pushing dynamic shapes to
   anonymous objects and maps. Small, but each one was hit repeatedly.
7. **Tick generation is integer-blind** (renderer, not language): count
   data gets 0.5 gridlines. A "these are integers" hint — or detecting
   whole-valued data — finishes the chart typography story.


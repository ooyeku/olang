# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

Releases before 0.23.0 predate this changelog and are not retroactively
documented.

## [Unreleased]

### Added

- **JIT strings — the lane closes.** String parameters, struct fields,
  and constants enter native code as borrowed pointers (a constant's
  Arc lives in the bytecode the JittedFn owns, so its pointer bakes
  into the code). Equality and lexicographic ordering run through a
  helper executing the VM's own comparison operators; concat is an
  allocation and follows the exact struct discipline — scratch-owned,
  straight-line only, ownership transferred once at the entry boundary
  by a string retain twin, loops driving native concat from bytecode.
  Mixed string/number `+` (formatting) stays on bytecode. Also fixed
  the same first-pass monotonicity trap for `+` operands that Return
  had: unresolved operands defer instead of narrowing irreversibly.
  59 JIT parity tests; tier output byte-identical on string kernels.
- **JIT struct construction — allocation with sound ownership.**
  MakeStruct compiles natively through a scratch-context model: every
  struct a native call builds is owned by a VM-side list for exactly
  that call, so a deopt at any point can never leak or dangle; struct
  returns transfer ownership once, at the entry boundary, where a
  retain helper resolves the borrowed pointer to an owned Arc (scratch
  allocation or entry argument — anything else deopts). A context
  pointer now threads through the whole native ABI. Two deliberate
  refusal rules keep the model where it wins, learned by measurement:
  an unbounded native loop of allocations held memory until call end
  and a cap-triggered mid-loop deopt cost more than never compiling —
  so MakeStruct compiles only in straight-line code (constructors),
  loops with struct-returning callees stay on bytecode and drive
  native constructors call by call. Also fixed a first-pass
  monotonicity bug where Return narrowed a not-yet-resolved register's
  allowed-set irreversibly. Constructor-driving kernel: 88 -> 81ms;
  55 JIT parity tests including returning-a-parameter, mixed-field
  constructors, and the allocating-loop refusal.

## [0.41.0] - 2026-08-09

### Added

- **JIT tuple extraction — multi-value native returns.** Functions
  returning tuples (up to 4 scalar elements) compile natively: MakeTuple
  becomes per-element SSA variables, Return becomes a multi-value native
  return (N elements + status), destructuring callers receive elements
  directly in registers (PatternTestTuple is statically proven and
  folds to true; ExtractElement/TupleGet read the element variables),
  and the entry wrapper writes one out-slot per element. Deopt unwinds
  through tuple producers exactly as scalars — a division by zero deep
  in a tuple-returning callee still yields the VM's canonical error.
  N-body: 56 -> 50ms, with force-style tuple producers and their
  destructuring consumers both native. 51 JIT parity tests.
- **JIT heap values: list indexing and `for` iteration — N-body
  compiles whole (332 -> 56ms).** Lists pass into native code as
  borrowed pointers, classified at specialization by element kind
  (float, int, or one struct shape); `xs[i]` and the `for`-loop
  instructions (IterLen/IterGet) compile through guarded host helpers
  that reproduce the VM's exact semantics — subscripts wrap negative
  indices, iteration does not, bounds violations deopt to the
  bytecode's canonical error. Struct elements return borrowed pointers
  into the list's own storage, valid for the synchronous call by the
  same argument as struct parameters. The register-path call site now
  extracts struct and list arguments too (it previously only handled
  int/float, which kept bytecode-to-bytecode calls off the JIT). The
  campaign's original acceptance target — N-body from 400ms to tens of
  milliseconds — is closed: 56ms, momentum conservation byte-identical.
  47 JIT parity tests including negative indexing, mixed-kind lists
  (refused, agreeing), and for-loops over struct lists.
- **JIT: the float-math builtins join the whitelist.** All 25
  `math.*` float builtins compile in JIT functions: sqrt, floor, ceil,
  and trunc as native IEEE instructions (bit-exact by definition), the
  other 21 through an imported helper that calls the VM's own
  eval_float_math — exactness by construction, not by reimplementation.
  Inference types them totally (numeric in, Float out, never deopts).
  A synthetic sqrt/sin/cos/pow kernel runs bit-identically and ~20%
  faster even when driven from bytecode. Measuring N-body exposed the
  true remaining blocker in its force loops: list indexing and tuple
  extraction ("other" in the refusal listings) — recorded on the
  roadmap as the heap-values rung. 42 JIT parity tests green.

## [0.40.0] - 2026-08-09

### Added

- **ods is part of the language — no flag, no import, no setup.** The
  `ods` cargo feature is gone: the data stack (Series, Frames, stats,
  plot) compiles into every build unconditionally, including the wasm
  playground — `ods.series([...])` works in the browser sandbox exactly
  as it does natively. Verified across every boundary this release
  built: Series round-trip the bytecode tier byte-identically, ride
  par_map worker threads as shared Arcs, and coexist with the JIT in
  the same program.


- **The `plot` namespace — ods Phase 4, charts as SVG text**
  (`docs/design/ods.md`). `plot.line`, `plot.scatter`, `plot.lines`
  (multi-series with legend), `plot.bar`, and `plot.hist` render
  complete standalone SVG documents from Series data — no rendering
  dependency, composing identically with `fs.write_file`, an `http`
  response, or the wasm playground. Defaults carry real charting
  discipline: a CVD-validated categorical palette assigned in fixed
  order (a 9th series errors rather than inventing a hue), 1-2-5 nice
  ticks, recessive grid and axes, ink-colored text, rounded data-ends
  anchored to the baseline, legends only at two or more series,
  XML-escaped labels, one y-axis always. Options ride in one map
  (`title`, `x_label`, `y_label`, `width`, `height`) and unknown keys
  refuse. Null pairs drop in xy charts; bars refuse null values. With
  this, every phase of the ods design doc is shipped: CSV → Frame →
  group_by → chart is one pipeline. Lazy evaluation was Phase 4's
  other mandate: evaluated and **deferred with a measured reopening
  gate** in the new `docs/design/ods-lazy.md` — the eager engine
  already beats NumPy on the benchmark fusion would improve.

- **Frame — ods Phase 3, the columnar table** (`docs/design/ods.md`).
  Series grew a String dtype (lexicographic comparisons, sort, gather,
  fill_null), and the `ods` namespace grew the tidyverse verb set over
  a new Frame type: `frame`, `read_csv` (type-inferring), 
  `frame_from_records` (accepts `json.parse` output — JSON arrays
  become Frames in one pipe), `select`, `with_column`, `filter`/`take`
  (shared with Series, dispatched by argument type), `sort_by`,
  `head`, `group_by` with count/sum/mean/min/max aggregations, inner
  and left hash `join`, `to_records`, and introspection. Null keys
  form their own group in `group_by` (R/Polars) but never match in
  joins (SQL); groups keep first-seen order. **B6 gate met: a 10M-row,
  1k-group sum+mean aggregates in 27.2 ms single-threaded vs 24.0 ms
  for Polars on 18 threads** — an inline Fx hasher and L1-resident
  accumulators, measured table in the design doc. `examples/dataproc`
  is rewritten on the Frame pipeline: **3.0 s → 0.06 s at 200k rows
  (50×)** with identical aggregates.

- **The `stats` namespace — ods Phase 2, statistical inference**
  (`docs/design/ods.md`). Distributions (normal, t, chi², F — pdf/cdf/
  ppf/sample, statrs-backed), `stats.describe`, pairwise-complete
  correlation and covariance, one-sample and Welch t-tests (the second
  argument picks the test: Series → two-sample, number → null mean),
  chi² goodness of fit, and `stats.lm` — OLS with coefficients, SE,
  t statistics, p-values, R², and R-style `na.omit` row handling,
  returned as an olang map of parallel Series. Sampling draws from the
  `random` module's stream, so `random.seed(k)` makes
  `stats.norm.sample(...)` reproducible. Every statistic is pinned
  against scipy/NumPy reference constants in tests, and **the B4 gate
  is met: a 1M×20 OLS fits in 36.4 ms parallel (90 ms sequential) vs
  137.8 ms for `numpy.linalg.lstsq`** — measured table and the
  faer-deferral rationale recorded in the design doc.

- **Series — ods Phase 1, the numerical engine** (`docs/design/ods.md`).
  A typed, null-aware 1-D array backed by the new `olang-ods` workspace
  crate: pure Rust kernels with no olang dependency, contiguous
  Arc-shared copy-on-write buffers, validity bitmaps, and rayon
  parallelism that respects the runtime's configured threshold. The
  `ods` namespace grows 24 functions — constructors (`series` from
  lists/ranges, `zeros`, `linspace`), null-skipping reductions (`sum
  mean var std min max quantile`), `sort argsort take filter cumsum
  dot`, and null tools — and operators are intercepted in both tiers:
  `s * 2.0 + 1.0` runs vectorized kernels, `s > 2` yields a Bool-series
  mask, `10.0 - s` broadcasts. Integer kernels are checked and error
  with the interpreter's exact wording; `==` between Series stays
  structural like every olang collection (`ods.eq`/`ods.ne` give
  elementwise masks). **Measured against NumPy at 10M elements: sum at
  parity sequentially (1.01ms vs 1.12ms) and 2.7× ahead in parallel,
  std 2.5× ahead, sort 13× ahead, null-aware mean 1.7× ahead of
  nanmean** — full table in the design doc. Pinned by 22 engine
  property tests against naive references plus 12 tier-transparency
  integration tests (tests/ods_series_test.rs).

- **JIT struct field access — the redirected P3 effort, on the safe
  Arc model.** Struct arguments pass into native code as *borrowed*
  pointers (JIT calls are synchronous; the caller's slot outlives the
  call — no refcount is ever touched), specialized per interned shape
  with field indices resolved at compile time. Every read goes through
  one guarded host helper that deopts on any surprise (same-shape
  instances may carry different field kinds in a dynamic language), so
  no layout assumption and no unsafe discipline leaks into the VM.
  Shape specs are only gathered when specializing; Ready calls extract
  a pointer and a shape id, nothing more. **A struct-field kernel:
  143ms -> 18ms (8x)**; N-body 350 -> 322ms (its inner kernel awaits
  math.sqrt in the whitelist). Four new parity tests: field kernels,
  mixed Int/Float/Bool fields, same-shape/different-kind deopt, and
  structs through native call chains.
- **P3 verdict: full NaN-boxing deferred, on the probe's own evidence.**
  The probe existed to price the rewrite, and it did: halving value
  size moved the most value-bound workload 13%, so halving again
  projects roughly another 10% — not the 2-3x the roadmap estimated
  from the old 32-byte baseline. Against that stands the cost: packing
  heap pointers means manual refcounting at every register move, the
  exact raw-pointer GcPtr scheme this codebase measured, found leaking
  every allocation, and deleted in 0.23. A ~10% win does not buy back
  that risk. The nanbox primitives remain proven and ready should the
  JIT's register model want them; the performance effort redirects to
  JIT struct field access on the safe Arc model — the actual N-body
  blocker. Recorded so the rung is not re-attempted without new data,
  like the compare+branch fusion before it.
- **NaN-boxing foundation (src/ovm/nanbox.rs).** The 8-byte packed
  value scheme for P3 proper, landed as primitives-with-proofs before
  any VM wiring: floats bit-exact with real NaNs canonicalized so no
  arithmetic result can alias a tag; 48-bit small integers ([-2^47,
  2^47) inline, wider admits NeedsHeap — settling upfront that olang's
  full-i64 integers split); 48-bit pointers; bool/unit constants.
  Pinned by boundary tests (±2^47, i64::MIN/MAX, -0.0, ±inf, payload
  NaNs including a tagged-int bit pattern) and a million-iteration
  randomized decode cross-check. Deliberately not wired: the value-model
  rewrite builds on these next, with the bit-level subtleties already
  settled where they were cheap.
- **Value-model slimming: OvmValue 32 -> 16 bytes — the P3 probe.** The
  roadmap prescribed removing the per-value header first as a cheap
  probe before NaN-boxing, and the probe paid: the ValueHeader (type
  tag, tier, lazy state) was fully dead — the tag derivable from the
  data, the tier never read, and no constructor ever produced a lazy
  value — yet its 8 bytes copied on every register move. With it gone,
  the lazy-forcing scaffolding went too, the two-slot Result variant
  packed behind one Arc, and (in the ods workstream, coordinated) the
  NativeHandle went thin, landing OvmValue at exactly 16 bytes, pinned
  by a size test. **N-body: 400ms -> ~350ms (-13%)** purely from
  layout; every differential suite byte-identical. The remaining rung
  is NaN-boxing proper (16 -> 8), now with measured grounds.
- **JIT call-graph groups: cross-function native calls.** The
  self-call-only restriction is gone. On a function's first call the
  JIT plans every function reachable through its CallFn sites, runs
  kind inference to a global fixpoint across the group (callee return
  masks feed caller registers; masks only grow, so it converges), and
  compiles the whole group with direct native-to-native calls —
  helpers, chains, and mutual recursion all stay native, each member
  entry-guarded on its own specialized signature and directly callable
  from the VM afterwards. Kind-mismatched or already-differently-
  specialized callees refuse the entry (fail-closed); the depth budget
  and deopt status propagate through the whole chain, so an overflow
  two native calls deep unwinds and re-runs on bytecode with the
  canonical error. **fib(30) split across two mutually recursive
  functions: 94ms → 5ms** — identical to single-function fib. Eight new
  parity tests: helper pipelines, 2- and 3-function cycles, mixed-kind
  chains, deopt-in-chain, kind-mismatched helpers, and depth guards
  through mutual recursion.
- **JIT type specialization: floats.** Compilation is now lazy and
  runtime-observed: a whitelisted function compiles on its first call,
  specialized to the Int/Float argument kinds that call carries, with
  the native entry guarding on exactly that signature — any other shape
  runs on bytecode (one specialization per function). Registers are
  typed i64 or f64 by the same fixpoint inference, mixed int/float
  arithmetic promotes the integer side exactly as the VM does, float
  division by zero deopts (olang errors there, not inf), NaN and IEEE
  overflow-to-inf behave identically to the VM, and float modulo is
  declined outright (fmod has no exact IR equivalent). One inference
  subtlety earned its comment: liveness flows backwards through
  register copies, or `zr = zr2` kernels misclassify their sources as
  dead. Float orbit kernels measure ~4.5× over bytecode; eleven new
  parity tests cover the float guard edges, polymorphic call sites
  (int-then-float and float-then-int), and fmod refusal.
- **The baseline JIT — the performance campaign's P2.** Hot bytecode
  compiles to native machine code via Cranelift (`src/ovm/jit.rs`),
  extending the correctness ladder unchanged: "can't compile
  identically → stay interpreted" gained "can't compile natively → stay
  on bytecode". A function qualifies when every instruction is in a
  pure integer/boolean whitelist, with operand kinds proven by a
  fixpoint inference (registers may hold mixed kinds only if nothing
  reads them — `if`-statement result slots taught that rule the hard
  way). Purity makes deopt trivial and total: non-integer arguments,
  overflow, division by zero, `i64::MIN` edges, and depth exhaustion
  all abandon the native run and re-execute on bytecode, which owns
  every error message; recursion carries a depth budget clamped to the
  VM's own limit, so runaway recursion errors identically instead of
  smashing the native stack. **fib(30): 89ms → 5ms (18×) — level with
  Node and Bun**; integer loop kernels 20–30×; the 1M-element pipeline
  27ms → 13ms. Pinned by tests/jit_test.rs (17 guard-edge parity tests)
  on top of the existing 150-test tier suite, which now runs everything
  through the JIT as well.

- **OVM modules and native values — ods Phase 0**
  (`docs/design/ods.md`). The OVM grew a module system: a Rust
  component registers stdlib-style namespaces, native value types, and
  operator behavior into *both* execution tiers through one registry
  (`src/native.rs`). Native values cross the tier boundary as one
  shared Arc — a refcount bump, never a conversion — so the
  lossy-round-trip failure mode that once kept maps and enums off the
  tier is unrepresentable for them (pinned by an Arc-identity test).
  First module: `ods` (feature `ods`, default on, pure Rust so the
  playground can enable it), registering `ods.version()` and the seam
  probes that pin the plumbing (`tests/ods_module_test.rs`). The
  numerical engine itself is Phase 1.

### Fixed

- **Two tier-vs-interpreter error divergences** the JIT parity suite
  exposed (both present in released 0.39): tiered runtime errors
  carried a doubled "Runtime error:" prefix (the tier re-wrapped an
  already-prefixed message), and `%` by zero said "Division by zero" on
  the tier where the interpreter says "Modulo by zero" — the VM now has
  a distinct ModuloByZero error with the interpreter's exact words.

### Changed

- **`otc` slimmed to the commands that earn their keep.** The toolchain
  had drifted badly behind the language: `otc run` executed files
  *without* package resolution (so any project with dependencies
  failed), `otc new` scaffolded the pre-`olang::pkg` manifest format
  the current toolchain can't read, and a whole legacy "git package
  system" (`install`/`list`/`update`/`remove`/`clean`/`info`/`config`/
  `cache`/`self`/`doctor`/`system-info`) coexisted with — and
  contradicted — `otc pkg`. otc is now six commands, each verified
  end-to-end: `new` (scaffolds the current `[package]` manifest, a
  parse-validated `src/main.ol` with a `test` block, README pointing at
  the `olang` binary), `pkg` (the real package manager: init/add/
  remove/install/tree/publish), `check`, `deps`, `unused`, and `ovm`
  (the tier-vs-interpreter divergence harness the OVM docs prescribe).

### Removed

- **`otc run` / `repl` / `test` / `build` / `version`** — running code
  is the `olang` binary's job (`olang file.ol`, `olang`, `olang test`),
  and `build` only ever copied source files into `build/`.
- **The legacy git package system** (~2,300 lines: `git_package.rs`,
  `global.rs`, `lock_file.rs`, `config.rs` and their commands),
  superseded by `otc pkg` + `olang.lock`.
- **The text-based refactor commands** (`move-fn`, `rename-fn`,
  `extract-file`, `merge-files`, `fix-imports`) and the cosmetic
  `tree` / `organize` — string-surgery on source trees predating the
  current grammar.

### Added

- **`par_map` / `par_filter` — the performance campaign's P1.** The
  parallel twins of `map` and `filter`: same arguments, same results in
  the same order, but fanned out across OS threads with no GIL. The
  design is the one the roadmap prescribed — `map` originally went
  sequential because cloning the interpreter per *element* was ruinous;
  par_map clones per *worker* (im-map environments make that cheap),
  gives each worker its own bytecode tier, and splits the list into
  contiguous chunks under `std::thread::scope`. Semantics are pinned by
  a 15-test differential suite: spawn-style snapshot isolation (the
  function never mutates the caller's environment — on any machine,
  including the single-core fallback), first-in-order error reporting
  (the error you get is the one `map` would have hit first), filter's
  keep-on-`true` rule, and fail-closed refusal in the VM (functions
  calling par_map stay interpreted). **Measured: 9–13× vs sequential
  `map` on compute-heavy kernels** (examples/parmap, which self-checks
  parallel == sequential on every run).

### Fixed

- **`spawn` threads now carry the bytecode tier.** `thread_safe_clone`
  set `bytecode_tier: None`, so every spawn worker (and http.serve
  handler thread) silently ran the pure tree-walker — losing the tier's
  ~10× on compute. Clones now get a fresh, quiet tier with the parent's
  promotion policy, and the parent's declaration knowledge (functions,
  struct shapes, trait impls/defaults, unit variants) is replayed into
  it, since workers never re-evaluate the declarations themselves.

## [0.39.0] - 2026-08-09

### Added

- **The playground: olang in the browser.** The whole language —
  interpreter, bytecode tier, and the pure stdlib — now compiles to
  `wasm32-unknown-unknown` and powers a `/playground` page on the
  website: editor, curated examples, output pane, and a 5-second
  kill-switch. Safety comes from the platform, not trust: the wasm
  instance imports exactly three host functions (two clocks and an
  entropy source) and touches nothing but its own linear memory — no
  filesystem, network, process, or DOM — and it runs inside a Web
  Worker the page terminates on timeout. The boundary is a hand-rolled
  C ABI (`olang_alloc`/`olang_run`/`olang_result_free` returning
  length-prefixed JSON), so no wasm-bindgen toolchain is involved;
  `bun run build` in website/ builds and stages the artifact
  (`playground/` cdylib crate → `static/playground/olang.wasm`).
- **A "native" cargo feature** (default-on) now carries everything a
  browser can't have: rusqlite, rustyline, reqwest/tokio/hyper, memmap2,
  hostname/whoami/dirs, rayon/num_cpus. Without it, `db`/`fs`/`http`/`os`
  aren't registered and their builtin bridges answer with a plain
  "not available in the playground" error; `should_parallelize` is
  always false and every parallel path degrades to its sequential twin.
  `src/clock.rs` is the one clock for both worlds — native re-exports
  `std::time`, the playground rebuilds `Instant`/wall-clock/sleep on the
  host imports (std's own clocks panic on wasm, as does
  `env::current_dir`, now wrapped). `print`/`println` route through
  `src/output.rs`: stdout natively, a drained capture buffer in the
  playground.

### Changed

- **Dependency prune.** Sixteen unused dependencies removed — all four
  cranelift crates, crossbeam ×2, atomic, memoffset, target-lexicon,
  wide, num-traits, dashmap, indexmap, bincode, glob, walkdir — none
  referenced by any source file. chrono drops its default `wasmbind`
  feature (wall-clock "now" goes through `src/clock.rs`).
- **Docs audit against 0.38.** The book and README now state the tier's
  real coverage (158/164 corpus functions promote; refusal lists match
  the compiler's actual `CompilationFailed` sites), the current
  benchmark standings, and the new examples (minilisp, app). New
  doc-tested example blocks: traits in the tour, tuple destructuring and
  nested functions in the language reference, `col` quantifiers/`_by`
  family and `random` sampling in the stdlib reference. Stale claims
  fixed in internals.md (file map, refusal examples), ovm.md
  (self-contradictory refusal list, retired map-builtin limitation),
  roadmap item 13 (closed into the performance campaign), stability.md
  (`time` was stable-but-unlisted).

## [0.38.0] - 2026-08-09

### Added

- **Native collection builtins.** The minilisp dogfood quantified the
  bridged-builtin tax: `map_get`/`map_set` converted the *entire*
  environment map to AST values and back on every lookup and binding,
  turning an environment-threading interpreter — a 57× workload by
  shape — into a 1.35× one. Ten builtins now run natively on the VM
  value model with zero boundary conversion: `len`, `head`, `tail`,
  `cons`, `concat`, `skip`, `map_get`, `map_set`, `map_has_key`, and
  `entries` — each mirroring the interpreter's checks in the same order
  with the same messages, including the map/struct-like duality
  (`map_set` on a struct yields a struct) and `entries`' sorted keys.
  One subtlety pinned by a test: `map_has_key` checks *presence*, not
  value — a key explicitly holding Unit still exists, so it cannot ride
  on map_get's Unit-for-missing. **minilisp's lisp-fib(17): 500ms →
  62ms — the tier's advantage on it went from 1.35× to 11×.** N-body,
  fib, and pipelines unmoved.

- **The corpus tail: self-recursive nested fns, tuples, and a live
  divergence.** Self-recursive nested `fn` declarations compile — the
  name binds to its own compiled id during the body's compile (recursion
  is a direct `CallFn`), with the body's capture parameters appended to
  each recursive call, and the escaped form carries the name so
  interpreted copies recurse through call-time self-definition. The
  capture-appending detail was found the honest way: 03_algorithms'
  `binary_search` recursed 2 arguments into a 4-parameter body at
  runtime; the fix is pinned by a test. Tuple *expressions* compile (the
  instruction existed; the compiler arm didn't), and `let` destructuring
  goes through the full pattern machinery — tuples, lists, structs,
  enums — with a non-matching let raising the interpreter's
  PatternMatchFailed. And extending `let` exposed a **live divergence in
  0.37.0**: the interpreter's `let` evaluates to the bound value
  (observable when a block ends in one — `fn f() = { let x = 5 }`
  returns 5), but the compiled form yielded Unit; nested-fn declarations
  had the same gap. Both fixed and pinned. **Corpus: promoted 152 → 158,
  rejections 14 → 6** — the six survivors are async (`spawn`, promises)
  and global assignment, all by-design refusals.

## [0.37.0] - 2026-08-09

### Added

- **The unresolved-identifier tail, diagnosed and closed.** A sweep of
  every remaining refusal found three mechanisms, fixed together:
  (1) *Native module calls* — `db.execute`, `fs.read`, `json.parse`,
  `col.frequencies`, and every other stdlib module call outside the
  math/str allowlists refused. Now the module resolves in the closure to
  its Module value, the function's existence is validated at compile
  time against the module's own field set, and the call bridges under
  the builtin value's own dispatch name — the exact name and
  implementation the interpreter calls through. (2) `map_has_key` was
  simply missing from the allowlist. (3) *Recursion through lambdas* —
  a lambda referencing its own enclosing function, or one declared later
  (the template example's mutual `render`/`render_node`), refused
  because the name is not in the declaration closure. Registered
  functions now resolve through the registry, with the closure-miss
  routed through the dependency channel so forward references register
  and retry. The examples caught a real bug in the first version of that
  fix: a compiled lambda handed to a *bridged* builtin runs interpreted,
  and without the function value carried in its closure it hit
  "Undefined variable: render" — escaped lambdas now carry
  registry-resolved functions as values, pinned by a test reproducing
  the exact template shape. **Corpus: promoted 122 → 152, rejections
  43 → 14** — the largest single-rung coverage jump of the arc; what
  remains is self-recursive nested fns (4), tuple-pattern lets, async,
  and stragglers.

- **Maps are first-class in the VM.** The last missing data type: maps
  used to be crushed into a struct shape that could not convert back,
  which kept every map-touching function and every map-returning builtin
  off the tier. `ValueData::Map` mirrors the interpreter's map exactly
  and converts losslessly both ways; `#{...}` literals compile to a
  `MakeMap` instruction with the interpreter's key-coercion rules (bad
  keys raise the same type error); `==`/`!=` compare structurally; and
  ten map builtins (`map_get`, `map_set`, `map_remove`, `map_keys`,
  `map_values`, `map_len`, `map_merge`, `map_clear`, `entries`,
  `group_by`) are allowlisted over the bridge, retiring the
  "map-returning builtins are excluded" limitation. Promoting the
  map-heavy workflow example exposed a *pre-existing* VM gap the
  differential suite now pins: the interpreter concatenates
  `String + Int/Float` in both orders and the VM errored — the missing
  arms are added with the interpreter's exact stringification. Two test
  fixtures that used maps as their "unrepresentable value" specimens now
  use a genuinely unconvertible value instead, and the
  helper-cannot-compile test moved to global assignment as its durable
  uncompilable feature (its third choice, after global reads and map
  literals each became compilable). Corpus: promoted 117 → 122,
  rejections 55 → 43; map-literal refusals to zero.

- **Template strings and nested `fn` declarations compile.** A
  `MakeTemplate` instruction builds the string with the interpreter's
  exact interpolation rules — String raw, Int/Float/Bool via
  `to_string`, everything else through the AST value's Display (structs,
  enums, and lists interpolate identically, verified byte-for-byte).
  Interpolated expressions compile in written order. A nested `fn` is
  compiled as the equivalent named closure over the current frame
  (`MakeClosure` machinery), so helpers capturing enclosing parameters
  and calling sibling nested fns promote; a self-recursive nested fn
  refuses through the bound-names rule and stays interpreted, pinned by
  a test. Corpus: promoted 112 → 117; template-string and
  nested-declaration refusals both to zero.

- **Trait method dispatch compiles.** `value.m(..)` — the top remaining
  promotion blocker in the example corpus (21 refusals) — now compiles to
  a `CallMethod` instruction when the receiver expression is pure
  (locals, field chains, literals). Dispatch mirrors the interpreter
  exactly: a struct *field* named like the method takes precedence and is
  called without self; otherwise a direct `impl` for the receiver's
  runtime type, then the type's traits in registration order for a
  default (defaults calling back into the receiver's own impl work);
  otherwise the field-access error. The runtime type-name mapping mirrors
  `Value::type_name`, so traits implemented for primitives (`impl
  Describe for Int`) dispatch too. The interpreter feeds impls, defaults,
  and type-trait facts to the tier as declarations evaluate; any change
  to the dispatch landscape invalidates compiled functions, so an `impl`
  declared after a function promoted still dispatches correctly — pinned
  by a test. Receivers with side effects refuse: the interpreter's
  dispatch fallthrough re-evaluates the receiver, which the VM will not
  replicate for effectful expressions. The language tour — whose
  trait-default method caught the previous attempt at compiling method
  calls — now runs output-identical under both tiers, and one call site
  serves Point, Circle, and Int receivers in the tests. Corpus: promoted
  108 → 112.

## [0.36.0] - 2026-08-09

### Changed

- **Immediate operands: numeric literals ride in the instruction.**
  `x + 1`, `n < 2`, `i % 2` compiled to a LoadConst into a fresh register
  plus the operation — two dispatches for a value known at compile time.
  A `BinImm` instruction carries the literal; execution reuses the same
  fast path and fallback, so overflow, division by zero, and type errors
  are byte-identical. A literal left operand fuses when the operation
  commutes or the comparison flips. Measured: fib(30) ~103ms → ~89ms,
  pipeline ~28ms → ~26ms.

  A second fusion — compare+branch pairs merged into one instruction —
  was built, measured, and **reverted**: it retired 3.5% of executed
  instructions but ran 3–4% *slower* on every benchmark. Growing the
  instruction enum perturbs the dispatch match's code layout more than
  the saved dispatches earn back, even with deliberately slimmed arms;
  the honest conclusion is that dispatch-loop layout, not instruction
  count, is now the binding constraint, and the next real lever there is
  threaded dispatch — a different project. The negative result is
  recorded here so it isn't re-attempted casually.

- **Structs have interned shapes, and field reads have inline caches.**
  A struct used to be a hash map: every `p.x` hashed the field name
  (~28% of the N-body kernel). Now a struct is an interned shape — one
  `Arc<StructShape>` per (type, field set), field names in canonical
  order with a stable id — plus a values vector in shape order. Every
  `GetField` site carries a one-entry inline cache packing (shape id →
  field index) into a single atomic word, so concurrent VMs sharing
  bytecode can never see a torn pair: a repeat read of the same shape is
  an integer compare and an array index, no hashing. Misses take a cold
  outlined path that refills the cache — outlined deliberately, because
  a first draft with the miss path inline perturbed the dispatch loop's
  code layout and cost fib/pipeline 10% each (caught by benchmarking
  non-struct workloads, recovered exactly). `MakeStruct` interns its
  shape at compile time and stores field registers in shape order
  (field expressions still evaluate in literal order). Two new tests
  pin the risky parts: a polymorphic read site rotating through shapes
  that place the same field at different indices, and shaped structs
  round-tripping the tier boundary into interpreter pattern matches.
  Measured: N-body ~485ms → ~415ms — **ahead of Ruby (468ms) and at
  parity with CPython (416ms)** — with fib and pipeline unchanged and
  the checksum bit-identical. The profile after this rung shows
  essentially all remaining N-body time in the dispatch loop itself,
  which is what the instruction-fusion rung attacks next.

- **Frames are windows on one register slab.** Every call used to swap a
  whole `ExecutionState` in and out through a frame pool, re-size its
  register and locals vectors, and reset per-call bookkeeping. The VM now
  keeps a single contiguous `Vec<OvmValue>` for all live frames: entering
  a function bumps a window past the caller's, returning restores two
  integers, and the slab only grows — stale values above the logical top
  are recycled in place with the immediate drop-skip when the next call
  claims them (the Lua register-stack design). `CallFn` goes further:
  arguments copy straight from the caller's window into the callee's, no
  intermediate buffer. The dead `locals` array and per-frame bookkeeping
  fields are gone; `LoadLocal`/`StoreLocal` (never emitted since locals
  moved to registers) now error like other unreachable instructions. Two
  new stress tests target the design's failure modes: deep recursion
  carrying heap values across windows, and native map loops stacking
  frames above a live caller. Measured: fib(30) 135ms → ~104ms, the
  pipeline benchmark 38ms → ~27ms — **past CPython (31ms) on idiomatic
  pipeline code** — N-body unchanged, checksum bit-identical.

## [0.35.0] - 2026-08-08

### Added

- **Function values are callable — and cross the tier boundary.** Two
  changes that complete the higher-order story. First, `Value::Function`
  now converts to the VM losslessly (wrapped verbatim instead of
  deep-copied into a dead-end object), so a user function passed as an
  argument no longer knocks the whole call back to the interpreter.
  Second, a new `CallValue` instruction calls whatever function value a
  register holds: parameters (`fn apply(f, x) = f(x)`), curried calls
  (`g(a)(b)`), immediately invoked lambdas, and closure aliases. Compiled
  values run in the VM; everything declined routes through the bridge
  interpreter, which owns arity errors, default parameters, and the
  "Cannot call non-function value" error. Callee resolution now mirrors
  the interpreter's scope order — locals first — fixing a latent
  divergence where a parameter named after a builtin (`fn apply(len, x) =
  len(x)`) would have called the builtin once function values could
  cross. Two course corrections along the way, both caught by the
  suites: known-but-unregistered user functions must still route through
  the tier's dependency channel (transitive/mutual recursion promotes
  both functions), and method calls (`value.m()`) must keep refusing —
  they dispatch on runtime type through trait impls, which a field read
  cannot replicate; the language tour caught that one, and a regression
  test now pins it.

- **The pure `str` module compiles, and so does `show`.** The 30 `str`
  functions (`length`, `char_at`, `split`, `parse_int`, ...) are
  allowlisted over the same bridge as `math.*`, and `show` stringifies
  through the same path as `to_string` — including structs and enums,
  verified bit-identical. String-heavy code (parsers, regex engines,
  template renderers) lives on these.

  Corpus sweep across all examples after this and the enum work:
  **promoted functions 55 → 108, rejections 125 → 64**, with rejection
  cascades collapsing from 92 to 15. The regex engine's parse pipeline
  and the algebraic-types combinators (`opt_map`, `find_first`) promote.

- **Enums are first-class in the VM.** Enum values used to be crushed
  into a struct shape with a `__variant` field that could not convert
  back, so no enum ever crossed the tier boundary and any function
  touching one stayed interpreted. The OVM now has a real enum value:
  construction (`Circle(2.0)`) compiles to `MakeEnum` with constructor
  arity checked at compile time, unit variants bake as closure constants,
  `==`/`!=` compare structurally, and conversion is lossless both ways.
  Enum-variant *patterns* compile too — including the interpreter's exact
  quirks: a bare unit-variant name is an equality match rather than a
  binding (per the declared-variant rule, with locals shadowing back to a
  binding), struct-variant payloads match positionally in field-name
  order, and a plain tuple of matching length satisfies an enum pattern
  (legacy behavior). Struct and anonymous-object patterns compile as
  well. Chosen by measurement: a sweep of the example corpus found enum
  patterns and enum-typed closure constants were the largest class of
  compilation refusals — the regex engine's 15-function cascade traced
  to a single unit variant. Direct enum/struct pattern rejections across
  the corpus: 14 → 0. The differential suite caught one real divergence
  during development (a unit-variant pattern compiled as a binding,
  swallowing every arm below it) — fixed by mirroring the interpreter's
  declared-variant rule, with a new-variant declaration invalidating
  previously compiled functions the same way struct redeclaration does.

## [0.34.0] - 2026-08-08

### Changed

- **Struct construction compiles.** `Body { x: 1.0, ... }` and anonymous
  objects now build natively in the VM via a `MakeStruct` instruction.
  Literals validate against the declared field set at *compile* time with
  the interpreter's exact rules — unknown type, missing field, and
  surprise field all refuse compilation, so the function stays
  interpreted and the interpreter raises its own error. The interpreter
  feeds struct declarations to the tier as they evaluate; a type
  redeclared with a *different* field set invalidates every compiled
  function (their baked validation could go stale) and its literals
  refuse from then on, keeping the interpreter's live registry the
  authority — pinned by a redeclaration test. With this, **every function
  in the N-body example promotes**: 6 promoted, 0 rejected, 7 tier
  crossings for the whole run (127M instructions, all inside the VM),
  ~525ms → ~453ms. The whole pipeline arc closes: `make_bodies`, `step`
  (struct-building capturing lambda inside `map`), and `simulate` were
  the last holdouts.

- **Capturing lambdas compile.** A lambda that captures the enclosing
  function's *runtime* state — a parameter, a local — no longer refuses
  the whole function. The lambda body compiles once as a standalone
  function whose trailing parameters are the captured names, and a new
  `MakeClosure` instruction snapshots the capture registers at the lambda
  expression — the interpreter's own capture-by-value moment, so a local
  reassigned after the lambda is created is not seen, and a lambda built
  in a loop captures each iteration's value (both pinned by tests). The
  resulting closure value runs natively through `map`/`filter` (captures
  appended to each element call), converts losslessly to an interpreter
  function at the bridge (fold/reduce and friends agree), and survives
  escaping — a compiled function can return the closure to interpreted
  code and it behaves identically. Still refused: a lambda capturing a
  name the enclosing function binds only later, and calling a
  lambda-valued expression directly. Measured over 1M elements:
  `map((x) => x * f + 1)` with `f` a runtime parameter went 417ms → 39ms
  (10.7×) — the last slow row of the pipeline table. All four pipeline
  forms now land within 1.6× of the explicit loop.

- **Functions that read globals compile now.** A free identifier that
  resolves in the function's own closure bakes as a constant — sound
  because the interpreter installs exactly that closure as the call
  environment, and closures are declaration-time snapshots (a global
  mutated after the function's declaration is not seen; pinned by a new
  test in both tiers before building on it). Function values bake in the
  lossless `AstFunction` representation, so a named function passed to
  `map`/`filter` now compiles end-to-end through the native higher-order
  path. This retires one of the tier's oldest limitations: the N-body
  example's softening constant, inlined as a literal specifically because
  a module-level binding kept the kernels off the tier, is a named
  constant again — kernels still promote, checksum bit-identical.
  Measured over 1M elements: `map` with a named function 192ms → 31ms
  (6.2×). Names absent from the closure (the interpreter would fall back
  to the caller's runtime scope) still refuse, as do assignments to
  globals. Tests updated: two asserted the old refusal as the expected
  behavior; new tests pin snapshot semantics exactly and prove baked
  function values survive redefinition of their name.

- **The higher-order builtins loop inside the VM.** `map`, `filter`, and
  `sum` previously bridged out of the VM on every call — the list and
  function converted to AST values, the interpreter looped, and every
  element crossed the tier boundary individually. When the collection is a
  list and the function argument compiles, the loop now runs natively:
  one VM `execute()` per element, no conversion anywhere, and a mapped
  list flows into `sum` without ever leaving the OVM value model.
  Function *values* (lambda constants, functions passed by value) compile
  on first sight, cached by body-allocation identity with Weak-upgrade
  validation; a new compiler mode resolves their free identifiers from
  the value's own attached closure — sound because the interpreter
  installs exactly that closure as the call environment (and closures are
  snapshots: a global mutated after declaration is not seen, verified).
  Anything declined — arity mismatch, default parameters, trait bounds,
  uncompilable body, non-list collection, `sum` past the parallel
  threshold — still bridges to the interpreter, which remains the
  semantic authority; a loop never falls back mid-flight, so element
  errors propagate exactly as the interpreter would. Interpreter quirks
  are mirrored, not "fixed": `filter` keeps an element only on exact
  `Boolean(true)`, `sum` promotes int→float mid-list and overflow-checks
  integers. Measured over 1M elements: `xs |> map((x) => x + 1) |> sum`
  224ms → 31ms (7.2×), within 1.3× of the equivalent explicit loop. Five
  new tier-agreement tests cover results, the exact-Boolean filter, string
  and nested maps, sum edge cases (empty, mixed, overflow, non-numeric),
  and mid-map error propagation.

- **The tier stopped allocating a string per call.** `try_call` cloned the
  callee's name into a fresh `String` on every call of every named
  function, purely to probe three maps with it — the callee is the
  caller's value and independent of the tier, so the lookups now borrow.
  Measured: `map` with a named compiled function 154ms → 145ms over 1M
  elements.

- **The bytecode tier is boxed, so a call no longer memcpys it.** The tier
  owns the whole VM — compiler, bytecode caches, execution state, frame and
  argument pools — which is 1,424 bytes, and it was stored inline in the
  interpreter. `call_function` moves the tier out of the interpreter and
  back on every call (so the tier can borrow the interpreter for builtins),
  which meant about 2.8 KB of memcpy per interpreted function call. Behind
  a `Box` it is two pointer moves. This costs nothing for code already
  running inside the VM, and pays where olang is currently weakest —
  pipelines and higher-order builtins, which call back through the
  interpreter once per element. Measured over 1M elements: `map` with a
  named compiled function 192ms → 154ms, `map` with a capturing lambda
  417ms → 382ms.

## [0.33.0] - 2026-08-08

### Added

- **`--ovm-stats` reports instructions retired.** The VM has always counted
  them; nothing surfaced the number, so the tier's actual workload was
  invisible and per-instruction cost could not be measured without
  instrumenting a build. (The N-body example retires 126.8M bytecode
  instructions.) Rejection *reasons* were already available under
  `--verbose`.
- **The bytecode tier compiles unary `-` and `!`.** The VM has had `Neg`
  and `Not` instructions, and an `execute_unary_op` matching the
  interpreter exactly (`checked_neg` with the same overflow message, `-x`
  on floats, `!b` on booleans, a type error otherwise), since it was
  written — the compiler simply never emitted them. A single `-x`
  anywhere in a function therefore refused the whole function and left it
  on the interpreter. Found while auditing the tier tests: the test named
  `indexing_promotes_and_agrees` promoted nothing, because its `xs[-1]`
  made it uncompilable.

### Changed

- **Value copies and drops stopped going out of line.** `clone_simple` is
  a 20-variant match, far past what LLVM will inline, so *every* register
  copy became a call into it — it was the second-hottest symbol in a
  profile. Immediates (which fill nearly every register in a numeric
  kernel) now clone through a small inlinable fast path with the heap
  variants behind `clone_heap`. The mirror image showed up next:
  `drop_in_place<ValueData>` was ~25% of VM samples, because overwriting a
  register runs the old value's drop glue. Writes and frame resets now
  skip that glue when the value being replaced owns no heap payload,
  decided by matching on the data itself rather than the header tag so it
  cannot disagree with reality. Measured: N-body ~680ms → ~530ms (8.0M
  interactions/sec), fib(30) ~218ms → ~140ms. Two new tier tests cover the
  skip decision: a register cycled through strings, ints, lists and floats
  on every pass, and heap values returned from pooled frames.

- **Profile-guided dispatch-loop cuts: phantom errors and double dispatch.**
  A CPU profile of the N-body run showed ~15% of VM time in
  `drop_in_place<BytecodeError>` — the register/constant accessors used
  eager `ok_or(...)`, constructing (and immediately dropping) an error
  value on every *successful* access, and `BytecodeError`'s String
  variants give it real drop glue. All hot-path accessors now construct
  errors lazily. Second find: every arithmetic/comparison instruction
  dispatched twice — the instruction match already knew the op, then
  `execute_binary_op` re-matched it. A `binary_fast` helper inlined with a
  constant op collapses each numeric instruction to a type check plus the
  operation; mixed-type operands, overflow, and division by zero fall back
  to `execute_binary_op`, which keeps owning the error messages. Measured:
  N-body ~940ms → ~680ms (6.2M interactions/sec), fib(30) ~267ms → ~218ms,
  checksum bit-identical. Cumulative for the whole performance arc:
  N-body 2,511ms → 680ms (3.7×), fib(30) 736ms → 218ms (3.4×).

- **The dispatch loop stopped paying for hashing and conversion it didn't
  need.** Three more measured per-operation taxes removed: (1) `GetField`
  cloned the field-name `Arc<String>` and the whole object value (two
  refcount round-trips) per read — it now borrows both, and struct field
  maps hash with FNV-1a instead of SipHash (field names are short
  identifiers from program text; the maps are tiny). (2) `math` builtins
  ran through the full interpreter bridge on every call: OVM→AST argument
  conversion, name-string dispatch, a result round-trip check, and
  AST→OVM conversion back. The 25 pure float functions (`sqrt`, `sin`,
  `pow`, `atan2`, ...) now compile to a `CallBuiltin` instruction resolved
  by id at compile time and evaluate as plain `f64` ops — non-numeric
  arguments still take the interpreter path so errors stay identical, and
  a new tier-agreement test locks every table entry (Float and Integer
  arguments) to the interpreter's results. (3) `Call*` argument marshaling
  reused pooled buffers instead of allocating per call. Measured: N-body
  1,427ms → ~940ms (3.6M interactions/sec, checksum bit-identical),
  fib(30) 285ms → ~267ms.

- **VM-internal calls resolve at compile time.** A call to a user function
  compiled to `CallNamed`, which re-hashed the function's *name* against
  the registry on every execution — fib(30)'s 2.7 million recursive calls
  each paid a string hash, then a hash-map bytecode lookup behind an
  RwLock. The compiler already validates every callee against its registry,
  so it now emits a new `CallFn` instruction carrying the resolved
  `FunctionId` (user functions checked before builtins, preserving
  shadowing semantics; builtins keep `CallNamed`). `execute()` fetches
  bytecode from a lock-free per-VM table indexed directly by id — sound
  because ids come from a global monotonic counter and are never reused —
  falling back to the shared RwLock cache only on first touch. Argument
  marshaling pre-sizes its vector, and the dispatch loop counts
  instructions in a local flushed once per call instead of writing a stats
  field per instruction. Measured: fib(30) 340ms → 285ms, the boundary
  microbench 65ms → 33ms per 100k calls (still flat in argument size),
  N-body 1,461ms → 1,427ms.

- **The tier call boundary no longer taxes every call.** Three per-call
  costs measured and removed: (1) arguments were deep-converted
  Value→OvmValue on every call — a function taking a 1,000-element list
  paid a full conversion walk per call (measured: identical work cost
  0.4µs with a 1-element argument and 8.5µs with 1,000). Arc-backed lists
  now hit a pointer-identity conversion cache validated by Weak upgrade,
  so an unchanged list converts once; per-call cost is flat in argument
  size. (2) The compiled bytecode (whole instruction vector + constants)
  was deep-cloned out of an RwLock per call; the cache now stores
  Arc<CompiledBytecode>. (3) Every call allocated a fresh register/local
  frame and took two Instant::now() samples for an unused statistic;
  frames are pooled with capacity retained and per-call timing is gone.
  Measured on the repository benchmarks: N-body 2,511ms → 1,461ms (1.7×),
  fib(30) 736ms → 340ms (2.2×), boundary microbench flat at 65ms for
  1/100/1,000-element arguments (was 83/246/1,701ms). Three new
  tier-agreement tests cover cache identity: repeated same-list calls,
  fresh lists in a loop (allocation reuse), and derivative lists.

## [0.32.0] - 2026-08-08

### Added

- **The bytecode tier compiles `math.*` calls.** The pure `math` module
  functions (`sqrt`, `sin`, `cos`, `pow`, `floor`, `abs`, `atan2`, ... — 30
  in all) are now compilable builtins, recognized when the module is a bare
  identifier (a local shadowing `math` is still field access, not the
  builtin). Before this, a single `math.sqrt` in a hot loop kept the whole
  function on the interpreter — so any real numeric kernel missed the tier.
- **`examples/nbody/`** — an N-body gravity simulation: `Body` structs whose
  force kernels (`accel_x`/`accel_y`) read five fields per interaction in an
  O(n²) loop and call `math.sqrt`, exactly the field-access + math shape the
  tier now accelerates. It times itself and reports throughput; a `test`
  block locks determinism and momentum conservation. **Measured 9× on the
  bytecode tier** (120 bodies × 150 steps: 23.2s interpreter → 2.6s tier).

### Changed

- **`olang test` runs through the bytecode tier**, exactly as `olang <file>`
  does by default, so tests execute at production speed and exercise the tier
  that actually ships. (The N-body example's self-check went from 24s to
  2.7s.)

## [0.31.0] - 2026-08-08

### Added

- **`examples/loadtest/`** — a self-contained HTTP load test, the server and
  its concurrent client fleet in one olang program. It boots a SQLite-backed
  API in a spawned task, fans client workers out across `spawn` threads
  (each firing a burst of requests and timing them), merges per-worker stats
  after `await`, and reports throughput and latency. A `test` block checks
  the load-test invariant on every run: the server's own recorded hit count
  equals the clients' successes exactly (2xx + the deliberate-500 route),
  with zero unexpected failures — so `http.serve`'s worker pool provably
  loses no writes under concurrent load. Measured ~17–19k req/s locally;
  1,200 requests through a 4-worker pool stay perfectly consistent.
- A Rust integration test (`concurrent_load_writes_are_not_lost`) pins the
  same invariant: 16 client threads × 40 writes against an 8-worker pool
  sharing one SQLite connection, asserting the final row count is exact.
  (Dogfooding `http.serve` under genuine concurrent load, driven by olang's
  own `spawn`/`await` rather than an external tool, found no bugs — the 0.29
  worker pool and 0.30 failure-as-a-value semantics compose correctly.)
- **The bytecode tier compiles field access and indexing.** `p.x` and
  `xs[i]` (negatives count from the end) now lower to dedicated
  name-based `GetField` and `IndexGet` instructions with the interpreter's
  exact semantics (missing field, out-of-bounds, and type errors all
  match). Structs, objects, and parsed JSON objects also became
  tier-representable — a value round-trips through the OVM when every field
  does — so functions that read fields off a struct argument finally
  *promote and run in bytecode* instead of falling back forever. Measured
  ~1.7× on a field-access-heavy hot loop (920ms → 540ms). Since almost
  every real function touches a field, this widens promotion from numeric
  kernels to ordinary code. Five new tier-agreement tests cover field
  access, indexing (incl. negative and OOB), nested field+index, and a
  struct returned from a compiled function round-tripping identically.

### Fixed

- **Trait-method dispatch stays correct under promotion.** Impl methods
  were never registered with the tier's ambiguity guard, so once struct
  arguments became tier-representable, a trait method like `area` — defined
  by several types, dispatched by receiver — could compile one type's body
  and run it for all receivers (`self.s` on a shape with no `s` field). Impl
  methods are now noted to the tier: multiple same-named impls mark the name
  ambiguous and keep it on the interpreter (correct dispatch), while a
  single-impl method still promotes. Found immediately by the tier-agreement
  doc test.

## [0.30.0] - 2026-08-08

### Added

- **`examples/pargrep/`** — parallel code search dogfooding the real
  `spawn`: the coordinator walks a tree, deals files to spawned worker
  threads (fs + re + str running concurrently), merges after `await`, and
  prints sequential-vs-parallel timings (~2× on the examples tree). A
  `test` block asserts the parallel result equals the sequential one on
  every run.

### Fixed

- **Awaiting a rejected promise yields `Err(e)` instead of aborting.**
  A failed `spawn` task, `Promise.reject`, or a rejecting `all`/`race`
  produced a hard runtime error nothing could catch — one failed worker
  killed the whole program, making failure handling impossible. `await`
  now returns the rejection as an ordinary `Err` value, composing with
  `match`, `unwrap_or`, `?`, and `try`/`catch`. Found immediately by
  dogfooding a parallel searcher's per-task recovery.
- **`try` blocks pass non-Result values through.** `try { await task }
  catch (e) { fallback }` failed with a type error whenever the task
  *succeeded* (successful `await` yields the bare value, not `Ok`). A try
  block's non-Result value now passes through unchanged; Ok/Err behavior
  is untouched.

## [0.29.0] - 2026-08-08

The consolidation release, addressing an external review's bottom line item
by item (the plan lives in `docs/roadmap.md`).

### Added

- **Bounded concurrent `http.serve`.** Independent connections run on a
  configurable worker pool (host parallelism by default), with a bounded
  queue, `503` overload responses, keep-alive request caps, socket timeouts,
  and `remote_addr` on requests. The notes-server example adds configurable
  JSON Lines access logging and its dependency-free benchmark reports actual
  average in-flight load.
- **`spawn` runs on a real OS thread** (C2). Previously it evaluated its
  expression eagerly and wrapped a resolved promise — concurrency
  decoration. Now `spawn expr` evaluates on a background thread against a
  thread-safe interpreter clone (the same worker pattern `http.serve`
  uses): spawn returns immediately, `await` joins, results are memoized so
  cloned promises can be awaited repeatedly, a failing task rejects, and
  capture is by value like closures. Three 100ms tasks awaited together
  take ~100ms — verified by tests in both tiers. The deterministic
  deadline model for `Promise.delay` is unchanged.

### Changed

- **The interpreter is split into semantic components** (C5). The
  5,628-line interpreter.rs is now src/interpreter/ — core evaluation
  (2,629 lines) plus modules, errors, module_cache, ops, patterns,
  environment, and spawn_registry components. Move-only; public paths
  preserved via re-exports; the full suite is the equivalence proof.
- **One capability authority** (C6). The README no longer contradicts the
  book (it had still claimed "v0.23, experimental", "11 modules", and a
  placeholder HTTP server). It is now a short, accurate overview whose
  code runs in CI, and docs/stability.md is explicitly the authoritative
  capability statement.
- **Struct construction is validated** (C1). A struct literal must name a
  declared struct type and supply exactly the declared field names —
  missing, surprise, and undeclared-type constructions are now errors
  naming the problem. Field VALUES stay dynamic; the book states the
  position plainly: declarations fix shape, not types. Anonymous objects
  remain free-form. (Previously `NeverDeclared { surprise: 42 }`
  constructed happily.)
- **The lazy facade is honest** (C4). Every "lazy" path (map, filter,
  take, skip, concat, and `lazy()` itself) built a thunk and immediately
  forced it — eager semantics at extra cost. All are now direct eager
  implementations; `lazy(v)`/`force(v)` keep their observable behavior
  (identity) and the stdlib reference says so. The fabricated
  memory-pressure machinery (estimates derived from a stack address) and
  the module-cache wipe after 50-element range maps are deleted;
  src/internal/ (2,687 lines) is gone entirely.
- **Numeric parallelism follows configuration** (C4). `sum` hard-coded
  parallel execution at ≥5 elements, bypassing the configured threshold;
  it now respects it, and the default threshold rises from 10 to 10,000 —
  small lists are always cheaper sequentially.

### Fixed

- **MVS resolution verifies requirements** (C3). The resolver kept an
  already-selected version whenever it was merely ≥ a new requirement's
  floor, never checking that it *satisfies* the requirement — `^1.0`
  alongside a selected 2.0.0 passed silently, and the `Conflict` error was
  unreachable. Selection now re-verifies every requirement in-loop and at
  fixpoint; incompatible ranges produce `Conflict` naming the package and
  requirements, in both resolution orders.

## [0.28.0] - 2026-08-08

### Added

- **`olang test` — the test runner.** Discovers every `.ol` file containing
  a top-level `test` block (recursively), runs each file in a fresh
  interpreter from its own directory with its package dependencies, and
  reports every block's outcome. Under the runner a failing block records
  its failure and later blocks still run (inline `olang <file>` behavior is
  unchanged: a failing assertion aborts). Files without test blocks are not
  executed. Non-zero exit on any failure; setup errors outside a block are
  reported as file errors.
- **`olang fmt` — the formatter.** Conservative whitespace hygiene applied
  only outside multi-line strings: CRLF→LF, trailing whitespace stripped,
  leading tabs → 4 spaces, blank-line runs collapsed, exactly one final
  newline. It never re-indents or reflows, and it refuses to write unless
  the formatted source re-parses to an AST identical to the original —
  unparseable or meaning-changing results are skipped and reported.
  `--check` reports and exits non-zero for CI. Applied to the examples tree
  (19 files cleaned; everything still green).
- **The book gains a [Tooling](docs/tooling.md) chapter** covering both
  tools and the examples harness; the roadmap's tooling track is landed.

## [0.27.0] - 2026-08-08

### Added

- **Roadmap Tier 4** — the two big lanes:
  - **The OVM compiles `&&`/`||` as conditional jumps.** Any function
    containing a logical operator previously fell back to the tree-walker
    permanently — and post-hardening, that was most interesting functions.
    The lowering short-circuits on *exactly* `Boolean(false)`/`Boolean(true)`
    (via a never-erring pattern-equality test, not truthiness), so the
    interpreter's strict semantics are preserved bit-for-bit — including
    `0 && true` being a type error. Guard-heavy hot loops measure ~2.2×
    faster. Fixing this surfaced a latent divergence: the OVM's And/Or
    *instructions* were JS-style truthiness coercions returning operands;
    they are now strict Boolean, matching the interpreter.
  - **`http.serve` keep-alive.** Connections are persistent per HTTP/1.1:
    the server loops requests on one connection until the client closes,
    sends `Connection: close`, idles past the timeout, or hits a
    per-connection cap. Responses advertise `Connection: keep-alive`
    accordingly. Verified by an integration test running two requests over
    one TCP stream and by curl's connection reuse against the notes API.
- **Roadmap Tier 3** — stdlib gaps:
  - **`time` module** — `time.now_ms()` (epoch milliseconds),
    `time.monotonic_ms()` (a clock that never goes backwards, for
    durations), and `time.sleep(ms)`.
  - **`fs.walk(dir)`** — every file below a directory, recursive and sorted;
    **`fs.glob(pattern)`** — files matching a pattern where `*` matches
    within a segment, `?` one character, and `**` spans segments.
  - **`os.exec` options** — an optional third argument
    `#{ "cwd": ..., "stdin": ..., "env": #{...} }` (any subset); the 2-arg
    form is unchanged, unknown options are rejected.
  - **`str.fmt(template, ...)`** — fills `{}` placeholders in display form
    (strings bare); `{{`/`}}` escape literal braces; placeholder/argument
    count mismatches are errors, not silence.
  - **db transactions** — `db.begin` / `db.commit` / `db.rollback` over a
    connection handle.
  - `examples/run_all.ol` now dogfoods the tier: per-run `cwd` via the exec
    option (no more chdir dance) and sub-second timing via
    `time.monotonic_ms` + `str.fmt`.
- **Roadmap Tier 2** — control flow and errors:
  - **`return expr`** — exits the nearest function (or lambda) with the
    value; bare `return` yields Unit; escapes loops within the function.
    Implemented as an unwind signal caught at the call boundary, like `?`.
    `return` is now a reserved keyword. Top-level `return` is an error.
  - **`break value`** — a loop becomes an expression: `loop { ... break x }`
    evaluates to `x` (works in `while` and `for` too). Bare `break` keeps
    its existing behavior. The bytecode tier refuses functions using
    `break value` or `return` (they stay on the interpreter) — never
    diverging; verified identical results across `--no-ovm`, default, and
    `--ovm-tier=1`.
  - **`error` declarations have semantics** (previously reserved-but-parsed):
    bare variants are singleton values, payload variants
    (`Invalid: { msg: String }`) become constructors taking the fields
    positionally. The values are ordinary enums, so `Err(NotFound)` and
    `match r { Err(Invalid(m)) => ... }` compose with the existing Result
    and pattern machinery. Moved from Reserved to Stable in the book.
- **Roadmap Tier 1** (`docs/roadmap.md` tracks the plan; every item grounded
  in dogfooding friction):
  - **`show(v)`** — display rendering: strings bare, everything else as
    `to_string` (which keeps its repr form, strings quoted).
  - **`entries(m)`** — the `(key, value)` tuples of a map or any struct-like
    value, sorted by key for deterministic iteration.
  - **Write-side struct-likeness** — `map_set`/`map_remove` now accept
    objects, structs, and parsed JSON, returning a new value of the same
    kind; the write side finally matches the read side.
  - **`for` tuple destructuring** — `for (i, x) in enumerate(xs)` and
    `for (k, v) in entries(m)` bind element parts directly (desugars to a
    tuple `let`, so both tiers agree by construction).
  - **Strings iterate** — `for ch in "abc"` yields 1-character strings, by
    character (not byte), in both execution tiers.
- **`http.serve` is a real HTTP server.** It was a placeholder that returned
  "server would start" without ever calling the handler. It now binds
  `127.0.0.1:port` (port 0 picks a free one and reports it), parses HTTP/1.1
  requests (method, path, decoded query map, lowercased header map, body),
  and calls the olang handler per request — a bare string return is a 200, a
  `http.response`/`response_with_headers` struct is honored. Handler errors
  become 500s, malformed requests 400s, and the server keeps serving through
  both. Sequential and blocking by design; integration-tested over real TCP.
- **`examples/webserver/`** — a notes JSON API on `http.serve`: a router with
  `:id` path parameters dispatching handlers over a SQLite store that
  persists across requests (the handler closes over the connection).
  `run_all.ol` skips long-running servers with a visible note.
- **`examples/markdown/`** — a markdown→HTML converter: a block parser
  (headings, lists, blockquotes, fenced code, rules, paragraphs) over a
  recursive inline span renderer (`code`, bold, italic, links, escaping).
  Handles unclosed markers gracefully and converts the repository's own
  README; a `test` block self-checks the conversion contract on every run.

### Fixed

- **Assertion arguments may span lines.** The `assert_eq`/`assert_ne`/
  `assert`/`assert_true`/`assert_false` grammar rules had no newline
  handling between arguments, unlike every other call form — so a
  multi-line `assert_eq(...)` silently fell out of the assertion grammar
  and parsed as a call to an undefined `assert_eq` function, failing at
  runtime with "Undefined variable". Found by the markdown converter's
  self-check block.
- **`json.stringify` serializes maps.** `#{ ... }` literals and db rows are
  `Map` values; stringify rejected them ("Cannot convert Map to JSON") while
  handling structs and objects. Maps now serialize as JSON objects — found
  dogfooding the webserver, where `db.query` rows feed straight into a JSON
  response.
- **`http.response_with_headers` accepts map headers.** It required a struct,
  but object field names cannot contain `-`, making `Content-Type`
  unwritable. A map literal (`#{ "Content-Type": "application/json" }`) now
  works.

## [0.26.0] - 2026-08-07

### Added

- **The olang book.** Comprehensive documentation under `docs/`: a hub
  (`docs/README.md`), a tour, a complete language reference
  (`docs/language.md`, superseding `docs/syntax.md`), a complete stdlib
  reference (`docs/stdlib.md`), a contributor internals guide
  (`docs/internals.md`), and a stability policy (`docs/stability.md`)
  committing the documented surface to remain stable while development
  focuses on optimization, new features, and stability. Every olang code
  block in the book is executed by `doc_examples_test` in CI.
- **`let mut` is real syntax.** Previously `let mut x = 1` mis-parsed as two
  statements, leaving a stray `mut` binding. `mut` is now a contextual
  keyword in `let`: it marks intent (all bindings are assignable) and the
  statement parses as one declaration.
- **`os.exec(program, args)` runs external programs.** Returns
  `Result<{ code, stdout, stderr }, Error>` — the exit code and captured
  output on success, an `Err` only when the program can't be launched. This
  lets an olang program drive other programs.
- **`examples/run_all.ol`** — a test harness that discovers every standalone
  script and every package (`main.ol`) under `examples/` and runs each in its
  own `olang` subprocess (from the program's own directory), captures output,
  and prints a pass/fail summary with a non-zero exit on any failure. Built on
  `os.exec` plus `fs.list_dir` — a self-hosted way to check the examples stay
  green.
- **Programs receive command-line arguments.** `olang script.ol a b c` now
  passes `a b c` through to the program; `os.args()` returns
  `[script, a, b, c]` (previously it returned the interpreter's own argv and
  the CLI rejected trailing args). This makes real CLIs writable in olang.
- **`share trait` and `share impl`.** Traits and impl blocks can now carry the
  `share` keyword for symmetry with `share fn`/`type`/`let`. (Traits already
  register globally, so a plain `trait` in a module also reaches consumers;
  `share` is now simply accepted rather than a parse error.)
- **`else if` chains.** Conditionals can chain with `else if COND => ...`
  instead of only nesting as `else => if ...`. Both tiers.
- **List concatenation with `+`.** `[1, 2] + [3, 4]` yields `[1, 2, 3, 4]`,
  matching how `+` already joins strings — so building a list up element by
  element (`acc = acc + [x]`) works. Both the interpreter and the OVM tier.
- **Enum variant constructors cross the module boundary.** Importing a shared
  enum type (`use m { Node }`), a variant by name (`use m { Text }`), or `*`
  now brings the variant constructors into scope, so a shared ADT is
  constructible in the importer and not only matchable. There is no qualified
  `Type::Variant` form, so the bare constructor had to travel with the import.
- **Multi-line `use` import lists.** The names inside `use m { ... }` may span
  lines and end with a trailing comma.
- **`examples/jsonschema/`** — a JSON Schema validator: the schema and document
  are both parsed JSON, and validation recursively walks them, collecting a
  pathed error (`$.address.zip`) per violated keyword (type, enum, required,
  properties, items, and the min/max/length bounds). Found no new bugs — the
  JSON, map-accessor, recursion, and comparison paths were already hardened by
  earlier rounds.
- **`examples/regex/`** — a backtracking regex engine: a recursive-descent
  parser compiles a pattern to a recursive `Re` AST, and a continuation-passing
  matcher walks it. Supports `. * + ? | ( )`, character classes, anchors, and
  `\d \w \s`, with `find`/`find_all`/`matches`.
- **`examples/parser/`** — a parser combinator library (parsers as
  `(input, pos) -> result` functions, composed by higher-order combinators)
  with a recursive arithmetic grammar that parses and evaluates in one pass.
- **`examples/workflow/`** — a data-driven state machine engine with guards
  and actions as first-class function values, running two machines (an
  expense-approval pipeline and a cyclic turnstile) on one engine.
- **`examples/template/`** — a mustache-style template engine self-hosted in
  olang (lexer, parser over a shared `Node` ADT, renderer), driven by a JSON
  context. Exercises all four fixes above.
- **`examples/dataproc/`** — a CSV→aggregate→JSON data pipeline: reads sales
  rows with `csv`, types and aggregates them, emits a `json` report, then
  reads it back and selects fields by a runtime key.
- **`examples/scheduler/`** — concurrent fan-out and timeouts with
  `async`/`await` and `Promise.all`/`race`.
- **`examples/loganalyzer/`** — a second dogfooded package: parses
  application logs with `re` capture groups, aggregates by level and route
  with `col` + pipelines, and reads files with `fs`. Handles malformed
  lines, missing/empty files, and 2000-line logs. Found no new bugs — the
  taskcli round had already hardened the shared package/args/import paths.
- **`examples/taskcli/`** — a persistent task tracker as a real multi-file
  package (SQLite store, a `col`+pipeline reporting module, a domain module,
  and CLI dispatch on `os.args()`), built by dogfooding the language.

### Fixed

- **A forgotten `=` in `let` is a parse error.** `let scores #{ ... }`
  silently parsed as an uninitialized `let scores` (bound to Unit) plus a
  stray expression statement — in the REPL the echoed map made the binding
  look successful. An uninitialized `let` must now end its statement; the
  error points at the unexpected token with a `let name = value` hint.
- **`?` propagates the `Err` instead of aborting.** `expr?` on an `Err`
  raised a runtime error ("Tried to unwrap error") rather than returning the
  `Err` from the enclosing function — making `?` unusable on its main path.
  It now unwinds to the function-call boundary and the function returns that
  `Err` to its caller. Found while writing the language reference: the
  documented behavior is now the real one.
- **Lists, tuples, maps, and structs compare with `==`/`!=`.** Structural
  equality existed only for enums; `[1, 2] == [1, 2]` was "Invalid binary
  operation". All compound values now compare structurally, matching the
  equality already used by pattern matching. Found while writing the
  language reference.
- **`&&` and `||` short-circuit.** The right operand was always evaluated, so
  a guard like `x != 0 && y / x > 0` still divided by zero and a backtracking
  matcher's progress guard recursed forever. The right operand now runs only
  when the left doesn't already settle the result. Found by dogfooding a regex
  engine.
- **Identifier patterns bind instead of misfiring as variant tests.** A bare
  name in a pattern was treated as a unit-variant equality test whenever it
  merely resolved to a unit variant *in scope* — so a binding sub-pattern like
  `b` in `Node(a, b)` silently failed to match when some `b` already in scope
  held a unit variant. A name is now a variant test only when it is a
  *declared* variant; otherwise it binds. Found by dogfooding a regex engine
  (`Concat(a, b)` with `b` bound to the `$` anchor node).
- **Strings compare with `<`, `<=`, `>`, `>=` in the interpreter.** Only
  `==`/`!=` worked; the ordering operators raised "Invalid binary operation"
  even though the bytecode tier accepted them — so the result depended on
  whether a function had been promoted. The interpreter now orders strings
  lexicographically by Unicode scalar value, matching the tier. Found by
  dogfooding a parser combinator library, where `c >= "0" && c <= "9"` is
  everywhere.
- **Same-named functions in different modules no longer collide.** The
  bytecode tier keyed compiled functions by bare name, so once two modules
  each defined (say) `initial_state`, the first one compiled ran for *both*
  namespaces — and a caller's private helper could resolve to another
  module's helper. With the default tier threshold of 1, this struck on the
  first call. A name bound to two distinct function bodies is now left to the
  interpreter, which resolves each through its own closure. Found by
  dogfooding a state machine engine running two machines at once.
- **`join` renders string elements without quotes.** `join(["a", "b"], ",")`
  is now `a,b`, not `"a","b"` — it had used each value's debug form. Matches
  `str.join`.
- **`map_get`/`map_has_key`/`map_keys`/`map_values` read any struct-like
  value.** They previously accepted only a `Map`, so a parsed JSON object,
  an anonymous object, or a named struct could be read by dot access but not
  by a runtime key. They now view a map or any struct's fields uniformly —
  found by dogfooding a data processor, where selecting a JSON column by a
  variable is the natural pattern.
- **`Promise.all` / `Promise.race` now compose.** Three async bugs found by
  dogfooding a concurrency program: (1) they accept any list expression, not
  just a literal `[...]`, so `Promise.all(jobs)` with a variable works;
  (2) they resolve *pending* delay-promises instead of erroring with "async
  scheduling not implemented", sleeping once until the latest deadline
  (`all`) or the earliest (`race`) — so fan-out over delays takes the max
  latency, not the sum; (3) async lambdas with no parameters (`async () => x`)
  parse (same zero-parameter bug that had bitten regular lambdas).
- **Sub-directory module imports resolve the named file.** `use lib.greet`
  loaded a generated directory index instead of `lib/greet.ol`, and
  generating that index *wrote a file into the user's source tree* on import.
  The directory auto-index feature is removed: directory imports resolve a
  user-written `index.ol`/`mod.ol`, and named modules resolve the named file.

## [0.25.0] - 2026-08-06

### Added

- **`mathx` — the pure subset of `math`, self-hosted in olang.** A second
  embedded module mirroring the parts of `math` that need no native float
  intrinsics: `abs`, `sign`, `min`, `max`, `gcd`, `lcm`, `factorial`,
  `floor`, `ceil`, `trunc`, `round`, `fract`, `radians`, `degrees`, a
  Newton's-method `sqrt`, and the constants `PI`/`E`/`TAU`. The
  transcendentals (`sin`, `cos`, `ln`, `exp`, ...) stay native, behind the
  FFI boundary. Differential-tested against `math`: exact equality for the
  integer/rational/rounding operations (all matched on the first run,
  including negative-number rounding semantics), and a tolerance for `sqrt`
  (Newton's method converges to within ~1 ulp of the correctly-rounded
  native sqrt). `:help mathx` lists it and points at `:help math.<fn>`.


- **Embedded olang-source stdlib modules ("builtin packages").** A stdlib
  module can now be written in olang and compiled into the binary via
  `include_str!`, resolving like a package (`use colx { ... }`) with no file
  on disk. This proves the pure (non-FFI) parts of the stdlib can be
  self-hosted while native primitives (fs, http, crypto, db, ...) stay Rust.
  The first embedded module, `colx`, mirrors part of the native `col`
  collections module and is differential-tested against it — every
  olang-implemented function must agree with its Rust counterpart. (That
  test immediately caught a bug: `take_while` relied on mutating a
  closure-captured flag, which olang closures don't propagate; it now
  threads state through the fold accumulator purely.)


- **Package manager.** olang projects are now packages: an `olang.toml`
  manifest plus a directory of `.ol` files. Dependencies come as source in
  three forms — local `path`, `git` (tag/rev/branch), and registry version
  requirements — and resolve through the same `use` mechanism as local
  modules (a `use` whose first segment is a dependency name resolves inside
  that dependency). `otc pkg` gains `init`, `add`, `remove`, `install`,
  `tree`, and `publish`. Running a file inside a package resolves its
  dependencies automatically.
  - **Lockfile** (`olang.lock`) pins every dependency exactly (git SHA /
    registry version / path) with a sha256 source checksum; `--frozen`
    fails CI if resolution would drift.
  - **Version resolution** uses Minimal Version Selection (Go-modules
    style): the lowest version satisfying every requirement across the
    graph — reproducible, no backtracking, explicit upgrades.
  - **Registry** is a git repo of TOML index entries (name@version -> git
    source + checksum); no hosted service required. Fetched sources are
    content-addressed by commit under `~/.olang/cache`.
  - New `pkg` module in the library (`manifest`, `lock`, `cache`,
    `registry`, `resolver`) and `OLANG_REGISTRY` / `OLANG_CACHE` env vars.

- **Trait bounds.** A generic function can constrain its type parameters:
  `fn describe<T: Show>(item: T)` accepts only arguments whose type
  implements `Show`, checked at the call boundary — a value that doesn't
  fails there with a clear message (`argument 1 of type Circle does not
  implement trait Show`) instead of deep inside the body. Multiple bounds
  with `+` (`<T: Show + Ord>`) require all of them; the error names the
  specific unmet trait. Unbounded generics (`fn identity<T>(x: T)`) are
  unaffected. New `implements(value, "Trait")` builtin reports membership.
  Bounds are enforced at runtime — olang stays dynamically typed — which
  keeps this independent of the (opt-in) static type checker.

- **Traits with runtime dispatch.** `trait Show { fn show(self) -> String }`
  declares a set of methods (with optional default bodies); `impl Show for
  Point { ... }` provides them for a type. A method call `value.method(args)`
  dispatches on the runtime type of `value`, passed as `self` — single-
  dispatch polymorphism (protocols / interfaces). Default methods, methods
  with arguments, and traits over both structs and enums all work; struct
  fields take precedence over methods of the same name. This gives olang
  interface-based polymorphism without static typing.

- **`07_algebraic_types.ol`** example — a recursive expression-tree
  evaluator, a generic Option combinator library, and a binary search tree,
  all built on the enum sum types that now construct. A demonstration that
  olang expresses real algebraic data types.

- **Enums construct at runtime.** `type Color = enum { Red, RGB(Int, Int, Int) }`
  now binds its variants: unit variants (`Red`) are values, payload variants
  (`RGB(1, 2, 3)`) are constructor callables that build an `Enum` value. This
  makes olang's algebraic data types real — previously `enum` declarations
  parsed and could be pattern-matched, but the variants were undefined at
  runtime, so examples worked around them with string-tagged structs. Enum
  values compare structurally, `typeof` returns the enum name, and generic
  enums (`Maybe<T>`) construct for any payload (type parameters are erased at
  runtime).

- **`db` module** — an embedded SQLite database via the bundled `rusqlite`
  (compiled from source, so the single-binary story holds — no system
  dependency). `db.open` (`:memory:` or a file path), `db.execute` (rows
  affected), `db.query` (list of column-name maps), `db.query_one`, and
  `db.close`. Query parameters bind to `?` placeholders — the safe path is
  the default. Open connections live in a global registry keyed by id (the
  pattern the RNG and promise registries already use), returned as a
  `Connection` handle. SQLite types map to olang as NULL→unit,
  INTEGER→Int, REAL→Float, TEXT→String.
- **`06_database.ol`** example — a SQLite-backed task tracker: schema,
  parameterized seeding, filtered queries, group-by aggregates, and
  updates. Registered in the example harness and documented.

- **`col` module** — the first higher-order stdlib module: `min_by`,
  `max_by`, `sort_by`, `count_by`, `frequencies`, `partition`, `flat_map`,
  `take_while`, `drop_while`, `all`, `any`, `sum_by`, `unique`, `window`,
  `zip_with`, `last`. These take function arguments and call back into the
  interpreter (dispatched through `BuiltinFunctions::call`, which has the
  interpreter), so a stdlib module can now be higher-order. The core
  operations (`map`, `filter`, `fold`, `group_by`, ...) remain top-level
  builtins.
- **REPL completeness for all stdlib modules**: TAB completion now
  enumerates every registered module, so `str.`, `re.`, `col.`, and every
  existing `module.function` complete automatically (a new module is picked
  up with no manual list). Help docs added for all `str` (29), `re` (8),
  and `col` (16) functions — `:help str.trim`, `:help col.min_by`, and the
  `String`/`Regex`/`Collections` category listings all resolve.

- **`str` module** — string manipulation: case conversion, trim, split/join,
  replace, substring, pad, index/search, repeat, char access, lines/words,
  and `parse_int`/`parse_float`. All indexing is by Unicode character.
- **`re` module** — regular expressions backed by the `regex` crate:
  `is_match`, `find`, `find_all`, `captures`, `split`, `replace`,
  `replace_all`, and `is_valid`. A malformed pattern is a recoverable `Err`,
  not a crash.
- **`05_text_processing.ol`** example — word frequency, log parsing via
  regex captures, email extraction, phone validation, a template engine,
  record parsing, and slugification.

- A curated example suite (`examples/01_language_tour.ol` through
  `04_stdlib_showcase.ol`, indexed in `examples/README.md`) that runs top to
  bottom and prints computed results — a language tour, a real analytics
  pipeline, classic algorithms, and a stdlib showcase (SHA-256/HMAC, bcrypt,
  RSA sign/verify, math, calendar arithmetic, JSON). Verified in CI by
  `tests/example_programs_test.rs`, so the examples cannot rot. Retired four
  stale examples tied to deleted subsystems (three SIMD demos, one GC memory
  test).
- Formatting and clippy are blocking CI gates: the tree is rustfmt-clean and
  clippy-clean at zero warnings (`-D warnings`). The one deliberate allowance
  is `clippy::result_large_err` — boxing the interpreter's error enum is a
  worthwhile future refactor tracked in `src/lib.rs`. The whole-tree reformat
  commit is listed in `.git-blame-ignore-revs`.

- **Documentation examples are tested in CI** (`tests/doc_examples_test.rs`):
  every ```olang block in README.md and docs/syntax.md must parse and run
  (```olang no-run blocks — needing files, network, or modules — must at
  least parse). 41 of the 61 documented examples were broken when this was
  introduced; all are fixed. docs/syntax.md gains sections for character
  literals, `Promise.all`/`Promise.race`/`spawn`, and the reserved-word
  list.

### Changed

- **`colx` now mirrors all of `col`.** The embedded olang collections module
  gained the remaining nine functions (`min_by`, `max_by`, `sort_by`,
  `drop_while`, `flat_map`, `frequencies`, `last`, `window`, `zip_with`),
  reaching full parity with the native `col` module — including an olang
  insertion sort for `sort_by`. Every function is differential-tested
  against its Rust counterpart (16 functions, with key-function, stability,
  and truncation cases), and a parity test asserts the mirror stays
  complete.


- **Stdlib return-type convention unified**: total operations (which cannot
  fail for correctly-typed input) return bare values; fallible operations
  return an olang `Result`. Concretely: `crypto` hashes/HMAC/`secure_compare`
  and `base64.encode` now return values directly instead of `Ok(...)`, and
  the `dates` module now returns `Result` for every date-string operation
  (parsing, arithmetic, formatting, component extraction) — a malformed date
  is a recoverable `Err` instead of aborting the program. Total date
  operations (`now`, `today`, `is_leap_year`, `days_in_month`) stay bare.
  `docs/syntax.md` documents the convention and both new modules.

- **Fast by default** — the bytecode tier is now enabled by default,
  promoting eligible functions on their *first* call (previously opt-in via
  `--ovm-tier` with a 50-call threshold, which also meant a hot loop inside
  a function called once never promoted). The slow "OVM routing" layer is
  no longer the default execution path — it measured ~70% slower than the
  plain interpreter, meaning the out-of-the-box configuration was the
  slowest available. Zero-flag results: fib(32) 4.5 s → 2.1 s; a
  3M-iteration loop 1.30 s → 0.29 s. `--no-ovm` remains the pure
  tree-walking escape hatch; `--ovm-tier=N` raises the threshold.
- **Slot resolution** — identifiers in function and lambda bodies are
  resolved to frame-slot indices once at declaration time (`src/resolve.rs`),
  so the hot path indexes into the frame instead of probing names. Every
  resolved reference keeps its name and the runtime verifies the slot before
  using it, falling back to a normal lookup on mismatch — a stale static
  model (e.g. a `let` inside a conditional shifting later slots) costs
  speed, never correctness. 5–9% across call- and loop-heavy benchmarks.
  Stale bytecode-cache files from older AST shapes are now removed quietly
  instead of warning on every startup.
- **Frame-based environments** — environments created for function calls,
  loop bodies, match arms, and catch blocks are now frames: every binding
  they create (`let`, loop variables, match bindings) lives in the probed
  locals vector with in-place rebinding, instead of being hashed into the
  persistent map. Only the root environment keeps map storage, so top-level
  definitions still persist across REPL inputs and snapshot into closures in
  O(1). A 3M-iteration loop with three `let`s per iteration runs ~33%
  faster; `for` iteration ~19% faster.
- **Interpreter call path rewritten** — function calls are ~50x cheaper
  (~0.65 µs, down from ~33 µs). Three changes: the call environment adopts
  the function's closure as a shared persistent map in O(1) instead of
  copying every entry; body evaluation swaps environments instead of
  overlaying and then restoring every closure entry around each call; and
  call-frame bindings (parameters, the function's own name) live in a
  probed vector instead of being hashed into the persistent map.
  `recursive_fib_13` benchmark: 24.5 ms → 0.31 ms. This collapses the
  bytecode tier's relative advantage on call-heavy code (its headline
  numbers were largely measuring interpreter overhead); loop-heavy code
  still benefits ~7.5x from the tier.

### Fixed

- **Newlines now separate statements.** Previously a newline was plain
  whitespace, so a parenthesized expression on the line after a statement
  attached as a call to the previous value — `let a = [1,2]` then `(a, a)`
  parsed as `[1,2](a, a)`, breaking tuple returns and any bare `(...)`
  statement. Newlines are now statement separators, admitted explicitly at
  continuation points (operator chains, pipelines, bracketed lists, match
  arms, bodies after `=`), so multi-line pipelines and calls are unchanged.
  Trailing commas in list literals are now allowed too, consistent with
  maps/structs/enums.


- **Unit-variant patterns dispatch correctly.** A bare name in a `match`
  arm that resolves to a unit enum variant is now matched as a variant, not
  bound as a catch-all — previously the first such arm (`North => ...`)
  captured every case.

- `to_int` and `to_float` are callable as plain identifiers. They were
  handled by the builtin dispatcher but never registered in the builtin
  function map, so `to_float(x)` failed with "undefined variable" outside
  the bytecode tier — a real bug surfaced while writing the example
  programs.

- Zero-parameter lambdas (`() => 3`) parse — the grammar always allowed
  them, but the parser discarded the body when no parameter list was
  present.
- Documentation no longer claims struct field shorthand, intersection
  types, union type declarations, or error-variant payloads — none of
  which parse. `Promise.delay`'s documented argument order was backwards
  (it is `Promise.delay(value, ms)`).


- Lambda capture no longer depends on `im::HashMap::union`, whose collision
  bias depends on which map is larger — a captured variable could resolve to
  an ancestor call frame's stale value, sending recursion through a captured
  lambda into infinite loops.

### Removed

- **~12,000 lines of dead execution machinery**: the `OlangVirtualMachine`
  routing layer and `ovm_integration` (the pre-0.24 default path, measured
  ~70% slower than the plain interpreter), `ovm_repl`, the placeholder JIT
  module, the tracing-GC/region-allocator remnants, and the pipeline, SIMD,
  lazy, fusion, and adaptive engines — none wired into execution. The OVM
  directory now contains exactly what runs: the bytecode VM, the tier, the
  value model, and safepoint flags. REPL commands `:ovm`, `:stats`, and
  `:memory` now report bytecode-tier statistics; `:gc` is gone (values are
  reference-counted; there is nothing to force).

## [0.23.0] - 2026-08-04

First stabilized release after a substantial correctness and performance
overhaul. The tree-walking interpreter remains the semantics reference; the
opt-in bytecode tier (`--ovm-tier`) is now honest, tested, and fast.

### Added

- **Bytecode tier** (`--ovm-tier[=N]`): hot user functions are promoted to a
  register-based bytecode VM after N calls (default 50), with transparent
  fallback — anything the VM cannot compile keeps running on the interpreter,
  so enabling the tier can never change program behavior. Covers arithmetic,
  comparisons, control flow, `while`/`for` loops with `break`/`continue`,
  `match` with guards, or-patterns, ranges, and destructuring of `Ok`/`Err`,
  lists (including `...rest`), and tuples, `Ok`/`Err` construction, pipelines,
  transitive and mutual recursion, 43 delegated builtins, and lambdas whose
  free variables resolve through the enclosing function's declaration-time
  closure. Measured speedups: ~120x on recursive calls, ~44x on match-heavy
  loops, ~26x on map/filter/fold pipelines.
- **Differential test suites** (110 tests) asserting the VM and the tier are
  observationally identical to the interpreter, including error cases.
- **REPL shell integration**: `:sh <cmd>` / `!<cmd>` run shell commands
  through `$SHELL`; `:cd`, `:pwd`, `:ls` navigate the filesystem (`cd`
  changes the REPL's own working directory). TAB completion for REPL
  commands, file paths (including inside string literals), and
  function/variable identifiers.
- **Interpreter benchmarks** (`cargo bench`) and a tier comparison harness
  (`cargo run --release --example tier_compare`).
- Continuous integration on Linux and macOS.

### Changed

- **Evaluation is by reference**: the interpreter no longer deep-clones AST
  subtrees on every loop iteration and function call.
- **OVM value model is reference-counted** (`Arc` payloads). The previous
  raw-pointer scheme leaked every allocation and its tracing GC operated on
  an object model the allocator never created; `gc.rs`/`memory.rs` are now
  accounting layers only.
- `--ovm-tier` takes its value with `=` (`--ovm-tier=10`) so the bare flag
  doesn't swallow the following filename.
- Documentation (README, `docs/ovm.md`) rewritten to state explicitly what is
  and is not implemented, with measured performance numbers.

### Fixed

- `break` and `continue` are real control flow; previously they were fake
  runtime errors that aborted the program (`loop { break }` could never
  terminate).
- Assignments inside `for` loop bodies write through to the enclosing scope
  instead of being silently discarded.
- Pipeline partial application (`5 |> add(3)`) resolves against the remaining
  parameters instead of erroring.
- Integer arithmetic is checked everywhere; overflow raises a runtime error
  instead of panicking (debug) or wrapping (release).
- `sum` no longer double-counts when a list mixes integers and floats on the
  parallel path; `map_filtered` no longer swaps its predicate and mapper;
  eager `concat` no longer returns a nested pair; `len` is character-based to
  match string indexing.
- Parser: identifiers with keyword prefixes (`match_count`) parse; numeric
  literals with trailing junk are rejected instead of silently splitting;
  string patterns process escape sequences; postfix operators after
  `Ok(...)`/`Err(...)` are kept; error snippets truncate on character
  boundaries instead of panicking on multibyte text.
- `Promise.delay` is awaitable: the promise carries its deadline and `await`
  sleeps out the remainder. Previously every await of a delayed promise
  errored and each delay leaked an async-runtime registry entry.
- Static analysis: exiting a scope restores the outer-scope entries it
  shadowed instead of deleting them; `let`-bound variables get tracking
  entries so usage counting and unused-variable reports include them;
  guarded patterns no longer count toward match exhaustiveness; enum
  exhaustiveness consults the actual declaration.
- Type checker: recursive functions type-check (the function is registered
  before its body is checked); `Unknown` types satisfy type-class constraints
  (gradual typing).
- REPL: no longer panics on multibyte strings in `:env` previews or
  did-you-mean suggestions.
- `dates` extractors accept the module's own `now()` output; negative
  `add_days` and large `add_months` are correct; `math.min`/`max` preserve
  integer precision; `csv.to_json`/`from_json` use a real JSON serializer;
  `random` functions reject negative counts and handle inclusive ranges
  correctly.
- Thread safety: `Expr` uses `Arc` internally and the unsound
  `unsafe impl Send/Sync for Value` is removed.

### Removed

- The placeholder JIT tier, which returned constant integers cast to
  pointers and dereferenced them (undefined behavior). Cranelift remains a
  dependency for an eventual real implementation.
- Broken bytecode optimization passes (dead-code elimination that deleted
  live control flow, register renaming that rewrote three opcodes,
  control-flow optimization that treated label ids as addresses).

### Security

- `crypto.sign_data`/`crypto.verify_signature` are real RSA PKCS#1 v1.5
  signatures over SHA-256. The previous implementation ignored the key
  arguments entirely and produced forgeable output any party could
  construct.
- `crypto.decrypt_aes` accepts the output of `crypto.encrypt_aes` directly
  (the embedded nonce is parsed rather than requiring manual hex slicing).

[Unreleased]: https://github.com/ooyeku/olang/compare/v0.23.0...HEAD
[0.23.0]: https://github.com/ooyeku/olang/releases/tag/v0.23.0

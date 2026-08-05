# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

Releases before 0.23.0 predate this changelog and are not retroactively
documented.

## [Unreleased]

### Added

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

### Fixed

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

### Changed

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

- Lambda capture no longer depends on `im::HashMap::union`, whose collision
  bias depends on which map is larger — a captured variable could resolve to
  an ancestor call frame's stale value, sending recursion through a captured
  lambda into infinite loops.

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

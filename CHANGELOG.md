# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

Releases before 0.23.0 predate this changelog and are not retroactively
documented.

## [Unreleased]

### Changed

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

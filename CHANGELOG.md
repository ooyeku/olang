# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

Releases before 0.23.0 predate this changelog and are not retroactively
documented.

## [Unreleased]

### Added

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

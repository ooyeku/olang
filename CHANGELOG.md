# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

Releases before 0.23.0 predate this changelog and are not retroactively
documented.

## [Unreleased]

### Changed

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

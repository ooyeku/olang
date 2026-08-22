# olang examples

Runnable programs demonstrating the language. Every entry below is a real
program that does real work and prints computed results — nothing is faked,
and nothing is a stub. Run any of them directly:

```bash
olang examples/metatool/main.ol
cd examples/survey && olang main.ol summary ../../src
```

To run **every** example at once — each standalone script and each package —
use the self-hosted harness, which launches each program in its own `olang`
subprocess and reports a pass/fail summary:

```bash
cd examples && olang run_all.ol
```

Three of them are servers that block forever by design (`app/`, `ledger/`,
`webserver/`), so the harness skips them — naming each skip rather than
passing over it silently — and a Rust integration test boots each one
instead. Everything else runs, including `oshell/`, which notices that its
stdin is not a terminal and exits cleanly.

**Where to start.** [`demo/`](demo/) is the guided tour: Harborline is one
system that reaches for nearly every part of the language at once, and
[the Harborline chapter](../docs/demo.md) walks through it. To learn one
feature at a time, [the book](../docs/README.md) teaches each with its own
runnable snippets. `tests/example_programs_test.rs` pins the flagship —
every `demo/` module must parse, and a bounded soak run must finish with
its invariants intact — so a language change that breaks it is a CI
failure rather than something a reader discovers.

## The programs

The first three demonstrate the package manager and the capability model
(see [docs/packages.md](../docs/packages.md) and
[docs/openness.md](../docs/openness.md)); the rest are complete
programs grouped by nothing in particular — read the one whose subject
you care about.

- [`packages/geometry/`](packages/geometry/) — a library package: shapes,
  areas, and 2D point math, exposing a public API with `share`
- [`packages/demo/`](packages/demo/) — depends on `geometry` by path and
  imports it with `use geometry { ... }`. Running it
  (`olang examples/packages/demo/main.ol`) resolves the dependency with no
  separate install step
- [`capabilities/`](capabilities/) — per-dependency capability attenuation:
  the same app runs twice against a malicious `analytics` dependency, and the
  guarded manifest (`fs = false` for the dependency) blocks the backdoor the
  unguarded one lets through
- [`demo/`](demo/) — Harborline, the flagship soak test and the closest
  thing to a guided tour: vessels arrive on a seeded random stream, a
  depth-and-tide-aware scheduler assigns berths, a crew of real threads
  unloads them over channels, and tariffs come from expression-tree pricing
  rules — every movement recorded in SQLite, every simulated day folded into
  a template-rendered digest whose HMAC chain (RSA-signed) makes the history
  tamper-evident. State is one world record that `tick()` maps to the next,
  so there is no hidden mutation; invariants are asserted every simulated day
  and a violation fails the process. Designed to run for hours in bounded
  memory — `olang main.ol --ticks 48 --fast` for two days in a second.
  Walked through in [the Harborline chapter](../docs/demo.md)
- [`taskcli/`](taskcli/) — a persistent task tracker as a real multi-file
  package: SQLite storage (`lib/store.ol`), a reporting module using `col`
  and pipelines (`lib/report.ol`), a domain module (`lib/model.ol`), and a
  CLI dispatch on `os.args()` (`main.ol`)
- [`loganalyzer/`](loganalyzer/) — parses application logs with `re` capture
  groups (`lib/parse.ol`), aggregates by level and route with `col` +
  pipelines (`lib/stats.ol`), and reads files with `fs` from `os.args()`
  (`main.ol`). Handles malformed lines, missing/empty files, and 2000-line
  logs
- [`scheduler/`](scheduler/) — concurrent fan-out and timeouts on real
  threads: four `spawn`ed fetches run concurrently and are collected with
  `map(task.join)` (total time = the slowest, not the sum), then each is
  given a budget with `task.join_timeout`
- [`meterflow/`](meterflow/) — a multi-source ETL: JSON-lines telemetry
  streamed in bounded memory, cleaned, aggregated across chunks, joined to
  CSV dimensions, cached in the native columnar format, and charted.
- [`dataproc/`](dataproc/) — a CSV→Frame→aggregate→JSON pipeline on the ods
  data stack: `ods.read_csv` infers column types, revenue is one vectorized
  column multiply, revenue-by-region is a `group_by`, then a `json` report
  is emitted, read back, and selected by runtime key — a parsed JSON object
  reads through the same `map_*` accessors as a map. (The records-and-fold
  version of this pipeline ran 50× slower at 200k rows — the measurement
  that opens [the Data Stack chapter](../docs/ods.md), which teaches every
  verb this program uses.)
- [`timeseries/`](timeseries/) — analysis over *ordered* data, the
  sequence counterpart to `dataproc`'s bag: where each row's answer
  depends on the rows around it. A service's daily latency over four
  weeks (generated deterministically, no file) is run through the window
  verbs — a 7-day moving average with `rolling`, day-over-day change with
  `shift`, a running worst-case with `cum_max`, and a severity `rank` —
  each added as a column, so the incident hidden in the noise surfaces by
  ordering rather than by a hand-picked threshold. A `group_by` by weekday
  sits alongside to show the sequence and bag views answering different
  questions over one frame. `test` blocks pin the windows
- [`derives/`](derives/) — declaration-driven code generation from an
  *imported macro library* (the macro system's dogfood): one `type Contact`
  declaration, and `@json`, `@builder`, and `@arbitrary` derive its
  serializer, its builder API, and a generated test suite — four `test`
  blocks stamped out at expansion time, discovered by `olang test` like
  handwritten ones. Delete a field and every derived artifact follows;
  `olang expand main.ol` shows exactly what the derives produced
- [`template/`](template/) — a mustache-style template engine self-hosted in
  olang: a lexer, a parser building a nested node tree over a shared `Node`
  ADT (`lib/ast.ol`), and a renderer walking it against a JSON context.
  Supports `{{ dotted.paths }}`, `{{#each}}`, `{{#if}}`, and `{{ . }}` for the
  current item
- [`workflow/`](workflow/) — a data-driven state machine engine (`lib/machine.ol`)
  where transitions carry guards and actions as first-class function values.
  Two machines run on it: an expense-approval pipeline that branches on amount
  (`lib/expense.ol`) and a cyclic turnstile (`lib/turnstile.ol`); context
  evolves immutably and each run yields an audit trail
- [`parser/`](parser/) — a parser combinator library (`lib/combinators.ol`):
  parsers are `(input, pos) -> result` functions, composed by higher-order
  combinators (`seq`, `alt`, `many`, `chainl1`, `between`, …). On top of it,
  `lib/calc.ol` is a recursive arithmetic grammar that parses and evaluates
  expressions in one pass, honoring precedence and parentheses
- [`regex/`](regex/) — a backtracking regex engine: a recursive-descent parser
  (`lib/parse.ol`) compiles a pattern into a recursive `Re` AST (`lib/ast.ol`),
  and a continuation-passing matcher (`lib/matcher.ol`) walks it with
  backtracking. Supports `. * + ? | ( )`, `[a-z]`/`[^…]` classes, `^`/`$`
  anchors, and `\d \w \s` escapes, with `find` / `find_all` / `matches`
- [`jsonschema/`](jsonschema/) — a JSON Schema validator (`lib/validate.ol`):
  the schema and document are both parsed JSON, and validation is a recursive
  walk collecting a pathed error (`$.address.zip`) per violated keyword —
  `type`, `enum`, `required`, `properties`, `items`, and the min/max/length
  bounds. Reads `schema.json` and `data/*.json` from disk
- [`webserver/`](webserver/) — a notes JSON API on `http.serve`: a router
  with `:id` path parameters (`lib/router.ol`) dispatching to handlers over
  a SQLite store that persists across requests. GET/POST/DELETE, JSON in and
  out, 404/400 handling. Long-running — `run_all.ol` skips it; it is
  integration-tested by `tests/http_serve_test.rs`
- [`loadtest/`](loadtest/) — a self-contained HTTP load test: it boots the
  API (SQLite-backed) in a spawned task, fans a fleet of client workers out
  across `spawn` threads (each firing a burst and timing it), merges the
  per-worker stats through `map(task.join)`, prints throughput/latency, and a `test`
  block asserts the server's own hit count equals the clients' successes
  exactly — proving `http.serve`'s worker pool loses no writes under
  concurrent load
- [`nbody/`](nbody/) — an N-body gravity simulation as a bytecode-tier
  benchmark: `Body` structs whose O(n^2) force kernels read fields and call
  `math.sqrt` in a hot loop — exactly what the tier accelerates. Times itself
  and reports throughput; ~44× faster on the tier than the interpreter — the
  widest tier gap in the corpus, which is what a float-and-field hot loop
  hands the JIT. A
  `test` block locks determinism and momentum conservation
- [`oshell/`](oshell/) — a Unix-like shell written in olang: an interactive
  `os.read_line` loop with pipelines threading stdout→stdin through
  `os.exec`, redirection and globbing on `fs`, `$VAR`/`~`/`$?` expansion,
  quoting, `;`/`&&`/`||` sequencing, 25 builtins implemented on the stdlib,
  aliases, and history persisted across sessions — the long-running
  systems-work proof (1,000 mixed commands soak through one session in
  ~1.4 s). Interactive, but it runs under the harness unattended: a
  non-terminal stdin ends the loop instead of hanging it, and scripted
  stdin drives it in CI-style checks
- [`minilisp/`](minilisp/) — a small Lisp interpreted by olang: reader and
  evaluator over `LVal` enum trees, maps as functional environments, Err
  values as the only error channel, and call-time self-binding for recursive
  defines — the same trick olang's own interpreter uses one level up. Every
  function promotes to the bytecode tier; a timed `fib(17)` runs through two
  layers of interpretation (~17× faster on the tier — this example is what
  motivated the native collection builtins), and a test block locks
  evaluation results and error messages
- [`statlab/`](statlab/) — robust inference on the ods data stack at
  scale: 10,000 simulated subjects, Welch's t-test cross-validated by a
  1,000-round permutation test and a 1,000-resample bootstrap CI (both
  Monte Carlos fanned across every core with `par_map`), a 3-predictor
  OLS at n=5,000 recovering its true coefficients, and SVG charts of
  the bootstrap distribution and the fit — the whole study in
  roughly half a second. A `test` block pins every inference. The `stats` and
  `plot` workflow it scales up is taught in
  [the Data Stack chapter](../docs/ods.md)
- [`parmap/`](parmap/) — data-parallel pipelines with `par_map` /
  `par_filter` and the `par for` loop: counts primes in 48 blocks both
  sequentially and fanned out across every core, asserts the answers
  are identical, and reports the measured speedup (~6–7× on an M-series,
  now that the JIT compiles the kernel natively on every worker);
  then runs the same work through `par for`, whose spawn-style snapshot
  rule means results cross back over a `chan` rather than through shared
  state. `test` blocks pin parallel == sequential and the snapshot rule
  on every run
- [`pargrep/`](pargrep/) — parallel code search on real `spawn` threads:
  files are dealt into chunks, one worker thread per chunk searches with
  `re` + `fs`, results merge through `map(task.join)`, and matching on
  each join's `Err` survives worker failure. Prints sequential-vs-parallel
  timings and self-checks that both agree
- [`ledger/`](ledger/) — a personal-finance web app run entirely by
  `olang main.ol`: a schema-migrated SQLite backend (money as integer
  cents) behind a validated JSON API for transactions, categories, and
  monthly budgets, with an olang-in-the-browser frontend whose analysis
  and charts run client-side on the ods data stack — Frames, `group_by`,
  and `viz` SVG rendered inside the wasm runtime. Seeded demo data via
  `seed.ol`; API contract locked by `tests/ledger_app_test.rs`
- [`app/`](app/) — a full-stack issue tracker run entirely by
  `olang main.ol`: a persistent, schema-migrated SQLite backend behind a
  JSON API with request validation (422s that name each field problem),
  filtered/paginated listing, comments, an audit trail, stats through
  the ods data stack, CSV export, JSON backups, optional bearer-token
  auth for writes, and a router with method-aware 405s, HEAD support,
  per-request logging, and one error envelope — plus its own
  spreadsheet-style frontend, written in olang and run in the browser
  as WebAssembly (the worked example behind
  [the Browser chapter](../docs/wasm.md)). The API contract is
  locked by `tests/tracker_app_test.rs`, which boots the real app.
  Long-running — `run_all.ol` skips it
- [`survey/`](survey/) — a codebase surveyor and the command-line flagship:
  it turns a directory into a report of files and lines by language, a bar
  chart of the biggest, and the largest files. The whole command surface
  (`summary` / `langs` / `files`) is one declarative `cli` spec, `term`
  draws colored tables and a live progress bar that degrade to plain text
  when piped, and `fs` walks the tree. Using only the stdlib and embedded
  packages, it bundles to one executable with `olang build` and documents
  itself with `olang doc`
- [`watch/`](watch/) — rerun a command on an interval and stream its output
  until Ctrl-C: the dogfood for the process story. Drives children with
  `proc`, chains them with `proc.pipeline` (`--pipe "ls | wc -l"`), reads
  exit codes, and traps SIGINT to shut down and print a summary rather than
  being killed mid-frame
- [`metatool/`](metatool/) — olang reading olang over `meta.parse`, which
  hands a parsed program back as ordinary values: a list of statement maps
  you walk with the same `map`/`filter`/`fold` as any data. It reports a
  program's imports and every bare `unwrap(...)` grouped by enclosing
  function — a linter as a script rather than a compiler change, which is
  the "open code" pillar of [openness](../docs/openness.md)
- [`macros/`](macros/) — the macro system dogfooded (experimental,
  [the Macros chapter](../docs/macros.md)): `@bake` evaluates an
  expression at expansion time and splices the result as a literal —
  compile-time computation in one userland line; `@unless` adds control
  flow the language "doesn't have"; `@dbg` prints an expression's source
  and value; and a `@json` derive reads a type's fields through
  `meta.parse` and generates its serializer. `olang expand main.ol`
  shows the program the runtime actually receives; `test` blocks pin
  baked-equals-runtime and the derive's output
- [`instrument/`](instrument/) — zero-cost instrumentation as an imported
  macro library: `@memo` rewrites a function into a cache around its own
  body (recursion memoizes itself — `fib(80)` in a millisecond, with a
  generated `_cache_size()` so the test pins the cache rather than the
  clock), `@trace` logs calls and returns, `@timed` wall-clocks any
  expression, and `@dbg` prints an expression's own source next to its
  value. `olang expand main.ol` shows every line the macros added
- [`contracts/`](contracts/) — checked boundaries as an imported macro
  library: `@require`/`@ensure` contracts that quote the violated
  condition's source (text no runtime function could recover), and
  `@fmtc`, a format string whose placeholder count is checked against its
  arguments at load — the mismatch every logging library meets in
  production, refused before the program runs
- [`markdown/`](markdown/) — a markdown→HTML converter: a block parser
  (`lib/blocks.ol` — headings, lists, blockquotes, fenced code, rules,
  paragraphs) over a recursive inline renderer (`lib/inline.ol` — `code`,
  **bold**, *italic*, links, HTML escaping). Converts a file argument or a
  built-in sample, and self-checks its contract with a `test` block on
  every run

## Also in this directory

- `benchmark.ol` — a standalone timing script: loop, arithmetic, and
  call-heavy microbenchmarks, run by the harness like any other example
- `utils/` — three small `share`d modules (`math`, `string`, `validation`)
  imported by dotted path: `use utils.math { calculate_average }`. It has
  no `main.ol`, so the harness does not run it
- `run_all.ol` — the harness itself, written in olang: it discovers targets,
  runs each in a subprocess with `os.exec`, and exits non-zero on any failure

## Notes

- The bytecode tier is on by default, so these run fast with no flags. Add
  `--ovm-stats` to see how many functions were promoted.

# olang examples

Runnable programs demonstrating the language. Every entry below is a real
program that does real work and prints computed results — nothing is faked,
and nothing is a stub. Run any of them directly:

```bash
olang examples/language/metatool/main.ol
cd examples/tools/survey && olang main.ol summary ../../src
```

To run **every** example at once — each standalone script and each package —
use the self-hosted harness, which launches each program in its own `olang`
subprocess and reports a pass/fail summary:

```bash
cd examples && olang run_all.ol
```

The examples are grouped by what they demonstrate:

- [`data-processing/`](data-processing/) — the ods data stack at work:
  ETL pipelines, statistics, time series, and data-science workstreams
  on real downloaded data
- [`web/`](web/) — full-stack applications on `http.serve` and the wasm
  frontend, plus a load tester
- [`language/`](language/) — languages built in the language: parsers,
  engines, macros, metaprogramming, and the module and package system
- [`concurrency/`](concurrency/) — threads, channels, parallel
  pipelines, and the Harborline soak test
- [`tools/`](tools/) — command-line programs: a codebase surveyor, a
  task tracker, a shell, a watcher, a planner

Three of the web examples are servers that block forever by design
(`app/`, `ledger/`, `webserver/`), so the harness skips them — naming
each skip rather than passing over it silently — and a Rust integration
test boots each one instead. Everything else runs, including
`tools/oshell/`, which notices that its stdin is not a terminal and
exits cleanly.

**Where to start.** [`demo/`](concurrency/demo/) is the guided tour: Harborline is one
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

- [`packages/geometry/`](language/packages/geometry/) — a library package: shapes,
  areas, and 2D point math, exposing a public API with `share`
- [`packages/demo/`](language/packages/demo/) — depends on `geometry` by path and
  imports it with `use geometry { ... }`. Running it
  (`olang examples/language/packages/demo/main.ol`) resolves the dependency with no
  separate install step
- [`capabilities/`](language/capabilities/) — per-dependency capability attenuation:
  the same app runs twice against a malicious `analytics` dependency, and the
  guarded manifest (`fs = false` for the dependency) blocks the backdoor the
  unguarded one lets through
- [`demo/`](concurrency/demo/) — Harborline, the flagship soak test and the closest
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
- [`taskcli/`](tools/taskcli/) — a persistent task tracker as a real multi-file
  package: SQLite storage (`lib/store.ol`), a reporting module using `col`
  and pipelines (`lib/report.ol`), a domain module (`lib/model.ol`), and a
  CLI dispatch on `os.args()` (`main.ol`)
- [`loganalyzer/`](data-processing/loganalyzer/) — parses application logs with `re` capture
  groups (`lib/parse.ol`), aggregates by level and route with `col` +
  pipelines (`lib/stats.ol`), and reads files with `fs` from `os.args()`
  (`main.ol`). Handles malformed lines, missing/empty files, and 2000-line
  logs
- [`scheduler/`](concurrency/scheduler/) — concurrent fan-out and timeouts on real
  threads: four `spawn`ed fetches run concurrently and are collected with
  `map(task.join)` (total time = the slowest, not the sum), then each is
  given a budget with `task.join_timeout`
- [`meterflow/`](data-processing/meterflow/) — a multi-source ETL: JSON-lines telemetry
  streamed in bounded memory, cleaned, aggregated across chunks, joined to
  CSV dimensions, cached in the native columnar format, and charted.
- [`dataproc/`](data-processing/dataproc/) — a CSV→Frame→aggregate→JSON pipeline on the ods
  data stack: `ods.read_csv` infers column types, revenue is one vectorized
  column multiply, revenue-by-region is a `group_by`, then a `json` report
  is emitted, read back, and selected by runtime key — a parsed JSON object
  reads through the same `map_*` accessors as a map. (The records-and-fold
  version of this pipeline ran 50× slower at 200k rows — the measurement
  that opens [the Data Stack chapter](../docs/ods.md), which teaches every
  verb this program uses.)
- [`timeseries/`](data-processing/timeseries/) — analysis over *ordered* data, the
  sequence counterpart to `dataproc`'s bag: where each row's answer
  depends on the rows around it. A service's daily latency over four
  weeks (generated deterministically, no file) is run through the window
  verbs — a 7-day moving average with `rolling`, day-over-day change with
  `shift`, a running worst-case with `cum_max`, and a severity `rank` —
  each added as a column, so the incident hidden in the noise surfaces by
  ordering rather than by a hand-picked threshold. A `group_by` by weekday
  sits alongside to show the sequence and bag views answering different
  questions over one frame. `test` blocks pin the windows
- [`derives/`](language/derives/) — declaration-driven code generation from an
  *imported macro library* (the macro system's dogfood): one `type Contact`
  declaration, and `@json`, `@builder`, and `@arbitrary` derive its
  serializer, its builder API, and a generated test suite — four `test`
  blocks stamped out at expansion time, discovered by `olang test` like
  handwritten ones. Delete a field and every derived artifact follows;
  `olang expand main.ol` shows exactly what the derives produced
- [`template/`](language/template/) — a mustache-style template engine self-hosted in
  olang: a lexer, a parser building a nested node tree over a shared `Node`
  ADT (`lib/ast.ol`), and a renderer walking it against a JSON context.
  Supports `{{ dotted.paths }}`, `{{#each}}`, `{{#if}}`, and `{{ . }}` for the
  current item
- [`workflow/`](language/workflow/) — a data-driven state machine engine (`lib/machine.ol`)
  where transitions carry guards and actions as first-class function values.
  Two machines run on it: an expense-approval pipeline that branches on amount
  (`lib/expense.ol`) and a cyclic turnstile (`lib/turnstile.ol`); context
  evolves immutably and each run yields an audit trail
- [`parser/`](language/parser/) — a parser combinator library (`lib/combinators.ol`):
  parsers are `(input, pos) -> result` functions, composed by higher-order
  combinators (`seq`, `alt`, `many`, `chainl1`, `between`, …). On top of it,
  `lib/calc.ol` is a recursive arithmetic grammar that parses and evaluates
  expressions in one pass, honoring precedence and parentheses
- [`regex/`](language/regex/) — a backtracking regex engine: a recursive-descent parser
  (`lib/parse.ol`) compiles a pattern into a recursive `Re` AST (`lib/ast.ol`),
  and a continuation-passing matcher (`lib/matcher.ol`) walks it with
  backtracking. Supports `. * + ? | ( )`, `[a-z]`/`[^…]` classes, `^`/`$`
  anchors, and `\d \w \s` escapes, with `find` / `find_all` / `matches`
- [`jsonschema/`](language/jsonschema/) — a JSON Schema validator (`lib/validate.ol`):
  the schema and document are both parsed JSON, and validation is a recursive
  walk collecting a pathed error (`$.address.zip`) per violated keyword —
  `type`, `enum`, `required`, `properties`, `items`, and the min/max/length
  bounds. Reads `schema.json` and `data/*.json` from disk
- [`webserver/`](web/webserver/) — a notes JSON API on `http.serve`: a router
  with `:id` path parameters (`lib/router.ol`) dispatching to handlers over
  a SQLite store that persists across requests. GET/POST/DELETE, JSON in and
  out, 404/400 handling. Long-running — `run_all.ol` skips it; it is
  integration-tested by `tests/http_serve_test.rs`
- [`loadtest/`](web/loadtest/) — a self-contained HTTP load test: it boots the
  API (SQLite-backed) in a spawned task, fans a fleet of client workers out
  across `spawn` threads (each firing a burst and timing it), merges the
  per-worker stats through `map(task.join)`, prints throughput/latency, and a `test`
  block asserts the server's own hit count equals the clients' successes
  exactly — proving `http.serve`'s worker pool loses no writes under
  concurrent load
- [`nbody/`](concurrency/nbody/) — an N-body gravity simulation as a bytecode-tier
  benchmark: `Body` structs whose O(n^2) force kernels read fields and call
  `math.sqrt` in a hot loop — exactly what the tier accelerates. Times itself
  and reports throughput; ~44× faster on the tier than the interpreter — the
  widest tier gap in the corpus, which is what a float-and-field hot loop
  hands the JIT. A
  `test` block locks determinism and momentum conservation
- [`oshell/`](tools/oshell/) — a Unix-like shell written in olang: an interactive
  `os.read_line` loop with pipelines threading stdout→stdin through
  `os.exec`, redirection and globbing on `fs`, `$VAR`/`~`/`$?` expansion,
  quoting, `;`/`&&`/`||` sequencing, 25 builtins implemented on the stdlib,
  aliases, and history persisted across sessions — the long-running
  systems-work proof (1,000 mixed commands soak through one session in
  ~1.4 s). Interactive, but it runs under the harness unattended: a
  non-terminal stdin ends the loop instead of hanging it, and scripted
  stdin drives it in CI-style checks
- [`minilisp/`](language/minilisp/) — a small Lisp interpreted by olang: reader and
  evaluator over `LVal` enum trees, maps as functional environments, Err
  values as the only error channel, and call-time self-binding for recursive
  defines — the same trick olang's own interpreter uses one level up. Every
  function promotes to the bytecode tier; a timed `fib(17)` runs through two
  layers of interpretation (~17× faster on the tier — this example is what
  motivated the native collection builtins), and a test block locks
  evaluation results and error messages
- [`statlab/`](data-processing/statlab/) — robust inference on the ods data stack at
  scale: 10,000 simulated subjects, Welch's t-test cross-validated by a
  1,000-round permutation test and a 1,000-resample bootstrap CI (both
  Monte Carlos fanned across every core with `par_map`), a 3-predictor
  OLS at n=5,000 recovering its true coefficients, and SVG charts of
  the bootstrap distribution and the fit — the whole study in
  roughly half a second. A `test` block pins every inference. The `stats` and
  `plot` workflow it scales up is taught in
  [the Data Stack chapter](../docs/ods.md)
- [`crimes/`](data-processing/crimes/) — 8.6 million rows, end to end:
  the complete City of Chicago crime record (~2 GB of live CSV)
  downloaded as part of the run, then trend inference (a two-decade
  decline fitted at R²≈0.9), per-category arrest rates, the city's
  hourly rhythm, a chi-square independence test, k-means over ~900k
  geocoded incidents, and a logistic regression predicting arrests
  (305k training rows, held-out accuracy/AUC, a converging loss
  curve) — the clustering and the classifier written in olang itself.
  The largest workstream in the gallery and the closest thing to a
  whole-engine benchmark; `data/` and `out/` never touch git
- [`climate/`](data-processing/climate/) — a real data-science workstream on real
  downloaded data: fetches Our World in Data's CO2 and energy datasets
  (~24 MB, cached, retried, atomically written), splits countries from
  aggregates with an anti-join, then answers real questions — the
  global emissions trajectory and its peak, top emitters and their
  global share, the absolute-decoupling list (GDP up, CO2 down over a
  decade), renewables' biggest movers since 2000, and a cross-sectional
  model of per-capita CO2 against energy use (r=0.87, R²=0.76) — into a
  self-contained `out/` report with SVG charts and derived CSVs.
  Neither the data nor the outputs touch version control; `test`
  blocks pin the prep and rendering helpers
- [`parmap/`](concurrency/parmap/) — data-parallel pipelines with `par_map` /
  `par_filter` and the `par for` loop: counts primes in 48 blocks both
  sequentially and fanned out across every core, asserts the answers
  are identical, and reports the measured speedup (~6–7× on an M-series,
  now that the JIT compiles the kernel natively on every worker);
  then runs the same work through `par for`, whose spawn-style snapshot
  rule means results cross back over a `chan` rather than through shared
  state. `test` blocks pin parallel == sequential and the snapshot rule
  on every run
- [`pargrep/`](concurrency/pargrep/) — parallel code search on real `spawn` threads:
  files are dealt into chunks, one worker thread per chunk searches with
  `re` + `fs`, results merge through `map(task.join)`, and matching on
  each join's `Err` survives worker failure. Prints sequential-vs-parallel
  timings and self-checks that both agree
- [`ledger/`](web/ledger/) — a personal-finance web app run entirely by
  `olang main.ol`: a schema-migrated SQLite backend (money as integer
  cents) behind a validated JSON API for transactions, categories, and
  monthly budgets, with an olang-in-the-browser frontend whose analysis
  and charts run client-side on the ods data stack — Frames, `group_by`,
  and `viz` SVG rendered inside the wasm runtime. Every response carries
  security headers, text goes out gzipped, the wasm from its brotli
  sibling, the frontend as a program image, and keyed rows repaint
  through `dom.morph`. Seeded demo data via `seed.ol`; API contract
  locked by `tests/ledger_app_test.rs`
- [`app/`](web/app/) — a full-stack issue tracker run entirely by
  `olang main.ol`: a persistent, schema-migrated SQLite backend behind a
  JSON API with request validation (422s that name each field problem),
  filtered/paginated listing, comments, an audit trail, stats through
  the ods data stack, CSV export, JSON backups, optional bearer-token
  auth for writes, and a router with method-aware 405s, HEAD support,
  per-request logging, and one error envelope — plus its own
  spreadsheet-style frontend, written in olang and run in the browser
  as WebAssembly (the worked example behind
  [the Browser chapter](../docs/wasm.md)) — served as a program image,
  gzipped, behind security headers, with keyed rows that repaint through
  `dom.morph`, and a drain on SIGTERM. The API contract is
  locked by `tests/tracker_app_test.rs`, which boots the real app.
  Long-running — `run_all.ol` skips it
- [`survey/`](tools/survey/) — a codebase surveyor and the command-line flagship:
  it turns a directory into a report of files and lines by language, a bar
  chart of the biggest, and the largest files. The whole command surface
  (`summary` / `langs` / `files`) is one declarative `cli` spec, `term`
  draws colored tables and a live progress bar that degrade to plain text
  when piped, and `fs` walks the tree. Using only the stdlib and embedded
  packages, it bundles to one executable with `olang build` and documents
  itself with `olang doc`
- [`watch/`](tools/watch/) — rerun a command on an interval and stream its output
  until Ctrl-C: the dogfood for the process story. Drives children with
  `proc`, chains them with `proc.pipeline` (`--pipe "ls | wc -l"`), reads
  exit codes, and traps SIGINT to shut down and print a summary rather than
  being killed mid-frame
- [`metatool/`](language/metatool/) — olang reading olang over `meta.parse`, which
  hands a parsed program back as ordinary values: a list of statement maps
  you walk with the same `map`/`filter`/`fold` as any data. It reports a
  program's imports and every bare `unwrap(...)` grouped by enclosing
  function — a linter as a script rather than a compiler change, which is
  the "open code" pillar of [openness](../docs/openness.md)
- [`macros/`](language/macros/) — the macro system dogfooded (experimental,
  [the Macros chapter](../docs/macros.md)): `@bake` evaluates an
  expression at expansion time and splices the result as a literal —
  compile-time computation in one userland line; `@unless` adds control
  flow the language "doesn't have"; `@dbg` prints an expression's source
  and value; and a `@json` derive reads a type's fields through
  `meta.parse` and generates its serializer. `olang expand main.ol`
  shows the program the runtime actually receives; `test` blocks pin
  baked-equals-runtime and the derive's output
- [`instrument/`](language/instrument/) — zero-cost instrumentation as an imported
  macro library: `@memo` rewrites a function into a cache around its own
  body (recursion memoizes itself — `fib(80)` in a millisecond, with a
  generated `_cache_size()` so the test pins the cache rather than the
  clock), `@trace` logs calls and returns, `@timed` wall-clocks any
  expression, and `@dbg` prints an expression's own source next to its
  value. `olang expand main.ol` shows every line the macros added
- [`contracts/`](language/contracts/) — checked boundaries as an imported macro
  library: `@require`/`@ensure` contracts that quote the violated
  condition's source (text no runtime function could recover), and
  `@fmtc`, a format string whose placeholder count is checked against its
  arguments at load — the mismatch every logging library meets in
  production, refused before the program runs
- [`markdown/`](language/markdown/) — a markdown→HTML converter: a block parser
  (`lib/blocks.ol` — headings, lists, blockquotes, fenced code, rules,
  paragraphs) over a recursive inline renderer (`lib/inline.ol` — `code`,
  **bold**, *italic*, links, HTML escaping). Converts a file argument or a
  built-in sample, and self-checks its contract with a `test` block on
  every run

- `metro/` — a transit planner for the fictional city of Arden, built
  on the bundled collections with every structure load-bearing: station
  registry in a `table`, fastest routes by `alg.dijkstra` over flat CSR,
  fare zones from BFS rings, a departure board draining a `heap`, a
  disruption drill answering continuity with `dsu` and the stranded set
  in a `bitset`, rankings by `sort_by_key` and quickselect — every
  printed answer asserted as it goes

## Also in this directory

- `benchmark.ol` — a standalone timing script: loop, arithmetic, and
  call-heavy microbenchmarks, run by the harness like any other example
- `crunch.ol` — the number-crunching gauntlet: five-million-frame tail
  recursion, ninety thousand ordinary frames, 300! and fib(1000) in
  `bigint`, an auto-parallel map over a million elements, `par_map` over
  the Collatz record hunt, a 150k-row trip through the parallel data
  stack, and a task-scheduling pipeline through all six bundled
  collections (topo-sort, Dijkstra, heap, table, dsu, bitset) — every
  stage cross-checked against an answer computed another way
  (`--heavy` scales it tenfold)
- `utils/` — three small `share`d modules (`math`, `string`, `validation`)
  imported by dotted path: `use utils.math { calculate_average }`. It has
  no `main.ol`, so the harness does not run it
- `run_all.ol` — the harness itself, written in olang: it discovers targets,
  runs each in a subprocess with `os.exec`, and exits non-zero on any failure

## Notes

- The bytecode tier is on by default, so these run fast with no flags. Add
  `--ovm-stats` to see how many functions were promoted.

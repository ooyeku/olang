# olang examples

Runnable programs demonstrating the language. Run any of them with:

```bash
olang examples/01_language_tour.ol
```

To run **every** example at once — each standalone script and each package —
use the self-hosted harness, which launches each program in its own `olang`
subprocess and reports a pass/fail summary:

```bash
cd examples && olang run_all.ol
```

The numbered programs are a curated, self-contained tour — each runs top to
bottom, does real work, and prints computed results (nothing is faked). They
are verified in CI by `tests/example_programs_test.rs`, so they cannot rot.

## Curated tour

| File | What it teaches |
|---|---|
| [`01_language_tour.ol`](01_language_tour.ol) | Every core construct: literals, collections, destructuring, functions, closures, control flow, pattern matching, pipelines, structs, error handling, maps |
| [`02_data_pipeline.ol`](02_data_pipeline.ol) | Real analytics — totals, filtering, group-by via fold, leaderboards, derived metrics — the pipeline sweet spot |
| [`03_algorithms.ol`](03_algorithms.ol) | Recursion, memoization, quicksort, binary search, the prime sieve, function composition, and the strategy pattern via closures |
| [`04_stdlib_showcase.ol`](04_stdlib_showcase.ol) | The batteries: SHA-256/HMAC hashing, bcrypt passwords, RSA sign/verify, math (stddev, trig identities), calendar arithmetic, and JSON |
| [`05_text_processing.ol`](05_text_processing.ol) | Real text work with `str` and `re`: word frequency, log parsing via regex captures, email extraction, validation, a template engine, and slugification |
| [`06_database.ol`](06_database.ol) | A SQLite-backed task tracker: schema, parameterized inserts, queries, group-by aggregates, updates — real persistence |
| [`07_algebraic_types.ol`](07_algebraic_types.ol) | Enums as real sum types: a recursive expression-tree evaluator, generic Option combinators, and a binary search tree |

## Packages

A two-package demonstration of the package manager (see
[docs/packages.md](../docs/packages.md)):

- [`packages/geometry/`](packages/geometry/) — a library package: shapes,
  areas, and 2D point math, exposing a public API with `share`
- [`packages/demo/`](packages/demo/) — depends on `geometry` by path and
  imports it with `use geometry { ... }`
- [`capabilities/`](capabilities/) — per-dependency capability attenuation:
  the same app runs twice against a malicious `analytics` dependency, and the
  guarded manifest (`fs = false` for the dependency) blocks the backdoor the
  unguarded one lets through
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
  per-worker stats after `await`, prints throughput/latency, and a `test`
  block asserts the server's own hit count equals the clients' successes
  exactly — proving `http.serve`'s worker pool loses no writes under
  concurrent load
- [`nbody/`](nbody/) — an N-body gravity simulation as a bytecode-tier
  benchmark: `Body` structs whose O(n^2) force kernels read fields and call
  `math.sqrt` in a hot loop — exactly what the tier accelerates. Times itself
  and reports throughput; ~9x faster on the tier than the interpreter. A
  `test` block locks determinism and momentum conservation
- [`oshell/`](oshell/) — a Unix-like shell written in olang: an interactive
  `os.read_line` loop with pipelines threading stdout→stdin through
  `os.exec`, redirection and globbing on `fs`, `$VAR`/`~`/`$?` expansion,
  quoting, `;`/`&&`/`||` sequencing, 25 builtins implemented on the stdlib,
  aliases, and history persisted across sessions — the long-running
  systems-work proof (1,000 mixed commands soak through one session in
  ~1.4 s). Interactive, so `run_all.ol` skips it; scripted stdin drives it
  in CI-style checks
- [`minilisp/`](minilisp/) — a small Lisp interpreted by olang: reader and
  evaluator over `LVal` enum trees, maps as functional environments, Err
  values as the only error channel, and call-time self-binding for recursive
  defines — the same trick olang's own interpreter uses one level up. Every
  function promotes to the bytecode tier; a timed `fib(17)` runs through two
  layers of interpretation (~11× faster on the tier — this example is what
  motivated the native collection builtins), and a test block locks
  evaluation results and error messages
- [`statlab/`](statlab/) — robust inference on the ods data stack at
  scale: 10,000 simulated subjects, Welch's t-test cross-validated by a
  1,000-round permutation test and a 1,000-resample bootstrap CI (both
  Monte Carlos fanned across every core with `par_map`), a 3-predictor
  OLS at n=5,000 recovering its true coefficients, and SVG charts of
  the bootstrap distribution and the fit — the whole study in under
  half a second. A `test` block pins every inference. The `stats` and
  `plot` workflow it scales up is taught in
  [the Data Stack chapter](../docs/ods.md)
- [`parmap/`](parmap/) — data-parallel pipelines with `par_map` /
  `par_filter` and the `par for` loop: counts primes in 48 blocks both
  sequentially and fanned out across every core, asserts the answers
  are identical, and reports the measured speedup (~7× on an M-series,
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
- [`markdown/`](markdown/) — a markdown→HTML converter: a block parser
  (`lib/blocks.ol` — headings, lists, blockquotes, fenced code, rules,
  paragraphs) over a recursive inline renderer (`lib/inline.ol` — `code`,
  **bold**, *italic*, links, HTML escaping). Converts a file argument or a
  built-in sample, and self-checks its contract with a `test` block on
  every run

```bash
olang examples/packages/demo/main.ol      # auto-resolves the dependency
```

## Topic examples

Older single-topic programs, still runnable:

- `string_interpolation.ol` — template strings and advanced literals
- `union_types.ol` — discriminated-union dispatch via `match`
- `dates.ol`, `crypto_test.ol` — focused stdlib walkthroughs
- `simple_sales.ol`, `stats_module.ol` — small analytics
- `base_utils.ol`, `extended_utils.ol`, `stats_module.ol` — `share`/`use` module system
- `chain_a/b/c.ol`, `conflict_*.ol` — transitive sharing and name-conflict resolution
- `loops.ol`, `fast_loops.ol`, `benchmark.ol` — iteration and performance

## Notes

- The bytecode tier is on by default, so these run fast with no flags. Add
  `--ovm-stats` to see how many functions were promoted.

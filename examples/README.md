# olang examples

Runnable programs demonstrating the language. Run any of them with:

```bash
olang examples/01_language_tour.ol
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
- [`taskcli/`](taskcli/) — a persistent task tracker as a real multi-file
  package: SQLite storage (`lib/store.ol`), a reporting module using `col`
  and pipelines (`lib/report.ol`), a domain module (`lib/model.ol`), and a
  CLI dispatch on `os.args()` (`main.ol`)
- [`loganalyzer/`](loganalyzer/) — parses application logs with `re` capture
  groups (`lib/parse.ol`), aggregates by level and route with `col` +
  pipelines (`lib/stats.ol`), and reads files with `fs` from `os.args()`
  (`main.ol`). Handles malformed lines, missing/empty files, and 2000-line
  logs
- [`scheduler/`](scheduler/) — concurrent fan-out and timeouts with
  `async`/`await` and `Promise.all`/`race`: four simulated fetches run
  concurrently (total time = the slowest, not the sum), and each is raced
  against a timeout budget
- [`dataproc/`](dataproc/) — a CSV→aggregate→JSON pipeline: reads sales rows
  with `csv.parse_with_headers`, types them (`lib/transform.ol`), aggregates
  revenue by region (`lib/aggregate.ol`), emits a `json` report, then reads
  it back and selects fields by a runtime key — a parsed JSON object reads
  through the same `map_*` accessors as a map

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
- `enum` types declare but their variants cannot yet be constructed at
  runtime; the tour expresses discriminated unions as tagged structs matched
  on a tag field, which is the working idiom today.

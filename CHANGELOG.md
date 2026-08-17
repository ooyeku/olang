# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **A Frame prints as a table.** `display()` gave a shape and a type
  list, which is what a debugger prints, not what an exploratory session
  wants. Printing a Frame — at the REPL, through `println`, through
  `to_string` — now renders the data:

  ```text
  Frame[2 x 3]
  ┌────────┬────────┬─────┐
  │ region │ amount │ qty │
  │ String │  Float │ Int │
  ├────────┼────────┼─────┤
  │ east   │   25.5 │  10 │
  │ west   │  320.0 │   3 │
  └────────┴────────┴─────┘
  ```

  Numeric columns are right-aligned so digits line up, text columns are
  left-aligned, and nulls print as `—`. Four caps keep a large Frame
  inside a terminal — twenty rows, twenty-eight characters per cell, a
  hundred characters of width, and however many columns fit — and each
  one that bites is reported under the table, because a table that
  silently dropped a column would be worse than the summary it replaced.
  A long Frame keeps both ends and elides its middle, which is what makes
  the result of a `sort_by` readable at a glance.

- **`ods.head(f)` defaults to 10 rows.** The verb typed most often at the
  REPL was the one that raised most often, because it demanded a count.

- **`meterflow`: the data stack's flagship ETL (Campaign 2, DP4).**
  A multi-source pipeline in [`examples/meterflow/`](examples/meterflow/):
  meter telemetry arrives as JSON lines too large to hold, two CSV
  dimension tables carry sites and tariffs, and the job streams the
  readings, gates them on quality, aggregates across chunks, joins the
  dimensions, prices the result, caches it in the native columnar format,
  and charts it.

  Streaming turns one pass into many partial aggregates, so the pipeline
  is correct only if chunk boundaries cannot change the answer. Over
  400,000 readings the streamed run and a whole-file computation agree
  exactly — 390,033 rows kept and 781,262.26 kWh either way, across 80
  chunk boundaries.

  **It was built to find what was missing, and it did.** Four gaps
  surfaced in the writing, all now closed:

  | | |
  |---|---|
  | `ods.all_of(masks)` · `ods.any_of(masks)` · `ods.not(mask)` | combine Bool masks, three-valued like SQL |
  | `ods.concat(frames)` | stack Frames, matching columns by name |
  | `ods.join(a, b, on)` | the second key name now defaults to the first |
  | `ods.eq(s, v)` · `ods.ne(s, v)` | now accept a plain value, as `==` already did |

  Mask combination was the blocking one: a filter with two conditions was
  simply not expressible. `&&` cannot serve, because the language compiles
  it to a conditional jump so that it can short-circuit, and that has no
  elementwise reading over a column — so these are functions taking a
  *list*, since a real filter has three or four conditions rather than
  two. `concat` was the other: without it, putting the partial results of
  a streaming loop back together required a detour through row-shaped
  values, which is exactly what the columnar representation exists to
  avoid.

- **A native columnar file format (Campaign 2, DP2 — the last lane).**
  CSV and JSON lines are interchange with the outside world and both pay
  for it: every load re-parses text and re-infers types, and neither can
  record what a column *was*. A column of zero-padded codes written to
  CSV comes back as integers with the padding gone.

  | | |
  |---|---|
  | `ods.write_frame(f, path)` | write a Frame → `Result<Unit>` |
  | `ods.read_frame(path, columns = all)` | read one back → `Result<Frame>` |
  | `ods.frame_info(path)` | its schema, from the header alone → `Result<Frame>` |

  Types survive exactly, the load is a read rather than a parse, and the
  optional column list decodes only what is named — the rest is stepped
  over using the byte lengths in the header. Over 500,000 rows and six
  columns: **CSV 138ms, columnar 50ms, one column 26ms**, and the file is
  15MB against the CSV's 17MB.

  Arrow and Parquet were declined: both are a dependency and both are
  opaque, which is the wrong trade for a language whose case is that the
  artifact should be inspectable. The header is UTF-8 text, one line per
  column, readable with `head`:

  ```text
  olang-columns 1
  rows 500000
  columns 6
  col "id" Int enc=plain nulls=0 bytes=4000000
  col "region" String enc=dict nulls=0 bytes=2000066
  data
  ```

  `enc` is how a String column is stored, and it exists because the
  obvious layout lost. Writing each row's text with its own offset made
  the file *larger than the CSV* — 24MB against 17MB — since an eight-byte
  offset costs more than the four characters it points at, and repetition
  is the normal case in a table. A repeating column is now written once as
  a dictionary of its distinct values plus one small code per row. The
  writer chooses by computing both sizes and taking the smaller, so there
  is no threshold to tune and no case where the choice is a guess.

- **A Frame is subscriptable: `f["amount"]`, `f[mask]`, `s[i]`.** Reaching
  a column was the most repeated gesture in data code and the most
  verbose: `ods.column` and `ods.get` together were **13.6% of all 937
  `ods` calls in this repository**, and `ods.column(raw, "amount") *
  ods.column(raw, "quantity")` is 62 characters to say one multiply.

  ```olang
  let sales = ods.with_column(raw, "revenue", raw["amount"] * raw["quantity"])
  let big   = sales[sales["revenue"] > 100.0]
  ```

  | subscript | means |
  |---|---|
  | `f["amount"]` | the column, as a Series |
  | `f[mask]` | the rows a Bool Series keeps |
  | `f[0]` | **refused**, naming `ods.head` / `ods.take` |
  | `s[3]`, `s[-1]` | the element; negatives count from the end, as in lists |

  Two meanings for one subscript, told apart by the key's type. Refusing
  the third is the point: pandas spells column selection, row filtering,
  positional slicing, and an error all `df[x]`, which is why `.loc` and
  `.iloc` had to be invented on top of it.

  A column name that is not there raises — a literal name in the source
  is a claim about the data's shape, and a wrong claim is a wrong program
  — and the message lists the columns that do exist. Subscripting is
  read-only; there is no `f["a"] = x`, because Frames are values and an
  assignment that silently produced a copy would be a trap.

  The mechanism is one trait method, `NativeObject::index`, defaulting to
  `None`, reached from the index path of *both* tiers. Any native value
  can now define its own subscript; the ones that don't keep the
  language's existing error.

- **`ods.describe(f)` and `ods.schema(f)`.** After `head`, `describe` is
  the first thing typed against unfamiliar data. Both return a **Frame**
  rather than a map, so they print as tables and can themselves be
  sorted, filtered, and written out. `describe` gives count, nulls, mean,
  std, and the five-number summary per column; `schema` gives name, type,
  and null count, for a Frame too wide to summarize. A Frame has one type
  per column, so the numeric statistics are null for String and Bool
  columns rather than absent — a result whose shape depended on its input
  would push the branch onto every caller.

- **`ods` reads and writes JSON lines (Campaign 2, DP2).** One JSON
  object per line is what log shippers, event queues, and export jobs
  emit, and the stack could not read any of it.

  | | |
  |---|---|
  | `ods.read_jsonl(text)` | parse JSON-lines text → `Result<Frame>` |
  | `ods.read_jsonl_file(path)` | read a file → `Result<Frame>` |
  | `ods.open_jsonl(path)` | stream a file → `Result<Reader>` |
  | `ods.to_jsonl(f)` | serialize a Frame → `String` |
  | `ods.write_jsonl(f, path)` | write a Frame → `Result<Unit>` |

  Columns are the union of the keys and a missing key is a null — the
  same rule as `frame_from_records`, because this is that function with a
  parser in front of it. Blank lines are skipped, since a trailing
  newline is how nearly every writer ends the format. A malformed or
  non-object line is an `Err` naming the line number, which on a
  million-line file is the whole diagnostic. `to_jsonl` omits nulls
  rather than writing them, so a Frame round-trips through `read_jsonl`
  to itself.

  **The streaming reader is now one type over both formats.** `open_csv`
  and `open_jsonl` both return a `Reader`, and `next_chunk`, `rows_read`,
  and `at_end` drive either — a streaming loop names its format once, at
  the `open_`, and a function that takes a reader takes both. A
  300,000-row JSON lines file streams at 30.6MB against 471MB read whole.
  JSON lines carry no header, so a reader remembers the columns its
  earlier chunks established and returns them on the empty chunk that
  ends the loop, keeping the promise the CSV reader makes from its
  header. The file-reaching calls demand `fs` at the matching level; the
  text parser and serializer stay pure.

- **`ods` streams files larger than memory (Campaign 2, DP2).**
  `read_csv_file` holds the whole table at once, which is the right shape
  until the file no longer fits. `ods.open_csv` returns a reader that
  hands back one chunk at a time, each an ordinary Frame:

  ```olang
  let r = unwrap(ods.open_csv("events.csv"))
  let mut total = 0.0
  loop {
      let chunk = unwrap(ods.next_chunk(r, 50000))
      if ods.n_rows(chunk) == 0 => break
      total = total + ods.sum(ods.column(chunk, "amount"))
  }
  ```

  Memory is set by the chunk size rather than the file size. Measured
  over a 1M-row CSV: the whole-file read peaks at 196.5MB, the streamed
  pass at **12.2MB** — 0.2MB more than the same pass over a file five
  times smaller. `ods.rows_read(r)` and `ods.at_end(r)` report progress;
  the final chunk is empty but still carries the file's columns, so a
  pipeline written against a chunk needs no special case for it. Asking
  for zero rows is refused rather than silently ending the loop.
  `open_csv` demands `fs` at read level, like `read_csv_file`.

  Two constraints from the language picked this shape. A callback API —
  `(chunk) => { total = total + chunk }` — is refused by 0.62's capture
  rule, the accumulator being a captured write; and a fold taking an
  olang lambda cannot live in `ods` at all, because `OvmModule::dispatch`
  receives `(func, args)` and no interpreter, which is exactly what lets
  both tiers dispatch identically. The loop needs no closure, so it needs
  no exception.

- **Thread confinement is now a property a native value can declare.**
  `cell` was confined to its creating thread by code that named cells
  specifically. That check moved onto the `NativeObject` trait as
  `confined_to()`, so `spawn` and `chan.send` refuse any confined value
  without knowing what it is. The CSV reader — which holds a file
  position, and so is exactly as unsafe to share as a cell — was refused
  at both boundaries before a line was written for it.

- **The data stack can reach files, and write them (Campaign 2, DP2).** `ods` could parse CSV *text* and had no way to emit
  anything at all, so the chapter described an end-to-end story the stack
  could not finish. Three functions close the loop:

  | | |
  |---|---|
  | `ods.read_csv_file(path)` | read a CSV file into a Frame → `Result` |
  | `ods.to_csv(f)` | serialize a Frame to CSV text → `String` |
  | `ods.write_csv(f, path)` | write a Frame to a file → `Result` |

  `to_csv` is the exact inverse of `read_csv`: nulls become empty cells,
  which is what `read_csv` reads back as null, and a value containing a
  comma or a quote is quoted, so the column count survives a round trip.
  It cannot fail, so it returns the string outright; the two that touch
  the disk return `Result`, because a missing file is the caller's input
  rather than their mistake.

  **Both file-touching calls demand `fs`**, at read and write level
  respectively — every other `ods` function stays pure and needs no
  grant. Without that, `ods` would have become a filesystem capability by
  the back door, which is the hole `db.open` had before 0.60.

### Changed

- **`caps::check` is now defined in terms of `caps::required`.** They were
  two independent walks over the builtin surface that happened to agree,
  under a comment claiming they could not disagree. Adding the `ods` file
  calls to `required` and not to `check` proved otherwise: `--trace-caps`
  reported the read as an `fs` effect while `--deny fs` cheerfully allowed
  it. One classification now answers both questions — "what would this
  need?" and "does this grant permit it?" — so the drift is not
  expressible.

- **Capabilities no longer switch the bytecode tier off (Campaign 3, C1).**
  A `[capabilities]` manifest or a `--deny` flag used to force the whole
  run onto the interpreter, so turning on the security feature turned off
  the performance work. On `fib(30)` that cost **1497 ms instead of 4 ms**
  — a sandboxed program was a different program, performance-wise, than an
  unsandboxed one.

  The gate was never the missing piece: `BuiltinFunctions::call_internal`
  is the one choke point both tiers already pass through. What was missing
  was *context*. The tier reaches builtins it cannot run natively through a
  bridge interpreter — a separate `Interpreter` — which carried no
  capability table and no notion of which function was executing, so it
  presented every call as unrestricted and unattributed. Enforcing
  partially being worse than not enforcing, the tier was stepped aside.

  Compiled functions now carry the file they were declared in;
  the VM keeps a stack of those files as it executes, mirroring the
  interpreter's `coverage_file_stack`; and the grant table, the
  `--trace-caps` set, and the executing function's file are handed to the
  bridge before each dispatch. A promoted function in an attenuated
  dependency is judged by *that dependency's* grant, and the profiler sees
  its effects:

  ```
  capability 'fs' denied: fs.exists requires it, and dependency 'lib' is
  granted fs=false ... (olang.toml [capabilities.dependencies.lib])
  ```

  A restricted run now shows the same promotion count, the same bytecode
  calls, and the same wall time as an unrestricted one. An unrestricted run
  pays one branch per call for the attribution stack and is unchanged at
  4 ms.

  Record/replay (`--record`) still runs interpreter-only; the timeline has
  its own reason (it must observe every nondeterministic call, in order)
  and its own lane.

### Removed

- **The bytecode tier's value model drops six unreachable variants.** 0.63
  deleted `async`/`await`/`Promise` from the grammar, parser, AST, and
  interpreter and stopped there: `ValueData` still carried `Promise`, and
  beside it `Thunk`, `Stream`, `LazyList`, `CompiledFunction`, and
  `OptimizedValue` — a lazy-evaluation and self-optimization scheme that
  no code ever constructed. Nothing failed when they were left behind,
  which is why they survived four releases.

  Gone with them: their object structs, the twelve supporting types that
  existed only as their fields (`GeneratorFunction`, `TransformationChain`,
  `OptimizationData`, `TypeFeedback`, `GcMap`, and so on), `PromiseState`,
  the VM's `PromiseError`, and the six dead lazy-evaluation variants of
  `InterpreterError`. `FunctionObject` loses its `optimization_data` field,
  which was written by `Default` and never read. `src/ovm/nanbox.rs` — 309
  lines marked "not yet wired", imported by nothing — is deleted too.

  Behavior is unchanged: no construction site existed, so no program could
  reach any of it. `fib(30)` still runs in 4 ms. This is a Rust-API break
  for anything embedding olang as a library, and 320 fewer lines in the
  file every tier value passes through.

- **`async`, `await`, `try`, `catch`, and `Promise` are ordinary
  identifiers.** 0.63 and 0.65 removed the constructs but kept grammar
  stubs so the parser could emit migration errors naming the
  replacements. There is no olang code outside this repository, so there
  was nobody to migrate: the stubs are gone, and the reserved-word list
  is down to fifteen words plus seven contextual ones. `let try = 3`,
  `row.catch`, and `fn await_all(ts)` all parse.

### Fixed

- **Whole-program tier agreement is now tested, and it found a real
  divergence on its first run.** `docs/ovm.md` promises that a tier which
  cannot reproduce the interpreter's result refuses to run rather than
  diverging — the basis for trusting a runtime that swaps engines under a
  program. Until now that was checked only by hand-written snippets
  calling one function with one set of arguments.

  `tests/tier_agreement_test.rs` runs the real binary over the real
  corpus — every runnable book example and the standalone example
  programs — under `--no-ovm` and `--ovm-tier=1`, and diffs stdout,
  stderr and exit status together. Elapsed times and throughputs are
  masked, since running faster is the point; everything else is compared
  verbatim.

  It found `examples/parser` diverging. Two closures built from the same
  higher-order combinator give different answers on the compiled tier,
  the second behaving as though it captured the first one's argument:
  `word "olang"` yields `olang` interpreted and `""` compiled, and
  `1 + 2 * 3` evaluates to `7` interpreted and a parse error compiled.
  This is the failure a reviewer reported after ~8,000 lines and which
  eight hand-written attempts failed to reproduce — reducing it to a
  single file, or to a two-file module, makes it disappear. The bug is
  open; the test that finds it is `#[ignore]`d with that reason stated,
  and un-ignoring it is the definition of done.

- **A program can read its own capability grant (Campaign 3, C3).**
  A denial still stops the program — that is deliberate and unchanged.
  What was missing is the other branch: a program that can degrade could
  not *ask*, only attempt the call and be killed by it, so degradation
  had to be written as recovery from an error, which the language does
  not offer.

  | | |
  |---|---|
  | `caps.allowed(name)` | does the calling code hold it → `Bool` |
  | `caps.level(name)` | `"none"` / `"read"` / `"full"` |
  | `caps.granted()` | the whole grant as a map |

  ```olang
  let plan = if caps.allowed("fs") => "cache to disk" else => "in memory"
  ```

  The answer is the **caller's** grant: code in an attenuated dependency
  sees what that dependency was given, which is the same set the gate
  would enforce a moment later — one classification, asked two ways.
  None of the three is itself gated, so the escape hatch stays available
  under the tightest restriction.

- **`os.exit` now requires `proc` (Campaign 3, C2).** It was the hole in
  the gate: a dependency denied `proc` could not spawn a process but
  could still terminate the host, which is a larger power than the one it
  was refused. `proc` now means "may affect processes", including this
  one.

- **A bundle's transparency record is validated before it is trusted
  (Campaign 3, C2).** `read_bundle` checked the *frame* — lengths,
  overflow, bounds — but nothing checked the *contents*. The `format`
  field selects which bytes the digest covers and the test was
  `format >= 3`, so a bundle claiming format 99 took the format-3 path,
  recomputed a digest that does not cover the meta record, matched it,
  and printed `[verified]`. An unknown format is now refused, and
  digests must be well-formed hex. The only verdict worse than
  "unverifiable" is a confident wrong one.

- **A struct shares its fields instead of copying them.**
  `Value::Struct` held a bare `HashMap` while `Value::Map` directly beside
  it held an `Arc`, so every clone of a struct rebuilt the whole field map
  — and a value is cloned on every bind, every argument pass, every
  return. Structs now hold `Arc<HashMap<..>>` like maps and lists do.

  **This is not the whole of the reported problem.** A review measured a
  struct argument costing 129ms against a list's 1ms and attributed it to
  this representation; the representation was indeed wrong, but fixing it
  did not move that number. Bisecting further: with `--no-ovm` the two are
  *identical* — 4ms and 5ms — and the gap only appears with the bytecode
  tier enabled. The penalty is therefore in the tier boundary rather than
  in the value, and is tracked separately.

- **Assertions work inside a nested block.** They were parsed only as a
  direct child of a test block, so `assert_eq` inside an `if`, a `for`,
  or a `while` fell through to an ordinary call and failed with
  "Undefined variable: assert_eq" — surprising, since a loop over cases
  is exactly where an assertion belongs. Assertions are now a statement
  form like any other, so they also work outside a test block, where a
  failure raises.

- **`()` has a literal, and a pattern.** Unit is what the language hands
  back from an empty branch and what an `X | ()` field holds, and it had
  no spelling: `let u = ()` was a parse error, and so was
  `match x { () => ... }`. Both work now, on both tiers. Where `()` is
  followed by `=>` the nullary-lambda reading still wins — except on the
  right of a comparison, so `if t == () => 0 else => 1` reads as "t is
  Unit, then this branch", which is the whole reason to write it.

- **`for _ in xs` discards its binding.** `_` is not an identifier —
  identifiers must start with a letter — so a loop that ignored its item
  needed an invented name. `par for _ in` too.

- **A tuple iterates in `for`.** Refusing it was a surprise with nothing
  behind it.

- **The tiers agreed to disagree about the empty tuple.** Adding the
  `()` literal surfaced it immediately: the interpreter read a zero-element
  tuple as Unit and the bytecode compiler built an actual empty tuple, so
  a hot function comparing against `()` got a different answer than a cold
  one. Both now read it as Unit, and the new forms above are covered by
  tier-agreement tests that run the same source through both.

- **A long float no longer pushes columns off a printed table.** A
  computed column carries full `f64` precision — a standard deviation of
  `173.98078198467783` — which is wide enough to cost a table two other
  columns. Floats whose full form exceeds twelve characters are shown to
  six significant digits, and the footer says so, like every other cap
  the renderer applies. Values that already fit are untouched, so an
  ordinary table carries no footnote.

- **`ods.read_csv` given a path now says so.** `read_csv` takes CSV
  *text*, so `ods.read_csv("data/sales.csv")` parsed the path itself: a
  Frame with one column named `data/sales.csv`, zero rows, and no error
  anywhere — the failure surfaced later, as a confusing result from
  something else. A single-line argument ending in `.csv`, `.tsv`, or
  `.txt` is now refused, naming `read_csv_file` and `open_csv` instead.
  The check reads the string rather than asking the filesystem, because
  `read_csv` holds no `fs` grant, and a function that probes the disk
  without one is the leak that keeping these two functions separate
  exists to prevent.

- **A `par for` body could not write to an enclosing binding, but nothing
  said so (breaking).** 0.62 made an assignment across a capture boundary
  an error, and the rule reached functions and closures but not `par for`
  — whose body also runs against a worker's snapshot. The write was dead,
  and *conditionally* dead: workers are clamped to the item count, so a
  one-element list took the sequential path where the write really landed.
  The same loop gave two different answers depending on the length of the
  list it was given.

  ```olang
  let mut a = 0
  par for x in [1] { a = a + 1 }        // a == 1
  let mut b = 0
  par for x in [1, 2] { b = b + 1 }     // b == 0
  ```

  `par for` now opens the same boundary as a function body, and the write
  is refused before the program runs. The message differs from the closure
  one because the remedy does: a cell is confined to its creating thread,
  so one made outside the loop cannot help inside it.

  ```text
  cannot assign to 'tally': `par for` runs its body on worker threads, each
  against its own snapshot of the environment, so the write would be discarded
  rather than reaching the outer 'tally'. Produce a value per item and combine
  them — `sum(par_map(xs, (x) => ...))` — or send results over a `chan`
  ```

  Reading an enclosing binding is unaffected — that is how the body gets
  its inputs — and a `let mut` declared inside the body is local to one
  iteration. `examples/parmap/` demonstrated the old dead write; it now
  collects its results over a channel and cross-checks them against the
  sequential total.

## [0.65.0] - 2026-08-16

The error-model boundary (D8), and the last lane of Campaign 1's language
work. The lane started by checking D8's own premise and found it wrong
twice over.

### Changed

- **`try`/`catch` is removed (breaking).** D8 kept it for "recovering
  from runtime errors at coarse boundaries". It never did that: it
  destructured a `Result`, and a genuine runtime error inside a `try`
  block still aborted the program. What it actually was is a third
  spelling of something `match` and `unwrap_or` already say — and across
  97 corpus files, including a shell, a load tester, and two fullstack
  apps, it was used **zero times**.

  ```olang
  match f(x) { Ok(v) => v, Err(e) => fallback }   // the general form
  unwrap_or(f(x), fallback)                       // when the fallback is a value
  ```

  The grammar still recognizes the old form so the error can name those
  two rather than failing with "expected a statement". `try` and `catch`
  stay reserved for this release and become ordinary identifiers at 1.0.

- **Recovery is documented as structural, because that is what it is.**
  The boundaries D8 named already recover, without any construct at the
  failure site:

  | Boundary | A runtime error inside it becomes |
  |---|---|
  | a spawned task | `Err(e)` from `task.join` |
  | an `http.serve` handler | a logged 500; the server keeps serving |

  Anywhere else a runtime error stops the program, which is what you want
  from a bug — and what 0.61–0.64 spent four releases establishing.

### Added

- **A discarded `Result` draws an advisory warning.** This was the actual
  hole in the error model. A fallible call in statement position dropped
  its failure silently, and `olang check` reported the file clean:

  ```olang
  fs.write_file("/nope/x.txt", "data")   // failed
  println("wrote it")                     // printed anyway
  ```

  ```text
  warning: the Result from fs.write_file is discarded, so a failure here is
  invisible. Bind it, match it, unwrap it to fail loudly, or write
  `let _ = ...` to say the failure is deliberately ignored.
  ```

  It is advisory, not a gate: unlike the scope and mutability rules this
  is a judgement about intent rather than a provable contradiction, and a
  script that genuinely does not care whether a log write landed is not
  wrong. A block's final statement is its value, so a function whose body
  *is* the fallible call is not flagged.

  The lint reads the help registry for which functions return `Result`,
  so the diagnostic and `:help` cannot disagree about what can fail.

### Fixed

- **`http.serve`'s bind failure was silently discarded in both flagship
  apps.** Starting `examples/app` or `examples/ledger` on a taken port
  printed the startup banner and exited with status 0, as though it had
  served. Found by the new warning on its first run over the corpus; both
  now report the failure and exit non-zero.

- **25 stale help entries from 0.64.** The conventions audit changed 28
  functions to return values rather than `Result` but left their
  documented return types behind, so `:help os.args` still said `Result`.
  Also surfaced by the new lint, which flagged `os.args()` as a discarded
  `Result` — using the help registry as the source of truth is what
  forced the two back into agreement.

### Migration

`try { A } catch (e) { B }` becomes `match A { Ok(v) => v, Err(e) => B }`,
or `unwrap_or(A, B)` when `B` is just a value. The corpus needed no
changes — it never used the construct — so the migration here is entirely
for code outside this repository.

## [0.64.0] - 2026-08-16

The standard-library conventions audit (D7). Every module was walked once
against one rule, because there is exactly one chance to do this: after
1.0 the API is frozen.

### Changed

- **One rule now decides what a stdlib function returns (breaking).**

  | The operation | Returns |
  |---|---|
  | cannot fail | its value |
  | can fail for reasons the caller could handle | `Result` |
  | was *called wrongly* | raises |

  The third tier is new, and it is what makes the second trustworthy.
  Previously a wrong argument count came back as `Err`, indistinguishable
  from a missing file — so `unwrap_or(fs.read_file(p), "")` swallowed a
  typo'd call exactly the way it swallows a real failure, and the default
  hid the bug indefinitely. Misuse now stops the program:

  ```text
  os.arch(1, 2)
  // error: os.arch expects 0 arguments, got 2
  ```

- **28 functions stopped returning `Result`.** The `os` module carried
  most of the noise — `unwrap(os.args())` appeared in nearly every
  script:

  | Module | Now returns a value |
  |---|---|
  | `os` | `args` `arch` `os_type` `family` `pid` `path_separator` `temp_dir` `username` `is_tty` `flush` `has_env` `list_env` `set_env` `remove_env` `interrupted` `reset_interrupt` |
  | `fs` | `exists` `is_dir` `is_file` |
  | `crypto` | `hex_encode` `export_public_key` |
  | `csv` | `add_row` `read_column` `row_count` `set_headers` `sort_by_column` |
  | `base64` | `validate` |
  | `http` | `decode_query` |

  What keeps `Result` is what genuinely fails: `os.get_env` (the variable
  may be absent), `os.cwd`, `os.chdir`, `os.home_dir`, `os.hostname`,
  `os.exe_path`, `os.exec`, `os.read_line`, `os.stdin`, `os.stdin_lines`,
  `os.on_interrupt`, and every file, network, database, and parsing
  operation.

- **Error messages name the module-qualified function.** `arch expects 0
  arguments` left the reader guessing which `arch`; it is now `os.arch`.

- **Seven declaration keywords are freed as identifiers (`share`,
  `error`, `test`, `type`, `trait`, `impl`, `use`).** Each only ever
  introduces a declaration, and the token after it disambiguates, so
  `let type = row.kind` is an ordinary binding while `type Point = ...`
  still declares a type — including both in one file. They are common
  enough in data and statistical code (a *share* of a total, an *error*
  term, a *test* case) that reserving them cost more than it bought.
  They also work as field names now: `row.type` used to be a parse error.

- **`fs.join` takes its parts variadically**, and still accepts a single
  list. `fs.join("data", "raw", name)` for the literal case,
  `fs.join(segments)` when they are computed — forcing a spread there
  would have been a downgrade. A non-string part raises.

### Migration

`unwrap`/`unwrap_or` raise on a non-`Result`, so every affected call site
fails loudly rather than silently. The fix is to delete the wrapper:

```olang
let args = os.args()                  // was: unwrap(os.args())
if fs.exists(path) => ...             // was: unwrap_or(fs.exists(path), false)
let tty = os.is_tty()                 // was: unwrap_or(os.is_tty(), false)
```

A `match` on one of these needs the same treatment — the `Ok`/`Err` arms
no longer fit and the match will fail at runtime:

```olang
let user = os.username()              // was: match os.username() { Ok(u) => u, Err(e) => "?" }
```

The repository corpus (46 call sites across 25 files, plus the embedded
`cli` and `term` packages) is migrated in this release.

## [0.63.0] - 2026-08-16

Removes `async`, `await`, and the `Promise` API. olang now has **one**
concurrency model — threads — and this is the last of Campaign 1's
breaking changes to the language surface.

### Changed

- **`async`, `await`, and `Promise` are gone (breaking).** They described
  a deadline-based scheduler that resembled asynchronous I/O without
  being it: `Promise.delay` simulated latency, `await` slept, and none of
  it involved concurrency at all. Meanwhile `spawn` was already a real OS
  thread and `await` on a spawned task was really a *join*. Two
  vocabularies for one mechanism, one of which was theatre.

  What remains is what was always doing the work:

  | Was | Now |
  |---|---|
  | `await t` | `task.join(t)` |
  | `Promise.all(ts)` | `ts \|> map(task.join)` |
  | `Promise.race([work, timeout])` | `task.join_timeout(t, ms)` |
  | `Promise.delay(v, ms)` | `time.sleep(ms)` |
  | `Promise.resolve(v)` | `v` |
  | `Promise.reject(e)` | `Err(e)` |
  | `async fn f() = ...` | `fn f() = ...`, called through `spawn f()` |

  There is no function colouring left: any function can be spawned,
  because a task is a thread running an ordinary call.

- **`spawn` returns a task handle** instead of a promise. `task.join(t)`
  blocks and returns the value, or `Err(e)` if the task failed —
  unchanged from what `await` did, so worker failure stays a value
  rather than a crash. Joining the same handle twice returns the
  memoized result; a handle dropped unjoined is fire-and-forget.

- **`Promise<T>` annotations are removed.** `spawn` produces a `Task`.

- The removed forms still *parse*, solely so the error can name the
  replacement. A keyword that merely falls out of a grammar produces
  "expected a statement", which tells a reader nothing:

  ```text
  `await` was removed in 0.63. olang's concurrency model is threads,
  channels, and data parallelism: `spawn f(x)` starts a task and
  `task.join(t)` collects its result (or `Err(e)` if it failed); join
  several with `tasks |> map(task.join)`; use `time.sleep(ms)` for delays
  and `chan` to stream results.
  ```

  `async` and `await` stay reserved for this release and become ordinary
  identifiers at 1.0.

### Added

- **`task` module.** `task.join(t)` collects a spawned task's result.
  `task.join_timeout(t, ms)` returns `Ok(v)` if it finished in time and
  `Err("timed out")` otherwise.

  **A timeout bounds the wait, not the work.** An OS thread cannot be
  cancelled from outside without leaving whatever it touched in an
  unknown state, so olang does not offer a cancel that would be a lie: a
  timed-out task runs to completion and its result stays collectible
  from the same handle. `join_timeout` wraps success in `Ok` precisely
  so "the task produced `Err`" stays distinguishable from "we stopped
  waiting". This is the honest replacement for `Promise.race`, which
  implied the loser stopped.

- No `task.join_all`. Joining a list of tasks is `map(task.join)` —
  they are all already running, so the fan-in needs no API of its own.

### Fixed

- **`:pkg load` no longer promises an import that cannot work.** Loading
  an *application* package (a `main.ol` plus a `lib/`, with no public
  root module) printed "use it with `use <name>`", and that import then
  failed with the generic "Module 'x' not found" plus a "system-level
  error" hint. Two bugs: the REPL advertised a root module without
  checking for one, and the resolver swallowed its own precise
  diagnosis and fell through to the global file search, so the reader
  was sent looking for a missing file instead of being told the package
  has no importable root. `:pkg load` now lists the modules the package
  actually offers, and `use <pkg>` names both what it looked for and
  what is there:

  ```text
  package 'loadtest' has no root module: `use loadtest` needs one of
  index.ol, mod.ol, loadtest.ol, or src/index.ol at examples/loadtest/.
  Importable modules there: loadtest.lib.server, loadtest.lib.stats
  ```

### Migration

`olang check .` reports every site. The corpus migration in this release
(scheduler, loadtest, pargrep, demo, and the book's concurrency chapter)
is the worked example. `examples/scheduler/` is the clearest before and
after: it was built entirely on `Promise.delay` + `Promise.race` and is
now `spawn` + `task.join` + `task.join_timeout`, with the "the task is
still running" caveat stated where the timeout is taken.

## [0.62.0] - 2026-08-16

Completes the mutability model 0.61.0 began. Plain `let` is an immutable
binding, `let mut` is a reassignable one, and this release adds the third
thing: a mutable *location*.

### Added

- **`cell` — the one mutable location.** `cell(v)` makes one;
  `cell.get(c)` reads it; `cell.set(c, v)` replaces the contents;
  `cell.update(c, f)` applies `f` to the current value, stores the
  result, and returns it. `cell.new(v)` is the same function as
  `cell(v)` — the module is callable, so the constructor reads as a
  noun while the operations stay namespaced.

  Values are immutable and closures capture by value, which is what
  makes `spawn` and `par_map` safe without locks. The case that model
  handles badly is state updated from deep inside a call chain or from
  a callback whose signature is fixed; threading an accumulator through
  functions that have no other interest in it is the honest workaround
  and often a poor one. A cell is the escape hatch, deliberately
  narrow. A cell is a location, not a value: two cells with equal
  contents are not equal, and binding one to a second name aliases it.

- **Cells are confined to the thread that created them.** Reading or
  writing a cell from another thread is an error naming both threads,
  and `chan.send` refuses to send one — including one nested inside a
  list, map, struct field, or `Result`. The no-shared-mutable-state
  guarantee that makes olang's parallelism lock-free is preserved
  exactly: two threads still cannot reach one mutable location. A cell
  created inside a task and used only there is unremarkable; the rule
  concerns crossing, not tasks.

  Confinement is checked on *access* rather than at the thread
  boundary, because `spawn` and `par_map` snapshot the whole
  environment rather than an enumerated capture list — there is no list
  of what crossed to inspect, and a crossing-time check would have had
  to refuse any `spawn` with a cell merely in scope. Checking on use is
  both sound and precise. `chan.send` is the one crossing that holds
  the value being sent, so it is checked there, at the mistake.

- **`cell.update` refuses re-entrant access.** Touching the same cell
  from inside its own update function is an error rather than a write
  that the function's return value silently overwrites. The lock is
  never held across the callback, so this is a diagnosable error, not a
  deadlock, and the flag is cleared whether the callback returns or
  raises.

- Cells need no timeline recording: mutation is deterministic within a
  thread and unreachable across threads, so a replayed run performs the
  same mutations in the same order.

### Changed

- **Assigning to a captured binding is an error (breaking).** Inside a
  closure or nested function, assigning to a name bound in an enclosing
  scope was a dead write — capture is by value, so it reached the
  snapshot and never the original — and had been an advisory warning
  since 0.51. It is now refused before the program runs, and the
  message points at `cell`:

  ```text
  cannot assign to 'total': it is captured from an enclosing scope, and
  functions capture by value — the outer 'total' would not change. Return
  the new value, or hold the state in a cell (`let total = cell(...)`,
  then `cell.set(total, ...)`)
  ```

  The warning only became fair once there was an alternative to name,
  which is why it lands with `cell` rather than before it. The check
  moved into the same pre-execution validator as the 0.61.0 rules, so
  the runtime, `olang check`, and the editor now agree on it — before,
  only `olang check` reported it and the program still ran. No file in
  the repository corpus was affected.

- Calling a module value invokes its `new`, so `cell(0)` and
  `cell.new(0)` are one function. This is a general rule, not a special
  case: a module whose purpose is constructing one kind of value may be
  called directly.

### Fixed

- The language reference's keyword table wrote `catch e { … }`; the
  syntax is `catch (e) { … }`, as every runnable example in the book
  already had it.

## [0.61.0] - 2026-08-16

This release settles olang's scope and mutability rules. Three behaviors
that had been advisory warnings since 0.50 are now enforced errors. This
is the **second and final deliberate breaking change before 1.0** (the
first was runtime type enforcement in 0.48.0); see the migration guide
below.

### Changed

- **Every block scopes its bindings (breaking).** A `let` inside
  `{ ... }` — a bare block, a loop body, an `if` branch, a `match` arm —
  ends with that block. Previously a bare block's bindings leaked into
  the enclosing scope, which meant a name's lifetime depended on which
  construct happened to surround it. Function bodies, `for` variables,
  and `match` arm bindings already scoped; blocks now agree with them,
  and the rule is one sentence instead of a table of exceptions.

  This also closed a real divergence between execution tiers. The
  bytecode compiler kept a flat map of local variables with no scope
  stack, so `let x = 1; { let x = 2 }; x` evaluated to `1` on the
  interpreter and `2` on the bytecode tier. Both tiers now push and pop
  a scope per binding block, and two differential tests pin the
  agreement.

- **Assignment is not a declaration (breaking).** `x = 1` where `x` was
  never bound is an error rather than an implicit `let`. This is what
  turns a mistyped name from a silently-created variable into a
  refusal.

- **`let mut` is enforced (breaking).** Assigning to a binding not
  declared `mut` is an error. `mut` was parsed and then discarded — it
  documented intent to the reader and promised nothing to the compiler.
  It is now a guarantee, and a plain `let` genuinely means immutable.

  Shadowing is unchanged and remains the recommended alternative: a
  fresh `let` of the same name always works and produces a new binding
  rather than a mutable one.

- All three are checked by a single validation pass that runs **before**
  the program executes, so a violation on a rarely-taken branch is
  caught anyway and cannot differ between the interpreter, bytecode, and
  JIT tiers. `olang check` and the language server report them in the
  same words as `olang run`, with the same line and column.

- `olang check` distinguishes these in its output: scope and mutability
  violations are labeled "the program is refused before it runs" rather
  than sharing the type checker's label.

- `meta.parse` now reports a `let` declaration's mutability, so
  project lints written against the AST can see it.

### Migration

Run `olang check .` — it lists every site with the exact fix in the
message. The three rewrites:

```olang
// A binding you reassign now needs `mut`
let mut total = 0
for n in xs { total = total + n }

// An assignment with no declaration becomes a declaration
let count = 0            // was: count = 0

// A block's binding, used after the block, moves out of the block
let mut resolved = ""    // was: declared inside the `if`
if ready => { resolved = compute() }
```

For a value you compute in stages, prefer shadowing over `mut`:

```olang
let raw = read_input()
let raw = str.trim(raw)
let raw = str.lower(raw)
```

The repository's own corpus — 97 example files, the embedded stdlib
modules, and every runnable example in the book — is migrated in this
release and checks clean.

## [0.60.0] - 2026-08-15

### Changed

- **CLI facelift — every command is now discoverable.** `olang` moved from
  a hand-rolled first-argument dispatch to real clap subcommands, so `olang
  --help` lists every command (`run`, `check`, `fmt`, `test`, `build`,
  `inspect`, `caps`, `replay`, `doc`, `bench`, `lsp`, `repl`) and `olang
  <command> --help` documents any one of them with its own flags — none of
  which the old flat help surfaced. The file-first form is unchanged: `olang
  script.ol [args]` still runs a file directly (a word that is neither a
  command nor a flag is taken as a file path), bare `olang` still starts the
  REPL, and every prior invocation — run options, `--watch`, `--deny`,
  script arguments, built binaries — behaves exactly as before. Run options
  are grouped under their own heading, and the help ends with a worked
  example block.

### Added

- **`olang record` — recording is now a first-class command.** Recording a
  run was only reachable through the `--record TRACE.olt` run option, so the
  timeline story was lopsided: `olang replay` was a visible command with no
  visible way to produce what it replays. `olang record <file> [-o
  trace.olt] [args]` runs a program and writes its trace (defaulting to the
  program's stem + `.olt`), the mirror of `olang replay`. The `--record` run
  option still works and is unchanged.

- **`olang inspect` reports the build platform.** A built binary now records
  the OS and CPU architecture it was built on (`built on: macos/aarch64`),
  and `inspect` flags whether that matches the current machine — a native
  `olang build` binary only runs on its own platform, so this tells you at a
  glance whether a binary that arrived from elsewhere will run here. The
  field is informational provenance (like the olang version): it is not part
  of the integrity digest, and binaries built before it existed inspect
  cleanly with the line omitted. The default `inspect` summary also now
  points at `--source` (the way it already pointed at `--manifest` and
  `--lockfile`), so printing a binary's embedded source is discoverable from
  the summary itself.

- **Parallel hash join (data-pipeline campaign DP1).** `ods.join` now runs
  its probe phase across every CPU core when the left frame is large
  (50,000+ rows): each left row is looked up in the built key table
  independently, and the per-thread chunks are concatenated in row order,
  so the parallel result is byte-identical to the sequential one (pinned by
  a test that passes in both feature modes). Measured ~2.6× on a 4M-row
  join. Small joins stay single-threaded. This is where the lack of a GIL
  shows — a large join uses the whole machine with no ceremony. (Elementwise
  Series operations were already parallel.)

- **Parallel `group_by` aggregation (data-pipeline campaign DP1b).** The
  aggregation pass in `ods.group_by` now scatters across every CPU core when
  the frame is large (100,000+ rows with under ~4M groups): each thread
  accumulates `sum`/`count`/`min`/`max` into per-group partials over a fixed
  row range, and the partials are merged sequentially in chunk order. Integer
  results are bit-identical to sequential; float `sum`/`mean` reorder their
  additions but the order is fixed by the chunk layout, so a given input
  always reproduces the same result run to run. The group-id hashing pass
  stays sequential. Measured ~1.4× on 8M rows over 1,000 groups (aggregation
  is a fraction of the total; the hashing pass dominates and is DP1c's
  target). Pinned by a test that passes in both feature modes.

## [0.59.0] - 2026-08-15

### Added

- **`--trace-caps --write` and `olang caps` — frictionless capability
  manifests (openness maturity lane OM3).** `olang run --trace-caps --write`
  folds the suggested least-privilege `[capabilities]` block directly into
  the package's `olang.toml`; it never overwrites an existing block, and
  falls back to printing when the program is not in a package. `olang caps
  [path]` prints the grant a package *declares* (base plus each dependency's
  attenuation) — the static counterpart to the observed `--trace-caps`
  profile — and defers to `inspect --caps` for a built binary.

- **`examples/capabilities` — a runnable malicious-dependency demo.** The
  same app runs twice against a third-party package with a backdoor: the
  unguarded variant lets it read a local secret; the guarded variant grants
  the dependency `fs = false`, so the identical read is refused at the gate
  while the app's own read still works. Pinned by a Rust test.

- **`olang check --rules <rules.ol>`: project lint rules in olang (openness
  lane O6).** A project defines lint functions named `rule_*` that take a
  file's AST, flattened to a list of `kind`-tagged node maps each carrying
  its nearest source `line`, and return findings (a message string or a
  `#{ "message", "line" }` map). The checker runs these rules alongside the
  built-in type checks and reports each finding with the file name, line,
  and rule name. Findings count as problems, so a violation produces a
  non-zero exit. A rules file that defines no `rule_*` functions is an
  error, and a rule that raises an error is reported by name.

- **`olang inspect <binary> --against <dir>`: provenance check.** Compares
  the binary's embedded source, manifest, and lockfile to a checkout on
  disk and reports each file as match, differ, or missing, exiting non-zero
  on any mismatch. Where `--verify` checks internal consistency,
  `--against` confirms that a binary was built from a specific source tree.

- **`olang run --trace-caps`: capability profiler.** Reports the
  capabilities a run used (`fs` as read or write, plus `net`, `db`, `proc`,
  and `env`) and prints a least-privilege `[capabilities]` manifest with
  every unused capability set to its most restrictive value. Runs on the
  interpreter tier so every effect is observed, and reports even if the
  program crashes.

### Security

- **Openness soundness cleanup (pass S6–S10).** Several smaller hardening
  fixes: (S6) machine-identity `os.*` — `arch`, `os_type`, `family`,
  `path_separator`, `args`, `cwd`, `exe_path`, `pid`, `is_tty` — and
  `crypto.random_bytes` are now recorded by the timeline, so a trace that
  branches on the machine or process context replays portably (dead
  `time.now`/`utc_now`/`today` entries removed). (S7) `olang check --rules`
  runs the rules file in a no-capability sandbox, so a hostile `rules.ol`
  cannot touch the filesystem, network, or processes when loaded. (S8)
  `olang inspect`'s bundle-footer parser uses checked arithmetic, so a
  crafted binary is rejected rather than triggering a huge allocation.
  (S10) the timeline's record step now actually enforces its documented
  round-trip guard — a result that does not serialize (a non-finite float)
  is not recorded, so replay diverges cleanly instead of serving a
  corrupted value. (`db`/`proc` recording and a symlinked-dependency
  attribution edge remain tracked.)

- **Record/replay no longer silently misapplies recorded values (openness
  soundness pass S4).** Two fixes make single-threaded replay sound. (1) Map
  iteration is now deterministic: `map_keys`/`map_values` iterate in
  key-sorted order (matching `entries`), so a `HashMap`'s
  process-randomized order can no longer make a record run and a replay run
  visit a map in different orders — which previously served one call's
  recorded result to a different call with no divergence raised. (2) The
  trace now records a fingerprint of each recorded call's arguments (trace
  format 2), and replay raises a divergence when a recorded op is invoked
  with different arguments than recorded. v1 traces replay without the
  argument check. Map iteration determinism also benefits any program that
  hashes or serializes iterated output.

- **`meta.parse` now emits every AST child, so project lints no longer
  silently miss code (openness soundness pass S5).** The conversion dropped
  children exactly where calls hide — `match` arms, `await`/`assert*`,
  and `map`/struct/object/template literals collapsed to a summary node — so
  a `check --rules` lint like "no bare `unwrap()`" returned clean on code
  that had one inside a `match` arm. Every variant now emits its children
  (`arms`, `entries`, `fields`, template `parts`, and the async/assertion
  interiors), and the expression conversion is **exhaustive** (no
  catch-all), so the compiler guarantees no variant is silently dropped and
  a new one is a build error until it is handled.

- **`db` and `net` are no longer latent filesystem capabilities (openness
  soundness pass S2/S3).** A filesystem sub-gate now confines file access
  that happens *through* other modules: `db.open` on a file path requires
  `fs` (an in-memory `:memory:` database needs none), a `db` query running
  `ATTACH` requires `fs`, and an `http.serve` handler returning `body_file`
  is refused (403) unless the program's grant permits `fs` read. Previously
  a program with `fs = false` could still create/write arbitrary files via
  `db.open`/`ATTACH` or read any file via `body_file`. Now `fs = false`
  actually confines the filesystem even when `db` or `net` is granted.

- **The transparency checksum now binds the executed AST, closing a
  critical integrity hole (openness soundness pass S1).** A built binary
  runs its embedded **AST**, not its source (the source is only shown for
  error snippets). The checksum previously covered source + manifest +
  lockfile but **not** the AST, so an attacker could replace the AST with a
  malicious program while leaving the source byte-identical: `inspect
  --verify` passed, `--source` printed the clean source, and the binary
  executed the swapped code. The digest is now `source ‖ AST ‖ manifest ‖
  lockfile` (bundle format 3), and both `--verify` and `--against` also
  assert that the embedded source parses to the embedded AST — so `--source`
  is honest and a swapped AST is reported as `DIVERGES`. Found by an
  adversarial audit and confirmed by a reconstructed attack; pinned by a
  regression test. Format-2 bundles (0.59.0-dev) verify with the old digest;
  rebuild to get AST binding.

- **The transparency checksum covers the manifest and lockfile, not only
  the source (openness lane O1a).** `olang build` records the checksum over
  the source, the embedded `olang.toml`, and the `olang.lock`, so a grant
  widened in place fails `--verify`. Bundles built before this carry no
  such checksum and fall back to the source checksum.

### Fixed

- **Two real per-tick memory leaks in the concurrency path, found by
  soak-testing.** A program that opened a channel or `spawn`ed a worker
  pool every tick (Harborline's crew loop is the canonical shape) grew
  memory without bound:
  - *Channels were pinned in a process-wide registry that `close` never
    removed* — every channel ever created leaked its entry. Channels are
    now `Value::Native` handles that own the channel by `Arc`, so the last
    handle drop frees it; there is no registry to leak. (`leaks`: 20 000
    channels went from ~47 MB to a flat ~14 MB.)
  - *The `spawn` registry kept every completed task's memoized result
    forever.* A promise now carries a drop-guard shared by its clones;
    when the last clone drops — the last place that could still `await`
    the task — the registry entry is removed. Double-await (a tested
    guarantee) and fan-out-then-collect both keep working. (`leaks`:
    64 000 spawns went from linear growth to flat.)

  Together these cut Harborline's real per-tick growth by ~65 %, and
  `leaks` now reports zero. Two defensive hardenings landed alongside:
  the bytecode tier's hot-mirror `Vec` (indexed by a global function id)
  is capped so a short-lived VM can't size it to the global high-water,
  and `JitCache` now frees its Cranelift module's executable pages on
  drop (Cranelift does not do this automatically). A residual RSS climb
  under extreme thread churn (tens of thousands of fresh OS threads) is
  reclaimable — `leaks` clean, malloc heap flat — and is a macOS
  thread-VM characteristic best addressed by pooling worker threads,
  tracked with the runtime's memory work.


## [0.58.0] - 2026-08-15

### Added

- **The `meta` module — the Open AST, the "open code" pillar.**
  `meta.parse(source)` parses olang source and returns the program as
  ordinary olang values: a list of `kind`-tagged statement maps you walk
  with the same `map`/`filter`/`fold`/`match` as any data. Because the
  syntax is stable, these node shapes are a stable public format —
  linters, codemods, and import extractors become olang scripts, not
  compiler changes (`otc deps` is four lines over it). Faithful for the
  shapes a tool inspects, summarizing the deep interior; a syntax error is
  an ordinary `Err`, never a crash. `examples/metatool` lints bare
  `unwrap()` calls per function. With this the openness campaign's
  three-pillar thesis — open artifacts, open execution, open code — is
  complete.

- **The Open Timeline — record, replay, deterministic re-execution.**
  `olang --record trace.olt program.ol` logs a run's nondeterministic
  inputs — `random.*`, the `time` clocks, the environment/stdin/`exec`
  surface of `os`, filesystem reads, `http`, and seeded-random `crypto` —
  and `olang replay trace.olt` re-runs the program serving each of those
  calls from the log, reproducing the run bit-for-bit: the same random
  rolls, timestamps, and environment, to the last digit. It works because
  olang programs are deterministic given their inputs (immutable values,
  capture-by-value closures, a seeded RNG), so reproducing the inputs
  reproduces the whole run. The `.olt` trace embeds the program source,
  so a trace is a portable, self-contained reproduction — replay works
  from a machine where the program does not exist, and a *crashed* run
  records on the way down, so the failure replays exactly. Divergence is
  detected: if the program's effect sequence no longer matches the trace,
  replay stops at the exact point and says so. Runs on the interpreter
  tier (the one choke point that sees every builtin), single-threaded in
  v1; `replay --why` (value provenance) is a recorded roadmap rung.
  Pinned by an integration suite (reproduction, portability, crash
  capture, divergence).

- **The transparent binary + capability manifests — "Open by
  construction."** Two halves of one identity feature for a language
  named *Open*:
  - `olang inspect <binary>` reads the transparency record out of any
    `olang build` executable: its exact source (`--source`), its
    `olang.toml` and `olang.lock` (`--manifest`/`--lockfile`), a sha256
    of the source verified on demand (`--verify`, nonzero on mismatch),
    the capability grant (`--caps`), or the whole paper trail extracted
    to a directory (`-o dir/`). A built binary already embedded its
    source (for error snippets); this turns that into a guarantee — you
    cannot ship an olang program as a black box. It doubles as a
    built-in SBOM and as the answer to "what version, which patches?".
  - `[capabilities]` in `olang.toml` gates the effectful stdlib surface
    (`fs`, `http` as `net`, `db`, `proc`, the environment functions of
    `os`) at the module boundary; pure computation is never gated.
    Absent = wide open, so it is opt-in and never breaks existing code.
    The novel part is **per-dependency attenuation**: a dependency can
    be granted *less* than the app, never more, enforced by attributing
    each gated call to the package whose code made it — a supply-chain
    compromise that adds `fs`/`net` behaviour to a dependency that was
    never granted it dies at the gate. `--deny` (and `OLANG_DENY`)
    restrict any run from the command line, and a built binary enforces
    the manifest it carries. Enforcement runs on the interpreter tier
    (the call stack is what attributes a call): a capability-restricted
    run steps the bytecode tier aside, like `par for`; unrestricted runs
    keep full speed. Pinned by an integration suite covering manifest
    grants, attenuation, `--deny`, ghost-dependency refusal, and the
    build → inspect → enforce round-trip.
- **`examples/demo` — Harborline, the consolidated flagship example.**
  The 23 loose scripts at the top of `examples/` are consolidated into
  one coherent, long-running system: a harbor-operations simulator with
  eleven library modules (domain ADTs and tariff expression trees, a
  BST scheduler with a tide model, a SQLite ledger, threaded unload
  crews over channels, fold-based analytics cross-checked against the
  stats module, template/regex/JSON/CSV text handling, an RSA-signed
  hmac digest chain, and a term dashboard). It runs forever by default
  with graceful Ctrl-C shutdown, is deterministic under `--seed`,
  bounded under `--ticks`, writes no files unless `--out` is given, and
  checks world-vs-ledger invariants every simulated day with the
  testing module — a soak test for the language, not a demonstration.
  95 assertions of module self-tests run via `olang test .`; the
  harness runs it bounded; `tests/example_programs_test.rs` pins a
  deterministic end-to-end run. Documented as a new book chapter,
  [Building Robust Systems](docs/demo.md).
- **Every builtin and stdlib symbol is documented in the REPL's `:help`.**
  Fifteen modules had no coverage at all (`proc`, `chan`, `time`, `toml`,
  `ods`, `stats`, `plot`, `dom`, `cli`, `term`, `ui`, `viz`, `dash`,
  `colx`, `mathx`) and several "covered" modules had gaps (`math`'s
  trig/log family, `fs` path helpers, `csv` builder verbs, `db`
  transactions, `random.gauss`, the `os` signal/tty/stdin functions,
  `str.fmt`). All ~270 registered functions now resolve, verified by
  diffing every registration site against the help index; a unit test
  pins per-module minimum counts so new modules can't silently ship
  undocumented.
- **`:help <module>` lists the module's functions.** A bare module name
  (`:help proc`, `:help stats`) previously fuzzy-matched to one arbitrary
  member; it now shows the full member listing, including nested
  namespaces (`:help stats.norm`). Exact function lookup, category
  lookup, and typo fuzzy-matching are unchanged, in that order.
- **`olang run file.ol` works.** There is no `run` subcommand (the file
  is the first positional), but the muscle memory from `cargo run`/`go
  run` is real: a first argument literally `run` that names no existing
  file now shifts to the next argument, with script argv preserved.

- **The package manager's trust model is now enforced, not aspirational.**
  Three `otc pkg` hardenings:
  - *Checksums are verified, everywhere content arrives.* Every install —
    fresh resolve or lockfile replay — re-checks fetched git/registry
    sources against the recorded sha256 and fails hard on a mismatch;
    a registry release's published checksum is enforced at fetch time.
    Previously checksums were recorded but never re-checked. Path
    dependencies stay exempt on install (editing one is development, not
    tampering). New `otc pkg verify` re-checks everything the lock pins
    on demand.
  - *The registry index is append-only.* `otc pkg publish` used to
    silently replace an already-published version's rev/checksum — the
    exact history rewrite the checksum exists to catch. Republishing an
    existing version is now an error; `--force` remains as a deliberate
    escape hatch.
  - *`otc pkg add` validates before writing.* A `--path` must exist, a
    `--version` must parse (and, with a registry configured, be
    satisfiable), conflicting source flags are an error instead of a
    silent precedence pick, `--tag` without `--git` is rejected, and
    changing an existing dependency's source requires `--force`. Typos
    now surface at add time, not as an opaque failure at the next
    install.
- **`proc` — child processes, streaming I/O, and pipelines.** A new
  native module for driving processes beyond `os.exec`'s run-to-
  completion model. `proc.spawn(program, args)` returns a live `Process`
  handle you feed with `write`/`write_line`/`close_stdin`, read a line at
  a time with `read_line` (`read_all` for the rest), and finish with
  `wait` (→ `#{ code }`) or `kill`. stdout and stderr are drained on
  background threads, so a child that floods one stream never deadlocks a
  caller reading the other. `proc.pipeline(stages)` chains commands the
  way the shell's `a | b | c` does — each stage's stdout wired to the
  next one's stdin — and returns `#{ code, stdout, stderr, codes }` with
  every stage's exit code. Lane T3 of the **Toolsmith campaign**.

- **`os.on_interrupt` — graceful Ctrl-C for long-running tools.**
  `os.on_interrupt()` traps SIGINT so it sets a flag instead of killing
  the process; `os.interrupted()` polls it (`while os.interrupted() ==
  false { ... }`) and `os.reset_interrupt()` clears it, so a server or
  watch loop can drain and exit cleanly.

- **`examples/watch` — the process-story dogfood.** A `watch(1)`-style
  tool that reruns a command on an interval and streams its output, or
  runs a pipeline with `--pipe`, until Ctrl-C — exercising `proc`
  streaming, `proc.pipeline`, and `os` signal handling through the `cli`
  + `term` toolkit.

- **`olang test --coverage` — line coverage from the test runner.**
  `--coverage` reports covered/total executable lines per file plus an
  overall figure; `--coverage-lines` additionally lists each file's
  uncovered ranges. Coverage is a report, never a gate — it leaves the
  exit code untouched. It is attributed to the file the code *lives in*:
  function values now carry a `def_file` that the interpreter pushes and
  pops across calls, so a helper defined in one file and exercised by a
  test in another is credited to the helper's file, not the test's. The
  executable-line denominator is derived from the parsed AST (the same
  statements the runtime records), so a fully-exercised file reads
  exactly 100%. Runs on the interpreter tier so the statement-level hook
  sees every line. Lane T7 of the **Toolsmith campaign** — the last piece
  of a credible test story.

### Fixed

- **Building the demo flushed out four latent bugs, all fixed:**
  - *Bytecode tier: a capturing lambda inherited its enclosing
    function's parameter annotations*, applied positionally to
    [own params..., captures...] — so a lambda capturing an annotated
    `String` parameter failed with "expects Int, got String" once
    compiled, while the interpreter ran fine. Lambdas now carry their
    own checks; pinned by a differential test.
  - *"Aggressive memory management" cleared the module cache mid-load.*
    A >10-element list literal in a module body triggered a cleanup
    that dropped the loader's in-flight placeholder entries — the
    anchors for relative `use` resolution — so the next import in the
    importing file failed with "Cannot find module". The cache is now
    left alone while a module load is in flight.
  - *`olang test` reported ✓ over failing assertions.* A test block was
    only marked failed if it raised; `testing.assert_*` results (which
    tally rather than raise) were never consulted, so a block full of
    failing assertions passed. The runner now charges each block with
    the assertion failures that occurred inside it.
  - *`otc deps` and `otc unused` were blind to every declaration* —
    they matched AST statements without unwrapping the `Located` span
    wrapper introduced by the error-experience overhaul, so `deps`
    always printed "No dependencies found" and `unused` scanned
    nothing. Both now match through the wrapper.
- **`embedded_term_test` no longer fails when run from a real
  terminal.** The test assumed a test binary's stdout is a pipe, but
  `cargo test` leaves the fd a tty when run interactively; the color-off
  leg is now forced explicitly with `NO_COLOR`.

- **The gallery no longer eats the GPU.** The browser host ran one
  unthrottled requestAnimationFrame chain per `dom.on_frame` handler and
  painted every animated canvas on every tick, visible or not — on a
  120Hz retina display the gallery's three live pieces (12,500-star
  galaxy, 50,000-point curtain, rose curves) repainted ~4.25MP of
  devicePixelRatio-2 backing store 120 times a second, two of them
  under semi-transparent full-surface clears (~0.5 Gpix/s of clears
  plus ~7.5M point-quads/s), pinning the GPU near 80%. Three host-side
  fixes in `olang-dom.js`, no olang code changes:
  - All `on_frame` handlers now share one animation loop capped near
    60fps — data animations gain nothing from 120Hz.
  - `dom.draw`/`dom.draw_points` skip painting for a canvas outside
    the viewport (IntersectionObserver, feature-detected): handlers
    keep running, offscreen canvases cost the GPU nothing, and
    painting resumes the frame after the canvas scrolls back in.
  - A canvas can trade retina backing for fill rate with
    `data-olang-dpr` in its markup; the gallery's three animated
    canvases run at 1.5 (44% fewer pixels each), stills keep full
    sharpness. devicePixelRatio is also capped at 2.
  Together: roughly 2x from the frame cap, ~1.8x from the smaller
  backing stores, and near-total savings for whatever isn't on
  screen — with three pieces and at most one in view, an order of
  magnitude less GPU work in normal browsing.
- **List, tuple, and Unit equality now work in promoted functions.** The
  bytecode VM had no `==`/`!=` arms for top-level List, Tuple, or Unit
  operands, so a function like `fn is_empty(xs) = xs == []` returned the
  right answer interpreted but raised "Unsupported operation: Equal" once
  it got hot enough to promote — a hard tier disagreement. All three now
  compare structurally, exactly mirroring the interpreter (including the
  deliberate asymmetry that `1 == 1.0` is true but `[1] == [1.0]` is
  false). Pinned by a differential test.
- **Binary-op type errors read the same on every tier.** A promoted
  function raising e.g. `[1] < [2]` said "Unsupported operation:
  LessThan" while the interpreter said "Invalid binary operation: cannot
  apply '<' to List and List"; `1 && 2` produced a bare "Invalid binary
  operation" with no detail. The VM now emits the interpreter's message,
  naming the operator and both operand types (mixed Int/Float pairs
  report their real types). Pinned by an error-text differential test.
- **REPL: a `//` comment containing a bracket no longer traps the session
  in multiline mode.** `1 + 1 // {` used to buffer forever (swallowing
  even `quit`) until a manual `:end`; bracket counting now stops at a
  comment, while `//` inside a string still counts as content.
- **REPL: `:type` no longer mutates the session.** `:type n = n + 1`
  executed the assignment it was asked about; a binding or assignment is
  now refused with a pointer to query the value instead.
- **REPL: interactive tutorials exit on EOF.** With a closed or piped
  stdin, `:tutorial_run` re-prompted forever at 100% CPU; end-of-input
  now ends the tutorial (and `quit`/`exit` work alongside `q`).
- **REPL: `:search` filters compose and validate.** Combined filters
  (`category:X limit:N`) previously cancelled each other, `limit:05`
  or an invalid `limit:` silently returned zero results, and the quoted
  phrases the usage examples themselves suggest never matched. Filters
  are now parsed token-wise, a bad limit is reported, and quotes are
  stripped.
- **REPL: `:help STATS` no longer advertises calls in casing the language
  rejects** — module listings echo the canonical lowercase name.
- **A missing script file is named in the error.** `olang nope.ol` said
  only "No such file or directory"; it now says which path it couldn't
  read.
- **Help accuracy: eight `:help` entries corrected against the
  implementation** — `csv.sort_by_column`'s boolean is *ascending* (the
  doc said descending), `csv.add_column` takes `(csv, values, name)` not
  `(csv, name, values)`, `csv.filter_rows` takes an integer column index,
  `csv.from_json` returns rows not CSV text (book table fixed to match),
  `str.fmt` and `ods.read_csv` return bare values not `Result`,
  `colx.partition` returns a tuple, and `plot.ramp` also accepts `auto`.

- **`olang check` warns on dead assignment to a captured binding.**
  Assigning to a variable captured from an enclosing scope inside a
  closure or function has no effect — capture is by value, so the write
  hits the snapshot and the outer variable never changes. The checker now
  flags it. It is provable (no false positives): the target is bound
  strictly outside the current function boundary; params, locals, and
  top-level reassignment are untouched. This is the language's sharpest
  footgun, and the lint makes it loud instead of silent. Fixing it also
  surfaced a real latent bug — `examples/03_algorithms.ol`'s
  closure-over-a-map memoization never actually memoized; it is rewritten
  to thread the cache, the correct olang idiom.
- **Empty-collection truthiness now agrees across tiers.** A promoted
  (bytecode-tier) function used an incomplete condition test that treated
  an empty string, list, tuple, or range as *truthy* — so `if xs => …` or
  `while xs { … }` on a promoted function took the wrong branch for an
  empty `xs`, disagreeing with the interpreter (which correctly treats
  them as falsy, like `0`/`false`/Unit). The tier's `is_truthy` now
  mirrors the interpreter's `to_boolean` exactly, pinned by a differential
  test. Found while writing the new Common Pitfalls chapter.
- **Name-colliding functions now promote to the bytecode tier (viz
  finding #3).** The tier dispatches `CallNamed` by name and marked any
  name shared by two distinct function bodies "ambiguous", never tiering
  it. Because embedded packages register their *private* helpers (`viz`'s
  `col`, `opt`, `groups`, `distinct`, `fmt`, …) into the global name
  space, any program reusing one of those common names — a data app, the
  chart gallery — silently ran that function, and its hot `map`/`filter`
  loop, on the interpreter. An ambiguous name is now dispatched **by body
  identity** (compiled under its own closure, keyed on the body pointer),
  so the correct body runs on the tier. A `use viz` program mapping a
  field-accessor lambda over records went **~1480 ms → ~170 ms native
  (~8.6×)** and **~1480 ms → ~140 ms in the wasm playground (~10×)**;
  results are identical on both tiers.

### Documentation

- **The docs are centered on the Open Language identity.** A new
  [Openness chapter](docs/openness.md) — now the book's opening chapter —
  states the three-pillar identity (open code, open artifacts, open
  execution), why it is uniquely possible for olang, and maps each pillar
  to its detailed home. The book landing, the repo README, and the
  website hero/metadata all lead with it; the Cargo package description
  follows. The name `olang` is unchanged — it already stands for *Open
  Language*.

- **New chapter: [Common Pitfalls](docs/pitfalls.md).** The language's
  sharp edges collected in one place with the idiom that avoids each —
  missing map keys returning Unit, integer division, the `&&`/`||` and
  range precedence surprises, `Result`/struct equality, closure capture,
  truthiness, indexing/slicing, empty-list builtins, `match`-pattern
  binding, bare-block scoping, and `os.args` argv. Wired into the book,
  the website, and the doc-examples test (every example is executed).
- Documented the three embedded packages `ui`, `viz`, and `dash` with
  function tables in the stdlib chapter (previously prose-only; `ui.esc`,
  `dash.stat`, and `dash.half` were undocumented), corrected the stdlib
  module count, completed the tooling chapter list, and added operator
  associativity + non-standard-precedence warnings to the language
  reference.

### Changed

- **`olang build` embeds the parsed AST, not just source (rung B).** A
  built standalone now carries its program's *pre-parsed* AST and
  deserializes it at startup instead of re-running the parser —
  deserialize is ~20× faster than parsing (a 400-function tool's startup
  parse of ~12 ms becomes ~0.6 ms, ~10 ms off cold start). The source is
  bundled too, only so runtime error snippets still render; bundles built
  before this (raw source, `oLaNgBnd`) still run. The AST round-trip is
  lossless across the language (enums, structs, closures, patterns,
  recursion), verified against the interpreter. Rung B of `olang build`
  (Toolsmith T4).
- **Faster `map`/`filter` on the bytecode tier.** The native `map`/
  `filter` loop now fetches the callee's bytecode and validates its arity
  and parameter checks once, before the element loop, instead of on every
  element (an `execute_prepared` fast path). ~16% faster on the hot
  builtin-lambda pattern natively; the JIT path is untouched.

- **Embedded packages parse once per process (faster cold start).** The
  built-in olang packages (`cli`, `term`, `viz`, `dash`, `colx`, `mathx`,
  `ui`) are compiled into the binary as source and were re-parsed on
  every `use` — and a fresh interpreter, which re-parses, is created for
  every CLI invocation, every `olang test` file, and every playground
  run. Their parsed AST is now cached process-wide (the source is
  immutable), so the parse is paid once. A 30-file `olang test` where
  each file uses `cli`+`term` dropped from ~111ms to ~44ms (2.5×), and
  the playground's repeat runs skip embedded parsing entirely. Lane T5 of
  the **Toolsmith campaign**.

## [0.57.0] - 2026-08-14

### Added

- **`survey` — the command-line flagship (dogfood).** A new example
  (`examples/survey`): a codebase surveyor that turns a directory into a
  report — colored totals, a per-language bar chart, aligned tables, and
  a live progress bar — exercising `cli`, `term`, and `fs` in one
  self-contained file. It is the terminal counterpart of the tracker web
  suite: written with the toolkit, shippable via `olang build`,
  documentable via `olang doc`. `term.table` also gained **visible-width
  alignment** (`term.visible_len`), so colored/styled table cells now
  align correctly.

- **`olang doc` — API reference from doc comments.** A new subcommand
  that scans `.ol` files for a source-level doc convention — `//!` for a
  module note, `///` above a declaration for its docs (both already
  valid olang comments) — and renders a browsable, suite-themed HTML
  page (or Markdown with `--md`). Only documented declarations appear.
  The `cli` and `term` packages now carry `///` docs, so their reference
  regenerates from source. Lane T6 of the **Toolsmith campaign** — the
  "maintain" layer.

- **`olang build` — standalone single-file executables.** `olang build
  prog.ol -o tool` bundles a program into a self-contained binary that
  runs with no olang installed. No C compiler or linker: `build` copies
  the runtime and appends the parse-checked source with a trailing
  marker; at startup the binary detects it and runs the embedded program
  with the full process argv (so a bundled tool's own flags reach
  `cli.args()` intact). The whole stdlib and the embedded packages
  (`cli`, `term`, …) travel inside it; single-source tools bundle
  cleanly. `examples/greet.ol` is a worked example. Lane T4 (Rung A) of
  the **Toolsmith campaign** — the "ship it" half of the build-and-ship
  arc.

- **`term` — the terminal toolkit.** A new embedded olang package
  (`use term`): ANSI color and text styling (`red`/`green`/`bold`/… and
  a general `style(s, opts)`), aligned `table`s and `rule`s, a progress
  `bar`, and interactive `prompt`/`confirm`/`select`. Styling gates on
  whether it will render — stdout is a TTY and `NO_COLOR` is unset, or
  `CLICOLOR_FORCE` is set — so the same program is colored on a terminal
  and plain in a pipe with no extra logic. Two native primitives back
  it: `os.is_tty()` and `os.flush()`. The `taskcli` example colorizes
  its summary and renders a `term.table` (plain when piped). Lane T2 of
  the **Toolsmith campaign**.

- **`cli` — declarative command-line argument parsing.** A new embedded
  olang package (`use cli`) that turns a program's argument surface into
  a spec map: typed flags (`bool`/`int`/`float`/`string`, short + long,
  defaults, `required`, env fallback), positional arguments,
  subcommands, and auto-generated `--help`. `cli.parse(spec, argv)` →
  `Ok(values)` \| `Err(message)`; `cli.help(spec)` renders usage;
  `cli.args()` is `os.args()` with the program path dropped. The
  `taskcli` example is rewritten onto it — its whole command surface
  (`list`/`open`/`stats`/`add --priority`) is one spec, with help and
  clean exit codes for free. First lane of the **Toolsmith campaign**
  (build & ship real command-line tools in olang).

## [0.56.0] - 2026-08-14

### Added

- **`dom.state` — a session-state primitive.** `dom.state_set(key,
  value)` / `dom.state_get(key)` name the DOM-resident-state pattern
  every closure-by-value browser app rediscovers: a blessed,
  JSON-typed, page-lifetime store (a Map or list round-trips; a
  missing key reads as Unit; not persisted — `storage_*` is the
  localStorage path). The charts page's cross-filter now rides it
  instead of a hidden input.

### Changed

- **`if`'s `=>` may start a new line.** A long condition can wrap
  before the arrow (`if a && b && c\n    => ...`) — the syntax
  friction that bit most when dogfooding viz. Same-line and `else if`
  chains are unchanged.
- **Charts use whole-number gridlines for count data.** `nice_ticks`
  detects whole-valued data and floors a fractional step to 1, so a
  bar chart of counts reads 0,1,2,3 rather than 0,0.5,1,…. Float data
  keeps fractional ticks.

## [0.55.0] - 2026-08-13

### Added

- **Embedded packages import each other.** The builtin olang packages
  (`ui`, `viz`, `dash`, …) can now `use` one another — the resolution
  already handled it (embedded modules resolve before the filesystem,
  natively and in wasm), but nothing used or tested it. `dash` now
  imports `ui`'s HTML escape (`use ui { esc }`) instead of
  re-implementing it, so the discipline lives in one place, with a
  test pinning that loading one embedded package transitively loads
  its imports.

- **Vectorized Series math — `ods.map`.** `ods.map(series, name)`
  applies a `math.*` unary function (sin, cos, exp, sqrt, ln, floor,
  … — 24 in all) across a whole column in one native kernel pass. The
  name is a String, not a closure, precisely so the loop stays in the
  kernel: it is the vectorized form of `map(xs, (v) => math.f(v))`
  with no per-element boundary crossing. Bit-identical to the scalar
  function (same `f64` methods), nulls propagate, domain-restricted
  functions raise the same error, and it composes with the existing
  elementwise Series arithmetic into full expressions
  (`ods.map(xs, "sin") * 2.0 + 1.0`, one kernel per term). A sin·2+1
  transform over 50,000 points: **238 ms → 1 ms (~240×)**, same
  checksum — and it runs in the interpreter kernel, no promotion
  needed. The gallery's 50k-point curtain is now generated this way.

- **In-place list building — the AddAssign of collections, on every
  tier.** `xs = xs + [v]` in a loop was O(n²): each iteration copied
  the whole list. Now it appends in place when the accumulator holds
  the only reference to its `Vec`, O(1) amortized, with an
  `Arc::get_mut` aliasing guard identical to the string case — a
  snapshot taken before an append is never mutated, self-append
  copies, a nested list is unharmed. The **bytecode tier** got this
  first (a promoted 60,000-element build: **3807 ms → 2 ms**), and now
  the **interpreter** does too: `Value::List` moved from a fixed-size
  `Arc<[Value]>` to a growable `Arc<Vec<Value>>` (reads deref
  identically, so it was a handful of edits), and the assignment path
  gained the same fusion. An unpromoted interpreter build of 8,000
  elements: **215 ms → 1 ms**. Cold, hot, native, and wasm paths are
  all O(n) now, pinned by interpreter-only and cross-tier differential
  tests.

## [0.54.0] - 2026-08-13

### Added

- **`plot` grows into a charting library.** Five new chart types —
  `plot.area`, `plot.bars` (grouped), `plot.stacked`, `plot.heatmap`
  (sequential color scale with a min/max key), and `plot.box`
  (five-number summaries) — joining line, scatter, lines, bar, and
  hist. Two new options on every chart: `theme: "dark"` re-tunes the
  full palette (surface, ink, grid, series hues, heat scale) for dark
  pages, and `responsive: true` drops the fixed pixel size so the SVG
  fills its container while the viewBox keeps the aspect ratio.
  Stacked bars refuse negative values; box plots drop nulls.
- **The `viz` grammar — charts as values.** `use viz`, an embedded
  olang package: a chart is a spec map holding data (records or a
  Frame), a mark, and column-name encodings. `color` splits rows into
  series, bar marks aggregate rows sharing a category (`"stack": true`
  stacks), `layers` composes marks over shared scales, and
  `viz.chart(spec)` compiles to plot SVG — pure, natively tested.
  `viz.draw(canvas, spec)` compiles the same xy specs to a canvas
  draw-list for point counts SVG can't carry. Underneath, `plot.xy`
  is new: layered mixed marks (line/area/scatter, each with its own
  x) in one document over shared scales.
- **The color system — expressive, not monochrome.** Ten curated hues
  per theme (mint leads dark, classic ten on light). Every chart can
  own its palette (`colors`), single-series bars can color each
  category (`vary`), heatmaps pick multi-stop ramps (`scale`:
  ocean/ember/thermal/diverging — diverging runs cold through the
  surface to warm for signed data), and `plot.ramp(name, t)` exposes
  the ramps to olang code. The viz grammar gains **`color_by`** —
  continuous color encoding per point, rendered individually in SVG
  and bucketed into at most 24 bulk calls on canvas. `dash.stat` adds
  accent-colored KPI tiles. The pages put it to work deliberately:
  varied status bars, an ocean heatmap, violet boxes and histograms,
  cyan-amber cumulative charts, a diverging interference field, an
  ember-ramped attractor, a magenta dust arm in the galaxy, a
  four-voice rose, and a curtain that sweeps the spectrum.
- **Fidelity and cohesion — the suite becomes one program.** Every
  canvas now renders at devicePixelRatio (the backing store scales
  once, the context pre-scales, all drawing stays in design units) —
  the blurriness on retina displays is gone. Charts are designed at
  the size they display: pages pass card-matched viewBoxes, tick and
  category type grows to 12.5px, titles to 16.5px, and the two-column
  grid gives every chart room to read. One design system
  (`suite.css`, served by the app) carries the tracker's palette to
  all seven pages — mint `#3ddc97` leads the dark series order, chart
  surfaces match the card panels, and the nav, headers, cards,
  buttons, and inputs share one look. The viz canvas target, tooltip,
  and dash stylesheet all speak the same tokens.
- **The roadmap gains the viz campaign's findings** — the
  language-level growth list the campaign surfaced (in-place list
  append as the AddAssign of collections, vectorized Series
  transforms, embedded-package imports, a session state primitive,
  syntax friction), recorded so the next lanes aim where data work
  actually pushed back.
- **The `dash` kit and the ops board.** `use dash`: KPI tiles, cards,
  wide cards, and the grid as pure HTML builders (escaped, natively
  tested) plus `dash.styles()` so a dashboard page ships no CSS. The
  flagship `/board.html`: a KPI row and five viz charts over live
  tracker data, a status filter carried in the URL (bookmarkable,
  back-button correct), one delegated tooltip for every chart, and a
  5-second auto-refresh — one olang source file end to end. Fetch
  failures now keep the last render on both data pages instead of
  erroring.
- **The beauty pass.** Area marks fill with a vertical gradient fading
  toward the axis (SVG, both themes); `dom.draw_points` gains a
  host-side rotation term; the gallery's spiral becomes a 12,500-star
  galaxy spinning on that one parameter, and the finale curtain grows
  to 50,000 points — still at frame rate.
- **The binary bulk path — big data at frame rate.**
  `dom.draw_points(canvas, xs, ys, style)`: coordinates cross the wasm
  boundary as ONE packed f64 buffer the page reads as a zero-copy
  typed-array view — no JSON, no per-point cost. Series are the fast
  lane (the data stack feeds the graphics pipeline directly); plain
  lists work too; nulls drop pairwise. Styles: points or path mode,
  color, size, alpha, and a host-side affine (sx/sy/tx/ty) — so
  animation re-sends the same buffer with new transform parameters and
  olang does zero per-point work per frame. `viz.draw` compiles point
  and line marks onto this path, and specs whose data is a Frame keep
  columns as Series end to end. The gallery finale animates a
  30,000-point Lissajous curtain at ~120 fps.
- **Interactive charts — hover, click-to-filter, brush.** With
  `"interactive": true` (plot option and viz spec key), marks carry
  their datum as `data-*` attributes: scatter points, bars and stacked
  segments, heatmap cells, and boxes become event targets, escaped and
  off by default. Three viz helpers ride the existing structured-event
  path: `viz.tooltip(el)` (floating datum tip on hover),
  `viz.on_mark(el, event, handler)` (handler fires only on mark hits,
  receiving the datum), and `viz.brush(el, handler)` (horizontal
  press-drag-release as width fractions). The charts page cross-filters
  every card from clicked status bars; the gallery forecast zooms by
  brushing and resets on double-click.
- **The data stack meets the page.** Two new pages in the example app:
  `/charts.html` — live tracker analytics (one `dom.fetch_json`, then
  `ods.frame_from_records`, frames, and plot SVG landed with
  `dom.set_html`) — and `/gallery.html`, the standing data-viz
  showcase (computed art and statistical pieces, plus a live draw-list
  centerpiece) where new viz capabilities land first. Both wired into
  the suite nav.

## [0.53.0] - 2026-08-13

### Added

- **`dom` workers — browser parallelism out of the box.** `dom.worker(path)`
  boots a second olang program in a Web Worker (its own thread, its own wasm
  instance); values cross as JSON both ways via `dom.worker_send` /
  `dom.worker_on` on the page and `dom.post` / `dom.on_message` inside the
  worker. Workers can post mid-computation, so long jobs stream progress
  while the page's frame loop keeps running. New `/primes.html` demo in
  `examples/app`: a prime counter with a live progress bar and an animation
  dial proving the main thread never blocks.
- **`dom.fetch_json`** — `dom.fetch`, but the callback receives the parsed
  response value directly instead of raw text.

### Changed

- **The example app is now a linked suite.** All four pages — the tracker,
  `/orbit.html`, `/notes.html`, `/primes.html` — share a nav, link to each
  other, and link their own `.ol` source; the primes button follows the
  worker lifecycle (disabled until ready and while counting).
- **The browser chapter caught up with the platform.** `docs/wasm.md` now
  documents the full dom surface: structured event Maps, the draw-list and
  frame loop, the `ui` view layer, routing and storage, Web Workers, and
  `fetch_json`, with a guided reading of all four frontends. `docs/ovm.md`
  documents the scratch watermark, inlining + scalar replacement, the
  collection/Result/Map JIT kinds, and tier-identical error traces;
  `docs/language.md` documents runtime stack traces.
- **Sophisticated web apps: the `ui` view layer, routing, and
  storage.** `use ui` — an embedded olang package — builds pages as
  values: `h(tag, attrs, children)` trees, `hk` reconciliation keys,
  pure `html()` rendering (escaped, testable natively), and
  `ui.render(el, children)` with keyed reconciliation: unchanged
  children untouched (input state and focus survive), changed ones
  re-rendered in place, additions/removals surgical, reorders
  repositioned via the new `dom.insert_before`. Alongside it the dom
  module gains SPA navigation — `push_state`, `location` (a Map of
  path and query), `on_route` for back/forward — and localStorage
  (`storage_get`/`set`/`remove`). The proof is
  `examples/app/static/notes.ol` at `/notes.html`: a notes SPA
  verified live in a browser — selection updates the URL, the back
  button unselects through `on_route`, additions reconcile in, and
  notes persist across the session. The dom harness pins routing,
  storage, and the reconciliation algebra (update-in-place, removal,
  reorder) against the real wasm build; three native tests pin the
  pure renderer.

- **Rich graphics out of the box: canvas draw-lists.** A scene is plain
  olang data — a list of op maps (`clear`, `rect`, `circle`, `line`,
  `path`, `text`, plus `save`/`restore`/`translate`/`rotate`/`scale`) —
  submitted with one `dom.draw(canvas, ops)` call per frame and
  replayed onto the canvas 2D context by the page. `dom.on_frame`
  completes the loop: register once, called every frame with a
  millisecond delta, no per-frame handler registration. The proof is
  `examples/app/static/orbit.ol`, served by the tracker at
  `/orbit.html`: an animated orbital system with motion trails where
  clicking adds a body at the clicked radius (structured event
  coordinates + `dom.measure`), verified live in a browser at 60fps.
  The dom harness pins the draw-list round-trip — op order, numeric
  fidelity, nested point lists — against the real wasm build.

- **The dom module grows up: structured events, node control, and
  time.** Every event handler now receives a structured event Map —
  `type`, target `id` and `value`, `key`, pointer `x`/`y`, modifier
  flags, and the target's `data-*` attributes — the same shape for
  every event, so any DOM event name works (`input`, `keydown`,
  `pointermove`, `submit`, `wheel`, …) and delegation stays the natural
  style. Sixteen new functions land alongside: attribute get/set/
  remove, classList add/remove/toggle, per-property styles,
  `dom.measure` (bounding rect as a Map), surgical `create`/`append`/
  `remove` structure edits, `scroll_into_view`, timers
  (`set_timeout`/`set_interval`/`clear_interval`), and
  `dom.request_frame` for animation loops with a millisecond delta.
  Structured payloads ride a new JSON dispatch entry over the wasm
  boundary; the tracker app is migrated to the Map shape and verified
  live in a browser (delegated clicks, change edits, and Enter-to-add
  all round-tripping to the server), and the dom harness proves the
  whole surface end to end against the real wasm build.

## [0.52.0] - 2026-08-13

### Added

- **Scalar replacement — structs and lists that never escape are never
  allocated.** At planning time the JIT now inlines tiny leaf callees
  (a constructor, a field-math helper) into their callers, propagates
  single-definition copies, and then dissolves aggregates: a struct
  whose register is only ever field-read becomes one register per
  field; a list read only at constant indices becomes one register per
  element. Construction disappears entirely — no allocation, no helper
  call, no guard — and the loop that built two structs per iteration
  compiles to pure float arithmetic. Measured: the struct-building
  benchmark drops from 1.475s to ~14ms (105x, now ~4x faster than V8
  on the identical workload) and the list-building one from 0.223s to
  ~7ms (30x, ~8x faster than V8). The transforms exist only on the
  JIT's planning clone — the VM's bytecode is untouched, and every
  error path deopts to a clean rerun, so semantics and stack traces
  cannot drift.

- **Loops may allocate now — the scratch watermark.** Native loops were
  forbidden from allocating (every scratch allocation lived until the
  call ended), which kept struct-, list-, map-, and Result-building
  loops on bytecode. The JIT now marks every scratch family's length at
  loop entry and truncates back to the mark on each taken back-edge,
  freeing the iteration's allocations — permitted exactly when a
  liveness gate proves every heap value born in the loop dies in its
  iteration (nothing carried across the back-edge, nothing read after
  the loop; values that escaped into another owner survive on their own
  reference count). Loops that fail the proof refuse and run on
  bytecode, byte-identically, with a precise debug diagnostic.
  Measured with `olang bench` against a saved baseline: struct-building
  loops 60% faster (2.5x), list-building loops 56% faster (2.3x),
  scalar kernels untouched, and allocation-heavy loops now run past the
  old 1M scratch cap natively instead of deopting mid-loop.

- **`olang bench` — reproducible timings and a regression guard.** Each
  `.ol` file runs as its own subprocess (fresh VM and JIT per run, the
  wall-clock a user experiences): one discarded warmup, then timed runs
  with median, min, max, and coefficient of variation per row, plus a
  warning when runs disagree on their output. `--save base.json` stores
  a baseline; `--against base.json` compares to one, calling a row
  changed only when it moves more than max(5%, 2×CV) — below that it's
  noise, not news; `--fail-on-regress` turns red rows into exit code 1,
  making a saved baseline a standing guard for performance work.
  Documented in the tooling chapter.

## [0.51.0] - 2026-08-13

### Fixed

- **Trivial constructors no longer pay the JIT call boundary.** A body
  that exists to allocate — `fn make(i) = [i, i * 2]` and its struct,
  map, and Result siblings — does the same allocation on every tier, so
  crossing the bytecode→native boundary (argument marshalling, scratch
  context, retain resolution, unmarshal) was pure cost: ~26% slower
  than v0.50.0 on a 3M-call constructor loop. Allocating bodies with at
  most eight compute instructions now decline the boundary and stay on
  bytecode, while still compiling as group members reached by direct
  native call. Boundary calls that do run got cheaper too: all-scalar
  calls skip the per-family argument sweep the collections lane added,
  and the scratch context is reused across calls (native calls never
  nest), so allocating bodies stop paying a malloc/free per call for
  bookkeeping. Trivial and compute-heavy constructor loops are both
  back at (or slightly better than) v0.50.0 timings, and guard tests
  pin the policy via the new `jit_native_calls` tier stat (also shown
  by `--ovm-stats`).

### Added

- **The JIT builds lists.** List literals (`[a, b]`) and list
  concatenation (`xs + [v]`) compile to native code in straight-line
  functions, and list-returning constructors hand ownership back across
  the entry boundary — previously any list construction or list return
  refused the JIT outright. Construction is scratch-owned like structs
  (a deopt can never leak), uniform `Int`/`Float` elements specialize,
  and everything else — mixed elements, string lists, empty literals,
  list-building loops — stays on bytecode with identical results. `->
  List` return annotations discharge statically. Eleven differential
  tests pin the seam.

- **Struct-element lists compile, and native callers see through their
  callees.** `[Point{..}, Point{..}]` literals specialize as struct
  lists: the helper resolves each borrowed element pointer back to an
  owned Arc, so the list owns its elements exactly like the VM's (mixed
  shapes and struct/scalar mixes refuse to bytecode). Alongside it, a
  latent inference gap is fixed: indexing or field-reading a value built
  by a *callee* used to refuse the whole function because the callee's
  return kind resolves one fixpoint iteration late — it now defers
  instead, so constructor-then-consume pipelines (`let pts =
  segment(a, b); pts[1].x`) compile end to end.

- **String-element lists join the JIT.** `ListStr` is the fourth list
  kind: `["alpha", "beta", tag]` literals construct natively, elements
  read back as borrowed strings (comparison and concat work on them
  directly), and string lists concat, return, and iterate like every
  other list kind. Constant elements are content-cloned — strings are
  immutable values with content equality, so identity is unobservable
  and there is no deopt case beyond the allocation cap.

- **Maps enter the JIT — the collections lane is complete.** String-keyed
  maps with a uniform value payload specialize: `#{...}` literals
  construct natively, `map_get` compiles as a key-guarded read (a miss —
  where the language returns Unit — deopts to bytecode, never misreads),
  `map_has_key` is a total native test that compiles inside loops, and
  `map_set` clone-and-inserts into scratch exactly like the VM's
  immutable-map native. Because these are *named builtins* and a user
  definition can shadow a builtin name, compiled code records which
  natives it baked — transitively through call-graph edges — and the
  moment a user function takes one of those names, every affected entry
  is demoted back to bytecode. Mixed-value maps, stringified non-string
  keys, and object receivers all stay on bytecode with identical
  results; `-> Map` return annotations discharge statically.

- **String building is linear now.** The accumulate pattern `s = s +
  piece` fuses into a single instruction that appends in place when the
  string is uniquely referenced — the Arc count proves no alias can
  observe it, and strings are immutable values with content equality,
  so identity is unobservable. Building a 400KB string by 200k
  concatenations drops from ~1.6s to ~10ms (it was O(n²), it is now
  O(n)) — faster than CPython's specialized in-place append on the
  same workload. Aliased and self-referencing strings copy exactly as
  before, fusion declines when the right side could assign, and the
  JIT canonicalizes the fused form back to a plain add so hot numeric
  loops compile unchanged.

- **Channels: `spawn`ed tasks can talk.** The new `chan` module is
  message passing between tasks — multi-producer multi-consumer queues
  of olang values. `chan.new()` is unbounded; `chan.bounded(n)` holds
  at most `n` in-flight messages and blocks senders when full
  (`bounded(0)` is a rendezvous). `send`/`recv`/`try_recv`/
  `recv_timeout` all speak Result, and closing is cooperative: after
  `chan.close(c)` sends fail but queued messages still drain, so a
  consumer loop just matches on `recv` and stops on `Err`. Handles are
  plain values that cross the `spawn` boundary like anything else;
  heap values (lists, maps, structs) travel intact. Documented in the
  stdlib chapter with a tested producer/consumer example.

- **The checker proves match exhaustiveness over literal enums.** A
  union of literals declares exactly which values are admissible, so
  `olang check` (and the editor) now knows what covering every case
  means: a `match` missing a member gets an advisory warning naming
  what's absent (`match is not exhaustive: "done" has no arm — add it
  or a catch-all`), an unguarded binding or `_` covers everything, and
  a match whose arms cover *none* of the admissible values is reported
  as a runtime error — it fails on every run. Or-patterns count member
  by member; guarded arms prove no coverage (a guard may reject);
  dynamic scrutinees are never judged. Documented in the types chapter.

- **Runtime errors on the bytecode tier carry the same trace as the
  interpreter.** A runtime failure now points at the innermost located
  statement and lists the call stack — identically on every tier.
  Compiled bytecode carries statement-granularity span markers, the VM
  records frames as the error unwinds (with the interpreter's exact
  frame-visibility semantics, quirks included), and the tier splices
  its trace onto the interpreter's live stack. Previously a tier-run
  error pointed at the top-level statement with no stack at all. Seven
  differential tests pin the whole ErrorLocation equal across tiers.

### Fixed

- **The bytecode tier reads operands in interpreter order.** A binary
  op whose left operand was a plain variable compiled to the variable's
  register itself, so `acc + { acc = 1  5 }` read `acc` *after* the
  right side mutated it — the tier answered 6 where the interpreter
  answers 15. The same late-read hazard lived in every multi-operand
  position: call arguments (and the callee and method receiver
  themselves), list/tuple/map/struct/anonymous-object literals,
  template interpolations, index and range operands, a match scrutinee
  re-tested after an assigning guard, and a for-loop iterable
  reassigned by its own body. The compiler now shields exactly the
  provable case — the operand's register is a variable's home register
  AND a later operand contains an assignment (the `assignment_free`
  whitelist from AddAssign fusion) — by copying the value to a fresh
  register at its evaluation point; assignment-free operands, i.e. all
  hot paths, emit no Move. Fourteen differential tests pin every
  position, each also asserting promotion so a silent refusal can't
  fake agreement.

- **The JIT enforces parameter annotations.** A specialization whose
  observed argument kinds could not provably satisfy the function's
  parameter annotations compiled anyway and skipped the check — so a
  hot `fn double(n: Int)` called with a Float returned a wrong value
  instead of the type error every other tier raises (present since the
  JIT and gradual typing first coexisted; surfaced by the new trace
  tests). Parameter annotations now discharge statically exactly like
  return annotations: unprovable specializations stay on bytecode,
  which checks per call.

## [0.50.0] - 2026-08-12

### Added

- **The JIT compiles Results.** `Ok`/`Err` construction, pattern tests,
  and payload extraction now run natively: Results ride borrowed
  pointers like structs, payload reads are guarded per side (a surprise
  deopts to bytecode, never misreads), and construction is scratch-owned
  so a deopt can never leak. Mixed return paths join — a function
  returning `Ok(n)` on one branch and `Err(code)` on another compiles as
  one specialization — and calls carrying the side a specialization
  never saw fall back to bytecode with identical results. `-> Result<T,
  E>` return annotations are discharged statically: the JIT only
  compiles a function whose inferred payloads provably satisfy the
  annotation. String payloads extract natively; string construction and
  Results built inside loops stay on bytecode by the same allocation
  discipline as structs. Eleven differential tests pin the seam shut.

- **The checker and language server see across module boundaries.** A
  file's `use`d modules are resolved with the runtime's local
  conventions and parsed alongside it: imported `share fn` signatures
  feed diagnostics in both `olang check` and the editor (a wrong
  argument to an imported function is flagged in the importing file),
  hover shows an imported function's typed signature marked with its
  source file, and go-to-definition jumps into the module. Also
  closed in passing: `share`d declarations were entirely invisible to
  the checker even within one file — their signatures now register and
  their bodies are checked (the corpus's 171 files, heavy with
  `share fn`, stay clean).

- **`olang check` warns when code relies on block leakage.** A bare
  block's `let`s remain visible afterwards — the other long-documented
  scoping pitfall. Using (or assigning) such a name after its block now
  draws an advisory warning, once per name, in `olang check` and the
  editor. Leak tracking is per function frame, so a block inside one
  function never taints another; declaring the name before the block
  is the fix and stays silent. The repo's 171 files produce zero.

- **Literal types check by value — unions of them are lightweight
  enums.** The last useful reserved annotation form graduated:
  `s: "open" | "in-progress" | "done"` admits exactly those strings
  (`parameter 's' of set_status expects "open" | "in-progress" |
  "done", got "cancelled"`), with int and bool literals equally valid
  branches. Enforced by one value comparison at every boundary on
  every tier; the checker proves violations when the argument is
  itself a literal (or a literal-annotated binding) and stays silent
  on dynamic values; scalar actuals now name their value in error
  text on both tiers ("got true", not "got Bool"). Intersection
  annotations remain the only reserved form.

- **The tracker example is a real product now.** `examples/app` grew
  from a bare grid into a full app — live search, status filter pills,
  sortable columns (assignee joined the sortable set), an issue drawer
  with editable title, cycling pills, and comments, a stats strip
  rendered from `/api/stats` (quantiles computed server-side on ods),
  and a live activity ticker — all in ~330 lines of olang running as
  WebAssembly, same stateless/delegated architecture as before (client
  state is four hidden inputs; twelve listeners bound once at boot).
  The `dom` module grew one function to make it possible:
  `dom.set_class(el, classes)` sets an element's class list wholesale —
  the stateless way to toggle visual state (a drawer's `open`, a
  pill's `active`). The wasm.md guided reading walks the new frontend.

### Fixed

- **TOML datetimes parse as plain strings.** The toml crate represents
  datetimes as a private one-key wrapper object over serde, which
  leaked through `toml.parse` as
  `{$__toml_private_datetime: "..."}` instead of the string the
  documentation promises. Unwrapped recursively; found by the pre-0.50
  edge-case sweep and regression-pinned.

- **A vanished stdout reader no longer kills olang programs — or the
  HTTP server.** `print`/`println` used Rust's `println!`, which
  aborts the whole process with "failed printing to stdout" the moment
  a pipe consumer exits — so `olang gen.ol | head -1` panicked instead
  of ending, and (the root cause of the long-standing http_serve_test
  flake) a server whose parent read the port line and closed the pipe
  before the second boot line died at startup under load, resetting
  every in-flight connection. Program output now emulates the Unix
  SIGPIPE default — terminate quietly with the conventional 141 — so
  olang composes in pipes like any well-behaved filter, and the
  server's informational boot lines ignore a closed stdout entirely
  (the socket is its real interface). Verified with 24 consecutive
  suite executions under the load pattern that previously failed
  within two.

### Added

- **`olang check` warns on assignment to undeclared names.** `x = 1`
  without `let` creates a binding — long documented as a pitfall with
  "a future release may warn here"; this is the release. Advisory
  only: rendered as a warning in `olang check` (exit code unchanged)
  and as a warning squiggle in editors; declared names, loop
  variables, parameters, and valueless `let`s stay quiet, and the
  first assignment binds the name so it warns once. The repo's own
  171 files produce zero warnings.
- **`testing.test_summary()` and `reset_tests()` are real.** Every
  `testing.assert_*` outcome is tallied (per thread — `par_map`
  workers keep their own counts): `test_summary()` returns
  `#{ "passed", "failed", "total" }` and `reset_tests()` zeroes it.
  `run_test` deliberately remains an honest redirect to `test` blocks.

- **`olang --watch script.ol` — the edit-run loop as a flag.** Reruns
  the script whenever any `.ol` file at or below its directory changes
  (module edits count), each run in a child process so a crash or
  `os.exit` ends the run and never the watcher; a save landing mid-run
  queues an immediate rerun. Polling, no new dependency.
- **REPL `:type` reports deep types.** Previously the shallow name;
  now the value's structure: `:type [1, 2, 3]` answers `List<Int>`,
  `Ok([1.5])` answers `Result<List<Float>, _>`, and a lambda shows its
  annotated signature (`(Int) -> ?`). Mixed elements fall back to the
  honest base (`List`), never a guess.

- **`toml` module — parse and emit the config format olang itself
  uses.** `toml.parse(text)` yields the same value shapes JSON objects
  do (tables → Maps, arrays → Lists, datetimes → strings; both modules
  bridge through the same serde conversions, so they cannot drift),
  `toml.stringify(map)` emits pretty TOML (a document is a table, so
  non-table values are a clean `Err`), and `toml.validate(text)`
  answers without erroring. The `toml` crate was already in the tree
  via the package manager — this is surface, not a new dependency. The
  stdlib is now twenty-one native modules.

- **Scripts are first-class: shebang, stdin, and path helpers.** A
  leading `#!/usr/bin/env olang` line now parses (masked, not
  stripped — every error line number and span still matches the file
  on disk), so `chmod +x script.ol` works. `os.stdin()` reads all of
  standard input and `os.stdin_lines()` yields it as lines with
  endings stripped — the pipe primitives (`cat log | olang
  analyze.ol`). And `fs` gained path surgery: `join(parts)`,
  `dirname`, `basename`, `ext`, and `abs_path` (absolute against the
  current directory with `.`/`..` normalized lexically — the file
  need not exist). Environment access needed nothing: `os.get_env` /
  `set_env` / `has_env` / `list_env` and `os.home_dir` already
  existed and are now regression-covered alongside the new surface.

### Fixed

- **The tracker example no longer defaults to a port macOS owns.**
  `examples/app` listened on 7000 by default — a port macOS AirPlay
  Receiver (Control Center) binds on every modern Mac and answers
  with `403 Forbidden`, so a browser hitting the app when it wasn't
  running got AirPlay's baffling "access denied" instead of
  connection-refused. The default is now 7317 (docs updated;
  an explicit port argument still works as before). Also fixed in
  passing: the boot-time wasm-artifact check used `fs.read_file` on a
  binary file, which fails on non-UTF-8 bytes and warned "missing"
  even when the artifact was present — it now uses `fs.exists`.

### Added

- **Editor hover shows types.** The language server's hover now
  carries the static checker's knowledge: annotated function
  signatures render in full (`fn dist(a: Float, b: Float) -> Float`),
  and unannotated `let` bindings show their inferred type when the
  checker can prove one (`let total: Int` for `let total = 1 + 2`).
  Unknown stays honest — a binding the checker can't type hovers as
  plain `let mystery`. Also recorded: the JIT note from 0.49's Result
  entry is now precise — Result values have no JIT representation at
  all, so `-> Result<...>` annotations cost nothing on the native
  tier that Result-returning code didn't already forgo; a future
  Result kind in the JIT would need its own discharge rule.

- **`Promise` annotations check at non-async sites, and the checker
  follows `await`.** A parameter or binding declared `Promise<Int>`
  now rejects a non-promise value at the boundary on every tier
  (previously the dedicated annotation form reduced to no check). The
  payload at rest stays unchecked — it doesn't exist until resolution,
  where the async return check already enforces it — but the checker
  flows it: async function signatures join the checker's world
  (calling one yields `Promise<T, E>`; a bare `-> T` wraps), `await`
  carries the resolved type onward, and passing an unawaited call
  where the payload type is expected is flagged (`expects Int, got
  Promise`).

- **Function-type annotations enforce callability and arity.**
  `f: (Int) -> Int` now verifies at the boundary that the value is
  callable (function or builtin) and can be called with exactly the
  annotation's parameter count — respecting default-parameter ranges
  (`expects (Int) -> Int, got a function taking 2 parameters`), on
  every tier, with the signature (not the bare word "Function") in
  the error text. Values that don't expose parameter counts (builtins)
  check callability only. Signature *types* are the checker's
  territory: a lambda whose own annotations contradict the declared
  signature is flagged as a promise-break (`expects (Int) -> Int, got
  (String) -> ?`), and lambdas now carry their signature through the
  checker's inference.

- **Union annotations gained semantics — `A | B` is enforced.** The
  reserved form graduated, additively as promised: a value satisfies a
  union if it satisfies any branch, checked at every annotated boundary
  on every tier at the same O(1) cost (`parameter 'x' of tag expects
  Int | String, got Bool`). Branches keep their own rules — a
  `Result<Int, String> | Int` union applies the Result payload check
  when the value is a Result — and a union containing an unenforceable
  branch (generic parameter, function type) stays entirely unchecked
  rather than wrongly strict. The union grammar also learned the
  parameterized branch forms (`Result<...>`, `Promise<...>`, `Name<T>`,
  `Map<K, V>` now parse as branches). The checker mirrors the runtime
  with its usual honesty: a violation is reported only when every
  branch is provably violated, labeled a runtime failure only when
  every branch's own check would fire (`expects List<Int> | Int, got
  List<String>` is a promise-break — the shallow runtime admits the
  List). Intersection and literal-type annotations remain reserved;
  union type *declarations* remain not planned.

Releases before 0.23.0 predate this changelog and are not retroactively
documented.

## [0.49.0] - 2026-08-11

### Added

- **`Result<T, E>` annotations enforce their payloads.** Gradual
  typing stage 4: a Result holds exactly one payload, so — unlike
  containers — the O(1) boundary discipline allows a step more. An
  `Ok` value now checks its payload against `T` and an `Err` against
  `E`, shallowly (one name comparison; `Result<List<Int>, E>` checks
  an Ok payload is "a List"), at every annotated boundary on every
  tier with identical text: `return value of parse expects
  Result<Int, String>, got Ok(String)`. Functions returning annotated
  Results stay on the enforcing bytecode path (the JIT refuses,
  conservatively). The static checker goes deeper: `Ok`/`Err`
  literals decompose with paths (`Ok payload of parameter 'r' of f
  expects Int, got String`), `expr?` carries the Ok payload's type,
  `match` arms narrow `Ok(v)`/`Err(e)` bindings to their payload
  types, and deep Result types flow through annotated bindings
  (`expects Result<Int, String>, got Result<String, String>`). Every
  Result annotation in the repo's 171 files was already honest:
  nothing changed behavior, and dishonest ones now cannot land.

### Fixed

- **The website playground linked again — and its book nav caught up.**
  The playground worker's import object predated the `dom` module, so
  any freshly built wasm failed to instantiate
  (`LinkError: ... "host_dom_fetch": function import requires a
  callable`), taking the whole playground — most visibly the data
  stack — down with it. The worker now supplies the nine `dom` host
  imports as inert sandbox stubs (`dom.query` finds nothing, reads
  yield empty strings, writes are no-ops — there is no document in the
  worker), and ods/stats/Frame programs verified running in the
  browser at 0.48.0. The site's hand-maintained chapter registry also
  still listed the deleted design docs and lacked the new chapters; it
  now mirrors the book index (Types, Editors, and Tooling pages added,
  design entries gone).

### Documentation

- **The book gained a Types chapter** (`docs/types.md`) — gradual
  typing end to end: annotations as enforced promises, the three
  rules, exactly where the runtime enforces (every tier, async
  resolution, precomputed checks), what the static checker proves and
  its two diagnostic labels, element types, and an adoption playbook.
  Its examples are doc-tested like every other chapter.
- **The design documents folded into the book.** `docs/design/ods.md`
  and `docs/design/ods-lazy.md` are gone as separate files; their
  durable content — the problem statement, the one-array-both-tiers
  decision, the benchmarks-are-the-spec tables with every recorded
  revision, and the lazy-evaluation verdict with its reopening gate —
  now lives in the Data Stack chapter as "The design record" and "Why
  eager evaluation". The book index lists chapters, not design docs.
- **Stale claims corrected**: Stability no longer promises a "future
  opt-in static checker" (enforcement and the checker shipped in
  0.48.0 and are now documented as the contract); Internals no longer
  advertises the dead `type_checker.rs` module and now maps
  `tools/check.rs`; the root README's struct bullet ("values are
  dynamic") predated field enforcement and now states it, with gradual
  typing added to the feature list; the Tooling chapter and book index
  now name all three `olang` tools.

### Added

- **`olang check` sees element types.** The static checker now goes
  where the runtime's shallow checks deliberately don't: literals
  decompose against their annotations element by element, so
  `let xs: List<Int> = [1, "a", 3]` reports `element 1 of let binding
  'xs' expects Int, got String`, with the path spelled out through
  nesting (`element 1 of element 1 of ...`), `Map<K, V>` keys and
  values, tuple arity and elements, and struct-literal fields against
  their declared field types. Deep types flow through annotated
  bindings and known return types, so passing a `List<String>` binding
  where `List<Int>` is declared is flagged too. Findings are labeled
  by kind: base-level violations say "this would fail at runtime" and
  keep the runtime's exact error text; element-level breaks — which
  the O(1) runtime checks let pass — say "the annotation's promise is
  broken here". The no-false-positive discipline is unchanged
  (anything unprovable is silent; the repo's 171 files still check
  clean), and pattern bindings (loops, lambdas, match arms, catch)
  now correctly shadow outer annotated names during analysis.

## [0.48.0] - 2026-08-10

### Added

- **`olang check` — the static side of gradual typing.** A new
  subcommand (and LSP integration) that reports type-annotation
  violations the runtime would provably reject, before the program
  runs: a literal argument against an annotated parameter, an
  annotated `let` initialized with a known-type value, a declared
  return contradicted by what the body provably produces, and
  wrong-arity calls to known functions. Its discipline is no false
  positives — anything the checker cannot prove stays silent, and
  unannotated dynamic code is never judged (the whole repo's 171 `.ol`
  files check clean). Diagnostics carry the runtime's exact error
  text and render miette-located with the source line and caret;
  non-zero exit on any finding makes it CI-ready. The same checker
  feeds the language server, so editors surface these as error
  squiggles while you type.

### Changed

- **BREAKING: type annotations are enforced at runtime — olang is
  gradually typed.** An annotation, wherever it appears, is now a kept
  promise: function parameters check at the call boundary
  (`parameter 'x' of f expects Int, got String`), declared return
  types check on the produced value, and `let x: Int = ...` checks at
  the binding. Unannotated code stays fully dynamic with zero cost and
  zero judgment. Containers check shallowly (`List<Int>` promises "a
  List"; element types are the future static checker's concern),
  Int/Float is strict (no widening, matching struct-field
  enforcement), and generic parameters are erased, never checked. An
  async function's `-> Promise<T, E>` unwraps: the check enforces `T`
  on the value the body resolves to.
  Enforcement is identical on every tier: checks are precomputed at
  declaration, the interpreter enforces at its boundary, the bytecode
  VM enforces at function entry and return, and the JIT statically
  discharges return annotations (compiling only when the inferred
  return kind provably satisfies the declaration — otherwise the
  function stays on the enforcing bytecode path). Every annotated
  example in the repo already passed; code with dishonest annotations
  now fails with a clear, located error.

## [0.47.0] - 2026-08-11

### Added

- **Runtime errors point at source.** Every statement now carries its
  source position, and when a runtime error surfaces the file runner
  renders the offending line with a caret (via miette) plus the
  interpreter call stack — `Undefined variable: x` now comes with
  `[file.ol:3:5]`, the source line, and `mid → deep`. Positions are a
  side channel: error messages themselves are unchanged, so tier
  behavior and error-matching programs are unaffected. Statement
  equality is span-insensitive (position is metadata, not identity),
  which keeps `olang fmt`'s AST-verification gate sound.

- **Did-you-mean in file mode.** An undefined name ranks every visible
  binding by edit distance and offers the closest matches as a help
  line under the located error — previously REPL-only, now where it
  matters most.

### Fixed

- **Tier error text matches the interpreter word-for-word.** The
  bytecode tier's private wordings are gone: pattern-match, arity
  (naming the first missing parameter), binary/unary type errors —
  which now also carry context on both tiers ("cannot apply '+' to Int
  and Bool"), with immediate-flipped operands reported in source
  order.

### Changed

- **Parse errors speak plain English.** The raw pest rendering
  ("Pest parsing error:", `mul_op`, `base_pattern`) is gone: expected
  lists collapse to human phrases, every syntax error carries
  line/column and a caret snippet, parse errors inside imported modules
  render the same way (previously a raw Rust Debug dump), and UTF-8
  BOMs are stripped instead of failing at 1:1 on an invisible
  character.

## [0.46.0] - 2026-08-10

### Removed

- **Internal cleanup: dead code and unused dependencies (no user-visible
  behavior change).** Removed 26 bytecode `Instruction` variants the
  compiler never emitted (list/string/tuple/pipeline/thunk/exception/
  memory/profiling/debug scaffolding) along with their executor and JIT
  arms; the inert persistent module-cache layer, which never read or
  wrote anything (the in-memory cache with content-hash invalidation is
  unchanged); and unused dependencies (`memmap2` and the
  `arbitrary`/`fake`/`quickcheck`/`lazy_static` dev-dependencies).

### Changed

- **Struct field type annotations are now enforced at construction
  (breaking).** Constructing a declared struct with a field value whose
  runtime type does not match the field's declared annotation is now a type
  error (e.g. `field 'x' of Point expects Int, got String`), where it was
  previously accepted — annotations were advisory. Enforcement covers the
  annotations the runtime can reliably check: `Int`, `Float`, `Bool`,
  `String`, and declared struct/enum type names. The match is exact — an
  `Int` value does not satisfy a `Float` field (no widening at
  construction). Fields whose annotation cannot be reliably checked (a
  generic type parameter, a list/map, a function type) stay dynamic, as do
  all fields of an anonymous `{ ... }` object. The behavior is identical
  across all three tiers (interpreter, bytecode, and JIT) and pinned by
  differential regression tests.

- **Mixing a number and a string under `+` is now a type error, not a
  silent coercion (breaking).** `"count: " + 5` and `1 + "x"` previously
  stringified the number and concatenated; they now raise a type error
  (`cannot add String and Int; use to_string(...) to convert`),
  Python-3 style. Convert the number explicitly with `to_string(...)`
  (or `show(...)`) first. String+string concatenation and
  number+number arithmetic are unchanged. The behavior is identical
  across all three tiers (interpreter, bytecode, and JIT) and pinned by
  differential regression tests.

## [0.45.0] - 2026-08-10

### Changed

- **`random.seed(n)` reproduces within a version, not across this
  upgrade.** rand 0.8 → 0.10 (with rand_distr 0.6): seeded streams are
  not part of rand's cross-major stability contract, so a program that
  recorded exact outputs under `random.seed(n)` on 0.44 may see
  different draws now. Measured specifics of this particular upgrade:
  the raw draw stream is unchanged in practice (`random.random`,
  `uniform`, `gauss`, `randint`, and the `randstr` family produced
  byte-identical seeded sequences before and after), but
  `random.shuffle`, `random.choice`, `random.choices`, and
  `random.sample` order differently — rand 0.9 changed uniform integer
  index sampling. Treat any exact seeded sequence as reproducible only
  within a single olang version; assert properties, not pinned streams
  (the repo's own tests and examples/statlab already do). Seeding,
  determinism within a run, and `stats.norm.sample` riding the same
  stream are all unchanged.
- **A maintained HTTP stack under the `http` client.** reqwest
  0.11 → 0.13 moves `http.get`/`post`/`put`/`delete`/`request` onto
  hyper 1 and a current TLS stack; no olang-visible API change
  (verified end to end against a live server and over https).
  `http.serve` is hand-rolled TCP and is untouched. The direct `hyper`
  and `tokio` dependencies existed only for this stack and were unused
  in the source — both deleted; reqwest brings its own runtime.
- **Entropy plumbing current, and bcrypt catches up.** getrandom
  0.2 → 0.4 for the RNG stack: the wasm playground's custom entropy
  backend migrates from the deleted `register_custom_getrandom!` macro
  to 0.4's `getrandom_backend="custom"` cfg (set for wasm32 builds in
  `.cargo/config.toml`) with the hook in src/playground.rs; the
  `host_random_bytes` page import is unchanged. That unblocks bcrypt
  0.15 → 0.19 (skipped in the M2 batch precisely because it dragged
  getrandom 0.4 into wasm), so `crypto.hash_password` gets four majors
  of maintenance; existing hashes still verify. The rand_core 0.6 era
  crypto crates (rsa, aes-gcm, argon2) keep getrandom 0.2's registered
  backend alongside — both paths verified in the playground harness. Cranelift 0.121 → 0.134 puts
  13 releases of instruction-selection and aarch64 codegen work under
  every JIT'd function with zero olang-side semantic change: the N-body
  benchmark drops 48 ms → 26 ms (best of 3, same machine, byte-identical
  momentum-conservation output); fib(30) holds at 4 ms. The full JIT
  parity suite and the differential tests pin the tier's bit-identical
  contract across the upgrade.
- **A years-newer SQLite under the `db` module.** rusqlite 0.31 → 0.40
  brings its bundled SQLite engine forward several years of upstream
  releases — query-planner, correctness, and performance work land under
  every `db.*` call with no API change on the olang side.
- **Prettier crash reports.** miette 5 → 7 brings its reworked
  graphical reporter to the CLI's panic hook — internal errors render
  with cleaner layout and labels instead of a raw Rust backtrace.
- **A better REPL.** rustyline 13 → 18 carries five majors of
  line-editing fixes — more robust history handling, completion, and
  terminal behavior in `olang repl`.
- **Faster regex and CSV paths.** regex 1.10 → 1.13 and csv 1.3 → 1.4
  pick up upstream performance work behind the `re` and `csv` modules;
  pest, serde, tokio, rayon, clap, base64, and thiserror move to their
  current releases in the same batch. No olang-visible API changes.

## [0.44.0] - 2026-08-10

### Added

- **`otc pkg update`, and `install` finally honors the lock.** A
  lockfile that still covers `olang.toml` is now replayed exactly:
  pinned revs fetched (offline once cached), lock left byte-identical —
  so repeated installs and every `olang` run (which installs
  implicitly) stopped re-resolving branch deps and rewriting the lock
  on each invocation. The lock records which tag/branch a git pin came
  from, so repointing a dependency in the manifest still re-resolves on
  a plain `install`; moving a branch dep to its new upstream head is
  the explicit `otc pkg update`. Fixed along the way: the bare git
  mirrors in `~/.olang/cache` were cloned without a fetch refspec, so
  the "refresh refs" fetch had never actually updated a branch ref.

- **`otc new --lib`.** Scaffolds a library package the resolver can
  actually consume: `index.ol` at the package root (where `use <name>`
  looks) with a `share`d function and a passing test block, plus a
  README showing how a depending package adds and imports it.

- **The tracker's frontend catches up with its backend.** Still plain
  HTML + JS with zero dependencies, now surfacing the whole API:
  server-driven search/filter/sort/paging with the view state mirrored
  into the URL hash (refresh and share keep the filters, shown as
  clearable chips), a comments drawer with avatars, relative times, and
  ⌘-Enter submit, a stats dashboard whose CSS bars use the same status
  colors as the grid pills (color follows the entity) plus the ods
  point quantiles, a live activity feed, CSV export and one-click
  backups, dark mode (auto-detected, toggleable, remembered), keyboard
  shortcuts (`/`, `n`, `Esc`), a full inline create row, two-step
  delete, field-by-field 422 toasts, and a 401 that prompts once for
  the bearer token. The list endpoint now carries each issue's comment
  count (one subquery — no n+1), locked into the contract test.

- **`examples/app` — the tracker grows a real backend.** The
  full-stack issue tracker now runs on a persistent, schema-migrated
  SQLite store (a `schema_version` table; migrations append, run once,
  in transactions) with request validation (422s naming each field
  problem), filtered/paginated listing (`?status= assignee= q= sort=
  order= limit= offset=` returning `{items, total, ...}`), comments
  with transactional cascade delete, an audit trail of every mutation
  (`/api/activity`), stats blending SQL rollups with ods point
  quantiles, CSV export, JSON backups, and optional bearer-token auth
  for writes (`TRACKER_TOKEN`). The router became a middleware layer:
  per-request ids and timing logs, one JSON error envelope everywhere,
  method-aware 405s with `Allow`, and HEAD riding GET so probes see
  200. The whole contract is locked by `tests/tracker_app_test.rs`,
  which boots the actual app on an ephemeral port and drives it over
  the wire — validation shapes, filter fallbacks, 404/405/400/401
  semantics, the audit sequence, cascade deletes, and auth gating.

- **`os.read_line()` — stdin, at last.** One line from stdin as
  `Ok(line)` (newline stripped), `Err("eof")` when the stream ends: the
  missing primitive for prompts, REPLs, and shells, and for reading
  piped input line by line.

- **`examples/oshell/` — a Unix-like shell written in olang.** An
  interactive `os.read_line` loop where every operation is the stdlib:
  pipelines thread stdout→stdin through `os.exec`, redirection
  (`< > >>`) and globbing ride on `fs`, `grep` is `re`, and 25 builtins
  (`cd ls cat head tail grep wc mkdir rm cp mv touch env export alias
  history which type ...`) shell out to nothing. Quoting, `$VAR`/`~`/`$?`
  expansion, `;`/`&&`/`||` with real short-circuit semantics, exit-code
  propagation (127 for not-found, `exit N` as the process code), and
  history persisted across sessions. The long-running systems-work
  proof: 1,000 mixed commands — 250 child processes among them — soak
  through one session in ~1.4 s with state consistent throughout.

- **`olang lsp` — the language server, in the same binary.** Speaks LSP
  over stdio: diagnostics as you type (parse errors with the parser's
  own line/column; analyzer warnings such as unused variables at their
  declaration sites), completions (keywords, global builtins, the 19
  stdlib modules, and fn/type/let names from the open file), and
  whole-document formatting through the olang fmt engine. Stateless by
  design — every edit re-parses whole files. Tested at the protocol
  level: tests/lsp_test.rs drives the real binary over stdio through
  the complete loop, including clean shutdown.
- **Declaration spans in the AST; hover and go-to-definition in the
  server.** `fn`, `let`, and `type` declarations now carry the 1-based
  source position of the name they bind (serde-defaulted, so cached
  ASTs stay readable; synthetic/desugared declarations carry None).
  On top of them the language server gains hover (the declaration's
  rendered signature) and go-to-definition, and unused-variable
  warnings move from text-search positions to exact declaration spans.
  Protocol test extended: hover content and definition target are
  asserted to the character.
- **VS Code extension (editors/vscode/).** TextMate grammar (par for,
  pipelines, template strings, module names), bracket/indent config,
  and a thin client launching `olang lsp`; `olang.serverPath` setting
  for custom binary locations. The book gains an Editors chapter with
  Neovim wiring included.

- **The tracker runs on olang end to end — page shim, dom.fetch, and
  the JS frontend deleted.** examples/app now serves ONE frontend:
  app.ol, running in the browser as WebAssembly through the page shim
  (static/olang-dom.js — real DOM host imports, session boot, event
  and fetch-response dispatch). dom.fetch(method, path, body, cb)
  bridges browser HTTP with responses delivered to 1-argument olang
  closures via the new olang_dispatch_event_with; http.serve gains
  body_file responses (raw bytes from disk) so the olang backend can
  serve its own wasm binary. The frontend is deliberately stateless —
  olang closures capture by value (spawn semantics), so state flows
  down through arguments and the DOM itself, which the first draft
  learned the hard way: a handler's write to module-level mut state
  vanished into its own environment (parsed 14, rendered 0). Verified
  in a real browser: render from the API, add, status-advance (PATCH),
  and delete all round-trip through olang closures; the harness covers
  the fetch-payload path headlessly.

- **The `dom` module — olang as a frontend language.** A new stdlib
  module (query, get/set_text, set_html, value/set_value, on) whose
  operations cross the wasm boundary as host imports the page
  implements; elements are opaque handles. The playground boundary
  gains a persistent session: olang_session_start runs a program and
  keeps its interpreter alive, dom.on registers olang functions in a
  handler registry, and olang_dispatch_event re-enters the live
  interpreter per event — click handlers are ordinary olang closures.
  Native builds error clearly ("only available in the browser").
  Proven end-to-end by playground/dom_harness.mjs: a fake DOM over the
  host imports, an olang counter program, two dispatched clicks, and
  asserted mutations (run: bun playground/dom_harness.mjs
  target/wasm32-unknown-unknown/release/olang_playground.wasm).

### Fixed

- **`otc unused` stopped flagging live code.** Usage now counts
  wildcard and bare imports (`use m { * }` / `use m` marks every shared
  function of `m` used) and namespace references (`m.f(...)`) — not
  just `use m { f }`. Over-counting is the deliberate direction: a
  linter that cries wolf gets ignored.

- `print` now flushes stdout, so a partial line — a shell prompt, a
  progress indicator — appears immediately instead of waiting for the
  next newline.

## [0.43.0] - 2026-08-09

### Added

- **`par for` — parallel iteration as a language construct; P1
  closes.** `par for x in xs { body }` (tuple bindings included) fans
  iterations across OS worker threads with an implicit barrier — one
  interpreter, tier and JIT included, per worker. Spawn-style snapshot
  semantics; first-sequential-error reporting; break/return cannot
  cross the parallel boundary; `par` is not a reserved word (it only
  means something before `for`, pinned by test). Iterates lists,
  ranges, and strings. The bytecode compiler refuses it (fail-closed,
  interpreter-owned); `olang fmt` handles it. With this the
  performance campaign's three levers are all resolved.

## [0.42.0] - 2026-08-09

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

[Unreleased]: https://github.com/ooyeku/olang/compare/v0.65.0...HEAD
[0.65.0]: https://github.com/ooyeku/olang/compare/v0.64.0...v0.65.0
[0.64.0]: https://github.com/ooyeku/olang/compare/v0.63.0...v0.64.0
[0.63.0]: https://github.com/ooyeku/olang/compare/v0.62.0...v0.63.0
[0.62.0]: https://github.com/ooyeku/olang/compare/v0.61.0...v0.62.0
[0.61.0]: https://github.com/ooyeku/olang/compare/v0.60.0...v0.61.0
[0.60.0]: https://github.com/ooyeku/olang/compare/v0.59.0...v0.60.0
[0.59.0]: https://github.com/ooyeku/olang/compare/v0.58.0...v0.59.0
[0.23.0]: https://github.com/ooyeku/olang/releases/tag/v0.23.0

# Roadmap

Part of [the olang book](README.md) ·
[Stability and compatibility](stability.md) ·
[Architecture and internals](internals.md)

This chapter is the plan of record for reaching olang 1.0. It records the
design decisions that define the 1.0 language, the campaigns that implement
them, the acceptance criteria for the release, and what is frozen until it
ships. It replaces the development log that previously occupied this file;
completed history is recorded in [CHANGELOG.md](../CHANGELOG.md) and the
repository history.

Item statuses progress from **planned** to **in progress** to **landed**,
and landed items name the release in which they shipped.

## Table of contents

- [Decisions of record](#decisions-of-record)
- [The definition of 1.0](#the-definition-of-10)
- [Campaign 1 — the semantics release](#campaign-1--the-semantics-release)
- [Campaign 2 — data-pipeline completion](#campaign-2--data-pipeline-completion)
- [Campaign 3 — capabilities on every tier](#campaign-3--capabilities-on-every-tier)
- [Campaign 4 — 1.0 readiness](#campaign-4--10-readiness)
- [Frozen until 1.0](#frozen-until-10)
- [Post-1.0 candidates](#post-10-candidates)
- [Process](#process)

## Decisions of record

A design review in August 2026 identified the language's outstanding
semantic debt and structural risks. The following decisions were made on
2026-08-15 and are final; the campaigns below implement them. Each is a
choice among alternatives that were considered and rejected.

| # | Decision | Resolution |
|---|---|---|
| D1 | Block scoping | Bare blocks scope their `let` bindings. Nothing leaks out of a block; shadowing an outer binding inside a block is permitted. The current leak-and-warn behavior is removed. |
| D2 | Binding creation | `let` is required for every first binding. Assignment to an undeclared name becomes an error rather than creating a binding. |
| D3 | Mutability | `let mut` is enforced. Reassigning a binding not declared `mut` becomes an error; `mut` changes from a documentation marker to a guarantee. |
| D4 | Mutable state | A thread-confined mutable cell is added: `cell(v)` with `get`, `set`, and `update`. A cell cannot cross `spawn`, `par_map`/`par_filter`/`par for`, or a channel; the attempt is an error at the crossing point. Replay determinism and the no-shared-mutable-state property are preserved. Assignment to a captured variable, currently a dead write that warns, becomes an error whose message points to `cell`. |
| D5 | Concurrency model | `async`/`await` and the `Promise` API are removed. The concurrency model is threads (`spawn`), channels (`chan`), and data parallelism (`par_map`, `par_filter`, `par for`). The deadline-based Promise scheduler, which resembled asynchronous I/O without being it, is deleted rather than repositioned. |
| D6 | Capabilities | Capability enforcement moves to the shared builtin-dispatch boundary with per-frame attribution, so a capability-restricted program runs at full speed on every tier. Capabilities graduate from a development-time audit mechanism to an enforced boundary at 1.0. |
| D7 | Standard-library conventions | One full conventions audit lands in the same breaking release: operations that cannot fail stop returning `Result`; keywords that can be contextual (such as `share`) stop being reserved as identifiers; argument-shape irregularities (such as `fs.join` taking a list) are corrected. 1.0's API is the cleaned one, and there is exactly one migration. |
| D8 | Error model | `Result` with `?` is the primary mechanism for expected fallibility. A runtime error is a bug and stops the program; recovery is *structural*, at the boundaries that already isolate a failing unit (a spawned task, an `http.serve` handler). **Amended in 0.65:** D8 originally kept `try`/`catch` for "recovering from runtime errors at coarse boundaries". It never did that — it destructured a `Result`, and a real runtime error inside a `try` block still aborted — and the boundaries it named already recover without it. `try`/`catch` was removed; the promised checker warning became a *discarded-Result* warning, which is where the actual hole was. |
| D9 | 1.0 scope | 1.0 is a data-scripting release: the semantics release, the completed data-pipeline campaign, cross-tier capabilities, and a complete book. The browser stack, JIT breadth, the adaptive-engine campaign, and registry growth are frozen (maintenance only) until 1.0 ships. |

Two further points follow from these decisions:

- The semantics release is the **second and final deliberate breaking
  change** before 1.0 (the first was runtime type enforcement in 0.48.0).
  Everything it changes is currently covered by the advisory warnings
  introduced in 0.50, so the checker already identifies every affected site
  in existing code.
- After 1.0, olang follows semantic versioning proper: no documented syntax
  or behavior changes within a major version.

## The definition of 1.0

1.0 ships when all of the following are true:

1. The semantics release (Campaign 1) has landed and the entire repository
   corpus — examples, embedded packages, the book, the test suite — runs
   under the new semantics with every gate green.
2. The data-pipeline campaign (Campaign 2) is complete: streaming input,
   the full verb set, and a published, reproducible benchmark against
   pandas and Polars alongside a flagship example.
3. Capability enforcement runs on every execution tier with no performance
   penalty for sandboxed programs (Campaign 3).
4. The book documents the final language completely, and
   [Stability and compatibility](stability.md) is rewritten as the 1.0
   compatibility contract.

## Campaign 1 — the semantics release

Implements D1–D5, D7, and D8. This campaign touches every layer — grammar,
interpreter, OVM, JIT, checker, formatter, language server — because the
tiers must agree on the new semantics exactly as they agree on the current
ones.

**S1–S3 shipped in 0.61.0.** Scope and mutability are settled: every block
scopes its bindings, assignment is not a declaration, and `let mut` is a
guarantee rather than a comment. All three are enforced by one validation
pass that runs before execution, which is what makes the three tiers agree
by construction rather than by testing. The one tier divergence the work
uncovered — the bytecode compiler's flat local-variable map, which
disagreed with the interpreter on nested shadowing — is fixed and pinned
by differential tests.

**S4 shipped in 0.62.0.** The mutability model is complete: plain `let`
is immutable, `let mut` is a reassignable binding, and `cell` is a
mutable *location*. Two decisions departed from D4's wording and are
recorded here as amendments. First, the operations are namespaced
(`cell.get`/`cell.set`/`cell.update`) rather than global `get`/`set`/
`update`, because three of the most common identifiers in any program
are too much to spend; `cell(v)` remains the constructor because the
module is callable. Second, confinement is enforced at *access* rather
than at the thread crossing: `spawn` and `par_map` snapshot the whole
environment rather than an enumerated capture list, so there is no list
of what crossed to inspect, and a crossing-time check would have had to
refuse any spawn with a cell merely in scope. `chan.send` is the one
crossing that holds the value, and is checked there.

S4's captured-write rule initially reached function and closure bodies
only, leaving `par for` uncovered — its body also runs against a worker
snapshot, so a write to an enclosing binding was silently dead, and
worse, conditionally so: workers are clamped to the item count, so a
one-item list took the sequential path and the write landed. The same
loop answered `1` for `[1]` and `0` for `[1, 2]`. `par for` now opens
the same boundary, with a message that points at `par_map` and `chan`
rather than at a cell — a cell is confined to its creating thread and
cannot cross into a worker.

**S5 shipped in 0.63.0.** `async`, `await`, and the `Promise` API are
gone; `spawn` returns a task handle that `task.join` collects. The
deadline-based promise scheduler is deleted rather than repositioned, as
D5 required — it simulated latency without providing concurrency, while
`spawn` beside it was a real thread and `await` on a spawned task was
really a join. One amendment to D5's wording: `Promise.race` has no
direct replacement. `task.join_timeout(t, ms)` bounds the *wait*, and
the task keeps running, because an OS thread cannot be cancelled from
outside without leaving what it touched in an unknown state. A general
`race` would have implied the losers stopped. The remaining lane (S8) is
unchanged.

**S6 shipped in 0.64.0.** The conventions audit walked all 22 stdlib
modules — 391 functions, 470 error sites — against one rule, and the
rule gained a tier D7 did not name. D7 asked that infallible operations
stop returning `Result`; the audit found the deeper problem was that
*misuse* also returned `Result`, which is what made `Result` unreliable:
`unwrap_or(f(x), default)` could not tell a missing file from a typo'd
call. Misuse now raises, which is the change that makes the other two
tiers mean something. Twenty-eight functions dropped `Result`, seven
declaration keywords became ordinary identifiers, and `fs.join` went
variadic while still accepting a list.

**S7 shipped in 0.65.0.** The lane began by checking D8's premise and
found it false twice over: `try`/`catch` did not recover runtime errors
(a type error inside a `try` block still aborted — it only destructured
`Result`s), and the "coarse boundaries" D8 named already recover without
it, structurally, because `task.join` turns a failed task into `Err(e)`
and `http.serve` logs a failing handler and returns 500. With zero uses
across 97 corpus files, `try`/`catch` was removed rather than
documented.

The promised warning changed target accordingly. "Catch used for control
flow" describes nothing once `catch` is gone; the real hole was that a
discarded `Result` was invisible to both the runtime and the checker — a
failed `fs.write_file` in statement position read exactly like a
successful one. That is now an advisory warning, and turning it on found
six live instances in the corpus, including `http.serve`'s bind failure
being dropped in both flagship apps, which made a taken port look like a
clean exit.

| Lane | Work | Status |
|---|---|---|
| S1 — lexical scoping | Blocks introduce a scope: `let` bindings are dropped at the closing brace, shadowing is permitted, and the environment model in the interpreter and both compiled tiers is updated together. The 0.50 scope-leak warning becomes an error. | **shipped 0.61.0** |
| S2 — required `let` | Assignment to an undeclared name is an error naming the variable and suggesting `let`. The 0.50 advisory warning becomes the error. | **shipped 0.61.0** |
| S3 — enforced `mut` | Reassignment of a non-`mut` binding is an error. The checker and language server list every site to migrate; the corpus is migrated in the same change. | **shipped 0.61.0** |
| S4 — the cell | `cell(v)`, `cell.get(c)`, `cell.set(c, v)`, `cell.update(c, f)`. Cells are values with identity confined to their creating thread: reading or writing one from another thread is an error, and `chan.send` refuses to send one. Dead captured-variable writes became errors pointing to `cell`. Timeline recording is unaffected because cell mutation is deterministic within a thread. | **shipped 0.62.0** |
| S5 — remove async | `async`, `await`, and the `Promise` API are removed from the interpreter and tiers. `spawn` returns a task handle collected by `task.join` / `task.join_timeout`. Programs using `Promise.delay/all/race` migrate to `time.sleep`, `map(task.join)`, and `chan`; the book's concurrency chapter is rewritten around the single model. | **shipped 0.63.0** |
| S6 — stdlib conventions audit | Every builtin and module function audited once against one rule: infallible operations return their value, handleable failure returns `Result`, and misuse raises. Seven declaration keywords freed as identifiers; `fs.join` variadic; the definitive before/after table recorded in the CHANGELOG. | **shipped 0.64.0** |
| S7 — error-model boundary | `try`/`catch` removed (it was `Result` sugar, not recovery, with zero corpus uses). A discarded `Result` in statement position draws an advisory warning — the real hole, since a failed write read exactly like a successful one. The book's error chapter is rewritten around Result-primary with recovery documented as structural. | **shipped 0.65.0** |
| S8 — migration | The release ships with a migration guide. `olang check` reports every site the release breaks (its 0.50 warnings are the census); the repository corpus is migrated in the release itself as the proof of the guide. | **shipped across 0.61–0.65** |

**S8 was never a separate release.** It planned one migration guide at the
end of the campaign, on the assumption the breaks would land together. They
did not: S1–S7 shipped as five releases, each carrying its own migration
section and migrating the corpus in the same commit as the break. By the
time S8 came up its deliverable existed five times over, for an audience
that does not exist — there is no olang outside this repository. It is
recorded as shipped rather than left planned, because a lane that can never
have work is a lie about the plan.

Acceptance: all gates green under the new semantics (workspace tests, doc
examples, the example harness, tier-agreement suites); the CHANGELOG
documents every break with its rationale and migration; no advisory warning
remains for behavior that no longer exists.

**Campaign 1 is complete.**

## Campaign 2 — data-pipeline completion

Completes the campaign begun in 0.60.0 (the parallel join and `group_by`
aggregation landed there). This work is additive and independent of
Campaign 1's semantics; example programs written for it are migrated with
the corpus if Campaign 1 lands first.

| Lane | Work | Status |
|---|---|---|
| DP1c — parallel group keys | Parallelize the group-identification pass of `group_by`. | **closed — measured, not needed** (see below) |
| DP2 — IO breadth | Read CSV from files and as a bounded-memory stream; JSON-lines input; a columnar interchange format; `to_csv` and file output. The end-to-end input-to-output story. | **shipped** |
| DP2b — ergonomics | Subscript syntax for Frames and Series, and the orientation verbs (`describe`, `schema`). Reaching a column was 13.6% of every `ods` call in the corpus and the most verbose thing in it. | **shipped** |
| DP3 — verb completeness | Respec'd against what a real pipeline reached for. **Tier 1 — shipped**: `rename`, `drop`, `distinct`, `tail`, frame-level `drop_null`. Frame-level `fill_null` was dropped from the tier on inspection: `with_column(f, c, fill_null(f[c], v))` already expresses it via 0.66's subscript, and a second spelling of an existing operation is surface without capability. **Tier 2 — shipped**: `value_counts`, `unique`/`n_unique`, `median`, `cast`, `sample`. The distinct verbs share `group_by`'s key pass so they cannot disagree with it; `median` is defined as `quantile(0.5)`; `cast` turns what a type cannot hold into null rather than an error or a wrong number; `sample` is seeded by the existing `random.seed` stream. **Tier 3** (the original list): **3a — shipped**: the rest of the join kinds (`join_full`, `join_semi`, `join_anti`). **3b — shipped**: reshape (`pivot` long→wide, built on `group_by` so the two cannot disagree; `unpivot` wide→long). **3c — shipped**: window functions (`shift`, `cum_max`/`cum_min`, `rank` with five tie methods, `rolling`). A separate `diff` was judged unnecessary: `s - shift(s, 1)` already expresses it. | done |
| DP4 — flagship and benchmark | A realistic end-to-end ETL example wired to `plot`, and a reproducible benchmark against pandas and Polars with methodology and hardware documented in [The data stack](ods.md). | **flagship shipped**; the benchmark remains |

**DP2 shipped.** `ods.read_csv_file`,
`ods.to_csv`, and `ods.write_csv` close the loop the chapter described but
the stack could not finish, and `ods.open_csv` / `ods.next_chunk` process
a file larger than memory: measured over a 1M-row CSV, the whole-file read
peaks at 196.5MB while the streamed pass peaks at 12.2MB — 0.2MB more than
the same pass over a file five times smaller. Every file-touching call
demands `fs`; every other `ods` function stays pure, which is what keeps
the module from becoming a filesystem capability by the back door.

Two findings shaped the streaming design, and both came from the language
rather than from the data stack. First, the reader cannot take a callback:
`(chunk) => { total = total + chunk }` is refused by 0.62's capture rule,
because the accumulator is a captured write. That ruled out the API every
peer language uses and left the loop — `next_chunk` until it returns an
empty Frame — which needs no closure and so needs no exception. Second, a
fold taking an olang lambda could not live in `ods` at all as the module
boundary stands: `OvmModule::dispatch` receives `(func, args)` and no
interpreter, which is precisely what lets both tiers dispatch identically.

JSON lines followed, and the lane's own design was what made it small:
`read_jsonl`, `read_jsonl_file`, `to_jsonl`, `write_jsonl`, and
`open_jsonl`, the last of which is driven by *the same* `next_chunk` /
`rows_read` / `at_end` verbs as the CSV reader. Both formats share one
`Reader` type, so a streaming loop names its format once, at the `open_`,
and a function taking a reader takes either. A 300,000-row JSON lines
file streams at 30.6MB against 471MB read whole.

The reader is therefore a `NativeObject` holding its own file position —
the mechanism `cell` and `task` already use — and, holding mutable state,
it is confined to the thread that opened it. That confinement was
generalized onto the `NativeObject` trait as `confined_to()` rather than
written for the reader: `chan.send` now refuses any confined value without
naming the types it knows, so the reader was refused at the channel
boundary before a line was written for it, and the next such handle will
be too.

**DP2b shipped.** `f["amount"]`, `f[mask]`, and `s[i]` replace the
gesture that measurement showed dominates data code: `ods.column` and
`ods.get` were 13.6% of the 937 `ods` calls in this repository. The
mechanism is one trait method — `NativeObject::index`, defaulting to
`None` — reached from the index path of both tiers, so every native value
can now define a subscript and the ones that don't keep the language's
existing error.

The subscript takes two keys and refuses a third. A String selects a
column, a Bool Series selects rows, and a row *position* is refused
outright, naming `ods.head` and `ods.take`. That refusal is what keeps
each subscript to one reading — pandas spells all four of column
selection, row filtering, positional slicing, and an error as `df[x]`,
which is why `.loc` and `.iloc` had to be invented on top of it. This is
the general lesson for the rest of DP3: copy the gestures, not the
overloading.

`ods.describe` and `ods.schema` answer "what is in this table" in one
call, and both return Frames rather than maps, so the table renderer
carries them and they compose with every other verb. Building `describe`
immediately found a defect in that renderer: full `f64` precision on a
computed column is wide enough to push two other columns out of the
width budget. Floats longer than twelve characters now print to six
significant digits, reported in the footer like every other cap.

The columnar format closed the lane, built as decided: a native,
self-describing olang format rather than Arrow or Parquet, because both
would be a dependency and both are opaque. `ods.write_frame`,
`ods.read_frame`, and `ods.frame_info` carry it. The header is UTF-8
text, one line per column, so `head` answers what is in a file, and
`frame_info` returns that same header as a Frame without reading the
data. Over 500,000 rows and six columns a CSV load takes 138ms against
50ms columnar, 26ms for a single column, in a file that is also smaller
— 15MB against 17MB. Arrow interop remains a post-1.0 candidate.

The size figure took a second pass, and the first one is the more useful
record: the obvious layout — every row's string with its own offset —
produced a file *larger than the CSV*, 24MB against 17MB, because an
eight-byte offset costs more than the four characters it points at.
Repetition is the normal case in a table, so a repeating String column is
now written once as a dictionary plus one code per row, chosen by
computing both sizes and taking the smaller. There is no threshold to
tune. The general shape of that lesson is the one DP3 should carry: a
columnar format's win is an *encoding* win, and the encoding slot in the
header (`enc=`) is where later ones go.

Acceptance: a streaming job processes input larger than memory; the
benchmark is reproducible from the repository; the flagship example ships in
`examples/` and runs in the harness.

**DP1c is closed without work, on evidence.** The lane assumed the
group-identification pass dominates `group_by`'s runtime. Measured over
2,000,000 rows:

| grouping | time |
|---|---|
| 4 groups | 10ms |
| 64 groups | 9ms |
| 2,000,000 groups (every row distinct) | 196ms |
| bare `sum`, no grouping | 0ms |

The key pass costs nothing at the cardinality `group_by` is actually for.
Grouping 2M rows into 4 or 64 is ~10ms either way — memory-bandwidth
territory, where threads do not help. The cost only appears when nearly
every row is its own group, which is a `sort_by` in a `group_by` costume.

That case is also where parallelising is hardest and least rewarding.
`group_ids_single` assigns ids in *first-seen order* and the output row
order depends on it, so a parallel version needs per-chunk local maps plus
a merge that orders distinct keys by their global minimum first-seen row
— and with 2M distinct keys the merge is the work. The lane would have
added a correctness-sensitive ordering dance to speed up the one shape
nobody should use `group_by` for.

Recorded rather than deleted, so the question is not reopened from the
same wrong premise. If a profile ever shows the key pass dominating a real
workload, that profile — not this assumption — is the thing to act on.

**DP3 is respec'd.** The original list (window functions, reshape,
further joins) was written before anything had tried to use the stack in
anger. DP4's flagship then found four verbs missing that were not on that
list at all — `concat`, mask combination, the join-key default, and
scalar `eq`/`ne` — and all four turned out to be things a first pipeline
cannot do without. They shipped with the flagship.

So DP3 is now ordered by evidence rather than by category. Tier 1 is what
the next pipeline will hit immediately: `rename` and `drop` (a join that
collides names currently leaves you no way to fix it), `distinct`, `tail`,
and frame-level null handling. Tier 2 is what a reader reaches for once
`describe` has shown them the shape. Tier 3 is the original list, held
until a real workload asks — the same discipline that turned out to be
right about DP1c.

**DP4's flagship shipped, and it earned its place by breaking things.**
[`examples/meterflow/`](../examples/meterflow/) streams JSON-lines
telemetry in bounded memory, gates it on quality, aggregates across
chunks, joins two CSV dimension tables, prices the result, caches it in
the native columnar format, and charts it. Over 400,000 readings the
streamed run and a whole-file computation agree exactly — 390,033 rows
and 781,262.26 kWh either way, across 80 chunk boundaries.

Writing it found four gaps, all closed in the same change: `all_of` /
`any_of` / `not` for mask combination, `concat` for stacking Frames,
`join`'s second key name defaulting to the first, and `eq` / `ne`
accepting a plain value as `==` already did. The first two were blocking.
A filter with two conditions was not expressible at all — `&&` compiles
to a short-circuiting jump, which has no elementwise reading over a
column — and without `concat` the partial results of a streaming loop
could only be recombined through a row-shaped detour, which is the thing
the columnar representation exists to avoid.

That is the argument for having built DP4 before DP3. None of the four
were on DP3's list, which names window functions, reshape, and
categorical operations; all four are things a first real pipeline cannot
do without. DP3 should now be written against what the flagship reached
for rather than ahead of it.

## Campaign 3 — capabilities on every tier

Implements D6. Enforcement used to run on the interpreter tier only, and a
capability-restricted run forwent the bytecode tier; this campaign removes
that trade.

| Lane | Work | Status |
|---|---|---|
| C1 — builtin-boundary enforcement | Capability checks move to the single dispatch point that all tiers share. Each compiled function carries its source-package provenance; OVM frames expose it; JIT code reaches builtins through helpers that pass through the same dispatch. The restriction that disables the bytecode tier under a capability manifest is then removed. | **shipped** |
| C2 — attribution hardening | Package roots are canonicalized so symlinks cannot confuse attribution; `os.exit` and remaining ungated process-affecting calls are brought under the `proc` gate; bundle-format fields are validated on read. | **shipped** |
| C3 — denial and the error model | A denial keeps stopping the program, and a `caps` module lets a program ask what it was granted (`caps.allowed("fs")`, `caps.granted()`) so it can choose a different path *before* attempting the call. Degradation becomes a branch the program takes deliberately, not an error it recovers from. | **shipped** |

**C1 shipped.** The gate was never the missing piece: `BuiltinFunctions::
call_internal` is the one choke point both tiers already pass through.
What was missing was *context*. The tier reaches builtins the VM cannot
run natively through a bridge interpreter — a separate `Interpreter` —
which carried no capability table and no notion of which function was
executing, so it presented every call as unrestricted and unattributed.
Rather than enforce partially, a manifest switched the tier off.

Compiled functions now carry the file they were declared in, the VM keeps
a stack of those files as it executes (the tier's mirror of the
interpreter's `coverage_file_stack`), and both the grant table and the
`--trace-caps` set are handed to the bridge before each dispatch. A
promoted function in an attenuated dependency is judged by that
dependency's grant, and the profiler sees its effects.

Measured on `fib(30)`: a `--deny`-restricted run went from 1497 ms to
4 ms, matching the unrestricted run exactly. An unrestricted run pays one
branch per call for the attribution stack.

**C3 was respecified after 0.65.** It read "a capability denial is a
runtime error recoverable at a `catch` boundary, consistent with D8" —
written before the lane that removed `catch` and corrected D8. Rewriting
it against the settled error model changed the answer rather than the
wording.

The tempting fix is to make a denial an `Err`, since every gated function
(`fs.read_file`, `http.get`, `db.query`) already returns `Result` and a
denial would slot straight in. That is the wrong shape. A `Result`
communicates a failure the caller might reasonably handle — the file was
missing, the host was unreachable — and the caller's usual reply is a
default: `unwrap_or(fs.read_file(p), "")`. A denial is not that. It is a
statement about what this program is *permitted* to do, which no retry or
fallback value can change, and burying it in the `Err` channel means the
one line written to tolerate a missing file also silently tolerates the
sandbox. That is exactly the hole 0.64 closed by making misuse raise
rather than return `Err`, and calling a capability the program was told it
may not call is misuse of the same kind.

So a denial stops the program, as it does today. What is missing is not
recovery but *introspection*: there is no way for a program to ask what it
was granted, so "degrade deliberately" is impossible to write. A `caps`
module supplies it, and degradation becomes an ordinary branch:

```olang no-run
let report = if caps.allowed("net") => fetch_live()
             else => read_cached()
```

This also folds in the separately-tracked OM2 lane ("graceful capability
failure"), which described the same problem from the other side.

Acceptance: a capability-restricted run matches the unrestricted run's tier
behavior and speed; the capability test suite passes on every tier; the
book's openness and packages chapters describe capabilities as an enforced
boundary, with the attribution model and its limits stated precisely.

**Campaign 3 is complete.** C2 turned out to be two real holes and one
already-closed item. `os.exit` escaped the `proc` gate entirely — a
dependency denied `proc` could not spawn a process but could still
terminate the host, a larger power than the one it was refused. And a
bundle's transparency record was never validated: `read_bundle` checked
lengths, overflow and bounds, but the `format` field — which selects
*which bytes the digest covers* — was tested with `>= 3`, so a bundle
claiming format 99 verified under format-3 rules and printed
`[verified]`. Both are fixed and both have tests that forge the
condition rather than assert the fix.

The third item, canonicalizing package roots against symlinks, was
already done — but nothing proved it, so it now has a test that reaches
a dependency through a symlink and confirms the attenuation holds.
"Already correct" and "known to be correct" are different states, and
only the second survives a refactor.

C3 completes the openness story. Degradation is now a branch a program
takes deliberately (`caps.allowed`, `caps.level`, `caps.granted`) rather
than an error it recovers from, which keeps the error model intact: a
denial still stops the program, and the way to avoid one is to not make
the call. The grant a program reads is the *caller's*, so attenuated
dependency code sees its own — the same classification the gate uses,
asked as a question instead of enforced as a refusal.

## The tier boundary, priced

A struct passed to a hot function cost 80x what the same data cost in a
list — 129ms against 1ms — while `--no-ovm` measured the two as
identical. The gap was entirely the interpreter/OVM boundary: the
argument-conversion cache, which lets an unchanged value cross once
instead of once per call, covered lists and nothing else. A struct
re-converted its whole payload on every call.

The cache now covers every Arc-backed compound value, keyed on variant
and allocation together. Structs cross as cheaply as lists, and the
workaround a reviewer had been forced into — rewriting every data
structure as an untyped list, because the readable `type Node = struct
{...}` form was O(n) per descent — is no longer necessary.

Two lessons worth keeping. The first is that the fix depended on a change
that had already been made and reported as *not* fixing anything: giving
struct fields an `Arc` did not move the benchmark, but it gave structs
the stable allocation identity the cache needed. The second is that the
original diagnosis — "structs are deep-copied" — was right about the
representation and wrong about the cost, and only bisecting with
`--no-ovm` separated them.

## Tier agreement, tested

`docs/ovm.md` promises that a lower tier which cannot reproduce the
interpreter's result refuses to run rather than diverging. That promise
is now enforced by `tests/tier_agreement_test.rs`, which runs the corpus
through both engines and diffs the output.

On its first full run it found `examples/parser` answering differently on
the compiled tier, and the fault was real: the on-demand compile cache
for higher-order calls was keyed on a function body's allocation identity
alone, while the compiled artifact also bakes in the captured
environment. Two closures from one factory therefore collided, and the
second ran the first's captures — silently, on the default execution
path, in the idiom the language most advertises.

A reviewer had reported exactly this after ~8,000 lines of use. Eight
attempts to reduce it to a snippet failed, because it needs a *shared
call site*: calling a closure directly takes a path that carries captures
explicitly, so the fault only appears when a combinator invokes it. That
is why a whole-program harness found in one run what targeted testing had
missed, and the argument for keeping the corpus in the loop rather than
relying on hand-written cases.

## Campaign 4 — 1.0 readiness

**R1, first sitting.** The audit began mechanically rather than by
reading, on the theory that a person re-reading 10,000 lines finds fewer
errors than a check that runs on every commit — and that the check keeps
working afterwards.

Two guards exist now. `tests/doc_references_test.rs` extracts every
`module.function` the book names — 322 of them — and resolves each
against the real binary; it found `ovm.md` citing `fs.read`, which has
been `fs.read_file` since long before 0.60. `tests/doc_examples_test.rs`
gained `packages.md` and `tooling.md`, whose runnable examples nothing
had ever executed, and which carry the instructions a new user follows
first.

A check on removed constructs (`async`, `await`, `Promise`, `try`,
`catch`) found only correct historical mentions — "no longer has",
"ordinary identifiers" — so the semantics releases are reflected
accurately.

What remains is the part no extractor can do: reading each chapter for
claims that are true of an older olang, explanations that no longer match
the implementation's reasoning, and examples that run but teach the wrong
idiom. That is the prose pass, and it is what R1 mostly is.

**R1, second sitting.** The prose pass, run as *probing* rather than
reading: for each falsifiable claim the book makes, write the program it
implies and see whether the binary agrees. That found four errors a
reader would have hit and no extractor could have caught, because each
is a sentence about behavior rather than a name or an example.

`stability.md` — the chapter that says it is authoritative wherever any
other text disagrees — carried the worst. Its "Reserved — parses today,
semantics later" section listed two constructs and neither parses:
`A & B` is a parse error, and the second bullet says in its own text
that it is not accepted, contradicting the heading above it. The `caps`
module, shipped in C3, appeared in no list. Its enforcement section
predated both guards the 1.0 audit built. And its semver illustration
still read 0.25 → 0.26.

`pitfalls.md` warned that `for _ in ...` is a parse error. It has not
been since the papercut batch; the pitfall told readers to avoid
something that works. What is true, and was documented nowhere, is that
an identifier may not *begin* with an underscore — so `_unused`, the
convention half the languages a reader comes from use for exactly this,
fails with `expected the end of the file`, naming neither the underscore
nor the line's real problem.

`language.md` was behind on two capabilities from the same batch: `for`
iterates tuples (`par for` still does not, and its own paragraph was
already correct), and `()` is a literal — the way a Unit is written down
rather than arrived at, which is what makes a missing map key comparable
to anything.

The lesson for the rest of the pass: the errors cluster in sentences
that were true when written. Prefer the claims a program can falsify,
and write that program.

**R1, third sitting.** Carrying that lesson further, and finding the one
class of error the existing guards structurally could not see.

Two tool claims were wrong. `tooling.md` documented `olang doc --md`
twice; the flag is `--markdown`. And `olang bench`'s four real flags
were invisible to `olang bench --help`, because the runner parses them
itself out of forwarded arguments and clap knows nothing about them — so
the book was the only place they existed. That is a gap in the tool
rather than in the book, and the tool now prints them.

Then the class no existing guard could see. `doc_examples_test` proves
the book's programs *run*; `doc_references_test` proves the names it
drops *exist*. Neither looks at the value a `// comment` claims a line
prints, so an example could run perfectly while teaching something
false. Fourteen did. A list of strings prints with its quotes and the
book wrote seven of them bare (`// [west, east]` for `["west",
"east"]`); a Float column prints with its `.0` and five comments dropped
it; `show` on an enum variant qualifies it (`Color.Blue`, not `Blue`);
and three comments naming a regression's true coefficient read as claims
about a line that prints something else entirely.

These are the comments a reader trusts most, because they are the only
place the book says what a value *is* rather than what a function does.
So the check is now permanent: `tests/doc_outputs_test.rs` runs every
block that claims a value and compares. Its whole design problem is
telling a claimed value from a note — most comments are notes, and
flagging those would make it a guard someone switches off — so it
accepts only what is unambiguously a value, and skips any block where a
`println` emitted more than one line rather than guessing at the
correspondence. A comment it declines to check costs a little coverage;
one it wrongly accepted would cost the guard its credibility.


**R1, fourth sitting.** Two lanes: the idiom the chapter taught but did
not follow, and a coverage question the existing guards ask only in one
direction.

`ods.md` gained a "Reaching a column" section in 0.66 explaining that
`f["amount"]` *is* how a column is reached — and then every example
after it went on calling `ods.column(f, "amount")`, six times before the
section and three after. Worse, `ods.column` is never introduced in the
chapter's prose at all, so a reader met an unexplained function six
times before being taught the explained way to do the same thing. The
examples now use the subscript throughout, and `column(f, name)` is
named in the stdlib reference as the call form of it, which it had also
been missing from.

The coverage question: `doc_references_test` checks doc → binary, that
every name the book drops resolves. Nothing checked binary → doc, that
every function the binary registers is named somewhere. Enumerating all
430 found three that are not: `ods.version()`, now documented, and
`ods.probe`/`ods.probe_tag`, which are Phase 0's plumbing fixture
sitting in the public surface. Removing them is a 1.0 surface decision
rather than a documentation one, so it is logged rather than done.

Worth recording because it bears on whether that check should become
permanent: the ad-hoc version of it gave three wrong answers before a
right one. It reported zero (it called `keys`, which does not exist, and
silently enumerated nothing), then 116 (it missed that reference tables
list bare backticked names), then five (it missed that
`random.randstr(n) (+ _alpha _alnum _numeric)` documents four functions
in one table row). Every one of those looked plausible. A guard built on
the obvious implementation would be wrong in exactly the direction that
gets a guard switched off, so the reverse check stays manual until
someone can make it careful enough.


**R1, fifth sitting.** `internals.md`, the chapter that exists to
explain *why* the runtime is built as it is — and therefore the one where
a stale explanation is hardest to notice, because the prose still reads
as reasoning.

Its documented gate was wrong in a way that matters. "Gates for every
change: `cargo test --release`" — but the repository root is both a
package and the workspace root, so a bare `cargo test` builds and tests
`olang` alone. The engine crate's property tests under `olang-ods/`, and
`otc`'s, never run. Someone could change a kernel, pass every gate the
book states, and ship it broken. The gate is `--workspace`, and the
chapter now says why.

The value model described `List(Arc<[Value]>)`. It is `Arc<Vec<Value>>`,
and the difference is load-bearing rather than pedantic: a boxed slice
cannot grow, so the in-place append fusion the language ships would be
impossible under the representation the book described. The struct
variant likewise omitted its `Arc`, which is the fix that stopped a
struct argument from copying every field.

The repository map was missing eight files, three of them whole shipped
campaigns: `caps.rs` (capabilities), `timeline.rs` (record/replay), and
`scoping.rs` (the 0.61 scope and mutability rules). A reader using the
map to find the capability model would conclude there wasn't one.

The testing table still credited `example_programs_test.rs` with "the
curated `examples/*.ol`", which have not existed since they were folded
into `examples/demo`, and listed neither the whole-program tier harness
nor either doc guard. And "an example program is a directory with
`olang.toml` and `main.ol`" overstates: 17 of 28 are packages, the rest
are bare scripts, and `run_all.ol` finds either by looking for the
`main.ol`.


**R1, sixth sitting.** `ovm.md`, and a comment in the compiler it
describes.

The chapter's correctness policy said the no-divergence rule "is
enforced by two test suites". It is three, and the third is the one this
chapter's own promise called into being: `tier_agreement_test.rs` quotes
`docs/ovm.md` in its header. Naming only the per-function suites
understated the guarantee in precisely the section that states it. The
entry now says why both scales are needed, using the divergence that
motivated the harness — the interpreter reading `()` as Unit while the
compiler built a zero-element tuple, so a hot function comparing
`x == ()` answered differently from a cold one, and nothing failed.

That is now the third chapter found listing a stale set of test suites
(`stability.md` and `internals.md` were the others). The pattern is
consistent enough to name: prose that enumerates the project's own
machinery goes stale silently, because adding a test never prompts
anyone to re-read the paragraph that counts them.

In the compiler itself, `src/ovm/bytecode.rs` introduced its
compilable-builtin set with "Higher-order builtins (map, filter,
reduce, ...) are excluded because a function argument cannot reach the
VM" — and then listed `map`, `filter`, `reduce`, and `fold` 130 lines
below, under a note explaining they became reachable once non-capturing
lambdas compiled to function values. Both exclusions in the leading
comment had been lifted; only the later note said so, which anyone
reading the list top-down would meet second.

One thing checked and deliberately left: the chapter's limitation 3
distinguishes a "native set" from builtins that "bridge" at a value
round trip. `builtin_names` turned out to gate something else — whether
a call compiles at all — so confirming or correcting that sentence needs
more of the dispatch path read than this sitting covered. Left as
written rather than rewritten on a guess.


| Lane | Work | Status |
|---|---|---|
| R1 — book audit | A full pass over the book against the final language: every chapter verified against implementation behavior, every example exercised, the semantics-release changes reflected everywhere. | **in progress** — mechanical checks built and clean; `stability.md`, `pitfalls.md`, `tooling.md` and the `language.md`/`ods.md` claim errors corrected, output comments now guarded; the long reference chapters remain to be read end to end |
| R2 — the 1.0 contract | [Stability and compatibility](stability.md) is rewritten as the 1.0 compatibility contract: what is frozen, what semver means from here, and the support expectations for each surface. | planned |
| R3 — release | The 1.0 release itself: final gates, the CHANGELOG's 1.0 entry, and version 1.0.0. | planned |

## Frozen until 1.0

The following surfaces receive bug fixes, documentation corrections, and
whatever updates Campaign 1 forces (the tiers, checker, and language server
must track the new semantics), but no new features until 1.0 ships:

- **The browser stack** — `dom`, `ui`, `viz`, `dash`, the playground, and
  the website.
- **JIT breadth and the adaptive engine** — the JIT's positioning is
  settled: the interpreter defines semantics, the Rust kernels of the data
  stack carry the performance identity, and the JIT accelerates numeric
  code within its tested whitelist. Extending the whitelist resumes, if at
  all, after 1.0.
- **The language server and editor extensions** — feature growth pauses;
  diagnostics track Campaign 1.
- **The registry and package ecosystem** — the mechanism is complete;
  growth is deferred.

## Post-1.0 candidates

Recorded so they are not lost; none is a commitment.

- The adaptive-engine campaign (feedback-driven tier promotion).
- Reproducible builds (byte-deterministic `olang build`).
- Multi-file `olang build` (bundling a package, not a single source file).
- Replay provenance (`replay --why`: tracing a value back through the
  recorded inputs).
- Timeline reach: recording `db` reads and channel interleavings so
  database-backed and concurrent programs replay end to end.
- Asynchronous I/O, only if designed as genuine non-blocking I/O; the
  removed async/Promise surface is not coming back in its old form.

## Process

- Work lands campaign by campaign; a campaign's acceptance criteria are
  checked before the next begins. Campaigns 2 and 3 may interleave where
  their surfaces do not overlap.
- Every change passes the standing gates: the workspace test suite, the
  executed documentation examples, the example harness, and a clean
  formatter and checker run.
- Decisions D1–D9 are not reopened by implementation difficulty; if a lane
  proves harder than planned, the plan's schedule moves, not its direction.

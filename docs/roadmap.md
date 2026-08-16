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
| D8 | Error model | `Result` with `?` is the primary mechanism for expected fallibility. `try`/`catch` is retained with a narrow, documented role: recovering from runtime errors at coarse boundaries (supervisors, servers, task edges). Using `catch` for ordinary control flow becomes a checker warning. |
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

**S5 shipped in 0.63.0.** `async`, `await`, and the `Promise` API are
gone; `spawn` returns a task handle that `task.join` collects. The
deadline-based promise scheduler is deleted rather than repositioned, as
D5 required — it simulated latency without providing concurrency, while
`spawn` beside it was a real thread and `await` on a spawned task was
really a join. One amendment to D5's wording: `Promise.race` has no
direct replacement. `task.join_timeout(t, ms)` bounds the *wait*, and
the task keeps running, because an OS thread cannot be cancelled from
outside without leaving what it touched in an unknown state. A general
`race` would have implied the losers stopped. The remaining lanes
(S6–S8) are unchanged.

| Lane | Work | Status |
|---|---|---|
| S1 — lexical scoping | Blocks introduce a scope: `let` bindings are dropped at the closing brace, shadowing is permitted, and the environment model in the interpreter and both compiled tiers is updated together. The 0.50 scope-leak warning becomes an error. | **shipped 0.61.0** |
| S2 — required `let` | Assignment to an undeclared name is an error naming the variable and suggesting `let`. The 0.50 advisory warning becomes the error. | **shipped 0.61.0** |
| S3 — enforced `mut` | Reassignment of a non-`mut` binding is an error. The checker and language server list every site to migrate; the corpus is migrated in the same change. | **shipped 0.61.0** |
| S4 — the cell | `cell(v)`, `cell.get(c)`, `cell.set(c, v)`, `cell.update(c, f)`. Cells are values with identity confined to their creating thread: reading or writing one from another thread is an error, and `chan.send` refuses to send one. Dead captured-variable writes became errors pointing to `cell`. Timeline recording is unaffected because cell mutation is deterministic within a thread. | **shipped 0.62.0** |
| S5 — remove async | `async`, `await`, and the `Promise` API are removed from the interpreter and tiers. `spawn` returns a task handle collected by `task.join` / `task.join_timeout`. Programs using `Promise.delay/all/race` migrate to `time.sleep`, `map(task.join)`, and `chan`; the book's concurrency chapter is rewritten around the single model. | **shipped 0.63.0** |
| S6 — stdlib conventions audit | Every builtin and module function is audited once: infallible operations return their value directly (`os.args`, `os.arch`, and the other host-introspection calls are the known cases); contextual keywords free `share` and any other colliding identifiers; `fs.join` becomes variadic; the definitive before/after table is recorded in the CHANGELOG. | planned |
| S7 — error-model boundary | The `catch`-for-control-flow checker warning lands, and the book's error-handling chapter is rewritten around the Result-primary model with `catch` documented for boundary recovery only. | planned |
| S8 — migration | The release ships with a migration guide. `olang check` reports every site the release breaks (its 0.50 warnings are the census); the repository corpus is migrated in the release itself as the proof of the guide. | planned |

Acceptance: all gates green under the new semantics (workspace tests, doc
examples, the example harness, tier-agreement suites); the CHANGELOG
documents every break with its rationale and migration; no advisory warning
remains for behavior that no longer exists.

## Campaign 2 — data-pipeline completion

Completes the campaign begun in 0.60.0 (the parallel join and `group_by`
aggregation landed there). This work is additive and independent of
Campaign 1's semantics; example programs written for it are migrated with
the corpus if Campaign 1 lands first.

| Lane | Work | Status |
|---|---|---|
| DP1c — parallel group keys | Parallelize the group-identification pass of `group_by`, which now dominates its runtime; the aggregation pass is already parallel. | planned |
| DP2 — IO breadth | Read CSV from files and as a bounded-memory stream; JSON-lines input; a columnar interchange format; `to_csv` and file output. The end-to-end input-to-output story. | planned |
| DP3 — verb completeness | Window functions, reshape (wide/long, pivot), additional join kinds and aggregations, and string and categorical column operations. | planned |
| DP4 — flagship and benchmark | A realistic end-to-end ETL example wired to `plot`, and a reproducible benchmark against pandas and Polars with methodology and hardware documented in [The data stack](ods.md). | planned |

Acceptance: a streaming job processes input larger than memory; the
benchmark is reproducible from the repository; the flagship example ships in
`examples/` and runs in the harness.

## Campaign 3 — capabilities on every tier

Implements D6. Enforcement today runs on the interpreter tier only, and a
capability-restricted run forgoes the bytecode tier; this campaign removes
that trade.

| Lane | Work | Status |
|---|---|---|
| C1 — builtin-boundary enforcement | Capability checks move to the single dispatch point that all tiers share. Each compiled function carries its source-package provenance; OVM frames expose it; JIT code reaches builtins through helpers that pass through the same dispatch. The restriction that disables the bytecode tier under a capability manifest is then removed. | planned |
| C2 — attribution hardening | Package roots are canonicalized so symlinks cannot confuse attribution; `os.exit` and remaining ungated process-affecting calls are brought under the `proc` gate; bundle-format fields are validated on read. | planned |
| C3 — denial and the error model | A capability denial is a runtime error recoverable at a `catch` boundary, consistent with D8, so a sandboxed program can degrade deliberately rather than only abort. | planned |

Acceptance: a capability-restricted run matches the unrestricted run's tier
behavior and speed; the capability test suite passes on every tier; the
book's openness and packages chapters describe capabilities as an enforced
boundary, with the attribution model and its limits stated precisely.

## Campaign 4 — 1.0 readiness

| Lane | Work | Status |
|---|---|---|
| R1 — book audit | A full pass over the book against the final language: every chapter verified against implementation behavior, every example exercised, the semantics-release changes reflected everywhere. | planned |
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

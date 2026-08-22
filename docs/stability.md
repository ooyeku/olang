# Stability and compatibility

Part of [the olang book](README.md) · [Architecture and internals](internals.md)

This chapter is olang's **compatibility contract**: the authoritative
statement of what is guaranteed, what may still change, and what a version
number means. Where the README, an old comment, or any other text disagrees
with what is written here and in the reference chapters, this document is
correct.

The language surface is **closed**. The syntax and behavior documented in
[the language reference](language.md) and [the stdlib reference](stdlib.md)
are a commitment, not a snapshot: the three deliberate breaking releases —
gradual-typing enforcement in 0.48, the scope, mutability,
stdlib-convention, and error-model changes across 0.61–0.65, and the
operator-precedence and absence-convention corrections in 0.68 — have all
shipped, each with a migration guide in the CHANGELOG. The 0.68 release
exists because stability is valuable in proportion to adoption: the old
precedence table froze three known footguns (`&&`/`||` on one level, `|>`
and ranges binding tighter than arithmetic) at a moment when fixing them
was still cheap, and pre-1.0 was the last such moment. What remains before the 1.0 tag is this contract and the release
mechanics, not further changes to what already works. The guarantees below
hold now and are what 1.0 commits to permanently; the [semver
contract](#versioning) states what could ever change them and at what cost.

## The 1.0 commitment

1. **Documented syntax keeps parsing.** A program written against the
   language reference continues to parse in future releases. New syntax is
   additive.
2. **Documented behavior keeps behaving.** The meaning of every construct
   with a runnable example in this book is locked by
   `tests/doc_examples_test.rs` — a change that breaks a documented example
   is a regression, not an evolution.
3. **The stdlib API is append-mostly.** Functions may be added; existing
   signatures and return conventions (which functions return `Result`)
   stay. Renames, if ever needed, keep the old name as an alias for at
   least one minor release with a deprecation note.
4. **The execution tiers agree.** Any observable difference between
   `--no-ovm` and the default tiered execution — bytecode or JIT — is a
   bug. Optimization work must be invisible.

## What each surface guarantees

Not every part of olang carries the same promise, and the honest thing is
to say which carries which. Four tiers, from the frozen core outward:

| Tier | What you can rely on | Examples |
|---|---|---|
| **Stable** | Syntax and behavior are frozen. A documented program keeps parsing and keeps giving the same result, under the semver contract above. | the core language, the listed stdlib modules |
| **Stable in behavior, evolving in scope** | Every documented function behaves as written, permanently. The *set* of functions grows additively; nothing existing changes. | concurrency, the OVM/JIT tiers, the data stack, gradual typing |
| **Experimental** | The model is settled and tested, but the surface may still gain fields or grow. Build on it; expect additions, not removals. | capabilities, `http.serve`, `dom`, `cell`, `testing` session state |
| **Reserved** | Not accepted at all. Writing one is a parse error, and if it ever gains meaning that will be purely additive. | intersection annotations, union *declarations* |

The rest of this section is the detail behind each tier.

### Stable

The core language: literals, variables and assignment, all operators and
their precedence, strings, lists/tuples/maps/objects, ranges, control flow,
pattern matching (all documented pattern kinds), functions (defaults, named
arguments, lambdas, closures-by-value, `return`), pipelines, `break value`,
structs, enums and their constructors (including cross-module), `error`
declarations, traits and impls, `Result` + `?`, modules (`use`/`share`
in all documented forms), `test` blocks, and the standard-library
modules `str`, `col`, `math`, `json`, `toml`, `csv`, `re`, `dates`,
`time`, `base64`, `fs`, `os`, `db`, `random`, `crypto`, `chan`, `task`,
`cell`, `proc`, `meta`, `caps`, and the global builtins. The
embedded olang modules (`colx`, `mathx`) and packages (`cli`, `term`, `ui`,
`viz`, `dash`) follow the same append-mostly rule.

**Scope and mutability settled in 0.61.0**, as the second deliberate
breaking change before 1.0 (recorded in the CHANGELOG with a migration
guide). Three rules that were advisory warnings in 0.50–0.60
became enforced errors, checked statically before a program runs and
therefore identical on every tier:

- Every block scopes its bindings. A `let` inside `{ ... }`, a loop
  body, an `if` branch, or a `match` arm ends with that block.
- Assignment is not a declaration. `x = 1` for an unbound `x` is an
  error; bindings are introduced by `let`.
- `let` is immutable, `let mut` is not. Assigning to a binding not
  declared `mut` is an error; shadowing with a fresh `let` is always
  available.

0.62.0 completed the model with a fourth rule and the primitive that
makes it livable: assigning to a binding *captured* from an enclosing
scope is an error (the write could only reach the closure's snapshot),
and [`cell`](stdlib.md#cell--mutable-locations) is the one mutable
location, confined to the thread that created it.

**The stdlib conventions settled in 0.64.0.** One rule decides what
every function returns, so its shape follows from what it does: an
operation that cannot fail returns its value, one that can fail for
reasons the caller could handle returns `Result`, and one *called
wrongly* raises. That third tier is what makes the second trustworthy —
before it, `unwrap_or(f(x), default)` could not tell a missing file from
a typo'd call. The same release freed seven declaration keywords
(`share`, `error`, `test`, `type`, `trait`, `impl`, `use`) as ordinary
identifiers and made `fs.join` variadic.

**The error model settled in 0.65.0.** `Result` with `?` carries
expected failure. A runtime error is a bug and stops the program: there
is no construct that catches one mid-expression, and `try`/`catch` —
which never did, despite looking like it — was removed. Recovery is
*structural*, at the boundaries that already isolate a failing unit: a
spawned task's failure becomes `Err(e)` from `task.join`, and an
`http.serve` handler's becomes a logged 500 with the server still
serving. `try` and `catch` are ordinary identifiers, as are `async`,
`await`, and `Promise` — the reserved-word list is fifteen words plus
seven contextual ones, and is not expected to change again.

These rules are part of the commitment above and are permanent under the
[semver contract](#versioning): changing any of them would be a 2.0, and
2.0 is the version that is not meant to happen. Campaign 1 of
[the roadmap](roadmap.md) — the language surface — is complete and closed.

### Stable in behavior, evolving in scope

- **Concurrency** — the documented API (`spawn`, `task.join`,
  `task.join_timeout`, `chan`, `par_map`, `par_filter`, `par for`) is
  stable as of 0.63, which removed `async`/`await` and the `Promise`
  API and left one model: threads. `spawn` runs on a real OS thread and
  returns a task handle; `par_map`/`par_filter` carry spawn's snapshot
  semantics and are differential-tested against `map`/`filter`;
  `task.join_timeout` bounds the wait, never the work, because an OS
  thread cannot be cancelled. The scheduling model may gain further
  capability without changing what existing programs observe.
- **The OVM and JIT tiers** — which functions get promoted or compiled
  to native code, and how fast they run, changes freely; results never
  do.
- **The data stack (`ods`, `stats`, `plot`)** — the documented functions
  behave as the stdlib chapters state, pinned by doc tests, engine
  property tests, and scipy/NumPy reference constants; the *scope* grows
  (new dtypes, verbs, statistics, chart kinds) under the append-mostly
  rule. Design decisions and their measured justifications live in
  [the ods chapter's design record](ods.md#the-design-record), including
  recorded deferrals (lazy evaluation, faer) with the conditions that
  would reopen them.
- **Gradual typing** — olang is gradually typed, as documented in
  [the Types chapter](types.md): annotations are enforced at runtime
  (parameters, returns, `let` bindings, struct fields — shallow
  container checks, `Result<T, E>` payload checks one level deep,
  `A | B` unions by any-branch, function types by callability and
  arity, strict Int/Float, identical on every tier), and
  `olang check` plus the language server report provable violations
  statically. All documented annotation forms keep parsing; the
  enforcement semantics are locked by the chapter's doc tests; the
  checker's no-false-positive discipline is a fixed rule — it may learn
  to prove *more*, but a clean program stays clean. Enforcement landed
  as the one deliberate breaking change of 0.48.0 (recorded in the
  CHANGELOG); reserved annotation forms below remain unenforced until
  their semantics land.

### Experimental — may change or be completed

- **Macros (`meta fn`, `@`, `olang expand`).** New in 0.68 and the most
  recent surface in the language ([Macros](macros.md)). The five laws —
  `@`-visible sites, no reach beyond the site, total parse, pure
  expansion, inspectable output — are the settled design. The invocation
  surface (expression sites; decorators on `type`, `fn`, and `let`),
  imported macro libraries via `use`, the template escapes, and the
  `meta` helpers (`eval`, `lit`, `fresh`) are implemented and hardened —
  fuzzed for tier agreement and byte-level expansion determinism, with
  placement rules enforced (top-level meta fns only, no `@` inside a
  meta fn body). The syntax was added additively: `@` was previously
  unused and `meta` remains an ordinary identifier everywhere except
  directly before `fn`, so no pre-macro program changed meaning.

  Graduation to stable requires, and is blocked on, all of: a macro
  fuzzer corpus an order of magnitude larger run clean (**done** —
  10,000 generated macro programs: expansion determinism, tier
  agreement, and clean refusal checked per seed; the run surfaced and
  fixed one real defect, nested macro calls in arguments. The generator
  is committed as `tests/macro_fuzz_corpus_test.rs` — a seeded,
  deterministic corpus whose first 150 seeds run on every `cargo test`
  and whose full campaign is the `#[ignore]`d
  `macro_fuzz_full_campaign`; `tests/tier_fuzz_corpus_test.rs` is the
  equivalent committed form of the tier-differential campaign, so both
  claims are reproducible rather than historical); runtime error
  spans source-mapped to `@` sites (**done** — every expanded line
  carries its origin, and errors point into the file as written, naming
  the generating macro); LSP expansion awareness (**done** — the
  semantic pass runs on the expanded program with positions mapped back
  to the buffer, generated-code findings name their macro, and expansion
  failures are diagnostics at their site); and at least three
  substantial macro libraries used by real programs in the corpus
  (**done** — `examples/derives`, `examples/instrument`, and
  `examples/contracts`, all imported with `use`, all under the harness
  and the tier corpus). Every listed criterion is now met; the
  graduation itself is a deliberate act for a later session, not an
  automatic consequence — experimental status holds until it is
  explicitly lifted.
  Until then the expansion engine's internals — the round/fuel model,
  the source-text exchange format's exact whitespace behavior — may
  change in ways `olang expand` output would show.

- **Capabilities and the transparent binary.** The `[capabilities]`
  manifest, per-dependency attenuation, `--deny`, and `olang inspect`
  are new: the model (opt-in restriction, shrink-only attenuation,
  gate at the effectful-module boundary, source-carrying binaries) is
  settled, but the gated surface may grow (new modules), the bundle
  format may gain fields. Enforcement runs on every tier, so a restricted
  run keeps full speed: the grant a program is given is independent of
  how fast its code happens to be running.

- `http.serve` — the request/response API (the `HttpRequest` fields, string
  and response-struct returns) is settled and integration-tested; the
  *execution model* (bounded worker pool, blocking caller, localhost-only)
  may grow further without changing existing handlers.
- The `dom` module — browser-only, and young: the function surface may
  grow (and payload conventions may gain fields) as frontend programs
  demand more; the element-handle model and the stateless pattern it
  supports are the stable core.
- `testing.test_summary` / `testing.reset_tests` are real as of 0.50:
  every `testing.assert_*` outcome is tallied per thread, `test_summary()`
  returns `#{ "passed", "failed", "total" }`, and `reset_tests()` zeroes
  it. `testing.run_test` deliberately remains a redirect to `test`
  blocks (a builtin cannot re-enter the interpreter to run your
  function; the error says so).
- The `--enable-parallel` / `set_parallel` evaluation modes
- The `cell` module is new as of 0.62: the model (one mutable location,
  confined to its creating thread, refused at `chan.send`, re-entrant
  `cell.update` rejected) is settled and covered by the scope and
  mutability commitment above, but it is the newest surface in the
  language and the least exercised by real programs.

### Reserved — not accepted, and additive if they ever are

Two type-level forms are named in the grammar's design but rejected by
the parser today. Writing either is a parse error, not a construct
waiting for semantics:

- Intersection type annotations (`A & B`)
- Union type *declarations* (`type X = A | B`). Union and literal-type
  *annotations* are a different thing and do have semantics — they
  gained them in 0.50 and are covered by the gradual-typing bullet
  above.

Both are safe to avoid entirely, which is the only option. If either is
ever accepted it will be additive, since no program can contain one now.


## Versioning

olang follows [semver](https://semver.org). From 1.0 onward a version
number means exactly this:

- **Patch** (`1.0.0 → 1.0.1`) — bug fixes only. No new surface, no changed
  behavior for a program that was relying on documented behavior.
- **Minor** (`1.0 → 1.1`) — additive. New functions, new modules, new
  syntax that does not change how any existing program parses or runs.
  Everything in [the commitment](#the-10-commitment) still holds.
- **Major** (`1.0 → 2.0`) — the only version that may break a documented
  program, and the bar is deliberately high: a change that would break a
  documented example is a 2.0 change, not a 1.x one, no matter how small.
  The intent is that 2.0 never needs to happen; it exists so the promise
  above can be absolute rather than hedged.

**What "breaking" means, precisely.** A change breaks compatibility if a
program written against this book — its syntax, its documented behavior,
its stdlib signatures and `Result` conventions — stops parsing, stops
running, or produces a different result. This is not a matter of judgment:
every documented example is executed in CI (`doc_examples_test`), so a
breaking change is a *failing test*, and shipping it in anything below a
major version is a release bug.

**Deprecation.** If a stdlib function ever has to be renamed, the old name
stays as an alias for at least one minor release, with a deprecation note
in its documentation and the CHANGELOG. A name is never removed in the
same release it is deprecated.

**Bug fixes are not breaking changes.** Behavior that changed because it
was a *bug* — `&&` not short-circuiting, `?` aborting instead of
propagating — is a fix, not a break, and lands in a minor or patch
release. Such changes are recorded in [CHANGELOG.md](../CHANGELOG.md)
under Fixed, with the reasoning, so the distinction between "we fixed
what was wrong" and "we changed what was right" is always on the record.

**Before the 1.0 tag.** The surface is already frozen; the version number
has not yet caught up. Until 1.0.0 is tagged, minor releases (`0.66 →
0.67`) remain additive and fix-only in practice, and the contract above is
the one they already honor. The 1.0 release changes the number and the
promise's *formality*, not the code's behavior.

## How this is enforced

- Every code block in the book runs in CI (`doc_examples_test`).
- Every `module.function` the book *names in passing* is resolved against
  the real binary (`doc_references_test`). Running the examples proves
  the examples; this covers the far larger surface the prose mentions
  without demonstrating, which is where a rename quietly leaves a lie.
- Every fixed bug lands with a regression test (one file per area under
  `tests/`).
- The examples — real programs — run end-to-end via
  `examples/run_all.ol` and `example_programs_test`.
- Tier agreement is tested two ways. `bytecode_tier_test` and
  `bytecode_differential_test` call individual functions on both tiers;
  `tier_agreement_test` runs every runnable book example and every
  standalone example program as a whole program under `--no-ovm` and
  under `--ovm-tier=1` and diffs stdout, stderr, and exit status. The
  second exists because the first cannot catch a divergence that needs a
  whole program to express — and the first one found was exactly that.
- `colx`/`mathx` differential tests exercise the language against its own
  stdlib.

If you find documented behavior that does not match the implementation,
that is a bug in one of them — please file it. The book is the contract,
and this chapter is where the contract is stated; every other chapter is
bound by it.

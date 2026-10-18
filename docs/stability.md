# Stability and compatibility

Part of [the olang book](README.md) · [Architecture and internals](internals.md)

This chapter is olang's **compatibility contract**: the authoritative
statement of what is guaranteed, what may still change, and what a version
number means. Where the README, an old comment, or any other text disagrees
with what is written here and in the reference chapters, this document is
correct.

The language surface is **closed**. The syntax and behavior documented in
[the language reference](language.md) and [the stdlib reference](stdlib.md)
are a commitment, not a snapshot. The deliberate breaking changes before
1.0 have shipped, each recorded in the CHANGELOG:
gradual-typing enforcement in 0.48; the scope, mutability,
stdlib-convention and error-model changes across 0.61–0.65; operator
precedence in 0.68; and, in 0.88, the last release before 1.0, the
settling of records (a missing field of an object reads as `()`), of
modules (a missing member reads as `()`), of how a lambda stage in a
pipeline ends, and of a map's order (its keys as they were first set)
([Settled before 1.0](#settled-before-10)). Earlier
editions of this chapter called 0.61 and then 0.68 the final deliberate
break; 0.88's changes were made because 1.0 freezes what it ships. What remains before the 1.0 tag is
the release itself. The guarantees below hold now and are what 1.0
commits to permanently; the [semver contract](#versioning) states what
could ever change them and at what cost.

## Table of contents

- [The 1.0 commitment](#the-10-commitment)
- [Platforms](#platforms)
- [Settled before 1.0](#settled-before-10)
- [What each surface guarantees](#what-each-surface-guarantees)
- [Tools and file formats](#tools-and-file-formats)
- [Versioning](#versioning)
- [How this is enforced](#how-this-is-enforced)

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
   least one minor release ([Deprecation](#versioning)).
4. **The execution tiers agree.** Any observable difference between
   `--no-ovm` and the default tiered execution — bytecode or JIT — is a
   bug. Optimization work must be invisible.

## Platforms

1.0 promises **macOS**: it is the platform every release is tested and
verified on (Apple silicon; the Intel build is made from the same
source), and every guarantee in this chapter is made for it. Linux and
Windows are **unsupported previews**: Linux binaries are released and
its headless tests pass, Windows compiles and has not been run, and a
program is welcome to run on either, but no behaviour there is
promised — the desktop engine's windows on Windows and Wayland, the
terminal layer on Windows, and the capabilities `gui.platform()`
reports false are untried or incomplete (the roadmap's [parked
platforms](roadmap.md#platforms-parked)). A difference between macOS and
a preview platform is not a break of this contract. When a platform is
supported, this section will say so, and that is an addition.

## Settled before 1.0

These rules were decided in 0.88 and in the pass that followed it, so
that 1.0 ships them; they are part of the commitment and will not be
reversed:

- **A missing field of an object or a parsed JSON object reads as `()`**,
  as a map's missing key does (`o.k`, `map_get(o, "k")`). A declared
  struct's missing field raises. Code written against this rule keeps
  working under every 1.x release; it is not a breaking change after
  1.0, because it is the 1.0 behaviour. `olang check` reports a read of a
  key no construction of the value has, and of a field a declared struct
  never has, as the safety net.
- **A module's missing member reads as `()`**; calling it raises naming
  the nearest member.
- **A field of `()` reads as `()`** (`s.pp.cur` when `s.pp` is absent),
  and `map_get((), k)` is `()`; a field of any other non-record value
  (`5.k`) and a declared struct's missing field still raise. `r?.f` is
  reserved: a parse error, so it stays free to mean optional chaining.
- **An unparenthesized lambda stage in a pipeline ends at the next
  `|>`.**
- **`??`'s level** is below the arithmetic, the pipeline and the ranges,
  above the comparisons.
- **A map keeps its keys in the order they were first set**, as a
  Python dict and a JavaScript object do; an object's (and a parsed JSON
  object's) fields keep theirs. Iteration (`entries`, `map_keys`,
  `map_values`), printing, `meta.lit`, `json.stringify` and
  `http.encode_query` follow it; setting a key again keeps its place,
  removing one closes the gap. Equality ignores the order (`#{ "a": 1,
  "b": 2 } == #{ "b": 2, "a": 1 }`). A record's columns follow it too:
  `ods.frame_from_records` and `ods.read_jsonl` take the keys in the
  order first met, `db.query`'s rows the SELECT's.

## What each surface guarantees

Not every part of olang carries the same promise, and the honest thing is
to say which carries which. Four tiers, from the frozen core outward:

| Tier | What you can rely on | Examples |
|---|---|---|
| **Stable** | Syntax and behavior are frozen. A documented program keeps parsing and keeps giving the same result, under the semver contract above. | the core language, macros, the listed stdlib modules, `cell`, `testing` |
| **Stable in behavior, evolving in scope** | Every documented function behaves as written, permanently. The *set* of functions grows additively; nothing existing changes. | concurrency, the OVM/JIT tiers, the data stack, gradual typing, `runtime`, `asset`, `collections` |
| **Experimental** | The model is settled and tested, but the surface may still change: a minor release may change or remove an Experimental function, option or field, with a CHANGELOG note saying what to change ([Versioning](#versioning)). Build on it knowing that. | capabilities and `caps`, `http.serve`, `dom`, `gui`, `tty`, `pty`, `vt` |
| **Reserved** | Not accepted at all. Writing one is a parse error, and if it ever gains meaning that will be purely additive. | intersection annotations (`A & B`) |

Every module stdlib.md documents sits in exactly one tier. A name the
reference calls internal — the `collections` layouts, `gui.flatten`,
the `gui.flat_` family and `gui.roles` (Loom's layout internals), the
`ods` probes — is in none: it is outside the contract and may change in
any release.

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
`cell`, `proc`, `meta`, `bytes`, `compress`, `bigint`, `vec`, `http`'s
client (`http.get`, `http.post`, `http.request` and the URL and query
helpers), `testing` (the `assert_*` family, `snapshot` and its helpers,
`test_summary` and `reset_tests`), `set_parallel`, and the global
builtins. The embedded olang modules (`colx`, `mathx`) and packages
(`cli`, `term`, `ui`, `viz`, `dash`) follow the same append-mostly rule.
`cell` and `testing`'s session state (`test_summary`, `reset_tests`),
Experimental before, are promoted: `cell`'s model — one mutable
location, confined to its creating thread, refused at `chan.send`,
re-entrant `cell.update` rejected — is the scope and mutability
commitment below, and every program in the corpus that mutates shared
state is built on it.

**Scope and mutability settled in 0.61.0**, as the second deliberate
breaking change before 1.0 (recorded in the CHANGELOG; the later ones
are listed at the top of this chapter). Three rules that were advisory warnings in 0.50–0.60
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
location, confined to the thread that created it. A closure captures the
local bindings it names by value; a top-level name is never captured —
every function reads its binding as it is when the function runs, on
every tier (settled before 1.0; language.md, "Closures capture by
value").

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
is no construct that catches one *mid-expression*, and `try`/`catch` —
which never did, despite looking like it — was removed. Recovery is
*structural*, at the boundaries that already isolate a failing unit, and
each boundary is a whole call: `attempt(f)` answers `Err(message)` when
a runtime error ended `f()`, `runtime.call_budget(f, args, ms)` reports
the error (or the time running out) that ended its call, a spawned
task's failure becomes `Err(e)` from `task.join`, and an `http.serve`
handler's becomes a logged 500 with the server still serving. Nothing
resumes inside the call that failed. `try` and `catch` are ordinary identifiers, as is `Promise`.
The reserved-word list is fixed at 1.0: twenty-one
words, six of them (`await`, `yield`, `async`, `defer`, `const`, `pub`)
reserved for forms the language may add. It will not grow after 1.0 —
that is the promise that keeps every identifier a program uses valid
forever; a new form uses one of the six, a contextual word, a builtin,
or an `@` macro. The *contextual* declaration words (eight,
after `meta` joined in 0.68) may gain members additively, since a
contextual word never stops being an ordinary identifier.

**Absence and the sentinels.** Where an operation finds nothing, or
nothing has arrived yet, it answers `()` (or `Ok(())` from a function
that answers a `Result`). Three error strings are named and are
contract: `"eof"` (a stream ended), `"timed out"` (a wait's time ran
out) and `"channel is closed"` (a channel closed and drained); a
program may match them, and they will not change in 1.x
([stdlib.md](stdlib.md#absence-is-unit) lists the three).

**Error text is not contract.** The text of a runtime error — what
`attempt` hands back, what a failed run prints, what `runtime.last_error`
reports — and the text of a stdlib `Err` payload are written for people
and may change in any release, including a patch; the tiers may word
one failure differently. Two things are contract: the message a
program gives `raise(message)`, which `attempt` and `task.join` hand
back exactly as given, and the three sentinels above. A program that
must tell one failure from another matches a sentinel, or raises its
own message, never another error's wording.

**Number semantics are read-back-clean.** Integers are 64-bit signed
and overflow is a runtime error pointing at `bigint`, never a silent
wraparound. Float literals accept a decimal point, an exponent, or
both (`3.14`, `1e20`, `2.5e-3`), and printing round-trips: `to_string`
of any finite float is a valid literal reading back to the identical
value. Floats trap rather than produce `NaN` or `inf` — `0.0 / 0.0`,
`math.sqrt(-1.0)`, overflow (`1e308 * 10.0`) and the other operations
whose result is not a finite number raise, on every tier. JSON keeps numbers lossless or loud: integers within
i64 arrive exactly, decimals take the standard IEEE reading, and an
integer outside i64 (or a float text that overflows f64) is an `Err`
naming the value — never a silently-lossy conversion.

**Macros stabilized in 0.68** ([Macros](macros.md)), graduated after
meeting every criterion the experimental entry set: source-mapped
runtime error spans, an expansion-aware language server, a
ten-thousand-program fuzz corpus run clean, and three macro libraries
imported by real programs in the corpus. Both fuzz campaigns are
committed as seeded, deterministic generators rather than historical
claims: `tests/macro_fuzz_corpus_test.rs` and
`tests/tier_fuzz_corpus_test.rs` run a 150-seed smoke corpus on every
`cargo test`, and their `#[ignore]`d `*_full_campaign` tests reproduce
the complete 10,000-seed runs on demand. What is frozen is the model,
not its incidentals:

- The syntax forms — `meta fn` declarations, `@name(args)` in
  expression position, `@name` decorators stacked on `type`, `fn`, and
  `let` declarations — and their additive nature (`@` was unused;
  `meta` is an ordinary identifier except directly before `fn`).
- The five laws: every expansion site says `@`; a macro reaches only
  the sites that name it; the parse is total (arguments are ordinary
  olang under the one grammar, and no token or reader macros will ever
  exist); expansion is pure, with the effectful and nondeterministic
  modules refusing at expansion time; expansion cannot hide
  (`olang expand`, embedded expanded source, site-attributed errors).
- The exchange contract: a meta fn receives argument and declaration
  *source text* and returns source text, validated by the parser before
  splicing; arguments are expanded before the macro runs (applicative
  order, to any depth); stacked decorators apply nearest-first; only
  names declared `meta fn` — locally or via `use` — are invocable.
- The `meta` helpers `eval`, `lit`, and `fresh`, with their documented
  signatures and semantics, and `meta.parse`'s pre-expansion view.
- Source-mapped errors as *behavior*: an error in untouched code points
  at its original line; an error in generated code points at its `@`
  site and names the macro.

Free to change, deliberately: exact error and diagnostic wording,
`olang expand`'s whitespace and presentation, the LSP's rendering of
mapped findings, and the expansion fuel — which may increase but never
drop below 16 rounds. The blocked-at-expansion module set may grow as
new effectful modules enter the language (an addition that keeps the
purity promise), and never shrinks.

These rules are part of the commitment above and are permanent under the
[semver contract](#versioning): changing any of them would be a 2.0, and
2.0 is the version that is not meant to happen. The language surface is
complete and closed; what [the roadmap](roadmap.md) still holds is
additive.

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
- **`runtime`** — the introspection module (memory, frames, the last
  error, `call_budget`, `load_module`): each documented function behaves
  as written; what the reports carry grows (a field added, never one
  taken away or renamed without a deprecated alias), and the numbers in
  them are measurements, not promises.
- **`asset`** and **`collections`** — `asset.read` and its siblings, and
  the auto-loaded `collections` (heap, deque, table, …), behave as
  documented; their sets grow. The collections' internal layouts are
  not part of the contract.
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

### Experimental — may change in a minor release

An Experimental surface may change in a **minor** release — a function,
option or field changed or removed — and every such change is recorded
in the CHANGELOG under Changed with what to change. A patch release
does not change one. Each graduates to a Stable tier by a CHANGELOG
entry saying so; nothing graduates silently.

- **Capabilities and the transparent binary.** The `[capabilities]`
  manifest, per-dependency attenuation, `--deny`, the `caps` module that
  reports a grant, `olang inspect`, and the bundle format a built binary
  carries. The model (opt-in restriction, shrink-only attenuation, gate
  at the effectful-module boundary, source-carrying binaries) is
  settled, but the gated surface may grow (new modules) and the bundle
  format may change. Enforcement runs on every tier, so a restricted
  run keeps full speed: the grant a program is given is independent of
  how fast its code happens to be running.
- **`http.serve`** — the request/response API (the `HttpRequest`
  fields, string and response-struct returns) is integration-tested; the
  execution model (a bounded worker pool, a blocking caller, listening
  on `127.0.0.1` unless the `bind` option names another address) and
  its options may still change.
- **The `dom` module** — browser-only, and young: the function surface
  may grow and its payload conventions change as frontend programs
  demand more; the element-handle model and the stateless pattern it
  supports are its core.
- **`gui`, `tty`, `pty` and `vt`** — the desktop and terminal engines
  Loom, Studio, Heddle and heddle-sql are built on. Their surface still
  grows with the engines (the roadmap's desktop rows), so a node's
  fields, an event's payload or an option may change in a minor release.
  The libraries built on them (Loom, Heddle) are where a program meets
  them, and they absorb those changes.

### Reserved — not accepted, and additive if they ever are

One type-level form is named in the grammar's design but rejected by
the parser: an intersection type annotation (`A & B`). Writing one is a
parse error, not a construct waiting for semantics, and if it is ever
accepted it will be additive, since no program can contain one now.
A union *alias*, `type Id = Int | String`, is not reserved: it parses
and is enforced as the union annotation it names (language.md, "Type
annotations"), as union and literal-type annotations have been since
0.50.


## Tools and file formats

Studio, CI scripts and editors drive the tools, so the tools have a
contract too, tiered as the language is:

| Surface | Tier | What holds |
|---|---|---|
| Command names and documented flags (`olang --help`, tooling.md) | Stable | A command or flag keeps its name and meaning; a renamed one keeps the old spelling as an alias. A new command may take a word an extensionless script used: run such a script as `olang ./name`. |
| Exit codes ([the table](tooling.md#exit-codes)) | Stable | `0`, `1`, `2`, `3` and `102` keep their meanings. |
| The JSON envelopes: `format`, `kind`, the line-delimited event names ([the documents](tooling.md#output-flags-and-json-documents)) | Stable | A document's `format` number changes only when a field changes meaning or goes away. |
| The JSON documents' fields | Stable, additive | Fields are added in minor releases without a new `format`; a consumer ignores fields it does not know. |
| `repl --serve`'s protocol | Stable, additive | Its `protocol` number changes only when a request or reply changes shape; new requests, fields and `features` are added under it. |
| `olang.toml` and `olang.lock` | Stable | The documented keys keep their meanings. The lock's `version` changes only when its meaning does, and an olang refuses a newer one by name. `[check] promote`'s class names are contract; `all` is every class the running olang knows. |
| `shelf.toml` | Stable | Its `version` is checked as the lock's is. |
| `.olt` traces | Stable | The trace format is versioned: a later olang reads an earlier trace, and refuses a newer one by name. |
| Public environment variables ([the table](tooling.md#environment-variables)) | Stable | The variables marked public keep their names and meanings. |
| `olang fmt`'s output | Stable within a minor | The formatter's output changes only in a minor release, and each change is recorded in the CHANGELOG, so `olang fmt --check` in CI changes verdict only when olang's minor version does. |
| The bundle format of `olang build`, the registry protocol, `olang profile`'s internals beyond its documented fields | Experimental | May change in a minor release, recorded in the CHANGELOG. |
| `~/.olang/state` (parse and compile caches, warm profiles), `meta.encode` images, diagnostic and error text, the internal environment variables | Not contract | May change in any release; a cache from another version is ignored, never misread. |

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
  documented example of a Stable surface is a 2.0 change, not a 1.x one,
  no matter how small. The intent is that 2.0 never needs to happen; it
  exists so the promise above can be absolute rather than hedged.

**Experimental surfaces are the exception, and the only one.** A
surface the [Experimental tier](#experimental--may-change-in-a-minor-release)
lists may change in a minor release, with a CHANGELOG entry under
Changed that names the surface and says what to change; its documented
examples change with it. A patch release changes none of them.

**What "breaking" means, precisely.** A change breaks compatibility if a
program written against this book — its syntax, its documented behavior,
its stdlib signatures and `Result` conventions — stops parsing, stops
running, or produces a different result. Error and diagnostic text is
not behavior in this sense ([above](#stable)), and neither is an
Experimental surface's change in a minor release. This is not a matter of judgment:
every documented example is executed in CI (`doc_examples_test`), so a
breaking change is a *failing test*, and shipping it in anything below a
major version is a release bug.

**Deprecation.** After 1.0, a stdlib function that has to be renamed
keeps its old name as an alias for at least one minor release, the
rename noted in its documentation and the CHANGELOG; the old name is
never removed in the release that renames it.

**Bug fixes are not breaking changes.** Behavior that changed because it
was a *bug* — `&&` not short-circuiting, `?` aborting instead of
propagating — is a fix, not a break, and lands in a minor or patch
release. Such changes are recorded in [CHANGELOG.md](../CHANGELOG.md)
under Fixed, with the reasoning, so the distinction between "we fixed
what was wrong" and "we changed what was right" is always on the record.

**Before the 1.0 tag.** Until 1.0.0 is tagged, a minor release may still
settle a rule, as 0.68 and 0.88 did — each such change listed in the
CHANGELOG and found by `olang check` where it can be. 0.88 is the last
release before 1.0: from 1.0 the contract above binds every release,
and the 1.0 release changes the number and the promise's *formality*,
not the code's behavior.

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

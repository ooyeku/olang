# Stability and compatibility

Part of [the olang book](README.md) · [Architecture and internals](internals.md)

This chapter is the authoritative statement of olang's capabilities. Where
the README, an old comment, or any other text disagrees with what is written
here and in the reference chapters, this document is correct.

olang's core language is now considered **stable in shape**: the syntax and
behavior documented in [the language reference](language.md) and
[the stdlib reference](stdlib.md) are a commitment, not a snapshot.
Development from here focuses on optimization, new features, and stability,
not on changing what already works.

## The commitment

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

## Stability tiers

### Stable

The core language: literals, variables and assignment, all operators and
their precedence, strings, lists/tuples/maps/objects, ranges, control flow,
pattern matching (all documented pattern kinds), functions (defaults, named
arguments, lambdas, closures-by-value, `return`), pipelines, `break value`,
structs, enums and their constructors (including cross-module), `error`
declarations, traits and impls, `Result` +
`?` + `try`/`catch`, modules (`use`/`share` in all documented forms),
`test` blocks, and the standard-library modules `str`, `col`, `math`,
`json`, `toml`, `csv`, `re`, `dates`, `time`, `base64`, `fs`, `os`, `db`,
`random`, `crypto`, `chan`, `proc`, `meta`, and the global builtins. The
embedded olang modules (`colx`, `mathx`) and packages (`cli`, `term`, `ui`,
`viz`, `dash`) follow the same append-mostly rule.

**Scope and mutability settled in 0.61.0**, as the second and final
deliberate breaking change before 1.0 (recorded in the CHANGELOG with a
migration guide). Three rules that were advisory warnings in 0.50–0.60
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

These rules are now part of the commitment above and will not change
again.

### Stable in behavior, evolving in scope

- **Async and concurrency** — the documented API (`async`/`await`,
  `Promise.resolve/reject/delay/all/race`, `spawn`, `par_map`,
  `par_filter`, `par for`) is stable. `spawn` runs on a real OS thread;
  `par_map`/`par_filter` carry spawn's snapshot semantics and are
  differential-tested against `map`/`filter`; `Promise.delay` keeps its
  deterministic deadline model. The scheduling model may gain further
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

- **Capabilities and the transparent binary.** The `[capabilities]`
  manifest, per-dependency attenuation, `--deny`, and `olang inspect`
  are new: the model (opt-in restriction, shrink-only attenuation,
  gate at the effectful-module boundary, source-carrying binaries) is
  settled, but the gated surface may grow (new modules), the bundle
  format may gain fields, and capability enforcement currently runs on
  the interpreter tier — a capability-restricted run forgoes the
  bytecode tier for sound attribution. Unrestricted runs are
  unaffected; a program with no manifest keeps full capability and full
  speed.

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

### Reserved — parses today, semantics later

- Intersection (`A & B`) type annotations
- Union type *declarations* (`type X = A | B`) are not yet accepted —
  union and literal-type *annotations* gained semantics in 0.50 and
  are covered by the gradual-typing bullet above

Reserved constructs are safe to avoid entirely; when they gain semantics it
will be additive.

## Versioning

olang follows [semver](https://semver.org) with a pre-1.0 mapping: **minor
versions** (0.25 → 0.26) may add features and fix bugs whose old behavior
was undocumented or wrong; **patch versions** are fixes only. Anything that
would break a documented example is deferred to a hypothetical 1.0 — and
the intent is that nothing needs to.

Behavior that changed because it was a *bug* (e.g. `&&` not
short-circuiting, `?` aborting instead of propagating) is recorded in
[CHANGELOG.md](../CHANGELOG.md) under Fixed, with the reasoning.

## How this is enforced

- Every code block in the book runs in CI (`doc_examples_test`).
- Every fixed bug lands with a regression test (one file per area under
  `tests/`).
- The examples — real programs — run end-to-end via
  `examples/run_all.ol` and `example_programs_test`.
- Tier agreement is tested directly (`bytecode_tier_test` and friends), and
  `colx`/`mathx` differential tests exercise the language against its own
  stdlib.

If you find documented behavior that does not match the implementation,
that is a bug in one of them — please file it; the book is the contract.

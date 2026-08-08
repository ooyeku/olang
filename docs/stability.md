# Stability

olang's core language is now considered **stable in shape**: the syntax and
behavior documented in [the language reference](language.md) and
[the stdlib reference](stdlib.md) are a commitment, not a snapshot.
Development from here focuses on **optimization, new features, and
stability** — not on changing what already works.

Part of [the olang book](README.md) · [Internals](internals.md)

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
4. **The two execution tiers agree.** Any observable difference between
   `--no-ovm` and the default tiered execution is a bug. Optimization work
   must be invisible.

## Stability tiers

### Stable

The core language: literals, variables and assignment, all operators and
their precedence, strings, lists/tuples/maps/objects, ranges, control flow,
pattern matching (all documented pattern kinds), functions (defaults, named
arguments, lambdas, closures-by-value), pipelines, structs, enums and their
constructors (including cross-module), traits and impls, `Result` +
`?` + `try`/`catch`, modules (`use`/`share` in all documented forms),
`test` blocks, and the stdlib modules `str`, `col`, `math`, `json`, `csv`,
`re`, `dates`, `base64`, `fs`, `os`, `db`, `random`, `crypto`, and the
global builtins.

### Stable in behavior, evolving in scope

- **Async and concurrency** — the documented API (`async`/`await`,
  `Promise.resolve/reject/delay/all/race`, `spawn`) is stable; the
  *scheduling model* (cooperative, deadline-based, single-threaded) may
  gain capability (e.g. real parallelism) without changing what existing
  programs observe.
- **The OVM tier** — which functions get promoted, and how fast they run,
  changes freely; results never do.
- **Type annotations** — all documented annotation forms keep parsing. A
  future static checker will be **opt-in** when introduced; annotations
  will not start rejecting today's running programs by default.

### Experimental — may change or be completed

- `http.serve` (the embedded HTTP server)
- `testing.run_test` (closure-based test execution)
- The `--enable-parallel` / `set_parallel` evaluation modes
- Assignment to an undeclared name (`x = 1` without `let`) currently
  creates a binding; prefer `let` — a future release may warn here.

### Reserved — parses today, semantics later

- `error Name { ... }` declarations (structured error types)
- Union (`A | B`) and intersection type *annotations*; union type
  *declarations* are not yet accepted
- Literal types in annotations

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

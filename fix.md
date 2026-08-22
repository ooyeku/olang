# fix.md — the last-mile branch

**Branch:** `last-mile`
**Goal:** resolve every nagging issue surfaced by the 2026-08-22 language review.
This file is the **source of truth** for what gets fixed on this branch: each item
below is checked off only when implemented, tested, and reflected in the docs.

CI re-enablement is explicitly **out of scope** — it is handled on a separate
branch.

Two design decisions were made up front and are recorded here so the work
doesn't relitigate them:

- **Absence model: Unit is absence.** The single lookup rule is: *lookups
  return the value or `Unit`; parsers and I/O return `Result`; a wrongly-made
  call raises.* No `Option` type, no `Result`-returning `map_get`.
- **Stdlib scope: all three sub-projects land** — HTTP client parameters, a
  `Bytes` value kind, and a native `Date` value.

---

## 1. Remove the interpreter "memory allocation limit" — [x] done

**Problem.** `src/interpreter/mod.rs` carries a `track_allocation` /
`force_memory_cleanup` / `reset_memory_tracking` apparatus that counts
"allocations" and aborts with *"Memory allocation limit (10000) exceeded …
This prevents memory corruption"*. It prevents nothing (the runtime is safe
Rust); what it actually does is kill the **documented idiomatic append**
(`xs = xs + [i]`) at exactly 10,000 iterations when the loop runs at top
level, while the identical loop inside a function is JIT-compiled and
finishes 20k iterations in milliseconds. Documented programs fail depending
on where a loop sits.

**Fix.**
- Delete the tracking fields, the three methods, and every call site.
- Delete the module-cache-clearing "cleanup" logic with it (its own comment
  documents a bug it caused during module loads).
- Regression test: a top-level `for` loop appending 20,000 elements with
  `+` runs to completion on both `--no-ovm` and default tiers.

## 2. Fix operator precedence (deliberate breaking change) — [x] done

**Problem.** Three orderings are documented footguns, verified on the 0.67
binary:
- `&&` and `||` share one level: `true || false && false` → `false`.
- `|>` binds tighter than arithmetic: `2 + 1 |> f` → `2 + f(1)` — the
  flagship operator silently mis-parses its most natural uses.
- Ranges bind tighter than `+`/`-`: `0..n-1` is `(0..n) - 1`, a runtime
  error.

Stability is valuable in proportion to adoption; freezing these pre-1.0 buys
permanent warts for a guarantee nobody yet depends on. This is the last
moment the fix is possible.

**New table, loosest → tightest** (bitwise, unary, postfix unchanged).
One refinement over the original plan: `|>` sits between comparison and
ranges — Elixir's placement — rather than absolutely loosest, so
`xs |> len == 3` keeps meaning `(xs |> len) == 3` while arithmetic still
feeds the pipe:

| Level | Operators |
|---|---|
| 1 | `\|\|` |
| 2 | `&&` |
| 3 | `==` `!=` `<` `<=` `>` `>=` |
| 4 | `\|>` (pipeline) |
| 5 | `..` `..=` |
| 6 | `+` `-` |
| 7 | `*` `/` `%` |
| 8 | `&` `\|` `^` `<<` `>>` |
| 9 | unary `-` `!` |
| 10 | call / `.field` / `[i]` / `?` |

After the change: `x + 1 |> f` pipes the sum; `a || b && c` is
`a || (b && c)`; `0..n-1` is `0..(n-1)`.

**Fix.**
- `grammar.pest` expression hierarchy + the mirroring parser code.
- Update `docs/language.md` (operator table + prose), `docs/pitfalls.md`
  (the two precedence entries become historical notes), and any doc example
  relying on the old parse.
- CHANGELOG entry with a migration note (the third and final deliberate
  pre-1.0 breaking change). Well-parenthesized code — which the old docs
  demanded — is unaffected.
- Tests: parser precedence tests updated; new cases pinning the three
  motivating examples; full doc-example + tier-agreement suites green.

## 3. One effect classification, three consumers — [x] done

**Problem.** "What is effectful" is curated in three independent lists that
already disagree: `caps::required` (capabilities), the macro-expansion
denylist in `Interpreter::expansion_denial`, and `Timeline::is_recorded`
(record/replay). Consequences found in review:
- A `meta fn` can call `dates.now`, `crypto.random_*`, and `ods`
  file I/O at expansion time — breaking macro Law 4 (pure expansion) and
  the replay guarantee.
- A `--rules` lint file escapes its all-denied capability sandbox via a
  `meta fn` performing I/O during expansion (expansion runs before the cap
  table is installed).
- `par for` is not gated in meta mode even though `spawn` and
  `par_map`/`par_filter` are, contradicting `docs/macros.md`.

**Fix.**
- New module `src/effects.rs`: one static classification table mapping
  builtin names/prefixes to effect classes (fs-read, fs-write, net, proc,
  env, clock, random, thread, stdin, exit). Documented as the single
  authority.
- `caps.rs`, `expansion_denial`, and `timeline.rs` consume it (each keeps
  its own policy — e.g. replay records clocks but not fs-writes — but the
  *classification* comes from one place).
- Meta mode denies every effect class plus threads; add the missing
  `Expr::ParForLoop` gate.
- Also fold in: `olang expand` resolves macro libraries against the file's
  directory like `run`/`check`/LSP do (base-dir inconsistency found in
  review).
- Tests: a table-driven test asserting the three consumers agree wherever
  they overlap; regression tests for the `--rules` escape, meta-mode
  `dates.now`/`ods.write_csv`/`par for`, and `olang expand` from a
  different CWD.

## 4. One absence story — [x] done

**Problem.** Four conventions for "not there": `map_get` → Unit,
`str.index_of` → `-1`, `str.parse_int` → Result, `min([])` raises. The `-1`
sentinel is actively dangerous because olang has negative indexing —
`xs[str.index_of(s, "z")]` on a miss silently reads the *last* element.
And the #1 documented pitfall (`unwrap_or(map_get(...))` being a type
error) has no ergonomic answer.

**Decision: Unit is absence.** Lookups return the value or `Unit`; parsers
and I/O return `Result`; misuse raises.

**Fix.**
- `str.index_of` returns `Unit` when absent (breaking; CHANGELOG migration
  note: replace `idx >= 0` with `idx != ()`). Same for any other `-1`
  sentinel lookups found in audit (`str.last_index_of` etc. if present).
- **Discovered during implementation and folded in:** `x == ()` /
  `x != ()` is now *total* — it answers (false/true) for every value
  instead of raising when `x` is present. Without this the documented
  absence test raised the moment a lookup succeeded (`Int != ()` was a
  cross-kind type error). Implemented identically in the interpreter and
  the bytecode tier; the JIT declines Unit comparisons and falls back.
  Ordering against Unit, and equality between two present-but-unrelated
  kinds, still raise.
- New `map_get_or(m, k, default)` global (append-only), so the
  missing-key-with-default idiom is one call.
- Docs: state the single rule in `docs/stdlib.md` conventions section;
  rewrite the pitfalls entry to teach `map_get_or`; sweep doc examples
  using `str.index_of(...) == -1`.
- Tests: stdlib-conventions test extended to pin the lookup rule.

## 5. Stdlib structural gaps: HTTP client, Bytes, Date — [x] done

### 5a. HTTP client parameters — [x] done
`http.get/post/put/delete/request` accept an optional trailing options map:
`#{ "headers": #{...}, "timeout_ms": Int, "bearer": String, "basic": (user, pass) }`,
with unknown option keys raising (a typo'd option must not become a
request that quietly lacked its auth header). The existing `HttpResponse`
struct gains a `headers` field (lowercased names, first value wins) — no
separate `http.fetch` needed, since the response struct already existed.
Existing call shapes keep working unchanged. Capability class stays `net`.

### 5b. `Bytes` value kind — [x] done
- Implemented as a **native value** (`NativeObject`, like Frames and
  cells) rather than a new `Value` variant — same user-visible
  semantics, no changes to the core enum or the tiers: immutable,
  structural equality, `typeof` → `Bytes`, `len(b)` works (new `length`
  hook on `NativeObject`, mirrored in the VM's `len` fast path), `b[i]`
  yields an Int (negative from the end), display is a capped hex
  preview.
- `bytes` module: `from_list`, `to_list`, `from_string`, `to_string`
  (UTF-8, Result), `len`, `slice`, `concat`.
- `fs.read_bytes` / `fs.write_bytes` (capability-gated like their text
  twins, via the unified effects table); `base64.encode*` and the
  `crypto` hashes accept Bytes; `base64.decode_bytes` decodes to it.
  `crypto.random_bytes` keeps its recorded string return (changing it
  would break both compatibility and trace replay); a Bytes-returning
  variant can land later, append-only.
- Tier story: a native handle crosses the tier boundary as an Arc
  refcount bump (the existing `ods` mechanism) — no conversion, no
  divergence surface.

### 5c. Native `Date` value — [x] done
- `Date` as a native value (NativeObject) wrapping a chrono
  `NaiveDate`: `typeof` → `Date`, structural equality and ordering,
  display as ISO `YYYY-MM-DD`.
- `dates` module reworked on top: `dates.date(y, m, d)`,
  `dates.parse(s)` → Result, `dates.today()` (clock; recorded for
  replay), `add_days`/`add_months`/`diff_days`/`year`/`month`/`day`/
  `weekday`/`format` operating on Date values. String-taking forms keep
  working (parse-at-the-boundary) for compatibility.
- While here: fix the module's convention violation — wrong arity /
  wrong argument type **raises** (rule 3) instead of returning `Err`.
- The strict-vs-lenient parse disagreement (`add_days` vs `year`) is
  resolved by the shared Date parse path.

## 6. Make the correctness claims reproducible — [x] done

### 6a. Commit the fuzzer generators — [x] done
The stability chapter cites two 10,000-program fuzzing campaigns (macro
corpus; tier-differential) whose generators are not in the repository, so
the strongest correctness evidence is unreproducible.
- `tests/fuzz/` gains two seeded, deterministic generators
  (`macro_fuzz.rs`, `tier_fuzz.rs` — or a shared crate/bin) that generate
  programs, run them per the original campaigns (expansion determinism,
  tier agreement, clean refusal), and are wired as `#[ignore]`d heavy
  tests plus a small always-on smoke corpus (e.g. 100 seeds) so CI (on its
  own branch) exercises the machinery.
- `docs/stability.md` updated to point at the committed generators.

### 6b. Record/replay refuses to lie under threads — [x] done
Worker clones get `timeline: None`, so a recorded/replayed program that
spawns tasks silently records/replays only the main thread — a "clean"
replay that proves nothing.
- When a timeline is attached and `spawn` / `par for` / `par_map` /
  `par_filter` executes: emit a loud one-time warning on stderr in record
  mode, and the same during replay (documented: the trace covers the main
  thread only).
- `docs/tooling.md` limitation section updated to match.

---

## Order of work

1 → 2 → 4 → 3 → 6 → 5 (small, well-bounded items first; the two new value
kinds last, each landed with the full suite green).

## Definition of done

- All checkboxes above ticked.
- `cargo test --workspace` green, including doc-example, tier-agreement,
  and differential suites.
- `examples/run_all.ol` passes.
- CHANGELOG records items 2 and 4 as deliberate breaking changes with
  migration notes.

## Verification record (2026-08-22)

- `cargo test --workspace --no-fail-fast`: **93 test targets, all
  green** (doc examples, tier agreement, differential, conventions,
  macro, openness suites included).
- `examples/run_all.ol`: **30/30 pass**, verified twice from a clean
  state (this surfaced and fixed a latent repo bug: meterflow depended
  on an untracked `cache/` directory).
- Both committed **10,000-seed fuzz campaigns run clean**
  (`macro_fuzz_full_campaign`, `tier_fuzz_full_campaign`).
- Hand-verified on both tiers: new precedence table, O(n) top-level
  append (200k appends < 100 ms), total Unit equality, Bytes and Date
  values, dates misuse raising, HTTP option validation, meta-mode
  refusal of `dates.now` / `crypto.random_*` / `ods` I/O / `par for`.

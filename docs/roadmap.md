# Roadmap

The plan of record. Item statuses progress from **planned** to **in
progress** to **landed**; nothing is removed, so the table is also the
history of what was decided. The campaigns that built the current
language — semantics, the data stack, capabilities on every tier, the
tier boundary, the engine additions — are complete; their record lives
in the CHANGELOG and this file's git history.

Version 1.0.0 ships when the project's owner decides it ships. Nothing
in this document is a countdown.

The workstreams below come from a deliberate audit for underdeveloped
ground (2026-08-26): each item cites what was actually observed, so
the work is tightening real edges, not speculation.

## W1 — text is made of graphemes

String operations index by codepoint, which corrupts what users see as
characters: `str.reverse("👋🏽ab")` tears the skin-tone modifier off its
emoji, `str.substring` and `str.char_at` cut clusters mid-character,
and `len` counts 👋🏽 as two. Decide the model (grapheme-aware default,
or documented codepoint semantics plus a grapheme API), implement it
uniformly across `len`/`str.*`, and pin it with tests over the unicode
segmentation cases.

| Item | Observed | Status |
|---|---|---|
| Grapheme decision + implementation | `str.reverse` corrupts emoji; `substring`/`char_at` split clusters | planned — direction: codepoints stay the documented indexing unit (O(1), lossless), a `str.graphemes` API (count/at/slice/reverse) serves the visible-character cases, and `str.reverse` keeps clusters whole; UAX #29 cases pinned |

## W2 — numbers read back in

The runtime prints floats the parser cannot read: `1e21` and
`9.999999999999999e-10` come out of `to_string`, while `1e20` in
source is a parse error — printed values are not valid literals.
Scientific-notation literals are simply missing from the grammar.
Alongside: float semantics are trapping (`1.0 / 0.0` and
`math.sqrt(-1.0)` error rather than produce inf/NaN) — a defensible
stance that must be stated as the contract, and JSON silently converts
big integers to floats (`99999999999999999999999999` → `1e26`).

| Item | Observed | Status |
|---|---|---|
| Scientific-notation literals | `1e20` fails to parse; printed floats aren't source | planned — full float literal syntax (`1e20`, `2.5e-3`, `1E+6`); print/parse round-trip pinned by proptest |
| Trapping-float contract documented | division by zero and `sqrt(-1)` trap by design | planned — language.md and stability.md state the no-NaN contract and the trap surface |
| JSON numeric fidelity | oversized ints silently become floats | planned — lossless within i64/f64, documented conversion at the edges, decide reject-vs-convert for oversized integers |

## W3 — errors that teach

The error surface is strong in places (index bounds, integer overflow
pointing at bigint) and embarrassing in others, all observed live:

| Item | Observed | Status |
|---|---|---|
| C-style block hint | `if x > 1 { ... }` → "expected an operator or a function call", no mention of `=>` | planned — brace-after-condition names the fix and shows the `=>` form |
| Unclosed-delimiter tracking | unclosed `[` reports "expected an operator" on the *next* line, never "opened at line N" | planned — every delimiter error names the opener's line and column |
| Module-not-found hint | `use geometry` → "This appears to be a system-level error" | planned — name the search that failed: spelling, `otc lib list`, embedded modules, `use lib.<name>` for local files |
| Non-function call | `x(1)` → "Cannot call non-function value" — no name, no type | planned — "'x' is an Int, not a function", with the binding site when known |

## W4 — the REPL under stress

An unbalanced `(` traps the session in continuation: every subsequent
line — including `:help` and commands — is swallowed into the buffer,
nothing indicates what is unbalanced, and only Ctrl-C escapes. The
session transcript that surfaced the bridge bug shows a user hitting
this six times in a row.

| Item | Observed | Status |
|---|---|---|
| Continuation escape + indicator | commands swallowed; no unbalance display; Ctrl-C the only exit | planned — continuation prompt shows the open delimiters, `:cancel` abandons the buffer, commands at the start of a continuation line warn instead of vanishing |
| Session-state differential harness | three session-poisoning bugs in one week (help cache, bridge landscape, meta.eval tier), all user-found | planned — generated action sequences replayed with every step asserted equal to the same step in a fresh session |

## W5 — tools that do what they claim

| Item | Observed | Status |
|---|---|---|
| `olang fmt` formats | `fn f( x ,y )=x+y` reported "all formatted (1 file scanned)" — the formatter normalizes nothing | planned — real normalization (spacing around `=`, `=>`, commas, operators), idempotent, semantics-preserving via a parse-identity gate, `--check` honest |
| LSP depth audit | feature-frozen for months; current depth unknown; the `:help` registry and doc pipeline make hover/signatures newly cheap | planned — audit first: diagnostics parity with `olang check`, hover from the 727-entry registry, `///` docs for user code |

## W6 — concurrency you can see into

| Item | Observed | Status |
|---|---|---|
| Blocked-channel visibility | `chan.recv` with no sender hangs the program forever, silently | planned — `chan.recv_timeout(c, ms)` for bounded waits; detect the all-threads-parked stall and abort with a report naming the blocked sites |
| Task/channel introspection | no way to ask what is running or blocked | planned — `task.list()` / channel state for the REPL and the profiler |
| Timeline reach | record/replay covers main-thread fs/net; db reads, channels, and workers run live and silently break the "clean replay is proof" property beyond a warning | planned |

## W7 — the engine keeps proving itself

| Item | Observed | Status |
|---|---|---|
| Runtime tier self-verification | tier agreement is proven by the suite, not by the running program; two "since it shipped" latent bugs this month argue for live evidence | planned — `--verify-tiers <rate>`: re-execute a sample of promoted calls on the interpreter, scream on divergence |
| HTTP server hardening | never adversarially probed: malformed requests, oversized payloads, slow clients | planned — abuse suite + explicit limits |
| Platform coverage | every gate runs on one macOS machine; `setup.sh` claims cross-platform | planned — at minimum, a documented Linux verification pass |

## Process

One lane at a time, each landing with its tests and documentation in
the same change. When an item's cost proves larger than expected, the
schedule moves, not the bar. New findings from use join the tables
with their evidence; nothing enters on speculation.

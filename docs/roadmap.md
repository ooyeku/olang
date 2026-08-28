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
| Scientific-notation literals | `1e20` fails to parse; printed floats aren't source | **landed** — full float literal syntax (`1e20`, `2.5e-3`, `1E+6`, underscores); print/parse round-trip pinned by a bit-for-bit proptest over arbitrary f64s |
| Trapping-float contract documented | division by zero and `sqrt(-1)` trap by design | **landed** — language.md and stability.md state the contract: would-be-`NaN` operations trap; the overflow edge below is recorded rather than papered over |
| Float overflow yields `inf` | `1e308 * 10.0` prints `inf`, and `inf - inf` then prints `NaN` — the trapping stance has a gap (found while documenting it) | planned — decide: trap on non-finite results from finite operands (a per-op check on all three tiers, with a JIT cost to measure), or promise `inf` propagation explicitly |
| JSON numeric fidelity | oversized ints silently become floats | **landed** — reject-not-convert: integers within i64 arrive exactly, decimals take the IEEE reading, an integer outside i64 (either sign, including u64-range) or an overflowing float text is an `Err` naming the value; serde's arbitrary_precision keeps the source text so the JSON grammar decides the reading |

## W3 — errors that teach

The error surface is strong in places (index bounds, integer overflow
pointing at bigint) and embarrassing in others, all observed live:

| Item | Observed | Status |
|---|---|---|
| C-style block hint | `if x > 1 { ... }` → "expected an operator or a function call", no mention of `=>` | **landed** — brace-after-condition suggests the exact rewrite (`if x > 1 => {`) and explains that only `if`/`else` take the arrow |
| Unclosed-delimiter tracking | unclosed `[` reports "expected an operator" on the *next* line, never "opened at line N" | **landed** — a string- and comment-aware scan names the opener: "Unclosed `(` opened at line 2, column 13" |
| Module-not-found hint | `use geometry` → "This appears to be a system-level error" | **landed** — the REPL now shows the full search (paths, shelf, near-miss suggestions incl. shelf libraries) and the help names `use lib.<name>` and `otc lib list` |
| Non-function call | `x(1)` → "Cannot call non-function value" — no name, no type | **landed** — "'x' is an Int, not a function"; Maps/Lists get an indexing hint, everything else a shadowing hint |

## W4 — the REPL under stress

An unbalanced `(` traps the session in continuation: every subsequent
line — including `:help` and commands — is swallowed into the buffer,
nothing indicates what is unbalanced, and only Ctrl-C escapes. The
session transcript that surfaced the bridge bug shows a user hitting
this six times in a row.

| Item | Observed | Status |
|---|---|---|
| Continuation escape + indicator | commands swallowed; no unbalance display; Ctrl-C the only exit | **landed** — the continuation prompt wears what is open (`(( ...> `), balancing the input evaluates it immediately (no `:end` needed), `:cancel` abandons, commands and `quit` mid-continuation warn and name the unclosed delimiter instead of vanishing; `:ml` now actually exists (deliberate multi-statement entry, `:end` to run) and `:ml`/`:end`/`:cancel` have true `:help` entries |
| Session-state differential harness | three session-poisoning bugs in one week (help cache, bridge landscape, meta.eval tier), all user-found | **landed** — seeded generated action sequences (redefine-after-promotion, hot loops, meta.eval, collections) driven through the real binary and pinned two ways: every step equal under the tiered REPL and the `--no-ovm` interpreter oracle, and every step equal on fresh replay of its prefix. First run caught a real one: maps/structs printed in per-process hash order — Display and the REPL colorizer now print sorted by key, the language's canonical order |

## W5 — tools that do what they claim

| Item | Observed | Status |
|---|---|---|
| Test blocks are not inert | building the web SDK: `use` of a module (and any bundled/spliced program) *executed* its test blocks as ordinary statements — a loaded module's tests printed, mutated, and ran their asserts at import time | **landed** — test blocks parse everywhere, execute only under `olang test`; pinned with a runner-still-runs-them check |
| Re-exports lose their closures | web-sdk's index re-exported `action` from a module with private cells; the re-closing pass rebound it over the *aggregator's* scope — "Undefined variable: mount_actions" at first call | **landed** — `share use` re-exports keep the closure their own module gave them; pinned |
| Test asserts inside match arms | `assert_eq` is "Undefined variable" inside a match arm within a `test` block (minimal repro pinned); tests route around it by extracting first | **landed** — the five asserts are builtins with the statement form's exact raising semantics, so expression positions (match arms, lambdas) resolve them |
| `olang fmt` formats | `fn f( x ,y )=x+y` reported "all formatted (1 file scanned)" — the formatter normalizes nothing | **landed** — token respacer calibrated against the whole in-repo corpus (the shipped style is the spec); alignment before `=>` and trailing comments preserved; raw-parse identity gate (span-insensitive decl equality fixed the gate refusing any line shift); all 127 corpus files formatted and idempotent |
| LSP depth audit | feature-frozen for months; current depth unknown; the `:help` registry and doc pipeline make hover/signatures newly cheap | **landed** — protocol-level audit, then the robustness batch: user-decl hovers with /// docs, `share` declarations first-class, inlay hints, code actions (doc stub, `let mut` quick fix), workspace symbols, `use` completions; diagnostics gained the scoping pass (assignment-to-immutable now surfaces) and stopped flagging stdlib modules. Grammar + Zed highlighting rebuilt for a distinct olang identity (112-file corpus, zero parse errors); extension 0.3.0 |

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

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
| Grapheme decision + implementation | `str.reverse` corrupts emoji; `substring`/`char_at` split clusters | **landed** — codepoints stay the documented indexing unit (stated in language.md's "Codepoints and graphemes"); `str.graphemes(s)` is the visible-character view (UAX #29 extended clusters as a list, composing with len/index/slice/join); `str.reverse` reverses by grapheme so skin tones, ZWJ families, accents, and flags stay whole; pinned by tests/graphemes_test.rs (ZWJ, modifiers, combining marks, regional indicators, Hangul jamo, CRLF) |

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
| Int/Int series division truncates silently | building examples/data-processing/climate: OWID GDP columns infer as Int, so `(gdp - gdp_then) / gdp_then` on Series truncated every growth ratio to 0 — "54 countries decoupled" first read "0 countries", with no error and no hint. Consistent with the scalar language (`7 / 2` is `3`), which is exactly why it hides: in data work ratios are the norm and a truncating `/` between measure columns is almost never what the analysis means | planned — decide: promote Series Int/Int `/` to Float (NumPy's reading; diverges from scalar `/`), or keep truncation and make `ods.cast`-before-divide the taught idiom with a check-time warning when a Series division discards a nonzero remainder. Whichever way: the choice must be stated in the ods chapter, and the climate example is the regression case |
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
| Blocked-channel visibility | `chan.recv` with no sender hangs the program forever, silently | shipped — the stall detector: every olang thread is censused, unbounded waits are parked sites, and an all-parked program aborts with a report naming each blocked site (`OLANG_STALL_ABORT=0` opts out) |
| Task/channel introspection | no way to ask what is running or blocked | shipped — `task.list()`, `task.parked()`, `chan.stat(c)` |
| Timeline reach | record/replay covers main-thread fs/net; db reads, channels, and workers run live and silently break the "clean replay is proof" property beyond a warning | planned |

## W7 — the engine keeps proving itself

| Item | Observed | Status |
|---|---|---|
| Runtime tier self-verification | tier agreement is proven by the suite, not by the running program; two "since it shipped" latent bugs this month argue for live evidence | shipped — `--verify-tiers <rate>` re-executes sampled native results (compiled calls and OSR regions, pure by whitelist construction) on the VM and aborts with a report on divergence; `:ovm` counts the session's verified calls |
| HTTP server hardening | never adversarially probed: malformed requests, oversized payloads, slow clients | shipped — explicit `serve` limits (`max_header_bytes`, `max_body_bytes`, `request_timeout_ms`), precise 400/408/413/431 answers, header-injection flattening; `tests/http_abuse_test.rs` is the gauntlet |
| Platform coverage | every gate runs on one macOS machine; `setup.sh` claims cross-platform | shipped — `dist/linux-verify.sh` runs the full suite and the examples harness in a Linux container (RELEASING.md gate list); architecture follows the host |

## W8 — what building the crimes workstream asked for

Every row below was observed while building
`examples/data-processing/crimes/` into a fourteen-stage,
three-model analysis (2026-08-30) — the largest olang program yet
written, and a deliberate audit of the data-science surface by use.

| Item | Observed | Status |
|---|---|---|
| Multi-series plotting | the calibration diagram needs its points *and* a y=x reference; the three models' F1-threshold curves belong on one chart with a legend; rates-with-intervals want grouped bars. None expressible — every `plot.*` chart is a single series, so cross-model comparisons live only in tables | planned — a multi-series form (shared axes, legend) and a histogram primitive; the crimes charts are the acceptance cases |
| A vector/matrix layer for numeric code | every kernel in `lib/ml.ol` — dot products, axpy updates, covariance, standardization — is a hand-written index loop, and the feature matrix is a list-of-columns convention each function re-documents; extraction from a frame repeats `ods.to_list(ods.cast(...))` per column | planned — `vec.dot/scale/add/sum` over Float lists (dedicated JIT targets), and `ods.to_matrix(frame, cols)` blessing the columns convention once |
| Error spans through promoted code | a division-by-zero inside `tree_train` reported its span as `main.ol:405 println("")` — the call stack named the right function but the wrong site, and finding the failing expression meant rebuilding the function inline with prints | shipped — the root cause was the emitter leaking one function's span rows into the next compile (a dependency inherited its caller's table, so errors pointed at another function's lines); fixed along with three trace losses found by the new suite: hot map/filter kernels dropped their trace entirely, bridged-lambda errors lost location crossing the boundary, and the interpreter itself recorded stacks after unwinding had popped the deep frames. tests/error_span_test.rs pins byte-identical reports across seven acceleration shapes |
| Template-literal ergonomics | backtick strings neither process escapes nor nest, and both facts surface only at runtime: progress bars printed a literal `\r` per frame, and report assembly needed four rounds of hoisting `let`s out of `${...}` | shipped — ruled: the semantics stay (templates process no escapes, do not nest), and the mistakes are caught early instead. `olang check` warns on an escape-looking `\n`/`\t`/`\r` inside a template, positioned at the exact backslash and read from the source spelling (`\\n` — the same two output characters — is the deliberate form and stays silent; escapes inside a `${...}`'s double-quoted strings are real syntax and exempt); the parse errors a backtick inside `${...}` produces now name the nesting rule and the bind-to-a-name fix. tests/template_lint_test.rs pins warnings, exemptions, both error paths, and runtime unchanged |
| Import shadowing | `use term` silently rebound `table` over the earlier `use lib.report { table }`; the failure surfaced at runtime as "len: argument must be a list" inside the wrong function | shipped — `use lib.report { table as md_table }` binds only the alias (share-use re-exports under it too; `as` stays contextual), and `olang check` warns on a wildcard `use` that shadows an earlier explicit import, naming both lines and the two fixes; tests/import_alias_test.rs pins runtime and warning both |
| `map_get` on a missing key | returns Unit, not a Result — `unwrap_or(map_get(q, "v"), d)` raises instead of defaulting, and the guard idiom (`map_has_key` first) must be known in advance | shipped — ruled: `map_get` keeps its shape and is documented as the raw read; `map_get_or(m, k, default)` (in the language since 0.69) is the blessed lookup-with-default, now beside `map_get` in the language reference. No Option type; absence is one rule — a missing key and a stored Unit both take the default |
| Derived group keys and splits | the year-month series needed a two-key `group_by` plus two `sort_by` calls; the train/test split is a hand-written every-5th-row scheme, and the report's own limitations section names the chronological leakage a forward-in-time split would fix | planned — a date-truncation/derived-key helper for `group_by`, and split utilities (deterministic holdout, forward-in-time) in ods or a stdlib `ml` module. The Int/Int series division row in W2 belongs to this same cluster |
| Optional parameters | `logistic_train_with`/`kmeans2_with` exist solely because there are no default arguments — every progress-aware function pays a duplicate-signature tax and the original name becomes a delegating stub | shipped — defaults and named arguments already parsed and half-worked; the row became three fixes: defaults now evaluate in the callee's scope with earlier parameters bound (they leaked the caller's environment — a live dynamic-scoping bug), defaulted functions promote and compile (they were blanket-rejected from the tier: a hot defaulted call was 650× slower; compiled call sites splice literal defaults, others take the interpreter-exact value path), and the semantics are documented with an eight-way differential battery. The `_with` stubs in crimes/lib/ml.ol are gone |
| Sole-owner map writes | the xlang wordfreq benchmark: `m = map_set(m, k, v)` clones the whole map per insert on both tiers even when the move fusion makes the argument sole-owner, so hot string-keyed counting is quadratic — the one benchmark olang cannot finish; `collections.table` is no rescue once the calling loop promotes (every put crosses the tier boundary converting the handle, the C7 T2 lane) | shipped — the fused `MapSetAssign` with the list writes' sole-owner discipline, VM and interpreter both, shadow-exact (compile-time decline for locals and known user functions, run-time re-check for late shadows); wordfreq DNF → 1.19 s. JIT lowering via a map ownership family remains available if profiles ever demand it |
| Approximate test assertions | the estimator test blocks reduce to `assert_eq(x > 0.5, true)` because there is no tolerance-based comparison — numeric tests are both weaker and less readable than they should be | shipped — `assert_close(actual, expected, tolerance, message?)` beside the existing asserts, everywhere-an-expression like them, Int/Float mixing, NaN fails, negative tolerance errors; the crimes estimator tests adopted it and tests/assert_close_test.rs pins the differential |

## Process

One lane at a time, each landing with its tests and documentation in
the same change. When an item's cost proves larger than expected, the
schedule moves, not the bar. New findings from use join the tables
with their evidence; nothing enters on speculation.

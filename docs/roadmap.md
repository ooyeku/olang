# Roadmap

The plan of record: what olang still has to do, and nothing else. An
item leaves this file in the change that lands it; what landed, and
why, is in the CHANGELOG and in this file's git history (the full
record of W1–W24, H0, L0 and the applications' readings is the
version of this file before 2026-10-09).

Version 1.0.0 ships when the project's owner decides it ships. Nothing
in this document is a countdown.

Every row was observed, not imagined: each cites the program that ran
into it and what was measured, re-checked against 0.87.0 on
2026-10-09. Tags name the surface: `[olang]` the language and
runtime, `[stdlib]` builtins and bundled packages, `[gui]` the desktop
engine, `[tooling]` the checker, LSP, REPL and test runner, `[otc]`
the project tool, `[web-sdk]` the web framework.

Off-macOS work is parked (macOS is the platform verified and polished
since 2026-10-07); those rows are kept together under
[Platforms](#platforms-parked) so they are not lost.

## Contents

- [E — Ergonomics: records and maps](#e--ergonomics-records-and-maps)
- [Language and runtime](#language-and-runtime)
- [Memory and start-up](#memory-and-start-up)
- [Tooling](#tooling)
- [Standard library](#standard-library)
- [The desktop engine (`gui`)](#the-desktop-engine-gui)
- [Web SDK](#web-sdk)
- [Platforms (parked)](#platforms-parked)
- [Process](#process)

## E — Ergonomics: records and maps

From a review of the ~110,000 lines of olang in this workspace
(Studio, open-track, Loom, Heddle, Shuttle, heddle-sql, open-track
desktop; 2026-10-09). The core reads well — expression `if`/`match`,
pipelines, `(model, effects)` tuples — but the data a program carries
is almost always a string-keyed map, and that is where the friction
is: 17,763 `map_get(` calls (one every six lines), 11,452
`#{ "key": … }` literals, 4,114 `map_set`/`map_merge` calls, and six
`type` declarations in the whole corpus. A line from Studio's
profiler:

```olang no-run
let clearable = len(filter(map_get(p, "profiles"), (e) => map_get(e, "id") != live))
```

Objects (`{ a: 1 }`, read with `.`) already exist and `map_get`,
`map_set`, `map_keys`, `map_has_key`, `map_get_or` accept them, so the
short form is there — but the frameworks standardised on `#{ "k": … }`
because objects, maps and parsed JSON objects are three kinds that the
builtins do not treat alike, and nothing makes updating one short.
The rows are in the order to take them; each is additive, so all fit
the stability contract.

| Item | Evidence | Done when |
|---|---|---|
| `[olang]` One record kind for the builtins | `map_len` and `map_merge` refuse an object and a parsed `JsonObject` ("argument must be a map", "first argument must be a map") while `map_get`/`map_set`/`map_keys` accept both — re-checked on 0.87.0. open-track desktop writes every server-record merge by hand; Loom's props stay maps because 39 `map_merge` and 10 `map_len` calls in loom/heddle/shuttle `lib/` would refuse an object | every `map_*` builtin accepts a Map, an Object and a JsonObject alike, on every tier, and stdlib.md's map table says so; Loom, Heddle and Shuttle can take `{ width: "fill" }` props |
| `[olang]` Spread in map and object literals | `#{ ...m, "a": 5 }` and `{ ...o, b: 2 }` are parse errors, and the error is wrong: "Unclosed `{` opened at line 6" for a brace that is closed. Updating one key of a model is `map_set(m, "k", v)`; two keys nest two calls | `#{ ...m, "k": v }` and `{ ...o, k: v }` build a copy with the keys replaced (later wins), on every tier; until then the parser names the unsupported spread instead of an unclosed brace |
| `[stdlib]` Nested reads and updates | the models are maps of maps (`map_get(map_get(m, "pp"), "cur")`, then `map_set(m, "pp", map_set(map_get(m, "pp"), "cur", x))`); `map_get_in`, `map_set_in` and `map_update` do not exist | `map_get_in(m, path)`, `map_set_in(m, path, v)` and `map_update(m, k, f)` (sole-owner in place, as `map_set` is), for maps and objects |
| `[olang]` Reading a map's key with `.` — reopen the ruling | language.md "Anonymous objects" rules that maps are read with `map_get` only, and Heddle logged the cost (a message's fields read with `map_get` everywhere). After the three rows above, decide between (a) `m.key` on a map for identifier keys, a missing key raising as an object's does, with `map_get` kept as the raw read; or (b) leaving maps alone and moving the frameworks' props, models and messages to objects. Recommended: (b) first — it needs no new semantics — and (a) only if the corpus still reads `map_get(m, "literal")` more than it reads `.` | the ruling written into language.md, and the frameworks' examples written in the chosen form |
| `[tooling]` The checker sees record shapes | a misspelt key is `()` at run time (`map_get(p, "runing")`), and neither `olang check` nor the language server can see inside string keys, so the most common data shape gets the least checking. An object's missing field already raises at run time | `olang check` reports a read of a key that no construction of that value can have (literal-keyed maps and objects, through `let` and parameters within a module), with no false report over the workspace's corpus |

## Language and runtime

| Item | Evidence | Done when |
|---|---|---|
| `[olang]` A raise primitive | a library that must stop with a message has `unwrap(Err(t))` (prefixed and quoted) or `assert(false, t)` (reads as a failed assertion); Heddle's `refuse` (heddle/lib/message.ol) is the workaround. language.md's error model rules out catching, not raising | `raise(text)`: the message intact through `attempt`, printed as a plain runtime error when uncaught |
| `[olang]` A lambda stage in a pipeline takes the stages after it | `5 \|> (v) => if v > 3 => v else => 0 \|> g` applies `g` only to the else branch (answers `5`; with `1`, `[0]`), and `olang check` is clean. Shuttle and Foundry carry the rule "write `let`s, not a lambda stage" in their CLAUDE.md; language.md's Pipelines section never says what an unparenthesized lambda does | ruled: either `\|>` ends an unparenthesized lambda body, or the rule is stated in language.md and the checker warns on a bare lambda stage followed by `\|>` (pinned) |
| `[olang]` `sort` and `<` on tuples | `sort([(2, 1), (1, 2)])` raises "cannot compare values of type Tuple" though `==` compares tuples (Heddle; otc-hub's collections sorts by key functions instead) | tuples order lexicographically under `<` and `sort`, on every tier |
| `[olang]` The bytecode tier refuses bitwise operators and `break value`, with an opaque reason | `fn bits(a, b) = (a & b) \| 4` stays on the tree-walker: "an expression the compiler has no form for (Discriminant(38))"; `let r = loop { break 5 }` the same with Discriminant(29). Heddle packs text attributes as bits and tests them with division and remainder instead (heddle/lib/theme.ol). `return` compiles since 00a1332 | `& \| ^ << >>` and `break value` compile on the VM (and natively), and every refusal names the construct rather than an enum discriminant |
| `[olang]` A list held in a map is copied on every append | `map_set(m, "log", map_get(m, "log") + [line])` copies the list: 10,000 appends 110 ms, 40,000 1,993 ms — quadratic. `cell.take` and the builtin moves (L1) do not reach a value read out of a map. Heddle keeps its log in 256-line chunks (heddle/lib/components/log.ol) | appending to a list held in a map is amortised O(1) (a move-out such as `map_take`, or a persistent vector), pinned by a scaling test |
| `[olang]` A closure over a large local pays per call on the bytecode tier | otc-hub/bench/04_capture_matrix.ol: `map` over 20,000 indices with `(i) => xs[i]`, `xs` a local 100k list, costs 23,550 ns an element against 50 for a parameter or global capture; 2,000 calls take 47 ms compiled, 0 ms under `--no-ovm`. Closure *creation* was fixed in W22; this is the call | a captured local is shared, not copied per call; 04's local-capture rows within 2× of the parameter rows |
| `[olang]` `take` and `reverse` deep-copy on the bytecode tier | otc-hub/bench/06_copying.ol, twelve 2,047-node trees: `take(xs, 11)` 2,488 µs and `reverse(xs)` 2,722 µs an op against ≤ 1 µs for `skip`, `filter`, `+`, `map`; 0 ms under `--no-ovm`. otc-hub's collections works around it with `vector.slice`/`prefix`/`reversed` | `take` and `reverse` share elements on the VM as `skip` does; 06's rows pinned |
| `[olang]` A module passed as a value falls back to the tree-walker | after builtins crossed the boundary (7d76560), the tiers fixture's remaining surprise is `apply(str, "iii")`; tooling.md documents it with "pass the data instead" (Studio) | a module value crosses the tier boundary as a builtin does, or tooling.md states it as a ruling |
| `[olang]` An interrupted promoted loop keeps the values from its promotion | `let mut k = 0; while true { k = k + 1 }` in `repl --serve`, interrupted: `k` reads 512, the count at promotion; tooling.md states this as current behaviour (Studio's REPL panel) | the loop's live variables are written back on every exit from the promoted remainder, interrupt and error included; the caveat leaves tooling.md |
| `[olang]` A lambda compiled once per body | open-track compiles about 100 per-closure bytecode bodies a request for lambdas that capture data; a38daa3 shares compiles only between closures with equal cheap captures and evicts the rest | each lambda body compiles once with its captures as run-time inputs; no compile on a request's path. Low: the leak it caused is gone |
| `[olang]` A match on string literals is a linear chain | Studio's `sa_update` matches about 300 string arms; not shown to be a cost yet | a match of string-literal arms compiles to a table — only when a profile asks for it |

## Memory and start-up

| Item | Evidence | Done when |
|---|---|---|
| `[olang]` Every http worker compiles its own copy of the program | `runtime.memory()` shows each `olang-http-N` at 45–60 MB live once warm, against 13 MB for the whole process with the tier off: bytecode, constants and a bridge per VM. 18 default workers is about 1 GB, so open-track caps itself at 8 workers. open-track served is 330–445 MB resident against the 250 MB target (W21/W22: "not met"; the rest above the 97 MB heap is the small-object zones' freed pages) | immutable compiled code is shared across a process's threads (JIT module and inline caches per thread), per-worker live memory after warm-up is a few MB, and open-track served at 18 workers is under 250 MB resident |
| `[olang]` Session start under 100 ms for a large client | open-track's browser boot is 490 ms with no hot list; 360 ms of it is before the app's first statement and grows with program size, and nothing attributes it. The browser runtime is 0.97 MB brotli and cannot shrink further while the parser and compiler ship in it; a program image as bytecode was measured and left "a design for later" (W15 items 5–6) | first a boot profile that splits decode, declare, top level and first render for a real bundle; then what it names (the bytecode image, which would also let a browser profile drop the parser and compiler). Done: open-track's session start under 100 ms and the runtime under 400 KB |
| `[olang]` A program's start compiles code its first frame never runs | olang Studio in a real window: first frame at ~340 ms from the process's start (budget 300). Its modules now load from the parse cache (~160 ms) and a compile no longer copies the registries or retries once per callee, but compiling `update` still compiles every function it could reach (the tier promotes ~1,800 functions at the first boot; a few hundred run): a callee not yet compiled is compiled before its caller, eagerly, down the whole graph | a callee met unregistered gets an id at once and compiles at its first call (a call site patched to the id once resolved, the JIT unaffected), and a refusal then falls back to the bridge as today; Studio's real-window first frame under 300 ms with no keystroke budget worse |
| `[olang]` What holds live memory, by site | growth under load was undiagnosable from open-track; `runtime.memory()` answers by owner, and `OLANG_ALLOC_TRACE` gives only large allocations, only in an `alloc-count` build | live bytes by allocation site or value kind in a release binary (`runtime.heap()`, or a dump at a signal). Low now the leak (OL-146) is fixed |

## Tooling

| Item | Evidence | Done when |
|---|---|---|
| `[tooling]` The checker misses a builtin's arity | `str.thousands(1234)` (it takes two) passes `olang check` in one file and across modules, and fails at run time (Loom) | `olang check` reports a wrong argument count to any builtin or stdlib function from the signature table; pinned |
| `[tooling]` The checker misses a parameter that shadows a stdlib module | `fn table_sort(rows, col, …)` checked clean and failed on `col.sort_by`; `fn f(str) = str.length("a")` is clean on 0.87.0. The `let` form already warns (Heddle) | the "shadows the stdlib module" advisory covers parameters and pattern bindings |
| `[tooling]` A large file's problems arrive 630 ms after an edit | in a 50,000-line file a typed problem shows ~0.63 s later: every edit re-parses and re-analyses the whole file (Studio, tools/live_lsp.ol). `olang check .` over open-track takes 12 s, all of it the parse | the server checks by top-level item: an edit re-parses its item and the items naming what it declares |
| `[tooling]` Native code calling native code has no time of its own | callees in a caller's native group have counted calls (7d76560) but their time is the caller's (tooling.md), so Studio's tier map shows none for them | time attributed per callee (group entries counted in the JIT call stub, or members reported with their root) |
| `[tooling]` REPL command output is not deterministic | `:env` lists modules in hash order; `:help tutorials`, `:tutorial` and `:search` ties vary — three piped runs, three outputs. Studio's pinned transcripts leave them out | sorted (name, then score), and the full transcripts pinned in tests/repl_commands_test.rs |
| `[tooling]` A missing `}` in a map literal blames the next line | `#{ "a": 1` one brace short reports "expected an operator" at the following statement (Heddle) | a statement ending inside an open `#{` says so and points at the opening brace |
| `[otc]` A path dependency's own path dependencies are not followed | open-track desktop → Loom → Heddle, all by path: "Cannot find module 'heddle'" until the app named Heddle too; a→b→c reproduces it, though a's olang.lock lists `cc` among `bb`'s dependencies | a path dependency's `[dependencies]` resolve relative to its own manifest and are written to the lockfile; pinned with three packages |
| `[tooling]` Undocumented behaviour | `olang test` runs each file with the working directory set to its own; `str.char_code` exists and appears nowhere in docs/ (Heddle) | both documented (tooling.md, stdlib.md) |

## Standard library

| Item | Evidence | Done when |
|---|---|---|
| `[stdlib]` Nothing writes to stderr | heddle-sql's plain output must keep errors out of `> out.csv` and writes `fs.append_file("/dev/stderr", …)` (Unix only); Heddle's picker has nowhere to put a refusal | `eprint`/`eprintln` on every target |
| `[stdlib]` `re` compiles its pattern on every call | `re.is_match` with a character class costs ~28 µs a call against 0.1 µs for `str.contains`; sanitising a 200×60 frame with it made painting ~5× slower (Heddle). regex_mod.rs calls `Regex::new` each call | a cache of compiled patterns, or a `re.compile` value |
| `[stdlib]` No way to run a child on the terminal | `$EDITOR` needs the terminal as stdin and stdout; Heddle's `run_editor` shells out to `sh -c '$EDITOR "$1" </dev/tty >/dev/tty'` | `proc.spawn(…, #{ stdio: "inherit" })`, or `tty.foreground(h, argv)` leaving and re-entering the tty around the child |
| `[stdlib]` Whether stdin alone is a terminal cannot be asked | `os.is_tty()` is stdout only and `tty.is_tty()` is stdin *and* stdout; Heddle's `auto` mode wants "inline when in a pipeline" | `os.is_tty(fd)` or `os.stdin_is_tty()` |
| `[stdlib]` `testing.snapshot` writes a String quoted, and only its own variable refreshes it | the snapshot of `"hello"` is stored with the quotes; Heddle maps `HEDDLE_UPDATE_SNAPSHOTS` onto `OLANG_UPDATE_SNAPSHOTS` | a String snapshot written raw; an update flag accepted from the caller |
| `[stdlib]` `meta.lit` refuses an object | "meta.lit: a Object has no literal source form"; Heddle's recordings convert objects to maps first | an object rendered as `{ a: 1 }` |
| `[stdlib]` No ASCII test; map keys must be scalars | the painter asks "printable ASCII?" of thousands of spans a frame (8–16% of it) and `str.is_ascii` does not exist; a style cannot be cached by its spec because `map_set` refuses a tuple key (Heddle) | `str.is_ascii(s)`; map keys of any value `==` compares |
| `[stdlib]` A dropped `db` connection is never closed | 3,000 connections opened and dropped left 3,010 descriptors open; connections live in db.rs's registry until `db.close`, while cursors are finished on drop (heddle-sql) | a connection closes when its last handle drops; pinned by a descriptor count |
| `[stdlib]` A cursor hands over every value whole | a window shows a text's first 1,000 characters and a blob's first 16 bytes; over 1 MB blobs, `db.next` moves 200 MB into olang a window (heddle-sql) | `#{ max_value_bytes: n }` on `db.cursor`/`db.query_rows`, the full length still reported |
| `[stdlib]` `db.interrupt` cannot be aimed | stopping a superseded count also stops a schema count or a window read on the same connection; heddle-sql keeps one connection per purpose | `db.interrupt(cursor)`, or the limit stated as a ruling in stdlib.md |
| `[stdlib]` No hex for `Bytes` | `crypto.hex_encode` takes a String only; heddle-sql's 256-entry table runs ~90 ms a MB | `bytes.to_hex(b)` / `bytes.from_hex(s)` |
| `[stdlib]` No ordered JSON output | `--json` must keep a query's column order, but `json.stringify` writes keys sorted; heddle-sql writes each object by hand | `json.stringify_pairs([(k, v)])`, or an ordered form `json.stringify` respects |
| `[stdlib]` `fs.search` answers whole | Studio's project search asked for streamed matches; `fs.search` returns all of them up to `limit` | a streaming form (a channel or callback of matches) |
| `[stdlib]` A loaded module's threads and blocking calls are outside its budget | Studio's plugins (S5): `runtime.call_budget` stops the calling thread's evaluation; a `spawn` the module makes, or a builtin that blocks (`os.exec`, `time.sleep`), runs on past it | a module's spawned work tied to its budget (or refused it), and a blocking builtin interruptible |
| `[stdlib]` A loaded module imports from its own folder only | Studio's plugins load through whatever interpreter runs the host's call — usually the VM's bridge, which has no dependency map — so a plugin's `use loom` (or any package) is not found; Studio hands plugins Loom's constructors in their context instead | `runtime.load_module` resolving packages as the host's program does, wherever it is called from |
| `[stdlib]` The terminal layer's leftovers | `tty` has no capability (a dependency may take the terminal over); record/replay does not capture tty input; a signal trapped with `os.on_interrupt` after `tty.enter` arrives as no event | a `tty` capability; tty input in recordings; trapped signals delivered as events |

## The desktop engine (`gui`)

| Item | Evidence | Done when |
|---|---|---|
| `[gui]` Inlay hints inside a line | lenses (a band under a line) and `eol` (text after a line's end) exist; an inlay — a parameter name or type inside a line — needs inline boxes. Studio shows the caret line's hints in its status bar instead (Loom, Studio) | `inlays: [(line, col, text, look)]` on a styled field: drawn inline, stepped over by the caret and selection, not in the value, announced as decoration |
| `[gui]` The plain `textarea` lays out its whole text on every keystroke | parley's PlainEditor re-lays-out the whole value: 2,000 lines cost 36 ms a keystroke headless against 0.6 ms in the styled field (Loom) | the plain field uses the styled field's per-paragraph layout, or the docs state it is for short values |
| `[gui]` A file hovering over a window has no position until it drops | winit 0.30 sends `HoveredFile` once and nothing as the file moves; the drop's position comes from AppKit at the drop, so Loom outlines every drop zone at once (Loom, open-track desktop) | the position reported while the file moves (AppKit's dragging callbacks or a winit with drag-moved events): a `files` hover event with `x`/`y`/`target` |
| `[gui]` A window cannot show its document's proxy icon | Mac document windows show the file's icon beside the title; the engine sets `edited` only (Studio) | `gui.set(h, #{ file: path })` sets the represented file on macOS |
| `[gui]` Span backgrounds in plain text | Loom's `diff_view` marks a changed character with a span `bg`; a plain `text` draws a span's colour and weight but not its background | span backgrounds drawn in plain text as in the styled field |
| `[gui]` Nearest-neighbour images on a canvas | a 220×90 pixel snapshot magnified 2× blurs; the canvas `image` op takes only `src` and `fit` (Studio) | `filter: "nearest"` on `image`, honoured by both renderers |
| `[gui]` A split's second pane has no least size of its own | `min_second` is accepted and ignored; flat.rs reads one `min` for both panes (Studio) | flat.rs honours `min_second` |
| `[gui]` Tray, clipboard images out, a `gui` capability | asked in loom/SPEC.md's engine list and not built: `gui.tray(spec)` with activations as events; `gui.clipboard_write` taking a picture; `gui = true` in a capability manifest (caps.rs knows fs/net/proc/db/env only) | the three, with the capability refusing `gui.open`/`gui.headless` when not granted |
| `[gui]` The accessibility client test cannot read its own process | on Darwin 27.2, `AXUIElementCopyActionNames` from the same process answers AXApplication for every element, so tests/gui_macos_a11y_test.rs skips that half | the test reads the window from a helper process, as VoiceOver does, and the skip goes |
| `[gui]` People, not scripts | the L0 criterion: VoiceOver used by a person (the Actions rotor included) and a Japanese input method typed by a person in a real window; dialogs, clipboard and the menu bar are run by hand only (open-track-desktop-checklist.md is the script) | each checked by hand and recorded |
| `[gui]` Upstream the accesskit_macos patch | custom actions are served from `vendor/accesskit_macos` through `[patch.crates-io]` | the patch released upstream and the vendor directory gone |

## Web SDK

| Item | Evidence | Done when |
|---|---|---|
| `[web-sdk]` The default hot list costs more boot than it saves | with the SDK's default (every client function) open-track's modules took 148 ms to load, 135 of it compiling; with `"hot": []` 13 ms and a 490 ms boot; a render-path-only list bought nothing back. The harness gate measures only the small SDK demo | a default that pays (what the boot render reaches, or compiled off the boot path) or none; the boot cost of a large client pinned |
| `[web-sdk]` `memo` stamps its inputs with `show` | html.ol stamps `memo` and `memo_list` with `show(inputs)`; the entry itself measured `show` and `==` equal natively (6 µs for 43 keys) | compared structurally, or recorded as not taken with that measurement. Low |

## Platforms (parked)

Kept, not scheduled, while the work is macOS-only.

| Item | Evidence | Done when |
|---|---|---|
| `[gui]` Windows and Linux windows run | Windows compiles (a `cargo check` and a MinGW link) and has never run; Linux builds and its headless tests pass in an arm64 container with no compositor and no GPU. Narrator, NVDA and Orca untried; input methods off macOS untried | the L0 window works by keyboard, pointer, screen reader and input method on Windows and on a Wayland compositor |
| `[gui]` The capabilities `gui.platform()` reports false | no file drops on Wayland (winit); a Windows drop lands at the pointer's last place; the clipboard's picture is read on macOS only (arboard without `image-data`); contrast, motion, transparency and the accent are not read off macOS | each flag true where the platform allows it |
| `[stdlib]` The terminal on Windows | `tty.query` and suspend are not implemented on Windows; the H0 criterion names Windows Terminal | both on Windows Terminal |

## Process

One lane at a time, each landing with its tests and documentation in
the same change, and its row removed from this file in that change.
When an item's cost proves larger than expected, the schedule moves,
not the bar.

The applications built on olang (Studio, open-track, Loom, Heddle,
Shuttle, Foundry, heddle-sql, open-track desktop) log what they find
in olang **here**, as a row in the matching table with its evidence:
what was observed, in which program, and the measurement. Their own
FINDINGS files hold only what they find in themselves and their
frameworks. Nothing enters on speculation, and a row that cannot be
reproduced at HEAD is not added.

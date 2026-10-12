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

- [Language and runtime](#language-and-runtime)
- [Memory and start-up](#memory-and-start-up)
- [Tooling](#tooling)
- [Standard library](#standard-library)
- [The desktop engine (`gui`)](#the-desktop-engine-gui)
- [Web SDK](#web-sdk)
- [Platforms (parked)](#platforms-parked)
- [Stdlib naming audit](#stdlib-naming-audit)
- [Process](#process)

## Language and runtime

| Item | Evidence | Done when |
|---|---|---|
| `[olang]` A list held in a map is copied on every append on the compiled tiers | `map_set(m, "log", map_get(m, "log") + [line])` copies the list: 10,000 appends 110 ms, 40,000 1,993 ms — quadratic (Heddle keeps its log in 256-line chunks, heddle/lib/components/log.ol). `m = map_update(m, "log", (xs) => xs + [line])` (0.88) moves the list out and is linear on the interpreter (`--no-ovm`: 20,000 27 ms, 160,000 202 ms) but not tiered (37 ms, 5,127 ms, re-measured 2026-10-11): the VM hands the lambda its argument by reference (`execute` takes a slice), so `xs + [line]` copies, and the JIT's list concatenation allocates anew | appending to a list held in a map is amortised O(1) on every tier (an owned-argument call for `map_update`'s function, or a `map_take`), pinned by a scaling test |
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
| `[olang]` olang Studio.app's first frame is over 180 ms from its process's start | the bytecode is kept between runs now (the compile cache: Studio's ~545 compiles ~65 → ~8 ms warm; tools/launch.ol ~218 → ~173 ms, interleaved medians), and the app's first frame from its process's start went ~252 → ~215–230 ms. What remains on its path: the JIT's 26 groups (~15 ms) compile every launch — their code holds this run's addresses (helpers moved by ASLR, interned shapes, the group's other members) and is specialized on the kinds its first call saw (docs/ovm.md "The compile cache") — and the program now reaches `gui.open` (~195 ms) about when the platform's warm-up on the main thread (AppKit, a hidden window) ends (~150–210 ms, varying with the machine), so a faster program waits for it. A cold compile cache costs ~10% more compiling (~+7 ms) (Studio, 2026-10-10) | native code kept beside its bytecode entry (code and relocations written by symbol, patched into `MAP_JIT` memory on load, keyed by the entry, the kinds and the CPU features) or compiled off the program's thread, and the platform's warm-up off the first frame's path: olang Studio.app's first frame under 180 ms from its process's start. Native code kept exactly needs the JIT's baked addresses made symbols first (docs/ovm.md "The compile cache": the three changes) — looked at again 2026-10-11 and left: a large design item |
| `[olang]` A freshly built application starts later under Launch Services | measured 2026-10-10 against tools/launch_app.sh's harness: `open` → process start is ~13–15 ms for an olang Studio.app that has been on disk a while and ~25 ms for one built in the last few minutes — the same bundle with the runtimes swapped follows the bundle, not the runtime, and a C executable in the same harness takes the same as olang's (the earlier "~30 vs ~13 ms" did not reproduce). After the process starts, olang reached `main` ~3.5 ms after a C executable linking the same frameworks, now as soon (chained fixups); its two static initializers (aws-lc's CPU probe and one C initializer) and its 17 libraries (all AppKit's own dependencies but libiconv) cost nothing measurable, and it has no Rust constructors | what the system does with a new bundle in its first minutes found (Gatekeeper's assessment, XProtect, Spotlight are candidates), and whether a build can avoid it; a launch measured on a bundle a few minutes old says so |
| `[olang]` What holds live memory, by site | growth under load was undiagnosable from open-track; `runtime.memory()` answers by owner, and `OLANG_ALLOC_TRACE` gives only large allocations, only in an `alloc-count` build | live bytes by allocation site or value kind in a release binary (`runtime.heap()`, or a dump at a signal). Low now the leak (OL-146) is fixed |

## Tooling

| Item | Evidence | Done when |
|---|---|---|
| `[tooling]` A large file's problems arrive 630 ms after an edit | in a 50,000-line file a typed problem shows ~0.63 s later: every edit re-parses and re-analyses the whole file (Studio, tools/live_lsp.ol). `olang check .` over open-track takes 12 s, all of it the parse | the server checks by top-level item: an edit re-parses its item and the items naming what it declares |
| `[tooling]` Native code calling native code has no time of its own | callees in a caller's native group have counted calls (7d76560) but their time is the caller's (tooling.md), so Studio's tier map shows none for them | time attributed per callee (group entries counted in the JIT call stub, or members reported with their root) |

## Standard library

| Item | Evidence | Done when |
|---|---|---|
| `[stdlib]` No process table and no signal by pid | Studio's process manager runs `/bin/ps -axo pid,ppid,pgid,pcpu,rss,etime,stat,command` and `lsof -a -d cwd` every 2 s for its children's CPU, memory, time and folder, and `/bin/kill -TERM -<pgid>` to stop one Loom did not start as a handle (core/procs.ol, face/autoface.ol); at quit it lists every descendant the same way | `os.processes()` (pid, parent, group, CPU, memory, start, state, command, folder) and `os.kill(pid, signal, #{ group })`, on every platform |
| `[stdlib]` No way to run a child on the terminal | `$EDITOR` needs the terminal as stdin and stdout; Heddle's `run_editor` shells out to `sh -c '$EDITOR "$1" </dev/tty >/dev/tty'` | `proc.spawn(…, #{ stdio: "inherit" })`, or `tty.foreground(h, argv)` leaving and re-entering the tty around the child |
| `[stdlib]` Map keys must be scalars | a style cannot be cached by its spec because `map_set` refuses a tuple key (Heddle) | map keys of any value `==` compares |
| `[stdlib]` A cursor hands over every value whole | a window shows a text's first 1,000 characters and a blob's first 16 bytes; over 1 MB blobs, `db.next` moves 200 MB into olang a window (heddle-sql) | `#{ max_value_bytes: n }` on `db.cursor`/`db.query_rows`, the full length still reported |
| `[stdlib]` `db.interrupt` cannot be aimed | stopping a superseded count also stops a schema count or a window read on the same connection; heddle-sql keeps one connection per purpose | `db.interrupt(cursor)`, or the limit stated as a ruling in stdlib.md |
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
| `[gui]` A window's frame, kept and given back | a window says where its content is (`moved`) but not its frame, and `gui.open` takes a size only: olang Studio restores every window and its tab bar at launch but not where each was (Studio S6) | `gui.open`'s `frame` (`x`, `y`, `w`, `h`, the screen's) and the frame said on `moved`/`resize`, kept on screen when the displays changed |
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

## Stdlib naming audit

Every global builtin (80) and every function of the native modules was
listed on 2026-10-11 and read against docs/STYLE.md and stability.md's
conventions (a function returns its value, a `Result` when it can fail
for a reason the caller can handle, and raises when called wrongly; a
lookup's absence is `()`; the subject is the first argument, so a call
pipes). One case was clear-cut and changed in 0.88; the rest are open
for the owner's decision before the freeze. Each would be a rename with
the old name kept as a deprecated alias (`olang check` flags it, `olang
fix deprecated` rewrites it), so none breaks a program.

| Finding | Names | Kind | Status |
|---|---|---|---|
| Two names, one function | `map_path` / `map_get_in` | duplicate | changed: `map_path` deprecated (stdlib.md, "Deprecated names") |
| Two names, one function | `skip` / `drop` (and `col.skip` / `col.drop`) | duplicate | debatable: `drop` is documented as the name most languages pair with `take`; deprecate one, or keep both as stated |
| Two names, one function | `fold` / `reduce` (same arguments: list, initial, function) | duplicate | debatable: `reduce` usually takes no initial value elsewhere, so the shared shape misleads; deprecate `reduce`, or give it the no-initial form (a behaviour change) |
| Two names, one rendering | `show` / `to_string` | duplicate | debatable: both render the same text (stdlib.md's table said `to_string` quotes strings; it does not); keep one as the documented spelling |
| Identity functions | `lazy`, `force` | vestigial | debatable: documented as the identity "kept for source compatibility"; deprecate both |
| A lookup that is a `Result` | `find(xs, pred)` answers `Ok`/`Err`, `col.index_of` answers the index or `()` | convention | debatable: the 0.69 rule says a lookup's absence is `()`; changing `find` breaks its callers' `match`es |
| The subject is not first | `re.find(pattern, text)` and every `re.*`; `str.*` take the text first | argument order | debatable: `text \|> re.find(p)` does not pipe; a reordering needs new names (`re.find_in(text, pattern)`) |
| One verb, two meanings | `str.replace` replaces every match (`str.replace_first` the first); `re.replace` replaces the first (`re.replace_all` every one) | naming | debatable: a `re.replace_first` alias and `re.replace` replacing all would align them, but changes `re.replace`'s behaviour |
| Text-level JSON helpers | `json.get_keys`, `json.get_values`, `json.get_type`, `json.array_length`, `json.get`, `json.set` on JSON text, beside `map_keys`, `typeof`, `len` on parsed values | overlap | debatable: they predate `json.parse` returning dot-readable values; deprecate the text-level set |
| Module-prefixed names inside a module | `random.randint`, `random.randbool`, `random.randstr*` | naming | debatable: `random.int`, `random.bool`, `random.string` read better under the module name |
| Hex both ways, twice | `crypto.hex_encode` / `crypto.hex_decode` (strings) and `bytes.to_hex` / `bytes.from_hex` | overlap | debatable: one pair could serve both, the String form converting through bytes |
| A tty test in two modules | `os.is_tty` and `tty.is_tty` | overlap | debatable: the same question; keep the one in `tty` |
| `entries` beside `map_*` | `entries(m)`, `map_keys(m)`, `map_values(m)` | naming | debatable: a `map_entries` alias would complete the family |
| Clocks in two modules | `time.now_ms`, `time.monotonic_ms`, `dates.stamp_ms`, `dates.timestamp`, `dates.now` | overlap | kept: `time` is clocks for measuring, `dates` is calendar time; stdlib.md says which to use |
| Modules registering a usual count | `ods`, `gui`, `vt` register a function by its usual argument count, the other modules by the fewest it takes | convention | debatable (internal): the checker judges only the latter's counts; registering the fewest everywhere would let it judge all |

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

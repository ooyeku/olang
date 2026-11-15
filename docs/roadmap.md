# Roadmap

The project's work list; not a chapter of [the book](README.md). It
holds what olang still has to do and nothing else: an item leaves this
file in the change that lands it, and what landed, and why, is in the
CHANGELOG.

Every row was observed, not imagined: each names the kind of program
that ran into it and what was measured. Tags name the surface:
`[olang]` the language and runtime, `[stdlib]` builtins and bundled
packages, `[gui]` the desktop engine, `[tooling]` the checker, LSP,
REPL and test runner, `[web-sdk]` the web framework. Any 1.x release
may carry a row, under [stability.md](stability.md)'s rules: it adds
without changing what a documented program observes.

Off-macOS work is parked (macOS is the platform olang is verified and
polished on); those rows are kept together under
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
- [Planned for 1.x](#planned-for-1x)

## Language and runtime

| Item | Evidence | Done when |
|---|---|---|
| `[olang]` The friction additions | `is_map`/`is_number` (9 and 4 definitions, 58 inline `typeof` tests); `stats.median` (copied 12 times); a checker warning for a parameter named like a stdlib module, and for `spawn f` with an uncalled function | each added |
| `[olang]` `first` and `last` cost twice a guarded index | compiled tiers: 2M calls ~420 ms against ~210 ms for `len(xs) > 0 => xs[0]`; they compile as a generic builtin call | compiled as an index with an empty check |
| `[olang]` A list held in a map is copied on every append on the compiled tiers | `map_set(m, "log", map_get(m, "log") + [line])` copies the list: 10,000 appends 110 ms, 40,000 1,993 ms — quadratic (a terminal UI's log keeps its lines in 256-line chunks to avoid it). `m = map_update(m, "log", (xs) => xs + [line])` (0.88) moves the list out and is linear on the interpreter (`--no-ovm`: 20,000 27 ms, 160,000 202 ms) but not tiered (37 ms, 5,127 ms, re-measured 2026-10-11): the VM hands the lambda its argument by reference (`execute` takes a slice), so `xs + [line]` copies, and the JIT's list concatenation allocates anew | appending to a list held in a map is amortised O(1) on every tier (an owned-argument call for `map_update`'s function, or a `map_take`), pinned by a scaling test |
| `[olang]` A closure over a large local pays per call on the bytecode tier | a capture benchmark: `map` over 20,000 indices with `(i) => xs[i]`, `xs` a local 100k list, costs 23,550 ns an element against 50 for a parameter or global capture; 2,000 calls take 47 ms compiled, 0 ms under `--no-ovm`. Closure *creation* was fixed in W22; this is the call | a captured local is shared, not copied per call; 04's local-capture rows within 2× of the parameter rows |
| `[olang]` `reverse` deep-copies on the bytecode tier | twelve 2,047-node trees: `reverse(xs)` ~1,280 µs an op on 1.0, against ~0 for `take`, `skip`, `filter`, `+`, `map`, which share their elements; 0 ms under `--no-ovm` | `reverse` shares elements on the VM as `take` does |
| `[olang]` A module passed as a value falls back to the tree-walker | after builtins crossed the boundary (7d76560), the tiers fixture's remaining surprise is `apply(str, "iii")`; tooling.md documents it with "pass the data instead" | a module value crosses the tier boundary as a builtin does, or tooling.md states it as a ruling |
| `[olang]` A lambda compiled once per body | a web app compiles about 100 per-closure bytecode bodies a request for lambdas that capture data; a38daa3 shares compiles only between closures with equal cheap captures and evicts the rest | each lambda body compiles once with its captures as run-time inputs; no compile on a request's path. Low: the leak it caused is gone |
| `[olang]` A match on string literals is a linear chain | a large desktop app's update function matches about 300 string arms; not shown to be a cost yet | a match of string-literal arms compiles to a table — only when a profile asks for it |

## Memory and start-up

| Item | Evidence | Done when |
|---|---|---|
| `[olang]` Every http worker compiles its own copy of the program | `runtime.memory()` shows each `olang-http-N` at 45–60 MB live once warm, against 13 MB for the whole process with the tier off: bytecode, constants and a bridge per VM. 18 default workers is about 1 GB, so a large web app caps itself at 8 workers. That app served is 330–445 MB resident against the 250 MB target (W21/W22: "not met"; the rest above the 97 MB heap is the small-object zones' freed pages) | immutable compiled code is shared across a process's threads (JIT module and inline caches per thread), per-worker live memory after warm-up is a few MB, and that app served at 18 workers is under 250 MB resident |
| `[olang]` Session start under 100 ms for a large client | a large web client's browser boot is 490 ms with no hot list; 360 ms of it is before the app's first statement and grows with program size, and nothing attributes it. The browser runtime is 0.97 MB brotli and cannot shrink further while the parser and compiler ship in it; a program image as bytecode was measured and left "a design for later" (W15 items 5–6) | first a boot profile that splits decode, declare, top level and first render for a real bundle; then what it names (the bytecode image, which would also let a browser profile drop the parser and compiler). Done: that client's session start under 100 ms and the runtime under 400 KB |
| `[olang]` A large desktop app's first frame is over 180 ms from its process's start | the bytecode is kept between runs now (the compile cache: a large desktop app's ~545 compiles ~65 → ~8 ms warm; its launch ~218 → ~173 ms, interleaved medians), and the app's first frame from its process's start went ~252 → ~215–230 ms. What remains on its path: the JIT's 26 groups (~15 ms) compile every launch — their code holds this run's addresses (helpers moved by ASLR, interned shapes, the group's other members) and is specialized on the kinds its first call saw (docs/ovm.md "The compile cache") — and the program now reaches `gui.open` (~195 ms) about when the platform's warm-up on the main thread (AppKit, a hidden window) ends (~150–210 ms, varying with the machine), so a faster program waits for it. A cold compile cache costs ~10% more compiling (~+7 ms) (2026-10-10) | native code kept beside its bytecode entry (code and relocations written by symbol, patched into `MAP_JIT` memory on load, keyed by the entry, the kinds and the CPU features) or compiled off the program's thread, and the platform's warm-up off the first frame's path: that app's first frame under 180 ms from its process's start. Native code kept exactly needs the JIT's baked addresses made symbols first (docs/ovm.md "The compile cache": the three changes) — looked at again 2026-10-11 and left: a large design item |
| `[olang]` Modules are read and evaluated before the first frame, all of them | `OLANG_BOOT_TRACE=1` on a large desktop app (2026-10-18): 153 modules read and evaluated in ~122–146 ms, against 89 in ~60 ms on 2026-10-08 — the cost follows the program's size, not what its first frame needs. It now outlasts the platform's warm-up (done ~115–165 ms), so it is on the first frame's path; `gui.open` follows ~40–55 ms later. Its Git, notebook, terminal and profiler views are needed only after the first frame. A top-level `use` loads its module before the first frame; a `use` inside a function loads it when that function first runs, but the app does not defer those four this way yet | a module whose top level has no effect (the checker's purity already says so) is evaluated at its first use rather than at start — a declaration's name bound to a thunk, resolved once — so no program observes the difference, and modules read and parsed in parallel where the parse cache misses. Done when that app's modules cost under 50 ms before its first frame, measured with `OLANG_BOOT_TRACE`, and a program's output is unchanged (tier agreement and the book's examples) |
| `[olang]` A freshly built application starts later under Launch Services | measured 2026-10-10 with a launch harness: `open` → process start is ~13–15 ms for an olang application bundle that has been on disk a while and ~25 ms for one built in the last few minutes — the same bundle with the runtimes swapped follows the bundle, not the runtime, and a C executable in the same harness takes the same as olang's (the earlier "~30 vs ~13 ms" did not reproduce). After the process starts, olang reached `main` ~3.5 ms after a C executable linking the same frameworks, now as soon (chained fixups); its two static initializers (aws-lc's CPU probe and one C initializer) and its 17 libraries (all AppKit's own dependencies but libiconv) cost nothing measurable, and it has no Rust constructors | what the system does with a new bundle in its first minutes found (Gatekeeper's assessment, XProtect, Spotlight are candidates), and whether a build can avoid it; a launch measured on a bundle a few minutes old says so |
| `[olang]` A long-running thread's VM registers keep dead values (OL-572) | a long-running desktop app on olang 74b6a872 (2026-11-08): a verify run inside it grew olang's values 185 → 699 MB and they stayed until restart; the app's model was ~27 MB. With MallocStackLogging, `leaks --referenceTree` put 240 MB under one thread-local `Vec` of VM records (OvmMaps from `dispatch_loop` holding `fs` and `str.split` results), and `malloc_history` named that `Vec` the VM's register slab (`push_frame_from_regs`, ~9,200 slots) on the main program thread. A returning frame is reset (`pop_frame`, both paths) and a loop's rare branch frees its data (OL-411), so the holder is a frame that never returns or a window left above `top`. The first holder of the same growth, the VM's free queue left undrained by an interpreter value's free, is fixed (OL-571) | the frame and slot named (a per-frame report of the VM's non-scalar slots, `OLANG_VM_SLOTS` or `runtime.vm_slots()`), the slot released, and the same verify run ending with olang's values within ~10% of where they began. |
| `[olang]` What holds live memory, by site | growth under load in a large web app was undiagnosable; `runtime.memory()` answers by owner, and `OLANG_ALLOC_TRACE` gives only large allocations, only in an `alloc-count` build | live bytes by allocation site or value kind in a release binary (`runtime.heap()`, or a dump at a signal). Low now the leak (OL-146) is fixed |
| `[gui]` A small window's resident memory is twice the target | a counter app in a real window was 167 MB resident against an 80 MB target: a bare wgpu window on Metal is 94 MB, and the bundled fonts add ~29 MB while they decompress | a small `gui` program under 80 MB resident on macOS, measured with its fonts registered |
| `[olang]` The runtime alone is larger than a small application's budget | olang 0.88.0's executable is 45.9 MB (45,893,408 bytes); two desktop apps built on it are 46 MB and 50 MB; a small program with `fonts: "core"` is meant to be under 25 MB | an application bundle carries only the runtime a program uses (features left out, or code a program never reaches stripped), and a counter app with `fonts: "core"` is under 25 MB; or the target is revised with the reason |
| `[olang]` Memory freed by a run loop of tasks is not given back | a terminal SQL client's plain output moved onto its UI framework's `emit` (a task a 2,000-row chunk): `select * from events` over a million rows peaked at 273 MB resident against 52 MB for one task, 155 MB at 400,000 rows (81 MB with 10,000-row chunks), while olang's heap read at each chunk stayed at 18–35 MB. The same turns in process stay at 52 MB, and a plain olang loop of a thread a chunk answering over a channel at 30 MB; what differs is the framework's real loop (a task a thread, answers through the inbox, `recv_timeout`). The client keeps one task writing `/dev/stdout` | the probe (`select * from events` over a million rows to `/dev/null` with the chunked arm) shows resident memory following olang's heap, or the retention is traced to the framework |

## Tooling

| Item | Evidence | Done when |
|---|---|---|
| `[tooling]` `doctor` and `fix` answer only as text | `olang doctor` and `olang fix` have no `--format json`, though an editor runs both | `--format json` on each, with `format`, `kind` and absolute paths as the other documents (tooling.md) |
| `[tooling]` `replay --why` | tooling.md promises it at record and replay's boundaries: a replay carrying each value's provenance, to answer "where did this number come from?" with a chain back to the recorded inputs | `olang replay --why <expr>` prints that chain for a value in a recorded run |
| `[tooling]` A large file's problems arrive 630 ms after an edit | in a 50,000-line file a typed problem shows ~0.63 s later: every edit re-parses and re-analyses the whole file. `olang check .` over a large web app takes 12 s, all of it the parse | the server checks by top-level item: an edit re-parses its item and the items naming what it declares |
| `[tooling]` Native code calling native code has no time of its own | callees in a caller's native group have counted calls (7d76560) but their time is the caller's (tooling.md), so a tier map shows none for them | time attributed per callee (group entries counted in the JIT call stub, or members reported with their root) |

## Standard library

| Item | Evidence | Done when |
|---|---|---|
| `[stdlib]` No process table and no signal by pid | a desktop app's process manager runs `/bin/ps -axo pid,ppid,pgid,pcpu,rss,etime,stat,command` and `lsof -a -d cwd` every 2 s for its children's CPU, memory, time and folder, and `/bin/kill -TERM -<pgid>` to stop one it did not start as a handle; at quit it lists every descendant the same way | `os.processes()` (pid, parent, group, CPU, memory, start, state, command, folder) and `os.kill(pid, signal, #{ group })`, on every platform |
| `[stdlib]` No way to run a child on the terminal | `$EDITOR` needs the terminal as stdin and stdout; a terminal UI's `run_editor` shells out to `sh -c '$EDITOR "$1" </dev/tty >/dev/tty'` | `proc.spawn(…, #{ stdio: "inherit" })`, or `tty.foreground(h, argv)` leaving and re-entering the tty around the child |
| `[stdlib]` Map keys must be scalars | a style cannot be cached by its spec because `map_set` refuses a tuple key (a terminal UI framework) | map keys of any value `==` compares, additive only: the scalar keys keep their text (language.md, "Values and runtime types": `map_keys(#{1: "a"})` stays `["1"]`) |
| `[stdlib]` A cursor hands over every value whole | a window shows a text's first 1,000 characters and a blob's first 16 bytes; over 1 MB blobs, `db.next` moves 200 MB into olang a window (a terminal SQL client) | `#{ max_value_bytes: n }` on `db.cursor`/`db.query_rows`, the full length still reported |
| `[stdlib]` `db.interrupt` cannot be aimed | stopping a superseded count also stops a schema count or a window read on the same connection; a SQL client keeps one connection per purpose | `db.interrupt(cursor)`, or the limit stated as a ruling in stdlib.md |
| `[stdlib]` `fs.search` answers whole | an editor's project search asked for streamed matches; `fs.search` returns all of them up to `limit` | a streaming form (a channel or callback of matches) |
| `[stdlib]` A loaded module's threads are outside its budget | a desktop app's plugins: `runtime.call_budget` stops the calling thread, its waits and an `os.exec` it is running included, but a task the module spawns runs on past it | a module's spawned work tied to its budget, or refused under it |
| `[stdlib]` The terminal layer's leftovers | `tty` has no capability (a dependency may take the terminal over); record/replay does not capture tty input; a signal trapped with `os.on_interrupt` after `tty.enter` arrives as no event | a `tty` capability; tty input in recordings; trapped signals delivered as events |

## The desktop engine (`gui`)

| Item | Evidence | Done when |
|---|---|---|
| `[gui]` Inlay hints inside a line | lenses (a band under a line) and `eol` (text after a line's end) exist; an inlay — a parameter name or type inside a line — needs inline boxes. an editor shows the caret line's hints in its status bar instead | `inlays: [(line, col, text, look)]` on a styled field: drawn inline, stepped over by the caret and selection, not in the value, announced as decoration |
| `[gui]` The plain `textarea` lays out its whole text on every keystroke | parley's PlainEditor re-lays-out the whole value: 2,000 lines cost 36 ms a keystroke headless against 0.6 ms in the styled field | the plain field uses the styled field's per-paragraph layout, or the docs state it is for short values |
| `[gui]` A file hovering over a window has no position until it drops | winit 0.30 sends `HoveredFile` once and nothing as the file moves; the drop's position comes from AppKit at the drop, so a UI framework outlines every drop zone at once | the position reported while the file moves (AppKit's dragging callbacks or a winit with drag-moved events): a `files` hover event with `x`/`y`/`target` |
| `[gui]` A window cannot show its document's proxy icon | Mac document windows show the file's icon beside the title; the engine sets `edited` only | `gui.set(h, #{ file: path })` sets the represented file on macOS |
| `[gui]` A window's frame, kept and given back | a window says where its content is (`moved`) but not its frame, and `gui.open` takes a size only: a desktop app restores every window and its tab bar at launch but not where each was | `gui.open`'s `frame` (`x`, `y`, `w`, `h`, the screen's) and the frame said on `moved`/`resize`, kept on screen when the displays changed |
| `[gui]` Nearest-neighbour images on a canvas | a 220×90 pixel snapshot magnified 2× blurs; the canvas `image` op takes only `src` and `fit` | `filter: "nearest"` on `image`, honoured by both renderers |
| `[gui]` A split's second pane has no least size of its own | `min_second` is accepted and ignored; flat.rs reads one `min` for both panes | flat.rs honours `min_second` |
| `[gui]` A picture's natural size is not the picture's | an `image` leaf with no `width`/`height` is 160 × 120: `gui.flatten`'s `leaf_size` applies a fixed rule, while `gui.image_info` reads the real size from the header | `gui.flatten` sizes such a leaf from its header (an SVG's units as logical pixels, a bitmap's pixels over the window's scale), pinned against `gui.image_info` |
| `[gui]` Static text cannot be selected | an editor's console and its profiler's error text pass `selectable: true` and nothing implements it, so the console's menu offers Copy Output (everything) rather than a selection | a text node with `selectable` is selected by the pointer and copied with ⌘C, and a context menu's Copy copies the selection |
| `[gui]` What a UI framework asks of the engine and is not built: a tray, clipboard images out, a `gui` capability, transitions | `gui.tray(spec)` with activations as events; `gui.clipboard_write` taking a picture; `gui = true` in a capability manifest (caps.rs knows fs/net/proc/db/env, and refuses `gui.*` to a module loaded at run time only); a node's `transition` (opacity, translation, scale, its box) animated by the engine between two frames and instant under Reduce Motion (nothing on screen moves today but an animated picture) | the four, with the capability refusing `gui.open`/`gui.headless` when not granted (capabilities are Experimental: a new gated module is allowed) |
| `[gui]` The accessibility client test cannot read its own process | on Darwin 27.2, `AXUIElementCopyActionNames` from the same process answers AXApplication for every element, so tests/gui_macos_a11y_test.rs skips that half | the test reads the window from a helper process, as VoiceOver does, and the skip goes |
| `[gui]` People, not scripts | the L0 criterion: VoiceOver used by a person (the Actions rotor included) and a Japanese input method typed by a person in a real window; dialogs, clipboard and the menu bar are run by hand only, from a written checklist | each checked by hand and recorded |
| `[gui]` A held quit cannot be cancelled, nor windows drawn while it finishes | `gui.hold_quit` (2026-10-20) makes an external quit the program's `quit` event and the platform waits (NSTerminateLater) until the program ends; the program cannot answer "don't quit" (a dialog asking to save), and while AppKit waits winit runs no handler, so a frame or window the program sends meanwhile is not shown (an editor's hot exit writes files only) | a `gui.quit_reply(false)` that cancels, and the window commands served in the modal panel mode (or a progress shown), on macOS; the same on Windows (`WM_QUERYENDSESSION`) and Linux |
| `[gui]` Upstream the accesskit_macos patch | custom actions are served from `vendor/accesskit_macos` through `[patch.crates-io]` | the patch released upstream and the vendor directory gone |

## Web SDK

| Item | Evidence | Done when |
|---|---|---|
| `[web-sdk]` The default hot list costs more boot than it saves | with the SDK's default (every client function) a large client's modules took 148 ms to load, 135 of it compiling; with `"hot": []` 13 ms and a 490 ms boot; a render-path-only list bought nothing back. The harness gate measures only the small SDK demo | a default that pays (what the boot render reaches, or compiled off the boot path) or none; the boot cost of a large client pinned |
| `[web-sdk]` `memo` stamps its inputs with `show` | html.ol stamps `memo` and `memo_list` with `show(inputs)`; the entry itself measured `show` and `==` equal natively (6 µs for 43 keys) | compared structurally, or recorded as not taken with that measurement. Low |

## Platforms (parked)

Kept, not scheduled, while the work is macOS-only.

| Item | Evidence | Done when |
|---|---|---|
| `[gui]` Windows and Linux windows run | Windows compiles (a `cargo check` and a MinGW link) and has never run; Linux builds and its headless tests pass in an arm64 container with no compositor and no GPU. Narrator, NVDA and Orca untried; input methods off macOS untried | the L0 window works by keyboard, pointer, screen reader and input method on Windows and on a Wayland compositor |
| `[gui]` The capabilities `gui.platform()` reports false | no file drops on Wayland (winit); a Windows drop lands at the pointer's last place; the clipboard's picture is read on macOS only (arboard without `image-data`); contrast, motion, transparency and the accent are not read off macOS | each flag true where the platform allows it |
| `[stdlib]` The terminal on Windows | `tty.query` and suspend are not implemented on Windows; the H0 criterion names Windows Terminal | both on Windows Terminal |

## Stdlib naming audit

Every global builtin and every function of the native modules was read
against stability.md's conventions (a function returns its value, a
`Result` when it can fail for a reason the caller can handle, and raises
when called wrongly; a lookup's absence is `()`; the subject is the
first argument, so a call pipes). What 1.0 settled is in the CHANGELOG;
these are the cases left open. A rename keeps the old name as an alias
for a minor release (stability.md, Deprecation).

| Finding | Names | Kind | Status |
|---|---|---|---|
| Two names, one function | `skip` / `drop` (and `col.skip` / `col.drop`) | duplicate | debatable: `drop` is documented as the name most languages pair with `take`; deprecate one, or keep both as stated |
| Two names, one rendering | `show` / `to_string` | duplicate | debatable: both render the same text (stdlib.md's table said `to_string` quotes strings; it does not); keep one as the documented spelling |
| The subject is not first | `re.find(pattern, text)` and every `re.*`; `str.*` take the text first | argument order | debatable: `text \|> re.find(p)` does not pipe; a reordering needs new names (`re.find_in(text, pattern)`) |
| Text-level JSON helpers | `json.get_keys`, `json.get_values`, `json.get_type`, `json.array_length`, `json.get`, `json.set` on JSON text, beside `map_keys`, `typeof`, `len` on parsed values | overlap | debatable: they predate `json.parse` returning dot-readable values; deprecate the text-level set |
| Module-prefixed names inside a module | `random.randint`, `random.randbool`, `random.randstr*` | naming | debatable: `random.int`, `random.bool`, `random.string` read better under the module name |
| Hex both ways, twice | `crypto.hex_encode` / `crypto.hex_decode` (strings) and `bytes.to_hex` / `bytes.from_hex` | overlap | debatable: one pair could serve both, the String form converting through bytes |
| A tty test in two modules | `os.is_tty` and `tty.is_tty` | overlap | debatable: the same question; keep the one in `tty` |
| `entries` beside `map_*` | `entries(m)`, `map_keys(m)`, `map_values(m)` | naming | debatable: a `map_entries` alias would complete the family |
| Modules registering a usual count | `ods`, `gui`, `vt` register a function by its usual argument count, the other modules by the fewest it takes | convention | debatable (internal): the checker judges only the latter's counts; registering the fewest everywhere would let it judge all |

## Process

One lane at a time, each landing with its tests and documentation in
the same change, and its row removed from this file in that change.
When an item's cost proves larger than expected, the schedule moves,
not the bar.

The applications built on olang log what they find in olang **here**,
as a row in the matching table with its evidence: what was observed,
in what kind of program, and the measurement. What they find in
themselves and their frameworks stays with them. Nothing enters on speculation, and a row that cannot be
reproduced at HEAD is not added.

## Planned for 1.x

Work that waited for 1.0 to ship. Each item may land in a 1.x release,
under [stability.md](stability.md)'s rules.

| Item | Where it stands |
|---|---|
| `[stdlib]` MCP, agents, `ai` and Draft | built on branches now kept as the tags `archive/ag-ai`, `archive/ag-check`, `archive/ag-core`, `archive/ag-draft`, `archive/ag-sandbox`, `archive/ag-trust` and `archive/integrate`; to ship as Experimental |
| `[stdlib]` `sim`: what S6 left | `gui` apps in a world (`gui.platform()` answered there, so the headless driver can load); `w.client` reaching module cells; a stopped node refusing connections; tasks as coroutines rather than threads for speed |
| `[olang]` The bytecode tier recompiles a lambda whose captures change | `db.transaction`'s callback is compiled again at every call (`hof_function_id` in src/ovm/bytecode.rs) |
| `[stdlib]` The `toad` module on the web | `toad` runs on the desktop only |
| `[olang]` Field shorthand in record literals, `{ holes }` for `{ holes: holes }` | patterns and parameters take it already; a literal refuses it (OL-592). From [Writing idiomatic olang](idioms.md) |
| `[olang]` An update form for nested record fields, and enum cases with named fields | `{ ...m, view: { ...m.view, top: 0 } }` is today's nested update; a case with more than a few fields falls back to a record (OL-307, OL-313) |
| `[olang]` String-prefix patterns in `match`, a placeholder in pipes, slices, `+=` | dispatch on a tag's prefix is a guard on every arm today (OL-292, OL-310, OL-309, OL-311) |
| `[olang]` Module aliases, and a warning when two `use m {}` namespaces collide | `use m as x` is refused; `core.search` and `face.search` bind one `search`, the later winning, with no warning (OL-312, OL-590) |
| `[tooling]` `olang fix records`, and an `olang fmt` that re-wraps at a width | olang's `tools/idioms/fix.ol` does the records rewrite today; fmt does not re-wrap (OL-588, OL-589) |


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
is: 23,970 `map_get` calls, 2,120 `map_get_or`, 11,452
`#{ "key": … }` literals, 4,114 `map_set`/`map_merge`, and six `type`
declarations in the whole corpus. Of the `map_get` calls, 18,485
(77%) read an identifier key off a name — `map_get(p, "profiles")` —
and 1,441 nest (`map_get(map_get(m, "pp"), "cur")`); about 4% use a
key computed at run time.

### The design (decided 2026-10-10)

**`.` reads a map's key, and `??` supplies a default.** Both are
additive: `.` on a Map is a type error today, and `??` does not parse.

1. `m.key` on a Map is exactly `map_get(m, "key")`: a missing key is
   `()`. Chains read nested maps (`m.pp.cur`). A key that is not an
   identifier, or is computed, stays `map_get(m, k)`. A key wins over
   a trait method of the same name, as a struct field already does
   (language.md, Traits).
2. `a ?? b` is `b` when `a` is `()`, else `a`; `b` is evaluated only
   when needed (as `&&` is). `map_get_or(o, "pad", 0)` becomes
   `o.pad ?? 0`, and `if x == () => d else => x` one expression. An
   operator, not a word, so the reserved list stays at fifteen.
3. Objects keep raising on a missing field. A map is an open bag
   whose keys may be absent; an object is a record whose fields are
   fixed. Nothing existing changes meaning.
4. A parsed `JsonObject` follows Map: `.` on a missing key is `()`
   (today it raises "has no field or method"). It carries outside
   data, where absence is ordinary. This one is a behaviour change and
   is ruled in language.md with the rest.

```olang no-run
// today
let live = if map_get(p, "running") == true => map_get(p, "live_id") else => ()
let clearable = len(filter(map_get(p, "profiles"), (e) => map_get(e, "id") != live))
let pad = map_get_or(opts, "pad", 0)

// with the design
let live = if p.running == true => p.live_id else => ()
let clearable = len(filter(p.profiles, (e) => e.id != live))
let pad = opts.pad ?? 0
```

Considered and not taken:

- **`.` raising on a missing key**, as an object's does. It would
  catch a typo at run time, but the migration could not be mechanical
  — each of 18,485 sites would need judging for whether its key can be
  absent, and optional keys are everywhere (2,120 `map_get_or`, and
  `== ()` checks through every framework's props). Typos are caught
  statically instead (E4): `.ident` is a key the checker can see,
  which a string argument is not.
- **Moving the frameworks to objects.** Every optional prop would need
  rewriting (an object raises on an absent field), and the Map and
  JsonObject plumbing would remain.
- **`?.` for optional chains.** `res?.field` already means "propagate
  the Result, then read the field"; the token is taken.

`.` stays read-only: olang has no field assignment, by design. Writes
get their own rows (E5, E6).

| Item | Evidence | Done when |
|---|---|---|
| E1 `[olang]` `.` reads a map's key | the design above; 18,485 `map_get(name, "ident")` calls. language.md "Anonymous objects" rules that maps are read with `map_get` only — this row reopens that ruling | field access on a Map (and a JsonObject) answers the key or `()` on the interpreter, the VM's field read and the JIT, pinned by the differential tests; the key-over-method rule pinned; language.md's ruling rewritten; pitfalls.md's "maps use `map_get`" and "Missing map keys return Unit" entries rewritten around `.` and `??` |
| E2 `[olang]` The `??` operator | the design above; 2,120 `map_get_or` calls and the `if x == () => d else => x` pattern | `a ?? b` in the grammar with its precedence stated in language.md (below the arithmetic operators, above the comparisons, as Kotlin's `?:`: `a.n ?? 0 > 3` is `(a.n ?? 0) > 3`), the right side evaluated only when the left is `()`, on every tier |
| E3 `[tooling]` The migration | 18,485 + 2,120 call sites across seven programs | a codemod written in olang over `meta.parse` (`olang fix maps`, or tools/) rewriting `map_get(chain, "ident")` → `chain.ident` and `map_get_or(x, "k", d)` → `x.k ?? d`, run over every application with its suite passing unchanged. The one inexact case — `map_get_or` returns a *stored* `()` where `??` takes the default — is flagged by the codemod, never rewritten |
| E4 `[tooling]` The checker sees keys | a misspelt key is `()` at run time (`p.runing`), and neither `olang check` nor the language server sees inside string keys, so the most common data shape gets the least checking. With E1 the key is syntax | `olang check` reports a `.key` read that no construction of that value can have (literal-keyed maps and objects, through `let`, parameters and returns within a module), with no false report over the workspace's corpus; the language server completes keys after `.` |
| E5 `[olang]` One record kind for the builtins | `map_len` and `map_merge` refuse an object and a parsed `JsonObject` ("argument must be a map", "first argument must be a map") while `map_get`/`map_set`/`map_keys` accept both — re-checked on 0.87.0. open-track desktop writes every server-record merge by hand | every `map_*` builtin accepts a Map, an Object and a JsonObject alike, on every tier, and stdlib.md's map table says so |
| E6 `[olang]` Writes: spread and nested updates | `#{ ...m, "a": 5 }` and `{ ...o, b: 2 }` are parse errors, and the error is wrong ("Unclosed `{` opened at line 6" for a closed brace); updating one key is `map_set(m, "k", v)`, a nested one `map_set(m, "pp", map_set(map_get(m, "pp"), "cur", x))` (94 sites written that way, 4,114 `map_set`/`map_merge` in all); `map_set_in` and `map_update` do not exist | `#{ ...m, "k": v }` and `{ ...o, k: v }` build a copy with the keys replaced (later wins); `map_set_in(m, path, v)`, `map_get_in(m, path)` and `map_update(m, k, f)`, sole-owner in place as `map_set` is; the parser names an unsupported form instead of an unclosed brace |

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
| `[olang]` A list element cannot be assigned | `xs[i] = v` is a parse error ("possible assignment in expression context"); a line edited in place rebuilds the list (Studio's replace across files, core/replace.ol, appends every line to a new list) | `xs[i] = v` on a `let mut` list (in place when the list is not shared), or a `list_set(xs, i, v)` builtin |
| `[olang]` No character from its code | `str.char_code` exists but its inverse does not: Loom's terminal (lib/vterm.ol) writes the C0 controls as a literal table of `\u{…}` escapes and the old mouse encoding's bytes through `json.parse("\"\\uXXXX\"")` | `str.from_char_code(n)` (and a list form), on every tier |
| `[olang]` `if a == (x, y) => return …` parses `(x, y) => …` as a lambda | Studio's terminal (face/termface.ol `tf_fit`): "expected an operator" at the next line; the tuple has to be bound first (`let want = (rows, cols)`) | a tuple after a comparison operator is an operand, or the error names the lambda the parser took it for |
| `[olang]` No `abs`, `all`, `any` among the global builtins | `min`/`max`/`clamp` are global but `abs` is not ("Undefined variable: abs" in face/termface.ol's wheel); `all`/`any` over a list with a predicate are written as `len(filter(…)) == 0` (lib/vterm.ol's digits) | `abs(n)`, `all(xs, f)`, `any(xs, f)` as globals, on every tier |
| `[olang]` A match on string literals is a linear chain | Studio's `sa_update` matches about 300 string arms; not shown to be a cost yet | a match of string-literal arms compiles to a table — only when a profile asks for it |

## Memory and start-up

| Item | Evidence | Done when |
|---|---|---|
| `[olang]` Every http worker compiles its own copy of the program | `runtime.memory()` shows each `olang-http-N` at 45–60 MB live once warm, against 13 MB for the whole process with the tier off: bytecode, constants and a bridge per VM. 18 default workers is about 1 GB, so open-track caps itself at 8 workers. open-track served is 330–445 MB resident against the 250 MB target (W21/W22: "not met"; the rest above the 97 MB heap is the small-object zones' freed pages) | immutable compiled code is shared across a process's threads (JIT module and inline caches per thread), per-worker live memory after warm-up is a few MB, and open-track served at 18 workers is under 250 MB resident |
| `[olang]` Session start under 100 ms for a large client | open-track's browser boot is 490 ms with no hot list; 360 ms of it is before the app's first statement and grows with program size, and nothing attributes it. The browser runtime is 0.97 MB brotli and cannot shrink further while the parser and compiler ship in it; a program image as bytecode was measured and left "a design for later" (W15 items 5–6) | first a boot profile that splits decode, declare, top level and first render for a real bundle; then what it names (the bytecode image, which would also let a browser profile drop the parser and compiler). Done: open-track's session start under 100 ms and the runtime under 400 KB |
| `[olang]` olang Studio.app's first frame is over 180 ms from its process's start | the bytecode is kept between runs now (the compile cache: Studio's ~545 compiles ~65 → ~8 ms warm; tools/launch.ol ~218 → ~173 ms, interleaved medians), and the app's first frame from its process's start went ~252 → ~215–230 ms. What remains on its path: the JIT's 26 groups (~15 ms) compile every launch — their code holds this run's addresses (helpers moved by ASLR, interned shapes, the group's other members) and is specialized on the kinds its first call saw (docs/ovm.md "The compile cache") — and the program now reaches `gui.open` (~195 ms) about when the platform's warm-up on the main thread (AppKit, a hidden window) ends (~150–210 ms, varying with the machine), so a faster program waits for it. A cold compile cache costs ~10% more compiling (~+7 ms) (Studio, 2026-10-10) | native code kept beside its bytecode entry (code and relocations written by symbol, patched into `MAP_JIT` memory on load, keyed by the entry, the kinds and the CPU features) or compiled off the program's thread, and the platform's warm-up off the first frame's path: olang Studio.app's first frame under 180 ms from its process's start |
| `[olang]` A freshly built application starts later under Launch Services | measured 2026-10-10 against tools/launch_app.sh's harness: `open` → process start is ~13–15 ms for an olang Studio.app that has been on disk a while and ~25 ms for one built in the last few minutes — the same bundle with the runtimes swapped follows the bundle, not the runtime, and a C executable in the same harness takes the same as olang's (the earlier "~30 vs ~13 ms" did not reproduce). After the process starts, olang reached `main` ~3.5 ms after a C executable linking the same frameworks, now as soon (chained fixups); its two static initializers (aws-lc's CPU probe and one C initializer) and its 17 libraries (all AppKit's own dependencies but libiconv) cost nothing measurable, and it has no Rust constructors | what the system does with a new bundle in its first minutes found (Gatekeeper's assessment, XProtect, Spotlight are candidates), and whether a build can avoid it; a launch measured on a bundle a few minutes old says so |
| `[olang]` What holds live memory, by site | growth under load was undiagnosable from open-track; `runtime.memory()` answers by owner, and `OLANG_ALLOC_TRACE` gives only large allocations, only in an `alloc-count` build | live bytes by allocation site or value kind in a release binary (`runtime.heap()`, or a dump at a signal). Low now the leak (OL-146) is fixed |

## Tooling

| Item | Evidence | Done when |
|---|---|---|
| `[tooling]` The checker misses a builtin's arity | `str.thousands(1234)` (it takes two) passes `olang check` in one file and across modules, and fails at run time (Loom) | `olang check` reports a wrong argument count to any builtin or stdlib function from the signature table; pinned |
| `[tooling]` The checker misses a parameter that shadows a stdlib module | `fn table_sort(rows, col, …)` checked clean and failed on `col.sort_by`; `fn f(str) = str.length("a")` is clean on 0.87.0. The `let` form already warns (Heddle) | the "shadows the stdlib module" advisory covers parameters and pattern bindings |
| `[tooling]` The language server ignores a file renamed | Studio's Rename and Move to Trash (its navigator's context menu) say `workspace/didRenameFiles` and close and reopen the open document under its new name; `olang lsp` drops the notification (src/tools/lsp.rs's notification arm), so a module importing the renamed file keeps a stale problem until it is edited, and `workspace/symbol` answers the old path until the next save | the server takes `didRenameFiles` (and `didDeleteFiles`): the documents re-keyed, the importers re-checked, symbols answered under the new paths |
| `[olang]` No `fs.real_path` | Studio's file operations must stay inside the folder opened with symbolic links resolved; `fs.abs_path` only normalizes `.`/`..` (`/tmp/x` stays `/tmp/x`, not `/private/tmp/x`), so core/fileops.ol runs `/bin/realpath` in its task (a process a check); Studio's terminal maps a link's file from the shell's real working directory (`pty.cwd`: `/private/var/…`) back onto the folder opened (`/var/…`) by its `/private` prefix (face/termface.ol `tf_alias`), or the file opened twice | `fs.real_path(p)` answers the path with its links resolved (its nearest existing folder made real when `p` does not exist yet, as the capability gate's `gate_path` does), on every platform |
| `[tooling]` A large file's problems arrive 630 ms after an edit | in a 50,000-line file a typed problem shows ~0.63 s later: every edit re-parses and re-analyses the whole file (Studio, tools/live_lsp.ol). `olang check .` over open-track takes 12 s, all of it the parse | the server checks by top-level item: an edit re-parses its item and the items naming what it declares |
| `[tooling]` Native code calling native code has no time of its own | callees in a caller's native group have counted calls (7d76560) but their time is the caller's (tooling.md), so Studio's tier map shows none for them | time attributed per callee (group entries counted in the JIT call stub, or members reported with their root) |
| `[tooling]` REPL command output is not deterministic | `:env` lists modules in hash order; `:help tutorials`, `:tutorial` and `:search` ties vary — three piped runs, three outputs. Studio's pinned transcripts leave them out | sorted (name, then score), and the full transcripts pinned in tests/repl_commands_test.rs |
| `[tooling]` A missing `}` in a map literal blames the next line | `#{ "a": 1` one brace short reports "expected an operator" at the following statement (Heddle) | a statement ending inside an open `#{` says so and points at the opening brace |
| `[otc]` A path dependency's own path dependencies are not followed | open-track desktop → Loom → Heddle, all by path: "Cannot find module 'heddle'" until the app named Heddle too; a→b→c reproduces it, though a's olang.lock lists `cc` among `bb`'s dependencies | a path dependency's `[dependencies]` resolve relative to its own manifest and are written to the lockfile; pinned with three packages |
| `[tooling]` Undocumented behaviour | `olang test` runs each file with the working directory set to its own; `str.char_code` exists and appears nowhere in docs/ (Heddle) | both documented (tooling.md, stdlib.md) |

## Standard library

| Item | Evidence | Done when |
|---|---|---|
| `[stdlib]` No process table and no signal by pid | Studio's process manager runs `/bin/ps -axo pid,ppid,pgid,pcpu,rss,etime,stat,command` and `lsof -a -d cwd` every 2 s for its children's CPU, memory, time and folder, and `/bin/kill -TERM -<pgid>` to stop one Loom did not start as a handle (core/procs.ol, face/autoface.ol); at quit it lists every descendant the same way | `os.processes()` (pid, parent, group, CPU, memory, start, state, command, folder) and `os.kill(pid, signal, #{ group })`, on every platform |
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

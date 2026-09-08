# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- **The route index keeps the first route at a path**, as the walk does:
  an app's page at `/` now answers under `serve` instead of the SDK's
  own shell, and sits in the app's table where a wrapper can gate it.
- **An imported module's shared functions and meta fns reach their
  private helpers at expansion time**: each module is loaded as a module
  of its own in the expansion interpreter, so a macro library's helpers
  no longer need to be shared under unique names. **A package module can
  use a sibling's macro** — its `use lib.x` resolves against the
  package root at expansion, from any working directory.
- **`serve` never hops a handler to a task under `olang test`**; only a
  direct `dispatch` does. A served test runs handlers on the worker, so
  `http.defer` holds a ticket for a real connection without clearing
  `OLANG_TEST`.

## [0.84.0] - 2026-09-07

### Added

- **`chan.ask(service, request)`**: the reply pattern in one call, spinning
  briefly before it parks. **`index_routes`** and an indexed `serve` table:
  an exact path is one lookup however many routes there are.
- **`data-on`** names the events an element's action fires on; the event
  payload carries `input_type`, and `checked` only for a checkbox or radio;
  `dom.checked` answers Unit for an element without a checked state.
- **`OLANG_ACCESS_LOG`** (all, errors, off) levels `serve`'s access line,
  and `"log": ()` silences it.
- The profiler reports threads parked in a channel receive, a sleep, a
  join, or an accept loop as blocked instead of charging the wait to the
  function that parked them.
- **`http.defer()` / `http.respond(ticket, response)`**: a handler that
  waits parks its connection and returns a ticket; the answer completes
  it from any thread. A long poll costs a socket, not a worker.
- **`olang check` knows declared enums and record shapes**: a `match`
  over a value of an `enum` type that misses a constructor is reported
  with the constructor's name; `{ name: Type }` annotations declare the
  keys a map or record carries, and a literal key the shape lacks is
  reported at `map_get`, field, and index sites with the nearest key.
- **`meta.exports(path)`** reads an imported module's literal top-level
  bindings at expansion time; **`meta.fresh`** names (`stem__m<N>`) are
  reserved — a program that spells one is refused before expansion.
- **Record/replay reaches the database and the other threads**: every
  `db.` call, every channel receive, `chan.ask`, `chan.send`, and
  `task.join` is in the trace; a replay opens no database and starts no
  worker.
- **The program image carries hot hints** (`meta.encode(src, #{ "hot":
  [...] })`): the browser runtime compiles the named functions at
  declaration, so the boot render runs on the VM. The web SDK's `serve`
  sends the app's client functions by default (`"hot"` overrides).
- **JIT raw int-list reads and writes are inline** for the list a region
  touched last: `sieve` 562 → 230 ms. **`text + to_string(n)` is one
  allocation** (`wordfreq` 615 → 508 ms), a constant range kernel is a
  fill, and a top-level `map`/`filter` over a range runs on the VM's
  range kernel instead of materializing the range (10 M elements: 1.6 s
  → 0.1 s).

### Changed

- **A function declared below its caller resolves at the entry file's top
  level**: declared on first use, one value ever. A module's code no
  longer captures the program root, so a package's private function is
  never shadowed by an app's same-named export, on any thread.
- **The map kinds compare by contents**: a parsed JSON object, an
  anonymous record, and a `#{}` map with the same keys and values are
  equal, at every depth.
- **The client bundler strips every package and project `use`** and keeps
  the browser's own modules; **the app's routes come before `serve`'s**, so
  a page at `/` is the app's; **a map is never a response** — only what
  `http.response` builds or a `{ status, body }` record is.
- A link carrying `data-action` no longer follows its `href` after the
  action (`data-follow` opts back in); a text box's `change` no longer
  fires its action — Enter and buttons do, `change` is for controls whose
  value is the argument.
- `attempt` runs its function on the VM's own tiers rather than bridging
  it to the interpreter; `term`'s color decision (`os.color_enabled()`,
  new) is made once and re-made only after the program changes its
  environment, `os.is_tty` once per process.
- **`actions(table)`** registers many actions in one update; **`drain_ms`**
  (an `http.serve` option and a `serve` key, default 5000) caps how long a
  drain waits before cutting the requests in flight.

### Changed

- **A draining server closes connections.** Every response it still
  answers says `Connection: close`, and it reads nothing more off a
  kept-alive connection, so a re-issued long poll cannot hold the drain.
- **`mount` adopts a server first paint** when the merged state is what
  the server rendered from: no render runs at boot.
- **A package's front door carries the macros its index imports** with a
  plain `use`, so `use shuttle` reaches `@resource`.

### Fixed

- **A host DOM call that throws no longer kills the session.** The shim
  catches it and the runtime raises it as an olang error the handler can
  `attempt` (`dom.query: '#wip-[]' is not a valid selector`).
- **A DOM event fired synchronously from inside a handler** (`dom.focus`
  → `focusin`, a blur's `change`) is queued and runs after the handler,
  never nested; a re-entered session is refused with a message instead
  of a panic.

## [0.83.0] - 2026-09-05

### Added

- **Graceful shutdown.** `os.on_shutdown(handler)` runs on SIGINT or
  SIGTERM; `http.shutdown()` drains every running `http.serve` (no new
  connections, in-flight requests finish, `serve` returns). The SDK's
  `serve` takes `"drain": true`, `"headers"` added to every response,
  `"trust_proxy"` for `req.remote_addr` behind a proxy, and a `"head"`
  that may be a function of the request (per-response CSP nonces).

- **The dom's other reads.** `dom.checked`, `dom.selection`,
  `dom.set_selection`, `dom.values`; and `"paste"`/`"drop"` events carry
  `files` read to base64, so paste-to-attach is olang.

- **Macros import from a library's front door.** `use web { when }` works:
  a module records the meta fns it declares or re-exports, and the
  runtime import of one is not a miss.

- **`olang test` runs an imported module's tests once per invocation**,
  and `otc install` says when a shelved starter is behind this toolchain.

- **`attempt(f)`** calls `f()` and answers `Ok(value)` or `Err(message)`
  for a runtime error, so a long-lived loop can answer 500 instead of
  ending the process; control flow passes through.

- **`==` across types answers.** Values of different types compare
  unequal instead of raising, on both tiers; Int and Float still compare
  numerically. `olang check` warns where the types are provably
  different.

- **`compress`.** `compress.gzip`, `gunzip`, `deflate`, `inflate`, and
  `gzip_level`, on every target. The SDK's `serve` gzips text responses
  of a kilobyte or more when the client accepts it (`"compress": false`
  turns it off).

- **`olang --in-task run` and `olang bench --in-task`** run a program on a
  spawned task — the context every http worker has — so a task's speed
  is pinned to the main thread's.

### Changed

- **Name resolution is lexical.** A function body resolves a name it does
  not bind in the scopes of its own definition, then in the complete
  top-level table of its file, then in the program's root; the frames
  that called it are never consulted. A library's local `head_for` can no
  longer capture an application function's reference to its own
  `head_for`, and an unresolved name is an error that names it. A nested
  `fn` captures its whole enclosing chain, so a parameter named like a
  builtin reaches the parameter when the function runs compiled or on a
  task. Module functions see every sibling, private or shared, however
  the file is ordered — from any thread and from the tier's bridge.

- **`http.serve` returns `Ok(())` on a drained stop**; the SDK's `serve`
  matches with a catch-all, so an app's "server drained" branch runs.

### Performance

- **Parsing is linear.** The parser asked pest for every span's line and
  column, and pest rescans the input on each ask: a 400 KB program took
  6 s to parse. Positions come from a line index now — 0.2 s for the
  same program, identical positions — which is what `olang check`,
  `olang test`, the LSP, and a browser's source fallback pay on a large
  file.

- **A browser profile of the runtime.** The wasm the web SDK and the
  examples ship omits the `re` module and the RSA suite (features
  `regex-module` and `rsa-crypto`, on by default natively): 6.3 → 4.8 MB
  raw, 1.84 → 1.38 MB gzip, 1.26 → 0.97 MB brotli. Calling one of the
  omitted functions says so. The website's playground keeps them.

- **The browser profiler.** `window.olangProfile.start()`, act,
  `table()`: every olang function that ran during a repaint, with its
  tier, calls, and exact self and total milliseconds, from the same
  shadow stack `olang profile` samples natively.

- **A repaint is a DOM diff, not markup.** `dom.patch(el, node)` hands the
  host the node tree as data and the host diffs it against the live DOM
  by `data-key`; the SDK's `mount`, `apply`, and `patch` use it, so a
  repaint renders, escapes, and parses no HTML. `memo(key, inputs,
  build)` keeps a subtree untouched while its inputs stand.

- **The VM's maps hash with FxHash**, as do its function and builtin name
  tables; `map_has_key` allocates nothing per call and `map_set` on a
  uniquely owned map overwrites in place. `to_string` on a scalar and
  `str.length` run natively instead of bridging to the interpreter.
  `wordfreq` 1,108 → 650 ms, `strbuild` 266 → 116 ms, the record
  pipeline 13 → 4 ms.

- **A hot inner loop enters the native tier at its own head** instead of
  waiting for the enclosing loop's back edge; `map`/`filter` over a
  range run the compiled kernel's raw entry per integer and land in the
  typed list layout, and `+` keeps a typed list typed. `sieve` 1,098 →
  550 ms.

### Fixed

- **A second definition of a name no longer slows every call to it.** The
  tier's ambiguity verdict reaches the compiler, which resolves the name
  through the caller's own closure: 58 ms → 4 ms for 4,000 calls through
  a by-name import that shared its name with another module's private
  function.

- **A lib function reached inside a task compiles as it does inline.** A
  worker's tier inherits the parent's function table and ambiguity
  verdicts: 20,000 by-name calls cost 18 ms inline and 18 ms in a task
  (was 2,207 ms).

- **`serve`'s `"headers"` keeps a streamed file response intact**: the
  merged headers are set on the response as it is, so `body_file` routes
  (the hashed wasm included) no longer answer 500 when headers are
  configured.

- **A lib-module call inside a task no longer rebuilds an interpreter per
  builtin call.** The bridge's capability re-seed dropped its cached
  interpreter on every dispatch; it is dropped only when the grant
  changes. 2,402 ms → 546 ms for 20,000 calls through a function value
  in a spawned task; the residual is the call running interpreted.

- **A captured binding named like a global builtin** (`let head = (r) =>
  …`, then `head(r)` inside a function) reached the builtin under the
  bytecode tier; it now shadows it, as the interpreter always did.

- **`task.watch(t, c)`.** The channel dies with the task: when `t` ends,
  however it ends, `c` is closed and the next `chan.recv` returns `Err`
  instead of waiting forever. Ownership is declared, not inferred.

### Changed

- **The expansion scope is a stated list** (docs/macros.md), pinned by a
  conformance test: a file's own functions and meta fns, the `share fn`s
  of the modules it imports, the pure stdlib, nothing else. The file's
  own functions had been claimed and were not loaded; they are now.
- **A test block's statements carry their positions**, so a lint or a
  failure inside one names its own line.
- **A value called inside a compiled function names the identifier**:
  `'span' is an Int, not a function`, the interpreter's exact wording,
  from the VM too.
- **Naming conventions** for the standard library are written down
  (`try_` twins, unit suffixes, one definition per name), with the
  0.82 audit's findings beside them.

## [0.82.0] - 2026-09-05

### Changed — two rulings from the roadmap's W2

- **A Float is always finite.** Overflow now raises like division by
  zero and out-of-domain math already did: `1e308 * 10.0` is
  `Float overflow in multiplication (the result is not a finite
  number)` on all three tiers (the bytecode fast path and the JIT check
  every float result and deopt to the same error), `math.pow`, `math.exp`
  and the rest error instead of returning `inf`, `str.parse_float("1e999")`
  and `to_float` report "out of range", and a literal beyond f64's range
  (`1e400`) is a syntax error. The data stack's Series kernels stay
  IEEE for throughput and are documented as the one place `inf` can
  appear.

- **Series division is true division.** `/` between two Int Series, or
  an Int Series and an Int, yields a Float Series — the scalar `/`
  still truncates. A ratio between measure columns is what data code
  means, and the truncating quotient had turned the climate example's
  growth rates into zeros with no error. `ods.cast(a / b, "int")`
  recovers the integer quotient; `+ - *` on Int Series stay Int and
  checked; division by zero still raises.

### Added

- **The program image.** `meta.encode(source)` returns the parsed
  program (macros expanded) as bytes behind a header naming the olang
  version that wrote it — an image the runtime loads without parsing
  (`src/olb.rs`, postcard). The web SDK's `serve` encodes the client
  bundle once at boot and serves it at `/app.olb` beside the source;
  the shim loads it through the wasm's new `olang_session_start_bin`
  entry and falls back to `/app.ol` when the runtime is another version.
  On the scaffold app the load step drops from 22.2 ms of parsing to
  0.9 ms, and the image is 17.7 KB to the source's 31.4 KB.

- **A server-rendered first paint.** `serve` takes `view` and `initial`
  (a value, or a `() => state` function evaluated per request), renders
  the view into the mount point on the server, and ships the state in a
  JSON `<script>` the mount point names; `mount` starts the store from
  that state, the server's keys over the caller's defaults. The page
  shows its data before the runtime has downloaded, and the boot needs
  no fetch. `otc new --web` now scaffolds three files — `lib/pages.ol`
  holds the view both halves render.

- **Boot phases, split.** The session result and `window.olangBoot`
  carry `program` ("image" or "source"), `load_ms`, and `run_ms`.

- **A Bytes body in an http response** is served raw — the binary-safe
  path that needs no file on disk (`body_file` remains).

- **`str.fixed(x, digits)` and `str.thousands(x, digits)`.** Exactly
  `digits` decimals, never exponent form, never `-0.00`; the second adds
  thousands separators. The column and money form `to_string` is not.

- **`db.transaction(conn, f)`.** `f(conn)` inside a transaction: commit
  unless `f` returns an `Err` or raises, which roll back; `f`'s result
  is handed through. The web SDK's `tx` now is this call.

- **`db.migrate(conn, steps)`.** The versioned-migration engine the
  example apps and the web SDK each carried, in the stdlib: a
  `schema_version` table, one transaction per version, the failing
  statement named. All three copies became callers.

- **`dom.request(method, path, body, k)`.** The whole response —
  `status`, `headers`, `body` — so a handler can tell a 404 from a 500
  from a network failure (status 0). The SDK's `api.call` rides it: a
  response that is not the envelope is `Err(#{ "message": "HTTP 500:
  …", "status": 500 })`, not a JSON parse failure.

- **`ods.try_series`, `ods.try_frame_from_records`, `viz.check`.** The
  Result forms of the constructors that raise on mixed-type data, and
  a spec check (`Ok(spec)` or `Err`: an unknown key, no data points) to
  guard a chart over records from a file, a request, or a user.

- **`db.query_one`'s absent shape is stated**: `Ok(())`, unambiguous
  because a row is always a map.

- **W11, the second reading of open-track.** A test block is its own
  scope. A path import (`use lib.csv`) wins over a stdlib module of the
  same name. A manifest dependency that cannot be resolved no longer
  fails every module: the resolvable ones load, and the `use` that needs
  the missing one names the package and the reason. A task that dies by
  raising says so on stderr. `dates.stamp_ms()`. `http.serve` takes
  `"bind"`. `dom.find` (Unit on a miss), `dom.query_all`, and
  `dom.request_with` (request headers). `olang check` warns about a
  parameter that shadows a function its body calls and about a binding
  that takes a stdlib module's name; an interpreted frame's "is an Int,
  not a function" names the identifier being called in every call shape. Call-stack frames from
  another file name it. The lockfile records shelf dependencies by name,
  and a locked path this machine lacks re-resolves. Web SDK:
  `not_found(what)`, `void_el(tag, attrs)`, the `@when(cond, node)`
  macro, `configure(#{ "headers": … })`, `serve`'s `"bind"` and
  `"sdk_dir"`, `sdk_dir()` reading the project's lock, and `dispatch`
  running handlers on a task thread under `olang test`.

- **The rest of the board.** Repaints reconcile: `dom.morph(el, html)`
  morphs markup into an element (text in place, attributes diffed,
  children by `data-key`), and the web SDK's `rerender` and `patch` use
  it, so focus, caret, and scroll survive a store write. Meta fn bodies
  reach the `share fn`s of imported modules. Interactive line charts
  carry vertex circles with their datum. `testing.snapshot(name,
  value)`, `olang test --watch`, `olang check --fix`. The `vec` module
  (`dot`, `add`, `sub`, `scale`, `sum`, `norm`, `mean`) and
  `ods.to_matrix`; `ods.date_part`, `ods.split`, `ods.split_at`. A
  runtime error carries one "Runtime error:" prefix however many layers
  wrapped it, and an error inside a compiled function is placed in that
  function's file.

### Changed

- **The wasm ships without symbol names.** wasm32 builds pass
  `-C strip=symbols`: the `name` section was 612 KB of a 6.7 MB
  artifact. 6,746,426 → 6,266,887 bytes with the image entry point
  added.

### Fixed

- **`action_arg` on a confirm-wrapped action.** `btn_confirm` renders
  `confirm:<action>`; `action_arg` took everything after the first colon
  and handed the confirmed handler `"del:7"` instead of `"7"`. The
  prefix is dropped first.

- **`confirm_armed` without a DOM** read a main-thread cell, which a
  view rendered on a server worker thread cannot do. Without a DOM
  nothing is armed.

- **A trap in the wasm names its panic.** The SDK shim reads the
  runtime's panic message (kept by the panic hook, read through
  `olang_last_panic`) when a call into the wasm traps and prints it
  before the bare `RuntimeError: unreachable` propagates.


- **The content-addressed wasm URL now serves someone.** The shim read
  its `data-src`/`data-wasm` attributes off `document.currentScript`,
  which is null inside a module script — so every attribute silently
  fell back: the program path happened to match the default, and the
  hashed runtime URL was never fetched (a preload for it downloaded the
  runtime twice). The shim now finds its own `<script>` element by src,
  and the shell preloads exactly the URL the shim fetches.

- **The browser parses the bundle once.** `serve` pre-expands the client
  bundle on the server (the new `meta.expand(source)`, `olang expand`
  as a value), so a bundle carrying `meta fn` declarations and `@` sites
  no longer costs the wasm parser the expansion pipeline's repeated
  complete passes at boot — parsing had become the whole of a 4.5 s
  session start once the download was solved. Independently, `Parser`
  flags macro constructs as it builds the tree instead of serializing
  the whole program to JSON to look for them, and a program that
  declares macros but never invokes one skips the expansion round.

## [0.81.0] - 2026-09-02

### Added

- **W9 — what building open-track asked for.** The first large program
  written against olang's packages from outside the repository produced
  forty-six rows (docs/roadmap.md, W9); this release lands them.

  *Wrong answers and divergence.* The module cache is keyed by the
  resolved file, not the dotted name, and a module inside a dependency
  resolves its own `use lib.x` against its package: two packages that
  each have a `lib.util` never hand one another's module out, and a
  dependency's internal imports no longer depend on the order its
  `index.ol` happened to import them (tests/module_resolution_test.rs).
  `x |> <any callable expression>` works in the interpreter as it
  always did in the bytecode tier — the same program no longer ran in
  the browser and failed natively. `to_string` is the identity on
  strings (the quoted form was the show/debug shape; generic code that
  stringified map values grew quotes and stopped comparing equal;
  `meta.lit` renders the literal form). `col.any`/`col.all` from a hot
  function ran 3.5× slower with the tier on than off: a lambda crossing
  the bridge got a fresh closure Arc per crossing, so the bridge
  recompiled it on every callback — both `col.any`/`col.all` now loop
  natively like `map`/`filter`, and a closure's interpreter form is
  built once; the shared builtin registry is an `Arc` instead of a
  per-call clone. `meta.eval(src, #{ "max_steps": n, "timeout_ms": ms
  })` bounds untrusted source; a runaway rule is one
  `Err("budget exceeded …")`, not a hung thread. And a closure that
  referenced a top-level `let` declared after it failed "Undefined
  variable" only with the tier on (so, only in the browser): a function
  value the compiler declined ran on the bridge interpreter, which knew
  the program's functions but not its globals — the bridge is now seeded
  with the host program's top-level values.

  *Errors that point the wrong way.* An error raised inside an imported
  module reports the module's own file and line (the importer's `use`
  line was blamed); a bare `share` — `share meta fn` — is a parse error
  naming the six declarations it may precede and that a meta fn is
  exported by being declared; macro splice failures lead with the parse
  error and mention a trailing `//` comment only when one is present;
  identifiers may begin with `_` (`_tmp`, `__gen0` — a lone `_` stays
  the wildcard, and `spawn`/`meta` are bounded so `spawn_cost` is a name); `max`/`min` accept two or more scalars; `olang test`
  names the top-level statement that failed outside any test block.

  *Modules, macros, and the project tool.* A package's macros travel
  through its `index.ol` re-exports, and the expander resolves manifest
  path and shelf dependencies (`use web` reaches `@store`). `olang eval
  '<source>'` evaluates a one-liner with the working directory's project
  libraries in scope, and `olang <file>` outside any project falls back
  to the working directory's for shelf lookups. `otc new` derives the
  import identifier from a hyphenated name and prints it, refuses names
  that cannot be identifiers, scaffolds `--web` on the web SDK (two
  files; `--web-bare` keeps the raw shape), and says `git init` when
  the new project is outside a repository.

  *Standard library.* `map_path(m, keys)` for nested reads; `drop` as
  `skip`'s alias and every list helper mirrored under `col.` (`col.take`,
  …) with `col.index_of` and `col.slice`; `dates.stamp()` as the storage
  timestamp (UTC, second precision, `Z`); `validate` accepts parsed JSON
  objects, a `"date"` kind, and `"where": (v) => Result`; `viz` accepts
  `w`/`h`, passes `font_size`/`font` to plot, and refuses unknown spec
  keys; `dom.prefers_dark()`, `dom.active_id()`, `dom.confirm(msg)`, and
  `dom.read_file(el, k)`; the shelf's markdown renders task lists as
  checkboxes.

  *The web SDK.* An input's Enter no longer double-fires through the
  `change` it produces; `lib.sql.row` is exported as `one` (and `olang
  check` warns when a module re-exports one name twice); `try_rows`/
  `try_exec`; `patch(id, node)` repaints one subtree; `btn_confirm`
  two-step buttons; `serve` takes `"log"` and a list of client files,
  serves the runtime at a content-addressed immutable URL with
  `Accept-Encoding` negotiation over pre-compressed siblings, and the
  shim paints the shell before running the bundle and reports boot
  phases in `window.olangBoot`; web.css supports `data-theme`; the viz
  tooltip reads the design tokens.

  *Documentation.* "Sharing state under `http.serve`": the channel-
  service recipe that replaces a module-level cell.

## [0.80.0] - 2026-08-31

### Added

- **Template-literal lints.** The semantics stay (backtick templates
  process no escapes and do not nest — ruled after the W8 triage), and
  the two mistakes are caught before they print. `olang check` warns
  on an escape-looking `\n`, `\t`, or `\r` inside a template,
  positioned at the exact backslash; the lint reads the source
  spelling, so writing `\\n` — the same two output characters — marks
  the backslash deliberate and stays silent, and escapes inside a
  `${...}`'s own double-quoted strings are real string syntax and
  exempt. The parse errors a backtick inside `${...}` produces (both
  the truncated-template shape and a genuinely unclosed `${`) now say
  that templates do not nest and to bind the inner template to a name
  first. Advisory warnings; runtime behavior is unchanged, and the
  whole repository's own `.ol` sources check clean.

- **`map_get` ruled and documented as the raw read.** The W8 decision
  row closed: `map_get` keeps its Unit-on-missing shape, and
  `map_get_or(m, key, default)` (in the language since 0.69) is the
  blessed lookup-with-default, now documented beside `map_get` in the
  language reference rather than only in the pitfalls chapter.

- **`assert_close(actual, expected, tolerance, message?)`** — the
  tolerance-based numeric assertion beside the existing asserts, with
  the same everywhere-an-expression reach (match arms, lambda bodies,
  `test` blocks) and the same raising semantics. `Int` and `Float` mix
  freely, a `NaN` on either side fails with the difference shown, and
  a negative tolerance is an error. The default failure message names
  both values, the tolerance, and the actual difference. The crimes
  estimator tests adopted it, replacing boolean threshold checks.

- **Error reports are byte-identical on every tier, and richer.** The
  tree_train incident (a division by zero attributed to an unrelated
  top-level line) came down to the bytecode emitter leaking one
  function's span rows into the next compile: a dependency compiled
  mid-way inherited its caller's table, so its errors pointed at
  another function's lines. Hunting it with a differential suite of
  seven acceleration shapes (JIT call chains, mid-function loop
  promotion, OSR regions, fused writes, promoted top-level loops,
  bridged kernels, deep stacks) surfaced three more losses, all
  fixed: an error inside a hot `map`/`filter` kernel dropped its span
  and frames entirely; a bridged lambda's error lost its location
  crossing the tier boundary as a string; and the interpreter itself
  captured call stacks only after unwinding had popped the deep
  frames, so `drive → level1 → level2 → level3` reported as `drive`.
  Stacks now record at raise depth on both tiers, tier frames splice
  with boundary dedup, and a nameless function renders as `<lambda>`
  in traces everywhere. tests/error_span_test.rs pins the whole
  report — message, span, and stack — byte for byte.

- **Import aliasing, and the shadowing trap warned about.**
  `use lib.report { table as md_table }` binds the export under the
  alias (and only the alias); `share use m { name as alias }`
  re-exports under it. `as` is contextual — a keyword only inside a
  `use` list. The companion fix: `olang check` now warns when a
  wildcard import (`use term`, or `{ * }`) also exports a name an
  earlier explicit import bound — the runtime silently rebinds, and
  the failure used to surface at a distance inside whichever call got
  the wrong binding. The warning names both lines and both fixes
  (qualify, or alias), resolves user modules from disk and stdlib
  packages from the embedded registry, and stays advisory. The
  language chapter also corrects a wrong claim: bare `use m` was
  documented as namespace-only, but it has always been wildcard sugar.

- **Default parameter values work everywhere, correctly, at speed.**
  Defaults and named arguments were parsed and half-implemented;
  landing the roadmap row meant three fixes. Semantics: a default now
  evaluates at call time in the function's own scope — the closure
  plus every parameter to its left, as if it were the first statement
  of the body — never the caller's environment. (Previously it
  evaluated in the caller's scope: `fn mid(x, y = x * 2)` called as
  `mid(10)` with a caller-side `x = 99` silently produced 208 — a
  live dynamic-scoping leak, now impossible, and
  parameter-referencing defaults like `hi = lo + 10` now work.)
  Speed: defaulted functions were blanket-rejected from promotion, so
  one default cost 650× on a hot call (1939 ms vs 3 ms on a 2M-call
  loop). The interpreter now fills defaults at the call boundary
  before the tier is offered the call, defaulted functions promote
  and JIT like any other, compiled call sites splice literal defaults
  as constants (the 2M-call loop runs 5 ms), and non-literal
  defaults take the interpreter-exact function-value path. An
  eight-way differential battery (tests/default_params_test.rs) pins
  scope, evaluation count, named-argument resolution, recursion,
  errors, and the hot paths; the language reference documents the
  semantics, and the `_with` callback stubs in crimes/lib/ml.ol are
  replaced by defaulted parameters with k-means confirmed still fully
  native.

- **The `~/.olang` contract, `otc doctor`, `otc clean`.** Everything
  the toolchain writes under the user's home is now defined in one
  runtime module (`src/home.rs`) with a stated layout — shims,
  toolchains, shelf, prunable cache, regenerable state — and
  `OLANG_HOME` relocates the whole tree. `otc doctor` audits an
  installation: every `olang`/`otc` on PATH with version and origin
  and which copy wins, olang/otc version agreement, legacy scaffolding
  from older layouts, dead shelf entries, and the sizes of the
  prunable parts; `--fix` applies the safe repairs (it found and
  removed six empty directories, two legacy wrapper scripts, a stray
  playground wasm, and an orphaned cache on the machine it was built
  on). `otc clean` reclaims cache and state with sizes reported. The
  setup script no longer scaffolds directories; REPL history migrates
  from `~/.olang_history` to `~/.olang/state/history` automatically.
- **`otc update` and side-by-side toolchains.** `otc update` installs
  the latest release — downloaded from the project's releases,
  verified against `SHA256SUMS`, staged, smoke-tested, and switched to
  by atomically relinking the shims in `~/.olang/bin`, so the running
  binaries are never overwritten. `--check` reports without
  installing; nothing contacts the network except these explicit
  invocations. `otc toolchain list/install/default/remove` keeps
  multiple releases side by side (removing the default refuses).
  Installations owned by Homebrew or cargo are reported by doctor,
  never modified.
- **REPL bracket assistance.** The line editor hints the closing
  brackets for whatever is still open — `(map [1, 2` shows `])` as
  ghost text, accepted with the Right arrow — suppressed inside
  unterminated strings, where a bracket is content. When the cursor
  sits on or after a bracket, its partner is underlined, with string-
  and comment-aware matching. Both ride the same delimiter scanner as
  the continuation prompt.
- **`completions` for both tools** (`olang completions zsh`,
  `otc completions zsh`; also bash, fish, elvish), and
  `olang --version --verbose` reports which binary answered and from
  where — the question behind most stale-version confusion, answered
  in full by `otc doctor`.

## [0.79.0] - 2026-08-30

### Added

- **Sole-owner map writes: `m = map_set(m, k, v)` is in place now.**
  The rebind form compiles to a fused `MapSetAssign` that takes the
  map out of its register and inserts through `Arc::get_mut` when the
  register held the only reference — the map counterpart of the list
  writes, and the fix for the quadratic hash-count loop the
  cross-language wordfreq benchmark exposed. The interpreter's
  move-call fusion admits the map_set builtin with the same
  discipline, so both tiers are linear. Shadowing stays exact: a
  local named map_set declines the fusion at compile time, a known
  user function resolves as a call, and the instruction re-checks the
  builtin's presence at run time, falling back to CallNamed's own
  resolution. wordfreq: DNF (>120 s) → 1.19 s — from unrunnable to
  faster than R, within 2× of Python's dict, with Lua/Node/compiled
  languages still ahead. Differentials pin value semantics (an alias
  kept before the write never sees it), the counting loop, struct
  receivers, and the shadow.

- **The cross-language benchmark suite, and what it found.**
  `benchmarks/xlang/` runs eight benchmarks — recursion, integer
  loops, a sieve, n-body physics, dense matrix multiply, string
  building, hash-map counting, k-means — identically in olang, C++,
  Rust, Go, Java, Node, Lua, Python, and R, each printing a CHECK
  value the runner validates across languages (float benchmarks use
  explicit temporaries and `-ffp-contract=off`, so even the chaotic
  n-body energy agrees bit for bit) and a self-timed MS the runner
  medians. Building it surfaced and fixed three engine issues:
  - **A tier divergence in the move-call fusion.** `m = map_set(m,
    k, map_get(m, k) + 1)` moved `m` into argument 0 before the later
    argument read it — Unit on the VM where the interpreter read the
    live map. The fusion now declines when any later argument
    references the target (`references_name`, conservative on
    unmodeled forms); pinned by a differential.
  - **Domain-constrained math kept whole loops off native.** One
    `math.sqrt` in a loop body excluded the entire region from the
    JIT. sqrt/asin/acos/ln/log2/log10 now compile behind a domain
    guard — an out-of-domain argument branches to deopt and the VM
    re-run raises the interpreter's exact error. The n-body loop went
    140 s → 0.65 s; domain errors stay byte-identical across tiers.
  - **Boxed scalar lists at OSR entry now convert to the typed
    layout once**, at the marshal, and are written back — a sieve
    whose list was built by `map(...) + [0]` (boxed) ran its marking
    loop through boxed writes; it now runs native (9.9 s → 1.1 s).
  Whitelist refusals also name their instruction under
  OLANG_JIT_DEBUG now. One limit was found and recorded rather than
  papered over: hot string-keyed map accumulation is quadratic in
  every current idiom (the builtin `map_set` clones per insert even
  when the moved argument is sole-owner; `collections.table` pays
  the tier boundary once its caller promotes) — wordfreq reports DNF
  for olang and the roadmap carries the row.

- **The crimes workstream becomes a full research report.** The
  Chicago dogfood now runs fourteen stages and writes a structured
  analysis (abstract, methods, findings, limitations) rather than a
  chart dump. New, all in olang: trend-plus-seasonal decomposition
  with the autocorrelation function (ACF(12)=0.91 on 288 months);
  Wilson intervals on per-category arrest rates; Cramér's V beside
  the chi-square; PCA by power iteration over the standardized
  design matrix; a k-means elbow sweep justifying k; and three
  classifiers on one deterministic split — logistic regression,
  gaussian naive Bayes in closed form, and a histogram CART (24-bin
  splits, flat-arena nodes) — compared by AUC with Hanley–McNeil
  95% intervals at F1-optimal operating points, with a reliability
  diagram and concordant feature importance across model families.
  The tree edges gradient descent (AUC 0.707 vs 0.703) and trains in
  ~4 s on 305k rows; the whole 8.6M-row run stays ~22 s and passes
  under `--verify-tiers`. One real bug found on real data: a CART
  cut reconstructed at a bin's upper edge sent every row left when
  the edge coincided with the feature maximum — the partition now
  uses the bin semantics' strict inequality, and empty-side splits
  are refused outright.

- **The engine keeps proving itself (W7): live tier verification, an
  HTTP abuse gauntlet, and a Linux pass.**
  - `--verify-tiers <rate>`: the differential suites prove tier
    agreement over the programs they contain; this flag proves it over
    the program actually running. At the sampled rate, each
    native-tier result — a compiled call or an OSR loop region — is
    re-executed on the bytecode VM with the same inputs and compared
    bit-for-bit, which is sound live because the JIT whitelist admits
    only code that is pure with respect to caller-visible state. A
    divergence aborts (exit 102) with both renderings clipped to
    readable length; `:ovm` reports the session's verified-call count.
    Rate 1 roughly doubles native work (kmeans probe 125 → 240 ms);
    0.05 costs ~14%. `OLANG_VERIFY_SELFTEST` forces a divergence so
    the scream path itself is testable end to end.
  - **`http.serve` hardening.** The limits are explicit options now —
    `max_header_bytes` (default 64 KB), `max_body_bytes` (10 MB), and
    `request_timeout_ms` (30 s), the whole-request deadline that stops
    a client dribbling one byte at a time from holding a worker
    forever, which per-read timeouts alone cannot. Abuse is answered
    precisely, with the connection closed: 400 for a malformed request
    line or an unparseable `Content-Length` (previously a silent zero
    that desynchronized the kept-alive stream), 431 past the header
    cap, 413 past the body cap, 408 past the deadline. Header values a
    handler returns are flattened to one line, so interpolating
    untrusted text into a header can no longer split the response or
    inject headers. `tests/http_abuse_test.rs` runs the gauntlet —
    garbage bytes, oversized blocks, bad lengths, an injection
    attempt, a dripping client — against one server and proves a
    well-formed request still works after all of it.
  - **A documented Linux verification pass.** `dist/linux-verify.sh`
    runs the full workspace suite and the examples harness inside a
    Linux container against the working tree, building into a cached
    container volume so the host is untouched; RELEASING.md carries it
    in the gate list. Architecture-honest: on Apple silicon it
    verifies linux/aarch64.

- **Concurrency you can see into (W6): the stall detector,
  `task.list()`, `task.parked()`, and `chan.stat(c)`.** A `chan.recv`
  no live thread could ever satisfy used to hang the program forever,
  silently. Now the runtime proves the deadlock and says so. Every
  thread that runs olang code — main, `spawn` workers, http workers,
  parallel-map workers — is censused; every unbounded wait (a
  blocking recv, a send against a full bounded channel, a
  `task.join`) is a registered parked site, and blocking waits poll
  under the hood so a parked thread can sample the world. When two
  consecutive quarter-second samples agree that every censused thread
  is parked — the park/unpark generation counter unchanged, parked
  equal to live — no internal wake is possible and none can arrive,
  and the program aborts (exit 101) with a report naming each blocked
  site by thread, operation, and channel. The proof is conservative
  by construction: a computing, sleeping, or timeout-bounded thread
  counts live but never parked, and a running `http.serve` disables
  the abort entirely because an external request can wake a worker.
  `OLANG_STALL_ABORT=0` restores the hang for embedding hosts.
  Introspection rides the same bookkeeping: `task.list()` reports
  every watched task's id, state, and elapsed time; `task.parked()`
  is the live view of blocked sites; `chan.stat(c)` snapshots a
  channel's queue depth, closed flag, and waiting counts. Seven
  subprocess tests pin the detector's two obligations — genuine
  deadlocks die fast with the right sites named, and anything that
  can still progress (a late sender, a rendezvous handoff, a clean
  close-and-drain) is never touched.

- **The logistic forward pass runs native: nested-list reads and the
  fused scalar append.** The last two ML-loop gates fell together.
  - A new `ListAppendAssign` instruction: the optimizer rewrites the
    accumulate idiom's pair — a one-element `MakeList` feeding
    `AddAssign` — into one instruction that appends the scalar
    directly, so the per-iteration wrapper list never exists on any
    tier. The VM arm mirrors `x = x + [v]` byte for byte (in-place
    under the sole-owner discipline, demotion on a mismatched
    element, the ordinary binary error off a non-list), and adds the
    empty-accumulator convention: `[]` carries no element type, so
    the first scalar append chooses the typed layout — invisible, and
    the list a loop grows from nothing is a `Vec<f64>` from element
    one. The JIT lowers the append through ownership-family helpers
    (a caller's list copies once, the region-born copy pushes in
    place; an Arc's pointer survives the Vec regrowing, so the
    rebound register stays honest).
  - A `ListListFloat` kind: a list whose elements are all typed float
    lists — the feature-matrix shape — crosses the boundary as a
    borrowed pointer, and `cols[j][i]` compiles to an inner-pointer
    extraction (0 deopts: out of bounds, or an element demoted out of
    the typed layout) followed by the ordinary raw read. Read-only by
    construction: the inner pointer resolves in no ownership family,
    so any write through it deopts rather than aliasing.
  The forward-pass probe (200k rows × 8 features, with a compiled
  activation call and the growing predictions list) runs 69 ms → 14
  ms; the full logistic-training probe drops 112 ms → 21 ms with both
  the forward and gradient loops entering natively every epoch — all
  bit-identical to the interpreter. The Chicago-crimes workstream's
  training stage (305k rows × 11 features × 100 epochs) drops 28.4 s
  → 5.6 s, taking the whole 8.6M-row workstream to 17.5 s end to end
  with every stage statistic unchanged — from ~131 s when this
  campaign began.

- **Every hot loop reaches native, and liveness stopped lying.** The
  k-means gate fell, and not where the map said it would: the reused
  register carrying `K_INT|K_UNIT` was never genuinely live — an
  unmodeled instruction (`MakeRange`, rebuilding the inner loop's
  range each outer iteration) sat on the path after the loop's exit,
  and the backward liveness treated anything it didn't model as
  "everything is live", dragging every dead temp, including the
  `if`-statement's Int-or-Unit result, into the OSR live-out marshal.
  Four fixes, each pinned by the k-means differential:
  - The uses/defs register model is now **total and single-sourced**:
    one exhaustive match in the optimizer covers every instruction
    (a new variant is a compile error there, not a silent liveness
    pessimization), and the JIT's copy of the model — which was
    missing `ListSetAssign` among others — now delegates to it. OSR
    live-in/live-out sets shrank from "everything ever touched"
    (22 in / 18 out on the k-means scan) to the true loop state
    (8 in / 1 out).
  - **One OSR region per loop head, not per function.** Regions key
    on `(function, head)`; offers fire at every back-edge-threshold
    multiple rather than exactly once, and a head that has entered
    natively re-enters on its next back edge. A function whose outer
    iteration drives several sequential hot loops — assign, then
    accumulate, then recount — runs each of them native on every
    pass instead of one of them, once.
  - **Multi-list live-outs marshal home.** A region writing several
    lists returns them as a tuple of raw pointers: the `MakeTuple`
    inference arm now admits raw typed-list elements, the tuple
    write-back gate accepts them (they resolve through the ownership
    families, so the write-back is faithful or refused, never
    garbled), and the alias scan's epilogue exemption now states the
    real property — nothing but Nops and the Return after the tuple —
    instead of a jump-target inequality that misfired on the exact
    jump every synthesized exit takes.
  - A finalize refusal now prints the offending registers and the
    instructions that define them, which is how a wrong hypothesis
    (SSA-style register splitting) died in minutes.
  The k-means probe (200k points, k=10, 12 iterations) runs 1460 ms →
  121 ms, bit-identical to the interpreter, with all three of its hot
  loops entering natively on all 12 iterations. The Chicago-crimes
  workstream (8.6M rows) drops ~131 s → 39.5 s end to end with every
  stage statistic unchanged; its k-means stage clusters 900k points
  in under a second, leaving logistic training as the one hot stage
  still on the VM. The DP4 benchmark and micro set are unchanged.
  Still open for the full ML-loop win: the nested-list kind
  (`cols[j][i]`) and native fused list append for the logistic
  forward pass.

- **Native list writes: `col.set` lowers to machine code.** The raw
  typed-list kinds gained their write half. `ListSetAssign` on a
  `ListFloatRaw`/`ListIntRaw` register compiles to a helper call that
  enforces the language's value semantics through an ownership
  oracle: the ScratchCtx's Arc families now distinguish caller-owned
  lists (`*_args` — also the deopt state) from region-born ones
  (`*_allocs`), so a first write to a caller's list copies it into
  the alloc family and every subsequent write mutates in place —
  a rewrite loop is O(n) with one copy, exactly the VM's
  Arc-clone-on-shared behavior, and the caller's binding is
  provably untouched on deopt. A static alias scan gates admission
  (a written register that is also moved, passed to a call, or
  packed into a list/map refuses the region), out-of-bounds or
  unknown pointers deopt cleanly, and MAX_TUPLE rises 16 → 24 so
  write-heavy regions can marshal their live-outs. A 40M-op
  read+write loop over a 2M-float list runs in 493 ms native. The
  k-means assignment loop still refuses, and the refusal now names
  the precise next gate: a reused register carries mask
  `K_INT|K_UNIT` (an if-without-else Unit result sharing a register
  with an Int temp), which flow-insensitive inference cannot split —
  SSA-style register splitting at region synthesis is the recorded
  fix, ahead of the nested-list kind and the fused-append lowering.

- **Raw typed-list kinds in the native ABI.** The JIT gained
  `ListFloatRaw`/`ListIntRaw`: a typed list (the OVM's
  `Vec<f64>`/`Vec<i64>` backing) crosses every native boundary — the
  call boundary, OSR entry, the self-call fast path — as a borrowed
  pointer to the raw vector itself, read by 8-byte-stride helpers. No
  boxing, no conversion, no size cap: the earlier stopgap (box small
  typed lists, refuse large ones) is gone. Indexing, `for` iteration,
  and length run native over raw scalars — a summing loop over a
  2M-float list measures 1.04 ns per element read — and pass-through
  returns resolve through their own retain families. Read-only by
  design for now: regions that write lists (`col.set`, the fused
  append) still run on the VM, and the two remaining sub-gates for
  the full ML-loop win are recorded — a nested-list kind for a
  feature matrix (`cols[j][i]`), and native lowering for
  `ListSetAssign`/list `AddAssign` with the sole-owner copy
  semantics.

- **The OSR marshal caps are raised, and refusals name themselves.**
  MAX_PARAMS 16 → 24 and MAX_TUPLE 8 → 16 (with the OSR live-in/out
  caps and the entry gate's own hardcoded 16 following), so a loop
  region carrying up to 24 live-ins and 16 live-outs — a training
  loop's forward pass runs 18/15 — now synthesizes, marshals, and is
  offered native entry. The silent exits on that path speak now: a
  cap refusal prints its counts, a mismatched entry names both heads,
  and an unclassifiable live-in names its register and element type.
  Which is how the next gate identified itself precisely: the forward
  pass carries a list-of-lists live-in (`cols[j][i]`), and the native
  ABI has no kind for a nested list — recorded as the next step.

- **List constants compile to native, and OSR picks the widest
  compilable loop.** Two JIT gaps closed on the road to native
  list-shaped code. A lambda's captured values are baked into its
  compiled bytecode as constants — and a List constant refused the
  JIT's whitelist, locking every list-capturing lambda (a gather's
  `(i) => vals[i]`, a training step's weight update) out of native
  code; classifiable list constants now qualify, seed their element
  kind through inference, and lower to a baked Arc pointer exactly as
  String constants do, with their Arcs joining the return-resolvable
  set. And on-stack replacement no longer gives up when the outermost
  enclosing loop nest refuses to compile: region selection now walks
  the nesting levels around the hot back edge, largest first, and
  takes the widest region that both synthesizes and passes the JIT
  whitelist — an epoch loop that allocates closures per iteration no
  longer blocks its clean inner counter loops from running native.
  OSR refusals also name the offending instruction now. Recorded next
  gates for the full ML-loop win, in order: the OSR marshal caps (a
  forward pass with ~17 live registers exceeds the 16-in/8-out
  marshal), then native lowering for the fused list append.

- **Typed-list backing in the VM.** A homogeneous scalar list crossing
  the tier boundary now lives in its native layout — `Vec<f64>` /
  `Vec<i64>` behind one Arc — instead of a vector of boxed values:
  indexing yields an immediate, iteration streams contiguous memory,
  destructuring patterns, in-place `col.set`/`col.swap`, the
  accumulate-append fusion, concatenation, and equality all work on
  raw scalars, and any operation that inserts a non-matching element
  rebuilds the boxed form (correctness first — the layout is
  invisible). Large conversions go through a bounded pointer-keyed
  cache whose keepalive makes it sound (a pinned list can be neither
  freed nor mutated in place), so a lambda capturing a 300k-element
  list still crosses the boundary in O(1) per call — the property the
  AstList wrapper had, which the first cut of detection broke: a
  gather over 240k elements briefly cost 102 seconds and now costs
  13 ms. Small typed lists cross the JIT call boundary by one-time
  boxing; large ones stay on the VM's typed instruction paths (an
  early per-call conversion there was the same O(n²) cliff).
  tests/typed_list_test.rs pins every reimplemented path
  differentially against the interpreter oracle; the crimes benchmark:
  k-means 9.5 → 7.5 s, logistic 122 → 113 s, results bit-identical.
  Honest scoreboard: the ML training loop still executes on the
  bytecode VM — its body has constructs the JIT refuses — so the
  native-code win for list-shaped ML needs raw-stride native list
  kinds and wider JIT list-op coverage, recorded as the next step.

- **examples/data-processing/crimes — 8.6 million rows, end to end.**
  The gallery's largest workstream and the closest thing to a
  whole-engine benchmark: the complete City of Chicago crime record
  (~2 GB of live CSV) downloaded as part of the run (cached, retried,
  atomically written), loaded through the general parser, and worked
  all the way up — month/hour derived from 8.6M timestamp strings by
  bulk lambdas, a fitted two-decade trend (R²≈0.9), per-category
  arrest rates, the city's hourly rhythm (with the midnight data-entry
  artifact named), a chi-square independence test, k-means over ~900k
  geocoded incidents, and a logistic regression predicting arrests on
  305k training rows with held-out accuracy, precision/recall, AUC,
  and a converging loss curve — the clustering and the classifier
  written in olang itself and executed on the bytecode tier. Outputs
  land in out/ (markdown report, five SVG charts, three derived CSVs);
  data/ and out/ never touch git. Nine test blocks pin the ML and
  prep helpers; the harness skips it by name (a 2 GB download and
  minutes of honest training).

- **examples/ is organized by category.** Thirty-seven examples now
  group under five directories that say what they demonstrate:
  `data-processing/` (the ods stack: ETL, statistics, time series,
  workstreams on real data), `web/` (full-stack apps and the load
  tester), `language/` (parsers, engines, macros, metaprogramming, the
  package system), `concurrency/` (threads, channels, parallel
  pipelines, Harborline), and `tools/` (command-line programs). The
  harness discovers targets recursively, every path reference across
  docs, tests, the Makefile, and the website moved with them, and the
  gallery README opens with the category map.

- **examples/data-processing/climate — a real data-science workstream on real
  downloaded data.** Eight stages end to end: fetch Our World in
  Data's CO2 and energy datasets (~24 MB of live CSV, cached under
  `data/` with retries, size sanity checks, and atomic `.part` writes
  so a partial download is never mistaken for a cached file), prepare
  country and world views (aggregates split off with an anti-join,
  coverage frontiers discovered from the data rather than assumed),
  then real analyses — the global emissions trajectory and its peak,
  top emitters and their global share, the absolute-decoupling list,
  renewables' biggest movers since 2000, and a cross-sectional model
  of per-capita CO2 against energy use (r=0.87, R²=0.76 on 164
  countries) with a Welch t-test across income halves — into a
  self-contained `out/` report: markdown, SVG charts, derived CSVs.
  Data and outputs are regenerated by the run and ignored by git.
  `test` blocks pin the prep and rendering helpers; the examples
  harness skips it (network) and points at its README.

- **Hot-loop promotion — the interpreter's own OSR.** A hot loop at the
  top level of a script had no exit from the tree-walk: every iteration
  interpreted, every call in it crossing the interpreter→VM boundary
  separately. Now a `for`-over-range or `while` loop still running
  after 512 interpreted iterations has its *remainder* synthesized into
  a function (parameters = loop bounds + free variables, return = loop
  value + assigned variables) and handed to the tier, which compiles it
  by body identity like any lambda — one boundary crossing runs every
  remaining iteration on the VM, JIT and OSR included. Bodies whose
  control flow crosses the loop's boundary (`return`, `?`) or that the
  analyzer cannot classify stay interpreted, and a compile refusal falls
  back before anything runs. Error messages AND spans from the promoted
  remainder are byte-identical to the interpreter's
  (tests/loop_promotion_test.rs pins the differentials; the tier fuzz
  corpus and agreement suites run clean over it).

### Changed

- **The data stack closed the Polars gap to 12%** (DP4 pipeline,
  1M rows: 126 ms → 38 ms across seven rounds; pandas 310 ms, Polars
  34 ms — end to end, 8.2× ahead of pandas, with the group and daily
  stages now ahead of Polars; checksums byte-identical across
  engines):
  - *Dictionary encoding parallelized*: group-by key encoding was two
    sequential hash passes over the key columns — most of the group
    stage. Large I64 and Str key columns now encode across all cores:
    chunks build local dictionaries and ids concurrently, the local
    dictionaries merge in chunk order — which reproduces sequential
    first-seen id order exactly, since a key's first occurrence lies
    in the earliest chunk containing it — and a parallel pass
    translates local to global ids. Group: 14 → 4 ms; daily (which
    groups too): 6 → 2 ms. Both now ahead of Polars on this workload.
  - *The file reads directly into the shared body*: `read_csv_file`
    reads the bytes straight into the `Arc<str>` allocation the string
    columns will share (zeroed slice, read_exact, UTF-8 validation,
    then the sound `Arc<[u8]>` → `Arc<str>` cast), and the fast path's
    spans are absolute into that whole-file Arc — the last copy of the
    file is gone, and peak memory during load drops by one body. A
    string column whose chunks saw no empty cell also skips its
    per-cell validity pass. Load 14 → 10 ms.
  - *The hidden sequential pass, found and removed*: what profiling
    kept showing as a ~2× parallel-scaling ceiling on the CSV scan was
    a quote-aware record-boundary pass walking every byte of the body
    sequentially before any worker started. The unquoted fast path now
    finds chunk boundaries with O(workers) newline probes; the fused
    scan's wall time fell from ~18 ms to ~3 ms and load from 27 ms to
    14 ms. (Diagnosed by stamping each parallel chunk's start offset:
    all 18 chunks began within 0.3 ms and finished within 4 ms — the
    other 15 ms preceded them.) Delimiter classification is NEON on
    aarch64 (16 bytes per compare, nibble-mask via vshrn) with the
    memchr path kept for other architectures, and data-stack workers
    promote themselves to user-initiated QoS on macOS so a background
    shell cannot silently confine them to efficiency cores.
  - *The CSV reader is fused*: one scan per record-aligned chunk both
    finds delimiters and parses each field into its column's
    speculative typed builder while the bytes are hot in cache —
    replacing the two-pass shape whose inference pass re-read every
    byte cold. The first non-empty cell seeds a column's speculation;
    a decimal upgrades an Int column to Float in place (an i64
    converts to exactly the double its text would parse to); a word
    demotes a numeric column to text and re-scans that chunk for its
    spans (paid only on real conflicts); chunk outcomes reconcile with
    the cascade's exact semantics. A fast-path i64 parser covers the
    ≤18-digit common case. Load 34 → 27 ms (52 at campaign start);
    tests/csv_reader_differential_test.rs pins the fused reader
    cell-identical to the general csv-crate parser across upgrades,
    demotions, validity, CRLF, blank lines, missing final newlines,
    i64 boundaries, and cross-chunk conflicts on a multi-megabyte
    table.
  - *String columns are views* — the Arrow/Polars string design:
    cells are `(start, len)` spans into one shared immutable buffer
    (`StrCol`), and the CSV reader's backing buffer is the file body
    itself, so loading a text column allocates nothing per cell and
    the splitter emits 8-byte spans instead of 16-byte slices. Bulk
    movement (gather, filter, sort, join) copies spans and shares the
    buffer; a gather keeping under 1/8 of the backing bytes rebuilds a
    tight buffer so a small filtered frame never pins a large file in
    memory. A text column is capped at 4 GB of backing text (u32
    offsets); larger bodies fall back to the owned-cell path. This
    lands the string-view deferral recorded in the design record (and
    retires the one-release `Arc<str>`-per-cell layout and its
    interner): clean 19 → 12 ms, filter 14 → 9 ms, load 52 → 34 ms
    across the campaign.
  - *group_by* (35 → 14 ms): multi-key grouping dictionary-encodes each
    key column independently, then combines per-row ids arithmetically
    and densifies through an array — the per-row `Vec<Option<Key>>`
    allocation and composite hashing are gone. Falls back to the hash
    path when the cardinality product exceeds the dense cap.
  - *CSV load* (52 → 44 ms): the unquoted fast path splits fields in
    one memchr2-driven pass over the bytes (replacing per-line
    re-scanning), pre-sizes its column vectors, and feeds the parallel
    chunks straight into type inference — the sequential chunk merge is
    gone. `OLANG_ODS_TIMING=1` prints the split/parse phase timings.
  - *Frame take/filter*: indices resolve (and the mask converts) once
    per frame instead of once per column, and `gather` skips its
    validity scan entirely for columns with no null bitmap.

- **The JIT learned `TakeMove`.** The by-move argument plumbing from
  Campaign 7 was refused by the JIT's inference pass, so any function
  that passed its argument onward — `fn score(x) = clip(shift(scale(x)))`
  — silently stayed on the bytecode VM while its leaf callees went
  native individually, each through its own call boundary. TakeMove now
  infers and lowers exactly as Move (it is only emitted when the source
  is provably dead), so composed small functions compile native and
  inline their callees.
- **Range `for` loops count on a scalar register.** `for i in a..b`
  (exclusive, body provably not assigning `i`) compiles to a counter
  loop — no Range value, no per-iteration IterGet — which also makes
  every live-in of such a loop a scalar, exactly what OSR can marshal:
  promoted hot loops now enter native code instead of stranding on VM
  dispatch because a Range object sat in a register.

  Together: the `chain` microbenchmark (300k iterations through four
  composed small functions) went from 160 ms to 2 ms; `collatz` from
  152 ms to ~33 ms.

- **`:ovm` is an insight report.** It answered "Promoted: 0 Rejected: 0"
  and nothing else; now it answers the developer's actual questions.
  Session totals (bytecode and native-JIT calls, OSR loop entries, VM
  instructions, compile time), every compiled function with its native
  call count and type specialization, every rejected function with the
  recorded reason it stays interpreted — the tier now keeps the
  compiler's message per rejection — plus a plain-language fix when the
  pattern is recognizable (default parameters, a blocked callee, an
  unsupported construct), polymorphic names explained, and the
  environment flags in effect. `:ovm <name>` tells one function's
  story: its tier, why, and what would change it. `:ovm status` remains
  an alias. Pinned by tests/ovm_insights_test.rs against the real
  binary.

- **Text is made of graphemes** (roadmap W1): `str.graphemes(s)` —
  the visible characters of a string as a list of UAX #29 extended
  grapheme clusters, the composable primitive for every
  visible-character question (count with `len`, index for the nth,
  slice with list operations and `str.join(gs, "")`). `str.reverse`
  now reverses by grapheme, so an emoji keeps its skin-tone modifier,
  a ZWJ family stays one character, accents stay on their letters, and
  flags never re-pair. Codepoints remain the documented indexing unit
  for `len`/`str.length`/`str.char_at`/`str.substring` — stated in
  language.md's new "Codepoints and graphemes" section — and the
  UAX #29 shapes are pinned in tests/graphemes_test.rs.

- **Numbers read back in** (roadmap W2): float literals accept an
  exponent without a decimal point — `1e20`, `2.5e-3`, `1E+6` — so
  every finite float the runtime prints is valid source; the
  print/parse round-trip is pinned bit-for-bit by proptest over
  arbitrary f64s. The trapping-float contract is now stated in
  language.md and stability.md (would-be-`NaN` operations raise;
  the overflow-to-`inf` edge is recorded on the roadmap for a
  decision, not papered over).

- **The REPL under continuation** (roadmap W4): an unbalanced delimiter
  no longer traps the session. The continuation prompt wears what is
  open (`(( ...> `, `" ...> ` inside a string), balancing the input
  evaluates it immediately — closing the delimiter is the exit, no
  `:end` required — `:cancel` abandons the buffer, and a command (or
  `quit`) typed mid-continuation warns and names the unclosed delimiter
  instead of vanishing into the buffer. `:ml`, documented for months
  but never implemented, now exists: deliberate multi-statement entry
  that collects until `:end`. All three (`:ml`, `:end`, `:cancel`)
  have accurate `:help` entries.
- **Session-state differential harness** (roadmap W4): seeded,
  generated action sequences — redefinition after promotion, hot
  loops, `meta.eval` in a warm session, collections — driven through
  the real `olang repl` binary and pinned two ways: every step must
  print the same thing under the tiered REPL and under `--no-ovm`
  (the interpreter oracle), and every step must print the same thing
  when its prefix is replayed in a fresh session. This is the harness
  the help-cache, bridge-landscape, and meta.eval-tier bugs — all
  user-found — argued for.

### Changed

- **JSON numeric fidelity — lossless or loud** (roadmap W2):
  `json.parse` (and the JSON-lines/TOML paths behind it) no longer
  silently converts oversized integers to floats.
  `99999999999999999999999999` used to come back as `1e26`; it is now
  an `Err` naming the value and pointing at string transport/bigint.
  Integers within i64 arrive exactly (2^53+1 included), decimals take
  the standard IEEE reading, and float text that overflows f64
  (`1e400`) errors instead of producing `inf`. serde_json's
  arbitrary_precision feature keeps the source text, so the JSON
  grammar itself decides integer-vs-float. The `toml` module's serde
  bridge leaked that feature's private number wrapper, so both
  directions now convert explicitly — same shapes, and TOML datetimes
  render as strings without the wrapper dance.

- **Deterministic printing for maps and structs**: the harness's first
  run caught `#{...}` and struct values printing in per-process hash
  order — the same program could print `#{"x": 1, "y": 2}` in one
  session and `#{"y": 2, "x": 1}` in the next. `Display` and the REPL
  colorizer now print keys sorted, the order `entries()` already
  documents as canonical.

## [0.78.0] - 2026-08-27

### Added

- **Asserts are expressions** (roadmap W5): `assert_eq`, `assert_ne`,
  `assert`, `assert_true`, and `assert_false` are now also builtins,
  so expression positions the statement-form grammar rewrite never
  reached — match arms, lambda bodies — resolve them with identical
  raising semantics. `match r { Ok(v) => assert_eq(v, 1), ... }` works
  inside a `test` block instead of failing with "Undefined variable".

### Changed

- **Errors that teach** (roadmap W3, all four items):
  - Calling a non-function names the binding and its type — `x(1)`
    where `x` is an Int reports "'x' is an Int, not a function" with a
    shadowing hint; calling a Map or List suggests indexing
    (`m[...]`) instead.
  - Module-not-found now teaches in the REPL too (it previously said
    "This appears to be a system-level error"): the full search list,
    near-miss suggestions that include registered shelf libraries, and
    help that names `use lib.<name>` and `otc lib list`.
  - `if x > 1 { ... }` — the C-style habit — gets a parse suggestion
    with the exact rewrite (`if x > 1 => {`) and a note that only
    `if`/`else` take the arrow while `while`/`for` keep braces.
  - Unclosed delimiters name their opener: a string- and
    comment-aware scan reports "Unclosed `(` opened at line 2,
    column 13" instead of a bare "expected an operator" on a later
    line.

## [0.77.0] - 2026-08-27

### Added

- **The web SDK** (`frameworks/web-sdk`): the foundation layer for
  full-stack olang web applications, and the proof of concept for
  what an ideal olang library looks like. One route table drives the
  whole stack — `route`/`rpc` declare endpoints, `serve` runs them
  behind one JSON envelope and serves the wasm frontend: shell, design
  system, dom shim, and the client program bundled with the SDK's
  browser modules so the browser loads one file and `use web { mount,
  ... }` is made true by the server. Views are plain data rendered
  identically server-side and in the browser; `forms` declares fields
  once and renders, validates (with the shelf's validate library), and
  reads them from that declaration; `sql` packages migrations and the
  parameterized-query disciplines. 105 olang tests — dispatch driven
  in-process with constructed requests, the data layer on `:memory:`,
  and a live integration test that spawns the real server on a task
  thread and drives it over a socket with the http client. Verified in
  a real browser end to end. Chapter: docs/web-sdk.md.

- **`dom.available()`** — the one dom function that exists everywhere:
  `true` in the browser (wasm), `false` natively, so isomorphic code
  degrades gracefully instead of trapping. The web SDK uses it to make
  `olang run client.ol` render the app's initial frame as HTML — a
  static preview with pointers to the real entry — while the same file
  stays fully interactive when served.

### Fixed

- **Test blocks are inert outside `olang test`.** They executed inline
  on every normal run — so `use` of any tested library ran its whole
  suite (prints, asserts, side effects) at import time, and a failing
  assertion in a library test aborted the importing program. Building
  the web SDK surfaced it: importing the server module printed its
  dispatch-test request log. The runner still executes them exactly as
  before.

- **`share use` re-exports keep their closures.** The module
  re-closing pass rebound re-exported functions over the aggregator's
  scope, stripping access to the source module's private helpers —
  web-sdk's index re-exporting `action` died with "Undefined variable:
  mount_actions". Re-exports now carry the closure their own module
  built.

## [0.76.0] - 2026-08-27

### Added

- **The language server earns "robust".** Four new capabilities, each
  protocol-tested against the real binary: inlay hints (parameter
  names at call sites, from the file's declarations and the help
  registry — lambdas and single-parameter calls stay unhinted), code
  actions ("Add /// documentation" on an undocumented declaration;
  "Declare with `let mut`" reading the scoping diagnostic and editing
  the binding), workspace-wide symbol search, and `use`-line
  completions offering the embedded packages, the shelf, and lib/
  modules. Two diagnostics-parity defects fixed along the way: the
  analyzer flagged stdlib modules ("Undefined variable: math") because
  no module namespaces were seeded, and assignment-to-immutable — an
  error that stops a program before its first statement — produced no
  editor diagnostic at all; the scoping pass now runs in diagnostics
  and feeds the quick fix.

- **olang has its own look.** The tree-sitter grammar grows structure
  for what makes olang olang — `|>`, `=>`, `->`, `#{` as their own
  nodes, template strings with `${...}` interpolations as embedded
  code, dotted paths with receiver/member/call-target distinguished,
  `///` doc comments split from `//` — and the Zed highlighting maps
  them away from the Rust-shaped defaults: pipeline and arrow marks as
  special punctuation, templates as special strings, `share` as an
  attribute so exports pop, dotted receivers in the voice themes give
  `self`. Verified across the whole corpus: 112 files, zero parse
  errors. Zed extension 0.3.0 pins the new grammar.

### Changed

- **The editor speaks with `:help`'s voice.** Hovering a user-defined
  function now shows its real signature and the author's `///` doc
  block, rendered like the builtin hovers — and it keeps working
  mid-edit, because the doc extraction is text-level and survives a
  file that does not currently parse. `share` declarations, which were
  invisible to hover, go-to-definition, rename, references, and the
  outline in both discovery paths (parsed and mid-edit scanner), are
  now first-class. Found by a protocol-level audit that drove every
  advertised capability over stdio; the rest of the server checked out,
  and editor formatting inherits the new `olang fmt` respacer
  automatically.

- **`olang fmt` now formats.** The formatter was line hygiene only —
  `fn f( x ,y )=x+y` passed `--check` as "all formatted". It now
  respaces every code line to canonical style: one space around binary
  operators and arrows, commas and colons gluing left and breathing
  right, calls and indexing hugging their value while keywords keep
  their space, unary signs gluing to their operand, ranges glued,
  braces breathing with empty pairs glued. The rules were calibrated
  against every `.ol` file in the repository — the shipped style is
  the specification — and two alignments are recognized as intentional
  and preserved: runs of spaces before `=>` (match-arm tables) and
  before a trailing comment. Strings, templates, comments, and
  indentation are never touched. The safety gate compares the raw
  pre-expansion tree (macro-using files format too) with
  span-insensitive declaration equality, fixing the old gate silently
  refusing any file where a whitespace change shifted a line number.
  The whole corpus is formatted and idempotent under the new rules.

## [0.75.0] - 2026-08-26

### Fixed

- **The bridge interpreter tracks declarations made after its birth.**
  The VM's exact-semantics fallback for declined function values is
  seeded from the declaration landscape — and was seeded exactly once,
  at first use. A bridge created during an early module load (`use
  heap` in the REPL, say) then answered for the whole session, and a
  later module's mutually recursive functions, reached as bare values
  through combinator closures, failed on it with "Undefined variable"
  — the parser example after any prior load. The landscape is now
  versioned (declarations, trait impls, structs, variants) and a stale
  bridge rebuilds; steady-state dispatches reuse one bridge as before.
  Two adjacent staleness holes closed with it: a module's re-closing
  now re-notes its exports to the tier, so the recorded values carry
  the sibling-complete closures the interpreter resolves through, and
  `meta.eval` at runtime gives its child interpreter the caller's
  execution tier — profiled `meta.eval` runs had silently tree-walked
  everything. Pinned by a regression test that drives the live
  reproduction through the REPL binary.

### Added

- **Every shelf ships stocked.** The library shelf — olang's local
  package registry — now seeds a curated set of starter libraries the
  first time it comes into being: `textkit` (text layout and humane
  formatting: pad, wrap, columns, table, money, human_bytes,
  human_duration, slugify), `validate` (declarative checks for
  map-shaped input, every problem reported with its field named), and
  `markdown` (a Markdown renderer with escaping safe for untrusted
  input). All three are pure olang, `///`-documented — so `:help`
  answers for them once imported — and carry their own test blocks,
  which run in the toolchain suite: a starter with failing tests
  cannot ship. They are ordinary shelf citizens: `otc add textkit`
  from any project, `otc lib remove` takes one off (and a removal
  sticks — seeding is tied to the shelf file's creation, not to every
  load), and the new `otc lib restore` brings one back or refreshes it
  to the running toolchain's copy. `otc lib list` marks them
  `(starter)`. The shelf gained its own book chapter, docs/shelf.md —
  the model, the commands, the starter APIs in full, and how pinning
  notices drift — with its examples under the documentation guards.

## [0.74.0] - 2026-08-25

### Added

- **`:help` answers for your own code.** The `///` doc-comment
  convention `olang doc` established now reaches the REPL: a doc
  comment above any declaration — entered in the session, in a `:run`
  file, or in a module a program `use`d — is what `:help <name>`
  shows, rendered like the builtin help with its origin named.
  `:help <module>` on a loaded file shows its `//!` note and every
  documented item; `module.name` qualifies the lookup. Builtins keep
  priority. Docs are re-read from source at lookup time, so an edited
  file answers with its current text.

- **Every `:help` entry teaches, and a guard keeps it that way.** The
  329 compact registry entries (ods, dom, stats, term, math, and the
  rest) carried a one-line description and nothing else; every entry
  in the registry now has a real description, a worked example, and a
  stated return type. A new test parses every example as real olang —
  it caught 142 fictions on its first run (`->` result notation, `;`
  as a match-arm separator, JS-style `{ }` map literals, `if` without
  `=>`, `fn` block bodies missing `=`) and several wrong facts
  (`os.args` documented as returning a Result it never returned). The
  six `error.*` help topics carry their advice as prose and their
  examples as code that parses.

- **Campaign 8: the final engine additions.** The JIT-breadth freeze is
  lifted (roadmap: Campaign 8); five lanes land as the last engine
  work before 1.0, each pinned by tier floors and differential tests.

  - **Polymorphic specialization (E1).** A function is no longer one
    kind-vector specialization forever: a call with new argument kinds
    compiles a variant beside the primary (three per function; failed
    shapes never retry), and group planning adopts or freshly plans
    variants for direct native-to-native calls — which also unfroze
    groups that refused outright on a callee kind mismatch. Mixed
    Int/Float repro: 100k → 200k native calls, 300k → 0 VM
    instructions.
  - **OSR tails (E2).** Nested loops synthesize their outermost
    enclosing region and the dispatch re-offers entry at that head
    (81M → 81k VM instructions on the nested repro); the live-out
    tuple cap rises 4 → 8 (wide-state repro 4M → 164k); and the call
    boundary marshals Boolean arguments, which had silently kept every
    Bool-taking function off native.
  - **JIT template strings (E3).** `MakeTemplate` joins the whitelist:
    scalar interpolations stringify natively with the interpreter's
    exact rules, literals bake as borrowed pointers, parts fold
    through the existing concat and watermark discipline. A template
    no longer poisons its whole function: the report shape went from
    13.5M VM instructions and zero native calls to zero and 300, 3.3×
    wall-clock.
  - **Build-embedded warm profiles (E5).** `olang build` embeds the
    build machine's warm profile for the source (run once, then
    build); the built binary installs it at startup, so its first run
    on a fresh machine pre-compiles proven-hot functions. A local
    sidecar still wins as fresher evidence; the profile is provenance,
    outside the integrity digest, and can never change a result.
  - **Data-stack fusion (E7).** Elementwise Series chains
    (`f["a"] * f["b"] + 1.0`) build lazily and materialize in one
    chunked, parallel pass at first observation — invisible by
    construction (Add/Sub/Mul only, bit-identical per-element FP;
    division, nulls, and mixed dtypes stay eager). 2.7× on a four-op
    chain over 5M elements; the ods.md eager-evaluation record carries
    the amendment.

- **Tier engagement is now a tested fact.** The differential suites pin
  that every tier computes the same answer; nothing pinned that the
  fast tiers *engage* — which is how tail-call elimination kept every
  self-tail-recursive function off native for weeks with no test red.
  `OLANG_TIER_STATS=1` now prints deterministic tier counters to
  stderr after a run: one aggregate line (promoted, rejected, bytecode
  calls, VM instructions, native calls) and one line per VM-callable
  function with the native calls its JIT entry served and its
  specialization kinds — both compilation channels, so a function with
  `native_calls=0` names exactly what the JIT declined.
  `tests/tier_floor_test.rs` runs four representative hot shapes under
  the flag and asserts floors on those counters: direct tail recursion
  (plain and nested-branch), a lambda calling a named recursive
  function through `map`, a map·filter·fold pipeline, and a
  once-called million-iteration loop. Reintroducing the TCE bug turns
  two of them red with the count that names the failure; the suite
  runs from cold (warm start disabled) in under a second.

### Changed

- **`meta.eval` at runtime is real evaluation.** The expansion-time
  purity sandbox (no filesystem, network, clock, randomness) applied to
  every `meta.eval` call, so evaluating an ordinary program's source at
  runtime refused at its first effect. The sandbox now belongs to
  expansion time alone: called outside a meta fn, `meta.eval` runs the
  source in a child interpreter that inherits the run's capability
  table (no escalation — evaluated code is judged by the same grants),
  its `--trace-caps` set, and the current file for module resolution.
  Inside a meta fn nothing changes: expansion stays a deterministic
  function of the source. Feeding `meta.parse` output back into
  `meta.eval` now gets a teaching error — nodes are the analysis
  format, summarized by design, and cannot be turned back into a
  program.

### Fixed

- **Tail-recursive functions reach native code.** Tail-call
  elimination left the old call-result plumbing (a merge move reading
  the register the eliminated call used to write) stranded as dead
  code past the new back-edge; the VM never executed it, but the JIT
  builds SSA for every instruction, and a read of a never-written
  register failed finalize — silently keeping every self-tail-recursive
  function off native since TCE shipped. The unreachable-code sweep
  now runs again after TCE. The collatz kernel that exposed it: 200k
  map elements 1202 ms → 73 ms (16.5×), VM instructions 193M → 400k.

- **Lambdas calling not-yet-compiled named functions compile.** The
  named tier path resolves unresolved callees (compile the dependency,
  retry the caller); the lambda path now has the same loop, so
  `map(xs, (n) => helper(n))` compiles even when `helper` is a
  recursive top-level function the registry has not seen — the last
  systematic "hot lambda stays interpreted" shape. crunch.ol's
  interpreted share fell from 30.5% to 2.7%. The tier's `promoted`
  stat now counts functions compiled through either channel.

### Changed

- **The REPL's `:profile` is the real profiler.** `:profile <code>`
  used to be a five-run stopwatch; it now runs the code once under the
  same sampler as `olang profile` — time by tier, per-function
  self/total with sample counts, hottest call paths, and the findings
  notes — at a finer 250µs interval suited to snippets, printing the
  expression's value as a normal evaluation would.

## [0.73.0] - 2026-08-25

### Added

- **Warm start, on by default.** A finished file run records what its
  tier learned — which named functions ran native, on which scalar
  argument kinds, how often — into a small profile keyed by a hash of
  the program's source (`~/.olang/warm/`). The next run of
  byte-identical source replays it: proven-hot functions compile and
  specialize at declaration time instead of at first call. Profiles
  are hints, never authority — a stale, wrong, or corrupted profile
  costs at most one refused compile attempt and cannot change a
  result, and any source edit changes the key. The current effect is
  honest single-digit milliseconds of first-call latency (Cranelift is
  fast on modern hardware); the per-function native-call counts the
  profile now carries are the foundation for future cross-run
  decisions. `OLANG_WARM=0` disables both sides for A/B;
  `OLANG_WARM_DIR` relocates the store.

- **Capability-specialized compilation, on by default.** Under a static
  manifest (`--deny`, or an `[capabilities]` block), the bytecode
  compiler knows the whole run's grant and each function's provenance —
  so `caps.allowed("net")` compiles to a constant, the branch it guards
  folds, and the denied side is swept to dead code. Sandboxed code gets
  faster, not slower: the degradation branch a program writes for the
  denied case no longer taxes the granted path, and code that can never
  run stops counting against JIT qualification. Calls the manifest
  provably grants skip the runtime gate's per-call table walk (denied
  calls keep the full gate — the error message is its job), and
  `--trace-caps` recording and the argument-dependent `fs` sub-gate are
  untouched. Two new optimizer passes carry it: constant-branch folding
  and an unreachable-code sweep, both general (a literal `while true`
  benefits the same way).

## [0.72.0] - 2026-08-24

### Changed

- **`map`, `filter`, and `fold` run their whole loop on the compiled
  tiers.** A kernel passed to these builtins used to pay a full
  interpreter→tier boundary crossing per element — the kernel itself
  ran native in nanoseconds while the per-element dispatch around it
  cost fifty times more, which is why profiles of idiomatic pipelines
  read "mostly interpreter". The builtins now hand the entire loop to
  the VM: one list conversion in, one native loop (JIT included), one
  conversion out; parallel chunks do the same per chunk, so automatic
  parallelism and `par_map` compose with native execution. Kernels the
  tier declines (defaults, bounds, uncompilable bodies) fall back to
  the unchanged per-element path. Measured with identical results: a
  2M-element map·filter pipeline 904 ms → 153 ms, a 2M fold 449 ms →
  81 ms, examples/data-processing/benchmark/main.ol 1001 ms → 350 ms with its tier profile
  going from 91% interpreted to ~0%.

### Fixed

- **The profiler no longer slows or stalls threaded programs.** Two
  defects: exited threads never left the sampler's registry, so a
  spawn-heavy program grew the scan list without bound while thread
  creation and the sampler contended on the same lock (the observed
  `--profile` hang); and every frame push took the global intern lock,
  serializing all threads of a parallel program into a crawl. Dead
  stacks are now pruned each tick, and pushes resolve names through a
  per-thread memo — the steady state takes no lock at all.

### Changed

- **otc, reimagined as the project tool.** The division of labor is one
  sentence: everything that touches a *file* lives in `olang`;
  everything that touches a *project* lives in `otc`. The surface is
  now seven top-level verbs — `new`, `add`, `remove`, `list`,
  `install`, `lib`, and (coming) `bench` — with `otc pkg ...`
  flattened away. Gone with it: `otc check` (a stale analyzer wrapper
  that reported "no issues" on files `olang check` flags), `otc ovm`
  (ran code, violating otc's own charter), `otc deps`/`unused`
  (deferred with the analysis work), and the registry/publish surface
  (dormant until distribution matters; `OLANG_REGISTRY` still works
  underneath).

### Added

- **The projects chapter, rewritten for the local-first workflow.**
  [Packages and dependencies](docs/packages.md) now leads with the
  seven-verb `otc` surface, the shelf, and the bench harness; the
  registry section is explicitly marked dormant surface until
  distribution matters. otc appears in the README and the tooling
  chapter's overview for the first time. Reliability got the same
  treatment: an end-to-end CLI test drives every verb (all three
  scaffolds parse-checked, the shelf round trip, add by name and by
  path with auto-init, refusals with their messages, the frozen
  install gate, and a program actually importing a shelved library),
  and the bench harness's spec parsing, growth classification, and
  output conventions are pinned by unit tests.

- **The library shelf: local dependencies by name.** `otc lib add
  ~/code/my-lib` registers a library once, per user; from then on any
  project says `otc add my-lib` — no paths, no registry. The manifest
  records only the name (`my-lib = { shelf = "my-lib" }`), so
  `olang.toml` stays free of machine-specific paths; the lockfile pins
  the resolved directory and checksum, so a moved or edited library is
  noticed rather than silently drifted past. `otc add ../my-lib`
  records a path dependency (relative to the project root, whatever
  directory the command ran from), and `otc add` in a directory with
  no manifest creates one instead of demanding an init step. Every
  mutation ends by resolving, so the lock is never behind the
  manifest.

- **`otc bench`: the project benchmark harness.** Benches are ordinary
  olang programs in `bench/`; the harness runs each as a fresh
  subprocess and adds what a stopwatch cannot. A
  `// bench: sizes = 1000, 4000, 16000` directive turns one bench into
  a scaling curve run per size, and the harness fits the growth and
  names it — `~O(n)`, `~O(n log n)`, or `⚠ ~O(n²) — check for a
  copy-per-iteration` (measured exponent included; process startup is
  measured and subtracted so small points read true). A `CHECKSUM`
  line in a bench's output must agree across repetitions or the
  measurement is refused — a timing whose answer wobbles is measuring
  something else. Each point reports median, coefficient of variation,
  and peak RSS (from the child's rusage). `--save`/`--against` keep
  baselines with a machine fingerprint (cross-machine comparisons warn
  instead of pretending), `--fail-on-regress` is the CI gate with a
  noise floor of max(5%, 2×CV), a `TIME <ms>` line lets a bench
  self-time to exclude startup and setup, and `--profile` reruns the
  slowest point under `olang profile` so "what regressed" arrives with
  "where it went".

- **`otc new --web`: the full-stack starter.** One olang process
  serving a SQLite-backed JSON API, the page, and the frontend's own
  olang source, run in the browser through the wasm runtime — the
  architecture the tracker and ledger examples proved, trimmed to a
  working notes app where every seam a real app grows along appears
  exactly once. The scaffold copies in an `olang_playground.wasm` when
  it can find one (`$OLANG_WASM`, next to the executable, or
  `~/.olang/`) and says exactly how to supply it when it cannot; the
  API works either way. Generated `.ol` files are parse-checked before
  writing, the same promise the other shapes make.

### Changed

- **The pipeline benchmark's two worst in-memory stages, closed.**
  `drop_null` consulted a materialized value per cell — six million
  `Scalar` constructions on a 1M-row frame, cloning every string it
  passed over — where nullness is exactly the validity bitmap: it now
  asks the bitmap, and a column with no nulls drops out of the check
  entirely. Frame `filter` ran its columns one after another; columns
  are independent, so it now runs one per core above 100k rows, the
  way `take` has since it shipped. Column type inference also stopped
  parsing numeric columns twice (test with one scan, rebuild with a
  second) in favor of keeping what it parses. On the published
  benchmark: clean 69 ms → 20 ms, filter 39 ms → 13 ms, whole pipeline
  223 ms → 126 ms. Every stage checksum is unchanged.

- **CSV reads borrow instead of allocating.** An unquoted file now
  parses into slices of the original text, so a numeric column is
  parsed straight out of the file and never becomes a `String` — where
  every cell used to be allocated before anything knew its type. Only
  columns that really are text allocate. A quote anywhere, or a row
  whose field count disagrees with the header, falls through to the
  general parser unchanged, keeping quoting rules and the line numbers
  in parse errors where they belong. Load of a 1M-row, 6-column file:
  76 ms → 52 ms, and the pipeline is 2.5× ahead of pandas end to end.

### Added

- **`olang profile` — a sampling profiler that knows about tiers.**
  Runs a program normally (same tiers, capabilities, and argv) while a
  background thread samples a per-thread shadow stack, then reports
  self time, total time, and the *execution tier* for every function
  that appeared, plus the hottest call paths. The tier column is the
  point: a hot function marked `interp` never promoted, which is a
  different bug from a hot function that is simply doing a lot of
  work. Recursion folds to one frame and JIT-inlined callees are
  attributed to their caller, both documented in the tooling chapter.
  Profiling is off unless asked for — the instrumentation costs one
  relaxed atomic load per call, with no measurable effect on ordinary
  runs.

  The report leads with **time by tier** (bars for native, vm, interp,
  and builtin — standard-library time is Rust, and counting it as
  interpreter time would misattribute the very thing the tool exists
  to report), then per-function self/total shares with sample counts,
  the hottest call paths, and closing **notes** that say what the
  numbers mean when they mean something: a large interpreted share and
  which functions carry it, a run too short to conclude from, or work
  that ran on parallel worker threads. Anonymous functions are named
  for where they were written (`<lambda in bench>`), and a builtin
  that runs user code (`map`, `fold`) appears in call paths so a
  lambda always has a visible caller.

- **`olang check` flags unrebound collection writes.** The bundled
  collections are values: `heap.push(h, x)` computes a new heap and
  returns it, so a bare call in statement position is a silent no-op
  that reads like a mutation. The checker now warns with the rebind
  spelled out (`h = heap.push(h, ...)`), covering both the short form
  and the fully qualified `collections.heap.push`. Rebound writes,
  discarded reads, and tail positions — where the handle is the
  block's value — stay silent.

### Fixed

- **Templated-string appends are O(1) again.** The accumulation shape
  every report and CSV builder uses — `xs = xs + [`row-${i}`]` — ran
  quadratically on the VM: `assignment_free` had no arm for template
  strings, so the append fusion refused the pattern and every
  iteration copied the whole list (40k appends took ~1.8s; fused they
  take ~10ms). `MakeTemplate` also joined the optimizer's and JIT's
  register models, so a template string in a function no longer
  disables the dead-move pass, inlining, or OSR renumbering around it.

### Added

- **The pipeline benchmark (DP4).** The last open lane of the
  data-pipeline campaign: an end-to-end ETL pass over a 1M-row CSV —
  load, clean, derive, filter, two-key group, join, sort, rolling
  window, write — implemented stage-for-stage identically in ods,
  pandas, and Polars under `benchmarks/`. Every stage prints a
  checksum and the runner refuses any result where engines or
  repetitions disagree, so timings only ever compare byte-identical
  answers. As measured (Apple M5 Pro, medians of five): ods 223 ms end
  to end versus pandas 308 ms and Polars 34 ms; the per-stage table,
  methodology, and hardware are recorded in the data-stack chapter.

## [0.71.0] - 2026-08-24

### Fixed

- **The call-depth cap gates the JIT attempt.** All three VM entry
  points tried native code before the depth check, so a base-case frame
  arriving exactly at the cap ran natively to completion without ever
  meeting the check — recursion at exactly the 100,000-frame limit
  returned a value on the tiered path where the interpreter raised.
  The cap now gates the attempt itself; both tiers refuse frame
  100,001 identically.

- **Identifiers may start with keywords.** Every reserved and
  contextual keyword could swallow the head of an identifier —
  `breaker` lexed as `break` + `er`, `returns = 5` as `return s`, and
  `useful = 1` failed inside `use_decl` — because grammar rules
  consumed keywords as bare literals with no word boundary. Every
  keyword a rule consumes is now a guarded atomic (the `mut_kw` idiom):
  it matches only when not followed by an identifier character, so
  `useful`, `typed`, `formal`, `input`, `matcher`, `iffy` and every
  other keyword-prefixed name parse as the identifiers they are.

### Added

- **The tier boundary, lane T4: transitive JIT inlining (Campaign 7).**
  The JIT's inliner no longer refuses a small callee just because it
  has calls of its own: when those calls themselves inline away (a
  distance function calling a square helper), the callee presents its
  expanded, call-free form and inlines like any leaf, to two levels of
  helper-within-helper. Recursive callees bottom out naturally — the
  recursive call survives expansion, and a form still containing one
  stays a call. Whitelisted named and builtin calls (`map_get`, float
  math) pass through the splice unexpanded. A three-level helper chain
  in a 30M-iteration loop runs 1.9× faster (153ms → 82ms); the
  transform exists only on the JIT's planning clone, so semantics and
  stack traces cannot drift, and two differential tests pin exact
  agreement with the interpreter.

- **REPL `:run` behaves like `olang run`.** The post-run cleanup kept
  only a hardcoded module list that was missing `str` and `time`, so
  the second `:run` of a session found its ambient modules wiped
  ("Undefined variable: str"); every Module-typed binding is now
  retained. `:run` also installs the file's own argv for the duration
  (argv[0] is the script path, as `olang run` provides), so a script
  that resolves paths from `os.args()[0]` — run_all.ol discovers its
  whole example set that way — works identically under the REPL. The
  "Generating very large range" warning is gone: it fired at a size
  the current tiers handle in milliseconds and suggested a feature
  that does not exist.

- **`meta.parse`/`meta.eval` name the fix for wrong-shaped input.**
  Passing the `Result` a file read returns now says "got Ok(String) —
  unwrap the read first: meta.parse(unwrap(f))" instead of the bare
  rule; other wrong kinds report their type name. A mistyped `mut`
  (`let m f = ...`) now gets "Did you mean `let mut f`?" from the
  error-suggestion engine instead of the generic "expected the end of
  the file".

- **The tier boundary, lane T3: on-stack replacement (Campaign 7).** A
  hot loop inside a function the whole-function JIT refuses — a
  `println` in the prologue, string formatting after the loop — no
  longer spins on the VM. After 8192 back-edges in one frame, the loop
  region alone is synthesized into a standalone function (the registers
  the loop reads become parameters; the ones it writes and something
  later still reads come back as the return, a tuple when there are
  several) and compiled through the ordinary JIT pipeline; the dispatch
  loop enters it mid-frame and resumes at the loop's exit with the
  returned state. Every instruction a region may contain is pure with
  respect to caller-visible state, so any native failure — refusal,
  deopt, runtime error — simply resumes the VM at the loop head with
  its registers untouched, and real errors re-raise with the original
  spans. Lists that crossed the boundary by move (T2's `AstList`)
  materialize once at entry and iterate natively. 7× on the
  arithmetic-loop repro (1098ms → 159ms at 50M iterations);
  `OLANG_OSR_OFF=1` disables it, `OLANG_OSR_DEBUG=1` traces region
  synthesis and entries.

### Changed

- **REPL: rich color.** Input lines syntax-highlight live as you type —
  keywords magenta, strings and templates green, numbers and booleans
  yellow, calls and `:commands` cyan, comments dimmed, and the `|>`/`=>`
  arrows blue. Results print in the same palette (map keys and struct
  fields cyan, type names blue, `Ok` green / `Err` red, `()` dimmed),
  the prompt is bold green with a dimmed continuation, `:set show_types`
  output and the `:time` line are tinted to stay out of the way. Colors
  engage only on a terminal — piped output remains plain text.

- **examples/web/ledger: robustness and visual pass.** The frontend gains a
  category manager (add, inline rename, delete — the API existed, the UI
  didn't), budget progress bars, summary stat cards, toast notifications
  that surface the server's real error messages (field-level 422 details
  included), two-click delete confirmation on transactions and
  categories, a "today" month-nav button, coalesced reloads (one paint
  per refresh instead of two), and a deterministic trend-chart palette
  (mint is always money in, red always money out).

### Added

- **The tier boundary, lane T2: arguments convert proportional to use
  (Campaign 7).** Large lists now cross the interpreter⇄VM boundary in
  O(1), wrapped as `AstList` handles and converted by move instead of
  element-by-element copy. Inside the VM, calls consume argument
  registers the optimizer proves dead by move (a per-call `arg_moves`
  mask), a `TakeMove` instruction replaces dead register copies, and the
  dead-move pass runs to a fixpoint so chains collapse. The bundled
  collections are VM-resident by default as a result — 2–8× faster
  across the board (table put/probe: 4.7s → 0.6s at 100k; the growth
  curve that was quadratic is linear: 207s → 0.18s at n=40k).
  `OLANG_COLLECTIONS_ON_INTERP=1` restores the old placement for A/B.
  Also fixed along the way: the tier boundary charged one stack frame
  twice, so recursion one frame under the depth cap died early on
  non-JIT paths, and the JIT's native budget allowed one frame past the
  cap.

- **Collections, in olang (Campaign 6).** One `collections` module of
  six submodules — `collections.heap`, `.deque`, `.table`
  (open-addressing hash), `.dsu` (union–find), `.bitset`, and `.alg`
  (stable merge sort, sort-by-key, the bisect family, quickselect, CSR
  graphs, BFS, topological sort, Dijkstra) — written entirely in olang,
  compiled into the binary, and *automatically available*: the one name
  loads on first touch, no `use` needed, no startup cost when unused,
  and no other global names claimed. `use collections { heap, table }`
  imports short names where code leans on them. One calling convention throughout:
  operations take the handle first and return it, the caller rebinds
  (`h = heap.push(h, prio, item)`), and reads never rebind. Handles are
  single flat lists with documented layouts; every module carries its
  own `test` blocks (run by the suite) and reference-grade code
  documentation. `alg.dijkstra` runs `heap` over a flat weighted graph
  — the collections composing, all in olang.

- **The mutation primitives beneath them.** `col.set(xs, i, v)`,
  `col.swap(xs, i, j)`, and `col.filled(n, v)` — and two assignment
  fusions that make the rebind discipline fast: `x = col.set(x, ...)`
  writes the list in place when `x` holds the only reference (both
  tiers — the VM compiles it to a register-level in-place write), and
  `x = f(x, ...)` passes `x` to any user function *by move*, so a
  handle crosses a call boundary without a copy. Arguments now also
  bind into call frames by move rather than clone. Aliasing is never
  unsafe: a second binding degrades the write to a copy, exactly as the
  language has always promised. Also `str.char_code`, the code-point
  primitive olang-written string hashing needed.

- **Blocks drop intermediate results promptly.** A block's non-final
  statements no longer pin their values until the next statement
  finishes — one hidden reference that could defeat every sole-owner
  fusion on the following line.

### Fixed

- **The bridge interpreter — where the VM runs function values it
  declines — was tierless, ungated, and fresh-budgeted (Campaign 7,
  T1).** Three defects with one root: a callback invoked inside a
  promoted function stranded everything it called on a tree-walk
  (measured: 2317ms against 5ms for the same 5M-frame call — the
  harness/callback trap); the declined-callee path seeded no capability
  table, so such a callee ran ungated by --deny and manifests; and its
  call-depth budget started at zero rather than at the caller's live
  depth. The bridge now carries its own compiled tier — seeded with the
  program's declaration and function landscape — the grant is re-seeded
  per dispatch and forwarded into that tier, and the depth base crosses
  the boundary in both directions.

  Exercising the round-trip surfaced four latent correctness bugs, each
  fixed on its own merits: `get_slot`'s depth walk returned "undefined"
  instead of falling back to name resolution when a resolved body ran
  under a shallower scope chain; a VM closure rebuilt as an AST function
  lost its *name*, so a rebuilt nested fn could not call itself; a bare
  name bound to two distinct function bodies (two modules' private
  `insert`) could hijack each other's call sites, now tracked and never
  seeded ambiguously; and the compiler resolved a callee through the
  function registry even when the compiled closure held a *different*
  function under that name — lexical scope now wins when they disagree.
  Regression tests pin every property, including the demo's exact
  nested-fn shape and the two-private-helpers collision.

### Improved

- **The REPL, end to end.** `:help` opens with a page instead of a wall:
  the commands, a curated module map, and the REPL surface — the
  category dump (with its case-duplicate drift: "math" beside "Math",
  "CSV" beside "csv") is gone, because a dotted function's category is
  now *derived* from its module and cannot drift. A missing module
  member suggests the nearest real one, identically on both tiers
  (`collections.headp` → "did you mean 'heap'?"); a mistyped colon
  command suggests the nearest command, plainly; `:use ...` runs the
  statement the colon reflex meant. `it` holds the last printed result.
  And the first bundled-collection write called without rebinding its
  handle earns a one-time tip explaining the convention — the unchanged
  variable is the design, not a bug, and now the REPL says so.

### Fixed

- **`testing.assert_eq` compared Native values (BigInt, Date, Bytes) as
  never-equal** — two equal BigInts "failed" with identical expected
  and actual in the message. Native values now compare by their own
  structural equality, and maps compare structurally too. Surfaced by
  hardening `crunch.ol`'s assertions from the tally-only form to
  `|> unwrap` (the documented idiom for inline scripts), which turned a
  silently vacuous check into a loud false failure.

### Changed

- **Function parameters are mutable bindings.** Assigning to a
  parameter rebinds the function's own local and never touches the
  caller (arguments pass by value, as ever). The old rule steered
  toward shadowing, which pins a second reference — exactly wrong for
  the collections' rebind convention. Purely permissive: no previously
  valid program changes meaning.

- **The bundled collection modules are interpreter-resident.** Their
  operations do O(log n) work against O(n)-sized handles, and the tier
  boundary converts list arguments in full — so promotion is declined
  for exactly these modules, where the interpreter's in-place fusions
  are the faster path end to end.

## [0.70.0] - 2026-08-23

### Added

- **`bigint` — arbitrary-precision integers (Campaign 5, N1).**
  `bigint.of` takes an Int or a digit string; the value then uses the
  ordinary operators on both tiers, and a plain Int operand promotes to
  BigInt on contact. The arithmetic follows the Int rules exactly —
  truncating division, dividend-signed remainder, the same zero-divisor
  errors — so promoting a computation changes its range and nothing
  else. Floats never mix implicitly (53 bits of mantissa would silently
  round the digits BigInt exists to keep); `bigint.to_float` is the one
  explicit, lossy door. With `parse`, `to_int`, `abs`, `neg`, `pow`,
  `mod_pow`, and `gcd`; help entries and editor completions included.
  The Int overflow errors now point at it: "Integer overflow in
  multiplication (bigint.of gives arbitrary precision)".

- **Automatic parallelism for provably pure bulk operations (Campaign
  5, H2).** `map` and `filter` over 50,000+ elements fan out across
  every core when the kernel passes a conservative purity proof — a
  whitelist walk over its AST (arithmetic, control flow, pure builtins
  and modules; any user-function call or effect disqualifies). Pure +
  order-preserving join means the result is bit-identical to the
  sequential run — same values, order, and first error; nothing
  recorded, nothing gated, tier agreement untouched — so the only
  observable difference is the clock: a 3M-element map runs 3.7× faster
  on 18 cores with no change to the program. Impure kernels stay
  sequential (their effects keep element order, which a test pins);
  macro expansion and coverage runs never fan out; `set_parallel(false)`
  disables it; `OLANG_DEBUG_AUTOPAR=1` explains each decision.

### Added

- **Tail-call elimination on all three tiers (Campaign 5, R4b).** A
  self-call in tail position — both branches of an `if`, a match arm, a
  block's final expression, the expression of a `return` — runs in the
  caller's frame: O(1) stack *and* O(1) logical depth, so tail
  recursion never meets the depth cap at all. The interpreter
  trampolines (deciding self-ness by function identity, so a same-named
  shadow or sibling closure stays an ordinary call, and re-running the
  full call boundary — arity, bounds, annotations — for every elided
  frame); the VM rewrites the call to a parameter rebind and a jump to
  the entry; the JIT compiles that jump as a native loop. Measured: 20
  calls each 500,000 frames deep in 4ms — tail recursion at native
  loop speed with unbounded depth — and a single 50M-frame descent in
  22ms. Elided frames are noted in error traces ("spin (tail calls
  elided)"), effects keep their order, non-tail recursion still meets
  the cap identically on both tiers, and an under-applied self-call
  (leaning on parameter defaults) deliberately takes the ordinary call
  path, keeping default-expression scoping exactly as it was. One
  consequence to know: a *runaway* tail recursion — `fn boom(n) =
  boom(n + 1)` — is now an infinite loop rather than a depth error,
  because tail calls are iteration; it non-halts exactly as
  `while true {}` always has. Runaway recursion that needs its frames
  back still meets the cap as a clean error.

### Changed

- **The call-depth cap is 100,000 frames (was 1,000) — and it is now a
  real number (Campaign 5, R4a).** Each user call grows the Rust stack
  in segments when headroom runs low (the segmented-stack discipline
  rustc itself uses), so every depth under the cap is physically
  reachable — 99,999-deep recursion works even from a thread with a
  small OS stack, and frame 100,000 raises the clean "Maximum call
  depth exceeded" error instead of the stack raising a signal.
  `--max-depth N` moves the cap in either direction. Both tiers now
  spend from one *shared* budget, seeded across the tier boundary: a
  recursion that promotes mid-descent previously got a fresh VM budget
  on top of the interpreter frames it had already spent, so a program
  near the cap could succeed on one tier and fail on the other. The JIT
  keeps its native recursion within a fixed 1,000-frame budget (its
  pre-existing ceiling) and deopts to bytecode beyond it — pure groups
  re-execute, so deeper recursion is slower, never wrong. The
  playground keeps its low wasm limit: that stack cannot grow.

### Performance

- **Parallel data-stack kernels (Campaign 5, H3).** The data stack's
  hot paths fan out across cores, joined in fixed order so results are
  cell-identical to the sequential run: `read_csv` splits large text at
  quote-aware record boundaries and parses the runs concurrently
  (malformed files fall back to the sequential reader so the error and
  its line number are unchanged); `read_jsonl` parses lines
  concurrently, reports the lowest-numbered bad line, and — the bigger
  win — goes straight from parsed JSON into column vectors instead of
  building a boxed map per line (the old row-shaped intermediate cost
  more than the parse and tripled peak memory); `sort_by` sorts its
  index and gathers its columns in parallel; the group-by composite-key
  build and the join probe fan out too. Measured at 1M rows: JSONL
  parse 1153ms → 265ms (4.3×), CSV parse 205ms → 75ms (2.7×), sort_by
  176ms → 93ms. `set_parallel(false)` now governs the data stack's
  own fan-outs as well, which it previously did not. Seven differential
  tests pin parallel ≡ sequential, including quoted embedded newlines
  and error identity.

- **Compiled-kernel parallelism (Campaign 5, H1).** Three root causes,
  each found by measuring: anonymous lambdas never promoted to the
  compiled tiers ("no stable identity" — but body and closure Arcs are
  identity, and the HOF cache already keyed on them); every call cloned
  the entire Function value (parameter vector, name, check tables —
  per element, allocator-contended across cores); and parallel workers
  serialized on the shared kernel's atomic reference counts
  (Weak::upgrade per call from twelve cores). Lambdas now promote by
  identity, both call entry points share a by-reference
  `call_user_function`, and each worker gets a `thread_localized`
  kernel with fresh Arcs. par_map over 3M trivial elements: 8.43s of
  CPU → 0.63s, wall 0.73 → 0.26; sequential `map` itself 32% faster.

### Fixed

- **`set_parallel(false)` did not disable `par_map`/`par_filter`.** The
  workers consulted only the thread count, never the enabled flag; the
  switch now actually switches, and the automatic fan-out honors it too.

- **The purity-verdict cache could hand a new kernel a dead one's
  verdict.** Keyed by body address alone, a freed body's reused
  allocation inherited the old verdict (an effectful lambda judged
  pure, caught by the suite). The cache now holds a Weak and verifies
  identity by upgrade + pointer equality — the HOF cache's discipline.

## [0.69.0] - 2026-08-22

### Changed — the last-mile corrections (deliberate breaking change, the third and final before 1.0)

- **Operator precedence now reads conventionally.** The table, loosest →
  tightest, is: `||` · `&&` · comparison · `|>` · ranges · `+ -` ·
  `* / %` · bitwise · unary · postfix. Three orderings changed:

  - `&&` binds tighter than `||` (they shared one level).
    `a || b && c` is now `a || (b && c)`.
  - Ranges bind looser than arithmetic. `0..n-1` is now `0..(n-1)`
    — previously it was `(0..n) - 1`, a runtime error.
  - The pipeline binds looser than arithmetic and ranges, tighter than
    comparison (Elixir's placement). `x + 1 |> f` now pipes the sum:
    it is `f(x + 1)`, not `x + f(1)`. A pipeline's *result* still feeds
    a comparison: `xs |> len == 3` remains `(xs |> len) == 3`.

  **Migration:** code that parenthesized mixed operators — what the book
  always advised — is unaffected. Audit unparenthesized `a || b && c`
  (meaning changes), unparenthesized arithmetic beside `|>` (meaning
  changes, almost always to what was intended), and `0..(n-1)` bounds
  (still correct, parentheses now optional). Bitwise precedence is
  unchanged: `1 << 4 * 2` is still `(1 << 4) * 2`.

- **Loops evaluate to Unit unless `break value` exits them.** `for`,
  `while`, and `loop` no longer yield the last body iteration's value —
  that behavior was undocumented, already absent from the bytecode tier
  (a latent tier divergence), and retaining the value aliased it in a
  way that defeated the O(n) append fusion for `xs = xs + [..]` bodies.
  `break v` still makes any loop evaluate to `v`.

- **Absence is Unit — the lookup convention, settled.** A lookup returns
  the value, or `Unit` when there is nothing there; parsers and I/O
  return `Result`; misuse raises. Three changes make it real:

  - `str.index_of` / `str.last_index_of` return `Unit` when absent
    (was `-1` — a sentinel that negative indexing made dangerous:
    `s[str.index_of(s, x)]` on a miss read the *last* character).
    **Migration:** `idx == -1` → `idx == ()`; `idx >= 0` → `idx != ()`.
  - **`x == ()` / `x != ()` is total.** The presence test answers for
    every value (false/true unless `x` is Unit) instead of raising on a
    present value — without this, `idx != ()` raised the moment the
    lookup succeeded. Equality between two present-but-unrelated kinds
    still raises; ordering against Unit still raises.
  - New global `map_get_or(m, k, default)` — the lookup-with-default in
    one call. A stored Unit takes the default too: under this
    convention a stored Unit *is* absence (use `map_has_key` when the
    distinction matters).

- **`dates.date(y, m, d)` returns a `Date` value** (was an ISO string
  inside the `Ok`). The display is the same ISO text, so formatting and
  printing keep working; code that concatenated the payload as a string
  should call `show(d)` or use a template. All other date-taking
  functions accept both forms and answer in kind (below), so no other
  call site changes.

### Added

- **`Bytes` — binary data as a value.** `bytes.from_list` / `to_list` /
  `from_string` / `to_string` (Result) / `len` / `slice` / `concat`;
  `len(b)` and `b[i]` (negative from the end) work; equality is
  structural; display is a capped hex preview. `fs.read_bytes` /
  `fs.write_bytes` move it to and from disk under the same fs
  capability gates as their text twins; `base64.encode` and the
  `crypto` hashes accept it; `base64.decode_bytes` decodes to it.

- **`Date` — a first-class calendar date.** `dates.date(y, m, d)` and
  the new `dates.parse(s)` build one; `typeof` says `Date`;
  comparisons order chronologically; `d2 - d1` is the signed day
  difference; `d ± n` shifts by days. Every date-taking `dates`
  function accepts a `Date` or a date string and answers in the
  caller's kind, so string-based code is unaffected. String parsing is
  uniformly flexible now — `dates.now()` output works everywhere a
  date string is accepted (`add_days` previously refused what `year`
  accepted).

- **The HTTP client can authenticate.** Every client verb takes an
  optional trailing options map: `"headers"`, `"timeout_ms"`,
  `"bearer"`, and `"basic": (user, pass)`. Responses now carry a
  `headers` map (lowercased names). An unknown option key raises
  rather than being ignored.

- **One effect classification, three consumers** (`src/effects.rs`).
  The capability gate, macro-expansion purity, and record/replay used
  to curate three independent lists of "what is effectful", and they
  had drifted: a `meta fn` could call `dates.now`, `crypto.random_*`,
  and the `ods` file readers/writers at expansion time — breaking
  macro Law 4 and, through it, the `--rules` sandbox. All three now
  ask one table; `par for` gained the meta-mode gate `spawn` already
  had; `olang expand` resolves macro imports against the file's
  directory like every other consumer.

- **The fuzzers are committed.** `tests/macro_fuzz_corpus_test.rs` and
  `tests/tier_fuzz_corpus_test.rs` are seeded, deterministic
  generators: 150-seed smoke corpora run on every `cargo test`, and
  the full 10,000-seed campaigns the stability chapter cites are the
  `#[ignore]`d `*_full_campaign` tests — reproducible rather than
  historical.

- **Record/replay warns when a thread is spawned.** The timeline
  covers the main thread only; a recorded or replayed run that starts
  a task or worker now says so on stderr (once) instead of letting a
  trace that silently missed worker effects present as a clean,
  fully-determined run.

- **`dates` misuse raises** (stdlib rule 3). Wrong arity or a
  wrong-typed argument aborts with a `dates.`-qualified message; before,
  it came back as `Err`, so `unwrap_or(dates.add_days(d), fallback)`
  silently swallowed a typo'd call. Data failures (a malformed date
  string, an invalid component) still return `Result`.

### Fixed

- **The interpreter's "memory allocation limit" is gone.** Top-level
  code that built a list past 10,000 elements with the documented
  idiomatic append (`xs = xs + [i]`) aborted with a spurious
  "Memory allocation limit (10000) exceeded" error — while the same loop
  inside a function ran fine. The tracking apparatus prevented nothing
  (the runtime is safe Rust) and is removed outright.

- **Top-level list accumulation is O(n).** The sole-owner append fusion
  now reaches bindings in the top-level persistent environment, not just
  function-frame locals: 200,000 appends run in under 100 ms where
  20,000 previously took seconds (and then hit the limit above).
  Aliasing is still honored — a snapshot taken before an append forces
  the copy, on every tier.

### Added

- **The language server grew the navigation set that makes an editor
  feel alive**: document outline (every `fn`, `meta fn`, `type`,
  top-level `let`, and `test` block), find-references and document
  highlight, rename (with refusals that say why — keywords and
  standard-library names are not yours to rename), and signature help
  with active-parameter tracking. Completions are context-aware: after
  `str.` you get the `str` module's functions with signatures and
  documentation, not keywords. Hover now answers for every documented
  name — builtins and stdlib functions render from the same registry
  `:help` prints, so the editor and the REPL never disagree. When a
  file is mid-edit and unparseable — most moments in a live editor —
  declarations fall back to a text scan, so hover, navigation, and the
  outline keep working while you type.

### Fixed

- **`olang lsp` rejected the `--stdio` flag every LSP client passes by
  convention — so the server had never once started inside VS Code.**
  clap exited with "unexpected argument" before the first protocol
  byte; the client retried into startFailed; the editor showed nothing.
  Found by installing the extension and reading the extension host's
  logs — no protocol test could see it, because tests spawned the
  server without the flag. The flag is now accepted (stdio is the only
  transport, so it changes nothing), and every protocol test spawns
  `olang lsp --stdio` exactly as real clients do. The editor chapter
  documents the failure signature, since a stale binary on PATH
  (`~/.cargo/bin` shadowing a newer install) reproduces it.

- **Both editor clients resolve the binary beyond `PATH`.** GUI-launched
  editors inherit launchd's minimal PATH, which contains none of the
  places olang installs to. VS Code and Zed now try the explicit
  setting, then PATH, then `~/.olang`, `~/.cargo/bin`,
  `/usr/local/bin`, and `/opt/homebrew/bin`, and say what they tried on
  failure. The abandoned `editors/zed-olang` twin (wrong repository
  URL, duplicate language id) is removed, and the Zed grammar pin moved
  from a months-old revision to the current one, picking up the
  template/raw-string/macro tokens.

- **Positions now cross the LSP wire in UTF-16 code units, the
  protocol's default.** The server was char-indexed end to end, so any
  non-ASCII earlier in a line — a `π`, an arrow, an emoji — shifted
  every hover, diagnostic, and definition to the right of reality,
  which is a large part of why the server felt broken in real use.
  Conversion now happens at every boundary, pinned by a protocol test
  with a surrogate-pair emoji ahead of the hovered word.

- **Completions offered keywords that left the language.** `async`,
  `await`, `try`, and `catch` sat in the server's hand-maintained list
  long after their removal, and `meta` was absent; the module list was
  missing `cell`, `chan`, `task`, `proc`, `caps`, and `meta`. The
  vocabularies now come from the grammar's appendix and the help
  registry, and a test diffs them.

- **Nineteen global builtins had no help entry** — the whole `map_*`
  family, `show`, `entries`, `take`, `skip`, `concat`, and friends were
  invisible to `:help` (falling to fuzzy search) and to every editor
  surface that renders from the registry. All nineteen are documented,
  and a coverage test now diffs the dispatcher's name list against the
  registry so the set cannot quietly grow again.

- **The VS Code client failed silently when the binary was missing.**
  A GUI-launched editor often has a shorter PATH than a shell; the old
  client called start() and nothing visibly happened. It now probes
  `olang --version` first and, on failure, shows an actionable error
  with a button to the `olang.serverPath` setting. The TextMate grammar
  caught up with the language: template strings with interpolation and
  escapes, raw strings, `meta fn` and `@` highlighting, numeric bases,
  and the removed keywords removed. The tree-sitter grammar for Zed
  gained the same tokens (`template`, `raw_string`, `macro`, `meta`)
  and dropped the dead keywords, with the parser regenerated and the
  highlight queries extended.

## [0.68.0] - 2026-08-22

### Fixed

- **The keyword appendix and the stability chapter still said "seven
  contextual words" — `meta` is the eighth.** The same class R1 kept
  finding: prose that counts the project's own machinery goes stale
  silently. `language.md`'s appendix now lists `meta` (and the `@`
  forms, with pointers to the macros chapter), and `stability.md` states
  the durable version of the claim: the fifteen *reserved* words will
  never grow, while the contextual words may gain members additively,
  since a contextual word never stops being an ordinary identifier.
  Also stated in the chapter, from the graduation contract: macro names
  are a closed set — only declared `meta fn` names are invocable with
  `@`, so the macros a file can use are exactly its declarations plus
  its imports.

### Changed

- **Macros are stable.** Graduated from experimental with every listed
  criterion met: source-mapped runtime error spans, an expansion-aware
  language server, a 10,000-program fuzz corpus run clean, and three
  macro libraries imported by real programs in the corpus. The
  compatibility contract now covers the model permanently: the syntax
  forms (`meta fn`; `@name(args)` expressions; `@name` decorators on
  `type`, `fn`, and `let`), the five laws, the source-text exchange
  contract with applicative argument order, the `meta` helpers
  (`eval`, `lit`, `fresh`) and `meta.parse`'s pre-expansion view, and
  source-mapped errors as behavior. Deliberately outside the freeze:
  error and diagnostic wording, `olang expand` presentation, and the
  expansion fuel above its floor of 16 rounds. Token and reader macros
  remain excluded by design — the total-parse law is part of the
  contract, so there will only ever be one grammar.

### Added

- **Two more macro libraries, completing the graduation corpus:**
  `examples/language/instrument` (`@memo` — a function rewritten into a cache
  around its own body, recursion memoizing itself, with a generated
  `_cache_size()` so tests pin the cache rather than the clock;
  `@trace`; `@timed`; `@dbg` printing an expression's own source) and
  `examples/language/contracts` (`@require`/`@ensure` contracts that quote the
  violated condition, and `@fmtc`, a format string whose placeholder
  count is checked against its arguments at load). Both imported with
  `use`, both under the harness and the tier-agreement corpus.

- **The macro fuzzer corpus: 10,000 generated programs run clean** —
  expansion determinism (expand twice, byte-identical), tier agreement
  (interpreter vs compiled, whole-program diff), and clean refusal (no
  panics, no internal errors) checked per seed.

### Fixed

- **Nested macro calls in arguments — found by the fuzzer within its
  first hundred seeds.** `@bake(@twice(4))` handed `bake` the raw text
  `@twice(4)`: the site collector cannot see nested calls (arguments
  travel as text), so macros that inspect or evaluate their argument met
  unexpanded `@` source, while template-splicing macros worked only
  because their output was re-expanded next round. Arguments are now
  expanded before the macro runs — applicative order, the rule function
  calls follow, to any depth under the same fuel. The rule is documented
  in the chapter; three regressions pin it.

- **The language server is expansion-aware** — the second graduation
  criterion, done. Syntax diagnostics keep coming from the file as
  written (positions always match the buffer); the semantic pass now
  runs on the *expanded* program and translates every position back
  through the expansion's line map. Calling a macro-generated function
  is no longer a false error; an unused variable below a line-shifting
  decorator is flagged on its true buffer line; a violation inside
  generated code is reported at the `@` site as
  `in code generated by @name: ...`; and a macro that fails to expand
  is an error diagnostic anchored at its site instead of silence.

### Fixed

- **`@name` could invoke any global binding, not just declared macros.**
  `@json` with no such meta fn in scope found the stdlib `json` module
  and tried to call it ("Cannot call non-function value"); `@map(...)`
  would have handed the builtin `map` source strings. Only names
  declared `meta fn` — locally or via `use` — are invocable as macros
  now; anything else is "no meta fn named 'x'" with the declaration and
  import forms named.

- **`use`-imported macro libraries resolve relative to the importing
  file, not the process working directory.** `olang check
  examples/language/derives/main.ol` from the repo root failed to find
  `derive.ol` and fell into the binding bug above; the run path, the
  checker, and the language server now all pass the file's directory as
  the import base, so a macro program means the same thing wherever the
  tool was launched from.

- **Runtime errors in macro programs are source-mapped** — the first
  graduation criterion, done. Expansion now produces a line map: every
  line of the expanded program records whether it is untouched source
  (and which original line) or a macro's output (and which `@` site
  produced it, surviving composition to the line the author wrote). A
  runtime error is translated through the map before display: an error
  in your own code shows your file at the right line with its exact
  content — immune to the line shift a decorator's generated
  declarations cause below them — and an error inside generated code
  points at the `@` site, names the macro, and offers `olang expand`
  for the generated text. The expanded-program note is gone from the
  runtime path; it remains only for whole-file parse errors, where no
  map exists yet.

- **Macro hardening — the M1–M4 campaign** (docs/macros.md), taking the
  experimental macro system from prototype to usable and reliable:

  - **Macro libraries**: a top-level `use m` brings `m`'s top-level meta
    fns into the importing file's expansion. `examples/language/derives` ships
    the first — `@json`, `@builder`, and `@arbitrary` deriving a
    serializer, a builder API, and a *generated test suite* from one
    `type` declaration.
  - **Decorators on `fn` and `let`**, joining `type` — the wrap pattern
    (`@noisy`, `@trace`) documented in the chapter.
  - **Template escapes**: `` \` ``, `\$`, and `\\` produce the bare
    character; every other backslash pair passes through unchanged. A
    macro can now generate a template that interpolates at runtime.
  - **REPL persistence**: a `meta fn` entered in the session stays
    available to later inputs.
  - **`olang expand --diff`** prints only the lines expansion changed.
  - **Error alignment**: a macro-bearing program runs as its expanded
    text, so error line numbers and code context refer to text that can
    actually be shown — with a note under any failure pointing at
    `olang expand`.
  - **Placement rules enforced**: meta fns are top-level declarations
    (a clear error anywhere else), and an `@` site inside a meta fn
    body is a phase error naming the fix.
  - **Determinism hardened**: `meta.fresh` is per-expansion and
    thread-local, so the same source expands byte-identically — under
    parallel test runs and parallel builds too, which a process-global
    counter (the first design) raced on.
  - **Fuzzed**: 600 generated macro programs — substitution, wrapping,
    gensym, and composition shapes — ran clean for tier agreement and
    cross-process expansion determinism; the only failures were the
    generator's own deliberately wrong-arity calls, each correctly
    refused with the site and macro named.
  - The LSP diagnoses macro files against the *unexpanded* buffer (raw
    parse), so its positions always match what the editor shows; the
    parse pre-filter is now token-precise, so a file that merely
    mentions `meta` in a comment pays nothing.

  `stability.md` now states the graduation criteria for macros leaving
  experimental status, so "experimental" is a stage with an exit rather
  than a parking lot.

### Added

- **Macros: `meta fn` and `@` — extending olang in olang
  (experimental).** A `meta fn` runs at load time: it receives the
  source text of its arguments, returns source text, and the parser
  splices the result over the `@` site before the interpreter or any
  tier sees the program. Five laws govern the whole system: every
  expansion site says `@` (no invisible macros — the blast radius of a
  macro is the set of sites that name it); importing a macro never
  changes code that doesn't say `@`; the parse is total (arguments are
  ordinary olang under the one frozen grammar — no token or reader
  macros, ever); expansion is pure (meta fns run in meta mode, where
  `fs`, `http`, `db`, `proc`, `os`, `time`, `random`, `task`, `chan`,
  `spawn`, and the parallel builtins refuse — so expansion is a
  deterministic function of the source, and record/replay is exact);
  and expansion cannot hide (`olang expand` prints the program the
  runtime receives, meta fns blanked with line numbers preserved,
  errors naming the macro and call-site line).

  Two invocation forms: `@name(args)` in expression position, and
  `@name` stacked above a `type` declaration (the derive form — the
  macro receives the declaration's source, reads its fields through
  `meta.parse`, and returns the declaration plus generated code;
  stacked decorators apply nearest-first). Macro output may contain
  further `@` sites, expanded on later rounds under a 16-round fuel
  with the loop named on exhaustion.

  Three new `meta` functions complete the loop: `meta.eval(source)`
  evaluates source in the same pure sandbox and returns `Result` —
  with `meta.lit(value)`, which renders a value back into source, that
  is compile-time computation in userland (`meta fn bake(e) =
  meta.lit(unwrap(meta.eval(e)))`); `meta.fresh(prefix)` yields
  collision-free names for generated temporaries. The Open AST's
  `type` node now carries its definition (struct fields with their
  annotations, enum variants, union members), which derive macros
  read; `meta.parse` stays the pre-expansion view, showing `@` sites
  as `macro_call` nodes.

  The syntax is additive: `@` was previously unused, and `meta` stays
  an ordinary identifier everywhere except directly before `fn`.
  Expansion happens inside `Parser::parse`, so files, the REPL, doc
  tests, and `olang build` all agree; macro-free source pays a
  substring scan and nothing else. Generated code is ordinary code —
  checked, promoted, and capability-gated like anything handwritten.
  Documented in a new book chapter (docs/macros.md), demonstrated in
  `examples/language/macros` (`@bake`, `@unless`, `@dbg`, a `@json` derive),
  and pinned by a 20-case test matrix plus the tier-agreement corpus.

### Fixed

- **Three common-mistake error messages now point at the fix instead of
  describing the runtime's fallback** — one coherent error-UX pass over
  the diagnostics a newcomer hits first:
  - Calling a method with no `impl` said "Field 'area' not found",
    conflating a method with a struct field. It now says "no method
    'area' for T", and when the declaring trait is known, "the trait S
    declares it, but there is no `impl S for T`".
  - Iterating a map said "Cannot iterate over Map({\"a\": Integer(1)})",
    leaking the interpreter's internal Debug shape. It now names the type
    and suggests `for (k, v) in entries(m)`.
  - Indexing a map or struct with `[]` said "Index must be an integer".
    It now says a Map is read with `map_get(m, key)` and a struct by
    field, since `[]` never applies to them.

  Each is fixed identically on the interpreter and the bytecode tier, so
  the two engines word the error the same — the tier-transparency the
  execution model promises. Surfaced by a soak pass that found the same
  shape of misdirection repeatedly: olang's happy paths are sound, and
  the rough edges cluster in diagnostics for plausible mistakes.

- **A malformed contextual-keyword declaration no longer reports the
  keyword as an undefined variable.** `error`, `share`, `test`, and
  `trait` are contextual keywords — ordinary identifiers everywhere else —
  so when their declaration form does not parse, the parser falls back to
  reading the keyword as a variable reference. Writing `error E { code:
  Int }` (an error type is variants, not struct fields) therefore failed
  with "Undefined variable: error", which sends a reader to look for a
  missing binding rather than at the malformed declaration. The error
  message now recognizes these four keywords and points at the correct
  declaration form. It is the flip side of the contextual-keyword design:
  freeing the names for use as identifiers means a broken declaration
  misparses as one. Found during a soak pass over the error model (the
  model itself — `error` types, `?`, structural recovery via `task.join`
  — is sound).

- **An undeclared type in an annotation is reported as unknown, not as a
  value mismatch.** A field, parameter, return, or `let` annotation naming
  a type that was never declared — `f: Widget` where no `Widget` exists —
  reduced to a name-comparison check that nothing matched, so enforcement
  blamed the value: `field 'f' of T expects Widget, got Int`. A reader who
  typoed a type name or referenced one they forgot to declare was sent to
  debug the value. It now says the annotation names an unknown type, the
  same way constructing an undeclared struct already reports one. The
  enforcer learned the declared enum *type* names (struct names it already
  had), so it can tell an unknown type from a real mismatch — a wrong value
  against a declared `enum` is still an ordinary "expects Color, got Int".
  Fixed identically on both engines: the interpreter's four enforcement
  sites and the bytecode tier's four, so the two never disagree. Forward
  references still resolve (enforcement is lazy, at construction). The
  static checker (`olang check`) still words this as a mismatch; correcting
  it there needs a verified-complete type registry across imports and is
  left as its own task. Found during a `cell` stress pass.

- **Template strings no longer strip leading whitespace.** `` `  x` ``
  evaluated to `"x"` — the leading spaces and tabs vanished, asymmetric
  with trailing and interior whitespace, which were always preserved. The
  cause was one character in the grammar: `template_string` was a plain
  rule, so pest consumed the implicit `WHITESPACE` between the opening
  backtick and the content; it is now compound-atomic. The bug was
  pre-existing and had been silently eating the indentation of report
  lines like `` `  utilization ${bar}` `` in `examples/concurrency/demo`; those now
  render with their intended indent. Found while migrating the corpus off
  `+ to_string(...)` (below), where every indented line hit it.

### Changed

- **The corpus is migrated off the `+ to_string(...)` string-building
  idiom to template strings** — both the examples (~80 sites, 25 files)
  and the book (53 sites across `types`, `tour`, `language`, `ods`, and
  `stdlib`). `"count: " + to_string(n)` becomes `` `count: ${n}` ``;
  templates auto-stringify any value, so the explicit conversion — the
  most visible friction in day-to-day olang — disappears from label and
  report strings. Two cases are deliberately kept as `to_string`: a value
  used as a standalone string (a map key, a function argument), and —
  subtly — the interpolation of a value that *might be a String*, because
  `to_string("x")` renders `"x"` with quotes while `${x}` renders it
  unquoted, so they are not equivalent there (this is why `stdlib.md`'s
  `show` vs `to_string` teaching block is untouched). The book migration
  is verified byte-for-byte by `doc_examples_test` and `doc_outputs_test`.
  The additive mechanism (template strings) already existed; this
  displaces the habit.

### Added

- **`examples/data-processing/timeseries` — analysis over ordered data.** The sequence
  counterpart to the bag examples (`dataproc`, `meterflow`), and the
  first program to exercise the window verbs shipped in 0.67 (`rolling`,
  `shift`, `cum_max`/`cum_min`, `rank`), which until now had only
  unit-test coverage. A service's daily latency over four weeks —
  generated deterministically, so the run reproduces and the `test`
  blocks can pin the windows — is run through a 7-day moving average, a
  day-over-day delta, a running worst-case, and a severity rank, each
  added as a column, so an incident hidden in the weekly noise surfaces
  by ordering rather than a hand-picked threshold. Discovered by
  `run_all.ol`, covered by `tier_agreement_test`, and in the website
  gallery.

## [0.67.0] - 2026-08-17

### Changed

- **`docs/stability.md` rewritten as the 1.0 compatibility contract
  (R2).** The chapter now opens as the contract itself rather than a
  "stable in shape" status note. Its versioning section states what a
  major, minor, and patch release each mean from 1.0 onward, with the
  breaking-change bar made concrete: a change breaks compatibility
  exactly when it makes a documented example fail in CI, so "is this
  breaking?" is a test result, not a judgment call. It defines the
  deprecation policy (an alias for at least one minor release) and the
  bug-fix-is-not-a-break distinction, and adds a per-surface guarantee
  table saying what a user can rely on at each stability tier. The
  surface freeze it describes is already real; 1.0 changes the version
  number and the promise's formality, not the code.

### Added

- **Eight pre-1.0 bug-hunt regressions** in `bytecode_tier_test.rs`. A
  differential fuzzer ran ~10,000 generated programs under `--no-ovm`
  and `--ovm-tier=1` and found no tier divergence across arithmetic,
  closures, strings, structs, `Result`/`?`, loops, `cell`, enums, and
  nested data. The hunt's durable output is the seams it swept that the
  suite had never named as concrete cases: a JIT deopt on a kind change
  (int-specialized function called later with a float), an overflow
  guard firing inside a hot function, signed division and remainder,
  a float accumulator whose rounding error must match bit-for-bit, and a
  loop snapshotting closures — the shape of the capture bug the harness
  caught earlier. Each is now pinned identical on both tiers.

### Fixed

- **A capability leak across the thread boundary (OM1).** Campaign 3's C1
  moved capability enforcement to the single dispatch point both tiers
  share, and measured it free on the main thread. It left a hole across
  threads. A worker — `spawn`, `par_map`, `par for`, or an `http.serve`
  handler — runs its own interpreter with its own bytecode tier, built by
  `thread_safe_clone`, and that clone carried the capability *table* but
  never seeded it into the worker's tier. A promoted function in a
  dependency denied `fs` could therefore read the filesystem from a
  spawned thread and return the bytes, while the identical direct call on
  the main thread was correctly refused. The leak was invisible to every
  test that did not cross a thread boundary.

  The same gap ran in reverse for `--trace-caps`: a worker's effects
  never reached the profile, so `--trace-caps --write` would author a
  manifest omitting them and then, on the next run, deny them. One fix —
  seed the worker tier's gate, and share the trace set (a `Arc<Mutex>`)
  onto the worker rather than starting it empty — closes both. A
  restricted `fib(30)`×4 across spawned workers still runs at full tier
  speed (12 ms, versus ~19 s interpreted), so enforcement remains one
  branch per call, not a tier downgrade. Three tests forge the leak.

### Added

- **DP3 Tier 3c: `shift`, `cum_max`/`cum_min`, `rank`, `rolling`.** The
  window functions, which close DP3.

  `ods.shift(s, by)` moves values down the column, filling what is
  vacated with nulls — not a wrapped value, because a window has an edge
  and the row before the first row does not exist. A difference then
  falls out of the operators already there (`s - ods.shift(s, 1)`), which
  is why there is no separate `diff` verb.

  `ods.cum_max` and `ods.cum_min` are the running extremes, the shape
  `cumsum` already had. `ods.rank(s, method)` ranks smallest first with
  the five standard tie treatments — min, max, average, ordinal, dense —
  defaulting to `"min"`, the competition ranking "rank" means
  unqualified. Nulls rank as null, since every other reduction skips
  them and giving them a position would place them somewhere silently.
  Only `average` can produce a half, so the other methods return an Int
  column usable as indices without a cast.

  `ods.rolling(s, window, agg)` is a trailing-window aggregate. The
  first `window - 1` elements are null: the window is not yet full, and
  reporting a partial reduction as a whole one is how a chart lies at
  its left edge. Nulls inside a window are skipped exactly as the
  whole-column reductions skip them, and an all-null window sums to null
  rather than to zero, since no data is not the measurement zero.

  It recomputes each window rather than keeping a running total, at
  O(n × window). The incremental form accumulates float drift a fresh
  sum does not, which would make a rolling mean disagree with the `mean`
  of the same window — so correctness first, and the cost is stated in
  the docs rather than hidden. If a wide window over a long column shows
  up in a measurement, that is the point to revisit it, the same
  discipline DP1c settled.

- **DP3 Tier 3b: `pivot` and `unpivot`.** The two shapes of the same
  data — long, which is what a database returns and what `group_by` and
  the plotting verbs want, and wide, which is what a person reads — and
  the pair of verbs that moves between them.

  `ods.pivot(f, index, columns, values, agg)` spreads. Its aggregation is
  a `group_by`, literally the same call, so pivoting and grouping cannot
  disagree about how a column reduces or what happens to nulls; what
  `pivot` adds is the scatter. There is a test asserting the cells equal
  the groups.

  The aggregation is required rather than defaulted: when a cell has more
  than one row behind it, which reduction applies is the caller's
  decision, and choosing one silently is how a wrong number reaches a
  report. A cell no row reached is null, which is a different fact from a
  null value in it. Two shapes are refused rather than guessed at — a
  null cannot name a column (and calling it `"null"` would collide with a
  genuine `"null"` string), and a value that would name an existing index
  column is refused rather than overwriting it.

  `ods.unpivot(f, ids, value_columns)` gathers. Omitting the value
  columns takes everything that is not an id, which is the form that
  survives a new column arriving upstream. The value columns stack into
  one column, so they must share a type; mixing them would mean choosing
  a common type for the caller, which is `cast`'s explicit job, so it
  refuses and names the two columns that disagree.

  `unpivot` after `pivot` returns the long form with the combinations
  that never occurred present as nulls, leaving `drop_null` as the
  caller's decision rather than one the verb made silently.

- **DP3 Tier 3a: `join_full`, `join_semi`, `join_anti`.** The rest of the
  join kinds, completing the set `join` and `join_left` started.

  `join_full` keeps every row from both sides. Its key column takes
  whichever side has a value, so a row that came only from the right is
  still identified by its key instead of being null in the one column
  that says what it is. It is also the only kind where mismatched key
  types matter: keys hash by type, so an Int column never matches a Float
  one, and the other kinds simply find nothing — a full join returns rows
  anyway, which is exactly when the mismatch would be papered over. So it
  refuses, naming both types.

  `join_semi` and `join_anti` ask about existence rather than
  combination, returning the left frame's columns alone. The distinction
  from an inner join is the multiplication: where the right side has
  three rows for a key, an inner join returns three and a semi join
  returns one. Every kind keeps the same null rule — a null key matches
  nothing — so semi and anti always partition the left frame between
  them, which is tested.

- **DP3 Tier 2: `value_counts`, `unique`/`n_unique`, `median`, `cast`,
  `sample`.** What a pipeline reaches for once `describe` has shown it the
  shape of the data.

  `ods.value_counts(s)` returns a value/count Frame, most frequent first,
  ties breaking by first appearance. `ods.unique(s)` gives the distinct
  values in first-seen order and `ods.n_unique(s)` counts them without
  building the column. All three run through the same key-identification
  pass `group_by` uses, so they cannot disagree with it: a null is a value
  (its own entry, not a skip) and all NaNs are one value, both differing
  from `==` on the corresponding scalars, and both matching what a
  `group_by` on the column already did.

  `ods.median(s)` is defined as `quantile(s, 0.5)` rather than computed
  again, joining the named reductions where its absence beside `mean` and
  `std` was the anomaly. `describe`'s median row is the same call, so the
  three agree by construction.

  `ods.cast(s, type)` converts between `"Float"`, `"Int"`, `"Bool"`, and
  `"String"` — the names `schema` reports. A value the target cannot hold
  becomes null rather than an error or a wrong number: one unparseable row
  should not fail a load, and `null_count` then reports exactly what was
  lost. Float to Int truncates toward zero, and a float that is infinite,
  NaN, or out of Int range becomes null instead of the saturated value a
  raw hardware conversion produces. Float to Bool is refused outright,
  since `0.5` is neither, with the error naming the comparison to write
  instead.

  `ods.sample(f, n)` draws random rows from a Frame or a Series, without
  replacement, returned in the frame's original order rather than draw
  order — a sample is meant to be read, and shuffling as a side effect
  makes a sample of sorted data unreadable. Asking for more rows than
  exist returns all of them, as `head` and `tail` do. Randomness comes
  from the stream `random.seed(k)` already governs, so there is one seed
  to set rather than one per verb. That stream is process-wide, which is
  documented as the reason a seed does *not* make sampling reproducible
  inside `par_map` — several threads drawing from one stream interleave.

- **DP3 Tier 1: `rename`, `drop`, `distinct`, `tail`, frame-level
  `drop_null`.** The five verbs a first pipeline reaches for before any
  of the ones the original DP3 list named. `ods.rename(f, mapping)`
  renames any number of columns from a Map, refusing an unknown name and
  refusing a target that already exists — its immediate use is undoing
  the `_right` suffix a colliding join leaves behind, which until now
  made the joined column unnameable. `ods.drop(f, names)` is `select`'s
  complement, and survives a new column arriving upstream where the
  equivalent `select` would silently discard it. `ods.tail(f, n = 10)`
  answers `head`; asking for more rows than exist returns the whole
  Frame. `ods.distinct(f, names = all)` removes duplicate rows keeping
  the first occurrence, and `ods.drop_null(f, names = all)` removes rows
  that are null anywhere — both over whole rows, both accepting a column
  subset. Row identity length-prefixes each field rather than joining
  with a separator, so no value can impersonate a field boundary and a
  null never collides with an empty string.

  Frame-level `fill_null` was on the Tier 1 list and is deliberately not
  here: `ods.with_column(f, "amount", ods.fill_null(f["amount"], 0.0))`
  already expresses it through the subscript added in 0.66, and a second
  spelling of an existing operation is surface area without capability.

### Added

- **`tests/doc_outputs_test.rs` — a `// comment` that states an output
  must state the real one.** The book had two guards: one proving its
  programs run, one proving the names it drops exist. Neither looked at
  the *value* a comment claimed a line prints, so an example could run
  perfectly while teaching something false. Fourteen did.

  A list of strings prints with its quotes, and seven comments wrote them
  bare (`// [west, east]` for `["west", "east"]`). A Float column prints
  with its `.0`, and five dropped it. `show` on an enum variant qualifies
  it — `Color.Blue`, not `Blue`. And three comments naming a regression's
  true coefficient read as claims about a line printing something else.

  These are the comments a reader trusts most, being the only place the
  book says what a value *is* rather than what a function does. The
  guard's design problem is telling a claimed value from a note, since
  most comments are notes and flagging them would make it a check someone
  switches off: it accepts only what is unambiguously a value, and skips
  any block where a `println` emitted more than one line rather than
  guessing at the correspondence.

### Fixed

- **Five corrections from R1's final sitting, closing the book audit.**
  `introduction.md` warned of a breaking release that already shipped
  and told projects to pin against a migration now in the past.
  `openness.md`'s closing line promised Campaign 3 as future work; it
  shipped in full. `language.md` carried the third home of the
  "intersection annotations exist for future use" claim (they do not
  parse; unions have had semantics since 0.50), and its keyword appendix
  said `for` iterates maps (it does not) while omitting tuples (it
  does). `stdlib.md`'s conventions named three modules as needing `use`;
  it is seven — `term`, `ui`, `viz`, and `dash` also do, verified by
  probing all of them. With these, R1 is complete: every chapter read or
  probed against the binary, roughly thirty corrections across eight
  sittings, and four permanent guards holding what a test can hold.

- **`docs/types.md` repeated a claim its source chapter had already
  corrected.** It said intersection annotations "parse today and gain
  semantics later" — false, they are a parse error — and linked to the
  anchor of the `stability.md` heading whose correction renamed it. A
  heading rename is an API change for every chapter that links to it,
  and nothing checked those links; `tests/doc_anchors_test.rs` now does,
  indexing every heading and verifying every `chapter.md#anchor` in the
  book. The guard was proven able to detect a break before being
  trusted.

- **`docs/ovm.md` credited two test suites for a guarantee that has
  three.** The chapter's correctness policy is the statement
  `tier_agreement_test.rs` was built to enforce — the harness quotes it
  by name — and the harness was missing from the list. It is the third
  chapter found with a stale suite list, after `stability.md` and
  `internals.md`.

  In the compiler, `src/ovm/bytecode.rs` introduced its
  compilable-builtin set by saying higher-order builtins are excluded
  "because a function argument cannot reach the VM", then listed `map`,
  `filter`, `reduce`, and `fold` 130 lines later under a note explaining
  that they became reachable. The leading comment is what a reader meets
  first.

- **`docs/internals.md` documented a gate that does not test the
  workspace.** "Gates for every change: `cargo test --release`" — but the
  repository root is both a package and the workspace root, so a bare
  `cargo test` tests `olang` alone and never reaches the `olang-ods`
  engine crate's property tests or `otc`'s. A kernel change could pass
  every documented gate and ship broken. Corrected to `--workspace`, with
  the reason stated.

  The same chapter described `List(Arc<[Value]>)`; it is
  `Arc<Vec<Value>>`, and the difference is load-bearing — a boxed slice
  cannot grow, so the in-place append fusion olang ships would be
  impossible under the representation documented. Its repository map was
  missing eight files including three shipped campaigns (`caps.rs`,
  `timeline.rs`, `scoping.rs`), and its testing table credited
  `example_programs_test.rs` with example files that no longer exist
  while listing neither the whole-program tier harness nor either doc
  guard.

- **The ods chapter now uses the subscript it teaches.** `ods.md` gained
  a section in 0.66 explaining that `f["amount"]` is how a column is
  reached, then went on calling `ods.column(f, "amount")` in every
  example around it — a function the chapter never introduces in prose,
  so a reader met it six times before being taught the explained way to
  do the same thing. The examples use the subscript throughout, and
  `column(f, name)` is now named in the stdlib reference, which had also
  been missing it. `ods.version()` is documented for the first time.

- **`olang bench --help` now lists the flags it accepts.** `--runs`,
  `--save`, `--against`, and `--fail-on-regress` are parsed by the
  benchmark runner itself out of forwarded arguments, so clap knew
  nothing about them and printed no options at all — leaving the book as
  the only place they existed.

- **`olang doc --md` corrected to `--markdown` in the book.** The flag
  `docs/tooling.md` documented twice has never existed under that name.

- **Four documentation claims that were true of an older olang** (R1's
  prose pass, second sitting). Found by probing rather than reading:
  writing the program each falsifiable claim implies, and checking that
  the binary agrees.

  `docs/stability.md` — the chapter that declares itself authoritative
  wherever other text disagrees — headed a section "Reserved — parses
  today, semantics later" over two constructs that do not parse at all;
  omitted the `caps` module from every list; credited only the
  per-function tier tests for tier agreement, predating both the
  whole-program harness and the doc-reference guard; and illustrated
  semver with 0.25 → 0.26.

  `docs/pitfalls.md` warned that `for _ in ...` is a parse error. It has
  not been since the papercut batch, so the pitfall told readers to
  avoid something that works. The real rule beside it was documented
  nowhere: an identifier may not begin with an underscore, so `_unused`
  fails with `expected the end of the file` — an error naming neither
  the underscore nor the line's actual problem.

  `docs/language.md` did not record that `for` iterates tuples, or that
  `()` is a literal rather than only a value one arrives at.

### Changed

- **DP1c closed without work, on measurement.** The lane assumed
  `group_by`'s key-identification pass dominates its runtime. Over
  2,000,000 rows it does not: grouping into 4 groups takes 10ms and into
  64 groups 9ms — memory-bandwidth territory, where threads do not help.
  The pass only costs anything (196ms) when nearly every row is its own
  group, which is a `sort_by` wearing a `group_by` costume, and which is
  also the case where parallelising is hardest: ids are assigned in
  first-seen order and the output row order depends on it, so a parallel
  version needs a merge that orders 2M distinct keys by their global
  minimum first-seen row. The numbers are recorded in the roadmap so the
  question is not reopened from the same wrong premise.

- **DP3 respec'd by evidence rather than category.** The original list
  (window functions, reshape, further joins) predated anything trying to
  use the data stack in anger. DP4's flagship then found four missing
  verbs that were not on it — `concat`, mask combination, the join-key
  default, scalar `eq`/`ne` — all of which a first pipeline cannot do
  without. DP3 is now tiered: what the next pipeline hits immediately
  (`rename`, `drop`, `distinct`, `tail`, frame-level null handling), what
  it reaches for once `describe` has shown the shape (`value_counts`,
  `unique`, `median`, `cast`, `sample`), and the original list held until
  a real workload asks for it.


## [0.66.0] - 2026-08-17

### Added

- **A Frame prints as a table.** `display()` gave a shape and a type
  list, which is what a debugger prints, not what an exploratory session
  wants. Printing a Frame — at the REPL, through `println`, through
  `to_string` — now renders the data:

  ```text
  Frame[2 x 3]
  ┌────────┬────────┬─────┐
  │ region │ amount │ qty │
  │ String │  Float │ Int │
  ├────────┼────────┼─────┤
  │ east   │   25.5 │  10 │
  │ west   │  320.0 │   3 │
  └────────┴────────┴─────┘
  ```

  Numeric columns are right-aligned so digits line up, text columns are
  left-aligned, and nulls print as `—`. Four caps keep a large Frame
  inside a terminal — twenty rows, twenty-eight characters per cell, a
  hundred characters of width, and however many columns fit — and each
  one that bites is reported under the table, because a table that
  silently dropped a column would be worse than the summary it replaced.
  A long Frame keeps both ends and elides its middle, which is what makes
  the result of a `sort_by` readable at a glance.

- **`ods.head(f)` defaults to 10 rows.** The verb typed most often at the
  REPL was the one that raised most often, because it demanded a count.

- **`meterflow`: the data stack's flagship ETL (Campaign 2, DP4).**
  A multi-source pipeline in [`examples/data-processing/meterflow/`](examples/data-processing/meterflow/):
  meter telemetry arrives as JSON lines too large to hold, two CSV
  dimension tables carry sites and tariffs, and the job streams the
  readings, gates them on quality, aggregates across chunks, joins the
  dimensions, prices the result, caches it in the native columnar format,
  and charts it.

  Streaming turns one pass into many partial aggregates, so the pipeline
  is correct only if chunk boundaries cannot change the answer. Over
  400,000 readings the streamed run and a whole-file computation agree
  exactly — 390,033 rows kept and 781,262.26 kWh either way, across 80
  chunk boundaries.

  **It was built to find what was missing, and it did.** Four gaps
  surfaced in the writing, all now closed:

  | | |
  |---|---|
  | `ods.all_of(masks)` · `ods.any_of(masks)` · `ods.not(mask)` | combine Bool masks, three-valued like SQL |
  | `ods.concat(frames)` | stack Frames, matching columns by name |
  | `ods.join(a, b, on)` | the second key name now defaults to the first |
  | `ods.eq(s, v)` · `ods.ne(s, v)` | now accept a plain value, as `==` already did |

  Mask combination was the blocking one: a filter with two conditions was
  simply not expressible. `&&` cannot serve, because the language compiles
  it to a conditional jump so that it can short-circuit, and that has no
  elementwise reading over a column — so these are functions taking a
  *list*, since a real filter has three or four conditions rather than
  two. `concat` was the other: without it, putting the partial results of
  a streaming loop back together required a detour through row-shaped
  values, which is exactly what the columnar representation exists to
  avoid.

- **A native columnar file format (Campaign 2, DP2 — the last lane).**
  CSV and JSON lines are interchange with the outside world and both pay
  for it: every load re-parses text and re-infers types, and neither can
  record what a column *was*. A column of zero-padded codes written to
  CSV comes back as integers with the padding gone.

  | | |
  |---|---|
  | `ods.write_frame(f, path)` | write a Frame → `Result<Unit>` |
  | `ods.read_frame(path, columns = all)` | read one back → `Result<Frame>` |
  | `ods.frame_info(path)` | its schema, from the header alone → `Result<Frame>` |

  Types survive exactly, the load is a read rather than a parse, and the
  optional column list decodes only what is named — the rest is stepped
  over using the byte lengths in the header. Over 500,000 rows and six
  columns: **CSV 138ms, columnar 50ms, one column 26ms**, and the file is
  15MB against the CSV's 17MB.

  Arrow and Parquet were declined: both are a dependency and both are
  opaque, which is the wrong trade for a language whose case is that the
  artifact should be inspectable. The header is UTF-8 text, one line per
  column, readable with `head`:

  ```text
  olang-columns 1
  rows 500000
  columns 6
  col "id" Int enc=plain nulls=0 bytes=4000000
  col "region" String enc=dict nulls=0 bytes=2000066
  data
  ```

  `enc` is how a String column is stored, and it exists because the
  obvious layout lost. Writing each row's text with its own offset made
  the file *larger than the CSV* — 24MB against 17MB — since an eight-byte
  offset costs more than the four characters it points at, and repetition
  is the normal case in a table. A repeating column is now written once as
  a dictionary of its distinct values plus one small code per row. The
  writer chooses by computing both sizes and taking the smaller, so there
  is no threshold to tune and no case where the choice is a guess.

- **A Frame is subscriptable: `f["amount"]`, `f[mask]`, `s[i]`.** Reaching
  a column was the most repeated gesture in data code and the most
  verbose: `ods.column` and `ods.get` together were **13.6% of all 937
  `ods` calls in this repository**, and `ods.column(raw, "amount") *
  ods.column(raw, "quantity")` is 62 characters to say one multiply.

  ```olang
  let sales = ods.with_column(raw, "revenue", raw["amount"] * raw["quantity"])
  let big   = sales[sales["revenue"] > 100.0]
  ```

  | subscript | means |
  |---|---|
  | `f["amount"]` | the column, as a Series |
  | `f[mask]` | the rows a Bool Series keeps |
  | `f[0]` | **refused**, naming `ods.head` / `ods.take` |
  | `s[3]`, `s[-1]` | the element; negatives count from the end, as in lists |

  Two meanings for one subscript, told apart by the key's type. Refusing
  the third is the point: pandas spells column selection, row filtering,
  positional slicing, and an error all `df[x]`, which is why `.loc` and
  `.iloc` had to be invented on top of it.

  A column name that is not there raises — a literal name in the source
  is a claim about the data's shape, and a wrong claim is a wrong program
  — and the message lists the columns that do exist. Subscripting is
  read-only; there is no `f["a"] = x`, because Frames are values and an
  assignment that silently produced a copy would be a trap.

  The mechanism is one trait method, `NativeObject::index`, defaulting to
  `None`, reached from the index path of *both* tiers. Any native value
  can now define its own subscript; the ones that don't keep the
  language's existing error.

- **`ods.describe(f)` and `ods.schema(f)`.** After `head`, `describe` is
  the first thing typed against unfamiliar data. Both return a **Frame**
  rather than a map, so they print as tables and can themselves be
  sorted, filtered, and written out. `describe` gives count, nulls, mean,
  std, and the five-number summary per column; `schema` gives name, type,
  and null count, for a Frame too wide to summarize. A Frame has one type
  per column, so the numeric statistics are null for String and Bool
  columns rather than absent — a result whose shape depended on its input
  would push the branch onto every caller.

- **`ods` reads and writes JSON lines (Campaign 2, DP2).** One JSON
  object per line is what log shippers, event queues, and export jobs
  emit, and the stack could not read any of it.

  | | |
  |---|---|
  | `ods.read_jsonl(text)` | parse JSON-lines text → `Result<Frame>` |
  | `ods.read_jsonl_file(path)` | read a file → `Result<Frame>` |
  | `ods.open_jsonl(path)` | stream a file → `Result<Reader>` |
  | `ods.to_jsonl(f)` | serialize a Frame → `String` |
  | `ods.write_jsonl(f, path)` | write a Frame → `Result<Unit>` |

  Columns are the union of the keys and a missing key is a null — the
  same rule as `frame_from_records`, because this is that function with a
  parser in front of it. Blank lines are skipped, since a trailing
  newline is how nearly every writer ends the format. A malformed or
  non-object line is an `Err` naming the line number, which on a
  million-line file is the whole diagnostic. `to_jsonl` omits nulls
  rather than writing them, so a Frame round-trips through `read_jsonl`
  to itself.

  **The streaming reader is now one type over both formats.** `open_csv`
  and `open_jsonl` both return a `Reader`, and `next_chunk`, `rows_read`,
  and `at_end` drive either — a streaming loop names its format once, at
  the `open_`, and a function that takes a reader takes both. A
  300,000-row JSON lines file streams at 30.6MB against 471MB read whole.
  JSON lines carry no header, so a reader remembers the columns its
  earlier chunks established and returns them on the empty chunk that
  ends the loop, keeping the promise the CSV reader makes from its
  header. The file-reaching calls demand `fs` at the matching level; the
  text parser and serializer stay pure.

- **`ods` streams files larger than memory (Campaign 2, DP2).**
  `read_csv_file` holds the whole table at once, which is the right shape
  until the file no longer fits. `ods.open_csv` returns a reader that
  hands back one chunk at a time, each an ordinary Frame:

  ```olang
  let r = unwrap(ods.open_csv("events.csv"))
  let mut total = 0.0
  loop {
      let chunk = unwrap(ods.next_chunk(r, 50000))
      if ods.n_rows(chunk) == 0 => break
      total = total + ods.sum(ods.column(chunk, "amount"))
  }
  ```

  Memory is set by the chunk size rather than the file size. Measured
  over a 1M-row CSV: the whole-file read peaks at 196.5MB, the streamed
  pass at **12.2MB** — 0.2MB more than the same pass over a file five
  times smaller. `ods.rows_read(r)` and `ods.at_end(r)` report progress;
  the final chunk is empty but still carries the file's columns, so a
  pipeline written against a chunk needs no special case for it. Asking
  for zero rows is refused rather than silently ending the loop.
  `open_csv` demands `fs` at read level, like `read_csv_file`.

  Two constraints from the language picked this shape. A callback API —
  `(chunk) => { total = total + chunk }` — is refused by 0.62's capture
  rule, the accumulator being a captured write; and a fold taking an
  olang lambda cannot live in `ods` at all, because `OvmModule::dispatch`
  receives `(func, args)` and no interpreter, which is exactly what lets
  both tiers dispatch identically. The loop needs no closure, so it needs
  no exception.

- **Thread confinement is now a property a native value can declare.**
  `cell` was confined to its creating thread by code that named cells
  specifically. That check moved onto the `NativeObject` trait as
  `confined_to()`, so `spawn` and `chan.send` refuse any confined value
  without knowing what it is. The CSV reader — which holds a file
  position, and so is exactly as unsafe to share as a cell — was refused
  at both boundaries before a line was written for it.

- **The data stack can reach files, and write them (Campaign 2, DP2).** `ods` could parse CSV *text* and had no way to emit
  anything at all, so the chapter described an end-to-end story the stack
  could not finish. Three functions close the loop:

  | | |
  |---|---|
  | `ods.read_csv_file(path)` | read a CSV file into a Frame → `Result` |
  | `ods.to_csv(f)` | serialize a Frame to CSV text → `String` |
  | `ods.write_csv(f, path)` | write a Frame to a file → `Result` |

  `to_csv` is the exact inverse of `read_csv`: nulls become empty cells,
  which is what `read_csv` reads back as null, and a value containing a
  comma or a quote is quoted, so the column count survives a round trip.
  It cannot fail, so it returns the string outright; the two that touch
  the disk return `Result`, because a missing file is the caller's input
  rather than their mistake.

  **Both file-touching calls demand `fs`**, at read and write level
  respectively — every other `ods` function stays pure and needs no
  grant. Without that, `ods` would have become a filesystem capability by
  the back door, which is the hole `db.open` had before 0.60.

### Changed

- **`caps::check` is now defined in terms of `caps::required`.** They were
  two independent walks over the builtin surface that happened to agree,
  under a comment claiming they could not disagree. Adding the `ods` file
  calls to `required` and not to `check` proved otherwise: `--trace-caps`
  reported the read as an `fs` effect while `--deny fs` cheerfully allowed
  it. One classification now answers both questions — "what would this
  need?" and "does this grant permit it?" — so the drift is not
  expressible.

- **Capabilities no longer switch the bytecode tier off (Campaign 3, C1).**
  A `[capabilities]` manifest or a `--deny` flag used to force the whole
  run onto the interpreter, so turning on the security feature turned off
  the performance work. On `fib(30)` that cost **1497 ms instead of 4 ms**
  — a sandboxed program was a different program, performance-wise, than an
  unsandboxed one.

  The gate was never the missing piece: `BuiltinFunctions::call_internal`
  is the one choke point both tiers already pass through. What was missing
  was *context*. The tier reaches builtins it cannot run natively through a
  bridge interpreter — a separate `Interpreter` — which carried no
  capability table and no notion of which function was executing, so it
  presented every call as unrestricted and unattributed. Enforcing
  partially being worse than not enforcing, the tier was stepped aside.

  Compiled functions now carry the file they were declared in;
  the VM keeps a stack of those files as it executes, mirroring the
  interpreter's `coverage_file_stack`; and the grant table, the
  `--trace-caps` set, and the executing function's file are handed to the
  bridge before each dispatch. A promoted function in an attenuated
  dependency is judged by *that dependency's* grant, and the profiler sees
  its effects:

  ```
  capability 'fs' denied: fs.exists requires it, and dependency 'lib' is
  granted fs=false ... (olang.toml [capabilities.dependencies.lib])
  ```

  A restricted run now shows the same promotion count, the same bytecode
  calls, and the same wall time as an unrestricted one. An unrestricted run
  pays one branch per call for the attribution stack and is unchanged at
  4 ms.

  Record/replay (`--record`) still runs interpreter-only; the timeline has
  its own reason (it must observe every nondeterministic call, in order)
  and its own lane.

### Removed

- **The bytecode tier's value model drops six unreachable variants.** 0.63
  deleted `async`/`await`/`Promise` from the grammar, parser, AST, and
  interpreter and stopped there: `ValueData` still carried `Promise`, and
  beside it `Thunk`, `Stream`, `LazyList`, `CompiledFunction`, and
  `OptimizedValue` — a lazy-evaluation and self-optimization scheme that
  no code ever constructed. Nothing failed when they were left behind,
  which is why they survived four releases.

  Gone with them: their object structs, the twelve supporting types that
  existed only as their fields (`GeneratorFunction`, `TransformationChain`,
  `OptimizationData`, `TypeFeedback`, `GcMap`, and so on), `PromiseState`,
  the VM's `PromiseError`, and the six dead lazy-evaluation variants of
  `InterpreterError`. `FunctionObject` loses its `optimization_data` field,
  which was written by `Default` and never read. `src/ovm/nanbox.rs` — 309
  lines marked "not yet wired", imported by nothing — is deleted too.

  Behavior is unchanged: no construction site existed, so no program could
  reach any of it. `fib(30)` still runs in 4 ms. This is a Rust-API break
  for anything embedding olang as a library, and 320 fewer lines in the
  file every tier value passes through.

- **`async`, `await`, `try`, `catch`, and `Promise` are ordinary
  identifiers.** 0.63 and 0.65 removed the constructs but kept grammar
  stubs so the parser could emit migration errors naming the
  replacements. There is no olang code outside this repository, so there
  was nobody to migrate: the stubs are gone, and the reserved-word list
  is down to fifteen words plus seven contextual ones. `let try = 3`,
  `row.catch`, and `fn await_all(ts)` all parse.

### Fixed

- **The book is checked against the implementation, not just proofread
  (Campaign 4, R1 — first sitting).** Running a chapter's code blocks
  proves the *examples* work; it says nothing about the much larger
  surface the prose names in passing, which is where a rename quietly
  leaves a lie behind.

  Two guards now cover it, and both found something:

  - **Every function the book names must resolve.** 322 references
    extracted from the prose and checked. One was wrong: `ovm.md` cited
    `fs.read`, which has been `fs.read_file` for a long time. The
    extractor deliberately skips the shapes that only look like calls
    — file names (`ods.md`, `olang-dom.js`) and glob prose
    (`str.parse_*`) — because a guard with false positives is a guard
    that gets switched off, and a self-test pins that behaviour.
  - **`packages.md` and `tooling.md` now have their examples run.** They
    had runnable code that nothing executed — the packaging and tooling
    instructions a new user follows first.

  One reference is exempted by name with its reason: `ods.fma` appears in
  the data-stack design record as a future option, pre-approved if a
  stated gate trips. Naming unbuilt options concretely is what makes a
  design record useful; listing the exemption is what stops an
  accidental reference to a removed function hiding among them.

- **Fixed: a struct argument cost 80x a list's on the compiled tier.**
  Passing a struct holding 5,000 elements to a hot function took 129ms
  against the same data in a list at 1ms. With `--no-ovm` the two were
  identical (4ms and 5ms), which located it: the cost was the tier
  boundary, not the value.

  `BytecodeTier::convert_arg` caches converted arguments by allocation
  identity so an unchanged value crosses once rather than once per call —
  but the cache covered `Value::List` alone. Everything else fell through
  to a full `Value -> OvmValue` walk on every call, so a struct
  re-converted its entire payload 3,000 times. The cache now covers every
  Arc-backed compound value (list, tuple, map, struct) through one
  `CachedOwner` type that carries the variant as well as the weak handle,
  so a struct entry can never answer for a map that happens to sit at the
  same address.

  Measured after: **129ms -> 1ms**, identical to the list. A struct-node
  binary tree of 65,536 nodes now walks 30 times in 116ms.

  A review had reported this as "the language punishes its own type
  system" — an AVL tree written the readable way, with `type Node =
  struct {...}`, was O(n) per descent, so every structure had to be
  rewritten as an untyped list. That workaround is no longer needed. The
  `Arc` on struct fields shipped earlier this week, which did *not* fix
  the symptom on its own, is what made the fix possible: it gave structs
  the stable allocation identity the cache keys on.

- **Fixed: two closures from one factory shared their captures on the
  compiled tier (silent wrong answers).** The bug a reviewer called a
  release blocker, found by the harness below and fixed here.

  ```olang
  fn apply(f, x) = f(x)
  fn adder(k) = (n) => n + k
  println(show(apply(adder(1), 0)) + " " + show(apply(adder(100), 0)))
  ```

  Interpreter: `1 100`. Compiled tier: **`1 1`**.

  The compiled tier caches functions it compiles on demand for the
  higher-order path, and the key was the body's allocation identity
  alone. But `compile_function_with_closure` bakes the captured
  environment *into* the compiled body, so two closures from one factory
  — same lambda body, different captures — are different compiled
  functions that the cache treated as one. The second ran the first's
  captures. The key is now (body, environment), with the same Weak-upgrade
  validation the body already had so a freed-and-reused address cannot
  produce a stale hit.

  It took a shared call site to surface: calling a closure directly takes
  a path that carries captures explicitly. That is why it hid from eight
  hand-written reductions and only appeared in a parser-combinator
  program, where every parser is invoked through a combinator.

- **Whole-program tier agreement is now tested, and it found a real
  divergence on its first run.** `docs/ovm.md` promises that a tier which
  cannot reproduce the interpreter's result refuses to run rather than
  diverging — the basis for trusting a runtime that swaps engines under a
  program. Until now that was checked only by hand-written snippets
  calling one function with one set of arguments.

  `tests/tier_agreement_test.rs` runs the real binary over the real
  corpus — every runnable book example and the standalone example
  programs — under `--no-ovm` and `--ovm-tier=1`, and diffs stdout,
  stderr and exit status together. Elapsed times and throughputs are
  masked, since running faster is the point; everything else is compared
  verbatim.

  It found `examples/language/parser` diverging. Two closures built from the same
  higher-order combinator give different answers on the compiled tier,
  the second behaving as though it captured the first one's argument:
  `word "olang"` yields `olang` interpreted and `""` compiled, and
  `1 + 2 * 3` evaluates to `7` interpreted and a parse error compiled.
  This is the failure a reviewer reported after ~8,000 lines and which
  eight hand-written attempts failed to reproduce. It is fixed above, and
  the harness that found it runs unignored.

- **A program can read its own capability grant (Campaign 3, C3).**
  A denial still stops the program — that is deliberate and unchanged.
  What was missing is the other branch: a program that can degrade could
  not *ask*, only attempt the call and be killed by it, so degradation
  had to be written as recovery from an error, which the language does
  not offer.

  | | |
  |---|---|
  | `caps.allowed(name)` | does the calling code hold it → `Bool` |
  | `caps.level(name)` | `"none"` / `"read"` / `"full"` |
  | `caps.granted()` | the whole grant as a map |

  ```olang
  let plan = if caps.allowed("fs") => "cache to disk" else => "in memory"
  ```

  The answer is the **caller's** grant: code in an attenuated dependency
  sees what that dependency was given, which is the same set the gate
  would enforce a moment later — one classification, asked two ways.
  None of the three is itself gated, so the escape hatch stays available
  under the tightest restriction.

- **`os.exit` now requires `proc` (Campaign 3, C2).** It was the hole in
  the gate: a dependency denied `proc` could not spawn a process but
  could still terminate the host, which is a larger power than the one it
  was refused. `proc` now means "may affect processes", including this
  one.

- **A bundle's transparency record is validated before it is trusted
  (Campaign 3, C2).** `read_bundle` checked the *frame* — lengths,
  overflow, bounds — but nothing checked the *contents*. The `format`
  field selects which bytes the digest covers and the test was
  `format >= 3`, so a bundle claiming format 99 took the format-3 path,
  recomputed a digest that does not cover the meta record, matched it,
  and printed `[verified]`. An unknown format is now refused, and
  digests must be well-formed hex. The only verdict worse than
  "unverifiable" is a confident wrong one.

- **A struct shares its fields instead of copying them.**
  `Value::Struct` held a bare `HashMap` while `Value::Map` directly beside
  it held an `Arc`, so every clone of a struct rebuilt the whole field map
  — and a value is cloned on every bind, every argument pass, every
  return. Structs now hold `Arc<HashMap<..>>` like maps and lists do.

  **This is not the whole of the reported problem.** A review measured a
  struct argument costing 129ms against a list's 1ms and attributed it to
  this representation; the representation was indeed wrong, but fixing it
  did not move that number. Bisecting further: with `--no-ovm` the two are
  *identical* — 4ms and 5ms — and the gap only appears with the bytecode
  tier enabled. The penalty is therefore in the tier boundary rather than
  in the value, and is tracked separately.

- **Assertions work inside a nested block.** They were parsed only as a
  direct child of a test block, so `assert_eq` inside an `if`, a `for`,
  or a `while` fell through to an ordinary call and failed with
  "Undefined variable: assert_eq" — surprising, since a loop over cases
  is exactly where an assertion belongs. Assertions are now a statement
  form like any other, so they also work outside a test block, where a
  failure raises.

- **`()` has a literal, and a pattern.** Unit is what the language hands
  back from an empty branch and what an `X | ()` field holds, and it had
  no spelling: `let u = ()` was a parse error, and so was
  `match x { () => ... }`. Both work now, on both tiers. Where `()` is
  followed by `=>` the nullary-lambda reading still wins — except on the
  right of a comparison, so `if t == () => 0 else => 1` reads as "t is
  Unit, then this branch", which is the whole reason to write it.

- **`for _ in xs` discards its binding.** `_` is not an identifier —
  identifiers must start with a letter — so a loop that ignored its item
  needed an invented name. `par for _ in` too.

- **A tuple iterates in `for`.** Refusing it was a surprise with nothing
  behind it.

- **The tiers agreed to disagree about the empty tuple.** Adding the
  `()` literal surfaced it immediately: the interpreter read a zero-element
  tuple as Unit and the bytecode compiler built an actual empty tuple, so
  a hot function comparing against `()` got a different answer than a cold
  one. Both now read it as Unit, and the new forms above are covered by
  tier-agreement tests that run the same source through both.

- **A long float no longer pushes columns off a printed table.** A
  computed column carries full `f64` precision — a standard deviation of
  `173.98078198467783` — which is wide enough to cost a table two other
  columns. Floats whose full form exceeds twelve characters are shown to
  six significant digits, and the footer says so, like every other cap
  the renderer applies. Values that already fit are untouched, so an
  ordinary table carries no footnote.

- **`ods.read_csv` given a path now says so.** `read_csv` takes CSV
  *text*, so `ods.read_csv("data/sales.csv")` parsed the path itself: a
  Frame with one column named `data/sales.csv`, zero rows, and no error
  anywhere — the failure surfaced later, as a confusing result from
  something else. A single-line argument ending in `.csv`, `.tsv`, or
  `.txt` is now refused, naming `read_csv_file` and `open_csv` instead.
  The check reads the string rather than asking the filesystem, because
  `read_csv` holds no `fs` grant, and a function that probes the disk
  without one is the leak that keeping these two functions separate
  exists to prevent.

- **A `par for` body could not write to an enclosing binding, but nothing
  said so (breaking).** 0.62 made an assignment across a capture boundary
  an error, and the rule reached functions and closures but not `par for`
  — whose body also runs against a worker's snapshot. The write was dead,
  and *conditionally* dead: workers are clamped to the item count, so a
  one-element list took the sequential path where the write really landed.
  The same loop gave two different answers depending on the length of the
  list it was given.

  ```olang
  let mut a = 0
  par for x in [1] { a = a + 1 }        // a == 1
  let mut b = 0
  par for x in [1, 2] { b = b + 1 }     // b == 0
  ```

  `par for` now opens the same boundary as a function body, and the write
  is refused before the program runs. The message differs from the closure
  one because the remedy does: a cell is confined to its creating thread,
  so one made outside the loop cannot help inside it.

  ```text
  cannot assign to 'tally': `par for` runs its body on worker threads, each
  against its own snapshot of the environment, so the write would be discarded
  rather than reaching the outer 'tally'. Produce a value per item and combine
  them — `sum(par_map(xs, (x) => ...))` — or send results over a `chan`
  ```

  Reading an enclosing binding is unaffected — that is how the body gets
  its inputs — and a `let mut` declared inside the body is local to one
  iteration. `examples/concurrency/parmap/` demonstrated the old dead write; it now
  collects its results over a channel and cross-checks them against the
  sequential total.

## [0.65.0] - 2026-08-16

The error-model boundary (D8), and the last lane of Campaign 1's language
work. The lane started by checking D8's own premise and found it wrong
twice over.

### Changed

- **`try`/`catch` is removed (breaking).** D8 kept it for "recovering
  from runtime errors at coarse boundaries". It never did that: it
  destructured a `Result`, and a genuine runtime error inside a `try`
  block still aborted the program. What it actually was is a third
  spelling of something `match` and `unwrap_or` already say — and across
  97 corpus files, including a shell, a load tester, and two fullstack
  apps, it was used **zero times**.

  ```olang
  match f(x) { Ok(v) => v, Err(e) => fallback }   // the general form
  unwrap_or(f(x), fallback)                       // when the fallback is a value
  ```

  The grammar still recognizes the old form so the error can name those
  two rather than failing with "expected a statement". `try` and `catch`
  stay reserved for this release and become ordinary identifiers at 1.0.

- **Recovery is documented as structural, because that is what it is.**
  The boundaries D8 named already recover, without any construct at the
  failure site:

  | Boundary | A runtime error inside it becomes |
  |---|---|
  | a spawned task | `Err(e)` from `task.join` |
  | an `http.serve` handler | a logged 500; the server keeps serving |

  Anywhere else a runtime error stops the program, which is what you want
  from a bug — and what 0.61–0.64 spent four releases establishing.

### Added

- **A discarded `Result` draws an advisory warning.** This was the actual
  hole in the error model. A fallible call in statement position dropped
  its failure silently, and `olang check` reported the file clean:

  ```olang
  fs.write_file("/nope/x.txt", "data")   // failed
  println("wrote it")                     // printed anyway
  ```

  ```text
  warning: the Result from fs.write_file is discarded, so a failure here is
  invisible. Bind it, match it, unwrap it to fail loudly, or write
  `let _ = ...` to say the failure is deliberately ignored.
  ```

  It is advisory, not a gate: unlike the scope and mutability rules this
  is a judgement about intent rather than a provable contradiction, and a
  script that genuinely does not care whether a log write landed is not
  wrong. A block's final statement is its value, so a function whose body
  *is* the fallible call is not flagged.

  The lint reads the help registry for which functions return `Result`,
  so the diagnostic and `:help` cannot disagree about what can fail.

### Fixed

- **`http.serve`'s bind failure was silently discarded in both flagship
  apps.** Starting `examples/web/app` or `examples/web/ledger` on a taken port
  printed the startup banner and exited with status 0, as though it had
  served. Found by the new warning on its first run over the corpus; both
  now report the failure and exit non-zero.

- **25 stale help entries from 0.64.** The conventions audit changed 28
  functions to return values rather than `Result` but left their
  documented return types behind, so `:help os.args` still said `Result`.
  Also surfaced by the new lint, which flagged `os.args()` as a discarded
  `Result` — using the help registry as the source of truth is what
  forced the two back into agreement.

### Migration

`try { A } catch (e) { B }` becomes `match A { Ok(v) => v, Err(e) => B }`,
or `unwrap_or(A, B)` when `B` is just a value. The corpus needed no
changes — it never used the construct — so the migration here is entirely
for code outside this repository.

## [0.64.0] - 2026-08-16

The standard-library conventions audit (D7). Every module was walked once
against one rule, because there is exactly one chance to do this: after
1.0 the API is frozen.

### Changed

- **One rule now decides what a stdlib function returns (breaking).**

  | The operation | Returns |
  |---|---|
  | cannot fail | its value |
  | can fail for reasons the caller could handle | `Result` |
  | was *called wrongly* | raises |

  The third tier is new, and it is what makes the second trustworthy.
  Previously a wrong argument count came back as `Err`, indistinguishable
  from a missing file — so `unwrap_or(fs.read_file(p), "")` swallowed a
  typo'd call exactly the way it swallows a real failure, and the default
  hid the bug indefinitely. Misuse now stops the program:

  ```text
  os.arch(1, 2)
  // error: os.arch expects 0 arguments, got 2
  ```

- **28 functions stopped returning `Result`.** The `os` module carried
  most of the noise — `unwrap(os.args())` appeared in nearly every
  script:

  | Module | Now returns a value |
  |---|---|
  | `os` | `args` `arch` `os_type` `family` `pid` `path_separator` `temp_dir` `username` `is_tty` `flush` `has_env` `list_env` `set_env` `remove_env` `interrupted` `reset_interrupt` |
  | `fs` | `exists` `is_dir` `is_file` |
  | `crypto` | `hex_encode` `export_public_key` |
  | `csv` | `add_row` `read_column` `row_count` `set_headers` `sort_by_column` |
  | `base64` | `validate` |
  | `http` | `decode_query` |

  What keeps `Result` is what genuinely fails: `os.get_env` (the variable
  may be absent), `os.cwd`, `os.chdir`, `os.home_dir`, `os.hostname`,
  `os.exe_path`, `os.exec`, `os.read_line`, `os.stdin`, `os.stdin_lines`,
  `os.on_interrupt`, and every file, network, database, and parsing
  operation.

- **Error messages name the module-qualified function.** `arch expects 0
  arguments` left the reader guessing which `arch`; it is now `os.arch`.

- **Seven declaration keywords are freed as identifiers (`share`,
  `error`, `test`, `type`, `trait`, `impl`, `use`).** Each only ever
  introduces a declaration, and the token after it disambiguates, so
  `let type = row.kind` is an ordinary binding while `type Point = ...`
  still declares a type — including both in one file. They are common
  enough in data and statistical code (a *share* of a total, an *error*
  term, a *test* case) that reserving them cost more than it bought.
  They also work as field names now: `row.type` used to be a parse error.

- **`fs.join` takes its parts variadically**, and still accepts a single
  list. `fs.join("data", "raw", name)` for the literal case,
  `fs.join(segments)` when they are computed — forcing a spread there
  would have been a downgrade. A non-string part raises.

### Migration

`unwrap`/`unwrap_or` raise on a non-`Result`, so every affected call site
fails loudly rather than silently. The fix is to delete the wrapper:

```olang
let args = os.args()                  // was: unwrap(os.args())
if fs.exists(path) => ...             // was: unwrap_or(fs.exists(path), false)
let tty = os.is_tty()                 // was: unwrap_or(os.is_tty(), false)
```

A `match` on one of these needs the same treatment — the `Ok`/`Err` arms
no longer fit and the match will fail at runtime:

```olang
let user = os.username()              // was: match os.username() { Ok(u) => u, Err(e) => "?" }
```

The repository corpus (46 call sites across 25 files, plus the embedded
`cli` and `term` packages) is migrated in this release.

## [0.63.0] - 2026-08-16

Removes `async`, `await`, and the `Promise` API. olang now has **one**
concurrency model — threads — and this is the last of Campaign 1's
breaking changes to the language surface.

### Changed

- **`async`, `await`, and `Promise` are gone (breaking).** They described
  a deadline-based scheduler that resembled asynchronous I/O without
  being it: `Promise.delay` simulated latency, `await` slept, and none of
  it involved concurrency at all. Meanwhile `spawn` was already a real OS
  thread and `await` on a spawned task was really a *join*. Two
  vocabularies for one mechanism, one of which was theatre.

  What remains is what was always doing the work:

  | Was | Now |
  |---|---|
  | `await t` | `task.join(t)` |
  | `Promise.all(ts)` | `ts \|> map(task.join)` |
  | `Promise.race([work, timeout])` | `task.join_timeout(t, ms)` |
  | `Promise.delay(v, ms)` | `time.sleep(ms)` |
  | `Promise.resolve(v)` | `v` |
  | `Promise.reject(e)` | `Err(e)` |
  | `async fn f() = ...` | `fn f() = ...`, called through `spawn f()` |

  There is no function colouring left: any function can be spawned,
  because a task is a thread running an ordinary call.

- **`spawn` returns a task handle** instead of a promise. `task.join(t)`
  blocks and returns the value, or `Err(e)` if the task failed —
  unchanged from what `await` did, so worker failure stays a value
  rather than a crash. Joining the same handle twice returns the
  memoized result; a handle dropped unjoined is fire-and-forget.

- **`Promise<T>` annotations are removed.** `spawn` produces a `Task`.

- The removed forms still *parse*, solely so the error can name the
  replacement. A keyword that merely falls out of a grammar produces
  "expected a statement", which tells a reader nothing:

  ```text
  `await` was removed in 0.63. olang's concurrency model is threads,
  channels, and data parallelism: `spawn f(x)` starts a task and
  `task.join(t)` collects its result (or `Err(e)` if it failed); join
  several with `tasks |> map(task.join)`; use `time.sleep(ms)` for delays
  and `chan` to stream results.
  ```

  `async` and `await` stay reserved for this release and become ordinary
  identifiers at 1.0.

### Added

- **`task` module.** `task.join(t)` collects a spawned task's result.
  `task.join_timeout(t, ms)` returns `Ok(v)` if it finished in time and
  `Err("timed out")` otherwise.

  **A timeout bounds the wait, not the work.** An OS thread cannot be
  cancelled from outside without leaving whatever it touched in an
  unknown state, so olang does not offer a cancel that would be a lie: a
  timed-out task runs to completion and its result stays collectible
  from the same handle. `join_timeout` wraps success in `Ok` precisely
  so "the task produced `Err`" stays distinguishable from "we stopped
  waiting". This is the honest replacement for `Promise.race`, which
  implied the loser stopped.

- No `task.join_all`. Joining a list of tasks is `map(task.join)` —
  they are all already running, so the fan-in needs no API of its own.

### Fixed

- **`:pkg load` no longer promises an import that cannot work.** Loading
  an *application* package (a `main.ol` plus a `lib/`, with no public
  root module) printed "use it with `use <name>`", and that import then
  failed with the generic "Module 'x' not found" plus a "system-level
  error" hint. Two bugs: the REPL advertised a root module without
  checking for one, and the resolver swallowed its own precise
  diagnosis and fell through to the global file search, so the reader
  was sent looking for a missing file instead of being told the package
  has no importable root. `:pkg load` now lists the modules the package
  actually offers, and `use <pkg>` names both what it looked for and
  what is there:

  ```text
  package 'loadtest' has no root module: `use loadtest` needs one of
  index.ol, mod.ol, loadtest.ol, or src/index.ol at examples/web/loadtest/.
  Importable modules there: loadtest.lib.server, loadtest.lib.stats
  ```

### Migration

`olang check .` reports every site. The corpus migration in this release
(scheduler, loadtest, pargrep, demo, and the book's concurrency chapter)
is the worked example. `examples/concurrency/scheduler/` is the clearest before and
after: it was built entirely on `Promise.delay` + `Promise.race` and is
now `spawn` + `task.join` + `task.join_timeout`, with the "the task is
still running" caveat stated where the timeout is taken.

## [0.62.0] - 2026-08-16

Completes the mutability model 0.61.0 began. Plain `let` is an immutable
binding, `let mut` is a reassignable one, and this release adds the third
thing: a mutable *location*.

### Added

- **`cell` — the one mutable location.** `cell(v)` makes one;
  `cell.get(c)` reads it; `cell.set(c, v)` replaces the contents;
  `cell.update(c, f)` applies `f` to the current value, stores the
  result, and returns it. `cell.new(v)` is the same function as
  `cell(v)` — the module is callable, so the constructor reads as a
  noun while the operations stay namespaced.

  Values are immutable and closures capture by value, which is what
  makes `spawn` and `par_map` safe without locks. The case that model
  handles badly is state updated from deep inside a call chain or from
  a callback whose signature is fixed; threading an accumulator through
  functions that have no other interest in it is the honest workaround
  and often a poor one. A cell is the escape hatch, deliberately
  narrow. A cell is a location, not a value: two cells with equal
  contents are not equal, and binding one to a second name aliases it.

- **Cells are confined to the thread that created them.** Reading or
  writing a cell from another thread is an error naming both threads,
  and `chan.send` refuses to send one — including one nested inside a
  list, map, struct field, or `Result`. The no-shared-mutable-state
  guarantee that makes olang's parallelism lock-free is preserved
  exactly: two threads still cannot reach one mutable location. A cell
  created inside a task and used only there is unremarkable; the rule
  concerns crossing, not tasks.

  Confinement is checked on *access* rather than at the thread
  boundary, because `spawn` and `par_map` snapshot the whole
  environment rather than an enumerated capture list — there is no list
  of what crossed to inspect, and a crossing-time check would have had
  to refuse any `spawn` with a cell merely in scope. Checking on use is
  both sound and precise. `chan.send` is the one crossing that holds
  the value being sent, so it is checked there, at the mistake.

- **`cell.update` refuses re-entrant access.** Touching the same cell
  from inside its own update function is an error rather than a write
  that the function's return value silently overwrites. The lock is
  never held across the callback, so this is a diagnosable error, not a
  deadlock, and the flag is cleared whether the callback returns or
  raises.

- Cells need no timeline recording: mutation is deterministic within a
  thread and unreachable across threads, so a replayed run performs the
  same mutations in the same order.

### Changed

- **Assigning to a captured binding is an error (breaking).** Inside a
  closure or nested function, assigning to a name bound in an enclosing
  scope was a dead write — capture is by value, so it reached the
  snapshot and never the original — and had been an advisory warning
  since 0.51. It is now refused before the program runs, and the
  message points at `cell`:

  ```text
  cannot assign to 'total': it is captured from an enclosing scope, and
  functions capture by value — the outer 'total' would not change. Return
  the new value, or hold the state in a cell (`let total = cell(...)`,
  then `cell.set(total, ...)`)
  ```

  The warning only became fair once there was an alternative to name,
  which is why it lands with `cell` rather than before it. The check
  moved into the same pre-execution validator as the 0.61.0 rules, so
  the runtime, `olang check`, and the editor now agree on it — before,
  only `olang check` reported it and the program still ran. No file in
  the repository corpus was affected.

- Calling a module value invokes its `new`, so `cell(0)` and
  `cell.new(0)` are one function. This is a general rule, not a special
  case: a module whose purpose is constructing one kind of value may be
  called directly.

### Fixed

- The language reference's keyword table wrote `catch e { … }`; the
  syntax is `catch (e) { … }`, as every runnable example in the book
  already had it.

## [0.61.0] - 2026-08-16

This release settles olang's scope and mutability rules. Three behaviors
that had been advisory warnings since 0.50 are now enforced errors. This
is the **second and final deliberate breaking change before 1.0** (the
first was runtime type enforcement in 0.48.0); see the migration guide
below.

### Changed

- **Every block scopes its bindings (breaking).** A `let` inside
  `{ ... }` — a bare block, a loop body, an `if` branch, a `match` arm —
  ends with that block. Previously a bare block's bindings leaked into
  the enclosing scope, which meant a name's lifetime depended on which
  construct happened to surround it. Function bodies, `for` variables,
  and `match` arm bindings already scoped; blocks now agree with them,
  and the rule is one sentence instead of a table of exceptions.

  This also closed a real divergence between execution tiers. The
  bytecode compiler kept a flat map of local variables with no scope
  stack, so `let x = 1; { let x = 2 }; x` evaluated to `1` on the
  interpreter and `2` on the bytecode tier. Both tiers now push and pop
  a scope per binding block, and two differential tests pin the
  agreement.

- **Assignment is not a declaration (breaking).** `x = 1` where `x` was
  never bound is an error rather than an implicit `let`. This is what
  turns a mistyped name from a silently-created variable into a
  refusal.

- **`let mut` is enforced (breaking).** Assigning to a binding not
  declared `mut` is an error. `mut` was parsed and then discarded — it
  documented intent to the reader and promised nothing to the compiler.
  It is now a guarantee, and a plain `let` genuinely means immutable.

  Shadowing is unchanged and remains the recommended alternative: a
  fresh `let` of the same name always works and produces a new binding
  rather than a mutable one.

- All three are checked by a single validation pass that runs **before**
  the program executes, so a violation on a rarely-taken branch is
  caught anyway and cannot differ between the interpreter, bytecode, and
  JIT tiers. `olang check` and the language server report them in the
  same words as `olang run`, with the same line and column.

- `olang check` distinguishes these in its output: scope and mutability
  violations are labeled "the program is refused before it runs" rather
  than sharing the type checker's label.

- `meta.parse` now reports a `let` declaration's mutability, so
  project lints written against the AST can see it.

### Migration

Run `olang check .` — it lists every site with the exact fix in the
message. The three rewrites:

```olang
// A binding you reassign now needs `mut`
let mut total = 0
for n in xs { total = total + n }

// An assignment with no declaration becomes a declaration
let count = 0            // was: count = 0

// A block's binding, used after the block, moves out of the block
let mut resolved = ""    // was: declared inside the `if`
if ready => { resolved = compute() }
```

For a value you compute in stages, prefer shadowing over `mut`:

```olang
let raw = read_input()
let raw = str.trim(raw)
let raw = str.lower(raw)
```

The repository's own corpus — 97 example files, the embedded stdlib
modules, and every runnable example in the book — is migrated in this
release and checks clean.

## [0.60.0] - 2026-08-15

### Changed

- **CLI facelift — every command is now discoverable.** `olang` moved from
  a hand-rolled first-argument dispatch to real clap subcommands, so `olang
  --help` lists every command (`run`, `check`, `fmt`, `test`, `build`,
  `inspect`, `caps`, `replay`, `doc`, `bench`, `lsp`, `repl`) and `olang
  <command> --help` documents any one of them with its own flags — none of
  which the old flat help surfaced. The file-first form is unchanged: `olang
  script.ol [args]` still runs a file directly (a word that is neither a
  command nor a flag is taken as a file path), bare `olang` still starts the
  REPL, and every prior invocation — run options, `--watch`, `--deny`,
  script arguments, built binaries — behaves exactly as before. Run options
  are grouped under their own heading, and the help ends with a worked
  example block.

### Added

- **`olang record` — recording is now a first-class command.** Recording a
  run was only reachable through the `--record TRACE.olt` run option, so the
  timeline story was lopsided: `olang replay` was a visible command with no
  visible way to produce what it replays. `olang record <file> [-o
  trace.olt] [args]` runs a program and writes its trace (defaulting to the
  program's stem + `.olt`), the mirror of `olang replay`. The `--record` run
  option still works and is unchanged.

- **`olang inspect` reports the build platform.** A built binary now records
  the OS and CPU architecture it was built on (`built on: macos/aarch64`),
  and `inspect` flags whether that matches the current machine — a native
  `olang build` binary only runs on its own platform, so this tells you at a
  glance whether a binary that arrived from elsewhere will run here. The
  field is informational provenance (like the olang version): it is not part
  of the integrity digest, and binaries built before it existed inspect
  cleanly with the line omitted. The default `inspect` summary also now
  points at `--source` (the way it already pointed at `--manifest` and
  `--lockfile`), so printing a binary's embedded source is discoverable from
  the summary itself.

- **Parallel hash join (data-pipeline campaign DP1).** `ods.join` now runs
  its probe phase across every CPU core when the left frame is large
  (50,000+ rows): each left row is looked up in the built key table
  independently, and the per-thread chunks are concatenated in row order,
  so the parallel result is byte-identical to the sequential one (pinned by
  a test that passes in both feature modes). Measured ~2.6× on a 4M-row
  join. Small joins stay single-threaded. This is where the lack of a GIL
  shows — a large join uses the whole machine with no ceremony. (Elementwise
  Series operations were already parallel.)

- **Parallel `group_by` aggregation (data-pipeline campaign DP1b).** The
  aggregation pass in `ods.group_by` now scatters across every CPU core when
  the frame is large (100,000+ rows with under ~4M groups): each thread
  accumulates `sum`/`count`/`min`/`max` into per-group partials over a fixed
  row range, and the partials are merged sequentially in chunk order. Integer
  results are bit-identical to sequential; float `sum`/`mean` reorder their
  additions but the order is fixed by the chunk layout, so a given input
  always reproduces the same result run to run. The group-id hashing pass
  stays sequential. Measured ~1.4× on 8M rows over 1,000 groups (aggregation
  is a fraction of the total; the hashing pass dominates and is DP1c's
  target). Pinned by a test that passes in both feature modes.

## [0.59.0] - 2026-08-15

### Added

- **`--trace-caps --write` and `olang caps` — frictionless capability
  manifests (openness maturity lane OM3).** `olang run --trace-caps --write`
  folds the suggested least-privilege `[capabilities]` block directly into
  the package's `olang.toml`; it never overwrites an existing block, and
  falls back to printing when the program is not in a package. `olang caps
  [path]` prints the grant a package *declares* (base plus each dependency's
  attenuation) — the static counterpart to the observed `--trace-caps`
  profile — and defers to `inspect --caps` for a built binary.

- **`examples/language/capabilities` — a runnable malicious-dependency demo.** The
  same app runs twice against a third-party package with a backdoor: the
  unguarded variant lets it read a local secret; the guarded variant grants
  the dependency `fs = false`, so the identical read is refused at the gate
  while the app's own read still works. Pinned by a Rust test.

- **`olang check --rules <rules.ol>`: project lint rules in olang (openness
  lane O6).** A project defines lint functions named `rule_*` that take a
  file's AST, flattened to a list of `kind`-tagged node maps each carrying
  its nearest source `line`, and return findings (a message string or a
  `#{ "message", "line" }` map). The checker runs these rules alongside the
  built-in type checks and reports each finding with the file name, line,
  and rule name. Findings count as problems, so a violation produces a
  non-zero exit. A rules file that defines no `rule_*` functions is an
  error, and a rule that raises an error is reported by name.

- **`olang inspect <binary> --against <dir>`: provenance check.** Compares
  the binary's embedded source, manifest, and lockfile to a checkout on
  disk and reports each file as match, differ, or missing, exiting non-zero
  on any mismatch. Where `--verify` checks internal consistency,
  `--against` confirms that a binary was built from a specific source tree.

- **`olang run --trace-caps`: capability profiler.** Reports the
  capabilities a run used (`fs` as read or write, plus `net`, `db`, `proc`,
  and `env`) and prints a least-privilege `[capabilities]` manifest with
  every unused capability set to its most restrictive value. Runs on the
  interpreter tier so every effect is observed, and reports even if the
  program crashes.

### Security

- **Openness soundness cleanup (pass S6–S10).** Several smaller hardening
  fixes: (S6) machine-identity `os.*` — `arch`, `os_type`, `family`,
  `path_separator`, `args`, `cwd`, `exe_path`, `pid`, `is_tty` — and
  `crypto.random_bytes` are now recorded by the timeline, so a trace that
  branches on the machine or process context replays portably (dead
  `time.now`/`utc_now`/`today` entries removed). (S7) `olang check --rules`
  runs the rules file in a no-capability sandbox, so a hostile `rules.ol`
  cannot touch the filesystem, network, or processes when loaded. (S8)
  `olang inspect`'s bundle-footer parser uses checked arithmetic, so a
  crafted binary is rejected rather than triggering a huge allocation.
  (S10) the timeline's record step now actually enforces its documented
  round-trip guard — a result that does not serialize (a non-finite float)
  is not recorded, so replay diverges cleanly instead of serving a
  corrupted value. (`db`/`proc` recording and a symlinked-dependency
  attribution edge remain tracked.)

- **Record/replay no longer silently misapplies recorded values (openness
  soundness pass S4).** Two fixes make single-threaded replay sound. (1) Map
  iteration is now deterministic: `map_keys`/`map_values` iterate in
  key-sorted order (matching `entries`), so a `HashMap`'s
  process-randomized order can no longer make a record run and a replay run
  visit a map in different orders — which previously served one call's
  recorded result to a different call with no divergence raised. (2) The
  trace now records a fingerprint of each recorded call's arguments (trace
  format 2), and replay raises a divergence when a recorded op is invoked
  with different arguments than recorded. v1 traces replay without the
  argument check. Map iteration determinism also benefits any program that
  hashes or serializes iterated output.

- **`meta.parse` now emits every AST child, so project lints no longer
  silently miss code (openness soundness pass S5).** The conversion dropped
  children exactly where calls hide — `match` arms, `await`/`assert*`,
  and `map`/struct/object/template literals collapsed to a summary node — so
  a `check --rules` lint like "no bare `unwrap()`" returned clean on code
  that had one inside a `match` arm. Every variant now emits its children
  (`arms`, `entries`, `fields`, template `parts`, and the async/assertion
  interiors), and the expression conversion is **exhaustive** (no
  catch-all), so the compiler guarantees no variant is silently dropped and
  a new one is a build error until it is handled.

- **`db` and `net` are no longer latent filesystem capabilities (openness
  soundness pass S2/S3).** A filesystem sub-gate now confines file access
  that happens *through* other modules: `db.open` on a file path requires
  `fs` (an in-memory `:memory:` database needs none), a `db` query running
  `ATTACH` requires `fs`, and an `http.serve` handler returning `body_file`
  is refused (403) unless the program's grant permits `fs` read. Previously
  a program with `fs = false` could still create/write arbitrary files via
  `db.open`/`ATTACH` or read any file via `body_file`. Now `fs = false`
  actually confines the filesystem even when `db` or `net` is granted.

- **The transparency checksum now binds the executed AST, closing a
  critical integrity hole (openness soundness pass S1).** A built binary
  runs its embedded **AST**, not its source (the source is only shown for
  error snippets). The checksum previously covered source + manifest +
  lockfile but **not** the AST, so an attacker could replace the AST with a
  malicious program while leaving the source byte-identical: `inspect
  --verify` passed, `--source` printed the clean source, and the binary
  executed the swapped code. The digest is now `source ‖ AST ‖ manifest ‖
  lockfile` (bundle format 3), and both `--verify` and `--against` also
  assert that the embedded source parses to the embedded AST — so `--source`
  is honest and a swapped AST is reported as `DIVERGES`. Found by an
  adversarial audit and confirmed by a reconstructed attack; pinned by a
  regression test. Format-2 bundles (0.59.0-dev) verify with the old digest;
  rebuild to get AST binding.

- **The transparency checksum covers the manifest and lockfile, not only
  the source (openness lane O1a).** `olang build` records the checksum over
  the source, the embedded `olang.toml`, and the `olang.lock`, so a grant
  widened in place fails `--verify`. Bundles built before this carry no
  such checksum and fall back to the source checksum.

### Fixed

- **Two real per-tick memory leaks in the concurrency path, found by
  soak-testing.** A program that opened a channel or `spawn`ed a worker
  pool every tick (Harborline's crew loop is the canonical shape) grew
  memory without bound:
  - *Channels were pinned in a process-wide registry that `close` never
    removed* — every channel ever created leaked its entry. Channels are
    now `Value::Native` handles that own the channel by `Arc`, so the last
    handle drop frees it; there is no registry to leak. (`leaks`: 20 000
    channels went from ~47 MB to a flat ~14 MB.)
  - *The `spawn` registry kept every completed task's memoized result
    forever.* A promise now carries a drop-guard shared by its clones;
    when the last clone drops — the last place that could still `await`
    the task — the registry entry is removed. Double-await (a tested
    guarantee) and fan-out-then-collect both keep working. (`leaks`:
    64 000 spawns went from linear growth to flat.)

  Together these cut Harborline's real per-tick growth by ~65 %, and
  `leaks` now reports zero. Two defensive hardenings landed alongside:
  the bytecode tier's hot-mirror `Vec` (indexed by a global function id)
  is capped so a short-lived VM can't size it to the global high-water,
  and `JitCache` now frees its Cranelift module's executable pages on
  drop (Cranelift does not do this automatically). A residual RSS climb
  under extreme thread churn (tens of thousands of fresh OS threads) is
  reclaimable — `leaks` clean, malloc heap flat — and is a macOS
  thread-VM characteristic best addressed by pooling worker threads,
  tracked with the runtime's memory work.


## [0.58.0] - 2026-08-15

### Added

- **The `meta` module — the Open AST, the "open code" pillar.**
  `meta.parse(source)` parses olang source and returns the program as
  ordinary olang values: a list of `kind`-tagged statement maps you walk
  with the same `map`/`filter`/`fold`/`match` as any data. Because the
  syntax is stable, these node shapes are a stable public format —
  linters, codemods, and import extractors become olang scripts, not
  compiler changes (`otc deps` is four lines over it). Faithful for the
  shapes a tool inspects, summarizing the deep interior; a syntax error is
  an ordinary `Err`, never a crash. `examples/language/metatool` lints bare
  `unwrap()` calls per function. With this the openness campaign's
  three-pillar thesis — open artifacts, open execution, open code — is
  complete.

- **The Open Timeline — record, replay, deterministic re-execution.**
  `olang --record trace.olt program.ol` logs a run's nondeterministic
  inputs — `random.*`, the `time` clocks, the environment/stdin/`exec`
  surface of `os`, filesystem reads, `http`, and seeded-random `crypto` —
  and `olang replay trace.olt` re-runs the program serving each of those
  calls from the log, reproducing the run bit-for-bit: the same random
  rolls, timestamps, and environment, to the last digit. It works because
  olang programs are deterministic given their inputs (immutable values,
  capture-by-value closures, a seeded RNG), so reproducing the inputs
  reproduces the whole run. The `.olt` trace embeds the program source,
  so a trace is a portable, self-contained reproduction — replay works
  from a machine where the program does not exist, and a *crashed* run
  records on the way down, so the failure replays exactly. Divergence is
  detected: if the program's effect sequence no longer matches the trace,
  replay stops at the exact point and says so. Runs on the interpreter
  tier (the one choke point that sees every builtin), single-threaded in
  v1; `replay --why` (value provenance) is a recorded roadmap rung.
  Pinned by an integration suite (reproduction, portability, crash
  capture, divergence).

- **The transparent binary + capability manifests — "Open by
  construction."** Two halves of one identity feature for a language
  named *Open*:
  - `olang inspect <binary>` reads the transparency record out of any
    `olang build` executable: its exact source (`--source`), its
    `olang.toml` and `olang.lock` (`--manifest`/`--lockfile`), a sha256
    of the source verified on demand (`--verify`, nonzero on mismatch),
    the capability grant (`--caps`), or the whole paper trail extracted
    to a directory (`-o dir/`). A built binary already embedded its
    source (for error snippets); this turns that into a guarantee — you
    cannot ship an olang program as a black box. It doubles as a
    built-in SBOM and as the answer to "what version, which patches?".
  - `[capabilities]` in `olang.toml` gates the effectful stdlib surface
    (`fs`, `http` as `net`, `db`, `proc`, the environment functions of
    `os`) at the module boundary; pure computation is never gated.
    Absent = wide open, so it is opt-in and never breaks existing code.
    The novel part is **per-dependency attenuation**: a dependency can
    be granted *less* than the app, never more, enforced by attributing
    each gated call to the package whose code made it — a supply-chain
    compromise that adds `fs`/`net` behaviour to a dependency that was
    never granted it dies at the gate. `--deny` (and `OLANG_DENY`)
    restrict any run from the command line, and a built binary enforces
    the manifest it carries. Enforcement runs on the interpreter tier
    (the call stack is what attributes a call): a capability-restricted
    run steps the bytecode tier aside, like `par for`; unrestricted runs
    keep full speed. Pinned by an integration suite covering manifest
    grants, attenuation, `--deny`, ghost-dependency refusal, and the
    build → inspect → enforce round-trip.
- **`examples/concurrency/demo` — Harborline, the consolidated flagship example.**
  The 23 loose scripts at the top of `examples/` are consolidated into
  one coherent, long-running system: a harbor-operations simulator with
  eleven library modules (domain ADTs and tariff expression trees, a
  BST scheduler with a tide model, a SQLite ledger, threaded unload
  crews over channels, fold-based analytics cross-checked against the
  stats module, template/regex/JSON/CSV text handling, an RSA-signed
  hmac digest chain, and a term dashboard). It runs forever by default
  with graceful Ctrl-C shutdown, is deterministic under `--seed`,
  bounded under `--ticks`, writes no files unless `--out` is given, and
  checks world-vs-ledger invariants every simulated day with the
  testing module — a soak test for the language, not a demonstration.
  95 assertions of module self-tests run via `olang test .`; the
  harness runs it bounded; `tests/example_programs_test.rs` pins a
  deterministic end-to-end run. Documented as a new book chapter,
  [Building Robust Systems](docs/demo.md).
- **Every builtin and stdlib symbol is documented in the REPL's `:help`.**
  Fifteen modules had no coverage at all (`proc`, `chan`, `time`, `toml`,
  `ods`, `stats`, `plot`, `dom`, `cli`, `term`, `ui`, `viz`, `dash`,
  `colx`, `mathx`) and several "covered" modules had gaps (`math`'s
  trig/log family, `fs` path helpers, `csv` builder verbs, `db`
  transactions, `random.gauss`, the `os` signal/tty/stdin functions,
  `str.fmt`). All ~270 registered functions now resolve, verified by
  diffing every registration site against the help index; a unit test
  pins per-module minimum counts so new modules can't silently ship
  undocumented.
- **`:help <module>` lists the module's functions.** A bare module name
  (`:help proc`, `:help stats`) previously fuzzy-matched to one arbitrary
  member; it now shows the full member listing, including nested
  namespaces (`:help stats.norm`). Exact function lookup, category
  lookup, and typo fuzzy-matching are unchanged, in that order.
- **`olang run file.ol` works.** There is no `run` subcommand (the file
  is the first positional), but the muscle memory from `cargo run`/`go
  run` is real: a first argument literally `run` that names no existing
  file now shifts to the next argument, with script argv preserved.

- **The package manager's trust model is now enforced, not aspirational.**
  Three `otc pkg` hardenings:
  - *Checksums are verified, everywhere content arrives.* Every install —
    fresh resolve or lockfile replay — re-checks fetched git/registry
    sources against the recorded sha256 and fails hard on a mismatch;
    a registry release's published checksum is enforced at fetch time.
    Previously checksums were recorded but never re-checked. Path
    dependencies stay exempt on install (editing one is development, not
    tampering). New `otc pkg verify` re-checks everything the lock pins
    on demand.
  - *The registry index is append-only.* `otc pkg publish` used to
    silently replace an already-published version's rev/checksum — the
    exact history rewrite the checksum exists to catch. Republishing an
    existing version is now an error; `--force` remains as a deliberate
    escape hatch.
  - *`otc pkg add` validates before writing.* A `--path` must exist, a
    `--version` must parse (and, with a registry configured, be
    satisfiable), conflicting source flags are an error instead of a
    silent precedence pick, `--tag` without `--git` is rejected, and
    changing an existing dependency's source requires `--force`. Typos
    now surface at add time, not as an opaque failure at the next
    install.
- **`proc` — child processes, streaming I/O, and pipelines.** A new
  native module for driving processes beyond `os.exec`'s run-to-
  completion model. `proc.spawn(program, args)` returns a live `Process`
  handle you feed with `write`/`write_line`/`close_stdin`, read a line at
  a time with `read_line` (`read_all` for the rest), and finish with
  `wait` (→ `#{ code }`) or `kill`. stdout and stderr are drained on
  background threads, so a child that floods one stream never deadlocks a
  caller reading the other. `proc.pipeline(stages)` chains commands the
  way the shell's `a | b | c` does — each stage's stdout wired to the
  next one's stdin — and returns `#{ code, stdout, stderr, codes }` with
  every stage's exit code. Lane T3 of the **Toolsmith campaign**.

- **`os.on_interrupt` — graceful Ctrl-C for long-running tools.**
  `os.on_interrupt()` traps SIGINT so it sets a flag instead of killing
  the process; `os.interrupted()` polls it (`while os.interrupted() ==
  false { ... }`) and `os.reset_interrupt()` clears it, so a server or
  watch loop can drain and exit cleanly.

- **`examples/tools/watch` — the process-story dogfood.** A `watch(1)`-style
  tool that reruns a command on an interval and streams its output, or
  runs a pipeline with `--pipe`, until Ctrl-C — exercising `proc`
  streaming, `proc.pipeline`, and `os` signal handling through the `cli`
  + `term` toolkit.

- **`olang test --coverage` — line coverage from the test runner.**
  `--coverage` reports covered/total executable lines per file plus an
  overall figure; `--coverage-lines` additionally lists each file's
  uncovered ranges. Coverage is a report, never a gate — it leaves the
  exit code untouched. It is attributed to the file the code *lives in*:
  function values now carry a `def_file` that the interpreter pushes and
  pops across calls, so a helper defined in one file and exercised by a
  test in another is credited to the helper's file, not the test's. The
  executable-line denominator is derived from the parsed AST (the same
  statements the runtime records), so a fully-exercised file reads
  exactly 100%. Runs on the interpreter tier so the statement-level hook
  sees every line. Lane T7 of the **Toolsmith campaign** — the last piece
  of a credible test story.

### Fixed

- **Building the demo flushed out four latent bugs, all fixed:**
  - *Bytecode tier: a capturing lambda inherited its enclosing
    function's parameter annotations*, applied positionally to
    [own params..., captures...] — so a lambda capturing an annotated
    `String` parameter failed with "expects Int, got String" once
    compiled, while the interpreter ran fine. Lambdas now carry their
    own checks; pinned by a differential test.
  - *"Aggressive memory management" cleared the module cache mid-load.*
    A >10-element list literal in a module body triggered a cleanup
    that dropped the loader's in-flight placeholder entries — the
    anchors for relative `use` resolution — so the next import in the
    importing file failed with "Cannot find module". The cache is now
    left alone while a module load is in flight.
  - *`olang test` reported ✓ over failing assertions.* A test block was
    only marked failed if it raised; `testing.assert_*` results (which
    tally rather than raise) were never consulted, so a block full of
    failing assertions passed. The runner now charges each block with
    the assertion failures that occurred inside it.
  - *`otc deps` and `otc unused` were blind to every declaration* —
    they matched AST statements without unwrapping the `Located` span
    wrapper introduced by the error-experience overhaul, so `deps`
    always printed "No dependencies found" and `unused` scanned
    nothing. Both now match through the wrapper.
- **`embedded_term_test` no longer fails when run from a real
  terminal.** The test assumed a test binary's stdout is a pipe, but
  `cargo test` leaves the fd a tty when run interactively; the color-off
  leg is now forced explicitly with `NO_COLOR`.

- **The gallery no longer eats the GPU.** The browser host ran one
  unthrottled requestAnimationFrame chain per `dom.on_frame` handler and
  painted every animated canvas on every tick, visible or not — on a
  120Hz retina display the gallery's three live pieces (12,500-star
  galaxy, 50,000-point curtain, rose curves) repainted ~4.25MP of
  devicePixelRatio-2 backing store 120 times a second, two of them
  under semi-transparent full-surface clears (~0.5 Gpix/s of clears
  plus ~7.5M point-quads/s), pinning the GPU near 80%. Three host-side
  fixes in `olang-dom.js`, no olang code changes:
  - All `on_frame` handlers now share one animation loop capped near
    60fps — data animations gain nothing from 120Hz.
  - `dom.draw`/`dom.draw_points` skip painting for a canvas outside
    the viewport (IntersectionObserver, feature-detected): handlers
    keep running, offscreen canvases cost the GPU nothing, and
    painting resumes the frame after the canvas scrolls back in.
  - A canvas can trade retina backing for fill rate with
    `data-olang-dpr` in its markup; the gallery's three animated
    canvases run at 1.5 (44% fewer pixels each), stills keep full
    sharpness. devicePixelRatio is also capped at 2.
  Together: roughly 2x from the frame cap, ~1.8x from the smaller
  backing stores, and near-total savings for whatever isn't on
  screen — with three pieces and at most one in view, an order of
  magnitude less GPU work in normal browsing.
- **List, tuple, and Unit equality now work in promoted functions.** The
  bytecode VM had no `==`/`!=` arms for top-level List, Tuple, or Unit
  operands, so a function like `fn is_empty(xs) = xs == []` returned the
  right answer interpreted but raised "Unsupported operation: Equal" once
  it got hot enough to promote — a hard tier disagreement. All three now
  compare structurally, exactly mirroring the interpreter (including the
  deliberate asymmetry that `1 == 1.0` is true but `[1] == [1.0]` is
  false). Pinned by a differential test.
- **Binary-op type errors read the same on every tier.** A promoted
  function raising e.g. `[1] < [2]` said "Unsupported operation:
  LessThan" while the interpreter said "Invalid binary operation: cannot
  apply '<' to List and List"; `1 && 2` produced a bare "Invalid binary
  operation" with no detail. The VM now emits the interpreter's message,
  naming the operator and both operand types (mixed Int/Float pairs
  report their real types). Pinned by an error-text differential test.
- **REPL: a `//` comment containing a bracket no longer traps the session
  in multiline mode.** `1 + 1 // {` used to buffer forever (swallowing
  even `quit`) until a manual `:end`; bracket counting now stops at a
  comment, while `//` inside a string still counts as content.
- **REPL: `:type` no longer mutates the session.** `:type n = n + 1`
  executed the assignment it was asked about; a binding or assignment is
  now refused with a pointer to query the value instead.
- **REPL: interactive tutorials exit on EOF.** With a closed or piped
  stdin, `:tutorial_run` re-prompted forever at 100% CPU; end-of-input
  now ends the tutorial (and `quit`/`exit` work alongside `q`).
- **REPL: `:search` filters compose and validate.** Combined filters
  (`category:X limit:N`) previously cancelled each other, `limit:05`
  or an invalid `limit:` silently returned zero results, and the quoted
  phrases the usage examples themselves suggest never matched. Filters
  are now parsed token-wise, a bad limit is reported, and quotes are
  stripped.
- **REPL: `:help STATS` no longer advertises calls in casing the language
  rejects** — module listings echo the canonical lowercase name.
- **A missing script file is named in the error.** `olang nope.ol` said
  only "No such file or directory"; it now says which path it couldn't
  read.
- **Help accuracy: eight `:help` entries corrected against the
  implementation** — `csv.sort_by_column`'s boolean is *ascending* (the
  doc said descending), `csv.add_column` takes `(csv, values, name)` not
  `(csv, name, values)`, `csv.filter_rows` takes an integer column index,
  `csv.from_json` returns rows not CSV text (book table fixed to match),
  `str.fmt` and `ods.read_csv` return bare values not `Result`,
  `colx.partition` returns a tuple, and `plot.ramp` also accepts `auto`.

- **`olang check` warns on dead assignment to a captured binding.**
  Assigning to a variable captured from an enclosing scope inside a
  closure or function has no effect — capture is by value, so the write
  hits the snapshot and the outer variable never changes. The checker now
  flags it. It is provable (no false positives): the target is bound
  strictly outside the current function boundary; params, locals, and
  top-level reassignment are untouched. This is the language's sharpest
  footgun, and the lint makes it loud instead of silent. Fixing it also
  surfaced a real latent bug — `examples/03_algorithms.ol`'s
  closure-over-a-map memoization never actually memoized; it is rewritten
  to thread the cache, the correct olang idiom.
- **Empty-collection truthiness now agrees across tiers.** A promoted
  (bytecode-tier) function used an incomplete condition test that treated
  an empty string, list, tuple, or range as *truthy* — so `if xs => …` or
  `while xs { … }` on a promoted function took the wrong branch for an
  empty `xs`, disagreeing with the interpreter (which correctly treats
  them as falsy, like `0`/`false`/Unit). The tier's `is_truthy` now
  mirrors the interpreter's `to_boolean` exactly, pinned by a differential
  test. Found while writing the new Common Pitfalls chapter.
- **Name-colliding functions now promote to the bytecode tier (viz
  finding #3).** The tier dispatches `CallNamed` by name and marked any
  name shared by two distinct function bodies "ambiguous", never tiering
  it. Because embedded packages register their *private* helpers (`viz`'s
  `col`, `opt`, `groups`, `distinct`, `fmt`, …) into the global name
  space, any program reusing one of those common names — a data app, the
  chart gallery — silently ran that function, and its hot `map`/`filter`
  loop, on the interpreter. An ambiguous name is now dispatched **by body
  identity** (compiled under its own closure, keyed on the body pointer),
  so the correct body runs on the tier. A `use viz` program mapping a
  field-accessor lambda over records went **~1480 ms → ~170 ms native
  (~8.6×)** and **~1480 ms → ~140 ms in the wasm playground (~10×)**;
  results are identical on both tiers.

### Documentation

- **The docs are centered on the Open Language identity.** A new
  [Openness chapter](docs/openness.md) — now the book's opening chapter —
  states the three-pillar identity (open code, open artifacts, open
  execution), why it is uniquely possible for olang, and maps each pillar
  to its detailed home. The book landing, the repo README, and the
  website hero/metadata all lead with it; the Cargo package description
  follows. The name `olang` is unchanged — it already stands for *Open
  Language*.

- **New chapter: [Common Pitfalls](docs/pitfalls.md).** The language's
  sharp edges collected in one place with the idiom that avoids each —
  missing map keys returning Unit, integer division, the `&&`/`||` and
  range precedence surprises, `Result`/struct equality, closure capture,
  truthiness, indexing/slicing, empty-list builtins, `match`-pattern
  binding, bare-block scoping, and `os.args` argv. Wired into the book,
  the website, and the doc-examples test (every example is executed).
- Documented the three embedded packages `ui`, `viz`, and `dash` with
  function tables in the stdlib chapter (previously prose-only; `ui.esc`,
  `dash.stat`, and `dash.half` were undocumented), corrected the stdlib
  module count, completed the tooling chapter list, and added operator
  associativity + non-standard-precedence warnings to the language
  reference.

### Changed

- **`olang build` embeds the parsed AST, not just source (rung B).** A
  built standalone now carries its program's *pre-parsed* AST and
  deserializes it at startup instead of re-running the parser —
  deserialize is ~20× faster than parsing (a 400-function tool's startup
  parse of ~12 ms becomes ~0.6 ms, ~10 ms off cold start). The source is
  bundled too, only so runtime error snippets still render; bundles built
  before this (raw source, `oLaNgBnd`) still run. The AST round-trip is
  lossless across the language (enums, structs, closures, patterns,
  recursion), verified against the interpreter. Rung B of `olang build`
  (Toolsmith T4).
- **Faster `map`/`filter` on the bytecode tier.** The native `map`/
  `filter` loop now fetches the callee's bytecode and validates its arity
  and parameter checks once, before the element loop, instead of on every
  element (an `execute_prepared` fast path). ~16% faster on the hot
  builtin-lambda pattern natively; the JIT path is untouched.

- **Embedded packages parse once per process (faster cold start).** The
  built-in olang packages (`cli`, `term`, `viz`, `dash`, `colx`, `mathx`,
  `ui`) are compiled into the binary as source and were re-parsed on
  every `use` — and a fresh interpreter, which re-parses, is created for
  every CLI invocation, every `olang test` file, and every playground
  run. Their parsed AST is now cached process-wide (the source is
  immutable), so the parse is paid once. A 30-file `olang test` where
  each file uses `cli`+`term` dropped from ~111ms to ~44ms (2.5×), and
  the playground's repeat runs skip embedded parsing entirely. Lane T5 of
  the **Toolsmith campaign**.

## [0.57.0] - 2026-08-14

### Added

- **`survey` — the command-line flagship (dogfood).** A new example
  (`examples/tools/survey`): a codebase surveyor that turns a directory into a
  report — colored totals, a per-language bar chart, aligned tables, and
  a live progress bar — exercising `cli`, `term`, and `fs` in one
  self-contained file. It is the terminal counterpart of the tracker web
  suite: written with the toolkit, shippable via `olang build`,
  documentable via `olang doc`. `term.table` also gained **visible-width
  alignment** (`term.visible_len`), so colored/styled table cells now
  align correctly.

- **`olang doc` — API reference from doc comments.** A new subcommand
  that scans `.ol` files for a source-level doc convention — `//!` for a
  module note, `///` above a declaration for its docs (both already
  valid olang comments) — and renders a browsable, suite-themed HTML
  page (or Markdown with `--md`). Only documented declarations appear.
  The `cli` and `term` packages now carry `///` docs, so their reference
  regenerates from source. Lane T6 of the **Toolsmith campaign** — the
  "maintain" layer.

- **`olang build` — standalone single-file executables.** `olang build
  prog.ol -o tool` bundles a program into a self-contained binary that
  runs with no olang installed. No C compiler or linker: `build` copies
  the runtime and appends the parse-checked source with a trailing
  marker; at startup the binary detects it and runs the embedded program
  with the full process argv (so a bundled tool's own flags reach
  `cli.args()` intact). The whole stdlib and the embedded packages
  (`cli`, `term`, …) travel inside it; single-source tools bundle
  cleanly. `examples/greet.ol` is a worked example. Lane T4 (Rung A) of
  the **Toolsmith campaign** — the "ship it" half of the build-and-ship
  arc.

- **`term` — the terminal toolkit.** A new embedded olang package
  (`use term`): ANSI color and text styling (`red`/`green`/`bold`/… and
  a general `style(s, opts)`), aligned `table`s and `rule`s, a progress
  `bar`, and interactive `prompt`/`confirm`/`select`. Styling gates on
  whether it will render — stdout is a TTY and `NO_COLOR` is unset, or
  `CLICOLOR_FORCE` is set — so the same program is colored on a terminal
  and plain in a pipe with no extra logic. Two native primitives back
  it: `os.is_tty()` and `os.flush()`. The `taskcli` example colorizes
  its summary and renders a `term.table` (plain when piped). Lane T2 of
  the **Toolsmith campaign**.

- **`cli` — declarative command-line argument parsing.** A new embedded
  olang package (`use cli`) that turns a program's argument surface into
  a spec map: typed flags (`bool`/`int`/`float`/`string`, short + long,
  defaults, `required`, env fallback), positional arguments,
  subcommands, and auto-generated `--help`. `cli.parse(spec, argv)` →
  `Ok(values)` \| `Err(message)`; `cli.help(spec)` renders usage;
  `cli.args()` is `os.args()` with the program path dropped. The
  `taskcli` example is rewritten onto it — its whole command surface
  (`list`/`open`/`stats`/`add --priority`) is one spec, with help and
  clean exit codes for free. First lane of the **Toolsmith campaign**
  (build & ship real command-line tools in olang).

## [0.56.0] - 2026-08-14

### Added

- **`dom.state` — a session-state primitive.** `dom.state_set(key,
  value)` / `dom.state_get(key)` name the DOM-resident-state pattern
  every closure-by-value browser app rediscovers: a blessed,
  JSON-typed, page-lifetime store (a Map or list round-trips; a
  missing key reads as Unit; not persisted — `storage_*` is the
  localStorage path). The charts page's cross-filter now rides it
  instead of a hidden input.

### Changed

- **`if`'s `=>` may start a new line.** A long condition can wrap
  before the arrow (`if a && b && c\n    => ...`) — the syntax
  friction that bit most when dogfooding viz. Same-line and `else if`
  chains are unchanged.
- **Charts use whole-number gridlines for count data.** `nice_ticks`
  detects whole-valued data and floors a fractional step to 1, so a
  bar chart of counts reads 0,1,2,3 rather than 0,0.5,1,…. Float data
  keeps fractional ticks.

## [0.55.0] - 2026-08-13

### Added

- **Embedded packages import each other.** The builtin olang packages
  (`ui`, `viz`, `dash`, …) can now `use` one another — the resolution
  already handled it (embedded modules resolve before the filesystem,
  natively and in wasm), but nothing used or tested it. `dash` now
  imports `ui`'s HTML escape (`use ui { esc }`) instead of
  re-implementing it, so the discipline lives in one place, with a
  test pinning that loading one embedded package transitively loads
  its imports.

- **Vectorized Series math — `ods.map`.** `ods.map(series, name)`
  applies a `math.*` unary function (sin, cos, exp, sqrt, ln, floor,
  … — 24 in all) across a whole column in one native kernel pass. The
  name is a String, not a closure, precisely so the loop stays in the
  kernel: it is the vectorized form of `map(xs, (v) => math.f(v))`
  with no per-element boundary crossing. Bit-identical to the scalar
  function (same `f64` methods), nulls propagate, domain-restricted
  functions raise the same error, and it composes with the existing
  elementwise Series arithmetic into full expressions
  (`ods.map(xs, "sin") * 2.0 + 1.0`, one kernel per term). A sin·2+1
  transform over 50,000 points: **238 ms → 1 ms (~240×)**, same
  checksum — and it runs in the interpreter kernel, no promotion
  needed. The gallery's 50k-point curtain is now generated this way.

- **In-place list building — the AddAssign of collections, on every
  tier.** `xs = xs + [v]` in a loop was O(n²): each iteration copied
  the whole list. Now it appends in place when the accumulator holds
  the only reference to its `Vec`, O(1) amortized, with an
  `Arc::get_mut` aliasing guard identical to the string case — a
  snapshot taken before an append is never mutated, self-append
  copies, a nested list is unharmed. The **bytecode tier** got this
  first (a promoted 60,000-element build: **3807 ms → 2 ms**), and now
  the **interpreter** does too: `Value::List` moved from a fixed-size
  `Arc<[Value]>` to a growable `Arc<Vec<Value>>` (reads deref
  identically, so it was a handful of edits), and the assignment path
  gained the same fusion. An unpromoted interpreter build of 8,000
  elements: **215 ms → 1 ms**. Cold, hot, native, and wasm paths are
  all O(n) now, pinned by interpreter-only and cross-tier differential
  tests.

## [0.54.0] - 2026-08-13

### Added

- **`plot` grows into a charting library.** Five new chart types —
  `plot.area`, `plot.bars` (grouped), `plot.stacked`, `plot.heatmap`
  (sequential color scale with a min/max key), and `plot.box`
  (five-number summaries) — joining line, scatter, lines, bar, and
  hist. Two new options on every chart: `theme: "dark"` re-tunes the
  full palette (surface, ink, grid, series hues, heat scale) for dark
  pages, and `responsive: true` drops the fixed pixel size so the SVG
  fills its container while the viewBox keeps the aspect ratio.
  Stacked bars refuse negative values; box plots drop nulls.
- **The `viz` grammar — charts as values.** `use viz`, an embedded
  olang package: a chart is a spec map holding data (records or a
  Frame), a mark, and column-name encodings. `color` splits rows into
  series, bar marks aggregate rows sharing a category (`"stack": true`
  stacks), `layers` composes marks over shared scales, and
  `viz.chart(spec)` compiles to plot SVG — pure, natively tested.
  `viz.draw(canvas, spec)` compiles the same xy specs to a canvas
  draw-list for point counts SVG can't carry. Underneath, `plot.xy`
  is new: layered mixed marks (line/area/scatter, each with its own
  x) in one document over shared scales.
- **The color system — expressive, not monochrome.** Ten curated hues
  per theme (mint leads dark, classic ten on light). Every chart can
  own its palette (`colors`), single-series bars can color each
  category (`vary`), heatmaps pick multi-stop ramps (`scale`:
  ocean/ember/thermal/diverging — diverging runs cold through the
  surface to warm for signed data), and `plot.ramp(name, t)` exposes
  the ramps to olang code. The viz grammar gains **`color_by`** —
  continuous color encoding per point, rendered individually in SVG
  and bucketed into at most 24 bulk calls on canvas. `dash.stat` adds
  accent-colored KPI tiles. The pages put it to work deliberately:
  varied status bars, an ocean heatmap, violet boxes and histograms,
  cyan-amber cumulative charts, a diverging interference field, an
  ember-ramped attractor, a magenta dust arm in the galaxy, a
  four-voice rose, and a curtain that sweeps the spectrum.
- **Fidelity and cohesion — the suite becomes one program.** Every
  canvas now renders at devicePixelRatio (the backing store scales
  once, the context pre-scales, all drawing stays in design units) —
  the blurriness on retina displays is gone. Charts are designed at
  the size they display: pages pass card-matched viewBoxes, tick and
  category type grows to 12.5px, titles to 16.5px, and the two-column
  grid gives every chart room to read. One design system
  (`suite.css`, served by the app) carries the tracker's palette to
  all seven pages — mint `#3ddc97` leads the dark series order, chart
  surfaces match the card panels, and the nav, headers, cards,
  buttons, and inputs share one look. The viz canvas target, tooltip,
  and dash stylesheet all speak the same tokens.
- **The roadmap gains the viz campaign's findings** — the
  language-level growth list the campaign surfaced (in-place list
  append as the AddAssign of collections, vectorized Series
  transforms, embedded-package imports, a session state primitive,
  syntax friction), recorded so the next lanes aim where data work
  actually pushed back.
- **The `dash` kit and the ops board.** `use dash`: KPI tiles, cards,
  wide cards, and the grid as pure HTML builders (escaped, natively
  tested) plus `dash.styles()` so a dashboard page ships no CSS. The
  flagship `/board.html`: a KPI row and five viz charts over live
  tracker data, a status filter carried in the URL (bookmarkable,
  back-button correct), one delegated tooltip for every chart, and a
  5-second auto-refresh — one olang source file end to end. Fetch
  failures now keep the last render on both data pages instead of
  erroring.
- **The beauty pass.** Area marks fill with a vertical gradient fading
  toward the axis (SVG, both themes); `dom.draw_points` gains a
  host-side rotation term; the gallery's spiral becomes a 12,500-star
  galaxy spinning on that one parameter, and the finale curtain grows
  to 50,000 points — still at frame rate.
- **The binary bulk path — big data at frame rate.**
  `dom.draw_points(canvas, xs, ys, style)`: coordinates cross the wasm
  boundary as ONE packed f64 buffer the page reads as a zero-copy
  typed-array view — no JSON, no per-point cost. Series are the fast
  lane (the data stack feeds the graphics pipeline directly); plain
  lists work too; nulls drop pairwise. Styles: points or path mode,
  color, size, alpha, and a host-side affine (sx/sy/tx/ty) — so
  animation re-sends the same buffer with new transform parameters and
  olang does zero per-point work per frame. `viz.draw` compiles point
  and line marks onto this path, and specs whose data is a Frame keep
  columns as Series end to end. The gallery finale animates a
  30,000-point Lissajous curtain at ~120 fps.
- **Interactive charts — hover, click-to-filter, brush.** With
  `"interactive": true` (plot option and viz spec key), marks carry
  their datum as `data-*` attributes: scatter points, bars and stacked
  segments, heatmap cells, and boxes become event targets, escaped and
  off by default. Three viz helpers ride the existing structured-event
  path: `viz.tooltip(el)` (floating datum tip on hover),
  `viz.on_mark(el, event, handler)` (handler fires only on mark hits,
  receiving the datum), and `viz.brush(el, handler)` (horizontal
  press-drag-release as width fractions). The charts page cross-filters
  every card from clicked status bars; the gallery forecast zooms by
  brushing and resets on double-click.
- **The data stack meets the page.** Two new pages in the example app:
  `/charts.html` — live tracker analytics (one `dom.fetch_json`, then
  `ods.frame_from_records`, frames, and plot SVG landed with
  `dom.set_html`) — and `/gallery.html`, the standing data-viz
  showcase (computed art and statistical pieces, plus a live draw-list
  centerpiece) where new viz capabilities land first. Both wired into
  the suite nav.

## [0.53.0] - 2026-08-13

### Added

- **`dom` workers — browser parallelism out of the box.** `dom.worker(path)`
  boots a second olang program in a Web Worker (its own thread, its own wasm
  instance); values cross as JSON both ways via `dom.worker_send` /
  `dom.worker_on` on the page and `dom.post` / `dom.on_message` inside the
  worker. Workers can post mid-computation, so long jobs stream progress
  while the page's frame loop keeps running. New `/primes.html` demo in
  `examples/web/app`: a prime counter with a live progress bar and an animation
  dial proving the main thread never blocks.
- **`dom.fetch_json`** — `dom.fetch`, but the callback receives the parsed
  response value directly instead of raw text.

### Changed

- **The example app is now a linked suite.** All four pages — the tracker,
  `/orbit.html`, `/notes.html`, `/primes.html` — share a nav, link to each
  other, and link their own `.ol` source; the primes button follows the
  worker lifecycle (disabled until ready and while counting).
- **The browser chapter caught up with the platform.** `docs/wasm.md` now
  documents the full dom surface: structured event Maps, the draw-list and
  frame loop, the `ui` view layer, routing and storage, Web Workers, and
  `fetch_json`, with a guided reading of all four frontends. `docs/ovm.md`
  documents the scratch watermark, inlining + scalar replacement, the
  collection/Result/Map JIT kinds, and tier-identical error traces;
  `docs/language.md` documents runtime stack traces.
- **Sophisticated web apps: the `ui` view layer, routing, and
  storage.** `use ui` — an embedded olang package — builds pages as
  values: `h(tag, attrs, children)` trees, `hk` reconciliation keys,
  pure `html()` rendering (escaped, testable natively), and
  `ui.render(el, children)` with keyed reconciliation: unchanged
  children untouched (input state and focus survive), changed ones
  re-rendered in place, additions/removals surgical, reorders
  repositioned via the new `dom.insert_before`. Alongside it the dom
  module gains SPA navigation — `push_state`, `location` (a Map of
  path and query), `on_route` for back/forward — and localStorage
  (`storage_get`/`set`/`remove`). The proof is
  `examples/web/app/static/notes.ol` at `/notes.html`: a notes SPA
  verified live in a browser — selection updates the URL, the back
  button unselects through `on_route`, additions reconcile in, and
  notes persist across the session. The dom harness pins routing,
  storage, and the reconciliation algebra (update-in-place, removal,
  reorder) against the real wasm build; three native tests pin the
  pure renderer.

- **Rich graphics out of the box: canvas draw-lists.** A scene is plain
  olang data — a list of op maps (`clear`, `rect`, `circle`, `line`,
  `path`, `text`, plus `save`/`restore`/`translate`/`rotate`/`scale`) —
  submitted with one `dom.draw(canvas, ops)` call per frame and
  replayed onto the canvas 2D context by the page. `dom.on_frame`
  completes the loop: register once, called every frame with a
  millisecond delta, no per-frame handler registration. The proof is
  `examples/web/app/static/orbit.ol`, served by the tracker at
  `/orbit.html`: an animated orbital system with motion trails where
  clicking adds a body at the clicked radius (structured event
  coordinates + `dom.measure`), verified live in a browser at 60fps.
  The dom harness pins the draw-list round-trip — op order, numeric
  fidelity, nested point lists — against the real wasm build.

- **The dom module grows up: structured events, node control, and
  time.** Every event handler now receives a structured event Map —
  `type`, target `id` and `value`, `key`, pointer `x`/`y`, modifier
  flags, and the target's `data-*` attributes — the same shape for
  every event, so any DOM event name works (`input`, `keydown`,
  `pointermove`, `submit`, `wheel`, …) and delegation stays the natural
  style. Sixteen new functions land alongside: attribute get/set/
  remove, classList add/remove/toggle, per-property styles,
  `dom.measure` (bounding rect as a Map), surgical `create`/`append`/
  `remove` structure edits, `scroll_into_view`, timers
  (`set_timeout`/`set_interval`/`clear_interval`), and
  `dom.request_frame` for animation loops with a millisecond delta.
  Structured payloads ride a new JSON dispatch entry over the wasm
  boundary; the tracker app is migrated to the Map shape and verified
  live in a browser (delegated clicks, change edits, and Enter-to-add
  all round-tripping to the server), and the dom harness proves the
  whole surface end to end against the real wasm build.

## [0.52.0] - 2026-08-13

### Added

- **Scalar replacement — structs and lists that never escape are never
  allocated.** At planning time the JIT now inlines tiny leaf callees
  (a constructor, a field-math helper) into their callers, propagates
  single-definition copies, and then dissolves aggregates: a struct
  whose register is only ever field-read becomes one register per
  field; a list read only at constant indices becomes one register per
  element. Construction disappears entirely — no allocation, no helper
  call, no guard — and the loop that built two structs per iteration
  compiles to pure float arithmetic. Measured: the struct-building
  benchmark drops from 1.475s to ~14ms (105x, now ~4x faster than V8
  on the identical workload) and the list-building one from 0.223s to
  ~7ms (30x, ~8x faster than V8). The transforms exist only on the
  JIT's planning clone — the VM's bytecode is untouched, and every
  error path deopts to a clean rerun, so semantics and stack traces
  cannot drift.

- **Loops may allocate now — the scratch watermark.** Native loops were
  forbidden from allocating (every scratch allocation lived until the
  call ended), which kept struct-, list-, map-, and Result-building
  loops on bytecode. The JIT now marks every scratch family's length at
  loop entry and truncates back to the mark on each taken back-edge,
  freeing the iteration's allocations — permitted exactly when a
  liveness gate proves every heap value born in the loop dies in its
  iteration (nothing carried across the back-edge, nothing read after
  the loop; values that escaped into another owner survive on their own
  reference count). Loops that fail the proof refuse and run on
  bytecode, byte-identically, with a precise debug diagnostic.
  Measured with `olang bench` against a saved baseline: struct-building
  loops 60% faster (2.5x), list-building loops 56% faster (2.3x),
  scalar kernels untouched, and allocation-heavy loops now run past the
  old 1M scratch cap natively instead of deopting mid-loop.

- **`olang bench` — reproducible timings and a regression guard.** Each
  `.ol` file runs as its own subprocess (fresh VM and JIT per run, the
  wall-clock a user experiences): one discarded warmup, then timed runs
  with median, min, max, and coefficient of variation per row, plus a
  warning when runs disagree on their output. `--save base.json` stores
  a baseline; `--against base.json` compares to one, calling a row
  changed only when it moves more than max(5%, 2×CV) — below that it's
  noise, not news; `--fail-on-regress` turns red rows into exit code 1,
  making a saved baseline a standing guard for performance work.
  Documented in the tooling chapter.

## [0.51.0] - 2026-08-13

### Fixed

- **Trivial constructors no longer pay the JIT call boundary.** A body
  that exists to allocate — `fn make(i) = [i, i * 2]` and its struct,
  map, and Result siblings — does the same allocation on every tier, so
  crossing the bytecode→native boundary (argument marshalling, scratch
  context, retain resolution, unmarshal) was pure cost: ~26% slower
  than v0.50.0 on a 3M-call constructor loop. Allocating bodies with at
  most eight compute instructions now decline the boundary and stay on
  bytecode, while still compiling as group members reached by direct
  native call. Boundary calls that do run got cheaper too: all-scalar
  calls skip the per-family argument sweep the collections lane added,
  and the scratch context is reused across calls (native calls never
  nest), so allocating bodies stop paying a malloc/free per call for
  bookkeeping. Trivial and compute-heavy constructor loops are both
  back at (or slightly better than) v0.50.0 timings, and guard tests
  pin the policy via the new `jit_native_calls` tier stat (also shown
  by `--ovm-stats`).

### Added

- **The JIT builds lists.** List literals (`[a, b]`) and list
  concatenation (`xs + [v]`) compile to native code in straight-line
  functions, and list-returning constructors hand ownership back across
  the entry boundary — previously any list construction or list return
  refused the JIT outright. Construction is scratch-owned like structs
  (a deopt can never leak), uniform `Int`/`Float` elements specialize,
  and everything else — mixed elements, string lists, empty literals,
  list-building loops — stays on bytecode with identical results. `->
  List` return annotations discharge statically. Eleven differential
  tests pin the seam.

- **Struct-element lists compile, and native callers see through their
  callees.** `[Point{..}, Point{..}]` literals specialize as struct
  lists: the helper resolves each borrowed element pointer back to an
  owned Arc, so the list owns its elements exactly like the VM's (mixed
  shapes and struct/scalar mixes refuse to bytecode). Alongside it, a
  latent inference gap is fixed: indexing or field-reading a value built
  by a *callee* used to refuse the whole function because the callee's
  return kind resolves one fixpoint iteration late — it now defers
  instead, so constructor-then-consume pipelines (`let pts =
  segment(a, b); pts[1].x`) compile end to end.

- **String-element lists join the JIT.** `ListStr` is the fourth list
  kind: `["alpha", "beta", tag]` literals construct natively, elements
  read back as borrowed strings (comparison and concat work on them
  directly), and string lists concat, return, and iterate like every
  other list kind. Constant elements are content-cloned — strings are
  immutable values with content equality, so identity is unobservable
  and there is no deopt case beyond the allocation cap.

- **Maps enter the JIT — the collections lane is complete.** String-keyed
  maps with a uniform value payload specialize: `#{...}` literals
  construct natively, `map_get` compiles as a key-guarded read (a miss —
  where the language returns Unit — deopts to bytecode, never misreads),
  `map_has_key` is a total native test that compiles inside loops, and
  `map_set` clone-and-inserts into scratch exactly like the VM's
  immutable-map native. Because these are *named builtins* and a user
  definition can shadow a builtin name, compiled code records which
  natives it baked — transitively through call-graph edges — and the
  moment a user function takes one of those names, every affected entry
  is demoted back to bytecode. Mixed-value maps, stringified non-string
  keys, and object receivers all stay on bytecode with identical
  results; `-> Map` return annotations discharge statically.

- **String building is linear now.** The accumulate pattern `s = s +
  piece` fuses into a single instruction that appends in place when the
  string is uniquely referenced — the Arc count proves no alias can
  observe it, and strings are immutable values with content equality,
  so identity is unobservable. Building a 400KB string by 200k
  concatenations drops from ~1.6s to ~10ms (it was O(n²), it is now
  O(n)) — faster than CPython's specialized in-place append on the
  same workload. Aliased and self-referencing strings copy exactly as
  before, fusion declines when the right side could assign, and the
  JIT canonicalizes the fused form back to a plain add so hot numeric
  loops compile unchanged.

- **Channels: `spawn`ed tasks can talk.** The new `chan` module is
  message passing between tasks — multi-producer multi-consumer queues
  of olang values. `chan.new()` is unbounded; `chan.bounded(n)` holds
  at most `n` in-flight messages and blocks senders when full
  (`bounded(0)` is a rendezvous). `send`/`recv`/`try_recv`/
  `recv_timeout` all speak Result, and closing is cooperative: after
  `chan.close(c)` sends fail but queued messages still drain, so a
  consumer loop just matches on `recv` and stops on `Err`. Handles are
  plain values that cross the `spawn` boundary like anything else;
  heap values (lists, maps, structs) travel intact. Documented in the
  stdlib chapter with a tested producer/consumer example.

- **The checker proves match exhaustiveness over literal enums.** A
  union of literals declares exactly which values are admissible, so
  `olang check` (and the editor) now knows what covering every case
  means: a `match` missing a member gets an advisory warning naming
  what's absent (`match is not exhaustive: "done" has no arm — add it
  or a catch-all`), an unguarded binding or `_` covers everything, and
  a match whose arms cover *none* of the admissible values is reported
  as a runtime error — it fails on every run. Or-patterns count member
  by member; guarded arms prove no coverage (a guard may reject);
  dynamic scrutinees are never judged. Documented in the types chapter.

- **Runtime errors on the bytecode tier carry the same trace as the
  interpreter.** A runtime failure now points at the innermost located
  statement and lists the call stack — identically on every tier.
  Compiled bytecode carries statement-granularity span markers, the VM
  records frames as the error unwinds (with the interpreter's exact
  frame-visibility semantics, quirks included), and the tier splices
  its trace onto the interpreter's live stack. Previously a tier-run
  error pointed at the top-level statement with no stack at all. Seven
  differential tests pin the whole ErrorLocation equal across tiers.

### Fixed

- **The bytecode tier reads operands in interpreter order.** A binary
  op whose left operand was a plain variable compiled to the variable's
  register itself, so `acc + { acc = 1  5 }` read `acc` *after* the
  right side mutated it — the tier answered 6 where the interpreter
  answers 15. The same late-read hazard lived in every multi-operand
  position: call arguments (and the callee and method receiver
  themselves), list/tuple/map/struct/anonymous-object literals,
  template interpolations, index and range operands, a match scrutinee
  re-tested after an assigning guard, and a for-loop iterable
  reassigned by its own body. The compiler now shields exactly the
  provable case — the operand's register is a variable's home register
  AND a later operand contains an assignment (the `assignment_free`
  whitelist from AddAssign fusion) — by copying the value to a fresh
  register at its evaluation point; assignment-free operands, i.e. all
  hot paths, emit no Move. Fourteen differential tests pin every
  position, each also asserting promotion so a silent refusal can't
  fake agreement.

- **The JIT enforces parameter annotations.** A specialization whose
  observed argument kinds could not provably satisfy the function's
  parameter annotations compiled anyway and skipped the check — so a
  hot `fn double(n: Int)` called with a Float returned a wrong value
  instead of the type error every other tier raises (present since the
  JIT and gradual typing first coexisted; surfaced by the new trace
  tests). Parameter annotations now discharge statically exactly like
  return annotations: unprovable specializations stay on bytecode,
  which checks per call.

## [0.50.0] - 2026-08-12

### Added

- **The JIT compiles Results.** `Ok`/`Err` construction, pattern tests,
  and payload extraction now run natively: Results ride borrowed
  pointers like structs, payload reads are guarded per side (a surprise
  deopts to bytecode, never misreads), and construction is scratch-owned
  so a deopt can never leak. Mixed return paths join — a function
  returning `Ok(n)` on one branch and `Err(code)` on another compiles as
  one specialization — and calls carrying the side a specialization
  never saw fall back to bytecode with identical results. `-> Result<T,
  E>` return annotations are discharged statically: the JIT only
  compiles a function whose inferred payloads provably satisfy the
  annotation. String payloads extract natively; string construction and
  Results built inside loops stay on bytecode by the same allocation
  discipline as structs. Eleven differential tests pin the seam shut.

- **The checker and language server see across module boundaries.** A
  file's `use`d modules are resolved with the runtime's local
  conventions and parsed alongside it: imported `share fn` signatures
  feed diagnostics in both `olang check` and the editor (a wrong
  argument to an imported function is flagged in the importing file),
  hover shows an imported function's typed signature marked with its
  source file, and go-to-definition jumps into the module. Also
  closed in passing: `share`d declarations were entirely invisible to
  the checker even within one file — their signatures now register and
  their bodies are checked (the corpus's 171 files, heavy with
  `share fn`, stay clean).

- **`olang check` warns when code relies on block leakage.** A bare
  block's `let`s remain visible afterwards — the other long-documented
  scoping pitfall. Using (or assigning) such a name after its block now
  draws an advisory warning, once per name, in `olang check` and the
  editor. Leak tracking is per function frame, so a block inside one
  function never taints another; declaring the name before the block
  is the fix and stays silent. The repo's 171 files produce zero.

- **Literal types check by value — unions of them are lightweight
  enums.** The last useful reserved annotation form graduated:
  `s: "open" | "in-progress" | "done"` admits exactly those strings
  (`parameter 's' of set_status expects "open" | "in-progress" |
  "done", got "cancelled"`), with int and bool literals equally valid
  branches. Enforced by one value comparison at every boundary on
  every tier; the checker proves violations when the argument is
  itself a literal (or a literal-annotated binding) and stays silent
  on dynamic values; scalar actuals now name their value in error
  text on both tiers ("got true", not "got Bool"). Intersection
  annotations remain the only reserved form.

- **The tracker example is a real product now.** `examples/web/app` grew
  from a bare grid into a full app — live search, status filter pills,
  sortable columns (assignee joined the sortable set), an issue drawer
  with editable title, cycling pills, and comments, a stats strip
  rendered from `/api/stats` (quantiles computed server-side on ods),
  and a live activity ticker — all in ~330 lines of olang running as
  WebAssembly, same stateless/delegated architecture as before (client
  state is four hidden inputs; twelve listeners bound once at boot).
  The `dom` module grew one function to make it possible:
  `dom.set_class(el, classes)` sets an element's class list wholesale —
  the stateless way to toggle visual state (a drawer's `open`, a
  pill's `active`). The wasm.md guided reading walks the new frontend.

### Fixed

- **TOML datetimes parse as plain strings.** The toml crate represents
  datetimes as a private one-key wrapper object over serde, which
  leaked through `toml.parse` as
  `{$__toml_private_datetime: "..."}` instead of the string the
  documentation promises. Unwrapped recursively; found by the pre-0.50
  edge-case sweep and regression-pinned.

- **A vanished stdout reader no longer kills olang programs — or the
  HTTP server.** `print`/`println` used Rust's `println!`, which
  aborts the whole process with "failed printing to stdout" the moment
  a pipe consumer exits — so `olang gen.ol | head -1` panicked instead
  of ending, and (the root cause of the long-standing http_serve_test
  flake) a server whose parent read the port line and closed the pipe
  before the second boot line died at startup under load, resetting
  every in-flight connection. Program output now emulates the Unix
  SIGPIPE default — terminate quietly with the conventional 141 — so
  olang composes in pipes like any well-behaved filter, and the
  server's informational boot lines ignore a closed stdout entirely
  (the socket is its real interface). Verified with 24 consecutive
  suite executions under the load pattern that previously failed
  within two.

### Added

- **`olang check` warns on assignment to undeclared names.** `x = 1`
  without `let` creates a binding — long documented as a pitfall with
  "a future release may warn here"; this is the release. Advisory
  only: rendered as a warning in `olang check` (exit code unchanged)
  and as a warning squiggle in editors; declared names, loop
  variables, parameters, and valueless `let`s stay quiet, and the
  first assignment binds the name so it warns once. The repo's own
  171 files produce zero warnings.
- **`testing.test_summary()` and `reset_tests()` are real.** Every
  `testing.assert_*` outcome is tallied (per thread — `par_map`
  workers keep their own counts): `test_summary()` returns
  `#{ "passed", "failed", "total" }` and `reset_tests()` zeroes it.
  `run_test` deliberately remains an honest redirect to `test` blocks.

- **`olang --watch script.ol` — the edit-run loop as a flag.** Reruns
  the script whenever any `.ol` file at or below its directory changes
  (module edits count), each run in a child process so a crash or
  `os.exit` ends the run and never the watcher; a save landing mid-run
  queues an immediate rerun. Polling, no new dependency.
- **REPL `:type` reports deep types.** Previously the shallow name;
  now the value's structure: `:type [1, 2, 3]` answers `List<Int>`,
  `Ok([1.5])` answers `Result<List<Float>, _>`, and a lambda shows its
  annotated signature (`(Int) -> ?`). Mixed elements fall back to the
  honest base (`List`), never a guess.

- **`toml` module — parse and emit the config format olang itself
  uses.** `toml.parse(text)` yields the same value shapes JSON objects
  do (tables → Maps, arrays → Lists, datetimes → strings; both modules
  bridge through the same serde conversions, so they cannot drift),
  `toml.stringify(map)` emits pretty TOML (a document is a table, so
  non-table values are a clean `Err`), and `toml.validate(text)`
  answers without erroring. The `toml` crate was already in the tree
  via the package manager — this is surface, not a new dependency. The
  stdlib is now twenty-one native modules.

- **Scripts are first-class: shebang, stdin, and path helpers.** A
  leading `#!/usr/bin/env olang` line now parses (masked, not
  stripped — every error line number and span still matches the file
  on disk), so `chmod +x script.ol` works. `os.stdin()` reads all of
  standard input and `os.stdin_lines()` yields it as lines with
  endings stripped — the pipe primitives (`cat log | olang
  analyze.ol`). And `fs` gained path surgery: `join(parts)`,
  `dirname`, `basename`, `ext`, and `abs_path` (absolute against the
  current directory with `.`/`..` normalized lexically — the file
  need not exist). Environment access needed nothing: `os.get_env` /
  `set_env` / `has_env` / `list_env` and `os.home_dir` already
  existed and are now regression-covered alongside the new surface.

### Fixed

- **The tracker example no longer defaults to a port macOS owns.**
  `examples/web/app` listened on 7000 by default — a port macOS AirPlay
  Receiver (Control Center) binds on every modern Mac and answers
  with `403 Forbidden`, so a browser hitting the app when it wasn't
  running got AirPlay's baffling "access denied" instead of
  connection-refused. The default is now 7317 (docs updated;
  an explicit port argument still works as before). Also fixed in
  passing: the boot-time wasm-artifact check used `fs.read_file` on a
  binary file, which fails on non-UTF-8 bytes and warned "missing"
  even when the artifact was present — it now uses `fs.exists`.

### Added

- **Editor hover shows types.** The language server's hover now
  carries the static checker's knowledge: annotated function
  signatures render in full (`fn dist(a: Float, b: Float) -> Float`),
  and unannotated `let` bindings show their inferred type when the
  checker can prove one (`let total: Int` for `let total = 1 + 2`).
  Unknown stays honest — a binding the checker can't type hovers as
  plain `let mystery`. Also recorded: the JIT note from 0.49's Result
  entry is now precise — Result values have no JIT representation at
  all, so `-> Result<...>` annotations cost nothing on the native
  tier that Result-returning code didn't already forgo; a future
  Result kind in the JIT would need its own discharge rule.

- **`Promise` annotations check at non-async sites, and the checker
  follows `await`.** A parameter or binding declared `Promise<Int>`
  now rejects a non-promise value at the boundary on every tier
  (previously the dedicated annotation form reduced to no check). The
  payload at rest stays unchecked — it doesn't exist until resolution,
  where the async return check already enforces it — but the checker
  flows it: async function signatures join the checker's world
  (calling one yields `Promise<T, E>`; a bare `-> T` wraps), `await`
  carries the resolved type onward, and passing an unawaited call
  where the payload type is expected is flagged (`expects Int, got
  Promise`).

- **Function-type annotations enforce callability and arity.**
  `f: (Int) -> Int` now verifies at the boundary that the value is
  callable (function or builtin) and can be called with exactly the
  annotation's parameter count — respecting default-parameter ranges
  (`expects (Int) -> Int, got a function taking 2 parameters`), on
  every tier, with the signature (not the bare word "Function") in
  the error text. Values that don't expose parameter counts (builtins)
  check callability only. Signature *types* are the checker's
  territory: a lambda whose own annotations contradict the declared
  signature is flagged as a promise-break (`expects (Int) -> Int, got
  (String) -> ?`), and lambdas now carry their signature through the
  checker's inference.

- **Union annotations gained semantics — `A | B` is enforced.** The
  reserved form graduated, additively as promised: a value satisfies a
  union if it satisfies any branch, checked at every annotated boundary
  on every tier at the same O(1) cost (`parameter 'x' of tag expects
  Int | String, got Bool`). Branches keep their own rules — a
  `Result<Int, String> | Int` union applies the Result payload check
  when the value is a Result — and a union containing an unenforceable
  branch (generic parameter, function type) stays entirely unchecked
  rather than wrongly strict. The union grammar also learned the
  parameterized branch forms (`Result<...>`, `Promise<...>`, `Name<T>`,
  `Map<K, V>` now parse as branches). The checker mirrors the runtime
  with its usual honesty: a violation is reported only when every
  branch is provably violated, labeled a runtime failure only when
  every branch's own check would fire (`expects List<Int> | Int, got
  List<String>` is a promise-break — the shallow runtime admits the
  List). Intersection and literal-type annotations remain reserved;
  union type *declarations* remain not planned.

Releases before 0.23.0 predate this changelog and are not retroactively
documented.

## [0.49.0] - 2026-08-11

### Added

- **`Result<T, E>` annotations enforce their payloads.** Gradual
  typing stage 4: a Result holds exactly one payload, so — unlike
  containers — the O(1) boundary discipline allows a step more. An
  `Ok` value now checks its payload against `T` and an `Err` against
  `E`, shallowly (one name comparison; `Result<List<Int>, E>` checks
  an Ok payload is "a List"), at every annotated boundary on every
  tier with identical text: `return value of parse expects
  Result<Int, String>, got Ok(String)`. Functions returning annotated
  Results stay on the enforcing bytecode path (the JIT refuses,
  conservatively). The static checker goes deeper: `Ok`/`Err`
  literals decompose with paths (`Ok payload of parameter 'r' of f
  expects Int, got String`), `expr?` carries the Ok payload's type,
  `match` arms narrow `Ok(v)`/`Err(e)` bindings to their payload
  types, and deep Result types flow through annotated bindings
  (`expects Result<Int, String>, got Result<String, String>`). Every
  Result annotation in the repo's 171 files was already honest:
  nothing changed behavior, and dishonest ones now cannot land.

### Fixed

- **The website playground linked again — and its book nav caught up.**
  The playground worker's import object predated the `dom` module, so
  any freshly built wasm failed to instantiate
  (`LinkError: ... "host_dom_fetch": function import requires a
  callable`), taking the whole playground — most visibly the data
  stack — down with it. The worker now supplies the nine `dom` host
  imports as inert sandbox stubs (`dom.query` finds nothing, reads
  yield empty strings, writes are no-ops — there is no document in the
  worker), and ods/stats/Frame programs verified running in the
  browser at 0.48.0. The site's hand-maintained chapter registry also
  still listed the deleted design docs and lacked the new chapters; it
  now mirrors the book index (Types, Editors, and Tooling pages added,
  design entries gone).

### Documentation

- **The book gained a Types chapter** (`docs/types.md`) — gradual
  typing end to end: annotations as enforced promises, the three
  rules, exactly where the runtime enforces (every tier, async
  resolution, precomputed checks), what the static checker proves and
  its two diagnostic labels, element types, and an adoption playbook.
  Its examples are doc-tested like every other chapter.
- **The design documents folded into the book.** `docs/design/ods.md`
  and `docs/design/ods-lazy.md` are gone as separate files; their
  durable content — the problem statement, the one-array-both-tiers
  decision, the benchmarks-are-the-spec tables with every recorded
  revision, and the lazy-evaluation verdict with its reopening gate —
  now lives in the Data Stack chapter as "The design record" and "Why
  eager evaluation". The book index lists chapters, not design docs.
- **Stale claims corrected**: Stability no longer promises a "future
  opt-in static checker" (enforcement and the checker shipped in
  0.48.0 and are now documented as the contract); Internals no longer
  advertises the dead `type_checker.rs` module and now maps
  `tools/check.rs`; the root README's struct bullet ("values are
  dynamic") predated field enforcement and now states it, with gradual
  typing added to the feature list; the Tooling chapter and book index
  now name all three `olang` tools.

### Added

- **`olang check` sees element types.** The static checker now goes
  where the runtime's shallow checks deliberately don't: literals
  decompose against their annotations element by element, so
  `let xs: List<Int> = [1, "a", 3]` reports `element 1 of let binding
  'xs' expects Int, got String`, with the path spelled out through
  nesting (`element 1 of element 1 of ...`), `Map<K, V>` keys and
  values, tuple arity and elements, and struct-literal fields against
  their declared field types. Deep types flow through annotated
  bindings and known return types, so passing a `List<String>` binding
  where `List<Int>` is declared is flagged too. Findings are labeled
  by kind: base-level violations say "this would fail at runtime" and
  keep the runtime's exact error text; element-level breaks — which
  the O(1) runtime checks let pass — say "the annotation's promise is
  broken here". The no-false-positive discipline is unchanged
  (anything unprovable is silent; the repo's 171 files still check
  clean), and pattern bindings (loops, lambdas, match arms, catch)
  now correctly shadow outer annotated names during analysis.

## [0.48.0] - 2026-08-10

### Added

- **`olang check` — the static side of gradual typing.** A new
  subcommand (and LSP integration) that reports type-annotation
  violations the runtime would provably reject, before the program
  runs: a literal argument against an annotated parameter, an
  annotated `let` initialized with a known-type value, a declared
  return contradicted by what the body provably produces, and
  wrong-arity calls to known functions. Its discipline is no false
  positives — anything the checker cannot prove stays silent, and
  unannotated dynamic code is never judged (the whole repo's 171 `.ol`
  files check clean). Diagnostics carry the runtime's exact error
  text and render miette-located with the source line and caret;
  non-zero exit on any finding makes it CI-ready. The same checker
  feeds the language server, so editors surface these as error
  squiggles while you type.

### Changed

- **BREAKING: type annotations are enforced at runtime — olang is
  gradually typed.** An annotation, wherever it appears, is now a kept
  promise: function parameters check at the call boundary
  (`parameter 'x' of f expects Int, got String`), declared return
  types check on the produced value, and `let x: Int = ...` checks at
  the binding. Unannotated code stays fully dynamic with zero cost and
  zero judgment. Containers check shallowly (`List<Int>` promises "a
  List"; element types are the future static checker's concern),
  Int/Float is strict (no widening, matching struct-field
  enforcement), and generic parameters are erased, never checked. An
  async function's `-> Promise<T, E>` unwraps: the check enforces `T`
  on the value the body resolves to.
  Enforcement is identical on every tier: checks are precomputed at
  declaration, the interpreter enforces at its boundary, the bytecode
  VM enforces at function entry and return, and the JIT statically
  discharges return annotations (compiling only when the inferred
  return kind provably satisfies the declaration — otherwise the
  function stays on the enforcing bytecode path). Every annotated
  example in the repo already passed; code with dishonest annotations
  now fails with a clear, located error.

## [0.47.0] - 2026-08-11

### Added

- **Runtime errors point at source.** Every statement now carries its
  source position, and when a runtime error surfaces the file runner
  renders the offending line with a caret (via miette) plus the
  interpreter call stack — `Undefined variable: x` now comes with
  `[file.ol:3:5]`, the source line, and `mid → deep`. Positions are a
  side channel: error messages themselves are unchanged, so tier
  behavior and error-matching programs are unaffected. Statement
  equality is span-insensitive (position is metadata, not identity),
  which keeps `olang fmt`'s AST-verification gate sound.

- **Did-you-mean in file mode.** An undefined name ranks every visible
  binding by edit distance and offers the closest matches as a help
  line under the located error — previously REPL-only, now where it
  matters most.

### Fixed

- **Tier error text matches the interpreter word-for-word.** The
  bytecode tier's private wordings are gone: pattern-match, arity
  (naming the first missing parameter), binary/unary type errors —
  which now also carry context on both tiers ("cannot apply '+' to Int
  and Bool"), with immediate-flipped operands reported in source
  order.

### Changed

- **Parse errors speak plain English.** The raw pest rendering
  ("Pest parsing error:", `mul_op`, `base_pattern`) is gone: expected
  lists collapse to human phrases, every syntax error carries
  line/column and a caret snippet, parse errors inside imported modules
  render the same way (previously a raw Rust Debug dump), and UTF-8
  BOMs are stripped instead of failing at 1:1 on an invisible
  character.

## [0.46.0] - 2026-08-10

### Removed

- **Internal cleanup: dead code and unused dependencies (no user-visible
  behavior change).** Removed 26 bytecode `Instruction` variants the
  compiler never emitted (list/string/tuple/pipeline/thunk/exception/
  memory/profiling/debug scaffolding) along with their executor and JIT
  arms; the inert persistent module-cache layer, which never read or
  wrote anything (the in-memory cache with content-hash invalidation is
  unchanged); and unused dependencies (`memmap2` and the
  `arbitrary`/`fake`/`quickcheck`/`lazy_static` dev-dependencies).

### Changed

- **Struct field type annotations are now enforced at construction
  (breaking).** Constructing a declared struct with a field value whose
  runtime type does not match the field's declared annotation is now a type
  error (e.g. `field 'x' of Point expects Int, got String`), where it was
  previously accepted — annotations were advisory. Enforcement covers the
  annotations the runtime can reliably check: `Int`, `Float`, `Bool`,
  `String`, and declared struct/enum type names. The match is exact — an
  `Int` value does not satisfy a `Float` field (no widening at
  construction). Fields whose annotation cannot be reliably checked (a
  generic type parameter, a list/map, a function type) stay dynamic, as do
  all fields of an anonymous `{ ... }` object. The behavior is identical
  across all three tiers (interpreter, bytecode, and JIT) and pinned by
  differential regression tests.

- **Mixing a number and a string under `+` is now a type error, not a
  silent coercion (breaking).** `"count: " + 5` and `1 + "x"` previously
  stringified the number and concatenated; they now raise a type error
  (`cannot add String and Int; use to_string(...) to convert`),
  Python-3 style. Convert the number explicitly with `to_string(...)`
  (or `show(...)`) first. String+string concatenation and
  number+number arithmetic are unchanged. The behavior is identical
  across all three tiers (interpreter, bytecode, and JIT) and pinned by
  differential regression tests.

## [0.45.0] - 2026-08-10

### Changed

- **`random.seed(n)` reproduces within a version, not across this
  upgrade.** rand 0.8 → 0.10 (with rand_distr 0.6): seeded streams are
  not part of rand's cross-major stability contract, so a program that
  recorded exact outputs under `random.seed(n)` on 0.44 may see
  different draws now. Measured specifics of this particular upgrade:
  the raw draw stream is unchanged in practice (`random.random`,
  `uniform`, `gauss`, `randint`, and the `randstr` family produced
  byte-identical seeded sequences before and after), but
  `random.shuffle`, `random.choice`, `random.choices`, and
  `random.sample` order differently — rand 0.9 changed uniform integer
  index sampling. Treat any exact seeded sequence as reproducible only
  within a single olang version; assert properties, not pinned streams
  (the repo's own tests and examples/data-processing/statlab already do). Seeding,
  determinism within a run, and `stats.norm.sample` riding the same
  stream are all unchanged.
- **A maintained HTTP stack under the `http` client.** reqwest
  0.11 → 0.13 moves `http.get`/`post`/`put`/`delete`/`request` onto
  hyper 1 and a current TLS stack; no olang-visible API change
  (verified end to end against a live server and over https).
  `http.serve` is hand-rolled TCP and is untouched. The direct `hyper`
  and `tokio` dependencies existed only for this stack and were unused
  in the source — both deleted; reqwest brings its own runtime.
- **Entropy plumbing current, and bcrypt catches up.** getrandom
  0.2 → 0.4 for the RNG stack: the wasm playground's custom entropy
  backend migrates from the deleted `register_custom_getrandom!` macro
  to 0.4's `getrandom_backend="custom"` cfg (set for wasm32 builds in
  `.cargo/config.toml`) with the hook in src/playground.rs; the
  `host_random_bytes` page import is unchanged. That unblocks bcrypt
  0.15 → 0.19 (skipped in the M2 batch precisely because it dragged
  getrandom 0.4 into wasm), so `crypto.hash_password` gets four majors
  of maintenance; existing hashes still verify. The rand_core 0.6 era
  crypto crates (rsa, aes-gcm, argon2) keep getrandom 0.2's registered
  backend alongside — both paths verified in the playground harness. Cranelift 0.121 → 0.134 puts
  13 releases of instruction-selection and aarch64 codegen work under
  every JIT'd function with zero olang-side semantic change: the N-body
  benchmark drops 48 ms → 26 ms (best of 3, same machine, byte-identical
  momentum-conservation output); fib(30) holds at 4 ms. The full JIT
  parity suite and the differential tests pin the tier's bit-identical
  contract across the upgrade.
- **A years-newer SQLite under the `db` module.** rusqlite 0.31 → 0.40
  brings its bundled SQLite engine forward several years of upstream
  releases — query-planner, correctness, and performance work land under
  every `db.*` call with no API change on the olang side.
- **Prettier crash reports.** miette 5 → 7 brings its reworked
  graphical reporter to the CLI's panic hook — internal errors render
  with cleaner layout and labels instead of a raw Rust backtrace.
- **A better REPL.** rustyline 13 → 18 carries five majors of
  line-editing fixes — more robust history handling, completion, and
  terminal behavior in `olang repl`.
- **Faster regex and CSV paths.** regex 1.10 → 1.13 and csv 1.3 → 1.4
  pick up upstream performance work behind the `re` and `csv` modules;
  pest, serde, tokio, rayon, clap, base64, and thiserror move to their
  current releases in the same batch. No olang-visible API changes.

## [0.44.0] - 2026-08-10

### Added

- **`otc pkg update`, and `install` finally honors the lock.** A
  lockfile that still covers `olang.toml` is now replayed exactly:
  pinned revs fetched (offline once cached), lock left byte-identical —
  so repeated installs and every `olang` run (which installs
  implicitly) stopped re-resolving branch deps and rewriting the lock
  on each invocation. The lock records which tag/branch a git pin came
  from, so repointing a dependency in the manifest still re-resolves on
  a plain `install`; moving a branch dep to its new upstream head is
  the explicit `otc pkg update`. Fixed along the way: the bare git
  mirrors in `~/.olang/cache` were cloned without a fetch refspec, so
  the "refresh refs" fetch had never actually updated a branch ref.

- **`otc new --lib`.** Scaffolds a library package the resolver can
  actually consume: `index.ol` at the package root (where `use <name>`
  looks) with a `share`d function and a passing test block, plus a
  README showing how a depending package adds and imports it.

- **The tracker's frontend catches up with its backend.** Still plain
  HTML + JS with zero dependencies, now surfacing the whole API:
  server-driven search/filter/sort/paging with the view state mirrored
  into the URL hash (refresh and share keep the filters, shown as
  clearable chips), a comments drawer with avatars, relative times, and
  ⌘-Enter submit, a stats dashboard whose CSS bars use the same status
  colors as the grid pills (color follows the entity) plus the ods
  point quantiles, a live activity feed, CSV export and one-click
  backups, dark mode (auto-detected, toggleable, remembered), keyboard
  shortcuts (`/`, `n`, `Esc`), a full inline create row, two-step
  delete, field-by-field 422 toasts, and a 401 that prompts once for
  the bearer token. The list endpoint now carries each issue's comment
  count (one subquery — no n+1), locked into the contract test.

- **`examples/web/app` — the tracker grows a real backend.** The
  full-stack issue tracker now runs on a persistent, schema-migrated
  SQLite store (a `schema_version` table; migrations append, run once,
  in transactions) with request validation (422s naming each field
  problem), filtered/paginated listing (`?status= assignee= q= sort=
  order= limit= offset=` returning `{items, total, ...}`), comments
  with transactional cascade delete, an audit trail of every mutation
  (`/api/activity`), stats blending SQL rollups with ods point
  quantiles, CSV export, JSON backups, and optional bearer-token auth
  for writes (`TRACKER_TOKEN`). The router became a middleware layer:
  per-request ids and timing logs, one JSON error envelope everywhere,
  method-aware 405s with `Allow`, and HEAD riding GET so probes see
  200. The whole contract is locked by `tests/tracker_app_test.rs`,
  which boots the actual app on an ephemeral port and drives it over
  the wire — validation shapes, filter fallbacks, 404/405/400/401
  semantics, the audit sequence, cascade deletes, and auth gating.

- **`os.read_line()` — stdin, at last.** One line from stdin as
  `Ok(line)` (newline stripped), `Err("eof")` when the stream ends: the
  missing primitive for prompts, REPLs, and shells, and for reading
  piped input line by line.

- **`examples/tools/oshell/` — a Unix-like shell written in olang.** An
  interactive `os.read_line` loop where every operation is the stdlib:
  pipelines thread stdout→stdin through `os.exec`, redirection
  (`< > >>`) and globbing ride on `fs`, `grep` is `re`, and 25 builtins
  (`cd ls cat head tail grep wc mkdir rm cp mv touch env export alias
  history which type ...`) shell out to nothing. Quoting, `$VAR`/`~`/`$?`
  expansion, `;`/`&&`/`||` with real short-circuit semantics, exit-code
  propagation (127 for not-found, `exit N` as the process code), and
  history persisted across sessions. The long-running systems-work
  proof: 1,000 mixed commands — 250 child processes among them — soak
  through one session in ~1.4 s with state consistent throughout.

- **`olang lsp` — the language server, in the same binary.** Speaks LSP
  over stdio: diagnostics as you type (parse errors with the parser's
  own line/column; analyzer warnings such as unused variables at their
  declaration sites), completions (keywords, global builtins, the 19
  stdlib modules, and fn/type/let names from the open file), and
  whole-document formatting through the olang fmt engine. Stateless by
  design — every edit re-parses whole files. Tested at the protocol
  level: tests/lsp_test.rs drives the real binary over stdio through
  the complete loop, including clean shutdown.
- **Declaration spans in the AST; hover and go-to-definition in the
  server.** `fn`, `let`, and `type` declarations now carry the 1-based
  source position of the name they bind (serde-defaulted, so cached
  ASTs stay readable; synthetic/desugared declarations carry None).
  On top of them the language server gains hover (the declaration's
  rendered signature) and go-to-definition, and unused-variable
  warnings move from text-search positions to exact declaration spans.
  Protocol test extended: hover content and definition target are
  asserted to the character.
- **VS Code extension (editors/vscode/).** TextMate grammar (par for,
  pipelines, template strings, module names), bracket/indent config,
  and a thin client launching `olang lsp`; `olang.serverPath` setting
  for custom binary locations. The book gains an Editors chapter with
  Neovim wiring included.

- **The tracker runs on olang end to end — page shim, dom.fetch, and
  the JS frontend deleted.** examples/web/app now serves ONE frontend:
  app.ol, running in the browser as WebAssembly through the page shim
  (static/olang-dom.js — real DOM host imports, session boot, event
  and fetch-response dispatch). dom.fetch(method, path, body, cb)
  bridges browser HTTP with responses delivered to 1-argument olang
  closures via the new olang_dispatch_event_with; http.serve gains
  body_file responses (raw bytes from disk) so the olang backend can
  serve its own wasm binary. The frontend is deliberately stateless —
  olang closures capture by value (spawn semantics), so state flows
  down through arguments and the DOM itself, which the first draft
  learned the hard way: a handler's write to module-level mut state
  vanished into its own environment (parsed 14, rendered 0). Verified
  in a real browser: render from the API, add, status-advance (PATCH),
  and delete all round-trip through olang closures; the harness covers
  the fetch-payload path headlessly.

- **The `dom` module — olang as a frontend language.** A new stdlib
  module (query, get/set_text, set_html, value/set_value, on) whose
  operations cross the wasm boundary as host imports the page
  implements; elements are opaque handles. The playground boundary
  gains a persistent session: olang_session_start runs a program and
  keeps its interpreter alive, dom.on registers olang functions in a
  handler registry, and olang_dispatch_event re-enters the live
  interpreter per event — click handlers are ordinary olang closures.
  Native builds error clearly ("only available in the browser").
  Proven end-to-end by playground/dom_harness.mjs: a fake DOM over the
  host imports, an olang counter program, two dispatched clicks, and
  asserted mutations (run: bun playground/dom_harness.mjs
  target/wasm32-unknown-unknown/release/olang_playground.wasm).

### Fixed

- **`otc unused` stopped flagging live code.** Usage now counts
  wildcard and bare imports (`use m { * }` / `use m` marks every shared
  function of `m` used) and namespace references (`m.f(...)`) — not
  just `use m { f }`. Over-counting is the deliberate direction: a
  linter that cries wolf gets ignored.

- `print` now flushes stdout, so a partial line — a shell prompt, a
  progress indicator — appears immediately instead of waiting for the
  next newline.

## [0.43.0] - 2026-08-09

### Added

- **`par for` — parallel iteration as a language construct; P1
  closes.** `par for x in xs { body }` (tuple bindings included) fans
  iterations across OS worker threads with an implicit barrier — one
  interpreter, tier and JIT included, per worker. Spawn-style snapshot
  semantics; first-sequential-error reporting; break/return cannot
  cross the parallel boundary; `par` is not a reserved word (it only
  means something before `for`, pinned by test). Iterates lists,
  ranges, and strings. The bytecode compiler refuses it (fail-closed,
  interpreter-owned); `olang fmt` handles it. With this the
  performance campaign's three levers are all resolved.

## [0.42.0] - 2026-08-09

### Added

- **JIT strings — the lane closes.** String parameters, struct fields,
  and constants enter native code as borrowed pointers (a constant's
  Arc lives in the bytecode the JittedFn owns, so its pointer bakes
  into the code). Equality and lexicographic ordering run through a
  helper executing the VM's own comparison operators; concat is an
  allocation and follows the exact struct discipline — scratch-owned,
  straight-line only, ownership transferred once at the entry boundary
  by a string retain twin, loops driving native concat from bytecode.
  Mixed string/number `+` (formatting) stays on bytecode. Also fixed
  the same first-pass monotonicity trap for `+` operands that Return
  had: unresolved operands defer instead of narrowing irreversibly.
  59 JIT parity tests; tier output byte-identical on string kernels.
- **JIT struct construction — allocation with sound ownership.**
  MakeStruct compiles natively through a scratch-context model: every
  struct a native call builds is owned by a VM-side list for exactly
  that call, so a deopt at any point can never leak or dangle; struct
  returns transfer ownership once, at the entry boundary, where a
  retain helper resolves the borrowed pointer to an owned Arc (scratch
  allocation or entry argument — anything else deopts). A context
  pointer now threads through the whole native ABI. Two deliberate
  refusal rules keep the model where it wins, learned by measurement:
  an unbounded native loop of allocations held memory until call end
  and a cap-triggered mid-loop deopt cost more than never compiling —
  so MakeStruct compiles only in straight-line code (constructors),
  loops with struct-returning callees stay on bytecode and drive
  native constructors call by call. Also fixed a first-pass
  monotonicity bug where Return narrowed a not-yet-resolved register's
  allowed-set irreversibly. Constructor-driving kernel: 88 -> 81ms;
  55 JIT parity tests including returning-a-parameter, mixed-field
  constructors, and the allocating-loop refusal.

## [0.41.0] - 2026-08-09

### Added

- **JIT tuple extraction — multi-value native returns.** Functions
  returning tuples (up to 4 scalar elements) compile natively: MakeTuple
  becomes per-element SSA variables, Return becomes a multi-value native
  return (N elements + status), destructuring callers receive elements
  directly in registers (PatternTestTuple is statically proven and
  folds to true; ExtractElement/TupleGet read the element variables),
  and the entry wrapper writes one out-slot per element. Deopt unwinds
  through tuple producers exactly as scalars — a division by zero deep
  in a tuple-returning callee still yields the VM's canonical error.
  N-body: 56 -> 50ms, with force-style tuple producers and their
  destructuring consumers both native. 51 JIT parity tests.
- **JIT heap values: list indexing and `for` iteration — N-body
  compiles whole (332 -> 56ms).** Lists pass into native code as
  borrowed pointers, classified at specialization by element kind
  (float, int, or one struct shape); `xs[i]` and the `for`-loop
  instructions (IterLen/IterGet) compile through guarded host helpers
  that reproduce the VM's exact semantics — subscripts wrap negative
  indices, iteration does not, bounds violations deopt to the
  bytecode's canonical error. Struct elements return borrowed pointers
  into the list's own storage, valid for the synchronous call by the
  same argument as struct parameters. The register-path call site now
  extracts struct and list arguments too (it previously only handled
  int/float, which kept bytecode-to-bytecode calls off the JIT). The
  campaign's original acceptance target — N-body from 400ms to tens of
  milliseconds — is closed: 56ms, momentum conservation byte-identical.
  47 JIT parity tests including negative indexing, mixed-kind lists
  (refused, agreeing), and for-loops over struct lists.
- **JIT: the float-math builtins join the whitelist.** All 25
  `math.*` float builtins compile in JIT functions: sqrt, floor, ceil,
  and trunc as native IEEE instructions (bit-exact by definition), the
  other 21 through an imported helper that calls the VM's own
  eval_float_math — exactness by construction, not by reimplementation.
  Inference types them totally (numeric in, Float out, never deopts).
  A synthetic sqrt/sin/cos/pow kernel runs bit-identically and ~20%
  faster even when driven from bytecode. Measuring N-body exposed the
  true remaining blocker in its force loops: list indexing and tuple
  extraction ("other" in the refusal listings) — recorded on the
  roadmap as the heap-values rung. 42 JIT parity tests green.

## [0.40.0] - 2026-08-09

### Added

- **ods is part of the language — no flag, no import, no setup.** The
  `ods` cargo feature is gone: the data stack (Series, Frames, stats,
  plot) compiles into every build unconditionally, including the wasm
  playground — `ods.series([...])` works in the browser sandbox exactly
  as it does natively. Verified across every boundary this release
  built: Series round-trip the bytecode tier byte-identically, ride
  par_map worker threads as shared Arcs, and coexist with the JIT in
  the same program.


- **The `plot` namespace — ods Phase 4, charts as SVG text**
  (`docs/design/ods.md`). `plot.line`, `plot.scatter`, `plot.lines`
  (multi-series with legend), `plot.bar`, and `plot.hist` render
  complete standalone SVG documents from Series data — no rendering
  dependency, composing identically with `fs.write_file`, an `http`
  response, or the wasm playground. Defaults carry real charting
  discipline: a CVD-validated categorical palette assigned in fixed
  order (a 9th series errors rather than inventing a hue), 1-2-5 nice
  ticks, recessive grid and axes, ink-colored text, rounded data-ends
  anchored to the baseline, legends only at two or more series,
  XML-escaped labels, one y-axis always. Options ride in one map
  (`title`, `x_label`, `y_label`, `width`, `height`) and unknown keys
  refuse. Null pairs drop in xy charts; bars refuse null values. With
  this, every phase of the ods design doc is shipped: CSV → Frame →
  group_by → chart is one pipeline. Lazy evaluation was Phase 4's
  other mandate: evaluated and **deferred with a measured reopening
  gate** in the new `docs/design/ods-lazy.md` — the eager engine
  already beats NumPy on the benchmark fusion would improve.

- **Frame — ods Phase 3, the columnar table** (`docs/design/ods.md`).
  Series grew a String dtype (lexicographic comparisons, sort, gather,
  fill_null), and the `ods` namespace grew the tidyverse verb set over
  a new Frame type: `frame`, `read_csv` (type-inferring), 
  `frame_from_records` (accepts `json.parse` output — JSON arrays
  become Frames in one pipe), `select`, `with_column`, `filter`/`take`
  (shared with Series, dispatched by argument type), `sort_by`,
  `head`, `group_by` with count/sum/mean/min/max aggregations, inner
  and left hash `join`, `to_records`, and introspection. Null keys
  form their own group in `group_by` (R/Polars) but never match in
  joins (SQL); groups keep first-seen order. **B6 gate met: a 10M-row,
  1k-group sum+mean aggregates in 27.2 ms single-threaded vs 24.0 ms
  for Polars on 18 threads** — an inline Fx hasher and L1-resident
  accumulators, measured table in the design doc. `examples/data-processing/dataproc`
  is rewritten on the Frame pipeline: **3.0 s → 0.06 s at 200k rows
  (50×)** with identical aggregates.

- **The `stats` namespace — ods Phase 2, statistical inference**
  (`docs/design/ods.md`). Distributions (normal, t, chi², F — pdf/cdf/
  ppf/sample, statrs-backed), `stats.describe`, pairwise-complete
  correlation and covariance, one-sample and Welch t-tests (the second
  argument picks the test: Series → two-sample, number → null mean),
  chi² goodness of fit, and `stats.lm` — OLS with coefficients, SE,
  t statistics, p-values, R², and R-style `na.omit` row handling,
  returned as an olang map of parallel Series. Sampling draws from the
  `random` module's stream, so `random.seed(k)` makes
  `stats.norm.sample(...)` reproducible. Every statistic is pinned
  against scipy/NumPy reference constants in tests, and **the B4 gate
  is met: a 1M×20 OLS fits in 36.4 ms parallel (90 ms sequential) vs
  137.8 ms for `numpy.linalg.lstsq`** — measured table and the
  faer-deferral rationale recorded in the design doc.

- **Series — ods Phase 1, the numerical engine** (`docs/design/ods.md`).
  A typed, null-aware 1-D array backed by the new `olang-ods` workspace
  crate: pure Rust kernels with no olang dependency, contiguous
  Arc-shared copy-on-write buffers, validity bitmaps, and rayon
  parallelism that respects the runtime's configured threshold. The
  `ods` namespace grows 24 functions — constructors (`series` from
  lists/ranges, `zeros`, `linspace`), null-skipping reductions (`sum
  mean var std min max quantile`), `sort argsort take filter cumsum
  dot`, and null tools — and operators are intercepted in both tiers:
  `s * 2.0 + 1.0` runs vectorized kernels, `s > 2` yields a Bool-series
  mask, `10.0 - s` broadcasts. Integer kernels are checked and error
  with the interpreter's exact wording; `==` between Series stays
  structural like every olang collection (`ods.eq`/`ods.ne` give
  elementwise masks). **Measured against NumPy at 10M elements: sum at
  parity sequentially (1.01ms vs 1.12ms) and 2.7× ahead in parallel,
  std 2.5× ahead, sort 13× ahead, null-aware mean 1.7× ahead of
  nanmean** — full table in the design doc. Pinned by 22 engine
  property tests against naive references plus 12 tier-transparency
  integration tests (tests/ods_series_test.rs).

- **JIT struct field access — the redirected P3 effort, on the safe
  Arc model.** Struct arguments pass into native code as *borrowed*
  pointers (JIT calls are synchronous; the caller's slot outlives the
  call — no refcount is ever touched), specialized per interned shape
  with field indices resolved at compile time. Every read goes through
  one guarded host helper that deopts on any surprise (same-shape
  instances may carry different field kinds in a dynamic language), so
  no layout assumption and no unsafe discipline leaks into the VM.
  Shape specs are only gathered when specializing; Ready calls extract
  a pointer and a shape id, nothing more. **A struct-field kernel:
  143ms -> 18ms (8x)**; N-body 350 -> 322ms (its inner kernel awaits
  math.sqrt in the whitelist). Four new parity tests: field kernels,
  mixed Int/Float/Bool fields, same-shape/different-kind deopt, and
  structs through native call chains.
- **P3 verdict: full NaN-boxing deferred, on the probe's own evidence.**
  The probe existed to price the rewrite, and it did: halving value
  size moved the most value-bound workload 13%, so halving again
  projects roughly another 10% — not the 2-3x the roadmap estimated
  from the old 32-byte baseline. Against that stands the cost: packing
  heap pointers means manual refcounting at every register move, the
  exact raw-pointer GcPtr scheme this codebase measured, found leaking
  every allocation, and deleted in 0.23. A ~10% win does not buy back
  that risk. The nanbox primitives remain proven and ready should the
  JIT's register model want them; the performance effort redirects to
  JIT struct field access on the safe Arc model — the actual N-body
  blocker. Recorded so the rung is not re-attempted without new data,
  like the compare+branch fusion before it.
- **NaN-boxing foundation (src/ovm/nanbox.rs).** The 8-byte packed
  value scheme for P3 proper, landed as primitives-with-proofs before
  any VM wiring: floats bit-exact with real NaNs canonicalized so no
  arithmetic result can alias a tag; 48-bit small integers ([-2^47,
  2^47) inline, wider admits NeedsHeap — settling upfront that olang's
  full-i64 integers split); 48-bit pointers; bool/unit constants.
  Pinned by boundary tests (±2^47, i64::MIN/MAX, -0.0, ±inf, payload
  NaNs including a tagged-int bit pattern) and a million-iteration
  randomized decode cross-check. Deliberately not wired: the value-model
  rewrite builds on these next, with the bit-level subtleties already
  settled where they were cheap.
- **Value-model slimming: OvmValue 32 -> 16 bytes — the P3 probe.** The
  roadmap prescribed removing the per-value header first as a cheap
  probe before NaN-boxing, and the probe paid: the ValueHeader (type
  tag, tier, lazy state) was fully dead — the tag derivable from the
  data, the tier never read, and no constructor ever produced a lazy
  value — yet its 8 bytes copied on every register move. With it gone,
  the lazy-forcing scaffolding went too, the two-slot Result variant
  packed behind one Arc, and (in the ods workstream, coordinated) the
  NativeHandle went thin, landing OvmValue at exactly 16 bytes, pinned
  by a size test. **N-body: 400ms -> ~350ms (-13%)** purely from
  layout; every differential suite byte-identical. The remaining rung
  is NaN-boxing proper (16 -> 8), now with measured grounds.
- **JIT call-graph groups: cross-function native calls.** The
  self-call-only restriction is gone. On a function's first call the
  JIT plans every function reachable through its CallFn sites, runs
  kind inference to a global fixpoint across the group (callee return
  masks feed caller registers; masks only grow, so it converges), and
  compiles the whole group with direct native-to-native calls —
  helpers, chains, and mutual recursion all stay native, each member
  entry-guarded on its own specialized signature and directly callable
  from the VM afterwards. Kind-mismatched or already-differently-
  specialized callees refuse the entry (fail-closed); the depth budget
  and deopt status propagate through the whole chain, so an overflow
  two native calls deep unwinds and re-runs on bytecode with the
  canonical error. **fib(30) split across two mutually recursive
  functions: 94ms → 5ms** — identical to single-function fib. Eight new
  parity tests: helper pipelines, 2- and 3-function cycles, mixed-kind
  chains, deopt-in-chain, kind-mismatched helpers, and depth guards
  through mutual recursion.
- **JIT type specialization: floats.** Compilation is now lazy and
  runtime-observed: a whitelisted function compiles on its first call,
  specialized to the Int/Float argument kinds that call carries, with
  the native entry guarding on exactly that signature — any other shape
  runs on bytecode (one specialization per function). Registers are
  typed i64 or f64 by the same fixpoint inference, mixed int/float
  arithmetic promotes the integer side exactly as the VM does, float
  division by zero deopts (olang errors there, not inf), NaN and IEEE
  overflow-to-inf behave identically to the VM, and float modulo is
  declined outright (fmod has no exact IR equivalent). One inference
  subtlety earned its comment: liveness flows backwards through
  register copies, or `zr = zr2` kernels misclassify their sources as
  dead. Float orbit kernels measure ~4.5× over bytecode; eleven new
  parity tests cover the float guard edges, polymorphic call sites
  (int-then-float and float-then-int), and fmod refusal.
- **The baseline JIT — the performance campaign's P2.** Hot bytecode
  compiles to native machine code via Cranelift (`src/ovm/jit.rs`),
  extending the correctness ladder unchanged: "can't compile
  identically → stay interpreted" gained "can't compile natively → stay
  on bytecode". A function qualifies when every instruction is in a
  pure integer/boolean whitelist, with operand kinds proven by a
  fixpoint inference (registers may hold mixed kinds only if nothing
  reads them — `if`-statement result slots taught that rule the hard
  way). Purity makes deopt trivial and total: non-integer arguments,
  overflow, division by zero, `i64::MIN` edges, and depth exhaustion
  all abandon the native run and re-execute on bytecode, which owns
  every error message; recursion carries a depth budget clamped to the
  VM's own limit, so runaway recursion errors identically instead of
  smashing the native stack. **fib(30): 89ms → 5ms (18×) — level with
  Node and Bun**; integer loop kernels 20–30×; the 1M-element pipeline
  27ms → 13ms. Pinned by tests/jit_test.rs (17 guard-edge parity tests)
  on top of the existing 150-test tier suite, which now runs everything
  through the JIT as well.

- **OVM modules and native values — ods Phase 0**
  (`docs/design/ods.md`). The OVM grew a module system: a Rust
  component registers stdlib-style namespaces, native value types, and
  operator behavior into *both* execution tiers through one registry
  (`src/native.rs`). Native values cross the tier boundary as one
  shared Arc — a refcount bump, never a conversion — so the
  lossy-round-trip failure mode that once kept maps and enums off the
  tier is unrepresentable for them (pinned by an Arc-identity test).
  First module: `ods` (feature `ods`, default on, pure Rust so the
  playground can enable it), registering `ods.version()` and the seam
  probes that pin the plumbing (`tests/ods_module_test.rs`). The
  numerical engine itself is Phase 1.

### Fixed

- **Two tier-vs-interpreter error divergences** the JIT parity suite
  exposed (both present in released 0.39): tiered runtime errors
  carried a doubled "Runtime error:" prefix (the tier re-wrapped an
  already-prefixed message), and `%` by zero said "Division by zero" on
  the tier where the interpreter says "Modulo by zero" — the VM now has
  a distinct ModuloByZero error with the interpreter's exact words.

### Changed

- **`otc` slimmed to the commands that earn their keep.** The toolchain
  had drifted badly behind the language: `otc run` executed files
  *without* package resolution (so any project with dependencies
  failed), `otc new` scaffolded the pre-`olang::pkg` manifest format
  the current toolchain can't read, and a whole legacy "git package
  system" (`install`/`list`/`update`/`remove`/`clean`/`info`/`config`/
  `cache`/`self`/`doctor`/`system-info`) coexisted with — and
  contradicted — `otc pkg`. otc is now six commands, each verified
  end-to-end: `new` (scaffolds the current `[package]` manifest, a
  parse-validated `src/main.ol` with a `test` block, README pointing at
  the `olang` binary), `pkg` (the real package manager: init/add/
  remove/install/tree/publish), `check`, `deps`, `unused`, and `ovm`
  (the tier-vs-interpreter divergence harness the OVM docs prescribe).

### Removed

- **`otc run` / `repl` / `test` / `build` / `version`** — running code
  is the `olang` binary's job (`olang file.ol`, `olang`, `olang test`),
  and `build` only ever copied source files into `build/`.
- **The legacy git package system** (~2,300 lines: `git_package.rs`,
  `global.rs`, `lock_file.rs`, `config.rs` and their commands),
  superseded by `otc pkg` + `olang.lock`.
- **The text-based refactor commands** (`move-fn`, `rename-fn`,
  `extract-file`, `merge-files`, `fix-imports`) and the cosmetic
  `tree` / `organize` — string-surgery on source trees predating the
  current grammar.

### Added

- **`par_map` / `par_filter` — the performance campaign's P1.** The
  parallel twins of `map` and `filter`: same arguments, same results in
  the same order, but fanned out across OS threads with no GIL. The
  design is the one the roadmap prescribed — `map` originally went
  sequential because cloning the interpreter per *element* was ruinous;
  par_map clones per *worker* (im-map environments make that cheap),
  gives each worker its own bytecode tier, and splits the list into
  contiguous chunks under `std::thread::scope`. Semantics are pinned by
  a 15-test differential suite: spawn-style snapshot isolation (the
  function never mutates the caller's environment — on any machine,
  including the single-core fallback), first-in-order error reporting
  (the error you get is the one `map` would have hit first), filter's
  keep-on-`true` rule, and fail-closed refusal in the VM (functions
  calling par_map stay interpreted). **Measured: 9–13× vs sequential
  `map` on compute-heavy kernels** (examples/concurrency/parmap, which self-checks
  parallel == sequential on every run).

### Fixed

- **`spawn` threads now carry the bytecode tier.** `thread_safe_clone`
  set `bytecode_tier: None`, so every spawn worker (and http.serve
  handler thread) silently ran the pure tree-walker — losing the tier's
  ~10× on compute. Clones now get a fresh, quiet tier with the parent's
  promotion policy, and the parent's declaration knowledge (functions,
  struct shapes, trait impls/defaults, unit variants) is replayed into
  it, since workers never re-evaluate the declarations themselves.

## [0.39.0] - 2026-08-09

### Added

- **The playground: olang in the browser.** The whole language —
  interpreter, bytecode tier, and the pure stdlib — now compiles to
  `wasm32-unknown-unknown` and powers a `/playground` page on the
  website: editor, curated examples, output pane, and a 5-second
  kill-switch. Safety comes from the platform, not trust: the wasm
  instance imports exactly three host functions (two clocks and an
  entropy source) and touches nothing but its own linear memory — no
  filesystem, network, process, or DOM — and it runs inside a Web
  Worker the page terminates on timeout. The boundary is a hand-rolled
  C ABI (`olang_alloc`/`olang_run`/`olang_result_free` returning
  length-prefixed JSON), so no wasm-bindgen toolchain is involved;
  `bun run build` in website/ builds and stages the artifact
  (`playground/` cdylib crate → `static/playground/olang.wasm`).
- **A "native" cargo feature** (default-on) now carries everything a
  browser can't have: rusqlite, rustyline, reqwest/tokio/hyper, memmap2,
  hostname/whoami/dirs, rayon/num_cpus. Without it, `db`/`fs`/`http`/`os`
  aren't registered and their builtin bridges answer with a plain
  "not available in the playground" error; `should_parallelize` is
  always false and every parallel path degrades to its sequential twin.
  `src/clock.rs` is the one clock for both worlds — native re-exports
  `std::time`, the playground rebuilds `Instant`/wall-clock/sleep on the
  host imports (std's own clocks panic on wasm, as does
  `env::current_dir`, now wrapped). `print`/`println` route through
  `src/output.rs`: stdout natively, a drained capture buffer in the
  playground.

### Changed

- **Dependency prune.** Sixteen unused dependencies removed — all four
  cranelift crates, crossbeam ×2, atomic, memoffset, target-lexicon,
  wide, num-traits, dashmap, indexmap, bincode, glob, walkdir — none
  referenced by any source file. chrono drops its default `wasmbind`
  feature (wall-clock "now" goes through `src/clock.rs`).
- **Docs audit against 0.38.** The book and README now state the tier's
  real coverage (158/164 corpus functions promote; refusal lists match
  the compiler's actual `CompilationFailed` sites), the current
  benchmark standings, and the new examples (minilisp, app). New
  doc-tested example blocks: traits in the tour, tuple destructuring and
  nested functions in the language reference, `col` quantifiers/`_by`
  family and `random` sampling in the stdlib reference. Stale claims
  fixed in internals.md (file map, refusal examples), ovm.md
  (self-contradictory refusal list, retired map-builtin limitation),
  roadmap item 13 (closed into the performance campaign), stability.md
  (`time` was stable-but-unlisted).

## [0.38.0] - 2026-08-09

### Added

- **Native collection builtins.** The minilisp dogfood quantified the
  bridged-builtin tax: `map_get`/`map_set` converted the *entire*
  environment map to AST values and back on every lookup and binding,
  turning an environment-threading interpreter — a 57× workload by
  shape — into a 1.35× one. Ten builtins now run natively on the VM
  value model with zero boundary conversion: `len`, `head`, `tail`,
  `cons`, `concat`, `skip`, `map_get`, `map_set`, `map_has_key`, and
  `entries` — each mirroring the interpreter's checks in the same order
  with the same messages, including the map/struct-like duality
  (`map_set` on a struct yields a struct) and `entries`' sorted keys.
  One subtlety pinned by a test: `map_has_key` checks *presence*, not
  value — a key explicitly holding Unit still exists, so it cannot ride
  on map_get's Unit-for-missing. **minilisp's lisp-fib(17): 500ms →
  62ms — the tier's advantage on it went from 1.35× to 11×.** N-body,
  fib, and pipelines unmoved.

- **The corpus tail: self-recursive nested fns, tuples, and a live
  divergence.** Self-recursive nested `fn` declarations compile — the
  name binds to its own compiled id during the body's compile (recursion
  is a direct `CallFn`), with the body's capture parameters appended to
  each recursive call, and the escaped form carries the name so
  interpreted copies recurse through call-time self-definition. The
  capture-appending detail was found the honest way: 03_algorithms'
  `binary_search` recursed 2 arguments into a 4-parameter body at
  runtime; the fix is pinned by a test. Tuple *expressions* compile (the
  instruction existed; the compiler arm didn't), and `let` destructuring
  goes through the full pattern machinery — tuples, lists, structs,
  enums — with a non-matching let raising the interpreter's
  PatternMatchFailed. And extending `let` exposed a **live divergence in
  0.37.0**: the interpreter's `let` evaluates to the bound value
  (observable when a block ends in one — `fn f() = { let x = 5 }`
  returns 5), but the compiled form yielded Unit; nested-fn declarations
  had the same gap. Both fixed and pinned. **Corpus: promoted 152 → 158,
  rejections 14 → 6** — the six survivors are async (`spawn`, promises)
  and global assignment, all by-design refusals.

## [0.37.0] - 2026-08-09

### Added

- **The unresolved-identifier tail, diagnosed and closed.** A sweep of
  every remaining refusal found three mechanisms, fixed together:
  (1) *Native module calls* — `db.execute`, `fs.read`, `json.parse`,
  `col.frequencies`, and every other stdlib module call outside the
  math/str allowlists refused. Now the module resolves in the closure to
  its Module value, the function's existence is validated at compile
  time against the module's own field set, and the call bridges under
  the builtin value's own dispatch name — the exact name and
  implementation the interpreter calls through. (2) `map_has_key` was
  simply missing from the allowlist. (3) *Recursion through lambdas* —
  a lambda referencing its own enclosing function, or one declared later
  (the template example's mutual `render`/`render_node`), refused
  because the name is not in the declaration closure. Registered
  functions now resolve through the registry, with the closure-miss
  routed through the dependency channel so forward references register
  and retry. The examples caught a real bug in the first version of that
  fix: a compiled lambda handed to a *bridged* builtin runs interpreted,
  and without the function value carried in its closure it hit
  "Undefined variable: render" — escaped lambdas now carry
  registry-resolved functions as values, pinned by a test reproducing
  the exact template shape. **Corpus: promoted 122 → 152, rejections
  43 → 14** — the largest single-rung coverage jump of the arc; what
  remains is self-recursive nested fns (4), tuple-pattern lets, async,
  and stragglers.

- **Maps are first-class in the VM.** The last missing data type: maps
  used to be crushed into a struct shape that could not convert back,
  which kept every map-touching function and every map-returning builtin
  off the tier. `ValueData::Map` mirrors the interpreter's map exactly
  and converts losslessly both ways; `#{...}` literals compile to a
  `MakeMap` instruction with the interpreter's key-coercion rules (bad
  keys raise the same type error); `==`/`!=` compare structurally; and
  ten map builtins (`map_get`, `map_set`, `map_remove`, `map_keys`,
  `map_values`, `map_len`, `map_merge`, `map_clear`, `entries`,
  `group_by`) are allowlisted over the bridge, retiring the
  "map-returning builtins are excluded" limitation. Promoting the
  map-heavy workflow example exposed a *pre-existing* VM gap the
  differential suite now pins: the interpreter concatenates
  `String + Int/Float` in both orders and the VM errored — the missing
  arms are added with the interpreter's exact stringification. Two test
  fixtures that used maps as their "unrepresentable value" specimens now
  use a genuinely unconvertible value instead, and the
  helper-cannot-compile test moved to global assignment as its durable
  uncompilable feature (its third choice, after global reads and map
  literals each became compilable). Corpus: promoted 117 → 122,
  rejections 55 → 43; map-literal refusals to zero.

- **Template strings and nested `fn` declarations compile.** A
  `MakeTemplate` instruction builds the string with the interpreter's
  exact interpolation rules — String raw, Int/Float/Bool via
  `to_string`, everything else through the AST value's Display (structs,
  enums, and lists interpolate identically, verified byte-for-byte).
  Interpolated expressions compile in written order. A nested `fn` is
  compiled as the equivalent named closure over the current frame
  (`MakeClosure` machinery), so helpers capturing enclosing parameters
  and calling sibling nested fns promote; a self-recursive nested fn
  refuses through the bound-names rule and stays interpreted, pinned by
  a test. Corpus: promoted 112 → 117; template-string and
  nested-declaration refusals both to zero.

- **Trait method dispatch compiles.** `value.m(..)` — the top remaining
  promotion blocker in the example corpus (21 refusals) — now compiles to
  a `CallMethod` instruction when the receiver expression is pure
  (locals, field chains, literals). Dispatch mirrors the interpreter
  exactly: a struct *field* named like the method takes precedence and is
  called without self; otherwise a direct `impl` for the receiver's
  runtime type, then the type's traits in registration order for a
  default (defaults calling back into the receiver's own impl work);
  otherwise the field-access error. The runtime type-name mapping mirrors
  `Value::type_name`, so traits implemented for primitives (`impl
  Describe for Int`) dispatch too. The interpreter feeds impls, defaults,
  and type-trait facts to the tier as declarations evaluate; any change
  to the dispatch landscape invalidates compiled functions, so an `impl`
  declared after a function promoted still dispatches correctly — pinned
  by a test. Receivers with side effects refuse: the interpreter's
  dispatch fallthrough re-evaluates the receiver, which the VM will not
  replicate for effectful expressions. The language tour — whose
  trait-default method caught the previous attempt at compiling method
  calls — now runs output-identical under both tiers, and one call site
  serves Point, Circle, and Int receivers in the tests. Corpus: promoted
  108 → 112.

## [0.36.0] - 2026-08-09

### Changed

- **Immediate operands: numeric literals ride in the instruction.**
  `x + 1`, `n < 2`, `i % 2` compiled to a LoadConst into a fresh register
  plus the operation — two dispatches for a value known at compile time.
  A `BinImm` instruction carries the literal; execution reuses the same
  fast path and fallback, so overflow, division by zero, and type errors
  are byte-identical. A literal left operand fuses when the operation
  commutes or the comparison flips. Measured: fib(30) ~103ms → ~89ms,
  pipeline ~28ms → ~26ms.

  A second fusion — compare+branch pairs merged into one instruction —
  was built, measured, and **reverted**: it retired 3.5% of executed
  instructions but ran 3–4% *slower* on every benchmark. Growing the
  instruction enum perturbs the dispatch match's code layout more than
  the saved dispatches earn back, even with deliberately slimmed arms;
  the honest conclusion is that dispatch-loop layout, not instruction
  count, is now the binding constraint, and the next real lever there is
  threaded dispatch — a different project. The negative result is
  recorded here so it isn't re-attempted casually.

- **Structs have interned shapes, and field reads have inline caches.**
  A struct used to be a hash map: every `p.x` hashed the field name
  (~28% of the N-body kernel). Now a struct is an interned shape — one
  `Arc<StructShape>` per (type, field set), field names in canonical
  order with a stable id — plus a values vector in shape order. Every
  `GetField` site carries a one-entry inline cache packing (shape id →
  field index) into a single atomic word, so concurrent VMs sharing
  bytecode can never see a torn pair: a repeat read of the same shape is
  an integer compare and an array index, no hashing. Misses take a cold
  outlined path that refills the cache — outlined deliberately, because
  a first draft with the miss path inline perturbed the dispatch loop's
  code layout and cost fib/pipeline 10% each (caught by benchmarking
  non-struct workloads, recovered exactly). `MakeStruct` interns its
  shape at compile time and stores field registers in shape order
  (field expressions still evaluate in literal order). Two new tests
  pin the risky parts: a polymorphic read site rotating through shapes
  that place the same field at different indices, and shaped structs
  round-tripping the tier boundary into interpreter pattern matches.
  Measured: N-body ~485ms → ~415ms — **ahead of Ruby (468ms) and at
  parity with CPython (416ms)** — with fib and pipeline unchanged and
  the checksum bit-identical. The profile after this rung shows
  essentially all remaining N-body time in the dispatch loop itself,
  which is what the instruction-fusion rung attacks next.

- **Frames are windows on one register slab.** Every call used to swap a
  whole `ExecutionState` in and out through a frame pool, re-size its
  register and locals vectors, and reset per-call bookkeeping. The VM now
  keeps a single contiguous `Vec<OvmValue>` for all live frames: entering
  a function bumps a window past the caller's, returning restores two
  integers, and the slab only grows — stale values above the logical top
  are recycled in place with the immediate drop-skip when the next call
  claims them (the Lua register-stack design). `CallFn` goes further:
  arguments copy straight from the caller's window into the callee's, no
  intermediate buffer. The dead `locals` array and per-frame bookkeeping
  fields are gone; `LoadLocal`/`StoreLocal` (never emitted since locals
  moved to registers) now error like other unreachable instructions. Two
  new stress tests target the design's failure modes: deep recursion
  carrying heap values across windows, and native map loops stacking
  frames above a live caller. Measured: fib(30) 135ms → ~104ms, the
  pipeline benchmark 38ms → ~27ms — **past CPython (31ms) on idiomatic
  pipeline code** — N-body unchanged, checksum bit-identical.

## [0.35.0] - 2026-08-08

### Added

- **Function values are callable — and cross the tier boundary.** Two
  changes that complete the higher-order story. First, `Value::Function`
  now converts to the VM losslessly (wrapped verbatim instead of
  deep-copied into a dead-end object), so a user function passed as an
  argument no longer knocks the whole call back to the interpreter.
  Second, a new `CallValue` instruction calls whatever function value a
  register holds: parameters (`fn apply(f, x) = f(x)`), curried calls
  (`g(a)(b)`), immediately invoked lambdas, and closure aliases. Compiled
  values run in the VM; everything declined routes through the bridge
  interpreter, which owns arity errors, default parameters, and the
  "Cannot call non-function value" error. Callee resolution now mirrors
  the interpreter's scope order — locals first — fixing a latent
  divergence where a parameter named after a builtin (`fn apply(len, x) =
  len(x)`) would have called the builtin once function values could
  cross. Two course corrections along the way, both caught by the
  suites: known-but-unregistered user functions must still route through
  the tier's dependency channel (transitive/mutual recursion promotes
  both functions), and method calls (`value.m()`) must keep refusing —
  they dispatch on runtime type through trait impls, which a field read
  cannot replicate; the language tour caught that one, and a regression
  test now pins it.

- **The pure `str` module compiles, and so does `show`.** The 30 `str`
  functions (`length`, `char_at`, `split`, `parse_int`, ...) are
  allowlisted over the same bridge as `math.*`, and `show` stringifies
  through the same path as `to_string` — including structs and enums,
  verified bit-identical. String-heavy code (parsers, regex engines,
  template renderers) lives on these.

  Corpus sweep across all examples after this and the enum work:
  **promoted functions 55 → 108, rejections 125 → 64**, with rejection
  cascades collapsing from 92 to 15. The regex engine's parse pipeline
  and the algebraic-types combinators (`opt_map`, `find_first`) promote.

- **Enums are first-class in the VM.** Enum values used to be crushed
  into a struct shape with a `__variant` field that could not convert
  back, so no enum ever crossed the tier boundary and any function
  touching one stayed interpreted. The OVM now has a real enum value:
  construction (`Circle(2.0)`) compiles to `MakeEnum` with constructor
  arity checked at compile time, unit variants bake as closure constants,
  `==`/`!=` compare structurally, and conversion is lossless both ways.
  Enum-variant *patterns* compile too — including the interpreter's exact
  quirks: a bare unit-variant name is an equality match rather than a
  binding (per the declared-variant rule, with locals shadowing back to a
  binding), struct-variant payloads match positionally in field-name
  order, and a plain tuple of matching length satisfies an enum pattern
  (legacy behavior). Struct and anonymous-object patterns compile as
  well. Chosen by measurement: a sweep of the example corpus found enum
  patterns and enum-typed closure constants were the largest class of
  compilation refusals — the regex engine's 15-function cascade traced
  to a single unit variant. Direct enum/struct pattern rejections across
  the corpus: 14 → 0. The differential suite caught one real divergence
  during development (a unit-variant pattern compiled as a binding,
  swallowing every arm below it) — fixed by mirroring the interpreter's
  declared-variant rule, with a new-variant declaration invalidating
  previously compiled functions the same way struct redeclaration does.

## [0.34.0] - 2026-08-08

### Changed

- **Struct construction compiles.** `Body { x: 1.0, ... }` and anonymous
  objects now build natively in the VM via a `MakeStruct` instruction.
  Literals validate against the declared field set at *compile* time with
  the interpreter's exact rules — unknown type, missing field, and
  surprise field all refuse compilation, so the function stays
  interpreted and the interpreter raises its own error. The interpreter
  feeds struct declarations to the tier as they evaluate; a type
  redeclared with a *different* field set invalidates every compiled
  function (their baked validation could go stale) and its literals
  refuse from then on, keeping the interpreter's live registry the
  authority — pinned by a redeclaration test. With this, **every function
  in the N-body example promotes**: 6 promoted, 0 rejected, 7 tier
  crossings for the whole run (127M instructions, all inside the VM),
  ~525ms → ~453ms. The whole pipeline arc closes: `make_bodies`, `step`
  (struct-building capturing lambda inside `map`), and `simulate` were
  the last holdouts.

- **Capturing lambdas compile.** A lambda that captures the enclosing
  function's *runtime* state — a parameter, a local — no longer refuses
  the whole function. The lambda body compiles once as a standalone
  function whose trailing parameters are the captured names, and a new
  `MakeClosure` instruction snapshots the capture registers at the lambda
  expression — the interpreter's own capture-by-value moment, so a local
  reassigned after the lambda is created is not seen, and a lambda built
  in a loop captures each iteration's value (both pinned by tests). The
  resulting closure value runs natively through `map`/`filter` (captures
  appended to each element call), converts losslessly to an interpreter
  function at the bridge (fold/reduce and friends agree), and survives
  escaping — a compiled function can return the closure to interpreted
  code and it behaves identically. Still refused: a lambda capturing a
  name the enclosing function binds only later, and calling a
  lambda-valued expression directly. Measured over 1M elements:
  `map((x) => x * f + 1)` with `f` a runtime parameter went 417ms → 39ms
  (10.7×) — the last slow row of the pipeline table. All four pipeline
  forms now land within 1.6× of the explicit loop.

- **Functions that read globals compile now.** A free identifier that
  resolves in the function's own closure bakes as a constant — sound
  because the interpreter installs exactly that closure as the call
  environment, and closures are declaration-time snapshots (a global
  mutated after the function's declaration is not seen; pinned by a new
  test in both tiers before building on it). Function values bake in the
  lossless `AstFunction` representation, so a named function passed to
  `map`/`filter` now compiles end-to-end through the native higher-order
  path. This retires one of the tier's oldest limitations: the N-body
  example's softening constant, inlined as a literal specifically because
  a module-level binding kept the kernels off the tier, is a named
  constant again — kernels still promote, checksum bit-identical.
  Measured over 1M elements: `map` with a named function 192ms → 31ms
  (6.2×). Names absent from the closure (the interpreter would fall back
  to the caller's runtime scope) still refuse, as do assignments to
  globals. Tests updated: two asserted the old refusal as the expected
  behavior; new tests pin snapshot semantics exactly and prove baked
  function values survive redefinition of their name.

- **The higher-order builtins loop inside the VM.** `map`, `filter`, and
  `sum` previously bridged out of the VM on every call — the list and
  function converted to AST values, the interpreter looped, and every
  element crossed the tier boundary individually. When the collection is a
  list and the function argument compiles, the loop now runs natively:
  one VM `execute()` per element, no conversion anywhere, and a mapped
  list flows into `sum` without ever leaving the OVM value model.
  Function *values* (lambda constants, functions passed by value) compile
  on first sight, cached by body-allocation identity with Weak-upgrade
  validation; a new compiler mode resolves their free identifiers from
  the value's own attached closure — sound because the interpreter
  installs exactly that closure as the call environment (and closures are
  snapshots: a global mutated after declaration is not seen, verified).
  Anything declined — arity mismatch, default parameters, trait bounds,
  uncompilable body, non-list collection, `sum` past the parallel
  threshold — still bridges to the interpreter, which remains the
  semantic authority; a loop never falls back mid-flight, so element
  errors propagate exactly as the interpreter would. Interpreter quirks
  are mirrored, not "fixed": `filter` keeps an element only on exact
  `Boolean(true)`, `sum` promotes int→float mid-list and overflow-checks
  integers. Measured over 1M elements: `xs |> map((x) => x + 1) |> sum`
  224ms → 31ms (7.2×), within 1.3× of the equivalent explicit loop. Five
  new tier-agreement tests cover results, the exact-Boolean filter, string
  and nested maps, sum edge cases (empty, mixed, overflow, non-numeric),
  and mid-map error propagation.

- **The tier stopped allocating a string per call.** `try_call` cloned the
  callee's name into a fresh `String` on every call of every named
  function, purely to probe three maps with it — the callee is the
  caller's value and independent of the tier, so the lookups now borrow.
  Measured: `map` with a named compiled function 154ms → 145ms over 1M
  elements.

- **The bytecode tier is boxed, so a call no longer memcpys it.** The tier
  owns the whole VM — compiler, bytecode caches, execution state, frame and
  argument pools — which is 1,424 bytes, and it was stored inline in the
  interpreter. `call_function` moves the tier out of the interpreter and
  back on every call (so the tier can borrow the interpreter for builtins),
  which meant about 2.8 KB of memcpy per interpreted function call. Behind
  a `Box` it is two pointer moves. This costs nothing for code already
  running inside the VM, and pays where olang is currently weakest —
  pipelines and higher-order builtins, which call back through the
  interpreter once per element. Measured over 1M elements: `map` with a
  named compiled function 192ms → 154ms, `map` with a capturing lambda
  417ms → 382ms.

## [0.33.0] - 2026-08-08

### Added

- **`--ovm-stats` reports instructions retired.** The VM has always counted
  them; nothing surfaced the number, so the tier's actual workload was
  invisible and per-instruction cost could not be measured without
  instrumenting a build. (The N-body example retires 126.8M bytecode
  instructions.) Rejection *reasons* were already available under
  `--verbose`.
- **The bytecode tier compiles unary `-` and `!`.** The VM has had `Neg`
  and `Not` instructions, and an `execute_unary_op` matching the
  interpreter exactly (`checked_neg` with the same overflow message, `-x`
  on floats, `!b` on booleans, a type error otherwise), since it was
  written — the compiler simply never emitted them. A single `-x`
  anywhere in a function therefore refused the whole function and left it
  on the interpreter. Found while auditing the tier tests: the test named
  `indexing_promotes_and_agrees` promoted nothing, because its `xs[-1]`
  made it uncompilable.

### Changed

- **Value copies and drops stopped going out of line.** `clone_simple` is
  a 20-variant match, far past what LLVM will inline, so *every* register
  copy became a call into it — it was the second-hottest symbol in a
  profile. Immediates (which fill nearly every register in a numeric
  kernel) now clone through a small inlinable fast path with the heap
  variants behind `clone_heap`. The mirror image showed up next:
  `drop_in_place<ValueData>` was ~25% of VM samples, because overwriting a
  register runs the old value's drop glue. Writes and frame resets now
  skip that glue when the value being replaced owns no heap payload,
  decided by matching on the data itself rather than the header tag so it
  cannot disagree with reality. Measured: N-body ~680ms → ~530ms (8.0M
  interactions/sec), fib(30) ~218ms → ~140ms. Two new tier tests cover the
  skip decision: a register cycled through strings, ints, lists and floats
  on every pass, and heap values returned from pooled frames.

- **Profile-guided dispatch-loop cuts: phantom errors and double dispatch.**
  A CPU profile of the N-body run showed ~15% of VM time in
  `drop_in_place<BytecodeError>` — the register/constant accessors used
  eager `ok_or(...)`, constructing (and immediately dropping) an error
  value on every *successful* access, and `BytecodeError`'s String
  variants give it real drop glue. All hot-path accessors now construct
  errors lazily. Second find: every arithmetic/comparison instruction
  dispatched twice — the instruction match already knew the op, then
  `execute_binary_op` re-matched it. A `binary_fast` helper inlined with a
  constant op collapses each numeric instruction to a type check plus the
  operation; mixed-type operands, overflow, and division by zero fall back
  to `execute_binary_op`, which keeps owning the error messages. Measured:
  N-body ~940ms → ~680ms (6.2M interactions/sec), fib(30) ~267ms → ~218ms,
  checksum bit-identical. Cumulative for the whole performance arc:
  N-body 2,511ms → 680ms (3.7×), fib(30) 736ms → 218ms (3.4×).

- **The dispatch loop stopped paying for hashing and conversion it didn't
  need.** Three more measured per-operation taxes removed: (1) `GetField`
  cloned the field-name `Arc<String>` and the whole object value (two
  refcount round-trips) per read — it now borrows both, and struct field
  maps hash with FNV-1a instead of SipHash (field names are short
  identifiers from program text; the maps are tiny). (2) `math` builtins
  ran through the full interpreter bridge on every call: OVM→AST argument
  conversion, name-string dispatch, a result round-trip check, and
  AST→OVM conversion back. The 25 pure float functions (`sqrt`, `sin`,
  `pow`, `atan2`, ...) now compile to a `CallBuiltin` instruction resolved
  by id at compile time and evaluate as plain `f64` ops — non-numeric
  arguments still take the interpreter path so errors stay identical, and
  a new tier-agreement test locks every table entry (Float and Integer
  arguments) to the interpreter's results. (3) `Call*` argument marshaling
  reused pooled buffers instead of allocating per call. Measured: N-body
  1,427ms → ~940ms (3.6M interactions/sec, checksum bit-identical),
  fib(30) 285ms → ~267ms.

- **VM-internal calls resolve at compile time.** A call to a user function
  compiled to `CallNamed`, which re-hashed the function's *name* against
  the registry on every execution — fib(30)'s 2.7 million recursive calls
  each paid a string hash, then a hash-map bytecode lookup behind an
  RwLock. The compiler already validates every callee against its registry,
  so it now emits a new `CallFn` instruction carrying the resolved
  `FunctionId` (user functions checked before builtins, preserving
  shadowing semantics; builtins keep `CallNamed`). `execute()` fetches
  bytecode from a lock-free per-VM table indexed directly by id — sound
  because ids come from a global monotonic counter and are never reused —
  falling back to the shared RwLock cache only on first touch. Argument
  marshaling pre-sizes its vector, and the dispatch loop counts
  instructions in a local flushed once per call instead of writing a stats
  field per instruction. Measured: fib(30) 340ms → 285ms, the boundary
  microbench 65ms → 33ms per 100k calls (still flat in argument size),
  N-body 1,461ms → 1,427ms.

- **The tier call boundary no longer taxes every call.** Three per-call
  costs measured and removed: (1) arguments were deep-converted
  Value→OvmValue on every call — a function taking a 1,000-element list
  paid a full conversion walk per call (measured: identical work cost
  0.4µs with a 1-element argument and 8.5µs with 1,000). Arc-backed lists
  now hit a pointer-identity conversion cache validated by Weak upgrade,
  so an unchanged list converts once; per-call cost is flat in argument
  size. (2) The compiled bytecode (whole instruction vector + constants)
  was deep-cloned out of an RwLock per call; the cache now stores
  Arc<CompiledBytecode>. (3) Every call allocated a fresh register/local
  frame and took two Instant::now() samples for an unused statistic;
  frames are pooled with capacity retained and per-call timing is gone.
  Measured on the repository benchmarks: N-body 2,511ms → 1,461ms (1.7×),
  fib(30) 736ms → 340ms (2.2×), boundary microbench flat at 65ms for
  1/100/1,000-element arguments (was 83/246/1,701ms). Three new
  tier-agreement tests cover cache identity: repeated same-list calls,
  fresh lists in a loop (allocation reuse), and derivative lists.

## [0.32.0] - 2026-08-08

### Added

- **The bytecode tier compiles `math.*` calls.** The pure `math` module
  functions (`sqrt`, `sin`, `cos`, `pow`, `floor`, `abs`, `atan2`, ... — 30
  in all) are now compilable builtins, recognized when the module is a bare
  identifier (a local shadowing `math` is still field access, not the
  builtin). Before this, a single `math.sqrt` in a hot loop kept the whole
  function on the interpreter — so any real numeric kernel missed the tier.
- **`examples/concurrency/nbody/`** — an N-body gravity simulation: `Body` structs whose
  force kernels (`accel_x`/`accel_y`) read five fields per interaction in an
  O(n²) loop and call `math.sqrt`, exactly the field-access + math shape the
  tier now accelerates. It times itself and reports throughput; a `test`
  block locks determinism and momentum conservation. **Measured 9× on the
  bytecode tier** (120 bodies × 150 steps: 23.2s interpreter → 2.6s tier).

### Changed

- **`olang test` runs through the bytecode tier**, exactly as `olang <file>`
  does by default, so tests execute at production speed and exercise the tier
  that actually ships. (The N-body example's self-check went from 24s to
  2.7s.)

## [0.31.0] - 2026-08-08

### Added

- **`examples/web/loadtest/`** — a self-contained HTTP load test, the server and
  its concurrent client fleet in one olang program. It boots a SQLite-backed
  API in a spawned task, fans client workers out across `spawn` threads
  (each firing a burst of requests and timing them), merges per-worker stats
  after `await`, and reports throughput and latency. A `test` block checks
  the load-test invariant on every run: the server's own recorded hit count
  equals the clients' successes exactly (2xx + the deliberate-500 route),
  with zero unexpected failures — so `http.serve`'s worker pool provably
  loses no writes under concurrent load. Measured ~17–19k req/s locally;
  1,200 requests through a 4-worker pool stay perfectly consistent.
- A Rust integration test (`concurrent_load_writes_are_not_lost`) pins the
  same invariant: 16 client threads × 40 writes against an 8-worker pool
  sharing one SQLite connection, asserting the final row count is exact.
  (Dogfooding `http.serve` under genuine concurrent load, driven by olang's
  own `spawn`/`await` rather than an external tool, found no bugs — the 0.29
  worker pool and 0.30 failure-as-a-value semantics compose correctly.)
- **The bytecode tier compiles field access and indexing.** `p.x` and
  `xs[i]` (negatives count from the end) now lower to dedicated
  name-based `GetField` and `IndexGet` instructions with the interpreter's
  exact semantics (missing field, out-of-bounds, and type errors all
  match). Structs, objects, and parsed JSON objects also became
  tier-representable — a value round-trips through the OVM when every field
  does — so functions that read fields off a struct argument finally
  *promote and run in bytecode* instead of falling back forever. Measured
  ~1.7× on a field-access-heavy hot loop (920ms → 540ms). Since almost
  every real function touches a field, this widens promotion from numeric
  kernels to ordinary code. Five new tier-agreement tests cover field
  access, indexing (incl. negative and OOB), nested field+index, and a
  struct returned from a compiled function round-tripping identically.

### Fixed

- **Trait-method dispatch stays correct under promotion.** Impl methods
  were never registered with the tier's ambiguity guard, so once struct
  arguments became tier-representable, a trait method like `area` — defined
  by several types, dispatched by receiver — could compile one type's body
  and run it for all receivers (`self.s` on a shape with no `s` field). Impl
  methods are now noted to the tier: multiple same-named impls mark the name
  ambiguous and keep it on the interpreter (correct dispatch), while a
  single-impl method still promotes. Found immediately by the tier-agreement
  doc test.

## [0.30.0] - 2026-08-08

### Added

- **`examples/concurrency/pargrep/`** — parallel code search dogfooding the real
  `spawn`: the coordinator walks a tree, deals files to spawned worker
  threads (fs + re + str running concurrently), merges after `await`, and
  prints sequential-vs-parallel timings (~2× on the examples tree). A
  `test` block asserts the parallel result equals the sequential one on
  every run.

### Fixed

- **Awaiting a rejected promise yields `Err(e)` instead of aborting.**
  A failed `spawn` task, `Promise.reject`, or a rejecting `all`/`race`
  produced a hard runtime error nothing could catch — one failed worker
  killed the whole program, making failure handling impossible. `await`
  now returns the rejection as an ordinary `Err` value, composing with
  `match`, `unwrap_or`, `?`, and `try`/`catch`. Found immediately by
  dogfooding a parallel searcher's per-task recovery.
- **`try` blocks pass non-Result values through.** `try { await task }
  catch (e) { fallback }` failed with a type error whenever the task
  *succeeded* (successful `await` yields the bare value, not `Ok`). A try
  block's non-Result value now passes through unchanged; Ok/Err behavior
  is untouched.

## [0.29.0] - 2026-08-08

The consolidation release, addressing an external review's bottom line item
by item (the plan lives in `docs/roadmap.md`).

### Added

- **Bounded concurrent `http.serve`.** Independent connections run on a
  configurable worker pool (host parallelism by default), with a bounded
  queue, `503` overload responses, keep-alive request caps, socket timeouts,
  and `remote_addr` on requests. The notes-server example adds configurable
  JSON Lines access logging and its dependency-free benchmark reports actual
  average in-flight load.
- **`spawn` runs on a real OS thread** (C2). Previously it evaluated its
  expression eagerly and wrapped a resolved promise — concurrency
  decoration. Now `spawn expr` evaluates on a background thread against a
  thread-safe interpreter clone (the same worker pattern `http.serve`
  uses): spawn returns immediately, `await` joins, results are memoized so
  cloned promises can be awaited repeatedly, a failing task rejects, and
  capture is by value like closures. Three 100ms tasks awaited together
  take ~100ms — verified by tests in both tiers. The deterministic
  deadline model for `Promise.delay` is unchanged.

### Changed

- **The interpreter is split into semantic components** (C5). The
  5,628-line interpreter.rs is now src/interpreter/ — core evaluation
  (2,629 lines) plus modules, errors, module_cache, ops, patterns,
  environment, and spawn_registry components. Move-only; public paths
  preserved via re-exports; the full suite is the equivalence proof.
- **One capability authority** (C6). The README no longer contradicts the
  book (it had still claimed "v0.23, experimental", "11 modules", and a
  placeholder HTTP server). It is now a short, accurate overview whose
  code runs in CI, and docs/stability.md is explicitly the authoritative
  capability statement.
- **Struct construction is validated** (C1). A struct literal must name a
  declared struct type and supply exactly the declared field names —
  missing, surprise, and undeclared-type constructions are now errors
  naming the problem. Field VALUES stay dynamic; the book states the
  position plainly: declarations fix shape, not types. Anonymous objects
  remain free-form. (Previously `NeverDeclared { surprise: 42 }`
  constructed happily.)
- **The lazy facade is honest** (C4). Every "lazy" path (map, filter,
  take, skip, concat, and `lazy()` itself) built a thunk and immediately
  forced it — eager semantics at extra cost. All are now direct eager
  implementations; `lazy(v)`/`force(v)` keep their observable behavior
  (identity) and the stdlib reference says so. The fabricated
  memory-pressure machinery (estimates derived from a stack address) and
  the module-cache wipe after 50-element range maps are deleted;
  src/internal/ (2,687 lines) is gone entirely.
- **Numeric parallelism follows configuration** (C4). `sum` hard-coded
  parallel execution at ≥5 elements, bypassing the configured threshold;
  it now respects it, and the default threshold rises from 10 to 10,000 —
  small lists are always cheaper sequentially.

### Fixed

- **MVS resolution verifies requirements** (C3). The resolver kept an
  already-selected version whenever it was merely ≥ a new requirement's
  floor, never checking that it *satisfies* the requirement — `^1.0`
  alongside a selected 2.0.0 passed silently, and the `Conflict` error was
  unreachable. Selection now re-verifies every requirement in-loop and at
  fixpoint; incompatible ranges produce `Conflict` naming the package and
  requirements, in both resolution orders.

## [0.28.0] - 2026-08-08

### Added

- **`olang test` — the test runner.** Discovers every `.ol` file containing
  a top-level `test` block (recursively), runs each file in a fresh
  interpreter from its own directory with its package dependencies, and
  reports every block's outcome. Under the runner a failing block records
  its failure and later blocks still run (inline `olang <file>` behavior is
  unchanged: a failing assertion aborts). Files without test blocks are not
  executed. Non-zero exit on any failure; setup errors outside a block are
  reported as file errors.
- **`olang fmt` — the formatter.** Conservative whitespace hygiene applied
  only outside multi-line strings: CRLF→LF, trailing whitespace stripped,
  leading tabs → 4 spaces, blank-line runs collapsed, exactly one final
  newline. It never re-indents or reflows, and it refuses to write unless
  the formatted source re-parses to an AST identical to the original —
  unparseable or meaning-changing results are skipped and reported.
  `--check` reports and exits non-zero for CI. Applied to the examples tree
  (19 files cleaned; everything still green).
- **The book gains a [Tooling](docs/tooling.md) chapter** covering both
  tools and the examples harness; the roadmap's tooling track is landed.

## [0.27.0] - 2026-08-08

### Added

- **Roadmap Tier 4** — the two big lanes:
  - **The OVM compiles `&&`/`||` as conditional jumps.** Any function
    containing a logical operator previously fell back to the tree-walker
    permanently — and post-hardening, that was most interesting functions.
    The lowering short-circuits on *exactly* `Boolean(false)`/`Boolean(true)`
    (via a never-erring pattern-equality test, not truthiness), so the
    interpreter's strict semantics are preserved bit-for-bit — including
    `0 && true` being a type error. Guard-heavy hot loops measure ~2.2×
    faster. Fixing this surfaced a latent divergence: the OVM's And/Or
    *instructions* were JS-style truthiness coercions returning operands;
    they are now strict Boolean, matching the interpreter.
  - **`http.serve` keep-alive.** Connections are persistent per HTTP/1.1:
    the server loops requests on one connection until the client closes,
    sends `Connection: close`, idles past the timeout, or hits a
    per-connection cap. Responses advertise `Connection: keep-alive`
    accordingly. Verified by an integration test running two requests over
    one TCP stream and by curl's connection reuse against the notes API.
- **Roadmap Tier 3** — stdlib gaps:
  - **`time` module** — `time.now_ms()` (epoch milliseconds),
    `time.monotonic_ms()` (a clock that never goes backwards, for
    durations), and `time.sleep(ms)`.
  - **`fs.walk(dir)`** — every file below a directory, recursive and sorted;
    **`fs.glob(pattern)`** — files matching a pattern where `*` matches
    within a segment, `?` one character, and `**` spans segments.
  - **`os.exec` options** — an optional third argument
    `#{ "cwd": ..., "stdin": ..., "env": #{...} }` (any subset); the 2-arg
    form is unchanged, unknown options are rejected.
  - **`str.fmt(template, ...)`** — fills `{}` placeholders in display form
    (strings bare); `{{`/`}}` escape literal braces; placeholder/argument
    count mismatches are errors, not silence.
  - **db transactions** — `db.begin` / `db.commit` / `db.rollback` over a
    connection handle.
  - `examples/run_all.ol` now dogfoods the tier: per-run `cwd` via the exec
    option (no more chdir dance) and sub-second timing via
    `time.monotonic_ms` + `str.fmt`.
- **Roadmap Tier 2** — control flow and errors:
  - **`return expr`** — exits the nearest function (or lambda) with the
    value; bare `return` yields Unit; escapes loops within the function.
    Implemented as an unwind signal caught at the call boundary, like `?`.
    `return` is now a reserved keyword. Top-level `return` is an error.
  - **`break value`** — a loop becomes an expression: `loop { ... break x }`
    evaluates to `x` (works in `while` and `for` too). Bare `break` keeps
    its existing behavior. The bytecode tier refuses functions using
    `break value` or `return` (they stay on the interpreter) — never
    diverging; verified identical results across `--no-ovm`, default, and
    `--ovm-tier=1`.
  - **`error` declarations have semantics** (previously reserved-but-parsed):
    bare variants are singleton values, payload variants
    (`Invalid: { msg: String }`) become constructors taking the fields
    positionally. The values are ordinary enums, so `Err(NotFound)` and
    `match r { Err(Invalid(m)) => ... }` compose with the existing Result
    and pattern machinery. Moved from Reserved to Stable in the book.
- **Roadmap Tier 1** (`docs/roadmap.md` tracks the plan; every item grounded
  in dogfooding friction):
  - **`show(v)`** — display rendering: strings bare, everything else as
    `to_string` (which keeps its repr form, strings quoted).
  - **`entries(m)`** — the `(key, value)` tuples of a map or any struct-like
    value, sorted by key for deterministic iteration.
  - **Write-side struct-likeness** — `map_set`/`map_remove` now accept
    objects, structs, and parsed JSON, returning a new value of the same
    kind; the write side finally matches the read side.
  - **`for` tuple destructuring** — `for (i, x) in enumerate(xs)` and
    `for (k, v) in entries(m)` bind element parts directly (desugars to a
    tuple `let`, so both tiers agree by construction).
  - **Strings iterate** — `for ch in "abc"` yields 1-character strings, by
    character (not byte), in both execution tiers.
- **`http.serve` is a real HTTP server.** It was a placeholder that returned
  "server would start" without ever calling the handler. It now binds
  `127.0.0.1:port` (port 0 picks a free one and reports it), parses HTTP/1.1
  requests (method, path, decoded query map, lowercased header map, body),
  and calls the olang handler per request — a bare string return is a 200, a
  `http.response`/`response_with_headers` struct is honored. Handler errors
  become 500s, malformed requests 400s, and the server keeps serving through
  both. Sequential and blocking by design; integration-tested over real TCP.
- **`examples/web/webserver/`** — a notes JSON API on `http.serve`: a router with
  `:id` path parameters dispatching handlers over a SQLite store that
  persists across requests (the handler closes over the connection).
  `run_all.ol` skips long-running servers with a visible note.
- **`examples/language/markdown/`** — a markdown→HTML converter: a block parser
  (headings, lists, blockquotes, fenced code, rules, paragraphs) over a
  recursive inline span renderer (`code`, bold, italic, links, escaping).
  Handles unclosed markers gracefully and converts the repository's own
  README; a `test` block self-checks the conversion contract on every run.

### Fixed

- **Assertion arguments may span lines.** The `assert_eq`/`assert_ne`/
  `assert`/`assert_true`/`assert_false` grammar rules had no newline
  handling between arguments, unlike every other call form — so a
  multi-line `assert_eq(...)` silently fell out of the assertion grammar
  and parsed as a call to an undefined `assert_eq` function, failing at
  runtime with "Undefined variable". Found by the markdown converter's
  self-check block.
- **`json.stringify` serializes maps.** `#{ ... }` literals and db rows are
  `Map` values; stringify rejected them ("Cannot convert Map to JSON") while
  handling structs and objects. Maps now serialize as JSON objects — found
  dogfooding the webserver, where `db.query` rows feed straight into a JSON
  response.
- **`http.response_with_headers` accepts map headers.** It required a struct,
  but object field names cannot contain `-`, making `Content-Type`
  unwritable. A map literal (`#{ "Content-Type": "application/json" }`) now
  works.

## [0.26.0] - 2026-08-07

### Added

- **The olang book.** Comprehensive documentation under `docs/`: a hub
  (`docs/README.md`), a tour, a complete language reference
  (`docs/language.md`, superseding `docs/syntax.md`), a complete stdlib
  reference (`docs/stdlib.md`), a contributor internals guide
  (`docs/internals.md`), and a stability policy (`docs/stability.md`)
  committing the documented surface to remain stable while development
  focuses on optimization, new features, and stability. Every olang code
  block in the book is executed by `doc_examples_test` in CI.
- **`let mut` is real syntax.** Previously `let mut x = 1` mis-parsed as two
  statements, leaving a stray `mut` binding. `mut` is now a contextual
  keyword in `let`: it marks intent (all bindings are assignable) and the
  statement parses as one declaration.
- **`os.exec(program, args)` runs external programs.** Returns
  `Result<{ code, stdout, stderr }, Error>` — the exit code and captured
  output on success, an `Err` only when the program can't be launched. This
  lets an olang program drive other programs.
- **`examples/run_all.ol`** — a test harness that discovers every standalone
  script and every package (`main.ol`) under `examples/` and runs each in its
  own `olang` subprocess (from the program's own directory), captures output,
  and prints a pass/fail summary with a non-zero exit on any failure. Built on
  `os.exec` plus `fs.list_dir` — a self-hosted way to check the examples stay
  green.
- **Programs receive command-line arguments.** `olang script.ol a b c` now
  passes `a b c` through to the program; `os.args()` returns
  `[script, a, b, c]` (previously it returned the interpreter's own argv and
  the CLI rejected trailing args). This makes real CLIs writable in olang.
- **`share trait` and `share impl`.** Traits and impl blocks can now carry the
  `share` keyword for symmetry with `share fn`/`type`/`let`. (Traits already
  register globally, so a plain `trait` in a module also reaches consumers;
  `share` is now simply accepted rather than a parse error.)
- **`else if` chains.** Conditionals can chain with `else if COND => ...`
  instead of only nesting as `else => if ...`. Both tiers.
- **List concatenation with `+`.** `[1, 2] + [3, 4]` yields `[1, 2, 3, 4]`,
  matching how `+` already joins strings — so building a list up element by
  element (`acc = acc + [x]`) works. Both the interpreter and the OVM tier.
- **Enum variant constructors cross the module boundary.** Importing a shared
  enum type (`use m { Node }`), a variant by name (`use m { Text }`), or `*`
  now brings the variant constructors into scope, so a shared ADT is
  constructible in the importer and not only matchable. There is no qualified
  `Type::Variant` form, so the bare constructor had to travel with the import.
- **Multi-line `use` import lists.** The names inside `use m { ... }` may span
  lines and end with a trailing comma.
- **`examples/language/jsonschema/`** — a JSON Schema validator: the schema and document
  are both parsed JSON, and validation recursively walks them, collecting a
  pathed error (`$.address.zip`) per violated keyword (type, enum, required,
  properties, items, and the min/max/length bounds). Found no new bugs — the
  JSON, map-accessor, recursion, and comparison paths were already hardened by
  earlier rounds.
- **`examples/language/regex/`** — a backtracking regex engine: a recursive-descent
  parser compiles a pattern to a recursive `Re` AST, and a continuation-passing
  matcher walks it. Supports `. * + ? | ( )`, character classes, anchors, and
  `\d \w \s`, with `find`/`find_all`/`matches`.
- **`examples/language/parser/`** — a parser combinator library (parsers as
  `(input, pos) -> result` functions, composed by higher-order combinators)
  with a recursive arithmetic grammar that parses and evaluates in one pass.
- **`examples/language/workflow/`** — a data-driven state machine engine with guards
  and actions as first-class function values, running two machines (an
  expense-approval pipeline and a cyclic turnstile) on one engine.
- **`examples/language/template/`** — a mustache-style template engine self-hosted in
  olang (lexer, parser over a shared `Node` ADT, renderer), driven by a JSON
  context. Exercises all four fixes above.
- **`examples/data-processing/dataproc/`** — a CSV→aggregate→JSON data pipeline: reads sales
  rows with `csv`, types and aggregates them, emits a `json` report, then
  reads it back and selects fields by a runtime key.
- **`examples/concurrency/scheduler/`** — concurrent fan-out and timeouts with
  `async`/`await` and `Promise.all`/`race`.
- **`examples/data-processing/loganalyzer/`** — a second dogfooded package: parses
  application logs with `re` capture groups, aggregates by level and route
  with `col` + pipelines, and reads files with `fs`. Handles malformed
  lines, missing/empty files, and 2000-line logs. Found no new bugs — the
  taskcli round had already hardened the shared package/args/import paths.
- **`examples/tools/taskcli/`** — a persistent task tracker as a real multi-file
  package (SQLite store, a `col`+pipeline reporting module, a domain module,
  and CLI dispatch on `os.args()`), built by dogfooding the language.

### Fixed

- **A forgotten `=` in `let` is a parse error.** `let scores #{ ... }`
  silently parsed as an uninitialized `let scores` (bound to Unit) plus a
  stray expression statement — in the REPL the echoed map made the binding
  look successful. An uninitialized `let` must now end its statement; the
  error points at the unexpected token with a `let name = value` hint.
- **`?` propagates the `Err` instead of aborting.** `expr?` on an `Err`
  raised a runtime error ("Tried to unwrap error") rather than returning the
  `Err` from the enclosing function — making `?` unusable on its main path.
  It now unwinds to the function-call boundary and the function returns that
  `Err` to its caller. Found while writing the language reference: the
  documented behavior is now the real one.
- **Lists, tuples, maps, and structs compare with `==`/`!=`.** Structural
  equality existed only for enums; `[1, 2] == [1, 2]` was "Invalid binary
  operation". All compound values now compare structurally, matching the
  equality already used by pattern matching. Found while writing the
  language reference.
- **`&&` and `||` short-circuit.** The right operand was always evaluated, so
  a guard like `x != 0 && y / x > 0` still divided by zero and a backtracking
  matcher's progress guard recursed forever. The right operand now runs only
  when the left doesn't already settle the result. Found by dogfooding a regex
  engine.
- **Identifier patterns bind instead of misfiring as variant tests.** A bare
  name in a pattern was treated as a unit-variant equality test whenever it
  merely resolved to a unit variant *in scope* — so a binding sub-pattern like
  `b` in `Node(a, b)` silently failed to match when some `b` already in scope
  held a unit variant. A name is now a variant test only when it is a
  *declared* variant; otherwise it binds. Found by dogfooding a regex engine
  (`Concat(a, b)` with `b` bound to the `$` anchor node).
- **Strings compare with `<`, `<=`, `>`, `>=` in the interpreter.** Only
  `==`/`!=` worked; the ordering operators raised "Invalid binary operation"
  even though the bytecode tier accepted them — so the result depended on
  whether a function had been promoted. The interpreter now orders strings
  lexicographically by Unicode scalar value, matching the tier. Found by
  dogfooding a parser combinator library, where `c >= "0" && c <= "9"` is
  everywhere.
- **Same-named functions in different modules no longer collide.** The
  bytecode tier keyed compiled functions by bare name, so once two modules
  each defined (say) `initial_state`, the first one compiled ran for *both*
  namespaces — and a caller's private helper could resolve to another
  module's helper. With the default tier threshold of 1, this struck on the
  first call. A name bound to two distinct function bodies is now left to the
  interpreter, which resolves each through its own closure. Found by
  dogfooding a state machine engine running two machines at once.
- **`join` renders string elements without quotes.** `join(["a", "b"], ",")`
  is now `a,b`, not `"a","b"` — it had used each value's debug form. Matches
  `str.join`.
- **`map_get`/`map_has_key`/`map_keys`/`map_values` read any struct-like
  value.** They previously accepted only a `Map`, so a parsed JSON object,
  an anonymous object, or a named struct could be read by dot access but not
  by a runtime key. They now view a map or any struct's fields uniformly —
  found by dogfooding a data processor, where selecting a JSON column by a
  variable is the natural pattern.
- **`Promise.all` / `Promise.race` now compose.** Three async bugs found by
  dogfooding a concurrency program: (1) they accept any list expression, not
  just a literal `[...]`, so `Promise.all(jobs)` with a variable works;
  (2) they resolve *pending* delay-promises instead of erroring with "async
  scheduling not implemented", sleeping once until the latest deadline
  (`all`) or the earliest (`race`) — so fan-out over delays takes the max
  latency, not the sum; (3) async lambdas with no parameters (`async () => x`)
  parse (same zero-parameter bug that had bitten regular lambdas).
- **Sub-directory module imports resolve the named file.** `use lib.greet`
  loaded a generated directory index instead of `lib/greet.ol`, and
  generating that index *wrote a file into the user's source tree* on import.
  The directory auto-index feature is removed: directory imports resolve a
  user-written `index.ol`/`mod.ol`, and named modules resolve the named file.

## [0.25.0] - 2026-08-06

### Added

- **`mathx` — the pure subset of `math`, self-hosted in olang.** A second
  embedded module mirroring the parts of `math` that need no native float
  intrinsics: `abs`, `sign`, `min`, `max`, `gcd`, `lcm`, `factorial`,
  `floor`, `ceil`, `trunc`, `round`, `fract`, `radians`, `degrees`, a
  Newton's-method `sqrt`, and the constants `PI`/`E`/`TAU`. The
  transcendentals (`sin`, `cos`, `ln`, `exp`, ...) stay native, behind the
  FFI boundary. Differential-tested against `math`: exact equality for the
  integer/rational/rounding operations (all matched on the first run,
  including negative-number rounding semantics), and a tolerance for `sqrt`
  (Newton's method converges to within ~1 ulp of the correctly-rounded
  native sqrt). `:help mathx` lists it and points at `:help math.<fn>`.


- **Embedded olang-source stdlib modules ("builtin packages").** A stdlib
  module can now be written in olang and compiled into the binary via
  `include_str!`, resolving like a package (`use colx { ... }`) with no file
  on disk. This proves the pure (non-FFI) parts of the stdlib can be
  self-hosted while native primitives (fs, http, crypto, db, ...) stay Rust.
  The first embedded module, `colx`, mirrors part of the native `col`
  collections module and is differential-tested against it — every
  olang-implemented function must agree with its Rust counterpart. (That
  test immediately caught a bug: `take_while` relied on mutating a
  closure-captured flag, which olang closures don't propagate; it now
  threads state through the fold accumulator purely.)


- **Package manager.** olang projects are now packages: an `olang.toml`
  manifest plus a directory of `.ol` files. Dependencies come as source in
  three forms — local `path`, `git` (tag/rev/branch), and registry version
  requirements — and resolve through the same `use` mechanism as local
  modules (a `use` whose first segment is a dependency name resolves inside
  that dependency). `otc pkg` gains `init`, `add`, `remove`, `install`,
  `tree`, and `publish`. Running a file inside a package resolves its
  dependencies automatically.
  - **Lockfile** (`olang.lock`) pins every dependency exactly (git SHA /
    registry version / path) with a sha256 source checksum; `--frozen`
    fails CI if resolution would drift.
  - **Version resolution** uses Minimal Version Selection (Go-modules
    style): the lowest version satisfying every requirement across the
    graph — reproducible, no backtracking, explicit upgrades.
  - **Registry** is a git repo of TOML index entries (name@version -> git
    source + checksum); no hosted service required. Fetched sources are
    content-addressed by commit under `~/.olang/cache`.
  - New `pkg` module in the library (`manifest`, `lock`, `cache`,
    `registry`, `resolver`) and `OLANG_REGISTRY` / `OLANG_CACHE` env vars.

- **Trait bounds.** A generic function can constrain its type parameters:
  `fn describe<T: Show>(item: T)` accepts only arguments whose type
  implements `Show`, checked at the call boundary — a value that doesn't
  fails there with a clear message (`argument 1 of type Circle does not
  implement trait Show`) instead of deep inside the body. Multiple bounds
  with `+` (`<T: Show + Ord>`) require all of them; the error names the
  specific unmet trait. Unbounded generics (`fn identity<T>(x: T)`) are
  unaffected. New `implements(value, "Trait")` builtin reports membership.
  Bounds are enforced at runtime — olang stays dynamically typed — which
  keeps this independent of the (opt-in) static type checker.

- **Traits with runtime dispatch.** `trait Show { fn show(self) -> String }`
  declares a set of methods (with optional default bodies); `impl Show for
  Point { ... }` provides them for a type. A method call `value.method(args)`
  dispatches on the runtime type of `value`, passed as `self` — single-
  dispatch polymorphism (protocols / interfaces). Default methods, methods
  with arguments, and traits over both structs and enums all work; struct
  fields take precedence over methods of the same name. This gives olang
  interface-based polymorphism without static typing.

- **`07_algebraic_types.ol`** example — a recursive expression-tree
  evaluator, a generic Option combinator library, and a binary search tree,
  all built on the enum sum types that now construct. A demonstration that
  olang expresses real algebraic data types.

- **Enums construct at runtime.** `type Color = enum { Red, RGB(Int, Int, Int) }`
  now binds its variants: unit variants (`Red`) are values, payload variants
  (`RGB(1, 2, 3)`) are constructor callables that build an `Enum` value. This
  makes olang's algebraic data types real — previously `enum` declarations
  parsed and could be pattern-matched, but the variants were undefined at
  runtime, so examples worked around them with string-tagged structs. Enum
  values compare structurally, `typeof` returns the enum name, and generic
  enums (`Maybe<T>`) construct for any payload (type parameters are erased at
  runtime).

- **`db` module** — an embedded SQLite database via the bundled `rusqlite`
  (compiled from source, so the single-binary story holds — no system
  dependency). `db.open` (`:memory:` or a file path), `db.execute` (rows
  affected), `db.query` (list of column-name maps), `db.query_one`, and
  `db.close`. Query parameters bind to `?` placeholders — the safe path is
  the default. Open connections live in a global registry keyed by id (the
  pattern the RNG and promise registries already use), returned as a
  `Connection` handle. SQLite types map to olang as NULL→unit,
  INTEGER→Int, REAL→Float, TEXT→String.
- **`06_database.ol`** example — a SQLite-backed task tracker: schema,
  parameterized seeding, filtered queries, group-by aggregates, and
  updates. Registered in the example harness and documented.

- **`col` module** — the first higher-order stdlib module: `min_by`,
  `max_by`, `sort_by`, `count_by`, `frequencies`, `partition`, `flat_map`,
  `take_while`, `drop_while`, `all`, `any`, `sum_by`, `unique`, `window`,
  `zip_with`, `last`. These take function arguments and call back into the
  interpreter (dispatched through `BuiltinFunctions::call`, which has the
  interpreter), so a stdlib module can now be higher-order. The core
  operations (`map`, `filter`, `fold`, `group_by`, ...) remain top-level
  builtins.
- **REPL completeness for all stdlib modules**: TAB completion now
  enumerates every registered module, so `str.`, `re.`, `col.`, and every
  existing `module.function` complete automatically (a new module is picked
  up with no manual list). Help docs added for all `str` (29), `re` (8),
  and `col` (16) functions — `:help str.trim`, `:help col.min_by`, and the
  `String`/`Regex`/`Collections` category listings all resolve.

- **`str` module** — string manipulation: case conversion, trim, split/join,
  replace, substring, pad, index/search, repeat, char access, lines/words,
  and `parse_int`/`parse_float`. All indexing is by Unicode character.
- **`re` module** — regular expressions backed by the `regex` crate:
  `is_match`, `find`, `find_all`, `captures`, `split`, `replace`,
  `replace_all`, and `is_valid`. A malformed pattern is a recoverable `Err`,
  not a crash.
- **`05_text_processing.ol`** example — word frequency, log parsing via
  regex captures, email extraction, phone validation, a template engine,
  record parsing, and slugification.

- A curated example suite (`examples/01_language_tour.ol` through
  `04_stdlib_showcase.ol`, indexed in `examples/README.md`) that runs top to
  bottom and prints computed results — a language tour, a real analytics
  pipeline, classic algorithms, and a stdlib showcase (SHA-256/HMAC, bcrypt,
  RSA sign/verify, math, calendar arithmetic, JSON). Verified in CI by
  `tests/example_programs_test.rs`, so the examples cannot rot. Retired four
  stale examples tied to deleted subsystems (three SIMD demos, one GC memory
  test).
- Formatting and clippy are blocking CI gates: the tree is rustfmt-clean and
  clippy-clean at zero warnings (`-D warnings`). The one deliberate allowance
  is `clippy::result_large_err` — boxing the interpreter's error enum is a
  worthwhile future refactor tracked in `src/lib.rs`. The whole-tree reformat
  commit is listed in `.git-blame-ignore-revs`.

- **Documentation examples are tested in CI** (`tests/doc_examples_test.rs`):
  every ```olang block in README.md and docs/syntax.md must parse and run
  (```olang no-run blocks — needing files, network, or modules — must at
  least parse). 41 of the 61 documented examples were broken when this was
  introduced; all are fixed. docs/syntax.md gains sections for character
  literals, `Promise.all`/`Promise.race`/`spawn`, and the reserved-word
  list.

### Changed

- **`colx` now mirrors all of `col`.** The embedded olang collections module
  gained the remaining nine functions (`min_by`, `max_by`, `sort_by`,
  `drop_while`, `flat_map`, `frequencies`, `last`, `window`, `zip_with`),
  reaching full parity with the native `col` module — including an olang
  insertion sort for `sort_by`. Every function is differential-tested
  against its Rust counterpart (16 functions, with key-function, stability,
  and truncation cases), and a parity test asserts the mirror stays
  complete.


- **Stdlib return-type convention unified**: total operations (which cannot
  fail for correctly-typed input) return bare values; fallible operations
  return an olang `Result`. Concretely: `crypto` hashes/HMAC/`secure_compare`
  and `base64.encode` now return values directly instead of `Ok(...)`, and
  the `dates` module now returns `Result` for every date-string operation
  (parsing, arithmetic, formatting, component extraction) — a malformed date
  is a recoverable `Err` instead of aborting the program. Total date
  operations (`now`, `today`, `is_leap_year`, `days_in_month`) stay bare.
  `docs/syntax.md` documents the convention and both new modules.

- **Fast by default** — the bytecode tier is now enabled by default,
  promoting eligible functions on their *first* call (previously opt-in via
  `--ovm-tier` with a 50-call threshold, which also meant a hot loop inside
  a function called once never promoted). The slow "OVM routing" layer is
  no longer the default execution path — it measured ~70% slower than the
  plain interpreter, meaning the out-of-the-box configuration was the
  slowest available. Zero-flag results: fib(32) 4.5 s → 2.1 s; a
  3M-iteration loop 1.30 s → 0.29 s. `--no-ovm` remains the pure
  tree-walking escape hatch; `--ovm-tier=N` raises the threshold.
- **Slot resolution** — identifiers in function and lambda bodies are
  resolved to frame-slot indices once at declaration time (`src/resolve.rs`),
  so the hot path indexes into the frame instead of probing names. Every
  resolved reference keeps its name and the runtime verifies the slot before
  using it, falling back to a normal lookup on mismatch — a stale static
  model (e.g. a `let` inside a conditional shifting later slots) costs
  speed, never correctness. 5–9% across call- and loop-heavy benchmarks.
  Stale bytecode-cache files from older AST shapes are now removed quietly
  instead of warning on every startup.
- **Frame-based environments** — environments created for function calls,
  loop bodies, match arms, and catch blocks are now frames: every binding
  they create (`let`, loop variables, match bindings) lives in the probed
  locals vector with in-place rebinding, instead of being hashed into the
  persistent map. Only the root environment keeps map storage, so top-level
  definitions still persist across REPL inputs and snapshot into closures in
  O(1). A 3M-iteration loop with three `let`s per iteration runs ~33%
  faster; `for` iteration ~19% faster.
- **Interpreter call path rewritten** — function calls are ~50x cheaper
  (~0.65 µs, down from ~33 µs). Three changes: the call environment adopts
  the function's closure as a shared persistent map in O(1) instead of
  copying every entry; body evaluation swaps environments instead of
  overlaying and then restoring every closure entry around each call; and
  call-frame bindings (parameters, the function's own name) live in a
  probed vector instead of being hashed into the persistent map.
  `recursive_fib_13` benchmark: 24.5 ms → 0.31 ms. This collapses the
  bytecode tier's relative advantage on call-heavy code (its headline
  numbers were largely measuring interpreter overhead); loop-heavy code
  still benefits ~7.5x from the tier.

### Fixed

- **Newlines now separate statements.** Previously a newline was plain
  whitespace, so a parenthesized expression on the line after a statement
  attached as a call to the previous value — `let a = [1,2]` then `(a, a)`
  parsed as `[1,2](a, a)`, breaking tuple returns and any bare `(...)`
  statement. Newlines are now statement separators, admitted explicitly at
  continuation points (operator chains, pipelines, bracketed lists, match
  arms, bodies after `=`), so multi-line pipelines and calls are unchanged.
  Trailing commas in list literals are now allowed too, consistent with
  maps/structs/enums.


- **Unit-variant patterns dispatch correctly.** A bare name in a `match`
  arm that resolves to a unit enum variant is now matched as a variant, not
  bound as a catch-all — previously the first such arm (`North => ...`)
  captured every case.

- `to_int` and `to_float` are callable as plain identifiers. They were
  handled by the builtin dispatcher but never registered in the builtin
  function map, so `to_float(x)` failed with "undefined variable" outside
  the bytecode tier — a real bug surfaced while writing the example
  programs.

- Zero-parameter lambdas (`() => 3`) parse — the grammar always allowed
  them, but the parser discarded the body when no parameter list was
  present.
- Documentation no longer claims struct field shorthand, intersection
  types, union type declarations, or error-variant payloads — none of
  which parse. `Promise.delay`'s documented argument order was backwards
  (it is `Promise.delay(value, ms)`).


- Lambda capture no longer depends on `im::HashMap::union`, whose collision
  bias depends on which map is larger — a captured variable could resolve to
  an ancestor call frame's stale value, sending recursion through a captured
  lambda into infinite loops.

### Removed

- **~12,000 lines of dead execution machinery**: the `OlangVirtualMachine`
  routing layer and `ovm_integration` (the pre-0.24 default path, measured
  ~70% slower than the plain interpreter), `ovm_repl`, the placeholder JIT
  module, the tracing-GC/region-allocator remnants, and the pipeline, SIMD,
  lazy, fusion, and adaptive engines — none wired into execution. The OVM
  directory now contains exactly what runs: the bytecode VM, the tier, the
  value model, and safepoint flags. REPL commands `:ovm`, `:stats`, and
  `:memory` now report bytecode-tier statistics; `:gc` is gone (values are
  reference-counted; there is nothing to force).

## [0.23.0] - 2026-08-04

First stabilized release after a substantial correctness and performance
overhaul. The tree-walking interpreter remains the semantics reference; the
opt-in bytecode tier (`--ovm-tier`) is now honest, tested, and fast.

### Added

- **Bytecode tier** (`--ovm-tier[=N]`): hot user functions are promoted to a
  register-based bytecode VM after N calls (default 50), with transparent
  fallback — anything the VM cannot compile keeps running on the interpreter,
  so enabling the tier can never change program behavior. Covers arithmetic,
  comparisons, control flow, `while`/`for` loops with `break`/`continue`,
  `match` with guards, or-patterns, ranges, and destructuring of `Ok`/`Err`,
  lists (including `...rest`), and tuples, `Ok`/`Err` construction, pipelines,
  transitive and mutual recursion, 43 delegated builtins, and lambdas whose
  free variables resolve through the enclosing function's declaration-time
  closure. Measured speedups: ~120x on recursive calls, ~44x on match-heavy
  loops, ~26x on map/filter/fold pipelines.
- **Differential test suites** (110 tests) asserting the VM and the tier are
  observationally identical to the interpreter, including error cases.
- **REPL shell integration**: `:sh <cmd>` / `!<cmd>` run shell commands
  through `$SHELL`; `:cd`, `:pwd`, `:ls` navigate the filesystem (`cd`
  changes the REPL's own working directory). TAB completion for REPL
  commands, file paths (including inside string literals), and
  function/variable identifiers.
- **Interpreter benchmarks** (`cargo bench`) and a tier comparison harness
  (`cargo run --release --example tier_compare`).
- Continuous integration on Linux and macOS.

### Changed

- **Evaluation is by reference**: the interpreter no longer deep-clones AST
  subtrees on every loop iteration and function call.
- **OVM value model is reference-counted** (`Arc` payloads). The previous
  raw-pointer scheme leaked every allocation and its tracing GC operated on
  an object model the allocator never created; `gc.rs`/`memory.rs` are now
  accounting layers only.
- `--ovm-tier` takes its value with `=` (`--ovm-tier=10`) so the bare flag
  doesn't swallow the following filename.
- Documentation (README, `docs/ovm.md`) rewritten to state explicitly what is
  and is not implemented, with measured performance numbers.

### Fixed

- `break` and `continue` are real control flow; previously they were fake
  runtime errors that aborted the program (`loop { break }` could never
  terminate).
- Assignments inside `for` loop bodies write through to the enclosing scope
  instead of being silently discarded.
- Pipeline partial application (`5 |> add(3)`) resolves against the remaining
  parameters instead of erroring.
- Integer arithmetic is checked everywhere; overflow raises a runtime error
  instead of panicking (debug) or wrapping (release).
- `sum` no longer double-counts when a list mixes integers and floats on the
  parallel path; `map_filtered` no longer swaps its predicate and mapper;
  eager `concat` no longer returns a nested pair; `len` is character-based to
  match string indexing.
- Parser: identifiers with keyword prefixes (`match_count`) parse; numeric
  literals with trailing junk are rejected instead of silently splitting;
  string patterns process escape sequences; postfix operators after
  `Ok(...)`/`Err(...)` are kept; error snippets truncate on character
  boundaries instead of panicking on multibyte text.
- `Promise.delay` is awaitable: the promise carries its deadline and `await`
  sleeps out the remainder. Previously every await of a delayed promise
  errored and each delay leaked an async-runtime registry entry.
- Static analysis: exiting a scope restores the outer-scope entries it
  shadowed instead of deleting them; `let`-bound variables get tracking
  entries so usage counting and unused-variable reports include them;
  guarded patterns no longer count toward match exhaustiveness; enum
  exhaustiveness consults the actual declaration.
- Type checker: recursive functions type-check (the function is registered
  before its body is checked); `Unknown` types satisfy type-class constraints
  (gradual typing).
- REPL: no longer panics on multibyte strings in `:env` previews or
  did-you-mean suggestions.
- `dates` extractors accept the module's own `now()` output; negative
  `add_days` and large `add_months` are correct; `math.min`/`max` preserve
  integer precision; `csv.to_json`/`from_json` use a real JSON serializer;
  `random` functions reject negative counts and handle inclusive ranges
  correctly.
- Thread safety: `Expr` uses `Arc` internally and the unsound
  `unsafe impl Send/Sync for Value` is removed.

### Removed

- The placeholder JIT tier, which returned constant integers cast to
  pointers and dereferenced them (undefined behavior). Cranelift remains a
  dependency for an eventual real implementation.
- Broken bytecode optimization passes (dead-code elimination that deleted
  live control flow, register renaming that rewrote three opcodes,
  control-flow optimization that treated label ids as addresses).

### Security

- `crypto.sign_data`/`crypto.verify_signature` are real RSA PKCS#1 v1.5
  signatures over SHA-256. The previous implementation ignored the key
  arguments entirely and produced forgeable output any party could
  construct.
- `crypto.decrypt_aes` accepts the output of `crypto.encrypt_aes` directly
  (the embedded nonce is parsed rather than requiring manual hex slicing).

[Unreleased]: https://github.com/ooyeku/olang/compare/v0.77.0...HEAD
[0.77.0]: https://github.com/ooyeku/olang/compare/v0.76.0...v0.77.0
[0.76.0]: https://github.com/ooyeku/olang/compare/v0.75.0...v0.76.0
[0.75.0]: https://github.com/ooyeku/olang/compare/v0.74.0...v0.75.0
[0.74.0]: https://github.com/ooyeku/olang/compare/v0.73.0...v0.74.0
[0.73.0]: https://github.com/ooyeku/olang/compare/v0.72.0...v0.73.0
[0.72.0]: https://github.com/ooyeku/olang/compare/v0.71.0...v0.72.0
[0.71.0]: https://github.com/ooyeku/olang/compare/v0.70.0...v0.71.0
[0.70.0]: https://github.com/ooyeku/olang/compare/v0.67.0...v0.70.0
[0.67.0]: https://github.com/ooyeku/olang/compare/v0.66.0...v0.67.0
[0.66.0]: https://github.com/ooyeku/olang/compare/v0.65.0...v0.66.0
[0.65.0]: https://github.com/ooyeku/olang/compare/v0.64.0...v0.65.0
[0.64.0]: https://github.com/ooyeku/olang/compare/v0.63.0...v0.64.0
[0.63.0]: https://github.com/ooyeku/olang/compare/v0.62.0...v0.63.0
[0.62.0]: https://github.com/ooyeku/olang/compare/v0.61.0...v0.62.0
[0.61.0]: https://github.com/ooyeku/olang/compare/v0.60.0...v0.61.0
[0.60.0]: https://github.com/ooyeku/olang/compare/v0.59.0...v0.60.0
[0.59.0]: https://github.com/ooyeku/olang/compare/v0.58.0...v0.59.0
[0.23.0]: https://github.com/ooyeku/olang/releases/tag/v0.23.0

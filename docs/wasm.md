# olang in the browser

Part of [the olang book](README.md) · [A tour of olang](tour.md) ·
[Language reference](language.md) · [Standard library reference](stdlib.md) ·
[The data stack](ods.md)

olang compiles to WebAssembly, so the same language can be used for both a
web server and the page it serves. A single olang process can serve an API,
serve the page, and serve the frontend's olang source; the browser loads the
language as WebAssembly and runs that source against the DOM. The frontend
and the backend are then two programs written in one language, one of which
runs in the browser.

This chapter describes the architecture that supports this, the `dom` module
that makes a page programmable, the patterns used to write browser olang,
and a reading of a complete frontend. Code blocks that drive a browser are
marked `no-run`: they are parse-checked by the test suite but need a page to
execute. The code they show runs in [`examples/app/`](../examples/app/).

## Table of contents

- [The same language, end to end](#the-same-language-end-to-end)
- [The architecture](#the-architecture)
- [The `dom` module](#the-dom-module)
- [The patterns](#the-patterns)
- [Graphics: the draw-list](#graphics-the-draw-list)
- [Declarative views: the `ui` module](#declarative-views-the-ui-module)
- [Routing and storage](#routing-and-storage)
- [Workers: a second olang, off the main thread](#workers-a-second-olang-off-the-main-thread)
- [Reading a real frontend](#reading-a-real-frontend)
- [Running it](#running-it)
- [The sandbox and its limits](#the-sandbox-and-its-limits)
- [Choosing this architecture](#choosing-this-architecture)

## The same language, end to end

[`examples/app/`](../examples/app/) is an issue tracker: a SQLite
store behind a validated JSON API, with a spreadsheet-style grid in
the browser. Start it and look at what the one process serves:

| Route | What it delivers |
|---|---|
| `GET /` | the page: static HTML with empty containers |
| `GET /olang-dom.js` | a small JavaScript shim (the *only* JS in the system) |
| `GET /app.ol` | the frontend — olang source, served as plain text |
| `GET /olang.wasm` | the olang language itself, compiled to WebAssembly |
| `GET /api/...` | the JSON API, handled by the same `http.serve` |

Three more pages use the same shim and link to each other through a shared
navigation bar: `/orbit.html` (an animated canvas scene), `/notes.html` (a
routed single-page application with localStorage persistence), and
`/primes.html` (a Web Worker running a second olang instance). Each page also
serves its `.ol` source as plain text, so the source of every demo can be
viewed directly.

The browser fetches the wasm build and the olang source, instantiates
the language, and runs `app.ol`. From that moment the page is driven
by olang: rendering, event handling, and API calls are all olang
functions executing in the browser. The shim is not a framework — it
is a loader and a device driver, and it never grows, because
everything application-shaped lives in `app.ol`.

The payoff is uniformity. The same `Result` handling, the same
`json.parse`, the same pipelines and pattern matching work on both
sides of the wire, and moving logic between client and server is a
cut-and-paste, not a rewrite. What runs in the browser is not a
lookalike or a subset compiled specially for the occasion — it is the
same interpreter and bytecode tier as the CLI, so a function debugged
at the REPL behaves identically inside the page.

## The architecture

Three layers, each with a single job.

### The wasm build

The engine is an ordinary build of the olang crate for the
`wasm32-unknown-unknown` target, packaged by the small
[`playground/`](../playground/) crate as a cdylib. It contains the
parser, the interpreter, the bytecode tier, and the full standard
library minus the operating-system modules — including the entire
[data stack](ods.md), which is pure Rust precisely so it would survive
this build.

Notably absent: `wasm-bindgen` or any binding generator. The boundary
is a hand-rolled C ABI of a few exported functions, because the
interface is genuinely small and a generator would add a build step,
generated glue, and a second place where types are defined, in
exchange for automating about a dozen signatures:

| Export | Contract |
|---|---|
| `olang_alloc(len)` / `olang_dealloc(ptr, len)` | the page allocates buffers inside wasm linear memory and writes UTF-8 into them |
| `olang_run(ptr, len)` | parse and evaluate source; returns a result buffer |
| `olang_session_start(ptr, len)` | run source and **keep the interpreter alive** as the page's session |
| `olang_dispatch_event(id)` / `olang_dispatch_event_with(id, ptr, len)` | re-enter the live session for one event, optionally carrying a string payload |
| `olang_dispatch_event_json(id, ptr, len)` | the structured variant: the payload is JSON, parsed into the Map the handler receives |
| `olang_result_free(ptr)` | free a result buffer |

Every call answers with the same shape: a length-prefixed JSON buffer
carrying `output` (everything the program printed), `value`, `error`,
and timing. Strings cross the boundary as `(pointer, length)` pairs
into linear memory in both directions. That is the entire protocol.

### The host imports

A wasm instance can only do what its imports allow, and this one
imports almost nothing: two clocks (`time` needs them), one entropy
source (`random` needs it), and — when the page enables the frontend
role — the `dom` operations, one import per primitive. Each import is a few lines of
JavaScript in the shim implementing one primitive over the real DOM:
`host_dom_query` runs `document.querySelector`, `host_dom_set_html`
assigns `innerHTML`, and so on. Elements cross the boundary as opaque
integer handles the shim mints; olang never holds a DOM object, only a
number naming one.

This is the security story as much as the plumbing story: the
capability list *is* the import list, so what browser olang can touch
is auditable in one screen of JavaScript.

### The persistent session and event re-entry

A frontend is not a script that runs and exits — it must outlive its
own boot to answer events. `olang_session_start` therefore parks the
interpreter, still warm, after the program's top level finishes. When
the program registered a handler with `dom.on`, the handler function
was stored in the session's registry under an integer id, and the shim
attached a real DOM listener that calls back into
`olang_dispatch_event_json(id, payload)` with a JSON event object,
parsed on the wasm side into the Map the handler receives. The event
path is:

```text
DOM event → shim listener → dispatch_event_json(id, {type, id, value, …})
        → the parked interpreter calls handler(event_map)
        → the handler queries, fetches, re-renders
```

The same mechanism carries asynchrony. `dom.fetch` hands the shim a
callback id and returns immediately; the shim performs the browser
`fetch`, and when the response arrives it dispatches the body text to
the stored callback. olang needs no event loop of its own — the
browser's loop drives it, one re-entry at a time.

## The `dom` module

The module's philosophy is the opposite of a widget toolkit's: olang
does not wrap the DOM object model — it treats the page as a
*rendering target with controls*. You query elements, wire events,
render by writing HTML, reach for node-level operations when a full
re-render would be too blunt, draw scenes onto canvases, and schedule
work with timers and animation frames. Everything else is ordinary
olang. The full surface, grouped:

| Group | Functions |
|---|---|
| Query & content | `query`, `get_text` / `set_text`, `set_html`, `value` / `set_value`, `focus` |
| Events | `on(el, event, handler)` — any DOM event name, plus the `enter` alias |
| Attributes & style | `get_attr` / `set_attr` / `remove_attr`, `set_class`, `class_add` / `class_remove` / `class_toggle`, `set_style`, `measure` |
| Structure | `create`, `append`, `remove`, `insert_before`, `scroll_into_view` |
| Timers & frames | `set_timeout`, `set_interval` / `clear_interval`, `request_frame`, `on_frame` |
| Graphics | `draw(canvas, ops)` — the draw-list ([below](#graphics-the-draw-list)) |
| Navigation & storage | `push_state`, `location`, `on_route`, `storage_get` / `storage_set` / `storage_remove` |
| HTTP | `fetch(method, path, body, callback)`, `fetch_json(…)` |
| Workers | `worker`, `worker_send` / `worker_on`, `worker_close`; inside a worker: `post` / `on_message` |

Per-function signatures live in the
[stdlib reference](stdlib.md#dom--the-browser); this chapter is about
how they compose.

`dom` is the one browser-only module: in a native build every call
reports that it needs the wasm build — the mirror image of `fs`, `os`,
`http`, and `db`, which exist natively and are absent from the browser.

### Elements are handles

`dom.query` returns an opaque handle — pass it back into the other
functions; there is nothing else to do with it. A handle stays
attached to the specific element it named, so a handle taken *before*
a `set_html` re-render points at a node that no longer exists
afterwards. The discipline that follows: query fresh handles inside
handlers, at the moment of use, rather than caching them at startup.

```olang no-run
dom.set_text(dom.query("#title"), "olang was here")
dom.set_html(dom.query("#list"),
    ["a", "b"] |> map((s) => "<li>" + s + "</li>") |> join(""))
```

### Events are structured Maps

`dom.on(el, event, handler)` registers a handler for any DOM event
name — `click`, `input`, `keydown`, `pointermove`, `submit`, `wheel`,
… — plus `enter`, the keydown-filtered alias, because "text field +
Enter" is the fundamental input gesture and wiring it by hand is
boilerplate. Every handler receives the **same event Map** regardless
of event type, so handlers pick the fields they need:

| Field | Contents |
|---|---|
| `type` | the event name that fired |
| `id`, `value` | the *target* element's id and current value |
| `key` | the key pressed, for keyboard events |
| `x`, `y` | pointer coordinates (client space — pair with `dom.measure` for element space) |
| `alt`, `ctrl`, `shift`, `meta` | modifier flags |
| `data` | a Map of the target's `data-*` attributes |

One shape, one convention to learn. The target's `id` and `data`
arrive even when the listener sits on an ancestor — which makes event
delegation (next section) the natural style rather than an advanced
technique:

```olang no-run
dom.on(dom.query("#rows"), "input", (e) => {
    update_item(map_get(e, "id"), map_get(e, "value"))
})
dom.on(dom.query("#sky"), "click", (e) => {
    let rect = dom.measure(dom.query("#sky"))
    add_body(map_get(e, "x") - map_get(rect, "x"),
             map_get(e, "y") - map_get(rect, "y"))
})
```

### `dom.fetch`

`dom.fetch(method, path, body, callback)` issues the request through
the browser and returns immediately; when the response arrives, the
callback receives its **body text** — parse it with `json.parse` if it
is JSON. A non-empty body is sent with a JSON content type. A network
failure delivers `{"error": "..."}`, so one `map_has_key` check covers
the failure path uniformly. There is deliberately no status code in
the callback: design the API so the body says what happened, and the
client stays one honest branch.

```olang no-run
dom.fetch("GET", "/api/issues", "", (resp) => {
    let parsed = unwrap(json.parse(resp))
    if map_has_key(parsed, "error") => show_error(map_get(parsed, "error"))
    else => render(map_get(parsed, "items"))
})
```

When the response *is* JSON — which for an API client is nearly
always — `dom.fetch_json` removes the parse step: the callback
receives the parsed value directly, and the error convention is
unchanged:

```olang no-run
dom.fetch_json("GET", "/api/issues", "", (parsed) => {
    if map_has_key(parsed, "error") => show_error(map_get(parsed, "error"))
    else => render(map_get(parsed, "items"))
})
```

## The patterns

The module's functions are a vocabulary; these three patterns are the
grammar. Together they are why a browser olang program stays a few
screens long.

### Stateless frontends

Recall from the language reference that olang closures
[capture by value](language.md#closures-capture-by-value): an event
handler that wrote to a module-level `let mut items` would update its
own snapshot and lose the write. Rather than fight that rule, the
browser architecture embraces it — a dom frontend keeps **no state in
the program at all**. The server is the source of truth for data; the
DOM itself holds the current value of every cell; and each event runs
the same loop:

```text
event → dom.fetch mutation → callback → reload() → GET → render() → one set_html
```

Note what this deletes: no model objects, no store, no cache
invalidation, no synchronization between a data structure and the
screen. When a handler needs a current value, it reads it back from
the page — a status *is* its button's label, one `dom.get_text` away.
The pattern costs one round trip per action and buys total freedom
from state bugs; for the tools-and-dashboards class of application
this module targets, that is the right trade.

### Event delegation

The naive way to wire a table is a listener per interactive cell —
which means rebinding everything after every re-render, a growing
handler registry, and the sluggishness that follows. The delegated
way exploits how events bubble and how the payload conventions carry
the target: attach **one** listener to the container, give the
elements inside it ids that encode their action (`del-17`, `adv-17`),
and dispatch on the prefix.

The tracker's whole grid — status cycling, priority cycling, assignee
edits, points edits, deletion, opening the detail drawer, for every
row — runs on two listeners bound once at boot; filters, sort headers,
and the drawer's own controls each add one more on their containers:

```olang no-run
dom.on(dom.query("#rows"), "click", (e) => on_rows_click(map_get(e, "id")))
dom.on(dom.query("#rows"), "change", (e) => on_rows_change(e))
dom.on(dom.query("#filters"), "click", (e) => on_filter_click(map_get(e, "id")))
dom.on(dom.query("#sort-row"), "click", (e) => on_sort_click(map_get(e, "id")))
```

Rows can be re-rendered any number of times — the listeners sit on
the containers, not on the short-lived rows, so nothing needs
rebinding and the registry never grows.

### Escape what you render

`dom.set_html` is the render primitive, and it renders whatever it is
given — so any user-entered text that reaches it must be escaped, or
a title like `<img onerror=...>` becomes code. The discipline is one
small function applied at every interpolation of untrusted content:

```olang
fn esc(s) = s
    |> str.replace("&", "&amp;")
    |> str.replace("<", "&lt;")
    |> str.replace(">", "&gt;")
    |> str.replace("\"", "&quot;")

println(esc("<b>bold & \"quoted\"</b>"))
```

Ampersand first (or the other replacements would be double-escaped),
quotes too (the text may land inside an attribute). This is the same
rule every server-side templating system enforces; in the browser the
stakes are the same and the mechanism is yours. When you adopt the
[`ui` module](#declarative-views-the-ui-module), this discipline comes
built in: `ui.esc` is this function, and `ui.html` applies it to every
text node and attribute automatically.

## Graphics: the draw-list

A scene is plain olang data: a list of op maps, submitted with **one**
`dom.draw(canvas, ops)` call, replayed by the shim onto the canvas 2D
context. The whole frame crosses the wasm boundary once, however many
shapes it holds:

```olang no-run
dom.on_frame((f) => {
    let t = map_get(f, "delta") / 1000.0
    dom.draw(sky, [
        #{ "op": "clear", "color": "rgba(11,14,20,0.35)" },
        #{ "op": "circle", "x": 360.0, "y": 240.0, "r": 16.0, "fill": "#f5c542" },
        #{ "op": "line", "x1": 0.0, "y1": 0.0, "x2": 60.0, "y2": 80.0,
           "stroke": "#fff", "line_width": 2 }
    ])
})
```

The ops: `clear` (with a color for motion trails, without to wipe),
`rect`, `circle`, `line`, `path` (a `points` list, optionally closed),
`text`, and the transforms `save` / `restore` / `translate` / `rotate`
/ `scale`. Fill and stroke take any CSS color.

Past a few thousand points, JSON itself becomes the cost — so bulk
data has its own lane. `dom.draw_points(canvas, xs, ys, style)` packs
the coordinates into **one binary f64 buffer** the page reads as a
zero-copy typed-array view: no serialization, no per-point boundary
crossing. `xs`/`ys` are ods Series (the fast lane — the data stack
feeds the graphics pipeline directly) or plain lists; nulls drop
pairwise. The style map picks `mode` (`"points"` or `"path"`),
`color`, `size`, `alpha` — plus a rotation `rot` and an affine
`sx`/`sy`/`tx`/`ty` applied *host-side*, which is the trick that
makes animation cheap: compute the data once, and every frame just
re-sends the same buffer with new transform parameters. The gallery
spins a 12,500-star galaxy on the rotation parameter alone and
animates a 50,000-point curtain at frame rate this way, and
`viz.draw` compiles its point and line marks onto this path
automatically.

The frame loop has two speeds. `dom.request_frame(fn)` is one-shot —
re-arm it inside the handler if you want another. `dom.on_frame(fn)`
is the persistent loop: register once at boot, and the handler runs
every frame with a millisecond `delta` for time-based motion (never
assume 16.6ms — the delta is real, and animation driven by it survives
a throttled background tab gracefully). Because handlers keep no
state (closures capture by value), animation state lives in the DOM
like everything else — the orbits demo keeps its bodies as JSON in a
hidden input, loading and saving each frame.

The host keeps the loop honest about power. All `on_frame` handlers
share one animation loop capped near 60fps (a 120Hz ProMotion display
doubles fill rate for no visible gain in a data animation), and
`dom.draw` / `dom.draw_points` skip painting entirely for a canvas
that is scrolled out of the viewport — the handler still runs (your
simulation time advances), but an offscreen canvas costs the GPU
nothing. A canvas can also trade retina sharpness for fill rate with
`data-olang-dpr="1.5"` (or `"1"`) in its markup: a per-frame animated
piece rarely needs a full devicePixelRatio backing store, and the
pixel cost falls with the square of the ratio.

See it whole in [`examples/app/static/orbit.ol`](../examples/app/static/orbit.ol),
served at `/orbit.html`: five bodies orbiting on trails, and a click
adds a new one at the clicked radius — structured event coordinates,
`dom.measure`, and the draw-list in ~100 lines.

Charts take the other road: the [data stack](ods.md) runs in this
build, and [`plot`](ods.md#plot--charts-as-svg-text) renders charts as
SVG *text* — so `dom.set_html(el, plot.line(...))` is a complete
rendering pipeline, with `theme: "dark"` and `responsive: true` making
the output drop into a page unstyled. One level up, the
[`viz` grammar](ods.md#the-viz-grammar) makes a chart a value: a spec
map of data, mark, and column encodings that compiles to plot SVG —
or, via `viz.draw(canvas, spec)`, to a draw-list for point counts SVG
can't carry. Interactivity is the same event delegation as everything
else: `"interactive": true` marks carry their datum as `data-*`
attributes, so `viz.tooltip`, `viz.on_mark`, and `viz.brush` ride the
structured-event path with zero new runtime. `/charts.html` draws
live tracker analytics that way (`fetch_json` delivers records,
records are what specs eat — hover any mark, click a status bar to
cross-filter), and `/gallery.html` is the standing showcase —
draw-lists for motion, plot and viz for statements, a brushable
forecast for zoom.

## Declarative views: the `ui` module

`dom.set_html` with string building is honest and fine at small scale,
but it has a cost the tracker pays deliberately: a re-render replaces
*everything* in the container, so focus and cursor position inside it
die. The `ui` module — an [embedded package](stdlib.md) written in
olang itself, loaded with `use ui` — is the next rung: build the page
as a *value*, and let reconciliation decide what actually changes.

```olang no-run
use ui { h, hk, render }

fn note_row(n) = hk(map_get(n, "id"), "div", #{ "class": "note" }, [
    h("span", #{}, [map_get(n, "text")]),
    h("button", #{ "id": "del-" + map_get(n, "id") }, ["x"])
])

fn draw() = render(dom.query("#list"), map(load_notes(), note_row))
```

`h(tag, attrs, children)` builds a node map; strings are text nodes,
escaped on render; `hk` is `h` plus a **reconciliation key**.
`ui.html(tree)` renders a tree to an HTML string — pure, no browser
needed, which is why the module's tests run natively. `ui.render(el,
children)` mounts a keyed list and diffs against what it rendered last
time: unchanged children are *not touched* (input state and focus
survive), changed ones re-render in place, added and removed keys
insert and remove surgically, and reorders reposition nodes without
rebuilding them. The memo travels in a `data-ui` attribute on the
mount element — the same DOM-resident-state discipline as everything
else, so `render` itself stays stateless.

The rule of thumb: string-build when a container is cheap to replace
wholesale; `ui.render` when the container holds user state (inputs,
focus, scroll) or when most of it doesn't change per event.

## Routing and storage

Two small surfaces make single-page applications honest. Navigation:
`dom.push_state(path)` updates the URL without a reload,
`dom.location()` reads it back as a Map of `path` and `query`, and
`dom.on_route(handler)` fires on back/forward with the same shape —
the URL becomes one more piece of DOM-resident state, bookmarkable and
back-button-correct. Persistence: `dom.storage_get` / `storage_set` /
`storage_remove` wrap localStorage (missing keys read as `""` — pair
with `json.parse` and `unwrap_or` for a default).

[`examples/app/static/notes.ol`](../examples/app/static/notes.ol),
served at `/notes.html`, composes all of it with `ui.render`: notes
persist across reloads, selecting one writes `?sel=` into the URL, and
the back button unselects — a complete SPA in ~80 lines.

## Workers: a second olang, off the main thread

The [sandbox section](#the-sandbox-and-its-limits) is honest that the
browser build runs interpreter-and-bytecode only — so what about
genuinely heavy compute? The browser's own answer is threads, and
`dom.worker` puts olang on them:

```olang no-run
let w = dom.worker("/primes-worker.ol")
dom.worker_on(w, (m) => show_progress(m))
dom.worker_send(w, #{ "upto": 200000 })
```

`dom.worker(path)` fetches an olang source file and boots it inside a
Web Worker: its own thread, its own wasm instance, and **no DOM** —
the worker program's whole surface is `dom.on_message(handler)` for
requests in and `dom.post(value)` for results out. Values cross as
plain maps and lists (anything `json.stringify` can carry). Closures
do not cross — *programs and messages do*, which is the same
discipline `chan` enforces natively; a worker is to the page what a
spawned task is to a native program.

The property that makes this more than an escape hatch: a worker may
`post` *mid-computation*, and the messages arrive as ordinary events
while the worker keeps grinding. Long jobs stream progress; the page's
frame loop never misses a beat. The demo at `/primes.html`
([`primes.ol`](../examples/app/static/primes.ol) /
[`primes-worker.ol`](../examples/app/static/primes-worker.ol)) makes
the property visible: the progress bar fills *during* the count while
an animation dial — driven by `dom.on_frame` on the main thread —
never stutters.

The worker side rides `olang-worker.js`, a page-served harness that
instantiates the wasm with inert DOM stubs (the playground sandbox's
posture) and bridges the two live imports over `postMessage`. Same
capability story as everything else: the import list is the whole
surface, and a worker's is two functions long.

## Reading a real frontend

[`examples/app/static/app.ol`](../examples/app/static/app.ol) is the
tracker's complete frontend: ~330 lines of olang for a full product —
live search, status filters, sortable columns, an issue drawer with
comments, a stats strip, and an activity ticker. Its JavaScript
predecessor needed 574 lines for a bare grid. Worth reading top to
bottom — here is the guided version.

**State lives in the DOM.** The client state that must exist — sort
column and order, the active status filter, the selected issue — is
four hidden inputs, read with `dom.value` and written with
`dom.set_value`. No olang variable outlives a handler; every render
derives everything fresh. `issues_url()` assembles the query string
from that state, and the *server* does the filtering, searching, and
sorting.

**Rendering is string building.** `row_html(issue)` renders one issue
as a `<tr>` — controls carry their prefixed ids (`open-`, `adv-`,
`pri-`, `asg-`, `pts-`, `del-` plus the issue id), the delegation
contract in action. `render_rows` lands the list with one `set_html`;
`render_stats` renders the `/api/stats` payload (whose quantiles the
server computes on the ods data stack); `render_activity` draws the
audit trail as a ticker; `render_drawer` fills the detail panel and
flips it visible with `dom.set_class` — the stateless way to toggle
visual state.

**The reload loop.** `reload_rows()` GETs `issues_url()` and
re-renders; `patch(id, body)` PATCHes one issue and, in its callback,
reloads — and re-opens the drawer when the patched issue is the
selected one. Every mutation funnels through this pair.

**The delegated dispatchers.** `on_rows_click` branches on the
target-id prefix from the event Map: `open-` opens the drawer, `adv-`
cycles status (reading the *current* status from the button's own
label — the DOM as state store), `pri-` cycles priority, `del-`
deletes. `on_rows_change` reads the target's `id` and `value` straight
from the event for assignee and points edits; `on_filter_click` and
`on_sort_click` drive the hidden state inputs and reload. Comments
POST and re-open the drawer.

**Boot.** Twelve `dom.on` registrations, each on an element present at boot,
followed by a first `reload()`.

The frontend uses no component classes, no virtual DOM, no state container,
and no lifecycle methods. The architecture described in this chapter — a
server-rendered page reloaded on change, with the DOM as the source of truth —
provides the structure those abstractions would otherwise supply, so the
program is roughly as long as its behavior requires.

Then read the companion pages in ascending order of machinery, each
linked from the tracker's header: `orbit.ol` (the draw-list and frame
loop, no HTML rendering at all), `notes.ol` (`ui.render`, routing,
storage — the SPA shape), `primes.ol` with `primes-worker.ol` (two
programs, one page — the parallelism shape), and the data-viz pair:
`charts.ol` (live analytics from `fetch_json` through ods frames into
plot SVG), `gallery.ol` (the visualization showcase), and `board.ol`
(the `dash` kit's flagship: KPI tiles, chart cards, a URL-carried
filter, and auto-refresh — a complete ops dashboard in one file).
Together with `app.ol` they exercise the module's entire surface, and
every one is served as source by the same process that serves its
page.

## Running it

The wasm artifact is built once and served as a static file. From the
repository root:

```bash
cargo build -p olang-playground --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/olang_playground.wasm examples/app/static/

cd examples/app
olang main.ol            # http://127.0.0.1:7317
```

(The target arrives via `rustup target add wasm32-unknown-unknown` if
it is not installed. `make wasm` from the repository root runs both
steps — and also stages the website playground's copy.) The tracker
checks for the artifact at boot and prints these exact commands if it
is missing.

On the server side, each frontend page is a handful of routes in
[`examples/app/main.ol`](../examples/app/main.ol) — the page, its
`.ol` source, and the shared shim — all serving text read at startup;
the wasm route uses `body_file`, which streams raw bytes from disk —
the response form for binary content:

```olang no-run
fn olang_wasm(req, params) = {
    status: 200,
    body_file: "static/olang_playground.wasm",
    headers: #{ "Content-Type": "application/wasm" }
}
```

One development-loop detail earns its mention: the text routes are
served with `Cache-Control: no-store`, so editing `app.ol` and
refreshing the browser is the whole iteration cycle — no build step,
no bundler, no cache to bust. The frontend is *source* until the
moment the page fetches it.

## The sandbox and its limits

Honesty about the boundaries, in both directions.

**Capability.** The wasm instance touches nothing but its own linear
memory and its explicit imports. `fs`, `os`, `http`, and `db` are
absent from the browser build — calling them reports exactly that —
and network access exists only as `dom.fetch` / `dom.fetch_json`,
which go through the browser and are therefore subject to the page's
same-origin rules like any other web request. A `dom.worker` instance
is the same sandbox again, minus the DOM: its import surface is
`post` and `on_message`, period. On the website's /playground, where
arbitrary code runs with no dom bridge at all, the page runs the
engine inside a Web Worker it terminates on timeout — which is what
bounds an infinite loop. Sandboxing comes from the platform, not from
trust in the engine.

**Speed.** The browser runs the interpreter and the bytecode tier —
the same execution model as the CLI's default. The third tier, the
[JIT](ovm.md#the-jit), does not exist in this build: it emits native
machine code, which has no meaning inside a wasm sandbox. Frontend
work — rendering, dispatching, fetching — never notices; it is
I/O-shaped and measured in DOM operations, not olang cycles. Heavy
numeric work is the case to watch, and the architecture holds two
answers: the server is the same language, one `dom.fetch` away, so
computation moves to where the fast tiers are without changing
languages — and [`dom.worker`](#workers-a-second-olang-off-the-main-thread)
moves it off the main thread when it must stay client-side, streaming
progress while the page stays live.

**Failure surfaces.** A parse or runtime error at boot renders into
the page; a handler error and everything the program prints go to the
browser console. `println` debugging works in the browser exactly as
it does natively — open the console.

## Choosing this architecture

Where browser olang is at its best today: applications shaped like
the tracker. Tools, dashboards, admin panels, internal apps — pages
with real interactivity backed by an API, where the stateless loop
fits naturally and rendering is honest HTML. In that shape it replaces
the entire frontend toolchain with one source file served as text, and
it keeps the whole system in one language.

The considered alternative — compiling olang *to JavaScript* — would
trade the runtime for reach: transpiled output could tap the browser's
own JIT. What that would cost is exactly what this architecture
guarantees: the wasm engine is the same tested interpreter as the CLI,
so browser behavior is native behavior by construction rather than by
a compiler's promise. A second backend is a second place semantics can
drift, and this book's execution-model chapters show how much
machinery exists to prevent precisely that. The direction the project
takes lives in [the roadmap](roadmap.md); the architecture described
here is the one that ships, and programs written to the `dom` module's
contract are the ones any future backend would have to honor.

---

Next: [The Data Stack](ods.md) — which runs, in full, inside the
build this chapter described — or [Internals](internals.md) for the
runtime underneath both.

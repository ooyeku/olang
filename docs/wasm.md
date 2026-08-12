# olang in the Browser

A web application normally means two languages: one for the server and
JavaScript for the page, with a translation layer — JSON shapes, DTOs,
duplicated validation — standing between them. olang compiles to
WebAssembly, and that changes the shape of the problem: **one language
end to end**. A single olang process serves the API, serves the page,
and serves the frontend's *olang source*; the browser loads the
language as wasm and runs that source against the real DOM. The
frontend and the backend are two programs in one language, one of
which happens to execute inside a browser.

This chapter tells that story whole: the architecture that makes it
work, the `dom` module that makes a page programmable, the patterns
that make browser olang simple rather than merely possible, and a
guided reading of a complete production-shaped frontend. Code blocks
that drive a browser are marked `no-run` — they are parse-checked by
the test suite but need a page to execute; everything they show is
running code from [`examples/app/`](../examples/app/).

Part of [the olang book](README.md) ·
[Tour](tour.md) · [Language](language.md) · [Stdlib](stdlib.md) ·
[The Data Stack](ods.md)

---

## Table of Contents

- [The same language, end to end](#the-same-language-end-to-end)
- [The architecture](#the-architecture)
- [The `dom` module](#the-dom-module)
- [The patterns](#the-patterns)
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
| `olang_result_free(ptr)` | free a result buffer |

Every call answers with the same shape: a length-prefixed JSON buffer
carrying `output` (everything the program printed), `value`, `error`,
and timing. Strings cross the boundary as `(pointer, length)` pairs
into linear memory in both directions. That is the entire protocol.

### The host imports

A wasm instance can only do what its imports allow, and this one
imports almost nothing: two clocks (`time` needs them), one entropy
source (`random` needs it), and — when the page enables the frontend
role — the nine `dom` operations. Each import is a few lines of
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
`olang_dispatch_event_with(id, payload)`. The event path is:

```text
DOM event → shim listener → dispatch_event(id, payload)
        → the parked interpreter calls handler(payload)
        → the handler queries, fetches, re-renders
```

The same mechanism carries asynchrony. `dom.fetch` hands the shim a
callback id and returns immediately; the shim performs the browser
`fetch`, and when the response arrives it dispatches the body text to
the stored callback. olang needs no event loop of its own — the
browser's loop drives it, one re-entry at a time.

## The `dom` module

Nine functions. The module is deliberately small because its
philosophy is the opposite of a widget toolkit's: olang does not wrap
the DOM object model — it treats the page as a *rendering target*.
You query elements, wire events, fetch data, and render by writing
HTML. Everything else is ordinary olang.

| Function | Description |
|---|---|
| `dom.query(sel)` | first element matching a CSS selector — an error if none matches |
| `dom.get_text(el)` / `dom.set_text(el, s)` | read / write an element's text content |
| `dom.set_html(el, html)` | replace an element's inner HTML — the render primitive |
| `dom.value(el)` / `dom.set_value(el, s)` | read / write a form control's value |
| `dom.focus(el)` | focus an element |
| `dom.set_class(el, classes)` | set an element's class list wholesale — the stateless way to toggle visual state (a drawer's `open`, a pill's `active`) |
| `dom.on(el, event, handler)` | attach an event handler |
| `dom.fetch(method, path, body, callback)` | asynchronous HTTP from the page |

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

### Events and their payloads

`dom.on(el, event, handler)` registers a handler. The handler receives
at most one argument — a **payload string** whose contents depend on
the event name (a zero-parameter handler simply ignores it):

| Event | Fires on | Payload |
|---|---|---|
| `"enter"` | the Enter key in that element | the element's current value |
| `"click"` | any click on or inside the element | the **id of the clicked target** (empty if it has none) |
| `"change"` | a change on or inside the element | the target's id and its new value, separated by a newline |
| any other name | that DOM event, verbatim | empty string |

Two of these conventions carry the module's whole event philosophy.
`"enter"` exists because "text field + Enter" is the fundamental input
gesture, and wiring `keydown` by hand for it is boilerplate. And
`"click"`/`"change"` deliver the *target's* id even when the listener
sits on an ancestor — which makes event delegation (next section) the
natural style rather than an advanced technique. The `"change"`
payload packs two facts into one string; splitting on the newline
recovers them:

```olang no-run
dom.on(dom.query("#rows"), "change", (payload) => {
    let parts = split(payload, "\n")     // [target id, new value]
    update_item(parts[0], parts[1])
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
dom.on(dom.query("#rows"), "click", (tid) => on_rows_click(tid))
dom.on(dom.query("#rows"), "change", (payload) => on_rows_change(payload))
dom.on(dom.query("#filters"), "click", (tid) => on_filter_click(tid))
dom.on(dom.query("#sort-row"), "click", (tid) => on_sort_click(tid))
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
stakes are the same and the mechanism is yours.

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

**The delegated dispatchers.** `on_rows_click(tid)` branches on the
target-id prefix: `open-` opens the drawer, `adv-` cycles status
(reading the *current* status from the button's own label — the DOM
as state store), `pri-` cycles priority, `del-` deletes.
`on_rows_change` splits its `id\nvalue` payload for assignee and
points edits; `on_filter_click` and `on_sort_click` drive the hidden
state inputs and reload. Comments POST and re-open the drawer.

**Boot.** Twelve `dom.on` registrations — every one on an element that
exists at boot — and a first `reload()`. There is no step five.

The exercise worth doing: skim the file and count what is *absent* —
no component classes, no virtual DOM, no state container, no
lifecycle. The architecture carries that weight, which is what lets
the program be only as long as its actual behavior.

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

On the server side, the frontend is four routes in
[`examples/app/main.ol`](../examples/app/main.ol). Three serve text
read at startup; the wasm route uses `body_file`, which streams raw
bytes from disk — the response form for binary content:

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
and network access exists only as `dom.fetch`, which goes through the
browser and is therefore subject to the page's same-origin rules like
any other web request. On the website's /playground, where arbitrary
code runs with no dom bridge at all, the page runs the engine inside a
Web Worker it terminates on timeout — which is what bounds an infinite
loop. Sandboxing comes from the platform, not from trust in the
engine.

**Speed.** The browser runs the interpreter and the bytecode tier —
the same execution model as the CLI's default. The third tier, the
[JIT](ovm.md#the-jit), does not exist in this build: it emits native
machine code, which has no meaning inside a wasm sandbox. Frontend
work — rendering, dispatching, fetching — never notices; it is
I/O-shaped and measured in DOM operations, not olang cycles. Heavy
numeric work is the case to watch, and the architecture already holds
the answer: the server is the same language, one `dom.fetch` away, so
computation moves to where the fast tiers are without changing
languages.

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

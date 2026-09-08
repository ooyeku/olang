# The web SDK

`frameworks/web-sdk` is the foundation layer for building full-stack
web applications in olang — one process serving a JSON API and a wasm
frontend, with one route table driving both sides. It is deliberately
two things at once: directly usable (a working app is two small
files), and foundational — the primitives a future olang web framework
would build on rather than reinvent.

- [The five contracts](#the-five-contracts)
- [A complete application](#a-complete-application)
- [Views as data](#views-as-data)
- [The route table](#the-route-table)
- [The browser side](#the-browser-side)
- [Forms](#forms)
- [The data layer](#the-data-layer)
- [The bundle](#the-bundle)
- [Testing](#testing)

## The five contracts

Everything else is built from these:

1. **Views are data.** An HTML tree is maps and lists — an element is
   `#{ "tag", "attrs", "children" }` — so a framework can transform,
   inspect, or diff views: the Open AST philosophy applied to markup.
   Rendering escapes by default; `raw` is the one opt-out.
2. **`mount` + re-render.** One place where data becomes pixels. A
   repaint hands the node tree to the host as data (`dom.patch`) and the
   host diffs it against the live DOM — no markup is rendered on the
   wasm side or parsed on the page: text is updated in place,
   attributes are diffed, and children match by `data-key` (else by
   position and tag), so the nodes that did not change are the nodes
   the browser keeps — focus, caret, scroll, an open select. Give list
   rows a `"data-key"` and reordering moves their elements instead of
   rebuilding them; wrap a subtree in `memo(key, inputs, build)` and it
   is neither rebuilt nor diffed while its inputs stand. Events are
   delegated, so nothing re-binds either way.
3. **One store.** `apply(f)` transforms state and repaints. State
   lives in a wasm-side cell — values never cross into JavaScript and
   back, so they keep their olang shapes exactly.
4. **One route table.** `route` and `rpc` declare endpoints once; the
   server dispatches them and the browser calls them by name.
5. **`render` is isomorphic.** The same view data renders server-side
   (pages, emails, static sites) and in the browser.

## A complete application

The server:

```olang no-run
use lib.routes { rpc }
use lib.server { serve }
use lib.sql { open_db, rows, row, insert_row }

let conn = open_db("app.db", [[
    "CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0)"
]])

let routes = [
    rpc("todos.list", (req, p) => rows(conn, "SELECT * FROM todos ORDER BY id", [])),
    rpc("todos.create", (req, p) => {
        let payload = unwrap(json.parse(req.body))
        let id = insert_row(conn, "todos", #{ "title": map_get(payload, "title"), "done": 0 })
        row(conn, "SELECT * FROM todos WHERE id = ?", [id])
    })
]

serve(#{ "title": "todos", "routes": routes, "client": "client.ol", "port": 7500 })
```

The client — served bundled, so `use web` works in the browser:

```olang no-run
use web { mount, action, apply, call, input_value,
          stack, card, btn_primary, div, input, h1 }

fn view(s) = stack([
    h1(#{}, ["todos"]),
    card([div(#{ "class": "row" }, [
        input(#{ "id": "title", "class": "field-input" }),
        btn_primary("Add", "add")
    ])]),
    stack(map(map_get(s, "todos"), (t) => card([map_get(t, "title")])))
])

fn refresh() = call("todos.list", #{}, (r) =>
    match r { Ok(ts) => { apply((s) => map_set(s, "todos", ts)) }, Err(m) => () })

action("add", (ev) =>
    call("todos.create", #{ "title": input_value("title") }, (r) => refresh()))

mount("#app", view, #{ "todos": [] })
refresh()
```

`serve` provides the rest: the HTML shell, the design system
(`web.css`, tokens first — dark mode follows the system), the dom
shim, the wasm, request logging, and one JSON envelope everywhere —
`{ "data": ... }` on success, `{ "error": { "code", "message",
"details?" } }` on failure, because the browser's fetch surfaces no
status codes and the body must carry the whole truth.

## Views as data

A library's macros import from its front door like everything else:
`use web { when }` then `@when(…)` works, because a module records the
meta fns it declares or re-exports and the runtime import of such a
name is satisfied by the expansion that already happened.

Two small forms beside `el`: `void_el(tag, attrs)` for an element with
no children (`img`, `br`, `input` written through `el`), and `@when(cond,
node)`, a macro — the node is built only when the condition holds, so
`@when(t != (), span([map_get(t, "id")]))` cannot raise on the case it
exists to skip, which a function `when` would.

```olang no-run
use web { div, span, render, escape }
let node = div(#{ "class": "card" }, ["hello ", span(#{}, ["world"])])
testing.assert_eq(render(node), "<div class=\"card\">hello <span>world</span></div>") |> unwrap
testing.assert_eq(escape("<b>"), "&lt;b&gt;") |> unwrap
```

Children may be nodes, strings, or lists (spliced), so `map(...)`
results drop straight in. Text and attribute values escape on render;
boolean attributes render bare (`disabled`), `false`/Unit render
absent.

## The route table

A handler answers a missing row with `not_found(what)` — the 404 with
the row named — beside `invalid(problems)` for a 422. `Err(message)` is
the 500, and it is for failures: an absent row is an answer, not a
crash. `req.remote_addr` is the peer (`ip:port`); with `"trust_proxy":
true` in the config it is the first entry of `X-Forwarded-For` instead.

`serve` wraps every response, the static routes included: `"headers"`
in its config is a map added to each one — the `Content-Security-Policy`,
`X-Content-Type-Options`, and `Referrer-Policy` a `<meta>` cannot carry —
and `"head"` may be a function of the request, `(req) => html`, so a
per-response CSP nonce is possible (the shell is then built per
request). `"drain": true` registers a shutdown handler: on SIGINT or
SIGTERM the server stops accepting, answers what is in flight with
`Connection: close`, reads nothing more off a kept-alive connection, and
after `"drain_ms"` (5000) cuts the rest — a long poll a tab keeps
re-issuing cannot hold the process open. `serve` then returns `Ok(())`.

A long poll should not hold a worker either: a handler that waits
returns `http.defer()` — a ticket — and the worker moves on, the parked
connection costing a socket rather than an interpreter. Whatever
produces the answer calls `http.respond(ticket, response)`, from a
spawned task or a channel service, with the same values a handler
returns (`http.response(...)`, a `{ status, body }` record, `json(...)`).
With sixty tabs polling, the server needs sixty sockets, not sixty
workers.
The configured headers are set on the response as it is, so a route that
streams a file (`body_file`) keeps streaming.

Dynamic responses are gzipped on the wire: a text body of a kilobyte or
more, to a client whose `Accept-Encoding` names gzip, goes out with
`Content-Encoding: gzip` (files and already-encoded responses pass
through). `"compress": false` turns it off.

The app's routes come before `serve`'s own: a page declared at `/`, or
an asset the app serves itself, wins over the SDK's static route for
that path, and the table is indexed once at start — an exact path is
one lookup however many routes there are; only `:param` routes are
walked. `route("GET", "/todos/:id", handler)` matches with `:param` capture;
`rpc("todos.create", handler)` mounts at `POST /api/rpc/todos.create`.
Handlers return a response — what `http.response` builds, or a
`{ status, body }` / `{ status, body_file }` record — a plain value
(wrapped in the envelope; a map is data whatever its keys, so a ticket
whose `status` field is "new" is an answer, not a response), or
`Err(message)` (a clean 500). Dispatch answers 404 with the
envelope, 405 with an `Allow` header, and `HEAD` rides `GET`.

## The browser side

`mount(selector, view_fn, initial)` wires the loop. A first paint the
server rendered with its state is adopted as it stands — the state is
what the page shows, so no render runs at boot — unless the caller's
defaults add keys the server did not render from. Events use
delegation — one set of listeners on the root, dispatching by each
element's `data-action`, so re-rendered markup never re-binds. An
action named `"toggle:7"` fires the registered `"toggle"` handler,
which reads its argument with `action_arg(ev)`; `actions(#{ name:
handler, … })` registers a table of them in one step.

Which events fire an action: a click on a button or a link (a link
carrying an action is prevented from following its `href` — the action
was the intent; `data-follow` on the anchor opts back in); Enter in a
text box; `change` on a control whose value is the argument — a select,
checkbox, radio, date, number, range, color, or file — but not on a text
box, so leaving the first of two boxes does not submit the form.
`data-on="change enter click"` names the events explicitly. The event
carries `input_type`, and `checked` only for a checkbox or radio.

One event is dispatched at a time. A host call inside a handler that
fires a DOM event synchronously (`dom.focus` → `focusin`, a blur's
`change`, `click()`) queues that event and runs it when the handler
returns, never nested inside it. A host call whose JavaScript throws (a
malformed selector) raises an olang error the handler can `attempt`;
the session lives on either way. `api.call(name,
payload, k)` posts to the rpc route and hands `k` the unwrapped
`Result`; it rides `dom.request`, which carries the status code, so a
response that is not the envelope — a proxy's HTML page, a bare
"internal server error" — arrives as `Err(#{ "message": "HTTP 502:
…", "status": 502, "details": [] })` rather than as a JSON parse
failure inside the handler.

The event contract, stated once: a click resolves its action from the
nearest `data-action` ancestor, so styled children of a button still
name it; a click *on* a form control (input, select, textarea) is a
focus or open gesture, never an action — controls fire on `change`, and
text inputs also on Enter, with the `change` that follows an Enter
within the same beat recognized as the same intent and not dispatched
twice; and a handled event repaints only when the store changed, so a
click into an action-carrying input never wipes its own text.

Repainting is `apply` (the whole view) or `patch(id, node)` — one
subtree rendered into the element with that id, leaving the rest of
the page and its focused input alone: the toast, the counter, the
chart that should not cost a full render on every store write.

Two events carry files: a `"drop"` on any element and a `"paste"` on a
text control deliver `files` as `[#{ "name", "type", "size", "base64" }]`
(the shape `dom.read_file` hands back; a text paste carries `text`
instead), so paste-to-attach is olang, not a page script. Form-control
state beyond `value` is readable: `dom.checked`, `dom.selection` and
`dom.set_selection` (insert at the cursor), `dom.values` for a
multi-select.

A request header rides every call once configured — `configure(#{
"headers": #{ "Authorization": "Bearer " + token } })` — so a bearer
token is the same shape in the browser as in the CLI. An element that
may be absent is a `dom.find(selector)`, which answers `()` on a miss
where `dom.query` raises; `dom.query_all` lists every match.

Destructive actions have a component: `btn_confirm(label, action)` arms
on the first click (its label becomes "Confirm …") and fires `action`
on the second within a few seconds; any other click disarms it. The
armed state is the view layer's, so the app registers only the real
action. `dom.confirm(message)` is the dialog alternative.

The page's own state is readable: `dom.active_id()` says what has
focus (a liveness poll re-syncing the store can leave an in-progress
edit alone), `dom.prefers_dark()` says which palette a themed chart
should pick, and `dom.read_file(el, k)` hands a file input's first
file to `k` as `#{ "name", "size", "type", "base64" }` — browser-side
uploads over an ordinary rpc body. The design system's tokens follow
the system scheme and also an explicit `data-theme="light"` or
`"dark"` on `<html>`, so a light/dark/system toggle is one attribute
write.

## Forms

Fields are declared once and used three ways:

```olang no-run
let fields = [field("title", "Title", "text", #{ "min": 1, "max": 80 })]
form_fields(fields, values, errors)   // the markup, errors shown in place
check(payload, rules(fields))         // validate (the shelf's validate library)
read(payload, fields)                 // narrowed + number-parsed input
```

The same rules run in the browser for immediacy and on the server for
truth.

## The data layer

`open_db(path, migrations)` applies versioned migrations exactly once,
each inside a transaction, recording `schema_version` — appending a
migration is how capability arrives. `rows`/`one`/`exec`/`insert_row`/
`update_row` keep parameters positional, and `one` (the single-row
query — exported under that name because `row` is the layout helper)
returns Unit for absence. `tx(conn, f)` commits on Ok and rolls back on
Err. `try_rows` and `try_exec` are the same calls as `Result`s, for a
query built from user text — an FTS5 `MATCH` that may not parse — where
failure is an answer rather than a bug.

## The store

`lib/store.ol` grows the one-store contract into a declarative,
persistence-aware store. The whole shape is a single `@store`
declaration — a prefix and a field table — validated and normalized
while the program loads:

```olang no-run
use web { mount, hydrate }

let SPEC = @store("app", [
    ["mode",  "url",   "list"],
    ["query", "url",   "state: open"],
    ["theme", "local", "system"],
    ["items", "mem",   []]
])
mount("#app", view, hydrate(SPEC))
```

Each field names where it lives: `mem` is ordinary per-load state;
`url` fields reflect into the querystring, so reloads land exactly
where the user was, links deep-link, and back/forward walk app
history; `local` fields persist in localStorage under
`<prefix>-<name>` (defaults are stored as absent, so users who never
chose keep following new defaults). `url`/`local` values are strings
by contract — parse at the use site.

The lifecycle is four calls: `hydrate(SPEC)` builds the initial state
(defaults ⊕ URL ⊕ storage), `persist(SPEC, current())` after any
navigation-ish change reflects it back (one `pushState` per real
change), `on_restore(SPEC, handler)` hands decoded url fields to the
app on back/forward, and `default_of(SPEC, name)` keeps the spec the
single source of defaults.

The macro is the checked front door: duplicate fields, unknown
classes, non-identifier names, and non-string persisted defaults are
load-time errors naming the `@store` site, and the normalized spec is
baked as a literal — `olang expand` shows exactly what the runtime
receives.

## The first paint

The shell can arrive with the page already on it. `serve` takes
`"view"` (the view function) and `"initial"` (the state it renders: a
value, or a `() => state` function run per request, so every load
shows the data as it stands), renders the view into the mount point on
the server, and places the state beside it in a JSON `<script>` the
mount point names by `data-olang-state`. The browser shows that HTML
before the runtime has downloaded. When `mount` runs, it starts the
store from that state — the server's keys over the caller's defaults,
so a `url` or `local` field `hydrate` restores keeps its value — and
the first client render reproduces what is already on screen. No boot
fetch is needed: the data came with the page.

```olang no-run
use web { serve, rows }
use lib.pages { home }        // the view, imported by both halves

serve(#{
    "routes": routes,
    "client": ["lib/pages.ol", "client.ol"],
    "view": home,
    "initial": () => #{ "notes": rows(conn, "SELECT * FROM notes ORDER BY id DESC", []),
                        "errors": [] }
})
```

Two rules follow. The view is a module both halves import
(`lib/pages.ol` in `otc new --web`), bundled ahead of the client. And
the view runs on a server worker thread, so it must be a pure function
of the state — no cells, no DOM — and the state must be JSON data
(maps, lists, strings, numbers, booleans).

## The bundle

The browser loads one program. `serve` builds it: the SDK's browser
modules spliced ahead of the app's client code, `use` lines and
`share` markers stripped, test blocks removed, macros expanded, and
the result parse-checked at boot — a broken client fails loudly at the
server, never as a blank page. `"client"` may be a list of paths,
bundled in order, so browser helpers live in tested lib modules that
server code can import too.

The bundle is served two ways. `/app.ol` is the source. `/app.olb` is
the program image: the bundle parsed once on the server and encoded
(`meta.encode`) as bytes the runtime loads without parsing, which the
shell offers through the shim's `data-bin`. Decoding an image costs a
fraction of parsing its source (under a millisecond against 22 ms for
the scaffold app), and the image is smaller than the text. An image
names the olang version that wrote it; a runtime of another version
refuses it and the shim reads the source instead, so a browser
holding a stale runtime still boots. Both revalidate by ETag, so the
HTTP cache is the cross-visit cache.

The image also carries a hot list: the functions the browser's
runtime compiles at declaration instead of at their first call, so the
boot render — the one call of `view` and its actions a page makes
before anything is hot — runs on the VM rather than the tree-walker.
By default the list is every function the app's own client files
declare; `"hot": [names]` names them explicitly, and `"hot": []` sends
no hint.

`serve` also takes `"log": (req, response, ms) => …` to own the
access line — `"log": ()` silences it, and absent the option the
environment levels it: `OLANG_ACCESS_LOG` is `all` (the default),
`errors` (400 and up), or `off` — `"bind"` for the address to listen on (`"0.0.0.0"` for
other machines; the default stays `127.0.0.1`), and `"sdk_dir"` to say
where the SDK's assets are read from — by default `WEB_SDK_DIR`, then
the directory the project's own `olang.lock` resolved `web` to (a path
entry or a shelf entry), then the machine's shelf. It serves the
runtime two ways: `/olang.<hash>.wasm`,
the content-addressed URL the shell references (immutable, cached for
a year — a new build is a new URL), and `/olang.wasm`, revalidated by
ETag. Both negotiate `Accept-Encoding`: a pre-compressed sibling on
disk next to the wasm (`olang_playground.wasm.br` or `.gz`) is served
with its `Content-Encoding`. The runtime the SDK ships is the browser
profile — no `re` module and no RSA, which a page never calls and
which were a third of the code: 4.8 MB, 1.4 MB gzipped, under 1 MB
brotli. The website's playground carries the whole stdlib (`make wasm`
builds both). The shim yields to the browser between instantiating the
runtime and running the bundle, so the shell paints first, and records
the boot phases in `window.olangBoot`: `fetch_instantiate_ms`,
`session_start_ms`, `total_ms`, and the split of the session start —
`program` ("image" or "source"), `load_ms` (decoding or parsing), and
`run_ms` (the bundle's top level, the first render included). Where a
repaint spends its time is `window.olangProfile`: `start()`, act,
`table()` — every olang function that ran, with its tier and exact
self and total milliseconds (see the tooling chapter's `olang profile`).

The playground wasm the demo serves (`static/olang_playground.wasm`)
is a build artifact, not a committed file: `make wasm` builds it and
copies it into place, as it does for the browser examples. A fresh
clone runs `make wasm` once before starting the demo.

## Testing

Under `olang test`, a direct `dispatch` runs each handler on a task
thread, the way `serve`'s workers do — so a handler that captured a
cell fails in the test that exercises it, with the same "cell escaped
its thread" the real server would raise, instead of passing every
in-process test and failing every request. `serve` itself never hops:
its worker is already the thread, so a served test — the real server
on a task, driven over a socket — runs handlers where production runs
them, and a handler that defers (`http.defer`) holds a ticket for a
real connection without any `OLANG_TEST` juggling.

The SDK's tests are olang tests — 63 of them, `olang test
frameworks/web-sdk`. The route table and envelope are exercised
in-process by constructing request values and calling `dispatch`
directly; the data layer runs against `:memory:`; the demo (`demo/`
— *shipit*, a project pulse board with inline validation, filters,
and a `viz` chart) doubles as the living example; and
`tests/live_server.ol` spawns the real server on a task thread and
drives it over a socket with the `http` client — shell, assets,
bundle, rpc round-trips, and the error envelope, end to end, in the
language itself.

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
   rebuilding them — a moved row keeps the focus and the caret of a
   control inside it, which a browser otherwise drops when an element
   leaves the document and returns; wrap a subtree in `memo(key, inputs, build)` and it
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
absent. A node keeps its children as they were given — nested lists,
bare strings, fragments — and the two readers of a tree, `render` and
the browser's patcher, flatten as they walk; code that inspects a tree
reads `children_of(node)`, the flat list of nodes.

### Keeping what did not change

Three forms skip work a repaint does not need. Each names its subtree
with a key, which becomes the element's `data-key`.

| Form | Rebuilds |
|---|---|
| `memo(key, inputs, build)` | when `inputs` differ, by value, from the last repaint's |
| `memo_list(key, items, key_of, inputs_of, row)` | the rows whose own `inputs_of(item)` changed; answers the row nodes, to splice into any container |
| `volatile(key, build)` | always, leaving no stamp a later `memo` of the key could match — "rebuild while this holds" as a call: `if editing => volatile("row", build) else => memo("row", inputs, build)` |

A kept subtree crosses to the page as a marker, and the browser leaves
the element exactly as it is, moving it if the order changed. A
`memo_list` that moved nothing costs one comparison and one marker for
all its rows. `memo` with `inputs` of `()` is `volatile`. Rendered to
markup — the server, a static preview — every form builds.

Whether a kept element is still on the page is the patcher's question:
`dom.patch` answers `#{ "missing": [keys], "nodes", "bytes",
"serialize_ms", "patch_ms" }`, and a `keep` it could not honor — a memo
stamped while its subtree was off the page — is named in `missing`.
`rerender` drops those stamps and paints once more, so a stale memo
costs one extra pass rather than a subtree that never appears, and no
repaint pays a selector scan per memo.

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

A state change repaints only when it moves something a view reads.
`watch(keys)` names the top-level state keys the view depends on;
`unwatch(keys)` names keys no view reads — a heartbeat, a presence
ping, a poll's bookkeeping, what a runtime over the SDK keeps for
itself. With neither, every key counts. A handled action is judged
once: a handler that went through `apply` is not painted again, one
that only updated the store is painted once, one that changed nothing
is not painted.

`paint_stats()` reports what repainting has cost since the page
loaded: `repaints`, `skipped` (a change that moved no watched key),
`healed` (a second pass after a `keep` the page could not honor), and —
summed over every pass, with the latest alone under `last` — `view_ms`
in the view function, `serialize_ms` and `patch_ms` inside `dom.patch`,
the `nodes` handed over, and `memo_hits` / `memo_misses`. The tree
crosses as JSON: 0.16 ms for a thousand nodes, measured in the dom
harness, which is why it is not a binary encoding.

Two events carry files: a `"drop"` on any element and a `"paste"` on a
text control deliver `files` as `[#{ "name", "type", "size", "base64" }]`
(the shape `dom.read_file` hands back; a text paste carries `text`
instead), so paste-to-attach is olang, not a page script. Form-control
state beyond `value` is readable: `dom.checked`, `dom.selection` and
`dom.set_selection` (insert at the cursor), `dom.values` for a
multi-select.

A reply says what it cost. Inside a `call` or `fetch` callback,
`last_timing()` is `#{ "network_ms", "queue_ms" }`: the time from the
send to the response's last byte (from the browser's resource timing),
and the time the reply then waited for the runtime to be free — a
repaint in progress, a handler still running. A client-side timer
around the call measures their sum and blames the server for the
page's own work. The same two fields arrive on every `dom.request`
response. Requests leave from a microtask, so the browser's own report
of a refused fetch names the shim, not two hundred frames of the
runtime.

`on_error(handler)` hears every handler that fails — `#{ "error",
"output", "trap" }`, after the failed dispatch has ended — so an app can
show the failure where a console would have hidden it; the page-side
twin is `window.olangOnError`.

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
store from that state — the server's keys over the caller's defaults —
and the first client render reproduces what is already on screen. No
boot fetch is needed: the data came with the page. What the browser
already holds is the exception: a `url` field `hydrate` read from the
address bar or a `local` field it read from storage keeps the browser's
value even when the server's state carries that key, and where the two
differ the first paint is not adopted — the view repaints once with the
preference the user chose.

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

The bundle is one namespace, so a client module that uses a name it
never imported still resolves it in the browser — from whichever module
is spliced beside it — and fails the day it runs natively or the
neighbor renames the function. `serve` checks each client module on its
own before splicing (`meta.unresolved`) and prints one `WARNING:` line
per module that has such names, naming them; `leaked_names(paths,
sources)` answers the same lines for a build script or a test.
Capitalized names are not reported: a variant constructor arrives with
the import of its enum, which the module's text alone does not show.

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
entry or a shelf entry), then the machine's shelf. A shelf entry is
looked up in the shelf `otc` and the resolver use — `OLANG_SHELF` (the
path to the `shelf.toml` itself), else `$OLANG_HOME/shelf.toml`, else
`~/.olang/shelf.toml` — so the assets come from the same SDK `use web`
resolved. The runtime it
serves is the one the `olang` binary embeds (`runtime.wasm()`): no file
on disk, no copy to keep in step, the same build as the server that
encodes the image. It is served two ways: `/olang.<hash>.wasm`, the
content-addressed URL the shell references (immutable, cached for a
year — a new build is a new URL), and `/olang.wasm`, revalidated by
ETag. Both negotiate `Accept-Encoding`: the brotli or gzip form,
computed once at boot, is served with its `Content-Encoding`. The
runtime is the browser profile — no `re` module and no RSA, which a
page never calls and which were a third of the code: 4.9 MB, 1.4 MB
gzipped, 1.2 MB brotli. The website's playground carries the whole
stdlib (`cargo xtask wasm --full` builds both). A binary built without
the runtime serves the shell and answers the runtime's URLs with a 503
that names the build. The shim yields to the browser between instantiating the
runtime and running the bundle, so the shell paints first, and records
the boot phases in `window.olangBoot`: `fetch_instantiate_ms`,
`session_start_ms`, `total_ms`, and the split of the session start —
`program` ("image" or "source"), `load_ms` (decoding or parsing), and
`run_ms` (the bundle's top level, the first render included). Where a
repaint spends its time is `window.olangProfile`: `start()`, act,
`table()` — every olang function that ran, with its tier and exact
self and total milliseconds (see the tooling chapter's `olang profile`).
The report also carries `repaints`, the number of `rerender` calls
since `start()`, and `refused`, the functions the bytecode tier refused
with the compiler's reason; `window.olangTier()` answers the tier's
report alone (`{ promoted, refused }`) for an app's own diagnostics
page.

The runtime the demo serves is the binary's own: a fresh clone runs
`make install` (which builds the wasm and installs olang with it), and
nothing is copied into the demo or the examples.

## Server push

A handler that has nothing to say yet parks its connection under a
topic and returns the ticket; any thread later answers every
connection held under that topic. The holds live in the runtime's
deferred-connection table (`http.hold`, `http.notify`), so a parked
connection costs a socket, not a worker, and one table serves however
many servers the process runs:

```olang no-run
use web { route, hold, notify, held }
route("GET", "/api/changes", (req, p) => hold("issues"))
// ... later, from the request that changed something:
notify("issues", json_response(200, changed))   // how many were answered
```

A notified connection is answered once; a client that wants the next
change asks again, which is the long poll a hundred tabs can hold
without a hundred workers. `notify_all(response)` answers every topic,
`held(topic)` counts what is parked, and a connection whose client left
is skipped. The live server test parks two requests and answers both
with one `notify`.

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

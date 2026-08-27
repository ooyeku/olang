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
2. **`mount` + re-render.** One place where data becomes pixels. v1
   re-renders the mounted tree wholesale with delegated events (a
   pattern proven by the example apps); the contract is
   engine-swappable, so keyed diffing can arrive without breaking a
   caller.
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

`route("GET", "/todos/:id", handler)` matches with `:param` capture;
`rpc("todos.create", handler)` mounts at `POST /api/rpc/todos.create`.
Handlers return a response, a plain value (wrapped in the envelope),
or `Err(message)` (a clean 500). Dispatch answers 404 with the
envelope, 405 with an `Allow` header, and `HEAD` rides `GET`.

## The browser side

`mount(selector, view_fn, initial)` wires the loop. Events use
delegation — one set of listeners on the root, dispatching by each
element's `data-action`, so re-rendered markup never re-binds. An
action named `"toggle:7"` fires the registered `"toggle"` handler,
which reads its argument with `action_arg(ev)`. `api.call(name,
payload, k)` posts to the rpc route and hands `k` the unwrapped
`Result`.

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
migration is how capability arrives. `rows`/`row`/`exec`/`insert_row`/
`update_row` keep parameters positional, and `row` returns Unit for
absence. `tx(conn, f)` commits on Ok and rolls back on Err.

## The bundle

The browser loads one source file. `serve` builds it: the SDK's
browser modules spliced ahead of the app's client code, `use` lines
and `share` markers stripped, test blocks removed, and the result
parse-checked at boot — a broken client fails loudly at the server,
never as a blank page.

## Testing

The SDK's tests are olang tests — 105 of them, `olang test
frameworks/web-sdk`. The route table and envelope are exercised
in-process by constructing request values and calling `dispatch`
directly; the data layer runs against `:memory:`; and
`tests/live_server.ol` spawns the real server on a task thread and
drives it over a socket with the `http` client — shell, assets,
bundle, rpc round-trips, and the error envelope, end to end, in the
language itself.

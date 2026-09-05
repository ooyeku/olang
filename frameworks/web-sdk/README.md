# web-sdk

The foundation layer for full-stack olang web applications — and the
proof of concept for what an ideal olang library looks like: `///` on
every export (so `:help` and editor hovers answer), `//!` module docs,
151 olang tests including a live-socket integration test the SDK runs
against itself, shelf-distributed, `olang fmt`-clean.

```bash
otc lib add frameworks/web-sdk     # once; then from any project:
otc add web
```

One route table drives the whole stack:

```olang
use web { serve, rpc }

let routes = [rpc("todos.list", (req, p) => rows(conn, "SELECT * FROM todos", []))]
serve(#{ "title": "todos", "routes": routes, "client": "client.ol" })
```

`serve` runs the JSON API behind one envelope and serves the wasm
frontend: the HTML shell, the design system (`web.css`), the dom shim,
and the client program — bundled with the SDK's browser modules
(`mount`, `apply`, `action`, `api.call`) so the browser loads one
file. Views are plain data, rendered the same way server-side and in
the browser. The demo (`demo/`) is a complete todo app in two small
files:

```bash
cd frameworks/web-sdk && olang run demo/server.ol
```

The full chapter: [docs/web-sdk.md](../../docs/web-sdk.md). Layer map:

| Module | Gives you |
|---|---|
| `lib/html.ol` | views as data: `el`/tags, escaping by default, `render`, `page` |
| `lib/routes.ol` | `route` and `rpc` — one table, matched with `:params` |
| `lib/server.ol` | `serve`, `dispatch`, the envelope, query helpers, the client bundler |
| `lib/sql.ol` | migrations, parameterized helpers, `tx` |
| `lib/forms.ol` | fields declared once → rendered, validated, read |
| `lib/ui.ol` | the opinionated components over `web.css` |
| `lib/state.ol` | the store (wasm-side cell) |
| `lib/view.ol` | `mount`, re-render, `data-action` delegation |
| `lib/api.ol` | `call(name, payload, k)` — the client half of the route table |

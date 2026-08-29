//! The `--web` template: a full-stack starter in the architecture the
//! tracker and ledger examples proved out — one olang process serves a
//! SQLite-backed JSON API, the page, and the frontend's own olang
//! source; the browser runs that source against the DOM through the
//! wasm runtime.
//!
//! The scaffold is a working notes app, deliberately small: one
//! resource, three endpoints, a frontend that lists/adds/deletes. Every
//! seam a real app grows along (another table, another route, another
//! frontend section) is present exactly once, so extending it is
//! repetition rather than research.
//!
//! The wasm runtime is the one artifact the scaffold cannot write from
//! source. `wasm_runtime()` looks in the places an installation puts it
//! and copies it in; when it is nowhere, the app still scaffolds and the
//! server still runs — the README and a boot-time warning say how to
//! supply it.

use std::path::PathBuf;

/// The generic pieces, taken verbatim from the proven examples at
/// compile time so there is exactly one source of truth for each.
const ROUTER_OL: &str = include_str!("../../../examples/web/ledger/lib/router.ol");
const DOM_SHIM_JS: &str = include_str!("../../../examples/web/app/static/olang-dom.js");

/// (relative path, contents) for every file the template writes.
pub fn files(name: &str) -> Vec<(String, String)> {
    let token_var = format!("{}_TOKEN", name.to_uppercase().replace('-', "_"));
    vec![
        ("main.ol".to_string(), main_ol(name)),
        (
            "lib/router.ol".to_string(),
            ROUTER_OL.replace("LEDGER_TOKEN", &token_var),
        ),
        ("static/index.html".to_string(), index_html(name)),
        ("static/app.ol".to_string(), app_ol(name)),
        ("static/olang-dom.js".to_string(), DOM_SHIM_JS.to_string()),
        ("static/style.css".to_string(), STYLE_CSS.to_string()),
        ("README.md".to_string(), readme(name)),
    ]
}

/// The `.ol` files the scaffold must refuse to write unless they parse.
pub fn parse_checked(rel: &str) -> bool {
    rel.ends_with(".ol")
}

/// Locate an olang_playground.wasm to copy into the new project:
/// `$OLANG_WASM` first, then next to the running executable, then
/// `~/.olang/`. None means "scaffold without it and say so".
pub fn wasm_runtime() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("OLANG_WASM") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("olang_playground.wasm");
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let stashed = PathBuf::from(home).join(".olang/olang_playground.wasm");
        if stashed.is_file() {
            return Some(stashed);
        }
    }
    None
}

fn main_ol(name: &str) -> String {
    format!(
        r#"// {name} — a full-stack olang app: this one process serves a
// SQLite-backed JSON API, the page, and the frontend's own olang
// source (static/app.ol), which the browser runs against the DOM
// through the wasm runtime.
//
//   olang main.ol [port] [db_path]     (defaults: 7500, {name}.db)

use lib.router {{ route, dispatch, json_response, error_response }}

let args = os.args()
let port = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 7500
let db_path = if len(args) > 2 => args[2] else => "{name}.db"

let conn = unwrap(db.open(db_path))
unwrap(db.execute(conn,
    "CREATE TABLE IF NOT EXISTS notes (
        id INTEGER PRIMARY KEY,
        text TEXT NOT NULL,
        created_at TEXT NOT NULL
    )"))

// ── static files, read once at startup ──
let page_html = unwrap(fs.read_file("static/index.html"))
let shim_js = unwrap(fs.read_file("static/olang-dom.js"))
let app_ol = unwrap(fs.read_file("static/app.ol"))
let style_css = unwrap(fs.read_file("static/style.css"))

if !fs.exists("static/olang_playground.wasm") => {{
    println("WARNING: static/olang_playground.wasm is missing — the frontend cannot boot.")
    println("The API still works. See README.md for how to supply the wasm runtime.")
}}

fn page(req, params) =
    http.response_with_headers(200, page_html,
        #{{ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" }})
fn shim(req, params) =
    http.response_with_headers(200, shim_js,
        #{{ "Content-Type": "text/javascript; charset=utf-8", "Cache-Control": "no-store" }})
fn frontend_source(req, params) =
    http.response_with_headers(200, app_ol,
        #{{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" }})
fn styles(req, params) =
    http.response_with_headers(200, style_css,
        #{{ "Content-Type": "text/css; charset=utf-8", "Cache-Control": "no-store" }})
// The wasm is binary: body_file serves raw bytes straight from disk.
fn wasm_binary(req, params) = {{
    status: 200,
    body_file: "static/olang_playground.wasm",
    headers: #{{ "Content-Type": "application/wasm" }}
}}

// ── the API ──

fn notes_list(req, params) =
    json_response(200, {{
        items: unwrap(db.query(conn, "SELECT id, text, created_at FROM notes ORDER BY id DESC"))
    }})

fn notes_create(req, params) = {{
    let body = if len(req.body) == 0 => "{{}}" else => req.body
    match json.parse(body) {{
        Err(e) => error_response(400, "bad_json", "body must be JSON"),
        Ok(doc) => {{
            let text = if map_has_key(doc, "text") => str.trim(map_get(doc, "text")) else => ""
            if text == "" => error_response(422, "invalid", "text must not be empty")
            else => {{
                let now = unwrap(dates.format_date(dates.today(), "%Y-%m-%d"))
                unwrap(db.execute(conn, "INSERT INTO notes (text, created_at) VALUES (?, ?)", [text, now]))
                let row = unwrap(db.query_one(conn, "SELECT id, text, created_at FROM notes ORDER BY id DESC LIMIT 1"))
                json_response(201, row)
            }}
        }}
    }}
}}

fn notes_delete(req, params) = match str.parse_int(map_get(params, "id")) {{
    Err(e) => error_response(400, "bad_id", "id must be an integer"),
    Ok(id) => {{
        let n = unwrap(db.execute(conn, "DELETE FROM notes WHERE id = ?", [id]))
        if n == 0 => error_response(404, "not_found", "no note " + show(id))
        else => json_response(200, {{ deleted: id }})
    }}
}}

fn health(req, params) = {{
    let count = map_get(unwrap(db.query_one(conn, "SELECT COUNT(*) AS n FROM notes")), "n")
    json_response(200, {{ status: "ok", notes: count }})
}}

// ── wiring ──

let routes = [
    route("GET", "/", page),
    route("GET", "/olang-dom.js", shim),
    route("GET", "/app.ol", frontend_source),
    route("GET", "/style.css", styles),
    route("GET", "/olang.wasm", wasm_binary),
    route("GET", "/api/health", health),
    route("GET", "/api/notes", notes_list),
    route("POST", "/api/notes", notes_create),
    route("DELETE", "/api/notes/:id", notes_delete)
]

fn app(req) = dispatch(routes, req)

println(`{name} listening on http://127.0.0.1:${{port}}  (db: ${{db_path}})`)
match http.serve(port, app) {{
    Err(e) => {{
        println("could not start on port " + show(port) + ": " + show(e))
        os.exit(1)
    }},
    Ok(v) => v
}}
"#
    )
}

fn index_html(name: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{name}</title>
  <link rel="stylesheet" href="/style.css">
</head>
<body>
  <main>
    <h1>{name}</h1>
    <div class="composer">
      <input id="new-text" placeholder="a note…" autofocus>
      <button id="add-btn">add</button>
    </div>
    <div id="list"></div>
    <p class="foot">frontend: olang (wasm) · backend: olang · sqlite —
      <a href="/app.ol">view frontend source</a></p>
  </main>
  <script src="/olang-dom.js" data-src="/app.ol" defer></script>
</body>
</html>
"#
    )
}

fn app_ol(name: &str) -> String {
    format!(
        r##"// {name}/static/app.ol — the frontend, in olang, running as
// WebAssembly. The server owns the data; this file fetches it, renders
// it, and posts mutations. Listeners bind once at boot to elements that
// exist at boot; rows re-render as HTML with actions encoded in element
// ids (del-17), so no handler ever holds a stale element.

// User text goes through esc() before set_html — always.
fn esc(s) = show(s)
    |> str.replace("&", "&amp;")
    |> str.replace("<", "&lt;")
    |> str.replace(">", "&gt;")
    |> str.replace("\"", "&quot;")

fn json_esc(s) = show(s)
    |> str.replace("\\", "\\\\")
    |> str.replace("\"", "\\\"")
    |> str.replace("\n", "\\n")

fn row_html(n) = {{
    let id = show(map_get(n, "id"))
    "<div class=\"row\"><span class=\"date\">" + esc(map_get(n, "created_at"))
        + "</span><span class=\"text\">" + esc(map_get(n, "text"))
        + "</span><button class=\"del\" id=\"del-" + id + "\">×</button></div>"
}}

fn render(items) =
    dom.set_html(dom.query("#list"),
        if len(items) == 0 => "<div class=\"empty\">no notes yet — write one above</div>"
        else => items |> map(row_html) |> join(""))

fn refresh() =
    dom.fetch_json("GET", "/api/notes", "", (resp) => {{
        if map_has_key(resp, "items") => render(map_get(resp, "items"))
    }})

fn add_note() = {{
    let text = str.trim(dom.value(dom.query("#new-text")))
    if text != "" => {{
        let body = "{{\"text\": \"" + json_esc(text) + "\"}}"
        dom.fetch_json("POST", "/api/notes", body, (resp) => {{
            dom.set_value(dom.query("#new-text"), "")
            dom.focus(dom.query("#new-text"))
            refresh()
        }})
    }}
}}

fn on_list_click(tid) = {{
    if starts_with(tid, "del-") => {{
        let id = str.substring(tid, 4, len(tid))
        dom.fetch_json("DELETE", "/api/notes/" + id, "", (resp) => {{ refresh() }})
    }}
}}

dom.on(dom.query("#add-btn"), "click", (e) => {{ add_note() }})
dom.on(dom.query("#new-text"), "enter", (e) => {{ add_note() }})
dom.on(dom.query("#list"), "click", (e) => {{ on_list_click(map_get(e, "id")) }})

refresh()
"##
    )
}

const STYLE_CSS: &str = r#":root {
  --bg: #0b0f14; --panel: #101720; --line: #1d2937;
  --text: #d9e6ef; --dim: #7f93a3; --accent: #3ddc97; --red: #ef6b73;
}
* { box-sizing: border-box; }
body {
  margin: 0; background: var(--bg); color: var(--text);
  font: 15px/1.5 ui-monospace, "SF Mono", Menlo, Consolas, monospace;
}
main { max-width: 640px; margin: 3rem auto; padding: 0 1rem; }
h1 { color: var(--accent); font-size: 1.2rem; }
.composer { display: flex; gap: 0.5rem; margin-bottom: 1rem; }
.composer input {
  flex: 1; background: var(--panel); color: var(--text);
  border: 1px solid var(--line); border-radius: 8px; padding: 0.55rem 0.8rem;
  font: inherit; outline: none;
}
.composer input:focus { border-color: var(--accent); }
.composer button {
  background: var(--accent); color: #06251a; border: 0; border-radius: 8px;
  padding: 0.55rem 1.1rem; font: inherit; font-weight: 600; cursor: pointer;
}
.row {
  display: flex; align-items: center; gap: 0.7rem;
  background: var(--panel); border: 1px solid var(--line); border-radius: 10px;
  padding: 0.55rem 0.8rem; margin-bottom: 0.5rem;
}
.row .date { color: var(--dim); font-size: 12px; white-space: nowrap; }
.row .text { flex: 1; }
.row .del {
  background: none; border: 1px solid var(--line); border-radius: 6px;
  color: var(--red); cursor: pointer; padding: 0.1rem 0.5rem;
}
.empty { color: var(--dim); text-align: center; padding: 2rem 0; }
.foot { color: var(--dim); font-size: 12.5px; margin-top: 2rem; }
.foot a { color: var(--dim); }
"#;

fn readme(name: &str) -> String {
    let token = format!("{}_TOKEN", name.to_uppercase().replace('-', "_"));
    format!(
        r#"# {name}

A full-stack olang app: one process serves a SQLite-backed JSON API,
this page, and the frontend's own olang source, which the browser runs
against the DOM through the wasm runtime.

```bash
olang main.ol              # http://127.0.0.1:7500, db {name}.db
olang main.ol 8080 my.db   # choose port and database
```

## Layout

```
main.ol            server: routes + /api/notes CRUD; static files read at boot
lib/router.ol      :param routing, 404/405, auth hook, the JSON error envelope
static/app.ol      the frontend: fetch, render, mutate — in olang
static/index.html  the page; static/olang-dom.js is the generic wasm shim
```

## The wasm runtime

The frontend runs on `static/olang_playground.wasm`. If the scaffold
found one on your machine it is already in place; otherwise build it
from an olang checkout and copy it in:

```bash
cargo build -p olang-playground --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/olang_playground.wasm static/
```

(Or set `OLANG_WASM=/path/to/olang_playground.wasm` before scaffolding.)
The server runs and the API works without it; only the browser frontend
needs it.

## Growing the app

Each seam appears exactly once, so extending is repetition:

- **another endpoint**: a handler function plus one line in `routes`
- **another table**: a `CREATE TABLE IF NOT EXISTS` next to the first
- **another frontend section**: a render function + a delegated handler
  in `static/app.ol`

Set `{token}` to require `Authorization: Bearer <token>`
on mutating requests (see `lib/router.ol`).
"#
    )
}

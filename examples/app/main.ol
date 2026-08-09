// tracker — a full web app served entirely by olang.
//
//   olang main.ol [port]        (default 7000)
//
// The backend is an in-memory SQLite issue store behind a JSON API; the
// frontend is a spreadsheet-style grid (plain HTML + JS) served by the
// same process from static/. One command runs the whole thing.
//
//   GET    /                  the app
//   GET    /api/issues        all issues
//   POST   /api/issues        create  {title?, status?, priority?, assignee?, points?, notes?}
//   PATCH  /api/issues/:id    partial update, any subset of the columns
//   DELETE /api/issues/:id    remove
//   GET    /health            liveness

use lib.router { route, dispatch, json_response }

// ── the store ──

let conn = unwrap(db.open(":memory:"))
unwrap(db.execute(conn, "CREATE TABLE issues (
    id       INTEGER PRIMARY KEY,
    title    TEXT NOT NULL,
    status   TEXT NOT NULL DEFAULT 'open',
    priority TEXT NOT NULL DEFAULT 'medium',
    assignee TEXT NOT NULL DEFAULT '',
    points   INTEGER NOT NULL DEFAULT 0,
    notes    TEXT NOT NULL DEFAULT '',
    updated  TEXT NOT NULL
)"))

fn seed(title, status, priority, assignee, points, notes) =
    unwrap(db.execute(conn,
        "INSERT INTO issues (title, status, priority, assignee, points, notes, updated)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        [title, status, priority, assignee, points, notes, dates.utc_now()]))

seed("Ship the tracker example", "in-progress", "high", "ada", 3, "the app you are looking at")
seed("Spreadsheet keyboard nav", "open", "medium", "grace", 2, "enter commits + moves down")
seed("Wire up column sorting", "done", "low", "ada", 1, "click a header")
seed("Decide on dark mode", "open", "low", "", 1, "")

let columns = ["title", "status", "priority", "assignee", "points", "notes"]
let select_cols = "id, title, status, priority, assignee, points, notes, updated"

// ── static files, read once at startup ──

let index_html = unwrap(fs.read_file("static/index.html"))
let app_js = unwrap(fs.read_file("static/app.js"))

fn page(req, params) =
    http.response_with_headers(200, index_html, #{ "Content-Type": "text/html; charset=utf-8" })
fn script(req, params) =
    http.response_with_headers(200, app_js, #{ "Content-Type": "text/javascript; charset=utf-8" })

// ── the API ──

fn list_issues(req, params) =
    json_response(200, unwrap(db.query(conn,
        "SELECT " + select_cols + " FROM issues ORDER BY id")))

fn field_or(doc, key, fallback) =
    if map_has_key(doc, key) => map_get(doc, key) else => fallback

fn create_issue(req, params) = {
    let body = if len(req.body) == 0 => "{}" else => req.body
    match json.parse(body) {
        Ok(doc) => {
            let made = unwrap(db.query_one(conn,
                "INSERT INTO issues (title, status, priority, assignee, points, notes, updated)
                 VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING " + select_cols,
                [field_or(doc, "title", "New issue"),
                 field_or(doc, "status", "open"),
                 field_or(doc, "priority", "medium"),
                 field_or(doc, "assignee", ""),
                 field_or(doc, "points", 0),
                 field_or(doc, "notes", ""),
                 dates.utc_now()]))
            json_response(201, made)
        },
        Err(e) => json_response(400, { message: "body must be JSON" })
    }
}

fn update_issue(req, params) = {
    let id = map_get(params, "id")
    match json.parse(req.body) {
        Ok(doc) => {
            // Build the UPDATE from the columns actually present, so a cell
            // edit sends only its own field. Column names come from the
            // whitelist, never the request.
            let mut sets = []
            let mut vals = []
            for col in columns {
                if map_has_key(doc, col) => {
                    sets = concat(sets, [col + " = ?"])
                    vals = concat(vals, [map_get(doc, col)])
                }
            }
            if len(sets) == 0 => json_response(400, { message: "no known fields in body" })
            else => {
                let sql = "UPDATE issues SET " + join(sets, ", ") +
                    ", updated = ? WHERE id = ? RETURNING " + select_cols
                let rows = unwrap(db.query(conn, sql,
                    concat(vals, [dates.utc_now(), id])))
                if len(rows) == 0 => json_response(404, { message: "no issue " + id })
                else => json_response(200, rows[0])
            }
        },
        Err(e) => json_response(400, { message: "body must be JSON" })
    }
}

fn delete_issue(req, params) = {
    let id = map_get(params, "id")
    unwrap(db.execute(conn, "DELETE FROM issues WHERE id = ?", [id]))
    json_response(200, { deleted: id })
}

fn health(req, params) = json_response(200, { status: "ok", service: "olang-tracker" })

// ── wiring ──

let routes = [
    route("GET", "/", page),
    route("GET", "/app.js", script),
    route("GET", "/health", health),
    route("GET", "/api/issues", list_issues),
    route("POST", "/api/issues", create_issue),
    route("PATCH", "/api/issues/:id", update_issue),
    route("DELETE", "/api/issues/:id", delete_issue)
]

fn app(req) = dispatch(routes, req)

let args = unwrap(os.args())
let port = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 7000
println("tracker running on http://127.0.0.1:" + show(port))
http.serve(port, app)

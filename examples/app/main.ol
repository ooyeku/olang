// tracker — a full web app served entirely by olang.
//
//   olang main.ol [port] [db_path]     (defaults: 7317, tracker.db)
//
// The backend is a persistent SQLite issue store (schema-migrated,
// transactional, audit-logged) behind a JSON API with validation,
// filtering, pagination, comments, stats (via the ods data stack),
// CSV export, and optional bearer-token auth for writes. The frontend
// is a spreadsheet-style grid served by the same process from static/.
//
//   GET    /                             the app
//   GET    /health                       liveness: uptime, issue count
//   GET    /api/issues                   ?status= &assignee= &q= &sort= &order= &limit= &offset=
//   POST   /api/issues                   create (validated; 422 lists problems)
//   GET    /api/issues/:id               one issue, comments embedded
//   PATCH  /api/issues/:id               partial update (validated)
//   DELETE /api/issues/:id               remove issue + comments, one transaction
//   GET    /api/issues/:id/comments      comments
//   POST   /api/issues/:id/comments      add {text, author?}
//   GET    /api/activity                 the audit trail, newest first (?limit=)
//   GET    /api/stats                    rollups + point quantiles (ods)
//   GET    /api/export.csv               every issue as CSV
//   POST   /api/admin/backup             JSON snapshot into backups/
//
// Set TRACKER_TOKEN to require `Authorization: Bearer <token>` on all
// mutating requests. Reads stay open.

use lib.router { route, dispatch, json_response, error_response, error_with_details, q_str, q_int, q_enum }
use lib.store {
    open_store, seed_if_empty, list_issues, get_issue, create_issue, update_issue,
    delete_issue, list_comments, add_comment, recent_events, stats, all_issues
}
use lib.validate { validate_issue, validate_comment, validate_id }

let args = unwrap(os.args())
// Default 7317, not 7000: macOS AirPlay Receiver (Control Center) listens
// on ports 5000 and 7000 on every modern Mac and answers with 403
// Forbidden — a browser hitting an olang app that isn't running (or that
// lost the port) gets AirPlay's 403 instead of connection-refused, which
// reads as a mysterious "access denied".
let port = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 7317
let db_path = if len(args) > 2 => args[2] else => "tracker.db"

let conn = open_store(db_path)
seed_if_empty(conn)
let boot_ms = time.monotonic_ms()

// ── static files, read once at startup ──

let olang_html = unwrap(fs.read_file("static/index.html"))
let olang_shim = unwrap(fs.read_file("static/olang-dom.js"))
let app_ol = unwrap(fs.read_file("static/app.ol"))
let orbit_html = unwrap(fs.read_file("static/orbit.html"))
let orbit_ol = unwrap(fs.read_file("static/orbit.ol"))
let notes_html = unwrap(fs.read_file("static/notes.html"))
let notes_ol = unwrap(fs.read_file("static/notes.ol"))
let worker_harness = unwrap(fs.read_file("static/olang-worker.js"))
let primes_html = unwrap(fs.read_file("static/primes.html"))
let primes_ol = unwrap(fs.read_file("static/primes.ol"))
let primes_worker_ol = unwrap(fs.read_file("static/primes-worker.ol"))
let charts_html = unwrap(fs.read_file("static/charts.html"))
let charts_ol = unwrap(fs.read_file("static/charts.ol"))
let gallery_html = unwrap(fs.read_file("static/gallery.html"))
let gallery_ol = unwrap(fs.read_file("static/gallery.ol"))
let board_html = unwrap(fs.read_file("static/board.html"))
let board_ol = unwrap(fs.read_file("static/board.ol"))

// The wasm artifact is gitignored; warn loudly at boot when missing.
// (fs.exists, not read_file: the artifact is binary, and reading it as
// a string fails on non-UTF-8 bytes even when the file is fine.)
if !unwrap_or(fs.exists("static/olang_playground.wasm"), false) => {
    println("WARNING: static/olang_playground.wasm is missing — the frontend cannot boot.")
    println("Build and copy it:")
    println("  cargo build -p olang-playground --target wasm32-unknown-unknown --release")
    println("  cp target/wasm32-unknown-unknown/release/olang_playground.wasm examples/app/static/")
}

// ── the olang frontend: the same tracker with its logic in app.ol,
//    running in the browser as wasm (see docs: the dom module) ──
fn olang_page(req, params) =
    http.response_with_headers(200, olang_html,
        #{ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" })
fn olang_shim_js(req, params) =
    http.response_with_headers(200, olang_shim,
        #{ "Content-Type": "text/javascript; charset=utf-8", "Cache-Control": "no-store" })
fn olang_source(req, params) =
    http.response_with_headers(200, app_ol,
        #{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" })
// The orbits demo: rich canvas graphics from the same shim (stage 2).
fn orbit_page(req, params) =
    http.response_with_headers(200, orbit_html,
        #{ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" })
fn orbit_source(req, params) =
    http.response_with_headers(200, orbit_ol,
        #{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" })
fn notes_page(req, params) =
    http.response_with_headers(200, notes_html,
        #{ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" })
fn notes_source(req, params) =
    http.response_with_headers(200, notes_ol,
        #{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" })
// The primes demo: a second olang in a Web Worker (stage 4).
fn worker_harness_js(req, params) =
    http.response_with_headers(200, worker_harness,
        #{ "Content-Type": "text/javascript; charset=utf-8", "Cache-Control": "no-store" })
fn primes_page(req, params) =
    http.response_with_headers(200, primes_html,
        #{ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" })
fn primes_source(req, params) =
    http.response_with_headers(200, primes_ol,
        #{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" })
fn primes_worker_source(req, params) =
    http.response_with_headers(200, primes_worker_ol,
        #{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" })
// The data-viz pages: live tracker analytics and the gallery (viz stage 1).
fn charts_page(req, params) =
    http.response_with_headers(200, charts_html,
        #{ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" })
fn charts_source(req, params) =
    http.response_with_headers(200, charts_ol,
        #{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" })
fn gallery_page(req, params) =
    http.response_with_headers(200, gallery_html,
        #{ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" })
fn gallery_source(req, params) =
    http.response_with_headers(200, gallery_ol,
        #{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" })
// The ops board: the dash kit's flagship (viz stage 5).
fn board_page(req, params) =
    http.response_with_headers(200, board_html,
        #{ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" })
fn board_source(req, params) =
    http.response_with_headers(200, board_ol,
        #{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" })
// The wasm is binary: body_file serves raw bytes straight from disk.
fn olang_wasm(req, params) = {
    status: 200,
    body_file: "static/olang_playground.wasm",
    headers: #{ "Content-Type": "application/wasm" }
}

// ── request-body plumbing ──

fn parse_body(req) = {
    let body = if len(req.body) == 0 => "{}" else => req.body
    json.parse(body)
}

// ── issues ──

fn issues_list(req, params) = {
    let filters = {
        status: q_enum(req, "status", ["open", "in-progress", "done"], ""),
        assignee: q_str(req, "assignee", ""),
        q: q_str(req, "q", ""),
        sort: q_enum(req, "sort", ["id", "title", "status", "priority", "points", "updated", "assignee"], "id"),
        order: q_enum(req, "order", ["asc", "desc"], "asc"),
        limit: q_int(req, "limit", 100, 1, 500),
        offset: q_int(req, "offset", 0, 0, 1000000)
    }
    let page = list_issues(conn, filters)
    json_response(200, {
        items: page.items, total: page.total,
        limit: filters.limit, offset: filters.offset
    })
}

fn issues_create(req, params) = match parse_body(req) {
    Err(e) => error_response(400, "bad_json", "body must be JSON"),
    Ok(doc) => match validate_issue(doc, false) {
        Err(problems) => error_with_details(422, "invalid", "validation failed", problems),
        Ok(fields) => json_response(201, create_issue(conn, fields))
    }
}

fn issues_get(req, params) = match validate_id(map_get(params, "id")) {
    Err(msg) => error_response(400, "bad_id", msg),
    Ok(id) => match get_issue(conn, id) {
        Err(e) => error_response(404, "not_found", "no issue " + show(id)),
        Ok(issue) => json_response(200, map_set(issue, "comments", list_comments(conn, id)))
    }
}

fn issues_update(req, params) = match validate_id(map_get(params, "id")) {
    Err(msg) => error_response(400, "bad_id", msg),
    Ok(id) => match parse_body(req) {
        Err(e) => error_response(400, "bad_json", "body must be JSON"),
        Ok(doc) => match validate_issue(doc, true) {
            Err(problems) => error_with_details(422, "invalid", "validation failed", problems),
            Ok(fields) => match update_issue(conn, id, fields) {
                Err(e) => error_response(404, "not_found", "no issue " + show(id)),
                Ok(updated) => json_response(200, updated)
            }
        }
    }
}

fn issues_delete(req, params) = match validate_id(map_get(params, "id")) {
    Err(msg) => error_response(400, "bad_id", msg),
    Ok(id) => match delete_issue(conn, id) {
        Err(e) => error_response(404, "not_found", "no issue " + show(id)),
        Ok(gone) => json_response(200, { deleted: map_get(gone, "id"), title: map_get(gone, "title") })
    }
}

// ── comments ──

fn comments_list(req, params) = match validate_id(map_get(params, "id")) {
    Err(msg) => error_response(400, "bad_id", msg),
    Ok(id) => match get_issue(conn, id) {
        Err(e) => error_response(404, "not_found", "no issue " + show(id)),
        Ok(issue) => json_response(200, list_comments(conn, id))
    }
}

fn comments_add(req, params) = match validate_id(map_get(params, "id")) {
    Err(msg) => error_response(400, "bad_id", msg),
    Ok(id) => match get_issue(conn, id) {
        Err(e) => error_response(404, "not_found", "no issue " + show(id)),
        Ok(issue) => match parse_body(req) {
            Err(e) => error_response(400, "bad_json", "body must be JSON"),
            Ok(doc) => match validate_comment(doc) {
                Err(problems) => error_with_details(422, "invalid", "validation failed", problems),
                Ok(c) => json_response(201, add_comment(conn, id, c.author, c.text))
            }
        }
    }
}

// ── activity, stats, export, admin ──

fn activity(req, params) =
    json_response(200, recent_events(conn, q_int(req, "limit", 50, 1, 500)))

fn stats_endpoint(req, params) = {
    let s = stats(conn)
    // The ods data stack computes the distribution shape SQL can't:
    // point quantiles across the whole backlog.
    let quantiles = if len(s.point_values) == 0 => { p25: 0.0, p50: 0.0, p75: 0.0 }
        else => {
            let series = ods.series(s.point_values)
            { p25: ods.quantile(series, 0.25),
              p50: ods.quantile(series, 0.5),
              p75: ods.quantile(series, 0.75) }
        }
    json_response(200, {
        totals: s.totals, by_status: s.by_status,
        by_assignee: s.by_assignee, point_quantiles: quantiles
    })
}

fn export_csv(req, params) = {
    let rows = all_issues(conn)
    let headers = ["id", "title", "status", "priority", "assignee", "points", "notes", "updated"]
    // DB rows read through map_get; build the cell grid explicitly so the
    // CSV layer only ever sees lists of strings.
    let grid = concat([headers],
        rows |> map((r) => headers |> map((h) => show(map_get(r, h)))))
    // csv.stringify returns the text directly on success, an Err value
    // on failure — check the shape rather than pattern-matching Ok.
    let out = csv.stringify(grid)
    if typeof(out) == "String" => http.response_with_headers(200, out, #{
        "Content-Type": "text/csv; charset=utf-8",
        "Content-Disposition": "attachment; filename=issues.csv"
    })
    else => error_response(500, "export_failed", show(out))
}

fn backup(req, params) = {
    fs.create_dir_all("backups")
    let snapshot = { taken: dates.utc_now(), issues: all_issues(conn) }
    let name = "backups/tracker-" + show(time.now_ms()) + ".json"
    match fs.write_file(name, unwrap(json.stringify(snapshot))) {
        Ok(v) => json_response(201, { backup: name, issues: len(map_get(snapshot, "issues")) }),
        Err(e) => error_response(500, "backup_failed", show(e))
    }
}

fn health(req, params) = {
    let n = map_get(unwrap(db.query_one(conn, "SELECT COUNT(*) AS n FROM issues")), "n")
    json_response(200, {
        status: "ok", service: "olang-tracker", db: db_path,
        issues: n, uptime_ms: time.monotonic_ms() - boot_ms
    })
}

// ── wiring ──

let routes = [
    route("GET", "/", olang_page),
    route("GET", "/olang-dom.js", olang_shim_js),
    route("GET", "/app.ol", olang_source),
    route("GET", "/orbit.html", orbit_page),
    route("GET", "/orbit.ol", orbit_source),
    route("GET", "/notes.html", notes_page),
    route("GET", "/notes.ol", notes_source),
    route("GET", "/olang-worker.js", worker_harness_js),
    route("GET", "/primes.html", primes_page),
    route("GET", "/primes.ol", primes_source),
    route("GET", "/primes-worker.ol", primes_worker_source),
    route("GET", "/charts.html", charts_page),
    route("GET", "/charts.ol", charts_source),
    route("GET", "/gallery.html", gallery_page),
    route("GET", "/gallery.ol", gallery_source),
    route("GET", "/board.html", board_page),
    route("GET", "/board.ol", board_source),
    route("GET", "/olang.wasm", olang_wasm),
    route("GET", "/health", health),
    route("GET", "/api/issues", issues_list),
    route("POST", "/api/issues", issues_create),
    route("GET", "/api/issues/:id", issues_get),
    route("PATCH", "/api/issues/:id", issues_update),
    route("DELETE", "/api/issues/:id", issues_delete),
    route("GET", "/api/issues/:id/comments", comments_list),
    route("POST", "/api/issues/:id/comments", comments_add),
    route("GET", "/api/activity", activity),
    route("GET", "/api/stats", stats_endpoint),
    route("GET", "/api/export.csv", export_csv),
    route("POST", "/api/admin/backup", backup)
]

fn app(req) = dispatch(routes, req)

println("tracker db=" + db_path + (match os.get_env("TRACKER_TOKEN") { Ok(t) => " auth=on", Err(e) => " auth=off" }))
http.serve(port, app)

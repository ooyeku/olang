// A concurrent notes API served by http.serve: routing with :param segments,
// structured access logs, JSON in and out, and a SQLite store that persists
// across requests (the handler closes over the synchronized connection).
// Run it, then talk to it with curl:
//
//   olang main.ol 8080                 // automatic worker count
//   olang main.ol 8080 8               // explicit worker count
//   curl http://127.0.0.1:8080/notes
//   curl -X POST -d '{"text": "ship the release"}' http://127.0.0.1:8080/notes
//   curl http://127.0.0.1:8080/notes/1
//   curl -X DELETE http://127.0.0.1:8080/notes/1
//
// Access logging is JSON Lines. OLANG_ACCESS_LOG accepts all, errors
// (default), or off. Use `OLANG_ACCESS_LOG=all olang main.ol 8080` to log
// every request; keep it at errors while benchmarking to avoid measuring
// terminal I/O.
//
// Stress-test a running server from another terminal:
//   python3 benchmark.py --duration 30 --concurrency 25

use lib.router { route, dispatch, json_response }

// ── the store ──
let conn = unwrap(db.open(":memory:"))
unwrap(db.execute(conn, "CREATE TABLE notes (id INTEGER PRIMARY KEY, text TEXT)"))
unwrap(db.execute(conn, "INSERT INTO notes (text) VALUES (?)", ["welcome to olang"]))
unwrap(db.execute(conn, "INSERT INTO notes (text) VALUES (?)", ["try POST /notes"]))

// ── handlers ──
fn index_page(req, params) = "olang notes API\n" +
    "  GET    /notes        list all notes\n" +
    "  POST   /notes        create ({\"text\": ...})\n" +
    "  GET    /notes/:id    fetch one\n" +
    "  DELETE /notes/:id    remove one\n"

fn list_notes(req, params) =
    json_response(200, unwrap(db.query(conn, "SELECT id, text FROM notes ORDER BY id")))

fn create_note(req, params) = {
    match json.parse(req.body) {
        Ok(doc) => {
            // INSERT ... RETURNING is one synchronized DB operation. A
            // separate INSERT followed by "latest row" can race with another
            // worker and return the wrong request's note.
            let made = unwrap(db.query_one(conn,
                "INSERT INTO notes (text) VALUES (?) RETURNING id, text", [doc.text]))
            json_response(201, made)
        },
        Err(e) => json_response(400, { message: "body must be JSON like {\"text\": ...}" })
    }
}

fn get_note(req, params) = {
    let id = map_get(params, "id")
    let rows = unwrap(db.query(conn, "SELECT id, text FROM notes WHERE id = ?", [id]))
    if len(rows) == 0 => json_response(404, { message: "no note " + id })
    else => json_response(200, rows[0])
}

fn delete_note(req, params) = {
    let id = map_get(params, "id")
    unwrap(db.execute(conn, "DELETE FROM notes WHERE id = ?", [id]))
    json_response(200, { deleted: id })
}

fn health(req, params) =
    json_response(200, { status: "ok", service: "olang-notes" })

// ── the app ──
let routes = [
    route("GET", "/health", health),
    route("GET", "/", index_page),
    route("GET", "/notes", list_notes),
    route("POST", "/notes", create_note),
    route("GET", "/notes/:id", get_note),
    route("DELETE", "/notes/:id", delete_note)
]

// ── structured access logging ──
fn env_or(name, fallback) = match os.get_env(name) {
    Ok(value) => value,
    Err(e) => fallback
}

fn response_status(response) = {
    if typeof(response) == "String" => 200
    else => {
        let status = map_get(response, "status")
        if typeof(status) == "Int" => status else => 200
    }
}

fn response_size(response) = {
    if typeof(response) == "String" => len(response)
    else => {
        let body = map_get(response, "body")
        if typeof(body) == "String" => len(body) else => 0
    }
}

fn header_or(req, name, fallback) = {
    let value = map_get(req.headers, name)
    if typeof(value) == "Unit" => fallback else => show(value)
}

fn should_log(mode, status) = {
    if mode == "all" => true
    else if mode == "errors" => status >= 400
    else => false
}

fn app(req) = {
    let started = time.monotonic_ms()
    let response = dispatch(routes, req)
    let elapsed_ms = time.monotonic_ms() - started
    let status = response_status(response)

    if should_log(log_mode, status) => {
        let record = {
            timestamp: dates.utc_now(),
            level: if status >= 500 => "ERROR" else if status >= 400 => "WARN" else => "INFO",
            event: "http_request",
            method: req.method,
            path: req.path,
            status: status,
            duration_ms: elapsed_ms,
            bytes_in: len(req.body),
            bytes_out: response_size(response),
            remote_addr: req.remote_addr,
            request_id: header_or(req, "x-request-id", "-"),
            user_agent: header_or(req, "user-agent", "-")
        }
        println(unwrap(json.stringify(record)))
    }
    response
}

let args = unwrap(os.args())
let port = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 8080
let worker_count = if len(args) > 2 => unwrap(str.parse_int(args[2])) else => 0

let requested_log_mode = str.to_lower(env_or("OLANG_ACCESS_LOG", "errors"))
let log_mode = if contains(["all", "errors", "off"], requested_log_mode) => requested_log_mode
    else => {
        println("invalid OLANG_ACCESS_LOG=" + requested_log_mode + "; using errors")
        "errors"
    }

let serve_options = if worker_count > 0 =>
    #{ "workers": worker_count, "queue_capacity": worker_count * 128 }
else => #{}

println("olang notes API — press Ctrl-C to stop")
println(unwrap(json.stringify({
    timestamp: dates.utc_now(),
    level: "INFO",
    event: "server_start",
    port: port,
    workers: if worker_count > 0 => worker_count else => "auto",
    access_log: log_mode
})))
unwrap(http.serve(port, app, serve_options))

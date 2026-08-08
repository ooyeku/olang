// A notes API served by http.serve: routing with :param segments, JSON in
// and out, and a SQLite store that persists across requests (the handler
// closes over the connection). Run it, then talk to it with curl:
//
//   olang main.ol 8080
//   curl http://127.0.0.1:8080/notes
//   curl -X POST -d '{"text": "ship the release"}' http://127.0.0.1:8080/notes
//   curl http://127.0.0.1:8080/notes/1
//   curl -X DELETE http://127.0.0.1:8080/notes/1

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
            unwrap(db.execute(conn, "INSERT INTO notes (text) VALUES (?)", [doc.text]))
            let made = unwrap(db.query_one(conn, "SELECT id, text FROM notes ORDER BY id DESC LIMIT 1"))
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

// ── the app ──
let routes = [
    route("GET", "/", index_page),
    route("GET", "/notes", list_notes),
    route("POST", "/notes", create_note),
    route("GET", "/notes/:id", get_note),
    route("DELETE", "/notes/:id", delete_note)
]

let args = unwrap(os.args())
let port = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 8080

println("olang notes API — press Ctrl-C to stop")
http.serve(port, (req) => dispatch(routes, req))

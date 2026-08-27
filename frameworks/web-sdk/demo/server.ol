// The SDK's own demo: a full-stack todo app in two small files.
// Run from the SDK root:  olang run demo/server.ol [port]
use lib.routes { rpc }
use lib.server { serve, invalid }
use lib.sql { open_db, rows, row, insert_row, update_row, exec }
use lib.forms { field, rules, read }
use validate { check }

let conn = open_db(
    if len(os.args()) > 2 => os.args()[2] else => ":memory:",
    [["CREATE TABLE todos (
        id INTEGER PRIMARY KEY,
        title TEXT NOT NULL,
        done INTEGER NOT NULL DEFAULT 0
    )"]])

let todo_fields = [field("title", "Title", "text", #{ "min": 1, "max": 120 })]

let routes = [
    rpc("todos.list", (req, p) => rows(conn, "SELECT * FROM todos ORDER BY id", [])),
    rpc("todos.create", (req, p) => {
        let payload = match json.parse(req.body) { Ok(v) => v, Err(e) => #{} }
        let fields = read(payload, todo_fields)
        match check(fields, rules(todo_fields)) {
            Err(problems) => invalid(problems),
            Ok(valid) => {
                let id = insert_row(conn, "todos", map_set(valid, "done", 0))
                row(conn, "SELECT * FROM todos WHERE id = ?", [id])
            }
        }
    }),
    rpc("todos.toggle", (req, p) => {
        let payload = match json.parse(req.body) { Ok(v) => v, Err(e) => #{} }
        let id = map_get(payload, "id")
        let t = row(conn, "SELECT * FROM todos WHERE id = ?", [id])
        if t == () => Err("no todo " + to_string(id))
        else => {
            let n = update_row(conn, "todos", id, #{ "done": 1 - map_get(t, "done") })
            row(conn, "SELECT * FROM todos WHERE id = ?", [id])
        }
    })
]

serve(#{
    "title": "web-sdk demo — todos",
    "routes": routes,
    "client": "demo/client.ol",
    "port": if len(os.args()) > 1 => unwrap(str.parse_int(os.args()[1])) else => 7500
})

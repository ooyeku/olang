// shipit — the web-sdk demo: a project pulse board.
// Run from the SDK root:  olang run demo/server.ol [port] [db]
use lib.routes { rpc }
use lib.server { serve, invalid }
use lib.sql { open_db, rows, row, insert_row, update_row, exec }
use lib.forms { field, rules, read }
use validate { check }

let conn = open_db(
    if len(os.args()) > 2 => os.args()[2] else => ":memory:",
    [["CREATE TABLE tasks (
        id       INTEGER PRIMARY KEY,
        title    TEXT NOT NULL,
        priority TEXT NOT NULL DEFAULT 'ship',
        points   INTEGER NOT NULL DEFAULT 1,
        done     INTEGER NOT NULL DEFAULT 0
    )"],
     ["CREATE INDEX idx_tasks_done ON tasks(done)"]])

// A board that greets you working: seed once, only when empty.
if len(rows(conn, "SELECT id FROM tasks LIMIT 1", [])) == 0 => {
    for t in [
        ["wire the route table", "ship", 3, 1],
        ["design system dark mode", "polish", 2, 1],
        ["bundle the client", "ship", 5, 1],
        ["form validation, both sides", "ship", 3, 0],
        ["chart the burn-up", "polish", 2, 0],
        ["write the chapter", "docs", 1, 0],
        ["live-socket test", "ship", 2, 0]
    ] {
        let n = insert_row(conn, "tasks", #{ "title": t[0], "priority": t[1],
            "points": t[2], "done": t[3] })
    }
}

// One declaration: the form's markup, its validation, and its reader.
let task_fields = [
    field("title", "Title", "text", #{ "min": 1, "max": 80 }),
    field("priority", "Priority", #{ "select": ["ship", "polish", "docs"] },
        #{ "one_of": ["ship", "polish", "docs"] }),
    field("points", "Points", "number", #{ "min": 1, "max": 13 })
]

let routes = [
    rpc("tasks.list", (req, p) =>
        rows(conn, "SELECT * FROM tasks ORDER BY done, id DESC", [])),

    rpc("tasks.create", (req, p) => {
        let payload = match json.parse(req.body) { Ok(v) => v, Err(e) => #{} }
        let fields = read(payload, task_fields)
        match check(fields, rules(task_fields)) {
            Err(problems) => invalid(problems),
            Ok(valid) => {
                let id = insert_row(conn, "tasks", map_set(valid, "done", 0))
                row(conn, "SELECT * FROM tasks WHERE id = ?", [id])
            }
        }
    }),

    rpc("tasks.toggle", (req, p) => {
        let payload = match json.parse(req.body) { Ok(v) => v, Err(e) => #{} }
        let id = map_get(payload, "id")
        let t = row(conn, "SELECT * FROM tasks WHERE id = ?", [id])
        if t == () => Err("no task " + to_string(id))
        else => {
            let n = update_row(conn, "tasks", id, #{ "done": 1 - map_get(t, "done") })
            row(conn, "SELECT * FROM tasks WHERE id = ?", [id])
        }
    }),

    rpc("tasks.delete", (req, p) => {
        let payload = match json.parse(req.body) { Ok(v) => v, Err(e) => #{} }
        let n = exec(conn, "DELETE FROM tasks WHERE id = ?", [map_get(payload, "id")])
        #{ "deleted": n }
    }),

    // SQL aggregates as an endpoint — the stats strip reads this.
    rpc("tasks.stats", (req, p) => {
        let open_row = row(conn,
            "SELECT COUNT(*) AS n, COALESCE(SUM(points), 0) AS pts FROM tasks WHERE done = 0", [])
        let done_row = row(conn,
            "SELECT COUNT(*) AS n, COALESCE(SUM(points), 0) AS pts FROM tasks WHERE done = 1", [])
        #{ "open": map_get(open_row, "n"), "open_pts": map_get(open_row, "pts"),
           "done": map_get(done_row, "n"), "done_pts": map_get(done_row, "pts") }
    })
]

serve(#{
    "title": "shipit — web-sdk demo",
    "routes": routes,
    "client": "demo/client.ol",
    "port": if len(os.args()) > 1 => unwrap(str.parse_int(os.args()[1])) else => 7500
})

// The whole stack over real HTTP, in olang: the test spawns the
// server on a task thread, then drives it with the http client —
// shell, assets, bundle, rpc round-trips, and the error envelope, all
// through a real socket.
use lib.routes { rpc }
use lib.server { serve }
use lib.sql { open_db, rows, row, insert_row }

let port = 41000 + time.monotonic_ms() % 4000
let base = "http://127.0.0.1:" + to_string(port)

let conn = open_db(":memory:", [[
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

let client = if fs.exists("demo/client.ol") => "demo/client.ol" else => "../demo/client.ol"
let server = spawn {
    serve(#{ "title": "live test", "routes": routes, "client": client, "port": port })
}

// Wait for the socket: retry the shell until it answers.
fn wait_ready(tries) = {
    if tries <= 0 => false
    else => match http.get(base + "/") {
        Ok(resp) => true,
        Err(e) => { time.sleep(50) wait_ready(tries - 1) }
    }
}
let ready = wait_ready(100)

test "the server comes up and serves the shell" {
    assert_eq(ready, true)
    let shell = unwrap(http.get(base + "/"))
    assert_eq(shell.status, 200)
    assert_eq(str.contains(shell.body, "olang-dom.js"), true)
    assert_eq(str.contains(shell.body, "web.css"), true)
}

test "assets arrive with their content types" {
    let css = unwrap(http.get(base + "/web.css"))
    assert_eq(css.status, 200)
    assert_eq(str.contains(css.body, "--accent"), true)
    let bundle = unwrap(http.get(base + "/app.ol"))
    assert_eq(bundle.status, 200)
    assert_eq(is_ok(meta.parse(bundle.body)), true)
}

test "rpc round-trips over the wire" {
    let created = unwrap(http.post(base + "/api/rpc/todos.create",
        unwrap(json.stringify(#{ "title": "over the wire" }))))
    assert_eq(created.status, 200)
    let todo = map_get(unwrap(json.parse(created.body)), "data")
    assert_eq(map_get(todo, "title"), "over the wire")

    let listed = unwrap(http.post(base + "/api/rpc/todos.list", "{}"))
    assert_eq(len(map_get(unwrap(json.parse(listed.body)), "data")), 1)
}

test "the error envelope crosses the socket" {
    // A real endpoint with the wrong method: 405. An unknown path: 404.
    let wrong_method = unwrap(http.get(base + "/api/rpc/todos.list"))
    assert_eq(wrong_method.status, 405)
    let gone = unwrap(http.get(base + "/definitely/not/here"))
    assert_eq(gone.status, 404)
    assert_eq(str.contains(gone.body, "not_found"), true)
}

// The whole stack over real HTTP, in olang: the test spawns the
// server on a task thread, then drives it with the http client —
// shell, assets, bundle, rpc round-trips, and the error envelope, all
// through a real socket.
use lib.routes { rpc }
use lib.server { serve }
use lib.sql { open_db, rows, row, insert_row }
use lib.html { div }

let port = 41000 + time.monotonic_ms() % 4000
let base = "http://127.0.0.1:" + to_string(port)
let painted_base = "http://127.0.0.1:" + to_string(port + 1)

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

// A second server with a server-rendered first paint and no client
// program: the shell carries the view over `initial`, and its state.
let painted = spawn {
    serve(#{ "title": "painted", "client": "", "port": port + 1,
             "view": (s) => div(#{ "class": "count" },
                                [to_string(map_get(s, "n")) + " notes"]),
             "initial": () => #{ "n": 3, "tag": "<b>" } })
}

// Wait for a socket: retry the shell until it answers.
fn wait_ready(url, tries) = {
    if tries <= 0 => false
    else => match http.get(url) {
        Ok(resp) => true,
        Err(e) => { time.sleep(50) wait_ready(url, tries - 1) }
    }
}
let ready = wait_ready(base + "/", 100)

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

test "the program image is served, and the source stays as the fallback" {
    let image = unwrap(http.get(base + "/app.olb"))
    assert_eq(image.status, 200)
    // The image header: magic first, then the version that wrote it.
    assert_eq(str.starts_with(image.body, "olb1"), true)
    let shell = unwrap(http.get(base + "/"))
    assert_eq(str.contains(shell.body, "data-bin=\"/app.olb\""), true)
    assert_eq(str.contains(shell.body, "data-src=\"/app.ol\""), true)
    // No view: the mount point is empty and carries no state.
    assert_eq(str.contains(shell.body, "<main id=\"app\"></main>"), true)
}

test "a view renders the first paint into the shell, with its state" {
    assert_eq(wait_ready(painted_base + "/", 100), true)
    let shell = unwrap(http.get(painted_base + "/"))
    assert_eq(shell.status, 200)
    assert_eq(str.contains(shell.body,
        "<main id=\"app\" data-olang-state=\"olang-state\"><div class=\"count\">3 notes</div></main>"), true)
    assert_eq(str.contains(shell.body,
        "<script type=\"application/json\" id=\"olang-state\">"), true)
    // The state is JSON with `<` escaped, so no value can end the script.
    assert_eq(str.contains(shell.body, "\"tag\":\"\\u003cb>\""), true)
    assert_eq(str.contains(shell.body, "\"n\":3"), true)
    // No client program: no image is offered, and /app.olb says so.
    assert_eq(str.contains(shell.body, "data-bin"), false)
    assert_eq(unwrap(http.get(painted_base + "/app.olb")).status, 404)
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

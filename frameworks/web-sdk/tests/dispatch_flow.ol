// End-to-end request flow, no sockets: constructed requests through
// dispatch, real SQL underneath, the SDK envelope on the wire.
use lib.routes { rpc, route }
use lib.server { dispatch, invalid, ok_data }
use lib.sql { open_db, rows, row, insert_row }
use lib.forms { field, rules, read }
use validate { check }

fn fake(method, path, body) =
    { method: method, path: path, query: #{}, headers: #{}, body: body }

fn demo_routes(conn) = {
    let fields = [field("title", "Title", "text", #{ "min": 1, "max": 40 })]
    [
        rpc("todos.list", (req, p) => rows(conn, "SELECT * FROM todos ORDER BY id", [])),
        rpc("todos.create", (req, p) => {
            let payload = match json.parse(req.body) { Ok(v) => v, Err(e) => #{} }
            let f = read(payload, fields)
            match check(f, rules(fields)) {
                Err(problems) => invalid(problems),
                Ok(valid) => {
                    let id = insert_row(conn, "todos", map_set(valid, "done", 0))
                    row(conn, "SELECT * FROM todos WHERE id = ?", [id])
                }
            }
        })
    ]
}

fn fresh() = {
    let conn = open_db(":memory:", [[
        "CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0)"
    ]])
    demo_routes(conn)
}

test "create then list round-trips through the envelope" {
    let table = fresh()
    let created = dispatch(table, fake("POST", "/api/rpc/todos.create", "{\"title\": \"ship\"}"))
    assert_eq(created.status, 200)
    let body = unwrap(json.parse(created.body))
    let todo = map_get(body, "data")
    assert_eq(map_get(todo, "title"), "ship")
    assert_eq(map_get(todo, "done"), 0)

    let listed = dispatch(table, fake("POST", "/api/rpc/todos.list", "{}"))
    let items = map_get(unwrap(json.parse(listed.body)), "data")
    assert_eq(len(items), 1)
}

test "validation failures wear the 422 envelope with details" {
    let table = fresh()
    let r = dispatch(table, fake("POST", "/api/rpc/todos.create", "{\"title\": \"\"}"))
    assert_eq(r.status, 422)
    let e = map_get(unwrap(json.parse(r.body)), "error")
    assert_eq(map_get(e, "code"), "invalid")
    assert_eq(len(map_get(e, "details")), 1)

    let long = dispatch(table, fake("POST", "/api/rpc/todos.create",
        "{\"title\": \"" + str.repeat("x", 50) + "\"}"))
    assert_eq(long.status, 422)
}

test "a malformed body never reaches the data layer" {
    let table = fresh()
    let r = dispatch(table, fake("POST", "/api/rpc/todos.create", "{broken"))
    // The handler's empty-map fallback validates and fails cleanly.
    assert_eq(r.status, 422)
    let listed = dispatch(table, fake("POST", "/api/rpc/todos.list", "{}"))
    assert_eq(len(map_get(unwrap(json.parse(listed.body)), "data")), 0)
}

test "sql injection strings are data, not structure" {
    let table = fresh()
    let evil = "x\"; DROP TABLE todos; --"
    let r = dispatch(table, fake("POST", "/api/rpc/todos.create",
        unwrap(json.stringify(#{ "title": evil }))))
    assert_eq(r.status, 200)
    let listed = dispatch(table, fake("POST", "/api/rpc/todos.list", "{}"))
    let items = map_get(unwrap(json.parse(listed.body)), "data")
    assert_eq(map_get(items[0], "title"), evil)
}

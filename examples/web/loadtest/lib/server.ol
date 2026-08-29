// The system under test: a small API backed by shared SQLite. Every request
// touches the database, so the pool workers write to one connection
// concurrently — the correctness question the load test answers is whether
// the server-side count matches what the clients think they sent.

share fn make_server(port, workers) = {
    let conn = unwrap(db.open(":memory:"))
    unwrap(db.execute(conn, "CREATE TABLE hits (id INTEGER PRIMARY KEY, route TEXT)"))

    // The handler closes over `conn` — the worker threads share it.
    fn handler(req) = {
        if req.path == "/oops" => {
            // A route that deliberately fails, to prove error responses don't
            // corrupt the pool: still records the hit, then returns 500.
            unwrap(db.execute(conn, "INSERT INTO hits (route) VALUES (?)", [req.path]))
            http.response(500, "deliberate failure")
        }
        else if req.path == "/count" => {
            let row = unwrap(db.query_one(conn, "SELECT COUNT(*) AS c FROM hits"))
            http.response_with_headers(200, unwrap(json.stringify(row)),
                #{ "Content-Type": "application/json" })
        }
        else => {
            unwrap(db.execute(conn, "INSERT INTO hits (route) VALUES (?)", [req.path]))
            "ok " + req.path
        }
    }

    let server = spawn http.serve(port, handler, #{ "workers": workers, "queue_capacity": 256 })
    { conn: conn, server: server }
}

// Total hits the server actually recorded — the ground truth clients are
// checked against.
share fn server_hit_count(conn) =
    map_get(unwrap(db.query_one(conn, "SELECT COUNT(*) AS c FROM hits")), "c")

// log_store.ol's protocol on SQLite: one table, a put a transaction
// that reads the key's version and writes the next. The four workers
// share one connection; db.transaction serializes them.
//
//   POST /put  #{ "key": k, "version": n }   -> 200 once committed, 409 + the held version
//   GET  /get?key=k                          -> #{ "version": n }
//
// SYNC (env) is SQLite's synchronous setting (FULL unless given); the
// journal is a WAL.

let sync = match os.get_env("SYNC") { Ok(s) => s, Err(e) => "FULL" }
if !fs.exists("/data") => unwrap(fs.create_dir("/data")) else => ()
let conn = unwrap(db.open("/data/kv.db"))
let wal = unwrap(db.query(conn, "PRAGMA journal_mode=WAL", []))
let synced = unwrap(db.query(conn, "PRAGMA synchronous=" + sync, []))
unwrap(db.execute(conn, "CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, version INTEGER NOT NULL)", []))

fn held(c, key) = {
    let rows = unwrap(db.query(c, "SELECT version FROM kv WHERE key = ?", [key]))
    if len(rows) == 0 => 0 else => rows[0].version
}

fn put(key, version) = unwrap(db.transaction(conn, (c) => {
    let h = held(c, key)
    if version == h + 1 => {
        unwrap(db.execute(c, "INSERT INTO kv (key, version) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET version = excluded.version", [key, version]))
        Ok(#{ "ok": true, "version": version })
    }
    else => Ok(#{ "ok": version == h, "version": h })
}))

fn handle(req) = {
    if req.method == "POST" && req.path == "/put" => {
        let r = unwrap(json.parse(req.body))
        let a = put(r.key, r.version)
        http.response(if a.ok => 200 else => 409, unwrap(json.stringify(#{ "version": a.version })))
    }
    else if req.path == "/get" => http.response(200, unwrap(json.stringify(#{ "version": held(conn, req.query.key) })))
    else => http.response(404, "no route for " + req.path)
}

http.serve(7600, handle)

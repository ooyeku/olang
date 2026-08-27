//! sql — the data layer's disciplines, packaged.
//!
//! `open(path, migrations)` gives every app the same guarantees the
//! example apps hand-rolled: a `schema_version` table, migrations
//! applied once, in order, each inside a transaction — adding
//! capability later means appending a migration, never editing one.
//! The helpers wrap the patterns that keep SQLite honest: positional
//! `?` parameters only, explicit rollback on a failed transactional
//! branch, and single-row lookups that distinguish "absent" from
//! "failed".

/// Open (or create) the database at `path` and bring it to the head
/// of `migrations` — a list of versions, each a list of statements.
/// Version N runs only when the recorded schema version is below N,
/// inside its own transaction. Returns the connection.
share fn open_db(path, migrations) = {
    let conn = unwrap(db.open(path))
    unwrap(db.execute(conn, "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)"))
    let rows = unwrap(db.query(conn, "SELECT version FROM schema_version"))
    let mut v = if len(rows) == 0 => {
        unwrap(db.execute(conn, "INSERT INTO schema_version (version) VALUES (0)"))
        0
    } else => map_get(rows[0], "version")
    while v < len(migrations) {
        unwrap(db.begin(conn))
        let mut failed = ""
        for stmt in migrations[v] {
            if failed == "" => {
                match db.execute(conn, stmt) {
                    Err(e) => { failed = show(e) },
                    Ok(x) => x
                }
            }
        }
        if failed != "" => {
            let r = db.rollback(conn)
            // The failing statement is the bug report — surface it.
            unwrap(Err("migration v" + to_string(v + 1) + " failed: " + failed))
        }
        unwrap(db.execute(conn, "UPDATE schema_version SET version = ?", [v + 1]))
        unwrap(db.commit(conn))
        v = v + 1
    }
    conn
}

/// The recorded schema version.
share fn version(conn) = {
    let rows = unwrap(db.query(conn, "SELECT version FROM schema_version"))
    if len(rows) == 0 => 0 else => map_get(rows[0], "version")
}

/// All rows for a parameterized query.
share fn rows(conn, query, params) = unwrap(db.query(conn, query, params))

/// One row or Unit — absence is a value, not an error.
share fn row(conn, query, params) = {
    let rows = unwrap(db.query(conn, query, params))
    if len(rows) == 0 => () else => rows[0]
}

/// Execute a parameterized statement; the affected-row count.
share fn exec(conn, statement, params) = unwrap(db.execute(conn, statement, params))

/// Insert from a map: `insert(conn, "todos", #{ "title": t })` builds
/// the column list and placeholders, so field sets stay in one place.
/// Returns the new row id.
share fn insert_row(conn, table_name, fields) = {
    let cols = sort(map_keys(fields))
    let marks = map(cols, (c) => "?") |> join(", ")
    let vals = map(cols, (c) => map_get(fields, c))
    unwrap(db.execute(conn,
        "INSERT INTO " + table_name + " (" + join(cols, ", ") + ") VALUES (" + marks + ")",
        vals))
    map_get(unwrap(db.query(conn, "SELECT last_insert_rowid() AS id"))[0], "id")
}

/// Update by id from a map; the affected-row count (0 = no such row).
share fn update_row(conn, table_name, id, fields) = {
    let cols = sort(map_keys(fields))
    if len(cols) == 0 => 0
    else => {
        let sets = map(cols, (c) => c + " = ?") |> join(", ")
        let vals = map(cols, (c) => map_get(fields, c)) + [id]
        unwrap(db.execute(conn,
            "UPDATE " + table_name + " SET " + sets + " WHERE id = ?", vals))
    }
}

/// Run `f(conn)` inside a transaction: commit on Ok, roll back on Err,
/// handing the result through either way.
share fn tx(conn, f) = {
    unwrap(db.begin(conn))
    let result = f(conn)
    match result {
        Err(e) => {
            let r = db.rollback(conn)
            Err(e)
        },
        other => {
            unwrap(db.commit(conn))
            result
        }
    }
}

test "migrations apply once, in order, and record the version" {
    let conn = open_db(":memory:", [
        ["CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT NOT NULL)"],
        ["ALTER TABLE todos ADD COLUMN done INTEGER NOT NULL DEFAULT 0"]
    ])
    assert_eq(version(conn), 2)
    let id = insert_row(conn, "todos", #{ "title": "ship" })
    assert_eq(map_get(row(conn, "SELECT * FROM todos WHERE id = ?", [id]), "done"), 0)
}

test "insert, update, one, and absence as Unit" {
    let conn = open_db(":memory:", [
        ["CREATE TABLE t (id INTEGER PRIMARY KEY, a TEXT NOT NULL, b INTEGER NOT NULL DEFAULT 0)"]
    ])
    let id = insert_row(conn, "t", #{ "a": "x", "b": 1 })
    assert_eq(update_row(conn, "t", id, #{ "a": "y" }), 1)
    assert_eq(map_get(row(conn, "SELECT * FROM t WHERE id = ?", [id]), "a"), "y")
    assert_eq(row(conn, "SELECT * FROM t WHERE id = ?", [999]), ())
    assert_eq(update_row(conn, "t", 999, #{ "a": "z" }), 0)
}

test "tx commits on Ok and rolls back on Err" {
    let conn = open_db(":memory:", [["CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)"]])
    let ok = tx(conn, (c) => Ok(insert_row(c, "t", #{ "n": 1 })))
    assert_eq(is_ok(ok), true)
    let bad = tx(conn, (c) => {
        let x = insert_row(c, "t", #{ "n": 2 })
        Err("nope")
    })
    assert_eq(bad, Err("nope"))
    assert_eq(len(rows(conn, "SELECT * FROM t", [])), 1)
}

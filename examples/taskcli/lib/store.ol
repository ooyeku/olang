// The storage layer: a SQLite-backed task store. All SQL is parameterized.

share fn open_store(path) = {
    let conn = unwrap(db.open(path))
    unwrap(db.execute(conn, "
        CREATE TABLE IF NOT EXISTS tasks (
            id       INTEGER PRIMARY KEY AUTOINCREMENT,
            title    TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 1,
            done     INTEGER NOT NULL DEFAULT 0
        )
    "))
    conn
}

share fn add_task(conn, title, priority) =
    unwrap(db.execute(conn, "INSERT INTO tasks (title, priority) VALUES (?, ?)", [title, priority]))

share fn complete_task(conn, id) =
    unwrap(db.execute(conn, "UPDATE tasks SET done = 1 WHERE id = ?", [id]))

share fn all_tasks(conn) =
    unwrap(db.query(conn, "SELECT id, title, priority, done FROM tasks ORDER BY done, priority DESC, id"))

share fn open_tasks(conn) =
    unwrap(db.query(conn, "SELECT id, title, priority, done FROM tasks WHERE done = 0 ORDER BY priority DESC, id"))

share fn close_store(conn) = unwrap(db.close(conn))

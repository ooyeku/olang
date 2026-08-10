// The tracker's data layer: SQLite with real migrations, relations,
// an audit trail, and transactions where multi-statement consistency
// matters. Every function returns plain values or Results — no HTTP in
// here (the handlers own status codes).

// ── schema migrations ────────────────────────────────────────────────
// A schema_version table records the applied version; each migration
// runs once, in order, inside a transaction. Adding capability later
// means appending a migration, never editing one.

share fn open_store(path) = {
    let conn = unwrap(db.open(path))
    unwrap(db.execute(conn, "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)"))
    let rows = unwrap(db.query(conn, "SELECT version FROM schema_version"))
    let current = if len(rows) == 0 => {
        unwrap(db.execute(conn, "INSERT INTO schema_version (version) VALUES (0)"))
        0
    } else => map_get(rows[0], "version")
    migrate(conn, current)
    conn
}

fn migrate(conn, from) = {
    let migrations = [
        // v1 — the original issues table
        ["CREATE TABLE IF NOT EXISTS issues (
            id       INTEGER PRIMARY KEY,
            title    TEXT NOT NULL,
            status   TEXT NOT NULL DEFAULT 'open',
            priority TEXT NOT NULL DEFAULT 'medium',
            assignee TEXT NOT NULL DEFAULT '',
            points   INTEGER NOT NULL DEFAULT 0,
            notes    TEXT NOT NULL DEFAULT '',
            updated  TEXT NOT NULL
        )"],
        // v2 — comments, the audit trail, and the indexes the list
        // endpoint's filters lean on
        ["CREATE TABLE comments (
            id       INTEGER PRIMARY KEY,
            issue_id INTEGER NOT NULL,
            author   TEXT NOT NULL DEFAULT '',
            text     TEXT NOT NULL,
            at       TEXT NOT NULL
        )",
         "CREATE TABLE events (
            id       INTEGER PRIMARY KEY,
            action   TEXT NOT NULL,
            issue_id INTEGER,
            detail   TEXT NOT NULL DEFAULT '',
            at       TEXT NOT NULL
        )",
         "CREATE INDEX idx_issues_status ON issues(status)",
         "CREATE INDEX idx_issues_assignee ON issues(assignee)",
         "CREATE INDEX idx_comments_issue ON comments(issue_id)"]
    ]
    let mut v = from
    while v < len(migrations) {
        unwrap(db.begin(conn))
        for stmt in migrations[v] { unwrap(db.execute(conn, stmt)) }
        unwrap(db.execute(conn, "UPDATE schema_version SET version = ?", [v + 1]))
        unwrap(db.commit(conn))
        v = v + 1
    }
    v
}

share fn seed_if_empty(conn) = {
    let n = map_get(unwrap(db.query_one(conn, "SELECT COUNT(*) AS n FROM issues")), "n")
    if n == 0 => {
        let rows = [
            ["Ship the tracker example", "in-progress", "high", "ada", 3, "the app you are looking at"],
            ["Spreadsheet keyboard nav", "open", "medium", "grace", 2, "enter commits + moves down"],
            ["Wire up column sorting", "done", "low", "ada", 1, "click a header"],
            ["Decide on dark mode", "open", "low", "", 1, ""]
        ]
        for r in rows {
            unwrap(db.execute(conn,
                "INSERT INTO issues (title, status, priority, assignee, points, notes, updated)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                concat(r, [dates.utc_now()])))
        }
        log_event(conn, "seed", 0, "seeded " + show(len(rows)) + " issues")
    }
}

share let select_cols = "id, title, status, priority, assignee, points, notes, updated"

// ── issues ───────────────────────────────────────────────────────────

// Filtered, sorted, paged listing. `filters` is a struct built by the
// handler from validated query params — every column name below comes
// from a whitelist there, never from the request.
share fn list_issues(conn, f) = {
    let mut where_parts = []
    let mut vals = []
    if f.status != "" => {
        where_parts = concat(where_parts, ["status = ?"])
        vals = concat(vals, [f.status])
    }
    if f.assignee != "" => {
        where_parts = concat(where_parts, ["assignee = ?"])
        vals = concat(vals, [f.assignee])
    }
    if f.q != "" => {
        where_parts = concat(where_parts, ["(title LIKE ? OR notes LIKE ?)"])
        let needle = "%" + f.q + "%"
        vals = concat(vals, [needle, needle])
    }
    let where_sql = if len(where_parts) == 0 => "" else => " WHERE " + join(where_parts, " AND ")
    let total = map_get(unwrap(db.query_one(conn,
        "SELECT COUNT(*) AS n FROM issues" + where_sql, vals)), "n")
    let items = unwrap(db.query(conn,
        "SELECT " + select_cols + " FROM issues" + where_sql +
        " ORDER BY " + f.sort + " " + f.order + " LIMIT ? OFFSET ?",
        concat(vals, [f.limit, f.offset])))
    { items: items, total: total }
}

// query_one yields Ok(Unit) when no row matches — normalize both that
// and driver errors to Err, so callers see one "not found" shape.
share fn get_issue(conn, id) = match db.query_one(conn,
    "SELECT " + select_cols + " FROM issues WHERE id = ?", [id]) {
    Err(e) => Err("not found"),
    Ok(row) => if typeof(row) == "Unit" => Err("not found") else => Ok(row)
}

share fn create_issue(conn, fields) = {
    let made = unwrap(db.query_one(conn,
        "INSERT INTO issues (title, status, priority, assignee, points, notes, updated)
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING " + select_cols,
        [map_get(fields, "title"), map_get(fields, "status"),
         map_get(fields, "priority"), map_get(fields, "assignee"),
         map_get(fields, "points"), map_get(fields, "notes"), dates.utc_now()]))
    log_event(conn, "create", map_get(made, "id"), map_get(fields, "title"))
    made
}

// Partial update from a validated field map; returns Err when the id
// does not exist.
share fn update_issue(conn, id, fields) = {
    let mut sets = []
    let mut vals = []
    for col in ["title", "status", "priority", "assignee", "points", "notes"] {
        if map_has_key(fields, col) => {
            sets = concat(sets, [col + " = ?"])
            vals = concat(vals, [map_get(fields, col)])
        }
    }
    let rows = unwrap(db.query(conn,
        "UPDATE issues SET " + join(sets, ", ") + ", updated = ? WHERE id = ? RETURNING " + select_cols,
        concat(vals, [dates.utc_now(), id])))
    if len(rows) == 0 => Err("not found")
    else => {
        log_event(conn, "update", to_int(id), join(map_keys(fields), ","))
        Ok(rows[0])
    }
}

// Deleting an issue removes its comments too — one transaction, so a
// failure can never leave orphaned rows.
share fn delete_issue(conn, id) = {
    match get_issue(conn, id) {
        Err(e) => Err("not found"),
        Ok(issue) => {
            unwrap(db.begin(conn))
            unwrap(db.execute(conn, "DELETE FROM comments WHERE issue_id = ?", [id]))
            unwrap(db.execute(conn, "DELETE FROM issues WHERE id = ?", [id]))
            unwrap(db.commit(conn))
            log_event(conn, "delete", to_int(id), map_get(issue, "title"))
            Ok(issue)
        }
    }
}

// ── comments ─────────────────────────────────────────────────────────

share fn list_comments(conn, issue_id) = unwrap(db.query(conn,
    "SELECT id, issue_id, author, text, at FROM comments WHERE issue_id = ? ORDER BY id", [issue_id]))

share fn add_comment(conn, issue_id, author, text) = {
    let made = unwrap(db.query_one(conn,
        "INSERT INTO comments (issue_id, author, text, at) VALUES (?, ?, ?, ?)
         RETURNING id, issue_id, author, text, at",
        [issue_id, author, text, dates.utc_now()]))
    log_event(conn, "comment", to_int(issue_id), author)
    made
}

// ── the audit trail ──────────────────────────────────────────────────

share fn log_event(conn, action, issue_id, detail) =
    unwrap(db.execute(conn,
        "INSERT INTO events (action, issue_id, detail, at) VALUES (?, ?, ?, ?)",
        [action, issue_id, detail, dates.utc_now()]))

share fn recent_events(conn, limit) = unwrap(db.query(conn,
    "SELECT id, action, issue_id, detail, at FROM events ORDER BY id DESC LIMIT ?", [limit]))

// ── rollups for /api/stats ───────────────────────────────────────────

share fn stats(conn) = {
    let by_status = unwrap(db.query(conn,
        "SELECT status, COUNT(*) AS n, SUM(points) AS points FROM issues GROUP BY status ORDER BY status"))
    let by_assignee = unwrap(db.query(conn,
        "SELECT COALESCE(NULLIF(assignee, ''), '(unassigned)') AS assignee,
                COUNT(*) AS n, SUM(points) AS points
         FROM issues GROUP BY assignee ORDER BY points DESC"))
    let totals = unwrap(db.query_one(conn,
        "SELECT COUNT(*) AS issues, COALESCE(SUM(points), 0) AS points FROM issues"))
    let point_rows = unwrap(db.query(conn, "SELECT points FROM issues"))
    { by_status: by_status, by_assignee: by_assignee, totals: totals,
      point_values: point_rows |> map((r) => map_get(r, "points")) }
}

share fn all_issues(conn) = unwrap(db.query(conn,
    "SELECT " + select_cols + " FROM issues ORDER BY id"))

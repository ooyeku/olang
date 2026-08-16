// The ledger's data layer: SQLite with versioned migrations and
// transactions where multi-statement consistency matters. Money is
// integer cents everywhere in this file; dates are "YYYY-MM-DD" strings
// and months are "YYYY-MM" keys. No HTTP in here — handlers own status
// codes; functions return plain values or Results.

// ── schema migrations ────────────────────────────────────────────────
// A schema_version table records the applied version; each migration
// runs once, in order, inside a transaction. Adding capability later
// means appending a migration, never editing one.

share fn open_store(path) = {
    let conn = unwrap(db.open(path))
    // SQLite does not enforce REFERENCES without this pragma; it is
    // per-connection, so it lives here rather than in a migration.
    unwrap(db.execute(conn, "PRAGMA foreign_keys = ON"))
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
        // v1 — categories, transactions, budgets
        ["CREATE TABLE categories (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL UNIQUE,
            kind       TEXT NOT NULL CHECK (kind IN ('expense', 'income')),
            created_at TEXT NOT NULL
        )",
         "CREATE TABLE transactions (
            id           INTEGER PRIMARY KEY,
            date         TEXT NOT NULL,
            amount_cents INTEGER NOT NULL,
            category_id  INTEGER NOT NULL REFERENCES categories(id),
            note         TEXT NOT NULL DEFAULT '',
            created_at   TEXT NOT NULL,
            updated_at   TEXT NOT NULL
        )",
         "CREATE TABLE budgets (
            category_id  INTEGER NOT NULL REFERENCES categories(id),
            month        TEXT NOT NULL,
            amount_cents INTEGER NOT NULL,
            PRIMARY KEY (category_id, month)
        )",
         "CREATE INDEX idx_transactions_date ON transactions(date)",
         "CREATE INDEX idx_transactions_category ON transactions(category_id)"]
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

// First run seeds a starter category set only — never transactions.
share fn seed_if_empty(conn) = {
    let n = map_get(unwrap(db.query_one(conn, "SELECT COUNT(*) AS n FROM categories")), "n")
    if n == 0 => {
        let starters = [
            ["Groceries", "expense"], ["Rent", "expense"], ["Transport", "expense"],
            ["Dining", "expense"], ["Utilities", "expense"], ["Health", "expense"],
            ["Entertainment", "expense"], ["Salary", "income"]
        ]
        for c in starters {
            unwrap(db.execute(conn,
                "INSERT INTO categories (name, kind, created_at) VALUES (?, ?, ?)",
                concat(c, [dates.utc_now()])))
        }
    }
}

// ── transactions ─────────────────────────────────────────────────────

share let tx_cols = "t.id, t.date, t.amount_cents, t.category_id, t.note,
    t.created_at, t.updated_at, c.name AS category, c.kind AS kind"

// All rows whose month key falls in [from_month, to_month], newest first.
// Month keys are "YYYY-MM", so lexicographic comparison is date order.
share fn list_transactions(conn, from_month, to_month) = unwrap(db.query(conn,
    "SELECT " + tx_cols + " FROM transactions t
     JOIN categories c ON c.id = t.category_id
     WHERE substr(t.date, 1, 7) >= ? AND substr(t.date, 1, 7) <= ?
     ORDER BY t.date DESC, t.id DESC",
    [from_month, to_month]))

// query_one yields Ok(Unit) when no row matches — normalize both that
// and driver errors to Err, so callers see one "not found" shape.
share fn get_transaction(conn, id) = match db.query_one(conn,
    "SELECT " + tx_cols + " FROM transactions t
     JOIN categories c ON c.id = t.category_id WHERE t.id = ?", [id]) {
    Err(e) => Err("not found"),
    Ok(row) => if typeof(row) == "Unit" => Err("not found") else => Ok(row)
}

share fn create_transaction(conn, fields) = {
    let now = dates.utc_now()
    let made = unwrap(db.query_one(conn,
        "INSERT INTO transactions (date, amount_cents, category_id, note, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
        [map_get(fields, "date"), map_get(fields, "amount_cents"),
         map_get(fields, "category_id"), map_get(fields, "note"), now, now]))
    unwrap(get_transaction(conn, map_get(made, "id")))
}

// Partial update from a validated field map; Err when the id is unknown.
share fn update_transaction(conn, id, fields) = {
    let mut sets = []
    let mut vals = []
    for col in ["date", "amount_cents", "category_id", "note"] {
        if map_has_key(fields, col) => {
            sets = concat(sets, [col + " = ?"])
            vals = concat(vals, [map_get(fields, col)])
        }
    }
    let n = unwrap(db.execute(conn,
        "UPDATE transactions SET " + join(sets, ", ") + ", updated_at = ? WHERE id = ?",
        concat(vals, [dates.utc_now(), id])))
    if n == 0 => Err("not found") else => get_transaction(conn, id)
}

share fn delete_transaction(conn, id) = {
    match get_transaction(conn, id) {
        Err(e) => Err("not found"),
        Ok(row) => {
            unwrap(db.execute(conn, "DELETE FROM transactions WHERE id = ?", [id]))
            Ok(row)
        }
    }
}

// ── categories ───────────────────────────────────────────────────────

// Each category carries its live transaction count, so the UI can show
// which are deletable without extra requests.
share fn list_categories(conn) = unwrap(db.query(conn,
    "SELECT c.id, c.name, c.kind, c.created_at,
            (SELECT COUNT(*) FROM transactions t WHERE t.category_id = c.id) AS tx_count
     FROM categories c ORDER BY c.kind, c.name"))

// UNIQUE(name) violations surface as Err from the driver; the handler
// maps that onto a 422.
share fn create_category(conn, name, kind) = match db.query_one(conn,
    "INSERT INTO categories (name, kind, created_at) VALUES (?, ?, ?)
     RETURNING id, name, kind, created_at, 0 AS tx_count",
    [name, kind, dates.utc_now()]) {
    Err(e) => Err("name already exists"),
    Ok(row) => Ok(row)
}

share fn rename_category(conn, id, name) = match db.query_one(conn,
    "UPDATE categories SET name = ? WHERE id = ?
     RETURNING id, name, kind, created_at,
       (SELECT COUNT(*) FROM transactions t WHERE t.category_id = categories.id) AS tx_count",
    [name, id]) {
    Err(e) => Err("name already exists"),
    Ok(row) => if typeof(row) == "Unit" => Err("not found") else => Ok(row)
}

// A category with transactions (or budgets) cannot be deleted — the
// caller turns "in use" into a 409. Budget rows for an unused category
// are removed in the same transaction as the category itself.
share fn delete_category(conn, id) = {
    let used = map_get(unwrap(db.query_one(conn,
        "SELECT COUNT(*) AS n FROM transactions WHERE category_id = ?", [id])), "n")
    if used > 0 => Err("in use")
    else => {
        unwrap(db.begin(conn))
        unwrap(db.execute(conn, "DELETE FROM budgets WHERE category_id = ?", [id]))
        let n = unwrap(db.execute(conn, "DELETE FROM categories WHERE id = ?", [id]))
        unwrap(db.commit(conn))
        if n == 0 => Err("not found") else => Ok(id)
    }
}

// ── budgets ──────────────────────────────────────────────────────────

share fn list_budgets(conn, from_month, to_month) = unwrap(db.query(conn,
    "SELECT b.category_id, b.month, b.amount_cents, c.name AS category
     FROM budgets b JOIN categories c ON c.id = b.category_id
     WHERE b.month >= ? AND b.month <= ?
     ORDER BY b.month, c.name",
    [from_month, to_month]))

// Upsert: one budget row per (category, month). A zero amount clears it.
share fn put_budget(conn, category_id, month, amount_cents) = {
    if amount_cents == 0 => {
        unwrap(db.execute(conn,
            "DELETE FROM budgets WHERE category_id = ? AND month = ?", [category_id, month]))
        Ok(#{ "category_id": category_id, "month": month, "amount_cents": 0 })
    } else => match db.query_one(conn,
        "INSERT INTO budgets (category_id, month, amount_cents) VALUES (?, ?, ?)
         ON CONFLICT (category_id, month) DO UPDATE SET amount_cents = excluded.amount_cents
         RETURNING category_id, month, amount_cents", [category_id, month, amount_cents]) {
        Err(e) => Err("unknown category"),
        Ok(row) => Ok(row)
    }
}

// ── module self-checks (olang test .) ────────────────────────────────

test "migrations, seed, and transaction round-trip" {
    let conn = open_store(":memory:")
    seed_if_empty(conn)
    let cats = list_categories(conn)
    assert_true(len(cats) >= 8, "starter categories seeded")
    let cid = map_get(cats[0], "id")
    let made = create_transaction(conn, #{
        "date": "2026-08-15", "amount_cents": -1250, "category_id": cid, "note": "coffee" })
    assert_eq(map_get(made, "amount_cents"), -1250)
    let listed = list_transactions(conn, "2026-08", "2026-08")
    assert_eq(len(listed), 1)
    let up = unwrap(update_transaction(conn, map_get(made, "id"), #{ "amount_cents": -1300 }))
    assert_eq(map_get(up, "amount_cents"), -1300)
    assert_true(is_ok(delete_transaction(conn, map_get(made, "id"))), "delete works")
    assert_true(is_err(get_transaction(conn, map_get(made, "id"))), "gone after delete")
}

test "category delete guards and budget upsert" {
    let conn = open_store(":memory:")
    seed_if_empty(conn)
    let cats = list_categories(conn)
    let cid = map_get(cats[0], "id")
    let t = create_transaction(conn, #{
        "date": "2026-08-01", "amount_cents": -100, "category_id": cid, "note": "" })
    assert_true(is_err(delete_category(conn, cid)), "in-use category refuses delete")
    let b1 = unwrap(put_budget(conn, cid, "2026-08", 50000))
    assert_eq(map_get(b1, "amount_cents"), 50000)
    let b2 = unwrap(put_budget(conn, cid, "2026-08", 60000))
    assert_eq(map_get(b2, "amount_cents"), 60000)
    assert_eq(len(list_budgets(conn, "2026-08", "2026-08")), 1)
    unwrap(put_budget(conn, cid, "2026-08", 0))
    assert_eq(len(list_budgets(conn, "2026-08", "2026-08")), 0)
}

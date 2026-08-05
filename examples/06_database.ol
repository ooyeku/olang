// ═══════════════════════════════════════════════════════════════════
// 06 — Database
// A small task-tracker backed by SQLite: schema, seeding with bound
// parameters, queries, aggregates, and updates. Real persistence.
// ═══════════════════════════════════════════════════════════════════

println("═══ task tracker (SQLite) ═══")

// Open an in-memory database (use a file path to persist to disk).
let db_conn = unwrap(db.open(":memory:"))

// ── Schema ──────────────────────────────────────────────────────────
unwrap(db.execute(db_conn, "
    CREATE TABLE tasks (
        id       INTEGER PRIMARY KEY,
        title    TEXT NOT NULL,
        priority INTEGER NOT NULL,
        done     INTEGER NOT NULL DEFAULT 0,
        assignee TEXT
    )
"))

// ── Seed data with bound parameters (never string-splice SQL) ───────
let seed = "INSERT INTO tasks (title, priority, assignee) VALUES (?, ?, ?)"
let rows = [
    ["Write the parser",     1, "Ann"],
    ["Fix the tier bug",     1, "Bob"],
    ["Update the docs",      3, "Ann"],
    ["Add the db module",    2, "Cy"],
    ["Review the PR",        2, "Bob"],
    ["Plan the release",     3, "Ann"]
]
for row in rows {
    unwrap(db.execute(db_conn, seed, row))
}
println(`seeded ${len(rows)} tasks`)

// ── Query: open tasks by priority ───────────────────────────────────
println("── open tasks (highest priority first) ──")
let open_tasks = unwrap(db.query(db_conn,
    "SELECT title, priority, assignee FROM tasks WHERE done = 0 ORDER BY priority, id"))
for task in open_tasks {
    let p = map_get(task, "priority")
    println(`  [P${p}] ${str.pad_end(map_get(task, "title"), 20, " ")} ${map_get(task, "assignee")}`)
}

// ── Parameterized filter: one person's workload ─────────────────────
let who = "Ann"
let mine = unwrap(db.query(db_conn,
    "SELECT title FROM tasks WHERE assignee = ? ORDER BY priority", [who]))
println(`── ${who}'s tasks (${len(mine)}) ──`)
for task in mine {
    println(`  ${map_get(task, "title")}`)
}

// ── Aggregates: workload per assignee ───────────────────────────────
println("── workload by assignee ──")
let workload = unwrap(db.query(db_conn, "
    SELECT assignee, COUNT(*) AS n, AVG(priority) AS avg_p
    FROM tasks GROUP BY assignee ORDER BY n DESC
"))
for w in workload {
    println(`  ${str.pad_end(map_get(w, "assignee"), 5, " ")} ${map_get(w, "n")} tasks, avg priority ${map_get(w, "avg_p")}`)
}

// ── Update: mark a task done, report rows affected ──────────────────
let affected = unwrap(db.execute(db_conn,
    "UPDATE tasks SET done = 1 WHERE title = ?", ["Review the PR"]))
println(`marked ${affected} task done`)

// ── Single-row aggregate ────────────────────────────────────────────
let summary = unwrap(db.query_one(db_conn, "
    SELECT COUNT(*) AS total,
           SUM(done) AS completed,
           SUM(CASE WHEN done = 0 THEN 1 ELSE 0 END) AS remaining
    FROM tasks
"))
println(`── summary ──`)
println(`  total: ${map_get(summary, "total")}, completed: ${map_get(summary, "completed")}, remaining: ${map_get(summary, "remaining")}`)

// ── Clean up ────────────────────────────────────────────────────────
unwrap(db.close(db_conn))
println("═══ tracker complete ═══")

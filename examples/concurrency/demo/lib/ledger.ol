// ledger — the harbor's system of record: SQLite with bound parameters,
// transactions around each day's charges, and the aggregate queries the
// reports are built from.
//
// The connection is owned by the world and threaded through, like all
// state. Every write is parameterized — SQL is never string-spliced.

// ── Schema ──────────────────────────────────────────────────────────
share fn open_ledger(path: String) = {
    let conn = unwrap(db.open(path))
    unwrap(db.execute(conn, "
        CREATE TABLE IF NOT EXISTS movements (
            id        INTEGER PRIMARY KEY,
            tick      INTEGER NOT NULL,
            callsign  TEXT NOT NULL,
            vessel    TEXT NOT NULL,
            event     TEXT NOT NULL,      -- arrived | berthed | departed
            berth     INTEGER
        )
    "))
    unwrap(db.execute(conn, "
        CREATE TABLE IF NOT EXISTS charges (
            id        INTEGER PRIMARY KEY,
            tick      INTEGER NOT NULL,
            callsign  TEXT NOT NULL,
            kind      TEXT NOT NULL,      -- container | bulk | reefer | hazmat
            units     REAL NOT NULL,
            amount    REAL NOT NULL
        )
    "))
    conn
}

share fn close_ledger(conn) = unwrap(db.close(conn))

// ── Writes (all parameterized) ──────────────────────────────────────
share fn record_movement(conn, tick: Int, v, event: String, berth: Int) =
    unwrap(db.execute(conn,
        "INSERT INTO movements (tick, callsign, vessel, event, berth) VALUES (?, ?, ?, ?, ?)",
        [tick, v.callsign, v.name, event, berth]))

// A departed vessel settles all her cargo charges atomically: the whole
// invoice lands, or none of it does.
share fn settle_charges(conn, tick: Int, callsign: String, items) = {
    unwrap(db.begin(conn))
    for item in items {
        unwrap(db.execute(conn,
            "INSERT INTO charges (tick, callsign, kind, units, amount) VALUES (?, ?, ?, ?, ?)",
            [tick, callsign, item.kind, item.units, item.amount]))
    }
    unwrap(db.commit(conn))
    len(items)
}

// ── Reads ───────────────────────────────────────────────────────────
share fn revenue_by_kind(conn) = unwrap(db.query(conn, "
    SELECT kind, COUNT(*) AS n, SUM(amount) AS revenue
    FROM charges GROUP BY kind ORDER BY revenue DESC
"))

share fn busiest_berths(conn) = unwrap(db.query(conn, "
    SELECT berth, COUNT(*) AS visits FROM movements
    WHERE event = 'berthed' GROUP BY berth ORDER BY visits DESC
"))

share fn movements_for(conn, callsign: String) = unwrap(db.query(conn,
    "SELECT tick, event, berth FROM movements WHERE callsign = ? ORDER BY tick", [callsign]))

share fn charges_between(conn, from_tick: Int, to_tick: Int) = unwrap(db.query(conn,
    "SELECT callsign, kind, units, amount FROM charges WHERE tick >= ? AND tick < ? ORDER BY id",
    [from_tick, to_tick]))

// One-row totals for the dashboard and the invariant checks.
share fn ledger_totals(conn) = unwrap(db.query_one(conn, "
    SELECT (SELECT COUNT(*) FROM movements)                          AS movements,
           (SELECT COUNT(*) FROM movements WHERE event = 'departed') AS departures,
           (SELECT COUNT(*) FROM charges)                            AS charge_rows,
           (SELECT COALESCE(SUM(amount), 0.0) FROM charges)          AS revenue
"))

// ── Archival pruning ────────────────────────────────────────────────
// An infinite run must not grow without bound. Rows older than the
// horizon are pruned once their day is sealed into the signed digest
// chain — the chain is the archive; the live tables are a window.
// Returns what was dropped so the caller can keep cumulative counters
// (the invariants compare world totals against live + archived).
share fn prune_before(conn, tick: Int) = {
    let dropped = unwrap(db.query_one(conn, "
        SELECT (SELECT COUNT(*) FROM movements WHERE tick < ?1)                          AS movements,
               (SELECT COUNT(*) FROM movements WHERE tick < ?1 AND event = 'departed')   AS departures,
               (SELECT COUNT(*) FROM charges WHERE tick < ?1)                            AS charge_rows,
               (SELECT COALESCE(SUM(amount), 0.0) FROM charges WHERE tick < ?1)          AS revenue
    ", [tick]))
    unwrap(db.begin(conn))
    unwrap(db.execute(conn, "DELETE FROM movements WHERE tick < ?", [tick]))
    unwrap(db.execute(conn, "DELETE FROM charges WHERE tick < ?", [tick]))
    unwrap(db.commit(conn))
    dropped
}

// ── Self-checks ─────────────────────────────────────────────────────
test "the ledger records, settles atomically, and aggregates" {
    let conn = open_ledger(":memory:")
    let v = { name: "MV Test", callsign: "TEST-100", draft: 6.0, hold: [], eta_tick: 0 }
    record_movement(conn, 1, v, "arrived", -1)
    record_movement(conn, 2, v, "berthed", 0)
    record_movement(conn, 9, v, "departed", 0)

    let n = settle_charges(conn, 9, v.callsign, [
        { kind: "container", units: 40.0, amount: 100.0 },
        { kind: "hazmat", units: 12.5, amount: 330.0 }
    ])
    testing.assert_eq(n, 2)

    let totals = ledger_totals(conn)
    testing.assert_eq(map_get(totals, "movements"), 3)
    testing.assert_eq(map_get(totals, "departures"), 1)
    testing.assert_eq(map_get(totals, "charge_rows"), 2)
    testing.assert_eq(map_get(totals, "revenue"), 430.0)

    let by_kind = revenue_by_kind(conn)
    testing.assert_eq(map_get(head(by_kind), "kind"), "hazmat")

    let track = movements_for(conn, "TEST-100")
    testing.assert_eq(len(track), 3)
    testing.assert_eq(map_get(track[2], "event"), "departed")

    // pruning drops the old window but reports exactly what it dropped,
    // so cumulative accounting survives the archive
    let dropped = prune_before(conn, 5)
    testing.assert_eq(map_get(dropped, "movements"), 2)
    testing.assert_eq(map_get(dropped, "departures"), 0)
    let left = ledger_totals(conn)
    testing.assert_eq(map_get(left, "movements"), 1)
    testing.assert_eq(map_get(left, "departures"), 1)
    testing.assert_eq(map_get(left, "charge_rows"), 2)

    close_ledger(conn)
}

//! The `db` module's interrupts, time limits, cursors, row-keeping
//! reads, and open options (heddle-sql's findings): `db.interrupt` from
//! another task, `timeout_ms`, `db.cursor`/`db.next`/`db.columns`/
//! `db.close_cursor`, `db.query_rows`, and `db.open(path, #{ readonly,
//! create })`. Each program asserts in olang and answers "ok".

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

fn assert_ok(src: &str) {
    assert_eq!(eval(src), Value::String("ok".to_string().into()));
}

/// A fresh file path under the temp dir, removed first.
fn temp_db(tag: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "olang_db_stream_{}_{}.sqlite",
        std::process::id(),
        tag
    ));
    for suffix in ["", "-journal", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{}", path.to_string_lossy(), suffix));
    }
    path.to_string_lossy().replace('\\', "/")
}

/// A statement that never ends on its own: counting an unbounded
/// recursive CTE.
const ENDLESS: &str =
    "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n";

/// Ten million rows, streamed by SQLite one at a time.
const TEN_MILLION: &str = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n LIMIT 10000000) SELECT i, 'row ' || i AS label FROM n";

// ── interrupt ───────────────────────────────────────────────────────

#[test]
fn interrupt_from_another_task_stops_a_running_query_promptly() {
    assert_ok(&format!(
        r#"
let c = unwrap(db.open(":memory:"))
let t = spawn {{ time.sleep(150); db.interrupt(c) }}
let t0 = time.monotonic_ms()
let r = db.query(c, "{ENDLESS}")
let took = time.monotonic_ms() - t0
assert_eq(r, Err("db.query: interrupted"))
assert(took < 1150, "the interrupted query answered after " + to_string(took) + " ms")
assert_eq(task.join(t), Ok(()))
// the connection is usable afterwards, and a later statement is not
// interrupted by the interrupt that is spent
assert_eq(map_get(unwrap(db.query_one(c, "SELECT 41 + 1 AS n")), "n"), 42)
unwrap(db.execute(c, "CREATE TABLE t (x)"))
assert_eq(unwrap(db.execute(c, "INSERT INTO t VALUES (1)")), 1)
unwrap(db.close(c))
"ok"
"#
    ));
}

#[test]
fn interrupt_stops_execute_and_query_rows_too() {
    assert_ok(&format!(
        r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE t (x)"))
let t1 = spawn {{ time.sleep(100); db.interrupt(c) }}
assert_eq(db.execute(c, "INSERT INTO t {INSERT}"), Err("db.execute: interrupted"))
task.join(t1)
assert_eq(map_get(unwrap(db.query_one(c, "SELECT count(*) AS n FROM t")), "n"), 0)
let t2 = spawn {{ time.sleep(100); db.interrupt(c) }}
assert_eq(db.query_rows(c, "{ENDLESS}"), Err("db.query_rows: interrupted"))
task.join(t2)
unwrap(db.close(c))
"ok"
"#,
        INSERT = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT i FROM n"
    ));
}

#[test]
fn an_interrupt_when_nothing_runs_stops_nothing_later() {
    assert_ok(
        r#"
let c = unwrap(db.open(":memory:"))
assert_eq(db.interrupt(c), Ok(()))
assert_eq(map_get(unwrap(db.query_one(c, "SELECT 1 AS n")), "n"), 1)
unwrap(db.close(c))
// a closed connection has nothing to interrupt
assert_eq(db.interrupt(c), Ok(()))
assert(is_err(db.interrupt(42)), "not a connection")
"ok"
"#,
    );
}

#[test]
fn interrupt_is_callable_from_a_rust_thread_while_the_statement_holds_the_connection() {
    use olang::stdlib::db::call_db_function;
    let s = |t: &str| Value::String(t.to_string().into());
    let conn = match call_db_function("open", vec![s(":memory:")]).unwrap() {
        Value::Ok(c) => *c,
        other => panic!("open: {other:?}"),
    };
    let c2 = conn.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let t0 = std::time::Instant::now();
        call_db_function("interrupt", vec![c2]).unwrap();
        // The interrupt takes no lock: it returns at once even though the
        // query holds the connection.
        t0.elapsed()
    });
    let t0 = std::time::Instant::now();
    let r = call_db_function("query", vec![conn.clone(), s(ENDLESS)]).unwrap();
    assert_eq!(r, Value::Err(Box::new(s("db.query: interrupted"))));
    assert!(t0.elapsed() < std::time::Duration::from_secs(1));
    assert!(stopper.join().unwrap() < std::time::Duration::from_millis(50));
    call_db_function("close", vec![conn]).unwrap();
}

// ── timeout_ms ──────────────────────────────────────────────────────

#[test]
fn timeout_ms_stops_a_statement_and_says_so() {
    assert_ok(&format!(
        r#"
let c = unwrap(db.open(":memory:"))
let t0 = time.monotonic_ms()
assert_eq(db.query(c, "{ENDLESS}", [], #{{ "timeout_ms": 100 }}), Err("db.query: timed out after 100 ms"))
let took = time.monotonic_ms() - t0
assert(took >= 90 && took < 1000, "timed out after " + to_string(took) + " ms")
// the options may stand where the params would
assert_eq(db.query_one(c, "{ENDLESS}", #{{ "timeout_ms": 50 }}), Err("db.query_one: timed out after 50 ms"))
assert_eq(db.query_rows(c, "{ENDLESS}", #{{ "timeout_ms": 50 }}), Err("db.query_rows: timed out after 50 ms"))
unwrap(db.execute(c, "CREATE TABLE t (x)"))
assert_eq(db.execute(c, "INSERT INTO t {INSERT}", [], #{{ "timeout_ms": 50 }}), Err("db.execute: timed out after 50 ms"))
// a statement inside its limit is untouched, and the limit is per call
assert_eq(map_get(unwrap(db.query_one(c, "SELECT ? + 1 AS n", [1], #{{ "timeout_ms": 5000 }})), "n"), 2)
assert_eq(len(unwrap(db.query(c, "SELECT 1"))), 1)
// bad options are named
assert_eq(db.query(c, "SELECT 1", [], #{{ "timeout_ms": 0 }}), Err("db.query: timeout_ms must be a positive number of milliseconds, got 0"))
assert_eq(db.query(c, "SELECT 1", [], #{{ "timeout": 5 }}), Err("db.query: unknown option \"timeout\" (it takes timeout_ms)"))
unwrap(db.close(c))
"ok"
"#,
        INSERT = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT i FROM n"
    ));
}

// ── cursors ─────────────────────────────────────────────────────────

#[test]
fn a_cursor_streams_the_first_rows_of_ten_million_without_the_rest() {
    assert_ok(&format!(
        r#"
let c = unwrap(db.open(":memory:"))
let before = map_get(runtime.memory(), "heap")
let t0 = time.monotonic_ms()
let cur = unwrap(db.cursor(c, "{TEN_MILLION}"))
let first = unwrap(db.next(cur, 10))
let took = time.monotonic_ms() - t0
let grew = map_get(runtime.memory(), "heap") - before
assert_eq(len(first), 10)
assert_eq(first[0], [1, "row 1"])
assert_eq(first[9], [10, "row 10"])
assert(took < 500, "the first 10 rows took " + to_string(took) + " ms")
// ten million rows as values would be hundreds of megabytes
assert(grew < 16000000, "reading 10 rows grew the heap by " + to_string(grew) + " bytes")
assert_eq(unwrap(db.next(cur, 3)), [[11, "row 11"], [12, "row 12"], [13, "row 13"]])
unwrap(db.close_cursor(cur))
unwrap(db.close(c))
"ok"
"#
    ));
}

#[test]
fn a_cursor_reads_to_the_end_then_answers_empty() {
    assert_ok(
        r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, data BLOB)"))
for i in 1..6 { unwrap(db.execute(c, "INSERT INTO t (name, data) VALUES (?, ?)", ["n" + to_string(i), bytes.from_list([i, 0, 255])])) }
let cur = unwrap(db.cursor(c, "SELECT id, name, data, id AS id FROM t WHERE id >= ? ORDER BY id", [2]))
let cols = unwrap(db.columns(cur))
assert_eq(map(cols, (k) => map_get(k, "name")), ["id", "name", "data", "id"])
assert_eq(map(cols, (k) => map_get(k, "decltype")), ["INTEGER", "TEXT", "BLOB", "INTEGER"])
assert_eq(map(cols, (k) => map_get(k, "index")), [0, 1, 2, 3])
let a = unwrap(db.next(cur, 3))
assert_eq(len(a), 3)
assert_eq(a[0], [2, "n2", bytes.from_list([2, 0, 255]), 2])
assert_eq(unwrap(db.next(cur, 3)), [[5, "n5", bytes.from_list([5, 0, 255]), 5]])
assert_eq(unwrap(db.next(cur, 3)), [])
assert_eq(unwrap(db.next(cur, 3)), [])
// the columns are known after the end, and after the close
unwrap(db.close_cursor(cur))
assert_eq(len(unwrap(db.columns(cur))), 4)
assert_eq(db.next(cur, 1), Err("db.next: the cursor is closed"))
// closing twice is no error
assert_eq(db.close_cursor(cur), Ok(()))
// a cursor over no rows has its columns and no rows
let none = unwrap(db.cursor(c, "SELECT name FROM t WHERE id > 100"))
assert_eq(map(unwrap(db.columns(none)), (k) => map_get(k, "name")), ["name"])
assert_eq(unwrap(db.next(none, 10)), [])
// misuse is named
assert_eq(db.next(none, 0), Err("db.next: n must be a positive integer, got 0"))
assert(is_err(db.next(c, 1)), "a connection is not a cursor")
assert(is_err(db.cursor(c, "SELECT nope FROM t")), "bad SQL")
assert_eq(db.cursor(c, "SELECT 1; SELECT 2"), Err("db.cursor: more than one statement: run them one at a time"))
unwrap(db.close(c))
"ok"
"#,
    );
}

#[test]
fn a_cursor_can_be_interrupted_and_timed_out() {
    assert_ok(&format!(
        r#"
let c = unwrap(db.open(":memory:"))
let cur = unwrap(db.cursor(c, "{ENDLESS}"))
let t = spawn {{ time.sleep(100); db.interrupt(cur) }}
let t0 = time.monotonic_ms()
assert_eq(db.next(cur, 1), Err("db.next: interrupted"))
assert(time.monotonic_ms() - t0 < 1000, "the interrupt was prompt")
task.join(t)
// a failed cursor is finished: later reads answer why
assert_eq(db.next(cur, 1), Err("db.next: the cursor failed: interrupted"))
// the connection goes on, and another cursor on it is not interrupted
let ok_cur = unwrap(db.cursor(c, "SELECT 1 UNION ALL SELECT 2"))
assert_eq(unwrap(db.next(ok_cur, 5)), [[1], [2]])
// timeout_ms bounds each read of a cursor
let slow = unwrap(db.cursor(c, "{ENDLESS}", [], #{{ "timeout_ms": 80 }}))
assert_eq(db.next(slow, 1), Err("db.next: timed out after 80 ms"))
unwrap(db.close(c))
"ok"
"#
    ));
}

#[test]
fn an_open_cursor_does_not_poison_the_connection_for_an_interrupt() {
    // sqlite3_interrupt would stay in force while any statement is
    // active; the progress handler stops only the one running.
    assert_ok(&format!(
        r#"
let c = unwrap(db.open(":memory:"))
let cur = unwrap(db.cursor(c, "SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3"))
assert_eq(unwrap(db.next(cur, 1)), [[1]])
let t = spawn {{ time.sleep(100); db.interrupt(c) }}
assert_eq(db.query(c, "{ENDLESS}"), Err("db.query: interrupted"))
task.join(t)
assert_eq(unwrap(db.next(cur, 5)), [[2], [3]])
assert_eq(len(unwrap(db.query(c, "SELECT 1"))), 1)
unwrap(db.close(c))
"ok"
"#
    ));
}

#[test]
fn closing_the_connection_finishes_its_cursors() {
    assert_ok(
        r#"
let c = unwrap(db.open(":memory:"))
let cur = unwrap(db.cursor(c, "SELECT 1 UNION ALL SELECT 2"))
assert_eq(unwrap(db.next(cur, 1)), [[1]])
assert_eq(db.close(c), Ok(()))
assert_eq(db.next(cur, 1), Err("db.next: connection is closed"))
assert_eq(db.close_cursor(cur), Ok(()))
assert_eq(db.cursor(c, "SELECT 1"), Err("db.cursor: connection is closed"))
"ok"
"#,
    );
}

#[test]
fn dropping_a_cursor_finalizes_its_statement() {
    // In rollback-journal mode an unfinished read holds a shared lock, so
    // another connection cannot commit a write. Once the cursor that read
    // is dropped, it can — at once, not after the 5 s busy timeout.
    let path = temp_db("drop");
    assert_ok(&format!(
        r#"
let w = unwrap(db.open("{path}"))
unwrap(db.execute(w, "CREATE TABLE t (x)"))
unwrap(db.execute(w, "INSERT INTO t VALUES (1), (2), (3)"))
let r = unwrap(db.open("{path}"))
fn peek(conn) = {{
    let cur = unwrap(db.cursor(conn, "SELECT x FROM t"))
    unwrap(db.next(cur, 1))
}}
assert_eq(peek(r), [[1]])
let t0 = time.monotonic_ms()
assert_eq(db.execute(w, "INSERT INTO t VALUES (4)"), Ok(1))
let took = time.monotonic_ms() - t0
assert(took < 2000, "the write waited " + to_string(took) + " ms for a dropped cursor's lock")
unwrap(db.close(r))
unwrap(db.close(w))
"ok"
"#
    ));
}

// ── query_rows ──────────────────────────────────────────────────────

#[test]
fn query_rows_keeps_order_repeats_types_and_blobs() {
    assert_ok(
        r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE p (name TEXT, age INTEGER, score REAL, photo BLOB, note)"))
unwrap(db.execute(c, "INSERT INTO p VALUES (?, ?, ?, ?, ?)", ["ann", 31, 2.5, bytes.from_list([0, 255, 128, 10]), ()]))
let r = unwrap(db.query_rows(c, "SELECT name, age, score, photo, note, 1 AS name, age * 2 FROM p"))
let cols = map_get(r, "columns")
assert_eq(map(cols, (k) => map_get(k, "name")), ["name", "age", "score", "photo", "note", "name", "age * 2"])
assert_eq(map(cols, (k) => map_get(k, "decltype")), ["TEXT", "INTEGER", "REAL", "BLOB", (), (), ()])
assert_eq(map(cols, (k) => map_get(k, "index")), [0, 1, 2, 3, 4, 5, 6])
let row = map_get(r, "rows")[0]
assert_eq(row, ["ann", 31, 2.5, bytes.from_list([0, 255, 128, 10]), (), 1, 62])
assert_eq(typeof(row[3]), "Bytes")
// a BLOB literal is Bytes, not a lossy string
let b = unwrap(db.query_rows(c, "SELECT x'00ff' AS b, '' AS e, x'' AS z"))
assert_eq(map_get(b, "rows"), [[bytes.from_list([0, 255]), "", bytes.from_list([])]])
// zero rows still name their columns
let none = unwrap(db.query_rows(c, "SELECT age, name FROM p WHERE age > ?", [100]))
assert_eq(map(map_get(none, "columns"), (k) => map_get(k, "name")), ["age", "name"])
assert_eq(map_get(none, "rows"), [])
// the rows keep SQL's order
unwrap(db.execute(c, "INSERT INTO p (name, age) VALUES ('bob', 7), ('cy', 19)"))
let ordered = unwrap(db.query_rows(c, "SELECT age FROM p ORDER BY age"))
assert_eq(map_get(ordered, "rows"), [[7], [19], [31]])
// db.query is unchanged: maps by name, a BLOB as a string
assert_eq(typeof(map_get(unwrap(db.query_one(c, "SELECT x'6869' AS b")), "b")), "String")
unwrap(db.close(c))
"ok"
"#,
    );
}

#[test]
fn bytes_bind_as_a_blob_and_round_trip() {
    assert_ok(
        r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE f (data BLOB)"))
let all = bytes.from_list(map(0..256, (i) => i))
unwrap(db.execute(c, "INSERT INTO f VALUES (?)", [all]))
unwrap(db.execute(c, "INSERT INTO f VALUES (?)", [bytes.from_list([])]))
let r = unwrap(db.query_rows(c, "SELECT data, typeof(data) AS t, length(data) AS n FROM f WHERE data = ?", [all]))
assert_eq(map_get(r, "rows"), [[all, "blob", 256]])
let empty = unwrap(db.query_rows(c, "SELECT typeof(data), length(data) FROM f WHERE length(data) = 0"))
assert_eq(map_get(empty, "rows"), [["blob", 0]])
unwrap(db.close(c))
"ok"
"#,
    );
}

// ── open options ────────────────────────────────────────────────────

#[test]
fn open_readonly_refuses_writes() {
    let path = temp_db("ro");
    assert_ok(&format!(
        r#"
let w = unwrap(db.open("{path}"))
unwrap(db.execute(w, "CREATE TABLE t (x)"))
unwrap(db.execute(w, "INSERT INTO t VALUES (1)"))
unwrap(db.close(w))
let r = unwrap(db.open("{path}", #{{ "readonly": true }}))
assert_eq(db.execute(r, "INSERT INTO t VALUES (2)"), Err("db.execute: attempt to write a readonly database"))
assert_eq(map_get(unwrap(db.query_one(r, "SELECT count(*) AS n FROM t")), "n"), 1)
unwrap(db.close(r))
"ok"
"#
    ));
}

#[test]
fn open_create_false_refuses_a_missing_file_and_opens_an_existing_one() {
    let path = temp_db("nocreate");
    assert_ok(&format!(
        r#"
assert_eq(db.open("{path}", #{{ "create": false }}), Err("db.open: no database at {path} (create: false)"))
assert_eq(db.open("{path}", #{{ "readonly": true }}), Err("db.open: no database at {path} (a read-only open never makes one)"))
assert(!fs.exists("{path}"), "a refused open makes no file")
let w = unwrap(db.open("{path}"))
unwrap(db.execute(w, "CREATE TABLE t (x)"))
unwrap(db.close(w))
let again = unwrap(db.open("{path}", #{{ "create": false }}))
assert_eq(unwrap(db.execute(again, "INSERT INTO t VALUES (1)")), 1)
unwrap(db.close(again))
assert_eq(db.open("{path}", #{{ "readonly": true, "create": true }}), Err("db.open: readonly and create cannot both be true: a read-only open never makes a file"))
assert_eq(db.open("{path}", #{{ "mode": "ro" }}), Err("db.open: unknown option \"mode\" (it takes readonly and create)"))
assert_eq(db.open("{path}", #{{ "readonly": 1 }}), Err("db.open: readonly must be true or false, got 1"))
// the defaults are today's: read-write, created when missing
let m = unwrap(db.open(":memory:", #{{}}))
unwrap(db.close(m))
"ok"
"#
    ));
}

#[test]
fn open_accepts_sqlite_uri_filenames() {
    let path = temp_db("uri");
    assert_ok(&format!(
        r#"
let w = unwrap(db.open("file:{path}?mode=rwc"))
unwrap(db.execute(w, "CREATE TABLE t (x)"))
unwrap(db.close(w))
let r = unwrap(db.open("file:{path}?mode=ro"))
assert_eq(db.execute(r, "INSERT INTO t VALUES (1)"), Err("db.execute: attempt to write a readonly database"))
unwrap(db.close(r))
assert(is_err(db.open("file:{path}-missing?mode=rw")), "mode=rw does not create")
"ok"
"#
    ));
}

// ── concurrency ─────────────────────────────────────────────────────

#[test]
fn two_tasks_with_their_own_connections_to_one_file_run_side_by_side() {
    let path = temp_db("side");
    assert_ok(&format!(
        r#"
let setup = unwrap(db.open("{path}"))
unwrap(db.execute(setup, "CREATE TABLE t (x)"))
unwrap(db.execute(setup, "INSERT INTO t VALUES (1), (2), (3)"))
unwrap(db.close(setup))
let slow = unwrap(db.open("{path}"))
let quick = unwrap(db.open("{path}"))
// a long statement on one connection…
let t = spawn {{ db.query(slow, "{ENDLESS}") }}
time.sleep(100)
// …does not hold up another connection's statements
let t0 = time.monotonic_ms()
for i in 0..20 {{ assert_eq(map_get(unwrap(db.query_one(quick, "SELECT sum(x) AS s FROM t")), "s"), 6) }}
let took = time.monotonic_ms() - t0
assert(took < 500, "20 quick reads took " + to_string(took) + " ms beside a running statement")
unwrap(db.interrupt(slow))
assert_eq(task.join(t), Err("db.query: interrupted"))
// two tasks, two connections, both reading at once
let a = spawn {{ let c = unwrap(db.open("{path}")); let r = unwrap(db.query_rows(c, "SELECT x FROM t ORDER BY x")); db.close(c); map_get(r, "rows") }}
let b = spawn {{ let c = unwrap(db.open("{path}")); let r = unwrap(db.query_rows(c, "SELECT x FROM t ORDER BY x DESC")); db.close(c); map_get(r, "rows") }}
assert_eq(task.join(a), [[1], [2], [3]])
assert_eq(task.join(b), [[3], [2], [1]])
unwrap(db.close(slow))
unwrap(db.close(quick))
"ok"
"#
    ));
}

#[test]
fn a_cursor_stays_open_while_another_connection_runs_a_statement() {
    let path = temp_db("cursor_beside");
    assert_ok(&format!(
        r#"
let setup = unwrap(db.open("{path}"))
unwrap(db.execute(setup, "CREATE TABLE t (x)"))
unwrap(db.execute(setup, "INSERT INTO t {INSERT}"))
unwrap(db.close(setup))
let a = unwrap(db.open("{path}"))
let b = unwrap(db.open("{path}"))
let cur = unwrap(db.cursor(a, "SELECT x FROM t ORDER BY x"))
assert_eq(unwrap(db.next(cur, 2)), [[1], [2]])
let t = spawn {{ map_get(unwrap(db.query_one(b, "SELECT count(*) AS n, sum(x) AS s FROM t")), "s") }}
assert_eq(unwrap(db.next(cur, 2)), [[3], [4]])
assert_eq(task.join(t), 5050)
let rest = unwrap(db.next(cur, 1000))
assert_eq(len(rest), 96)
assert_eq(rest[95], [100])
assert_eq(unwrap(db.next(cur, 1)), [])
// a cursor is read from another task as well
let cur2 = unwrap(db.cursor(b, "SELECT x FROM t WHERE x > ? ORDER BY x", [97]))
let t2 = spawn {{ unwrap(db.next(cur2, 10)) }}
assert_eq(task.join(t2), [[98], [99], [100]])
unwrap(db.close(a))
unwrap(db.close(b))
"ok"
"#,
        INSERT = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n LIMIT 100) SELECT i FROM n"
    ));
}

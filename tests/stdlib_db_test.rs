//! Coverage for the `db` (SQLite) module: CRUD, parameterized queries,
//! result mapping, error handling, and file persistence across connections.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

#[test]
fn create_insert_and_query_roundtrip() {
    let src = r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE t (id INTEGER, name TEXT)"))
unwrap(db.execute(c, "INSERT INTO t VALUES (?, ?)", [1, "ann"]))
unwrap(db.execute(c, "INSERT INTO t VALUES (?, ?)", [2, "bob"]))
let rows = unwrap(db.query(c, "SELECT name FROM t ORDER BY id"))
unwrap(db.close(c))
len(rows)
"#;
    assert_eq!(eval(src), Value::Integer(2));
}

#[test]
fn rows_map_columns_to_values() {
    let src = r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE t (n INTEGER, f REAL, s TEXT)"))
unwrap(db.execute(c, "INSERT INTO t VALUES (?, ?, ?)", [42, 3.5, "hi"]))
let row = unwrap(db.query_one(c, "SELECT n, f, s FROM t"))
unwrap(db.close(c))
[map_get(row, "n"), map_get(row, "f"), map_get(row, "s")]
"#;
    match eval(src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::Integer(42));
            assert_eq!(items[1], Value::Float(3.5));
            assert_eq!(items[2], Value::String("hi".to_string().into()));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn execute_reports_rows_affected() {
    let src = r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE t (n INTEGER)"))
unwrap(db.execute(c, "INSERT INTO t VALUES (1), (2), (3)"))
let affected = unwrap(db.execute(c, "UPDATE t SET n = n + 10 WHERE n >= 2"))
unwrap(db.close(c))
affected
"#;
    assert_eq!(eval(src), Value::Integer(2));
}

#[test]
fn query_one_returns_unit_when_no_row() {
    let src = r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE t (n INTEGER)"))
let row = unwrap(db.query_one(c, "SELECT n FROM t WHERE n = 999"))
unwrap(db.close(c))
row
"#;
    assert_eq!(eval(src), Value::Unit);
}

#[test]
fn parameters_are_bound_not_interpolated() {
    // A value containing SQL metacharacters must be treated as data
    let src = r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE t (name TEXT)"))
unwrap(db.execute(c, "INSERT INTO t VALUES (?)", ["Robert'); DROP TABLE t;--"]))
let rows = unwrap(db.query(c, "SELECT name FROM t WHERE name = ?", ["Robert'); DROP TABLE t;--"]))
let found = len(rows)
// the table still exists and the exact string round-tripped
let total = len(unwrap(db.query(c, "SELECT name FROM t")))
unwrap(db.close(c))
[found, total]
"#;
    match eval(src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::Integer(1));
            assert_eq!(items[1], Value::Integer(1));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn null_maps_to_unit() {
    let src = r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE t (a INTEGER, b TEXT)"))
unwrap(db.execute(c, "INSERT INTO t (a) VALUES (1)"))
let row = unwrap(db.query_one(c, "SELECT a, b FROM t"))
unwrap(db.close(c))
typeof(map_get(row, "b"))
"#;
    assert_eq!(eval(src), Value::String("Unit".to_string().into()));
}

#[test]
fn bad_sql_is_a_recoverable_err() {
    let src = r#"
let c = unwrap(db.open(":memory:"))
let result = match db.query(c, "SELECT * FROM does_not_exist") {
    Ok(_) => "ok",
    Err(e) => "err"
}
unwrap(db.close(c))
result
"#;
    assert_eq!(eval(src), Value::String("err".to_string().into()));
}

#[test]
fn operating_on_a_closed_connection_errs() {
    let src = r#"
let c = unwrap(db.open(":memory:"))
unwrap(db.close(c))
match db.query(c, "SELECT 1") {
    Ok(_) => "ok",
    Err(e) => "closed"
}
"#;
    assert_eq!(eval(src), Value::String("closed".to_string().into()));
}

#[test]
fn data_persists_to_a_file_across_connections() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("olang_db_test_{}.sqlite", std::process::id()));
    let path_str = path.to_string_lossy().replace('\\', "/");
    let _ = std::fs::remove_file(&path);

    // Write with one connection, close it
    let write = format!(
        r#"
let c = unwrap(db.open("{p}"))
unwrap(db.execute(c, "CREATE TABLE t (v TEXT)"))
unwrap(db.execute(c, "INSERT INTO t VALUES (?)", ["persisted"]))
unwrap(db.close(c))
"#,
        p = path_str
    );
    eval(&write);

    // Reopen a fresh connection and read it back
    let read = format!(
        r#"
let c = unwrap(db.open("{p}"))
let row = unwrap(db.query_one(c, "SELECT v FROM t"))
unwrap(db.close(c))
map_get(row, "v")
"#,
        p = path_str
    );
    let result = eval(&read);
    let _ = std::fs::remove_file(&path);

    assert_eq!(result, Value::String("persisted".to_string().into()));
}

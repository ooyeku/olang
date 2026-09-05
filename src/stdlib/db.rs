//! SQLite database module (`db`), backed by the bundled `rusqlite`.
//!
//! A `rusqlite::Connection` is a stateful, non-value handle, so open
//! connections live in a global registry keyed by an integer id (the same
//! pattern the RNG and promise registries use). `db.open` returns a
//! `Connection` struct carrying that id; the other functions take it back.
//!
//! Convention: every operation is fallible (I/O, SQL errors) and returns an
//! olang `Result`. Query parameters are bound to `?` placeholders, so values
//! never need to be spliced into SQL by hand — the safe path is the easy one.
//!
//! Value mapping (SQLite <-> olang):
//! - NULL    <-> Unit
//! - INTEGER <-> Integer
//! - REAL    <-> Float
//! - TEXT    <-> String
//! - BLOB    <-> String (lossy: bytes are surfaced as a UTF-8 string)

use crate::ast::Value;
use rusqlite::types::{ToSqlOutput, Value as SqlValue, ValueRef};
use rusqlite::{Connection, ToSql};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Open connections, keyed by id. `next_id` hands out fresh ids.
struct Registry {
    connections: HashMap<i64, Connection>,
    next_id: i64,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            connections: HashMap::new(),
            next_id: 1,
        })
    })
}

/// Creates the db module.
pub fn create_db_module() -> Value {
    let mut module = HashMap::new();
    module.insert("open".to_string(), create_builtin_function("open", 1));
    // execute / query accept an optional params list, so register the
    // 2-arg arity; extra args are tolerated by the implementation.
    module.insert("execute".to_string(), create_builtin_function("execute", 2));
    module.insert("query".to_string(), create_builtin_function("query", 2));
    module.insert(
        "query_one".to_string(),
        create_builtin_function("query_one", 2),
    );
    module.insert("close".to_string(), create_builtin_function("close", 1));
    module.insert(
        "transaction".to_string(),
        create_builtin_function("transaction", 2),
    );
    module.insert("migrate".to_string(), create_builtin_function("migrate", 2));
    module.insert("begin".to_string(), create_builtin_function("begin", 1));
    module.insert("commit".to_string(), create_builtin_function("commit", 1));
    module.insert(
        "rollback".to_string(),
        create_builtin_function("rollback", 1),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("db.{}", name),
        arity,
    })
}

/// Dispatcher for `db` functions.
pub fn call_db_function(name: &str, args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "open" => db_open(args),
        "execute" => db_execute(args),
        "query" => db_query(args, false),
        "query_one" => db_query(args, true),
        "close" => db_close(args),
        // Transactions: thin wrappers over execute, so a handle flows the
        // same way and errors surface identically.
        "begin" => db_transaction_statement(args, "BEGIN"),
        "commit" => db_transaction_statement(args, "COMMIT"),
        "rollback" => db_transaction_statement(args, "ROLLBACK"),
        "migrate" => db_migrate(args),
        // Reached only when no interpreter intercepted the call.
        "transaction" => {
            Err("db.transaction calls your function, which only the interpreter can do".into())
        }
        _ => Err(format!("Unknown db function: {}", name).into()),
    }
}

// ── helpers ─────────────────────────────────────────────────────────

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

fn err(msg: impl Into<String>) -> Value {
    Value::Err(Box::new(Value::String(std::sync::Arc::new(msg.into()))))
}

fn string(s: impl Into<String>) -> Value {
    Value::String(std::sync::Arc::new(s.into()))
}

/// A connection handle is a struct { id: Integer }; pull the id back out.
fn connection_id(value: &Value) -> Result<i64, Value> {
    match value {
        Value::Struct { type_name, fields } if type_name == "Connection" => {
            match fields.get("id") {
                Some(Value::Integer(id)) => Ok(*id),
                _ => Err(err("db: malformed connection handle")),
            }
        }
        _ => Err(err("db: expected a connection (from db.open)")),
    }
}

/// Convert an olang value into a bound SQL parameter.
struct Param(Value);

impl ToSql for Param {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let v = match &self.0 {
            Value::Integer(n) => SqlValue::Integer(*n),
            Value::Float(f) => SqlValue::Real(*f),
            Value::String(s) => SqlValue::Text(s.to_string()),
            Value::Boolean(b) => SqlValue::Integer(*b as i64),
            Value::Unit => SqlValue::Null,
            other => {
                return Err(rusqlite::Error::ToSqlConversionFailure(
                    format!("db: cannot bind {} as a SQL parameter", other.type_name()).into(),
                ));
            }
        };
        Ok(ToSqlOutput::Owned(v))
    }
}

/// The params argument (optional third arg): a list of values, or nothing.
fn extract_params(args: &[Value], index: usize) -> Result<Vec<Param>, Value> {
    match args.get(index) {
        None => Ok(Vec::new()),
        Some(Value::List(items)) => Ok(items.iter().cloned().map(Param).collect()),
        Some(other) => Err(err(format!(
            "db: params must be a list, got {}",
            other.type_name()
        ))),
    }
}

/// Convert a SQLite column value into an olang value.
fn sql_to_value(cell: ValueRef<'_>) -> Value {
    match cell {
        ValueRef::Null => Value::Unit,
        ValueRef::Integer(n) => Value::Integer(n),
        ValueRef::Real(f) => Value::Float(f),
        ValueRef::Text(bytes) => string(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => string(String::from_utf8_lossy(bytes).into_owned()),
    }
}

// ── operations ──────────────────────────────────────────────────────

/// db.open(path) -> Result<Connection>. Use ":memory:" for an in-memory
/// database.
/// Run BEGIN/COMMIT/ROLLBACK on a connection handle.
/// Usage: db.begin(conn) / db.commit(conn) / db.rollback(conn) -> Result<_, Error>
/// db.migrate(conn, steps) -> Result<Int>: bring the database to the head
/// of `steps` — a list of versions, each a list of statements (or one
/// statement). A `schema_version` table records the applied version;
/// version N runs only when the recorded version is below N, inside its
/// own transaction, so a failed step leaves the database at the version
/// before it. Returns the version now recorded.
fn db_migrate(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let id = match connection_id(args.first().unwrap_or(&Value::Unit)) {
        Ok(id) => id,
        Err(e) => return Ok(e),
    };
    let steps: Vec<Vec<String>> = match args.get(1) {
        Some(Value::List(versions)) => {
            let mut out = Vec::with_capacity(versions.len());
            for (i, version) in versions.iter().enumerate() {
                match version {
                    Value::String(s) => out.push(vec![s.to_string()]),
                    Value::List(stmts) => {
                        let mut v = Vec::with_capacity(stmts.len());
                        for stmt in stmts.iter() {
                            match stmt {
                                Value::String(s) => v.push(s.to_string()),
                                other => {
                                    return Ok(err(format!(
                                        "db.migrate: version {} holds a {}, expected SQL strings",
                                        i + 1,
                                        other.type_name()
                                    )));
                                }
                            }
                        }
                        out.push(v);
                    }
                    other => {
                        return Ok(err(format!(
                            "db.migrate: version {} is a {}, expected a list of SQL strings",
                            i + 1,
                            other.type_name()
                        )));
                    }
                }
            }
            out
        }
        _ => {
            return Ok(err(
                "db.migrate: steps must be a list of versions, each a list of SQL strings"
                    .to_string(),
            ));
        }
    };

    let reg = registry().lock().unwrap();
    let conn = match reg.connections.get(&id) {
        Some(c) => c,
        None => return Ok(err("db.migrate: connection is closed".to_string())),
    };
    let fail = |what: &str, e: rusqlite::Error| err(format!("db.migrate: {}: {}", what, e));
    if let Err(e) = conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)",
        [],
    ) {
        return Ok(fail("schema_version", e));
    }
    let recorded: Option<i64> =
        match conn.query_row("SELECT version FROM schema_version", [], |row| row.get(0)) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Ok(fail("schema_version", e)),
        };
    let mut version = match recorded {
        Some(v) => v,
        None => {
            if let Err(e) = conn.execute("INSERT INTO schema_version (version) VALUES (0)", []) {
                return Ok(fail("schema_version", e));
            }
            0
        }
    };
    while (version as usize) < steps.len() {
        let next = version + 1;
        if let Err(e) = conn.execute_batch("BEGIN") {
            return Ok(fail("begin", e));
        }
        for stmt in &steps[version as usize] {
            if let Err(e) = conn.execute_batch(stmt) {
                let _ = conn.execute_batch("ROLLBACK");
                // The failing statement is the bug report — surface it.
                return Ok(err(format!(
                    "db.migrate: migration v{} failed: {}",
                    next, e
                )));
            }
        }
        if let Err(e) = conn.execute("UPDATE schema_version SET version = ?1", [next]) {
            let _ = conn.execute_batch("ROLLBACK");
            return Ok(fail("schema_version", e));
        }
        if let Err(e) = conn.execute_batch("COMMIT") {
            return Ok(fail("commit", e));
        }
        version = next;
    }
    Ok(ok(Value::Integer(version)))
}

fn db_transaction_statement(
    args: Vec<Value>,
    sql: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(std::sync::Arc::new(
            format!(
                "{} expects 1 argument (connection), got {}",
                sql.to_lowercase(),
                args.len()
            ),
        )))));
    }
    let conn = args.into_iter().next().unwrap();
    db_execute(vec![
        conn,
        Value::String(std::sync::Arc::new(sql.to_string())),
    ])
}

fn db_open(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let path = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        _ => return Ok(err("db.open: path must be a string")),
    };

    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(e) => return Ok(err(format!("db.open: {}", e))),
    };

    let mut reg = registry().lock().unwrap();
    let id = reg.next_id;
    reg.next_id += 1;
    reg.connections.insert(id, conn);

    let mut fields = HashMap::new();
    fields.insert("id".to_string(), Value::Integer(id));
    fields.insert("path".to_string(), string(path));
    Ok(ok(Value::Struct {
        type_name: "Connection".to_string(),
        fields: std::sync::Arc::new(fields),
    }))
}

/// db.execute(conn, sql[, params]) -> Result<Int> (rows affected). For
/// statements that change data (CREATE/INSERT/UPDATE/DELETE).
fn db_execute(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let id = match connection_id(args.first().unwrap_or(&Value::Unit)) {
        Ok(id) => id,
        Err(e) => return Ok(e),
    };
    let sql = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        _ => return Ok(err("db.execute: sql must be a string")),
    };
    let params = match extract_params(&args, 2) {
        Ok(p) => p,
        Err(e) => return Ok(e),
    };

    let reg = registry().lock().unwrap();
    let conn = match reg.connections.get(&id) {
        Some(c) => c,
        None => return Ok(err("db.execute: connection is closed")),
    };

    let bound: Vec<&dyn ToSql> = params.iter().map(|p| p as &dyn ToSql).collect();
    match conn.execute(&sql, bound.as_slice()) {
        Ok(affected) => Ok(ok(Value::Integer(affected as i64))),
        Err(e) => Ok(err(format!("db.execute: {}", e))),
    }
}

/// db.query(conn, sql[, params]) -> Result<List<Map>>. Each row is a map
/// from column name to value. query_one returns Ok(row) or Ok(unit) when
/// there is no row.
fn db_query(args: Vec<Value>, one: bool) -> Result<Value, Box<dyn std::error::Error>> {
    let func = if one { "query_one" } else { "query" };
    let id = match connection_id(args.first().unwrap_or(&Value::Unit)) {
        Ok(id) => id,
        Err(e) => return Ok(e),
    };
    let sql = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        _ => return Ok(err(format!("db.{}: sql must be a string", func))),
    };
    let params = match extract_params(&args, 2) {
        Ok(p) => p,
        Err(e) => return Ok(e),
    };

    let reg = registry().lock().unwrap();
    let conn = match reg.connections.get(&id) {
        Some(c) => c,
        None => return Ok(err(format!("db.{}: connection is closed", func))),
    };

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => return Ok(err(format!("db.{}: {}", func, e))),
    };

    let column_names: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let bound: Vec<&dyn ToSql> = params.iter().map(|p| p as &dyn ToSql).collect();

    let mut rows = match stmt.query(bound.as_slice()) {
        Ok(r) => r,
        Err(e) => return Ok(err(format!("db.{}: {}", func, e))),
    };

    let mut out = Vec::new();
    loop {
        match rows.next() {
            Ok(Some(row)) => {
                let mut map = HashMap::new();
                for (i, name) in column_names.iter().enumerate() {
                    let cell = row.get_ref(i).map(sql_to_value).unwrap_or(Value::Unit);
                    map.insert(name.clone(), cell);
                }
                let row_value = Value::Map(std::sync::Arc::new(map));
                if one {
                    return Ok(ok(row_value));
                }
                out.push(row_value);
            }
            Ok(None) => break,
            Err(e) => return Ok(err(format!("db.{}: {}", func, e))),
        }
    }

    if one {
        // No row matched
        Ok(ok(Value::Unit))
    } else {
        Ok(ok(Value::List(out.into())))
    }
}

/// db.close(conn) -> Result<Unit>. Drops the connection from the registry.
fn db_close(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let id = match connection_id(args.first().unwrap_or(&Value::Unit)) {
        Ok(id) => id,
        Err(e) => return Ok(e),
    };
    let mut reg = registry().lock().unwrap();
    match reg.connections.remove(&id) {
        Some(conn) => {
            // Explicit close surfaces any final error
            match conn.close() {
                Ok(()) => Ok(ok(Value::Unit)),
                Err((_, e)) => Ok(err(format!("db.close: {}", e))),
            }
        }
        None => Ok(err("db.close: connection is already closed")),
    }
}

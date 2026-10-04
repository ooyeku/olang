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
//! - INTEGER <-> Integer (a Bool binds as 0/1)
//! - REAL    <-> Float
//! - TEXT    <-> String
//! - BLOB    <-> Bytes from `db.query_rows` and `db.next`; `db.query` and
//!   `db.query_one` keep their old answer, a String (lossy: invalid UTF-8
//!   replaced), so a program that read a BLOB as text still does. Bytes
//!   bind as a BLOB everywhere.
//!
//! # Locking, interrupts, and cursors
//!
//! The registry lock is held only to find a connection. Each connection
//! has its own lock ([`ConnEntry::state`]), held for the length of one
//! statement: two connections run side by side, and two threads sharing
//! one connection take turns — which is what SQLite's multi-thread mode
//! (rusqlite opens with `SQLITE_OPEN_NO_MUTEX`) requires.
//!
//! An interrupt must not need that lock (the statement it stops holds
//! it), so it goes through [`Control`], which lives beside the lock: an
//! atomic flag and a deadline, read by a progress handler SQLite calls
//! every [`PROGRESS_OPS`] virtual-machine instructions of whatever
//! statement the connection is running. The handler answering `true`
//! makes that statement fail with `SQLITE_INTERRUPT`, which is answered
//! as `Err("db.<fn>: interrupted")`, or `Err("db.<fn>: timed out after N
//! ms")` when the deadline tripped it. Each statement clears the flag as
//! it starts, so an interrupt stops what is running when it is called and
//! nothing after it. `sqlite3_interrupt` is not used: its flag stays set
//! while any statement on the connection is active — an open cursor is
//! one — so one interrupt would fail every later statement until every
//! cursor closed.
//!
//! A cursor is a prepared statement kept between calls. rusqlite's
//! `Statement` borrows its `Connection`, and the connection lives behind a
//! lock in a shared registry, so a cursor holds a raw `sqlite3_stmt`
//! instead ([`RawStmt`]). It is sound because the raw statement is owned
//! by the connection's locked state ([`ConnState::cursors`]), never by the
//! olang value: it is stepped, read, and finalized only while that lock is
//! held (so never concurrently with another use of the connection), and it
//! is finalized before the connection closes (`db.close` finalizes every
//! cursor first; [`ConnState`]'s drop does the same). The olang `Cursor`
//! value holds only the connection entry and the cursor's id; dropping the
//! last copy finalizes the statement — at once when the connection is
//! free, else at the connection's next use (a drop never waits on a
//! running statement).

use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use rusqlite::ffi;
use rusqlite::types::{ToSqlOutput, Value as SqlValue, ValueRef};
use rusqlite::{Connection, OpenFlags, ToSql};
use std::any::Any;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::Instant;

/// How often (in SQLite virtual-machine instructions) the progress
/// handler looks for an interrupt or a passed deadline. A thousand
/// instructions are microseconds of work, and the check is two atomic
/// loads (a clock read only when a deadline is set).
const PROGRESS_OPS: c_int = 1000;

/// What stops a running statement from outside its lock.
#[derive(Default)]
struct Control {
    /// `db.interrupt` was called since the running statement started.
    cancel: AtomicBool,
    /// The deadline tripped the last interruption (so its error says so).
    timed_out: AtomicBool,
    /// Nanoseconds since [`epoch`] (+1) when the running statement must
    /// stop; 0 for none.
    deadline: AtomicU64,
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn now_ns() -> u64 {
    epoch().elapsed().as_nanos() as u64 + 1
}

impl Control {
    /// The progress handler: `true` stops the running statement.
    fn should_stop(&self) -> bool {
        if self.cancel.load(Ordering::SeqCst) {
            return true;
        }
        let deadline = self.deadline.load(Ordering::SeqCst);
        if deadline != 0 && now_ns() >= deadline {
            self.timed_out.store(true, Ordering::SeqCst);
            return true;
        }
        false
    }

    /// A statement starts (under the connection's lock).
    fn arm(&self, timeout_ms: Option<u64>) {
        self.cancel.store(false, Ordering::SeqCst);
        self.timed_out.store(false, Ordering::SeqCst);
        let deadline = timeout_ms
            .map(|ms| now_ns().saturating_add(ms.saturating_mul(1_000_000)))
            .unwrap_or(0);
        self.deadline.store(deadline, Ordering::SeqCst);
    }

    /// The statement is over.
    fn disarm(&self) {
        self.deadline.store(0, Ordering::SeqCst);
    }
}

/// A prepared statement owned outside rusqlite (a cursor's, or the one
/// `db.query_rows` runs). Finalized on drop.
///
/// Invariant (what makes the `Send` below sound): a `RawStmt` is only ever
/// reachable through the [`ConnState`] of the connection it was prepared
/// on, or on the stack of a function that holds that state's lock, so
/// every use — step, read, finalize — happens under the lock, and it is
/// always dropped before the connection is closed.
struct RawStmt(NonNull<ffi::sqlite3_stmt>);

// SAFETY: see the invariant above — the pointer is used by one thread at
// a time, serialized by the connection's mutex, which is exactly what
// SQLite's multi-thread mode requires of a connection and its statements.
unsafe impl Send for RawStmt {}

impl Drop for RawStmt {
    fn drop(&mut self) {
        // SAFETY: the statement is live (finalized only here) and its
        // connection is open (the invariant above).
        unsafe {
            ffi::sqlite3_finalize(self.0.as_ptr());
        }
    }
}

/// A cursor's statement, and how it ended.
struct CursorSlot {
    /// `None` once the rows ran out or a read failed (then finalized).
    stmt: Option<RawStmt>,
    ncols: usize,
    /// The error a failed `db.next` answered; every later call answers it.
    failed: Option<String>,
}

/// What the connection's lock guards. `cursors` is declared (and, in
/// `drop`, cleared) before `conn`, so no statement outlives its
/// connection.
struct ConnState {
    cursors: HashMap<u64, CursorSlot>,
    /// `None` after `db.close`.
    conn: Option<Connection>,
}

impl Drop for ConnState {
    fn drop(&mut self) {
        self.cursors.clear();
    }
}

/// One open connection.
struct ConnEntry {
    state: Mutex<ConnState>,
    control: Arc<Control>,
    /// Cursors dropped while the connection was busy: finalized at its
    /// next use.
    orphans: Mutex<Vec<u64>>,
}

impl ConnEntry {
    fn lock(&self) -> MutexGuard<'_, ConnState> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.reap(&mut st);
        st
    }

    fn reap(&self, st: &mut ConnState) {
        let orphans: Vec<u64> =
            std::mem::take(&mut *self.orphans.lock().unwrap_or_else(|e| e.into_inner()));
        for id in orphans {
            st.cursors.remove(&id);
        }
    }

    /// Finalize a cursor's statement now if the connection is free, else
    /// at its next use. Never waits.
    fn release(&self, id: u64) {
        match self.state.try_lock() {
            Ok(mut st) => {
                st.cursors.remove(&id);
            }
            Err(TryLockError::Poisoned(p)) => {
                p.into_inner().cursors.remove(&id);
            }
            Err(TryLockError::WouldBlock) => {
                self.orphans
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(id);
                // The holder may have let go before the push: reap now if
                // we can, so the statement does not linger to the next use.
                if let Ok(mut st) = self.state.try_lock() {
                    self.reap(&mut st);
                }
            }
        }
    }
}

/// Open connections, keyed by id. `next_id` hands out fresh ids.
struct Registry {
    connections: HashMap<i64, Arc<ConnEntry>>,
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

fn lookup(id: i64) -> Option<Arc<ConnEntry>> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .connections
        .get(&id)
        .cloned()
}

static NEXT_CURSOR: AtomicU64 = AtomicU64::new(1);

/// Creates the db module.
pub fn create_db_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    // The arity registered is the fewest arguments a call takes; the
    // optional ones (params, options) are read when they are there.
    for (name, arity) in [
        ("open", 1),
        ("execute", 2),
        ("query", 2),
        ("query_one", 2),
        ("query_rows", 2),
        ("cursor", 2),
        ("next", 2),
        ("columns", 1),
        ("close_cursor", 1),
        ("interrupt", 1),
        ("close", 1),
        ("transaction", 2),
        ("migrate", 2),
        ("begin", 1),
        ("commit", 1),
        ("rollback", 1),
    ] {
        module.insert(name.to_string(), create_builtin_function(name, arity));
    }

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
        "query_rows" => db_query_rows(args),
        "cursor" => db_cursor(args),
        "next" => db_next(args),
        "columns" => db_columns(args),
        "close_cursor" => db_close_cursor(args),
        "interrupt" => db_interrupt(args),
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

fn list(items: Vec<Value>) -> Value {
    Value::List(Arc::new(items))
}

/// One `db.transaction` at a time per connection. A SQLite connection
/// has one transaction, so two threads sharing a connection — every http
/// worker of a served app — that both ran `db.transaction` interleaved:
/// the second `BEGIN` failed inside the first's, or its statements
/// joined a transaction another thread then rolled back. The lock is
/// per connection and re-entrant on its thread (a nested call reaches
/// SQLite's own "transaction within a transaction" error as before);
/// plain statements do not take it, so a task the transaction's body
/// waits on may still use the connection.
struct TxLock {
    owner: Mutex<Option<(std::thread::ThreadId, usize)>>,
    freed: std::sync::Condvar,
}

fn tx_locks() -> &'static Mutex<HashMap<i64, std::sync::Arc<TxLock>>> {
    static LOCKS: OnceLock<Mutex<HashMap<i64, std::sync::Arc<TxLock>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Held for the length of one `db.transaction`.
pub struct TxGuard(std::sync::Arc<TxLock>);

impl Drop for TxGuard {
    fn drop(&mut self) {
        let mut owner = self.0.owner.lock().unwrap_or_else(|e| e.into_inner());
        match owner.as_mut() {
            Some((_, depth)) if *depth > 1 => *depth -= 1,
            _ => {
                *owner = None;
                self.0.freed.notify_one();
            }
        }
    }
}

/// Wait for the connection's transaction slot. `None` when the value is
/// not a connection — `begin` then says so.
pub fn transaction_enter(conn: &Value) -> Option<TxGuard> {
    let id = connection_id(conn).ok()?;
    let lock = tx_locks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(id)
        .or_insert_with(|| {
            std::sync::Arc::new(TxLock {
                owner: Mutex::new(None),
                freed: std::sync::Condvar::new(),
            })
        })
        .clone();
    let me = std::thread::current().id();
    {
        let mut owner = lock.owner.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match owner.as_mut() {
                None => {
                    *owner = Some((me, 1));
                    break;
                }
                Some((thread, depth)) if *thread == me => {
                    *depth += 1;
                    break;
                }
                Some(_) => {
                    owner = lock.freed.wait(owner).unwrap_or_else(|e| e.into_inner());
                }
            }
        }
    }
    Some(TxGuard(lock))
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
            other => match crate::stdlib::bytes::bytes_of(other) {
                Ok(b) => SqlValue::Blob(b.to_vec()),
                Err(_) => {
                    return Err(rusqlite::Error::ToSqlConversionFailure(
                        format!("db: cannot bind {} as a SQL parameter", other.type_name()).into(),
                    ));
                }
            },
        };
        Ok(ToSqlOutput::Owned(v))
    }
}

/// A call's optional tail: `params` (a list, or `()`), then an options
/// map — or the options map alone where the params would be.
struct Tail {
    params: Vec<Value>,
    timeout_ms: Option<u64>,
}

fn call_tail(func: &str, args: &[Value], from: usize) -> Result<Tail, Value> {
    let mut rest = args.iter().skip(from);
    let mut params = Vec::new();
    let mut opts = None;
    match rest.next() {
        None | Some(Value::Unit) => {}
        Some(Value::List(items)) => params = items.iter().cloned().collect(),
        Some(m @ Value::Map(_)) => opts = Some(m),
        Some(other) => {
            return Err(err(format!(
                "db.{}: params must be a list, got {}",
                func,
                other.type_name()
            )));
        }
    }
    if opts.is_none() {
        match rest.next() {
            None | Some(Value::Unit) => {}
            Some(m @ Value::Map(_)) => opts = Some(m),
            Some(other) => {
                return Err(err(format!(
                    "db.{}: options must be a map (#{{ \"timeout_ms\": … }}), got {}",
                    func,
                    other.type_name()
                )));
            }
        }
    }
    if rest.next().is_some() {
        return Err(err(format!(
            "db.{}: too many arguments (conn, sql, params, options)",
            func
        )));
    }
    let mut timeout_ms = None;
    if let Some(Value::Map(m)) = opts {
        for (key, value) in m.iter() {
            match key.as_str() {
                "timeout_ms" => timeout_ms = timeout_of(func, value)?,
                other => {
                    return Err(err(format!(
                        "db.{}: unknown option \"{}\" (it takes timeout_ms)",
                        func, other
                    )));
                }
            }
        }
    }
    Ok(Tail { params, timeout_ms })
}

fn timeout_of(func: &str, value: &Value) -> Result<Option<u64>, Value> {
    match value {
        Value::Unit => Ok(None),
        Value::Integer(n) if *n > 0 => Ok(Some(*n as u64)),
        other => Err(err(format!(
            "db.{}: timeout_ms must be a positive number of milliseconds, got {}",
            func, other
        ))),
    }
}

fn sql_arg(func: &str, args: &[Value]) -> Result<String, Value> {
    match args.get(1) {
        Some(Value::String(s)) => Ok(s.to_string()),
        _ => Err(err(format!("db.{}: sql must be a string", func))),
    }
}

/// Convert a SQLite column value into an olang value — `db.query`'s
/// mapping, where a BLOB is a (lossy) String.
fn sql_to_value(cell: ValueRef<'_>) -> Value {
    match cell {
        ValueRef::Null => Value::Unit,
        ValueRef::Integer(n) => Value::Integer(n),
        ValueRef::Real(f) => Value::Float(f),
        ValueRef::Text(bytes) => string(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => string(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// Why an operation failed, before it is worded for the caller.
struct Failure {
    /// SQLite answered `SQLITE_INTERRUPT` (an interrupt or a deadline).
    interrupted: bool,
    message: String,
}

impl Failure {
    fn plain(message: impl Into<String>) -> Self {
        Failure {
            interrupted: false,
            message: message.into(),
        }
    }
}

impl From<rusqlite::Error> for Failure {
    fn from(e: rusqlite::Error) -> Self {
        let interrupted = matches!(
            &e,
            rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted
        );
        Failure {
            interrupted,
            message: e.to_string(),
        }
    }
}

/// Run `f` on the connection's locked state, with the interrupt control
/// armed for `timeout_ms`. The answer's `Err` is the olang `Err` value,
/// worded `db.<func>: …`.
fn with_entry<R>(
    func: &str,
    entry: &ConnEntry,
    timeout_ms: Option<u64>,
    f: impl FnOnce(&mut ConnState) -> Result<R, Failure>,
) -> Result<R, Value> {
    let mut st = entry.lock();
    if st.conn.is_none() {
        return Err(err(format!("db.{}: connection is closed", func)));
    }
    entry.control.arm(timeout_ms);
    let outcome = f(&mut st);
    entry.control.disarm();
    drop(st);
    outcome.map_err(|failure| err(worded(func, &entry.control, timeout_ms, failure)))
}

fn worded(func: &str, control: &Control, timeout_ms: Option<u64>, failure: Failure) -> String {
    format!("db.{}: {}", func, reason(control, timeout_ms, failure))
}

/// What a failure says, without the `db.<fn>: ` prefix.
fn reason(control: &Control, timeout_ms: Option<u64>, failure: Failure) -> String {
    if failure.interrupted {
        match timeout_ms {
            Some(ms) if control.timed_out.load(Ordering::SeqCst) => {
                format!("timed out after {} ms", ms)
            }
            _ => "interrupted".to_string(),
        }
    } else {
        failure.message
    }
}

/// `with_entry` for a connection handle.
fn with_conn<R>(
    func: &str,
    conn: &Value,
    timeout_ms: Option<u64>,
    f: impl FnOnce(&mut ConnState) -> Result<R, Failure>,
) -> Result<R, Value> {
    let id = connection_id(conn)?;
    let entry = lookup(id).ok_or_else(|| err(format!("db.{}: connection is closed", func)))?;
    with_entry(func, &entry, timeout_ms, f)
}

fn open_conn(st: &ConnState) -> &Connection {
    st.conn
        .as_ref()
        .expect("with_entry checked the connection is open")
}

// ── raw statements (db.query_rows and cursors) ──────────────────────

/// The connection's last error, as a failure.
///
/// SAFETY: `db` is a live connection handle, used under its lock.
unsafe fn failure_of(db: *mut ffi::sqlite3, code: c_int) -> Failure {
    let msg = unsafe { ffi::sqlite3_errmsg(db) };
    let message = if msg.is_null() {
        format!("SQLite error {}", code)
    } else {
        unsafe { CStr::from_ptr(msg) }
            .to_string_lossy()
            .into_owned()
    };
    Failure {
        interrupted: code & 0xff == ffi::SQLITE_INTERRUPT,
        message,
    }
}

/// Prepare one statement (and refuse a second after it).
///
/// SAFETY: `db` is a live connection handle and the caller holds its lock
/// for as long as the returned statement lives (see [`RawStmt`]).
unsafe fn prepare(db: *mut ffi::sqlite3, sql: &str) -> Result<RawStmt, Failure> {
    let (first, tail) = unsafe { prepare_one(db, sql.as_bytes())? };
    let Some(first) = first else {
        return Err(Failure::plain("no statement to run (the SQL is empty)"));
    };
    // What follows the first statement may be only whitespace and
    // comments: SQLite prepares that as no statement at all.
    if tail < sql.len() {
        match unsafe { prepare_one(db, &sql.as_bytes()[tail..]) } {
            Ok((None, _)) => {}
            _ => {
                return Err(Failure::plain(
                    "more than one statement: run them one at a time",
                ));
            }
        }
    }
    Ok(first)
}

/// SAFETY: as [`prepare`].
unsafe fn prepare_one(
    db: *mut ffi::sqlite3,
    sql: &[u8],
) -> Result<(Option<RawStmt>, usize), Failure> {
    let len = c_int::try_from(sql.len()).map_err(|_| Failure::plain("the SQL is too long"))?;
    let start = sql.as_ptr() as *const c_char;
    let mut stmt: *mut ffi::sqlite3_stmt = std::ptr::null_mut();
    let mut tail: *const c_char = std::ptr::null();
    let rc = unsafe { ffi::sqlite3_prepare_v2(db, start, len, &mut stmt, &mut tail) };
    if rc != ffi::SQLITE_OK {
        if !stmt.is_null() {
            unsafe { ffi::sqlite3_finalize(stmt) };
        }
        return Err(unsafe { failure_of(db, rc) });
    }
    let consumed = if tail.is_null() {
        sql.len()
    } else {
        (tail as usize)
            .saturating_sub(start as usize)
            .min(sql.len())
    };
    Ok((NonNull::new(stmt).map(RawStmt), consumed))
}

/// Bind `params` to the statement's placeholders, in order.
///
/// SAFETY: as [`prepare`]; `stmt` belongs to `db`.
unsafe fn bind(db: *mut ffi::sqlite3, stmt: &RawStmt, params: &[Value]) -> Result<(), Failure> {
    let s = stmt.0.as_ptr();
    let expected = unsafe { ffi::sqlite3_bind_parameter_count(s) } as usize;
    if params.len() != expected {
        return Err(Failure::plain(format!(
            "the statement takes {} parameter(s), {} given",
            expected,
            params.len()
        )));
    }
    for (i, value) in params.iter().enumerate() {
        let at = (i + 1) as c_int;
        let rc = unsafe {
            match value {
                Value::Integer(n) => ffi::sqlite3_bind_int64(s, at, *n),
                Value::Float(f) => ffi::sqlite3_bind_double(s, at, *f),
                Value::Boolean(b) => ffi::sqlite3_bind_int64(s, at, *b as i64),
                Value::Unit => ffi::sqlite3_bind_null(s, at),
                Value::String(text) => {
                    let len = c_int::try_from(text.len())
                        .map_err(|_| Failure::plain("a parameter is too long"))?;
                    ffi::sqlite3_bind_text(
                        s,
                        at,
                        text.as_ptr() as *const c_char,
                        len,
                        ffi::SQLITE_TRANSIENT(),
                    )
                }
                other => match crate::stdlib::bytes::bytes_of(other) {
                    Ok([]) => ffi::sqlite3_bind_zeroblob(s, at, 0),
                    Ok(b) => {
                        let len = c_int::try_from(b.len())
                            .map_err(|_| Failure::plain("a parameter is too long"))?;
                        ffi::sqlite3_bind_blob(
                            s,
                            at,
                            b.as_ptr() as *const std::ffi::c_void,
                            len,
                            ffi::SQLITE_TRANSIENT(),
                        )
                    }
                    Err(_) => {
                        return Err(Failure::plain(format!(
                            "cannot bind {} as a SQL parameter",
                            other.type_name()
                        )));
                    }
                },
            }
        };
        if rc != ffi::SQLITE_OK {
            return Err(unsafe { failure_of(db, rc) });
        }
    }
    Ok(())
}

/// The statement's result columns: `#{ "name", "decltype", "index" }`
/// each, `decltype` `()` for an expression.
///
/// SAFETY: as [`bind`].
unsafe fn columns(stmt: &RawStmt) -> Vec<Value> {
    let s = stmt.0.as_ptr();
    let n = unsafe { ffi::sqlite3_column_count(s) };
    let text = |p: *const c_char| {
        if p.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
        }
    };
    (0..n)
        .map(|i| {
            let name = text(unsafe { ffi::sqlite3_column_name(s, i) }).unwrap_or_default();
            let decltype = text(unsafe { ffi::sqlite3_column_decltype(s, i) });
            let mut m = crate::ast::ValueMap::default();
            m.insert("name".to_string(), string(name));
            m.insert(
                "decltype".to_string(),
                decltype.map(string).unwrap_or(Value::Unit),
            );
            m.insert("index".to_string(), Value::Integer(i as i64));
            Value::Map(Arc::new(m))
        })
        .collect()
}

/// Step once: `Ok(true)` when a row is ready, `Ok(false)` at the end.
///
/// SAFETY: as [`bind`].
unsafe fn step(db: *mut ffi::sqlite3, stmt: &RawStmt) -> Result<bool, Failure> {
    match unsafe { ffi::sqlite3_step(stmt.0.as_ptr()) } {
        ffi::SQLITE_ROW => Ok(true),
        ffi::SQLITE_DONE => Ok(false),
        rc => Err(unsafe { failure_of(db, rc) }),
    }
}

/// The current row, in column order; a BLOB is Bytes.
///
/// SAFETY: as [`bind`], after a `step` that answered a row.
unsafe fn row(stmt: &RawStmt, ncols: usize) -> Value {
    let s = stmt.0.as_ptr();
    let cells = (0..ncols as c_int)
        .map(|i| unsafe {
            match ffi::sqlite3_column_type(s, i) {
                ffi::SQLITE_INTEGER => Value::Integer(ffi::sqlite3_column_int64(s, i)),
                ffi::SQLITE_FLOAT => Value::Float(ffi::sqlite3_column_double(s, i)),
                ffi::SQLITE_TEXT => {
                    let p = ffi::sqlite3_column_text(s, i);
                    let n = ffi::sqlite3_column_bytes(s, i) as usize;
                    if p.is_null() {
                        string("")
                    } else {
                        let b = std::slice::from_raw_parts(p, n);
                        string(String::from_utf8_lossy(b).into_owned())
                    }
                }
                ffi::SQLITE_BLOB => {
                    let p = ffi::sqlite3_column_blob(s, i) as *const u8;
                    let n = ffi::sqlite3_column_bytes(s, i) as usize;
                    if p.is_null() || n == 0 {
                        crate::stdlib::bytes::to_value(Vec::new())
                    } else {
                        crate::stdlib::bytes::to_value(std::slice::from_raw_parts(p, n).to_vec())
                    }
                }
                _ => Value::Unit,
            }
        })
        .collect();
    list(cells)
}

/// Prepare and bind on the locked connection: the statement and its
/// column descriptors.
fn prepare_bound(
    st: &ConnState,
    sql: &str,
    params: &[Value],
) -> Result<(RawStmt, Vec<Value>), Failure> {
    let conn = open_conn(st);
    // SAFETY: the connection is open and its lock is held by our caller
    // for as long as the statement is used (it is either dropped before
    // the lock is released or stored in the locked state).
    unsafe {
        let db = conn.handle();
        let stmt = prepare(db, sql)?;
        bind(db, &stmt, params)?;
        let cols = columns(&stmt);
        Ok((stmt, cols))
    }
}

// ── the cursor value ────────────────────────────────────────────────

/// `db.cursor`'s answer: a handle on a statement the connection's state
/// owns (see the module doc). Dropping the last copy finalizes it.
struct CursorObject {
    id: u64,
    entry: Arc<ConnEntry>,
    columns: Value,
    timeout_ms: Option<u64>,
    sql: String,
}

impl std::fmt::Debug for CursorObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl Drop for CursorObject {
    fn drop(&mut self) {
        self.entry.release(self.id);
    }
}

impl NativeObject for CursorObject {
    fn module(&self) -> &'static str {
        "db"
    }
    fn type_name(&self) -> &'static str {
        "Cursor"
    }
    fn display(&self) -> String {
        // No id: a display is what a replay fingerprints, and a replayed
        // cursor is a different one reading the same statement.
        cursor_display(&self.sql)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<CursorObject>()
            .map(|o| o.id == self.id && Arc::ptr_eq(&o.entry, &self.entry))
            .unwrap_or(false)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn cursor_display(sql: &str) -> String {
    let flat: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut shown: String = flat.chars().take(48).collect();
    if shown.len() < flat.len() {
        shown.push('…');
    }
    format!("<db cursor: {}>", shown)
}

/// The statement a replayed cursor stood for (the timeline revives a
/// recorded cursor as this; every operation on it replays from the log,
/// so it is never read).
pub fn cursor_sql(value: &Value) -> Option<String> {
    match value {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<CursorObject>()
                .map(|c| c.sql.clone())
        }
        _ => None,
    }
}

/// A cursor on no connection, for replay (see [`cursor_sql`]).
pub fn detached_cursor(sql: &str) -> Value {
    let entry = Arc::new(ConnEntry {
        state: Mutex::new(ConnState {
            cursors: HashMap::new(),
            conn: None,
        }),
        control: Arc::new(Control::default()),
        orphans: Mutex::new(Vec::new()),
    });
    Value::Native(NativeHandle::new(CursorObject {
        id: NEXT_CURSOR.fetch_add(1, Ordering::Relaxed),
        entry,
        columns: list(Vec::new()),
        timeout_ms: None,
        sql: sql.to_string(),
    }))
}

fn cursor_of<'a>(func: &str, value: &'a Value) -> Result<&'a CursorObject, Value> {
    match value {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<CursorObject>()
                .ok_or_else(|| err(format!("db.{}: expected a cursor (from db.cursor)", func)))
        }
        _ => Err(err(format!(
            "db.{}: expected a cursor (from db.cursor), got {}",
            func,
            value.type_name()
        ))),
    }
}

// ── operations ──────────────────────────────────────────────────────

/// db.migrate(conn, steps) -> Result<Int>: bring the database to the head
/// of `steps` — a list of versions, each a list of statements (or one
/// statement). A `schema_version` table records the applied version;
/// version N runs only when the recorded version is below N, inside its
/// own transaction, so a failed step leaves the database at the version
/// before it. Returns the version now recorded.
fn db_migrate(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if let Err(e) = connection_id(args.first().unwrap_or(&UNIT)) {
        return Ok(e);
    }
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

    // Every failure below is already worded; `with_conn` adds the prefix.
    let outcome = with_conn("migrate", &args[0], None, |st| {
        let conn = open_conn(st);
        let fail = |what: &str, e: rusqlite::Error| {
            let mut f = Failure::from(e);
            f.message = format!("{}: {}", what, f.message);
            f
        };
        conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)",
            [],
        )
        .map_err(|e| fail("schema_version", e))?;
        let recorded: Option<i64> =
            match conn.query_row("SELECT version FROM schema_version", [], |row| row.get(0)) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(fail("schema_version", e)),
            };
        let mut version = match recorded {
            Some(v) => v,
            None => {
                conn.execute("INSERT INTO schema_version (version) VALUES (0)", [])
                    .map_err(|e| fail("schema_version", e))?;
                0
            }
        };
        while (version as usize) < steps.len() {
            let next = version + 1;
            conn.execute_batch("BEGIN").map_err(|e| fail("begin", e))?;
            for stmt in &steps[version as usize] {
                if let Err(e) = conn.execute_batch(stmt) {
                    let _ = conn.execute_batch("ROLLBACK");
                    // The failing statement is the bug report — surface it.
                    return Err(fail(&format!("migration v{} failed", next), e));
                }
            }
            if let Err(e) = conn.execute("UPDATE schema_version SET version = ?1", [next]) {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(fail("schema_version", e));
            }
            conn.execute_batch("COMMIT")
                .map_err(|e| fail("commit", e))?;
            version = next;
        }
        Ok(version)
    });
    Ok(match outcome {
        Ok(version) => ok(Value::Integer(version)),
        Err(e) => e,
    })
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

/// Is this path a real file path SQLite would create (not in-memory, not
/// a URI, which SQLite reads itself)?
fn is_plain_file_path(path: &str) -> bool {
    !(path.is_empty() || path == ":memory:" || path.starts_with("file:"))
}

/// db.open(path[, opts]) -> Result<Connection>. `":memory:"` for an
/// in-memory database; SQLite URI filenames (`file:…?mode=ro`) are read
/// as URIs. Options: `readonly` (default false) and `create` (default
/// true, and false when `readonly`).
fn db_open(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let path = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        _ => return Ok(err("db.open: path must be a string")),
    };
    let mut readonly = false;
    let mut create = None;
    match args.get(1) {
        None | Some(Value::Unit) => {}
        Some(Value::Map(m)) => {
            for (key, value) in m.iter() {
                if key != "readonly" && key != "create" {
                    return Ok(err(format!(
                        "db.open: unknown option \"{}\" (it takes readonly and create)",
                        key
                    )));
                }
                let flag = match value {
                    Value::Boolean(b) => *b,
                    other => {
                        return Ok(err(format!(
                            "db.open: {} must be true or false, got {}",
                            key, other
                        )));
                    }
                };
                if key == "readonly" {
                    readonly = flag;
                } else {
                    create = Some(flag);
                }
            }
        }
        Some(other) => {
            return Ok(err(format!(
                "db.open: options must be a map (#{{ \"readonly\": true, \"create\": false }}), got {}",
                other.type_name()
            )));
        }
    }
    if args.len() > 2 {
        return Ok(err("db.open: too many arguments (path, options)"));
    }
    if readonly && create == Some(true) {
        return Ok(err(
            "db.open: readonly and create cannot both be true: a read-only open never makes a file",
        ));
    }
    let create = create.unwrap_or(!readonly);
    if !create && is_plain_file_path(&path) && !std::path::Path::new(&path).exists() {
        let why = if readonly {
            "a read-only open never makes one"
        } else {
            "create: false"
        };
        return Ok(err(format!("db.open: no database at {} ({})", path, why)));
    }

    let mut flags = OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    flags |= if readonly {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let conn = match Connection::open_with_flags(&path, flags) {
        Ok(c) => c,
        Err(e) => return Ok(err(format!("db.open: {}: {}", path, e))),
    };

    let control = Arc::new(Control::default());
    let watch = control.clone();
    if let Err(e) = conn.progress_handler(PROGRESS_OPS, Some(move || watch.should_stop())) {
        return Ok(err(format!("db.open: {}", e)));
    }
    let entry = Arc::new(ConnEntry {
        state: Mutex::new(ConnState {
            cursors: HashMap::new(),
            conn: Some(conn),
        }),
        control,
        orphans: Mutex::new(Vec::new()),
    });

    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    let id = reg.next_id;
    reg.next_id += 1;
    reg.connections.insert(id, entry);

    let mut fields = crate::ast::ValueMap::default();
    fields.insert("id".to_string(), Value::Integer(id));
    fields.insert("path".to_string(), string(path));
    Ok(ok(Value::Struct {
        type_name: "Connection".to_string(),
        fields: std::sync::Arc::new(fields),
    }))
}

/// db.execute(conn, sql[, params][, opts]) -> Result<Int> (rows
/// affected). For statements that change data (CREATE/INSERT/UPDATE/DELETE).
fn db_execute(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if let Err(e) = connection_id(args.first().unwrap_or(&UNIT)) {
        return Ok(e);
    }
    let sql = match sql_arg("execute", &args) {
        Ok(s) => s,
        Err(e) => return Ok(e),
    };
    let tail = match call_tail("execute", &args, 2) {
        Ok(t) => t,
        Err(e) => return Ok(e),
    };
    let params: Vec<Param> = tail.params.into_iter().map(Param).collect();
    let outcome = with_conn("execute", &args[0], tail.timeout_ms, |st| {
        let bound: Vec<&dyn ToSql> = params.iter().map(|p| p as &dyn ToSql).collect();
        Ok(open_conn(st).execute(&sql, bound.as_slice())?)
    });
    Ok(match outcome {
        Ok(affected) => ok(Value::Integer(affected as i64)),
        Err(e) => e,
    })
}

/// db.query(conn, sql[, params][, opts]) -> Result<List<Map>>. Each row is
/// a map from column name to value. query_one returns Ok(row) or Ok(unit)
/// when there is no row.
fn db_query(args: Vec<Value>, one: bool) -> Result<Value, Box<dyn std::error::Error>> {
    let func = if one { "query_one" } else { "query" };
    if let Err(e) = connection_id(args.first().unwrap_or(&UNIT)) {
        return Ok(e);
    }
    let sql = match sql_arg(func, &args) {
        Ok(s) => s,
        Err(e) => return Ok(e),
    };
    let tail = match call_tail(func, &args, 2) {
        Ok(t) => t,
        Err(e) => return Ok(e),
    };
    let params: Vec<Param> = tail.params.into_iter().map(Param).collect();

    let outcome = with_conn(func, &args[0], tail.timeout_ms, |st| {
        let mut stmt = open_conn(st).prepare(&sql)?;
        let column_names: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
        let bound: Vec<&dyn ToSql> = params.iter().map(|p| p as &dyn ToSql).collect();
        let mut rows = stmt.query(bound.as_slice())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut map = crate::ast::ValueMap::default();
            for (i, name) in column_names.iter().enumerate() {
                let cell = row.get_ref(i).map(sql_to_value).unwrap_or(Value::Unit);
                map.insert(name.clone(), cell);
            }
            out.push(Value::Map(std::sync::Arc::new(map)));
            if one {
                break;
            }
        }
        Ok(out)
    });
    Ok(match outcome {
        Ok(mut rows) if one => ok(if rows.is_empty() {
            Value::Unit
        } else {
            rows.swap_remove(0)
        }),
        Ok(rows) => ok(list(rows)),
        Err(e) => e,
    })
}

/// db.query_rows(conn, sql[, params][, opts]) ->
/// Result<#{ columns: [#{ name, decltype, index }], rows: [[value]] }>:
/// the columns in order (repeated names kept, known with no rows), each
/// row a list in that order, BLOBs as Bytes.
fn db_query_rows(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if let Err(e) = connection_id(args.first().unwrap_or(&UNIT)) {
        return Ok(e);
    }
    let sql = match sql_arg("query_rows", &args) {
        Ok(s) => s,
        Err(e) => return Ok(e),
    };
    let tail = match call_tail("query_rows", &args, 2) {
        Ok(t) => t,
        Err(e) => return Ok(e),
    };
    let outcome = with_conn("query_rows", &args[0], tail.timeout_ms, |st| {
        let (stmt, cols) = prepare_bound(st, &sql, &tail.params)?;
        let db = unsafe { open_conn(st).handle() };
        let mut rows = Vec::new();
        // SAFETY: the statement lives on this stack frame, under the lock.
        unsafe {
            while step(db, &stmt)? {
                rows.push(row(&stmt, cols.len()));
            }
        }
        Ok((cols, rows))
    });
    Ok(match outcome {
        Ok((cols, rows)) => {
            let mut m = crate::ast::ValueMap::default();
            m.insert("columns".to_string(), list(cols));
            m.insert("rows".to_string(), list(rows));
            ok(Value::Map(Arc::new(m)))
        }
        Err(e) => e,
    })
}

/// db.cursor(conn, sql[, params][, opts]) -> Result<Cursor>: the
/// statement prepared and bound, not yet run. `timeout_ms` bounds each
/// `db.next`.
fn db_cursor(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let id = match connection_id(args.first().unwrap_or(&UNIT)) {
        Ok(id) => id,
        Err(e) => return Ok(e),
    };
    let sql = match sql_arg("cursor", &args) {
        Ok(s) => s,
        Err(e) => return Ok(e),
    };
    let tail = match call_tail("cursor", &args, 2) {
        Ok(t) => t,
        Err(e) => return Ok(e),
    };
    let Some(entry) = lookup(id) else {
        return Ok(err("db.cursor: connection is closed"));
    };
    let cursor_id = NEXT_CURSOR.fetch_add(1, Ordering::Relaxed);
    let outcome = with_entry("cursor", &entry, tail.timeout_ms, |st| {
        let (stmt, cols) = prepare_bound(st, &sql, &tail.params)?;
        st.cursors.insert(
            cursor_id,
            CursorSlot {
                stmt: Some(stmt),
                ncols: cols.len(),
                failed: None,
            },
        );
        Ok(cols)
    });
    Ok(match outcome {
        Ok(cols) => ok(Value::Native(NativeHandle::new(CursorObject {
            id: cursor_id,
            entry,
            columns: list(cols),
            timeout_ms: tail.timeout_ms,
            sql,
        }))),
        Err(e) => e,
    })
}

/// db.next(cur, n) -> Result<List<List>>: up to `n` more rows, `[]` at
/// the end. A failed read (an error, an interrupt, a timeout) finishes
/// the cursor: that call and every later one answer the same Err.
/// What a missing argument reads as. A named static because `Value`
/// implements `Drop`, so `&Value::Unit` is no longer promoted to one.
static UNIT: Value = Value::Unit;

fn db_next(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let cur = match cursor_of("next", args.first().unwrap_or(&UNIT)) {
        Ok(c) => c,
        Err(e) => return Ok(e),
    };
    let n = match args.get(1) {
        Some(Value::Integer(n)) if *n > 0 => *n as usize,
        Some(other) => {
            return Ok(err(format!(
                "db.next: n must be a positive integer, got {}",
                other
            )));
        }
        None => return Ok(err("db.next: n (how many rows) is required")),
    };
    if args.len() > 2 {
        return Ok(err("db.next: too many arguments (cursor, n)"));
    }
    let id = cur.id;
    let timeout_ms = cur.timeout_ms;
    let entry = &cur.entry;
    let outcome = with_entry("next", entry, timeout_ms, |st| {
        let db = unsafe { open_conn(st).handle() };
        let Some(slot) = st.cursors.get_mut(&id) else {
            return Err(Failure::plain("the cursor is closed"));
        };
        if let Some(failed) = &slot.failed {
            return Err(Failure::plain(format!("the cursor failed: {}", failed)));
        }
        let mut rows = Vec::new();
        let Some(stmt) = slot.stmt.as_ref() else {
            return Ok(rows);
        };
        while rows.len() < n {
            // SAFETY: the statement is owned by the locked state.
            match unsafe { step(db, stmt) } {
                Ok(true) => rows.push(unsafe { row(stmt, slot.ncols) }),
                Ok(false) => {
                    // The end: finalize now, so the read ends with it.
                    slot.stmt = None;
                    break;
                }
                Err(failure) => {
                    // Finished: remember why, so every later call says so.
                    let why = reason(&entry.control, timeout_ms, failure);
                    slot.stmt = None;
                    slot.failed = Some(why.clone());
                    return Err(Failure::plain(why));
                }
            }
        }
        Ok(rows)
    });
    Ok(match outcome {
        Ok(rows) => ok(list(rows)),
        Err(e) => e,
    })
}

/// db.columns(cur) -> Result<List<Map>>: the cursor's column descriptors.
fn db_columns(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(match cursor_of("columns", args.first().unwrap_or(&UNIT)) {
        Ok(c) => ok(c.columns.clone()),
        Err(e) => e,
    })
}

/// db.close_cursor(cur) -> Result<Unit>: finalize now. Closing a closed
/// cursor is no error.
fn db_close_cursor(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let cur = match cursor_of("close_cursor", args.first().unwrap_or(&UNIT)) {
        Ok(c) => c,
        Err(e) => return Ok(e),
    };
    cur.entry.lock().cursors.remove(&cur.id);
    Ok(ok(Value::Unit))
}

/// db.interrupt(conn_or_cursor) -> Result<Unit>: stop the statement the
/// connection is running now, from any thread; it answers
/// `Err("db.<fn>: interrupted")`. Takes no lock.
fn db_interrupt(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let target = args.first().unwrap_or(&UNIT);
    let entry = match cursor_of("interrupt", target) {
        Ok(cur) => Some(cur.entry.clone()),
        Err(_) => match connection_id(target) {
            Ok(id) => lookup(id),
            Err(_) => {
                return Ok(err(
                    "db.interrupt: expected a connection (from db.open) or a cursor (from db.cursor)",
                ));
            }
        },
    };
    if let Some(entry) = entry {
        entry.control.cancel.store(true, Ordering::SeqCst);
    }
    Ok(ok(Value::Unit))
}

/// db.close(conn) -> Result<Unit>. Drops the connection from the registry,
/// finalizing its cursors first.
fn db_close(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let id = match connection_id(args.first().unwrap_or(&UNIT)) {
        Ok(id) => id,
        Err(e) => return Ok(e),
    };
    tx_locks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    let entry = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .connections
        .remove(&id);
    let Some(entry) = entry else {
        return Ok(err("db.close: connection is already closed"));
    };
    let mut st = entry.lock();
    st.cursors.clear();
    match st.conn.take() {
        // Explicit close surfaces any final error
        Some(conn) => match conn.close() {
            Ok(()) => Ok(ok(Value::Unit)),
            Err((_, e)) => Ok(err(format!("db.close: {}", e))),
        },
        None => Ok(err("db.close: connection is already closed")),
    }
}

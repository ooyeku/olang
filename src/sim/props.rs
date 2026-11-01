//! Properties (S4): what must hold of a world as it runs, as values.
//!
//! ```text
//! sim.always("an acked put is durable", (s) => durable(s) >= acked(s))
//! sim.never("two leaders", (s) => leaders(s) > 1)
//! sim.eventually("the replicas agree", sim.seconds(30), (s) => agree(s))
//! sim.linearizable("the register", "reg", sim.register(0))
//! let run = w.run(#{ "for": sim.hours(1), "props": [...] })      // stops at the first violation
//! sim.check(run, [...])                                           // the same, after the fact: raises
//! ```
//!
//! **When a property is checked.** At every *instant* of virtual time
//! at which anything ran: when nothing is left to run at the current
//! millisecond and the clock is about to move on (and once more at the
//! run's end). Running code costs no virtual time, so an instant is the
//! smallest step the world can be seen to take; a state that holds only
//! in the middle of one instant (between two tasks' turns at the same
//! millisecond) is not seen. Checking at every yield point instead would
//! cost a predicate call per scheduler step (millions per run).
//!
//! - `always(name, pred)`: `pred(s)` is true at every check.
//! - `never(name, pred)`: `pred(s)` is false at every check.
//! - `eventually(name, within, pred)`: `pred(s)` is never false for longer
//!   than `within` ms at a stretch. When it turns false the world sets a
//!   check at the stretch's deadline, so a violation is reported at the
//!   millisecond it happens even if the world is quiet then. A stretch
//!   still open when the run ends is not a violation (the run may simply
//!   have ended too soon).
//! - `linearizable(name, history, model)`: checked once, at the end, over
//!   the history recorded under `history` (a name) or given (a list).
//!
//! **What a predicate sees.** One argument, the world's state `s`:
//! `s.now` (virtual ms), `s.time` (the world's wall clock), `s.step`,
//! `s.history(name)` (what `sim.record` kept so far), `s.up(node)`, and
//! `s.disk(node)`: a node's disk as a power cut now would leave it —
//! `.read(path)` (its durable contents, `()` when the name is not
//! durable), `.read_bytes(path)`, `.exists(path)`, `.list(dir)`,
//! `.current(path)` (what the node reads now, synced or not), and
//! `.query(db, sql, params)`: a query on the durable image of a SQLite
//! database (a copy with only what was synced, opened through the world's
//! VFS and recovered as SQLite would after the crash).
//!
//! A predicate runs with the world paused (no task runs while it does)
//! and changes nothing: its random draws come from a stream of its own,
//! and every effect (`time.now_ms`, `fs`, `http`, `spawn`, ...) is
//! refused. A predicate that raises, or answers anything but a Bool,
//! breaks its property, with the error as the reason.

use super::lin::{self, Model, Verdict};
use super::{World, current, vfs};
use crate::ast::{Value, ValueMap};
use crate::interpreter::Interpreter;
use crate::native::{NativeHandle, NativeObject};
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::any::Any;
use std::cell::RefCell;
use std::sync::{Arc, Mutex, Weak};

/// The default bound on a linearizability search, in steps.
pub const LIN_MAX_STEPS: u64 = 2_000_000;

/// A property that broke: which, when, at which step, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub property: String,
    /// always, never, eventually or linearizable.
    pub kind: &'static str,
    pub t: u64,
    pub step: u64,
    pub detail: String,
}

impl Violation {
    pub fn describe(&self) -> String {
        let mut text = format!(
            "\"{}\" broke at {} (step {})",
            self.property,
            super::cli::fmt_virtual(self.t),
            self.step
        );
        if !self.detail.is_empty() {
            text.push_str(": ");
            text.push_str(&self.detail);
        }
        text
    }
}

/// Where a linearizability property's history comes from.
#[derive(Debug, Clone)]
pub enum Hist {
    Named(String),
    Given(Arc<Vec<Value>>),
}

#[derive(Debug, Clone)]
pub enum Kind {
    Always,
    Never,
    Eventually(u64),
    Linearizable { history: Hist, model: Value },
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Always => "always",
            Kind::Never => "never",
            Kind::Eventually(_) => "eventually",
            Kind::Linearizable { .. } => "linearizable",
        }
    }
}

/// A property, as `sim.always` and its kin make it.
#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub kind: Kind,
    /// The predicate (`()` for linearizability).
    pub pred: Value,
}

impl Property {
    /// Checked only at the end of a run (over its history).
    pub fn at_end_only(&self) -> bool {
        matches!(self.kind, Kind::Linearizable { .. })
    }
}

/// A property as an olang value.
#[derive(Debug)]
pub struct PropertyObject(pub Property);

impl NativeObject for PropertyObject {
    fn module(&self) -> &'static str {
        "sim.Property"
    }
    fn type_name(&self) -> &'static str {
        "Property"
    }
    fn display(&self) -> String {
        format!("<property {} \"{}\">", self.0.kind.name(), self.0.name)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<PropertyObject>()
            .is_some_and(|o| o.0.name == self.0.name && o.0.kind.name() == self.0.kind.name())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn field(&self, name: &str) -> Option<Result<Value, String>> {
        match name {
            "name" => Some(Ok(s(&self.0.name))),
            "kind" => Some(Ok(s(self.0.kind.name()))),
            "within" => Some(Ok(match self.0.kind {
                Kind::Eventually(ms) => Value::Integer(ms as i64),
                _ => Value::Unit,
            })),
            _ => None,
        }
    }
}

/// The properties a run checks, and the interpreter their predicates
/// run on.
pub struct Checker {
    props: Vec<Property>,
    interp: Option<Interpreter>,
    /// eventually: since when each has been false.
    false_since: Vec<Option<u64>>,
    pub checks: u64,
}

impl Checker {
    pub fn new(props: Vec<Property>, interp: Option<Interpreter>) -> Checker {
        let n = props.len();
        Checker {
            props,
            interp,
            false_since: vec![None; n],
            checks: 0,
        }
    }
}

fn s(t: &str) -> Value {
    Value::String(Arc::new(t.to_string()))
}

// ─── checking mode ───────────────────────────────────────────────────

struct Checking {
    rng: Arc<Mutex<StdRng>>,
}

thread_local! {
    static CHECKING: RefCell<Option<Checking>> = const { RefCell::new(None) };
}

/// Is this thread running a property's predicate (or a model's step)?
pub fn checking() -> bool {
    CHECKING.with(|c| c.borrow().is_some())
}

/// The predicate's own random stream, while one runs.
pub fn check_rng() -> Option<Arc<Mutex<StdRng>>> {
    CHECKING.with(|c| c.borrow().as_ref().map(|k| k.rng.clone()))
}

/// While held, builtins called on this thread are a predicate's: pure
/// ones run, effects are refused, random draws come from `seed`'s stream.
pub(super) struct CheckGuard(Option<Checking>);

pub(super) fn enter_checking(seed: u64) -> CheckGuard {
    let previous = CHECKING.with(|c| {
        c.borrow_mut().replace(Checking {
            rng: Arc::new(Mutex::new(StdRng::seed_from_u64(seed))),
        })
    });
    CheckGuard(previous)
}

impl Drop for CheckGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        CHECKING.with(|c| *c.borrow_mut() = previous);
    }
}

/// The refusal a predicate meets for an effect.
pub fn refusal(name: &str) -> String {
    format!(
        "sim: a property's predicate cannot call {name}: it reads the world through its \
         state (s.now, s.disk(node), s.history(name), s.up(node)) and changes nothing"
    )
}

// ─── the check ───────────────────────────────────────────────────────

/// What a predicate's answer means for its property: `Ok(holds)`, or
/// why it could not be read.
fn holds(answer: Result<Value, crate::interpreter::InterpreterError>) -> Result<bool, String> {
    match answer {
        Ok(Value::Boolean(b)) => Ok(b),
        Ok(other) => Err(format!(
            "its predicate answered {} ({}), not a Bool",
            super::render(&other),
            other.type_name()
        )),
        Err(e) => Err(format!("its predicate raised: {}", super::error_text(&e))),
    }
}

impl World {
    /// Check every property against the world as it stands at virtual
    /// time `now` (step `step`); `last` at the run's end (when
    /// linearizability is checked too). The first that breaks.
    pub(super) fn check_props(&self, now: u64, step: u64, last: bool) -> Option<Violation> {
        let checker = self.checker.as_ref()?;
        let mut c = checker.lock().unwrap_or_else(|e| e.into_inner());
        let world = current().map(|ctx| Arc::downgrade(&ctx.world))?;
        let broke = |p: &Property, detail: String| Violation {
            property: p.name.clone(),
            kind: p.kind.name(),
            t: now,
            step,
            detail,
        };
        let Some(mut interp) = c.interp.take() else {
            let p = c.props.first()?.clone();
            return Some(broke(
                &p,
                "this world has no interpreter to run predicates on".into(),
            ));
        };
        let _checking = enter_checking(super::sched::split(self.seed, 8) ^ step);
        let state = Value::Native(NativeHandle::new(StateObject {
            world: world.clone(),
            now,
            step,
            wall: self.start_ms + now as i64,
        }));
        let mut found = None;
        for i in 0..c.props.len() {
            let p = c.props[i].clone();
            match &p.kind {
                Kind::Always | Kind::Never => {
                    let answer = interp.call_function(p.pred.clone(), vec![state.clone()]);
                    match holds(answer) {
                        Ok(b) if b == matches!(p.kind, Kind::Always) => {}
                        Ok(_) => found = Some(broke(&p, String::new())),
                        Err(why) => found = Some(broke(&p, why)),
                    }
                }
                Kind::Eventually(within) => {
                    let answer = interp.call_function(p.pred.clone(), vec![state.clone()]);
                    match holds(answer) {
                        Ok(true) => c.false_since[i] = None,
                        Ok(false) => match c.false_since[i] {
                            None => {
                                c.false_since[i] = Some(now);
                                self.lock().sched.check_at(now + within + 1);
                            }
                            Some(since) if now - since > *within => {
                                found = Some(broke(
                                    &p,
                                    format!(
                                        "false since {} ({} ms, allowed {within})",
                                        super::cli::fmt_virtual(since),
                                        now - since
                                    ),
                                ))
                            }
                            Some(_) => {}
                        },
                        Err(why) => found = Some(broke(&p, why)),
                    }
                }
                Kind::Linearizable { history, model } if last => {
                    let entries = match history {
                        Hist::Given(h) => h.clone(),
                        Hist::Named(name) => self.history_of(name),
                    };
                    match linearize(&mut interp, &entries, model, LIN_MAX_STEPS) {
                        Ok(Verdict::Linearizable) => {}
                        Ok(Verdict::Violation(why)) => found = Some(broke(&p, why)),
                        Ok(Verdict::Unknown(why)) => {
                            // not a violation: said, and the run goes on
                            let mut st = self.lock();
                            let running = st.running;
                            self.event(
                                &mut st,
                                running,
                                "unknown",
                                format!("\"{}\": {why}", p.name),
                            );
                        }
                        Err(why) => found = Some(broke(&p, why)),
                    }
                }
                Kind::Linearizable { .. } => {}
            }
            if found.is_some() {
                break;
            }
        }
        c.interp = Some(interp);
        c.checks += 1;
        found
    }

    /// The history recorded under `name` so far.
    pub(super) fn history_of(&self, name: &str) -> Arc<Vec<Value>> {
        self.lock().histories.get(name).cloned().unwrap_or_default()
    }

    /// `sim.record(name, op)` by task `me`.
    pub(super) fn record(&self, me: usize, name: &str, op: &ValueMap) -> Result<(), String> {
        match op.get("type") {
            Some(Value::String(t)) if matches!(t.as_str(), "invoke" | "ok" | "fail" | "info") => {}
            _ => {
                return Err(
                    "sim.record: an op is a map whose `type` is invoke, ok, fail or info".into(),
                );
            }
        }
        let mut st = self.lock();
        let node = st.task_node.get(me).copied().unwrap_or(0);
        let mut entry = op.clone();
        if !entry.contains_key("process") {
            entry.insert(
                "process".into(),
                s(&format!("{}/{}", self.nodes[node].name, me)),
            );
        }
        entry.insert("node".into(), s(&self.nodes[node].name));
        entry.insert("t".into(), Value::Integer(st.sched.now as i64));
        st.record_seq += 1;
        entry.insert("index".into(), Value::Integer(st.record_seq as i64));
        let list = st.histories.entry(name.to_string()).or_default();
        Arc::make_mut(list).push(Value::Map(Arc::new(entry)));
        Ok(())
    }
}

// ─── models ──────────────────────────────────────────────────────────

/// An olang model: an initial state and `step(state, op)`.
struct OlangModel<'a> {
    interp: &'a mut Interpreter,
    init: Value,
    step: Value,
}

impl Model for OlangModel<'_> {
    fn init(&self) -> Value {
        self.init.clone()
    }
    fn step(&mut self, state: &Value, op: &lin::Op) -> Result<Option<Value>, String> {
        match &self
            .interp
            .call_function(self.step.clone(), vec![state.clone(), lin::op_value(op)])
        {
            Ok(Value::Ok(next)) => Ok(Some((**next).clone())),
            Ok(Value::Err(_)) => Ok(None),
            Ok(other) => Err(format!(
                "a model's step answers Ok(next) or Err(why), got {}",
                other.type_name()
            )),
            Err(e) => Err(format!(
                "the model's step raised: {}",
                super::error_text(&e)
            )),
        }
    }
}

/// Check `history` against `model` (`sim.register`, `sim.kv`, or
/// `#{ init, step }`), with `interp` for an olang step function.
pub fn linearize(
    interp: &mut Interpreter,
    history: &[Value],
    model: &Value,
    max_steps: u64,
) -> Result<Verdict, String> {
    let Value::Map(m) = model else {
        return Err(format!(
            "a model is sim.register(init), sim.kv(), or #{{ init, step }}, got {}",
            model.type_name()
        ));
    };
    match m.get("model") {
        Some(Value::String(k)) if k.as_str() == "register" => lin::check(
            history,
            &mut lin::Register {
                init: m.get("init").cloned().unwrap_or(Value::Unit),
            },
            max_steps,
        ),
        Some(Value::String(k)) if k.as_str() == "kv" => {
            lin::check(history, &mut lin::Kv, max_steps)
        }
        _ => {
            let step = match m.get("step") {
                Some(f @ (Value::Function(_) | Value::Builtin(_))) => f.clone(),
                _ => return Err("a model #{ init, step } needs a step function".into()),
            };
            let init = m.get("init").cloned().unwrap_or(Value::Unit);
            let _checking = if checking() {
                None
            } else {
                Some(enter_checking(0))
            };
            lin::check(history, &mut OlangModel { interp, init, step }, max_steps)
        }
    }
}

/// A verdict as olang data: `#{ ok, verdict, reason }` (`ok` is `()` when
/// the search was cut off).
pub fn verdict_value(v: &Verdict) -> Value {
    let mut m = ValueMap::default();
    let (ok, name, reason) = match v {
        Verdict::Linearizable => (Value::Boolean(true), "linearizable", String::new()),
        Verdict::Violation(why) => (Value::Boolean(false), "violation", why.clone()),
        Verdict::Unknown(why) => (Value::Unit, "unknown", why.clone()),
    };
    m.insert("ok".into(), ok);
    m.insert("verdict".into(), s(name));
    m.insert("reason".into(), s(&reason));
    Value::Map(Arc::new(m))
}

// ─── what a predicate sees ───────────────────────────────────────────

/// `s`: the world's state, as a predicate sees it.
#[derive(Debug)]
pub struct StateObject {
    pub world: Weak<World>,
    pub now: u64,
    pub step: u64,
    pub wall: i64,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<world seed {}>", self.seed)
    }
}

impl NativeObject for StateObject {
    fn module(&self) -> &'static str {
        "sim.State"
    }
    fn type_name(&self) -> &'static str {
        "State"
    }
    fn display(&self) -> String {
        format!("<world state at {} ms>", self.now)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<StateObject>()
            .is_some_and(|o| o.step == self.step && o.world.ptr_eq(&self.world))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn field(&self, name: &str) -> Option<Result<Value, String>> {
        match name {
            "now" => Some(Ok(Value::Integer(self.now as i64))),
            "step" => Some(Ok(Value::Integer(self.step as i64))),
            "time" => Some(Ok(Value::Integer(self.wall))),
            "nodes" => Some(self.live().map(|w| {
                Value::List(Arc::new(
                    w.nodes
                        .iter()
                        .skip(1)
                        .filter(|n| n.kind != super::node::Kind::Hook)
                        .map(|n| s(&n.name))
                        .collect(),
                ))
            })),
            _ => None,
        }
    }
}

impl StateObject {
    fn live(&self) -> Result<Arc<World>, String> {
        self.world
            .upgrade()
            .ok_or_else(|| "sim: this state's world has ended".to_string())
    }

    /// `s.disk(node)`.
    pub fn disk(&self, node: &str) -> Result<Value, String> {
        let w = self.live()?;
        let n = w
            .node_named(node)
            .ok_or_else(|| format!("s.disk: the world has no node `{node}`"))?;
        Ok(Value::Native(NativeHandle::new(DiskObject {
            world: self.world.clone(),
            node: n,
            name: node.to_string(),
        })))
    }

    /// `s.history(name)`.
    pub fn history(&self, name: &str) -> Result<Value, String> {
        Ok(Value::List(self.live()?.history_of(name)))
    }

    /// `s.up(node)`.
    pub fn up(&self, node: &str) -> Result<Value, String> {
        let w = self.live()?;
        let n = w
            .node_named(node)
            .ok_or_else(|| format!("s.up: the world has no node `{node}`"))?;
        Ok(Value::Boolean(w.lock().nodes[n].up))
    }
}

/// `s.disk(node)`: a node's disk, as a power cut now would leave it.
#[derive(Debug)]
pub struct DiskObject {
    world: Weak<World>,
    node: usize,
    name: String,
}

impl NativeObject for DiskObject {
    fn module(&self) -> &'static str {
        "sim.Disk"
    }
    fn type_name(&self) -> &'static str {
        "Disk"
    }
    fn display(&self) -> String {
        format!("<disk of {}>", self.name)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<DiskObject>()
            .is_some_and(|o| o.node == self.node && o.world.ptr_eq(&self.world))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn field(&self, name: &str) -> Option<Result<Value, String>> {
        match name {
            "node" => Some(Ok(s(&self.name))),
            _ => None,
        }
    }
}

impl DiskObject {
    fn with<T>(&self, f: impl FnOnce(&super::disk::Disk, &str) -> T) -> Result<T, String> {
        let w = self
            .world
            .upgrade()
            .ok_or_else(|| "sim: this disk's world has ended".to_string())?;
        let cwd = w.lock().nodes[self.node].cwd.clone();
        let d = vfs::lock(&w.disks[self.node]);
        Ok(f(&d.disk, &cwd))
    }

    /// `.read(path)`: the durable contents as text (`()` when the name is
    /// not durable).
    pub fn read(&self, path: &str, bytes: bool) -> Result<Value, String> {
        self.with(
            |d, cwd| match d.durable_read(&super::disk::normalize(cwd, path)) {
                None => Value::Unit,
                Some(b) if bytes => crate::stdlib::bytes::to_value(b.to_vec()),
                Some(b) => s(&String::from_utf8_lossy(b)),
            },
        )
    }

    /// `.current(path)`: what the node reads now, synced or not.
    pub fn current(&self, path: &str) -> Result<Value, String> {
        self.with(|d, cwd| match d.read(&super::disk::normalize(cwd, path)) {
            Ok(b) => s(&String::from_utf8_lossy(b)),
            Err(_) => Value::Unit,
        })
    }

    pub fn exists(&self, path: &str) -> Result<Value, String> {
        self.with(|d, cwd| {
            let p = super::disk::normalize(cwd, path);
            Value::Boolean(d.durable_read(&p).is_some() || d.durable_list(&p).is_some())
        })
    }

    pub fn list(&self, dir: &str) -> Result<Value, String> {
        self.with(
            |d, cwd| match d.durable_list(&super::disk::normalize(cwd, dir)) {
                Some(names) => Value::List(Arc::new(names.iter().map(|n| s(n)).collect())),
                None => Value::Unit,
            },
        )
    }

    /// `.query(db, sql, params)`: rows of the durable image of a SQLite
    /// database, as maps; `Err` when it cannot be read.
    pub fn query(&self, db: &str, sql: &str, params: &[Value]) -> Result<Value, String> {
        let (image, path) = self.with(|d, cwd| {
            let p = super::disk::normalize(cwd, db);
            (d.durable_read(&p).is_some().then(|| d.durable_image()), p)
        })?;
        let Some(image) = image else {
            return Ok(Value::Err(Box::new(s(&format!(
                "{db}: no such database on {}'s disk (nothing durable there)",
                self.name
            )))));
        };
        let nd = Arc::new(Mutex::new(vfs::NodeDisk::new(image, 0, 0.0)));
        let key = vfs::register(&nd);
        let rows = durable_rows(key, &path, sql, params);
        vfs::unregister(key);
        // the image's syncs cost the world nothing
        vfs::take_debt_ms();
        Ok(match rows {
            Ok(rows) => Value::Ok(Box::new(rows)),
            Err(e) => Value::Err(Box::new(s(&e))),
        })
    }
}

fn durable_rows(key: u64, path: &str, sql: &str, params: &[Value]) -> Result<Value, String> {
    use rusqlite::types::{Value as Sql, ValueRef};
    let vfs_name = vfs::registered()?;
    let conn = rusqlite::Connection::open_with_flags_and_vfs(
        vfs::sqlite_path(key, path),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        vfs_name,
    )
    .map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let names: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let binds: Vec<Sql> = params
        .iter()
        .map(|p| match p {
            Value::Integer(n) => Sql::Integer(*n),
            Value::Float(f) => Sql::Real(*f),
            Value::Boolean(b) => Sql::Integer(*b as i64),
            Value::Unit => Sql::Null,
            Value::String(t) => Sql::Text(t.to_string()),
            other => Sql::Text(other.to_string()),
        })
        .collect();
    let mut rows = stmt
        .query(rusqlite::params_from_iter(binds))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut m = ValueMap::default();
        for (i, name) in names.iter().enumerate() {
            let v = match row.get_ref(i).map_err(|e| e.to_string())? {
                ValueRef::Null => Value::Unit,
                ValueRef::Integer(n) => Value::Integer(n),
                ValueRef::Real(f) => Value::Float(f),
                ValueRef::Text(t) => s(&String::from_utf8_lossy(t)),
                ValueRef::Blob(b) => crate::stdlib::bytes::to_value(b.to_vec()),
            };
            m.insert(name.clone(), v);
        }
        out.push(Value::Map(Arc::new(m)));
    }
    Ok(Value::List(Arc::new(out)))
}

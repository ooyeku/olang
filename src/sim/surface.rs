//! The `sim` module as olang sees it:
//!
//! ```text
//! let w = sim.world(#{ "seed": 7, "start": 1767225600000, "net": #{ "latency": (2, 40), "loss": 0.01 } })
//! let run = w.run_fn(main, #{ "for": sim.hours(6) })          // S1: one program
//!
//! let kv = w.node("kv", ["kv_server.ol", "7500"], #{ "env": #{ "MODE": "x" } })   // S2: nodes
//! w.client("c1", () => workload("c1"))
//! w.link("kv", "c1", #{ "latency": (5, 80), "loss": 0.02, "dup": 0.01 })
//! w.at(sim.minutes(10), () => { w.partition(["kv"], ["c1"]); time.sleep(sim.seconds(20)); w.heal() })
//! let run = w.run(sim.hours(1))
//! run.outcome  run.events  run.history  run.net  run.nodes
//! ```
//!
//! A world is a value: its configuration and the nodes added to it.
//! Running it (`run_fn`, `run`) makes a fresh world each time, so one
//! value runs the same world as often as it is asked. `w.partition`,
//! `w.heal`, `w.link` before a run set how the network starts; called
//! inside the running world (from a client or a hook) they change it
//! then. `w.crash` and `w.restart` act inside the running world only.
//!
//! S3 adds the disk and the process around a node:
//!
//! ```text
//! w.node("db", ["store.ol"], #{ "disk": sim.disk(#{ "fsync_lies": 0.01, "torn_writes": true, "full_at": 1048576, "slow": 4 }),
//!     "files": #{ "/etc/store.conf": "mode=strict" }, "cwd": "/srv", "stdin": "go\n",
//!     "crash": #{ "p": 0.001, "restart_after": 2000 } })
//! w.crash(n, #{ restart_after }) · w.kill(n, #{ restart_after }) · w.stop(n, #{ restart_after })
//! w.write_stdin(n, text) · w.close_stdin(n) · w.slow_disk(n, factor)
//! sim.maybe("after_append")            // false outside a world
//! run.faults  run.disks
//! ```

use super::net::{Link, NodeId};
use super::node::Kind;
use super::plan::Site;
use super::{Config, NodeInfo, Plan, Report, Start};
use crate::ast::{BuiltinFunction, Value, ValueMap};
use crate::interpreter::{Interpreter, InterpreterError};
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_SPEC: AtomicU64 = AtomicU64::new(1);

/// What a world value holds beyond its configuration: the nodes and how
/// the network starts.
#[derive(Clone, Default)]
struct Spec {
    nodes: Vec<(NodeInfo, Start)>,
    links: Vec<(String, String, Link)>,
    cuts: Vec<(Vec<String>, Vec<String>)>,
    hooks: usize,
}

struct WorldObject {
    cfg: Config,
    id: u64,
    spec: Mutex<Spec>,
}

impl std::fmt::Debug for WorldObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<world seed {}>", self.cfg.seed)
    }
}

impl WorldObject {
    fn spec(&self) -> std::sync::MutexGuard<'_, Spec> {
        self.spec.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl NativeObject for WorldObject {
    fn module(&self) -> &'static str {
        "sim.World"
    }
    fn type_name(&self) -> &'static str {
        "World"
    }
    fn display(&self) -> String {
        format!("<world seed {}>", self.cfg.seed)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<WorldObject>()
            .is_some_and(|o| o.id == self.id)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn field(&self, name: &str) -> Option<Result<Value, String>> {
        match name {
            "seed" => Some(Ok(Value::Integer(self.cfg.seed as i64))),
            "start" => Some(Ok(Value::Integer(self.cfg.start_ms))),
            "nodes" => Some(Ok(Value::List(Arc::new(
                self.spec()
                    .nodes
                    .iter()
                    .filter(|(i, _)| i.kind != Kind::Hook)
                    .map(|(i, _)| s(&i.name))
                    .collect(),
            )))),
            _ => None,
        }
    }
}

/// A node, as `w.node` and `w.client` answer it: its name and its URL.
#[derive(Debug)]
struct NodeObject {
    name: String,
}

impl NativeObject for NodeObject {
    fn module(&self) -> &'static str {
        "sim.Node"
    }
    fn type_name(&self) -> &'static str {
        "Node"
    }
    fn display(&self) -> String {
        format!("<node {}>", self.name)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<NodeObject>()
            .is_some_and(|o| o.name == self.name)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn field(&self, name: &str) -> Option<Result<Value, String>> {
        match name {
            "name" => Some(Ok(s(&self.name))),
            "url" => Some(Ok(s(&format!("http://{}", self.name)))),
            _ => None,
        }
    }
}

fn s(t: &str) -> Value {
    Value::String(Arc::new(t.to_string()))
}

fn builtin(name: &str, arity: usize) -> Value {
    Value::Builtin(BuiltinFunction {
        name: format!("sim.{name}"),
        arity,
    })
}

pub fn create_sim_module() -> Value {
    let mut module = ValueMap::default();
    module.insert("world".to_string(), builtin("world", 1));
    module.insert("disk".to_string(), builtin("disk", 1));
    module.insert("maybe".to_string(), builtin("maybe", 1));
    for unit in ["ms", "seconds", "minutes", "hours"] {
        module.insert(unit.to_string(), builtin(unit, 1));
    }
    // S4: properties, histories, linearizability
    for (name, arity) in [
        ("always", 2),
        ("never", 2),
        ("eventually", 3),
        ("linearizable", 3),
        ("check", 2),
        ("record", 2),
        ("linearize", 3),
        ("register", 1),
        ("kv", 0),
        ("seed", 0),
    ] {
        module.insert(name.to_string(), builtin(name, arity));
    }
    for (ty, methods) in [
        ("State", &[("disk", 2), ("history", 2), ("up", 2)][..]),
        (
            "Disk",
            &[
                ("read", 2),
                ("read_bytes", 2),
                ("exists", 2),
                ("list", 2),
                ("current", 2),
                ("query", 4),
            ][..],
        ),
        ("Run", &[("history", 2), ("check", 2), ("events", 1)][..]),
    ] {
        let mut m = ValueMap::default();
        for (name, arity) in methods {
            m.insert(name.to_string(), builtin(&format!("{ty}.{name}"), *arity));
        }
        module.insert(
            ty.to_string(),
            Value::Struct {
                type_name: "Module".into(),
                fields: Arc::new(m).into(),
            },
        );
    }
    let mut world = ValueMap::default();
    for (name, arity) in [
        ("run_fn", 3),
        ("run", 2),
        ("node", 4),
        ("client", 4),
        ("at", 3),
        ("every", 3),
        ("link", 4),
        ("partition", 3),
        ("heal", 1),
        ("crash", 3),
        ("restart", 2),
        ("kill", 3),
        ("stop", 3),
        ("write_stdin", 3),
        ("close_stdin", 2),
        ("slow_disk", 3),
    ] {
        world.insert(name.to_string(), builtin(&format!("World.{name}"), arity));
    }
    module.insert(
        "World".to_string(),
        Value::Struct {
            type_name: "Module".into(),
            fields: Arc::new(world).into(),
        },
    );
    Value::Struct {
        type_name: "Module".into(),
        fields: Arc::new(module).into(),
    }
}

fn misuse(message: impl Into<String>) -> InterpreterError {
    InterpreterError::runtime(message)
}

// ─── olang sim FILE, when FILE builds worlds ─────────────────────────

struct Collect {
    seed: u64,
    reports: Vec<Report>,
    /// S4: the plan forcing each world run of the file, by its number.
    plans: Option<Vec<super::plan::RunPlan>>,
    next_index: usize,
}

thread_local! {
    static COLLECT: RefCell<Option<Collect>> = const { RefCell::new(None) };
}

/// `olang sim` running a file that builds worlds: every `sim.world` made
/// on this thread until `collect_end` takes `seed`, and every run's
/// report is kept.
pub fn collect_begin(seed: u64) {
    collect_begin_forced(seed, None);
}

/// `collect_begin`, with each world run of the file forced by its plan
/// (`plans[i]` for the i-th run; a run past the list draws).
pub fn collect_begin_forced(seed: u64, plans: Option<Vec<super::plan::RunPlan>>) {
    COLLECT.with(|c| {
        *c.borrow_mut() = Some(Collect {
            seed,
            reports: Vec::new(),
            plans,
            next_index: 0,
        })
    });
}

/// The next world run's number and the plan forcing it, when `olang sim`
/// collects.
fn next_run() -> (usize, Option<super::plan::RunPlan>) {
    COLLECT.with(|c| match c.borrow_mut().as_mut() {
        Some(c) => {
            let i = c.next_index;
            c.next_index += 1;
            (i, c.plans.as_ref().and_then(|p| p.get(i).cloned()))
        }
        None => (0, None),
    })
}

/// The seed `olang sim` (or `olang test`) runs worlds with, if any.
pub fn collecting_seed() -> Option<u64> {
    COLLECT.with(|c| c.borrow().as_ref().map(|c| c.seed))
}

/// The reports of the worlds run since `collect_begin`.
pub fn collect_end() -> Vec<Report> {
    COLLECT.with(|c| c.borrow_mut().take().map(|c| c.reports).unwrap_or_default())
}

fn collected(report: &Report) {
    COLLECT.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.reports.push(report.clone());
        }
    });
}

/// Dispatch `sim.<name>`.
pub fn call(
    name: &str,
    args: Vec<Value>,
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    match name {
        "always" | "never" | "eventually" | "linearizable" => property(name, &args),
        "check" => check(args, interpreter),
        "record" => record(&args),
        "linearize" => linearize_call(&args, interpreter),
        "register" => {
            let mut m = ValueMap::default();
            m.insert("model".into(), s("register"));
            m.insert("init".into(), args.first().cloned().unwrap_or(Value::Unit));
            Ok(Value::Map(Arc::new(m)))
        }
        "kv" => {
            let mut m = ValueMap::default();
            m.insert("model".into(), s("kv"));
            m.insert("init".into(), Value::Map(Arc::new(ValueMap::default())));
            Ok(Value::Map(Arc::new(m)))
        }
        "seed" => Ok(match (super::current(), collecting_seed()) {
            (Some(ctx), _) => Value::Integer(ctx.world.seed as i64),
            (None, Some(seed)) => Value::Integer(seed as i64),
            (None, None) => Value::Unit,
        }),
        m if m.starts_with("State.") || m.starts_with("Disk.") || m.starts_with("Run.") => {
            view_method(m, args, interpreter)
        }
        "world" => world(&args),
        "disk" => disk_value(&args),
        "maybe" => match args.as_slice() {
            [Value::String(point)] => Ok(Value::Boolean(super::maybe(point))),
            _ => Err(misuse("sim.maybe expects the point's name as a String")),
        },
        "ms" => duration(&args, "ms", 1),
        "seconds" => duration(&args, "seconds", 1_000),
        "minutes" => duration(&args, "minutes", 60_000),
        "hours" => duration(&args, "hours", 3_600_000),
        _ => {
            let Some(method) = name.strip_prefix("World.") else {
                return Err(misuse(format!("Unknown sim function: sim.{name}")));
            };
            let mut args = args.into_iter();
            let wv = args.next().unwrap_or(Value::Unit);
            let w = match &wv {
                Value::Native(h) => h.0.as_any().downcast_ref::<WorldObject>(),
                _ => None,
            }
            .ok_or_else(|| misuse(format!("{method} is a method of a world (sim.world)")))?;
            let rest: Vec<Value> = args.collect();
            match method {
                "run_fn" => run_fn(w, &wv, rest, interpreter),
                "run" => run(w, &wv, rest, interpreter),
                "node" => node(w, rest, interpreter),
                "client" => client(w, rest, interpreter),
                "at" => hook(w, rest, false),
                "every" => hook(w, rest, true),
                "link" => link(w, rest),
                "partition" => partition(w, rest),
                "heal" => heal(w),
                "crash" => halt(w, rest, super::node::Halt::Crash),
                "kill" => halt(w, rest, super::node::Halt::Kill),
                "stop" => stop(w, rest),
                "restart" => restart(w, rest),
                "write_stdin" => stdin(w, rest, true),
                "close_stdin" => stdin(w, rest, false),
                "slow_disk" => slow_disk(w, rest),
                _ => Err(misuse(format!("Unknown sim function: sim.{name}"))),
            }
        }
    }
}

/// `sim.ms(n)`, `sim.seconds(n)`, ...: a duration as virtual ms.
fn duration(args: &[Value], name: &str, scale: i64) -> Result<Value, InterpreterError> {
    let ms = match args {
        [Value::Integer(n)] => n.checked_mul(scale),
        [Value::Float(f)] if f.is_finite() => Some((f * scale as f64).round() as i64),
        _ => None,
    };
    match ms {
        Some(ms) if ms >= 0 => Ok(Value::Integer(ms)),
        _ => Err(misuse(format!(
            "sim.{name} expects a non-negative number, got {}",
            args.first()
                .map(|a| a.type_name())
                .unwrap_or_else(|| "nothing".into())
        ))),
    }
}

fn fields_of(v: &Value) -> Option<&ValueMap> {
    match v {
        Value::Map(m) => Some(m),
        Value::Struct { fields, .. } => Some(fields),
        _ => None,
    }
}

fn int_field(opts: &ValueMap, key: &str, what: &str) -> Result<Option<i64>, InterpreterError> {
    match opts.get(key) {
        None | Some(Value::Unit) => Ok(None),
        Some(Value::Integer(n)) => Ok(Some(*n)),
        Some(other) => Err(misuse(format!(
            "{what}: `{key}` must be an Int, got {}",
            other.type_name()
        ))),
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(n) => Some(*n as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

/// `#{ latency, loss, dup }` over `base`: latency an Int (fixed) or a
/// `(least, most)` pair of ms.
fn link_of(opts: &Value, base: Link, what: &str) -> Result<Link, InterpreterError> {
    let Some(o) = fields_of(opts) else {
        return Err(misuse(format!(
            "{what}: a link is #{{ latency, loss, dup }}, got {}",
            opts.type_name()
        )));
    };
    let mut link = base;
    for (key, v) in o.iter() {
        match key.as_str() {
            "latency" => {
                link.latency = match v {
                    Value::Integer(n) if *n >= 0 => (*n as u64, *n as u64),
                    Value::Tuple(p) | Value::List(p) if p.len() == 2 => match (&p[0], &p[1]) {
                        (Value::Integer(a), Value::Integer(b)) if *a >= 0 && *b >= 0 => {
                            (*a as u64, *b as u64)
                        }
                        _ => {
                            return Err(misuse(format!(
                                "{what}: `latency` is ms, or a (least, most) pair of ms"
                            )));
                        }
                    },
                    _ => {
                        return Err(misuse(format!(
                            "{what}: `latency` is ms, or a (least, most) pair of ms"
                        )));
                    }
                }
            }
            "loss" | "dup" => {
                let p = num(v)
                    .ok_or_else(|| misuse(format!("{what}: `{key}` is a probability in 0..1")))?;
                if key == "loss" {
                    link.loss = p
                } else {
                    link.dup = p
                }
            }
            other => {
                return Err(misuse(format!(
                    "{what}: unknown option `{other}` (a link knows latency, loss and dup)"
                )));
            }
        }
    }
    link.validate()
        .map_err(|e| misuse(format!("{what}: {e}")))?;
    Ok(link)
}

/// `sim.world(#{ seed, start, max_steps, net })`.
fn world(args: &[Value]) -> Result<Value, InterpreterError> {
    let mut cfg = Config::default();
    match args {
        [] | [Value::Unit] => {}
        [opts] => {
            let Some(opts) = fields_of(opts) else {
                return Err(misuse(format!(
                    "sim.world expects #{{ seed, start, net }}, got {}",
                    opts.type_name()
                )));
            };
            for key in opts.keys() {
                if !matches!(key.as_str(), "seed" | "start" | "max_steps" | "net") {
                    return Err(misuse(format!(
                        "sim.world: unknown option `{key}` (a world knows seed, start, \
                         max_steps and net; a disk is a node's: w.node(name, argv, #{{ disk }}))"
                    )));
                }
            }
            if let Some(seed) = int_field(opts, "seed", "sim.world")? {
                cfg.seed = seed as u64;
            }
            if let Some(start) = int_field(opts, "start", "sim.world")? {
                cfg.start_ms = start;
            }
            if let Some(steps) = int_field(opts, "max_steps", "sim.world")? {
                if steps <= 0 {
                    return Err(misuse("sim.world: `max_steps` must be positive"));
                }
                cfg.max_steps = steps as u64;
            }
            if let Some(net) = opts.get("net") {
                cfg.net = link_of(net, cfg.net, "sim.world `net`")?;
            }
        }
        _ => return Err(misuse("sim.world expects one argument: #{ seed, start }")),
    }
    // `olang sim FILE --seed S` decides every world's seed.
    if let Some(seed) = COLLECT.with(|c| c.borrow().as_ref().map(|c| c.seed)) {
        cfg.seed = seed;
    }
    Ok(Value::Native(NativeHandle::new(WorldObject {
        cfg,
        id: NEXT_SPEC.fetch_add(1, Ordering::Relaxed),
        spec: Mutex::new(Spec::default()),
    })))
}

/// `#{ for }` or a bare duration: the run's deadline.
fn run_for(opts: Option<&Value>, what: &str) -> Result<Option<u64>, InterpreterError> {
    match opts {
        None | Some(Value::Unit) => Ok(None),
        Some(Value::Integer(ms)) if *ms >= 0 => Ok(Some(*ms as u64)),
        Some(Value::Integer(_)) => {
            Err(misuse(format!("{what}: the duration must be non-negative")))
        }
        Some(o) => {
            let Some(opts) = fields_of(o) else {
                return Err(misuse(format!(
                    "{what}: options are #{{ for }} (or a duration), got {}",
                    o.type_name()
                )));
            };
            for key in opts.keys() {
                if key != "for" {
                    return Err(misuse(format!(
                        "{what}: unknown option `{key}` (it knows `for`)"
                    )));
                }
            }
            match int_field(opts, "for", what)? {
                Some(ms) if ms < 0 => Err(misuse(format!("{what}: `for` must be non-negative"))),
                Some(ms) => Ok(Some(ms as u64)),
                None => Ok(None),
            }
        }
    }
}

/// `w.run_fn(f, #{ for, props })`: run `f()` as the world's main program.
fn run_fn(
    world: &WorldObject,
    wv: &Value,
    args: Vec<Value>,
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    let mut args = args.into_iter();
    let Some(f) = args.next() else {
        return Err(misuse("run_fn expects a function: w.run_fn(f, #{ for })"));
    };
    if !matches!(f, Value::Function(_) | Value::Builtin(_)) {
        return Err(misuse(format!(
            "run_fn expects a function to run as the world's main program, got {}",
            f.type_name()
        )));
    }
    let opts = args.next();
    let (run_for, props) = run_opts(opts.as_ref(), args.next().as_ref(), "run_fn")?;
    let report = run_fn_with(world, &f, run_for, props, interpreter)?;
    collected(&report);
    let rerun = Rerun {
        world: wv.clone(),
        run_for,
        main: Some(f),
    };
    Ok(report_value_with(&report, Some(rerun)))
}

fn run_fn_with(
    world: &WorldObject,
    f: &Value,
    run_for: Option<u64>,
    props: Vec<super::props::Property>,
    interpreter: &mut Interpreter,
) -> Result<Report, InterpreterError> {
    let mut cfg = world.cfg.clone();
    cfg.run_for = run_for;
    let (index, forced) = next_run();
    cfg.forced = forced;
    // Every builtin passes the one dispatch point that sees it, on either
    // tier; with worlds on the interpreter alone (`--no-ovm`), the
    // bytecode tier is set aside while the world runs.
    let kept = (!super::bytecode_tier())
        .then(|| interpreter.suspend_bytecode_tier())
        .flatten();
    let f = f.clone();
    let spec = world.spec().clone();
    let report = if spec.nodes.is_empty() {
        let template = (!props.is_empty()).then(|| interpreter.thread_safe_clone());
        super::run_main_with(cfg, NodeInfo::main(Vec::new()), props, template, || {
            let r = interpreter.call_function(f, Vec::new());
            super::sited(interpreter, r)
        })
    } else {
        // The world's nodes (and hooks) start first, then `f` runs as
        // node 0's program; the run ends when `f` does.
        let mut nodes = vec![(NodeInfo::main(Vec::new()), Start::Main)];
        nodes.extend(spec.nodes.iter().cloned());
        match plan_of(&spec, &nodes, "w.run_fn") {
            Err(e) => {
                interpreter.resume_bytecode_tier(kept);
                return Err(e);
            }
            Ok(plan) => {
                let template = interpreter.thread_safe_clone();
                super::run_world_main(cfg, nodes, plan, template, world.id, props, || {
                    let r = interpreter.call_function(f, Vec::new());
                    super::sited(interpreter, r)
                })
            }
        }
    };
    interpreter.resume_bytecode_tier(kept);
    let mut report = report.map_err(misuse)?;
    report.run_index = index;
    Ok(report)
}

/// A run's options: a duration, or `#{ for, props }`; `props` may also
/// come second (`w.run(sim.hours(1), [props])`).
fn run_opts(
    opts: Option<&Value>,
    second: Option<&Value>,
    what: &str,
) -> Result<(Option<u64>, Vec<super::props::Property>), InterpreterError> {
    let mut props = Vec::new();
    let run_for = match opts {
        // the options as a map or a record (OL-600)
        Some(o) if o.record_fields().is_some_and(|m| m.contains_key("props")) => {
            let mut rest = o.record_fields().expect("a map or a record").clone();
            if let Some(p) = rest.shift_remove("props") {
                props = props_of(&p, what)?;
            }
            let _ = o;
            run_for(Some(&Value::Map(Arc::new(rest))), what)?
        }
        Some(list @ Value::List(_)) => {
            props = props_of(list, what)?;
            None
        }
        other => run_for(other, what)?,
    };
    if let Some(p) = second {
        props.extend(props_of(p, what)?);
    }
    Ok((run_for, props))
}

/// A node's name: lowercase letters, digits and `-`, a letter first, so
/// it is a host name in a URL.
fn check_name(spec: &Spec, name: &Value, what: &str) -> Result<String, InterpreterError> {
    let Value::String(name) = name else {
        return Err(misuse(format!("{what}: a node's name is a String")));
    };
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        return Err(misuse(format!(
            "{what}: `{name}` is not a node name (lowercase letters, digits and -, a letter \
             first: it is the host in the node's URL)"
        )));
    }
    if matches!(name.as_str(), "world" | "main" | "localhost") {
        return Err(misuse(format!("{what}: `{name}` is reserved")));
    }
    if spec.nodes.iter().any(|(n, _)| &*n.name == name.as_str()) {
        return Err(misuse(format!(
            "{what}: the world has a node `{name}` already"
        )));
    }
    Ok(name.to_string())
}

fn node_value(name: &str) -> Value {
    Value::Native(NativeHandle::new(NodeObject {
        name: name.to_string(),
    }))
}

fn base_info(name: &str, kind: Kind) -> NodeInfo {
    NodeInfo {
        name: Arc::from(name),
        kind,
        ..NodeInfo::main(Vec::new())
    }
}

/// `w.node(name, argv, #{ env, skew, deny })`: an olang program as a
/// node. `argv` is the file, or `["olang", "file.ol", args...]`.
fn node(
    w: &WorldObject,
    args: Vec<Value>,
    interpreter: &Interpreter,
) -> Result<Value, InterpreterError> {
    let mut spec = w.spec();
    let what = "w.node";
    let (Some(name), Some(argv)) = (args.first(), args.get(1)) else {
        return Err(misuse(
            "w.node expects a name and a program: w.node(name, argv, opts)",
        ));
    };
    let name = check_name(&spec, name, what)?;
    let mut words: Vec<String> = match argv {
        Value::String(f) => vec![f.to_string()],
        Value::List(items) => items
            .iter()
            .map(|v| match v {
                Value::String(t) => Ok(t.to_string()),
                other => Err(misuse(format!(
                    "{what}: argv is a list of Strings, got {}",
                    other.type_name()
                ))),
            })
            .collect::<Result<_, _>>()?,
        other => {
            return Err(misuse(format!(
                "{what}: the program is a file or an argv list, got {}",
                other.type_name()
            )));
        }
    };
    if words.first().is_some_and(|w| w == "olang") {
        words.remove(0);
    }
    if words.first().is_some_and(|w| w == "run") {
        words.remove(0);
    }
    let Some(file) = words.first().cloned() else {
        return Err(misuse(format!("{what}: argv names no program")));
    };
    // Relative to the file that builds the world, else the directory
    // the run started in.
    let base = interpreter
        .current_file_for_snapshots()
        .and_then(|f| f.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    let path = {
        let p = std::path::PathBuf::from(&file);
        if p.is_absolute() { p } else { base.join(p) }
    };
    let source = std::fs::read_to_string(&path)
        .map_err(|e| misuse(format!("{what}: {}: {e}", path.display())))?;
    crate::parser::Parser::new()
        .parse(&source)
        .map_err(|e| misuse(format!("{what}: {}: {e}", path.display())))?;
    let mut info = base_info(&name, Kind::Program);
    info.argv = words;
    info.deps = package_deps(&path);
    if let Some(opts) = args.get(2) {
        node_options(&mut info, opts, what)?;
    }
    spec.nodes.push((info, Start::Program { path, source }));
    Ok(node_value(&name))
}

fn function_arg(v: Option<&Value>, what: &str) -> Result<Value, InterpreterError> {
    match v {
        Some(f @ (Value::Function(_) | Value::Builtin(_))) => Ok(f.clone()),
        Some(other) => Err(misuse(format!(
            "{what} expects a function, got {}",
            other.type_name()
        ))),
        None => Err(misuse(format!("{what} expects a function"))),
    }
}

/// `w.client(name, f, opts)`: a node that is an olang function (its
/// options are a node's: env, disk, files, cwd, stdin, crash).
fn client(
    w: &WorldObject,
    args: Vec<Value>,
    interpreter: &Interpreter,
) -> Result<Value, InterpreterError> {
    let mut spec = w.spec();
    let name = check_name(&spec, args.first().unwrap_or(&Value::Unit), "w.client")?;
    let f = function_arg(args.get(1), "w.client(name, f)")?;
    let mut info = base_info(&name, Kind::Client);
    if let Some(opts) = args.get(2) {
        node_options(&mut info, opts, "w.client")?;
    }
    let _ = interpreter;
    spec.nodes.push((info, Start::Function(f)));
    Ok(node_value(&name))
}

/// A node's options: env, skew, deny (S2); disk, files, cwd, stdin,
/// crash (S3).
fn node_options(info: &mut NodeInfo, opts: &Value, what: &str) -> Result<(), InterpreterError> {
    if matches!(opts, Value::Unit) {
        return Ok(());
    }
    let Some(o) = fields_of(opts) else {
        return Err(misuse(format!(
            "{what}: options are #{{ env, skew, deny, disk, files, cwd, stdin, crash }}"
        )));
    };
    for (key, v) in o.iter() {
        match key.as_str() {
            "env" => {
                let Some(env) = fields_of(v) else {
                    return Err(misuse(format!("{what}: `env` is a map of Strings")));
                };
                let mut out = BTreeMap::new();
                for (k, v) in env.iter() {
                    let v = match v {
                        Value::String(t) => t.to_string(),
                        other => other.to_string(),
                    };
                    out.insert(k.clone(), v);
                }
                info.env = out;
            }
            "skew" => match v {
                Value::Integer(ms) => info.skew_ms = *ms,
                _ => return Err(misuse(format!("{what}: `skew` is ms (an Int)"))),
            },
            "deny" => match v {
                Value::String(list) => {
                    info.deny = Some(crate::caps::parse_deny(list).map_err(misuse)?)
                }
                _ => return Err(misuse(format!("{what}: `deny` is a list like \"fs,proc\""))),
            },
            "disk" => info.disk = disk_opts(v, what)?,
            "files" => {
                let Some(files) = fields_of(v) else {
                    return Err(misuse(format!(
                        "{what}: `files` is a map of path to contents (a String or Bytes)"
                    )));
                };
                let mut out = Vec::new();
                for (path, contents) in files.iter() {
                    let bytes = match contents {
                        Value::String(t) => t.as_bytes().to_vec(),
                        other => crate::stdlib::bytes::bytes_of(other)
                            .map_err(|_| {
                                misuse(format!(
                                    "{what}: `files`: {path}'s contents are a String or Bytes, got {}",
                                    other.type_name()
                                ))
                            })?
                            .to_vec(),
                    };
                    out.push((path.clone(), bytes));
                }
                info.files = out;
            }
            "cwd" => match v {
                Value::String(dir) => info.cwd = super::disk::normalize("/", dir),
                _ => return Err(misuse(format!("{what}: `cwd` is a path"))),
            },
            "stdin" => match v {
                Value::String(t) => info.stdin = Some(t.to_string()),
                Value::Unit => info.stdin = None,
                _ => return Err(misuse(format!("{what}: `stdin` is a String"))),
            },
            "crash" => {
                let Some(c) = fields_of(v) else {
                    return Err(misuse(format!(
                        "{what}: `crash` is #{{ p, restart_after }}"
                    )));
                };
                let p = c.get("p").and_then(num).unwrap_or(0.0);
                if !(0.0..=1.0).contains(&p) {
                    return Err(misuse(format!(
                        "{what}: `crash.p` is a probability in 0..1"
                    )));
                }
                let restart_after = match int_field(c, "restart_after", what)? {
                    Some(ms) if ms < 0 => {
                        return Err(misuse(format!(
                            "{what}: `restart_after` must be non-negative"
                        )));
                    }
                    other => other.map(|ms| ms as u64),
                };
                for k in c.keys() {
                    if !matches!(k.as_str(), "p" | "restart_after") {
                        return Err(misuse(format!(
                            "{what}: `crash` knows p and restart_after, not `{k}`"
                        )));
                    }
                }
                info.crash = Some(super::CrashPlan { p, restart_after });
            }
            "optional" => match v {
                Value::Boolean(b) => info.optional = *b,
                _ => return Err(misuse(format!("{what}: `optional` is true or false"))),
            },
            other => {
                return Err(misuse(format!(
                    "{what}: unknown option `{other}` (a node knows env, skew, deny, disk, \
                     files, cwd, stdin, crash and optional)"
                )));
            }
        }
    }
    Ok(())
}

/// `#{ fsync_lies, torn_writes, full_at, slow }` as a disk's options.
fn disk_opts(v: &Value, what: &str) -> Result<super::disk::DiskOpts, InterpreterError> {
    let Some(o) = fields_of(v) else {
        return Err(misuse(format!(
            "{what}: a disk is sim.disk(#{{ fsync_lies, torn_writes, full_at, slow }}), got {}",
            v.type_name()
        )));
    };
    let mut d = super::disk::DiskOpts::default();
    for (key, v) in o.iter() {
        match key.as_str() {
            "fsync_lies" => {
                d.fsync_lies = num(v)
                    .ok_or_else(|| misuse(format!("{what}: `fsync_lies` is a probability")))?
            }
            "torn_writes" => match v {
                Value::Boolean(b) => d.torn_writes = *b,
                _ => return Err(misuse(format!("{what}: `torn_writes` is true or false"))),
            },
            "full_at" => match v {
                Value::Integer(n) if *n >= 0 => d.full_at = Some(*n as u64),
                Value::Unit => d.full_at = None,
                _ => return Err(misuse(format!("{what}: `full_at` is a number of bytes"))),
            },
            "slow" => {
                d.slow = num(v).ok_or_else(|| misuse(format!("{what}: `slow` is a factor")))?
            }
            other => {
                return Err(misuse(format!(
                    "{what}: a disk knows fsync_lies, torn_writes, full_at and slow, not `{other}`"
                )));
            }
        }
    }
    d.validate().map_err(|e| misuse(format!("{what}: {e}")))?;
    Ok(d)
}

/// `sim.disk(#{ fsync_lies, torn_writes, full_at, slow })`: a disk's
/// options, checked, as the map `w.node`'s `disk` takes.
fn disk_value(args: &[Value]) -> Result<Value, InterpreterError> {
    let d = match args {
        [] | [Value::Unit] => super::disk::DiskOpts::default(),
        [opts] => disk_opts(opts, "sim.disk")?,
        _ => {
            return Err(misuse(
                "sim.disk expects one argument: #{ fsync_lies, torn_writes, full_at, slow }",
            ));
        }
    };
    let mut m = ValueMap::default();
    m.insert("fsync_lies".into(), Value::Float(d.fsync_lies));
    m.insert("torn_writes".into(), Value::Boolean(d.torn_writes));
    m.insert(
        "full_at".into(),
        d.full_at
            .map(|n| Value::Integer(n as i64))
            .unwrap_or(Value::Unit),
    );
    m.insert("slow".into(), Value::Float(d.slow));
    Ok(Value::Map(Arc::new(m)))
}

/// A node program's package dependencies, resolved as `olang run`
/// resolves them (the manifest above the file, its lock replayed) but
/// fetching nothing and writing no lock: a world reads the machine's
/// packages, never changes them.
pub(crate) fn package_deps(
    path: &std::path::Path,
) -> Option<std::collections::HashMap<String, std::path::PathBuf>> {
    let root = crate::pkg::manifest::Manifest::find_root(path)?;
    let opts = crate::pkg::InstallOptions {
        offline: true,
        ..Default::default()
    };
    let (map, _missing) = crate::pkg::install_lenient(&root, &opts);
    let mut map: std::collections::HashMap<_, _> = map.into_iter().collect();
    if let Ok(m) = crate::pkg::manifest::Manifest::load(&root) {
        map.entry(m.package.name.clone()).or_insert(root.clone());
    }
    Some(map)
}

/// `w.at(t, f)` / `w.every(dt, f)`: faults as code, on virtual time.
fn hook(w: &WorldObject, args: Vec<Value>, every: bool) -> Result<Value, InterpreterError> {
    let what = if every {
        "w.every(dt, f)"
    } else {
        "w.at(t, f)"
    };
    let t = match args.first() {
        Some(Value::Integer(t)) if *t >= 0 => *t as u64,
        _ => {
            return Err(misuse(format!(
                "{what}: the time is virtual ms (sim.minutes(10))"
            )));
        }
    };
    if every && t == 0 {
        return Err(misuse("w.every: the interval must be positive"));
    }
    let f = function_arg(args.get(1), what)?;
    let mut spec = w.spec();
    spec.hooks += 1;
    let name = format!("hook-{}", spec.hooks);
    spec.nodes.push((
        base_info(&name, Kind::Hook),
        Start::Hook {
            at: t,
            every: every.then_some(t),
            f,
        },
    ));
    Ok(Value::Unit)
}

fn node_name(v: &Value, what: &str) -> Result<String, InterpreterError> {
    match v {
        Value::String(t) => Ok(t.to_string()),
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<NodeObject>()
                .map(|n| n.name.clone())
                .ok_or_else(|| misuse(format!("{what}: expected a node or its name")))
        }
        other => Err(misuse(format!(
            "{what}: expected a node or its name, got {}",
            other.type_name()
        ))),
    }
}

fn names(v: Option<&Value>, what: &str) -> Result<Vec<String>, InterpreterError> {
    match v {
        Some(Value::List(items)) => items.iter().map(|i| node_name(i, what)).collect(),
        Some(one) => Ok(vec![node_name(one, what)?]),
        None => Err(misuse(format!("{what}: expected a list of nodes"))),
    }
}

/// The running world this value made, when called from inside it.
fn live(w: &WorldObject, what: &str) -> Result<Option<super::Ctx>, InterpreterError> {
    match super::current() {
        None => Ok(None),
        Some(ctx) if ctx.world.spec == w.id => Ok(Some(ctx)),
        Some(_) => Err(misuse(format!(
            "{what}: this world is not the one running here"
        ))),
    }
}

fn resolve(ctx: &super::Ctx, name: &str, what: &str) -> Result<NodeId, InterpreterError> {
    ctx.world
        .node_named(name)
        .ok_or_else(|| misuse(format!("{what}: the world has no node `{name}`")))
}

/// `w.link(a, b, #{ latency, loss, dup })`.
fn link(w: &WorldObject, args: Vec<Value>) -> Result<Value, InterpreterError> {
    let what = "w.link";
    let (Some(a), Some(b), Some(opts)) = (args.first(), args.get(1), args.get(2)) else {
        return Err(misuse(
            "w.link expects two nodes and #{ latency, loss, dup }",
        ));
    };
    let (a, b) = (node_name(a, what)?, node_name(b, what)?);
    let link = link_of(opts, w.cfg.net, what)?;
    match live(w, what)? {
        Some(ctx) => {
            let (x, y) = (resolve(&ctx, &a, what)?, resolve(&ctx, &b, what)?);
            ctx.world.set_link(ctx.task, x, y, link);
        }
        None => w.spec().links.push((a, b, link)),
    }
    Ok(Value::Unit)
}

/// `w.partition([a], [b])`: cut every pair across the two groups.
fn partition(w: &WorldObject, args: Vec<Value>) -> Result<Value, InterpreterError> {
    let what = "w.partition";
    let left = names(args.first(), what)?;
    let right = names(args.get(1), what)?;
    match live(w, what)? {
        Some(ctx) => {
            let l = left
                .iter()
                .map(|n| resolve(&ctx, n, what))
                .collect::<Result<Vec<_>, _>>()?;
            let r = right
                .iter()
                .map(|n| resolve(&ctx, n, what))
                .collect::<Result<Vec<_>, _>>()?;
            let place = l.first().copied().unwrap_or(0);
            if let Some(key) = forced_call(&ctx, Site::Partition, place, None, what) {
                ctx.world.set_partition(ctx.task, &l, &r, Some(key));
            }
        }
        None => w.spec().cuts.push((left, right)),
    }
    Ok(Value::Unit)
}

/// `w.heal()`: every partition is gone.
fn heal(w: &WorldObject) -> Result<Value, InterpreterError> {
    match live(w, "w.heal")? {
        Some(ctx) => ctx.world.set_heal(ctx.task),
        None => w.spec().cuts.clear(),
    }
    Ok(Value::Unit)
}

fn restart_after(opts: Option<&Value>, what: &str) -> Result<Option<u64>, InterpreterError> {
    match opts {
        None | Some(Value::Unit) => Ok(None),
        Some(o) => {
            let Some(o) = fields_of(o) else {
                return Err(misuse(format!("{what}: options are #{{ restart_after }}")));
            };
            match int_field(o, "restart_after", what)? {
                Some(ms) if ms >= 0 => Ok(Some(ms as u64)),
                Some(_) => Err(misuse(format!(
                    "{what}: `restart_after` must be non-negative"
                ))),
                None => Ok(None),
            }
        }
    }
}

/// A fault called in code: its key when it happens, `None` when a forced
/// plan leaves it out (then the call does nothing, and says so).
fn forced_call(
    ctx: &super::Ctx,
    site: Site,
    node: NodeId,
    delay: Option<u64>,
    what: &str,
) -> Option<super::plan::Key> {
    let d = ctx.world.code_fault(site, node, delay);
    if d.fire {
        Some(d.key)
    } else {
        let name = ctx.world.nodes[node].name.clone();
        ctx.world.note(
            ctx.task,
            "skipped",
            format!("{what}({name}): left out by the plan"),
        );
        None
    }
}

/// A crash, kill or stop aimed at a node that is down already (crashed
/// and not back yet, or ended) does nothing, as pulling the plug of a
/// machine that is off does nothing: the call is an event (`ignored`),
/// not a fault, and it visits no fault site, so a plan neither records
/// nor needs it. A hook that crashes a server every few minutes can
/// fall while the last crash's restart is still pending.
fn already_down(ctx: &super::Ctx, n: NodeId, what: &str) -> bool {
    if n == 0 || ctx.world.node_up(n) {
        return false;
    }
    let name = ctx.world.nodes[n].name.clone();
    ctx.world.note(
        ctx.task,
        "ignored",
        format!("{what}({name}): {name} is down already"),
    );
    true
}

/// `w.crash(n, #{ restart_after })` / `w.kill(n, ...)`, inside the
/// running world.
fn halt(
    w: &WorldObject,
    args: Vec<Value>,
    how: super::node::Halt,
) -> Result<Value, InterpreterError> {
    let what = match how {
        super::node::Halt::Crash => "w.crash",
        _ => "w.kill",
    };
    let Some(ctx) = live(w, what)? else {
        return Err(misuse(format!(
            "{what} acts on the running world: call it from a client or a hook (w.at)"
        )));
    };
    let name = node_name(args.first().unwrap_or(&Value::Unit), what)?;
    let n = resolve(&ctx, &name, what)?;
    let after = restart_after(args.get(1), what)?;
    let site = match how {
        super::node::Halt::Crash => Site::Crash,
        _ => Site::Kill,
    };
    if already_down(&ctx, n, what) {
        return Ok(Value::Unit);
    }
    let Some(key) = forced_call(&ctx, site, n, after, what) else {
        return Ok(Value::Unit);
    };
    ctx.world
        .halt(ctx.task, n, how, key.delay, what, Some(key))
        .map_err(misuse)?;
    // A node that stops itself stops here.
    super::World::stopped(&ctx.world.lock(), ctx.task)?;
    Ok(Value::Unit)
}

/// `w.stop(n, #{ restart_after })`: SIGTERM, inside the running world.
fn stop(w: &WorldObject, args: Vec<Value>) -> Result<Value, InterpreterError> {
    let what = "w.stop";
    let Some(ctx) = live(w, what)? else {
        return Err(misuse(
            "w.stop acts on the running world: call it from a client or a hook",
        ));
    };
    let name = node_name(args.first().unwrap_or(&Value::Unit), what)?;
    let n = resolve(&ctx, &name, what)?;
    let after = restart_after(args.get(1), what)?;
    if already_down(&ctx, n, what) {
        return Ok(Value::Unit);
    }
    let Some(key) = forced_call(&ctx, Site::Stop, n, after, what) else {
        return Ok(Value::Unit);
    };
    ctx.world
        .stop(ctx.task, n, key.delay, Some(key))
        .map_err(misuse)?;
    super::World::stopped(&ctx.world.lock(), ctx.task)?;
    Ok(Value::Unit)
}

/// `w.write_stdin(n, text)` / `w.close_stdin(n)`.
fn stdin(w: &WorldObject, args: Vec<Value>, write: bool) -> Result<Value, InterpreterError> {
    let what = if write {
        "w.write_stdin"
    } else {
        "w.close_stdin"
    };
    let Some(ctx) = live(w, what)? else {
        return Err(misuse(format!(
            "{what} acts on the running world: call it from a client or a hook (a node's \
             first stdin is #{{ stdin }})"
        )));
    };
    let name = node_name(args.first().unwrap_or(&Value::Unit), what)?;
    let n = resolve(&ctx, &name, what)?;
    let text = if write {
        match args.get(1) {
            Some(Value::String(t)) => Some(t.to_string()),
            _ => return Err(misuse("w.write_stdin expects a node and a String")),
        }
    } else {
        None
    };
    ctx.world.feed_stdin(ctx.task, n, text.as_deref());
    Ok(Value::Unit)
}

/// `w.slow_disk(n, factor)`: from now on node n's syncs take `factor`
/// times as long.
fn slow_disk(w: &WorldObject, args: Vec<Value>) -> Result<Value, InterpreterError> {
    let what = "w.slow_disk";
    let Some(ctx) = live(w, what)? else {
        return Err(misuse(
            "w.slow_disk acts on the running world (or give the node #{ disk: sim.disk(#{ slow }) })",
        ));
    };
    let name = node_name(args.first().unwrap_or(&Value::Unit), what)?;
    let n = resolve(&ctx, &name, what)?;
    let factor = args
        .get(1)
        .and_then(num)
        .filter(|f| f.is_finite() && *f >= 0.0)
        .ok_or_else(|| misuse("w.slow_disk expects a node and a non-negative factor"))?;
    ctx.world.set_slow(ctx.task, n, factor);
    Ok(Value::Unit)
}

/// `w.restart(n)`, inside the running world.
fn restart(w: &WorldObject, args: Vec<Value>) -> Result<Value, InterpreterError> {
    let what = "w.restart";
    let Some(ctx) = live(w, what)? else {
        return Err(misuse(
            "w.restart acts on the running world: call it from a client or a hook",
        ));
    };
    let name = node_name(args.first().unwrap_or(&Value::Unit), what)?;
    let n = resolve(&ctx, &name, what)?;
    ctx.world.restart(ctx.task, n).map_err(misuse)?;
    Ok(Value::Unit)
}

/// `w.run(duration)` / `w.run(#{ for, props })`: run the world's nodes,
/// stopping at the first property that breaks.
fn run(
    w: &WorldObject,
    wv: &Value,
    args: Vec<Value>,
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    let (run_for, props) = run_opts(args.first(), args.get(1), "w.run")?;
    let report = run_nodes(w, run_for, props, interpreter)?;
    collected(&report);
    let rerun = Rerun {
        world: wv.clone(),
        run_for,
        main: None,
    };
    Ok(report_value_with(&report, Some(rerun)))
}

fn run_nodes(
    w: &WorldObject,
    run_for: Option<u64>,
    props: Vec<super::props::Property>,
    interpreter: &mut Interpreter,
) -> Result<Report, InterpreterError> {
    let what = "w.run";
    let mut cfg = w.cfg.clone();
    cfg.run_for = run_for;
    let spec = w.spec().clone();
    if spec.nodes.iter().all(|(n, _)| n.kind == Kind::Hook) {
        return Err(misuse(
            "w.run: the world has no nodes (add them with w.node and w.client), or use w.run_fn",
        ));
    }
    let mut nodes = vec![(NodeInfo::world(), Start::Main)];
    nodes.extend(spec.nodes.iter().cloned());
    let plan = plan_of(&spec, &nodes, what)?;
    let (index, forced) = next_run();
    cfg.forced = forced;
    let kept = (!super::bytecode_tier())
        .then(|| interpreter.suspend_bytecode_tier())
        .flatten();
    let template = interpreter.thread_safe_clone();
    let report = super::run_world(cfg, nodes, plan, template, w.id, props);
    interpreter.resume_bytecode_tier(kept);
    let mut report = report.map_err(misuse)?;
    report.run_index = index;
    Ok(report)
}

/// The network a run starts with: the spec's links and cuts by node id
/// (node 0 is the world, or `run_fn`'s main program).
fn plan_of(spec: &Spec, nodes: &[(NodeInfo, Start)], what: &str) -> Result<Plan, InterpreterError> {
    let id = |name: &str| {
        nodes
            .iter()
            .position(|(n, _)| &*n.name == name && n.kind != Kind::Hook)
            // `run_fn`'s main program is the node `main`; the world is no node
            .filter(|i| *i > 0 || name == "main")
            .ok_or_else(|| misuse(format!("{what}: the world has no node `{name}`")))
    };
    let mut plan = Plan::default();
    for (a, b, link) in &spec.links {
        plan.links.push((id(a)?, id(b)?, *link));
    }
    for (l, r) in &spec.cuts {
        plan.cuts.push((
            l.iter().map(|n| id(n)).collect::<Result<_, _>>()?,
            r.iter().map(|n| id(n)).collect::<Result<_, _>>()?,
        ));
    }
    Ok(plan)
}

/// A report as olang data.
pub fn report_value(r: &Report) -> Value {
    report_value_with(r, None)
}

/// A report as olang data, with what it takes to run it again
/// (`sim.check`).
fn report_value_with(r: &Report, rerun: Option<Rerun>) -> Value {
    let mut m = ValueMap::default();
    m.insert(
        "handle".to_string(),
        Value::Native(NativeHandle::new(RunObject {
            report: Arc::new(r.clone()),
            rerun,
        })),
    );
    m.insert(
        "violation".to_string(),
        match &r.violation {
            None => Value::Unit,
            Some(v) => {
                let mut e = ValueMap::default();
                e.insert("property".into(), s(&v.property));
                e.insert("kind".into(), s(v.kind));
                e.insert("t".into(), Value::Integer(v.t as i64));
                e.insert("step".into(), Value::Integer(v.step as i64));
                e.insert("detail".into(), s(&v.detail));
                Value::Map(Arc::new(e))
            }
        },
    );
    m.insert(
        "preemptions".to_string(),
        Value::Integer(r.preemptions as i64),
    );
    let mut histories = ValueMap::default();
    for (name, h) in &r.histories {
        histories.insert(name.clone(), Value::List(h.clone()));
    }
    m.insert("histories".to_string(), Value::Map(Arc::new(histories)));
    m.insert("seed".to_string(), Value::Integer(r.seed as i64));
    m.insert("outcome".to_string(), s(r.outcome));
    m.insert("value".to_string(), r.value.clone().unwrap_or(Value::Unit));
    m.insert(
        "error".to_string(),
        r.error.as_deref().map(s).unwrap_or(Value::Unit),
    );
    m.insert("now".to_string(), Value::Integer(r.now as i64));
    m.insert("steps".to_string(), Value::Integer(r.steps as i64));
    m.insert("tasks".to_string(), Value::Integer(r.tasks as i64));
    m.insert(
        "failed_tasks".to_string(),
        Value::Integer(r.failed_tasks as i64),
    );
    let events = r
        .events
        .iter()
        .map(|e| {
            let mut ev = ValueMap::default();
            ev.insert("t".to_string(), Value::Integer(e.t as i64));
            ev.insert("node".to_string(), s(&e.node));
            ev.insert(
                "task".to_string(),
                e.task
                    .map(|t| Value::Integer(t as i64))
                    .unwrap_or(Value::Unit),
            );
            ev.insert("kind".to_string(), s(e.kind));
            ev.insert("detail".to_string(), s(&e.detail));
            Value::Map(Arc::new(ev))
        })
        .collect::<Vec<_>>();
    m.insert("events".to_string(), Value::List(Arc::new(events)));
    m.insert(
        "nodes".to_string(),
        Value::List(Arc::new(r.nodes.iter().map(|n| s(n)).collect())),
    );
    let history = r
        .history
        .iter()
        .map(|h| {
            let mut e = ValueMap::default();
            let opt_int = |v: Option<i64>| v.map(Value::Integer).unwrap_or(Value::Unit);
            e.insert("id".to_string(), Value::Integer(h.id as i64));
            e.insert("node".to_string(), s(&h.node));
            e.insert("method".to_string(), s(&h.method));
            e.insert("url".to_string(), s(&h.url));
            e.insert("body".to_string(), s(&h.body));
            e.insert("sent".to_string(), Value::Integer(h.sent as i64));
            e.insert("done".to_string(), opt_int(h.done.map(|d| d as i64)));
            e.insert("outcome".to_string(), s(h.outcome));
            e.insert("status".to_string(), opt_int(h.status));
            e.insert(
                "response".to_string(),
                h.response.as_deref().map(s).unwrap_or(Value::Unit),
            );
            Value::Map(Arc::new(e))
        })
        .collect::<Vec<_>>();
    m.insert("history".to_string(), Value::List(Arc::new(history)));
    let mut net = ValueMap::default();
    for (k, v) in [
        ("sent", r.net.sent),
        ("delivered", r.net.delivered),
        ("lost", r.net.lost),
        ("cut", r.net.cut),
        ("duplicated", r.net.duplicated),
        ("late", r.net.late),
        ("refused", r.net.refused),
        ("timeouts", r.net.timeouts),
    ] {
        net.insert(k.to_string(), Value::Integer(v as i64));
    }
    m.insert("net".to_string(), Value::Map(Arc::new(net)));
    let faults = r
        .faults
        .iter()
        .map(|f| {
            let mut e = ValueMap::default();
            e.insert("t".to_string(), Value::Integer(f.t as i64));
            e.insert("node".to_string(), s(&f.node));
            e.insert("kind".to_string(), s(f.kind));
            e.insert("detail".to_string(), s(&f.detail));
            e.insert("step".to_string(), Value::Integer(f.step as i64));
            Value::Map(Arc::new(e))
        })
        .collect::<Vec<_>>();
    m.insert("faults".to_string(), Value::List(Arc::new(faults)));
    let mut disks = ValueMap::default();
    for (name, d) in &r.disks {
        let mut e = ValueMap::default();
        for (k, v) in [
            ("writes", d.writes),
            ("syncs", d.syncs),
            ("lies", d.lies),
            ("lost", d.lost),
            ("torn", d.torn),
            ("full", d.full),
            ("crashes", d.crashes),
        ] {
            e.insert(k.to_string(), Value::Integer(v as i64));
        }
        disks.insert(name.clone(), Value::Map(Arc::new(e)));
    }
    m.insert("disks".to_string(), Value::Map(Arc::new(disks)));
    Value::Map(Arc::new(m))
}

// ─── S4: properties, histories, checks ───────────────────────────────

/// What it takes to run a world again, as `sim.check` does: the world
/// value, its run's duration, and the main function of a `run_fn`.
#[derive(Debug, Clone)]
struct Rerun {
    world: Value,
    run_for: Option<u64>,
    main: Option<Value>,
}

/// The handle a run's map carries (`run.history(name)`, `run.check`).
#[derive(Debug)]
struct RunObject {
    report: Arc<Report>,
    rerun: Option<Rerun>,
}

impl NativeObject for RunObject {
    fn module(&self) -> &'static str {
        "sim.Run"
    }
    fn type_name(&self) -> &'static str {
        "Run"
    }
    fn display(&self) -> String {
        format!("<run seed {} {}>", self.report.seed, self.report.outcome)
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<RunObject>()
            .is_some_and(|o| Arc::ptr_eq(&o.report, &self.report))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The run behind a run's map (or its handle).
fn run_object(v: &Value) -> Option<&RunObject> {
    let handle = match v {
        Value::Map(m) => m.get("handle")?,
        other => other,
    };
    match handle {
        Value::Native(h) => h.0.as_any().downcast_ref::<RunObject>(),
        _ => None,
    }
}

fn text_arg<'a>(v: Option<&'a Value>, what: &str) -> Result<&'a str, InterpreterError> {
    match v {
        Some(Value::String(t)) => Ok(t.as_str()),
        Some(other) => Err(misuse(format!(
            "{what} expects a String, got {}",
            other.type_name()
        ))),
        None => Err(misuse(format!("{what} expects a String"))),
    }
}

/// `sim.always(name, pred)`, `sim.never(name, pred)`,
/// `sim.eventually(name, within, pred)`,
/// `sim.linearizable(name, history, model)`.
fn property(kind: &str, args: &[Value]) -> Result<Value, InterpreterError> {
    use super::props::{Hist, Kind, Property, PropertyObject};
    let what = format!("sim.{kind}");
    let name = text_arg(args.first(), &format!("{what}: the property's name"))?.to_string();
    let p = match kind {
        "always" | "never" => Property {
            name,
            kind: if kind == "always" {
                Kind::Always
            } else {
                Kind::Never
            },
            pred: function_arg(args.get(1), &format!("{what}(name, pred)"))?,
        },
        "eventually" => {
            let within = match args.get(1) {
                Some(Value::Integer(ms)) if *ms >= 0 => *ms as u64,
                _ => {
                    return Err(misuse(
                        "sim.eventually(name, within, pred): `within` is virtual ms (sim.seconds(30))",
                    ));
                }
            };
            Property {
                name,
                kind: Kind::Eventually(within),
                pred: function_arg(args.get(2), "sim.eventually(name, within, pred)")?,
            }
        }
        _ => {
            let history = match args.get(1) {
                Some(Value::String(h)) => Hist::Named(h.to_string()),
                Some(Value::List(h)) => Hist::Given(h.clone()),
                _ => {
                    return Err(misuse(
                        "sim.linearizable(name, history, model): the history is a recorded \
                         history's name (sim.record) or a list of its entries",
                    ));
                }
            };
            let model = args.get(2).cloned().ok_or_else(|| {
                misuse("sim.linearizable(name, history, model): a model (sim.register(0), sim.kv(), #{ init, step })")
            })?;
            Property {
                name,
                kind: Kind::Linearizable { history, model },
                pred: Value::Unit,
            }
        }
    };
    Ok(Value::Native(NativeHandle::new(PropertyObject(p))))
}

/// A list of properties (or one).
fn props_of(v: &Value, what: &str) -> Result<Vec<super::props::Property>, InterpreterError> {
    let one = |v: &Value| -> Result<super::props::Property, InterpreterError> {
        match v {
            Value::Native(h) => {
                h.0.as_any()
                    .downcast_ref::<super::props::PropertyObject>()
                    .map(|p| p.0.clone())
                    .ok_or_else(|| {
                        misuse(format!(
                            "{what}: `props` holds properties (sim.always, ...)"
                        ))
                    })
            }
            other => Err(misuse(format!(
                "{what}: `props` holds properties (sim.always, sim.never, sim.eventually, \
                 sim.linearizable), got {}",
                other.type_name()
            ))),
        }
    };
    match v {
        Value::List(items) => items.iter().map(one).collect(),
        Value::Unit => Ok(Vec::new()),
        other => Ok(vec![one(other)?]),
    }
}

/// `sim.check(run, props)`: do `props` hold of `run`? Linearizability is
/// checked over the run's recorded histories; any other property runs
/// the world again with the properties attached (one seed is one run, so
/// it is the same run, now watched) and stops at the first violation.
/// Raises naming the property, the virtual time and the step; answers
/// the run (the watched one, when it ran again).
fn check(args: Vec<Value>, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
    let what = "sim.check(run, props)";
    let run = args.first().and_then(run_object).ok_or_else(|| {
        misuse(format!(
            "{what}: the first argument is a run (w.run, w.run_fn)"
        ))
    })?;
    let props = props_of(args.get(1).unwrap_or(&Value::Unit), what)?;
    let report: Report = if props.iter().all(|p| p.at_end_only()) {
        let mut r = (*run.report).clone();
        for p in &props {
            let super::props::Kind::Linearizable { history, model } = &p.kind else {
                continue;
            };
            let entries = match history {
                super::props::Hist::Given(h) => h.clone(),
                super::props::Hist::Named(n) => r.histories.get(n).cloned().unwrap_or_default(),
            };
            let verdict =
                super::props::linearize(interpreter, &entries, model, super::props::LIN_MAX_STEPS)
                    .map_err(|e| misuse(format!("{what}: \"{}\": {e}", p.name)))?;
            if let super::lin::Verdict::Violation(why) = verdict {
                r.violation = Some(super::props::Violation {
                    property: p.name.clone(),
                    kind: "linearizable",
                    t: r.now,
                    step: r.steps,
                    detail: why,
                });
                r.outcome = "violated";
                r.error = r.violation.as_ref().map(|v| v.describe());
                collected(&r);
                break;
            }
        }
        r
    } else {
        let Some(rerun) = &run.rerun else {
            return Err(misuse(format!(
                "{what}: this run cannot be run again (it came from another check)"
            )));
        };
        let w = match &rerun.world {
            Value::Native(h) => h.0.as_any().downcast_ref::<WorldObject>(),
            _ => None,
        }
        .ok_or_else(|| misuse(format!("{what}: the run's world is gone")))?;
        let r = match &rerun.main {
            Some(f) => run_fn_with(w, f, rerun.run_for, props, interpreter)?,
            None => run_nodes(w, rerun.run_for, props, interpreter)?,
        };
        collected(&r);
        r
    };
    if let Some(v) = &report.violation {
        return Err(misuse(format!("sim.check: property {}", v.describe())));
    }
    Ok(report_value(&report))
}

/// `sim.record(name, op)`: add an operation to the history `name`
/// (nothing outside a world).
fn record(args: &[Value]) -> Result<Value, InterpreterError> {
    let name = text_arg(args.first(), "sim.record(name, op): the history's name")?;
    let rec = args.get(1).map(|x| x.as_options());
    let op = match rec.as_deref() {
        Some(Value::Map(m)) => m,
        _ => {
            return Err(misuse(
                "sim.record(name, op): op is #{ type: invoke|ok|fail|info, f, value }",
            ));
        }
    };
    if let Some(ctx) = super::current() {
        ctx.world.record(ctx.task, name, op).map_err(misuse)?;
    }
    Ok(Value::Unit)
}

/// `sim.linearize(history, model, #{ max_steps })`: the checker on its own.
fn linearize_call(
    args: &[Value],
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    let what = "sim.linearize(history, model)";
    let history = match args.first() {
        Some(Value::List(h)) => h.clone(),
        Some(v) if run_object(v).is_some() => {
            return Err(misuse(format!(
                "{what}: give run.history(name), not the run"
            )));
        }
        _ => return Err(misuse(format!("{what}: the history is a list of entries"))),
    };
    let model = args.get(1).ok_or_else(|| {
        misuse(format!(
            "{what}: a model (sim.register(0), sim.kv(), #{{ init, step }})"
        ))
    })?;
    let max = match args.get(2).and_then(fields_of) {
        Some(o) => int_field(o, "max_steps", what)?
            .map(|n| n.max(1) as u64)
            .unwrap_or(super::props::LIN_MAX_STEPS),
        None => super::props::LIN_MAX_STEPS,
    };
    let verdict = super::props::linearize(interpreter, &history, model, max)
        .map_err(|e| misuse(format!("{what}: {e}")))?;
    Ok(super::props::verdict_value(&verdict))
}

/// `s.disk(n)`, `s.history(name)`, `s.up(n)`; a disk view's reads;
/// `run.history(name)`, `run.check(props)`.
fn view_method(
    name: &str,
    args: Vec<Value>,
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    let (ty, method) = name.split_once('.').unwrap_or((name, ""));
    let recv = args.first().cloned().unwrap_or(Value::Unit);
    let arg = |i: usize, what: &str| {
        text_arg(args.get(i), &format!("{name}: {what}")).map(str::to_string)
    };
    match ty {
        "State" => {
            let st = match &recv {
                Value::Native(h) => h.0.as_any().downcast_ref::<super::props::StateObject>(),
                _ => None,
            }
            .ok_or_else(|| {
                misuse(format!(
                    "{method} is a method of a world's state (a predicate's s)"
                ))
            })?;
            let a = arg(1, "a name")?;
            match method {
                "disk" => st.disk(&a),
                "history" => st.history(&a),
                "up" => st.up(&a),
                _ => Err(format!("unknown method sim.{name}")),
            }
            .map_err(misuse)
        }
        "Disk" => {
            let d = match &recv {
                Value::Native(h) => h.0.as_any().downcast_ref::<super::props::DiskObject>(),
                _ => None,
            }
            .ok_or_else(|| {
                misuse(format!(
                    "{method} is a method of a disk view (s.disk(node))"
                ))
            })?;
            let a = arg(1, "a path")?;
            match method {
                "read" => d.read(&a, false),
                "read_bytes" => d.read(&a, true),
                "exists" => d.exists(&a),
                "list" => d.list(&a),
                "current" => d.current(&a),
                "query" => {
                    let sql = arg(2, "the SQL")?;
                    let params = match args.get(3) {
                        Some(Value::List(p)) => p.to_vec(),
                        _ => Vec::new(),
                    };
                    d.query(&a, &sql, &params)
                }
                _ => Err(format!("unknown method sim.{name}")),
            }
            .map_err(misuse)
        }
        _ => {
            let run = run_object(&recv)
                .ok_or_else(|| misuse(format!("{method} is a method of a run (w.run)")))?;
            match method {
                "history" => {
                    let a = arg(1, "the history's name")?;
                    Ok(Value::List(
                        run.report.histories.get(&a).cloned().unwrap_or_default(),
                    ))
                }
                "check" => check(
                    vec![recv.clone(), args.get(1).cloned().unwrap_or(Value::Unit)],
                    interpreter,
                ),
                "events" => super::view::events_frame(&run.report).map_err(misuse),
                _ => Err(misuse(format!("unknown method sim.{name}"))),
            }
        }
    }
}

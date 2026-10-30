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

use super::net::{Link, NodeId};
use super::node::Kind;
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
    for unit in ["ms", "seconds", "minutes", "hours"] {
        module.insert(unit.to_string(), builtin(unit, 1));
    }
    let mut world = ValueMap::default();
    for (name, arity) in [
        ("run_fn", 3),
        ("run", 2),
        ("node", 4),
        ("client", 3),
        ("at", 3),
        ("every", 3),
        ("link", 4),
        ("partition", 3),
        ("heal", 1),
        ("crash", 3),
        ("restart", 2),
    ] {
        world.insert(name.to_string(), builtin(&format!("World.{name}"), arity));
    }
    module.insert(
        "World".to_string(),
        Value::Struct {
            type_name: "Module".to_string(),
            fields: Arc::new(world),
        },
    );
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

fn misuse(message: impl Into<String>) -> InterpreterError {
    InterpreterError::runtime(message)
}

// ─── olang sim FILE, when FILE builds worlds ─────────────────────────

struct Collect {
    seed: u64,
    reports: Vec<Report>,
}

thread_local! {
    static COLLECT: RefCell<Option<Collect>> = const { RefCell::new(None) };
}

/// `olang sim` running a file that builds worlds: every `sim.world` made
/// on this thread until `collect_end` takes `seed`, and every run's
/// report is kept.
pub fn collect_begin(seed: u64) {
    COLLECT.with(|c| {
        *c.borrow_mut() = Some(Collect {
            seed,
            reports: Vec::new(),
        })
    });
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
        "world" => world(&args),
        "ms" => duration(&args, "ms", 1),
        "seconds" => duration(&args, "seconds", 1_000),
        "minutes" => duration(&args, "minutes", 60_000),
        "hours" => duration(&args, "hours", 3_600_000),
        _ => {
            let Some(method) = name.strip_prefix("World.") else {
                return Err(misuse(format!("Unknown sim function: sim.{name}")));
            };
            let mut args = args.into_iter();
            let w = args.next();
            let w = match &w {
                Some(Value::Native(h)) => h.0.as_any().downcast_ref::<WorldObject>(),
                _ => None,
            }
            .ok_or_else(|| misuse(format!("{method} is a method of a world (sim.world)")))?;
            let rest: Vec<Value> = args.collect();
            match method {
                "run_fn" => run_fn(w, rest, interpreter),
                "run" => run(w, rest, interpreter),
                "node" => node(w, rest, interpreter),
                "client" => client(w, rest),
                "at" => hook(w, rest, false),
                "every" => hook(w, rest, true),
                "link" => link(w, rest),
                "partition" => partition(w, rest),
                "heal" => heal(w),
                "crash" => crash(w, rest),
                "restart" => restart(w, rest),
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
                         max_steps and net; disks arrive in S3)"
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

/// `w.run_fn(f, #{ for })`: run `f()` as the world's main program.
fn run_fn(
    world: &WorldObject,
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
    let mut cfg = world.cfg.clone();
    cfg.run_for = run_for(args.next().as_ref(), "run_fn")?;
    // The world runs on the interpreter tier: every builtin through the
    // one dispatch point that sees it (as record and replay run).
    let kept = interpreter.suspend_bytecode_tier();
    let report = super::run(cfg, || interpreter.call_function(f, Vec::new()));
    interpreter.resume_bytecode_tier(kept);
    let report = report.map_err(misuse)?;
    collected(&report);
    Ok(report_value(&report))
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
    if let Some(opts) = args.get(2).filter(|o| !matches!(o, Value::Unit)) {
        let Some(o) = fields_of(opts) else {
            return Err(misuse(format!(
                "{what}: options are #{{ env, skew, deny }}"
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
                other => {
                    return Err(misuse(format!(
                        "{what}: unknown option `{other}` (a node knows env, skew and deny; \
                         disks arrive in S3)"
                    )));
                }
            }
        }
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

/// `w.client(name, f)`: a node that is an olang function.
fn client(w: &WorldObject, args: Vec<Value>) -> Result<Value, InterpreterError> {
    let mut spec = w.spec();
    let name = check_name(&spec, args.first().unwrap_or(&Value::Unit), "w.client")?;
    let f = function_arg(args.get(1), "w.client(name, f)")?;
    spec.nodes
        .push((base_info(&name, Kind::Client), Start::Function(f)));
    Ok(node_value(&name))
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
            ctx.world.set_partition(ctx.task, &l, &r);
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

/// `w.crash(n, #{ restart_after })`, inside the running world.
fn crash(w: &WorldObject, args: Vec<Value>) -> Result<Value, InterpreterError> {
    let what = "w.crash";
    let Some(ctx) = live(w, what)? else {
        return Err(misuse(
            "w.crash acts on the running world: call it from a client or a hook (w.at)",
        ));
    };
    let name = node_name(args.first().unwrap_or(&Value::Unit), what)?;
    let n = resolve(&ctx, &name, what)?;
    let restart_after = match args.get(1) {
        None | Some(Value::Unit) => None,
        Some(o) => {
            let Some(o) = fields_of(o) else {
                return Err(misuse("w.crash: options are #{ restart_after }"));
            };
            match int_field(o, "restart_after", what)? {
                Some(ms) if ms >= 0 => Some(ms as u64),
                Some(_) => return Err(misuse("w.crash: `restart_after` must be non-negative")),
                None => None,
            }
        }
    };
    ctx.world
        .crash(ctx.task, n, restart_after)
        .map_err(misuse)?;
    // A node that crashes itself stops here.
    super::World::stopped(&ctx.world.lock(), ctx.task)?;
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

/// `w.run(duration)` / `w.run(#{ for })`: run the world's nodes.
fn run(
    w: &WorldObject,
    args: Vec<Value>,
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    let what = "w.run";
    let mut cfg = w.cfg.clone();
    cfg.run_for = run_for(args.first(), what)?;
    let spec = w.spec().clone();
    if spec.nodes.iter().all(|(n, _)| n.kind == Kind::Hook) {
        return Err(misuse(
            "w.run: the world has no nodes (add them with w.node and w.client), or use w.run_fn",
        ));
    }
    let mut nodes = vec![(NodeInfo::world(), Start::Main)];
    nodes.extend(spec.nodes);
    let id = |name: &str| {
        nodes
            .iter()
            .position(|(n, _)| &*n.name == name && n.kind != Kind::Hook)
            .filter(|i| *i > 0)
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
    let kept = interpreter.suspend_bytecode_tier();
    let template = interpreter.thread_safe_clone();
    let report = super::run_world(cfg, nodes, plan, template, w.id);
    interpreter.resume_bytecode_tier(kept);
    let report = report.map_err(misuse)?;
    collected(&report);
    Ok(report_value(&report))
}

/// A report as olang data.
pub fn report_value(r: &Report) -> Value {
    let mut m = ValueMap::default();
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
    Value::Map(Arc::new(m))
}

//! Nodes (S2): each node of a world is a tree of tasks with an
//! interpreter of its own — its own globals, module cache and capability
//! grant — sharing the world's scheduler, clock and seed.
//!
//! - **A program node** (`w.node(name, argv)`) runs an olang file on a
//!   fresh `Interpreter`, with `os.args()` and `os.get_env` answering
//!   the node's own argv and environment.
//! - **A client node** (`w.client(name, f)`) runs `f()` on a clone of
//!   the interpreter that built the world (its closures see what the
//!   builder saw).
//! - **A hook** (`w.at(t, f)`, `w.every(dt, f)`) runs on node 0, the
//!   world itself: faults as code (a partition at minute ten).
//!
//! **The coordinator** is task 0, on the thread that called `w.run`. It
//! starts every node, then waits: the run is `done` when every client has
//! finished (when there are no clients, every program), `failed` the
//! moment a node's program, a client or a hook raises, and otherwise
//! runs to its deadline. Servers never finish; at the end they unwind
//! with the world, as S1's tasks do.
//!
//! **A crash** (`w.crash(n)`) stops a node where it stands: its servers
//! stop listening (what they queued is lost), what is in flight to it is
//! dropped on arrival, and each of its tasks unwinds at its next yield
//! point. Memory is simply gone: a restart (`w.restart(n)`, or
//! `#{ restart_after }`) runs the node's program again on a fresh
//! interpreter.
//!
//! **S3: how a node stops.** A node's process can end four ways, and
//! each treats the disk as the real one would:
//!
//! - **`w.crash(n)`**: the power goes. Memory is gone and the disk is
//!   what was synced (`disk.rs`). No shutdown handler runs.
//! - **`w.kill(n)`**: SIGKILL. Memory is gone, the disk keeps every write
//!   (the operating system has them); no handler runs.
//! - **`w.stop(n)`**: SIGTERM. The node's `os.on_shutdown` handlers run,
//!   each as a task of the node, and the node ends when its program
//!   returns; with no handler, it is a kill.
//! - **Its program ends** (it returns, or calls `os.exit`): the process
//!   exits, its other tasks stop and its servers close, nothing is lost.
//!   `os.exit(n)` with `n` other than 0 fails the run, as a raise does.
//!
//! A restart is a fresh process: a fresh interpreter, the environment,
//! working directory and stdin the node was given, and the disk as the
//! stop left it.

use super::disk::DiskOpts;
use super::net::{Link, NodeId};
use super::sched::{TaskId, Wait};
use super::wire::COORD_BOX;
use super::{
    Config, Ctx, Ended, Report, State, World, current, error_text, misuse, spawn_task, vfs,
};
use crate::ast::{Value, ValueMap};
use crate::interpreter::{Interpreter, InterpreterError};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

/// What a node is, fixed for the run.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub name: Arc<str>,
    /// `os.args()` on the node: the program's path, then its arguments.
    pub argv: Vec<String>,
    /// `os.get_env` on the node.
    pub env: BTreeMap<String, String>,
    /// Added to the node's wall clock (`time.now_ms`, `dates.*`); its
    /// monotonic clock and timers are the world's.
    pub skew_ms: i64,
    /// The capabilities the node's program is denied (`#{ deny: "fs" }`).
    pub deny: Option<crate::caps::Caps>,
    /// A client holds the run open until it finishes; a program does
    /// when there are no clients.
    pub kind: Kind,
    /// How its disk misbehaves (`#{ disk: sim.disk(...) }`).
    pub disk: DiskOpts,
    /// The files its disk starts with, durable (`#{ files }`).
    pub files: Vec<(String, Vec<u8>)>,
    /// Its working directory (`#{ cwd }`, `/` unless given).
    pub cwd: String,
    /// What its stdin holds when it starts (`#{ stdin }`); without one
    /// its stdin is empty and closed, as `< /dev/null` is.
    pub stdin: Option<String>,
    /// Crashes at random effect boundaries (`#{ crash: #{ p, restart_after } }`).
    pub crash: Option<CrashPlan>,
    /// Its program's package dependencies (name → directory), resolved
    /// as `olang run` resolves them, when it lives in a package.
    pub deps: Option<HashMap<String, PathBuf>>,
    /// S4: the world-builder lets the shrinker leave it out
    /// (`#{ optional: true }`).
    pub optional: bool,
}

/// Crashes the seed places: at every effect a node's tasks make (and
/// every write and sync SQLite makes for it) the node crashes with
/// probability `p`, and comes back `restart_after` ms later if given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrashPlan {
    pub p: f64,
    pub restart_after: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Node 0: the world itself, or `run_fn`'s main program.
    World,
    Program,
    Client,
    Hook,
}

impl NodeInfo {
    /// `run_fn`'s (and `olang sim FILE`'s) one node.
    pub fn main(argv: Vec<String>) -> NodeInfo {
        NodeInfo {
            name: Arc::from("main"),
            argv,
            env: BTreeMap::new(),
            skew_ms: 0,
            deny: None,
            kind: Kind::World,
            disk: DiskOpts::default(),
            files: Vec::new(),
            cwd: "/".to_string(),
            stdin: None,
            crash: None,
            deps: None,
            optional: false,
        }
    }

    /// Node 0 of a world built with nodes.
    pub fn world() -> NodeInfo {
        NodeInfo {
            name: Arc::from("world"),
            kind: Kind::World,
            ..NodeInfo::main(Vec::new())
        }
    }
}

/// How a node starts (and restarts).
#[derive(Clone)]
pub enum Start {
    /// `run_fn`'s main program: started by its caller.
    Main,
    Program {
        path: PathBuf,
        source: String,
    },
    Function(Value),
    Hook {
        at: u64,
        every: Option<u64>,
        f: Value,
    },
}

/// What the network looks like when the run starts.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub links: Vec<(NodeId, NodeId, Link)>,
    pub cuts: Vec<(Vec<NodeId>, Vec<NodeId>)>,
}

/// A node's state while the world runs.
pub(super) struct NodeState {
    pub up: bool,
    /// Its current main task.
    pub main: Option<TaskId>,
    /// How its current main task ended (none while it runs).
    pub exit: Option<Result<Value, String>>,
    pub restart_pending: bool,
    pub crashes: u64,
    /// The last client port it used (connections number from 49152).
    pub next_port: u16,
    /// Its process's environment (`os.set_env` changes it; a restart
    /// starts from the one it was given).
    pub env: BTreeMap<String, String>,
    pub cwd: String,
    pub stdin: Stdin,
    /// `w.stop` asked it to stop: its handlers are running.
    pub stopping: bool,
}

/// A node's stdin: what is written and not yet read, and whether more
/// can come.
#[derive(Debug, Clone, Default)]
pub(super) struct Stdin {
    pub buf: String,
    pub open: bool,
}

impl Stdin {
    fn given(text: &Option<String>) -> Stdin {
        Stdin {
            buf: text.clone().unwrap_or_default(),
            open: text.is_some(),
        }
    }
}

impl NodeState {
    pub fn new(info: &NodeInfo) -> NodeState {
        NodeState {
            up: true,
            main: None,
            exit: None,
            restart_pending: false,
            crashes: 0,
            next_port: 49_151,
            env: info.env.clone(),
            cwd: info.cwd.clone(),
            stdin: Stdin::given(&info.stdin),
            stopping: false,
        }
    }
}

/// How a node's process ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Halt {
    /// The power went: the disk keeps what was synced.
    Crash,
    /// SIGKILL: the disk keeps every write.
    Kill,
    /// The program ended (returned, or `os.exit`).
    Exit,
}

/// Run a world of nodes: start each, coordinate, report.
/// `nodes[0]` is the world itself (`NodeInfo::world()`); `template` is
/// the interpreter clients and hooks are cloned from.
pub fn run_world(
    cfg: Config,
    nodes: Vec<(NodeInfo, Start)>,
    plan: Plan,
    template: Interpreter,
    spec: u64,
    props: Vec<super::props::Property>,
) -> Result<Report, String> {
    let (infos, starts): (Vec<NodeInfo>, Vec<Start>) = nodes.into_iter().unzip();
    let world = Arc::new(World::new(&cfg, infos, starts, Some(template), spec, props));
    {
        let mut st = world.lock();
        for (a, b, link) in &plan.links {
            st.net.set_link(*a, *b, *link);
        }
        for (left, right) in &plan.cuts {
            st.net.partition(left, right);
        }
    }
    super::run_in(world, coordinate)
}

/// Task 0 of a world of nodes.
fn coordinate() -> Result<Value, InterpreterError> {
    let ctx = current().ok_or_else(|| misuse("sim: the coordinator runs inside its world"))?;
    let w = &ctx.world;
    for n in 1..w.nodes.len() {
        if w.dropped[n] {
            // left out by the plan: a node that never starts
            let mut st = w.lock();
            st.nodes[n].up = false;
            st.nodes[n].exit = Some(Ok(Value::Unit));
            let detail = format!("{} (left out by the plan)", w.nodes[n].name);
            w.event(&mut st, 0, "dropped", detail);
            continue;
        }
        start_node(w, 0, n)?;
    }
    let watched: Vec<NodeId> = {
        let live = |n: &NodeId| !w.dropped[*n];
        let clients: Vec<NodeId> = (1..w.nodes.len())
            .filter(|n| w.nodes[*n].kind == Kind::Client)
            .filter(live)
            .collect();
        if clients.is_empty() {
            (1..w.nodes.len())
                .filter(|n| w.nodes[*n].kind == Kind::Program)
                .filter(live)
                .collect()
        } else {
            clients
        }
    };
    loop {
        {
            let st = w.lock();
            if let Some(failure) = &st.failure {
                return Err(misuse(failure.clone()));
            }
            let settled = |n: &NodeId| {
                let s = &st.nodes[*n];
                s.exit.is_some() && !s.restart_pending
            };
            if !watched.is_empty() && watched.iter().all(settled) {
                let mut results = ValueMap::default();
                for n in &watched {
                    if let Some(Ok(v)) = &st.nodes[*n].exit {
                        results.insert(w.nodes[*n].name.to_string(), v.clone());
                    }
                }
                return Ok(Value::Map(Arc::new(results)));
            }
        }
        // Past the deadline by one, so the deadline (not this wait) ends it.
        let until = w.lock().sched.deadline.map(|d| d + 1);
        w.block(
            0,
            Wait::Net(COORD_BOX),
            "the world: waiting for its clients".into(),
            until,
        )?;
    }
}

/// Start node `n`'s main task (spawned by `parent`).
pub(super) fn start_node(
    w: &Arc<World>,
    parent: TaskId,
    n: NodeId,
) -> Result<TaskId, InterpreterError> {
    let start = w.starts.lock().unwrap_or_else(|e| e.into_inner())[n].clone();
    let info = w.nodes[n].clone();
    let label = format!("node {}", info.name);
    let body: Box<dyn FnOnce() -> Result<Value, String> + Send> = match start {
        Start::Main => return Err(misuse("sim: the main program is started by its caller")),
        Start::Program { path, source } => Box::new(move || {
            let mut interpreter = Interpreter::new();
            super::tier_for(&mut interpreter);
            interpreter.set_current_file(&path);
            if let Some(deps) = info.deps {
                interpreter.set_dependency_map(deps);
            }
            if let Some(caps) = info.deny {
                interpreter.set_capabilities(crate::caps::CapTable {
                    app: caps,
                    ..Default::default()
                });
            }
            let program = crate::parser::Parser::new()
                .parse(&source)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            interpreter
                .eval_program(program)
                .map_err(|e| error_text(&e))
        }),
        Start::Function(f) => {
            let mut interpreter = template_clone(w)?;
            Box::new(move || {
                interpreter
                    .call_function(f, Vec::new())
                    .map_err(|e| error_text(&e))
            })
        }
        Start::Hook { at, every, f } => {
            let mut interpreter = template_clone(w)?;
            Box::new(move || {
                let ctx = current().ok_or("sim: a hook runs inside its world")?;
                sleep_until(&ctx, at).map_err(|e| InterpreterError::from(e).to_string())?;
                let mut tick = at;
                loop {
                    interpreter
                        .call_function(f.clone(), Vec::new())
                        .map_err(|e| error_text(&e))?;
                    let Some(dt) = every else {
                        return Ok(Value::Unit);
                    };
                    // Ticks fall at at + k·dt; one that `f` overran is skipped.
                    let now = ctx.world.now();
                    let dt = dt.max(1);
                    tick += dt;
                    if tick <= now {
                        tick += (now - tick) / dt * dt + dt;
                    }
                    sleep_until(&ctx, tick).map_err(|e| InterpreterError::from(e).to_string())?;
                }
            })
        }
    };
    {
        // A fresh process: the environment, directory and stdin it was
        // given; the disk as it was left.
        let mut st = w.lock();
        let info = &w.nodes[n];
        let s = &mut st.nodes[n];
        s.env = info.env.clone();
        s.cwd = info.cwd.clone();
        s.stdin = Stdin::given(&info.stdin);
        s.stopping = false;
    }
    let (t, _handle) = spawn_task(w, parent, n, None, &label, body)?;
    let mut st = w.lock();
    let s = &mut st.nodes[n];
    s.main = Some(t);
    s.exit = None;
    s.up = true;
    s.restart_pending = false;
    Ok(t)
}

fn template_clone(w: &World) -> Result<Interpreter, InterpreterError> {
    let template = w.template.lock().unwrap_or_else(|e| e.into_inner());
    template
        .as_ref()
        .map(|i| i.thread_safe_clone())
        .ok_or_else(|| misuse("sim: this world has no interpreter to run functions with"))
}

fn sleep_until(ctx: &Ctx, at: u64) -> Result<(), Ended> {
    while ctx.world.now() < at {
        ctx.world.block(
            ctx.task,
            Wait::Sleep,
            format!("waiting until {at} ms"),
            Some(at),
        )?;
    }
    Ok(())
}

impl World {
    /// Bookkeeping as a task ends (under the lock): a node's main task
    /// records how the node ended, a raise fails the run, a program's
    /// end is its process's end, and the coordinator looks again.
    pub(super) fn node_task_exit(
        &self,
        st: &mut State,
        me: TaskId,
        outcome: &Result<Value, String>,
    ) {
        let n = st.task_node[me];
        if n != 0 || self.nodes.len() > 1 {
            let is_main = st.nodes[n].main == Some(me);
            if is_main && st.killed[me].is_none() {
                st.nodes[n].exit = Some(outcome.clone());
                if let Err(e) = outcome {
                    let e = e.strip_prefix("Runtime error: ").unwrap_or(e);
                    let what = match self.nodes[n].kind {
                        Kind::Client => "client",
                        Kind::Hook => "hook on",
                        _ => "node",
                    };
                    if st.failure.is_none() {
                        st.failure = Some(format!("{what} {}: {e}", self.nodes[n].name));
                    }
                }
                // A program's process ends with its program: what it
                // spawned stops, its servers close.
                if self.nodes[n].kind == Kind::Program && st.nodes[n].up {
                    let why = if st.nodes[n].stopping {
                        "stopped: its program returned".to_string()
                    } else {
                        "its program returned".to_string()
                    };
                    self.halt_locked(st, me, n, Halt::Exit, &why, None);
                }
            } else if is_main {
                st.nodes[n].exit.get_or_insert(Err("crashed".into()));
            }
        }
        st.sched.wake(Wait::Net(COORD_BOX));
    }

    /// Node `n`'s process ends (under the lock): its tasks are told to
    /// stop, its servers close, its handlers are dropped, and its disk is
    /// what `how` leaves (see the module doc). Returns what the disk lost.
    pub(super) fn halt_locked(
        &self,
        st: &mut State,
        me: TaskId,
        n: NodeId,
        how: Halt,
        cause: &str,
        key: Option<super::plan::Key>,
    ) -> String {
        let lost = {
            let mut d = vfs::lock(&self.disks[n]);
            let lost = match how {
                Halt::Crash => {
                    let mut rng =
                        std::mem::replace(&mut d.rng, rand::SeedableRng::seed_from_u64(0));
                    let lost = d.disk.crash(&mut rng);
                    d.rng = rng;
                    lost
                }
                Halt::Kill | Halt::Exit => {
                    d.disk.kill();
                    String::new()
                }
            };
            d.forget_process();
            lost
        };
        st.nodes[n].up = false;
        st.nodes[n].stopping = false;
        st.nodes[n].stdin.open = false;
        if how == Halt::Crash {
            st.nodes[n].crashes += 1;
        }
        self.drop_listeners(st, n);
        let why = match how {
            Halt::Crash => Ended::Crash,
            Halt::Kill => Ended::Kill,
            Halt::Exit => Ended::Exit,
        };
        for t in 0..st.sched.task_count() {
            // The task ending the process by its program's return (or by
            // os.exit, which unwinds it itself) is left to end.
            if st.task_node[t] != n || st.sched.is_done(t) || (t == me && how == Halt::Exit) {
                continue;
            }
            st.killed[t] = Some(why);
            st.sched.kick(t);
        }
        if st.nodes[n].main.is_some_and(|t| st.sched.is_done(t)) {
            st.nodes[n].exit.get_or_insert(Err("crashed".into()));
        }
        // the handlers go with the process (dropped outside this lock's
        // other users: they are only interpreters and functions)
        let dropped = self
            .handlers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&n);
        drop(dropped);
        let detail = match how {
            Halt::Crash if lost.is_empty() => format!("{} ({cause})", self.nodes[n].name),
            Halt::Crash => format!("{} ({cause}): {lost}", self.nodes[n].name),
            _ => format!("{} ({cause})", self.nodes[n].name),
        };
        match how {
            Halt::Crash => self.fault_at(st, Some(me), n, "crash", detail, key),
            Halt::Kill => self.fault_at(st, Some(me), n, "kill", detail, key),
            Halt::Exit => self.event(st, me, "exit", detail),
        }
        st.sched.wake(Wait::Stdin(n));
        st.sched.wake(Wait::Net(COORD_BOX));
        lost
    }

    /// `w.crash(n)` / `w.kill(n)`: node `n` stops where it stands, and
    /// with `restart_after` comes back that many ms later.
    pub(super) fn halt(
        self: &Arc<World>,
        me: TaskId,
        n: NodeId,
        how: Halt,
        restart_after: Option<u64>,
        cause: &str,
        key: Option<super::plan::Key>,
    ) -> Result<(), String> {
        let verb = match how {
            Halt::Crash => "w.crash",
            Halt::Kill => "w.kill",
            Halt::Exit => "os.exit",
        };
        if n == 0 {
            return Err(format!("{verb}: the world itself cannot stop"));
        }
        {
            let mut st = self.lock();
            if !st.nodes[n].up {
                return Err(format!("{verb}: {} is down already", self.nodes[n].name));
            }
            let cause = match restart_after {
                Some(ms) => format!("{cause}, restart after {ms} ms"),
                None => cause.to_string(),
            };
            self.halt_locked(&mut st, me, n, how, &cause, key);
            st.nodes[n].restart_pending = restart_after.is_some();
        }
        if let Some(ms) = restart_after {
            self.restart_later(me, n, ms)?;
        }
        Ok(())
    }

    /// Restart `n` `ms` after it is down (a stopping node is waited for).
    pub(super) fn restart_later(
        self: &Arc<World>,
        me: TaskId,
        n: NodeId,
        ms: u64,
    ) -> Result<(), String> {
        let world = self.clone();
        spawn_task(
            self,
            me,
            0,
            None,
            &format!("restart {}", self.nodes[n].name),
            move || {
                let ctx = current().ok_or("sim: a restart runs inside its world")?;
                let stop = |e: Ended| InterpreterError::from(e).to_string();
                // Back `ms` after now, once the node is down and its old
                // tasks have unwound (a stopping node runs its handlers
                // first; a crashed one's tasks unwind at once).
                let mut at = world.now() + ms;
                loop {
                    sleep_until(&ctx, at).map_err(stop)?;
                    let ready = {
                        let st = world.lock();
                        !st.nodes[n].up
                            && (0..st.sched.task_count())
                                .all(|t| st.task_node[t] != n || st.sched.is_done(t))
                    };
                    if ready {
                        break;
                    }
                    at = world.now() + 10;
                }
                world.restart(ctx.task, n).map(|_| Value::Unit)
            },
        )
        .map_err(|e| error_text(&e))?;
        Ok(())
    }

    /// `w.stop(n)`: SIGTERM. Its shutdown handlers run as tasks of the
    /// node; without one, it is a kill.
    pub(super) fn stop(
        self: &Arc<World>,
        me: TaskId,
        n: NodeId,
        restart_after: Option<u64>,
        key: Option<super::plan::Key>,
    ) -> Result<(), String> {
        if n == 0 {
            return Err("w.stop: the world itself cannot stop".into());
        }
        let handlers = self
            .handlers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&n)
            .unwrap_or_default();
        if handlers.is_empty() {
            return self.halt(
                me,
                n,
                Halt::Kill,
                restart_after,
                "w.stop: no shutdown handler",
                key,
            );
        }
        {
            let mut st = self.lock();
            if !st.nodes[n].up {
                return Err(format!("w.stop: {} is down already", self.nodes[n].name));
            }
            st.nodes[n].stopping = true;
            st.nodes[n].restart_pending = restart_after.is_some();
            let detail = format!(
                "{} ({} shutdown handler(s))",
                self.nodes[n].name,
                handlers.len()
            );
            self.fault_at(&mut st, Some(me), n, "stop", detail, key);
        }
        for (mut interpreter, handler) in handlers {
            spawn_task(self, me, n, None, "shutdown handler", move || {
                interpreter
                    .call_function(handler, vec![Value::String(Arc::new("shutdown".into()))])
                    .map_err(|e| error_text(&e))
            })
            .map_err(|e| error_text(&e))?;
        }
        if let Some(ms) = restart_after {
            self.restart_later(me, n, ms)?;
        }
        Ok(())
    }

    /// `w.restart(n)`: a crashed node runs its program again, fresh.
    pub(super) fn restart(self: &Arc<World>, me: TaskId, n: NodeId) -> Result<TaskId, String> {
        {
            let mut st = self.lock();
            if st.nodes[n].up {
                return Err(format!("w.restart: {} is up", self.nodes[n].name));
            }
            // Its old tasks must be gone before the new program starts.
            if (0..st.sched.task_count()).any(|t| st.task_node[t] == n && !st.sched.is_done(t)) {
                return Err(format!(
                    "w.restart: {}'s old tasks are still unwinding (restart it a moment later)",
                    self.nodes[n].name
                ));
            }
            let detail = self.nodes[n].name.to_string();
            self.event(&mut st, me, "restart", detail);
        }
        start_node(self, me, n).map_err(|e| error_text(&e))
    }
}

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
//! interpreter. (S3 adds the disk that survives a crash.)

use super::net::{Link, NodeId};
use super::sched::{TaskId, Wait};
use super::wire::COORD_BOX;
use super::{Config, Ctx, Ended, Report, State, World, current, error_text, misuse, spawn_task};
use crate::ast::{Value, ValueMap};
use crate::interpreter::{Interpreter, InterpreterError};
use std::collections::BTreeMap;
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
}

impl NodeState {
    pub fn new() -> NodeState {
        NodeState {
            up: true,
            main: None,
            exit: None,
            restart_pending: false,
            crashes: 0,
            next_port: 49_151,
        }
    }
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
) -> Result<Report, String> {
    let (infos, starts): (Vec<NodeInfo>, Vec<Start>) = nodes.into_iter().unzip();
    let world = Arc::new(World::new(&cfg, infos, starts, Some(template), spec));
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
        start_node(w, 0, n)?;
    }
    let watched: Vec<NodeId> = {
        let clients: Vec<NodeId> = (1..w.nodes.len())
            .filter(|n| w.nodes[*n].kind == Kind::Client)
            .collect();
        if clients.is_empty() {
            (1..w.nodes.len())
                .filter(|n| w.nodes[*n].kind == Kind::Program)
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
            interpreter.set_current_file(&path);
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
    /// records how the node ended, a raise fails the run, and the
    /// coordinator looks again.
    pub(super) fn node_task_exit(
        &self,
        st: &mut State,
        me: TaskId,
        outcome: &Result<Value, String>,
    ) {
        let n = st.task_node[me];
        if n != 0 || self.nodes.len() > 1 {
            let is_main = st.nodes[n].main == Some(me);
            if is_main && !st.killed[me] {
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
            } else if is_main {
                st.nodes[n].exit = Some(Err("crashed".into()));
            }
        }
        st.sched.wake(Wait::Net(COORD_BOX));
    }

    /// `w.crash(n)`: node `n` stops where it stands.
    pub(super) fn crash(
        self: &Arc<World>,
        me: TaskId,
        n: NodeId,
        restart_after: Option<u64>,
    ) -> Result<(), String> {
        if n == 0 {
            return Err("w.crash: the world itself cannot crash".into());
        }
        {
            let mut st = self.lock();
            if !st.nodes[n].up {
                return Err(format!("w.crash: {} is down already", self.nodes[n].name));
            }
            let detail = match restart_after {
                Some(ms) => format!("{} (restart after {ms} ms)", self.nodes[n].name),
                None => self.nodes[n].name.to_string(),
            };
            self.event(&mut st, me, "crash", detail);
            st.nodes[n].up = false;
            st.nodes[n].crashes += 1;
            st.nodes[n].restart_pending = restart_after.is_some();
            self.drop_listeners(&mut st, n);
            for t in 0..st.sched.task_count() {
                if st.task_node[t] == n && !st.sched.is_done(t) {
                    st.killed[t] = true;
                    st.sched.kick(t);
                }
            }
            if st.nodes[n].main.is_some_and(|t| st.sched.is_done(t)) {
                st.nodes[n].exit.get_or_insert(Err("crashed".into()));
            }
        }
        if let Some(ms) = restart_after {
            let world = self.clone();
            let at = self.now() + ms;
            spawn_task(
                self,
                me,
                0,
                None,
                &format!("restart {}", self.nodes[n].name),
                move || {
                    let ctx = current().ok_or("sim: a restart runs inside its world")?;
                    sleep_until(&ctx, at).map_err(|e| InterpreterError::from(e).to_string())?;
                    world.restart(ctx.task, n).map(|_| Value::Unit)
                },
            )
            .map_err(|e| error_text(&e))?;
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

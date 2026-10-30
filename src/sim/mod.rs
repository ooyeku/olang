//! `sim` — deterministic simulation of a whole program (plans/SIM.md).
//!
//! A *world* runs an olang program on a virtual clock, with randomness
//! drawn from streams split from one seed, under a cooperative scheduler
//! the seed drives. Two runs with one seed are the same run, event for
//! event; a race that some seed exposes comes back every time with that
//! seed. Milestone S1 simulates the clock, randomness, and `spawn`,
//! `task` and `chan`; S2 adds nodes (each an interpreter of its own) and
//! the network between them (`http` over simulated links, with latency,
//! loss, duplication and partitions). Everything else that touches the
//! world is refused inside one (`effects::sim_handling`), never run for
//! real.
//!
//! **How it fits the runtime** (plans/SIM-S1.md has the design note):
//!
//! - **One dispatch point.** Every builtin passes
//!   `BuiltinFunctions::call_internal`; inside a world it first asks
//!   [`intercept`], which answers from the world, refuses, or lets a pure
//!   call through. The fourth consumer of `effects.rs`.
//! - **A baton, not a new interpreter.** Tasks stay OS threads, each
//!   running the ordinary tree-walker, but only the task holding the
//!   world's baton runs; every other one waits on its own condition
//!   variable. At each yield point (`spawn`, every channel operation,
//!   `task.join`, `time.sleep`) the holder asks the scheduler
//!   ([`sched::Sched`]) who runs next and hands the baton over. One task
//!   runs at a time, in the order the seed picks.
//! - **Thread-local context.** A thread in a world carries its world and
//!   task id, so the clock (`crate::clock`) and the random module answer
//!   from the world wherever they are called from, and the bytecode
//!   tier's bridge dispatch is intercepted the same way. A world still
//!   runs on the interpreter tier (as record and replay do): `run_fn`
//!   sets the bytecode tier aside while the world runs.
//! - **Nodes and the network** (S2, plans/SIM-S2.md): a node is a task
//!   tree with an interpreter of its own (`node.rs`); `http` between
//!   nodes is messages on the world's timer queue (`wire.rs`, `http.rs`),
//!   and what a link does to each one is `net.rs`'s draw.

pub mod cli;
pub mod http;
pub mod net;
mod node;
pub mod sched;
pub mod surface;
mod wire;

pub use node::{NodeInfo, Plan, Start, run_world};
pub use wire::{History, NetStats};

use crate::ast::{Expr, Value};
use crate::effects::SimHandling;
use crate::interpreter::{Interpreter, InterpreterError};
use net::NodeId;
use rand::SeedableRng;
use rand::rngs::StdRng;
use sched::{End, Pick, Sched, TaskId, Wait, Woke};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// 2026-01-01T00:00:00Z: where a world's clock starts unless told.
pub const DEFAULT_START_MS: i64 = 1_767_225_600_000;

/// Yield points a run may take before it is called a livelock.
pub const DEFAULT_MAX_STEPS: u64 = 10_000_000;

/// How long (real time) the end of a world waits for its tasks to unwind.
const DRAIN_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// What a world is made with.
#[derive(Debug, Clone)]
pub struct Config {
    pub seed: u64,
    /// The world's epoch: what `time.now_ms()` reads at virtual time 0.
    pub start_ms: i64,
    /// Stop when virtual time passes this many ms (`for`), if given.
    pub run_for: Option<u64>,
    pub max_steps: u64,
    /// What a link does unless `w.link` says otherwise.
    pub net: net::Link,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            seed: 0,
            start_ms: DEFAULT_START_MS,
            run_for: None,
            max_steps: DEFAULT_MAX_STEPS,
            net: net::Link::default(),
        }
    }
}

/// One thing that happened in a world: when (virtual ms), on which node,
/// in which task (none for what the network does on its own: a
/// delivery, a drop), what kind, and a short rendering of the detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub t: u64,
    pub node: Arc<str>,
    pub task: Option<TaskId>,
    pub kind: &'static str,
    pub detail: String,
}

/// What a run ended as, and everything that happened in it.
#[derive(Debug, Clone)]
pub struct Report {
    pub seed: u64,
    /// "done", "failed", "deadline", "deadlock" or "step_limit".
    pub outcome: &'static str,
    /// The main program's value, when it returned one.
    pub value: Option<Value>,
    /// Why it failed: the main program's error, or the deadlock's sites.
    pub error: Option<String>,
    pub events: Vec<Event>,
    /// Virtual ms at the end.
    pub now: u64,
    pub steps: u64,
    pub tasks: usize,
    /// Spawned tasks that raised (before the world ended).
    pub failed_tasks: usize,
    /// A task did not unwind when the world ended (it caught the end
    /// and kept computing); its thread was left behind.
    pub stuck: bool,
    /// The nodes, in order (the first is the world itself, or `main`).
    pub nodes: Vec<String>,
    /// Every request a node made over the network, as it went.
    pub history: Vec<History>,
    pub net: NetStats,
}

impl Report {
    /// A report of no run at all (a file that built no world).
    pub fn empty(seed: u64) -> Report {
        Report {
            seed,
            outcome: "done",
            value: None,
            error: None,
            events: Vec::new(),
            now: 0,
            steps: 0,
            tasks: 0,
            failed_tasks: 0,
            stuck: false,
            nodes: Vec::new(),
            history: Vec::new(),
            net: NetStats::default(),
        }
    }

    /// Whether this run is a failure worth a seed: the main program
    /// raised, every task deadlocked, or the run never settled.
    pub fn failed(&self) -> bool {
        matches!(self.outcome, "failed" | "deadlock" | "step_limit")
    }
}

/// What the world knows of a channel used inside it.
struct ChanInfo {
    /// The world's own number for it: 1, 2, ... in order of first use,
    /// so it is the same in every run of a seed.
    local: u64,
    /// Made by `chan.bounded(0)` inside the world: a send waits until a
    /// receiver has taken its message.
    rendezvous: bool,
    sent: u64,
    taken: u64,
}

struct State {
    sched: Sched,
    /// The task holding the baton: the only one running olang code.
    running: TaskId,
    ended: Option<End>,
    events: Vec<Event>,
    chans: HashMap<u64, ChanInfo>,
    tasks_by_global: HashMap<u64, TaskId>,
    outcomes: Vec<Option<Result<Value, String>>>,
    cvs: Vec<Arc<Condvar>>,
    failed_tasks: usize,
    /// Each task's node (task 0 is node 0's).
    task_node: Vec<NodeId>,
    /// Tasks of a crashed node: each unwinds at its next yield point.
    killed: Vec<bool>,
    nodes: Vec<node::NodeState>,
    net: net::Net,
    wire: wire::Wire,
    /// A node's program (or a client, or a hook) raised: the run fails
    /// with this once the world's coordinator sees it.
    failure: Option<String>,
}

/// A simulated world. Shared (`Arc`) by every task thread in it.
pub struct World {
    seed: u64,
    start_ms: i64,
    /// Node 0 is the world itself (or the main program of `run_fn`).
    nodes: Vec<NodeInfo>,
    /// How each node starts (and restarts): a program, a function, a hook.
    starts: Mutex<Vec<Start>>,
    /// The interpreter clients and hooks are cloned from (the one that
    /// built the world), when there is one.
    template: Mutex<Option<Interpreter>>,
    /// The surface's number for the `sim.world` value this world runs,
    /// so `w.partition` inside it acts on its own world only.
    spec: u64,
    state: Mutex<State>,
}

/// The task cannot go on: the world has ended, or its node crashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    World,
    Crash,
}

pub const ENDED_MESSAGE: &str = "sim: the world has ended, and this task stops with it";
pub const CRASHED_MESSAGE: &str = "sim: this node crashed, and its tasks stop with it";

impl From<Ended> for InterpreterError {
    fn from(e: Ended) -> Self {
        InterpreterError::runtime(match e {
            Ended::World => ENDED_MESSAGE,
            Ended::Crash => CRASHED_MESSAGE,
        })
    }
}

impl World {
    fn new(
        cfg: &Config,
        nodes: Vec<NodeInfo>,
        starts: Vec<Start>,
        template: Option<Interpreter>,
        spec: u64,
    ) -> World {
        let mut sched = Sched::new(sched::split(cfg.seed, 1));
        sched.deadline = cfg.run_for;
        sched.max_steps = cfg.max_steps;
        let node_states = (0..nodes.len()).map(|_| node::NodeState::new()).collect();
        World {
            seed: cfg.seed,
            start_ms: cfg.start_ms,
            nodes,
            starts: Mutex::new(starts),
            template: Mutex::new(template),
            spec,
            state: Mutex::new(State {
                sched,
                running: 0,
                ended: None,
                events: Vec::new(),
                chans: HashMap::new(),
                tasks_by_global: HashMap::new(),
                outcomes: vec![None],
                cvs: vec![Arc::new(Condvar::new())],
                failed_tasks: 0,
                task_node: vec![0],
                killed: vec![false],
                nodes: node_states,
                net: net::Net::new(sched::split(cfg.seed, 2), cfg.net),
                wire: wire::Wire::default(),
                failure: None,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Task `t`'s own random stream, split from the world's seed by
    /// (node, task). Node 0's streams are S1's (seed, task) ones.
    fn task_rng(&self, node: NodeId, t: TaskId) -> Arc<Mutex<StdRng>> {
        let seed = if node == 0 {
            sched::split(self.seed, 1_000 + t as u64)
        } else {
            sched::split(sched::split(self.seed, 1_000_000 + node as u64), t as u64)
        };
        Arc::new(Mutex::new(StdRng::seed_from_u64(seed)))
    }

    fn now(&self) -> u64 {
        self.lock().sched.now
    }

    fn event(&self, st: &mut State, task: TaskId, kind: &'static str, detail: String) {
        let node = st.task_node.get(task).copied().unwrap_or(0);
        st.events.push(Event {
            t: st.sched.now,
            node: self.nodes[node].name.clone(),
            task: Some(task),
            kind,
            detail,
        });
    }

    /// Something the network did on its own, at `node`.
    fn net_event(&self, st: &mut State, node: NodeId, kind: &'static str, detail: String) {
        st.events.push(Event {
            t: st.sched.now,
            node: self.nodes[node].name.clone(),
            task: None,
            kind,
            detail,
        });
    }

    /// Choose who runs next (`Sched::next`), handling every delivery
    /// that falls due on the way.
    fn pick(&self, st: &mut State) -> Result<TaskId, End> {
        let mut counted = true;
        loop {
            match st.sched.next(counted)? {
                Pick::Run(t) => return Ok(t),
                Pick::Deliver(id) => {
                    counted = false;
                    self.deliver(st, id);
                }
            }
        }
    }

    /// Why `me` cannot go on, if it cannot.
    fn stopped(st: &State, me: TaskId) -> Result<(), Ended> {
        if st.ended.is_some() {
            Err(Ended::World)
        } else if st.killed.get(me).copied().unwrap_or(false) {
            Err(Ended::Crash)
        } else {
            Ok(())
        }
    }

    /// Wait until the baton is `me`'s.
    fn wait_turn<'a>(&self, mut st: MutexGuard<'a, State>, me: TaskId) -> MutexGuard<'a, State> {
        let cv = st.cvs[me].clone();
        while st.running != me {
            st = cv.wait(st).unwrap_or_else(|e| e.into_inner());
        }
        st
    }

    /// The world stops: record why and give the baton to the main task,
    /// which unwinds and then lets every other task unwind in turn.
    fn end(&self, st: &mut State, end: End) {
        let detail = match &end {
            End::Deadlock(sites) => sites
                .iter()
                .map(|(t, s)| format!("task {t}: {s}"))
                .collect::<Vec<_>>()
                .join("; "),
            End::Deadline => "the run's time is up".to_string(),
            End::StepLimit => format!("{} yield points without settling", st.sched.steps - 1),
            End::MainReturned => String::new(),
        };
        if !matches!(end, End::MainReturned) {
            let running = st.running;
            self.event(st, running, end.name(), detail);
        }
        st.ended = Some(end);
        st.running = 0;
        st.cvs[0].notify_one();
    }

    /// Hand the baton to whoever the seed picks (perhaps `me` again) and
    /// wait for it to come back.
    fn reschedule<'a>(
        &self,
        mut st: MutexGuard<'a, State>,
        me: TaskId,
    ) -> (MutexGuard<'a, State>, Result<(), Ended>) {
        if let Err(e) = Self::stopped(&st, me) {
            return (st, Err(e));
        }
        match self.pick(&mut st) {
            Ok(next) if next == me => {}
            Ok(next) => {
                st.running = next;
                st.cvs[next].notify_one();
                st = self.wait_turn(st, me);
            }
            Err(end) => {
                self.end(&mut st, end);
                st = self.wait_turn(st, me);
            }
        }
        let r = Self::stopped(&st, me);
        (st, r)
    }

    /// A yield point: the seed may let another task run first.
    fn yield_point(&self, me: TaskId) -> Result<(), Ended> {
        let st = self.lock();
        self.reschedule(st, me).1
    }

    /// Block `me` until `wait` changes (or virtual time `until`).
    fn block(
        &self,
        me: TaskId,
        wait: Wait,
        site: String,
        until: Option<u64>,
    ) -> Result<Woke, Ended> {
        let mut st = self.lock();
        Self::stopped(&st, me)?;
        st.sched.block(me, wait, site, until);
        let (mut st, r) = self.reschedule(st, me);
        r?;
        Ok(st.sched.woke(me))
    }

    /// A task spawned by `parent` on `node`: its world id and random
    /// stream. `global` is the interpreter's number for an olang
    /// `spawn` (none for the world's own tasks: node programs, server
    /// workers, hooks).
    fn add_task(
        &self,
        parent: TaskId,
        node: NodeId,
        global: Option<u64>,
        what: &str,
    ) -> (TaskId, Arc<Mutex<StdRng>>) {
        let mut st = self.lock();
        let t = st.sched.add_task();
        st.cvs.push(Arc::new(Condvar::new()));
        st.outcomes.push(None);
        st.task_node.push(node);
        st.killed.push(false);
        if let Some(global) = global {
            st.tasks_by_global.insert(global, t);
        }
        let detail = if what.is_empty() {
            format!("task {t}")
        } else {
            format!("task {t}: {what}")
        };
        self.event(&mut st, parent, "spawn", detail);
        drop(st);
        (t, self.task_rng(node, t))
    }

    /// A spawned task's first wait: for the seed to pick it.
    fn wait_first_turn(&self, me: TaskId) -> Result<(), Ended> {
        let st = self.lock();
        let st = self.wait_turn(st, me);
        Self::stopped(&st, me)
    }

    /// A spawned task has ended: keep its outcome, wake its joiners, and
    /// pass the baton on. The thread exits after this without waiting.
    fn task_exit(&self, me: TaskId, outcome: Result<Value, String>) {
        let mut st = self.lock();
        let ended = st.ended.is_some();
        let killed = st.killed[me];
        match &outcome {
            Ok(v) => self.event(&mut st, me, "end", render(v)),
            Err(_) if killed && !ended => {
                self.event(&mut st, me, "stop", "its node crashed".into())
            }
            Err(e) if !ended => {
                st.failed_tasks += 1;
                let e = e.strip_prefix("Runtime error: ").unwrap_or(e).to_string();
                self.event(&mut st, me, "fail", e);
            }
            Err(_) => {}
        }
        if !ended {
            self.node_task_exit(&mut st, me, &outcome);
        }
        st.outcomes[me] = Some(outcome);
        st.sched.finish(me);
        if ended {
            st.running = 0;
            st.cvs[0].notify_one();
            return;
        }
        match self.pick(&mut st) {
            Ok(next) => {
                st.running = next;
                st.cvs[next].notify_one();
            }
            Err(end) => self.end(&mut st, end),
        }
    }

    /// The main program is over (returned, raised, or stopped by the
    /// world's end): let every other task unwind, one at a time, and
    /// report.
    fn main_exit(&self, result: Result<Value, String>) -> Report {
        let mut st = self.lock();
        if st.ended.is_none() {
            match &result {
                Ok(v) => self.event(&mut st, 0, "done", render(v)),
                Err(e) => self.event(&mut st, 0, "failed", e.clone()),
            }
            st.ended = Some(End::MainReturned);
        }
        // The rest unwind in id order, each holding the baton alone.
        let mut stuck = false;
        let give_up = crate::clock::Instant::now() + DRAIN_WAIT;
        let cv = st.cvs[0].clone();
        'drain: for t in 1..st.sched.task_count() {
            if st.sched.is_done(t) {
                continue;
            }
            st.running = t;
            st.cvs[t].notify_one();
            while st.running != 0 {
                let left = give_up.saturating_duration_since(crate::clock::Instant::now());
                if left.is_zero() {
                    stuck = true;
                    break 'drain;
                }
                st = cv
                    .wait_timeout(st, left)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        }
        let end = st.ended.clone().unwrap_or(End::MainReturned);
        let (outcome, value, error) = match (end, result) {
            (End::MainReturned, Ok(v)) => ("done", Some(v), None),
            (End::MainReturned, Err(e)) => ("failed", None, Some(e)),
            (End::Deadlock(sites), _) => (
                "deadlock",
                None,
                Some(format!(
                    "every task is blocked and no timer is set — {}",
                    sites
                        .iter()
                        .map(|(t, s)| format!("task {t}: {s}"))
                        .collect::<Vec<_>>()
                        .join("; ")
                )),
            ),
            (End::Deadline, _) => ("deadline", None, None),
            (End::StepLimit, _) => (
                "step_limit",
                None,
                Some(format!(
                    "{} yield points without the run ending (a livelock?)",
                    st.sched.max_steps
                )),
            ),
        };
        Report {
            seed: self.seed,
            outcome,
            value,
            error,
            events: std::mem::take(&mut st.events),
            now: st.sched.now,
            steps: st.sched.steps,
            tasks: st.sched.task_count(),
            failed_tasks: st.failed_tasks,
            stuck,
            nodes: self.nodes.iter().map(|n| n.name.to_string()).collect(),
            history: std::mem::take(&mut st.wire.history),
            net: st.wire.stats.clone(),
        }
    }

    // ─── channels ────────────────────────────────────────────────────

    /// The world's number for a channel (registering it on first use)
    /// and whether it is a rendezvous.
    fn chan_info(&self, channel: &Value) -> Result<(u64, bool), String> {
        let (global, cap) = crate::stdlib::chan::sim_identity(channel)?;
        let mut st = self.lock();
        if let Some(info) = st.chans.get(&global) {
            return Ok((info.local, info.rendezvous));
        }
        if cap == Some(0) {
            return Err(
                "sim: a rendezvous channel (chan.bounded(0)) made outside the world \
                        cannot be used inside it; make it inside the world"
                    .to_string(),
            );
        }
        let local = st.chans.len() as u64 + 1;
        st.chans.insert(
            global,
            ChanInfo {
                local,
                rendezvous: false,
                sent: 0,
                taken: 0,
            },
        );
        Ok((local, false))
    }

    fn new_rendezvous(&self) -> Value {
        let channel = crate::stdlib::chan::sim_rendezvous();
        if let Ok((global, _)) = crate::stdlib::chan::sim_identity(&channel) {
            let mut st = self.lock();
            let local = st.chans.len() as u64 + 1;
            st.chans.insert(
                global,
                ChanInfo {
                    local,
                    rendezvous: true,
                    sent: 0,
                    taken: 0,
                },
            );
        }
        channel
    }

    fn chan_counts(&self, channel: &Value) -> (u64, u64) {
        let Ok((global, _)) = crate::stdlib::chan::sim_identity(channel) else {
            return (0, 0);
        };
        let st = self.lock();
        st.chans
            .get(&global)
            .map(|i| (i.sent, i.taken))
            .unwrap_or((0, 0))
    }

    /// A message went in: count it, log it, wake the channel's waiters.
    /// Its ticket (the count) is what a rendezvous sender waits on.
    fn sent(&self, me: TaskId, channel: &Value, local: u64, detail: String) -> u64 {
        let mut st = self.lock();
        let ticket = match crate::stdlib::chan::sim_identity(channel)
            .ok()
            .and_then(|(g, _)| st.chans.get_mut(&g))
        {
            Some(info) => {
                info.sent += 1;
                info.sent
            }
            None => 0,
        };
        self.event(&mut st, me, "send", format!("#{local} <- {detail}"));
        st.sched.wake(Wait::Chan(local));
        ticket
    }

    fn taken(&self, me: TaskId, channel: &Value, local: u64, v: &Value) {
        let mut st = self.lock();
        if let Some(info) = crate::stdlib::chan::sim_identity(channel)
            .ok()
            .and_then(|(g, _)| st.chans.get_mut(&g))
        {
            info.taken += 1;
        }
        self.event(&mut st, me, "recv", format!("#{local} -> {}", render(v)));
        st.sched.wake(Wait::Chan(local));
    }

    fn closed(&self, me: TaskId, local: u64, why: &str) {
        let mut st = self.lock();
        self.event(&mut st, me, "close", format!("#{local}{why}"));
        st.sched.wake(Wait::Chan(local));
    }

    fn note(&self, me: TaskId, kind: &'static str, detail: String) {
        let mut st = self.lock();
        self.event(&mut st, me, kind, detail);
    }

    // ─── tasks ───────────────────────────────────────────────────────

    fn task_local(&self, global: u64) -> Option<TaskId> {
        self.lock().tasks_by_global.get(&global).copied()
    }

    fn outcome(&self, t: TaskId) -> Option<Result<Value, String>> {
        self.lock().outcomes.get(t).cloned().flatten()
    }
}

// ─── The thread's context ────────────────────────────────────────────

#[derive(Clone)]
struct Ctx {
    world: Arc<World>,
    task: TaskId,
    node: NodeId,
    rng: Arc<Mutex<StdRng>>,
}

thread_local! {
    static IN_WORLD: Cell<bool> = const { Cell::new(false) };
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// While held, this thread is task `ctx.task` of `ctx.world`.
struct Entered {
    previous: Option<Ctx>,
}

fn enter(ctx: Ctx) -> Entered {
    let previous = CTX.with(|c| c.borrow_mut().replace(ctx));
    IN_WORLD.with(|w| w.set(true));
    Entered { previous }
}

impl Drop for Entered {
    fn drop(&mut self) {
        let previous = self.previous.take();
        IN_WORLD.with(|w| w.set(previous.is_some()));
        CTX.with(|c| *c.borrow_mut() = previous);
    }
}

fn current() -> Option<Ctx> {
    if !in_world() {
        return None;
    }
    CTX.with(|c| c.borrow().clone())
}

/// Is this thread running inside a simulated world? One thread-local
/// read: the whole cost of `sim` to a program outside a world.
#[inline]
pub fn in_world() -> bool {
    IN_WORLD.with(|w| w.get())
}

/// The world's wall clock, as epoch ms, when this thread is in a world:
/// as this task's node reads it (its skew included).
pub fn virtual_epoch_ms() -> Option<i64> {
    let ctx = current()?;
    Some(ctx.world.wall_ms(ctx.node))
}

/// This task's random stream, when this thread is in a world.
pub fn task_rng() -> Option<Arc<Mutex<StdRng>>> {
    if !in_world() {
        return None;
    }
    CTX.with(|c| c.borrow().as_ref().map(|ctx| ctx.rng.clone()))
}

/// Run `main` as the main program (task 0) of a new world, on this
/// thread, and report what happened. `main` runs with this thread in
/// the world; tasks it spawns run on threads of their own, one at a
/// time, as the seed picks.
pub fn run(
    cfg: Config,
    main: impl FnOnce() -> Result<Value, InterpreterError>,
) -> Result<Report, String> {
    run_main(cfg, NodeInfo::main(Vec::new()), main)
}

/// `run`, with the main program's node described (its argv, its env).
pub fn run_main(
    cfg: Config,
    info: NodeInfo,
    main: impl FnOnce() -> Result<Value, InterpreterError>,
) -> Result<Report, String> {
    if in_world() {
        return Err("sim: worlds do not nest: a world cannot run inside another".to_string());
    }
    let world = Arc::new(World::new(&cfg, vec![info], vec![Start::Main], None, 0));
    run_in(world, main)
}

/// Run `main` as task 0 of `world`, on this thread.
fn run_in(
    world: Arc<World>,
    main: impl FnOnce() -> Result<Value, InterpreterError>,
) -> Result<Report, String> {
    if in_world() {
        return Err("sim: worlds do not nest: a world cannot run inside another".to_string());
    }
    let rng = world.task_rng(0, 0);
    {
        let mut st = world.lock();
        let seed = world.seed;
        world.event(&mut st, 0, "start", format!("seed {seed}"));
    }
    let entered = enter(Ctx {
        world: world.clone(),
        task: 0,
        node: 0,
        rng,
    });
    // A panic must not leave the world's other threads waiting for a
    // baton that never comes: it is the main program's failure.
    let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(main)) {
        Ok(r) => r.map_err(|e| error_text(&e)),
        Err(_) => Err("sim: the main program panicked (an engine bug)".to_string()),
    };
    drop(entered);
    Ok(world.main_exit(result))
}

// ─── The dispatch hook ───────────────────────────────────────────────

fn misuse(message: impl Into<String>) -> InterpreterError {
    InterpreterError::runtime(message)
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

fn err(message: &str) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(message.to_string()))))
}

/// An error as a report says it: a runtime error's own message.
fn error_text(e: &InterpreterError) -> String {
    match e {
        InterpreterError::RuntimeError { message } => message.clone(),
        other => other.to_string(),
    }
}

/// A value, short enough for an event's detail.
fn render(v: &Value) -> String {
    let text = v.to_string();
    if text.chars().count() > 120 {
        let cut: String = text.chars().take(117).collect();
        format!("{cut}...")
    } else {
        text
    }
}

/// The error a refused effect raises inside a world.
pub fn refusal(name: &str) -> String {
    let what = match name.split('.').next().unwrap_or(name) {
        "fs" | "asset" | "testing" => "the filesystem",
        "http" => "the network",
        "db" => "the database",
        "proc" | "pty" => "processes",
        "os" => "the operating system (environment, processes, console)",
        "tty" | "gui" => "the terminal and windows",
        "runtime" => "the runtime's modules and profiler",
        "crypto" => "the operating system's entropy",
        "task" => "real threads and real elapsed time",
        "sim" => {
            return format!(
                "sim: {name} is refused inside a world: worlds do not nest (one world per run in S1)"
            );
        }
        _ => "the real world",
    };
    format!(
        "sim: {name} is refused inside a world: {what} is not simulated yet (a world \
         simulates time, randomness, spawn, task, chan, and http between its nodes), and \
         a world never reaches the real one"
    )
}

/// Called from builtin dispatch for every builtin while this thread is
/// in a world: `Some(answer)` when the world answers or refuses, `None`
/// to run the builtin as usual (pure calls, and the calls whose own
/// implementation already reads the world's clock or random stream).
pub fn intercept(
    name: &str,
    args: &[Value],
    interpreter: &mut Interpreter,
) -> Option<Result<Value, InterpreterError>> {
    let ctx = current()?;
    match crate::effects::sim_handling(name) {
        SimHandling::Pure => {
            if matches!(name, "print" | "println") {
                let text = args
                    .iter()
                    .map(|a| match a {
                        Value::String(s) => s.to_string(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                ctx.world.note(ctx.task, "print", text);
            }
            None
        }
        SimHandling::Refused => Some(Err(misuse(refusal(name)))),
        SimHandling::Simulated => simulated(&ctx, name, args, interpreter),
    }
}

fn simulated(
    ctx: &Ctx,
    name: &str,
    args: &[Value],
    interpreter: &mut Interpreter,
) -> Option<Result<Value, InterpreterError>> {
    let w = &ctx.world;
    let me = ctx.task;
    if let Some(verb) = name.strip_prefix("http.") {
        return http::call(ctx, verb, args, interpreter);
    }
    Some(match name {
        "time.now_ms" if args.is_empty() => Ok(Value::Integer(w.wall_ms(ctx.node))),
        "os.args" if args.is_empty() => Ok(Value::List(Arc::new(
            w.nodes[ctx.node]
                .argv
                .iter()
                .map(|a| Value::String(Arc::new(a.clone())))
                .collect(),
        ))),
        "os.get_env" => match args {
            [Value::String(key)] => Ok(match w.nodes[ctx.node].env.get(key.as_str()) {
                Some(v) => ok(Value::String(Arc::new(v.clone()))),
                None => err(&format!("Environment variable '{key}' not found")),
            }),
            _ => Err(misuse("os.get_env expects the variable's name as a String")),
        },
        "os.has_env" => match args {
            [Value::String(key)] => Ok(Value::Boolean(
                w.nodes[ctx.node].env.contains_key(key.as_str()),
            )),
            _ => Err(misuse("os.has_env expects the variable's name as a String")),
        },
        "time.monotonic_ms" if args.is_empty() => Ok(Value::Integer(w.now() as i64)),
        "time.monotonic" if args.is_empty() => Ok(Value::Float(w.now() as f64)),
        "time.sleep" => sleep(ctx, args),
        "chan.bounded" if matches!(args, [Value::Integer(0)]) => Ok(w.new_rendezvous()),
        "chan.send" => match args {
            [c, v] => send(ctx, c, v.clone()),
            _ => Err(misuse("chan.send expects a channel and a value")),
        },
        "chan.recv" => match args {
            [c] => recv(ctx, c, None, "chan.recv"),
            _ => Err(misuse("chan.recv expects a channel")),
        },
        "chan.recv_timeout" => match args {
            [c, limit] => match crate::stdlib::timeout_ms_arg(limit) {
                Some(ms) => recv(ctx, c, Some(ms), "chan.recv_timeout"),
                None => Err(misuse(
                    "chan.recv_timeout expects a channel and a non-negative number of \
                     milliseconds (or #{ timeout_ms })",
                )),
            },
            _ => Err(misuse(
                "chan.recv_timeout expects a channel and a number of milliseconds",
            )),
        },
        "chan.try_recv" => match args {
            [c] => try_recv(ctx, c),
            _ => Err(misuse("chan.try_recv expects a channel")),
        },
        "chan.ask" => match args {
            [service, request] => ask(ctx, service, request.clone()),
            _ => Err(misuse("chan.ask expects a service channel and a request")),
        },
        "chan.close" => match args {
            [c] => close(ctx, c),
            _ => Err(misuse("chan.close expects a channel")),
        },
        "task.join" => match args {
            [t] => join(ctx, t, None),
            _ => Err(misuse("task.join expects one argument: the task")),
        },
        "task.join_timeout" => match args {
            [t, limit] => match crate::stdlib::timeout_ms_arg(limit) {
                Some(ms) => join(ctx, t, Some(ms)),
                None => Err(misuse(
                    "task.join_timeout expects a task and a non-negative number of \
                     milliseconds (or #{ timeout_ms })",
                )),
            },
            _ => Err(misuse(
                "task.join_timeout expects a task and a number of milliseconds",
            )),
        },
        // The rest run their own implementation, which reads the world:
        // `chan.new`/`chan.stat`/`chan.bounded(n)` make or read a channel,
        // `random.*`, `crypto.random_*` and `dates.*` draw from the task's
        // stream and the world's clock, `par_map`/`par_filter` run in
        // order on this thread, `task.watch` notes the channel.
        _ => {
            let _ = me;
            return None;
        }
    })
}

fn sleep(ctx: &Ctx, args: &[Value]) -> Result<Value, InterpreterError> {
    let ms = match args {
        [Value::Integer(n)] if *n >= 0 => *n as u64,
        [Value::Integer(_)] => return Err(misuse("sleep: milliseconds must be non-negative")),
        [other] => {
            return Err(misuse(format!(
                "sleep: milliseconds must be an integer, got {}",
                other.type_name()
            )));
        }
        _ => {
            return Err(misuse(format!(
                "sleep expects 1 argument (ms), got {}",
                args.len()
            )));
        }
    };
    let w = &ctx.world;
    w.note(ctx.task, "sleep", format!("{ms} ms"));
    if ms == 0 {
        w.yield_point(ctx.task)?;
    } else {
        let until = w.now() + ms;
        w.block(
            ctx.task,
            Wait::Sleep,
            format!("time.sleep({ms})"),
            Some(until),
        )?;
    }
    Ok(Value::Unit)
}

fn send(ctx: &Ctx, channel: &Value, value: Value) -> Result<Value, InterpreterError> {
    if let Some(kind) = crate::stdlib::cell::confined_within(&value) {
        return Err(misuse(format!(
            "chan.send: a {kind} cannot be sent through a channel — it belongs to the \
             thread that created it. Send what it holds instead"
        )));
    }
    let w = &ctx.world;
    let me = ctx.task;
    let (local, rendezvous) = w.chan_info(channel).map_err(misuse)?;
    let detail = render(&value);
    w.yield_point(me)?;
    let mut value = value;
    loop {
        match crate::stdlib::chan::sim_try_send(channel, value).map_err(misuse)? {
            crate::stdlib::chan::SimSend::Sent => {
                let ticket = w.sent(me, channel, local, detail);
                // A rendezvous send is done when a receiver has its message.
                while rendezvous && w.chan_counts(channel).1 < ticket {
                    w.block(
                        me,
                        Wait::Chan(local),
                        format!("chan.send on rendezvous channel #{local} (no receiver yet)"),
                        None,
                    )?;
                }
                return Ok(ok(Value::Unit));
            }
            crate::stdlib::chan::SimSend::Full(back) => {
                value = back;
                w.block(
                    me,
                    Wait::Chan(local),
                    format!("chan.send on full channel #{local}"),
                    None,
                )?;
            }
            crate::stdlib::chan::SimSend::Closed => return Ok(err("channel is closed")),
        }
    }
}

fn recv(
    ctx: &Ctx,
    channel: &Value,
    timeout: Option<u64>,
    op: &str,
) -> Result<Value, InterpreterError> {
    let w = &ctx.world;
    let me = ctx.task;
    let (local, _) = w.chan_info(channel).map_err(misuse)?;
    w.yield_point(me)?;
    let until = timeout.map(|ms| w.now() + ms);
    loop {
        match crate::stdlib::chan::sim_try_recv(channel).map_err(misuse)? {
            crate::stdlib::chan::SimRecv::Got(v) => {
                w.taken(me, channel, local, &v);
                return Ok(ok(v));
            }
            crate::stdlib::chan::SimRecv::Closed => return Ok(err("channel is closed")),
            crate::stdlib::chan::SimRecv::Empty => {
                if let Some(u) = until
                    && w.now() >= u
                {
                    w.note(me, "timeout", format!("{op} on #{local}"));
                    return Ok(err("timed out"));
                }
                w.block(
                    me,
                    Wait::Chan(local),
                    format!("{op} on channel #{local}"),
                    until,
                )?;
            }
        }
    }
}

fn try_recv(ctx: &Ctx, channel: &Value) -> Result<Value, InterpreterError> {
    let w = &ctx.world;
    let (local, _) = w.chan_info(channel).map_err(misuse)?;
    w.yield_point(ctx.task)?;
    Ok(
        match crate::stdlib::chan::sim_try_recv(channel).map_err(misuse)? {
            crate::stdlib::chan::SimRecv::Got(v) => {
                w.taken(ctx.task, channel, local, &v);
                ok(v)
            }
            crate::stdlib::chan::SimRecv::Empty => ok(Value::Unit),
            crate::stdlib::chan::SimRecv::Closed => err("channel is closed"),
        },
    )
}

fn ask(ctx: &Ctx, service: &Value, request: Value) -> Result<Value, InterpreterError> {
    if let Some(kind) = crate::stdlib::cell::confined_within(&request) {
        return Err(misuse(format!(
            "chan.ask: a {kind} cannot be sent through a channel — send what it holds instead"
        )));
    }
    let reply = crate::stdlib::chan::fresh_channel();
    let mut message = crate::ast::ValueMap::default();
    message.insert("req".to_string(), request);
    message.insert("reply".to_string(), reply.clone());
    let sent = send(ctx, service, Value::Map(Arc::new(message)))?;
    if matches!(sent, Value::Err(_)) {
        return Ok(sent);
    }
    recv(ctx, &reply, None, "chan.ask")
}

fn close(ctx: &Ctx, channel: &Value) -> Result<Value, InterpreterError> {
    let w = &ctx.world;
    let (local, _) = w.chan_info(channel).map_err(misuse)?;
    w.yield_point(ctx.task)?;
    crate::stdlib::chan::close_value(channel);
    w.closed(ctx.task, local, "");
    Ok(Value::Unit)
}

/// A channel `task.watch` tied to a task that has just ended.
fn close_watched(world: &World, me: TaskId, channel: &Value) {
    crate::stdlib::chan::close_value(channel);
    if let Ok((local, _)) = world.chan_info(channel) {
        world.closed(me, local, " (its task ended)");
    }
}

fn join(ctx: &Ctx, task: &Value, timeout: Option<u64>) -> Result<Value, InterpreterError> {
    let w = &ctx.world;
    let me = ctx.task;
    let global = crate::stdlib::task::task_id(task).map_err(misuse)?;
    let Some(t) = w.task_local(global) else {
        return Err(misuse(
            "sim: this task was spawned outside the world, and a world joins only its own",
        ));
    };
    w.yield_point(me)?;
    let until = timeout.map(|ms| w.now() + ms);
    loop {
        if let Some(outcome) = w.outcome(t) {
            w.note(me, "join", format!("task {t}"));
            let settled = crate::stdlib::task::settle(outcome);
            return Ok(match timeout {
                Some(_) => ok(settled),
                None => settled,
            });
        }
        if let Some(u) = until
            && w.now() >= u
        {
            w.note(me, "timeout", format!("task.join_timeout on task {t}"));
            return Ok(err("timed out"));
        }
        w.block(me, Wait::Task(t), format!("task.join on task {t}"), until)?;
    }
}

/// `spawn expr` inside a world: a task of the world, on a thread of its
/// own that runs only while it holds the baton.
pub fn spawn(interpreter: &Interpreter, expr: &Expr) -> Result<Value, InterpreterError> {
    let Some(ctx) = current() else {
        return Err(misuse(
            "sim: spawn outside a world reached the world's spawn",
        ));
    };
    World::stopped(&ctx.world.lock(), ctx.task)?;
    let mut worker = interpreter.thread_safe_clone();
    let expr = expr.clone();
    let global = crate::interpreter::spawn_registry::next_id();
    let (_, handle) = spawn_task(
        &ctx.world,
        ctx.task,
        ctx.node,
        Some(global),
        "",
        move || worker.eval_expr(&expr).map_err(|e| e.to_string()),
    )?;
    crate::interpreter::spawn_registry::register(global, handle);
    let value = crate::stdlib::task::handle(global);
    ctx.world.yield_point(ctx.task)?;
    Ok(value)
}

/// A task of `world` on `node`, spawned by `parent`, running `body` on a
/// thread of its own once the seed first picks it. `global` is the
/// interpreter's number for an olang `spawn` (its `task.watch`ed
/// channels close as it ends); the world's own tasks have none.
fn spawn_task<F>(
    world: &Arc<World>,
    parent: TaskId,
    node: NodeId,
    global: Option<u64>,
    what: &str,
    body: F,
) -> Result<(TaskId, std::thread::JoinHandle<Result<Value, String>>), InterpreterError>
where
    F: FnOnce() -> Result<Value, String> + Send + 'static,
{
    let (local, rng) = world.add_task(parent, node, global, what);
    let world = world.clone();
    let handle = std::thread::Builder::new()
        .name(format!("olang-sim-{local}"))
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let _entered = enter(Ctx {
                world: world.clone(),
                task: local,
                node,
                rng,
            });
            let _counted = crate::memory::enter_task(&format!("olang-sim-{local}"));
            let outcome = match world.wait_first_turn(local) {
                Ok(()) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
                    .unwrap_or_else(|_| Err("task thread panicked".to_string())),
                Err(e) => Err(InterpreterError::from(e).to_string()),
            };
            // `task.watch`ed channels close as the task ends, under its
            // baton, so their waiters see it in order.
            if let Some(global) = global {
                for channel in crate::interpreter::spawn_registry::finish(global) {
                    close_watched(&world, local, &channel);
                }
            }
            world.task_exit(local, outcome.clone());
            outcome
        })
        .map_err(|e| misuse(format!("spawn: could not start thread: {e}")))?;
    Ok((local, handle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleep_ms(ms: i64) -> Result<Value, InterpreterError> {
        let mut i = Interpreter::new();
        intercept("time.sleep", &[Value::Integer(ms)], &mut i).expect("a world answers time.sleep")
    }

    #[test]
    fn outside_a_world_nothing_is_intercepted() {
        assert!(!in_world());
        assert_eq!(virtual_epoch_ms(), None);
        assert!(task_rng().is_none());
        let mut i = Interpreter::new();
        assert!(intercept("time.sleep", &[Value::Integer(1)], &mut i).is_none());
    }

    #[test]
    fn the_clock_is_virtual_inside_a_world_and_real_after_it() {
        let started = crate::clock::Instant::now();
        let cfg = Config {
            seed: 3,
            start_ms: 5_000,
            ..Config::default()
        };
        let report = run(cfg, || {
            assert_eq!(virtual_epoch_ms(), Some(5_000));
            assert_eq!(crate::clock::epoch_ms(), 5_000);
            sleep_ms(3_600_000)?;
            assert_eq!(crate::clock::epoch_ms(), 5_000 + 3_600_000);
            Ok(Value::Integer(crate::clock::epoch_ms()))
        })
        .unwrap();
        assert_eq!(report.outcome, "done");
        assert_eq!(report.now, 3_600_000);
        assert_eq!(report.value, Some(Value::Integer(3_605_000)));
        assert!(started.elapsed().as_secs() < 5);
        assert!(!in_world());
        assert!(crate::clock::epoch_ms() > DEFAULT_START_MS);
    }

    #[test]
    fn a_deadline_ends_the_world_and_the_main_program_sees_it() {
        let cfg = Config {
            run_for: Some(1_000),
            ..Config::default()
        };
        let report = run(cfg, || {
            sleep_ms(400)?;
            sleep_ms(400)?;
            let after = sleep_ms(400);
            assert!(after.is_err(), "the third sleep crosses the deadline");
            after
        })
        .unwrap();
        assert_eq!(report.outcome, "deadline");
        assert_eq!(report.now, 1_000);
    }

    #[test]
    fn refused_effects_never_run() {
        let report = run(Config::default(), || {
            let mut i = Interpreter::new();
            let answer =
                intercept("fs.write_file", &[], &mut i).expect("refused, not passed through");
            assert!(
                answer
                    .unwrap_err()
                    .to_string()
                    .contains("refused inside a world")
            );
            assert!(
                intercept("str.trim", &[], &mut i).is_none(),
                "pure calls run as usual"
            );
            Ok(Value::Unit)
        })
        .unwrap();
        assert_eq!(report.outcome, "done");
    }

    #[test]
    fn the_step_budget_ends_a_livelock() {
        let cfg = Config {
            max_steps: 50,
            ..Config::default()
        };
        let report = run(cfg, || {
            loop {
                sleep_ms(0)?;
            }
        })
        .unwrap();
        assert_eq!(report.outcome, "step_limit");
        assert!(report.failed());
    }

    #[test]
    fn worlds_do_not_nest() {
        let report = run(Config::default(), || {
            assert!(run(Config::default(), || Ok(Value::Unit)).is_err());
            Ok(Value::Unit)
        })
        .unwrap();
        assert_eq!(report.outcome, "done");
    }

    #[test]
    fn each_task_draws_from_its_own_seeded_stream() {
        let draw = |seed| {
            run(
                Config {
                    seed,
                    ..Config::default()
                },
                || {
                    let rng = task_rng().expect("a world's stream");
                    let mut rng = rng.lock().unwrap();
                    Ok(Value::Integer(rand::Rng::next_u32(&mut *rng) as i64))
                },
            )
            .unwrap()
            .value
        };
        assert_eq!(draw(1), draw(1));
        assert_ne!(draw(1), draw(2));
    }
}

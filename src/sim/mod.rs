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
//! - **The disk and crashes** (S3, plans/SIM-S3.md): each node has an
//!   in-memory disk whose files keep what was synced apart from what was
//!   only written (`disk.rs`); `fs.*` answers from it (`files.rs`), and
//!   SQLite reaches it through a VFS that can lie about `fsync`, tear
//!   pages, fill up and slow down (`vfs.rs`, `dbs.rs`). A crash keeps the
//!   disk as it really was; a node has its own environment, stdin,
//!   shutdown handlers and exit (`life.rs`).

pub mod cli;
mod dbs;
pub mod disk;
mod files;
pub mod http;
mod life;
pub mod lin;
pub mod net;
mod node;
pub mod plan;
pub mod props;
pub mod sched;
pub mod shrink;
pub mod surface;
mod vfs;
pub mod view;
mod wire;

pub use node::{CrashPlan, NodeInfo, Plan, Start, run_world, run_world_main};
pub use wire::{History, NetStats};

use crate::ast::{Expr, Value};
use crate::effects::SimHandling;
use crate::interpreter::{Interpreter, InterpreterError};
use net::NodeId;
use rand::SeedableRng;
use rand::rngs::StdRng;
use sched::{End, Pick, Sched, TaskId, Wait, Woke};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
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
    /// S4: the fault decisions to force instead of drawing (`plan.rs`).
    pub forced: Option<plan::RunPlan>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            seed: 0,
            start_ms: DEFAULT_START_MS,
            run_for: None,
            max_steps: DEFAULT_MAX_STEPS,
            net: net::Link::default(),
            forced: None,
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
    /// S5: a message's other end. On an arrival (`deliver`, `late`,
    /// `refused`, a drop in flight or at a node that is down) the node it
    /// came from and the virtual ms it was sent; on a drop or a duplicate
    /// when sent, the node it was for and the same ms as `t`. What
    /// `olang sim view` draws as an arrow; not part of the digest.
    pub link: Option<Link>,
    /// S5: the event is a fault the world injected (`run.faults` has it
    /// too). Not part of the digest (the faults are).
    pub fault: bool,
}

/// The other end of a message an event is about (`Event::link`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub peer: Arc<str>,
    pub sent: u64,
    /// An arrival at the event's node (from `peer`), not a message lost
    /// or copied as it left it (for `peer`).
    pub inbound: bool,
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
    /// Every fault the world injected (crashes, kills, stops, lying
    /// syncs, full disks, buggify points that fired), in order: what
    /// S4's shrinker will minimise.
    pub faults: Vec<Fault>,
    /// Each node's disk, as the run left it: (node, what it did).
    pub disks: Vec<(String, disk::DiskStats)>,
    /// S4: every fault that fired, as a plan that forces this run again
    /// (`plan.rs`), with the nodes it left out and its schedule.
    pub plan: plan::RunPlan,
    /// The property that broke, when one did (`props.rs`).
    pub violation: Option<props::Violation>,
    /// Yield points at which a task that could have gone on lost the baton.
    pub preemptions: u64,
    /// What `sim.record` recorded, by name (`run.history(name)`).
    pub histories: BTreeMap<String, Arc<Vec<Value>>>,
    /// The nodes the world-builder marked `optional` (the shrinker may
    /// leave them out).
    pub optional: Vec<String>,
    /// Which world run of its file this is (0 first): a plan file forces
    /// each run of a file by its number.
    pub run_index: usize,
    /// Where the failure was raised (`file:line:col`, or `line:col` in
    /// the entry file), when it was an error the interpreter located.
    pub site: Option<String>,
}

/// A fault the world injected: when, where, what, and the yield point
/// it fell at (the scheduler's step count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub t: u64,
    pub node: Arc<str>,
    pub kind: &'static str,
    pub detail: String,
    pub step: u64,
    /// The visit it fired on, when the fault is one a plan can force (a
    /// full disk is a consequence, not a decision: none).
    pub key: Option<plan::Key>,
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
            faults: Vec::new(),
            disks: Vec::new(),
            plan: plan::RunPlan::default(),
            violation: None,
            preemptions: 0,
            histories: BTreeMap::new(),
            optional: Vec::new(),
            run_index: 0,
            site: None,
        }
    }

    /// Whether this run is a failure worth a seed: the main program
    /// raised, every task deadlocked, the run never settled, or a
    /// property broke.
    pub fn failed(&self) -> bool {
        matches!(
            self.outcome,
            "failed" | "deadlock" | "step_limit" | "violated"
        )
    }

    /// The faults a plan can force (what the shrinker counts).
    pub fn plannable_faults(&self) -> usize {
        self.faults.iter().filter(|f| f.key.is_some()).count()
    }

    /// What kind of failure this is, the same however the run got there,
    /// for the shrinker, which keeps a change only when the failure keeps
    /// its signature. A broken property is its name. Anything else is the
    /// outcome, the place the error was raised (`at file:line:col`, when
    /// the interpreter located it), and the shape of the error's first
    /// line ([`shape`]): numbers, quoted text and bracketed values set
    /// aside, since they vary from run to run of one bug (a list of keys,
    /// a count, an id), while the words around them and the site do not.
    pub fn signature(&self) -> String {
        if let Some(v) = &self.violation {
            return format!("property {}", v.property);
        }
        let text = self.error.as_deref().unwrap_or("");
        let first = shape(text.lines().next().unwrap_or(""));
        match &self.site {
            Some(site) => format!("{} at {site}: {first}", self.outcome),
            None => format!("{}: {first}", self.outcome),
        }
    }

    /// A hash of everything the run did (events, faults, outcome, time,
    /// steps), the same on every machine and in every process: two runs
    /// are the same run when their digests are equal.
    pub fn digest(&self) -> u64 {
        let mut h = Fnv::new();
        h.str(self.outcome);
        h.u64(self.now);
        h.u64(self.steps);
        h.str(self.error.as_deref().unwrap_or(""));
        for e in &self.events {
            h.u64(e.t);
            h.str(&e.node);
            h.u64(e.task.map(|t| t as u64 + 1).unwrap_or(0));
            h.str(e.kind);
            h.str(&e.detail);
        }
        for f in &self.faults {
            h.u64(f.t);
            h.str(&f.node);
            h.str(f.kind);
            h.str(&f.detail);
            h.u64(f.step);
        }
        if let Some(v) = &self.violation {
            h.str(&v.property);
            h.u64(v.t);
            h.u64(v.step);
        }
        h.0
    }
}

/// An error message with what varies taken out: a run of digits is `#`,
/// text in double quotes is `"…"`, and what a balanced `(...)`, `[...]`
/// or `{...}` holds is `…` (the brackets stay). `client c3: expected
/// [1, 4] for "k-17"` and `client c1: expected [2] for "k-9"` have one
/// shape, `client c#: expected […] for "…"`.
pub fn shape(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            out.push('#');
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            continue;
        }
        let close = match c {
            '"' => Some('"'),
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            _ => None,
        };
        if let Some(close) = close
            && let Some(end) = matching(&chars, i, close)
        {
            out.push(c);
            if end > i + 1 {
                out.push('…');
            }
            out.push(close);
            i = end + 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Where the bracket (or quote) at `open` closes, nesting counted.
fn matching(chars: &[char], open: usize, close: char) -> Option<usize> {
    let first = chars[open];
    if first == '"' {
        return (open + 1..chars.len()).find(|&j| chars[j] == '"');
    }
    let mut depth = 0usize;
    let mut quoted = false;
    for (j, &c) in chars.iter().enumerate().skip(open) {
        if c == '"' {
            quoted = !quoted;
        } else if quoted {
        } else if c == first {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(j);
            }
        }
    }
    None
}

/// `r`, with the place its error was raised kept for the failure's
/// signature (`Report::site`), when `interpreter` located it.
pub(crate) fn sited<T>(
    interpreter: &mut Interpreter,
    r: Result<T, InterpreterError>,
) -> Result<T, InterpreterError> {
    if r.is_err()
        && let Some(site) = error_site(interpreter)
        && let Some(ctx) = current()
    {
        ctx.world.lock().error_sites.insert(ctx.task, site);
    }
    r
}

/// Where `interpreter`'s last error was raised: `file:line:col` (the
/// file's name alone), or `line:col` in its entry file.
pub(crate) fn error_site(interpreter: &mut Interpreter) -> Option<String> {
    let loc = interpreter.take_error_location()?;
    if loc.line == 0 {
        return None;
    }
    Some(match loc.file.as_deref() {
        Some(f) => {
            let name = std::path::Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| f.to_string());
            format!("{name}:{}:{}", loc.line, loc.column)
        }
        None => format!("{}:{}", loc.line, loc.column),
    })
}

/// FNV-1a, 64 bits: a digest that is the same everywhere.
pub(crate) struct Fnv(pub u64);

impl Fnv {
    pub fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    pub fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 = (self.0 ^ *x as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    pub fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.bytes(s.as_bytes());
    }
    pub fn u64(&mut self, n: u64) {
        self.bytes(&n.to_le_bytes());
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
    /// Tasks of a node that crashed, was killed or exited: each unwinds
    /// at its next yield point, with why.
    killed: Vec<Option<Ended>>,
    nodes: Vec<node::NodeState>,
    net: net::Net,
    wire: wire::Wire,
    /// A node's program (or a client, or a hook) raised: the run fails
    /// with this once the world's coordinator sees it.
    failure: Option<String>,
    faults: Vec<Fault>,
    /// `sim.maybe`: which points are on in this world, and the stream
    /// that decides each time one is reached.
    maybe_on: BTreeMap<String, bool>,
    maybe_draws: sched::Keyed,
    /// Connections opened in this world: owner → (node, the disk's boot).
    db_owners: BTreeMap<u64, (NodeId, u64)>,
    /// `db.transaction` in progress: connection → (task, depth).
    tx: BTreeMap<i64, (TaskId, usize)>,
    /// S4: the world's fault sites (code, buggify points, the network);
    /// each disk keeps its own (`vfs::NodeDisk`).
    sites: plan::Sites,
    /// `sim.record`: each history, as the olang values `run.history`
    /// answers (shared, so a property reads one without copying it).
    histories: BTreeMap<String, Arc<Vec<Value>>>,
    record_seq: u64,
    /// The property that broke.
    violation: Option<props::Violation>,
    /// Tasks each node has started so far: a node's k-th task draws from
    /// its own k-th stream, whatever the other nodes do (so leaving an
    /// optional node out moves no other node's draws).
    node_tasks: Vec<u64>,
    /// Connections `db.open` made in this world (each one's `id`).
    db_opens: u64,
    /// Where each task's last error was raised (`note_error_site`), and
    /// where the error that failed the run was.
    error_sites: HashMap<TaskId, String>,
    failure_site: Option<String>,
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
    /// Each node's disk (and SQLite's memory beside it), apart from the
    /// state's lock: SQLite reaches it from inside a `db` call
    /// (`vfs.rs`). The order is always state, then disk.
    disks: Vec<vfs::Shared>,
    /// `os.on_shutdown` handlers per node: each with the interpreter
    /// clone it registered.
    handlers: Mutex<BTreeMap<NodeId, Vec<(Interpreter, Value)>>>,
    /// The virtual clock, readable without the state's lock (a disk's
    /// fault sites key their visits by it).
    clock: Arc<std::sync::atomic::AtomicU64>,
    /// S4: the properties checked as the world runs (`props.rs`).
    checker: Option<Mutex<props::Checker>>,
    /// Optional nodes the plan leaves out: never started.
    dropped: Vec<bool>,
}

impl Drop for World {
    fn drop(&mut self) {
        for d in &self.disks {
            vfs::unregister(vfs::lock(d).key);
        }
    }
}

/// The task cannot go on: the world has ended, or its node crashed, was
/// killed, or exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    World,
    Crash,
    Kill,
    Exit,
}

pub const ENDED_MESSAGE: &str = "sim: the world has ended, and this task stops with it";
pub const CRASHED_MESSAGE: &str = "sim: this node crashed, and its tasks stop with it";
pub const KILLED_MESSAGE: &str = "sim: this node was killed, and its tasks stop with it";
pub const EXITED_MESSAGE: &str = "sim: this node's process ended, and its tasks stop with it";

impl Ended {
    fn message(self) -> &'static str {
        match self {
            Ended::World => ENDED_MESSAGE,
            Ended::Crash => CRASHED_MESSAGE,
            Ended::Kill => KILLED_MESSAGE,
            Ended::Exit => EXITED_MESSAGE,
        }
    }
}

impl From<Ended> for InterpreterError {
    fn from(e: Ended) -> Self {
        InterpreterError::runtime(e.message())
    }
}

impl World {
    fn new(
        cfg: &Config,
        nodes: Vec<NodeInfo>,
        starts: Vec<Start>,
        template: Option<Interpreter>,
        spec: u64,
        props: Vec<props::Property>,
    ) -> World {
        let mut sched = Sched::new(sched::split(cfg.seed, 1));
        sched.deadline = cfg.run_for;
        sched.max_steps = cfg.max_steps;
        let node_states = nodes.iter().map(node::NodeState::new).collect();
        let clock = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let nodes_len = nodes.len();
        // A forced run: each site answers from the plan (`plan.rs`).
        let node_id = |name: &str| nodes.iter().position(|n| &*n.name == name);
        let resolved: Vec<(plan::Site, u64, u64, u64, Option<u64>)> = cfg
            .forced
            .iter()
            .flat_map(|p| p.faults.iter())
            .filter_map(|e| {
                let site = plan::Site::from_name(&e.site)?;
                let place = plan::place_of(site, &e.at, &node_id)?;
                Some((site, place, e.t, e.k, e.delay))
            })
            .collect();
        let disk_site = |s: plan::Site| matches!(s, plan::Site::CrashPoint | plan::Site::FsyncLie);
        let sites_for = |node: Option<usize>| match &cfg.forced {
            None => plan::Sites::drawn(),
            Some(_) => plan::Sites::forced(resolved.iter().copied().filter(|e| match node {
                Some(n) => disk_site(e.0) && e.1 == n as u64,
                None => !disk_site(e.0),
            })),
        };
        let dropped: Vec<bool> = nodes
            .iter()
            .map(|n| {
                n.optional
                    && cfg
                        .forced
                        .as_ref()
                        .is_some_and(|p| p.drop.iter().any(|d| d == &*n.name))
            })
            .collect();
        if let Some(p) = &cfg.forced
            && p.stick > 0.0
        {
            sched.set_stick(p.stick, sched::split(cfg.seed, 7));
        }
        if !props.is_empty() {
            sched.checking = true;
        }
        let checker = (!props.is_empty()).then(|| {
            Mutex::new(props::Checker::new(
                props,
                template.as_ref().map(|t| t.thread_safe_clone()),
            ))
        });
        let disks = nodes
            .iter()
            .enumerate()
            .map(|(i, info)| {
                let mut d = disk::Disk::new(info.disk);
                // every machine has a /tmp (os.temp_dir)
                d.seed_dir("/tmp");
                for (path, bytes) in &info.files {
                    d.seed_file(&disk::normalize(&info.cwd, path), bytes.clone());
                }
                let crash_p = info.crash.map(|c| c.p).unwrap_or(0.0);
                let seed = sched::split(sched::split(cfg.seed, 3), i as u64);
                let mut node_disk = vfs::NodeDisk::new(d, seed, crash_p);
                node_disk.place = i as u64;
                node_disk.restart_after = info.crash.and_then(|c| c.restart_after);
                node_disk.sites = sites_for(Some(i));
                node_disk.clock = clock.clone();
                let nd = Arc::new(Mutex::new(node_disk));
                vfs::register(&nd);
                nd
            })
            .collect();
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
                killed: vec![None],
                nodes: node_states,
                net: net::Net::new(sched::split(cfg.seed, 2), cfg.net),
                wire: wire::Wire::default(),
                failure: None,
                faults: Vec::new(),
                maybe_on: BTreeMap::new(),
                maybe_draws: sched::Keyed::new(sched::split(cfg.seed, 5)),
                db_owners: BTreeMap::new(),
                tx: BTreeMap::new(),
                sites: sites_for(None),
                histories: BTreeMap::new(),
                record_seq: 0,
                violation: None,
                node_tasks: vec![0; nodes_len],
                db_opens: 0,
                error_sites: HashMap::new(),
                failure_site: None,
            }),
            disks,
            handlers: Mutex::new(BTreeMap::new()),
            clock,
            checker,
            dropped,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Task `t`'s own random stream, split from the world's seed by
    /// (node, task). Node 0's streams are S1's (seed, task) ones; another
    /// node's `t` is the task's number within its node.
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
            link: None,
            fault: false,
        });
    }

    /// Something the network did on its own, at `node`, to a message
    /// whose other end is `peer`, sent at `sent` (`Event::link`).
    fn link_event(
        &self,
        st: &mut State,
        node: NodeId,
        kind: &'static str,
        detail: String,
        link: Option<(NodeId, u64, bool)>,
    ) {
        st.events.push(Event {
            t: st.sched.now,
            node: self.nodes[node].name.clone(),
            task: None,
            kind,
            detail,
            link: link.map(|(peer, sent, inbound)| Link {
                peer: self.nodes[peer].name.clone(),
                sent,
                inbound,
            }),
            fault: false,
        });
    }

    /// Choose who runs next (`Sched::next`), handling every delivery
    /// that falls due on the way.
    ///
    /// With properties, the scheduler stops before the clock moves on (and
    /// before the world ends) to have them checked: the lock is let go
    /// while they run (the baton stays here, so nothing else runs), and a
    /// property that breaks ends the world.
    fn pick<'a>(
        &'a self,
        mut st: MutexGuard<'a, State>,
    ) -> (MutexGuard<'a, State>, Result<TaskId, End>) {
        let mut counted = true;
        let prev = st.running;
        loop {
            match st.sched.next(counted, prev) {
                Err(end) => return (st, Err(end)),
                Ok(Pick::Run(t)) => {
                    self.clock
                        .store(st.sched.now, std::sync::atomic::Ordering::Relaxed);
                    return (st, Ok(t));
                }
                Ok(Pick::Deliver(id)) => {
                    counted = false;
                    self.clock
                        .store(st.sched.now, std::sync::atomic::Ordering::Relaxed);
                    self.deliver(&mut st, id);
                }
                Ok(Pick::Check) => {
                    counted = false;
                    let (now, step) = (st.sched.now, st.sched.steps);
                    drop(st);
                    let broke = self.check_props(now, step, false);
                    st = self.lock();
                    if let Some(v) = broke {
                        st.violation = Some(v);
                        return (st, Err(End::Violated));
                    }
                }
            }
        }
    }

    /// Why `me` cannot go on, if it cannot.
    fn stopped(st: &State, me: TaskId) -> Result<(), Ended> {
        if st.ended.is_some() {
            Err(Ended::World)
        } else if let Some(why) = st.killed.get(me).copied().flatten() {
            Err(why)
        } else {
            Ok(())
        }
    }

    /// A fault, recorded (and as an event of the node it struck: of the
    /// task that caused it when that task is the node's own).
    fn fault(
        &self,
        st: &mut State,
        task: Option<TaskId>,
        node: NodeId,
        kind: &'static str,
        detail: String,
    ) {
        self.fault_at(st, task, node, kind, detail, None)
    }

    /// A fault the world records without an event of its own (the
    /// network's drops and duplicates, a partition: each has its event).
    fn record_fault(
        &self,
        st: &mut State,
        node: NodeId,
        kind: &'static str,
        detail: String,
        key: Option<plan::Key>,
    ) {
        st.faults.push(Fault {
            t: st.sched.now,
            node: self.nodes[node].name.clone(),
            kind,
            detail,
            step: st.sched.steps,
            key,
        });
    }

    /// The event just recorded is the one a `record_fault` fault has
    /// (S5: the view marks it).
    fn last_is_fault(st: &mut State) {
        if let Some(e) = st.events.last_mut() {
            e.fault = true;
        }
    }

    /// `fault`, with the visit it fired on (`plan.rs`).
    fn fault_at(
        &self,
        st: &mut State,
        task: Option<TaskId>,
        node: NodeId,
        kind: &'static str,
        detail: String,
        key: Option<plan::Key>,
    ) {
        let name = self.nodes[node].name.clone();
        let task = task.filter(|t| st.task_node.get(*t) == Some(&node));
        st.faults.push(Fault {
            t: st.sched.now,
            node: name.clone(),
            kind,
            detail: detail.clone(),
            step: st.sched.steps,
            key,
        });
        st.events.push(Event {
            t: st.sched.now,
            node: name,
            task,
            kind,
            detail,
            link: None,
            fault: true,
        });
    }

    /// What `node`'s disk did since it was last looked at (SQLite's
    /// syncs and faults), as events of `task`.
    fn flush_disk(&self, st: &mut State, task: TaskId, node: NodeId) {
        let events = std::mem::take(&mut vfs::lock(&self.disks[node]).events);
        for (kind, detail, key) in events {
            if matches!(kind, "fsync_lie" | "disk_full") {
                self.fault_at(st, Some(task), node, kind, detail, key);
            } else {
                self.event(st, task, kind, detail);
            }
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
            End::Violated => st
                .violation
                .as_ref()
                .map(|v| v.describe())
                .unwrap_or_default(),
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
        &'a self,
        st: MutexGuard<'a, State>,
        me: TaskId,
    ) -> (MutexGuard<'a, State>, Result<(), Ended>) {
        if let Err(e) = Self::stopped(&st, me) {
            return (st, Err(e));
        }
        let (mut st, picked) = self.pick(st);
        match picked {
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
        st.killed.push(None);
        if let Some(global) = global {
            st.tasks_by_global.insert(global, t);
        }
        let stream = if node == 0 {
            t as u64
        } else {
            let k = st.node_tasks[node];
            st.node_tasks[node] += 1;
            k
        };
        let detail = if what.is_empty() {
            format!("task {t}")
        } else {
            format!("task {t}: {what}")
        };
        self.event(&mut st, parent, "spawn", detail);
        drop(st);
        (t, self.task_rng(node, stream as usize))
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
            Err(_) if killed.is_some() && !ended => {
                let why = match killed {
                    Some(Ended::Kill) => "its node was killed",
                    Some(Ended::Exit) => "its node's process ended",
                    _ => "its node crashed",
                };
                self.event(&mut st, me, "stop", why.into())
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
        let (mut st, picked) = self.pick(st);
        match picked {
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
            (End::Violated, _) => (
                "violated",
                None,
                st.violation.as_ref().map(|v| v.describe()),
            ),
            (End::StepLimit, _) => (
                "step_limit",
                None,
                Some(format!(
                    "{} yield points without the run ending (a livelock?)",
                    st.sched.max_steps
                )),
            ),
        };
        let plan = self.recorded_plan(&st);
        let site = match outcome {
            "failed" => st
                .failure
                .as_ref()
                .map(|_| st.failure_site.clone())
                .unwrap_or_else(|| st.error_sites.get(&0).cloned()),
            _ => None,
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
            faults: std::mem::take(&mut st.faults),
            disks: self
                .nodes
                .iter()
                .zip(&self.disks)
                .filter(|(n, _)| n.kind != node::Kind::Hook)
                .map(|(n, d)| (n.name.to_string(), vfs::lock(d).disk.stats.clone()))
                .collect(),
            plan,
            violation: st.violation.clone(),
            preemptions: st.sched.preemptions,
            histories: std::mem::take(&mut st.histories),
            optional: self
                .nodes
                .iter()
                .filter(|n| n.optional)
                .map(|n| n.name.to_string())
                .collect(),
            run_index: 0,
            site,
        }
    }

    /// The plan that forces this run again: every fault that fired, keyed
    /// by its visit; the nodes left out; the schedule's stickiness.
    fn recorded_plan(&self, st: &State) -> plan::RunPlan {
        let name = |n: usize| {
            self.nodes
                .get(n)
                .map(|i| i.name.to_string())
                .unwrap_or_default()
        };
        plan::RunPlan {
            faults: st
                .faults
                .iter()
                .filter_map(|f| {
                    let key = f.key?;
                    Some(plan::Entry {
                        site: key.site.name().to_string(),
                        at: plan::place_name(key.site, key.place, &name),
                        t: key.t,
                        k: key.k,
                        delay: key.delay,
                        kind: f.kind.to_string(),
                        detail: f.detail.clone(),
                    })
                })
                .collect(),
            drop: self
                .nodes
                .iter()
                .zip(&self.dropped)
                .filter(|(_, d)| **d)
                .map(|(n, _)| n.name.to_string())
                .collect(),
            stick: st.sched.stick(),
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

/// This task's random stream, when this thread is in a world (a
/// property's predicate draws from a stream of its own).
pub fn task_rng() -> Option<Arc<Mutex<StdRng>>> {
    if !in_world() {
        return None;
    }
    if let Some(rng) = props::check_rng() {
        return Some(rng);
    }
    CTX.with(|c| c.borrow().as_ref().map(|ctx| ctx.rng.clone()))
}

/// The calling node's disk key and working directory, when this thread
/// is in a world (SQLite resolving a name with no disk in it: `vfs.rs`).
fn node_disk_here() -> Option<(u64, String)> {
    let ctx = current()?;
    let key = vfs::lock(&ctx.world.disks[ctx.node]).key;
    let cwd = ctx.world.lock().nodes[ctx.node].cwd.clone();
    Some((key, cwd))
}

// ─── S5: the tier a world runs on ───────────────────────────────────

/// 0: as `OLANG_SIM_TIER` says (the bytecode tier unless it is
/// `interp`); 1: the bytecode tier; 2: the interpreter alone.
static TIER: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Whether worlds run on the bytecode tier (S5). Every builtin call the
/// tier makes still passes `BuiltinFunctions::call_internal`, where
/// `intercept` answers it from the world (the tier's bridge dispatches
/// there, and the world is found through a thread-local, not through the
/// interpreter); native (JIT) code calls no builtin that touches the
/// world. `olang --no-ovm sim` and `OLANG_SIM_TIER=interp` run worlds on
/// the interpreter alone, as S1–S4 did: the same runs, event for event.
pub fn bytecode_tier() -> bool {
    match TIER.load(std::sync::atomic::Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => !matches!(
            std::env::var("OLANG_SIM_TIER").as_deref(),
            Ok("interp") | Ok("interpreter") | Ok("off")
        ),
    }
}

pub fn set_bytecode_tier(on: bool) {
    TIER.store(if on { 1 } else { 2 }, std::sync::atomic::Ordering::Relaxed);
}

/// Give a world's interpreter the bytecode tier, when worlds run on it
/// (and it has none).
pub(crate) fn tier_for(interpreter: &mut Interpreter) {
    if bytecode_tier() && !interpreter.bytecode_tier_present() {
        let verbose = std::env::var_os("OLANG_TIER_VERBOSE").is_some();
        interpreter.enable_bytecode_tier(1, verbose);
    }
}

static TEST_SEEDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(3);

/// Seeds `olang test` runs a block that runs worlds with (3 unless
/// `--seeds` says).
pub fn test_seeds() -> u64 {
    TEST_SEEDS.load(std::sync::atomic::Ordering::Relaxed).max(1)
}

pub fn set_test_seeds(n: u64) {
    TEST_SEEDS.store(n.max(1), std::sync::atomic::Ordering::Relaxed);
}

/// `sim.maybe(point)`: false outside a world; inside one, true when the
/// world's seed turned the point on and this visit fires.
pub fn maybe(point: &str) -> bool {
    match current() {
        Some(ctx) => ctx.world.maybe(ctx.task, point),
        None => false,
    }
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
    run_main_with(cfg, info, Vec::new(), None, main)
}

/// `run_main`, with properties checked as it runs (`template` is the
/// interpreter their predicates run on).
pub fn run_main_with(
    cfg: Config,
    info: NodeInfo,
    props: Vec<props::Property>,
    template: Option<Interpreter>,
    main: impl FnOnce() -> Result<Value, InterpreterError>,
) -> Result<Report, String> {
    if in_world() {
        return Err("sim: worlds do not nest: a world cannot run inside another".to_string());
    }
    let world = Arc::new(World::new(
        &cfg,
        vec![info],
        vec![Start::Main],
        template,
        0,
        props,
    ));
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
    // A last look at the properties, as the world stands at its end
    // (unless one already broke, or the run failed on its own).
    if result.is_ok() && world.checker.is_some() {
        let ended = world.lock().ended.clone();
        if matches!(ended, None | Some(End::MainReturned) | Some(End::Deadline)) {
            let (now, step) = {
                let st = world.lock();
                (st.sched.now, st.sched.steps)
            };
            if let Some(v) = world.check_props(now, step, true) {
                let mut st = world.lock();
                world.event(&mut st, 0, "violated", v.describe());
                st.violation = Some(v);
                st.ended = Some(End::Violated);
            }
        }
    }
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
///
/// Written through a writer that stops the formatting once it has more
/// than it keeps: an event's detail costs the same for a value of ten
/// bytes or ten megabytes (it rendered the whole value and cut it, about
/// a tenth of an olang-track seed's time).
fn render(v: &Value) -> String {
    struct Bounded {
        text: String,
        chars: usize,
    }
    impl std::fmt::Write for Bounded {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for c in s.chars() {
                if self.chars == 121 {
                    return Err(std::fmt::Error);
                }
                self.text.push(c);
                self.chars += 1;
            }
            Ok(())
        }
    }
    let mut out = Bounded {
        text: String::new(),
        chars: 0,
    };
    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{v}"));
    if out.chars > 120 {
        let cut: String = out.text.chars().take(117).collect();
        format!("{cut}...")
    } else {
        out.text
    }
}

/// The error a refused effect raises inside a world.
pub fn refusal(name: &str) -> String {
    let what = match name.split('.').next().unwrap_or(name) {
        "fs" | "asset" | "testing" | "ods" => "this part of the filesystem",
        "http" => "the network",
        "db" => "the database",
        "proc" | "pty" => "processes",
        "os" => "the operating system (environment, processes, console)",
        "tty" | "gui" => "the terminal and windows",
        "runtime" => "the runtime's modules and profiler",
        "crypto" => "the operating system's entropy",
        "task" => "real threads and real elapsed time",
        "sim" => {
            return format!("sim: {name} is refused inside a world: worlds do not nest");
        }
        _ => "the real world",
    };
    format!(
        "sim: {name} is refused inside a world: a world does not simulate {what} yet (it \
         simulates time, randomness, spawn, task, chan, http between its nodes, each \
         node's files and SQLite on its own disk, and its environment, stdin and exit), \
         and a world never reaches the real one"
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
    // A property's predicate: pure calls run, every effect is refused.
    if props::checking() {
        return match crate::effects::sim_handling(name) {
            SimHandling::Pure => None,
            _ => Some(Err(misuse(props::refusal(name)))),
        };
    }
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
    // Every effect is a point a node with `#{ crash }` may crash at.
    if let Err(e) = w.maybe_crash(ctx, name) {
        return Some(Err(e.into()));
    }
    if let Some(verb) = name.strip_prefix("http.") {
        return http::call(ctx, verb, args, interpreter);
    }
    if let Some(verb) = name.strip_prefix("fs.") {
        return files::call(ctx, verb, args);
    }
    if let Some(verb) = name.strip_prefix("db.") {
        return dbs::call(ctx, verb, args, interpreter);
    }
    if let Some(verb) = name.strip_prefix("os.") {
        return life::call(ctx, verb, args, interpreter);
    }
    Some(match name {
        "time.now_ms" if args.is_empty() => Ok(Value::Integer(w.wall_ms(ctx.node))),
        "time.monotonic_ms" if args.is_empty() => Ok(Value::Integer(w.now() as i64)),
        "time.monotonic" if args.is_empty() => Ok(Value::Float(w.now() as f64)),
        "time.sleep" => sleep(ctx, args),
        "chan.bounded" if matches!(args, [Value::Integer(0)]) => Ok(w.new_rendezvous()),
        // the channel's number is the world's (the process's would differ
        // from one run to the next in one process: Shuttle's log bus
        // writes it into the environment)
        "chan.stat" => match args {
            [c] => (|| {
                let real = crate::stdlib::chan::call_chan_function("stat", args.to_vec())
                    .map_err(|e| misuse(e.to_string()))?;
                let (local, _) = w.chan_info(c).map_err(misuse)?;
                Ok(match &real {
                    Value::Map(m) => {
                        let mut m = (**m).clone();
                        m.insert("id".to_string(), Value::Integer(local as i64));
                        Value::Map(Arc::new(m))
                    }
                    _ => real.clone(),
                })
            })(),
            _ => Err(misuse("chan.stat expects a channel")),
        },
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
    if props::checking() {
        return Err(misuse(props::refusal("spawn")));
    }
    let Some(ctx) = current() else {
        return Err(misuse(
            "sim: spawn outside a world reached the world's spawn",
        ));
    };
    World::stopped(&ctx.world.lock(), ctx.task)?;
    let mut worker = interpreter.thread_safe_clone();
    let expr = expr.clone();
    let global = crate::interpreter::spawn_registry::next_id();
    let stop = crate::interrupt::Handle::fresh();
    let thread_stop = stop.clone();
    let (_, handle) = spawn_task(
        &ctx.world,
        ctx.task,
        ctx.node,
        Some(global),
        "",
        move || {
            crate::interrupt::adopt(&thread_stop);
            worker.eval_expr(&expr).map_err(|e| e.to_string())
        },
    )?;
    crate::interpreter::spawn_registry::register(global, handle, stop);
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

    #[test]
    fn a_failure_shape_sets_aside_what_varies() {
        assert_eq!(
            shape("client c3: expected [1, 4] for \"k-17\" (2 tries)"),
            "client c#: expected […] for \"…\" (…)"
        );
        assert_eq!(
            shape("Assertion failed: List([Integer(2), Integer(9)]) != List([])"),
            "Assertion failed: List(…) != List(…)"
        );
        assert_eq!(
            shape("Assertion failed: List([Integer(4)]) != List([])"),
            "Assertion failed: List(…) != List(…)"
        );
        assert_eq!(
            shape("unbalanced [ and \" stay"),
            "unbalanced [ and \" stay"
        );
        assert_eq!(shape("a map {\"a\": [1, {2}]} here"), "a map {…} here");
    }

    #[test]
    fn a_signature_is_the_site_and_the_shape() {
        let mut a = Report::empty(1);
        a.outcome = "failed";
        a.error = Some("client c1: unexpected keys [a, b] in #{\"k\": [a, b]}".into());
        a.site = Some("kv.ol:12:5".into());
        let mut b = a.clone();
        b.error = Some("client c4: unexpected keys [c] in #{\"k\": [c]}".into());
        assert_eq!(a.signature(), b.signature());
        assert_eq!(
            a.signature(),
            "failed at kv.ol:12:5: client c#: unexpected keys […] in #{…}"
        );
        let mut elsewhere = b.clone();
        elsewhere.site = Some("kv.ol:30:5".into());
        assert_ne!(
            a.signature(),
            elsewhere.signature(),
            "another site, another failure"
        );
    }

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
            let answer = intercept("fs.watch", &[], &mut i).expect("refused, not passed through");
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

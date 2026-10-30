# `sim` — whole systems, simulated deterministically

*Local-only (branch `rc6`, olang 1.0.0-rc.6) until it is polished. The
remote stays on rc.5.*

## The idea in one paragraph

Run a whole system (three olang-track servers, a desktop client, a
flaky network, a disk that lies) inside one process, on a clock that is
not the wall clock and a scheduler driven by a seed. Simulated time runs
as fast as the CPU allows: an hour of traffic in half a second. Every
crash, dropped packet, slow `fsync`, clock jump and thread interleaving
is a decision the seed made, so any failure, however rare, comes back
exactly with `olang replay`. Run ten thousand seeds overnight across
every core. Each failure arrives shrunk to its smallest schedule, as a
trace you can step through.

## Why it is worth building

- **The bugs that hurt are the ones tests do not reach.** Data loss on
  a crash mid-write, two agents updating one issue, a long poll that
  never wakes, a backup taken during a migration, an offline queue
  replayed out of order. Today they are found in production, by luck.
- **The industry agrees, and pays for it.** FoundationDB credits this
  method for its reliability; TigerBeetle and Dropbox's sync engine are
  built around it; Antithesis is a company selling it as a hypervisor.
  Each had to rebuild its world by hand: every clock, socket and file
  call routed through an abstraction the whole codebase must respect.
- **olang already did that work.** `src/effects.rs` is the one
  classification of every builtin that touches the world, and three
  policies already read it (the capability gate, macro purity, record and
  replay). The Open Timeline already records and replays every input,
  threads included. `sim` is a fourth reader of the same table: the world
  is *simulated* instead of real. No program changes to be simulated. Its
  `http.post` simply talks to a simulated server.
- **It makes olang the language where distributed bugs are cheap.** That
  is a reason to choose a language, not a feature in its changelog.

## The art of what is possible

### 1. A server that never loses an issue
```olang
use sim

let w = sim.world(#{ "seed": seed, "net": #{ "latency": (2, 40), "loss": 0.01 } })
let server = w.node("server", ["olang", "main.ol", "7500", "track.db"], #{ "disk": sim.disk(#{ "fsync_lies": 0.001, "torn_writes": true }) })
let agents = map(range(4), (i) => w.node("agent" + to_string(i), ["olang", "tools/agent_workload.ol"]))
w.every(sim.minutes(3), () => w.crash(server, #{ "restart_after": sim.seconds(2) }))
let run = w.run(sim.hours(6))
sim.check(run, [
    sim.always("an acknowledged write survives a crash", (s) => s.acked_issues() <= s.disk_issues(server)),
    sim.eventually("every agent's last write is visible", sim.seconds(30), (s) => s.converged()),
    sim.linearizable("issues", run.history("issues"))
])
```
Six simulated hours: a crash every three minutes, four agents
writing, a disk that sometimes lies about `fsync`. One seed takes about a
second of real time.

### 2. Search, then shrink
```bash
olang sim tests/durability.sim.ol --seeds 10000 --jobs 12
#   9,996 passed · 4 failed · 3h 14m simulated per second
#   seed 7311: "an acknowledged write survives a crash" broke at 2h 41m 03.120s
#   shrunk: 11 faults → 2 (a crash 40 µs after the WAL append, a torn page on restart)
#   trace: .sim/7311.olt   (olang replay .sim/7311.olt · olang sim view .sim/7311.olt)
```
Shrinking removes faults, shortens delays and drops nodes until the
smallest schedule that still fails remains. The trace replays it
bit-for-bit, any number of times.

### 3. Watch it happen
`olang sim view` (and Studio, and a Loom panel) draws the run as
swimlanes, one per node. Messages are arrows between lanes, faults
are marks, and each node's state can be inspected at any instant. Scrub
to 2h 41m 03.120s and see the WAL append, the crash 40 µs later, and
the restart reading a torn page. Every value there can be inspected,
because a simulated run is also a recorded one.

### 4. Bugs it would find in this workspace today
- **olang-track:**
  - a crash between the WAL append and the checkpoint;
  - two MCP agents updating one issue (the 409 path, run ten thousand ways);
  - the long poll under a server restart (the `draining` answer);
  - the web client's offline queue replaying out of order;
  - the daily backup racing a migration.
- **Shuttle:** `drain` with requests in flight; the outbox's retries under a flaky webhook endpoint; the session limiter.
- **Studio:** the kernel protocol when the kernel dies mid-reply; the language server under fast edits.
- **The desktop app and its server together:** Loom's headless driver as a simulated node. The app's built-in server crashes while you type a description: does the draft survive, does the app restart it, does the save land once?

### 5. Beyond testing
- **Capacity:** "1,000 clients against one olang-track". Simulated, the queueing shows before any hardware is bought.
- **Teaching:** a distributed-systems course where every bug in the textbook is a seed you can replay.
- **Incident reproduction:** a production `.olt` recording, imported as a node's input, replayed inside a simulated network to ask "what if the disk had been slower?".
- **Fuzzing protocols:** messages mutated by the seed, coverage-guided.

## What gets simulated

| World | Real | Simulated |
|---|---|---|
| time | `time.now_ms`, `sleep`, `dates.now`, timeouts | one virtual clock per world; per-node skew and drift; sleeping costs nothing |
| randomness | `random.*`, `crypto.random_*` | streams split from the world's seed, per node |
| scheduling | OS threads for `spawn`, `task`, `par_*`, `chan` | one OS thread runs every node's tasks; the seed picks the next runnable task at every channel operation and effect |
| network | `http.get/post`, `http.serve`, sockets | an in-process network: addresses are nodes; latency, loss, duplication, reordering and partitions per link |
| disk | `fs.*`, `db.*` (SQLite) | an in-memory file system per node; SQLite through a VFS that can fail `fsync`, tear pages, fill up or slow down |
| processes | `proc.spawn`, `os.exec` | another olang program is a node; a non-olang program is a scripted stub (or refused) |
| crashes | — | `crash(node)` at any effect boundary: memory gone, disk as it really was (unflushed writes may be lost), restart on command |
| environment | `os.get_env`, args, stdin | per node, given |

## How it fits the runtime

- **One table, a fourth consumer.** `effects.rs` gains a `sim` handling
  for each effect class. A builtin's dispatch already asks the timeline
  "record this?"; in a world it asks the world "answer this", and the
  answer is the simulated clock, network, disk or scheduler.
- **Nodes are interpreters, not processes.** A node is an `Interpreter`
  with its own module cache, globals and capability grant, sharing the
  world's thread. Starting one is cheap: no process, no fork.
- **The scheduler is the heart.** In a world, `spawn` and the
  `task` and `chan` operations are cooperative: each is a yield point,
  and the seed chooses which runnable task runs next. That is what makes
  races reproducible. `par_map` runs sequentially in a seeded order.
- **Tiers.** The first version runs on the interpreter, as record and
  replay does: the one dispatch point that sees every builtin. A second
  pass routes the bytecode tier's builtin calls through the same hook, so
  long simulations run fast. Native (JIT) code is pure by construction,
  so it needs nothing.
- **Traces.** A world records as the Open Timeline does, with one
  stream per node plus the world's own decisions (faults, schedule).
  `olang replay` replays it, and `olang sim view` draws it.
- **Faults are code.** Fault plans are olang values (`sim.every`,
  `sim.at`, `sim.randomly(p, fault)`), and "buggify" points let a program
  mark where faults would be most interesting:
  `sim.maybe("after_wal_append")` costs nothing outside a world.
- **Safety.** A simulated node has no real effects at all. Nothing in a
  world can touch the real disk, network or processes, so a seed can never
  hurt the machine.

## The surface

```olang
sim.world(opts) → World                         // seed, net, start time, nodes' defaults
w.node(name, argv, opts) → Node                  // an olang program as a node (args, env, disk, grant)
w.client(name, f) → Node                         // a node that is an olang function (a workload)
w.run(duration) → Run                            // until the time, or quiescence, or a property fails
w.crash(n, opts) · w.restart(n) · w.partition([a], [b]) · w.heal() · w.skew(n, ms) · w.slow_disk(n, factor)
w.at(t, f) · w.every(dt, f) · sim.randomly(p, fault)
sim.always(name, pred) · sim.eventually(name, within, pred) · sim.never(name, pred)
sim.linearizable(name, history, model) · sim.converges(name, …)
run.history(name) · run.trace() → .olt · run.events() → Frame (an ods Frame: every event, for analysis)
sim.maybe(point) → Bool                          // a buggify point: true in some worlds, never outside one
sim.minutes(n) · sim.seconds(n) · sim.ms(n)
```

Command line: `olang sim FILE [--seeds N] [--seed S] [--jobs J] [--shrink]
[--for 6h]`, `olang sim view TRACE`, `olang sim shrink TRACE`.
`olang test` runs `sim` tests with a small seed budget by default, and
`--seeds` raises it.

## Milestones

| | milestone | done when |
|---|---|---|
| S1 | the clock and the scheduler | a world runs one olang program on virtual time with seeded scheduling of `spawn`, `task` and `chan`; two runs with one seed are identical event for event; a known race in a test program is found by some seed and replayed |
| S2 | the network | `http.serve` and `http.get/post` between nodes in one world; latency, loss, partitions; an olang-track server and a workload client run together |
| S3 | the disk and crashes | an in-memory fs per node, SQLite through a fault-injecting VFS, crash and restart with unflushed writes lost; olang-track's durability property checked over 1,000 seeds |
| S4 | properties, search and shrinking | `always/eventually/never`, the linearizability checker, `olang sim --seeds --jobs`, a shrinker that minimises faults and schedules |
| S5 | seeing | `olang sim view` (a swimlane timeline drawn with Loom), the trace's events as an `ods` Frame, the bytecode tier through the hook |
| S6 | in practice | simulation suites for olang-track (server, MCP agents, the desktop app through Loom's driver), Shuttle (drain, outbox) and Studio (the kernel); the bugs found fixed; docs/sim.md; the performance budget (≥ 1 simulated hour per real second for a server and four clients) |

## Risks, and how to meet them

- **Not all nondeterminism goes through `effects.rs`.** The fix is the
  existing `cross_consumer_agreement` test, extended: a builtin unknown
  to `sim` refuses inside a world rather than reaching the real world.
- **Rust-side threads** (the http server's workers, the gui engine).
  A world runs `http.serve` on its own simulated workers, and gui is
  refused in a world (S6 drives Loom apps through the headless driver
  instead).
- **SQLite's own threads and caches.** One connection per node, through
  the custom VFS, with no shared cache.
- **Speed.** Interpreter-only S1–S4 may be slow for long runs. S5
  routes the bytecode tier through the hook, and the budget in S6 is a
  test.

## Done, and polished, when
- Every milestone's "done when" holds.
- `olang sim` has run 100,000 seeds of olang-track's suite with every finding fixed or filed.
- The docs teach it with the five examples above.
- The rc.6 changelog reads as a feature, not an experiment.

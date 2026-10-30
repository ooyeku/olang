# `sim` S2: the network (design note)

*Branch `sim-s2` from `rc6`, olang 1.0.0-rc.6 (local). The plan is
[SIM.md](SIM.md); S1's note is [SIM-S1.md](SIM-S1.md), and S2 builds on
its baton scheduler unchanged.*

## What S2 adds

A world now holds **nodes**, and `http` between them travels a
**simulated network**: seeded latency, loss and duplication per link,
and partitions that heal. `http.serve` and `http.get/post/put/delete/
request` work between nodes in one world, and nothing opens a socket.

```olang
use kv_client { client }

let w = sim.world(#{ "seed": 1, "net": #{ "latency": (2, 40), "loss": 0.02, "dup": 0.01 } })
w.node("kv", ["olang", "kv_server.ol", "7500"], #{ "env": #{ "MODE": "x" }, "skew": 250 })
for i in range(1, 5) {
    let name = "c" + to_string(i)
    w.client(name, () => client(name, false, sim.hours(1)))
}
w.link("kv", "c1", #{ "latency": (50, 400) })
w.every(sim.minutes(5), () => {
    w.partition(["kv"], ["c" + to_string(random.randint(1, 4))])
    time.sleep(sim.seconds(20))
    w.heal()
})
let run = w.run(sim.hours(2))
run.outcome  run.events  run.history  run.net  run.nodes
```

`olang sim FILE` runs a file like this one as itself (outside any
world), with every world it makes taking the seed asked for: `--seeds N`
finds failing seeds, `--seed S` replays one. A file is taken as a
world-builder when it calls `sim.world(`; S1 programs (which cannot call
it: worlds do not nest) still run inside one world as before.

## The pieces

- **`net.rs`: the link model** (pure, unit-tested). A link is an
  unordered pair of nodes with a latency range, a loss and a duplication
  probability; links not set take the world's `net`. A partition cuts
  every pair across two groups and adds up until `heal`. A message is
  checked against partitions when sent and again when it arrives, so a
  partition that starts mid-flight drops it: the request that reached
  the server and the answer that never came back. Draws come from the
  network's own stream, `split(seed, 2)`, apart from the scheduler's
  and the tasks'; each message on a link takes the same draws whatever
  happens to it.
- **`sched.rs`: deliveries on the timer queue.** A timer now fires a
  task (`Fire::Task`) or delivers a message (`Fire::Deliver`). The
  scheduler's `next` hands deliveries that fall due back to the world
  before it draws who runs, so the tasks a delivery wakes are in the
  draw. Time still moves only when nothing can run, and a world with
  messages in flight is never a deadlock. `Wait::Net(box)` is the new
  wait: a mailbox. `kick` makes a task runnable whatever it waits on
  (a crash).
- **`wire.rs`: the world's network state.** Mailboxes, listeners,
  messages in flight, deferred requests, the history and the counts. A
  send asks the link and schedules each copy; a delivery puts a request
  in the listener's queue (or answers 503 when the queue is full, 413
  when the body is too large, or refuses when nothing listens), and an
  answer in the client's mailbox (or drops it as `late` when the client
  stopped waiting).
- **`http.rs`: http in a world.** The client resolves the host to a node
  (`localhost` is its own), sends, and waits on its mailbox until its
  `timeout_ms` of *virtual* time (30 s unless given, reqwest's default).
  A timeout, refusal or unknown host is an `Err` value, as a network
  failure is outside. The answer is the same `HttpResponse`. The server
  listens on (node, port), and its workers (4 unless `workers` says, so
  a world runs alike on every machine) are tasks of the node, each with
  its interpreter clone, as the real pool's. A handler that raises is a
  500 and a `fail` event. `http.shutdown()` stops the node's servers
  (per node: a node is a process). `http.defer/respond/hold/notify/
  notify_all/held` park requests in the world, per node.
- **`node.rs`: nodes, the coordinator, crashes.**
  - A **program node** runs an olang file on a fresh `Interpreter`
    (own globals, module cache, and grant: `#{ deny: "fs,proc" }`), with
    `os.args()` and `os.get_env/has_env` answering its own argv and env.
  - A **client node** runs `f()` on a clone of the interpreter that
    built the world.
  - A **hook** (`w.at(t, f)`, `w.every(dt, f)`) is faults as code.
  - The **coordinator** is task 0 on the caller's thread: it starts the
    nodes, then waits on its own mailbox (woken as tasks end). The run
    is `done` when every client has finished (when there are no clients,
    every program), `failed` once a node's program, a client or a hook
    raises, and otherwise runs to its deadline.
- **`mod.rs`**: `Ctx` carries the node; events carry the node's name (and
  no task for what the network does on its own); random streams are
  split by (seed, node, task) (node 0 keeps S1's); `time.now_ms` and
  `dates.*` read the world's clock plus the node's skew; `intercept`
  takes the interpreter, which `http.serve` needs for its workers.

## Clocks, randomness, determinism

Each node's wall clock is the world's plus its `skew` (ms, fixed);
monotonic time and timers are the world's. Drift is S3's or later.
Random streams: `split(split(seed, 1e6 + node), task)`. The network's
stream is `split(seed, 2)`. Every table in the world's state is a
`BTreeMap` or a `Vec`, so nothing depends on hash order; mailbox,
message, request and port numbers are the world's own counters.

## Crashes (S2's part)

`w.crash(n, #{ restart_after })`, from inside the world: the node's
servers stop listening (their queues are lost), what is in flight to it
drops on arrival, and each of its tasks unwinds at its next yield point
(`sim: this node crashed`). Memory is gone; a restart runs the program
again on a fresh interpreter. There is no disk yet, so a crash loses
everything, and S3 makes it lose only what was not flushed.

## The guarantee, kept

`effects.rs`'s `sim_handling` moves `http.get/post/put/delete/request/
serve/shutdown/defer/respond/hold/notify/notify_all/held` and
`os.args/get_env/has_env` to `Simulated`; `http.response` and
`response_with_headers` are listed there too and run as they always do
(they build values). Everything else effectful stays refused. The
walk-the-stdlib test now also holds that no `http` member is refused
(every one is answered or pure) and that the rest of `os` is. Inside a
world, `http` cannot open a socket: the in-world path never reaches
`stdlib::http`'s client or server; `body_file` (a file read) answers 500.

## The proof

- `tests/fixtures/sim/kv_server.ol`: a counter store in olang-track's
  rpc style (POST /rpc, JSON in and out), its state in one actor task
  the four workers ask. `add` is idempotent by request id.
- `kv_client.ol`: a client adds to its own key every 0.5–3 s for an
  hour, retrying what fails, then reads its key back and asserts the
  server counted each add once.
- `kv_world.ol`: a server and four clients, 2% loss, 1% duplication,
  a partition every ten minutes on average. The clients retry with the
  same id: every seed passes, with timeouts and retries in the history.
- `kv_partition_bug.ol`: the planted bug. Clients retry under a fresh
  id (they take a timed-out add as not applied). The network loses
  nothing, so only a partition can time a request out. Most seeds pass;
  a seed where a partition cuts an answer after the server applied the
  add counts it twice, and the client's check fails. `olang sim
  kv_partition_bug.ol --seeds 30` names such seeds; `--seed S` replays
  one, event for event, and its events show the partition, the cut
  answer, and the retry.

## Measured (M-class Mac, release build)

`kv_world.ol`: a server and four clients for a simulated hour, about
7,900 requests (each a JSON rpc through the server's workers and its
actor), 15,000 messages and 86,000 events. One seed runs in about 0.45 s
of real time; `--seeds 20` runs at 2.3 simulated hours per real second,
and `kv_partition_bug.ol --seeds 30` (a slower WAN) at 2.6. S6's budget
(at least one simulated hour per real second for a server and four
clients) holds with room. The cost is still S1's: a thread handoff at
every switch, on the interpreter tier.

## Limits in S2

- One request per connection: no keep-alive, no streaming, no
  `max_requests_per_connection` or idle timeouts (nothing idles).
- A partition is symmetric; one-way cuts and per-direction links are a
  small step on `net.rs` if wanted.
- `w.crash` loses all memory (no disk until S3), and restarts need the
  node's old tasks to have unwound (they do at once unless a program
  catches the crash error and keeps computing).
- Clock skew is a fixed offset; drift is not modeled.
- A node is olang: a non-olang program as a scripted stub is later.
- The world runs on the interpreter tier (as S1).

## The stretch: olang-track as a node

Not attempted past reading it: olang-track's server (`api.ol` →
`lib/server.ol`'s `run_server`) needs, before it serves a request,
`fs.exists`/`fs.read_file`/`fs.list_dir` (its repository files and data
folder), `db.open` and `db.query` (SQLite: the schema, migrations, every
rpc), `os.set_env`, `os.on_shutdown`, `os.read_line` (the lifeline) and
`os.exit`, and the `shuttle` package (`olang sim` loads no package
dependencies yet). Its db and fs needs are S3's (an in-memory fs per
node, SQLite through a VFS); `os.set_env`, `on_shutdown` and a stdin per
node are small additions that belong with them.

## What S3 needs from this

- **A per-node fs** answers `fs.*` from node state (`NodeState` grows a
  file tree); a crash keeps it, minus unflushed writes. The crash path
  here (`World::crash`: listeners, in-flight, tasks kicked) is the one
  to extend: drop the unflushed part of the fs too.
- **SQLite through a VFS** per node, one connection per node, no shared
  cache; its `fsync` and page writes are the fault points. A crash then
  is "memory gone, disk as it really was".
- **`os.set_env`, `os.on_shutdown`, a stdin per node, `os.exit`** (a
  node's exit: its main task ends, its servers close), so olang-track's
  `run_server` runs unchanged; and package dependencies for `olang sim`
  (shuttle), resolved the way `olang run` resolves them.
- **Restart semantics**: `on_shutdown` handlers run on a graceful stop
  but never on a crash; a restart re-reads the disk.
- **Search over crash points** (S4) needs crashes at any yield point:
  `sim.maybe(point)` and a fault plan that the shrinker can minimize.
  Crashes are already world decisions recorded as events.

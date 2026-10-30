# `sim` S1: the clock and the scheduler (design note)

*Branch `rc6`, olang 1.0.0-rc.6 (local). The plan is [SIM.md](SIM.md).*

## Where the scheduler lives: a baton between threads

Tasks stay OS threads. Only the task holding the world's **baton** runs
olang code; every other task of the world waits on its own condition
variable under the world's one lock. At each yield point the holder asks
the scheduler who runs next, hands the baton over, and waits for it to
come back.

Running every task on one OS thread would need the interpreter to
suspend a task in the middle of an evaluation and resume another. The
tree-walker keeps a task's state on the Rust stack (a recursive
`eval_expr`), so that means continuations or green threads under the
interpreter. That is a rewrite of the evaluator, and every builtin that
calls back into olang (`map`, `cell.update`, `md.render`) would have to
learn it too. The baton gives the same guarantee (one task at a time, in
the order the seed picks) with no change to how olang code is evaluated.
`spawn` in a world still starts a thread, a `thread_safe_clone` of the
interpreter, exactly as outside. The cost is a thread handoff per switch,
which is measured below.

The rule that makes it deterministic: **the baton holder is the only
thread that touches olang state or the world.** Channel queues stay
crossbeam's, but inside a world every operation on them is a *try*, made
by the holder. A blocked receive is a task marked blocked in the
scheduler, never a thread parked inside crossbeam. Because nothing else
runs, the try and the bookkeeping cannot race.

## The pieces

- **`src/sim/sched.rs`** holds the clock and the scheduler: pure
  bookkeeping with no threads, unit-tested on its own.
  - *Time moves only when nothing can run.* Running code costs no
    virtual time. When no task is runnable, the clock jumps to the
    earliest timer, and every timer due then fires in the order it was
    set.
  - *The seed picks.* At a yield point the runnable tasks are taken in
    id order and one is drawn from the scheduler's own stream. That
    stream is split from the world's seed (splitmix64), separately from
    each task's random stream.
  - *The end:* a deadline passed, every task blocked with no timer (a
    deadlock, naming each task's site), or the step budget spent (10 M
    yield points by default, against livelock).
- **`src/sim/mod.rs`** holds the world (state, baton, events), the
  thread-local context, the dispatch hook, and the world's versions of
  the scheduler's operations.
- **`src/sim/surface.rs`** is the olang surface. **`src/sim/cli.rs`** is
  `olang sim`.

**Yield points:** `spawn` (after the task is registered),
`chan.send/recv/try_recv/recv_timeout/ask/close`, `task.join` and
`task.join_timeout`, and `time.sleep`. A blocked operation retries when
it is woken: a wake is a hint, so a spurious one costs a step and never
gives a wrong answer. Clock and random reads are not yield points: they
cannot change who can run.

**Answered by the world:**
- `time.now_ms`, `monotonic_ms` and `monotonic` read virtual time;
  `time.sleep` blocks on a virtual timer.
- `chan.recv_timeout` and `task.join_timeout` are blocks with a timer.
- `chan.bounded(0)` keeps its rendezvous semantics: a one-slot queue,
  and the sender waits until a receiver has taken its message.
- `dates.now/utc_now/today/stamp/stamp_ms` read `crate::clock`, which
  answers from the world while the thread is in one. Local time in a
  world is UTC, so a run reproduces on any machine.
- `random.*`, the ods samplers that draw through `random`, and
  `crypto.random_bytes/random_hex` draw from **the running task's own
  stream**, split from the seed by task id. One task drawing one more
  number never moves another task's draws, which S4's shrinker will
  need.
- `par_map`/`par_filter` run in order on the task's thread, and
  auto-parallel fan-out is off.

**Refused** (an error inside a world; the real call is never made):
everything `effects.rs` gates or records that the world does not
answer: `fs`, `http`, `db`, `proc`, `pty`, all of `os`, the ods file
readers and writers. Also refused by name: `tty`, `gui`, `asset`,
`testing.snapshot*`, `runtime.*` (except `features`/`version`), the
crypto calls that take the OS's entropy (`encrypt_aes`, `encrypt_rsa`,
`generate_key_pair`, `hash_password`), `task.list/parked` (real threads,
real elapsed time), and `sim.world` (worlds do not nest). This is
`effects::sim_handling`, the table's fourth column. The test
`sim_never_lets_a_classified_effect_through` walks the whole stdlib and
fails if any gated or recorded builtin would run for real in a world.

**One dispatch point.** `BuiltinFunctions::call_internal` asks
`sim::intercept` after the capability gate and before the timeline. The
world is found through a thread-local, so the bytecode tier's bridge
dispatch is intercepted too. A world still runs on the interpreter tier,
as record and replay do: `w.run_fn` sets the bytecode tier aside while
the world runs, and `olang sim` never enables it. Outside a world the
hook costs one thread-local read per builtin.

**Events:** an ordered list of `(t, task, kind, detail)`.
- Kinds: `start`, `spawn`, `send`, `recv`, `close`, `sleep`, `timeout`,
  `join`, `print`, `end`, `fail`, and the outcome (`done`, `failed`,
  `deadline`, `deadlock`, `step_limit`).
- Channels and tasks carry the world's own numbers (channel #1, #2 in
  order of first use; task 0 is the main program, then spawn order), so
  the list is the same in every run of a seed, in any process.

**The world's end:** the main program returning, a deadline, a deadlock
or the step budget. The baton goes to the main task. Every other task
then unwinds in id order, holding the baton alone: its pending
operation fails with "the world has ended". A task that catches that
error and keeps computing is given 10 s of real time, then left behind
(`stuck` in the report).

## Surface (S1)

```olang
let w = sim.world(#{ "seed": 7, "start": 1767225600000 })   // start: epoch ms, default 2026-01-01
let run = w.run_fn(main, #{ "for": sim.hours(6) })
run.outcome   // "done" | "failed" | "deadline" | "deadlock" | "step_limit"
run.value  run.error  run.now  run.steps  run.tasks  run.failed_tasks
run.events    // [#{ t, task, kind, detail }, ...]
sim.ms(n)  sim.seconds(n)  sim.minutes(n)  sim.hours(n)
```

`olang sim FILE --seed S [--events] [--for 6h]` runs FILE as a world's
main program. It shows the program's output, then a summary; on failure
it prints the error and the last events. `olang sim FILE --seeds N
[--seed FROM]` runs N seeds in turn, keeps the program's output quiet,
prints each failing seed, and ends with `reproduce: olang sim FILE
--seed S`. A seed fails when its outcome is `failed`, `deadlock` or
`step_limit`.

## Measured (M-class Mac, release build)

- **Sleep-bound:** `tests/fixtures/sim/virtual_time.ol` (a ticker
  sleeping a minute sixty times while main sleeps an hour) runs a
  virtual hour in about 3 ms of real time for one seed, and twenty seeds
  in 8 ms: about 2,400 virtual hours per real second.
- **Message-bound:** a server and four clients, each asking every
  0.1–1 s, make about 26,000 request/reply pairs per virtual hour. That
  workload runs at 2.6–2.8 virtual hours per real second, roughly
  340,000 events per second.

The message-bound cost is the thread handoff at each switch (a
condition-variable wake). S6's budget (≥ 1 simulated hour per real
second for a server and four clients) already holds for this workload;
a heavier server will need the bytecode tier through the hook (S5), and
perhaps spin-then-park handoff.

## What S2 and S3 need from this

*S2 is built: [SIM-S2.md](SIM-S2.md) says how each point below came out.*

- **Nodes.** A node is an `Interpreter` with its own globals and module
  cache, sharing the world. `Ctx` (world, task) gains a node id; the
  scheduler is already world-wide, so tasks of every node are drawn from
  one runnable set. Per-node clock skew is a per-node offset in
  `virtual_epoch_ms`. Per-node random streams split from `(seed, node,
  task)`.
- **The network** is a new `Wait` kind (a socket, an accept) plus
  timers for latency. `http.get/post/serve` move from `Refused` to
  `Simulated` in `sim_handling`, with an in-world implementation in the
  `intercept` match, as `chan` has now. Loss, duplication and
  partitions are draws from a world-level stream (`split(seed, 2)`),
  kept apart from scheduling.
- **The disk and crashes.** `fs`/`db` become `Simulated` against a
  per-node in-memory fs and a SQLite VFS. A crash is the world's end in
  miniature, applied to one node: its tasks unwind with an error (the
  same drain the world's end uses), and its memory is dropped.
- **Traces.** `Report.events` is already the world's decision log in
  order. S5 writes it as an `.olt` stream beside each node's timeline.
- **Search and shrinking** (S4) need every seed's world to be
  independent. The world's state, numbering and streams are its own,
  and worlds on different threads share nothing, so `--jobs` can run
  one world per thread.

## Limits in S1

- `olang sim` runs a single file: no packages' dependencies and no macro
  expansion (`use` of a sibling file works).
- A program reading the clock in a busy loop (`while time.now_ms() < t
  {}`) never advances it. The step budget does not catch that loop,
  because it has no yield point.
- Channels made outside a world can be used inside it, except
  rendezvous ones (refused, with a clear error).
- A task handle's printed form (`<task 12>`) and `chan.stat(c).id` are
  process-wide counters. A program that prints them prints different
  numbers in a second world in the same process. The events use the
  world's own numbers and are unaffected.

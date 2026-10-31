# `sim` S4: properties, search and shrinking (design note)

*Branch `sim-s4` from `rc6`, olang 1.0.0-rc.6 (local). The plan is
[SIM.md](SIM.md); the earlier notes are [SIM-S1.md](SIM-S1.md),
[SIM-S2.md](SIM-S2.md) and [SIM-S3.md](SIM-S3.md). S4 keeps their
scheduler, network and disk, and adds what turns a world into a search.*

## What S4 adds

What must hold is now a value the world checks as it runs; every fault
is a decision a plan can force; seeds run side by side; and a failing
seed comes back as the smallest run that still fails, saved so it
replays bit for bit.

```olang
use kv_client { recorded_client }

let w = sim.world(#{ "seed": 1, "net": #{ "latency": (20, 300) } })
w.node("kv", ["olang", "kv_server.ol", "7500"])
for i in range(1, 5) {
    let name = "c" + to_string(i)
    w.client(name, () => recorded_client(name, true, sim.hours(1)), #{ "optional": i > 1 })
}
let run = w.run(#{ "for": sim.hours(2), "props": [
    sim.linearizable("every add counts once", "kv", sim.kv()),
    sim.always("an acked put is durable", (s) => acked(s) <= durable(s.disk("kv"))),
    sim.never("two leaders", (s) => leaders(s) > 1),
    sim.eventually("the replicas agree", sim.seconds(30), (s) => agree(s))
] })
run.outcome            // "violated"
run.violation          // #{ property, kind, t, step, detail }
run.history("kv")      // what sim.record kept
sim.check(run, props)  // the same, after the fact: raises naming the property
```

```bash
olang sim kv_linear.ol --seeds 16 --jobs 4
#   seed 5: "every add counts once" broke at 1h 00m 25.794s (step 81890): key c4: ...
#   olang sim: 16 seed(s) · 15 passed · 1 failed · 5h 38m simulated per second (5.63 virtual hours) · 4 job(s) · 2.852 s real
#   digest: 04fc8dbf6310b78a
olang sim kv_linear.ol --seed 5 --shrink --jobs 4
#   shrunk: 3 faults → 1 · 17 runs · preemptions 17106 → 6665 (schedule stick 0.5) · left out: c2
#        20m 00.000s  partition  kv       [kv] | [c4]
#   plan: .sim/5.simplan   (olang sim kv_linear.ol --replay .sim/5.simplan)
olang sim --replay .sim/5.simplan       # replayed bit for bit (digest ba157ea63adc9ec0, 1 fault(s))
olang sim shrink .sim/5.simplan         # shrink a saved plan further
olang test tests/                       # a test block that runs worlds runs over 3 seeds (--seeds N)
```

## Properties (`props.rs`)

`sim.always(name, pred)`, `sim.never(name, pred)`,
`sim.eventually(name, within, pred)` and
`sim.linearizable(name, history, model)` make property values. A run
takes them as `w.run(#{ for, props })` (or `w.run(d, props)`, and the
same for `w.run_fn`); `sim.check(run, props)` checks them after the
fact; `run.check(props)` is the same call.

### When a property is checked

At every **instant** at which anything ran: when nothing is left to run
at the current millisecond and the clock is about to move on, and once
more when the run ends. The scheduler (`Sched::next`) answers
`Pick::Check` instead of advancing the clock when any step was taken
since the last check; the world lets go of its lock (the baton stays with
the task that yielded, so nothing else runs), runs the predicates, and
ends the world with `End::Violated` at the first that breaks.

Running code costs no virtual time, so an instant is the smallest step
of the world anyone can observe; a state that exists only between two
tasks' turns within one millisecond is not seen. Checking at every yield
point would cost a predicate call per scheduler step (millions per run;
an olang-track seed takes about 30,000 steps a simulated minute, a
kv-world hour 80,000), against one per instant (a few thousand). The
choice is documented in `props.rs` and is the one that keeps a
property's cost near the run's own.

- `always`: true at every check. `never`: false at every check.
- `eventually(name, within, pred)`: never false for longer than `within`
  ms at a stretch. When it turns false the world sets a check timer
  (`Fire::Check`) at the stretch's deadline, so the violation is
  reported at the millisecond it happens even when the world is quiet
  then. A stretch still open when the run ends is not a violation.
- `linearizable`: once, at the end, over the named history (or a list).
  An *unknown* verdict (the search's bound) is an event, not a failure.

A violation records the property, its kind, the virtual time, the
scheduler's step and why (`run.violation`); the outcome is `violated`,
which `Report::failed` counts, so `olang sim --seeds` names the seed, the
property and the time.

### What a predicate sees

One argument, `s`: `s.now`, `s.time` (the world's wall clock), `s.step`,
`s.nodes`, `s.history(name)`, `s.up(node)`, and `s.disk(node)`, a node's
disk **as a power cut now would leave it**:

| read | answers |
|---|---|
| `.read(path)` / `.read_bytes(path)` | the durable contents under the durable name; `()` when the name is not durable |
| `.exists(path)`, `.list(dir)` | durable names |
| `.current(path)` | what the node reads now, synced or not |
| `.query(db, sql, params)` | `Ok(rows)` from the durable image of a SQLite database: a copy of the disk with only what was synced (`Disk::durable_image`), opened through the world's VFS and recovered as SQLite would after the crash (a hot WAL replayed); `Err` when it cannot be read |

A predicate runs on an interpreter cloned from the one that built the
world, with the world paused, and changes nothing: `props::checking()`
makes `intercept` refuse every non-pure builtin (`time.now_ms`, `fs`,
`http`, `db`, `spawn`: "a property's predicate cannot call ..."), and its
random draws come from a stream of its own (seeded from the world's seed
and the step), so a watched run is the same run as an unwatched one up
to the violation. That is what lets `sim.check` run a world again.

### `sim.check(run, props)`

Linearizability is checked over the run's recorded histories. Any other
property needs the states, which a finished run no longer has, so
`sim.check` runs the world again from its value (the run's map carries a
handle with the world, its duration and, for `run_fn`, the function):
one seed is one run, so it is the same run, now watched, stopped at the
first violation. It raises `sim.check: property "…" broke at 2m
03.120s (step 1234): …`, and the CLI reports the watched run.

## Histories and linearizability (`lin.rs`)

`sim.record(name, op)` adds `op` to the history `name` (nothing outside
a world, so a client that records costs nothing elsewhere). An op is
Jepsen's shape: `type` is `invoke`, `ok`, `fail` or `info`; `f` names
the operation; `value` is its argument on an invoke and its output on an
`ok` (the invocation's value when an `ok` has none). The world adds
`process` (the recording node and task, unless given), `node`, `t` and a
global `index`. `run.history(name)` and `s.history(name)` answer the
list; it is shared (an `Arc`), so reading it in a predicate copies
nothing. A process invoking again while its last operation is open
leaves that one indeterminate.

`sim.linearizable(name, history, model)` and the bare checker
`sim.linearize(history, model, #{ max_steps })` →
`#{ ok, verdict, reason }` take a model:

- `sim.register(init)`: `write v`, `read → v`, `cas [old, new]` (an
  output of `false` says it did not swap);
- `sim.kv()`: `put [k, v]`, `get [k] → v`, `add [k, by] → total`, `cas [k,
  old, new]`;
- an olang value `#{ init, step }`, `step(state, op) → Ok(next) |
  Err(why)`, `op` being `#{ f, value, output, known, process }`.

**The search** is Wing and Gong's with Lowe's memoisation (the method of
Knossos and Porcupine): the history's calls and returns in a linked list;
linearize a call whose op the model accepts in the current state, lift
it with its return, start again from the front; when a return comes
before its op could be placed, backtrack. A (linearized set, model state)
pair seen before is not explored again; the key is the set's bits and two
64-bit hashes of the state's text. An indeterminate op returns "at
infinity": reaching its return means every completed op is placed, and
the ops left may never have happened. A failed op is dropped.

**Locality**: a `kv` history is split by key and each key searched on
its own (a history is linearizable iff each object's is), which makes
an hour of four clients (7,000 ops) a few milliseconds.

**The bound**: 2,000,000 search steps by default (`max_steps`); past it
the verdict is *unknown*, never a hang. Ten processes writing
concurrently with a read after are linearizable in a few hundred steps
with the memo; without it the search would be 10!.

## Faults as a plan (`plan.rs`)

Every place a fault is decided is a **site**: the draws (`crash_point`
at an effect or inside SQLite, `fsync_lie`, `maybe`, `loss`, `dup`) and
the calls in code (`crash`, `kill`, `stop`, `partition`, `slow_disk`).
A site has a *place*: a node, or a link (`from>to`) for the network.
Each visit to a site is counted, and a visit's **key** is (site, place,
virtual ms, k), k numbering the visits to that site and place within
that millisecond. Every fault records the key of the visit it fired on
(`Fault.key`), and the run's `Report.plan` lists them: a plan that
forces the run again. `disk_full` is a consequence, not a decision, and
has no key; torn writes are part of a crash (their draws are kept).

**Forcing.** A world given a plan (`Config.forced`) still makes every
draw it would have made (so latencies, tears and the network's other
draws advance exactly as before), but whether a fault fires is the
plan's answer: a visit fires when the plan holds an unused entry for
its site and place whose (t, k) is at or before the visit's. With its
own recorded plan a run comes back event for event (tested on the log,
kv and SQLite worlds). With entries dropped, the remaining ones fire at
their instant if the run reaches it, else at the site's first visit
after; a call in code the plan does not list does nothing (an event,
`skipped`, says so). A forced entry may carry a shorter restart delay.

The draw sites are the ones S3 named: `NodeDisk::{draw_lie, draw_crash}`
(with the world's clock in an atomic the disk can read without the
state's lock), `World::maybe_crash` and `World::maybe`, the network's
`transmit` (loss and duplication, through `Net::transmit_with`), and the
code faults in `surface.rs`. Disk sites live with their disk (SQLite
reaches them from inside a `db` call); the rest with the world.

A plan also carries the **optional nodes to leave out**
(`#{ optional: true }` on `w.node`/`w.client` lets the shrinker drop it;
a dropped node never starts) and the **schedule's stickiness**: with
`stick` p, a task that yields and could go on keeps the baton with
chance p (1: until it blocks), from a stream of its own. Every run
counts its **preemptions** (yield points at which a runnable task lost
the baton).

Two determinism fixes came with it: a node's tasks draw from streams
keyed by the task's number *within its node* (they were keyed by the
global task id, so leaving one node out moved every other node's
draws), and the report's plan is taken before the faults are moved out.

## Search: `--jobs` (`cli.rs`)

`olang sim FILE --seeds N --jobs J` starts J worker processes
(`olang sim FILE --worker`, hidden), each fed one request at a time on
stdin (JSON: a seed, an optional plan, whether to send plans back) and
answering one marked line on stdout (the outcome, the failure line, the
property and time, the signature, the run's **digest**, fault and
preemption counts). A worker takes the next seed as it finishes one, so
a slow seed does not hold the others. Each world is still one thread and
one seed; results are put back in seed order. The run's digest is FNV-1a
over every event, fault, the outcome, the time and the steps, the same
on every machine and in every process; the summary ends with the digest
of the sweep (every seed's digest in order), and one job and four give
the same digest (tested on the kv, log and SQLite worlds). Progress is a
line updated in place when stderr is a terminal.

```
seed 5: "every add counts once" broke at 1h 00m 25.794s (step 81890): key c4: ...
olang sim: 16 seed(s) · 15 passed · 1 failed · 5h 38m simulated per second (5.63 virtual hours) · 4 job(s) · 2.852 s real
digest: 04fc8dbf6310b78a
reproduce: olang sim kv_linear.ol --seed 5
```

## Shrinking (`shrink.rs`)

`--shrink` (with `--seed S`, or the first failing seed of `--seeds`)
and `olang sim shrink PLAN` (from a saved plan). A change is kept only
when the run still fails **the same way**: `Report::signature` is the
property's name when one broke, else the outcome and the error's first
line with its numbers replaced (`client w#: ... version #`).

1. The seed's run is recorded and replayed from its own plan; if the
   digest differs, the shrinker stops (nothing sound to shrink).
2. **Faults**, by ddmin: none at all; then complements of halves,
   quarters, ..., single faults. 1-minimal: no one fault can go. With
   `--jobs` a round's candidates run side by side and the first in order
   that still fails is kept, so the answer does not depend on J.
3. **Nodes**: each optional node left out, one at a time.
4. **Delays**: each restart delay tried at 0, a quarter, a half.
5. **The schedule**: stick 1, 0.95, 0.75, 0.5; the stickiest that
   still fails.
6. The plan becomes what the last run actually fired, run once more to
   check the digest, and is saved as `.sim/<seed>.simplan`.

Every step is a run of the whole world; at most 300 runs (`--shrink-
budget N`), and a shrink cut short says so.

**The plan file** is JSON (the Open Timeline records one interpreter's
inputs; a world's decisions are a different thing, and a plan is small):
`olang_sim_plan` (1), `file` (absolute), `seed`, `for`, `max_steps`,
`runs` (each world run of the file, by number: `faults` — site, at, t,
k, delay, kind, detail —, `drop`, `stick`), and the `outcome`,
`signature`, `failure` and `digest` it reproduces. `olang sim --replay
PLAN` runs it and checks the digest ("replayed … bit for bit"), exiting
3 when it diverges. A plan replays with the same environment variables
the file read (track_world.ol's `CRASH`, say): those are the file's
inputs, not the world's.

## `olang test`

A `test` block that runs worlds is a sim test: the runner runs it with
seed 1 (every `sim.world` in it takes the seed, as under `olang sim`),
and when it made worlds, again with seeds 2 and 3 (`olang test --seeds
N`), stopping at the first seed that fails and naming it ("sim seed 5:
…"). A block that runs no world runs once, as before. `sim.seed()`
answers the seed (inside a world, or while `olang sim`/`olang test`
decide it).

## The guarantee, kept

`effects.rs` is unchanged: the new builtins are the `sim` module's own
(pure by classification; `sim.world` stays refused inside a world, so
`sim.check` cannot nest), and a predicate's effects are refused by the
checking mode above. The cross-consumer test and the walk of the stdlib
pass as before. The wasm build compiles (`stub.rs` gains `test_seeds`).

## The proof

All in `tests/sim_test.rs` after `// ─── S4` (fixtures in
`tests/fixtures/sim/`), run with `cargo test --profile fasttest --test
sim_test`; unit tests for the checker in `src/sim/lin.rs`, for sites and
forcing in `src/sim/plan.rs`.

- **Properties pass and break correctly.** A history-driven `run_fn`
  world: `always`, `never` and `eventually` hold; `always` breaks at
  500 ms and `never` at 400 ms (the instants the history crosses their
  lines), each with its step; `eventually(…, 150, …)` breaks at 151 ms,
  the stretch's deadline, though nothing happens then; a predicate that
  raises, or calls `time.now_ms`, breaks its property with that reason.
  (Writing the test showed the instant rule at work: a write completed
  and the next invoked in the same millisecond is never seen with none
  open.)
- **Disks from a predicate.** `s.disk(n)` reads synced bytes, the
  current bytes, and nothing for a name never synced; `.query` on the
  durable image of SQLite holds "acknowledged puts are durable rows" on
  an honest disk and breaks it when fsync always lies.
- **The checker on known histories**, through `sim.linearize` and in the
  unit tests: a write overlapping two reads that see old then new
  (linearizable) and new then old (not); an indeterminate write read or
  not (both fine) but not read then un-happened; a failed write never
  happened; two compare-and-sets from 0 both succeeding (not); a kv
  history with a double-counted add (caught, naming the key); ten
  concurrent writers then a read (linearizable, then not with an
  impossible read); an olang counter model; a two-step bound (unknown).
- **The S2 bug as a linearizability violation.** `kv_linear.ol` (the
  partition bug's clients, recording instead of asserting): 16 seeds with
  `--jobs 4` find seeds where "every add counts once" breaks ("no order
  of the 1,749 operations explains the history"); with retries under the
  same id (`KV_BUGGY=0`) 8 of 8 pass.
- **The S3 bug as a durability violation.** `log_durable_bug.ol` breaks
  "an acknowledged put is on the store's disk" on 4 of 4 seeds within
  0.3 s of virtual time, no crash needed: the property sees an
  acknowledgement the disk does not have yet. `log_durable_safe.ol`
  (sync, then acknowledge) holds it through power cuts and torn writes.
- **`sim.check`** runs an unwatched run again with the property and
  raises naming it and its time; `run.history(name)` answers the run's
  history.
- **One job and four give the same results**: the sweep digest and every
  failure line are equal for kv_linear (8 seeds), log_bug (8) and
  sqlite_lies (4), and for olang-track with power cuts (3 seeds, one job
  and three).
- **A run comes back from its plan**: log_bug, kv_partition_bug and
  sqlite_lies on seed 5, forced by their recorded plans, have the same
  events and digest; with no faults forced, another run.
- **The shrinker**: `log_noisy.ol` buries log_bug's one decisive power
  cut in a spare node's random crashes, slow disks, partitions and
  buggify points and 1% loss on the writers' links. `--seed 1 --shrink
  --jobs 2`: **57 faults → 1** (the store's power cut, its restart delay
  shortened to 0), the spare node left out, preemptions 2,072 → 0, in
  17 runs (about 1 s). The saved plan replays bit for bit twice, and
  `olang sim shrink` on it shrinks no further.
- **`olang test`**: `sim_blocks_test.ol`'s three blocks run over three
  seeds and pass; a block that fails only on seed 5 passes with the
  default budget and fails with `--seeds 6`, naming seed 5.

Shrinking the earlier fixtures (`--shrink`): log_bug seed 3, 1 fault → 1
(delay 1000 → 0 ms); kv_partition_bug seed 3, 3 faults → 1 (the one
partition, at 1h 00m); kv_linear seed 5, 3 faults → 1 with client `c2`
left out and the schedule at stick 0.5 (preemptions 17,106 → 6,665).

### Measured (M-class Mac, release build, `--seeds 16`)

| world | simulated per seed | real per seed, 1 job | 4 jobs: simulated per real second |
|---|---|---|---|
| `kv_world.ol` (S2: four clients, an hour) | 1 h | 0.46 s (2.19 sim h/s; S3: 2.24) | 5.42 h/s (2.5×) |
| `log_safe.ol` (the store, ~30 crashes) | 10 min | 0.73 s (S3: 0.68) | 0.71 h/s (3.1×) |
| `sqlite_honest.ol` (SQLite, crashes mid-commit) | 10 min | 0.80–0.93 s (S3: 0.74) | 0.41 h/s |
| `kv_linear.ol` (with the linearizability check) | 1 h | 0.84 s (1.20 sim h/s) | 5.6 h/s |
| `log_durable_safe.ol` (an `always` over the disk at every instant) | 2 min | 0.70 s (the same store unwatched: 0.23 s) | |
| `track_world.ol`, power cuts, `MINUTES=2` | 2 min + the check | 3.4 s | 100 seeds in 88 s |

Workers scale to about 2.5–3× on four jobs here (each seed's tasks are
threads that hand a baton, so a world is one busy core; the rest is the
machine). A property costs what its predicate costs, once per instant: the
durability property (the log's durable text read and searched, the
history read from its end) makes the log store's run about 3× as long
(0.70 s a seed against 0.23 s); written as a fold over the whole
history it was quadratic, 7.3 s for one seed. Checking more coarsely, or
a predicate on the bytecode tier, are S5's to weigh.

### olang-track, again

**Power cuts now pass: 100 seeds, 100 passed** (`CRASH=power MINUTES=2
EVERY=20 olang sim track_world.ol --seeds 100 --jobs 4`, 88 s; four or
five power cuts a seed). S3 found 100 of 100 losing acknowledged issues
at `synchronous=NORMAL`; olang-track's commit cc4fd13 syncs every commit
(`synchronous=FULL`), and the sweep confirms it. The S3 test that held
the loss is now the test that holds the fix.

Running olang-track seed after seed in one process found two
process-wide counters reaching a run, so a seed rendered differently
the second time (and one job and four disagreed): a SQLite connection's
`id` (the db registry's key, printed in values Shuttle sends through
channels) and a channel's `id` from `chan.stat` (Shuttle's log bus writes
it into the environment). In a world a connection now shows its number
within the world (`id`; the registry's key moved to its handle, which
`connection_id` reads first), and `chan.stat` answers the world's number
for the channel.

**Speed.** One 10-minute olang-track seed with no crash (two agents
filing issues): 7.9 s real (S3: 7.8 s). A profile: about half the busy
time is SQLite executing olang-track's own statements (index scans,
record comparisons: the snapshot rebuild and activity rows of each
`issues.create`), most of the rest the interpreter; the world's own
machinery (scheduler, baton, disk, sites) does not show. The cheap win
was event details: `render` printed a whole value and cut it to 120
characters (about 8% of the time, on Shuttle's large messages); it now
writes through a writer that stops at 121 characters. **7.9 s → 7.0 s.**
That is 0.086 simulated hours per real second for a server and two
agents, against S6's budget of one hour per second for a server and four
clients: the gap is the interpreter tier (S5) and the application's SQL.

## Limits in S4

- **Instants, not steps.** A property never sees a state that lasts less
  than a millisecond of virtual time between two tasks' turns.
- **`sim.check` runs the world twice** when a property needs states;
  `w.run(#{ props })` watches the first run instead.
- **The shrinker's unit is a fault**, not a message's latency or a
  scheduling decision: the schedule is simplified as a whole (stickier),
  not decision by decision, and latencies are not shortened. A failure
  that needs a precise interleaving keeps the seed's schedule (stick 0).
- **Signatures are textual.** Two different bugs that fail with the same
  property, or the same assertion text, count as one.
- **Linearizability's memo key is a hash** of the state's text (two 64-bit
  hashes; a collision would prune a branch, astronomically unlikely). A
  custom model's state should print canonically (the same state, the
  same text) for the memo to work; it is still correct when it does not,
  only slower.
- **Workers are processes** that resolve the file afresh (module load,
  package resolution): a few ms a seed, nothing next to a seed's run.

## What S5 needs from this

- **Speed.** A world still runs on the interpreter tier: olang-track does
  10 simulated minutes in 7 s (0.086 h/s; the budget is 1 h/s). About
  half is olang-track's own SQL; the other half is what the bytecode
  tier through the hook should cut. Properties are olang too, and a
  predicate on the bytecode tier would make checking at every instant
  nearly free.
- **Seeing a plan.** `olang sim view` should draw a run with its faults
  marked by their keys, and a shrunk plan beside the original.
- **Events as a Frame.** The digest walks every event; the same events
  are S5's `run.events()` Frame.
- **The trace.** A plan file plus the seed is a complete trace of a
  world's decisions; S5's `.olt` for worlds can embed it.

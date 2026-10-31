# `sim` S5: seeing (design note)

*Branch `sim-s5` from `rc6`, olang 1.0.0-rc.6 (local). The plan is
[SIM.md](SIM.md); the earlier notes are [SIM-S1.md](SIM-S1.md),
[SIM-S2.md](SIM-S2.md), [SIM-S3.md](SIM-S3.md) and
[SIM-S4.md](SIM-S4.md). S5 changes no run: every S1–S4 world gives the
same events, digest for digest. It adds ways to look at a run, and runs
worlds on the bytecode tier.*

## What S5 adds

```olang
let run = w.run(sim.hours(1))
let f = run.events()                                  // an ods Frame, a row an event
f.group_by("kind").agg(#{ "n": ods.count() })         // what the run did, by kind
ods.filter(f, f["fault"])                             // every fault, with node and time
let arrivals = ods.filter(f, f["kind"] == "deliver")
arrivals["t"] - arrivals["sent"]                      // each message's latency
```

```bash
olang sim view .sim/5.simplan                  # a saved plan, replayed bit for bit, opened in the browser
olang sim view kv_partition_bug.ol --seed 5    # a file and a seed
olang sim view kv.ol --seed 5 --html out.html  # write the page (opens nothing): CI, a bug report
olang sim view kv.ol --seed 5 --json out.json  # the timeline as data, for another renderer
olang --no-ovm sim kv.ol --seeds 100           # worlds on the interpreter alone, as S1–S4 ran
```

## `run.events()` (`view.rs`)

A run's events as an `ods` Frame, one row an event, in the order they
happened. The columns:

| column | type | what |
|---|---|---|
| `seq` | Int | the event's place in the run (0, 1, …) |
| `t` | Int | virtual ms |
| `node` | Str | the node it happened on |
| `task` | Int, null | the task (the world's number); null for what the network did on its own |
| `kind` | Str | `send`, `recv`, `http`, `deliver`, `crash`, `partition`, `fsync`, … (S1–S4's kinds) |
| `detail` | Str | the event's rendering (cut at 120 characters, as before) |
| `peer` | Str, null | a message's other end: the sender of an arrival, the destination of a loss on leaving |
| `sent` | Int, null | when that message was handed to the network |
| `inbound` | Bool, null | an arrival at `node` (true), or a message lost or copied as it left it (false) |
| `fault` | Bool | the world injected it (the same faults as `run.faults`) |

`run.events` (no call) is still the list of maps S1 made: a fit's
method wins over its key of the same name (`native_sig::method_receiver`),
so `run.events()` is the method and `run.events` the field.

**What changed underneath.** `Event` gains `link` (the message's other
end and its send time; `wire.rs` stamps every message with the ms it was
sent) and `fault` (set by `fault_at`, and on the event a `record_fault`
fault already had: a loss's `drop`, a duplicate's `dup`, a partition).
Neither is part of the digest, so every digest S4 recorded still holds
(and saved plans still replay).

## `olang sim view`: the route

**Decision: one model, drawn by a self-contained page; the model is also
data another renderer can draw.**

`view::model(report)` turns a run into plain data: the lanes (one per
node that did anything, in the world's order), every event placed on its
lane with its class (`msg`, `lost`, `fault`, `life`, `error`, `edge`,
`tick`), each message as the two ends of an arrow, the stretches a node
was down (from its crash, kill, stop or exit to its restart, or the
run's end) and the partitions (from `partition` to `heal`, over the
lanes they cut apart). `view.html` (built in with `include_str!`) draws
it: one file, no network, no library, every byte the page needs inline.

Why not a native window first (the stdlib `gui` engine, or Loom):

- **Headless by nature.** A page is a file: CI writes one for a failing
  seed and keeps it as an artifact, a bug report carries it, and nothing
  needs a display or a main thread. The tests check the model exactly
  (lanes, arrows, faults, down stretches, digest) and the page's
  structure (self-contained, the model inside it, nothing loaded).
- **No dependency the other way.** olang must not depend on Loom (its
  own repo). A `gui` program inside olang is possible, but `gui` is
  refused inside a world and a window has to be driven to be tested;
  the view's logic would live in an olang program olang ships, tested
  through screenshots. The model in Rust is tested directly.
- **Big runs.** An hour of four clients is 77,000 events; olang-track's
  ten minutes 24,000; a long run, a million. The page draws on a canvas
  only what is in view (events sorted by time, binary-searched), and
  past 24,000 events in view it draws each lane's density in two-pixel
  columns instead (faults stay individual). SVG with a node an event
  would not scroll.
- **Loom and Studio later.** `--json` writes the model; a Loom panel or
  Studio draws the same data with `gui` (S6's "a Loom panel"), with
  nothing in the model to change. That is the step left for when a
  native view is wanted.

**What the page shows.** A header: the file and seed, and when the run
failed, what broke and when (the property and its time, or the error's
first line) in the accent colour. The lanes, one per node, labelled in a
gutter. Messages are arrows from the sender's lane at the send to the
receiver's lane at the arrival; a lost message is a dashed line ending
in a cross (a drop in flight ends at the receiver; a loss on leaving is a
short stub toward its destination). Faults are diamonds in the accent
colour, named when zoomed in; a node's down stretch is a band on its
lane; a partition is a tinted band over the lanes it cut, for as long as
it lasted. Restarts and exits are hollow squares; failures are rings.

- **Scrubbing.** A playhead runs through virtual time: drag on the time
  axis, or click or drag the overview strip at the bottom (the whole
  run's density, its faults as ticks, the part in view outlined). What
  happened after the playhead is drawn faint, so the picture is the run
  up to that instant. The playhead's time is shown on the axis.
- **Inspecting.** Clicking an event selects it; the inspector shows its
  time, node, task, kind (a fault says so), for a message its sender,
  send time and latency, its full detail, and the node's events around
  that instant (eight before and after; each one selectable). Hovering
  shows a tooltip.
- **Moving.** The wheel zooms at the pointer (a trackpad's pinch too),
  sideways scrolls pan, dragging pans, a double click or `F` shows the
  whole run. `←`/`→` step to the previous or next event, `⇧←`/`⇧→` to
  the previous or next fault, `+`/`−` zoom about the playhead,
  Home/End the run's ends, `/` the filter. The filter keeps the events
  whose kind, node or detail contains the text and fades the rest.
- **Where it opens.** On the violation's instant, else the first error,
  else the first fault, else the end.
- **Look.** Neutral greys, one accent (red, for faults and what broke),
  hairlines between regions, no gradients, no decoration, light and dark
  from the system's setting. No counts or statistics in the page: only
  the run.

**What it replays.** A plan (`.simplan`, or any file holding an
`olang_sim_plan`) runs its file with its seed, duration and forced
faults, as `--replay` does, and its digest is checked: a divergence is
said on stderr and the page shows the run as it ran now. Otherwise the
target is a file run with `--seed` (default 1), `--for` and
`--max-steps`. The program's output is kept quiet. A file that builds
several worlds shows the run `olang sim` reports for it (the last
failing one, or the last). Without `--html`/`--json` the page is written
to `.sim/<seed>.html` and opened with the system's opener (`open`,
`xdg-open`, `explorer`); `--no-open` only writes it.

## The bytecode tier through the hook

Worlds now run on the bytecode tier (and the JIT where the tier compiles
a pure body), with the interpreter's results.

**Why it is sound.** S1 already put the world's hook in
`BuiltinFunctions::call_internal`, found through a thread-local rather
than through the interpreter. The tier calls a builtin in one of two
ways:

1. **Through the bridge** (`execute_builtin_call`): every module call
   (`time.*`, `chan.*`, `http.*`, `fs.*`, `db.*`, `os.*`, `random.*`,
   `print`, …) goes through `BuiltinFunctions::call`, where the world
   answers or refuses it, as on the interpreter. The bridge is a second
   interpreter, seeded with the caps of the caller; `http.serve`,
   `db.transaction` and `os.on_shutdown`, which the world gives an
   interpreter, take the bridge's (its globals are the host's), as they
   do outside a world.
2. **Natively**, without the bridge: the float math of `CallBuiltin`
   (which falls back to the bridge for an out-of-domain argument), the
   owned-argument fast paths (`map_set`, `map_set_in`, `map_update`,
   `map_merge`, the spread), the higher-order loops (`map`, `filter`,
   `col.any`, `col.all`, `contains`, the O(1) probes), `cell`'s VM
   forms, and the JIT's named calls (`map_get`, `map_has_key`,
   `map_set`). All pure. `BytecodeVm::natives_without_bridge` lists them
   and the new test `the_bytecode_tiers_own_natives_are_pure` holds
   each to `effects::sim_handling` = `Pure`: a native path added later
   for an effect fails it. One exception was found and closed:
   `gui.flat*` runs natively for Loom's layout; inside a world it now
   takes the bridge, where `gui` is refused (as on the interpreter).

`spawn` never compiles (the tier refuses it: a task starts on the
interpreter, `sim::spawn`), so every task switch is the world's. A run's
digest includes its steps (yield points), so equal digests also say the
tiers made the same builtin calls in the same order.

**Where the tier is on.** `run_file`/`run_builder` (`olang sim`, its
workers, `olang sim view`), every program node (`node.rs`), and
`w.run`/`w.run_fn`, which no longer set the caller's tier aside. Clients,
hooks, predicates and spawned tasks are `thread_safe_clone`s, which carry
a fresh tier of their own. `olang --no-ovm sim …` (or
`OLANG_SIM_TIER=interp`) runs every world on the interpreter alone, as
S1–S4 did; `--jobs` workers are started with the parent's choice.
`OLANG_TIER_VERBOSE=1` shows what the tier promotes inside a world.

### Measured (M-class Mac, `fasttest` build, the same binary both ways)

| world | interpreter | bytecode tier | |
|---|---|---|---|
| olang-track, 10 simulated minutes, two agents, no crash (`CRASH=none MINUTES=10`, seed 1) | 7.49–7.74 s | 6.76–6.77 s | 1.12× |
| `log_safe.ol`, `--seeds 3` (the log store, ~25 crashes a seed) | 2.07 s | 1.53 s | 1.35× |
| `log_durable_safe.ol`, `--seeds 2` (an `always` over the disk at every instant) | 0.52 s | 0.43 s | 1.21× |
| `kv_world.ol`, `--seeds 3` (four clients, an hour each) | 1.27 s | 1.14 s | 1.11× |
| `kv_linear.ol`, `--seeds 3` (with the linearizability check) | 1.22 s | 1.14 s | 1.07× |
| `sqlite_honest.ol`, `--seeds 2` (SQLite, crashes mid-commit) | 1.60 s | 1.81 s | 0.88× |

(S4 measured olang-track's ten minutes at 7.0 s on a release build; the
rows here compare the two tiers on one fasttest binary, best of two.)

**Where olang-track's time goes now.** A `sample` of the bytecode-tier
run: of the samples in which a thread was busy, about 85–90% are inside
SQLite (`sqlite3VdbeExec`, B-tree moves, index record comparisons: the
snapshot rebuild and activity rows of each `issues.create`); the VM's
dispatch loop is about 1%. The tier took what the interpreter cost; the
rest is the application's SQL and the thread handoffs. About 89
simulated seconds per real second (0.025 simulated hours; S4's note
gave 0.086 for its 7.0 s, which is 600 s / 7.0 s read as minutes) for a
server and two agents, against S6's budget
of one hour for a server and four clients: S6 needs olang-track's
queries (or their indexes) looked at, not the language.

**sqlite_honest is slower** on the tier: a node that crashes restarts on
a fresh interpreter with a fresh tier, which compiles its functions
again at their first calls, and the world is otherwise bound by copying
synced file bytes (`Disk::sync_ino`, a `memmove` per sync). A higher
promotion threshold did not change it (tried 5, 20, 100). A tier shared
across a node's boots (its compiled code keyed by source) is the fix if
it matters.

## The guarantee, kept

`effects.rs` is unchanged. The cross-consumer test and the walk of the
stdlib pass as before; the new test holds the tier's own natives pure.
Every builtin a compiled function calls inside a world is answered or
refused by the world: `a_function_compiled_before_a_world_still_meets_the_world`
compiles `time.monotonic_ms`, `http.get` and `os.username` callers
before the world starts, then calls them inside it (virtual time, the
world's network, a refusal). The wasm build's stub gains
`bytecode_tier`/`set_bytecode_tier`.

## Smaller changes in this round

- `olang sim`'s report is on **stdout**, as `olang test`'s is: each
  failing seed, the summary, the digest, the reproduce line, `--events`,
  the last events of a failure, a shrink's summary and plan, a replay's
  "bit for bit". Errors, progress and a replay that diverged stay on
  stderr. `olang sim f.ol --seeds 100 | tail -3` now shows the summary.
- `--events` with `--seeds` is refused (exit 2) with what to do instead;
  it was silently ignored.
- A refusal reads "a world does not simulate processes yet" (it said
  "processes is not simulated yet"); `sim.world`'s refusal no longer
  names S1.
- `:help` has every `sim` member (S1–S5) and every `peg` function.

## The proof

All in `tests/sim_test.rs` after `// ─── S5`, run with `cargo test
--profile fasttest --test sim_test` (63 tests), plus
`effects::tests::the_bytecode_tiers_own_natives_are_pure`.

- **The Frame**: a row for every event; `ods.filter` on `kind` agrees
  with the list; `group_by("kind")` counts add up to every event; the
  column names. In a world of nodes: arrivals carry their sender as
  `peer`, nothing arrives before it was sent, `inbound` on every
  arrival, the partition row is a fault.
- **The model**: `log_noisy.ol` seed 1 has a lane per node, an arrow
  for every arrival (from a lane at or before its arrival), faults as
  marks, the spare's crashes as down stretches ending at their restarts,
  and its partitions as 1,000 ms spans.
- **`olang sim view`** from a file and seed writes a page that is one
  file (nothing loaded; the model inside it) titled with the file and
  seed, and JSON with the run's lanes and digest equal to `olang sim
  --seed 5`'s; from a shrunk plan, the plan's run bit for bit (its
  digest), the crash drawn as a fault. `olang sim view` with nothing to
  view says how.
- **The tiers agree**: `lost_update` (12 seeds), `deadlock`,
  `virtual_time`, `kv_partition_bug` (6), `kv_linear`, `log_bug`,
  `log_noisy`, `log_durable_safe`, `sqlite_lies`, `node_env`: the same
  sweep digest with `OLANG_SIM_TIER=interp` and `bytecode`; by hand also
  `kv_world`, `log_safe`, `log_props`, `log_world`, `sqlite_honest`,
  `sqlite_world` and olang-track's ten minutes (the same output and
  event count). `--jobs 2` workers inherit the tier (the same digest
  both ways and as one process). Every in-process S1–S4 test now runs
  its node programs and builder files on the tier too.
- **A compiled function meets the world**, above.
- **The CLI**: the report on stdout; `--events --seeds` refused; the
  refusal's sentence; `:help` for every `sim` member and `peg` function.

## Limits in S5

- **A page, not a window.** The view is drawn by a browser; a native
  view (Loom, Studio) is the model drawn with `gui`, not built here.
- **One run a page.** A file that runs several worlds shows the one
  `olang sim` reports; a shrunk plan beside its original (S4 asked for
  it) is not drawn side by side (two pages, or the model's JSON, compare
  them).
- **Node state.** The inspector shows events, not a node's memory at an
  instant: an interpreter's state is not recorded between yield points.
  A predicate (`sim.always`) or `sim.record` is the way to see values.
- **Detail is cut** at 120 characters, as S4 renders it; the full value
  is not kept.
- **Message pairing** is by arrival: a send shows no mark of its own;
  its arrival (or loss) draws the arrow back to it. A request's `http`
  event and its arrival are not linked by id.
- **The tier across restarts** compiles again on each boot (above).

## What S6 needs from this

- A `gui`/Loom renderer of `view::model` for Studio and the desktop
  app's simulation suites, if a native view is wanted.
- olang-track's speed is now SQL-bound: the budget (one simulated hour
  a second) is a question for its queries.
- A node's compiled code kept across its restarts.

## Docs

*Ready to fold into docs/stdlib.md's `sim` section and the book chapter
docs/sim.md. Written to stand beside S1–S4's text.*

### For docs/stdlib.md (the `sim` table)

| Function | Description |
|---|---|
| `run.events()` | every event of the run as an `ods` Frame, one row an event in the order they happened: `seq`, `t` (virtual ms), `node`, `task` (null for what the network did on its own), `kind`, `detail`, `peer` and `sent` (a message's other end and when it was sent), `inbound` (an arrival, rather than a loss as it left), `fault` (the world injected it). `run.events` without the call is the same events as a list of maps |

And under the command line:

| Command | What it does |
|---|---|
| `olang sim view PLAN` | replay a saved plan (`.sim/<seed>.simplan`) bit for bit and draw the run as a swimlane timeline in the browser |
| `olang sim view FILE --seed S` | run FILE with seed S (`--for`, `--max-steps` as `olang sim`) and draw it |
| `--html OUT` | write the timeline as one self-contained HTML page and open nothing |
| `--json OUT` | write the timeline's data (lanes, events, messages, faults, down stretches, partitions) as JSON |
| `--no-open` | write the page to `.sim/<seed>.html` without opening it |
| `olang --no-ovm sim …` | run worlds on the interpreter alone (`OLANG_SIM_TIER=interp` too); worlds run on the bytecode tier otherwise, with the same results |

`olang sim` prints its report (failing seeds, the summary, the digest,
the line that reproduces a failure, a shrink, a replay) on stdout and
errors and progress on stderr, as `olang test` does. `--events` shows
one run's events and needs `--seed`, not `--seeds`.

### For docs/sim.md (the book chapter)

#### Seeing a run

A failing seed is a story told in events: who sent what to whom, which
reply never came back, when the power went. Two ways to read it.

**As data.** `run.events()` is the run as an `ods` Frame, so the
questions you would ask a log are a line each:

```olang
let run = w.run(sim.hours(1))
let f = run.events()
f.group_by("kind").agg(#{ "n": ods.count() })          // what happened, how often
ods.filter(f, f["fault"])                              // every fault: when, where, what
let arrivals = ods.filter(f, f["kind"] == "deliver")
let slow = ods.filter(arrivals, arrivals["t"] - arrivals["sent"] > 500)
ods.filter(f, f["node"] == "kv")                       // one node's story
```

Each message's arrival carries its sender (`peer`) and the instant it
was sent (`sent`), so latencies, and the messages a partition cut, are
columns, not parsing.

**As a picture.** `olang sim view` draws a run as swimlanes, one per
node:

```bash
olang sim kv.ol --seed 5 --shrink         # saves .sim/5.simplan
olang sim view .sim/5.simplan             # the shrunk run, in the browser
olang sim view kv.ol --seed 5 --html kv-5.html
```

Messages are arrows from the sender's lane to the receiver's, a lost one
ends in a cross; faults are red marks, a node that is down is a band on
its lane, and a partition shades the lanes it cut for as long as it
lasted. Drag along the time axis (or the strip at the bottom) to move
the playhead: what happened after it fades, so the picture is the world
at that instant. Click an event to see it whole, with what its node did
just before and after. `⇧→` jumps to the next fault, `F` shows the
whole run, `/` filters.

The page is one file with nothing to fetch: keep it with a bug report,
or have CI write one for each failing seed (`--html`). `--json` writes
the same timeline as data.

A plan replays bit for bit, so the picture is the run that failed, not
one like it; `olang sim view` checks the plan's digest and says so if
this olang runs it differently.

#### Speed

Worlds run on the bytecode tier, as programs do outside one: every
builtin a compiled function calls is still answered by the world (its
clock, its network, its disks) or refused, and the runs are the same,
seed for seed, as on the interpreter alone (`olang --no-ovm sim`). What
a world costs then is mostly what its program does: for olang-track,
its SQL.

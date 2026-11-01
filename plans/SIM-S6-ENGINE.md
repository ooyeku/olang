# `sim` S6: the engine in practice (design note)

*Branch `sim-s6` from `rc6`, olang 1.0.0-rc.6 (local). The plan is
[SIM.md](SIM.md); the earlier notes are SIM-S1 to SIM-S5. Work paused
here by the owner, to consolidate: what is done is committed, what is
not is listed below.*

## Done (committed)

1. **Keyed draws** (`sched::Keyed`, `sched::Mix`). The world's own
   choices are no longer taken one after another from long streams. A
   draw is keyed by where it is made and by the virtual millisecond it is
   made in (and by its number within that millisecond):
   - a message's loss, duplication and latency: its link (sender and
     receiver) and the instant it is sent. A message a partition cuts
     draws too.
   - a scheduling pick, and the sticky schedule's draw: the instant.
   - `sim.maybe`: the point and its node. The draw is made whether the
     point is on or not.
   - a disk's lies, sync latencies, crash points and tears: the node and
     the instant.

   Removing a fault therefore moves nothing at instants the change never
   reached. On `kv_partition_bug.ol`, the failing seeds of 60 were shrunk
   with the build before and after. Before, 5 partitions that cut a
   client other than the one that failed were kept, across 9 failing
   seeds. After, none were kept, across 10 failing seeds, and every
   shrink reached `stick 1`. Every digest changed with this, so plans
   saved before it diverge.

2. **Failure signature**. A failure is now matched by the property's
   name. Without a property, it is matched by the outcome, the site the
   error was raised at (`Report.site`, `file:line:col`, from the
   interpreter's error location) and the *shape* of its first line
   (`shape`). The shape sets numbers, double-quoted text and what
   brackets hold aside. The new fixture is `log_noisy_list.ol`: its
   error lists the lost versions and every unanswered put. Shrunk by
   text, it stopped at 6 or 7 faults; it now reaches 1.

3. **`w.crash`, `w.kill` or `w.stop` on a node that is already down**
   now does nothing. The call leaves an `ignored` event; it is not a
   fault and it visits no fault site. Before, it raised "is down
   already".

4. **`sim.maybe`'s rate** was already what the code does: about a
   quarter of points are on in a run, and a point that is on fires a
   quarter of the times it is reached. The "about 15%" that was observed
   is the chance a point reached about three times fires at least once
   in a run. The docs now say so: about one run in sixteen for a point
   reached once, one in seven for a point reached three times. Each
   point now also draws on its own.

5. **Sync by replay** (`Disk::sync_ino`). A sync replays the writes made
   since the last sync onto the durable bytes. It used to copy the whole
   file. On `sqlite_honest.ol --seeds 2`, this change alone takes the
   run from 24.8G to 14.5G instructions.

6. **`olang sim view`**:
   - The lanes start at the top; the empty band above them is gone.
   - Zoomed out past one event every two pixels, a lane is drawn as how
     busy it was: a band, thick where busy. Beside it, a red rate shows
     the network's own losses and duplicates. Crashes, errors and
     restarts are clustered marks with a count.
   - The overview strip is an activity curve, with partitions shaded and
     only the rare faults ticked.

   Checked in the browser pane on `kv_world.ol` seed 1, which has about
   85,000 events.

Tests added to `tests/sim_test.rs`:
- `the_shrinker_sets_aside_a_failure_message_that_varies`
- `shrinking_under_partitions_keeps_only_the_cuts_of_the_client_that_failed`
- `crashing_a_node_that_is_down_already_is_an_event_not_an_error`
- `a_buggify_point_fires_about_a_quarter_of_the_time_in_a_quarter_of_the_runs_and_alone`

There are also unit tests in `net.rs` (a removed partition moves no
other message, and a message draws by its instant) and in `mod.rs`
(`shape` and the signature).

## Measured

Measured on an M-class Mac, `fasttest` build, while other sessions were
building, so wall times are noisy; instruction counts come from
`/usr/bin/time -l`.

| world | rc6 before | sim-s6 |
|---|---|---|
| `kv_world.ol --seed 1` (a server and four clients, one hour) | 0.41–0.47 s, 5.9G instr | 0.42 s, 5.9G instr: about 2.3 simulated hours a second |
| `kv_world.ol --seeds 3` | 1.24 s | 1.25 s |
| `kv_linear.ol --seeds 3` | 1.17 s | 1.22 s |
| `sqlite_honest.ol --seeds 2` | 1.8–2.4 s, 25.4G instr | 1.0–1.4 s, 14.5G instr |
| `log_safe.ol --seeds 2` | 18.1G instr | 18.4G instr (other draws, another run) |

The budget is at least one simulated hour per real second for a server
and four clients. `kv_world` meets it, at about 2.3 hours a second
unloaded and about 1.5 under load. olang-track does not meet it; its
time is its own SQL, which is another agent's work.

## Findings, not changed

- **Where a busy world's time goes.** In a `sample` of `kv_world`,
  about half the wall time no thread is running. That time is the baton
  passing between task threads, through a condition variable and the
  kernel. I tried spinning for the baton (4–40 µs) before sleeping: it
  was slower everywhere (user time 0.27 s → 0.7–3 s; wall time 0.44 s →
  0.7–0.95 s), so it was reverted. The remaining lever is fewer threads
  (tasks as coroutines), which is a redesign.
- **The bytecode tier across restarts** (item 6) was not done. It is
  neither straightforward nor the cause of the slowdown:
  - Without any crash, the tier is still about 2.3G instructions slower
    than the interpreter on the SQLite store (13.8G against 11.5G). With
    25 restarts a seed the gap is 2.6G, so restarts account for about
    2% of the run.
  - The compile cache already reuses compiled code across boots, keyed
    by source. With `OLANG_COMPILE_CACHE=0` the run takes 17.3G.
  - The gap comes from the store's `db.transaction(conn, (c) => …)`. Its
    lambda captures new values at each call, so the VM compiles it
    again each time (`hof_function_id`): it serialises the body and
    hashes it with sha256, and canonicalises a path in
    `set_current_def_file`.

  That is a VM matter (`ovm/bytecode.rs`), not a sim one.
- `assert_eq` on lists reports Rust's debug form (`List([Integer(2)])`),
  in and out of a world.

## Left undone

Uncommitted in the worktree (work stopped by the owner; built, not
tested, not reviewed):
- **E1**: in a world, `http.serve` returns at `http.shutdown` before its
  workers finish. The uncommitted change makes it wait, for up to
  `drain_ms`, until its workers have answered what is in flight or
  queued (`World::drain_wait`). `drain_ms` is made `pub(crate)`.
- **E2**: a client or a hook can now reach the cells of the program
  that built the world (`cell::act_for`, with the world's `home`
  thread). This is sound because only one task of a world runs at a
  time, and the builder's thread waits inside the world while they run.
  A task they spawn is still refused.
- **E3**: `gui.platform()` is answered in a world, as a fact about the
  build (like `runtime.features`), so Loom can load. Not yet checked
  against an actual Loom program in a world.
- **Refusal**: a request to a node whose process ended (killed, stopped,
  or its program returned) while its machine is still on is now refused
  at once. A node whose power went (`w.crash`) still answers nothing.
- The tests for these four are at the end of `tests/sim_test.rs`, and
  `docs/sim.md` has their text. None of it has been run.

Not started:
- The tier kept across restarts (see above).
- Further perf work.
- A `gui`/Loom renderer of the view model.
- The `doc_examples`, `doc_references`, `help_examples` and
  `bytecode_differential` suites have not been run on this branch; only
  `sim_test` and the lib's `sim::`/`effects` tests have.

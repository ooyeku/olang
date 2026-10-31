# `sim` S3: the disk and crashes (design note)

*Branch `sim-s3` from `rc6`, olang 1.0.0-rc.6 (local). The plan is
[SIM.md](SIM.md); S1's note is [SIM-S1.md](SIM-S1.md), S2's
[SIM-S2.md](SIM-S2.md). S3 keeps their scheduler and network unchanged.*

## What S3 adds

Each node now has a **disk**: an in-memory file system on which a file
keeps what was written apart from what was synced. `fs.*` answers from
it, **SQLite** reaches it through a VFS that can lie about `fsync`, tear
unsynced writes, fill up and slow down, and a **crash** keeps the disk
as it really was. A node is also a **process**: its own environment,
working directory, stdin, shutdown handlers and exit.

```olang
let w = sim.world(#{ "seed": 1 })
w.node("store", ["olang", "sqlite_store.ol"], #{
    "disk": sim.disk(#{ "fsync_lies": 0.01, "torn_writes": true, "full_at": 64 * 1048576, "slow": 2 }),
    "files": #{ "/etc/store.conf": "mode=strict" },      // durable from the start
    "cwd": "/srv", "stdin": "go\n", "env": #{ "SYNC": "FULL" },
    "crash": #{ "p": 0.0002, "restart_after": sim.seconds(1) } })
w.client("w1", () => writer("http://store:7600", "k1", sim.minutes(10)))
w.every(sim.seconds(20), () => w.crash("store", #{ "restart_after": sim.seconds(1) }))
w.at(sim.minutes(3), () => w.stop("store", #{ "restart_after": 1000 }))   // SIGTERM
w.at(sim.minutes(4), () => w.kill("store", #{ "restart_after": 1000 }))   // SIGKILL
w.at(sim.minutes(5), () => { w.write_stdin("store", "reload\n"); w.slow_disk("store", 50) })
let run = w.run(sim.minutes(15))
run.faults   run.disks     // what the world injected, what each disk did
sim.maybe("after_append")  // a buggify point: false outside a world
```

`fs.sync(path)` is new in the stdlib (outside a world it is `fsync`, of a
file or a folder). `olang sim FILE` now also takes a file whose world is
built by a local module it `use`s, and resolves the file's (and each node
program's) package dependencies as `olang run` does, fetching nothing and
writing no lock.

## The semantics

### When a write counts

A node's disk has two states per file and two namespaces (`disk.rs`):

- **A write** (`fs.write_file`, `write_bytes`, `append_file`, `copy_file`;
  SQLite's `xWrite` and `xTruncate`) changes what the node reads at once
  and what survives a crash not at all. So do creating, removing and
  renaming (`move_file`, `remove_file`, `create_dir`, SQLite's `xDelete`):
  the current namespace changes, the durable one does not.
- **`fs.sync(file)`** (SQLite: `xSync`) makes the file's bytes durable
  *and its own name* (its entry, and the directories above it). POSIX
  does not promise the name; many file systems keep it with the first
  fsync, and SQLite's unix VFS syncs the directory after creating a
  journal or WAL anyway (the VFS here does the same for the pager, which
  counts on it). A program that wants the strict rule loses nothing by
  also syncing the folder.
- **`fs.sync(dir)`** (SQLite: `xDelete` with `syncDir`) makes the
  folder's entries durable: what was created, removed or renamed directly
  in it.
- **Write + close is not durable.** There is no close in olang's `fs`, and
  nothing becomes durable without a sync: the strict rule, which is the
  one that finds bugs.

A sync takes one to three ms of virtual time (times the disk's `slow`),
drawn from the node's disk stream, and counts only when it returns: a
crash during it keeps nothing of it. With `fsync_lies: p` a sync answers
success and keeps nothing, with probability `p`; the lie is a fault.

### What a stop keeps

| how the node stops | memory | the disk | handlers |
|---|---|---|---|
| `w.crash(n)` (power lost) | gone | the durable state only | never |
| `w.kill(n)` (SIGKILL) | gone | every write (the OS has it; still unsynced, so a later crash can lose it) | never |
| `w.stop(n)` (SIGTERM) | gone once its program returns | every write | `os.on_shutdown` handlers run, each as a task of the node; with none, a kill |
| its program returns, or `os.exit(code)` | gone | every write | never |

`os.exit(code)` with a code other than 0 fails the run (as a raise does);
the node's other tasks stop and its servers close, as a process's would.
A program node whose main program returns ends the same way (S2 left
its spawned tasks running). `restart_after` brings a node back that many
ms after it is down (for a stop: after its program returned and its tasks
unwound): a fresh interpreter, the env, cwd and stdin it was given, and
the disk as the stop left it.

**A crash with `torn_writes`** may keep part of what was never synced:
each pending write, in the order it was made, lands whole, lands torn
(its first sectors, cut at a 512-byte boundary), or not at all (a third
each, from the disk's stream). Without it a crash loses every unsynced
write: the worst case, and deterministic. A truncation lands or not.

### SQLite through the VFS (`vfs.rs`, `dbs.rs`)

`db.open(path)` in a world opens `path` on the node's disk through a VFS
registered with SQLite as `olang-sim` (`rusqlite::ffi`, `sqlite3_vfs_
register`), never as a URI (a `vfs=` parameter could leave the disk).
Every file SQLite opens for the connection is a node file: the database,
its rollback journal, its WAL; temporary files are the handle's own
memory. Locks (`xLock`, the five SQLite levels across a node's
connections) and the WAL index (`xShm*`, regions on the heap, eight lock
slots) are process memory: per node and per boot, gone with a crash or a
kill. The VFS draws its randomness (`xRandomness`) from the calling
task's stream and its clock (`xCurrentTime*`) from the world's.

- **Fault points.** `xSync` may lie. `xWrite`, `xTruncate`, `xSync` and
  `xDelete` are crash points for a node with `#{ crash }`: the disk
  *freezes* at the call (it and every later write fail), and the node
  crashes as the `db` call returns, so the disk is exactly as it was at
  that instant. A write past `full_at` is `SQLITE_FULL`. Torn writes come
  from the crash itself (above).
- **Time.** SQLite cannot be paused inside a call, so a sync is not a
  yield point: its time is owed and slept as the `db` call returns.
  Every `db` call is a yield point before it runs.
- **Ownership.** A connection carries an owner (`stdlib::db`'s
  `ConnEntry.owner`): 0 for the real disk, a world's number otherwise. In
  a world a `db` call on a connection (or cursor) from outside, from
  another node, or from a process that crashed is refused or answered
  `Err`, never run.
- **`db.transaction`** serializes on the world's scheduler (`Wait::Tx`)
  rather than a real lock, which would hang the baton.

**The unsafe code** is the `extern "C"` callbacks in `vfs.rs` and
nothing else. Each runs under `catch_unwind` (a panic is
`SQLITE_IOERR`, never an unwind into C), reads its `sqlite3_file` as the
`File` `x_open` made (`pMethods` set last, so a failed open has none;
the `Box<Handle>` freed only by `x_close`), and touches SQLite's buffers
within the lengths given. Shared-memory regions are boxed slices owned
by the `Shm` every mapping handle holds. The unit tests drive it with
rusqlite directly.

What the VFS showed about SQLite, all documented behaviour, all in
`vfs.rs`'s tests:

- WAL at `synchronous=FULL` and DELETE at `EXTRA` keep every commit
  through crashes, torn writes included, and `PRAGMA integrity_check`
  stays `ok`.
- DELETE at `FULL` may roll back the **last** commit after a power cut:
  the journal's unlink is not synced, the journal comes back hot. Never
  more than that one, never a corrupt file.
- WAL at `NORMAL` loses the commits since the last checkpoint: the log
  is synced only then.
- A lying fsync loses committed rows (the whole database, when its
  creation was never really synced).

### A node's process (`life.rs`)

- `os.get_env/has_env/env/list_env` read the node's environment;
  `os.set_env/remove_env` change it for this process only.
- `os.cwd()` is the node's `cwd` (`/` unless given); relative paths in
  `fs` and `db` resolve against it. `os.pid()` is a number per node and
  boot, `os.hostname()` the node's name, `os.temp_dir()` its `/tmp`
  (every disk has one). `os.is_tty/stdin_is_tty/color_enabled` are
  false (a node's output is the world's events); `os.flush` does
  nothing; no signal reaches a world (`os.interrupted()` is false,
  `w.stop` is SIGTERM).
- `os.read_line()` reads the node's stdin: `#{ stdin }` and what
  `w.write_stdin(n, text)` adds, waiting on virtual time while it is open
  (`Wait::Stdin`); `w.close_stdin(n)` ends it and then it answers
  `Err("eof")`. A node given no stdin reads an empty, closed one, as
  `< /dev/null`.
- `os.on_shutdown(f)` keeps `f` for `w.stop`.

### Crashes at any point, and buggify

- **`#{ crash: #{ p, restart_after } }`** on a node: at every effect its
  tasks make (every simulated builtin call: a yield point or an effect
  boundary) and every write, truncation, sync and delete SQLite makes for
  it, the node crashes with probability `p`, drawn from its own stream
  (`split(split(seed, 4), node)`), and comes back `restart_after` ms
  later. The crash's detail names the point (`at fs.sync`, `at xWrite
  /data/kv.db-wal`).
- **`sim.maybe(point)`**: false outside a world. Inside one, the seed
  and the point's name alone decide whether the point is on for the run
  (a quarter are), so adding a point moves no other; one that is on is
  true a quarter of the times it is reached.
- **`run.faults`**: every fault the world injected, in order: `crash`,
  `kill`, `stop`, `fsync_lie`, `disk_full`, `slow_disk`, `maybe`, each
  with its time, node, detail and the scheduler's step. This is the list
  S4's shrinker minimises. `run.disks` is each disk's counts (writes,
  syncs, lies, lost, torn, full, crashes).

### Determinism

Every disk decision comes from a stream split from the world's seed:
per node, `split(split(seed, 3), node)` for lies, latencies and tears,
`split(split(seed, 4), node)` for crash points; `sim.maybe` from
`split(seed, 5)` and `split(split(seed, 6), fnv(point))`. Every table is a
`BTreeMap`; SQLite's randomness and clock are the world's. One seed is
one run, event for event, faults and disk counts included (tested, for
the log store and for SQLite with crashes inside commits).

## The guarantee, kept

`effects.rs`'s `sim_handling` moves to `Simulated`: every `db.*`; the
`fs` calls a disk answers (`read_file`, `write_file`, `read_bytes`,
`write_bytes`, `append_file`, `exists`, `is_file`, `is_dir`, `list_dir`,
`walk`, `glob`, `create_dir(_all)`, `remove_dir(_all)`, `remove_file`,
`copy_file`, `move_file`, `file_size`, `file_info`, `abs_path`,
`real_path`, `sync`); and `os.env/list_env/set_env/remove_env/cwd/
read_line/on_shutdown/exit/pid/hostname/temp_dir/is_tty/stdin_is_tty/
color_enabled/flush/path_separator/on_interrupt/interrupted/
reset_interrupt`. `runtime.wasm` joins `runtime.version` and
`runtime.features` as pure (the binary's own bytes). Still refused:
`fs.scan/search/watch*/trash`, `os.exec/stdin/stdin_lines/username/
home_dir/os_type/arch/family/exe_path/chdir`, `proc`, `pty`, `tty`, `gui`,
the rest of `runtime`, `asset`, and the OS-entropy crypto calls. The
walk-the-stdlib test now holds that every `db` member is simulated and
every `fs` member is simulated, pure or refused, as listed. No path in
a world reaches the real disk: `files.rs` and `vfs.rs` touch only the
node's `Disk`, and a test writes a file and a database inside a world at
real temp paths and checks neither appears.

## The proof

All in `tests/sim_test.rs` (fixtures in `tests/fixtures/sim/`), run
with `cargo test --profile fasttest --test sim_test`; unit tests for the
disk model and the VFS in `src/sim/{disk,vfs}.rs`.

- **(a) A durability bug, found and replayed.** `log_store.ol` keeps a
  key-value log on its disk, versions in order (a put of the next
  version only; 409 with the held version otherwise), replays the log at
  start and cuts a torn tail. `log_world.ol` runs it with three writers
  for ten simulated minutes, torn writes, a power cut every 20 s on a
  coin's toss, and random crash points (`p` 0.0005) between any two of
  the store's effects. `log_bug.ol` (acknowledge, then sync every
  500 ms) is caught on every seed tried; `olang sim log_bug.ol --seeds 8`
  names them, and `--seed S` replays the failure byte for byte, its
  events showing the crash and the unsynced writes it lost.
  `log_safe.ol` (sync, then acknowledge) passes every seed.
- **(b) SQLite.** `sqlite_store.ol` is the same protocol on SQLite (WAL,
  `synchronous=FULL`, one connection shared by four workers through
  `db.transaction`). `sqlite_honest.ol`: torn writes, power cuts, and
  crash points inside SQLite's own writes and syncs (in the middle of a
  commit): every seed passes. `sqlite_lies.ol`: the disk lies about
  fsync half the time, and every seed tried loses an acknowledged put.
- **(c) Determinism**: the log store's and SQLite's worlds, run twice on
  one seed, have equal events, faults and disk counts; another seed
  differs.
- **(d) The process**: `life_server.ol` reads stdin, counts boots on its
  disk, keeps a shutdown handler; a client stops it (the handler runs,
  writes and syncs a file), crashes it (no handler; the removal it had
  synced holds), and ends it with a `quit` line on stdin (`os.exit(0)`).
  Env, stdin and the shutdown file come back as the table above says.
  `os.exit(3)` fails the run. `sim.maybe` is false outside a world and
  the same per seed inside. A node program in a package resolves its
  path dependency, and no lock is written.

### Measured (M-class Mac, release build, one process, `--seeds 10`)

| world | simulated per seed | real per seed | simulated hours per real second |
|---|---|---|---|
| `log_safe.ol`: the store, 3 writers, ~30 crashes, torn writes | 10 min | 0.68 s | 0.25 |
| `sqlite_honest.ol`: SQLite, 3 writers, ~30 crashes (some mid-commit), ~6,900 syncs | 10 min | 0.74 s | 0.22 |
| `log_bug.ol` / `sqlite_lies.ol`: 20 of 20 seeds fail | to the failure (21 s to 3 min) | 0.03 / 0.05 s | |
| `track_world.ol`: olang-track, 2 agents, no crash | 10 min | 7.8 s | 0.08 |
| `track_world.ol`: olang-track, kills and stops every ~30 s | 2 min + the check | 2.6 s | 0.01 |
| `kv_world.ol` (S2's, unchanged) | 1 h | 0.45 s | 2.24 (S2: 2.3) |

The disk costs little: a store with a sync per put runs at about a
quarter of S2's key-value world, most of it the same interpreter work.
olang-track is slower because it is a real application on the
interpreter tier: an `issues.create` is a transaction, an activity row, a
mention pass and a snapshot rebuild, about 19 ms of real time each; a
restart (module load, migrations, the doctor, the backup) is about
0.4 s.

## The stretch: olang-track as a node

**It runs, unchanged.** `track_world.ol` starts olang-track's headless
server (`api.ol` → `lib/server.ol`'s `run_server`) as a node, from the
checkout, with its sources (`lib/`, `static/`, `packages/`, the entries)
and the shelf libraries it reads as files (the web SDK's stylesheet and
browser modules, Shuttle's runtime, a `shelf.toml` naming them) copied
onto the node's disk by the builder (outside the world; read-only on the
checkout), its database at `/data/track.db` on the node's disk, and
`use shuttle`/`use web` resolved from the checkout's `olang.toml` and
lock. Everything of the boot runs: the banner, the env check (it warns
that the node's shelf has no `.olang` packages, correctly), the data
folder, migrations through `db.migrate`, `harden` (WAL, NORMAL), the
corruption check, the run marker, the doctor, seed, purge, the daily
`VACUUM INTO` backup gzipped to the node's disk, Shuttle's jobs, the
meter, `os.on_shutdown`, and `serve` on four simulated workers. Two
agents file issues (`issues.create`) every 1–5 s and read each
acknowledged one back (`issues.get`) at the end.

What it took (no olang-track change): per-node `fs`, SQLite through the
VFS, `os.set_env`, `os.on_shutdown`, `os.pid`, `os.temp_dir`,
`os.color_enabled`, `runtime.wasm` as pure, package resolution for
`olang sim`, and the shelf's files on the node's disk.

**The durability check:**

- **Kills and stops** (SIGKILL, or SIGTERM with olang-track's drain):
  **1,000 seeds, 1,000 passed.** `CRASH=kill MINUTES=2 EVERY=20 olang
  sim track_world.ol --seed S --seeds 250`, four processes side by side
  (seeds 1–1,000), about 700 s of wall time each (fasttest build). A seed
  is two minutes of two agents filing about 70 issues, about three
  kills or stops (37 in a 12-seed sample: some 3,000 over the sweep;
  each graceful stop runs olang-track's two shutdown handlers, drains
  `serve` and returns in about 4 ms), restarts three seconds later (each
  boot finds the run marker set after a kill and says so), and the read
  back of every acknowledged issue. No acknowledged issue was ever lost.
  The run also shows olang-track's boot is idempotent under restarts:
  migrations, `seed`, `purge`, the backup and `backfill` all ran again
  and again on the same database without harm.
- **Power cuts**: olang-track **loses acknowledged issues**: 100 of 100
  seeds (`CRASH=power`, the same sweep's shape), and it reuses their ids (an issue another agent was told was
  `OLT-8` comes back as someone else's). The cause is `harden`'s
  `PRAGMA synchronous=NORMAL` in WAL mode: SQLite syncs the WAL only at a
  checkpoint (olang-track's every ten minutes), so a power cut or an OS
  crash takes every commit since. lib/server.ol's comment ("a crash
  loses at most ten minutes of checkpointing, never data") holds for a
  process crash, not for the machine's. `synchronous=FULL` (a sync per
  commit) would close it; whether that is worth its cost on a laptop is
  the owner's call. Not changed here.

## Limits in S3

- **The worst-case page cache.** A crash loses everything not synced;
  a real OS writes dirty pages back within about 30 s, so real losses
  are bounded where these are not. A `writeback` delay on the disk
  would model it.
- **A sync is atomic per call.** `fs.sync` is durable at its end; a
  crash during it keeps none of it (not part). Torn writes come only from
  the crash.
- **Crash points inside a `db` call** freeze the disk there, but the
  node unwinds when the call returns: code after a failed `db` call
  inside the same task does not run, but nothing it does could reach the
  disk anyway.
- **Not simulated:** `fs.scan/search/watch`, `fs.trash`, symbolic links,
  permissions (`readonly` is always false), `os.exec` and `proc`
  (refused), memory-mapped SQLite (`xFetch`: the VFS is version 2), and
  machine facts (`os.os_type`, `arch`: refused, so a seed replays the
  same on any machine).
- **Module loading still reads the real disk**: a node's `use` loads
  code from the real files, as S2's did; its `fs` reads see only its
  disk, so a program that reads its own sources (olang-track's build
  stamp) needs them copied on (`#{ files }`).
- **`print`** in a world still reaches the real stdout as well as the
  events (S1's behaviour).

## What S4 needs from this

- **The fault list.** `Report.faults` (`run.faults`) is every injected
  fault with its step; hook-driven ones (`w.crash` from `w.every`) are
  code, the rest (`#{ crash }` points, lies, `maybe`) are draws. The
  shrinker needs the draws replayable as a plan: a recorded run whose
  fault draws are forced (and dropped one at a time) rather than drawn.
  The draw sites are few: `vfs::NodeDisk::{draw_lie, draw_crash,
  sync_latency_us}`, `Disk::crash`'s tear draws, `World::maybe_crash`,
  `World::maybe`.
- **Properties over the disk.** `sim.always("an acknowledged write
  survives a crash", ...)` wants to read a node's durable state from a
  property; the disk is reachable from the world (`World.disks`), not
  yet from olang.
- **`--jobs`.** The 1,000-seed olang-track sweep ran as four processes by
  hand; `olang sim --jobs` should do that.
- **Speed.** A world still runs on the interpreter tier; olang-track's
  boot (module load, migrations) is most of a seed's time.

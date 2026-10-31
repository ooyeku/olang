// SQLite on a disk that tears what was not synced but never lies, with
// the power going at random moments and at random points inside its
// commits: no acknowledged put is ever lost.
use sqlite_world { run_sqlite }
let run = run_sqlite(0, true, 10)
let crashes = len(filter(run.faults, (f) => f.kind == "crash"))
println(run.outcome + ": " + to_string(crashes) + " crashes, " + to_string(run.disks.store.syncs) + " syncs")

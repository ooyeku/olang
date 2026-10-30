// The same store on a disk whose fsync lies half the time: a commit
// SQLite was told was durable is not, the next crash takes it, and a
// writer finds its acknowledged put gone.
use sqlite_world { run_sqlite }
let run = run_sqlite(0.5, false, 10)
println(run.outcome)

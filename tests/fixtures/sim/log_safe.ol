// The log store that syncs before it acknowledges: crashes lose only
// puts nobody was told were kept. Every seed passes.
use log_world { run_store }
let run = run_store("sync_then_ack", 10)
let crashes = len(filter(run.faults, (f) => f.kind == "crash"))
println(run.outcome + ": " + to_string(crashes) + " crashes, " + to_string(run.disks.store.lost) + " unsynced writes lost")

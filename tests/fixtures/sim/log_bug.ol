// The planted bug: the log store acknowledges a put before it syncs it
// (a task syncs every 500 ms). A crash between the answer and the sync
// loses a put the writer was told was kept, and the writer's check fails.
//
//   olang sim tests/fixtures/sim/log_bug.ol --seeds 20   (finds seeds)
//   olang sim tests/fixtures/sim/log_bug.ol --seed S     (replays one)
use log_world { run_store }
let run = run_store("ack_then_sync", 10)
println(run.outcome)

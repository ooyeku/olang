// The log store that syncs before it acknowledges, watched by the same
// property through crashes and torn writes: it holds on every seed.
use log_props { durable_world }
let run = durable_world("sync_then_ack", 2)
println(run.outcome + ": " + to_string(len(run.history("puts"))) + " entries in the history")

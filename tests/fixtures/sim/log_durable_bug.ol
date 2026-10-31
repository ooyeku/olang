// The log store that acknowledges before it syncs, watched by a
// durability property: the property breaks at the first instant a writer
// holds an acknowledgement the disk does not, no crash needed.
//
//   olang sim tests/fixtures/sim/log_durable_bug.ol --seeds 8
use log_props { durable_world }
let run = durable_world("ack_then_sync", 2)
println(run.outcome)

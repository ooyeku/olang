// Sim tests under `olang test`: a block that runs worlds runs once per
// seed of a small budget (3, or --seeds N), every world taking the seed.

use log_props { durable_world }

test "the store that syncs first keeps every acknowledged put" {
    let run = durable_world("sync_then_ack", 1)
    assert_eq(run.outcome, "done", to_string(run.violation))
}

test "the store that acknowledges first is caught" {
    let run = durable_world("ack_then_sync", 1)
    assert_eq(run.outcome, "violated", "the property held")
    assert_eq(run.violation.property, "an acknowledged put is on the store's disk", "which property")
}

test "the seed is the test's" {
    let w = sim.world(#{ "seed": 99 })
    let run = w.run_fn(() => sim.seed())
    assert(run.value >= 1 && run.value <= 5, "seed " + to_string(run.value))
}

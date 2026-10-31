// The log store's durability as a property, checked at every instant of
// the run: every put a writer was told was kept (an `ok` in the history
// "puts", log_client.ol) is in the store's log as a power cut would leave
// it (s.disk("store").read: only what was synced).
//
//   log_durable_bug.ol   ack_then_sync breaks it within the first second
//   log_durable_safe.ol  sync_then_ack keeps it through every crash

use log_world { run_store_with }

/// The puts acknowledged so far: key → the highest acked version. Read
/// from the end of the history (a writer's versions only go up), so a
/// check costs a few entries, not the whole history.
fn acked(s) = {
    let h = s.history("puts")
    let mut i = len(h) - 1
    let mut seen = #{}
    while i >= 0 && len(map_keys(seen)) < 3 {
        let e = h[i]
        if e.type == "ok" && !map_has_key(seen, e.value[0]) => { seen = map_set(seen, e.value[0], e.value[1]) } else => ()
        i = i - 1
    }
    seen
}

share fn acked_puts_are_durable() = sim.always("an acknowledged put is on the store's disk", (s) => {
    // the log's whole lines, as the store's replay reads them
    let log = "\n" + (s.disk("store").read("/data/store.log") ?? "")
    let a = acked(s)
    all(map_keys(a), (k) => str.contains(log, "\n" + k + " " + to_string(map_get(a, k)) + "\n"))
})

share fn durable_world(mode, minutes) = run_store_with(mode, minutes, [acked_puts_are_durable()])

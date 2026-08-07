// A second, structurally different machine on the same engine: a subway
// turnstile. Where the expense workflow is a mostly-linear approval pipeline,
// this one cycles — locked and unlocked hand off to each other indefinitely —
// and counts events in its context to show actions accumulating over a loop.

use lib.machine { transition, always }

// Bump a running counter held in the context (0 if not yet present).
fn bump(field) = (ctx, event) => {
    let seen = if map_has_key(ctx, field) => map_get(ctx, field) else => 0
    map_set(ctx, field, seen + 1)
}

// locked --coin--> unlocked, unlocked --push--> locked; the "wrong" event in
// each state is a self-loop that just tallies (a shove on a locked gate, a
// coin into an already-unlocked one).
share fn transitions() = [
    transition("locked", "coin", "unlocked", always, bump("coins")),
    transition("locked", "push", "locked", always, bump("blocked")),
    transition("unlocked", "push", "locked", always, bump("entries")),
    transition("unlocked", "coin", "unlocked", always, bump("wasted"))
]

share fn initial_state() = "locked"
share fn new_gate() = #{}

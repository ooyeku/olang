// A small, data-driven state machine engine.
//
// A machine is a set of transitions. Each transition names the state it
// leaves, the event that triggers it, and the state it enters — plus a guard
// (a predicate over the context and event) and an action (a pure context
// update). Guards let several transitions share a from-state/event pair and
// pick the one whose condition holds; actions thread evolving data through the
// run without mutation. Guards and actions are ordinary function values kept
// in the transition record, so the engine stays generic over any workflow.

// Build one transition. `guard(ctx, event) -> Bool`, `action(ctx, event) -> ctx`.
share fn transition(from, on, to, guard, action) = {
    from: from, on: on, to: to, guard: guard, action: action
}

// A guard that always fires, and an action that leaves the context untouched —
// the common case where a transition is unconditional and carries no data.
share fn always(ctx, event) = true
share fn keep(ctx, event) = ctx

// Take a single step. Returns a record describing the outcome:
//   { ok, state, ctx, note }
// The first transition that leaves `state`, matches the event name, and whose
// guard holds is taken; its action produces the next context. With no match,
// the machine holds its state and reports why.
share fn step(transitions, state, ctx, event) = {
    let candidates = transitions |> filter((t) =>
        (t.from == state) && (t.on == event.name) && t.guard(ctx, event))
    if len(candidates) == 0 => {
        ok: false,
        state: state,
        ctx: ctx,
        note: "reject " + event.name + " in " + state
    }
    else => {
        let t = candidates[0]
        {
            ok: true,
            state: t.to,
            ctx: t.action(ctx, event),
            note: state + " --" + event.name + "--> " + t.to
        }
    }
}

// Drive a whole event sequence, threading state + context and recording an
// audit trail. Returns { state, ctx, log } where `log` is the list of notes.
share fn run(transitions, initial, ctx, events) = {
    let start = { state: initial, ctx: ctx, log: [] }
    events |> fold(start, (world, event) => {
        let r = step(transitions, world.state, world.ctx, event)
        { state: r.state, ctx: r.ctx, log: world.log + [r.note] }
    })
}

//! state — the application's one store (browser side).
//!
//! State lives in a cell on the wasm side of the boundary — values
//! never cross into JavaScript and back, so they keep their olang
//! shapes exactly (an empty list stays an empty list). The store is
//! deliberately minimal: `current`, `set`, and `update(f)`.
//! Re-rendering belongs to `web.view` — its `apply(f)` is `update`
//! plus a repaint, and is what event handlers normally call.

let app_state = cell.new(())

/// Install the initial state. `web.view.mount` calls this.
share fn init(initial) = cell.set(app_state, initial)

/// The current state.
share fn current() = cell.get(app_state)

/// Replace the state wholesale.
share fn set(next) = cell.set(app_state, next)

/// Transform the state: `update((s) => map_set(s, "n", s.n + 1))`.
/// Returns the new state.
share fn update(f) = {
    let next = f(current())
    set(next)
    next
}

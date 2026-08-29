// Simulated work: each "fetch" takes `ms` milliseconds and returns a result
// tagged by name. `spawn` puts it on its own thread, so several run at once.
share fn fetch(name, ms) = {
    time.sleep(ms)
    { name: name, ms: ms }
}

// Start a fetch in the background. The caller gets a task handle, and
// nothing has blocked yet — the work is already running.
share fn start(name, ms) = spawn fetch(name, ms)

// Wait for a fetch, but no longer than `limit`.
//
// Note what this does and does not do: it bounds the *wait*, not the work.
// An OS thread cannot be cancelled from outside without leaving whatever it
// touched in an unknown state, so a timed-out fetch keeps running to
// completion — its result stays collectible from the same handle later.
// That is the honest shape of a timeout over real threads, and naming it
// `join_timeout` rather than `race` is the point.
share fn with_timeout(name, ms, limit) = {
    let t = start(name, ms)
    match task.join_timeout(t, limit) {
        Ok(r) => r
        Err(e) => { name: "TIMEOUT", ms: limit }
    }
}

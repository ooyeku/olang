// Simulated async work: each "fetch" resolves after `ms` milliseconds with a
// result tagged by name. Promise.delay(value, ms) models the latency.
share fn fetch(name, ms) = Promise.delay({ name: name, ms: ms }, ms)

// Race a promise against a timeout: whichever settles first wins.
// Returns { name, ms } on success or { name: "TIMEOUT", ms: limit }.
share fn with_timeout(name, ms, limit) = {
    let work = fetch(name, ms)
    let timeout = Promise.delay({ name: "TIMEOUT", ms: limit }, limit)
    await Promise.race([work, timeout])
}

use lib.fetcher { start, with_timeout }

println("═══ concurrent scheduler ═══")

// ── Fan-out: fetch four resources concurrently ──────────────────────
// `spawn` starts each one immediately on its own thread; joining them is
// just `map`, since by then they are all already running. Total wall time
// is the slowest, not the sum.
let jobs = [start("users", 120), start("orders", 80), start("stats", 150), start("health", 30)]
let results = jobs |> map(task.join)

println("── all completed (concurrently) ──")
for r in results {
    println("  " + str.pad_end(r.name, 8, " ") + to_string(r.ms) + "ms")
}
let slowest = col.max_by(results, (r) => r.ms)
println("slowest: " + slowest.name + " at " + to_string(slowest.ms) + "ms (total wall time ~ this, not the sum)")

// ── Timeouts: give each fetch a 100ms budget ────────────────────────
// The budget bounds how long we wait. A task that blows it keeps running.
println("── with a 100ms timeout budget ──")
for spec in [("fast", 40), ("slow", 250)] {
    let (name, latency) = spec
    let outcome = with_timeout(name, latency, 100)
    let verdict = if outcome.name == "TIMEOUT" => "timed out (still running)" else => "ok (" + to_string(outcome.ms) + "ms)"
    println("  " + str.pad_end(name, 6, " ") + verdict)
}

// ── A failed task is a value, not a crash ───────────────────────────
// `task.join` reports failure as `Err(e)`, so one bad worker cannot take
// the program down with it.
fn faulty() = 1 + "not a number"
match task.join(spawn faulty()) {
    Err(e) => println("faulty worker: reported, not fatal")
    v => println("unexpected: " + show(v))
}

println("═══ done ═══")

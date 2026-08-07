use lib.fetcher { fetch, with_timeout }

println("═══ concurrent scheduler ═══")

// ── Fan-out: fetch four resources concurrently ──────────────────────
// Create all the promises first (they start their timers immediately),
// then await them together. Total time = the slowest, not the sum.
let jobs = [fetch("users", 120), fetch("orders", 80), fetch("stats", 150), fetch("health", 30)]
let results = await Promise.all(jobs)

println("── all completed (concurrently) ──")
for r in results {
    println("  " + str.pad_end(r.name, 8, " ") + to_string(r.ms) + "ms")
}
let slowest = col.max_by(results, (r) => r.ms)
println("slowest: " + slowest.name + " at " + to_string(slowest.ms) + "ms (total wall time ~ this, not the sum)")

// ── Timeouts: race each fetch against a 100ms budget ────────────────
println("── with a 100ms timeout budget ──")
for spec in [("fast", 40), ("slow", 250)] {
    let (name, latency) = spec
    let outcome = with_timeout(name, latency, 100)
    let verdict = if outcome.name == "TIMEOUT" => "timed out" else => "ok (" + to_string(outcome.ms) + "ms)"
    println("  " + str.pad_end(name, 6, " ") + verdict)
}

println("═══ done ═══")

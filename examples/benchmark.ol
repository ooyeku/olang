// benchmark.ol — micro-benchmarks over olang's core pipeline operators.
//
// Each workload runs under a monotonic timer and returns a checksum, so the
// work cannot be optimized away and a changed result stays visible. The run
// ends with a per-workload table — time taken and share of the total — and
// the total wall-clock time.
//
// Run:  olang run examples/benchmark.ol

random.seed(7) // deterministic: the seeded workload reproduces run to run

let size = 500000

// Time a workload. `thunk` is a zero-argument function; its return value is
// kept as the checksum column.
fn bench(name, thunk) = {
    let start = time.monotonic_ms()
    let check = thunk()
    { name: name, ms: time.monotonic_ms() - start, check: check }
}

// Pad to a column width, right or left, for a tidy table.
fn rpad(s, w) = { let g = w - len(s); if g > 0 => s + str.repeat(" ", g) else => s }
fn lpad(s, w) = { let g = w - len(s); if g > 0 => str.repeat(" ", g) + s else => s }

let results = [
    bench("map · filter · sum", () => {
        range(size) |> map((v) => v * v) |> filter((v) => { v % 2 == 0 }) |> sum()
    }),
    bench("filter · map · take", () => {
        range(size) |> filter((v) => { v > 50 }) |> map((v) => v * v) |> take(100) |> sum()
    }),
    bench("reverse · take · sum", () => {
        range(size) |> map((v) => v * v) |> reverse() |> take(1000) |> sum()
    }),
    bench("math composition", () => {
        range(size)
            |> map((v) => (v * 1.0) |> math.sqrt() |> math.sin() |> math.cos() |> math.round())
            |> sum()
    }),
    bench("seeded random walk", () => {
        let mut acc = 0
        for i in range(size) { acc = acc + random.randint(0, 100) }
        acc
    })
]

// Total wall-clock time across every workload.
let mut total = 0
for r in results { total = total + r.ms }

// ── report ──
let bar = str.repeat("─", 60)
println(`olang benchmark  (${size} elements per workload)`)
println(bar)
println("  " + rpad("workload", 24) + lpad("time", 9) + lpad("share", 8) + "   checksum")
println(bar)
for r in results {
    let pct = if total > 0 => (r.ms * 100) / total else => 0
    println("  " + rpad(r.name, 24)
        + lpad(`${r.ms} ms`, 9)
        + lpad(`${pct}%`, 8)
        + "   " + show(r.check))
}
println(bar)
println("  " + rpad("total", 24) + lpad(`${total} ms`, 9)
    + `      over ${len(results)} workloads`)

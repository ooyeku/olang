// crunch.ol — the number-crunching gauntlet.
//
// One program that leans, hard, on everything the performance campaign
// shipped: tail recursion that never meets the depth cap, ordinary
// recursion ninety thousand frames deep, arbitrary-precision integers,
// bulk operations that fan out across every core on their own, explicit
// parallel kernels, and the parallel data stack. Every stage checks its
// answer against a value computed another way — the point is not that it
// runs fast, but that fast and correct are the same program.
//
// Run:            olang run examples/crunch.ol
// Ten times more: olang run examples/crunch.ol --heavy

let heavy = len(os.args()) > 1 && os.args()[1] == "--heavy"
let scale = if heavy => 10 else => 1

fn rpad(s, w) = { let g = w - len(s); if g > 0 => s + str.repeat(" ", g) else => s }
fn lpad(s, w) = { let g = w - len(s); if g > 0 => str.repeat(" ", g) + s else => s }

let started = time.monotonic_ms()
let mut stages = []
// Stages time themselves inline rather than through a thunk-taking
// harness: a function value invoked inside another function runs its
// user-function calls on the interpreter, and this file exists to
// exercise the compiled tiers.
let mut t0 = 0

// ── 1. tail recursion: millions of frames, one frame ──────────────────
// A self-call in tail position runs in the caller's frame on every tier,
// so depth is not a resource this function consumes. The check is the
// closed form, which shares no code with the recursion.
fn sum_down(n, acc) = if n <= 0 => acc else => sum_down(n - 1, acc + n)

t0 = time.monotonic_ms()
let n1 = 5000000 * scale
testing.assert_eq(sum_down(n1, 0), n1 * (n1 + 1) / 2)
stages = stages + [{ name: "tail recursion", detail: `${n1} frames deep`, ms: time.monotonic_ms() - t0 }]

// ── 2. deep ordinary recursion: the 100k cap is real ──────────────────
// `1 + probe(...)` needs its frame back — nothing to elide — and ninety
// thousand of those frames must actually fit, which the segmented stack
// guarantees. This stage does not scale: the cap is a constant.
fn probe(n) = if n <= 0 => 0 else => 1 + probe(n - 1)

t0 = time.monotonic_ms()
testing.assert_eq(probe(90000), 90000)
stages = stages + [{ name: "deep recursion", detail: "90000 non-tail frames", ms: time.monotonic_ms() - t0 }]

// ── 3. bigint: integers without a ceiling ──────────────────────────────
// 300! (615 digits), fib(1000) (209 digits), and a Fermat check with
// mod_pow — the numbers other languages compute natively, in olang since
// 0.70. The overflow error points here; this is what it points at.
t0 = time.monotonic_ms()
let f300 = fold(range(1, 301), bigint.of(1), (acc, i) => acc * i)
testing.assert_eq(len(to_string(f300)), 615)
testing.assert_eq(str.substring(to_string(f300), 0, 6), "306057")

let mut fib_a = bigint.of(0)
let mut fib_b = bigint.of(1)
for i in range(0, 1000) {
    let next = fib_a + fib_b
    fib_a = fib_b
    fib_b = next
}
testing.assert_eq(len(to_string(fib_a)), 209)
testing.assert_eq(fib_a % 10000000000, bigint.of(6849228875))

// 1000003 is prime, so 2^1000002 ≡ 1 (mod 1000003).
testing.assert_eq(bigint.mod_pow(bigint.of(2), bigint.of(1000002), bigint.of(1000003)), bigint.of(1))
testing.assert_eq(bigint.gcd(f300, bigint.pow(bigint.of(10), 60)), bigint.pow(bigint.of(10), 60))
stages = stages + [{ name: "bigint", detail: "300!, fib(1000), Fermat", ms: time.monotonic_ms() - t0 }]

// ── 4. automatic parallelism: a plain map, every core ─────────────────
// The kernel is provably pure (arithmetic and pure builtins only), the
// input is far past the 50k threshold, so this ordinary `map` fans out
// on its own. Results are bit-identical to sequential — checked against
// a fold that never parallelizes.
t0 = time.monotonic_ms()
let n4 = 1000000 * scale
let xs = range(0, n4)
let squared_sum = xs |> map((x) => x * x % 1000003) |> sum()
let by_fold = fold(xs, 0, (acc, x) => acc + x * x % 1000003)
testing.assert_eq(squared_sum, by_fold)
stages = stages + [{ name: "auto-parallel map", detail: `${n4} pure elements`, ms: time.monotonic_ms() - t0 }]

// ── 5. explicit parallel kernels: par_map on user functions ───────────
// A user-function kernel is beyond the purity proof, so the fan-out is
// explicit. collatz_steps is tail-recursive — each worker runs it as a
// native loop — and the known record under 100,000 is 350 steps, at
// n = 77031.
fn collatz_steps(n, steps) =
    if n == 1 => steps
    else => if n % 2 == 0 => collatz_steps(n / 2, steps + 1)
    else => collatz_steps(3 * n + 1, steps + 1)

t0 = time.monotonic_ms()
let steps = par_map(range(1, 100000), (n) => collatz_steps(n, 0))
testing.assert_eq(fold(steps, 0, (a, b) => math.max(a, b)), 350)
testing.assert_eq(steps[77030], 350)
stages = stages + [{ name: "par_map", detail: "collatz to 100000", ms: time.monotonic_ms() - t0 }]

// ── 6. the parallel data stack: parse, group, sort, join ──────────────
// The CSV text parses in record-boundary chunks across cores, group_by
// scatters in parallel, sort_by argsorts and gathers in parallel — and
// every aggregate is re-derived from the raw rows by plain folds.
t0 = time.monotonic_ms()
{
    let n = 150000 * scale
    let cats = ["alpha", "beta", "gamma", "delta"]
    // A tiny LCG keeps the data deterministic without touching random.
    let seeds = range(0, n) |> map((i) => (i * 1103515245 + 12345) % 100000)
    let lines = range(0, n)
        |> map((i) => `${i},${cats[i % 4]},${(i * 1103515245 + 12345) % 100000}`)
    let csv_text = "id,cat,score\n" + str.join(lines, "\n")

    let df = ods.read_csv(csv_text)
    testing.assert_eq(ods.n_rows(df), n)

    let by_cat = ods.group_by(df, ["cat"], [["total", "sum", "score"], ["rows", "count", ""]])
    testing.assert_eq(ods.n_rows(by_cat), 4)

    // Re-derive alpha's total from the seeds: alpha rows are i % 4 == 0.
    let alpha_expected = range(0, n)
        |> filter((i) => i % 4 == 0)
        |> fold(0, (acc, i) => acc + (i * 1103515245 + 12345) % 100000)
    let alpha_row = ods.filter(by_cat, ods.eq(ods.column(by_cat, "cat"), "alpha"))
    testing.assert_eq(ods.get(ods.column(alpha_row, "total"), 0), alpha_expected)

    // The row count survives a parallel sort and a self-join.
    let sorted = ods.sort_by(df, "score", true)
    testing.assert_eq(ods.n_rows(sorted), n)
    testing.assert_eq(ods.get(ods.column(sorted, "score"), 0), fold(seeds, 0, (a, b) => math.max(a, b)))

    let dim = ods.frame_from_records(map(range(0, 4), (i) => #{ "cat": cats[i], "rank": i }))
    let joined = ods.join_left(df, dim, "cat")
    testing.assert_eq(ods.n_rows(joined), n)
}
stages = stages + [{ name: "data stack", detail: `${150000 * scale} CSV rows`, ms: time.monotonic_ms() - t0 }]

// ── 7. the bundled collections: structures composing, all in olang ────
// A task-scheduling pipeline exercising every bundled module at once:
// dependencies topo-sorted over a CSR graph, earliest starts by
// Dijkstra over the same edges weighted, a heap draining tasks in
// priority order, a table counting by category, dsu grouping connected
// tasks, and a bitset tracking completion — cross-checked at each step.
t0 = time.monotonic_ms()
{
    let n = 2000 * scale
    // A layered DAG: each task depends on one or two earlier tasks.
    let mut edges = []
    let mut wedges = []
    for i in range(1, n) {
        let a = (i * 7919 + 13) % i
        edges = edges + [[a, i]]
        wedges = wedges + [[a, i, 1 + (i * 31) % 9]]
        if i % 3 == 0 && i > 1 => {
            let b = (i * 104729 + 7) % i
            edges = edges + [[b, i]]
            wedges = wedges + [[b, i, 1 + (i * 17) % 9]]
        }
    }
    let g = alg.graph(n, edges)
    let order = alg.topo_sort(g) |> unwrap
    testing.assert_eq(len(order), n)

    // Every edge points forward in the order — the topological contract.
    let mut position = col.filled(n, 0)
    for k in range(0, n) {
        position = col.set(position, order[k], k)
    }
    let mut forward = true
    for e in edges {
        if position[e[0]] >= position[e[1]] => { forward = false }
    }
    testing.assert_eq(forward, true)

    // Earliest reachable cost from the root, and hop counts agree with
    // reachability.
    let dist = alg.dijkstra(alg.wgraph(n, wedges), 0)
    let hops = alg.bfs(g, 0)
    let mut consistent = true
    for u in range(0, n) {
        if (dist[u] == -1) != (hops[u] == -1) => { consistent = false }
    }
    testing.assert_eq(consistent, true)

    // Drain the tasks by cost through the heap; verify sorted order.
    let mut h = heap.new()
    for u in range(0, n) {
        if dist[u] != -1 => { h = heap.push(h, dist[u], u) }
    }
    let mut done = bitset.new(n)
    let mut last = -1
    let mut ordered = true
    while !heap.is_empty(h) {
        let c = heap.top_prio(h)
        done = bitset.add(done, heap.top_item(h))
        h = heap.pop(h)
        if c < last => { ordered = false }
        last = c
    }
    testing.assert_eq(ordered, true)

    // The completion set is exactly the reachable set.
    let mut reachable = 0
    for u in range(0, n) {
        if dist[u] != -1 => { reachable = reachable + 1 }
    }
    testing.assert_eq(bitset.count(done), reachable)

    // Count tasks by cost band in the flat table; totals re-derived.
    let mut t = table.new()
    for u in range(0, n) {
        if dist[u] != -1 => {
            let band = dist[u] / 10
            t = table.put(t, band, table.get_or(t, band, 0) + 1)
        }
    }
    let mut banded = 0
    for v in table.values(t) {
        banded = banded + v
    }
    testing.assert_eq(banded, reachable)

    // Connectivity groups over the undirected edges match bfs
    // reachability from the root for the root's own group.
    let mut d = dsu.new(n)
    for e in edges {
        d = dsu.union(d, e[0], e[1])
    }
    let mut agree = true
    for u in range(0, n) {
        if dsu.connected(d, 0, u) != (hops[u] != -1) => { agree = false }
    }
    testing.assert_eq(agree, true)
}
stages = stages + [{ name: "collections", detail: `${2000 * scale}-task pipeline`, ms: time.monotonic_ms() - t0 }]

// ── the tally ──────────────────────────────────────────────────────────
let total = time.monotonic_ms() - started
println(`crunch: the number-crunching gauntlet${if heavy => " (heavy)" else => ""}`)
println("")
for s in stages {
    println("  " + rpad(s.name, 20) + rpad(s.detail, 28) + lpad(`${s.ms} ms`, 9))
}
println("")
println(`  ${len(stages)} stages verified in ${total} ms — every answer cross-checked`)

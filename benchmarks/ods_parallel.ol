// The data stack's automatic parallelism, op class by op class: the
// milliseconds one call takes at each size. Run it twice and compare —
// as is (the machine's threads) and with OLANG_PARALLELISM=1 (one):
//
//   olang benchmarks/ods_parallel.ol
//   OLANG_PARALLELISM=1 olang benchmarks/ods_parallel.ol
//
// Sizes default to 15k, 50k, 1M and 10M; `--sizes 15000,200000` (the
// script's own arguments, after `--`) picks others.

fn sizes() = {
    let args = os.args()
    let at = col.index_of(args, "--sizes")
    if at == () || at < 0 => [15000, 50000, 1000000, 10000000]
    else => str.split(args[at + 1], ",") |> map((s) => to_int(s))
}

// One call's time in ms: calls repeated until ~50M elements were touched
// (at least 3), the fastest of three rounds of that.
fn per_call(n, f) = {
    let reps = math.max(3, 50000000 / n)
    let mut best = 1e18
    for round in 0..3 {
        let t0 = time.monotonic()
        for i in 0..reps { f() }
        let ms = (time.monotonic() - t0) / to_float(reps)
        if ms < best => { best = ms }
    }
    best
}

fn row(name, n, ms) = println(`${str.pad_end(name, 14, " ")} ${str.pad_start(to_string(n), 9, " ")}  ${str.pad_start(str.fixed(ms, 4), 10, " ")} ms`)

let threads = os.env("OLANG_PARALLELISM") ?? "all"
println(`threads: ${threads}`)
for n in sizes() {
    let s = ods.linspace(1.0, 2.0, n)
    let t = ods.linspace(3.0, 1.0, n)
    let keys = ods.cast(ods.map(ods.linspace(0.0, 999.0, n), "floor"), "Int")
    let f = ods.frame([["k", keys], ["v", s]])
    row("mul+mean", n, per_call(n, () => ods.mean(s * t)))
    row("div+mean", n, per_call(n, () => ods.mean(s / t)))
    row("add+len", n, per_call(n, () => ods.len(s + 1.5)))
    row("sum", n, per_call(n, () => ods.sum(s)))
    row("std", n, per_call(n, () => ods.std(s)))
    row("dot", n, per_call(n, () => ods.dot(s, t)))
    row("compare", n, per_call(n, () => s > 1.5))
    row("sort", n, per_call(n, () => ods.sort(t)))
    row("group_by", n, per_call(n, () => ods.group_by(f, ["k"], [["m", "mean", "v"]])))
}

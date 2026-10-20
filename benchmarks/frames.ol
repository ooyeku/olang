// Grouping, joins, crosstabs, time buckets and rolling windows, as a
// program calls them: milliseconds per call, the fastest of seven. The
// data is frames_pandas.py's (run it first; it writes the CSV):
//
//   python3 benchmarks/frames_pandas.py /tmp/frames.csv
//   olang benchmarks/frames.ol -- /tmp/frames.csv
//   OLANG_PARALLELISM=1 olang benchmarks/frames.ol -- /tmp/frames.csv

use ods { col }

fn best(f) = {
    f()
    let mut t = 1e18
    for round in 0..7 {
        let t0 = time.monotonic()
        f()
        let ms = time.monotonic() - t0
        if ms < t => { t = ms }
    }
    t
}

fn row(name, ms) = println(`${str.pad_end(name, 45, " ")} ${str.pad_start(str.fixed(ms, 2), 10, " ")} ms`)

let args = os.args()
let path = args[len(args) - 1]
let df = ods.read_csv_file(path)?
let n = ods.n_rows(df)
let right = ods.frame({ k: ods.repeat(ods.arange(1000), 50), k2: ods.tile(ods.arange(50), 1000) })
    |> ods.with({ w: col("k") * 0.5 + col("k2") })
let right = right |> ods.with({ key: col("k") * 50 + col("k2") })
let qs = map(range(0, 10000), (i) => (i * 7) % 50000)
let by = df |> ods.group_by("k")

println(`threads: ${os.env("OLANG_PARALLELISM") ?? "all"}, n = ${n}`)
row("group_by agg: sum, mean, std, median, count", best(() => df.group_by("k").agg({
    s: col("x").sum(), m: col("x").mean(), sd: col("x").std(), med: col("x").median(), n: ods.count() })))
row("group_by agg: sum(x * y) / sum(y)", best(() => by.agg({ r: (col("x") * col("y")).sum() / col("y").sum() })))
row("transform: mean over k", best(() => by.transform(col("x").mean())))
row("x - mean over k (a with column)", best(() => df |> ods.with({ d: col("x") - col("x").mean().over("k") })))
row("group_by apply, 1,000 groups", best(() => by.apply((t) => ods.max(t["x"]) - ods.min(t["x"]))))
row("crosstab k2 x (k % 20)", best(() => ods.crosstab(df["k2"], df["k"] % 20)))
row("join on [k, k2], many_to_one", best(() => ods.join(df, right, ["k", "k2"], how: "left", validate: "many_to_one")))
row("resample 1d from text (parse + bucket)", best(() => ods.resample(df, "ts", "1d", { x: col("x").sum(), y: col("y").mean() })))
row("resample 1mo from text", best(() => ods.resample(df, "ts", "1mo", { x: col("x").sum() })))
row("rolling 365 mean", best(() => ods.rolling(df["x"], 365, "mean")))
row("rolling 365 mean, exact", best(() => ods.rolling(df["x"], 365, "mean", exact: true)))
row("rolling 365 std", best(() => ods.rolling(df["x"], 365, "std")))
row("rolling 365 max", best(() => ods.rolling(df["x"], 365, "max")))
row("rolling 24 median, centred", best(() => ods.rolling(df["x"], 24, "median", center: true)))
row("lookup: 10,000 scalar keys, one at a time", best(() => map(qs, (q) => ods.lookup(right, "key", q, "w"))))

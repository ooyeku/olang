// pipeline.ol — the DP4 end-to-end benchmark, ods edition.
//
// Eight stages of a realistic ETL pass over benchmarks/data/sales.csv;
// each prints `STAGE <name> <ms> <checksum>` and the run ends with
// `TOTAL <ms>`. The pandas and Polars editions print the same lines
// with the same checksums — the runner refuses a result where any
// checksum disagrees, so speed is only ever measured on identical
// answers.
//
//   olang run benchmarks/pipeline.ol

fn stage(name, t0, checksum) = {
    println(`STAGE ${name} ${time.monotonic_ms() - t0} ${checksum}`)
    time.monotonic_ms()
}

let started = time.monotonic_ms()
let mut t = started

// 1. load — the whole table off disk.
let sales = unwrap(ods.read_csv_file("benchmarks/data/sales.csv"))
t = stage("load", t, ods.n_rows(sales))

// 2. clean — drop the rows whose price is null, derive revenue.
let priced = ods.drop_null(sales)
let clean = ods.with_column(priced, "revenue",
    ods.cast(priced["units"], "Float") * priced["price"])
t = stage("clean", t, math.round(ods.sum(clean["revenue"])))

// 3. filter — the bulk orders.
let bulk = ods.filter(clean, clean["units"] >= 5)
t = stage("filter", t, ods.n_rows(bulk))

// 4. group — revenue and volume per region x category.
let grouped = ods.group_by(bulk, ["region", "category"], [
    ["revenue", "sum", "revenue"],
    ["avg_units", "mean", "units"],
    ["orders", "count"]
])
t = stage("group", t, `${ods.n_rows(grouped)}:${math.round(ods.max(grouped["revenue"]))}`)

// 5. join — attach each region's manager from the dimension table.
let dim = unwrap(ods.read_csv_file("benchmarks/data/regions.csv"))
let managed = ods.join(grouped, dim, "region")
t = stage("join", t, ods.n_rows(managed))

// 6. sort — best cell first.
let ranked = ods.sort_by(managed, "revenue", true)
let top = `${ods.to_list(ranked["region"])[0]}/${ods.to_list(ranked["category"])[0]}`
t = stage("sort", t, top)

// 7. daily — revenue per day, 7-day trailing mean over the calendar.
let daily = bulk
    |> ods.group_by(["date"], [["revenue", "sum", "revenue"]])
    |> ods.sort_by("date", false)
let smooth = ods.rolling(daily["revenue"], 7, "mean")
t = stage("daily", t, math.round(ods.sum(smooth)))

// 8. write — the ranked summary back to disk.
unwrap(ods.write_csv(ranked, "benchmarks/data/out_ods.csv"))
t = stage("write", t, ods.n_rows(ranked))

println(`TOTAL ${time.monotonic_ms() - started}`)

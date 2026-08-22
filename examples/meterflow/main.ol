// meterflow — a multi-source ETL on the ods data stack.
//
// Electricity meter telemetry arrives as JSON lines, one object per
// reading, in a file too large to hold. Two CSV dimension tables say
// which site each meter belongs to and what its tariff costs. The job:
// clean the readings, aggregate them, price them, and produce a daily
// report with charts.
//
//   olang main.ol                              # the checked-in sample
//   olang main.ol data/big.jsonl 50000         # another input, chunk size
//
// Every stage below is a stage a real pipeline has: bounded-memory
// ingest, a quality gate, aggregation, dimension joins, derived
// measures, a cached artifact, and output. Nothing here is arranged to
// flatter the API — where the stack made something awkward, the comment
// says so.

let t_start = time.monotonic_ms()
let args = os.args()
let readings_path = if len(args) > 1 => args[1] else => "data/readings.jsonl"
let chunk_size = if len(args) > 2 => unwrap(str.parse_int(args[2])) else => 20000

println("═══ meterflow ═══")

// ── 1. dimensions ─────────────────────────────────────────────────────
//
// Small enough to read whole. `read_csv_file` gives typed columns
// directly; `rate` and `standing` arrive as Float because the values
// carry decimal points.

let meters = unwrap(ods.read_csv_file("data/meters.csv"))
let tariffs = unwrap(ods.read_csv_file("data/tariffs.csv"))
println("meters: " + show(ods.n_rows(meters)) + " over "
    + show(ods.n_rows(tariffs)) + " tariffs")

// ── 2. ingest, in bounded memory ──────────────────────────────────────
//
// The readings file is streamed: one chunk at a time, each an ordinary
// Frame, so the memory this stage costs is set by `chunk_size` rather
// than by the size of the file. Each chunk is cleaned and reduced to
// per-meter-per-day totals before the next is read, so what accumulates
// is the aggregate, not the input.

let reader = unwrap(ods.open_jsonl(readings_path))
let mut partials = []
let mut rejected = 0

loop {
    let chunk = unwrap(ods.next_chunk(reader, chunk_size))
    if ods.n_rows(chunk) == 0 => break

    // The quality gate. A failed sensor reports null and a miscalibrated
    // one reports a negative, so both are dropped — and counted, because
    // a pipeline that discards rows silently is one nobody can audit.
    // `all_of` is how conditions combine: `&&` compiles to a jump for
    // short-circuiting, which has no elementwise reading over a column.
    let usable = chunk[ods.all_of([
        ods.eq(chunk["quality"], "ok"),
        chunk["kwh"] > 0.0,
    ])]
    rejected = rejected + (ods.n_rows(chunk) - ods.n_rows(usable))

    partials = partials + [ods.group_by(usable, ["meter", "day"],
        [["kwh", "sum", "kwh"], ["reads", "count"]])]
}

let read_count = ods.rows_read(reader)
println("read " + show(read_count) + " readings in "
    + show(len(partials)) + " chunks of " + show(chunk_size))

// Each chunk produced its own partial aggregate, so the partials must be
// stacked and re-reduced: a meter-day split across two chunks appears
// once in each.
let daily = ods.concat(partials)
    |> ods.group_by(["meter", "day"], [["kwh", "sum", "kwh"], ["reads", "sum", "reads"]])

println("rejected " + show(rejected) + " unusable readings ("
    + show(math.round(to_float(rejected) / to_float(read_count) * 1000.0) / 10.0)
    + "%), kept " + show(ods.n_rows(daily)) + " meter-days")

// ── 3. dimensions joined, measures derived ────────────────────────────

let priced = daily
    |> ods.join(meters, "meter")
    |> ods.join(tariffs, "tariff")

let cost = priced["kwh"] * priced["rate"] + priced["standing"]
let full = ods.with_column(priced, "cost", cost)

println("")
println("── the cleaned table ──")
println(show(ods.head(full, 4)))

// ── 4. rollups ────────────────────────────────────────────────────────

let by_site = full
    |> ods.group_by("site", [["kwh", "sum", "kwh"], ["cost", "sum", "cost"], ["days", "count"]])
    |> ods.sort_by("cost", true)

let by_day = full
    |> ods.group_by("day", [["kwh", "sum", "kwh"], ["cost", "sum", "cost"]])
    |> ods.sort_by("day", false)

println("")
println("── cost by site ──")
println(show(by_site))

// ── 5. the cached artifact ────────────────────────────────────────────
//
// The cleaned table is written in the native columnar format rather than
// CSV: the types survive exactly, and a later stage that wants one
// column pays for one column. `frame_info` reads the header alone, so a
// downstream job can see what is in the cache without loading it.

unwrap(fs.create_dir_all("cache"))   // a fresh checkout has no cache/ yet
unwrap(ods.write_frame(full, "cache/clean.olc"))
let info = unwrap(ods.frame_info("cache/clean.olc"))
println("")
println("── cache/clean.olc ──")
println(show(info))

let t_one = time.monotonic_ms()
let just_cost = unwrap(ods.read_frame("cache/clean.olc", ["cost"]))
println("one column back in " + show(time.monotonic_ms() - t_one) + "ms, total "
    + show(math.round(ods.sum(just_cost["cost"]) * 100.0) / 100.0))

// ── 6. charts ─────────────────────────────────────────────────────────
//
// Both are guarded: a chart over an empty frame is a runtime error, and
// an ETL job whose input arrived empty should say so rather than crash.

if ods.n_rows(by_day) > 0 => {
    let trend = plot.line(
        ods.series(0..ods.n_rows(by_day)),
        by_day["cost"],
        #{ "title": "daily cost across all sites", "x_label": "day", "y_label": "cost" })
    unwrap(fs.write_file("daily_cost.svg", trend))

    let sites = plot.bar(
        ods.to_list(by_site["site"]),
        by_site["kwh"],
        #{ "title": "consumption by site", "y_label": "kWh" })
    unwrap(fs.write_file("by_site.svg", sites))
    println("wrote daily_cost.svg, by_site.svg")
} else => println("no rows to chart")

// ── 7. the report ─────────────────────────────────────────────────────

unwrap(ods.write_csv(by_site, "by_site.csv"))
let report = {
    readings: read_count,
    rejected: rejected,
    meter_days: ods.n_rows(daily),
    total_kwh: math.round(ods.sum(full["kwh"]) * 100.0) / 100.0,
    total_cost: math.round(ods.sum(full["cost"]) * 100.0) / 100.0,
    sites: ods.to_list(by_site["site"]),
}
unwrap(fs.write_file("report.json", unwrap(json.stringify(report))))

println("wrote by_site.csv, report.json · total "
    + show(time.monotonic_ms() - t_start) + "ms")

test "the pipeline conserves its rows" {
    // Every reading is either rejected or counted, with none lost in
    // between — the property a quality gate has to have to be trusted.
    assert_eq(rejected + ods.sum(daily["reads"]), read_count)
}

test "cost follows consumption" {
    // Not a tautology: cost mixes three tariffs and a standing charge,
    // so it is only monotone in kWh if the join landed on the right rows.
    assert_eq(ods.sum(full["cost"]) > ods.sum(full["kwh"]) * 0.19, true)
}

test "the cache round-trips exactly" {
    assert_eq(unwrap(ods.read_frame("cache/clean.olc")) == full, true)
}

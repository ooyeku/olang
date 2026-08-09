// CSV → Frame → aggregate → JSON, on the ods data stack.
//
// The pipeline that used to be to_record maps and fold loops is now
// column arithmetic and Frame verbs: read_csv infers column types,
// revenue is one vectorized multiply, and revenue-by-region is a
// group_by instead of a hand-rolled map fold.

let args = unwrap(os.args())
let path = if len(args) > 1 => args[1] else => "data/sales.csv"

// ── load: CSV text → typed Frame ──
let raw = ods.read_csv(unwrap(fs.read_file(path)))
let revenue = ods.column(raw, "amount") * ods.column(raw, "quantity")
let sales = ods.with_column(raw, "revenue", revenue)

println("═══ sales data processor ═══")
println("records: " + to_string(ods.n_rows(sales)))

// ── revenue by region ──
let by_region = sales
    |> ods.group_by("region", [["revenue", "sum", "revenue"]])
    |> ods.sort_by("region", false)
println("── revenue by region ──")
for rec in ods.to_records(by_region) {
    println("  " + str.pad_end(map_get(rec, "region"), 6, " ") + "$" + to_string(map_get(rec, "revenue")))
}

// ── totals ──
let total = ods.sum(ods.column(sales, "revenue"))
let units = ods.sum(ods.column(sales, "quantity"))
println("── totals ──")
println("  revenue: $" + to_string(total))
println("  units:   " + to_string(units))

let top = head(sales |> ods.sort_by("revenue", true) |> ods.head(1) |> ods.to_records())
println("  top sale: " + map_get(top, "product") + " in " + map_get(top, "region") + " ($" + to_string(map_get(top, "revenue")) + ")")

// ── emit a JSON report ──
let report = {
    total_revenue: total,
    total_units: units,
    regions: ods.to_list(ods.column(by_region, "region")),
    top_product: map_get(top, "product")
}
let out = unwrap(json.stringify(report))
unwrap(fs.write_file("report.json", out))
println("── wrote report.json ──")
println(unwrap(json.prettify(out)))

// ── read the report back and pull fields by dynamic key ──
// A parsed JSON object reads through the same map accessors as a real map,
// so a caller can select a field named at runtime.
let loaded = unwrap(json.parse(unwrap(fs.read_file("report.json"))))
println("── verify (fields by runtime key) ──")
for field in ["total_revenue", "total_units", "top_product"] {
    println("  " + str.pad_end(field, 14, " ") + to_string(map_get(loaded, field)))
}

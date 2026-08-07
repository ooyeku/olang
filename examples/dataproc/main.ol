use lib.transform { to_record }
use lib.aggregate { revenue_by_region, top_sale }

let args = unwrap(os.args())
let path = if len(args) > 1 => args[1] else => "data/sales.csv"

let raw = unwrap(fs.read_file(path))
let rows = unwrap(csv.parse_with_headers(raw))
let records = rows |> map(to_record)

println("═══ sales data processor ═══")
println("records: " + to_string(len(records)))

// ── revenue by region ──
let by_region = revenue_by_region(records)
println("── revenue by region ──")
for region in sort(map_keys(by_region)) {
    println("  " + str.pad_end(region, 6, " ") + "$" + to_string(map_get(by_region, region)))
}

// ── totals ──
let total = records |> fold(0.0, (acc, r) => acc + r.revenue)
let units = records |> fold(0, (acc, r) => acc + r.quantity)
println("── totals ──")
println("  revenue: $" + to_string(total))
println("  units:   " + to_string(units))

let top = top_sale(records)
println("  top sale: " + top.product + " in " + top.region + " ($" + to_string(top.revenue) + ")")

// ── emit a JSON report ──
let report = {
    total_revenue: total,
    total_units: units,
    regions: map_keys(by_region),
    top_product: top.product
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

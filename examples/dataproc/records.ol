// records.ol — the same job as main.ol, without the data stack.
//
// This is the representation the data-stack chapter (docs/ods.md) compares
// against: the CSV parsed into a list of per-row maps, revenue computed by
// a map over rows, and the region totals accumulated by a fold. It exists
// so the chapter's representation comparison is reproducible — run both
// programs on the same generated input and compare wall-clock times:
//
//   olang gen.ol 200000
//   time olang records.ol data/sales_large.csv
//   time olang main.ol    data/sales_large.csv

let args = unwrap(os.args())
let path = if len(args) > 1 => args[1] else => "data/sales.csv"

// ── load: CSV text → list of row maps ──
let text = unwrap(fs.read_file(path))
let all = str.lines(str.trim(text))
let rows = skip(all, 1)

fn parse_row(line) = {
    let parts = str.split(line, ",")
    #{
        "region": parts[0],
        "product": parts[1],
        "amount": unwrap(str.parse_float(parts[2])),
        "quantity": unwrap(str.parse_int(parts[3]))
    }
}
let records = rows |> map(parse_row)

println("═══ sales data processor (records) ═══")
println("records: " + to_string(len(records)))

// ── revenue per row, total, and per-region totals, by fold ──
let with_rev = records
    |> map((r) => map_set(r, "revenue", map_get(r, "amount") * to_float(map_get(r, "quantity"))))

let total = with_rev |> fold(0.0, (acc, r) => acc + map_get(r, "revenue"))
let units = with_rev |> fold(0, (acc, r) => acc + map_get(r, "quantity"))

let by_region = with_rev |> fold(#{}, (acc, r) => {
    let region = map_get(r, "region")
    let sofar = if map_has_key(acc, region) => map_get(acc, region) else => 0.0
    map_set(acc, region, sofar + map_get(r, "revenue"))
})

println("── revenue by region ──")
for region in sort(map_keys(by_region)) {
    println("  " + str.pad_end(region, 6, " ") + "$" + to_string(map_get(by_region, region)))
}
println("── totals ──")
println("  revenue: $" + to_string(total))
println("  units:   " + to_string(units))

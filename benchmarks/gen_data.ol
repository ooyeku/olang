// gen_data.ol — deterministic dataset for the DP4 pipeline benchmark.
//
// Writes two CSVs into benchmarks/data/:
//
//   sales.csv    N rows: id, date, region, category, units, price
//                (every 50th price cell is empty — a null on read)
//   regions.csv  the dimension table: region, manager
//
// The generator is seeded, so every run of every engine reads the same
// bytes. Row count comes from argv (default 1,000,000).
//
//   olang run benchmarks/gen_data.ol [rows]

let args = os.args()
let n = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 1000000

random.seed(20260824)

let regions = ["north", "south", "east", "west", "coast", "plains", "delta", "ridge"]
let managers = ["Ada", "Grace", "Edsger", "Barbara", "Donald", "Tony", "Radia", "Ken"]
let categories = [
    "apparel", "audio", "auto", "bakery", "beauty", "books", "dairy", "decor",
    "garden", "grocery", "health", "kitchen", "media", "office", "outdoor", "pets",
    "produce", "seafood", "sports", "tools", "toys", "travel", "video", "wine"
]

// 730 calendar days across two years, precomputed once.
let mut days = []
for d in range(0, 730) {
    days = days + [unwrap(dates.format_date(unwrap(dates.add_days("2025-01-01", d)), "%Y-%m-%d"))]
}

// One row per index through `map` — a single-pass list build, no
// repeated appends. Draws stay deterministic: map is sequential and the
// draws per row are a fixed count.
fn row(i, days, regions, categories) = {
    let date = days[random.randint(0, 729)]
    let region = regions[random.randint(0, 7)]
    let category = categories[random.randint(0, 23)]
    let units = random.randint(1, 20)
    // Two-decimal price; every 50th row leaves the cell empty (null).
    let price = if i % 50 == 49 => ""
        else => {
            let cents = random.randint(99, 49999)
            `${cents / 100}.${str.pad_start(to_string(cents % 100), 2, "0")}`
        }
    `${i},${date},${region},${category},${units},${price}`
}
let lines = ["id,date,region,category,units,price"]
    + map(range(0, n), (i) => row(i, days, regions, categories))
unwrap(fs.write_file("benchmarks/data/sales.csv", str.join(lines, "\n") + "\n"))

let mut dim = ["region,manager"]
for i in range(0, 8) {
    dim = dim + [`${regions[i]},${managers[i]}`]
}
unwrap(fs.write_file("benchmarks/data/regions.csv", str.join(dim, "\n") + "\n"))

println(`wrote ${n} rows to benchmarks/data/sales.csv (+ regions.csv)`)

// gen.ol — generate a large sales CSV for the dataproc pipeline.
//
// The checked-in data/sales.csv is a nine-row sample so the example runs
// instantly under the harness. This script generates an input of any size
// with the same schema, for reproducing the measurements quoted in the
// data-stack chapter (docs/ods.md):
//
//   olang gen.ol                      # 200000 rows -> data/sales_large.csv
//   olang gen.ol 1000000              # choose the row count
//   olang gen.ol 200000 other.csv     # choose the output path
//
// Generation is seeded, so a given row count always produces the same file.

let args = os.args()
let rows = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 200000
let out = if len(args) > 2 => args[2] else => "data/sales_large.csv"

random.seed(40)

let regions = ["west", "east", "north", "south"]
let products = ["laptop", "mouse", "monitor", "keyboard", "dock", "webcam"]
let prices = [1200.00, 25.50, 320.00, 80.00, 150.00, 65.00]

fn row(i) = {
    let r = regions[random.randint(0, len(regions) - 1)]
    let p = random.randint(0, len(products) - 1)
    let q = random.randint(1, 12)
    `${r},${products[p]},${prices[p]},${q}`
}
let lines = ["region,product,amount,quantity"] + (range(rows) |> map(row))

unwrap(fs.write_file(out, str.join(lines, "\n") + "\n"))
println(`wrote ${rows} rows to ${out}`)

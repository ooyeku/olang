// ═══════════════════════════════════════════════════════════════════
// 02 — Data Pipeline
// Real analytics over a sales dataset using pipelines, closures, folds,
// and map aggregation. This is olang's sweet spot.
// ═══════════════════════════════════════════════════════════════════

println("═══ sales analytics ═══")

// A record is a tagged struct; a dataset is a list of them.
let sales = [
    { product: "Laptop",  amount: 1200.0, category: "Electronics", rep: "Ann" },
    { product: "Mouse",   amount: 25.0,   category: "Electronics", rep: "Bob" },
    { product: "Desk",    amount: 450.0,  category: "Furniture",   rep: "Ann" },
    { product: "Chair",   amount: 180.0,  category: "Furniture",   rep: "Cy"  },
    { product: "Monitor", amount: 320.0,  category: "Electronics", rep: "Bob" },
    { product: "Lamp",    amount: 60.0,   category: "Furniture",   rep: "Ann" },
    { product: "Cable",   amount: 15.0,   category: "Electronics", rep: "Cy"  },
    { product: "Bookcase",amount: 275.0,  category: "Furniture",   rep: "Bob" }
]

// ── Totals via pipeline ─────────────────────────────────────────────
let amounts = sales |> map((s) => s.amount)
let total = amounts |> fold(0.0, (acc, x) => acc + x)
let count = len(sales)
let average = total / to_float(count)
println(`transactions: ${count}`)
println(`total revenue: $${total}`)
println(`average sale:  $${average}`)

// ── Filtering: high-value transactions ──────────────────────────────
let big = sales
    |> filter((s) => s.amount >= 300.0)
    |> map((s) => s.product)
println(`>= $300: ${big}`)

// ── Group-by via fold into a map ────────────────────────────────────
// Sum revenue per category without a group_by builtin — fold is enough.
fn revenue_by(records, key_fn) = {
    records |> fold(#{}, (acc, r) => {
        let k = key_fn(r)
        let prior = if map_has_key(acc, k) => map_get(acc, k) else => 0.0
        map_set(acc, k, prior + r.amount)
    })
}

let by_category = revenue_by(sales, (s) => s.category)
println("revenue by category:")
for cat in sort(map_keys(by_category)) {
    println(`  ${cat}: $${map_get(by_category, cat)}`)
}

let by_rep = revenue_by(sales, (s) => s.rep)
println("revenue by rep:")
for rep in sort(map_keys(by_rep)) {
    println(`  ${rep}: $${map_get(by_rep, rep)}`)
}

// ── Leaderboard: rep with the most revenue ──────────────────────────
fn top_entry(m) = {
    let keys = map_keys(m)
    keys |> fold((head(keys), map_get(m, head(keys))), (best, k) => {
        let (best_key, best_val) = best
        let v = map_get(m, k)
        if v > best_val => (k, v) else => best
    })
}
let (leader, leader_rev) = top_entry(by_rep)
println(`top rep: ${leader} with $${leader_rev}`)

// ── Derived metrics: share of total per category ────────────────────
println("category share of total:")
for cat in sort(map_keys(by_category)) {
    let pct = map_get(by_category, cat) / total * 100.0
    let rounded = to_float(to_int(pct * 10.0)) / 10.0
    println(`  ${cat}: ${rounded}%`)
}

println("═══ analytics complete ═══")

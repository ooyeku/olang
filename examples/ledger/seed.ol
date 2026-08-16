// seed.ol — fill a ledger database with three months of plausible demo
// data, for trying the app without entering transactions by hand:
//
//   olang seed.ol              # seeds ledger.db
//   olang seed.ol demo.db      # or a named database
//
// Seeding is random but seeded, so repeated runs against fresh databases
// produce the same books. Running against a non-empty database adds rows;
// it never deletes.

use lib.store { open_store, seed_if_empty, list_categories, create_transaction, put_budget }
use lib.format { this_month, month_add }

let args = unwrap(os.args())
let db_path = if len(args) > 1 => args[1] else => "ledger.db"

random.seed(11)

let conn = open_store(db_path)
seed_if_empty(conn)
let cats = list_categories(conn)

fn cat_id(name) = {
    let hit = cats |> filter((c) => map_get(c, "name") == name)
    map_get(hit[0], "id")
}

// Plausible monthly shapes: [category, low_cents, high_cents, times_per_month]
let expense_plan = [
    ["Groceries", 3500, 14000, 8],
    ["Rent", 185000, 185000, 1],
    ["Transport", 250, 4500, 10],
    ["Dining", 1200, 8500, 6],
    ["Utilities", 4500, 16000, 3],
    ["Health", 1500, 9000, 2],
    ["Entertainment", 900, 6500, 4]
]

let months = [month_add(this_month(), -2), month_add(this_month(), -1), this_month()]
let mut made = 0

for month in months {
    // Income: one salary deposit on the 1st.
    create_transaction(conn, #{
        "date": month + "-01", "amount_cents": 520000,
        "category_id": cat_id("Salary"), "note": "salary" })
    made = made + 1
    // Expenses per the plan, spread across the month.
    for p in expense_plan {
        let mut i = 0
        while i < p[3] {
            let day = random.randint(1, 28)
            let day_s = if day < 10 => "0" + to_string(day) else => to_string(day)
            create_transaction(conn, #{
                "date": month + "-" + day_s,
                "amount_cents": 0 - random.randint(p[1], p[2]),
                "category_id": cat_id(p[0]),
                "note": "" })
            made = made + 1
            i = i + 1
        }
    }
    // Budgets for the big recurring categories.
    unwrap(put_budget(conn, cat_id("Groceries"), month, 60000))
    unwrap(put_budget(conn, cat_id("Dining"), month, 30000))
    unwrap(put_budget(conn, cat_id("Transport"), month, 20000))
    unwrap(put_budget(conn, cat_id("Entertainment"), month, 15000))
}

println("seeded " + to_string(made) + " transactions and 12 budgets across "
    + to_string(len(months)) + " months into " + db_path)

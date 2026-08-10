// Drive two different machines on the same engine and print, for each run,
// the audit trail, final state, and resulting context. The two workflow
// modules deliberately share function names (`transitions`, `initial_state`),
// so they are reached through their module namespaces.

use lib.machine { run }
use lib.expense
use lib.turnstile

fn event(name, actor) = { name: name, actor: actor }

fn show_ctx(ctx) = {
    let parts = map_keys(ctx) |> sort |> map((k) => k + "=" + to_string(map_get(ctx, k)))
    "{" + join(parts, ", ") + "}"
}

fn scenario(machine, title, ctx, events) = {
    println("── " + title + " ──")
    let result = run(machine.transitions(), machine.initial_state(), ctx, events)
    for line in result.log {
        println("   " + line)
    }
    println("   => state: " + result.state)
    println("   => ctx:   " + show_ctx(result.ctx))
    println("")
}

println("═══ expense approval ═══")

scenario(expense, "small expense, straight through", expense.new_expense(500), [
    event("submit", "ada"),
    event("approve", "manager_moe"),
    event("pay", "cashier_cy")
])

scenario(expense, "large expense, two approvals", expense.new_expense(2500), [
    event("submit", "ada"),
    event("approve", "manager_moe"),
    event("approve", "director_di"),
    event("pay", "cashier_cy")
])

scenario(expense, "rejected at review", expense.new_expense(800), [
    event("submit", "ada"),
    event("reject", "manager_moe")
])

scenario(expense, "illegal event is refused, state holds", expense.new_expense(500), [
    event("pay", "sneaky_sam"),
    event("submit", "ada")
])

println("═══ turnstile (same engine, a different machine) ═══")

scenario(turnstile, "pay, go through, then get blocked", turnstile.new_gate(), [
    event("coin", "rider"),
    event("push", "rider"),
    event("push", "freeloader"),
    event("coin", "rider"),
    event("push", "rider")
])

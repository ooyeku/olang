// console — how the harbor talks to a human: a term-styled dashboard
// when stdout is a terminal, plain single-line ticks when it is a pipe.
// Pure string building throughout — every render function returns text,
// so the whole dashboard is testable without a tty.

use term
use prelude { round1, money }
use schedule { day_label, utilization }

// One berth cell for the quay strip: id + occupant (or open water).
fn berth_cell(b) = {
    let label = if b.occupant == "" => "·" else => str.substring(b.occupant, 0, 4)
    let core = `${b.id}:${label}`
    if b.occupant == "" => core else => term.cyan(core)
}

// The live strip: "quay [0:ABCD 1:· 2:EFGH] 66% | roads 3"
share fn tick_line(tick: Int, berths, queue_len: Int, revenue: Float) = {
    let cells = berths |> map(berth_cell) |> join(" ")
    let util = to_int(utilization(berths) * 100.0)
    `${day_label(tick)}  quay [${cells}] ${util}%  roads ${queue_len}  revenue ${round1(revenue)}`
}

// The end-of-day table, term-aligned.
share fn day_table(rows) = {
    if len(rows) == 0 => "  (no charges today)"
    else => term.table(["kind", "n", "revenue"],
        rows |> map((r) => [map_get(r, "kind"),
                            show(map_get(r, "n")),
                            money(map_get(r, "revenue"))]))
}

// A utilization bar for the day summary.
share fn util_bar(fraction: Float) = term.bar(fraction, 24)

// Announcements, colored when they matter.
share fn announce(kind: String, text: String) = match kind {
    "arrive" => term.blue("⚓ " + text),
    "berth"  => term.green("⇒ " + text),
    "depart" => term.yellow("⇐ " + text),
    "alert"  => term.red("! " + text),
    _ => text
}

// The boot banner.
share fn banner(berth_count: Int, seed: Int, ticks: Int) = {
    let horizon = if ticks == 0 => "until interrupted (Ctrl-C to stop)" else => `${ticks} ticks`
    join([
        term.bold("HARBORLINE — a long-running olang system"),
        `  berths: ${berth_count}   seed: ${seed}   horizon: ${horizon}`,
        term.rule(56)
    ], "\n")
}

// ── Self-checks ─────────────────────────────────────────────────────
test "renders are plain text with the right shape" {
    let berths = [
        { id: 0, depth: 6.0, sealed: false, occupant: "ABCD-123" },
        { id: 1, depth: 7.7, sealed: false, occupant: "" }
    ]
    let line = tick_line(0, berths, 2, 45.5)
    testing.assert_true(str.contains(line, "roads 2"))
    testing.assert_true(str.contains(line, "50%"))
    testing.assert_true(str.contains(line, "0:ABCD"))
    testing.assert_true(str.contains(line, "1:·"))

    let table = day_table([#{ "kind": "bulk", "n": 2, "revenue": 132.0 }])
    testing.assert_true(str.contains(table, "bulk"))
    testing.assert_true(str.contains(day_table([]), "no charges"))

    testing.assert_true(str.contains(announce("alert", "hazmat waiting"), "hazmat"))
    testing.assert_true(str.contains(banner(4, 7, 0), "berths: 4"))
    testing.assert_true(str.contains(banner(4, 7, 48), "48 ticks"))
}

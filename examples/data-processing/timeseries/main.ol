// timeseries — analysis over ordered data, where each row's answer
// depends on the rows around it.
//
// The other data examples treat a table as an unordered bag: dataproc
// and meterflow reach for group_by, join, and describe, which answer
// "what is the total per region" or "what is the distribution" — the
// row's neighbors never matter. This one treats the same table as a
// *sequence*. A seven-day moving average smooths the week's rhythm; a
// day-over-day delta compares each point to the one before it; a running
// maximum remembers the worst so far; a rank orders every day by
// severity. These are the window verbs (`rolling`, `shift`, `cum_max`,
// `rank`) that the bag verbs cannot express, because they are questions
// about position in a series rather than membership in a group.
//
// The subject is a service's daily latency over four weeks, with a
// weekday/weekend rhythm, a slow upward creep, and a three-day incident.
// The data is generated deterministically — no random, no file — so the
// run reproduces exactly and the `test` block can pin the windows.
//
//   olang main.ol

// ── generate a deterministic latency series ─────────────────────────
// latency rises on weekdays (busier), creeps up slowly over the month,
// and spikes for a three-day incident. Pure and seeded by the day index,
// so every run is identical.
fn latency_for(day) = {
    let weekday = day % 7                          // 0..6, 0 = a Monday
    let season = if weekday < 5 => 30 else => 0    // weekdays run hotter
    let trend = day                                // a slow monthly creep
    let incident = if day >= 19 && day <= 21 => 220 else => 0
    90 + season + trend + incident
}

// A March date string for each day, 1..28 — one clean month, no rollover.
fn date_for(day) = "2026-03-" + str.pad_start(to_string(day + 1), 2, "0")

let days = range(0, 28)
let frame = ods.frame([
    ["date", days |> map(date_for)],
    ["latency_ms", days |> map(latency_for)]
])

println("═══ service latency — 28 days ═══")
println(`rows: ${ods.n_rows(frame)}`)

// ── the four windows, each a new column ─────────────────────────────
// A window verb reads a Series and returns a Series of the same length,
// so it composes with with_column exactly like arithmetic does. The
// first rows of a trailing window are null — the window is not full yet,
// and reporting a partial average as if it were whole is how a chart
// lies at its left edge.
let latency = frame["latency_ms"]

let analyzed = frame
    // seven-day moving average: the week's rhythm, smoothed away
    |> ods.with_column("ma7", ods.rolling(latency, 7, "mean"))
    // day-over-day change: each day compared to the one before it
    |> ods.with_column("delta", latency - ods.shift(latency, 1))
    // the worst latency seen up to and including each day
    |> ods.with_column("peak", ods.cum_max(latency))
    // severity rank, 1 = calmest day, 28 = worst
    |> ods.with_column("rank", ods.rank(latency, "min"))

// A Frame prints as a table; show the second week, where the moving
// average has warmed up and the numbers tell the ordinary story before
// the incident.
println("── days 8–14 (a normal week) ──")
println(to_string(ods.head(ods.tail(analyzed, 21), 7)))

// ── insights the windows make cheap ─────────────────────────────────

// The single biggest day-over-day jump — the incident's onset. `delta`
// has a null first row (nothing precedes day one), so drop it before
// sorting, or the null sorts to the end and the max is still right; here
// we sort descending and take the top row.
let by_jump = analyzed |> ods.drop_null(["delta"]) |> ods.sort_by("delta", true)
let onset = head(ods.to_records(ods.head(by_jump, 1)))
println("── biggest single-day jump ──")
println(`  ${map_get(onset, "date")}: +${map_get(onset, "delta")}ms  (to ${map_get(onset, "latency_ms")}ms)`)

// The three worst days by rank — the incident, found by ordering rather
// than by a hand-picked threshold.
let worst = analyzed |> ods.sort_by("rank", true) |> ods.head(3)
println("── three worst days (by severity rank) ──")
for rec in ods.to_records(worst) {
    println(`  #${map_get(rec, "rank")}  ${map_get(rec, "date")}  ${map_get(rec, "latency_ms")}ms`)
}

// Days that ran hot relative to their own trailing average — a spike is
// a point well above where the last week said it should be. The moving
// average is null for the first six days, so those never flag.
let ready = analyzed |> ods.drop_null(["ma7"])
// A comparison between two Series is a Bool mask, and a Frame indexed by
// a mask keeps the rows it marks — so the filter is one expression.
let flagged = ready[ready["latency_ms"] > ready["ma7"] * 1.4]
println("── days running >40% above their 7-day average ──")
for rec in ods.to_records(flagged) {
    println(`  ${map_get(rec, "date")}  ${map_get(rec, "latency_ms")}ms  vs ma7 ${math.round(map_get(rec, "ma7"))}ms`)
}

// The running peak never decreases: by the last day it holds the
// incident's high-water mark, whatever happened since.
println("── high-water mark by month end ──")
let last = head(ods.to_records(ods.tail(analyzed, 1)))
println(`  peak latency this month: ${map_get(last, "peak")}ms`)

// ── windows and the bag, together ───────────────────────────────────
// The sequence view does not replace the group view; they answer
// different questions. Average latency by weekday is a group_by — an
// unordered reduction — over the same frame.
let with_weekday = analyzed
    |> ods.with_column("weekday", ods.series(days |> map((d) => d % 7)))
let by_weekday = with_weekday
    |> ods.group_by("weekday", [["avg_latency", "mean", "latency_ms"]])
    |> ods.sort_by("weekday", false)
println("── average latency by weekday (0 = Mon) ──")
for rec in ods.to_records(by_weekday) {
    println(`  day ${map_get(rec, "weekday")}: ${math.round(map_get(rec, "avg_latency"))}ms`)
}

// ── self-checks: the windows compute what they claim ────────────────
test "window verbs on a known series" {
    let s = ods.series([3, 1, 4, 1, 5, 9, 2, 6])
    // rolling mean of window 3: first two null, then trailing averages.
    let ma = ods.to_list(ods.rolling(s, 3, "mean"))
    testing.assert_eq(ma[0], ())
    testing.assert_eq(ma[1], ())
    testing.assert_eq(ma[3], 2.0)              // (1 + 4 + 1) / 3
    // shift moves values down, filling the vacated head with null.
    let sh = ods.to_list(ods.shift(s, 1))
    testing.assert_eq(sh[0], ())
    testing.assert_eq(sh[1], 3)
    // cum_max never decreases.
    testing.assert_eq(ods.to_list(ods.cum_max(s)), [3, 3, 4, 4, 5, 9, 9, 9])
    // rank(min): ties share the lowest position; 1 is smallest.
    testing.assert_eq(ods.to_list(ods.rank(s, "min")), [4, 1, 5, 1, 6, 8, 3, 7])
}

test "the incident is found by ordering, not by a threshold" {
    // Regenerate the series the program analyzes and confirm the windows
    // locate the incident the same way the report does.
    let lat = ods.series(range(0, 28) |> map(latency_for))
    // The worst day is day 21 (Monday inside the incident): 361ms.
    let ranks = ods.to_list(ods.rank(lat, "min"))
    testing.assert_eq(ods.get(lat, 21), 361)
    testing.assert_eq(ranks[21], 28)           // highest rank = worst day
    // The running peak ends at the incident high, and never fell after.
    let peak = ods.to_list(ods.cum_max(lat))
    testing.assert_eq(peak[27], 361)
    // The biggest single-day jump is the incident onset at day 19. It is
    // +191, not the full +220 of the incident: day 18 is a weekday (its
    // season adds 30) and day 19 a weekend (season 0), so the spike lands
    // net of a 30ms weekend drop, plus 1ms of trend — 329 − 138.
    let delta_series = lat - ods.shift(lat, 1)
    testing.assert_eq(ods.to_list(delta_series)[19], 191)
    // and nothing else jumps as far: the normal weekly peak is Sun→Mon,
    // where the weekday season returns (+30) plus a day of trend (+31).
    // `ods.max` skips the null first element the shift leaves behind.
    testing.assert_eq(ods.max(delta_series), 191)
}

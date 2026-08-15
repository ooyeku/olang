// metrics — analytics over the day's traffic: fold-based group-by,
// leaderboards, an honest stddev cross-checked against the stats module,
// and ods Series where columnar wins.
//
// This module re-shares two prelude helpers (`pct`, `money`) so report
// code imports its whole analytics vocabulary from one place — the
// transitive-sharing pattern for building a layered API.

use prelude { pct, money, round1 }
share use prelude { pct, money }

// ── Group-by via fold: sum a numeric field per key ──────────────────
// No group_by builtin needed — fold into a map is the idiom.
share fn total_by(records, key_fn, amount_fn) =
    records |> fold(#{}, (acc, r) => {
        let k = key_fn(r)
        let prior = if map_has_key(acc, k) => map_get(acc, k) else => 0.0
        map_set(acc, k, prior + amount_fn(r))
    })

// The largest entry of a keyed total map: [key, value].
share fn top_entry(m) = {
    let keys = map_keys(m)
    if len(keys) == 0 => ["", 0.0]
    else => keys |> fold([head(keys), map_get(m, head(keys))], (best, k) => {
        let v = map_get(m, k)
        if v > best[1] => [k, v] else => best
    })
}

// Share-of-total per key, one decimal, as a sorted list of lines.
share fn share_lines(m) = {
    let total = map_keys(m) |> fold(0.0, (acc, k) => acc + map_get(m, k))
    sort(map_keys(m)) |> map((k) =>
        `${k}: ${money(map_get(m, k))} (${pct(map_get(m, k), total)}%)`)
}

// ── Descriptive statistics, two ways ────────────────────────────────
// The honest fold-based stddev...
share fn stddev(xs) = {
    let n = to_float(len(xs))
    let mean = (xs |> fold(0.0, (a, x) => a + x)) / n
    let variance = (xs |> fold(0.0, (a, x) => a + (x - mean) * (x - mean))) / n
    math.sqrt(variance)
}

// ...and the columnar route: an ods Series and the stats module. The
// self-check below proves both agree — the demo's tiers-of-abstraction
// invariant.
share fn describe_series(xs) = {
    let s = ods.series(xs)
    let d = stats.describe(s)
    {
        count: map_get(d, "count"),
        mean: map_get(d, "mean"),
        std: map_get(d, "std"),
        min: map_get(d, "min"),
        max: map_get(d, "max")
    }
}

// Dwell times: how long each departed vessel spent in harbor, from its
// movement rows (arrived tick -> departed tick).
share fn dwell_hours(movement_rows) = {
    let arrivals = movement_rows |> filter((m) => map_get(m, "event") == "arrived")
    let departures = movement_rows |> filter((m) => map_get(m, "event") == "departed")
    if len(arrivals) == 0 || len(departures) == 0 => 0
    else => map_get(head(departures), "tick") - map_get(head(arrivals), "tick")
}

// ── Self-checks ─────────────────────────────────────────────────────
test "group-by, leaderboard, shares" {
    let rows = [
        { kind: "container", amount: 100.0 },
        { kind: "bulk", amount: 60.0 },
        { kind: "container", amount: 40.0 }
    ]
    let by_kind = total_by(rows, (r) => r.kind, (r) => r.amount)
    testing.assert_eq(map_get(by_kind, "container"), 140.0)
    let top = top_entry(by_kind)
    testing.assert_eq(top[0], "container")
    let lines = share_lines(by_kind)
    testing.assert_eq(lines[0], "bulk: $60.00 (30.0%)")
    testing.assert_eq(lines[1], "container: $140.00 (70.0%)")
}

test "fold stddev agrees with the stats module" {
    let xs = [4.0, 8.0, 15.0, 16.0, 23.0, 42.0]
    let by_hand = stddev(xs)
    let d = describe_series(xs)
    // stats.describe reports SAMPLE std (n-1); convert to population to
    // compare against the fold (which divides by n).
    let n = to_float(len(xs))
    let population = d.std * math.sqrt((n - 1.0) / n)
    testing.assert_true(math.abs(by_hand - population) < 0.0001)
    testing.assert_eq(d.count, 6)
    testing.assert_eq(d.min, 4.0)
    testing.assert_eq(d.max, 42.0)
}

test "dwell time from movement rows" {
    let rows = [
        #{ "tick": 3, "event": "arrived", "berth": -1 },
        #{ "tick": 5, "event": "berthed", "berth": 1 },
        #{ "tick": 14, "event": "departed", "berth": 1 }
    ]
    testing.assert_eq(dwell_hours(rows), 11)
}

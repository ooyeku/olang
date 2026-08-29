// schedule — who berths where, and when: a BST-ordered waiting queue,
// depth-table allocation via binary search, a tide model built on the
// cache-threading idiom, and real calendar arithmetic for the log.
//
// Everything is a pure function from state to state: the world owns the
// queue and passes it in; a berth decision returns a new queue. That is
// the demo's central robustness pattern — no hidden mutation anywhere.

use prelude { sort_with, binary_search, memo_fib, clamp }
use cargo { needs_sealed_berth }

// ── A binary search tree keyed on priority ──────────────────────────
// The waiting queue is a BST of [priority, callsign] pairs: insertion
// keeps order, in-order traversal reads the queue best-first. Priority
// is an Int; ties break toward the earlier insertion (left subtree).
share type Queue = enum { Empty, Node(Queue, Int, String, Queue) }

share fn enqueue(q, priority: Int, callsign: String) = match q {
    Empty => Node(Empty, priority, callsign, Empty),
    Node(left, p, c, right) =>
        if priority > p => Node(insert_left(left, p, c, right, priority, callsign), p, c, right)
        else => Node(left, p, c, enqueue(right, priority, callsign))
}

// helper keeps the arms readable: pushing into the left subtree
fn insert_left(left, p, c, right, priority, callsign) = enqueue(left, priority, callsign)

// Best-first order: high priority first (reverse in-order would also work;
// we build it directly).
share fn queue_order(q) = match q {
    Empty => [],
    Node(left, p, c, right) =>
        concat(concat(queue_order(left), [[p, c]]), queue_order(right))
}

share fn queue_size(q) = match q {
    Empty => 0,
    Node(left, p, c, right) => 1 + queue_size(left) + queue_size(right)
}

// ── Priority policy: hazmat first, then the big ships ───────────────
share fn vessel_priority(v) = {
    let hazard = if len(v.hold |> filter(needs_sealed_berth)) > 0 => 100 else => 0
    hazard + to_int(v.draft * 5.0)
}

// ── Berth allocation ────────────────────────────────────────────────
// Berths are records { id, depth, sealed, occupant } — occupant "" when
// free. A vessel needs depth >= draft + safety margin under keel, and a
// sealed berth when carrying high-class hazmat.
share fn make_berths(n: Int) = map(range(0, n), (i) => {
    id: i,
    depth: 6.0 + to_float(i) * 1.7,     // deepening along the quay
    sealed: i == n - 1,                  // the last berth is the sealed one
    occupant: ""
})

// The shallowest free berth that fits: scan a sorted depth table with
// binary search to find the cut point, then take the first free berth
// at or beyond it. Returns berth id, or -1.
share fn find_berth(berths, v, needs_sealed: Bool, tide: Float) = {
    let need = v.draft + 0.5 - tide
    let fits = berths |> filter((b) =>
        b.occupant == "" && b.depth >= need && (!needs_sealed || b.sealed))
    if len(fits) == 0 => -1
    else => {
        let by_depth = sort_with(fits, (a, b) => a.depth < b.depth)
        head(by_depth).id
    }
}

// A depth table for the quay chart: sorted, searchable.
share fn depth_index(berths, metres: Float) = {
    let depths = berths |> map((b) => to_int(b.depth * 10.0))
    binary_search(depths, to_int(metres * 10.0))
}

// Occupy / release, functionally: a new berth list every time.
share fn occupy(berths, id: Int, callsign: String) =
    berths |> map((b) => if b.id == id =>
        { id: b.id, depth: b.depth, sealed: b.sealed, occupant: callsign }
        else => b)

share fn release(berths, callsign: String) =
    berths |> map((b) => if b.occupant == callsign =>
        { id: b.id, depth: b.depth, sealed: b.sealed, occupant: "" }
        else => b)

share fn utilization(berths) = {
    let busy = berths |> filter((b) => b.occupant != "")
    if len(berths) == 0 => 0.0 else => to_float(len(busy)) / to_float(len(berths))
}

// ── The tide ────────────────────────────────────────────────────────
// A toy semidiurnal tide: base sine plus a slow Fibonacci-weighted
// spring/neap swell. fib is memoized by THREADING the cache — the
// canonical replacement for in-place memoization under capture-by-value.
// The caller owns the cache and passes it back in every tick.
share fn tide_at(tick: Int, cache) = {
    let phase = to_float(tick % 12) / 12.0 * 6.28318
    let springs = memo_fib(15 + (tick / 144) % 5, cache)   // slow cycle
    let swell = to_float(springs[0] % 7) / 10.0
    [clamp(1.6 * math.sin(phase) + swell, -2.0, 2.5), springs[1]]
}

// ── Calendar: ticks are hours; the log speaks in dates ──────────────
share let EPOCH = "2026-08-01"

share fn date_of(tick: Int) = unwrap(dates.add_days(EPOCH, tick / 24))

share fn day_of(tick: Int) = tick / 24

share fn hour_of(tick: Int) = tick % 24

share fn day_label(tick: Int) = {
    let d = date_of(tick)
    let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
    let wd = names[unwrap(dates.weekday(d))]
    `${wd} ${d} ${str.pad_start(show(hour_of(tick)), 2, "0")}:00`
}

share fn days_since_epoch(d: String) = unwrap(dates.diff_days(d, EPOCH))

// ── Self-checks ─────────────────────────────────────────────────────
test "the queue orders best-first" {
    let q = enqueue(enqueue(enqueue(Empty, 10, "AAAA-111"), 90, "BBBB-222"), 40, "CCCC-333")
    let order = queue_order(q) |> map((e) => e[1])
    testing.assert_eq(order, ["BBBB-222", "CCCC-333", "AAAA-111"])
    testing.assert_eq(queue_size(q), 3)
}

test "berth allocation respects depth, tide, and sealing" {
    let berths = make_berths(4)     // depths 6.0, 7.7, 9.4, 11.1 (sealed)
    let small = { name: "x", callsign: "SMOL-100", draft: 5.0, hold: [], eta_tick: 0 }
    let deep = { name: "x", callsign: "DEEP-100", draft: 10.0, hold: [], eta_tick: 0 }
    // small ship gets the shallowest fit
    testing.assert_eq(find_berth(berths, small, false, 0.0), 0)
    // deep ship needs 10.5m: only the sealed 11.1m berth fits at zero tide
    testing.assert_eq(find_berth(berths, deep, false, 0.0), 3)
    // high tide opens the 9.4m berth to her
    testing.assert_eq(find_berth(berths, deep, false, 1.2), 2)
    // sealed requirement forces the sealed berth even for a small ship
    testing.assert_eq(find_berth(berths, small, true, 0.0), 3)
    // occupation removes it from the pool
    let taken = occupy(berths, 3, "DEEP-100")
    testing.assert_eq(find_berth(taken, small, true, 0.0), -1)
    testing.assert_eq(utilization(taken), 0.25)
    testing.assert_eq(utilization(release(taken, "DEEP-100")), 0.0)
}

test "the tide is bounded and its cache threads" {
    let first = tide_at(0, #{})
    let again = tide_at(6, first[1])
    testing.assert_true(first[0] >= -2.0 && first[0] <= 2.5)
    testing.assert_true(again[0] >= -2.0 && again[0] <= 2.5)
    // the threaded cache accumulates fib entries
    testing.assert_true(len(map_keys(again[1])) >= len(map_keys(first[1])))
}

test "calendar arithmetic" {
    testing.assert_eq(date_of(0), "2026-08-01")
    testing.assert_eq(date_of(49), "2026-08-03")
    testing.assert_eq(hour_of(49), 1)
    testing.assert_eq(days_since_epoch("2026-08-31"), 30)
    testing.assert_true(dates.is_leap_year(2028))
    testing.assert_eq(dates.days_in_month(2026, 8), 31)
}

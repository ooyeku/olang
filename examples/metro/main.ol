// metro — a transit planner for the fictional city of Arden, built on
// the `collections` module. Every submodule has a load-bearing job:
//
//   collections.table    station names to ids (and per-zone tallies)
//   collections.alg      the network as flat CSR graphs; Dijkstra for
//                        fastest routes, BFS for fare zones,
//                        sort_by_key and quickselect for the rankings
//   collections.heap     inside Dijkstra — and directly, draining the
//                        departure board in time order
//   collections.dsu      service continuity: which stations still
//                        reach which when links close
//   collections.bitset   the stranded set under a disruption
//
// Everything is deterministic and self-checking: the planner asserts
// its own answers (route symmetry, zone accounting, continuity counts)
// as it prints them, so the harness runs it like any other example.
//
// Run:  olang run examples/metro/main.ol

// The planner leans on five submodules; the import gives them their
// short names (fully qualified `collections.heap.push(...)` needs no
// `use` at all).
use collections { heap, table, dsu, bitset, alg }

// ── the network ───────────────────────────────────────────────────────
// Four lines, twenty-three stations, times in minutes between adjacent
// stops. A segment [a, b, mins] runs both ways; lines meet where they
// share a station, so transfers are free at the platform.

let lines = [
    ["Red", [
        ["Union", "Foundry", 3], ["Foundry", "Millbank", 2],
        ["Millbank", "Harbor", 4], ["Harbor", "Lighthouse", 6],
        ["Union", "Court", 2], ["Court", "Northgate", 3],
        ["Northgate", "Quarry", 5]]],
    ["Blue", [
        ["Union", "Market", 2], ["Market", "Grove", 3],
        ["Grove", "Stadium", 3], ["Stadium", "Airfield", 7],
        ["Union", "Chapel", 3], ["Chapel", "Westfield", 4],
        ["Westfield", "Orchard", 3]]],
    ["Green", [
        ["Market", "Foundry", 2], ["Grove", "Riverside", 2],
        ["Riverside", "Millbank", 3], ["Riverside", "Fairground", 4],
        ["Fairground", "Observatory", 5]]],
    ["Coast", [
        ["Harbor", "Boatyard", 2], ["Boatyard", "Dunes", 4],
        ["Dunes", "Lighthouse", 3], ["Boatyard", "Saltworks", 3],
        ["Saltworks", "Pierhead", 2], ["Pierhead", "Ferry", 2]]],
]

// ── station registry: names to dense ids, through the flat table ─────
let mut ids = table.new()
let mut names = []
for line in lines {
    for seg in line[1] {
        for end in [seg[0], seg[1]] {
            if !table.has(ids, end) => {
                ids = table.put(ids, end, len(names))
                names = names + [end]
            }
        }
    }
}
let n = len(names)

// Both directions of every segment, once as [u, v] and once weighted.
let mut hops = []
let mut timed = []
let mut segments = 0
for line in lines {
    for seg in line[1] {
        let a = table.get(ids, seg[0])
        let b = table.get(ids, seg[1])
        hops = hops + [[a, b], [b, a]]
        timed = timed + [[a, b, seg[2]], [b, a, seg[2]]]
        segments = segments + 1
    }
}

println("═══ metro: the Arden network ═══")
println(`  ${n} stations, ${segments} links, ${len(lines)} lines`)
println("")

// ── fastest routes: Dijkstra over the weighted CSR ────────────────────
let g_time = alg.wgraph(n, timed)
let g_hops = alg.graph(n, hops)
let union = table.get(ids, "Union")
let from_union = alg.dijkstra(g_time, union)

fn rpad(s, w) = { let g = w - len(s); if g > 0 => s + str.repeat(" ", g) else => s }

println("── fastest from Union ──")
for stop in ["Lighthouse", "Airfield", "Observatory", "Ferry"] {
    let sid = table.get(ids, stop)
    let mins = from_union[sid]
    // A route out equals the route back: every link runs both ways.
    testing.assert_eq(alg.dijkstra(g_time, sid)[union], mins) |> unwrap
    println(`  ${rpad(stop, 14)}${mins} min`)
}
testing.assert_eq(from_union[table.get(ids, "Lighthouse")], 15) |> unwrap

// ── fare zones: BFS rings from Union, tallied in a table ─────────────
// A station's zone is its stop count from Union — the shape fare
// schedules actually take.
let rings = alg.bfs(g_hops, union)
let mut zone_counts = table.new()
for sid in range(0, n) {
    zone_counts = table.put(zone_counts, rings[sid], table.get_or(zone_counts, rings[sid], 0) + 1)
}
println("")
println("── fare zones (stops from Union) ──")
let mut zoned = 0
for zone in alg.sort(table.keys(zone_counts)) {
    let count = table.get(zone_counts, zone)
    zoned = zoned + count
    println(`  zone ${zone}: ${count} stations`)
}
testing.assert_eq(zoned, n) |> unwrap

// ── the departure board: a heap drains in time order ─────────────────
// Next trains out of Union, keyed by minutes until departure.
let mut board = heap.new()
board = heap.push(board, 4, "Blue toward Airfield")
board = heap.push(board, 2, "Red toward Lighthouse")
board = heap.push(board, 7, "Blue toward Orchard")
board = heap.push(board, 2, "Red toward Quarry")
board = heap.push(board, 11, "Red toward Lighthouse")
println("")
println("── departures: Union ──")
let mut previous = -1
while !heap.is_empty(board) {
    let mins = heap.top_prio(board)
    let train = heap.top_item(board)
    board = heap.pop(board)
    testing.assert_eq(mins >= previous, true) |> unwrap
    previous = mins
    println(`  in ${mins} min  ${train}`)
}

// ── service continuity: dsu under a disruption ────────────────────────
// The Coast line floods: every Coast link closes. Union-find over the
// surviving links answers "who still reaches whom"; the bitset holds
// the stranded set.
let mut survives = dsu.new(n)
for line in lines {
    if line[0] != "Coast" => {
        for seg in line[1] {
            survives = dsu.union(survives, table.get(ids, seg[0]), table.get(ids, seg[1]))
        }
    }
}
let mut stranded = bitset.new(n)
for sid in range(0, n) {
    if !dsu.connected(survives, union, sid) => { stranded = bitset.add(stranded, sid) }
}
println("")
println("── disruption drill: Coast line closed ──")
println(`  islands: ${dsu.groups(survives)}  (was 1)`)
let mut cut = []
for sid in bitset.to_list(stranded) {
    cut = cut + [names[sid]]
}
println(`  stranded: ${str.join(alg.sort(cut), ", ")}`)
// Lighthouse still reaches Union over the Red line; the pure Coast
// stations do not.
testing.assert_eq(bitset.has(stranded, table.get(ids, "Lighthouse")), false) |> unwrap
testing.assert_eq(bitset.has(stranded, table.get(ids, "Ferry")), true) |> unwrap
testing.assert_eq(bitset.count(stranded), 5) |> unwrap

// ── rankings: the far side of the map ─────────────────────────────────
// Stations sorted by travel time from Union — sort_by_key runs its key
// once per station — and the median commute by quickselect, no sort.
let all_ids = range(0, n)
let farthest = alg.sort_by_key(all_ids, (sid) => 0 - from_union[sid])
println("")
println("── farthest from Union ──")
for k in range(0, 3) {
    println(`  ${rpad(names[farthest[k]], 14)}${from_union[farthest[k]]} min`)
}
let commutes = map(filter(all_ids, (sid) => sid != union), (sid) => from_union[sid])
let median = alg.select_kth(commutes, len(commutes) / 2)
println(`  median commute ${median} min`)
testing.assert_eq(median, alg.sort(commutes)[len(commutes) / 2]) |> unwrap

println("")
println("all checks passed ✓")

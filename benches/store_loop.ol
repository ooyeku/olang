// The store loop: a compiled `step` called by an interpreted driver,
// the store a 200-key map holding a 100-element list of 40-key maps, a
// short list of the same records, and an index of sixty of them by id.
// What an application's message loop does on every message — and what
// a deep conversion at the tier boundary made seven times slower than
// running with no tier at all. Compare: olang benches/store_loop.ol
// against olang --no-ovm benches/store_loop.ol.
fn row(i) = fold(0..40, #{ "id": i }, (m, k) => map_set(m, "f" + to_string(k), k + i))

fn store() = {
    let base = fold(0..200, #{}, (m, k) => map_set(m, "k" + to_string(k), k))
    let with_rows = map_set(map_set(base, "rows", map(0..100, (i) => row(i))), "count", 0)
    // What a real store also holds: a short list of records (under the
    // list wrapper's 64-element threshold, so it converted eagerly and
    // deeply) and an index of records by id.
    let with_recent = map_set(with_rows, "recent", map(0..8, (i) => row(i)))
    map_set(with_recent, "by_id", fold(0..60, #{}, (m, i) => map_set(m, "id" + to_string(i), row(i))))
}

// Compiled: reads two keys, writes one. Called 3,000 times.
fn step(m, msg) = {
    let n = map_get(m, "count") + map_get(msg, "by")
    let first = map_get(head(map_get(m, "rows")), "id")
    map_set(m, "count", n + first)
}

let t0 = time.monotonic_ms()
let mut m = store()
let msg = #{ "by": 1, "note": [1, 2, 3] }
let mut i = 0
while i < 3000 {
    m = step(m, msg)
    i = i + 1
}
println(`CHECK ${map_get(m, "count")}`)
println(`MS ${time.monotonic_ms() - t0}`)

let n = 3000000
let t0 = time.monotonic_ms()
fn count_words(n) = {
    let mut m = #{}
    let mut seed = 42
    let mut i = 0
    while i < n {
        seed = seed * 48271 % 2147483647
        let word = "w" + to_string(seed % 50000)
        let cur = if map_has_key(m, word) => map_get(m, word) else => 0
        m = map_set(m, word, cur + 1)
        i = i + 1
    }
    m
}
let m = count_words(n)
let mut maxf = 0
let mut distinct = 0
for k in map_keys(m) {
    distinct = distinct + 1
    if map_get(m, k) > maxf => { maxf = map_get(m, k) }
}
println(`CHECK ${distinct * 1000000 + maxf}`)
println(`MS ${time.monotonic_ms() - t0}`)

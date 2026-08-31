let n = 10000000
fn sieve(n) = {
    let mut composite = map(0..n, (i) => 0) + [0]
    let mut i = 2
    while i * i <= n {
        if composite[i] == 0 => {
            let mut j = i * i
            while j <= n {
                composite = col.set(composite, j, 1)
                j = j + i
            }
        }
        i = i + 1
    }
    composite
}
fn count_primes(composite, n) = {
    let mut count = 0
    for p in 2..(n + 1) { if composite[p] == 0 => { count = count + 1 } }
    count
}
let t0 = time.monotonic_ms()
let composite = sieve(n)
let count = count_primes(composite, n)
println(`CHECK ${count}`)
println(`MS ${time.monotonic_ms() - t0}`)

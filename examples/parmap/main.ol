// parmap — data-parallel pipelines on real OS threads.
//
// `par_map` is `map` fanned out across worker threads: one interpreter
// clone per worker (each with its own bytecode tier), contiguous chunks,
// order preserved. olang has no GIL — immutable values and capture-by-value
// closures make this safe in a way CPython structurally cannot offer.
//
// The one semantic difference from `map`, by design: the function runs
// against worker snapshots (exactly like `spawn`), so mutating enclosing
// state from inside it does nothing visible here.
//
// This program counts primes in 48 blocks of 1000 numbers, both ways,
// checks the answers agree exactly, and reports the speedup.

fn count_primes(block) = {
    let lo = block * 1000 + 2
    let hi = lo + 1000
    let mut count = 0
    let mut n = lo
    while n < hi {
        let mut is_prime = true
        let mut d = 2
        while d * d <= n {
            if n % d == 0 => { is_prime = false; break }
            d = d + 1
        }
        if is_prime => { count = count + 1 }
        n = n + 1
    }
    count
}

let blocks = range(0, 48)

let t0 = time.monotonic_ms()
let sequential = map(blocks, count_primes)
let t1 = time.monotonic_ms()
let parallel = par_map(blocks, count_primes)
let t2 = time.monotonic_ms()

let seq_ms = t1 - t0
let par_ms = t2 - t1

println("blocks: " + show(len(blocks)) + " x 1000 numbers")
println("primes found: " + show(sum(parallel)))
println("map:     " + show(seq_ms) + "ms")
println("par_map: " + show(par_ms) + "ms")
if par_ms > 0 && seq_ms / par_ms > 1 =>
    { println("speedup: " + show(seq_ms / par_ms) + "x") }

if sequential != parallel => {
    println("MISMATCH — par_map diverged from map")
    println(show(sequential))
    println(show(parallel))
}

// par_filter: same fan-out, filter's keep-on-true rule.
let dense = par_filter(blocks, (b) => count_primes(b) > 100)
println("blocks with >100 primes: " + show(dense))

test "par_map and par_filter agree with their sequential twins" {
    assert_eq(par_map([1, 2, 3], (x) => x * 2), map([1, 2, 3], (x) => x * 2))
    assert_eq(head(map(range(0, 48), count_primes)), 168)   // pi(1000) = 168
    assert_eq(sequential, parallel)
    assert_eq(sum(sequential), sum(parallel))
    assert_eq(par_filter(1..20, (x) => x % 5 == 0), filter(1..20, (x) => x % 5 == 0))
}

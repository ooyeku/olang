// parmap — data-parallel pipelines on real OS threads.
//
// `par_map` is `map` fanned out across worker threads: one interpreter
// clone per worker (each with its own bytecode tier), contiguous chunks,
// order preserved. olang has no GIL — immutable values and capture-by-value
// closures make this safe in a way CPython structurally cannot offer.
// `par for` (0.43) is the loop-construct twin: the same fan-out, for
// per-iteration effects rather than values, with an implicit barrier.
//
// The one semantic difference from `map`, by design: the function runs
// against worker snapshots (exactly like `spawn`), so mutating enclosing
// state from inside it does nothing visible here.
//
// This program counts primes in 48 blocks of 20,000 numbers, both ways,
// checks the answers agree exactly, and reports the speedup. (The block
// size is chosen so there is real work per element: as of 0.43 the JIT
// compiles count_primes to native code, so small blocks finish before
// the fan-out can pay for itself.)

fn count_primes(block) = {
    let lo = block * 20000 + 2
    let hi = lo + 20000
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

println("blocks: " + show(len(blocks)) + " x 20000 numbers")
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
let dense = par_filter(blocks, (b) => count_primes(b) > 1700)
println("blocks with >1700 primes: " + show(len(dense)))

// par for: the same fan-out as a loop construct. Use it for effects and
// heavy per-iteration work; when you want values back, that's par_map.
let t3 = time.monotonic_ms()
par for block in blocks {
    let c = count_primes(block)
    if c < 0 => println("unreachable: negative prime count")
}
let t4 = time.monotonic_ms()
println("par for: " + show(t4 - t3) + "ms (same fan-out, as a loop)")

// The snapshot rule: like spawn and par_map, each iteration runs against
// a worker's own clone of the environment. A write to an outer binding
// would land on that clone, so `par for b in blocks { sink = sink + 1 }`
// is refused before the program runs. Results cross the boundary by
// channel — or come back as values from par_map.
let counted = chan.new()
par for block in blocks { chan.send(counted, count_primes(block)) }
let mut tally = 0
for i in blocks { tally = tally + unwrap(chan.recv(counted)) }
chan.close(counted)
println("primes via par for + chan: " + show(tally))

test "par_map and par_filter agree with their sequential twins" {
    assert_eq(par_map([1, 2, 3], (x) => x * 2), map([1, 2, 3], (x) => x * 2))
    assert_eq(head(map(range(0, 1), count_primes)), 2262)   // pi(20000) = 2262
    assert_eq(sequential, parallel)
    assert_eq(sum(sequential), sum(parallel))
    assert_eq(par_filter(1..20, (x) => x % 5 == 0), filter(1..20, (x) => x % 5 == 0))
}

test "par for honors the spawn/par_map snapshot model" {
    // A write to an outer binding is refused before the program runs, so
    // the two ways across the boundary are a channel and par_map.
    let c = chan.new()
    par for x in [1, 2, 3] { chan.send(c, x) }
    let mut got = 0
    for i in [1, 2, 3] { got = got + unwrap(chan.recv(c)) }
    assert_eq(got, 6)
    let squares = par_map(0..4, (x) => x * x)
    assert_eq(squares, map(0..4, (x) => x * x))
}

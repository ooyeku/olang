// The interpreter's parallel path, timed against its sequential one.
//
// `par_map` workers each run a tree-walking interpreter, which allocates
// on nearly every step — so anything the runtime does per allocation or
// per call that touches state shared between threads shows up here as a
// ratio near (or above) 1, and nowhere else: the other benches run on the
// bytecode tier, which hardly allocates. (W21's allocation counter shared
// one slot between these workers and this ratio was 0.92; it is 0.25 on
// eight cores.) Run it with the tier off:
//
//     olang --no-ovm benches/par_interp.ol

fn work(block) = {
    let mut count = 0
    let mut n = block * 20000 + 2
    let hi = n + 20000
    while n < hi {
        let mut d = 2
        let mut prime = true
        while d * d <= n { if n % d == 0 => { prime = false; break }; d = d + 1 }
        if prime => { count = count + 1 }
        n = n + 1
    }
    count
}

let blocks = range(0, 8)
let t0 = time.monotonic()
let a = map(blocks, work)
let t1 = time.monotonic()
let b = par_map(blocks, work)
let t2 = time.monotonic()
println("map " + to_string(math.round(t1 - t0)) + " ms, par_map " + to_string(math.round(t2 - t1))
    + " ms, ratio " + to_string(math.round((t2 - t1) / (t1 - t0) * 100.0) / 100.0)
    + (if a == b => "" else => "  MISMATCH"))

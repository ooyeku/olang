// Run and time the N-body simulation. It is deliberately heavy — the default
// (200 bodies, 300 steps) is ~24 million body-interactions, each a handful of
// field reads, a square root, and several floating-point ops.
//
//   olang main.ol [bodies] [steps]
//
// The whole point is the bytecode tier: `accel_x`/`accel_y` promote and run
// the O(n^2) inner loop in bytecode. Compare the two tiers directly:
//
//   olang --no-ovm main.ol      # interpreter only
//   olang main.ol               # bytecode tier on (default)

use lib.nbody { make_bodies, simulate, total_momentum }

let args = os.args()
let n = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 120
let steps = if len(args) > 2 => unwrap(str.parse_int(args[2])) else => 150
let dt = 0.01

let bodies = make_bodies(n)
let p_start = total_momentum(bodies)

let t0 = time.monotonic_ms()
let result = simulate(bodies, steps, dt)
let elapsed = time.monotonic_ms() - t0

// Body-interactions: every step, every body pulls on every body, twice (one
// pass per acceleration component).
let interactions = steps * n * n * 2
let rate = if elapsed > 0 => (interactions / elapsed) * 1000 else => 0
let p_end = total_momentum(result)
let sample = result[n / 2]

println(str.fmt("N-body: {} bodies, {} steps, dt={}", n, steps, dt))
println("")
println(str.fmt("  body-interactions:  {}", interactions))
println(str.fmt("  wall clock:         {}ms", elapsed))
println(str.fmt("  throughput:         {} interactions/sec", rate))
println("")
println(str.fmt("  momentum drift:     {} -> {}  (conserved ~ 0)", p_start, p_end))
println(str.fmt("  sample body[{}]:   x={} y={}", n / 2, sample.x, sample.y))

// ── self-check: physics holds and the run is reproducible ──
test "n-body simulation is correct and deterministic" {
    // A small, fully-specified run whose outcome is fixed — this locks both
    // determinism and (because the test runs on the default bytecode tier)
    // agreement with the interpreter that produced the constant.
    let b = simulate(make_bodies(20), 50, 0.01)
    let check = b[10]
    assert_true(math.abs(check.x - 1.9984304393692718) < 0.0000001, "body 10 x drifted")
    assert_true(math.abs(check.y - (0.0 - 4.2451258921150306)) < 0.0000001, "body 10 y drifted")
    // Momentum is conserved: a disk started at rest stays at rest overall.
    assert_true(total_momentum(b) < 0.00001, "momentum was not conserved")
}

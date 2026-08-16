// An N-body gravitational simulation. Each body is a struct; the force on a
// body sums a contribution from every other body, reading five fields per
// interaction inside a hot O(n) loop — so a step is O(n^2) field accesses
// plus a square root per pair. This is exactly the shape the bytecode tier
// is meant to accelerate: `accel_x`/`accel_y` are self-contained, read fields
// off their arguments, call `math.sqrt`, and return a scalar, so they promote
// and run in bytecode.

share type Body = struct {
    x: Float, y: Float,
    vx: Float, vy: Float,
    mass: Float
}

// Squared softening length: keeps the 1/r^2 force finite when bodies get
// close (and makes a body's self-interaction contribute exactly zero).
// This used to be inlined as a literal because a module-level binding kept
// the kernels off the bytecode tier; the tier now bakes closure constants,
// so the named constant compiles.
let SOFTENING2 = 0.5

// Acceleration components on `bi` from every body in `bodies`. These are the
// hot kernels: a tight loop of field reads and floating-point math.
share fn accel_x(bi, bodies) = {
    let mut ax = 0.0
    for bj in bodies {
        let dx = bj.x - bi.x
        let dy = bj.y - bi.y
        let d2 = dx * dx + dy * dy + SOFTENING2
        ax = ax + bj.mass * dx / (d2 * math.sqrt(d2))
    }
    ax
}

share fn accel_y(bi, bodies) = {
    let mut ay = 0.0
    for bj in bodies {
        let dx = bj.x - bi.x
        let dy = bj.y - bi.y
        let d2 = dx * dx + dy * dy + SOFTENING2
        ay = ay + bj.mass * dy / (d2 * math.sqrt(d2))
    }
    ay
}

// Advance every body one timestep (semi-implicit Euler). Building the new
// bodies is O(n); the two `accel` calls per body are the O(n^2) work.
share fn step(bodies, dt) = bodies |> map((bi) => {
    let mut ax = accel_x(bi, bodies)
    let mut ay = accel_y(bi, bodies)
    let nvx = bi.vx + ax * dt
    let nvy = bi.vy + ay * dt
    Body { x: bi.x + nvx * dt, y: bi.y + nvy * dt, vx: nvx, vy: nvy, mass: bi.mass }
})

share fn simulate(bodies, steps, dt) = {
    let mut bs = bodies
    for s in 0..steps { bs = step(bs, dt) }
    bs
}

// A deterministic starting disk (golden-angle spiral), no RNG — so a run is
// exactly reproducible and the two tiers must agree bit for bit.
share fn make_bodies(n) = range(0, n) |> map((i) => {
    let fi = to_float(i)
    let angle = fi * 2.399963
    let r = math.sqrt(fi) * 1.5
    Body { x: r * math.cos(angle), y: r * math.sin(angle), vx: 0.0, vy: 0.0, mass: 1.0 }
})

// Total momentum magnitude — a conserved quantity (pairwise forces are equal
// and opposite), so it stays ~0 for a disk started at rest. A cheap physics
// check that the integrator is behaving.
share fn total_momentum(bodies) = {
    let px = bodies |> fold(0.0, (acc, b) => acc + b.mass * b.vx)
    let py = bodies |> fold(0.0, (acc, b) => acc + b.mass * b.vy)
    math.sqrt(px * px + py * py)
}

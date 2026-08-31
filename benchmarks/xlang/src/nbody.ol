let pi = 3.141592653589793
let solar = 4.0 * pi * pi
let days = 365.24
let n_steps = 2000000
let dt = 0.01
let mut x = [0.0, 4.841431442464721, 8.34336671824458, 12.894369562139131, 15.379697114850917]
let mut y = [0.0, -1.1603200440274284, 4.124798564124305, -15.111151401698631, -25.919314609987964]
let mut z = [0.0, -0.10362204447112311, -0.4035234171143214, -0.22330757889265573, 0.17925877295037118]
let mut vx = [0.0, 0.001660076642744037, -0.002767425107268624, 0.002964601375647616, 0.0026806777249038932]
let mut vy = [0.0, 0.007699011184197404, 0.004998528012349172, 0.0023784717395948095, 0.001628241700382423]
let mut vz = [0.0, -6.90460016972063e-05, 2.3041729757376393e-05, -2.9658956854023756e-05, -9.515922545197159e-05]
let mut m = [1.0, 0.0009547919384243266, 0.0002858859806661308, 4.366244043351563e-05, 5.1513890204661145e-05]
for i in 0..5 {
    m = col.set(m, i, m[i] * solar)
    vx = col.set(vx, i, vx[i] * days)
    vy = col.set(vy, i, vy[i] * days)
    vz = col.set(vz, i, vz[i] * days)
}
let mut px = 0.0
let mut py = 0.0
let mut pz = 0.0
for i in 0..5 {
    px = px + vx[i] * m[i]
    py = py + vy[i] * m[i]
    pz = pz + vz[i] * m[i]
}
vx = col.set(vx, 0, 0.0 - px / solar)
vy = col.set(vy, 0, 0.0 - py / solar)
vz = col.set(vz, 0, 0.0 - pz / solar)
fn energy(x, y, z, vx, vy, vz, m) = {
    let mut e = 0.0
    for i in 0..5 {
        let sq = vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i]
        e = e + 0.5 * m[i] * sq
        for j in (i + 1)..5 {
            let dx = x[i] - x[j]
            let dy = y[i] - y[j]
            let dz = z[i] - z[j]
            let d2 = dx * dx + dy * dy + dz * dz
            e = e - m[i] * m[j] / math.sqrt(d2)
        }
    }
    e
}
fn simulate(x0, y0, z0, vx0, vy0, vz0, m, n_steps, dt) = {
    let mut x = x0
    let mut y = y0
    let mut z = z0
    let mut vx = vx0
    let mut vy = vy0
    let mut vz = vz0
    let mut s = 0
    while s < n_steps {
        for i in 0..5 {
            for j in (i + 1)..5 {
                let dx = x[i] - x[j]
                let dy = y[i] - y[j]
                let dz = z[i] - z[j]
                let d2 = dx * dx + dy * dy + dz * dz
                let dist = math.sqrt(d2)
                let mag = dt / (d2 * dist)
                let mi = m[i] * mag
                let mj = m[j] * mag
                vx = col.set(vx, i, vx[i] - dx * mj)
                vy = col.set(vy, i, vy[i] - dy * mj)
                vz = col.set(vz, i, vz[i] - dz * mj)
                vx = col.set(vx, j, vx[j] + dx * mi)
                vy = col.set(vy, j, vy[j] + dy * mi)
                vz = col.set(vz, j, vz[j] + dz * mi)
            }
        }
        for i in 0..5 {
            x = col.set(x, i, x[i] + dt * vx[i])
            y = col.set(y, i, y[i] + dt * vy[i])
            z = col.set(z, i, z[i] + dt * vz[i])
        }
        s = s + 1
    }
    energy(x, y, z, vx, vy, vz, m)
}
let t0 = time.monotonic_ms()
let e = simulate(x, y, z, vx, vy, vz, m, n_steps, dt)
println(`CHECK ${to_int(math.round(e * 1000000.0))}`)
println(`MS ${time.monotonic_ms() - t0}`)

let n = 200000
let k = 10
let iters = 15
fn gen(n) = {
    let mut xs = []
    let mut ys = []
    let mut seed = 42
    let mut i = 0
    while i < n {
        seed = seed * 48271 % 2147483647
        xs = xs + [10.0 * to_float(seed) / 2147483647.0]
        seed = seed * 48271 % 2147483647
        ys = ys + [10.0 * to_float(seed) / 2147483647.0]
        i = i + 1
    }
    #{ "xs": xs, "ys": ys }
}
fn cluster(xs, ys, k, iters) = {
    let n = len(xs)
    let stride = n / k
    let mut cx = map(0..k, (c) => xs[c * stride])
    let mut cy = map(0..k, (c) => ys[c * stride])
    let mut assign = map(0..n, (i) => 0)
    let mut it = 0
    while it < iters {
        for i in 0..n {
            let mut best = 0
            let mut bd = 1000000.0
            for c in 0..k {
                let dx = xs[i] - cx[c]
                let dy = ys[i] - cy[c]
                let t1 = dx * dx
                let t2 = dy * dy
                let d = t1 + t2
                if d < bd => { bd = d; best = c }
            }
            assign = col.set(assign, i, best)
        }
        let mut sx = map(0..k, (c) => 0.0)
        let mut sy = map(0..k, (c) => 0.0)
        let mut ct = map(0..k, (c) => 0)
        for i in 0..n {
            let c = assign[i]
            sx = col.set(sx, c, sx[c] + xs[i])
            sy = col.set(sy, c, sy[c] + ys[i])
            ct = col.set(ct, c, ct[c] + 1)
        }
        for c in 0..k {
            if ct[c] > 0 => {
                cx = col.set(cx, c, sx[c] / to_float(ct[c]))
                cy = col.set(cy, c, sy[c] / to_float(ct[c]))
            }
        }
        it = it + 1
    }
    let mut sizes = map(0..k, (c) => 0)
    for i in 0..n { sizes = col.set(sizes, assign[i], sizes[assign[i]] + 1) }
    let mut largest = 0
    for c in 0..k { if sizes[c] > largest => { largest = sizes[c] } }
    largest
}
let pts = gen(n)
let t0 = time.monotonic_ms()
let largest = cluster(map_get(pts, "xs"), map_get(pts, "ys"), k, iters)
println(`CHECK ${largest}`)
println(`MS ${time.monotonic_ms() - t0}`)

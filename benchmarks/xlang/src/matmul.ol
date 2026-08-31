let n = 300
fn build(n, mode) =
    map(0..n, (i) => map(0..n, (j) =>
        to_float((if mode == 0 => i * j else => i + j) % 100) * 0.01))
fn mul(a, b, n) = {
    let mut c = []
    for i in 0..n {
        let mut row = map(0..n, (j) => 0.0)
        for j in 0..n {
            let mut s = 0.0
            for k in 0..n { s = s + a[i][k] * b[k][j] }
            row = col.set(row, j, s)
        }
        c = c + [row]
    }
    c
}
fn total(c, n) = {
    let mut t = 0.0
    for i in 0..n { for j in 0..n { t = t + c[i][j] } }
    t
}
let a = build(n, 0)
let b = build(n, 1)
let t0 = time.monotonic_ms()
let c = mul(a, b, n)
let t = total(c, n)
println(`CHECK ${to_int(math.round(t))}`)
println(`MS ${time.monotonic_ms() - t0}`)

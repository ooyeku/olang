fn steps(start) = {
    let mut n = start
    let mut c = 0
    while n != 1 {
        if n % 2 == 0 => { n = n / 2 } else => { n = 3 * n + 1 }
        c = c + 1
    }
    c
}
let t0 = time.monotonic_ms()
let mut total = 0
for i in 1..300001 { total = total + steps(i) }
println(`CHECK ${total}`)
println(`MS ${time.monotonic_ms() - t0}`)

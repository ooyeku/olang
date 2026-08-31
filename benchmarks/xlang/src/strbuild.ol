let n = 2000000
let t0 = time.monotonic_ms()
let mut s = ""
for i in 0..n { s = s + to_string(i % 1000) }
println(`CHECK ${str.length(s)}`)
println(`MS ${time.monotonic_ms() - t0}`)

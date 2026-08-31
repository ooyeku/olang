fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)
let t0 = time.monotonic_ms()
let r = fib(32)
let ms = time.monotonic_ms() - t0
println(`CHECK ${r}`)
println(`MS ${ms}`)

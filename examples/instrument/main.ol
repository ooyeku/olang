// A real workload wearing the instrument library's macros.
//
// The program computes Collatz stopping times and a memoized Fibonacci;
// what the macros add — the cache, the trace lines, the timers — is
// ordinary generated code, visible with `olang expand main.ol`.

use instrument

// Memoized: recursion goes through the public name, so the cache makes
// this linear. fib(80) would not return before the heat death of the
// universe without it.
@memo
fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)

// Traced: every call and return is printed.
@trace
fn collatz_step(n) = if n % 2 == 0 => n / 2 else => 3 * n + 1

fn collatz_len(start) = {
    let mut n = start
    let mut steps = 0
    while n != 1 {
        n = if n % 2 == 0 => n / 2 else => 3 * n + 1
        steps = steps + 1
    }
    steps
}

println("═══ instrument: macros on a real workload ═══")

let big = @timed("fib(80) memoized", fib(80))
println(`fib(80) = ${big}`)
println(`cache holds ${fib_cache_size()} entries`)

println("── one traced step chain ──")
let s1 = collatz_step(6)
let s2 = collatz_step(s1)
println(`6 → ${s1} → ${s2}`)

let len27 = @dbg(collatz_len(27))
println(`collatz(27) takes ${len27} steps`)

test "memoization is real, measured by the cache not the clock" {
    assert_eq(fib(10), 55)
    // fib(80) above + fib(10) here: every value 0..=80 visited once.
    assert_eq(fib_cache_size(), 81)
    assert_eq(fib(80), 23416728348467685)
}

test "tracing preserves the value" {
    assert_eq(collatz_step(6), 3)
    assert_eq(collatz_len(27), 111)
}

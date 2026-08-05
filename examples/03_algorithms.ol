// ═══════════════════════════════════════════════════════════════════
// 03 — Algorithms
// Classic algorithms in olang: recursion, higher-order functions,
// closures as strategies, and functions returning functions.
// ═══════════════════════════════════════════════════════════════════

println("═══ algorithms ═══")

// ── Recursion: factorial and Fibonacci ──────────────────────────────
fn factorial(n) = if n <= 1 => 1 else => n * factorial(n - 1)
fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)

println(`factorial(10) = ${factorial(10)}`)
println(`fib(20) = ${fib(20)}`)

// ── Memoization via a closure over a map ────────────────────────────
// A stateful counter proves each input is computed once.
fn make_memo_fib() = {
    let cache = #{}
    fn go(n) = {
        if n < 2 => n
        else => {
            let key = to_string(n)
            if map_has_key(cache, key) => map_get(cache, key)
            else => {
                let result = go(n - 1) + go(n - 2)
                cache = map_set(cache, key, result)
                result
            }
        }
    }
    go
}
let mfib = make_memo_fib()
println(`memoized fib(30) = ${mfib(30)}`)

// ── Quicksort: recursion + partition via filter ─────────────────────
fn quicksort(xs) = {
    if len(xs) <= 1 => xs
    else => {
        let pivot = head(xs)
        let rest = tail(xs)
        let smaller = rest |> filter((x) => x < pivot)
        let larger = rest |> filter((x) => x >= pivot)
        concat(concat(quicksort(smaller), [pivot]), quicksort(larger))
    }
}
println(`quicksort: ${quicksort([5, 2, 8, 1, 9, 3, 7, 4, 6])}`)

// ── Binary search over a sorted list ────────────────────────────────
fn binary_search(xs, target) = {
    fn go(lo, hi) = {
        if lo > hi => -1
        else => {
            let mid = (lo + hi) / 2
            let v = xs[mid]
            match true {
                _ if v == target => mid,
                _ if v < target => go(mid + 1, hi),
                _ => go(lo, mid - 1)
            }
        }
    }
    go(0, len(xs) - 1)
}
let sorted = [1, 3, 5, 7, 9, 11, 13, 15]
println(`binary_search(9)  = index ${binary_search(sorted, 9)}`)
println(`binary_search(10) = index ${binary_search(sorted, 10)}`)

// ── Sieve of Eratosthenes: primes up to N ───────────────────────────
fn primes_up_to(n) = {
    fn is_prime(k) = {
        fn check(d) = {
            if d * d > k => true
            else => if k % d == 0 => false else => check(d + 1)
        }
        if k < 2 => false else => check(2)
    }
    range(2, n + 1) |> filter(is_prime)
}
println(`primes < 30: ${primes_up_to(29)}`)

// ── Higher-order: compose and pipeline of transforms ────────────────
fn compose(f, g) = (x) => f(g(x))
let inc = (x) => x + 1
let dbl = (x) => x * 2
let inc_then_dbl = compose(dbl, inc)
println(`compose(dbl, inc)(10) = ${inc_then_dbl(10)}`)      // (10+1)*2 = 22

// ── Reduce as a universal tool ──────────────────────────────────────
fn product(xs) = xs |> fold(1, (a, b) => a * b)
fn maximum(xs) = xs |> fold(head(xs), (a, b) => if b > a => b else => a)
fn count_where(xs, pred) = xs |> fold(0, (acc, x) => if pred(x) => acc + 1 else => acc)

println(`product([1..5]) = ${product([1, 2, 3, 4, 5])}`)
println(`maximum = ${maximum([3, 9, 2, 8, 5])}`)
println(`evens in 1..20 = ${count_where(range(1, 21), (x) => x % 2 == 0)}`)

// ── Strategy pattern: pick a comparator at runtime ──────────────────
fn sort_by(xs, better) = {
    // insertion sort driven by a caller-supplied comparator closure
    fn insert(sorted, x) = {
        if len(sorted) == 0 => [x]
        else => if better(x, head(sorted)) => concat([x], sorted)
                else => concat([head(sorted)], insert(tail(sorted), x))
    }
    xs |> fold([], (acc, x) => insert(acc, x))
}
let people = [
    { name: "Ann", age: 30 },
    { name: "Bob", age: 25 },
    { name: "Cy",  age: 41 }
]
let youngest_first = sort_by(people, (a, b) => a.age < b.age) |> map((p) => p.name)
let oldest_first = sort_by(people, (a, b) => a.age > b.age) |> map((p) => p.name)
println(`by age asc:  ${youngest_first}`)
println(`by age desc: ${oldest_first}`)

// ── Greatest common divisor (Euclid) ────────────────────────────────
fn gcd(a, b) = if b == 0 => a else => gcd(b, a % b)
println(`gcd(48, 36) = ${gcd(48, 36)}`)

println("═══ algorithms complete ═══")

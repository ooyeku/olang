// prelude — the demo's core toolkit: pure functions only, no I/O.
//
// Everything here is deliberately dependency-free so every other module
// can import it. It carries the language's foundational idioms: closures
// as strategies, structural recursion, an Option sum type, and the
// cache-threading pattern that replaces in-place memoization (closures
// capture by value, so a captured map cannot be mutated — thread it).

// ── Option: the classic optional sum type ───────────────────────────
share type Option = enum { Some(Int), None }

share fn opt_map(o, f) = match o {
    Some(v) => Some(f(v)),
    None    => None
}

share fn opt_or(o, default) = match o {
    Some(v) => v,
    None    => default
}

share fn opt_filter(o, pred) = match o {
    Some(v) => if pred(v) => Some(v) else => None,
    None    => None
}

// Safe search that returns an Option instead of crashing on absence.
share fn find_first(list, pred) = fold(reverse(list), None, (acc, x) =>
    if pred(x) => Some(x) else => acc)

// ── Higher-order tools ──────────────────────────────────────────────
share fn compose(f, g) = (x) => f(g(x))

share fn count_where(xs, pred) = xs |> fold(0, (acc, x) => if pred(x) => acc + 1 else => acc)

share fn product(xs) = xs |> fold(1, (a, b) => a * b)

share fn maximum(xs) = xs |> fold(head(xs), (a, b) => if b > a => b else => a)

// ── Comparator-driven sorting: the strategy pattern ─────────────────
// Insertion sort steered by a caller-supplied `better` closure. Used by
// the berth scheduler to order the waiting queue by whatever policy the
// harbormaster configures.
share fn sort_with(xs, better) = {
    fn insert(sorted, x) = {
        if len(sorted) == 0 => [x]
        else => if better(x, head(sorted)) => concat([x], sorted)
                else => concat([head(sorted)], insert(tail(sorted), x))
    }
    xs |> fold([], (acc, x) => insert(acc, x))
}

// Quicksort for plain numeric lists (recursion + partition via filter).
share fn quicksort(xs) = {
    if len(xs) <= 1 => xs
    else => {
        let pivot = head(xs)
        let rest = tail(xs)
        let smaller = rest |> filter((x) => x < pivot)
        let larger = rest |> filter((x) => x >= pivot)
        concat(concat(quicksort(smaller), [pivot]), quicksort(larger))
    }
}

// Binary search over a sorted list; -1 when absent.
share fn binary_search(xs, target) = {
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

// ── Numeric helpers ─────────────────────────────────────────────────
share fn clamp(x, lo, hi) = if x < lo => lo else => if x > hi => hi else => x

share fn gcd(a, b) = if b == 0 => a else => gcd(b, a % b)

// Round to one decimal for display.
share fn round1(x: Float) = to_float(to_int(x * 10.0)) / 10.0

// Percent share, one decimal.
share fn pct(part: Float, whole: Float) = if whole == 0.0 => 0.0 else => round1(part / whole * 100.0)

// Money formatting: two decimals, always — negative-safe, via str.fixed:
// the sign goes before the currency mark, and an amount that rounds to
// zero carries no sign.
share fn money(x: Float) = {
    let text = str.fixed(if x < 0.0 => 0.0 - x else => x, 2)
    if x < 0.0 && text != "0.00" => "-$" + text else => "$" + text
}

// ── The cache-threading idiom ───────────────────────────────────────
// olang closures capture by value: `cache = map_set(...)` inside a
// closure writes to a snapshot and is provably dead (`olang check`
// warns). The working idiom is functional — pass the cache in, return
// [value, cache'] out. The tide model in lib.schedule leans on this.
share fn memo_fib(n, cache) = {
    let key = to_string(n)
    if map_has_key(cache, key) => [map_get(cache, key), cache]
    else => if n < 2 => [n, cache]
    else => {
        let a = memo_fib(n - 1, cache)
        let b = memo_fib(n - 2, a[1])
        let result = a[0] + b[0]
        [result, map_set(b[1], key, result)]
    }
}

// ── Self-checks ─────────────────────────────────────────────────────
test "option combinators" {
    let first_big = find_first([3, 8, 15, 22], (n) => n > 10)
    testing.assert_eq(opt_or(first_big, -1), 15)
    testing.assert_eq(opt_or(find_first([1], (n) => n > 9), -1), -1)
    testing.assert_eq(opt_or(opt_map(Some(4), (x) => x * 2), 0), 8)
    testing.assert_eq(opt_or(opt_filter(Some(7), (x) => x > 100), -1), -1)
}

test "sorting and search" {
    testing.assert_eq(quicksort([5, 2, 8, 1, 9]), [1, 2, 5, 8, 9])
    let by_desc = sort_with([3, 1, 2], (a, b) => a > b)
    testing.assert_eq(by_desc, [3, 2, 1])
    testing.assert_eq(binary_search([1, 3, 5, 7, 9], 7), 3)
    testing.assert_eq(binary_search([1, 3, 5, 7, 9], 4), -1)
}

test "numeric helpers" {
    testing.assert_eq(clamp(12, 0, 10), 10)
    testing.assert_eq(gcd(48, 36), 12)
    testing.assert_eq(money(1234.5), "$1234.50")
    testing.assert_eq(money(-3.075), "-$3.08")
    testing.assert_eq(money(-0.001), "$0.00")
    testing.assert_eq(pct(25.0, 200.0), 12.5)
    testing.assert_eq(memo_fib(30, #{})[0], 832040)
}

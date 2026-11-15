// compat: functions — declarations, defaults, named arguments, lambdas,
// closures by value, early return, recursion, nested functions.

fn add(a, b) = a + b
fn greet(name, greeting = "Hello") = `${greeting}, ${name}`
fn box(w, h = 1, d = 1) = w * h * d
println(add(2, 3))
println(greet("Ada"))
println(greet("Ada", "Hi"))
println(greet("Ada", greeting: "Hey"))
println(box(2, d: 5))

fn typed(n: Int, s: String) -> String = `${s}${n}`
println(typed(3, "n="))
println(is_err(attempt(() => typed("x", "n="))))

let double = (v) => v * 2
println(double(21))
println([1, 2, 3] |> map((v) => v + 1))

// a closure captures the local bindings it names by value; a top-level
// name is never captured
fn snapshot() = {
    let mut local = 1
    let snap = () => local
    local = 100
    snap()
}
println(snapshot())
let mut counter = 1
let reads = () => counter
counter = 100
println(reads())

fn make_adder(k) = (v) => v + k
let add10 = make_adder(10)
println(add10(5))

fn first_negative(xs) = {
    for v in xs {
        if v < 0 => return v
    }
    ()
}
println(first_negative([3, -2, -5]))
println(first_negative([1]))

fn fact(n) = if n <= 1 => 1 else => n * fact(n - 1)
println(fact(20))

fn outer(v) = {
    fn inner(w) = w * 3
    inner(v) + 1
}
println(outer(4))

// a top-level name is read when the function runs
let base = 1
fn uses_base() = base
let base = 2
println(uses_base())

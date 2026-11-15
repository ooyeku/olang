// compat: the core language — literals, bindings, operators and their
// precedence, strings, ranges, control flow, break with a value, pipelines.

// literals and how values print
println(42)
println(-7)
println(1_000_000)
println(0xff)
println(3.14)
println(1e20)
println(2.5e-3)
println(true)
println(())
println("text")
println([1, 2, 3])
println((1, "a"))
println(#{ "k": 1, "j": [2] })
println({ name: "olang", year: 2026 })
println(show("quoted"))

// bindings and the mutability model
let x = 10
let mut n = 0
n = n + x
let x = x * 2          // shadowing
println(`${x} ${n}`)

// operators and precedence
println(1 + 2 * 3)
println((1 + 2) * 3)
println(2 - 3 - 4)
println(17 / 5)
println(17 % 5)
println(17.0 / 4.0)
println(-2 * 3 + 1)
println(1 < 2 && 2 < 3 || false)
println(!true || true)
println(5 & 3)
println(5 | 3)
println(5 ^ 3)
println(1 << 4)
println(() ?? "default")
println(0 ?? "default")
println([1, 2] + [3])
println("ab" + "cd")
println([5] == [5.0])
println(1 == 1 && "a" != "b")

// strings
let s = "Hello, World"
println(len(s))
println(`interpolated ${1 + 1} and ${s}`)
println("line\ttab\\slash \"quote\"")
println(str.to_upper(s) + " " + str.to_lower(s))

// ranges
println(range(0, 5))
for i in 0..3 { print(i) print(" ") }
println("")

// control flow
fn sign(v) = if v > 0 => "pos" else if v == 0 => "zero" else => "neg"
println(`${sign(3)} ${sign(0)} ${sign(-3)}`)
let block = {
    let a = 2
    let b = 3
    a * b
}
println(block)
let mut i = 0
let mut total = 0
while i < 5 {
    i = i + 1
    if i == 2 => continue
    total = total + i
}
println(total)
let found = loop {
    i = i + 1
    if i % 7 == 0 => break i
}
println(found)
for (k, v) in enumerate(["a", "b"]) { println(`${k}=${v}`) }

// pipelines
let squares = [1, 2, 3, 4, 5]
    |> filter((v) => v % 2 == 1)
    |> map((v) => v * v)
println(squares)
println(fold([1, 2, 3, 4], 0, (acc, v) => acc + v))
println([3, 1, 2] |> sort)

// ═══════════════════════════════════════════════════════════════════
// 01 — Language Tour
// Every core construct in one guided pass. Start here.
// ═══════════════════════════════════════════════════════════════════

println("═══ olang language tour ═══")

// ── Literals ────────────────────────────────────────────────────────
let integer = 42
let float = 3.14159
let boolean = true
let binary = 0b1010          // 10
let octal = 0o755            // 493
let hex = 0xFF               // 255
let character = 'g'
let text = "quoted \"strings\" with escapes"
let raw = r"raw strings keep \n literally"
let name = "world"
let template = `hello ${name}, 6 x 7 = ${6 * 7}`

println(template)
println(`numeric bases: ${binary}, ${octal}, ${hex}`)

// ── Collections ─────────────────────────────────────────────────────
let list = [1, 2, 3, 4, 5]
let tuple = ("pair", 2)
let scores = #{"ann": 91, "bob": 84}

// ── Destructuring ───────────────────────────────────────────────────
let (label, arity) = tuple
let [first, ...rest] = list
println(`tuple -> ${label}/${arity}, list -> head ${first}, tail ${rest}`)

// ── Functions: named, defaults, lambdas, closures ──────────────────
fn square(n) = n * n
fn greet(who: String = "there") = "hi " + who
let add = (a, b) => a + b
let make_adder = (n) => (x) => x + n     // closure over n
let add10 = make_adder(10)
let pi = () => 3.14159                   // zero-parameter lambda

println(greet())
println(`square(7)=${square(7)}, add(2,3)=${add(2, 3)}, add10(5)=${add10(5)}, pi()=${pi()}`)

// ── Control flow: if is an expression ───────────────────────────────
let n = 17
let parity = if n % 2 == 0 => "even" else => "odd"
println(`${n} is ${parity}`)

// ── Loops: for, while, loop, break, continue ────────────────────────
let evens_below_20 = {
    let acc = []
    for i in 0..20 {
        if i % 2 == 1 => continue
        acc = concat(acc, [i])
    }
    acc
}
println(`evens: ${evens_below_20}`)

let countdown = {
    let x = 5
    let steps = []
    loop {
        if x == 0 => break
        steps = concat(steps, [x])
        x = x - 1
    }
    steps
}
println(`countdown: ${countdown}`)

// ── Pattern matching: literals, ranges, guards, or-patterns ─────────
fn describe(x) = match x {
    0 => "zero",
    1 | 2 | 3 => "small",
    4..=10 => "medium",
    v if v < 0 => "negative",
    _ => "large"
}
println(`describe: ${describe(0)}, ${describe(2)}, ${describe(7)}, ${describe(-4)}, ${describe(99)}`)

// ── Destructuring patterns: Results, lists, tuples ──────────────────
fn shape_of(value) = match value {
    Ok([a, b]) => `ok pair ${a},${b}`,
    Ok(v) => `ok ${v}`,
    Err(e) => `err ${e}`,
    _ => "not a result"
}
println(shape_of(Ok([1, 2])) + " | " + shape_of(Ok(9)) + " | " + shape_of(Err("boom")))

// ── Pipelines: the idiomatic core ───────────────────────────────────
let report = range(1, 11)
    |> map((x) => x * x)
    |> filter((x) => x % 2 == 1)
    |> fold(0, (acc, x) => acc + x)
println(`sum of odd squares 1..10: ${report}`)

fn add3(a, b) = a + b
println(`partial pipe: ${5 |> add3(3)}`)      // piped value fills the first slot

// ── Structs and tagged-union dispatch ───────────────────────────────
// Structs carry named fields. `enum` types declare but a discriminated
// union is expressed at runtime as a tagged struct + match on the tag.
type Point = struct { x: Int, y: Int }

let here = Point { x: 3, y: 4 }
fn step(p, direction) = match direction {
    "north" => Point { x: p.x, y: p.y + 1 },
    "south" => Point { x: p.x, y: p.y - 1 },
    "east" => Point { x: p.x + 1, y: p.y },
    "west" => Point { x: p.x - 1, y: p.y },
    _ => p
}
let there = step(step(here, "north"), "east")
println(`walked from (${here.x},${here.y}) to (${there.x},${there.y})`)

// ── Error handling: Result, ?, try/catch ────────────────────────────
fn safe_div(a, b) = if b == 0 => Err("division by zero") else => Ok(a / b)

fn ratio_sum(pairs) = {
    let total = 0
    for pair in pairs {
        let (a, b) = pair
        total = total + match safe_div(a, b) {
            Ok(v) => v,
            Err(e) => 0
        }
    }
    total
}
println(`ratio sum: ${ratio_sum([(10, 2), (9, 3), (1, 0)])}`)

let recovered = try { safe_div(1, 0) } catch (e) { -1 }
println(`try/catch recovered: ${recovered}`)

// ── Maps ────────────────────────────────────────────────────────────
let tally = #{}
let words = ["red", "blue", "red", "green", "red"]
let counted = words |> fold(#{}, (acc, w) =>
    map_set(acc, w, if map_has_key(acc, w) => map_get(acc, w) + 1 else => 1))
println(`tally: red=${map_get(counted, "red")}, keys=${sort(map_keys(counted))}`)

println("═══ tour complete ═══")

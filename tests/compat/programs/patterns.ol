// compat: pattern matching — every documented pattern kind.

type Dir = enum { North, South }
type Tree = enum { Leaf(Int), Node(Tree, Tree) }
type Point = struct { x: Int, y: Int }

fn describe(n) = match n {
    0 => "zero",
    1 | 2 | 3 => "a few",
    4..10 => "several",
    x if x < 0 => "negative",
    _ => "many"
}
println([0, 2, 7, -4, 99] |> map(describe))

fn letter(c) = match c { 'a'..'n' => "first half", _ => "second half" }
println(`${letter("c")} ${letter("q")}`)

fn kind(v) = match v {
    "yes" => "literal string",
    true => "literal bool",
    5 => "the Int 5",
    _ => "something else"
}
println([kind("yes"), kind(true), kind(5), kind(5.0)])

fn pair(t) = match t { (a, b) if a == b => "same", (a, b) => `${a}/${b}` }
println(`${pair((1, 1))} ${pair((1, 2))} ${pair([3, 4])}`)

fn sum(xs) = match xs { [] => 0, [h, ...t] => h + sum(t) }
println(sum([1, 2, 3, 4]))
fn shape(xs) = match xs { [a, b] => `two: ${a} ${b}`, [_, ...rest] => `head and ${len(rest)} more`, [] => "empty" }
println([shape([1, 2]), shape([1, 2, 3]), shape([])])

fn total(t) = match t { Leaf(n) => n, Node(l, r) => total(l) + total(r) }
println(total(Node(Node(Leaf(1), Leaf(2)), Leaf(3))))

fn flip(d) = match d { North => South, South => North }
println(flip(North) == South)

fn res(r) = match r { Ok(v) => `ok ${v}`, Err(e) => `err ${e}` }
println(`${res(Ok(1))} ${res(Err("no"))}`)

fn at(p) = match p { Point { x: 0, y } => `on the y axis at ${y}`, Point { x, y } => `${x},${y}` }
println(`${at(Point { x: 0, y: 5 })} ${at(Point { x: 2, y: 3 })}`)

fn area(s) = match s {
    { kind: "circle", r: r } => 3.0 * r * r,
    { kind: "rect", w: w, h: h } => w * h,
    {} => -1.0,
    _ => 0.0
}
println([area({ kind: "rect", w: 3.0, h: 4.0 }), area({ kind: "circle", r: 1.0 }), area({ other: 1 }), area(7)])

// destructuring lets
let (a, b) = (1, 2)
let [first, ...others] = [10, 20, 30]
println(`${a} ${b} ${first} ${others}`)

// a value no arm matches is a runtime error, caught at a boundary
println(is_err(attempt(() => match 3 { 1 => "one" })))

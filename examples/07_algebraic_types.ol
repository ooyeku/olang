// ═══════════════════════════════════════════════════════════════════
// 07 — Algebraic Data Types
// Enums as real sum types: a recursive expression evaluator, a generic
// Option library, and a binary tree. The canonical ADT demonstrations —
// none of this was expressible before enums constructed at runtime.
// ═══════════════════════════════════════════════════════════════════

println("═══ algebraic data types ═══")

// ── A recursive expression tree ─────────────────────────────────────
// Expr is a sum type; payload variants nest to build a tree.
type Expr = enum {
    Num(Float),
    Add(Expr, Expr),
    Sub(Expr, Expr),
    Mul(Expr, Expr),
    Neg(Expr)
}

// Evaluate the tree by structural recursion over the variants.
fn eval(e) = match e {
    Num(n)    => n,
    Add(a, b) => eval(a) + eval(b),
    Sub(a, b) => eval(a) - eval(b),
    Mul(a, b) => eval(a) * eval(b),
    Neg(x)    => 0.0 - eval(x)
}

// Pretty-print the tree the same way.
fn show(e) = match e {
    Num(n)    => to_string(n),
    Add(a, b) => "(" + show(a) + " + " + show(b) + ")",
    Sub(a, b) => "(" + show(a) + " - " + show(b) + ")",
    Mul(a, b) => "(" + show(a) + " * " + show(b) + ")",
    Neg(x)    => "-" + show(x)
}

// Build:  (2 + 3) * -(4 - 1)   =  5 * -3  = -15
let expr = Mul(
    Add(Num(2.0), Num(3.0)),
    Neg(Sub(Num(4.0), Num(1.0)))
)
println("── expression evaluator ──")
println(`  ${show(expr)} = ${eval(expr)}`)

// A second expression, folded from a list of numbers into a sum tree
fn sum_tree(nums) = fold(tail(nums), Num(head(nums)), (acc, n) => Add(acc, Num(n)))
let s = sum_tree([1.0, 2.0, 3.0, 4.0])
println(`  ${show(s)} = ${eval(s)}`)

// ── A generic Option library ────────────────────────────────────────
// Some(x)/None over any payload — the classic optional type, with the
// combinators you'd expect. (Runtime generics: works for any value.)
type Option = enum { Some(Int), None }

fn opt_map(o, f) = match o {
    Some(v) => Some(f(v)),
    None    => None
}
fn opt_or(o, default) = match o {
    Some(v) => v,
    None    => default
}
fn opt_filter(o, pred) = match o {
    Some(v) => if pred(v) => Some(v) else => None,
    None    => None
}

// Safe lookup that returns an Option instead of crashing.
fn find_first(list, pred) = fold(reverse(list), None, (acc, x) =>
    if pred(x) => Some(x) else => acc)

println("── option combinators ──")
let nums = [3, 8, 15, 22, 9]
let first_big = find_first(nums, (n) => n > 10)
println(`  first > 10: ${opt_or(first_big, 0 - 1)}`)
println(`  none case:  ${opt_or(find_first(nums, (n) => n > 99), 0 - 1)}`)

let doubled = opt_map(first_big, (x) => x * 2)
println(`  mapped:     ${opt_or(doubled, 0)}`)

let filtered = opt_filter(Some(7), (x) => x > 100)
println(`  filtered:   ${opt_or(filtered, 0 - 1)}`)

// ── A binary search tree (insert + in-order traversal) ──────────────
type Tree = enum { Leaf, Node(Tree, Int, Tree) }

fn insert(tree, x) = match tree {
    Leaf => Node(Leaf, x, Leaf),
    Node(left, v, right) =>
        if x < v => Node(insert(left, x), v, right)
        else => if x > v => Node(left, v, insert(right, x))
                else => tree
}

fn in_order(tree) = match tree {
    Leaf => [],
    Node(left, v, right) => concat(concat(in_order(left), [v]), in_order(right))
}

println("── binary search tree ──")
let tree = fold([5, 3, 8, 1, 4, 7, 9, 2], Leaf, (t, x) => insert(t, x))
println(`  sorted via in-order traversal: ${in_order(tree)}`)

println("═══ ADT demo complete ═══")

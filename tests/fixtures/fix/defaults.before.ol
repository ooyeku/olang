// `olang fix defaults`: each rewrite, then each form left (and why).

// if x == () => d else => x  →  x ?? d (and `!=` turned round)
fn pad(o) = if o.pad == () => 0 else => o.pad
fn name(u) = if u != () => u else => "guest"
fn total(o) = 1 + if o.n == () => 0 else => o.n
fn neg(o) = -if o.n == () => 0 else => o.n
fn same(a, b) = if a == () => b == 2 else => a
fn chain(a, b) = if a != () => a else if b != () => b else => 0
fn nested(a, b) = if a == () => (if b == () => 1 else => b) else => a
fn arg(o) = len(if o.xs == () => [] else => o.xs)
fn later(o) = if o.k == () => raise("no k") else => o.k
fn inner(o) = if o.k == () => 0 else if o.j == () => 1 else => o.k
fn text(o) = `pad ${if o.pad == () => 0 else => o.pad}`
fn piped(xs) = xs |> (v) => if v == () => 0 else => v

// left: a block, a jump, ()
fn blocky(o) = if o.n == () => { 5 } else => o.n
fn early(o) = {
    let n = if o.n == () => return 0 else => o.n
    n + 1
}
fn unit(o) = if o.n == () => () else => o.n
// left: a comment would be lost
fn noted(o) = if o.n == () => 0 // none yet
  else => o.n
// not a default: the else branch is not what was tested
fn other(o) = if o.n == () => 0 else => o.m

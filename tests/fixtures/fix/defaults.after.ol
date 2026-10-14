// `olang fix defaults`: each rewrite, then each form left (and why).

// if x == () => d else => x  →  x ?? d (and `!=` turned round)
fn pad(o) = o.pad ?? 0
fn name(u) = u ?? "guest"
fn total(o) = 1 + (o.n ?? 0)
fn neg(o) = -(o.n ?? 0)
fn same(a, b) = a ?? (b == 2)
fn chain(a, b) = a ?? (b ?? 0)
fn nested(a, b) = a ?? (b ?? 1)
fn arg(o) = len(o.xs ?? [])
fn later(o) = o.k ?? raise("no k")
fn inner(o) = if o.k == () => 0 else if o.j == () => 1 else => o.k
fn text(o) = `pad ${o.pad ?? 0}`
fn piped(xs) = xs |> (v) => v ?? 0

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

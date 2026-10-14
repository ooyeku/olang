// `olang fix first-last`: each rewrite, then each form left (and why).

fn tail(s) = {
    let parts = str.split(s, "/")
    if len(parts) == 0 => () else => parts[len(parts) - 1]
}
fn head(xs: [Int]) = if len(xs) > 0 => xs[0] else => ()
fn head2(xs: [Int]) = if len(xs) > 0 => xs[0]
fn top(xs: [Int]) = if 0 < len(xs) => xs[0] else => ()
fn ends(xs: [Int]) = len(xs) > 0 && xs[len(xs) - 1] == 2
fn both(xs: [Int]) = if len(xs) == 0 || xs[0] == 9 => 0 else => xs[0] + xs[len(xs) - 1]
fn name(s: String) = if len(s) == 0 => "" else => s[0] + s[len(s) - 1]
fn grown(xs: [Int]) = {
    let ys = xs + [1]
    if len(ys) > 0 => ys[0].x else => ()
}
fn mixed(xs: [Int], c) = if len(xs) == 0 || c => () else => xs[len(xs) - 1]
fn more(s) = {
    let mut seen = []
    seen = seen + [s]
    if len(seen) == 0 => () else => seen[len(seen) - 1]
}

// left: not proven non-empty — `xs[len(xs) - 1]` raises on [], `last` answers ()
fn unguarded(xs: [Int]) = xs[len(xs) - 1]
// left: not proven a list, tuple or string — a Bytes value would take
// len and [i] but not last
fn any(xs) = if len(xs) == 0 => () else => xs[len(xs) - 1]
fn field(o) = if len(o.xs) > 0 => o.xs[0] else => ()
fn assigned(xs: [Int], b) = {
    let mut ys = [1]
    ys = b
    if len(ys) > 0 => ys[0] else => ()
}
// not guarded: a lambda is called later, the name is bound again in the
// branch
fn later(xs: [Int]) = if len(xs) > 0 => () => xs[0] else => () => 0
fn rebound(xs: [Int]) = if len(xs) > 0 => { let xs = []; xs } else => ()

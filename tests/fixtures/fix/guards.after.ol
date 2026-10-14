// `olang fix guards`: each rewrite, then each form left (and why).

// if x == () => () else => x.f…  →  x.f…
fn cursor(s) = s.pp.cur
fn depth(t) = t.node.depth
fn label(n) = n.label
fn title(w) = w.doc.title
fn same(m) = m
fn pick(a, b) = if a == () => b else => b.next

// x != () && x.f == v  →  x.f == v (v a literal that is not ())
fn running(t) = t.state == "running"
fn wraps(p) = p.wrapped == true
fn deep(m) = m.trun.state == "done"
fn present(h) = h.pty != ()
fn busy(a, h) = a > 0 && h.n == 1

// x == () || x.f != v  →  x.f != v
fn idle(t) = t.state != "running"
fn bare(s) = s.items == ()

fn branch(t) = if t.kind == "dir" => "d" else => "f"   // in a condition

// left: the read is used as a Bool, and `&&` needs one
fn on(o) = o != () && o.enabled
// left: compared with a value that may itself be ()
fn match_(o, v) = o != () && o.k == v
// left: () answers these differently
fn fresh(s) = s != () && s.ended == ()
fn big(s) = s != () && s.n > 3
// left: a call or an index on the guarded path
fn call(r) = if r == () => () else => r.f(1)
fn index(r) = if r == () => () else => r.xs[0]
// left: a comment would be lost
fn noted(r) = if r == () => () // none yet
  else => r.f
// not a guard of this read
fn other(a, b) = if a == () => () else => b.f
fn split_(a) = a.x != () && a.y == 1

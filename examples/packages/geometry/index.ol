// geometry — a small shared library.
// `share` marks the public API; anything unshared stays private to the package.

// ── 2D points ───────────────────────────────────────────────────────
share fn point(x, y) = { x: x, y: y }

share fn distance(a, b) = {
    let dx = a.x - b.x
    let dy = a.y - b.y
    sqrt_approx(dx * dx + dy * dy)
}

// ── Shape areas (tagged structs, dispatched on a "kind" field) ──────
share fn circle(r) = { kind: "circle", r: r }
share fn rectangle(w, h) = { kind: "rectangle", w: w, h: h }
share fn triangle(base, height) = { kind: "triangle", base: base, height: height }

share fn area(shape) = match shape {
    { kind: "circle", r: r } => 3.14159265 * r * r,
    { kind: "rectangle", w: w, h: h } => w * h,
    { kind: "triangle", base: b, height: h } => b * h / 2.0,
    _ => 0.0
}

share fn describe(shape) = shape.kind + " with area " + to_string(area(shape))

// ── A private helper (not shared) — Newton's method for sqrt ────────
fn sqrt_approx(n) = {
    let mut guess = n / 2.0 + 1.0
    let mut i = 0
    while i < 20 {
        guess = (guess + n / guess) / 2.0
        i = i + 1
    }
    guess
}

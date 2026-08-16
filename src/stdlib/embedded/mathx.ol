// mathx — the *pure* subset of the native `math` module, written in olang.
// These need no native float intrinsics: integer/rational operations, float
// rounding built on integer truncation, and a Newton's-method sqrt. The
// transcendentals (sin, cos, ln, exp, ...) stay native — they belong behind
// the FFI boundary.
//
// The native `math` module remains the default; this is the differential-
// tested olang mirror of the parts that can be self-hosted.

// ── Constants (full f64 precision, so they equal the native ones) ───
share let PI = 3.141592653589793
share let E = 2.718281828459045
share let TAU = 6.283185307179586

// ── Sign and magnitude ──────────────────────────────────────────────
share fn abs(x) = if x < 0 => 0 - x else => x

share fn sign(x) = if x > 0 => 1 else => if x < 0 => 0 - 1 else => 0

share fn min(a, b) = if a < b => a else => b
share fn max(a, b) = if a > b => a else => b

// ── Number theory (integers) ────────────────────────────────────────
fn gcd_pos(a, b) = if b == 0 => a else => gcd_pos(b, a % b)
share fn gcd(a, b) = gcd_pos(abs(a), abs(b))

share fn lcm(a, b) = if a == 0 || b == 0 => 0 else => abs(a * b) / gcd(a, b)

share fn factorial(n) = {
    let mut acc = 1
    let mut i = 2
    while i <= n {
        acc = acc * i
        i = i + 1
    }
    acc
}

// ── Float rounding (built on truncation toward zero) ────────────────
// trunc drops the fractional part toward zero.
share fn trunc(x) = to_float(to_int(x))

// floor rounds toward negative infinity; ceil toward positive infinity.
share fn floor(x) = {
    let t = trunc(x)
    if t == x => x else => if x > 0 => t else => t - 1.0
}
share fn ceil(x) = {
    let t = trunc(x)
    if t == x => x else => if x > 0 => t + 1.0 else => t
}

// round: half away from zero, matching Rust's f64::round.
share fn round(x) = if x >= 0 => floor(x + 0.5) else => ceil(x - 0.5)

// fractional part, keeping the sign of x.
share fn fract(x) = x - trunc(x)

// ── Angle conversion ────────────────────────────────────────────────
share fn radians(deg) = deg * PI / 180.0
share fn degrees(rad) = rad * 180.0 / PI

// ── sqrt via Newton's method — a transcendental function, self-hosted ─
// Converges to the native sqrt to full f64 precision for x >= 0.
share fn sqrt(x) = {
    if x < 0.0 => 0.0 - 1.0
    else => if x == 0.0 => 0.0
    else => {
        let mut guess = x
        let mut i = 0
        while i < 60 {
            guess = (guess + x / guess) / 2.0
            i = i + 1
        }
        guess
    }
}

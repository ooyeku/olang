// mathx — the *pure* subset of the native `math` module, written in olang.
// These need no native float intrinsics: integer/rational operations, float
// rounding built on integer truncation, and a Newton's-method sqrt. The
// transcendentals (sin, cos, ln, exp, ...) stay native — they belong behind
// the FFI boundary.
//
// The native `math` module remains the default; this is the differential-
// tested olang mirror of the parts that can be self-hosted.

// ── Constants (full f64 precision, so they equal the native ones) ───
/// @stable
share let PI = 3.141592653589793
/// @stable
share let E = 2.718281828459045
/// @stable
share let TAU = 6.283185307179586

// ── Sign and magnitude ──────────────────────────────────────────────
/// @stable
share fn abs(x) = if x < 0 => 0 - x else => x

/// @stable
share fn sign(x) = if x > 0 => 1 else => if x < 0 => 0 - 1 else => 0

// the first of equal values wins, as math.min and math.max keep it
/// @stable
share fn min(a, b) = if b < a => b else => a
/// @stable
share fn max(a, b) = if b > a => b else => a

// ── Number theory (integers) ────────────────────────────────────────
fn gcd_pos(a, b) = if b == 0 => a else => gcd_pos(b, a % b)
/// @stable
share fn gcd(a, b) = gcd_pos(abs(a), abs(b))

/// @stable
share fn lcm(a, b) = if a == 0 || b == 0 => 0 else => abs(a * b) / gcd(a, b)

/// @stable
share fn factorial(n) = {
    let mut acc = 1
    let mut i = 2
    while i <= n {
        acc = acc * i
        i = i + 1
    }
    acc
}

// ── Rounding (built on truncation toward zero) ──────────────────────
// Each answers an Int, as math's do; to_int raises outside the Int range.
// trunc drops the fractional part toward zero.
/// @stable
share fn trunc(x) = to_int(x)

// floor rounds toward negative infinity; ceil toward positive infinity.
/// @stable
share fn floor(x) = {
    let t = trunc(x)
    if t == x => t else => if x > 0 => t else => t - 1
}
/// @stable
share fn ceil(x) = {
    let t = trunc(x)
    if t == x => t else => if x > 0 => t + 1 else => t
}

// round: half away from zero, matching Rust's f64::round.
/// @stable
share fn round(x) = if x >= 0 => floor(x + 0.5) else => ceil(x - 0.5)

// fractional part, keeping the sign of x.
/// @stable
share fn fract(x) = to_float(x) - to_float(trunc(x))

// ── Angle conversion ────────────────────────────────────────────────
/// @stable
share fn radians(deg) = deg * PI / 180.0
/// @stable
share fn degrees(rad) = rad * 180.0 / PI

// ── sqrt via Newton's method — a transcendental function, self-hosted ─
// Converges to the native sqrt to full f64 precision for x >= 0.
/// @stable
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

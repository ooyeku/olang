//! The baseline JIT: native code must be unobservable except in speed.
//!
//! Every test here runs a program three ways — interpreted, on the
//! bytecode tier (which now JITs qualifying functions) — and demands
//! byte-identical results. The interesting cases are the guard edges:
//! overflow, division by zero, i64::MIN, non-integer arguments, depth
//! exhaustion — where the native code must deopt and let bytecode (or
//! the interpreter) produce the canonical answer.

use olang::ast::Value;
use olang::{Interpreter, Parser};

// 512MB: debug-build interpreter frames are enormous, and the recursion
// tests interpret ~1000 frames deep before their guards trip.
fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn eval(source: &str, tier_threshold: Option<u32>) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        if let Some(threshold) = tier_threshold {
            interpreter.enable_bytecode_tier(threshold, false);
        }
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

/// Tiered (JIT-active) and interpreted runs must agree exactly.
fn assert_jit_transparent(source: &str) {
    let interpreted = eval(source, None);
    let tiered = eval(source, Some(1));
    assert_eq!(tiered, interpreted, "tier+jit diverged from interpreter");
}

#[test]
fn fib_is_correct_under_the_jit() {
    assert_jit_transparent("fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)\nfib(24)");
}

#[test]
fn loop_kernels_are_correct_under_the_jit() {
    assert_jit_transparent(
        r#"
fn kernel(n) = {
    let mut acc = 0
    let mut i = 0
    while i < 1000 { acc = acc + (n * i) % 7; i = i + 1 }
    acc
}
kernel(3) + kernel(11)
"#,
    );
}

#[test]
fn bool_returning_functions_are_correct_under_the_jit() {
    assert_jit_transparent(
        r#"
fn is_even(n) = n % 2 == 0
fn both_even(a, b) = is_even(a) && is_even(b)
show(is_even(7)) + " " + show(both_even(4, 8))
"#,
    );
}

#[test]
fn overflow_deopts_to_the_canonical_error() {
    // 2^62 * 4 overflows i64: the native guard must deopt and the
    // canonical overflow error must surface, identical to interpretation.
    let src = r#"
fn quad(n) = n * 4
quad(4611686018427387904)
"#;
    let interpreted = eval(src, None);
    let tiered = eval(src, Some(1));
    assert!(interpreted.is_err(), "interpreter should overflow");
    assert_eq!(tiered, interpreted);
}

#[test]
fn addition_overflow_matches() {
    let src = r#"
fn bump(n) = n + 1
bump(9223372036854775807)
"#;
    assert_eq!(eval(src, Some(1)), eval(src, None));
}

#[test]
fn division_by_zero_matches() {
    let src = r#"
fn ratio(a, b) = a / b
ratio(10, 0)
"#;
    assert_eq!(eval(src, Some(1)), eval(src, None));
}

#[test]
fn modulo_by_zero_matches() {
    let src = r#"
fn wrap(a, b) = a % b
wrap(10, 0)
"#;
    assert_eq!(eval(src, Some(1)), eval(src, None));
}

#[test]
fn i64_min_division_edge_matches() {
    // i64::MIN / -1 overflows; the guard must deopt.
    let src = r#"
fn div(a, b) = a / b
div(-9223372036854775808, -1)
"#;
    assert_eq!(eval(src, Some(1)), eval(src, None));
}

#[test]
fn negation_of_i64_min_matches() {
    let src = r#"
fn flip(n) = -n
flip(-9223372036854775808)
"#;
    assert_eq!(eval(src, Some(1)), eval(src, None));
}

#[test]
fn float_arguments_take_the_bytecode_path() {
    // The entry guard requires all-Integer arguments; floats must give
    // exactly the bytecode/interpreter behavior.
    assert_jit_transparent(
        r#"
fn add_one(x) = x + 1
show(add_one(5)) + " " + show(add_one(2.5))
"#,
    );
}

#[test]
fn deep_recursion_still_errors_cleanly() {
    // Depth budget exhaustion deopts; the re-run hits the canonical
    // recursion-guard error rather than overflowing the native stack.
    // (3000 exceeds both the native budget and the interpreter's guard;
    // release-binary probes confirmed both modes error identically.)
    let src = r#"
fn down(n) = if n == 0 => 0 else => down(n - 1)
down(3000)
"#;
    let interpreted = eval(src, None);
    let tiered = eval(src, Some(1));
    assert_eq!(tiered.is_err(), interpreted.is_err());
    assert_eq!(tiered, interpreted);
}

#[test]
fn recursion_within_budget_is_correct() {
    assert_jit_transparent("fn down(n) = if n == 0 => 0 else => down(n - 1)\ndown(900)");
}

#[test]
fn mutual_recursion_stays_on_bytecode_and_agrees() {
    // Mutual recursion has non-self CallFn instructions: the JIT declines,
    // bytecode runs it, results agree.
    assert_jit_transparent(
        r#"
fn is_even(n) = if n == 0 => true else => is_odd(n - 1)
fn is_odd(n) = if n == 0 => false else => is_even(n - 1)
show(is_even(100)) + " " + show(is_odd(77))
"#,
    );
}

#[test]
fn jitted_functions_compose_with_higher_order_builtins() {
    assert_jit_transparent(
        r#"
fn square(n) = n * n
sum(map(1..100, square))
"#,
    );
}

#[test]
fn jitted_functions_work_inside_par_map() {
    assert_jit_transparent(
        r#"
fn collatz_len(n0) = {
    let mut n = n0
    let mut steps = 0
    while n != 1 {
        if n % 2 == 0 => { n = n / 2 } else => { n = 3 * n + 1 }
        steps = steps + 1
    }
    steps
}
sum(par_map(1..500, collatz_len))
"#,
    );
}

#[test]
fn comparison_chains_and_logic_are_correct() {
    assert_jit_transparent(
        r#"
fn classify(n) = {
    let small = n < 10
    let even = n % 2 == 0
    if small && even => 1 else => if small || even => 2 else => 3
}
show(classify(4)) + show(classify(7)) + show(classify(12)) + show(classify(13))
"#,
    );
}

#[test]
fn negative_operands_and_neg_are_correct() {
    assert_jit_transparent(
        r#"
fn f(a, b) = -a * b + (a % b) - (a / b)
show(f(-17, 5)) + " " + show(f(17, -5)) + " " + show(f(-17, -5))
"#,
    );
}

// ── float specialization ───────────────────────────────────────────────

#[test]
fn float_kernels_are_correct_under_the_jit() {
    assert_jit_transparent(
        r#"
fn orbit(cr, ci) = {
    let mut zr = 0.0
    let mut zi = 0.0
    let mut i = 0
    while i < 50 {
        let zr2 = zr * zr - zi * zi + cr
        let zi2 = 2.0 * zr * zi + ci
        zr = zr2
        zi = zi2
        if zr * zr + zi * zi > 4.0 => { break }
        i = i + 1
    }
    i
}
show(orbit(0.1, 0.1)) + " " + show(orbit(0.8, 0.3)) + " " + show(orbit(-1.0, 0.2))
"#,
    );
}

#[test]
fn mixed_int_float_arithmetic_matches() {
    assert_jit_transparent(
        r#"
fn blend(a, b) = a * 2 + b / 4.0 - 1
show(blend(3.5, 10.0)) + " " + show(blend(2.25, 9.0))
"#,
    );
}

#[test]
fn float_recursion_with_mixed_params_matches() {
    assert_jit_transparent(
        "fn fpow(base, n) = if n == 0 => 1.0 else => base * fpow(base, n - 1)\nfpow(1.5, 20)",
    );
}

#[test]
fn float_division_by_zero_matches() {
    let src = r#"
fn ratio(a, b) = a / b
ratio(1.5, 0.0)
"#;
    assert_eq!(eval(src, Some(1)), eval(src, None));
}

#[test]
fn float_overflow_to_infinity_matches() {
    // IEEE overflow is not an error in olang: both tiers produce inf.
    assert_jit_transparent(
        r#"
fn big(x) = x + x
show(big(1.0e308))
"#,
    );
}

#[test]
fn nan_comparisons_match() {
    assert_jit_transparent(
        r#"
fn cmp(a, b) = {
    let lt = a < b
    let ge = a >= b
    let eq = a == b
    let ne = a != b
    show(lt) + show(ge) + show(eq) + show(ne)
}
let inf = 1.0e308 * 10.0
let nan = inf * 0.0
cmp(nan, 1.0) + " " + cmp(nan, nan) + " " + cmp(1.0, 2.0)
"#,
    );
}

#[test]
fn float_compares_against_int_operands_match() {
    assert_jit_transparent(
        r#"
fn check(x) = if x > 2 => "big" else => "small"
check(2.5) + " " + check(1.5)
"#,
    );
}

#[test]
fn polymorphic_call_sites_stay_correct() {
    // First call specializes on Int; the later Float call must deopt to
    // bytecode and still agree exactly.
    assert_jit_transparent(
        r#"
fn twice(x) = x + x
show(twice(21)) + " " + show(twice(0.75)) + " " + show(twice(3))
"#,
    );
}

#[test]
fn float_first_polymorphism_stays_correct() {
    assert_jit_transparent(
        r#"
fn half(x) = x / 2.0
show(half(5.0)) + " " + show(half(7))
"#,
    );
}

#[test]
fn float_modulo_is_refused_but_correct() {
    // fmod has no exact IR equivalent, so the JIT declines the whole
    // function; bytecode runs it and results agree.
    assert_jit_transparent(
        r#"
fn wrap(a, b) = a % b
show(wrap(7.5, 2.0)) + " " + show(wrap(-7.5, 2.0))
"#,
    );
}

#[test]
fn negative_zero_and_fneg_match() {
    assert_jit_transparent(
        r#"
fn flip(x) = -x
show(flip(2.5)) + " " + show(flip(-2.5)) + " " + show(flip(0.0) == 0.0)
"#,
    );
}

// ── cross-function native calls ────────────────────────────────────────

#[test]
fn helper_calls_compile_and_agree() {
    assert_jit_transparent(
        r#"
fn scale(x) = x * 3
fn shift(x) = x + 7
fn pipeline(x) = scale(shift(x)) - shift(scale(x))
show(pipeline(5)) + " " + show(pipeline(-11))
"#,
    );
}

#[test]
fn mutual_recursion_compiles_and_agrees() {
    assert_jit_transparent(
        r#"
fn is_even(n) = if n == 0 => true else => is_odd(n - 1)
fn is_odd(n) = if n == 0 => false else => is_even(n - 1)
show(is_even(600)) + " " + show(is_odd(431))
"#,
    );
}

#[test]
fn split_fib_across_two_functions_agrees() {
    assert_jit_transparent(
        r#"
fn fib_a(n) = if n < 2 => n else => fib_b(n - 1) + fib_b(n - 2)
fn fib_b(n) = if n < 2 => n else => fib_a(n - 1) + fib_a(n - 2)
fib_a(22)
"#,
    );
}

#[test]
fn call_chains_of_mixed_kinds_agree() {
    assert_jit_transparent(
        r#"
fn to_ratio(n) = n / 4.0
fn add_half(x) = x + 0.5
fn score(n) = add_half(to_ratio(n)) * 2.0
show(score(10)) + " " + show(score(7))
"#,
    );
}

#[test]
fn deopt_deep_in_a_native_chain_matches() {
    // The overflow happens two native calls deep; the whole chain must
    // unwind and re-run on bytecode with the canonical error.
    let src = r#"
fn inner(n) = n * 3
fn middle(n) = inner(n) + 1
fn outer(n) = middle(n)
outer(4611686018427387904)
"#;
    assert_eq!(eval(src, Some(1)), eval(src, None));
}

#[test]
fn kind_mismatched_helper_refuses_but_agrees() {
    // half() gets specialized for Float by its direct call; the later
    // int-calling group must decline and stay correct on bytecode.
    assert_jit_transparent(
        r#"
fn half(x) = x / 2.0
fn use_float() = half(9.0)
fn use_int(n) = half(n)
show(use_float()) + " " + show(use_int(9))
"#,
    );
}

#[test]
fn deep_mutual_recursion_depth_guard_matches() {
    // Mutual recursion past the depth budget: native deopts, bytecode
    // re-runs, the recursion guard error matches interpretation.
    let src = r#"
fn ping(n) = if n == 0 => 0 else => pong(n - 1)
fn pong(n) = if n == 0 => 1 else => ping(n - 1)
ping(3000)
"#;
    let interpreted = eval(src, None);
    let tiered = eval(src, Some(1));
    assert_eq!(tiered.is_err(), interpreted.is_err());
    assert_eq!(tiered, interpreted);
}

#[test]
fn three_function_cycle_agrees() {
    assert_jit_transparent(
        r#"
fn red(n) = if n <= 0 => 0 else => green(n - 1) + 1
fn green(n) = if n <= 0 => 0 else => blue(n - 1) + 2
fn blue(n) = if n <= 0 => 0 else => red(n - 1) + 3
show(red(30)) + " " + show(green(31)) + " " + show(blue(32))
"#,
    );
}

// ── struct field access ────────────────────────────────────────────────

#[test]
fn struct_field_kernels_agree() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn dist2(a, b) = {
    let dx = b.x - a.x
    let dy = b.y - a.y
    dx * dx + dy * dy
}
show(dist2(P { x: 1.0, y: 2.0 }, P { x: 4.0, y: 6.0 }))
"#,
    );
}

#[test]
fn mixed_field_kinds_and_int_fields_agree() {
    assert_jit_transparent(
        r#"
type Row = struct { id: Int, w: Float, live: Bool }
fn score(r) = if r.live => r.w * 2.0 + to_float(r.id) else => 0.0
show(score(Row { id: 7, w: 1.5, live: true })) + " " + show(score(Row { id: 1, w: 9.0, live: false }))
"#,
    );
}

#[test]
fn same_shape_different_field_kind_deopts_and_agrees() {
    // Specialized on a float x; the later int-x instance must deopt at
    // the guarded read and agree exactly.
    assert_jit_transparent(
        r#"
type Box = struct { v: Float }
fn get(b) = b.v
show(get(Box { v: 2.5 }) + get(Box { v: 1.5 }))
"#,
    );
}

#[test]
fn structs_passed_through_call_chains_agree() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn dx(a, b) = b.x - a.x
fn dy(a, b) = b.y - a.y
fn taxi(a, b) = dx(a, b) + dy(a, b)
show(taxi(P { x: 1.0, y: 1.0 }, P { x: 4.0, y: 9.0 }))
"#,
    );
}

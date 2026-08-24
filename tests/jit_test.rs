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

#[test]
fn math_builtins_agree_bit_exactly() {
    assert_jit_transparent(
        r#"
fn kernel(x) = math.sqrt(x * x + 1.0) + math.sin(x) * math.cos(x) + math.pow(x, 2.0)
fn probe(x) = math.floor(x) + math.ceil(x) + math.trunc(x) + math.atan2(x, 2.0)
show(kernel(1.7)) + " " + show(kernel(-3.2)) + " " + show(probe(2.6)) + " " + show(math.sqrt(-1.0))
"#,
    );
}

#[test]
fn math_on_int_args_promotes_like_the_vm() {
    assert_jit_transparent(
        r#"
fn f(n) = math.sqrt(n * n)
show(f(12)) + " " + show(f(-5))
"#,
    );
}

// ── heap values: list indexing and for-loop iteration ──────────────────

#[test]
fn list_indexing_kernels_agree() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn sumdist(pts, n) = {
    let mut acc = 0.0
    let mut i = 0
    while i < n {
        let a = pts[i]
        let b = pts[n - 1 - i]
        let dx = b.x - a.x
        acc = acc + math.sqrt(dx * dx)
        i = i + 1
    }
    acc
}
let pts = [P { x: 1.0, y: 2.0 }, P { x: 4.0, y: 6.0 }, P { x: 9.0, y: 1.0 }]
show(sumdist(pts, 3))
"#,
    );
}

#[test]
fn negative_and_out_of_range_indices_agree() {
    // Subscripts wrap negatives (VM semantics); the JIT helper must too.
    assert_jit_transparent(
        r#"
fn last_plus_first(xs) = xs[-1] + xs[0]
show(last_plus_first([10, 20, 30]))
"#,
    );
}

#[test]
fn for_loops_over_lists_agree() {
    assert_jit_transparent(
        r#"
type B = struct { m: Float, v: Float }
fn energy(bodies) = {
    let mut e = 0.0
    for b in bodies { e = e + 0.5 * b.m * b.v * b.v }
    e
}
show(energy([B { m: 1.0, v: 2.0 }, B { m: 3.0, v: 0.5 }]))
"#,
    );
}

#[test]
fn float_and_int_lists_agree() {
    assert_jit_transparent(
        r#"
fn total(xs) = {
    let mut t = 0.0
    for x in xs { t = t + x }
    t
}
fn itotal(xs) = {
    let mut t = 0
    for x in xs { t = t + x }
    t
}
show(total([1.5, 2.5, 3.0])) + " " + show(itotal([1, 2, 3, 4]))
"#,
    );
}

#[test]
fn mixed_list_stays_on_bytecode_and_agrees() {
    // A list holding both kinds refuses classification; results agree.
    assert_jit_transparent(
        r#"
fn first(xs) = xs[0]
show(first([1, 2.5]))
"#,
    );
}

// ── tuple extraction ───────────────────────────────────────────────────

#[test]
fn tuple_returning_functions_agree() {
    assert_jit_transparent(
        r#"
fn force(a, b) = {
    let dx = b - a
    (dx, dx * 2.0, dx * 3.0)
}
fn use_force(a, b) = {
    let (fx, fy, fz) = force(a, b)
    fx + fy + fz
}
show(use_force(1.0, 3.0)) + " " + show(use_force(-2.5, 2.5))
"#,
    );
}

#[test]
fn mixed_kind_tuples_agree() {
    assert_jit_transparent(
        r#"
fn divmod(a, b) = (a / b, a % b, a > b)
fn f(a, b) = {
    let (q, r, big) = divmod(a, b)
    if big => q * 100 + r else => r
}
show(f(47, 10)) + " " + show(f(3, 10))
"#,
    );
}

#[test]
fn tuple_get_by_index_agrees() {
    assert_jit_transparent(
        r#"
fn pair(x) = (x + 1.0, x * 2.0)
fn f(x) = pair(x).0 + pair(x).1
show(f(4.0))
"#,
    );
}

#[test]
fn deopt_inside_tuple_producer_agrees() {
    // Division by zero deep in the tuple producer must yield the VM's
    // canonical error through deopt re-execution.
    assert_jit_transparent(
        r#"
fn danger(a, b) = (a / b, a * b)
fn f(a, b) = {
    let (q, p) = danger(a, b)
    q + p
}
let ok = f(10, 2)
let bad = try f(1, 0) catch e => -1
show(ok) + " " + show(bad)
"#,
    );
}

// ── struct construction ────────────────────────────────────────────────

#[test]
fn native_constructors_agree() {
    assert_jit_transparent(
        r#"
type V = struct { x: Float, y: Float }
fn add(a, b) = V { x: a.x + b.x, y: a.y + b.y }
fn scale(v, k) = V { x: v.x * k, y: v.y * k }
fn step(v) = scale(add(v, V { x: 0.1, y: 0.2 }), 0.999)
fn orbit(v, steps) = {
    let mut cur = v
    let mut i = 0
    while i < steps { cur = step(cur); i = i + 1 }
    cur.x + cur.y
}
show(orbit(V { x: 1.0, y: 2.0 }, 50))
"#,
    );
}

#[test]
fn mixed_field_constructors_agree() {
    assert_jit_transparent(
        r#"
type Row = struct { id: Int, w: Float, live: Bool }
fn make(id, w) = Row { id: id, w: w * 2.0, live: id > 0 }
fn probe(id, w) = {
    let r = make(id, w)
    if r.live => r.w + to_float(r.id) else => 0.0 - r.w
}
show(probe(3, 1.5)) + " " + show(probe(-1, 4.0))
"#,
    );
}

#[test]
fn returning_a_parameter_struct_agrees() {
    // Returning an entry argument exercises the args side of retain.
    assert_jit_transparent(
        r#"
type P = struct { x: Float }
fn pick(a, b, flip) = if flip => a else => b
show(pick(P { x: 1.5 }, P { x: 2.5 }, true).x + pick(P { x: 1.5 }, P { x: 2.5 }, false).x)
"#,
    );
}

#[test]
fn allocating_loops_stay_on_bytecode_and_agree() {
    // The refusal rule: loops that build structs are driven from
    // bytecode; results must be identical either way.
    assert_jit_transparent(
        r#"
type A = struct { v: Float }
fn build_sum(n) = {
    let mut acc = 0.0
    let mut i = 0
    while i < n { acc = acc + A { v: to_float(i) * 0.5 }.v; i = i + 1 }
    acc
}
show(build_sum(200))
"#,
    );
}

// ── strings ────────────────────────────────────────────────────────────

#[test]
fn string_kernels_agree() {
    assert_jit_transparent(
        r#"
type User = struct { name: String, score: Int }
fn label(u) = u.name + ": " + (if u.score > 90 => "gold" else => "std")
fn pick(a, b) = if a.name < b.name => a else => b
let u1 = User { name: "ada", score: 95 }
let u2 = User { name: "bob", score: 80 }
label(u1) + " | " + label(u2) + " | " + pick(u1, u2).name
"#,
    );
}

#[test]
fn string_comparisons_agree() {
    assert_jit_transparent(
        r#"
fn rel(a, b) = show(a == b) + show(a != b) + show(a < b) + show(a <= b) + show(a > b) + show(a >= b)
rel("abc", "abd") + " " + rel("z", "z") + " " + rel("", "a") + " " + rel("ab", "a")
"#,
    );
}

#[test]
fn string_returns_and_concat_chains_agree() {
    assert_jit_transparent(
        r#"
fn greet(name) = "hello, " + name + "!"
fn twice(s) = greet(s) + " " + greet(s)
twice("world")
"#,
    );
}

#[test]
fn concat_in_loops_stays_on_bytecode_and_agrees() {
    assert_jit_transparent(
        r#"
fn join_n(s, n) = {
    let mut acc = ""
    let mut i = 0
    while i < n { acc = acc + s; i = i + 1 }
    acc
}
join_n("ab", 50)
"#,
    );
}

// ── Results: construction, tests, extraction, returns ──────────────────
//
// The JIT's ninth kind. Results ride borrowed Arc<ResultObject> pointers
// like structs; payload reads are guarded per side, construction is
// scratch-owned, and mixed Ok/Err return paths join side-by-side into
// one Result kind. Everything unprovable deopts to bytecode — these
// tests demand the seam is invisible.

#[test]
fn result_construction_and_match_agree() {
    assert_jit_transparent(
        r#"
fn sign(n) = if n >= 0 => Ok(n) else => Err(0 - n)
fn amount(r) = match r {
    Ok(v) => v,
    Err(e) => e * 100
}
amount(sign(5)) + amount(sign(0 - 3)) + amount(sign(42))
"#,
    );
}

#[test]
fn result_params_extract_on_both_sides() {
    assert_jit_transparent(
        r#"
fn take(r) = match r {
    Ok(v) => v * 2,
    Err(e) => e - 1
}
take(Ok(10)) + take(Err(4)) + take(Ok(7))
"#,
    );
}

#[test]
fn result_float_payloads_agree() {
    assert_jit_transparent(
        r#"
fn halve(x) = if x > 0.0 => Ok(x / 2.0) else => Err(x * x)
fn get(r) = match r {
    Ok(v) => v,
    Err(e) => e + 0.5
}
get(halve(3.0)) + get(halve(0.0 - 2.0))
"#,
    );
}

#[test]
fn result_string_payloads_extract_and_compare() {
    // String payload extraction is a borrowed read; returning the
    // extracted string deopts (unknown pointer at the retain boundary)
    // and bytecode must produce the same answer.
    assert_jit_transparent(
        r#"
fn msg(r) = match r {
    Ok(v) => v,
    Err(e) => e
}
fn is_boom(r) = match r {
    Ok(v) => false,
    Err(e) => e == "boom"
}
msg(Err("boom")) + show(is_boom(Err("boom"))) + show(is_boom(Err("quiet")))
"#,
    );
}

#[test]
fn result_string_construction_stays_on_bytecode_and_agrees() {
    // v1 constructs scalar payloads only; Err("...") refuses the JIT and
    // must fall back cleanly.
    assert_jit_transparent(
        r#"
fn safe_div(a, b) = if b == 0 => Err("div by zero") else => Ok(a / b)
fn run(a, b) = match safe_div(a, b) {
    Ok(v) => v,
    Err(e) => 0 - 1
}
run(10, 2) + run(7, 0) + run(9, 3)
"#,
    );
}

#[test]
fn result_returns_cross_function_boundaries() {
    assert_jit_transparent(
        r#"
fn classify(n) = if n % 2 == 0 => Ok(n / 2) else => Err(n * 3 + 1)
fn step(n) = match classify(n) {
    Ok(v) => v,
    Err(e) => e
}
step(6) + step(7) + step(20)
"#,
    );
}

#[test]
fn result_annotated_returns_discharge_statically() {
    assert_jit_transparent(
        r#"
fn double(n) -> Result<Int, String> = Ok(n * 2)
fn get(r) = match r {
    Ok(v) => v,
    Err(e) => 0
}
get(double(4)) + get(double(9))
"#,
    );
}

#[test]
fn result_specialization_deopts_on_the_other_side() {
    // First calls specialize on Ok(Int); later Err(Str) arguments must
    // classify differently and take the bytecode path with the same
    // answers.
    assert_jit_transparent(
        r#"
fn unwrap_or_neg(r) = match r {
    Ok(v) => v,
    Err(e) => 0 - 1
}
let mut acc = 0
let mut i = 0
while i < 20 { acc = acc + unwrap_or_neg(Ok(i)); i = i + 1 }
acc + unwrap_or_neg(Err("late")) + unwrap_or_neg(Ok(100))
"#,
    );
}

#[test]
fn result_bool_payloads_agree() {
    assert_jit_transparent(
        r#"
fn flag(n) = if n > 0 => Ok(n % 2 == 0) else => Err(n == 0 - 1)
fn read(r) = match r {
    Ok(v) => if v => 1 else => 2,
    Err(e) => if e => 3 else => 4
}
read(flag(4)) + read(flag(3)) + read(flag(0 - 1)) + read(flag(0 - 5))
"#,
    );
}

#[test]
fn result_construction_in_loops_stays_on_bytecode_and_agrees() {
    assert_jit_transparent(
        r#"
fn tally(n) = {
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + match (if i % 3 == 0 => Ok(i) else => Err(1)) {
            Ok(v) => v,
            Err(e) => e
        }
        i = i + 1
    }
    acc
}
tally(30)
"#,
    );
}

#[test]
fn result_try_operator_agrees_under_the_jit() {
    assert_jit_transparent(
        r#"
fn half(n) = if n % 2 == 0 => Ok(n / 2) else => Err(n)
fn quarter(n) = {
    let a = half(n)?
    let b = half(a)?
    Ok(b)
}
fn read(r) = match r {
    Ok(v) => v,
    Err(e) => 0 - e
}
read(quarter(8)) + read(quarter(6)) + read(quarter(5))
"#,
    );
}

// ── Lists: construction, concat, returns ───────────────────────────────
//
// MakeList and list + list compile in straight-line code (scratch-owned,
// same allocation discipline as structs); list-returning constructors
// compile and hand ownership back through the retain boundary. Loops
// that build lists stay on bytecode and drive native constructors.

#[test]
fn list_literals_construct_and_index_natively() {
    assert_jit_transparent(
        r#"
fn pair(a, b) = [a, b]
fn first_of(a, b) = pair(a, b)[0]
first_of(3, 4) + first_of(10, 20)
"#,
    );
}

#[test]
fn list_returns_cross_the_entry_boundary() {
    assert_jit_transparent(
        r#"
fn triple(n) = [n, n * 2, n * 3]
let xs = triple(5)
xs[0] + xs[1] + xs[2]
"#,
    );
}

#[test]
fn float_list_construction_agrees() {
    assert_jit_transparent(
        r#"
fn origin() = [0.0, 0.0]
fn scaled(x) = [x * 2.0, x * 4.0]
let a = origin()
let b = scaled(1.5)
a[0] + b[0] + b[1]
"#,
    );
}

#[test]
fn list_concat_agrees_in_straight_line_code() {
    assert_jit_transparent(
        r#"
fn glue(a, b, c) = [a, b] + [c]
let xs = glue(1, 2, 3)
xs[0] + xs[1] + xs[2]
"#,
    );
}

#[test]
fn list_param_concat_agrees() {
    assert_jit_transparent(
        r#"
fn extend(xs, v) = xs + [v]
let ys = extend([1, 2], 3)
ys[0] + ys[1] + ys[2]
"#,
    );
}

#[test]
fn returning_a_list_parameter_agrees() {
    assert_jit_transparent(
        r#"
fn choose(xs, ys, flag) = if flag => xs else => ys
let picked = choose([1, 2], [3, 4], false)
picked[0] + picked[1]
"#,
    );
}

#[test]
fn mixed_element_lists_stay_on_bytecode_and_agree() {
    assert_jit_transparent(
        r#"
fn mixed(n) = [n, 1.5]
fn tagged(s) = [s, s]
let a = mixed(2)
let b = tagged("x")
show(a[1]) + b[0]
"#,
    );
}

#[test]
fn list_building_loops_stay_on_bytecode_and_agree() {
    assert_jit_transparent(
        r#"
fn upto(n) = {
    let mut acc = [0]
    let mut i = 1
    while i < n { acc = acc + [i]; i = i + 1 }
    acc
}
let xs = upto(10)
xs[3] + xs[9]
"#,
    );
}

#[test]
fn empty_list_literals_stay_on_bytecode_and_agree() {
    assert_jit_transparent(
        r#"
fn base() = []
fn grown(v) = base() + [v]
grown(7)[0]
"#,
    );
}

#[test]
fn annotated_list_returns_discharge_statically() {
    assert_jit_transparent(
        r#"
fn doubles(n) -> List = [n * 2, n * 4]
let xs = doubles(3)
xs[0] + xs[1]
"#,
    );
}

#[test]
fn constructed_lists_iterate_in_loops_via_deopt() {
    assert_jit_transparent(
        r#"
fn nums() = [2, 4, 6]
fn total() = {
    let mut acc = 0
    for x in nums() { acc = acc + x }
    acc
}
total()
"#,
    );
}

// ── struct-element lists ───────────────────────────────────────────────
//
// MakeList with uniform struct elements compiles as ListStruct: each
// borrowed element pointer is resolved back to an owned Arc (a scratch
// allocation or an entry argument) inside the helper, so the list owns
// its elements exactly like the VM's. Mixed shapes refuse to bytecode.

#[test]
fn struct_element_lists_construct_and_read_back() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn segment(a, b) = [P { x: a, y: 0.0 }, P { x: b, y: 1.0 }]
fn spread(a, b) = {
    let pts = segment(a, b)
    pts[1].x - pts[0].x + pts[1].y
}
show(spread(2.0, 7.0))
"#,
    );
}

#[test]
fn struct_params_collect_into_lists() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn wrap(a, b) = [a, b]
fn head_x(a, b) = wrap(a, b)[0].x
show(head_x(P { x: 3.5, y: 0.0 }, P { x: 9.0, y: 1.0 }))
"#,
    );
}

#[test]
fn struct_list_concat_agrees() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn join(a, b, c) = [P { x: a, y: 0.0 }] + [P { x: b, y: 0.0 }, P { x: c, y: 0.0 }]
fn total(a, b, c) = {
    let pts = join(a, b, c)
    pts[0].x + pts[1].x + pts[2].x
}
show(total(1.0, 2.0, 3.0))
"#,
    );
}

#[test]
fn struct_lists_iterate_across_function_boundaries() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn corners(w, h) = [P { x: 0.0, y: 0.0 }, P { x: w, y: 0.0 }, P { x: w, y: h }]
fn perimeter_x(w, h) = {
    let mut acc = 0.0
    for p in corners(w, h) { acc = acc + p.x }
    acc
}
show(perimeter_x(4.0, 3.0))
"#,
    );
}

#[test]
fn mixed_shape_lists_stay_on_bytecode_and_agree() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
type Q = struct { v: Int }
fn odd(a) = [P { x: a, y: 0.0 }, Q { v: 1 }]
fn read(a) = odd(a)[1].v
read(2.0)
"#,
    );
}

#[test]
fn struct_and_scalar_mix_stays_on_bytecode_and_agrees() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn odd(a) = [P { x: a, y: 0.0 }, 5]
fn read(a) = odd(a)[1]
read(2.0)
"#,
    );
}

#[test]
fn callee_built_lists_index_inside_native_callers() {
    // The caller indexes and field-reads a list built by its callee —
    // the element kind resolves a fixpoint iteration late, which must
    // defer, not refuse.
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn segment(a, b) = [P { x: a, y: 0.0 }, P { x: b, y: 1.0 }]
fn spread(a, b) = {
    let pts = segment(a, b)
    pts[1].x - pts[0].x + pts[1].y
}
show(spread(2.0, 7.0))
"#,
    );
}

#[test]
fn callee_built_structs_field_read_inside_native_callers() {
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn make(a) = P { x: a, y: a * 2.0 }
fn use_it(a) = {
    let p = make(a)
    p.x + p.y
}
show(use_it(3.0))
"#,
    );
}

// ── string-element lists ───────────────────────────────────────────────
//
// ListStr, the fourth list kind. Element reads hand out borrowed
// pointers like struct elements; construction resolves scratch/arg
// pointers to owned Arcs and content-clones anything else (baked
// constants, list elements) — strings are immutable values with content
// equality, so identity is unobservable.

#[test]
fn string_lists_construct_and_index() {
    assert_jit_transparent(
        r#"
fn tags(extra) = ["alpha", "beta", extra]
fn pick(extra, i) = tags(extra)[i]
pick("gamma", 0) + pick("gamma", 2)
"#,
    );
}

#[test]
fn string_list_params_read_and_compare() {
    assert_jit_transparent(
        r#"
fn nth_is(xs, i, want) = xs[i] == want
fn label(xs, i) = if nth_is(xs, i, "hot") => 1 else => 0
label(["cold", "hot"], 1) + label(["cold", "hot"], 0)
"#,
    );
}

#[test]
fn string_lists_concat_and_return() {
    assert_jit_transparent(
        r#"
fn greetings(name) = ["hi " + name] + ["bye " + name]
let gs = greetings("ada")
gs[0] + " / " + gs[1]
"#,
    );
}

#[test]
fn string_lists_iterate_across_function_boundaries() {
    assert_jit_transparent(
        r#"
fn words() = ["a", "bb", "ccc"]
fn joined() = {
    let mut acc = ""
    for w in words() { acc = acc + w }
    acc
}
joined()
"#,
    );
}

#[test]
fn string_and_scalar_mixes_stay_on_bytecode_and_agree() {
    assert_jit_transparent(
        r#"
fn odd(n) = ["x", n]
fn read(n) = show(odd(n)[1])
read(5)
"#,
    );
}

// ── maps ───────────────────────────────────────────────────────────────
//
// Kind::Map(payload): string-keyed maps with a uniform value payload.
// map_get is a key-guarded read (a miss — the VM returns Unit — deopts),
// map_has_key is total, map_set is clone-and-insert into scratch, and
// #{...} literals construct natively. A user function shadowing one of
// these builtin names refuses at specialization and demotes any entry
// that already baked the native.

#[test]
fn map_literals_construct_and_read() {
    assert_jit_transparent(
        r#"
fn scores(a, b) = #{ "ada": a, "bob": b }
fn of(m, k) = map_get(m, k)
let m = scores(99, 87)
of(m, "ada") + of(m, "bob")
"#,
    );
}

#[test]
fn map_reads_compile_in_loops() {
    assert_jit_transparent(
        r#"
fn total(m, n) = {
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + (if map_has_key(m, "step") => map_get(m, "step") else => 0)
        i = i + 1
    }
    acc
}
total(#{ "step": 3 }, 10) + total(#{ "other": 1 }, 10)
"#,
    );
}

#[test]
fn map_set_chains_build_natively() {
    assert_jit_transparent(
        r#"
fn configured(port) = map_set(map_set(#{ "tls": 0 }, "port", port), "workers", 4)
let c = configured(8080)
map_get(c, "port") + map_get(c, "workers") + map_get(c, "tls")
"#,
    );
}

#[test]
fn string_valued_maps_agree() {
    assert_jit_transparent(
        r#"
fn headers(host) = #{ "host": host, "accept": "text/html" }
fn header(m, k) = map_get(m, k)
let h = headers("example.test")
header(h, "host") + " " + header(h, "accept")
"#,
    );
}

#[test]
fn map_misses_deopt_and_agree() {
    assert_jit_transparent(
        r#"
fn lookup(m, k) = map_get(m, k)
let m = #{ "a": 1 }
show(lookup(m, "a")) + " " + show(lookup(m, "missing"))
"#,
    );
}

#[test]
fn mixed_value_maps_stay_on_bytecode_and_agree() {
    assert_jit_transparent(
        r#"
fn odd(n) = #{ "count": n, "label": "x" }
fn read(n) = map_get(odd(n), "count")
read(4)
"#,
    );
}

#[test]
fn stringified_keys_stay_on_bytecode_and_agree() {
    assert_jit_transparent(
        r#"
fn by_num(m, k) = map_get(m, k)
by_num(#{ "1": 10 }, 1)
"#,
    );
}

#[test]
fn map_get_on_objects_stays_on_bytecode_and_agrees() {
    assert_jit_transparent(
        r#"
type Cfg = struct { port: Int }
fn read(c) = map_get(c, "port")
read(Cfg { port: 8080 })
"#,
    );
}

#[test]
fn annotated_map_returns_discharge_statically() {
    assert_jit_transparent(
        r#"
fn build(n) -> Map = #{ "n": n, "twice": n * 2 }
map_get(build(21), "twice")
"#,
    );
}

#[test]
fn user_shadow_of_map_get_refuses_the_native() {
    // A user definition of map_get exists before `read` ever runs: the
    // JIT's shadowed-name set must refuse to bake the native, so `read`
    // resolves the user's function exactly as the interpreter does.
    // (Shadowing AFTER a caller is already hot is a pre-existing tier
    // divergence tracked separately; the JIT's note_shadow demotion
    // keeps native code consistent with bytecode either way.)
    assert_jit_transparent(
        r#"
fn map_get(m, k) = 999
fn read(m, k) = map_get(m, k)
let m = #{ "a": 1 }
let mut acc = 0
let mut i = 0
while i < 10 { acc = acc + read(m, "a"); i = i + 1 }
acc + read(m, "a")
"#,
    );
}

#[test]
fn empty_map_literals_construct_and_grow() {
    assert_jit_transparent(
        r#"
fn base() = #{}
fn grown(v) = map_set(base(), "v", v)
map_get(grown(7), "v")
"#,
    );
}

/// Run tiered and return the tier's stats alongside the result.
fn eval_with_stats(source: &str) -> (Result<Value, String>, olang::ovm::tier::TierStats) {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string());
        let mut interpreter = Interpreter::new();
        interpreter.enable_bytecode_tier(1, false);
        let result = match program {
            Ok(p) => interpreter.eval_program(p).map_err(|e| e.to_string()),
            Err(e) => Err(e),
        };
        let stats = interpreter.bytecode_tier_stats().unwrap_or_default();
        (result, stats)
    })
}

// ── The constructor call boundary ──
//
// Perf guard for the regression where `fn make(i) = [i, i * 2]` called
// 3M times from a bytecode loop ran ~26% slower once the JIT learned to
// compile list constructors: the native body does the same allocation
// the bytecode would, so every boundary crossing (argument marshalling,
// scratch-context setup, retain resolution, unmarshal) is pure cost.
// The policy under test: trivial allocating bodies decline the boundary
// and stay on bytecode; bodies with enough compute still take it.

#[test]
fn trivial_list_constructor_loops_compile_as_one_native_entry() {
    let (result, stats) = eval_with_stats(
        r#"
fn make(i) = [i, i * 2]
fn bench(n) = {
    let mut i = 0
    let mut acc = 0
    while i < n { acc = acc + make(i)[0]; i = i + 1 }
    acc
}
bench(200)
"#,
    );
    assert_eq!(result, Ok(Value::Integer(19900)));
    // The watermarked loop compiles the CALLER, so the boundary is
    // crossed once for the whole loop — never per constructor call
    // (which would show ~200 entries here).
    assert_eq!(
        stats.jit_native_calls, 1,
        "the calling loop should enter native code exactly once"
    );
}

#[test]
fn trivial_struct_constructor_loops_compile_as_one_native_entry() {
    let (result, stats) = eval_with_stats(
        r#"
type Point = struct { x: Int, y: Int }
fn make(i) = Point { x: i, y: i * 2 }
fn bench(n) = {
    let mut i = 0
    let mut acc = 0
    while i < n { acc = acc + make(i).x; i = i + 1 }
    acc
}
bench(200)
"#,
    );
    assert_eq!(result, Ok(Value::Integer(19900)));
    // Same as the list twin: one native entry for the whole loop.
    assert_eq!(
        stats.jit_native_calls, 1,
        "the calling loop should enter native code exactly once"
    );
}

#[test]
fn compute_heavy_constructor_still_takes_the_native_boundary() {
    // Nine compute ops — above BOUNDARY_MIN_COMPUTE — so the native body
    // out-earns the boundary and the JIT must still run it.
    let (result, stats) = eval_with_stats(
        r#"
type Body = struct { a: Int, b: Int }
fn make(i) = {
    let a = i * 3 + 1
    let b = a * a - i
    let c = (b + a) * 2 + i * 7
    let d = c - b + a * 5
    Body { a: a + d, b: b + c }
}
fn bench(n) = {
    let mut i = 0
    let mut acc = 0
    while i < n { acc = acc + make(i).a; i = i + 1 }
    acc
}
bench(200)
"#,
    );
    assert!(result.is_ok(), "heavy constructor result: {result:?}");
    assert!(
        stats.jit_native_calls > 0,
        "a compute-heavy constructor must still run natively at the boundary"
    );
}

#[test]
fn declined_constructor_still_compiles_inside_a_native_caller() {
    // The boundary policy must not leak into group compilation: a
    // straight-line caller that needs the constructor natively still
    // compiles it as a group member and reaches it by direct call.
    assert_jit_transparent(
        r#"
fn make(i) = [i, i * 2]
fn use_it(i) = make(i)
use_it(21)
"#,
    );
}

// ── the watermark: allocation inside native loops ──────────────────────
//
// A loop may allocate when every heap value born in an iteration
// provably dies in it: scratch is marked at loop entry and truncated at
// each back-edge. The gate is liveness at the loop head — anything that
// survives an iteration (a carried binding, an accumulating list, a
// value read after the loop) refuses and stays on bytecode.

#[test]
fn allocating_loops_agree_and_run_native() {
    let (result, stats) = eval_with_stats(
        r#"
type P = struct { x: Float, y: Float }
fn dist2(a, b) = {
    let dx = b.x - a.x
    let dy = b.y - a.y
    dx * dx + dy * dy
}
fn kernel(n) = {
    let mut acc = 0.0
    let mut i = 0.0
    while i < n {
        acc = acc + dist2(P { x: i, y: 0.5 }, P { x: 0.0, y: i })
        i = i + 1.0
    }
    acc
}
kernel(500.0)
"#,
    );
    assert!(matches!(result, Ok(Value::Float(_))));
    assert!(
        stats.jit_native_calls >= 1,
        "the allocating loop should run natively"
    );
}

#[test]
fn loop_carried_heap_values_refuse_and_agree() {
    // `last` survives the back-edge, so truncation would free it out
    // from under the next iteration (and the after-loop read): the gate
    // must refuse, and bytecode must produce the identical answer.
    assert_jit_transparent(
        r#"
type P = struct { x: Float, y: Float }
fn escape(n) = {
    let mut last = P { x: 0.0, y: 0.0 }
    let mut i = 0.0
    while i < n {
        last = P { x: i, y: i }
        i = i + 1.0
    }
    last.x
}
escape(200.0)
"#,
    );
}

#[test]
fn accumulating_lists_refuse_and_agree() {
    // xs is live at the head (each iteration reads the last one): the
    // concat allocation cannot be watermarked.
    assert_jit_transparent(
        r#"
fn upto(n) = {
    let mut xs = [0]
    let mut i = 1
    while i < n { xs = xs + [i]; i = i + 1 }
    xs[n - 1]
}
upto(50)
"#,
    );
}

#[test]
fn watermarked_loops_outlive_the_allocation_cap() {
    // 1.2M iterations, each allocating: without per-iteration release
    // this deopts at the scratch cap mid-loop; with it, the loop runs
    // native start to finish. Either way the answer must agree.
    assert_jit_transparent(
        r#"
type B = struct { v: Int }
fn total(n) = {
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + B { v: i }.v
        i = i + 1
    }
    acc
}
total(1200000)
"#,
    );
}

#[test]
fn preloop_allocations_survive_the_watermark() {
    // The struct is built BEFORE the loop (below the mark) and read
    // inside every iteration: truncation must never touch it.
    assert_jit_transparent(
        r#"
type Cfg = struct { step: Float }
fn run(n) = {
    let cfg = Cfg { step: 2.5 }
    let mut acc = 0.0
    let mut i = 0.0
    while i < n {
        acc = acc + cfg.step
        i = i + 1.0
    }
    acc
}
run(300.0)
"#,
    );
}

#[test]
fn results_and_maps_in_loops_agree_under_the_watermark() {
    assert_jit_transparent(
        r#"
fn sign(n) = if n % 2 == 0 => Ok(n) else => Err(n * 3)
fn tally(n) = {
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + match sign(i) {
            Ok(v) => v,
            Err(e) => e
        }
        i = i + 1
    }
    acc
}
fn mtally(n) = {
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + map_get(#{ "v": i }, "v")
        i = i + 1
    }
    acc
}
tally(400) + mtally(400)
"#,
    );
}

// ── Campaign 7, T4: transitive inlining ─────────────────────────────

#[test]
fn helper_chains_inline_transitively_and_agree() {
    // score → dist2 → sq: three levels of helper. dist2 has a call of
    // its own, so the pre-T4 inliner refused it; now sq inlines into
    // dist2's expanded form and that form inlines into score's callers.
    // Correctness first: the flattened native loop must match the
    // interpreter exactly, guard edges included.
    assert_jit_transparent(
        "fn sq(x) = x * x\n\
         fn dist2(a, b) = sq(b - a) + 1\n\
         fn score(a, b, c) = dist2(a, b) + dist2(b, c)\n\
         fn run(n) = {\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < n { acc = (acc + score(i, i + 3, i + 5)) % 1000000007 i = i + 1 }\n\
             acc\n\
         }\n\
         run(20000)",
    );
}

#[test]
fn a_recursive_helper_stays_a_call_and_agrees() {
    // fact calls itself: its expansion bottoms out with the recursive
    // call still present, so the inliner must leave it as a call — and
    // the answer must not drift either way.
    assert_jit_transparent(
        "fn fact(n) = if n <= 1 => 1 else => n * fact(n - 1)\n\
         fn run(n) = {\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < n { acc = (acc + fact(10)) % 1000000007 i = i + 1 }\n\
             acc\n\
         }\n\
         run(5000)",
    );
}

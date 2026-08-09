//! End-to-end tests for hot-function promotion.
//!
//! These run whole programs through the public `Interpreter` API twice — once
//! with the bytecode tier enabled and once without — and assert the observable
//! results are identical. Where the differential suite tests the VM in
//! isolation, these test the promotion *decision*: that hot functions get
//! promoted, that ineligible ones keep working, and that mixing the two in one
//! program is invisible to the user.

use olang::ast::Value;
use olang::{Interpreter, Parser};

/// Run a closure on a thread with the same generous stack the CLI gives the
/// interpreter (test threads default to ~2 MB, far less than deep recursion
/// needs).
fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

/// Evaluate a program, optionally with the bytecode tier enabled.
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

/// Assert a program produces the same result with and without promotion.
fn assert_tier_transparent(source: &str) {
    let interpreted = eval(source, None);
    let promoted = eval(source, Some(2));

    match (&interpreted, &promoted) {
        (Ok(a), Ok(b)) => assert_eq!(
            a, b,
            "promotion changed the result\n  interpreted: {:?}\n  promoted:    {:?}\n  source: {}",
            a, b, source
        ),
        (Err(_), Err(_)) => {}
        _ => panic!(
            "promotion changed success/failure\n  interpreted: {:?}\n  promoted:    {:?}\n  source: {}",
            interpreted, promoted, source
        ),
    }
}

/// Run a program with the tier on and report how many functions were promoted.
fn promotion_count(source: &str, threshold: u32) -> u32 {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).unwrap();
        let mut interpreter = Interpreter::new();
        interpreter.enable_bytecode_tier(threshold, false);
        interpreter.eval_program(program).unwrap();
        interpreter.bytecode_tier_stats().unwrap().promoted
    })
}

#[test]
fn hot_recursive_function_is_promoted() {
    let src = r#"
fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)
fib(15)
"#;
    assert_eq!(promotion_count(src, 2), 1, "fib should be promoted");
    assert_tier_transparent(src);
}

#[test]
fn promoted_recursion_produces_correct_values() {
    let src = r#"
fn fact(n) = if n <= 1 => 1 else => n * fact(n - 1)
fact(12)
"#;
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(479001600));
    assert_tier_transparent(src);
}

#[test]
fn cold_functions_are_not_promoted() {
    let src = r#"
fn once(n) = n * 2
once(21)
"#;
    assert_eq!(
        promotion_count(src, 50),
        0,
        "a function called once must not be promoted"
    );
    assert_tier_transparent(src);
}

#[test]
fn loop_heavy_function_is_transparent() {
    let src = r#"
fn sum_to(n) = {
    let total = 0
    let i = 0
    while i <= n {
        total = total + i
        i = i + 1
    }
    total
}
sum_to(10) + sum_to(100) + sum_to(1000)
"#;
    assert_eq!(
        eval(src, Some(1)).unwrap(),
        Value::Integer(55 + 5050 + 500500)
    );
    assert_tier_transparent(src);
}

#[test]
fn functions_using_globals_now_promote_and_agree() {
    // A free identifier resolving in the function's own closure compiles as
    // a baked constant — sound because the interpreter installs exactly that
    // closure as the call environment, and closures are snapshots.
    let src = r#"
let base = 100
fn offset(n) = n + base
offset(1) + offset(2) + offset(3)
"#;
    assert_eq!(promotion_count(src, 1), 1, "offset should now compile");
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(306));
    assert_tier_transparent(src);
}

#[test]
fn baked_globals_are_declaration_time_snapshots() {
    // The whole baking design rests on closures being snapshots: a global
    // mutated after the function's declaration must NOT be seen — by either
    // tier. Pin the exact value, not just agreement.
    let src = r#"
let mut base = 100
fn offset(n) = n + base
base = 200
offset(1) + offset(2) + offset(3)
"#;
    assert_eq!(eval(src, None).unwrap(), Value::Integer(306));
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(306));
    assert_tier_transparent(src);
}

#[test]
fn baked_function_values_survive_redefinition_of_the_name() {
    // `go` bakes the helper *value* from its own closure. Redefining the
    // name afterwards must not change what go calls — in either tier —
    // because go's closure still holds the original.
    let src = r#"
fn helper(x) = x + 1
fn go(xs) = xs |> map(helper) |> sum
let xs = [10, 20, 30]
let before = go(xs) + go(xs)
fn helper(x) = x + 1000
before + go(xs)
"#;
    assert_eq!(eval(src, None).unwrap(), Value::Integer(63 * 3));
    assert_tier_transparent(src);
}

#[test]
fn functions_using_unsupported_features_keep_working() {
    // Map literals are not compiled; the function must fall back and still
    // produce the right answer.
    let src = r#"
fn lookup(k) = {
    let m = #{"a": 1, "b": 2}
    map_get(m, k)
}
to_string(lookup("a")) + to_string(lookup("b"))
"#;
    assert_eq!(promotion_count(src, 1), 0);
    assert_tier_transparent(src);
}

#[test]
fn pipelines_and_closures_keep_working() {
    let src = r#"
fn add_n(n) = (x) => x + n
let add5 = add_n(5)
let nums = [1, 2, 3, 4]
nums |> map(add5) |> sum
"#;
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(30));
    assert_tier_transparent(src);
}

#[test]
fn mixed_eligible_and_ineligible_functions() {
    // square is compilable; describe is not (uses a global and a builtin the
    // VM doesn't implement). Both must work, in the same program.
    let src = r#"
let label = "n="
fn square(n) = n * n
fn describe(n) = label + to_string(n)
square(2) + square(3) + square(4) + len(describe(7))
"#;
    assert_tier_transparent(src);
    assert_eq!(eval(src, Some(2)).unwrap(), eval(src, None).unwrap());
}

#[test]
fn errors_are_reported_identically() {
    let src = r#"
fn div(a, b) = a / b
div(10, 2)
div(1, 0)
"#;
    assert!(eval(src, None).is_err());
    assert!(eval(src, Some(1)).is_err());
    assert_tier_transparent(src);
}

#[test]
fn integer_overflow_errors_after_promotion() {
    let src = r#"
fn bump(n) = n + 1
bump(1)
bump(2)
bump(9223372036854775807)
"#;
    assert_tier_transparent(src);
}

#[test]
fn runaway_recursion_errors_rather_than_crashing() {
    let src = r#"
fn boom(n) = boom(n + 1)
boom(0)
"#;
    // Must be a clean error in both modes — not a stack overflow abort
    assert!(eval(src, None).is_err());
    assert!(eval(src, Some(2)).is_err());
}

#[test]
fn deep_but_valid_recursion_agrees() {
    let src = r#"
fn countdown(n) = if n <= 0 => 0 else => 1 + countdown(n - 1)
countdown(400)
"#;
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(400));
    assert_tier_transparent(src);
}

#[test]
fn string_and_list_results_round_trip() {
    let src = r#"
fn greet(name) = "hi " + name
fn pair(a, b) = [a, b, a + b]
greet("ann")
greet("bob")
pair(1, 2)
pair(3, 4)
"#;
    assert_tier_transparent(src);
    assert_eq!(
        eval(src, Some(1)).unwrap(),
        Value::List(vec![Value::Integer(3), Value::Integer(4), Value::Integer(7)].into())
    );
}

#[test]
fn promotion_threshold_is_respected() {
    let src = r#"
fn f(n) = n + 1
f(1)
f(2)
f(3)
f(4)
f(5)
"#;
    assert_eq!(promotion_count(src, 3), 1, "should promote on the 3rd call");
    assert_eq!(promotion_count(src, 99), 0, "threshold above call count");
}

// --- Transitive compilation ---
// A promoted function may call other user functions; the tier compiles those
// too. These cover the cases where that can go wrong.

#[test]
fn helper_functions_are_compiled_transitively() {
    let src = r#"
fn square(n) = n * n
fn sum_squares(n) = if n <= 0 => 0 else => square(n) + sum_squares(n - 1)
sum_squares(20)
"#;
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(2870));
    // Both the caller and the helper should be promoted
    assert_eq!(promotion_count(src, 2), 2);
    assert_tier_transparent(src);
}

#[test]
fn mutual_recursion_compiles() {
    let src = r#"
fn is_even(n) = if n == 0 => true else => is_odd(n - 1)
fn is_odd(n) = if n == 0 => false else => is_even(n - 1)
is_even(100)
"#;
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Boolean(true));
    assert_eq!(promotion_count(src, 2), 2);
    assert_tier_transparent(src);
}

#[test]
fn deep_helper_chain_compiles() {
    let src = r#"
fn a(n) = n + 1
fn b(n) = a(n) * 2
fn c(n) = b(n) + a(n)
fn d(n) = c(n) + b(n)
d(1) + d(2) + d(3)
"#;
    assert_tier_transparent(src);
    assert_eq!(eval(src, Some(1)).unwrap(), eval(src, None).unwrap());
}

#[test]
fn caller_is_rejected_when_helper_cannot_compile() {
    // `describe` builds a map literal, which the tier does not compile, so
    // neither it nor its caller may be promoted — but the program must
    // still produce the right answer. (This used to use a global as the
    // uncompilable feature; globals bake as closure constants now.)
    let src = r#"
fn describe(n) = {
    let m = #{"n": n}
    to_string(map_get(m, "n"))
}
fn label(n) = describe(n) + "!"
len(label(1)) + len(label(2))
"#;
    assert_eq!(promotion_count(src, 1), 0);
    assert_tier_transparent(src);
}

#[test]
fn helper_returning_a_string_round_trips() {
    let src = r#"
fn greet(name) = "hi " + name
fn shout(name) = greet(name) + "!"
shout("ann")
shout("bob")
"#;
    assert_tier_transparent(src);
    assert_eq!(
        eval(src, Some(1)).unwrap(),
        Value::String("hi bob!".to_string().into())
    );
}

#[test]
fn redefining_a_function_uses_the_new_body() {
    // A promoted function that is later redefined must not keep executing the
    // stale compiled body.
    let src = r#"
fn f(n) = n + 1
f(1)
f(2)
f(3)
fn f(n) = n + 100
f(4)
"#;
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(104));
    assert_tier_transparent(src);
}

#[test]
fn helper_errors_propagate_through_the_caller() {
    let src = r#"
fn div(a, b) = a / b
fn safe(a) = div(10, a)
safe(2)
safe(1)
safe(0)
"#;
    assert!(eval(src, None).is_err());
    assert!(eval(src, Some(1)).is_err());
    assert_tier_transparent(src);
}

#[test]
fn rejected_helper_does_not_leave_a_dangling_registration() {
    // A function's name is registered with the VM before compiling so that
    // recursion resolves. If compilation then fails, that registration must be
    // withdrawn — otherwise a later function compiles a call against a name
    // that has no bytecode and dies at runtime with "Function not found".
    //
    // Order matters: `clamp` must be rejected on its own first (it uses an
    // operator the VM lacks), and only later does `poly` compile a call to it.
    let src = r#"
fn square(n) = n * n
fn clamp(n) = if n > 10 => n % 10 else => n
fn poly(n) = square(clamp(n)) + clamp(n)
fn run(limit) = {
    let total = 0
    let i = 0
    while i < limit {
        total = total + poly(i)
        i = i + 1
    }
    total
}
run(300)
"#;
    assert_tier_transparent(src);
    assert_eq!(eval(src, Some(5)).unwrap(), eval(src, None).unwrap());
}

// --- Builtins ---

#[test]
fn user_function_shadows_a_builtin_of_the_same_name() {
    // The VM knows a `clamp` builtin. A user function of the same name must
    // win, exactly as it does in the interpreter's environment lookup —
    // otherwise a promoted caller silently calls the wrong function.
    let src = r#"
fn clamp(n) = n + 1000
fn use_it(n) = clamp(n)
use_it(1) + use_it(2) + use_it(3)
"#;
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(3006));
    assert_tier_transparent(src);
}

#[test]
fn builtins_are_callable_from_promoted_functions() {
    let src = r#"
fn describe(n) = len(to_string(n * 111))
describe(1) + describe(22) + describe(333)
"#;
    assert_tier_transparent(src);
    assert_eq!(promotion_count(src, 2), 1, "should still promote");
}

#[test]
fn list_builtins_agree() {
    let src = r#"
fn work(n) = {
    let xs = range(0, n)
    sum(xs) + len(xs) + head(reverse(xs))
}
work(5) + work(10) + work(20)
"#;
    assert_tier_transparent(src);
}

#[test]
fn string_builtins_agree() {
    let src = r#"
fn shout(s) = {
    let parts = split(s, ",")
    join(parts, "-") + to_string(len(parts))
}
shout("a,b,c")
shout("x,y")
"#;
    assert_tier_transparent(src);
}

#[test]
fn result_builtins_agree() {
    let src = r#"
fn check(n) = if is_ok(Ok(n)) => unwrap_or(Ok(n), 0) else => 0
check(1) + check(2) + check(3)
"#;
    assert_tier_transparent(src);
}

#[test]
fn builtin_errors_match_the_interpreter() {
    // head([]) errors; promotion must not change that
    let src = r#"
fn first(xs) = head(xs)
first([1, 2])
first([3])
first([])
"#;
    assert!(eval(src, None).is_err());
    assert!(eval(src, Some(1)).is_err());
    assert_tier_transparent(src);
}

// --- for loops, break, continue ---

#[test]
fn for_over_range_agrees() {
    let src = r#"
fn total(n) = {
    let acc = 0
    for i in 0..n {
        acc = acc + i
    }
    acc
}
total(10) + total(100) + total(1000)
"#;
    assert_eq!(
        eval(src, Some(1)).unwrap(),
        Value::Integer(45 + 4950 + 499500)
    );
    assert_tier_transparent(src);
}

#[test]
fn for_over_inclusive_range_agrees() {
    let src = r#"
fn total(n) = {
    let acc = 0
    for i in 0..=n {
        acc = acc + i
    }
    acc
}
total(10) + total(100)
"#;
    assert_tier_transparent(src);
}

#[test]
fn for_over_empty_and_reversed_ranges() {
    let src = r#"
fn total(a, b) = {
    let acc = 0
    for i in a..b {
        acc = acc + i
    }
    acc
}
total(5, 5) + total(10, 1) + total(0, 3)
"#;
    assert_tier_transparent(src);
}

#[test]
fn for_over_list_agrees() {
    let src = r#"
fn total(xs) = {
    let acc = 0
    for x in xs {
        acc = acc + x
    }
    acc
}
total([1, 2, 3]) + total([]) + total([10, 20])
"#;
    assert_tier_transparent(src);
}

#[test]
fn break_exits_the_loop() {
    let src = r#"
fn first_big(n) = {
    let found = 0
    for i in 0..n {
        if i * i > 50 => {
            found = i
            break
        }
    }
    found
}
first_big(100) + first_big(3)
"#;
    assert_tier_transparent(src);
}

#[test]
fn continue_skips_iterations() {
    let src = r#"
fn odds(n) = {
    let acc = 0
    for i in 0..n {
        if i % 2 == 0 => continue
        acc = acc + i
    }
    acc
}
odds(10) + odds(21)
"#;
    assert_tier_transparent(src);
}

#[test]
fn break_and_continue_in_while_loops() {
    let src = r#"
fn work(n) = {
    let acc = 0
    let i = 0
    while true {
        i = i + 1
        if i > n => break
        if i % 3 == 0 => continue
        acc = acc + i
    }
    acc
}
work(10) + work(30)
"#;
    assert_tier_transparent(src);
}

#[test]
fn nested_loops_with_break() {
    // break must exit only the innermost loop
    let src = r#"
fn grid(n) = {
    let acc = 0
    for i in 0..n {
        for j in 0..n {
            if j > i => break
            acc = acc + 1
        }
    }
    acc
}
grid(5) + grid(10)
"#;
    assert_tier_transparent(src);
}

// --- match expressions ---

#[test]
fn match_on_literals_agrees() {
    let src = r#"
fn classify(n) = match n % 4 {
    0 => "zero",
    1 => "one",
    2 => "two",
    _ => "three"
}
classify(0) + classify(1) + classify(2) + classify(3) + classify(7)
"#;
    assert_tier_transparent(src);
    assert_eq!(promotion_count(src, 2), 1, "classify should be promoted");
}

#[test]
fn match_binding_pattern_agrees() {
    let src = r#"
fn describe(n) = match n {
    0 => 0,
    x => x * 2
}
describe(0) + describe(5) + describe(9)
"#;
    assert_tier_transparent(src);
}

#[test]
fn match_range_patterns_agree() {
    let src = r#"
fn band(n) = match n {
    0 => "zero",
    1..10 => "small",
    10..100 => "medium",
    _ => "large"
}
band(0) + band(5) + band(50) + band(500)
"#;
    assert_tier_transparent(src);
}

#[test]
fn match_guards_agree() {
    let src = r#"
fn sign(n) = match n {
    x if x < 0 => 0 - 1,
    0 => 0,
    _ => 1
}
sign(0 - 5) + sign(0) + sign(5)
"#;
    assert_tier_transparent(src);
}

#[test]
fn match_or_patterns_agree() {
    let src = r#"
fn vowelish(n) = match n {
    1 | 5 | 9 => "hit",
    _ => "miss"
}
vowelish(1) + vowelish(2) + vowelish(5) + vowelish(9)
"#;
    assert_tier_transparent(src);
}

#[test]
fn match_on_strings_agrees() {
    let src = r#"
fn route(path) = match path {
    "home" => 1,
    "about" => 2,
    _ => 0
}
route("home") + route("about") + route("other")
"#;
    assert_tier_transparent(src);
}

#[test]
fn match_type_mismatch_does_not_error() {
    // A literal pattern of a different type must simply not match, rather
    // than raising the type error a plain equality would.
    let src = r#"
fn odd_one(n) = match n {
    "text" => 1,
    0 => 2,
    _ => 3
}
odd_one(0) + odd_one(7)
"#;
    assert_tier_transparent(src);
}

#[test]
fn non_exhaustive_match_fails_the_same_way() {
    let src = r#"
fn only_zero(n) = match n {
    0 => "zero"
}
only_zero(0)
only_zero(1)
"#;
    assert!(eval(src, None).is_err());
    assert!(eval(src, Some(1)).is_err());
    assert_tier_transparent(src);
}

#[test]
fn result_destructuring_is_now_compiled() {
    // Superseded the earlier expectation that Ok/Err patterns force a
    // fallback — they compile now, and must still agree.
    let src = r#"
fn unwrap_or_zero(r) = match r {
    Ok(v) => v,
    Err(e) => 0
}
unwrap_or_zero(Ok(5)) + unwrap_or_zero(Err("x")) + unwrap_or_zero(Ok(7))
"#;
    assert_eq!(promotion_count(src, 1), 1);
    assert_tier_transparent(src);
}

#[test]
fn match_inside_a_loop_agrees() {
    let src = r#"
fn tally(n) = {
    let acc = 0
    for i in 0..n {
        acc = acc + match i % 3 {
            0 => 10,
            1 => 100,
            _ => 1
        }
    }
    acc
}
tally(10) + tally(31)
"#;
    assert_tier_transparent(src);
}

// --- destructuring patterns ---

#[test]
fn result_patterns_agree() {
    let src = r#"
fn take(r) = match r {
    Ok(v) => v,
    Err(e) => 0 - 1
}
take(Ok(5)) + take(Err("boom")) + take(Ok(10))
"#;
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(14));
    assert_tier_transparent(src);
    assert_eq!(promotion_count(src, 2), 1, "should now be promoted");
}

#[test]
fn result_patterns_with_nested_literals() {
    let src = r#"
fn kind(r) = match r {
    Ok(0) => "zero",
    Ok(x) => "some",
    Err(e) => "bad"
}
kind(Ok(0)) + kind(Ok(3)) + kind(Err("x"))
"#;
    assert_tier_transparent(src);
}

#[test]
fn result_pattern_against_non_result_does_not_error() {
    // An Ok(..) pattern must simply not match a plain integer
    let src = r#"
fn kind(v) = match v {
    Ok(x) => 1,
    Err(e) => 2,
    _ => 3
}
kind(Ok(1)) + kind(Err("e")) + kind(42)
"#;
    assert_tier_transparent(src);
}

#[test]
fn tuple_patterns_agree() {
    let src = r#"
fn combine(t) = match t {
    (a, b) => a * 10 + b,
    _ => 0
}
combine((1, 2)) + combine((3, 4))
"#;
    assert_tier_transparent(src);
}

#[test]
fn tuple_arity_must_match() {
    let src = r#"
fn shape(t) = match t {
    (a, b) => 2,
    (a, b, c) => 3,
    _ => 0
}
shape((1, 2)) + shape((1, 2, 3)) + shape(9)
"#;
    assert_tier_transparent(src);
}

#[test]
fn list_patterns_agree() {
    let src = r#"
fn describe(xs) = match xs {
    [] => 0,
    [a] => a,
    [a, b] => a + b,
    _ => 0 - 1
}
describe([]) + describe([5]) + describe([2, 3]) + describe([1, 2, 3])
"#;
    assert_tier_transparent(src);
}

#[test]
fn list_rest_patterns_agree() {
    let src = r#"
fn total(xs) = match xs {
    [] => 0,
    [head, ...tail] => head + total(tail)
}
total([1, 2, 3, 4, 5])
"#;
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(15));
    assert_tier_transparent(src);
}

#[test]
fn nested_destructuring_agrees() {
    let src = r#"
fn dig(r) = match r {
    Ok([a, b]) => a + b,
    Ok([a]) => a,
    Ok(_) => 0,
    Err(e) => 0 - 1
}
dig(Ok([1, 2])) + dig(Ok([7])) + dig(Ok([1,2,3])) + dig(Err("x"))
"#;
    assert_tier_transparent(src);
}

#[test]
fn destructuring_with_guards_agrees() {
    let src = r#"
fn pick(r) = match r {
    Ok(v) if v > 10 => "big",
    Ok(v) => "small",
    Err(e) => "bad"
}
pick(Ok(50)) + pick(Ok(1)) + pick(Err("e"))
"#;
    assert_tier_transparent(src);
}

#[test]
fn destructuring_in_a_loop_agrees() {
    let src = r#"
fn sum_oks(xs) = {
    let total = 0
    for x in xs {
        total = total + match x {
            Ok(v) => v,
            Err(e) => 0
        }
    }
    total
}
sum_oks([Ok(1), Err("a"), Ok(2), Ok(3)])
"#;
    assert_tier_transparent(src);
}

#[test]
fn struct_patterns_still_fall_back() {
    // Struct patterns are not compiled; the function must stay interpreted
    // and still be correct.
    let src = r#"
type User = struct { name: String, age: Int }
fn label(u) = match u {
    User { name, age } => name
}
label(User { name: "ann", age: 30 })
label(User { name: "bob", age: 40 })
"#;
    assert_eq!(promotion_count(src, 1), 0);
    assert_tier_transparent(src);
}

#[test]
fn result_constructing_functions_are_promoted() {
    // Ok(..)/Err(..) construction compiles, so a function producing Results
    // is eligible rather than forcing its whole call graph to interpret.
    let src = r#"
fn parse(n) = if n % 5 == 0 => Err("bad") else => Ok(n * 2)
fn value(r) = match r {
    Ok(v) => v,
    Err(e) => 0
}
value(parse(1)) + value(parse(5)) + value(parse(7))
"#;
    assert_eq!(promotion_count(src, 2), 2, "both should be promoted");
    assert_tier_transparent(src);
}

#[test]
fn results_can_cross_the_tier_boundary_as_arguments() {
    // A Result passed into a promoted function must reach the VM rather than
    // forcing a fallback — Results round-trip through the value model.
    let src = r#"
fn value(r) = match r {
    Ok(v) => v,
    Err(e) => 0
}
fn run(limit) = {
    let total = 0
    for i in 0..limit {
        total = total + value(Ok(i))
    }
    total
}
run(200)
"#;
    let parser = Parser::new();
    let program = parser.parse(src).unwrap();
    let mut interpreter = Interpreter::new();
    interpreter.enable_bytecode_tier(2, false);
    interpreter.eval_program(program).unwrap();
    let stats = interpreter.bytecode_tier_stats().unwrap();
    assert!(
        stats.bytecode_calls > 0,
        "promoted functions taking Result arguments should actually run on the VM"
    );
    assert_tier_transparent(src);
}

#[test]
fn results_round_trip_as_return_values() {
    let src = r#"
fn wrap(n) = if n > 0 => Ok(n) else => Err("neg")
fn total(n) = match wrap(n) {
    Ok(v) => v,
    Err(e) => 0
}
total(5) + total(0 - 3) + total(9)
"#;
    assert_tier_transparent(src);
}

// --- non-capturing lambdas and pipelines ---

#[test]
fn pipeline_with_lambdas_is_promoted() {
    let src = r#"
fn process(n) = range(0, n)
    |> map((x) => x * 3)
    |> filter((x) => x % 2 == 0)
    |> fold(0, (acc, x) => acc + x)
process(10) + process(100) + process(50)
"#;
    assert_eq!(promotion_count(src, 2), 1, "should now be promoted");
    assert_tier_transparent(src);
}

#[test]
fn lambda_results_match_the_interpreter() {
    let src = r#"
fn doubled(xs) = map(xs, (x) => x * 2)
fn evens(xs) = filter(xs, (x) => x % 2 == 0)
fn total(xs) = fold(xs, 0, (acc, x) => acc + x)
total(doubled([1, 2, 3])) + total(evens([1, 2, 3, 4]))
"#;
    assert_tier_transparent(src);
}

#[test]
fn capturing_lambda_falls_back() {
    // The lambda references `factor` from the enclosing scope, so it cannot
    // be built with an empty closure — the function must stay interpreted
    // and still be correct.
    let src = r#"
fn scale(xs, factor) = map(xs, (x) => x * factor)
sum(scale([1, 2, 3], 10)) + sum(scale([1, 2], 5))
"#;
    assert_eq!(promotion_count(src, 1), 0);
    assert_tier_transparent(src);
}

#[test]
fn lambda_calling_a_function_is_now_compiled() {
    // The lambda carries the enclosing function's declaration-time closure,
    // so `square` resolves — this was the largest remaining rejection.
    let src = r#"
fn square(n) = n * n
fn squares(xs) = map(xs, (x) => square(x))
sum(squares([1, 2, 3])) + sum(squares([4, 5]))
"#;
    // Only `squares` promotes: `square` is called solely from inside the
    // lambda, which executes in the interpreter, so its own call counter
    // never advances at bytecode level.
    assert_eq!(promotion_count(src, 1), 1, "squares itself promotes");
    assert_tier_transparent(src);
}

#[test]
fn lambda_with_a_block_body_agrees() {
    let src = r#"
fn work(xs) = map(xs, (x) => {
    let doubled = x * 2
    doubled + 1
})
sum(work([1, 2, 3])) + sum(work([4, 5]))
"#;
    assert_tier_transparent(src);
}

#[test]
fn lambda_with_a_conditional_body_agrees() {
    let src = r#"
fn clip(xs) = map(xs, (x) => if x > 10 => 10 else => x)
sum(clip([1, 20, 5, 30])) + sum(clip([]))
"#;
    assert_tier_transparent(src);
}

#[test]
fn multi_parameter_lambdas_agree() {
    let src = r#"
fn combine(xs) = fold(xs, 0, (acc, x) => acc * 2 + x)
combine([1, 2, 3]) + combine([4, 5])
"#;
    assert_tier_transparent(src);
}

#[test]
fn pipeline_without_arguments_agrees() {
    // `xs |> f` (no argument list) desugars to `f(xs)`
    let src = r#"
fn total(xs) = xs |> sum
total([1, 2, 3]) + total([4, 5])
"#;
    assert_tier_transparent(src);
}

#[test]
fn lambda_errors_propagate_identically() {
    let src = r#"
fn risky(xs) = map(xs, (x) => x / 0)
risky([1])
risky([2])
"#;
    assert!(eval(src, None).is_err());
    assert!(eval(src, Some(1)).is_err());
    assert_tier_transparent(src);
}

#[test]
fn nested_lambdas_agree() {
    // The inner lambda captures the outer lambda's parameter — legal, since
    // the outer lambda executes in the interpreter, which handles the inner
    // capture naturally. The analysis just treats `x` as bound within the
    // outer body.
    let src = r#"
fn outer(xs) = map(xs, (x) => sum(map([1, 2], (y) => y * x)))
sum(outer([1, 2])) + sum(outer([3]))
"#;
    assert_tier_transparent(src);
}

// --- closure attachment ---
// A lambda carries the enclosing function's declaration-time closure. Free
// variables resolving there compile; anything touching the enclosing
// function's runtime state must still fall back.

#[test]
fn lambda_referencing_a_global_constant_agrees() {
    let src = r#"
let factor = 7
fn scale(xs) = map(xs, (x) => x * factor)
sum(scale([1, 2, 3])) + sum(scale([4, 5]))
"#;
    assert_eq!(promotion_count(src, 1), 1);
    assert_tier_transparent(src);
}

#[test]
fn global_mutation_after_declaration_agrees() {
    // Both tiers must see the declaration-time snapshot: the interpreter
    // layers the closure over the call-site chain, so the reassigned value
    // never wins inside `scale` either way.
    let src = r#"
let factor = 7
fn scale(xs) = map(xs, (x) => x * factor)
factor = 100
sum(scale([1, 2, 3]))
"#;
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(42));
    assert_tier_transparent(src);
}

#[test]
fn lambda_capturing_enclosing_param_still_falls_back() {
    let src = r#"
fn scale(xs, k) = map(xs, (x) => x * k)
sum(scale([1, 2, 3], 10)) + sum(scale([1, 2], 5))
"#;
    assert_eq!(promotion_count(src, 1), 0);
    assert_tier_transparent(src);
}

#[test]
fn lambda_capturing_enclosing_local_still_falls_back() {
    let src = r#"
fn work(xs) = {
    let offset = 3
    map(xs, (x) => x + offset)
}
sum(work([1, 2, 3]))
"#;
    assert_eq!(promotion_count(src, 1), 0);
    assert_tier_transparent(src);
}

#[test]
fn lambda_free_var_assigned_by_enclosing_fn_falls_back() {
    // `acc` is a global in the closure, but the enclosing function assigns
    // it, so the interpreter's lambda would capture the runtime value — the
    // declaration-time snapshot cannot represent that.
    let src = r#"
let acc = 1
fn f(xs) = {
    acc = 5
    map(xs, (x) => x * acc)
}
sum(f([1, 2]))
"#;
    assert_eq!(promotion_count(src, 1), 0);
    assert_tier_transparent(src);
}

#[test]
fn lambda_calling_a_builtin_agrees() {
    let src = r#"
fn stringify(xs) = map(xs, (x) => to_string(x * 2))
join(stringify([1, 2, 3]), ",")
"#;
    assert_tier_transparent(src);
}

#[test]
fn lambda_using_stdlib_module_agrees() {
    // `math` resolves through the closure (prelude bindings)
    let src = r#"
fn powers(xs) = map(xs, (x) => math.pow(x, 2))
sum(powers([1, 2, 3])) + sum(powers([4]))
"#;
    assert_tier_transparent(src);
}

#[test]
fn transitive_helper_with_closure_lambda_agrees() {
    // The lambda's callee is itself compiled transitively
    let src = r#"
fn double(n) = n * 2
fn doubles(xs) = map(xs, (x) => double(x))
fn run(n) = sum(doubles(range(0, n)))
run(10) + run(100)
"#;
    assert_tier_transparent(src);
}

#[test]
fn lambda_with_match_body_agrees() {
    let src = r#"
fn classify(xs) = map(xs, (x) => match x % 2 { 0 => "even", _ => "odd" })
join(classify([1, 2, 3]), "-")
"#;
    assert_tier_transparent(src);
}

#[test]
fn recursive_reference_inside_lambda_falls_back() {
    // `f` is not in its own declaration-time closure (the snapshot is taken
    // before the name is defined), so a lambda referencing it is rejected.
    let src = r#"
fn f(n) = if n <= 0 => 0 else => sum(map([1], (x) => f(n - 1)))
f(3)
"#;
    assert_eq!(promotion_count(src, 1), 0);
    assert_tier_transparent(src);
}

// ── logical operators (Tier 4: compiled as conditional jumps) ──────────

#[test]
fn logical_and_or_promote_and_agree() {
    // Guards everywhere: functions with && / || must now compile AND match
    // the interpreter exactly.
    assert_tier_transparent(
        r#"
fn in_range(x, lo, hi) = (x >= lo) && (x <= hi)
fn either_positive(a, b) = (a > 0) || (b > 0)
let mut hits = 0
for i in 0..50 {
    if in_range(i % 10, 3, 7) => { hits = hits + 1 }
    if either_positive(i - 49, 1) => { hits = hits + 1 }
}
hits
"#,
    );
}

#[test]
fn logical_ops_short_circuit_in_the_bytecode_tier() {
    // The right operand must not run when the left settles the result —
    // unwrap(Err(...)) would abort the program if evaluated.
    assert_tier_transparent(
        r#"
fn boom() = unwrap(Err("must not run"))
fn safe_and(flag) = flag && (boom() == 0)
fn safe_or(flag) = flag || (boom() == 0)
let mut n = 0
for i in 0..20 {
    if !safe_and(false) => { n = n + 1 }
    if safe_or(true) => { n = n + 1 }
}
n
"#,
    );
}

#[test]
fn logical_type_errors_agree_across_tiers() {
    // `0 && true` is a type error in the interpreter; the promoted function
    // must error too, not coerce by truthiness.
    let src = r#"
fn bad() = 0 && true
let mut n = 0
for i in 0..20 { n = n + (if bad() => 1 else => 0) }
n
"#;
    let interpreted = eval(src, None);
    let tiered = eval(src, Some(1));
    assert!(interpreted.is_err(), "interpreter should type-error");
    assert!(tiered.is_err(), "bytecode tier should type-error too");
}

// ── field access + indexing (roadmap lane 13) ─────────────────────────

#[test]
fn field_access_promotes_and_agrees() {
    assert_tier_transparent(
        r#"
type P = struct { x: Int, y: Int }
fn sum_dist(pts) = {
    let mut s = 0
    for p in pts { s = s + p.x * p.x + p.y * p.y }
    s
}
let pts = [P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]
sum_dist(pts) + sum_dist(pts)
"#,
    );
}

#[test]
fn indexing_promotes_and_agrees() {
    assert_tier_transparent(
        r#"
fn pick(xs) = xs[0] + xs[1] + xs[-1]
let mut total = 0
for i in 0..40 { total = total + pick([i, i + 1, i + 2, i + 3]) }
total
"#,
    );
}

#[test]
fn struct_returned_from_compiled_function_round_trips() {
    // A struct built and returned by a promoted function must survive the
    // OVM->AST conversion identically.
    assert_tier_transparent(
        r#"
type Pair = struct { a: Int, b: Int }
fn shift(p) = Pair { a: p.b, b: p.a + p.b }
let mut p = Pair { a: 0, b: 1 }
for i in 0..30 { p = shift(p) }
p.a + p.b
"#,
    );
}

#[test]
fn nested_field_and_index_agree() {
    assert_tier_transparent(
        r#"
type Box = struct { items: List, label: String }
fn first_item(b) = b.items[0]
let mut sum = 0
for i in 0..40 { sum = sum + first_item(Box { items: [i, i * 2], label: "b" }) }
sum
"#,
    );
}

#[test]
fn index_out_of_bounds_agrees_across_tiers() {
    // Both tiers must error the same way — no wrong value on either.
    let src = r#"
fn bad(xs) = xs[5]
let mut n = 0
for i in 0..30 { n = n + bad([1, 2]) }
n
"#;
    assert!(eval(src, None).is_err());
    assert!(eval(src, Some(1)).is_err());
}

// ── module math builtins compile in the tier ──────────────────────────

#[test]
fn math_builtins_promote_and_agree() {
    // A field-access + math kernel (the N-body shape) must promote and match
    // the interpreter — `math.sqrt` and friends are now compilable builtins.
    assert_tier_transparent(
        r#"
type Body = struct { x: Float, mass: Float }
fn pull(bi, bodies) = {
    let mut a = 0.0
    for bj in bodies {
        let dx = bj.x - bi.x
        let d2 = dx * dx + 0.5
        a = a + bj.mass * dx / (d2 * math.sqrt(d2))
    }
    a
}
let bodies = [Body { x: 0.0, mass: 1.0 }, Body { x: 1.0, mass: 2.0 }, Body { x: 3.0, mass: 1.5 }]
let mut total = 0.0
for i in 0..20 { total = total + pull(bodies[1], bodies) }
total
"#,
    );
}

#[test]
fn assorted_math_functions_agree_across_tiers() {
    assert_tier_transparent(
        r#"
fn f(x) = math.sqrt(x) + math.pow(x, 3.0) + math.abs(0.0 - x)
    + math.floor(x + 0.9) + math.max(x, 2.0) + math.min(x, 2.0)
let mut s = 0.0
for i in 0..30 { s = s + f(to_float(i) * 0.1 + 1.0) }
s
"#,
    );
}

#[test]
fn every_float_math_fast_path_function_agrees_across_tiers() {
    // The VM resolves these to a CallBuiltin fast path (pure f64, no AST
    // round-trip). Every function in that table, with Float and with Integer
    // arguments, must produce bit-identical results to the interpreter.
    assert_tier_transparent(
        r#"
fn probe(x) = math.sqrt(x) + math.cbrt(x) + math.floor(x) + math.ceil(x)
    + math.round(x) + math.trunc(x) + math.fract(x)
    + math.sin(x) + math.cos(x) + math.tan(x)
    + math.asin(x / 10.0) + math.acos(x / 10.0) + math.atan(x)
    + math.sinh(x) + math.cosh(x) + math.tanh(x)
    + math.exp(x) + math.exp2(x) + math.ln(x) + math.log2(x) + math.log10(x)
    + math.to_degrees(x) + math.to_radians(x)
    + math.pow(x, 2.5) + math.atan2(x, 3.0)
fn probe_int(n) = math.sqrt(n) + math.pow(n, 2) + math.atan2(n, 2)
let mut s = 0.0
for i in 0..30 { s = s + probe(to_float(i) * 0.17 + 1.1) + probe_int(i + 1) }
s
"#,
    );
}

#[test]
fn a_local_shadowing_a_module_name_is_field_access_not_a_builtin() {
    // `math.sqrt` where `math` is a *local* must read the field, not call the
    // builtin — the compiler distinguishes a bare module identifier from a
    // slot-resolved local. Both tiers must agree.
    assert_tier_transparent(
        r#"
type M = struct { sqrt: Int }
fn use_it(math) = math.sqrt + 1
let mut n = 0
for i in 0..30 { n = n + use_it(M { sqrt: 41 }) }
n
"#,
    );
}

// ── native higher-order builtins (map / filter / sum in the VM) ───────

#[test]
fn native_map_filter_sum_agree_with_the_interpreter() {
    let src = r#"
let scale = 7
fn go(xs) = {
    let a = xs |> map((x) => x + 1) |> sum
    let b = xs |> filter((x) => x > 25) |> sum
    let c = xs |> map((x) => x * scale) |> sum
    let d = xs |> map((x) => x * 0.5) |> sum
    a + b + c + d
}
let xs = range(0, 60)
let mut t = 0.0
for i in 0..10 { t = t + go(xs) }
t
"#;
    assert!(promotion_count(src, 2) >= 1, "go must be promoted");
    assert_tier_transparent(src);
}

#[test]
fn native_filter_keeps_only_exact_boolean_true() {
    // The interpreter keeps an element only when the predicate returns
    // exactly Boolean(true); any other value silently drops it. A native
    // filter that used truthiness instead would keep every nonzero int.
    let src = r#"
fn go(xs) = xs |> filter((x) => x) |> len
let xs = range(1, 50)
let mut t = 0
for i in 0..10 { t = t + go(xs) }
t
"#;
    assert!(promotion_count(src, 2) >= 1, "go must be promoted");
    assert_tier_transparent(src);
}

#[test]
fn native_map_over_strings_and_nested_maps_agree() {
    let src = r#"
fn go(xs) = {
    let tagged = xs |> map((s) => s + "!")
    let lens = tagged |> map((s) => len(s)) |> sum
    lens + len(tagged)
}
let xs = ["a", "bc", "def"]
let mut t = 0
for i in 0..10 { t = t + go(xs) }
t
"#;
    assert!(promotion_count(src, 2) >= 1, "go must be promoted");
    assert_tier_transparent(src);
}

#[test]
fn native_sum_edge_cases_agree() {
    // Empty list, int→float promotion mid-list, and both error paths
    // (integer overflow, non-numeric element) must match the interpreter.
    let src = r#"
fn go() = {
    let empty = [] |> sum
    let mixed = [1, 2, 2.5, 3] |> sum
    empty + mixed
}
let mut t = 0.0
for i in 0..10 { t = t + go() }
t
"#;
    assert!(promotion_count(src, 2) >= 1, "go must be promoted");
    assert_tier_transparent(src);

    assert_tier_transparent(
        r#"
fn overflowing() = [9223372036854775807, 1] |> sum
overflowing()
overflowing()
overflowing()
"#,
    );
    assert_tier_transparent(
        r#"
fn bad() = [1, "two", 3] |> sum
bad()
bad()
bad()
"#,
    );
}

#[test]
fn native_map_propagates_element_errors_like_the_interpreter() {
    // The 26th element divides by zero: both tiers must fail the whole
    // call, and calls before it must have completed identically.
    assert_tier_transparent(
        r#"
fn go(xs) = xs |> map((x) => 100 / (25 - x)) |> len
let xs = range(0, 50)
go([1, 2, 3])
go([1, 2, 3])
go(xs)
"#,
    );
}

// ── unary operators compile ───────────────────────────────────────────

#[test]
fn unary_negation_and_not_promote_and_agree() {
    // Until these compiled, a single `-x` anywhere in a function refused
    // the whole function and left it on the interpreter.
    let src = r#"
fn mix(n, f, b) = {
    let a = -n
    let c = -f
    let d = !b
    let e = --n
    let g = -(n * 2) + (0 - n)
    if d => a + e + g + c
    else => a - e + g - c
}
let mut s = 0.0
for i in 0..40 { s = s + mix(i, 1.5, i > 20) }
s
"#;
    assert_eq!(promotion_count(src, 2), 1, "mix should be promoted");
    assert_tier_transparent(src);
}

#[test]
fn negation_overflow_and_type_errors_match_the_interpreter() {
    // checked_neg on i64::MIN, and negating a non-number, must fail the
    // same way in both tiers rather than diverging.
    assert_tier_transparent(
        r#"
fn neg(n) = -n
neg(1)
neg(2)
neg(-9223372036854775807 - 1)
"#,
    );
    assert_tier_transparent(
        r#"
fn neg(n) = -n
neg(1)
neg(2)
neg("nope")
"#,
    );
}

// ── register slot reuse (perf: skipping drop glue) ────────────────────

#[test]
fn registers_alternating_between_heap_and_immediate_values_stay_correct() {
    // Writing a register skips drop glue when the value being overwritten
    // owns no heap payload. A register that alternates between strings,
    // lists and numbers exercises both branches of that decision on every
    // pass: a mistake either drops a live payload or keeps a dead one.
    // (Deliberately free of `show`, which the tier refuses to compile — a
    // rejected function would leave this test exercising the interpreter
    // twice and proving nothing.)
    assert_tier_transparent(
        r#"
fn churn(n) = {
    let mut slot = "start"
    let mut acc = 0
    for i in 0..n {
        slot = "s" + "tring"
        acc = acc + len(slot)
        slot = i
        acc = acc + slot
        slot = [i, i + 1, i + 2]
        acc = acc + len(slot) + slot[2]
        slot = 0.5
        acc = acc + 1
    }
    acc
}
churn(60) + churn(60) + churn(60)
"#,
    );
}

#[test]
fn heap_values_survive_being_passed_through_reused_frames() {
    // Frames are pooled and their register vectors reset in place rather
    // than cleared. A string built in one call must not be observable in
    // the next call that reuses the frame, and must come back intact.
    assert_tier_transparent(
        r#"
fn tag(prefix, n) = {
    let mut out = prefix
    for i in 0..n { out = out + "-ab" }
    len(out)
}
let mut all = 0
for k in 0..150 { all = all + tag("r", 3) }
all + tag("zz", 9)
"#,
    );
}

// ── argument-conversion cache (perf: the tier boundary) ───────────────

#[test]
fn repeated_calls_with_the_same_list_stay_correct() {
    // The tier caches Value->OvmValue conversion by allocation identity.
    // Hammering one list through a promoted function must keep exact results.
    assert_tier_transparent(
        r#"
fn first(xs) = xs[0]
fn total(xs) = {
    let mut s = 0
    for i in 0..500 { s = s + first(xs) }
    s
}
let xs = [7, 8, 9]
total(xs) + total(xs)
"#,
    );
}

#[test]
fn fresh_lists_never_see_stale_cached_conversions() {
    // Lists created and dropped in a loop can reuse allocations; a stale
    // cache hit would return a previous list's contents. Each iteration's
    // result must reflect ITS list.
    assert_tier_transparent(
        r#"
fn head_of(xs) = xs[0]
let mut total = 0
for i in 0..300 {
    let fresh = [i * 3, 99]
    total = total + head_of(fresh)
}
total
"#,
    );
}

#[test]
fn mutated_derivatives_are_distinct_allocations() {
    // map/filter build new lists; passing originals and derivatives
    // alternately must never cross wires.
    assert_tier_transparent(
        r#"
fn sum_all(xs) = {
    let mut s = 0
    for x in xs { s = s + x }
    s
}
let base = [1, 2, 3, 4]
let doubled = base |> map((x) => x * 2)
let mut acc = 0
for i in 0..100 { acc = acc + sum_all(base) + sum_all(doubled) }
acc
"#,
    );
}

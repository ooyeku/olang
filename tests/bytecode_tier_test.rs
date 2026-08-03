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
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(55 + 5050 + 500500));
    assert_tier_transparent(src);
}

#[test]
fn functions_using_globals_keep_working() {
    // Not compilable (references a global), must stay interpreted and correct
    let src = r#"
let base = 100
fn offset(n) = n + base
offset(1) + offset(2) + offset(3)
"#;
    assert_eq!(promotion_count(src, 1), 0);
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(306));
    assert_tier_transparent(src);
}

#[test]
fn functions_using_unsupported_features_keep_working() {
    let src = r#"
fn classify(n) = match n % 3 {
    0 => "zero",
    1 => "one",
    _ => "two"
}
classify(1) + classify(2) + classify(3)
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
    // `describe` references a global, so neither it nor its caller may be
    // promoted — but the program must still produce the right answer.
    let src = r#"
let prefix = "n="
fn describe(n) = prefix + to_string(n)
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

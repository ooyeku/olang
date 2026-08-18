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

/// Run a program with the tier on and report how many calls executed as
/// bytecode (each tier entry from the interpreter).
fn bytecode_calls(source: &str, threshold: u32) -> u64 {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).unwrap();
        let mut interpreter = Interpreter::new();
        interpreter.enable_bytecode_tier(threshold, false);
        interpreter.eval_program(program).unwrap();
        interpreter.bytecode_tier_stats().unwrap().bytecode_calls
    })
}

#[test]
fn empty_collection_truthiness_agrees_across_tiers() {
    // An empty string/list/tuple/range is falsy, exactly like 0/false/Unit.
    // The bytecode tier's condition test used to treat empty collections as
    // truthy (only Unit and zero were falsy), so a promoted `if xs => …`
    // took the wrong branch for an empty `xs` — a tier disagreement.
    let src = r#"
fn t(c) = if c => 1 else => 0
fn w(c) = { let mut n = 0
    let mut c = c
    while c { n = n + 1; c = false }
    n }
[ t(""), t([]), t("x"), t([1]), t(0), t(5), w([]), w([9]) ]
"#;
    assert_tier_transparent(src);
    assert_eq!(
        eval(src, Some(2)).unwrap(),
        Value::List(
            vec![
                Value::Integer(0), // t("")  — empty string is falsy
                Value::Integer(0), // t([])  — empty list is falsy
                Value::Integer(1), // t("x")
                Value::Integer(1), // t([1])
                Value::Integer(0), // t(0)
                Value::Integer(1), // t(5)
                Value::Integer(0), // w([])  — loop body never runs
                Value::Integer(1), // w([9]) — runs once
            ]
            .into()
        )
    );
}

#[test]
fn collection_equality_agrees_across_tiers() {
    // The bytecode VM had no ==/!= arms for top-level List, Tuple, or Unit
    // operands, so `f(a, b) = a == b` worked interpreted but raised
    // "Unsupported operation: Equal" once promoted. All three now compare
    // structurally, exactly like the interpreter's Value equality — including
    // its asymmetry: 1 == 1.0 is true but [1] == [1.0] is false.
    let src = r#"
fn eq(a, b) = a == b
fn ne(a, b) = a != b
fn unit(x) = { let n = 0
    while n > 0 { n } }
[ eq([1, 2], [1, 2]), eq([1, 2], [1, 3]), eq([], []),
  ne([1], [2]), eq([1], [1.0]),
  eq((1, 2), (1, 2)), ne((1, 2), (1, 3)),
  eq([(1, 2)], [(1, 2)]),
  eq(unit(1), unit(2)), ne(unit(1), unit(2)) ]
"#;
    assert_tier_transparent(src);
    assert_eq!(
        eval(src, Some(2)).unwrap(),
        Value::List(
            vec![
                Value::Boolean(true),  // [1,2] == [1,2]
                Value::Boolean(false), // [1,2] == [1,3]
                Value::Boolean(true),  // [] == []
                Value::Boolean(true),  // [1] != [2]
                Value::Boolean(false), // [1] == [1.0] — structural, not coercing
                Value::Boolean(true),  // (1,2) == (1,2)
                Value::Boolean(true),  // (1,2) != (1,3)
                Value::Boolean(true),  // [(1,2)] == [(1,2)]
                Value::Boolean(true),  // Unit == Unit
                Value::Boolean(false), // Unit != Unit
            ]
            .into()
        )
    );
}

#[test]
fn capturing_lambda_does_not_inherit_enclosing_annotations() {
    // The bytecode compiler stamped the enclosing function's param_checks
    // onto every lambda it queued, applied positionally to
    // [own params..., captures...] — so a lambda capturing an annotated
    // String parameter of `fn f(xs, id: Int, cs: String)` failed with
    // "parameter 'cs' of <lambda> expects Int, got String" on the tier
    // while the interpreter ran fine. Lambdas now carry their own checks.
    let src = r#"
fn f(xs, id: Int, cs: String) = map(xs, (b) => cs + show(b + id))
f([1, 2], 3, "S")
"#;
    assert_tier_transparent(src);
    assert_eq!(
        eval(src, Some(1)).unwrap(),
        Value::List(
            vec![
                Value::String("S4".to_string().into()),
                Value::String("S5".to_string().into()),
            ]
            .into()
        )
    );
    // A lambda's OWN annotations still bite, with the same message on
    // both tiers.
    let bad = "let g = (n: Int) => n + 1\ng(\"x\")";
    let interpreted = eval(bad, None).unwrap_err();
    let promoted = eval(bad, Some(1)).unwrap_err();
    assert_eq!(interpreted, promoted);
}

#[test]
fn unsupported_binary_op_error_text_matches_interpreter() {
    // Same failure, same words: a promoted function raising a binary-op type
    // error must produce the interpreter's message ("Invalid binary
    // operation: cannot apply '<' to List and List"), not the VM-internal
    // "Unsupported operation: LessThan".
    for src in [
        "fn f(a, b) = a < b\nf([1, 2], [1, 3])",
        "fn f(a, b) = a - b\nf(\"a\", \"b\")",
        "fn f(a, b) = a && b\nf(1, 2)",
        "fn f(a, b) = a || b\nf(1.0, 2)",
        "fn f(a, b) = a < b\nf(false, true)",
    ] {
        let interpreted = eval(src, None).unwrap_err();
        let promoted = eval(src, Some(2)).unwrap_err();
        assert_eq!(
            interpreted, promoted,
            "error text diverged across tiers for: {src}"
        );
    }
}

#[test]
fn function_colliding_with_an_embedded_package_helper_still_promotes() {
    // `viz` defines a PRIVATE `col`. A user `col` of the same name used to
    // make the tier mark the name ambiguous and refuse to tier it — so the
    // user's function, and its hot map loop, silently ran on the interpreter
    // (viz finding #3: this is why viz/dash-using programs booted slowly).
    // It now dispatches by body identity and promotes normally.
    let src = r#"
use viz
fn col(recs, k) = map(recs, (r) => map_get(r, k))
fn build(n) = map(range(0, n), (i) => #{ "x": i })
let recs = build(1000)
let mut total = 0
for j in range(0, 200) { total = total + len(col(recs, "x")) }
total
"#;
    // The name collides, but the result is the interpreter's on both tiers.
    assert_tier_transparent(src);
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(200 * 1000));
    // And `col` actually promotes: ~198 of its 200 calls run as bytecode.
    // With the ambiguity bug it dropped to the interpreter and this was ~1.
    let calls = bytecode_calls(src, 2);
    assert!(
        calls > 50,
        "a name-colliding function must still tier (bytecode_calls={calls}, expected ~198)"
    );
}

#[test]
fn loop_heavy_function_is_transparent() {
    let src = r#"
fn sum_to(n) = {
    let mut total = 0
    let mut i = 0
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
let mut base = 100
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
let mut xs = [10, 20, 30]
let before = go(xs) + go(xs)
fn helper(x) = x + 1000
before + go(xs)
"#;
    assert_eq!(eval(src, None).unwrap(), Value::Integer(63 * 3));
    assert_tier_transparent(src);
}

#[test]
fn map_literals_and_builtins_now_promote() {
    // (This used to assert map literals were rejected.) Maps are
    // first-class in the VM: literals compile to MakeMap with the
    // interpreter's key-coercion rules, and the map builtins bridge
    // losslessly.
    let src = r#"
fn lookup(k) = {
    let m = #{"a": 1, "b": 2}
    map_get(m, k)
}
to_string(lookup("a")) + to_string(lookup("b"))
"#;
    assert_eq!(promotion_count(src, 1), 1, "lookup should now compile");
    assert_tier_transparent(src);
}

#[test]
fn maps_promote_and_agree_across_builders_and_readers() {
    let src = r#"
fn build(n) = {
    let mut m = #{"base": n, 3: 30, true: 100}
    for i in 0..5 { m = map_set(m, "k" + show(i), i * n) }
    m
}
fn totals(n) = {
    let m = build(n)
    let mut t = map_len(m) + map_get(m, "3") + map_get(m, true)
    for kv in entries(m) { t = t + kv[1] }
    t + map_len(map_merge(m, #{"extra": 1}))
}
let mut acc = 0
for i in 0..20 { acc = acc + totals(i) }
acc + len(map_keys(build(2)))
"#;
    assert!(
        promotion_count(src, 2) >= 2,
        "build and totals should promote"
    );
    assert_tier_transparent(src);
}

#[test]
fn maps_cross_the_boundary_and_compare_structurally() {
    // A map built in compiled code returns to the interpreter as a real
    // Map (it used to come back as a struct), and == compares contents.
    let src = r#"
fn make(n) = #{"x": n, "y": n * 2}
make(1)
make(2)
let mut a = make(5)
let b = #{"y": 10, "x": 5}
if a == b => 1 else => 0
"#;
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(1));
    assert_tier_transparent(src);
}

#[test]
fn invalid_map_keys_error_identically() {
    assert_tier_transparent(
        r#"
fn bad(n) = #{[1, 2]: n}
bad(1)
bad(2)
bad(3)
"#,
    );
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
    // `bump` spawns, which the tier does not compile — a task needs the
    // interpreter's own environment to clone — so neither it nor its
    // caller may be promoted, yet the program must still produce the right
    // answer. (This test has burned through four previous "uncompilable"
    // features: a global read, a map literal, assignment to a global which
    // 0.62 made illegal, and `try`/`catch` which 0.65 removed. `spawn` is
    // the durable choice: compiling it would mean compiling the thread
    // boundary. If it ever does compile, pick another construct rather
    // than deleting the test — what it guards is that an uncompilable
    // *callee* keeps its caller interpreted.)
    let src = r#"
fn inner(n) = n
fn bump(n) = task.join(spawn inner(n))
fn label(n) = bump(n) + 1
label(1) + label(2)
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
    let mut total = 0
    let mut i = 0
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
    let mut xs = range(0, n)
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
    let mut acc = 0
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
    let mut acc = 0
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
    let mut acc = 0
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
    let mut acc = 0
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
    let mut found = 0
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
    let mut acc = 0
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
    let mut acc = 0
    let mut i = 0
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
    let mut acc = 0
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
    let mut acc = 0
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
    let mut total = 0
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
fn struct_patterns_now_compile() {
    // (This used to assert struct patterns were rejected.)
    let src = r#"
type User = struct { name: String, age: Int }
fn label(u) = match u {
    User { name, age } => name
}
label(User { name: "ann", age: 30 })
label(User { name: "bob", age: 40 })
"#;
    assert_eq!(promotion_count(src, 1), 1, "label should now compile");
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
    let mut total = 0
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
fn capturing_lambda_now_compiles_with_runtime_captures() {
    // The lambda captures `factor`, a runtime parameter. MakeClosure
    // snapshots the register at the lambda expression — the interpreter's
    // capture-by-value moment — and the body runs compiled with the capture
    // as a hidden trailing parameter. Two different factors in one program
    // prove the captures are per-closure, not baked.
    let src = r#"
fn scale(xs, factor) = map(xs, (x) => x * factor)
sum(scale([1, 2, 3], 10)) + sum(scale([1, 2], 5))
"#;
    assert_eq!(promotion_count(src, 1), 1, "scale should now compile");
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(60 + 15));
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
let mut factor = 7
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
let mut factor = 7
fn scale(xs) = map(xs, (x) => x * factor)
factor = 100
sum(scale([1, 2, 3]))
"#;
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(42));
    assert_tier_transparent(src);
}

#[test]
fn lambda_capturing_enclosing_param_compiles() {
    let src = r#"
fn scale(xs, k) = map(xs, (x) => x * k)
sum(scale([1, 2, 3], 10)) + sum(scale([1, 2], 5))
"#;
    assert_eq!(promotion_count(src, 1), 1);
    assert_tier_transparent(src);
}

#[test]
fn lambda_capturing_enclosing_local_compiles() {
    let src = r#"
fn work(xs) = {
    let offset = 3
    map(xs, (x) => x + offset)
}
sum(work([1, 2, 3]))
"#;
    assert_eq!(promotion_count(src, 1), 1);
    // work runs once, so compare under the threshold where it promotes
    assert_eq!(eval(src, Some(1)).unwrap(), eval(src, None).unwrap());
}

#[test]
#[allow(clippy::identity_op)] // the sum below mirrors the loop structure per iteration
fn captures_snapshot_at_lambda_creation_not_at_call() {
    // The local is reassigned after the lambda is created; the closure must
    // hold the creation-time value in both tiers. Also: a lambda created in
    // a loop captures each iteration's value, not the last one.
    let src = r#"
fn snap(xs) = {
    let mut k = 10
    let f = (x) => x * k
    k = 99
    map(xs, f) |> sum
}
fn per_iteration(xs) = {
    let mut fs_total = 0
    for i in 0..3 {
        let f = (x) => x + i
        fs_total = fs_total + (map(xs, f) |> sum)
    }
    fs_total
}
snap([1, 2, 3]) + per_iteration([10, 20])
"#;
    assert!(promotion_count(src, 1) >= 1);
    let expected = Value::Integer(60 + (30 + 0 + 0) + (30 + 1 + 1) + (30 + 2 + 2));
    assert_eq!(eval(src, None).unwrap(), expected);
    // Compare under threshold 1, where promotion actually happens — the
    // functions here run once each, so the threshold-2 comparison is inert.
    assert_eq!(eval(src, Some(1)).unwrap(), expected);
}

#[test]
fn escaping_closures_convert_back_to_interpreter_functions() {
    // A compiled function RETURNS the closure; the caller (interpreted, at
    // top level) invokes it. The closure crosses the tier boundary as a
    // value and must behave as the interpreter-built one.
    let src = r#"
fn adder(n) = (x) => x + n
let add5 = adder(5)
let add9 = adder(9)
add5(1) + add9(1) + sum(map([1, 2, 3], add5))
"#;
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(6 + 10 + 21));
    assert_tier_transparent(src);
}

#[test]
fn capturing_lambda_through_bridged_builtins_agrees() {
    // fold is not on the native path; the closure converts to an AST
    // function at the bridge and runs interpreted. Same answer required.
    let src = r#"
fn total(xs, k) = fold(xs, 0, (acc, x) => acc + x * k)
total([1, 2, 3], 10) + total([1, 2], 5)
"#;
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(60 + 15));
    assert_tier_transparent(src);
}

#[test]
fn a_lambda_capturing_a_reassigned_local_agrees() {
    // This used to assign a *global* from inside `f` and assert the tier
    // refused to promote it. 0.62 made that construct illegal outright
    // (the write could only reach the closure's snapshot), so what remains
    // to test is the legal shape: a local reassigned before the lambda is
    // built. The lambda must capture the value at creation — 5, not 1 —
    // on both tiers.
    let src = r#"
fn f(xs) = {
    let mut acc = 1
    acc = 5
    map(xs, (x) => x * acc)
}
sum(f([1, 2])) + sum(f([3]))
"#;
    assert_eq!(eval(src, None).unwrap(), Value::Integer(30));
    assert_tier_transparent(src);
}

#[test]
fn a_cell_written_from_a_promoted_function_stays_one_location() {
    // The modern replacement for the assign-a-global shape. Unlike that
    // one, this compiles: a cell handle crosses the tier boundary as a
    // shared `Arc`, so the promoted function writes the same location the
    // interpreter did on its earlier calls.
    let src = r#"
let store = cell(0)
fn bump(n) = {
    cell.set(store, cell.get(store) + n)
    cell.get(store)
}
fn label(n) = bump(n) + 1
label(1) + label(2) + cell.get(store)
"#;
    // 2 + 4 + 3
    assert_eq!(eval(src, None).unwrap(), Value::Integer(9));
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
fn recursive_reference_inside_lambda_now_compiles() {
    // `f` is not in its own declaration-time closure, but it IS a
    // registered function: the compiled lambda calls it through the
    // registry, and the lambda's AST form carries the function value so an
    // escaped copy still resolves it interpreted. (This used to reject.)
    let src = r#"
fn f(n) = if n <= 0 => 0 else => sum(map([1], (x) => f(n - 1)))
f(3)
f(3)
f(3)
"#;
    assert!(promotion_count(src, 2) >= 1, "f should now compile");
    assert_tier_transparent(src);
}

#[test]
fn mutual_recursion_through_lambdas_promotes_and_agrees() {
    // The template-example shape: render_node's lambda calls render, which
    // is declared LATER and calls back into render_node. The dependency
    // channel registers the forward reference and retries.
    let src = r#"
fn render_node(nodes) = nodes |> map((n) => render(n)) |> sum
fn render(n) = if n > 3 => n * 2 else => render_node([n + 1, n + 2])
let mut t = 0
for i in 0..12 { t = t + render(i) }
t
"#;
    assert!(promotion_count(src, 2) >= 2);
    assert_tier_transparent(src);
}

#[test]
fn escaped_lambdas_carrying_registry_functions_resolve_interpreted() {
    // The exact template bug: a compiled lambda referencing a registry
    // function is handed to a BRIDGED builtin (fold is not native), so it
    // runs interpreted — the carried function value must resolve.
    let src = r#"
fn double(x) = x * 2
fn go(xs) = fold(xs, 0, (acc, x) => acc + double(x))
go([1, 2, 3])
go([1, 2, 3])
go([1, 2, 3])
"#;
    assert!(promotion_count(src, 2) >= 1);
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(12));
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

#[test]
fn type_correct_struct_constructs_across_tiers() {
    // Every checkable field kind, built by a hot function, must construct and
    // agree on the interpreter, the bytecode tier, and the JIT.
    assert_tier_transparent(
        r#"
type Rec = struct { n: Int, f: Float, s: String, b: Bool }
fn build(i) = Rec { n: i, f: 1.5, s: "x", b: true }
let mut n = 0
for i in 0..200 { n = n + build(i).n }
n
"#,
    );
}

#[test]
fn mismatched_field_type_errors_across_tiers() {
    // A struct built with a value whose runtime type contradicts the declared
    // field type is a type error on BOTH tiers. `build` is warmed up with
    // matching Int values (so it promotes to the bytecode tier), then handed a
    // String — the error must come from the promoted MakeStruct exactly as it
    // does from the interpreter.
    let src = r#"
type P = struct { x: Int }
fn build(v) = P { x: v }
let mut n = 0
for i in 0..100 { n = n + build(i).x }
build("boom").x
"#;
    let interpreted = eval(src, None);
    let promoted = eval(src, Some(2));
    assert!(
        interpreted.is_err(),
        "interpreter should reject: {interpreted:?}"
    );
    assert!(
        promoted.is_err(),
        "promoted tier should reject: {promoted:?}"
    );
    // The message content is identical on both paths.
    assert!(
        interpreted
            .unwrap_err()
            .contains("field 'x' of P expects Int, got String"),
        "interpreter message"
    );
    assert!(
        promoted
            .unwrap_err()
            .contains("field 'x' of P expects Int, got String"),
        "promoted message"
    );
}

#[test]
fn int_into_float_field_errors_across_tiers() {
    // Enforcement is strict — no Int->Float widening — and identically so on
    // both tiers.
    let src = r#"
type V = struct { x: Float }
fn build(v) = V { x: v }
let mut n = 0
for i in 0..100 { n = n + build(1.0 * i).x }
build(3).x
"#;
    assert!(eval(src, None).is_err());
    assert!(eval(src, Some(2)).is_err());
}

#[test]
fn untyped_and_generic_fields_unaffected_across_tiers() {
    // Anonymous objects declare no field types, and a generic type parameter
    // has no runtime identity to check — both accept any value, on every tier.
    assert_tier_transparent(
        r#"
type Box<T> = struct { value: T }
fn wrap(i) = Box { value: i }
fn anon(i) = { tag: "n", v: i }
let mut n = 0
for i in 0..120 { n = n + wrap(i).value + anon(i).v }
n
"#,
    );
    // A generic field even accepts a value of a different type than earlier
    // calls used, with no error on either tier.
    let src = r#"
type Box<T> = struct { value: T }
fn wrap(v) = Box { value: v }
let mut n = 0
for i in 0..100 { n = n + wrap(i).value }
to_string(wrap("done").value)
"#;
    assert_eq!(eval(src, None), eval(src, Some(2)));
    assert!(eval(src, Some(2)).is_ok());
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

// ── template strings and nested fn declarations ───────────────────────

#[test]
fn template_strings_promote_and_stringify_identically() {
    // Interpolation rules are the interpreter's: String raw, Int/Float/
    // Bool via to_string, everything else through Value's Display —
    // structs, enums, lists included.
    let src = r#"
type P = struct { x: Int }
type E = enum { A(Int), B }
fn tpl(n) = {
    let p = P { x: n }
    `n=${n} f=${n * 0.5} s=${"txt"} b=${n > 2} p=${p} l=${[1, n]} e=${A(n)} u=${B}`
}
let mut out = ""
for i in 0..20 { out = out + tpl(i) }
len(out)
"#;
    assert!(promotion_count(src, 2) >= 1, "tpl should promote");
    assert_tier_transparent(src);
}

#[test]
fn nested_fn_declarations_promote_as_named_closures() {
    // A nested fn captures the enclosing frame like a lambda; a sibling
    // nested fn is a local function value by the time it is called.
    let src = r#"
fn outer(n) = {
    fn helper(x) = x * 2
    fn scaled(x) = helper(x) + n
    scaled(n) + helper(3)
}
let mut t = 0
for i in 0..20 { t = t + outer(i) }
t
"#;
    assert!(promotion_count(src, 2) >= 1, "outer should promote");
    assert_tier_transparent(src);
}

#[test]
fn recursive_nested_fns_now_compile() {
    // A nested fn referencing itself binds its own name to its own
    // compiled id (recursion is CallFn), and the escaped form carries the
    // name so interpreted copies recurse too. (This used to refuse.)
    let src = r#"
fn outer(n) = {
    fn count(x) = if x <= 0 => 0 else => 1 + count(x - 1)
    count(n)
}
outer(5) + outer(6) + outer(7)
"#;
    assert!(promotion_count(src, 2) >= 1, "outer should promote");
    assert_tier_transparent(src);
}

#[test]
fn recursive_nested_fns_with_captures_keep_them_through_recursion() {
    // The 03_algorithms bug: a nested fn capturing enclosing state whose
    // body recurses. Self-calls must append the capture parameters, or
    // recursion arrives with the wrong arity (2 args into a 4-param body).
    let src = r#"
fn binary_search(xs, target) = {
    fn go(lo, hi) = {
        if lo >= hi => 0 - 1
        else => {
            let mid = (lo + hi) / 2
            if xs[mid] == target => mid
            else if xs[mid] < target => go(mid + 1, hi)
            else => go(lo, mid)
        }
    }
    go(0, len(xs))
}
let mut xs = [1, 3, 5, 7, 9, 11]
binary_search(xs, 7) * 100 + binary_search(xs, 4) + binary_search(xs, 11)
"#;
    assert!(promotion_count(src, 2) >= 1, "binary_search should promote");
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(300 - 1 + 5));
    assert_tier_transparent(src);
}

#[test]
fn escaped_recursive_nested_fns_recurse_interpreted() {
    // The nested fn ESCAPES (returned, then handed to a bridged builtin):
    // the carried name makes interpreted self-recursion work.
    let src = r#"
fn make_counter() = {
    fn count(x) = if x <= 0 => 0 else => 1 + count(x - 1)
    count
}
fn go(xs) = {
    let c = make_counter()
    fold(xs, 0, (acc, x) => acc + c(x))
}
go([1, 2, 3])
go([1, 2, 3])
go([1, 2, 3])
"#;
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(6));
    assert_tier_transparent(src);
}

#[test]
fn tuple_expressions_and_let_destructuring_promote() {
    let src = r#"
fn pairs(n) = {
    let mut t = (n, n * 2, "x")
    let (a, b, s) = t
    a + b + len(s)
}
let mut acc = 0
for i in 0..20 { acc = acc + pairs(i) }
acc
"#;
    assert!(promotion_count(src, 2) >= 1, "pairs should promote");
    assert_tier_transparent(src);
}

#[test]
fn let_destructuring_mismatch_errors_identically() {
    assert_tier_transparent(
        r#"
fn bad(n) = {
    let (a, b) = (n, n + 1, n + 2)
    a + b
}
bad(1)
bad(2)
bad(3)
"#,
    );
}

#[test]
fn a_block_ending_in_let_evaluates_to_the_bound_value() {
    // eval_let_decl returns the bound value; a compiled block must too.
    // This was a live divergence in 0.37.0: `fn f() = { let x = 5 }`
    // returned 5 interpreted and Unit compiled.
    let src = r#"
fn f(n) = { let x = n * 5 }
f(1)
f(1)
f(1) + f(2)
"#;
    assert!(promotion_count(src, 2) >= 1);
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(15));
    assert_tier_transparent(src);
}

// ── struct shapes and inline caches ───────────────────────────────────

#[test]
fn polymorphic_field_sites_stay_correct_across_shapes() {
    // One GetField site reading `.x` off three different shapes in
    // rotation: the inline cache thrashes but must never serve a stale
    // index. The shapes deliberately place `x` at different positions in
    // sorted field order.
    let src = r#"
type A = struct { x: Int, z: Int }
type B = struct { a: Int, x: Int }
type C = struct { a: Int, b: Int, x: Int }
fn read_x(v) = v.x
fn go(n) = {
    let mut t = 0
    for i in 0..n {
        t = t + read_x(A { x: 1, z: 9 })
            + read_x(B { a: 9, x: 2 })
            + read_x(C { a: 9, b: 9, x: 3 })
            + read_x({ x: 4 })
    }
    t
}
go(50) + go(50)
"#;
    assert!(promotion_count(src, 2) >= 1);
    assert_eq!(
        eval(src, Some(1)).unwrap(),
        Value::Integer((1 + 2 + 3 + 4) * 100)
    );
    assert_tier_transparent(src);
}

#[test]
fn shaped_structs_round_trip_and_match_after_boundary_crossings() {
    // Shapes are a VM-internal layout: structs must convert back to the
    // interpreter's map form unchanged, and struct patterns must keep
    // matching on both sides of the boundary.
    let src = r#"
type P = struct { y: Int, x: Int }
fn make(n) = P { y: n + 1, x: n }
fn read(p) = match p { P { x, y } => x * 100 + y }
let p = make(4)
read(p) + read(make(7))
read(p) + read(make(7))
"#;
    assert_tier_transparent(src);
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(405 + 708));
}

// ── the register slab (frame windows) ─────────────────────────────────

#[test]
fn deep_recursion_carries_heap_values_across_frame_windows() {
    // Frames are windows on one shared slab. A window bug (overlap, stale
    // base, missed reset) scrambles values BETWEEN frames — so recurse
    // deep carrying strings and lists built per frame, and check both the
    // result and that each frame's values survived its callees.
    let src = r#"
fn weave(n, tag) = {
    if n <= 0 => len(tag)
    else => {
        let mine = tag + "-" + "x"
        let below = weave(n - 1, mine)
        let list = [n, below, len(mine)]
        list[0] + list[1] + list[2] - len(mine)
    }
}
weave(60, "seed") + weave(60, "seed")
"#;
    assert!(promotion_count(src, 2) >= 1, "weave should promote");
    assert_tier_transparent(src);
}

#[test]
fn interleaved_native_loops_and_recursion_agree() {
    // A native map loop pushes a frame per element while the enclosing
    // compiled function's window stays live below it; recursion inside the
    // mapped function stacks further windows on top.
    let src = r#"
fn fact(n) = if n <= 1 => 1 else => n * fact(n - 1)
fn go(xs, k) = xs |> map((x) => fact(x % 6) + x * k) |> sum
let mut xs = range(0, 200)
let mut t = 0
for i in 0..20 { t = t + go(xs, i) }
t
"#;
    assert!(promotion_count(src, 2) >= 1);
    assert_tier_transparent(src);
}

// ── function-valued callees (CallValue) ───────────────────────────────

#[test]
fn higher_order_user_functions_promote_and_agree() {
    // A function passed as an argument crosses the boundary (functions
    // round-trip now) and is called through CallValue inside the VM.
    let src = r#"
fn apply(f, x) = f(x)
fn double(n) = n * 2
fn go(n) = apply(double, n) + apply((x) => x + 1, n)
let mut t = 0
for i in 0..30 { t = t + go(i) }
t
"#;
    assert!(promotion_count(src, 2) >= 1, "go should promote");
    assert_tier_transparent(src);
}

#[test]
fn a_parameter_shadowing_a_builtin_is_called_as_the_parameter() {
    // `len` here is the parameter, not the builtin — the interpreter
    // resolves locals first and now so does the compiler. This divergence
    // was latent until function values could cross the boundary.
    let src = r#"
fn apply(len, x) = len(x)
fn double(n) = n * 2
apply(double, 21)
apply(double, 21)
apply(double, 21)
"#;
    assert!(promotion_count(src, 2) >= 1);
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(42));
    assert_tier_transparent(src);
}

#[test]
fn curried_calls_and_returned_closures_agree() {
    let src = r#"
fn curry_add(a) = (b) => a + b
fn go(n) = curry_add(n)(10) + curry_add(3)(n)
let mut t = 0
for i in 0..30 { t = t + go(i) }
t
"#;
    assert!(promotion_count(src, 2) >= 1);
    assert_tier_transparent(src);
}

#[test]
fn calling_a_non_function_value_errors_identically() {
    assert_tier_transparent(
        r#"
fn bad(x) = x(1)
bad(42)
bad(42)
bad(42)
"#,
    );
}

#[test]
fn adt_combinators_promote_and_agree() {
    // The algebraic-types shape that used to reject: an option map/filter
    // built from enum patterns and a function-valued parameter.
    let src = r#"
type Opt = enum { SomeV(Int), NoneV }
fn opt_map(o, f) = match o {
    SomeV(x) => SomeV(f(x)),
    NoneV => NoneV
}
fn unwrap_or_zero(o) = match o { SomeV(x) => x, NoneV => 0 }
fn go(n) = {
    let mut a = opt_map(SomeV(n), (x) => x * 2)
    let b = opt_map(NoneV, (x) => x * 2)
    unwrap_or_zero(a) + unwrap_or_zero(b)
}
let mut t = 0
for i in 0..30 { t = t + go(i) }
t
"#;
    assert!(promotion_count(src, 2) >= 2);
    assert_tier_transparent(src);
}

#[test]
fn trait_method_calls_compile_and_dispatch_on_runtime_type() {
    // CallMethod mirrors the interpreter's dispatch: impl method by the
    // receiver's runtime type, trait defaults (which call back into the
    // receiver's own impl), and primitives as receivers. One call site
    // serves three types.
    let src = r#"
type Point = struct { x: Int, y: Int }
type Circle = struct { r: Float }
trait Describe {
    fn describe(self) -> String
    fn loud(self) -> String = self.describe() + "!"
}
impl Describe for Point {
    fn describe(self) = "point"
}
impl Describe for Circle {
    fn describe(self) = "circle"
}
impl Describe for Int {
    fn describe(self) = "int"
}
fn render(v) = v.describe() + v.loud()
fn go(n) = {
    let p = Point { x: n, y: 2 }
    let c = Circle { r: 0.5 }
    len(render(p)) + len(render(c)) + len(render(n))
}
let mut t = 0
for i in 0..20 { t = t + go(i) }
t
"#;
    assert!(promotion_count(src, 2) >= 2, "render and go should promote");
    assert_eq!(
        eval(src, Some(1)).unwrap(),
        Value::Integer(((5 + 6) + (6 + 7) + (3 + 4)) * 20)
    );
    assert_tier_transparent(src);
}

#[test]
fn struct_fields_take_precedence_over_trait_methods() {
    // A struct FIELD named like a method wins, and is called WITHOUT the
    // receiver as self — the interpreter's precedence rule.
    let src = r#"
type Holder = struct { get: Function, tag: Int }
trait Get {
    fn get(self) -> Int = 999
}
impl Get for Holder {
    fn get(self) = 999
}
fn go(n) = {
    let h = Holder { get: (x) => x * 2, tag: n }
    h.get(21)
}
go(1)
go(2)
go(3)
"#;
    assert_eq!(eval(src, None).unwrap(), Value::Integer(42));
    assert_tier_transparent(src);
}

#[test]
fn missing_methods_error_identically() {
    assert_tier_transparent(
        r#"
type P = struct { x: Int }
fn bad(p) = p.nope()
let p = P { x: 1 }
bad(p)
bad(p)
bad(p)
"#,
    );
    // non-struct receiver with no impl
    assert_tier_transparent(
        r#"
fn bad(n) = n.nope()
bad(1)
bad(2)
bad(3)
"#,
    );
}

#[test]
fn late_impl_declarations_invalidate_compiled_dispatch() {
    // `speak` compiles while only Dog's impl exists; declaring Cat's impl
    // afterwards must invalidate it so the new type dispatches correctly.
    let src = r#"
type Dog = struct { name: String }
type Cat = struct { name: String }
trait Speak { fn speak(self) -> String }
impl Speak for Dog {
    fn speak(self) = "woof"
}
fn hear(a) = a.speak()
let d = Dog { name: "rex" }
let first = hear(d) + hear(d) + hear(d)
impl Speak for Cat {
    fn speak(self) = "meow"
}
let c = Cat { name: "tom" }
first + hear(c)
"#;
    assert_eq!(
        eval(src, Some(1)).unwrap(),
        Value::String("woofwoofwoofmeow".to_string().into())
    );
    assert_tier_transparent(src);
}

// ── str module builtins ───────────────────────────────────────────────

#[test]
fn str_module_calls_promote_and_agree() {
    let src = r#"
fn norm(s) = str.trim(str.to_lower(s)) + str.char_at(s, 0) + show(str.length(s))
fn parse(s) = match str.parse_int(s) { Ok(v) => v, Err(_) => 0 - 1 }
let mut acc = ""
let mut n = 0
for i in 0..30 {
    acc = norm("  MiXeD  ")
    n = n + parse("42") + parse("nope")
}
acc + show(n)
"#;
    assert!(
        promotion_count(src, 2) >= 2,
        "norm and parse should promote"
    );
    assert_tier_transparent(src);
}

// ── enums: construction, unit variants, and patterns ──────────────────

#[test]
fn enum_construction_and_matching_promote_and_agree() {
    let src = r#"
type Shape = enum { Circle(Float), Rect(Float, Float), Empty }
fn area(s) = match s {
    Circle(r) => 3.0 * r * r,
    Rect(w, h) => w * h,
    Empty => 0.0
}
fn total(n) = {
    let mut t = 0.0
    for i in 0..n {
        let f = to_float(i)
        t = t + area(Circle(f)) + area(Rect(f, 2.0)) + area(Empty)
    }
    t
}
total(40) + total(40) + total(40)
"#;
    assert!(
        promotion_count(src, 2) >= 2,
        "area and total should promote"
    );
    assert_tier_transparent(src);
}

#[test]
fn recursive_adt_functions_promote_and_agree() {
    // The algebraic-types shape: a recursive enum consumed by a recursive
    // match — trees of Node/Leaf.
    let src = r#"
type Tree = enum { Leaf(Int), Node(Tree, Tree) }
fn total(t) = match t {
    Leaf(n) => n,
    Node(l, r) => total(l) + total(r)
}
fn build(depth) = {
    if depth <= 0 => Leaf(1)
    else => Node(build(depth - 1), build(depth - 1))
}
let tree = build(10)
let mut acc = 0
for i in 0..20 { acc = acc + total(tree) }
acc
"#;
    assert!(promotion_count(src, 2) >= 1);
    assert_tier_transparent(src);
}

#[test]
fn enum_values_cross_the_tier_boundary_and_compare() {
    // Enums round-trip losslessly now: constructed in compiled code,
    // returned to the interpreter, compared and matched there.
    let src = r#"
type Opt = enum { SomeV(Int), NoneV }
fn wrap(n) = if n > 0 => SomeV(n) else => NoneV
wrap(1)
wrap(2)
let mut a = wrap(5)
let b = wrap(0 - 1)
match a { SomeV(x) => x, NoneV => 0 } + match b { SomeV(x) => x, NoneV => 100 }
"#;
    assert_tier_transparent(src);
    assert_eq!(eval(src, Some(1)).unwrap(), Value::Integer(105));
}

#[test]
fn enum_arity_mismatch_falls_back_and_errors_identically() {
    // Wrong constructor arity refuses compilation; the interpreter raises
    // its arity error in both tiers.
    assert_tier_transparent(
        r#"
type Shape = enum { Circle(Float) }
fn bad(n) = Circle(n, n)
bad(1.0)
bad(2.0)
bad(3.0)
"#,
    );
}

#[test]
fn struct_and_anonymous_patterns_promote_and_agree() {
    let src = r#"
type User = struct { name: String, age: Int }
fn describe(u) = match u {
    User { age, name } if age >= 18 => name + ":adult",
    User { name, age } => name + ":" + to_string(age)
}
fn tag(rec) = match rec {
    { kind, size } => kind + to_string(size),
    _ => "unknown"
}
let mut s = ""
for i in 0..20 {
    s = s + describe(User { name: "a", age: i }) + tag({ kind: "k", size: i })
}
len(s)
"#;
    assert!(promotion_count(src, 2) >= 1);
    assert_tier_transparent(src);
}

#[test]
fn enum_patterns_against_wrong_shapes_agree() {
    // Non-matching variants, plain tuples (the interpreter accepts a tuple
    // of matching length against an enum pattern — legacy behavior), and
    // non-enum values must fall through arms identically.
    let src = r#"
type E = enum { A(Int), B }
fn probe(v) = match v {
    A(x) => x,
    B => 100,
    _ => 999
}
probe(A(7))
probe(B)
probe((42, 43))
probe(A(7)) + probe(B) + probe(7) + probe((1, 2, 3))
"#;
    assert_tier_transparent(src);
}

// ── struct construction compiles ──────────────────────────────────────

#[test]
fn struct_construction_promotes_and_agrees() {
    let src = r#"
type Pair = struct { a: Int, b: Int }
fn shift(p) = Pair { a: p.b, b: p.a + p.b }
let mut p = Pair { a: 0, b: 1 }
for i in 0..30 { p = shift(p) }
p.b
"#;
    assert!(promotion_count(src, 2) >= 1, "shift should be promoted");
    assert_tier_transparent(src);
}

#[test]
fn anonymous_objects_promote_and_agree() {
    let src = r#"
fn tag(x) = { value: x, doubled: x * 2 }
let mut t = 0
for i in 0..30 { t = t + tag(i).doubled + tag(i).value }
t
"#;
    assert!(promotion_count(src, 2) >= 1, "tag should be promoted");
    assert_tier_transparent(src);
}

#[test]
fn struct_literals_in_lambdas_promote() {
    // The nbody shape: a capturing lambda building structs inside map.
    let src = r#"
type P = struct { x: Float, v: Float }
fn advance(ps, dt) = ps |> map((p) => P { x: p.x + p.v * dt, v: p.v })
fn run(ps) = {
    let mut cur = ps
    for i in 0..20 { cur = advance(cur, 0.5) }
    cur
}
let start = [P { x: 0.0, v: 1.0 }, P { x: 2.0, v: -0.5 }]
let done = run(start)
done[0].x + done[1].x
"#;
    assert!(promotion_count(src, 2) >= 1);
    assert_tier_transparent(src);
}

#[test]
fn invalid_struct_literals_error_identically() {
    // Validation failures refuse compilation, so the interpreter raises its
    // own error — success/failure must match in every case.
    for body in [
        "Nope { a: 1 }",             // unknown type
        "Pair { a: 1 }",             // missing field
        "Pair { a: 1, b: 2, c: 3 }", // surprise field
    ] {
        let src = format!(
            r#"
type Pair = struct {{ a: Int, b: Int }}
fn bad(n) = {}
bad(1)
bad(2)
bad(3)
"#,
            body
        );
        assert_tier_transparent(&src);
    }
}

#[test]
fn struct_redeclaration_with_new_shape_falls_back_and_agrees() {
    // `make` compiles against Pair's first shape; redeclaring Pair with a
    // different field set must invalidate that compile — the interpreter's
    // registry is live, so the second `make` call validates against the new
    // shape and errors, and the tier must do exactly the same.
    let src = r#"
type Pair = struct { a: Int, b: Int }
fn make(n) = Pair { a: n, b: n + 1 }
let first = make(1)
make(2)
make(3)
type Pair = struct { a: Int, b: Int, c: Int }
make(4)
"#;
    assert_tier_transparent(src);
}

// ── native collection builtins ────────────────────────────────────────

#[test]
fn native_collection_builtins_agree_across_maps_and_structs() {
    // map_get / map_set / map_has_key / entries work on maps AND
    // struct-likes (the interpreter's field_map duality), with the same
    // key coercion; len/head/tail/cons/concat/skip cover the list family.
    let src = r#"
type P = struct { a: Int, b: Int }
fn go(n) = {
    let m = map_set(#{"x": n, 7: n * 2}, true, n + 1)
    let p = map_set(P { a: n, b: 2 }, "c", n * 3)
    let e = entries(m)
    let l = concat([n], skip([1, 2, 3, 4], 2))
    map_get(m, "x") + map_get(m, 7) + map_get(m, true)
        + map_get(p, "a") + map_get(p, "c")
        + len(e) + len(l) + head(l) + len(tail(l)) + head(cons(n, l))
        + (if map_has_key(m, "x") => 1 else => 0)
        + (if map_has_key(p, "c") => 10 else => 0)
        + (if map_has_key(m, "zz") => 100 else => 0)
}
let mut t = 0
for i in 0..25 { t = t + go(i) }
t
"#;
    assert!(promotion_count(src, 2) >= 1, "go should promote");
    assert_tier_transparent(src);
}

#[test]
fn native_collection_errors_match_the_interpreter() {
    for src in [
        r#"fn f(n) = head([])
f(1)
f(2)
f(3)"#,
        r#"fn f(n) = tail([])
f(1)
f(2)
f(3)"#,
        r#"fn f(n) = len(n)
f(1)
f(2)
f(3)"#,
        r#"fn f(n) = concat([1], n)
f(1)
f(2)
f(3)"#,
        r#"fn f(n) = skip([1, 2], 0 - 1)
f(1)
f(2)
f(3)"#,
        r#"fn f(n) = map_get(n, "k")
f(1)
f(2)
f(3)"#,
        r#"fn f(n) = map_set(#{}, [1], n)
f(1)
f(2)
f(3)"#,
    ] {
        assert_tier_transparent(src);
    }
}

#[test]
fn map_has_key_checks_presence_not_value() {
    // A key explicitly holding Unit still exists: has_key must be true
    // even though map_get returns Unit for it.
    let src = r#"
fn unit_value() = { let _ = 0 }
fn go() = {
    let m = map_set(#{}, "k", println(""))
    if map_has_key(m, "k") => 1 else => 0
}
go()
go()
go()
"#;
    assert_eq!(eval(src, Some(2)).unwrap(), Value::Integer(1));
    assert_tier_transparent(src);
}

// ── native higher-order builtins (map / filter / sum in the VM) ───────

#[test]
fn native_map_filter_sum_agree_with_the_interpreter() {
    let src = r#"
let scale = 7
fn go(xs) = {
    let mut a = xs |> map((x) => x + 1) |> sum
    let b = xs |> filter((x) => x > 25) |> sum
    let c = xs |> map((x) => x * scale) |> sum
    let d = xs |> map((x) => x * 0.5) |> sum
    a + b + c + d
}
let mut xs = range(0, 60)
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
let mut xs = range(1, 50)
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
let mut xs = ["a", "bc", "def"]
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
let mut xs = range(0, 50)
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
    let mut a = -n
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
let mut xs = [7, 8, 9]
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
let mut base = [1, 2, 3, 4]
let doubled = base |> map((x) => x * 2)
let mut acc = 0
for i in 0..100 { acc = acc + sum_all(base) + sum_all(doubled) }
acc
"#,
    );
}

#[test]
fn group_by_preserves_key_type_and_order() {
    // Int keys stay Int (not stringified), first-seen order is stable, and
    // the emitted key compares equal to the classifier's own return value.
    // Both tiers must agree.
    assert_tier_transparent(
        r#"
let g = group_by([1, 2, 3, 4, 5], (x) => x % 2)
let by_letter = group_by(["ax", "by", "az"], (s) => str.substring(s, 0, 1))
to_string(g) + " | " + to_string(by_letter) + " | " + to_string(by_letter[0][0] == "a")
"#,
    );
}

// ── Trait dispatch through the tier: a bare method name resolves to different
// bodies by receiver type. The tier caches compiled functions by name, so
// without a body guard it would replay one type's method for another. These
// assert the two tiers agree even when a single call site sees several types.

#[test]
fn trait_default_and_override_dispatch_through_map() {
    // `desc` is a trait default for A and an override for B. One `map` call
    // site sees both types; the compiled tier must not replay A's default for
    // B. Repeats force compilation, then a differing body at the same name.
    assert_tier_transparent(
        r#"
type A = struct { n: Int }
type B = struct { n: Int }
trait D { fn desc(self) = "default" }
impl D for A { }
impl D for B { fn desc(self) = "override-b" }
let items = [A { n: 1 }, B { n: 2 }, A { n: 3 }, B { n: 4 }, A { n: 5 }]
join(items |> map((i) => i.desc()), ";")
"#,
    );
}

#[test]
fn trait_dispatch_three_types_default_and_overrides() {
    assert_tier_transparent(
        r#"
type A = struct { n: Int }
type B = struct { n: Int }
type C = struct { n: Int }
trait D { fn desc(self) = "def" }
impl D for A { }
impl D for B { fn desc(self) = "b" }
impl D for C { fn desc(self) = "c" }
let items = [A { n: 1 }, B { n: 2 }, C { n: 3 }, A { n: 4 }, C { n: 5 }, B { n: 6 }]
join(items |> map((i) => i.desc()), ";")
"#,
    );
}

#[test]
fn trait_default_and_override_dispatch_direct_calls() {
    // Not through a higher-order builtin: a helper called in a loop over
    // alternating types promotes, and each call resolves the method itself.
    assert_tier_transparent(
        r#"
type A = struct { n: Int }
type B = struct { n: Int }
trait D { fn desc(self) = "default" }
impl D for A { }
impl D for B { fn desc(self) = "override-b" }
fn describe(x) = x.desc()
let mut acc = ""
for i in range(50) {
    acc = describe(A { n: 1 }) + "," + describe(B { n: 2 })
}
acc
"#,
    );
}

#[test]
fn trait_method_in_fold_inside_promoted_function() {
    // A promoted function whose fold-lambda calls a trait method. `fold`
    // bridges to an interpreter that must carry the program's trait tables,
    // or `x.tag()` raises a spurious "Field not found". The loop promotes
    // `total`; mixed types also exercise the by-type resolution.
    assert_tier_transparent(
        r#"
type A = struct { v: Int }
type B = struct { v: Int }
trait T { fn tag(self) = 1 }
impl T for A { }
impl T for B { fn tag(self) = 10 }
fn total(items) = items |> fold(0, (acc, x) => acc + x.tag())
let data = [A { v: 1 }, B { v: 2 }, A { v: 3 }, B { v: 4 }]
let mut acc = 0
for i in range(50) {
    acc = total(data)
}
to_string(acc)
"#,
    );
}

#[test]
fn trait_default_via_fold_default_only() {
    // The exact isolated repro: a type that only takes the trait default,
    // used inside a fold-lambda in a promoted function.
    assert_tier_transparent(
        r#"
type A = struct { v: Int }
trait T { fn tag(self) = 1 }
impl T for A { }
fn total(items) = items |> fold(0, (acc, x) => acc + x.tag())
let data = [A { v: 1 }, A { v: 2 }, A { v: 3 }]
let mut acc = 0
for i in range(50) {
    acc = total(data)
}
to_string(acc)
"#,
    );
}

#[test]
fn trait_method_default_builds_struct_in_fold() {
    // The bridged fold-lambda's trait default constructs a *declared* struct,
    // so the bridge interpreter needs the struct tables too, not just traits.
    assert_tier_transparent(
        r#"
type A = struct { v: Int }
type W = struct { w: Int }
trait T { fn wrap(self) = W { w: 99 } }
impl T for A { }
fn firstw(items) = items |> fold(W { w: 0 }, (acc, x) => x.wrap())
let data = [A { v: 1 }, A { v: 2 }]
let mut acc = 0
for i in range(50) {
    acc = firstw(data).w
}
to_string(acc)
"#,
    );
}

#[test]
fn to_int_rejects_non_finite_floats() {
    // Saturating conversion silently invented a number; now it errors.
    assert_tier_transparent("to_int(1.5)"); // valid still works
    assert_tier_transparent(
        r#"
let bad = to_float("nan")
try to_int(bad) catch e => -1
"#,
    );
}

#[test]
fn sort_rejects_incomparable_mixed_types() {
    assert_tier_transparent("to_string(sort([3, 1, 2]))"); // homogeneous ok
    assert_tier_transparent("to_string(sort([\"b\", \"a\"]))"); // strings ok
    assert_tier_transparent(r#"try to_string(sort([1, "a", 2])) catch e => "rejected""#);
}

#[test]
fn mixed_string_number_add_is_a_type_error_on_all_tiers() {
    // Mixing a number and a string under `+` is a type error (Python-3
    // style), not a silent stringify — and the interpreter and the bytecode
    // tier reject it identically. String+string still concatenates and
    // number+number still adds.
    assert!(
        eval(
            r#"fn f() = "a" + 1
f()"#,
            None
        )
        .is_err(),
        "\"a\" + 1 must error in the interpreter"
    );
    assert!(
        eval(
            r#"fn f() = "a" + 1
f()"#,
            Some(2)
        )
        .is_err(),
        "\"a\" + 1 must error on the bytecode tier"
    );
    assert!(
        eval(
            r#"fn f() = 1 + "a"
f()"#,
            None
        )
        .is_err(),
        "1 + \"a\" must error in the interpreter"
    );
    assert!(
        eval(
            r#"fn f() = 1 + "a"
f()"#,
            Some(2)
        )
        .is_err(),
        "1 + \"a\" must error on the bytecode tier"
    );

    // Both tiers agree (both error) on the mixed cases...
    assert_tier_transparent(
        r#"fn f() = "a" + 1
f()"#,
    );
    assert_tier_transparent(
        r#"fn f() = 1 + "a"
f()"#,
    );
    assert_tier_transparent(
        r#"fn f() = "x" + 0.5
f()"#,
    );

    // ...and still agree (both succeed) on the operations that stay valid.
    assert_eq!(
        eval(r#""a" + "b""#, None).unwrap(),
        Value::String(std::sync::Arc::new("ab".to_string())),
        "string+string still concatenates"
    );
    assert_tier_transparent(r#""a" + "b""#);
    assert_eq!(
        eval("1 + 2", None).unwrap(),
        Value::Integer(3),
        "1 + 2 still adds"
    );
    assert_tier_transparent("1 + 2");

    // A `match` around the type error behaves identically on both tiers:
    // matching destructures a `Value::Err`, it does not trap a hard runtime
    // type error, so the error propagates out on every tier — transparently.
    // (This was written against `try`/`catch`, removed in 0.65 because it
    // did exactly this and nothing more.)
    let handled = r#"fn bad() = "a" + 1
match bad() { Err(e) => "caught", v => v }"#;
    assert!(
        eval(handled, None).is_err(),
        "the type error propagates through the match in the interpreter"
    );
    assert!(
        eval(handled, Some(2)).is_err(),
        "the type error propagates through the match on the bytecode tier"
    );
    assert_tier_transparent(handled);

    // Recovering a `Value::Err` — the shape this is actually for — still
    // works and stays tier-transparent.
    assert_eq!(
        eval(
            r#"fn safe() = Err("boom")
match safe() { Err(e) => "caught: " + e, v => v }"#,
            None
        )
        .unwrap(),
        Value::String(std::sync::Arc::new("caught: boom".to_string())),
        "match still destructures a Value::Err"
    );
    assert_tier_transparent(
        r#"fn safe() = Err("boom")
match safe() { Err(e) => "caught: " + e, v => v }"#,
    );
}

#[test]
fn min_max_average_raise_on_empty_list_like_head() {
    // These returned a Result value (Err "EmptyList") on [] but a bare
    // value otherwise — an inconsistent type that produced a misleading
    // downstream "type mismatch". They now raise, matching head/tail.
    assert_tier_transparent("to_string(min([3, 1, 2]))"); // non-empty still works
    // These raise, so the tier contract is about the error text matching.
    assert_tier_transparent("to_string(min([]))");
    assert_tier_transparent("to_string(max([]))");
    assert_tier_transparent("to_string(average([]))");
}

#[test]
fn error_messages_are_identical_across_tiers() {
    // The tier's contract covers error TEXT, not just values: a failing
    // program must print the same thing whichever tier executed it.
    // These pin the classes that used to diverge (prefix doubling,
    // VM-private wordings, operand order under immediate flipping).
    assert_tier_transparent(
        r#"fn f(x) = match x { 1 => "one" }
f(2)"#,
    );
    assert_tier_transparent("fn f() = 1 + true\nf()");
    assert_tier_transparent("fn f() = true + 1\nf()");
    assert_tier_transparent("fn g(a, b) = a + b\nfn f() = g(1)\nf()");
    assert_tier_transparent("fn g(a) = a\nfn f() = g(1, 2)\nf()");
    assert_tier_transparent("fn f() = len(42)\nf()");
    assert_tier_transparent("fn f() = -\"abc\"\nf()");
}

#[test]
fn annotation_enforcement_is_tier_transparent() {
    // Gradual typing stage 1: annotations are promises on every tier.
    // Param mismatch (checked before the tier), return mismatch (VM
    // Return check; JIT statically discharges or refuses), and the
    // happy paths all behave identically.
    assert_tier_transparent("fn f(x: Int) -> Int = x + 1\nto_string(f(41))");
    assert_tier_transparent("fn f(x: Int) = x\nf(\"nope\")");
    assert_tier_transparent("fn g(x: Int) -> String = x * 2\ng(5)");
    assert_tier_transparent("fn h(xs: List) = len(xs)\nto_string(h([1, 2, 3]))");
    assert_tier_transparent("fn h(xs: List) = len(xs)\nh(42)");
    // Strict Int/Float: no widening.
    assert_tier_transparent("fn f(x: Float) = x\nf(1)");
    // Unannotated stays fully dynamic.
    assert_tier_transparent("fn d(x) = x\nto_string(d(1)) + to_string(d(\"s\"))");
}

#[test]
fn result_payload_enforcement_is_tier_transparent() {
    // Gradual typing stage 4: Result<T, E> checks the present side's
    // payload shallowly, with identical text on every tier.
    // Honest payloads pass.
    assert_tier_transparent(
        "fn p(s: String) -> Result<Int, String> = if s == \"y\" => Ok(1) else => Err(\"no\")\nshow(p(\"y\")) + show(p(\"n\"))",
    );
    // Dishonest Ok payload on return.
    assert_tier_transparent("fn g(x) -> Result<Int, String> = Ok(x)\ng(\"s\")");
    // Dishonest Err payload on return.
    assert_tier_transparent("fn g(x) -> Result<Int, String> = Err(x)\ng(42)");
    // Param boundary, both sides.
    assert_tier_transparent("fn f(r: Result<Int, String>) = r\nshow(f(Ok(\"s\")))");
    assert_tier_transparent("fn f(r: Result<Int, String>) = r\nshow(f(Err(1)))");
    // Payload checks are shallow: a Result payload satisfies by base name.
    assert_tier_transparent("fn f(r: Result<List, String>) = r\nshow(f(Ok([1, 2])))");
    // A non-Result against a Result annotation is the base mismatch.
    assert_tier_transparent("fn f(r: Result<Int, String>) = r\nf(42)");
}

#[test]
fn union_annotation_enforcement_is_tier_transparent() {
    // 0.50 arc: `A | B` accepts any branch, rejects all-branch misses,
    // identically on every tier.
    assert_tier_transparent("fn f(x: Int | String) = show(x)\nf(1) + f(\"s\")");
    assert_tier_transparent("fn f(x: Int | String) = x\nf(true)");
    assert_tier_transparent("fn g(x) -> Int | String = x\nshow(g(1)) + show(g(\"s\"))");
    assert_tier_transparent("fn g(x) -> Int | String = x\ng(1.5)");
    // A Result branch keeps its payload rule inside the union.
    assert_tier_transparent(
        "fn f(x: Result<Int, String> | Int) = show(x)\nshow(f(1)) + show(f(Ok(2)))",
    );
    assert_tier_transparent("fn f(x: Result<Int, String> | Int) = x\nf(Ok(\"bad\"))");
    // Shallow container branches stay shallow.
    assert_tier_transparent("fn f(x: List<Int> | Int) = show(x)\nshow(f([\"any\", 1]))");
}

#[test]
fn function_annotation_enforcement_is_tier_transparent() {
    // 0.50 arc: `(A) -> R` checks callability + arity on every tier.
    assert_tier_transparent(
        "fn apply(f: (Int) -> Int, x: Int) -> Int = f(x)\nto_string(apply((n) => n + 1, 41))",
    );
    assert_tier_transparent("fn apply(f: (Int) -> Int, x: Int) = f(x)\napply(7, 1)");
    assert_tier_transparent("fn apply(f: (Int) -> Int, x: Int) = f(x)\napply((a, b) => a, 1)");
    // A default-parameter lambda satisfies any arity in its range.
    assert_tier_transparent(
        "fn apply(f: (Int) -> Int, x: Int) = f(x)\nto_string(apply((a, b = 1) => a + b, 5))",
    );
    // Promise annotations base-check at non-async sites.
    assert_tier_transparent("fn f(p: Promise<Int>) = p\nf(42)");
}

#[test]
fn literal_type_enforcement_is_tier_transparent() {
    // 0.50: literal annotations compare by value on every tier.
    assert_tier_transparent("fn set(s: \"open\" | \"done\") = s\nset(\"open\") + set(\"done\")");
    assert_tier_transparent("fn set(s: \"open\" | \"done\") = s\nset(\"nope\")");
    assert_tier_transparent("fn pick(n: 1 | 2 | 3) -> Int = n\nto_string(pick(2))");
    assert_tier_transparent("fn pick(n: 1 | 2 | 3) = n\npick(9)");
    assert_tier_transparent("fn strict(b: true) = b\nstrict(false)");
}

// ── runtime error traces ───────────────────────────────────────────────
//
// A runtime error carries the innermost located statement's span and the
// call stack live at that point (ErrorLocation). The interpreter has
// always captured these; the VM now tracks the same trace — statement
// span markers in compiled bytecode, frames recorded during unwind —
// and the tier splices it onto the interpreter's live stack. These
// tests pin the whole ErrorLocation equal across tiers.

fn error_trace(
    source: &str,
    tier_threshold: Option<u32>,
) -> (String, Option<olang::ast::ErrorLocation>) {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).expect("parse");
        let mut interpreter = Interpreter::new();
        if let Some(threshold) = tier_threshold {
            interpreter.enable_bytecode_tier(threshold, false);
        }
        let err = interpreter
            .eval_program(program)
            .expect_err("program should fail");
        (err.to_string(), interpreter.take_error_location())
    })
}

fn assert_trace_transparent(source: &str) {
    let (msg_i, loc_i) = error_trace(source, None);
    let (msg_t, loc_t) = error_trace(source, Some(1));
    assert_eq!(msg_t, msg_i, "tier changed the error message");
    assert_eq!(loc_t, loc_i, "tier changed the error location/stack");
    // The trace must actually exist — a pair of Nones would pass the
    // equality vacuously.
    assert!(loc_i.is_some(), "interpreter produced no error location");
}

#[test]
fn traces_agree_for_nested_block_bodies() {
    assert_trace_transparent(
        r#"
fn inner(n) = {
    let x = n + 1
    x / 0
}
fn middle(n) = inner(n * 2)
fn outer(n) = middle(n + 1)
outer(3)
"#,
    );
}

#[test]
fn traces_agree_for_expression_bodies() {
    assert_trace_transparent(
        r#"
fn inner(n) = n / 0
fn outer(n) = inner(n)
outer(3)
"#,
    );
}

#[test]
fn traces_agree_for_arity_errors() {
    assert_trace_transparent(
        r#"
fn add2(a, b) = a + b
fn caller() = {
    let x = 1
    add2(x)
}
caller()
"#,
    );
}

#[test]
fn traces_agree_for_builtin_errors_in_loops() {
    assert_trace_transparent(
        r#"
fn walk(xs, n) = {
    let mut acc = 0
    let mut i = 0
    while i <= n {
        acc = acc + xs[i]
        i = i + 1
    }
    acc
}
walk([1, 2, 3], 5)
"#,
    );
}

#[test]
fn traces_agree_when_the_error_comes_late() {
    // The function is hot (and possibly jitted) before the failing call:
    // the deopt path must produce the identical trace.
    assert_trace_transparent(
        r#"
fn div(a, b) = {
    let q = a / b
    q
}
fn run() = {
    let mut acc = 0
    let mut i = 10
    while i >= 0 {
        acc = acc + div(100, i)
        i = i - 1
    }
    acc
}
run()
"#,
    );
}

#[test]
fn traces_agree_for_mixed_tier_stacks() {
    // The outer function uses a construct the VM refuses (try/catch is
    // interpreter-only), so the interpreter runs it while the callee
    // tiers — the trace must splice interpreter and VM frames.
    assert_trace_transparent(
        r#"
fn deep(n) = {
    let x = n - 1
    100 / x
}
fn tiered(n) = deep(n)
fn glue(n) = {
    let v = tiered(n)
    v
}
glue(1)
"#,
    );
}

#[test]
fn traces_agree_for_type_annotation_violations() {
    assert_trace_transparent(
        r#"
fn double(n: Int) = n * 2
fn feed() = {
    let x = 1.5
    double(x)
}
feed()
"#,
    );
}

// ── AddAssign fusion: in-place string building ─────────────────────────
//
// `x = x + rhs` fuses into AddAssign, which appends in place when x
// holds the only reference to its string (Arc count proves no aliases;
// strings are immutable values with content equality, so identity is
// unobservable). These tests pin that aliases survive, self-append
// copies, numbers fuse harmlessly, and results agree across tiers.

#[test]
fn string_accumulation_agrees_and_is_linear_shaped() {
    assert_tier_transparent(
        r#"
fn build(n) = {
    let mut s = ""
    let mut i = 0
    while i < n {
        s = s + "ab"
        i = i + 1
    }
    len(s)
}
build(2000)
"#,
    );
}

#[test]
fn aliased_strings_survive_in_place_append() {
    // `snapshot` holds a second reference when the append happens: the
    // fused path must copy, never mutate the shared buffer.
    assert_tier_transparent(
        r#"
fn f() = {
    let mut s = "a"
    let snapshot = s
    s = s + "b"
    snapshot + " " + s
}
f()
"#,
    );
}

#[test]
fn self_append_copies() {
    assert_tier_transparent(
        r#"
fn f() = {
    let mut s = "ab"
    s = s + s
    s = s + s
    s
}
f()
"#,
    );
}

#[test]
fn numeric_and_list_accumulation_fuse_harmlessly() {
    assert_tier_transparent(
        r#"
fn nums(n) = {
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + i * 2
        i = i + 1
    }
    acc
}
fn lists() = {
    let mut xs = [1]
    xs = xs + [2]
    xs = xs + [3]
    xs[0] + xs[1] + xs[2]
}
nums(1000) + lists()
"#,
    );
}

#[test]
fn strings_passed_elsewhere_before_append_are_unharmed() {
    // The accumulated string is stored into a list mid-way; later
    // appends must not mutate the stored copy.
    assert_tier_transparent(
        r#"
fn f() = {
    let mut s = "x"
    s = s + "y"
    let kept = [s]
    s = s + "z"
    kept[0] + " " + s
}
f()
"#,
    );
}

// ── AddAssign fusion: in-place list building ───────────────────────────
//
// The list analog of the string case: `xs = xs + [v]` fuses into
// AddAssign, which extends in place when xs holds the only reference to
// its Vec. Lists are mutable containers others can reference, so the
// aliasing guard matters even more than for strings — these pin that a
// snapshot taken before an append is never mutated, self-append copies,
// and a list handed to another list mid-loop is unharmed by later
// appends. All must agree across tiers.

#[test]
fn list_accumulation_agrees_and_is_linear_shaped() {
    assert_tier_transparent(
        r#"
fn build(n) = {
    let mut xs = []
    let mut i = 0
    while i < n {
        xs = xs + [i * i]
        i = i + 1
    }
    len(xs) + xs[0] + xs[n - 1]
}
build(2000)
"#,
    );
}

#[test]
fn aliased_lists_survive_in_place_append() {
    // `snapshot` holds a second reference when the append happens: the
    // fused path must copy, never extend the shared Vec. Reading the
    // snapshot's length after the append is what catches a leak.
    assert_tier_transparent(
        r#"
fn f() = {
    let mut xs = [1, 2]
    let snapshot = xs
    xs = xs + [3]
    len(snapshot) * 100 + len(xs)
}
f()
"#,
    );
}

#[test]
fn self_append_list_copies() {
    assert_tier_transparent(
        r#"
fn f() = {
    let mut xs = [1, 2]
    xs = xs + xs
    xs = xs + xs
    len(xs) * 1000 + xs[0] + xs[7]
}
f()
"#,
    );
}

#[test]
fn lists_stored_elsewhere_before_append_are_unharmed() {
    // The accumulating list is nested inside another list mid-way;
    // later appends must not mutate the nested copy.
    assert_tier_transparent(
        r#"
fn f() = {
    let mut xs = [1]
    xs = xs + [2]
    let kept = [xs, [9]]
    xs = xs + [3]
    len(kept[0]) * 100 + len(xs)
}
f()
"#,
    );
}

// ── Operand evaluation order: a variable read as an earlier operand must
// not observe assignments made by a later operand. The compiler shields
// the variable's register with a Move in exactly that case; these run
// each shape through both tiers and require the results to agree — and
// require promotion, so the shield is actually exercised.

/// Transparent AND promoted — a divergence test that silently stayed on
/// the interpreter would prove nothing.
fn assert_tier_transparent_and_promoted(src: &str) {
    assert!(
        promotion_count(src, 2) >= 1,
        "test function must be promoted\n  source: {}",
        src
    );
    assert_tier_transparent(src);
}

#[test]
fn binop_left_variable_read_before_assigning_rhs() {
    // Interpreter: acc (10) is read before the rhs block sets acc = 1,
    // so the result is 10 + 5 = 15 — not 1 + 5.
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut acc = 10
    acc = acc + { acc = 1
    5 }
    acc
}
f() + f() + f()
"#,
    );
}

#[test]
fn comparison_left_variable_read_before_assigning_rhs() {
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut a = 1
    let r = a < { a = 100
    2 }
    if r => 1 else => 0
}
f() + f() + f()
"#,
    );
}

#[test]
fn logical_and_left_read_before_assigning_rhs() {
    // `a && rhs`: the combine step must use a's value from BEFORE the
    // rhs sets it to false.
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut a = true
    if a && { a = false
    true } => 1 else => 0
}
f() + f() + f()
"#,
    );
}

#[test]
fn call_argument_read_before_later_assigning_argument() {
    assert_tier_transparent_and_promoted(
        r#"
fn add3(x, y, z) = x + y + z
fn f() = {
    let mut v = 1
    add3(v, { v = 100
    2 }, 3)
}
f() + f() + f()
"#,
    );
}

#[test]
fn callee_variable_read_before_assigning_argument() {
    // The interpreter evaluates the callee before the arguments, so the
    // reassignment inside the argument must not change which function
    // is called.
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut g = (x) => x + 1
    g({ g = (x) => x * 100
    10 })
}
f() + f() + f()
"#,
    );
}

#[test]
fn list_and_tuple_elements_read_before_later_assigning_element() {
    assert_tier_transparent_and_promoted(
        r#"
fn l() = {
    let mut v = 1
    [v, { v = 9
    2 }]
}
fn t() = {
    let mut v = 1
    (v, { v = 9
    2 })
}
let mut total = 0
for round in 0..3 {
    total = total + l()[0] * 1000 + l()[1] * 100 + t()[0] * 10 + t()[1]
}
total
"#,
    );
}

#[test]
fn index_object_read_before_assigning_index_expression() {
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut xs = [1, 2, 3]
    xs[{ xs = [7, 8, 9]
    0 }]
}
f() + f() + f()
"#,
    );
}

#[test]
fn range_start_read_before_assigning_end() {
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut a = 0
    let mut s = 0
    for i in a..({ a = 5
    3 }) {
        s = s + i
    }
    s
}
f() + f() + f()
"#,
    );
}

#[test]
fn for_loop_iterable_snapshot_survives_body_reassignment() {
    // The interpreter iterates the value the iterable had when the loop
    // began; reassigning the variable mid-loop must not change that.
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut xs = [1, 2, 3]
    let mut s = 0
    for x in xs {
        xs = [10, 20]
        s = s + x
    }
    s
}
f() + f() + f()
"#,
    );
}

#[test]
fn map_key_read_before_assigning_value_expression() {
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut k = "a"
    let m = #{k: { k = "b"
    1 }, "z": 2}
    map_get(m, "a")
}
f() + f() + f()
"#,
    );
}

#[test]
fn template_interpolation_read_before_later_assigning_interpolation() {
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut v = 1
    `${v}-${{ v = 2
    v }}`
}
f() + f() + f()
"#,
    );
}

#[test]
fn struct_and_anon_object_fields_read_before_later_assigning_field() {
    assert_tier_transparent_and_promoted(
        r#"
type Pair = struct { a: Int, b: Int }
fn s() = {
    let mut v = 1
    let p = Pair { a: v, b: { v = 9
    2 } }
    p.a * 100 + p.b
}
fn o() = {
    let mut v = 1
    let obj = { a: v, b: { v = 9
    2 } }
    obj.a * 100 + obj.b
}
let mut total = 0
for round in 0..3 {
    total = total + s() * 1000 + o()
}
total
"#,
    );
}

#[test]
fn match_scrutinee_read_before_assigning_guard() {
    // A failing arm's guard reassigns the scrutinee variable; the next
    // arm must still test the value captured when the match began.
    assert_tier_transparent_and_promoted(
        r#"
fn f() = {
    let mut x = 1
    match x {
        _ if { x = 2
        false } => 100,
        1 => 200,
        _ => 300
    }
}
f() + f() + f()
"#,
    );
}

#[test]
fn assignment_free_operands_still_skip_the_shield() {
    // Plain arithmetic on variables must stay shield-free and promoted —
    // this is the hot path the Move must not tax. (Behavioral check only;
    // instruction-level Move counting lives with the compiler.)
    assert_tier_transparent_and_promoted(
        r#"
fn f(a, b) = a + b * a - b
f(3, 4) + f(5, 6) + f(7, 8)
"#,
    );
}

// ── Pre-1.0 bug-hunt regressions ──────────────────────────────────────
//
// A differential fuzzer ran ~10,000 generated programs under `--no-ovm`
// and `--ovm-tier=1` and found no tier divergence; these pin the seams
// that fuzzing swept but the suite never named as concrete cases. Each
// is a shape where a tier *could* plausibly diverge — a JIT deopt on a
// kind change, an overflow guard inside a hot function, a closure's
// captured snapshot — and each is asserted identical on both tiers.

#[test]
fn a_function_specialized_for_ints_handles_a_later_float_call() {
    // The JIT specializes `add` for i64 across 200 hot calls, then a
    // f64 call must deopt to bytecode and still give the float answer,
    // not a reinterpreted-bits integer.
    assert_tier_transparent(
        r#"
fn add(a, b) = a + b
let mut s = 0
let mut i = 0
while i < 200 { s = s + add(i, 1); i = i + 1 }
[add(s, 0), add(1.5, 2.5)]
"#,
    );
}

#[test]
fn a_polymorphic_function_interleaves_int_and_float_calls() {
    assert_tier_transparent(
        r#"
fn dbl(x) = x + x
[dbl(3), dbl(2.5), dbl(3), dbl(4.5)]
"#,
    );
}

#[test]
fn an_overflow_guard_fires_the_same_inside_a_hot_function() {
    // 2^62 is representable; the next doubling overflows. Both tiers must
    // agree on where the guard trips — a native multiply that wrapped
    // instead of trapping would diverge from the interpreter here.
    assert_tier_transparent(
        r#"
fn pow2(n) = if n <= 0 => 1 else => 2 * pow2(n - 1)
pow2(62)
"#,
    );
    // And the overflowing call fails on both tiers, not just one.
    assert_tier_transparent(
        r#"
fn pow2(n) = if n <= 0 => 1 else => 2 * pow2(n - 1)
pow2(64)
"#,
    );
}

#[test]
fn a_struct_field_read_on_a_hot_path_agrees_across_tiers() {
    assert_tier_transparent(
        r#"
type V = struct { x: Int }
fn getx(v) = v.x
let mut t = 0
let mut i = 0
while i < 150 { t = t + getx(V { x: i }); i = i + 1 }
t
"#,
    );
}

#[test]
fn a_recursive_int_result_feeds_a_float_caller_identically() {
    assert_tier_transparent(
        r#"
fn cnt(n) = if n <= 0 => 0 else => 1 + cnt(n - 1)
fn scale(n) = to_float(cnt(n)) * 0.5
scale(20)
"#,
    );
}

#[test]
fn a_loop_snapshotting_closures_captures_distinct_values_on_both_tiers() {
    // The shape of the closure-capture bug the tier-agreement harness
    // found earlier: a snapshot per iteration must stay distinct after
    // promotion, not collapse to one shared cell.
    assert_tier_transparent(
        r#"
let mut fns = []
let mut i = 0
while i < 4 { let snap = i * i; fns = fns + [(() => snap)]; i = i + 1 }
fns |> map((f) => f())
"#,
    );
}

#[test]
fn signed_division_and_remainder_agree_across_tiers() {
    // Truncated division: the remainder takes the dividend's sign. A
    // native path using a different rounding rule would diverge.
    assert_tier_transparent(
        r#"
fn dm(a, b) = [a / b, a % b]
[dm(-7, 2), dm(7, -2), dm(-7, -2), dm(7, 2)]
"#,
    );
}

#[test]
fn a_float_accumulator_in_a_hot_loop_matches_the_interpreter_bit_for_bit() {
    // Float addition is not associative; if the JIT reordered or used
    // an FMA the interpreter did not, the last bits would differ. The
    // answer here is deliberately one that carries rounding error.
    assert_tier_transparent(
        r#"
fn acc(n, s) = if n <= 0 => s else => acc(n - 1, s + 0.1)
acc(100, 0.0)
"#,
    );
}

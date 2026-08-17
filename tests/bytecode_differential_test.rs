//! Differential tests: the bytecode VM must be observationally identical to
//! the tree-walking interpreter on every program it accepts.
//!
//! Each case defines olang functions, runs a target function with given
//! arguments through BOTH the interpreter and the bytecode VM, and asserts
//! the results match (or that both fail). Any divergence is a bytecode-tier
//! bug by definition — the interpreter is the semantics reference.
//!
//! Programs outside the bytecode tier's supported subset are expected to be
//! *rejected at compilation* (never miscompiled); see
//! `unsupported_features_fail_compilation`.

use olang::ast::{Statement, Value};
use olang::ovm::FunctionId;
use olang::ovm::OvmValue;
use olang::ovm::bytecode::BytecodeVm;
use olang::{Interpreter, Parser};

/// Run `target(args)` through the interpreter after evaluating `source`.
fn interpreter_result(source: &str, target: &str, args: &[Value]) -> Result<Value, String> {
    let parser = Parser::new();
    let program = parser.parse(source).map_err(|e| e.to_string())?;
    let mut interpreter = Interpreter::new();
    interpreter
        .eval_program(program)
        .map_err(|e| e.to_string())?;

    let callee = interpreter
        .get_user_variables()
        .get(target)
        .cloned()
        .cloned()
        .ok_or_else(|| format!("function '{}' not defined", target))?;

    interpreter
        .call_function(callee, args.to_vec())
        .map_err(|e| e.to_string())
}

/// Compile every function in `source` into a bytecode VM and run
/// `target(args)`.
fn bytecode_result(source: &str, target: &str, args: &[Value]) -> Result<Value, String> {
    let parser = Parser::new();
    let program = parser.parse(source).map_err(|e| e.to_string())?;

    let mut vm = BytecodeVm::new();
    let mut target_id = None;

    // Register all names first so calls between functions (and recursion)
    // resolve during execution.
    let mut ids = Vec::new();
    for statement in &program.statements {
        if let Statement::FunctionDecl(func) = statement.unwrapped() {
            let id = FunctionId::new();
            vm.register_function(func.name.clone(), id);
            ids.push((id, func.clone()));
            if func.name == target {
                target_id = Some(id);
            }
        }
    }

    for (id, func) in &ids {
        vm.compile_function(*id, func).map_err(|e| e.to_string())?;
    }

    let target_id = target_id.ok_or_else(|| format!("function '{}' not defined", target))?;
    let ovm_args: Vec<OvmValue> = args.iter().map(|v| OvmValue::from_ast(v.clone())).collect();

    vm.execute(target_id, &ovm_args)
        .map_err(|e| e.to_string())
        .and_then(|v| v.to_ast().map_err(|e| format!("{:?}", e)))
}

/// Run a closure on a thread with a generous stack — recursive programs need
/// more than the 2 MB default test-thread stack at interpreter depth.
fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

/// Assert interpreter and bytecode VM agree on `target(args)`.
fn assert_same(source: &str, target: &str, args: &[Value]) {
    let (expected, actual) = {
        let (source, target, args) = (source.to_string(), target.to_string(), args.to_vec());
        with_big_stack(move || {
            (
                interpreter_result(&source, &target, &args),
                bytecode_result(&source, &target, &args),
            )
        })
    };

    match (&expected, &actual) {
        (Ok(e), Ok(a)) => assert_eq!(
            e, a,
            "DIVERGENCE for {}({:?}):\n  interpreter: {:?}\n  bytecode:    {:?}\n  source: {}",
            target, args, e, a, source
        ),
        (Err(_), Err(_)) => {} // both failing is agreement (messages may differ)
        _ => panic!(
            "DIVERGENCE for {}({:?}):\n  interpreter: {:?}\n  bytecode:    {:?}\n  source: {}",
            target, args, expected, actual, source
        ),
    }
}

fn ints(values: &[i64]) -> Vec<Value> {
    values.iter().map(|n| Value::Integer(*n)).collect()
}

#[test]
fn arithmetic() {
    let src = "fn calc(a, b) = a * 3 + b - 2";
    assert_same(src, "calc", &ints(&[5, 4]));
    assert_same(src, "calc", &ints(&[0, 0]));
    assert_same(src, "calc", &ints(&[-7, 100]));
}

#[test]
fn division_and_errors() {
    let src = "fn div(a, b) = a / b";
    assert_same(src, "div", &ints(&[10, 3]));
    assert_same(src, "div", &ints(&[-9, 2]));
    assert_same(src, "div", &ints(&[1, 0])); // both must error
    assert_same(src, "div", &[Value::Integer(i64::MIN), Value::Integer(-1)]); // overflow: both must error
}

#[test]
fn integer_overflow_agrees() {
    let src = "fn bump(a) = a + 1";
    assert_same(src, "bump", &ints(&[41]));
    assert_same(src, "bump", &[Value::Integer(i64::MAX)]); // both must error
}

#[test]
fn float_arithmetic_and_mixed() {
    let src = "fn scale(a, b) = a * b + 0.5";
    assert_same(src, "scale", &[Value::Float(2.5), Value::Float(4.0)]);
    assert_same(src, "scale", &[Value::Integer(3), Value::Float(1.5)]);
    assert_same(src, "scale", &[Value::Float(1e-17), Value::Float(1.0)]);
}

#[test]
fn float_equality_matches_interpreter() {
    let src = "fn same(a, b) = a == b";
    assert_same(src, "same", &[Value::Float(1e-17), Value::Float(0.0)]);
    assert_same(src, "same", &[Value::Float(0.5), Value::Float(0.5)]);
}

#[test]
fn comparisons() {
    let src = "fn cmp(a, b) = a < b";
    for (a, b) in [(1, 2), (2, 1), (3, 3), (-5, 5)] {
        assert_same(src, "cmp", &ints(&[a, b]));
    }
    let src = "fn cmp(a, b) = a >= b";
    for (a, b) in [(1, 2), (2, 1), (3, 3)] {
        assert_same(src, "cmp", &ints(&[a, b]));
    }
}

#[test]
fn if_else_branches() {
    let src = "fn pick(n) = if n > 10 => n * 2 else => n - 1";
    assert_same(src, "pick", &ints(&[11]));
    assert_same(src, "pick", &ints(&[10]));
    assert_same(src, "pick", &ints(&[-3]));
}

#[test]
fn nested_ifs() {
    let src = r#"
fn classify(n) = if n < 0 => 0 - 1 else => if n == 0 => 0 else => 1
"#;
    for n in [-5, 0, 7] {
        assert_same(src, "classify", &ints(&[n]));
    }
}

#[test]
fn if_without_else() {
    let src = "fn maybe(n) = if n > 0 => n";
    assert_same(src, "maybe", &ints(&[5]));
    assert_same(src, "maybe", &ints(&[-5])); // Unit in both
}

#[test]
fn function_calls_between_functions() {
    let src = r#"
fn double(x) = x * 2
fn quad(x) = double(double(x))
"#;
    assert_same(src, "quad", &ints(&[5]));
    assert_same(src, "quad", &ints(&[-3]));
}

#[test]
fn recursion_factorial() {
    let src = "fn fact(n) = if n <= 1 => 1 else => n * fact(n - 1)";
    for n in [0, 1, 5, 10] {
        assert_same(src, "fact", &ints(&[n]));
    }
}

#[test]
fn recursion_fib() {
    let src = "fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)";
    for n in [0, 1, 2, 10, 15] {
        assert_same(src, "fib", &ints(&[n]));
    }
}

#[test]
fn while_loop_with_locals() {
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
"#;
    for n in [0, 1, 10, 100] {
        assert_same(src, "sum_to", &ints(&[n]));
    }
}

#[test]
fn while_loop_never_entered() {
    let src = r#"
fn keep_x(n) = {
    let mut x = 42
    while n > 100 {
        x = 0
    }
    x
}
"#;
    assert_same(src, "keep_x", &ints(&[5]));
}

#[test]
fn string_values() {
    let src = "fn greet(name) = name + \"!\"";
    assert_same(src, "greet", &[Value::String("héllo".to_string().into())]);

    let src = "fn measure(s) = len(s)";
    assert_same(src, "measure", &[Value::String("héllo".to_string().into())]);
    assert_same(src, "measure", &[Value::String(String::new().into())]);
}

#[test]
fn list_construction() {
    let src = "fn triple(a) = [a, a * 2, a * 3]";
    assert_same(src, "triple", &ints(&[7]));

    let src = "fn count(a) = len([a, a, a])";
    assert_same(src, "count", &ints(&[1]));
}

#[test]
fn range_values() {
    let src = "fn r(a, b) = a..b";
    assert_same(src, "r", &ints(&[1, 10]));
    assert_same(src, "r", &ints(&[10, 1]));
}

#[test]
fn wrong_arity_fails_in_both() {
    let src = "fn add(a, b) = a + b";
    assert_same(src, "add", &ints(&[1])); // both must error
    assert_same(src, "add", &ints(&[1, 2, 3])); // both must error
}

#[test]
fn unsupported_features_fail_compilation() {
    // Programs outside the tier's subset must be REJECTED, never miscompiled.
    let cases = [
        // free variable / global
        ("fn f(x) = x + y", "f"),
        // or-patterns that bind are rejected: alternatives would leave
        // different bindings on the success path
        ("fn f(x) = match x { Ok(a) | Err(a) => a, _ => 0 }", "f"),
    ];
    for (src, target) in cases {
        let result = bytecode_result(src, target, &ints(&[1]));
        assert!(
            result.is_err(),
            "expected compilation rejection for {:?}, got {:?}",
            src,
            result
        );
    }
}

#[test]
fn capturing_lambdas_compile_and_agree() {
    // Formerly on the rejection list: a lambda capturing the enclosing
    // function's runtime state now compiles (MakeClosure + hidden trailing
    // capture parameters) and must agree with the interpreter for every
    // argument value — the captures are per-closure, not baked constants.
    // (A lambda calling a builtin still rejects in THIS harness, which
    // compiles against an empty closure; the tier-level suite covers it
    // with the real prelude closure.)
    for arg in [1i64, 3, 10] {
        assert_same("fn f(x) = map([1, 2], (y) => y * x)", "f", &ints(&[arg]));
    }
    // Also formerly rejected: a struct pattern against a non-struct value
    // falls through to the wildcard arm in both tiers.
    assert_same(
        "fn f(x) = match x { User { name, age } => name, _ => 0 }",
        "f",
        &ints(&[1]),
    );
}

#[test]
fn mixed_string_number_add_is_rejected_on_both_tiers() {
    // Mixing a number and a string under `+` is a type error (Python-3
    // style), not a silent stringify — and the interpreter and the VM
    // reject it identically. String+String concatenation still works, so
    // convert the number with to_string(...) first.
    for arg in [0i64, 5, -3] {
        assert_same(r#"fn f(x) = "n=" + x"#, "f", &ints(&[arg]));
        assert_same(r#"fn f(x) = x + "!""#, "f", &ints(&[arg]));
        assert_same(
            r#"fn f(x) = ("v" + (x * 1.5)) + (x * 0.5 + "w")"#,
            "f",
            &ints(&[arg]),
        );
        // The valid path — stringify, then concatenate — still agrees.
        assert_same(r#"fn f(x) = "n=" + to_string(x)"#, "f", &ints(&[arg]));
    }
}

#[test]
fn function_valued_callees_compile_and_agree() {
    // CallValue: an immediately invoked lambda, a curried call, and — the
    // shadowing case that used to be a latent divergence — a parameter
    // named after a builtin, which must be CALLED as the parameter.
    for arg in [1i64, 5, -3] {
        assert_same("fn f(x) = ((y) => y * 3)(x)", "f", &ints(&[arg]));
        assert_same("fn f(x) = ((a) => (b) => a + b)(x)(10)", "f", &ints(&[arg]));
        assert_same("fn f(len) = ((z) => len + z)(1)", "f", &ints(&[arg]));
    }
}

// --- Aliasing and binding cases (register-window hazards) ---
// Variables live in registers; a copy that accidentally shares a register
// with its source would silently change results. These lock that down.

#[test]
fn copy_then_mutate_source() {
    let src = r#"
fn f(a) = {
    let mut a = a
    let mut x = a
    a = a + 100
    x
}
"#;
    assert_same(src, "f", &ints(&[7]));
}

#[test]
fn mutate_copy_leaves_source() {
    let src = r#"
fn f(a) = {
    let mut x = a
    x = x + 100
    a
}
"#;
    assert_same(src, "f", &ints(&[7]));
}

#[test]
fn assignment_to_parameter() {
    let src = r#"
fn f(a, b) = {
    let mut a = a
    let mut b = b
    a = a + b
    b = b * 2
    a + b
}
"#;
    assert_same(src, "f", &ints(&[3, 4]));
}

#[test]
fn self_referential_assignment() {
    let src = r#"
fn f(n) = {
    let mut acc = n
    acc = acc + acc
    acc = acc * acc
    acc
}
"#;
    assert_same(src, "f", &ints(&[3]));
}

#[test]
fn chained_copies() {
    let src = r#"
fn f(a) = {
    let mut x = a
    let mut y = x
    let z = y
    x = 1
    y = 2
    z
}
"#;
    assert_same(src, "f", &ints(&[9]));
}

#[test]
fn nested_block_shadowing_does_not_leak() {
    // 0.61: a block scopes its bindings. The inner `let x` shadows only
    // for the block, so `f` returns the outer 1 — and the bytecode tier
    // must agree, which it only does if the compiler restores its
    // name->register map when the block ends.
    let src = r#"
fn f(a) = {
    let mut x = 1
    { let x = 2
      x }
    x
}
"#;
    assert_same(src, "f", &ints(&[0]));
}

#[test]
fn a_loop_body_binding_is_fresh_each_pass() {
    // The body binds `step` every iteration; scoping it must not change
    // the accumulated result on either tier.
    let src = r#"
fn f(n) = {
    let mut total = 0
    let mut i = 0
    while i < n {
        let step = i * 2
        total = total + step
        i = i + 1
    }
    total
}
"#;
    for n in [0, 1, 7] {
        assert_same(src, "f", &ints(&[n]));
    }
}

#[test]
fn shadowing_in_block() {
    let src = r#"
fn f(a) = {
    let mut x = a
    let mut x = x + 1
    x
}
"#;
    assert_same(src, "f", &ints(&[5]));
}

#[test]
fn swap_via_temporary() {
    let src = r#"
fn f(a, b) = {
    let mut a = a
    let mut b = b
    let tmp = a
    a = b
    b = tmp
    a * 10 + b
}
"#;
    assert_same(src, "f", &ints(&[3, 7]));
}

#[test]
fn loop_carried_variables() {
    let src = r#"
fn f(n) = {
    let mut a = 0
    let mut b = 1
    let mut i = 0
    while i < n {
        let next = a + b
        a = b
        b = next
        i = i + 1
    }
    a
}
"#;
    for n in [0, 1, 5, 20] {
        assert_same(src, "f", &ints(&[n]));
    }
}

#[test]
fn argument_registers_not_clobbered_by_call() {
    let src = r#"
fn double(x) = x * 2
fn f(a, b) = double(a) + b + a
"#;
    assert_same(src, "f", &ints(&[3, 5]));
}

#[test]
fn nested_calls_preserve_caller_values() {
    let src = r#"
fn inner(x) = x + 1
fn middle(x) = inner(x) * 2
fn outer(a, b) = middle(a) + middle(b) + a + b
"#;
    assert_same(src, "outer", &ints(&[2, 3]));
}

#[test]
fn modulo_matches_interpreter() {
    let src = "fn m(a, b) = a % b";
    for (a, b) in [(10, 3), (-10, 3), (10, -3), (-10, -3), (7, 7)] {
        assert_same(src, "m", &ints(&[a, b]));
    }
    assert_same(src, "m", &ints(&[1, 0])); // both must error
    assert_same(src, "m", &[Value::Integer(i64::MIN), Value::Integer(-1)]);
}

#[test]
fn math_domain_errors_agree_across_tiers() {
    // The compiled float-math shortcut must raise the same domain errors
    // the interpreter does, not silently return NaN. Both tiers erroring
    // is agreement; the bug was the bytecode tier returning Ok(NaN).
    let floats = |xs: &[f64]| xs.iter().map(|x| Value::Float(*x)).collect::<Vec<_>>();
    assert_same("fn f(x) = math.sqrt(x)", "f", &floats(&[-4.0]));
    assert_same("fn f(x) = math.sqrt(x)", "f", &floats(&[9.0])); // valid: agree on value
    assert_same("fn f(x) = math.asin(x)", "f", &floats(&[2.0]));
    assert_same("fn f(x) = math.acos(x)", "f", &floats(&[5.0]));
    assert_same("fn f(x) = math.ln(x)", "f", &floats(&[0.0]));
    assert_same("fn f(x) = math.log2(x)", "f", &floats(&[-1.0]));
    assert_same("fn f(x) = math.log10(x)", "f", &floats(&[0.0]));
}

/// Two closures from one factory, called through a shared higher-order
/// function, must not share captures.
///
/// The compiled tier caches functions it compiles on demand for the
/// higher-order path, and the key was the body's allocation identity
/// alone. `compile_function_with_closure` bakes the captured environment
/// *into* the compiled body, so two closures from one factory — same
/// lambda body, different captures — are different compiled functions
/// that the cache treated as one. The second closure ran the first's
/// captures, silently.
///
/// It needed a shared call site to surface: calling the closures directly
/// takes a different path that carries captures explicitly. That is why
/// this reproduces with `apply` and not without it, and why the failure
/// hid until a parser-combinator program ran through the corpus.
#[test]
fn two_closures_from_one_factory_keep_their_own_captures() {
    let source = r#"
fn apply(f, x) = f(x)
fn adder(k) = (n) => n + k
fn probe() = {
    let a1 = adder(1)
    let a100 = adder(100)
    apply(a1, 0) * 1000 + apply(a100, 0)
}
"#;
    let interpreted = interpreter_result(source, "probe", &[]).expect("interpreter");
    let compiled = bytecode_result(source, "probe", &[]).expect("bytecode");
    assert_eq!(
        interpreted, compiled,
        "the second closure must use its own capture, not the first's"
    );
    assert_eq!(interpreted, Value::Integer(1100));
}

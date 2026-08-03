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
use olang::ovm::bytecode::BytecodeVm;
use olang::ovm::FunctionId;
use olang::ovm::OvmValue;
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
        if let Statement::FunctionDecl(func) = statement {
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
    let total = 0
    let i = 0
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
    let x = 42
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
        // match expression
        ("fn f(x) = match x { 1 => 2, _ => 3 }", "f"),
        // lambda
        ("fn f(x) = ((y) => y)(x)", "f"),
        // pipeline
        ("fn f(x) = x |> to_string", "f"),
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

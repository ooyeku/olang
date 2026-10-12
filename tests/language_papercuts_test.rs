//! The 0.88 language papercuts: each a form a real program ran into
//! (docs/roadmap.md's rows, closed one by one).
//!
//! Every case runs three ways and must agree exactly — the interpreter
//! (the reference), the bytecode VM (each function compiled directly,
//! which must succeed: these forms stay on the tier), and the tiered
//! interpreter at threshold 1 (bytecode promoted at its first call and
//! the JIT taking what it can).

use olang::ast::{Statement, Value};
use olang::ovm::FunctionId;
use olang::ovm::OvmValue;
use olang::ovm::bytecode::BytecodeVm;
use olang::{Interpreter, Parser};

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn eval(source: &str, tier: Option<u32>) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        if let Some(threshold) = tier {
            interpreter.enable_bytecode_tier(threshold, false);
        }
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

/// `main()` compiled on the bytecode VM with every function of the
/// program; refusing to compile is a failure of the test.
fn on_vm(source: &str) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
        let mut vm = BytecodeVm::new();
        let mut ids = Vec::new();
        let mut main = None;
        for statement in &program.statements {
            if let Statement::FunctionDecl(func) = statement.unwrapped() {
                let id = FunctionId::new();
                vm.register_function(func.name.clone(), id);
                if func.name == "main" {
                    main = Some(id);
                }
                ids.push((id, func.clone()));
            }
        }
        for (id, func) in &ids {
            if let Err(e) = vm.compile_function(*id, func) {
                panic!("`{}` refused by the bytecode tier: {}", func.name, e);
            }
        }
        let main = main.expect("a main function");
        vm.execute(main, &[] as &[OvmValue])
            .map_err(|e| e.to_string())
            .and_then(|v| v.into_ast().map_err(|e| format!("{:?}", e)))
    })
}

/// The interpreter's answer, which the tiered run must give too, message
/// for message. For programs that need the interpreter's environment
/// (modules, declared types, traits) to compile.
fn agree_tiered(source: &str) -> Result<Value, String> {
    let interpreted = eval(source, None);
    let tiered = eval(source, Some(1));
    assert_eq!(tiered, interpreted, "tiered diverged from the interpreter\n{source}");
    interpreted
}

/// The interpreter's answer, which the VM and the tiered run must give
/// too. The program ends with a call to `main()`.
fn agree(source: &str) -> Result<Value, String> {
    let interpreted = agree_tiered(source);
    // The VM run is of `main` alone; its error text may name less
    // context, so errors are compared as errors.
    let vm = on_vm(source);
    match (&interpreted, &vm) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "the VM diverged\n{source}"),
        (Err(_), Err(_)) => {}
        _ => panic!("the VM diverged: {interpreted:?} vs {vm:?}\n{source}"),
    }
    interpreted
}

fn s(text: &str) -> Value {
    Value::String(std::sync::Arc::new(text.to_string()))
}

// ── A lambda stage ends at the next `|>` ───────────────────────────────

#[test]
fn a_lambda_stage_ends_at_the_next_pipe() {
    let src = r#"
fn wrap(x) = [x]
fn main() = {
    let a = 5 |> (v) => if v > 3 => v else => 0 |> wrap
    let b = 1 |> (v) => if v > 3 => v else => 0 |> wrap
    (a, b)
}
main()
"#;
    let five = Value::List(std::sync::Arc::new(vec![Value::Integer(5)]));
    let zero = Value::List(std::sync::Arc::new(vec![Value::Integer(0)]));
    assert_eq!(agree(src), Ok(Value::Tuple(std::sync::Arc::new(vec![five, zero]))));
}

#[test]
fn stages_of_lambdas_chain_and_each_sees_its_own_parameter() {
    let src = r#"
fn main() = {
    let m = #{}
        |> (s) => if true => map_set(s, "a", 1) else => s
        |> (s) => if false => s else if true => map_set(s, "b", 2) else => s
        |> (s) => map_set(s, "c", 3)
    m.a + m.b + m.c
}
main()
"#;
    assert_eq!(agree_tiered(src), Ok(Value::Integer(6)));
}

#[test]
fn a_lower_operator_before_the_pipe_stays_in_the_stage() {
    // `v == 1 |> g` in a stage: the stage is `v == 1`, then `g`.
    let src = r#"
fn not(b) = !b
fn main() = 1 |> (v) => v == 1 |> not
main()
"#;
    assert_eq!(agree(src), Ok(Value::Boolean(false)));
}

#[test]
fn a_pipeline_in_parentheses_or_outside_a_stage_stays_in_the_body() {
    let src = r#"
fn wrap(x) = [x]
fn main() = {
    let h = (v) => v |> wrap
    let a = 3 |> (v) => (v |> wrap)
    let b = [1, 2] |> map((v) => v + 1 |> wrap)
    (h(1), a, b)
}
main()
"#;
    let l = |xs: Vec<Value>| Value::List(std::sync::Arc::new(xs));
    assert_eq!(
        agree_tiered(src),
        Ok(Value::Tuple(std::sync::Arc::new(vec![
            l(vec![Value::Integer(1)]),
            l(vec![Value::Integer(3)]),
            l(vec![l(vec![Value::Integer(2)]), l(vec![Value::Integer(3)])]),
        ])))
    );
}

#[test]
fn a_block_bodied_stage_is_unchanged() {
    let src = r#"
fn main() = 2 |> (v) => { v * 3 } |> (w) => w + 1
main()
"#;
    assert_eq!(agree(src), Ok(Value::Integer(7)));
}

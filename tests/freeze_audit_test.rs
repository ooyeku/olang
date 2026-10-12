//! The 1.0 freeze audit's rows (docs/roadmap.md, "The freeze audit"),
//! closed one by one: each ruling with its program on every tier.
//!
//! Every case runs three ways and must agree exactly — the interpreter
//! (the reference), the bytecode VM (each function compiled directly,
//! which must succeed), and the tiered interpreter at threshold 1
//! (bytecode promoted at its first call and the JIT taking what it can).

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
    assert_eq!(
        tiered, interpreted,
        "tiered diverged from the interpreter\n{source}"
    );
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

fn l(xs: Vec<Value>) -> Value {
    Value::List(std::sync::Arc::new(xs))
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

// ── A bad date format is an Err, not an abort ──────────────────────────

#[test]
fn a_bad_date_format_is_an_err() {
    let src = r#"
fn main() = [
    dates.format_date("2026-10-12", "%Q"),
    dates.format_datetime("2026-10-12T10:00:00", "%Y %"),
    dates.format_time("10:00:00", "%Q"),
    dates.format_date("2026-10-12", "%H"),
    dates.format_date("2026-10-12", "%Y")
]
main()
"#;
    let Ok(Value::List(ref items)) = agree_tiered(src) else {
        panic!("expected a list");
    };
    for item in &items[..4] {
        assert!(matches!(item, Value::Err(_)), "{item:?}");
    }
    assert_eq!(items[4], ok(s("2026")));
}

// ── The collections' empty reads and pops ──────────────────────────────

#[test]
fn an_empty_heap_or_deque_reads_unit_and_pops_to_itself() {
    let src = r#"
use collections { heap, deque }
fn main() = {
    let mut q = deque.new()
    let reads = [deque.front(q), deque.back(q)]
    q = deque.pop_front(q)
    q = deque.pop_back(q)
    q = deque.push_back(q, 1)
    let mut h = heap.new()
    let hreads = [heap.top_prio(h), heap.top_item(h)]
    h = heap.pop(h)
    h = heap.push(h, 2, "two")
    h = heap.pop(h)
    [reads, hreads, deque.to_list(q), heap.size(h), heap.top_item(h)]
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            l(vec![Value::Unit, Value::Unit]),
            l(vec![Value::Unit, Value::Unit]),
            l(vec![Value::Integer(1)]),
            Value::Integer(0),
            Value::Unit,
        ]))
    );
}

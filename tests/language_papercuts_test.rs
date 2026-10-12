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

// ── A tuple after a comparison is an operand ───────────────────────────

#[test]
fn a_tuple_after_a_comparison_is_compared_not_a_lambda() {
    let src = r#"
fn same(a, x, y) = {
    if a == (x, y) => return "same"
    "diff"
}
fn kind(p) = match p {
    q if q == (0, 0) => "origin",
    q if q != (0, 0) => "elsewhere"
}
fn main() = [same((1, 2), 1, 2), same((1, 2), 2, 1), kind((0, 0)), kind((3, 4))]
main()
"#;
    assert_eq!(
        agree(src),
        Ok(Value::List(std::sync::Arc::new(vec![
            s("same"),
            s("diff"),
            s("origin"),
            s("elsewhere")
        ])))
    );
}

#[test]
fn unit_and_a_parenthesized_lambda_still_compare() {
    let src = r#"
fn main() = {
    let t = ()
    let a = if t == () => 1 else => 2
    let b = if (1) == (1) => 3 else => 4
    a + b
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Integer(4)));
}

// ── `xs[i] = v` ─────────────────────────────────────────────────────────

fn ints(xs: &[i64]) -> Value {
    Value::List(std::sync::Arc::new(xs.iter().map(|n| Value::Integer(*n)).collect()))
}

#[test]
fn an_element_of_a_mut_list_is_assigned() {
    let src = r#"
fn main() = {
    let mut xs = [1, 2, 3, 4]
    xs[1] = 9
    xs[-1] = 7
    for i in 0..2 {
        xs[i] = xs[i] * 10
    }
    xs
}
main()
"#;
    assert_eq!(agree(src), Ok(ints(&[10, 90, 3, 7])));
}

#[test]
fn another_binding_of_the_list_keeps_its_elements() {
    let src = r#"
fn main() = {
    let mut a = [1, 2]
    let b = a
    a[0] = 5
    (a, b)
}
main()
"#;
    assert_eq!(
        agree(src),
        Ok(Value::Tuple(std::sync::Arc::new(vec![ints(&[5, 2]), ints(&[1, 2])])))
    );
}

#[test]
fn a_list_in_a_map_or_an_object_is_written_one_level_in() {
    let src = r#"
fn main() = {
    let mut m = #{ "rows": [1, 2, 3], "name": "t" }
    m.rows[2] = 30
    let mut o = { cells: ["a", "b"] }
    o.cells[0] = "z"
    (m.rows, m.name, o.cells[0], typeof(o))
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(Value::Tuple(std::sync::Arc::new(vec![
            ints(&[1, 2, 30]),
            s("t"),
            s("z"),
            s("Object")
        ])))
    );
}

#[test]
fn an_index_outside_the_list_raises_the_reads_error() {
    let src = r#"
fn main() = {
    let mut xs = [1, 2, 3]
    xs[3] = 0
    xs
}
main()
"#;
    let err = agree(src).expect_err("out of bounds");
    assert!(err.contains("Index 3 out of bounds for list of length 3"), "{err}");
    let src = r#"
fn main() = {
    let mut m = #{ "a": 1 }
    let mut xs = m
    xs["a"] = 2
    xs
}
main()
"#;
    let err = agree(src).expect_err("a map");
    assert!(err.contains("a Map is not indexed with `[]`"), "{err}");
}

#[test]
fn many_writes_to_one_list_stay_in_place() {
    // 20,000 writes to a 20,000-element list: a copy per write would be
    // 400 million element copies.
    let src = r#"
fn main() = {
    let n = 20000
    let mut xs = col.filled(n, 0)
    for i in 0..n {
        xs[i] = i
    }
    xs[n - 1] + xs[0]
}
main()
"#;
    let started = std::time::Instant::now();
    assert_eq!(agree_tiered(src), Ok(Value::Integer(19999)));
    assert!(started.elapsed().as_secs() < 20, "{:?}", started.elapsed());
}

#[test]
fn an_element_assignment_is_an_assignment() {
    // The binding must be `let mut`: the scope rules report it before
    // the program runs.
    let program = Parser::new().parse("let xs = [1]\nxs[0] = 2\n").expect("parses");
    let mut interpreter = Interpreter::new();
    let err = interpreter.eval_program(program).expect_err("immutable");
    assert!(err.to_string().contains("not declared mutable"), "{err}");
}

// ── `raise` ─────────────────────────────────────────────────────────────

#[test]
fn raise_stops_with_its_message_intact() {
    let src = r#"
fn widget(kind) = if kind == "button" => 1 else => raise("widget: no kind " + kind)
fn main() = (
    attempt(() => widget("dial")),
    attempt(() => widget("button")),
    attempt(() => raise(42)),
    attempt(() => raise("Runtime error-free text: [1, 2]"))
)
main()
"#;
    let err = |t: &str| Value::Err(Box::new(s(t)));
    assert_eq!(
        agree_tiered(src),
        Ok(Value::Tuple(std::sync::Arc::new(vec![
            err("widget: no kind dial"),
            Value::Ok(Box::new(Value::Integer(1))),
            err("42"),
            err("Runtime error-free text: [1, 2]"),
        ])))
    );
}

#[test]
fn an_uncaught_raise_is_a_runtime_error() {
    let src = r#"
fn check(n) = if n > 0 => n else => raise("check: n must be positive, got " + show(n))
fn main() = check(0)
main()
"#;
    let err = agree(src).expect_err("raises");
    assert_eq!(err, "Runtime error: check: n must be positive, got 0");
}

// ── A function named as a builtin calls itself ─────────────────────────

#[test]
fn a_function_named_as_a_builtin_calls_itself_on_every_tier() {
    // Its declaration-time closure holds the builtin `find`; the bytecode
    // tier called that at threshold 1 ("Arity mismatch: expected 2, got
    // 3"), where the interpreter reaches the user's function.
    let src = r#"
fn find(a, b, c) = if a > 0 => find(a - 1, b, c) else => b + c
fn sort(xs, n) = if n == 0 => xs else => sort(xs + [n], n - 1)
(find(3, 1, 2), sort([], 2))
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(Value::Tuple(std::sync::Arc::new(vec![Value::Integer(3), ints(&[2, 1])])))
    );
}

// ── Tuples order lexicographically ──────────────────────────────────────

#[test]
fn tuples_order_lexicographically_under_every_comparison() {
    let src = r#"
fn main() = [
    (1, 2) < (1, 3),
    (2, 0) > (1, 9),
    (1, "b") <= (1, "b"),
    (1, (2, 3)) < (1, (2, 4)),
    (1, 2) < (1, 2, 0),
    (true, 1) < (true, 2),
    (1.5, "a") >= (1, "z")
]
main()
"#;
    assert_eq!(
        agree(src),
        Ok(Value::List(std::sync::Arc::new(vec![Value::Boolean(true); 7])))
    );
}

#[test]
fn sort_orders_a_list_of_tuples() {
    let src = r#"
fn main() = sort([(2, "b"), (1, "z"), (2, "a"), (1, "a")])
main()
"#;
    let t = |n: i64, x: &str| Value::Tuple(std::sync::Arc::new(vec![Value::Integer(n), s(x)]));
    assert_eq!(
        agree_tiered(src),
        Ok(Value::List(std::sync::Arc::new(vec![
            t(1, "a"),
            t(1, "z"),
            t(2, "a"),
            t(2, "b")
        ])))
    );
    // Several keys through col.sort_by: numbers as numbers, not text.
    let src = r#"
fn main() = col.sort_by([#{ "y": 10, "n": "a" }, #{ "y": 9, "n": "b" }], (r) => (r.y, r.n)) |> map((r) => r.n)
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(Value::List(std::sync::Arc::new(vec![s("b"), s("a")])))
    );
}

#[test]
fn an_unequal_pair_without_an_order_raises() {
    let src = r#"
fn main() = (1, "a") < ("b", 2)
main()
"#;
    let err = agree(src).expect_err("no order");
    assert!(err.contains("cannot order Int and String inside a tuple"), "{err}");
    let src = r#"
fn main() = sort([(1, "a"), (1, 2)])
main()
"#;
    let err = agree_tiered(src).expect_err("no order");
    assert!(err.contains("sort: cannot order ") && err.contains(" inside a tuple"), "{err}");
}

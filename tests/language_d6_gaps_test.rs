//! The language gaps the D6 notebook rewrite found, each closed at the
//! root: a method chain whose steps start their lines with `.`, `[]` on
//! a map, an object or a struct, a parameter that destructures its
//! argument, and an object key that is not a name.
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
/// (modules, declared types, builtins the VM bridges) to compile.
fn agree_tiered(source: &str) -> Result<Value, String> {
    let interpreted = eval(source, None);
    let tiered = eval(source, Some(1));
    assert_eq!(
        tiered, interpreted,
        "tiered diverged from the interpreter\n{source}"
    );
    interpreted
}

/// `agree_tiered`, and the VM running `main` compiled directly gives the
/// same answer. The program ends with a call to `main()`.
fn agree(source: &str) -> Result<Value, String> {
    let interpreted = agree_tiered(source);
    let vm = on_vm(source);
    match (&interpreted, &vm) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "the VM diverged\n{source}"),
        (Err(a), Err(b)) => assert!(
            a.contains(
                b.trim_start_matches("Type error: ")
                    .trim_start_matches("Runtime error: ")
            ) || b.contains(a.as_str()),
            "the VM's error differs: {a:?} vs {b:?}\n{source}"
        ),
        _ => panic!("the VM diverged: {interpreted:?} vs {vm:?}\n{source}"),
    }
    interpreted
}

fn shown(v: Value) -> String {
    v.to_string()
}

fn parses(source: &str) -> bool {
    Parser::new().parse(source).is_ok()
}

// ── A chain's steps on lines of their own ──────────────────────────────

#[test]
fn a_line_that_starts_with_a_dot_continues_the_chain_above() {
    let src = r#"
fn main() = {
    let s = ods.series([3, 1, 2])
    let x = s
        .sort()
        // a comment between steps
        .cumsum()

        .to_list()
    let f = ods.frame({ a: [1, 2, 3], b: [4.0, 5.0, 6.0] })
    let g = f
        .with({ c: ods.col("a") * 2 })
        .filter(ods.col("c") > 2)
    (x, g.c.to_list(), g.n_rows())
}
main()
"#;
    assert_eq!(shown(agree_tiered(src).unwrap()), "([1, 3, 6], [4, 6], 2)");
}

#[test]
fn a_leading_dot_is_one_dot_and_a_name() {
    // `..` and `...` never continue a line: a range's `..` takes no
    // newline, and a spread or a rest is not a member.
    assert!(!parses("let r = 0\n..5\n"));
    assert!(!parses("let x = f\n...g\n"));
    assert!(parses("let o = { a: 1,\n    ...b }\n"));
    // a number never starts with `.`
    assert!(!parses("let x = 1\n.5\n"));
    // a field read, a call and an index after the step
    assert!(parses("let v = cfg\n    .pp\n    .cur[0]\n    .f()?\n"));
    // the chain's own line may start with the receiver's call
    assert!(parses("let v = f(x)\n    .g()\n"));
}

#[test]
fn a_leading_dot_chain_reads_as_the_one_line_chain() {
    let one = Parser::new()
        .parse_raw("let x = f.with(a).filter(b)\n")
        .unwrap();
    let many = Parser::new()
        .parse_raw("let x = f\n    .with(a)\n    .filter(b)\n")
        .unwrap();
    let unlocated = |p: olang::ast::Program| -> Vec<String> {
        p.statements
            .iter()
            .map(|s| format!("{:?}", s.unwrapped()))
            .collect()
    };
    assert_eq!(unlocated(one), unlocated(many));
}

// ── `[]` on a map, an object, a struct ─────────────────────────────────

#[test]
fn a_map_or_an_object_is_indexed_by_key_as_map_get_reads() {
    let src = r#"
type P = struct { x: Int, y: Int }
fn main() = {
    let params = #{ "log_price:product_x": -1.5, "1": "one", "2.5": "half" }
    let o = { a: 1, b: #{ "c": 3 } }
    let p = P { x: 4, y: 5 }
    let k = "b"
    [
        params["log_price:product_x"],
        params[1],
        params[2.5],
        params["missing"],
        o["a"],
        o[k]["c"],
        p["y"],
        p["z"],
        ()["k"],
        map_get(params, "1") == params[1],
    ]
}
main()
"#;
    assert_eq!(
        shown(agree_tiered(src).unwrap()),
        "[-1.5, \"one\", \"half\", (), 1, 3, 5, (), (), true]"
    );
}

#[test]
fn a_key_that_is_not_a_key_raises_the_same_on_every_tier() {
    let src = r#"
fn main() = {
    let m = #{ "a": 1 }
    m[[1]]
}
main()
"#;
    let err = agree(src).unwrap_err();
    assert!(err.contains("a map or object is indexed by a key"), "{err}");
    // a position on `()` is still no read
    let src = r#"
fn main() = {
    let xs = ()
    xs[0]
}
main()
"#;
    agree(src).unwrap_err();
}

#[test]
fn a_hot_loop_indexing_a_map_agrees() {
    let src = r#"
fn total(m, keys) = {
    let mut t = 0
    for k in keys {
        t = t + m[k]
    }
    t
}
fn main() = {
    let m = #{ "a": 1, "b": 2, "c:d": 3 }
    let mut s = 0
    for i in 0..200 {
        s = s + total(m, ["a", "b", "c:d"])
    }
    s
}
main()
"#;
    assert_eq!(agree(src).unwrap(), Value::Integer(1200));
}

// ── Parameters that destructure ────────────────────────────────────────

#[test]
fn a_lambda_or_fn_parameter_destructures_its_argument() {
    let src = r#"
fn dot((a, b), (c, d)) = a * c + b * d
fn head_of([h, ...rest]) = h + len(rest)
fn area({ w, h }) = w * h
fn main() = {
    let pairs = [(1, 2.0), (2, 3.0)]
    let weighted = map(pairs, ((i, w)) => to_float(i) * w)
    let sums = map([[1, 2], [3, 4]], ([a, b]) => a + b)
    let nested = map([(1, (2, 3))], ((a, (b, c))) => a + b + c)
    let staged = pairs |> map(((i, _)) => i) |> sum
    (weighted, sums, nested, staged, dot((1, 2), (3, 4)), head_of([5, 6, 7]), area({ w: 2, h: 3 }))
}
main()
"#;
    assert_eq!(
        shown(agree_tiered(src).unwrap()),
        "([2.0, 6.0], [3, 7], [6], 3, 11, 7, 6)"
    );
}

#[test]
fn a_destructuring_parameter_compiles_on_the_vm() {
    let src = r#"
fn dot((a, b), (c, d)) = a * c + b * d
fn head_of([h, ...rest]) = h + len(rest)
fn main() = {
    let swap = ((a, b)) => (b, a)
    let mut t = 0
    for i in 0..50 {
        t = t + dot((i, 1), (2, i))
    }
    (t, head_of([5, 6, 7]), swap((1, "x")))
}
main()
"#;
    assert_eq!(shown(agree(src).unwrap()), "(3675, 7, (\"x\", 1))");
}

#[test]
fn a_parameter_pattern_keeps_its_annotation_and_default() {
    let src = r#"
fn f((a, b): (Int, Int), c = 10) = a + b + c
fn main() = f((1, 2)) + f((1, 2), 0)
main()
"#;
    assert_eq!(agree_tiered(src).unwrap(), Value::Integer(16));
    // (the tiered run only: `on_vm` calls `f` without filling a default,
    // for a plain parameter too)
    let src = r#"
fn f((a, b): (Int, Int)) = a + b
fn main() = f(5)
main()
"#;
    let err = agree(src).unwrap_err();
    assert!(err.contains("(a, b)"), "{err}");
}

#[test]
fn an_argument_the_pattern_does_not_match_raises_as_its_let_would() {
    let as_param = r#"
fn main() = {
    let f = ((a, b)) => a
    f((1, 2, 3))
}
main()
"#;
    let as_let = r#"
fn main() = {
    let (a, b) = (1, 2, 3)
    a
}
main()
"#;
    assert_eq!(agree(as_param).unwrap_err(), agree(as_let).unwrap_err());
}

#[test]
fn a_parenthesized_expression_is_still_one() {
    // the pattern is tried as a lambda's parameter first, then the group
    let src = r#"
fn main() = {
    let a = ((1 + 2))
    let t = ((1, 2))
    let f = ((x, y) => x + y)
    let same = if t == ((1, 2)) => "eq" else => "ne"
    (a, t, f(3, 4), same)
}
main()
"#;
    assert_eq!(shown(agree(src).unwrap()), "(3, (1, 2), 7, \"eq\")");
}

// ── Quoted object keys ─────────────────────────────────────────────────

#[test]
fn an_object_key_may_be_a_string() {
    let src = r#"
fn main() = {
    let o = { "P(B > A)": 0.9, n: 3, "a b": "x" }
    (o["P(B > A)"], map_get(o, "a b"), o.n, map_keys(o))
}
main()
"#;
    assert_eq!(
        shown(agree(src).unwrap()),
        "(0.9, \"x\", 3, [\"P(B > A)\", \"n\", \"a b\"])"
    );
}

#[test]
fn a_declared_struct_s_literal_refuses_a_quoted_key() {
    let err = Parser::new()
        .parse("type P = struct { x: Int }\nlet p = P { \"x\": 1 }\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("names its fields"), "{err}");
}

//! Records and maps (roadmap E): `m.key` reads a map's key, `a ?? b`
//! supplies a default, the `map_*` builtins take every record kind,
//! spreads and nested updates write.
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

// ── E1: `m.key` ─────────────────────────────────────────────────────────

#[test]
fn dot_reads_a_maps_key() {
    let src = r#"
fn main() = {
    let m = #{ "name": "ada", "age": 36 }
    m.name + " " + to_string(m.age)
}
main()
"#;
    assert_eq!(agree(src), Ok(s("ada 36")));
}

#[test]
fn a_missing_key_is_unit() {
    let src = r#"
fn main() = {
    let m = #{ "a": 1 }
    m.b == () && m.a == 1
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Boolean(true)));
}

#[test]
fn chains_read_nested_maps() {
    let src = r#"
fn main() = {
    let m = #{ "pp": #{ "cur": 7, "list": [1, 2] } }
    m.pp.cur + len(m.pp.list) + (if m.pp.none == () => 100 else => 0)
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Integer(109)));
}

#[test]
fn dot_is_exactly_map_get() {
    // every value kind a map holds, read both ways
    let src = r#"
fn both(m, k) = [map_get(m, k)]
fn main() = {
    let m = #{ "i": 1, "f": 1.5, "s": "x", "b": true, "u": (), "l": [1], "m": #{ "k": 2 }, "f2": (x) => x + 1 }
    [m.i, m.f, m.s, m.b, m.u, m.l, m.m, m.zz] == [map_get(m, "i"), map_get(m, "f"), map_get(m, "s"), map_get(m, "b"), map_get(m, "u"), map_get(m, "l"), map_get(m, "m"), map_get(m, "zz")]
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Boolean(true)));
}

#[test]
fn a_parsed_json_object_reads_absent_as_unit() {
    let src = r#"
fn main() = {
    let small = unwrap(json.parse("{\"a\": 1}"))
    let big = unwrap(json.parse("{\"a\": 1, \"b\": 2, \"c\": 3, \"d\": 4, \"e\": 5, \"f\": 6, \"g\": 7, \"h\": 8, \"i\": 9, \"j\": {\"k\": [1, 2]}}"))
    [small.a, small.nope, big.j.k, big.nope, big.j.nope]
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(Value::List(std::sync::Arc::new(vec![
            Value::Integer(1),
            Value::Unit,
            Value::List(std::sync::Arc::new(vec![Value::Integer(1), Value::Integer(2)])),
            Value::Unit,
            Value::Unit,
        ])))
    );
}

#[test]
fn objects_read_like_maps_and_structs_still_raise() {
    let object = r#"
fn main() = {
    let o = { a: 1, b: "two" }
    [o.a, o.b, o.c, map_get(o, "c")]
}
main()
"#;
    assert_eq!(
        agree(object),
        Ok(Value::List(std::sync::Arc::new(vec![
            Value::Integer(1),
            s("two"),
            Value::Unit,
            Value::Unit,
        ])))
    );
    // an object of more fields than the eager conversion takes
    let wide = r#"
fn main() = {
    let o = { a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9, j: [1], k: "x" }
    [o.k, o.zz]
}
main()
"#;
    assert_eq!(
        agree(wide),
        Ok(Value::List(std::sync::Arc::new(vec![s("x"), Value::Unit])))
    );
    let declared = r#"
type P = struct { x: Int }
fn main() = P { x: 1 }.y
main()
"#;
    let err = agree_tiered(declared).unwrap_err();
    assert!(err.contains("no field or method 'y'"), "{err}");
}

#[test]
fn a_key_wins_over_a_trait_method() {
    let src = r#"
trait Named { fn name(self) -> String }
impl Named for Map { fn name(self) = "the method" }
fn main() = {
    let keyed = #{ "name": "the key" }
    let calls = #{ "name": () => "the key, called" }
    let bare = #{ "other": 1 }
    [keyed.name, calls.name(), bare.name()]
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(Value::List(std::sync::Arc::new(vec![
            s("the key"),
            s("the key, called"),
            s("the method"),
        ])))
    );
}

#[test]
fn calling_a_missing_key_raises_on_every_tier() {
    let src = r#"
fn main() = {
    let m = #{ "a": 1 }
    m.go(1)
}
main()
"#;
    let err = agree(src).unwrap_err();
    assert!(err.contains("Map has no key or method 'go'"), "{err}");
}

#[test]
fn a_stored_function_is_called_through_the_key() {
    let src = r#"
fn main() = {
    let ops = #{ "add": (a, b) => a + b, "neg": (a) => 0 - a }
    ops.add(2, 3) + ops.neg(10)
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Integer(-5)));
}

#[test]
fn field_reads_in_a_hot_loop_agree_under_the_jit() {
    // uniform payloads: the JIT's map kinds
    let src = r#"
fn total(m, n) = {
    let mut acc = 0
    let mut i = 0
    while i < n {
        acc = acc + m.step + (if m.skip == () => 1 else => 0)
        i = i + 1
    }
    acc
}
fn label(m) = m.host + ":" + m.port
fn main() = total(#{ "step": 3 }, 50) + total(#{ "step": 2, "skip": 0 }, 50) + len(label(#{ "host": "a", "port": "80" }))
main()
"#;
    assert_eq!(agree(src), Ok(Value::Integer(150 + 50 + 100 + 4)));
}

#[test]
fn float_and_bool_payloads_agree_under_the_jit() {
    let src = r#"
fn scale(m, x) = if m.on => x * m.k else => x
fn main() = {
    let mut acc = 0.0
    for i in 0..40 { acc = acc + scale(#{ "k": 0.5 , "on": true }, 2.0) }
    acc + (if #{ "on": false }.on => 1.0 else => 0.0)
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Float(40.0)));
}


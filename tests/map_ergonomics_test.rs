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
            Value::List(std::sync::Arc::new(vec![
                Value::Integer(1),
                Value::Integer(2)
            ])),
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

// ── E2: `a ?? b` ────────────────────────────────────────────────────────

#[test]
fn coalesce_takes_the_right_only_for_unit() {
    let src = r#"
fn main() = {
    let m = #{ "a": 1, "u": (), "f": false, "z": 0, "e": "" }
    [m.a ?? 9, m.u ?? 9, m.none ?? 9, m.f ?? 9, m.z ?? 9, m.e ?? 9, () ?? () ?? 7]
}
main()
"#;
    assert_eq!(
        agree(src),
        Ok(Value::List(std::sync::Arc::new(vec![
            Value::Integer(1),
            Value::Integer(9),
            Value::Integer(9),
            Value::Boolean(false),
            Value::Integer(0),
            s(""),
            Value::Integer(7),
        ])))
    );
}

#[test]
fn coalesce_is_map_get_or() {
    let src = r#"
fn main() = {
    let m = #{ "a": 1, "u": (), "l": [] }
    [m.a ?? 0, m.u ?? 0, m.x ?? 0, m.l ?? 0] == [map_get_or(m, "a", 0), map_get_or(m, "u", 0), map_get_or(m, "x", 0), map_get_or(m, "l", 0)]
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Boolean(true)));
}

#[test]
fn the_right_side_is_evaluated_only_when_needed() {
    // On every tier: a cell counts the right side's evaluations.
    let src = r#"
let hits = cell(0)
fn bump(v) = {
    cell.update(hits, (n) => n + 1)
    v
}
fn pick(m) = m.k ?? bump(5)
fn main() = {
    let a = pick(#{ "k": 1 })
    let b = pick(#{ "j": 1 })
    let c = pick(#{ "k": () })
    [a, b, c, cell.get(hits)]
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(Value::List(std::sync::Arc::new(vec![
            Value::Integer(1),
            Value::Integer(5),
            Value::Integer(5),
            Value::Integer(2),
        ])))
    );
}

#[test]
fn an_error_on_the_right_is_raised_only_when_taken() {
    let src = r#"
fn main() = {
    let m = #{ "k": 1 }
    m.k ?? (1 / 0)
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Integer(1)));
    let taken = r#"
fn main() = {
    let m = #{ "j": 1 }
    m.k ?? (1 / 0)
}
main()
"#;
    assert!(agree(taken).is_err());
}

#[test]
fn coalesce_precedence() {
    // below arithmetic, the pipeline and ranges; above comparisons
    let src = r#"
fn main() = {
    let m = #{ "n": 5 }
    let a = m.zz ?? 0 > 3          // (m.zz ?? 0) > 3
    let b = m.n ?? 0 > 3           // (m.n ?? 0) > 3
    let c = m.zz ?? 1 + 2          // m.zz ?? (1 + 2)
    let d = 1 + 2 ?? 9             // (1 + 2) ?? 9
    let e = [1, 2] |> len ?? 0     // ([1, 2] |> len) ?? 0
    let f = m.zz ?? 0 == 0 && true // ((m.zz ?? 0) == 0) && true
    let g = m.zz ?? m.yy ?? m.n    // chains
    [a, b, c, d, e, f, g]
}
main()
"#;
    assert_eq!(
        agree(src),
        Ok(Value::List(std::sync::Arc::new(vec![
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Integer(3),
            Value::Integer(3),
            Value::Integer(2),
            Value::Boolean(true),
            Value::Integer(5),
        ])))
    );
}

#[test]
fn coalesce_beside_try_and_across_lines() {
    let src = r#"
fn half(r) = {
    let v = r?
    Ok(v.x ?? 0)
}
fn main() = {
    let wide = #{ "a": () }.a
        ?? "fallback"
    [half(Ok(#{ "x": 4 })), half(Ok(#{})), wide]
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(Value::List(std::sync::Arc::new(vec![
            Value::Ok(Box::new(Value::Integer(4))),
            Value::Ok(Box::new(Value::Integer(0))),
            s("fallback"),
        ])))
    );
}

#[test]
fn coalesce_in_a_hot_loop_agrees() {
    let src = r#"
fn total(m, n) = {
    let mut acc = 0
    for i in 0..n { acc = acc + (m.step ?? 1) + (m.none ?? 2) }
    acc
}
fn main() = total(#{ "step": 3 }, 100) + total(#{ "other": 3 }, 100)
main()
"#;
    assert_eq!(agree(src), Ok(Value::Integer(500 + 300)));
}

// ── E5: one record kind for the builtins ────────────────────────────────

#[test]
fn every_map_builtin_takes_every_record_kind() {
    // Each builtin over a map, an object (small and wide, the VM's two
    // layouts) and a parsed JSON object, compiled: the answers are the
    // interpreter's on every tier.
    let src = r#"
fn kinds() = [
    #{ "a": 1, "b": 2 },
    { a: 1, b: 2 },
    { a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9, j: 10, k: 11, l: 12, m: 13, n: 14, o: 15, p: 16, q: [17] },
    unwrap(json.parse("{\"a\": 1, \"b\": 2}")),
]
fn probe(m) = [
    map_get(m, "a"), map_get_or(m, "z", 0), map_path(m, ["a"]), map_has_key(m, "b"),
    sort(map_keys(m)) |> take(2), len(map_values(m)), len(entries(m)), map_len(m),
    typeof(map_set(m, "z", 3)), map_len(map_remove(m, "a")),
    typeof(map_merge(m, #{ "c": 3 })), map_get(map_merge(m, #{ "a": 9 }), "a"),
    typeof(map_merge(#{ "c": 3 }, m)), map_len(map_merge(#{}, m)) == map_len(m),
    typeof(map_clear(m)), map_len(map_clear(m)),
    m.a, m.zz ?? "absent",
]
fn main() = map(kinds(), probe)
main()
"#;
    let got = agree_tiered(src).expect("every builtin answers");
    let Value::List(rows) = &got else { panic!() };
    assert_eq!(rows.len(), 4);
    for row in rows.iter() {
        let Value::List(cells) = row else { panic!() };
        assert!(
            matches!(cells[7], Value::Integer(_)),
            "map_len answered: {row:?}"
        );
        assert_eq!(cells[0], Value::Integer(1));
        assert_eq!(cells[17], s("absent"));
    }
    // the kinds survive the writers
    let kinds = r#"
fn main() = [
    typeof(map_merge({ a: 1 }, #{ "b": 2 })),
    typeof(map_merge(unwrap(json.parse("{}")), { b: 2 })),
    typeof(map_clear({ a: 1 })),
    map_len({ a: 1, b: 2, c: 3 }),
    map_len(unwrap(json.parse("{\"x\": [1, 2]}"))),
]
main()
"#;
    assert_eq!(
        agree_tiered(kinds),
        Ok(Value::List(std::sync::Arc::new(vec![
            s("Object"),
            s("JsonObject"),
            s("Object"),
            Value::Integer(3),
            Value::Integer(1),
        ])))
    );
}

#[test]
fn map_builtins_on_objects_compile_on_the_vm() {
    let src = r#"
fn main() = {
    let o = { a: 1, b: 2 }
    let merged = map_merge(o, #{ "c": 3 })
    [map_len(o), map_len(merged), map_len(map_clear(o)), merged.c]
}
main()
"#;
    assert_eq!(
        agree(src),
        Ok(Value::List(std::sync::Arc::new(vec![
            Value::Integer(2),
            Value::Integer(3),
            Value::Integer(0),
            Value::Integer(3),
        ])))
    );
}

#[test]
fn the_builtins_still_refuse_what_is_not_a_record() {
    for call in [
        "map_len(5)",
        "map_merge(5, #{})",
        "map_merge(#{}, [1])",
        "map_clear(\"x\")",
    ] {
        let src = format!("fn main() = {call}\nmain()\n");
        let err = agree(&src).unwrap_err();
        assert!(err.contains("must be a map or object"), "{call}: {err}");
    }
}

// ── E6: spreads and nested updates ──────────────────────────────────────

fn list(items: Vec<Value>) -> Value {
    Value::List(std::sync::Arc::new(items))
}

#[test]
fn spreads_copy_with_the_later_key_winning() {
    let src = r#"
fn main() = {
    let m = #{ "a": 1, "b": 2 }
    let o = { x: 1, y: 2 }
    let over = #{ ...m, "a": 5, "c": 3 }
    let under = #{ "a": 5, ...m }
    let obj = { ...o, y: 9, z: 0 }
    let mixed = #{ ...m, ...o, "k": [1] }
    [over.a, over.b, over.c, under.a, obj.y, obj.z, typeof(obj), typeof(#{ ...o }), typeof({ ...m }),
     len(map_keys(mixed)), m.c, map_len(o)]
}
main()
"#;
    assert_eq!(
        agree(src),
        Ok(list(vec![
            Value::Integer(5),
            Value::Integer(2),
            Value::Integer(3),
            Value::Integer(1),
            Value::Integer(9),
            Value::Integer(0),
            s("Object"),
            s("Map"),
            s("Object"),
            Value::Integer(5),
            Value::Unit,
            Value::Integer(2),
        ]))
    );
}

#[test]
fn spreads_evaluate_in_source_order_and_refuse_non_records() {
    let src = r#"
let log = cell([])
fn note(tag, v) = {
    cell.update(log, (l) => l + [tag])
    v
}
fn main() = {
    let r = #{ "a": note("a", 1), ...note("m", #{ "b": 2 }), "c": note("c", 3) }
    [cell.get(log), r.b]
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(list(vec![
            list(vec![s("a"), s("m"), s("c")]),
            Value::Integer(2)
        ]))
    );
    let err = agree("fn main() = #{ ...5 }\nmain()\n").unwrap_err();
    assert!(
        err.contains("a spread `...` takes a map or an object, got Int"),
        "{err}"
    );
}

#[test]
fn nested_reads_and_writes() {
    let src = r#"
fn main() = {
    let s = #{ "pp": #{ "cur": 1, "list": [1] }, "o": { k: 1 } }
    let a = map_set_in(s, ["pp", "cur"], 2)
    let b = map_set_in(s, ["qq", "new"], 3)
    let c = map_set_in(s, ["o", "k"], 4)
    let d = map_update(s, "n", (v) => (v ?? 0) + 1)
    let e = map_update(map_update(#{ "n": 1 }, "n", (v) => v + 1), "n", (v) => v * 10)
    [a.pp.cur, s.pp.cur, b.qq.new, typeof(c.o), c.o.k, d.n, e.n,
     map_get_in(a, ["pp", "cur"]), map_get_in(s, ["pp", "none", "x"]), map_get_in(s, [])]
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(list(vec![
            Value::Integer(2),
            Value::Integer(1),
            Value::Integer(3),
            s("Object"),
            Value::Integer(4),
            Value::Integer(1),
            Value::Integer(20),
            Value::Integer(2),
            Value::Unit,
            Value::Map(std::sync::Arc::new({
                let mut m = olang::ast::ValueMap::default();
                let mut pp = olang::ast::ValueMap::default();
                pp.insert("cur".into(), Value::Integer(1));
                pp.insert("list".into(), list(vec![Value::Integer(1)]));
                m.insert("pp".into(), Value::Map(std::sync::Arc::new(pp)));
                let mut o = olang::ast::ValueMap::default();
                o.insert("k".into(), Value::Integer(1));
                m.insert(
                    "o".into(),
                    Value::Struct {
                        type_name: "Object".into(),
                        fields: std::sync::Arc::new(o),
                    },
                );
                m
            })),
        ]))
    );
    for (call, says) in [
        (
            "map_set_in(#{ \"a\": 1 }, [\"a\", \"b\"], 2)",
            "cannot set key 'b' in Int",
        ),
        ("map_set_in(#{}, [], 2)", "the path is empty"),
        (
            "map_update(5, \"k\", (v) => v)",
            "first argument must be a map or object",
        ),
    ] {
        let err = agree(&format!("fn main() = {call}\nmain()\n")).unwrap_err();
        assert!(err.contains(says), "{call}: {err}");
    }
}

#[test]
fn nested_writes_compile_on_the_vm() {
    let src = r#"
fn bump(s) = map_set_in(s, ["pp", "cur"], s.pp.cur + 1)
fn main() = {
    let mut s = #{ "pp": #{ "cur": 0 } }
    for i in 0..30 { s = bump(s) }
    let t = #{ ...s, "pp": #{ ...s.pp, "seen": true } }
    [s.pp.cur, t.pp.seen, map_get_in(t, ["pp", "cur"])]
}
main()
"#;
    assert_eq!(
        agree(src),
        Ok(list(vec![
            Value::Integer(30),
            Value::Boolean(true),
            Value::Integer(30)
        ]))
    );
}

#[test]
fn map_update_takes_a_lambda_on_every_tier() {
    let src = r#"
fn main() = {
    let mut s = #{ "pp": #{ "cur": 0 }, "log": [] }
    for i in 0..30 {
        s = map_update(s, "log", (l) => l + [i])
        s = map_update(s, "pp", (pp) => #{ ...pp, "cur": pp.cur + 1 })
    }
    [s.pp.cur, len(s.log)]
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(list(vec![Value::Integer(30), Value::Integer(30)]))
    );
}

#[test]
fn the_parser_names_an_unsupported_spread() {
    for (src, says) in [
        (
            "let xs = [1]\nlet ys = [...xs]\n",
            "a spread `...` is not supported here",
        ),
        (
            "fn f(a) = a\nlet xs = [1]\nf(...xs)\n",
            "a spread `...` is not supported here",
        ),
        (
            "type P = struct { x: Int }\nlet p = P { x: 1 }\nlet q = P { ...p, x: 2 }\n",
            "not supported in a declared struct's literal",
        ),
    ] {
        let err = olang::Parser::new().parse(src).unwrap_err().to_string();
        assert!(err.contains(says), "{src}: {err}");
        assert!(!err.contains("Unclosed"), "{src}: {err}");
    }
}

// ── a module reads like a record ───────────────────────────────────────

#[test]
fn a_module_member_read_is_unit_when_absent_and_a_call_raises() {
    let src = r#"
fn main() = [str.trimm == (), str.trim == (), map_get(str, "trimm") == ()]
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(list(vec![
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Boolean(true)
        ]))
    );
    for call in [
        "str.trimm(\" x\")",
        "\" x\" |> str.trimm",
        "\" x\" |> str.trimm()",
    ] {
        let err = agree_tiered(&format!("fn main() = {call}\nmain()\n")).unwrap_err();
        assert!(err.contains("did you mean 'trim'"), "{call}: {err}");
    }
}

#[test]
fn nested_reads_are_native_on_the_vm() {
    let src = r#"
fn main() = {
    let s = #{ "pp": #{ "cur": 3, "o": { k: [1, 2] } }, "n": 1 }
    [map_get_in(s, ["pp", "cur"]), map_get_in(s, ["pp", "o", "k"]), map_get_in(s, ["pp", "none", "x"]),
     map_path(s, ["n"]), map_get_in(s, [])]
}
main()
"#;
    let got = agree(src).expect("reads");
    let Value::List(items) = &got else { panic!() };
    assert_eq!(items[0], Value::Integer(3));
    assert_eq!(items[2], Value::Unit);
    assert_eq!(items[3], Value::Integer(1));
    let err = agree("fn main() = map_get_in(#{ \"a\": 1 }, [\"a\", \"b\"])\nmain()\n").unwrap_err();
    assert!(err.contains("cannot read key 'b'"), "{err}");
}

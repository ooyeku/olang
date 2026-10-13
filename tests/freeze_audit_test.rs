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

// ── The four tier disagreements ────────────────────────────────────────

#[test]
fn an_integer_literal_pattern_does_not_match_a_float() {
    let src = r#"
fn kind(x) = match x { 5 => "int five", _ => "other" }
fn pair(x) = match x { [1, 2] => "list", _ => "other" }
fn tup(x) = match x { (1, y) => y, _ => "other" }
fn main() = [kind(5.0), kind(5), pair([1.0, 2]), pair([1, 2]), tup((1.0, 9)), tup((1, 9))]
main()
"#;
    assert_eq!(
        agree(src),
        Ok(l(vec![
            s("other"),
            s("int five"),
            s("other"),
            s("list"),
            s("other"),
            Value::Integer(9),
        ]))
    );
}

#[test]
fn a_let_bound_lambda_calls_itself() {
    let src = r#"
fn main() = {
    let fact = (k) => if k <= 1 => 1 else => k * fact(k - 1)
    fact(5)
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Integer(120)));
}

#[test]
fn a_let_bound_lambda_that_escapes_still_calls_itself() {
    let src = r#"
fn make() = {
    let count = (k) => if k == 0 => "done" else => count(k - 1)
    count
}
fn main() = make()(3)
main()
"#;
    assert_eq!(agree_tiered(src), Ok(s("done")));
}

#[test]
fn a_lambda_naming_an_earlier_binding_keeps_capturing_it() {
    // Only a name nothing else binds is the lambda's own: an earlier
    // binding of the name is captured, as every free name is.
    let src = r#"
fn main() = {
    let f = (k) => k + 100
    let f = (k) => if k == 0 => f(5) else => 0
    f(0)
}
main()
"#;
    assert_eq!(agree(src), Ok(Value::Integer(105)));
}

#[test]
fn a_float_range_is_refused_on_every_tier() {
    let src = r#"
fn count(a) = {
    let mut t = 0
    for i in a..3 { t = t + 1 }
    t
}
fn main() = count(0.5)
main()
"#;
    let err = agree(src).expect_err("a Float bound is refused");
    assert!(err.contains("Range start must be an integer"), "{err}");
    let src = r#"
fn count(b) = {
    let mut t = 0
    for i in 0..b { t = t + 1 }
    t
}
fn main() = [count(3), count(0)]
main()
"#;
    assert_eq!(
        agree(src),
        Ok(l(vec![Value::Integer(3), Value::Integer(0)]))
    );
    let src = r#"
fn count(b) = {
    let mut t = 0
    for i in 0..b { t = t + 1 }
    t
}
fn main() = count(2.5)
main()
"#;
    let err = agree(src).expect_err("a Float bound is refused");
    assert!(err.contains("Range end must be an integer"), "{err}");
}

#[test]
fn a_cell_inside_a_container_compares_by_identity() {
    let src = r#"
fn main() = {
    let c = cell.new(1)
    let d = cell.new(1)
    [c == c, c == d, [c] == [c], [c] == [d], #{ "a": c } == #{ "a": c }, (c, 1) == (c, 1)]
}
main()
"#;
    let t = Value::Boolean(true);
    let f = Value::Boolean(false);
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![t.clone(), f.clone(), t.clone(), f, t.clone(), t]))
    );
}

// ── The naming decisions: find, reduce, re.replace ─────────────────────

#[test]
fn find_answers_the_element_or_unit() {
    let src = r#"
fn main() = [find([3, 8, 2], (x) => x > 5), find([3, 8, 2], (x) => x > 9)]
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![Value::Integer(8), Value::Unit]))
    );
}

#[test]
fn reduce_starts_from_the_first_element() {
    let src = r#"
fn main() = [
    reduce([3, 9, 4], (a, b) => if a > b => a else => b),
    reduce([7], (a, b) => a + b),
    reduce([], (a, b) => a + b),
    reduce(1..=4, (a, b) => a * b),
    reduce(0..0, (a, b) => a + b),
    fold([1, 2], 10, (a, b) => a + b)
]
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            Value::Integer(9),
            Value::Integer(7),
            Value::Unit,
            Value::Integer(24),
            Value::Unit,
            Value::Integer(13),
        ]))
    );
    let err = agree_tiered("fn main() = reduce([1], 0, (a, b) => a + b)\nmain()\n")
        .expect_err("an initial value is an arity error");
    assert!(err.contains('2') || err.contains("argument"), "{err}");
}

#[test]
fn re_replace_replaces_every_match_and_replace_first_one() {
    let src = r#"
fn main() = [
    unwrap(re.replace("[0-9]", "a1b1", "X")),
    unwrap(re.replace_first("[0-9]", "a1b1", "X")),
    unwrap(re.replace_all("[0-9]", "a1b1", "X"))
]
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![s("aXbX"), s("aXb1"), s("aXbX")]))
    );
}

// ── Language rulings ───────────────────────────────────────────────────

fn parse_err(src: &str) -> String {
    match Parser::new().parse(src) {
        Ok(_) => panic!("expected a parse error\n{src}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_function_declared_twice_in_a_file_is_refused() {
    let err =
        agree_tiered("fn f() = 1\nfn f() = 2\nfn main() = f()\nmain()\n").expect_err("refused");
    assert!(err.contains("'f' is already declared"), "{err}");
}

#[test]
fn bitwise_operators_take_rusts_order() {
    let src = r#"
fn main() = [1 + 2 << 3, 1 << 4 * 2, 6 & 3 == 2, 1 | 2 ^ 3, 12 & 10 | 1, 1 | 6 & 3]
main()
"#;
    let t = Value::Boolean(true);
    assert_eq!(
        agree(src),
        Ok(l(vec![
            Value::Integer(24),
            Value::Integer(256),
            t,
            Value::Integer(1),
            Value::Integer(9),
            Value::Integer(3),
        ]))
    );
}

#[test]
fn a_line_starting_with_minus_or_plus_is_a_new_statement() {
    let src = "fn main() = {\n    let base = 10\n    let shown = base\n    -1\n    let sum = base +\n        5\n    let piped = [1, 2]\n        |> len\n    (shown, sum, piped)\n}\nmain()\n";
    assert_eq!(
        agree(src),
        Ok(Value::Tuple(std::sync::Arc::new(vec![
            Value::Integer(10),
            Value::Integer(15),
            Value::Integer(2),
        ])))
    );
}

#[test]
fn a_block_ending_in_let_or_semicolon_is_unit_and_an_empty_body_too() {
    let src = r#"
fn empty() = {}
fn main() = [{ let a = 1 }, { 1; }, empty(), { 1 }]
main()
"#;
    assert_eq!(
        agree(src),
        Ok(l(vec![
            Value::Unit,
            Value::Unit,
            Value::Unit,
            Value::Integer(1)
        ]))
    );
}

#[test]
fn comparisons_do_not_chain() {
    for src in ["1 == 1 == 1", "1 < 2 < 3", "1 < 2 == true"] {
        assert!(parse_err(src).contains("comparisons do not chain"), "{src}");
    }
    assert_eq!(agree_tiered("(1 < 2) == true\n"), Ok(Value::Boolean(true)));
}

#[test]
fn a_named_argument_to_a_builtin_is_refused() {
    let err = agree_tiered(r#"str.pad_start("x", 3, bogus: "0")"#).expect_err("refused");
    assert!(err.contains("named argument 'bogus'"), "{err}");
}

#[test]
fn pattern_rules() {
    let err = agree_tiered("let (x, x) = (1, 2)\n").expect_err("a binder twice");
    assert!(err.contains("'x' is bound twice"), "{err}");
    let err = agree_tiered("match (1, 2) { (1, y) | (z, 2) => 1, _ => 0 }\n")
        .expect_err("alternatives bind different names");
    assert!(err.contains("bind different names"), "{err}");
    assert_eq!(
        agree_tiered("match (1, 2) { (1, y) | (y, 2) => y, _ => 0 }\n"),
        Ok(Value::Integer(2))
    );
    let src = r#"
fn kind(c) = match c { 'a'..='z' => "letter", _ => "other" }
fn main() = [kind("q"), kind("zebra"), kind("")]
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![s("letter"), s("other"), s("other")]))
    );
    assert!(parse_err("type E = enum { C { n: Int } }").contains("named fields"));
    let err = agree_tiered("match 3 { 1 => 0 }\n").expect_err("no arm");
    assert!(err.contains("Pattern match failed"), "{err}");
}

#[test]
fn unit_tuple_and_lambda_return_annotations_are_enforced() {
    let cases = [
        ("fn f(x: ()) = x\nf(1)\n", "expects ()"),
        ("fn f(x: Int | ()) = x\nf(\"s\")\n", "expects Int | ()"),
        ("fn f(p: (Int, Int)) = p\nf((1, 2, 3))\n", "a 3-tuple"),
        ("let g = (x) -> Int => \"s\"\ng(1)\n", "return value"),
    ];
    for (src, want) in cases {
        let err = agree_tiered(src).expect_err(src);
        assert!(err.contains(want), "{src}: {err}");
    }
    assert_eq!(
        agree_tiered("fn f(x: Int | ()) = x\n[f(()), f(2)]\n"),
        Ok(l(vec![Value::Unit, Value::Integer(2)]))
    );
    assert_eq!(
        agree_tiered("fn f(p: (Int, Int)) -> (Int, Int) = p\nf((1, 2))\n"),
        Ok(Value::Tuple(std::sync::Arc::new(vec![
            Value::Integer(1),
            Value::Integer(2)
        ])))
    );
}

#[test]
fn numbers_at_their_edges() {
    let err = agree_tiered("to_int(1e30)\n").expect_err("out of range");
    assert!(err.contains("outside the Int range"), "{err}");
    assert_eq!(agree_tiered("to_int(-2.9)\n"), Ok(Value::Integer(-2)));
    let src = "fn shift(a, n) = a << n\nfn main() = shift(3, 63)\nmain()\n";
    let err = agree(src).expect_err("a shift that loses bits overflows");
    assert!(err.contains("overflow"), "{err}");
    assert_eq!(
        agree(
            "fn shift(a, n) = a << n\nfn main() = [shift(1, 62), shift(-1, 63), -7 % 3, 7 % -3]\nmain()\n"
        ),
        Ok(l(vec![
            Value::Integer(1 << 62),
            Value::Integer(i64::MIN),
            Value::Integer(-1),
            Value::Integer(1),
        ]))
    );
}

#[test]
fn equality_is_one_relation() {
    let src = r#"
fn main() = [
    Ok(1) == Ok(1), Ok(1) == Err(1), Ok([1]) != Ok([2]),
    1 == 1.0, 9007199254740993 == 9007199254740992.0, [1] == [1.0],
    contains([1, 2], 1.0), col.index_of([1, 2], 2.0)
]
main()
"#;
    let (t, f) = (Value::Boolean(true), Value::Boolean(false));
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            t.clone(),
            f.clone(),
            t.clone(),
            t.clone(),
            f.clone(),
            f,
            t,
            Value::Integer(1)
        ]))
    );
    for src in [
        "fn g() = 1\n[g] == [g]\n",
        "fn g() = 1\nOk(g) == Ok(g)\n",
        "#{ \"r\": 0..2 } == #{ \"r\": 0..2 }\n",
        "fn g() = 1\ncontains([g], g)\n",
    ] {
        let err = agree_tiered(src).expect_err(src);
        assert!(err.contains("cannot compare"), "{src}: {err}");
    }
}

#[test]
fn the_parser_bugs() {
    let src = r#"
type Integer = struct { v: Int }
type Mapping = struct { }
fn f(x: Integer) -> Int = x.v
fn g(m: Mapping) = 1
fn main() = {
    let mut n = 0
    for x in [1, 2] {}
    while false {}
    [f(Integer { v: 3 }), g(Mapping {}), -9223372036854775808]
}
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            Value::Integer(3),
            Value::Integer(1),
            Value::Integer(i64::MIN)
        ]))
    );
}

#[test]
fn meta_rename_renames_values_and_keeps_fields() {
    let src = r##"
let src = "fn helper(x) = x + 1\nlet r = { helper: helper(1) }\nfn use_it(helper) = `${helper(2)}`\nprintln(r.helper)\n"
unwrap(meta.rename(src, #{ "helper": "h2" }))
"##;
    assert_eq!(
        agree_tiered(src),
        Ok(s(
            "fn h2(x) = x + 1\nlet r = { helper: h2(1) }\nfn use_it(h2) = `${h2(2)}`\nprintln(r.helper)\n"
        ))
    );
}

// ── A field of () reads () ─────────────────────────────────────────────

#[test]
fn a_field_of_unit_reads_unit() {
    let src = r#"
fn chain(s) = s.pp.cur
fn main() = [chain(#{}), chain(#{ "pp": #{ "cur": 3 } }), ().k ?? 5, map_get((), "k"), map_get_or((), "k", 7)]
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            Value::Unit,
            Value::Integer(3),
            Value::Integer(5),
            Value::Unit,
            Value::Integer(7),
        ]))
    );
    let err = agree_tiered("fn f(x) = x.k\nf(5)\n").expect_err("a field of an Int raises");
    assert!(err.contains("Cannot access field"), "{err}");
    let err = agree_tiered("type P = struct { x: Int }\nfn f(p) = p.y\nf(P { x: 1 })\n")
        .expect_err("a struct's missing field raises");
    assert!(err.contains("no field"), "{err}");
    assert!(parse_err("let r = Ok(1)\nr?.name\n").contains("`?.` is reserved"));
    assert_eq!(
        agree_tiered(
            "let r: Result<Map<String, Int>, String> = Ok(#{ \"a\": 1 })\nfn f(r) = (r?).a\nmatch f(r) { x => x }\n"
        ),
        Ok(Value::Integer(1))
    );
}

// ── A negative index raises ────────────────────────────────────────────

#[test]
fn a_negative_index_raises_and_first_and_last_read_the_ends() {
    let src = r#"
fn at(xs, i) = xs[i]
fn main() = [at([1, 2, 3], 0), last([1, 2, 3]), first([1, 2, 3]), last([]), first(""), last("abc")]
main()
"#;
    assert_eq!(
        agree(src),
        Ok(l(vec![
            Value::Integer(1),
            Value::Integer(3),
            Value::Integer(1),
            Value::Unit,
            Value::Unit,
            s("c"),
        ]))
    );
    for src in [
        "fn at(xs, i) = xs[i]\nfn main() = at([1, 2, 3], -1)\nmain()\n",
        "fn at(xs, i) = xs[i]\nfn main() = at((1, 2), -1)\nmain()\n",
        "fn at(xs, i) = xs[i]\nfn main() = at(\"abc\", -1)\nmain()\n",
        "fn put(i) = {\n    let mut xs = [1, 2]\n    xs[i] = 9\n    xs\n}\nfn main() = put(-1)\nmain()\n",
    ] {
        let err = agree(src).expect_err(src);
        assert!(err.contains("out of bounds"), "{src}: {err}");
    }
    for src in [
        "str.substring(\"abc\", -1, 2)\n",
        "str.char_at(\"abc\", -1)\n",
        "col.slice([1, 2, 3], -1, 2)\n",
        "bytes.slice(bytes.from_list([1, 2]), -1, 2)\n",
        "col.set([1, 2], -1, 0)\n",
    ] {
        let err = agree_tiered(src).expect_err(src);
        assert!(
            err.contains("negative") || err.contains("out of bounds"),
            "{src}: {err}"
        );
    }
    assert_eq!(
        agree_tiered(
            "[str.substring(\"abc\", 1, 99), str.substring(\"abc\", 2, 1), str.char_at(\"abc\", 9)]\n"
        ),
        Ok(l(vec![s("bc"), s(""), Value::Unit]))
    );
    assert_eq!(
        agree_tiered("col.slice([1, 2, 3], 1, 99)\n"),
        Ok(l(vec![Value::Integer(2), Value::Integer(3)]))
    );
}

// ── A condition is a Bool ──────────────────────────────────────────────

#[test]
fn a_condition_is_a_bool() {
    let src = r#"
fn pick(c) = if c => "yes" else => "no"
fn main() = [pick(true), pick(false)]
main()
"#;
    assert_eq!(agree(src), Ok(l(vec![s("yes"), s("no")])));
    for src in [
        "fn pick(c) = if c => 1 else => 2\nfn main() = pick([])\nmain()\n",
        "fn pick(c) = if c => 1 else => 2\nfn main() = pick(())\nmain()\n",
        "fn spin(c) = {\n    while c { return 1 }\n    2\n}\nfn main() = spin(1)\nmain()\n",
        "fn guard(x) = match x { y if y => 1, _ => 2 }\nfn main() = guard(\"s\")\nmain()\n",
    ] {
        let err = agree(src).expect_err(src);
        assert!(err.contains("a condition must be a Bool"), "{src}: {err}");
    }
}

// ── Printing writes the source form ────────────────────────────────────

#[test]
fn printing_writes_the_source_form() {
    let src = r#"
type PrintedPoint = struct { y: Int, x: Int }
type Shape = enum { Dot, Circle(Float) }
fn shown(v) = show(v)
fn main() = [
    shown([PrintedPoint { x: 1, y: 2 }, Circle(2.0), Dot]),
    shown(#{ "k": ["a\"b\n"] }),
    shown({ b: 1, a: "x" }),
    shown(unwrap(json.parse("{\"a b\": 1}"))),
    shown("top"),
    `${[1, "s"]}`,
    shown([-0.0, 1e20, 0.00001, 1500.0])
]
main()
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            s("[PrintedPoint { y: 2, x: 1 }, Circle(2.0), Dot]"),
            s("#{\"k\": [\"a\\\"b\\n\"]}"),
            s("{ a: \"x\", b: 1 }"),
            s("#{\"a b\": 1}"),
            s("top"),
            s("[1, \"s\"]"),
            s("[-0.0, 1e20, 1e-5, 1500.0]"),
        ]))
    );
}

// ── Reserved words ─────────────────────────────────────────────────────

#[test]
fn the_reserved_words_are_refused_as_names_and_allowed_as_members() {
    for word in ["await", "yield", "async", "defer", "const", "pub"] {
        for src in [format!("let {word} = 1\n"), format!("fn {word}(x) = x\n")] {
            let err = parse_err(&src);
            assert!(
                err.contains(&format!("`{word}` is a reserved word")),
                "{src}: {err}"
            );
        }
    }
    assert_eq!(
        agree_tiered("let m = #{ \"const\": 2 }\n[typeof(http.defer), m.const]\n"),
        Ok(l(vec![s("Builtin"), Value::Integer(2)]))
    );
}

// ── task.join answers a Result ─────────────────────────────────────────

#[test]
fn task_join_tells_a_raise_from_a_returned_err() {
    let src = r#"
let ok = spawn 7
let bad = spawn Err("bad")
let raised = spawn unwrap(Err("boom"))
let timed = task.join_timeout(spawn { time.sleep(300); 1 }, 10)
[task.join(ok), task.join(bad), match task.join(raised) { Err(e) => str.contains(e, "boom"), Ok(v) => false },
 task.join_timeout(ok, 1000), timed]
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            ok(Value::Integer(7)),
            ok(Value::Err(Box::new(s("bad")))),
            Value::Boolean(true),
            ok(ok(Value::Integer(7))),
            Value::Err(Box::new(s("timed out"))),
        ]))
    );
}

// ── math: rounding answers an Int ──────────────────────────────────────

#[test]
fn rounding_answers_an_int_on_every_tier() {
    // A hot loop compiles the rounding natively; the last call's Float is
    // outside the Int range, so native code must hand it back to raise.
    let src = r#"
fn whole(x) = math.floor(x) + math.ceil(x) + math.round(x) + math.trunc(x)
fn main() = {
    let mut acc = 0
    for i in range(0, 2000) { acc = acc + whole(to_float(i) / 4.0 - 100.0) + whole(i) }
    [acc, math.round(2.5), math.round(-2.5), math.floor(9007199254740993), math.pow(2, 10),
     math.pow(2.0, 10), math.sign(-2.5), math.min(1, 2.5), math.max([3, 9, 4]), math.min(2.0, 2),
     ]
}
main()
"#;
    assert_eq!(
        agree(src),
        Ok(l(vec![
            Value::Integer(9_194_700),
            Value::Integer(3),
            Value::Integer(-3),
            Value::Integer(9_007_199_254_740_993),
            Value::Integer(1024),
            Value::Float(1024.0),
            Value::Integer(-1),
            Value::Integer(1),
            Value::Integer(9),
            Value::Float(2.0),
        ]))
    );
    // Warmed, then handed a Float outside the Int range: native code
    // hands it back, and the module raises.
    let raising = r#"
fn whole(x) = math.floor(x) + math.ceil(x) + math.round(x) + math.trunc(x)
let mut acc = 0
for i in range(0, 2000) { acc = acc + whole(to_float(i) / 4.0) }
[match attempt(() => whole(1e300)) { Err(e) => e, Ok(v) => "no" },
 match attempt(() => math.pow(2, 64)) { Err(e) => e, Ok(v) => "no" }]
"#;
    assert_eq!(
        agree_tiered(raising),
        Ok(l(vec![
            s("math.floor: 1e300 is outside the Int range"),
            s("math.pow: integer overflow"),
        ]))
    );
}

#[test]
fn vec_sum_of_ints_is_an_int() {
    assert_eq!(
        agree_tiered(
            "[vec.sum([1, 2, 3]), vec.sum([]), vec.sum([1, 2.5]), vec.mean([2, 4]), str.fixed(2.5, 0)]\n"
        ),
        Ok(l(vec![
            Value::Integer(6),
            Value::Integer(0),
            Value::Float(3.5),
            Value::Float(3.0),
            s("2")
        ]))
    );
}

// ── stdlib records: data-keyed are Maps, results plain objects ─────────

#[test]
fn stdlib_records_are_maps_or_plain_objects() {
    let src = r#"
let x = unwrap(os.exec("sh", ["-c", "exit 3"]))
let w = unwrap(proc.wait(unwrap(proc.spawn("sh", ["-c", "kill -9 $$"]))))
let u = unwrap(http.parse_url("http://a.example/x"))
let r = http.response(404, "no")
[typeof(http.decode_query("a=1")), http.decode_query("a=1").b ?? "d", typeof(os.list_env()),
 typeof(unwrap(csv.parse_with_headers("a\n1\n"))[0]), typeof(x), x.code, x.signal, x.timed_out,
 typeof(w), w.code, w.signal, typeof(u), u.port, u.query, typeof(r), typeof(r.headers), r.success,
 typeof(unwrap(fs.file_info("."))), typeof(unwrap(proc.pipeline([["echo", "a"]])))]
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            s("Map"),
            s("d"),
            s("Map"),
            s("Map"),
            s("Object"),
            Value::Integer(3),
            Value::Unit,
            Value::Boolean(false),
            s("Object"),
            Value::Unit,
            Value::Integer(9),
            s("Object"),
            Value::Unit,
            Value::Unit,
            s("HttpResponse"),
            s("Map"),
            Value::Boolean(false),
            s("Object"),
            s("Object"),
        ]))
    );
}

// ── absence is (), "nothing yet" is Ok(()) ─────────────────────────────

#[test]
fn absence_is_unit_and_nothing_yet_is_ok_unit() {
    let src = r#"
let c = chan.new()
let empty = chan.try_recv(c)
chan.close(c)
[re.find("z+", "abc"), re.captures("z", "abc"), re.captures("(x)?b", "b"), str.char_at("ab", 2),
 empty, chan.try_recv(c), chan.recv_timeout(chan.new(), 1), os.env("OLANG_FREEZE_SURELY_UNSET"),
 is_err(os.get_env("OLANG_FREEZE_SURELY_UNSET"))]
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            ok(Value::Unit),
            ok(Value::Unit),
            ok(l(vec![s("b"), Value::Unit])),
            Value::Unit,
            ok(Value::Unit),
            Value::Err(Box::new(s("channel is closed"))),
            Value::Err(Box::new(s("timed out"))),
            Value::Unit,
            Value::Boolean(true),
        ]))
    );
}

// ── dates: RFC 3339 out, read back, milliseconds ───────────────────────

#[test]
fn dates_write_rfc3339_and_read_their_own_output() {
    let src = r#"
let dt = unwrap(dates.datetime(2026, 1, 2, 3, 4, 5))
[dt, unwrap(dates.parse_datetime(dt)), unwrap(dates.parse_datetime("2026-01-02 03:04:05")),
 unwrap(dates.parse_datetime("2026-01-02T03:04:05+02:00")), unwrap(dates.from_timestamp(1700000000123)),
 unwrap(dates.from_timestamp_s(1700000000)), unwrap(dates.timestamp("1970-01-02")),
 unwrap(dates.timestamp_s("1970-01-02T00:00:00+00:00")), is_ok(dates.parse_datetime(dates.now())),
 is_ok(dates.parse_datetime(dates.utc_now())), is_ok(dates.parse_datetime(dates.stamp())),
 typeof(dates.today()), typeof(unwrap(dates.parse(dt))), unwrap(dates.weekday("2026-10-11")),
 unwrap(dates.diff_days("2026-01-05", "2026-01-02"))]
"#;
    assert_eq!(
        agree_tiered(src),
        Ok(l(vec![
            s("2026-01-02T03:04:05+00:00"),
            s("2026-01-02T03:04:05+00:00"),
            s("2026-01-02T03:04:05+00:00"),
            s("2026-01-02T03:04:05+02:00"),
            s("2023-11-14T22:13:20.123+00:00"),
            s("2023-11-14T22:13:20+00:00"),
            Value::Integer(86_400_000),
            Value::Integer(86_400),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            s("String"),
            s("Date"),
            Value::Integer(0),
            Value::Integer(3),
        ]))
    );
}

// ── assignment answers () ──────────────────────────────────────────────

#[test]
fn an_assignment_answers_unit_on_every_tier() {
    let src = r#"
fn ends_in_assignment() = { let mut x = 1; x = 7 }
fn bound() = { let mut x = 1; let y = (x = 5); [y, x] }
fn branch() = { let mut m = #{}; let r = if true => { m = map_set(m, "a", 1) } else => (); [r, m] }
fn indexed() = { let mut xs = [1]; xs[0] = 9 }
fn appended() = { let mut xs = [1]; xs = xs + [2] }
fn hot() = {
    let mut i = 0
    let mut last = 0
    while i < 3000 { last = (i = i + 1) }
    [last, i]
}
fn main() = [ends_in_assignment(), bound(), branch(), indexed(), appended(), hot()]
main()
"#;
    assert_eq!(
        agree(src),
        Ok(l(vec![
            Value::Unit,
            l(vec![Value::Unit, Value::Integer(5)]),
            l(vec![
                Value::Unit,
                eval("#{ \"a\": 1 }", None).expect("a map"),
            ]),
            Value::Unit,
            Value::Unit,
            l(vec![Value::Unit, Value::Integer(3000)]),
        ]))
    );
}

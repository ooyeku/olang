//! `meta` for an index (OL-435, OL-436, OL-455, OL-456, OL-457): items
//! one at a time, recovery from a syntax error, names resolved and
//! signatures typed by the language server's own code, fingerprints.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str) -> Value {
    let program = Parser::new().parse(source).expect("parses");
    Interpreter::new().eval_program(program).expect("runs")
}

fn s(text: &str) -> Value {
    Value::String(std::sync::Arc::new(text.to_string()))
}

fn lit(text: &str) -> String {
    format!("{text:?}")
}

const FILE: &str =
    "/// adds\nfn f(a) = a + 1\n\nfn g( = 3\n\nlet h = f(2)\ntest \"t\" { assert_eq(h, 3) }\n";

#[test]
fn parse_items_answers_each_items_range_and_kind_without_a_parse() {
    let src = format!(
        r#"map(meta.parse_items({}), (i) => [i.kind, i.name ?? "", i.start, i.end, i.line])"#,
        lit(FILE)
    );
    let g = FILE.find("fn g").unwrap();
    let h = FILE.find("let h").unwrap();
    let t = FILE.find("test").unwrap();
    let row = |k: &str, n: &str, a: usize, b: usize, l: i64| {
        Value::List(std::sync::Arc::new(vec![
            s(k),
            s(n),
            Value::Integer(a as i64),
            Value::Integer(b as i64),
            Value::Integer(l),
        ]))
    };
    assert_eq!(
        eval(&src),
        Value::List(std::sync::Arc::new(vec![
            row("fn", "f", 0, g, 1),
            row("fn", "g", g, h, 4),
            row("let", "h", h, t, 6),
            row("test", "t", t, FILE.len(), 7),
        ]))
    );
}

#[test]
fn a_file_mid_edit_keeps_its_other_items() {
    let src = format!(
        r#"let r = meta.parse({}, #{{"recover": true}})
[map(r.items, (n) => n.name ?? n.kind), len(r.errors), first(r.errors).line, first(r.errors).start]"#,
        lit(FILE)
    );
    let g = FILE.find("fn g").unwrap() as i64;
    assert_eq!(
        eval(&src),
        Value::List(std::sync::Arc::new(vec![
            Value::List(std::sync::Arc::new(vec![s("f"), s("h"), s("t")])),
            Value::Integer(1),
            Value::Integer(4),
            Value::Integer(g),
        ]))
    );
}

#[test]
fn an_item_reparsed_alone_has_the_files_spans() {
    let h = FILE.find("let h").unwrap();
    let item = "let h = f(2)\n";
    let src = format!(
        r#"let alone = first(unwrap(meta.parse({}, #{{"byte_offset": {h}, "line_offset": 5}})))
let whole = first(filter(meta.parse({}, #{{"recover": true}}).items, (n) => n.name == "h"))
[alone.start, alone.end, alone.line, alone.value.start, alone.name_span.start,
 whole.start == alone.start, whole.value.callee.end == alone.value.callee.end]"#,
        lit(item),
        lit(FILE)
    );
    assert_eq!(
        eval(&src),
        Value::List(std::sync::Arc::new(vec![
            Value::Integer(h as i64),
            Value::Integer((h + 12) as i64),
            Value::Integer(6),
            Value::Integer((h + 8) as i64),
            Value::Integer((h + 4) as i64),
            Value::Boolean(true),
            Value::Boolean(true),
        ]))
    );
}

#[test]
fn fingerprints_ignore_where_an_item_is() {
    let src = r#"
let a = unwrap(meta.fingerprint("fn f(x) = x + 1"))
let moved = unwrap(meta.fingerprint("\n\n   fn f(x) =\n  x + 1"))
let body = unwrap(meta.fingerprint("fn f(x) = x + 2"))
let sig = unwrap(meta.fingerprint("fn f(x, y) = x + 1"))
let node = unwrap(meta.fingerprint(first(unwrap(meta.parse("\nfn f(x) = x + 1")))))
[a == moved, a.sig == body.sig, a.body != body.body, a.sig != sig.sig, a.body == sig.body, a == node, str.length(a.sig)]
"#;
    let t = Value::Boolean(true);
    assert_eq!(
        eval(src),
        Value::List(std::sync::Arc::new(vec![
            t.clone(),
            t.clone(),
            t.clone(),
            t.clone(),
            t.clone(),
            t,
            Value::Integer(16)
        ]))
    );
}

#[test]
fn resolve_names_what_each_name_reads() {
    let file = "fn add(a, b) = a + b\nlet x = add(1, len([2]))\nlet y = str.trim(\" z \") + q\n";
    let src = format!(
        r#"map(unwrap(meta.resolve({})), (n) => [n.name, n.start, n.def ?? (), n.builtin ?? false, n.module ?? false, n.unresolved ?? false])"#,
        lit(file)
    );
    let v = eval(&src);
    let Value::List(rows) = &v else { panic!() };
    let find = |name: &str, nth: usize| {
        rows.iter()
            .filter(|r| matches!(r, Value::List(c) if c[0] == s(name)))
            .nth(nth)
            .cloned()
            .unwrap_or_else(|| panic!("{name} #{nth}"))
    };
    let at = |name: &str, nth: usize| file.match_indices(name).nth(nth).unwrap().0 as i64;
    let def = |a: i64, b: i64| {
        let mut m = olang::ast::ValueMap::default();
        m.insert("start".into(), Value::Integer(a));
        m.insert("end".into(), Value::Integer(b));
        Value::Map(std::sync::Arc::new(m))
    };
    let f = Value::Boolean(false);
    let t = Value::Boolean(true);
    let row = |n: &str, start: i64, d: Value, b: &Value, m: &Value, u: &Value| {
        Value::List(std::sync::Arc::new(vec![
            s(n),
            Value::Integer(start),
            d,
            b.clone(),
            m.clone(),
            u.clone(),
        ]))
    };
    // the call of `add` reads the fn's name
    let add_def = def(at("add", 0), at("add", 0) + 3);
    assert_eq!(
        find("add", 1),
        row("add", at("add", 1), add_def, &f, &f, &f)
    );
    // `a` in the body reads the parameter
    let a_def = def(at("a", 1), at("a", 1) + 1);
    assert_eq!(
        find("a", 1),
        row("a", file.find("= a").unwrap() as i64 + 2, a_def, &f, &f, &f)
    );
    assert_eq!(
        find("len", 0),
        row("len", at("len", 0), Value::Unit, &t, &f, &f)
    );
    assert_eq!(
        find("str", 0),
        row("str", at("str", 0), Value::Unit, &f, &t, &f)
    );
    assert_eq!(find("q", 0), row("q", at("q", 0), Value::Unit, &f, &f, &t));
    let err = eval(r#"is_err(meta.resolve("fn ( = "))"#);
    assert_eq!(err, Value::Boolean(true));
}

#[test]
fn types_are_what_hover_shows() {
    let file = "/// Doubles.\nfn double(x: Int) -> Int = x * 2\nshare fn half(x) = x / 2\nlet n = 3\ntype P = struct { a: Int }\n";
    let src = format!(
        r#"map(unwrap(meta.types({})), (d) => [d.name, d.signature, d.doc, d.name_start])"#,
        lit(file)
    );
    let all = eval(&src);
    let Value::List(rows) = &all else { panic!() };
    let names: Vec<_> = rows
        .iter()
        .map(|r| match r {
            Value::List(c) => (c[0].clone(), c[1].clone()),
            _ => panic!(),
        })
        .collect();
    assert_eq!(
        names,
        vec![
            (s("double"), s("fn double(x: Int) -> Int")),
            (s("half"), s("share fn half(x)")),
            (s("n"), s("let n: Int")),
            (s("P"), s("type P")),
        ]
    );
    let Value::List(first) = &rows[0] else {
        panic!()
    };
    assert_eq!(first[2], s("Doubles."));
    assert_eq!(
        first[3],
        Value::Integer(file.find("double").unwrap() as i64)
    );
}

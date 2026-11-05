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

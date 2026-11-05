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

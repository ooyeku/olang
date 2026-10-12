//! `meta.parse`'s node format, held to its documentation: stdlib.md
//! lists every node kind and its fields between the `meta.parse nodes`
//! markers, and this test parses a program that uses every form and
//! checks both directions — each kind and field the parser produces is
//! documented, and each documented kind and field is produced. The
//! format is a public one (stability.md); kinds and fields are only
//! ever added, and an addition fails here until the table has it.

use olang::Parser;
use olang::ast::Value;
use std::collections::{BTreeMap, BTreeSet};

const SAMPLE: &str = r#"use geometry { area }
share use lib.x
type P = struct { x: Int, y: (Int, String) }
type E = enum { A, B(Int) }
type R = { name: String, tags: [String], m: Map<String, Int>, f: (Int) -> Int, r: Result<Int, String>, g: Box<Int>, l: "a" | "b", u: () }
error Oops { Bad: () }
trait Show { fn show(self) -> String }
impl Show for P { fn show(self) = "p" }
meta fn twice(x) = x + " + " + x
@derive
type Q = struct { a: Int }
test "t" { assert_eq(1, 1) }
let mut counter: Int = 0
share let shared_value = 1
fn f(a: Int, b = 2) -> Int = {
    let xs = [1, 2]
    let ys = xs + [3]
    let m = #{ "k": 1, ...#{} }
    let o = { a: 1, ...{ b: 2 } }
    let mut zs = [0]
    zs[0] = 5
    let rec = { items: [1] }
    let mut r2 = rec
    r2.items[0] = 9
    counter = counter + 1
    let t = (1, "s")
    let s = `x ${a}`
    let raw = r"raw"
    let fl = 1.5
    let bw = 1 | 2
    let neg = -a
    let pipe = xs |> map((v) => v * 2)
    let rg = 0..=3
    let ok = Ok(1)
    let er = Err("e")
    let tr = ok?
    let st = P { x: 1, y: (2, "s") }
    let fa = st.x
    let ix = xs[0]
    let c = @twice(1)
    let w = if a > 1 => "big" else => "small"
    let mm = match t {
        (1, n) if n == "s" => 1,
        [h, ...rest] => 2,
        A | B(_) => 3,
        P { x, y: _ } => 4,
        { k: v } => 5,
        Ok(q) => 6,
        Err(_) => 7,
        1..10 => 8,
        "s" => 9,
        _ => 0
    }
    for i in 0..3 { continue }
    par for j in xs { j }
    while false { break }
    let lp = loop { break 1 }
    let task = spawn { 1 }
    assert(true)
    assert_eq(1, 1)
    assert_ne(1, 2)
    assert_true(true)
    assert_false(false)
    g(x: 1)
    let tm: Map<String, Int> = #{}
    let tr2: Result<Int, String> = Ok(1)
    let tu: () = ()
    let tn: P = st
    let ti: Int = 1
    let tl: [Int] = []
    let sp = #{ ...m }
    return a
}
f(1)
"#;

/// The sample as `meta.parse` hands it to a program.
fn nodes() -> Value {
    let program = Parser::new().parse_raw(SAMPLE).expect("the sample parses");
    Value::List(std::sync::Arc::new(olang::stdlib::meta::program_nodes(
        &program,
    )))
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.to_string()),
        _ => None,
    }
}

fn walk(v: &Value, out: &mut BTreeMap<String, BTreeSet<String>>) {
    match v {
        Value::List(items) => items.iter().for_each(|x| walk(x, out)),
        Value::Map(m) => {
            if let Some(kind) = m.get("kind").and_then(text) {
                let key = match m.get("form").and_then(text) {
                    Some(form) => format!("{kind}/{form}"),
                    None => kind,
                };
                let fields = out.entry(key).or_default();
                for k in m.keys() {
                    if k != "kind" && k != "form" {
                        fields.insert(k.clone());
                    }
                }
            }
            m.values().for_each(|x| walk(x, out));
        }
        _ => {}
    }
}

fn documented() -> BTreeMap<String, BTreeSet<String>> {
    let doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/stdlib.md"),
    )
    .expect("stdlib.md");
    let start = doc.find("<!-- meta.parse nodes -->").expect("start marker");
    let end = doc.find("<!-- /meta.parse nodes -->").expect("end marker");
    let mut out = BTreeMap::new();
    for line in doc[start..end].lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 3 || !cells[1].starts_with('`') {
            continue;
        }
        let kind = cells[1].trim_matches('`').to_string();
        let fields = cells[2]
            .split(',')
            .map(|f| f.trim().trim_matches('`').to_string())
            .filter(|f| !f.is_empty() && f != "—")
            .collect();
        out.insert(kind, fields);
    }
    out
}

#[test]
fn every_node_kind_and_field_is_documented_and_produced() {
    let mut produced = BTreeMap::new();
    walk(&nodes(), &mut produced);
    let documented = documented();
    assert!(documented.len() > 50, "the table was not found");
    let mut problems = Vec::new();
    for (kind, fields) in &produced {
        match documented.get(kind) {
            None => problems.push(format!("`{kind}` is produced but not documented")),
            Some(doc) => {
                for f in fields.difference(doc) {
                    problems.push(format!(
                        "`{kind}`'s field `{f}` is produced but not documented"
                    ));
                }
            }
        }
    }
    for (kind, fields) in &documented {
        match produced.get(kind) {
            None => problems.push(format!(
                "`{kind}` is documented but the sample does not produce it"
            )),
            Some(made) => {
                for f in fields.difference(made) {
                    problems.push(format!(
                        "`{kind}`'s field `{f}` is documented but not produced"
                    ));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "stdlib.md's node table and meta.parse disagree:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn spread_and_index_assignment_are_nodes_of_their_own() {
    let mut produced = BTreeMap::new();
    walk(&nodes(), &mut produced);
    assert!(produced.contains_key("spread_literal"));
    assert!(produced.contains_key("index_assign"));
    // The hidden builtins they are built by never show as calls.
    let found = format!("{:?}", nodes());
    assert!(!found.contains("__spread") && !found.contains("__index_set"));
}

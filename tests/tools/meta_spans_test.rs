//! `meta.parse`'s spans (OL-434): every statement and expression node
//! carries `start` and `end`, its byte range in the source, and the
//! spans parse builds the same tree a plain parse does.

use olang::Parser;
use olang::ast::Value;
use std::collections::BTreeSet;

const SPAN_KEYS: &[&str] = &["start", "end", "name_span"];

/// The node without its span fields, all the way down.
fn strip(v: &Value) -> Value {
    match v {
        Value::List(items) => Value::List(std::sync::Arc::new(items.iter().map(strip).collect())),
        Value::Map(m) => {
            let mut out = olang::ast::ValueMap::default();
            for (k, x) in m.iter() {
                if !SPAN_KEYS.contains(&k.as_str()) {
                    out.insert(k.clone(), strip(x));
                }
            }
            Value::Map(std::sync::Arc::new(out))
        }
        other => other.clone(),
    }
}

fn int(m: &olang::ast::ValueMap, k: &str) -> Option<usize> {
    match m.get(k) {
        Some(Value::Integer(n)) => Some(*n as usize),
        _ => None,
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// Kinds that are not expressions or statements: their parts carry spans.
const NO_SPAN: &[&str] = &[
    "arm",
    "named_arg",
    "entry",
    "field_value",
    "pattern",
    "type",
    "error",
];

/// A span a made node took from its construct: wider than its name.
fn inherited(src: &str, a: usize, b: usize, name: &str) -> bool {
    b - a > name.len() && src[a..b].contains(name)
}

/// Every node's span is inside the source, an identifier's covers its
/// name, a child's sits inside its parent's; the kinds with none listed.
fn check(src: &str, v: &Value, parent: Option<(usize, usize)>, missing: &mut BTreeSet<String>) {
    match v {
        Value::List(items) => items.iter().for_each(|x| check(src, x, parent, missing)),
        Value::Map(m) => {
            let mut here = parent;
            if let Some(kind) = text(m.get("kind")) {
                match (int(m, "start"), int(m, "end")) {
                    (Some(a), Some(b)) => {
                        assert!(a <= b && b <= src.len(), "{kind} {a}..{b} of {}", src.len());
                        assert!(src.is_char_boundary(a) && src.is_char_boundary(b));
                        // (a name the parser made, `__matmul`, has its construct's)
                        let name = text(m.get("name")).unwrap_or_default();
                        if kind == "ident"
                            && !name.starts_with("__")
                            && !inherited(src, a, b, &name)
                        {
                            assert_eq!(&src[a..b], name, "an ident's span is its name");
                        }
                        if let Some((pa, pb)) = parent {
                            assert!(pa <= a && b <= pb, "{kind} {a}..{b} outside {pa}..{pb}");
                        }
                        here = Some((a, b));
                    }
                    _ => {
                        missing.insert(kind.clone());
                    }
                }
                if let Some(Value::Map(ns)) = m.get("name_span") {
                    let (a, b) = (int(ns, "start").unwrap(), int(ns, "end").unwrap());
                    let name = text(m.get("name")).unwrap_or_default();
                    assert_eq!(&src[a..b], name, "name_span of {kind}");
                }
            }
            for (k, x) in m.iter() {
                if !SPAN_KEYS.contains(&k.as_str()) {
                    check(src, x, here, missing);
                }
            }
        }
        _ => {}
    }
}

fn sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in ["src/stdlib/embedded", "examples", "frameworks/web-sdk/lib"] {
        let mut stack = vec![root.join(dir)];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "ol")
                    && let Ok(t) = std::fs::read_to_string(&p)
                {
                    out.push((p.display().to_string(), t));
                }
            }
        }
    }
    out
}

#[test]
fn spans_change_nothing_but_add_start_and_end() {
    let mut checked = 0;
    let mut missing = BTreeSet::new();
    for (path, src) in sources() {
        let Ok(program) = Parser::new().parse_raw(&src) else {
            continue;
        };
        let plain = Value::List(std::sync::Arc::new(olang::stdlib::meta::program_nodes(
            &program,
        )));
        let spanned = Value::List(std::sync::Arc::new(
            olang::stdlib::meta::parse_spanned(&src).unwrap_or_else(|e| panic!("{path}: {e}")),
        ));
        assert_eq!(
            strip(&spanned),
            plain,
            "{path}: the spans parse built another tree"
        );
        check(&src, &spanned, None, &mut missing);
        checked += 1;
    }
    assert!(checked > 50, "only {checked} files");
    let unexpected: Vec<_> = missing
        .iter()
        .filter(|k| !NO_SPAN.contains(&k.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "kinds without a span: {unexpected:?}"
    );
}

#[test]
fn spans_are_bytes_and_names_have_their_own() {
    let src = "let cafe = 1\nfn add(a, b) = a + fs.join([\"é\"])\n";
    let nodes = olang::stdlib::meta::parse_spanned(src).unwrap();
    let Value::Map(f) = &nodes[1] else { panic!() };
    assert_eq!(
        (int(f, "start"), int(f, "end")),
        (src.find("fn"), Some(src.len() - 1))
    );
    let Some(Value::Map(ns)) = f.get("name_span") else {
        panic!()
    };
    assert_eq!(
        &src[int(ns, "start").unwrap()..int(ns, "end").unwrap()],
        "add"
    );
    let Some(Value::Map(body)) = f.get("body") else {
        panic!()
    };
    let Some(Value::Map(call)) = body.get("right") else {
        panic!()
    };
    assert_eq!(
        &src[int(call, "start").unwrap()..int(call, "end").unwrap()],
        "fs.join([\"é\"])"
    );
    let Value::Map(l) = &nodes[0] else { panic!() };
    let Some(Value::Map(ns)) = l.get("name_span") else {
        panic!()
    };
    assert_eq!(
        &src[int(ns, "start").unwrap()..int(ns, "end").unwrap()],
        "cafe"
    );
}

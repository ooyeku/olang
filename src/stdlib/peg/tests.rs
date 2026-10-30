//! The machine on its own: grammars built from rule data as `peg.ol`
//! hands them over, matched without an interpreter. The olang suites
//! (src/stdlib/embedded/peg_test.ol, peg_native_test.ol) test the whole
//! package and compare the two engines.

use super::grammar::{self, E, RuleIn};
use super::machine::{Extra, Machine};
use super::values::{Input, error_data};
use crate::ast::Value;

fn lit(s: &str) -> E {
    E::Lit {
        codes: s.chars().map(|c| c as i32).collect(),
        s: s.to_string(),
        ci: false,
    }
}
fn class(lo: char, hi: char) -> E {
    E::Class {
        ranges: vec![(lo as i32, hi as i32)],
        neg: false,
        src: format!("[{lo}-{hi}]"),
    }
}
fn r(n: &str) -> E {
    E::Ref(n.to_string())
}
fn rule(name: &str, e: E) -> RuleIn {
    let token = name.trim_start_matches('_').starts_with(|c: char| c.is_ascii_uppercase());
    RuleIn {
        name: name.to_string(),
        desc: None,
        e,
        token,
        silent: name.starts_with('_'),
    }
}

fn build(rules: Vec<RuleIn>, memo: &[&str], heads: &[&str]) -> grammar::Grammar {
    let memo: Vec<String> = memo.iter().map(|s| s.to_string()).collect();
    let heads: Vec<String> = heads.iter().map(|s| s.to_string()).collect();
    grammar::build(rules, &memo, &heads, &[]).expect("builds")
}

fn accepts(g: &grammar::Grammar, text: &str) -> bool {
    let input = Input::new(text);
    Machine::new(g, &input.cs).accepts(0)
}

fn message(g: &grammar::Grammar, text: &str) -> String {
    let input = Input::new(text);
    let mut m = Machine::new(g, &input.cs);
    let (far, exp) = m.failure(0, 0);
    match error_data(&input, far, &exp, &g.descs, &m.oks) {
        Value::Map(ref m) => match m.get("message") {
            Some(Value::String(s)) => s.as_str().to_string(),
            _ => panic!("no message"),
        },
        _ => panic!("not a map"),
    }
}

fn list_grammar() -> grammar::Grammar {
    // list = "[" item ("," item)* "]" ; item = Num ; Num = [0-9]+ ; skip = [ ]+
    build(
        vec![
            rule(
                "list",
                E::Seq(vec![
                    lit("["),
                    r("item"),
                    E::Star(Box::new(E::Seq(vec![lit(","), r("item")]))),
                    lit("]"),
                ]),
            ),
            rule("item", r("Num")),
            rule("Num", E::Plus(Box::new(class('0', '9')))),
            rule(
                "skip",
                E::Plus(Box::new(E::Class {
                    ranges: vec![(32, 32)],
                    neg: false,
                    src: "[ ]".into(),
                })),
            ),
        ],
        &[],
        &[],
    )
}

#[test]
fn sequences_choices_repetition_and_skip() {
    let g = list_grammar();
    assert!(accepts(&g, "[1,22,333]"));
    assert!(accepts(&g, " [ 1 , 22 ] "));
    assert!(!accepts(&g, "[1,]"));
    assert!(!accepts(&g, "[1 2]"));
    // a token takes no skip inside it
    assert!(!accepts(&g, "[1 1]"));
}

#[test]
fn the_tree_is_built_compact() {
    let g = list_grammar();
    let input = Input::new("[1, 22]");
    let mut m = Machine::new(&g, &input.cs);
    let root = m.build(0, 0).expect("parses");
    let n = m.nodes[root as usize];
    assert_eq!((n.kind, n.start, n.end, n.nkids), (0, 0, 7, 2));
    let item = m.nodes[m.store[n.kids as usize + 1].node as usize];
    assert_eq!((item.start, item.end), (4, 6));
    let num = m.nodes[m.store[item.kids as usize].node as usize];
    assert_eq!(g.kinds[num.kind as usize].as_str(), "Num");
    assert!(matches!(num.extra, Extra::None));
}

#[test]
fn errors_name_the_furthest_failure() {
    let g = list_grammar();
    assert_eq!(
        message(&g, "[1, 2 3]"),
        "expected \",\" or \"]\" after \"2\", found \"3\" (1:7)"
    );
    assert_eq!(
        message(&g, "[1,"),
        "expected Num after \",\", found end of input (1:4)"
    );
    assert_eq!(
        message(&g, "[1] x"),
        "expected end of input after \"]\", found \"x\" (1:5)"
    );
}

#[test]
fn left_recursion_grows_a_seed() {
    // sum = sum "-" Num / Num ; Num = [0-9]+
    let g = build(
        vec![
            rule(
                "sum",
                E::Alt(vec![E::Seq(vec![r("sum"), lit("-"), r("Num")]), r("Num")]),
            ),
            rule("Num", E::Plus(Box::new(class('0', '9')))),
        ],
        &["sum"],
        &["sum"],
    );
    assert!(accepts(&g, "9-4-3"));
    assert!(!accepts(&g, "9-"));
    let input = Input::new("9-4-3");
    let mut m = Machine::new(&g, &input.cs);
    let root = m.build(0, 0).expect("parses");
    // left-leaning: ((9-4)-3)
    let top = m.nodes[root as usize];
    assert_eq!(top.nkids, 2);
    let left = m.nodes[m.store[top.kids as usize].node as usize];
    assert_eq!((left.start, left.end, left.nkids), (0, 3, 2));
}

#[test]
fn precedence_climbs() {
    // e = precedence(Num) { left "+"; left "*"; right "^"; prefix "-" }
    let g = build(
        vec![
            rule(
                "e",
                E::Prec {
                    atom: Box::new(r("Num")),
                    levels: vec![
                        ("left".into(), vec![lit("+")]),
                        ("left".into(), vec![lit("*")]),
                        ("right".into(), vec![lit("^")]),
                        ("prefix".into(), vec![lit("-")]),
                    ],
                },
            ),
            rule("Num", E::Plus(Box::new(class('0', '9')))),
        ],
        &[],
        &[],
    );
    assert!(accepts(&g, "1+2*3^4^5"));
    assert!(accepts(&g, "-1"));
    assert!(!accepts(&g, "1+"));
    let input = Input::new("1+2*3");
    let mut m = Machine::new(&g, &input.cs);
    let root = m.build(0, 0).expect("parses");
    let top = m.nodes[root as usize];
    // 1 + (2*3)
    assert!(matches!(top.extra, Extra::Prec { op_start: 1, op_end: 2, .. }));
    let right = m.nodes[m.store[top.kids as usize + 1].node as usize];
    assert!(matches!(right.extra, Extra::Prec { op_start: 3, .. }));
}

#[test]
fn lookahead_and_case_insensitive_literals() {
    // kw = "select"i !Id ; Id = [a-z]
    let g = build(
        vec![
            rule(
                "Kw",
                E::Seq(vec![
                    E::Lit {
                        codes: "select".chars().map(|c| c as i32).collect(),
                        s: "select".into(),
                        ci: true,
                    },
                    E::Not(Box::new(r("Id"))),
                ]),
            ),
            rule("Id", class('a', 'z')),
        ],
        &[],
        &[],
    );
    assert!(accepts(&g, "SeLeCt"));
    assert!(!accepts(&g, "selects"));
}

#[test]
fn code_points_not_bytes() {
    // W = [α-ω]+
    let g = build(
        vec![rule(
            "W",
            E::Plus(Box::new(E::Class {
                ranges: vec![('α' as i32, 'ω' as i32)],
                neg: false,
                src: "[α-ω]".into(),
            })),
        )],
        &[],
        &[],
    );
    let input = Input::new("λόγος");
    assert_eq!(input.n(), 5);
    assert!(accepts(&g, "αβγ"));
    assert!(!accepts(&g, "αβc"));
    assert_eq!(input.slice(1, 3), "όγ");
}

#[test]
fn a_rule_tried_twice_at_one_place_is_cached() {
    // e = t "+" e / t ; t = "(" e ")" / [0-9]  — exponential without it
    let g = build(
        vec![
            rule(
                "e",
                E::Alt(vec![E::Seq(vec![r("t"), lit("+"), r("e")]), r("t")]),
            ),
            rule(
                "t",
                E::Alt(vec![E::Seq(vec![lit("("), r("e"), lit(")")]), class('0', '9')]),
            ),
        ],
        &["t"],
        &[],
    );
    let deep = format!("{}1{}", "(".repeat(40), ")".repeat(40));
    assert!(accepts(&g, &format!("{deep}+{deep}")));
}

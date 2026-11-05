//! `meta.tokens` packed and from an offset (OL-454): three Int lists in
//! bytes and the lexer's state, so an editor relexes from a line.

use olang::stdlib::tokens::{LexState, scan_state, tokens, tokens_from};

#[test]
fn from_an_offset_in_code_matches_the_whole_lex() {
    let src = "let a = 1\nfn f(x) = x + `t ${a} u`\n// done\nlet s = \"two\nlines\"\n";
    let whole = tokens(src);
    let from = src.find("fn").unwrap();
    let (part, end) = tokens_from(src, from, LexState::Code);
    let tail: Vec<_> = whole
        .iter()
        .filter(|t| t.a >= from)
        .map(|t| (t.a, t.b))
        .collect();
    let got: Vec<_> = part.iter().map(|t| (t.a, t.b)).collect();
    assert_eq!(got, tail);
    assert_eq!(end, LexState::Code);
}

#[test]
fn the_state_carries_an_open_string_to_the_next_line() {
    let src = "let s = \"one\ntwo\" + `a\nb ${x}\nc` + r\"r\n\"\n";
    let line2 = src.find("two").unwrap();
    // the first line ends inside the string
    assert_eq!(scan_state(&src[..line2], LexState::Code).1, LexState::Text);
    let (toks, end) = tokens_from(src, line2, LexState::Text);
    assert_eq!(
        (toks[0].kind, &src[toks[0].a..toks[0].b]),
        ("string", "two\"")
    );
    assert_eq!(end, LexState::Code);
    // and inside a template, and a raw string
    let tl = src.find("b ${").unwrap();
    assert_eq!(scan_state(&src[..tl], LexState::Code).1, LexState::Template);
    let rl = src.rfind("\"\n").unwrap();
    assert_eq!(scan_state(&src[..rl], LexState::Code).1, LexState::Raw);
    let joined: String = tokens_from(src, tl, LexState::Template)
        .0
        .iter()
        .map(|t| &src[t.a..t.b])
        .collect();
    assert_eq!(joined, &src[tl..]);
}

#[test]
fn meta_tokens_packed_answers_three_int_lists_in_bytes() {
    let src = r#"
let text = "let s = \"é\n"
let first = meta.tokens(text, #{"packed": true})
let next = meta.tokens(text + "x\" + 1\n", #{"packed": true, "from": str.byte_len(text), "state": first.state})
[first.state, len(first.kind) == len(first.start), first.kinds[first.kind[0]], first.end[len(first.end) - 1],
 next.kinds[next.kind[0]], next.start[0], next.state]
"#;
    let program = olang::Parser::new().parse(src).unwrap();
    let v = olang::Interpreter::new().eval_program(program).unwrap();
    let s = |t: &str| olang::ast::Value::String(std::sync::Arc::new(t.to_string()));
    let i = |n: i64| olang::ast::Value::Integer(n);
    assert_eq!(
        v,
        olang::ast::Value::List(std::sync::Arc::new(vec![
            s("string"),
            olang::ast::Value::Boolean(true),
            s("keyword"),
            i(12),
            s("string"),
            i(12),
            s("code"),
        ]))
    );
}

//! `meta.tokens` and `meta.highlight`: olang source as tokens, from the
//! lexer the language server colours with (src/tools/semantic.rs), with
//! the spaces and punctuation it does not report filled in so the tokens
//! cover the text exactly: their texts concatenate to the source.
//!
//! The same code runs natively and in the browser runtime, so a site's
//! build and its playground highlight alike.

use crate::ast::Value;
use crate::tools::semantic::{self as sem, Token};
use std::sync::Arc;

/// Every module name a program may write, on any profile: highlighting
/// does not depend on which runtime is reading.
const MODULES: &[&str] = &[
    "str",
    "col",
    "math",
    "json",
    "toml",
    "csv",
    "re",
    "dates",
    "time",
    "random",
    "sim",
    "crypto",
    "base64",
    "bytes",
    "compress",
    "asset",
    "fs",
    "os",
    "http",
    "db",
    "testing",
    "ods",
    "stats",
    "plot",
    "learn",
    "mat",
    "cell",
    "chan",
    "task",
    "proc",
    "pty",
    "vt",
    "tty",
    "gui",
    "caps",
    "meta",
    "bigint",
    "collections",
    "runtime",
    "dom",
    "vec",
    "md",
    "gg",
    "peg",
    "cli",
    "term",
    "ui",
    "viz",
    "dash",
    "colx",
    "mathx",
    "ml",
    "web",
];

/// One token: its kind, its byte range in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tok {
    pub kind: &'static str,
    pub a: usize,
    pub b: usize,
}

fn kind_of(t: &Token, text: &str) -> &'static str {
    match t.kind {
        sem::T_KEYWORD => match &text[t.a..t.b] {
            "true" | "false" => "constant",
            _ => "keyword",
        },
        sem::T_STRING => "string",
        sem::T_NUMBER => "number",
        sem::T_COMMENT => "comment",
        sem::T_OPERATOR => "operator",
        sem::T_FUNCTION | sem::T_METHOD => "function",
        sem::T_TYPE => "type",
        sem::T_ENUM_MEMBER => "constant",
        sem::T_NAMESPACE => "module",
        sem::T_PROPERTY => "property",
        sem::T_VARIABLE if t.mods & sem::M_READONLY != 0 => "constant",
        _ => "ident",
    }
}

/// The tokens of `src`, covering it exactly.
pub fn tokens(src: &str) -> Vec<Tok> {
    let mut sem_toks = sem::tokens(src, &|_| false, MODULES);
    sem_toks.retain(|t| t.b > t.a);
    sem_toks.sort_by_key(|t| t.a);
    // the interpolation marks: `${` and the `}` closing one
    let marks: std::collections::HashMap<usize, usize> =
        sem::interpolation_marks(src).into_iter().collect();
    let mut out: Vec<Tok> = Vec::with_capacity(sem_toks.len() * 2);
    let mut at = 0usize;
    let fill = |out: &mut Vec<Tok>, from: usize, to: usize| {
        // a gap: runs of spaces, and each other character punctuation
        let mut i = from;
        while i < to {
            let c = src[i..].chars().next().expect("in range");
            if let Some(&b) = marks.get(&i) {
                out.push(Tok {
                    kind: "interpolation",
                    a: i,
                    b,
                });
                i = b;
            } else if c.is_whitespace() {
                let end = src[i..to]
                    .find(|d: char| !d.is_whitespace())
                    .map(|k| i + k)
                    .unwrap_or(to);
                out.push(Tok {
                    kind: "space",
                    a: i,
                    b: end,
                });
                i = end;
            } else {
                let w = c.len_utf8();
                out.push(Tok {
                    kind: "punct",
                    a: i,
                    b: i + w,
                });
                i += w;
            }
        }
    };
    for t in &sem_toks {
        if t.a < at {
            continue;
        }
        fill(&mut out, at, t.a);
        out.push(Tok {
            kind: kind_of(t, src),
            a: t.a,
            b: t.b,
        });
        at = t.b;
    }
    fill(&mut out, at, src.len());
    // `@name` written together is a macro (or a decorator); `a @ b` stays an operator
    let mut merged: Vec<Tok> = Vec::with_capacity(out.len());
    let mut k = 0;
    while k < out.len() {
        let t = out[k];
        if &src[t.a..t.b] == "@"
            && let Some(n) = out.get(k + 1)
            && n.a == t.b
            && !matches!(n.kind, "space" | "punct" | "string" | "number" | "comment")
        {
            merged.push(Tok {
                kind: "macro",
                a: t.a,
                b: n.b,
            });
            k += 2;
            continue;
        }
        if &src[t.a..t.b] == "@" {
            merged.push(Tok {
                kind: "operator",
                ..t
            });
        } else {
            merged.push(t);
        }
        k += 1;
    }
    merged
}

fn esc_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// `src` as HTML for inside a `<code>`: every token but a space a
/// `<span class="ol-KIND">`.
pub fn highlight(src: &str) -> String {
    spans(src, &tokens(src))
}

/// `src` in `lang` (a name `lexers::language` gave) as HTML, the same way.
pub fn highlight_in(src: &str, lang: &str) -> String {
    spans(src, &super::lexers::tokens(src, lang))
}

fn spans(src: &str, toks: &[Tok]) -> String {
    let mut out = String::with_capacity(src.len() * 3);
    for &t in toks {
        let text = &src[t.a..t.b];
        if t.kind == "space" {
            esc_into(&mut out, text);
        } else {
            out.push_str("<span class=\"ol-");
            out.push_str(t.kind);
            out.push_str("\">");
            esc_into(&mut out, text);
            out.push_str("</span>");
        }
    }
    out
}

/// `meta.tokens(src)`: `[#{ kind, text, start, end, line }]`, the
/// offsets in characters, the line from 1.
pub fn tokens_value(src: &str) -> Value {
    tokens_value_of(src, tokens(src))
}

/// `meta.tokens(src, lang)`: the same for another language.
pub fn tokens_value_in(src: &str, lang: &str) -> Value {
    tokens_value_of(src, super::lexers::tokens(src, lang))
}

/// Tokens as `meta.tokens` answers them: maps with character offsets.
pub fn tokens_value_of(src: &str, toks: Vec<Tok>) -> Value {
    let mut items = Vec::with_capacity(toks.len());
    let (mut chars, mut line, mut at) = (0usize, 1usize, 0usize);
    for t in toks {
        // advance the character count and line to the token's start
        for c in src[at..t.a].chars() {
            chars += 1;
            if c == '\n' {
                line += 1;
            }
        }
        let text = &src[t.a..t.b];
        let n = text.chars().count();
        let mut m = crate::ast::ValueMap::default();
        m.insert("kind".into(), Value::String(Arc::new(t.kind.to_string())));
        m.insert("text".into(), Value::String(Arc::new(text.to_string())));
        m.insert("start".into(), Value::Integer(chars as i64));
        m.insert("end".into(), Value::Integer((chars + n) as i64));
        m.insert("line".into(), Value::Integer(line as i64));
        items.push(Value::Map(Arc::new(m)));
        chars += n;
        line += text.matches('\n').count();
        at = t.b;
    }
    Value::List(Arc::new(items))
}

// ── Packed tokens from an offset (OL-454) ───────────────────────────

/// The kinds `meta.tokens(src, #{"packed": true})` numbers its tokens by:
/// a token's kind is its index here.
pub const KINDS: &[&str] = &[
    "space",
    "punct",
    "ident",
    "keyword",
    "constant",
    "string",
    "number",
    "comment",
    "operator",
    "function",
    "type",
    "module",
    "property",
    "interpolation",
    "macro",
];

fn kind_code(kind: &str) -> i64 {
    KINDS.iter().position(|k| *k == kind).unwrap_or(2) as i64
}

/// Where the lexer is between two tokens: in code, or inside a string,
/// a raw string or a template that the text so far has not closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LexState {
    Code,
    Text,
    Raw,
    Template,
}

impl LexState {
    pub fn name(self) -> &'static str {
        match self {
            LexState::Code => "code",
            LexState::Text => "string",
            LexState::Raw => "raw",
            LexState::Template => "template",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "code" => LexState::Code,
            "string" => LexState::Text,
            "raw" => LexState::Raw,
            "template" => LexState::Template,
            _ => return None,
        })
    }
}

/// Scan `src` from `state`: the byte where the construct `state` is in
/// closes (0 for code), and the state at the end of `src`. A `//` comment
/// ends at its line, so it is never a state.
pub fn scan_state(src: &str, state: LexState) -> (usize, LexState) {
    let b = src.as_bytes();
    let mut st = state;
    let mut close = if state == LexState::Code {
        Some(0)
    } else {
        None
    };
    // `${` depth inside a template, so a `}` in code closes only its own
    let mut i = 0;
    let mut interp: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    while i < b.len() {
        let c = b[i];
        match st {
            LexState::Code => match c {
                b'/' if b.get(i + 1) == Some(&b'/') => {
                    i = memchr::memchr(b'\n', &b[i..]).map_or(b.len(), |n| i + n);
                    continue;
                }
                b'"' => st = LexState::Text,
                b'r' if b.get(i + 1) == Some(&b'"')
                    && !(i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) =>
                {
                    st = LexState::Raw;
                    i += 1;
                }
                b'`' => st = LexState::Template,
                b'{' => depth += 1,
                b'}' => {
                    if interp.last() == Some(&depth) {
                        interp.pop();
                        st = LexState::Template;
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                }
                _ => {}
            },
            LexState::Text => match c {
                b'\\' => i += 1,
                b'"' => {
                    st = LexState::Code;
                    close.get_or_insert(i + 1);
                }
                _ => {}
            },
            LexState::Raw => match c {
                b'\\' if b.get(i + 1) == Some(&b'"') => i += 1,
                b'"' => {
                    st = LexState::Code;
                    close.get_or_insert(i + 1);
                }
                _ => {}
            },
            LexState::Template => match c {
                b'\\' => i += 1,
                b'`' => {
                    st = LexState::Code;
                    if interp.is_empty() {
                        close.get_or_insert(i + 1);
                    }
                }
                b'$' if b.get(i + 1) == Some(&b'{') => {
                    interp.push(depth);
                    st = LexState::Code;
                    i += 1;
                }
                _ => {}
            },
        }
        i += 1;
    }
    (close.unwrap_or(b.len()), st)
}

/// The tokens of `src[from..]` lexed from `state`, in the whole text's
/// byte offsets, and the state at the end. A construct `state` says is
/// open is one token up to where it closes; the rest is lexed as code.
pub fn tokens_from(src: &str, from: usize, state: LexState) -> (Vec<Tok>, LexState) {
    let rest = &src[from..];
    let (close, end_state) = scan_state(rest, state);
    let mut out = Vec::new();
    if close > 0 {
        out.push(Tok {
            kind: "string",
            a: from,
            b: from + close,
        });
    }
    if close < rest.len() {
        out.extend(tokens(&rest[close..]).into_iter().map(|t| Tok {
            a: t.a + from + close,
            b: t.b + from + close,
            ..t
        }));
    }
    (out, end_state)
}

/// `#{kind, start, end, kinds, state}`: the tokens as three Int lists
/// (a kind is its index in `kinds`), in bytes, and the state at the end.
pub fn packed_value(toks: &[Tok], state: LexState) -> Value {
    let ints = |f: &dyn Fn(&Tok) -> i64| {
        Value::List(Arc::new(
            toks.iter().map(|t| Value::Integer(f(t))).collect(),
        ))
    };
    let mut m = crate::ast::ValueMap::default();
    m.insert("kind".into(), ints(&|t| kind_code(t.kind)));
    m.insert("start".into(), ints(&|t| t.a as i64));
    m.insert("end".into(), ints(&|t| t.b as i64));
    m.insert(
        "kinds".into(),
        Value::List(Arc::new(
            KINDS
                .iter()
                .map(|k| Value::String(Arc::new(k.to_string())))
                .collect(),
        )),
    );
    m.insert(
        "state".into(),
        Value::String(Arc::new(state.name().to_string())),
    );
    Value::Map(Arc::new(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(String, &'static str)> {
        tokens(src)
            .into_iter()
            .filter(|t| t.kind != "space")
            .map(|t| (src[t.a..t.b].to_string(), t.kind))
            .collect()
    }

    #[test]
    fn tokens_cover_the_source() {
        for src in [
            "let x = `a ${f(1) + #{ \"k\": 2 }.k} b` // c\n",
            "fn f(a, b) = a |> g\n/* x */ \"multi\nline\" @derive(x)\nlet m = a @ b\n",
            "émoji = \"✓\" + 'c'\n\t\r\n",
            "unterminated \"string",
            "",
        ] {
            let joined: String = tokens(src).iter().map(|t| &src[t.a..t.b]).collect();
            assert_eq!(joined, src);
        }
    }

    #[test]
    fn kinds_by_meaning() {
        let k = kinds(
            "use web { el }\nfn add(a) = str.upper(`v ${a}`) + 1.5 // n\nlet Ok = true\n@trace(x)\nPoint { y: MAX }",
        );
        for want in [
            ("fn", "keyword"),
            ("add", "function"),
            ("str", "module"),
            ("upper", "function"),
            ("`v ", "string"),
            ("${", "interpolation"),
            ("}", "interpolation"),
            ("1.5", "number"),
            ("// n", "comment"),
            ("+", "operator"),
            ("(", "punct"),
            ("true", "constant"),
            ("@trace", "macro"),
            ("Point", "type"),
            ("MAX", "constant"),
        ] {
            assert!(
                k.contains(&(want.0.to_string(), want.1)),
                "{want:?} in {k:?}"
            );
        }
    }

    /// Every keyword and keyword form grammar.pest knows, in a place
    /// it is one, is a keyword token; the contextual ones stay names
    /// where the grammar reads a name.
    #[test]
    fn every_grammar_keyword_is_a_keyword() {
        let src = "use lib.geo { area as region }\n\
                   share fn f(x) = x\n\
                   meta fn m(e) = e\n\
                   type P = struct { x: Int }\n\
                   type E = enum { A, B }\n\
                   trait T { fn t(self) }\n\
                   impl T for P { fn t(self) = 1 }\n\
                   error Bad { Oops }\n\
                   test \"t\" { assert(true) }\n\
                   let mut n = if n { 1 } else { 2 }\n\
                   for i in xs { continue }\n\
                   par for i in xs { break }\n\
                   while false { loop { return match x { _ => 0 } } }\n\
                   let h = spawn { 1 }\n\
                   let g = spawn work(1)\n";
        let k = kinds(src);
        for w in [
            "use", "as", "share", "fn", "meta", "type", "struct", "enum", "trait", "impl", "for",
            "error", "test", "let", "mut", "if", "else", "in", "par", "continue", "break", "while",
            "loop", "return", "match", "spawn",
        ] {
            assert!(
                k.iter().any(|(t, kind)| t == w && *kind == "keyword"),
                "{w} is a keyword in {k:?}"
            );
            assert!(
                !k.iter().any(|(t, kind)| t == w && *kind == "ident"),
                "{w} is never an ident here: {k:?}"
            );
        }
        for w in ["await", "yield", "async", "defer", "const", "pub"] {
            let k = kinds(&format!("{w} x"));
            assert_eq!(k[0], (w.to_string(), "keyword"), "{w}");
            // a member and a key are not the reserved word
            let k = kinds(&format!("http.{w}()\n#{{ {w}: 1 }}"));
            assert!(
                !k.iter().any(|(t, kind)| t == w && *kind == "keyword"),
                "{w}: {k:?}"
            );
        }
        for (src, word) in [
            ("let spawn = 1\nspawn + 1", "spawn"),
            ("task.spawn(f)", "spawn"),
            ("meta.tokens(s)", "meta"),
            ("let par = 2", "par"),
            ("let type = row.kind", "type"),
            ("let as = 1", "as"),
        ] {
            let k = kinds(src);
            assert!(
                !k.iter().any(|(t, kind)| t == word && *kind == "keyword"),
                "{src}: {k:?}"
            );
        }
        assert!(
            kinds("true false")
                .iter()
                .all(|(_, kind)| *kind == "constant")
        );
    }

    #[test]
    fn highlight_escapes_and_classes() {
        let h = highlight("let s = \"<a>\"");
        assert_eq!(
            h,
            "<span class=\"ol-keyword\">let</span> <span class=\"ol-ident\">s</span> <span class=\"ol-operator\">=</span> <span class=\"ol-string\">&quot;&lt;a&gt;&quot;</span>"
        );
    }
}

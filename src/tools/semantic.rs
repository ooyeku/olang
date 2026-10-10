//! Semantic tokens for `olang lsp`: every word, literal, comment and
//! operator of an olang source classified by what it is — a module, a
//! type, a function and where it is declared, a parameter, a variable,
//! a property — so an editor colours by meaning rather than by regex.
//!
//! It is a lexer with a little context, not the parser: it answers for
//! text that does not parse yet (the file being typed), and it is fast
//! enough to answer a 50,000-line file in a few milliseconds, or a range
//! of it in microseconds. Tokens never span a line (a multi-line string
//! is one token a line) and positions are UTF-16, as the protocol wants.

/// The token types, in the legend's order.
pub const TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "enumMember",
    "function",
    "method",
    "parameter",
    "variable",
    "property",
    "keyword",
    "string",
    "number",
    "comment",
    "operator",
];

/// The token modifiers, in the legend's order (bit `1 << i`).
pub const TOKEN_MODIFIERS: &[&str] = &["declaration", "defaultLibrary", "documentation", "readonly"];

pub const T_NAMESPACE: u32 = 0;
pub const T_TYPE: u32 = 1;
pub const T_ENUM_MEMBER: u32 = 2;
pub const T_FUNCTION: u32 = 3;
pub const T_METHOD: u32 = 4;
pub const T_PARAMETER: u32 = 5;
pub const T_VARIABLE: u32 = 6;
pub const T_PROPERTY: u32 = 7;
pub const T_KEYWORD: u32 = 8;
pub const T_STRING: u32 = 9;
pub const T_NUMBER: u32 = 10;
pub const T_COMMENT: u32 = 11;
pub const T_OPERATOR: u32 = 12;

pub const M_DECLARATION: u32 = 1;
pub const M_DEFAULT_LIBRARY: u32 = 2;
pub const M_DOCUMENTATION: u32 = 4;
pub const M_READONLY: u32 = 8;

/// One token: its line, its start and length in UTF-16 units, its type
/// and modifier bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub line: u32,
    pub start: u32,
    pub len: u32,
    pub kind: u32,
    pub mods: u32,
}

const HARD_KEYWORDS: &[&str] = &[
    "fn", "let", "if", "else", "match", "for", "while", "loop", "break", "continue", "return",
    "true", "false", "struct", "enum",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lx {
    Word,
    Number,
    Str,
    Comment(bool),
    Punct,
}

#[derive(Clone, Copy, Debug)]
struct Lexeme {
    kind: Lx,
    /// Byte range in the text.
    a: usize,
    b: usize,
    line: u32,
    /// UTF-16 column of `a` on its line.
    col: u32,
    len16: u32,
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The lexemes of `text`: words, numbers, strings (a piece a line;
/// a template's `${…}` lexed as code), comments and punctuation.
fn lex(text: &str) -> Vec<Lexeme> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 0u32;
    let mut col = 0u32;
    // a template's interpolations: the brace depth inside each
    let mut interp: Vec<u32> = Vec::new();
    // inside a template's text (resumed after an interpolation closes)
    let mut in_template = false;
    let n = bytes.len();
    // A string piece from `a` to `b`, split at its line breaks.
    macro_rules! string_piece {
        ($a:expr, $b:expr, $line0:expr, $col0:expr) => {{
            let (mut pa, mut pl, mut pc) = ($a, $line0, $col0);
            let mut units = 0u32;
            for (off, ch) in text[$a..$b].char_indices() {
                let at = $a + off;
                if ch == '\n' {
                    if at > pa {
                        out.push(Lexeme { kind: Lx::Str, a: pa, b: at, line: pl, col: pc, len16: units });
                    }
                    pl += 1;
                    pc = 0;
                    pa = at + 1;
                    units = 0;
                } else {
                    units += ch.len_utf16() as u32;
                }
            }
            if $b > pa {
                out.push(Lexeme { kind: Lx::Str, a: pa, b: $b, line: pl, col: pc, len16: units });
            }
            (pl, pc + units)
        }};
    }
    while i < n {
        if in_template {
            // the template's text up to its end or an interpolation
            let start = i;
            let mut j = i;
            let mut closed = false;
            let mut opened = false;
            while j < n {
                let c = bytes[j];
                if c == b'\\' && j + 1 < n {
                    j += 1 + text[j + 1..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    continue;
                }
                if c == b'`' {
                    j += 1;
                    closed = true;
                    break;
                }
                if c == b'$' && j + 1 < n && bytes[j + 1] == b'{' {
                    opened = true;
                    break;
                }
                j += 1;
            }
            let (l2, c2) = string_piece!(start, j, line, col);
            line = l2;
            col = c2;
            i = j;
            in_template = false;
            if opened {
                out.push(Lexeme { kind: Lx::Punct, a: i, b: i + 2, line, col, len16: 2 });
                i += 2;
                col += 2;
                interp.push(0);
            }
            let _ = closed;
            continue;
        }
        let c = text[i..].chars().next().unwrap();
        let cl = c.len_utf8();
        if c == '\n' {
            line += 1;
            col = 0;
            i += 1;
            continue;
        }
        if c == ' ' || c == '\t' || c == '\r' {
            i += 1;
            col += 1;
            continue;
        }
        // comments: to the end of the line
        if c == '/' && i + 1 < n && bytes[i + 1] == b'/' {
            let end = text[i..].find('\n').map(|k| i + k).unwrap_or(n);
            let doc = text[i..].starts_with("///") || text[i..].starts_with("//!");
            let units: u32 = text[i..end].chars().map(|c| c.len_utf16() as u32).sum();
            out.push(Lexeme { kind: Lx::Comment(doc), a: i, b: end, line, col, len16: units });
            col += units;
            i = end;
            continue;
        }
        // strings: "…", r"…", '…'; a template's start
        if c == '"' || (c == 'r' && i + 1 < n && bytes[i + 1] == b'"') || c == '\'' {
            let raw = c == 'r';
            let quote = if c == '\'' { b'\'' } else { b'"' };
            let mut j = i + if raw { 2 } else { 1 };
            while j < n {
                let b = bytes[j];
                if b == b'\\' && !raw && j + 1 < n {
                    j += 1 + text[j + 1..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    continue;
                }
                if b == b'\\' && raw && j + 1 < n && bytes[j + 1] == b'"' {
                    j += 2;
                    continue;
                }
                if b == quote {
                    j += 1;
                    break;
                }
                // a char literal never spans a line
                if quote == b'\'' && b == b'\n' {
                    break;
                }
                j += 1;
            }
            let j = j.min(n);
            let (l2, c2) = string_piece!(i, j, line, col);
            line = l2;
            col = c2;
            i = j;
            continue;
        }
        if c == '`' {
            // the backtick itself starts the template's first piece
            let mut j = i + 1;
            let mut opened = false;
            while j < n {
                let b = bytes[j];
                if b == b'\\' && j + 1 < n {
                    j += 1 + text[j + 1..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    continue;
                }
                if b == b'`' {
                    j += 1;
                    break;
                }
                if b == b'$' && j + 1 < n && bytes[j + 1] == b'{' {
                    opened = true;
                    break;
                }
                j += 1;
            }
            let j = j.min(n);
            let (l2, c2) = string_piece!(i, j, line, col);
            line = l2;
            col = c2;
            i = j;
            if opened {
                out.push(Lexeme { kind: Lx::Punct, a: i, b: i + 2, line, col, len16: 2 });
                i += 2;
                col += 2;
                interp.push(0);
            }
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i + 1;
            let hexy = c == '0' && j < n && matches!(bytes[j], b'x' | b'b' | b'o');
            if hexy {
                j += 1;
            }
            while j < n {
                let b = bytes[j];
                let ok = b.is_ascii_alphanumeric()
                    || b == b'_'
                    || (b == b'.' && !hexy && j + 1 < n && bytes[j + 1].is_ascii_digit())
                    || ((b == b'+' || b == b'-') && !hexy && matches!(bytes[j - 1], b'e' | b'E'));
                if !ok {
                    break;
                }
                j += 1;
            }
            out.push(Lexeme { kind: Lx::Number, a: i, b: j, line, col, len16: (j - i) as u32 });
            col += (j - i) as u32;
            i = j;
            continue;
        }
        if is_ident_start(c) {
            let mut j = i + cl;
            let mut units = c.len_utf16() as u32;
            while j < n {
                let d = text[j..].chars().next().unwrap();
                if !is_ident(d) {
                    break;
                }
                units += d.len_utf16() as u32;
                j += d.len_utf8();
            }
            out.push(Lexeme { kind: Lx::Word, a: i, b: j, line, col, len16: units });
            col += units;
            i = j;
            continue;
        }
        // punctuation: two-character operators first
        // (the operators are ASCII: a character wider than a byte here is
        // no operator, and `i + 2` may fall inside it)
        let two = text.get(i..(i + 2).min(n)).unwrap_or("");
        let w = if matches!(two, "|>" | "=>" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "->" | "..") {
            2
        } else {
            cl
        };
        if !interp.is_empty() {
            if c == '{' {
                *interp.last_mut().unwrap() += 1;
            } else if c == '}' {
                let d = interp.last_mut().unwrap();
                if *d == 0 {
                    interp.pop();
                    out.push(Lexeme { kind: Lx::Punct, a: i, b: i + 1, line, col, len16: 1 });
                    i += 1;
                    col += 1;
                    in_template = true;
                    continue;
                }
                *d -= 1;
            }
        }
        out.push(Lexeme { kind: Lx::Punct, a: i, b: i + w, line, col, len16: c.len_utf16() as u32 * if w == 2 { 2 } else { 1 } });
        col += if w == 2 { 2 } else { c.len_utf16() as u32 };
        i += w;
    }
    out
}

fn is_operator(p: &str) -> bool {
    matches!(
        p,
        "|>" | "=>" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "->" | ".." | "+" | "-" | "*" | "/"
            | "%" | "=" | "<" | ">" | "!" | "?" | "|" | "&"
    )
}

fn all_caps(w: &str) -> bool {
    w.chars().count() > 1
        && w.chars().any(|c| c.is_alphabetic())
        && !w.chars().any(|c| c.is_lowercase())
}

/// A scope of parameter names: alive while the bracket depth stays at or
/// above `depth`, or (a top-level `fn … =`) until the next line that
/// starts in the first column.
struct Scope {
    names: Vec<String>,
    depth: i32,
    top: bool,
    /// The body has begun (a top-level scope ends at a new item only then).
    begun: bool,
}

/// Every token of `text`. `is_builtin(name)` says whether a global name
/// is the language's own; `modules` are the standard modules.
pub fn tokens(text: &str, is_builtin: &dyn Fn(&str) -> bool, modules: &[&str]) -> Vec<Token> {
    let lx = lex(text);
    let word = |l: &Lexeme| &text[l.a..l.b];
    // names this file declares with `fn`, and the modules it imports
    let mut declared_fns = std::collections::HashSet::new();
    let mut imported = std::collections::HashSet::new();
    for (k, l) in lx.iter().enumerate() {
        if l.kind == Lx::Word && word(l) == "fn" {
            if let Some(nx) = lx.get(k + 1) {
                if nx.kind == Lx::Word {
                    declared_fns.insert(word(nx).to_string());
                }
            }
        }
        if l.kind == Lx::Word && word(l) == "use" && (k == 0 || lx[k - 1].line != l.line || word(&lx[k - 1]) == "share") {
            let mut j = k + 1;
            while j < lx.len() && lx[j].line == l.line && lx[j].kind == Lx::Word || (j < lx.len() && word(&lx[j]) == ".") {
                if lx[j].kind == Lx::Word {
                    imported.insert(word(&lx[j]).to_string());
                }
                j += 1;
            }
        }
    }
    // lambda parameter lists: `(a, b) =>` with no name before the `(`
    let mut lambda_param = vec![false; lx.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mut matching = vec![usize::MAX; lx.len()];
    for (k, l) in lx.iter().enumerate() {
        if l.kind == Lx::Punct {
            match word(l) {
                "(" | "[" | "{" | "${" => stack.push(k),
                ")" | "]" | "}" => {
                    if let Some(o) = stack.pop() {
                        matching[k] = o;
                        matching[o] = k;
                    }
                }
                _ => {}
            }
        }
    }
    for (k, l) in lx.iter().enumerate() {
        if l.kind == Lx::Punct && word(l) == "=>" && k > 0 {
            let p = &lx[k - 1];
            if p.kind == Lx::Punct && word(p) == ")" && matching[k - 1] != usize::MAX {
                let o = matching[k - 1];
                let named = o > 0 && lx[o - 1].kind == Lx::Word && !HARD_KEYWORDS.contains(&word(&lx[o - 1]));
                if !named {
                    for q in o + 1..k - 1 {
                        if lx[q].kind == Lx::Word {
                            lambda_param[q] = true;
                        }
                    }
                }
            } else if p.kind == Lx::Word && k > 1 && !(lx[k - 2].kind == Lx::Word) {
                // `x => …` as a lambda: only after `(` or `,` (an argument)
                let pp = word(&lx[k - 2]);
                if pp == "(" || pp == "," {
                    lambda_param[k - 1] = true;
                }
            }
        }
    }

    let mut out = Vec::with_capacity(lx.len());
    let mut scopes: Vec<Scope> = Vec::new();
    let mut depth: i32 = 0;
    // a `fn`'s parameter list being read: (the scope, the depth inside it)
    let mut reading_params: Option<i32> = None;
    // a `let` or `for` pattern being read, until `=` or `in`
    let mut binding: Option<&str> = None;
    let mut last_line_seen = u32::MAX;
    for (k, l) in lx.iter().enumerate() {
        // a new item in the first column ends a top-level `fn … =`
        if l.line != last_line_seen {
            last_line_seen = l.line;
            if l.col == 0 && depth == 0 && !matches!(l.kind, Lx::Comment(_)) {
                scopes.retain(|s| !(s.top && s.begun));
            }
        }
        let prev = (k > 0).then(|| &lx[k - 1]);
        let next = lx.get(k + 1);
        let pw = prev.map(|p| word(p)).unwrap_or("");
        let nw = next.map(|p| word(p)).unwrap_or("");
        let after_error_kw = pw == "error" && prev.is_some_and(|p| out_was_keyword(&out, p));
        let mut push = |kind: u32, mods: u32| {
            out.push(Token { line: l.line, start: l.col, len: l.len16, kind, mods })
        };
        match l.kind {
            Lx::Comment(doc) => push(T_COMMENT, if doc { M_DOCUMENTATION } else { 0 }),
            Lx::Str => push(T_STRING, 0),
            Lx::Number => push(T_NUMBER, 0),
            Lx::Punct => {
                let p = word(l);
                match p {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => {
                        depth -= 1;
                        if reading_params == Some(depth) {
                            reading_params = None;
                        }
                        scopes.retain(|s| s.top || s.depth <= depth);
                    }
                    _ => {}
                }
                if (p == "=" || p == "{") && reading_params.is_none() {
                    if let Some(s) = scopes.last_mut() {
                        s.begun = true;
                    }
                }
                if p == "=" && binding.is_some() {
                    binding = None;
                }
                if is_operator(p) {
                    push(T_OPERATOR, 0);
                }
                if p == "=>" && k > 0 {
                    // a lambda's parameters are in scope in its body
                    let names: Vec<String> = (0..k).rev().take_while(|q| lambda_param[*q] || lx[*q].kind == Lx::Punct && matches!(word(&lx[*q]), "(" | ")" | ","))
                        .filter(|q| lambda_param[*q])
                        .map(|q| word(&lx[q]).to_string())
                        .collect();
                    if !names.is_empty() {
                        scopes.push(Scope { names, depth, top: false, begun: true });
                    }
                }
            }
            Lx::Word => {
                let w = word(l);
                let at_line_start = prev.is_none_or(|p| p.line != l.line);
                if HARD_KEYWORDS.contains(&w) {
                    push(T_KEYWORD, 0);
                    if w == "let" || w == "for" {
                        binding = Some(w);
                    }
                    if w == "fn" {
                        let top = depth == 0;
                        scopes.push(Scope { names: Vec::new(), depth: depth + 1, top, begun: false });
                    }
                    continue;
                }
                let contextual = match w {
                    "share" | "use" | "meta" => at_line_start,
                    "test" => next.is_some_and(|n| n.kind == Lx::Str),
                    "type" | "trait" | "impl" | "error" => {
                        (at_line_start || pw == "share") && next.is_some_and(|n| n.kind == Lx::Word)
                    }
                    "mut" => pw == "let",
                    "in" => binding == Some("for"),
                    "par" => nw == "for" || nw == "map",
                    "spawn" => next.is_some_and(|n| n.kind == Lx::Word),
                    "self" => true,
                    _ => false,
                };
                if contextual {
                    push(T_KEYWORD, 0);
                    if w == "in" {
                        binding = None;
                    }
                    continue;
                }
                // a fn's name, its parameters
                if pw == "fn" {
                    push(T_FUNCTION, M_DECLARATION);
                    if nw == "(" {
                        reading_params = Some(depth);
                    }
                    continue;
                }
                if let Some(d) = reading_params {
                    if depth == d + 1 && !(pw == ":" || pw == "=") {
                        if let Some(s) = scopes.last_mut() {
                            s.names.push(w.to_string());
                        }
                        push(T_PARAMETER, M_DECLARATION);
                        continue;
                    }
                }
                if lambda_param[k] {
                    push(T_PARAMETER, M_DECLARATION);
                    continue;
                }
                if matches!(pw, "struct" | "enum" | "type" | "trait" | "impl") {
                    push(T_TYPE, M_DECLARATION);
                    continue;
                }
                if after_error_kw {
                    push(T_TYPE, M_DECLARATION);
                    continue;
                }
                // a module path after `use`
                if lx[..k].iter().rev().take_while(|p| p.line == l.line).any(|p| word(p) == "use") {
                    let in_braces = lx[..k].iter().rev().take_while(|p| p.line == l.line).any(|p| word(p) == "{");
                    if !in_braces {
                        let std = modules.contains(&w);
                        push(T_NAMESPACE, if std { M_DEFAULT_LIBRARY } else { 0 });
                        continue;
                    }
                }
                if matches!(w, "Ok" | "Err" | "Some" | "None") {
                    push(T_ENUM_MEMBER, M_DEFAULT_LIBRARY);
                    continue;
                }
                if pw == "." {
                    let module_call = k >= 2 && modules.contains(&word(&lx[k - 2]));
                    if nw == "(" {
                        push(if module_call { T_FUNCTION } else { T_METHOD }, if module_call { M_DEFAULT_LIBRARY } else { 0 });
                    } else {
                        push(T_PROPERTY, 0);
                    }
                    continue;
                }
                if nw == "." && (modules.contains(&w) || imported.contains(w)) && !scopes.iter().any(|s| s.names.iter().any(|n| n == w)) {
                    push(T_NAMESPACE, if modules.contains(&w) { M_DEFAULT_LIBRARY } else { 0 });
                    continue;
                }
                if let Some(b) = binding {
                    if pw == b || pw == "mut" || pw == "(" || pw == "," || pw == "[" {
                        push(T_VARIABLE, M_DECLARATION);
                        continue;
                    }
                }
                let first_upper = w.chars().next().is_some_and(|c| c.is_uppercase());
                if first_upper {
                    if all_caps(w) {
                        push(T_VARIABLE, M_READONLY);
                    } else {
                        push(T_TYPE, 0);
                    }
                    continue;
                }
                if scopes.iter().any(|s| s.names.iter().any(|n| n == w)) {
                    push(T_PARAMETER, 0);
                    continue;
                }
                if nw == "(" {
                    let lib = !declared_fns.contains(w) && is_builtin(w);
                    push(T_FUNCTION, if lib { M_DEFAULT_LIBRARY } else { 0 });
                    continue;
                }
                if declared_fns.contains(w) {
                    push(T_FUNCTION, 0);
                    continue;
                }
                push(T_VARIABLE, 0);
            }
        }
    }
    out
}

fn out_was_keyword(out: &[Token], p: &Lexeme) -> bool {
    out.last().is_some_and(|t| t.line == p.line && t.start == p.col && t.kind == T_KEYWORD)
}

/// The tokens encoded as the protocol's relative integers, those on
/// lines `lo..=hi` only.
pub fn encode(tokens: &[Token], lo: u32, hi: u32) -> Vec<u32> {
    let mut data = Vec::with_capacity(tokens.len() * 5);
    let (mut pl, mut pc) = (0u32, 0u32);
    for t in tokens {
        if t.line < lo || t.line > hi || t.len == 0 {
            continue;
        }
        let dl = t.line - pl;
        let dc = if dl == 0 { t.start - pc } else { t.start };
        data.extend_from_slice(&[dl, dc, t.len, t.kind, t.mods]);
        pl = t.line;
        pc = t.start;
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(String, &'static str, u32)> {
        let toks = tokens(src, &|w| w == "println" || w == "len", &["str", "fs"]);
        let lines: Vec<&str> = src.split('\n').collect();
        toks.iter()
            .map(|t| {
                let l = lines[t.line as usize];
                let s: String = l.chars().skip(t.start as usize).take(t.len as usize).collect();
                (s, TOKEN_TYPES[t.kind as usize], t.mods)
            })
            .collect()
    }

    #[test]
    fn a_function_its_parameters_and_calls() {
        let k = kinds("/// doc\nfn add(a, b) = a + b\nlet x = add(1, 2.5)\nprintln(str.upper(\"hi\"))");
        assert_eq!(k[0], ("/// doc".into(), "comment", M_DOCUMENTATION));
        assert_eq!(k[1], ("fn".into(), "keyword", 0));
        assert_eq!(k[2], ("add".into(), "function", M_DECLARATION));
        assert_eq!(k[3], ("a".into(), "parameter", M_DECLARATION));
        assert!(k.contains(&("b".into(), "parameter", 0)));
        assert!(k.contains(&("x".into(), "variable", M_DECLARATION)));
        assert!(k.contains(&("add".into(), "function", 0)));
        assert!(k.contains(&("2.5".into(), "number", 0)));
        assert!(k.contains(&("println".into(), "function", M_DEFAULT_LIBRARY)));
        assert!(k.contains(&("str".into(), "namespace", M_DEFAULT_LIBRARY)));
        assert!(k.contains(&("upper".into(), "function", M_DEFAULT_LIBRARY)));
        assert!(k.contains(&("\"hi\"".into(), "string", 0)));
    }

    #[test]
    fn templates_lambdas_and_multiline_strings() {
        let k = kinds("let f = (n) => `v ${n + 1}!`\nlet s = \"a\nb\"\nshare let MAX = 3\nstruct Point { x: Int }");
        assert!(k.contains(&("n".into(), "parameter", M_DECLARATION)));
        assert!(k.contains(&("n".into(), "parameter", 0)));
        assert!(k.contains(&("`v ".into(), "string", 0)));
        assert!(k.contains(&("}!`".into(), "string", 0)) || k.contains(&("!`".into(), "string", 0)));
        assert!(k.contains(&("\"a".into(), "string", 0)));
        assert!(k.contains(&("b\"".into(), "string", 0)));
        assert!(k.contains(&("share".into(), "keyword", 0)));
        assert!(k.contains(&("MAX".into(), "variable", M_DECLARATION)));
        assert!(k.contains(&("Point".into(), "type", M_DECLARATION)));
        assert!(k.contains(&("x".into(), "property", 0)) || k.contains(&("x".into(), "variable", 0)));
    }

    #[test]
    fn encoding_is_relative_and_ranged() {
        let t = tokens("let a = 1\nlet b = 2", &|_| false, &[]);
        let all = encode(&t, 0, u32::MAX);
        assert_eq!(&all[..5], &[0, 0, 3, T_KEYWORD, 0]);
        let second = encode(&t, 1, 1);
        assert_eq!(&second[..5], &[1, 0, 3, T_KEYWORD, 0]);
    }
}

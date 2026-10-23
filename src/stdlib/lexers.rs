//! Tokenizers for the languages beside olang that a site or a notebook
//! shows: Python, Rust, JavaScript (TypeScript), shell, TOML, JSON and
//! Markdown. Each gives `Tok`s that cover the text exactly, with the
//! kinds `meta.tokens` uses for olang, so `meta.highlight(src, lang)`
//! colours them with the same `ol-KIND` classes.
//!
//! They are small and forgiving: any text lexes, finished or not, and
//! nothing is parsed. Pure Rust, so the browser runtime has them too.

use super::tokens::Tok;

/// The language a fence's or a caller's name means, or None when no
/// tokenizer knows it.
pub fn language(name: &str) -> Option<&'static str> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "olang" | "ol" => "olang",
        "python" | "py" | "python3" => "python",
        "rust" | "rs" => "rust",
        "js" | "javascript" | "mjs" | "cjs" | "jsx" | "ts" | "typescript" | "tsx" => "js",
        "sh" | "bash" | "shell" | "zsh" | "console" | "shell-session" | "sh-session"
        | "shellsession" => "shell",
        "toml" => "toml",
        "json" | "jsonc" | "json5" => "json",
        "md" | "markdown" => "markdown",
        _ => return None,
    })
}

/// The names `language` knows, for an error.
pub const LANGUAGES: &str = "olang, python, rust, js, shell, toml, json, markdown";

/// The tokens of `src` in `lang` (a name `language` gave).
pub fn tokens(src: &str, lang: &str) -> Vec<Tok> {
    let mut out = match lang {
        "olang" => return super::tokens::tokens(src),
        "python" => code(src, &PYTHON),
        "rust" => code(src, &RUST),
        "js" => code(src, &JS),
        "json" => code(src, &JSON),
        "shell" => shell(src),
        "toml" => toml(src),
        "markdown" => markdown(src),
        _ => vec![Tok {
            kind: "ident",
            a: 0,
            b: src.len(),
        }],
    };
    out.retain(|t| t.b > t.a);
    out
}

// ── shared pieces ────────────────────────────────────────────────────

fn ch(src: &str, i: usize) -> Option<char> {
    src.get(i..).and_then(|s| s.chars().next())
}

fn next_i(src: &str, i: usize) -> usize {
    i + ch(src, i).map(char::len_utf8).unwrap_or(1)
}

fn is_id_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_id(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The end of the run of whitespace from `i`.
fn spaces(src: &str, mut i: usize) -> usize {
    while let Some(c) = ch(src, i) {
        if !c.is_whitespace() {
            break;
        }
        i += c.len_utf8();
    }
    i
}

/// The end of the identifier from `i`.
fn ident_end(src: &str, mut i: usize) -> usize {
    while let Some(c) = ch(src, i) {
        if !is_id(c) {
            break;
        }
        i += c.len_utf8();
    }
    i
}

/// The end of a number from `i` (a digit there).
fn number_end(src: &str, i: usize) -> usize {
    let b = src.as_bytes();
    let hexy =
        b[i] == b'0' && matches!(b.get(i + 1), Some(b'x' | b'X' | b'b' | b'B' | b'o' | b'O'));
    let mut j = i + 1;
    while j < b.len() {
        let c = b[j];
        let ok = c.is_ascii_alphanumeric()
            || c == b'_'
            || (c == b'.' && !hexy && b.get(j + 1).is_some_and(u8::is_ascii_digit))
            || ((c == b'+' || c == b'-') && !hexy && matches!(b[j - 1], b'e' | b'E'));
        if !ok {
            break;
        }
        j += 1;
    }
    j
}

/// The end of the line from `i` (before its `\n`).
fn line_end(src: &str, i: usize) -> usize {
    src[i..].find('\n').map(|k| i + k).unwrap_or(src.len())
}

/// The end of a string opened by `quote` (`i` just after it): the closing
/// quote included, a backslash escaping the next character when
/// `escapes`; a single-line string stops at the line's end.
fn string_end(src: &str, mut i: usize, quote: &str, escapes: bool, multiline: bool) -> usize {
    while i < src.len() {
        if src[i..].starts_with(quote) {
            return i + quote.len();
        }
        let c = ch(src, i).expect("in range");
        if c == '\n' && !multiline {
            return i;
        }
        if c == '\\' && escapes {
            i = next_i(src, i);
            if i < src.len() {
                i = next_i(src, i);
            }
            continue;
        }
        i += c.len_utf8();
    }
    src.len()
}

const OPERATOR_CHARS: &str = "+-*/%=<>!&|^~?:@";

// ── C-like code: Python, Rust, JavaScript, JSON ──────────────────────

struct Spec {
    keywords: &'static [&'static str],
    constants: &'static [&'static str],
    types: &'static [&'static str],
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Quote characters of one-line strings with backslash escapes.
    quotes: &'static [char],
    /// Python's `"""` and `'''`.
    triple: bool,
    /// Letters that may prefix a string (`f"…"`, `b'…'`, `r"…"`).
    prefixes: &'static [&'static str],
    rust: bool,
    /// JavaScript's template literal.
    template: bool,
    /// `@name` (Python's decorator).
    decorators: bool,
    /// A word after one of these names a function; after the others a type.
    defines_fn: &'static [&'static str],
    defines_type: &'static [&'static str],
    /// A string before `:` is a key (JSON).
    keys: bool,
}

const PYTHON: Spec = Spec {
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
        "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
        "yield", "match", "case",
    ],
    constants: &["True", "False", "None", "self", "cls"],
    types: &[
        "int",
        "float",
        "str",
        "bool",
        "list",
        "dict",
        "set",
        "tuple",
        "bytes",
        "object",
        "type",
        "complex",
        "frozenset",
    ],
    line_comments: &["#"],
    block_comment: None,
    quotes: &['"', '\''],
    triple: true,
    prefixes: &[
        "f", "r", "b", "u", "rb", "br", "fr", "rf", "F", "R", "B", "U",
    ],
    rust: false,
    template: false,
    decorators: true,
    defines_fn: &["def"],
    defines_type: &["class"],
    keys: false,
};

const RUST: Spec = Spec {
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
        "pub", "ref", "return", "static", "struct", "super", "trait", "type", "unsafe", "use",
        "where", "while",
    ],
    constants: &["true", "false", "self", "Self", "None", "Some", "Ok", "Err"],
    types: &[
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
        "f32", "f64", "bool", "char", "str",
    ],
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"'],
    triple: false,
    prefixes: &["b"],
    rust: true,
    template: false,
    decorators: false,
    defines_fn: &["fn"],
    defines_type: &["struct", "enum", "trait", "type", "union"],
    keys: false,
};

const JS: Spec = Spec {
    keywords: &[
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "finally",
        "for",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "of",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "try",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        "yield",
        "async",
        "await",
        "from",
        "as",
        "interface",
        "type",
        "enum",
        "implements",
        "declare",
        "readonly",
        "get",
        "set",
    ],
    constants: &["true", "false", "null", "undefined", "NaN", "Infinity"],
    types: &[
        "string", "number", "boolean", "any", "unknown", "never", "object", "bigint",
    ],
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\''],
    triple: false,
    prefixes: &[],
    rust: false,
    template: true,
    decorators: true,
    defines_fn: &["function"],
    defines_type: &["class", "interface", "enum", "type"],
    keys: false,
};

const JSON: Spec = Spec {
    keywords: &[],
    constants: &["true", "false", "null"],
    types: &[],
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\''],
    triple: false,
    prefixes: &[],
    rust: false,
    template: false,
    decorators: false,
    defines_fn: &[],
    defines_type: &[],
    keys: true,
};

fn all_caps(w: &str) -> bool {
    w.chars().any(|c| c.is_uppercase())
        && w.chars()
            .all(|c| c.is_uppercase() || c.is_ascii_digit() || c == '_')
}

fn code(src: &str, spec: &Spec) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0usize;
    let n = src.len();
    // the last word token's text, for `def name` / `class Name`
    let mut last_word: &str = "";
    let push =
        |out: &mut Vec<Tok>, kind: &'static str, a: usize, b: usize| out.push(Tok { kind, a, b });
    while i < n {
        let c = ch(src, i).expect("in range");
        let rest = &src[i..];
        if c.is_whitespace() {
            let e = spaces(src, i);
            push(&mut out, "space", i, e);
            i = e;
            continue;
        }
        if let Some(lc) = spec.line_comments.iter().find(|lc| rest.starts_with(**lc)) {
            let _ = lc;
            let e = line_end(src, i);
            push(&mut out, "comment", i, e);
            i = e;
            continue;
        }
        if let Some((open, close)) = spec.block_comment
            && rest.starts_with(open)
        {
            let e = src[i + open.len()..]
                .find(close)
                .map(|k| i + open.len() + k + close.len())
                .unwrap_or(n);
            push(&mut out, "comment", i, e);
            i = e;
            continue;
        }
        // Rust: `#[attr]` / `#![attr]`
        if spec.rust && (rest.starts_with("#[") || rest.starts_with("#![")) {
            let mut depth = 0i32;
            let mut j = i;
            let le = line_end(src, i);
            while j < le {
                match src.as_bytes()[j] {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                j = next_i(src, j);
            }
            push(&mut out, "macro", i, j);
            i = j;
            continue;
        }
        // Rust: raw strings `r"…"`, `r#"…"#`, `br#"…"#`
        if spec.rust {
            let after_prefix = if rest.starts_with("br") {
                Some(i + 2)
            } else if rest.starts_with('r') {
                Some(i + 1)
            } else {
                None
            };
            if let Some(p) = after_prefix {
                let hashes = src[p..].bytes().take_while(|&b| b == b'#').count();
                if src.as_bytes().get(p + hashes) == Some(&b'"') {
                    let close = format!("\"{}", "#".repeat(hashes));
                    let e = src[p + hashes + 1..]
                        .find(&close)
                        .map(|k| p + hashes + 1 + k + close.len())
                        .unwrap_or(n);
                    push(&mut out, "string", i, e);
                    i = e;
                    last_word = "";
                    continue;
                }
            }
            // a char literal or a lifetime
            if c == '\'' {
                let j = i + 1;
                let lit = if src.as_bytes().get(j) == Some(&b'\\') {
                    Some(string_end(src, j, "'", true, false))
                } else if let Some(d) = ch(src, j) {
                    let k = j + d.len_utf8();
                    (src.as_bytes().get(k) == Some(&b'\'')).then_some(k + 1)
                } else {
                    None
                };
                if let Some(e) = lit {
                    push(&mut out, "string", i, e);
                    i = e;
                } else {
                    let e = ident_end(src, j);
                    push(&mut out, if e > j { "type" } else { "punct" }, i, e.max(j));
                    i = e.max(j);
                }
                continue;
            }
        }
        // a string, maybe prefixed
        let prefix_len = spec
            .prefixes
            .iter()
            .filter(|p| rest.starts_with(**p))
            .map(|p| p.len())
            .filter(|&l| ch(src, i + l).is_some_and(|q| spec.quotes.contains(&q)))
            .max()
            .unwrap_or(0);
        if let Some(q) = ch(src, i + prefix_len)
            && spec.quotes.contains(&q)
            && (prefix_len > 0 || c == q)
        {
            let s = i + prefix_len;
            let triple: String = std::iter::repeat_n(q, 3).collect();
            // a backslash steps over the next character in Python's
            // `r"…"` too: `r"\""` is one string
            let e = if spec.triple && src[s..].starts_with(&triple) {
                string_end(src, s + 3, &triple, true, true)
            } else {
                string_end(src, s + 1, &q.to_string(), true, spec.rust)
            };
            let key = spec.keys && {
                let k = spaces(src, e);
                src[k..].starts_with(':')
            };
            push(&mut out, if key { "property" } else { "string" }, i, e);
            i = e;
            last_word = "";
            continue;
        }
        if spec.template && c == '`' {
            let e = string_end(src, i + 1, "`", true, true);
            push(&mut out, "string", i, e);
            i = e;
            continue;
        }
        if c.is_ascii_digit()
            || (c == '.'
                && src.as_bytes().get(i + 1).is_some_and(u8::is_ascii_digit)
                && !out
                    .last()
                    .is_some_and(|t| t.kind == "ident" || t.kind == "property"))
        {
            let e = if c == '.' {
                number_end(src, i + 1)
            } else {
                number_end(src, i)
            };
            push(&mut out, "number", i, e);
            i = e;
            continue;
        }
        if spec.decorators && c == '@' && ch(src, i + 1).is_some_and(is_id_start) {
            let mut e = ident_end(src, i + 1);
            while src.as_bytes().get(e) == Some(&b'.') && ch(src, e + 1).is_some_and(is_id_start) {
                e = ident_end(src, e + 1);
            }
            push(&mut out, "macro", i, e);
            i = e;
            continue;
        }
        if is_id_start(c) || (spec.template && c == '$') {
            let mut e = ident_end(src, i + c.len_utf8());
            if spec.template {
                while ch(src, e) == Some('$') {
                    e = ident_end(src, e + 1);
                }
            }
            let w = &src[i..e];
            let prev_dot = out
                .iter()
                .rev()
                .find(|t| t.kind != "space")
                .is_some_and(|t| &src[t.a..t.b] == "." || &src[t.a..t.b] == "?.");
            let called =
                src.as_bytes().get(e) == Some(&b'(') || (spec.rust && src[e..].starts_with("::<"));
            let kind = if spec.rust
                && src.as_bytes().get(e) == Some(&b'!')
                && src.as_bytes().get(e + 1) != Some(&b'=')
            {
                e += 1;
                "macro"
            } else if prev_dot {
                if called { "function" } else { "property" }
            } else if spec.keywords.contains(&w) {
                "keyword"
            } else if spec.constants.contains(&w) {
                "constant"
            } else if spec.defines_fn.contains(&last_word) {
                "function"
            } else if spec.defines_type.contains(&last_word) || spec.types.contains(&w) {
                "type"
            } else if called {
                "function"
            } else if all_caps(w) && w.chars().count() > 1 {
                "constant"
            } else if w.chars().next().is_some_and(char::is_uppercase) && !spec.keys {
                "type"
            } else {
                "ident"
            };
            push(&mut out, kind, i, e);
            last_word = &src[i..e];
            i = e;
            continue;
        }
        if OPERATOR_CHARS.contains(c) {
            let mut e = i;
            while let Some(d) = ch(src, e) {
                if !OPERATOR_CHARS.contains(d) {
                    break;
                }
                // `//` or `#` starting a comment ends the operator
                if e > i && spec.line_comments.iter().any(|lc| src[e..].starts_with(lc)) {
                    break;
                }
                if e > i
                    && spec
                        .block_comment
                        .is_some_and(|(o, _)| src[e..].starts_with(o))
                {
                    break;
                }
                e += d.len_utf8();
            }
            push(&mut out, "operator", i, e);
            i = e;
            continue;
        }
        let e = next_i(src, i);
        let kind = if c == '.' && src[e..].starts_with("..") {
            "operator"
        } else {
            "punct"
        };
        push(&mut out, kind, i, e);
        if c != '.' {
            last_word = "";
        }
        i = e;
    }
    out
}

// ── shell ────────────────────────────────────────────────────────────

const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "function", "return", "export", "local", "readonly", "select", "time",
];

fn shell(src: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let n = src.len();
    let mut i = 0usize;
    // a word here is a command's name
    let mut command = true;
    let mut line_start = true;
    while i < n {
        let c = ch(src, i).expect("in range");
        if c == '\n' {
            let e = spaces(src, i);
            out.push(Tok {
                kind: "space",
                a: i,
                b: e,
            });
            i = e;
            command = true;
            line_start = true;
            continue;
        }
        if c.is_whitespace() {
            let mut e = i;
            while let Some(d) = ch(src, e) {
                if !d.is_whitespace() || d == '\n' {
                    break;
                }
                e += d.len_utf8();
            }
            out.push(Tok {
                kind: "space",
                a: i,
                b: e,
            });
            i = e;
            continue;
        }
        // a prompt: `$ ` or `% ` or `> ` opening a console line
        if line_start && matches!(c, '$' | '%' | '>') && matches!(ch(src, i + 1), Some(' ') | None)
        {
            out.push(Tok {
                kind: "punct",
                a: i,
                b: i + 1,
            });
            i += 1;
            line_start = false;
            command = true;
            continue;
        }
        line_start = false;
        if c == '#' {
            let e = line_end(src, i);
            out.push(Tok {
                kind: "comment",
                a: i,
                b: e,
            });
            i = e;
            continue;
        }
        if c == '\'' {
            let e = string_end(src, i + 1, "'", false, true);
            out.push(Tok {
                kind: "string",
                a: i,
                b: e,
            });
            i = e;
            command = false;
            continue;
        }
        if c == '"' {
            let e = string_end(src, i + 1, "\"", true, true);
            out.push(Tok {
                kind: "string",
                a: i,
                b: e,
            });
            i = e;
            command = false;
            continue;
        }
        if c == '$' {
            let e = match ch(src, i + 1) {
                Some('{') => src[i..].find('}').map(|k| i + k + 1).unwrap_or(n),
                Some('(') => i + 2,
                Some(d) if is_id_start(d) => ident_end(src, i + 1),
                Some(d) if d.is_ascii_digit() || "?#@*!$-".contains(d) => i + 2,
                _ => i + 1,
            };
            let kind = if e == i + 2 && ch(src, i + 1) == Some('(') {
                "punct"
            } else {
                "property"
            };
            if kind == "punct" {
                command = true;
            }
            out.push(Tok { kind, a: i, b: e });
            i = e;
            continue;
        }
        if c == '\\' {
            let e = next_i(src, i);
            let e = if e < n { next_i(src, e) } else { e };
            let kind = if src[i..e].ends_with('\n') {
                "punct"
            } else {
                "string"
            };
            out.push(Tok { kind, a: i, b: e });
            i = e;
            continue;
        }
        if "|&;<>()`".contains(c) {
            let mut e = i + 1;
            while let Some(d) = ch(src, e) {
                if !"|&;<>".contains(d) || !"|&;<>".contains(c) {
                    break;
                }
                e += 1;
            }
            let t = &src[i..e];
            out.push(Tok {
                kind: if matches!(t, "(" | ")" | "`") {
                    "punct"
                } else {
                    "operator"
                },
                a: i,
                b: e,
            });
            if !matches!(t, ">" | ">>" | "<" | "2>" | ")") {
                command = true;
            }
            i = e;
            continue;
        }
        // a word
        let mut e = i;
        while let Some(d) = ch(src, e) {
            if d.is_whitespace() || "|&;<>()`'\"$\\".contains(d) {
                break;
            }
            e += d.len_utf8();
        }
        let w = &src[i..e];
        // `NAME=value`
        if command
            && let Some(eq) = w.find('=')
            && eq > 0
            && w[..eq].chars().all(is_id)
            && w.chars().next().is_some_and(is_id_start)
        {
            out.push(Tok {
                kind: "property",
                a: i,
                b: i + eq,
            });
            out.push(Tok {
                kind: "operator",
                a: i + eq,
                b: i + eq + 1,
            });
            if i + eq + 1 < e {
                out.push(Tok {
                    kind: "string",
                    a: i + eq + 1,
                    b: e,
                });
            }
            i = e;
            continue;
        }
        let kind = if SHELL_KEYWORDS.contains(&w) && (command || w == "in" || w == "do") {
            "keyword"
        } else if command {
            "function"
        } else if w.starts_with('-') && w.len() > 1 {
            "constant"
        } else if w.bytes().all(|b| b.is_ascii_digit()) {
            "number"
        } else {
            "ident"
        };
        command = SHELL_KEYWORDS.contains(&w)
            && command
            && !matches!(
                w,
                "for" | "case" | "select" | "export" | "local" | "readonly" | "function" | "return"
            );
        out.push(Tok {
            kind,
            a: i,
            b: e.max(next_i(src, i).min(n)),
        });
        i = e.max(next_i(src, i).min(n));
    }
    out
}

// ── TOML ─────────────────────────────────────────────────────────────

fn toml(src: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let n = src.len();
    let mut i = 0usize;
    // the open `[` (array) and `{` (inline table) of a value
    let mut nest: Vec<u8> = Vec::new();
    let mut key = true;
    // inside a `[table]` header's brackets
    let mut header = false;
    while i < n {
        let c = ch(src, i).expect("in range");
        let rest = &src[i..];
        if c.is_whitespace() {
            let e = spaces(src, i);
            if src[i..e].contains('\n') && nest.is_empty() {
                key = true;
                header = false;
            }
            out.push(Tok {
                kind: "space",
                a: i,
                b: e,
            });
            i = e;
            continue;
        }
        if c == '#' {
            let e = line_end(src, i);
            out.push(Tok {
                kind: "comment",
                a: i,
                b: e,
            });
            i = e;
            continue;
        }
        let at_line_start = src[..i].rfind('\n').map(|k| k + 1).unwrap_or(0);
        let first = src[at_line_start..i].trim().is_empty();
        if c == '[' && first && nest.is_empty() {
            let e = if rest.starts_with("[[") { i + 2 } else { i + 1 };
            out.push(Tok {
                kind: "punct",
                a: i,
                b: e,
            });
            header = true;
            key = true;
            i = e;
            continue;
        }
        if header && c == ']' {
            let e = if rest.starts_with("]]") { i + 2 } else { i + 1 };
            out.push(Tok {
                kind: "punct",
                a: i,
                b: e,
            });
            header = false;
            i = e;
            continue;
        }
        if rest.starts_with("\"\"\"") || rest.starts_with("'''") {
            let q = &rest[..3];
            let e = string_end(src, i + 3, q, q == "\"\"\"", true);
            out.push(Tok {
                kind: "string",
                a: i,
                b: e,
            });
            i = e;
            continue;
        }
        if c == '"' || c == '\'' {
            let e = string_end(src, i + 1, &c.to_string(), c == '"', false);
            let kind = if header {
                "type"
            } else if key {
                "property"
            } else {
                "string"
            };
            out.push(Tok { kind, a: i, b: e });
            i = e;
            continue;
        }
        if c == '=' {
            out.push(Tok {
                kind: "operator",
                a: i,
                b: i + 1,
            });
            key = false;
            i += 1;
            continue;
        }
        if c == '[' || c == '{' {
            nest.push(c as u8);
            key = c == '{';
            out.push(Tok {
                kind: "punct",
                a: i,
                b: i + 1,
            });
            i += 1;
            continue;
        }
        if c == ']' || c == '}' {
            nest.pop();
            key = false;
            out.push(Tok {
                kind: "punct",
                a: i,
                b: i + 1,
            });
            i += 1;
            continue;
        }
        if c == ',' {
            key = nest.last() == Some(&b'{');
            out.push(Tok {
                kind: "punct",
                a: i,
                b: i + 1,
            });
            i += 1;
            continue;
        }
        if c == '.' && (key || header) {
            out.push(Tok {
                kind: "punct",
                a: i,
                b: i + 1,
            });
            i += 1;
            continue;
        }
        // a bare word: a key, a header's name, or a value
        let mut e = i;
        while let Some(d) = ch(src, e) {
            let bare = d.is_alphanumeric() || matches!(d, '_' | '-');
            let value = !key && !header && matches!(d, ':' | '.' | '+');
            if !(bare || value) {
                break;
            }
            e += d.len_utf8();
        }
        if e == i {
            e = next_i(src, i);
            out.push(Tok {
                kind: "punct",
                a: i,
                b: e,
            });
            i = e;
            continue;
        }
        let w = &src[i..e];
        let kind = if header {
            "type"
        } else if key {
            "property"
        } else if matches!(w, "true" | "false") {
            "constant"
        } else if w.starts_with(|d: char| d.is_ascii_digit() || d == '+' || d == '-')
            || matches!(w, "inf" | "nan")
        {
            "number"
        } else {
            "ident"
        };
        out.push(Tok { kind, a: i, b: e });
        i = e;
    }
    out
}

// ── Markdown ─────────────────────────────────────────────────────────

fn markdown(src: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0usize;
    let n = src.len();
    // the open fence: its marker
    let mut fence: Option<String> = None;
    while i < n {
        let le = line_end(src, i);
        let line = &src[i..le];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let marker: String = trimmed
            .chars()
            .take_while(|&d| d == '`' || d == '~')
            .collect();
        let is_fence = indent < 4
            && marker.len() >= 3
            && marker
                .chars()
                .all(|d| d == marker.chars().next().unwrap_or('`'));
        if let Some(open) = &fence {
            if is_fence
                && marker.starts_with(open.as_str())
                && trimmed[marker.len()..].trim().is_empty()
            {
                push_line(&mut out, src, i, le, "punct");
                fence = None;
            } else {
                push_line(&mut out, src, i, le, "string");
            }
        } else if is_fence {
            out.push(Tok {
                kind: "space",
                a: i,
                b: i + indent,
            });
            out.push(Tok {
                kind: "punct",
                a: i + indent,
                b: i + indent + marker.len(),
            });
            out.push(Tok {
                kind: "type",
                a: i + indent + marker.len(),
                b: le,
            });
            fence = Some(marker);
        } else if indent < 4 && trimmed.starts_with('#') && {
            let h = trimmed.chars().take_while(|&d| d == '#').count();
            h <= 6 && trimmed[h..].chars().next().is_none_or(char::is_whitespace)
        } {
            out.push(Tok {
                kind: "space",
                a: i,
                b: i + indent,
            });
            out.push(Tok {
                kind: "keyword",
                a: i + indent,
                b: le,
            });
        } else if indent < 4 && trimmed.starts_with('>') {
            out.push(Tok {
                kind: "space",
                a: i,
                b: i + indent,
            });
            out.push(Tok {
                kind: "comment",
                a: i + indent,
                b: le,
            });
        } else {
            out.push(Tok {
                kind: "space",
                a: i,
                b: i + indent,
            });
            let mut j = i + indent;
            // a list item's marker
            let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
            let m = if trimmed.starts_with(['-', '*', '+']) && trimmed[1..].starts_with(' ') {
                1
            } else if digits > 0
                && trimmed[digits..].starts_with(['.', ')'])
                && trimmed[digits + 1..].starts_with(' ')
            {
                digits + 1
            } else {
                0
            };
            if m > 0 {
                out.push(Tok {
                    kind: "punct",
                    a: j,
                    b: j + m,
                });
                j += m;
            }
            inline(src, j, le, &mut out);
        }
        if le < n {
            out.push(Tok {
                kind: "space",
                a: le,
                b: le + 1,
            });
        }
        i = le + 1;
    }
    out
}

/// A line as one token of `kind`, its leading spaces as a space.
fn push_line(out: &mut Vec<Tok>, src: &str, a: usize, b: usize, kind: &'static str) {
    let s = spaces(src, a).min(b);
    out.push(Tok {
        kind: "space",
        a,
        b: s,
    });
    out.push(Tok { kind, a: s, b });
}

/// A Markdown line's inline text: code spans, links, tags and emphasis
/// marks among the words.
fn inline(src: &str, mut i: usize, end: usize, out: &mut Vec<Tok>) {
    while i < end {
        let c = ch(src, i).expect("in range");
        let rest = &src[i..end];
        if c.is_whitespace() {
            let mut e = i;
            while let Some(d) = ch(src, e) {
                if e >= end || !d.is_whitespace() {
                    break;
                }
                e += d.len_utf8();
            }
            out.push(Tok {
                kind: "space",
                a: i,
                b: e,
            });
            i = e;
            continue;
        }
        if c == '`' {
            let ticks = rest.bytes().take_while(|&b| b == b'`').count();
            let close = "`".repeat(ticks);
            let e = rest[ticks..]
                .find(&close)
                .map(|k| i + ticks + k + ticks)
                .unwrap_or(i + ticks);
            out.push(Tok {
                kind: if e > i + ticks { "string" } else { "punct" },
                a: i,
                b: e,
            });
            i = e;
            continue;
        }
        if c == '<'
            && ch(src, i + 1).is_some_and(|d| d.is_ascii_alphabetic() || d == '/' || d == '!')
            && let Some(k) = rest.find('>')
        {
            let inner = &rest[1..k];
            let kind = if inner.contains("://") || inner.contains('@') {
                "property"
            } else {
                "type"
            };
            out.push(Tok {
                kind,
                a: i,
                b: i + k + 1,
            });
            i += k + 1;
            continue;
        }
        if c == ']'
            && rest[1..].starts_with('(')
            && let Some(k) = rest.find(')')
        {
            out.push(Tok {
                kind: "punct",
                a: i,
                b: i + 2,
            });
            out.push(Tok {
                kind: "property",
                a: i + 2,
                b: i + k,
            });
            out.push(Tok {
                kind: "punct",
                a: i + k,
                b: i + k + 1,
            });
            i += k + 1;
            continue;
        }
        if "*_~[]!\\|".contains(c) {
            let e = if c == '\\' {
                next_i(src, next_i(src, i).min(end)).min(end).max(i + 1)
            } else {
                let mut e = i;
                while ch(src, e) == Some(c) && e < end {
                    e += 1;
                }
                e
            };
            out.push(Tok {
                kind: if c == '\\' { "string" } else { "operator" },
                a: i,
                b: e,
            });
            i = e;
            continue;
        }
        let mut e = i;
        while let Some(d) = ch(src, e) {
            if e >= end || d.is_whitespace() || "`<*_~[]!\\|".contains(d) {
                break;
            }
            e += d.len_utf8();
        }
        if e == i {
            e = next_i(src, i);
        }
        let w = &src[i..e];
        let kind = if w.starts_with("http://") || w.starts_with("https://") {
            "property"
        } else {
            "ident"
        };
        out.push(Tok { kind, a: i, b: e });
        i = e;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str, lang: &str) -> Vec<(String, &'static str)> {
        tokens(src, lang)
            .into_iter()
            .filter(|t| t.kind != "space")
            .map(|t| (src[t.a..t.b].to_string(), t.kind))
            .collect()
    }

    fn has(src: &str, lang: &str, want: &[(&str, &str)]) {
        let k = kinds(src, lang);
        for (t, kind) in want {
            assert!(
                k.iter().any(|(x, y)| x == t && y == kind),
                "{lang}: {t:?} as {kind} in {k:?}"
            );
        }
    }

    const SAMPLES: &[&str] = &[
        "",
        "x",
        "\"unterminated",
        "'",
        "r#\"raw",
        "#[",
        "/* open",
        "`tmpl ${",
        "\"\"\"doc",
        "$",
        "${",
        "\\",
        "[[",
        "```",
        "é ✓ 'c' \"ü\"\n\t\r\n",
        "a.b.c = 1e-3 + 0x1F // c\n# h\n",
        "<a href=\"x\">[t](u) `c` *e* **b**",
        "key = [1, {a = \"b\"}, 'c'] # n\n[t.x]\n",
        "$ olang run --flag=1 x | grep \"a $B\" > out.txt # c\nX=1 cmd\n",
    ];

    #[test]
    fn every_language_covers_any_text() {
        for lang in ["python", "rust", "js", "json", "shell", "toml", "markdown"] {
            for src in SAMPLES {
                let toks = tokens(src, lang);
                let joined: String = toks.iter().map(|t| &src[t.a..t.b]).collect();
                assert_eq!(&joined, src, "{lang}");
                let mut at = 0;
                for t in &toks {
                    assert_eq!(t.a, at, "{lang}: {src:?} {toks:?}");
                    assert!(t.b > t.a);
                    at = t.b;
                }
            }
        }
    }

    #[test]
    fn python() {
        has(
            "@cache\ndef fib(n: int) -> int:\n    \"\"\"doc\n    more\"\"\"\n    return n if n < 2 else fib(n - 1)  # c\nclass Point: pass\nx = f'{a}' + rb\"z\" + None\n",
            "python",
            &[
                ("@cache", "macro"),
                ("def", "keyword"),
                ("fib", "function"),
                ("int", "type"),
                ("\"\"\"doc\n    more\"\"\"", "string"),
                ("# c", "comment"),
                ("Point", "type"),
                ("f'{a}'", "string"),
                ("rb\"z\"", "string"),
                ("None", "constant"),
                ("2", "number"),
                ("<", "operator"),
            ],
        );
    }

    #[test]
    fn rust() {
        has(
            "#[derive(Debug)]\npub fn go<'a>(s: &'a str) -> Option<u8> {\n    let c = 'x'; // c\n    println!(\"{}\", r#\"raw \"q\"\"#);\n    Some(MAX)\n}\n",
            "rust",
            &[
                ("#[derive(Debug)]", "macro"),
                ("pub", "keyword"),
                ("go", "function"),
                ("'a", "type"),
                ("str", "type"),
                ("Option", "type"),
                ("'x'", "string"),
                ("// c", "comment"),
                ("println!", "macro"),
                ("r#\"raw \"q\"\"#", "string"),
                ("Some", "constant"),
                ("MAX", "constant"),
            ],
        );
    }

    #[test]
    fn js() {
        has(
            "const w = new Worker(`a ${b}`); // c\nexport function run(x) { return x?.y ?? null }\n",
            "js",
            &[
                ("const", "keyword"),
                ("Worker", "function"),
                ("`a ${b}`", "string"),
                ("run", "function"),
                ("y", "property"),
                ("??", "operator"),
                ("null", "constant"),
            ],
        );
    }

    #[test]
    fn json() {
        has(
            "{\"name\": \"olang\", \"n\": -1.5e3, \"ok\": true, \"x\": null}",
            "json",
            &[
                ("\"name\"", "property"),
                ("\"olang\"", "string"),
                ("1.5e3", "number"),
                ("true", "constant"),
                ("null", "constant"),
                ("{", "punct"),
            ],
        );
    }

    #[test]
    fn shell() {
        has(
            "$ cargo build --release -j 4 # build\nif [ -f x ]; then echo \"$HOME\" | tr a b; fi\nOLANG=1 olang test\n",
            "shell",
            &[
                ("$", "punct"),
                ("cargo", "function"),
                ("build", "ident"),
                ("--release", "constant"),
                ("4", "number"),
                ("# build", "comment"),
                ("if", "keyword"),
                ("then", "keyword"),
                ("echo", "function"),
                ("\"$HOME\"", "string"),
                ("|", "operator"),
                ("tr", "function"),
                ("fi", "keyword"),
                ("OLANG", "property"),
                ("olang", "function"),
            ],
        );
    }

    #[test]
    fn toml() {
        has(
            "[package]\nname = \"olang\" # n\nversion = \"1.0\"\n[dependencies.web]\nfeatures = [\"a\", 'b']\ninline = { x = 1, y = true }\nwhen = 1979-05-27T07:32:00Z\n",
            "toml",
            &[
                ("package", "type"),
                ("name", "property"),
                ("\"olang\"", "string"),
                ("# n", "comment"),
                ("dependencies", "type"),
                ("web", "type"),
                ("'b'", "string"),
                ("x", "property"),
                ("y", "property"),
                ("1", "number"),
                ("true", "constant"),
                ("1979-05-27T07:32:00Z", "number"),
            ],
        );
    }

    #[test]
    fn markdown() {
        has(
            "# Title\n\nSome `code` and [a link](https://x.y).\n\n- item\n> quote\n```olang\nlet x = 1\n```\n",
            "markdown",
            &[
                ("# Title", "keyword"),
                ("`code`", "string"),
                ("https://x.y", "property"),
                ("-", "punct"),
                ("> quote", "comment"),
                ("```", "punct"),
                ("olang", "type"),
                ("let x = 1", "string"),
            ],
        );
    }

    #[test]
    fn names() {
        assert_eq!(language("Python"), Some("python"));
        assert_eq!(language("bash"), Some("shell"));
        assert_eq!(language("ol"), Some("olang"));
        assert_eq!(language("cobol"), None);
    }
}

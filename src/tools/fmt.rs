//! `olang fmt [paths] [--check]` — a conservative, safety-verified
//! formatter.
//!
//! Two layers, applied only *outside* string literals and comments:
//!
//! **Line hygiene** — CRLF becomes LF, trailing whitespace is stripped,
//! leading tabs become 4 spaces, runs of blank lines collapse to one,
//! the file ends with exactly one newline.
//!
//! **Token respacing** — each code line is tokenized and re-emitted
//! with canonical spacing: one space around binary operators and `=`;
//! `f(x, y)` not `f( x ,y )`; commas and `:` glue left and breathe
//! right; calls and indexing glue to their value (`f(x)`, `xs[i]`)
//! while keywords keep their space (`if (x)`); unary `-`/`+`/`!` glue
//! to their operand; ranges glue (`1..5`); braces breathe (`{ x }`,
//! `#{ "a": 1 }`) except empty pairs (`{}`, `#{}`). The rules were
//! calibrated against the whole in-repo corpus — the shipped style IS
//! the spec.
//!
//! What it deliberately does not do: re-indent (leading whitespace is
//! the author's), re-wrap, or reflow lines — and two alignments are
//! recognized as intentional and preserved: two or more spaces before
//! `=>` (match-arm tables) and before a trailing comment.
//!
//! The safety property is absolute: the formatted source must parse to
//! a raw AST identical to the original's (spans aside), or the file is
//! left untouched and reported. `--check` reports files that would
//! change and exits non-zero without writing anything.

use crate::Parser;
use colored::*;

/// Where the scanner is, with respect to string-ish tokens that whitespace
/// rules must not reach into.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Code,
    DoubleQuote,       // "..." — single line in practice, tracked for safety
    RawString,         // r"..." — may span lines
    Template,          // `...` — may span lines
    TemplateExpr(u32), // ${ ... } inside a template, brace-depth counted
}

/// Track string state across a line, returning the mode at line end.
/// `starts_in` is the mode the line begins in.
fn scan_line_mode(line: &str, starts_in: Mode) -> Mode {
    let mut mode = starts_in;
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let next = bytes.get(i + 1).copied();
        match mode {
            Mode::Code => match c {
                '/' if next == Some('/') => return mode, // comment: rest of line is inert
                'r' if next == Some('"') => {
                    mode = Mode::RawString;
                    i += 1;
                }
                '"' => mode = Mode::DoubleQuote,
                '`' => mode = Mode::Template,
                _ => {}
            },
            Mode::DoubleQuote => match c {
                '\\' => i += 1, // skip escaped char
                '"' => mode = Mode::Code,
                _ => {}
            },
            Mode::RawString => match c {
                '\\' if next == Some('"') => i += 1,
                '"' => mode = Mode::Code,
                _ => {}
            },
            Mode::Template => match c {
                '\\' => i += 1,
                '`' => mode = Mode::Code,
                '$' if next == Some('{') => {
                    mode = Mode::TemplateExpr(0);
                    i += 1;
                }
                _ => {}
            },
            Mode::TemplateExpr(depth) => match c {
                '{' => mode = Mode::TemplateExpr(depth + 1),
                '}' => {
                    mode = if depth == 0 {
                        Mode::Template
                    } else {
                        Mode::TemplateExpr(depth - 1)
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    mode
}

/// One lexical token of a code line, with the whitespace gap that
/// preceded it in the original — kept so deliberate alignment (before
/// `=>`, before a trailing comment) can survive normalization.
#[derive(Debug)]
struct Tok {
    gap: usize,
    text: String,
    kind: TokKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TokKind {
    Word,    // identifier, keyword, or number (incl. 0x.., 1_000, 1.5)
    Str,     // any string form, verbatim (".."/r".."/`..`/'c')
    Comment, // // to end of line, verbatim
    Op,      // operator or punctuation
}

/// Keywords that read as words: an opening `(` or `[` after one is an
/// expression, not a call, and an operator after one is unary.
fn is_keyword(w: &str) -> bool {
    matches!(
        w,
        "let"
            | "mut"
            | "fn"
            | "share"
            | "if"
            | "else"
            | "while"
            | "for"
            | "in"
            | "match"
            | "return"
            | "break"
            | "continue"
            | "use"
            | "type"
            | "trait"
            | "impl"
            | "test"
            | "spawn"
            | "meta"
            | "error"
            | "catch"
            | "true"
            | "false"
    )
}

const MULTI_OPS: &[&str] = &[
    "...", "|>", "=>", "->", "==", "!=", "<=", ">=", "&&", "||", "..", "#{",
];

/// Tokenize one line known to start in Code mode. Returns the tokens,
/// the mode at line end, and — when a multi-line string opens on this
/// line — the verbatim tail from its opening character.
fn tokenize_line(line: &str, mut mode: Mode) -> (Vec<Tok>, Mode, Option<String>) {
    debug_assert!(mode == Mode::Code);
    let chars: Vec<char> = line.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let mut gap = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' || c == '\t' {
            gap += 1;
            i += 1;
            continue;
        }
        // Comment: verbatim to end of line.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            toks.push(Tok {
                gap,
                text: chars[i..].iter().collect(),
                kind: TokKind::Comment,
            });
            return (toks, Mode::Code, None);
        }
        // String forms. A form that does not close on this line hands
        // the tail back verbatim (mode carries to the next line).
        let (is_str, raw, template) = match c {
            '"' => (true, false, false),
            '`' => (true, false, true),
            'r' if chars.get(i + 1) == Some(&'"') => (true, true, false),
            '\'' => (true, false, false),
            _ => (false, false, false),
        };
        if is_str {
            let start = i;
            let rest: String = chars[i..].iter().collect();
            let end_mode = scan_line_mode(&rest, Mode::Code);
            if end_mode != Mode::Code && (template || raw) {
                // Multi-line string opens here: tail is content.
                mode = end_mode;
                let tail: String = chars[start..].iter().collect();
                return (toks, mode, Some(tail));
            }
            // Single-line: walk to the closing quote with the same
            // escape rules scan_line_mode uses.
            let mut j = start;
            let mut m = Mode::Code;
            let closer = if c == '\'' {
                '\''
            } else if template {
                '`'
            } else {
                '"'
            };
            if c == '\'' {
                // char literal: 'x' — exactly one char, no escapes.
                j = start + 2;
                if chars.get(j) == Some(&'\'') {
                    j += 1;
                }
            } else {
                loop {
                    j += 1;
                    let cj = match chars.get(j) {
                        Some(x) => *x,
                        None => break,
                    };
                    match m {
                        Mode::Code => {
                            if raw {
                                if cj == '\\' && chars.get(j + 1) == Some(&'"') {
                                    j += 1;
                                } else if cj == '"' {
                                    j += 1;
                                    break;
                                }
                            } else if cj == '\\' {
                                j += 1;
                            } else if template && cj == '$' && chars.get(j + 1) == Some(&'{') {
                                m = Mode::TemplateExpr(0);
                                j += 1;
                            } else if cj == closer {
                                j += 1;
                                break;
                            }
                        }
                        Mode::TemplateExpr(d) => {
                            if cj == '{' {
                                m = Mode::TemplateExpr(d + 1);
                            } else if cj == '}' {
                                m = if d == 0 {
                                    Mode::Code
                                } else {
                                    Mode::TemplateExpr(d - 1)
                                };
                            } else if cj == '"' {
                                // string inside an interpolation: skip it
                                let inner: String = chars[j..].iter().collect();
                                let mut k = 1;
                                let ic: Vec<char> = inner.chars().collect();
                                while k < ic.len() {
                                    if ic[k] == '\\' {
                                        k += 1;
                                    } else if ic[k] == '"' {
                                        break;
                                    }
                                    k += 1;
                                }
                                j += k;
                            }
                        }
                        _ => {}
                    }
                }
            }
            let j = j.min(chars.len());
            toks.push(Tok {
                gap,
                text: chars[start..j].iter().collect(),
                kind: TokKind::Str,
            });
            gap = 0;
            i = j;
            continue;
        }
        // Word: identifier or number (numbers absorb alnum and `_`;
        // a `.` joins only when a digit follows and it is not `..`).
        if c.is_alphanumeric() || c == '_' {
            let start = i;
            let numeric = c.is_ascii_digit();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if numeric
                && chars.get(i) == Some(&'.')
                && chars
                    .get(i + 1)
                    .map(|d| d.is_ascii_digit())
                    .unwrap_or(false)
            {
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    i += 1;
                }
            }
            toks.push(Tok {
                gap,
                text: chars[start..i].iter().collect(),
                kind: TokKind::Word,
            });
            gap = 0;
            continue;
        }
        // Operator/punctuation: longest match first.
        let rest: String = chars[i..].iter().collect();
        let op = MULTI_OPS
            .iter()
            .find(|m| rest.starts_with(**m))
            .map(|m| m.to_string())
            .unwrap_or_else(|| c.to_string());
        i += op.chars().count();
        toks.push(Tok {
            gap,
            text: op,
            kind: TokKind::Op,
        });
        gap = 0;
    }
    (toks, mode, None)
}

/// The token respacer: emit one code line with canonical spacing.
/// Alignment survives in exactly two places — runs of two or more
/// spaces before `=>` and before a trailing comment are the author's.
fn respace(indent: &str, toks: &[Tok], tail: Option<&str>) -> String {
    // A keyword after `.` is a member name (`proc.spawn`), not syntax:
    // demote it to a plain word for spacing decisions.
    let toks: Vec<Tok> = toks
        .iter()
        .enumerate()
        .map(|(i, t)| Tok {
            gap: t.gap,
            text: t.text.clone(),
            kind: if t.kind == TokKind::Word
                && i > 0
                && toks[i - 1].text == "."
                && is_keyword(&t.text)
            {
                TokKind::Str // value-like, never keyword-spaced
            } else {
                t.kind
            },
        })
        .collect();
    let toks = &toks[..];
    // A `-`/`+` after anything but a value is a sign and glues to its
    // operand; `!`, `...`, and `@` always do.
    let mut glue_right = vec![false; toks.len()];
    for (i, t) in toks.iter().enumerate() {
        if t.kind == TokKind::Op {
            glue_right[i] = match t.text.as_str() {
                "!" | "..." | "@" => true,
                "-" | "+" => i > 0 && !value_like(&toks[i - 1]),
                _ => false,
            };
        }
    }
    let mut out = String::from(indent);
    for (idx, t) in toks.iter().enumerate() {
        let space = if idx == 0 || glue_right[idx - 1] {
            0
        } else if (t.text == "=>" || t.kind == TokKind::Comment) && t.gap >= 2 {
            t.gap // preserved alignment
        } else {
            spacing(&toks[idx - 1], t)
        };
        for _ in 0..space {
            out.push(' ');
        }
        if t.kind == TokKind::Comment {
            out.push_str(t.text.trim_end());
        } else {
            out.push_str(&t.text);
        }
    }
    if let Some(tail) = tail {
        if let Some(last) = toks.last() {
            let dummy = Tok {
                gap: 0,
                text: "\"".to_string(),
                kind: TokKind::Str,
            };
            if !glue_right[toks.len() - 1] {
                for _ in 0..spacing(last, &dummy) {
                    out.push(' ');
                }
            }
        }
        out.push_str(tail);
    }
    out
}

/// Is this token something a following `-`/`+` would be *binary* after
/// (and something a call's `(` glues to)?
fn value_like(p: &Tok) -> bool {
    match p.kind {
        TokKind::Word => !is_keyword(&p.text),
        TokKind::Str => true,
        TokKind::Op => matches!(p.text.as_str(), ")" | "]" | "}" | "?"),
        TokKind::Comment => false,
    }
}

/// Spaces to emit between adjacent tokens `p` and `t`.
fn spacing(p: &Tok, t: &Tok) -> usize {
    let pt = p.text.as_str();
    let tt = t.text.as_str();

    if t.kind == TokKind::Comment {
        return 1;
    }
    // Glue-left punctuation…
    if matches!(tt, "," | ";" | ")" | "]" | "?" | ":" | ".") {
        return 0;
    }
    // …and glue-right openers/joiners.
    if matches!(pt, "(" | "[" | "." | ":") {
        return if pt == ":" { 1 } else { 0 };
    }
    // Ranges glue on both sides: 1..5.
    if tt == ".." || pt == ".." {
        return 0;
    }
    // Empty braces glue: `{}` and `#{}`. Nested closers breathe (`} }`).
    if tt == "}" && matches!(pt, "{" | "#{") {
        return 0;
    }
    // Calls and indexing glue to a value; a literal after a keyword or
    // an operator keeps its space: `f(x)` vs `if (x)`.
    if (tt == "(" || tt == "[") && value_like(p) {
        return 0;
    }
    // Everything else — words, strings, braces, binary operators —
    // one space.
    1
}

/// Apply the whitespace rules. Lines that begin inside a multi-line string
/// are passed through byte-for-byte.
pub fn format_source(source: &str) -> String {
    let source = source.replace("\r\n", "\n");
    let mut out: Vec<String> = Vec::new();
    let mut mode = Mode::Code;
    let mut last_was_blank = false;

    for line in source.split('\n') {
        let starts_in_string = mode != Mode::Code;
        let end_mode = scan_line_mode(line, mode);

        if starts_in_string {
            // Inside a template/raw string: whitespace is content.
            out.push(line.to_string());
            last_was_blank = false;
        } else {
            // Leading tabs -> 4 spaces (indentation only).
            let mut rebuilt = String::new();
            let mut in_indent = true;
            for c in line.chars() {
                match c {
                    '\t' if in_indent => rebuilt.push_str("    "),
                    c => {
                        if c != ' ' {
                            in_indent = false;
                        }
                        rebuilt.push(c);
                    }
                }
            }
            // Split indentation from content and respace the tokens.
            // A line that opens a multi-line string keeps everything
            // from the opening quote verbatim (the tail).
            let indent_end = rebuilt
                .char_indices()
                .find(|(_, c)| *c != ' ')
                .map(|(i, _)| i)
                .unwrap_or(rebuilt.len());
            let (indent, content) = rebuilt.split_at(indent_end);
            let (toks, _mode_after, tail) = tokenize_line(content, Mode::Code);
            let cleaned = if toks.is_empty() && tail.is_none() {
                String::new()
            } else {
                let formatted = respace(indent, &toks, tail.as_deref());
                if end_mode == Mode::Code {
                    formatted.trim_end().to_string()
                } else {
                    formatted
                }
            };

            let is_blank = cleaned.is_empty();
            if is_blank && last_was_blank {
                // collapse runs of blank lines
            } else {
                out.push(cleaned);
            }
            last_was_blank = is_blank;
        }
        mode = end_mode;
    }

    // Exactly one trailing newline.
    while out.last().map(|l| l.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out.push(String::new());
    out.join("\n")
}

/// Format files under the given paths. Returns the process exit code.
pub fn run(paths: &[std::path::PathBuf], check: bool) -> i32 {
    // A named path that doesn't exist is a mistake, not "nothing to format".
    for path in paths {
        if !path.exists() {
            eprintln!("error: path not found: {}", path.display());
            return 1;
        }
    }

    let parser = Parser::new();
    let mut changed: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut parse_errors = 0usize;
    let mut scanned = 0usize;

    for path in paths {
        for file in super::discover_ol_files(path) {
            scanned += 1;
            let original = match std::fs::read_to_string(&file) {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Parse up front: a file that doesn't parse is a real problem
            // fmt must surface (and fail on), not silently report as
            // "all formatted".
            // parse_raw, not parse: formatting needs syntax, not macro
            // expansion — a file importing its meta fns from a sibling
            // module is still formattable, and the gate compares the
            // raw trees on both sides identically.
            let before = match parser.parse_raw(&original) {
                Ok(p) => p,
                Err(e) => {
                    parse_errors += 1;
                    println!("  {} {} (does not parse)", "✗".red().bold(), file.display());
                    println!("      {}", e.to_string().red());
                    continue;
                }
            };
            let formatted = format_source(&original);
            if formatted == original {
                continue;
            }

            // Safety gate: identical AST or nothing.
            match parser.parse_raw(&formatted) {
                Ok(after) if before == after => {
                    changed.push(file.display().to_string());
                    if !check && std::fs::write(&file, &formatted).is_err() {
                        eprintln!("  could not write {}", file.display());
                    }
                }
                _ => {
                    skipped.push(format!(
                        "{} (formatting would change the AST)",
                        file.display()
                    ));
                }
            }
        }
    }

    for s in &skipped {
        println!("  {} {}", "~ skipped".yellow(), s);
    }
    if changed.is_empty() {
        if parse_errors > 0 {
            println!(
                "  {} file{} did not parse",
                parse_errors,
                if parse_errors == 1 { "" } else { "s" }
            );
            return 1;
        }
        println!(
            "  {} ({} file{} scanned)",
            "all formatted".green(),
            scanned,
            if scanned == 1 { "" } else { "s" }
        );
        0
    } else if check {
        for f in &changed {
            println!("  {} {}", "would reformat".red(), f);
        }
        println!(
            "  {} of {} file{} need formatting",
            changed.len(),
            scanned,
            if scanned == 1 { "" } else { "s" }
        );
        1
    } else {
        for f in &changed {
            println!("  {} {}", "reformatted".green(), f);
        }
        // Reformatted the good files, but an unparseable file still fails.
        if parse_errors > 0 { 1 } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_whitespace_and_collapses_blanks() {
        let src = "let x = 1   \n\n\n\nlet y = 2\t\n";
        assert_eq!(format_source(src), "let x = 1\n\nlet y = 2\n");
    }

    #[test]
    fn leading_tabs_become_spaces() {
        let src = "fn f(x) = {\n\tx + 1\n}\n";
        assert_eq!(format_source(src), "fn f(x) = {\n    x + 1\n}\n");
    }

    #[test]
    fn exactly_one_trailing_newline() {
        assert_eq!(format_source("let x = 1"), "let x = 1\n");
        assert_eq!(format_source("let x = 1\n\n\n"), "let x = 1\n");
    }

    #[test]
    fn template_string_interiors_are_untouched() {
        let src = "let t = `line one   \n\ttabbed content\n\n\nstill inside`\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn trailing_spaces_inside_a_double_quoted_string_survive() {
        let src = "let s = \"padded   \" + \"x\"\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn comment_lines_are_cleaned_but_kept() {
        let src = "// a comment with trailing space   \nlet x = 1\n";
        assert_eq!(
            format_source(src),
            "// a comment with trailing space\nlet x = 1\n"
        );
    }

    #[test]
    fn respacing_normalizes_the_shame_cases() {
        assert_eq!(format_source("fn f( x ,y )=x+y\n"), "fn f(x, y) = x + y\n");
        assert_eq!(format_source("let y= [1,2,  3]\n"), "let y = [1, 2, 3]\n");
        assert_eq!(
            format_source("if true=>println(\"x\")\n"),
            "if true => println(\"x\")\n"
        );
        assert_eq!(
            format_source("let r=map(xs,(x)=>x*2)|>fold(0,(a,b)=>a+b)\n"),
            "let r = map(xs, (x) => x * 2) |> fold(0, (a, b) => a + b)\n"
        );
    }

    #[test]
    fn house_style_is_already_formatted() {
        // The rules were calibrated on the corpus: canonical code
        // passes through byte-identical.
        for line in [
            "fn bench(name, thunk) = {\n",
            "let m = #{ \"a\": 1, \"b\": -2 }\n",
            "let s = { ok: false, params: #{} }\n",
            "let xs = range(0, 20000) |> map((v) => v * v)\n",
            "match proc.spawn(argv[0], skip(argv, 1)) {\n",
            "    testing.assert_eq(par[0].kind, \"container\")\n",
            "for i in range(1, n) { acc = acc + i }\n",
            "let t = tok(str_p(s))\n",
            "if e >= 0 => { res = a } else => { res = -1 }\n",
        ] {
            assert_eq!(format_source(line), line, "house style must not churn");
        }
    }

    #[test]
    fn unary_signs_glue_binary_signs_breathe() {
        assert_eq!(format_source("let z = a- -1\n"), "let z = a - -1\n");
        assert_eq!(format_source("let z = f(-1,!b)\n"), "let z = f(-1, !b)\n");
        // A continuation line's leading + is binary, not a sign.
        assert_eq!(
            format_source("let s = a\n    + b\n"),
            "let s = a\n    + b\n"
        );
    }

    #[test]
    fn deliberate_alignment_survives() {
        let arms = "match k {\n    \"berth\"  => green(x),\n    \"alert\"  => red(x)\n}\n";
        assert_eq!(format_source(arms), arms);
        let comments = "let a = 1   // one\nlet bb = 2  // two\n";
        assert_eq!(format_source(comments), comments);
    }

    #[test]
    fn strings_and_templates_are_never_respaced() {
        let src = "let s = \"a  ,  b\" + `x  ${ 1+2 }  y`\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn keywords_after_a_dot_are_member_names() {
        assert_eq!(
            format_source("match proc.spawn(a, b) {\n"),
            "match proc.spawn(a, b) {\n"
        );
    }

    #[test]
    fn idempotent() {
        let src = "let x = 1   \n\n\n\n\tlet y = 2\n";
        let once = format_source(src);
        assert_eq!(format_source(&once), once);
    }
}

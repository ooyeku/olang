//! `olang fix maps [paths] [--check]` — the migration to `.` and `??`
//! (roadmap E3): `map_get(chain, "ident")` becomes `chain.ident`, and
//! `map_get_or(x, "k", d)` becomes `x.k ?? d`.
//!
//! The rewrite is made on the source text, so everything it does not
//! rewrite — layout, comments, blank lines — stays byte for byte; the
//! places come from the grammar's own parse tree (which knows where each
//! call, argument and operator is), and the template interpolations,
//! which the grammar keeps as text, are parsed the way the parser parses
//! them. Two rewrites are exact by definition: `m.key` is `map_get(m,
//! "key")` on every kind a key is read from, and `x ?? d` is
//! `map_get_or(x, "k", d)` — both take the default for an absent key and
//! for a stored `()`. What differs is when `d` is evaluated: `??`
//! evaluates it only when it is needed. So a `map_get_or` whose default
//! may have an effect (a call to a user function, or to a builtin outside
//! the pure set) is **flagged, never rewritten**: the person decides.
//!
//! Left alone, and counted: a key that is not an identifier (or is a
//! reserved word), a computed key, a call whose value is called
//! (`map_get(m, "f")(x)` would read as a method call), a call a pipeline
//! feeds (`x |> map_get(m, "k")` passes `x` first), a call with a
//! comment inside it (the comment would be lost), a macro's arguments
//! (the macro is handed their text), and the calls of a `map_get` or a
//! `map_get_or` the file defines or imports itself.
//!
//! The safety property is `olang fmt`'s: the rewritten file must parse,
//! and to the original's tree once both are read back the same way (each
//! `a.k` as `map_get(a, "k")` and each `a.k ?? d` as `map_get_or(a, "k",
//! d)`), or the file is left untouched and reported. A second run
//! changes nothing.

use crate::parser::{OlangParser, Rule};
use pest::Parser as PestParser;
use pest::iterators::Pair;
use std::path::{Path, PathBuf};

/// What one file's fix did.
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// `map_get(x, "k")` → `x.k`
    pub gets: usize,
    /// `map_get_or(x, "k", d)` → `x.k ?? d`
    pub get_ors: usize,
    /// `map_get_or` calls left because the default may have an effect:
    /// (line, the call's text, first line only).
    pub flagged: Vec<(usize, String)>,
    /// Calls left alone, by reason.
    pub left: Vec<(usize, &'static str)>,
}

/// The fixed text of `src` and what was done, or why the file was left.
pub fn fix_maps(src: &str) -> Result<(String, Report), String> {
    let (bom, body) = match src.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", src),
    };
    // A shebang line is masked with spaces, as the parser does, so every
    // offset is the file's; it is put back afterwards.
    let shebang_end = if body.starts_with("#!") {
        body.find('\n').unwrap_or(body.len())
    } else {
        0
    };
    let masked = if shebang_end > 0 {
        format!("{}{}", " ".repeat(shebang_end), &body[shebang_end..])
    } else {
        body.to_string()
    };
    let parser = crate::Parser::new();
    let before = parser
        .parse_raw(&masked)
        .map_err(|e| format!("does not parse: {}", first_line(&e.to_string())))?;
    let shadowed = [shadows(&masked, "map_get"), shadows(&masked, "map_get_or")];
    if shadowed == [true, true] {
        return Ok((src.to_string(), Report::default()));
    }
    let program = OlangParser::parse(Rule::program, &masked)
        .map_err(|e| format!("does not parse: {}", first_line(&e.to_string())))?
        .next()
        .ok_or_else(|| "does not parse".to_string())?;
    let mut fixer = Fixer {
        src: &masked,
        base: 0,
        file: &masked,
        report: Report::default(),
        shadowed,
    };
    let span = program.as_span();
    let out = format!(
        "{}{}{}",
        &masked[..span.start()],
        fixer.render(program, Ctx::default()).text,
        &masked[span.end()..]
    );
    let report = fixer.report;
    if report.gets + report.get_ors == 0 {
        return Ok((src.to_string(), report));
    }
    // The safety gate: the same program, read back.
    let after = parser
        .parse_raw(&out)
        .map_err(|e| format!("the rewrite does not parse ({}); left as it was", first_line(&e.to_string())))?;
    let (b, a) = (normalized(&before), normalized(&after));
    if b != a {
        if let Some(dir) = std::env::var_os("OLANG_FIX_DEBUG") {
            let dir = std::path::PathBuf::from(dir);
            let _ = std::fs::write(dir.join("before.json"), serde_json::to_string_pretty(&b).unwrap_or_default());
            let _ = std::fs::write(dir.join("after.json"), serde_json::to_string_pretty(&a).unwrap_or_default());
            let _ = std::fs::write(dir.join("after.ol"), &out);
        }
        return Err("the rewrite would change the program; left as it was".to_string());
    }
    let restored = if shebang_end > 0 {
        format!("{}{}", &body[..shebang_end], &out[shebang_end..])
    } else {
        out
    };
    Ok((format!("{bom}{restored}"), report))
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

/// Whether the file declares, binds or imports a `name` of its own: its
/// calls of `name` are not the builtin's (Shuttle's weft shares a
/// `map_get_or` of its own, which evaluates its default eagerly).
fn shadows(src: &str, name: &str) -> bool {
    let word = |rest: &str| {
        rest.trim_start()
            .strip_prefix(name)
            .is_some_and(|after| !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
    };
    src.lines().any(|line| {
        let t = line.trim_start();
        let t = t.strip_prefix("share ").unwrap_or(t).trim_start();
        let decl = ["fn ", "let mut ", "let "].iter().find_map(|kw| t.strip_prefix(kw));
        decl.is_some_and(word)
            || (t.starts_with("use ")
                && t.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .any(|w| w == name))
    })
}

/// A program as plain data with positions dropped and each rewritten
/// form read back as the call it came from: `a.k` as `map_get(a, "k")`,
/// `a.k ?? d` as `map_get_or(a, "k", d)`.
fn normalized(program: &crate::ast::Program) -> serde_json::Value {
    let mut v = serde_json::to_value(program).unwrap_or_default();
    normalize(&mut v);
    v
}

fn normalize(v: &mut serde_json::Value) {
    use serde_json::{Value as J, json};
    // children first
    match v {
        J::Object(o) => {
            for x in o.values_mut() {
                normalize(x);
            }
        }
        J::Array(a) => {
            for x in a.iter_mut() {
                normalize(x);
            }
        }
        _ => {}
    }
    let J::Object(o) = v else { return };
    // Where a node sits is not what it is.
    o.remove("line");
    o.remove("column");
    o.remove("span");
    o.remove("name_span");
    if o.len() != 1 {
        return;
    }
    let (tag, inner) = o.iter().next().map(|(k, v)| (k.clone(), v.clone())).unwrap();
    let replacement = match tag.as_str() {
        "Located" => inner.get("stmt").cloned(),
        "FieldAccess" => {
            let object = inner.get("object").cloned().unwrap_or(J::Null);
            let field = inner.get("field").cloned().unwrap_or(J::Null);
            Some(json!({ "Call": {
                "callee": { "Identifier": "map_get" },
                "arguments": [ { "Positional": object }, { "Positional": { "String": field } } ],
            }}))
        }
        "BinaryOp" if inner.get("op") == Some(&json!("Coalesce")) => {
            let left = inner.get("left").cloned().unwrap_or(J::Null);
            let right = inner.get("right").cloned().unwrap_or(J::Null);
            // a normalized `x.k` on the left: map_get_or(x, "k", d)
            match left.get("Call") {
                Some(call)
                    if call.get("callee") == Some(&json!({ "Identifier": "map_get" })) =>
                {
                    let mut args = call
                        .get("arguments")
                        .and_then(|a| a.as_array())
                        .cloned()
                        .unwrap_or_default();
                    args.push(json!({ "Positional": right }));
                    Some(json!({ "Call": {
                        "callee": { "Identifier": "map_get_or" },
                        "arguments": args,
                    }}))
                }
                _ => None,
            }
        }
        _ => None,
    };
    if let Some(r) = replacement {
        *v = r;
    }
}

/// Where a rendered expression sits, as far as parenthesizing goes.
#[derive(Clone, Copy, Default)]
struct Ctx {
    /// Something binds tighter than `??` next to it (an operator above
    /// `??` on the ladder, a unary operator, a postfix): a `??` written
    /// here needs parentheses.
    tight: bool,
    /// The right operand of `|>`: the pipeline passes its left as the
    /// first argument, so the call is not the call written.
    piped: bool,
}

struct Rendered {
    text: String,
    /// The text is an `a ?? b` at its top level.
    coalesce: bool,
}

struct Fixer<'a> {
    /// The text the pairs index into (the file, or one interpolation).
    src: &'a str,
    /// Where `src` starts in the file, for line numbers.
    base: usize,
    file: &'a str,
    report: Report,
    /// The file has its own `map_get`, its own `map_get_or`.
    shadowed: [bool; 2],
}

impl<'a> Fixer<'a> {
    fn line_of(&self, offset: usize) -> usize {
        let at = (self.base + offset).min(self.file.len());
        self.file[..at].bytes().filter(|b| *b == b'\n').count() + 1
    }

    /// The pair's own text with every rewrite inside it made; the text
    /// between children (whitespace, comments) is kept as it is.
    fn copy(&mut self, pair: Pair<'a, Rule>, child_ctx: impl Fn(&Pair<'a, Rule>) -> Ctx) -> String {
        let span = pair.as_span();
        let mut out = String::new();
        let mut pos = span.start();
        for child in pair.into_inner() {
            let cs = child.as_span();
            out.push_str(&self.src[pos..cs.start()]);
            let ctx = child_ctx(&child);
            out.push_str(&self.render(child, ctx).text);
            pos = cs.end();
        }
        out.push_str(&self.src[pos..span.end()]);
        out
    }

    fn render(&mut self, pair: Pair<'a, Rule>, ctx: Ctx) -> Rendered {
        match pair.as_rule() {
            Rule::binary_expr => self.render_binary(pair, ctx),
            Rule::unary_expr => {
                let has_op = pair.clone().into_inner().any(|p| p.as_rule() == Rule::unary_op);
                let inner_ctx = Ctx {
                    tight: ctx.tight || has_op,
                    piped: ctx.piped && !has_op,
                };
                let mut coalesce = false;
                let span = pair.as_span();
                let mut out = String::new();
                let mut pos = span.start();
                for child in pair.into_inner() {
                    let cs = child.as_span();
                    out.push_str(&self.src[pos..cs.start()]);
                    let r = self.render(child, inner_ctx);
                    coalesce = r.coalesce && !has_op;
                    out.push_str(&r.text);
                    pos = cs.end();
                }
                out.push_str(&self.src[pos..span.end()]);
                Rendered { text: out, coalesce }
            }
            Rule::call_expr => self.render_call(pair, ctx),
            Rule::template_string => Rendered {
                text: self.render_template(pair),
                coalesce: false,
            },
            // A macro is handed its arguments' source: left as written.
            Rule::macro_call => Rendered {
                text: pair.as_str().to_string(),
                coalesce: false,
            },
            _ => {
                // A rule that is just its one child (an `expr` around a
                // `binary_expr`, a `positional_arg` around an `expr`) is
                // that child, context and all.
                let span = pair.as_span();
                let mut inner = pair.clone().into_inner();
                if let (Some(only), None) = (inner.next(), inner.next())
                    && only.as_span().start() == span.start()
                    && only.as_span().end() == span.end()
                {
                    return self.render(only, ctx);
                }
                Rendered {
                    text: self.copy(pair, |_| Ctx::default()),
                    coalesce: false,
                }
            }
        }
    }

    fn render_binary(&mut self, pair: Pair<'a, Rule>, ctx: Ctx) -> Rendered {
        let children: Vec<Pair<'a, Rule>> = pair.clone().into_inner().collect();
        let level = |r: Rule| -> Option<u8> {
            Some(match r {
                Rule::or_op => 1,
                Rule::and_op => 2,
                Rule::comp_op => 3,
                Rule::coalesce_op => 4,
                Rule::pipe_op => 5,
                Rule::range_op => 6,
                Rule::add_op => 7,
                Rule::mul_op => 8,
                Rule::bitwise_op => 9,
                _ => return None,
            })
        };
        let single = children.len() == 1;
        let span = pair.as_span();
        let mut out = String::new();
        let mut pos = span.start();
        let mut coalesce = false;
        for (i, child) in children.iter().enumerate() {
            let cs = child.as_span();
            out.push_str(&self.src[pos..cs.start()]);
            let left_op = if i > 0 { level(children[i - 1].as_rule()) } else { None };
            let right_op = children.get(i + 1).and_then(|c| level(c.as_rule()));
            let child_ctx = if single {
                ctx
            } else {
                Ctx {
                    tight: left_op.is_some_and(|l| l >= 4) || right_op.is_some_and(|l| l >= 4),
                    piped: i > 0 && children[i - 1].as_rule() == Rule::pipe_op,
                }
            };
            let r = self.render(child.clone(), child_ctx);
            if single {
                coalesce = r.coalesce;
            } else if level(child.as_rule()) == Some(4) {
                coalesce = true;
            }
            out.push_str(&r.text);
            pos = cs.end();
        }
        out.push_str(&self.src[pos..span.end()]);
        // Several operands: the top level is the loosest operator's.
        if !single {
            let loosest = children.iter().filter_map(|c| level(c.as_rule())).min();
            coalesce = loosest == Some(4);
        }
        Rendered { text: out, coalesce }
    }

    /// A template's interpolations, each fixed as the expression the
    /// parser reads from it; the literal text is kept verbatim.
    fn render_template(&mut self, pair: Pair<'a, Rule>) -> String {
        let span = pair.as_span();
        let text = &self.src[span.start()..span.end()];
        let mut out = String::with_capacity(text.len());
        let bytes: Vec<char> = text.chars().collect();
        let mut i = 0;
        let mut byte_at = 0usize; // byte offset of bytes[i] within text
        while i < bytes.len() {
            let c = bytes[i];
            if c == '\\' && i + 1 < bytes.len() {
                out.push(c);
                out.push(bytes[i + 1]);
                byte_at += c.len_utf8() + bytes[i + 1].len_utf8();
                i += 2;
                continue;
            }
            if c == '$' && bytes.get(i + 1) == Some(&'{') {
                out.push_str("${");
                byte_at += 2;
                i += 2;
                // the expression's extent, as the parser finds it
                let start_byte = byte_at;
                let mut depth = 1;
                let mut in_string = false;
                let mut escape = false;
                let mut j = i;
                let mut end_byte = byte_at;
                while j < bytes.len() {
                    let d = bytes[j];
                    if in_string {
                        if escape {
                            escape = false;
                        } else if d == '\\' {
                            escape = true;
                        } else if d == '"' {
                            in_string = false;
                        }
                    } else if d == '"' {
                        in_string = true;
                    } else if d == '{' {
                        depth += 1;
                    } else if d == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    end_byte += d.len_utf8();
                    j += 1;
                }
                let inner = &text[start_byte..end_byte];
                out.push_str(&self.fix_interpolation(inner, span.start() + start_byte));
                byte_at = end_byte;
                i = j;
                continue;
            }
            out.push(c);
            byte_at += c.len_utf8();
            i += 1;
        }
        out
    }

    fn fix_interpolation(&mut self, inner: &str, at: usize) -> String {
        let trimmed = inner.trim().to_string();
        let lead = &inner[..inner.len() - inner.trim_start().len()];
        let trail = &inner[inner.trim_end().len()..];
        let Ok(mut pairs) = OlangParser::parse(Rule::expr, &trimmed) else {
            return inner.to_string();
        };
        let Some(expr) = pairs.next() else {
            return inner.to_string();
        };
        if expr.as_span().end() != trimmed.len() {
            return inner.to_string();
        }
        // A fixer of its own over the interpolation's text.
        let mut sub = Fixer {
            src: &trimmed,
            base: self.base + at + lead.len(),
            file: self.file,
            report: Report::default(),
            shadowed: self.shadowed,
        };
        let text = sub.render(expr, Ctx::default()).text;
        self.report.gets += sub.report.gets;
        self.report.get_ors += sub.report.get_ors;
        self.report.flagged.extend(sub.report.flagged);
        self.report.left.extend(sub.report.left);
        format!("{lead}{text}{trail}")
    }

    fn render_call(&mut self, pair: Pair<'a, Rule>, ctx: Ctx) -> Rendered {
        let plain = |this: &mut Self, pair: Pair<'a, Rule>| Rendered {
            text: this.copy(pair, |_| Ctx::default()),
            coalesce: false,
        };
        let children: Vec<Pair<'a, Rule>> = pair.clone().into_inner().collect();
        let Some(name) = children
            .first()
            .and_then(|p| p.clone().into_inner().next())
            .filter(|p| p.as_rule() == Rule::identifier)
            .map(|p| p.as_str())
        else {
            return plain(self, pair);
        };
        let is_get = name == "map_get";
        let is_get_or = name == "map_get_or";
        if !(is_get || is_get_or) || children.len() < 2 || children[1].as_rule() != Rule::function_call {
            return plain(self, pair);
        }
        if (is_get && self.shadowed[0]) || (is_get_or && self.shadowed[1]) {
            return plain(self, pair);
        }
        let line = self.line_of(pair.as_span().start());
        let args: Vec<Pair<'a, Rule>> = children[1]
            .clone()
            .into_inner()
            .filter(|p| p.as_rule() == Rule::arg_list)
            .flat_map(|l| l.into_inner())
            .filter(|p| p.as_rule() == Rule::argument)
            .collect();
        let want = if is_get { 2 } else { 3 };
        let positional: Vec<Pair<'a, Rule>> = args
            .iter()
            .filter_map(|a| a.clone().into_inner().next())
            .filter(|a| a.as_rule() == Rule::positional_arg)
            .collect();
        if args.len() != want || positional.len() != want {
            return plain(self, pair);
        }
        if ctx.piped {
            self.report.left.push((line, "a pipeline passes the first argument"));
            return plain(self, pair);
        }
        // `map_get(m, "f")(x)` would read as the method call `m.f(x)`.
        if children.get(2).is_some_and(|p| p.as_rule() == Rule::function_call) {
            self.report.left.push((line, "its value is called"));
            return plain(self, pair);
        }
        let Some(key) = identifier_key(&positional[1]) else {
            self.report.left.push((line, "the key is not an identifier literal"));
            return plain(self, pair);
        };
        // Everything dropped must be code: a comment there would be lost.
        let call_span = children[1].as_span();
        let recv_span = positional[0].as_span();
        let dropped = [
            &self.src[call_span.start()..recv_span.start()],
            &self.src[recv_span.end()..positional[1].as_span().start()],
            &self.src[positional[1].as_span().end()
                ..if is_get_or { positional[2].as_span().start() } else { call_span.end() }],
            if is_get_or {
                &self.src[positional[2].as_span().end()..call_span.end()]
            } else {
                ""
            },
        ];
        if dropped.iter().any(|d| d.contains("//")) {
            self.report.left.push((line, "a comment inside the call"));
            return plain(self, pair);
        }
        let Some(receiver_expr) = positional[0].clone().into_inner().next() else {
            return plain(self, pair);
        };
        let Some(atomic) = receiver_is_atomic(&receiver_expr) else {
            self.report.left.push((line, "the receiver is not an expression `.` can follow"));
            return plain(self, pair);
        };
        if is_get_or {
            let default_text = positional[2].as_str();
            if !default_is_pure(default_text) {
                let call = pair.as_str().lines().next().unwrap_or("").trim().to_string();
                self.report.flagged.push((line, call));
                return plain(self, pair);
            }
        }
        let recv = self.render(receiver_expr, Ctx::default());
        let recv_text = if atomic && !recv.coalesce {
            recv.text
        } else {
            format!("({})", recv.text)
        };
        let mut text = format!("{recv_text}.{key}");
        let mut coalesce = false;
        if is_get_or {
            let Some(default_expr) = positional[2].clone().into_inner().next() else {
                return plain(self, pair);
            };
            let simple = !default_needs_parens(&default_expr);
            let d = self.render(default_expr, Ctx::default());
            let d_text = if simple && !d.coalesce {
                d.text
            } else {
                format!("({})", d.text)
            };
            text = format!("{text} ?? {d_text}");
            coalesce = true;
            self.report.get_ors += 1;
        } else {
            self.report.gets += 1;
        }
        // The postfixes after the call (`.x`, `[i]`, `?`), as written.
        let has_postfix = children.len() > 2;
        if coalesce && (ctx.tight || has_postfix) {
            text = format!("({text})");
            coalesce = false;
        }
        let mut pos = call_span.end();
        for child in children.into_iter().skip(2) {
            let cs = child.as_span();
            text.push_str(&self.src[pos..cs.start()]);
            text.push_str(&self.render(child, Ctx::default()).text);
            pos = cs.end();
        }
        text.push_str(&self.src[pos..pair.as_span().end()]);
        Rendered { text, coalesce }
    }
}

/// The key of `map_get(m, "key")` when it is a string literal naming an
/// identifier `.` can take (not a reserved word, no escapes).
fn identifier_key(arg: &Pair<'_, Rule>) -> Option<String> {
    // positional_arg > expr > binary_expr > unary_expr > call_expr >
    // primary > literal > string
    let mut p = arg.clone();
    for rule in [
        Rule::expr,
        Rule::binary_expr,
        Rule::unary_expr,
        Rule::call_expr,
        Rule::primary,
        Rule::literal,
        Rule::string,
    ] {
        let mut inner = p.into_inner();
        let next = inner.next()?;
        if next.as_rule() != rule || inner.next().is_some() {
            return None;
        }
        p = next;
    }
    let text = p.as_str();
    let key = text.strip_prefix('"')?.strip_suffix('"')?;
    if key.contains('\\') {
        return None;
    }
    let mut chars = key.chars();
    let first = chars.next()?;
    let valid = (first.is_ascii_alphabetic()
        || (first == '_' && key.len() > 1))
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    const RESERVED: [&str; 15] = [
        "fn", "let", "if", "else", "match", "for", "while", "loop", "break", "continue",
        "return", "true", "false", "struct", "enum",
    ];
    if !valid || RESERVED.contains(&key) {
        return None;
    }
    Some(key.to_string())
}

/// Whether `.k` may follow the expression as written (`Some(true)`),
/// only in parentheses (`Some(false)`), or not at all (`None`: an
/// assignment or a loop in argument position).
fn receiver_is_atomic(expr: &Pair<'_, Rule>) -> Option<bool> {
    let binary = expr.clone().into_inner().next()?;
    if binary.as_rule() != Rule::binary_expr {
        return None;
    }
    let mut operands = binary.into_inner();
    let unary = operands.next()?;
    if operands.next().is_some() || unary.as_rule() != Rule::unary_expr {
        return Some(false);
    }
    let mut parts = unary.into_inner();
    let call = parts.next()?;
    if call.as_rule() != Rule::call_expr {
        return Some(false); // a unary operator
    }
    let primary = call.into_inner().next()?;
    let what = primary.into_inner().next()?;
    Some(match what.as_rule() {
        Rule::identifier
        | Rule::paren_expr
        | Rule::map_literal
        | Rule::macro_call
        | Rule::struct_literal
        | Rule::anonymous_object => true,
        Rule::literal => matches!(
            what.into_inner().next().map(|p| p.as_rule()),
            Some(
                Rule::string
                    | Rule::raw_string
                    | Rule::template_string
                    | Rule::list
                    | Rule::unit
                    | Rule::boolean
            )
        ),
        _ => false,
    })
}

/// Whether `x.k ?? d` needs `d` in parentheses: an operator at or below
/// `??`'s level would take `x.k ?? d`'s left as its own (`a == b`), and
/// an `if`, `match`, lambda or block runs on to the end of the line.
fn default_needs_parens(expr: &Pair<'_, Rule>) -> bool {
    let Some(binary) = expr.clone().into_inner().next() else {
        return true;
    };
    if binary.as_rule() != Rule::binary_expr {
        return true;
    }
    let children: Vec<_> = binary.into_inner().collect();
    if children.iter().any(|c| {
        matches!(
            c.as_rule(),
            Rule::or_op | Rule::and_op | Rule::comp_op | Rule::coalesce_op | Rule::cmp_rhs
        )
    }) {
        return true;
    }
    // each operand: an open-ended primary needs closing
    children
        .iter()
        .filter(|c| c.as_rule() == Rule::unary_expr)
        .any(|u| {
            u.clone()
                .into_inner()
                .find(|p| p.as_rule() == Rule::call_expr)
                .and_then(|call| call.into_inner().next())
                .and_then(|primary| primary.into_inner().next())
                .is_some_and(|what| {
                    matches!(
                        what.as_rule(),
                        Rule::if_expr | Rule::match_expr | Rule::lambda | Rule::spawn_expr | Rule::block
                    )
                })
        })
}

/// A default whose evaluation can have no effect: the builtins'
/// purity proof (`expr_is_pure`, a whitelist) over its tree.
fn default_is_pure(text: &str) -> bool {
    let Ok(program) = crate::Parser::new().parse_raw(text.trim()) else {
        return false;
    };
    let [statement] = program.statements.as_slice() else {
        return false;
    };
    match statement.unwrapped() {
        crate::ast::Statement::Expression(e) => crate::builtin::BuiltinFunctions::expr_is_pure(e),
        _ => false,
    }
}

/// `olang fix maps`: every `.ol` file under `paths`, fixed in place (or,
/// with `check`, reported). `skip` names files left untouched.
pub fn run_maps(paths: &[PathBuf], check: bool, skip: &[PathBuf]) -> i32 {
    let mut total = Report::default();
    let mut files_changed = 0;
    let mut failed = 0;
    let skip: Vec<PathBuf> = skip.iter().filter_map(|p| p.canonicalize().ok()).collect();
    for path in paths {
        for file in super::discover_ol_files(path) {
            if file
                .canonicalize()
                .is_ok_and(|c| skip.iter().any(|s| c.starts_with(s)))
            {
                println!("  skipped {}", file.display());
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&file) else {
                continue;
            };
            match fix_maps(&src) {
                Ok((fixed, report)) => {
                    for (line, call) in &report.flagged {
                        println!(
                            "  flagged {}:{}: the default may have an effect, and `??` evaluates it only when used — {}",
                            file.display(),
                            line,
                            call
                        );
                    }
                    if fixed != src {
                        files_changed += 1;
                        println!(
                            "  {} {} ({} map_get, {} map_get_or)",
                            if check { "would fix" } else { "fixed" },
                            display(&file),
                            report.gets,
                            report.get_ors
                        );
                        if !check && std::fs::write(&file, &fixed).is_err() {
                            eprintln!("  could not write {}", file.display());
                            failed += 1;
                        }
                    }
                    total.gets += report.gets;
                    total.get_ors += report.get_ors;
                    total.flagged.extend(report.flagged);
                    total.left.extend(report.left);
                }
                Err(why) => {
                    failed += 1;
                    println!("  left {} — {}", file.display(), why);
                }
            }
        }
    }
    let mut reasons: Vec<(&str, usize)> = Vec::new();
    for (_, why) in &total.left {
        match reasons.iter_mut().find(|(w, _)| w == why) {
            Some((_, n)) => *n += 1,
            None => reasons.push((why, 1)),
        }
    }
    println!(
        "olang fix maps: {} map_get → `.`, {} map_get_or → `??` in {} file{}; {} flagged{}",
        total.gets,
        total.get_ors,
        files_changed,
        if files_changed == 1 { "" } else { "s" },
        total.flagged.len(),
        if reasons.is_empty() {
            String::new()
        } else {
            format!(
                "; left: {}",
                reasons
                    .iter()
                    .map(|(w, n)| format!("{n} ({w})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    if failed > 0 || (check && files_changed > 0) { 1 } else { 0 }
}

fn display(p: &Path) -> String {
    p.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::fix_maps;

    fn fixed(src: &str) -> String {
        match fix_maps(src) {
            Ok((t, _)) => t,
            Err(e) => panic!("{src}: {e}"),
        }
    }

    #[test]
    fn rewrites_reads_and_defaults_keeping_layout_and_comments() {
        let src = "// head\nlet a = map_get(p, \"profiles\")   // trailing\nlet b = map_get(map_get(m, \"pp\"), \"cur\")\nlet c = map_get_or(opts, \"pad\", 0) + 1\nlet d = len(filter(map_get(p, \"xs\"), (e) => map_get(e, \"id\") != live))\n";
        assert_eq!(
            fixed(src),
            "// head\nlet a = p.profiles   // trailing\nlet b = m.pp.cur\nlet c = (opts.pad ?? 0) + 1\nlet d = len(filter(p.xs, (e) => e.id != live))\n"
        );
    }

    #[test]
    fn parenthesizes_where_precedence_needs_it() {
        assert_eq!(fixed("let x = map_get(a + b, \"k\")\n"), "let x = (a + b).k\n");
        assert_eq!(fixed("let x = map_get_or(m, \"k\", a == b)\n"), "let x = m.k ?? (a == b)\n");
        assert_eq!(fixed("let x = map_get_or(m, \"k\", 1) > 3\n"), "let x = m.k ?? 1 > 3\n");
        assert_eq!(fixed("let x = !map_get_or(m, \"on\", false)\n"), "let x = !(m.on ?? false)\n");
        assert_eq!(fixed("let x = map_get_or(m, \"k\", #{}).j\n"), "let x = (m.k ?? #{}).j\n");
        assert_eq!(
            fixed("let x = map_get(map_get_or(m, \"k\", #{}), \"j\")\n"),
            "let x = (m.k ?? #{}).j\n"
        );
        assert_eq!(
            fixed("let x = map_get_or(m, \"a\", map_get_or(m, \"b\", 0))\n"),
            "let x = m.a ?? (m.b ?? 0)\n"
        );
    }

    #[test]
    fn leaves_what_is_not_exact_or_not_an_identifier() {
        for src in [
            "let x = map_get(m, \"content-type\")\n",
            "let x = map_get(m, k)\n",
            "let x = map_get(m, \"if\")\n",
            "let x = map_get(m, \"f\")(1)\n",
            "let x = y |> map_get(m, \"k\")\n",
            "let x = map_get(m, // why\n \"k\")\n",
            "let x = map_get_or(m, \"k\", fresh())\n",
            "fn map_get(a, b) = 1\nlet x = map_get(m, \"k\")\n",
            "use lib.w { map_get_or }\nlet x = map_get_or(m, \"k\", 0)\n",
        ] {
            assert_eq!(fixed(src), src, "{src}");
        }
        let (_, report) = fix_maps("let x = map_get_or(m, \"k\", fresh())\n").unwrap();
        assert_eq!(report.flagged.len(), 1);
        // its own map_get_or leaves the builtin map_get's calls rewritable
        assert_eq!(
            fixed("fn map_get_or(m, k, d) = d\nlet x = map_get_or(m, \"k\", map_get(m, \"j\"))\n"),
            "fn map_get_or(m, k, d) = d\nlet x = map_get_or(m, \"k\", m.j)\n"
        );
    }

    #[test]
    fn templates_and_idempotence() {
        let src = "let s = `${map_get(p, \"name\")} is ${map_get_or(p, \"age\", \"?\")}`\n";
        let once = fixed(src);
        assert_eq!(once, "let s = `${p.name} is ${p.age ?? \"?\"}`\n");
        assert_eq!(fixed(&once), once);
    }
}

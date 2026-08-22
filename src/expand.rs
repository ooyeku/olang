//! Macro expansion — the `meta fn` / `@` stage (docs/macros.md).
//!
//! Expansion is a text→text transform that runs inside `Parser::parse`,
//! between the raw parse and the program the interpreter sees. Each round
//! parses the current text with the real grammar (the total-parse law: a
//! file parses completely before any macro runs), finds the `@` sites the
//! *parser* reported — never a textual scan, so `@` inside a string
//! literal is data — and replaces each site with what its `meta fn`
//! returns. Rounds repeat until no sites remain, bounded by fuel.
//!
//! The exchange format is source text. A meta fn receives each argument
//! as the source of that expression and returns source, which is
//! validated by the same parser before it is spliced. The frozen grammar
//! is the contract; `meta.parse` (the Open AST) is the inspection tool.
//!
//! Purity is enforced, not advised: meta fns run on a dedicated
//! interpreter in meta mode, where the effectful and nondeterministic
//! modules (`fs`, `http`, `db`, `proc`, `os`, `time`, `random`, `task`,
//! `chan`) refuse, as do `spawn` and the parallel builtins. Expansion is
//! therefore a pure function of the source, which is what keeps
//! record/replay exact and expansion results cacheable.
//!
//! Meta fns exist only at expansion time. The final text has every
//! `meta fn` declaration blanked out — with newlines preserved, so line
//! numbers in the rest of the file do not shift.

use crate::ast::{Program, Value};
use crate::parser::Parser;

/// Expansion rounds before giving up. Sixteen nested macro-producing
/// macros is far beyond any sane program; an expansion that has not
/// converged by then is a loop, and the error says which sites remain.
const FUEL: usize = 16;

/// One `@name(args)` expression site, as byte offsets into the text.
struct CallSite {
    name: String,
    args_src: Vec<String>,
    span: (usize, usize),
    line: u32,
}

/// One decorated declaration: stacked `@name` above a `type`.
struct DecoratedSite {
    decorators: Vec<(String, Vec<String>, u32)>,
    decl_src: String,
    span: (usize, usize),
}

/// A `meta fn` declaration: where it sits (for stripping) and its source.
struct MetaFn {
    span: (usize, usize),
    src: String,
}

/// Does this program use macros at all? Callers pre-filter with a cheap
/// text scan; this is the authoritative answer, from the AST.
pub fn program_uses_macros(program: &Program) -> bool {
    let json = match serde_json::to_value(program) {
        Ok(v) => v,
        Err(_) => return false,
    };
    json_has_key(&json, &["MacroCall", "MetaFnDecl", "DecoratedDecl"])
}

fn json_has_key(v: &serde_json::Value, keys: &[&str]) -> bool {
    match v {
        serde_json::Value::Object(map) => {
            map.keys().any(|k| keys.contains(&k.as_str()))
                || map.values().any(|v| json_has_key(v, keys))
        }
        serde_json::Value::Array(items) => items.iter().any(|v| json_has_key(v, keys)),
        _ => false,
    }
}

/// Where a line of the expanded program came from: untouched source (its
/// original 1-based line), or a macro's output (attributed to the `@`
/// site's original line). This is what lets a runtime error inside an
/// expanded program point back into the file the author is looking at.
#[derive(Debug, Clone, PartialEq)]
pub enum LineOrigin {
    Original(usize),
    Generated {
        macro_name: String,
        site_line: usize,
    },
}

/// The expanded program plus its line map: `line_origins[i]` is the
/// origin of expanded line `i + 1`.
pub struct Expansion {
    pub text: String,
    pub line_origins: Vec<LineOrigin>,
}

/// Expand `source` to a macro-free program text. Errors are strings that
/// name the macro and the line of the site that failed.
pub fn expand_source(source: &str) -> Result<String, String> {
    expand_source_mapped(source).map(|e| e.text)
}

/// Rewrite one splice's worth of the line map, before the text itself is
/// spliced (offsets refer to the current text). The replaced lines map to
/// the macro that produced them, rooted at the site's original line — and
/// when the site itself sits in generated text (a macro whose output
/// called another macro), the root survives, so attribution always lands
/// on a line the author wrote.
fn splice_line_map(
    map: &mut Vec<LineOrigin>,
    text: &str,
    span: (usize, usize),
    replacement: &str,
    macro_name: &str,
) {
    let start_line = line_of(text, span.0);
    let old_lines = text[span.0..span.1].matches('\n').count() + 1;
    let new_lines = replacement.matches('\n').count() + 1;
    let site_line = match map.get(start_line - 1) {
        Some(LineOrigin::Original(m)) => *m,
        Some(LineOrigin::Generated { site_line, .. }) => *site_line,
        None => start_line,
    };
    let generated = LineOrigin::Generated {
        macro_name: macro_name.to_string(),
        site_line,
    };
    let end = (start_line - 1 + old_lines).min(map.len());
    map.splice(
        start_line - 1..end,
        std::iter::repeat_n(generated, new_lines),
    );
}

/// `expand_source`, keeping the line map. The map is exact for
/// declaration-level output and meta fn stripping (both line-preserving
/// or tracked), and attributes every generated line to the `@` site that
/// produced it.
pub fn expand_source_mapped(source: &str) -> Result<Expansion, String> {
    // Same source, same program: gensym names restart at zero for every
    // expansion, so re-parsing a file yields byte-identical output.
    crate::stdlib::meta::reset_fresh_counter();
    let parser = Parser::new();
    let mut interp = expansion_interpreter();
    let mut text = source.to_string();
    let mut line_map: Vec<LineOrigin> = (1..=source.matches('\n').count() + 1)
        .map(LineOrigin::Original)
        .collect();

    for _round in 0..FUEL {
        let program = parser
            .parse_raw(&text)
            .map_err(|e| format!("macro expansion produced text that does not parse:\n{e}"))?;
        let (meta_fns, calls, decorated) = collect_sites(&program, &text);

        // Placement rules, enforced before anything runs. A meta fn is a
        // top-level declaration: nested in a block it would be stripped
        // out of a function's body, which cannot mean anything coherent.
        // And an @ site inside a meta fn's own body is a phase error —
        // at expansion time a meta fn IS the macro; call it directly.
        let top_level_spans: std::collections::HashSet<(usize, usize)> = program
            .statements
            .iter()
            .filter_map(|st| match st.unwrapped() {
                crate::ast::Statement::MetaFnDecl { span, .. } => Some(*span),
                _ => None,
            })
            .collect();
        for f in &meta_fns {
            if !top_level_spans.contains(&f.span) {
                return Err(format!(
                    "meta fn (line {}) must be a top-level declaration — it exists \
                     only at expansion time and cannot live inside a function or block",
                    line_of(&text, f.span.0)
                ));
            }
        }
        for c in &calls {
            if meta_fns
                .iter()
                .any(|f| c.span.0 >= f.span.0 && c.span.1 <= f.span.1)
            {
                return Err(format!(
                    "@{} (line {}): a macro call inside a meta fn body — at expansion \
                     time a meta fn is ordinary code, so call {}(...) directly instead",
                    c.name, c.line, c.name
                ));
            }
        }

        // Imported macros: a top-level `use m` also brings m's top-level
        // meta fns into the expansion environment, which is what makes a
        // macro *library* possible. Resolution mirrors the runtime's
        // common relative forms — m.ol, then m/index.ol, with dots as
        // directories — against the working directory (the same base a
        // bare `olang run` resolves from). One level: an imported
        // module's own imports are its business, not re-walked here.
        // The module file's content is an expansion input exactly like
        // the source itself; a missing file is not an error (the module
        // may exist only for runtime), but an unparseable one is.
        for u in collect_imports(&program) {
            let rel = u.join("/");
            let candidates = [format!("{rel}.ol"), format!("{rel}/index.ol")];
            let Some(module_src) = candidates
                .iter()
                .find_map(|c| std::fs::read_to_string(c).ok())
            else {
                continue;
            };
            let module_prog = parser.parse_raw(&module_src).map_err(|e| {
                format!(
                    "use {}: the imported module does not parse: {e}",
                    u.join(".")
                )
            })?;
            for st in &module_prog.statements {
                if let crate::ast::Statement::MetaFnDecl { span, .. } = st.unwrapped() {
                    let fn_src = module_src[span.0..span.1]
                        .trim_start()
                        .strip_prefix("meta")
                        .unwrap_or(&module_src[span.0..span.1])
                        .trim_start()
                        .to_string();
                    let prog = parser.parse_raw(&fn_src).map_err(|e| {
                        format!("use {}: imported meta fn does not parse: {e}", u.join("."))
                    })?;
                    interp.eval_program(prog).map_err(|e| {
                        format!("use {}: imported meta fn failed to load: {e}", u.join("."))
                    })?;
                }
            }
        }

        // Define (or redefine) every meta fn for this round. The source
        // still contains them across rounds, so composition works: a
        // macro's output may call other macros. Locals load after imports,
        // so a local meta fn shadows an imported one of the same name.
        for f in &meta_fns {
            let fn_src = f
                .src
                .trim_start()
                .strip_prefix("meta")
                .unwrap_or(&f.src)
                .trim_start()
                .to_string();
            let prog = parser
                .parse_raw(&fn_src)
                .map_err(|e| format!("meta fn does not parse: {e}"))?;
            interp
                .eval_program(prog)
                .map_err(|e| format!("meta fn failed to load: {e}"))?;
        }

        if calls.is_empty() && decorated.is_empty() {
            // Done: strip the meta fns (newline-preserving, so lines in
            // the rest of the file keep their numbers) and hand back.
            let mut out = text.into_bytes();
            for f in &meta_fns {
                for b in &mut out[f.span.0..f.span.1] {
                    if *b != b'\n' {
                        *b = b' ';
                    }
                }
            }
            let text = String::from_utf8(out).map_err(|e| e.to_string())?;
            return Ok(Expansion {
                text,
                line_origins: line_map,
            });
        }

        // Expand innermost-first: a call site whose span contains another
        // site waits for the inner one. Splice from the end of the text
        // backwards so earlier offsets stay valid.
        let all_spans: Vec<(usize, usize)> = calls.iter().map(|c| c.span).collect();
        let mut splices: Vec<(usize, usize, String, String)> = Vec::new();

        for c in &calls {
            let is_leaf = !all_spans.iter().any(|s| s.0 > c.span.0 && s.1 <= c.span.1);
            if !is_leaf {
                continue;
            }
            let args: Vec<Value> = c
                .args_src
                .iter()
                .map(|a| Value::String(std::sync::Arc::new(a.clone())))
                .collect();
            let out = call_meta_fn(&mut interp, &c.name, args)
                .map_err(|e| format!("@{} (line {}): {}", c.name, c.line, e))?;
            // Validate the fragment as it will actually compose: wrapped
            // in parentheses, the way it sits inside the surrounding
            // expression. This catches what a bare parse cannot — output
            // ending in a `//` comment parses alone but swallows the
            // call site's closing token when spliced inline. Failing
            // here blames the macro at its site instead of surfacing as
            // an unattributed whole-file error next round.
            parser.parse_raw(&format!("({})\n", out)).map_err(|e| {
                format!(
                    "@{} (line {}) generated source that does not splice as an \
                     expression (a trailing // comment in the output is the usual \
                     cause):\n{}\n── generated ──\n{}",
                    c.name, c.line, e, out
                )
            })?;
            splices.push((c.span.0, c.span.1, out, c.name.clone()));
        }

        for d in &decorated {
            let inner = all_spans.iter().any(|s| s.0 > d.span.0 && s.1 <= d.span.1);
            if inner {
                continue;
            }
            // Nearest decorator first: the one directly above the
            // declaration transforms it, the next wraps that result.
            let mut cur = d.decl_src.clone();
            for (name, extra, line) in d.decorators.iter().rev() {
                let mut args: Vec<Value> = vec![Value::String(std::sync::Arc::new(cur.clone()))];
                args.extend(
                    extra
                        .iter()
                        .map(|a| Value::String(std::sync::Arc::new(a.clone()))),
                );
                cur = call_meta_fn(&mut interp, name, args)
                    .map_err(|e| format!("@{} (line {}): {}", name, line, e))?;
                parser.parse_raw(&cur).map_err(|e| {
                    format!(
                        "@{} (line {}) generated source that does not parse:\n{}\n── generated ──\n{}",
                        name, line, e, cur
                    )
                })?;
            }
            // Attribution names the outermost decorator — the one whose
            // output is the final text of this span.
            let outer = d
                .decorators
                .first()
                .map(|(n, _, _)| n.clone())
                .unwrap_or_default();
            splices.push((d.span.0, d.span.1, cur, outer));
        }

        if splices.is_empty() {
            // Sites exist but none is a leaf: impossible unless the span
            // logic is wrong — refuse rather than loop.
            return Err("macro expansion made no progress (internal error)".to_string());
        }
        splices.sort_by_key(|(start, _, _, _)| std::cmp::Reverse(*start));
        for (start, end, replacement, macro_name) in splices {
            splice_line_map(
                &mut line_map,
                &text,
                (start, end),
                &replacement,
                &macro_name,
            );
            text.replace_range(start..end, &replacement);
        }
    }

    // Fuel exhausted: report what still wants expanding.
    let program = parser
        .parse_raw(&text)
        .map_err(|e| format!("macro expansion produced text that does not parse:\n{e}"))?;
    let (_, calls, decorated) = collect_sites(&program, &text);
    let mut remaining: Vec<String> = calls
        .iter()
        .map(|c| format!("@{} (line {})", c.name, c.line))
        .collect();
    remaining.extend(
        decorated
            .iter()
            .flat_map(|d| d.decorators.iter())
            .map(|(n, _, l)| format!("@{n} (line {l})")),
    );
    Err(format!(
        "macro expansion did not terminate after {FUEL} rounds — a macro is \
         producing macro calls in a loop. Remaining sites: {}",
        remaining.join(", ")
    ))
}

/// The interpreter meta fns run on: meta mode blocks the effectful and
/// nondeterministic modules, so expansion is a pure function of source.
fn expansion_interpreter() -> crate::interpreter::Interpreter {
    let mut interp = crate::interpreter::Interpreter::new();
    interp.set_meta_mode(true);
    interp
}

fn call_meta_fn(
    interp: &mut crate::interpreter::Interpreter,
    name: &str,
    args: Vec<Value>,
) -> Result<String, String> {
    let result = interp
        .call_named_function(name, args)
        .map_err(|e| e.to_string())?;
    match result {
        Value::String(s) => Ok(s.as_ref().clone()),
        other => Err(format!(
            "a meta fn must return source text as a String, got {}",
            other.type_name()
        )),
    }
}

/// Pull every macro construct out of the parsed program, with byte spans.
/// Walks the serialized AST rather than matching every Expr variant, so a
/// site is found wherever it nests — a match arm, a lambda body, a list —
/// without this file having to track the AST's shape.
fn collect_sites(
    program: &Program,
    text: &str,
) -> (Vec<MetaFn>, Vec<CallSite>, Vec<DecoratedSite>) {
    let mut meta_fns = Vec::new();
    let mut calls = Vec::new();
    let mut decorated = Vec::new();
    let json = match serde_json::to_value(program) {
        Ok(v) => v,
        Err(_) => return (meta_fns, calls, decorated),
    };
    walk_json(&json, &mut |key, obj| match key {
        "MetaFnDecl" => {
            if let Some(span) = span_of(obj) {
                meta_fns.push(MetaFn {
                    src: text[span.0..span.1].to_string(),
                    span,
                });
            }
        }
        "MacroCall" => {
            if let (Some(span), Some(name)) = (span_of(obj), str_of(obj, "name")) {
                calls.push(CallSite {
                    name,
                    args_src: str_list_of(obj, "args_src"),
                    span,
                    line: obj.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                });
            }
        }
        "DecoratedDecl" => {
            if let (Some(span), Some(decl_src)) = (span_of(obj), str_of(obj, "decl_src")) {
                let decorators = obj
                    .get("decorators")
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|d| {
                                Some((
                                    d.get("name")?.as_str()?.to_string(),
                                    d.get("args_src")
                                        .and_then(|a| a.as_array())
                                        .map(|a| {
                                            a.iter()
                                                .filter_map(|x| x.as_str())
                                                .map(String::from)
                                                .collect()
                                        })
                                        .unwrap_or_default(),
                                    d.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                decorated.push(DecoratedSite {
                    decorators,
                    decl_src,
                    span,
                });
            }
        }
        _ => {}
    });
    (meta_fns, calls, decorated)
}

/// Top-level `use` paths, for imported-macro resolution.
fn collect_imports(program: &Program) -> Vec<Vec<String>> {
    program
        .statements
        .iter()
        .filter_map(|st| match st.unwrapped() {
            crate::ast::Statement::UseDecl(u) => Some(u.path.clone()),
            _ => None,
        })
        .collect()
}

fn line_of(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

/// Does the text contain a `meta fn` declaration token — `meta`, spaces,
/// `fn` at word boundaries? The cheap pre-filter `Parser::parse` uses so
/// a file that merely *mentions* meta (a comment, `meta.parse`) does not
/// pay for AST serialization.
pub fn has_meta_fn_token(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut i = 0;
    while let Some(pos) = input[i..].find("meta") {
        let at = i + pos;
        let before_ok =
            at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        let mut j = at + 4;
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        let fn_here = j > at + 4
            && input[j..].starts_with("fn")
            && !bytes
                .get(j + 2)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
        if before_ok && fn_here {
            return true;
        }
        i = at + 4;
    }
    false
}

fn walk_json(v: &serde_json::Value, f: &mut impl FnMut(&str, &serde_json::Value)) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, inner) in map {
                f(k, inner);
                walk_json(inner, f);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk_json(item, f);
            }
        }
        _ => {}
    }
}

fn span_of(obj: &serde_json::Value) -> Option<(usize, usize)> {
    let span = obj.get("span")?.as_array()?;
    Some((
        span.first()?.as_u64()? as usize,
        span.get(1)?.as_u64()? as usize,
    ))
}

fn str_of(obj: &serde_json::Value, key: &str) -> Option<String> {
    Some(obj.get(key)?.as_str()?.to_string())
}

fn str_list_of(obj: &serde_json::Value, key: &str) -> Vec<String> {
    obj.get(key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|x| x.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

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

/// `expand_source_mapped`, resolving `use` imports relative to `base_dir`
/// (the directory of the file being expanded) before the working
/// directory. The run path, `olang check`, and the LSP all know the
/// file's directory; plain `Parser::parse` (REPL, doc snippets) does not
/// and uses the working directory alone.
pub fn expand_source_mapped_with_dir(
    source: &str,
    base_dir: Option<&std::path::Path>,
) -> Result<Expansion, String> {
    expand_impl(source, base_dir)
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
    expand_impl(source, None)
}

fn expand_impl(source: &str, base_dir: Option<&std::path::Path>) -> Result<Expansion, String> {
    // Same source, same program: gensym names restart at zero for every
    // expansion, so re-parsing a file yields byte-identical output.
    crate::stdlib::meta::reset_fresh_counter();
    // A name of the form `meta.fresh` produces (`stem__mN`) belongs to
    // the expander alone: a program that spells one is refused here, so
    // a generated temporary can never collide with a hand-written name.
    if let Some((name, line)) = reserved_name_in(source) {
        return Err(format!(
            "line {line}: `{name}` is a name reserved for macro-generated temporaries \
             (the `__m<N>` suffix is what meta.fresh appends); rename it"
        ));
    }
    let parser = Parser::new();
    let mut interp = expansion_interpreter();
    // What `meta.exports(path)` answers this expansion: the literal
    // top-level bindings of each imported module, by import path. The
    // table lives for this expansion and is cleared when it ends.
    let _exports_guard = crate::stdlib::meta::ExportsGuard::new();
    let mut exports: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    // Only names declared `meta fn` — locally or in an imported module —
    // are invocable as macros. Without this registry, an `@` call would
    // fall through to ANY global binding: `@json` found the stdlib json
    // module and tried to call it, and `@map(...)` would have handed the
    // builtin `map` source strings. A macro is a declared thing.
    let mut known_macros: std::collections::HashSet<String> = std::collections::HashSet::new();
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
            let names = [format!("{rel}.ol"), format!("{rel}/index.ol")];
            let mut candidates: Vec<std::path::PathBuf> = Vec::new();
            if let Some(dir) = base_dir {
                candidates.extend(names.iter().map(|n| dir.join(n)));
            }
            candidates.extend(names.iter().map(std::path::PathBuf::from));
            // Package imports resolve as the runtime resolves them —
            // without this, a LIBRARY could never export a macro. The
            // nearest manifest's path and shelf dependencies come first,
            // then the shelf by bare name: `use web` finds the package's
            // index.ol, `use web.lib.store` a module inside it.
            if let Some(dir) = package_dir(&u[0], base_dir) {
                if u.len() == 1 {
                    candidates.push(dir.join("index.ol"));
                } else {
                    let rest = u[1..].join("/");
                    candidates.push(dir.join(format!("{rest}.ol")));
                    candidates.push(dir.join(format!("{rest}/index.ol")));
                }
            }
            let Some((module_path, module_src)) = candidates
                .iter()
                .find_map(|c| std::fs::read_to_string(c).ok().map(|src| (c.clone(), src)))
            else {
                continue;
            };
            let label = u.join(".");
            let module_prog =
                load_module_macros(&parser, &mut interp, &mut known_macros, &module_src, &label)?;
            // The module's shared functions join the expansion scope, so
            // a thin meta fn can delegate to a tested, shared validator.
            // They run under meta mode: one that reaches for an effect
            // fails at the call, exactly like a meta fn body would.
            load_module_functions(&mut interp, &module_prog);
            // Its literal top-level bindings are what `meta.exports`
            // reads: a `let` or `share let` whose value is a literal (a
            // number, string, list, map, tuple of literals) is a
            // declaration a macro in another file may consult.
            let mut literals = collect_literal_bindings(&mut interp, &module_prog);
            // A package's macros travel through its index.ol re-exports:
            // `share use lib.store { ... }` in the index brings lib/store.ol's
            // meta fns along, so `use web` reaches `@store` without the
            // consumer naming the declaring module's path. One level, and
            // resolved against the index's own directory.
            let module_dir = module_path.parent().map(|d| d.to_path_buf());
            for st in &module_prog.statements {
                // `share use` and plain `use` alike: an index that imports
                // `lib/decl.ol { resource }` makes `@resource` a macro of
                // the package's front door, so `use shuttle` reaches it.
                let sub_path = match st.unwrapped() {
                    crate::ast::Statement::ShareDecl(crate::ast::ShareDecl::Use(su)) => {
                        su.path.clone()
                    }
                    crate::ast::Statement::UseDecl(u2) => u2.path.clone(),
                    _ => continue,
                };
                let Some(dir) = &module_dir else { continue };
                let sub = sub_path.join("/");
                let sub_candidates = [
                    dir.join(format!("{sub}.ol")),
                    dir.join(format!("{sub}/index.ol")),
                ];
                let Some(sub_src) = sub_candidates
                    .iter()
                    .find_map(|c| std::fs::read_to_string(c).ok())
                else {
                    continue;
                };
                let sub_label = format!("{label} (re-export of {})", sub_path.join("."));
                let sub_prog = load_module_macros(
                    &parser,
                    &mut interp,
                    &mut known_macros,
                    &sub_src,
                    &sub_label,
                )?;
                load_module_functions(&mut interp, &sub_prog);
                // A re-exported module's literals are the package's too.
                for (k, v) in collect_literal_bindings(&mut interp, &sub_prog) {
                    literals.entry(k).or_insert(v);
                }
            }
            exports.insert(label.clone(), Value::Map(std::sync::Arc::new(literals)));
        }
        crate::stdlib::meta::set_exports(exports.clone());

        // The file's own functions are in a meta fn's scope (the macros
        // chapter's first rule): a macro delegates to a helper declared
        // beside it. Loaded before the meta fns so a meta fn body may call
        // them; effectful bodies fail at the call, as ever.
        load_local_functions(&mut interp, &program);

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
            if let Some(name) = meta_fn_name(&fn_src) {
                known_macros.insert(name);
            }
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

        // Arguments are expanded before the macro runs: a nested call
        // (`@bake(@twice(4))`) is invisible to the site collector — the
        // argument is carried as text — so it is expanded HERE, and every
        // macro receives macro-free source. Applicative order, the same
        // rule function calls follow; without it, a macro that inspects
        // or evaluates its argument would meet raw `@` text, while a
        // template-splicing macro would work only by re-expansion luck.
        let mut splices: Vec<(usize, usize, String, String)> = Vec::new();

        for c in &calls {
            let args: Vec<Value> = {
                let mut expanded_args = Vec::with_capacity(c.args_src.len());
                for a in &c.args_src {
                    let ex = expand_fragment(&parser, &mut interp, &known_macros, a)
                        .map_err(|e| format!("@{} (line {}): {}", c.name, c.line, e))?;
                    expanded_args.push(Value::String(std::sync::Arc::new(ex)));
                }
                expanded_args
            };
            let out = call_meta_fn(&mut interp, &known_macros, &c.name, args)
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
                    "@{} (line {}): generated source does not parse: {}{}\n── generated ──\n{}",
                    c.name,
                    c.line,
                    e,
                    trailing_comment_hint(&out),
                    out
                )
            })?;
            splices.push((c.span.0, c.span.1, out, c.name.clone()));
        }

        for d in &decorated {
            // Nearest decorator first: the one directly above the
            // declaration transforms it, the next wraps that result.
            let mut cur = d.decl_src.clone();
            for (name, extra, line) in d.decorators.iter().rev() {
                let mut args: Vec<Value> = vec![Value::String(std::sync::Arc::new(cur.clone()))];
                for a in extra {
                    let ex = expand_fragment(&parser, &mut interp, &known_macros, a)
                        .map_err(|e| format!("@{} (line {}): {}", name, line, e))?;
                    args.push(Value::String(std::sync::Arc::new(ex)));
                }
                cur = call_meta_fn(&mut interp, &known_macros, name, args)
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

/// Expand the macro calls inside one source fragment (a macro argument),
/// returning macro-free source. Recursion handles nesting of any depth,
/// bounded by the same fuel discipline as whole-file rounds. A fragment
/// that does not parse on its own (a named argument, say) is returned
/// unchanged — the outer splice validation owns real errors.
fn expand_fragment(
    parser: &Parser,
    interp: &mut crate::interpreter::Interpreter,
    known: &std::collections::HashSet<String>,
    fragment: &str,
) -> Result<String, String> {
    let mut text = fragment.to_string();
    for _ in 0..FUEL {
        let Ok(program) = parser.parse_raw(&text) else {
            return Ok(text);
        };
        let (_, calls, _) = collect_sites(&program, &text);
        if calls.is_empty() {
            return Ok(text);
        }
        let mut splices: Vec<(usize, usize, String)> = Vec::new();
        for c in &calls {
            let mut args = Vec::with_capacity(c.args_src.len());
            for a in &c.args_src {
                let ex = expand_fragment(parser, interp, known, a)?;
                args.push(Value::String(std::sync::Arc::new(ex)));
            }
            let out = call_meta_fn(interp, known, &c.name, args)
                .map_err(|e| format!("@{}: {}", c.name, e))?;
            parser.parse_raw(&format!("({})\n", out)).map_err(|e| {
                format!(
                    "@{}: generated source does not parse: {}{}\n── generated ──\n{}",
                    c.name,
                    e,
                    trailing_comment_hint(&out),
                    out
                )
            })?;
            splices.push((c.span.0, c.span.1, out));
        }
        splices.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        for (start, end, replacement) in splices {
            text.replace_range(start..end, &replacement);
        }
    }
    Err("macro expansion inside an argument did not terminate".to_string())
}

fn call_meta_fn(
    interp: &mut crate::interpreter::Interpreter,
    known: &std::collections::HashSet<String>,
    name: &str,
    args: Vec<Value>,
) -> Result<String, String> {
    if !known.contains(name) {
        return Err(format!(
            "no meta fn named '{name}' — declare one with `meta fn {name}(...) = ...` \
             in this file, or import a module that declares it with `use`"
        ));
    }
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
/// The directory a package import resolves to, or None for a plain
/// relative module. The nearest manifest (from `base_dir`, else the
/// working directory) is consulted first: a `{ path = ... }` dependency
/// points at its directory, a `{ shelf = ... }` one at the shelf entry it
/// names. Failing that, the bare name is looked up on the shelf directly.
fn package_dir(name: &str, base_dir: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    use crate::pkg::manifest::{Dependency, Manifest};
    let start = base_dir
        .map(|d| d.to_path_buf())
        .or_else(|| std::env::current_dir().ok());
    if let Some(start) = start
        && let Some(root) = Manifest::find_root(&start)
        && let Ok(manifest) = Manifest::load(&root)
        && let Some(dep) = manifest.dependencies.get(name)
    {
        match dep {
            Dependency::Path { path } => return Some(root.join(path)),
            Dependency::Shelf { shelf } => {
                return crate::pkg::shelf::Shelf::load()
                    .ok()
                    .and_then(|s| s.libraries.get(shelf).cloned());
            }
            _ => {}
        }
    }
    crate::pkg::shelf::Shelf::load()
        .ok()
        .and_then(|s| s.libraries.get(name).cloned())
}

/// Load every top-level meta fn of a module's source into the expansion
/// interpreter and record its name as a known macro. The module file's
/// content is an expansion input exactly like the source itself; an
/// unparseable one is an error. Returns the parsed module for callers
/// that walk its re-exports.
fn load_module_macros(
    parser: &Parser,
    interp: &mut crate::interpreter::Interpreter,
    known_macros: &mut std::collections::HashSet<String>,
    module_src: &str,
    label: &str,
) -> Result<Program, String> {
    let module_prog = parser
        .parse_raw(module_src)
        .map_err(|e| format!("use {label}: the imported module does not parse: {e}"))?;
    for st in &module_prog.statements {
        if let crate::ast::Statement::MetaFnDecl { span, .. } = st.unwrapped() {
            let fn_src = module_src[span.0..span.1]
                .trim_start()
                .strip_prefix("meta")
                .unwrap_or(&module_src[span.0..span.1])
                .trim_start()
                .to_string();
            let prog = parser
                .parse_raw(&fn_src)
                .map_err(|e| format!("use {label}: imported meta fn does not parse: {e}"))?;
            interp
                .eval_program(prog)
                .map_err(|e| format!("use {label}: imported meta fn failed to load: {e}"))?;
            if let Some(name) = meta_fn_name(&fn_src) {
                known_macros.insert(name);
            }
        }
    }
    Ok(module_prog)
}

/// The one splice failure with a non-obvious cause: output whose last
/// line ends in a `//` comment parses alone but swallows the call site's
/// closing token when spliced inline. Name it only when it is present —
/// a guess offered for every failure pointed authors away from the real
/// parse error (a leading-underscore name, in the incident behind this).
/// Define an imported module's `share fn`s in the expansion interpreter.
/// A definition that fails to load (a body the meta-mode interpreter
/// refuses, a duplicate) is skipped: the module still loads at runtime
/// as it always did, and the function is simply not reachable from a
/// meta fn body.
fn load_module_functions(interp: &mut crate::interpreter::Interpreter, module: &Program) {
    for st in &module.statements {
        if let crate::ast::Statement::ShareDecl(crate::ast::ShareDecl::Function(f)) = st.unwrapped()
        {
            let prog = Program {
                statements: vec![crate::ast::Statement::FunctionDecl(f.clone())],
            };
            let _ = interp.eval_program(prog);
        }
    }
}

/// The expanding file's own `fn` declarations (shared or not) join the
/// expansion interpreter, so a meta fn body can call the helpers written
/// beside it. A definition the meta-mode interpreter refuses is skipped —
/// it stays a runtime function only.
fn load_local_functions(interp: &mut crate::interpreter::Interpreter, program: &Program) {
    for st in &program.statements {
        let f = match st.unwrapped() {
            crate::ast::Statement::FunctionDecl(f) => f,
            crate::ast::Statement::ShareDecl(crate::ast::ShareDecl::Function(f)) => f,
            _ => continue,
        };
        let prog = Program {
            statements: vec![crate::ast::Statement::FunctionDecl(f.clone())],
        };
        let _ = interp.eval_program(prog);
    }
}

fn trailing_comment_hint(out: &str) -> &'static str {
    let last = out.trim_end().rsplit('\n').next().unwrap_or("");
    if last.contains("//") {
        " (the output ends in a // comment, which swallows the call site's closing token when spliced inline)"
    } else {
        ""
    }
}

/// The top-level `let`/`share let` bindings of a module whose value is a
/// literal, evaluated in the expansion interpreter (pure by
/// construction: a literal has no effects). Anything computed is not a
/// declaration a macro can read and is left out.
fn collect_literal_bindings(
    interp: &mut crate::interpreter::Interpreter,
    module: &Program,
) -> std::collections::HashMap<String, Value> {
    let mut out = std::collections::HashMap::new();
    for st in &module.statements {
        let decl = match st.unwrapped() {
            crate::ast::Statement::LetDecl(l) => l,
            crate::ast::Statement::ShareDecl(crate::ast::ShareDecl::Let(l)) => l,
            _ => continue,
        };
        let crate::ast::Pattern::Identifier(name) = &decl.pattern else {
            continue;
        };
        let Some(value) = &decl.value else { continue };
        if !is_literal_expr(value) {
            continue;
        }
        if let Ok(v) = interp.eval_expr(value) {
            out.insert(name.clone(), v);
        }
    }
    out
}

fn is_literal_expr(expr: &crate::ast::Expr) -> bool {
    use crate::ast::Expr as E;
    match expr {
        E::Integer(_) | E::Float(_) | E::String(_) | E::Boolean(_) => true,
        E::List(items) => items.iter().all(is_literal_expr),
        E::Tuple(items) => items.iter().all(is_literal_expr),
        E::MapLiteral { entries } => entries
            .iter()
            .all(|e| is_literal_expr(&e.key) && is_literal_expr(&e.value)),
        E::UnaryOp { operand, .. } => is_literal_expr(operand),
        _ => false,
    }
}

/// The first identifier in `source` that spells a macro-generated name
/// (`stem__m<digits>`), with its line — outside strings and comments.
fn reserved_name_in(source: &str) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut line = 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line += 1;
            i += 1;
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if c == b'"' || c == b'`' {
            let quote = c;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    i += 1;
                } else if bytes[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 1;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &source[start..i];
            if is_reserved_name(word) {
                return Some((word.to_string(), line));
            }
        } else {
            i += 1;
        }
    }
    None
}

/// `stem__m<digits>` with a non-empty stem.
fn is_reserved_name(word: &str) -> bool {
    let Some(at) = word.rfind("__m") else {
        return false;
    };
    let digits = &word[at + 3..];
    at > 0 && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

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

/// The declared name of a `fn name(...)` source fragment.
fn meta_fn_name(fn_src: &str) -> Option<String> {
    let rest = fn_src.trim_start().strip_prefix("fn")?.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_string())
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

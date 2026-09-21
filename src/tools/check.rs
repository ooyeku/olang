//! The static side of gradual typing: report annotation violations that
//! are provable before the program runs.
//!
//! The runtime is the authority — every annotation is already enforced at
//! its boundary on every tier, shallowly (`List<Int>` promises "a List" in
//! O(1)). This pass exists to move failures earlier and to go where the
//! runtime deliberately doesn't: **element types**. `[1, "a"]` against
//! `List<Int>` runs fine (the runtime checks the container), but the
//! annotation's promise is provably broken, and the checker says so.
//!
//! Its discipline is **no false positives**: a diagnostic appears only
//! when some annotation in the flagged chain is provably dishonest. Two
//! kinds are reported, distinguished in the output:
//! - violations the runtime would also reject (wrong base type, wrong
//!   arity) — these carry the runtime's exact message text;
//! - element-level breaks the runtime's shallow checks let through —
//!   labeled as broken promises, not runtime failures.
//!
//! Annotations are trusted as premises (`let xs: List<Int>` means the
//! elements are Ints from here on): if a flagged use site is actually
//! fine at runtime, the *source* annotation was dishonest instead — the
//! program still contains a provably false annotation either way.
//! Everything unprovable stays silent; unannotated code is never judged.

use crate::ast::{Argument, Expr, Program, Statement, TypeAnnotation, TypeDefinition, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `olang check [paths] [--rules FILE]` — parse every `.ol` file and report
/// provable annotation violations. With `--rules`, also run project-authored
/// lints written in olang over the meta AST. Exit 0 when everything is clean
/// (or unknowable), 1 when a violation, parse error, or rule finding appears.
pub fn run(paths: &[PathBuf], rules: Option<&Path>) -> i32 {
    // Many files of one project: keep each imported module's parse.
    crate::expand::keep_module_parses(true);
    let mut files = Vec::new();
    for path in paths {
        if !path.exists() {
            eprintln!("olang check: path not found: {}", path.display());
            return 1;
        }
        files.extend(super::discover_ol_files(path));
    }
    if files.is_empty() {
        println!("olang check: no .ol files found");
        return 0;
    }

    let mut problems = 0usize;
    let mut warnings = 0usize;
    let parser = crate::parser::Parser::new();
    for file in &files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("olang check: cannot read {}: {}", file.display(), e);
                problems += 1;
                continue;
            }
        };
        // Expand macros with the FILE's directory as the import base, so
        // `use`-imported macro libraries resolve the same way they do
        // when the program runs from its own directory. (Bare
        // `parser.parse` would expand against the checker's cwd.)
        let expanded;
        let source = if source.contains('@') || crate::expand::has_meta_fn_token(&source) {
            match crate::expand::expand_source_mapped_with_dir(&source, file.parent()) {
                Ok(e) => {
                    expanded = e.text;
                    expanded.clone()
                }
                Err(message) => {
                    eprintln!("{}: {}", file.display(), message);
                    problems += 1;
                    continue;
                }
            }
        } else {
            source
        };
        let program = match parser.parse(&source) {
            Ok(p) => p,
            Err(e) => {
                // A file that doesn't parse can't run either; that counts.
                eprintln!("{}: {}", file.display(), e);
                problems += 1;
                continue;
            }
        };
        let modules = module_programs(&source, file.parent());
        let context: Vec<&Program> = modules.iter().map(|(_, _, p)| p).collect();
        let mut diagnostics = check_program_with_context(&context, &program);
        diagnostics.extend(use_shadow_warnings(&program, file.parent()));
        diagnostics.extend(unimported_names(&program, &source, &modules, file.parent()));
        diagnostics.extend(template_escape_warnings(&source));
        diagnostics.extend(shadow_warnings(&program));
        // The project's `[check] promote`: the advisory classes it names
        // are errors here, so an exhaustiveness or shape finding gates
        // without a rules file.
        let promoted = promotions_for(file.parent());
        for d in diagnostics.iter_mut() {
            promote(d, &promoted);
        }
        for d in diagnostics {
            if d.warning {
                warnings += 1;
            } else {
                problems += 1;
            }
            let offset = byte_offset_of(&source, d.line as usize, d.column as usize);
            let label = if d.warning {
                "advisory — the program still runs"
            } else if d.scope {
                "the program is refused before it runs"
            } else if d.runtime {
                "this would fail at runtime"
            } else {
                "the annotation's promise is broken here"
            };
            let mut diagnostic = miette::MietteDiagnostic::new(d.message.clone())
                .with_label(miette::LabeledSpan::at_offset(offset, label));
            if d.warning {
                diagnostic = diagnostic.with_severity(miette::Severity::Warning);
            }
            let report = miette::Report::new(diagnostic).with_source_code(
                miette::NamedSource::new(file.display().to_string(), source.clone()),
            );
            eprintln!("{:?}", report);
        }
    }

    // Project-authored lints: run the rules file's `rule_*` functions over
    // each target file's meta AST. Findings count as problems (you opted
    // into the rule, so a hit should gate).
    if let Some(rules_path) = rules {
        problems += run_rules(rules_path, &files);
    }

    let warn_note = match warnings {
        0 => String::new(),
        1 => ", 1 warning".to_string(),
        n => format!(", {} warnings", n),
    };
    if problems == 0 {
        println!(
            "olang check: {} file{} clean{}",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            warn_note
        );
        0
    } else {
        eprintln!(
            "olang check: {} problem{} found{}",
            problems,
            if problems == 1 { "" } else { "s" },
            warn_note
        );
        1
    }
}

/// `olang check --rules FILE`: run project-authored lints over every target
/// file's meta AST. A rule is a top-level function named `rule_*` taking one
/// argument — the flat list of every AST node in the file (each a map with
/// at least `kind` and `line`) — and returning a list of findings. A finding
/// is a string (its message) or a map `#{ "message": ..., "line": ... }`.
/// Rules are olang, so "open code" (the meta module) becomes a first-class
/// consumer: an org encodes its invariants in the same language it ships.
/// Returns the number of findings (each counts as a problem).
fn run_rules(rules_path: &Path, files: &[PathBuf]) -> usize {
    // Load and run the rules program once; its `rule_*` functions stay live
    // in the interpreter to be called per target file.
    let source = match std::fs::read_to_string(rules_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "olang check --rules: cannot read {}: {}",
                rules_path.display(),
                e
            );
            return 1;
        }
    };
    let program = match crate::parser::Parser::new().parse(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "olang check --rules: {} does not parse: {}",
                rules_path.display(),
                e
            );
            return 1;
        }
    };
    let mut interp = crate::interpreter::Interpreter::new();
    // A lint is pure analysis over AST data — it needs no effects. Sandbox
    // the rules program (which may be third-party) so it cannot touch the
    // filesystem, network, processes, database, or environment when it is
    // loaded and run. Rules that legitimately need I/O are out of scope by
    // design; the built-in checker runs no target or rule code at all.
    interp.set_capabilities(crate::caps::CapTable {
        app: crate::caps::Caps {
            fs: crate::caps::FsCap::None,
            net: false,
            proc: false,
            db: false,
            env: false,
        },
        deps: Vec::new(),
    });
    let abs = std::fs::canonicalize(rules_path).unwrap_or_else(|_| rules_path.to_path_buf());
    interp.set_current_file(&abs);
    if let Err(e) = interp.eval_program(program) {
        eprintln!(
            "olang check --rules: {} failed to load: {:?}",
            rules_path.display(),
            e
        );
        return 1;
    }

    // Collect the rule functions (name starts with `rule_`), stable order.
    let mut rules: Vec<(String, Value)> = interp
        .get_user_variables()
        .into_iter()
        .filter(|(name, v)| name.starts_with("rule_") && matches!(v, Value::Function(_)))
        .map(|(name, v)| (name, v.clone()))
        .collect();
    rules.sort_by(|a, b| a.0.cmp(&b.0));
    if rules.is_empty() {
        eprintln!(
            "olang check --rules: {} defines no rule_* functions",
            rules_path.display()
        );
        return 1;
    }

    // Don't lint the rules file itself.
    let rules_canon = abs;
    let mut findings = 0usize;
    for file in files {
        let file_canon = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        if file_canon == rules_canon {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(file) else {
            continue;
        };
        // Parse to the meta AST; a file that doesn't parse is already
        // reported by the type-check pass, so skip it silently here.
        let nodes = match crate::stdlib::meta::call_meta_function(
            "parse",
            vec![Value::String(Arc::new(src))],
        ) {
            Ok(Value::Ok(inner)) => match *inner {
                Value::List(items) => (*items).clone(),
                _ => continue,
            },
            _ => continue,
        };
        // Flatten to every node, each stamped with its nearest source line,
        // so a rule is a plain filter with usable positions.
        let flat = Value::List(Arc::new(flatten_ast(&nodes)));

        for (name, func) in &rules {
            match interp.call_function(func.clone(), vec![flat.clone()]) {
                Ok(result) => {
                    for (line, message) in rule_findings(&result) {
                        findings += 1;
                        if line > 0 {
                            eprintln!("{}:{}: [{}] {}", file.display(), line, name, message);
                        } else {
                            eprintln!("{}: [{}] {}", file.display(), name, message);
                        }
                    }
                }
                Err(e) => {
                    findings += 1;
                    eprintln!(
                        "{}: [{}] rule raised an error: {:?}",
                        file.display(),
                        name,
                        e
                    );
                }
            }
        }
    }
    findings
}

/// Flatten the meta AST into a preorder list of every node, stamping each
/// with the nearest enclosing source line so nested expression nodes (which
/// the parser does not position) still report a usable location.
fn flatten_ast(nodes: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for n in nodes {
        flatten_into(n, 0, &mut out);
    }
    out
}

fn flatten_into(v: &Value, inherited_line: i64, out: &mut Vec<Value>) {
    match v {
        Value::Map(m) => {
            let line = match m.get("line") {
                Some(Value::Integer(l)) => *l,
                _ => inherited_line,
            };
            if m.contains_key("kind") {
                let mut node = (**m).clone();
                node.entry("line".to_string())
                    .or_insert(Value::Integer(line));
                out.push(Value::Map(Arc::new(node)));
            }
            for val in m.values() {
                flatten_into(val, line, out);
            }
        }
        Value::List(items) => {
            for it in items.iter() {
                flatten_into(it, inherited_line, out);
            }
        }
        _ => {}
    }
}

/// Read a rule's return value into (line, message) findings. Accepts a list
/// of strings, a list of `#{ message, line }` maps, or a mix; a bare string
/// or map (a single finding) is accepted too. Anything else is no findings.
fn rule_findings(result: &Value) -> Vec<(i64, String)> {
    fn one(v: &Value) -> Option<(i64, String)> {
        match v {
            Value::String(s) => Some((0, s.to_string())),
            Value::Map(m) => {
                let message = match m.get("message").or_else(|| m.get("msg")) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => return None,
                };
                let line = match m.get("line") {
                    Some(Value::Integer(l)) => *l,
                    _ => 0,
                };
                Some((line, message))
            }
            _ => None,
        }
    }
    match result {
        Value::List(items) => items.iter().filter_map(one).collect(),
        other => one(other).into_iter().collect(),
    }
}

/// Every name a top-level declaration of `program` introduces: functions,
/// lets, types, and the variants and constructors of its enums and error
/// types — what a bare `use` of the module may bring into scope, taken
/// generously (private names too), because this list only ever excuses.
fn declared_names(program: &Program, out: &mut std::collections::HashSet<String>) {
    fn of(stmt: &Statement, out: &mut std::collections::HashSet<String>) {
        match stmt {
            Statement::Located { stmt, .. } => of(stmt, out),
            Statement::FunctionDecl(f) => {
                out.insert(f.name.clone());
            }
            Statement::LetDecl(l) => {
                if let crate::ast::Pattern::Identifier(n) = &l.pattern {
                    out.insert(n.clone());
                }
            }
            Statement::TypeDecl(t) => {
                out.insert(t.name.clone());
                if let crate::ast::TypeDefinition::Enum { variants } = &t.definition {
                    out.extend(variants.iter().map(|v| v.name.clone()));
                }
            }
            Statement::ErrorTypeDecl(e) => {
                out.insert(e.name.clone());
                out.extend(e.variants.iter().map(|v| v.name.clone()));
            }
            Statement::ShareDecl(sd) => match sd {
                crate::ast::ShareDecl::Function(f) => {
                    out.insert(f.name.clone());
                }
                crate::ast::ShareDecl::Let(l) => {
                    if let crate::ast::Pattern::Identifier(n) = &l.pattern {
                        out.insert(n.clone());
                    }
                }
                crate::ast::ShareDecl::Type(t) => of(&Statement::TypeDecl(t.clone()), out),
                crate::ast::ShareDecl::Use(u) => {
                    out.extend(
                        u.items
                            .iter()
                            .filter_map(|i| i.bound_name().map(str::to_string)),
                    );
                }
                _ => {}
            },
            _ => {}
        }
    }
    for stmt in &program.statements {
        of(stmt, out);
    }
}

/// A name the file uses and nothing in it defines or imports. Natively
/// that is an undefined variable the moment the line runs; in a browser
/// bundle, which is one namespace, it resolves from whichever module is
/// spliced beside this one — so it works until the day it does not
/// (open-track's `refresh_fx`, Shuttle's `check_record`). An error.
///
/// What a file may name without importing it item by item: everything a
/// bare `use m` brings, and the variants of any enum a context module
/// declares (a variant arrives with its type). When a bare `use` cannot
/// be resolved, what it brings is unknown and nothing is reported.
fn unimported_names(
    program: &Program,
    source: &str,
    modules: &[(PathBuf, String, Program)],
    dir: Option<&Path>,
) -> Vec<CheckDiagnostic> {
    let candidates = crate::analyze::Analyzer::unresolved_names(program);
    if candidates.is_empty() {
        return Vec::new();
    }
    let mut excused = std::collections::HashSet::new();
    declared_names(program, &mut excused);
    // Modules that load on first touch, without a `use`.
    excused.extend(crate::stdlib::get_stdlib().into_keys());
    excused.extend(
        crate::stdlib::embedded::names()
            .into_iter()
            .map(String::from),
    );
    excused.insert("log".to_string());
    for stmt in &program.statements {
        let (Statement::UseDecl(u) | Statement::ShareDecl(crate::ast::ShareDecl::Use(u))) =
            stmt.unwrapped()
        else {
            continue;
        };
        // `use a.b.c` binds `c`, the module itself, whatever else it brings.
        if let Some(last) = u.path.last() {
            excused.insert(last.clone());
        }
        let bare = u.items.is_empty()
            || u.items
                .iter()
                .any(|i| matches!(i, crate::ast::UseItem::Wildcard));
        if !bare || u.path.is_empty() {
            continue;
        }
        // A stdlib module brings its own name and nothing else.
        if u.path.len() == 1 && crate::stdlib::get_stdlib().contains_key(&u.path[0]) {
            continue;
        }
        if u.path.len() == 1 && crate::stdlib::embedded::is_embedded(&u.path[0]) {
            match crate::stdlib::embedded::parsed(&u.path[0]) {
                Ok(Some(parsed)) => declared_names(&parsed, &mut excused),
                _ => return Vec::new(),
            }
            continue;
        }
        let brought = modules_used(&format!("use {}\n", u.path.join(".")), dir, false);
        if brought.is_empty() {
            return Vec::new();
        }
        for (_, _, module) in &brought {
            declared_names(module, &mut excused);
        }
    }
    // A variant arrives with the import of its type.
    for (_, _, module) in modules {
        for stmt in &module.statements {
            let decl = match stmt.unwrapped() {
                Statement::TypeDecl(t) => t,
                Statement::ShareDecl(crate::ast::ShareDecl::Type(t)) => t,
                _ => continue,
            };
            if let crate::ast::TypeDefinition::Enum { variants } = &decl.definition {
                excused.extend(variants.iter().map(|v| v.name.clone()));
            }
        }
    }
    candidates
        .into_iter()
        .filter(|name| !excused.contains(name))
        .map(|name| {
            let (line, column) = first_use_of(source, &name);
            CheckDiagnostic {
                line,
                column,
                message: format!(
                    "Undefined variable: {name} — nothing in this file defines or imports it \
                     (in a browser bundle it would resolve from a neighboring module, by accident)"
                ),
                runtime: true,
                warning: false,
                scope: false,
            }
        })
        .collect()
}

/// 1-based line and column of the first whole-word use of `name` outside
/// a `//` comment; (0, 0) when it is not found as written (an expansion).
fn first_use_of(source: &str, name: &str) -> (u32, u32) {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    for (index, line) in source.lines().enumerate() {
        let code = line.split("//").next().unwrap_or(line);
        let mut from = 0;
        while let Some(at) = code[from..].find(name) {
            let start = from + at;
            let end = start + name.len();
            let before = code[..start].chars().next_back();
            let after = code[end..].chars().next();
            if !before.is_some_and(|c| word(c) || c == '.') && !after.is_some_and(word) {
                return (index as u32 + 1, code[..start].chars().count() as u32 + 1);
            }
            from = end;
        }
    }
    (0, 0)
}

/// Warnings for the import-shadowing trap: a bare `use module` (a
/// wildcard import) that also exports a name an *earlier* explicit
/// import bound. The runtime silently rebinds — the failure then
/// surfaces at a distance, inside whichever call received the wrong
/// binding — so the collision is reported here, where both lines are
/// visible. Exports come from the module's `share` declarations,
/// resolved from disk for user modules and from the embedded registry
/// for stdlib packages; a module that cannot be resolved is skipped.
pub fn use_shadow_warnings(
    program: &Program,
    doc_dir: Option<&std::path::Path>,
) -> Vec<CheckDiagnostic> {
    fn share_names(p: &Program, out: &mut Vec<String>) {
        for stmt in &p.statements {
            if let Statement::ShareDecl(d) = stmt.unwrapped() {
                match d {
                    crate::ast::ShareDecl::Function(f) => out.push(f.name.clone()),
                    crate::ast::ShareDecl::Type(t) => out.push(t.name.clone()),
                    crate::ast::ShareDecl::Let(l) => {
                        if let crate::ast::Pattern::Identifier(n) = &l.pattern {
                            out.push(n.clone());
                        }
                    }
                    crate::ast::ShareDecl::Use(u) => {
                        for item in &u.items {
                            if let Some(n) = item.bound_name() {
                                out.push(n.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn exports_of(path: &[String], doc_dir: Option<&std::path::Path>) -> Option<Vec<String>> {
        let mut names = Vec::new();
        if path.len() == 1 && crate::stdlib::embedded::is_embedded(&path[0]) {
            let parsed = crate::stdlib::embedded::parsed(&path[0]).ok()??;
            share_names(&parsed, &mut names);
            return Some(names);
        }
        let dir = doc_dir?;
        let joined = path.join("/");
        let last = path.last()?;
        let candidates = [
            dir.join(format!("{}.ol", joined)),
            dir.join(&joined).join("index.ol"),
            dir.join(&joined).join("mod.ol"),
            dir.join(format!("{}.ol", last)),
        ];
        for c in candidates {
            if let Ok(src) = std::fs::read_to_string(&c) {
                let prog = crate::parser::Parser::new().parse(&src).ok()?;
                share_names(&prog, &mut names);
                return Some(names);
            }
        }
        None
    }

    let mut out = Vec::new();
    // A module's `share use` re-exports must bind each name once: two
    // re-exports of one name (the web SDK's `lib.sql.row` and
    // `lib.ui.row`) let the later silently win, and a consumer's
    // `use pkg { row }` failed far away with an arity mismatch.
    let mut reexported: Vec<(String, u32)> = Vec::new();
    for stmt in &program.statements {
        let (line, column) = match stmt {
            Statement::Located { line, column, .. } => (*line, *column),
            _ => (0, 0),
        };
        let Statement::ShareDecl(crate::ast::ShareDecl::Use(u)) = stmt.unwrapped() else {
            continue;
        };
        for item in &u.items {
            let Some(n) = item.bound_name() else { continue };
            if let Some((_, first_line)) = reexported.iter().find(|(name, _)| name == n) {
                out.push(CheckDiagnostic {
                    line,
                    column,
                    message: format!(
                        "`share use {}` re-exports '{}', already re-exported on line {} — \
the later binding wins silently for every importer; alias one of them \
(`{} as other_name`)",
                        u.path.join("."),
                        n,
                        first_line,
                        n
                    ),
                    runtime: false,
                    warning: true,
                    scope: false,
                });
            } else {
                reexported.push((n.to_string(), line));
            }
        }
    }
    // Explicit imports seen so far: bound name -> declaration line.
    let mut explicit: Vec<(String, u32)> = Vec::new();
    for stmt in &program.statements {
        let (line, column) = match stmt {
            Statement::Located { line, column, .. } => (*line, *column),
            _ => (0, 0),
        };
        let (Statement::UseDecl(u) | Statement::ShareDecl(crate::ast::ShareDecl::Use(u))) =
            stmt.unwrapped()
        else {
            continue;
        };
        let is_wildcard = u
            .items
            .iter()
            .any(|i| matches!(i, crate::ast::UseItem::Wildcard));
        if is_wildcard {
            let Some(exports) = exports_of(&u.path, doc_dir) else {
                continue;
            };
            let module = u.path.join(".");
            for (name, decl_line) in &explicit {
                if exports.iter().any(|e| e == name) {
                    out.push(CheckDiagnostic {
                        line,
                        column,
                        message: format!(
                            "`use {}` also exports '{}', shadowing the explicit import \
from line {} — call it as `{}.{}`, or alias the earlier import \
(`{} as other_name`)",
                            module,
                            name,
                            decl_line,
                            module.rsplit('.').next().unwrap_or(&module),
                            name,
                            name
                        ),
                        runtime: false,
                        warning: true,
                        scope: false,
                    });
                }
            }
        }
        for item in &u.items {
            if let Some(n) = item.bound_name() {
                explicit.push((n.to_string(), line));
            }
        }
    }
    out
}

/// Warnings for escape-looking sequences in template literals. Backtick
/// strings process no escapes — `\n` inside one is a backslash and an
/// `n`, and the mistake surfaces only at run time (a literal `\r` per
/// progress-bar frame). The lint reads the source *spelling*, because
/// the parsed literal cannot tell `\n` from a deliberate `\\n`: writing
/// `\\n` (the escaped backslash) keeps the same two output characters
/// and silences the warning, which makes it the opt-out for code that
/// wants the backslash (generated LaTeX, regex source, and the like).
/// Interpolations are exempt — a double-quoted `"\n"` inside `${...}`
/// is real string syntax and processes its escapes normally.
/// Two shadowing pitfalls the runtime cannot report at the right place:
///
/// - a parameter (or a path import's leaf) that reuses the name of a
///   function the same body then calls — `fn row(s, span) = span(…)`
///   with `span` imported fails at the call, in the browser, three frames
///   away from the parameter that caused it;
/// - a `let` that reuses a stdlib module's name (`let fs = …`) and so
///   turns every later `fs.exists(…)` in its scope into a field access on
///   a value.
pub fn shadow_warnings(program: &Program) -> Vec<CheckDiagnostic> {
    use crate::ast::Value;
    let nodes = crate::stdlib::meta::program_nodes(program);
    let stdlib: std::collections::HashSet<String> =
        crate::stdlib::get_stdlib().keys().cloned().collect();
    let str_of = |v: &Value| match v {
        Value::String(s) => Some(s.to_string()),
        _ => None,
    };
    let field = |m: &Value, k: &str| -> Option<Value> {
        match m {
            Value::Map(map) => map.get(k).cloned(),
            Value::Struct { fields, .. } => fields.get(k).cloned(),
            _ => None,
        }
    };
    let list_of = |v: Option<Value>| -> Vec<Value> {
        match v {
            Some(Value::List(items)) => items.as_ref().clone(),
            _ => Vec::new(),
        }
    };
    let line_of = |m: &Value| -> (u32, u32) {
        let l = match field(m, "line") {
            Some(Value::Integer(n)) => n as u32,
            _ => 0,
        };
        let c = match field(m, "column") {
            Some(Value::Integer(n)) => n as u32,
            _ => 0,
        };
        (l, c)
    };
    // Names a body may call that a parameter could shadow: imports and
    // module-level functions.
    let mut callable: std::collections::HashSet<String> = std::collections::HashSet::new();
    for node in &nodes {
        match field(node, "kind").and_then(|k| str_of(&k)).as_deref() {
            Some("use") => {
                for item in list_of(field(node, "items")) {
                    if let Some(n) = str_of(&item)
                        .or_else(|| field(&item, "alias").and_then(|a| str_of(&a)))
                        .or_else(|| field(&item, "name").and_then(|a| str_of(&a)))
                    {
                        callable.insert(n);
                    }
                }
            }
            Some("fn") => {
                if let Some(n) = field(node, "name").and_then(|n| str_of(&n)) {
                    callable.insert(n);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for node in &nodes {
        let kind = field(node, "kind").and_then(|k| str_of(&k));
        let (line, column) = line_of(node);
        match kind.as_deref() {
            Some("fn") => {
                let name = field(node, "name")
                    .and_then(|n| str_of(&n))
                    .unwrap_or_default();
                let params: Vec<String> = list_of(field(node, "params"))
                    .iter()
                    .filter_map(|p| str_of(p).or_else(|| field(p, "name").and_then(|n| str_of(&n))))
                    .collect();
                let body = field(node, "body").map(|b| vec![b]).unwrap_or_default();
                let mut called: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for n in flatten_ast(&body) {
                    if field(&n, "kind").and_then(|k| str_of(&k)).as_deref() == Some("call")
                        && let Some(t) = field(&n, "target").and_then(|t| str_of(&t))
                    {
                        called.insert(t);
                    }
                }
                for p in params {
                    if callable.contains(&p) && called.contains(&p) {
                        out.push(CheckDiagnostic {
                            line,
                            column,
                            message: format!(
                                "parameter '{}' of `{}` shadows the function '{}' that this body \
calls — the call reaches the argument, not the function; rename the parameter",
                                p, name, p
                            ),
                            runtime: false,
                            warning: true,
                            scope: false,
                        });
                    }
                }
            }
            Some("use") => {
                let path = match field(node, "path") {
                    Some(Value::String(p)) => p.split('.').map(str::to_string).collect::<Vec<_>>(),
                    other => list_of(other).iter().filter_map(str_of).collect::<Vec<_>>(),
                };
                if path.len() >= 2
                    && let Some(leaf) = path.last()
                    && stdlib.contains(leaf)
                {
                    out.push(CheckDiagnostic {
                        line,
                        column,
                        message: format!(
                            "`use {}` binds `{}`, the name of a stdlib module: `{}.…` now reaches \
this module, not the standard library — rename the file if that is not intended",
                            path.join("."),
                            leaf,
                            leaf
                        ),
                        runtime: false,
                        warning: true,
                        scope: false,
                    });
                }
            }
            _ => {}
        }
    }
    // `let` bindings, at any depth, that take a stdlib module's name.
    for n in flatten_ast(&nodes) {
        if field(&n, "kind").and_then(|k| str_of(&k)).as_deref() == Some("let")
            && let Some(name) = field(&n, "name").and_then(|v| str_of(&v))
            && stdlib.contains(&name)
        {
            let (line, column) = line_of(&n);
            out.push(CheckDiagnostic {
                line,
                column,
                message: format!(
                    "`let {}` shadows the stdlib module `{}` for the rest of its scope — a later \
`{}.…` reaches this binding, not the module",
                    name, name, name
                ),
                runtime: false,
                warning: true,
                scope: false,
            });
        }
    }
    out
}

/// `olang check --fix`: apply the rewrites whose meaning is unambiguous.
/// Today that is the template-escape lint — `\n`, `\t`, `\r` inside a
/// backtick template become `${"\n"}` and so on, the control character
/// taken from a double-quoted string exactly as the warning says.
/// Returns (file, rewrites) for every file that changed.
pub fn fix(paths: &[PathBuf]) -> Vec<(PathBuf, usize)> {
    let mut out = Vec::new();
    for path in paths {
        for file in crate::tools::discover_ol_files(path) {
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
            let (fixed, n) = fix_template_escapes(&source);
            if n > 0 && std::fs::write(&file, fixed).is_ok() {
                out.push((file, n));
            }
        }
    }
    out
}

/// The template-escape rewrite over one source text: every `\n`, `\t`,
/// `\r` the lint flags becomes `${"\n"}`… Returns the text and how many
/// rewrites it made. The lint reports one position per escape kind per
/// template, so the rewrite runs to a fixed point.
pub fn fix_template_escapes(source: &str) -> (String, usize) {
    let mut text = source.to_string();
    let mut total = 0usize;
    for _ in 0..1000 {
        let warnings = template_escape_warnings(&text);
        let Some(first) = warnings.first() else { break };
        // The message names the escape: "`\n` in a backtick template …".
        let Some(esc) = first
            .message
            .strip_prefix("`\\")
            .and_then(|r| r.chars().next())
        else {
            break;
        };
        let at = byte_offset_of(&text, first.line as usize, first.column as usize);
        if !text[at..].starts_with(&format!("\\{esc}")) {
            break;
        }
        text.replace_range(at..at + 2, &format!("${{\"\\{esc}\"}}"));
        total += 1;
    }
    (text, total)
}

pub fn template_escape_warnings(source: &str) -> Vec<CheckDiagnostic> {
    fn advance(c: char, line: &mut u32, col: &mut u32) {
        if c == '\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
    }
    let mut out = Vec::new();
    for (start_line, start_col, raw) in crate::parser::Parser::template_literal_spans(source) {
        let mut line = start_line;
        let mut col = start_col;
        let mut warned: Vec<char> = Vec::new();
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            let (at_line, at_col) = (line, col);
            advance(ch, &mut line, &mut col);
            match ch {
                '\\' => match chars.peek().copied() {
                    // The three real template escapes; `\\` is also the
                    // lint's opt-out spelling.
                    Some('`') | Some('$') | Some('\\') => {
                        let next = chars.next().expect("peeked");
                        advance(next, &mut line, &mut col);
                    }
                    Some(esc @ ('n' | 't' | 'r')) if !warned.contains(&esc) => {
                        warned.push(esc);
                        out.push(CheckDiagnostic {
                            line: at_line,
                            column: at_col,
                            message: format!(
                                "`\\{esc}` in a backtick template is a literal \
                                 backslash and '{esc}' — templates process no \
                                 escapes. Take the control character from a \
                                 double-quoted string, or write `\\\\{esc}` to \
                                 say the two characters are deliberate"
                            ),
                            runtime: false,
                            warning: true,
                            scope: false,
                        });
                    }
                    _ => {}
                },
                // Skip `${...}` bodies: they are expression syntax, where
                // double-quoted strings process escapes normally. Mirrors
                // the parser's brace/string tracking.
                '$' if chars.peek() == Some(&'{') => {
                    let brace = chars.next().expect("peeked");
                    advance(brace, &mut line, &mut col);
                    let mut depth = 1;
                    let mut in_string = false;
                    let mut prev_escape = false;
                    for c in chars.by_ref() {
                        advance(c, &mut line, &mut col);
                        if in_string {
                            if prev_escape {
                                prev_escape = false;
                            } else if c == '\\' {
                                prev_escape = true;
                            } else if c == '"' {
                                in_string = false;
                            }
                            continue;
                        }
                        match c {
                            '"' => in_string = true,
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The modules a source file `use`s, resolved with the runtime's local
/// conventions (`use a.b` → a/b.ol, a/b/index.ol, a/b/mod.ol, or b.ol in
/// the same directory), read and parsed. Failures are silently skipped —
/// checking degrades to single-file. Capped to bound cost.
pub fn module_programs(
    text: &str,
    doc_dir: Option<&std::path::Path>,
) -> Vec<(PathBuf, String, Program)> {
    modules_used(text, doc_dir, false)
}

/// A module's tree, parsed once per process while the file stands as it
/// is: a run over an application checks fifty files that import the same
/// dozen modules, and parsing (and macro-expanding) each of them for each
/// importer made `olang check .` a minute long.
fn parsed_module(path: &Path, src: &str) -> Option<Program> {
    thread_local! {
        static PARSED: std::cell::RefCell<HashMap<PathBuf, (u64, Option<Program>)>> =
            std::cell::RefCell::new(HashMap::new());
    }
    // The source's length and a cheap hash of it: an edit between two
    // checks (the language server) is a different key.
    let stamp = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        src.hash(&mut h);
        h.finish()
    };
    PARSED.with(|cache| {
        if let Some((have, program)) = cache.borrow().get(path)
            && *have == stamp
        {
            return program.clone();
        }
        let program = crate::parser::Parser::new()
            .parse_with_dir(src, path.parent())
            .ok();
        cache
            .borrow_mut()
            .insert(path.to_path_buf(), (stamp, program.clone()));
        program
    })
}

/// `module_programs`, or — for a module it found — only what that module
/// re-exports (`share use`): a module's private imports are not part of
/// what importing it brings in, and following them made the walk
/// exponential in the depth of an application's import graph.
fn modules_used(
    text: &str,
    doc_dir: Option<&std::path::Path>,
    reexports_only: bool,
) -> Vec<(PathBuf, String, Program)> {
    let Some(dir) = doc_dir else {
        return Vec::new();
    };
    let Ok(program) = crate::parser::Parser::new().parse(text) else {
        return Vec::new();
    };
    // The modules the file names, and apart from them the modules those
    // re-export. Re-exports are returned FIRST: a later signature replaces
    // an earlier one of the same name, and the function a file imports
    // from a module it names must win over a like-named one some package
    // re-exports (the SDK's `watch(keys)` stood in for an app module's
    // `watch(conn, cfg, id, who)` — and did so because the SDK's dozen
    // re-exports used up a cap of sixteen before the app's module loaded).
    let mut out: Vec<(PathBuf, String, Program)> = Vec::new();
    let mut reexported: Vec<(PathBuf, String, Program)> = Vec::new();
    for stmt in &program.statements {
        if out.len() >= 64 {
            break;
        }
        let u = match stmt.unwrapped() {
            Statement::UseDecl(u) if !reexports_only => u,
            Statement::ShareDecl(crate::ast::ShareDecl::Use(u)) => u,
            _ => continue,
        };
        if u.path.is_empty() {
            continue;
        }
        let joined = u.path.join("/");
        let last = u.path.last().cloned().unwrap_or_default();
        // The runtime's order: the file's directory, then its own
        // package's root (a nested module's `use lib.x` means its
        // package's lib/x.ol, wherever the check was started), then a
        // package dependency's index or a module inside it.
        let mut candidates = vec![
            dir.join(format!("{}.ol", joined)),
            dir.join(&joined).join("index.ol"),
            dir.join(&joined).join("mod.ol"),
        ];
        let absolute = std::path::absolute(if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        })
        .unwrap_or_else(|_| dir.to_path_buf());
        if let Some(root) = crate::pkg::manifest::Manifest::find_root(&absolute) {
            candidates.push(root.join(format!("{}.ol", joined)));
            candidates.push(root.join(&joined).join("index.ol"));
        }
        if let Some(pkg) = crate::expand::package_dir(&u.path[0], Some(dir)) {
            if u.path.len() == 1 {
                candidates.push(pkg.join("index.ol"));
            } else {
                let rest = u.path[1..].join("/");
                candidates.push(pkg.join(format!("{}.ol", rest)));
                candidates.push(pkg.join(&rest).join("index.ol"));
            }
        }
        candidates.push(dir.join(format!("{}.ol", last)));
        for c in candidates {
            if out.iter().any(|(p, _, _)| *p == c) {
                break;
            }
            if let Ok(src) = std::fs::read_to_string(&c) {
                if let Some(prog) = parsed_module(&c, &src) {
                    // A package's index re-exports: the modules it
                    // `share use`s carry the signatures and types.
                    let reexports = modules_used(&src, c.parent(), true);
                    out.push((c, src, prog));
                    for r in reexports {
                        if reexported.len() < 96 && !reexported.iter().any(|(p, _, _)| *p == r.0) {
                            reexported.push(r);
                        }
                    }
                }
                break;
            }
        }
    }
    reexported.retain(|(path, _, _)| !out.iter().any(|(p, _, _)| p == path));
    reexported.extend(out);
    reexported
}

/// Byte offset of a 1-based (line, column) position — what miette's span
/// labels want. Clamped to the source length.
fn byte_offset_of(source: &str, line: usize, column: usize) -> usize {
    let mut offset = 0usize;
    for (i, l) in source.split('\n').enumerate() {
        if i + 1 == line {
            let col_bytes: usize = l
                .chars()
                .take(column.saturating_sub(1))
                .map(|c| c.len_utf8())
                .sum();
            return (offset + col_bytes).min(source.len());
        }
        offset += l.len() + 1;
    }
    offset.min(source.len())
}

/// A provable annotation violation, positioned at the statement carrying it.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckDiagnostic {
    pub line: u32,
    pub column: u32,
    pub message: String,
    /// True when the runtime's own (shallow) enforcement would also reject
    /// this — the message then matches the runtime's error text exactly.
    /// False for element-level breaks the runtime deliberately lets pass.
    pub runtime: bool,
    /// True for advisory findings (style/pitfall warnings): reported and
    /// surfaced as warnings, never a non-zero exit on their own.
    pub warning: bool,
    /// True for scope and mutability violations, which the same validator
    /// raises before execution — the program is refused, not attempted.
    /// Only the rendered label distinguishes them; they gate like any
    /// other problem.
    pub scope: bool,
}

/// The checker's knowledge of a type, structurally deep where the source
/// says so. `Unknown` is the gradual escape hatch: anything dynamic,
/// erased (generic type parameters), ambiguous, or simply not modeled
/// stays Unknown and is never reported. The *base* of every non-Unknown
/// variant agrees with the runtime's shallow `FieldTypeCheck` vocabulary,
/// so base-level verdicts match the runtime's exactly.
#[derive(Debug, Clone, PartialEq)]
enum SType {
    Unknown,
    Int,
    Float,
    Bool,
    String,
    List(Box<SType>),
    Map(Box<SType>, Box<SType>),
    Tuple(Vec<SType>),
    Result(Box<SType>, Box<SType>),
    /// `(A, B) -> R`. `required` is the callable's no-default parameter
    /// floor (annotations have no defaults, so theirs equals the count).
    Function {
        params: Vec<SType>,
        required: usize,
        ret: Box<SType>,
    },
    /// A literal type: satisfied only by exactly that value. The checker
    /// proves violations when the expression is itself a scalar literal
    /// (or a binding annotated with a different literal).
    Lit(crate::ast::LitCheck),
    /// `A | B`. Only constructed when the runtime enforces the union too
    /// (every branch checkable), so verdict parity holds; otherwise the
    /// annotation reduces to Unknown, silent like the runtime.
    Union(Vec<SType>),
    Named(std::string::String),
    /// `{ name: Type, ... }`: the keys a map or record is declared to
    /// carry. Checker-only — the runtime does not enforce a shape — so
    /// it never claims a base and never proves a violation; it answers
    /// at `map_get`/field/index sites with a literal key.
    Record(Vec<(std::string::String, SType)>),
}

impl SType {
    /// Deep reduction of a source annotation. Mirrors the runtime's
    /// `FieldTypeCheck::from_annotation` at the base level — wherever the
    /// runtime has no check, this is Unknown — and additionally keeps
    /// element structure the runtime discards.
    fn from_annotation(ann: &TypeAnnotation, type_params: &[String]) -> SType {
        match ann {
            TypeAnnotation::Int => SType::Int,
            TypeAnnotation::Float => SType::Float,
            TypeAnnotation::Bool => SType::Bool,
            TypeAnnotation::String => SType::String,
            TypeAnnotation::List(t) => {
                SType::List(Box::new(SType::from_annotation(t, type_params)))
            }
            TypeAnnotation::Map {
                key_type,
                value_type,
            } => SType::Map(
                Box::new(SType::from_annotation(key_type, type_params)),
                Box::new(SType::from_annotation(value_type, type_params)),
            ),
            TypeAnnotation::Tuple(ts) => SType::Tuple(
                ts.iter()
                    .map(|t| SType::from_annotation(t, type_params))
                    .collect(),
            ),
            TypeAnnotation::Result { ok_type, err_type } => SType::Result(
                Box::new(SType::from_annotation(ok_type, type_params)),
                Box::new(SType::from_annotation(err_type, type_params)),
            ),
            TypeAnnotation::Function {
                params,
                return_type,
            } => SType::Function {
                params: params
                    .iter()
                    .map(|t| SType::from_annotation(t, type_params))
                    .collect(),
                required: params.len(),
                ret: Box::new(SType::from_annotation(return_type, type_params)),
            },
            TypeAnnotation::Literal { value } => match value.as_ref() {
                crate::ast::Value::Integer(i) => SType::Lit(crate::ast::LitCheck::Int(*i)),
                crate::ast::Value::String(st) => {
                    SType::Lit(crate::ast::LitCheck::Str(st.as_ref().clone()))
                }
                crate::ast::Value::Boolean(b) => SType::Lit(crate::ast::LitCheck::Bool(*b)),
                _ => SType::Unknown,
            },
            TypeAnnotation::Union { types } => {
                // Mirror the runtime: a union with an unenforceable branch
                // is entirely unchecked there, so it must be silent here.
                if types
                    .iter()
                    .any(|t| crate::ast::FieldTypeCheck::from_annotation(t, type_params).is_none())
                {
                    SType::Unknown
                } else {
                    SType::Union(
                        types
                            .iter()
                            .map(|t| SType::from_annotation(t, type_params))
                            .collect(),
                    )
                }
            }
            TypeAnnotation::Custom(name) if !type_params.iter().any(|p| p == name) => {
                // A `type Name = <annotation>` alias reduces to what it
                // names (the aliases of the program under check are
                // installed for the run by `check_program_with_context`).
                if let Some(target) = CHECK_ALIASES.with(|a| {
                    a.borrow().as_ref().and_then(|table| {
                        table
                            .get(name)
                            .map(|t| crate::ast::resolve_type_aliases(t, table, 16))
                    })
                }) {
                    return SType::from_annotation(&target, type_params);
                }
                SType::Named(name.clone())
            }
            TypeAnnotation::Record { fields } => SType::Record(
                fields
                    .iter()
                    .map(|f| {
                        (
                            f.name.clone(),
                            SType::from_annotation(&f.field_type, type_params),
                        )
                    })
                    .collect(),
            ),
            TypeAnnotation::Generic {
                base_type,
                type_args,
            } if !type_params.iter().any(|p| p == base_type) => match base_type.as_str() {
                "List" => SType::List(Box::new(
                    type_args
                        .first()
                        .map(|t| SType::from_annotation(t, type_params))
                        .unwrap_or(SType::Unknown),
                )),
                "Map" => SType::Map(
                    Box::new(
                        type_args
                            .first()
                            .map(|t| SType::from_annotation(t, type_params))
                            .unwrap_or(SType::Unknown),
                    ),
                    Box::new(
                        type_args
                            .get(1)
                            .map(|t| SType::from_annotation(t, type_params))
                            .unwrap_or(SType::Unknown),
                    ),
                ),
                "Result" => SType::Result(
                    Box::new(
                        type_args
                            .first()
                            .map(|t| SType::from_annotation(t, type_params))
                            .unwrap_or(SType::Unknown),
                    ),
                    Box::new(
                        type_args
                            .get(1)
                            .map(|t| SType::from_annotation(t, type_params))
                            .unwrap_or(SType::Unknown),
                    ),
                ),
                other => SType::Named(other.to_string()),
            },
            // Type variables, functions, Result, Unit, constrained forms:
            // the runtime has no check here; neither do we.
            _ => SType::Unknown,
        }
    }

    /// The base (shallow) name — what the runtime's error text uses, and
    /// what a value's runtime `type_name` reports. None for Unknown.
    fn base_name(&self) -> Option<&str> {
        match self {
            SType::Unknown => None,
            SType::Int => Some("Int"),
            SType::Float => Some("Float"),
            SType::Bool => Some("Bool"),
            SType::String => Some("String"),
            SType::List(_) => Some("List"),
            SType::Map(_, _) => Some("Map"),
            SType::Tuple(_) => Some("Tuple"),
            SType::Result(_, _) => Some("Result"),
            SType::Function { .. } => Some("Function"),
            SType::Lit(crate::ast::LitCheck::Int(_)) => Some("Int"),
            SType::Lit(crate::ast::LitCheck::Str(_)) => Some("String"),
            SType::Lit(crate::ast::LitCheck::Bool(_)) => Some("Bool"),
            // A union has no single base; violation() handles it directly.
            SType::Union(_) => None,
            SType::Named(n) => Some(n),
            // A shape is a promise about keys, not about the value's kind
            // (a `#{}` map and a `{ }` record both carry keys).
            SType::Record(_) => None,
        }
    }

    /// Deep display for element-level messages: `List<Int>`,
    /// `Map<String, Int>`, `(Int, String)`. Unknown parts render as `?`;
    /// a fully-unknown container falls back to its base name.
    fn display(&self) -> String {
        match self {
            SType::Unknown => "?".to_string(),
            SType::List(t) if **t == SType::Unknown => "List".to_string(),
            SType::List(t) => format!("List<{}>", t.display()),
            SType::Map(k, v) if **k == SType::Unknown && **v == SType::Unknown => "Map".to_string(),
            SType::Map(k, v) => format!("Map<{}, {}>", k.display(), v.display()),
            SType::Result(o, e) if **o == SType::Unknown && **e == SType::Unknown => {
                "Result".to_string()
            }
            SType::Result(o, e) => format!("Result<{}, {}>", o.display(), e.display()),
            SType::Function { params, ret, .. } => format!(
                "({}) -> {}",
                params
                    .iter()
                    .map(|p| p.display())
                    .collect::<Vec<_>>()
                    .join(", "),
                ret.display()
            ),
            SType::Lit(lit) => lit.display(),
            SType::Union(bs) => bs
                .iter()
                .map(|b| b.display())
                .collect::<Vec<_>>()
                .join(" | "),
            SType::Tuple(ts) => format!(
                "({})",
                ts.iter()
                    .map(|t| t.display())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            SType::Record(fields) => format!(
                "{{ {} }}",
                fields
                    .iter()
                    .map(|(n, t)| format!("{}: {}", n, t.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            other => other.base_name().unwrap_or("?").to_string(),
        }
    }
}

/// Levenshtein distance, for the "did you mean" on a shape key.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The warning classes the nearest `olang.toml` above `dir` promotes to
/// errors (`[check] promote = [...]`); empty without a manifest.
pub fn promotions_for(dir: Option<&Path>) -> Vec<String> {
    let start = dir
        .map(|d| d.to_path_buf())
        .or_else(|| std::env::current_dir().ok());
    let Some(start) = start else {
        return Vec::new();
    };
    let start = std::path::absolute(if start.as_os_str().is_empty() {
        Path::new(".")
    } else {
        start.as_path()
    })
    .unwrap_or(start);
    let Some(root) = crate::pkg::manifest::Manifest::find_root(&start) else {
        return Vec::new();
    };
    crate::pkg::manifest::Manifest::load(&root)
        .ok()
        .and_then(|m| m.check)
        .map(|c| c.promote)
        .unwrap_or_default()
}

/// The class a warning belongs to, by its message: `exhaustiveness`,
/// `shape`, `result`, `shadow`, `copy`, or none.
pub fn warning_class(message: &str) -> Option<&'static str> {
    if message.contains("is not exhaustive") || message.contains("covers none of the scrutinee") {
        Some("exhaustiveness")
    } else if message.contains("is not a key of the declared shape") {
        Some("shape")
    } else if message.contains("the Result from") && message.contains("is discarded") {
        Some("result")
    } else if message.contains("is copied on every pass") || message.contains("moves every element")
    {
        // A loop that rebuilds the collection it is accumulating.
        Some("copy")
    } else if message.contains("shadows the stdlib module") {
        // `let cell = …`: every later `cell.get` in the scope reaches the
        // binding and fails at run time, far from the `let`.
        Some("shadow")
    } else {
        None
    }
}

/// Turn a warning into an error when its class is promoted.
pub fn promote(d: &mut CheckDiagnostic, promoted: &[String]) {
    if !d.warning || promoted.is_empty() {
        return;
    }
    let Some(class) = warning_class(&d.message) else {
        return;
    };
    if promoted.iter().any(|p| p == class || p == "all") {
        d.warning = false;
        d.message = format!(
            "{} (promoted to an error by [check] promote in olang.toml)",
            d.message
        );
    }
}

/// The most specific type both sides agree on; Unknown on any conflict.
/// Deliberately not a union — a list holding an Int and a String infers as
/// `List<?>`, and the literal-decomposition path is what flags its
/// elements precisely.
fn unify(a: &SType, b: &SType) -> SType {
    match (a, b) {
        _ if a == b => a.clone(),
        (SType::List(x), SType::List(y)) => SType::List(Box::new(unify(x, y))),
        (SType::Map(k1, v1), SType::Map(k2, v2)) => {
            SType::Map(Box::new(unify(k1, k2)), Box::new(unify(v1, v2)))
        }
        (SType::Tuple(xs), SType::Tuple(ys)) if xs.len() == ys.len() => {
            SType::Tuple(xs.iter().zip(ys).map(|(x, y)| unify(x, y)).collect())
        }
        (SType::Result(o1, e1), SType::Result(o2, e2)) => {
            SType::Result(Box::new(unify(o1, o2)), Box::new(unify(e1, e2)))
        }
        _ => SType::Unknown,
    }
}

/// A provable disagreement between a declared type and a known actual
/// type: `(expected_text, actual_text, runtime_would_reject)`. None
/// wherever anything is Unknown or the types agree.
fn violation(expected: &SType, actual: &SType) -> Option<(String, String, bool)> {
    // A union is violated only when every branch is provably violated;
    // the runtime would reject only if every branch's own check would.
    if let SType::Union(branches) = expected {
        if *actual == SType::Unknown {
            return None;
        }
        let mut all_runtime = true;
        for b in branches {
            match violation(b, actual) {
                Some((_, _, rt)) => all_runtime = all_runtime && rt,
                None => return None,
            }
        }
        return Some((expected.display(), actual.display(), all_runtime));
    }
    // Literal expectations: provable only against a known literal value
    // (equal → satisfied, different → runtime-rejected) or a different
    // base. A plain same-base value stays silent — its value is unknown.
    if let SType::Lit(exp_lit) = expected {
        return match actual {
            SType::Lit(act_lit) => {
                (exp_lit != act_lit).then(|| (exp_lit.display(), act_lit.display(), true))
            }
            other => match other.base_name() {
                Some(name) if name != expected.base_name().unwrap_or("") => {
                    Some((exp_lit.display(), name.to_string(), true))
                }
                _ => None,
            },
        };
    }
    let be = expected.base_name()?;
    let ba = actual.base_name()?;
    if be != ba {
        // The runtime's shallow check fails too; use its vocabulary —
        // except function expectations, which name the signature.
        let expected_text = match expected {
            SType::Function { .. } => expected.display(),
            _ => be.to_string(),
        };
        return Some((expected_text, ba.to_string(), true));
    }
    // Function vs function: arity is runtime-checked (the value exposes
    // its parameter counts); signature types inside are checker-only.
    if let (
        SType::Function {
            params: ep,
            ret: er,
            ..
        },
        SType::Function {
            params: ap,
            required: areq,
            ret: ar,
        },
    ) = (expected, actual)
    {
        let n = ep.len();
        if n < *areq || n > ap.len() {
            let desc = if *areq == ap.len() {
                format!(
                    "a function taking {} parameter{}",
                    ap.len(),
                    if ap.len() == 1 { "" } else { "s" }
                )
            } else {
                format!("a function taking {} to {} parameters", areq, ap.len())
            };
            return Some((expected.display(), desc, true));
        }
        let deep = ep.iter().zip(ap).any(|(e, a)| violation(e, a).is_some())
            || violation(er, ar).is_some();
        return deep.then(|| (expected.display(), actual.display(), false));
    }
    // Same base: refine only within matching structural constructors.
    let deep = match (expected, actual) {
        (SType::List(x), SType::List(y)) => violation(x, y).is_some(),
        (SType::Map(k1, v1), SType::Map(k2, v2)) => {
            violation(k1, k2).is_some() || violation(v1, v2).is_some()
        }
        (SType::Tuple(xs), SType::Tuple(ys)) => {
            xs.len() != ys.len() || xs.iter().zip(ys).any(|(x, y)| violation(x, y).is_some())
        }
        (SType::Result(o1, e1), SType::Result(o2, e2)) => {
            violation(o1, o2).is_some() || violation(e1, e2).is_some()
        }
        _ => false,
    };
    deep.then(|| (expected.display(), actual.display(), false))
}

/// What an argument must be for the call not to fail when it runs.
#[derive(Debug, Clone, PartialEq)]
enum Need {
    /// A list or a range — the first argument of `fold`, `map`, `len`, …
    /// A number or a boolean there is refused by every one of them.
    Sequence(String),
    /// A map, a record or an object — the first argument of `map_get` and
    /// its family. Anything else is refused.
    MapLike(String),
    /// A callable taking this many arguments: the parameter is called
    /// with them.
    Callable(usize, String),
    /// A string: the parameter is added to a string literal, and `+`
    /// refuses a String and anything else ("use to_string(...)").
    Text,
}

/// What a builtin needs of its first argument. Only the builtins, and
/// only the kinds, that the runtime refuses on both tiers (checked one by
/// one): everything absent from this table is unknown and never reported.
fn builtin_need(name: &str) -> Option<Need> {
    match name {
        "map" | "filter" | "fold" | "reduce" | "sum" | "head" | "tail" | "sort" | "reverse"
        | "join" | "take" | "drop" | "flatten" | "zip" | "len" | "contains" => {
            Some(Need::Sequence(name.to_string()))
        }
        "map_get" | "map_set" | "map_keys" | "map_values" | "map_has_key" | "map_remove" => {
            Some(Need::MapLike(name.to_string()))
        }
        _ => None,
    }
}

impl Need {
    /// Why a value of type `ty` cannot satisfy this need — None when it
    /// can, or when the checker does not know.
    fn refuses(&self, ty: &SType) -> Option<String> {
        let kind = match ty {
            SType::Int => "an Int",
            SType::Float => "a Float",
            SType::Bool => "a Bool",
            SType::String => "a String",
            SType::List(_) => "a List",
            SType::Function { .. } => "a function",
            _ => return None,
        };
        match self {
            Need::Text => matches!(ty, SType::Int | SType::Float | SType::Bool | SType::List(_))
                .then(|| {
                    format!("{kind}, added to a string there: convert it with `to_string(...)`")
                }),
            Need::Sequence(builtin) => matches!(ty, SType::Int | SType::Float | SType::Bool)
                .then(|| format!("{kind}, and `{builtin}` needs a list")),
            Need::MapLike(builtin) => matches!(
                ty,
                SType::Int | SType::Float | SType::Bool | SType::String | SType::List(_)
            )
            .then(|| format!("{kind}, and `{builtin}` needs a map or a record")),
            Need::Callable(count, how) => match ty {
                SType::Function {
                    params, required, ..
                } if *count < *required || *count > params.len() => Some(format!(
                    "a function of {} parameter{}, and {how} calls it with {count}",
                    params.len(),
                    if params.len() == 1 { "" } else { "s" }
                )),
                SType::Function { .. } => None,
                _ => Some(format!("{kind}, and {how} calls it")),
            },
        }
    }
}

/// What `body` shows of its parameters on the path every call takes: the
/// body itself and the statements of its block, through call arguments,
/// operands, pipelines and `let` values — never into a branch, a loop, a
/// lambda or the right of `&&`/`||`, where a guard may stand. A parameter
/// rebound by a `let` stops being the parameter from there on.
fn parameter_needs(function: &str, parameters: &[String], body: &Expr) -> Vec<Option<Need>> {
    fn walk(e: &Expr, function: &str, live: &[String], all: &[String], out: &mut [Option<Need>]) {
        let mut note = |name: &str, need: Need| {
            if live.iter().any(|p| p == name)
                && let Some(i) = all.iter().position(|p| p == name)
                && out[i].is_none()
            {
                out[i] = Some(need);
            }
        };
        match e {
            Expr::Call { callee, arguments } => {
                if let Expr::Identifier(name) = callee.as_ref() {
                    if let (Some(need), Some(Argument::Positional(Expr::Identifier(first)))) =
                        (builtin_need(name), arguments.first())
                    {
                        note(first, need);
                    }
                    if arguments
                        .iter()
                        .all(|a| matches!(a, Argument::Positional(_)))
                    {
                        note(
                            name,
                            Need::Callable(arguments.len(), format!("`{function}`")),
                        );
                    }
                }
                for a in arguments {
                    let (Argument::Positional(inner) | Argument::Named { value: inner, .. }) = a;
                    walk(inner, function, live, all, out);
                }
            }
            Expr::BinaryOp { left, op, right } => {
                if matches!(op, crate::ast::BinaryOp::Add) {
                    match (left.as_ref(), right.as_ref()) {
                        (Expr::String(_), Expr::Identifier(p))
                        | (Expr::Identifier(p), Expr::String(_)) => note(p, Need::Text),
                        _ => {}
                    }
                }
                walk(left, function, live, all, out);
                if !matches!(op, crate::ast::BinaryOp::And | crate::ast::BinaryOp::Or) {
                    walk(right, function, live, all, out);
                }
            }
            Expr::Pipeline { left, right } => {
                walk(left, function, live, all, out);
                walk(right, function, live, all, out);
            }
            Expr::Block(statements) => {
                let mut live: Vec<String> = live.to_vec();
                for statement in statements {
                    match statement.unwrapped() {
                        Statement::Expression(inner) => walk(inner, function, &live, all, out),
                        Statement::LetDecl(decl) => {
                            if let Some(value) = &decl.value {
                                walk(value, function, &live, all, out);
                            }
                            let mut bound = std::collections::HashSet::new();
                            crate::resolve::pattern_names_of(&decl.pattern, &mut bound);
                            live.retain(|p| !bound.contains(p));
                        }
                        _ => return,
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = vec![None; parameters.len()];
    walk(body, function, parameters, parameters, &mut out);
    out
}

/// A known function's checkable surface.
struct FnSig {
    param_names: Vec<String>,
    params: Vec<SType>,
    /// Parameters without default values — the arity floor.
    required: usize,
    total: usize,
    ret: SType,
}

pub fn check_program(program: &Program) -> Vec<CheckDiagnostic> {
    check_program_with_context(&[], program)
}

thread_local! {
    /// The `type Name = <annotation>` aliases of the program under check
    /// (its context modules included), read by `SType::from_annotation`.
    static CHECK_ALIASES: std::cell::RefCell<
        Option<HashMap<String, crate::ast::TypeAnnotation>>,
    > = const { std::cell::RefCell::new(None) };
}

/// The names `program` imports item by item — or `None` when any `use`
/// is bare or a wildcard, and what it brings in cannot be listed from the
/// file alone.
fn imported_names(program: &Program) -> Option<std::collections::HashSet<String>> {
    let mut names = std::collections::HashSet::new();
    let mut any_use = false;
    for stmt in &program.statements {
        let (Statement::UseDecl(u) | Statement::ShareDecl(crate::ast::ShareDecl::Use(u))) =
            stmt.unwrapped()
        else {
            continue;
        };
        any_use = true;
        if u.items.is_empty() {
            return None;
        }
        for item in &u.items {
            match item {
                crate::ast::UseItem::Specific(name) => {
                    names.insert(name.clone());
                }
                // An alias is a new name for the function: its signature
                // is not collected under it, so it is simply not checked.
                crate::ast::UseItem::Aliased { .. } => {}
                crate::ast::UseItem::Wildcard => return None,
            }
        }
    }
    // A context handed to a file with no `use` at all came from the
    // caller's own knowledge, not from this file's imports: keep it.
    any_use.then_some(names)
}

/// Check `program` with signatures collected from `context` first — the
/// modules a file `use`s, resolved and parsed by the caller (the LSP).
/// Only `program`'s statements are walked; context contributes function
/// signatures and struct shapes.
pub fn check_program_with_context(context: &[&Program], program: &Program) -> Vec<CheckDiagnostic> {
    let mut checker = Checker::default();
    // Aliases first, so a signature collected below reduces through them
    // whatever the declaration order.
    let mut aliases: HashMap<String, crate::ast::TypeAnnotation> = HashMap::new();
    for p in context.iter().copied().chain(std::iter::once(program)) {
        for (name, target) in crate::ast::alias_declarations(&p.statements) {
            aliases.insert(name, target);
        }
    }
    CHECK_ALIASES.with(|a| *a.borrow_mut() = Some(aliases));
    // Two context modules may each declare a function of one name — the
    // SDK's `watch(keys)` and an application package's `watch(conn, cfg,
    // id, who)` — and the file means whichever it imported, which a flat
    // list of programs cannot say. A name declared twice with different
    // shapes has no signature here: unknown, and so never reported.
    let mut shapes: HashMap<String, (Vec<String>, usize, usize)> = HashMap::new();
    let mut ambiguous: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in context {
        let mut one = Checker::default();
        one.collect(p);
        for (name, sig) in &one.sigs {
            let shape = (sig.param_names.clone(), sig.required, sig.total);
            match shapes.get(name) {
                Some(seen) if *seen != shape => {
                    ambiguous.insert(name.clone());
                }
                _ => {
                    shapes.insert(name.clone(), shape);
                }
            }
        }
        checker.collect(p);
    }
    checker.sigs.retain(|name, _| !ambiguous.contains(name));
    checker.needs.retain(|name, _| !ambiguous.contains(name));
    // A context module's functions are signatures for THIS file only
    // under the names it imported. `use web { rpc, route }` does not put
    // the SDK's `p(attrs, children)` in scope, and a local `p` called
    // with one argument is not a call of it. With a bare `use m` (every
    // shared name arrives) the set is unknown, and all are kept.
    if let Some(imported) = imported_names(program) {
        checker.sigs.retain(|name, _| imported.contains(name));
        checker.needs.retain(|name, _| imported.contains(name));
    }
    checker.collect(program);
    checker.push_scope();
    for stmt in &program.statements {
        checker.check_statement(stmt, (0, 0));
    }

    // Scope and mutability come from the same validator the interpreter
    // runs before execution, so `olang check` and `olang run` report the
    // same violations in the same words. Names the context modules bring
    // in seed it, so a cross-file binding is not mistaken for undeclared.
    let mut predefined = crate::scoping::Predefined::new();
    for p in context {
        let _ = crate::scoping::validate_program(p, &mut predefined);
    }
    for e in crate::scoping::validate_program(program, &mut predefined) {
        checker.out.push(CheckDiagnostic {
            line: e.line,
            column: e.column,
            message: e.message,
            runtime: false,
            warning: false,
            scope: true,
        });
    }

    CHECK_ALIASES.with(|a| *a.borrow_mut() = None);
    checker.out
}

/// Top-level binding types as display text, for editor hover: function
/// signatures (annotations as written, Unknown parameters left bare) and
/// let bindings (the annotation, or the checker's inferred type when it
/// knows one). Names map to complete hover lines.
pub fn hover_types(program: &Program) -> HashMap<String, String> {
    let mut checker = Checker::default();
    checker.collect(program);
    checker.push_scope();
    for stmt in &program.statements {
        checker.check_statement(stmt, (0, 0));
    }
    let mut out = HashMap::new();
    for (name, sig) in &checker.sigs {
        let params = sig
            .param_names
            .iter()
            .zip(&sig.params)
            .map(|(n, t)| match t {
                SType::Unknown => n.clone(),
                t => format!("{}: {}", n, t.display()),
            })
            .collect::<Vec<_>>()
            .join(", ");
        let ret = match &sig.ret {
            SType::Unknown => String::new(),
            t => format!(" -> {}", t.display()),
        };
        out.insert(name.clone(), format!("fn {}({}){}", name, params, ret));
    }
    if let Some(root) = checker.scopes.first() {
        for (name, ty) in root {
            if out.contains_key(name) {
                continue;
            }
            let line = match ty {
                SType::Unknown => format!("let {}", name),
                t => format!("let {}: {}", name, t.display()),
            };
            out.insert(name.clone(), line);
        }
    }
    out
}

#[derive(Default)]
struct Checker {
    sigs: HashMap<String, FnSig>,
    /// What each parameter of a function of this file must be, as far as
    /// its body shows on its unconditional path (see `Need`).
    needs: HashMap<String, Vec<Option<Need>>>,
    structs: HashMap<String, Vec<(String, SType)>>,
    /// Declared enums: type name → its constructors, in declaration
    /// order, and the reverse map from a constructor to its enum.
    enums: HashMap<String, Vec<String>>,
    variant_owner: HashMap<String, String>,
    scopes: Vec<HashMap<String, SType>>,
    /// Names that leaked out of a popped block, per remaining scope frame
    /// (parallel to `scopes`). The runtime lets a bare block's `let`s
    /// remain visible afterwards; the book says to write as if blocks
    /// scoped, and using a leaked name draws an advisory warning.
    leaked: Vec<std::collections::HashSet<String>>,
    warned_leaks: std::collections::HashSet<String>,
    /// True while walking a block's final statement, which is the block's
    /// value rather than a discarded expression.
    in_tail_position: bool,
    /// Qualified stdlib names documented as returning `Result`, from the
    /// help registry. Populated lazily — most checks never need it.
    fallible: std::collections::HashSet<String>,
    out: Vec<CheckDiagnostic>,
}

impl Checker {
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.leaked.push(std::collections::HashSet::new());
    }

    /// Pop a scope. `merge` says what survives into the parent frame:
    /// blocks leak their own names AND anything already leaked into
    /// them; other constructs (loops, match arms, catch) propagate only
    /// accumulated leaks; function boundaries discard everything (the
    /// runtime leak is frame-local).
    fn pop_scope(&mut self, merge: Merge) {
        let scope = self.scopes.pop().unwrap_or_default();
        let leaks = self.leaked.pop().unwrap_or_default();
        if let (Some(parent), Merge::Block) = (self.leaked.last_mut(), merge) {
            parent.extend(scope.into_keys());
            parent.extend(leaks);
        } else if let (Some(parent), Merge::Construct) = (self.leaked.last_mut(), merge) {
            parent.extend(leaks);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Merge {
    Block,
    Construct,
    Function,
}

impl Checker {
    // ── pass 1: signatures and struct shapes ───────────────────────────

    fn collect(&mut self, program: &Program) {
        for stmt in &program.statements {
            self.collect_statement(stmt.unwrapped());
        }
    }

    fn collect_statement(&mut self, stmt: &Statement) {
        {
            match stmt {
                // `share` wraps a declaration without changing its shape;
                // shared functions and types are signatures like any other
                // (and the cross-file context the LSP feeds is built from
                // exactly these).
                Statement::ShareDecl(sd) => match sd {
                    crate::ast::ShareDecl::Function(f) => {
                        self.collect_statement(&Statement::FunctionDecl(f.clone()))
                    }
                    crate::ast::ShareDecl::Type(t) => {
                        self.collect_statement(&Statement::TypeDecl(t.clone()))
                    }
                    _ => {}
                },
                Statement::FunctionDecl(f) => {
                    let names: Vec<String> = f.parameters.iter().map(|p| p.name.clone()).collect();
                    let needs = parameter_needs(&f.name, &names, &f.body);
                    if needs.iter().any(Option::is_some) {
                        self.needs.insert(f.name.clone(), needs);
                    } else {
                        self.needs.remove(&f.name);
                    }
                    let required = f
                        .parameters
                        .iter()
                        .filter(|p| p.default_value.is_none())
                        .count();
                    self.sigs.insert(
                        f.name.clone(),
                        FnSig {
                            param_names: f.parameters.iter().map(|p| p.name.clone()).collect(),
                            params: f
                                .parameters
                                .iter()
                                .map(|p| {
                                    p.type_annotation
                                        .as_ref()
                                        .map(|a| SType::from_annotation(a, &f.type_params))
                                        .unwrap_or(SType::Unknown)
                                })
                                .collect(),
                            required,
                            total: f.parameters.len(),
                            ret: f
                                .return_type
                                .as_ref()
                                .map(|a| SType::from_annotation(a, &f.type_params))
                                .unwrap_or(SType::Unknown),
                        },
                    );
                }
                Statement::TypeDecl(t) => {
                    if let TypeDefinition::Struct { fields } = &t.definition {
                        self.structs.insert(
                            t.name.clone(),
                            fields
                                .iter()
                                .map(|f| {
                                    (
                                        f.name.clone(),
                                        SType::from_annotation(&f.field_type, &t.type_params),
                                    )
                                })
                                .collect(),
                        );
                    }
                    if let TypeDefinition::Enum { variants } = &t.definition {
                        let names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                        for v in &names {
                            self.variant_owner.insert(v.clone(), t.name.clone());
                        }
                        self.enums.insert(t.name.clone(), names);
                    }
                }
                _ => {}
            }
        }
    }

    // ── scope helpers ──────────────────────────────────────────────────

    fn lookup(&self, name: &str) -> SType {
        for scope in self.scopes.iter().rev() {
            if let Some(t) = scope.get(name) {
                return t.clone();
            }
        }
        SType::Unknown
    }

    fn bind(&mut self, name: &str, ty: SType) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    /// Shadow every name a pattern binds with Unknown, so pattern-bound
    /// variables never inherit an outer binding's type by accident.
    fn bind_pattern_unknown(&mut self, pattern: &crate::ast::Pattern) {
        use crate::ast::Pattern as P;
        match pattern {
            P::Identifier(n) => self.bind(n, SType::Unknown),
            P::List { patterns, rest } => {
                for p in patterns {
                    self.bind_pattern_unknown(p);
                }
                if let Some(r) = rest {
                    let r = r.clone();
                    self.bind(&r, SType::Unknown);
                }
            }
            P::Tuple(ps) => {
                for p in ps {
                    self.bind_pattern_unknown(p);
                }
            }
            P::Ok(p) | P::Err(p) => self.bind_pattern_unknown(p),
            P::EnumVariant { patterns, .. } => {
                for p in patterns {
                    self.bind_pattern_unknown(p);
                }
            }
            P::Struct { field_patterns, .. } | P::AnonymousStruct { field_patterns } => {
                for (_, p) in field_patterns {
                    self.bind_pattern_unknown(p);
                }
            }
            _ => {}
        }
    }

    /// Exhaustiveness over literal-union enums. When the scrutinee's
    /// type is a union of literals (or a single literal), the annotation
    /// admits exactly those values — so every member should be covered
    /// by some arm. An unguarded binding or wildcard covers everything.
    /// Uncovered members are advisory (the program runs until one
    /// arrives); a match that covers NONE of the admissible values fails
    /// on every execution, which is provable.
    fn check_match_exhaustiveness(
        &mut self,
        scrutinee: &SType,
        arms: &[crate::ast::MatchArm],
        span: (u32, u32),
    ) {
        use crate::ast::LitCheck;
        if let SType::Named(type_name) = scrutinee
            && let Some(variants) = self.enums.get(type_name).cloned()
        {
            self.check_enum_exhaustiveness(type_name, &variants, arms, span);
            return;
        }
        let members: Vec<LitCheck> = match scrutinee {
            SType::Lit(l) => vec![l.clone()],
            SType::Union(bs) if bs.iter().all(|b| matches!(b, SType::Lit(_))) => bs
                .iter()
                .filter_map(|b| match b {
                    SType::Lit(l) => Some(l.clone()),
                    _ => None,
                })
                .collect(),
            _ => return,
        };
        if members.is_empty() {
            return;
        }
        let mut covered = vec![false; members.len()];
        for arm in arms {
            // A guard may reject at runtime, so a guarded arm proves no
            // coverage (conservative both for catch-alls and members).
            if arm.guard.is_some() {
                continue;
            }
            if Self::pattern_covers_all(&arm.pattern) {
                return;
            }
            Self::mark_covered(&arm.pattern, &members, &mut covered);
        }
        let missing: Vec<String> = members
            .iter()
            .zip(&covered)
            .filter(|(_, c)| !**c)
            .map(|(m, _)| m.display())
            .collect();
        if missing.is_empty() {
            return;
        }
        if missing.len() == members.len() {
            self.diag(
                span,
                true,
                format!(
                    "this match covers none of the scrutinee's possible values ({}) — it fails on every run",
                    missing.join(", ")
                ),
            );
        } else {
            self.warn(
                span,
                format!(
                    "match is not exhaustive: {} {} no arm — add {} or a catch-all",
                    missing.join(", "),
                    if missing.len() == 1 { "has" } else { "have" },
                    if missing.len() == 1 { "it" } else { "them" },
                ),
            );
        }
    }

    /// A `match` over a value of a declared enum type: every constructor
    /// needs an arm, or a catch-all. A bare name that is one of the
    /// enum's constructors is that constructor (the runtime reads it
    /// so), not a binding; a constructor pattern proves its constructor
    /// covered only when its sub-patterns are irrefutable; a guarded arm
    /// proves nothing. Missing constructors are a warning — a project's
    /// rules promote it — because the scrutinee's type is inferred, and
    /// an inference is a strong hint rather than a proof.
    fn check_enum_exhaustiveness(
        &mut self,
        type_name: &str,
        variants: &[String],
        arms: &[crate::ast::MatchArm],
        span: (u32, u32),
    ) {
        let mut covered = vec![false; variants.len()];
        for arm in arms {
            if arm.guard.is_some() {
                continue;
            }
            if Self::enum_arm_covers(&arm.pattern, variants, &mut covered) {
                return;
            }
        }
        let missing: Vec<&str> = variants
            .iter()
            .zip(&covered)
            .filter(|(_, c)| !**c)
            .map(|(v, _)| v.as_str())
            .collect();
        if missing.is_empty() {
            return;
        }
        self.warn(
            span,
            format!(
                "match over `{}` is not exhaustive: {} {} no arm — add {} or a catch-all",
                type_name,
                missing.join(", "),
                if missing.len() == 1 { "has" } else { "have" },
                if missing.len() == 1 { "it" } else { "them" },
            ),
        );
    }

    /// Mark the constructors this arm covers; true when the arm is a
    /// catch-all (a wildcard, or a binding that is not a constructor).
    fn enum_arm_covers(
        pattern: &crate::ast::Pattern,
        variants: &[String],
        covered: &mut [bool],
    ) -> bool {
        use crate::ast::Pattern as P;
        match pattern {
            P::Wildcard => true,
            P::Identifier(name) => match variants.iter().position(|v| v == name) {
                Some(i) => {
                    covered[i] = true;
                    false
                }
                None => true,
            },
            P::EnumVariant {
                variant_name,
                patterns,
            } => {
                if let Some(i) = variants.iter().position(|v| v == variant_name)
                    && patterns.iter().all(Self::pattern_covers_all)
                {
                    covered[i] = true;
                }
                false
            }
            P::Or { alternatives } => {
                // Every alternative marks what it covers; the arm is a
                // catch-all when any alternative is.
                let mut catch_all = false;
                for alt in alternatives {
                    if Self::enum_arm_covers(alt, variants, covered) {
                        catch_all = true;
                    }
                }
                catch_all
            }
            _ => false,
        }
    }

    /// Does this pattern match every possible value, unconditionally?
    fn pattern_covers_all(pattern: &crate::ast::Pattern) -> bool {
        use crate::ast::Pattern as P;
        match pattern {
            P::Identifier(_) | P::Wildcard => true,
            P::Or { alternatives } => alternatives.iter().any(Self::pattern_covers_all),
            _ => false,
        }
    }

    /// Mark union members this pattern provably matches.
    fn mark_covered(
        pattern: &crate::ast::Pattern,
        members: &[crate::ast::LitCheck],
        covered: &mut [bool],
    ) {
        use crate::ast::{LitCheck, Pattern as P, Value};
        match pattern {
            P::Literal(v) => {
                for (i, m) in members.iter().enumerate() {
                    let hit = match (m, v) {
                        (LitCheck::Int(a), Value::Integer(b)) => a == b,
                        (LitCheck::Str(a), Value::String(b)) => a == b.as_ref(),
                        (LitCheck::Bool(a), Value::Boolean(b)) => a == b,
                        _ => false,
                    };
                    if hit {
                        covered[i] = true;
                    }
                }
            }
            P::Or { alternatives } => {
                for alt in alternatives {
                    Self::mark_covered(alt, members, covered);
                }
            }
            // Guarded, structural, and binding patterns prove nothing
            // here (bindings are handled as catch-alls above).
            _ => {}
        }
    }

    /// A literal key read off a value whose declared shape (`{ a: Int,
    /// b: String }`) does not carry it: a typo the runtime answers with
    /// Unit (or a missing-field error), and a shape the author wrote to
    /// be checked. The shape reaches here from an annotation — on a
    /// parameter, a `let`, or a function's return — whichever file or
    /// macro output wrote it.
    fn check_shape_key(&mut self, object: &Expr, key: &str, via: &str, span: (u32, u32)) {
        let SType::Record(fields) = self.infer(object) else {
            return;
        };
        if fields.iter().any(|(n, _)| n == key) {
            return;
        }
        let shape = SType::Record(fields.clone()).display();
        let nearest = fields
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| edit_distance(n, key) <= 2.max(key.len() / 3))
            .min_by_key(|n| edit_distance(n, key));
        let hint = match nearest {
            Some(n) => format!(" — did you mean `{}`?", n),
            None => String::new(),
        };
        let read = match via {
            "map_get" => "map_get reads Unit for it at runtime".to_string(),
            "map_set" => "map_set adds a key no reader of the shape looks for".to_string(),
            "index" => "the index reads Unit for it at runtime".to_string(),
            _ => "the field is missing at runtime".to_string(),
        };
        self.warn(
            span,
            format!(
                "`{}` is not a key of the declared shape {}: {}{}",
                key, shape, read, hint
            ),
        );
    }

    fn diag(&mut self, span: (u32, u32), runtime: bool, message: String) {
        self.out.push(CheckDiagnostic {
            line: span.0,
            column: span.1,
            message,
            runtime,
            warning: false,
            scope: false,
        });
    }

    /// A loop that copies the collection it is building, once per pass.
    /// `v = v + [x]` and `v = map_set(v, k, x)` extend in place, and so
    /// does `let t = v + [x]; v = t` when `t` is not read again. What
    /// cannot: a temporary that is still read after the rebind holds a
    /// second reference, so the extension copies (O(n) a pass, O(n²) a
    /// loop); and a prepend, `v = [x] + v`, moves every element each time.
    /// Both run correctly and look innocent — 525 ms against 1 ms for
    /// 60,000 elements, measured — so they are said here. Class `copy`.
    fn copies_per_pass(&mut self, body: &Expr, span: (u32, u32)) {
        let Expr::Block(statements) = body else {
            return;
        };
        let extends = |e: &Expr, of: &mut Option<String>| -> bool {
            match e {
                Expr::BinaryOp {
                    left,
                    op: crate::ast::BinaryOp::Add,
                    right,
                } => match (left.as_ref(), right.as_ref()) {
                    (Expr::Identifier(v), Expr::List(_)) => {
                        *of = Some(v.clone());
                        true
                    }
                    _ => false,
                },
                Expr::Call { callee, arguments } => {
                    let named = matches!(callee.as_ref(), Expr::Identifier(n) if n == "map_set");
                    match arguments.first() {
                        Some(Argument::Positional(Expr::Identifier(v))) if named => {
                            *of = Some(v.clone());
                            true
                        }
                        _ => false,
                    }
                }
                _ => false,
            }
        };
        for (i, statement) in statements.iter().enumerate() {
            let at = match statement {
                Statement::Located { line, column, .. } => (*line, *column),
                _ => span,
            };
            match statement.unwrapped() {
                // let t = v + [x] … v = t, with t read in between or after.
                Statement::LetDecl(decl) => {
                    let (crate::ast::Pattern::Identifier(temp), Some(value)) =
                        (&decl.pattern, &decl.value)
                    else {
                        continue;
                    };
                    let mut of = None;
                    if !extends(value, &mut of) {
                        continue;
                    }
                    let collection = of.unwrap_or_default();
                    let rebinds_at = statements[i + 1..].iter().position(|s| {
                        matches!(s.unwrapped(), Statement::Expression(Expr::Assignment { target, value })
                            if *target == collection
                                && matches!(value.as_ref(), Expr::Identifier(n) if n == temp))
                    });
                    let Some(offset) = rebinds_at else { continue };
                    let others: Vec<&Statement> = statements[i + 1..]
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j != offset)
                        .map(|(_, s)| s)
                        .collect();
                    if crate::resolve::mentioned_names(&others)
                        .binary_search(temp)
                        .is_ok()
                    {
                        self.warn(
                            at,
                            format!(
                                "`{collection}` is copied on every pass of this loop: `{temp}` still holds \
                                 the extended value when `{collection} = {temp}` runs, so the extension \
                                 cannot happen in place. Rebind first — `{collection} = ...` — and read \
                                 `{collection}` afterwards"
                            ),
                        );
                    }
                }
                // v = [x] + v
                Statement::Expression(Expr::Assignment { target, value }) => {
                    if let Expr::BinaryOp {
                        left,
                        op: crate::ast::BinaryOp::Add,
                        right,
                    } = value.as_ref()
                        && matches!(left.as_ref(), Expr::List(_))
                        && matches!(right.as_ref(), Expr::Identifier(n) if n == target)
                    {
                        self.warn(
                            at,
                            format!(
                                "`{target} = [..] + {target}` moves every element of `{target}` on each \
                                 pass of this loop: append (`{target} = {target} + [..]`, in place) and \
                                 `reverse` once after it"
                            ),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// A fallible call in statement position drops its failure on the
    /// floor: nothing binds it, matches it, unwraps it, or returns it, so
    /// a write that failed reads exactly like one that succeeded.
    ///
    /// Only *calls* count, and only in statement position — an expression
    /// used for its value is by definition not discarded. `let _ = f(x)`
    /// is the way to say the failure is deliberately ignored, and it does
    /// not trip this because it is a declaration, not an expression
    /// statement.
    fn warn_discarded_result(&mut self, expr: &Expr, span: (u32, u32)) {
        if self.in_tail_position {
            return;
        }
        let Expr::Call { callee, .. } = expr else {
            return;
        };
        // Only module calls (`fs.write_file`) have a documented return
        // type to consult; a user function's is the checker's own business
        // and is handled by the annotation rules.
        let Expr::FieldAccess { object, field } = callee.as_ref() else {
            return;
        };
        let Expr::Identifier(module) = object.as_ref() else {
            return;
        };
        if self.fallible.is_empty() {
            self.fallible = crate::help::HelpSystem::new().result_returning_functions();
        }
        let qualified = format!("{}.{}", module, field);
        if !self.fallible.contains(&qualified) {
            return;
        }
        self.warn(
            span,
            format!(
                "the Result from {qualified} is discarded, so a failure here is invisible. \
                 Bind it, match it, unwrap it to fail loudly, or write `let _ = ...` to say \
                 the failure is deliberately ignored"
            ),
        );
    }

    /// A bundled-collection *write* in statement position throws away
    /// the handle it returns. The collections are values, not objects:
    /// `heap.push(h, x)` computes a new heap and discards it, leaving
    /// `h` exactly as it was — a silent no-op that reads like a
    /// mutation. The rebind (`h = heap.push(h, x)`) is the convention,
    /// and the REPL already hints it; this is the same guidance for
    /// code that never passes through a prompt.
    fn warn_unrebound_collection_write(&mut self, expr: &Expr, span: (u32, u32)) {
        if self.in_tail_position {
            // The handle is the block's value — returned, not dropped.
            return;
        }
        let Expr::Call { callee, .. } = expr else {
            return;
        };
        let Expr::FieldAccess { object, field } = callee.as_ref() else {
            return;
        };
        // `heap.push(...)` after `use collections { heap }`, and the
        // fully qualified `collections.heap.push(...)`.
        let submodule = match object.as_ref() {
            Expr::Identifier(name) => name.as_str(),
            Expr::FieldAccess {
                object: outer,
                field: sub,
            } if matches!(outer.as_ref(), Expr::Identifier(n) if n == "collections") => {
                sub.as_str()
            }
            _ => return,
        };
        if !Self::is_collection_write(submodule, field) {
            return;
        }
        self.warn(
            span,
            format!(
                "{submodule}.{field} returns the updated collection rather than mutating in place, so this call has no effect. Rebind the handle: `h = {submodule}.{field}(h, ...)`"
            ),
        );
    }

    /// The write half of each bundled collection's surface. Reads are
    /// absent on purpose: discarding a read is pointless but harmless,
    /// and warning on it would be noise.
    fn is_collection_write(module: &str, op: &str) -> bool {
        const WRITERS: &[(&str, &[&str])] = &[
            ("heap", &["push", "pop"]),
            (
                "deque",
                &["push_back", "push_front", "pop_back", "pop_front"],
            ),
            ("table", &["put", "remove"]),
            ("dsu", &["union"]),
            ("bitset", &["add", "remove"]),
        ];
        WRITERS
            .iter()
            .any(|(m, ops)| *m == module && ops.contains(&op))
    }

    fn warn(&mut self, span: (u32, u32), message: String) {
        self.out.push(CheckDiagnostic {
            line: span.0,
            column: span.1,
            message,
            runtime: false,
            warning: true,
            scope: false,
        });
    }

    /// Is `name` bound anywhere in scope (any type, including Unknown)?
    fn bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains_key(name)) || self.sigs.contains_key(name)
    }

    // ── inference: conservative, Unknown-biased ────────────────────────

    fn infer(&self, expr: &Expr) -> SType {
        match expr {
            Expr::Integer(_) => SType::Int,
            Expr::Float(_) => SType::Float,
            Expr::String(_) => SType::String,
            Expr::Boolean(_) => SType::Bool,
            Expr::List(elems) => {
                let mut elem: Option<SType> = None;
                for e in elems.iter() {
                    let t = self.infer(e);
                    elem = Some(match elem {
                        Some(prev) => unify(&prev, &t),
                        None => t,
                    });
                }
                SType::List(Box::new(elem.unwrap_or(SType::Unknown)))
            }
            Expr::MapLiteral { entries } => {
                let mut key: Option<SType> = None;
                let mut val: Option<SType> = None;
                for e in entries {
                    let (k, v) = (self.infer(&e.key), self.infer(&e.value));
                    key = Some(match key {
                        Some(prev) => unify(&prev, &k),
                        None => k,
                    });
                    val = Some(match val {
                        Some(prev) => unify(&prev, &v),
                        None => v,
                    });
                }
                SType::Map(
                    Box::new(key.unwrap_or(SType::Unknown)),
                    Box::new(val.unwrap_or(SType::Unknown)),
                )
            }
            Expr::Tuple(elems) => SType::Tuple(elems.iter().map(|e| self.infer(e)).collect()),
            Expr::StructLiteral(lit) => SType::Named(lit.type_name.clone()),
            Expr::ResultOk(e) => SType::Result(Box::new(self.infer(e)), Box::new(SType::Unknown)),
            Expr::ResultErr(e) => SType::Result(Box::new(SType::Unknown), Box::new(self.infer(e))),
            // `expr?` unwraps the Ok payload (or propagates the Err out).
            Expr::Try(e) => match self.infer(e) {
                SType::Result(ok, _) => *ok,
                _ => SType::Unknown,
            },
            Expr::Identifier(name) | Expr::LocalRef { name, .. } => {
                let found = self.lookup(name);
                // A bare constructor of a declared enum (a unit variant)
                // is a value of that enum, unless a binding shadows it.
                if found == SType::Unknown
                    && !self.bound(name)
                    && let Some(owner) = self.variant_owner.get(name)
                {
                    return SType::Named(owner.clone());
                }
                found
            }
            Expr::Call { callee, arguments } => {
                if let Expr::Identifier(name) = callee.as_ref() {
                    if let Some(sig) = self.sigs.get(name) {
                        return sig.ret.clone();
                    }
                    // `Circle(1.0)`: a payload constructor builds its enum.
                    if !self.bound(name)
                        && let Some(owner) = self.variant_owner.get(name)
                    {
                        return SType::Named(owner.clone());
                    }
                    // `map_get(record, "key")` on a declared shape reads
                    // the key's declared type.
                    if name == "map_get"
                        && let [
                            Argument::Positional(object),
                            Argument::Positional(Expr::String(key)),
                        ] = arguments.as_slice()
                        && let SType::Record(fields) = self.infer(object)
                    {
                        return fields
                            .iter()
                            .find(|(n, _)| n.as_str() == key.as_ref())
                            .map(|(_, t)| t.clone())
                            .unwrap_or(SType::Unknown);
                    }
                }
                SType::Unknown
            }
            Expr::FieldAccess { object, field } => match self.infer(object) {
                SType::Record(fields) => fields
                    .iter()
                    .find(|(n, _)| n == field)
                    .map(|(_, t)| t.clone())
                    .unwrap_or(SType::Unknown),
                _ => SType::Unknown,
            },
            Expr::Index { object, index } => match (self.infer(object), index.as_ref()) {
                (SType::Record(fields), Expr::String(key)) => fields
                    .iter()
                    .find(|(n, _)| n.as_str() == key.as_ref())
                    .map(|(_, t)| t.clone())
                    .unwrap_or(SType::Unknown),
                _ => SType::Unknown,
            },
            Expr::If {
                then_branch,
                else_branch: Some(else_branch),
                ..
            } => {
                let (t, e) = (self.infer(then_branch), self.infer(else_branch));
                if t == e { t } else { SType::Unknown }
            }
            Expr::Match { arms, .. } => {
                let mut agreed: Option<SType> = None;
                for arm in arms {
                    let t = self.infer(&arm.expression);
                    match &agreed {
                        None => agreed = Some(t),
                        Some(prev) if *prev == t => {}
                        Some(_) => return SType::Unknown,
                    }
                }
                agreed.unwrap_or(SType::Unknown)
            }
            Expr::BinaryOp { left, op, right } => {
                use crate::ast::BinaryOp as B;
                let (l, r) = (self.infer(left), self.infer(right));
                match op {
                    B::Equal
                    | B::NotEqual
                    | B::LessThan
                    | B::LessThanEqual
                    | B::GreaterThan
                    | B::GreaterThanEqual
                    | B::And
                    | B::Or => SType::Bool,
                    B::Add | B::Subtract | B::Multiply | B::Divide | B::Modulo => {
                        match (l.base_name(), r.base_name()) {
                            (Some("Int"), Some("Int")) => SType::Int,
                            (Some("Float"), Some("Int"))
                            | (Some("Int"), Some("Float"))
                            | (Some("Float"), Some("Float")) => SType::Float,
                            (Some("String"), Some("String")) if matches!(op, B::Add) => {
                                SType::String
                            }
                            _ => SType::Unknown,
                        }
                    }
                }
            }
            Expr::UnaryOp { op, operand } => {
                use crate::ast::UnaryOp as U;
                match (op, self.infer(operand)) {
                    (U::Negate, SType::Int) => SType::Int,
                    (U::Negate, SType::Float) => SType::Float,
                    (U::Not, _) => SType::Bool,
                    _ => SType::Unknown,
                }
            }
            Expr::Lambda {
                parameters,
                return_type,
                ..
            } => SType::Function {
                params: parameters
                    .iter()
                    .map(|p| {
                        p.type_annotation
                            .as_ref()
                            .map(|a| SType::from_annotation(a, &[]))
                            .unwrap_or(SType::Unknown)
                    })
                    .collect(),
                required: parameters
                    .iter()
                    .filter(|p| p.default_value.is_none())
                    .count(),
                ret: Box::new(
                    return_type
                        .as_ref()
                        .map(|a| SType::from_annotation(a, &[]))
                        .unwrap_or(SType::Unknown),
                ),
            },
            // Everything else — blocks, pipelines, field access,
            // indexing, try, ranges — stays Unknown. The runtime enforces;
            // this pass only proves what is cheap to prove.
            _ => SType::Unknown,
        }
    }

    // ── bidirectional checking: literals decompose against the annotation

    /// Check `expr` against a declared type. Literal containers decompose
    /// element-by-element with precise paths (`element 1 of let binding
    /// 'xs'`); everything else falls back to comparing the inferred type.
    fn check_against(&mut self, expr: &Expr, expected: &SType, span: (u32, u32), site: &str) {
        self.check_against_at(expr, expected, span, site, false, false)
    }

    /// `elem`: this position is inside a container decomposition, which the
    /// runtime's shallow checks never see. `irp`: this position is inside a
    /// Result payload — the runtime checks a payload's *base* one level
    /// deep, so the first payload level keeps runtime visibility and
    /// anything nested beyond it does not.
    fn check_against_at(
        &mut self,
        expr: &Expr,
        expected: &SType,
        span: (u32, u32),
        site: &str,
        elem: bool,
        irp: bool,
    ) {
        if *expected == SType::Unknown {
            return;
        }
        match (expr, expected) {
            (Expr::ResultOk(inner), SType::Result(t, _)) => {
                self.check_against_at(
                    inner,
                    t,
                    span,
                    &format!("Ok payload of {}", site),
                    elem || irp,
                    true,
                );
            }
            (Expr::ResultErr(inner), SType::Result(_, t)) => {
                self.check_against_at(
                    inner,
                    t,
                    span,
                    &format!("Err payload of {}", site),
                    elem || irp,
                    true,
                );
            }
            (Expr::List(elems), SType::List(t)) => {
                for (i, e) in elems.iter().enumerate() {
                    self.check_against_at(
                        e,
                        t,
                        span,
                        &format!("element {} of {}", i, site),
                        true,
                        false,
                    );
                }
            }
            (Expr::Tuple(elems), SType::Tuple(ts)) => {
                if elems.len() != ts.len() {
                    self.diag(
                        span,
                        false,
                        format!(
                            "{} expects a {}-element tuple, got {} elements",
                            site,
                            ts.len(),
                            elems.len()
                        ),
                    );
                } else {
                    for (i, (e, t)) in elems.iter().zip(ts).enumerate() {
                        self.check_against_at(
                            e,
                            t,
                            span,
                            &format!("element {} of {}", i, site),
                            true,
                            false,
                        );
                    }
                }
            }
            (Expr::MapLiteral { entries }, SType::Map(k, v)) => {
                for (i, entry) in entries.iter().enumerate() {
                    self.check_against_at(
                        &entry.key,
                        k,
                        span,
                        &format!("map key {} of {}", i, site),
                        true,
                        false,
                    );
                    let value_site = match &entry.key {
                        Expr::String(s) => format!("value for key \"{}\" of {}", s, site),
                        _ => format!("map value {} of {}", i, site),
                    };
                    self.check_against_at(&entry.value, v, span, &value_site, true, false);
                }
            }
            _ => {
                let actual = match expr {
                    Expr::Integer(i) => SType::Lit(crate::ast::LitCheck::Int(*i)),
                    Expr::String(st) => SType::Lit(crate::ast::LitCheck::Str(st.as_ref().clone())),
                    Expr::Boolean(b) => SType::Lit(crate::ast::LitCheck::Bool(*b)),
                    other => self.infer(other),
                };
                if let Some((exp, act, runtime)) = violation(expected, &actual) {
                    self.diag(
                        span,
                        runtime && !elem,
                        format!("{} expects {}, got {}", site, exp, act),
                    );
                }
            }
        }
    }

    // ── pass 2: the walk ───────────────────────────────────────────────

    fn check_statement(&mut self, stmt: &Statement, span: (u32, u32)) {
        match stmt {
            Statement::Located { line, column, stmt } => {
                self.check_statement(stmt, (*line, *column));
            }
            Statement::Expression(e) => {
                self.warn_discarded_result(e, span);
                self.warn_unrebound_collection_write(e, span);
                // Nested constructs start fresh: an expression *inside*
                // this one is not in the enclosing block's tail slot.
                let tail = std::mem::replace(&mut self.in_tail_position, false);
                self.check_expr(e, span);
                self.in_tail_position = tail;
            }
            Statement::LetDecl(decl) => {
                if decl.value.is_none() {
                    // `let pending` binds to Unit until assigned; track the
                    // name so later assignment isn't flagged as undeclared.
                    self.bind_pattern_unknown(&decl.pattern.clone());
                }
                if let Some(value) = &decl.value {
                    self.check_expr(value, span);
                    let annotated = decl
                        .type_annotation
                        .as_ref()
                        .map(|ann| SType::from_annotation(ann, &[]))
                        .unwrap_or(SType::Unknown);
                    let binding = match &decl.pattern {
                        crate::ast::Pattern::Identifier(n) => n.clone(),
                        _ => "value".to_string(),
                    };
                    self.check_against(
                        value,
                        &annotated,
                        span,
                        &format!("let binding '{}'", binding),
                    );
                    // Bind what we know: the annotation when present (a
                    // trusted premise — the runtime enforces its base, and
                    // its elements are the author's promise), else the
                    // inferred type.
                    if let crate::ast::Pattern::Identifier(name) = &decl.pattern {
                        let ty = match &annotated {
                            SType::Unknown => self.infer(value),
                            known => known.clone(),
                        };
                        self.bind(&name.clone(), ty);
                    } else {
                        self.bind_pattern_unknown(&decl.pattern.clone());
                    }
                }
            }
            Statement::FunctionDecl(f) => {
                // Body scope: parameter annotations are trusted (runtime
                // enforces them at every call).
                self.push_scope();
                for p in &f.parameters {
                    let ty = p
                        .type_annotation
                        .as_ref()
                        .map(|a| SType::from_annotation(a, &f.type_params))
                        .unwrap_or(SType::Unknown);
                    self.bind(&p.name.clone(), ty);
                }
                self.check_expr(&f.body, span);
                // Provable return contradiction: only when the body's type
                // is directly known (or the body is a decomposable literal).
                if let Some(ret) = f
                    .return_type
                    .as_ref()
                    .map(|a| SType::from_annotation(a, &f.type_params))
                {
                    self.check_against(&f.body, &ret, span, &format!("return value of {}", f.name));
                }
                self.pop_scope(Merge::Function);
            }
            // `share` wraps a declaration; check what it wraps.
            Statement::ShareDecl(sd) => match sd {
                crate::ast::ShareDecl::Function(f) => {
                    self.check_statement(&Statement::FunctionDecl(f.clone()), span)
                }
                crate::ast::ShareDecl::Let(l) => {
                    self.check_statement(&Statement::LetDecl(l.clone()), span)
                }
                _ => {}
            },
            // Declarations without checkable bodies.
            _ => {}
        }
    }

    /// An argument that cannot be what the call needs: a builtin's first
    /// argument (`fold(5, …)`), or what a function of this program passes
    /// straight on to one, or calls (`total(5)` where `total` folds its
    /// parameter; a one-parameter lambda handed to a function that calls
    /// it with two). Only what is provable: a literal or an inferred type
    /// the runtime refuses on every tier.
    fn check_needs(&mut self, name: &str, arguments: &[Argument], span: (u32, u32)) {
        if self.scopes.iter().any(|scope| scope.contains_key(name)) {
            return;
        }
        let positional = |i: usize| match arguments.get(i) {
            Some(Argument::Positional(e)) => Some(e),
            _ => None,
        };
        if !self.sigs.contains_key(name)
            && let Some(need) = builtin_need(name)
            && let Some(first) = positional(0)
            && let Some(why) = need.refuses(&self.infer(first))
        {
            self.diag(
                span,
                true,
                format!("the first argument of `{name}` is {why}"),
            );
            return;
        }
        let Some(needs) = self.needs.get(name).cloned() else {
            return;
        };
        if !arguments
            .iter()
            .all(|a| matches!(a, Argument::Positional(_)))
        {
            return;
        }
        for (i, need) in needs.iter().enumerate() {
            let (Some(need), Some(argument)) = (need, positional(i)) else {
                continue;
            };
            if let Some(why) = need.refuses(&self.infer(argument)) {
                let via = match need {
                    Need::Callable(..) | Need::Text => String::new(),
                    Need::Sequence(b) | Need::MapLike(b) => {
                        format!(" (`{name}` passes it to `{b}`)")
                    }
                };
                self.diag(
                    span,
                    true,
                    format!("argument {} of `{name}` is {why}{via}", i + 1),
                );
            }
        }
    }

    fn check_expr(&mut self, expr: &Expr, span: (u32, u32)) {
        if let Expr::Call { callee, arguments } = expr
            && let Expr::Identifier(name) = callee.as_ref()
        {
            self.check_needs(name, arguments, span);
        }
        // Check this node, then walk its children.
        if let Expr::Call { callee, arguments } = expr
            && let Expr::Identifier(name) = callee.as_ref()
            // Named arguments change binding order; stay silent on them.
            && arguments
                .iter()
                .all(|a| matches!(a, Argument::Positional(_)))
            // A local of the same name — a parameter, a `let`, a lambda's
            // argument — is what the call means, whatever a function
            // elsewhere is called.
            && !self.scopes.iter().any(|scope| scope.contains_key(name))
            && let Some(sig) = self.sigs.get(name)
        {
            let param_names = sig.param_names.clone();
            let params = sig.params.clone();
            let (required, total) = (sig.required, sig.total);
            if arguments.len() > total {
                self.diag(
                    span,
                    true,
                    format!(
                        "Arity mismatch: expected {}, got {}",
                        total,
                        arguments.len()
                    ),
                );
            } else if arguments.len() < required {
                self.diag(
                    span,
                    true,
                    format!(
                        "Missing required argument: {}",
                        param_names[arguments.len()]
                    ),
                );
            }
            let fn_name = name.clone();
            for (i, a) in arguments.iter().enumerate() {
                if let (Argument::Positional(e), Some(p)) = (a, params.get(i)) {
                    self.check_against(
                        e,
                        p,
                        span,
                        &format!("parameter '{}' of {}", param_names[i], fn_name),
                    );
                }
            }
        }
        // Struct literals check their fields against the declaration —
        // base mismatches mirror the runtime's construction check, element
        // mismatches go deeper.
        if let Expr::StructLiteral(lit) = expr
            && let Some(fields) = self.structs.get(&lit.type_name)
        {
            let fields = fields.clone();
            let type_name = lit.type_name.clone();
            for fv in &lit.fields {
                if let Some((_, ft)) = fields.iter().find(|(n, _)| n == &fv.name) {
                    self.check_against(
                        &fv.value,
                        ft,
                        span,
                        &format!("field '{}' of {}", fv.name, type_name),
                    );
                }
            }
        }
        self.walk_children(expr, span);
    }

    fn walk_children(&mut self, expr: &Expr, span: (u32, u32)) {
        match expr {
            Expr::Call { callee, arguments } => {
                self.check_expr(callee, span);
                for a in arguments {
                    match a {
                        Argument::Positional(e) => self.check_expr(e, span),
                        Argument::Named { value, .. } => self.check_expr(value, span),
                    }
                }
                if let Expr::Identifier(name) = callee.as_ref()
                    && name == "map_get"
                    && !self.bound("map_get")
                    && let [
                        Argument::Positional(object),
                        Argument::Positional(Expr::String(key)),
                    ] = arguments.as_slice()
                {
                    self.check_shape_key(object, key, "map_get", span);
                }
                // A write of a key the shape does not declare is the
                // same typo, made on the other side.
                if let Expr::Identifier(name) = callee.as_ref()
                    && name == "map_set"
                    && !self.bound("map_set")
                    && let [
                        Argument::Positional(object),
                        Argument::Positional(Expr::String(key)),
                        _,
                    ] = arguments.as_slice()
                {
                    self.check_shape_key(object, key, "map_set", span);
                }
            }
            Expr::List(elems) => {
                for e in elems.iter() {
                    self.check_expr(e, span);
                }
            }
            Expr::Tuple(elems) => {
                for e in elems.iter() {
                    self.check_expr(e, span);
                }
            }
            Expr::MapLiteral { entries } => {
                for e in entries {
                    self.check_expr(&e.key, span);
                    self.check_expr(&e.value, span);
                }
            }
            Expr::StructLiteral(lit) => {
                for fv in &lit.fields {
                    self.check_expr(&fv.value, span);
                }
            }
            Expr::BinaryOp { left, op, right } => {
                self.check_expr(left, span);
                self.check_expr(right, span);
                // `==` across two provably different types is an answer the
                // program cannot want: always false (`!=` always true). Int
                // and Float compare numerically, so they are one kind here.
                use crate::ast::BinaryOp as B;
                if matches!(op, B::Equal | B::NotEqual)
                    && let (Some(a), Some(b)) =
                        (self.infer(left).base_name(), self.infer(right).base_name())
                    && a != b
                    && !(matches!(a, "Int" | "Float") && matches!(b, "Int" | "Float"))
                {
                    self.out.push(CheckDiagnostic {
                        line: span.0,
                        column: span.1,
                        message: format!(
                            "`{}` between {} and {} is always {}: values of different types never compare equal",
                            if matches!(op, B::Equal) { "==" } else { "!=" },
                            a,
                            b,
                            if matches!(op, B::Equal) { "false" } else { "true" }
                        ),
                        runtime: false,
                        warning: true,
                        scope: false,
                    });
                }
            }
            Expr::Pipeline { left, right } => {
                self.check_expr(left, span);
                // The pipeline injects `left` as an argument of the
                // right-hand call, so that call's arity can't be judged
                // from its written arguments. Walk into it without the
                // call-level checks.
                self.walk_children(right, span);
            }
            Expr::UnaryOp { operand, .. } => self.check_expr(operand, span),
            Expr::Assignment { target, value } => {
                self.check_expr(value, span);
                // Whether the target is declared, whether it is `mut`, and
                // whether it is captured from an enclosing scope are all
                // decided by the scope validator (src/scoping.rs) — the
                // same one the interpreter runs — so the two never disagree
                // about an assignment.
                let ty = self.infer(value);
                let target = target.clone();
                self.bind(&target, ty);
            }
            Expr::LocalAssign { name, value, .. } => {
                self.check_expr(value, span);
                let ty = self.infer(value);
                let name = name.clone();
                self.bind(&name, ty);
            }
            Expr::Block(stmts) => {
                self.push_scope();
                // A block evaluates to its last statement, so that one is
                // the block's value — and a function body's value is what
                // the function returns. Treating it as a discard would
                // flag `fn parse(b) = { json.parse(b) }`, which returns
                // the Result perfectly well.
                let last = stmts.len().saturating_sub(1);
                for (i, s) in stmts.iter().enumerate() {
                    self.in_tail_position = i == last;
                    self.check_statement(s, span);
                }
                self.in_tail_position = false;
                self.pop_scope(Merge::Block);
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.check_expr(condition, span);
                self.check_expr(then_branch, span);
                if let Some(e) = else_branch {
                    self.check_expr(e, span);
                }
            }
            Expr::ForLoop {
                variable,
                iterable,
                body,
            }
            | Expr::ParForLoop {
                variable,
                iterable,
                body,
            } => {
                self.check_expr(iterable, span);
                self.push_scope();
                self.bind(&variable.clone(), SType::Unknown);
                self.copies_per_pass(body, span);
                self.check_expr(body, span);
                self.pop_scope(Merge::Construct);
            }
            Expr::WhileLoop { condition, body } => {
                self.check_expr(condition, span);
                self.copies_per_pass(body, span);
                self.check_expr(body, span);
            }
            Expr::Loop { body } => {
                self.copies_per_pass(body, span);
                self.check_expr(body, span)
            }
            Expr::Lambda {
                parameters, body, ..
            } => {
                self.push_scope();
                for p in parameters {
                    let ty = p
                        .type_annotation
                        .as_ref()
                        .map(|a| SType::from_annotation(a, &[]))
                        .unwrap_or(SType::Unknown);
                    self.bind(&p.name.clone(), ty);
                }
                self.check_expr(body, span);
                self.pop_scope(Merge::Function);
            }

            Expr::Match { value, arms } => {
                self.check_expr(value, span);
                let scrutinee = self.infer(value);
                self.check_match_exhaustiveness(&scrutinee, arms, span);
                for arm in arms {
                    self.push_scope();
                    self.bind_pattern_unknown(&arm.pattern.clone());
                    // Narrowing: `Ok(x)` / `Err(e)` against a known Result
                    // scrutinee bind their payload types.
                    if let SType::Result(ok, err) = &scrutinee {
                        use crate::ast::Pattern as P;
                        match &arm.pattern {
                            P::Ok(inner) => {
                                if let P::Identifier(n) = inner.as_ref() {
                                    let (n, t) = (n.clone(), (**ok).clone());
                                    self.bind(&n, t);
                                }
                            }
                            P::Err(inner) => {
                                if let P::Identifier(n) = inner.as_ref() {
                                    let (n, t) = (n.clone(), (**err).clone());
                                    self.bind(&n, t);
                                }
                            }
                            _ => {}
                        }
                    }
                    self.check_expr(&arm.expression, span);
                    self.pop_scope(Merge::Construct);
                }
            }
            Expr::Identifier(name) | Expr::LocalRef { name, .. } => {
                // A name that was declared inside a block is gone once the
                // block ends. Reading it is an error at runtime ("undefined
                // variable"); reporting it here names the cause instead,
                // once per name.
                if !self.bound(name)
                    && self.leaked.iter().any(|l| l.contains(name))
                    && self.warned_leaks.insert(name.clone())
                {
                    self.diag(
                        span,
                        false,
                        format!(
                            "'{}' is not in scope here: it is declared inside a block, and a block's bindings end with the block — declare it before the block",
                            name
                        ),
                    );
                }
            }
            Expr::FieldAccess { object, field } => {
                self.check_expr(object, span);
                self.check_shape_key(object, field, "field", span);
            }
            Expr::Index { object, index } => {
                self.check_expr(object, span);
                self.check_expr(index, span);
                if let Expr::String(key) = index.as_ref() {
                    self.check_shape_key(object, key, "index", span);
                }
            }
            Expr::ResultOk(e) | Expr::ResultErr(e) | Expr::Try(e) => self.check_expr(e, span),
            Expr::Range { start, end, .. } => {
                self.check_expr(start, span);
                self.check_expr(end, span);
            }
            // Leaves and unmodeled shapes: nothing to walk (their nested
            // calls, if any, are misses this pass accepts).
            _ => {}
        }
    }
}

/// `olang check --tier [path]`: compile every function of each file under
/// the bytecode tier's rules, without running the program, and list what
/// the tier refuses with the compiler's reason. A refused function runs
/// on the tree-walker — correct, and several times slower — and until now
/// the only way to learn that was a profile of the running program.
///
/// Declarations are evaluated (imports, types, functions, and `let`s of
/// literal values, which a function body may name); other top-level
/// statements are not, so nothing a program does at start happens here.
/// A refusal that names a top-level binding this pass did not evaluate is
/// not reported: whether it resolves is a question for the running
/// program. Refusals are advisories unless the project's `[check] promote`
/// names `tier`.
#[cfg(feature = "native")]
pub fn run_tier(paths: &[PathBuf]) -> i32 {
    let mut files = Vec::new();
    for path in paths {
        if !path.exists() {
            eprintln!("olang check: path not found: {}", path.display());
            return 1;
        }
        files.extend(super::discover_ol_files(path));
    }
    let mut refused_total = 0usize;
    let mut promoted_errors = 0usize;
    for file in &files {
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        let parser = crate::parser::Parser::new();
        let Ok(program) = parser.parse_with_dir(&source, file.parent()) else {
            continue; // `olang check` proper reports parse errors
        };
        let absolute = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        let mut interpreter = crate::interpreter::Interpreter::new();
        interpreter.enable_bytecode_tier(1, std::env::var_os("OLANG_TIER_VERBOSE").is_some());
        interpreter.set_current_file(&absolute);
        if let Some(root) = crate::pkg::manifest::Manifest::find_root(&absolute)
            && let Ok(map) = crate::pkg::install(&root, &crate::pkg::InstallOptions::default())
        {
            interpreter.set_dependency_map(map.into_iter().collect());
        }
        let mut skipped: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut declarations = Vec::new();
        for statement in &program.statements {
            use crate::ast::ShareDecl as S;
            let keep = match statement.unwrapped() {
                Statement::UseDecl(_)
                | Statement::TypeDecl(_)
                | Statement::ErrorTypeDecl(_)
                | Statement::FunctionDecl(_)
                | Statement::TraitDecl(_)
                | Statement::ImplDecl(_) => true,
                Statement::ShareDecl(S::Let(l)) | Statement::LetDecl(l) => {
                    let literal = l.value.as_ref().is_none_or(crate::expand::is_literal_expr);
                    if !literal && let crate::ast::Pattern::Identifier(name) = &l.pattern {
                        skipped.insert(name.clone());
                    }
                    literal
                }
                Statement::ShareDecl(_) => true,
                _ => false,
            };
            if keep {
                declarations.push(statement.clone());
            }
        }
        if interpreter
            .eval_program(Program {
                statements: declarations,
            })
            .is_err()
        {
            continue;
        }
        let file_key = absolute.to_string_lossy().to_string();
        interpreter.note_root_as_module_scope(&file_key);
        let refusals: Vec<(String, String)> = interpreter
            .tier_compile_ahead(Some(&file_key))
            .into_iter()
            .filter(|(_, reason)| {
                !skipped
                    .iter()
                    .any(|name| reason.contains(&format!("'{name}'")))
            })
            .collect();
        if refusals.is_empty() {
            continue;
        }
        let promoted = promotions_for(file.parent())
            .iter()
            .any(|p| p == "tier" || p == "all");
        println!("{}", file.display());
        for (name, reason) in &refusals {
            println!(
                "  {} `{}` stays on the tree-walker: {}",
                if promoted { "×" } else { "⚠" },
                name,
                reason
            );
        }
        refused_total += refusals.len();
        if promoted {
            promoted_errors += refusals.len();
        }
    }
    if refused_total == 0 {
        println!(
            "olang check --tier: {} file{}, every function compiles",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        );
    } else {
        println!(
            "olang check --tier: {} function{} refused by the bytecode tier{}",
            refused_total,
            if refused_total == 1 { "" } else { "s" },
            if promoted_errors > 0 {
                " (an error here: [check] promote names `tier`)"
            } else {
                ""
            }
        );
    }
    if promoted_errors > 0 { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn check(src: &str) -> Vec<CheckDiagnostic> {
        check_program(&Parser::new().parse(src).expect("parses"))
    }

    #[test]
    fn an_unrebound_collection_write_warns() {
        // The handle is the value; a bare write computes a new
        // collection and drops it, leaving the binding untouched.
        let d = check(
            "use collections { heap }\nlet mut h = heap.new()\nheap.push(h, 3, \"x\")\nprintln(heap.size(h))\n",
        );
        assert_eq!(d.len(), 1, "{:?}", d);
        assert!(d[0].warning, "the program still runs — advisory");
        assert!(d[0].message.contains("has no effect"), "{}", d[0].message);
        assert!(
            d[0].message.contains("h = heap.push(h, ...)"),
            "the fix must be spelled out: {}",
            d[0].message
        );
    }

    #[test]
    fn the_fully_qualified_write_warns_too() {
        let d = check(
            "let mut t = collections.table.new()\ncollections.table.put(t, \"k\", 1)\nprintln(collections.table.size(t))\n",
        );
        assert_eq!(d.len(), 1, "{:?}", d);
        assert!(d[0].message.contains("table.put"), "{}", d[0].message);
    }

    #[test]
    fn rebound_writes_reads_and_tail_positions_stay_silent() {
        // Rebound: the convention, and the whole point.
        assert!(
            check("use collections { heap }\nlet mut h = heap.new()\nh = heap.push(h, 1, \"a\")\nprintln(heap.size(h))\n").is_empty()
        );
        // A read discarded is pointless but harmless — not this rule's business.
        assert!(check("use collections { heap }\nlet h = heap.new()\nheap.size(h)\n").is_empty());
        // Tail position: the new handle *is* the function's value.
        assert!(
            check("use collections { heap }\nfn seeded() = {\n    let h = heap.new()\n    heap.push(h, 1, \"a\")\n}\nprintln(heap.size(seeded()))\n").is_empty()
        );
        // A same-named user function is not the bundled module.
        assert!(check("fn put(a, b) = a + b\nlet table = 1\nput(table, 2)\n").is_empty());
    }

    #[test]
    fn nonexhaustive_enum_match_warns_with_missing_members() {
        let d = check(
            "fn advance(s: \"open\" | \"active\" | \"done\") = match s {\n    \"open\" => 1,\n    \"active\" => 2\n}\n",
        );
        assert_eq!(d.len(), 1);
        assert!(d[0].warning, "missing members are advisory");
        assert!(d[0].message.contains("\"done\""));
        assert!(d[0].message.contains("not exhaustive"));
    }

    #[test]
    fn exhaustive_enum_matches_are_silent() {
        assert!(
            check("fn f(s: \"a\" | \"b\") = match s {\n    \"a\" => 1,\n    \"b\" => 2\n}\n")
                .is_empty()
        );
        // a catch-all binding covers everything
        assert!(
            check("fn f(s: \"a\" | \"b\") = match s {\n    \"a\" => 1,\n    other => 2\n}\n")
                .is_empty()
        );
        // or-patterns count member by member
        assert!(
            check("fn f(n: 1 | 2 | 3) = match n {\n    1 | 2 => \"low\",\n    3 => \"high\"\n}\n")
                .is_empty()
        );
    }

    #[test]
    fn match_covering_no_member_is_a_runtime_error() {
        let d = check("fn f(s: \"a\" | \"b\") = match s {\n    \"x\" => 1,\n    \"y\" => 2\n}\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].runtime, "covering nothing fails on every run");
        assert!(!d[0].warning);
        assert!(d[0].message.contains("covers none"));
    }

    #[test]
    fn guarded_arms_prove_no_coverage() {
        let d =
            check("fn f(n: 1 | 2) = match n {\n    1 => \"one\",\n    2 if true => \"two\"\n}\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].warning);
        assert!(d[0].message.contains('2'));
        // ...including guarded catch-alls
        let d =
            check("fn f(n: 1 | 2) = match n {\n    1 => \"one\",\n    x if x > 0 => \"pos\"\n}\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].warning);
    }

    #[test]
    fn int_and_bool_literal_enums_check_too() {
        let d = check("fn f(n: 1 | 2 | 3) = match n {\n    1 => \"one\"\n}\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains('2') && d[0].message.contains('3'));
        assert!(
            check("fn f(b: true | false) = match b {\n    true => 1,\n    false => 0\n}\n")
                .is_empty()
        );
    }

    #[test]
    fn dynamic_scrutinees_are_never_judged_for_exhaustiveness() {
        assert!(check("fn f(s) = match s {\n    \"a\" => 1\n}\n").is_empty());
        assert!(check("fn f(s: String) = match s {\n    \"a\" => 1\n}\n").is_empty());
    }

    #[test]
    fn provable_param_mismatch_is_reported_with_position() {
        let d = check("fn f(x: Int) = x\nf(\"hello\")\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].line, 2);
        assert_eq!(d[0].message, "parameter 'x' of f expects Int, got String");
        assert!(d[0].runtime);
    }

    #[test]
    fn dynamic_code_is_never_judged() {
        assert!(check("fn f(x) = x\nf(\"anything\")\nf(1)\n").is_empty());
        assert!(check("fn f(x: Int) = x\nfn g(y) = f(y)\n").is_empty());
    }

    #[test]
    fn annotated_let_flows_into_calls() {
        let d = check("fn f(x: Int) = x\nlet s: String = \"hi\"\nf(s)\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("expects Int, got String"));
    }

    #[test]
    fn known_return_types_flow() {
        let d = check("fn mk() -> String = \"s\"\nfn f(x: Int) = x\nf(mk())\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("expects Int, got String"));
    }

    #[test]
    fn let_annotation_mismatch_and_return_contradiction() {
        let d = check("let x: Int = \"nope\"\n");
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message
                .contains("let binding 'x' expects Int, got String")
        );
        let d = check("fn g(x: Int) -> String = x * 2\n");
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message
                .contains("return value of g expects String, got Int")
        );
    }

    #[test]
    fn arity_is_checked_statically_for_known_functions() {
        let d = check("fn f(a, b) = a\nf(1, 2, 3)\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("Arity mismatch: expected 2, got 3"));
        let d = check("fn f(a, b) = a\nf(1)\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("Missing required argument: b"));
    }

    #[test]
    fn pipeline_rhs_calls_are_not_arity_checked() {
        // `xs |> take(n)` injects xs as an argument; the written argument
        // list is intentionally one short.
        assert!(check("fn take(xs, n) = xs\nlet r = [1] |> take(2)\n").is_empty());
        // But provable violations nested *inside* the rhs still surface.
        let d = check("fn take(xs, n) = xs\nfn f(x: Int) = x\nlet r = [1] |> take(f(\"s\"))\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("expects Int, got String"));
    }

    #[test]
    fn strict_float_and_shallow_containers() {
        let d = check("fn f(x: Float) = x\nf(1)\n");
        assert_eq!(d.len(), 1, "Int does not satisfy Float");
        assert!(check("fn f(xs: List) = xs\nf([1, 2])\n").is_empty());
        let d = check("fn f(xs: List) = xs\nf(42)\n");
        assert_eq!(d.len(), 1);
    }

    // ── stage 3: element types ─────────────────────────────────────────

    #[test]
    fn list_literal_elements_are_checked_with_index_paths() {
        let d = check("let xs: List<Int> = [1, \"a\", 3]\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "element 1 of let binding 'xs' expects Int, got String"
        );
        assert!(!d[0].runtime, "the runtime's shallow check would pass this");
    }

    #[test]
    fn nested_list_elements_compose_paths() {
        let d = check("let xs: List<List<Int>> = [[1], [2, \"a\"]]\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "element 1 of element 1 of let binding 'xs' expects Int, got String"
        );
    }

    #[test]
    fn empty_and_unknown_lists_stay_silent() {
        assert!(check("let xs: List<Int> = []\n").is_empty());
        assert!(check("fn mk(n) = [n]\nlet xs: List<Int> = mk(1)\n").is_empty());
        // A generic parameter erases the element promise, not the base.
        assert!(check("fn f<T>(xs: List<T>) = xs\nf([1, \"mixed\"])\n").is_empty());
        let d = check("fn f<T>(xs: List<T>) = xs\nf(42)\n");
        assert_eq!(d.len(), 1, "the base is still checked");
    }

    #[test]
    fn map_and_tuple_literals_decompose() {
        let d = check("let m: Map<String, Int> = #{ \"a\": 1, \"b\": \"two\" }\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "value for key \"b\" of let binding 'm' expects Int, got String"
        );
        let d = check("let p: (Int, String) = (1, 2)\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "element 1 of let binding 'p' expects String, got Int"
        );
        let d = check("let p: (Int, String) = (1, \"a\", 3)\n");
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message
                .contains("expects a 2-element tuple, got 3 elements")
        );
    }

    #[test]
    fn deep_types_flow_through_annotated_bindings() {
        let d = check("fn f(xs: List<Int>) = xs\nlet ys: List<String> = [\"a\"]\nf(ys)\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "parameter 'xs' of f expects List<Int>, got List<String>"
        );
        assert!(!d[0].runtime);
    }

    #[test]
    fn list_literal_arguments_decompose_at_call_sites() {
        let d = check("fn f(xs: List<Int>) = xs\nf([1, \"a\"])\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "element 1 of parameter 'xs' of f expects Int, got String"
        );
    }

    #[test]
    fn struct_literal_fields_are_checked_deeply() {
        let src = "type P = struct { x: Int, tags: List<String> }\n";
        let d = check(&format!("{}let p = P {{ x: \"no\", tags: [] }}\n", src));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].message, "field 'x' of P expects Int, got String");
        assert!(
            d[0].runtime,
            "the runtime's construction check catches this"
        );
        let d = check(&format!("{}let p = P {{ x: 1, tags: [\"a\", 2] }}\n", src));
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "element 1 of field 'tags' of P expects String, got Int"
        );
        assert!(!d[0].runtime);
    }

    #[test]
    fn pattern_bindings_shadow_outer_types() {
        // The loop/lambda/match variable must not inherit the outer
        // binding's annotated type.
        assert!(check("fn f(x: Int) = x\nlet x: Int = 1\nfor x in [\"a\"] { f(x) }\n").is_empty());
        assert!(check("fn f(x: Int) = x\nlet x: Int = 1\nlet g = (x) => f(x)\n").is_empty());
    }

    #[test]
    fn branch_agreement_infers_through_if_and_match() {
        let d = check("fn f(x: Int) = x\nlet v = if true => \"a\" else => \"b\"\nf(v)\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("expects Int, got String"));
        // Disagreeing branches stay unknown.
        assert!(check("fn f(x: Int) = x\nlet v = if true => \"a\" else => 1\nf(v)\n").is_empty());
    }

    // ── 0.61: `let` is required, and `mut` is a guarantee ──────────────

    #[test]
    fn assignment_without_a_declaration_is_an_error() {
        let d = check("count = 1\n");
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(!d[0].warning, "an error, not an advisory");
        assert!(d[0].message.contains("cannot assign to 'count'"));
        assert!(d[0].message.contains("not declared in this scope"));
        // A `mut` declaration authorizes assignment; a valueless one too.
        assert!(check("let mut ok = 1\nok = 2\n").is_empty());
        assert!(check("let mut pending\npending = 1\n").is_empty());
    }

    #[test]
    fn assigning_a_binding_that_is_not_mut_is_an_error() {
        let d = check("let n = 1\nn = 2\n");
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(!d[0].warning);
        assert!(d[0].message.contains("not declared mutable"));
        // Parameters rebind (they are the function's own locals — the
        // collections' `h = heap.push(h, ...)` convention); loop
        // variables remain immutable.
        assert!(check("fn f(x) = { x = x + 1\n x }\n").is_empty());
        let d = check("for x in [1] { x = x + 1 }\n");
        assert_eq!(d.len(), 1, "{d:?}");
        // Shadowing with a fresh `let` needs no `mut` — the pipeline idiom.
        assert!(check("let t = \" a \"\nlet t = str.trim(t)\nprintln(t)\n").is_empty());
    }

    #[test]
    fn a_rejected_assignment_still_reports_the_type_violation() {
        // The scope error and the type error are independent findings.
        let d = check("fn f(x: Int) = x\ns = \"str\"\nf(s)\n");
        assert_eq!(d.len(), 2, "scope error plus type violation: {:?}", d);
        assert!(d.iter().any(|x| x.message.contains("cannot assign to 's'")));
        assert!(
            d.iter()
                .any(|x| !x.warning && x.message.contains("expects Int, got String"))
        );
    }

    // ── 0.50 arc: cross-file context ───────────────────────────────────

    #[test]
    fn shared_declarations_are_checked_and_seed_context() {
        // `share fn` signatures are signatures (previously invisible).
        let d = check("share fn helper(x: Int) -> Int = x + 1\nhelper(\"wrong\")\n");
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message
                .contains("parameter 'x' of helper expects Int, got String")
        );
        // Shared bodies are walked too.
        let d = check("share fn bad(x: Int) -> String = x * 2\n");
        assert_eq!(d.len(), 1);
        // Context programs contribute signatures without being walked.
        let module = Parser::new()
            .parse("share fn area(w: Int, h: Int) -> Int = w * h\n")
            .expect("parses");
        let main = Parser::new().parse("area(\"bad\", 4)\n").expect("parses");
        let d = check_program_with_context(&[&module], &main);
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message
                .contains("parameter 'w' of area expects Int, got String")
        );
    }

    // ── 0.61: blocks scope their bindings ──────────────────────────────

    #[test]
    fn using_a_block_scoped_binding_after_the_block_is_an_error() {
        let d = check(
            "{\n    let inner = 1\n}\nprintln(to_string(inner))\nprintln(to_string(inner))\n",
        );
        assert_eq!(d.len(), 1, "once per name: {:?}", d);
        assert!(!d[0].warning, "an error, not an advisory");
        assert!(d[0].message.contains("'inner' is not in scope here"));
        // Assigning to one reports it as undeclared — which it is, once the
        // block has ended.
        let d = check("{\n    let n = 1\n}\nn = 2\n");
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].message.contains("cannot assign to 'n'"));
    }

    #[test]
    fn assigning_a_captured_binding_inside_a_closure_is_an_error() {
        // The classic footgun: capture is by value, so the write is dead.
        // An error, not a warning, since 0.62 — `cell` is the alternative
        // the message points at.
        let d = check("let mut c = 0\nlet inc = () => { c = c + 1; c }\ninc()\n");
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(!d[0].warning);
        assert!(d[0].scope);
        assert!(d[0].message.contains("captured from an enclosing scope"));
        assert!(d[0].message.contains("cell"));
        // A named nested function captures by value too.
        let d = check("let mut g = 0\nfn f() = { g = 5; g }\nf()\n");
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(!d[0].warning);
        // Holding the state in a cell is the fix, and it checks clean.
        assert!(
            check("let c = cell(0)\nlet inc = () => cell.update(c, (n) => n + 1)\ninc()\n")
                .is_empty()
        );
    }

    #[test]
    fn capture_lint_has_no_false_positives() {
        // A local declared inside the closure is live.
        assert!(check("let f = () => { let mut n = 0; n = n + 1; n }\nf()\n").is_empty());
        // A mutable shadow of a parameter is inside the boundary, so
        // writing it is live.
        assert!(check("let g = (x) => { let mut x = x; x = x + 1; x }\ng(1)\n").is_empty());
        // Top-level reassignment captures nothing.
        assert!(check("let mut x = 0\nx = 5\nprintln(to_string(x))\n").is_empty());
        // Shadowing the captured name with a local `let` first is fine.
        assert!(
            check("let mut c = 0\nfn f() = { let mut c = 0\n    c = c + 1\n    c }\nf()\n")
                .is_empty()
        );
        // Reading a captured binding (not assigning) is always fine.
        assert!(check("let base = 10\nlet add = (x) => x + base\nadd(5)\n").is_empty());
    }

    #[test]
    fn block_scopes_stop_at_function_boundaries() {
        // A block inside one function must not taint another.
        assert!(check("fn a() = {\n    { let x = 1 }\n    0\n}\nfn b(x) = x\nb(1)\n").is_empty());
        // Declaring before the block is the fix, and assigning an outer
        // `mut` binding from inside a block stays legal.
        assert!(
            check("let mut outer = 0\n{\n    outer = 1\n}\nprintln(to_string(outer))\n").is_empty()
        );
        // A binding from a nested block is out of scope outside both.
        let d = check("{\n    { let deep = 1 }\n}\nprintln(to_string(deep))\n");
        assert_eq!(d.len(), 1, "{d:?}");
    }

    // ── 0.50 arc: literal types ────────────────────────────────────────

    #[test]
    fn literal_unions_are_lightweight_enums() {
        let src = "fn set(s: \"open\" | \"done\") = s\n";
        assert!(check(&format!("{}set(\"open\")\nset(\"done\")\n", src)).is_empty());
        let d = check(&format!("{}set(\"nope\")\n", src));
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "parameter 's' of set expects \"open\" | \"done\", got \"nope\""
        );
        assert!(d[0].runtime);
        // A non-literal same-base value stays silent — not provable.
        assert!(check(&format!("{}fn dyn_s(x) = x\nset(dyn_s(1))\n", src)).is_empty());
        // A different base is provable even without a known value.
        let d = check(&format!("{}set(42)\n", src));
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn scalar_literal_annotations_check_by_value() {
        assert!(check("let five: 5 = 5\n").is_empty());
        let d = check("let five: 5 = 6\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("expects 5, got 6"));
        let d = check("let flag: true = false\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("expects true, got false"));
        // Literal-annotated bindings carry their literal type onward.
        let d = check(
            "fn set(s: \"open\" | \"done\") = s\nlet st: \"open\" = \"open\"\nset(st)\nlet bad: \"x\" = \"x\"\nset(bad)\n",
        );
        assert_eq!(d.len(), 1, "st satisfies, bad provably violates: {:?}", d);
    }

    // ── 0.50 arc: hover types ──────────────────────────────────────────

    #[test]
    fn hover_types_render_signatures_and_inferred_lets() {
        let program = Parser::new()
            .parse(
                "fn dist(a: Float, b: Float) -> Float = a + b\nfn dyn_fn(x) = x\nlet total = 1 + 2\nlet xs: List<Int> = [1]\nlet mystery = dyn_fn(1)\n",
            )
            .expect("parses");
        let h = hover_types(&program);
        assert_eq!(h["dist"], "fn dist(a: Float, b: Float) -> Float");
        assert_eq!(h["dyn_fn"], "fn dyn_fn(x)");
        assert_eq!(h["total"], "let total: Int");
        assert_eq!(h["xs"], "let xs: List<Int>");
        assert_eq!(h["mystery"], "let mystery");
    }

    // ── 0.50 arc: Promise at non-async sites ───────────────────────────

    #[test]
    fn a_task_handle_is_an_ordinary_value_to_the_checker() {
        // 0.63 deleted async/await/Promise and 0.65 deleted try/catch;
        // nothing about concurrency or error handling needs a special
        // case in the checker any more. A task handle has no type of its
        // own here, and no unwrapping rule to get wrong.
        assert!(check("fn go() = 1\nlet t = spawn go()\nlet v = task.join(t)\n").is_empty());
        // And the freed words are just names.
        assert!(
            check("let async = 1\nlet await = 2\nlet try = 3\nasync + await + try\n").is_empty()
        );
    }

    // ── stage 4: Result payloads ───────────────────────────────────────

    #[test]
    fn result_literal_payloads_decompose() {
        let d = check("fn f(r: Result<Int, String>) = r\nf(Ok(\"s\"))\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "Ok payload of parameter 'r' of f expects Int, got String"
        );
        assert!(d[0].runtime, "the runtime checks one payload level");
        let d = check("fn f(r: Result<Int, String>) = r\nf(Err(42))\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "Err payload of parameter 'r' of f expects String, got Int"
        );
        assert!(d[0].runtime);
        // Honest payloads are silent.
        assert!(check("fn f(r: Result<Int, String>) = r\nf(Ok(1))\nf(Err(\"e\"))\n").is_empty());
    }

    #[test]
    fn nested_result_payloads_are_beyond_the_runtime() {
        let d = check("let r: Result<Result<Int, String>, String> = Ok(Ok(\"s\"))\n");
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message
                .contains("Ok payload of Ok payload of let binding 'r' expects Int, got String")
        );
        assert!(
            !d[0].runtime,
            "the runtime checks payload bases one level deep"
        );
    }

    #[test]
    fn try_operator_unwraps_the_ok_payload() {
        let d = check(
            "fn get() -> Result<Int, String> = Ok(1)\nfn g(x: String) = x\nfn h() -> Result<Int, String> = {\n    let v = get()?\n    g(v)\n    Ok(v)\n}\n",
        );
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message
                .contains("parameter 'x' of g expects String, got Int")
        );
    }

    #[test]
    fn match_arms_narrow_result_payloads() {
        let d = check(
            "fn get() -> Result<Int, String> = Ok(1)\nfn g(x: String) = x\nlet r = match get() { Ok(v) => g(v), Err(e) => g(e) }\n",
        );
        assert_eq!(d.len(), 1, "Ok arm flagged, Err arm honest: {:?}", d);
        assert!(d[0].message.contains("expects String, got Int"));
    }

    // ── 0.50 arc: unions ───────────────────────────────────────────────

    #[test]
    fn union_annotations_accept_any_branch_and_reject_all_branch_misses() {
        let src = "fn f(x: Int | String) = x\n";
        assert!(check(&format!("{}f(1)\nf(\"s\")\n", src)).is_empty());
        let d = check(&format!("{}f(true)\n", src));
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message, "parameter 'x' of f expects Int | String, got true",
            "scalar actuals name their value, matching the runtime"
        );
        assert!(d[0].runtime, "every branch fails at base level");
    }

    #[test]
    fn union_deep_branch_failures_are_promise_breaks() {
        // The runtime's shallow check admits any List; only the checker
        // can prove the union is still violated.
        let d = check("fn f(x: List<Int> | Int) = x\nlet ys: List<String> = [\"a\"]\nf(ys)\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "parameter 'x' of f expects List<Int> | Int, got List<String>"
        );
        assert!(!d[0].runtime);
        // A list of Ints satisfies the List<Int> branch.
        assert!(check("fn f(x: List<Int> | Int) = x\nlet ys: List<Int> = [1]\nf(ys)\n").is_empty());
    }

    #[test]
    fn union_with_unenforceable_branch_is_silent() {
        // A generic-parameter branch erases the whole union, exactly as
        // the runtime skips it.
        assert!(check("fn f<T>(x: T | Int) = x\nf(true)\n").is_empty());
    }

    // ── 0.50 arc: function types ───────────────────────────────────────

    #[test]
    fn function_annotations_check_callability_and_arity() {
        let src = "fn apply(f: (Int) -> Int, x: Int) = f(x)\n";
        assert!(check(&format!("{}apply((n) => n + 1, 1)\n", src)).is_empty());
        let d = check(&format!("{}apply(7, 1)\n", src));
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "parameter 'f' of apply expects (Int) -> Int, got Int"
        );
        assert!(d[0].runtime);
        let d = check(&format!("{}apply((a, b) => a, 1)\n", src));
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "parameter 'f' of apply expects (Int) -> Int, got a function taking 2 parameters"
        );
        assert!(d[0].runtime, "arity is value-exposed; the runtime rejects");
    }

    #[test]
    fn function_signature_types_are_checker_territory() {
        // An annotated lambda parameter contradicting the declared
        // signature is provable but runs shallow at runtime.
        let d = check("fn apply(f: (Int) -> Int, x: Int) = f(x)\napply((s: String) => 1, 1)\n");
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "parameter 'f' of apply expects (Int) -> Int, got (String) -> ?"
        );
        assert!(!d[0].runtime);
        // Unannotated lambda params stay unknown — silent.
        assert!(check("fn apply(f: (Int) -> Int, x: Int) = f(x)\napply((s) => 1, 1)\n").is_empty());
    }

    #[test]
    fn deep_result_types_flow_through_bindings() {
        let d = check(
            "fn f(r: Result<Int, String>) = r\nfn mk() -> Result<String, String> = Ok(\"s\")\nlet r = mk()\nf(r)\n",
        );
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "parameter 'r' of f expects Result<Int, String>, got Result<String, String>"
        );
        assert!(
            !d[0].runtime,
            "the value may be Err at runtime — not provable"
        );
        // Unknown-producing calls stay silent.
        assert!(check("fn f(r: Result<Int, String>) = r\nfn mk(x) = Ok(x)\nf(mk(1))\n").is_empty());
    }
}

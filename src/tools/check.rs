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

/// Every finding `olang check` makes on one file — `program` parsed from
/// `source` (macros already expanded), `dir` its directory — with the
/// project's promotions applied. The language server publishes exactly
/// this list, so the editor and the checker never disagree.
/// `unresolved` is `Analyzer::unresolved_names(program)`, taken by the
/// caller so a server that walks the file once for its own reasons does
/// not walk it twice.
pub fn file_diagnostics(
    program: &Program,
    source: &str,
    dir: Option<&Path>,
    unresolved: Vec<String>,
) -> Vec<CheckDiagnostic> {
    // the file's own tree says what it imports: parsing `source` again
    // for its `use` lines doubled the cost of a large file
    let modules = match dir {
        Some(d) => modules_used_by(program, d, false),
        None => Vec::new(),
    };
    let context: Vec<&Program> = modules.iter().map(|(_, _, p)| p).collect();
    let mut diagnostics = check_program_with_context(&context, program);
    diagnostics.extend(use_shadow_warnings(program, dir));
    diagnostics.extend(unimported_names(program, source, &modules, dir, unresolved));
    diagnostics.extend(template_escape_warnings(source));
    diagnostics.extend(shadow_warnings_in(program, source));
    diagnostics.extend(binding_shadow_warnings(program));
    diagnostics.extend(key_warnings(program, source));
    // A line's `// allow: <class>` silences its advisories of the class
    // (never an error), before `[check] promote` makes any an error: an
    // allow is explicit and local, so it wins.
    let allows = crate::tools::allow::scan(source);
    if !allows.is_empty() {
        diagnostics.retain(|d| !allows.silences(d));
    }
    // The project's `[check] promote`: the advisory classes it names
    // are errors here, so an exhaustiveness or shape finding gates
    // without a rules file.
    let promoted = promotions_for(dir);
    for d in diagnostics.iter_mut() {
        promote(d, &promoted);
    }
    // an allow naming no class: said, never silenced or promoted
    diagnostics.extend(allows.problems());
    diagnostics
}

/// `olang check [paths] [--rules FILE]` — parse every `.ol` file and report
/// provable annotation violations. With `--rules`, also run project-authored
/// lints written in olang over the meta AST. Exit 0 when everything is clean
/// (or unknowable), 1 when a violation, parse error, or rule finding appears.
/// `olang check --format json`: every file's problems as the language
/// server publishes them (`lsp::file_diagnostics`: the same checker, the
/// same rules, the unused names as hints tagged `unnecessary`), one
/// document on stdout. Each diagnostic says its severity (`error`,
/// `warning`, `information`, `hint`), its rule (`code`), where it is —
/// `line`/`column` to `end_line`/`end_column`, 1-based, in characters,
/// and `range` as the server's (0-based, UTF-16) — its message and its
/// tags. Exit 1 when any is an error, as the text form.
#[cfg(feature = "native")]
pub fn run_json(paths: &[PathBuf]) -> i32 {
    use lsp_types::{DiagnosticSeverity as S, DiagnosticTag as T, NumberOrString};
    crate::expand::keep_module_parses(true);
    let mut files = Vec::new();
    for path in paths {
        if !path.exists() {
            eprintln!("olang check: path not found: {}", path.display());
            return 2;
        }
        files.extend(super::discover_sources(path));
    }
    let (mut errors, mut warnings) = (0usize, 0usize);
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let mut roots = std::collections::BTreeSet::new();
    for path in paths {
        if let Some(root) = std::path::absolute(path)
            .ok()
            .and_then(|p| crate::pkg::manifest::Manifest::find_root(&p))
        {
            roots.insert(root);
        }
    }
    for root in &roots {
        // a manifest that declares capabilities and cannot be read: the
        // program would refuse to run, so the check fails with it
        if let Err(e) = crate::pkg::manifest::Manifest::load_for_capabilities(root) {
            errors += 1;
            rows.push(serde_json::json!({
                "file": root.join("olang.toml").to_string_lossy(),
                "diagnostics": [{
                    "severity": "error", "code": "manifest", "message": e.to_string(), "tags": [],
                    "line": 1, "column": 1, "end_line": 1, "end_column": 1,
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                }],
            }));
        }
        if let Some(problem) = crate::pkg::manifest::Manifest::load(root)
            .ok()
            .and_then(|m| m.olang_value_problem())
        {
            errors += 1;
            rows.push(serde_json::json!({
                "file": root.join("olang.toml").to_string_lossy(),
                "diagnostics": [{
                    "severity": "error", "code": "manifest", "message": problem, "tags": [],
                    "line": 1, "column": 1, "end_line": 1, "end_column": 1,
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                }],
            }));
        }
        for problem in crate::pkg::manifest::Manifest::warnings(root) {
            warnings += 1;
            rows.push(serde_json::json!({
                "file": root.join("olang.toml").to_string_lossy(),
                "diagnostics": [{
                    "severity": "warning", "code": "manifest", "message": problem, "tags": [],
                    "line": 1, "column": 1, "end_line": 1, "end_column": 1,
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                }],
            }));
        }
    }
    // every path in the document absolute, as the other JSON outputs
    let abs = |p: &Path| {
        std::path::absolute(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .display()
            .to_string()
    };
    // a UTF-16 column on `line` of `text` as a 1-based character column
    let char_col = |text: &str, line: u32, utf16: u32| -> usize {
        let l = text.lines().nth(line as usize).unwrap_or("");
        let mut units = 0u32;
        let mut chars = 0usize;
        for c in l.chars() {
            if units >= utf16 {
                break;
            }
            units += c.len_utf16() as u32;
            chars += 1;
        }
        chars + 1
    };
    for file in &files {
        // A notebook is checked as its program (`notebook::Mode::Check`:
        // its cells, `no-run` ones too, on the file's own lines), its own
        // problems first.
        let (text, notebook, front) = match crate::notebook::read(
            file,
            crate::notebook::Mode::Check,
        ) {
            Ok(p) => (p.source, p.diagnostics, p.front_matter),
            Err(e) => {
                errors += 1;
                rows.push(serde_json::json!({
                    "file": abs(file), "error": format!("cannot read: {}", e), "diagnostics": [] }));
                continue;
            }
        };
        let mut found = crate::tools::lsp::file_diagnostics(&text, file.parent());
        // a notebook cell's last statement is the cell's value
        let no_effect = |d: &lsp_types::Diagnostic| matches!(&d.code, Some(NumberOrString::String(c)) if c == "no_effect");
        if crate::notebook::is_notebook(file) && found.iter().any(no_effect) {
            let values = std::fs::read_to_string(file)
                .map(|t| crate::notebook::cell_value_lines(&t))
                .unwrap_or_default();
            found.retain(|d| !(no_effect(d) && values.contains(&(d.range.start.line + 1))));
        }
        // a notebook's params are bound on its front matter's lines, and
        // a param no cell reads is no unused name
        if let Some((open, close)) = front {
            found.retain(|d| {
                let line = d.range.start.line + 1;
                !(line > open
                    && line < close
                    && matches!(&d.code, Some(NumberOrString::String(c)) if c == "unused"))
            });
        }
        let mut out = Vec::with_capacity(found.len() + notebook.len());
        for d in notebook {
            errors += 1;
            let (l, c) = (d.line, d.column);
            out.push(serde_json::json!({
                "severity": "error", "code": "notebook", "message": d.message, "tags": [],
                "line": l, "column": c, "end_line": l, "end_column": c,
                "range": { "start": { "line": l - 1, "character": c - 1 }, "end": { "line": l - 1, "character": c - 1 } },
            }));
        }
        for d in found {
            let severity = match d.severity {
                Some(S::ERROR) | None => "error",
                Some(S::WARNING) => "warning",
                Some(S::INFORMATION) => "information",
                _ => "hint",
            };
            match severity {
                "error" => errors += 1,
                "warning" => warnings += 1,
                _ => {}
            }
            let code = match &d.code {
                Some(NumberOrString::String(c)) => serde_json::Value::String(c.clone()),
                Some(NumberOrString::Number(n)) => serde_json::Value::from(*n),
                None => serde_json::Value::Null,
            };
            let tags: Vec<&str> = d
                .tags
                .iter()
                .flatten()
                .filter(|t| **t == T::UNNECESSARY)
                .map(|_| "unnecessary")
                .collect();
            let (s, e) = (d.range.start, d.range.end);
            out.push(serde_json::json!({
                "severity": severity, "code": code, "message": d.message, "tags": tags,
                "line": s.line + 1, "column": char_col(&text, s.line, s.character),
                "end_line": e.line + 1, "end_column": char_col(&text, e.line, e.character),
                "range": d.range,
            }));
        }
        rows.push(serde_json::json!({ "file": abs(file), "diagnostics": out }));
    }
    println!(
        "{}",
        serde_json::json!({
            "format": 1,
            "kind": "check",
            "olang": crate::version::VERSION,
            "files": rows,
            "errors": errors,
            "warnings": warnings,
        })
    );
    if errors > 0 { 1 } else { 0 }
}

pub fn run(paths: &[PathBuf], rules: Option<&Path>) -> i32 {
    // Many files of one project: keep each imported module's parse.
    crate::expand::keep_module_parses(true);
    let mut files = Vec::new();
    for path in paths {
        if !path.exists() {
            eprintln!("olang check: path not found: {}", path.display());
            return 2;
        }
        files.extend(super::discover_sources(path));
    }
    if files.is_empty() {
        println!("olang check: no .ol files found");
        return 0;
    }

    let mut problems = 0usize;
    let mut warnings = 0usize;
    // The manifests of the projects checked: a `[package] lock` value
    // olang does not know is said once per project (it reads as "auto").
    let mut roots = std::collections::BTreeSet::new();
    for path in paths {
        if let Some(root) = std::path::absolute(path)
            .ok()
            .and_then(|p| crate::pkg::manifest::Manifest::find_root(&p))
        {
            roots.insert(root);
        }
    }
    for root in &roots {
        if let Err(e) = crate::pkg::manifest::Manifest::load_for_capabilities(root) {
            eprintln!("{}: error: {}", root.join("olang.toml").display(), e);
            problems += 1;
        }
        if let Some(problem) = crate::pkg::manifest::Manifest::load(root)
            .ok()
            .and_then(|m| m.olang_value_problem())
        {
            eprintln!("{}: error: {}", root.join("olang.toml").display(), problem);
            problems += 1;
        }
        for problem in crate::pkg::manifest::Manifest::warnings(root) {
            eprintln!(
                "{}: warning: {}",
                root.join("olang.toml").display(),
                problem
            );
            warnings += 1;
        }
    }
    let parser = crate::parser::Parser::new();
    for file in &files {
        let (source, shown) = match std::fs::read_to_string(file) {
            // A notebook: its program checked (`no-run` cells too), its
            // own problems said first, every line the file's.
            Ok(text) if crate::notebook::is_notebook(file) => {
                let program = crate::notebook::program(&text, crate::notebook::Mode::Check);
                for d in &program.diagnostics {
                    problems += 1;
                    let offset = byte_offset_of(&text, d.line as usize, d.column as usize);
                    let report = miette::Report::new(
                        miette::MietteDiagnostic::new(d.message.clone()).with_label(
                            miette::LabeledSpan::at_offset(
                                offset,
                                "the notebook is refused before it runs",
                            ),
                        ),
                    )
                    .with_source_code(miette::NamedSource::new(
                        file.display().to_string(),
                        text.clone(),
                    ));
                    eprintln!("{:?}", report);
                }
                (program.source, Some(text))
            }
            Ok(s) => (s, None),
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
        let unresolved = crate::analyze::Analyzer::unresolved_names(&program);
        let mut diagnostics = file_diagnostics(&program, &source, file.parent(), unresolved);
        // a notebook cell's last statement is the cell's value
        if let Some(text) = &shown
            && diagnostics.iter().any(|d| d.class == Some("no_effect"))
        {
            let values = crate::notebook::cell_value_lines(text);
            diagnostics.retain(|d| !(d.class == Some("no_effect") && values.contains(&d.line)));
        }
        for d in diagnostics {
            if d.warning {
                warnings += 1;
            } else {
                problems += 1;
            }
            let offset = byte_offset_of(
                shown.as_deref().unwrap_or(&source),
                d.line as usize,
                d.column as usize,
            );
            let label = if d.warning {
                "advisory — the program still runs"
            } else if d.scope {
                "the program is refused before it runs"
            } else if d.class == Some("member") {
                "the module has no such member"
            } else if d.runtime {
                "this would fail at runtime"
            } else {
                // A promoted advisory speaks for its own class; the
                // annotation label is the type checker's.
                promoted_label(&d).unwrap_or("the annotation's promise is broken here")
            };
            let mut diagnostic = miette::MietteDiagnostic::new(d.message.clone())
                .with_label(miette::LabeledSpan::at_offset(offset, label));
            if d.warning {
                diagnostic = diagnostic.with_severity(miette::Severity::Warning);
            }
            // a notebook's own text around it: the program's lines are
            // the file's, its prose made empty
            let report =
                miette::Report::new(diagnostic).with_source_code(miette::NamedSource::new(
                    file.display().to_string(),
                    shown.clone().unwrap_or_else(|| source.clone()),
                ));
            eprintln!("{:?}", report);
        }
    }

    // Project-authored lints: run the rules file's `rule_*` functions over
    // each target file's meta AST. Findings count as problems (you opted
    // into the rule, so a hit should gate). `--rules` names the file; with
    // none, each project's `[check] rules` (relative to its root) run over
    // the files checked inside that project.
    if let Some(rules_path) = rules {
        problems += run_rules(rules_path, &files);
    } else {
        for root in &roots {
            let Ok(manifest) = crate::pkg::manifest::Manifest::load(root) else {
                continue;
            };
            let listed = manifest.check.map(|c| c.rules).unwrap_or_default();
            if listed.is_empty() {
                continue;
            }
            let inside: Vec<PathBuf> = files
                .iter()
                .filter(|f| {
                    std::path::absolute(f)
                        .ok()
                        .and_then(|a| crate::pkg::manifest::Manifest::find_root(&a))
                        .is_some_and(|r| &r == root)
                })
                .cloned()
                .collect();
            for rel in listed {
                problems += run_rules(&root.join(rel), &inside);
            }
        }
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
        fixed: false,
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
        let Ok(src) = crate::notebook::read(file, crate::notebook::Mode::Check).map(|p| p.source)
        else {
            continue;
        };
        // `// allow: rule_<name>` on a finding's line: that rule's, by
        // its name (`all` is the checker's classes, not a rule's)
        let allows = crate::tools::allow::scan(&src);
        // Parse to the meta AST; a file that doesn't parse is already
        // reported by the type-check pass, so skip it silently here.
        let nodes = match crate::stdlib::meta::call_meta_function(
            "parse",
            vec![Value::String(Arc::new(src))],
        ) {
            Ok(Value::Ok(ref inner)) => match &**inner {
                Value::List(items) => items.as_ref().clone(),
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
                        if line > 0 && allows.allows(line as u32, name) {
                            continue;
                        }
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

/// `flatten_ast` without the copies: `f` sees every node (a map with a
/// `kind`) in the same preorder, with the nearest enclosing line.
fn visit_nodes(v: &Value, inherited_line: i64, f: &mut dyn FnMut(&crate::ast::ValueMap, i64)) {
    match v {
        Value::Map(m) => {
            let line = match m.get("line") {
                Some(Value::Integer(l)) => *l,
                _ => inherited_line,
            };
            if m.contains_key("kind") {
                f(m, line);
            }
            for val in m.values() {
                visit_nodes(val, line, f);
            }
        }
        Value::List(items) => {
            for it in items.iter() {
                visit_nodes(it, inherited_line, f);
            }
        }
        _ => {}
    }
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
/// of strings, a list of `{ message, line }` objects or maps, or a mix; a
/// bare string, object or map (a single finding) is accepted too. Anything
/// else is no findings.
fn rule_findings(result: &Value) -> Vec<(i64, String)> {
    fn of(m: &crate::ast::ValueMap) -> Option<(i64, String)> {
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
    fn one(v: &Value) -> Option<(i64, String)> {
        match v {
            Value::String(s) => Some((0, s.to_string())),
            Value::Map(m) => of(m),
            Value::Struct { fields, .. } if crate::ast::reads_absent_as_unit(v) => of(fields),
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
    candidates: Vec<String>,
) -> Vec<CheckDiagnostic> {
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
    // A module (a file that shares something) is told where such a name
    // can come from: a module never sees the program's names.
    let module = program
        .statements
        .iter()
        .any(|s| matches!(s.unwrapped(), Statement::ShareDecl(_)));
    candidates
        .into_iter()
        .filter(|name| !excused.contains(name))
        .map(|name| {
            let (line, column) = first_use_of(source, &name);
            CheckDiagnostic {
                line,
                column,
                message: if module {
                    format!(
                        "Undefined variable: {name} — nothing in this file defines or imports it; \
a module doesn't see the program's names: pass it in or import it"
                    )
                } else {
                    format!(
                        "Undefined variable: {name} — nothing in this file defines or imports it"
                    )
                },
                runtime: true,
                warning: false,
                scope: false,
                class: None,
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
                    class: None,
                });
            } else {
                reexported.push((n.to_string(), line));
            }
        }
    }
    // The file's own top-level names bound so far, and where: a `let`'s,
    // a `fn`'s (bound where it stands; a later one rebinds the import).
    let mut own: Vec<(String, u32)> = Vec::new();
    let bare = |path: &[String]| path.last().cloned().unwrap_or_default();
    // Explicit imports seen so far: bound name -> declaration line.
    let mut explicit: Vec<(String, u32)> = Vec::new();
    for stmt in &program.statements {
        let (line, column) = match stmt {
            Statement::Located { line, column, .. } => (*line, *column),
            _ => (0, 0),
        };
        let mut warn = |message: String, warning: bool| {
            out.push(CheckDiagnostic {
                line,
                column,
                message,
                // the refused import raises at run time, as said here
                runtime: !warning,
                warning,
                scope: false,
                class: None,
            })
        };
        if let Statement::LetDecl(l) | Statement::ShareDecl(crate::ast::ShareDecl::Let(l)) =
            stmt.unwrapped()
        {
            let mut names = std::collections::HashSet::new();
            crate::resolve::pattern_names_of(&l.pattern, &mut names);
            own.extend(names.into_iter().map(|n| (n, line)));
            continue;
        }
        if let Statement::FunctionDecl(f)
        | Statement::ShareDecl(crate::ast::ShareDecl::Function(f)) = stmt.unwrapped()
        {
            own.push((f.name.clone(), line));
            continue;
        }
        let (Statement::UseDecl(u) | Statement::ShareDecl(crate::ast::ShareDecl::Use(u))) =
            stmt.unwrapped()
        else {
            continue;
        };
        let module = u.path.join(".");
        let is_wildcard = u
            .items
            .iter()
            .any(|i| matches!(i, crate::ast::UseItem::Wildcard));
        if is_wildcard {
            let Some(exports) = exports_of(&u.path, doc_dir) else {
                continue;
            };
            for name in &exports {
                // What the import leaves alone (the runtime skips it).
                if crate::stdlib::is_module_name(name) && module != "collections" {
                    warn(
                        format!(
                            "`use {}` also exports '{}', which names the stdlib module `{}`: \
the import leaves `{}` the module — reach the export as `{}.{}`, or list it under \
another name (`{} as other_name`)",
                            module,
                            name,
                            name,
                            name,
                            bare(&u.path),
                            name,
                            name
                        ),
                        true,
                    );
                } else if let Some((_, at)) = own.iter().find(|(n, _)| n == name) {
                    warn(
                        format!(
                            "`use {}` also exports '{}', which this file binds on line {}: \
the import leaves `{}` the file's — reach the export as `{}.{}`, or list it under \
another name (`{} as other_name`)",
                            module,
                            name,
                            at,
                            name,
                            bare(&u.path),
                            name,
                            name
                        ),
                        true,
                    );
                }
            }
            for (name, decl_line) in &explicit {
                if exports.iter().any(|e| e == name) {
                    warn(
                        format!(
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
                        true,
                    );
                }
            }
        }
        for item in &u.items {
            if let Some(n) = item.bound_name() {
                // A listed name the file already binds: the runtime refuses
                // the import rather than replace it.
                if let Some((_, at)) = own.iter().find(|(o, _)| o == n) {
                    let listed = match item {
                        crate::ast::UseItem::Aliased { name, alias } => {
                            format!("{name} as {alias}")
                        }
                        _ => n.to_string(),
                    };
                    warn(
                        format!(
                            "`use {} {{ {} }}` would replace '{}', which this file binds on line {}: \
an import never replaces a file's own name — list it under another name \
(`{} as other_name`) or rename the binding",
                            module,
                            listed,
                            n,
                            at,
                            item.source_name().unwrap_or(n)
                        ),
                        false,
                    );
                }
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
    let nodes = crate::stdlib::meta::program_nodes(program);
    let callable = callable_names(&nodes);
    shadow_warnings_of(&nodes, &callable)
}

/// `shadow_warnings`, the same findings in the same order, without
/// turning the whole program into meta nodes: only the top-level
/// statements that can warn are converted — a function with a parameter
/// named like a callable, a `use` whose leaf is a stdlib module — and
/// the whole program only when `source` (the text `program` was parsed
/// from) binds a stdlib module's name with `let` somewhere. The full
/// conversion was most of a 50,000-line file's check.
pub fn shadow_warnings_in(program: &Program, source: &str) -> Vec<CheckDiagnostic> {
    use crate::ast::{ShareDecl, UseItem};
    let stdlib = crate::stdlib::get_stdlib();
    if may_let_bind_any(source, |w| stdlib.contains_key(w)) {
        return shadow_warnings(program);
    }
    // What `callable_names` reads from the nodes, read from the tree.
    let mut callable: std::collections::HashSet<String> = std::collections::HashSet::new();
    for stmt in &program.statements {
        match stmt.unwrapped() {
            Statement::UseDecl(u) | Statement::ShareDecl(ShareDecl::Use(u)) => {
                callable.extend(u.items.iter().map(|i| match i {
                    UseItem::Specific(n) => n.clone(),
                    UseItem::Aliased { name, alias } => format!("{} as {}", name, alias),
                    UseItem::Wildcard => "*".to_string(),
                }));
            }
            Statement::FunctionDecl(f) | Statement::ShareDecl(ShareDecl::Function(f)) => {
                callable.insert(f.name.clone());
            }
            _ => {}
        }
    }
    let nodes: Vec<crate::ast::Value> = program
        .statements
        .iter()
        .filter(|stmt| match stmt.unwrapped() {
            Statement::UseDecl(u) | Statement::ShareDecl(ShareDecl::Use(u)) => {
                u.path.len() >= 2 && u.path.last().is_some_and(|l| stdlib.contains_key(l))
            }
            Statement::FunctionDecl(f) | Statement::ShareDecl(ShareDecl::Function(f)) => {
                f.parameters.iter().any(|p| callable.contains(&p.name))
            }
            _ => false,
        })
        .map(crate::stdlib::meta::stmt_node)
        .collect();
    shadow_warnings_of(&nodes, &callable)
}

/// Whether `source` may hold `let NAME` (or `let mut NAME`) with `hit(NAME)`
/// — conservative: comments and strings are not told apart.
fn may_let_bind_any(source: &str, hit: impl Fn(&str) -> bool) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut rest = source;
    while let Some(at) = rest.find("let") {
        let before = rest[..at].chars().next_back();
        let after = &rest[at + 3..];
        rest = after;
        if before.is_some_and(word) || !after.starts_with(char::is_whitespace) {
            continue;
        }
        let mut name = after.trim_start();
        if let Some(m) = name.strip_prefix("mut")
            && m.starts_with(char::is_whitespace)
        {
            name = m.trim_start();
        }
        let end = name.find(|c: char| !word(c)).unwrap_or(name.len());
        if hit(&name[..end]) {
            return true;
        }
    }
    false
}

/// Names a body may call that a parameter could shadow: imports and
/// module-level functions, read from the top-level meta nodes.
fn callable_names(nodes: &[Value]) -> std::collections::HashSet<String> {
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
            Some(Value::List(ref items)) => items.as_ref().clone(),
            _ => Vec::new(),
        }
    };
    let mut callable: std::collections::HashSet<String> = std::collections::HashSet::new();
    for node in nodes {
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
    callable
}

/// Parameters and pattern bindings that take a stdlib module's name and
/// are then read as that module: `fn table_sort(rows, col) = … col.sort_by
/// (…)` reaches the parameter, and fails at run time. A binding of the
/// name that is never read as the module is not reported — `col` and
/// `time` are ordinary parameter names. (`let` has its own rule above.)
pub fn binding_shadow_warnings(program: &Program) -> Vec<CheckDiagnostic> {
    let mut modules: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for (name, module) in crate::stdlib::get_stdlib().iter() {
        if let Value::Struct { fields, .. } = module {
            modules.insert(name.clone(), fields.keys().cloned().collect());
        }
    }
    let mut walker = BindingShadows {
        modules,
        at: (0, 0),
        out: Vec::new(),
    };
    for statement in &program.statements {
        walker.statement(statement);
    }
    walker.out
}

struct BindingShadows {
    modules: HashMap<String, std::collections::HashSet<String>>,
    at: (u32, u32),
    out: Vec<CheckDiagnostic>,
}

impl BindingShadows {
    fn statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Located { line, column, stmt } => {
                let saved = self.at;
                self.at = (*line, *column);
                self.statement(stmt);
                self.at = saved;
            }
            Statement::Expression(e) => self.expr(e),
            Statement::LetDecl(l) => {
                if let Some(v) = &l.value {
                    self.expr(v);
                }
            }
            Statement::FunctionDecl(f) => self.function(f),
            Statement::ShareDecl(crate::ast::ShareDecl::Function(f)) => self.function(f),
            Statement::ShareDecl(crate::ast::ShareDecl::Let(l)) => {
                if let Some(v) = &l.value {
                    self.expr(v);
                }
            }
            Statement::ImplDecl(i) => {
                for m in &i.methods {
                    self.function(m);
                }
            }
            Statement::TestDecl(t) => {
                for s in &t.body {
                    self.statement(s);
                }
            }
            _ => {}
        }
    }

    fn function(&mut self, f: &crate::ast::FunctionDecl) {
        for p in &f.parameters {
            self.binder(
                &p.name,
                &[&f.body],
                &format!("parameter `{}` of `{}`", p.name, f.name),
            );
        }
        self.expr(&f.body);
    }

    /// `name` bound over `scope`: reported when the scope reads it as the
    /// stdlib module of that name.
    fn binder(&mut self, name: &str, scope: &[&Expr], what: &str) {
        let Some(members) = self.modules.get(name) else {
            return;
        };
        let Some(member) = scope
            .iter()
            .find_map(|e| Self::module_read(e, name, members))
        else {
            return;
        };
        self.out.push(CheckDiagnostic {
            line: self.at.0,
            column: self.at.1,
            message: format!(
                "{} shadows the stdlib module `{}`: `{}.{}` in its scope reaches the binding, \
not the module — rename the binding",
                what, name, name, member
            ),
            runtime: false,
            warning: true,
            scope: false,
            class: Some("shadow"),
        });
    }

    /// The first `name.member` in `e` whose member the module has.
    fn module_read(
        e: &Expr,
        name: &str,
        members: &std::collections::HashSet<String>,
    ) -> Option<String> {
        if let Expr::FieldAccess { object, field } = e
            && matches!(object.as_ref(), Expr::Identifier(n) | Expr::LocalRef { name: n, .. } if n == name)
            && members.contains(field)
        {
            return Some(field.clone());
        }
        crate::tools::keys::children(e)
            .into_iter()
            .find_map(|c| Self::module_read(c, name, members))
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Lambda {
                parameters, body, ..
            } => {
                for p in parameters {
                    self.binder(
                        &p.name,
                        &[body],
                        &format!("the lambda parameter `{}`", p.name),
                    );
                }
                self.expr(body);
                return;
            }
            Expr::ForLoop {
                variable,
                body,
                iterable,
            }
            | Expr::ParForLoop {
                variable,
                body,
                iterable,
            } => {
                self.expr(iterable);
                self.binder(
                    variable,
                    &[body],
                    &format!("the loop variable `{}`", variable),
                );
                self.expr(body);
                return;
            }
            Expr::Match { value, arms } => {
                self.expr(value);
                for arm in arms {
                    let mut names = std::collections::HashSet::new();
                    crate::resolve::pattern_names_of(&arm.pattern, &mut names);
                    let mut scope: Vec<&Expr> = vec![&arm.expression];
                    if let Some(g) = &arm.guard {
                        scope.push(g);
                    }
                    let mut names: Vec<String> = names.into_iter().collect();
                    names.sort();
                    for n in names {
                        self.binder(&n, &scope, &format!("the pattern binding `{}`", n));
                    }
                    if let Some(g) = &arm.guard {
                        self.expr(g);
                    }
                    self.expr(&arm.expression);
                }
                return;
            }
            Expr::Block(statements) => {
                for (i, s) in statements.iter().enumerate() {
                    if let Statement::LetDecl(l) = s.unwrapped()
                        && !matches!(l.pattern, crate::ast::Pattern::Identifier(_))
                    {
                        let mut names = std::collections::HashSet::new();
                        crate::resolve::pattern_names_of(&l.pattern, &mut names);
                        let rest: Vec<&Expr> = statements[i + 1..]
                            .iter()
                            .filter_map(|r| match r.unwrapped() {
                                Statement::Expression(x) => Some(x),
                                Statement::LetDecl(l) => l.value.as_ref(),
                                _ => None,
                            })
                            .collect();
                        let mut names: Vec<String> = names.into_iter().collect();
                        names.sort();
                        if let Statement::Located { line, column, .. } = s {
                            self.at = (*line, *column);
                        }
                        for n in names {
                            self.binder(&n, &rest, &format!("the pattern binding `{}`", n));
                        }
                    }
                    self.statement(s);
                }
                return;
            }
            _ => {}
        }
        for c in crate::tools::keys::children(e) {
            self.expr(c);
        }
    }
}

/// The findings of `shadow_warnings` over `nodes`, given the callables.
fn shadow_warnings_of(
    nodes: &[Value],
    callable: &std::collections::HashSet<String>,
) -> Vec<CheckDiagnostic> {
    use crate::ast::Value;
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
            Some(Value::List(ref items)) => items.as_ref().clone(),
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
    let mut out = Vec::new();
    for node in nodes {
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
                let mut called: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                if let Some(body) = field(node, "body") {
                    visit_nodes(&body, 0, &mut |n, _| {
                        if matches!(n.get("kind"), Some(Value::String(k)) if k.as_str() == "call")
                            && let Some(Value::String(t)) = n.get("target")
                        {
                            called.insert(t.to_string());
                        }
                    });
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
                            class: None,
                        });
                    }
                }
            }
            Some("use") => {
                let path = match field(node, "path") {
                    Some(Value::String(ref p)) => {
                        p.split('.').map(str::to_string).collect::<Vec<_>>()
                    }
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
                        class: None,
                    });
                }
            }
            _ => {}
        }
    }
    // `let` bindings, at any depth, that take a stdlib module's name.
    let mut lets: Vec<(String, u32, u32)> = Vec::new();
    for node in nodes {
        visit_nodes(node, 0, &mut |n, line| {
            if matches!(n.get("kind"), Some(Value::String(k)) if k.as_str() == "let")
                && let Some(Value::String(name)) = n.get("name")
                && stdlib.contains(name.as_str())
            {
                let column = match n.get("column") {
                    Some(Value::Integer(c)) => *c as u32,
                    _ => 0,
                };
                lets.push((name.to_string(), line as u32, column));
            }
        });
    }
    for (name, line, column) in lets {
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
            class: Some("shadow"),
        });
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

/// A conservative lexical pass: false only when no backtick template in
/// `source` can contain `\n`, `\t` or `\r`. Comments and double-quoted
/// (and raw) strings are skipped so their backticks and escapes do not
/// count; inside a template anything suspicious answers true, `${…}`
/// bodies included.
fn may_hold_template_escape(source: &str) -> bool {
    if !source.contains('`') {
        return false;
    }
    let b = source.as_bytes();
    let mut i = 0;
    let mut in_template = false;
    while i < b.len() {
        let c = b[i];
        if in_template {
            match c {
                b'\\' => {
                    if matches!(b.get(i + 1), Some(b'n' | b't' | b'r')) {
                        return true;
                    }
                    i += 2;
                    continue;
                }
                b'`' => in_template = false,
                _ => {}
            }
            i += 1;
            continue;
        }
        match c {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'`' => in_template = true,
            _ => {}
        }
        i += 1;
    }
    false
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
    // The spans come from a second full parse of the file; most files
    // have no template holding a `\n`, `\t` or `\r` at all.
    if !may_hold_template_escape(source) {
        return Vec::new();
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
                            class: None,
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
    // A file being typed does not parse (a call opened, not closed): its
    // `use` lines alone still say which modules it reads — what the
    // language server's signature help needs most when the file is
    // mid-edit.
    let program = match crate::parser::Parser::new().parse(text) {
        Ok(p) => p,
        Err(_) => {
            let uses: Vec<&str> = text
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    t.starts_with("use ") || t.starts_with("share use ")
                })
                .collect();
            match crate::parser::Parser::new().parse(&uses.join("\n")) {
                Ok(p) => p,
                Err(_) => return Vec::new(),
            }
        }
    };
    modules_used_by(&program, dir, reexports_only)
}

/// `modules_used` over an already-parsed program.
fn modules_used_by(
    program: &Program,
    dir: &Path,
    reexports_only: bool,
) -> Vec<(PathBuf, String, Program)> {
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
                    let reexports = match c.parent() {
                        Some(d) => modules_used_by(&prog, d, true),
                        None => Vec::new(),
                    };
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
    /// The advisory's class, which `[check] promote` names and
    /// `--format json` reports as its `code`: `exhaustiveness`, `shape`,
    /// `keys`, `result`, `copy`, `shadow`, `no_effect` (and `allow`, an
    /// allow comment's own warning, which is never promoted). Set where
    /// the finding is made, never read back from its wording.
    pub class: Option<&'static str>,
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
            // `Any` is any value: nothing to prove.
            TypeAnnotation::Custom(name) if name == "Any" => SType::Unknown,
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

/// `.key` reads no construction of their value has (roadmap E4), each
/// at the read itself: the statement's line, and the first `.key` from
/// there on.
pub fn key_warnings(program: &Program, source: &str) -> Vec<CheckDiagnostic> {
    let lines: Vec<&str> = source.lines().collect();
    if std::env::var_os("OLANG_KEYS_STATS").is_some() {
        let (known, total) = crate::tools::keys::read_coverage(program);
        eprintln!("keys-stats {known} {total}");
    }
    crate::tools::keys::impossible_reads(program)
        .into_iter()
        .map(|f| {
            let needle = format!(".{}", f.key);
            let mut at = (f.line, f.column);
            'find: for (i, text) in lines.iter().enumerate().skip(f.line.saturating_sub(1) as usize).take(40) {
                let from = if i + 1 == f.line as usize { f.column.saturating_sub(1) as usize } else { 0 };
                let chars: Vec<char> = text.chars().collect();
                let hay: String = chars.iter().skip(from).collect();
                let mut search = 0;
                while let Some(pos) = hay[search..].find(&needle) {
                    let end = search + pos + needle.len();
                    let rest = hay[end..].trim_start();
                    // the read itself, not a presence test of the key
                    let presence = rest.starts_with("== ()")
                        || rest.starts_with("!= ()")
                        || rest.starts_with("??");
                    if !presence && !hay[end..].starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                        let col = from + hay[..search + pos].chars().count() + 2;
                        at = ((i + 1) as u32, col as u32);
                        break 'find;
                    }
                    search = end;
                }
            }
            let nearest = f
                .known
                .iter()
                .filter(|k| edit_distance(k, &f.key) <= 2.max(f.key.len() / 3))
                .min_by_key(|k| edit_distance(k, &f.key));
            let hint = match nearest {
                Some(k) => format!(" — did you mean `{}`?", k),
                None => String::new(),
            };
            let what = if f.receiver.is_empty() { "this value".to_string() } else { format!("`{}`", f.receiver) };
            let known = if f.known.is_empty() {
                "none".to_string()
            } else {
                f.known.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")
            };
            CheckDiagnostic {
                line: at.0,
                column: at.1,
                message: format!(
                    "`{}` is not a key any construction of {} has (it has {}): the read is () at run time{}",
                    f.key, what, known, hint
                ),
                runtime: false,
                warning: true,
                scope: false,
                class: Some("keys"),
            }
        })
        .collect()
}

/// The rule a finding comes from, as `olang check --format json` and the
/// language server's `code` name it: an advisory's class (`shadow`,
/// `exhaustiveness`, `shape`, `result`, `copy`), else `scope` (refused
/// before the program runs), `runtime` (would fail when it runs),
/// `advisory` (another warning) or `type` (an annotation's promise).
pub fn rule_of(d: &CheckDiagnostic) -> &'static str {
    if let Some(class) = d.class {
        class
    } else if d.scope {
        "scope"
    } else if d.runtime {
        "runtime"
    } else if d.warning {
        "advisory"
    } else {
        "type"
    }
}

/// The label an advisory promoted by `[check] promote` is rendered with:
/// what its class says about the spot, not the annotation checker's.
pub fn promoted_label(d: &CheckDiagnostic) -> Option<&'static str> {
    if !d.message.ends_with(PROMOTED) {
        return None;
    }
    Some(match d.class? {
        "shadow" => "shadows the module from here",
        "exhaustiveness" => "this match misses a case",
        "shape" => "the declared shape has no such key",
        "keys" => "no construction has this key",
        "result" => "the Result is dropped here",
        "copy" => "the collection is copied here on every pass",
        "no_effect" => "this value is computed and dropped",
        _ => return None,
    })
}

/// The format number of `olang check --tier --format json`, its own
/// (it shared `--ovm-stats`'s until 0.89).
pub const TIER_CHECK_FORMAT: u32 = 1;

/// What a promoted advisory's message ends with.
const PROMOTED: &str = " (promoted to an error by [check] promote in olang.toml)";

/// Turn a warning into an error when its class is promoted.
pub fn promote(d: &mut CheckDiagnostic, promoted: &[String]) {
    if !d.warning || promoted.is_empty() {
        return;
    }
    let Some(class) = d.class else {
        return;
    };
    if promoted.iter().any(|p| p == class || p == "all") {
        d.warning = false;
        d.message = format!("{}{}", d.message, PROMOTED);
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
                if !matches!(
                    op,
                    crate::ast::BinaryOp::And
                        | crate::ast::BinaryOp::Or
                        | crate::ast::BinaryOp::Coalesce
                ) {
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
/// is a wildcard (a bare `use` of a file module is one), and what it
/// brings in cannot be listed from the file alone.
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
        // No items is the namespace alone — `use m {}`, or a stdlib
        // module's bare `use gg` (the parser gives it no items): it puts
        // no name in scope, so `find` stays the builtin beside it.
        if u.items.is_empty() {
            continue;
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
    // The program's last statement is its value (the REPL's result, a
    // notebook cell's output); the others' values are discarded.
    let last = program.statements.len().saturating_sub(1);
    for (i, stmt) in program.statements.iter().enumerate() {
        checker.top_level_tail = i == last;
        checker.check_statement(stmt, (0, 0));
    }
    checker.top_level_tail = false;

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
            class: None,
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
    // The program's last statement is its value (the REPL's result, a
    // notebook cell's output); the others' values are discarded.
    let last = program.statements.len().saturating_sub(1);
    for (i, stmt) in program.statements.iter().enumerate() {
        checker.top_level_tail = i == last;
        checker.check_statement(stmt, (0, 0));
    }
    checker.top_level_tail = false;
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
    /// True while walking the program's last top-level statement, its
    /// value. (`in_tail_position` stays a block's, so a dropped Result
    /// at the end of a script is still reported.)
    top_level_tail: bool,
    /// Qualified stdlib names documented as returning `Result`, from the
    /// help registry. Populated lazily — most checks never need it.
    fallible: std::collections::HashSet<String>,
    /// Names a `use` or a top-level `let` binds in this file, and whether
    /// a wildcard `use` may bind any name: a call of one of them is not a
    /// builtin's.
    user_bound: std::collections::HashSet<String>,
    wildcard_use: bool,
    /// The embedded modules this file imports by their own name (`use
    /// gg`, `use ml { split }`): their namespaces' members are known.
    embedded_uses: std::collections::HashSet<String>,
    /// `module.member` reads compared with `()`: a presence test, which
    /// raises when the member is missing (`map_get` is the test).
    probed: std::collections::HashSet<(String, String)>,
    /// Every trait method name the file declares or implements: a read of
    /// one off a struct may be the method, not a field.
    methods: std::collections::HashSet<String>,
    out: Vec<CheckDiagnostic>,
}

/// How many arguments each global builtin takes, as its implementation
/// counts them (`min`, `max`): a call outside the range raises. Measured
/// against the runtime and pinned by `builtin_arity_table_agrees_with_the_runtime`.
/// The variadic output and assertion forms are left out.
const GLOBAL_ARITY: &[(&str, usize, usize)] = &[
    ("abs", 1, 1),
    ("all", 2, usize::MAX),
    ("any", 2, usize::MAX),
    ("attempt", 1, 1),
    ("average", 1, 1),
    ("chunk", 2, 2),
    ("clamp", 3, 3),
    ("concat", 2, 2),
    ("cons", 2, 2),
    ("contains", 2, 2),
    ("drop", 2, 2),
    ("ends_with", 2, 2),
    ("entries", 1, 1),
    ("enumerate", 1, 1),
    ("eprint", 1, 1),
    ("filter", 2, 2),
    ("find", 2, 2),
    ("flatten", 1, 1),
    ("fold", 3, 3),
    ("first", 1, 1),
    ("group_by", 2, 2),
    ("head", 1, 1),
    ("implements", 2, 2),
    ("is_err", 1, 1),
    ("is_ok", 1, 1),
    ("join", 2, 2),
    ("last", 1, 1),
    ("len", 1, 1),
    ("map", 2, 2),
    ("map_clear", 1, 1),
    ("map_filtered", 3, 3),
    ("map_get", 2, 2),
    ("map_get_in", 2, 2),
    ("map_get_or", 3, 3),
    ("map_has_key", 2, 2),
    ("map_keys", 1, 1),
    ("map_len", 1, 1),
    ("map_merge", 2, 2),
    ("map_remove", 2, 2),
    ("map_set", 3, 3),
    ("map_set_in", 3, 3),
    ("map_update", 3, 3),
    ("map_values", 1, 1),
    ("max", 1, usize::MAX),
    ("min", 1, usize::MAX),
    ("par_filter", 2, 2),
    ("par_map", 2, 2),
    ("raise", 1, 1),
    ("range", 1, 3),
    ("reduce", 2, 2),
    ("result_map", 2, 2),
    ("result_map_err", 2, 2),
    ("reverse", 1, 1),
    ("show", 1, 1),
    ("skip", 2, 2),
    ("sort", 1, 1),
    ("split", 2, 2),
    ("starts_with", 2, 2),
    ("sum", 1, 1),
    ("tail", 1, 1),
    ("take", 2, 2),
    ("to_float", 1, 1),
    ("to_int", 1, 1),
    ("to_string", 1, 1),
    ("typeof", 1, 1),
    ("unwrap", 1, 1),
    ("unwrap_or", 2, 2),
    ("unwrap_or_else", 2, 2),
    ("zip", 2, 2),
];

/// The names an embedded module (`gg`, `ml`, `ui`) shares, read from its
/// source once per process.
fn embedded_exports(module: &str) -> Option<Vec<String>> {
    static TABLE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Option<Vec<String>>>>> =
        std::sync::OnceLock::new();
    let table = TABLE.get_or_init(Default::default);
    if let Some(hit) = table.lock().ok()?.get(module) {
        return hit.clone();
    }
    let names = crate::stdlib::embedded::parsed(module)
        .ok()
        .flatten()
        .map(|p| {
            let mut names = Vec::new();
            for stmt in &p.statements {
                if let Statement::ShareDecl(d) = stmt.unwrapped() {
                    match d {
                        crate::ast::ShareDecl::Function(f) => names.push(f.name.clone()),
                        crate::ast::ShareDecl::Type(t) => names.push(t.name.clone()),
                        crate::ast::ShareDecl::Let(l) => {
                            let mut found = std::collections::HashSet::new();
                            crate::resolve::pattern_names_of(&l.pattern, &mut found);
                            names.extend(found);
                        }
                        crate::ast::ShareDecl::Use(u) => names.extend(
                            u.items
                                .iter()
                                .filter_map(|i| i.bound_name())
                                .map(String::from),
                        ),
                        _ => {}
                    }
                }
            }
            names
        });
    table.lock().ok()?.insert(module.to_string(), names.clone());
    names
}

/// The fewest arguments a stdlib module's function takes: the arity its
/// module registers it with ("the fewest arguments a call takes"), which
/// its implementation asks for ("str.thousands: missing argument 2").
fn module_min_arity(module: &str, function: &str) -> Option<usize> {
    static TABLE: std::sync::OnceLock<HashMap<String, usize>> = std::sync::OnceLock::new();
    TABLE
        .get_or_init(|| {
            let mut t = HashMap::new();
            for (name, module) in crate::stdlib::get_stdlib().iter() {
                if let Value::Struct { fields, .. } = module {
                    for (f, v) in fields.iter() {
                        if let Value::Builtin(b) = v {
                            t.insert(format!("{}.{}", name, f), b.arity);
                        }
                    }
                }
            }
            t
        })
        .get(&format!("{}.{}", module, function))
        .copied()
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
    // ── pass 1: signatures and struct shapes ─────────────────────────
    fn collect(&mut self, program: &Program) {
        for stmt in &program.statements {
            self.collect_statement(stmt.unwrapped());
            let use_decl = match stmt.unwrapped() {
                Statement::UseDecl(u) | Statement::ShareDecl(crate::ast::ShareDecl::Use(u)) => {
                    Some(u)
                }
                _ => None,
            };
            if let Some(u) = use_decl {
                // `use ods` binds the stdlib module itself: its calls stay
                // the module's (checked against its signatures).
                let stdlib_module =
                    matches!(u.path.as_slice(), [one] if crate::stdlib::is_module_name(one));
                if let Some(last) = u.path.last()
                    && !stdlib_module
                {
                    self.user_bound.insert(last.clone());
                }
                if let [one] = u.path.as_slice()
                    && crate::stdlib::embedded::is_embedded(one)
                {
                    self.embedded_uses.insert(one.clone());
                }
                for item in &u.items {
                    match item.bound_name() {
                        Some(n) => {
                            self.user_bound.insert(n.to_string());
                        }
                        None => self.wildcard_use = true,
                    }
                }
            }
            match stmt.unwrapped() {
                Statement::ImplDecl(i) | Statement::ShareDecl(crate::ast::ShareDecl::Impl(i)) => {
                    self.methods
                        .extend(i.methods.iter().map(|m| m.name.clone()));
                }
                Statement::TraitDecl(t) | Statement::ShareDecl(crate::ast::ShareDecl::Trait(t)) => {
                    self.methods
                        .extend(t.methods.iter().map(|m| m.name.clone()));
                }
                _ => {}
            }
            let let_decl = match stmt.unwrapped() {
                Statement::LetDecl(l) | Statement::ShareDecl(crate::ast::ShareDecl::Let(l)) => {
                    Some(l)
                }
                _ => None,
            };
            if let Some(l) = let_decl {
                let mut names = std::collections::HashSet::new();
                crate::resolve::pattern_names_of(&l.pattern, &mut names);
                self.user_bound.extend(names);
            }
        }
    }

    /// A call of a global builtin or a stdlib module's function with an
    /// argument count its implementation refuses. A name the file binds
    /// (a local, a `use`, a top-level `let` or `fn`) is not the builtin.
    fn check_builtin_arity(&mut self, callee: &Expr, arguments: &[Argument], span: (u32, u32)) {
        if !arguments
            .iter()
            .all(|a| matches!(a, Argument::Positional(_)))
        {
            return;
        }
        let bound = |me: &Self, n: &str| {
            me.sigs.contains_key(n)
                || me.user_bound.contains(n)
                || me.scopes.iter().any(|scope| scope.contains_key(n))
        };
        let got = arguments.len();
        match callee {
            Expr::Identifier(name) if !self.wildcard_use && !bound(self, name) => {
                let Some(&(_, min, max)) = GLOBAL_ARITY.iter().find(|(n, _, _)| n == name) else {
                    return;
                };
                if got >= min && got <= max {
                    return;
                }
                let message = if min == max {
                    format!("Arity mismatch: expected {}, got {}", min, got)
                } else if max == usize::MAX {
                    format!("`{}` takes at least {} arguments, got {}", name, min, got)
                } else {
                    format!("`{}` takes {} to {} arguments, got {}", name, min, max, got)
                };
                self.diag(span, true, message);
            }
            Expr::FieldAccess { object, field } => {
                let Expr::Identifier(module) = object.as_ref() else {
                    return;
                };
                // The modules that register a function by the fewest
                // arguments it takes; the data stack's and the engines'
                // (`ods`, `gui`, `vt`, …) register a usual count instead.
                const MIN_REGISTERED: &[&str] = &[
                    "str", "col", "math", "json", "toml", "csv", "re", "dates", "time", "base64",
                    "fs", "os", "db", "crypto", "chan", "task", "cell", "proc", "bytes",
                    "compress", "bigint", "random", "caps", "runtime", "asset", "vec", "meta",
                    "testing", "md",
                ];
                if bound(self, module) || !MIN_REGISTERED.contains(&module.as_str()) {
                    return;
                }
                let Some(min) = module_min_arity(module, field) else {
                    return;
                };
                if got < min {
                    self.diag(
                        span,
                        true,
                        format!(
                            "{}.{}: missing argument {} (it takes at least {}, got {})",
                            module,
                            field,
                            got + 1,
                            min,
                            got
                        ),
                    );
                }
            }
            _ => {}
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

    // ── scope helpers ───────────────────────────────────────────────
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
            if let Some(d) = self.out.last_mut() {
                d.class = Some("exhaustiveness");
            }
        } else {
            self.advise(
                "exhaustiveness",
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
        self.advise(
            "exhaustiveness",
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
            _ => "the read is () at run time".to_string(),
        };
        self.advise(
            "shape",
            span,
            format!(
                "`{}` is not a key of the declared shape {}: {}{}",
                key, shape, read, hint
            ),
        );
    }

    /// A member a stdlib module does not have: `math.pi` (it is
    /// `math.PI`) and `str.trimm(x)` raise at run time. The module is a
    /// native one the file does not rebind, or an embedded one it imports
    /// (`use gg` → `gg.*`). A read compared with `()` (a presence test)
    /// raises too: `map_get(m, "name")` is the test.
    fn check_module_member(&mut self, object: &Expr, field: &str, call: bool, span: (u32, u32)) {
        let Expr::Identifier(module) = object else {
            return;
        };
        let members: Vec<String> = if self.embedded_uses.contains(module) {
            match embedded_exports(module) {
                Some(m) => m,
                None => return,
            }
        } else {
            let bound = self.sigs.contains_key(module)
                || self.user_bound.contains(module)
                || self.scopes.iter().any(|scope| scope.contains_key(module));
            if bound {
                return;
            }
            match crate::stdlib::native_members().get(module) {
                Some(m) => m.clone(),
                None => return,
            }
        };
        if members.iter().any(|m| m == field) {
            return;
        }
        let probe = self.probed.contains(&(module.clone(), field.to_string()));
        // the same name in another case first (`pi` → `PI`), then the
        // nearest spelling
        let lower = field.to_lowercase();
        let nearest = members
            .iter()
            .find(|m| m.to_lowercase() == lower)
            .or_else(|| {
                members
                    .iter()
                    .filter(|m| {
                        let d = edit_distance(m, field);
                        d <= (field.len() / 3).max(1) && d < field.len()
                    })
                    .min_by_key(|m| edit_distance(m, field))
            });
        let hint = match nearest {
            Some(n) => format!(" — did you mean `{}.{}`?", module, n),
            None => String::new(),
        };
        let what = if probe {
            format!(
                "reading it raises at run time, so a presence test is `map_get({}, \"{}\") == ()`",
                module, field
            )
        } else if call {
            "calling it raises at run time".to_string()
        } else {
            "reading it raises at run time".to_string()
        };
        self.out.push(CheckDiagnostic {
            line: span.0,
            column: span.1,
            message: format!("`{}` has no member `{}`: {}{}", module, field, what, hint),
            runtime: call,
            warning: false,
            scope: false,
            class: Some("member"),
        });
    }

    /// A field read off a value of a declared struct type (`type P =
    /// struct { x: Int }`) that the declaration does not have: reading it
    /// raises at run time. Only a value the checker knows the type of is
    /// judged — an annotation, a struct literal, a declared return — and
    /// a name some trait method has is left (it may be read as one).
    fn check_struct_field(&mut self, object: &Expr, field: &str, span: (u32, u32)) {
        let SType::Named(type_name) = self.infer(object) else {
            return;
        };
        let Some(fields) = self.structs.get(&type_name) else {
            return;
        };
        if fields.iter().any(|(n, _)| n == field) || self.methods.contains(field) {
            return;
        }
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        let nearest = names
            .iter()
            .filter(|n| {
                let d = edit_distance(n, field);
                d <= (field.len() / 3).max(1) && d < field.len()
            })
            .min_by_key(|n| edit_distance(n, field));
        let hint = match nearest {
            Some(n) => format!(" — did you mean `{}`?", n),
            None => String::new(),
        };
        self.advise("shape", 
            span,
            format!(
                "`{}` is not a field of the declared struct {} ({}): reading it raises at run time{}",
                field,
                type_name,
                names.join(", "),
                hint
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
            class: None,
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
                        self.advise("copy", 
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
                        self.advise("copy", 
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
        self.advise(
            "result",
            span,
            format!(
                "the Result from {qualified} is discarded, so a failure here is invisible. \
                 Bind it, match it, unwrap it to fail loudly, or write `let _ = ...` to say \
                 the failure is deliberately ignored"
            ),
        );
    }

    /// An expression statement that computes a value and does nothing
    /// else — a literal, a name, arithmetic or a comparison over them —
    /// outside a block's last slot, where its value is discarded. It
    /// is almost always a line that was meant to continue the one above
    /// (`-10.0` under `let a = b`, a negation of its own) or a
    /// comparison meant as an assertion. `()` is exempt: it is the
    /// written way to say "nothing here".
    fn warn_no_effect(&mut self, expr: &Expr, span: (u32, u32)) {
        if self.in_tail_position || self.top_level_tail || !Self::no_effect(expr) {
            return;
        }
        let message = if let Expr::UnaryOp {
            op: crate::ast::UnaryOp::Negate,
            ..
        } = expr
        {
            "this statement is a negation whose value is unused: a line that starts with \
             `-x` is a statement of its own. To continue the expression above, write the \
             `-` with a space after it (`- x`) or end the line above with the operator"
                .to_string()
        } else {
            "this expression's value is unused, and computing it has no effect: use it \
             (bind it, return it from the block's last line) or remove it"
                .to_string()
        };
        self.advise("no_effect", span, message);
    }

    /// Whether evaluating `e` can do nothing but produce its value.
    fn no_effect(e: &Expr) -> bool {
        match e {
            Expr::Integer(_)
            | Expr::Float(_)
            | Expr::String(_)
            | Expr::RawString(_)
            | Expr::Boolean(_)
            | Expr::Identifier(_)
            | Expr::LocalRef { .. } => true,
            Expr::BinaryOp { left, right, .. } | Expr::BitwiseOp { left, right, .. } => {
                Self::no_effect(left) && Self::no_effect(right)
            }
            Expr::UnaryOp { operand, .. } => Self::no_effect(operand),
            Expr::TemplateString { parts } => parts.iter().all(|p| match p {
                crate::ast::TemplatePart::Literal(_) => true,
                crate::ast::TemplatePart::Interpolation(x) => Self::no_effect(x),
            }),
            Expr::Tuple(xs) => !xs.is_empty() && xs.iter().all(Self::no_effect),
            Expr::List(xs) => xs.iter().all(Self::no_effect),
            _ => false,
        }
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
            class: None,
        });
    }

    /// An advisory of a class `[check] promote` can name.
    fn advise(&mut self, class: &'static str, span: (u32, u32), message: String) {
        self.out.push(CheckDiagnostic {
            line: span.0,
            column: span.1,
            message,
            runtime: false,
            warning: true,
            scope: false,
            class: Some(class),
        });
    }

    /// Is `name` bound anywhere in scope (any type, including Unknown)?
    fn bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains_key(name)) || self.sigs.contains_key(name)
    }

    // ── native values: Series, Frame, Date (L1, L2) ─────────────────
    /// The native type a checker type is, when it is one whose methods
    /// and signatures `native_sig` knows (and the file declares no type
    /// of that name).
    fn native_kind(&self, t: &SType) -> Option<&'static str> {
        match t {
            SType::Named(n) if !self.structs.contains_key(n) => match n.as_str() {
                "Series" => Some("Series"),
                "Frame" => Some("Frame"),
                "Date" => Some("Date"),
                "GroupBy" => Some("GroupBy"),
                "Rng" => Some("Rng"),
                "HoltWinters" => Some("HoltWinters"),
                "Matrix" => Some("Matrix"),
                _ => None,
            },
            _ => None,
        }
    }

    /// The builtin a call names: `module.f(…)` on a stdlib module the file
    /// does not rebind, or `value.m(…)` on a native value with no `impl`
    /// of `m` in the file (its method, or Err with the type when it has
    /// none). `None` when the call is neither.
    fn native_target(&self, callee: &Expr) -> Option<Result<&'static str, &'static str>> {
        let Expr::FieldAccess { object, field } = callee else {
            return None;
        };
        if let Expr::Identifier(module) = object.as_ref()
            && !self.bound(module)
            && !self.user_bound.contains(module)
        {
            let full = format!("{}.{}", module, field);
            if let Some(s) = crate::native_sig::sig(&full) {
                return Some(Ok(s.full));
            }
        }
        let kind = self.native_kind(&self.infer(object))?;
        if self.methods.contains(field) {
            return None;
        }
        let home = crate::native_sig::home_of(kind)?;
        Some(crate::native_sig::method_target(home, kind, field).ok_or(kind))
    }

    /// `module.f(…)` on a stdlib module the file does not rebind.
    fn is_module_call(&self, callee: &Expr) -> bool {
        matches!(callee, Expr::FieldAccess { object, field }
            if matches!(object.as_ref(), Expr::Identifier(m)
                if !self.bound(m)
                    && !self.user_bound.contains(m)
                    && crate::native_sig::sig(&format!("{}.{}", m, field)).is_some()))
    }

    /// The native kind a call answers: `lead` is a piped value's kind
    /// (`Some` inside a pipeline).
    fn native_call_kind(
        &self,
        callee: &Expr,
        arguments: &[Argument],
        lead: Option<Option<&'static str>>,
    ) -> Option<&'static str> {
        let full = self.native_target(callee)?.ok()?;
        let method_receiver = match callee {
            Expr::FieldAccess { object, .. } if !self.is_module_call(callee) => {
                self.native_kind(&self.infer(object))
            }
            _ => None,
        };
        let first = match (method_receiver, lead) {
            (Some(k), _) => Some(k),
            (None, Some(k)) => k,
            (None, None) => match arguments.first() {
                Some(Argument::Positional(e)) => self.native_kind(&self.infer(e)),
                _ => None,
            },
        };
        // `ods.group_by(f, keys)` is a GroupBy; given its aggregations,
        // the Frame they make.
        if full == "ods.group_by" {
            let given = arguments.len()
                + usize::from(lead.is_some())
                + usize::from(method_receiver.is_some());
            return Some(if given >= 3 { "Frame" } else { "GroupBy" });
        }
        crate::native_sig::returns(full, first).filter(|k| *k != "Model")
    }

    /// A call of a native builtin or method: an unknown method, and names
    /// the signature does not take, raise at run time — said here first.
    fn check_native_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        lead: usize,
        span: (u32, u32),
    ) {
        let Some(target) = self.native_target(callee) else {
            return;
        };
        let full = match target {
            Ok(full) => full,
            Err(kind) => {
                if let Expr::FieldAccess { field, .. } = callee {
                    let message = crate::native_sig::no_method(kind, field);
                    self.diag(span, true, message);
                }
                return;
            }
        };
        let names: Vec<&str> = arguments
            .iter()
            .filter_map(|a| match a {
                Argument::Named { name, .. } => Some(name.as_str()),
                Argument::Positional(_) => None,
            })
            .collect();
        if names.is_empty()
            || arguments
                .iter()
                .skip_while(|a| matches!(a, Argument::Positional(_)))
                .any(|a| matches!(a, Argument::Positional(_)))
        {
            return;
        }
        // A method's receiver is its first argument.
        let receiver = usize::from(!self.is_module_call(callee));
        let n_pos = lead + receiver + arguments.len() - names.len();
        if let Err(message) = crate::native_sig::plan(full, n_pos, &names) {
            self.diag(span, true, message);
        }
    }

    // ── inference: conservative, Unknown-biased ─────────────────────
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
            // `()` is Unit, the absence value, not an empty tuple: a
            // binding set to it is a placeholder for a value assigned later
            // (`let mut fg = ()`, then a String), and comparing anything
            // with it is the presence test — so it has no type to judge by.
            Expr::Tuple(elems) if elems.is_empty() => SType::Unknown,
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
                self.native_call_kind(callee, arguments, None)
                    .map(|k| SType::Named(k.to_string()))
                    .unwrap_or(SType::Unknown)
            }
            Expr::Pipeline { left, right } => {
                let lead = self.infer(left);
                let lead = self.native_kind(&lead);
                let found = match right.as_ref() {
                    Expr::Call { callee, arguments } => {
                        self.native_call_kind(callee, arguments, Some(lead))
                    }
                    callee @ Expr::FieldAccess { .. } => {
                        self.native_call_kind(callee, &[], Some(lead))
                    }
                    _ => None,
                };
                found
                    .map(|k| SType::Named(k.to_string()))
                    .unwrap_or(SType::Unknown)
            }
            // A Frame's column (L6): `f.price`, unless `price` is a method.
            Expr::FieldAccess { object, field }
                if self.native_kind(&self.infer(object)) == Some("Frame")
                    && crate::native_sig::method_target("ods", "Frame", field).is_none() =>
            {
                SType::Named("Series".to_string())
            }
            Expr::Index { object, index }
                if self.native_kind(&self.infer(object)) == Some("Frame") =>
            {
                match index.as_ref() {
                    Expr::String(_) => SType::Named("Series".to_string()),
                    _ => SType::Unknown,
                }
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
                // A Series on either side of arithmetic or a comparison
                // answers a Series (elementwise; a comparison is a mask) —
                // `==` between two Series is the structural Bool.
                let series = |t: &SType| self.native_kind(t) == Some("Series");
                if (series(&l) || series(&r))
                    && !matches!(op, B::And | B::Or | B::Coalesce | B::MatMul)
                    && !(matches!(op, B::Equal | B::NotEqual) && series(&l) && series(&r))
                {
                    return SType::Named("Series".to_string());
                }
                match op {
                    B::Equal
                    | B::NotEqual
                    | B::LessThan
                    | B::LessThanEqual
                    | B::GreaterThan
                    | B::GreaterThanEqual
                    | B::And
                    | B::Or => SType::Bool,
                    // defined by native values alone
                    B::MatMul => SType::Unknown,
                    // the left's type when it is not `()`, else the right's
                    B::Coalesce => {
                        if l == r {
                            l
                        } else {
                            SType::Unknown
                        }
                    }
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
                let t = self.infer(operand);
                if self.native_kind(&t) == Some("Series") {
                    return t;
                }
                match (op, t) {
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

    // ── pass 2: the walk ────────────────────────────────────────────
    fn check_statement(&mut self, stmt: &Statement, span: (u32, u32)) {
        match stmt {
            Statement::Located { line, column, stmt } => {
                self.check_statement(stmt, (*line, *column));
            }
            Statement::Expression(e) => {
                self.warn_discarded_result(e, span);
                self.warn_unrebound_collection_write(e, span);
                self.warn_no_effect(e, span);
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
            // An annotated parameter is checked against its annotation,
            // which names the type it promises; one report, not two.
            if self
                .sigs
                .get(name)
                .and_then(|sig| sig.params.get(i))
                .is_some_and(|ty| *ty != SType::Unknown)
            {
                continue;
            }
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
        if let Expr::Call { callee, arguments } = expr {
            self.check_builtin_arity(callee, arguments, span);
            self.check_native_call(callee, arguments, 0, span);
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
                // `value.name(…)` may call a trait method: the field rule
                // is for reads, so the callee's object is walked without it.
                match callee.as_ref() {
                    Expr::FieldAccess { object, field } => {
                        self.check_expr(object, span);
                        self.check_module_member(object, field, true, span);
                        self.check_shape_key(object, field, "field", span);
                    }
                    _ => self.check_expr(callee, span),
                }
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
                // `gui.turn == ()`: asking whether this build has it.
                if matches!(
                    op,
                    crate::ast::BinaryOp::Equal | crate::ast::BinaryOp::NotEqual
                ) {
                    for (a, b) in [(left, right), (right, left)] {
                        if let (Expr::FieldAccess { object, field }, Expr::Tuple(t)) =
                            (a.as_ref(), b.as_ref())
                            && t.is_empty()
                            && let Expr::Identifier(m) = object.as_ref()
                        {
                            self.probed.insert((m.clone(), field.clone()));
                        }
                    }
                }
                self.check_expr(left, span);
                self.check_expr(right, span);
                // `==` across two provably different types is an answer the
                // program cannot want: always false (`!=` always true). Int
                // and Float compare numerically, so they are one kind here.
                use crate::ast::BinaryOp as B;
                // A native value defines its own `==` (a Series against a
                // scalar is a mask), so it is never "always false".
                let (lt, rt) = (self.infer(left), self.infer(right));
                if matches!(op, B::Equal | B::NotEqual)
                    && self.native_kind(&lt).is_none()
                    && self.native_kind(&rt).is_none()
                    && let (Some(a), Some(b)) = (lt.base_name(), rt.base_name())
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
                        class: None,
                    });
                }
            }
            Expr::Pipeline { left, right } => {
                self.check_expr(left, span);
                // The pipeline injects `left` as an argument of the
                // right-hand call, so that call's arity can't be judged
                // from its written arguments. Walk into it without the
                // call-level checks.
                if let Expr::Call { callee, arguments } = right.as_ref() {
                    self.check_native_call(callee, arguments, 1, span);
                }
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
                self.check_module_member(object, field, false, span);
                self.check_shape_key(object, field, "field", span);
                self.check_struct_field(object, field, span);
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
    run_tier_as(paths, false)
}

/// One function's static verdict (`olang check --tier --format json`).
#[cfg(feature = "native")]
struct TierVerdict {
    name: String,
    line: u32,
    col: u32,
    end_line: u32,
    /// `bytecode`, `refused`, `unchecked` (it names a top-level binding
    /// this pass did not evaluate), or `unknown` (the pass did not reach
    /// it: a nested or trait function).
    verdict: &'static str,
    reason: Option<String>,
    ambiguous: bool,
    pinned: Option<&'static str>,
}

/// `olang check --tier`, as text or (`json`) every function of each file
/// with its verdict. A function pinned native (`// studio: native`, or
/// `[check] native` in olang.toml) that the tier refuses is an error.
#[cfg(feature = "native")]
pub fn run_tier_as(paths: &[PathBuf], json: bool) -> i32 {
    let mut files = Vec::new();
    for path in paths {
        if !path.exists() {
            eprintln!("olang check: path not found: {}", path.display());
            return 2;
        }
        files.extend(super::discover_ol_files(path));
    }
    let mut refused_total = 0usize;
    let mut unchecked = 0usize;
    let mut promoted_errors = 0usize;
    let mut file_rows: Vec<serde_json::Value> = Vec::new();
    for file in &files {
        let Ok(source) = crate::notebook::read(file, crate::notebook::Mode::Run).map(|p| p.source)
        else {
            continue;
        };
        // the path as a run names it (absolute, not resolved through
        // symlinks), so a report and a run's statistics key the same file
        let absolute = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        let parser = crate::parser::Parser::new();
        let program = match parser.parse_with_dir(&source, file.parent()) {
            Ok(p) => p,
            Err(e) => {
                // A file that does not parse has no verdict; the text form
                // says so rather than counting it as compiling.
                if json {
                    file_rows.push(serde_json::json!({
                        "file": absolute.to_string_lossy(), "error": e.to_string(), "functions": [] }));
                } else {
                    println!("{}\n  × not checked: {}", file.display(), e);
                }
                unchecked += 1;
                continue;
            }
        };
        let facts = crate::tier_stats::FileFacts::of(&absolute, &source);
        // every top-level function, where it is
        let mut declared: Vec<(String, u32, u32, u32)> = Vec::new();
        let starts: Vec<u32> = program
            .statements
            .iter()
            .filter_map(|s| match s {
                Statement::Located { line, .. } => Some(*line),
                _ => None,
            })
            .collect();
        for statement in &program.statements {
            let line = match statement {
                Statement::Located { line, .. } => *line,
                _ => 0,
            };
            if let Statement::FunctionDecl(f)
            | Statement::ShareDecl(crate::ast::ShareDecl::Function(f)) = statement.unwrapped()
            {
                let (l, c) = f.name_span.unwrap_or((line, 1));
                let end = starts
                    .iter()
                    .copied()
                    .filter(|s| *s > l)
                    .min()
                    .map(|s| s - 1)
                    .unwrap_or(source.lines().count() as u32)
                    .max(l);
                declared.push((f.name.clone(), l, c, end));
            }
        }
        let mut interpreter = crate::interpreter::Interpreter::new();
        interpreter.enable_bytecode_tier(1, std::env::var_os("OLANG_TIER_VERBOSE").is_some());
        interpreter.set_current_file(&absolute);
        // The dependencies this machine holds, fetched from nowhere (a
        // check reads a project; it does not install one): one missing is
        // said where a `use` names it, as the file's "not checked" reason.
        if let Some(root) = crate::pkg::manifest::Manifest::find_root(&absolute) {
            let opts = crate::pkg::InstallOptions {
                registry: std::env::var("OLANG_REGISTRY").ok().map(PathBuf::from),
                offline: true,
                ..Default::default()
            };
            let (map, missing) = crate::pkg::install_lenient(&root, &opts);
            interpreter.set_missing_dependencies(missing.into_iter().collect());
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
        // The declarations load without effects: a module the file uses
        // runs its top level as it loads, and `check --tier` on a file
        // opened (olang Studio's tier map does, on every file shown) ran a
        // module's `os.exec` with the person's privileges. They load with
        // no capability (no fs, net, proc, db, env; a module loaded at run
        // time is held to that too), then the table the check had is put
        // back. A module that needs an effect to load is "not checked".
        let none = crate::caps::Caps::none();
        let was =
            interpreter.replace_capabilities(Some(std::sync::Arc::new(crate::caps::CapTable {
                app: none,
                deps: Vec::new(),
                fixed: false,
            })));
        let loaded = interpreter.eval_program(Program {
            statements: declarations,
        });
        interpreter.restore_capabilities(was);
        if let Err(e) = loaded {
            if json {
                file_rows.push(serde_json::json!({
                    "file": absolute.to_string_lossy(), "error": e.to_string(), "functions": [] }));
            } else {
                println!(
                    "{}\n  ⚠ not checked (its declarations did not load): {}",
                    file.display(),
                    e
                );
            }
            continue;
        }
        let file_key = absolute.to_string_lossy().to_string();
        interpreter.note_root_as_module_scope(&file_key);
        let all_refusals = interpreter.tier_compile_ahead(Some(&file_key));
        let (compiled, ambiguous) = interpreter.tier_compiled();
        let names_skipped = |reason: &str| {
            skipped
                .iter()
                .any(|name| reason.contains(&format!("'{name}'")))
        };
        let refusals: Vec<(String, String)> = all_refusals
            .iter()
            .filter(|(_, reason)| !names_skipped(reason))
            .cloned()
            .collect();
        let verdicts: Vec<TierVerdict> = declared
            .iter()
            .map(|(name, line, col, end_line)| {
                let refusal = all_refusals.iter().find(|(n, _)| n == name);
                let (verdict, reason) = match refusal {
                    Some((_, r)) if names_skipped(r) => ("unchecked", Some(r.clone())),
                    Some((_, r)) => ("refused", Some(r.clone())),
                    None if compiled.contains(name) || ambiguous.contains(name) => {
                        ("bytecode", None)
                    }
                    None => ("unknown", None),
                };
                TierVerdict {
                    name: name.clone(),
                    line: *line,
                    col: *col,
                    end_line: *end_line,
                    verdict,
                    reason,
                    ambiguous: ambiguous.contains(name),
                    pinned: facts.pin(name),
                }
            })
            .collect();
        // `// allow: tier` on a function's `fn` line: its refusal is not
        // reported, unless the function is pinned native (a pin says it
        // must compile; the allow cannot unsay it)
        let allows = crate::tools::allow::scan(&source);
        let allowed = |name: &str| {
            facts.pin(name).is_none()
                && declared
                    .iter()
                    .find(|(n, ..)| n == name)
                    .is_some_and(|(_, l, ..)| allows.allows(*l, "tier"))
        };
        let refusals: Vec<(String, String)> =
            refusals.into_iter().filter(|(n, _)| !allowed(n)).collect();
        let pinned_refused: Vec<&TierVerdict> = verdicts
            .iter()
            .filter(|v| v.pinned.is_some() && v.verdict == "refused")
            .collect();
        let promoted = promotions_for(file.parent())
            .iter()
            .any(|p| p == "tier" || p == "all");
        refused_total += refusals.len();
        if promoted {
            promoted_errors += refusals.len();
        } else {
            promoted_errors += pinned_refused.len();
        }
        if json {
            let rows: Vec<serde_json::Value> = verdicts
                .iter()
                .map(|v| {
                    let code = v.reason.as_deref().map(crate::tier_stats::reason_code);
                    serde_json::json!({
                        "name": v.name, "line": v.line, "col": v.col, "end_line": v.end_line,
                        "verdict": v.verdict, "reason": v.reason, "code": code,
                        "docs": code.map(crate::tier_stats::docs_ref),
                        "ambiguous": v.ambiguous, "pinned": v.pinned,
                        "allowed": v.verdict == "refused" && allowed(&v.name),
                        "error": v.verdict == "refused" && !allowed(&v.name) && (promoted || v.pinned.is_some()),
                    })
                })
                .collect();
            file_rows.push(serde_json::json!({
                "file": file_key, "error": null, "functions": rows }));
            continue;
        }
        if refusals.is_empty() {
            continue;
        }
        println!("{}", file.display());
        for (name, reason) in &refusals {
            let pinned = verdicts
                .iter()
                .find(|v| &v.name == name)
                .and_then(|v| v.pinned);
            println!(
                "  {} `{}` {}stays on the tree-walker: {}",
                if promoted || pinned.is_some() {
                    "×"
                } else {
                    "⚠"
                },
                name,
                match pinned {
                    Some("comment") => "is pinned native (// studio: native) but ",
                    Some(_) => "is pinned native ([check] native) but ",
                    None => "",
                },
                reason
            );
        }
    }
    if json {
        println!(
            "{}",
            serde_json::json!({
                "format": TIER_CHECK_FORMAT,
                "kind": "tier-check",
                "olang": crate::version::VERSION,
                "files": file_rows,
                "refused": refused_total,
                "errors": promoted_errors,
            })
        );
        return if promoted_errors > 0 { 1 } else { 0 };
    }
    if unchecked > 0 {
        println!(
            "olang check --tier: {} file{} not checked (it does not parse)",
            unchecked,
            if unchecked == 1 { "" } else { "s" }
        );
        return 1;
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
                " (an error here: [check] promote names `tier`, or a function is pinned native)"
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
    fn a_field_a_declared_struct_never_has_is_reported() {
        let d = check(
            "type Point = struct { x: Int, y: Int }\ntype Task = { title: String }\nfn a(p: Point) = p.z\nfn b(p: Point) = p.x + p.y\nfn c() = Point { x: 1, y: 2 }.k\nfn e(t: Task) = t.titel\nfn f(q) = q.anything\n",
        );
        let messages: Vec<&str> = d.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages.len(), 3, "{messages:?}");
        assert!(messages[0].starts_with(
            "`z` is not a field of the declared struct Point (x, y): reading it raises at run time"
        ));
        assert!(messages[1].starts_with("`k` is not a field of the declared struct Point"));
        assert!(messages[2].starts_with("`titel` is not a key of the declared shape"));
        assert!(messages[2].contains("the read is () at run time"));
        assert!(d.iter().all(|d| d.warning && d.class == Some("shape")));
        // A trait method of the name, called or read, is not a field.
        let d = check(
            "type Point = struct { x: Int }\ntrait Norm { fn norm(self) }\nimpl Norm for Point { fn norm(self) = self.x }\nfn a(p: Point) = p.norm()\nfn b(p: Point) = p.norm\n",
        );
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn a_wrong_argument_count_to_a_builtin_is_reported() {
        let d = check("println(str.thousands(1234))\n");
        assert_eq!(d.len(), 1, "{:?}", d);
        assert!(d[0].runtime && !d[0].warning);
        assert!(
            d[0].message.contains("str.thousands: missing argument 2"),
            "{}",
            d[0].message
        );
        let d = check("let r = sort([1], 2)\nlet n = len()\nlet x = range(1, 2, 3, 4)\n");
        let messages: Vec<&str> = d.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "Arity mismatch: expected 1, got 2",
                "Arity mismatch: expected 1, got 0",
                "`range` takes 1 to 3 arguments, got 4"
            ]
        );
        // Variadic forms, a pipeline's injected argument, and a name the
        // file binds itself are not judged.
        assert!(check("let a = min(1, 2, 3)\nlet b = str.fmt(\"{} {}\", 1, 2)\nlet c = [3, 1] |> sort\nlet d = [1] |> take(1)\nlet e = fs.join(\"a\", \"b\", \"c\")\n").is_empty());
        assert!(check("fn sort(a, b) = a\nlet r = sort(1, 2)\n").is_empty());
        assert!(
            check("fn f(str) = str.thousands(1)\n")
                .iter()
                .all(|d| d.warning)
        );
        assert!(check("use colx { * }\nlet r = all([1])\n").is_empty());
    }

    #[test]
    fn the_global_arity_table_agrees_with_the_runtime() {
        // Every entry's bounds are the runtime's: one short and one over
        // (where there is an over) raise; the bounds themselves do not
        // raise an arity error.
        let names = crate::builtin::BuiltinFunctions::global_names();
        for (name, min, max) in GLOBAL_ARITY {
            assert!(
                names.iter().any(|n| n == name),
                "{name} is a global builtin"
            );
            // (raised at all, raised over the count)
            let call = |k: usize| {
                let args = vec!["()"; k].join(", ");
                let src = format!("show(attempt(() => {name}({args})))");
                let program = Parser::new().parse(&src).expect("parses");
                let mut interpreter = crate::Interpreter::new();
                let v = interpreter
                    .eval_program(program)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|e| e.to_string());
                let count =
                    v.contains("Arity mismatch") || v.contains("missing") || v.contains("expects");
                (
                    v.trim_start_matches('"').starts_with("Err(") || count,
                    count,
                )
            };
            assert!(call(min - 1).0, "{name} with {} arguments", min - 1);
            assert!(!call(*min).1, "{name} with {min} arguments");
            if *max != usize::MAX {
                assert!(!call(*max).1, "{name} with {max} arguments");
                assert!(call(max + 1).1, "{name} with {} arguments", max + 1);
            }
        }
    }

    #[test]
    fn a_parameter_or_pattern_shadowing_a_stdlib_module_is_reported() {
        let program = Parser::new()
            .parse(
                "fn f(str) = str.length(\"a\")\nfn table_sort(rows, col) = col.sort_by(rows, (r) => r)\nfn ok(col, time) = col + time\nlet h = (json) => json.parse(\"1\")\nfn g(p) = match p { (math, _) => math.sqrt(4.0), _ => 0 }\n",
            )
            .expect("parses");
        let d = binding_shadow_warnings(&program);
        let messages: Vec<&str> = d.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(d.len(), 4, "{messages:?}");
        assert!(messages[0].starts_with("parameter `str` of `f` shadows the stdlib module `str`"));
        assert!(messages[1].contains("`col.sort_by`"));
        assert!(messages[2].starts_with("the lambda parameter `json`"));
        assert!(messages[3].starts_with("the pattern binding `math`"));
        assert!(d.iter().all(|d| d.warning && d.class == Some("shadow")));
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
    fn native_methods_and_names_are_checked() {
        let src = "let s = ods.series([1.0])\nlet f = ods.frame([[\"a\", s]])\n";
        // a method the value lacks, followed through bindings and chains
        let d = check(&format!("{src}s.nope()\nf.a.fill_null(0.0).nopes()\n"));
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d[0].message.contains("a Series has no method 'nope'"));
        // `use ods` binds the module itself, so its calls are still checked
        let d = check(&format!(
            "use ods\n{src}s.clip(lo: 0.0).nope()\ns.round(digts: 1)\n"
        ));
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d[1].message.contains("it takes s, digits"));
        // names the signature does not take; a pipeline's left counts
        let d = check(&format!(
            "{src}ods.rolling(s, windw: 2, agg: \"m\")\ns |> ods.rolling(window: 2, agg: \"m\")\nf.sort_by(\"a\", descending: true)\n"
        ));
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].message.contains("no parameter named 'windw'"));
        // an impl of the name, or a rebound module, is the program's own
        let d = check(&format!(
            "{src}trait T {{ fn nope(self) }}\nimpl T for Series {{ fn nope(self) = 1 }}\ns.nope()\n"
        ));
        assert!(d.is_empty(), "{d:?}");
        let d = check("let ods = #{ \"rolling\": (s, w) => s }\nods.rolling(1, w: 2)\n");
        assert!(
            d.iter().all(|x| !x.message.contains("no parameter")),
            "{d:?}"
        );
        // a group_by without aggregations is a GroupBy, with them a Frame
        let d = check(&format!(
            "{src}let by = f.group_by(\"a\")\nby.agg(#{{}})\nby.agx()\nby.mean(q: 1)\n\
             ods.group_by(f, \"a\", #{{}}).sort_by(\"a\")\n(f |> ods.group_by(\"a\")).size()\n"
        ));
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(
            d[0].message.contains("a GroupBy has no method 'agx'"),
            "{d:?}"
        );
        assert!(d[1].message.contains("no parameter named 'q'"), "{d:?}");
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

    // ── stage 3: element types ──────────────────────────────────────
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

    // ── 0.61: `let` is required, and `mut` is a guarantee ───────────
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

    // ── 0.50 arc: cross-file context ────────────────────────────────
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

    #[test]
    fn a_namespace_alone_leaves_a_builtin_its_name() {
        // `use gg` (a stdlib module: its namespace alone) beside
        // `use web { el }`: the SDK's `find(routes, method, path)` was
        // not imported, so `find(xs, f)` is the builtin.
        let web = Parser::new()
            .parse("share fn find(routes, method, path) = routes\nshare fn el(tag) = tag\n")
            .expect("parses");
        let main = Parser::new()
            .parse("use gg\nuse web { el }\nprintln(show(find([1, 2, 3], (r) => r > 1)))\n")
            .expect("parses");
        let d = check_program_with_context(&[&web], &main);
        assert!(
            d.is_empty(),
            "{:?}",
            d.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
        // Imported by name, it is the SDK's, and checked as such.
        let main = Parser::new()
            .parse("use gg\nuse web { find }\nfind([1, 2, 3], 1)\n")
            .expect("parses");
        let d = check_program_with_context(&[&web], &main);
        assert!(
            d.iter()
                .any(|x| x.message.contains("Missing required argument: path"))
        );
    }

    // ── 0.61: blocks scope their bindings ───────────────────────────
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

    // ── 0.50 arc: literal types ─────────────────────────────────────
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

    // ── 0.50 arc: hover types ───────────────────────────────────────
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

    // ── 0.50 arc: Promise at non-async sites ────────────────────────
    #[test]
    fn a_task_handle_is_an_ordinary_value_to_the_checker() {
        // 0.63 deleted async/await/Promise and 0.65 deleted try/catch;
        // nothing about concurrency or error handling needs a special
        // case in the checker any more. A task handle has no type of its
        // own here, and no unwrapping rule to get wrong.
        assert!(check("fn go() = 1\nlet t = spawn go()\nlet v = task.join(t)\n").is_empty());
        // `try` is just a name; `async` and `await` are reserved since
        // the freeze pass, for a form the language may add.
        assert!(check("let try = 3\ntry + 1\n").is_empty());
        assert!(Parser::new().parse("let async = 1\n").is_err());
    }

    // ── stage 4: Result payloads ────────────────────────────────────
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

    // ── 0.50 arc: unions ────────────────────────────────────────────
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

    // ── 0.50 arc: function types ────────────────────────────────────
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

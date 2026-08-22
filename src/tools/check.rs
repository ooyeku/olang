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
        for d in check_program_with_context(&context, &program) {
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

/// The modules a source file `use`s, resolved with the runtime's local
/// conventions (`use a.b` → a/b.ol, a/b/index.ol, a/b/mod.ol, or b.ol in
/// the same directory), read and parsed. Failures are silently skipped —
/// checking degrades to single-file. Capped to bound cost.
pub fn module_programs(
    text: &str,
    doc_dir: Option<&std::path::Path>,
) -> Vec<(PathBuf, String, Program)> {
    let Some(dir) = doc_dir else {
        return Vec::new();
    };
    let Ok(program) = crate::parser::Parser::new().parse(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for stmt in &program.statements {
        if out.len() >= 16 {
            break;
        }
        let (Statement::UseDecl(u) | Statement::ShareDecl(crate::ast::ShareDecl::Use(u))) =
            stmt.unwrapped()
        else {
            continue;
        };
        if u.path.is_empty() {
            continue;
        }
        let joined = u.path.join("/");
        let last = u.path.last().cloned().unwrap_or_default();
        let candidates = [
            dir.join(format!("{}.ol", joined)),
            dir.join(&joined).join("index.ol"),
            dir.join(&joined).join("mod.ol"),
            dir.join(format!("{}.ol", last)),
        ];
        for c in candidates {
            if out.iter().any(|(p, _, _)| *p == c) {
                break;
            }
            if let Ok(src) = std::fs::read_to_string(&c) {
                if let Ok(prog) = crate::parser::Parser::new().parse(&src) {
                    out.push((c, src, prog));
                }
                break;
            }
        }
    }
    out
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
                SType::Named(name.clone())
            }
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
            other => other.base_name().unwrap_or("?").to_string(),
        }
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

/// Check `program` with signatures collected from `context` first — the
/// modules a file `use`s, resolved and parsed by the caller (the LSP).
/// Only `program`'s statements are walked; context contributes function
/// signatures and struct shapes.
pub fn check_program_with_context(context: &[&Program], program: &Program) -> Vec<CheckDiagnostic> {
    let mut checker = Checker::default();
    for p in context {
        checker.collect(p);
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
    structs: HashMap<String, Vec<(String, SType)>>,
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
            Expr::Identifier(name) | Expr::LocalRef { name, .. } => self.lookup(name),
            Expr::Call { callee, .. } => {
                if let Expr::Identifier(name) = callee.as_ref()
                    && let Some(sig) = self.sigs.get(name)
                {
                    return sig.ret.clone();
                }
                SType::Unknown
            }
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

    fn check_expr(&mut self, expr: &Expr, span: (u32, u32)) {
        // Check this node, then walk its children.
        if let Expr::Call { callee, arguments } = expr
            && let Expr::Identifier(name) = callee.as_ref()
            // Named arguments change binding order; stay silent on them.
            && arguments
                .iter()
                .all(|a| matches!(a, Argument::Positional(_)))
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
            Expr::BinaryOp { left, right, .. } => {
                self.check_expr(left, span);
                self.check_expr(right, span);
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
                self.check_expr(body, span);
                self.pop_scope(Merge::Construct);
            }
            Expr::WhileLoop { condition, body } => {
                self.check_expr(condition, span);
                self.check_expr(body, span);
            }
            Expr::Loop { body } => self.check_expr(body, span),
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
            Expr::FieldAccess { object, .. } => self.check_expr(object, span),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn check(src: &str) -> Vec<CheckDiagnostic> {
        check_program(&Parser::new().parse(src).expect("parses"))
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
        // Parameters and loop variables are immutable.
        let d = check("fn f(x) = { x = x + 1\n x }\n");
        assert_eq!(d.len(), 1, "{d:?}");
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

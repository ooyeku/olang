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

use crate::ast::{Argument, Expr, Program, Statement, TypeAnnotation, TypeDefinition};
use std::collections::HashMap;
use std::path::PathBuf;

/// `olang check [paths]` — parse every `.ol` file and report provable
/// annotation violations. Exit 0 when everything is clean (or unknowable),
/// 1 when a violation or parse error is found.
pub fn run(paths: &[PathBuf]) -> i32 {
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
        let program = match parser.parse(&source) {
            Ok(p) => p,
            Err(e) => {
                // A file that doesn't parse can't run either; that counts.
                eprintln!("{}: {}", file.display(), e);
                problems += 1;
                continue;
            }
        };
        for d in check_program(&program) {
            if d.warning {
                warnings += 1;
            } else {
                problems += 1;
            }
            let offset = byte_offset_of(&source, d.line as usize, d.column as usize);
            let label = if d.warning {
                "advisory — the program still runs"
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
    /// `Promise<T, E>`. Base-checked like a named type at rest; `await`
    /// unwraps `T`.
    Promise(Box<SType>, Box<SType>),
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
            TypeAnnotation::Promise {
                value_type,
                error_type,
            } => SType::Promise(
                Box::new(SType::from_annotation(value_type, type_params)),
                Box::new(
                    error_type
                        .as_ref()
                        .map(|t| SType::from_annotation(t, type_params))
                        .unwrap_or(SType::Unknown),
                ),
            ),
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
                "Promise" => SType::Promise(
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
            SType::Promise(_, _) => Some("Promise"),
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
            SType::Promise(v, e) if **v == SType::Unknown && **e == SType::Unknown => {
                "Promise".to_string()
            }
            SType::Promise(v, e) => format!("Promise<{}, {}>", v.display(), e.display()),
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
        (SType::Promise(v1, e1), SType::Promise(v2, e2)) => {
            violation(v1, v2).is_some() || violation(e1, e2).is_some()
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
    let mut checker = Checker::default();
    checker.collect(program);
    checker.push_scope();
    for stmt in &program.statements {
        checker.check_statement(stmt, (0, 0));
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
            match stmt.unwrapped() {
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
                Statement::AsyncFunctionDecl(f) => {
                    // Calling an async fn yields a Promise; its annotation
                    // describes the resolved value (a bare `-> T` wraps).
                    let resolved = f
                        .return_type
                        .as_ref()
                        .map(|a| SType::from_annotation(a, &f.type_params))
                        .unwrap_or(SType::Unknown);
                    let ret = match resolved {
                        p @ SType::Promise(_, _) => p,
                        other => SType::Promise(Box::new(other), Box::new(SType::Unknown)),
                    };
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
                            required: f
                                .parameters
                                .iter()
                                .filter(|p| p.default_value.is_none())
                                .count(),
                            total: f.parameters.len(),
                            ret,
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

    fn diag(&mut self, span: (u32, u32), runtime: bool, message: String) {
        self.out.push(CheckDiagnostic {
            line: span.0,
            column: span.1,
            message,
            runtime,
            warning: false,
        });
    }

    fn warn(&mut self, span: (u32, u32), message: String) {
        self.out.push(CheckDiagnostic {
            line: span.0,
            column: span.1,
            message,
            runtime: false,
            warning: true,
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
            // `await` unwraps the resolved payload.
            Expr::Await { expression } => match self.infer(expression) {
                SType::Promise(v, _) => *v,
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
            Statement::Expression(e) => self.check_expr(e, span),
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
                // Stability has long said assignment-to-undeclared "may
                // warn in a future release"; this is the release. Advisory
                // only — the runtime still creates the binding.
                if !self.bound(target) {
                    if self.leaked.iter().any(|l| l.contains(target)) {
                        if self.warned_leaks.insert(target.clone()) {
                            self.warn(
                                span,
                                format!(
                                    "'{}' is declared inside a block and is only visible here because blocks don't scope yet — declare it before the block",
                                    target
                                ),
                            );
                        }
                    } else {
                        self.warn(
                            span,
                            format!(
                                "assignment to undeclared name '{}' creates a binding — declare it with `let {} = ...`",
                                target, target
                            ),
                        );
                    }
                }
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
                for s in stmts {
                    self.check_statement(s, span);
                }
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
            Expr::TryCatch {
                try_block,
                catch_var,
                catch_block,
            } => {
                self.check_expr(try_block, span);
                self.push_scope();
                self.bind(&catch_var.clone(), SType::Unknown);
                self.check_expr(catch_block, span);
                self.pop_scope(Merge::Construct);
            }
            Expr::Identifier(name) | Expr::LocalRef { name, .. } => {
                // Using a name that only exists because a block leaked it:
                // the book says to write as if blocks scoped, and a future
                // release may tighten this. Advisory, once per name.
                if !self.bound(name)
                    && self.leaked.iter().any(|l| l.contains(name))
                    && self.warned_leaks.insert(name.clone())
                {
                    self.warn(
                        span,
                        format!(
                            "'{}' is declared inside a block and is only visible here because blocks don't scope yet — declare it before the block",
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

    // ── 0.50 arc: undeclared-assignment warning ────────────────────────

    #[test]
    fn undeclared_assignment_warns_declared_does_not() {
        let d = check("count = 1\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].warning, "advisory, not an error");
        assert!(
            d[0].message
                .contains("assignment to undeclared name 'count'")
        );
        // Declared names, loop vars, params, and valueless lets are quiet.
        assert!(check("let ok = 1\nok = 2\n").is_empty());
        assert!(check("let pending\npending = 1\n").is_empty());
        assert!(check("fn f(x) = { x = x + 1\n x }\n").is_empty());
        assert!(check("for x in [1] { x = x + 1 }\n").is_empty());
        // One warning per name: the first assignment binds it.
        assert_eq!(check("n = 1\nn = 2\n").len(), 1);
        // The assigned value's type flows onward.
        let d = check("fn f(x: Int) = x\ns = \"str\"\nf(s)\n");
        assert_eq!(d.len(), 2, "warning plus the type violation: {:?}", d);
        assert!(d.iter().any(|x| x.warning));
        assert!(
            d.iter()
                .any(|x| !x.warning && x.message.contains("expects Int, got String"))
        );
    }

    // ── 0.50 arc: block-scoping warning ────────────────────────────────

    #[test]
    fn using_a_block_leaked_binding_warns_once() {
        let d = check(
            "{\n    let inner = 1\n}\nprintln(to_string(inner))\nprintln(to_string(inner))\n",
        );
        assert_eq!(d.len(), 1, "once per name: {:?}", d);
        assert!(d[0].warning);
        assert!(d[0].message.contains("'inner' is declared inside a block"));
        // Assignment to a leaked name draws the block warning, not the
        // undeclared-assignment one.
        let d = check("{\n    let n = 1\n}\nn = 2\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("declared inside a block"));
    }

    #[test]
    fn block_leaks_stop_at_function_boundaries() {
        // A leak inside one function must not taint another.
        assert!(check("fn a() = {\n    { let x = 1 }\n    0\n}\nfn b(x) = x\nb(1)\n").is_empty());
        // Declaring before the block is the fix and stays silent.
        assert!(
            check("let outer = 0\n{\n    outer = 1\n}\nprintln(to_string(outer))\n").is_empty()
        );
        // Nested blocks leak transitively.
        let d = check("{\n    { let deep = 1 }\n}\nprintln(to_string(deep))\n");
        assert_eq!(d.len(), 1);
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
    fn promise_annotations_base_check_and_await_unwraps() {
        // Base check at rest.
        let d = check("fn f(p: Promise<Int>) = p\nf(42)\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].message, "parameter 'p' of f expects Promise, got Int");
        assert!(d[0].runtime);
        // await carries the resolved type into the flow; a bare -> T on an
        // async fn wraps into Promise<T, _>.
        let d = check(
            "async fn get() -> Promise<Int, String> = { 1 }\nfn g(x: String) = x\nlet v = await get()\ng(v)\n",
        );
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message
                .contains("parameter 'x' of g expects String, got Int")
        );
        // The unawaited call is a Promise, not the payload.
        let d =
            check("async fn get() -> Promise<Int, String> = { 1 }\nfn h(x: Int) = x\nh(get())\n");
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message
                .contains("parameter 'x' of h expects Int, got Promise")
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

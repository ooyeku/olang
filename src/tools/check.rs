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
            problems += 1;
            let offset = byte_offset_of(&source, d.line as usize, d.column as usize);
            let label = if d.runtime {
                "this would fail at runtime"
            } else {
                "the annotation's promise is broken here"
            };
            let diagnostic = miette::MietteDiagnostic::new(d.message.clone())
                .with_label(miette::LabeledSpan::at_offset(offset, label));
            let report = miette::Report::new(diagnostic).with_source_code(
                miette::NamedSource::new(file.display().to_string(), source.clone()),
            );
            eprintln!("{:?}", report);
        }
    }

    if problems == 0 {
        println!(
            "olang check: {} file{} clean",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        );
        0
    } else {
        eprintln!(
            "olang check: {} problem{} found",
            problems,
            if problems == 1 { "" } else { "s" }
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
        _ => SType::Unknown,
    }
}

/// A provable disagreement between a declared type and a known actual
/// type: `(expected_text, actual_text, runtime_would_reject)`. None
/// wherever anything is Unknown or the types agree.
fn violation(expected: &SType, actual: &SType) -> Option<(String, String, bool)> {
    let be = expected.base_name()?;
    let ba = actual.base_name()?;
    if be != ba {
        // The runtime's shallow check fails too; use its vocabulary.
        return Some((be.to_string(), ba.to_string(), true));
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
    checker.scopes.push(HashMap::new());
    for stmt in &program.statements {
        checker.check_statement(stmt, (0, 0));
    }
    checker.out
}

#[derive(Default)]
struct Checker {
    sigs: HashMap<String, FnSig>,
    structs: HashMap<String, Vec<(String, SType)>>,
    scopes: Vec<HashMap<String, SType>>,
    out: Vec<CheckDiagnostic>,
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
        });
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
            // Everything else — blocks, pipelines, lambdas, field access,
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
        self.check_against_at(expr, expected, span, site, false)
    }

    fn check_against_at(
        &mut self,
        expr: &Expr,
        expected: &SType,
        span: (u32, u32),
        site: &str,
        elem: bool,
    ) {
        if *expected == SType::Unknown {
            return;
        }
        match (expr, expected) {
            (Expr::List(elems), SType::List(t)) => {
                for (i, e) in elems.iter().enumerate() {
                    self.check_against_at(e, t, span, &format!("element {} of {}", i, site), true);
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
                    );
                    let value_site = match &entry.key {
                        Expr::String(s) => format!("value for key \"{}\" of {}", s, site),
                        _ => format!("map value {} of {}", i, site),
                    };
                    self.check_against_at(&entry.value, v, span, &value_site, true);
                }
            }
            _ => {
                let actual = self.infer(expr);
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
                self.scopes.push(HashMap::new());
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
                self.scopes.pop();
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
            Expr::Block(stmts) => {
                self.scopes.push(HashMap::new());
                for s in stmts {
                    self.check_statement(s, span);
                }
                self.scopes.pop();
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
                self.scopes.push(HashMap::new());
                self.bind(&variable.clone(), SType::Unknown);
                self.check_expr(body, span);
                self.scopes.pop();
            }
            Expr::WhileLoop { condition, body } => {
                self.check_expr(condition, span);
                self.check_expr(body, span);
            }
            Expr::Loop { body } => self.check_expr(body, span),
            Expr::Lambda {
                parameters, body, ..
            } => {
                self.scopes.push(HashMap::new());
                for p in parameters {
                    let ty = p
                        .type_annotation
                        .as_ref()
                        .map(|a| SType::from_annotation(a, &[]))
                        .unwrap_or(SType::Unknown);
                    self.bind(&p.name.clone(), ty);
                }
                self.check_expr(body, span);
                self.scopes.pop();
            }
            Expr::Match { value, arms } => {
                self.check_expr(value, span);
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    self.bind_pattern_unknown(&arm.pattern.clone());
                    self.check_expr(&arm.expression, span);
                    self.scopes.pop();
                }
            }
            Expr::TryCatch {
                try_block,
                catch_var,
                catch_block,
            } => {
                self.check_expr(try_block, span);
                self.scopes.push(HashMap::new());
                self.bind(&catch_var.clone(), SType::Unknown);
                self.check_expr(catch_block, span);
                self.scopes.pop();
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
}

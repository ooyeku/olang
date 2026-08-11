//! The static side of gradual typing: report annotation violations that
//! are *provable* before the program runs.
//!
//! The runtime is the authority — every annotation is already enforced at
//! its boundary on every tier. This pass exists to move the same failures
//! earlier: to `olang check` and the editor, at the moment the mistake is
//! typed. Its discipline is **no false positives**: a violation is only
//! reported when the checker can prove the runtime would reject it — a
//! literal argument against an annotated parameter, an annotated binding
//! initialized with a known-type value, a declared return contradicted by
//! what the body provably produces. Everything it cannot prove is silent;
//! dynamic code stays unjudged. The messages are the runtime's own, so
//! the diagnostic you see in the editor is the error you would have hit.

use crate::ast::{
    Argument, Expr, FieldTypeCheck, Program, Statement, param_checks_of, return_check_of,
};
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
            let diagnostic = miette::MietteDiagnostic::new(d.message.clone()).with_label(
                miette::LabeledSpan::at_offset(offset, "this would fail at runtime"),
            );
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
}

/// The checker's knowledge of an expression's type. `Unknown` is the
/// gradual escape hatch: anything dynamic, inferred-ambiguous, or simply
/// not modeled stays Unknown and is never reported.
#[derive(Debug, Clone, PartialEq)]
enum SType {
    Known(FieldTypeCheck),
    Unknown,
}

impl SType {
    fn name(&self) -> Option<&str> {
        match self {
            SType::Known(c) => Some(c.expected_name()),
            SType::Unknown => None,
        }
    }
}

/// A known function's checkable surface.
struct FnSig {
    param_names: Vec<String>,
    param_checks: Vec<Option<FieldTypeCheck>>,
    /// Parameters without default values — the arity floor.
    required: usize,
    total: usize,
    ret: Option<FieldTypeCheck>,
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
    scopes: Vec<HashMap<String, SType>>,
    out: Vec<CheckDiagnostic>,
}

impl Checker {
    // ── pass 1: signatures ─────────────────────────────────────────────

    fn collect(&mut self, program: &Program) {
        for stmt in &program.statements {
            if let Statement::FunctionDecl(f) = stmt.unwrapped() {
                let required = f
                    .parameters
                    .iter()
                    .filter(|p| p.default_value.is_none())
                    .count();
                self.sigs.insert(
                    f.name.clone(),
                    FnSig {
                        param_names: f.parameters.iter().map(|p| p.name.clone()).collect(),
                        param_checks: param_checks_of(&f.parameters, &f.type_params),
                        required,
                        total: f.parameters.len(),
                        ret: return_check_of(f.return_type.as_ref(), &f.type_params),
                    },
                );
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

    // ── inference: conservative, Unknown-biased ────────────────────────

    fn infer(&self, expr: &Expr) -> SType {
        match expr {
            Expr::Integer(_) => SType::Known(FieldTypeCheck::Int),
            Expr::Float(_) => SType::Known(FieldTypeCheck::Float),
            Expr::String(_) => SType::Known(FieldTypeCheck::String),
            Expr::Boolean(_) => SType::Known(FieldTypeCheck::Bool),
            Expr::List(_) => SType::Known(FieldTypeCheck::List),
            Expr::MapLiteral { .. } => SType::Known(FieldTypeCheck::Map),
            Expr::Tuple(_) => SType::Known(FieldTypeCheck::Tuple),
            Expr::StructLiteral(lit) => SType::Known(FieldTypeCheck::Named(lit.type_name.clone())),
            Expr::Identifier(name) | Expr::LocalRef { name, .. } => self.lookup(name),
            Expr::Call { callee, .. } => {
                if let Expr::Identifier(name) = callee.as_ref()
                    && let Some(sig) = self.sigs.get(name)
                {
                    return match &sig.ret {
                        Some(c) => SType::Known(c.clone()),
                        None => SType::Unknown,
                    };
                }
                SType::Unknown
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
                    | B::Or => SType::Known(FieldTypeCheck::Bool),
                    B::Add | B::Subtract | B::Multiply | B::Divide | B::Modulo => {
                        match (l.name(), r.name()) {
                            (Some("Int"), Some("Int")) => SType::Known(FieldTypeCheck::Int),
                            (Some("Float"), Some("Int"))
                            | (Some("Int"), Some("Float"))
                            | (Some("Float"), Some("Float")) => SType::Known(FieldTypeCheck::Float),
                            (Some("String"), Some("String")) if matches!(op, B::Add) => {
                                SType::Known(FieldTypeCheck::String)
                            }
                            _ => SType::Unknown,
                        }
                    }
                }
            }
            Expr::UnaryOp { op, operand } => {
                use crate::ast::UnaryOp as U;
                match (op, self.infer(operand).name()) {
                    (U::Negate, Some("Int")) => SType::Known(FieldTypeCheck::Int),
                    (U::Negate, Some("Float")) => SType::Known(FieldTypeCheck::Float),
                    (U::Not, _) => SType::Known(FieldTypeCheck::Bool),
                    _ => SType::Unknown,
                }
            }
            // Everything else — blocks, if, match, pipelines, lambdas,
            // field access, indexing, try, ranges — stays Unknown. The
            // runtime enforces; this pass only proves what is cheap to
            // prove.
            _ => SType::Unknown,
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
                    let inferred = self.infer(value);
                    let annotated = decl
                        .type_annotation
                        .as_ref()
                        .and_then(|ann| FieldTypeCheck::from_annotation(ann, &[]));
                    if let (Some(check), Some(actual)) = (&annotated, inferred.name())
                        && !check.accepts(actual)
                    {
                        let binding = match &decl.pattern {
                            crate::ast::Pattern::Identifier(n) => n.as_str(),
                            _ => "value",
                        };
                        self.out.push(CheckDiagnostic {
                            line: span.0,
                            column: span.1,
                            message: format!(
                                "let binding '{}' expects {}, got {}",
                                binding,
                                check.expected_name(),
                                actual
                            ),
                        });
                    }
                    // Bind what we know: the annotation when present (the
                    // runtime guarantees it), else the inferred type.
                    if let crate::ast::Pattern::Identifier(name) = &decl.pattern {
                        let ty = match annotated {
                            Some(c) => SType::Known(c),
                            None => inferred,
                        };
                        self.bind(&name.clone(), ty);
                    }
                }
            }
            Statement::FunctionDecl(f) => {
                // Body scope: parameter annotations are trusted (runtime
                // enforces them at every call).
                self.scopes.push(HashMap::new());
                let checks = param_checks_of(&f.parameters, &f.type_params);
                for (p, c) in f.parameters.iter().zip(&checks) {
                    let ty = match c {
                        Some(c) => SType::Known(c.clone()),
                        None => SType::Unknown,
                    };
                    self.bind(&p.name.clone(), ty);
                }
                self.check_expr(&f.body, span);
                // Provable return contradiction: only when the body's type
                // is directly known.
                if let Some(ret) = return_check_of(f.return_type.as_ref(), &f.type_params)
                    && let Some(actual) = self.infer(&f.body).name()
                    && !ret.accepts(actual)
                {
                    self.out.push(CheckDiagnostic {
                        line: span.0,
                        column: span.1,
                        message: format!(
                            "return value of {} expects {}, got {}",
                            f.name,
                            ret.expected_name(),
                            actual
                        ),
                    });
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
            if arguments.len() > sig.total {
                self.out.push(CheckDiagnostic {
                    line: span.0,
                    column: span.1,
                    message: format!(
                        "Arity mismatch: expected {}, got {}",
                        sig.total,
                        arguments.len()
                    ),
                });
            } else if arguments.len() < sig.required {
                let missing = &sig.param_names[arguments.len()];
                self.out.push(CheckDiagnostic {
                    line: span.0,
                    column: span.1,
                    message: format!("Missing required argument: {}", missing),
                });
            }
            let pairs: Vec<(usize, FieldTypeCheck, String)> = arguments
                .iter()
                .enumerate()
                .filter_map(|(i, a)| {
                    let Argument::Positional(e) = a else {
                        return None;
                    };
                    let check = self.sigs.get(name)?.param_checks.get(i)?.clone()?;
                    let actual = self.infer(e).name()?.to_string();
                    (!check.accepts(&actual)).then_some((i, check, actual))
                })
                .collect();
            for (i, check, actual) in pairs {
                let sig = &self.sigs[name];
                self.out.push(CheckDiagnostic {
                    line: span.0,
                    column: span.1,
                    message: format!(
                        "parameter '{}' of {} expects {}, got {}",
                        sig.param_names[i],
                        name,
                        check.expected_name(),
                        actual
                    ),
                });
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
            Expr::ForLoop { iterable, body, .. } | Expr::ParForLoop { iterable, body, .. } => {
                self.check_expr(iterable, span);
                self.check_expr(body, span);
            }
            Expr::WhileLoop { condition, body } => {
                self.check_expr(condition, span);
                self.check_expr(body, span);
            }
            Expr::Loop { body } => self.check_expr(body, span),
            Expr::Lambda { body, .. } => {
                self.scopes.push(HashMap::new());
                self.check_expr(body, span);
                self.scopes.pop();
            }
            Expr::Match { value, arms } => {
                self.check_expr(value, span);
                for arm in arms {
                    self.check_expr(&arm.expression, span);
                }
            }
            Expr::TryCatch {
                try_block,
                catch_block,
                ..
            } => {
                self.check_expr(try_block, span);
                self.check_expr(catch_block, span);
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
}

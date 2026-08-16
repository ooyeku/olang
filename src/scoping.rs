//! Lexical scope and mutability validation.
//!
//! olang binds names with `let`, and from 0.61 two rules are enforced
//! before a program runs:
//!
//!   * **`let` is required for a first binding.** `x = 1` on a name that
//!     was never declared is an error, not a silent new binding.
//!   * **`mut` is a guarantee.** Assigning to a binding declared without
//!     `mut` is an error. A fresh `let` of the same name still shadows,
//!     which is the idiomatic way to thread a value through a pipeline.
//!
//! Both rules are lexical: whether an assignment is legal depends only on
//! the enclosing declarations, never on runtime values. Validating them in
//! one pass before execution means the interpreter, the bytecode tier, and
//! the JIT cannot disagree — an invalid program is rejected before any of
//! them runs it — and the reader gets the error with a line number instead
//! of a failure halfway through the run.
//!
//! The pass deliberately checks *assignment targets only*. Reads of unknown
//! names stay a runtime concern: builtins, stdlib modules, and imported
//! members all live in the environment rather than in the AST, so a static
//! read check here would be a false-positive machine.

use crate::ast::{
    Argument, Expr, LetDecl, MatchArm, Pattern, Program, ShareDecl, Statement, TemplatePart,
};
use std::collections::HashMap;

/// A scope or mutability violation, positioned at the enclosing statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

/// Names visible to a program before its first statement: the REPL's
/// accumulated session bindings. Maps a name to whether it is mutable.
pub type Predefined = HashMap<String, bool>;

/// Validate a program's scoping and mutability. `predefined` seeds the
/// outermost scope (empty for a fresh program; the session's bindings in
/// the REPL). On success the top-level bindings the program introduces are
/// written back into `predefined`, so a REPL session accumulates them.
pub fn validate_program(program: &Program, predefined: &mut Predefined) -> Vec<ScopeError> {
    let mut v = Validator {
        scopes: vec![std::mem::take(predefined)],
        fn_boundaries: Vec::new(),
        errors: Vec::new(),
        line: 0,
        column: 0,
    };
    v.hoist(&program.statements);
    for stmt in &program.statements {
        v.stmt(stmt);
    }
    // Hand the top-level scope back to the caller for the next program.
    *predefined = v.scopes.swap_remove(0);
    v.errors
}

struct Validator {
    /// Innermost scope last. Each frame maps a name to its mutability.
    scopes: Vec<HashMap<String, bool>>,
    /// Scope index at which each open function body begins, innermost last.
    /// A binding found below the top of this stack lives outside the
    /// current function, and is therefore captured by value.
    fn_boundaries: Vec<usize>,
    errors: Vec<ScopeError>,
    /// Position of the statement being walked, used for error spans.
    line: u32,
    column: u32,
}

impl Validator {
    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    fn bind(&mut self, name: &str, mutable: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), mutable);
        }
    }

    /// Mutability of the innermost binding of `name`, if any.
    fn lookup(&self, name: &str) -> Option<bool> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    /// Index of the innermost scope binding `name` (nearest wins, so a
    /// local shadows a captured outer binding).
    fn binding_scope(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rposition(|s| s.contains_key(name))
    }

    /// Is `name` bound strictly outside the innermost function body — i.e.
    /// captured rather than local? Functions capture by value, so a write
    /// to such a name cannot reach the original binding.
    fn is_captured(&self, name: &str) -> bool {
        match (self.fn_boundaries.last(), self.binding_scope(name)) {
            (Some(&boundary), Some(index)) => index < boundary,
            _ => false,
        }
    }

    fn error(&mut self, message: String) {
        self.errors.push(ScopeError {
            message,
            line: self.line,
            column: self.column,
        });
    }

    /// Declarations are visible to the whole scope that contains them, so a
    /// function may call one declared later in the file. Values are not
    /// hoisted: `let` still binds where it is written.
    fn hoist(&mut self, statements: &[Statement]) {
        for stmt in statements {
            let mut s = stmt;
            while let Statement::Located { stmt: inner, .. } = s {
                s = inner;
            }
            match s {
                Statement::FunctionDecl(d) => self.bind(&d.name, false),
                Statement::ShareDecl(ShareDecl::Function(d)) => self.bind(&d.name, false),
                _ => {}
            }
        }
    }

    // ── statements ────────────────────────────────────────────────────

    fn stmt(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Located {
                line,
                column,
                stmt: inner,
            } => {
                let (pl, pc) = (self.line, self.column);
                self.line = *line;
                self.column = *column;
                self.stmt(inner);
                self.line = pl;
                self.column = pc;
            }
            Statement::Expression(e) => self.expr(e),
            Statement::LetDecl(d) => self.let_decl(d),
            Statement::FunctionDecl(d) => {
                self.bind(&d.name, false);
                self.function_body(&d.parameters, &d.body);
            }
            // Type, trait, and impl declarations introduce type-level names
            // and method tables, not assignable value bindings.
            Statement::TypeDecl(_) | Statement::ErrorTypeDecl(_) | Statement::TraitDecl(_) => {}
            Statement::ImplDecl(d) => {
                for m in &d.methods {
                    self.function_body(&m.parameters, &m.body);
                }
            }
            Statement::ShareDecl(s) => match s {
                ShareDecl::Function(d) => {
                    self.bind(&d.name, false);
                    self.function_body(&d.parameters, &d.body);
                }
                ShareDecl::Let(d) => self.let_decl(d),
                ShareDecl::Type(_) | ShareDecl::Trait(_) => {}
                ShareDecl::Impl(d) => {
                    for m in &d.methods {
                        self.function_body(&m.parameters, &m.body);
                    }
                }
                ShareDecl::Use(u) => self.use_decl(u),
            },
            Statement::UseDecl(u) => self.use_decl(u),
            Statement::TestDecl(t) => {
                // A test's bindings belong to that test, not to the file.
                self.push();
                self.hoist(&t.body);
                for s in &t.body {
                    self.stmt(s);
                }
                self.pop();
            }
        }
    }

    fn use_decl(&mut self, u: &crate::ast::UseDecl) {
        // An import is a binding like any other, and not assignable.
        for item in &u.items {
            match item {
                crate::ast::UseItem::Specific(name) => self.bind(name, false),
                // A wildcard's names are only known once the module loads;
                // nothing to record here.
                crate::ast::UseItem::Wildcard => {}
            }
        }
        if let Some(last) = u.path.last() {
            self.bind(last, false);
        }
    }

    fn let_decl(&mut self, d: &LetDecl) {
        // The value is evaluated in the scope *before* the binding exists,
        // so `let text = str.trim(text)` reads the outer `text`.
        if let Some(v) = &d.value {
            self.expr(v);
        }
        let mut names = Vec::new();
        collect_pattern_names(&d.pattern, &mut names);
        for n in names {
            self.bind(&n, d.mutable);
        }
    }

    /// A function body is a fresh scope holding its parameters. Parameters
    /// are immutable: assigning to one changes only the callee's copy, so
    /// the useful form is a local `let mut`.
    fn function_body(&mut self, parameters: &[crate::ast::Parameter], body: &Expr) {
        // Default-value expressions are evaluated at the call site, in the
        // caller's scope, so they are walked before the boundary opens.
        for p in parameters {
            if let Some(default) = &p.default_value {
                self.expr(default);
            }
        }
        self.push();
        self.fn_boundaries.push(self.scopes.len() - 1);
        for p in parameters {
            self.bind(&p.name, false);
        }
        self.expr(body);
        self.fn_boundaries.pop();
        self.pop();
    }

    // ── assignment: the two rules ─────────────────────────────────────

    fn assign(&mut self, target: &str, value: &Expr) {
        self.expr(value);
        match self.lookup(target) {
            None => self.error(format!(
                "cannot assign to '{target}': it is not declared in this scope. \
                 Declare it first with `let {target} = ...`, or `let mut {target} = ...` \
                 if it needs to change"
            )),
            Some(false) => self.error(format!(
                "cannot assign to '{target}': it is not declared mutable. \
                 Declare it with `let mut {target} = ...`, or bind a new value \
                 with `let {target} = ...` to shadow it"
            )),
            // Declared `mut` and in scope — but a `mut` binding that lives
            // outside this function is still unreachable from inside it.
            // Functions and closures capture by value, so the write lands
            // on the snapshot and the original never changes. Before cells
            // existed there was nothing to point at and this was only a
            // warning; now there is.
            Some(true) if self.is_captured(target) => self.error(format!(
                "cannot assign to '{target}': it is captured from an enclosing scope, \
                 and functions capture by value — the outer '{target}' would not change. \
                 Return the new value, or hold the state in a cell \
                 (`let {target} = cell(...)`, then `cell.set({target}, ...)`)"
            )),
            Some(true) => {}
        }
    }

    // ── expressions ───────────────────────────────────────────────────

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Assignment { target, value } => self.assign(target, value),
            Expr::LocalAssign { name, value, .. } => self.assign(name, value),

            Expr::Block(statements) => {
                // Blocks scope their bindings: a `let` inside one is gone
                // at the closing brace.
                self.push();
                self.hoist(statements);
                for s in statements {
                    self.stmt(s);
                }
                self.pop();
            }

            Expr::Lambda {
                parameters, body, ..
            } => self.function_body(parameters, body),

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
                self.expr(iterable);
                self.push();
                // The loop variable is rebound by the loop on each pass;
                // assigning to it would be overwritten immediately.
                self.bind(variable, false);
                self.expr(body);
                self.pop();
            }

            Expr::Match { value, arms } => {
                self.expr(value);
                for MatchArm {
                    pattern,
                    guard,
                    expression,
                } in arms
                {
                    self.push();
                    let mut names = Vec::new();
                    collect_pattern_names(pattern, &mut names);
                    for n in names {
                        self.bind(&n, false);
                    }
                    if let Some(g) = guard {
                        self.expr(g);
                    }
                    self.expr(expression);
                    self.pop();
                }
            }

            Expr::TryCatch {
                try_block,
                catch_var,
                catch_block,
            } => {
                self.expr(try_block);
                self.push();
                self.bind(catch_var, false);
                self.expr(catch_block);
                self.pop();
            }

            // ── plain recursion ──
            Expr::Call { callee, arguments } => {
                self.expr(callee);
                for a in arguments {
                    match a {
                        Argument::Positional(v) => self.expr(v),
                        Argument::Named { value, .. } => self.expr(value),
                    }
                }
            }
            Expr::List(items) => {
                for i in items.iter() {
                    self.expr(i);
                }
            }
            Expr::Tuple(items) => {
                for i in items.iter() {
                    self.expr(i);
                }
            }
            Expr::Pipeline { left, right } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.expr(then_branch);
                if let Some(e) = else_branch {
                    self.expr(e);
                }
            }
            Expr::Range { start, end, .. } => {
                self.expr(start);
                self.expr(end);
            }
            Expr::BinaryOp { left, right, .. } | Expr::BitwiseOp { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::UnaryOp { operand, .. } => self.expr(operand),
            Expr::StructLiteral(s) => {
                for f in &s.fields {
                    self.expr(&f.value);
                }
            }
            Expr::AnonymousObject { fields } => {
                for f in fields {
                    self.expr(&f.value);
                }
            }
            Expr::MapLiteral { entries } => {
                for entry in entries {
                    self.expr(&entry.key);
                    self.expr(&entry.value);
                }
            }
            Expr::FieldAccess { object, .. } => self.expr(object),
            Expr::Index { object, index } => {
                self.expr(object);
                self.expr(index);
            }
            Expr::WhileLoop { condition, body } => {
                self.expr(condition);
                self.expr(body);
            }
            Expr::Loop { body } => self.expr(body),
            Expr::TemplateString { parts } => {
                for p in parts {
                    match p {
                        TemplatePart::Literal(_) => {}
                        TemplatePart::Interpolation(e) => self.expr(e),
                    }
                }
            }
            Expr::AssertEq {
                actual, expected, ..
            }
            | Expr::AssertNe {
                actual, expected, ..
            } => {
                self.expr(actual);
                self.expr(expected);
            }
            Expr::Assert { condition, .. } => self.expr(condition),
            Expr::AssertTrue { expression, .. } | Expr::AssertFalse { expression, .. } => {
                self.expr(expression)
            }
            Expr::ResultOk(e)
            | Expr::ResultErr(e)
            | Expr::Try(e)
            | Expr::Spawn(e)
            | Expr::Spread(e)
            | Expr::Rest(e) => self.expr(e),
            Expr::Break(e) | Expr::Return(e) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }

            // ── leaves ──
            Expr::Integer(_)
            | Expr::Float(_)
            | Expr::String(_)
            | Expr::RawString(_)
            | Expr::Boolean(_)
            | Expr::Identifier(_)
            | Expr::LocalRef { .. }
            | Expr::Continue => {}
        }
    }
}

/// Every name a pattern binds, including list rests and nested patterns.
fn collect_pattern_names(p: &Pattern, out: &mut Vec<String>) {
    match p {
        Pattern::Identifier(n) | Pattern::Rest(n) => out.push(n.clone()),
        Pattern::List { patterns, rest } => {
            for p in patterns {
                collect_pattern_names(p, out);
            }
            if let Some(r) = rest {
                out.push(r.clone());
            }
        }
        Pattern::Tuple(patterns)
        | Pattern::EnumVariant { patterns, .. }
        | Pattern::Or {
            alternatives: patterns,
        } => {
            for p in patterns {
                collect_pattern_names(p, out);
            }
        }
        Pattern::Ok(p) | Pattern::Err(p) => collect_pattern_names(p, out),
        Pattern::Struct { field_patterns, .. } | Pattern::AnonymousStruct { field_patterns } => {
            for (_, p) in field_patterns {
                collect_pattern_names(p, out);
            }
        }
        Pattern::Guarded { pattern, .. } => collect_pattern_names(pattern, out),
        Pattern::Literal(_) | Pattern::Wildcard | Pattern::Range { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn errors(src: &str) -> Vec<String> {
        let program = Parser::new().parse(src).expect("parses");
        let mut predefined = Predefined::new();
        validate_program(&program, &mut predefined)
            .into_iter()
            .map(|e| e.message)
            .collect()
    }

    #[test]
    fn mut_bindings_may_be_reassigned() {
        assert!(errors("let mut n = 1\nn = 2\n").is_empty());
    }

    #[test]
    fn plain_let_may_not_be_reassigned() {
        let e = errors("let n = 1\nn = 2\n");
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("not declared mutable"), "{}", e[0]);
    }

    #[test]
    fn assignment_without_a_declaration_is_an_error() {
        let e = errors("n = 1\n");
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("not declared in this scope"), "{}", e[0]);
    }

    #[test]
    fn a_fresh_let_shadows_without_mut() {
        // The documented pipeline idiom stays legal.
        assert!(
            errors("let text = \"  hi \"\nlet text = str.trim(text)\nprintln(text)\n").is_empty()
        );
    }

    #[test]
    fn block_bindings_do_not_escape() {
        let e = errors("{ let mut inner = 1 }\ninner = 2\n");
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("not declared in this scope"), "{}", e[0]);
    }

    #[test]
    fn a_block_may_assign_an_outer_mut_binding() {
        assert!(errors("let mut total = 0\n{ total = 1 }\n").is_empty());
    }

    #[test]
    fn parameters_and_loop_variables_are_immutable() {
        let e = errors("fn f(x) = { x = 1 }\n");
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("not declared mutable"), "{}", e[0]);
        let e = errors("for i in 0..3 { i = 9 }\n");
        assert_eq!(e.len(), 1, "{e:?}");
    }

    #[test]
    fn match_and_catch_bindings_are_immutable() {
        let e = errors("let v = 1\nlet r = match v { n => { n = 2 } }\n");
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("not declared mutable"), "{}", e[0]);
    }

    #[test]
    fn functions_may_be_called_before_they_are_declared() {
        assert!(errors("fn a() = b()\nfn b() = 1\nprintln(to_string(a()))\n").is_empty());
    }

    #[test]
    fn errors_carry_the_statement_position() {
        let program = Parser::new().parse("let n = 1\nn = 2\n").expect("parses");
        let mut predefined = Predefined::new();
        let errs = validate_program(&program, &mut predefined);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].line, 2, "reports the assignment's line");
    }

    #[test]
    fn top_level_bindings_carry_into_the_next_program() {
        // The REPL evaluates one line at a time against one session.
        let mut predefined = Predefined::new();
        let first = Parser::new().parse("let mut total = 0\n").unwrap();
        assert!(validate_program(&first, &mut predefined).is_empty());
        let second = Parser::new().parse("total = 5\n").unwrap();
        assert!(validate_program(&second, &mut predefined).is_empty());
        let third = Parser::new().parse("other = 5\n").unwrap();
        assert_eq!(validate_program(&third, &mut predefined).len(), 1);
    }

    #[test]
    fn destructured_bindings_follow_the_declaration() {
        assert!(errors("let mut (a, b) = (1, 2)\na = 3\n").is_empty());
        let e = errors("let (a, b) = (1, 2)\na = 3\n");
        assert_eq!(e.len(), 1, "{e:?}");
    }
}

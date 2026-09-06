//! Hot-loop promotion: the interpreter's own on-stack replacement.
//!
//! A hot loop *inside a function* already escapes the tree-walk — the
//! function promotes, and the VM's OSR natively enters its loops. A hot
//! loop at the **top level of a script** had no such exit: every
//! iteration tree-walked, and every call it made crossed the
//! interpreter→VM boundary separately (the `chain` benchmark spent
//! ~500ns/iteration on exactly that).
//!
//! The escape: when a top-level `for`-over-range or `while` loop is
//! still running after a threshold of interpreted iterations, the
//! *remainder* of the loop is synthesized into a function — parameters
//! are the loop bounds plus the free variables the body reads, the
//! return is a tuple of the loop's value plus the variables it assigns
//! — and handed to the tier, which compiles it by body identity exactly
//! as it compiles a lambda handed to `map`. One boundary crossing runs
//! every remaining iteration on the VM (JIT included); the live-outs
//! write back through the ordinary assignment path.
//!
//! Soundness: the VM is a full, correct tier (not a speculative one),
//! so effects inside the promoted remainder are as real as they would
//! have been interpreted, and an error from it is the loop's error. The
//! only constructs that must *never* be wrapped in a function are the
//! ones whose control flow crosses the loop's own boundary — `return`
//! and `?` — and the analyzer refuses those (plus anything it cannot
//! classify) by returning `None`, which simply leaves the loop
//! interpreted. A compile refusal downstream (unsupported construct,
//! unresolvable callee) falls back the same way, before any native
//! iteration runs.

use crate::ast::{Argument, Expr, Function, LetDecl, Parameter, Pattern, Statement, TemplatePart};
use std::collections::HashSet;
use std::sync::Arc;

/// What a loop body touches, in first-seen order: free names it reads
/// (module functions included — the caller filters), and free names it
/// assigns.
pub struct BodyFacts {
    pub reads: Vec<String>,
    pub writes: Vec<String>,
}

/// Analyze a loop body. `None` means the body must stay interpreted —
/// it contains `return` or `?` (their control flow crosses a function
/// boundary), a construct the walker cannot classify, or a declaration
/// form a synthetic wrapper would change the meaning of.
pub fn analyze(body: &Expr, loop_var: Option<&str>) -> Option<BodyFacts> {
    let mut w = Walker {
        scopes: vec![HashSet::new()],
        reads: Vec::new(),
        seen_reads: HashSet::new(),
        writes: Vec::new(),
        seen_writes: HashSet::new(),
        hazard: false,
    };
    if let Some(v) = loop_var {
        w.declare(v);
    }
    w.walk(body);
    if w.hazard {
        None
    } else {
        Some(BodyFacts {
            reads: w.reads,
            writes: w.writes,
        })
    }
}

/// The synthesized continuation of a `for v in lo..hi` loop: callable as
/// `f(lo, hi, live_ins...)`, returning `(loop_value, writes...)`.
pub fn synthesize_range_continuation(
    variable: &str,
    inclusive: bool,
    body: &Expr,
    live_ins: &[String],
    writes: &[String],
    def_file: Option<String>,
) -> Function {
    let loop_expr = Expr::ForLoop {
        variable: variable.to_string(),
        iterable: Box::new(Expr::Range {
            start: Box::new(Expr::Identifier("__lo".to_string())),
            end: Box::new(Expr::Identifier("__hi".to_string())),
            inclusive,
        }),
        body: Box::new(body.clone()),
    };
    let mut params = vec![param("__lo"), param("__hi")];
    params.extend(live_ins.iter().map(|n| param(n)));
    build_continuation(loop_expr, params, writes, def_file)
}

/// The synthesized continuation of a `while cond` loop: callable as
/// `f(live_ins...)`, returning `(loop_value, writes...)`.
pub fn synthesize_while_continuation(
    condition: &Expr,
    body: &Expr,
    live_ins: &[String],
    writes: &[String],
    def_file: Option<String>,
) -> Function {
    let loop_expr = Expr::WhileLoop {
        condition: Box::new(condition.clone()),
        body: Box::new(body.clone()),
    };
    let params: Vec<Parameter> = live_ins.iter().map(|n| param(n)).collect();
    build_continuation(loop_expr, params, writes, def_file)
}

fn build_continuation(
    loop_expr: Expr,
    params: Vec<Parameter>,
    writes: &[String],
    def_file: Option<String>,
) -> Function {
    // Bare loop value when nothing writes back; a tuple carrying the
    // written variables otherwise.
    let tail = if writes.is_empty() {
        Expr::Identifier("__r".to_string())
    } else {
        let mut items = vec![Expr::Identifier("__r".to_string())];
        items.extend(writes.iter().map(|n| Expr::Identifier(n.clone())));
        Expr::Tuple(Arc::new(items))
    };
    let body = Expr::Block(vec![
        Statement::LetDecl(LetDecl {
            pattern: Pattern::Identifier("__r".to_string()),
            type_annotation: None,
            value: Some(loop_expr),
            name_span: None,
            mutable: false,
        }),
        Statement::Expression(tail),
    ]);

    let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
    let resolved = crate::resolve::Resolver::resolve_function_body(&body, None, &param_names);
    Function {
        name: Some("<hot loop>".to_string()),
        param_checks: crate::ast::param_checks_of(&params, &[]),
        return_check: None,
        parameters: params,
        body: Arc::new(resolved),
        closure: Arc::new(im::HashMap::new()),
        param_bounds: Vec::new(),
        def_file,
        parent_scope: 0,
    }
}

fn param(name: &str) -> Parameter {
    Parameter {
        name: name.to_string(),
        type_annotation: None,
        default_value: None,
    }
}

struct Walker {
    scopes: Vec<HashSet<String>>,
    reads: Vec<String>,
    seen_reads: HashSet<String>,
    writes: Vec<String>,
    seen_writes: HashSet<String>,
    hazard: bool,
}

impl Walker {
    fn is_local(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }

    fn declare(&mut self, name: &str) {
        self.scopes
            .last_mut()
            .expect("scope stack never empty")
            .insert(name.to_string());
    }

    fn read(&mut self, name: &str) {
        if !self.is_local(name) && self.seen_reads.insert(name.to_string()) {
            self.reads.push(name.to_string());
        }
    }

    fn write(&mut self, name: &str) {
        if !self.is_local(name) {
            // A written variable is also a live-in: the remainder may
            // read-modify it, so its current value must arrive.
            self.read(name);
            if self.seen_writes.insert(name.to_string()) {
                self.writes.push(name.to_string());
            }
        }
    }

    fn pattern_names(p: &Pattern, out: &mut Vec<String>) {
        match p {
            Pattern::Identifier(n) => out.push(n.clone()),
            Pattern::Literal(_) | Pattern::Wildcard => {}
            Pattern::List { patterns, rest } => {
                for p in patterns {
                    Self::pattern_names(p, out);
                }
                if let Some(r) = rest {
                    out.push(r.clone());
                }
            }
            Pattern::Tuple(ps) | Pattern::EnumVariant { patterns: ps, .. } => {
                for p in ps {
                    Self::pattern_names(p, out);
                }
            }
            Pattern::Ok(p) | Pattern::Err(p) => Self::pattern_names(p, out),
            Pattern::Struct { field_patterns, .. }
            | Pattern::AnonymousStruct { field_patterns } => {
                for (_, p) in field_patterns {
                    Self::pattern_names(p, out);
                }
            }
            // Anything unrecognized: bind nothing, which at worst makes a
            // body-local name look free — a harmless extra live-in.
            _ => {}
        }
    }

    fn walk_scoped(&mut self, declare: &[String], exprs: &[&Expr]) {
        self.scopes.push(HashSet::new());
        for d in declare {
            self.declare(d);
        }
        for e in exprs {
            self.walk(e);
        }
        self.scopes.pop();
    }

    fn walk(&mut self, e: &Expr) {
        if self.hazard {
            return;
        }
        match e {
            Expr::Integer(_)
            | Expr::Float(_)
            | Expr::String(_)
            | Expr::Boolean(_)
            | Expr::RawString(_)
            | Expr::Continue => {}
            Expr::Identifier(n) | Expr::LocalRef { name: n, .. } => self.read(n),
            Expr::LocalAssign { name, value, .. }
            | Expr::Assignment {
                target: name,
                value,
            } => {
                self.walk(value);
                self.write(name);
            }
            Expr::List(items) => {
                for i in items.iter() {
                    self.walk(i);
                }
            }
            Expr::Tuple(items) => {
                for i in items.iter() {
                    self.walk(i);
                }
            }
            Expr::Call { callee, arguments } => {
                self.walk(callee);
                for a in arguments {
                    match a {
                        Argument::Positional(e) => self.walk(e),
                        Argument::Named { value, .. } => self.walk(value),
                    }
                }
            }
            Expr::Lambda {
                parameters, body, ..
            } => {
                let names: Vec<String> = parameters.iter().map(|p| p.name.clone()).collect();
                for p in parameters {
                    if let Some(d) = &p.default_value {
                        self.walk(d);
                    }
                }
                self.walk_scoped(&names, &[body]);
            }
            Expr::Pipeline { left, right } => {
                self.walk(left);
                self.walk(right);
            }
            Expr::Match { value, arms } => {
                self.walk(value);
                for arm in arms {
                    let mut names = Vec::new();
                    Self::pattern_names(&arm.pattern, &mut names);
                    self.scopes.push(HashSet::new());
                    for n in &names {
                        self.declare(n);
                    }
                    if let Some(g) = &arm.guard {
                        self.walk(g);
                    }
                    self.walk(&arm.expression);
                    self.scopes.pop();
                }
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.walk(condition);
                self.walk(then_branch);
                if let Some(e) = else_branch {
                    self.walk(e);
                }
            }
            Expr::Range { start, end, .. } => {
                self.walk(start);
                self.walk(end);
            }
            Expr::Block(stmts) => {
                self.scopes.push(HashSet::new());
                for s in stmts {
                    self.walk_stmt(s);
                }
                self.scopes.pop();
            }
            Expr::BinaryOp { left, right, .. } | Expr::BitwiseOp { left, right, .. } => {
                self.walk(left);
                self.walk(right);
            }
            Expr::UnaryOp { operand, .. } => self.walk(operand),
            Expr::StructLiteral(sl) => {
                for f in &sl.fields {
                    self.walk(&f.value);
                }
            }
            Expr::AnonymousObject { fields } => {
                for f in fields {
                    self.walk(&f.value);
                }
            }
            Expr::MapLiteral { entries } => {
                for e in entries {
                    self.walk(&e.key);
                    self.walk(&e.value);
                }
            }
            Expr::FieldAccess { object, .. } => self.walk(object),
            Expr::ResultOk(e) | Expr::ResultErr(e) | Expr::Spread(e) | Expr::Rest(e) => {
                self.walk(e)
            }
            Expr::ForLoop {
                variable,
                iterable,
                body,
            } => {
                self.walk(iterable);
                self.walk_scoped(std::slice::from_ref(variable), &[body]);
            }
            Expr::WhileLoop { condition, body } => {
                self.walk(condition);
                self.walk_scoped(&[], &[body]);
            }
            Expr::Loop { body } => self.walk_scoped(&[], &[body]),
            Expr::Break(v) => {
                if let Some(v) = v {
                    self.walk(v);
                }
            }
            Expr::Index { object, index } => {
                self.walk(object);
                self.walk(index);
            }
            Expr::TemplateString { parts } => {
                for p in parts {
                    if let TemplatePart::Interpolation(e) = p {
                        self.walk(e);
                    }
                }
            }
            Expr::AssertEq {
                actual, expected, ..
            }
            | Expr::AssertNe {
                actual, expected, ..
            } => {
                self.walk(actual);
                self.walk(expected);
            }
            Expr::Assert { condition, .. } => self.walk(condition),
            Expr::AssertTrue { expression, .. } | Expr::AssertFalse { expression, .. } => {
                self.walk(expression)
            }
            // Control flow that crosses a function boundary — wrapping it
            // changes what it means. And everything unclassified: refuse,
            // which just leaves the loop interpreted.
            Expr::Return(_)
            | Expr::Try(_)
            | Expr::Spawn(_)
            | Expr::ParForLoop { .. }
            | Expr::MacroCall { .. } => self.hazard = true,
        }
    }

    fn walk_stmt(&mut self, s: &Statement) {
        if self.hazard {
            return;
        }
        match s {
            Statement::Located { stmt, .. } => self.walk_stmt(stmt),
            Statement::Expression(e) => self.walk(e),
            Statement::LetDecl(ld) => {
                if let Some(v) = &ld.value {
                    self.walk(v);
                }
                let mut names = Vec::new();
                Self::pattern_names(&ld.pattern, &mut names);
                for n in names {
                    self.declare(&n);
                }
            }
            // Declarations that bind more than a value (functions, types,
            // shares, tests): a synthetic wrapper would scope them
            // differently — refuse.
            _ => self.hazard = true,
        }
    }
}

//! The checker sees keys (roadmap E4): a `.key` read that no construction
//! of the value can have.
//!
//! `m.key` on a map whose key is absent is `()`, so a misspelt key —
//! `p.runing` for `p.running` — fails nowhere: it reads nothing. With `.`
//! the key is syntax, and this pass follows what each value can be within
//! one module:
//!
//! - a map or object literal whose keys are all written (`#{ "a": 1 }`,
//!   `{ a: 1 }`) has exactly those keys, each with the shape of its value;
//! - `map_set(m, "k", v)`, `map_set_in`, `map_update`, `map_merge`, a
//!   spread, `m.k`, `a ?? b`, `if`/`match`/blocks follow from their parts;
//! - a `let` (or an assignment) binds the shapes of every value it is
//!   given anywhere in the function — flow-insensitive, so a name ever
//!   given something unknown is unknown;
//! - a function's parameter is the join of the arguments of every call of
//!   it in the module, and its result the join of what it returns — unless
//!   the function is shared, referenced as a value, declared twice, or the
//!   file uses macros (its calls may be elsewhere): then it is unknown.
//!
//! Everything else is unknown, and an unknown value is never reported:
//! every join only adds keys, so a report means no construction this pass
//! saw has the key, and every construction it could not see makes the
//! value unknown. A read compared with `()` or defaulted with `??` is a
//! presence test, and is not reported.

use crate::ast::{Argument, Expr, FunctionDecl, Pattern, Program, Statement, TemplatePart};
use std::collections::{BTreeMap, HashMap, HashSet};

/// What a value can be, as far as its keys go.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Nothing seen yet (or `()`): adds nothing to a join.
    Bottom,
    /// A map or object with at most these keys, each with its value's shape.
    Keys(BTreeMap<String, Shape>),
    /// Anything.
    Open,
}

const MAX_DEPTH: usize = 6;
const MAX_ROUNDS: usize = 16;

impl Shape {
    fn join(&self, other: &Shape) -> Shape {
        self.join_at(other, 0)
    }

    fn join_at(&self, other: &Shape, depth: usize) -> Shape {
        match (self, other) {
            (Shape::Open, _) | (_, Shape::Open) => Shape::Open,
            (Shape::Bottom, x) | (x, Shape::Bottom) => x.clone().limit(depth),
            (Shape::Keys(a), Shape::Keys(b)) => {
                if depth > MAX_DEPTH {
                    return Shape::Open;
                }
                let mut out = a.clone();
                for (k, v) in b {
                    let joined = match out.get(k) {
                        Some(mine) => mine.join_at(v, depth + 1),
                        None => v.clone().limit(depth + 1),
                    };
                    out.insert(k.clone(), joined);
                }
                Shape::Keys(out)
            }
        }
    }

    /// Deep shapes (a value nested in itself through recursion) are cut
    /// to `Open`, so the fixpoint ends.
    fn limit(self, depth: usize) -> Shape {
        match self {
            Shape::Keys(m) if depth > MAX_DEPTH => {
                let _ = m;
                Shape::Open
            }
            Shape::Keys(m) => Shape::Keys(
                m.into_iter()
                    .map(|(k, v)| (k, v.limit(depth + 1)))
                    .collect(),
            ),
            other => other,
        }
    }

    fn with_key(&self, key: &str, value: Shape) -> Shape {
        match self {
            Shape::Keys(m) => {
                let mut m = m.clone();
                let v = match m.get(key) {
                    Some(old) => old.join(&value),
                    None => value,
                };
                m.insert(key.to_string(), v);
                Shape::Keys(m)
            }
            // nothing seen yet: least is the map of that key (the fixpoint
            // grows from here; a write on `()` raises)
            Shape::Bottom => Shape::Keys(BTreeMap::from([(key.to_string(), value)])),
            Shape::Open => Shape::Open,
        }
    }

    fn key(&self, key: &str) -> Shape {
        match self {
            Shape::Keys(m) => m.get(key).cloned().unwrap_or(Shape::Bottom),
            Shape::Bottom => Shape::Bottom,
            Shape::Open => Shape::Open,
        }
    }
}

/// A `.key` read no construction has.
#[derive(Debug, Clone)]
pub struct KeyFinding {
    pub line: u32,
    pub column: u32,
    pub key: String,
    /// The receiver as written, when it is a name or a chain of names.
    pub receiver: String,
    /// The keys the constructions have.
    pub known: Vec<String>,
}

type Env = HashMap<String, Shape>;

struct FnInfo<'p> {
    decl: &'p FunctionDecl,
    /// Called from places this pass cannot see: shared, used as a value,
    /// declared twice.
    escapes: bool,
    params: Vec<Shape>,
    ret: Shape,
}

struct Analysis<'p> {
    funcs: HashMap<String, FnInfo<'p>>,
    module: Env,
    /// Module-level names assigned inside a function: unknown.
    globally_assigned: HashSet<String>,
    /// Every body this pass walks, with its parameters: functions, the
    /// top level (and test blocks), impl methods.
    bodies: Vec<Body<'p>>,
}

enum BodyKind {
    Function(String),
    Opaque,
}

struct Body<'p> {
    kind: BodyKind,
    params: Vec<&'p crate::ast::Parameter>,
    exprs: Vec<(&'p Expr, (u32, u32))>,
    stmts: Vec<&'p Statement>,
    env: Env,
}

fn located(stmt: &Statement, span: (u32, u32)) -> (u32, u32) {
    match stmt {
        Statement::Located { line, column, .. } => (*line, *column),
        _ => span,
    }
}

/// The `.key` reads of `program` that no construction of their value has.
pub fn impossible_reads(program: &Program) -> Vec<KeyFinding> {
    let Some(analysis) = Analysis::run(program) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for body in &analysis.bodies {
        for stmt in &body.stmts {
            analysis.report_stmt(stmt, (0, 0), &body.env, &mut out);
        }
        for (e, span) in &body.exprs {
            analysis.report_expr(e, *span, &body.env, &mut out, false);
        }
    }
    out.sort_by_key(|f| (f.line, f.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.key == b.key);
    out
}

/// How many `.key` reads the pass knew the keys of, and how many reads
/// there are (`OLANG_KEYS_STATS=1 olang check` prints them a file).
pub fn read_coverage(program: &Program) -> (usize, usize) {
    let Some(analysis) = Analysis::run(program) else {
        return (0, 0);
    };
    let mut known = 0;
    let mut total = 0;
    fn walk(a: &Analysis<'_>, e: &Expr, env: &Env, known: &mut usize, total: &mut usize) {
        if let Expr::FieldAccess { object, .. } = e {
            *total += 1;
            if matches!(a.shape(object, env), Shape::Keys(_)) {
                *known += 1;
            }
        }
        if let Expr::Block(stmts) = e {
            for s in stmts {
                if let Statement::Expression(x) = s.unwrapped() {
                    walk(a, x, env, known, total);
                } else if let Statement::LetDecl(l) = s.unwrapped()
                    && let Some(v) = &l.value
                {
                    walk(a, v, env, known, total);
                }
            }
            return;
        }
        for c in children(e) {
            walk(a, c, env, known, total);
        }
    }
    for body in &analysis.bodies {
        for (e, _) in &body.exprs {
            walk(&analysis, e, &body.env, &mut known, &mut total);
        }
        for st in &body.stmts {
            match st.unwrapped() {
                Statement::Expression(x) => walk(&analysis, x, &body.env, &mut known, &mut total),
                Statement::LetDecl(l) => {
                    if let Some(v) = &l.value {
                        walk(&analysis, v, &body.env, &mut known, &mut total)
                    }
                }
                _ => {}
            }
        }
    }
    (known, total)
}

/// Every key the constructions bound to `name` anywhere in the module
/// have, for completion after `name.`; `None` when it is not known.
pub fn keys_of_name(program: &Program, name: &str) -> Option<Vec<String>> {
    let analysis = Analysis::run(program)?;
    let mut joined = Shape::Bottom;
    let mut seen = false;
    for body in &analysis.bodies {
        if let Some(s) = body.env.get(name) {
            seen = true;
            joined = joined.join(s);
        }
    }
    if let Some(s) = analysis.module.get(name) {
        seen = true;
        joined = joined.join(s);
    }
    match (seen, joined) {
        (true, Shape::Keys(m)) if !m.is_empty() => Some(m.into_keys().collect()),
        _ => None,
    }
}

fn uses_macros(program: &Program) -> bool {
    program.statements.iter().any(|s| {
        matches!(
            s.unwrapped(),
            Statement::MetaFnDecl { .. } | Statement::DecoratedDecl { .. }
        )
    }) || serde_json::to_string(program)
        .map(|j| j.contains("\"MacroCall\""))
        .unwrap_or(true)
}

impl<'p> Analysis<'p> {
    fn run(program: &'p Program) -> Option<Analysis<'p>> {
        if uses_macros(program) {
            return None;
        }
        let mut a = Analysis {
            funcs: HashMap::new(),
            module: Env::new(),
            globally_assigned: HashSet::new(),
            bodies: Vec::new(),
        };
        let mut declared_twice = HashSet::new();
        let mut top = Body {
            kind: BodyKind::Opaque,
            params: Vec::new(),
            exprs: Vec::new(),
            stmts: Vec::new(),
            env: Env::new(),
        };
        for stmt in &program.statements {
            let span = located(stmt, (0, 0));
            match stmt.unwrapped() {
                Statement::FunctionDecl(f) => a.add_fn(f, false, &mut declared_twice, span),
                Statement::ShareDecl(crate::ast::ShareDecl::Function(f)) => {
                    a.add_fn(f, true, &mut declared_twice, span)
                }
                Statement::ShareDecl(crate::ast::ShareDecl::Impl(i)) | Statement::ImplDecl(i) => {
                    for m in &i.methods {
                        a.bodies.push(Body {
                            kind: BodyKind::Opaque,
                            params: m.parameters.iter().collect(),
                            exprs: vec![(&m.body, span)],
                            stmts: Vec::new(),
                            env: Env::new(),
                        });
                    }
                }
                Statement::TraitDecl(t) | Statement::ShareDecl(crate::ast::ShareDecl::Trait(t)) => {
                    for m in &t.methods {
                        if let Some(b) = &m.default_body {
                            a.bodies.push(Body {
                                kind: BodyKind::Opaque,
                                params: m.parameters.iter().collect(),
                                exprs: vec![(b, span)],
                                stmts: Vec::new(),
                                env: Env::new(),
                            });
                        }
                    }
                }
                Statement::TestDecl(t) => {
                    a.bodies.push(Body {
                        kind: BodyKind::Opaque,
                        params: Vec::new(),
                        exprs: Vec::new(),
                        stmts: t.body.iter().collect(),
                        env: Env::new(),
                    });
                }
                _ => top.stmts.push(stmt),
            }
        }
        a.bodies.push(top);
        for name in declared_twice {
            if let Some(f) = a.funcs.get_mut(&name) {
                f.escapes = true;
            }
        }
        // References that are not calls: the function escapes.
        let mut escaped = HashSet::new();
        let mut assigned = HashSet::new();
        for stmt in &program.statements {
            scan_stmt(stmt, &mut escaped, &mut assigned);
        }
        for name in &escaped {
            if let Some(f) = a.funcs.get_mut(name) {
                f.escapes = true;
            }
        }
        a.globally_assigned = assigned;
        for f in a.funcs.values_mut() {
            if f.escapes {
                f.params = vec![Shape::Open; f.decl.parameters.len()];
                f.ret = Shape::Open;
            }
        }
        a.solve()?;
        Some(a)
    }

    fn add_fn(
        &mut self,
        f: &'p FunctionDecl,
        shared: bool,
        twice: &mut HashSet<String>,
        span: (u32, u32),
    ) {
        if self.funcs.contains_key(&f.name) {
            twice.insert(f.name.clone());
            return;
        }
        self.funcs.insert(
            f.name.clone(),
            FnInfo {
                decl: f,
                escapes: shared || !f.type_params.is_empty(),
                params: vec![Shape::Bottom; f.parameters.len()],
                ret: Shape::Bottom,
            },
        );
        self.bodies.push(Body {
            kind: BodyKind::Function(f.name.clone()),
            params: f.parameters.iter().collect(),
            exprs: vec![(&f.body, span)],
            stmts: Vec::new(),
            env: Env::new(),
        });
    }

    /// The fixpoint: bodies' environments, call sites' arguments into
    /// parameters, returns into calls, until nothing changes.
    fn solve(&mut self) -> Option<()> {
        for _ in 0..MAX_ROUNDS {
            let mut changed = false;
            // the module's own names first (the top level is the last body)
            let top = self.bodies.len() - 1;
            let module_env = self.body_env(top, &Env::new());
            let mut module_env = module_env;
            for name in &self.globally_assigned {
                if module_env.contains_key(name) {
                    module_env.insert(name.clone(), Shape::Open);
                }
            }
            if module_env != self.module {
                self.module = module_env;
                changed = true;
            }
            let mut new_params: HashMap<String, Vec<Shape>> = HashMap::new();
            let mut new_rets: HashMap<String, Shape> = HashMap::new();
            for i in 0..self.bodies.len() {
                let outer = if i == top {
                    Env::new()
                } else {
                    self.module.clone()
                };
                let env = self.body_env(i, &outer);
                // call sites
                let mut calls = Vec::new();
                {
                    let body = &self.bodies[i];
                    for s in &body.stmts {
                        collect_calls_stmt(s, &mut calls);
                    }
                    for (e, _) in &body.exprs {
                        collect_calls(e, &mut calls);
                    }
                    for p in &body.params {
                        if let Some(d) = &p.default_value {
                            collect_calls(d, &mut calls);
                        }
                    }
                }
                for (name, args) in calls {
                    let Some(info) = self.funcs.get(name) else {
                        continue;
                    };
                    if info.escapes || env.contains_key(name) {
                        continue;
                    }
                    let mut shapes = vec![Shape::Bottom; info.decl.parameters.len()];
                    let mut positional = 0usize;
                    let mut given = vec![false; shapes.len()];
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) => {
                                if positional < shapes.len() {
                                    shapes[positional] = self.shape(e, &env);
                                    given[positional] = true;
                                }
                                positional += 1;
                            }
                            CallArg::Named(n, e) => {
                                if let Some(at) =
                                    info.decl.parameters.iter().position(|p| p.name == *n)
                                {
                                    shapes[at] = self.shape(e, &env);
                                    given[at] = true;
                                }
                            }
                            CallArg::Shape(s) => {
                                if positional < shapes.len() {
                                    shapes[positional] = s;
                                    given[positional] = true;
                                }
                                positional += 1;
                            }
                        }
                    }
                    for (j, p) in info.decl.parameters.iter().enumerate() {
                        if !given[j] {
                            shapes[j] = match &p.default_value {
                                Some(d) => self.shape(d, &self.module),
                                None => Shape::Bottom,
                            };
                        }
                    }
                    let entry = new_params
                        .entry(name.to_string())
                        .or_insert_with(|| vec![Shape::Bottom; shapes.len()]);
                    for (slot, s) in entry.iter_mut().zip(shapes) {
                        *slot = slot.join(&s);
                    }
                }
                if let BodyKind::Function(name) = &self.bodies[i].kind {
                    let mut ret = Shape::Bottom;
                    for (e, _) in &self.bodies[i].exprs {
                        ret = ret.join(&self.result_shape(e, &env));
                        for r in returns(e) {
                            ret =
                                ret.join(&r.map(|r| self.shape(r, &env)).unwrap_or(Shape::Bottom));
                        }
                    }
                    new_rets.insert(name.clone(), ret);
                }
                if env != self.bodies[i].env {
                    self.bodies[i].env = env;
                    changed = true;
                }
            }
            for (name, info) in self.funcs.iter_mut() {
                if info.escapes {
                    continue;
                }
                // never called here: called from where this pass cannot see
                let params = new_params
                    .remove(name)
                    .unwrap_or_else(|| vec![Shape::Open; info.decl.parameters.len()]);
                if params != info.params {
                    info.params = params;
                    changed = true;
                }
                let ret = new_rets.remove(name).unwrap_or(Shape::Bottom);
                if ret != info.ret {
                    info.ret = ret;
                    changed = true;
                }
            }
            if !changed {
                return Some(());
            }
        }
        None
    }

    /// A body's names, each the join of every value bound to it.
    fn body_env(&self, i: usize, outer: &Env) -> Env {
        let body = &self.bodies[i];
        let mut env = outer.clone();
        let fn_params = match &body.kind {
            BodyKind::Function(name) => self.funcs.get(name).map(|f| f.params.clone()),
            BodyKind::Opaque => None,
        };
        for (j, p) in body.params.iter().enumerate() {
            let s = fn_params
                .as_ref()
                .and_then(|ps| ps.get(j).cloned())
                .unwrap_or(Shape::Open);
            let s = match &p.default_value {
                Some(d) if fn_params.is_some() => s.join(&self.shape(d, outer)),
                _ => s,
            };
            env.insert(p.name.clone(), s);
        }
        // Every name the body binds starts as nothing seen: an unknown
        // name is a free one (a function, an import, a builtin).
        {
            let mut seed = env.clone();
            let mut mark = |name: &str, _s: Shape, next: &mut Env| {
                next.entry(name.to_string()).or_insert(Shape::Bottom);
            };
            for s in &body.stmts {
                self.bind_stmt(s, &env, &mut seed, &mut mark);
            }
            for (e, _) in &body.exprs {
                self.bind_expr(e, &env, &mut seed, &mut mark);
            }
            env = seed;
        }
        let mut stable = false;
        for _ in 0..256 {
            let mut next = env.clone();
            let mut bind = |name: &str, s: Shape, next: &mut Env| {
                let joined = match next.get(name) {
                    Some(old) => old.join(&s),
                    None => s,
                };
                next.insert(name.to_string(), joined);
            };
            for s in &body.stmts {
                self.bind_stmt(s, &env, &mut next, &mut bind);
            }
            for (e, _) in &body.exprs {
                self.bind_expr(e, &env, &mut next, &mut bind);
            }
            if next == env {
                stable = true;
                break;
            }
            env = next;
        }
        if !stable {
            for v in env.values_mut() {
                *v = Shape::Open;
            }
        }
        env
    }

    fn bind_stmt(
        &self,
        stmt: &Statement,
        env: &Env,
        next: &mut Env,
        bind: &mut impl FnMut(&str, Shape, &mut Env),
    ) {
        match stmt.unwrapped() {
            Statement::Expression(e) => self.bind_expr(e, env, next, bind),
            Statement::LetDecl(l) | Statement::ShareDecl(crate::ast::ShareDecl::Let(l)) => {
                if let Some(v) = &l.value {
                    self.bind_expr(v, env, next, bind);
                    match &l.pattern {
                        Pattern::Identifier(n) => bind(n, self.shape(v, env), next),
                        p => bind_pattern(p, next, bind),
                    }
                } else {
                    bind_pattern(&l.pattern, next, bind);
                }
            }
            Statement::FunctionDecl(f) => {
                // a function inside a body: a value of its own, its
                // parameters unknown
                bind(&f.name, Shape::Open, next);
                for p in &f.parameters {
                    bind(&p.name, Shape::Open, next);
                }
                self.bind_expr(&f.body, env, next, bind);
            }
            Statement::TestDecl(t) => {
                for s in &t.body {
                    self.bind_stmt(s, env, next, bind);
                }
            }
            _ => {}
        }
    }

    fn bind_expr(
        &self,
        e: &Expr,
        env: &Env,
        next: &mut Env,
        bind: &mut impl FnMut(&str, Shape, &mut Env),
    ) {
        match e {
            Expr::Assignment { target, value }
            | Expr::LocalAssign {
                name: target,
                value,
                ..
            } => {
                bind(target, self.shape(value, env), next);
            }
            Expr::Lambda { parameters, .. } => {
                for p in parameters {
                    bind(&p.name, Shape::Open, next);
                }
            }
            Expr::ForLoop { variable, .. } | Expr::ParForLoop { variable, .. } => {
                bind(variable, Shape::Open, next);
            }
            Expr::Match { arms, .. } => {
                for arm in arms {
                    bind_pattern(&arm.pattern, next, bind);
                }
            }
            Expr::Block(stmts) => {
                for s in stmts {
                    self.bind_stmt(s, env, next, bind);
                }
                return;
            }
            _ => {}
        }
        for c in children(e) {
            self.bind_expr(c, env, next, bind);
        }
    }

    /// The shape of what `e` evaluates to (its tail, for a block).
    fn result_shape(&self, e: &Expr, env: &Env) -> Shape {
        self.shape(e, env)
    }

    fn shape(&self, e: &Expr, env: &Env) -> Shape {
        match e {
            Expr::MapLiteral { entries } => {
                let mut m = BTreeMap::new();
                for entry in entries {
                    let key = match &entry.key {
                        Expr::String(s) => s.as_ref().clone(),
                        Expr::Integer(i) => i.to_string(),
                        Expr::Boolean(b) => b.to_string(),
                        _ => return Shape::Open,
                    };
                    let v = self.shape(&entry.value, env);
                    let v = match m.get(&key) {
                        Some(old) => Shape::join(old, &v),
                        None => v,
                    };
                    m.insert(key, v);
                }
                Shape::Keys(m).limit(0)
            }
            Expr::AnonymousObject { fields } => Shape::Keys(
                fields
                    .iter()
                    .map(|f| (f.name.clone(), self.shape(&f.value, env)))
                    .collect(),
            )
            .limit(0),
            Expr::Tuple(items) if items.is_empty() => Shape::Bottom,
            Expr::Identifier(n) | Expr::LocalRef { name: n, .. } => match env.get(n) {
                Some(s) => s.clone(),
                None => Shape::Open,
            },
            Expr::FieldAccess { object, field } => self.shape(object, env).key(field),
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => {
                let t = self.shape(then_branch, env);
                match else_branch {
                    Some(e) => t.join(&self.shape(e, env)),
                    None => t,
                }
            }
            Expr::Match { arms, .. } => arms.iter().fold(Shape::Bottom, |acc, arm| {
                acc.join(&self.shape(&arm.expression, env))
            }),
            Expr::Block(stmts) => match stmts.last().map(|s| s.unwrapped()) {
                Some(Statement::Expression(e)) => self.shape(e, env),
                _ => Shape::Bottom,
            },
            Expr::BinaryOp {
                left,
                op: crate::ast::BinaryOp::Coalesce,
                right,
            } => self.shape(left, env).join(&self.shape(right, env)),
            Expr::Return(_) | Expr::Break(_) | Expr::Continue => Shape::Bottom,
            Expr::Call { callee, arguments } => {
                let mut args = Vec::new();
                let mut lits = Vec::new();
                for a in arguments {
                    match a {
                        Argument::Positional(e) => {
                            args.push(self.shape(e, env));
                            lits.push(literal_key(e));
                        }
                        Argument::Named { .. } => return Shape::Open,
                    }
                }
                self.call_shape(callee, &lits, args, env)
            }
            Expr::Pipeline { left, right } => {
                let mut args = vec![self.shape(left, env)];
                let mut lits = vec![None];
                match right.as_ref() {
                    Expr::Call { callee, arguments } => {
                        for a in arguments {
                            match a {
                                Argument::Positional(e) => {
                                    args.push(self.shape(e, env));
                                    lits.push(literal_key(e));
                                }
                                Argument::Named { .. } => return Shape::Open,
                            }
                        }
                        self.call_shape(callee, &lits, args, env)
                    }
                    callee @ Expr::Identifier(_) => self.call_shape(callee, &lits, args, env),
                    _ => Shape::Open,
                }
            }
            _ => Shape::Open,
        }
    }

    fn call_shape(
        &self,
        callee: &Expr,
        lits: &[Option<String>],
        args: Vec<Shape>,
        env: &Env,
    ) -> Shape {
        let Expr::Identifier(name) = callee else {
            return Shape::Open;
        };
        if env.contains_key(name) {
            return Shape::Open; // a binding of that name, not the builtin
        }
        let key_at = |i: usize| -> Option<String> { lits.get(i).cloned().flatten() };
        match name.as_str() {
            "map_set" if args.len() == 3 => match key_at(1) {
                Some(k) => args[0].with_key(&k, args[2].clone()),
                None => Shape::Open,
            },
            "map_update" if args.len() == 3 => match key_at(1) {
                Some(k) => args[0].with_key(&k, Shape::Open),
                None => Shape::Open,
            },
            "map_remove" if args.len() == 2 => args[0].clone(),
            "map_merge" if args.len() == 2 => match (&args[0], &args[1]) {
                (Shape::Open, _) | (_, Shape::Open) => Shape::Open,
                _ => args[0].join(&args[1]),
            },
            "map_clear" if args.len() == 1 => match &args[0] {
                Shape::Keys(_) => Shape::Keys(BTreeMap::new()),
                Shape::Bottom => Shape::Bottom,
                Shape::Open => Shape::Open,
            },
            "map_get" if args.len() == 2 => match key_at(1) {
                Some(k) => args[0].key(&k),
                None => Shape::Open,
            },
            n if n == crate::parser::SPREAD_BUILTIN => {
                let mut out = Shape::Keys(BTreeMap::new());
                for a in &args {
                    match a {
                        Shape::Open => return Shape::Open,
                        _ => out = out.join(a),
                    }
                }
                out
            }
            _ => match self.funcs.get(name.as_str()) {
                Some(f) if !f.escapes && !env.contains_key(name) => f.ret.clone(),
                _ => Shape::Open,
            },
        }
    }

    fn report_stmt(
        &self,
        stmt: &Statement,
        span: (u32, u32),
        env: &Env,
        out: &mut Vec<KeyFinding>,
    ) {
        let span = located(stmt, span);
        match stmt.unwrapped() {
            Statement::Expression(e) => self.report_expr(e, span, env, out, false),
            Statement::LetDecl(l) | Statement::ShareDecl(crate::ast::ShareDecl::Let(l)) => {
                if let Some(v) = &l.value {
                    self.report_expr(v, span, env, out, false);
                }
            }
            Statement::FunctionDecl(f) => self.report_expr(&f.body, span, env, out, false),
            _ => {}
        }
    }

    /// `presence`: the value is compared with `()` or defaulted by `??`.
    fn report_expr(
        &self,
        e: &Expr,
        span: (u32, u32),
        env: &Env,
        out: &mut Vec<KeyFinding>,
        presence: bool,
    ) {
        match e {
            Expr::FieldAccess { object, field } => {
                if !presence
                    && let Shape::Keys(m) = self.shape(object, env)
                    && !m.contains_key(field)
                {
                    out.push(KeyFinding {
                        line: span.0,
                        column: span.1,
                        key: field.clone(),
                        receiver: chain_text(object).unwrap_or_default(),
                        known: m.keys().cloned().collect(),
                    });
                }
                self.report_expr(object, span, env, out, false);
                return;
            }
            // `x.f(...)`: a key or a method; the receiver is still read
            Expr::Call { callee, arguments } => {
                match callee.as_ref() {
                    Expr::FieldAccess { object, .. } => {
                        self.report_expr(object, span, env, out, false)
                    }
                    other => self.report_expr(other, span, env, out, false),
                }
                for a in arguments {
                    let (Argument::Positional(v) | Argument::Named { value: v, .. }) = a;
                    self.report_expr(v, span, env, out, false);
                }
                return;
            }
            Expr::BinaryOp { left, op, right } => {
                use crate::ast::BinaryOp as B;
                let unit = |x: &Expr| matches!(x, Expr::Tuple(t) if t.is_empty());
                let (lp, rp) = match op {
                    B::Coalesce => (true, false),
                    B::Equal | B::NotEqual => (unit(right), unit(left)),
                    _ => (false, false),
                };
                self.report_expr(left, span, env, out, lp);
                self.report_expr(right, span, env, out, rp);
                return;
            }
            Expr::Block(stmts) => {
                for s in stmts {
                    self.report_stmt(s, span, env, out);
                }
                return;
            }
            _ => {}
        }
        for c in children(e) {
            self.report_expr(c, span, env, out, false);
        }
    }
}

fn literal_key(e: &Expr) -> Option<String> {
    match e {
        Expr::String(s) => Some(s.as_ref().clone()),
        Expr::Integer(i) => Some(i.to_string()),
        Expr::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

fn chain_text(e: &Expr) -> Option<String> {
    match e {
        Expr::Identifier(n) | Expr::LocalRef { name: n, .. } => Some(n.clone()),
        Expr::FieldAccess { object, field } => Some(format!("{}.{}", chain_text(object)?, field)),
        _ => None,
    }
}

fn bind_pattern(p: &Pattern, next: &mut Env, bind: &mut impl FnMut(&str, Shape, &mut Env)) {
    let mut names = Vec::new();
    pattern_names(p, &mut names);
    for n in names {
        bind(&n, Shape::Open, next);
    }
}

fn pattern_names(p: &Pattern, out: &mut Vec<String>) {
    match p {
        Pattern::Identifier(n) | Pattern::Rest(n) => out.push(n.clone()),
        Pattern::List { patterns, rest } => {
            for q in patterns {
                pattern_names(q, out);
            }
            if let Some(r) = rest {
                out.push(r.clone());
            }
        }
        Pattern::Tuple(ps) | Pattern::EnumVariant { patterns: ps, .. } => {
            for q in ps {
                pattern_names(q, out);
            }
        }
        Pattern::Ok(q) | Pattern::Err(q) => pattern_names(q, out),
        Pattern::Struct { field_patterns, .. } | Pattern::AnonymousStruct { field_patterns } => {
            for (_, q) in field_patterns {
                pattern_names(q, out);
            }
        }
        Pattern::Or { alternatives } => {
            for q in alternatives {
                pattern_names(q, out);
            }
        }
        Pattern::Guarded { pattern, .. } => pattern_names(pattern, out),
        _ => {}
    }
}

enum CallArg<'e> {
    Positional(&'e Expr),
    Named(&'e String, &'e Expr),
    #[allow(dead_code)]
    Shape(Shape),
}

/// Every direct call by name: `f(a, b)`, `a |> f(b)`, `a |> f`.
fn collect_calls<'e>(e: &'e Expr, out: &mut Vec<(&'e str, Vec<CallArg<'e>>)>) {
    match e {
        Expr::Call { callee, arguments } => {
            if let Expr::Identifier(n) = callee.as_ref() {
                out.push((
                    n.as_str(),
                    arguments
                        .iter()
                        .map(|a| match a {
                            Argument::Positional(v) => CallArg::Positional(v),
                            Argument::Named { name, value } => CallArg::Named(name, value),
                        })
                        .collect(),
                ));
            }
        }
        Expr::Pipeline { left, right } => match right.as_ref() {
            Expr::Call { callee, arguments } => {
                if let Expr::Identifier(n) = callee.as_ref() {
                    let mut args = vec![CallArg::Positional(left.as_ref())];
                    args.extend(arguments.iter().map(|a| match a {
                        Argument::Positional(v) => CallArg::Positional(v),
                        Argument::Named { name, value } => CallArg::Named(name, value),
                    }));
                    out.push((n.as_str(), args));
                }
                collect_calls(left, out);
                for a in arguments {
                    let (Argument::Positional(v) | Argument::Named { value: v, .. }) = a;
                    collect_calls(v, out);
                }
                return;
            }
            Expr::Identifier(n) => {
                out.push((n.as_str(), vec![CallArg::Positional(left.as_ref())]));
                collect_calls(left, out);
                return;
            }
            _ => {}
        },
        Expr::Block(stmts) => {
            for s in stmts {
                collect_calls_stmt(s, out);
            }
            return;
        }
        _ => {}
    }
    for c in children(e) {
        collect_calls(c, out);
    }
}

fn collect_calls_stmt<'e>(s: &'e Statement, out: &mut Vec<(&'e str, Vec<CallArg<'e>>)>) {
    match s.unwrapped() {
        Statement::Expression(e) => collect_calls(e, out),
        Statement::LetDecl(l) | Statement::ShareDecl(crate::ast::ShareDecl::Let(l)) => {
            if let Some(v) = &l.value {
                collect_calls(v, out);
            }
        }
        Statement::FunctionDecl(f) => collect_calls(&f.body, out),
        Statement::TestDecl(t) => {
            for s in &t.body {
                collect_calls_stmt(s, out);
            }
        }
        _ => {}
    }
}

/// `return e` in a function's own body (not a lambda's).
fn returns(e: &Expr) -> Vec<Option<&Expr>> {
    let mut out = Vec::new();
    fn walk<'e>(e: &'e Expr, out: &mut Vec<Option<&'e Expr>>) {
        match e {
            Expr::Return(v) => out.push(v.as_deref()),
            Expr::Lambda { .. } => return,
            Expr::Block(stmts) => {
                for s in stmts {
                    match s.unwrapped() {
                        Statement::Expression(e) => walk(e, out),
                        Statement::LetDecl(l) => {
                            if let Some(v) = &l.value {
                                walk(v, out);
                            }
                        }
                        _ => {}
                    }
                }
                return;
            }
            _ => {}
        }
        for c in children(e) {
            walk(c, out);
        }
    }
    walk(e, &mut out);
    out
}

/// Names referenced other than as a call's callee (a function so named
/// escapes), and names assigned inside functions.
fn scan_stmt(s: &Statement, escaped: &mut HashSet<String>, assigned: &mut HashSet<String>) {
    match s.unwrapped() {
        Statement::Expression(e) => scan(e, escaped, assigned, false),
        Statement::LetDecl(l) | Statement::ShareDecl(crate::ast::ShareDecl::Let(l)) => {
            if let Some(v) = &l.value {
                scan(v, escaped, assigned, false);
            }
        }
        Statement::FunctionDecl(f) | Statement::ShareDecl(crate::ast::ShareDecl::Function(f)) => {
            for p in &f.parameters {
                if let Some(d) = &p.default_value {
                    scan(d, escaped, assigned, true);
                }
            }
            scan(&f.body, escaped, assigned, true);
        }
        Statement::ImplDecl(i) | Statement::ShareDecl(crate::ast::ShareDecl::Impl(i)) => {
            for m in &i.methods {
                scan(&m.body, escaped, assigned, true);
            }
        }
        Statement::TraitDecl(t) | Statement::ShareDecl(crate::ast::ShareDecl::Trait(t)) => {
            for m in &t.methods {
                if let Some(b) = &m.default_body {
                    scan(b, escaped, assigned, true);
                }
            }
        }
        Statement::TestDecl(t) => {
            for s in &t.body {
                scan_stmt(s, escaped, assigned);
            }
        }
        _ => {}
    }
}

fn scan(e: &Expr, escaped: &mut HashSet<String>, assigned: &mut HashSet<String>, in_fn: bool) {
    match e {
        Expr::Identifier(n) | Expr::LocalRef { name: n, .. } => {
            escaped.insert(n.clone());
            return;
        }
        Expr::Assignment { target, value }
        | Expr::LocalAssign {
            name: target,
            value,
            ..
        } => {
            if in_fn {
                assigned.insert(target.clone());
            }
            scan(value, escaped, assigned, in_fn);
            return;
        }
        Expr::Call { callee, arguments } => {
            if !matches!(callee.as_ref(), Expr::Identifier(_)) {
                scan(callee, escaped, assigned, in_fn);
            }
            for a in arguments {
                let (Argument::Positional(v) | Argument::Named { value: v, .. }) = a;
                scan(v, escaped, assigned, in_fn);
            }
            return;
        }
        Expr::Pipeline { left, right } => {
            scan(left, escaped, assigned, in_fn);
            match right.as_ref() {
                Expr::Identifier(_) => {}
                Expr::Call { .. } => scan(right, escaped, assigned, in_fn),
                other => scan(other, escaped, assigned, in_fn),
            }
            return;
        }
        Expr::Block(stmts) => {
            for s in stmts {
                match s.unwrapped() {
                    Statement::Expression(e) => scan(e, escaped, assigned, in_fn),
                    Statement::LetDecl(l) => {
                        if let Some(v) = &l.value {
                            scan(v, escaped, assigned, in_fn);
                        }
                    }
                    Statement::FunctionDecl(f) => scan(&f.body, escaped, assigned, true),
                    _ => {}
                }
            }
            return;
        }
        _ => {}
    }
    for c in children(e) {
        scan(c, escaped, assigned, in_fn);
    }
}

/// An expression's direct sub-expressions (a block's statements are
/// walked by the callers that care about them).
pub(crate) fn children(e: &Expr) -> Vec<&Expr> {
    let mut v: Vec<&Expr> = Vec::new();
    match e {
        Expr::List(items) => v.extend(items.iter()),
        Expr::Tuple(items) => v.extend(items.iter()),
        Expr::LocalAssign { value, .. } | Expr::Assignment { value, .. } => v.push(value),
        Expr::Call { callee, arguments } => {
            v.push(callee);
            for a in arguments {
                let (Argument::Positional(x) | Argument::Named { value: x, .. }) = a;
                v.push(x);
            }
        }
        Expr::Lambda {
            parameters, body, ..
        } => {
            for p in parameters {
                if let Some(d) = &p.default_value {
                    v.push(d);
                }
            }
            v.push(body);
        }
        Expr::Pipeline { left, right } => {
            v.push(left);
            v.push(right);
        }
        Expr::Match { value, arms } => {
            v.push(value);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    v.push(g);
                }
                v.push(&arm.expression);
            }
        }
        Expr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            v.push(condition);
            v.push(then_branch);
            if let Some(e) = else_branch {
                v.push(e);
            }
        }
        Expr::Range { start, end, .. } => {
            v.push(start);
            v.push(end);
        }
        Expr::Block(stmts) => {
            for s in stmts {
                match s.unwrapped() {
                    Statement::Expression(e) => v.push(e),
                    Statement::LetDecl(l) => {
                        if let Some(x) = &l.value {
                            v.push(x);
                        }
                    }
                    Statement::FunctionDecl(f) => v.push(&f.body),
                    _ => {}
                }
            }
        }
        Expr::BinaryOp { left, right, .. } | Expr::BitwiseOp { left, right, .. } => {
            v.push(left);
            v.push(right);
        }
        Expr::UnaryOp { operand, .. } => v.push(operand),
        Expr::StructLiteral(s) => v.extend(s.fields.iter().map(|f| &f.value)),
        Expr::AnonymousObject { fields } => v.extend(fields.iter().map(|f| &f.value)),
        Expr::MapLiteral { entries } => {
            for en in entries {
                v.push(&en.key);
                v.push(&en.value);
            }
        }
        Expr::FieldAccess { object, .. } => v.push(object),
        Expr::ResultOk(x) | Expr::ResultErr(x) | Expr::Try(x) | Expr::Spawn(x) => v.push(x),
        Expr::Spread(x) | Expr::Rest(x) => v.push(x),
        Expr::ForLoop { iterable, body, .. } | Expr::ParForLoop { iterable, body, .. } => {
            v.push(iterable);
            v.push(body);
        }
        Expr::WhileLoop { condition, body } => {
            v.push(condition);
            v.push(body);
        }
        Expr::Loop { body } => v.push(body),
        Expr::Break(Some(x)) | Expr::Return(Some(x)) => v.push(x),
        Expr::Index { object, index } => {
            v.push(object);
            v.push(index);
        }
        Expr::TemplateString { parts } => {
            for p in parts {
                if let TemplatePart::Interpolation(x) = p {
                    v.push(x);
                }
            }
        }
        Expr::AssertEq {
            actual, expected, ..
        }
        | Expr::AssertNe {
            actual, expected, ..
        } => {
            v.push(actual);
            v.push(expected);
        }
        Expr::Assert { condition, .. } => v.push(condition),
        Expr::AssertTrue { expression, .. } | Expr::AssertFalse { expression, .. } => {
            v.push(expression)
        }
        _ => {}
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(src: &str) -> Vec<String> {
        let p = crate::Parser::new().parse_raw(src).expect("parses");
        impossible_reads(&p)
            .into_iter()
            .map(|f| format!("{}:{}", f.line, f.key))
            .collect()
    }

    #[test]
    fn a_misspelt_key_of_a_literal_is_found() {
        let src = "fn main() = {\n    let p = #{ \"running\": true, \"live_id\": 3 }\n    if p.runing == true => p.live_id else => 0\n}\n";
        assert_eq!(found(src), vec!["3:runing"]);
    }

    #[test]
    fn through_parameters_returns_and_writes() {
        let src = r#"
fn make(n) = { a: n, pp: #{ "cur": 1 } }
fn bump(s) = map_set(s, "seen", true)
fn read(s) = s.pp.cur + s.pp.nope
fn flag(s) = s.seen
fn main() = {
    let s = bump(make(1))
    read(s) + (if flag(s) => 1 else => 0) + s.b
}
"#;
        assert_eq!(found(src), vec!["4:nope", "8:b"]);
    }

    #[test]
    fn unknowns_are_never_reported() {
        for src in [
            // shared: callers elsewhere
            "share fn f(m) = m.x\nlet y = f(#{ \"a\": 1 })\n",
            // used as a value
            "fn f(m) = m.x\nlet y = map([#{ \"a\": 1 }], f)\nlet z = f(#{ \"a\": 1 })\n",
            // a computed key
            "fn main(k) = {\n let m = map_set(#{ \"a\": 1 }, k, 2)\n m.b\n}\n",
            // presence tests and defaults
            "fn main() = {\n let m = #{ \"a\": 1 }\n [m.b == (), m.c ?? 0, () == m.d]\n}\n",
            // given something unknown somewhere
            "fn main(x) = {\n let mut m = #{ \"a\": 1 }\n m = x\n m.b\n}\n",
            // a pipeline of writes
            "fn main() = {\n let m = #{ \"a\": 1 } |> map_set(\"b\", 2)\n m.b\n}\n",
        ] {
            assert!(found(src).is_empty(), "{src}: {:?}", found(src));
        }
    }

    #[test]
    fn spreads_and_nested_writes_carry_keys() {
        let src = "fn main() = {\n let a = #{ \"x\": 1 }\n let b = #{ ...a, \"y\": 2 }\n b.x + b.y + b.z\n}\n";
        assert_eq!(found(src), vec!["4:z"]);
    }

    #[test]
    fn keys_for_completion() {
        let p = crate::Parser::new()
            .parse_raw("fn main() = {\n let opts = #{ \"pad\": 1, \"width\": 2 }\n opts\n}\n")
            .unwrap();
        assert_eq!(
            keys_of_name(&p, "opts"),
            Some(vec!["pad".to_string(), "width".to_string()])
        );
    }
}

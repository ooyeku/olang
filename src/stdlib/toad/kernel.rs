//! A kernel: a plain olang function, `fn name(p, w) = #{ "field": … }`,
//! lowered to a small scalar program that runs over a whole swarm at once,
//! on the CPU ([`super::cpu`]) or the GPU ([`super::wgsl`]).
//!
//! `p` is one entity's fields, `w` the world (`w.dt` and the swarm's
//! uniforms). The body is `let`s and one map whose entries are the
//! fields' new values; a field it does not name keeps its value, and
//! `"dead": <bool>` removes the entity. What a kernel may say: numbers,
//! pairs (`(x, y)` or `v2(x, y)`, with `.x` and `.y`), booleans,
//! arithmetic, comparisons, `&&` `||` `!`, `if`, and the math helpers in
//! `Lower::call`. A name the kernel does not bind is looked up in the
//! function's closure, then its module file's scope, then the caller's
//! scope, and must be a number, a pair or a boolean: a constant, baked in
//! when the kernel is compiled.
//!
//! Lowering splits every pair into two scalars, so the program is a list
//! of scalar operations in SSA order: each [`Op`] reads earlier ones.

use crate::ast::{Argument, BinaryOp, Expr, Function, Pattern, Statement, UnaryOp, Value};
use std::collections::HashMap;

/// A field's or a uniform's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    F,
    V2,
}

impl Ty {
    pub fn width(self) -> usize {
        match self {
            Ty::F => 1,
            Ty::V2 => 2,
        }
    }
}

/// A named slot made of `ty.width()` scalars starting at `at`.
#[derive(Clone, Debug)]
pub struct Slot {
    pub name: String,
    pub ty: Ty,
    pub at: usize,
}

/// A list of named slots laid out one after another.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub slots: Vec<Slot>,
    pub width: usize,
}

impl Layout {
    pub fn push(&mut self, name: &str, ty: Ty) {
        self.slots.push(Slot {
            name: name.to_string(),
            ty,
            at: self.width,
        });
        self.width += ty.width();
    }
    pub fn get(&self, name: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.name == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum B {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    Min,
    Max,
    Pow,
    Atan2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum U {
    Neg,
    Not,
    Sqrt,
    Abs,
    Floor,
    Ceil,
    Fract,
    Sin,
    Cos,
    Tan,
    Exp,
    Ln,
    Sign,
}

/// One scalar operation; operands are earlier ops' indices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Const(f32),
    /// The entity's scalar column `c`.
    Col(usize),
    /// The world's scalar `u` (0 is `dt`).
    Uni(usize),
    Bin(B, usize, usize),
    Un(U, usize),
    /// `c ? a : b` (c is 0 or 1).
    Sel(usize, usize, usize),
}

/// A compiled kernel.
#[derive(Clone, Debug)]
pub struct Program {
    pub name: String,
    pub fields: Layout,
    pub uniforms: Layout,
    pub ops: Vec<Op>,
    /// (column, op): the columns the kernel writes, after every op ran.
    pub writes: Vec<(usize, usize)>,
    /// The op that says the entity is dead, when the kernel says so.
    pub dead: Option<usize>,
    /// Each field scalar's value for an entity spawned without it.
    pub defaults: Vec<f32>,
}

/// A value while lowering: a scalar, a pair, or a boolean (0 or 1).
#[derive(Clone, Copy, Debug)]
enum V {
    S(usize),
    P(usize, usize),
    Bool(usize),
}

struct Lower<'a> {
    f: &'a Function,
    scope: &'a dyn Fn(Option<&str>, &str) -> Option<Value>,
    fields: &'a Layout,
    uniforms: &'a Layout,
    p: String,
    w: String,
    ops: Vec<Op>,
    memo: HashMap<OpKey, usize>,
    scopes: Vec<HashMap<String, V>>,
}

/// Ops are hashed to share common subexpressions (`p.life` read twice is
/// one column read).
#[derive(Hash, PartialEq, Eq)]
struct OpKey(u8, u64, u64, u64);

fn key(op: &Op) -> OpKey {
    match *op {
        Op::Const(c) => OpKey(0, c.to_bits() as u64, 0, 0),
        Op::Col(c) => OpKey(1, c as u64, 0, 0),
        Op::Uni(u) => OpKey(2, u as u64, 0, 0),
        Op::Bin(b, x, y) => OpKey(3, b as u64, x as u64, y as u64),
        Op::Un(u, x) => OpKey(4, u as u64, x as u64, 0),
        Op::Sel(c, a, b) => OpKey(5, c as u64, a as u64, b as u64),
    }
}

type R<T> = Result<T, String>;

impl<'a> Lower<'a> {
    fn err<T>(&self, what: impl std::fmt::Display) -> R<T> {
        Err(format!(
            "toad: the kernel {} cannot run on a swarm: {}",
            self.f.name.as_deref().unwrap_or("(a lambda)"),
            what
        ))
    }

    fn op(&mut self, op: Op) -> usize {
        // fold constants so a kernel's arithmetic on literals costs nothing
        let folded = match op {
            Op::Bin(b, x, y) => match (self.ops[x], self.ops[y]) {
                (Op::Const(a), Op::Const(c)) => Some(Op::Const(super::cpu::bin(b, a, c))),
                _ => None,
            },
            Op::Un(u, x) => match self.ops[x] {
                Op::Const(a) => Some(Op::Const(super::cpu::un(u, a))),
                _ => None,
            },
            _ => None,
        };
        let op = folded.unwrap_or(op);
        let k = key(&op);
        if let Some(&i) = self.memo.get(&k) {
            return i;
        }
        self.ops.push(op);
        let i = self.ops.len() - 1;
        self.memo.insert(k, i);
        i
    }

    fn k(&mut self, c: f32) -> usize {
        self.op(Op::Const(c))
    }

    fn lookup(&self, name: &str) -> Option<V> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn constant(&mut self, name: &str) -> R<V> {
        let num = |v: &Value| match v {
            Value::Integer(n) => Some(*n as f32),
            Value::Float(x) => Some(*x as f32),
            _ => None,
        };
        let found = self.f.closure.get(name).cloned().or_else(|| (self.scope)(self.f.def_file.as_deref(), name));
        match found.as_ref() {
            Some(v) if num(v).is_some() => Ok(V::S(self.k(num(v).unwrap()))),
            Some(Value::Boolean(b)) => Ok(V::Bool(self.k(if *b { 1.0 } else { 0.0 }))),
            Some(Value::Tuple(t)) if t.len() == 2 && t.iter().all(|x| num(x).is_some()) => {
                let x = self.k(num(&t[0]).unwrap());
                let y = self.k(num(&t[1]).unwrap());
                Ok(V::P(x, y))
            }
            Some(other) => self.err(format!(
                "`{}` is {}; a kernel can use only numbers, pairs and booleans from outside it",
                name,
                other.type_name()
            )),
            None => self.err(format!("`{}` is not defined", name)),
        }
    }

    fn name(&mut self, name: &str) -> R<V> {
        if let Some(v) = self.lookup(name) {
            return Ok(v);
        }
        if name == self.p || name == self.w {
            return self.err(format!(
                "`{}` is used whole; a kernel reads its fields (`{}.pos`)",
                name, name
            ));
        }
        self.constant(name)
    }

    fn slot(&mut self, l: &Layout, name: &str, col: bool, whose: &str) -> R<V> {
        let Some(s) = l.get(name) else {
            let known: Vec<&str> = l.slots.iter().map(|s| s.name.as_str()).collect();
            return self.err(format!(
                "{} has no `{}` (it has {})",
                whose,
                name,
                known.join(", ")
            ));
        };
        let (at, ty) = (s.at, s.ty);
        let mk = |me: &mut Self, i| me.op(if col { Op::Col(i) } else { Op::Uni(i) });
        Ok(match ty {
            Ty::F => V::S(mk(self, at)),
            Ty::V2 => {
                let x = mk(self, at);
                let y = mk(self, at + 1);
                V::P(x, y)
            }
        })
    }

    fn scalar(&mut self, e: &Expr) -> R<usize> {
        match self.expr(e)? {
            V::S(i) => Ok(i),
            V::Bool(_) => self.err("a boolean is used as a number"),
            V::P(..) => self.err("a pair is used as a number"),
        }
    }

    fn boolean(&mut self, e: &Expr) -> R<usize> {
        match self.expr(e)? {
            V::Bool(i) => Ok(i),
            _ => self.err("a condition is not a boolean (compare it: `x > 0.0`)"),
        }
    }

    fn bin(&mut self, b: B, x: V, y: V) -> R<V> {
        use V::*;
        let arith = matches!(b, B::Add | B::Sub | B::Mul | B::Div | B::Rem | B::Min | B::Max | B::Pow);
        Ok(match (x, y) {
            (S(a), S(c)) => {
                let r = self.op(Op::Bin(b, a, c));
                if arith || b == B::Atan2 { S(r) } else { Bool(r) }
            }
            (P(ax, ay), P(cx, cy)) if arith => {
                let rx = self.op(Op::Bin(b, ax, cx));
                let ry = self.op(Op::Bin(b, ay, cy));
                P(rx, ry)
            }
            (P(ax, ay), S(c)) if arith => {
                let rx = self.op(Op::Bin(b, ax, c));
                let ry = self.op(Op::Bin(b, ay, c));
                P(rx, ry)
            }
            (S(a), P(cx, cy)) if arith => {
                let rx = self.op(Op::Bin(b, a, cx));
                let ry = self.op(Op::Bin(b, a, cy));
                P(rx, ry)
            }
            (Bool(a), Bool(c)) if matches!(b, B::And | B::Or | B::Eq | B::Ne) => {
                let r = match b {
                    B::And => self.op(Op::Bin(B::Mul, a, c)),
                    B::Or => self.op(Op::Bin(B::Max, a, c)),
                    _ => self.op(Op::Bin(b, a, c)),
                };
                Bool(r)
            }
            (P(..), P(..)) => return self.err("pairs compare only by their parts (`a.x < b.x`)"),
            _ => return self.err(format!("{:?} does not take these two kinds of value", b)),
        })
    }

    fn sel(&mut self, c: usize, a: V, b: V) -> R<V> {
        use V::*;
        Ok(match (a, b) {
            (S(x), S(y)) => S(self.op(Op::Sel(c, x, y))),
            (Bool(x), Bool(y)) => Bool(self.op(Op::Sel(c, x, y))),
            (P(ax, ay), P(bx, by)) => {
                let x = self.op(Op::Sel(c, ax, bx));
                let y = self.op(Op::Sel(c, ay, by));
                P(x, y)
            }
            _ => return self.err("an `if`'s two branches are different kinds of value"),
        })
    }

    fn call(&mut self, name: &str, args: &[Argument]) -> R<V> {
        let mut vs = Vec::new();
        for a in args {
            match a {
                Argument::Positional(e) => vs.push(self.expr(e)?),
                Argument::Named { name, .. } => {
                    return self.err(format!("a named argument (`{}`) in a kernel", name));
                }
            }
        }
        let un = |u| Some(u);
        let one = match name {
            "sqrt" => un(U::Sqrt),
            "abs" => un(U::Abs),
            "floor" => un(U::Floor),
            "ceil" => un(U::Ceil),
            "fract" => un(U::Fract),
            "sin" => un(U::Sin),
            "cos" => un(U::Cos),
            "tan" => un(U::Tan),
            "exp" => un(U::Exp),
            "ln" | "log" => un(U::Ln),
            "sign" => un(U::Sign),
            _ => None,
        };
        if let Some(u) = one {
            return match vs.as_slice() {
                [V::S(x)] => Ok(V::S(self.op(Op::Un(u, *x)))),
                [V::P(x, y)] => {
                    let rx = self.op(Op::Un(u, *x));
                    let ry = self.op(Op::Un(u, *y));
                    Ok(V::P(rx, ry))
                }
                _ => self.err(format!("`{}` takes one number or pair", name)),
            };
        }
        match (name, vs.as_slice()) {
            ("min", [a, b]) => self.bin(B::Min, *a, *b),
            ("max", [a, b]) => self.bin(B::Max, *a, *b),
            ("pow", [a, b]) => self.bin(B::Pow, *a, *b),
            ("atan2", [V::S(y), V::S(x)]) => Ok(V::S(self.op(Op::Bin(B::Atan2, *y, *x)))),
            ("v2", [V::S(x), V::S(y)]) => Ok(V::P(*x, *y)),
            ("clamp", [v, lo, hi]) => {
                let m = self.bin(B::Max, *v, *lo)?;
                self.bin(B::Min, m, *hi)
            }
            ("mix" | "lerp", [a, b, t]) => {
                let d = self.bin(B::Sub, *b, *a)?;
                let s = self.bin(B::Mul, d, *t)?;
                self.bin(B::Add, *a, s)
            }
            ("dot", [V::P(ax, ay), V::P(bx, by)]) => {
                let x = self.op(Op::Bin(B::Mul, *ax, *bx));
                let y = self.op(Op::Bin(B::Mul, *ay, *by));
                Ok(V::S(self.op(Op::Bin(B::Add, x, y))))
            }
            ("length", [V::P(x, y)]) => {
                let xx = self.op(Op::Bin(B::Mul, *x, *x));
                let yy = self.op(Op::Bin(B::Mul, *y, *y));
                let s = self.op(Op::Bin(B::Add, xx, yy));
                Ok(V::S(self.op(Op::Un(U::Sqrt, s))))
            }
            ("normalize", [V::P(x, y)]) => {
                let xx = self.op(Op::Bin(B::Mul, *x, *x));
                let yy = self.op(Op::Bin(B::Mul, *y, *y));
                let s = self.op(Op::Bin(B::Add, xx, yy));
                let l = self.op(Op::Un(U::Sqrt, s));
                let tiny = self.k(1e-12);
                let l = self.op(Op::Bin(B::Max, l, tiny));
                let rx = self.op(Op::Bin(B::Div, *x, l));
                let ry = self.op(Op::Bin(B::Div, *y, l));
                Ok(V::P(rx, ry))
            }
            _ => self.err(format!(
                "`{}` with {} argument(s) is not something a kernel can call",
                name,
                vs.len()
            )),
        }
    }

    fn block(&mut self, stmts: &[Statement]) -> R<V> {
        self.scopes.push(HashMap::new());
        let mut last = None;
        for (i, s) in stmts.iter().enumerate() {
            let mut s = s;
            while let Statement::Located { stmt, .. } = s {
                s = stmt;
            }
            match s {
                Statement::LetDecl(l) => {
                    let Pattern::Identifier(n) = &l.pattern else {
                        return self.err("a `let` that is not a plain name");
                    };
                    let Some(v) = &l.value else {
                        return self.err(format!("`let {}` has no value", n));
                    };
                    let v = self.expr(v)?;
                    self.scopes.last_mut().unwrap().insert(n.clone(), v);
                }
                Statement::Expression(e) if i + 1 == stmts.len() => last = Some(self.expr(e)?),
                _ => return self.err("a block holds `let`s and then one value"),
            }
        }
        self.scopes.pop();
        match last {
            Some(v) => Ok(v),
            None => self.err("a block has no value at its end"),
        }
    }

    fn expr(&mut self, e: &Expr) -> R<V> {
        match e {
            Expr::Integer(n) => Ok(V::S(self.k(*n as f32))),
            Expr::Float(x) => Ok(V::S(self.k(*x as f32))),
            Expr::Boolean(b) => Ok(V::Bool(self.k(if *b { 1.0 } else { 0.0 }))),
            Expr::Identifier(n) | Expr::LocalRef { name: n, .. } => self.name(n),
            Expr::Tuple(items) if items.len() == 2 => {
                let x = self.scalar(&items[0])?;
                let y = self.scalar(&items[1])?;
                Ok(V::P(x, y))
            }
            Expr::FieldAccess { object, field } => {
                if let Expr::Identifier(n) | Expr::LocalRef { name: n, .. } = object.as_ref() {
                    if self.lookup(n).is_none() {
                        if *n == self.p {
                            let l = self.fields;
                            return self.slot(l, field, true, "the swarm");
                        }
                        if *n == self.w {
                            let l = self.uniforms;
                            return self.slot(l, field, false, "the world");
                        }
                    }
                }
                match (self.expr(object)?, field.as_str()) {
                    (V::P(x, _), "x") => Ok(V::S(x)),
                    (V::P(_, y), "y") => Ok(V::S(y)),
                    _ => self.err(format!("`.{}` of something that is not a pair", field)),
                }
            }
            Expr::BinaryOp { left, op, right } => {
                let b = match op {
                    BinaryOp::Add => B::Add,
                    BinaryOp::Subtract => B::Sub,
                    BinaryOp::Multiply => B::Mul,
                    BinaryOp::Divide => B::Div,
                    BinaryOp::Modulo => B::Rem,
                    BinaryOp::LessThan => B::Lt,
                    BinaryOp::LessThanEqual => B::Le,
                    BinaryOp::GreaterThan => B::Gt,
                    BinaryOp::GreaterThanEqual => B::Ge,
                    BinaryOp::Equal => B::Eq,
                    BinaryOp::NotEqual => B::Ne,
                    BinaryOp::And => B::And,
                    BinaryOp::Or => B::Or,
                    other => return self.err(format!("the operator `{}`", other.symbol())),
                };
                let x = self.expr(left)?;
                let y = self.expr(right)?;
                self.bin(b, x, y)
            }
            Expr::UnaryOp { op, operand } => match (op, self.expr(operand)?) {
                (UnaryOp::Negate, V::S(x)) => Ok(V::S(self.op(Op::Un(U::Neg, x)))),
                (UnaryOp::Negate, V::P(x, y)) => {
                    let rx = self.op(Op::Un(U::Neg, x));
                    let ry = self.op(Op::Un(U::Neg, y));
                    Ok(V::P(rx, ry))
                }
                (UnaryOp::Not, V::Bool(x)) => Ok(V::Bool(self.op(Op::Un(U::Not, x)))),
                _ => self.err("`-` takes a number or a pair, `!` a boolean"),
            },
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let c = self.boolean(condition)?;
                let a = self.expr(then_branch)?;
                let Some(eb) = else_branch else {
                    return self.err("an `if` without an `else`");
                };
                let b = self.expr(eb)?;
                self.sel(c, a, b)
            }
            Expr::Block(stmts) => self.block(stmts),
            Expr::Call { callee, arguments } => match callee.as_ref() {
                Expr::Identifier(n) | Expr::LocalRef { name: n, .. } => self.call(n, arguments),
                Expr::FieldAccess { object, field } => match object.as_ref() {
                    Expr::Identifier(m) if m == "math" => self.call(field, arguments),
                    _ => self.err(format!("the call `.{}(…)`", field)),
                },
                _ => self.err("a call of something that is not a name"),
            },
            Expr::MapLiteral { .. } => self.err("a map that is not the kernel's result"),
            Expr::String(_) | Expr::TemplateString { .. } | Expr::RawString(_) => {
                self.err("a string (a GPU has none)")
            }
            Expr::List(_) => self.err("a list (a kernel works on one entity's numbers)"),
            Expr::Match { .. } => self.err("`match` (not yet; use `if`)"),
            other => self.err(format!("{}", describe(other))),
        }
    }
}

fn describe(e: &Expr) -> &'static str {
    match e {
        Expr::ForLoop { .. } | Expr::WhileLoop { .. } | Expr::Loop { .. } => "a loop",
        Expr::Lambda { .. } => "a function inside it",
        Expr::Pipeline { .. } => "a pipeline (`|>`)",
        Expr::Index { .. } => "indexing",
        Expr::Spawn(_) => "`spawn`",
        Expr::Try(_) => "`?`",
        _ => "this kind of expression",
    }
}

/// Lower `f` against the swarm's fields and the world's uniforms.
pub fn compile(
    f: &Function,
    fields: &Layout,
    uniforms: &Layout,
    scope: &dyn Fn(Option<&str>, &str) -> Option<Value>,
) -> Result<Program, String> {
    let name = f.name.clone().unwrap_or_else(|| "(a lambda)".to_string());
    if f.parameters.len() != 2 {
        return Err(format!(
            "toad: the kernel {} takes {} parameter(s); a kernel is fn {}(p, w): the entity and the world",
            name,
            f.parameters.len(),
            name
        ));
    }
    let mut l = Lower {
        f,
        scope,
        fields,
        uniforms,
        p: f.parameters[0].name.clone(),
        w: f.parameters[1].name.clone(),
        ops: Vec::new(),
        memo: HashMap::new(),
        scopes: vec![HashMap::new()],
    };
    // the body: lets, then one map
    let mut body = f.body.as_ref();
    let mut lets: &[Statement] = &[];
    if let Expr::Block(stmts) = body {
        let (last, init) = stmts
            .split_last()
            .ok_or_else(|| format!("toad: the kernel {} is empty", name))?;
        let mut last = last;
        while let Statement::Located { stmt, .. } = last {
            last = stmt;
        }
        let Statement::Expression(e) = last else {
            return l.err("its body does not end in the map of new fields");
        };
        body = e;
        lets = init;
    }
    if !lets.is_empty() {
        // the lets live in the outermost scope; reuse block() on them alone
        for s in lets {
            let mut s = s;
            while let Statement::Located { stmt, .. } = s {
                s = stmt;
            }
            let Statement::LetDecl(d) = s else {
                return l.err("its body holds something other than `let`s before the map");
            };
            let Pattern::Identifier(n) = &d.pattern else {
                return l.err("a `let` that is not a plain name");
            };
            let Some(v) = &d.value else {
                return l.err(format!("`let {}` has no value", n));
            };
            let v = l.expr(v)?;
            l.scopes[0].insert(n.clone(), v);
        }
    }
    let Expr::MapLiteral { entries } = body else {
        return l.err("it does not answer a map of the fields' new values (#{ \"pos\": … })");
    };
    let mut writes = Vec::new();
    let mut dead = None;
    for e in entries {
        let k = match &e.key {
            Expr::String(s) => s.to_string(),
            _ => return l.err("a field name that is not a literal string"),
        };
        let v = l.expr(&e.value)?;
        if k == "dead" {
            match v {
                V::Bool(i) => dead = Some(i),
                _ => return l.err("`dead` is not a boolean"),
            }
            continue;
        }
        let Some(s) = fields.get(&k).cloned() else {
            return l.err(format!("it sets `{}`, which the swarm does not have", k));
        };
        match (s.ty, v) {
            (Ty::F, V::S(i)) => writes.push((s.at, i)),
            (Ty::V2, V::P(x, y)) => {
                writes.push((s.at, x));
                writes.push((s.at + 1, y));
            }
            (Ty::F, _) => return l.err(format!("`{}` is a number and is set to something else", k)),
            (Ty::V2, _) => return l.err(format!("`{}` is a pair and is set to something else", k)),
        }
    }
    let (ops, writes, dead) = prune(l.ops, writes, dead);
    Ok(Program {
        name,
        fields: fields.clone(),
        uniforms: uniforms.clone(),
        ops,
        writes,
        dead,
        defaults: vec![0.0; fields.width],
    })
}

/// Only the ops the writes and the dead flag need, renumbered in order
/// (a constant folded away, a `let` never used, leave nothing behind).
fn prune(
    ops: Vec<Op>,
    writes: Vec<(usize, usize)>,
    dead: Option<usize>,
) -> (Vec<Op>, Vec<(usize, usize)>, Option<usize>) {
    let mut live = vec![false; ops.len()];
    let mut stack: Vec<usize> = writes.iter().map(|w| w.1).chain(dead).collect();
    while let Some(i) = stack.pop() {
        if live[i] {
            continue;
        }
        live[i] = true;
        match ops[i] {
            Op::Bin(_, a, b) => stack.extend([a, b]),
            Op::Un(_, a) => stack.push(a),
            Op::Sel(c, a, b) => stack.extend([c, a, b]),
            _ => {}
        }
    }
    let mut at = vec![usize::MAX; ops.len()];
    let mut out = Vec::new();
    for (i, op) in ops.into_iter().enumerate() {
        if !live[i] {
            continue;
        }
        let m = |j: usize| at[j];
        let op = match op {
            Op::Bin(b, x, y) => Op::Bin(b, m(x), m(y)),
            Op::Un(u, x) => Op::Un(u, m(x)),
            Op::Sel(c, x, y) => Op::Sel(m(c), m(x), m(y)),
            other => other,
        };
        at[i] = out.len();
        out.push(op);
    }
    let writes = writes.into_iter().map(|(c, j)| (c, at[j])).collect();
    (out, writes, dead.map(|j| at[j]))
}

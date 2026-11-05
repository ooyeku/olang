//! `__peg` — the native matching core of the `peg` package
//! (src/stdlib/embedded/peg.ol). Internal: `peg.ol` is its only caller.
//!
//! `peg.ol` reads a grammar's text, checks it, analyses it (what can
//! match nothing, left recursion, the rules that memoise) and words every
//! message; this module runs it. `__peg.compile` takes the rules as
//! data and answers an opaque grammar handle; `parse`, `accepts` and
//! `recover` match an input with it and answer olang values: the tree
//! (or the start rule's value, the actions applied), or the data of the
//! error, which `peg.ol` turns into the error value it always made.
//!
//! The handle is immutable and shared freely across threads: every parse
//! has its own state. Nothing here blocks or spawns, so the browser
//! playground runs it as it is. Design and measurements:
//! plans/PEG-NATIVE.md.

mod grammar;
mod machine;
mod values;

#[cfg(test)]
mod tests;

use crate::ast::{BuiltinFunction, Value, ValueMap};
use crate::interpreter::{Interpreter, InterpreterError};
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::sync::Arc;

pub(crate) use grammar::Grammar;
use machine::{MAX_DEPTH, Machine};
use values::{Input, Tree, error_data};

/// A compiled grammar.
pub(crate) struct PegGrammar(pub Arc<Grammar>);

impl std::fmt::Debug for PegGrammar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<peg grammar of {} rules>", self.0.rules.len())
    }
}

impl NativeObject for PegGrammar {
    fn module(&self) -> &'static str {
        "__peg"
    }
    fn type_name(&self) -> &'static str {
        "PegGrammar"
    }
    fn display(&self) -> String {
        format!("<peg grammar of {} rules>", self.0.rules.len())
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<PegGrammar>()
            .is_some_and(|o| Arc::ptr_eq(&self.0, &o.0))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

const FUNCTIONS: &[(&str, usize)] = &[
    ("compile", 4),
    ("parse", 3),
    ("accepts", 3),
    ("recover", 4),
    ("listing", 1),
];

pub fn create_module() -> Value {
    let mut module = ValueMap::default();
    for (name, arity) in FUNCTIONS {
        module.insert(
            name.to_string(),
            Value::Builtin(BuiltinFunction {
                name: format!("__peg.{name}"),
                arity: *arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".into(),
        fields: Arc::new(module).into(),
    }
}

fn err(message: impl Into<String>) -> InterpreterError {
    InterpreterError::runtime(message)
}

fn grammar_of(v: &Value) -> Result<Arc<Grammar>, InterpreterError> {
    match v {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<PegGrammar>()
                .map(|g| g.0.clone())
                .ok_or_else(|| err("__peg: expected a grammar handle"))
        }
        _ => Err(err("__peg: expected a grammar handle")),
    }
}

fn text_of(v: &Value, what: &str) -> Result<Arc<String>, InterpreterError> {
    match v {
        Value::String(s) => Ok(s.clone()),
        other => Err(err(format!(
            "peg: {what} must be a String, got {}",
            other.type_name()
        ))),
    }
}

fn start_of(v: &Value, g: &Grammar) -> Result<u32, InterpreterError> {
    match v {
        Value::Integer(i) if *i >= 0 && (*i as usize) < g.rules.len() => Ok(*i as u32),
        _ => Err(err("__peg: the start rule's index is out of range")),
    }
}

fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::List(xs) => xs
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.as_str().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn too_deep() -> InterpreterError {
    err(format!(
        "peg: the input nests too deeply to parse (more than {MAX_DEPTH} rules inside one another)"
    ))
}

/// `__peg.*` with the interpreter, which actions are called on.
pub fn call(
    name: &str,
    args: Vec<Value>,
    interp: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    let want = FUNCTIONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, a)| *a)
        .ok_or_else(|| err(format!("__peg has no function `{name}`")))?;
    if args.len() != want {
        return Err(InterpreterError::ArityMismatch {
            expected: want,
            got: args.len(),
        });
    }
    match name {
        "compile" => compile(&args),
        "listing" => Ok(Value::String(Arc::new(grammar_of(&args[0])?.listing()))),
        "accepts" => {
            let g = grammar_of(&args[0])?;
            let text = text_of(&args[1], "the input")?;
            let start = start_of(&args[2], &g)?;
            let input = Input::new(&text);
            let mut m = Machine::new(&g, &input.cs);
            let ok = m.accepts(start);
            if m.too_deep {
                return Err(too_deep());
            }
            Ok(Value::Boolean(ok))
        }
        "parse" => {
            let g = grammar_of(&args[0])?;
            let text = text_of(&args[1], "the input")?;
            let start = start_of(&args[2], &g)?;
            parse(&g, &text, start, interp)
        }
        "recover" => {
            let g = grammar_of(&args[0])?;
            let text = text_of(&args[1], "the input")?;
            let start = start_of(&args[2], &g)?;
            recover(&g, &text, start, &strings(&args[3]), interp)
        }
        _ => unreachable!("checked against FUNCTIONS"),
    }
}

/// `__peg.compile(rules, memo, heads, actions)`.
fn compile(args: &[Value]) -> Result<Value, InterpreterError> {
    let rules = match &args[0] {
        Value::List(xs) => xs
            .iter()
            .map(grammar::read_rule)
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?,
        _ => return Err(err("__peg.compile: the rules must be a list")),
    };
    let actions: Vec<(String, Value)> = match &args[3] {
        Value::Map(m) => m
            .iter()
            .filter(|(_, v)| !matches!(v, Value::Unit))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        Value::Struct { fields, .. } => {
            fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        }
        _ => Vec::new(),
    };
    let g = grammar::build(rules, &strings(&args[1]), &strings(&args[2]), &actions).map_err(err)?;
    Ok(Value::Native(NativeHandle::new(PegGrammar(Arc::new(g)))))
}

fn parse(
    g: &Grammar,
    text: &str,
    start: u32,
    interp: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    let input = Input::new(text);
    let mut m = Machine::new(g, &input.cs);
    match m.build(start, 0) {
        Some(nd) => {
            let root = m.root_of(nd, start);
            let mut tree = Tree::new(&m, &input, g);
            let v = tree.value(root, interp)?;
            Ok(Value::Ok(Box::new(v)))
        }
        None => {
            if m.too_deep {
                return Err(too_deep());
            }
            let (far, exp) = m.failure(start, 0);
            if m.too_deep {
                return Err(too_deep());
            }
            Ok(Value::Err(Box::new(error_data(
                &input, far, &exp, &g.descs, &m.oks,
            ))))
        }
    }
}

/// Where to go on after an error: past the first sync string at or after
/// it, or -1.
fn sync_after(cs: &[i32], from: i64, syncs: &[Vec<i32>]) -> i64 {
    let n = cs.len() as i64 - 1;
    let mut q = from;
    while q < n {
        for sy in syncs {
            if !sy.is_empty()
                && sy
                    .iter()
                    .enumerate()
                    .all(|(i, c)| cs.get(q as usize + i) == Some(c))
            {
                return q + sy.len() as i64;
            }
        }
        q += 1;
    }
    -1
}

/// Parse, and at each error skip past the next sync point and go on:
/// `Ok(tree)`, or `Err([tree, [error data]])` with every error of one
/// pass and the tree of what did parse.
fn recover(
    g: &Grammar,
    text: &str,
    start: u32,
    syncs: &[String],
    interp: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    let input = Input::new(text);
    let n = input.n();
    let syncs: Vec<Vec<i32>> = syncs
        .iter()
        .map(|s| s.chars().map(|c| c as i32).collect())
        .collect();
    let mut m = Machine::new(g, &input.cs);
    let mut p = 0;
    let mut kids: Vec<u32> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();
    loop {
        m.reset(false);
        let from = m.skip_at(p);
        let nd = m.rule_b(start, from);
        let r = if nd < 0 {
            -1
        } else {
            let e = m.end_of(nd);
            m.skip_at(e)
        };
        if nd >= 0 {
            let node = m.nodes[nd as usize];
            for k in node.kids..node.kids + node.nkids {
                kids.push(m.store[k as usize].node);
            }
        }
        if m.too_deep {
            return Err(too_deep());
        }
        if r == n {
            break;
        }
        let (far, exp) = m.failure(start, p);
        if m.too_deep {
            return Err(too_deep());
        }
        errors.push(error_data(&input, far, &exp, &g.descs, &m.oks));
        let at = sync_after(&input.cs, far, &syncs);
        if at < 0 || at >= n {
            break;
        }
        p = if at <= p { p + 1 } else { at };
    }
    let kind = g.rules[start as usize].kind;
    let root = m.make(kind, 0, n, &kids);
    let mut tree = Tree::new(&m, &input, g);
    let v = tree.value(root, interp)?;
    if errors.is_empty() {
        Ok(Value::Ok(Box::new(v)))
    } else {
        Ok(Value::Err(Box::new(Value::List(Arc::new(vec![
            v,
            Value::List(Arc::new(errors)),
        ])))))
    }
}

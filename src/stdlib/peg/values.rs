//! The input as the matcher reads it, and what a parse answers as olang
//! values: the tree (each node a map — kind, text, span, children, its
//! labels, a table's op and fixity — with the actions applied bottom up),
//! and the data of an error, worded as `peg.ol` words it.

use super::grammar::{Fix, Grammar, NONE, quote};
use super::machine::{Extra, Field, Machine};
use crate::ast::{Value, ValueMap};
use crate::interpreter::{Interpreter, InterpreterError};
use std::sync::Arc;

/// The input: its code points with -1 after the last (positions are
/// code-point offsets, as `str.substring` takes them), and where each
/// code point starts in the text when it is not all ASCII.
pub(crate) struct Input<'s> {
    pub text: &'s str,
    pub cs: Vec<i32>,
    boff: Option<Vec<u32>>,
}

impl<'s> Input<'s> {
    pub fn new(text: &'s str) -> Input<'s> {
        if text.is_ascii() {
            let mut cs = Vec::with_capacity(text.len() + 1);
            cs.extend(text.bytes().map(|b| b as i32));
            cs.push(-1);
            Input {
                text,
                cs,
                boff: None,
            }
        } else {
            let mut cs = Vec::with_capacity(text.len() + 1);
            let mut boff = Vec::with_capacity(text.len() + 1);
            for (i, ch) in text.char_indices() {
                cs.push(ch as i32);
                boff.push(i as u32);
            }
            cs.push(-1);
            boff.push(text.len() as u32);
            Input {
                text,
                cs,
                boff: Some(boff),
            }
        }
    }

    /// The text between two positions.
    #[inline]
    pub fn slice(&self, a: i64, b: i64) -> &'s str {
        if b <= a {
            return "";
        }
        match &self.boff {
            None => &self.text[a as usize..b as usize],
            Some(off) => &self.text[off[a as usize] as usize..off[b as usize] as usize],
        }
    }

    pub fn n(&self) -> i64 {
        self.cs.len() as i64 - 1
    }
}

fn s(v: &str) -> Value {
    Value::String(Arc::new(v.to_string()))
}

fn map_of(entries: Vec<(&str, Value)>) -> Value {
    let mut m = ValueMap::with_capacity_and_hasher(entries.len(), Default::default());
    for (k, v) in entries {
        m.insert(k.to_string(), v);
    }
    Value::Map(Arc::new(m))
}

// ── trees ──────────────────────────────────────────────────────────────

/// Turns the arena's nodes into olang values.
pub(crate) struct Tree<'t, 'a> {
    m: &'t Machine<'a>,
    input: &'t Input<'t>,
    kinds: Vec<Value>,
    actions: &'t [Option<Value>],
    empty: Value,
    /// The line cursor: (position, line, start of the line).
    cur: (i64, i64, i64),
    depth: u32,
    /// The text and span of the node made last, with its bounds: a node
    /// over the same text (a rule that wraps one other, as `value` wraps
    /// `String`) shares them.
    last: Option<(i64, i64, Value, Value)>,
    /// Short texts already made, shared: a document repeats its keywords,
    /// its keys and its small numbers.
    texts: rustc_hash::FxHashMap<&'t str, Value>,
}

impl<'t, 'a> Tree<'t, 'a> {
    pub fn new(m: &'t Machine<'a>, input: &'t Input<'t>, g: &'t Grammar) -> Tree<'t, 'a> {
        Tree {
            m,
            input,
            kinds: g.kinds.iter().map(|k| Value::String(k.clone())).collect(),
            actions: &g.actions,
            empty: Value::List(Arc::new(Vec::new())),
            cur: (0, 1, 0),
            depth: 0,
            last: None,
            texts: Default::default(),
        }
    }

    fn text(&mut self, a: i64, b: i64) -> Value {
        let t = self.input.slice(a, b);
        // (a short input repeats little: the table would cost more)
        if t.len() > 24 || self.input.cs.len() < 2048 {
            return Value::String(Arc::new(t.to_string()));
        }
        self.texts
            .entry(t)
            .or_insert_with(|| Value::String(Arc::new(t.to_string())))
            .clone()
    }

    /// Line and column of `a`, moving the cursor forward: nodes are
    /// visited in order of their start, so a walk reads the input once.
    fn advance(&mut self, a: i64) -> (i64, i64) {
        let (mut q, mut line, mut ls) = self.cur;
        if a < q {
            q = 0;
            line = 1;
            ls = 0;
        }
        let cs = &self.input.cs;
        while q < a {
            if cs[q as usize] == 10 {
                line += 1;
                ls = q + 1;
            }
            q += 1;
        }
        self.cur = (q, line, ls);
        (line, a - ls + 1)
    }

    pub fn value(
        &mut self,
        id: u32,
        interp: &mut Interpreter,
    ) -> Result<Value, InterpreterError> {
        self.depth += 1;
        let out = if self.depth % 64 == 0 {
            crate::interpreter::with_stack_headroom(|| self.value_inner(id, interp))
        } else {
            self.value_inner(id, interp)
        };
        self.depth -= 1;
        out
    }

    fn value_inner(
        &mut self,
        id: u32,
        interp: &mut Interpreter,
    ) -> Result<Value, InterpreterError> {
        let nd = self.m.nodes[id as usize];
        let (line, column) = self.advance(nd.start);
        let kids_value = if nd.nkids == 0 {
            self.empty.clone()
        } else {
            let mut kids: Vec<Value> = Vec::with_capacity(nd.nkids as usize);
            for k in nd.kids..nd.kids + nd.nkids {
                let c = self.m.store[k as usize].node;
                kids.push(self.value(c, interp)?);
            }
            Value::List(Arc::new(kids))
        };
        let kids: &[Value] = match &kids_value {
            Value::List(xs) => xs.as_slice(),
            _ => &[],
        };
        let nfields = match nd.extra {
            Extra::None => 0,
            Extra::Fields(f) => self.m.fields[f as usize].len(),
            Extra::Prec { fix, .. } => {
                if fix == Fix::Prefix || fix == Fix::Postfix {
                    3
                } else {
                    4
                }
            }
        };
        let shared = match &self.last {
            Some((a, b, t, sp)) if *a == nd.start && *b == nd.end => Some((t.clone(), sp.clone())),
            _ => None,
        };
        let (text, span) = match shared {
            Some(both) => both,
            None => {
                let mut span = ValueMap::with_capacity_and_hasher(4, Default::default());
                span.insert("line".to_string(), Value::Integer(line));
                span.insert("column".to_string(), Value::Integer(column));
                span.insert("start".to_string(), Value::Integer(nd.start));
                span.insert("end".to_string(), Value::Integer(nd.end));
                let span = Value::Map(Arc::new(span));
                let text = self.text(nd.start, nd.end);
                self.last = Some((nd.start, nd.end, text.clone(), span.clone()));
                (text, span)
            }
        };
        let mut m = ValueMap::with_capacity_and_hasher(4 + nfields, Default::default());
        let kind = if nd.kind == NONE {
            Value::Unit
        } else {
            self.kinds[nd.kind as usize].clone()
        };
        m.insert("kind".to_string(), kind);
        m.insert("text".to_string(), text);
        m.insert("span".to_string(), span);
        m.insert("children".to_string(), kids_value.clone());
        match nd.extra {
            Extra::None => {}
            Extra::Fields(f) => {
                for (k, fv) in &self.m.fields[f as usize] {
                    let v = match fv {
                        Field::One(i) => kids[*i as usize].clone(),
                        Field::Many(is) => Value::List(Arc::new(
                            is.iter().map(|i| kids[*i as usize].clone()).collect(),
                        )),
                    };
                    m.insert(self.m.g.kinds[*k as usize].as_str().to_string(), v);
                }
            }
            Extra::Prec {
                op_start,
                op_end,
                fix,
            } => {
                m.insert("op".to_string(), s(self.input.slice(op_start, op_end)));
                m.insert("fixity".to_string(), s(fix.word()));
                if fix == Fix::Prefix || fix == Fix::Postfix {
                    m.insert("operand".to_string(), kids[0].clone());
                } else {
                    m.insert("left".to_string(), kids[0].clone());
                    m.insert("right".to_string(), kids[1].clone());
                }
            }
        }
        let value = Value::Map(Arc::new(m));
        if nd.kind != NONE
            && let Some(Some(act)) = self.actions.get(nd.kind as usize)
        {
            return interp.call_function(act.clone(), vec![value]);
        }
        Ok(value)
    }
}

// ── errors ─────────────────────────────────────────────────────────────

fn is_id_char(c: i32) -> bool {
    (97..=122).contains(&c) || (65..=90).contains(&c) || c == 95 || (48..=57).contains(&c)
}

fn is_word_code(c: i32) -> bool {
    is_id_char(c) || c >= 128
}

/// (line, column, start of the line) of a position, 1-based.
fn line_col(cs: &[i32], pos: i64) -> (i64, i64, i64) {
    let mut line = 1;
    let mut ls = 0;
    let mut i = 0;
    while i < pos && cs[i as usize] >= 0 {
        if cs[i as usize] == 10 {
            line += 1;
            ls = i + 1;
        }
        i += 1;
    }
    (line, pos - ls + 1, ls)
}

/// The data of a parse error at `far`, as `peg.ol` words it: the map an
/// error value has but for `show`, whose text is under "shown".
pub(crate) fn error_data(
    input: &Input,
    far: i64,
    expected_ix: &[u32],
    descs: &[String],
    oks: &[i64],
) -> Value {
    let cs = &input.cs;
    let mut expected: Vec<&str> = Vec::new();
    for &i in expected_ix {
        let d = descs[i as usize].as_str();
        if !expected.contains(&d) {
            expected.push(d);
        }
    }
    // what stands at the position: a word, or one character
    let found: Option<&str> = if cs[far as usize] < 0 {
        None
    } else if is_word_code(cs[far as usize]) {
        let mut q = far;
        while is_word_code(cs[q as usize]) && q - far < 24 {
            q += 1;
        }
        Some(input.slice(far, q))
    } else {
        Some(input.slice(far, far + 1))
    };
    // the last piece matched before it
    let after: Option<&str> = {
        let mut b = far;
        while b > 0 && matches!(cs[(b - 1) as usize], 32 | 9 | 10 | 13) {
            b -= 1;
        }
        if b <= 0 || b as usize >= oks.len() || oks[b as usize] < 0 {
            None
        } else {
            let a = oks[b as usize];
            if b - a > 24 { None } else { Some(input.slice(a, b)) }
        }
    };
    let saw = match found {
        None => "end of input".to_string(),
        Some(f) => quote(f),
    };
    let message = if expected.is_empty() {
        format!("unexpected {saw}")
    } else {
        let list = if expected.len() == 1 {
            expected[0].to_string()
        } else {
            format!(
                "{} or {}",
                expected[..expected.len() - 1].join(", "),
                expected[expected.len() - 1]
            )
        };
        let after_part = match after {
            None => String::new(),
            Some(a) => format!(" after {}", quote(a)),
        };
        format!("expected {list}{after_part}, found {saw}")
    };
    let (line, column, ls) = line_col(cs, far);
    let width = found.map(|f| f.chars().count() as i64).unwrap_or(0);
    let full = format!("{message} ({line}:{column})");
    // the line, and a caret under the position (tabs stay tabs)
    let mut e = ls;
    while cs[e as usize] >= 0 && cs[e as usize] != 10 {
        e += 1;
    }
    let line_text = if e > ls && cs[(e - 1) as usize] == 13 {
        input.slice(ls, e - 1)
    } else {
        input.slice(ls, e)
    };
    let num = line.to_string();
    let mut pad = String::new();
    for i in ls..far {
        pad.push(if cs[i as usize] == 9 { '\t' } else { ' ' });
    }
    let shown = format!(
        "{full}\n {num} | {line_text}\n {} | {pad}^",
        " ".repeat(num.len())
    );
    map_of(vec![
        ("message", s(&full)),
        (
            "span",
            map_of(vec![
                ("line", Value::Integer(line)),
                ("column", Value::Integer(column)),
                ("start", Value::Integer(far)),
                ("end", Value::Integer(far + width)),
            ]),
        ),
        (
            "expected",
            Value::List(Arc::new(expected.iter().map(|d| s(d)).collect())),
        ),
        ("found", found.map(s).unwrap_or(Value::Unit)),
        ("after", after.map(s).unwrap_or(Value::Unit)),
        ("shown", s(&shown)),
    ])
}

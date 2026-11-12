//! The linearizability checker (S4): is a history of concurrent
//! operations explained by some order of them, one at a time, that a
//! sequential model accepts and that respects real time (an operation
//! that completed before another was invoked comes first)?
//!
//! **The history** is what `sim.record` kept, in order: entries of
//! `type` `invoke`, `ok`, `fail` or `info`, each with a `process` (the
//! recording task, unless given), an `f` (the operation) and a `value`.
//! An `ok` completes its process's open invocation and carries the
//! operation's output in `value` (the invocation's value when it has
//! none); a `fail` says the operation did not happen (it is dropped);
//! an `info`, or an invocation never completed, is *indeterminate*: it
//! may have happened at any point after its invocation, or never, and
//! its output is unknown. A process invoking again while its last
//! operation is open leaves that one indeterminate.
//!
//! **The search** is Wing and Gong's, with Lowe's memoisation (as in
//! Knossos and Porcupine): walk the history's calls and returns, try to
//! linearize each call whose operation the model accepts in the current
//! state, backtrack when a return comes before its operation could be
//! placed, and never revisit a (linearized set, model state) pair seen
//! before. A key-value history is split by key first (linearizability is
//! local: the whole is linearizable when each key's history is), so each
//! search is small. The search is bounded: past `max_steps` it answers
//! *unknown* rather than run on.
//!
//! **Models** are a register (`write`, `read`, `cas`), a key-value map
//! (`put`, `get`, `add`, `cas` on a key), or an olang value: an initial
//! state and a step function `(state, op) => Ok(next) | Err(why)`.

use crate::ast::{Value, ValueMap};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// One operation of a history.
#[derive(Debug, Clone)]
pub struct Op {
    pub process: String,
    pub f: String,
    pub value: Value,
    /// What it answered; `None` when unknown (indeterminate).
    pub output: Option<Value>,
    /// Its invocation's place in the history.
    pub call: usize,
    /// Its completion's place; `None` when it never completed (it may
    /// have happened, or not).
    pub ret: Option<usize>,
}

/// What the checker found.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Linearizable,
    /// No order works: why, naming the operation that could not be placed.
    Violation(String),
    /// The search ran past its bound.
    Unknown(String),
}

/// A sequential model: what an operation does in a state.
pub trait Model {
    fn init(&self) -> Value;
    /// `Ok(Some(next))` when `op` may happen in `state` (with its output,
    /// when known), `Ok(None)` when it may not, `Err` when the model
    /// itself fails.
    fn step(&mut self, state: &Value, op: &Op) -> Result<Option<Value>, String>;
    /// For a model of independent keys: the key `op` acts on.
    fn key_of(&self, _op: &Op) -> Option<String> {
        None
    }
}

fn s(t: &str) -> Value {
    Value::String(Arc::new(t.to_string()))
}

fn items(v: &Value) -> Option<&[Value]> {
    match v {
        Value::List(x) | Value::Tuple(x) => Some(x.as_slice()),
        _ => None,
    }
}

fn field<'a>(m: &'a ValueMap, k: &str) -> Option<&'a Value> {
    m.get(k)
}

/// A single register: `write v`, `read → v`, `cas [old, new]` (an output
/// of `false` says it did not swap).
pub struct Register {
    pub init: Value,
}

impl Model for Register {
    fn init(&self) -> Value {
        self.init.clone()
    }
    fn step(&mut self, state: &Value, op: &Op) -> Result<Option<Value>, String> {
        Ok(match op.f.as_str() {
            "write" => Some(op.value.clone()),
            "read" => match &op.output {
                Some(v) if v != state => None,
                _ => Some(state.clone()),
            },
            "cas" => {
                let pair = items(&op.value)
                    .filter(|p| p.len() == 2)
                    .ok_or("register: cas takes [old, new]")?;
                let swapped = state == &pair[0];
                match &op.output {
                    Some(Value::Boolean(false)) => (!swapped).then(|| state.clone()),
                    _ => swapped.then(|| pair[1].clone()),
                }
            }
            other => {
                return Err(format!(
                    "register: no operation `{other}` (write, read, cas)"
                ));
            }
        })
    }
}

/// A map of independent registers: `put [k, v]`, `get k → v` (`()` when
/// absent), `add [k, by] → total`, `cas [k, old, new]`. Its state, per
/// key, is that key's value.
pub struct Kv;

impl Kv {
    fn key_and_rest(op: &Op) -> Result<(String, Vec<Value>), String> {
        match items(&op.value) {
            Some(parts) if !parts.is_empty() => Ok((key_text(&parts[0]), parts[1..].to_vec())),
            _ => Ok((key_text(&op.value), Vec::new())),
        }
    }
}

fn key_text(v: &Value) -> String {
    match v {
        Value::String(t) => t.to_string(),
        other => other.to_string(),
    }
}

impl Model for Kv {
    fn init(&self) -> Value {
        Value::Unit
    }
    fn key_of(&self, op: &Op) -> Option<String> {
        Kv::key_and_rest(op).ok().map(|(k, _)| k)
    }
    /// The state is the one key's value (the history is split by key).
    fn step(&mut self, state: &Value, op: &Op) -> Result<Option<Value>, String> {
        let (_, rest) = Kv::key_and_rest(op)?;
        Ok(match op.f.as_str() {
            "put" | "write" => Some(rest.first().cloned().ok_or("kv: put takes [key, value]")?),
            "get" | "read" => match &op.output {
                Some(v) if v != state => None,
                _ => Some(state.clone()),
            },
            "add" => {
                let by = rest.first().cloned().unwrap_or(Value::Integer(1));
                let next = match (state, &by) {
                    (Value::Unit, b) => b.clone(),
                    (Value::Integer(a), Value::Integer(b)) => Value::Integer(a + b),
                    (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
                    (Value::Integer(a), Value::Float(b)) => Value::Float(*a as f64 + b),
                    (Value::Float(a), Value::Integer(b)) => Value::Float(a + *b as f64),
                    _ => return Err("kv: add needs numbers".into()),
                };
                match &op.output {
                    Some(out @ (Value::Integer(_) | Value::Float(_))) if out != &next => None,
                    _ => Some(next),
                }
            }
            "cas" => {
                if rest.len() != 2 {
                    return Err("kv: cas takes [key, old, new]".into());
                }
                let swapped = state == &rest[0];
                match &op.output {
                    Some(Value::Boolean(false)) => (!swapped).then(|| state.clone()),
                    _ => swapped.then(|| rest[1].clone()),
                }
            }
            other => return Err(format!("kv: no operation `{other}` (put, get, add, cas)")),
        })
    }
}

/// The operations of a recorded history (`sim.record`'s entries).
pub fn parse(history: &[Value]) -> Result<Vec<Op>, String> {
    let mut ops: Vec<Op> = Vec::new();
    // process → the index in `ops` of its open invocation
    let mut open: HashMap<String, usize> = HashMap::new();
    let mut failed: HashSet<usize> = HashSet::new();
    for (i, e) in history.iter().enumerate() {
        // an entry is a map or a record (OL-600)
        let Some(m) = e.record_fields() else {
            return Err(format!("history entry {i} is not a map or a record"));
        };
        let kind = match field(m, "type") {
            Some(Value::String(t)) => t.to_string(),
            _ => {
                return Err(format!(
                    "history entry {i} has no `type` (invoke, ok, fail, info)"
                ));
            }
        };
        let process = match field(m, "process") {
            Some(Value::String(p)) => p.to_string(),
            Some(Value::Unit) | None => "?".to_string(),
            Some(other) => other.to_string(),
        };
        let f = match field(m, "f") {
            Some(Value::String(t)) => Some(t.to_string()),
            _ => None,
        };
        let value = field(m, "value").cloned();
        match kind.as_str() {
            "invoke" => {
                if let Some(prev) = open.remove(&process) {
                    // invoked again: the last is left indeterminate
                    ops[prev].ret = None;
                    ops[prev].output = None;
                }
                let Some(f) = f else {
                    return Err(format!(
                        "history entry {i}: an invoke names its operation (`f`)"
                    ));
                };
                ops.push(Op {
                    process: process.clone(),
                    f,
                    value: value.unwrap_or(Value::Unit),
                    output: None,
                    call: i,
                    ret: None,
                });
                open.insert(process, ops.len() - 1);
            }
            "ok" => {
                let Some(at) = open.remove(&process) else {
                    return Err(format!(
                        "history entry {i}: `ok` from process {process} with no open invocation"
                    ));
                };
                ops[at].ret = Some(i);
                ops[at].output = Some(value.unwrap_or_else(|| ops[at].value.clone()));
            }
            "fail" => {
                if let Some(at) = open.remove(&process) {
                    failed.insert(at);
                }
            }
            "info" => {
                open.remove(&process);
            }
            other => {
                return Err(format!(
                    "history entry {i}: unknown type `{other}` (invoke, ok, fail, info)"
                ));
            }
        }
    }
    Ok(ops
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !failed.contains(i))
        .map(|(_, o)| o)
        .collect())
}

/// The canonical text of a state, for the memo.
fn state_key(v: &Value) -> (u64, u64) {
    let text = format!("{v:?}");
    let mut a = super::Fnv::new();
    a.str(&text);
    let mut b = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut b);
    (a.0, b.finish())
}

fn bits_key(bits: &[u64]) -> u64 {
    let mut h = super::Fnv::new();
    for w in bits {
        h.u64(*w);
    }
    h.0
}

fn render_op(op: &Op) -> String {
    let out = match &op.output {
        Some(v) => format!(" → {v}"),
        None => " → ?".to_string(),
    };
    format!("{} {}({}){out}", op.process, op.f, op.value)
}

/// Check `ops` against `model`, spending at most `budget` steps
/// (decremented). Ops are checked as given (one key's, or the whole).
fn wgl(ops: &[Op], model: &mut dyn Model, budget: &mut u64) -> Result<Verdict, String> {
    if ops.is_empty() {
        return Ok(Verdict::Linearizable);
    }
    // Entries: a call and a return per op; a never-completed op returns
    // after everything ("at infinity").
    struct Entry {
        op: usize,
        call: bool,
        at: (usize, usize),
    }
    let mut entries: Vec<Entry> = Vec::with_capacity(ops.len() * 2);
    for (i, op) in ops.iter().enumerate() {
        entries.push(Entry {
            op: i,
            call: true,
            at: (op.call, 0),
        });
        entries.push(Entry {
            op: i,
            call: false,
            at: (op.ret.unwrap_or(usize::MAX), i),
        });
    }
    entries.sort_by_key(|e| e.at);
    let n = entries.len();
    // a doubly linked list over the entries, `n` its head and `end` past
    // its last
    let end = usize::MAX;
    let mut nxt: Vec<usize> = (0..=n)
        .map(|i| if i + 1 < n { i + 1 } else { end })
        .collect();
    nxt[n] = 0;
    let mut prv: Vec<usize> = (0..n).map(|i| if i == 0 { n } else { i - 1 }).collect();
    prv.push(end);
    // the return entry of each op's call entry
    let mut partner = vec![0usize; n];
    {
        let mut call_of = vec![usize::MAX; ops.len()];
        for (i, e) in entries.iter().enumerate() {
            if e.call {
                call_of[e.op] = i;
            }
        }
        for (i, e) in entries.iter().enumerate() {
            if !e.call {
                partner[call_of[e.op]] = i;
                partner[i] = call_of[e.op];
            }
        }
    }
    let lift = |nxt: &mut Vec<usize>, prv: &mut Vec<usize>, i: usize| {
        for j in [i, partner[i]] {
            let (p, q) = (prv[j], nxt[j]);
            nxt[p] = q;
            if q != end {
                prv[q] = p;
            }
        }
    };
    let unlift = |nxt: &mut Vec<usize>, prv: &mut Vec<usize>, i: usize| {
        for j in [partner[i], i] {
            let (p, q) = (prv[j], nxt[j]);
            nxt[p] = j;
            if q != end {
                prv[q] = j;
            }
        }
    };
    let words = ops.len().div_ceil(64);
    let mut bits = vec![0u64; words];
    let mut state = model.init();
    let mut cache: HashSet<(u64, (u64, u64))> = HashSet::new();
    let mut stack: Vec<(usize, Value)> = Vec::new();
    let mut deepest = (0usize, None::<usize>);
    let mut entry = nxt[n];
    while nxt[n] != end {
        if *budget == 0 {
            return Ok(Verdict::Unknown(format!(
                "the search ran past its bound with {} of {} operations placed",
                stack.len(),
                ops.len()
            )));
        }
        *budget -= 1;
        if entry == end {
            // only calls that cannot be placed are left, behind every
            // return: every completed op was placed
            return Ok(Verdict::Linearizable);
        }
        let e = &entries[entry];
        if e.call {
            let op = &ops[e.op];
            match model.step(&state, op)? {
                Some(next_state) => {
                    bits[e.op / 64] |= 1 << (e.op % 64);
                    let key = (bits_key(&bits), state_key(&next_state));
                    if cache.insert(key) {
                        stack.push((entry, std::mem::replace(&mut state, next_state)));
                        lift(&mut nxt, &mut prv, entry);
                        entry = nxt[n];
                        continue;
                    }
                    bits[e.op / 64] &= !(1 << (e.op % 64));
                    entry = nxt[entry];
                }
                None => entry = nxt[entry],
            }
        } else {
            if ops[e.op].ret.is_none() {
                // returns at infinity: everything that completed is placed
                return Ok(Verdict::Linearizable);
            }
            if stack.len() >= deepest.0 {
                deepest = (stack.len(), Some(e.op));
            }
            let Some((call_entry, before)) = stack.pop() else {
                let stuck = deepest.1.map(|i| render_op(&ops[i])).unwrap_or_default();
                return Ok(Verdict::Violation(format!(
                    "no order of the {} operations explains the history: after {} placed, {} \
                     completed before it could take effect",
                    ops.len(),
                    deepest.0,
                    stuck
                )));
            };
            let op = entries[call_entry].op;
            bits[op / 64] &= !(1 << (op % 64));
            state = before;
            unlift(&mut nxt, &mut prv, call_entry);
            entry = nxt[call_entry];
        }
    }
    Ok(Verdict::Linearizable)
}

/// Is `history` (recorded entries) linearizable for `model`? At most
/// `max_steps` search steps in all.
pub fn check(history: &[Value], model: &mut dyn Model, max_steps: u64) -> Result<Verdict, String> {
    let ops = parse(history)?;
    let mut budget = max_steps;
    let first = ops.first().map(|o| model.key_of(o));
    if matches!(first, Some(Some(_))) {
        // independent keys: one search per key
        let mut by_key: BTreeMap<String, Vec<Op>> = BTreeMap::new();
        for op in ops {
            let k = model.key_of(&op).unwrap_or_default();
            by_key.entry(k).or_default().push(op);
        }
        let mut unknown = None;
        for (k, ops) in by_key {
            match wgl(&ops, model, &mut budget)? {
                Verdict::Linearizable => {}
                Verdict::Violation(why) => {
                    return Ok(Verdict::Violation(format!("key {k}: {why}")));
                }
                Verdict::Unknown(why) => unknown = Some(format!("key {k}: {why}")),
            }
        }
        return Ok(match unknown {
            Some(why) => Verdict::Unknown(why),
            None => Verdict::Linearizable,
        });
    }
    wgl(&ops, model, &mut budget)
}

/// An op as an olang model's step function sees it.
pub fn op_value(op: &Op) -> Value {
    let mut m = ValueMap::default();
    m.insert("process".into(), s(&op.process));
    m.insert("f".into(), s(&op.f));
    m.insert("value".into(), op.value.clone());
    m.insert("output".into(), op.output.clone().unwrap_or(Value::Unit));
    m.insert("known".into(), Value::Boolean(op.output.is_some()));
    Value::Map(Arc::new(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(kind: &str, process: &str, f: &str, value: Value) -> Value {
        let mut m = ValueMap::default();
        m.insert("type".into(), s(kind));
        m.insert("process".into(), s(process));
        if !f.is_empty() {
            m.insert("f".into(), s(f));
        }
        if value != Value::Unit {
            m.insert("value".into(), value);
        }
        Value::Map(Arc::new(m))
    }
    fn i(n: i64) -> Value {
        Value::Integer(n)
    }
    fn reg(h: &[Value]) -> Verdict {
        check(h, &mut Register { init: i(0) }, 100_000).unwrap()
    }

    #[test]
    fn a_sequential_register_history_is_linearizable_and_a_stale_read_is_not() {
        let ok = [
            e("invoke", "a", "write", i(1)),
            e("ok", "a", "", Value::Unit),
            e("invoke", "b", "read", Value::Unit),
            e("ok", "b", "", i(1)),
        ];
        assert_eq!(reg(&ok), Verdict::Linearizable);
        let stale = [
            e("invoke", "a", "write", i(1)),
            e("ok", "a", "", Value::Unit),
            e("invoke", "b", "read", Value::Unit),
            e("ok", "b", "", i(0)),
        ];
        assert!(
            matches!(reg(&stale), Verdict::Violation(_)),
            "{:?}",
            reg(&stale)
        );
    }

    #[test]
    fn concurrent_operations_may_take_effect_in_either_order() {
        // b's read overlaps a's write: 0 or 1 are both fine
        for seen in [0, 1] {
            let h = [
                e("invoke", "a", "write", i(1)),
                e("invoke", "b", "read", Value::Unit),
                e("ok", "b", "", i(seen)),
                e("ok", "a", "", Value::Unit),
            ];
            assert_eq!(reg(&h), Verdict::Linearizable, "read {seen}");
        }
        // two reads overlapping one write cannot see 1 then 0 if the
        // first read finished before the second began
        let h = [
            e("invoke", "w", "write", i(1)),
            e("invoke", "a", "read", Value::Unit),
            e("ok", "a", "", i(1)),
            e("invoke", "b", "read", Value::Unit),
            e("ok", "b", "", i(0)),
            e("ok", "w", "", Value::Unit),
        ];
        assert!(matches!(reg(&h), Verdict::Violation(_)));
    }

    #[test]
    fn an_indeterminate_write_may_or_may_not_have_happened() {
        for seen in [0, 2] {
            let h = [
                e("invoke", "a", "write", i(2)),
                e("info", "a", "", Value::Unit),
                e("invoke", "b", "read", Value::Unit),
                e("ok", "b", "", i(seen)),
            ];
            assert_eq!(reg(&h), Verdict::Linearizable, "read {seen}");
        }
        // but it cannot be read and then un-happen
        let h = [
            e("invoke", "a", "write", i(2)),
            e("info", "a", "", Value::Unit),
            e("invoke", "b", "read", Value::Unit),
            e("ok", "b", "", i(2)),
            e("invoke", "b", "read", Value::Unit),
            e("ok", "b", "", i(0)),
        ];
        assert!(matches!(reg(&h), Verdict::Violation(_)));
        // and a failed write never happened
        let h = [
            e("invoke", "a", "write", i(2)),
            e("fail", "a", "", Value::Unit),
            e("invoke", "b", "read", Value::Unit),
            e("ok", "b", "", i(2)),
        ];
        assert!(matches!(reg(&h), Verdict::Violation(_)));
    }

    #[test]
    fn compare_and_set_swaps_once() {
        let pair = |a, b| Value::List(Arc::new(vec![i(a), i(b)]));
        // two concurrent cas(0 → x): only one may succeed
        let both = [
            e("invoke", "a", "cas", pair(0, 1)),
            e("invoke", "b", "cas", pair(0, 2)),
            e("ok", "a", "", Value::Unit),
            e("ok", "b", "", Value::Unit),
        ];
        assert!(matches!(reg(&both), Verdict::Violation(_)));
        let one = [
            e("invoke", "a", "cas", pair(0, 1)),
            e("invoke", "b", "cas", pair(0, 2)),
            e("ok", "a", "", Value::Unit),
            e("ok", "b", "", Value::Boolean(false)),
            e("invoke", "c", "read", Value::Unit),
            e("ok", "c", "", i(1)),
        ];
        assert_eq!(reg(&one), Verdict::Linearizable);
    }

    #[test]
    fn a_kv_history_is_checked_key_by_key_and_a_double_add_is_caught() {
        let kv = |k: &str, n| Value::List(Arc::new(vec![s(k), i(n)]));
        let mut h = Vec::new();
        for (p, k) in [("c1", "x"), ("c2", "y")] {
            h.push(e("invoke", p, "add", kv(k, 1)));
            h.push(e("ok", p, "", i(1)));
            h.push(e("invoke", p, "add", kv(k, 1)));
            h.push(e("ok", p, "", i(2)));
        }
        assert_eq!(check(&h, &mut Kv, 10_000).unwrap(), Verdict::Linearizable);
        // an add that counted twice: the client saw 3 after 1
        let mut bad = h.clone();
        bad.push(e("invoke", "c1", "add", kv("x", 1)));
        bad.push(e("ok", "c1", "", i(4)));
        match check(&bad, &mut Kv, 10_000).unwrap() {
            Verdict::Violation(why) => assert!(why.starts_with("key x"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn many_concurrent_writers_stay_tractable_and_the_bound_says_unknown() {
        // 10 processes, each writing its own number concurrently, then a
        // read of any of them: linearizable; memoisation keeps it small
        let mut h = Vec::new();
        for p in 0..10 {
            h.push(e("invoke", &format!("p{p}"), "write", i(p)));
        }
        for p in 0..10 {
            h.push(e("ok", &format!("p{p}"), "", Value::Unit));
        }
        h.push(e("invoke", "r", "read", Value::Unit));
        h.push(e("ok", "r", "", i(7)));
        assert_eq!(reg(&h), Verdict::Linearizable);
        h.push(e("invoke", "r", "read", Value::Unit));
        h.push(e("ok", "r", "", i(99)));
        assert!(matches!(reg(&h), Verdict::Violation(_)));
        assert!(matches!(
            check(&h, &mut Register { init: i(0) }, 50).unwrap(),
            Verdict::Unknown(_)
        ));
    }
}

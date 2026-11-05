//! `pmap`, `pvec`, `psorted` — persistent collections (OL-424).
//!
//! A Map or a List is copied whole the first time it changes while an
//! older version is still held: an editor keeping its undo history, a
//! store keeping its last snapshot, paid O(n) for every update. These
//! three share structure instead (`im`'s HAMT, RRB vector and B-tree), so
//! an update with older versions kept is O(log n), and every version stays
//! a value: nothing a program holds ever changes.
//!
//! - `pmap`: a hash map, keys Ints or Strings.
//! - `pvec`: a vector, O(log n) get, set, push, pop, slice and concat.
//! - `psorted`: a map kept in key order (Ints before Strings), with range
//!   scans, first and last.
//!
//! They behave like the Map and List they stand for: `==` compares
//! contents (the same object first, in O(1)), and a pmap or a psorted
//! equals a Map holding the same entries, a pvec a List holding the same
//! items; `hash`, `json` and `value.encode` follow. `show` writes
//! `pmap#{…}`, `pvec[…]` and `psorted#{…}`. They are native values, so
//! crossing between the tiers is a refcount, never a conversion.

use crate::ast::{Value, ValueMap};
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::sync::Arc;

/// A key: an Int or a String. Ints order before Strings.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PKey {
    Int(i64),
    Str(Arc<str>),
}

impl PKey {
    fn of(v: &Value, who: &str) -> Result<PKey, String> {
        match v {
            Value::Integer(n) => Ok(PKey::Int(*n)),
            Value::String(s) => Ok(PKey::Str(Arc::from(s.as_str()))),
            other => Err(format!(
                "{who}: a key is an Int or a String, got {}",
                other.type_name()
            )),
        }
    }

    pub fn value(&self) -> Value {
        match self {
            PKey::Int(n) => Value::Integer(*n),
            PKey::Str(s) => Value::String(Arc::new(s.to_string())),
        }
    }

    /// The key as a Map's key: a String as itself, an Int as its text.
    pub fn text(&self) -> String {
        match self {
            PKey::Int(n) => n.to_string(),
            PKey::Str(s) => s.to_string(),
        }
    }
}

type Hasher = std::hash::BuildHasherDefault<rustc_hash::FxHasher>;
pub type PHashMap = im::HashMap<PKey, Value, Hasher>;
pub type PVector = im::Vector<Value>;
pub type POrdMap = im::OrdMap<PKey, Value>;

#[derive(Clone)]
pub enum Persistent {
    Map(PHashMap),
    Vec(PVector),
    Sorted(POrdMap),
}

impl std::fmt::Debug for Persistent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.show())
    }
}

/// A value as it is written inside a collection (a String quoted).
fn nested(v: &Value) -> String {
    let s = Value::List(Arc::new(vec![v.clone()])).to_string();
    s[1..s.len() - 1].to_string()
}

fn key_text(k: &PKey) -> String {
    match k {
        PKey::Int(n) => n.to_string(),
        PKey::Str(s) => nested(&Value::String(Arc::new(s.to_string()))),
    }
}

impl Persistent {
    fn kind(&self) -> &'static str {
        match self {
            Persistent::Map(_) => "PMap",
            Persistent::Vec(_) => "PVec",
            Persistent::Sorted(_) => "PSorted",
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Persistent::Map(m) => m.len(),
            Persistent::Vec(v) => v.len(),
            Persistent::Sorted(m) => m.len(),
        }
    }

    /// The entries in key order (a pmap's too, so every listing is
    /// deterministic).
    fn sorted_entries(&self) -> Vec<(PKey, Value)> {
        match self {
            Persistent::Map(m) => {
                let mut e: Vec<(PKey, Value)> =
                    m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                e.sort_by(|a, b| a.0.cmp(&b.0));
                e
            }
            Persistent::Sorted(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            Persistent::Vec(_) => Vec::new(),
        }
    }

    fn show(&self) -> String {
        match self {
            Persistent::Vec(v) => format!(
                "pvec[{}]",
                v.iter().map(nested).collect::<Vec<_>>().join(", ")
            ),
            _ => format!(
                "{}#{{{}}}",
                if matches!(self, Persistent::Map(_)) {
                    "pmap"
                } else {
                    "psorted"
                },
                self.sorted_entries()
                    .iter()
                    .map(|(k, v)| format!("{}: {}", key_text(k), nested(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    fn get(&self, k: &PKey) -> Option<&Value> {
        match self {
            Persistent::Map(m) => m.get(k),
            Persistent::Sorted(m) => m.get(k),
            Persistent::Vec(_) => None,
        }
    }

    /// As a plain Map (Int keys as their text), or a List.
    pub fn to_plain(&self) -> Value {
        match self {
            Persistent::Vec(v) => Value::List(Arc::new(v.iter().cloned().collect())),
            _ => Value::Map(Arc::new(
                self.sorted_entries()
                    .into_iter()
                    .map(|(k, v)| (k.text(), v))
                    .collect(),
            )),
        }
    }

    fn same_object(&self, other: &Persistent) -> bool {
        match (self, other) {
            (Persistent::Map(a), Persistent::Map(b)) => a.ptr_eq(b),
            (Persistent::Vec(a), Persistent::Vec(b)) => a.ptr_eq(b),
            (Persistent::Sorted(a), Persistent::Sorted(b)) => a.ptr_eq(b),
            _ => false,
        }
    }

    /// `==` against another persistent collection.
    fn equals(&self, other: &Persistent) -> bool {
        if self.same_object(other) {
            return true;
        }
        if self.len() != other.len() {
            return false;
        }
        match (self, other) {
            (Persistent::Vec(a), Persistent::Vec(b)) => a
                .iter()
                .zip(b.iter())
                .all(|(x, y)| crate::interpreter::ops::loose_eq(x, y)),
            (Persistent::Vec(_), _) | (_, Persistent::Vec(_)) => false,
            (Persistent::Sorted(a), Persistent::Sorted(b)) => a
                .iter()
                .zip(b.iter())
                .all(|((k, x), (l, y))| k == l && crate::interpreter::ops::loose_eq(x, y)),
            (a, b) => a.sorted_entries().iter().all(|(k, x)| {
                b.get(k)
                    .is_some_and(|y| crate::interpreter::ops::loose_eq(x, y))
            }),
        }
    }

    /// `==` against a plain Map or List: a pmap or a psorted equals a Map
    /// holding the same entries (String keys), a pvec a List holding the
    /// same items.
    pub fn equals_plain(&self, other: &Value) -> Option<bool> {
        match (self, other) {
            (Persistent::Vec(v), Value::List(items)) => Some(
                v.len() == items.len()
                    && v.iter()
                        .zip(items.iter())
                        .all(|(x, y)| crate::interpreter::ops::loose_eq(x, y)),
            ),
            (Persistent::Vec(_), _) => None,
            (_, Value::Map(m)) => Some(self.equals_map(m)),
            (_, Value::Struct { type_name, fields })
                if type_name == "Object" || type_name == "JsonObject" =>
            {
                Some(self.equals_map(fields))
            }
            _ => None,
        }
    }

    fn equals_map(&self, m: &ValueMap) -> bool {
        self.len() == m.len()
            && m.iter().all(|(k, v)| {
                self.get(&PKey::Str(Arc::from(k.as_str())))
                    .is_some_and(|x| crate::interpreter::ops::loose_eq(x, v))
            })
    }
}

#[derive(Debug)]
pub struct PersistentObject(pub Persistent);

impl NativeObject for PersistentObject {
    fn module(&self) -> &'static str {
        "persistent"
    }
    fn type_name(&self) -> &'static str {
        self.0.kind()
    }
    fn display(&self) -> String {
        self.0.show()
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<PersistentObject>()
            .is_some_and(|o| self.0.equals(&o.0))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn length(&self) -> Option<usize> {
        Some(self.0.len())
    }
    /// `p.k` on a pmap or a psorted is `p["k"]`, `()` when absent, as on
    /// a Map.
    fn field(&self, name: &str) -> Option<Result<Value, String>> {
        match &self.0 {
            Persistent::Vec(_) => None,
            p => Some(Ok(p
                .get(&PKey::Str(Arc::from(name)))
                .cloned()
                .unwrap_or(Value::Unit))),
        }
    }
    fn index(&self, key: &Value) -> Option<Result<Value, String>> {
        Some(match &self.0 {
            Persistent::Vec(v) => match key {
                Value::Integer(i) if *i >= 0 && (*i as usize) < v.len() => {
                    Ok(v[*i as usize].clone())
                }
                Value::Integer(i) => Err(format!(
                    "Index {i} out of bounds for a PVec of length {}",
                    v.len()
                )),
                other => Err(format!(
                    "a PVec takes an Int position, got {}",
                    other.type_name()
                )),
            },
            p => PKey::of(key, p.kind()).map(|k| p.get(&k).cloned().unwrap_or(Value::Unit)),
        })
    }
}

fn wrap(p: Persistent) -> Value {
    Value::Native(NativeHandle::new(PersistentObject(p)))
}

/// The persistent collection a value is, if it is one.
pub fn of(v: &Value) -> Option<&Persistent> {
    match v {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<PersistentObject>()
                .map(|o| &o.0)
        }
        _ => None,
    }
}

/// `for x in v` over a pvec: its items (the interpreter's loop).
pub fn items(v: &Value) -> Option<Vec<Value>> {
    match of(v)? {
        Persistent::Vec(items) => Some(items.iter().cloned().collect()),
        _ => None,
    }
}

/// The length and the `i`th item of a pvec, for the VM's loops.
pub fn vec_len(h: &NativeHandle) -> Option<usize> {
    match &h.0.as_any().downcast_ref::<PersistentObject>()?.0 {
        Persistent::Vec(v) => Some(v.len()),
        _ => None,
    }
}

pub fn vec_item(h: &NativeHandle, i: usize) -> Option<Value> {
    match &h.0.as_any().downcast_ref::<PersistentObject>()?.0 {
        Persistent::Vec(v) => v.get(i).cloned(),
        _ => None,
    }
}

const MAP_FNS: &[(&str, usize)] = &[
    ("new", 0),
    ("of", 1),
    ("get", 2),
    ("set", 3),
    ("remove", 2),
    ("has", 2),
    ("len", 1),
    ("keys", 1),
    ("values", 1),
    ("entries", 1),
    ("to_map", 1),
    ("diff", 2),
];
const SORTED_EXTRA: &[(&str, usize)] = &[("first", 1), ("last", 1), ("range", 3)];
const VEC_FNS: &[(&str, usize)] = &[
    ("new", 0),
    ("of", 1),
    ("get", 2),
    ("set", 3),
    ("push", 2),
    ("pop", 1),
    ("last", 1),
    ("len", 1),
    ("slice", 3),
    ("concat", 2),
    ("to_list", 1),
];

fn module(prefix: &str, fns: &[&[(&str, usize)]]) -> Value {
    let mut m = ValueMap::default();
    for list in fns {
        for (name, arity) in *list {
            m.insert(
                name.to_string(),
                Value::Builtin(crate::ast::BuiltinFunction {
                    name: format!("{prefix}.{name}"),
                    arity: *arity,
                }),
            );
        }
    }
    Value::Struct {
        type_name: "Module".into(),
        fields: Arc::new(m).into(),
    }
}

pub fn create_pmap_module() -> Value {
    module("pmap", &[MAP_FNS])
}
pub fn create_psorted_module() -> Value {
    module("psorted", &[MAP_FNS, SORTED_EXTRA])
}
pub fn create_pvec_module() -> Value {
    module("pvec", &[VEC_FNS])
}

fn want<'a>(args: &'a [Value], n: usize, who: &str) -> Result<&'a [Value], String> {
    if args.len() != n {
        return Err(format!(
            "{who} expects {n} argument{}, got {}",
            if n == 1 { "" } else { "s" },
            args.len()
        ));
    }
    Ok(args)
}

/// Build a keyed collection from a Map, an object, a pmap or a psorted.
fn entries_of(v: &Value, who: &str) -> Result<Vec<(PKey, Value)>, String> {
    if let Some(p) = of(v) {
        return match p {
            Persistent::Vec(_) => Err(format!("{who}: a PVec has no keys")),
            p => Ok(p.sorted_entries()),
        };
    }
    let m = match v {
        Value::Map(m) => m.clone(),
        Value::Struct { type_name, fields }
            if type_name == "Object" || type_name == "JsonObject" =>
        {
            fields.to_arc()
        }
        other => {
            return Err(format!("{who}: expected a Map, got {}", other.type_name()));
        }
    };
    Ok(m.iter()
        .map(|(k, v)| (PKey::Str(Arc::from(k.as_str())), v.clone()))
        .collect())
}

fn int_arg(v: &Value, who: &str, what: &str) -> Result<i64, String> {
    match v {
        Value::Integer(n) => Ok(*n),
        other => Err(format!(
            "{who}: {what} must be an Int, got {}",
            other.type_name()
        )),
    }
}

fn pair(k: &PKey, v: &Value) -> Value {
    Value::Tuple(Arc::new(vec![k.value(), v.clone()]))
}

/// `pmap.*` and `psorted.*`.
pub fn call_keyed(prefix: &str, name: &str, args: Vec<Value>) -> Result<Value, String> {
    let who = format!("{prefix}.{name}");
    let sorted = prefix == "psorted";
    let this = |v: &Value| -> Result<Persistent, String> {
        match of(v) {
            Some(p @ Persistent::Map(_)) if !sorted => Ok(p.clone()),
            Some(p @ Persistent::Sorted(_)) if sorted => Ok(p.clone()),
            _ => Err(format!(
                "{who}: expected a {} (from {prefix}.new or {prefix}.of), got {}",
                if sorted { "PSorted" } else { "PMap" },
                match v {
                    Value::Native(h) => h.0.type_name().to_string(),
                    other => other.type_name(),
                }
            )),
        }
    };
    match name {
        "new" => {
            want(&args, 0, &who)?;
            Ok(wrap(if sorted {
                Persistent::Sorted(POrdMap::new())
            } else {
                Persistent::Map(PHashMap::default())
            }))
        }
        "of" => {
            let a = want(&args, 1, &who)?;
            let e = entries_of(&a[0], &who)?;
            Ok(wrap(if sorted {
                Persistent::Sorted(e.into_iter().collect())
            } else {
                Persistent::Map(e.into_iter().collect())
            }))
        }
        "get" | "has" | "remove" => {
            let a = want(&args, 2, &who)?;
            let p = this(&a[0])?;
            let k = PKey::of(&a[1], &who)?;
            Ok(match (name, p) {
                ("get", p) => p.get(&k).cloned().unwrap_or(Value::Unit),
                ("has", p) => Value::Boolean(p.get(&k).is_some()),
                (_, Persistent::Map(m)) => wrap(Persistent::Map(m.without(&k))),
                (_, Persistent::Sorted(m)) => wrap(Persistent::Sorted(m.without(&k))),
                _ => unreachable!("keyed"),
            })
        }
        "set" => {
            let a = want(&args, 3, &who)?;
            let k = PKey::of(&a[1], &who)?;
            Ok(wrap(match this(&a[0])? {
                Persistent::Map(m) => Persistent::Map(m.update(k, a[2].clone())),
                Persistent::Sorted(m) => Persistent::Sorted(m.update(k, a[2].clone())),
                _ => unreachable!("keyed"),
            }))
        }
        "len" => Ok(Value::Integer(this(&want(&args, 1, &who)?[0])?.len() as i64)),
        "keys" | "values" | "entries" => {
            let p = this(&want(&args, 1, &who)?[0])?;
            let e = p.sorted_entries();
            Ok(Value::List(Arc::new(
                e.iter()
                    .map(|(k, v)| match name {
                        "keys" => k.value(),
                        "values" => v.clone(),
                        _ => pair(k, v),
                    })
                    .collect(),
            )))
        }
        "to_map" => Ok(this(&want(&args, 1, &who)?[0])?.to_plain()),
        "diff" => {
            let a = want(&args, 2, &who)?;
            let (old, new) = (this(&a[0])?, this(&a[1])?);
            Ok(diff(&old, &new))
        }
        "first" | "last" => {
            let p = this(&want(&args, 1, &who)?[0])?;
            let Persistent::Sorted(m) = p else {
                unreachable!("sorted")
            };
            let e = if name == "first" {
                m.get_min()
            } else {
                m.get_max()
            };
            Ok(e.map(|(k, v)| pair(k, v)).unwrap_or(Value::Unit))
        }
        "range" => {
            let a = want(&args, 3, &who)?;
            let Persistent::Sorted(m) = this(&a[0])? else {
                unreachable!("sorted")
            };
            use std::ops::Bound;
            let lo = match &a[1] {
                Value::Unit => Bound::Unbounded,
                v => Bound::Included(PKey::of(v, &who)?),
            };
            let hi = match &a[2] {
                Value::Unit => Bound::Unbounded,
                v => Bound::Excluded(PKey::of(v, &who)?),
            };
            if let (Bound::Included(l), Bound::Excluded(h)) = (&lo, &hi)
                && l >= h
            {
                return Ok(Value::List(Arc::new(Vec::new())));
            }
            Ok(Value::List(Arc::new(
                m.range((lo, hi)).map(|(k, v)| pair(k, v)).collect(),
            )))
        }
        _ => Err(format!("Unknown {prefix} function: {name}")),
    }
}

/// What changed from `old` to `new`: `#{ added, removed, changed }`, each
/// a list of keys in key order. A psorted pair is compared by walking
/// both trees, skipping the subtrees they share.
fn diff(old: &Persistent, new: &Persistent) -> Value {
    let (mut added, mut removed, mut changed) = (Vec::new(), Vec::new(), Vec::new());
    match (old, new) {
        (Persistent::Sorted(a), Persistent::Sorted(b)) => {
            for d in a.diff(b) {
                match d {
                    im::ordmap::DiffItem::Add(k, _) => added.push(k.clone()),
                    im::ordmap::DiffItem::Remove(k, _) => removed.push(k.clone()),
                    im::ordmap::DiffItem::Update {
                        old: (k, x),
                        new: (_, y),
                    } => {
                        if !crate::interpreter::ops::loose_eq(x, y) {
                            changed.push(k.clone())
                        }
                    }
                }
            }
        }
        (Persistent::Map(a), Persistent::Map(b)) => {
            if !a.ptr_eq(b) {
                for (k, x) in a.iter() {
                    match b.get(k) {
                        None => removed.push(k.clone()),
                        Some(y) if !x.same(y) && !crate::interpreter::ops::loose_eq(x, y) => {
                            changed.push(k.clone())
                        }
                        _ => {}
                    }
                }
                for k in b.keys() {
                    if !a.contains_key(k) {
                        added.push(k.clone());
                    }
                }
            }
        }
        _ => {}
    }
    for l in [&mut added, &mut removed, &mut changed] {
        l.sort();
    }
    let list = |l: Vec<PKey>| Value::List(Arc::new(l.iter().map(|k| k.value()).collect()));
    let mut m = ValueMap::default();
    m.insert("added".into(), list(added));
    m.insert("removed".into(), list(removed));
    m.insert("changed".into(), list(changed));
    Value::Map(Arc::new(m))
}

/// `pvec.*`.
pub fn call_vec(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let who = format!("pvec.{name}");
    let this = |v: &Value| -> Result<PVector, String> {
        match of(v) {
            Some(Persistent::Vec(x)) => Ok(x.clone()),
            _ => Err(format!(
                "{who}: expected a PVec (from pvec.new or pvec.of), got {}",
                match v {
                    Value::Native(h) => h.0.type_name().to_string(),
                    other => other.type_name(),
                }
            )),
        }
    };
    let pos = |i: i64, len: usize, past_end_ok: bool| -> Result<usize, String> {
        let limit = if past_end_ok {
            len
        } else {
            len.saturating_sub(1)
        };
        if i < 0 || (i as usize) > limit || (!past_end_ok && len == 0) {
            Err(format!(
                "{who}: position {i} out of bounds for a PVec of length {len}"
            ))
        } else {
            Ok(i as usize)
        }
    };
    match name {
        "new" => {
            want(&args, 0, &who)?;
            Ok(wrap(Persistent::Vec(PVector::new())))
        }
        "of" => {
            let a = want(&args, 1, &who)?;
            match &a[0] {
                Value::List(items) => Ok(wrap(Persistent::Vec(items.iter().cloned().collect()))),
                v => match of(v) {
                    Some(Persistent::Vec(x)) => Ok(wrap(Persistent::Vec(x.clone()))),
                    _ => Err(format!("{who}: expected a List, got {}", v.type_name())),
                },
            }
        }
        "get" => {
            let a = want(&args, 2, &who)?;
            let v = this(&a[0])?;
            let i = pos(int_arg(&a[1], &who, "the position")?, v.len(), false)?;
            Ok(v[i].clone())
        }
        "set" => {
            let a = want(&args, 3, &who)?;
            let v = this(&a[0])?;
            let i = pos(int_arg(&a[1], &who, "the position")?, v.len(), false)?;
            Ok(wrap(Persistent::Vec(v.update(i, a[2].clone()))))
        }
        "push" => {
            let a = want(&args, 2, &who)?;
            let mut v = this(&a[0])?;
            v.push_back(a[1].clone());
            Ok(wrap(Persistent::Vec(v)))
        }
        "pop" => {
            let mut v = this(&want(&args, 1, &who)?[0])?;
            v.pop_back();
            Ok(wrap(Persistent::Vec(v)))
        }
        "last" => Ok(this(&want(&args, 1, &who)?[0])?
            .last()
            .cloned()
            .unwrap_or(Value::Unit)),
        "len" => Ok(Value::Integer(this(&want(&args, 1, &who)?[0])?.len() as i64)),
        "slice" => {
            let a = want(&args, 3, &who)?;
            let v = this(&a[0])?;
            let from = int_arg(&a[1], &who, "from")?;
            let to = int_arg(&a[2], &who, "to")?;
            if from < 0 || to < 0 {
                return Err(format!("{who}: a position is negative"));
            }
            let from = (from as usize).min(v.len());
            let to = (to as usize).min(v.len()).max(from);
            Ok(wrap(Persistent::Vec(v.clone().slice(from..to))))
        }
        "concat" => {
            let a = want(&args, 2, &who)?;
            let mut v = this(&a[0])?;
            v.append(this(&a[1])?);
            Ok(wrap(Persistent::Vec(v)))
        }
        "to_list" => Ok(Persistent::Vec(this(&want(&args, 1, &who)?[0])?).to_plain()),
        _ => Err(format!("Unknown pvec function: {name}")),
    }
}

/// The operators: `==` and `!=` with a Map, a List or another persistent
/// collection. Both tiers come here (`native::binary_op_hook`).
pub struct PersistentModule;

impl crate::native::OvmModule for PersistentModule {
    fn name(&self) -> &'static str {
        "persistent"
    }
    fn namespaces(&self) -> Vec<(String, Value)> {
        Vec::new()
    }
    fn dispatch(&self, func: &str, _args: Vec<Value>) -> Result<Value, String> {
        Err(format!(
            "{func} dispatches through the stdlib, not the native registry"
        ))
    }
    fn binary_op(
        &self,
        op: &crate::ast::BinaryOp,
        lhs: &Value,
        rhs: &Value,
    ) -> Option<Result<Value, String>> {
        use crate::ast::BinaryOp as B;
        let eq = matches!(op, B::Equal);
        if !eq && !matches!(op, B::NotEqual) {
            return None;
        }
        let same = equals(lhs, rhs)?;
        Some(Ok(Value::Boolean(same == eq)))
    }
}

/// `==` where one side is a persistent collection; `None` when neither is.
pub fn equals(lhs: &Value, rhs: &Value) -> Option<bool> {
    match (of(lhs), of(rhs)) {
        (Some(a), Some(b)) => Some(a.equals(b)),
        (Some(a), None) => Some(a.equals_plain(rhs).unwrap_or(false)),
        (None, Some(b)) => Some(b.equals_plain(lhs).unwrap_or(false)),
        (None, None) => None,
    }
}

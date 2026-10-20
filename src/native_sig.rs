//! Parameter names and defaults for native builtins (L1), and the methods
//! native values answer to (L2).
//!
//! A native function is Rust, so it has no parameter list a call could
//! name. This table gives it one: `learn.split(df, test: 0.25, seed: 7)`
//! and `ods.sort_by(f, "price", descending: true)` map their names onto
//! positions *before* dispatch, so the function itself never sees a
//! name and every tier binds the same way:
//!
//! - the interpreter binds at the call (`bind`);
//! - the bytecode compiler binds at compile time (`plan`): the names are
//!   static, so the call it emits is the positional call it stands for,
//!   and a named call costs nothing at run time;
//! - the JIT never sees a native call.
//!
//! A call without names never comes here. A short positional call does,
//! once, to have its trailing defaults filled (`fill_defaults`), at the
//! native-module dispatch both tiers share.
//!
//! **The spec language.** One line per function, its parameters in
//! order:
//!
//! | Form | Meaning |
//! |---|---|
//! | `name` | required |
//! | `name?` | optional; the function decides what absence means, and a gap before a later argument is `()` |
//! | `name = lit` | a default (`10`, `0.5`, `"min"`, `true`, `()`), filled whenever the call stops short of it |
//! | `**name{k1 k2}` | the trailing options map: named arguments that are no parameter go into it, merged over a map given by position. Listed keys are checked here; an empty list leaves the check to the function |
//! | `… -> K` | what it returns, when a native kind (`S`, `F`, `M`, `D`, `R`, or `=` for its first argument's): the checker and the language server follow bindings through it |
//! | `name:SF` | the first parameter's receiver kinds, which make the function a method of those values: `S` Series (and a column expression), `F` Frame, `M` fitted model, `D` Date, `R` streaming reader |
//!
//! **Methods.** `recv.m(args)` on a native value, with no `impl` of `m`
//! for its type, is `home.m(recv, args)`: the value's own module first,
//! then the modules its type borrows from (`METHOD_MODULES`). A function
//! is a method of a value when its first parameter takes that kind; a
//! function the table does not list yet is a method of its home module's
//! values. A map whose `model` is a fitted model (what every `learn`
//! fit returns) takes `learn`'s model methods, so `fit.predict(test)`
//! reads as the fit it is.

use crate::ast::{Value, ValueMap};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

/// A default as written in the table. Kept as data rather than a `Value`
/// so the table is a plain static; a `Value` is built when one is used.
#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Int(i64),
    Float(f64),
    Str(&'static str),
    Bool(bool),
    Unit,
}

impl Lit {
    pub fn value(&self) -> Value {
        match self {
            Lit::Int(n) => Value::Integer(*n),
            Lit::Float(x) => Value::Float(*x),
            Lit::Str(s) => Value::String(Arc::new((*s).to_string())),
            Lit::Bool(b) => Value::Boolean(*b),
            Lit::Unit => Value::Unit,
        }
    }

    fn text(&self) -> String {
        match self {
            Lit::Int(n) => n.to_string(),
            Lit::Float(x) => crate::ast::format_float(*x),
            Lit::Str(s) => format!("\"{}\"", s),
            Lit::Bool(b) => b.to_string(),
            Lit::Unit => "()".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Default {
    Required,
    Optional,
    Lit(Lit),
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: &'static str,
    pub default: Default,
}

/// The trailing options map.
#[derive(Debug, Clone)]
pub struct Options {
    pub name: &'static str,
    /// The keys it takes; empty when the function checks them itself.
    pub keys: Vec<&'static str>,
}

/// Receiver kinds, as a bit set.
pub const RECV_SERIES: u8 = 1;
pub const RECV_FRAME: u8 = 2;
pub const RECV_MODEL: u8 = 4;
pub const RECV_DATE: u8 = 8;
pub const RECV_READER: u8 = 16;

#[derive(Debug, Clone)]
pub struct Sig {
    /// The full builtin name (`ods.rolling`).
    pub full: &'static str,
    pub params: Vec<Param>,
    pub options: Option<Options>,
    /// What the first parameter takes, as a method receiver (0: not a method).
    pub recv: u8,
    /// What it returns, when that is a native kind the tools can follow:
    /// a receiver bit, `RET_SAME` (the kind of its first argument), or 0.
    pub ret: u8,
}

/// `-> =`: the function returns what its first argument is (`ods.head`
/// of a Series is a Series, of a Frame a Frame).
pub const RET_SAME: u8 = 128;

/// The type name a receiver bit stands for.
pub fn kind_name(bit: u8) -> Option<&'static str> {
    match bit {
        RECV_SERIES => Some("Series"),
        RECV_FRAME => Some("Frame"),
        RECV_MODEL => Some("Model"),
        RECV_DATE => Some("Date"),
        RECV_READER => Some("Reader"),
        _ => None,
    }
}

/// The module whose functions a native kind's methods are.
pub fn home_of(type_name: &str) -> Option<&'static str> {
    match type_name {
        "Series" | "Expr" | "Frame" | "Reader" => Some("ods"),
        "Model" => Some("learn"),
        "Date" => Some("dates"),
        _ => None,
    }
}

/// The native kind a call of `full` answers, given the kind of its first
/// argument: what the checker and the language server follow a binding
/// through (`let s = f["a"].fill_null(0.0)` is a Series).
pub fn returns(full: &str, first: Option<&str>) -> Option<&'static str> {
    let s = sig(full)?;
    if s.ret == RET_SAME {
        let first = first?;
        return ["Series", "Frame", "Model", "Date", "Reader"]
            .into_iter()
            .find(|k| *k == first);
    }
    kind_name(s.ret)
}

impl Sig {
    /// `ods.rolling(s, window, agg)`, with defaults and the options' keys:
    /// what `:help` and the language server show.
    pub fn display(&self) -> String {
        let mut parts: Vec<String> = self
            .params
            .iter()
            .map(|p| match &p.default {
                Default::Required => p.name.to_string(),
                Default::Optional => format!("{}?", p.name),
                Default::Lit(l) => format!("{} = {}", p.name, l.text()),
            })
            .collect();
        if let Some(o) = &self.options {
            if o.keys.is_empty() {
                parts.push(format!("{}…", o.name));
            } else {
                parts.extend(o.keys.iter().map(|k| format!("{}:", k)));
            }
        }
        format!("{}({})", self.full, parts.join(", "))
    }

    /// Every name a call may use: the parameters, then the options' keys.
    pub fn names(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self.params.iter().map(|p| p.name).collect();
        if let Some(o) = &self.options {
            out.extend(o.keys.iter().copied());
        }
        out
    }

    fn takes_list(&self) -> String {
        let params: Vec<&str> = self.params.iter().map(|p| p.name).collect();
        let mut text = if params.is_empty() {
            "no parameters".to_string()
        } else {
            params.join(", ")
        };
        if let Some(o) = &self.options {
            if o.keys.is_empty() {
                text.push_str(&format!(", and options in `{}`", o.name));
            } else {
                text.push_str(&format!(", and the options {}", o.keys.join(", ")));
            }
        }
        text
    }

    /// Whether this function is a method of a value of type `type_name`.
    pub fn is_method_of(&self, type_name: &str) -> bool {
        self.recv & recv_bit(type_name) != 0
    }
}

/// The receiver bit a native type answers to.
pub fn recv_bit(type_name: &str) -> u8 {
    match type_name {
        // A column expression takes every column method (D1's `Expr`).
        "Series" | "Expr" => RECV_SERIES,
        "Frame" => RECV_FRAME,
        "Model" => RECV_MODEL,
        "Date" => RECV_DATE,
        "Reader" => RECV_READER,
        _ => 0,
    }
}

// ---------------------------------------------------------------------
// Parsing the table
// ---------------------------------------------------------------------

fn parse_lit(text: &'static str) -> Lit {
    let t = text.trim();
    if t == "()" {
        Lit::Unit
    } else if t == "true" || t == "false" {
        Lit::Bool(t == "true")
    } else if let Some(s) = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Lit::Str(s)
    } else if let Ok(n) = t.parse::<i64>() {
        Lit::Int(n)
    } else if let Ok(x) = t.parse::<f64>() {
        Lit::Float(x)
    } else {
        panic!("native_sig: bad default `{}`", t)
    }
}

fn parse_spec(full: &'static str, spec: &'static str) -> Sig {
    let (spec, ret) = match spec.rsplit_once(" -> ") {
        Some((head, kind)) => (
            head,
            match kind.trim() {
                "S" => RECV_SERIES,
                "F" => RECV_FRAME,
                "M" => RECV_MODEL,
                "D" => RECV_DATE,
                "R" => RECV_READER,
                "=" => RET_SAME,
                other => panic!("native_sig: {full}: unknown return kind {other}"),
            },
        ),
        None => (spec, 0),
    };
    let mut params = Vec::new();
    let mut options = None;
    let mut recv = 0u8;
    // Split on commas outside braces.
    let mut parts: Vec<&'static str> = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in spec.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&spec[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if !spec[start..].trim().is_empty() {
        parts.push(&spec[start..]);
    }
    for (i, part) in parts.into_iter().enumerate() {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("**") {
            let (name, keys) = match rest.split_once('{') {
                Some((n, k)) => (
                    n.trim(),
                    k.trim_end_matches('}').split_whitespace().collect(),
                ),
                None => (rest.trim(), Vec::new()),
            };
            options = Some(Options { name, keys });
            continue;
        }
        assert!(options.is_none(), "native_sig: {full}: options come last");
        let (head, default) = match part.split_once('=') {
            Some((h, d)) => (h.trim(), Default::Lit(parse_lit(d))),
            None => match part.strip_suffix('?') {
                Some(h) => (h.trim(), Default::Optional),
                None => (part, Default::Required),
            },
        };
        let name = match head.split_once(':') {
            Some((n, kinds)) => {
                assert!(i == 0, "native_sig: {full}: only the first parameter is a receiver");
                for k in kinds.trim().chars() {
                    recv |= match k {
                        'S' => RECV_SERIES,
                        'F' => RECV_FRAME,
                        'M' => RECV_MODEL,
                        'D' => RECV_DATE,
                        'R' => RECV_READER,
                        other => panic!("native_sig: {full}: unknown receiver kind {other}"),
                    };
                }
                n.trim()
            }
            None => head,
        };
        params.push(Param { name, default });
    }
    Sig {
        full,
        params,
        options,
        recv,
        ret,
    }
}

static SIGS: LazyLock<HashMap<&'static str, Sig>> = LazyLock::new(|| {
    TABLE
        .iter()
        .map(|(full, spec)| (*full, parse_spec(full, spec)))
        .collect()
});

/// The signature of a native builtin, by its full name.
pub fn sig(full: &str) -> Option<&'static Sig> {
    SIGS.get(full)
}

/// Every registered signature (the docs test reads them).
pub fn all() -> impl Iterator<Item = &'static Sig> {
    SIGS.values()
}

// ---------------------------------------------------------------------
// Binding
// ---------------------------------------------------------------------

/// Where one argument of the positional call comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    /// The call's argument at this index: positional arguments first (in
    /// order), then the named ones (in the order written).
    Arg(usize),
    /// A default.
    Lit(Lit),
    /// The options map: a positional map (`base`, an argument index) with
    /// the named options laid over it, in order.
    Options {
        base: Option<usize>,
        entries: Vec<(&'static str, usize)>,
    },
}

/// How far a named call reaches, once its names are checked: the length
/// of the positional call, and whether it builds an options map.
struct Layout {
    len: usize,
    options: bool,
}

/// Check a named call's names against the signature (the one place its
/// errors are worded) and measure the positional call. `name_at(j)` is
/// the j-th name as written. Allocates nothing but an error.
fn layout<'n>(
    full: &str,
    n_pos: usize,
    n_named: usize,
    name_at: &dyn Fn(usize) -> &'n str,
) -> Result<(&'static Sig, Layout), String> {
    let Some(sig) = sig(full) else {
        return Err(format!(
            "{} takes its arguments by position: named argument '{}' is not accepted \
             by a builtin",
            full,
            if n_named > 0 { name_at(0) } else { "" }
        ));
    };
    let n = sig.params.len();
    let mut len = n_pos;
    let mut options = false;
    for j in 0..n_named {
        let name = name_at(j);
        if (0..j).any(|k| name_at(k) == name) {
            return Err(format!("Duplicate named argument: {}", name));
        }
        if let Some(i) = sig.params.iter().position(|p| p.name == name) {
            if i < n_pos {
                return Err(format!(
                    "{}: argument '{}' is given both by position and by name",
                    full, name
                ));
            }
            len = len.max(i + 1);
            continue;
        }
        match &sig.options {
            Some(o) if o.keys.is_empty() || o.keys.contains(&name) => options = true,
            _ => {
                return Err(format!(
                    "{}: no parameter {}named '{}' — it takes {}",
                    full,
                    if sig.options.is_some() { "or option " } else { "" },
                    name,
                    sig.takes_list()
                ));
            }
        }
    }
    if options {
        len = len.max(n + 1);
    }
    // A required parameter the call skips over.
    for i in n_pos..len.min(n) {
        let p = &sig.params[i];
        if p.default == Default::Required && !(0..n_named).any(|j| name_at(j) == p.name) {
            return Err(format!(
                "{}: missing argument '{}' (argument {})",
                full,
                p.name,
                i + 1
            ));
        }
    }
    Ok((sig, Layout { len, options }))
}

/// Where parameter `i` (at or past the positional ones) comes from: the
/// named argument that names it (its index among the names), else its
/// default. `layout` has ruled out a required gap.
fn source<'n>(sig: &Sig, i: usize, n_named: usize, name_at: &dyn Fn(usize) -> &'n str) -> Result<usize, Lit> {
    let p = &sig.params[i];
    match (0..n_named).find(|&j| name_at(j) == p.name) {
        Some(j) => Ok(j),
        None => Err(match &p.default {
            Default::Lit(l) => l.clone(),
            _ => Lit::Unit,
        }),
    }
}

/// Whether the j-th name is an option rather than a parameter.
fn is_option<'n>(sig: &Sig, j: usize, name_at: &dyn Fn(usize) -> &'n str) -> bool {
    let name = name_at(j);
    !sig.params.iter().any(|p| p.name == name)
}

/// The positional call a named one stands for: `n_pos` positional
/// arguments, then one per name in `names`. Static, so the bytecode
/// compiler plans at compile time what the interpreter binds at run time
/// (both through `layout` and `source`).
pub fn plan(full: &str, n_pos: usize, names: &[&str]) -> Result<Vec<Slot>, String> {
    let name_at = |j: usize| names[j];
    let (sig, lay) = layout(full, n_pos, names.len(), &name_at)?;
    let n = sig.params.len();
    let mut slots = Vec::with_capacity(lay.len.max(n_pos));
    for i in 0..lay.len.max(n_pos) {
        if lay.options && i == n {
            let entries = (0..names.len())
                .filter(|&j| is_option(sig, j, &name_at))
                .map(|j| (option_key(sig, names[j]), n_pos + j))
                .collect();
            slots.push(Slot::Options {
                base: (i < n_pos).then_some(i),
                entries,
            });
        } else if i < n_pos || i > n {
            slots.push(Slot::Arg(i));
        } else {
            slots.push(match source(sig, i, names.len(), &name_at) {
                Ok(j) => Slot::Arg(n_pos + j),
                Err(l) => Slot::Lit(l),
            });
        }
    }
    Ok(slots)
}

/// An option's key as the table spells it, or — a key the table leaves
/// to the function — as the program does.
fn option_key(sig: &Sig, name: &str) -> &'static str {
    sig.options
        .as_ref()
        .and_then(|o| o.keys.iter().find(|k| **k == name).copied())
        .unwrap_or_else(|| leak_name(name))
}

/// Option keys the table does not list come from the program's text, a
/// small closed set per program; keeping one copy of each lets a plan
/// hold `&'static str` throughout.
fn leak_name(name: &str) -> &'static str {
    static NAMES: LazyLock<std::sync::Mutex<std::collections::HashSet<&'static str>>> =
        LazyLock::new(std::default::Default::default);
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = names.get(name) {
        return n;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.insert(leaked);
    leaked
}

/// A value a named call is bound over: the interpreter's `Value`, or
/// the bytecode tier's own (so its method call binds without converting).
pub trait BindValue: Sized {
    fn unit() -> Self;
    fn lit(l: &Lit) -> Self;
    /// The options map: `base` (a map given by position) with `entries`
    /// written over it.
    fn options(
        full: &str,
        base: Option<Self>,
        entries: Vec<(&'static str, Self)>,
    ) -> Result<Self, String>;
}

impl BindValue for Value {
    fn unit() -> Self {
        Value::Unit
    }
    fn lit(l: &Lit) -> Self {
        l.value()
    }
    fn options(
        full: &str,
        base: Option<Self>,
        entries: Vec<(&'static str, Self)>,
    ) -> Result<Self, String> {
        options_map(full, base, entries.into_iter())
    }
}

/// Lay out a named call's values as the positional call: the
/// interpreter's half of `plan`. `vals` holds the `n_pos` positional
/// values, then one per name (`name_at(j)` the j-th, as written). The
/// result allocates only itself (and an options map when the call
/// builds one).
pub fn bind_values<'n, V: BindValue>(
    full: &str,
    mut vals: Vec<V>,
    n_pos: usize,
    name_at: &dyn Fn(usize) -> &'n str,
) -> Result<Vec<V>, String> {
    let n_named = vals.len() - n_pos;
    let (sig, lay) = layout(full, n_pos, n_named, name_at)?;
    let n = sig.params.len();
    let total = lay.len.max(n_pos);
    let mut out = Vec::with_capacity(total);
    let take = |vals: &mut Vec<V>, k: usize| std::mem::replace(&mut vals[k], V::unit());
    for i in 0..total {
        if lay.options && i == n {
            let base = (i < n_pos).then(|| take(&mut vals, i));
            let mut entries = Vec::new();
            for j in 0..n_named {
                if is_option(sig, j, name_at) {
                    entries.push((option_key(sig, name_at(j)), take(&mut vals, n_pos + j)));
                }
            }
            out.push(V::options(full, base, entries)?);
        } else if i < n_pos || i > n {
            out.push(take(&mut vals, i));
        } else {
            out.push(match source(sig, i, n_named, name_at) {
                Ok(j) => take(&mut vals, n_pos + j),
                Err(l) => V::lit(&l),
            });
        }
    }
    Ok(out)
}

/// `bind_values` over owned names.
pub fn bind(
    full: &str,
    positional: Vec<Value>,
    named: Vec<(String, Value)>,
) -> Result<Vec<Value>, String> {
    let n_pos = positional.len();
    let mut vals = positional;
    let mut names = Vec::with_capacity(named.len());
    for (k, v) in named {
        names.push(k);
        vals.push(v);
    }
    bind_values(full, vals, n_pos, &|j| names[j].as_str())
}

/// The options map of a call: the positional map, with the named options
/// written over it in order.
pub fn options_map(
    full: &str,
    base: Option<Value>,
    entries: impl Iterator<Item = (&'static str, Value)>,
) -> Result<Value, String> {
    let mut map: ValueMap = match base {
        None | Some(Value::Unit) => ValueMap::default(),
        Some(Value::Map(ref m)) => (**m).clone(),
        Some(other) => {
            return Err(format!(
                "{}: the options must be a map (#{{...}}), got {}",
                full,
                other.type_name()
            ));
        }
    };
    for (k, v) in entries {
        map.insert(k.to_string(), v);
    }
    Ok(Value::Map(Arc::new(map)))
}

/// A short positional call to a native-module function completed with its
/// trailing defaults (`ods.head(f)` is `ods.head(f, 10)`). Stops at the
/// first parameter with no literal default, so a function's own handling
/// of an absent argument stays its own.
pub fn fill_defaults(full: &str, args: &mut Vec<Value>) {
    let Some(sig) = sig(full) else { return };
    if args.len() >= sig.params.len() {
        return;
    }
    for p in &sig.params[args.len()..] {
        match &p.default {
            Default::Lit(l) => args.push(l.value()),
            _ => break,
        }
    }
}

// ---------------------------------------------------------------------
// Methods
// ---------------------------------------------------------------------

/// Modules a native type borrows methods from, after its own.
fn borrowed_modules(type_name: &str) -> &'static [&'static str] {
    match type_name {
        "Series" | "Expr" => &["stats", "plot"],
        "Frame" => &["plot"],
        _ => &[],
    }
}

/// Home modules whose functions are methods of their values even before
/// the table lists them (the data stack grows faster than this table).
const OPEN_HOMES: &[&str] = &["ods", "learn", "dates"];

/// module → function → full name, for every builtin module.
static MODULE_FUNCTIONS: LazyLock<HashMap<String, HashMap<String, &'static str>>> =
    LazyLock::new(|| {
        let mut out: HashMap<String, HashMap<String, &'static str>> = HashMap::new();
        for (module, value) in crate::stdlib::get_stdlib().iter() {
            if let Value::Struct { fields, .. } = value {
                let entry = out.entry(module.clone()).or_default();
                for (name, v) in fields.iter() {
                    if let Value::Builtin(b) = v {
                        let full: &'static str = Box::leak(b.name.clone().into_boxed_str());
                        entry.insert(name.clone(), full);
                    }
                }
            }
        }
        out
    });

/// The builtin `recv.method(…)` calls on a native value of `type_name`
/// owned by module `home`, if it is one of its methods.
pub fn method_target(home: &str, type_name: &str, method: &str) -> Option<&'static str> {
    // The data stack's types answer from one table built once: a method
    // call costs two hashes of short strings.
    if let Some(methods) = METHOD_TABLE.get(type_name)
        && home_of(type_name) == Some(home)
    {
        return methods.get(method).copied();
    }
    resolve_method(home, type_name, method)
}

type FxMap<K, V> = std::collections::HashMap<K, V, rustc_hash::FxBuildHasher>;

/// type → method → builtin, for the types `home_of` names.
static METHOD_TABLE: LazyLock<FxMap<&'static str, FxMap<&'static str, &'static str>>> =
    LazyLock::new(|| {
        let mut out: FxMap<&'static str, FxMap<&'static str, &'static str>> = FxMap::default();
        for type_name in ["Series", "Expr", "Frame", "Model", "Date", "Reader"] {
            let home = home_of(type_name).expect("a data type");
            let mut methods = FxMap::default();
            for full in methods_from(home, type_name) {
                let name: &'static str = full.rsplit('.').next().unwrap_or(full);
                methods.insert(name, full);
            }
            out.insert(type_name, methods);
        }
        out
    });

/// Every method a type has, as full names, resolved one by one.
fn methods_from(home: &str, type_name: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for module in std::iter::once(&home).chain(borrowed_modules(type_name).iter()) {
        let Some(functions) = MODULE_FUNCTIONS.get(*module) else {
            continue;
        };
        let mut names: Vec<&String> = functions.keys().collect();
        names.sort();
        for name in names {
            if seen.contains(name.as_str()) {
                continue;
            }
            if let Some(full) = resolve_method(home, type_name, name) {
                seen.insert(name.clone());
                out.push(full);
            }
        }
    }
    out
}

/// `method_target` from first principles: the home module, then the
/// borrowed ones.
fn resolve_method(home: &str, type_name: &str, method: &str) -> Option<&'static str> {
    let try_module = |module: &str, open: bool| -> Option<&'static str> {
        let full = *MODULE_FUNCTIONS.get(module)?.get(method)?;
        match sig(full) {
            Some(s) => s.is_method_of(type_name).then_some(full),
            None => open.then_some(full),
        }
    };
    if let Some(full) = try_module(home, OPEN_HOMES.contains(&home)) {
        return Some(full);
    }
    borrowed_modules(type_name)
        .iter()
        .find_map(|m| try_module(m, false))
}

/// The methods of a value of `type_name` owned by `home`, as full names:
/// what completes after `value.`.
pub fn methods_of(home: &str, type_name: &str) -> Vec<&'static str> {
    methods_from(home, type_name)
}

/// The home module and type of a native receiver for method calls: the
/// value's own, or — for a map whose `model` is a fitted model, as every
/// `learn` fit is — learn's model.
pub fn method_receiver(value: &Value) -> Option<(&'static str, &'static str)> {
    match value {
        Value::Native(h) => Some((h.0.module(), h.0.type_name())),
        Value::Map(m) => match m.get("model") {
            Some(Value::Native(h)) if h.0.type_name() == "Model" => Some(("learn", "Model")),
            _ => None,
        },
        _ => None,
    }
}

/// The error for a method a native value does not have.
pub fn no_method(type_name: &str, method: &str) -> String {
    format!(
        "a {} has no method '{}' (and no `impl` gives it one)",
        type_name, method
    )
}

// ---------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------

macro_rules! plot_opts {
    () => {
        "**opts{title x_label y_label width height theme paper ink responsive interactive colors vary scale font_size font}"
    };
}

/// (full name, parameters). Names follow the reference (docs/ods.md,
/// docs/stdlib.md) and the functions' own error messages.
pub const TABLE: &[(&str, &str)] = &[
    ("ods.version", ""),
    // ── ods: Series ──
    ("ods.series", "values -> S"),
    ("ods.try_series", "values"),
    ("ods.date_part", "s:S, unit -> S"),
    ("ods.zeros", "n -> S"),
    ("ods.linspace", "start, stop, num -> S"),
    ("ods.to_list", "s:S"),
    ("ods.len", "s:S"),
    ("ods.get", "s:S, i"),
    ("ods.null_count", "s:S"),
    ("ods.is_null", "s:S -> S"),
    ("ods.fill_null", "s:S, value, limit? -> S"),
    ("ods.sum", "s:S"),
    ("ods.mean", "s:S"),
    ("ods.var", "s:S"),
    ("ods.std", "s:S"),
    ("ods.min", "s:S"),
    ("ods.max", "s:S"),
    ("ods.quantile", "s:S, q"),
    ("ods.median", "s:S"),
    ("ods.unique", "s:S -> S"),
    ("ods.n_unique", "s:S"),
    ("ods.value_counts", "s:S -> F"),
    ("ods.cast", "s:S, type -> S"),
    ("ods.sample", "x:SF, n -> ="),
    ("ods.shift", "s:S, by -> S"),
    ("ods.cum_max", "s:S -> S"),
    ("ods.cum_min", "s:S -> S"),
    ("ods.rank", "s:S, method = \"min\" -> S"),
    ("ods.rolling", "s:S, window, agg -> S"),
    ("ods.cumsum", "s:S -> S"),
    ("ods.map", "s:S, fn -> S"),
    ("ods.dot", "a:S, b"),
    ("ods.sort", "s:S, descending = false -> S"),
    ("ods.argsort", "s:S, descending = false -> S"),
    ("ods.take", "x:SF, idx -> ="),
    ("ods.filter", "x:SF, mask -> ="),
    ("ods.eq", "a:S, b -> S"),
    ("ods.ne", "a:S, b -> S"),
    ("ods.all_of", "masks -> S"),
    ("ods.any_of", "masks -> S"),
    ("ods.not", "mask:S -> S"),
    ("ods.is_in", "s:S, values -> S"),
    ("ods.contains", "s:S, text -> S"),
    // ── ods: column math, nulls and conditionals (D1) ──
    ("ods.abs", "s:S -> S"),
    ("ods.exp", "s:S -> S"),
    ("ods.exp2", "s:S -> S"),
    ("ods.expm1", "s:S -> S"),
    ("ods.ln", "s:S -> S"),
    ("ods.ln1p", "s:S -> S"),
    ("ods.log2", "s:S -> S"),
    ("ods.log10", "s:S -> S"),
    ("ods.sqrt", "s:S -> S"),
    ("ods.cbrt", "s:S -> S"),
    ("ods.sin", "s:S -> S"),
    ("ods.cos", "s:S -> S"),
    ("ods.tan", "s:S -> S"),
    ("ods.asin", "s:S -> S"),
    ("ods.acos", "s:S -> S"),
    ("ods.atan", "s:S -> S"),
    ("ods.sinh", "s:S -> S"),
    ("ods.cosh", "s:S -> S"),
    ("ods.tanh", "s:S -> S"),
    ("ods.sigmoid", "s:S -> S"),
    ("ods.logit", "s:S -> S"),
    ("ods.sign", "s:S -> S"),
    ("ods.floor", "s:S -> S"),
    ("ods.ceil", "s:S -> S"),
    ("ods.trunc", "s:S -> S"),
    ("ods.pow", "s:S, exponent -> S"),
    ("ods.round", "s:S, digits = 0 -> S"),
    ("ods.clip", "s:S, lo?, hi? -> S"),
    ("ods.between", "s:S, lo, hi?, closed = \"both\" -> S"),
    ("ods.where", "mask:S, a, b -> S"),
    ("ods.coalesce", "a:S, b? -> S"),
    ("ods.not_null", "s:S -> S"),
    ("ods.is_nan", "s:S -> S"),
    ("ods.interpolate", "s:S, limit? -> S"),
    ("ods.recode", "s:S, mapping, default? -> S"),
    ("ods.full", "n, value -> S"),
    ("ods.ones", "n -> S"),
    ("ods.full_like", "s:S, value -> S"),
    ("ods.arange", "start, stop?, step = 1 -> S"),
    ("ods.repeat", "s:S, times -> S"),
    ("ods.tile", "s:S, n -> S"),
    ("ods.argmax", "s:S"),
    ("ods.argmin", "s:S"),
    ("ods.diff", "s:S, lag = 1 -> S"),
    ("ods.pct_change", "s:S, lag = 1 -> S"),
    ("ods.cumprod", "s:S -> S"),
    ("ods.mode", "s:S"),
    ("ods.any", "s:S"),
    ("ods.all", "s:S"),
    ("ods.searchsorted", "s:S, v, side = \"left\""),
    ("ods.count", "s:S"),
    ("ods.first", "s:S"),
    ("ods.last", "s:S"),
    // ── ods: column expressions (D1) ──
    ("ods.col", "name -> S"),
    ("ods.lit", "v -> S"),
    ("ods.alias", "e:S, name -> S"),
    ("ods.over", "agg:S, keys -> S"),
    ("ods.with", "f:F, exprs -> F"),
    // ── ods: Frame ──
    ("ods.frame", "columns -> F"),
    ("ods.read_csv", "text -> F"),
    ("ods.read_csv_file", "path"),
    ("ods.read_jsonl", "text -> F"),
    ("ods.read_jsonl_file", "path"),
    ("ods.open_csv", "path"),
    ("ods.open_jsonl", "path"),
    ("ods.next_chunk", "reader:R, rows"),
    ("ods.rows_read", "reader:R"),
    ("ods.at_end", "reader:R"),
    ("ods.to_csv", "f:F"),
    ("ods.write_csv", "f:F, path"),
    ("ods.to_jsonl", "f:F"),
    ("ods.write_jsonl", "f:F, path"),
    ("ods.read_frame", "path, columns?"),
    ("ods.write_frame", "f:F, path"),
    ("ods.frame_info", "path"),
    ("ods.concat", "frames -> F"),
    ("ods.frame_from_records", "records -> F"),
    ("ods.try_frame_from_records", "records"),
    ("ods.to_matrix", "f:F, names"),
    ("ods.split", "f:F, frac"),
    ("ods.split_at", "f:F, column, frac"),
    ("ods.to_records", "f:F"),
    ("ods.columns", "f:F"),
    ("ods.column", "f:F, name -> S"),
    ("ods.n_rows", "f:F"),
    ("ods.n_cols", "f:F"),
    // A Frame's columns (`f.select(["a", col("b") * 2.0])`), or — given
    // a list of masks — np.select's first-true-wins (`ods.select([m1, m2],
    // [v1, v2], default: v)`): one name, told apart by its first argument.
    ("ods.select", "x:F, items, default? -> ="),
    ("ods.with_column", "f:F, name, values -> F"),
    ("ods.sort_by", "f:F, by, descending = false -> F"),
    ("ods.head", "x:SF, n = 10 -> ="),
    ("ods.tail", "x:SF, n = 10 -> ="),
    ("ods.rename", "f:F, mapping -> F"),
    ("ods.drop", "f:F, names -> F"),
    ("ods.distinct", "f:F, names? -> F"),
    ("ods.drop_null", "f:F, names? -> F"),
    ("ods.describe", "f:F -> F"),
    ("ods.schema", "f:F -> F"),
    ("ods.group_by", "f:F, by, aggs -> F"),
    ("ods.join", "left:F, right, on, right_on? -> F"),
    ("ods.join_left", "left:F, right, on, right_on? -> F"),
    ("ods.join_full", "left:F, right, on, right_on? -> F"),
    ("ods.join_semi", "left:F, right, on, right_on? -> F"),
    ("ods.join_anti", "left:F, right, on, right_on? -> F"),
    ("ods.pivot", "f:F, index, columns, values, agg -> F"),
    ("ods.unpivot", "f:F, id_columns, value_columns? -> F"),
    // ── stats ──
    ("stats.describe", "s:S"),
    ("stats.corr", "a:S, b"),
    ("stats.cov", "a:S, b"),
    ("stats.t_test", "a:S, b"),
    ("stats.chi2_test", "observed:S, expected"),
    ("stats.lm", "y:S, x"),
    ("stats.norm.pdf", "x, mean = 0.0, std = 1.0"),
    ("stats.norm.cdf", "x, mean = 0.0, std = 1.0"),
    ("stats.norm.ppf", "p, mean = 0.0, std = 1.0"),
    ("stats.norm.sample", "n, mean = 0.0, std = 1.0 -> S"),
    ("stats.t.pdf", "x, df"),
    ("stats.t.cdf", "x, df"),
    ("stats.t.ppf", "p, df"),
    ("stats.t.sample", "n, df -> S"),
    ("stats.chi2.pdf", "x, df"),
    ("stats.chi2.cdf", "x, df"),
    ("stats.chi2.ppf", "p, df"),
    ("stats.chi2.sample", "n, df -> S"),
    ("stats.f.pdf", "x, d1, d2"),
    ("stats.f.cdf", "x, d1, d2"),
    ("stats.f.ppf", "p, d1, d2"),
    ("stats.f.sample", "n, d1, d2 -> S"),
    // ── plot ──
    ("plot.line", concat!("x:S, y, ", plot_opts!())),
    ("plot.scatter", concat!("x:S, y, ", plot_opts!())),
    ("plot.area", concat!("x:S, y, ", plot_opts!())),
    ("plot.lines", concat!("x:S, series, ", plot_opts!())),
    ("plot.xy", concat!("layers, ", plot_opts!())),
    ("plot.bar", concat!("labels:S, values, ", plot_opts!())),
    ("plot.bars", concat!("labels:S, series, ", plot_opts!())),
    ("plot.stacked", concat!("labels:S, series, ", plot_opts!())),
    ("plot.hist", concat!("s:S, bins = 10, ", plot_opts!())),
    ("plot.heatmap", concat!("x_labels, y_labels, rows, ", plot_opts!())),
    ("plot.box", concat!("series, ", plot_opts!())),
    ("plot.ramp", "scale, t"),
    ("plot.describe", "svg"),
    ("plot.gg", "plot"),
    ("plot.gg_scene", "plot"),
    // ── learn ──
    ("learn.split", "data:F, **opts{test stratify shuffle seed}"),
    ("learn.kfold", "data:F, k, **opts{shuffle seed stratify} -> S"),
    ("learn.folds", "data:F, k, **opts{shuffle seed stratify}"),
    ("learn.time_splits", "data:F, n_splits, **opts{test_size gap max_train}"),
    ("learn.scaler", "data:F, cols?, **opts{method} -> M"),
    ("learn.one_hot", "data:F, cols?, **opts{drop_first} -> M"),
    ("learn.imputer", "data:F, cols?, **opts{strategy fill} -> M"),
    ("learn.transform", "model:M, data -> F"),
    ("learn.poly", "data:F, cols, degree, **opts{interaction_only} -> F"),
    ("learn.linear", "data:F, y, x?, **opts{weights task class_weight intercept} -> M"),
    ("learn.ridge", "data:F, y, x?, **opts{weights task class_weight alpha intercept} -> M"),
    ("learn.lasso", "data:F, y, x?, **opts{weights task class_weight alpha intercept max_iter tol} -> M"),
    ("learn.elastic_net", "data:F, y, x?, **opts{weights task class_weight alpha l1_ratio intercept max_iter tol} -> M"),
    ("learn.logistic", "data:F, y, x?, **opts{weights task class_weight penalty C max_iter tol} -> M"),
    ("learn.tree", "data:F, y, x?, **opts{weights task class_weight criterion max_depth min_samples_split min_samples_leaf max_features min_impurity_decrease seed} -> M"),
    ("learn.forest", "data:F, y, x?, **opts{weights task class_weight criterion max_depth min_samples_split min_samples_leaf max_features min_impurity_decrease n_trees bootstrap max_samples oob seed} -> M"),
    ("learn.boost", "data:F, y, x?, **opts{weights task class_weight n_trees learning_rate max_depth max_leaf_nodes min_samples_leaf l2 subsample max_bins early_stopping validation_fraction validation_rows validation_tail n_iter_no_change tol seed} -> M"),
    ("learn.knn", "data:F, y, x?, **opts{weights task class_weight k weights_by} -> M"),
    ("learn.naive_bayes", "data:F, y, x?, **opts{weights task class_weight var_smoothing} -> M"),
    ("learn.kmeans", "data:F, cols?, k, **opts{n_init max_iter tol seed} -> M"),
    ("learn.gmm", "data:F, cols?, k, **opts{max_iter tol reg_covar seed} -> M"),
    ("learn.pca", "data:F, cols?, **opts{n_components scale} -> M"),
    ("learn.silhouette", "data:F, cols?, labels, **opts{sample seed}"),
    ("learn.predict", "model:M, data, **opts{kind level trees} -> S"),
    ("learn.importance", "model:M -> F"),
    ("learn.summary", "model:M, **opts{level} -> F"),
    ("learn.params", "model:M"),
    ("learn.score", "model:M, data, metric"),
    ("learn.spec", "kind, y?, x?, options?"),
    ("learn.fit", "spec, data -> M"),
    ("learn.cross_validate", "fit:M, data, **opts{folds metrics metric seed shuffle} -> F"),
    ("learn.permutation_importance", "model:M, data, **opts{metric repeats seed} -> F"),
    ("learn.partial_dependence", "model:M, data, feature, **opts{grid points sample seed} -> F"),
    ("learn.learning_curve", "fit:M, data, **opts{sizes folds metric seed} -> F"),
    ("learn.grid_search", "fit:M, data, grid, **opts{folds metric seed}"),
    ("learn.rmse", "y:S, pred"),
    ("learn.mse", "y:S, pred"),
    ("learn.mae", "y:S, pred"),
    ("learn.mape", "y:S, pred"),
    ("learn.r2", "y:S, pred"),
    ("learn.accuracy", "y:S, pred"),
    ("learn.precision", "y:S, pred, **opts{average pos}"),
    ("learn.recall", "y:S, pred, **opts{average pos}"),
    ("learn.f1", "y:S, pred, **opts{average pos}"),
    ("learn.log_loss", "y:S, p, **opts{pos}"),
    ("learn.brier", "y:S, p, **opts{pos}"),
    ("learn.auc", "y:S, p, **opts{pos}"),
    ("learn.average_precision", "y:S, p, **opts{pos}"),
    ("learn.roc", "y:S, p, **opts{pos} -> F"),
    ("learn.pr_curve", "y:S, p, **opts{pos} -> F"),
    ("learn.confusion", "y:S, pred"),
    ("learn.classification_report", "y:S, pred -> F"),
    ("learn.at_threshold", "y:S, p, **opts{threshold pos}"),
    ("learn.threshold_sweep", "y:S, p, **opts{pos} -> F"),
    ("learn.calibration", "y:S, p, **opts{bins strategy pos} -> F"),
    // ── dates: a Date's methods ──
    ("dates.format_date", "d:D, fmt"),
    ("dates.add_days", "d:D, n"),
    ("dates.add_weeks", "d:D, n"),
    ("dates.add_months", "d:D, n"),
    ("dates.add_years", "d:D, n"),
    ("dates.diff_days", "a:D, b"),
    ("dates.year", "d:D"),
    ("dates.month", "d:D"),
    ("dates.day", "d:D"),
    ("dates.weekday", "d:D"),
    ("dates.epoch_day", "d:D"),
    ("dates.start_of_week", "d:D"),
    ("dates.start_of_month", "d:D"),
    // ── the stdlib's options maps ──
    ("gui.open", "**opts"),
    ("gui.headless", "**opts"),
    ("gui.measure", "text, style?, **opts"),
    ("gui.read", "w, what, arg?"),
    ("gui.fonts", "sources, **opts"),
    ("gui.dialog", "kind, **opts"),
    ("gui.compare", "a, b, **opts"),
    ("pty.spawn", "argv, **opts"),
    ("pty.read", "p, **opts"),
    ("pty.kill", "p, signal?"),
    ("pty.wait", "p, ms?"),
    ("vt.new", "rows, cols, **opts"),
    ("vt.render", "t, **opts"),
    ("vt.find", "t, query, **opts"),
    ("tty.enter", "**opts"),
    ("proc.spawn", "argv, **opts"),
    ("proc.kill", "p, **opts"),
    ("proc.pipeline", "stages, **opts"),
    ("os.exec", "argv, **opts"),
    ("fs.scan", "root, **opts"),
    ("fs.search", "root, query, **opts"),
    ("fs.watch", "root, **opts"),
    ("fs.watch_next", "watch, **opts"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_parses() {
        for (full, spec) in TABLE {
            let s = parse_spec(full, spec);
            assert_eq!(s.full, *full);
        }
        assert_eq!(SIGS.len(), TABLE.len(), "a function is listed twice");
    }

    #[test]
    fn plans() {
        // ods.sort_by(f, "x", descending: true)
        assert_eq!(
            plan("ods.sort_by", 2, &["descending"]).unwrap(),
            vec![Slot::Arg(0), Slot::Arg(1), Slot::Arg(2)]
        );
        // ods.sort_by(f, descending: true, by: "x")
        assert_eq!(
            plan("ods.sort_by", 1, &["descending", "by"]).unwrap(),
            vec![Slot::Arg(0), Slot::Arg(2), Slot::Arg(1)]
        );
        // a gap takes its default
        assert_eq!(
            plan("stats.norm.cdf", 1, &["std"]).unwrap(),
            vec![Slot::Arg(0), Slot::Lit(Lit::Float(0.0)), Slot::Arg(1)]
        );
        // options
        assert_eq!(
            plan("learn.split", 1, &["test", "seed"]).unwrap(),
            vec![
                Slot::Arg(0),
                Slot::Options {
                    base: None,
                    entries: vec![("test", 1), ("seed", 2)]
                }
            ]
        );
        assert!(plan("learn.split", 1, &["tst"]).unwrap_err().contains("test, stratify"));
        assert!(plan("ods.sort_by", 2, &["by"]).unwrap_err().contains("both"));
        assert!(plan("ods.rolling", 1, &["agg"]).unwrap_err().contains("missing argument 'window'"));
        assert!(plan("ods.rolling", 2, &["windw"]).unwrap_err().contains("s, window, agg"));
        assert!(plan("len", 0, &["x"]).unwrap_err().contains("by position"));
    }

    #[test]
    fn column_functions_are_methods_of_columns_and_expressions() {
        for (ty, m) in [("Series", "clip"), ("Series", "fill_null"), ("Expr", "over"),
            ("Expr", "alias"), ("Expr", "sqrt"), ("Frame", "with"), ("Frame", "select")]
        {
            assert!(method_target("ods", ty, m).is_some(), "{ty}.{m}");
        }
        // a Frame takes no column math, and a column no Frame verb
        assert!(method_target("ods", "Frame", "sqrt").is_none());
        assert!(method_target("ods", "Series", "with").is_none());
        // ods.clip(s, lo: 0.0): hi left open
        assert_eq!(plan("ods.clip", 1, &["lo"]).unwrap(), vec![Slot::Arg(0), Slot::Arg(1)]);
        assert_eq!(returns("ods.col", None), Some("Series"));
    }
}

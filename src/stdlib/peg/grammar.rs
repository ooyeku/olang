//! A grammar as the matcher runs it: the rules `peg.ol` read and
//! analysed, flattened into one array of expressions with every name
//! resolved, every character test a bitmap, every ordered choice given
//! its first-character guards, every label its list fields, and every
//! precedence table its operators in the order they are tried.
//!
//! The olang side hands over the rules as it holds them (maps tagged by
//! `"t"`, tokens already written in place inside tokens), the rules that
//! memoise (in slot order) and the heads of left recursion. Nullability,
//! first sets and the label analysis are recomputed here by the same
//! definitions `peg.ol` uses, so the two engines agree by construction.

use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) type Id = u32;
pub(crate) const NONE: u32 = u32::MAX;

/// A set of characters: a bitmap for ASCII, ranges past it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CharSet {
    pub ascii: [u64; 2],
    /// Ranges consulted for a character of 128 or more.
    pub wide: Vec<(i32, i32)>,
    /// A character of 128 or more is in the set when it is *not* in
    /// `wide` (a negated class).
    pub wide_neg: bool,
}

impl CharSet {
    #[inline(always)]
    pub fn has(&self, c: i32) -> bool {
        if c < 0 {
            false
        } else if c < 128 {
            (self.ascii[(c >> 6) as usize] >> (c & 63)) & 1 == 1
        } else {
            let inside = self.wide.iter().any(|&(lo, hi)| c >= lo && c <= hi);
            inside != self.wide_neg
        }
    }

    fn of_class(ranges: &[(i32, i32)], neg: bool) -> CharSet {
        let mut ascii = [0u64; 2];
        for c in 0..128i32 {
            let inside = ranges.iter().any(|&(lo, hi)| c >= lo && c <= hi);
            if inside != neg {
                ascii[(c >> 6) as usize] |= 1 << (c & 63);
            }
        }
        CharSet {
            ascii,
            wide: ranges
                .iter()
                .copied()
                .filter(|&(_, hi)| hi >= 128)
                .collect(),
            wide_neg: neg,
        }
    }
}

/// The characters an expression can start with (`peg.ol`'s `fs_*`).
#[derive(Clone, Copy, PartialEq, Debug)]
struct First {
    a: [bool; 128],
    w: bool,
}

impl First {
    fn empty() -> First {
        First {
            a: [false; 128],
            w: false,
        }
    }
    fn all() -> First {
        First {
            a: [true; 128],
            w: true,
        }
    }
    fn union(mut self, o: &First) -> First {
        for i in 0..128 {
            self.a[i] |= o.a[i];
        }
        self.w |= o.w;
        self
    }
    fn ranges(ranges: &[(i32, i32)], neg: bool) -> First {
        let mut f = First {
            a: [neg; 128],
            w: neg,
        };
        for &(lo, hi) in ranges {
            let mut c = lo;
            while c <= hi && c < 128 {
                if c >= 0 {
                    f.a[c as usize] = !neg;
                }
                c += 1;
            }
            if hi >= 128 && !neg {
                f.w = true;
            }
        }
        f
    }
    /// The guard an ordered choice tests, or None where it lets every
    /// character through.
    fn guard(&self) -> Option<CharSet> {
        if self.w && self.a.iter().all(|b| *b) {
            return None;
        }
        let mut ascii = [0u64; 2];
        for c in 0..128usize {
            if self.a[c] {
                ascii[c >> 6] |= 1 << (c & 63);
            }
        }
        Some(CharSet {
            ascii,
            wide: if self.w {
                vec![(128, 1_114_111)]
            } else {
                vec![]
            },
            wide_neg: false,
        })
    }
}

/// An expression as read from the olang side.
#[derive(Clone, Debug)]
pub(crate) enum E {
    Lit {
        codes: Vec<i32>,
        s: String,
        ci: bool,
    },
    Class {
        ranges: Vec<(i32, i32)>,
        neg: bool,
        src: String,
    },
    Any,
    Ref(String),
    Seq(Vec<E>),
    Alt(Vec<E>),
    Star(Box<E>),
    Plus(Box<E>),
    Opt(Box<E>),
    And(Box<E>),
    Not(Box<E>),
    Label(String, Box<E>),
    Prec {
        atom: Box<E>,
        levels: Vec<(String, Vec<E>)>,
    },
}

impl E {
    fn parts(&self) -> Vec<&E> {
        match self {
            E::Seq(xs) | E::Alt(xs) => xs.iter().collect(),
            E::Star(e) | E::Plus(e) | E::Opt(e) | E::And(e) | E::Not(e) | E::Label(_, e) => {
                vec![e]
            }
            E::Prec { atom, levels } => {
                let mut v: Vec<&E> = vec![atom];
                for (_, ops) in levels {
                    v.extend(ops.iter());
                }
                v
            }
            _ => vec![],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fix {
    Prefix,
    Postfix,
    Left,
    Right,
}

impl Fix {
    pub fn word(self) -> &'static str {
        match self {
            Fix::Prefix => "prefix",
            Fix::Postfix => "postfix",
            Fix::Left | Fix::Right => "infix",
        }
    }
}

#[derive(Debug)]
pub(crate) struct PrecOp {
    pub fix: Fix,
    pub lev: u32,
    pub e: Id,
}

#[derive(Debug)]
pub(crate) struct Prec {
    pub atom: Id,
    /// Longest literal first, as `peg.ol`'s `prec_ops` sorts them.
    pub ops: Vec<PrecOp>,
    /// The kind of the nodes it makes: the rule it is written in.
    pub kind: u32,
}

#[derive(Debug)]
pub(crate) enum Expr {
    /// `""`: matches nothing, skips nothing.
    Empty,
    Lit {
        codes: Box<[i32]>,
        /// Lower-cased letters, for a literal that ignores case.
        ci: bool,
        desc: u32,
    },
    Class {
        set: CharSet,
        desc: u32,
    },
    Any,
    Ref(Id),
    Seq(Box<[Id]>),
    /// `guards`: present when no alternative can match nothing — each
    /// alternative's first-character test (None for a lone class or
    /// literal, or a set that lets everything through).
    Alt {
        items: Box<[Id]>,
        guards: Option<Box<[Option<CharSet>]>>,
    },
    Star(Id),
    /// `class*` in a token or `skip`, where nothing is skipped between
    /// repetitions: one loop over the characters.
    StarClass {
        set: CharSet,
        desc: u32,
    },
    /// `(class / rest...)*` in a token or `skip`: run over the class's
    /// characters in place and try the rest only where it stops (the
    /// olang engine's "lead" loop). A noting parse runs `general`, the
    /// plain repetition, so its failures are noted as they always were.
    StarLead {
        lead: CharSet,
        rest: Id,
        general: Id,
    },
    Plus(Id),
    Opt(Id),
    And(Id),
    Not {
        e: Id,
        /// `!.`: failing, it is the end of input that was expected.
        any: bool,
    },
    Label {
        kind: u32,
        e: Id,
        /// The label names the nodes of a rule (`key:Key`, `xs:x*`)
        /// rather than making a node of its own.
        wraps: bool,
        /// The labels of its own node that hold lists.
        lists: Box<[u32]>,
    },
    Prec(Id),
}

#[derive(Debug)]
pub(crate) struct Rule {
    pub name: String,
    pub kind: u32,
    pub token: bool,
    pub silent: bool,
    pub is_skip: bool,
    /// Its words in an error: its own (`value "a value"`) or its name.
    pub desc: u32,
    pub has_desc: bool,
    pub body: Id,
    /// Its memo slot pair, when it memoises.
    pub slot: Option<u32>,
    pub lr_head: bool,
    /// The body is a precedence table and the rule is not silent: the
    /// table's node is the rule's.
    pub prec: Option<Id>,
    /// Some(list labels) when the rule's node has labels.
    pub labels: Option<Box<[u32]>>,
}

#[derive(Debug)]
pub(crate) struct Grammar {
    pub exprs: Vec<Expr>,
    pub rules: Vec<Rule>,
    pub precs: Vec<Prec>,
    /// Kind names: the rules', then the labels'.
    pub kinds: Vec<Arc<String>>,
    pub descs: Vec<String>,
    pub end_desc: u32,
    pub any_desc: u32,
    pub skip: Option<Id>,
    /// `skip = class+` (or `*`): skipped by one loop.
    pub skip_class: Option<CharSet>,
    /// Each kind's action, when it has one.
    pub actions: Vec<Option<Value>>,
    pub nslots: usize,
}

// ── reading the olang side's data ──────────────────────────────────────

fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Map(m) => m.get(key),
        Value::Struct { fields, .. } => fields.get(key),
        _ => None,
    }
    .filter(|x| !matches!(x, Value::Unit))
}

fn bad(what: &str) -> String {
    format!("peg: malformed grammar data ({what})")
}

fn string(v: &Value, key: &str) -> Result<String, String> {
    match field(v, key) {
        Some(Value::String(s)) => Ok(s.as_str().to_string()),
        _ => Err(bad(key)),
    }
}

fn boolean(v: &Value, key: &str) -> bool {
    matches!(field(v, key), Some(Value::Boolean(true)))
}

fn list<'a>(v: &'a Value, key: &str) -> Result<&'a [Value], String> {
    match field(v, key) {
        Some(Value::List(xs)) => Ok(xs.as_slice()),
        None => Ok(&[]),
        _ => Err(bad(key)),
    }
}

fn int(v: &Value) -> Result<i32, String> {
    match v {
        Value::Integer(n) => Ok(*n as i32),
        _ => Err(bad("a code")),
    }
}

pub(crate) fn read_expr(v: &Value) -> Result<E, String> {
    let t = string(v, "t")?;
    let sub = |key: &str| -> Result<Box<E>, String> {
        Ok(Box::new(read_expr(field(v, key).ok_or_else(|| bad(key))?)?))
    };
    let items = || -> Result<Vec<E>, String> { list(v, "items")?.iter().map(read_expr).collect() };
    Ok(match t.as_str() {
        "lit" => E::Lit {
            codes: list(v, "codes")?
                .iter()
                .map(int)
                .collect::<Result<_, _>>()?,
            s: string(v, "s")?,
            ci: boolean(v, "ci"),
        },
        "class" => {
            let mut ranges = Vec::new();
            for r in list(v, "ranges")? {
                match r {
                    Value::Tuple(xs) | Value::List(xs) if xs.len() == 2 => {
                        ranges.push((int(&xs[0])?, int(&xs[1])?))
                    }
                    _ => return Err(bad("a range")),
                }
            }
            E::Class {
                ranges,
                neg: boolean(v, "neg"),
                src: string(v, "src")?,
            }
        }
        "any" => E::Any,
        "ref" => E::Ref(string(v, "name")?),
        "seq" => E::Seq(items()?),
        "alt" => E::Alt(items()?),
        "star" => E::Star(sub("e")?),
        "plus" => E::Plus(sub("e")?),
        "opt" => E::Opt(sub("e")?),
        "and" => E::And(sub("e")?),
        "not" => E::Not(sub("e")?),
        "label" => E::Label(string(v, "name")?, sub("e")?),
        "prec" => {
            let mut levels = Vec::new();
            for l in list(v, "levels")? {
                let ops = list(l, "ops")?
                    .iter()
                    .map(read_expr)
                    .collect::<Result<_, _>>()?;
                levels.push((string(l, "assoc")?, ops));
            }
            E::Prec {
                atom: sub("atom")?,
                levels,
            }
        }
        other => return Err(bad(&format!("expression `{other}`"))),
    })
}

/// A rule as read: name, words, body, token, silent.
pub(crate) struct RuleIn {
    pub name: String,
    pub desc: Option<String>,
    pub e: E,
    pub token: bool,
    pub silent: bool,
}

pub(crate) fn read_rule(v: &Value) -> Result<RuleIn, String> {
    Ok(RuleIn {
        name: string(v, "name")?,
        desc: match field(v, "desc") {
            Some(Value::String(s)) => Some(s.as_str().to_string()),
            _ => None,
        },
        e: read_expr(field(v, "e").ok_or_else(|| bad("e"))?)?,
        token: boolean(v, "token"),
        silent: boolean(v, "silent"),
    })
}

// ── the analyses, as peg.ol defines them ───────────────────────────────

struct Analysis<'a> {
    rules: &'a [RuleIn],
    index: HashMap<&'a str, usize>,
    nul: Vec<bool>,
    fst: Vec<First>,
}

impl<'a> Analysis<'a> {
    fn new(rules: &'a [RuleIn]) -> Analysis<'a> {
        let index = rules
            .iter()
            .enumerate()
            .map(|(i, r)| (r.name.as_str(), i))
            .collect();
        let mut a = Analysis {
            rules,
            index,
            nul: vec![false; rules.len()],
            fst: vec![First::empty(); rules.len()],
        };
        let mut changed = true;
        while changed {
            changed = false;
            for (i, r) in rules.iter().enumerate() {
                if !a.nul[i] && a.nullable(&r.e) {
                    a.nul[i] = true;
                    changed = true;
                }
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            for (i, r) in rules.iter().enumerate() {
                let f = a.first(&r.e);
                if f != a.fst[i] {
                    a.fst[i] = f;
                    changed = true;
                }
            }
        }
        a
    }

    fn rule(&self, name: &str) -> Option<&'a RuleIn> {
        self.index.get(name).map(|&i| &self.rules[i])
    }

    fn nullable(&self, e: &E) -> bool {
        match e {
            E::Lit { codes, .. } => codes.is_empty(),
            E::Class { .. } | E::Any => false,
            E::Ref(n) => self.index.get(n.as_str()).is_some_and(|&i| self.nul[i]),
            E::Seq(xs) => xs.iter().all(|x| self.nullable(x)),
            E::Alt(xs) => xs.iter().any(|x| self.nullable(x)),
            E::Star(_) | E::Opt(_) | E::And(_) | E::Not(_) => true,
            E::Plus(x) | E::Label(_, x) => self.nullable(x),
            E::Prec { atom, .. } => self.nullable(atom),
        }
    }

    fn first(&self, e: &E) -> First {
        match e {
            E::Lit { codes, ci, .. } => {
                if codes.is_empty() {
                    First::empty()
                } else {
                    let c = codes[0];
                    if *ci && is_letter(c) {
                        First::ranges(&[(lower(c), lower(c)), (upper(c), upper(c))], false)
                    } else {
                        First::ranges(&[(c, c)], false)
                    }
                }
            }
            E::Class { ranges, neg, .. } => First::ranges(ranges, *neg),
            E::Any => First::all(),
            E::Ref(n) => self
                .index
                .get(n.as_str())
                .map(|&i| self.fst[i])
                .unwrap_or_else(First::empty),
            E::Seq(xs) => {
                let mut acc = First::empty();
                for x in xs {
                    acc = acc.union(&self.first(x));
                    if !self.nullable(x) {
                        break;
                    }
                }
                acc
            }
            E::Alt(xs) => xs
                .iter()
                .fold(First::empty(), |acc, x| acc.union(&self.first(x))),
            E::Star(x) | E::Plus(x) | E::Opt(x) | E::Label(_, x) => self.first(x),
            E::Prec { atom, levels } => {
                let mut acc = self.first(atom);
                for (assoc, ops) in levels {
                    if assoc == "prefix" {
                        for op in ops {
                            acc = acc.union(&self.first(op));
                        }
                    }
                }
                acc
            }
            E::And(_) | E::Not(_) => First::empty(),
        }
    }

    fn label_wraps(&self, e: &E) -> bool {
        let inner = match e {
            E::Opt(x) | E::Star(x) | E::Plus(x) => x,
            other => other,
        };
        matches!(inner, E::Ref(n) if self.rule(n).is_some())
    }

    fn label_scan(&self, e: &E, rep: bool, seen: &mut Vec<String>, out: &mut Vec<(String, bool)>) {
        match e {
            E::Label(name, inner) => {
                let many =
                    rep || (matches!(**inner, E::Star(_) | E::Plus(_)) && self.label_wraps(inner));
                out.push((name.clone(), many));
            }
            E::Star(x) | E::Plus(x) => self.label_scan(x, true, seen, out),
            E::Ref(n) => {
                if let Some(r) = self.rule(n)
                    && r.silent
                    && !r.token
                    && !seen.contains(n)
                {
                    seen.push(n.clone());
                    self.label_scan(&r.e, rep, seen, out);
                    seen.pop();
                }
            }
            E::Prec { .. } | E::And(_) | E::Not(_) => {}
            other => {
                for x in other.parts() {
                    self.label_scan(x, rep, seen, out);
                }
            }
        }
    }

    fn found_labels(&self, e: &E) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        self.label_scan(e, false, &mut Vec::new(), &mut out);
        out
    }

    /// The labels of a scope that hold lists: repeated, or written twice.
    fn list_labels(&self, e: &E) -> Vec<String> {
        let found = self.found_labels(e);
        let mut names: Vec<String> = Vec::new();
        for (n, _) in &found {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
        names
            .into_iter()
            .filter(|n| {
                let hits: Vec<_> = found.iter().filter(|x| &x.0 == n).collect();
                hits.len() > 1 || hits.iter().any(|x| x.1)
            })
            .collect()
    }
}

fn is_letter(c: i32) -> bool {
    (65..=90).contains(&c) || (97..=122).contains(&c)
}
pub(crate) fn lower(c: i32) -> i32 {
    if (65..=90).contains(&c) { c + 32 } else { c }
}
fn upper(c: i32) -> i32 {
    if (97..=122).contains(&c) { c - 32 } else { c }
}

/// A string as a grammar literal would write it, for messages (`peg.ol`'s
/// `quote`).
pub(crate) fn quote(t: &str) -> String {
    let mut out = String::with_capacity(t.len() + 2);
    out.push('"');
    for ch in t.chars() {
        let c = ch as u32;
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ if c < 32 => out.push_str(&format!("\\u{{{:x}}}", c)),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

// ── building ───────────────────────────────────────────────────────────

struct Builder<'a> {
    an: &'a Analysis<'a>,
    g: Grammar,
    kind_ix: HashMap<String, u32>,
    desc_ix: HashMap<String, u32>,
    /// While a rule's body is flattened: the rule (for a table's kind)
    /// and whether it is a token or `skip` (where `class*` runs inline).
    rule_kind: u32,
    in_token: bool,
}

impl<'a> Builder<'a> {
    fn kind(&mut self, name: &str) -> u32 {
        if let Some(&k) = self.kind_ix.get(name) {
            return k;
        }
        let k = self.g.kinds.len() as u32;
        self.g.kinds.push(Arc::new(name.to_string()));
        self.kind_ix.insert(name.to_string(), k);
        k
    }

    fn desc(&mut self, d: &str) -> u32 {
        if let Some(&k) = self.desc_ix.get(d) {
            return k;
        }
        let k = self.g.descs.len() as u32;
        self.g.descs.push(d.to_string());
        self.desc_ix.insert(d.to_string(), k);
        k
    }

    fn push(&mut self, e: Expr) -> Id {
        self.g.exprs.push(e);
        (self.g.exprs.len() - 1) as Id
    }

    fn flatten(&mut self, e: &E) -> Result<Id, String> {
        let ex = match e {
            E::Lit { codes, s, ci } => {
                if codes.is_empty() {
                    Expr::Empty
                } else {
                    let desc = self.desc(&quote(s));
                    Expr::Lit {
                        codes: codes
                            .iter()
                            .map(|&c| if *ci && is_letter(c) { lower(c) } else { c })
                            .collect(),
                        ci: *ci,
                        desc,
                    }
                }
            }
            E::Class { ranges, neg, src } => Expr::Class {
                set: CharSet::of_class(ranges, *neg),
                desc: self.desc(src),
            },
            E::Any => Expr::Any,
            E::Ref(n) => Expr::Ref(
                *self
                    .an
                    .index
                    .get(n.as_str())
                    .ok_or_else(|| format!("peg: no rule is named `{n}`"))? as Id,
            ),
            E::Seq(xs) => {
                let ids = xs
                    .iter()
                    .map(|x| self.flatten(x))
                    .collect::<Result<Vec<_>, _>>()?;
                Expr::Seq(ids.into())
            }
            E::Alt(xs) => {
                let ids = xs
                    .iter()
                    .map(|x| self.flatten(x))
                    .collect::<Result<Vec<_>, _>>()?;
                let guards = if xs.iter().any(|x| self.an.nullable(x)) {
                    None
                } else {
                    Some(
                        xs.iter()
                            .map(|x| match x {
                                E::Class { .. } | E::Lit { .. } => None,
                                other => self.an.first(other).guard(),
                            })
                            .collect::<Vec<_>>()
                            .into(),
                    )
                };
                Expr::Alt {
                    items: ids.into(),
                    guards,
                }
            }
            E::Star(x) => match &**x {
                E::Class { ranges, neg, src } if self.in_token => Expr::StarClass {
                    set: CharSet::of_class(ranges, *neg),
                    desc: self.desc(src),
                },
                E::Alt(items)
                    if self.in_token && items.len() > 1 && matches!(items[0], E::Class { .. }) =>
                {
                    let E::Class { ranges, neg, .. } = &items[0] else {
                        unreachable!("matched above")
                    };
                    let lead = CharSet::of_class(ranges, *neg);
                    let rest = if items.len() == 2 {
                        self.flatten(&items[1])?
                    } else {
                        self.flatten(&E::Alt(items[1..].to_vec()))?
                    };
                    let general = self.flatten(x)?;
                    Expr::StarLead {
                        lead,
                        rest,
                        general,
                    }
                }
                _ => Expr::Star(self.flatten(x)?),
            },
            E::Plus(x) => Expr::Plus(self.flatten(x)?),
            E::Opt(x) => Expr::Opt(self.flatten(x)?),
            E::And(x) => Expr::And(self.flatten(x)?),
            E::Not(x) => Expr::Not {
                e: self.flatten(x)?,
                any: matches!(**x, E::Any),
            },
            E::Label(name, x) => {
                let wraps = self.an.label_wraps(x);
                let lists: Vec<u32> = self
                    .an
                    .list_labels(x)
                    .iter()
                    .map(|n| self.kind(n))
                    .collect();
                let kind = self.kind(name);
                Expr::Label {
                    kind,
                    e: self.flatten(x)?,
                    wraps,
                    lists: lists.into(),
                }
            }
            E::Prec { atom, levels } => {
                let atom = self.flatten(atom)?;
                let mut ops: Vec<(PrecOp, i64)> = Vec::new();
                for (lev, (assoc, xs)) in levels.iter().enumerate() {
                    let fix = match assoc.as_str() {
                        "left" => Fix::Left,
                        "right" => Fix::Right,
                        "prefix" => Fix::Prefix,
                        "postfix" => Fix::Postfix,
                        other => return Err(bad(&format!("level `{other}`"))),
                    };
                    for x in xs {
                        let key = match x {
                            E::Lit { codes, .. } => -(codes.len() as i64),
                            _ => 0,
                        };
                        let e = self.flatten(x)?;
                        ops.push((
                            PrecOp {
                                fix,
                                lev: lev as u32,
                                e,
                            },
                            key,
                        ));
                    }
                }
                // stable: equal keys keep their order
                ops.sort_by_key(|o| o.1);
                self.g.precs.push(Prec {
                    atom,
                    ops: ops.into_iter().map(|o| o.0).collect(),
                    kind: self.rule_kind,
                });
                Expr::Prec((self.g.precs.len() - 1) as Id)
            }
        };
        Ok(self.push(ex))
    }
}

/// The grammar to run: `rules` as `peg.ol` holds them after analysis,
/// `memo` the rules that memoise (a rule's slot is its place here),
/// `heads` the heads of left recursion, `actions` by rule or label name.
pub(crate) fn build(
    rules: Vec<RuleIn>,
    memo: &[String],
    heads: &[String],
    actions: &[(String, Value)],
) -> Result<Grammar, String> {
    let an = Analysis::new(&rules);
    let mut b = Builder {
        an: &an,
        g: Grammar {
            exprs: Vec::new(),
            rules: Vec::new(),
            precs: Vec::new(),
            kinds: Vec::new(),
            descs: Vec::new(),
            end_desc: 0,
            any_desc: 0,
            skip: None,
            skip_class: None,
            actions: Vec::new(),
            nslots: memo.len() * 2,
        },
        kind_ix: HashMap::new(),
        desc_ix: HashMap::new(),
        rule_kind: NONE,
        in_token: false,
    };
    // The words of "end of input" first, as peg.ol's table has them.
    b.g.end_desc = b.desc("end of input");
    b.g.any_desc = b.desc("any character");
    for r in &rules {
        b.kind(&r.name);
    }
    for (i, r) in rules.iter().enumerate() {
        b.rule_kind = i as u32;
        b.in_token = r.token || r.name == "skip";
        let body = b.flatten(&r.e)?;
        let desc_text = r.desc.clone().unwrap_or_else(|| r.name.clone());
        let desc = b.desc(&desc_text);
        let prec = match (&b.g.exprs[body as usize], r.silent) {
            (Expr::Prec(p), false) => Some(*p),
            _ => None,
        };
        let labels = if !r.silent && !an.found_labels(&r.e).is_empty() {
            let ls: Vec<u32> = an.list_labels(&r.e).iter().map(|n| b.kind(n)).collect();
            Some(ls.into_boxed_slice())
        } else {
            None
        };
        b.g.rules.push(Rule {
            name: r.name.clone(),
            kind: i as u32,
            token: r.token,
            silent: r.silent,
            is_skip: r.name == "skip",
            desc,
            has_desc: r.desc.is_some(),
            body,
            slot: memo.iter().position(|m| m == &r.name).map(|s| s as u32),
            lr_head: heads.contains(&r.name),
            prec,
            labels,
        });
    }
    if let Some(i) = rules.iter().position(|r| r.name == "skip") {
        b.g.skip = Some(i as Id);
        if let E::Plus(x) | E::Star(x) = &rules[i].e
            && let E::Class { ranges, neg, .. } = &**x
        {
            b.g.skip_class = Some(CharSet::of_class(ranges, *neg));
        }
    }
    let mut acts = vec![None; b.g.kinds.len()];
    for (name, f) in actions {
        if let Some(&k) = b.kind_ix.get(name) {
            acts[k as usize] = Some(f.clone());
        }
    }
    b.g.actions = acts;
    Ok(b.g)
}

// ── a listing, for `g.source` ──────────────────────────────────────────

impl Grammar {
    fn show(&self, e: Id, out: &mut String) {
        let wrap = |g: &Grammar, x: Id, out: &mut String| {
            let compound = matches!(
                g.exprs[x as usize],
                Expr::Seq(_) | Expr::Alt { .. } | Expr::Label { .. }
            );
            if compound {
                out.push('(');
            }
            g.show(x, out);
            if compound {
                out.push(')');
            }
        };
        match &self.exprs[e as usize] {
            Expr::Empty => out.push_str("\"\""),
            Expr::Lit { desc, ci, .. } => {
                out.push_str(&self.descs[*desc as usize]);
                if *ci {
                    out.push('i');
                }
            }
            Expr::Class { desc, .. } => out.push_str(&self.descs[*desc as usize]),
            Expr::StarClass { desc, .. } => {
                out.push_str(&self.descs[*desc as usize]);
                out.push_str("*  {run}");
            }
            Expr::StarLead { general, .. } => {
                out.push('(');
                self.show(*general, out);
                out.push_str(")*  {lead run}");
            }
            Expr::Any => out.push('.'),
            Expr::Ref(r) => out.push_str(&self.rules[*r as usize].name),
            Expr::Seq(xs) => {
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    if matches!(self.exprs[*x as usize], Expr::Alt { .. }) {
                        out.push('(');
                        self.show(*x, out);
                        out.push(')');
                    } else {
                        self.show(*x, out);
                    }
                }
            }
            Expr::Alt { items, guards } => {
                for (i, x) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(" / ");
                    }
                    self.show(*x, out);
                }
                if guards.is_some() {
                    out.push_str("  {dispatch}");
                }
            }
            Expr::Star(x) => {
                wrap(self, *x, out);
                out.push('*');
            }
            Expr::Plus(x) => {
                wrap(self, *x, out);
                out.push('+');
            }
            Expr::Opt(x) => {
                wrap(self, *x, out);
                out.push('?');
            }
            Expr::And(x) => {
                out.push('&');
                wrap(self, *x, out);
            }
            Expr::Not { e, .. } => {
                out.push('!');
                wrap(self, *e, out);
            }
            Expr::Label { kind, e, .. } => {
                out.push_str(&self.kinds[*kind as usize]);
                out.push(':');
                wrap(self, *e, out);
            }
            Expr::Prec(p) => {
                let pr = &self.precs[*p as usize];
                out.push_str("precedence(");
                self.show(pr.atom, out);
                out.push_str(") {");
                for op in &pr.ops {
                    out.push_str(&format!(" {:?}@{} ", op.fix, op.lev).to_lowercase());
                    self.show(op.e, out);
                    out.push(';');
                }
                out.push_str(" }");
            }
        }
    }

    /// The program the matcher runs, one rule a line.
    pub fn listing(&self) -> String {
        let mut out = String::from("// peg: the grammar as the native matcher runs it\n");
        for r in &self.rules {
            let mut notes = Vec::new();
            if r.token {
                notes.push("token".to_string());
            }
            if r.silent {
                notes.push("silent".to_string());
            }
            if let Some(s) = r.slot {
                notes.push(if r.lr_head {
                    format!("left-recursive head, seed slot {s}")
                } else {
                    format!("memo slot {s}")
                });
            }
            if r.has_desc {
                notes.push(format!("said as {}", quote(&self.descs[r.desc as usize])));
            }
            out.push_str(&r.name);
            out.push_str(" = ");
            self.show(r.body, &mut out);
            if !notes.is_empty() {
                out.push_str("    // ");
                out.push_str(&notes.join(", "));
            }
            out.push('\n');
        }
        out
    }
}

//! The flat layout's two per-node steps, for Loom (loom/lib/layout.ol):
//! a view read into the layout core's arrays (`gui.flatten`), and the
//! core's boxes made into placements (`gui.flat_emit`, `gui.flat_join`).
//!
//! The rules stay Loom's: which roles are layers, fill, shrink, or read
//! the context; each leaf's natural size; the theme's tokens and looks —
//! all arrive in `spec`, a map Loom builds once a theme. The core's
//! passes (loom/lib/flex.ol) run between the two steps, in olang, on the
//! native tier. What is here is the work per node that olang's bytecode
//! VM was slow at: reading props, measuring text, building maps.
//!
//! The arrays are flex.ol's: `I` holds FI ints a node, `F` FF floats, in
//! tree order (a node before its children; a subtree is a run of
//! indices); the children of node i are `kids[I[i*FI+FIRST] ..][..COUNT]`.

use super::scene::Style;
use super::text::{self, TextSystem};
use super::values::*;
use crate::ast::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub const FI: usize = 14;
pub const FF: usize = 22;

const I_KIND: usize = 0;
const I_FIRST: usize = 2;
const I_COUNT: usize = 3;
const I_PARENT: usize = 12;
const F_NATH: usize = 17;
const PAD: usize = 80;

// ── reading the spec ─────────────────────────────────────────────────

struct Spec<'a> {
    theme: &'a crate::ast::ValueMap,
    lifted: HashSet<&'a str>,
    fills: HashSet<&'a str>,
    ctx: HashSet<&'a str>,
    shrinks: HashSet<&'a str>,
    placed: HashSet<&'a str>,
    leaves: &'a crate::ast::ValueMap,
    heading: Vec<Value>,
    label: &'a Value,
    looks: &'a crate::ast::ValueMap,
    /// The font style of a text that sets none of its own, by role
    /// (heading level): most texts share one, built once a call.
    plain: RefCell<HashMap<usize, Value>>,
    /// Styles parsed for measuring, by the address of their map (each
    /// entry keeps its map alive, so an address is never reused).
    parsed: RefCell<HashMap<usize, (Value, Style)>>,
}

fn strs(v: Option<&Value>) -> HashSet<&str> {
    let mut out = HashSet::new();
    if let Some(Value::List(l)) = v {
        for x in l.iter() {
            if let Value::String(s) = x {
                out.insert(s.as_str());
            }
        }
    }
    out
}

fn a_map<'a>(v: Option<&'a Value>, what: &str) -> Res<&'a crate::ast::ValueMap> {
    match v {
        Some(Value::Map(m)) => Ok(m),
        _ => Err(format!("gui.flatten: the spec's \"{what}\" must be a map")),
    }
}

impl<'a> Spec<'a> {
    fn read(v: &'a Value) -> Res<Spec<'a>> {
        let f = fields(v).ok_or("gui.flatten: the spec must be a map")?;
        if let Some(Value::Tuple(t)) = f.get("fields") {
            let want = (FI as i64, FF as i64);
            if t.len() != 2 || t[0] != Value::Integer(want.0) || t[1] != Value::Integer(want.1) {
                return Err(format!(
                    "gui.flatten: the core's arrays are {}×{} a node here, and {v_} in the spec",
                    FI,
                    FF,
                    v_ = Value::Tuple(t.clone())
                ));
            }
        }
        let heading = match f.get("heading") {
            Some(Value::List(l)) if l.len() == 4 => l.as_ref().clone(),
            _ => return Err("gui.flatten: the spec's \"heading\" must be four sizes".into()),
        };
        Ok(Spec {
            theme: a_map(f.get("theme"), "theme")?,
            lifted: strs(f.get("lifted")),
            fills: strs(f.get("fills")),
            ctx: strs(f.get("ctx")),
            shrinks: strs(f.get("shrinks")),
            placed: strs(f.get("placed")),
            leaves: a_map(f.get("leaves"), "leaves")?,
            heading,
            label: f.get("label").unwrap_or(&UNIT.0),
            looks: a_map(f.get("looks"), "looks")?,
            plain: RefCell::new(HashMap::new()),
            parsed: RefCell::new(HashMap::new()),
        })
    }

    fn tok_of(&self, key: &str) -> Value {
        self.theme.get(key).cloned().unwrap_or(Value::Unit)
    }

    fn look(&self, key: &str) -> Value {
        self.looks.get(key).cloned().unwrap_or(Value::Unit)
    }
}

// ── olang's small helpers, as Loom uses them ─────────────────────────

const EMPTY: &[Value] = &[];

// `()`, to answer by reference where olang answers `()` for a missing
// key. `Value` has drop glue, so `&Value::Unit` is not promoted to a
// static; a unit holds nothing shared, so this one is safe to share.
struct UnitValue(Value);
unsafe impl Sync for UnitValue {}
static UNIT: UnitValue = UnitValue(Value::Unit);

fn props_of(node: &Value) -> Option<&crate::ast::ValueMap> {
    match node {
        Value::Map(m) => match m.get("props") {
            Some(Value::Map(p)) => Some(p),
            _ => None,
        },
        _ => None,
    }
}

/// `map_get`: the value, or `()`.
fn mget<'a>(m: Option<&'a crate::ast::ValueMap>, k: &str) -> &'a Value {
    m.and_then(|m| m.get(k)).unwrap_or(&UNIT.0)
}

/// `map_get_or`.
fn mget_or<'a>(m: Option<&'a crate::ast::ValueMap>, k: &str, d: &'a Value) -> &'a Value {
    match mget(m, k) {
        Value::Unit => d,
        v => v,
    }
}

fn role_of(node: &Value) -> &str {
    match node {
        Value::Map(m) => match m.get("role") {
            Some(Value::String(s)) => s.as_str(),
            _ => "",
        },
        _ => "",
    }
}

fn children_of(node: &Value) -> &[Value] {
    match node {
        Value::Map(m) => match m.get("children") {
            Some(Value::List(l)) => l,
            _ => EMPTY,
        },
        _ => EMPTY,
    }
}

/// `ly_num`: an Int as a Float; anything else that is not a number, 0.
fn fnum(v: &Value) -> f64 {
    match v {
        Value::Integer(i) => *i as f64,
        Value::Float(f) => *f,
        _ => 0.0,
    }
}

fn is_true(v: &Value) -> bool {
    matches!(v, Value::Boolean(true))
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.as_str().to_string(),
        Value::Unit => String::new(),
        other => other.to_string(),
    }
}

fn st(text: &str) -> Value {
    Value::String(Arc::new(text.to_string()))
}

fn vmap(m: crate::ast::ValueMap) -> Value {
    Value::Map(Arc::new(m))
}

fn hm(fields: Vec<(&str, Value)>) -> crate::ast::ValueMap {
    let mut m = crate::ast::ValueMap::with_capacity_and_hasher(fields.len(), Default::default());
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    m
}

fn tuple(xs: Vec<Value>) -> Value {
    Value::Tuple(Arc::new(xs))
}

fn list(xs: Vec<Value>) -> Value {
    Value::List(Arc::new(xs))
}

/// `show_key` for `ly_gkey`: a string as itself, anything else shown.
fn gkey_of(node: &Value, path: &str) -> String {
    let k = match node {
        Value::Map(m) => m.get("key").unwrap_or(&UNIT.0),
        _ => &UNIT.0,
    };
    match k {
        Value::Unit => format!("~{path}"),
        Value::String(s) => s.as_str().to_string(),
        other => other.to_string(),
    }
}

// The size props as the core's (kind, value): 0 auto, 1 fixed, 2 fill
// (weight), 3 fit, 4 percent (`ly_fspec`).
fn fspec(v: &Value) -> (i64, f64) {
    match v {
        Value::Integer(_) | Value::Float(_) => (1, fnum(v).max(0.0)),
        Value::String(s) => {
            let s = s.as_str();
            if let Some(n) = s.strip_suffix('%') {
                (4, n.parse::<i64>().unwrap_or(0) as f64)
            } else if s == "fill" {
                (2, 1.0)
            } else if let Some(n) = s.strip_prefix("fill:") {
                (2, (n.parse::<i64>().unwrap_or(1) as f64).max(1.0))
            } else if s == "fit" {
                (3, 0.0)
            } else {
                (0, 0.0)
            }
        }
        _ => (0, 0.0),
    }
}

fn falign(v: &Value) -> i64 {
    match v {
        Value::String(s) => match s.as_str() {
            "start" => 1,
            "center" => 2,
            "end" => 3,
            _ => 0,
        },
        _ => 0,
    }
}

fn fjustify(v: &Value) -> i64 {
    match v {
        Value::String(s) => match s.as_str() {
            "center" => 1,
            "end" => 2,
            "between" => 3,
            _ => 0,
        },
        _ => 0,
    }
}

fn pad4(p: &Value) -> (f64, f64, f64, f64) {
    let items: &[Value] = match p {
        Value::Integer(_) | Value::Float(_) => {
            let n = fnum(p);
            return (n, n, n, n);
        }
        Value::List(l) => l,
        Value::Tuple(t) => t,
        _ => return (0.0, 0.0, 0.0, 0.0),
    };
    let g = |i: usize| items.get(i).map(fnum).unwrap_or(0.0);
    match items.len() {
        0 => (0.0, 0.0, 0.0, 0.0),
        1 => (g(0), g(0), g(0), g(0)),
        2 => (g(0), g(1), g(0), g(1)),
        3 => (g(0), g(1), g(2), g(1)),
        _ => (g(0), g(1), g(2), g(3)),
    }
}

fn border_of(p: Option<&crate::ast::ValueMap>) -> f64 {
    match mget(p, "style") {
        Value::Map(st) => match st.get("border") {
            None | Some(Value::Unit) => 0.0,
            Some(_) => match st.get("border_width") {
                None | Some(Value::Unit) => 1.0,
                Some(v) => fnum(v),
            },
        },
        _ => 0.0,
    }
}

// ── styles (loom/lib/theme.ol: tok, font_style, with_style) ──────────

fn tok(spec: &Spec, v: &Value) -> Value {
    match v {
        Value::String(s) => match spec.theme.get(s.as_str()) {
            None | Some(Value::Unit) => v.clone(),
            Some(c) => c.clone(),
        },
        _ => v.clone(),
    }
}

fn font_style(spec: &Spec, p: Option<&crate::ast::ValueMap>, role: &str) -> crate::ast::ValueMap {
    let mut s = if role == "heading" {
        let at = match mget(p, "level") {
            Value::Integer(1) => 0,
            Value::Integer(2) => 1,
            Value::Integer(3) => 2,
            _ => 3,
        };
        hm(vec![
            ("size", spec.heading[at].clone()),
            ("weight", Value::Integer(650)),
            ("color", spec.tok_of("fg")),
        ])
    } else {
        hm(vec![
            ("size", spec.tok_of("body")),
            ("color", spec.tok_of("fg")),
        ])
    };
    if is_true(mget(p, "muted")) {
        s.insert("color".into(), spec.tok_of("muted"));
    }
    for k in ["size", "weight", "font", "italic", "line_height"] {
        let v = mget(p, k);
        if *v != Value::Unit {
            s.insert(k.into(), v.clone());
        }
    }
    let c = mget(p, "color");
    if *c != Value::Unit {
        s.insert("color".into(), tok(spec, c));
    }
    if is_true(mget(p, "wrap")) {
        s.insert("wrap".into(), Value::Boolean(true));
    }
    let a = mget(p, "align");
    if *a != Value::Unit && role != "group" {
        s.insert("align".into(), a.clone());
    }
    s
}

// The props `font_style` reads beyond `level`.
const FONT_PROPS: &[&str] = &[
    "muted",
    "size",
    "weight",
    "font",
    "italic",
    "line_height",
    "color",
    "wrap",
    "align",
];

/// `font_style` as a value, shared among the texts that set none of
/// their own.
fn font_style_v(spec: &Spec, p: Option<&crate::ast::ValueMap>, role: &str) -> Value {
    if FONT_PROPS.iter().any(|k| *mget(p, k) != Value::Unit) {
        return vmap(font_style(spec, p, role));
    }
    let slot = if role == "heading" {
        match mget(p, "level") {
            Value::Integer(l @ 1..=3) => *l as usize,
            _ => 4,
        }
    } else {
        0
    };
    spec.plain
        .borrow_mut()
        .entry(slot)
        .or_insert_with(|| vmap(font_style(spec, p, role)))
        .clone()
}

fn with_style(spec: &Spec, look: Value, style: &Value) -> Value {
    match style {
        Value::Map(m) => {
            let mut s = match &look {
                Value::Map(l) => (**l).clone(),
                _ => crate::ast::ValueMap::default(),
            };
            for (k, v) in m.iter() {
                let v2 = if matches!(v, Value::Map(_)) {
                    with_style(spec, vmap(crate::ast::ValueMap::default()), v)
                } else {
                    tok(spec, v)
                };
                s.insert(k.clone(), v2);
            }
            vmap(s)
        }
        // `with_style` answers the look for `()`; any other style is
        // iterated by olang's `entries` and would refuse there
        _ => look,
    }
}

// ── measuring (as gui.measure measures) ──────────────────────────────

fn measure(
    spec: &Spec,
    ts: &mut TextSystem,
    txt: &str,
    style: &Value,
    width: f64,
) -> Res<(f64, f64)> {
    let parse = || -> Res<Style> {
        let mut s = Style::default();
        s.apply(style, "gui.flatten")?;
        Ok(s)
    };
    let fresh;
    let mut parsed = spec.parsed.borrow_mut();
    let s: &Style = match style {
        Value::Map(m) => {
            let at = Arc::as_ptr(m) as usize;
            if let std::collections::hash_map::Entry::Vacant(e) = parsed.entry(at) {
                e.insert((style.clone(), parse()?));
            }
            &parsed[&at].1
        }
        _ => {
            fresh = parse()?;
            &fresh
        }
    };
    let max_w = if s.wrap { Some(width as f32) } else { None };
    let shaped = ts.shape(txt, &s.font, s.color, max_w, text::Align::Start, 1.0);
    let (w, h) = shaped.logical_size();
    Ok((w as f64, h as f64))
}

fn label_size(spec: &Spec, ts: &mut TextSystem, label: &Value, width: f64) -> Res<(f64, f64)> {
    measure(spec, ts, &text_of(label), spec.label, width)
}

/// A leaf's natural size at `avail` pixels, by its rule in `spec.leaves`
/// (loom/lib/layout.ol LY_LEAVES, which `ly_leaf_size` reads too).
fn leaf_size(
    spec: &Spec,
    ts: &mut TextSystem,
    node: &Value,
    role: &str,
    avail: f64,
) -> Res<(f64, f64)> {
    let p = props_of(node);
    let rule = match spec.leaves.get(role) {
        Some(Value::Map(r)) => r,
        _ => return Ok((0.0, 0.0)),
    };
    let r = Some(rule.as_ref());
    let num_or = |k: &str, d: f64| match mget(r, k) {
        Value::Unit => d,
        v => fnum(v),
    };
    if let Value::Tuple(t) = mget(r, "size") {
        return Ok((
            t.first().map(fnum).unwrap_or(0.0),
            t.get(1).map(fnum).unwrap_or(0.0),
        ));
    }
    if is_true(mget(r, "text")) {
        let fs = font_style_v(spec, p, role);
        return measure(spec, ts, &text_of(mget(p, "text")), &fs, avail);
    }
    if let Value::String(field) = mget(r, "label") {
        let (lw, lh) = label_size(spec, ts, mget(p, field.as_str()), avail)?;
        let h = match mget(r, "h") {
            Value::Unit => (lh + num_or("dh", 0.0)).max(num_or("min_h", 0.0)),
            v => fnum(v),
        };
        return Ok((lw + num_or("dw", 0.0), h));
    }
    if let Value::String(field) = mget(r, "widest") {
        let mut widest: f64 = 0.0;
        if let Value::List(opts) = mget(p, field.as_str()) {
            for o in opts.iter() {
                widest = widest.max(label_size(spec, ts, o, 10000.0)?.0);
            }
        }
        return Ok((widest + num_or("dw", 0.0), num_or("h", 0.0)));
    }
    if let Value::String(field) = mget(r, "rows") {
        let count = match mget(p, field.as_str()) {
            Value::List(l) => l.len() as f64,
            Value::Map(m) => m.get("count").map(fnum).unwrap_or(0.0),
            _ => 0.0,
        };
        let rh = match mget(p, "row_height") {
            Value::Unit => num_or("row_h", 32.0),
            v => fnum(v),
        };
        return Ok((
            num_or("w", 0.0),
            (count * rh + num_or("pad", 0.0)).min(num_or("max_h", f64::MAX)),
        ));
    }
    Ok((0.0, 0.0))
}

// ── gui.flatten ──────────────────────────────────────────────────────

/// `gui.flatten(root, spec, prev)`: the view as the core's arrays —
/// `#{ n, I, F, kids, nodes, paths, index, gkeys, glifts, wraps, changed }`
/// — or `()` when the view holds a `memo` node (Loom expands those
/// first). `changed` marks (1) each node whose role, key, or props differ
/// from `prev`'s (last frame's answer) at the same index, when the two
/// trees have one shape; otherwise it is `()`.
pub fn gui_flatten(args: Vec<Value>) -> Res<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!(
            "gui.flatten: expected 2 or 3 arguments, got {}",
            args.len()
        ));
    }
    let spec = Spec::read(&args[1])?;
    let root = &args[0];
    let mut ts = text::system().lock().map_err(|_| "gui: text poisoned")?;

    let mut ints: Vec<i64> = Vec::new();
    let mut floats: Vec<f64> = Vec::new();
    let mut ks: Vec<i64> = Vec::new();
    let mut nodes: Vec<Value> = Vec::new();
    let mut paths: Vec<Value> = Vec::new();
    let mut indexes: Vec<Value> = Vec::new();
    let mut gkeys: Vec<Value> = Vec::new();
    let mut glifts: Vec<Value> = Vec::new();
    let mut wraps: Vec<Value> = Vec::new();
    // Last frame's answer, walked in step with this one while the two
    // trees keep one shape: a node whose own role, key, props, and
    // layers are unchanged copies its entries instead of reading and
    // measuring again.
    let prev = Prev::read(args.get(2));
    let mut aligned = prev.is_some();
    let mut changed: Vec<Value> = Vec::new();
    // (node, parent index, the parent's path, its place j among the
    // parent's children, its index among siblings, its slot in kids);
    // a path is spelled only for a node read afresh
    let (row, column, stretch) = (st("row"), st("column"), st("stretch"));
    let mut stack: Vec<(Value, i64, Value, usize, i64, i64)> =
        vec![(root.clone(), -1, Value::Unit, 0, 0, -1)];
    while let Some((node, parent, ppath, j_at, index, slot)) = stack.pop() {
        let spell = || match &ppath {
            Value::String(pp) => format!("{pp}.{j_at}"),
            _ => "0".to_string(),
        };
        let idx = nodes.len() as i64;
        if slot >= 0 {
            ks[slot as usize] = idx;
        }
        let role: &str = role_of(&node);
        if role == "memo" {
            return Ok(Value::Unit);
        }
        let p = props_of(&node);
        // the children laid out here, and those lifted to a layer
        let all = children_of(&node);
        let mut kids: Vec<&Value> = Vec::new();
        let mut lifted: Vec<Value> = Vec::new();
        if role == "group" {
            for (i, c) in all.iter().enumerate() {
                if *c != Value::Unit && spec.lifted.contains(role_of(c)) {
                    lifted.push(tuple(vec![c.clone(), st(&format!("{}.L{i}", spell()))]));
                } else {
                    kids.push(c);
                }
            }
        } else if role == "region" || role == "split" {
            kids = all.iter().collect();
        }
        let kind: i64 = match role {
            "group" => 0,
            "region" => 1,
            "split" => 2,
            "text" | "heading" if is_true(mget(p, "wrap")) => 4,
            _ => 3,
        };
        let cnt = kids.len() as i64;
        let first = ks.len() as i64;

        let lifted = list(lifted);
        let i = idx as usize;
        let reuse = match &prev {
            Some(pv) if aligned => {
                if i >= pv.n
                    || pv.int(i, I_KIND) != kind
                    || pv.int(i, I_COUNT) != cnt
                    || pv.int(i, I_PARENT) != parent
                {
                    aligned = false;
                    false
                } else {
                    own_eq(&node, &pv.nodes[i]) && lifted == pv.glifts[i]
                }
            }
            _ => false,
        };
        if reuse {
            let pv = prev.as_ref().expect("reuse implies a last frame");
            ints.extend_from_slice(&pv.ints[i * FI..(i + 1) * FI]);
            ints[i * FI + I_FIRST] = first;
            floats.extend_from_slice(&pv.floats[i * FF..(i + 1) * FF]);
            gkeys.push(pv.gkeys[i].clone());
            paths.push(pv.paths[i].clone());
            changed.push(Value::Integer(0));
        } else {
            let path = spell();
            // ── the node's entry (`ly_fentry`) ──
            let (wk, wv) = fspec(mget(p, "width"));
            let (hk, hv) = fspec(mget(p, "height"));
            let (rk, rv, ck, cv) = if kind == 0 {
                if is_true(mget(p, "spacer")) {
                    let v = match mget(p, "weight") {
                        Value::Integer(w) => (*w as f64).max(1.0),
                        _ => 1.0,
                    };
                    (2, v, 2, v)
                } else {
                    (3, 0.0, 3, 0.0)
                }
            } else if spec.fills.contains(role) {
                (2, 1.0, 2, 1.0)
            } else if role == "input" || role == "textarea" {
                (2, 1.0, 3, 0.0)
            } else {
                (3, 0.0, 3, 0.0)
            };
            let (pt, pr, pb, pl) = if kind == 0 {
                pad4(mget(p, "pad"))
            } else {
                (0.0, 0.0, 0.0, 0.0)
            };
            let border = if kind == 0 { border_of(p) } else { 0.0 };
            let gap = if kind == 0 { fnum(mget(p, "gap")) } else { 0.0 };
            let sa = mget(p, "align_self");
            let (nw, nh, fullw) = if kind == 3 {
                let (a, b) = leaf_size(&spec, &mut ts, &node, role, 1000000.0)?;
                (a, b, 0.0)
            } else if kind == 4 {
                let fs = font_style_v(&spec, p, role);
                (
                    0.0,
                    0.0,
                    measure(&spec, &mut ts, &text_of(mget(p, "text")), &fs, 1000000.0)?.0,
                )
            } else {
                (0.0, 0.0, 0.0)
            };
            let dir = if kind == 0 {
                mget_or(p, "dir", &column)
            } else if kind == 2 {
                mget_or(p, "dir", &row)
            } else {
                &column
            };
            let dir = match dir {
                Value::String(s) if s.as_str() == "row" => 1,
                Value::String(s) if s.as_str() == "stack" => 2,
                _ => 0,
            };
            let (align, justify) = if kind == 0 {
                (
                    falign(mget_or(p, "align", &stretch)),
                    fjustify(mget(p, "justify")),
                )
            } else {
                (0, 0)
            };
            let opt = |k: &str| match mget(p, k) {
                Value::Unit => -1.0,
                v => fnum(v),
            };
            let mn = mget(p, "min");
            ints.extend_from_slice(&[
                kind,
                dir,
                first,
                cnt,
                align,
                justify,
                if *sa == Value::Unit { -1 } else { falign(sa) },
                wk,
                hk,
                rk,
                ck,
                spec.shrinks.contains(role) as i64,
                parent,
                (kind == 2 || spec.ctx.contains(role)) as i64,
            ]);
            floats.extend_from_slice(&[
                wv,
                hv,
                rv,
                cv,
                border,
                pt,
                pr,
                pb,
                pl,
                gap,
                opt("min"),
                opt("max"),
                opt("min_width"),
                opt("max_width"),
                opt("min_height"),
                opt("max_height"),
                nw,
                nh,
                fullw,
                if kind == 2 {
                    match mget(p, "ratio") {
                        Value::Unit => 0.5,
                        v => fnum(v),
                    }
                } else {
                    0.5
                },
                if *mn == Value::Unit { 80.0 } else { fnum(mn) },
                0.0,
            ]);
            gkeys.push(st(&gkey_of(&node, &path)));
            paths.push(st(&path));
            changed.push(Value::Integer(1));
        }
        indexes.push(Value::Integer(index));
        glifts.push(lifted);
        if kind == 4 {
            wraps.push(Value::Integer(idx));
        }
        ks.extend(std::iter::repeat_n(-1, kids.len()));
        let here = paths.last().cloned().unwrap_or(Value::Unit);
        // pushed last to first, so the first is reached first; a split's
        // children are numbered 0 and 2 (its divider is 1)
        for j in (0..kids.len()).rev() {
            let ix = if role == "split" && j == 1 {
                2
            } else {
                j as i64
            };
            stack.push((kids[j].clone(), idx, here.clone(), j, ix, first + j as i64));
        }
        nodes.push(node);
    }
    let n = nodes.len();
    // which nodes changed: known only when the trees kept one shape
    let changed = match &prev {
        Some(pv) if aligned && pv.n == n => {
            changed.resize(n.max(PAD), Value::Integer(0));
            list(changed)
        }
        _ => Value::Unit,
    };
    // Padded past 64 entries: a list of 64 numbers or fewer crosses into
    // olang's VM boxed, a longer one typed, and the native passes keep
    // one compiled form only when they always see one layout. The passes
    // read the first `n` nodes; what follows is never read.
    ints.resize(ints.len().max(PAD), 0);
    floats.resize(floats.len().max(PAD), 0.0);
    ks.resize(ks.len().max(PAD), 0);
    Ok(map(vec![
        ("n", Value::Integer(n as i64)),
        ("I", list(ints.into_iter().map(Value::Integer).collect())),
        ("F", list(floats.into_iter().map(Value::Float).collect())),
        ("kids", list(ks.into_iter().map(Value::Integer).collect())),
        ("nodes", list(nodes)),
        ("paths", list(paths)),
        ("index", list(indexes)),
        ("gkeys", list(gkeys)),
        ("glifts", list(glifts)),
        ("wraps", list(wraps)),
        ("changed", changed),
    ]))
}

/// Last frame's answer from `gui.flatten`, decoded.
struct Prev<'a> {
    n: usize,
    ints: Vec<i64>,
    floats: Vec<f64>,
    nodes: &'a [Value],
    paths: &'a [Value],
    gkeys: &'a [Value],
    glifts: &'a [Value],
}

impl<'a> Prev<'a> {
    fn read(v: Option<&'a Value>) -> Option<Prev<'a>> {
        let f = fields(v?)?;
        let n = match f.get("n")? {
            Value::Integer(n) => *n as usize,
            _ => return None,
        };
        let l = |k: &str| match f.get(k) {
            Some(Value::List(l)) if l.len() >= n => Some(&l[..]),
            _ => None,
        };
        let (pi, pf) = (l("I")?, l("F")?);
        if pi.len() < n * FI || pf.len() < n * FF {
            return None;
        }
        Some(Prev {
            n,
            ints: pi
                .iter()
                .map(|v| {
                    if let Value::Integer(i) = v {
                        *i
                    } else {
                        i64::MIN
                    }
                })
                .collect(),
            floats: pf.iter().map(fnum).collect(),
            nodes: l("nodes")?,
            paths: l("paths")?,
            gkeys: l("gkeys")?,
            glifts: l("glifts")?,
        })
    }

    fn int(&self, i: usize, f: usize) -> i64 {
        self.ints[i * FI + f]
    }
}

/// A node's own role, key, and props are those of `b` (its children are
/// compared at their own indices).
fn own_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Map(x), Value::Map(y)) => {
            Arc::ptr_eq(x, y)
                || (x.get("role") == y.get("role")
                    && x.get("key") == y.get("key")
                    && x.get("props") == y.get("props"))
        }
        _ => a == b,
    }
}

// ── gui.flat_emit ────────────────────────────────────────────────────

fn nums_of(v: &Value, what: &str) -> Res<Vec<f64>> {
    match v {
        Value::List(l) => Ok(l.iter().map(fnum).collect()),
        _ => Err(format!("gui.flat_emit: {what} must be a list of numbers")),
    }
}

fn ints_of(v: &Value, what: &str) -> Res<Vec<i64>> {
    match v {
        Value::List(l) => Ok(l
            .iter()
            .map(|x| match x {
                Value::Integer(i) => *i,
                Value::Float(f) => *f as i64,
                _ => 0,
            })
            .collect()),
        _ => Err(format!("gui.flat_emit: {what} must be a list of integers")),
    }
}

fn list_of<'a>(f: &'a crate::ast::ValueMap, k: &str) -> Res<&'a [Value]> {
    match f.get(k) {
        Some(Value::List(l)) => Ok(l),
        _ => Err(format!("gui.flat_emit: \"{k}\" must be a list")),
    }
}

/// One placement (`ly_rec`), with the engine's operation (`ly_gop`).
#[allow(clippy::too_many_arguments)]
fn rec(
    gkey: &str,
    parent: &Value,
    index: i64,
    b: (f64, f64, f64, f64),
    origin: (f64, f64),
    node: &Value,
    op: crate::ast::ValueMap,
    focusable: bool,
    path: &str,
) -> Value {
    let (x, y, w, h) = b;
    let rel = tuple(vec![
        Value::Float(x - origin.0),
        Value::Float(y - origin.1),
        Value::Float(w),
        Value::Float(h),
    ]);
    let mut gop = crate::ast::ValueMap::with_capacity_and_hasher(op.len() + 4, Default::default());
    gop.insert("key".to_string(), st(gkey));
    gop.insert("box".to_string(), rel.clone());
    if *parent != Value::Unit {
        gop.insert("parent".to_string(), parent.clone());
        gop.insert("index".to_string(), Value::Integer(index));
    }
    for (k, v) in op.iter() {
        if *v != Value::Unit {
            gop.insert(k.clone(), v.clone());
        }
    }
    vmap(hm(vec![
        ("gkey", st(gkey)),
        ("parent", parent.clone()),
        ("index", Value::Integer(index)),
        ("rel", rel),
        (
            "abs",
            tuple(vec![
                Value::Float(x),
                Value::Float(y),
                Value::Float(w),
                Value::Float(h),
            ]),
        ),
        ("node", node.clone()),
        ("op", vmap(op)),
        ("gop", vmap(gop)),
        ("focusable", Value::Boolean(focusable)),
        ("path", st(path)),
    ]))
}

fn group_op(
    spec: &Spec,
    p: Option<&crate::ast::ValueMap>,
    default_role: &str,
) -> crate::ast::ValueMap {
    let dr = st(default_role);
    hm(vec![
        ("role", mget_or(p, "a11y_role", &dr).clone()),
        ("name", mget(p, "name").clone()),
        (
            "style",
            with_style(
                spec,
                vmap(crate::ast::ValueMap::default()),
                mget(p, "style"),
            ),
        ),
    ])
}

/// `gui.flat_emit(flat, geo, spec, keep, prev)`: each node's placements
/// from the core's boxes `geo` (x, y, w, h a node), by index —
/// `#{ recs, gkeys, probs, lifts, pre, todo }`. A node `keep` marks (1)
/// takes `prev`'s (last frame's answer, as Loom completed it) unless its
/// parent's gkey changed. Groups, regions, splits, and the leaves in
/// `spec.placed` are placed here; the other leaves are listed in `todo`
/// for Loom to place, their entries left empty. `pre[i]` is what comes
/// before node i (a split's divider, before its second pane).
pub fn gui_flat_emit(args: Vec<Value>) -> Res<Value> {
    if args.len() != 5 {
        return Err(format!(
            "gui.flat_emit: expected 5 arguments, got {}",
            args.len()
        ));
    }
    let flat = fields(&args[0]).ok_or("gui.flat_emit: the arrays must be a map")?;
    let geo = nums_of(&args[1], "the boxes")?;
    let spec = Spec::read(&args[2])?;
    let keep = match &args[3] {
        Value::Unit => None,
        v => Some(ints_of(v, "keep")?),
    };
    let prev = match (&args[4], &keep) {
        (Value::Map(m), Some(_)) => Some(m),
        _ => None,
    };
    let n = match flat.get("n") {
        Some(Value::Integer(n)) => *n as usize,
        _ => return Err("gui.flat_emit: the arrays have no \"n\"".into()),
    };
    let iv = ints_of(flat.get("I").unwrap_or(&Value::Unit), "I")?;
    let kids = ints_of(flat.get("kids").unwrap_or(&Value::Unit), "kids")?;
    let nodes = list_of(flat, "nodes")?;
    let paths = list_of(flat, "paths")?;
    let indexes = list_of(flat, "index")?;
    let fgkeys = list_of(flat, "gkeys")?;
    let glifts = list_of(flat, "glifts")?;
    let fv = nums_of(flat.get("F").unwrap_or(&Value::Unit), "F")?;
    if iv.len() < n * FI || geo.len() < n * 4 || nodes.len() < n {
        return Err("gui.flat_emit: the arrays and the boxes disagree on the node count".into());
    }
    let empty: Vec<Value> = Vec::new();
    let (p_recs, p_gkeys, p_probs, p_lifts) = match prev {
        Some(m) => (
            list_of(m, "recs")?,
            list_of(m, "gkeys")?,
            list_of(m, "probs")?,
            list_of(m, "lifts")?,
        ),
        None => (&empty[..], &empty[..], &empty[..], &empty[..]),
    };
    let reusable = prev.is_some()
        && p_recs.len() == n
        && p_gkeys.len() == n
        && p_probs.len() == n
        && p_lifts.len() == n;

    let accent = spec.tok_of("accent");
    let hairline = spec.tok_of("hairline");
    let empty_list = list(Vec::new());
    let mut recs: Vec<Value> = Vec::with_capacity(n);
    let mut gkeys: Vec<Value> = Vec::with_capacity(n);
    let mut probs: Vec<Value> = Vec::with_capacity(n);
    let mut lifts: Vec<Value> = Vec::with_capacity(n);
    let mut pre: Vec<Value> = vec![empty_list.clone(); n];
    let mut renamed = vec![false; n];
    let mut todo: Vec<Value> = Vec::new();
    let g = |i: usize, f: usize| iv[i * FI + f];
    for i in 0..n {
        let pi = g(i, I_PARENT);
        let kind = g(i, I_KIND);
        let reuse = reusable
            && keep.as_ref().is_some_and(|k| k.get(i) == Some(&1))
            && (pi < 0 || !renamed[pi as usize]);
        if reuse {
            recs.push(p_recs[i].clone());
            gkeys.push(p_gkeys[i].clone());
            probs.push(p_probs[i].clone());
            lifts.push(p_lifts[i].clone());
            continue;
        }
        let node = &nodes[i];
        let role = role_of(node);
        let p = props_of(node);
        let path = match &paths[i] {
            Value::String(s) => s.as_str(),
            _ => "",
        };
        let index = match indexes[i] {
            Value::Integer(v) => v,
            _ => 0,
        };
        let gk = match &fgkeys[i] {
            Value::String(s) => s.as_str().to_string(),
            other => other.to_string(),
        };
        if reusable && p_gkeys[i] != fgkeys[i] {
            renamed[i] = true;
        }
        let parent = if pi < 0 {
            Value::Unit
        } else {
            gkeys[pi as usize].clone()
        };
        let b = (geo[i * 4], geo[i * 4 + 1], geo[i * 4 + 2], geo[i * 4 + 3]);
        let origin = if pi < 0 {
            (0.0, 0.0)
        } else {
            (geo[pi as usize * 4], geo[pi as usize * 4 + 1])
        };
        let mut mine: Vec<Value> = Vec::new();
        let mut pr: Vec<Value> = Vec::new();
        let mut placed = true;
        match kind {
            0 => mine.push(rec(
                &gk,
                &parent,
                index,
                b,
                origin,
                node,
                group_op(&spec, p, "group"),
                false,
                path,
            )),
            1 => {
                let mut op = group_op(&spec, p, "group");
                let region = st("region");
                op.insert("role".into(), mget_or(p, "a11y_role", &region).clone());
                op.insert("scroll".into(), Value::Boolean(true));
                mine.push(rec(&gk, &parent, index, b, origin, node, op, false, path));
            }
            2 => {
                // the split, its divider, and the divider's line
                // (`ly_flat_split`); the panes are placed by the core
                let first = g(i, I_FIRST) as usize;
                let cnt = g(i, I_COUNT);
                let row = matches!(mget(p, "dir"), Value::Unit)
                    || matches!(mget(p, "dir"), Value::String(s) if s.as_str() == "row");
                let (x, y, w, h) = b;
                let pane = if cnt > 0 {
                    let c = kids[first] as usize;
                    if row { geo[c * 4 + 2] } else { geo[c * 4 + 3] }
                } else {
                    0.0
                };
                let (bar, grab) = (8.0, 24.0);
                let reach = (grab - bar) / 2.0;
                let room = ((if row { w } else { h }) - bar).max(0.0);
                let me_op = hm(vec![
                    ("role", st("group")),
                    ("name", mget(p, "name").clone()),
                ]);
                let me = rec(&gk, &parent, index, b, origin, node, me_op, false, path);
                let d = if row {
                    (x + pane - reach, y, grab, h)
                } else {
                    (x, y + pane - reach, w, grab)
                };
                let pct = (100.0 * pane / room.max(1.0)) as i64;
                let dkey = format!("{gk}:divider");
                let div = rec(
                    &dkey,
                    &st(&gk),
                    1,
                    d,
                    (x, y),
                    node,
                    hm(vec![
                        ("role", st("separator")),
                        ("name", st("Resize")),
                        ("value", st(&format!("{pct}%"))),
                        ("focusable", Value::Boolean(true)),
                        ("grab", Value::Boolean(true)),
                        ("style", vmap(hm(vec![("focus_ring", accent.clone())]))),
                    ]),
                    true,
                    "",
                );
                let lb = if row {
                    (d.0 + grab / 2.0, d.1, 1.0, d.3)
                } else {
                    (d.0, d.1 + grab / 2.0, d.2, 1.0)
                };
                let line = rec(
                    &format!("{gk}:line"),
                    &st(&dkey),
                    0,
                    lb,
                    (d.0, d.1),
                    node,
                    hm(vec![
                        ("role", st("group")),
                        ("style", vmap(hm(vec![("bg", hairline.clone())]))),
                    ]),
                    false,
                    "",
                );
                mine.push(me);
                if cnt > 1 {
                    pre[kids[first + 1] as usize] = list(vec![div, line]);
                } else {
                    mine.push(div);
                    mine.push(line);
                }
            }
            _ if spec.placed.contains(role) => match role {
                "text" | "heading" => {
                    let fsv = font_style_v(&spec, p, role);
                    let wrap = is_true(mget(p, "wrap"));
                    let op = hm(vec![
                        ("role", st(role)),
                        ("text", mget(p, "text").clone()),
                        ("style", with_style(&spec, fsv.clone(), mget(p, "style"))),
                        ("level", mget(p, "level").clone()),
                        ("name", mget(p, "name").clone()),
                    ]);
                    // a text that does not wrap is one line at any width:
                    // its height is its natural one
                    let nh = if wrap {
                        let mut ts = text::system().lock().map_err(|_| "gui: text poisoned")?;
                        measure(&spec, &mut ts, &text_of(mget(p, "text")), &fsv, b.2)?.1
                    } else {
                        fv.get(i * FF + F_NATH).copied().unwrap_or(0.0)
                    };
                    if nh > b.3 + 0.5 && *mget(p, "truncate") == Value::Unit && b.3 > 0.0 {
                        pr.push(st(&format!(
                            "text \"{gk}\" needs {}px of height and has {}",
                            Value::Float(nh),
                            Value::Float(b.3)
                        )));
                    }
                    mine.push(rec(&gk, &parent, index, b, origin, node, op, false, path));
                }
                "button" => {
                    let disabled = is_true(mget(p, "disabled"));
                    let look = if disabled {
                        spec.look("button_disabled")
                    } else if is_true(mget(p, "primary")) {
                        spec.look("button_primary")
                    } else {
                        spec.look("button")
                    };
                    let op = hm(vec![
                        ("role", st("button")),
                        ("text", mget(p, "label").clone()),
                        ("name", mget(p, "name").clone()),
                        ("disabled", Value::Boolean(disabled)),
                        ("style", with_style(&spec, look, mget(p, "style"))),
                    ]);
                    mine.push(rec(
                        &gk, &parent, index, b, origin, node, op, !disabled, path,
                    ));
                }
                "separator" => {
                    let op = hm(vec![
                        ("role", st("separator")),
                        ("style", spec.look("separator")),
                    ]);
                    mine.push(rec(&gk, &parent, index, b, origin, node, op, false, path));
                }
                _ => placed = false,
            },
            _ => placed = false,
        }
        if !placed {
            todo.push(Value::Integer(i as i64));
        }
        recs.push(list(mine));
        gkeys.push(st(&gk));
        probs.push(list(pr));
        lifts.push(glifts[i].clone());
    }
    Ok(map(vec![
        ("recs", list(recs)),
        ("gkeys", list(gkeys)),
        ("probs", list(probs)),
        ("lifts", list(lifts)),
        ("pre", list(pre)),
        ("todo", list(todo)),
    ]))
}

/// `gui.flat_join(pre, recs, probs, lifts)`: the frame's placements in
/// tree order, its problems, and its lifted layers — each list of lists
/// joined. Answers `#{ nodes, problems, lifted }`.
pub fn gui_flat_join(args: Vec<Value>) -> Res<Value> {
    if args.len() != 4 {
        return Err(format!(
            "gui.flat_join: expected 4 arguments, got {}",
            args.len()
        ));
    }
    let join = |vs: &[&Value]| -> Vec<Value> {
        let mut out = Vec::new();
        let ls: Vec<&[Value]> = vs
            .iter()
            .map(|v| match v {
                Value::List(l) => &l[..],
                _ => EMPTY,
            })
            .collect();
        let n = ls.iter().map(|l| l.len()).max().unwrap_or(0);
        for i in 0..n {
            for l in &ls {
                if let Some(Value::List(xs)) = l.get(i) {
                    out.extend(xs.iter().cloned());
                }
            }
        }
        out
    };
    Ok(map(vec![
        ("nodes", list(join(&[&args[0], &args[1]]))),
        ("problems", list(join(&[&args[2]]))),
        ("lifted", list(join(&[&args[3]]))),
    ]))
}

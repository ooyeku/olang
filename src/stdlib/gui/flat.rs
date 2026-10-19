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
use crate::ovm::value::{OvmValue, ValueData};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

pub const FI: usize = 14;
pub const FF: usize = 22;

const I_KIND: usize = 0;
const I_FIRST: usize = 2;
const I_COUNT: usize = 3;
const I_SHRINK: usize = 11;
const I_PARENT: usize = 12;
const I_FRESH: usize = 13;
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
    let mut out = HashSet::default();
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
            plain: RefCell::new(HashMap::default()),
            parsed: RefCell::new(HashMap::default()),
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

/// A group that cannot give room (lib/layout.ol's `ly_rigid`): said so
/// (`keep: true`), or its children all controls and pictures — shrunk, a
/// row of buttons only cut its last ones to slivers.
fn rigid_group(spec: &Spec, node: &Value, p: Option<&crate::ast::ValueMap>, role: &str) -> bool {
    if role != "group" {
        return false;
    }
    if is_true(mget(p, "keep")) {
        return true;
    }
    let kids = children_of(node);
    let mut n = 0;
    for k in kids {
        if *k == Value::Unit {
            continue;
        }
        n += 1;
        if spec.shrinks.contains(role_of(k)) {
            return false;
        }
    }
    n > 0
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
            None | Some(Value::Unit) => tok_alpha(spec, s).unwrap_or_else(|| v.clone()),
            Some(c) => c.clone(),
        },
        _ => v.clone(),
    }
}

/// A token at an opacity, as a canvas spells one (`"danger/12"`: the
/// token's colour at 12%): a tinted row or a marked span in a style.
fn tok_alpha(spec: &Spec, s: &str) -> Option<Value> {
    let (base, pct) = s.split_once('/')?;
    let pct: f64 = pct.parse().ok()?;
    match spec.theme.get(base) {
        Some(Value::String(c)) if c.len() == 7 && c.starts_with('#') => {
            let a = (pct.clamp(0.0, 100.0) * 255.0 / 100.0).round() as u8;
            Some(Value::String(Arc::new(format!("{}{:02x}", c, a))))
        }
        _ => None,
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
    // a text's styled runs: their looks' tokens resolved
    if let Value::List(l) = mget(p, "spans") {
        let resolved: Vec<Value> = l
            .iter()
            .map(|sp| match sp {
                Value::Tuple(t) if t.len() >= 3 => Value::Tuple(Arc::new(vec![
                    t[0].clone(),
                    t[1].clone(),
                    with_style(spec, vmap(crate::ast::ValueMap::default()), &t[2]),
                ])),
                other => other.clone(),
            })
            .collect();
        s.insert("spans".into(), Value::List(Arc::new(resolved)));
    }
    let c = mget(p, "color");
    if *c != Value::Unit {
        s.insert("color".into(), tok(spec, c));
    }
    if is_true(mget(p, "wrap")) {
        s.insert("wrap".into(), Value::Boolean(true));
    }
    if is_true(mget(p, "truncate")) {
        s.insert("truncate".into(), Value::Boolean(true));
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
    "truncate",
    "size",
    "weight",
    "font",
    "italic",
    "line_height",
    "spans",
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
        // a text's `pad` widens its box as the recursive layout's does
        // (lib/layout.ol `ly_extra`): the words drawn inside it, not cut
        let (pt, pr, pb, pl) = pad4(mget(p, "pad"));
        let (w, h) = measure(
            spec,
            ts,
            &text_of(mget(p, "text")),
            &fs,
            (avail - pl - pr).max(0.0),
        )?;
        return Ok((w + pl + pr, h + pt + pb));
    }
    if let Value::String(field) = mget(r, "label") {
        let (lw, lh) = label_size(spec, ts, mget(p, field.as_str()), avail)?;
        let h = match mget(r, "h") {
            Value::Unit => (lh + num_or("dh", 0.0)).max(num_or("min_h", 0.0)),
            v => fnum(v),
        };
        // a node may say its own room around its label (a compact segment)
        let dw = match mget(p, "dw") {
            Value::Unit => num_or("dw", 0.0),
            v => fnum(v),
        };
        return Ok((lw + dw, h));
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

/// `gui.flatten(root, spec, prev)`: the view read into the core's arrays
/// — `#{ n, id, nodes, paths, index, gkeys, glifts, wraps, changed }` — or
/// `()` when the view holds a `memo` node (Loom expands those first).
/// A fourth argument names the root's path (`"0"` without one): a
/// subtree laid out on its own — a row of a virtual list — keeps the
/// paths, and so the keys of its keyless nodes, it has in the whole.
/// `changed` marks (1) each node whose role, key, or props differ from
/// `prev`'s (last frame's answer) at the same index, when the two trees
/// have one shape; otherwise it is `()`.
///
/// The arrays themselves stay in the engine, under the answer's `id`:
/// `gui.flat_arrays(id)` hands them over (to the VM without a copy), and
/// the next frame's `gui.flatten` and this frame's `gui.flat_emit` read
/// them where they are. Crossing as boxed lists, they cost more than the
/// layout they serve. When the view keeps last frame's shape, only the
/// changed nodes are read, and `paths`, `index`, `wraps` are last frame's
/// lists (and `gkeys`, `glifts`, unless a changed node's differ).
pub fn gui_flatten(args: Vec<Value>) -> Res<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err(format!(
            "gui.flatten: expected 2 to 4 arguments, got {}",
            args.len()
        ));
    }
    let spec = Spec::read(&args[1])?;
    let root = &args[0];
    let base = match args.get(3) {
        Some(Value::String(s)) => s.as_str().to_string(),
        _ => "0".to_string(),
    };
    let mut ts = text::system().lock().map_err(|_| "gui: text poisoned")?;
    let prev = Prev::read(args.get(2));
    if let Some(pv) = &prev
        && let Some(answer) = flatten_aligned(root, &spec, &mut ts, pv)?
    {
        return Ok(answer);
    }
    flatten_full(root, &spec, &mut ts, prev.as_ref(), &base)
}

/// A split's share for the core (`F_RATIO`, Loom's lib/flex.ol): its
/// ratio (0 to 1); or, sized in pixels (`size`), `2 + size` for the
/// first pane's or `-(1 + size)` for the second's (`fixed: "second"`).
fn split_ratio(p: Option<&crate::ast::ValueMap>) -> f64 {
    match mget(p, "size") {
        Value::Unit => match mget(p, "ratio") {
            Value::Unit => 0.5,
            v => fnum(v),
        },
        v => {
            let px = fnum(v).max(0.0);
            if matches!(mget(p, "fixed"), Value::String(s) if s.as_str() == "second") {
                -(1.0 + px)
            } else {
                2.0 + px
            }
        }
    }
}

/// A split's divider thickness (`bar`, 8 unless given).
fn split_bar(p: Option<&crate::ast::ValueMap>) -> f64 {
    match mget(p, "bar") {
        Value::Unit => 8.0,
        v => fnum(v).max(0.0),
    }
}

/// The core's kind for a node of `role` (0 group, 1 region, 2 split, 3
/// leaf, 4 wrapping text).
fn kind_of(role: &str, p: Option<&crate::ast::ValueMap>) -> i64 {
    match role {
        "group" => 0,
        "region" => 1,
        "split" => 2,
        "text" | "heading" if is_true(mget(p, "wrap")) => 4,
        _ => 3,
    }
}

/// Whether a group lifts any child to a layer (most do not, and the
/// answer's `glifts` entry is then an empty list).
fn lifts_any(spec: &Spec, role: &str, all: &[Value]) -> bool {
    role == "group"
        && all
            .iter()
            .any(|c| *c != Value::Unit && spec.lifted.contains(role_of(c)))
}

/// A node's laid-out children and the ones it lifts to a layer, each
/// lifted one as `(child, "<path>.L<i>")`.
fn split_kids<'v>(
    spec: &Spec,
    role: &str,
    all: &'v [Value],
    path: &dyn Fn() -> String,
) -> (Vec<&'v Value>, Value) {
    let mut kids: Vec<&Value> = Vec::new();
    let mut lifted: Vec<Value> = Vec::new();
    if role == "group" {
        for (i, c) in all.iter().enumerate() {
            if *c != Value::Unit && spec.lifted.contains(role_of(c)) {
                lifted.push(tuple(vec![c.clone(), st(&format!("{}.L{i}", path()))]));
            } else {
                kids.push(c);
            }
        }
    } else if role == "region" || role == "split" {
        kids = all.iter().collect();
    }
    (kids, list(lifted))
}

/// A node's entry in the arrays (`ly_fentry`): FI ints and FF floats.
#[allow(clippy::too_many_arguments)]
fn node_entry(
    spec: &Spec,
    ts: &mut TextSystem,
    node: &Value,
    p: Option<&crate::ast::ValueMap>,
    role: &str,
    kind: i64,
    cnt: i64,
    first: i64,
    parent: i64,
) -> Res<([i64; FI], [f64; FF])> {
    let (row, column, stretch) = (st("row"), st("column"), st("stretch"));
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
        let (a, b) = leaf_size(spec, ts, node, role, 1000000.0)?;
        (a, b, 0.0)
    } else if kind == 4 {
        let fs = font_style_v(spec, p, role);
        (
            0.0,
            0.0,
            measure(spec, ts, &text_of(mget(p, "text")), &fs, 1000000.0)?.0,
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
    // a region that scrolls across too (`axis: "both"`): its child is as
    // wide as it is wide, not as the region (lib/flex.ol)
    let dir = if kind == 1
        && matches!(mget(p, "axis"), Value::String(a) if a.as_str() == "both" || a.as_str() == "x")
    {
        3
    } else {
        dir
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
    Ok((
        [
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
            (spec.shrinks.contains(role) && !rigid_group(spec, node, p, role)) as i64,
            parent,
            (kind == 2 || spec.ctx.contains(role)) as i64,
        ],
        [
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
            if kind == 2 { split_ratio(p) } else { 0.5 },
            if *mn == Value::Unit { 80.0 } else { fnum(mn) },
            if kind == 2 { split_bar(p) } else { 0.0 },
        ],
    ))
}

/// A view laid out against last frame's, when the two keep one shape:
/// every node at the same index with the same kind, child count, and
/// parent. Answers the patch (see `gui_flatten`), or `None` when the
/// shape changed (or a `memo` node appears) and the arrays must be built
/// afresh.
fn flatten_aligned(
    root: &Value,
    spec: &Spec,
    ts: &mut TextSystem,
    pv: &Prev,
) -> Res<Option<Value>> {
    let last = &pv.st;
    let mut nodes: Vec<Value> = Vec::with_capacity(last.n);
    let mut patch_idx: Vec<i64> = Vec::new();
    let mut patch_i: Vec<i64> = Vec::new();
    let mut patch_f: Vec<f64> = Vec::new();
    let mut gkey_over: Vec<(usize, Value)> = Vec::new();
    let mut lift_over: Vec<(usize, Value)> = Vec::new();
    let mut changed = vec![Value::Integer(0); last.n.max(PAD)];
    // the tree is walked by reference: a node is cloned once, into the
    // answer's `nodes`
    let mut stack: Vec<(&Value, i64)> = Vec::with_capacity(64);
    stack.push((root, -1));
    while let Some((node, parent)) = stack.pop() {
        let i = nodes.len();
        if i >= last.n {
            return Ok(None);
        }
        let role: &str = role_of(node);
        if role == "memo" {
            return Ok(None);
        }
        let p = props_of(node);
        let all = children_of(node);
        let kind = kind_of(role, p);
        let lays_out = role == "group" || role == "region" || role == "split";
        let lifts = lifts_any(spec, role, all);
        // a node's path is last frame's at its index (the shape is the
        // same); spelled only for a node read again
        let path = || match &pv.paths[i] {
            Value::String(s) => s.as_str().to_string(),
            _ => String::new(),
        };
        let (lifted_kids, lifted) = if lifts {
            let (k, l) = split_kids(spec, role, all, &path);
            (Some(k), l)
        } else {
            (None, Value::Unit)
        };
        let cnt = match &lifted_kids {
            Some(k) => k.len(),
            None if lays_out => all.len(),
            None => 0,
        } as i64;
        if last.int(i, I_KIND) != kind
            || last.int(i, I_COUNT) != cnt
            || last.int(i, I_PARENT) != parent
        {
            return Ok(None);
        }
        let lifted_same = if lifts {
            lifted == pv.glifts[i]
        } else {
            matches!(&pv.glifts[i], Value::List(l) if l.is_empty())
        };
        if !(lifted_same && own_eq(node, &pv.nodes[i])) {
            let (ints, floats) = node_entry(
                spec,
                ts,
                node,
                p,
                role,
                kind,
                cnt,
                last.int(i, I_FIRST),
                parent,
            )?;
            patch_idx.push(i as i64);
            patch_i.extend_from_slice(&ints);
            patch_f.extend_from_slice(&floats);
            changed[i] = Value::Integer(1);
            let gk = st(&gkey_of(node, &path()));
            if gk != pv.gkeys[i] {
                gkey_over.push((i, gk));
            }
            if !lifted_same {
                lift_over.push((i, if lifts { lifted } else { list(Vec::new()) }));
            }
        }
        match lifted_kids {
            Some(k) => {
                for c in k.into_iter().rev() {
                    stack.push((c, i as i64));
                }
            }
            None if lays_out => {
                for c in all.iter().rev() {
                    stack.push((c, i as i64));
                }
            }
            None => {}
        }
        nodes.push(node.clone());
    }
    if nodes.len() != last.n {
        return Ok(None);
    }
    let n = last.n;
    // this frame's arrays: last frame's, shared when nothing changed, with
    // the changed nodes written over a copy otherwise
    let (ints, floats) = if patch_idx.is_empty() {
        (last.ints.clone(), last.floats.clone())
    } else {
        let mut ints = last.ints.as_ref().clone();
        let mut floats = last.floats.as_ref().clone();
        for (c, &at) in patch_idx.iter().enumerate() {
            let at = at as usize;
            ints[at * FI..(at + 1) * FI].copy_from_slice(&patch_i[c * FI..(c + 1) * FI]);
            floats[at * FF..(at + 1) * FF].copy_from_slice(&patch_f[c * FF..(c + 1) * FF]);
        }
        (Arc::new(ints), Arc::new(floats))
    };
    let id = keep_state(FlatState {
        n,
        ints,
        floats,
        kids: last.kids.clone(),
        geo: Mutex::new(None),
    });
    let overridden = |base: &Value, over: Vec<(usize, Value)>| -> Value {
        if over.is_empty() {
            return base.clone();
        }
        let mut items = match base {
            Value::List(l) => l.as_ref().clone(),
            _ => Vec::new(),
        };
        for (i, v) in over {
            if i < items.len() {
                items[i] = v;
            }
        }
        list(items)
    };
    Ok(Some(map(vec![
        ("n", Value::Integer(n as i64)),
        ("id", Value::Integer(id)),
        ("nodes", list(nodes)),
        ("paths", pv.paths_v.clone()),
        ("index", pv.index_v.clone()),
        ("gkeys", overridden(pv.gkeys_v, gkey_over)),
        ("glifts", overridden(pv.glifts_v, lift_over)),
        ("wraps", pv.wraps_v.clone()),
        ("changed", list(changed)),
    ])))
}

/// The arrays built afresh: each node read (props parsed, leaves
/// measured) unless last frame's answer holds it unchanged at the same
/// place in a tree of the same shape so far.
fn flatten_full(
    root: &Value,
    spec: &Spec,
    ts: &mut TextSystem,
    prev: Option<&Prev>,
    base: &str,
) -> Res<Value> {
    let mut ints: Vec<i64> = Vec::new();
    let mut floats: Vec<f64> = Vec::new();
    let mut ks: Vec<i64> = Vec::new();
    let mut nodes: Vec<Value> = Vec::new();
    let mut paths: Vec<Value> = Vec::new();
    let mut indexes: Vec<Value> = Vec::new();
    let mut gkeys: Vec<Value> = Vec::new();
    let mut glifts: Vec<Value> = Vec::new();
    let mut wraps: Vec<Value> = Vec::new();
    let mut aligned = prev.is_some();
    let mut changed: Vec<Value> = Vec::new();
    // last frame's nodes by gkey, made once the trees stop lining up by
    // index: a node that has its key and is what it was (its own role,
    // key and props, kind, child count, lifted layers) takes last frame's
    // entry, not read again (a cell scrolled into a notebook moves every
    // node after it; they are all what they were)
    let mut by_key: Option<HashMap<&str, usize>> = None;
    // (node, parent index, the parent's path, its place j among the
    // parent's children, its index among siblings, its slot in kids);
    // a path is spelled only for a node read afresh
    let mut stack: Vec<(Value, i64, Value, usize, i64, i64)> =
        vec![(root.clone(), -1, Value::Unit, 0, 0, -1)];
    while let Some((node, parent, ppath, j_at, index, slot)) = stack.pop() {
        let spell = || match &ppath {
            Value::String(pp) => format!("{pp}.{j_at}"),
            _ => base.to_string(),
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
        let all = children_of(&node);
        let (kids, lifted) = split_kids(spec, role, all, &spell);
        let kind = kind_of(role, p);
        let cnt = kids.len() as i64;
        let first = ks.len() as i64;
        let i = idx as usize;
        let reuse = match prev {
            Some(pv) if aligned => {
                if i >= pv.st.n
                    || pv.st.int(i, I_KIND) != kind
                    || pv.st.int(i, I_COUNT) != cnt
                    || pv.st.int(i, I_PARENT) != parent
                {
                    aligned = false;
                    false
                } else {
                    own_eq(&node, &pv.nodes[i]) && lifted == pv.glifts[i]
                }
            }
            _ => false,
        };
        let mut spelled: Option<String> = None;
        let keyed = match prev {
            Some(pv) if !reuse => {
                let path = spell();
                let gk = gkey_of(&node, &path);
                let at = by_key.get_or_insert_with(|| {
                    let mut m = HashMap::default();
                    for (j, k) in pv.gkeys[..pv.st.n].iter().enumerate() {
                        if let Value::String(k) = k {
                            m.insert(k.as_str(), j);
                        }
                    }
                    m
                });
                match at.get(gk.as_str()) {
                    Some(&j)
                        if pv.st.int(j, I_KIND) == kind
                            && pv.st.int(j, I_COUNT) == cnt
                            && own_eq(&node, &pv.nodes[j])
                            && lifted == pv.glifts[j] =>
                    {
                        Some((j, path))
                    }
                    _ => {
                        spelled = Some(path);
                        None
                    }
                }
            }
            _ => None,
        };
        if reuse {
            let pv = prev.expect("reuse implies a last frame");
            ints.extend_from_slice(&pv.st.ints[i * FI..(i + 1) * FI]);
            ints[i * FI + I_FIRST] = first;
            floats.extend_from_slice(&pv.st.floats[i * FF..(i + 1) * FF]);
            gkeys.push(pv.gkeys[i].clone());
            paths.push(pv.paths[i].clone());
            changed.push(Value::Integer(0));
        } else if let (Some((j, path)), Some(pv)) = (keyed, prev) {
            // its entry is its own, but where it is in the tree, and
            // whether it gives room (its children's roles), are this frame's
            ints.extend_from_slice(&pv.st.ints[j * FI..(j + 1) * FI]);
            ints[i * FI + I_FIRST] = first;
            ints[i * FI + I_PARENT] = parent;
            ints[i * FI + I_SHRINK] =
                (spec.shrinks.contains(role) && !rigid_group(spec, &node, p, role)) as i64;
            floats.extend_from_slice(&pv.st.floats[j * FF..(j + 1) * FF]);
            gkeys.push(pv.gkeys[j].clone());
            paths.push(st(&path));
            changed.push(Value::Integer(0));
        } else {
            let path = spelled.take().unwrap_or_else(spell);
            let (ie, fe) = node_entry(spec, ts, &node, p, role, kind, cnt, first, parent)?;
            ints.extend_from_slice(&ie);
            floats.extend_from_slice(&fe);
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
    let changed = match prev {
        Some(pv) if aligned && pv.st.n == n => {
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
    let id = keep_state(FlatState {
        n,
        ints: Arc::new(ints),
        floats: Arc::new(floats),
        kids: Arc::new(ks),
        geo: Mutex::new(None),
    });
    Ok(map(vec![
        ("n", Value::Integer(n as i64)),
        ("id", Value::Integer(id)),
        ("nodes", list(nodes)),
        ("paths", list(paths)),
        ("index", list(indexes)),
        ("gkeys", list(gkeys)),
        ("glifts", list(glifts)),
        ("wraps", list(wraps)),
        ("changed", changed),
    ]))
}

// ── the arrays the engine keeps ──────────────────────────────────────

/// One answer's arrays, kept by the engine under the answer's `id`, and
/// the boxes the core computed over them (`gui.flat_keep`).
struct FlatState {
    n: usize,
    ints: Arc<Vec<i64>>,
    floats: Arc<Vec<f64>>,
    kids: Arc<Vec<i64>>,
    geo: Mutex<Option<Arc<Vec<f64>>>>,
}

impl FlatState {
    fn int(&self, i: usize, f: usize) -> i64 {
        self.ints[i * FI + f]
    }
}

/// The last answers' arrays, newest last. A layout reads its own last
/// frame and this frame, so a few windows' worth is plenty; an answer
/// whose arrays were let go is laid out afresh, never wrongly.
struct Kept {
    next: i64,
    states: VecDeque<(i64, Arc<FlatState>)>,
}

// A frame lays out its windows' pages and, on its own, each row of a
// virtual list it had not laid out before (a page down: a screenful):
// enough room that last frame's pages outlive a frame of new rows.
const KEPT_STATES: usize = 256;

fn kept() -> &'static Mutex<Kept> {
    static KEPT: OnceLock<Mutex<Kept>> = OnceLock::new();
    KEPT.get_or_init(|| {
        Mutex::new(Kept {
            next: 1,
            states: VecDeque::new(),
        })
    })
}

fn keep_state(s: FlatState) -> i64 {
    let mut k = match kept().lock() {
        Ok(k) => k,
        Err(poisoned) => poisoned.into_inner(),
    };
    let id = k.next;
    k.next += 1;
    k.states.push_back((id, Arc::new(s)));
    while k.states.len() > KEPT_STATES {
        k.states.pop_front();
    }
    id
}

/// The arrays an answer's `id` names, while the engine keeps them.
fn state_of(answer: &crate::ast::ValueMap) -> Option<Arc<FlatState>> {
    match answer.get("id") {
        Some(Value::Integer(i)) => state_by_id(*i),
        _ => None,
    }
}

fn state_by_id(id: i64) -> Option<Arc<FlatState>> {
    let k = kept().lock().ok()?;
    k.states
        .iter()
        .rev()
        .find(|(i, _)| *i == id)
        .map(|(_, s)| s.clone())
}

fn geo_of(st: &FlatState) -> Option<Arc<Vec<f64>>> {
    st.geo.lock().ok().and_then(|g| g.clone())
}

fn id_arg(v: Option<&Value>, what: &str) -> Res<i64> {
    match v {
        Some(Value::Integer(i)) => Ok(*i),
        _ => Err(format!("{what}: expected an answer's id")),
    }
}

/// `gui.flat_arrays(id)`: `(I, F, kids)`, the arrays the engine keeps for
/// answer `id`, or `()` once it let them go. In olang's VM they arrive as
/// typed lists sharing the engine's memory (`flat_vm_native`); here, on
/// the tree-walker, as lists.
pub fn gui_flat_arrays(args: Vec<Value>) -> Res<Value> {
    let id = id_arg(args.first(), "gui.flat_arrays")?;
    Ok(match state_by_id(id) {
        Some(st) => tuple(vec![
            list(st.ints.iter().map(|&i| Value::Integer(i)).collect()),
            list(st.floats.iter().map(|&f| Value::Float(f)).collect()),
            list(st.kids.iter().map(|&i| Value::Integer(i)).collect()),
        ]),
        None => Value::Unit,
    })
}

/// `gui.flat_keep(id, geo)`: the core's boxes for answer `id` (x, y, w, h
/// a node), kept beside its arrays for `gui.flat_emit` and the next
/// frame's `gui.flat_geo`.
pub fn gui_flat_keep(args: Vec<Value>) -> Res<Value> {
    let id = id_arg(args.first(), "gui.flat_keep")?;
    let geo = nums_of(args.get(1).unwrap_or(&Value::Unit), "the boxes")?;
    keep_geo(id, Arc::new(geo));
    Ok(Value::Unit)
}

fn keep_geo(id: i64, geo: Arc<Vec<f64>>) {
    if let Some(st) = state_by_id(id)
        && let Ok(mut g) = st.geo.lock()
    {
        *g = Some(geo);
    }
}

/// `gui.flat_geo(id)`: the boxes kept for answer `id`, or `()`.
pub fn gui_flat_geo(args: Vec<Value>) -> Res<Value> {
    let id = id_arg(args.first(), "gui.flat_geo")?;
    Ok(match state_by_id(id).and_then(|st| geo_of(&st)) {
        Some(g) => list(g.iter().map(|&f| Value::Float(f)).collect()),
        None => Value::Unit,
    })
}

/// The VM's own flat layout calls. `gui.flat_arrays`, `gui.flat_keep`,
/// and `gui.flat_geo`: the arrays and boxes cross as typed lists that
/// share the engine's memory — no list of numbers is boxed or copied
/// either way. `gui.flatten`, `gui.flat_emit`, `gui.flat_join`: their
/// answers in the VM's form (`vm_flatten`, `vm_flat_emit`,
/// `vm_flat_join`), the placements read natively. `None` for any other
/// name, and for arguments of another shape (the call then goes the
/// usual way).
pub fn flat_vm_native(name: &str, args: &[OvmValue]) -> Option<Result<OvmValue, String>> {
    match name {
        "gui.flatten" => return vm_flatten(args),
        "gui.flat_emit" => return vm_flat_emit(args),
        "gui.flat_join" => return vm_flat_join(args),
        _ => {}
    }
    let id = match args.first().map(|a| &a.data) {
        Some(ValueData::Integer(i)) => *i,
        _ => return None,
    };
    match name {
        "gui.flat_arrays" => Some(Ok(match state_by_id(id) {
            Some(st) => OvmValue::new_tuple(vec![
                OvmValue {
                    data: ValueData::IntList(st.ints.clone()),
                },
                OvmValue {
                    data: ValueData::FloatList(st.floats.clone()),
                },
                OvmValue {
                    data: ValueData::IntList(st.kids.clone()),
                },
            ]),
            None => OvmValue::new_unit(),
        })),
        "gui.flat_keep" => match args.get(1).map(|a| &a.data) {
            Some(ValueData::FloatList(g)) => {
                keep_geo(id, g.clone());
                Some(Ok(OvmValue::new_unit()))
            }
            _ => None,
        },
        "gui.flat_geo" => Some(Ok(match state_by_id(id).and_then(|st| geo_of(&st)) {
            Some(g) => OvmValue {
                data: ValueData::FloatList(g),
            },
            None => OvmValue::new_unit(),
        })),
        _ => None,
    }
}

/// Last frame's answer from `gui.flatten`: the arrays the engine kept
/// for it, and its lists.
struct Prev<'a> {
    st: Arc<FlatState>,
    nodes: &'a [Value],
    paths: &'a [Value],
    gkeys: &'a [Value],
    glifts: &'a [Value],
    paths_v: &'a Value,
    index_v: &'a Value,
    gkeys_v: &'a Value,
    glifts_v: &'a Value,
    wraps_v: &'a Value,
}

impl<'a> Prev<'a> {
    fn read(v: Option<&'a Value>) -> Option<Prev<'a>> {
        let f = fields(v?)?;
        let st = state_of(f)?;
        let n = st.n;
        let l = |k: &str| match f.get(k) {
            Some(Value::List(l)) if l.len() >= n => Some(&l[..]),
            _ => None,
        };
        Some(Prev {
            nodes: l("nodes")?,
            paths: l("paths")?,
            gkeys: l("gkeys")?,
            glifts: l("glifts")?,
            paths_v: f.get("paths")?,
            index_v: f.get("index")?,
            gkeys_v: f.get("gkeys")?,
            glifts_v: f.get("glifts")?,
            wraps_v: f.get("wraps")?,
            st,
        })
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
                    && match (x.get("props"), y.get("props")) {
                        (Some(Value::Map(a)), Some(Value::Map(b))) => Arc::ptr_eq(a, b) || a == b,
                        (a, b) => a == b,
                    })
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

/// What a node says of itself on its own placement (not on the parts
/// placed for it, a list's body or a split's divider): its `description`,
/// and its assistive `actions` (`[#{ label, … }]`) as their labels.
fn with_own(op: &mut crate::ast::ValueMap, gkey: &str, node: &Value) {
    let own = match node {
        Value::Map(m) => matches!(m.get("key"), Some(Value::String(k)) if k.as_str() == gkey),
        _ => false,
    };
    if !own {
        return;
    }
    let p = props_of(node);
    if let Value::String(d) = mget(p, "description")
        && !op.contains_key("description")
    {
        op.insert("description".into(), Value::String(d.clone()));
    }
    // its assistive actions: `actions`, or else a context menu given as a
    // list (its items with a label, outside submenus, enabled or not, in
    // order: the program maps an action's index back the same way)
    let acts = match mget(p, "actions") {
        Value::List(a) => Some(a),
        _ => match mget(p, "context") {
            Value::List(c) => Some(c),
            _ => None,
        },
    };
    if let Some(acts) = acts
        && !op.contains_key("actions")
    {
        let labels: Vec<Value> = acts
            .iter()
            .filter_map(|a| match a {
                Value::Map(m) if !m.contains_key("submenu") => match m.get("label") {
                    Some(Value::String(l)) => Some(Value::String(l.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        op.insert("actions".into(), list(labels));
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
    let mut op = op;
    with_own(&mut op, gkey, node);
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

/// A node's `a11y_role`, or `default` when it gives none. A role the
/// engine does not know would make `gui.apply` refuse the whole patch, so
/// the frame drew nothing new and nothing said why: here it is the
/// default instead, with a problem that names the node.
fn checked_role(
    p: Option<&crate::ast::ValueMap>,
    default: &str,
    gk: &str,
    pr: &mut Vec<Value>,
) -> Value {
    match mget(p, "a11y_role") {
        Value::Unit => st(default),
        Value::String(r) if super::scene::ROLES.contains(&r.as_str()) => Value::String(r.clone()),
        other => {
            pr.push(st(&format!(
                "node \"{gk}\" has the a11y_role {other}, which is not a role; it is presented as \"{default}\" (the roles are {})",
                super::scene::ROLES.join(", ")
            )));
            st(default)
        }
    }
}

/// `gui.roles()`: the roles a node may have, as the engine knows them.
pub fn gui_roles(_args: Vec<Value>) -> Res<Value> {
    Ok(list(super::scene::ROLES.iter().map(|r| st(r)).collect()))
}

fn group_op(
    spec: &Spec,
    p: Option<&crate::ast::ValueMap>,
    default_role: &str,
) -> crate::ast::ValueMap {
    let dr = st(default_role);
    let mut op = hm(vec![
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
    ]);
    // `inert`: drawn and said, never under the pointer (a stand-in for a
    // shape on a canvas, placed over it)
    if is_true(mget(p, "inert")) {
        op.insert("inert".into(), Value::Boolean(true));
    }
    // a group presented as a tab, a radio, a switch or a button says its
    // state as a leaf does: chosen, checked, its value, unavailable (the
    // recursive layout carried these; the flat core dropped them, and
    // VoiceOver never heard a section's tab was the one chosen)
    for k in ["selected", "checked", "disabled", "expanded"] {
        match mget(p, k) {
            Value::Unit => {}
            Value::Boolean(b) => {
                op.insert(k.into(), Value::Boolean(*b));
            }
            v if k == "selected" || k == "disabled" || k == "expanded" => {
                op.insert(k.into(), Value::Boolean(is_true(v)));
            }
            v => {
                op.insert(k.into(), v.clone());
            }
        }
    }
    op
}

// ── what the placements are read from, in either tier's form ─────────

/// A list's items as the interpreter holds them, or as olang's VM does:
/// last frame's placements come back from Loom in the form they were
/// answered in (the VM's own lists, when the VM asked).
#[derive(Clone, Copy)]
enum Items<'a> {
    Ast(&'a [Value]),
    Vm(&'a [OvmValue]),
}

impl<'a> Items<'a> {
    fn len(&self) -> usize {
        match self {
            Items::Ast(l) => l.len(),
            Items::Vm(l) => l.len(),
        }
    }

    /// Item `i` as a string, when it is one.
    fn str_at(&self, i: usize) -> Option<&'a str> {
        match self {
            Items::Ast(l) => match l.get(i) {
                Some(Value::String(s)) => Some(s.as_str()),
                _ => None,
            },
            Items::Vm(l) => match l.get(i).map(|v| &v.data) {
                Some(ValueData::String(s)) => Some(s.as_str()),
                _ => None,
            },
        }
    }

    /// Item `i` is `v` (`!=` as olang says it of two gkeys).
    fn same_as(&self, i: usize, v: &Value) -> bool {
        match (self, v) {
            (Items::Ast(l), _) => l.get(i) == Some(v),
            (Items::Vm(_), Value::String(s)) => self.str_at(i) == Some(s.as_str()),
            (Items::Vm(l), _) => l
                .get(i)
                .and_then(|x| x.to_ast().ok())
                .is_some_and(|x| x == *v),
        }
    }

    /// Item `i` in the interpreter's form.
    fn ast(&self, i: usize) -> Value {
        match self {
            Items::Ast(l) => l.get(i).cloned().unwrap_or(Value::Unit),
            Items::Vm(l) => l
                .get(i)
                .and_then(|x| x.to_ast().ok())
                .unwrap_or(Value::Unit),
        }
    }

    /// Item `i` in the VM's form.
    fn vm(&self, i: usize) -> OvmValue {
        match self {
            Items::Ast(l) => l
                .get(i)
                .map(|v| OvmValue::from_ast(v.clone()))
                .unwrap_or_else(OvmValue::new_unit),
            Items::Vm(l) => l.get(i).cloned().unwrap_or_else(OvmValue::new_unit),
        }
    }
}

/// A list of integers as either tier holds it (the VM's typed, or boxed).
#[derive(Clone, Copy)]
enum Ints<'a> {
    Ast(&'a [Value]),
    Raw(&'a [i64]),
    Vm(&'a [OvmValue]),
}

impl Ints<'_> {
    fn len(&self) -> usize {
        match self {
            Ints::Ast(l) => l.len(),
            Ints::Raw(l) => l.len(),
            Ints::Vm(l) => l.len(),
        }
    }

    fn at(&self, i: usize) -> Option<i64> {
        match self {
            Ints::Ast(l) => match l.get(i)? {
                Value::Integer(v) => Some(*v),
                Value::Float(f) => Some(*f as i64),
                _ => None,
            },
            Ints::Raw(l) => l.get(i).copied(),
            Ints::Vm(l) => match &l.get(i)?.data {
                ValueData::Integer(v) => Some(*v),
                ValueData::Float(f) => Some(*f as i64),
                _ => None,
            },
        }
    }

    fn get(&self, i: usize) -> i64 {
        self.at(i).unwrap_or(0)
    }
}

/// A flatten answer, read: its arrays (kept by the engine) and its lists.
struct FlatIn<'a> {
    n: usize,
    st: Arc<FlatState>,
    nodes: &'a [Value],
    paths: &'a [Value],
    index: Ints<'a>,
    gkeys: &'a [Value],
    glifts: &'a [Value],
}

impl<'a> FlatIn<'a> {
    fn of_ast(flat: &'a crate::ast::ValueMap) -> Res<FlatIn<'a>> {
        let n = match flat.get("n") {
            Some(Value::Integer(n)) => *n as usize,
            _ => return Err("gui.flat_emit: the arrays have no \"n\"".into()),
        };
        let st = state_of(flat).ok_or(
            "gui.flat_emit: the engine no longer holds this answer's arrays; lay the view out again",
        )?;
        Ok(FlatIn {
            n,
            st,
            nodes: list_of(flat, "nodes")?,
            paths: list_of(flat, "paths")?,
            index: Ints::Ast(list_of(flat, "index")?),
            gkeys: list_of(flat, "gkeys")?,
            glifts: list_of(flat, "glifts")?,
        })
    }

    /// The VM's form of an answer (`vm_flatten`'s, or one wrapped); `None`
    /// for anything else (the call then goes the usual way).
    fn of_vm(flat: &'a OvmValue) -> Option<FlatIn<'a>> {
        let n = match vm_field(flat, "n")? {
            Fld::Vm(OvmValue {
                data: ValueData::Integer(n),
            })
            | Fld::Ast(Value::Integer(n)) => *n as usize,
            _ => return None,
        };
        let id = match vm_field(flat, "id")? {
            Fld::Vm(OvmValue {
                data: ValueData::Integer(i),
            })
            | Fld::Ast(Value::Integer(i)) => *i,
            _ => return None,
        };
        let l = |k: &str| vm_field(flat, k).and_then(|f| f.ast_items());
        Some(FlatIn {
            n,
            st: state_by_id(id)?,
            nodes: l("nodes")?,
            paths: l("paths")?,
            index: vm_field(flat, "index")?.ints()?,
            gkeys: l("gkeys")?,
            glifts: l("glifts")?,
        })
    }
}

/// Last frame's placements as Loom completed them (`flat_emit`'s `prev`).
struct PrevOut<'a> {
    recs: Items<'a>,
    gkeys: Items<'a>,
    probs: Items<'a>,
    lifts: Items<'a>,
}

static NO_PREV: PrevOut<'static> = PrevOut {
    recs: Items::Ast(EMPTY),
    gkeys: Items::Ast(EMPTY),
    probs: Items::Ast(EMPTY),
    lifts: Items::Ast(EMPTY),
};

impl<'a> PrevOut<'a> {
    fn of_ast(m: &'a crate::ast::ValueMap) -> Res<PrevOut<'a>> {
        Ok(PrevOut {
            recs: Items::Ast(list_of(m, "recs")?),
            gkeys: Items::Ast(list_of(m, "gkeys")?),
            probs: Items::Ast(list_of(m, "probs")?),
            lifts: Items::Ast(list_of(m, "lifts")?),
        })
    }

    fn of_vm(m: &'a OvmValue) -> Option<PrevOut<'a>> {
        let l = |k: &str| vm_field(m, k).and_then(|f| f.items());
        Some(PrevOut {
            recs: l("recs")?,
            gkeys: l("gkeys")?,
            probs: l("probs")?,
            lifts: l("lifts")?,
        })
    }

    fn all_len(&self, n: usize) -> bool {
        self.recs.len() == n && self.gkeys.len() == n && self.probs.len() == n && self.lifts.len() == n
    }
}

/// A field of a map the VM holds: a VM map's own, or a wrapped map's.
enum Fld<'a> {
    Vm(&'a OvmValue),
    Ast(&'a Value),
}

fn vm_field<'a>(m: &'a OvmValue, k: &str) -> Option<Fld<'a>> {
    match &m.data {
        ValueData::Map(mm) => mm.get(k).map(Fld::Vm),
        ValueData::AstMap(mm) | ValueData::AstStruct(mm, _) => mm.get(k).map(Fld::Ast),
        _ => None,
    }
}

impl<'a> Fld<'a> {
    /// A list of interpreter values, borrowed (a wrapped list).
    fn ast_items(&self) -> Option<&'a [Value]> {
        match self {
            Fld::Vm(v) => match &v.data {
                ValueData::AstList(l) => Some(&l[..]),
                _ => None,
            },
            Fld::Ast(Value::List(l)) => Some(&l[..]),
            Fld::Ast(_) => None,
        }
    }

    fn items(&self) -> Option<Items<'a>> {
        match self {
            Fld::Vm(v) => match &v.data {
                ValueData::List(l) => Some(Items::Vm(&l[..])),
                ValueData::AstList(l) => Some(Items::Ast(&l[..])),
                _ => None,
            },
            Fld::Ast(Value::List(l)) => Some(Items::Ast(&l[..])),
            Fld::Ast(_) => None,
        }
    }

    fn ints(&self) -> Option<Ints<'a>> {
        match self {
            Fld::Vm(v) => match &v.data {
                ValueData::IntList(l) => Some(Ints::Raw(&l[..])),
                ValueData::List(l) => Some(Ints::Vm(&l[..])),
                ValueData::AstList(l) => Some(Ints::Ast(&l[..])),
                _ => None,
            },
            Fld::Ast(Value::List(l)) => Some(Ints::Ast(&l[..])),
            Fld::Ast(_) => None,
        }
    }
}

// ── gui.flat_emit ────────────────────────────────────────────────────

/// A node's placements: last frame's (`j` in its lists, its first
/// placement's path made `path` when that changed), or made now.
enum Placed<'a> {
    Kept {
        j: usize,
        path: Option<&'a Value>,
    },
    Made {
        recs: Vec<Value>,
        probs: Vec<Value>,
        gkey: Value,
    },
}

/// What `emit` worked out: each node's placements, what comes before
/// each (a split's divider), and the leaves left for Loom.
struct Emitted<'a> {
    placed: Vec<Placed<'a>>,
    pre: Vec<Vec<Value>>,
    todo: Vec<i64>,
}

/// Each node's placements from the core's boxes `geo`. `cand[i]` is the
/// placement last frame's lists hold for node i (by index, or by key),
/// taken unless its parent's gkey changed (`renames`: last frame's gkeys,
/// by index, when that is to be judged).
fn emit<'a>(
    spec: &Spec,
    inp: &FlatIn<'a>,
    geo: &[f64],
    cand: &[Option<(usize, Option<&'a Value>)>],
    prev: Option<&PrevOut<'a>>,
    renames: Option<Items<'a>>,
) -> Res<Emitted<'a>> {
    let n = inp.n;
    let held = &inp.st;
    let (iv, kids): (&[i64], &[i64]) = (&held.ints, &held.kids);
    let fv: &[f64] = &held.floats;
    let (nodes, paths, indexes, fgkeys) = (inp.nodes, inp.paths, inp.index, inp.gkeys);
    if iv.len() < n * FI
        || geo.len() < n * 4
        || nodes.len() < n
        || paths.len() < n
        || indexes.len() < n
        || fgkeys.len() < n
        || inp.glifts.len() < n
    {
        return Err("gui.flat_emit: the arrays and the boxes disagree on the node count".into());
    }
    let accent = spec.tok_of("accent");
    let hairline = spec.tok_of("hairline");
    let mut placed: Vec<Placed<'a>> = Vec::with_capacity(n);
    // each node's gkey as the interpreter holds it, for its children
    let mut gk_of: Vec<Value> = Vec::with_capacity(n);
    let mut pre: Vec<Vec<Value>> = vec![Vec::new(); n];
    let mut renamed = vec![false; n];
    let mut todo: Vec<i64> = Vec::new();
    let g = |i: usize, f: usize| iv[i * FI + f];
    for i in 0..n {
        let pi = g(i, I_PARENT);
        let kind = g(i, I_KIND);
        if let Some(Some((j, path))) = cand.get(i)
            && (pi < 0 || !renamed[pi as usize])
            && let Some(prev) = prev
        {
            gk_of.push(match renames {
                Some(_) => prev.gkeys.ast(*j),
                None => fgkeys[i].clone(),
            });
            placed.push(Placed::Kept { j: *j, path: *path });
            continue;
        }
        let node = &nodes[i];
        let role = role_of(node);
        let p = props_of(node);
        let path = match &paths[i] {
            Value::String(s) => s.as_str(),
            _ => "",
        };
        let index = indexes.get(i);
        let gk = match &fgkeys[i] {
            Value::String(s) => s.as_str().to_string(),
            other => other.to_string(),
        };
        if let Some(pg) = renames
            && !pg.same_as(i, &fgkeys[i])
        {
            renamed[i] = true;
        }
        let parent = if pi < 0 {
            Value::Unit
        } else {
            gk_of[pi as usize].clone()
        };
        let b = (geo[i * 4], geo[i * 4 + 1], geo[i * 4 + 2], geo[i * 4 + 3]);
        let origin = if pi < 0 {
            (0.0, 0.0)
        } else {
            (geo[pi as usize * 4], geo[pi as usize * 4 + 1])
        };
        let mut mine: Vec<Value> = Vec::new();
        let mut pr: Vec<Value> = Vec::new();
        let mut is_placed = true;
        match kind {
            0 => {
                // a group with a `msg` is one control: a button unless it
                // says otherwise, focusable unless disabled
                let active = *mget(p, "msg") != Value::Unit;
                let disabled = is_true(mget(p, "disabled"));
                let default_role = if active { "button" } else { "group" };
                let mut op = group_op(spec, p, default_role);
                op.insert("role".into(), checked_role(p, default_role, &gk, &mut pr));
                if active {
                    op.insert("disabled".into(), Value::Boolean(disabled));
                    // the engine takes the focus to it whatever role it presents
                    op.insert("focusable".into(), Value::Boolean(!disabled));
                }
                mine.push(rec(
                    &gk,
                    &parent,
                    index,
                    b,
                    origin,
                    node,
                    op,
                    active && !disabled,
                    path,
                ));
            }
            1 => {
                let mut op = group_op(spec, p, "group");
                op.insert("role".into(), checked_role(p, "region", &gk, &mut pr));
                op.insert("scroll".into(), Value::Boolean(true));
                // `focusable`: the keys scroll it (a picture at its own size)
                let keys = is_true(mget(p, "focusable"));
                if keys {
                    op.insert("focusable".into(), Value::Boolean(true));
                }
                mine.push(rec(&gk, &parent, index, b, origin, node, op, keys, path));
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
                let (bar, grab) = (split_bar(p), 24.0);
                let reach = (grab - bar) / 2.0;
                // the line sits in the bar (a hairline bar is the line)
                let mid = if bar <= 1.0 { reach } else { reach + bar / 2.0 };
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
                    (d.0 + mid, d.1, 1.0, d.3)
                } else {
                    (d.0, d.1 + mid, d.2, 1.0)
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
                    pre[kids[first + 1] as usize] = vec![div, line];
                } else {
                    mine.push(div);
                    mine.push(line);
                }
            }
            _ if spec.placed.contains(role) => match role {
                "text" | "heading" => {
                    let fsv = font_style_v(spec, p, role);
                    let wrap = is_true(mget(p, "wrap"));
                    let op = hm(vec![
                        ("role", st(role)),
                        ("text", mget(p, "text").clone()),
                        ("style", with_style(spec, fsv.clone(), mget(p, "style"))),
                        ("level", mget(p, "level").clone()),
                        ("name", mget(p, "name").clone()),
                    ]);
                    // a text that does not wrap is one line at any width:
                    // its height is its natural one
                    let nh = if wrap {
                        let mut ts = text::system().lock().map_err(|_| "gui: text poisoned")?;
                        measure(spec, &mut ts, &text_of(mget(p, "text")), &fsv, b.2)?.1
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
                        ("style", with_style(spec, look, mget(p, "style"))),
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
                _ => is_placed = false,
            },
            _ => is_placed = false,
        }
        if !is_placed {
            todo.push(i as i64);
        }
        let gkv = st(&gk);
        gk_of.push(gkv.clone());
        placed.push(Placed::Made {
            recs: mine,
            probs: pr,
            gkey: gkv,
        });
    }
    Ok(Emitted { placed, pre, todo })
}

/// `rs` (a node's placements) with its first one's path `path`, as Loom's
/// keyed keep wrote it (`map_set(rs[0], "path", …)`).
fn with_first_path(rs: Value, path: &Value) -> Value {
    match &rs {
        Value::List(l) if !l.is_empty() => {
            let mut items = l.as_ref().clone();
            if let Value::Map(m) = &items[0] {
                let mut m2 = m.as_ref().clone();
                m2.insert("path".to_string(), path.clone());
                items[0] = Value::Map(Arc::new(m2));
            }
            Value::List(Arc::new(items))
        }
        _ => rs,
    }
}

/// `with_first_path` on the VM's form.
fn with_first_path_vm(rs: OvmValue, path: &Value) -> OvmValue {
    let ValueData::List(l) = &rs.data else {
        return rs;
    };
    if l.is_empty() {
        return rs;
    }
    let mut items = l.as_ref().clone();
    items[0] = match &items[0].data {
        ValueData::Map(m) => {
            let mut m2 = m.as_ref().clone();
            m2.insert("path".to_string(), OvmValue::from_ast(path.clone()));
            OvmValue::new_map(Arc::new(m2))
        }
        ValueData::AstMap(m) => {
            let mut m2 = m.as_ref().clone();
            m2.insert("path".to_string(), path.clone());
            OvmValue {
                data: ValueData::AstMap(Arc::new(m2)),
            }
        }
        _ => items[0].clone(),
    };
    OvmValue::new_list(items)
}

impl Emitted<'_> {
    /// The answer in the interpreter's form.
    fn to_ast(self, inp: &FlatIn, prev: Option<&PrevOut>, kept_gkeys_from_prev: bool) -> Value {
        let n = self.placed.len();
        let mut recs = Vec::with_capacity(n);
        let mut gkeys = Vec::with_capacity(n);
        let mut probs = Vec::with_capacity(n);
        let mut lifts = Vec::with_capacity(n);
        for (i, p) in self.placed.into_iter().enumerate() {
            match p {
                // (kept only with last frame's lists at hand)
                Placed::Kept { j, path } => {
                    let pv = prev.unwrap_or(&NO_PREV);
                    let rs = pv.recs.ast(j);
                    recs.push(match path {
                        Some(path) => with_first_path(rs, path),
                        None => rs,
                    });
                    gkeys.push(if kept_gkeys_from_prev {
                        pv.gkeys.ast(j)
                    } else {
                        inp.gkeys[i].clone()
                    });
                    probs.push(pv.probs.ast(j));
                    lifts.push(pv.lifts.ast(j));
                }
                Placed::Made {
                    recs: r,
                    probs: pr,
                    gkey,
                } => {
                    recs.push(list(r));
                    gkeys.push(gkey);
                    probs.push(list(pr));
                    lifts.push(inp.glifts[i].clone());
                }
            }
        }
        let empty = list(Vec::new());
        let pre: Vec<Value> = self
            .pre
            .into_iter()
            .map(|p| if p.is_empty() { empty.clone() } else { list(p) })
            .collect();
        map(vec![
            ("recs", list(recs)),
            ("gkeys", list(gkeys)),
            ("probs", list(probs)),
            ("lifts", list(lifts)),
            ("pre", list(pre)),
            (
                "todo",
                list(self.todo.into_iter().map(Value::Integer).collect()),
            ),
        ])
    }

    /// The answer in the VM's form: placements Loom reads natively — a
    /// map's field a lookup, not a conversion a read.
    fn to_vm(self, inp: &FlatIn, prev: Option<&PrevOut>, kept_gkeys_from_prev: bool) -> OvmValue {
        let n = self.placed.len();
        let mut recs = Vec::with_capacity(n);
        let mut gkeys = Vec::with_capacity(n);
        let mut probs = Vec::with_capacity(n);
        let mut lifts = Vec::with_capacity(n);
        for (i, p) in self.placed.into_iter().enumerate() {
            match p {
                Placed::Kept { j, path } => {
                    let pv = prev.unwrap_or(&NO_PREV);
                    let rs = pv.recs.vm(j);
                    recs.push(match path {
                        Some(path) => with_first_path_vm(rs, path),
                        None => rs,
                    });
                    gkeys.push(if kept_gkeys_from_prev {
                        pv.gkeys.vm(j)
                    } else {
                        OvmValue::from_ast(inp.gkeys[i].clone())
                    });
                    probs.push(pv.probs.vm(j));
                    lifts.push(pv.lifts.vm(j));
                }
                Placed::Made {
                    recs: r,
                    probs: pr,
                    gkey,
                } => {
                    recs.push(OvmValue::new_list(r.iter().map(vm_rec).collect()));
                    gkeys.push(OvmValue::from_ast(gkey));
                    probs.push(OvmValue::new_list(
                        pr.into_iter().map(OvmValue::from_ast).collect(),
                    ));
                    lifts.push(vm_lifts(&inp.glifts[i]));
                }
            }
        }
        let empty = OvmValue::new_list(Vec::new());
        let pre: Vec<OvmValue> = self
            .pre
            .into_iter()
            .map(|p| {
                if p.is_empty() {
                    empty.clone()
                } else {
                    OvmValue::new_list(p.iter().map(vm_rec).collect())
                }
            })
            .collect();
        vm_map(vec![
            ("recs", OvmValue::new_list(recs)),
            ("gkeys", OvmValue::new_list(gkeys)),
            ("probs", OvmValue::new_list(probs)),
            ("lifts", OvmValue::new_list(lifts)),
            ("pre", OvmValue::new_list(pre)),
            (
                "todo",
                OvmValue::new_list(self.todo.into_iter().map(OvmValue::new_integer).collect()),
            ),
        ])
    }
}

fn vm_map(fields: Vec<(&str, OvmValue)>) -> OvmValue {
    let mut m = crate::ovm::value::OvmMap::with_capacity_and_hasher(fields.len(), Default::default());
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    OvmValue::new_map(Arc::new(m))
}

/// A value the engine built, in the VM's own collections all the way
/// down (a placement's op, its style): read in place, never converted.
fn vm_deep(v: &Value) -> OvmValue {
    match v {
        Value::Map(m) => {
            let mut out =
                crate::ovm::value::OvmMap::with_capacity_and_hasher(m.len(), Default::default());
            for (k, x) in m.iter() {
                out.insert(k.clone(), vm_deep(x));
            }
            OvmValue::new_map(Arc::new(out))
        }
        Value::List(l) => OvmValue::new_list(l.iter().map(vm_deep).collect()),
        Value::Tuple(t) => OvmValue::new_tuple(t.iter().map(vm_deep).collect()),
        other => OvmValue::from_ast(other.clone()),
    }
}

/// A placement in the VM's form: its fields the VM's own, but its view
/// node (`node`, the program's, read a prop at a time) wrapped as it is.
fn vm_rec(v: &Value) -> OvmValue {
    match v {
        Value::Map(m) => {
            let mut out =
                crate::ovm::value::OvmMap::with_capacity_and_hasher(m.len(), Default::default());
            for (k, x) in m.iter() {
                out.insert(
                    k.clone(),
                    if k == "node" {
                        OvmValue::from_ast(x.clone())
                    } else {
                        vm_deep(x)
                    },
                );
            }
            OvmValue::new_map(Arc::new(out))
        }
        other => OvmValue::from_ast(other.clone()),
    }
}

/// A node's lifted layers (view nodes) in the VM's form: the list the
/// VM's own, each node wrapped.
fn vm_lifts(v: &Value) -> OvmValue {
    match v {
        Value::List(l) => OvmValue::new_list(
            l.iter().map(|x| OvmValue::from_ast(x.clone())).collect(),
        ),
        other => OvmValue::from_ast(other.clone()),
    }
}

// ── keyed: what last frame placed that this frame keeps ───────────────

/// Last frame's flatten answer, for the keyed keep.
struct PrevFlat<'a> {
    st: Arc<FlatState>,
    nodes: &'a [Value],
    paths: &'a [Value],
    index: Ints<'a>,
    glifts: &'a [Value],
}

impl<'a> PrevFlat<'a> {
    fn of_ast(v: &'a Value) -> Option<PrevFlat<'a>> {
        let f = fields(v)?;
        let st = state_of(f)?;
        let l = |k: &str| match f.get(k) {
            Some(Value::List(l)) => Some(&l[..]),
            _ => None,
        };
        Some(PrevFlat {
            st,
            nodes: l("nodes")?,
            paths: l("paths")?,
            index: Ints::Ast(l("index")?),
            glifts: l("glifts")?,
        })
    }

    fn of_vm(v: &'a OvmValue) -> Option<PrevFlat<'a>> {
        let i = FlatIn::of_vm(v)?;
        Some(PrevFlat {
            st: i.st,
            nodes: i.nodes,
            paths: i.paths,
            index: i.index,
            glifts: i.glifts,
        })
    }
}

/// `a == b` as olang answers it, false where it raises (Loom's `ly_same`).
fn ly_same(a: &Value, b: &Value) -> bool {
    matches!(crate::interpreter::ops::search_eq(a, b), Ok(true))
}

fn field_or_unit<'a>(v: &'a Value, k: &str) -> &'a Value {
    match v {
        Value::Map(m) => m.get(k).unwrap_or(&UNIT.0),
        _ => &UNIT.0,
    }
}

/// Loom's `ly_own_same`: the same role, key and props.
fn ly_own_same(a: &Value, b: &Value) -> bool {
    ly_same(field_or_unit(a, "role"), field_or_unit(b, "role"))
        && ly_same(field_or_unit(a, "key"), field_or_unit(b, "key"))
        && ly_same(field_or_unit(a, "props"), field_or_unit(b, "props"))
}

/// Where each node's subtree ends in tree order (flex.ol's `subtree_ends`).
fn subtree_ends(n: usize, iv: &[i64], kids: &[i64]) -> Vec<usize> {
    let mut e = vec![0usize; n];
    for i in (0..n).rev() {
        let cnt = iv[i * FI + I_COUNT];
        e[i] = if cnt == 0 {
            i + 1
        } else {
            let last = kids[(iv[i * FI + I_FIRST] + cnt - 1) as usize] as usize;
            e[last]
        };
    }
    e
}

// A subtree this large or smaller is compared whole (Loom's LY_WHOLE_MAX).
const WHOLE_MAX: usize = 400;

/// Loom's keyed keep (`ly_keyed_keep`, lib/layout.ol), on a frame whose
/// shape changed: a node is kept when last frame had its key and it is
/// placed as it was — its own role, key and props the same, not placed
/// every frame, the same kind and number of children, the same box and
/// index under a parent with the same key and origin, the same layers
/// lifted. Subtrees that are what they were, at the same paths, are
/// judged in one run. Answers each node's source in last frame's lists
/// (and its first placement's new path, when that changed).
fn keyed_keep<'a>(
    inp: &FlatIn<'a>,
    geo: &[f64],
    pf: &PrevFlat<'a>,
    old: &[f64],
    ogk: Items<'a>,
) -> Vec<Option<(usize, Option<&'a Value>)>> {
    let n = inp.n;
    let on = pf.st.n;
    let mut cand: Vec<Option<(usize, Option<&'a Value>)>> = vec![None; n];
    let (iv, kids) = (&inp.st.ints[..], &inp.st.kids[..]);
    let (oi, okids) = (&pf.st.ints[..], &pf.st.kids[..]);
    if iv.len() < n * FI
        || oi.len() < on * FI
        || geo.len() < n * 4
        || old.len() < on * 4
        || ogk.len() < on
        || pf.nodes.len() < on
        || pf.paths.len() < on
        || pf.index.len() < on
        || pf.glifts.len() < on
    {
        return cand;
    }
    let gk = |i: usize| match &inp.gkeys[i] {
        Value::String(s) => Some(s.as_str()),
        _ => None,
    };
    let mut at: HashMap<&str, usize> = HashMap::default();
    for j in 0..on {
        if let Some(k) = ogk.str_at(j) {
            at.insert(k, j);
        }
    }
    let ends = subtree_ends(n, iv, kids);
    let oends = subtree_ends(on, oi, okids);
    let i_of = |i: usize, f: usize| iv[i * FI + f];
    let o_of = |j: usize, f: usize| oi[j * FI + f];
    let box_same = |i: usize, j: usize| (0..4).all(|c| geo[i * 4 + c] == old[j * 4 + c]);
    // a parent's origin unmoved (the roots: both roots)
    let origin_same = |pi: i64, pj: i64| {
        if pi < 0 {
            pj < 0
        } else {
            pj >= 0
                && geo[pi as usize * 4] == old[pj as usize * 4]
                && geo[pi as usize * 4 + 1] == old[pj as usize * 4 + 1]
        }
    };
    let gk_same = |i: usize, j: usize| gk(i).is_some() && gk(i) == ogk.str_at(j);
    let lifts_same = |i: usize, j: usize| {
        let empty = |v: &Value| matches!(v, Value::List(l) if l.is_empty());
        (empty(&inp.glifts[i]) && empty(&pf.glifts[j])) || ly_same(&inp.glifts[i], &pf.glifts[j])
    };
    let mut i = 0;
    while i < n {
        let j0 = gk(i).and_then(|k| at.get(k).copied());
        let size = ends[i] - i;
        // moved on the page: what it holds moved with it, and none of it
        // is kept
        let shifted = j0.is_some_and(|j| geo[i * 4] != old[j * 4] || geo[i * 4 + 1] != old[j * 4 + 1]);
        let whole = !shifted
            && j0.is_some_and(|j| {
                size > 1
                    && size <= WHOLE_MAX
                    && oends[j] - j == size
                    && ly_same(&inp.nodes[i], &pf.nodes[j])
            });
        let moved = match j0 {
            None => true,
            Some(j) => inp.paths[i] != pf.paths[j],
        };
        let run = whole && !moved;
        if run {
            let j0 = j0.unwrap_or(0);
            // what the arrays do not say of its first: its parent's key,
            // its index
            let pi = i_of(i, I_PARENT);
            let pj = o_of(j0, I_PARENT);
            let first_ok = (if pi < 0 {
                pj < 0
            } else {
                pj >= 0 && gk_same(pi as usize, pj as usize)
            }) && inp.index.at(i) == pf.index.at(j0);
            // kept_runs (lib/flex.ol)
            for d in 0..size {
                let (a, b) = (i + d, j0 + d);
                let v = i_of(a, I_FRESH) == 0
                    && o_of(b, I_KIND) == i_of(a, I_KIND)
                    && o_of(b, I_COUNT) == i_of(a, I_COUNT)
                    && box_same(a, b)
                    && origin_same(i_of(a, I_PARENT), o_of(b, I_PARENT));
                if v && (d > 0 || first_ok) {
                    cand[a] = Some((b, None));
                }
            }
        }
        let count = if run || shifted {
            0
        } else if whole {
            size
        } else {
            1
        };
        for d in 0..count {
            let ii = i + d;
            let Some(j) = j0.map(|j| j + d) else {
                continue;
            };
            let inner = whole && d > 0;
            let pi = i_of(ii, I_PARENT);
            let pj = o_of(j, I_PARENT);
            let same = i_of(ii, I_FRESH) == 0
                && o_of(j, I_KIND) == i_of(ii, I_KIND)
                && o_of(j, I_COUNT) == i_of(ii, I_COUNT)
                && box_same(ii, j)
                && origin_same(pi, pj)
                && (if inner {
                    gk_same(ii, j) && pi >= 0 && pj >= 0 && gk_same(pi as usize, pj as usize)
                } else {
                    (pi < 0 || (pj >= 0 && gk_same(pi as usize, pj as usize)))
                        && inp.index.at(ii) == pf.index.at(j)
                        && ly_own_same(&inp.nodes[ii], &pf.nodes[j])
                })
                && lifts_same(ii, j);
            if same {
                let path = (inp.paths[ii] != pf.paths[j]).then_some(&inp.paths[ii]);
                cand[ii] = Some((j, path));
            }
        }
        i += if run || shifted { size } else { count };
    }
    cand
}

/// The boxes for an answer: given (`geo`), else those kept for it.
fn boxes_for(held: &FlatState, given: Option<Vec<f64>>) -> Res<Arc<Vec<f64>>> {
    match given {
        Some(g) => Ok(Arc::new(g)),
        None => geo_of(held).ok_or_else(|| "gui.flat_emit: no boxes given or kept for this answer".to_string()),
    }
}

/// `gui.flat_emit(flat, geo, spec, keep, prev)`: each node's placements
/// from the core's boxes `geo` (x, y, w, h a node; `()`: those
/// `gui.flat_keep` kept), by index — `#{ recs, gkeys, probs, lifts, pre,
/// todo }`. A node `keep` marks (1) takes `prev`'s (last frame's answer,
/// as Loom completed it) unless its parent's gkey changed. With `keep`
/// `"keyed"` (a frame whose shape changed), `prev` is `#{ flat, out }`,
/// last frame's flatten answer and its placements, and the nodes kept are
/// worked out here by engine key (Loom's keyed keep: `gui.platform()`
/// says `flat_keyed`). Groups, regions, splits, and the leaves in
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
    let spec = Spec::read(&args[2])?;
    let inp = FlatIn::of_ast(flat)?;
    let given = match &args[1] {
        Value::Unit => None,
        v => Some(nums_of(v, "the boxes")?),
    };
    let geo = boxes_for(&inp.st, given)?;
    let n = inp.n;
    match (&args[3], &args[4]) {
        (Value::String(k), prev) if k.as_str() == "keyed" => {
            let pv = fields(prev);
            let pout = match pv.and_then(|m| m.get("out")) {
                Some(Value::Map(m)) => Some(PrevOut::of_ast(m)?),
                _ => None,
            };
            let pflat = pv.and_then(|m| m.get("flat")).and_then(PrevFlat::of_ast);
            let cand = match (&pout, &pflat) {
                (Some(po), Some(pf)) => match geo_of(&pf.st) {
                    Some(old) if po.all_len(pf.st.n) => keyed_keep(&inp, &geo, pf, &old, po.gkeys),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            let e = emit(&spec, &inp, &geo, &cand, pout.as_ref(), None)?;
            Ok(e.to_ast(&inp, pout.as_ref(), false))
        }
        (keep, prev) => {
            let keep = match keep {
                Value::Unit => None,
                v => Some(ints_of(v, "keep")?),
            };
            let prev = match (prev, &keep) {
                (Value::Map(m), Some(_)) => Some(PrevOut::of_ast(m)?),
                _ => None,
            };
            let reusable = prev.as_ref().is_some_and(|p| p.all_len(n));
            let cand: Vec<Option<(usize, Option<&Value>)>> = match (&keep, reusable) {
                (Some(k), true) => (0..n).map(|i| (k.get(i) == Some(&1)).then_some((i, None))).collect(),
                _ => Vec::new(),
            };
            let renames = if reusable { prev.as_ref().map(|p| p.gkeys) } else { None };
            let e = emit(&spec, &inp, &geo, &cand, prev.as_ref(), renames)?;
            Ok(e.to_ast(&inp, prev.as_ref(), true))
        }
    }
}

/// The VM's own `gui.flat_emit`: the flatten answer and last frame's
/// placements read where they are, the placements answered in the VM's
/// form. `None` for arguments of another shape, and for any error (the
/// call then goes the usual way, which raises it).
fn vm_flat_emit(args: &[OvmValue]) -> Option<Res<OvmValue>> {
    if args.len() != 5 {
        return None;
    }
    let inp = FlatIn::of_vm(&args[0])?;
    let spec_v = args[2].to_ast().ok()?;
    let spec = Spec::read(&spec_v).ok()?;
    let given = match &args[1].data {
        ValueData::Unit => None,
        ValueData::FloatList(g) => Some(g.as_ref().clone()),
        _ => return None,
    };
    let geo = boxes_for(&inp.st, given).ok()?;
    let n = inp.n;
    let keyed = matches!(&args[3].data, ValueData::String(k) if k.as_str() == "keyed");
    if keyed {
        let pout = match vm_field(&args[4], "out") {
            Some(Fld::Vm(m)) => PrevOut::of_vm(m),
            _ => None,
        };
        let pflat = match vm_field(&args[4], "flat") {
            Some(Fld::Vm(f)) => PrevFlat::of_vm(f),
            _ => None,
        };
        let cand = match (&pout, &pflat) {
            (Some(po), Some(pf)) => match geo_of(&pf.st) {
                Some(old) if po.all_len(pf.st.n) => keyed_keep(&inp, &geo, pf, &old, po.gkeys),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let e = emit(&spec, &inp, &geo, &cand, pout.as_ref(), None).ok()?;
        Some(Ok(e.to_vm(&inp, pout.as_ref(), false)))
    } else {
        let keep: Option<Ints> = match &args[3].data {
            ValueData::Unit => None,
            ValueData::IntList(k) => Some(Ints::Raw(&k[..])),
            ValueData::List(k) => Some(Ints::Vm(&k[..])),
            ValueData::AstList(k) => Some(Ints::Ast(&k[..])),
            _ => return None,
        };
        let prev = match (&args[4].data, &keep) {
            (ValueData::Map(_) | ValueData::AstMap(_), Some(_)) => Some(PrevOut::of_vm(&args[4])?),
            _ => None,
        };
        let reusable = prev.as_ref().is_some_and(|p| p.all_len(n));
        let cand: Vec<Option<(usize, Option<&Value>)>> = match (&keep, reusable) {
            (Some(k), true) => (0..n).map(|i| (k.at(i) == Some(1)).then_some((i, None))).collect(),
            _ => Vec::new(),
        };
        let renames = if reusable { prev.as_ref().map(|p| p.gkeys) } else { None };
        let e = emit(&spec, &inp, &geo, &cand, prev.as_ref(), renames).ok()?;
        Some(Ok(e.to_vm(&inp, prev.as_ref(), true)))
    }
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

/// The VM's own `gui.flat_join`: the lists joined in the VM's form (a
/// list the VM answered stays its own; a wrapped one's items convert).
/// `None` for a typed list (the call then goes the usual way).
fn vm_flat_join(args: &[OvmValue]) -> Option<Res<OvmValue>> {
    if args.len() != 4 {
        return None;
    }
    fn items(v: &OvmValue) -> Option<Option<Items<'_>>> {
        Some(match &v.data {
            ValueData::List(l) => Some(Items::Vm(&l[..])),
            ValueData::AstList(l) => Some(Items::Ast(&l[..])),
            ValueData::IntList(_) | ValueData::FloatList(_) => return None,
            _ => None,
        })
    }
    let join = |vs: &[Option<Items>]| -> OvmValue {
        let mut out: Vec<OvmValue> = Vec::new();
        let n = vs.iter().flatten().map(|l| l.len()).max().unwrap_or(0);
        for i in 0..n {
            for l in vs.iter().flatten() {
                match l {
                    Items::Vm(l) => match l.get(i).map(|x| &x.data) {
                        Some(ValueData::List(xs)) => out.extend(xs.iter().cloned()),
                        Some(ValueData::AstList(xs)) => {
                            out.extend(xs.iter().map(|x| OvmValue::from_ast(x.clone())))
                        }
                        _ => {}
                    },
                    Items::Ast(l) => {
                        if let Some(Value::List(xs)) = l.get(i) {
                            out.extend(xs.iter().map(|x| OvmValue::from_ast(x.clone())));
                        }
                    }
                }
            }
        }
        OvmValue::new_list(out)
    };
    let (pre, recs, probs, lifts) = (
        items(&args[0])?,
        items(&args[1])?,
        items(&args[2])?,
        items(&args[3])?,
    );
    Some(Ok(vm_map(vec![
        ("nodes", join(&[pre, recs])),
        ("problems", join(&[probs])),
        ("lifted", join(&[lifts])),
    ])))
}

/// The VM's own `gui.flatten`: the view and spec converted as any call's
/// arguments are (the conversion keeps what it converted, by identity),
/// and the answer in the VM's form — its lists of view nodes, paths and
/// keys wrapped where they are, its lists of integers (`index`, `wraps`,
/// `changed`) typed, so the passes read them natively. `None` on an
/// error (the call then goes the usual way, which raises it).
fn vm_flatten(args: &[OvmValue]) -> Option<Res<OvmValue>> {
    if !(2..=4).contains(&args.len()) {
        return None;
    }
    let ast: Vec<Value> = args
        .iter()
        .map(|a| a.to_ast().ok())
        .collect::<Option<Vec<_>>>()?;
    let answer = gui_flatten(ast).ok()?;
    Some(Ok(vm_flat_answer(answer)))
}

fn vm_flat_answer(v: Value) -> OvmValue {
    let m = match &v {
        Value::Map(m) => m.clone(),
        _ => return OvmValue::from_ast(v),
    };
    let mut out = crate::ovm::value::OvmMap::with_capacity_and_hasher(m.len(), Default::default());
    for (k, x) in m.iter() {
        let vm = match (k.as_str(), x) {
            ("nodes" | "paths" | "gkeys" | "glifts", Value::List(l)) => OvmValue {
                data: ValueData::AstList(l.clone()),
            },
            ("index" | "wraps" | "changed", Value::List(l)) => {
                match l
                    .iter()
                    .map(|e| match e {
                        Value::Integer(i) => Some(*i),
                        _ => None,
                    })
                    .collect::<Option<Vec<i64>>>()
                {
                    Some(ints) => OvmValue {
                        data: ValueData::IntList(Arc::new(ints)),
                    },
                    None => OvmValue::from_ast(x.clone()),
                }
            }
            _ => OvmValue::from_ast(x.clone()),
        };
        out.insert(k.clone(), vm);
    }
    OvmValue::new_map(Arc::new(out))
}

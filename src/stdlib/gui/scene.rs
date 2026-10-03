//! The retained tree a window draws: keyed nodes positioned by the
//! program (Loom's layout runs in olang), their styles, their
//! accessibility, and the patch operations `gui.apply` takes.
//!
//! A node's `rect` is relative to its parent's content origin — the
//! parent's top-left, moved by the parent's scroll offset — so a
//! subtree moves or scrolls without its children being re-sent.

use super::text::{Align, Color, Font};
use super::values::*;
use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Style {
    pub bg: Option<Color>,
    pub border: Option<Color>,
    pub border_width: f32,
    /// top-left, top-right, bottom-right, bottom-left.
    pub radius: [f32; 4],
    pub color: Color,
    pub font: Font,
    pub align: Align,
    /// Vertical placement of a text in its content box.
    pub valign: Align,
    pub wrap: bool,
    /// top, right, bottom, left.
    pub pad: [f32; 4],
    pub clip: bool,
    pub opacity: f32,
    /// The caret and the selection of an editable node.
    pub caret: Color,
    pub selection: Color,
    pub placeholder: Color,
    /// A focus ring drawn by the engine when the node has focus and no
    /// `focus` variant of its own; `None` turns it off.
    pub focus_ring: Option<Color>,
}

pub const DEFAULT_FG: Color = [24, 24, 27, 255];
pub const DEFAULT_ACCENT: Color = [37, 99, 235, 255];

impl Default for Style {
    fn default() -> Self {
        Style {
            bg: None,
            border: None,
            border_width: 0.0,
            radius: [0.0; 4],
            color: DEFAULT_FG,
            font: Font::default(),
            align: Align::Start,
            valign: Align::Start,
            wrap: false,
            pad: [0.0; 4],
            clip: false,
            opacity: 1.0,
            caret: DEFAULT_FG,
            selection: [37, 99, 235, 64],
            placeholder: [113, 113, 122, 255],
            focus_ring: Some(DEFAULT_ACCENT),
        }
    }
}

const STYLE_KEYS: &[&str] = &[
    "bg",
    "border",
    "border_width",
    "radius",
    "color",
    "font",
    "size",
    "weight",
    "italic",
    "line_height",
    "align",
    "valign",
    "wrap",
    "pad",
    "clip",
    "opacity",
    "caret",
    "selection",
    "placeholder_color",
    "focus_ring",
    "hover",
    "pressed",
    "focus",
];

impl Style {
    /// Apply a style map's properties over this style.
    pub fn apply(&mut self, m: &Value, what: &str) -> Res<()> {
        if let Some(f) = fields(m) {
            for k in f.keys() {
                if !STYLE_KEYS.contains(&k.as_str()) {
                    return Err(format!(
                        "{what}: unknown style \"{k}\" (the styles are {})",
                        STYLE_KEYS.join(", ")
                    ));
                }
            }
        } else {
            return Err(format!("{what}: a style must be a map"));
        }
        if let Some(Value::String(t)) = get(m, "bg")
            && t.as_str() == "none"
        {
            self.bg = None;
        } else if let Some(c) = get_color(m, "bg", what)? {
            self.bg = Some(c);
        }
        if let Some(c) = get_color(m, "border", what)? {
            self.border = Some(c);
        }
        if let Some(n) = get_num(m, "border_width", what)? {
            self.border_width = n.max(0.0);
        }
        if let Some(r) = get_sides(m, "radius", what)? {
            self.radius = r;
        }
        if let Some(c) = get_color(m, "color", what)? {
            self.color = c;
        }
        if let Some(f) = get_str(m, "font", what)? {
            self.font.family = f.to_string();
        }
        if let Some(n) = get_num(m, "size", what)? {
            self.font.size = n.max(1.0);
        }
        if let Some(n) = get_num(m, "weight", what)? {
            self.font.weight = n.clamp(1.0, 1000.0);
        }
        if let Some(b) = get_bool(m, "italic", what)? {
            self.font.italic = b;
        }
        if let Some(n) = get_num(m, "line_height", what)? {
            self.font.line_height = n.max(0.0);
        }
        if let Some(a) = get_str(m, "align", what)? {
            self.align = Align::parse(a)
                .ok_or_else(|| format!("{what}: \"align\" is start, center, or end"))?;
        }
        if let Some(a) = get_str(m, "valign", what)? {
            self.valign = Align::parse(a)
                .ok_or_else(|| format!("{what}: \"valign\" is start, center, or end"))?;
        }
        if let Some(b) = get_bool(m, "wrap", what)? {
            self.wrap = b;
        }
        if let Some(p) = get_sides(m, "pad", what)? {
            self.pad = p;
        }
        if let Some(b) = get_bool(m, "clip", what)? {
            self.clip = b;
        }
        if let Some(n) = get_num(m, "opacity", what)? {
            self.opacity = n.clamp(0.0, 1.0);
        }
        if let Some(c) = get_color(m, "caret", what)? {
            self.caret = c;
        }
        if let Some(c) = get_color(m, "selection", what)? {
            self.selection = c;
        }
        if let Some(c) = get_color(m, "placeholder_color", what)? {
            self.placeholder = c;
        }
        match get(m, "focus_ring") {
            Some(Value::Boolean(false)) => self.focus_ring = None,
            Some(_) => self.focus_ring = get_color(m, "focus_ring", what)?,
            None => {}
        }
        Ok(())
    }
}

/// What an editable node is.
#[derive(Clone, Debug, Default)]
pub struct EditProps {
    pub value: String,
    pub multiline: bool,
    pub placeholder: String,
    pub secure: bool,
    /// The edit revision the program last saw (from a `changed` event),
    /// so a value it has not yet caught up with is not taken for a reset.
    pub rev: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub key: String,
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub role: String,
    /// x, y, width, height — logical pixels, relative to the parent's
    /// content origin.
    pub rect: [f32; 4],
    pub style: Style,
    /// The raw maps of the `hover`, `pressed`, and `focus` variants,
    /// applied over `style` while the node is in that state.
    pub variants: [Option<Value>; 3],
    pub text: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub focusable: bool,
    pub disabled: bool,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    /// A value for assistive output (a slider's number, a progress).
    pub value: Option<String>,
    pub numeric: Option<[f32; 3]>,
    pub edit: Option<EditProps>,
    pub scrollable: bool,
    /// The scroll offset, owned by the engine (the wheel moves it).
    pub offset: (f32, f32),
    pub level: Option<u32>,
    pub live: Option<String>,
    /// The size of what a scrolling node scrolls through, when it is
    /// larger than its children reach: a virtualized list lays out only
    /// the rows on screen but scrolls through all of them.
    pub content: Option<(f32, f32)>,
    /// A modal layer (a dialog, a popover, a menu): while it is in the
    /// tree, the pointer and the focus stay inside the topmost one.
    pub modal: bool,
    /// The pointer finds this node before its siblings, wherever it sits
    /// among them (a divider whose target reaches over the panes beside
    /// it); reading and focus keep its place in the tree.
    pub grab: bool,
    /// A canvas's drawing operations (canvas.rs).
    pub draw: Option<Value>,
    /// An image's source: a PNG's path or its Bytes; and how it fits its
    /// box ("contain", the default, "cover", or "fill").
    pub image: Option<Value>,
    pub fit: String,
}

pub const HOVER: usize = 0;
pub const PRESSED: usize = 1;
pub const FOCUS: usize = 2;

/// The roles a node may have; assistive output and default behaviour
/// (what activates, what takes focus) follow from it.
pub const ROLES: &[&str] = &[
    "window",
    "group",
    "text",
    "heading",
    "image",
    "button",
    "checkbox",
    "radio",
    "switch",
    "slider",
    "input",
    "textarea",
    "search",
    "list",
    "listitem",
    "table",
    "row",
    "cell",
    "columnheader",
    "tree",
    "treeitem",
    "tablist",
    "tab",
    "tabpanel",
    "dialog",
    "menu",
    "menuitem",
    "tooltip",
    "status",
    "log",
    "progressbar",
    "region",
    "link",
    "separator",
    "figure",
];

/// A role a pointer or Enter/Space activates.
pub fn activates(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "checkbox"
            | "radio"
            | "switch"
            | "tab"
            | "menuitem"
            | "link"
            | "listitem"
            | "treeitem"
            | "row"
            | "columnheader"
    )
}

/// A role that takes focus unless told otherwise.
pub fn focusable_by_default(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "checkbox"
            | "radio"
            | "switch"
            | "slider"
            | "input"
            | "textarea"
            | "search"
            | "tab"
            | "menuitem"
            | "link"
            | "list"
            | "table"
            | "tree"
    )
}

const NODE_KEYS: &[&str] = &[
    "op",
    "key",
    "parent",
    "index",
    "role",
    "box",
    "style",
    "text",
    "name",
    "description",
    "focusable",
    "disabled",
    "checked",
    "selected",
    "expanded",
    "value",
    "range",
    "edit",
    "scroll",
    "level",
    "live",
    "content",
    "modal",
    "grab",
    "draw",
    "image",
    "fit",
];

#[derive(Default)]
pub struct Scene {
    pub nodes: HashMap<String, Node>,
    pub root: Option<String>,
    pub focus: Option<String>,
    pub hover: Option<String>,
    pub pressed: Option<String>,
}

/// What applying a patch changed, for the window to act on.
#[derive(Default)]
pub struct Applied {
    pub removed: Vec<String>,
    pub edits: Vec<String>,
    pub focus_set: bool,
}

impl Scene {
    pub fn apply(&mut self, ops: &Value, out: &mut Applied) -> Res<()> {
        let items: &[Value] = match ops {
            Value::List(l) => l,
            _ => return Err("gui.apply: the patch must be a list of operations".into()),
        };
        for (i, op) in items.iter().enumerate() {
            let what = format!("gui.apply: operation {}", i + 1);
            let kind = get_str(op, "op", &what)?.unwrap_or("node");
            match kind {
                "node" => self.upsert(op, &what, out)?,
                "remove" => {
                    let key = get_str(op, "key", &what)?
                        .ok_or_else(|| format!("{what}: \"remove\" needs a \"key\""))?;
                    self.remove(key, out);
                }
                "focus" => {
                    let key = get_str(op, "key", &what)?;
                    if let Some(k) = key
                        && !self.nodes.contains_key(k)
                    {
                        return Err(format!("{what}: no node \"{k}\" to focus"));
                    }
                    self.focus = key.map(str::to_string);
                    out.focus_set = true;
                }
                "scroll" => {
                    let key = get_str(op, "key", &what)?
                        .ok_or_else(|| format!("{what}: \"scroll\" needs a \"key\""))?;
                    let to = get_pair(op, "to", &what)?.unwrap_or((0.0, 0.0));
                    let node = self
                        .nodes
                        .get_mut(key)
                        .ok_or_else(|| format!("{what}: no node \"{key}\""))?;
                    node.offset = to;
                    self.clamp_scroll(key);
                }
                "clear" => {
                    let keys: Vec<String> = self.nodes.keys().cloned().collect();
                    out.removed.extend(keys);
                    *self = Scene::default();
                }
                other => {
                    return Err(format!(
                        "{what}: unknown op \"{other}\" (node, remove, focus, scroll, clear)"
                    ));
                }
            }
        }
        Ok(())
    }

    fn upsert(&mut self, op: &Value, what: &str, out: &mut Applied) -> Res<()> {
        if let Some(f) = fields(op) {
            for k in f.keys() {
                if !NODE_KEYS.contains(&k.as_str()) {
                    return Err(format!(
                        "{what}: unknown field \"{k}\" (the fields are {})",
                        NODE_KEYS.join(", ")
                    ));
                }
            }
        } else {
            return Err(format!("{what}: an operation must be a map"));
        }
        let key = get_str(op, "key", what)?
            .ok_or_else(|| format!("{what}: a node needs a \"key\""))?
            .to_string();
        let role = get_str(op, "role", what)?.unwrap_or("group").to_string();
        if !ROLES.contains(&role.as_str()) {
            return Err(format!(
                "{what}: unknown role \"{role}\" (the roles are {})",
                ROLES.join(", ")
            ));
        }
        let parent = get_str(op, "parent", what)?.map(str::to_string);
        if let Some(p) = &parent {
            if !self.nodes.contains_key(p) {
                return Err(format!(
                    "{what}: node \"{key}\" names parent \"{p}\", which does not exist (send a parent before its children)"
                ));
            }
            if *p == key || self.is_ancestor(&key, p) {
                return Err(format!("{what}: node \"{key}\" cannot be its own ancestor"));
            }
        }
        let rect = get_rect(op, "box", what)?.unwrap_or([0.0; 4]);
        let mut style = Style::default();
        let mut variants: [Option<Value>; 3] = [None, None, None];
        if let Some(st) = get(op, "style") {
            style.apply(st, what)?;
            for (i, name) in ["hover", "pressed", "focus"].iter().enumerate() {
                if let Some(v) = get(st, name) {
                    // Validate now, so a typo fails at the patch, not at
                    // the first hover.
                    style.clone().apply(v, &format!("{what}: style.{name}"))?;
                    variants[i] = Some(v.clone());
                }
            }
        }
        let edit = match get(op, "edit") {
            None => None,
            Some(e) => Some(EditProps {
                value: get_str(e, "value", what)?.unwrap_or("").to_string(),
                multiline: get_bool(e, "multiline", what)?.unwrap_or(role == "textarea"),
                placeholder: get_str(e, "placeholder", what)?.unwrap_or("").to_string(),
                secure: get_bool(e, "secure", what)?.unwrap_or(false),
                rev: get_num(e, "rev", what)?.map(|n| n as i64),
            }),
        };
        if edit.is_some() {
            out.edits.push(key.clone());
        }
        let numeric = match get(op, "range") {
            None => None,
            Some(r) => match nums(r).as_deref() {
                Some([v, lo, hi]) => Some([*v, *lo, *hi]),
                _ => return Err(format!("{what}: \"range\" is (value, min, max)")),
            },
        };
        let focusable = get_bool(op, "focusable", what)?
            .unwrap_or_else(|| focusable_by_default(&role) || edit.is_some());
        let value_str = match get(op, "value") {
            None => None,
            Some(Value::String(t)) => Some(t.to_string()),
            Some(other) => Some(format!("{}", other)),
        };
        let previous = self.nodes.get(&key);
        let (children, offset) = previous
            .map(|n| (n.children.clone(), n.offset))
            .unwrap_or_default();
        let old_parent = previous.and_then(|n| n.parent.clone());
        let was_root = self.root.as_deref() == Some(key.as_str());
        let node = Node {
            key: key.clone(),
            parent: parent.clone(),
            children,
            role: role.clone(),
            rect,
            style,
            variants,
            text: get_str(op, "text", what)?.map(str::to_string),
            name: get_str(op, "name", what)?.map(str::to_string),
            description: get_str(op, "description", what)?.map(str::to_string),
            focusable,
            disabled: get_bool(op, "disabled", what)?.unwrap_or(false),
            checked: get_bool(op, "checked", what)?,
            selected: get_bool(op, "selected", what)?,
            expanded: get_bool(op, "expanded", what)?,
            value: value_str,
            numeric,
            edit,
            scrollable: get_bool(op, "scroll", what)?.unwrap_or(role == "region"),
            offset,
            level: get_num(op, "level", what)?.map(|n| n as u32),
            live: get_str(op, "live", what)?.map(str::to_string),
            content: get_pair(op, "content", what)?,
            modal: get_bool(op, "modal", what)?.unwrap_or(false),
            grab: get_bool(op, "grab", what)?.unwrap_or(false),
            draw: get(op, "draw").cloned(),
            image: get(op, "image").cloned(),
            fit: get_str(op, "fit", what)?.unwrap_or("contain").to_string(),
        };
        // Detach from the old parent (or the root slot) when it moved.
        if previous.is_some() && old_parent != parent {
            match &old_parent {
                Some(op) => {
                    if let Some(p) = self.nodes.get_mut(op) {
                        p.children.retain(|c| *c != key);
                    }
                }
                None => {
                    if was_root {
                        self.root = None;
                    }
                }
            }
        }
        match &parent {
            None => {
                if let Some(r) = &self.root
                    && *r != key
                {
                    return Err(format!(
                        "{what}: node \"{key}\" has no parent, but \"{r}\" is already the window's root"
                    ));
                }
                self.root = Some(key.clone());
            }
            Some(p) => {
                let index = get_num(op, "index", what)?.map(|n| n.max(0.0) as usize);
                let parent_node = self.nodes.get_mut(p).expect("checked above");
                let at = parent_node.children.iter().position(|c| *c == key);
                match (at, index) {
                    (Some(i), Some(want)) if i != want => {
                        parent_node.children.remove(i);
                        let want = want.min(parent_node.children.len());
                        parent_node.children.insert(want, key.clone());
                    }
                    (Some(_), _) => {}
                    (None, Some(want)) => {
                        let want = want.min(parent_node.children.len());
                        parent_node.children.insert(want, key.clone());
                    }
                    (None, None) => parent_node.children.push(key.clone()),
                }
            }
        }
        self.nodes.insert(key.clone(), node);
        self.clamp_scroll(&key);
        Ok(())
    }

    fn is_ancestor(&self, maybe: &str, of: &str) -> bool {
        let mut at = self.nodes.get(of).and_then(|n| n.parent.clone());
        while let Some(k) = at {
            if k == maybe {
                return true;
            }
            at = self.nodes.get(&k).and_then(|n| n.parent.clone());
        }
        false
    }

    pub fn remove(&mut self, key: &str, out: &mut Applied) {
        let Some(node) = self.nodes.remove(key) else {
            return;
        };
        match &node.parent {
            Some(p) => {
                if let Some(pn) = self.nodes.get_mut(p) {
                    pn.children.retain(|c| c != key);
                }
            }
            None => {
                if self.root.as_deref() == Some(key) {
                    self.root = None;
                }
            }
        }
        out.removed.push(key.to_string());
        for slot in [&mut self.focus, &mut self.hover, &mut self.pressed] {
            if slot.as_deref() == Some(key) {
                *slot = None;
            }
        }
        for child in node.children {
            // The children's parent is gone; detach them first so the
            // recursion does not look for it.
            if let Some(c) = self.nodes.get_mut(&child) {
                c.parent = None;
            }
            self.remove_orphan(&child, out);
        }
    }

    fn remove_orphan(&mut self, key: &str, out: &mut Applied) {
        let Some(node) = self.nodes.remove(key) else {
            return;
        };
        out.removed.push(key.to_string());
        for slot in [&mut self.focus, &mut self.hover, &mut self.pressed] {
            if slot.as_deref() == Some(key) {
                *slot = None;
            }
        }
        for child in node.children {
            self.remove_orphan(&child, out);
        }
    }

    /// The node's style in its current state.
    pub fn effective_style(&self, node: &Node) -> Style {
        let mut st = node.style.clone();
        let k = Some(node.key.as_str());
        let states = [
            self.hover.as_deref() == k && !node.disabled,
            self.pressed.as_deref() == k && !node.disabled,
            self.focus.as_deref() == k,
        ];
        for (i, on) in states.iter().enumerate() {
            if *on && let Some(v) = &node.variants[i] {
                let _ = st.apply(v, "variant");
            }
        }
        st
    }

    /// The node's rectangle in window coordinates (logical pixels).
    pub fn absolute(&self, key: &str) -> Option<[f32; 4]> {
        let node = self.nodes.get(key)?;
        let mut x = node.rect[0];
        let mut y = node.rect[1];
        let mut at = node.parent.clone();
        while let Some(p) = at {
            let pn = self.nodes.get(&p)?;
            x += pn.rect[0] - pn.offset.0;
            y += pn.rect[1] - pn.offset.1;
            at = pn.parent.clone();
        }
        Some([x, y, node.rect[2], node.rect[3]])
    }

    /// What of a node shows: its absolute box cut by every ancestor
    /// that scrolls or clips, `None` when nothing of it does.
    pub fn visible(&self, key: &str) -> Option<[f32; 4]> {
        let a = self.absolute(key)?;
        let (mut x0, mut y0, mut x1, mut y1) = (a[0], a[1], a[0] + a[2], a[1] + a[3]);
        let mut at = self.nodes.get(key)?.parent.clone();
        while let Some(p) = at {
            let pn = self.nodes.get(&p)?;
            if pn.scrollable || pn.style.clip {
                let b = self.absolute(&p)?;
                x0 = x0.max(b[0]);
                y0 = y0.max(b[1]);
                x1 = x1.min(b[0] + b[2]);
                y1 = y1.min(b[1] + b[3]);
            }
            at = pn.parent.clone();
        }
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
    }

    /// The extent of a node's children, for clamping its scroll.
    fn content_size(&self, node: &Node) -> (f32, f32) {
        let (mut w, mut h) = node.content.unwrap_or((0.0, 0.0));
        for c in &node.children {
            if let Some(cn) = self.nodes.get(c) {
                w = w.max(cn.rect[0] + cn.rect[2]);
                h = h.max(cn.rect[1] + cn.rect[3]);
            }
        }
        (w, h)
    }

    pub fn clamp_scroll(&mut self, key: &str) {
        let Some(node) = self.nodes.get(key) else {
            return;
        };
        let (cw, ch) = self.content_size(node);
        let max_x = (cw - node.rect[2]).max(0.0);
        let max_y = (ch - node.rect[3]).max(0.0);
        let n = self.nodes.get_mut(key).expect("present");
        n.offset.0 = n.offset.0.clamp(0.0, max_x);
        n.offset.1 = n.offset.1.clamp(0.0, max_y);
    }

    /// The topmost node under a point (logical window coordinates),
    /// honouring clipping and scrolling. Later siblings are on top.
    pub fn hit(&self, x: f32, y: f32) -> Option<String> {
        // Under a modal layer, only the layer answers.
        if let Some(m) = self.modal_root() {
            let n = self.nodes.get(&m)?;
            let abs = self.absolute(&m)?;
            return self.hit_in(&m, abs[0] - n.rect[0], abs[1] - n.rect[1], x, y);
        }
        let root = self.root.as_ref()?;
        self.hit_in(root, 0.0, 0.0, x, y)
    }

    /// The topmost modal layer: the last modal node in tree order.
    pub fn modal_root(&self) -> Option<String> {
        fn walk(s: &Scene, key: &str, found: &mut Option<String>) {
            let Some(n) = s.nodes.get(key) else {
                return;
            };
            if n.modal {
                *found = Some(key.to_string());
            }
            for c in &n.children {
                walk(s, c, found);
            }
        }
        let mut found = None;
        if let Some(r) = &self.root {
            walk(self, r, &mut found);
        }
        found
    }

    /// Whether `key` is `ancestor` or inside it.
    pub fn within(&self, key: &str, ancestor: &str) -> bool {
        let mut at = Some(key.to_string());
        while let Some(k) = at {
            if k == ancestor {
                return true;
            }
            at = self.nodes.get(&k).and_then(|n| n.parent.clone());
        }
        false
    }

    fn hit_in(&self, key: &str, ox: f32, oy: f32, x: f32, y: f32) -> Option<String> {
        let node = self.nodes.get(key)?;
        let rx = ox + node.rect[0];
        let ry = oy + node.rect[1];
        let inside = x >= rx && y >= ry && x < rx + node.rect[2] && y < ry + node.rect[3];
        let clips = node.style.clip || node.scrollable;
        if clips && !inside {
            return None;
        }
        let cx = rx - node.offset.0;
        let cy = ry - node.offset.1;
        // Children that grab the pointer first, then the rest, each topmost
        // (last) first.
        let grabs = |c: &&String| self.nodes.get(c.as_str()).is_some_and(|n| n.grab);
        let order = node
            .children
            .iter()
            .rev()
            .filter(grabs)
            .chain(node.children.iter().rev().filter(|c| !grabs(c)));
        for child in order {
            if let Some(h) = self.hit_in(child, cx, cy, x, y) {
                return Some(h);
            }
        }
        if inside && node.role != "text" || inside && node.parent.is_none() {
            Some(key.to_string())
        } else {
            None
        }
    }

    /// The nearest node at or above `key` whose role activates, or that
    /// takes focus — what a click on `key` is a click on.
    pub fn target_up(&self, key: &str, pred: impl Fn(&Node) -> bool) -> Option<String> {
        let mut at = Some(key.to_string());
        while let Some(k) = at {
            let n = self.nodes.get(&k)?;
            if pred(n) {
                return Some(k);
            }
            at = n.parent.clone();
        }
        None
    }

    /// The nearest scrollable node at or above `key`.
    pub fn scroller_up(&self, key: &str) -> Option<String> {
        self.target_up(key, |n| n.scrollable)
    }

    /// Focusable nodes in tree order, skipping disabled ones.
    pub fn focus_order(&self) -> Vec<String> {
        let mut out = Vec::new();
        // A modal layer traps the focus.
        let start = self.modal_root().or_else(|| self.root.clone());
        if let Some(r) = &start {
            self.collect_focusable(r, &mut out);
        }
        out
    }

    fn collect_focusable(&self, key: &str, out: &mut Vec<String>) {
        let Some(n) = self.nodes.get(key) else {
            return;
        };
        if n.focusable && !n.disabled {
            out.push(key.to_string());
        }
        for c in &n.children {
            self.collect_focusable(c, out);
        }
    }

    /// Scroll every scrollable ancestor so `key` is visible.
    pub fn reveal(&mut self, key: &str) {
        let Some(mut at) = self.nodes.get(key).and_then(|n| n.parent.clone()) else {
            return;
        };
        loop {
            let (Some(target), Some(sc)) = (self.absolute(key), self.absolute(&at)) else {
                return;
            };
            let parent_next = self.nodes.get(&at).and_then(|n| n.parent.clone());
            if self.nodes.get(&at).is_some_and(|n| n.scrollable) {
                let n = self.nodes.get_mut(&at).expect("present");
                if target[1] < sc[1] {
                    n.offset.1 -= sc[1] - target[1];
                } else if target[1] + target[3] > sc[1] + sc[3] {
                    n.offset.1 += target[1] + target[3] - (sc[1] + sc[3]);
                }
                if target[0] < sc[0] {
                    n.offset.0 -= sc[0] - target[0];
                } else if target[0] + target[2] > sc[0] + sc[2] {
                    n.offset.0 += target[0] + target[2] - (sc[0] + sc[2]);
                }
                self.clamp_scroll(&at);
            }
            match parent_next {
                Some(p) => at = p,
                None => return,
            }
        }
    }

    /// A node as a map, for `gui.read`.
    pub fn node_value(&self, key: &str) -> Option<Value> {
        let n = self.nodes.get(key)?;
        let abs = self.absolute(key)?;
        Some(map(vec![
            ("key", s(&n.key)),
            ("role", s(&n.role)),
            (
                "bounds",
                Value::Tuple(Arc::new(abs.iter().map(|v| float(*v)).collect())),
            ),
            ("text", opt_str(n.text.as_deref())),
            ("name", opt_str(n.name.as_deref())),
            (
                "children",
                Value::List(Arc::new(n.children.iter().map(|c| s(c)).collect())),
            ),
            (
                "offset",
                Value::Tuple(Arc::new(vec![float(n.offset.0), float(n.offset.1)])),
            ),
        ]))
    }
}

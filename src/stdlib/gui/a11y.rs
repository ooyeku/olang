//! The accessibility tree: the scene's roles, names, states, and bounds
//! as the platform's accessibility objects (AccessKit: NSAccessibility,
//! UI Automation, AT-SPI), and assistive actions turned back into the
//! events a pointer or key would send.
//!
//! The same walk answers `gui.read(w, "a11y")`, so a test pins what a
//! screen reader is given without one running.

use super::scene::{self, Node};
use super::values::*;
use super::window::WinState;
use crate::ast::Value;
use accesskit::{
    Action, ActionData, ActionRequest, CustomAction, NodeId, Rect, Role, Toggled, TreeId, TreeInfo,
    TreeUpdate,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// The window's own node.
const WINDOW_NODE: NodeId = NodeId(1);

pub fn node_id(key: &str) -> NodeId {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    // Keep clear of the window's id.
    NodeId(h.finish() | 2)
}

fn key_of(st: &WinState, id: NodeId) -> Option<String> {
    st.scene.nodes.keys().find(|k| node_id(k) == id).cloned()
}

fn role_of(n: &Node) -> Role {
    match n.role.as_str() {
        "window" => Role::Window,
        "text" => Role::Label,
        "heading" => Role::Heading,
        "image" => Role::Image,
        "button" => Role::Button,
        "checkbox" => Role::CheckBox,
        "radio" => Role::RadioButton,
        "switch" => Role::Switch,
        "slider" => Role::Slider,
        "input" => {
            if n.edit.as_ref().is_some_and(|e| e.secure) {
                Role::PasswordInput
            } else {
                Role::TextInput
            }
        }
        "textarea" => Role::MultilineTextInput,
        "search" => Role::SearchInput,
        "list" => Role::List,
        "listitem" => Role::ListItem,
        "table" => Role::Table,
        "row" => Role::Row,
        "cell" => Role::Cell,
        "columnheader" => Role::ColumnHeader,
        "tree" => Role::Tree,
        "treeitem" => Role::TreeItem,
        "tablist" => Role::TabList,
        "tab" => Role::Tab,
        "tabpanel" => Role::TabPanel,
        "dialog" => Role::Dialog,
        "menu" => Role::Menu,
        "menuitem" => Role::MenuItem,
        "tooltip" => Role::Tooltip,
        "status" => Role::Status,
        "log" => Role::Log,
        "progressbar" => Role::ProgressIndicator,
        "region" => {
            if n.scrollable {
                Role::ScrollView
            } else {
                Role::Region
            }
        }
        "link" => Role::Link,
        "separator" => Role::Splitter,
        "figure" => Role::Figure,
        _ => Role::GenericContainer,
    }
}

/// What a node is called: its name, else its text.
fn label_of(n: &Node) -> Option<String> {
    n.name.clone().or_else(|| n.text.clone())
}

/// The value assistive output reads for a node.
fn value_of(st: &WinState, n: &Node) -> Option<String> {
    if let Some(ed) = st.editors.get(&n.key) {
        if ed.secure() {
            return None;
        }
        return Some(ed.value());
    }
    n.value.clone()
}

/// The tree for the platform.
pub fn tree(st: &WinState) -> TreeUpdate {
    let s = st.scale as f64;
    let mut nodes = Vec::new();
    let mut window = accesskit::Node::new(Role::Window);
    window.set_label(st.title.clone());
    if let Some(r) = &st.scene.root {
        window.set_children(vec![node_id(r)]);
    }
    nodes.push((WINDOW_NODE, window));
    let said = |c: &String| st.scene.nodes.get(c).is_some_and(|n| !n.decorative);
    let lenders = lenders(st);
    for (key, n) in &st.scene.nodes {
        if n.decorative {
            continue;
        }
        let mut node = accesskit::Node::new(role_of(n));
        if let Some(l) = label_of(n) {
            node.set_label(l);
        }
        if let Some(d) = &n.description {
            node.set_description(d.clone());
        }
        if let Some(a) = &n.active
            && st.scene.nodes.contains_key(a)
        {
            node.set_active_descendant(node_id(a));
        }
        if let Some(v) = value_of(st, n) {
            node.set_value(v);
        }
        if let Some(e) = &n.edit
            && !e.placeholder.is_empty()
        {
            node.set_placeholder(e.placeholder.clone());
        }
        if let Some(abs) = st.scene.absolute(key) {
            node.set_bounds(Rect {
                x0: abs[0] as f64 * s,
                y0: abs[1] as f64 * s,
                x1: (abs[0] + abs[2]) as f64 * s,
                y1: (abs[1] + abs[3]) as f64 * s,
            });
        }
        let mut kids: Vec<NodeId> = n
            .children
            .iter()
            .filter(|c| said(c))
            .map(|c| node_id(c))
            .collect();
        // a styled field's text as runs: lines, words, characters, and
        // the selection a screen reader reads and moves
        if let Some(r) = st.editors.get(key).and_then(|e| e.rich())
            && !r.secure
        {
            let (runs, sel) = text_runs(st, key, r);
            for (id, run) in runs {
                kids.push(id);
                nodes.push((id, run));
            }
            if let Some(sel) = sel {
                node.set_text_selection(sel);
            }
            node.add_action(Action::SetTextSelection);
        }
        node.set_children(kids);
        if n.disabled {
            node.set_disabled();
        }
        if let Some(c) = n.checked {
            node.set_toggled(if c { Toggled::True } else { Toggled::False });
        }
        if let Some(sel) = n.selected {
            node.set_selected(sel);
        }
        if let Some(x) = n.expanded {
            node.set_expanded(x);
        }
        if let Some(l) = n.level {
            node.set_level(l as usize);
        }
        if let Some([v, lo, hi]) = n.numeric {
            node.set_numeric_value(v as f64);
            node.set_min_numeric_value(lo as f64);
            node.set_max_numeric_value(hi as f64);
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
        }
        if let Some(live) = &n.live {
            node.set_live(match live.as_str() {
                "assertive" => accesskit::Live::Assertive,
                _ => accesskit::Live::Polite,
            });
        } else if matches!(n.role.as_str(), "status" | "log") {
            node.set_live(accesskit::Live::Polite);
        }
        if n.focusable && !n.disabled {
            node.add_action(Action::Focus);
        }
        if scene::activates(&n.role) && !n.disabled {
            node.add_action(Action::Click);
        }
        // an animation with a play button: pressed, it plays or pauses
        if let Some(a) = st.anim_shown(key)
            && a.badge.is_some()
        {
            node.add_action(Action::Click);
            let said = if a.paused { "Animation, paused" } else { "Animation, playing" };
            node.set_description(match &n.description {
                Some(d) => format!("{d}. {said}"),
                None => said.to_string(),
            });
        }
        if n.edit.is_some() && !n.disabled {
            node.add_action(Action::SetValue);
        }
        if n.scrollable {
            node.set_scroll_x(n.offset.0 as f64);
            node.set_scroll_y(n.offset.1 as f64);
        }
        // The node's actions (a card's "Move right"), its own or lent by
        // the container whose active descendant it is (see `actions_for`).
        // On macOS they are NSAccessibilityCustomAction objects (VoiceOver's
        // Actions rotor), through olang's patched accesskit_macos
        // (vendor/accesskit_macos).
        if let Some((_, actions)) = actions_for(st, key, &lenders) {
            node.set_custom_actions(
                actions
                    .iter()
                    .enumerate()
                    .map(|(i, label)| CustomAction {
                        id: i as i32,
                        description: label.clone(),
                    })
                    .collect::<Vec<_>>(),
            );
            node.add_action(Action::CustomAction);
        }
        node.add_action(Action::ScrollIntoView);
        nodes.push((node_id(key), node));
    }
    let focus = match &st.scene.focus {
        Some(f) if st.focused_window => node_id(f),
        _ => WINDOW_NODE,
    };
    let mut info = TreeInfo::new(WINDOW_NODE);
    info.toolkit_name = Some("Loom".to_string());
    info.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_string());
    TreeUpdate {
        nodes,
        tree: Some(info),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// The id of paragraph `i`'s run from character `c` of field `key`.
fn run_id(key: &str, i: usize, c: usize) -> NodeId {
    node_id(&format!("{key}\u{0}run:{i}:{c}"))
}

/// The runs of a styled field: a run a paragraph (in pieces of at most
/// 255 characters, AccessKit's limit), each ending with its line break,
/// and the selection as positions in them.
fn text_runs(
    st: &WinState,
    key: &str,
    r: &super::rich::RichEditor,
) -> (
    Vec<(NodeId, accesskit::Node)>,
    Option<accesskit::TextSelection>,
) {
    let s = st.scale as f64;
    let origin = st.scene.absolute(key);
    let n = r.para_count();
    let ((ai, ac), (fi, fc)) = r.selection_in_paras();
    let mut out = Vec::new();
    let (mut anchor, mut focus) = (None, None);
    for i in 0..n {
        let text = r.para_text(i);
        let mut chars: Vec<char> = text.chars().collect();
        if i + 1 < n {
            chars.push('\n');
        }
        let pieces = if chars.is_empty() {
            1
        } else {
            chars.len().div_ceil(255)
        };
        for k in 0..pieces {
            let c0 = k * 255;
            let c1 = ((k + 1) * 255).min(chars.len());
            let piece: String = chars[c0..c1].iter().collect();
            let id = run_id(key, i, c0);
            let mut run = accesskit::Node::new(Role::TextRun);
            run.set_character_lengths(
                chars[c0..c1]
                    .iter()
                    .map(|c| c.len_utf8() as u8)
                    .collect::<Vec<_>>(),
            );
            let mut starts = Vec::new();
            let mut prev_space = true;
            for (j, c) in chars[c0..c1].iter().enumerate() {
                let space = c.is_whitespace();
                if !space && prev_space {
                    starts.push(j as u8);
                }
                prev_space = space;
            }
            run.set_word_starts(starts);
            run.set_value(piece);
            if let (Some(abs), Some((top, h))) = (origin, r.para_top(i)) {
                let y = abs[1] as f64 * s + (top - r.scroll_y) as f64;
                run.set_bounds(Rect {
                    x0: abs[0] as f64 * s,
                    y0: y,
                    x1: (abs[0] + abs[2]) as f64 * s,
                    y1: y + h as f64,
                });
            }
            let last = k + 1 == pieces;
            let at = |c: usize| -> Option<accesskit::TextPosition> {
                (c >= c0 && (c < c1 || (last && c <= c1))).then(|| accesskit::TextPosition {
                    node: id,
                    character_index: c - c0,
                })
            };
            if ai == i && anchor.is_none() {
                anchor = at(ac);
            }
            if fi == i && focus.is_none() {
                focus = at(fc);
            }
            out.push((id, run));
        }
    }
    let sel = match (anchor, focus) {
        (Some(anchor), Some(focus)) => Some(accesskit::TextSelection { anchor, focus }),
        _ => None,
    };
    (out, sel)
}

/// A run's id as (paragraph, character), for a selection a client sets.
fn run_place(st: &WinState, key: &str, id: NodeId) -> Option<(usize, usize)> {
    let r = st.editors.get(key)?.rich()?;
    for i in 0..r.para_count() {
        let n = r.para_text(i).chars().count() + 1;
        let mut c = 0;
        while c < n.max(1) {
            if run_id(key, i, c) == id {
                return Some((i, c));
            }
            c += 255;
        }
    }
    None
}

/// Containers whose actions their active descendant shows, by that
/// descendant: a list's actions act on its selected row, and a screen
/// reader's cursor is on the row (the platform's focus follows the active
/// descendant), so the row offers them too.
fn lenders(st: &WinState) -> HashMap<&str, &str> {
    st.scene
        .nodes
        .iter()
        .filter(|(_, n)| !n.actions.is_empty() && !n.disabled)
        .filter_map(|(k, n)| Some((n.active.as_deref()?, k.as_str())))
        .filter(|(a, _)| st.scene.nodes.contains_key(*a))
        .collect()
}

/// The assistive actions node `key` offers, with the key of the node they
/// belong to: its own, or else those of the container whose active
/// descendant it is.
fn actions_for<'a>(
    st: &'a WinState,
    key: &'a str,
    lenders: &HashMap<&'a str, &'a str>,
) -> Option<(&'a str, &'a [String])> {
    let n = st.scene.nodes.get(key)?;
    if n.disabled {
        return None;
    }
    if !n.actions.is_empty() {
        return Some((key, &n.actions));
    }
    let owner = *lenders.get(key)?;
    Some((owner, &st.scene.nodes.get(owner)?.actions))
}

/// An assistive action, as the events a pointer or key would send.
pub fn action(st: &mut WinState, req: &ActionRequest, out: &mut Vec<Value>) {
    let Some(key) = key_of(st, req.target_node) else {
        return;
    };
    let wid = st.id;
    let ev = |kind: &str, mut fields: Vec<(&str, Value)>| {
        fields.push(("window", Value::Integer(wid as i64)));
        event(kind, fields)
    };
    match req.action {
        Action::Click if st.anim_shown(&key).is_some_and(|a| a.badge.is_some()) => {
            st.toggle_play(&key);
        }
        Action::Click => out.push(ev(
            "activate",
            vec![("key", s(&key)), ("source", s("a11y"))],
        )),
        Action::Focus => {
            if st.scene.focus.as_deref() != Some(key.as_str()) {
                st.scene.focus = Some(key.clone());
                let moved = st.scene.reveal(&key);
                st.said_scrolled(moved, out);
                st.dirty = true;
                st.a11y_dirty = true;
                out.push(ev("focus", vec![("key", s(&key))]));
            }
        }
        Action::ScrollIntoView => {
            let moved = st.scene.reveal(&key);
            st.said_scrolled(moved, out);
            st.dirty = true;
        }
        Action::SetTextSelection => {
            if let Some(ActionData::SetTextSelection(sel)) = &req.data
                && let Some((ai, ac)) = run_place(st, &key, sel.anchor.node)
                && let Some((fi, fc)) = run_place(st, &key, sel.focus.node)
                && let Some(r) = st.editors.get_mut(&key).and_then(|e| e.rich_mut())
            {
                r.select_paras(
                    (ai, ac + sel.anchor.character_index),
                    (fi, fc + sel.focus.character_index),
                );
                st.dirty = true;
                st.a11y_dirty = true;
                st.said_selection_of(&key, out);
            }
        }
        Action::SetValue => {
            if let Some(ActionData::Value(v)) = &req.data {
                out.push(ev(
                    "a11y",
                    vec![
                        ("key", s(&key)),
                        ("action", s("set_value")),
                        ("value", s(v)),
                    ],
                ));
            } else if let Some(ActionData::NumericValue(v)) = &req.data {
                out.push(ev(
                    "a11y",
                    vec![
                        ("key", s(&key)),
                        ("action", s("set_value")),
                        ("value", Value::Float(*v)),
                    ],
                ));
            }
        }
        Action::Increment => out.push(ev(
            "a11y",
            vec![("key", s(&key)), ("action", s("increment"))],
        )),
        Action::Decrement => out.push(ev(
            "a11y",
            vec![("key", s(&key)), ("action", s("decrement"))],
        )),
        Action::CustomAction => {
            if let Some(ActionData::CustomAction(i)) = &req.data {
                // a row's lent action is its container's
                let lenders = lenders(st);
                let chosen = actions_for(st, &key, &lenders).and_then(|(owner, actions)| {
                    Some((owner.to_string(), actions.get(*i as usize)?.clone()))
                });
                if let Some((owner, label)) = chosen {
                    out.push(ev(
                        "a11y",
                        vec![
                            ("key", s(&owner)),
                            ("action", s("custom")),
                            ("index", Value::Integer(*i as i64)),
                            ("label", s(&label)),
                        ],
                    ));
                }
            }
        }
        _ => {}
    }
}

/// The tree as olang values, for tests: one map per node, in tree order.
pub fn as_value(st: &mut WinState) -> Value {
    let mut out = Vec::new();
    if let Some(r) = st.scene.root.clone() {
        walk(st, &r, 0, &mut out);
    }
    Value::List(Arc::new(out))
}

fn walk(st: &WinState, key: &str, depth: i64, out: &mut Vec<Value>) {
    let Some(n) = st.scene.nodes.get(key) else {
        return;
    };
    if n.decorative {
        return;
    }
    let mut actions = Vec::new();
    if n.focusable && !n.disabled {
        actions.push(s("focus"));
    }
    if scene::activates(&n.role) && !n.disabled
        || st.anim_shown(key).is_some_and(|a| a.badge.is_some())
    {
        actions.push(s("click"));
    }
    if n.edit.is_some() && !n.disabled {
        actions.push(s("set_value"));
    }
    let custom: Vec<Value> = actions_for(st, key, &lenders(st))
        .map(|(_, actions)| actions.iter().map(|a| s(a)).collect())
        .unwrap_or_default();
    out.push(map(vec![
        ("key", s(key)),
        ("role", s(&format!("{:?}", role_of(n)))),
        ("name", opt_str(label_of(n).as_deref())),
        ("value", opt_str(value_of(st, n).as_deref())),
        ("depth", Value::Integer(depth)),
        (
            "focused",
            Value::Boolean(st.scene.focus.as_deref() == Some(key)),
        ),
        ("disabled", Value::Boolean(n.disabled)),
        (
            "checked",
            n.checked.map(Value::Boolean).unwrap_or(Value::Unit),
        ),
        (
            "selected",
            n.selected.map(Value::Boolean).unwrap_or(Value::Unit),
        ),
        ("actions", Value::List(Arc::new(actions))),
        ("custom", Value::List(Arc::new(custom))),
        ("description", opt_str(n.description.as_deref())),
        ("active", opt_str(n.active.as_deref())),
        (
            "text_runs",
            match st.editors.get(key).and_then(|e| e.rich()) {
                Some(r) => Value::Integer(text_runs(st, key, r).0.len() as i64),
                None => Value::Unit,
            },
        ),
        (
            "text_selection",
            match st.editors.get(key).and_then(|e| e.rich()) {
                Some(r) => {
                    let ((ai, ac), (fi, fc)) = r.selection_in_paras();
                    Value::Tuple(Arc::new(vec![
                        Value::Integer(ai as i64),
                        Value::Integer(ac as i64),
                        Value::Integer(fi as i64),
                        Value::Integer(fc as i64),
                    ]))
                }
                None => Value::Unit,
            },
        ),
    ]));
    for c in &n.children {
        walk(st, c, depth + 1, out);
    }
}

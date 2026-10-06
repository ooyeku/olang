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
        if ed.secure {
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
        node.set_children(
            n.children
                .iter()
                .filter(|c| said(c))
                .map(|c| node_id(c))
                .collect::<Vec<_>>(),
        );
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
        if n.edit.is_some() && !n.disabled {
            node.add_action(Action::SetValue);
        }
        if n.scrollable {
            node.set_scroll_x(n.offset.0 as f64);
            node.set_scroll_y(n.offset.1 as f64);
        }
        // The node's own actions (a card's "Move right"). AccessKit hands
        // them to UI Automation and AT-SPI; its macOS adapter does not yet
        // publish custom actions, so on macOS they are reached through the
        // program's commands and the node's action menu (Loom).
        if !n.actions.is_empty() && !n.disabled {
            node.set_custom_actions(
                n.actions
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

/// An assistive action, as the events a pointer or key would send.
pub fn action(st: &mut WinState, req: &ActionRequest, out: &mut Vec<Value>) {
    let Some(key) = key_of(st, req.target_node) else {
        return;
    };
    let ev = |kind: &str, mut fields: Vec<(&str, Value)>| {
        fields.push(("window", Value::Integer(st.id as i64)));
        event(kind, fields)
    };
    match req.action {
        Action::Click => out.push(ev(
            "activate",
            vec![("key", s(&key)), ("source", s("a11y"))],
        )),
        Action::Focus => {
            if st.scene.focus.as_deref() != Some(key.as_str()) {
                st.scene.focus = Some(key.clone());
                st.scene.reveal(&key);
                st.dirty = true;
                st.a11y_dirty = true;
                out.push(ev("focus", vec![("key", s(&key))]));
            }
        }
        Action::ScrollIntoView => {
            st.scene.reveal(&key);
            st.dirty = true;
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
                let label = st
                    .scene
                    .nodes
                    .get(&key)
                    .and_then(|n| n.actions.get(*i as usize).cloned());
                if let Some(label) = label {
                    out.push(ev(
                        "a11y",
                        vec![
                            ("key", s(&key)),
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
    if scene::activates(&n.role) && !n.disabled {
        actions.push(s("click"));
    }
    if n.edit.is_some() && !n.disabled {
        actions.push(s("set_value"));
    }
    let custom: Vec<Value> = if n.disabled {
        Vec::new()
    } else {
        n.actions.iter().map(|a| s(a)).collect()
    };
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
    ]));
    for c in &n.children {
        walk(st, c, depth + 1, out);
    }
}

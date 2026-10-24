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
        // something that wants the person now (a file changed on disk
        // under edits): said at once, interrupting
        "alert" => Role::Alert,
        "progressbar" => Role::ProgressIndicator,
        "region" => {
            if n.scrollable {
                Role::ScrollView
            } else if n.name.is_some() {
                // a region is a landmark when it is named (as ARIA's is)
                Role::Region
            } else {
                Role::GenericContainer
            }
        }
        "link" => Role::Link,
        "separator" => Role::Splitter,
        "figure" => Role::Figure,
        "navigation" => Role::Navigation,
        "main" => Role::Main,
        "banner" => Role::Banner,
        "complementary" => Role::Complementary,
        "contentinfo" => Role::ContentInfo,
        "form" => Role::Form,
        "grid" => Role::Grid,
        "rowheader" => Role::RowHeader,
        "rowgroup" => Role::RowGroup,
        "spinbutton" => Role::SpinButton,
        "meter" => Role::Meter,
        "toolbar" => Role::Toolbar,
        "radiogroup" => Role::RadioGroup,
        _ => Role::GenericContainer,
    }
}

// ── where a node stands in its set, its table's rows and columns ─────

/// What the tree's shape says of a node: its place in its set (a list's
/// item, a tree's, a tab, a table's row), a table's row and column, a
/// set's or a table's size. Loom's virtualized lists lay out only the rows
/// on screen, so a row's own `pos` wins over its order among the nodes.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Place {
    /// 0-based place in its set, and the set's size (on the item).
    pub in_set: Option<(usize, usize)>,
    /// A table's row: its 0-based index. A cell: its row's.
    pub row: Option<usize>,
    /// A cell's or a header's 0-based column.
    pub column: Option<usize>,
    /// A container's items, or a table's rows and columns.
    pub size: Option<usize>,
    pub rows: Option<usize>,
    pub columns: Option<usize>,
}

/// The item role a container's set is made of.
fn item_role(container: &str) -> Option<&'static [&'static str]> {
    Some(match container {
        "list" => &["listitem"],
        "tree" => &["treeitem"],
        "table" | "grid" => &["row"],
        "tablist" => &["tab"],
        "menu" => &["menuitem"],
        "radiogroup" => &["radio"],
        _ => return None,
    })
}

fn is_cell(role: &str) -> bool {
    matches!(role, "cell" | "columnheader" | "rowheader")
}

/// The items of container `key`: its descendants of its item role, found
/// through plain groups (a table's body, a list's scroller), not inside
/// another container or an item.
fn items_of(sc: &scene::Scene, key: &str, roles: &[&str], out: &mut Vec<String>) {
    let Some(n) = sc.nodes.get(key) else { return };
    for c in &n.children {
        let Some(cn) = sc.nodes.get(c) else { continue };
        if cn.decorative {
            continue;
        }
        if roles.contains(&cn.role.as_str()) {
            out.push(c.clone());
        } else if item_role(&cn.role).is_none() && !is_cell(&cn.role) {
            items_of(sc, c, roles, out);
        }
    }
}

/// A row's cells (and a header band's headers): through plain groups.
fn cells_of(sc: &scene::Scene, key: &str, out: &mut Vec<String>) {
    let Some(n) = sc.nodes.get(key) else { return };
    for c in &n.children {
        let Some(cn) = sc.nodes.get(c) else { continue };
        if cn.decorative {
            continue;
        }
        if is_cell(&cn.role) {
            out.push(c.clone());
        } else if item_role(&cn.role).is_none() && cn.role != "row" {
            cells_of(sc, c, out);
        }
    }
}

/// Every node's place, by key (only nodes that have one).
pub fn places(sc: &scene::Scene) -> HashMap<String, Place> {
    let mut out: HashMap<String, Place> = HashMap::new();
    for (key, n) in &sc.nodes {
        if n.decorative {
            continue;
        }
        let Some(roles) = item_role(&n.role) else {
            continue;
        };
        let mut items = Vec::new();
        items_of(sc, key, roles, &mut items);
        let table = matches!(n.role.as_str(), "table" | "grid");
        // a table's header row (its cells all column headers) is not one
        // of its rows: its headers say their columns
        let mut head_rows = Vec::new();
        if table {
            items.retain(|r| {
                let mut cells = Vec::new();
                cells_of(sc, r, &mut cells);
                let head =
                    !cells.is_empty() && cells.iter().all(|c| sc.nodes[c].role == "columnheader");
                if head {
                    head_rows.push(r.clone());
                }
                !head
            });
        }
        // the set's size: what the items say of it, else how many there are
        let said = items
            .iter()
            .filter_map(|k| sc.nodes.get(k)?.pos.map(|p| p.1 as usize))
            .max();
        let size = said.unwrap_or(items.len()).max(items.len());
        if n.role == "tree" {
            tree_places(sc, &items, &mut out);
        } else {
            for (i, k) in items.iter().enumerate() {
                let at = sc.nodes[k].pos.map(|p| p.0 as usize).unwrap_or(i);
                let e = out.entry(k.clone()).or_default();
                e.in_set = Some((at, size));
                if table {
                    e.row = Some(at);
                }
            }
        }
        out.entry(key.clone()).or_default().size = Some(size);
        if table {
            // the columns: the most cells a row has, or the headers
            let mut columns = 0;
            for r in &items {
                let mut cells = Vec::new();
                cells_of(sc, r, &mut cells);
                columns = columns.max(cells.len());
                let row = out.get(r).and_then(|p| p.row);
                for (j, ck) in cells.iter().enumerate() {
                    let e = out.entry(ck.clone()).or_default();
                    e.column = Some(j);
                    e.row = row;
                }
            }
            let mut heads = Vec::new();
            headers_of(sc, key, &mut heads);
            for r in &head_rows {
                cells_of(sc, r, &mut heads);
            }
            for (j, hk) in heads.iter().enumerate() {
                out.entry(hk.clone()).or_default().column = Some(j);
            }
            columns = columns.max(heads.len());
            let c = out.entry(key.clone()).or_default();
            c.rows = Some(size);
            c.columns = Some(columns);
        }
    }
    out
}

/// A table's column headers outside its rows (its header band).
fn headers_of(sc: &scene::Scene, key: &str, out: &mut Vec<String>) {
    let Some(n) = sc.nodes.get(key) else { return };
    for c in &n.children {
        let Some(cn) = sc.nodes.get(c) else { continue };
        if cn.decorative {
            continue;
        }
        if cn.role == "columnheader" {
            out.push(c.clone());
        } else if item_role(&cn.role).is_none() && cn.role != "row" && !is_cell(&cn.role) {
            headers_of(sc, c, out);
        }
    }
}

/// A tree's items' places: among the items of their own level under the
/// same parent item (a flat tree says each item's `level`), unless an
/// item says its `pos`.
fn tree_places(sc: &scene::Scene, items: &[String], out: &mut HashMap<String, Place>) {
    let level = |k: &String| sc.nodes[k].level.unwrap_or(1);
    for (i, k) in items.iter().enumerate() {
        if let Some((at, n)) = sc.nodes[k].pos {
            out.entry(k.clone()).or_default().in_set = Some((at as usize, n as usize));
            continue;
        }
        let l = level(k);
        // the run of siblings: back and on to an item of a lower level
        let mut first = i;
        while first > 0 && level(&items[first - 1]) >= l {
            first -= 1;
        }
        let mut last = i;
        while last + 1 < items.len() && level(&items[last + 1]) >= l {
            last += 1;
        }
        let sibs: Vec<usize> = (first..=last).filter(|&j| level(&items[j]) == l).collect();
        let at = sibs.iter().position(|&j| j == i).unwrap_or(0);
        out.entry(k.clone()).or_default().in_set = Some((at, sibs.len()));
    }
}

/// What a node is called: its name, else its text.
fn label_of(n: &Node) -> Option<String> {
    n.name.clone().or_else(|| n.text.clone())
}

/// How changes to a node are said: its own `live`, else its role's
/// (a status and a log politely, an alert at once). `None` inherits.
fn live_of(n: &Node) -> Option<accesskit::Live> {
    match n.live.as_deref() {
        Some("off") => Some(accesskit::Live::Off),
        Some("assertive") => Some(accesskit::Live::Assertive),
        Some(_) => Some(accesskit::Live::Polite),
        None => match n.role.as_str() {
            "status" | "log" => Some(accesskit::Live::Polite),
            "alert" => Some(accesskit::Live::Assertive),
            _ => None,
        },
    }
}

/// Whether assistive output may step the node up and down: a slider or
/// a stepper, or a control with a range (not a measure: a progress, a
/// meter).
fn steps(n: &Node) -> bool {
    !n.disabled
        && (matches!(n.role.as_str(), "slider" | "spinbutton")
            || n.numeric.is_some() && !matches!(n.role.as_str(), "progressbar" | "meter"))
}

/// The page scrolls a scroller can make from where it is (none up at its
/// top, none down at its end).
fn page_scrolls(n: &Node, max_x: f32, max_y: f32) -> Vec<Action> {
    let mut out = Vec::new();
    if n.offset.1 > 0.5 {
        out.push(Action::ScrollUp);
    }
    if n.offset.1 < max_y - 0.5 {
        out.push(Action::ScrollDown);
    }
    if n.offset.0 > 0.5 {
        out.push(Action::ScrollLeft);
    }
    if n.offset.0 < max_x - 0.5 {
        out.push(Action::ScrollRight);
    }
    out
}

/// How far a scroller can move, across and down.
fn scroll_max(sc: &scene::Scene, n: &Node) -> (f32, f32) {
    let (cw, ch) = sc.content_size(n);
    ((cw - n.rect[2]).max(0.0), (ch - n.rect[3]).max(0.0))
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

/// Each field's visual lines (the text engine's: their boxes and their
/// characters' places), by key, for the runs: what lets Zoom and
/// VoiceOver box a character and follow the caret.
pub type Lines = HashMap<String, Vec<(super::text::FieldLine, [f32; 4])>>;

fn lines_of(st: &mut WinState) -> Lines {
    let keys: Vec<String> = st
        .editors
        .keys()
        .filter(|k| st.scene.nodes.get(*k).is_some_and(|n| !n.decorative))
        .cloned()
        .collect();
    let mut out = HashMap::new();
    for k in keys {
        if let Some(l) = st.field_lines(&k) {
            out.insert(k, l);
        }
    }
    out
}

/// The tree for the platform.
pub fn tree(st: &mut WinState) -> TreeUpdate {
    let lines = lines_of(st);
    tree_with(st, &lines)
}

fn tree_with(st: &WinState, lines: &Lines) -> TreeUpdate {
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
    let places = places(&st.scene);
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
        if let Some(f) = field_text(st, key, lines.get(key)) {
            let (runs, sel) = text_runs(st, key, &f);
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
        // a heading's or a tree item's level: 1 the top, as the program
        // says it (AccessKit counts from 0)
        if let Some(l) = n.level {
            node.set_level(l.saturating_sub(1) as usize);
        }
        if let Some(p) = places.get(key) {
            if let Some((at, size)) = p.in_set {
                node.set_position_in_set(at);
                node.set_size_of_set(size);
            }
            if let Some(r) = p.row {
                node.set_row_index(r);
            }
            if let Some(c) = p.column {
                node.set_column_index(c);
            }
            if let Some(size) = p.size {
                node.set_size_of_set(size);
            }
            if let Some(r) = p.rows {
                node.set_row_count(r);
            }
            if let Some(c) = p.columns {
                node.set_column_count(c);
            }
        }
        if let Some([v, lo, hi]) = n.numeric {
            node.set_numeric_value(v as f64);
            node.set_min_numeric_value(lo as f64);
            node.set_max_numeric_value(hi as f64);
        }
        if steps(n) {
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
        }
        if let Some(x) = n.expanded
            && !n.disabled
        {
            node.add_action(if x { Action::Collapse } else { Action::Expand });
        }
        if let Some(live) = live_of(n) {
            node.set_live(live);
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
            let said = if a.paused {
                "Animation, paused"
            } else {
                "Animation, playing"
            };
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
            let (max_x, max_y) = scroll_max(&st.scene, n);
            node.set_scroll_x_min(0.0);
            node.set_scroll_y_min(0.0);
            node.set_scroll_x_max(max_x as f64);
            node.set_scroll_y_max(max_y as f64);
            // a page at a time, as VoiceOver's scroll gestures ask: the
            // ways it can still go
            for a in page_scrolls(n, max_x, max_y) {
                node.add_action(a);
            }
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

/// AccessKit's longest run (its lengths and word starts are bytes).
const RUN_MAX: usize = 255;

/// A field's text as a screen reader reads it: its paragraphs, where each
/// sits (a styled field's own), each paragraph's visual lines laid out
/// (their boxes and their characters' places, so Zoom follows the caret
/// itself and VoiceOver boxes a character; a styled field's in view
/// only), the selection as (paragraph, character) ends, and how far it
/// is scrolled down.
pub struct FieldText {
    paras: Vec<String>,
    tops: Vec<Option<(f32, f32)>>,
    lines: Vec<Vec<(super::text::FieldLine, [f32; 4])>>,
    sel: ((usize, usize), (usize, usize)),
    scroll_y: f32,
}

fn field_text(
    st: &WinState,
    key: &str,
    lines: Option<&Vec<(super::text::FieldLine, [f32; 4])>>,
) -> Option<FieldText> {
    let ed = st.editors.get(key)?;
    if ed.secure() {
        return None;
    }
    let mut f = if let Some(r) = ed.rich() {
        let n = r.para_count();
        FieldText {
            paras: (0..n).map(|i| r.para_text(i).to_string()).collect(),
            tops: (0..n).map(|i| r.para_top(i)).collect(),
            lines: vec![Vec::new(); n],
            sel: r.selection_in_paras(),
            scroll_y: r.scroll_y,
        }
    } else {
        // a plain field (an input, a search): its lines, the selection's
        // character offsets placed in them
        let value = ed.value();
        let paras: Vec<String> = value.split('\n').map(str::to_string).collect();
        let (a, f) = ed.selection_chars();
        let place = |mut c: usize| -> (usize, usize) {
            for (i, p) in paras.iter().enumerate() {
                let n = p.chars().count();
                if c <= n || i + 1 == paras.len() {
                    return (i, c.min(n));
                }
                c -= n + 1;
            }
            (0, 0)
        };
        FieldText {
            tops: vec![None; paras.len()],
            lines: vec![Vec::new(); paras.len()],
            sel: (place(a), place(f)),
            paras,
            scroll_y: 0.0,
        }
    };
    for l in lines.into_iter().flatten() {
        if let Some(at) = f.lines.get_mut(l.0.para) {
            at.push(l.clone());
        }
    }
    for p in f.lines.iter_mut() {
        p.sort_by_key(|l| l.0.start);
        p.dedup_by_key(|l| l.0.start);
    }
    Some(f)
}

/// One run of a paragraph: its first and end characters there, and where
/// it is drawn — a visual line's box and its characters' left edges
/// (from the box's left) and widths, or `None` (a paragraph not laid out
/// now: the paragraph's box, else the field's).
struct RunSpan {
    c0: usize,
    c1: usize,
    line: Option<([f32; 4], Vec<f32>, Vec<f32>)>,
    /// The run before it on the same line (a line longer than a run).
    joined: bool,
}

/// The runs of paragraph `i` (`chars` its characters, its line break
/// included): a run a visual line when it is laid out, a run the
/// paragraph when not, each in pieces of at most `RUN_MAX` characters.
fn spans_of(f: &FieldText, i: usize, chars: usize) -> Vec<RunSpan> {
    let mut whole: Vec<(usize, usize, Option<([f32; 4], Vec<f32>, Vec<f32>)>)> = Vec::new();
    let lines = &f.lines[i];
    if lines.is_empty() || lines[0].0.start != 0 {
        whole.push((0, chars, None));
    } else {
        for (k, (l, b)) in lines.iter().enumerate() {
            let c0 = l.start.min(chars);
            let c1 = lines
                .get(k + 1)
                .map_or(chars, |n| n.0.start.min(chars))
                .max(c0);
            // each character placed: one the layout gave no glyph (a line
            // break, a trailing space) sits after the last, no wider
            let n = c1 - c0;
            let mut xs = Vec::with_capacity(n);
            let mut ws = Vec::with_capacity(n);
            for j in 0..n {
                match (l.xs.get(j), l.widths.get(j)) {
                    (Some(x), Some(w)) => {
                        xs.push(*x);
                        ws.push(*w);
                    }
                    _ => {
                        let end = xs.last().zip(ws.last()).map_or(0.0, |(x, w)| x + w);
                        xs.push(end);
                        ws.push(0.0);
                    }
                }
            }
            whole.push((c0, c1, Some((*b, xs, ws))));
        }
    }
    let mut out = Vec::new();
    for (c0, c1, line) in whole {
        let mut at = c0;
        let mut joined = false;
        loop {
            let end = (at + RUN_MAX).min(c1);
            out.push(RunSpan {
                c0: at,
                c1: end,
                line: line.as_ref().map(|(b, xs, ws)| {
                    (
                        *b,
                        xs[at - c0..end - c0].to_vec(),
                        ws[at - c0..end - c0].to_vec(),
                    )
                }),
                joined,
            });
            joined = true;
            at = end;
            if at >= c1 {
                break;
            }
        }
    }
    out
}

/// The runs of a field: a run a visual line (in pieces of at most 255
/// characters, AccessKit's limit, joined on their line), a paragraph's
/// last ending with its line break; each character's place along its
/// line; and the selection as positions in them.
fn text_runs(
    st: &WinState,
    key: &str,
    f: &FieldText,
) -> (
    Vec<(NodeId, accesskit::Node)>,
    Option<accesskit::TextSelection>,
) {
    let s = st.scale as f64;
    let origin = st.scene.absolute(key);
    let n = f.paras.len();
    let ((ai, ac), (fi, fc)) = f.sel;
    let mut out: Vec<(NodeId, accesskit::Node)> = Vec::new();
    let (mut anchor, mut focus) = (None, None);
    for i in 0..n {
        let mut chars: Vec<char> = f.paras[i].chars().collect();
        if i + 1 < n {
            chars.push('\n');
        }
        let spans = spans_of(f, i, chars.len());
        let count = spans.len();
        for (k, sp) in spans.into_iter().enumerate() {
            let (c0, c1) = (sp.c0, sp.c1);
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
            match (&sp.line, origin, f.tops[i]) {
                // a laid-out line: its own box, each character along it
                // (left to right: the places are from the box's left)
                (Some((b, xs, ws)), _, _) => {
                    run.set_bounds(Rect {
                        x0: b[0] as f64,
                        y0: b[1] as f64,
                        x1: (b[0] + b[2]) as f64,
                        y1: (b[1] + b[3]) as f64,
                    });
                    run.set_character_positions(xs.clone());
                    run.set_character_widths(ws.clone());
                    run.set_text_direction(accesskit::TextDirection::LeftToRight);
                }
                (None, Some(abs), Some((top, h))) => {
                    let y = abs[1] as f64 * s + (top - f.scroll_y) as f64;
                    run.set_bounds(Rect {
                        x0: abs[0] as f64 * s,
                        y0: y,
                        x1: (abs[0] + abs[2]) as f64 * s,
                        y1: y + h as f64,
                    });
                }
                (None, Some(abs), None) => run.set_bounds(Rect {
                    x0: abs[0] as f64 * s,
                    y0: abs[1] as f64 * s,
                    x1: (abs[0] + abs[2]) as f64 * s,
                    y1: (abs[1] + abs[3]) as f64 * s,
                }),
                _ => {}
            }
            // a line longer than a run: its pieces one line
            if sp.joined
                && let Some((prev_id, prev)) = out.last_mut()
            {
                prev.set_next_on_line(id);
                run.set_previous_on_line(*prev_id);
            }
            let last = k + 1 == count;
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
fn run_place(f: &FieldText, key: &str, id: NodeId) -> Option<(usize, usize)> {
    for (i, p) in f.paras.iter().enumerate() {
        let n = p.chars().count() + usize::from(i + 1 < f.paras.len());
        for sp in spans_of(f, i, n) {
            if run_id(key, i, sp.c0) == id {
                return Some((i, sp.c0));
            }
        }
    }
    None
}

/// The box of the selection's focus as a collapsed range, as AccessKit
/// works it out (`Range::bounding_boxes`): its run's box, narrowed to
/// the character's left edge (after the last, its right edge). Device
/// pixels; `None` when its run has no characters' places.
pub fn caret_box(
    runs: &[(NodeId, accesskit::Node)],
    sel: &accesskit::TextSelection,
) -> Option<[f64; 4]> {
    let (_, run) = runs.iter().find(|(id, _)| *id == sel.focus.node)?;
    let b = run.bounds()?;
    let xs = run.character_positions()?;
    let ws = run.character_widths()?;
    let i = sel.focus.character_index;
    let x = if i < xs.len() {
        xs[i]
    } else if i > 0 {
        xs[i - 1] + ws[i - 1]
    } else {
        0.0
    };
    Some([b.x0 + x as f64, b.y0, 0.0, b.y1 - b.y0])
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
            let lines = st.field_lines(&key);
            if let Some(ActionData::SetTextSelection(sel)) = &req.data
                && let Some(f) = field_text(st, &key, lines.as_ref())
                && let Some((ai, ac)) = run_place(&f, &key, sel.anchor.node)
                && let Some((fi, fc)) = run_place(&f, &key, sel.focus.node)
            {
                let (a, b) = (
                    (ai, ac + sel.anchor.character_index),
                    (fi, fc + sel.focus.character_index),
                );
                match st.editors.get_mut(&key) {
                    Some(e) if e.rich().is_some() => {
                        if let Some(r) = e.rich_mut() {
                            r.select_paras(a, b);
                        }
                    }
                    Some(e) => {
                        // a plain field's selection: as character offsets
                        let off = |(i, c): (usize, usize)| -> usize {
                            f.paras[..i]
                                .iter()
                                .map(|p| p.chars().count() + 1)
                                .sum::<usize>()
                                + c
                        };
                        if let (Some(p), Ok(mut ts)) = (e.plain_mut(), super::text::system().lock())
                        {
                            p.select_chars(off(a), off(b), &mut ts);
                        }
                    }
                    None => return,
                }
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
        // a tree's item or a disclosure opened or closed: the program's,
        // as a press of its chevron is
        Action::Expand | Action::Collapse => out.push(ev(
            "a11y",
            vec![
                ("key", s(&key)),
                (
                    "action",
                    s(if req.action == Action::Expand {
                        "expand"
                    } else {
                        "collapse"
                    }),
                ),
            ],
        )),
        // a page at a time (VoiceOver's three-finger swipes): the engine
        // scrolls as the wheel does, and says where it went
        Action::ScrollUp | Action::ScrollDown | Action::ScrollLeft | Action::ScrollRight => {
            let moved = scroll_page(&mut st.scene, &key, req.action);
            st.said_scrolled(moved, out);
            st.dirty = true;
        }
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

/// Scroll `key` (or the scroller around it) a page in the direction the
/// action says, a line's worth short of the view so a line stays.
pub fn scroll_page(sc: &mut scene::Scene, key: &str, a: Action) -> Vec<(String, (f32, f32))> {
    let Some(at) = sc.scroller_up(key) else {
        return Vec::new();
    };
    let Some(n) = sc.nodes.get(&at) else {
        return Vec::new();
    };
    let before = n.offset;
    let (pw, ph) = (
        (n.rect[2] - 40.0).max(n.rect[2] * 0.5),
        (n.rect[3] - 40.0).max(n.rect[3] * 0.5),
    );
    let (dx, dy) = match a {
        Action::ScrollUp => (0.0, -ph),
        Action::ScrollDown => (0.0, ph),
        Action::ScrollLeft => (-pw, 0.0),
        _ => (pw, 0.0),
    };
    if let Some(n) = sc.nodes.get_mut(&at) {
        n.offset.0 += dx;
        n.offset.1 += dy;
    }
    sc.clamp_scroll(&at);
    let after = sc.nodes[&at].offset;
    if after == before {
        Vec::new()
    } else {
        vec![(at, after)]
    }
}

/// The tree as olang values, for tests: one map per node, in tree order.
pub fn as_value(st: &mut WinState) -> Value {
    let mut out = Vec::new();
    let places = places(&st.scene);
    let lines = lines_of(st);
    if let Some(r) = st.scene.root.clone() {
        walk(st, &places, &lines, &r, 0, &mut out);
    }
    Value::List(Arc::new(out))
}

fn walk(
    st: &WinState,
    places: &HashMap<String, Place>,
    lines: &Lines,
    key: &str,
    depth: i64,
    out: &mut Vec<Value>,
) {
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
    if steps(n) {
        actions.push(s("increment"));
        actions.push(s("decrement"));
    }
    if let Some(x) = n.expanded
        && !n.disabled
    {
        actions.push(s(if x { "collapse" } else { "expand" }));
    }
    if n.scrollable {
        let (mx, my) = scroll_max(&st.scene, n);
        for a in page_scrolls(n, mx, my) {
            actions.push(s(match a {
                Action::ScrollUp => "scroll_up",
                Action::ScrollDown => "scroll_down",
                Action::ScrollLeft => "scroll_left",
                _ => "scroll_right",
            }));
        }
    }
    let place = places.get(key).copied().unwrap_or_default();
    let int = |v: Option<usize>| v.map(|v| Value::Integer(v as i64)).unwrap_or(Value::Unit);
    let pair = |v: Option<(usize, usize)>| {
        v.map(|(a, b)| {
            Value::Tuple(Arc::new(vec![
                Value::Integer(a as i64),
                Value::Integer(b as i64),
            ]))
        })
        .unwrap_or(Value::Unit)
    };
    let bounds = match st.scene.absolute(key) {
        Some(b) => Value::Tuple(Arc::new(
            b.iter().map(|v| super::values::float(*v)).collect(),
        )),
        None => Value::Unit,
    };
    let ft = field_text(st, key, lines.get(key));
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
        // its box in the window (logical pixels): what Zoom follows
        ("bounds", bounds),
        (
            "placeholder",
            opt_str(
                n.edit
                    .as_ref()
                    .map(|e| e.placeholder.as_str())
                    .filter(|p| !p.is_empty()),
            ),
        ),
        (
            "level",
            n.level
                .map(|l| Value::Integer(l as i64))
                .unwrap_or(Value::Unit),
        ),
        (
            "expanded",
            n.expanded.map(Value::Boolean).unwrap_or(Value::Unit),
        ),
        (
            "live",
            match live_of(n) {
                Some(accesskit::Live::Off) => s("off"),
                Some(accesskit::Live::Polite) => s("polite"),
                Some(accesskit::Live::Assertive) => s("assertive"),
                None => Value::Unit,
            },
        ),
        // its place in its set (0-based, and the set's size), a table's
        // row and column, a container's size, a table's rows and columns
        ("in_set", pair(place.in_set)),
        ("row", int(place.row)),
        ("column", int(place.column)),
        ("size", int(place.size)),
        ("rows", int(place.rows)),
        ("columns", int(place.columns)),
        (
            "range",
            match n.numeric {
                Some([v, lo, hi]) => Value::Tuple(Arc::new(vec![
                    super::values::float(v),
                    super::values::float(lo),
                    super::values::float(hi),
                ])),
                None => Value::Unit,
            },
        ),
        (
            "text_runs",
            match &ft {
                Some(f) => Value::Integer(text_runs(st, key, f).0.len() as i64),
                None => Value::Unit,
            },
        ),
        // the caret's box as the platform works it out from the runs (a
        // collapsed range's: what Zoom follows), logical pixels
        (
            "caret_box",
            match ft.as_ref().and_then(|f| {
                let (runs, sel) = text_runs(st, key, f);
                caret_box(&runs, sel.as_ref()?)
            }) {
                Some(b) => Value::Tuple(Arc::new(
                    b.iter()
                        .map(|v| super::values::float(*v as f32 / st.scale))
                        .collect(),
                )),
                None => Value::Unit,
            },
        ),
        (
            "text_selection",
            match &ft {
                Some(f) => {
                    let ((ai, ac), (fi, fc)) = f.sel;
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
        walk(st, places, lines, c, depth + 1, out);
    }
}

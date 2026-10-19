//! The accessibility tree's depth (headless): where a node stands in its
//! set and its table, the roles VoiceOver's rotor gathers (landmarks,
//! headings, controls), live regions, the actions an assistive client
//! may ask for (step, open and close, scroll a page), and the boxes Zoom
//! follows. `gui.read(w, "a11y")` says what the platform is given; the
//! AccessKit tree itself is checked where the platform reads it from.

#![cfg(feature = "gui")]

use accesskit::{Action, Live, NodeId, Role};
use olang::stdlib::gui::a11y;
use olang::stdlib::gui::window::WinState;
use olang::{Interpreter, Parser, Value};

/// The engine has one event channel for the process.
static EVENTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn events_turn() -> std::sync::MutexGuard<'static, ()> {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn run(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parses");
    Interpreter::new().eval_program(program).expect("runs")
}

fn text(v: &Value) -> String {
    format!("{v}")
}

/// A window's scene from a patch written as olang (a list of node maps).
fn scene(patch: &str) -> WinState {
    let ops = run(patch);
    let mut st = WinState::new(1, true, "a11y", 600.0, 400.0, 2.0);
    let mut out = Vec::new();
    st.apply(&ops, &mut out).expect("the patch applies");
    st
}

/// The AccessKit node of key `k`.
fn ak<'a>(tree: &'a accesskit::TreeUpdate, k: &str) -> &'a accesskit::Node {
    let id: NodeId = a11y::node_id(k);
    &tree
        .nodes
        .iter()
        .find(|(i, _)| *i == id)
        .unwrap_or_else(|| panic!("no node {k}"))
        .1
}

// A table as Loom's table_view lays it out: a header row of column
// headers (not one of its rows), a body that scrolls, and only the rows
// on screen (rows 40 to 42 of 1,000), each saying its place.
const TABLE: &str = r##"[
  #{ "key": "root", "box": (0, 0, 600, 400) },
  #{ "key": "t", "parent": "root", "role": "table", "box": (0, 0, 600, 300), "name": "Rows" },
  #{ "key": "t:h", "parent": "t", "role": "row", "box": (0, 0, 600, 24) },
  #{ "key": "t:h:0", "parent": "t:h", "role": "columnheader", "box": (0, 0, 200, 24), "text": "Name" },
  #{ "key": "t:h:1", "parent": "t:h", "role": "columnheader", "box": (200, 0, 200, 24), "text": "Size" },
  #{ "key": "t:h:2", "parent": "t:h", "role": "columnheader", "box": (400, 0, 200, 24), "text": "Kind" },
  #{ "key": "t:b", "parent": "t", "box": (0, 24, 600, 276), "scroll": true, "content": (600, 24000) },
  #{ "key": "t:b:40", "parent": "t:b", "role": "row", "box": (0, 960, 600, 24), "pos": (40, 1000) },
  #{ "key": "t:b:40:0", "parent": "t:b:40", "role": "cell", "box": (0, 0, 200, 24), "text": "a" },
  #{ "key": "t:b:40:1", "parent": "t:b:40", "role": "cell", "box": (200, 0, 200, 24), "text": "1" },
  #{ "key": "t:b:40:2", "parent": "t:b:40", "role": "cell", "box": (400, 0, 200, 24), "text": "x" },
  #{ "key": "t:b:41", "parent": "t:b", "role": "row", "box": (0, 984, 600, 24), "pos": (41, 1000) },
  #{ "key": "t:b:41:0", "parent": "t:b:41", "role": "cell", "box": (0, 0, 200, 24), "text": "b" },
  #{ "key": "t:b:41:1", "parent": "t:b:41", "role": "cell", "box": (200, 0, 200, 24), "text": "2" },
  #{ "key": "t:b:41:2", "parent": "t:b:41", "role": "cell", "box": (400, 0, 200, 24), "text": "y" },
  #{ "key": "t:b:42", "parent": "t:b", "role": "row", "box": (0, 1008, 600, 24), "pos": (42, 1000) },
  #{ "key": "t:b:42:0", "parent": "t:b:42", "role": "cell", "box": (0, 0, 200, 24), "text": "c" }
]"##;

#[test]
fn a_virtualized_tables_rows_and_columns() {
    let st = scene(TABLE);
    let places = a11y::places(&st.scene);
    let p = |k: &str| places.get(k).copied().unwrap_or_default();
    // the table: all its rows, not the three on screen; its columns
    assert_eq!(
        (p("t").rows, p("t").columns, p("t").size),
        (Some(1000), Some(3), Some(1000))
    );
    // a row: its place, as it said
    assert_eq!(
        (p("t:b:41").row, p("t:b:41").in_set),
        (Some(41), Some((41, 1000)))
    );
    // a cell: its column, and its row's index
    assert_eq!(
        (p("t:b:41:2").column, p("t:b:41:2").row),
        (Some(2), Some(41))
    );
    // the headers: their columns; their row is no row of the table's
    assert_eq!(p("t:h:1").column, Some(1));
    assert_eq!((p("t:h").row, p("t:h").in_set), (None, None));
    // as the platform reads them
    let tree = a11y::tree(&st);
    let t = ak(&tree, "t");
    assert_eq!(
        (t.role(), t.row_count(), t.column_count()),
        (Role::Table, Some(1000), Some(3))
    );
    let r = ak(&tree, "t:b:42");
    assert_eq!(
        (r.row_index(), r.position_in_set(), r.size_of_set()),
        (Some(42), Some(42), Some(1000))
    );
    let c = ak(&tree, "t:b:40:1");
    assert_eq!(
        (c.role(), c.row_index(), c.column_index()),
        (Role::Cell, Some(40), Some(1))
    );
    assert_eq!(ak(&tree, "t:h:2").column_index(), Some(2));
}

#[test]
fn lists_and_trees_count_their_places() {
    let st = scene(
        r##"[
      #{ "key": "root", "box": (0, 0, 600, 400) },
      #{ "key": "l", "parent": "root", "role": "list", "box": (0, 0, 200, 300), "name": "Files" },
      #{ "key": "l:sc", "parent": "l", "box": (0, 0, 200, 300) },
      #{ "key": "l:0", "parent": "l:sc", "role": "listitem", "box": (0, 0, 200, 20), "text": "a" },
      #{ "key": "l:1", "parent": "l:sc", "role": "listitem", "box": (0, 20, 200, 20), "text": "b" },
      #{ "key": "l:x", "parent": "l:sc", "role": "listitem", "box": (0, 40, 200, 20), "decorative": true },
      #{ "key": "l:2", "parent": "l:sc", "role": "listitem", "box": (0, 60, 200, 20), "text": "c" },
      #{ "key": "tr", "parent": "root", "role": "tree", "box": (200, 0, 200, 300), "name": "Project" },
      #{ "key": "tr:0", "parent": "tr", "role": "treeitem", "box": (0, 0, 200, 20), "text": "src", "level": 1, "expanded": true },
      #{ "key": "tr:1", "parent": "tr", "role": "treeitem", "box": (0, 20, 200, 20), "text": "a.ol", "level": 2 },
      #{ "key": "tr:2", "parent": "tr", "role": "treeitem", "box": (0, 40, 200, 20), "text": "b.ol", "level": 2 },
      #{ "key": "tr:3", "parent": "tr", "role": "treeitem", "box": (0, 60, 200, 20), "text": "tests", "level": 1, "expanded": false },
      #{ "key": "tr:4", "parent": "tr", "role": "treeitem", "box": (0, 80, 200, 20), "text": "README", "level": 1 },
      #{ "key": "tabs", "parent": "root", "role": "tablist", "box": (400, 0, 200, 30) },
      #{ "key": "tab0", "parent": "tabs", "role": "tab", "box": (0, 0, 100, 30), "text": "One", "selected": true },
      #{ "key": "tab1", "parent": "tabs", "role": "tab", "box": (100, 0, 100, 30), "text": "Two", "selected": false }
    ]"##,
    );
    let places = a11y::places(&st.scene);
    let in_set = |k: &str| places.get(k).and_then(|p| p.in_set);
    // a decorative row is not one of the set
    assert_eq!(
        (in_set("l:0"), in_set("l:2"), in_set("l:x")),
        (Some((0, 3)), Some((2, 3)), None)
    );
    // a flat tree: each item among its own level's run of siblings
    assert_eq!(
        (
            in_set("tr:0"),
            in_set("tr:1"),
            in_set("tr:2"),
            in_set("tr:3"),
            in_set("tr:4")
        ),
        (
            Some((0, 3)),
            Some((0, 2)),
            Some((1, 2)),
            Some((1, 3)),
            Some((2, 3))
        )
    );
    assert_eq!(in_set("tab1"), Some((1, 2)));
    let tree = a11y::tree(&st);
    // a tree item's level as AccessKit counts it (from 0), and what opens
    // or closes it
    let i = ak(&tree, "tr:1");
    assert_eq!(
        (i.level(), i.position_in_set(), i.size_of_set()),
        (Some(1), Some(0), Some(2))
    );
    assert!(ak(&tree, "tr:0").supports_action(Action::Collapse));
    assert!(ak(&tree, "tr:3").supports_action(Action::Expand));
    assert!(!ak(&tree, "tr:4").supports_action(Action::Expand));
    assert_eq!(ak(&tree, "l").size_of_set(), Some(3));
}

#[test]
fn landmarks_headings_and_live_regions() {
    let st = scene(
        r##"[
      #{ "key": "root", "box": (0, 0, 600, 400) },
      #{ "key": "nav", "parent": "root", "role": "navigation", "box": (0, 0, 200, 400), "name": "Sidebar" },
      #{ "key": "main", "parent": "root", "role": "main", "box": (200, 0, 400, 400) },
      #{ "key": "h", "parent": "main", "role": "heading", "box": (0, 0, 400, 30), "text": "Settings", "level": 2 },
      #{ "key": "reg", "parent": "main", "role": "region", "box": (0, 30, 400, 100), "scroll": false },
      #{ "key": "named", "parent": "main", "role": "region", "box": (0, 130, 400, 100), "scroll": false, "name": "Output" },
      #{ "key": "st", "parent": "main", "role": "status", "box": (0, 230, 400, 20), "text": "Saved" },
      #{ "key": "quiet", "parent": "main", "role": "status", "box": (0, 250, 400, 20), "text": "3 files", "live": "off" },
      #{ "key": "lg", "parent": "main", "role": "log", "box": (0, 270, 400, 60) },
      #{ "key": "lg:0", "parent": "lg", "role": "text", "box": (0, 0, 400, 20), "text": "built" },
      #{ "key": "al", "parent": "main", "role": "alert", "box": (0, 330, 400, 20), "text": "Changed on disk" },
      #{ "key": "pol", "parent": "main", "role": "group", "box": (0, 350, 400, 20), "live": "polite" },
      #{ "key": "m", "parent": "main", "role": "meter", "box": (0, 370, 100, 8), "name": "Memory", "value": "40%", "range": (0.4, 0, 1) }
    ]"##,
    );
    let tree = a11y::tree(&st);
    assert_eq!(ak(&tree, "nav").role(), Role::Navigation);
    assert_eq!(ak(&tree, "main").role(), Role::Main);
    // an unnamed region is no landmark; a named one is
    assert_eq!(ak(&tree, "reg").role(), Role::GenericContainer);
    assert_eq!(ak(&tree, "named").role(), Role::Region);
    // a heading's level 2 is AccessKit's 1
    let h = ak(&tree, "h");
    assert_eq!((h.role(), h.level()), (Role::Heading, Some(1)));
    assert_eq!(ak(&tree, "st").live(), Some(Live::Polite));
    assert_eq!(ak(&tree, "quiet").live(), Some(Live::Off));
    assert_eq!(ak(&tree, "lg").live(), Some(Live::Polite));
    // a log's lines inherit its politeness (AccessKit's consumer does)
    assert_eq!(ak(&tree, "lg:0").live(), None);
    assert_eq!(ak(&tree, "al").live(), Some(Live::Assertive));
    assert_eq!(ak(&tree, "pol").live(), Some(Live::Polite));
    let m = ak(&tree, "m");
    assert_eq!(
        (m.role(), m.numeric_value(), m.value()),
        (Role::Meter, Some(0.4f32 as f64), Some("40%"))
    );
    // a measure is not stepped
    assert!(!m.supports_action(Action::Increment));
}

#[test]
fn the_actions_a_client_may_ask_for() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (400, 300) })
        let ev = gui.events()
        let mut stale = true
        while stale { match chan.try_recv(ev) { Ok(()) => { stale = false }, Ok(e) => (), _ => { stale = false } } }
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 400, 300) },
          #{ "key": "step", "parent": "root", "role": "spinbutton", "box": (0, 0, 100, 24), "name": "Font size", "value": "13" },
          #{ "key": "item", "parent": "root", "role": "treeitem", "box": (0, 30, 100, 20), "text": "src", "expanded": false },
          #{ "key": "sc", "parent": "root", "role": "region", "box": (100, 0, 300, 200), "name": "Log", "content": (300, 1000) },
          #{ "key": "sc:0", "parent": "sc", "box": (0, 0, 300, 20), "role": "text", "text": "line" },
          #{ "key": "off", "parent": "root", "role": "spinbutton", "box": (0, 60, 100, 24), "disabled": true }
        ])
        let acts = (k) => map_get(filter(gui.read(w, "a11y"), (n) => map_get(n, "key") == k)[0], "actions")
        let before = [acts("step"), acts("item"), acts("sc"), acts("off")]
        gui.input(w, #{ "kind": "a11y", "key": "step", "action": "increment" })
        gui.input(w, #{ "kind": "a11y", "key": "step", "action": "decrement" })
        gui.input(w, #{ "kind": "a11y", "key": "item", "action": "expand" })
        gui.input(w, #{ "kind": "a11y", "key": "sc:0", "action": "scroll_down" })
        let mut got = []
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(()) => { more = false }, Ok(e) => { got = got + [e] }, _ => { more = false } } }
        let said = map(filter(got, (e) => map_get(e, "kind") == "a11y" || map_get(e, "kind") == "scrolled"),
                       (e) => [e.kind, e.key, e.action ?? e.y])
        let line = filter(gui.read(w, "a11y"), (n) => map_get(n, "key") == "sc:0")[0]
        [before, said, map_get(line, "bounds")]
    "##);
    assert_eq!(
        text(&v),
        "[[[\"focus\", \"increment\", \"decrement\"], [\"click\", \"expand\"], [\"scroll_up\", \"scroll_down\"], []], \
         [[\"a11y\", \"step\", \"increment\"], [\"a11y\", \"step\", \"decrement\"], [\"a11y\", \"item\", \"expand\"], [\"scrolled\", \"sc\", 160.0]], \
         (100.0, -160.0, 300.0, 20.0)]"
    );
}

#[test]
fn a_plain_fields_text_is_runs_zoom_can_follow() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (300, 120) })
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 120) },
          #{ "key": "f", "parent": "root", "role": "input", "box": (10, 10, 260, 24), "edit": #{ "value": "hello world" } },
          #{ "key": "pw", "parent": "root", "role": "input", "box": (10, 40, 260, 24), "edit": #{ "value": "secret", "secure": true } },
          #{ "op": "focus", "key": "f" }
        ])
        gui.input(w, #{ "kind": "key", "key": "right" })
        gui.input(w, #{ "kind": "key", "key": "right" })
        gui.input(w, #{ "kind": "key", "key": "right" })
        let node = (k) => filter(gui.read(w, "a11y"), (n) => map_get(n, "key") == k)[0]
        [map_get(node("f"), "text_runs"), map_get(node("f"), "text_selection"), map_get(node("f"), "bounds"), map_get(node("pw"), "text_runs")]
    "##);
    assert_eq!(text(&v), "[1, (0, 3, 0, 3), (10.0, 10.0, 260.0, 24.0), ()]");
    // the platform's tree: the run lies over the field (device pixels)
    let mut st = WinState::new(1, true, "f", 300.0, 120.0, 2.0);
    let mut out = Vec::new();
    st.apply(
        &run(r##"[#{ "key": "root", "box": (0, 0, 300, 120) },
                 #{ "key": "f", "parent": "root", "role": "input", "box": (10, 10, 260, 24), "edit": #{ "value": "ab\ncd" } }]"##),
        &mut out,
    )
    .unwrap();
    let tree = a11y::tree(&st);
    let f = ak(&tree, "f");
    assert_eq!(f.children().len(), 2, "a run a line");
    let run0 = tree
        .nodes
        .iter()
        .find(|(i, _)| *i == f.children()[0])
        .unwrap();
    assert_eq!(run0.1.role(), Role::TextRun);
    let b = run0.1.bounds().unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (20.0, 20.0, 540.0, 68.0));
    assert!(f.text_selection().is_some());
}

#[test]
fn a_new_role_is_known_to_the_engine() {
    let v = run(r##"
        let roles = gui.roles()
        [contains(roles, "navigation"), contains(roles, "spinbutton"), contains(roles, "grid"), map_get(gui.platform(), "a11y_depth")]
    "##);
    assert_eq!(text(&v), "[true, true, true, true]");
}

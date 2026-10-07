//! `gui` (Loom's engine, L0): headless windows driven through the same
//! input path a platform window uses, the accessibility tree, and the
//! GPU renderer checked against the software reference.
//!
//! Every test here runs without a display. The renderer comparison
//! needs a GPU adapter and skips (saying so) where there is none.

#![cfg(feature = "gui")]

use olang::stdlib::gui::window::{Clipboard, Input, Mods, PointerAction, WinState};
use olang::stdlib::gui::{gpu, soft};
use olang::{Interpreter, Parser, Value};
use std::sync::Arc;

/// The engine has one event channel for the process; tests that read it
/// take turns, or one would take another's events.
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

struct NoClip;
impl Clipboard for NoClip {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _: String) {}
}

fn m(fields: Vec<(&str, Value)>) -> Value {
    let mut h = olang::ast::ValueMap::default();
    for (k, v) in fields {
        h.insert(k.to_string(), v);
    }
    Value::Map(Arc::new(h))
}

fn s(t: &str) -> Value {
    Value::String(Arc::new(t.to_string()))
}

fn tup(v: &[f64]) -> Value {
    Value::Tuple(Arc::new(v.iter().map(|x| Value::Float(*x)).collect()))
}

/// A scene with every primitive: fills, borders, radii (uneven ones
/// too), clipping, a translucent layer, text in two sizes, and a field.
fn reference_scene(scale: f32) -> WinState {
    let mut st = WinState::new(1, true, "reference", 360.0, 240.0, scale);
    let style = |pairs: Vec<(&str, Value)>| m(pairs);
    let ops = Value::List(Arc::new(vec![
        m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 360.0, 240.0])),
            ("style", style(vec![("bg", s("#fafafa"))])),
        ]),
        m(vec![
            ("key", s("card")),
            ("parent", s("root")),
            ("box", tup(&[16.0, 16.0, 200.0, 120.0])),
            (
                "style",
                style(vec![
                    ("bg", s("#ffffff")),
                    ("border", s("#d4d4d8")),
                    ("border_width", Value::Integer(1)),
                    ("radius", Value::Integer(10)),
                    ("clip", Value::Boolean(true)),
                ]),
            ),
        ]),
        m(vec![
            ("key", s("title")),
            ("parent", s("card")),
            ("role", s("heading")),
            ("box", tup(&[12.0, 10.0, 180.0, 24.0])),
            ("text", s("Reference")),
            (
                "style",
                style(vec![
                    ("size", Value::Integer(18)),
                    ("weight", Value::Integer(600)),
                ]),
            ),
        ]),
        m(vec![
            ("key", s("body")),
            ("parent", s("card")),
            ("role", s("text")),
            ("box", tup(&[12.0, 40.0, 176.0, 60.0])),
            (
                "text",
                s("Both renderers draw this paragraph the same way."),
            ),
            ("style", style(vec![("wrap", Value::Boolean(true))])),
        ]),
        // Hangs off the card's edge: clipped by it.
        m(vec![
            ("key", s("overflow")),
            ("parent", s("card")),
            ("box", tup(&[150.0, 90.0, 80.0, 60.0])),
            ("style", style(vec![("bg", s("#f59e0b"))])),
        ]),
        m(vec![
            ("key", s("pill")),
            ("parent", s("root")),
            ("role", s("button")),
            ("box", tup(&[232.0, 16.0, 112.0, 36.0])),
            ("text", s("Continue")),
            (
                "style",
                style(vec![
                    ("bg", s("#2563eb")),
                    ("color", s("#ffffff")),
                    ("radius", Value::Integer(18)),
                    ("align", s("center")),
                    ("valign", s("center")),
                ]),
            ),
        ]),
        m(vec![
            ("key", s("odd")),
            ("parent", s("root")),
            ("box", tup(&[232.0, 64.0, 112.0, 72.0])),
            (
                "style",
                style(vec![
                    ("bg", s("#10b98180")),
                    ("border", s("#065f46")),
                    ("border_width", Value::Integer(3)),
                    ("radius", tup(&[0.0, 24.0, 4.0, 12.0])),
                ]),
            ),
        ]),
        m(vec![
            ("key", s("field")),
            ("parent", s("root")),
            ("role", s("input")),
            ("box", tup(&[16.0, 152.0, 328.0, 36.0])),
            ("edit", m(vec![("value", s("Editable text, with a caret"))])),
            (
                "style",
                style(vec![
                    ("bg", s("#ffffff")),
                    ("border", s("#a1a1aa")),
                    ("border_width", Value::Integer(1)),
                    ("radius", Value::Integer(6)),
                    ("pad", tup(&[0.0, 10.0, 0.0, 10.0])),
                ]),
            ),
        ]),
        m(vec![
            ("key", s("hair")),
            ("parent", s("root")),
            ("box", tup(&[16.0, 204.0, 328.0, 1.0])),
            ("style", style(vec![("bg", s("#18181b"))])),
        ]),
    ]));
    let mut out = Vec::new();
    st.apply(&ops, &mut out)
        .expect("the reference scene applies");
    st.apply(
        &Value::List(Arc::new(vec![m(vec![
            ("op", s("focus")),
            ("key", s("field")),
        ])])),
        &mut out,
    )
    .expect("focus");
    st
}

#[test]
fn the_gpu_draws_what_the_software_renderer_draws() {
    let g = match gpu::gpu(None, None) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped: no GPU here ({e})");
            return;
        }
    };
    for scale in [1.0, 2.0] {
        let mut st = reference_scene(scale);
        let dl = st.display_list();
        let soft_px = soft::rgba(&soft::render(&dl));
        let gpu_px = g.render_rgba(&dl);
        assert_eq!(soft_px.len(), gpu_px.len());
        let mut over = 0usize;
        let mut worst = 0u8;
        let mut total: u64 = 0;
        for (a, b) in soft_px.chunks(4).zip(gpu_px.chunks(4)) {
            let d = (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap();
            worst = worst.max(d);
            total += d as u64;
            // Antialiased edges are computed differently (coverage by
            // scanline against coverage by distance); a pixel that
            // differs by more than this is a renderer bug.
            if d > 48 {
                over += 1;
            }
        }
        let n = soft_px.len() / 4;
        let mean = total as f64 / n as f64;
        assert!(
            over * 1000 <= n && mean < 1.0,
            "at {scale}x: {over} of {n} pixels differ by more than 48 (worst {worst}, mean {mean:.3})"
        );
    }
}

#[test]
fn input_through_the_window_edits_focuses_and_activates() {
    let mut st = reference_scene(1.0);
    let mut out = Vec::new();
    let mut clip = NoClip;
    // End, then type.
    st.input(
        Input::Key {
            key: "end".into(),
            text: None,
            mods: Mods::default(),
            repeat: false,
        },
        &mut clip,
        &mut out,
    );
    st.input(
        Input::Key {
            key: "!".into(),
            text: Some("!".into()),
            mods: Mods::default(),
            repeat: false,
        },
        &mut clip,
        &mut out,
    );
    assert_eq!(st.editors["field"].value(), "Editable text, with a caret!");
    // A click on the button: focus moves to it, then it activates.
    for action in [PointerAction::Down, PointerAction::Up] {
        st.input(
            Input::Pointer {
                action,
                x: 280.0,
                y: 30.0,
                button: "left".into(),
                clicks: 1,
            },
            &mut clip,
            &mut out,
        );
    }
    assert_eq!(st.scene.focus.as_deref(), Some("pill"));
    let kinds: Vec<String> = out
        .iter()
        .filter_map(|e| match e {
            Value::Map(m) => match &m["kind"] {
                Value::String(k) => Some(k.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(kinds.contains(&"changed".to_string()), "{kinds:?}");
    assert!(kinds.contains(&"activate".to_string()), "{kinds:?}");
}

#[test]
fn a_stale_value_from_the_program_does_not_undo_typing() {
    let mut st = reference_scene(1.0);
    let mut out = Vec::new();
    let mut clip = NoClip;
    let field_patch = |value: &str, rev: Option<i64>| {
        let mut edit = vec![("value", s(value))];
        if let Some(r) = rev {
            edit.push(("rev", Value::Integer(r)));
        }
        Value::List(Arc::new(vec![m(vec![
            ("key", s("field")),
            ("parent", s("root")),
            ("role", s("input")),
            ("box", tup(&[16.0, 152.0, 328.0, 36.0])),
            ("edit", m(edit)),
        ])]))
    };
    st.apply(&field_patch("", None), &mut out).unwrap();
    for ch in ["a", "b"] {
        st.input(
            Input::Key {
                key: ch.into(),
                text: Some(ch.into()),
                mods: Mods::default(),
                repeat: false,
            },
            &mut clip,
            &mut out,
        );
    }
    let rev_of_a = st.editors["field"].rev() - 1;
    // The program answers the first keystroke after the second landed.
    st.apply(&field_patch("a", Some(rev_of_a)), &mut out)
        .unwrap();
    assert_eq!(st.editors["field"].value(), "ab");
    // A program that transforms the value is obeyed.
    let rev = st.editors["field"].rev();
    st.apply(&field_patch("AB", Some(rev)), &mut out).unwrap();
    assert_eq!(st.editors["field"].value(), "AB");
}

#[test]
fn the_accessibility_tree_names_every_control() {
    let mut st = reference_scene(1.0);
    let tree = olang::stdlib::gui::a11y::tree(&st);
    // The window, plus every node.
    assert_eq!(tree.nodes.len(), st.scene.nodes.len() + 1);
    let v = olang::stdlib::gui::a11y::as_value(&mut st);
    let t = text(&v);
    assert!(t.contains("\"Button\""), "{t}");
    assert!(t.contains("\"Continue\""), "{t}");
    assert!(t.contains("\"TextInput\""), "{t}");
    assert!(t.contains("Editable text, with a caret"), "{t}");
}

#[test]
fn headless_windows_from_olang() {
    // its windows send events on the one channel
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (200, 100) })
        let r = gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 200, 100), "style": #{ "bg": "#ffffff" } },
          #{ "key": "b", "parent": "root", "role": "button", "box": (10, 10, 80, 30), "text": "OK" },
          #{ "key": "f", "parent": "root", "role": "input", "box": (10, 50, 180, 30), "edit": #{ "value": "" } }
        ])
        gui.input(w, #{ "kind": "pointer", "action": "down", "x": 20, "y": 60 })
        gui.input(w, #{ "kind": "pointer", "action": "up", "x": 20, "y": 60 })
        gui.input(w, #{ "kind": "text", "text": "héllo" })
        gui.input(w, #{ "kind": "key", "key": "a", "mod": true })
        gui.input(w, #{ "kind": "key", "key": "x", "mod": true })
        gui.input(w, #{ "kind": "key", "key": "v", "mod": true })
        gui.input(w, #{ "kind": "key", "key": "v", "mod": true })
        let png = gui.read(w, "pixels")
        [r, gui.read(w, "focus"), gui.read(w, "value", "f"), gui.read(w, "keys"), bytes.len(png) > 100, gui.read(w, "hit", (15, 15))]
    "##);
    assert_eq!(
        text(&v),
        r#"[Ok(()), "f", "héllohéllo", ["b", "f"], true, "b"]"#
    );
}

#[test]
fn a_bad_patch_says_what_is_wrong() {
    // its windows send events on the one channel
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (100, 100) })
        [gui.apply(w, [#{ "key": "a", "role": "buton" }]),
         gui.apply(w, [#{ "key": "b", "parent": "nope" }]),
         gui.apply(w, [#{ "key": "c", "style": #{ "colour": "#fff" } }])]
    "##);
    let t = text(&v);
    assert!(t.contains("unknown role \"buton\""), "{t}");
    assert!(t.contains("parent \"nope\", which does not exist"), "{t}");
    assert!(t.contains("unknown style \"colour\""), "{t}");
}

#[test]
fn content_lets_a_list_scroll_past_its_laid_out_rows() {
    // its pointer and wheel events go to the one channel: another test
    // reading it meanwhile would read them
    let _turn = events_turn();
    // A virtualized list lays out a few rows but scrolls through all of
    // them: the wheel is clamped to `content`, not to the children.
    let v = run(r##"
        let w = gui.headless(#{ "size": (200, 100) })
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 200, 100) },
          #{ "key": "l", "parent": "root", "role": "list", "box": (0, 0, 200, 100), "scroll": true, "content": (200, 32000) },
          #{ "key": "r0", "parent": "l", "role": "listitem", "box": (0, 0, 200, 32), "text": "row 0" }
        ])
        gui.input(w, #{ "kind": "pointer", "action": "move", "x": 50, "y": 50 })
        gui.input(w, #{ "kind": "wheel", "dy": -5000 })
        gui.input(w, #{ "kind": "wheel", "dy": -1000000 })
        let n = gui.read(w, "node", "l")
        // the row scrolled out of the list shows nowhere
        [map_get(n, "offset"), map_get(n, "visible"), map_get(gui.read(w, "node", "r0"), "visible")]
    "##);
    assert_eq!(text(&v), "[(0.0, 31900.0), (0.0, 0.0, 200.0, 100.0), ()]");
}

#[test]
fn a_modal_layer_keeps_the_focus_and_hears_a_press_outside() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (300, 200) })
        let ev = gui.events()
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 200) },
          #{ "key": "behind", "parent": "root", "role": "button", "box": (10, 10, 80, 30), "text": "Behind" },
          #{ "key": "dlg", "parent": "root", "role": "dialog", "modal": true, "box": (100, 60, 180, 120), "name": "Confirm" },
          #{ "key": "ok", "parent": "dlg", "role": "button", "box": (10, 70, 70, 30), "text": "OK" },
          #{ "key": "no", "parent": "dlg", "role": "button", "box": (90, 70, 70, 30), "text": "Cancel" }
        ])
        let first = gui.read(w, "focus")
        gui.input(w, #{ "kind": "key", "key": "tab" })
        gui.input(w, #{ "kind": "key", "key": "tab" })
        let after_tabs = gui.read(w, "focus")
        gui.input(w, #{ "kind": "pointer", "action": "down", "x": 20, "y": 20 })
        gui.input(w, #{ "kind": "pointer", "action": "up", "x": 20, "y": 20 })
        let mut kinds = []
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { kinds = kinds + [map_get(e, "kind")] }, _ => { more = false } } }
        [first, after_tabs, gui.read(w, "hit", (20, 20)), contains(kinds, "outside"), contains(kinds, "activate")]
    "##);
    assert_eq!(text(&v), r#"["ok", "ok", (), true, false]"#);
}

#[test]
fn a_drag_is_followed_across_windows_and_a_node_has_actions_of_its_own() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (300, 200) })
        let ev = gui.events()
        let mut stale = true
        while stale { match chan.try_recv(ev) { Ok(e) => (), _ => { stale = false } } }
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 200) },
          #{ "key": "card", "parent": "root", "role": "button", "box": (10, 10, 100, 40), "text": "Card",
             "actions": ["Move left", "Move right"], "description": "To do, 1 of 3" },
          #{ "key": "ghost", "parent": "root", "role": "group", "box": (0, 0, 300, 200), "inert": true }
        ])
        // the picture over everything is never under the pointer
        let hit = gui.read(w, "hit", (20, 20))
        gui.input(w, #{ "kind": "place", "x": 400, "y": 300 })
        gui.input(w, #{ "kind": "pointer", "action": "down", "x": 20, "y": 20 })
        // a drag past the window's edge still reports, with its place on the screen
        gui.input(w, #{ "kind": "pointer", "action": "move", "x": 350, "y": -30 })
        gui.input(w, #{ "kind": "a11y", "key": "card", "action": "custom", "index": 1 })
        let mut got = []
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { got = got + [e] }, _ => { more = false } } }
        let moved = filter(got, (e) => map_get(e, "kind") == "moved")
        let far = filter(got, (e) => map_get(e, "kind") == "pointer" && map_get(e, "action") == "move")
        let acted = filter(got, (e) => map_get(e, "kind") == "a11y")
        let card = filter(gui.read(w, "a11y"), (n) => map_get(n, "key") == "card")[0]
        [hit, map_get(moved[0], "x"), map_get(far[0], "sx"), map_get(far[0], "sy"), map_get(acted[0], "action"), map_get(acted[0], "index"),
         map_get(acted[0], "label"), map_get(card, "custom"), map_get(card, "description")]
    "##);
    assert_eq!(
        text(&v),
        r#"["card", 400.0, 750.0, 270.0, "custom", 1, "Move right", ["Move left", "Move right"], "To do, 1 of 3"]"#
    );
    // the platform's tree carries them as custom actions
    let mut st = reference_scene(1.0);
    let n = st.scene.nodes.values_mut().find(|n| n.role == "button").expect("a button");
    n.actions = vec!["Move right".into()];
    let tree = olang::stdlib::gui::a11y::tree(&st);
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| n.custom_actions().iter().any(|a| a.description == "Move right")),
        "a custom action in the tree"
    );
}

#[test]
fn a_lists_actions_are_its_active_rows_too() {
    // A board column's moves act on its selected card, and a screen
    // reader's cursor is on the card (the active descendant): the card
    // offers the column's actions, and choosing one is the column's.
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (300, 200) })
        let ev = gui.events()
        let mut stale = true
        while stale { match chan.try_recv(ev) { Ok(e) => (), _ => { stale = false } } }
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 200) },
          #{ "key": "col", "parent": "root", "role": "list", "box": (0, 0, 150, 200), "focusable": true,
             "active": "r1", "actions": ["Move to Backlog", "Move to Done"] },
          #{ "key": "r0", "parent": "col", "role": "listitem", "box": (0, 0, 150, 40), "text": "Zero" },
          #{ "key": "r1", "parent": "col", "role": "listitem", "box": (0, 40, 150, 40), "text": "One",
             "actions": [] }
        ])
        gui.input(w, #{ "kind": "a11y", "key": "r1", "action": "custom", "index": 1 })
        let mut got = []
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { got = got + [e] }, _ => { more = false } } }
        let acted = filter(got, (e) => map_get(e, "kind") == "a11y")
        let custom = (k) => map_get(filter(gui.read(w, "a11y"), (n) => map_get(n, "key") == k)[0], "custom")
        [custom("col"), custom("r1"), custom("r0"), len(acted), map_get(acted[0], "key"), map_get(acted[0], "label")]
    "##);
    assert_eq!(
        text(&v),
        r#"[["Move to Backlog", "Move to Done"], ["Move to Backlog", "Move to Done"], [], 1, "col", "Move to Done"]"#
    );
    // in the platform's tree, the active row carries them; the other not
    let mut st = reference_scene(1.0);
    let keys: Vec<String> = st.scene.nodes.keys().take(2).cloned().collect();
    let (list, row) = (&keys[0], &keys[1]);
    let l = st.scene.nodes.get_mut(list).unwrap();
    l.actions = vec!["Move to Done".into()];
    l.active = Some(row.clone());
    (l.decorative, l.disabled) = (false, false);
    let r = st.scene.nodes.get_mut(row).unwrap();
    r.actions.clear();
    (r.decorative, r.disabled) = (false, false);
    let tree = olang::stdlib::gui::a11y::tree(&st);
    let custom = |k: &str| {
        let id = olang::stdlib::gui::a11y::node_id(k);
        tree.nodes
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, n)| n.custom_actions().len())
    };
    assert_eq!((custom(list), custom(row)), (Some(1), Some(1)));
}

/// A canvas (shapes and text) and two images, one fitted inside its box
/// and one covering it.
fn picture_scene(scale: f32, png: &str) -> WinState {
    let mut st = WinState::new(1, true, "pictures", 300.0, 160.0, scale);
    let op = |pairs: Vec<(&str, Value)>| m(pairs);
    let n = |v: f64| Value::Float(v);
    let draw = Value::List(Arc::new(vec![
        op(vec![
            ("op", s("rect")),
            ("x", n(10.0)),
            ("y", n(10.0)),
            ("w", n(60.0)),
            ("h", n(40.0)),
            ("radius", n(6.0)),
            ("fill", s("#dc2626")),
        ]),
        op(vec![
            ("op", s("circle")),
            ("cx", n(100.0)),
            ("cy", n(40.0)),
            ("r", n(24.0)),
            ("stroke", s("#2563eb")),
            ("width", n(3.0)),
        ]),
        op(vec![
            ("op", s("line")),
            ("x1", n(10.0)),
            ("y1", n(90.0)),
            ("x2", n(130.0)),
            ("y2", n(70.0)),
            ("color", s("#16a34a")),
            ("width", n(2.0)),
        ]),
        op(vec![
            ("op", s("path")),
            (
                "points",
                Value::List(Arc::new(vec![
                    tup(&[20.0, 130.0]),
                    tup(&[60.0, 100.0]),
                    tup(&[100.0, 130.0]),
                ])),
            ),
            ("close", Value::Boolean(true)),
            ("fill", s("#f59e0b")),
        ]),
        op(vec![
            ("op", s("text")),
            ("x", n(70.0)),
            ("y", n(134.0)),
            ("text", s("Q3")),
            ("align", s("center")),
            ("size", n(13.0)),
        ]),
    ]));
    let ops = Value::List(Arc::new(vec![
        m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 300.0, 160.0])),
            ("style", m(vec![("bg", s("#ffffff"))])),
        ]),
        m(vec![
            ("key", s("chart")),
            ("parent", s("root")),
            ("role", s("figure")),
            ("name", s("Sales")),
            ("box", tup(&[0.0, 0.0, 140.0, 160.0])),
            ("draw", draw),
        ]),
        m(vec![
            ("key", s("fit")),
            ("parent", s("root")),
            ("role", s("image")),
            ("name", s("Logo")),
            ("box", tup(&[150.0, 10.0, 140.0, 60.0])),
            ("image", s(png)),
            ("fit", s("contain")),
        ]),
        m(vec![
            ("key", s("cover")),
            ("parent", s("root")),
            ("role", s("image")),
            ("box", tup(&[150.0, 90.0, 140.0, 60.0])),
            ("image", s(png)),
            ("fit", s("cover")),
        ]),
    ]));
    let mut out = Vec::new();
    st.apply(&ops, &mut out).expect("the picture scene applies");
    st
}

/// A 40×40 PNG: four coloured quarters.
fn quarters_png(dir: &std::path::Path) -> String {
    let mut pm = tiny_skia::Pixmap::new(40, 40).unwrap();
    let quarters = [
        (0, 0, [220, 38, 38]),
        (20, 0, [37, 99, 235]),
        (0, 20, [22, 163, 74]),
        (20, 20, [245, 158, 11]),
    ];
    for (x, y, c) in quarters {
        let mut p = tiny_skia::Paint::default();
        p.set_color_rgba8(c[0], c[1], c[2], 255);
        pm.fill_rect(
            tiny_skia::Rect::from_xywh(x as f32, y as f32, 20.0, 20.0).unwrap(),
            &p,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    let path = dir.join("quarters.png");
    pm.save_png(&path).unwrap();
    path.to_string_lossy().to_string()
}

#[test]
fn canvases_and_images_draw_alike_on_both_renderers() {
    let dir = tempfile::tempdir().unwrap();
    let png = quarters_png(dir.path());
    let px = |rgba: &[u8], w: u32, x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    };
    for scale in [1.0f32, 2.0] {
        let mut st = picture_scene(scale, &png);
        let dl = st.display_list();
        let soft_px = soft::rgba(&soft::render(&dl));
        let w = dl.width;
        let at = |x: f32, y: f32| px(&soft_px, w, (x * scale) as u32, (y * scale) as u32);
        // The canvas's rectangle, its path, and the background between.
        assert_eq!(at(40.0, 30.0), [220, 38, 38], "at {scale}x");
        assert_eq!(at(60.0, 125.0), [245, 158, 11], "at {scale}x");
        assert_eq!(at(100.0, 40.0), [255, 255, 255], "at {scale}x");
        // Contained: a 60×60 square centred in 140×60, so its sides are
        // background; its top-left quarter is red.
        assert_eq!(at(160.0, 40.0), [255, 255, 255], "at {scale}x");
        assert_eq!(at(200.0, 20.0), [220, 38, 38], "at {scale}x");
        // Covered: 140×140 cropped to the middle 60 rows — left is red at
        // the top, green at the bottom, and it reaches both sides.
        assert_eq!(at(152.0, 92.0), [220, 38, 38], "at {scale}x");
        assert_eq!(at(152.0, 148.0), [22, 163, 74], "at {scale}x");
        assert_eq!(at(288.0, 148.0), [245, 158, 11], "at {scale}x");
        // The canvas text is drawn as text (dark pixels near it).
        let dark = (60..80).any(|x| (134..152).any(|y| at(x as f32, y as f32)[0] < 120));
        assert!(dark, "the canvas text is drawn at {scale}x");

        let Ok(g) = gpu::gpu(None, None) else {
            eprintln!("skipped the GPU half: no GPU here");
            continue;
        };
        let gpu_px = g.render_rgba(&dl);
        let mut over = 0usize;
        for (a, b) in soft_px.chunks(4).zip(gpu_px.chunks(4)) {
            if (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap() > 48 {
                over += 1;
            }
        }
        let n = soft_px.len() / 4;
        assert!(over * 200 <= n, "at {scale}x: {over} of {n} pixels differ");
        assert_eq!(
            px(&gpu_px, w, (200.0 * scale) as u32, (20.0 * scale) as u32),
            [220, 38, 38]
        );
    }
}

#[test]
fn a_test_can_close_command_paste_and_see_the_caret() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (300, 100) })
        let ev = gui.events()
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 100) },
          #{ "key": "f", "parent": "root", "role": "input", "box": (10, 10, 200, 30), "edit": #{ "value": "" } },
          #{ "op": "focus", "key": "f" }
        ])
        let caret = gui.read(w, "caret")
        gui.input(w, #{ "kind": "clipboard", "text": "pasted" })
        gui.input(w, #{ "kind": "key", "key": "v", "mod": true })
        gui.input(w, #{ "kind": "menu", "id": "save" })
        gui.input(w, #{ "kind": "close" })
        let mut kinds = []
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { kinds = kinds + [map_get(e, "kind") + (if map_get(e, "id") == () => "" else => ":" + map_get(e, "id"))] }, _ => { more = false } } }
        gui.apply(w, [#{ "op": "focus" }])
        [caret != () && caret[0] >= 10.0 && caret[3] > 10.0, gui.read(w, "value", "f"), contains(kinds, "menu:save"), contains(kinds, "close_requested"), gui.read(w, "caret")]
    "##);
    assert_eq!(text(&v), r#"[true, "pasted", true, true, ()]"#);
}

#[test]
fn compare_finds_what_changed_and_draws_where() {
    // its windows send events on the one channel
    let _turn = events_turn();
    let v = run(r##"
        fn shot(label) = {
            let w = gui.headless(#{ "size": (120, 40) })
            gui.apply(w, [#{ "key": "root", "box": (0, 0, 120, 40), "style": #{ "bg": "#ffffff" } },
                          #{ "key": "b", "parent": "root", "box": (10, 10, 100, 20), "style": #{ "bg": label } }])
            let png = gui.read(w, "pixels")
            gui.close(w)
            png
        }
        let a = shot("#2563eb")
        let same = gui.compare(a, shot("#2563eb"))
        let moved = gui.compare(a, shot("#dc2626"))
        let small = gui.compare(a, gui.read(gui.headless(#{ "size": (10, 10) }), "pixels"))
        [map_get(same, "same"), map_get(same, "differing"), map_get(moved, "same"), map_get(moved, "differing"), bytes.len(map_get(moved, "diff")) > 100, map_get(small, "same"), map_get(small, "diff")]
    "##);
    assert_eq!(text(&v), "[true, 0, false, 2000, true, false, ()]");
}

#[test]
fn a_node_that_grabs_is_found_before_the_siblings_over_it() {
    // its windows send events on the one channel
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (200, 100) })
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 200, 100) },
          #{ "key": "a", "parent": "root", "role": "button", "box": (0, 0, 100, 100), "text": "A" },
          #{ "key": "div", "parent": "root", "role": "separator", "box": (88, 0, 24, 100), "focusable": true, "grab": true },
          #{ "key": "b", "parent": "root", "role": "button", "box": (100, 0, 100, 100), "text": "B" }
        ])
        [gui.read(w, "hit", (90, 50)), gui.read(w, "hit", (108, 50)), gui.read(w, "hit", (150, 50)), gui.read(w, "keys")]
    "##);
    assert_eq!(text(&v), r#"["div", "div", "b", ["a", "div", "b"]]"#);
}

#[test]
fn a_trailing_space_after_right_to_left_text_keeps_the_caret_in_view() {
    // its windows send events on the one channel
    let _turn = events_turn();
    // A right-to-left line's trailing spaces hang left of its start; the
    // field scrolled no further left than 0 and the caret went out of it.
    let v = run(r##"
        let mut out = []
        for t in ["שלום עולם 123", "مرحبا", "abc"] {
            let w = gui.headless(#{ "size": (300, 80) })
            gui.apply(w, [
              #{ "key": "root", "box": (0, 0, 300, 80) },
              #{ "key": "f", "parent": "root", "role": "input", "box": (10, 10, 200, 34), "edit": #{ "value": "" }, "style": #{ "pad": (0, 10, 0, 10) } },
              #{ "op": "focus", "key": "f" }
            ])
            gui.input(w, #{ "kind": "text", "text": t })
            gui.input(w, #{ "kind": "key", "key": "space", "text": " " })
            let c = gui.read(w, "caret")
            out = out + [c != () && c[0] >= 10.0 && c[0] + c[2] <= 210.0]
            gui.close(w)
        }
        out
    "##);
    assert_eq!(text(&v), "[true, true, true]");
}

#[test]
fn a_zoomed_window_lays_out_in_larger_units_at_the_same_pixels() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (400, 200) })
        let ev = gui.events()
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 400, 200) },
          #{ "key": "b", "parent": "root", "role": "button", "box": (10, 10, 100, 40), "text": "B" }
        ])
        let before = gui.read(w, "rgba")
        gui.set(w, #{ "zoom": 2.0 })
        let size = gui.read(w, "size")
        let after = gui.read(w, "rgba")
        let mut resized = ()
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { if map_get(e, "kind") == "resize" => { resized = (map_get(e, "width"), map_get(e, "height")) } }, _ => { more = false } } }
        [size, resized, map_get(before, "width") == map_get(after, "width"), gui.read(w, "hit", (60, 30)), gui.read(w, "hit", (150, 30))]
    "##);
    // 400×200 logical pixels at zoom 2 are 200×100 units (what reads and
    // the program's boxes are in); the pixels are the same.
    assert_eq!(
        text(&v),
        r#"[(200.0, 100.0, 2.0), (200.0, 100.0), true, "b", "root"]"#
    );
}

#[test]
fn the_system_settings_are_read_and_a_change_is_an_event() {
    let _turn = events_turn();
    let v = run(r##"
        let c = gui.context()
        let w = gui.headless(#{ "size": (100, 100) })
        let ev = gui.events()
        gui.input(w, #{ "kind": "appearance", "dark": true, "contrast": true })
        let mut got = ()
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { if map_get(e, "kind") == "appearance" => { got = e } }, _ => { more = false } } }
        [sort(map_keys(c)), map_get(got, "dark"), map_get(got, "contrast"), map_get(got, "reduce_motion")]
    "##);
    assert_eq!(
        text(&v),
        r#"[["contrast", "dark", "reduce_motion"], true, true, false]"#
    );
}

#[test]
fn under_a_zoom_the_pointer_lands_where_the_pixels_show() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (400, 200) })
        let ev = gui.events()
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 400, 200) },
          #{ "key": "b", "parent": "root", "role": "button", "box": (10, 10, 100, 40), "text": "B" }
        ])
        gui.set(w, #{ "zoom": 2.0 })
        // logical (120, 60) is (60, 30) in units: on the button
        gui.input(w, #{ "kind": "pointer", "action": "down", "x": 120, "y": 60 })
        gui.input(w, #{ "kind": "pointer", "action": "up", "x": 120, "y": 60 })
        let mut hit = ()
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { if map_get(e, "kind") == "activate" => { hit = map_get(e, "key") } }, _ => { more = false } } }
        hit
    "##);
    assert_eq!(text(&v), r#""b""#);
}

// ── pictures and files ───────────────────────────────────────────────

const QUADS: [[u8; 3]; 4] = [[220, 38, 38], [37, 99, 235], [22, 163, 74], [250, 204, 21]];

/// A window of `size` at `scale` showing image `src` at (0, 0) in a box
/// of `bw × bh` as `fit` says, drawn by the software renderer: its RGBA
/// and its width in pixels.
fn picture_pixels(src: Value, fit: &str, size: (f32, f32), bw: f32, bh: f32, scale: f32) -> (Vec<u8>, u32) {
    let mut st = WinState::new(7, true, "pictures", size.0, size.1, scale);
    let patch = Value::List(Arc::new(vec![
        m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, size.0 as f64, size.1 as f64])),
            ("style", m(vec![("bg", s("#ffffff"))])),
        ]),
        m(vec![
            ("key", s("pic")),
            ("parent", s("root")),
            ("role", s("image")),
            ("name", s("quads")),
            ("box", tup(&[0.0, 0.0, bw as f64, bh as f64])),
            ("image", src),
            ("fit", s(fit)),
        ]),
    ]));
    let mut out = Vec::new();
    st.apply(&patch, &mut out).expect("applies");
    let dl = st.display_list();
    (soft::rgba(&soft::render(&dl)), dl.width)
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= tol)
}

fn at(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

#[test]
fn pictures_in_every_format_show_the_same_quadrants() {
    for f in ["png", "jpg", "webp", "gif", "svg"] {
        let path = s(&format!("tests/fixtures/pictures/quads.{f}"));
        let (px, w) = picture_pixels(path, "none", (60.0, 40.0), 60.0, 40.0, 1.0);
        let tol = if f == "jpg" { 24 } else { 6 };
        for (i, (x, y)) in [(5, 4), (34, 4), (5, 25), (34, 25)].into_iter().enumerate() {
            let got = at(&px, w, x, y);
            assert!(near(got, QUADS[i], tol), "{f}: quadrant {i} at ({x}, {y}) is {got:?}, not {:?}", QUADS[i]);
        }
        // its own size: nothing past 40 × 30
        assert!(near(at(&px, w, 50, 35), [255, 255, 255], 2), "{f}: drawn past its size");
    }
}

#[test]
fn its_own_size_is_one_pixel_to_one_display_pixel_and_an_svg_is_drawn_at_the_scale() {
    // a raster at 2×: 40 × 30 display pixels, unscaled
    let (px, w) = picture_pixels(s("tests/fixtures/pictures/quads.png"), "none", (60.0, 40.0), 60.0, 40.0, 2.0);
    assert_eq!(at(&px, w, 39, 29), QUADS[3]);
    assert_eq!(at(&px, w, 0, 0), QUADS[0]);
    assert!(near(at(&px, w, 45, 10), [255, 255, 255], 0), "a raster at its own size is not scaled up");
    // an SVG's units are logical pixels: 80 × 60 display pixels, sharp
    let (px, w) = picture_pixels(s("tests/fixtures/pictures/quads.svg"), "none", (60.0, 40.0), 60.0, 40.0, 2.0);
    assert!(near(at(&px, w, 70, 50), QUADS[3], 2));
    assert!(near(at(&px, w, 41, 31), QUADS[3], 2));
    assert!(near(at(&px, w, 38, 28), QUADS[0], 2));
    // contained in a smaller box, the picture keeps its proportions
    let (px, w) = picture_pixels(s("tests/fixtures/pictures/quads.png"), "contain", (60.0, 40.0), 20.0, 30.0, 1.0);
    assert!(near(at(&px, w, 2, 9), QUADS[0], 30));
    assert!(near(at(&px, w, 2, 2), [255, 255, 255], 2), "letterboxed above");
}

#[test]
fn a_large_picture_is_kept_at_its_thumbnail_size_and_decoded_off_the_thread_that_asks() {
    use olang::stdlib::gui::picture;
    let mut pm = tiny_skia::Pixmap::new(2400, 1800).expect("pixmap");
    pm.fill(tiny_skia::Color::from_rgba8(37, 99, 235, 255));
    let src = olang::stdlib::bytes::to_value(pm.encode_png().expect("png"));
    // a window (one that is not real: nothing to redraw) asks: not yet
    let first = picture::get(&src, Some((120.0, 90.0)), Some(987_654)).expect("readable");
    assert!(first.is_none(), "a window's picture comes from a decoding thread");
    let mut got = None;
    for _ in 0..400 {
        if let Some(hit) = picture::get(&src, Some((120.0, 90.0)), Some(987_654)).expect("readable") {
            got = Some(hit);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let (pic, info) = got.expect("decoded within four seconds");
    assert_eq!((info.width, info.height), (2400, 1800));
    // 120 across: kept at the power of two above it, not 2400
    assert_eq!((pic.width, pic.height), (128, 96));
    // asked again, the same picture: the cache answers
    let again = picture::get(&src, Some((110.0, 80.0)), None).expect("readable").expect("held");
    assert_eq!(again.0.id, pic.id);
    // its own size is a picture of its own
    let whole = picture::get(&src, None, None).expect("readable").expect("decoded");
    assert_eq!((whole.0.width, whole.0.height), (2400, 1800));
}

#[test]
fn a_picture_that_is_not_one_says_so() {
    use olang::stdlib::gui::picture;
    let junk = olang::stdlib::bytes::to_value(b"not a picture at all".to_vec());
    let e = picture::get(&junk, None, None).expect_err("refused");
    assert!(e.contains("not a PNG, JPEG, WebP, GIF, or SVG"), "{e}");
    let v = run(r##"
        [gui.image_info("tests/fixtures/pictures/quads.jpg"), gui.image_info("tests/fixtures/pictures/quads.svg"),
         gui.image_info("tests/fixtures/pictures/quads.webp"), gui.image_info("tests/fixtures/pictures/quads.gif"),
         is_err(gui.image_info("tests/fixtures/pictures/none.png"))]
    "##);
    let t = text(&v);
    assert!(t.contains(r#""format": "jpeg""#) && t.contains(r#""format": "svg""#), "{t}");
    assert!(t.contains(r#""width": 40"#) && t.contains(r#""height": 30"#), "{t}");
    assert!(t.ends_with("true]"), "{t}");
}

#[test]
fn files_dropped_on_a_window_say_where_they_landed() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (300, 200) })
        let ev = gui.events()
        let mut stale = true
        while stale { match chan.try_recv(ev) { Ok(e) => (), Err(e) => { stale = false } } }
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 200) },
          #{ "key": "left", "parent": "root", "role": "region", "box": (0, 0, 150, 200) },
          #{ "key": "right", "parent": "root", "role": "region", "box": (150, 0, 150, 200) }
        ])
        gui.input(w, #{ "kind": "files", "action": "hover", "paths": ["/tmp/a.png"], "x": 200, "y": 50 })
        gui.input(w, #{ "kind": "files", "action": "drop", "paths": ["/tmp/a.png", "/tmp/b.txt"], "x": 40, "y": 50 })
        gui.input(w, #{ "kind": "files", "action": "cancel" })
        let mut got = []
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { if map_get(e, "kind") == "files" => { got = got + [(map_get(e, "action"), map_get(e, "target"), map_get(e, "paths"))] } }, Err(e) => { more = false } } }
        got
    "##);
    assert_eq!(
        text(&v),
        r#"[("hover", "right", ["/tmp/a.png"]), ("drop", "left", ["/tmp/a.png", "/tmp/b.txt"]), ("cancel", (), [])]"#
    );
}

/// A canvas's operations as the window draws them: rectangles and lines
/// along an axis are the renderers' own rectangles, a clip and a
/// transform are kept on a stack (a transform moves and sizes geometry,
/// not a stroke or a text), and hit regions answer the topmost.
#[test]
fn a_canvas_is_drawn_as_primitives_with_clips_transforms_and_hit_regions() {
    use olang::stdlib::gui::canvas::{self, Item};
    let n = |v: f64| Value::Float(v);
    let ops = Value::List(Arc::new(vec![
        m(vec![("op", s("rect")), ("x", n(0.0)), ("y", n(0.0)), ("w", n(200.0)), ("h", n(100.0)), ("fill", s("#ffffff")), ("hit", s("ground"))]),
        m(vec![("op", s("push")), ("clip", tup(&[20.0, 0.0, 180.0, 100.0])), ("translate", tup(&[20.0, 10.0])), ("scale", tup(&[4.0, 1.0]))]),
        // days 2..5 at 4 px a day, from x 20: 28..48
        m(vec![("op", s("rect")), ("x", n(2.0)), ("y", n(0.0)), ("w", n(5.0)), ("h", n(20.0)), ("radius", n(4.0)), ("fill", s("#2563eb")), ("hit", s("bar"))]),
        // a line down the clip's left edge and one past it, cut
        m(vec![("op", s("line")), ("x1", n(-10.0)), ("y1", n(0.0)), ("x2", n(-10.0)), ("y2", n(80.0)), ("color", s("#dc2626")), ("width", n(2.0))]),
        m(vec![("op", s("text")), ("x", n(2.0)), ("y", n(30.0)), ("text", s("OT-1")), ("size", n(12.0)), ("max_w", n(40.0))]),
        m(vec![("op", s("pop"))]),
        m(vec![("op", s("path")), ("points", Value::List(Arc::new(vec![tup(&[0.0, 90.0]), tup(&[8.0, 94.0]), tup(&[0.0, 98.0])]))), ("close", Value::Boolean(true)), ("fill", s("#16a34a"))]),
        m(vec![("op", s("rect")), ("x", n(100.0)), ("y", n(60.0)), ("w", n(80.0)), ("h", n(20.0)), ("gradient", Value::List(Arc::new(vec![s("#000000"), s("#ffffff")])))]),
    ]));
    let d = canvas::draw(&ops, 200.0, 100.0, 2.0).expect("draws");
    // the ground, the bar, the line, the text, the arrowhead, the gradient
    assert_eq!(d.items.len(), 6, "{:?}", d.items);
    match &d.items[1] {
        Item::Rect { x, y, w, h, radii, clip, .. } => {
            assert_eq!((*x, *y, *w, *h), (28.0, 10.0, 20.0, 20.0));
            assert_eq!(radii[0], 4.0, "a radius is not scaled");
            assert_eq!(*clip, [20.0, 0.0, 200.0, 100.0]);
        }
        other => panic!("the bar is a rectangle: {other:?}"),
    }
    match &d.items[2] {
        Item::Rect { x, w, .. } => assert_eq!((*x, *w), (-21.0, 2.0)),
        other => panic!("an upright line is a rectangle: {other:?}"),
    }
    match &d.items[3] {
        Item::Text(t) => assert_eq!((t.x, t.y, t.size, t.max_w), (28.0, 40.0, 12.0, Some(40.0))),
        other => panic!("text: {other:?}"),
    }
    match &d.items[4] {
        Item::Pic { x, y, pic, .. } => {
            assert!(*x <= 0.0 && *y <= 90.0 && pic.width <= 24 && pic.height <= 24, "a piece the size of the arrowhead");
        }
        other => panic!("a path is a raster piece: {other:?}"),
    }
    // the same shape a whole pixel along is the same picture
    let moved = Value::List(Arc::new(vec![m(vec![
        ("op", s("path")),
        ("points", Value::List(Arc::new(vec![tup(&[30.0, 90.0]), tup(&[38.0, 94.0]), tup(&[30.0, 98.0])]))),
        ("close", Value::Boolean(true)),
        ("fill", s("#16a34a")),
    ])]));
    let d2 = canvas::draw(&moved, 200.0, 100.0, 2.0).expect("draws");
    match (&d.items[4], &d2.items[0]) {
        (Item::Pic { pic: a, x: xa, .. }, Item::Pic { pic: b, x: xb, .. }) => {
            assert_eq!(a.id, b.id, "one raster for the shape");
            assert_eq!(xb - xa, 30.0);
        }
        _ => panic!("pieces"),
    }
    assert!(matches!(&d.items[5], Item::Pic { pic, .. } if pic.width == 256 && pic.height == 1));
    // hits: the bar over the ground; outside the clip the bar is not there
    assert_eq!(text(&canvas::hit(&ops, 200.0, 100.0, 30.0, 20.0)), r#""bar""#);
    assert_eq!(text(&canvas::hit(&ops, 200.0, 100.0, 60.0, 20.0)), r#""ground""#);
    assert_eq!(text(&canvas::hit(&ops, 200.0, 100.0, 250.0, 20.0)), "()");

    // drawn: the gradient dark at its left, light at its right; the bar blue
    let mut st = WinState::new(1, true, "canvas", 200.0, 100.0, 2.0);
    let scene = Value::List(Arc::new(vec![
        m(vec![("key", s("root")), ("box", tup(&[0.0, 0.0, 200.0, 100.0])), ("style", m(vec![("bg", s("#ffffff"))]))]),
        m(vec![("key", s("cv")), ("parent", s("root")), ("role", s("figure")), ("name", s("Timeline")), ("box", tup(&[0.0, 0.0, 200.0, 100.0])), ("draw", ops)]),
    ]));
    let mut out = Vec::new();
    st.apply(&scene, &mut out).expect("applies");
    let dl = st.display_list();
    let px = soft::rgba(&soft::render(&dl));
    let at = |x: f32, y: f32| {
        let i = (((y * 2.0) as u32 * dl.width + (x * 2.0) as u32) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    assert!(at(102.0, 70.0)[0] < 30, "the gradient starts dark: {:?}", at(102.0, 70.0));
    assert!(at(178.0, 70.0)[0] > 225, "and ends light: {:?}", at(178.0, 70.0));
    assert_eq!(at(38.0, 20.0), [37, 99, 235], "the bar");
    assert_eq!(at(19.0, 40.0), [255, 255, 255], "the line is clipped away");
    assert_eq!(at(3.0, 94.0), [22, 163, 74], "the arrowhead");
    if let Ok(g) = gpu::gpu(None, None) {
        let gp = g.render_rgba(&dl);
        let over = px.chunks(4).zip(gp.chunks(4)).filter(|(a, b)| (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap() > 48).count();
        assert!(over * 200 <= px.len() / 4, "{over} pixels differ between the renderers");
    } else {
        eprintln!("skipped the GPU half: no GPU here");
    }
}

/// A canvas that hears the wheel and the pinch (`wheel`) gets them as
/// its own events, with the modifiers held, before a region around it
/// scrolls; a pointer over it says which hit region it is over; and a
/// node's active descendant is said in its place.
#[test]
fn a_canvas_hears_the_wheel_the_pinch_and_says_what_is_under_the_pointer() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (300, 200) })
        let ev = gui.events()
        let mut stale = true
        while stale { match chan.try_recv(ev) { Ok(e) => (), Err(e) => { stale = false } } }
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 200) },
          #{ "key": "sc", "parent": "root", "role": "region", "box": (0, 0, 300, 200), "content": (300, 800) },
          #{ "key": "cv", "parent": "sc", "role": "figure", "name": "Timeline", "box": (0, 0, 300, 200), "wheel": true, "focusable": true, "active": "follow",
             "draw": [#{ "op": "rect", "x": 10, "y": 10, "w": 50, "h": 20, "fill": "#2563eb", "hit": "bar:7" }] },
          #{ "key": "follow", "parent": "root", "role": "button", "name": "OT-7, 3 to 9 Oct", "box": (10, 10, 50, 20), "inert": true }
        ])
        gui.input(w, #{ "kind": "pointer", "action": "move", "x": 20, "y": 20 })
        gui.input(w, #{ "kind": "wheel", "dx": -12, "dy": 30 })
        gui.input(w, #{ "kind": "wheel", "dx": 0, "dy": 10, "mod": true })
        gui.input(w, #{ "kind": "pinch", "delta": 0.25, "phase": "move" })
        let mut got = []
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { got = got + [e] }, Err(e) => { more = false } } }
        let moves = filter(got, (e) => map_get(e, "kind") == "pointer")
        let wheels = filter(got, (e) => map_get(e, "kind") == "wheel")
        let pinches = filter(got, (e) => map_get(e, "kind") == "pinch")
        let scrolled = filter(got, (e) => map_get(e, "kind") == "scrolled")
        let cv = filter(gui.read(w, "a11y"), (n) => map_get(n, "key") == "cv")[0]
        [map_get(moves[0], "hit"), len(wheels), map_get(wheels[0], "key"), map_get(wheels[0], "dx"), map_get(wheels[0], "mod"), map_get(wheels[1], "mod"),
         map_get(pinches[0], "delta"), map_get(pinches[0], "key"), len(scrolled), map_get(cv, "active")]
    "##);
    assert_eq!(text(&v), r#"["bar:7", 2, "cv", -12.0, false, true, 0.25, "cv", 0, "follow"]"#);
}

#[test]
fn a_styled_field_edits_a_paragraph_at_a_time_and_says_what_changed() {
    let _turn = events_turn();
    let v = run(r##"
        let w = gui.headless(#{ "size": (300, 120) })
        let ev = gui.events()
        let mut lines = []
        for i in 0..400 { lines = lines + ["line " + to_string(i)] }
        let text = join(lines, "\n")
        let styles = [#{ "weight": 700 }, #{ "font": "mono", "line_bg": "#eeeeee" }]
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 120) },
          #{ "key": "f", "parent": "root", "role": "textarea", "box": (10, 10, 260, 100),
             "edit": #{ "value": text, "rich": true, "styles": styles, "spans": [[(0, 4, 0)], [(0, 0, 1)]], "select": (0, 0, 1), "undo": false } },
          #{ "op": "focus", "key": "f" }
        ])
        let mut drain = true
        while drain { match chan.try_recv(ev) { Ok(e) => (), _ => { drain = false } } }
        // typed in the first line: one line changed, the caret after it
        gui.input(w, #{ "kind": "text", "text": "X" })
        let mut changed = ()
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(e) => { if map_get(e, "kind") == "changed" => { changed = e } }, _ => { more = false } } }
        let d = map_get(changed, "delta")
        let caret = map_get(changed, "caret")
        // the end of the text, by key: the field scrolls to its caret
        gui.input(w, #{ "kind": "key", "key": "down", "mod": true })
        let shown = gui.read(w, "caret")
        // ⌘Z is the program's here (undo: false): the value stays
        gui.input(w, #{ "kind": "key", "key": "z", "mod": true })
        // the program selects: the field takes it, and says so
        gui.apply(w, [#{ "key": "f", "parent": "root", "role": "textarea", "box": (10, 10, 260, 100),
             "edit": #{ "value": "X" + text, "rich": true, "styles": styles, "spans": [], "select": (1, 3, 2), "undo": false } }])
        let mut sel = ()
        let mut more2 = true
        while more2 { match chan.try_recv(ev) { Ok(e) => { if map_get(e, "kind") == "select" => { sel = map_get(e, "selection") } }, _ => { more2 = false } } }
        [map_get(d, "first"), map_get(d, "removed"), map_get(d, "lines"), map_get(d, "at"), map_get(d, "inserted"), map_get(changed, "selection"),
         caret[0] > 10.0 && caret[1] >= 8.0, shown != () && shown[1] > 10.0 && shown[1] < 110.0, gui.read(w, "value", "f") == "X" + text, sel]
    "##);
    assert_eq!(
        text(&v),
        r#"[0, 1, ["Xline 0"], 0, "X", (1, 1), true, true, true, (1, 3)]"#
    );
}

#[test]
fn a_styled_field_composes_in_place_and_is_read_as_runs() {
    let _turn = events_turn();
    // the text's end: ⌘↓ on macOS, ctrl+end on Windows and Linux
    let to_end = if cfg!(target_os = "macos") {
        "down"
    } else {
        "end"
    };
    let v = run(&r##"
        let w = gui.headless(#{ "size": (300, 120) })
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 120) },
          #{ "key": "f", "parent": "root", "role": "textarea", "box": (10, 10, 260, 100),
             "edit": #{ "value": "ab\ncd", "rich": true, "styles": [], "spans": [] } },
          #{ "op": "focus", "key": "f" }
        ])
        gui.input(w, #{ "kind": "key", "key": "TO_END", "mod": true })
        gui.input(w, #{ "kind": "compose", "text": "にほ" })
        // composing: the value is not yet changed, the input method's
        // window is at the composition
        let during = gui.read(w, "value", "f")
        let ime = gui.read(w, "ime")
        gui.input(w, #{ "kind": "commit", "text": "日本" })
        let node = filter(gui.read(w, "a11y"), (n) => map_get(n, "key") == "f")[0]
        [during, ime != () && ime[0] > 10.0 && ime[1] > 20.0, gui.read(w, "value", "f"), map_get(node, "text_runs"), map_get(node, "text_selection")]
    "##
    .replace("TO_END", to_end));
    assert_eq!(
        text(&v),
        "[\"ab\ncd\", true, \"ab\ncd日本\", 2, (1, 4, 1, 4)]"
    );
}

#[test]
fn a_text_with_a_bold_run_is_wider_and_measured_so() {
    let v = run(r##"
        let plain = gui.measure("make it bold", #{ "size": 14 })
        let bold = gui.measure("make it bold", #{ "size": 14, "spans": [(8, 12, #{ "weight": 800 })] })
        map_get(bold, "width") > map_get(plain, "width")
    "##);
    assert_eq!(text(&v), "true");
}

/// A platform window that has not learnt where it is on the screen (all
/// of them on Wayland) sends pointer events without `sx`/`sy`, so a drag
/// stays in its own window instead of landing in another by a guess; once
/// placed it sends them.
#[test]
fn a_window_that_cannot_learn_its_place_sends_no_screen_point() {
    let mut st = WinState::new(7, false, "real", 300.0, 200.0, 1.0);
    let mut out = Vec::new();
    st.apply(
        &Value::List(Arc::new(vec![m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 300.0, 200.0])),
        ])])),
        &mut out,
    )
    .expect("applies");
    let mut clip = NoClip;
    let mut press = |st: &mut WinState| {
        let mut out = Vec::new();
        st.input(
            Input::Pointer {
                action: PointerAction::Move,
                x: 20.0,
                y: 30.0,
                button: "left".into(),
                clicks: 0,
            },
            &mut clip,
            &mut out,
        );
        let ev = out
            .into_iter()
            .find(|e| text(e).contains("\"pointer\""))
            .expect("a pointer event");
        let Value::Map(f) = &ev else { panic!("a map") };
        (
            f.get("sx").cloned().unwrap_or(Value::Unit),
            f.get("sy").cloned().unwrap_or(Value::Unit),
        )
    };
    let (sx, sy) = press(&mut st);
    assert_eq!((text(&sx), text(&sy)), ("()".to_string(), "()".to_string()));
    let mut moved = Vec::new();
    st.place(100.0, 50.0, &mut moved);
    assert_eq!(moved.len(), 1, "a `moved` event");
    let (sx, sy) = press(&mut st);
    assert_eq!(
        (text(&sx), text(&sy)),
        ("120.0".to_string(), "80.0".to_string())
    );
}

/// `gui.platform()` says what this OS's windows can do; the degraded
/// ones are named, not discovered.
#[test]
fn the_platform_says_what_its_windows_can_do() {
    let v = run(r#"
        let p = gui.platform()
        [map_get(p, "os"), map_get(p, "native_menu"), map_get(p, "clipboard_image"), map_get(p, "file_drop"),
         map_get(p, "drop_position"), map_get(p, "window_position"), map_get(p, "system_settings"),
         map_get(p, "ime"), map_get(p, "accessibility")]
    "#);
    let want = if cfg!(target_os = "macos") {
        r#"["macos", true, true, true, true, true, true, true, true]"#
    } else if cfg!(target_os = "windows") {
        r#"["windows", false, false, true, false, true, false, true, true]"#
    } else {
        r#"["linux", false, false, false, false, false, false, true, true]"#
    };
    assert_eq!(text(&v), want);
    // and what is missing says so when asked for
    if !cfg!(target_os = "macos") {
        let v = run(r#"match gui.clipboard_image() { Ok(x) => "ok", Err(e) => e }"#);
        assert!(text(&v).contains("macOS only"), "{}", text(&v));
        let v = run(r#"gui.menu([])"#);
        assert_eq!(text(&v), "false");
    }
}

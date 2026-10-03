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
    let mut h = std::collections::HashMap::new();
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
    let rev_of_a = st.editors["field"].rev - 1;
    // The program answers the first keystroke after the second landed.
    st.apply(&field_patch("a", Some(rev_of_a)), &mut out)
        .unwrap();
    assert_eq!(st.editors["field"].value(), "ab");
    // A program that transforms the value is obeyed.
    let rev = st.editors["field"].rev;
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

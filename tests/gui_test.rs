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

#[test]
fn content_lets_a_list_scroll_past_its_laid_out_rows() {
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
        map_get(n, "offset")
    "##);
    assert_eq!(text(&v), "(0.0, 31900.0)");
}

#[test]
fn a_modal_layer_keeps_the_focus_and_hears_a_press_outside() {
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

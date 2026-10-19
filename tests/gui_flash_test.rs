//! The repaint rainbow (`gui.set(w, #{ "flash": true })`, Loom's
//! inspector): every region a frame draws is tinted with that frame's
//! colour, over its pixels, and damage never sees the tint — a region
//! keeps the colour of the frame that last drew it, an unchanged frame
//! draws nothing, and turning it off draws the window whole again.
//! Software and GPU (where there is an adapter) tint alike.

#![cfg(feature = "gui")]

use olang::Value;
use olang::stdlib::gui::damage::{self, Damage};
use olang::stdlib::gui::window::WinState;
use olang::stdlib::gui::{gpu, soft};
use std::sync::Arc;

fn s(t: &str) -> Value {
    Value::String(Arc::new(t.to_string()))
}

fn m(fields: Vec<(&str, Value)>) -> Value {
    let mut h = olang::ast::ValueMap::default();
    for (k, v) in fields {
        h.insert(k.to_string(), v);
    }
    Value::Map(Arc::new(h))
}

fn tup(v: &[f64]) -> Value {
    Value::Tuple(Arc::new(v.iter().map(|x| Value::Float(*x)).collect()))
}

fn scene(card: &str) -> Value {
    Value::List(Arc::new(vec![
        m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 400.0, 300.0])),
            ("style", m(vec![("bg", s("#ffffff"))])),
        ]),
        m(vec![
            ("key", s("card")),
            ("parent", s("root")),
            ("box", tup(&[300.0, 200.0, 60.0, 60.0])),
            ("style", m(vec![("bg", s(card))])),
        ]),
    ]))
}

/// The pixel at (x, y) of straight RGBA.
fn at(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

#[test]
fn each_frame_tints_what_it_drew_and_damage_never_sees_the_tint() {
    let mut st = WinState::new(1, true, "flash", 400.0, 300.0, 1.0);
    let mut out = Vec::new();
    st.apply(&scene("#000000"), &mut out).unwrap();
    let mut kept = soft::Retained::default();
    let g = gpu::gpu(None, None).ok();
    let mut target = g
        .as_ref()
        .map(|g| gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm));
    // the rainbow on: the first frame is drawn whole, all of it frame 0's colour
    st.flash = Some(0);
    let dl = st.display_list();
    assert_eq!(dl.flash, Some(0));
    assert_eq!(kept.render(&dl).clone(), Damage::Full);
    let soft0 = soft::rgba(kept.pixmap().unwrap());
    let white = at(&soft0, 400, 10, 10);
    assert!(
        white[1] < 255 && white[0] == 255,
        "white tinted with frame 0's red: {white:?}"
    );
    if let Some(t) = target.as_mut() {
        t.render(&dl);
        let px = at(&t.rgba(), 400, 10, 10);
        assert!(px[1] < 255 && px[0] == 255, "the GPU tints too: {px:?}");
    }
    // the card changes: only its tiles are drawn, in frame 1's colour; the
    // rest keeps frame 0's
    st.apply(&scene("#ffffff"), &mut out).unwrap();
    let dl = st.display_list();
    assert_eq!(dl.flash, Some(1));
    let d = kept.render(&dl).clone();
    assert!(matches!(d, Damage::Rects(_)), "a part drawn, not {d:?}");
    let soft1 = soft::rgba(kept.pixmap().unwrap());
    assert_eq!(
        at(&soft1, 400, 10, 10),
        white,
        "a region not drawn keeps its tint"
    );
    // frame 1's orange over white, where frame 0's red was
    let card = at(&soft1, 400, 330, 230);
    assert!(
        card[0] == 255 && card[1] > white[1] && card[2] < white[2],
        "the card's region in frame 1's colour: {card:?} (frame 0's: {white:?})"
    );
    assert_eq!(damage::flash_color(1)[..3], [255, 149, 0]);
    if let Some(t) = target.as_mut() {
        let gd = t.render(&dl);
        assert_eq!(gd, d, "the GPU's damage is the software's");
        let px = t.rgba();
        for (x, y, want) in [(10, 10, white), (330, 230, card)] {
            let got = at(&px, 400, x, y);
            for k in 0..3 {
                assert!(
                    (got[k] as i32 - want[k] as i32).abs() <= 3,
                    "at ({x}, {y}) GPU {got:?} and software {want:?} tint alike"
                );
            }
        }
    }
    // nothing changed: nothing drawn, the tints stay
    let dl = st.display_list();
    assert_eq!(kept.render(&dl).clone(), Damage::None);
    assert_eq!(soft::rgba(kept.pixmap().unwrap()), soft1);
    // off: the window drawn whole again, as a frame without the rainbow
    st.flash = None;
    st.redraw = true;
    let dl = st.display_list();
    assert_eq!((dl.flash, dl.redraw), (None, true));
    assert_eq!(kept.render(&dl).clone(), Damage::Full);
    assert_eq!(
        soft::rgba(kept.pixmap().unwrap()),
        soft::rgba(&soft::render(&dl))
    );
    if let Some(t) = target.as_mut() {
        assert_eq!(t.render(&dl), Damage::Full);
    }
}

#[test]
fn gui_set_turns_it_on_and_off() {
    // a headless window's reads are the program's picture, untinted; the
    // engine says it has the rainbow and takes the option
    let program = olang::Parser::new()
        .parse(
            r##"
        let w = gui.headless(#{ "size": (100, 80) })
        gui.apply(w, [#{ "key": "root", "box": (0, 0, 100, 80), "style": #{ "bg": "#ffffff" } }])
        let plain = gui.read(w, "rgba")
        gui.set(w, #{ "flash": true })
        let read = gui.read(w, "rgba")
        gui.set(w, #{ "flash": false })
        [gui.platform().flash, plain == read]
    "##,
        )
        .expect("parses");
    let v = olang::Interpreter::new()
        .eval_program(program)
        .expect("runs");
    assert_eq!(format!("{v}"), "[true, true]");
}

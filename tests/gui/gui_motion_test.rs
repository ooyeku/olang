//! Motion (loom/ROADMAP.md item 17, SPEC §7): a style's `transition`
//! animates a node's opacity, box and colours between frames, brings a
//! node in from its `enter` pose and draws one leaving to its `exit` — on
//! the window's clock, with no program turn; instant under reduced motion
//! and in a headless window that has not asked for motion. A moving node
//! damages only its own tiles, and a damaged frame is the whole frame.

#![cfg(feature = "gui")]

use olang::Value;
use olang::stdlib::gui::damage::Damage;
use olang::stdlib::gui::window::{Clipboard, Input, PointerAction, WinState};
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

fn int(i: i64) -> Value {
    Value::Integer(i)
}

struct NoClip;
impl Clipboard for NoClip {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _: String) {}
}

fn linear(ms: i64, extra: Vec<(&str, Value)>) -> Value {
    let mut f = vec![("curve", s("linear")), ("duration", int(ms))];
    f.extend(extra);
    m(f)
}

/// A 200×100 window at 1× with a white root, motion on.
fn window() -> WinState {
    let mut st = WinState::new(1, true, "motion", 200.0, 100.0, 1.0);
    st.motion.enabled = true;
    apply(
        &mut st,
        vec![m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 200.0, 100.0])),
            ("style", m(vec![("bg", s("#ffffff"))])),
        ])],
    );
    st.set_clock(0.0);
    st.display_list();
    st
}

fn apply(st: &mut WinState, ops: Vec<Value>) {
    let mut out = Vec::new();
    st.apply(&Value::List(Arc::new(ops)), &mut out)
        .expect("applies");
}

fn boxed(key: &str, x: f64, bg: &str, tr: Value, extra: Vec<(&str, Value)>) -> Value {
    let mut style = vec![("bg", s(bg)), ("transition", tr)];
    style.extend(extra);
    m(vec![
        ("key", s(key)),
        ("parent", s("root")),
        ("box", tup(&[x, 20.0, 40.0, 40.0])),
        ("style", m(style)),
    ])
}

/// The pixel at `(x, y)` of the frame drawn at `ms`.
fn pixel_at(st: &mut WinState, ms: f64, x: u32, y: u32) -> [u8; 4] {
    st.set_clock(ms);
    let dl = st.display_list();
    let pm = soft::render(&dl);
    let i = ((y * dl.width + x) * 4) as usize;
    let d = &soft::rgba(&pm)[i..i + 4];
    [d[0], d[1], d[2], d[3]]
}

fn near(a: u8, b: u8) -> bool {
    (a as i32 - b as i32).abs() <= 3
}

#[test]
fn an_opacity_change_fades_on_the_clock_and_asks_for_the_next_frame_until_it_lands() {
    let mut st = window();
    apply(
        &mut st,
        vec![boxed("a", 20.0, "#000000", linear(100, vec![]), vec![])],
    );
    assert_eq!(
        pixel_at(&mut st, 0.0, 40, 40)[0],
        0,
        "drawn as it is the first time"
    );
    assert_eq!(st.next_frame, None, "nothing moves");
    apply(
        &mut st,
        vec![boxed(
            "a",
            20.0,
            "#000000",
            linear(100, vec![]),
            vec![("opacity", int(0))],
        )],
    );
    assert_eq!(
        pixel_at(&mut st, 0.0, 40, 40)[0],
        0,
        "it starts where it was"
    );
    assert!(st.next_frame.is_some(), "a frame asked for at once");
    let mid = pixel_at(&mut st, 50.0, 40, 40)[0];
    assert!(near(mid, 128), "half way: {mid}");
    assert_eq!(pixel_at(&mut st, 100.0, 40, 40)[0], 255, "landed: gone");
    // landed: no more frames
    st.display_list();
    assert_eq!(st.next_frame, None);
}

#[test]
fn a_box_moves_and_a_colour_fades_between_frames() {
    let mut st = window();
    apply(
        &mut st,
        vec![boxed("a", 20.0, "#000000", linear(100, vec![]), vec![])],
    );
    pixel_at(&mut st, 0.0, 0, 0);
    apply(
        &mut st,
        vec![boxed("a", 120.0, "#ff0000", linear(100, vec![]), vec![])],
    );
    pixel_at(&mut st, 0.0, 0, 0);
    // at 50 ms the box is at x 70 (70 … 110), half way to red
    let p = pixel_at(&mut st, 50.0, 90, 40);
    assert!(near(p[0], 128) && p[1] == 0, "{p:?}");
    assert_eq!(
        pixel_at(&mut st, 50.0, 30, 40),
        [255, 255, 255, 255],
        "left where it was"
    );
    let p = pixel_at(&mut st, 100.0, 140, 40);
    assert_eq!(p, [255, 0, 0, 255]);
    // only what it animates moves: a box-only transition changes colour at once
    let tr = linear(
        100,
        vec![("animate", Value::List(Arc::new(vec![s("box")])))],
    );
    apply(&mut st, vec![boxed("a", 120.0, "#0000ff", tr, vec![])]);
    assert_eq!(pixel_at(&mut st, 101.0, 140, 40), [0, 0, 255, 255]);
}

#[test]
fn a_node_added_enters_from_its_pose_and_one_removed_leaves_to_its_exit_under_nothing() {
    let mut st = window();
    let tr = linear(
        100,
        vec![
            ("enter", m(vec![("opacity", int(0)), ("y", int(-10))])),
            (
                "exit",
                m(vec![("opacity", int(0)), ("scale", Value::Float(0.5))]),
            ),
        ],
    );
    apply(
        &mut st,
        vec![boxed("pop", 20.0, "#000000", tr.clone(), vec![])],
    );
    assert_eq!(
        pixel_at(&mut st, 0.0, 40, 40)[0],
        255,
        "it comes in from nothing"
    );
    let mid = pixel_at(&mut st, 50.0, 40, 40)[0];
    assert!(near(mid, 128), "half in: {mid}");
    // half of the 10 px move left: its top at 15
    assert_eq!(pixel_at(&mut st, 50.0, 40, 16)[0], mid);
    assert_eq!(pixel_at(&mut st, 100.0, 40, 40)[0], 0);
    assert_eq!(
        pixel_at(&mut st, 100.0, 40, 18)[0],
        255,
        "at rest: its top at 20"
    );
    // removed: drawn leaving, found by nothing
    apply(
        &mut st,
        vec![m(vec![("op", s("remove")), ("key", s("pop"))])],
    );
    assert!(st.scene.hit(40.0, 40.0).as_deref() != Some("pop"));
    let at = pixel_at(&mut st, 150.0, 40, 40)[0];
    assert!(near(at, 128), "half gone: {at}");
    // scaled to 0.75 about its centre (40, 40): its edge at 25, not 20
    assert_eq!(pixel_at(&mut st, 150.0, 22, 40)[0], 255);
    assert!(pixel_at(&mut st, 150.0, 27, 40)[0] < 200);
    assert_eq!(pixel_at(&mut st, 200.0, 40, 40)[0], 255, "gone");
    st.display_list();
    assert!(!st.motion.moving() && st.next_frame.is_none());
    // the first frame of a window enters nothing
    let mut st2 = WinState::new(2, true, "motion", 200.0, 100.0, 1.0);
    st2.motion.enabled = true;
    apply(
        &mut st2,
        vec![
            m(vec![
                ("key", s("root")),
                ("box", tup(&[0.0, 0.0, 200.0, 100.0])),
                ("style", m(vec![("bg", s("#ffffff"))])),
            ]),
            boxed("pop", 20.0, "#000000", tr, vec![]),
        ],
    );
    assert_eq!(pixel_at(&mut st2, 0.0, 40, 40)[0], 0);
}

#[test]
fn a_node_back_while_it_leaves_is_itself_again() {
    let mut st = window();
    let tr = linear(100, vec![("exit", m(vec![("opacity", int(0))]))]);
    apply(
        &mut st,
        vec![boxed("pop", 20.0, "#000000", tr.clone(), vec![])],
    );
    pixel_at(&mut st, 0.0, 0, 0);
    apply(
        &mut st,
        vec![m(vec![("op", s("remove")), ("key", s("pop"))])],
    );
    pixel_at(&mut st, 10.0, 0, 0);
    assert_eq!(st.motion.ghosts.len(), 1);
    apply(&mut st, vec![boxed("pop", 20.0, "#000000", tr, vec![])]);
    assert!(
        st.motion.ghosts.is_empty(),
        "the ghost goes as the node comes back"
    );
    assert_eq!(pixel_at(&mut st, 20.0, 40, 40)[0], 0);
}

#[test]
fn reduced_motion_and_a_still_headless_window_make_every_transition_instant() {
    for reduced in [true, false] {
        let mut st = window();
        if reduced {
            let mut set = st.settings;
            set.reduce_motion = true;
            st.set_settings(set);
        } else {
            st.motion.enabled = false;
        }
        let tr = linear(
            100,
            vec![
                ("enter", m(vec![("opacity", int(0))])),
                ("exit", m(vec![("opacity", int(0))])),
            ],
        );
        apply(
            &mut st,
            vec![boxed("a", 20.0, "#000000", tr.clone(), vec![])],
        );
        assert_eq!(pixel_at(&mut st, 0.0, 40, 40)[0], 0, "no entrance");
        apply(&mut st, vec![boxed("a", 120.0, "#000000", tr, vec![])]);
        assert_eq!(pixel_at(&mut st, 1.0, 140, 40)[0], 0, "no move");
        apply(&mut st, vec![m(vec![("op", s("remove")), ("key", s("a"))])]);
        assert_eq!(pixel_at(&mut st, 2.0, 140, 40)[0], 255, "no exit");
        assert_eq!(st.next_frame, None);
    }
    // a headless window is still until a test asks
    let st = WinState::new(3, true, "m", 10.0, 10.0, 1.0);
    assert!(!st.motion.enabled);
}

#[test]
fn a_hover_look_fades_with_no_program_turn() {
    let mut st = window();
    let tr = linear(
        100,
        vec![("animate", Value::List(Arc::new(vec![s("color")])))],
    );
    apply(
        &mut st,
        vec![m(vec![
            ("key", s("b")),
            ("parent", s("root")),
            ("role", s("button")),
            ("box", tup(&[20.0, 20.0, 40.0, 40.0])),
            (
                "style",
                m(vec![
                    ("bg", s("#000000")),
                    ("transition", tr),
                    ("hover", m(vec![("bg", s("#ffffff"))])),
                ]),
            ),
        ])],
    );
    assert_eq!(pixel_at(&mut st, 0.0, 40, 40)[0], 0);
    let mut out = Vec::new();
    st.input(
        Input::Pointer {
            action: PointerAction::Move,
            x: 40.0,
            y: 40.0,
            button: "left".into(),
            clicks: 0,
        },
        &mut NoClip,
        &mut out,
    );
    pixel_at(&mut st, 0.0, 40, 40);
    let mid = pixel_at(&mut st, 50.0, 40, 40)[0];
    assert!(near(mid, 128), "{mid}");
    assert_eq!(pixel_at(&mut st, 100.0, 40, 40)[0], 255);
}

#[test]
fn a_spring_retargeted_mid_flight_never_jumps() {
    let mut st = window();
    let tr = m(vec![("curve", s("spring")), ("duration", int(300))]);
    apply(
        &mut st,
        vec![boxed("a", 0.0, "#000000", tr.clone(), vec![])],
    );
    pixel_at(&mut st, 0.0, 0, 0);
    apply(
        &mut st,
        vec![boxed("a", 150.0, "#000000", tr.clone(), vec![])],
    );
    let edge = |st: &mut WinState, ms: f64| -> u32 {
        st.set_clock(ms);
        let dl = st.display_list();
        let pm = soft::render(&dl);
        let px = soft::rgba(&pm);
        (0..200)
            .find(|x| px[((40 * dl.width + x) * 4) as usize] < 128)
            .unwrap_or(200)
    };
    let mut last = edge(&mut st, 0.0);
    for ms in (8..120).step_by(8) {
        let e = edge(&mut st, ms as f64);
        assert!(e >= last, "moving right: {last} then {e}");
        last = e;
    }
    apply(&mut st, vec![boxed("a", 0.0, "#000000", tr, vec![])]);
    for ms in (120..900).step_by(8) {
        let e = edge(&mut st, ms as f64);
        assert!(
            (e as i64 - last as i64).abs() <= 12,
            "a step of {} px at {ms} ms",
            e as i64 - last as i64
        );
        last = e;
    }
    assert_eq!(last, 0, "settled back");
}

/// A moving node damages its own tiles, nothing else; the damaged frame
/// is the whole frame, in software and on the GPU (which agrees with
/// software).
#[test]
fn a_moving_node_damages_only_its_tiles_and_damaged_frames_are_whole_frames() {
    let mut st = WinState::new(4, true, "motion", 800.0, 600.0, 2.0);
    st.motion.enabled = true;
    let mut ops = vec![m(vec![
        ("key", s("root")),
        ("box", tup(&[0.0, 0.0, 800.0, 600.0])),
        ("style", m(vec![("bg", s("#f4f4f5"))])),
    ])];
    for i in 0..40 {
        ops.push(m(vec![
            ("key", s(&format!("t{i}"))),
            ("parent", s("root")),
            ("role", s("text")),
            ("box", tup(&[400.0, i as f64 * 14.0, 380.0, 14.0])),
            ("text", s(&format!("line {i} stays where it is"))),
        ]));
    }
    let tr = linear(
        160,
        vec![(
            "enter",
            m(vec![("opacity", int(0)), ("scale", Value::Float(0.9))]),
        )],
    );
    ops.push(m(vec![
        ("key", s("pop")),
        ("parent", s("root")),
        ("box", tup(&[40.0, 40.0, 120.0, 60.0])),
        (
            "style",
            m(vec![
                ("bg", s("#2563eb")),
                ("radius", int(8)),
                ("transition", tr.clone()),
            ]),
        ),
    ]));
    apply(&mut st, ops);
    st.set_clock(0.0);
    let mut kept = soft::Retained::default();
    let g = gpu::gpu(None, None).ok();
    let mut target = g
        .as_ref()
        .map(|g| gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm));
    let dl = st.display_list();
    kept.render(&dl);
    if let Some(t) = target.as_mut() {
        t.render(&dl);
    }
    // a child with text comes in, and the box fades and moves
    apply(
        &mut st,
        vec![
            m(vec![
                ("key", s("pop")),
                ("parent", s("root")),
                ("box", tup(&[60.0, 50.0, 120.0, 60.0])),
                (
                    "style",
                    m(vec![
                        ("bg", s("#2563eb")),
                        ("radius", int(8)),
                        ("opacity", Value::Float(0.6)),
                        ("transition", tr.clone()),
                    ]),
                ),
            ]),
            m(vec![
                ("key", s("label")),
                ("parent", s("pop")),
                ("role", s("text")),
                ("box", tup(&[8.0, 8.0, 100.0, 20.0])),
                ("text", s("Hello")),
                (
                    "style",
                    m(vec![("color", s("#ffffff")), ("transition", tr)]),
                ),
            ]),
        ],
    );
    let mut frames = 0;
    for ms in (0..=200).step_by(16) {
        st.set_clock(ms as f64);
        let dl = st.display_list();
        let whole = soft::render(&dl);
        let d = kept.render(&dl).clone();
        assert_eq!(
            kept.pixmap().unwrap().data(),
            whole.data(),
            "at {ms} ms: the damaged frame is the whole frame"
        );
        if let Damage::Rects(rects) = &d {
            let area = d.area(dl.width, dl.height);
            // the box's tiles, never the text beside it (from 800 device px)
            assert!(
                area * 6 < (dl.width * dl.height) as i64,
                "at {ms} ms the frame drew {area} px"
            );
            assert!(
                rects.iter().all(|r| r.x1 <= 800),
                "at {ms} ms the text was drawn again: {rects:?}"
            );
            frames += 1;
        }
        assert!(!matches!(d, Damage::Full), "at {ms} ms: a whole frame");
        if let (Some(g), Some(t)) = (&g, target.as_mut()) {
            t.render(&dl);
            let whole_gpu = g.render_rgba(&dl);
            assert_eq!(
                t.rgba(),
                whole_gpu,
                "at {ms} ms the GPU's damaged frame is its whole frame"
            );
            let sw = soft::rgba(&whole);
            let worst = sw
                .iter()
                .zip(&whole_gpu)
                .map(|(a, b)| (*a as i32 - *b as i32).abs())
                .max()
                .unwrap_or(0);
            assert!(
                worst <= 40,
                "at {ms} ms the GPU and software differ by {worst}"
            );
        }
    }
    assert!(frames >= 8, "it moved over {frames} frames");
    // landed: the same frame draws nothing
    st.set_clock(400.0);
    let dl = st.display_list();
    kept.render(&dl);
    let dl = st.display_list();
    assert_eq!(*kept.render(&dl), Damage::None);
    assert_eq!(st.next_frame, None);
}

#[test]
fn a_hover_only_transition_fades_the_hover_and_lands_a_selection_at_once() {
    let mut st = window();
    let tr = linear(
        100,
        vec![("animate", Value::List(Arc::new(vec![s("hover")])))],
    );
    let node = |bg: &str, tr: Value| {
        m(vec![
            ("key", s("row")),
            ("parent", s("root")),
            ("role", s("button")),
            ("box", tup(&[20.0, 20.0, 40.0, 40.0])),
            (
                "style",
                m(vec![
                    ("bg", s(bg)),
                    ("transition", tr),
                    ("hover", m(vec![("bg", s("#808080"))])),
                ]),
            ),
        ])
    };
    apply(&mut st, vec![node("#000000", tr.clone())]);
    assert_eq!(pixel_at(&mut st, 0.0, 40, 40)[0], 0);
    // selected (the program's look changes): at once
    apply(&mut st, vec![node("#ffffff", tr.clone())]);
    assert_eq!(pixel_at(&mut st, 1.0, 40, 40)[0], 255);
    // the pointer comes: the hover look fades in
    let mut out = Vec::new();
    st.input(
        Input::Pointer {
            action: PointerAction::Move,
            x: 40.0,
            y: 40.0,
            button: "left".into(),
            clicks: 0,
        },
        &mut NoClip,
        &mut out,
    );
    pixel_at(&mut st, 10.0, 40, 40);
    let mid = pixel_at(&mut st, 60.0, 40, 40)[0];
    assert!(mid > 140 && mid < 245, "{mid}");
    assert_eq!(pixel_at(&mut st, 110.0, 40, 40)[0], 128);
}

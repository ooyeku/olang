//! The engine's geometry (loom/ROADMAP.md items 11–13): boxes on whole
//! device pixels at every scale, continuous corners, analytic shadows —
//! each drawn alike by the software reference and the GPU.
//!
//! The GPU halves need an adapter and skip (saying so) where there is none.

#![cfg(feature = "gui")]

use olang::Value;
use olang::stdlib::gui::window::WinState;
use olang::stdlib::gui::{gpu, soft};
use std::sync::Arc;

/// The GPU's glyph atlas is the process's: tests drawing on it take turns.
static GPU_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

fn f(v: f64) -> Value {
    Value::Float(v)
}

fn tup(v: &[f64]) -> Value {
    Value::Tuple(Arc::new(v.iter().map(|x| Value::Float(*x)).collect()))
}

fn window(w: f64, h: f64, scale: f32, nodes: Vec<Value>) -> WinState {
    let mut st = WinState::new(1, true, "geometry", w as f32, h as f32, scale);
    let mut ops = vec![m(vec![
        ("key", s("root")),
        ("box", tup(&[0.0, 0.0, w, h])),
        ("style", m(vec![("bg", s("#ffffff"))])),
    ])];
    ops.extend(nodes);
    let mut out = Vec::new();
    st.apply(&Value::List(Arc::new(ops)), &mut out)
        .expect("the scene applies");
    st
}

fn boxed(key: &str, b: [f64; 4], style: Vec<(&str, Value)>) -> Value {
    m(vec![
        ("key", s(key)),
        ("parent", s("root")),
        ("box", tup(&b)),
        ("style", m(style)),
    ])
}

fn soft_px(st: &mut WinState) -> (Vec<u8>, u32) {
    let dl = st.display_list();
    let w = dl.width;
    (soft::rgba(&soft::render(&dl)), w)
}

fn at(px: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

/// The two renderers within their antialiasing tolerance (gui_test.rs's:
/// no more than one pixel in a thousand off by more than 48, and less than
/// one level off on average — a frame of paths is mostly edges).
fn agree(st: &mut WinState, what: &str) {
    let _turn = GPU_TURN.lock().unwrap_or_else(|e| e.into_inner());
    let g = match gpu::gpu(None, None) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped the GPU half: no GPU here ({e})");
            return;
        }
    };
    let dl = st.display_list();
    let a = soft::rgba(&soft::render(&dl));
    let b = g.render_rgba(&dl);
    assert_eq!(a.len(), b.len());
    let (mut over, mut worst, mut total) = (0usize, 0u8, 0u64);
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        let d = (0..4).map(|i| p[i].abs_diff(q[i])).max().unwrap_or(0);
        worst = worst.max(d);
        total += d as u64;
        if d > 48 {
            over += 1;
        }
    }
    let n = a.len() / 4;
    let mean = total as f64 / n as f64;
    if let Some(dir) = std::env::var_os("GUI_GEOMETRY_DUMP") {
        // the two frames and where they differ, for a look
        let save = |px: &[u8], name: &str| {
            let mut pm = tiny_skia::Pixmap::new(dl.width, dl.height).expect("a pixmap");
            for (i, c) in px.chunks(4).enumerate() {
                let col = tiny_skia::ColorU8::from_rgba(c[0], c[1], c[2], c[3]).premultiply();
                pm.pixels_mut()[i] = col;
            }
            let tag: String = what
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect();
            let _ = pm.save_png(std::path::Path::new(&dir).join(format!("{tag}-{name}.png")));
        };
        save(&a, "soft");
        save(&b, "gpu");
        let diff: Vec<u8> = a
            .chunks(4)
            .zip(b.chunks(4))
            .flat_map(|(p, q)| {
                let d = (0..4).map(|i| p[i].abs_diff(q[i])).max().unwrap_or(0);
                [d.saturating_mul(4), 0, 0, 255]
            })
            .collect();
        save(&diff, "diff");
    }
    assert!(
        over * 1000 <= n && mean < 1.0,
        "{what}: {over} of {n} pixels differ by more than 48 (worst {worst}, mean {mean:.3})"
    );
}

/// Continuous corners and shadows, at whole and fractional scales.
fn geometry_scene(scale: f32) -> WinState {
    window(
        360.0,
        260.0,
        scale,
        vec![
            boxed(
                "sheet",
                [24.0, 20.0, 180.0, 120.0],
                vec![
                    ("bg", s("#ffffff")),
                    ("border", s("#d4d4d8")),
                    ("border_width", f(1.0)),
                    ("radius", f(12.0)),
                    ("corner", s("continuous")),
                    ("clip", Value::Boolean(true)),
                    (
                        "shadow",
                        m(vec![
                            ("color", s("#00000040")),
                            ("blur", f(24.0)),
                            ("y", f(8.0)),
                        ]),
                    ),
                ],
            ),
            m(vec![
                ("key", s("inside")),
                ("parent", s("sheet")),
                ("box", tup(&[-10.0, -10.0, 90.0, 60.0])),
                ("style", m(vec![("bg", s("#2563eb"))])),
            ]),
            boxed(
                "button",
                [230.0, 24.0, 96.0, 28.0],
                vec![
                    ("bg", s("#2563eb")),
                    ("radius", f(6.0)),
                    ("corner", s("continuous")),
                    (
                        "shadow",
                        m(vec![
                            ("color", s("#0000004d")),
                            ("blur", f(2.0)),
                            ("y", f(1.0)),
                        ]),
                    ),
                ],
            ),
            boxed(
                "uneven",
                [230.0, 70.0, 100.0, 70.0],
                vec![
                    ("bg", s("#10b98180")),
                    ("border", s("#065f46")),
                    ("border_width", f(3.0)),
                    ("radius", tup(&[0.0, 24.0, 4.0, 12.0])),
                    ("corner", s("continuous")),
                ],
            ),
            boxed(
                "pill",
                [24.0, 170.0, 120.0, 24.0],
                vec![
                    ("bg", s("#18181b")),
                    ("radius", f(12.0)),
                    ("corner", s("continuous")),
                ],
            ),
            boxed(
                "spread",
                [180.0, 170.0, 60.0, 40.0],
                vec![
                    ("bg", s("#fafafa")),
                    ("radius", f(8.0)),
                    (
                        "shadow",
                        m(vec![
                            ("color", s("#7c3aed99")),
                            ("blur", f(10.0)),
                            ("x", f(-3.0)),
                            ("spread", f(4.0)),
                        ]),
                    ),
                ],
            ),
            boxed(
                "hair",
                [24.0, 230.333, 300.0, 1.0],
                vec![("bg", s("#18181b"))],
            ),
        ],
    )
}

#[test]
fn the_gpu_draws_shadows_and_continuous_corners_as_software_does() {
    for scale in [1.0f32, 1.5, 2.0] {
        let mut st = geometry_scene(scale);
        agree(&mut st, &format!("at {scale}x"));
    }
}

#[test]
fn a_hairline_is_one_whole_pixel_at_a_fractional_scale() {
    // 1.5×: a one-point rule at y = 10.333 is 1.5 device pixels from 15.5;
    // drawn as it lies it is two grey rows, snapped it is one black one
    for y in [10.0, 10.333, 10.5, 11.25] {
        let mut st = window(
            40.0,
            30.0,
            1.5,
            vec![boxed(
                "rule",
                [2.0, y, 30.0, 1.0],
                vec![("bg", s("#000000"))],
            )],
        );
        let (px, w) = soft_px(&mut st);
        let col: Vec<u8> = (0..(30.0 * 1.5) as u32)
            .map(|r| at(&px, w, 20, r)[0])
            .collect();
        let inked: Vec<usize> = col
            .iter()
            .enumerate()
            .filter(|(_, v)| **v < 255)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            inked.len(),
            1,
            "at y = {y}: one row inked, got {inked:?} ({col:?})"
        );
        assert_eq!(
            col[inked[0]], 0,
            "at y = {y}: the row is the rule's colour, whole"
        );
    }
}

#[test]
fn a_border_at_a_fractional_scale_is_whole_pixels() {
    let mut st = window(
        60.0,
        40.0,
        1.5,
        vec![boxed(
            "card",
            [5.333, 5.333, 40.0, 20.0],
            vec![
                ("bg", s("#ffffff")),
                ("border", s("#000000")),
                ("border_width", f(1.0)),
            ],
        )],
    );
    let (px, w) = soft_px(&mut st);
    // across the card's middle row: the left border one black pixel, then white
    let y = ((5.333 + 10.0) * 1.5) as u32;
    let row: Vec<u8> = (0..20).map(|x| at(&px, w, x, y)[0]).collect();
    let dark: Vec<usize> = row
        .iter()
        .enumerate()
        .filter(|(_, v)| **v < 250)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(dark.len(), 1, "one column of border: {row:?}");
    assert_eq!(row[dark[0]], 0, "the border is whole: {row:?}");
    agree(&mut st, "a border at 1.5x");
}

#[test]
fn continuous_corners_ease_in_from_the_edge() {
    let corner = |kind: &str| {
        let mut st = window(
            120.0,
            120.0,
            1.0,
            vec![boxed(
                "b",
                [10.0, 10.0, 100.0, 100.0],
                vec![
                    ("bg", s("#000000")),
                    ("radius", f(20.0)),
                    ("corner", s(kind)),
                ],
            )],
        );
        soft_px(&mut st)
    };
    let (round, w) = corner("round");
    let (cont, _) = corner("continuous");
    // past the quarter circle (20 from the corner) the round box's top
    // edge is straight and whole; the continuous one's is still curving
    // away (it reaches 1.528 × 20 along the edge)
    assert_eq!(at(&round, w, 32, 10)[0], 0, "round: whole past its corner");
    assert!(
        at(&cont, w, 32, 10)[0] > 10,
        "continuous: still curving at 22 from the corner"
    );
    // and within it the continuous curve leaves the edge later, then faster
    let (r, c) = (at(&round, w, 25, 10)[0], at(&cont, w, 25, 10)[0]);
    assert!(
        c > r + 60,
        "continuous lighter than round at 15 from the corner ({c} against {r})"
    );
    // along the diagonal both are as deep (the same radius looks the same size)
    let d = |px: &[u8]| {
        (0..30)
            .find(|i| at(px, w, 10 + i, 10 + i)[0] < 128)
            .unwrap_or(99)
    };
    assert!(
        d(&round).abs_diff(d(&cont)) <= 1,
        "as deep at the diagonal: {} and {}",
        d(&round),
        d(&cont)
    );
    // a capsule's ends stay round: no room to ease in
    let mut a = window(
        80.0,
        40.0,
        1.0,
        vec![boxed(
            "p",
            [10.0, 10.0, 60.0, 20.0],
            vec![("bg", s("#000000")), ("radius", f(10.0))],
        )],
    );
    let mut b = window(
        80.0,
        40.0,
        1.0,
        vec![boxed(
            "p",
            [10.0, 10.0, 60.0, 20.0],
            vec![
                ("bg", s("#000000")),
                ("radius", f(10.0)),
                ("corner", s("continuous")),
            ],
        )],
    );
    let (pa, wa) = soft_px(&mut a);
    let (pb, _) = soft_px(&mut b);
    let worst = pa
        .iter()
        .zip(&pb)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= 16,
        "a capsule's continuous ends are its round ones (worst {worst}, {wa} wide)"
    );
}

#[test]
fn the_window_says_every_corner_is_continuous() {
    let draw = |continuous: Option<bool>, own: Option<&str>| {
        let mut style = vec![("bg", s("#000000")), ("radius", f(20.0))];
        if let Some(c) = own {
            style.push(("corner", s(c)));
        }
        let mut st = window(
            120.0,
            120.0,
            1.0,
            vec![boxed("b", [10.0, 10.0, 100.0, 100.0], style)],
        );
        if let Some(c) = continuous {
            st.scene.continuous = c;
        }
        soft_px(&mut st).0
    };
    let round = draw(None, None);
    let cont = draw(None, Some("continuous"));
    assert_ne!(round, cont);
    assert_eq!(
        draw(Some(true), None),
        cont,
        "the window's corner is the default"
    );
    assert_eq!(
        draw(Some(true), Some("round")),
        round,
        "a style's own corner wins"
    );
}

#[test]
fn a_shadow_falls_off_as_a_gaussian() {
    let mut st = window(
        200.0,
        200.0,
        1.0,
        vec![boxed(
            "b",
            [50.0, 50.0, 100.0, 100.0],
            vec![
                ("bg", s("#ffffff00")),
                (
                    "shadow",
                    m(vec![("color", s("#000000ff")), ("blur", f(20.0))]),
                ),
            ],
        )],
    );
    let (px, w) = soft_px(&mut st);
    // inside, a whole shadow; at its edge half; three sigmas out nothing
    assert_eq!(
        at(&px, w, 100, 100)[0],
        0,
        "inside: the shadow's colour, whole"
    );
    let edge = at(&px, w, 150, 100)[0] as i32;
    assert!((edge - 128).abs() <= 6, "at the box's edge: half ({edge})");
    assert_eq!(at(&px, w, 182, 100)[0], 255, "three sigmas out: nothing");
    // falling off steadily
    let row: Vec<u8> = (140..182).map(|x| at(&px, w, x, 100)[0]).collect();
    assert!(row.windows(2).all(|p| p[0] <= p[1]), "steady: {row:?}");
    // a corner's is rounder and fainter than an edge's
    let c = at(&px, w, 155, 155)[0];
    let e = at(&px, w, 155, 100)[0];
    assert!(
        c > e,
        "fainter past the corner ({c}) than past the edge ({e})"
    );
    agree(&mut st, "a black shadow");
}

#[test]
fn a_style_says_its_shadow_corner_and_material_or_refuses() {
    let try_style = |style: Vec<(&str, Value)>| {
        let mut st = WinState::new(1, true, "t", 40.0, 40.0, 1.0);
        let ops = Value::List(Arc::new(vec![m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 40.0, 40.0])),
            ("style", m(style)),
        ])]));
        let mut out = Vec::new();
        st.apply(&ops, &mut out).map(|_| ())
    };
    assert!(try_style(vec![("corner", s("continuous"))]).is_ok());
    assert!(try_style(vec![("corner", s("squircle"))]).is_err());
    assert!(try_style(vec![("shadow", m(vec![("blur", f(4.0))]))]).is_ok());
    assert!(try_style(vec![("shadow", Value::Unit)]).is_ok());
    assert!(try_style(vec![("shadow", f(3.0))]).is_err());
    assert!(try_style(vec![("material", s("popover"))]).is_ok());
    assert!(try_style(vec![("material", s("glass"))]).is_err());
}

fn canvas(key: &str, b: [f64; 4], ops: Vec<Value>) -> Value {
    m(vec![
        ("key", s(key)),
        ("parent", s("root")),
        ("role", s("figure")),
        ("box", tup(&b)),
        ("draw", Value::List(Arc::new(ops))),
    ])
}

fn l(v: Vec<Value>) -> Value {
    Value::List(Arc::new(v))
}

/// Vector paths of every kind: curves filled and stroked, joins, caps,
/// dashes, a dashed circle, a hairline, a long chart line, clipped.
fn paths_scene(scale: f32) -> WinState {
    let pts: Vec<Value> = (0..2000)
        .map(|i| {
            let x = i as f64 * 0.17;
            tup(&[x, 60.0 + 30.0 * (x * 0.21).sin() + 8.0 * (x * 1.7).cos()])
        })
        .collect();
    window(
        380.0,
        300.0,
        scale,
        vec![
            canvas(
                "shapes",
                [10.0, 10.0, 360.0, 150.0],
                vec![
                    m(vec![
                        ("op", s("path")),
                        (
                            "d",
                            l(vec![
                                l(vec![s("M"), f(10.0), f(120.0)]),
                                l(vec![
                                    s("C"),
                                    f(40.0),
                                    f(-20.0),
                                    f(90.0),
                                    f(160.0),
                                    f(130.0),
                                    f(30.0),
                                ]),
                                l(vec![s("Q"), f(150.0), f(0.0), f(170.0), f(110.0)]),
                                l(vec![s("Z")]),
                            ]),
                        ),
                        ("fill", s("#2563eb66")),
                        ("stroke", s("#1e3a8a")),
                        ("width", f(2.5)),
                    ]),
                    m(vec![
                        ("op", s("path")),
                        (
                            "points",
                            l(vec![
                                tup(&[190.0, 120.0]),
                                tup(&[220.0, 20.0]),
                                tup(&[250.0, 120.0]),
                                tup(&[280.0, 30.0]),
                            ]),
                        ),
                        ("stroke", s("#b91c1c")),
                        ("width", f(6.0)),
                        ("join", s("miter")),
                        ("cap", s("square")),
                    ]),
                    m(vec![
                        ("op", s("path")),
                        (
                            "points",
                            l(vec![
                                tup(&[290.0, 120.0]),
                                tup(&[310.0, 40.0]),
                                tup(&[340.0, 120.0]),
                            ]),
                        ),
                        ("stroke", s("#15803d")),
                        ("width", f(5.0)),
                        ("join", s("bevel")),
                        ("cap", s("butt")),
                    ]),
                    m(vec![
                        ("op", s("line")),
                        ("x1", f(10.0)),
                        ("y1", f(140.0)),
                        ("x2", f(350.0)),
                        ("y2", f(135.0)),
                        ("color", s("#18181b")),
                        ("width", f(1.5)),
                        ("dash", tup(&[6.0, 3.0])),
                        ("cap", s("round")),
                    ]),
                    m(vec![
                        ("op", s("circle")),
                        ("cx", f(320.0)),
                        ("cy", f(30.0)),
                        ("r", f(18.0)),
                        ("stroke", s("#7c3aed")),
                        ("width", f(2.0)),
                        ("dash", tup(&[4.0, 4.0])),
                    ]),
                    m(vec![
                        ("op", s("circle")),
                        ("cx", f(280.0)),
                        ("cy", f(140.0)),
                        ("r", f(6.0)),
                        ("fill", s("#f59e0b")),
                        ("stroke", s("#78350f")),
                        ("width", f(1.0)),
                    ]),
                    m(vec![
                        ("op", s("line")),
                        ("x1", f(10.0)),
                        ("y1", f(5.0)),
                        ("x2", f(180.0)),
                        ("y2", f(12.0)),
                        ("color", s("#18181b")),
                        ("width", f(0.5)),
                    ]),
                ],
            ),
            canvas(
                "chart",
                [10.0, 170.0, 300.0, 120.0],
                vec![
                    m(vec![
                        ("op", s("push")),
                        ("clip", tup(&[0.0, 0.0, 300.0, 120.0])),
                    ]),
                    m(vec![
                        ("op", s("path")),
                        ("points", l(pts)),
                        ("stroke", s("#2563eb")),
                        ("width", f(1.5)),
                    ]),
                    m(vec![("op", s("pop"))]),
                ],
            ),
        ],
    )
}

#[test]
fn the_gpu_draws_vector_paths_as_software_does() {
    for scale in [1.0f32, 2.0, 1.5] {
        let mut st = paths_scene(scale);
        agree(&mut st, &format!("paths at {scale}x"));
    }
}

#[test]
fn a_path_is_drawn_as_geometry_not_a_picture() {
    use olang::stdlib::gui::window::Prim;
    let mut st = paths_scene(2.0);
    let dl = st.display_list();
    let paths = dl
        .prims
        .iter()
        .filter(|p| matches!(p, Prim::Path(_)))
        .count();
    let images = dl
        .prims
        .iter()
        .filter(|p| matches!(p, Prim::Image { .. }))
        .count();
    assert_eq!(images, 0, "no picture rasterized for a path");
    // the curve, two polylines, the dashed line, the dashed circle, the
    // hairline, the chart; the plain circle is a box with round corners
    assert_eq!(paths, 7);
}

#[test]
fn a_canvas_path_says_what_it_cannot_draw() {
    let err = |op: Value| {
        olang::stdlib::gui::canvas::draw(&l(vec![op]), 40.0, 40.0, 1.0)
            .err()
            .unwrap_or_default()
    };
    assert!(
        err(m(vec![
            ("op", s("path")),
            ("d", l(vec![l(vec![s("X"), f(1.0)])]))
        ]))
        .contains("unknown command")
    );
    assert!(
        err(m(vec![
            ("op", s("path")),
            ("d", l(vec![l(vec![s("C"), f(1.0)])]))
        ]))
        .contains("takes 6 numbers")
    );
    assert!(err(m(vec![("op", s("line")), ("cap", s("pointy"))])).contains("\"cap\""));
    assert!(
        err(m(vec![
            ("op", s("path")),
            ("points", l(vec![tup(&[0.0, 0.0]), tup(&[9.0, 9.0])])),
            ("stroke", s("#000000")),
            ("join", s("sharp"))
        ]))
        .contains("\"join\"")
    );
}

#[test]
fn gradients_mix_in_linear_light() {
    use olang::stdlib::gui::canvas::gradient_at;
    // red to green: the middle is a clear yellow, not a muddy brown
    let mid = gradient_at(&[[255, 0, 0, 255], [0, 255, 0, 255]], 0.5);
    assert!(
        (mid[0] - 188.0).abs() < 1.5 && (mid[1] - 188.0).abs() < 1.5,
        "{mid:?}"
    );
    // a fade to transparent keeps its colour (premultiplied)
    let fade = gradient_at(&[[37, 99, 235, 255], [0, 0, 0, 0]], 0.5);
    assert!(
        (fade[2] - 235.0).abs() < 1.0 && (fade[3] - 127.5).abs() < 1.0,
        "{fade:?}"
    );
    // the ends are the stops
    assert_eq!(
        gradient_at(&[[10, 20, 30, 255], [200, 100, 50, 255]], 0.0).map(|v| v.round()),
        [10.0, 20.0, 30.0, 255.0]
    );
}

#[test]
fn the_gpu_draws_gradients_as_software_does() {
    for scale in [1.0f32, 2.0] {
        let mut st = window(
            260.0,
            140.0,
            scale,
            vec![canvas(
                "g",
                [10.0, 10.0, 240.0, 120.0],
                vec![
                    m(vec![
                        ("op", s("rect")),
                        ("x", f(0.0)),
                        ("y", f(0.0)),
                        ("w", f(110.0)),
                        ("h", f(110.0)),
                        ("radius", f(14.0)),
                        ("dir", s("radial")),
                        (
                            "gradient",
                            l(vec![s("#ffffff"), s("#2563eb"), s("#1e1b4b")]),
                        ),
                    ]),
                    m(vec![
                        ("op", s("rect")),
                        ("x", f(120.0)),
                        ("y", f(0.0)),
                        ("w", f(110.0)),
                        ("h", f(110.0)),
                        ("dir", s("y")),
                        ("gradient", l(vec![s("#ef4444"), s("#22c55e00")])),
                    ]),
                ],
            )],
        );
        agree(&mut st, &format!("gradients at {scale}x"));
        // the radial one is lightest at its centre
        let (px, w) = soft_px(&mut st);
        let c = (65.0 * scale as f64) as u32;
        let centre = at(&px, w, c, c);
        let edge = at(&px, w, (14.0 * scale) as u32, c);
        assert!(
            centre[0] > 240 && edge[0] < centre[0],
            "centre {centre:?}, edge {edge:?}"
        );
    }
}

/// Stripes under a popover with a material (a translucent white box).
fn material_scene(scale: f32, reduce: bool, stripe_shift: f64) -> WinState {
    let mut nodes = Vec::new();
    for i in 0..30 {
        nodes.push(boxed(
            &format!("s{i}"),
            [i as f64 * 8.0 + stripe_shift, 0.0, 4.0, 160.0],
            vec![("bg", s("#18181b"))],
        ));
    }
    nodes.push(boxed(
        "pop",
        [40.0, 30.0, 140.0, 90.0],
        vec![
            ("bg", s("#ffffffcc")),
            ("radius", f(10.0)),
            ("material", s("popover")),
            ("border", s("#d4d4d8")),
            ("border_width", f(1.0)),
        ],
    ));
    let mut st = window(240.0, 160.0, scale, nodes);
    st.settings.reduce_transparency = reduce;
    st
}

#[test]
fn a_material_is_its_opaque_fallback_in_software_and_under_reduced_transparency() {
    use olang::stdlib::gui::window::Prim;
    for reduce in [false, true] {
        let mut st = material_scene(1.0, reduce, 0.0);
        let dl = st.display_list();
        let backdrops = dl
            .prims
            .iter()
            .filter(|p| matches!(p, Prim::Backdrop { .. }))
            .count();
        assert_eq!(backdrops, if reduce { 0 } else { 1 });
        let (px, w) = soft_px(&mut st);
        // inside the popover: white whatever is under it (the stripes gone)
        let row: Vec<u8> = (60..160).map(|x| at(&px, w, x, 75)[0]).collect();
        assert!(row.iter().all(|v| *v == 255), "opaque: {row:?}");
    }
}

#[test]
fn the_gpu_blurs_what_is_under_a_material_and_damage_keeps_it_whole() {
    let _turn = GPU_TURN.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(g) = gpu::gpu(None, None) else {
        eprintln!("skipped: no GPU here");
        return;
    };
    let mut st = material_scene(2.0, false, 0.0);
    let dl = st.display_list();
    let px = g.render_rgba(&dl);
    let w = dl.width;
    if let Some(dir) = std::env::var_os("GUI_GEOMETRY_DUMP") {
        let mut pm = tiny_skia::Pixmap::new(dl.width, dl.height).expect("a pixmap");
        for (i, c) in px.chunks(4).enumerate() {
            pm.pixels_mut()[i] =
                tiny_skia::ColorU8::from_rgba(c[0], c[1], c[2], c[3]).premultiply();
        }
        let _ = pm.save_png(std::path::Path::new(&dir).join("material-gpu.png"));
    }
    // inside: the stripes blurred to an even grey under the white (no
    // stripe whole), lighter than a stripe and darker than white
    let row: Vec<u8> = (140..340).map(|x| at(&px, w, x, 150)[0]).collect();
    let (lo, hi) = (*row.iter().min().unwrap(), *row.iter().max().unwrap());
    assert!(hi - lo < 24 && lo > 150 && hi < 250, "blurred: {lo}..{hi}");
    // outside: the stripes as they are
    assert_eq!(at(&px, w, 1, 10)[0], 24);
    // the stripes moved under it: a frame drawn over its damage is the
    // frame drawn whole (the material read what is under it again)
    let mut target = gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm);
    target.render(&dl);
    for shift in [3.0, 5.0, 11.0] {
        let mut moved = material_scene(2.0, false, shift);
        let dl2 = moved.display_list();
        target.render(&dl2);
        let damaged = target.rgba();
        let whole = g.render_rgba(&dl2);
        let worst = damaged
            .iter()
            .zip(&whole)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(
            worst <= 2,
            "stripes moved {shift}: damaged against whole, worst {worst}"
        );
    }
}

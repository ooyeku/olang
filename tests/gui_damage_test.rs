//! Damage tracking (loom/ROADMAP.md item 6) and the differential fuzz of
//! the engine's drawing (item 25): random patches, input, pictures and
//! text, and every frame drawn three ways — whole, damaged over the kept
//! frame, and on the GPU — which must agree.
//!
//! - software, damaged ≡ software, whole, byte for byte;
//! - GPU, damaged ≡ GPU, whole, byte for byte;
//! - GPU ≡ software within the renderers' antialiasing tolerance.
//!
//! The cheap run is part of the suite. `GUI_FUZZ_STEPS=5000` (and
//! `GUI_FUZZ_SEED`) runs longer; a failure says its seed and step.

#![cfg(feature = "gui")]

use olang::Value;
use olang::stdlib::gui::damage::Damage;
use olang::stdlib::gui::window::{Clipboard, Input, Mods, PointerAction, WinState};
use olang::stdlib::gui::{gpu, soft};
use std::sync::Arc;

/// The GPU's glyph atlas is the process's: tests that draw on the GPU take
/// turns (one filling every page would leave another's frame no room).
static GPU_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_turn() -> std::sync::MutexGuard<'static, ()> {
    GPU_TURN.lock().unwrap_or_else(|e| e.into_inner())
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next() % 10_000) as f64 / 10_000.0 * (hi - lo)
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

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

struct NoClip;
impl Clipboard for NoClip {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _: String) {}
}

const W: f64 = 420.0;
const H: f64 = 300.0;
const WORDS: &[&str] = &[
    "Loom",
    "damage",
    "tile",
    "glyph",
    "atlas",
    "caret",
    "scroll",
    "W",
    "fifty",
    "olang",
    "∑",
    "é",
    "Résumé",
    "Quick brown fox",
    "1234567890",
    "ffi",
];

fn colour(r: &mut Rng) -> Value {
    let a = if r.chance(25) { r.below(255) } else { 255 };
    s(&format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        r.below(256),
        r.below(256),
        r.below(256),
        a
    ))
}

fn style(r: &mut Rng) -> Value {
    let mut f = vec![("bg", colour(r))];
    if r.chance(40) {
        f.push(("border", colour(r)));
        f.push(("border_width", Value::Float(r.f(0.5, 4.0))));
    }
    if r.chance(40) {
        f.push(("radius", Value::Float(r.f(0.0, 18.0))));
    }
    if r.chance(30) {
        f.push(("clip", Value::Boolean(true)));
    }
    if r.chance(25) {
        f.push((
            "corner",
            s(if r.chance(70) { "continuous" } else { "round" }),
        ));
    }
    if r.chance(12) {
        f.push((
            "shadow",
            m(vec![
                ("color", colour(r)),
                ("blur", Value::Float(r.f(0.0, 24.0))),
                ("x", Value::Float(r.f(-6.0, 6.0))),
                ("y", Value::Float(r.f(-2.0, 10.0))),
                ("spread", Value::Float(r.f(-4.0, 6.0))),
            ]),
        ));
    }
    if r.chance(15) {
        f.push(("opacity", Value::Float(r.f(0.2, 1.0))));
    }
    if r.chance(30) {
        f.push(("hover", m(vec![("bg", colour(r))])));
    }
    if r.chance(50) {
        f.push(("color", colour(r)));
        f.push(("size", Value::Float(r.f(9.0, 26.0))));
    }
    m(f)
}

fn a_box(r: &mut Rng) -> Value {
    let x = r.f(-20.0, W - 20.0);
    let y = r.f(-20.0, H - 20.0);
    tup(&[x.round(), y.round(), r.f(4.0, 220.0), r.f(2.0, 120.0)])
}

fn canvas_ops(r: &mut Rng) -> Value {
    let n = |v: f64| Value::Float(v);
    let mut ops = Vec::new();
    for _ in 0..1 + r.below(4) {
        ops.push(match r.below(5) {
            4 => {
                // a vector path: a polyline or curves, filled and or stroked,
                // its joins, caps and dashes
                let pts: Vec<Value> = (0..2 + r.below(6))
                    .map(|_| tup(&[r.f(0.0, 90.0), r.f(0.0, 70.0)]))
                    .collect();
                let mut f = if r.chance(50) {
                    vec![("op", s("path")), ("points", Value::List(Arc::new(pts)))]
                } else {
                    let p = |r: &mut Rng| [r.f(0.0, 90.0), r.f(0.0, 70.0)];
                    let (a, b, c, d) = (p(r), p(r), p(r), p(r));
                    let cmd = |v: Vec<Value>| Value::List(Arc::new(v));
                    vec![
                        ("op", s("path")),
                        (
                            "d",
                            cmd(vec![
                                cmd(vec![s("M"), n(a[0]), n(a[1])]),
                                cmd(vec![s("Q"), n(b[0]), n(b[1]), n(c[0]), n(c[1])]),
                                cmd(vec![
                                    s("C"),
                                    n(d[0]),
                                    n(d[1]),
                                    n(a[0]),
                                    n(c[1]),
                                    n(b[0]),
                                    n(a[1]),
                                ]),
                            ]),
                        ),
                    ]
                };
                if r.chance(40) {
                    f.push(("close", Value::Boolean(true)));
                    f.push(("fill", colour(r)));
                }
                if r.chance(70) {
                    f.push(("stroke", colour(r)));
                    f.push(("width", n(r.f(0.3, 8.0))));
                    f.push(("join", s(["round", "miter", "bevel"][r.below(3) as usize])));
                    f.push(("cap", s(["round", "butt", "square"][r.below(3) as usize])));
                    if r.chance(30) {
                        f.push(("dash", tup(&[r.f(1.0, 8.0), r.f(1.0, 6.0)])));
                    }
                }
                m(f)
            }
            0 => m(vec![
                ("op", s("rect")),
                ("x", n(r.f(0.0, 60.0))),
                ("y", n(r.f(0.0, 40.0))),
                ("w", n(r.f(4.0, 60.0))),
                ("h", n(r.f(4.0, 40.0))),
                ("fill", colour(r)),
            ]),
            1 => m(vec![
                ("op", s("circle")),
                ("cx", n(r.f(10.0, 60.0))),
                ("cy", n(r.f(10.0, 50.0))),
                ("r", n(r.f(3.0, 30.0))),
                ("fill", colour(r)),
            ]),
            2 => m(vec![
                ("op", s("line")),
                ("x1", n(r.f(0.0, 80.0))),
                ("y1", n(r.f(0.0, 60.0))),
                ("x2", n(r.f(0.0, 80.0))),
                ("y2", n(r.f(0.0, 60.0))),
                ("color", colour(r)),
                ("width", n(r.f(0.5, 4.0))),
            ]),
            _ => m(vec![
                ("op", s("text")),
                ("x", n(r.f(0.0, 60.0))),
                ("y", n(r.f(0.0, 60.0))),
                ("text", s(WORDS[r.below(WORDS.len() as u64) as usize])),
                ("size", n(r.f(8.0, 20.0))),
            ]),
        });
    }
    Value::List(Arc::new(ops))
}

/// A picture for image nodes: a PNG of four coloured quarters, once.
fn picture_path() -> String {
    static P: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("gui-damage-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut pm = tiny_skia::Pixmap::new(40, 30).expect("a pixmap");
        for (x, y, c) in [
            (0, 0, [220, 38, 38]),
            (20, 0, [37, 99, 235]),
            (0, 15, [22, 163, 74]),
            (20, 15, [245, 158, 11]),
        ] {
            let mut p = tiny_skia::Paint::default();
            p.set_color_rgba8(c[0], c[1], c[2], 200);
            pm.fill_rect(
                tiny_skia::Rect::from_xywh(x as f32, y as f32, 20.0, 15.0).expect("a rect"),
                &p,
                tiny_skia::Transform::identity(),
                None,
            );
        }
        let path = dir.join("quarters.png");
        pm.save_png(&path).expect("written");
        path.to_string_lossy().to_string()
    })
    .clone()
}

/// A hostile style or box now and then: absurd sizes, huge places,
/// nothing at all (what `tests/gui_hostile_test.rs` throws, here drawn
/// three ways).
fn hostile(r: &mut Rng, f: &mut Vec<(&str, Value)>) {
    match r.below(6) {
        0 => f.push((
            "box",
            tup(&[r.f(-1e6, 1e6), r.f(-1e6, 1e6), r.f(0.0, 1e6), r.f(0.0, 1e6)]),
        )),
        1 => f.push(("box", tup(&[r.f(0.0, W), r.f(0.0, H), 0.0, 0.0]))),
        2 => f.push((
            "style",
            m(vec![
                ("bg", colour(r)),
                ("radius", Value::Float(1e6)),
                ("border", colour(r)),
                ("border_width", Value::Float(500.0)),
            ]),
        )),
        3 => f.push((
            "style",
            m(vec![
                ("size", Value::Float(if r.chance(50) { 0.5 } else { 900.0 })),
                ("color", colour(r)),
            ]),
        )),
        4 => f.push((
            "style",
            m(vec![("bg", colour(r)), ("opacity", Value::Float(0.0))]),
        )),
        _ => f.push((
            "style",
            m(vec![
                ("bg", s("#00000000")),
                ("border", colour(r)),
                ("border_width", Value::Float(0.25)),
            ]),
        )),
    }
}

/// A node, new or changed: a box, a text, a canvas, a picture, or a
/// scroll region.
fn node(r: &mut Rng, key: &str, parents: &[String]) -> Value {
    let parent = parents[r.below(parents.len() as u64) as usize].clone();
    let mut f = vec![
        ("key", s(key)),
        ("parent", s(&parent)),
        ("box", a_box(r)),
        ("style", style(r)),
    ];
    if r.chance(5) {
        f.retain(|(k, _)| *k != "box" && *k != "style");
        hostile(r, &mut f);
        if !f.iter().any(|(k, _)| *k == "box") {
            f.push(("box", a_box(r)));
        }
    }
    match r.below(6) {
        0 | 1 => {
            let words: Vec<&str> = (0..1 + r.below(6))
                .map(|_| WORDS[r.below(WORDS.len() as u64) as usize])
                .collect();
            f.push(("role", s("text")));
            f.push(("text", s(&words.join(" "))));
        }
        2 => {
            f.push(("role", s("figure")));
            f.push(("draw", canvas_ops(r)));
        }
        4 => {
            f.push(("role", s("image")));
            f.push(("image", s(&picture_path())));
            f.push(("fit", s(["contain", "cover", "fill"][r.below(3) as usize])));
        }
        3 => {
            f.push(("role", s("region")));
            f.push(("content", tup(&[300.0, 600.0])));
        }
        _ => {}
    }
    m(f)
}

/// A world of nodes, patched at random.
struct World {
    st: WinState,
    keys: Vec<String>,
    next: usize,
}

impl World {
    fn new(r: &mut Rng, scale: f32) -> World {
        let mut st = WinState::new(1, true, "damage", W as f32, H as f32, scale);
        let mut ops = vec![
            m(vec![
                ("key", s("root")),
                ("box", tup(&[0.0, 0.0, W, H])),
                ("style", m(vec![("bg", s("#f4f4f5"))])),
            ]),
            m(vec![
                ("key", s("field")),
                ("parent", s("root")),
                ("role", s("input")),
                ("box", tup(&[12.0, H - 44.0, 300.0, 32.0])),
                ("edit", m(vec![("value", s("type here"))])),
                (
                    "style",
                    m(vec![
                        ("bg", s("#ffffff")),
                        ("border", s("#a1a1aa")),
                        ("border_width", Value::Integer(1)),
                        ("radius", Value::Integer(6)),
                    ]),
                ),
            ]),
        ];
        // a long list scrolled by the steps (its rows are not the world's
        // to change: they are what a move must keep)
        ops.push(m(vec![
            ("key", s("big")),
            ("parent", s("root")),
            ("role", s("region")),
            ("box", tup(&[150.0, 0.0, 260.0, 250.0])),
            ("content", tup(&[260.0, 2400.0])),
            (
                "style",
                m(vec![("bg", s("#ffffff")), ("clip", Value::Boolean(true))]),
            ),
        ]));
        for i in 0..80 {
            ops.push(m(vec![
                ("key", s(&format!("big{i}"))),
                ("parent", s("big")),
                ("role", s("text")),
                ("box", tup(&[4.0, i as f64 * 30.0, 250.0, 26.0])),
                ("text", s(&format!("{} {i}", WORDS[i % WORDS.len()]))),
                (
                    "style",
                    m(vec![
                        ("bg", colour(r)),
                        ("radius", Value::Float(r.f(0.0, 8.0))),
                    ]),
                ),
            ]));
        }
        let mut keys = vec!["root".to_string()];
        for i in 0..10 {
            let k = format!("n{i}");
            ops.push(node(r, &k, &keys));
            keys.push(k);
        }
        let mut out = Vec::new();
        st.apply(&Value::List(Arc::new(ops)), &mut out)
            .expect("applies");
        st.apply(
            &Value::List(Arc::new(vec![m(vec![
                ("op", s("focus")),
                ("key", s("field")),
            ])])),
            &mut out,
        )
        .expect("focus");
        st.input(Input::Focused(true), &mut NoClip, &mut out);
        World { st, keys, next: 10 }
    }

    /// One random step: a patch, input, or nothing.
    fn step(&mut self, r: &mut Rng) {
        let mut out = Vec::new();
        let mut clip = NoClip;
        let pick = if self.keys.len() < 2 { 2 } else { r.below(10) };
        match pick {
            // a node changed
            0 | 1 => {
                let k = self.keys[1 + r.below(self.keys.len() as u64 - 1) as usize].clone();
                let parents: Vec<String> =
                    self.keys.iter().take_while(|p| **p != k).cloned().collect();
                let op = node(r, &k, &parents);
                let _ = self.st.apply(&Value::List(Arc::new(vec![op])), &mut out);
            }
            // a node added
            2 => {
                let k = format!("n{}", self.next);
                self.next += 1;
                let op = node(r, &k, &self.keys);
                if self
                    .st
                    .apply(&Value::List(Arc::new(vec![op])), &mut out)
                    .is_ok()
                {
                    self.keys.push(k);
                }
            }
            // a node removed (its subtree goes with it)
            3 if self.keys.len() > 3 => {
                let i = 1 + r.below(self.keys.len() as u64 - 1) as usize;
                let k = self.keys[i].clone();
                let _ = self.st.apply(
                    &Value::List(Arc::new(vec![m(vec![("op", s("remove")), ("key", s(&k))])])),
                    &mut out,
                );
                self.keys.retain(|x| self.st.scene.nodes.contains_key(x));
            }
            // the long list scrolled a little (often by whole pixels)
            4 if r.chance(60) => {
                let at = self
                    .st
                    .scene
                    .nodes
                    .get("big")
                    .map(|n| n.offset.1)
                    .unwrap_or(0.0) as f64;
                let by = r.f(-80.0, 80.0);
                let by = if r.chance(70) { by.round() } else { by };
                let _ = self.st.apply(
                    &Value::List(Arc::new(vec![m(vec![
                        ("op", s("scroll")),
                        ("key", s("big")),
                        ("to", tup(&[0.0, (at + by).max(0.0)])),
                    ])])),
                    &mut out,
                );
            }
            // a scroll
            4 => {
                let k = self.keys[r.below(self.keys.len() as u64) as usize].clone();
                let _ = self.st.apply(
                    &Value::List(Arc::new(vec![m(vec![
                        ("op", s("scroll")),
                        ("key", s(&k)),
                        ("to", tup(&[0.0, r.f(0.0, 200.0).round()])),
                    ])])),
                    &mut out,
                );
            }
            // the pointer moves (hover looks), or wheels
            5 => self.st.input(
                Input::Pointer {
                    action: PointerAction::Move,
                    x: r.f(0.0, W) as f32,
                    y: r.f(0.0, H) as f32,
                    button: "left".into(),
                    clicks: 0,
                },
                &mut clip,
                &mut out,
            ),
            6 => self.st.input(
                Input::Wheel {
                    dx: 0.0,
                    dy: r.f(-60.0, 60.0) as f32,
                    mods: None,
                },
                &mut clip,
                &mut out,
            ),
            // typing in the field
            7 => {
                let text = WORDS[r.below(WORDS.len() as u64) as usize];
                let key = if r.chance(20) { "backspace" } else { "x" };
                self.st.input(
                    Input::Key {
                        key: key.into(),
                        text: (key != "backspace").then(|| text.to_string()),
                        mods: Mods::default(),
                        repeat: false,
                    },
                    &mut clip,
                    &mut out,
                );
            }
            // the window's focus (rings and carets) or nothing at all
            8 => self
                .st
                .input(Input::Focused(r.chance(50)), &mut clip, &mut out),
            _ => {}
        }
    }
}

fn steps(default: usize) -> usize {
    std::env::var("GUI_FUZZ_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn seed(default: u64) -> u64 {
    std::env::var("GUI_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn first_difference(a: &[u8], b: &[u8], w: u32) -> String {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => {
            let px = i / 4;
            format!(
                "first at ({}, {}): {:?} against {:?}",
                px % w as usize,
                px / w as usize,
                &a[px * 4..px * 4 + 4],
                &b[px * 4..px * 4 + 4]
            )
        }
        None => format!("lengths {} and {}", a.len(), b.len()),
    }
}

/// How far a pixel the GPU drew elsewhere and moved may be from its whole
/// frame's, per channel.
const MOVED_TOLERANCE: u8 = 2;

/// Two frames alike within `tol` per channel.
fn within(a: &[u8], b: &[u8], tol: u8) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= tol)
}

/// The renderers' antialiasing differs; beyond this they disagree.
fn gpu_agrees(soft_px: &[u8], gpu_px: &[u8]) -> Result<(), String> {
    let n = soft_px.len() / 4;
    let (mut over, mut total) = (0usize, 0u64);
    for (a, b) in soft_px.chunks(4).zip(gpu_px.chunks(4)) {
        let d = (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap_or(0);
        total += d as u64;
        if d > 48 {
            over += 1;
        }
    }
    let mean = total as f64 / n.max(1) as f64;
    if over * 200 <= n && mean < 2.0 {
        Ok(())
    } else {
        Err(format!(
            "{over} of {n} pixels differ by more than 48 (mean {mean:.3})"
        ))
    }
}

/// Drive a world and hold every frame's three drawings to each other.
fn fuzz(seed: u64, steps: usize, scale: f32, with_gpu: bool) -> (usize, usize, usize) {
    let mut r = Rng(seed.max(1));
    let mut world = World::new(&mut r, scale);
    let mut kept = soft::Retained::default();
    let g = if with_gpu {
        gpu::gpu(None, None).ok()
    } else {
        None
    };
    let mut target = g
        .as_ref()
        .map(|g| gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm));
    let (mut partial, mut unchanged, mut whole, mut moved) = (0, 0, 0, 0);
    // pixels the GPU moved since its last whole frame: drawn where they
    // were, they may differ from a whole frame's in the last bit (the
    // shader's arithmetic is in absolute f32 places)
    let mut gpu_moved = false;
    for step in 0..steps {
        if step > 0 {
            world.step(&mut r);
        }
        let dl = world.st.display_list();
        let full = soft::render(&dl);
        let d = kept.render(&dl).clone();
        match d {
            Damage::None => unchanged += 1,
            Damage::Full => whole += 1,
            Damage::Rects(_) => partial += 1,
        }
        if !kept.last_shifts.is_empty() {
            moved += 1;
        }
        let pm = kept.pixmap().expect("a kept frame");
        if pm.data() != full.data() && std::env::var_os("GUI_FUZZ_DEBUG").is_some() {
            eprintln!("shifts {:?} scrolls {:?}", kept.last_shifts, dl.scrolls);
            for (i, p) in dl.prims.iter().enumerate() {
                let txt = format!("{p:?}");
                eprintln!("{i}: {}", &txt[..txt.len().min(400)]);
            }
        }
        assert!(
            pm.data() == full.data(),
            "seed {seed} step {step} at {scale}x: the software frame drawn over its damage ({d:?}) is not the whole frame: {}",
            first_difference(pm.data(), full.data(), dl.width)
        );
        if let (Some(g), Some(t)) = (&g, target.as_mut()) {
            let gd = t.render(&dl);
            if gd == Damage::Full {
                gpu_moved = false;
            }
            gpu_moved |= !t.last_shifts.is_empty();
            let damaged = t.rgba();
            let whole_gpu = g.render_rgba(&dl);
            assert!(
                if gpu_moved {
                    within(&damaged, &whole_gpu, MOVED_TOLERANCE)
                } else {
                    damaged == whole_gpu
                },
                "seed {seed} step {step} at {scale}x: the GPU frame drawn over its damage ({d:?}, moved: {gpu_moved}) is not the whole frame: {}",
                first_difference(&damaged, &whole_gpu, dl.width)
            );
            if let Err(e) = gpu_agrees(&soft::rgba(&full), &whole_gpu) {
                if let Some(dir) = std::env::var_os("GUI_FUZZ_DEBUG") {
                    // both frames as PNGs, and the primitives, to look at
                    let dir = std::path::PathBuf::from(dir);
                    let _ = full.save_png(dir.join("soft.png"));
                    if let Some(sz) = tiny_skia::IntSize::from_wh(dl.width, dl.height) {
                        let mut pre = whole_gpu.clone();
                        for px in pre.chunks_mut(4) {
                            let a = px[3] as u32;
                            for c in &mut px[..3] {
                                *c = ((*c as u32 * a + 127) / 255) as u8;
                            }
                        }
                        if let Some(pm) = tiny_skia::Pixmap::from_vec(pre, sz) {
                            let _ = pm.save_png(dir.join("gpu.png"));
                        }
                    }
                    let a = soft::rgba(&full);
                    let mut worst = (0u8, 0usize);
                    for (i, (x, y)) in a.chunks(4).zip(whole_gpu.chunks(4)).enumerate() {
                        let d = (0..4).map(|k| x[k].abs_diff(y[k])).max().unwrap_or(0);
                        if d > worst.0 {
                            worst = (d, i);
                        }
                    }
                    eprintln!(
                        "worst at ({}, {}) by {}",
                        worst.1 % dl.width as usize,
                        worst.1 / dl.width as usize,
                        worst.0
                    );
                    for (i, p) in dl.prims.iter().enumerate() {
                        let txt = format!("{p:?}");
                        eprintln!("{i}: {}", &txt[..txt.len().min(300)]);
                    }
                }
                panic!(
                    "seed {seed} step {step} at {scale}x: the GPU and software frames disagree: {e}"
                );
            }
        }
    }
    eprintln!(
        "seed {seed} at {scale}x: {partial} drawn in part, {unchanged} unchanged, {whole} whole, {moved} moved"
    );
    (partial, unchanged, whole)
}

#[test]
fn a_damaged_frame_is_the_whole_frame_in_software() {
    for (i, scale) in [1.0f32, 2.0, 1.5].into_iter().enumerate() {
        let (partial, unchanged, _) = fuzz(seed(0x5eed) + i as u64, steps(60), scale, false);
        assert!(partial > 0, "some frames are drawn in part");
        assert!(unchanged > 0, "some frames change nothing");
    }
}

#[test]
fn a_damaged_frame_is_the_whole_frame_on_the_gpu_and_agrees_with_software() {
    let _turn = gpu_turn();
    if gpu::gpu(None, None).is_err() {
        eprintln!("skipped: no GPU here");
        return;
    }
    for (i, scale) in [1.0f32, 2.0].into_iter().enumerate() {
        fuzz(seed(0xd1ff) + i as u64, steps(40), scale, true);
    }
}

#[test]
fn an_unchanged_frame_draws_nothing_and_a_caret_move_draws_a_tile_or_two() {
    let mut r = Rng(7);
    let mut world = World::new(&mut r, 2.0);
    let mut kept = soft::Retained::default();
    let dl = world.st.display_list();
    assert_eq!(*kept.render(&dl), Damage::Full);
    let dl = world.st.display_list();
    assert_eq!(
        *kept.render(&dl),
        Damage::None,
        "the same frame again draws nothing"
    );
    // the caret moves one character
    let mut out = Vec::new();
    world.st.input(
        Input::Key {
            key: "right".into(),
            text: None,
            mods: Mods::default(),
            repeat: false,
        },
        &mut NoClip,
        &mut out,
    );
    let dl = world.st.display_list();
    let d = kept.render(&dl).clone();
    let area = d.area(dl.width, dl.height);
    assert!(
        matches!(d, Damage::Rects(_)) && area * 20 < (dl.width * dl.height) as i64,
        "a caret's move redraws a small part of the window, not {d:?}"
    );
}

#[test]
fn the_glyph_atlas_takes_more_pages_and_evicts_the_least_recently_drawn() {
    let _turn = gpu_turn();
    let Ok(g) = gpu::gpu(None, None) else {
        eprintln!("skipped: no GPU here");
        return;
    };
    // frames of large text, each in many sizes: more glyphs than a page
    let mut target = gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm);
    for round in 0..30 {
        let mut st = WinState::new(9, true, "atlas", 900.0, 700.0, 1.0);
        let mut ops = vec![m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 900.0, 700.0])),
            ("style", m(vec![("bg", s("#ffffff"))])),
        ])];
        for i in 0..6 {
            ops.push(m(vec![
                ("key", s(&format!("t{i}"))),
                ("parent", s("root")),
                ("role", s("text")),
                ("box", tup(&[0.0, i as f64 * 110.0, 900.0, 110.0])),
                (
                    "text",
                    s("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"),
                ),
                (
                    "style",
                    m(vec![
                        ("size", Value::Float(30.0 + (round * 6 + i) as f64 * 0.21)),
                        ("wrap", Value::Boolean(true)),
                    ]),
                ),
            ]));
        }
        let mut out = Vec::new();
        st.apply(&Value::List(Arc::new(ops)), &mut out)
            .expect("applies");
        let dl = st.display_list();
        target.render(&dl);
        // whatever the atlas went through, the frame is the reference's
        let damaged = target.rgba();
        let whole = g.render_rgba(&dl);
        assert!(
            damaged == whole,
            "round {round}: the frame is drawn whole and right"
        );
        let soft_px = soft::rgba(&soft::render(&dl));
        gpu_agrees(&soft_px, &whole).unwrap_or_else(|e| panic!("round {round}: {e}"));
    }
    let stats = g.stats();
    assert!(
        stats.atlas_pages >= 2,
        "the atlas grew past a page: {stats:?}"
    );
    assert!(stats.atlas_pages <= 4, "and stays bounded: {stats:?}");
}

#[test]
fn the_renderers_frames_and_memory_are_read_with_any_first_argument() {
    let program = olang::Parser::new()
        .parse(
            r#"let r = gui.read((), "render")
let w = gui.headless(#{ "size": (100, 80) })
let r2 = gui.read(w, "render")
[r.frames_drawn >= 0, r.texture_bytes >= 0, r.atlas_pages != (), r2.raster_glyphs != ()]"#,
        )
        .expect("parses");
    let v = olang::Interpreter::new()
        .eval_program(program)
        .expect("runs");
    assert_eq!(format!("{v}"), "[true, true, true, true]");
}

/// A long list in a scroll region, a sidebar beside it.
fn scrolled(scale: f32) -> WinState {
    let mut st = WinState::new(3, true, "scroll", 600.0, 400.0, scale);
    let mut ops = vec![
        m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 600.0, 400.0])),
            ("style", m(vec![("bg", s("#fafafa"))])),
        ]),
        m(vec![
            ("key", s("side")),
            ("parent", s("root")),
            ("role", s("text")),
            ("box", tup(&[0.0, 0.0, 150.0, 400.0])),
            ("text", s("Sidebar")),
            ("style", m(vec![("bg", s("#eeeef0"))])),
        ]),
        m(vec![
            ("key", s("list")),
            ("parent", s("root")),
            ("role", s("region")),
            ("box", tup(&[160.0, 0.0, 440.0, 400.0])),
            ("content", tup(&[440.0, 3000.0])),
            ("style", m(vec![("bg", s("#ffffff"))])),
        ]),
    ];
    for i in 0..100 {
        ops.push(m(vec![
            ("key", s(&format!("row{i}"))),
            ("parent", s("list")),
            ("role", s("text")),
            ("box", tup(&[8.0, i as f64 * 30.0, 420.0, 28.0])),
            ("text", s(&format!("Row {i}: some text in a scrolled list"))),
            (
                "style",
                m(vec![
                    ("bg", s(if i % 2 == 0 { "#f4f4f5" } else { "#ffffff" })),
                    ("radius", Value::Integer(4)),
                ]),
            ),
        ]));
    }
    let mut out = Vec::new();
    st.apply(&Value::List(Arc::new(ops)), &mut out)
        .expect("applies");
    st
}

#[test]
fn a_scroll_moves_the_kept_frame_and_draws_only_what_it_uncovered() {
    let _turn = gpu_turn();
    for scale in [1.0f32, 2.0] {
        let mut st = scrolled(scale);
        let mut kept = soft::Retained::default();
        let g = gpu::gpu(None, None).ok();
        let mut target = g
            .as_ref()
            .map(|g| gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm));
        let mut moved = 0;
        for (i, to) in [0.0, 40.0, 100.0, 90.0, 400.0, 413.0, 0.0]
            .into_iter()
            .enumerate()
        {
            let mut out = Vec::new();
            st.apply(
                &Value::List(Arc::new(vec![m(vec![
                    ("op", s("scroll")),
                    ("key", s("list")),
                    ("to", tup(&[0.0, to])),
                ])])),
                &mut out,
            )
            .expect("scrolls");
            let dl = st.display_list();
            let d = kept.render(&dl).clone();
            let full = soft::render(&dl);
            let pm = kept.pixmap().expect("kept");
            assert!(
                pm.data() == full.data(),
                "at {scale}x, scrolled to {to}: the moved frame is the whole frame ({d:?}, {:?}): {}",
                kept.last_shifts,
                first_difference(pm.data(), full.data(), dl.width)
            );
            if i > 0 && !kept.last_shifts.is_empty() {
                moved += 1;
            }
            // a scroll by less than a fifth of the view
            if (1..=3).contains(&i) {
                assert!(
                    !kept.last_shifts.is_empty(),
                    "a scroll to {to} moves the kept frame"
                );
                assert!(
                    d.area(dl.width, dl.height) * 2 < (dl.width * dl.height) as i64,
                    "a scroll by {to} draws less than half the window: {d:?}"
                );
            }
            if let (Some(g), Some(t)) = (&g, target.as_mut()) {
                t.render(&dl);
                assert_eq!(
                    t.last_shifts, kept.last_shifts,
                    "both renderers move the same"
                );
                let damaged = t.rgba();
                let whole = g.render_rgba(&dl);
                assert!(
                    within(&damaged, &whole, MOVED_TOLERANCE),
                    "at {scale}x, scrolled to {to}: the GPU's moved frame is its whole frame: {}",
                    first_difference(&damaged, &whole, dl.width)
                );
            }
        }
        assert!(
            moved >= 3,
            "scrolls by whole pixels move the kept frame ({moved})"
        );
    }
}

/// Frame costs, damaged against whole, offscreen (no display needed):
/// `cargo test --profile fasttest --test gui_damage_test measure -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_damaged_against_whole_frames() {
    let _turn = gpu_turn();
    let g = gpu::gpu(None, None).ok();
    let (w, h) = (1200.0, 800.0);
    let base = |st: &mut WinState, extra: Vec<Value>| {
        let mut ops = vec![m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, w, h])),
            ("style", m(vec![("bg", s("#fafafa"))])),
        ])];
        ops.push(m(vec![
            ("key", s("side")),
            ("parent", s("root")),
            ("box", tup(&[0.0, 0.0, 260.0, h])),
            ("style", m(vec![("bg", s("#f1f1f3"))])),
        ]));
        for i in 0..30 {
            ops.push(m(vec![
                ("key", s(&format!("s{i}"))),
                ("parent", s("side")),
                ("role", s("text")),
                ("box", tup(&[12.0, 12.0 + i as f64 * 24.0, 236.0, 22.0])),
                ("text", s(&format!("Sidebar item {i}"))),
            ]));
        }
        ops.extend(extra);
        let mut out = Vec::new();
        st.apply(&Value::List(Arc::new(ops)), &mut out)
            .expect("applies");
        st.input(Input::Focused(true), &mut NoClip, &mut out);
    };
    type Step = Box<dyn Fn(&mut WinState, usize)>;
    let mut scenarios: Vec<(&str, Vec<Value>, Step)> = Vec::new();
    // typing in 50,000 lines
    let text: Vec<String> = (0..50_000)
        .map(|i| format!("let value_{i} = compute({i}, \"some text here\")  // line {i}"))
        .collect();
    scenarios.push((
        "typing",
        vec![
            m(vec![
                ("key", s("ed")),
                ("parent", s("root")),
                ("role", s("input")),
                ("box", tup(&[270.0, 10.0, w - 280.0, h - 20.0])),
                (
                    "edit",
                    m(vec![
                        ("value", s(&text.join("\n"))),
                        ("multiline", Value::Boolean(true)),
                        ("rich", Value::Boolean(true)),
                        ("wrap", Value::Boolean(false)),
                    ]),
                ),
                (
                    "style",
                    m(vec![
                        ("bg", s("#ffffff")),
                        ("font", s("mono")),
                        ("size", Value::Integer(13)),
                    ]),
                ),
            ]),
            m(vec![("op", s("focus")), ("key", s("ed"))]),
        ],
        Box::new(|st, _| {
            st.input(
                Input::Key {
                    key: "x".into(),
                    text: Some("x".into()),
                    mods: Mods::default(),
                    repeat: false,
                },
                &mut NoClip,
                &mut Vec::new(),
            )
        }),
    ));
    // a caret's blink
    scenarios.push((
        "blink",
        vec![
            m(vec![
                ("key", s("doc")),
                ("parent", s("root")),
                ("role", s("text")),
                ("box", tup(&[280.0, 20.0, 880.0, 700.0])),
                (
                    "text",
                    s(&(0..40)
                        .map(|i| format!("A paragraph of text, line {i}"))
                        .collect::<Vec<_>>()
                        .join("\n")),
                ),
                ("style", m(vec![("wrap", Value::Boolean(true))])),
            ]),
            m(vec![
                ("key", s("caret")),
                ("parent", s("root")),
                ("box", tup(&[400.0, 60.0, 2.0, 18.0])),
                ("style", m(vec![("bg", s("#2563eb"))])),
            ]),
        ],
        Box::new(|st, i| {
            let bg = if i % 2 == 0 { "#ffffff" } else { "#2563eb" };
            let _ = st.apply(
                &Value::List(Arc::new(vec![m(vec![
                    ("key", s("caret")),
                    ("parent", s("root")),
                    ("box", tup(&[400.0, 60.0, 2.0, 18.0])),
                    ("style", m(vec![("bg", s(bg))])),
                ])])),
                &mut Vec::new(),
            );
        }),
    ));
    // hover over a list
    scenarios.push((
        "hover",
        (0..30)
            .map(|i| {
                m(vec![
                    ("key", s(&format!("r{i}"))),
                    ("parent", s("root")),
                    ("role", s("listitem")),
                    ("box", tup(&[280.0, 10.0 + i as f64 * 26.0, 600.0, 24.0])),
                    (
                        "text",
                        s(&format!("Row {i} — a list row with some text in it")),
                    ),
                    ("focusable", Value::Boolean(false)),
                    (
                        "style",
                        m(vec![
                            ("bg", s("#ffffff")),
                            ("hover", m(vec![("bg", s("#e0e7ff"))])),
                        ]),
                    ),
                ])
            })
            .collect(),
        Box::new(|st, i| {
            st.input(
                Input::Pointer {
                    action: PointerAction::Move,
                    x: 400.0,
                    y: 12.0 + (i % 30) as f32 * 26.0,
                    button: "left".into(),
                    clicks: 0,
                },
                &mut NoClip,
                &mut Vec::new(),
            )
        }),
    ));
    // a notebook-like scroll
    let mut cells = vec![m(vec![
        ("key", s("nb")),
        ("parent", s("root")),
        ("role", s("region")),
        ("box", tup(&[270.0, 0.0, 920.0, h])),
        ("content", tup(&[900.0, 40.0 * 180.0 + 20.0])),
        ("style", m(vec![("clip", Value::Boolean(true))])),
    ])];
    for i in 0..40 {
        let c = format!("c{i}");
        cells.push(m(vec![
            ("key", s(&c)),
            ("parent", s("nb")),
            ("box", tup(&[20.0, i as f64 * 180.0 + 10.0, 860.0, 170.0])),
            (
                "style",
                m(vec![
                    ("bg", s("#ffffff")),
                    ("border", s("#e4e4e7")),
                    ("border_width", Value::Integer(1)),
                    ("radius", Value::Integer(8)),
                ]),
            ),
        ]));
        cells.push(m(vec![
            ("key", s(&format!("ct{i}"))),
            ("parent", s(&c)),
            ("role", s("text")),
            ("box", tup(&[12.0, 10.0, 836.0, 60.0])),
            (
                "text",
                s(&format!(
                    "let df = read_csv(\"sales.csv\")\nlet by = group_by(df, \"region\")\n{i}"
                )),
            ),
            (
                "style",
                m(vec![("font", s("mono")), ("size", Value::Integer(13))]),
            ),
        ]));
        let draw: Vec<Value> = (0..12)
            .map(|j| {
                let hh = ((j * 7 + i * 3) % 80) as f64;
                m(vec![
                    ("op", s("rect")),
                    ("x", Value::Float(j as f64 * 32.0)),
                    ("y", Value::Float(84.0 - hh)),
                    ("w", Value::Float(24.0)),
                    ("h", Value::Float(hh)),
                    ("fill", s("#60a5fa")),
                ])
            })
            .collect();
        cells.push(m(vec![
            ("key", s(&format!("cv{i}"))),
            ("parent", s(&c)),
            ("role", s("figure")),
            ("box", tup(&[12.0, 76.0, 400.0, 84.0])),
            ("draw", Value::List(Arc::new(draw))),
        ]));
    }
    scenarios.push((
        "scroll",
        cells,
        Box::new(|st, i| {
            let _ = st.apply(
                &Value::List(Arc::new(vec![m(vec![
                    ("op", s("scroll")),
                    ("key", s("nb")),
                    ("to", tup(&[0.0, 24.0 * (i + 1) as f64])),
                ])])),
                &mut Vec::new(),
            );
        }),
    ));
    // a terminal flood
    scenarios.push((
        "flood",
        vec![m(vec![
            ("key", s("term")),
            ("parent", s("root")),
            ("box", tup(&[270.0, 0.0, 930.0, h])),
            ("style", m(vec![("bg", s("#111111"))])),
        ])],
        Box::new(|st, f| {
            let ops: Vec<Value> = (0..45)
                .map(|r| {
                    m(vec![
                        ("key", s(&format!("t{r}"))),
                        ("parent", s("term")),
                        ("role", s("text")),
                        ("box", tup(&[8.0, 4.0 + r as f64 * 17.0, 910.0, 17.0])),
                        (
                            "text",
                            s(&format!(
                                "[{}] building target/release/deps/olang-{}.o",
                                f + r,
                                (f + r) * 7919 % 100000
                            )),
                        ),
                        (
                            "style",
                            m(vec![
                                ("font", s("mono")),
                                ("size", Value::Integer(12)),
                                ("color", s("#d4d4d4")),
                            ]),
                        ),
                    ])
                })
                .collect();
            let _ = st.apply(&Value::List(Arc::new(ops)), &mut Vec::new());
        }),
    ));
    let p50 = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    for (name, extra, step) in scenarios {
        let mut st = WinState::new(5, true, "measure", w as f32, h as f32, 2.0);
        base(&mut st, extra);
        let mut kept = soft::Retained::default();
        let mut t = g
            .as_ref()
            .map(|g| gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm));
        let mut whole_t = g
            .as_ref()
            .map(|g| gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm));
        let (mut list, mut sd, mut sf, mut gd, mut gf, mut area) =
            (vec![], vec![], vec![], vec![], vec![], vec![]);
        for i in 0..40 {
            if i > 0 {
                step(&mut st, i);
            }
            let t0 = std::time::Instant::now();
            let dl = st.display_list();
            list.push(t0.elapsed().as_secs_f64() * 1e3);
            let t0 = std::time::Instant::now();
            let d = kept.render(&dl).clone();
            sd.push(t0.elapsed().as_secs_f64() * 1e3);
            let t0 = std::time::Instant::now();
            let _ = soft::render(&dl);
            sf.push(t0.elapsed().as_secs_f64() * 1e3);
            area.push(100.0 * d.area(dl.width, dl.height) as f64 / (dl.width * dl.height) as f64);
            if let (Some(g), Some(t), Some(wt)) = (&g, t.as_mut(), whole_t.as_mut()) {
                let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
                // in turn first, so neither gains from the other's warmth
                for k in 0..2 {
                    if (i + k) % 2 == 0 {
                        let t0 = std::time::Instant::now();
                        t.render(&dl);
                        let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
                        gd.push(t0.elapsed().as_secs_f64() * 1e3);
                    } else {
                        wt.invalidate();
                        let t0 = std::time::Instant::now();
                        wt.render(&dl);
                        let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
                        gf.push(t0.elapsed().as_secs_f64() * 1e3);
                    }
                }
            }
        }
        let skip = |v: Vec<f64>| v.into_iter().skip(2).collect::<Vec<_>>();
        eprintln!(
            "{name:7} list {:.2} ms | software whole {:.2} damaged {:.2} | gpu whole {:.2} damaged {:.2} | drew {:.1}%",
            p50(skip(list)),
            p50(skip(sf)),
            p50(skip(sd)),
            if gf.is_empty() { 0.0 } else { p50(skip(gf)) },
            if gd.is_empty() { 0.0 } else { p50(skip(gd)) },
            p50(skip(area)),
        );
    }
}

#[test]
fn a_code_editor_scrolled_is_drawn_whole_moved_or_not() {
    let _turn = gpu_turn();
    for scale in [1.0f32, 2.0] {
        let mut st = WinState::new(4, true, "editor", 500.0, 300.0, scale);
        let text: Vec<String> = (0..400)
            .map(|i| format!("let line_{i} = compute({i}) // a comment"))
            .collect();
        let ops = vec![
            m(vec![
                ("key", s("root")),
                ("box", tup(&[0.0, 0.0, 500.0, 300.0])),
                ("style", m(vec![("bg", s("#ffffff"))])),
            ]),
            m(vec![
                ("key", s("ed")),
                ("parent", s("root")),
                ("role", s("input")),
                ("box", tup(&[10.0, 10.0, 480.0, 280.0])),
                (
                    "edit",
                    m(vec![
                        ("value", s(&text.join("\n"))),
                        ("multiline", Value::Boolean(true)),
                        ("rich", Value::Boolean(true)),
                        ("wrap", Value::Boolean(false)),
                    ]),
                ),
                (
                    "style",
                    m(vec![("font", s("mono")), ("size", Value::Integer(13))]),
                ),
            ]),
            m(vec![("op", s("focus")), ("key", s("ed"))]),
        ];
        let mut out = Vec::new();
        st.apply(&Value::List(Arc::new(ops)), &mut out)
            .expect("applies");
        st.input(Input::Focused(true), &mut NoClip, &mut out);
        let mut kept = soft::Retained::default();
        let g = gpu::gpu(None, None).ok();
        let mut target = g
            .as_ref()
            .map(|g| gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm));
        let mut moved = 0;
        for i in 0..50 {
            if i > 0 {
                let input = if i % 10 < 7 {
                    Input::Key {
                        key: "down".into(),
                        text: None,
                        mods: Mods::default(),
                        repeat: false,
                    }
                } else {
                    Input::Wheel {
                        dx: 0.0,
                        dy: -20.0,
                        mods: None,
                    }
                };
                st.input(input, &mut NoClip, &mut out);
            }
            let dl = st.display_list();
            kept.render(&dl);
            if std::env::var_os("GUI_FUZZ_DEBUG").is_some() {
                eprintln!(
                    "{i}: {:?} {:?} {:?}",
                    dl.scrolls, kept.last_shifts, kept.last
                );
            }
            if !kept.last_shifts.is_empty() {
                moved += 1;
            }
            let full = soft::render(&dl);
            let pm = kept.pixmap().expect("kept");
            assert!(
                pm.data() == full.data(),
                "at {scale}x, step {i}: the editor's moved frame is its whole frame: {}",
                first_difference(pm.data(), full.data(), dl.width)
            );
            if let (Some(g), Some(t)) = (&g, target.as_mut()) {
                t.render(&dl);
                let (a, b) = (t.rgba(), g.render_rgba(&dl));
                assert!(
                    within(&a, &b, MOVED_TOLERANCE),
                    "at {scale}x, step {i}: the GPU's: {}",
                    first_difference(&a, &b, dl.width)
                );
            }
        }
        // its rows are a fractional height apart (15.13 px at 13 px), so
        // its scrolls are seldom by whole pixels: a move is taken only when
        // one is
        eprintln!("the editor at {scale}x: {moved} of 49 frames moved");
    }
}

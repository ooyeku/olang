//! Scroll physics (loom/ROADMAP.md item 18): a trackpad's gesture pulls a
//! region past its ends against a rubber band and springs back as the
//! fingers lift; its momentum bounces at an end and is spent there; a
//! mouse wheel's notch glides; a scroll with no phase moves at once, as
//! before. The program hears offsets inside the region only, and a
//! stretched frame drawn over its damage is the whole frame.

#![cfg(feature = "gui")]

use olang::Value;
use olang::stdlib::gui::scroll::{GLIDE_MS, Phase, rubber};
use olang::stdlib::gui::soft;
use olang::stdlib::gui::window::{Clipboard, Input, PointerAction, WinState};
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

struct NoClip;
impl Clipboard for NoClip {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _: String) {}
}

/// A 300×200 window: a region `list` (0, 0, 300, 200) of 40 rows 30 high,
/// the pointer over it.
fn window(scale: f32) -> WinState {
    let mut st = WinState::new(1, true, "scroll", 300.0, 200.0, scale);
    let mut ops = vec![
        m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 300.0, 200.0])),
            ("style", m(vec![("bg", s("#ffffff"))])),
        ]),
        m(vec![
            ("key", s("list")),
            ("parent", s("root")),
            ("role", s("region")),
            ("scroll", Value::Boolean(true)),
            ("box", tup(&[0.0, 0.0, 300.0, 200.0])),
            (
                "style",
                m(vec![("bg", s("#f4f4f5")), ("clip", Value::Boolean(true))]),
            ),
        ]),
    ];
    for i in 0..40 {
        ops.push(m(vec![
            ("key", s(&format!("r{i}"))),
            ("parent", s("list")),
            ("role", s("text")),
            ("box", tup(&[0.0, i as f64 * 30.0, 300.0, 30.0])),
            ("text", s(&format!("row {i}"))),
            (
                "style",
                m(vec![(
                    "bg",
                    s(if i % 2 == 0 { "#dbeafe" } else { "#fef3c7" }),
                )]),
            ),
        ]));
    }
    let mut out = Vec::new();
    st.apply(&Value::List(Arc::new(ops)), &mut out)
        .expect("applies");
    st.set_clock(0.0);
    input(
        &mut st,
        Input::Pointer {
            action: PointerAction::Move,
            x: 150.0,
            y: 100.0,
            button: "left".into(),
            clicks: 0,
        },
    );
    st.display_list();
    st
}

fn input(st: &mut WinState, i: Input) -> Vec<Value> {
    let mut out = Vec::new();
    st.input(i, &mut NoClip, &mut out);
    out
}

fn scroll(st: &mut WinState, at: f64, dy: f32, phase: Phase) -> Vec<Value> {
    st.set_clock(at);
    input(
        st,
        Input::Scroll {
            dx: 0.0,
            dy,
            mods: None,
            phase,
            precise: true,
        },
    )
}

fn scrolled(evs: &[Value]) -> Vec<f64> {
    evs.iter()
        .filter_map(|e| match e {
            Value::Map(f) if matches!(f.get("kind"), Some(Value::String(k)) if k.as_str() == "scrolled") => match f.get("y") {
                Some(Value::Float(y)) => Some(*y),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn offset(st: &WinState) -> f32 {
    st.scene.nodes["list"].offset.1
}

fn stretch(st: &WinState) -> f32 {
    st.stretch_of("list", [300.0, 200.0])[1]
}

#[test]
fn a_pull_past_the_top_stretches_against_the_band_and_springs_back_as_the_fingers_lift() {
    let mut st = window(1.0);
    let mut evs = scroll(&mut st, 10.0, 0.0, Phase::Began);
    for i in 0..10 {
        evs.extend(scroll(&mut st, 20.0 + i as f64 * 8.0, 20.0, Phase::Changed));
    }
    assert_eq!(offset(&st), 0.0, "the program's offset stays at the top");
    assert!(scrolled(&evs).is_empty(), "a stretch is not said: {evs:?}");
    let shown = stretch(&st);
    assert!(
        (shown - rubber(200.0, 200.0)).abs() < 0.01 && shown < 200.0,
        "{shown}"
    );
    // drawn moved: the first row's top at the stretch
    let dl = st.display_list();
    let pm = soft::rgba(&soft::render(&dl));
    let at = |y: u32| pm[((y * dl.width + 150) * 4) as usize..][..3].to_vec();
    assert_eq!(
        at(shown as u32 / 2),
        vec![0xf4, 0xf4, 0xf5],
        "the region's own ground above the rows"
    );
    assert_eq!(at(shown as u32 + 5), vec![0xdb, 0xea, 0xfe]);
    // back towards the content: the pull gives first
    scroll(&mut st, 120.0, -10.0, Phase::Changed);
    assert_eq!(offset(&st), 0.0);
    assert!(stretch(&st) < shown);
    // let go: back to rest, never past it, then still
    scroll(&mut st, 130.0, 0.0, Phase::Ended);
    let mut last = stretch(&st);
    assert!(
        st.next_frame.is_some() || {
            st.display_list();
            st.next_frame.is_some()
        }
    );
    for ms in (138..1200).step_by(8) {
        st.set_clock(ms as f64);
        let now = stretch(&st);
        assert!(
            now <= last + 1e-3 && now >= -1e-3,
            "{ms}: {now} after {last}"
        );
        last = now;
    }
    assert_eq!(stretch(&st), 0.0);
    st.display_list();
    assert_eq!(st.next_frame, None, "settled: no more frames");
    // the momentum after a stretch let go is spent
    let evs = scroll(&mut st, 1210.0, -40.0, Phase::MomentumBegan);
    assert!(scrolled(&evs).is_empty() && offset(&st) == 0.0);
    scroll(&mut st, 1220.0, 0.0, Phase::MomentumEnded);
}

#[test]
fn momentum_runs_to_the_end_bounces_there_and_the_rest_is_spent() {
    let mut st = window(1.0);
    scroll(&mut st, 0.0, 0.0, Phase::Began);
    scroll(&mut st, 8.0, -300.0, Phase::Changed);
    scroll(&mut st, 16.0, 0.0, Phase::Ended);
    assert_eq!(offset(&st), 300.0);
    // momentum: 30 px a 8 ms (3750 px/s) towards the end (max 1000)
    let mut t = 24.0;
    let mut said = Vec::new();
    said.extend(scrolled(&scroll(&mut st, t, -30.0, Phase::MomentumBegan)));
    while offset(&st) < 1000.0 {
        t += 8.0;
        said.extend(scrolled(&scroll(&mut st, t, -30.0, Phase::Momentum)));
        assert!(t < 1000.0);
    }
    assert_eq!(*said.last().unwrap(), 1000.0, "the end is said");
    // out past the end, then back
    let mut peak = 0.0f32;
    for i in 1..40 {
        let at = t + i as f64 * 8.0;
        // the rest of the momentum keeps coming, and is spent
        let evs = scroll(&mut st, at, -20.0, Phase::Momentum);
        assert!(scrolled(&evs).is_empty());
        peak = peak.min(stretch(&st));
    }
    assert!(peak < -15.0 && peak > -150.0, "a bounce of {peak} px");
    scroll(&mut st, t + 400.0, 0.0, Phase::MomentumEnded);
    st.set_clock(t + 1500.0);
    assert!(stretch(&st).abs() < 0.11);
    assert_eq!(offset(&st), 1000.0);
}

#[test]
fn a_mouse_wheel_s_notch_glides_and_notches_in_a_row_carry_it_on() {
    let mut st = window(1.0);
    st.set_clock(100.0);
    let evs = input(
        &mut st,
        Input::Scroll {
            dx: 0.0,
            dy: -48.0,
            mods: None,
            phase: Phase::Legacy,
            precise: false,
        },
    );
    assert!(
        scrolled(&evs).is_empty() && offset(&st) == 0.0,
        "not a jump"
    );
    let tick = |st: &mut WinState, ms: f64| -> Vec<f64> {
        st.set_clock(ms);
        let mut out = Vec::new();
        st.tick(&mut out);
        st.display_list();
        scrolled(&out)
    };
    let mid = tick(&mut st, 100.0 + GLIDE_MS as f64 / 2.0);
    assert!(mid.len() == 1 && mid[0] > 24.0 && mid[0] < 48.0, "{mid:?}");
    assert!(st.next_frame.is_some());
    // a second notch while it glides: on to 96
    let evs = input(
        &mut st,
        Input::Scroll {
            dx: 0.0,
            dy: -48.0,
            mods: None,
            phase: Phase::Legacy,
            precise: false,
        },
    );
    assert!(scrolled(&evs).is_empty());
    let mut last = offset(&st);
    let start = 100.0 + GLIDE_MS as f64 / 2.0;
    for ms in (0..=(GLIDE_MS as i64 + 16)).step_by(8) {
        tick(&mut st, start + ms as f64);
        assert!(offset(&st) >= last);
        last = offset(&st);
    }
    assert_eq!(offset(&st), 96.0);
    assert!(tick(&mut st, start + 400.0).is_empty());
    assert!(
        st.elastic.is_empty(),
        "arrived: let go (only the bar's fade is left to draw)"
    );
    // past the end it stops there
    for _ in 0..40 {
        input(
            &mut st,
            Input::Scroll {
                dx: 0.0,
                dy: -48.0,
                mods: None,
                phase: Phase::Legacy,
                precise: false,
            },
        );
    }
    tick(&mut st, start + 2000.0);
    assert_eq!(offset(&st), 1000.0);
}

#[test]
fn a_scroll_with_no_phase_moves_at_once_as_before() {
    let mut st = window(1.0);
    let evs = input(
        &mut st,
        Input::Wheel {
            dx: 0.0,
            dy: -40.0,
            mods: None,
        },
    );
    assert_eq!(scrolled(&evs), vec![40.0]);
    let evs = scroll(&mut st, 5.0, -40.0, Phase::Legacy);
    assert_eq!(scrolled(&evs), vec![80.0]);
    assert!(st.elastic.is_empty());
}

#[test]
fn an_inner_region_at_its_end_stretches_itself_and_an_axis_it_does_not_scroll_goes_out() {
    let mut st = WinState::new(1, true, "nested", 300.0, 200.0, 1.0);
    let ops = vec![
        m(vec![
            ("key", s("root")),
            ("box", tup(&[0.0, 0.0, 300.0, 200.0])),
        ]),
        m(vec![
            ("key", s("page")),
            ("parent", s("root")),
            ("scroll", Value::Boolean(true)),
            ("box", tup(&[0.0, 0.0, 300.0, 200.0])),
            ("content", tup(&[600.0, 800.0])),
        ]),
        m(vec![
            ("key", s("inner")),
            ("parent", s("page")),
            ("scroll", Value::Boolean(true)),
            ("box", tup(&[0.0, 0.0, 300.0, 100.0])),
            ("content", tup(&[300.0, 400.0])),
        ]),
        m(vec![
            ("key", s("fill")),
            ("parent", s("inner")),
            ("box", tup(&[0.0, 0.0, 300.0, 400.0])),
        ]),
    ];
    let mut out = Vec::new();
    st.apply(&Value::List(Arc::new(ops)), &mut out)
        .expect("applies");
    input(
        &mut st,
        Input::Pointer {
            action: PointerAction::Move,
            x: 50.0,
            y: 50.0,
            button: "left".into(),
            clicks: 0,
        },
    );
    st.display_list();
    st.set_clock(0.0);
    input(
        &mut st,
        Input::Scroll {
            dx: 0.0,
            dy: 0.0,
            mods: None,
            phase: Phase::Began,
            precise: true,
        },
    );
    // up past the inner's top: the inner stretches, the page stays
    input(
        &mut st,
        Input::Scroll {
            dx: 0.0,
            dy: 30.0,
            mods: None,
            phase: Phase::Changed,
            precise: true,
        },
    );
    assert!(st.stretch_of("inner", [300.0, 100.0])[1] > 0.0);
    assert_eq!(st.scene.nodes["page"].offset, (0.0, 0.0));
    // sideways: the inner does not scroll across, the page does
    input(
        &mut st,
        Input::Scroll {
            dx: -50.0,
            dy: 0.0,
            mods: None,
            phase: Phase::Changed,
            precise: true,
        },
    );
    assert_eq!(st.scene.nodes["page"].offset.0, 50.0);
}

/// Stretched and springing frames drawn over their damage are the whole
/// frames (the region's kept frame moves with the stretch).
#[test]
fn a_stretched_frame_drawn_over_its_damage_is_the_whole_frame() {
    for scale in [1.0, 2.0] {
        let mut st = window(scale);
        let mut kept = soft::Retained::default();
        kept.render(&st.display_list());
        let mut t = 0.0;
        let mut steps: Vec<(f32, Phase)> = vec![(0.0, Phase::Began)];
        steps.extend((0..8).map(|_| (13.0, Phase::Changed)));
        steps.push((-7.5, Phase::Changed));
        steps.push((0.0, Phase::Ended));
        for (dy, ph) in steps {
            t += 8.0;
            scroll(&mut st, t, dy, ph);
            let dl = st.display_list();
            let whole = soft::render(&dl);
            kept.render(&dl);
            assert_eq!(
                kept.pixmap().unwrap().data(),
                whole.data(),
                "at {scale}x, {t} ms ({ph:?})"
            );
        }
        for _ in 0..60 {
            t += 8.0;
            st.set_clock(t);
            let dl = st.display_list();
            let whole = soft::render(&dl);
            kept.render(&dl);
            assert_eq!(
                kept.pixmap().unwrap().data(),
                whole.data(),
                "at {scale}x springing back, {t} ms"
            );
        }
    }
}

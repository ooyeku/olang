//! A chart of 10,000 points panned (loom/ROADMAP.md item 11's measure):
//! each step a frame's display list made and drawn on the GPU over the
//! kept frame, the GPU waited for. Ignored by the suite; run by hand:
//!
//!   cargo test --profile fasttest --test gui_chart_bench_test -- --ignored --nocapture

#![cfg(feature = "gui")]

use olang::Value;
use olang::stdlib::gui::gpu;
use olang::stdlib::gui::window::WinState;
use std::sync::Arc;
use std::time::Instant;

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

const N: usize = 10_000;

/// The chart panned by `dx` logical pixels: its line, the area under it,
/// a grid.
fn chart(dx: f64) -> Value {
    let pts: Vec<Value> = (0..N)
        .map(|i| {
            let x = i as f64 * 0.22 - dx;
            let y = 300.0 + 160.0 * (x * 0.013).sin() + 40.0 * (x * 0.31).sin() * (x * 0.007).cos();
            tup(&[x, y])
        })
        .collect();
    let mut area = pts.clone();
    area.push(tup(&[N as f64 * 0.22 - dx, 600.0]));
    area.push(tup(&[-dx, 600.0]));
    let mut ops = vec![m(vec![
        ("op", s("push")),
        ("clip", tup(&[0.0, 0.0, 1100.0, 600.0])),
    ])];
    for k in 0..12 {
        ops.push(m(vec![
            ("op", s("line")),
            ("x1", Value::Float(0.0)),
            ("y1", Value::Float(k as f64 * 50.0)),
            ("x2", Value::Float(1100.0)),
            ("y2", Value::Float(k as f64 * 50.0)),
            ("color", s("#e4e4e7")),
            ("width", Value::Float(1.0)),
        ]));
    }
    ops.push(m(vec![
        ("op", s("path")),
        ("points", Value::List(Arc::new(area))),
        ("close", Value::Boolean(true)),
        ("fill", s("#2563eb22")),
    ]));
    ops.push(m(vec![
        ("op", s("path")),
        ("points", Value::List(Arc::new(pts))),
        ("stroke", s("#2563eb")),
        ("width", Value::Float(1.5)),
    ]));
    ops.push(m(vec![("op", s("pop"))]));
    Value::List(Arc::new(ops))
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

#[test]
#[ignore]
fn a_chart_of_ten_thousand_points_pans() {
    let g = match gpu::gpu(None, None) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped: no GPU here ({e})");
            return;
        }
    };
    let mut st = WinState::new(1, true, "chart", 1200.0, 700.0, 2.0);
    let mut out = Vec::new();
    let node = |dx: f64| {
        Value::List(Arc::new(vec![
            m(vec![
                ("key", s("root")),
                ("box", tup(&[0.0, 0.0, 1200.0, 700.0])),
                ("style", m(vec![("bg", s("#ffffff"))])),
            ]),
            m(vec![
                ("key", s("chart")),
                ("parent", s("root")),
                ("role", s("figure")),
                ("box", tup(&[50.0, 50.0, 1100.0, 600.0])),
                ("draw", chart(dx)),
            ]),
        ]))
    };
    st.apply(&node(0.0), &mut out).expect("applies");
    let mut target = gpu::Target::new(g.clone(), wgpu::TextureFormat::Rgba8Unorm);
    let (mut list, mut draw, mut total) = (Vec::new(), Vec::new(), Vec::new());
    for step in 0..80 {
        // the program's patch is not the engine's frame: made before the clock
        st.apply(&node(step as f64 * 3.7), &mut out)
            .expect("applies");
        let t0 = Instant::now();
        let dl = st.display_list();
        let t1 = Instant::now();
        target.render(&dl);
        let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
        let t2 = Instant::now();
        if step >= 10 {
            list.push((t1 - t0).as_secs_f64() * 1e3);
            draw.push((t2 - t1).as_secs_f64() * 1e3);
            total.push((t2 - t0).as_secs_f64() * 1e3);
        }
    }
    eprintln!(
        "chart pan, {N} points, 2400×1400: display list p50 {:.2} p90 {:.2} ms; GPU (prepare, encode, drawn) p50 {:.2} p90 {:.2} ms; frame p50 {:.2} p90 {:.2} ms",
        pct(&mut list, 0.5),
        pct(&mut list, 0.9),
        pct(&mut draw, 0.5),
        pct(&mut draw, 0.9),
        pct(&mut total, 0.5),
        pct(&mut total, 0.9)
    );
}

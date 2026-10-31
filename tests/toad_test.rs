//! `toad`: kernels compiled from olang functions, swarms stepped on the
//! CPU and the GPU. The kernel and the interpreter agree, what a GPU
//! cannot do is refused with the reason, spawns are deterministic, the
//! dead leave in order, and the GPU agrees with the CPU (when there is one).
#![cfg(feature = "toad")]

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn run(src: &str) -> Result<String, String> {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let program = Parser::new().parse(&src).map_err(|e| e.to_string())?;
            let mut i = Interpreter::new();
            i.eval_program(program)
                .map(|v| match v {
                    Value::String(ref s) => s.to_string(),
                    other => format!("{other:?}"),
                })
                .map_err(|e| e.to_string())
        })
        .expect("spawn")
        .join()
        .expect("join")
}

const DRIFT: &str = r#"
let PULL = 0.4
fn normalize(a) = { let l = math.sqrt(a[0] * a[0] + a[1] * a[1]); if l < 0.000001 => (0.0, 0.0) else => a / l }
fn length(a) = math.sqrt(a[0] * a[0] + a[1] * a[1])
fn drift(p, w) = {
    let to = w.home - p.pos
    let vel = p.vel * 0.98 + normalize(to) * min(length(to), 200.0) * PULL * w.dt
    #{ "pos": p.pos + vel * w.dt, "vel": vel, "dead": length(p.pos) > 450.0 }
}
let FIELDS = #{ "pos": (0.0, 0.0), "vel": (0.0, 0.0) }
let WORLD = #{ "home": (0.0, 0.0) }
fn ok(r) = match r { Ok(v) => v, Err(e) => raise(e) }
"#;

#[test]
fn a_kernel_steps_as_the_function_does() {
    let src = format!(
        "{DRIFT}
let p = #{{ \"pos\": (120.0, -40.0), \"vel\": (3.0, 1.0) }}
let w = #{{ \"home\": (10.0, 20.0), \"dt\": 0.01 }}
let want = drift(p, w)
let s = ok(toad.swarm(ok(toad.kernel(drift, FIELDS, WORLD)), 4, \"play\"))
toad.spawn(s, 1, p)
toad.step(s, w)
let got = toad.read(s, \"pos\")[0]
show(abs(got[0] - want.pos[0]) < 0.0005 && abs(got[1] - want.pos[1]) < 0.0005)"
    );
    assert_eq!(run(&src).as_deref(), Ok("true"));
}

fn refusal(kernel: &str) -> String {
    let src = format!(
        "{DRIFT}
{kernel}
match toad.kernel(k, FIELDS, WORLD) {{ Ok(_) => \"compiled\", Err(e) => e }}"
    );
    run(&src).expect("ran")
}

#[test]
fn what_a_gpu_cannot_do_is_refused_with_the_reason() {
    assert!(refusal(r#"fn k(p, w) = #{ "pos": p.pos, "x": "s" }"#).contains("a string"));
    assert!(refusal(r#"fn k(p, w) = #{ "pos": p.pos + p.speed }"#).contains("has no `speed` (it has pos, vel)"));
    assert!(refusal(r#"fn k(p, w) = #{ "pos": [1, 2] }"#).contains("a list"));
    assert!(refusal(r#"fn k(p, w) = #{ "vel": range(0, 2) |> fold((0.0, 0.0), (a, b) => a) }"#).contains("pipeline"));
    assert!(refusal(r#"fn k(p) = #{ "pos": p.pos }"#).contains("takes 1 parameter"));
    assert!(refusal(r#"fn k(p, w) = #{ "pos": p.pos, "glow": 1.0 }"#).contains("does not have"));
    assert!(refusal(r#"fn k(p, w) = #{ "pos": p.vel.x }"#).contains("a pair and is set to something else"));
    assert!(refusal(r#"let NAME = "x"
fn k(p, w) = #{ "pos": p.pos * NAME }"#).contains("`NAME` is String"));
    assert!(refusal(r#"fn k(p, w) = #{ "pos": if p.pos.x > 0.0 => p.pos }"#).contains("without an `else`"));
}

#[test]
fn constants_are_baked_and_folded() {
    // PULL from the top level, 2 * 3 folded: the program has no op for them
    let src = format!(
        "{DRIFT}
fn k(p, w) = #{{ \"vel\": p.vel * (2.0 * 3.0 * PULL) }}
let id = ok(toad.kernel(k, FIELDS, WORLD))
show(str.contains(toad.wgsl(id), \"2.4\") || str.contains(toad.wgsl(id), \"2.4000\")) + \" \" + show(toad.ops(id))"
    );
    let out = run(&src).expect("ran");
    // the vel columns, the constant, two products
    assert_eq!(out, "true 5", "{out}");
}

#[test]
fn spawns_from_one_seed_are_the_same() {
    let src = format!(
        "{DRIFT}
let id = ok(toad.kernel(drift, FIELDS, WORLD))
let a = ok(toad.swarm(id, 100, \"play\"))
let b = ok(toad.swarm(id, 100, \"play\"))
let gen = #{{ \"pos\": (\"disc\", (0.0, 0.0), 100.0), \"vel\": (\"cone\", (0.0, 1.0), 0.5), \"seed\": 9 }}
toad.spawn(a, 50, gen)
toad.spawn(b, 50, gen)
show(toad.read(a, \"pos\") == toad.read(b, \"pos\") && toad.read(a, \"vel\") == toad.read(b, \"vel\"))"
    );
    assert_eq!(run(&src).as_deref(), Ok("true"));
}

#[test]
fn the_dead_leave_in_order_and_are_counted() {
    let src = format!(
        "{DRIFT}
let s = ok(toad.swarm(ok(toad.kernel(drift, FIELDS, WORLD)), 10, \"play\"))
for x in [10.0, 500.0, 20.0, 600.0, 30.0] {{ toad.spawn(s, 1, #{{ \"pos\": (x, 0.0) }}) }}
let r = toad.step(s, #{{ \"home\": (0.0, 0.0), \"dt\": 0.0 }})
show(r.died) + \" \" + show(r.alive) + \" \" + show(toad.read(s, \"pos\") |> map((p) => p[0]))"
    );
    assert_eq!(run(&src).as_deref(), Ok("2 3 [10.0, 20.0, 30.0]"));
}

#[test]
fn the_gpu_agrees_with_the_cpu() {
    let src = format!(
        "{DRIFT}
fn wave(p, w) = {{
    let v = p.vel + (sin(w.t + p.pos.x * 0.01), cos(w.t * 1.3 + p.pos.y * 0.01)) * 20.0 * w.dt
    #{{ \"pos\": p.pos + v * w.dt, \"vel\": v * 0.99 }}
}}
match toad.gpu() {{
    Err(e) => \"no gpu\",
    Ok(_) => {{
        let id = ok(toad.kernel(wave, FIELDS, #{{ \"t\": 0.0 }}))
        let c = ok(toad.swarm(id, 500, \"play\"))
        let g = ok(toad.swarm(id, 500, \"show\"))
        let gen = #{{ \"pos\": (\"disc\", (0.0, 0.0), 300.0), \"vel\": (\"cone\", (0.0, 30.0), 3.1), \"seed\": 4 }}
        toad.spawn(c, 500, gen)
        toad.spawn(g, 500, gen)
        for i in range(0, 120) {{
            let w = #{{ \"t\": to_float(i) / 60.0, \"dt\": 1.0 / 60.0 }}
            toad.step(c, w)
            toad.step(g, w)
        }}
        let a = toad.read(c, \"pos\")
        let b = toad.read(g, \"pos\")
        let worst = range(0, 500) |> fold(0.0, (m, i) => max(m, max(abs(a[i][0] - b[i][0]), abs(a[i][1] - b[i][1]))))
        show(worst < 0.01)
    }}
}}"
    );
    let out = run(&src).expect("ran");
    assert!(out == "true" || out == "no gpu", "{out}");
}

#[test]
fn a_frame_draws_shapes_and_answers_its_pixels() {
    let src = r##"
match toad.gpu() {
    Err(e) => "no gpu",
    Ok(_) => {
        let f = toad.frame([#{ "shapes": [#{ "kind": "box", "at": (0.0, 0.0), "size": (40.0, 40.0), "color": "#ffffff" }] }],
                           #{ "size": (64, 64), "png": true })
        show(len(f.png) > 100)
    }
}"##;
    let out = run(src).expect("ran");
    assert!(out == "true" || out == "no gpu", "{out}");
}

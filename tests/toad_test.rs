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

#[test]
fn light_falls_off_and_a_blocker_casts_a_shadow() {
    // a light at the centre; a blocker to its right; probe the floor at
    // the same distance left (open) and right (behind the blocker), and
    // far away
    let src = r##"
match toad.gpu() {
    Err(e) => "no gpu",
    Ok(_) => {
        let floor = #{ "kind": "box", "at": (0.0, 0.0), "size": (2000.0, 2000.0), "color": "#808080" }
        let lamp = #{ "kind": "circle", "at": (0.0, 0.0), "size": 8.0, "color": "#ffffff", "emit": 20.0 }
        let wall = #{ "kind": "box", "at": (60.0, 0.0), "size": (16.0, 120.0), "color": "#808080", "cast": true }
        let f = toad.frame([#{ "shapes": [floor, lamp, wall] }],
            #{ "size": (512, 512), "scale": 1.0, "gi": 1.0, "probe": [(256 - 120, 256), (256 + 120, 256), (500, 500)] })
        let lum = (c) => c[0] + c[1] + c[2]
        let open = lum(f.probe[0])
        let behind = lum(f.probe[1])
        let far = lum(f.probe[2])
        show(open > behind + 30 && open > far) + " " + show(open) + " " + show(behind) + " " + show(far)
    }
}"##;
    let out = run(src).expect("ran");
    assert!(out.starts_with("true") || out == "no gpu", "{out}");
}

#[test]
fn an_unlit_frame_shows_colours_as_written() {
    let src = r##"
match toad.gpu() {
    Err(e) => "no gpu",
    Ok(_) => {
        let f = toad.frame([#{ "shapes": [#{ "kind": "box", "at": (0.0, 0.0), "size": (100.0, 100.0), "color": "#3080c0" }] }],
            #{ "size": (64, 64), "probe": [(32, 32)] })
        show(f.probe[0])
    }
}"##;
    let out = run(src).expect("ran");
    // AgX moves a colour a little; the hue and order of channels stay
    if out != "no gpu" {
        let nums: Vec<i64> = out.trim_matches(|c| c == '(' || c == ')').split(", ").map(|n| n.parse().unwrap()).collect();
        assert!(nums[2] > nums[1] && nums[1] > nums[0], "{out}");
    }
}


#[test]
fn a_material_is_shaded_from_where_the_light_is() {
    // a domed disc (relief) with a light to its right: its right side
    // brighter than its left; the same disc flat is even
    let src = r##"
match toad.gpu() {
    Err(e) => "no gpu",
    Ok(_) => {
        let floor = #{ "kind": "box", "at": (0.0, 0.0), "size": (2000.0, 2000.0), "color": "#202020" }
        let lamp = #{ "kind": "circle", "at": (200.0, 0.0), "size": 10.0, "color": "#ffffff", "emit": 30.0 }
        let dome = #{ "kind": "circle", "at": (0.0, 0.0), "size": 90.0, "color": "#808080", "relief": 80.0, "gloss": 0.4, "cast": true }
        let v = #{ "size": (512, 512), "scale": 1.0, "gi": 1.0, "probe": [(256 + 60, 256), (256 - 60, 256)] }
        let f = toad.frame([#{ "shapes": [floor, dome, lamp] }], v)
        let lum = (c) => c[0] + c[1] + c[2]
        show(lum(f.probe[0]) > lum(f.probe[1]) + 40) + " " + show(f.probe)
    }
}"##;
    let out = run(src).expect("ran");
    assert!(out.starts_with("true") || out == "no gpu", "{out}");
}

#[test]
fn a_pattern_is_named_and_a_shape_overrides_its_material() {
    let refused = run(r##"
match toad.gpu() {
    Err(e) => "pattern one of",
    Ok(_) => match attempt(() => toad.frame([#{ "shapes": [#{ "kind": "box", "pattern": "plaid" }] }], #{ "size": (16, 16) })) {
        Ok(_) => "drawn",
        Err(e) => show(e)
    }
}"##)
    .expect("ran");
    assert!(refused.contains("pattern") && refused.contains("one of"), "{refused}");
    // a checker material, and the same with its pattern taken away: the
    // second shows one colour
    let src = r##"
match toad.gpu() {
    Err(e) => "no gpu",
    Ok(_) => {
        let mat = #{ "color": "#000000", "color2": "#ffffff", "pattern": "checker", "scale": 8.0 }
        let a = toad.frame([#{ "shapes": [#{ "kind": "box", "size": (64.0, 64.0), "material": mat }] }],
                           #{ "size": (64, 64), "probe": [(28, 28), (36, 28)] })
        let b = toad.frame([#{ "shapes": [#{ "kind": "box", "size": (64.0, 64.0), "material": mat, "pattern": () }] }],
                           #{ "size": (64, 64), "probe": [(28, 28), (36, 28)] })
        show(a.probe[0] != a.probe[1]) + " " + show(b.probe[0] == b.probe[1])
    }
}"##;
    let out = run(src).expect("ran");
    assert!(out == "true true" || out == "no gpu", "{out}");
}

#[test]
fn text_in_the_world_is_drawn_at_its_size() {
    // a label's ink is where it is and not beside it, at any zoom
    let src = r##"
match toad.gpu() {
    Err(e) => "no gpu",
    Ok(_) => {
        let l = #{ "text": "MMMM", "at": (0.0, 0.0), "size": 40.0, "color": "#ffffff" }
        let lum = (c) => c[0] + c[1] + c[2]
        let row = range(0, 40) |> map((i) => (88 + i * 2, 64))
        let ink = (f) => f.probe |> fold(0, (m, c) => max(m, lum(c)))
        let one = toad.frame([#{ "sprites": [l] }], #{ "size": (256, 128), "scale": 1.0, "probe": row })
        let two = toad.frame([#{ "sprites": [l] }], #{ "size": (256, 128), "scale": 2.5, "probe": row })
        let above = toad.frame([#{ "sprites": [l] }], #{ "size": (256, 128), "scale": 1.0, "probe": row |> map((p) => (p[0], 8)) })
        show(ink(one) > 400 && ink(two) > 400 && ink(above) < 30) + " " + show((ink(one), ink(two), ink(above)))
    }
}"##;
    let out = run(src).expect("ran");
    assert!(out.starts_with("true") || out == "no gpu", "{out}");
}

// ── physics ────────────────────────────────────────────────────────────

const FLOOR: &str = r#"
fn ok(r) = match r { Ok(v) => v, Err(e) => raise(e) }
let w = toad.physics(#{ "gravity": (0.0, -980.0) })
let floor = toad.body(w, #{ "kind": "static", "shape": "box", "size": (2000.0, 40.0), "at": (0.0, -20.0) })
fn steps(n) = { let mut all = []
    for i in range(0, n) { all = all + toad.simulate(w, 1.0 / 120.0) }
    all }
"#;

#[test]
fn a_body_falls_rests_and_says_it_touched() {
    let src = format!(
        "{FLOOR}
let ball = toad.body(w, #{{ \"shape\": \"circle\", \"size\": 25.0, \"at\": (0.0, 400.0) }})
let touches = steps(240)
let at = toad.pose(w, ball).at
let first = touches[0]
show(abs(at[1] - 25.0) < 0.5) + \" \" + show((first.a, first.b, first.start)) + \" \" + show(first.normal)"
    );
    let out = run(&src).expect("ran");
    // the ball rests on the floor's top (0) by its radius; the first touch
    // is the two of them starting, the normal up from the ball's side
    assert!(out.starts_with("true (2, 1, true)") || out.starts_with("true (1, 2, true)"), "{out}");
}

#[test]
fn a_ray_meets_the_first_body_and_overlap_finds_what_is_there() {
    let src = format!(
        "{FLOOR}
let ball = toad.body(w, #{{ \"kind\": \"static\", \"shape\": \"circle\", \"size\": 25.0, \"at\": (0.0, 100.0) }})
steps(1)
let hit = toad.ray(w, (0.0, 300.0), (0.0, -300.0), #{{}})
let past = toad.ray(w, (0.0, 300.0), (0.0, -300.0), #{{ \"skip\": ball }})
let none = toad.ray(w, (500.0, 300.0), (500.0, 200.0), #{{}})
show((hit.body == ball, abs(hit.at[1] - 125.0) < 0.1, past.body == floor, none == (), toad.overlap(w, (0.0, 100.0), 5.0, #{{}}) == [ball]))"
    );
    assert_eq!(run(&src).as_deref(), Ok("(true, true, true, true, true)"));
}

#[test]
fn joints_hold_bodies_together() {
    // a ball on a 100-unit rope from a fixed point never falls further
    let src = format!(
        "{FLOOR}
let hook = toad.body(w, #{{ \"kind\": \"static\", \"shape\": \"circle\", \"size\": 4.0, \"at\": (0.0, 600.0) }})
let ball = toad.body(w, #{{ \"shape\": \"circle\", \"size\": 10.0, \"at\": (80.0, 600.0) }})
toad.joint(w, hook, ball, #{{ \"kind\": \"rope\", \"length\": 100.0 }})
let mut far = 0.0
for i in range(0, 360) {{
    toad.simulate(w, 1.0 / 120.0)
    let p = toad.pose(w, ball).at
    far = max(far, math.sqrt(p[0] * p[0] + (p[1] - 600.0) * (p[1] - 600.0)))
}}
show(far < 104.0) + \" \" + show(far)"
    );
    let out = run(&src).expect("ran");
    assert!(out.starts_with("true"), "{out}");
}

#[test]
fn physics_is_the_same_every_run() {
    let src = format!(
        "{FLOOR}
for i in range(0, 40) {{
    toad.body(w, #{{ \"shape\": if i % 2 == 0 => \"circle\" else => \"box\", \"size\": if i % 2 == 0 => 10.0 else => (20.0, 20.0),
                     \"at\": (to_float(i % 8) * 22.0, 30.0 + to_float(i / 8) * 25.0), \"rot\": to_float(i) * 0.3 }})
}}
steps(300)
show(range(2, 42) |> map((id) => toad.pose(w, id).at))"
    );
    let a = run(&src).expect("ran");
    let b = run(&src).expect("ran");
    assert_eq!(a, b);
}

#[test]
fn a_swarm_lands_on_bodies_bounces_and_is_counted() {
    let src = format!(
        "{FLOOR}
fn fall(p, w) = #{{ \"pos\": p.pos + p.vel * w.dt, \"vel\": p.vel + (0.0, -980.0) * w.dt }}
let s = ok(toad.swarm(ok(toad.kernel(fall, #{{ \"pos\": (0.0, 0.0), \"vel\": (0.0, 0.0), \"hit\": 0.0 }}, #{{}})), 2000, \"play\"))
toad.spawn(s, 2000, #{{ \"pos\": (\"box\", (0.0, 300.0), (800.0, 200.0)), \"seed\": 3 }})
let mut hits = 0
for i in range(0, 240) {{
    toad.step(s, #{{ \"dt\": 1.0 / 120.0 }})
    let r = toad.collide(s, w, #{{ \"radius\": 3.0, \"bounce\": 0.2, \"mark\": \"hit\" }})
    hits = hits + (r.hits |> fold(0, (a, h) => a + h[1]))
}}
let low = toad.read(s, \"pos\") |> fold(1e9, (a, p) => min(a, p[1]))
toad.step(s, #{{ \"dt\": 1.0 / 120.0 }})
let gone = toad.collide(s, w, #{{ \"radius\": 3.0, \"die\": true }})
show(low > 2.9) + \" \" + show(hits > 2000) + \" \" + show(gone.died) + \" \" + show(toad.count(s))"
    );
    // none fell through; every one touched the floor at least once; all
    // resting on it die when touching kills
    assert_eq!(run(&src).as_deref(), Ok("true true 2000 0"));
}

#[test]
fn two_swarms_meet_each_taken_once_in_order() {
    let src = r#"
fn ok(r) = match r { Ok(v) => v, Err(e) => raise(e) }
fn still(p, w) = #{ "pos": p.pos }
let k = ok(toad.kernel(still, #{ "pos": (0.0, 0.0), "met": 0.0 }, #{}))
let shots = ok(toad.swarm(k, 10, "play"))
let foes = ok(toad.swarm(k, 10, "play"))
// three shots on one foe; one shot alone; one foe alone
for x in [0.0, 1.0, 2.0, 100.0] { toad.spawn(shots, 1, #{ "pos": (x, 0.0) }) }
for x in [0.5, 300.0] { toad.spawn(foes, 1, #{ "pos": (x, 0.0) }) }
let r = toad.meet(shots, foes, #{ "radius_a": 2.0, "radius_b": 2.0, "kill": "both" })
show((r.met, r.died_a, r.died_b, toad.count(shots), toad.count(foes), toad.read(shots, "pos") |> map((p) => p[0])))
"#;
    // the first shot in order takes the foe; the other two pass
    assert_eq!(run(src).as_deref(), Ok("(1, 1, 1, 3, 1, [1.0, 2.0, 100.0])"));
}

#[test]
fn what_physics_cannot_build_is_refused_with_the_reason() {
    let refused = |body: &str| -> String {
        run(&format!(
            "let w = toad.physics(#{{}})
match attempt(() => {body}) {{ Ok(_) => \"built\", Err(e) => show(e) }}"
        ))
        .expect("ran")
    };
    assert!(refused(r#"toad.body(w, #{ "shape": "blob" })"#).contains("circle, box, capsule, poly or chain"));
    assert!(refused(r#"toad.body(w, #{ "kind": "floaty" })"#).contains("dynamic, static or kinematic"));
    assert!(refused(r#"toad.body(w, #{ "shape": "poly" })"#).contains("needs `points`"));
    assert!(refused(r#"toad.joint(w, toad.body(w, #{}), toad.body(w, #{}), #{ "kind": "glue" })"#).contains("pin, weld, slide, rope or spring"));
    assert!(refused(r#"toad.joint(w, toad.body(w, #{}), toad.body(w, #{}), #{ "kind": "rope" })"#).contains("needs a `length`"));
    assert!(refused(r#"toad.push(w, 99, #{})"#).contains("no body 99"));
}

// ── sound ──────────────────────────────────────────────────────────────

#[cfg(feature = "audio")]
fn quiet(src: &str) -> String {
    // SAFETY: the tests that touch it set the same value
    unsafe { std::env::set_var("TOAD_MUTE", "1") };
    run(src).expect("ran")
}

#[cfg(feature = "audio")]
#[test]
fn sounds_are_made_in_code_as_long_as_their_envelope() {
    let out = quiet(
        r#"
let blip = toad.sound(#{ "wave": "square", "freq": 880.0, "attack": 0.01, "hold": 0.04, "decay": 0.05, "volume": 0.4 })
let i = toad.sound_info(blip)
let late = toad.sound(#{ "layers": [#{ "decay": 0.1 }, #{ "delay": 0.5, "decay": 0.1 }] })
show((math.round(i.seconds * 100.0), math.round(i.peak * 10.0), i.channels, math.round(toad.sound_info(late).seconds * 10.0)))
"#,
    );
    // 0.1 s long, peaking at its volume, mono; the late layer makes it 0.6 s
    assert_eq!(out, "(10, 4, 1, 6)");
}

#[cfg(feature = "audio")]
#[test]
fn the_mix_rounds_off_what_is_too_loud_and_lets_a_sound_end() {
    let out = quiet(
        r#"
let loud = toad.sound(#{ "layers": [#{ "wave": "square", "freq": 100.0, "decay": 0.3, "volume": 1.0 },
                                    #{ "wave": "square", "freq": 100.0, "decay": 0.3, "volume": 1.0 }] })
let m = toad.listen(loud, 1.0)
show((toad.sound_info(loud).peak > 1.5, m.peak <= 1.0, m.voices_left))
"#,
    );
    assert_eq!(out, "(true, true, 0)");
}

#[cfg(feature = "audio")]
#[test]
fn a_wav_file_is_read_and_what_cannot_sound_is_refused() {
    // a tenth of a second of a 16-bit stereo WAV at 22050 Hz, a square
    let dir = std::env::temp_dir().join(format!("toad-wav-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("beep.wav");
    let frames = 2205u32;
    let mut b = Vec::new();
    let data = frames * 4;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&22050u32.to_le_bytes());
    b.extend_from_slice(&(22050u32 * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for i in 0..frames {
        let v: i16 = if (i / 50) % 2 == 0 { 8000 } else { -8000 };
        b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(&path, b).unwrap();
    let out = quiet(&format!(
        r#"
let s = toad.sound(#{{ "file": "{}" }})
let i = toad.sound_info(s)
show((math.round(i.seconds * 10.0), i.channels, i.rate, math.round(i.peak * 100.0)))
 + " " + show(attempt(() => toad.sound(#{{ "wave": "kazoo" }})))
 + " " + show(attempt(() => toad.sound(#{{ "file": "x.mp3" }})))
 + " " + show(attempt(() => toad.volume("loud", 1.0)))
"#,
        path.display()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.starts_with("(1, 2, 22050, 24)"), "{out}");
    assert!(out.contains("sine, square, saw, triangle or noise"), "{out}");
    assert!(out.contains(".wav or .ogg"), "{out}");
    assert!(out.contains("master, fx or music"), "{out}");
}

#[cfg(feature = "audio")]
#[test]
fn a_muted_run_plays_silently() {
    let out = quiet(
        r#"
let s = toad.sound(#{ "decay": 0.1 })
let v = toad.play(s, #{ "volume": 0.5, "loop": true })
toad.tune(v, #{ "pitch": 2.0 })
toad.stop(v, 0.1)
show(toad.audio())
"#,
    );
    assert!(out.contains("muted"), "{out}");
}

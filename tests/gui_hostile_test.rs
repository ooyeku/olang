//! `gui` given what a program may give it and a person never should: an
//! absurd text size, a device or a FIFO as a picture's file, sizes far
//! past a display. Each must answer (an error, nothing drawn) and never
//! take the process down: a panic in the engine aborts the application
//! (release builds abort on panic), losing what the person had open.

#![cfg(feature = "gui")]

use olang::{Interpreter, Parser, Value};

static EVENTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run(src: &str) -> Value {
    let _turn = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    let program = Parser::new().parse(src).expect("parses");
    Interpreter::new().eval_program(program).expect("runs")
}

#[test]
fn a_text_at_an_absurd_size_draws_nothing_and_the_window_goes_on() {
    let v = run(r##"
        let w = gui.headless(#{ "size": (200, 100) })
        let r = gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 200, 100), "style": #{ "bg": "#ffffff" } },
          #{ "key": "t", "parent": "root", "role": "text", "box": (0, 0, 200, 100), "text": "Wide", "style": #{ "size": 1.0e30 } },
          #{ "key": "c", "parent": "root", "role": "figure", "box": (0, 0, 200, 100),
             "draw": [#{ "op": "text", "x": 0, "y": 0, "text": "W", "size": 1.0e12 }] }
        ])
        let png = gui.read(w, "pixels")
        [r, bytes.len(png) > 100]
    "##);
    assert_eq!(format!("{v}"), "[Ok(()), true]");
}

#[test]
fn a_picture_from_a_device_or_a_folder_is_refused_not_read_forever() {
    let v = run(r##"
        let a = gui.image_info("/dev/zero")
        let b = gui.image_info("/")
        let w = gui.headless(#{ "size": (120, 80) })
        let r = gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 120, 80), "style": #{ "bg": "#ffffff" } },
          #{ "key": "i", "parent": "root", "role": "image", "box": (0, 0, 120, 80), "image": "/dev/zero" }
        ])
        let png = gui.read(w, "pixels")
        [match a { Ok(_) => "ok", Err(e) => if str.contains(e, "not a file") => "refused" else => e },
         match b { Ok(_) => "ok", Err(e) => if str.contains(e, "not a file") => "refused" else => e },
         r, bytes.len(png) > 100]
    "##);
    assert_eq!(format!("{v}"), r#"["refused", "refused", Ok(()), true]"#);
}

#[test]
fn a_lost_gpu_device_is_made_again_and_draws_what_the_software_renderer_draws() {
    use olang::stdlib::gui::{gpu, soft, window::WinState};
    let _turn = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(g) = gpu::gpu(None, None) else {
        eprintln!("skipped: no GPU here");
        return;
    };
    let mut st = WinState::new(1, true, "lost", 120.0, 80.0, 1.0);
    let ops = Parser::new()
        .parse(r##"[#{ "key": "root", "box": (0, 0, 120, 80), "style": #{ "bg": "#ffffff" } },
                    #{ "key": "b", "parent": "root", "box": (10, 10, 60, 30), "style": #{ "bg": "#3366cc", "radius": 6 } }]"##)
        .ok();
    let ops = ops.and_then(|p| Interpreter::new().eval_program(p).ok()).expect("ops");
    let mut out = vec![];
    st.apply(&ops, &mut out).expect("applies");
    let dl = st.display_list();
    let before = gpu::recoveries();
    // a GPU reset: the device destroyed under the engine
    assert!(gpu::simulate_loss());
    assert!(g.is_lost());
    let g2 = gpu::gpu(None, None).expect("a device made again");
    assert!(!g2.is_lost());
    assert!(!std::sync::Arc::ptr_eq(&g, &g2));
    assert_eq!(gpu::recoveries(), before + 1);
    // and it draws, as the reference does
    let a = soft::rgba(&soft::render(&dl));
    let b = g2.render_rgba(&dl);
    assert_eq!(a.len(), b.len());
    let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
    let mid = |px: &[u8]| px[((40 * 120 + 40) * 4)..((40 * 120 + 40) * 4 + 4)].to_vec();
    assert_eq!(mid(&a), mid(&b), "the button's middle (worst pixel {worst})");
}

#[test]
fn a_gpu_error_is_said_not_a_panic_that_aborts_the_application() {
    use olang::stdlib::gui::gpu;
    let _turn = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(g) = gpu::gpu(None, None) else {
        eprintln!("skipped: no GPU here");
        return;
    };
    // a texture past the device's limit: wgpu's own handler panics on it
    let most = g.device.limits().max_texture_dimension_2d;
    let _t = g.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("too wide"),
        size: wgpu::Extent3d { width: most + 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    // a validation error is no loss: the device still draws
    assert!(!g.is_lost());
}

#[test]
fn a_new_field_scrolled_to_a_line_of_wrapped_text_shows_that_line() {
    // asked at the field's birth, before it had a width: the tops of the
    // wrapped paragraphs above were guesses and the line landed elsewhere
    let v = run(r##"
        let ev = gui.events()
        let mut lines = []
        for i in 0..60 { lines = lines + ["p" + to_string(i) + " " + str.repeat("word ", 90)] }
        let text = join(lines, "\n")
        let w = gui.headless(#{ "size": (300, 200) })
        gui.apply(w, [#{ "key": "root", "box": (0, 0, 300, 200), "style": #{ "bg": "#ffffff" } },
            #{ "key": "f", "parent": "root", "role": "textarea", "box": (0, 0, 300, 200), "style": #{ "size": 13 },
               "edit": #{ "value": text, "rich": true, "undo": false, "report": true, "select": (0, 0, 1), "scroll_to": (40, 1) } }])
        let _p = gui.read(w, "pixels")
        let mut drain = true
        while drain { match chan.try_recv(ev) { Ok(()) => { drain = false }, Ok(e) => (), _ => { drain = false } } }
        gui.input(w, #{ "kind": "pointer", "action": "move", "x": 150, "y": 100 })
        gui.input(w, #{ "kind": "wheel", "dy": -1 })
        let mut top = ()
        let mut more = true
        while more { match chan.try_recv(ev) { Ok(()) => { more = false }, Ok(e) => { if map_get(e, "kind") == "viewport" => { top = map_get(e, "top") } }, _ => { more = false } } }
        top
    "##);
    assert_eq!(format!("{v}"), "40");
}

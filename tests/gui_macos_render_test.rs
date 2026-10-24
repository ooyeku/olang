//! The render thread on a real window (macOS): the hand-over under a
//! storm of patches, and a resize.
//!
//! An olang program opens a window and, asked, applies a few hundred
//! patches as fast as it can, each painting the whole window one colour
//! of four (and moving a box, and changing a text). While they land, the
//! test reads the render thread's kept frame again and again (between
//! its frames, as it draws them): every frame it holds is one patch's
//! whole — the band the patches paint is one of the four colours, never
//! two (no frame torn between patches). Once the storm settles, the kept
//! frame the GPU drew agrees with the software renderer's drawing of the
//! window's display list. Then the window is resized three times: each
//! time the render thread presents a frame of the new size within a few
//! refreshes, and it agrees with the software renderer too.
//!
//! The harness is off: AppKit needs the process's first thread, which
//! `gui::platform::host` takes for winit's loop.

#[cfg(all(feature = "gui", target_os = "macos"))]
mod mac {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;
    use olang::stdlib::gui::platform;
    use olang::stdlib::gui::{render, soft, windows};
    use olang::{Interpreter, Parser};
    use std::ffi::c_void;
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    const PROGRAM: &str = r##"
        let ev = gui.events()
        let w = unwrap(gui.open(#{ "title": "olang render", "size": (320, 200) }))
        let colours = ["#ffffff", "#fde68a", "#bbf7d0", "#bfdbfe"]
        fn scene(k) = [
          #{ "key": "root", "box": (0, 0, 320, 200), "style": #{ "bg": colours[k % 4] } },
          #{ "key": "t", "parent": "root", "role": "text", "box": (10, 8, 200, 20), "text": "frame " + to_string(k),
             "style": #{ "color": "#111111" } },
          #{ "key": "b", "parent": "root", "box": (10 + (k % 20) * 10, 120, 40, 40), "style": #{ "bg": "#2563eb", "radius": 6 } }
        ]
        gui.apply(w, scene(0))
        let mut going = true
        while going {
          match chan.recv_timeout(ev, #{ "timeout_ms": 30000 }) {
            Ok(e) => {
              if e == () || e.kind == "test_done" => { going = false }
              else if e.kind == "storm" => {
                for k in 1..e.n { gui.apply(w, scene(k)) }
              }
            },
            Err(_) => { going = false }
          }
        }
        gui.close(w)
        "ended"
    "##;

    const COLOURS: [[u8; 3]; 4] = [
        [0xff, 0xff, 0xff],
        [0xfd, 0xe6, 0x8a],
        [0xbb, 0xf7, 0xd0],
        [0xbf, 0xdb, 0xfe],
    ];

    unsafe extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
    }

    type Job = Box<dyn FnOnce() + Send>;

    extern "C" fn run_job(ctx: *mut c_void) {
        let job = unsafe { Box::from_raw(ctx as *mut Job) };
        job();
    }

    fn on_main<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        let job: Job = Box::new(move || {
            let _ = tx.send(f());
        });
        let ctx = Box::into_raw(Box::new(job)) as *mut c_void;
        unsafe { dispatch_async_f(&raw const _dispatch_main_q, ctx, run_job) };
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the main thread answers")
    }

    fn the_window() -> u64 {
        *windows().lock().unwrap().keys().next().expect("a window")
    }

    fn to_front() {
        on_main(|| {
            let cls = AnyClass::get(c"NSApplication").unwrap();
            let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
            let wins: Retained<objc2_foundation::NSArray<AnyObject>> =
                unsafe { msg_send![&*app, windows] };
            for w in wins.to_vec() {
                let t: Option<Retained<NSString>> = unsafe { msg_send![&*w, title] };
                if t.is_some_and(|t| t.to_string() == "olang render") {
                    let _: () = unsafe { msg_send![&*w, orderFrontRegardless] };
                }
            }
        })
    }

    /// The band the patches paint (between the text and the box, the
    /// window's width): one colour of the four, or `None` (torn, or
    /// something else).
    fn band_colour(w: u32, px: &[u8], scale: f32) -> Option<usize> {
        let (y0, y1) = ((40.0 * scale) as u32, (100.0 * scale) as u32);
        let first = {
            let i = ((y0 * w) * 4) as usize;
            [px[i], px[i + 1], px[i + 2]]
        };
        let c = COLOURS.iter().position(|c| *c == first)?;
        for y in y0..y1 {
            for x in 0..w.min((320.0 * scale) as u32) {
                let i = ((y * w + x) * 4) as usize;
                if [px[i], px[i + 1], px[i + 2]] != first {
                    return None;
                }
            }
        }
        Some(c)
    }

    /// The renderers' antialiasing differs; beyond this they disagree (as
    /// tests/gui_damage_test.rs holds them).
    fn agree(soft_px: &[u8], gpu_px: &[u8]) -> Result<(), String> {
        if soft_px.len() != gpu_px.len() {
            return Err(format!(
                "sizes differ: {} and {}",
                soft_px.len(),
                gpu_px.len()
            ));
        }
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

    /// Wait until no frame was drawn for `quiet`.
    fn settle(quiet: Duration) {
        let start = Instant::now();
        let mut last = platform::frames_presented();
        let mut since = Instant::now();
        while since.elapsed() < quiet && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
            let now = platform::frames_presented();
            if now != last {
                last = now;
                since = Instant::now();
            }
        }
    }

    /// The window's display list now, drawn by the software renderer.
    fn soft_now(id: u64) -> (u32, u32, Vec<u8>) {
        let st = windows().lock().unwrap().get(&id).cloned().unwrap();
        let dl = st.lock().unwrap().display_list();
        let pm = soft::render(&dl);
        (dl.width, dl.height, soft::rgba(&pm))
    }

    fn scale_of(id: u64) -> f32 {
        let st = windows().lock().unwrap().get(&id).cloned().unwrap();
        st.lock().unwrap().scale
    }

    pub fn main() {
        olang::stdio::set_panic_hook();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(90));
            eprintln!("gui_macos_render_test: timed out");
            std::process::exit(2);
        });
        static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let code = platform::host(|| {
            std::thread::spawn(|| {
                let prober = std::thread::spawn(|| {
                    let say = |s: String| SAID.lock().unwrap().push(s);
                    let start = Instant::now();
                    while platform::frames_presented() == 0 {
                        assert!(start.elapsed() < Duration::from_secs(20), "no first frame");
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let id = the_window();
                    to_front();
                    settle(Duration::from_millis(200));
                    say(format!(
                        "on the render thread: {}",
                        render::kept(id).is_some()
                    ));
                    let scale = scale_of(id);
                    // the storm: the kept frame read again and again as it lands
                    let shown0 = platform::frames_shown();
                    olang::stdlib::gui::emit(vec![olang::stdlib::gui::values::event(
                        "storm",
                        vec![("n", olang::Value::Integer(600))],
                    )]);
                    let (mut reads, mut torn, mut seen) = (0, 0, [false; 4]);
                    let t = Instant::now();
                    while t.elapsed() < Duration::from_millis(1500) {
                        if let Some((w, _h, px)) = render::kept(id) {
                            reads += 1;
                            match band_colour(w, &px, scale) {
                                Some(c) => seen[c] = true,
                                None => torn += 1,
                            }
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    let presented = platform::frames_shown() - shown0;
                    println!(
                        "storm: {presented} frames presented, the kept frame read {reads} times, {torn} torn, colours seen {seen:?}"
                    );
                    say(format!("no torn frames: {}", reads > 20 && torn == 0));
                    settle(Duration::from_millis(300));
                    let (gw, gh, gpu_px) = render::kept(id).expect("the kept frame");
                    let (sw, sh, soft_px) = soft_now(id);
                    let a = agree(&soft_px, &gpu_px);
                    println!("after the storm: GPU {gw}x{gh}, software {sw}x{sh}: {a:?}");
                    say(format!("GPU and software agree: {}", a.is_ok()));
                    // a resize: a frame of the new size presented within a
                    // few refreshes, alike in both renderers
                    let mut ok = true;
                    for (lw, lh) in [(360, 230), (280, 180), (400, 260)] {
                        let want = (
                            (lw as f32 * scale).round() as u32,
                            (lh as f32 * scale).round() as u32,
                        );
                        let t = Instant::now();
                        platform::set(
                            id,
                            olang::stdlib::gui::values::map(vec![(
                                "size",
                                olang::Value::Tuple(std::sync::Arc::new(vec![
                                    olang::Value::Integer(lw),
                                    olang::Value::Integer(lh),
                                ])),
                            )]),
                        );
                        let mut got = None;
                        while t.elapsed() < Duration::from_secs(2) {
                            if let Some((w, h, _)) = render::kept(id)
                                && (w, h) == want
                            {
                                got = Some(t.elapsed());
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        settle(Duration::from_millis(200));
                        let (gw, gh, gpu_px) = render::kept(id).expect("the kept frame");
                        let (sw, sh, soft_px) = soft_now(id);
                        let a = agree(&soft_px, &gpu_px);
                        println!(
                            "resized to {lw}x{lh}: a frame of {gw}x{gh} after {:?} (software {sw}x{sh}): {a:?}",
                            got
                        );
                        ok &= got.is_some_and(|d| d < Duration::from_millis(250)) && a.is_ok();
                    }
                    say(format!("resized: {ok}"));
                    olang::stdlib::gui::emit(vec![olang::stdlib::gui::values::event(
                        "test_done",
                        vec![],
                    )]);
                });
                let program = Parser::new().parse(PROGRAM).expect("parses");
                let v = Interpreter::new().eval_program(program);
                let _ = prober.join();
                println!("program: {v:?}");
                0
            })
        });
        assert_eq!(code, 0);
        let said = SAID.lock().unwrap().clone();
        for s in &said {
            println!("{s}");
        }
        assert_eq!(
            said,
            vec![
                "on the render thread: true",
                "no torn frames: true",
                "GPU and software agree: true",
                "resized: true"
            ]
        );
        println!("gui_macos_render_test: ok");
    }
}

fn main() {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    mac::main();
}

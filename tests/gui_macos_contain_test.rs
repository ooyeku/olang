//! No panic crosses into AppKit, and a lost GPU device draws again
//! (macOS).
//!
//! An olang program opens a real window. The test makes each kind of
//! call the platform makes into olang panic once, on purpose
//! (`olang::contain::arm`): winit's handlers (a window's event, a
//! request from the program, the loop about to wait), a frame's drawing,
//! the methods AppKit calls on the application's delegate (documents to
//! open, the tab bar's +), a notice's block, a menu item's action, and a
//! `gui` builtin on the program's thread. Each must be contained: the
//! program hears a `problem` naming the place, the window draws again,
//! and the process never aborts (olang's panic hook, installed here,
//! aborts on any panic no `catch` holds). Then the process's GPU device
//! is lost as a GPU reset loses it (destroyed): the next frame must make
//! a device again and present.
//!
//! The harness is off: AppKit needs the process's first thread, which
//! `gui::platform::host` takes for winit's loop.

#[cfg(all(feature = "gui", target_os = "macos"))]
mod mac {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use olang::stdlib::gui::{gpu, platform};
    use olang::{Interpreter, Parser};
    use std::ffi::c_void;
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    // The program: a window, a builtin made to panic (its error kept), then
    // every `problem` heard until the test says it is done.
    const PROGRAM: &str = r##"
        let ev = gui.events()
        let w = unwrap(gui.open(#{ "title": "olang contain", "size": (320, 200) }))
        let scene = [
          #{ "key": "root", "box": (0, 0, 320, 200), "style": #{ "bg": "#ffffff" } },
          #{ "key": "b", "parent": "root", "role": "button", "box": (10, 10, 140, 40), "text": "Button" }
        ]
        // the first apply is made to panic (armed by the test): an error
        let applied = match gui.apply(w, scene) { Ok(_) => "ok", Err(e) => if str.contains(e, "contained") => "contained" else => e }
        gui.apply(w, scene)
        let _m = gui.menu([#{ "title": "olang", "items": [#{ "id": "quit", "label": "Quit" }] },
                  #{ "title": "Test", "items": [#{ "id": "probe", "label": "Probe" }] }])
        let mut wheres = []
        let mut going = true
        let t0 = time.monotonic()
        while going && time.monotonic() - t0 < 45000.0 {
          match chan.recv_timeout(ev, #{ "timeout_ms": 100 }) {
            Ok(e) => {
              if e != () && e.kind == "problem" => { wheres = wheres + [e.where] }
              if e != () && e.kind == "test_done" => { going = false }
            },
            Err(_) => ()
          }
        }
        gui.close(w)
        [applied, sort(wheres)]
    "##;

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

    /// Run `f` on the main thread (winit's loop drains the main queue) and
    /// wait for its answer.
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

    fn wait_for(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
        let start = Instant::now();
        while !ok() {
            if start.elapsed() > Duration::from_secs(secs) {
                panic!("timed out waiting for {what}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn the_window() -> u64 {
        let ws = olang::stdlib::gui::windows().lock().unwrap();
        *ws.keys().next().expect("a window")
    }

    fn delegate() -> Retained<AnyObject> {
        let cls = AnyClass::get(c"NSApplication").expect("AppKit");
        let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
        let d: Option<Retained<AnyObject>> = unsafe { msg_send![&*app, delegate] };
        d.expect("winit's delegate")
    }

    /// Make `site` panic, do `poke` (which reaches it), and wait for the
    /// panic to be contained and the window to present a frame after it.
    fn contain(site: &str, id: u64, poke: impl Fn()) {
        let before = platform::problems_contained();
        olang::contain::arm(site);
        let start = Instant::now();
        while platform::problems_contained() == before {
            poke();
            std::thread::sleep(Duration::from_millis(60));
            if start.elapsed() > Duration::from_secs(8) {
                panic!("{site}: never reached (no problem contained)");
            }
        }
        let frames = platform::frames_presented();
        platform::redraw(id);
        wait_for(&format!("a frame after {site}'s panic"), 8, || {
            platform::redraw(id);
            platform::frames_presented() > frames
        });
        println!("contained: {site}");
    }

    pub fn main() {
        olang::stdio::set_panic_hook();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(90));
            eprintln!("gui_macos_contain_test: timed out");
            std::process::exit(2);
        });
        static RESULT: Mutex<Option<String>> = Mutex::new(None);
        static RECOVERED: Mutex<Option<(u64, bool)>> = Mutex::new(None);
        let code = platform::host(|| {
            std::thread::spawn(|| {
                let prober = std::thread::spawn(|| {
                    wait_for("the window's first frame", 20, || {
                        platform::frames_presented() > 0
                    });
                    let id = the_window();
                    contain("window_event", id, || {
                        // a size asked: the window hears it resized
                        let sz = olang::stdlib::gui::values::map(vec![(
                            "size",
                            olang::Value::Tuple(std::sync::Arc::new(vec![
                                olang::Value::Integer(330),
                                olang::Value::Integer(210),
                            ])),
                        )]);
                        platform::set(id, sz);
                    });
                    contain("user_event", id, || platform::redraw(id));
                    contain("about_to_wait", id, || platform::redraw(id));
                    contain("render", id, || platform::redraw(id));
                    contain("settings_notice", id, || {
                        on_main(platform::post_settings_notice)
                    });
                    contain("new_tab", id, || {
                        on_main(|| {
                            let d = delegate();
                            let none: *const AnyObject = std::ptr::null();
                            let _: () = unsafe { msg_send![&*d, newWindowForTab: none] };
                        })
                    });
                    contain("open_urls", id, || {
                        on_main(|| {
                            let d = delegate();
                            let url_cls = AnyClass::get(c"NSURL").unwrap();
                            let path = objc2_foundation::NSString::from_str("/tmp");
                            let url: Retained<AnyObject> =
                                unsafe { msg_send![url_cls, fileURLWithPath: &*path] };
                            let urls = objc2_foundation::NSArray::from_retained_slice(&[url]);
                            let none: *const AnyObject = std::ptr::null();
                            let _: () =
                                unsafe { msg_send![&*d, application: none, openURLs: &*urls] };
                        })
                    });
                    contain("menu_event", id, || {
                        on_main(|| {
                            let cls = AnyClass::get(c"NSApplication").unwrap();
                            let app: Retained<AnyObject> =
                                unsafe { msg_send![cls, sharedApplication] };
                            let bar: Option<Retained<AnyObject>> =
                                unsafe { msg_send![&*app, mainMenu] };
                            let Some(bar) = bar else { return };
                            let n: isize = unsafe { msg_send![&*bar, numberOfItems] };
                            if n < 2 {
                                return;
                            }
                            let top: Retained<AnyObject> =
                                unsafe { msg_send![&*bar, itemAtIndex: 1isize] };
                            let sub: Option<Retained<AnyObject>> =
                                unsafe { msg_send![&*top, submenu] };
                            if let Some(sub) = sub {
                                let _: () = unsafe {
                                    msg_send![&*sub, performActionForItemAtIndex: 0isize]
                                };
                            }
                        })
                    });
                    // the device lost: drawing goes on, on a device made again
                    // (the scene is as it was: only a frame drawn whole on the
                    // new device presents — the kept frame was the lost one's)
                    std::thread::sleep(Duration::from_millis(300));
                    let had_gpu = gpu::simulate_loss();
                    if had_gpu {
                        let shown = platform::frames_shown();
                        wait_for("a frame presented on a device made again", 15, || {
                            platform::redraw(id);
                            platform::frames_shown() > shown && gpu::recoveries() > 0
                        });
                    }
                    *RECOVERED.lock().unwrap() = Some((gpu::recoveries(), had_gpu));
                    olang::stdlib::gui::emit(vec![olang::stdlib::gui::values::event(
                        "test_done",
                        vec![],
                    )]);
                });
                olang::contain::arm("gui.apply");
                let program = Parser::new().parse(PROGRAM).expect("parses");
                let v = Interpreter::new().eval_program(program);
                match &v {
                    Ok(v) => eprintln!("the program answered {v}"),
                    Err(e) => eprintln!("the program failed: {e}"),
                }
                let _ = prober.join();
                *RESULT.lock().unwrap() = Some(match v {
                    Ok(v) => format!("{v}"),
                    Err(e) => format!("error: {e}"),
                });
                0
            })
        });
        let result = RESULT.lock().unwrap().take().unwrap_or_default();
        let recovered = RECOVERED.lock().unwrap().take();
        println!("program: {result}");
        println!("recovered: {recovered:?}");
        assert_eq!(code, 0);
        assert!(result.starts_with("[\"contained\", ["), "{result}");
        for site in [
            "about_to_wait",
            "gui.apply",
            "menu_event",
            "new_tab",
            "open_urls",
            "render",
            "settings_notice",
            "user_event",
            "window_event",
        ] {
            assert!(
                result.contains(&format!("\"{site}\"")),
                "{site} not heard: {result}"
            );
        }
        match recovered {
            Some((n, true)) => assert!(n >= 1, "the device was not made again"),
            Some((_, false)) => println!("no GPU here: the device's loss not tried"),
            None => panic!("the prober did not finish"),
        }
        println!("gui_macos_contain_test: ok");
    }
}

fn main() {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    mac::main();
}

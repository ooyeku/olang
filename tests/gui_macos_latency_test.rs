//! Input to photon on a real window (macOS): what `GUI_LATENCY` measures.
//!
//! An olang program opens a window and answers every key with a patch (a
//! line of text that says how many keys came), after `work` ms of its own
//! (a turn's cost). The test sends the window's view key events as AppKit
//! would (`sendEvent:` on the window: winit's view takes them), in two
//! runs: keys well apart (a person typing), and keys faster than the
//! display refreshes while each turn costs a few ms (a key held, a
//! trackpad's scroll). For each frame presented after input the engine
//! times the input to the present call, to the GPU done, and to the first
//! refresh at or after that; the runs' percentiles are printed. It checks
//! only that frames were timed and that no frame waited more than a few
//! refreshes behind its input: the numbers are for comparing builds.
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
    use olang::stdlib::gui::platform::frames;
    use olang::{Interpreter, Parser};
    use std::ffi::c_void;
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    const PROGRAM: &str = r##"
        let ev = gui.events()
        let w = unwrap(gui.open(#{ "title": "olang latency", "size": (420, 160) }))
        fn scene(n) = [
          #{ "key": "root", "box": (0, 0, 420, 160), "style": #{ "bg": "#ffffff" } },
          #{ "key": "t", "parent": "root", "role": "text", "box": (10, 10, 400, 30), "text": "keys: " + to_string(n) },
          #{ "key": "b", "parent": "root", "box": (10 + (n % 40) * 9, 60, 20, 20), "style": #{ "bg": "#2563eb", "radius": 4 } }
        ]
        gui.apply(w, scene(0))
        let mut n = 0
        let mut work = 0.0
        let mut going = true
        while going {
          match chan.recv_timeout(ev, #{ "timeout_ms": 30000 }) {
            Ok(e) => {
              if e == () => { going = false }
              else if e.kind == "key" => {
                n = n + 1
                if work > 0.0 => {
                  let t0 = time.monotonic()
                  while time.monotonic() - t0 < work { let _x = 0 }
                }
                gui.apply(w, scene(n))
              }
              else if e.kind == "work" => { work = e.ms }
              // a box moved under a 500 ms transition: the engine draws its
              // frames alone, at the display link's rate
              else if e.kind == "animate" => {
                gui.apply(w, [#{ "key": "m", "parent": "root", "box": (e.x, 100, 40, 40),
                                 "style": #{ "bg": "#16a34a", "radius": 6, "transition": #{ "duration": 500, "curve": "linear" } } }])
              }
              else if e.kind == "test_done" => { going = false }
            },
            Err(_) => { going = false }
          }
        }
        gui.close(w)
        n
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

    /// A key typed into the window: a key down (and its up) sent to it as
    /// AppKit sends a key event.
    fn type_key() {
        on_main(|| {
            let app_cls = AnyClass::get(c"NSApplication").unwrap();
            let app: Retained<AnyObject> = unsafe { msg_send![app_cls, sharedApplication] };
            let wins: Retained<objc2_foundation::NSArray<AnyObject>> =
                unsafe { msg_send![&*app, windows] };
            let Some(w) = wins.to_vec().into_iter().find(|w| {
                let t: Option<Retained<NSString>> = unsafe { msg_send![&**w, title] };
                t.is_some_and(|t| t.to_string() == "olang latency")
            }) else {
                return;
            };
            let num: isize = unsafe { msg_send![&*w, windowNumber] };
            let ev_cls = AnyClass::get(c"NSEvent").unwrap();
            let chars = NSString::from_str("j");
            let none: *const AnyObject = std::ptr::null();
            for (ty, up) in [(10usize, false), (11usize, true)] {
                let _ = up;
                let e: Option<Retained<AnyObject>> = unsafe {
                    msg_send![ev_cls, keyEventWithType: ty, location: objc2_foundation::NSPoint::new(0.0, 0.0),
                        modifierFlags: 0usize, timestamp: 0.0f64, windowNumber: num, context: none,
                        characters: &*chars, charactersIgnoringModifiers: &*chars, isARepeat: false, keyCode: 38u16]
                };
                if let Some(e) = e {
                    let _: () = unsafe { msg_send![&*w, sendEvent: &*e] };
                }
            }
        })
    }

    fn run(what: &str, keys: usize, every: Duration, work_ms: f64) -> Vec<(f64, f64)> {
        olang::stdlib::gui::emit(vec![olang::stdlib::gui::values::event(
            "work",
            vec![("ms", olang::Value::Float(work_ms))],
        )]);
        std::thread::sleep(Duration::from_millis(200));
        frames::samples().lock().unwrap().clear();
        let start = Instant::now();
        for i in 0..keys {
            let at = start + every * i as u32;
            if let Some(d) = at.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
            type_key();
        }
        std::thread::sleep(Duration::from_millis(400));
        let s = frames::samples().lock().unwrap().clone();
        println!("{what}: {} frames timed", s.len());
        frames::report();
        s
    }

    /// A box moved under a 500 ms transition, with no program turn after
    /// the patch: the frames the engine presented while it moved.
    fn animate(x: f64) -> u64 {
        olang::stdlib::gui::emit(vec![olang::stdlib::gui::values::event(
            "animate",
            vec![("x", olang::Value::Float(x))],
        )]);
        std::thread::sleep(Duration::from_millis(30));
        let a = platform::frames_presented();
        std::thread::sleep(Duration::from_millis(500));
        platform::frames_presented() - a
    }

    pub fn main() {
        // SAFETY: set before any thread reads the environment.
        unsafe { std::env::set_var("GUI_LATENCY", "1") };
        olang::stdio::set_panic_hook();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(120));
            eprintln!("gui_macos_latency_test: timed out");
            std::process::exit(2);
        });
        // a run's name and its samples, as run returns them
        type Run = (String, Vec<(f64, f64)>);
        static RUNS: Mutex<Vec<Run>> = Mutex::new(Vec::new());
        static ANIM: Mutex<Vec<u64>> = Mutex::new(Vec::new());
        let code = platform::host(|| {
            std::thread::spawn(|| {
                let prober = std::thread::spawn(|| {
                    let start = Instant::now();
                    while platform::frames_presented() == 0 {
                        assert!(start.elapsed() < Duration::from_secs(20), "no first frame");
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    // in front, so it can be seen (a covered window's display
                    // link rests), without taking the person's keyboard
                    on_main(|| {
                        let app_cls = AnyClass::get(c"NSApplication").unwrap();
                        let app: Retained<AnyObject> =
                            unsafe { msg_send![app_cls, sharedApplication] };
                        let wins: Retained<objc2_foundation::NSArray<AnyObject>> =
                            unsafe { msg_send![&*app, windows] };
                        for w in wins.to_vec() {
                            let t: Option<Retained<NSString>> = unsafe { msg_send![&*w, title] };
                            if t.is_some_and(|t| t.to_string() == "olang latency") {
                                let _: () = unsafe { msg_send![&*w, orderFrontRegardless] };
                            }
                        }
                    });
                    std::thread::sleep(Duration::from_millis(300));
                    let a = run(
                        "typing (a key every 60 ms, a turn of 1 ms)",
                        60,
                        Duration::from_millis(60),
                        1.0,
                    );
                    let b = run(
                        "flood (a key every 4 ms, a turn of 5 ms)",
                        200,
                        Duration::from_millis(4),
                        5.0,
                    );
                    // the box in place, then moved twice
                    animate(10.0);
                    std::thread::sleep(Duration::from_millis(600));
                    let moved: Vec<u64> = [360.0, 10.0]
                        .iter()
                        .map(|x| {
                            let n = animate(*x);
                            std::thread::sleep(Duration::from_millis(200));
                            n
                        })
                        .collect();
                    let hz = std::env::var("GUI_LINK_HZ").unwrap_or_else(|_| "the screen's".into());
                    println!("a 500 ms transition (link at {hz} Hz): {moved:?} frames presented");
                    ANIM.lock().unwrap().extend(moved);
                    RUNS.lock().unwrap().push(("typing".into(), a));
                    RUNS.lock().unwrap().push(("flood".into(), b));
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
        for (what, s) in RUNS.lock().unwrap().iter() {
            assert!(!s.is_empty(), "{what}: no frame was timed");
            let worst = s.iter().map(|x| x.1).fold(0.0, f64::max);
            assert!(
                worst < 250.0,
                "{what}: a frame showed {worst:.1} ms after its input"
            );
        }
        // a transition runs on the display link: frames for its whole
        // length (not one, as a timer-less engine would draw), at least 20
        // in 500 ms at 60 Hz
        for n in ANIM.lock().unwrap().iter() {
            assert!(*n >= 20, "a 500 ms transition presented {n} frames");
        }
        println!("gui_macos_latency_test: ok");
    }
}

fn main() {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    mac::main();
}

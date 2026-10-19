//! A program's long turn on a real window (macOS): the engine answers on
//! its own while the program is busy, and says the program is busy.
//!
//! An olang program opens a window with a scrolling list, then runs a
//! turn of 1.5 s that touches nothing of the engine's (`gui.turn(true)`
//! before, `gui.turn(false)` after, as Loom's loop does). While it runs
//! the test moves the pointer over the list and turns the wheel (AppKit's
//! events, given to the window's view as AppKit gives them): the list scrolls — the
//! engine's, with no turn of the program's — and frames go on being
//! presented. Past the watch's limit (300 ms here) every window says the
//! program is not responding (the spinner turns, frames keep coming);
//! once the turn ends it no longer does.
//!
//! The harness is off: AppKit needs the process's first thread, which
//! `gui::platform::host` takes for winit's loop.

#[cfg(all(feature = "gui", target_os = "macos"))]
mod mac {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSPoint, NSString};
    use olang::stdlib::gui::platform::{self, turn_watch};
    use olang::stdlib::gui::windows;
    use olang::{Interpreter, Parser};
    use std::ffi::c_void;
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    const PROGRAM: &str = r##"
        let ev = gui.events()
        let w = unwrap(gui.open(#{ "title": "olang busy", "size": (300, 200) }))
        let mut rows = []
        for i in 0..200 {
          rows = rows + [#{ "key": "r" + to_string(i), "parent": "l", "box": (0, i * 30, 300, 30), "text": "Row " + to_string(i) }]
        }
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 300, 200), "style": #{ "bg": "#ffffff" } },
          #{ "key": "l", "parent": "root", "role": "list", "box": (0, 0, 300, 200), "scroll": true, "content": (300, 6000) }
        ] + rows)
        // wait for the test to say the window is up, then a long turn
        let mut going = true
        while going {
          match chan.recv_timeout(ev, #{ "timeout_ms": 20000 }) {
            Ok(e) => { if e == () || e.kind == "go" => { going = false } },
            Err(_) => { going = false }
          }
        }
        gui.turn(true)
        let t0 = time.monotonic()
        while time.monotonic() - t0 < 1500.0 { let _x = 0 }
        gui.turn(false)
        let mut done = false
        while !done {
          match chan.recv_timeout(ev, #{ "timeout_ms": 20000 }) {
            Ok(e) => { if e == () || e.kind == "test_done" => { done = true } },
            Err(_) => { done = true }
          }
        }
        gui.close(w)
        "ended"
    "##;

    unsafe extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
        fn CGEventCreateScrollWheelEvent(
            source: *const c_void,
            units: u32,
            wheel_count: u32,
            wheel1: i32, ...
        ) -> *mut c_void;
        fn CFRelease(cf: *const c_void);
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

    fn list_offset(id: u64) -> f32 {
        let st = windows().lock().unwrap().get(&id).cloned().unwrap();
        let st = st.lock().unwrap();
        st.scene.nodes.get("l").map(|n| n.offset.1).unwrap_or(-1.0)
    }

    fn ns_window() -> Retained<AnyObject> {
        let cls = AnyClass::get(c"NSApplication").unwrap();
        let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
        let wins: Retained<objc2_foundation::NSArray<AnyObject>> = unsafe { msg_send![&*app, windows] };
        wins.to_vec()
            .into_iter()
            .find(|w| {
                let t: Option<Retained<NSString>> = unsafe { msg_send![&**w, title] };
                t.is_some_and(|t| t.to_string() == "olang busy")
            })
            .expect("the window")
    }

    /// The pointer moved over the list, then the wheel turned down, as
    /// AppKit's events sent to the window.
    fn wheel_down() {
        on_main(|| {
            let w = ns_window();
            let num: isize = unsafe { msg_send![&*w, windowNumber] };
            let ev_cls = AnyClass::get(c"NSEvent").unwrap();
            let none: *const AnyObject = std::ptr::null();
            let mv: Option<Retained<AnyObject>> = unsafe {
                msg_send![ev_cls, mouseEventWithType: 5usize, location: NSPoint::new(150.0, 100.0),
                    modifierFlags: 0usize, timestamp: 0.0f64, windowNumber: num, context: none,
                    eventNumber: 0isize, clickCount: 0isize, pressure: 0.0f32]
            };
            // to winit's view itself (the window would route by where the
            // person's pointer is)
            let view: Retained<AnyObject> = unsafe { msg_send![&*w, contentView] };
            if let Some(mv) = mv {
                let _: () = unsafe { msg_send![&*view, mouseMoved: &*mv] };
            }
            // SAFETY: CoreGraphics' scroll event, pixels (0), one wheel.
            let cg = unsafe { CGEventCreateScrollWheelEvent(std::ptr::null(), 0, 1, -60) };
            if cg.is_null() {
                return;
            }
            let ev: Option<Retained<AnyObject>> = unsafe { msg_send![ev_cls, eventWithCGEvent: cg] };
            unsafe { CFRelease(cg) };
            if let Some(ev) = ev {
                let _: () = unsafe { msg_send![&*view, scrollWheel: &*ev] };
            }
        })
    }

    pub fn main() {
        // SAFETY: set before any thread reads the environment.
        unsafe { std::env::set_var("GUI_NOT_RESPONDING_MS", "300") };
        olang::stdio::set_panic_hook();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(60));
            eprintln!("gui_macos_busy_test: timed out");
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
                    std::thread::sleep(Duration::from_millis(200));
                    olang::stdlib::gui::emit(vec![olang::stdlib::gui::values::event("go", vec![])]);
                    // the turn runs: the list scrolls by the wheel, frames go on
                    std::thread::sleep(Duration::from_millis(100));
                    let before = list_offset(id);
                    let frames0 = platform::frames_presented();
                    wheel_down();
                    let t = Instant::now();
                    while list_offset(id) == before && t.elapsed() < Duration::from_millis(500) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    let after = list_offset(id);
                    say(format!("scrolled while busy: {}", after > before));
                    // past the limit: the windows say so, and the spinner turns
                    let t = Instant::now();
                    while !turn_watch::shown() && t.elapsed() < Duration::from_millis(1200) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    say(format!("shown while busy: {}", turn_watch::shown()));
                    let f1 = platform::frames_presented();
                    std::thread::sleep(Duration::from_millis(400));
                    let turning = platform::frames_presented() - f1;
                    say(format!("frames while busy: {}", platform::frames_presented() > frames0 && turning >= 2));
                    println!("frames presented during the turn: {} (spinner: {turning} in 400 ms)", platform::frames_presented() - frames0);
                    // the turn ends: no longer said
                    let t = Instant::now();
                    while turn_watch::shown() && t.elapsed() < Duration::from_secs(3) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    say(format!("shown after: {}", turn_watch::shown()));
                    olang::stdlib::gui::emit(vec![olang::stdlib::gui::values::event("test_done", vec![])]);
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
                "scrolled while busy: true",
                "shown while busy: true",
                "frames while busy: true",
                "shown after: false"
            ]
        );
        println!("gui_macos_busy_test: ok");
    }
}

fn main() {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    mac::main();
}

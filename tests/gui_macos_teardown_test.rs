//! Real windows closed one after another go away (macOS).
//!
//! An olang program opens four platform windows in turn — each with
//! accessibility nodes that have actions, its title changed and marked
//! edited, drawn — and closes each before opening the next, as an editor
//! does. A closed window must leave AppKit (it is no longer among the
//! application's windows, so it was deallocated), the first one included:
//! the process's GPU instance once kept the first window for good, so it
//! stayed on the screen after the program closed it and was torn down by
//! winit as the loop ended, long after its accessibility adapter. The
//! process must end with the program's status.
//!
//! The harness is off: AppKit needs the process's first thread, which
//! `gui::platform::host` takes for winit's loop.

#[cfg(all(feature = "gui", target_os = "macos"))]
mod mac {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSArray, NSString};
    use olang::{Interpreter, Parser};
    use std::ffi::c_void;
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    const WINDOWS: usize = 4;

    const PROGRAM: &str = r##"
        let ev = gui.events()
        for i in 0..4 {
          let w = unwrap(gui.open(#{ "title": "olang teardown " + to_string(i), "size": (320, 200) }))
          gui.apply(w, [
            #{ "key": "root", "box": (0, 0, 320, 200), "style": #{ "bg": "#ffffff" } },
            #{ "key": "card", "parent": "root", "role": "button", "box": (10, 10, 140, 40), "text": "Card",
               "focusable": true, "actions": ["Move left", "Move right"], "description": "a card" },
            #{ "key": "list", "parent": "root", "role": "list", "box": (170, 10, 140, 120), "name": "a list",
               "focusable": true, "active": "row1", "actions": ["Sort"] },
            #{ "key": "row0", "parent": "list", "role": "listitem", "box": (0, 0, 140, 40), "text": "Row zero" },
            #{ "key": "row1", "parent": "list", "role": "listitem", "box": (0, 40, 140, 40), "text": "Row one" }
          ])
          gui.set(w, #{ "title": "olang teardown " + to_string(i), "edited": true })
          let t0 = time.monotonic()
          while time.monotonic() - t0 < 400.0 {
            let _e = chan.recv_timeout(ev, 50)
          }
          gui.set(w, #{ "edited": false })
          gui.close(w)
        }
        // the loop runs on while the test looks
        let _s = time.sleep(2500)
        "closed"
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

    fn class_name(obj: &AnyObject) -> String {
        obj.class().name().to_string_lossy().into_owned()
    }

    /// The program's windows AppKit still has: their titles, whether each
    /// is on the screen, and its content view's class. A content view
    /// that has gone is said as "-".
    fn ours() -> Vec<(String, bool, String)> {
        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return vec![];
        };
        let app: Option<Retained<AnyObject>> = unsafe { msg_send![cls, sharedApplication] };
        let Some(app) = app else { return vec![] };
        let windows: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![&*app, windows] };
        let mut out = vec![];
        for w in windows.map(|a| a.to_vec()).unwrap_or_default() {
            let t: Option<Retained<NSString>> = unsafe { msg_send![&*w, title] };
            let t = t.map(|t| t.to_string()).unwrap_or_default();
            if !t.starts_with("olang teardown") {
                continue;
            }
            let visible: bool = unsafe { msg_send![&*w, isVisible] };
            let view: Option<Retained<AnyObject>> = unsafe { msg_send![&*w, contentView] };
            out.push((
                t,
                visible,
                view.map(|v| class_name(&v)).unwrap_or("-".into()),
            ));
        }
        out
    }

    pub fn main() {
        // Never hang: a wedged run fails.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(60));
            eprintln!("gui_macos_teardown_test: timed out");
            std::process::exit(2);
        });
        static RESULT: Mutex<Option<String>> = Mutex::new(None);
        static SEEN: Mutex<Option<String>> = Mutex::new(None);
        let code = olang::stdlib::gui::platform::host(|| {
            std::thread::spawn(|| {
                let prober = std::thread::spawn(|| {
                    // Watch the windows come and go: every title seen, and
                    // at most one of the program's windows at a time (a
                    // closed one leaves before the next has been up long).
                    let start = Instant::now();
                    let mut seen: Vec<String> = vec![];
                    let mut most_at_once = 0;
                    let mut lingering = vec![];
                    loop {
                        let now = on_main(ours);
                        for (t, _, _) in &now {
                            if !seen.contains(t) {
                                seen.push(t.clone());
                            }
                        }
                        let up = now.iter().filter(|(_, v, _)| *v).count();
                        most_at_once = most_at_once.max(up);
                        if seen.len() == WINDOWS && now.is_empty() {
                            break;
                        }
                        if start.elapsed() > Duration::from_secs(8) {
                            lingering = now;
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    seen.sort();
                    *SEEN.lock().unwrap() = Some(format!(
                        "seen={seen:?}\nmost visible at once={most_at_once}\nlingering={lingering:?}"
                    ));
                });
                let program = Parser::new().parse(PROGRAM).expect("parses");
                let v = Interpreter::new().eval_program(program);
                let _ = prober.join();
                *RESULT.lock().unwrap() = Some(match v {
                    Ok(v) => format!("{v}"),
                    Err(e) => format!("error: {e}"),
                });
                0
            })
        });
        let seen = SEEN.lock().unwrap().take().unwrap_or_default();
        let result = RESULT.lock().unwrap().take().unwrap_or_default();
        println!("{seen}");
        println!("program: {result}");
        assert_eq!(code, 0);
        assert_eq!(result, r#""closed""#);
        let titles: Vec<String> = (0..WINDOWS)
            .map(|i| format!("{:?}", format!("olang teardown {i}")))
            .collect();
        assert!(
            seen.lines()
                .any(|l| l == format!("seen=[{}]", titles.join(", "))),
            "not every window was seen:\n{seen}"
        );
        assert!(
            seen.lines().any(|l| l == "lingering=[]"),
            "a closed window stayed with AppKit:\n{seen}"
        );
        assert!(
            seen.lines().any(|l| l == "most visible at once=1"),
            "a closed window stayed on the screen while the next was open:\n{seen}"
        );
        println!("gui_macos_teardown_test: ok");
    }
}

fn main() {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    mac::main();
}

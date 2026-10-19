//! The text input client on a real window (macOS): what AppKit asks a
//! view that edits text (NSTextInputClient), answered for the focused
//! field.
//!
//! An olang program opens a window with a focused field. The test, on the
//! main thread as AppKit would:
//! - puts text in outside a key press and a composition, as dictation and
//!   the Character Viewer do (`insertText:replacementRange:`): it reaches
//!   the field (winit took text only to end a composition, so dictated
//!   text was dropped), and over a replacement range it replaces;
//! - asks the selection (`selectedRange`), a range's text
//!   (`attributedSubstringForProposedRange:actualRange:`), its place on
//!   the screen (`firstRectForCharacterRange:actualRange:`) and the
//!   character under a point (`characterIndexForPoint:`) — what Look Up
//!   (⌃⌘D) and an input method read;
//! - composes (`setMarkedText:…`, then `insertText:`): the composition is
//!   committed once, as before;
//! - and finds the view answers a force click (`quickLookWithEvent:`).
//!
//! The harness is off: AppKit needs the process's first thread.

#[cfg(all(feature = "gui", target_os = "macos"))]
mod mac {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
    use objc2_foundation::NSString;
    use olang::stdlib::gui::platform::edit_menu::{Point, Range, Rect};
    use olang::stdlib::gui::{platform, windows};
    use olang::{Interpreter, Parser};
    use std::ffi::c_void;
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    const PROGRAM: &str = r##"
        let ev = gui.events()
        let w = unwrap(gui.open(#{ "title": "olang text input", "size": (360, 120) }))
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 360, 120), "style": #{ "bg": "#ffffff" } },
          #{ "key": "f", "parent": "root", "role": "input", "box": (10, 10, 300, 30), "edit": #{ "value": "hello world" } },
          #{ "op": "focus", "key": "f" }
        ])
        let mut going = true
        let t0 = time.monotonic()
        while going && time.monotonic() - t0 < 40000.0 {
          match chan.recv_timeout(ev, #{ "timeout_ms": 100 }) {
            Ok(e) => { if e != () && e.kind == "test_done" => { going = false } },
            Err(_) => ()
          }
        }
        gui.close(w)
    "##;

    unsafe extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
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
        rx.recv_timeout(Duration::from_secs(10)).expect("the main thread answers")
    }

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let start = Instant::now();
        while !ok() {
            if start.elapsed() > Duration::from_secs(10) {
                panic!("timed out waiting for {what}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn field_value(id: u64) -> String {
        let st = windows().lock().unwrap().get(&id).cloned().unwrap();
        let st = st.lock().unwrap();
        st.editors.get("f").map(|e| e.value()).unwrap_or_default()
    }

    fn view() -> Retained<AnyObject> {
        let cls = AnyClass::get(c"NSApplication").unwrap();
        let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
        let wins: Retained<objc2_foundation::NSArray<AnyObject>> = unsafe { msg_send![&*app, windows] };
        for w in wins.to_vec() {
            let t: Option<Retained<NSString>> = unsafe { msg_send![&*w, title] };
            if t.is_some_and(|t| t.to_string() == "olang text input") {
                let v: Option<Retained<AnyObject>> = unsafe { msg_send![&*w, contentView] };
                return v.expect("a content view");
            }
        }
        panic!("the window is not among AppKit's")
    }

    const NOT_FOUND: usize = isize::MAX as usize;

    fn insert(text: &'static str, range: Range) {
        on_main(move || {
            let v = view();
            let s = NSString::from_str(text);
            let _: () = unsafe { msg_send![&*v, insertText: &*s, replacementRange: range] };
        })
    }

    pub fn main() {
        olang::stdio::set_panic_hook();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(90));
            eprintln!("gui_macos_text_input_test: timed out");
            std::process::exit(2);
        });
        static CHECKS: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let code = platform::host(|| {
            std::thread::spawn(|| {
                let prober = std::thread::spawn(|| {
                    let say = |s: String| CHECKS.lock().unwrap().push(s);
                    wait_for("the window's first frame", || platform::frames_presented() > 0);
                    let id = *windows().lock().unwrap().keys().next().expect("a window");
                    // dictation: text put in with no key and no composition
                    // (the caret at the end, where focusing put it)
                    on_main(|| {
                        let v = view();
                        let _: () = unsafe { msg_send![&*v, selectAll: std::ptr::null::<AnyObject>()] };
                    });
                    wait_for("all selected", || {
                        let st = windows().lock().unwrap().get(&id).cloned().unwrap();
                        let st = st.lock().unwrap();
                        st.editors.get("f").is_some_and(|e| e.selection_chars() == (0, 11))
                    });
                    insert("dictated words", Range { location: NOT_FOUND, length: 0 });
                    wait_for("the dictated text", || field_value(id) == "dictated words");
                    say(format!("dictated: {:?}", field_value(id)));
                    // over a range: replaced
                    insert("spoken", Range { location: 0, length: 8 });
                    wait_for("the replaced text", || field_value(id) == "spoken words");
                    say(format!("replaced: {:?}", field_value(id)));
                    // what Look Up and an input method read
                    let (sel, sub, actual, rect, frame, at) = on_main(|| {
                        let v = view();
                        let sel: Range = unsafe { msg_send![&*v, selectedRange] };
                        let mut actual = Range { location: 0, length: 0 };
                        let sub: Option<Retained<AnyObject>> = unsafe {
                            msg_send![&*v, attributedSubstringForProposedRange: Range { location: 7, length: 5 }, actualRange: &mut actual]
                        };
                        let sub = sub.map(|s| {
                            let st: Retained<NSString> = unsafe { msg_send![&*s, string] };
                            st.to_string()
                        });
                        let mut a2 = Range { location: 0, length: 0 };
                        let rect: Rect = unsafe {
                            msg_send![&*v, firstRectForCharacterRange: Range { location: 7, length: 5 }, actualRange: &mut a2]
                        };
                        let win: Retained<AnyObject> = unsafe { msg_send![&*v, window] };
                        let frame: Rect = unsafe { msg_send![&*win, frame] };
                        let mid = Point { x: rect.origin.x + rect.size.x / 2.0, y: rect.origin.y + rect.size.y / 2.0 };
                        let at: usize = unsafe { msg_send![&*v, characterIndexForPoint: mid] };
                        (sel, sub, actual, rect, frame, at)
                    });
                    say(format!("selected: {} {}", sel.location, sel.length));
                    say(format!("substring: {sub:?} {} {}", actual.location, actual.length));
                    let inside = rect.size.x > 10.0
                        && rect.size.y > 5.0
                        && rect.origin.x >= frame.origin.x
                        && rect.origin.x + rect.size.x <= frame.origin.x + frame.size.x
                        && rect.origin.y >= frame.origin.y
                        && rect.origin.y + rect.size.y <= frame.origin.y + frame.size.y;
                    say(format!("rect on the window: {inside}"));
                    say(format!("index under the rect: {}", (7..=12).contains(&at)));
                    // a composition is committed once, as winit did
                    on_main(|| {
                        let v = view();
                        let s = NSString::from_str("か");
                        let none = Range { location: NOT_FOUND, length: 0 };
                        let _: () = unsafe {
                            msg_send![&*v, setMarkedText: &*s, selectedRange: Range { location: 1, length: 0 }, replacementRange: none]
                        };
                        let _: () = unsafe { msg_send![&*v, insertText: &*s, replacementRange: none] };
                    });
                    wait_for("the composed text", || field_value(id).contains('か'));
                    std::thread::sleep(Duration::from_millis(200));
                    say(format!("composed: {}", field_value(id).matches('か').count()));
                    let looks: bool = on_main(|| {
                        let v = view();
                        let b: Bool = unsafe { msg_send![&*v, respondsToSelector: Sel::register(c"quickLookWithEvent:")] };
                        b.as_bool()
                    });
                    say(format!("force click looks up: {looks}"));
                    olang::stdlib::gui::emit(vec![olang::stdlib::gui::values::event("test_done", vec![])]);
                });
                let program = Parser::new().parse(PROGRAM).expect("parses");
                let _ = Interpreter::new().eval_program(program);
                let _ = prober.join();
                0
            })
        });
        let checks = CHECKS.lock().unwrap().clone();
        for c in &checks {
            println!("{c}");
        }
        assert_eq!(code, 0);
        let want = [
            r#"dictated: "dictated words""#,
            r#"replaced: "spoken words""#,
            "selected: 6 0",
            r#"substring: Some("words") 7 5"#,
            "rect on the window: true",
            "index under the rect: true",
            "composed: 1",
            "force click looks up: true",
        ];
        assert_eq!(checks, want.map(String::from).to_vec());
        println!("gui_macos_text_input_test: ok");
    }
}

fn main() {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    mac::main();
}

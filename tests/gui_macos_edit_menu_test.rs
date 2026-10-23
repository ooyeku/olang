//! The Mac's standard Edit menu and Services on a real window (macOS).
//!
//! An olang program opens a window with a text field and a list and sets
//! a menu bar whose Edit menu holds the standard items (`gui.menu`'s
//! `standard`). The test, on the main thread as AppKit would, finds the
//! items AppKit shows (titles, selectors, keys), asks the window's view
//! to validate them (`validateMenuItem:`, as the menu does before it
//! opens or takes a key) and sends them their actions (as choosing them
//! does): with the field focused they edit it (Select All, then Delete
//! empties it, Undo brings the text back); with the list focused, Copy
//! is the program's — it hears ⌘C as a key, which its command for that
//! chord in the list's context takes — and Undo, which no command binds
//! there, is greyed. Services: the view offers a field's selection
//! (`validRequestorForSendType:returnType:`, `writeSelectionToPasteboard:
//! types:`) and takes a Service's text into the field
//! (`readSelectionFromPasteboard:`), on pasteboards of the test's own (the
//! person's clipboard is not touched).
//!
//! The harness is off: AppKit needs the process's first thread, which
//! `gui::platform::host` takes for winit's loop.

#[cfg(all(feature = "gui", target_os = "macos"))]
mod mac {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
    use objc2_foundation::NSString;
    use olang::stdlib::gui::{platform, windows};
    use olang::{Interpreter, Parser, Value};
    use std::ffi::c_void;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    const PROGRAM: &str = r##"
        let ev = gui.events()
        let w = unwrap(gui.open(#{ "title": "olang edit menu", "size": (320, 200) }))
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 320, 200), "style": #{ "bg": "#ffffff" } },
          #{ "key": "f", "parent": "root", "role": "input", "box": (10, 10, 200, 30), "edit": #{ "value": "hello world" } },
          #{ "key": "l", "parent": "root", "role": "list", "box": (10, 60, 200, 100), "focusable": true },
          #{ "op": "focus", "key": "f" }
        ])
        let _m = gui.menu([
          #{ "title": "olang", "items": [#{ "id": "quit", "label": "Quit", "keys": "mod+q" }] },
          #{ "title": "Edit", "items": [
            #{ "standard": "undo", "contexts": [] }, #{ "standard": "redo", "contexts": [] }, "separator",
            #{ "standard": "cut", "contexts": [] }, #{ "standard": "copy", "contexts": ["list"] },
            #{ "standard": "paste", "contexts": [] }, #{ "standard": "paste_plain", "contexts": [] },
            #{ "standard": "delete", "contexts": [] }, #{ "standard": "select_all", "contexts": [] },
            "separator",
            #{ "label": "Find", "submenu": [#{ "id": "find", "label": "Find", "keys": "mod+f" }] },
            #{ "id": "comment", "label": "Toggle Comment", "keys": "mod+/" }] }])
        let mut keys = []
        let mut going = true
        let t0 = time.monotonic()
        while going && time.monotonic() - t0 < 40000.0 {
          match chan.recv_timeout(ev, #{ "timeout_ms": 100 }) {
            Ok(e) => {
              if e != () && e.kind == "key" => { keys = keys + [e.mod_chord] }
              if e != () && e.kind == "test_done" => { going = false }
            },
            Err(_) => ()
          }
        }
        gui.close(w)
        keys
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

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let start = Instant::now();
        while !ok() {
            if start.elapsed() > Duration::from_secs(10) {
                panic!("timed out waiting for {what}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn the_window() -> u64 {
        *windows().lock().unwrap().keys().next().expect("a window")
    }

    fn field_value(id: u64) -> String {
        let st = windows().lock().unwrap().get(&id).cloned().unwrap();
        let st = st.lock().unwrap();
        st.editors.get("f").map(|e| e.value()).unwrap_or_default()
    }

    fn focus(id: u64, key: &str) {
        let st = windows().lock().unwrap().get(&id).cloned().unwrap();
        let op = olang::stdlib::gui::values::map(vec![
            ("op", Value::String(Arc::new("focus".into()))),
            ("key", Value::String(Arc::new(key.into()))),
        ]);
        st.lock()
            .unwrap()
            .apply(&Value::List(Arc::new(vec![op])), &mut vec![])
            .unwrap();
    }

    /// Our window's content view (winit's, with AccessKit's class on it).
    fn view() -> Retained<AnyObject> {
        let cls = AnyClass::get(c"NSApplication").unwrap();
        let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
        let wins: Retained<objc2_foundation::NSArray<AnyObject>> =
            unsafe { msg_send![&*app, windows] };
        for w in wins.to_vec() {
            let t: Option<Retained<NSString>> = unsafe { msg_send![&*w, title] };
            if t.is_some_and(|t| t.to_string() == "olang edit menu") {
                let v: Option<Retained<AnyObject>> = unsafe { msg_send![&*w, contentView] };
                return v.expect("a content view");
            }
        }
        panic!("the window is not among AppKit's")
    }

    /// The Edit menu's items as AppKit shows them: (title, selector, key
    /// equivalent, modifier mask), a separator as "-".
    fn edit_items() -> Vec<(String, String, String, usize)> {
        let cls = AnyClass::get(c"NSApplication").unwrap();
        let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
        let bar: Retained<AnyObject> = unsafe { msg_send![&*app, mainMenu] };
        let n: isize = unsafe { msg_send![&*bar, numberOfItems] };
        for i in 0..n {
            let top: Retained<AnyObject> = unsafe { msg_send![&*bar, itemAtIndex: i] };
            let sub: Option<Retained<AnyObject>> = unsafe { msg_send![&*top, submenu] };
            let Some(sub) = sub else { continue };
            let t: Retained<NSString> = unsafe { msg_send![&*sub, title] };
            if t.to_string() != "Edit" {
                continue;
            }
            let k: isize = unsafe { msg_send![&*sub, numberOfItems] };
            return (0..k)
                .map(|j| {
                    let it: Retained<AnyObject> = unsafe { msg_send![&*sub, itemAtIndex: j] };
                    let sep: bool = unsafe { msg_send![&*it, isSeparatorItem] };
                    let hidden: bool = unsafe { msg_send![&*it, isHidden] };
                    let alternate: bool = unsafe { msg_send![&*it, isAlternate] };
                    if hidden || alternate {
                        return ("(hidden)".into(), String::new(), String::new(), 0);
                    }
                    if sep {
                        return ("-".into(), String::new(), String::new(), 0);
                    }
                    let t: Retained<NSString> = unsafe { msg_send![&*it, title] };
                    let a: Option<Sel> = unsafe { msg_send![&*it, action] };
                    let key: Retained<NSString> = unsafe { msg_send![&*it, keyEquivalent] };
                    let mask: usize = unsafe { msg_send![&*it, keyEquivalentModifierMask] };
                    (
                        t.to_string(),
                        a.map(|a| a.name().to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        key.to_string(),
                        mask,
                    )
                })
                .collect();
        }
        vec![]
    }

    /// Whether the view enables the standard item with `sel`.
    fn enabled(sel: &'static std::ffi::CStr) -> bool {
        on_main(move || {
            let v = view();
            let cls = AnyClass::get(c"NSMenuItem").unwrap();
            let title = NSString::from_str("probe");
            let key = NSString::from_str("");
            let alloc: objc2::rc::Allocated<AnyObject> = unsafe { msg_send![cls, alloc] };
            let it: Retained<AnyObject> = unsafe {
                msg_send![alloc, initWithTitle: &*title, action: Sel::register(sel), keyEquivalent: &*key]
            };
            let b: Bool = unsafe { msg_send![&*v, validateMenuItem: &*it] };
            b.as_bool()
        })
    }

    /// Choose the standard item with `sel` (its action sent to the view).
    fn choose(sel: &'static std::ffi::CStr) {
        on_main(move || {
            let v = view();
            let none: *const AnyObject = std::ptr::null();
            let s = Sel::register(sel);
            let ok: Bool = unsafe { msg_send![&*v, respondsToSelector: s] };
            assert!(ok.as_bool(), "the view answers {sel:?}");
            let _: () = unsafe { msg_send![&*v, performSelector: s, withObject: none] };
        })
    }

    fn pasteboard() -> Retained<AnyObject> {
        let cls = AnyClass::get(c"NSPasteboard").unwrap();
        unsafe { msg_send![cls, pasteboardWithUniqueName] }
    }

    pub fn main() {
        olang::stdio::set_panic_hook();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(90));
            eprintln!("gui_macos_edit_menu_test: timed out");
            std::process::exit(2);
        });
        static RESULT: Mutex<Option<String>> = Mutex::new(None);
        static CHECKS: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let code = platform::host(|| {
            std::thread::spawn(|| {
                let prober = std::thread::spawn(|| {
                    let say = |s: String| CHECKS.lock().unwrap().push(s);
                    wait_for("the window's first frame", || {
                        platform::frames_presented() > 0
                    });
                    let id = the_window();
                    wait_for("the Edit menu", || on_main(|| edit_items().len() > 9));
                    let items = on_main(edit_items);
                    // ours, then what AppKit adds to an Edit menu (AutoFill,
                    // Start Dictation…, Emoji & Symbols), each shown once
                    // (their other keys are hidden alternates)
                    let titles: Vec<&str> = items.iter().map(|i| i.0.as_str()).collect();
                    let ours = titles
                        .iter()
                        .position(|t| *t == "Toggle Comment")
                        .map_or(titles.len(), |i| i + 1);
                    say(format!("titles={:?}", &titles[..ours]));
                    let extra: Vec<&str> = titles[ours..]
                        .iter()
                        .copied()
                        .filter(|t| *t != "-" && *t != "(hidden)")
                        .collect();
                    let mut once = extra.clone();
                    once.sort();
                    once.dedup();
                    println!("AppKit's own: {extra:?}");
                    assert_eq!(
                        once.len(),
                        extra.len(),
                        "AppKit's items added more than once: {extra:?}"
                    );
                    let copy = items.iter().find(|i| i.0 == "Copy").cloned().unwrap();
                    say(format!("copy={}/{}/{}", copy.1, copy.2, copy.3 == 1 << 20));
                    let pms = items
                        .iter()
                        .find(|i| i.0 == "Paste and Match Style")
                        .cloned()
                        .unwrap();
                    say(format!(
                        "paste_plain={}/{}",
                        pms.1,
                        pms.3 == (1 << 20) | (1 << 19) | (1 << 17)
                    ));
                    // the field focused, nothing selected
                    say(format!(
                        "field: copy={} cut={} delete={} paste={} select_all={} undo={}",
                        enabled(c"copy:"),
                        enabled(c"cut:"),
                        enabled(c"delete:"),
                        enabled(c"paste:"),
                        enabled(c"selectAll:"),
                        enabled(c"undo:")
                    ));
                    choose(c"selectAll:");
                    wait_for("a selection", || enabled(c"copy:"));
                    say(format!(
                        "selected: copy={} delete={}",
                        enabled(c"copy:"),
                        enabled(c"delete:")
                    ));
                    // Services: the selection offered, and written
                    let offered = on_main(|| {
                        let v = view();
                        let t = NSString::from_str("public.utf8-plain-text");
                        let none: *const AnyObject = std::ptr::null();
                        let r: *mut AnyObject = unsafe {
                            msg_send![&*v, validRequestorForSendType: &*t, returnType: none]
                        };
                        let pb = pasteboard();
                        let types = objc2_foundation::NSArray::from_retained_slice(
                            std::slice::from_ref(&t),
                        );
                        let w: Bool = unsafe {
                            msg_send![&*v, writeSelectionToPasteboard: &*pb, types: &*types]
                        };
                        let got: Option<Retained<NSString>> =
                            unsafe { msg_send![&*pb, stringForType: &*t] };
                        let _: () = unsafe { msg_send![&*pb, releaseGlobally] };
                        (
                            std::ptr::eq(r, Retained::as_ptr(&v)),
                            w.as_bool(),
                            got.map(|g| g.to_string()),
                        )
                    });
                    say(format!("services send: {offered:?}"));
                    choose(c"delete:");
                    wait_for("the field emptied", || field_value(id).is_empty());
                    choose(c"undo:");
                    wait_for("the text back", || field_value(id) == "hello world");
                    say(format!("after delete and undo: {:?}", field_value(id)));
                    // a Service's text into the field (its selection replaced)
                    choose(c"selectAll:");
                    wait_for("a selection", || enabled(c"copy:"));
                    let took = on_main(|| {
                        let v = view();
                        let pb = pasteboard();
                        let t = NSString::from_str("public.utf8-plain-text");
                        let text = NSString::from_str("HELLO WORLD");
                        let _: isize = unsafe { msg_send![&*pb, clearContents] };
                        let _: Bool = unsafe { msg_send![&*pb, setString: &*text, forType: &*t] };
                        let r: Bool = unsafe { msg_send![&*v, readSelectionFromPasteboard: &*pb] };
                        let _: () = unsafe { msg_send![&*pb, releaseGlobally] };
                        r.as_bool()
                    });
                    wait_for("the Service's text", || field_value(id) == "HELLO WORLD");
                    say(format!("services take: {took} {:?}", field_value(id)));
                    // the list focused: Copy is the program's (its command
                    // binds ⌘C in the list's context); Undo has no command
                    focus(id, "l");
                    say(format!(
                        "list: copy={} undo={} paste={}",
                        enabled(c"copy:"),
                        enabled(c"undo:"),
                        enabled(c"paste:")
                    ));
                    choose(c"copy:");
                    std::thread::sleep(Duration::from_millis(300));
                    olang::stdlib::gui::emit(vec![olang::stdlib::gui::values::event(
                        "test_done",
                        vec![],
                    )]);
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
        let result = RESULT.lock().unwrap().take().unwrap_or_default();
        let checks = CHECKS.lock().unwrap().clone();
        for c in &checks {
            println!("{c}");
        }
        println!("program: {result}");
        assert_eq!(code, 0);
        let want = [
            r#"titles=["Undo", "Redo", "-", "Cut", "Copy", "Paste", "Paste and Match Style", "Delete", "Select All", "-", "Find", "-", "Toggle Comment"]"#,
            "copy=copy:/c/true",
            "paste_plain=pasteAsPlainText:/true",
            "field: copy=false cut=false delete=false paste=true select_all=true undo=true",
            "selected: copy=true delete=true",
            r#"services send: (true, true, Some("hello world"))"#,
            r#"after delete and undo: "hello world""#,
            r#"services take: true "HELLO WORLD""#,
            "list: copy=true undo=false paste=false",
        ];
        assert_eq!(checks, want.map(String::from).to_vec());
        // the list's Copy reached the program as ⌘C
        assert_eq!(result, r#"["mod+c"]"#);
        println!("gui_macos_edit_menu_test: ok");
    }
}

fn main() {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    mac::main();
}

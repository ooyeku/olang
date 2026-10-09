//! A real window's node actions, as VoiceOver finds them (macOS).
//!
//! An olang program opens a platform window whose card has actions of its
//! own. The test, on the main thread's run loop as AppKit's accessibility
//! server would be, walks the window's NSAccessibility elements to the
//! card, reads its `accessibilityCustomActions` (what VoiceOver's Actions
//! rotor lists), and runs one; the program must hear the same `a11y`
//! `custom` event that a keyboard-free assistive request sends.
//!
//! The same window hears the system's settings live: the test posts, in
//! this process, the notice AppKit sends when an accessibility display
//! option changes (`NSWorkspaceAccessibilityDisplayOptionsDidChange`;
//! the person's settings are not touched), and the program must hear an
//! `appearance` event for it.
//!
//! When this process is trusted for Accessibility, the actions are also
//! read through the AX client API (`AXUIElementCopyActionNames`), the way
//! VoiceOver reads another process; untrusted, that half says so and is
//! skipped.
//!
//! The harness is off: AppKit needs the process's first thread, which
//! `gui::platform::host` takes for winit's loop.

#[cfg(all(feature = "gui", target_os = "macos"))]
mod mac {
    use block2::Block;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::msg_send;
    use objc2_foundation::{NSArray, NSString};
    use olang::{Interpreter, Parser};
    use std::ffi::c_void;
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    const TITLE: &str = "olang a11y actions";

    const PROGRAM: &str = r##"
        let w = match gui.open(#{ "title": "olang a11y actions", "size": (320, 200) }) {
          Ok(w) => w,
          Err(e) => e
        }
        let ev = gui.events()
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 320, 200), "style": #{ "bg": "#ffffff" } },
          #{ "key": "card", "parent": "root", "role": "button", "box": (10, 10, 140, 40), "text": "Card",
             "focusable": true, "actions": ["Move to To do", "Move to In progress"],
             "description": "Backlog, 1 of 3" },
          #{ "key": "plain", "parent": "root", "role": "button", "box": (10, 60, 140, 40), "text": "Plain" },
          #{ "key": "col", "parent": "root", "role": "list", "box": (170, 10, 140, 120), "name": "In progress, 2 issues",
             "focusable": true, "active": "row1", "actions": ["Move to Backlog", "Move to Done"] },
          #{ "key": "row0", "parent": "col", "role": "listitem", "box": (0, 0, 140, 40), "text": "Row zero" },
          #{ "key": "row1", "parent": "col", "role": "listitem", "box": (0, 40, 140, 40), "text": "Row one" },
          #{ "key": "tabs", "parent": "root", "role": "tablist", "box": (10, 150, 300, 30), "name": "Sections" },
          #{ "key": "tab0", "parent": "tabs", "role": "tab", "box": (0, 0, 140, 30), "text": "Editor tab", "selected": false },
          #{ "key": "tab1", "parent": "tabs", "role": "tab", "box": (150, 0, 140, 30), "text": "Commands tab", "selected": true }
        ])
        let mut got = []
        let mut notices = 0
        let mut tries = 0
        let mut moved = false
        let mut done = false
        while !done && tries < 150 {
          tries = tries + 1
          match chan.recv_timeout(ev, 200) {
            Ok(e) => {
              if map_get(e, "kind") == "appearance" && map_get(e, "why") == "settings" => { notices = notices + 1 } else => ()
              if map_get(e, "kind") == "a11y" && map_get(e, "action") == "custom" => {
                got = got + [[map_get(e, "key"), map_get(e, "index"), map_get(e, "label")]]
                if moved => { done = true } else => ()
                if map_get(e, "label") == "Move to Done" => {
                  moved = true
                  // what Loom's announce(text) places: a polite live region
                  gui.apply(w, [#{ "key": "say1", "parent": "root", "role": "status", "name": "Row one moved to Done",
                                   "live": "polite", "inert": true, "box": (0, 0, 1, 1) }])
                } else => ()
              }
            },
            _ => ()
          }
        }
        gui.close(w)
        // AppKit answers the notice with its own (the system's colours are
        // read again): one or more
        got + [["settings heard", notices > 0]]
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

    fn string(obj: &AnyObject, sel: &str) -> Option<String> {
        let s: Option<Retained<NSString>> = match sel {
            "title" => unsafe { msg_send![obj, accessibilityTitle] },
            "help" => unsafe { msg_send![obj, accessibilityHelp] },
            "name" => unsafe { msg_send![obj, name] },
            _ => None,
        };
        s.map(|s| s.to_string())
    }

    fn children(obj: &AnyObject) -> Vec<Retained<AnyObject>> {
        let a: Option<Retained<NSArray<AnyObject>>> =
            unsafe { msg_send![obj, accessibilityChildren] };
        a.map(|a| a.to_vec()).unwrap_or_default()
    }

    fn find(obj: &AnyObject, title: &str, depth: usize) -> Option<Retained<AnyObject>> {
        for c in children(obj) {
            if string(&c, "title").as_deref() == Some(title) {
                return Some(c);
            }
            if depth > 0
                && let Some(f) = find(&c, title, depth - 1)
            {
                return Some(f);
            }
        }
        None
    }

    /// The content view of the program's window.
    fn content_view() -> Option<Retained<AnyObject>> {
        let cls = AnyClass::get(c"NSApplication")?;
        let app: Option<Retained<AnyObject>> = unsafe { msg_send![cls, sharedApplication] };
        let windows: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![&*app?, windows] };
        for w in windows?.to_vec() {
            let t: Option<Retained<NSString>> = unsafe { msg_send![&*w, title] };
            if t.map(|t| t.to_string()).as_deref() == Some(TITLE) {
                return unsafe { msg_send![&*w, contentView] };
            }
        }
        None
    }

    /// What the card's element answers: its custom actions' names, its
    /// help (the node's description), whether the selector is allowed,
    /// and the plain button's custom actions (none).
    #[derive(Debug)]
    #[allow(dead_code)] // read through Debug
    struct Seen {
        names: Vec<String>,
        help: Option<String>,
        allowed: bool,
    }

    fn look(title: &str) -> Option<Seen> {
        let view = content_view()?;
        let el = find(&view, title, 6)?;
        let actions: Option<Retained<NSArray<AnyObject>>> =
            unsafe { msg_send![&*el, accessibilityCustomActions] };
        let names = actions
            .map(|a| a.to_vec())
            .unwrap_or_default()
            .iter()
            .filter_map(|a| string(a, "name"))
            .collect();
        let allowed: Bool = unsafe {
            msg_send![&*el, isAccessibilitySelectorAllowed: objc2::sel!(accessibilityCustomActions)]
        };
        Some(Seen {
            names,
            help: string(&el, "help"),
            allowed: allowed.as_bool(),
        })
    }

    /// Run the custom action `name` of the element titled `title`, as
    /// VoiceOver does when it is chosen from the Actions rotor.
    /// Whether the tab titled `title` says it is the one chosen: on the
    /// Mac a tab is a radio button, its value 1 when chosen (what
    /// VoiceOver reads as "selected").
    fn chosen(title: &str) -> Option<bool> {
        let view = content_view()?;
        let el = find(&view, title, 6)?;
        let v: Option<Retained<AnyObject>> = unsafe { msg_send![&*el, accessibilityValue] };
        let b: Bool = unsafe { msg_send![&*v?, boolValue] };
        Some(b.as_bool())
    }

    fn perform(title: &'static str, name: &'static str) -> bool {
        let Some(view) = content_view() else { return false };
        let Some(card) = find(&view, title, 6) else { return false };
        let actions: Option<Retained<NSArray<AnyObject>>> =
            unsafe { msg_send![&*card, accessibilityCustomActions] };
        for a in actions.map(|a| a.to_vec()).unwrap_or_default() {
            if string(&a, "name").as_deref() == Some(name) {
                let h: *mut Block<dyn Fn() -> Bool> = unsafe { msg_send![&a, handler] };
                if h.is_null() {
                    return false;
                }
                return unsafe { (*h).call(()) }.as_bool();
            }
        }
        false
    }

    // The AX client API, as another process (VoiceOver) reads this one.
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
        fn AXUIElementCopyAttributeValue(
            e: *const c_void,
            attr: *const c_void,
            value: *mut *const c_void,
        ) -> i32;
        fn AXUIElementCopyActionNames(e: *const c_void, names: *mut *const c_void) -> i32;
        fn CFRelease(cf: *const c_void);
        fn AXUIElementPerformAction(e: *const c_void, action: *const c_void) -> i32;
        fn AXObserverCreateWithInfoCallback(
            pid: i32,
            callback: extern "C" fn(*const c_void, *const c_void, *const c_void, *const c_void, *mut c_void),
            out: *mut *const c_void,
        ) -> i32;
        fn AXObserverAddNotification(
            o: *const c_void,
            e: *const c_void,
            notification: *const c_void,
            refcon: *mut c_void,
        ) -> i32;
        fn AXObserverGetRunLoopSource(o: *const c_void) -> *const c_void;
        fn CFRunLoopGetCurrent() -> *const c_void;
        fn CFRunLoopAddSource(rl: *const c_void, source: *const c_void, mode: *const c_void);
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, after_source: u8) -> i32;
        static kCFRunLoopDefaultMode: *const c_void;
    }

    /// Through the AX client API, from this (not the main) thread, as
    /// VoiceOver does from its own process: the card's action names (a
    /// custom action is "Name:<name>\nTarget:…\nSelector:…"), and the
    /// result of performing the one named `run`.
    fn ax_client_actions(run: &str) -> Option<(Vec<String>, i32)> {
        unsafe {
            let app = AXUIElementCreateApplication(std::process::id() as i32);
            let found = ax_find(app, "Card", 8);
            CFRelease(app);
            let card = found?;
            let mut names: *const c_void = std::ptr::null();
            let r = AXUIElementCopyActionNames(card, &mut names);
            if r != 0 || names.is_null() {
                CFRelease(card);
                return None;
            }
            let arr = &*(names as *const NSArray<NSString>);
            let out: Vec<String> = arr.to_vec().iter().map(|s| s.to_string()).collect();
            let mut performed = -1;
            for a in arr.to_vec() {
                if a.to_string().starts_with(&format!("Name:{run}\n")) {
                    performed = AXUIElementPerformAction(card, Retained::as_ptr(&a) as *const c_void);
                }
            }
            CFRelease(names);
            CFRelease(card);
            Some((out, performed))
        }
    }

    /// Whether the AX client API is served for this process's own
    /// elements: its window reads as an `AXWindow`. On some macOS builds a
    /// process reading itself is answered with the application element
    /// for every element (its window's role is `AXApplication`), and the
    /// walk to the card cannot happen; that half is then skipped and said.
    fn ax_served() -> bool {
        unsafe {
            let app = AXUIElementCreateApplication(std::process::id() as i32);
            let mut ok = false;
            if let Some(ws) = ax_attr(app, "AXWindows") {
                let arr = &*(ws as *const NSArray<AnyObject>);
                if arr.count() > 0 {
                    let w = &*arr.objectAtIndex(0) as *const AnyObject as *const c_void;
                    if let Some(rv) = ax_attr(w, "AXRole") {
                        ok = (&*(rv as *const NSString)).to_string() == "AXWindow";
                        CFRelease(rv);
                    }
                }
                CFRelease(ws);
            }
            CFRelease(app);
            ok
        }
    }

    static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());

    extern "C" fn heard(
        _o: *const c_void,
        _e: *const c_void,
        _n: *const c_void,
        info: *const c_void,
        _r: *mut c_void,
    ) {
        if info.is_null() {
            return;
        }
        let d = unsafe { &*(info as *const objc2_foundation::NSDictionary<NSString, AnyObject>) };
        if let Some(t) = d.objectForKey(&NSString::from_str("AXAnnouncementKey")) {
            let t: Retained<NSString> = unsafe { msg_send![&t, description] };
            HEARD.lock().unwrap().push(t.to_string());
        }
    }

    /// Listen, as VoiceOver does, for this process's announcements
    /// ("AXAnnouncementRequested") on this thread's run loop.
    fn listen() -> Option<*const c_void> {
        unsafe {
            let mut o: *const c_void = std::ptr::null();
            if AXObserverCreateWithInfoCallback(std::process::id() as i32, heard, &mut o) != 0 {
                return None;
            }
            let app = AXUIElementCreateApplication(std::process::id() as i32);
            let n = NSString::from_str("AXAnnouncementRequested");
            let r = AXObserverAddNotification(o, app, Retained::as_ptr(&n) as *const c_void, std::ptr::null_mut());
            CFRelease(app);
            if r != 0 {
                return None;
            }
            CFRunLoopAddSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(o), kCFRunLoopDefaultMode);
            Some(o)
        }
    }

    /// Run this thread's run loop until `text` is heard or `wait` passes.
    fn hear(text: &str, wait: Duration) -> Vec<String> {
        let start = Instant::now();
        while start.elapsed() < wait && !HEARD.lock().unwrap().iter().any(|t| t == text) {
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, 0) };
        }
        HEARD.lock().unwrap().clone()
    }

    unsafe fn ax_attr(e: *const c_void, name: &str) -> Option<*const c_void> {
        let attr = NSString::from_str(name);
        let mut v: *const c_void = std::ptr::null();
        let r = unsafe {
            AXUIElementCopyAttributeValue(e, Retained::as_ptr(&attr) as *const c_void, &mut v)
        };

        (r == 0 && !v.is_null()).then_some(v)
    }

    /// The first element under `e` titled `title` (retained; the caller
    /// releases it).
    unsafe fn ax_find(e: *const c_void, title: &str, depth: usize) -> Option<*const c_void> {
        let kids = unsafe { ax_attr(e, "AXChildren") }?;
        let arr = unsafe { &*(kids as *const NSArray<AnyObject>) };
        let mut out = None;
        for i in 0..arr.count() {
            let c = &*arr.objectAtIndex(i) as *const AnyObject as *const c_void;
            // an element that lists the application (or its menu bar) as
            // its child leads back up: not where the window's nodes are
            if let Some(rv) = unsafe { ax_attr(c, "AXRole") } {
                let role = unsafe { &*(rv as *const NSString) }.to_string();
                unsafe { CFRelease(rv) };
                if role == "AXApplication" || role == "AXMenuBar" {
                    continue;
                }
            }
            if let Some(t) = unsafe { ax_attr(c, "AXTitle") } {
                let s = unsafe { &*(t as *const NSString) }.to_string();
                unsafe { CFRelease(t) };
                if s == title {
                    let r: Retained<AnyObject> =
                        unsafe { Retained::retain(c as *mut AnyObject) }.unwrap();
                    out = Some(Retained::into_raw(r) as *const c_void);
                    break;
                }
            }
            if depth > 0
                && let Some(f) = unsafe { ax_find(c, title, depth - 1) }
            {
                out = Some(f);
                break;
            }
        }
        unsafe { CFRelease(kids) };
        out
    }

    pub fn main() {
        // Never hang: a wedged run fails.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(60));
            eprintln!("gui_macos_a11y_test: timed out");
            std::process::exit(2);
        });
        static RESULT: Mutex<Option<String>> = Mutex::new(None);
        static SEEN: Mutex<Option<String>> = Mutex::new(None);
        let code = olang::stdlib::gui::platform::host(|| {
            std::thread::spawn(|| {
                let prober = std::thread::spawn(|| {
                    // The adapter starts when first asked (it asks the
                    // loop for the tree), so ask until the card is there.
                    let start = Instant::now();
                    let card = loop {
                        if let Some(s) = on_main(|| look("Card"))
                            && !s.names.is_empty()
                        {
                            break s;
                        }
                        assert!(
                            start.elapsed() < Duration::from_secs(20),
                            "the card never showed its actions"
                        );
                        std::thread::sleep(Duration::from_millis(100));
                    };
                    let plain = on_main(|| look("Plain"));
                    let row = on_main(|| look("Row one"));
                    let other_row = on_main(|| look("Row zero"));
                    let tabs = (on_main(|| chosen("Commands tab")), on_main(|| chosen("Editor tab")));
                    let trusted = unsafe { AXIsProcessTrusted() };
                    let served = trusted && ax_served();
                    let ax = if served { ax_client_actions("Move to To do") } else { None };
                    let observer = if trusted { listen() } else { None };
                    let ran = on_main(|| perform("Card", "Move to In progress"));
                    let ran_row = on_main(|| perform("Row one", "Move to Done"));
                    // the program announces the move
                    let heard = if observer.is_some() {
                        Some(hear("Row one moved to Done", Duration::from_secs(5)))
                    } else {
                        std::thread::sleep(Duration::from_millis(500));
                        None
                    };
                    // the system's settings "changed": the loop hears AppKit's notice
                    on_main(olang::stdlib::gui::platform::post_settings_notice);
                    // and ends at the next action
                    let ran_last = on_main(|| perform("Card", "Move to To do"));
                    if let Some(o) = observer {
                        unsafe { CFRelease(o) };
                    }
                    *SEEN.lock().unwrap() = Some(format!(
                        "card={card:?}\nplain={plain:?}\nrow={row:?}\nother_row={other_row:?}\ntabs={tabs:?}\ntrusted={trusted} served={served} ax={ax:?}\nran={ran} ran_row={ran_row} ran_last={ran_last}\nheard={heard:?}"
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
        println!("seen: {seen}");
        println!("program: {result}");
        assert_eq!(code, 0);
        let has = |line: &str| assert!(seen.lines().any(|l| l == line), "no {line:?} in\n{seen}");
        has(r#"card=Seen { names: ["Move to To do", "Move to In progress"], help: Some("Backlog, 1 of 3"), allowed: true }"#);
        has(r#"plain=Some(Seen { names: [], help: None, allowed: false })"#);
        // the list's active row offers the list's actions; the other row none
        has(r#"row=Some(Seen { names: ["Move to Backlog", "Move to Done"], help: None, allowed: true })"#);
        has(r#"other_row=Some(Seen { names: [], help: None, allowed: false })"#);
        has("ran=true ran_row=true ran_last=true");
        // a tab says whether it is the one chosen
        has("tabs=(Some(true), Some(false))");
        if seen.contains("served=true") {
            assert!(
                seen.lines().any(|l| l.starts_with("trusted=")
                    && l.contains(r#""Name:Move to In progress\n"#)
                    && l.ends_with(", 0))")),
                "the AX client did not see or run the custom actions: {seen}"
            );
        } else if seen.contains("trusted=true") {
            println!("note: the AX client API answers this process's own elements with the application (this macOS build); the client half was skipped");
        } else {
            println!("note: this process is not trusted for Accessibility; the AX client half was skipped");
        }
        let expect = r#"["card", 1, "Move to In progress"], ["col", 1, "Move to Done"], ["card", 0, "Move to To do"], ["settings heard", true]]"#;
        if seen.contains("trusted=true") {
            assert!(
                seen.lines().any(|l| l.starts_with("heard=Some(") && l.contains(r#""Row one moved to Done""#)),
                "the announcement was not heard: {seen}"
            );
        }
        if seen.contains("served=true") {
            assert_eq!(result, format!(r#"[["card", 0, "Move to To do"], {expect}"#));
        } else {
            assert_eq!(result, format!("[{expect}"));
        }
        println!("gui_macos_a11y_test: ok");
    }
}

fn main() {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    mac::main();
}

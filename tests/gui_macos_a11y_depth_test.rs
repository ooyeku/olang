//! A real window's accessibility depth, as VoiceOver and Zoom read it
//! (macOS): the rotor's searches (headings, landmarks, links, form
//! controls, tables) through `AXUIElementsForSearchPredicate`, a table's
//! rows, columns and headers, a tree's levels and disclosure, the focused
//! element's frame (what Zoom follows), a page scroll, and a live
//! region's value said when it changes.
//!
//! The test walks the window's NSAccessibility elements on the main
//! thread, as AppKit's accessibility server does for another process.
//! When this process is trusted for Accessibility, the search is also
//! asked through the AX client API and the announcement is listened for;
//! untrusted, those halves say so and are skipped.
//!
//! The harness is off: AppKit needs the process's first thread, which
//! `gui::platform::host` takes for winit's loop.

#[cfg(all(feature = "gui", target_os = "macos"))]
mod mac {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::{ClassType, Encode, Encoding};
    use objc2_foundation::{NSArray, NSDictionary, NSString};
    use olang::{Interpreter, Parser};
    use std::ffi::c_void;
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    const TITLE: &str = "olang a11y depth";

    // AppKit's NSRange and NSRect, as messages return them (the crate's
    // own need features this build may not have)
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct Range {
        location: usize,
        length: usize,
    }
    unsafe impl Encode for Range {
        const ENCODING: Encoding =
            Encoding::Struct("_NSRange", &[usize::ENCODING, usize::ENCODING]);
    }
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct Pair {
        a: f64,
        b: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct Rect {
        origin: Pair,
        size: Pair,
    }
    unsafe impl Encode for Rect {
        const ENCODING: Encoding = Encoding::Struct(
            "CGRect",
            &[
                Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]),
                Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]),
            ],
        );
    }

    const PROGRAM: &str = r##"
        let w = match gui.open(#{ "title": "olang a11y depth", "size": (640, 420) }) {
          Ok(w) => w,
          Err(e) => e
        }
        let ev = gui.events()
        gui.apply(w, [
          #{ "key": "root", "box": (0, 0, 640, 420), "style": #{ "bg": "#ffffff" } },
          #{ "key": "nav", "parent": "root", "role": "navigation", "box": (0, 0, 180, 420), "name": "Sidebar" },
          #{ "key": "tr", "parent": "nav", "role": "tree", "box": (0, 0, 180, 200), "name": "Project" },
          #{ "key": "tr:0", "parent": "tr", "role": "treeitem", "box": (0, 0, 180, 20), "text": "src", "level": 1, "expanded": false },
          #{ "key": "tr:1", "parent": "tr", "role": "treeitem", "box": (0, 20, 180, 20), "text": "README", "level": 1 },
          #{ "key": "main", "parent": "root", "role": "main", "box": (180, 0, 460, 420) },
          #{ "key": "h", "parent": "main", "role": "heading", "box": (10, 0, 300, 28), "text": "Settings heading", "level": 1 },
          #{ "key": "name", "parent": "main", "role": "input", "box": (10, 30, 200, 24), "name": "Name field", "style": #{ "pad": (0, 6, 0, 6) }, "edit": #{ "value": "olang world", "select": (3, 3, 1) } },
          #{ "key": "go", "parent": "main", "role": "button", "box": (220, 30, 80, 24), "text": "Go button" },
          #{ "key": "lnk", "parent": "main", "role": "link", "box": (310, 30, 80, 24), "text": "Docs link" },
          #{ "key": "size", "parent": "main", "role": "spinbutton", "box": (10, 60, 100, 24), "name": "Size stepper", "value": "13" },
          #{ "key": "prog", "parent": "main", "role": "progressbar", "box": (120, 60, 100, 8), "name": "Progress", "value": "40%", "range": (0.4, 0, 1), "live": "polite" },
          #{ "key": "t", "parent": "main", "role": "table", "box": (10, 90, 440, 150), "name": "Rows table" },
          #{ "key": "t:h", "parent": "t", "box": (0, 0, 440, 24) },
          #{ "key": "t:h:0", "parent": "t:h", "role": "columnheader", "box": (0, 0, 150, 24), "text": "Name header" },
          #{ "key": "t:h:1", "parent": "t:h", "role": "columnheader", "box": (150, 0, 150, 24), "text": "Size header" },
          #{ "key": "t:h:2", "parent": "t:h", "role": "columnheader", "box": (300, 0, 140, 24), "text": "Kind header" },
          #{ "key": "t:b", "parent": "t", "box": (0, 24, 440, 126), "scroll": true, "content": (440, 24000) },
          #{ "key": "t:b:40", "parent": "t:b", "role": "row", "box": (0, 960, 440, 24), "pos": (40, 1000), "name": "Row forty" },
          #{ "key": "t:b:40:0", "parent": "t:b:40", "role": "cell", "box": (0, 0, 150, 24), "text": "a cell" },
          #{ "key": "t:b:40:1", "parent": "t:b:40", "role": "cell", "box": (150, 0, 150, 24), "text": "b cell" },
          #{ "key": "t:b:40:2", "parent": "t:b:40", "role": "cell", "box": (300, 0, 140, 24), "text": "c cell" },
          #{ "key": "t:b:41", "parent": "t:b", "role": "row", "box": (0, 984, 440, 24), "pos": (41, 1000), "name": "Row forty-one" },
          #{ "key": "t:b:41:0", "parent": "t:b:41", "role": "cell", "box": (0, 0, 150, 24), "text": "d cell" },
          #{ "key": "log", "parent": "main", "role": "region", "box": (10, 250, 440, 100), "name": "Output region", "content": (440, 2000) },
          #{ "key": "log:0", "parent": "log", "role": "text", "box": (0, 0, 440, 20), "text": "first line" },
          #{ "op": "focus", "key": "name" }
        ])
        // where the caret is drawn (logical pixels in the window)
        // (read again on each event until the window, key, has drawn it)
        let mut caret = gui.read(w, "caret")
        let mut heard = []
        let mut tries = 0
        let mut done = false
        let mut moved = false
        while !done && tries < 200 {
          tries = tries + 1
          match chan.recv_timeout(ev, #{ "timeout_ms": 200 }) {
            Ok(e) => {
              if caret == () => { caret = gui.read(w, "caret") } else => ()
              if e.kind == "a11y" => {
                heard = heard + [[e.key, e.action]]
                if e.action == "expand" && !moved => {
                  moved = true
                  // the progress moves on: a live node's value, said
                  gui.apply(w, [#{ "key": "prog", "parent": "main", "role": "progressbar", "box": (120, 60, 100, 8), "name": "Progress",
                                   "value": "60%", "range": (0.6, 0, 1), "live": "polite" }])
                } else => ()
                if e.action == "increment" => { done = true } else => ()
              } else => ()
              if e.kind == "scrolled" => { heard = heard + [["scrolled", e.key, e.y]] } else => ()
            },
            _ => ()
          }
        }
        gui.close(w)
        [["caret", caret]] + heard
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

    fn title(obj: &AnyObject) -> Option<String> {
        let t: Option<Retained<NSString>> = unsafe { msg_send![obj, accessibilityTitle] };
        let t = t.map(|t| t.to_string()).filter(|t| !t.is_empty());
        if t.is_some() {
            return t;
        }
        // a static text's words are its value
        let v: Option<Retained<AnyObject>> = unsafe { msg_send![obj, accessibilityValue] };
        let v = v?;
        let is: bool = unsafe { msg_send![&*v, isKindOfClass: NSString::class()] };
        is.then(|| unsafe { &*(Retained::as_ptr(&v) as *const NSString) }.to_string())
    }

    fn role(obj: &AnyObject) -> String {
        let r: Option<Retained<NSString>> = unsafe { msg_send![obj, accessibilityRole] };
        r.map(|r| r.to_string()).unwrap_or_default()
    }

    fn children(obj: &AnyObject) -> Vec<Retained<AnyObject>> {
        let a: Option<Retained<NSArray<AnyObject>>> =
            unsafe { msg_send![obj, accessibilityChildren] };
        a.map(|a| a.to_vec()).unwrap_or_default()
    }

    fn find(obj: &AnyObject, want: &str, depth: usize) -> Option<Retained<AnyObject>> {
        for c in children(obj) {
            if title(&c).as_deref() == Some(want) {
                return Some(c);
            }
            if depth > 0
                && let Some(f) = find(&c, want, depth - 1)
            {
                return Some(f);
            }
        }
        None
    }

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

    /// The window's root element (the view's one child).
    /// Whether the window is on the screen and not covered: AppKit builds
    /// no accessibility tree for a window nothing can see (a person's own
    /// window over it, a display asleep or locked).
    fn shown() -> bool {
        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return false;
        };
        let app: Option<Retained<AnyObject>> = unsafe { msg_send![cls, sharedApplication] };
        let Some(app) = app else { return false };
        let windows: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![&*app, windows] };
        windows
            .map(|ws| ws.to_vec())
            .unwrap_or_default()
            .iter()
            .any(|w| {
                let t: Option<Retained<NSString>> = unsafe { msg_send![&**w, title] };
                let state: usize = unsafe { msg_send![&**w, occlusionState] };
                t.map(|t| t.to_string()).as_deref() == Some(TITLE) && state & 2 != 0
            })
    }

    fn root() -> Option<Retained<AnyObject>> {
        let v = content_view()?;
        children(&v).into_iter().next()
    }

    fn predicate(key: &str, start: Option<&AnyObject>, previous: bool) -> Retained<AnyObject> {
        let mut keys: Vec<Retained<NSString>> = vec![
            NSString::from_str("AXSearchKey"),
            NSString::from_str("AXResultsLimit"),
            NSString::from_str("AXDirection"),
        ];
        let mut vals: Vec<Retained<AnyObject>> = vec![
            Retained::into_super(Retained::into_super(NSString::from_str(key))),
            {
                let cls = AnyClass::get(c"NSNumber").unwrap();
                let n: Retained<AnyObject> = unsafe { msg_send![cls, numberWithInteger: -1isize] };
                n
            },
            Retained::into_super(Retained::into_super(NSString::from_str(if previous {
                "AXDirectionPrevious"
            } else {
                "AXDirectionNext"
            }))),
        ];
        if let Some(s) = start {
            keys.push(NSString::from_str("AXStartElement"));
            vals.push(
                unsafe { Retained::retain(s as *const AnyObject as *mut AnyObject) }.unwrap(),
            );
        }
        let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
        let d: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_retained_objects(&key_refs, &vals);
        Retained::into_super(Retained::into_super(d))
    }

    /// The titles of what a rotor search for `key` finds under `root`.
    fn search(
        root: &AnyObject,
        key: &str,
        start: Option<&AnyObject>,
        previous: bool,
    ) -> Vec<String> {
        let attr = NSString::from_str("AXUIElementsForSearchPredicate");
        let p = predicate(key, start, previous);
        let found: Option<Retained<NSArray<AnyObject>>> =
            unsafe { msg_send![root, accessibilityAttributeValue: &*attr, forParameter: &*p] };
        found
            .map(|a| a.to_vec())
            .unwrap_or_default()
            .iter()
            .map(|e| title(e).unwrap_or_else(|| role(e)))
            .collect()
    }

    fn int(obj: &AnyObject, sel: &str) -> isize {
        match sel {
            "rows" => unsafe { msg_send![obj, accessibilityRowCount] },
            "columns" => unsafe { msg_send![obj, accessibilityColumnCount] },
            "index" => unsafe { msg_send![obj, accessibilityIndex] },
            "level" => unsafe { msg_send![obj, accessibilityDisclosureLevel] },
            _ => -1,
        }
    }

    fn allowed(obj: &AnyObject, sel: objc2::runtime::Sel) -> bool {
        let b: Bool = unsafe { msg_send![obj, isAccessibilitySelectorAllowed: sel] };
        b.as_bool()
    }

    fn frame(obj: &AnyObject) -> Rect {
        unsafe { msg_send![obj, accessibilityFrame] }
    }

    /// Everything the rotor and a table walk see, as lines.
    fn look() -> Option<String> {
        let root = root()?;
        // wait for the tree: the table is there
        let table = find(&root, "Rows table", 8)?;
        let mut out = Vec::new();
        out.push(format!(
            "headings={:?}",
            search(&root, "AXHeadingSearchKey", None, false)
        ));
        out.push(format!(
            "landmarks={:?}",
            search(&root, "AXLandmarkSearchKey", None, false)
        ));
        out.push(format!(
            "links={:?}",
            search(&root, "AXLinkSearchKey", None, false)
        ));
        out.push(format!(
            "controls={:?}",
            search(&root, "AXControlSearchKey", None, false)
        ));
        out.push(format!(
            "tables={:?}",
            search(&root, "AXTableSearchKey", None, false)
        ));
        out.push(format!(
            "fields={:?}",
            search(&root, "AXTextFieldSearchKey", None, false)
        ));
        // from the button on, and back from it
        let go = find(&root, "Go button", 8)?;
        out.push(format!(
            "after_go={:?}",
            search(&root, "AXControlSearchKey", Some(&go), false)
        ));
        out.push(format!(
            "before_go={:?}",
            search(&root, "AXControlSearchKey", Some(&go), true)
        ));
        let h = find(&root, "Settings heading", 8)?;
        out.push(format!("heading_role={}", role(&h)));
        out.push(format!(
            "table_role={} rows={} columns={}",
            role(&table),
            int(&table, "rows"),
            int(&table, "columns")
        ));
        let heads: Option<Retained<NSArray<AnyObject>>> =
            unsafe { msg_send![&*table, accessibilityColumnHeaderUIElements] };
        out.push(format!(
            "headers={:?}",
            heads
                .map(|a| a.to_vec())
                .unwrap_or_default()
                .iter()
                .filter_map(|e| title(e))
                .collect::<Vec<_>>()
        ));
        let rows: Option<Retained<NSArray<AnyObject>>> =
            unsafe { msg_send![&*table, accessibilityRows] };
        let rows = rows.map(|a| a.to_vec()).unwrap_or_default();
        out.push(format!(
            "table_rows={:?}",
            rows.iter().map(|r| int(r, "index")).collect::<Vec<_>>()
        ));
        let cell = find(&root, "b cell", 10)?;
        let cr: Range = unsafe { msg_send![&*cell, accessibilityColumnIndexRange] };
        let rr: Range = unsafe { msg_send![&*cell, accessibilityRowIndexRange] };
        out.push(format!(
            "cell column={} row={} role={}",
            cr.location,
            rr.location,
            role(&cell)
        ));
        let at: Option<Retained<AnyObject>> =
            unsafe { msg_send![&*table, accessibilityCellForColumn: 2isize, row: 40isize] };
        out.push(format!("cell_at={:?}", at.and_then(|c| title(&c))));
        let item = find(&root, "src", 8)?;
        let disclosed: Bool = unsafe { msg_send![&*item, isAccessibilityDisclosed] };
        out.push(format!(
            "item role={} level={} disclosed={} allowed={}",
            role(&item),
            int(&item, "level"),
            disclosed.as_bool(),
            allowed(&item, objc2::sel!(setAccessibilityDisclosed:))
        ));
        // Zoom: the focused element and its frame (the field, 200 × 24)
        let view = content_view()?;
        let focused: Option<Retained<AnyObject>> =
            unsafe { msg_send![&*view, accessibilityFocusedUIElement] };
        let f = focused?;
        let fr = frame(&f);
        out.push(format!(
            "focused={:?} size=({}, {})",
            title(&f),
            fr.size.a.round(),
            fr.size.b.round()
        ));
        // Zoom following the caret: the caret's own box (a collapsed range
        // at it), from the field's top left (points, AppKit's y up)
        let sel: Range = unsafe { msg_send![&*f, accessibilitySelectedTextRange] };
        let cr: Rect = unsafe {
            msg_send![&*f, accessibilityFrameForRange: Range { location: sel.location, length: 0 }]
        };
        out.push(format!(
            "caret at={} dx={:.1} dy={:.1} h={:.1}",
            sel.location,
            cr.origin.a - fr.origin.a,
            (fr.origin.b + fr.size.b) - (cr.origin.b + cr.size.b),
            cr.size.b
        ));
        // inside the window, under the heading
        let hr = frame(&h);
        out.push(format!("below_heading={}", fr.origin.b < hr.origin.b));
        let scroller = find(&root, "Output region", 8)?;
        let names: Option<Retained<NSArray<NSString>>> =
            unsafe { msg_send![&*scroller, accessibilityActionNames] };
        out.push(format!(
            "scroller role={} actions={:?}",
            role(&scroller),
            names
                .map(|a| a.to_vec())
                .unwrap_or_default()
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
        ));
        Some(out.join("\n"))
    }

    /// The assistive requests: open the tree's item, scroll the output a
    /// page, step the stepper up (the program ends on it).
    fn act(what: &str) -> bool {
        let Some(root) = root() else { return false };
        match what {
            "disclose" => {
                let Some(item) = find(&root, "src", 8) else {
                    return false;
                };
                let _: () = unsafe { msg_send![&*item, setAccessibilityDisclosed: true] };
                true
            }
            "page" => {
                let Some(sc) = find(&root, "Output region", 8) else {
                    return false;
                };
                let a = NSString::from_str("AXScrollDownByPage");
                let _: () = unsafe { msg_send![&*sc, accessibilityPerformAction: &*a] };
                true
            }
            "step" => {
                let Some(s) = find(&root, "Size stepper", 8) else {
                    return false;
                };
                let ok: Bool = unsafe { msg_send![&*s, accessibilityPerformIncrement] };
                ok.as_bool()
            }
            _ => false,
        }
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
        fn AXUIElementCopyAttributeValue(
            e: *const c_void,
            attr: *const c_void,
            value: *mut *const c_void,
        ) -> i32;
        fn AXUIElementCopyParameterizedAttributeValue(
            e: *const c_void,
            attr: *const c_void,
            param: *const c_void,
            value: *mut *const c_void,
        ) -> i32;
        fn CFRelease(cf: *const c_void);
        fn AXValueCreate(kind: u32, value: *const c_void) -> *const c_void;
        fn AXValueGetValue(v: *const c_void, kind: u32, out: *mut c_void) -> bool;
        fn AXObserverCreateWithInfoCallback(
            pid: i32,
            callback: extern "C" fn(
                *const c_void,
                *const c_void,
                *const c_void,
                *const c_void,
                *mut c_void,
            ),
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

    unsafe fn ax_attr(e: *const c_void, name: &str) -> Option<*const c_void> {
        let attr = NSString::from_str(name);
        let mut v: *const c_void = std::ptr::null();
        let r = unsafe {
            AXUIElementCopyAttributeValue(e, Retained::as_ptr(&attr) as *const c_void, &mut v)
        };
        (r == 0 && !v.is_null()).then_some(v)
    }

    /// Through the AX client API, as VoiceOver asks from its own process:
    /// the headings under the window. `None` when the API answers this
    /// process with the application for every element.
    fn ax_client_headings() -> Option<usize> {
        unsafe {
            let app = AXUIElementCreateApplication(std::process::id() as i32);
            let ws = ax_attr(app, "AXWindows");
            CFRelease(app);
            let ws = ws?;
            let arr = &*(ws as *const NSArray<AnyObject>);
            let mut found = None;
            for i in 0..arr.count() {
                let w = &*arr.objectAtIndex(i) as *const AnyObject as *const c_void;
                let Some(t) = ax_attr(w, "AXTitle") else {
                    continue;
                };
                let is = (&*(t as *const NSString)).to_string() == TITLE;
                CFRelease(t);
                if !is {
                    continue;
                }
                // the window's content: its group of the program's nodes
                let Some(kids) = ax_attr(w, "AXChildren") else {
                    continue;
                };
                let ka = &*(kids as *const NSArray<AnyObject>);
                for j in 0..ka.count() {
                    let c = &*ka.objectAtIndex(j) as *const AnyObject as *const c_void;
                    let p = predicate("AXHeadingSearchKey", None, false);
                    let attr = NSString::from_str("AXUIElementsForSearchPredicate");
                    let mut v: *const c_void = std::ptr::null();
                    let r = AXUIElementCopyParameterizedAttributeValue(
                        c,
                        Retained::as_ptr(&attr) as *const c_void,
                        Retained::as_ptr(&p) as *const c_void,
                        &mut v,
                    );
                    if r == 0 && !v.is_null() {
                        let n = (&*(v as *const NSArray<AnyObject>)).count();
                        CFRelease(v);
                        found = Some(found.unwrap_or(0) + n);
                    }
                }
                CFRelease(kids);
            }
            CFRelease(ws);
            found
        }
    }

    /// Through the AX client API, as Zoom asks: the focused field's
    /// `AXBoundsForRange` at its caret (a collapsed range), from the
    /// field's own top left (points, the client's y down). `None` when the
    /// API answers this process with the application.
    fn ax_client_caret() -> Option<(usize, f64, f64, f64)> {
        #[repr(C)]
        struct CfRange {
            location: isize,
            length: isize,
        }
        unsafe {
            let app = AXUIElementCreateApplication(std::process::id() as i32);
            let f = ax_attr(app, "AXFocusedUIElement");
            CFRelease(app);
            let f = f?;
            let point = |name: &str| -> Option<(f64, f64)> {
                let v = ax_attr(f, name)?;
                let mut p = Pair { a: 0.0, b: 0.0 };
                // kAXValueCGPointType 1, kAXValueCGSizeType 2
                let kind = if name == "AXPosition" { 1 } else { 2 };
                let ok = AXValueGetValue(v, kind, &mut p as *mut Pair as *mut c_void);
                CFRelease(v);
                ok.then_some((p.a, p.b))
            };
            let pos = point("AXPosition");
            let sel = ax_attr(f, "AXSelectedTextRange");
            let mut r = CfRange {
                location: 0,
                length: 0,
            };
            if let Some(v) = sel {
                // kAXValueCFRangeType 4
                AXValueGetValue(v, 4, &mut r as *mut CfRange as *mut c_void);
                CFRelease(v);
            }
            let at = CfRange {
                location: r.location,
                length: 0,
            };
            let param = AXValueCreate(4, &at as *const CfRange as *const c_void);
            let attr = NSString::from_str("AXBoundsForRange");
            let mut v: *const c_void = std::ptr::null();
            let e = AXUIElementCopyParameterizedAttributeValue(
                f,
                Retained::as_ptr(&attr) as *const c_void,
                param,
                &mut v,
            );
            CFRelease(param);
            CFRelease(f);
            let pos = pos?;
            if e != 0 || v.is_null() {
                return None;
            }
            let mut b = Rect {
                origin: Pair { a: 0.0, b: 0.0 },
                size: Pair { a: 0.0, b: 0.0 },
            };
            // kAXValueCGRectType 3
            let ok = AXValueGetValue(v, 3, &mut b as *mut Rect as *mut c_void);
            CFRelease(v);
            ok.then_some((
                r.location as usize,
                b.origin.a - pos.0,
                b.origin.b - pos.1,
                b.size.b,
            ))
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
        let d = unsafe { &*(info as *const NSDictionary<NSString, AnyObject>) };
        if let Some(t) = d.objectForKey(&NSString::from_str("AXAnnouncementKey")) {
            let t: Retained<NSString> = unsafe { msg_send![&t, description] };
            HEARD.lock().unwrap().push(t.to_string());
        }
    }

    fn listen() -> Option<*const c_void> {
        unsafe {
            let mut o: *const c_void = std::ptr::null();
            if AXObserverCreateWithInfoCallback(std::process::id() as i32, heard, &mut o) != 0 {
                return None;
            }
            let app = AXUIElementCreateApplication(std::process::id() as i32);
            let n = NSString::from_str("AXAnnouncementRequested");
            let r = AXObserverAddNotification(
                o,
                app,
                Retained::as_ptr(&n) as *const c_void,
                std::ptr::null_mut(),
            );
            CFRelease(app);
            if r != 0 {
                return None;
            }
            CFRunLoopAddSource(
                CFRunLoopGetCurrent(),
                AXObserverGetRunLoopSource(o),
                kCFRunLoopDefaultMode,
            );
            Some(o)
        }
    }

    fn hear(text: &str, wait: Duration) -> Vec<String> {
        let start = Instant::now();
        while start.elapsed() < wait && !HEARD.lock().unwrap().iter().any(|t| t == text) {
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, 0) };
        }
        HEARD.lock().unwrap().clone()
    }

    /// What the prober says when the window never reached the screen.
    const NOT_SHOWN: &str = "not shown";

    pub fn main() {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(90));
            eprintln!("gui_macos_a11y_depth_test: timed out");
            std::process::exit(2);
        });
        static RESULT: Mutex<Option<String>> = Mutex::new(None);
        static SEEN: Mutex<Option<String>> = Mutex::new(None);
        let code = olang::stdlib::gui::platform::host(|| {
            std::thread::spawn(|| {
                let prober = std::thread::spawn(|| {
                    let start = Instant::now();
                    let mut ever_shown = false;
                    let seen = loop {
                        if let Some(s) = on_main(look) {
                            break s;
                        }
                        ever_shown = ever_shown || on_main(shown);
                        if start.elapsed() >= Duration::from_secs(40) {
                            // never on the screen: nothing to read, not a failure
                            assert!(!ever_shown, "the window's tree never showed");
                            *SEEN.lock().unwrap() = Some(NOT_SHOWN.to_string());
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    };
                    let trusted = unsafe { AXIsProcessTrusted() };
                    let client = if trusted { ax_client_headings() } else { None };
                    let client_caret = if trusted { ax_client_caret() } else { None };
                    let observer = if trusted { listen() } else { None };
                    let disclosed = on_main(|| act("disclose"));
                    let said = if observer.is_some() {
                        Some(hear("Progress, 60%", Duration::from_secs(5)))
                    } else {
                        std::thread::sleep(Duration::from_millis(500));
                        None
                    };
                    let paged = on_main(|| act("page"));
                    std::thread::sleep(Duration::from_millis(300));
                    let stepped = on_main(|| act("step"));
                    if let Some(o) = observer {
                        unsafe { CFRelease(o) };
                    }
                    *SEEN.lock().unwrap() = Some(format!(
                        "{seen}\nacted={disclosed} {paged} {stepped}\ntrusted={trusted} client={client:?}\nclient_caret={client_caret:?}\nsaid={said:?}"
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
        if seen == NOT_SHOWN {
            println!(
                "gui_macos_a11y_depth_test: skipped, the window was never on the screen \
                 (covered, or the display asleep or locked); it needs a real display"
            );
            return;
        }
        let result = RESULT.lock().unwrap().take().unwrap_or_default();
        println!("seen:\n{seen}");
        println!("program: {result}");
        assert_eq!(code, 0);
        let has = |line: &str| assert!(seen.lines().any(|l| l == line), "no {line:?} in\n{seen}");
        has(r#"headings=["Settings heading"]"#);
        has(r#"landmarks=["Sidebar", "AXGroup"]"#);
        has(r#"links=["Docs link"]"#);
        has(r#"controls=["Name field", "Go button", "Size stepper"]"#);
        has(r#"tables=["Rows table"]"#);
        has(r#"fields=["Name field"]"#);
        has(r#"after_go=["Size stepper"]"#);
        has(r#"before_go=["Name field"]"#);
        has("heading_role=AXHeading");
        has("table_role=AXTable rows=1000 columns=3");
        has(r#"headers=["Name header", "Size header", "Kind header"]"#);
        has("table_rows=[40, 41]");
        has("cell column=1 row=40 role=AXCell");
        has(r#"cell_at=Some("c cell")"#);
        has("item role=AXRow level=0 disclosed=false allowed=true");
        has(r#"focused=Some("Name field") size=(200, 24)"#);
        has("below_heading=true");
        // the caret's box, as Zoom and VoiceOver read it, is where it is
        // drawn: the program's caret (logical, the window's) from the
        // field's top left (190, 30)
        let drawn = super::caret_of(&result);
        let line = seen
            .lines()
            .find(|l| l.starts_with("caret at="))
            .unwrap_or_else(|| panic!("no caret in\n{seen}"));
        let field = |name: &str| -> f64 {
            line.split(' ')
                .find_map(|w| w.strip_prefix(&format!("{name}=")))
                .and_then(|v| v.parse().ok())
                .unwrap_or(f64::NAN)
        };
        assert_eq!(field("at"), 3.0, "{line}");
        assert!(
            (field("dx") - (drawn.0 - 190.0)).abs() <= 1.5,
            "{line} drawn {drawn:?}"
        );
        assert!(field("dx") > 6.0, "the caret, not the field's edge: {line}");
        assert!(
            field("dy") <= drawn.1 - 30.0 + 1.5
                && field("dy") + field("h") >= drawn.1 - 30.0 + drawn.3 - 1.5,
            "{line} drawn {drawn:?}"
        );
        has(r#"scroller role=AXScrollArea actions=["AXScrollToVisible", "AXScrollDownByPage"]"#);
        has("acted=true true true");
        assert!(result.contains(r#"["tr:0", "expand"]"#), "{result}");
        assert!(result.contains(r#"["scrolled", "log", 60.0]"#), "{result}");
        assert!(result.contains(r#"["size", "increment"]"#), "{result}");
        if seen.contains("trusted=true") {
            assert!(
                seen.lines()
                    .any(|l| l.starts_with("said=Some(") && l.contains(r#""Progress, 60%""#)),
                "the live value was not said: {seen}"
            );
            if let Some(c) = seen
                .lines()
                .find_map(|l| l.strip_prefix("client_caret=Some("))
            {
                let v: Vec<f64> = c
                    .trim_matches(|c| c == '(' || c == ')')
                    .split(", ")
                    .filter_map(|x| x.parse().ok())
                    .collect();
                assert_eq!(v.len(), 4, "{c}");
                assert_eq!(v[0], 3.0, "{c}");
                assert!(
                    (v[1] - (drawn.0 - 190.0)).abs() <= 1.5,
                    "AXBoundsForRange {c}, drawn {drawn:?}"
                );
                assert!(
                    v[2] <= drawn.1 - 30.0 + 1.5,
                    "AXBoundsForRange {c}, drawn {drawn:?}"
                );
            }
            if seen.contains("client=None") {
                println!(
                    "note: the AX client API answers this process's own elements with the application (this macOS build); the client half was skipped"
                );
            } else {
                has("trusted=true client=Some(1)");
            }
        } else {
            println!(
                "note: this process is not trusted for Accessibility; the AX client half was skipped"
            );
        }
        println!("gui_macos_a11y_depth_test: ok");
    }
}

/// The caret the program read, `["caret", (x, y, w, h)]` in its result.
#[cfg(all(feature = "gui", target_os = "macos"))]
fn caret_of(result: &str) -> (f64, f64, f64, f64) {
    let pat = "[\"caret\", (";
    let at = result.find(pat).expect("the caret in the result") + pat.len();
    let end = at + result[at..].find(')').expect("its end");
    let v: Vec<f64> = result[at..end]
        .split(", ")
        .filter_map(|x| x.parse().ok())
        .collect();
    assert_eq!(v.len(), 4, "{result}");
    (v[0], v[1], v[2], v[3])
}

fn main() {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    mac::main();
}

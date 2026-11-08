//! tray — the menu bar's side of `gui` (macOS): status items, the panel
//! anchored under one, the accessory application (no Dock icon, no main
//! window), and the system's notifications.
//!
//!   gui.accessory(true)
//!   let item = unwrap(gui.status_item(#{ "icon": "symbol:gauge", "title": "1.2 GB" }))
//!   ... a `status` event: the item was clicked ...
//!   let panel = unwrap(gui.open(#{ "panel": item, "size": (320, 420) }))
//!   ... a `dismiss` event: a click outside it, or Escape ...
//!   gui.notify(#{ "title": "Studio", "body": "Memory is climbing", "action": "open" })
//!
//! Everything AppKit does here runs on the loop's thread (the process's
//! first): the builtins ask it there through [`platform::on_main`] and
//! wait for the answer. A click on an item, a notification clicked, a
//! panel dismissed reach the program on the events channel as a window's
//! events do. Elsewhere every function answers `Err("unsupported")`.

use super::values::*;
use crate::ast::Value;
use std::sync::Arc;

type Res<T> = Result<T, String>;

pub fn status_value(id: u64) -> Value {
    let mut f = crate::ast::ValueMap::default();
    f.insert("id".to_string(), Value::Integer(id as i64));
    Value::Struct {
        type_name: "StatusItem".into(),
        fields: Arc::new(f).into(),
    }
}

/// The item id inside a `StatusItem` handle (or a bare id).
pub fn status_id(function: &str, v: Option<&Value>) -> Res<u64> {
    match v {
        Some(Value::Struct { type_name, fields }) if type_name == "StatusItem" => {
            match fields.get("id") {
                Some(Value::Integer(id)) => Ok(*id as u64),
                _ => Err(format!("{function}: malformed StatusItem handle")),
            }
        }
        Some(Value::Integer(id)) if *id > 0 => Ok(*id as u64),
        Some(other) => Err(format!(
            "{function}: expected a StatusItem (from gui.status_item), got {}",
            other.type_name()
        )),
        None => Err(format!("{function}: expected a StatusItem")),
    }
}

/// What an item shows: each `Some` replaces, `None` keeps.
#[derive(Default, Clone)]
pub struct Look {
    pub title: Option<String>,
    pub icon: Option<String>,
    pub tooltip: Option<String>,
    /// `""` none, else `"amber"`, `"red"`, `"green"`, or a `#rrggbb`.
    pub dot: Option<String>,
}

const LOOK_KEYS: [&str; 4] = ["title", "icon", "tooltip", "dot"];

fn opt_text(v: &Value, key: &str, function: &str) -> Res<Option<String>> {
    // `()` is a value here (it clears), not an absent key
    match fields(v).and_then(|f| f.get(key)) {
        None => Ok(None),
        Some(Value::Unit) => Ok(Some(String::new())),
        Some(Value::String(t)) => Ok(Some(t.to_string())),
        Some(other) => Err(format!(
            "{function}: \"{key}\" is a string or (), got {}",
            other.type_name()
        )),
    }
}

pub fn look_of(function: &str, v: Option<&Value>) -> Res<Look> {
    let Some(v) = v else {
        return Ok(Look::default());
    };
    if matches!(v, Value::Unit) {
        return Ok(Look::default());
    }
    let Some(f) = fields(v) else {
        return Err(format!("{function}: the options are a map"));
    };
    for k in f.keys() {
        if !LOOK_KEYS.contains(&k.as_str()) {
            return Err(format!(
                "{function}: unknown option \"{k}\" (the options are {})",
                LOOK_KEYS.join(", ")
            ));
        }
    }
    let look = Look {
        title: opt_text(v, "title", function)?,
        icon: opt_text(v, "icon", function)?,
        tooltip: opt_text(v, "tooltip", function)?,
        dot: opt_text(v, "dot", function)?,
    };
    if let Some(d) = &look.dot
        && dot_rgb(d).is_none()
    {
        return Err(format!(
            "{function}: \"dot\" is \"amber\", \"red\", \"green\", a \"#rrggbb\" colour, or () for none"
        ));
    }
    Ok(look)
}

/// A dot's colour, `Some(None)` for no dot, `None` when it is not one.
#[allow(clippy::option_option)]
pub fn dot_rgb(d: &str) -> Option<Option<(f64, f64, f64)>> {
    match d {
        "" | "none" => Some(None),
        "amber" => Some(Some((1.0, 0.62, 0.04))),
        "red" => Some(Some((1.0, 0.23, 0.19))),
        "green" => Some(Some((0.20, 0.78, 0.35))),
        _ => {
            let c = parse_color(&Value::String(Arc::new(d.to_string())))?;
            Some(Some((
                c[0] as f64 / 255.0,
                c[1] as f64 / 255.0,
                c[2] as f64 / 255.0,
            )))
        }
    }
}

/// A notification asked for.
pub struct Note {
    pub title: String,
    pub body: String,
    pub action: Option<String>,
    pub sound: bool,
}

pub fn note_of(function: &str, v: Option<&Value>) -> Res<Note> {
    let known = ["title", "body", "action", "sound"];
    let Some(v) = v.filter(|v| fields(v).is_some()) else {
        return Err(format!(
            "{function}: the options are a map: #{{ title, body, action }}"
        ));
    };
    for k in fields(v).expect("a map").keys() {
        if !known.contains(&k.as_str()) {
            return Err(format!(
                "{function}: unknown option \"{k}\" (the options are {})",
                known.join(", ")
            ));
        }
    }
    let title = get_str(v, "title", function)?.unwrap_or("").to_string();
    let body = get_str(v, "body", function)?.unwrap_or("").to_string();
    if title.is_empty() && body.is_empty() {
        return Err(format!(
            "{function}: a notification needs a title or a body"
        ));
    }
    Ok(Note {
        title,
        body,
        action: get_str(v, "action", function)?.map(|s| s.to_string()),
        sound: get_bool(v, "sound", function)?.unwrap_or(false),
    })
}

// ── the builtins ────────────────────────────────────────────────────

fn unsupported() -> Value {
    err("unsupported")
}

pub fn gui_status_item(args: &[Value]) -> Res<Value> {
    let look = look_of("gui.status_item", args.first())?;
    #[cfg(target_os = "macos")]
    {
        let id = super::next_id();
        Ok(
            match super::platform::on_main(move || mac::make_item(id, look)) {
                Ok(Ok(())) => ok(status_value(id)),
                Ok(Err(e)) | Err(e) => err(format!("gui.status_item: {e}")),
            },
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        drop(look);
        Ok(unsupported())
    }
}

/// `gui.status_set(item, #{ title, icon, tooltip, dot })` and the
/// one-field forms (`status_set_title`, …).
pub fn gui_status_set(function: &str, item: Option<&Value>, look: Look) -> Res<Value> {
    let id = status_id(function, item)?;
    #[cfg(target_os = "macos")]
    {
        let f = function.to_string();
        Ok(
            match super::platform::on_main(move || mac::set_item(id, look)) {
                Ok(Ok(())) => ok(Value::Unit),
                Ok(Err(e)) | Err(e) => err(format!("{f}: {e}")),
            },
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (id, look);
        Ok(unsupported())
    }
}

pub fn gui_status_remove(args: &[Value]) -> Res<Value> {
    let id = status_id("gui.status_remove", args.first())?;
    #[cfg(target_os = "macos")]
    {
        Ok(
            match super::platform::on_main(move || mac::remove_item(id)) {
                Ok(gone) => ok(Value::Boolean(gone)),
                Err(e) => err(format!("gui.status_remove: {e}")),
            },
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        Ok(unsupported())
    }
}

static ACCESSORY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the application runs as an accessory (no Dock icon, no menu
/// bar of its own): read by the loop as it starts.
pub fn accessory() -> bool {
    ACCESSORY.load(std::sync::atomic::Ordering::Acquire)
}

pub fn gui_accessory(args: &[Value]) -> Res<Value> {
    let on = match args.first() {
        Some(Value::Boolean(b)) => *b,
        _ => return Err("gui.accessory: the argument is true (an accessory: no Dock icon, no main window) or false".into()),
    };
    if !cfg!(target_os = "macos") {
        return Ok(unsupported());
    }
    ACCESSORY.store(on, std::sync::atomic::Ordering::Release);
    #[cfg(target_os = "macos")]
    if super::platform::loop_running() {
        let _ = super::platform::on_main(move || mac::set_policy(on));
    }
    Ok(ok(Value::Boolean(on)))
}

pub fn gui_notify(args: &[Value]) -> Res<Value> {
    let note = note_of("gui.notify", args.first())?;
    #[cfg(target_os = "macos")]
    {
        let id = format!("olang-{}", super::next_id());
        let id2 = id.clone();
        Ok(
            match super::platform::on_main(move || mac::notify(&id2, note)) {
                Ok(Ok(clickable)) => ok(map(vec![
                    ("id", s(&id)),
                    ("clickable", Value::Boolean(clickable)),
                ])),
                Ok(Err(e)) | Err(e) => err(format!("gui.notify: {e}")),
            },
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        drop(note);
        Ok(unsupported())
    }
}

// ── panels: what the loop asks ──────────────────────────────────────

/// Panels shown and dismissed, by item: whether one is shown, and when
/// one was last dismissed (a click on the item that dismissed its panel
/// says `was_open`, so the program does not open it again at once).
static PANELS: std::sync::Mutex<Vec<(u64, bool, Option<std::time::Instant>)>> =
    std::sync::Mutex::new(Vec::new());

pub fn note_panel(item: u64, shown: bool) {
    let mut p = PANELS.lock().unwrap_or_else(|e| e.into_inner());
    let at = if shown {
        None
    } else {
        Some(std::time::Instant::now())
    };
    match p.iter_mut().find(|r| r.0 == item) {
        Some(r) => {
            r.1 = shown;
            if at.is_some() {
                r.2 = at;
            }
        }
        None => p.push((item, shown, at)),
    }
}

pub fn panel_was_open(item: u64) -> bool {
    let p = PANELS.lock().unwrap_or_else(|e| e.into_inner());
    p.iter().any(|r| {
        r.0 == item
            && (r.1
                || r.2
                    .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(350)))
    })
}

#[cfg(target_os = "macos")]
pub mod mac {
    //! AppKit, on the loop's thread only.
    use super::super::values::*;
    use super::Look;
    use crate::ast::Value;
    use objc2::msg_send;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct Item {
        /// The NSStatusItem, retained (a raw pointer: only this thread
        /// touches it).
        ptr: usize,
        title: String,
        dot: String,
    }

    static ITEMS: Mutex<Option<HashMap<u64, Item>>> = Mutex::new(None);

    fn with_items<R>(f: impl FnOnce(&mut HashMap<u64, Item>) -> R) -> R {
        let mut g = ITEMS.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(HashMap::new))
    }

    fn cls(name: &std::ffi::CStr) -> Result<&'static AnyClass, String> {
        AnyClass::get(name).ok_or_else(|| format!("no {}", name.to_string_lossy()))
    }

    pub fn set_policy(on: bool) {
        // NSApplicationActivationPolicyAccessory 1, Regular 0
        if let Ok(c) = cls(c"NSApplication") {
            // SAFETY: the shared application, on the main thread.
            unsafe {
                let app: Option<Retained<AnyObject>> = msg_send![c, sharedApplication];
                if let Some(app) = app {
                    let _: bool =
                        msg_send![&*app, setActivationPolicy: if on { 1isize } else { 0isize }];
                }
            }
        }
    }

    /// The program's action for every item's button: a click (left, or
    /// right / ⌃-click) said as a `status` event.
    unsafe extern "C-unwind" fn clicked(_this: *mut AnyObject, _cmd: Sel, sender: *mut AnyObject) {
        if sender.is_null() {
            return;
        }
        // SAFETY: AppKit calls the action with the item's button, on the
        // main thread; the current event is the click's.
        unsafe {
            let tag: isize = msg_send![sender, tag];
            let mut right = false;
            if let Ok(c) = cls(c"NSApplication") {
                let app: *mut AnyObject = msg_send![c, sharedApplication];
                let ev: *mut AnyObject = msg_send![app, currentEvent];
                if !ev.is_null() {
                    let ty: usize = msg_send![ev, type];
                    let flags: usize = msg_send![ev, modifierFlags];
                    // NSEventTypeRightMouseDown 3, RightMouseUp 4; ⌃ 1 << 18
                    right = ty == 3 || ty == 4 || flags & (1 << 18) != 0;
                }
            }
            let item = tag as u64;
            let frame = button_frame(sender);
            super::super::emit(vec![event(
                "status",
                vec![
                    ("item", Value::Integer(item as i64)),
                    ("button", s(if right { "right" } else { "left" })),
                    ("was_open", Value::Boolean(super::panel_was_open(item))),
                    (
                        "frame",
                        match frame {
                            Some((x, y, w, h)) => Value::Tuple(std::sync::Arc::new(vec![
                                Value::Float(x),
                                Value::Float(y),
                                Value::Float(w),
                                Value::Float(h),
                            ])),
                            None => Value::Unit,
                        },
                    ),
                ],
            )]);
        }
    }

    /// The primary screen's height (AppKit's y runs up from its bottom;
    /// winit's and the program's down from its top).
    fn primary_height() -> f64 {
        // SAFETY: NSScreen's class methods, on the main thread.
        unsafe {
            let Ok(c) = cls(c"NSScreen") else { return 0.0 };
            let screens: Option<Retained<objc2_foundation::NSArray<AnyObject>>> =
                msg_send![c, screens];
            match screens.and_then(|a| a.firstObject()) {
                Some(s) => {
                    let f: NSRect = msg_send![&*s, frame];
                    f.size.height
                }
                None => 0.0,
            }
        }
    }

    /// A button's frame on the screen, top-left origin, in points.
    unsafe fn button_frame(button: *mut AnyObject) -> Option<(f64, f64, f64, f64)> {
        // SAFETY: a live NSStatusBarButton, on the main thread.
        unsafe {
            let w: *mut AnyObject = msg_send![button, window];
            if w.is_null() {
                return None;
            }
            let f: NSRect = msg_send![w, frame];
            let top = primary_height() - (f.origin.y + f.size.height);
            Some((f.origin.x, top, f.size.width, f.size.height))
        }
    }

    fn target() -> Result<*mut AnyObject, String> {
        static TARGET: Mutex<usize> = Mutex::new(0);
        let mut t = TARGET.lock().unwrap_or_else(|e| e.into_inner());
        if *t != 0 {
            return Ok(*t as *mut AnyObject);
        }
        let c = match AnyClass::get(c"OlangStatusTarget") {
            Some(c) => c,
            None => {
                let sup = cls(c"NSObject")?;
                let mut b = ClassBuilder::new(c"OlangStatusTarget", sup)
                    .ok_or("the status item's target class")?;
                // SAFETY: an action takes its sender and answers nothing.
                unsafe {
                    b.add_method(
                        objc2::sel!(clicked:),
                        clicked as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                    )
                };
                b.register()
            }
        };
        // SAFETY: a new NSObject, kept for the process's life.
        let obj: Retained<AnyObject> = unsafe {
            let a: Allocated<AnyObject> = msg_send![c, alloc];
            msg_send![a, init]
        };
        let p = Retained::into_raw(obj);
        *t = p as usize;
        Ok(p)
    }

    pub fn make_item(id: u64, look: Look) -> Result<(), String> {
        let bar_cls = cls(c"NSStatusBar")?;
        let target = target()?;
        // SAFETY: the system status bar and its new item, on the main
        // thread; the item is retained in ITEMS until it is removed.
        let item: Retained<AnyObject> = unsafe {
            let bar: Option<Retained<AnyObject>> = msg_send![bar_cls, systemStatusBar];
            let bar = bar.ok_or("no status bar")?;
            // NSVariableStatusItemLength
            let it: Option<Retained<AnyObject>> = msg_send![&*bar, statusItemWithLength: -1.0f64];
            it.ok_or("the status bar made no item")?
        };
        // SAFETY: as above; the button is the item's.
        unsafe {
            let button: *mut AnyObject = msg_send![&*item, button];
            if button.is_null() {
                return Err("the item has no button".into());
            }
            let _: () = msg_send![button, setTag: id as isize];
            let _: () = msg_send![button, setTarget: target];
            let _: () = msg_send![button, setAction: objc2::sel!(clicked:)];
            // NSEventMaskLeftMouseUp | NSEventMaskRightMouseUp
            let _: isize = msg_send![button, sendActionOn: (1usize << 2) | (1usize << 4)];
            // the image beside the title, never over it
            let _: () = msg_send![button, setImagePosition: 2isize];
        }
        let ptr = Retained::into_raw(item) as usize;
        with_items(|m| {
            m.insert(
                id,
                Item {
                    ptr,
                    title: String::new(),
                    dot: String::new(),
                },
            )
        });
        set_item(id, look)
    }

    pub fn remove_item(id: u64) -> bool {
        let Some(it) = with_items(|m| m.remove(&id)) else {
            return false;
        };
        // SAFETY: the item retained by make_item, given back here once.
        unsafe {
            let item = Retained::from_raw(it.ptr as *mut AnyObject).expect("retained");
            if let Ok(c) = cls(c"NSStatusBar") {
                let bar: Option<Retained<AnyObject>> = msg_send![c, systemStatusBar];
                if let Some(bar) = bar {
                    let _: () = msg_send![&*bar, removeStatusItem: &*item];
                }
            }
        }
        true
    }

    /// The item's button, while it is.
    pub fn button(id: u64) -> Option<*mut AnyObject> {
        let ptr = with_items(|m| m.get(&id).map(|i| i.ptr))?;
        // SAFETY: a retained NSStatusItem, on the main thread.
        let b: *mut AnyObject = unsafe { msg_send![ptr as *mut AnyObject, button] };
        (!b.is_null()).then_some(b)
    }

    pub fn set_item(id: u64, look: Look) -> Result<(), String> {
        let Some(button) = button(id) else {
            return Err(format!("status item {id} is removed"));
        };
        let (title, dot) = with_items(|m| {
            let it = m.get_mut(&id).expect("there");
            if let Some(t) = &look.title {
                it.title = t.clone();
            }
            if let Some(d) = &look.dot {
                it.dot = d.clone();
            }
            (it.title.clone(), it.dot.clone())
        });
        // SAFETY: the item's live button, on the main thread.
        unsafe {
            if look.title.is_some() || look.dot.is_some() {
                let _: () = msg_send![button, setAttributedTitle: &*attributed(&title, &dot)];
            }
            if let Some(t) = &look.tooltip {
                let tip = NSString::from_str(t);
                let none: *const AnyObject = std::ptr::null();
                if t.is_empty() {
                    let _: () = msg_send![button, setToolTip: none];
                } else {
                    let _: () = msg_send![button, setToolTip: &*tip];
                }
            }
            if let Some(i) = &look.icon {
                let img = if i.is_empty() { None } else { Some(image(i)?) };
                match img {
                    Some(img) => {
                        let _: () = msg_send![button, setImage: &*img];
                    }
                    None => {
                        let none: *const AnyObject = std::ptr::null();
                        let _: () = msg_send![button, setImage: none];
                    }
                }
            }
        }
        Ok(())
    }

    /// The title as the menu bar shows it: its digits all one width (a
    /// live readout keeps its place), after the status dot when there is
    /// one.
    unsafe fn attributed(title: &str, dot: &str) -> Retained<AnyObject> {
        // SAFETY: Foundation and AppKit's classes, on the main thread.
        unsafe {
            let font_cls = AnyClass::get(c"NSFont").expect("NSFont");
            let menu_font: *mut AnyObject = msg_send![font_cls, menuBarFontOfSize: 0.0f64];
            let size: f64 = if menu_font.is_null() {
                13.0
            } else {
                msg_send![menu_font, pointSize]
            };
            let font: *mut AnyObject =
                msg_send![font_cls, monospacedDigitSystemFontOfSize: size, weight: 0.0f64];
            let s_cls =
                AnyClass::get(c"NSMutableAttributedString").expect("NSMutableAttributedString");
            let all: Allocated<AnyObject> = msg_send![s_cls, alloc];
            let out: Retained<AnyObject> = msg_send![all, init];
            let font_key = NSString::from_str("NSFont");
            let color_key = NSString::from_str("NSColor");
            let dict_cls = AnyClass::get(c"NSDictionary").expect("NSDictionary");
            let a_cls = AnyClass::get(c"NSAttributedString").expect("NSAttributedString");
            let part = |text: &str, color: *mut AnyObject| -> Retained<AnyObject> {
                let t = NSString::from_str(text);
                let attrs: Retained<AnyObject> = if color.is_null() {
                    msg_send![dict_cls, dictionaryWithObject: font, forKey: &*font_key]
                } else {
                    let objs = [font, color];
                    let keys: [*const NSString; 2] = [&*font_key, &*color_key];
                    msg_send![dict_cls, dictionaryWithObjects: objs.as_ptr(), forKeys: keys.as_ptr(), count: 2usize]
                };
                let a: Allocated<AnyObject> = msg_send![a_cls, alloc];
                msg_send![a, initWithString: &*t, attributes: &*attrs]
            };
            if let Some(Some((r, g, b))) = super::dot_rgb(dot) {
                let c_cls = AnyClass::get(c"NSColor").expect("NSColor");
                let color: *mut AnyObject = match dot {
                    "amber" => msg_send![c_cls, systemOrangeColor],
                    "red" => msg_send![c_cls, systemRedColor],
                    "green" => msg_send![c_cls, systemGreenColor],
                    _ => msg_send![c_cls, colorWithSRGBRed: r, green: g, blue: b, alpha: 1.0f64],
                };
                let d = part(if title.is_empty() { "●" } else { "● " }, color);
                let _: () = msg_send![&*out, appendAttributedString: &*d];
            }
            if !title.is_empty() {
                let t = part(title, std::ptr::null_mut());
                let _: () = msg_send![&*out, appendAttributedString: &*t];
            }
            out
        }
    }

    /// A template image (it follows the menu bar's light and dark):
    /// `symbol:NAME` for an SF Symbol, else an image file (PNG, PDF),
    /// drawn 18 points high.
    unsafe fn image(spec: &str) -> Result<Retained<AnyObject>, String> {
        // SAFETY: NSImage, on the main thread.
        unsafe {
            let c = cls(c"NSImage")?;
            let img: Option<Retained<AnyObject>> = if let Some(name) = spec.strip_prefix("symbol:")
            {
                let n = NSString::from_str(name);
                let none: *const AnyObject = std::ptr::null();
                msg_send![c, imageWithSystemSymbolName: &*n, accessibilityDescription: none]
            } else {
                if !std::path::Path::new(spec).is_file() {
                    return Err(format!("no image at {spec}"));
                }
                let p = NSString::from_str(spec);
                let a: Allocated<AnyObject> = msg_send![c, alloc];
                msg_send![a, initWithContentsOfFile: &*p]
            };
            let img = img.ok_or_else(|| format!("no image {spec}"))?;
            if !spec.starts_with("symbol:") {
                let sz: NSSize = msg_send![&*img, size];
                if sz.height > 0.0 {
                    let h = 18.0;
                    let _: () = msg_send![&*img, setSize: NSSize::new(sz.width * h / sz.height, h)];
                }
            }
            let _: () = msg_send![&*img, setTemplate: true];
            Ok(img)
        }
    }

    // ── panels ──────────────────────────────────────────────────────

    /// A panel's window: borderless and rounded, over other windows and
    /// on every space, and shown under its item.
    pub fn panel_style(nw: &AnyObject) {
        // SAFETY: a live NSWindow, on the main thread.
        unsafe {
            let c_cls = AnyClass::get(c"NSColor").expect("NSColor");
            let clear: *mut AnyObject = msg_send![c_cls, clearColor];
            let _: () = msg_send![nw, setOpaque: false];
            let _: () = msg_send![nw, setBackgroundColor: clear];
            let _: () = msg_send![nw, setHasShadow: true];
            // NSPopUpMenuWindowLevel
            let _: () = msg_send![nw, setLevel: 101isize];
            // moveToActiveSpace | transient | fullScreenAuxiliary
            let _: () =
                msg_send![nw, setCollectionBehavior: (1usize << 1) | (1usize << 3) | (1usize << 8)];
            let content: *mut AnyObject = msg_send![nw, contentView];
            if !content.is_null() {
                let _: () = msg_send![content, setWantsLayer: true];
                let layer: *mut AnyObject = msg_send![content, layer];
                if !layer.is_null() {
                    let _: () = msg_send![layer, setCornerRadius: 10.0f64];
                    let _: () = msg_send![layer, setMasksToBounds: true];
                }
            }
        }
    }

    /// Put the panel under its item, centred on it and kept on its
    /// screen, and bring the application forward so it takes the keys.
    pub fn anchor(nw: &AnyObject, item: u64) {
        let Some(button) = button(item) else { return };
        // SAFETY: a live NSWindow and the item's button, on the main thread.
        unsafe {
            let bw: *mut AnyObject = msg_send![button, window];
            if bw.is_null() {
                return;
            }
            let bf: NSRect = msg_send![bw, frame];
            let pf: NSRect = msg_send![nw, frame];
            let screen: *mut AnyObject = msg_send![bw, screen];
            let mut x = bf.origin.x + bf.size.width / 2.0 - pf.size.width / 2.0;
            if !screen.is_null() {
                let vf: NSRect = msg_send![screen, visibleFrame];
                let lo = vf.origin.x + 6.0;
                let hi = vf.origin.x + vf.size.width - pf.size.width - 6.0;
                x = x.max(lo).min(hi.max(lo));
            }
            let top = bf.origin.y - 4.0;
            let _: () = msg_send![nw, setFrameTopLeftPoint: NSPoint::new(x, top)];
            if let Ok(c) = cls(c"NSApplication") {
                let app: *mut AnyObject = msg_send![c, sharedApplication];
                let _: () = msg_send![app, activateIgnoringOtherApps: true];
            }
        }
    }

    // ── notifications ───────────────────────────────────────────────

    fn bundled() -> bool {
        // SAFETY: NSBundle's main bundle, on the main thread.
        unsafe {
            let Ok(c) = cls(c"NSBundle") else {
                return false;
            };
            let b: *mut AnyObject = msg_send![c, mainBundle];
            if b.is_null() {
                return false;
            }
            let id: *mut AnyObject = msg_send![b, bundleIdentifier];
            !id.is_null()
        }
    }

    /// UserNotifications, loaded the first time one is posted (a program
    /// that posts none never loads it).
    fn center() -> Result<*mut AnyObject, String> {
        static CENTER: Mutex<usize> = Mutex::new(0);
        let mut g = CENTER.lock().unwrap_or_else(|e| e.into_inner());
        if *g != 0 {
            return Ok(*g as *mut AnyObject);
        }
        // SAFETY: NSBundle loading a system framework, on the main thread.
        unsafe {
            let b_cls = cls(c"NSBundle")?;
            let path = NSString::from_str("/System/Library/Frameworks/UserNotifications.framework");
            let fw: *mut AnyObject = msg_send![b_cls, bundleWithPath: &*path];
            if fw.is_null() {
                return Err("no UserNotifications framework".into());
            }
            let loaded: bool = msg_send![fw, load];
            if !loaded {
                return Err("UserNotifications did not load".into());
            }
        }
        let c = cls(c"UNUserNotificationCenter")?;
        // SAFETY: the process's notification center (an app bundle's:
        // `bundled` was checked), kept for the process's life.
        let center: *mut AnyObject = unsafe { msg_send![c, currentNotificationCenter] };
        if center.is_null() {
            return Err("no notification center".into());
        }
        let delegate = delegate()?;
        // SAFETY: as above; the delegate lives as long as the process.
        unsafe {
            let _: () = msg_send![center, setDelegate: delegate];
            // UNAuthorizationOptionSound 2 | Alert 4
            let done =
                block2::RcBlock::new(|_granted: objc2::runtime::Bool, _e: *mut AnyObject| {});
            let _: () = msg_send![center, requestAuthorizationWithOptions: 6usize, completionHandler: &*done];
        }
        *g = center as usize;
        Ok(center)
    }

    unsafe extern "C-unwind" fn did_receive(
        _this: *mut AnyObject,
        _cmd: Sel,
        _center: *mut AnyObject,
        response: *mut AnyObject,
        handler: *mut block2::Block<dyn Fn()>,
    ) {
        // SAFETY: the response AppKit hands the delegate; its handler is
        // called once, as UserNotifications asks.
        unsafe {
            if !response.is_null() {
                let n: *mut AnyObject = msg_send![response, notification];
                let req: *mut AnyObject = msg_send![n, request];
                let ident: *mut NSString = msg_send![req, identifier];
                let content: *mut AnyObject = msg_send![req, content];
                let info: *mut AnyObject = msg_send![content, userInfo];
                let key = NSString::from_str("action");
                let act: *mut NSString = if info.is_null() {
                    std::ptr::null_mut()
                } else {
                    msg_send![info, objectForKey: &*key]
                };
                let text = |p: *mut NSString| {
                    if p.is_null() {
                        None
                    } else {
                        Some((*p).to_string())
                    }
                };
                let action = text(act);
                super::super::emit(vec![event(
                    "notification",
                    vec![
                        ("id", opt_str(text(ident).as_deref())),
                        ("action", opt_str(action.as_deref())),
                    ],
                )]);
            }
            if let Some(h) = handler.as_ref() {
                h.call(());
            }
        }
    }

    unsafe extern "C-unwind" fn will_present(
        _this: *mut AnyObject,
        _cmd: Sel,
        _center: *mut AnyObject,
        _n: *mut AnyObject,
        handler: *mut block2::Block<dyn Fn(usize)>,
    ) {
        // shown while the application is in front too: banner | list | sound
        // SAFETY: the handler UserNotifications passes, called once.
        if let Some(h) = unsafe { handler.as_ref() } {
            h.call(((1usize << 4) | (1usize << 3) | (1usize << 1),));
        }
    }

    fn delegate() -> Result<*mut AnyObject, String> {
        let c = match AnyClass::get(c"OlangNoticeDelegate") {
            Some(c) => c,
            None => {
                let sup = cls(c"NSObject")?;
                let mut b = ClassBuilder::new(c"OlangNoticeDelegate", sup)
                    .ok_or("the notifications' delegate class")?;
                if let Some(p) =
                    objc2::runtime::AnyProtocol::get(c"UNUserNotificationCenterDelegate")
                {
                    b.add_protocol(p);
                }
                // SAFETY: the delegate methods' signatures as UserNotifications
                // declares them.
                unsafe {
                    b.add_method(
                        objc2::sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
                        did_receive
                            as unsafe extern "C-unwind" fn(
                                *mut AnyObject,
                                Sel,
                                *mut AnyObject,
                                *mut AnyObject,
                                *mut block2::Block<dyn Fn()>,
                            ),
                    );
                    b.add_method(
                        objc2::sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
                        will_present
                            as unsafe extern "C-unwind" fn(
                                *mut AnyObject,
                                Sel,
                                *mut AnyObject,
                                *mut AnyObject,
                                *mut block2::Block<dyn Fn(usize)>,
                            ),
                    );
                }
                b.register()
            }
        };
        // SAFETY: a new NSObject, kept for the process's life.
        let obj: Retained<AnyObject> = unsafe {
            let a: Allocated<AnyObject> = msg_send![c, alloc];
            msg_send![a, init]
        };
        Ok(Retained::into_raw(obj))
    }

    /// Post a notification. Answers whether a click on it reaches the
    /// program: only an application bundle has a notification center;
    /// a program run as a script is shown one by the system (`osascript`)
    /// that says nothing back.
    pub fn notify(id: &str, n: super::Note) -> Result<bool, String> {
        if !bundled() {
            let script = format!(
                "display notification {} with title {}{}",
                apple_string(&n.body),
                apple_string(&n.title),
                if n.sound {
                    " sound name \"default\""
                } else {
                    ""
                }
            );
            std::process::Command::new("/usr/bin/osascript")
                .arg("-e")
                .arg(script)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("osascript: {e}"))?;
            return Ok(false);
        }
        let center = center()?;
        // SAFETY: UserNotifications' classes (loaded by `center`), on the
        // main thread.
        unsafe {
            let c_cls = cls(c"UNMutableNotificationContent")?;
            let a: Allocated<AnyObject> = msg_send![c_cls, alloc];
            let content: Retained<AnyObject> = msg_send![a, init];
            let _: () = msg_send![&*content, setTitle: &*NSString::from_str(&n.title)];
            let _: () = msg_send![&*content, setBody: &*NSString::from_str(&n.body)];
            if let Some(act) = &n.action {
                let dict_cls = cls(c"NSDictionary")?;
                let v = NSString::from_str(act);
                let k = NSString::from_str("action");
                let info: *mut AnyObject =
                    msg_send![dict_cls, dictionaryWithObject: &*v, forKey: &*k];
                let _: () = msg_send![&*content, setUserInfo: info];
            }
            if n.sound {
                let s_cls = cls(c"UNNotificationSound")?;
                let snd: *mut AnyObject = msg_send![s_cls, defaultSound];
                let _: () = msg_send![&*content, setSound: snd];
            }
            let r_cls = cls(c"UNNotificationRequest")?;
            let none: *const AnyObject = std::ptr::null();
            let req: *mut AnyObject = msg_send![r_cls, requestWithIdentifier: &*NSString::from_str(id), content: &*content, trigger: none];
            let nb: *const AnyObject = std::ptr::null();
            let _: () = msg_send![center, addNotificationRequest: req, withCompletionHandler: nb];
        }
        Ok(true)
    }

    fn apple_string(t: &str) -> String {
        format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pairs: Vec<(&str, Value)>) -> Value {
        map(pairs)
    }

    #[test]
    fn looks_are_read_and_checked() {
        let l = look_of(
            "t",
            Some(&m(vec![("title", s("1.2 GB")), ("dot", s("amber"))])),
        )
        .unwrap();
        assert_eq!(l.title.as_deref(), Some("1.2 GB"));
        assert_eq!(l.dot.as_deref(), Some("amber"));
        assert!(l.icon.is_none());
        let none = look_of("t", Some(&m(vec![("dot", Value::Unit)]))).unwrap();
        assert_eq!(none.dot.as_deref(), Some(""));
        assert!(look_of("t", Some(&m(vec![("colour", s("x"))]))).is_err());
        assert!(look_of("t", Some(&m(vec![("dot", s("purple-ish"))]))).is_err());
        assert!(look_of("t", Some(&m(vec![("dot", s("#ff8800"))]))).is_ok());
    }

    #[test]
    fn notes_need_words() {
        assert!(note_of("t", Some(&m(vec![]))).is_err());
        assert!(note_of("t", None).is_err());
        let n = note_of(
            "t",
            Some(&m(vec![("title", s("Studio")), ("action", s("open"))])),
        )
        .unwrap();
        assert_eq!(n.action.as_deref(), Some("open"));
        assert!(note_of("t", Some(&m(vec![("title", s("a")), ("bell", s("x"))]))).is_err());
    }

    #[test]
    fn handles_round_trip() {
        assert_eq!(status_id("t", Some(&status_value(7))).unwrap(), 7);
        assert!(status_id("t", Some(&s("7"))).is_err());
    }

    #[test]
    fn a_dismissed_panel_was_open_a_moment() {
        note_panel(990_001, true);
        assert!(panel_was_open(990_001));
        note_panel(990_001, false);
        assert!(panel_was_open(990_001));
        assert!(!panel_was_open(990_002));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn elsewhere_it_is_unsupported() {
        let r = gui_status_item(&[]).unwrap();
        assert!(matches!(r, Value::Err(_)));
        let r = gui_notify(&[m(vec![("title", s("x"))])]).unwrap();
        assert!(matches!(r, Value::Err(_)));
    }
}

//! `gui` — windows drawn by olang: Loom's engine (loom/SPEC.md §16).
//!
//! The program lays its view out (in olang) and sends the engine
//! positioned, keyed nodes; the engine draws them on the GPU (or with
//! the software reference), edits text fields, hit-tests, keeps the
//! platform's accessibility tree, and sends back events on one channel.
//!
//!   let w = unwrap(gui.open(#{ "title": "Hello", "size": (480, 320) }))
//!   gui.apply(w, [#{ "key": "root", "role": "group", "box": (0, 0, 480, 320),
//!                     "style": #{ "bg": "#ffffff" } },
//!                 #{ "key": "hi", "parent": "root", "role": "text", "box": (24, 24, 300, 20),
//!                     "text": "Hello" }])
//!   let events = gui.events()
//!   ... chan.recv(events) ...
//!
//! The platform's event loop must own the process's first thread on
//! macOS. olang already runs every program on a thread of its own, so
//! the first thread is free: the first `gui.open` asks it to start the
//! loop ([`platform`]); a program that never opens a window never
//! starts one. Headless windows (`gui.headless`) need no loop and no
//! display: they draw with the software renderer, take input from
//! `gui.input`, and are what `loom.test` drives.
//!
//! Module map: [`values`] (reading and making olang values), [`text`]
//! (shaping, measuring), [`raster`] (glyph images), [`scene`] (the node
//! tree and patches), [`edit`] (text fields), [`window`] (one window's
//! state, input handling, the display list), [`soft`] and [`gpu`] (the
//! renderers), [`a11y`] (the accessibility tree), [`platform`] (the
//! event loop, platform windows, dialogs, menus).

pub mod a11y;
pub mod canvas;
pub mod colr;
pub mod context;
pub mod edit;
pub mod flat;
pub mod gpu;
pub mod picture;
pub mod platform;
pub mod raster;
pub mod rich;
pub mod scene;
pub mod soft;
pub mod text;
pub mod values;
pub mod window;

use crate::ast::Value;
use crate::stdlib::chan::ChannelSender;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use values::*;
use window::{Input, Mods, PointerAction, WinState};

type DynRes = Result<Value, Box<dyn std::error::Error>>;

const FUNCTIONS: &[(&str, usize)] = &[
    ("available", 0),
    ("open", 1),
    ("headless", 1),
    ("close", 1),
    ("events", 0),
    ("apply", 2),
    ("measure", 3),
    ("read", 3),
    ("input", 2),
    ("set", 2),
    ("fonts", 2),
    ("clipboard_read", 0),
    ("clipboard_write", 1),
    ("clipboard_image", 0),
    ("image_info", 1),
    ("dialog", 2),
    ("menu", 1),
    ("context_menu", 3),
    ("compare", 3),
    ("context", 0),
    ("platform", 0),
    ("wake", 0),
    ("prepare", 0),
    ("flatten", 4),
    ("flat_emit", 5),
    ("flat_join", 4),
    ("flat_arrays", 1),
    ("flat_keep", 2),
    ("flat_geo", 1),
    ("roles", 0),
];

pub fn create_gui_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in FUNCTIONS {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("gui.{}", name),
                arity: *arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

pub fn call_gui_function(name: &str, args: Vec<Value>) -> DynRes {
    let r = match name {
        "available" => gui_available(args),
        "open" => gui_open(args),
        "headless" => gui_headless(args),
        "close" => gui_close(args),
        "events" => gui_events(args),
        "apply" => gui_apply(args),
        "measure" => gui_measure(args),
        "flatten" => flat::gui_flatten(args),
        "flat_emit" => flat::gui_flat_emit(args),
        "flat_join" => flat::gui_flat_join(args),
        "flat_arrays" => flat::gui_flat_arrays(args),
        "flat_keep" => flat::gui_flat_keep(args),
        "flat_geo" => flat::gui_flat_geo(args),
        "roles" => flat::gui_roles(args),
        "read" => gui_read(args),
        "compare" => gui_compare(args),
        // A `wake` event on the channel: what lets a task blocked on
        // gui.events() see that it should stop, with no polling.
        "wake" => {
            arity("gui.wake", &args, 0, 0)?;
            emit(vec![event("wake", vec![])]);
            Ok(Value::Unit)
        }
        "prepare" => {
            arity("gui.prepare", &args, 0, 0)?;
            platform::prepare();
            Ok(Value::Unit)
        }
        "context" => {
            arity("gui.context", &args, 0, 0)?;
            Ok(context::as_value(None))
        }
        "platform" => {
            arity("gui.platform", &args, 0, 0)?;
            Ok(platform::capabilities())
        }
        "input" => gui_input(args),
        "set" => gui_set(args),
        "fonts" => gui_fonts(args),
        "clipboard_read" => gui_clipboard_read(args),
        "clipboard_write" => gui_clipboard_write(args),
        "clipboard_image" => gui_clipboard_image(args),
        "image_info" => gui_image_info(args),
        "dialog" => gui_dialog(args),
        "menu" => gui_menu(args),
        "context_menu" => gui_context_menu(args),
        _ => Err(format!("Unknown gui function: {name}")),
    };
    r.map_err(|e| e.into())
}

// ── the registry and the events channel ─────────────────────────────

pub type Shared = Arc<Mutex<WinState>>;

static WINDOWS: OnceLock<Mutex<HashMap<u64, Shared>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static EVENTS: Mutex<Option<(Value, ChannelSender)>> = Mutex::new(None);

pub fn windows() -> &'static Mutex<HashMap<u64, Shared>> {
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

fn window(function: &str, id: u64) -> Res<Shared> {
    windows()
        .lock()
        .map_err(|_| "gui: registry poisoned".to_string())?
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("{function}: window {id} is closed"))
}

fn events_channel() -> Value {
    let mut slot = EVENTS.lock().unwrap_or_else(|p| p.into_inner());
    if slot.is_none() {
        *slot = Some(crate::stdlib::chan::channel_with_sender());
    }
    slot.as_ref().expect("set").0.clone()
}

/// Send events to the program. Before `gui.events()` was first called
/// nobody listens; the channel is made here so nothing is lost.
pub fn emit(events: Vec<Value>) {
    if events.is_empty() {
        return;
    }
    let mut slot = EVENTS.lock().unwrap_or_else(|p| p.into_inner());
    if slot.is_none() {
        *slot = Some(crate::stdlib::chan::channel_with_sender());
    }
    let sender = &slot.as_ref().expect("set").1;
    for e in events {
        sender.send(e);
    }
}

fn arity(function: &str, args: &[Value], min: usize, max: usize) -> Res<()> {
    if args.len() < min || args.len() > max {
        let want = if min == max {
            min.to_string()
        } else {
            format!("{min} to {max}")
        };
        return Err(format!(
            "{function}: expected {want} argument(s), got {}",
            args.len()
        ));
    }
    Ok(())
}

// ── the functions ───────────────────────────────────────────────────

fn gui_available(args: Vec<Value>) -> Res<Value> {
    arity("gui.available", &args, 0, 0)?;
    Ok(Value::Boolean(platform::available()))
}

/// The options `gui.open` and `gui.headless` share.
pub struct OpenOpts {
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub min: Option<(f32, f32)>,
    pub resizable: bool,
    pub renderer: String,
    pub scale: f32,
    pub clear: Option<text::Color>,
    /// The windows this one may share a tab bar with (macOS: the
    /// window's `tabbingIdentifier`).
    pub tabbing: Option<String>,
    /// An open window this one joins as a tab (macOS), by its id.
    pub tab_of: Option<u64>,
}

fn open_opts(function: &str, v: Option<&Value>) -> Res<OpenOpts> {
    let mut o = OpenOpts {
        title: "olang".to_string(),
        width: 800.0,
        height: 600.0,
        min: None,
        resizable: true,
        renderer: std::env::var("LOOM_RENDERER").unwrap_or_else(|_| "gpu".to_string()),
        scale: 1.0,
        clear: None,
        tabbing: None,
        tab_of: None,
    };
    let Some(v) = v else {
        return Ok(o);
    };
    if matches!(v, Value::Unit) {
        return Ok(o);
    }
    let known = [
        "title",
        "size",
        "min",
        "resizable",
        "renderer",
        "scale",
        "background",
        "tabbing",
        "tab_of",
    ];
    match fields(v) {
        Some(f) => {
            for k in f.keys() {
                if !known.contains(&k.as_str()) {
                    return Err(format!(
                        "{function}: unknown option \"{k}\" (the options are {})",
                        known.join(", ")
                    ));
                }
            }
        }
        None => return Err(format!("{function}: options must be a map")),
    }
    if let Some(t) = get_str(v, "title", function)? {
        o.title = t.to_string();
    }
    if let Some((w, h)) = get_pair(v, "size", function)? {
        o.width = w.max(1.0);
        o.height = h.max(1.0);
    }
    o.min = get_pair(v, "min", function)?;
    if let Some(b) = get_bool(v, "resizable", function)? {
        o.resizable = b;
    }
    if let Some(r) = get_str(v, "renderer", function)? {
        if r != "gpu" && r != "software" {
            return Err(format!(
                "{function}: \"renderer\" is \"gpu\" or \"software\""
            ));
        }
        o.renderer = r.to_string();
    }
    if let Some(sc) = get_num(v, "scale", function)? {
        o.scale = sc.clamp(0.5, 4.0);
    }
    o.clear = get_color(v, "background", function)?;
    o.tabbing = get_str(v, "tabbing", function)?.map(str::to_string);
    o.tab_of = match get_num(v, "tab_of", function)? {
        Some(n) if n >= 1.0 => Some(n as u64),
        _ => None,
    };
    Ok(o)
}

fn gui_open(args: Vec<Value>) -> Res<Value> {
    arity("gui.open", &args, 0, 1)?;
    let opts = open_opts("gui.open", args.first())?;
    match platform::open(opts) {
        Ok(id) => Ok(ok(window_value(id, false))),
        Err(e) => Ok(err(e)),
    }
}

fn gui_headless(args: Vec<Value>) -> Res<Value> {
    arity("gui.headless", &args, 0, 1)?;
    let opts = open_opts("gui.headless", args.first())?;
    let id = next_id();
    let mut st = WinState::new(id, true, &opts.title, opts.width, opts.height, opts.scale);
    if let Some(c) = opts.clear {
        st.clear = c;
    }
    windows()
        .lock()
        .map_err(|_| "gui: registry poisoned")?
        .insert(id, Arc::new(Mutex::new(st)));
    Ok(window_value(id, true))
}

fn gui_close(args: Vec<Value>) -> Res<Value> {
    arity("gui.close", &args, 1, 1)?;
    let id = window_id("gui.close", args.first())?;
    let removed = windows()
        .lock()
        .map_err(|_| "gui: registry poisoned")?
        .remove(&id);
    if let Some(w) = removed {
        let headless = w.lock().map(|s| s.headless).unwrap_or(true);
        if !headless {
            platform::close(id);
        }
    }
    Ok(Value::Unit)
}

fn gui_events(args: Vec<Value>) -> Res<Value> {
    arity("gui.events", &args, 0, 0)?;
    Ok(events_channel())
}

fn gui_apply(args: Vec<Value>) -> Res<Value> {
    arity("gui.apply", &args, 2, 2)?;
    let id = window_id("gui.apply", args.first())?;
    let w = window("gui.apply", id)?;
    let mut out = Vec::new();
    let (result, headless) = {
        let mut st = w.lock().map_err(|_| "gui: window poisoned")?;
        (st.apply(&args[1], &mut out), st.headless)
    };
    emit(out);
    if !headless {
        platform::redraw(id);
    }
    Ok(match result {
        Ok(()) => ok(Value::Unit),
        Err(e) => err(e),
    })
}

fn gui_measure(args: Vec<Value>) -> Res<Value> {
    arity("gui.measure", &args, 1, 3)?;
    let t = match &args[0] {
        Value::String(t) => t.as_str().to_string(),
        other => format!("{other}"),
    };
    let mut style = scene::Style::default();
    if let Some(st) = args.get(1)
        && !matches!(st, Value::Unit)
    {
        style.apply(st, "gui.measure")?;
    }
    let (width, scale) = match args.get(2) {
        None | Some(Value::Unit) => (None, 1.0),
        Some(o) => (
            get_num(o, "width", "gui.measure")?,
            get_num(o, "scale", "gui.measure")?.unwrap_or(1.0),
        ),
    };
    let max_w = if style.wrap { width } else { None };
    let mut ts = text::system().lock().map_err(|_| "gui: text poisoned")?;
    let shaped = ts.shape(
        &t,
        &style.font,
        style.color,
        max_w,
        text::Align::Start,
        scale,
    );
    let (w, h) = shaped.logical_size();
    let baseline = shaped
        .lines
        .first()
        .map(|l| l.baseline / scale)
        .unwrap_or(0.0);
    Ok(map(vec![
        ("width", float(w)),
        ("height", float(h)),
        ("lines", Value::Integer(shaped.lines.len() as i64)),
        ("baseline", float(baseline)),
    ]))
}

fn gui_read(args: Vec<Value>) -> Res<Value> {
    arity("gui.read", &args, 2, 3)?;
    let id = window_id("gui.read", args.first())?;
    let what = match &args[1] {
        Value::String(t) => t.as_str().to_string(),
        other => {
            return Err(format!(
                "gui.read: what to read must be a String, got {}",
                other.type_name()
            ));
        }
    };
    let w = window("gui.read", id)?;
    let mut st = w.lock().map_err(|_| "gui: window poisoned")?;
    let arg = args.get(2);
    let key_arg = || -> Res<String> {
        match arg {
            Some(Value::String(k)) => Ok(k.to_string()),
            _ => Err(format!("gui.read: \"{what}\" needs a node key")),
        }
    };
    Ok(match what.as_str() {
        "focus" => opt_str(st.scene.focus.as_deref()),
        "title" => opt_str(Some(st.title.as_str())),
        "hover" => opt_str(st.scene.hover.as_deref()),
        "size" => Value::Tuple(Arc::new(vec![
            float(st.width),
            float(st.height),
            float(st.scale),
        ])),
        "hit" => {
            let p = arg
                .and_then(nums)
                .filter(|v| v.len() == 2)
                .ok_or("gui.read: \"hit\" needs a point (x, y)")?;
            opt_str(st.scene.hit(p[0], p[1]).as_deref())
        }
        "node" => {
            let k = key_arg()?;
            match st.scene.node_value(&k) {
                // and what of it shows, inside the window
                Some(v) => {
                    let (w, h) = (st.width, st.height);
                    let shown = st.scene.visible(&k).and_then(|[x, y, bw, bh]| {
                        let (x0, y0, x1, y1) =
                            (x.max(0.0), y.max(0.0), (x + bw).min(w), (y + bh).min(h));
                        (x1 > x0 && y1 > y0).then(|| {
                            Value::Tuple(Arc::new(vec![
                                float(x0),
                                float(y0),
                                float(x1 - x0),
                                float(y1 - y0),
                            ]))
                        })
                    });
                    match &v {
                        Value::Map(m) => {
                            let mut m = (**m).clone();
                            m.insert("visible".to_string(), shown.unwrap_or(Value::Unit));
                            Value::Map(Arc::new(m))
                        }
                        _ => v,
                    }
                }
                None => Value::Unit,
            }
        }
        "value" => {
            let k = key_arg()?;
            match st.editors.get(&k) {
                Some(e) => s(&e.value()),
                None => Value::Unit,
            }
        }
        "selection" => {
            let k = key_arg()?;
            match st.editors.get(&k) {
                Some(e) => {
                    let (a, f) = e.selection_chars();
                    Value::Tuple(Arc::new(vec![
                        Value::Integer(a as i64),
                        Value::Integer(f as i64),
                    ]))
                }
                None => Value::Unit,
            }
        }
        "keys" => Value::List(Arc::new(
            st.scene.focus_order().iter().map(|k| s(k)).collect(),
        )),
        "a11y" => a11y::as_value(&mut st),
        "pixels" => {
            let dl = st.display_list();
            drop(st);
            let pm = soft::render(&dl);
            let png = pm
                .encode_png()
                .map_err(|e| format!("gui.read: encoding the frame: {e}"))?;
            crate::stdlib::bytes::to_value(png)
        }
        "rgba" => {
            let dl = st.display_list();
            drop(st);
            let pm = soft::render(&dl);
            map(vec![
                ("width", Value::Integer(pm.width() as i64)),
                ("height", Value::Integer(pm.height() as i64)),
                ("data", crate::stdlib::bytes::to_value(soft::rgba(&pm))),
            ])
        }
        "prims" => {
            let dl = st.display_list();
            Value::Integer(dl.prims.len() as i64)
        }
        "ime" => match st.ime_area() {
            Some([x, y, w, h]) => {
                Value::Tuple(Arc::new(vec![float(x), float(y), float(w), float(h)]))
            }
            None => Value::Unit,
        },
        "caret" => {
            st.display_list();
            match st.caret {
                Some([x, y, w, h]) => {
                    Value::Tuple(Arc::new(vec![float(x), float(y), float(w), float(h)]))
                }
                None => Value::Unit,
            }
        }
        // an animated picture as the last frame drew it: `#{ frames,
        // frame, paused, button }` (`button` its place, when it has one),
        // or () for a picture that does not animate (or not yet)
        "animation" => {
            let k = key_arg()?;
            st.display_list();
            match st.anim_shown(&k) {
                Some(a) => map(vec![
                    ("frames", Value::Integer(a.frames as i64)),
                    ("frame", Value::Integer(a.frame as i64)),
                    ("paused", Value::Boolean(a.paused)),
                    (
                        "button",
                        a.badge
                            .map(|b| Value::Tuple(Arc::new(b.iter().map(|v| float(*v)).collect())))
                            .unwrap_or(Value::Unit),
                    ),
                ]),
                None => Value::Unit,
            }
        }
        // when the next frame of an animation in view is due (ms, the
        // window's clock), or ()
        "next_frame" => {
            st.display_list();
            st.next_frame.map(|t| float(t as f32)).unwrap_or(Value::Unit)
        }
        // the system's settings as this window last heard them
        "settings" => {
            let set = st.settings;
            let mut f = vec![("dark", Value::Boolean(set.dark.unwrap_or(false)))];
            f.extend([
                ("contrast", Value::Boolean(set.contrast)),
                ("reduce_motion", Value::Boolean(set.reduce_motion)),
                ("reduce_transparency", Value::Boolean(set.reduce_transparency)),
                ("accent", set.accent.map(|c| s(&context::hex(c))).unwrap_or(Value::Unit)),
            ]);
            map(f)
        }
        other => {
            return Err(format!(
                "gui.read: unknown \"{other}\" (focus, hover, size, hit, node, value, selection, keys, a11y, pixels, rgba, prims, caret, ime, settings, animation, next_frame)"
            ));
        }
    })
}

/// The clipboard of input sent by `gui.input`: the window's own, so a
/// test never reads or clobbers the user's, nor another test's.
pub struct OwnClipboard(pub Option<String>);

impl window::Clipboard for OwnClipboard {
    fn get(&mut self) -> Option<String> {
        self.0.clone()
    }
    fn set(&mut self, text: String) {
        self.0 = Some(text);
    }
}

fn mods_of(v: &Value) -> Res<Mods> {
    let what = "gui.input";
    let m = get(v, "mods");
    let flag = |k: &str| -> Res<bool> {
        match m {
            Some(m) => Ok(get_bool(m, k, what)?.unwrap_or(false)),
            None => Ok(get_bool(v, k, what)?.unwrap_or(false)),
        }
    };
    let mut mods = Mods {
        ctrl: flag("ctrl")?,
        alt: flag("alt")?,
        shift: flag("shift")?,
        super_: flag("super")?,
    };
    // `mod` is the platform's command key, as Loom's chords spell it.
    if flag("mod")? {
        if cfg!(target_os = "macos") {
            mods.super_ = true;
        } else {
            mods.ctrl = true;
        }
    }
    Ok(mods)
}

/// An input event from a map, as a test or an automation sends it.
fn input_of(v: &Value) -> Res<Input> {
    let what = "gui.input";
    let kind = get_str(v, "kind", what)?.ok_or("gui.input: the event needs a \"kind\"")?;
    Ok(match kind {
        "key" => Input::Key {
            key: get_str(v, "key", what)?
                .ok_or("gui.input: a key event needs a \"key\"")?
                .to_string(),
            text: get_str(v, "text", what)?.map(str::to_string),
            mods: mods_of(v)?,
            repeat: get_bool(v, "repeat", what)?.unwrap_or(false),
        },
        "text" => {
            // Typed text without a key: each character as its own key.
            let t = get_str(v, "text", what)?.ok_or("gui.input: \"text\" needs a \"text\"")?;
            Input::ImeCommit(t.to_string())
        }
        // The modifier keys held (a press with ⌘ held, as a test sends it).
        "modifiers" => Input::Modifiers(mods_of(v)?),
        "pointer" => {
            let action = match get_str(v, "action", what)?.unwrap_or("move") {
                "down" => PointerAction::Down,
                "up" => PointerAction::Up,
                "move" => PointerAction::Move,
                "leave" => PointerAction::Leave,
                other => return Err(format!("gui.input: unknown pointer action \"{other}\"")),
            };
            Input::Pointer {
                action,
                x: get_num(v, "x", what)?.unwrap_or(0.0),
                y: get_num(v, "y", what)?.unwrap_or(0.0),
                button: get_str(v, "button", what)?.unwrap_or("left").to_string(),
                clicks: get_num(v, "clicks", what)?.map(|n| n as u32).unwrap_or(1),
            }
        }
        "wheel" => Input::Wheel {
            dx: get_num(v, "dx", what)?.unwrap_or(0.0),
            dy: get_num(v, "dy", what)?.unwrap_or(0.0),
            // a test's modifiers, when it says any
            mods: if ["mods", "mod", "shift", "alt", "ctrl", "super"]
                .iter()
                .any(|k| get(v, k).is_some())
            {
                Some(mods_of(v)?)
            } else {
                None
            },
        },
        // A trackpad's pinch: `delta` the change of magnification
        // (positive zooms in), `phase` start, move, end, or cancel.
        "pinch" => Input::Pinch {
            delta: get_num(v, "delta", what)?.unwrap_or(0.0),
            phase: get_str(v, "phase", what)?.unwrap_or("move").to_string(),
        },
        "compose" => Input::ImePreedit(get_str(v, "text", what)?.unwrap_or("").to_string(), None),
        "commit" => Input::ImeCommit(get_str(v, "text", what)?.unwrap_or("").to_string()),
        "resize" => Input::Resize {
            width: get_num(v, "width", what)?.ok_or("gui.input: resize needs \"width\"")?,
            height: get_num(v, "height", what)?.ok_or("gui.input: resize needs \"height\"")?,
            scale: get_num(v, "scale", what)?.unwrap_or(1.0),
        },
        "window_focus" => Input::Focused(get_bool(v, "on", what)?.unwrap_or(true)),
        // Files from another program: `action` hover, drop, or cancel,
        // `paths`, and where (`x`, `y`; else the pointer's last place).
        "files" => {
            let action = get_str(v, "action", what)?.unwrap_or("drop");
            if !["hover", "drop", "cancel"].contains(&action) {
                return Err(format!(
                    "gui.input: unknown files action \"{action}\" (hover, drop, cancel)"
                ));
            }
            let paths = match get(v, "paths") {
                Some(Value::List(l)) => l
                    .iter()
                    .map(|p| match p {
                        Value::String(t) => Ok(t.to_string()),
                        other => Err(format!("gui.input: a file's path is a String, got {other}")),
                    })
                    .collect::<Res<Vec<_>>>()?,
                None => vec![],
                Some(other) => return Err(format!("gui.input: \"paths\" is a list, got {other}")),
            };
            let at = match (get_num(v, "x", what)?, get_num(v, "y", what)?) {
                (Some(x), Some(y)) => Some((x, y)),
                _ => None,
            };
            Input::Files {
                action: action.to_string(),
                paths,
                at,
            }
        }
        other => {
            return Err(format!(
                "gui.input: unknown kind \"{other}\" (key, text, pointer, wheel, pinch, compose, commit, resize, window_focus, files, close, menu, open, new_tab, clipboard, appearance, place, a11y)"
            ));
        }
    })
}

fn gui_input(args: Vec<Value>) -> Res<Value> {
    arity("gui.input", &args, 2, 2)?;
    let id = window_id("gui.input", args.first())?;
    let w = window("gui.input", id)?;
    // What a platform does around a window, for a test to do instead.
    let what = "gui.input";
    match get_str(&args[1], "kind", what)? {
        Some("close") => {
            emit(vec![event(
                "close_requested",
                vec![("window", Value::Integer(id as i64))],
            )]);
            return Ok(Value::Unit);
        }
        // A script's choice in the window's next context menu: the native
        // menu shows, then closes as though the item `label` was chosen
        // (nothing else can drive it).
        Some("context_pick") => {
            let label = get_str(&args[1], "label", what)?
                .ok_or("gui.input: \"context_pick\" needs the item's \"label\"")?;
            platform::context_pick(id, label.to_string());
            return Ok(Value::Unit);
        }
        Some("menu") => {
            let item = get_str(&args[1], "id", what)?
                .ok_or("gui.input: \"menu\" needs the item's \"id\"")?;
            emit(vec![event("menu", vec![("id", s(item))])]);
            return Ok(Value::Unit);
        }
        // The application asked to open files and folders (the Finder's
        // Open With, a drop on the Dock icon): `paths`.
        Some("open") => {
            let paths = match get(&args[1], "paths") {
                Some(Value::List(l)) => l.clone(),
                _ => return Err("gui.input: \"open\" needs \"paths\", a list".into()),
            };
            emit(vec![event("open", vec![("paths", Value::List(paths))])]);
            return Ok(Value::Unit);
        }
        // The tab bar's + button.
        Some("new_tab") => {
            emit(vec![event("new_tab", vec![])]);
            return Ok(Value::Unit);
        }
        // The system's settings changing, as a platform window reports
        // them: `dark`, `contrast`, `reduce_motion`, `reduce_transparency`
        // (each false when left out), `accent` ("#rrggbb", or none). The
        // window takes them (its pictures stop under reduced motion) and
        // the program hears `appearance`.
        Some("appearance") => {
            let b = |k: &str| -> Res<bool> { Ok(get_bool(&args[1], k, what)?.unwrap_or(false)) };
            let accent = match get_str(&args[1], "accent", what)? {
                Some(t) => Some(context::parse_hex(t).ok_or_else(|| {
                    format!("{what}: \"accent\" is a colour \"#rrggbb\", not \"{t}\"")
                })?),
                None => None,
            };
            let set = context::Settings {
                dark: Some(b("dark")?),
                contrast: b("contrast")?,
                reduce_motion: b("reduce_motion")?,
                reduce_transparency: b("reduce_transparency")?,
                accent,
            };
            w.lock().map_err(|_| "gui: window poisoned")?.set_settings(set);
            emit(vec![context::event_of(id, set, None, "test")]);
            return Ok(Value::Unit);
        }
        // A headless window put at a place on the screen (a real one is
        // where the platform says): `moved` follows, as when a person
        // drags a window.
        Some("place") => {
            let mut out = Vec::new();
            {
                let mut st = w.lock().map_err(|_| "gui: window poisoned")?;
                if !st.headless {
                    return Err("gui.input: \"place\" places a headless window; a real window's place is the platform's".into());
                }
                let x = get_num(&args[1], "x", what)?.unwrap_or(0.0);
                let y = get_num(&args[1], "y", what)?.unwrap_or(0.0);
                st.place(x, y, &mut out);
            }
            emit(out);
            return Ok(Value::Unit);
        }
        // An assistive action, as a screen reader asks for it: `action`
        // ("click", "focus", "set_value" with `value`, "increment",
        // "decrement", "custom" with `index`) on the node `key`.
        Some("a11y") => {
            use accesskit::{Action, ActionData, ActionRequest, TreeId};
            let key = get_str(&args[1], "key", what)?
                .ok_or("gui.input: \"a11y\" needs the node's \"key\"")?;
            let (action, data) = match get_str(&args[1], "action", what)?.unwrap_or("click") {
                "click" => (Action::Click, None),
                "focus" => (Action::Focus, None),
                "increment" => (Action::Increment, None),
                "decrement" => (Action::Decrement, None),
                "set_value" => (
                    Action::SetValue,
                    Some(ActionData::Value(
                        get_str(&args[1], "value", what)?.unwrap_or("").into(),
                    )),
                ),
                "custom" => (
                    Action::CustomAction,
                    Some(ActionData::CustomAction(
                        get_num(&args[1], "index", what)?.unwrap_or(0.0) as i32,
                    )),
                ),
                other => {
                    return Err(format!(
                        "gui.input: unknown assistive action \"{other}\" (click, focus, set_value, increment, decrement, custom)"
                    ));
                }
            };
            let mut out = Vec::new();
            {
                let mut st = w.lock().map_err(|_| "gui: window poisoned")?;
                if !st.scene.nodes.contains_key(key) {
                    return Err(format!(
                        "gui.input: no node \"{key}\" for the assistive action"
                    ));
                }
                let req = ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: a11y::node_id(key),
                    data,
                };
                a11y::action(&mut st, &req, &mut out);
            }
            emit(out);
            return Ok(Value::Unit);
        }
        // A headless window's clock for its animations, in ms: they move
        // only when a test moves it (a real window's is the time).
        Some("clock") => {
            let ms = get_num(&args[1], "ms", what)?
                .ok_or("gui.input: \"clock\" needs \"ms\"")?;
            let mut st = w.lock().map_err(|_| "gui: window poisoned")?;
            if !st.headless {
                return Err("gui.input: \"clock\" sets a headless window's clock; a real window's is the time".into());
            }
            st.set_clock(ms as f64);
            return Ok(Value::Unit);
        }
        Some("clipboard") => {
            if !w.lock().map_err(|_| "gui: window poisoned")?.headless {
                return Err("gui.input: \"clipboard\" sets a headless window's clipboard; a real window's is the platform's".into());
            }
            let t = get_str(&args[1], "text", what)?.unwrap_or("").to_string();
            w.lock().map_err(|_| "gui: window poisoned")?.clip = Some(t);
            return Ok(Value::Unit);
        }
        _ => {}
    }
    let input = input_of(&args[1])?;
    let mut out = Vec::new();
    let headless = {
        let mut st = w.lock().map_err(|_| "gui: window poisoned")?;
        let headless = st.headless;
        let mut clip = OwnClipboard(st.clip.take());
        st.input(input, &mut clip, &mut out);
        st.clip = clip.0;
        headless
    };
    emit(out);
    if !headless {
        platform::redraw(id);
    }
    Ok(Value::Unit)
}

fn gui_set(args: Vec<Value>) -> Res<Value> {
    arity("gui.set", &args, 2, 2)?;
    let id = window_id("gui.set", args.first())?;
    let w = window("gui.set", id)?;
    let v = &args[1];
    let what = "gui.set";
    let headless = {
        let mut st = w.lock().map_err(|_| "gui: window poisoned")?;
        if let Some(t) = get_str(v, "title", what)? {
            st.title = t.to_string();
        }
        if let Some(c) = get_color(v, "background", what)? {
            st.clear = c;
            st.dirty = true;
        }
        if let Some(on) = get_bool(v, "scripted", what)? {
            st.scripted = on;
        }
        if let Some(z) = get_num(v, "zoom", what)? {
            let mut out = Vec::new();
            st.set_zoom(z, &mut out);
            emit(out);
        }
        if st.headless
            && let Some((w, h)) = get_pair(v, "size", what)?
        {
            st.width = w.max(1.0);
            st.height = h.max(1.0);
            st.dirty = true;
        }
        st.headless
    };
    if !headless {
        platform::set(id, v.clone());
    }
    Ok(Value::Unit)
}

fn gui_fonts(args: Vec<Value>) -> Res<Value> {
    arity("gui.fonts", &args, 1, 2)?;
    let items: Vec<Value> = match &args[0] {
        Value::List(l) => l.as_ref().clone(),
        other => vec![other.clone()],
    };
    let background = match args.get(1) {
        None | Some(Value::Unit) => false,
        Some(Value::Map(m)) => matches!(m.get("background"), Some(Value::Boolean(true))),
        Some(_) => return Err("gui.fonts: the options are a map (#{ \"background\": true })".into()),
    };
    if background {
        return fonts_in_background(items);
    }
    let mut ts = text::system().lock().map_err(|_| "gui: text poisoned")?;
    let mut total = 0;
    for item in items {
        let bytes = match &item {
            Value::String(path) => std::fs::read(path.as_str())
                .map_err(|e| format!("gui.fonts: reading {path}: {e}"))?,
            other => crate::stdlib::bytes::bytes_of(other)
                .map_err(|_| "gui.fonts: each font is a path (String) or Bytes".to_string())?
                .to_vec(),
        };
        let bytes = if bytes.starts_with(b"wOF2") {
            return Err("gui.fonts: WOFF2 is not read; give a TTF, OTF, or TTC".into());
        } else {
            maybe_brotli(bytes)?
        };
        total += ts.register(bytes);
    }
    Ok(Value::Integer(total as i64))
}

/// `gui.fonts(sources, #{ "background": true })`: the sources are read
/// here (a path that cannot be read is this call's error), then
/// decompressed and registered on a thread of their own — the text
/// system's first use (discovering the system's fonts) with them —
/// while the program goes on; whatever shapes or measures text first
/// waits until they are in. Answers the number of sources handed over.
/// Loom's bundled fonts were most of a window's first 60 ms this way
/// (brotli, 15 MB of faces, and the system's fonts found).
fn fonts_in_background(items: Vec<Value>) -> Res<Value> {
    let mut sources: Vec<Vec<u8>> = Vec::with_capacity(items.len());
    for item in &items {
        let bytes = match item {
            Value::String(path) => std::fs::read(path.as_str())
                .map_err(|e| format!("gui.fonts: reading {path}: {e}"))?,
            other => crate::stdlib::bytes::bytes_of(other)
                .map_err(|_| "gui.fonts: each font is a path (String) or Bytes".to_string())?
                .to_vec(),
        };
        if bytes.starts_with(b"wOF2") {
            return Err("gui.fonts: WOFF2 is not read; give a TTF, OTF, or TTC".into());
        }
        sources.push(bytes);
    }
    let n = sources.len();
    let pending = text::FontsPending::begin();
    std::thread::Builder::new()
        .name("gui-fonts".to_string())
        .spawn(move || {
            let _pending = pending;
            // each decompressed on a thread of its own (the emoji face
            // alone is most of the work); the text system made meanwhile
            let fonts: Vec<Res<Vec<u8>>> = std::thread::scope(|scope| {
                let jobs: Vec<_> = sources
                    .into_iter()
                    .map(|b| scope.spawn(move || maybe_brotli(b)))
                    .collect();
                let _ = text::system_now();
                jobs.into_iter()
                    .map(|j| j.join().unwrap_or_else(|_| Err("gui.fonts: a font's reader stopped".into())))
                    .collect()
            });
            let Ok(mut ts) = text::system_now().lock() else {
                return;
            };
            for font in fonts {
                match font {
                    Ok(bytes) => {
                        ts.register(bytes);
                    }
                    Err(e) => eprintln!("{e}"),
                }
            }
        })
        .map_err(|e| format!("gui.fonts: starting the thread: {e}"))?;
    Ok(Value::Integer(n as i64))
}

/// Fonts may arrive brotli-compressed (Loom embeds them so).
fn maybe_brotli(bytes: Vec<u8>) -> Res<Vec<u8>> {
    let is_font = |b: &[u8]| {
        b.starts_with(&[0, 1, 0, 0])
            || b.starts_with(b"OTTO")
            || b.starts_with(b"ttcf")
            || b.starts_with(b"true")
    };
    if is_font(&bytes) {
        return Ok(bytes);
    }
    let mut out = Vec::new();
    let mut r = brotli::Decompressor::new(&bytes[..], 4096);
    use std::io::Read;
    if r.read_to_end(&mut out).is_ok() && is_font(&out) {
        return Ok(out);
    }
    Err("gui.fonts: not a font (TTF, OTF, or TTC, optionally brotli-compressed)".into())
}

fn gui_clipboard_read(args: Vec<Value>) -> Res<Value> {
    arity("gui.clipboard_read", &args, 0, 0)?;
    Ok(match platform::clipboard_get() {
        Some(t) => ok(s(&t)),
        None => err("gui.clipboard_read: the clipboard holds no text"),
    })
}

fn gui_clipboard_write(args: Vec<Value>) -> Res<Value> {
    arity("gui.clipboard_write", &args, 1, 1)?;
    let t = match &args[0] {
        Value::String(t) => t.to_string(),
        other => {
            return Err(format!(
                "gui.clipboard_write: expected a String, got {}",
                other.type_name()
            ));
        }
    };
    Ok(match platform::clipboard_set(t) {
        Ok(()) => ok(Value::Unit),
        Err(e) => err(e),
    })
}

/// The image on the clipboard as PNG Bytes (a screenshot copied, an
/// image copied from another program): `Ok(#{ png, width, height })`,
/// or `Err` when it holds none. Callable from any thread.
fn gui_clipboard_image(args: Vec<Value>) -> Res<Value> {
    arity("gui.clipboard_image", &args, 0, 0)?;
    Ok(match platform::clipboard_image() {
        Ok(png) => match picture::info_of(&png) {
            Ok(i) => ok(map(vec![
                ("png", crate::stdlib::bytes::to_value(png)),
                ("width", Value::Integer(i.width as i64)),
                ("height", Value::Integer(i.height as i64)),
            ])),
            Err(e) => err(format!("gui.clipboard_image: {e}")),
        },
        Err(e) => err(format!("gui.clipboard_image: {e}")),
    })
}

/// A picture's size and format from its header: `Ok(#{ width, height,
/// format })` (`"png"`, `"jpeg"`, `"webp"`, `"gif"`, `"svg"`; an SVG's
/// size in its own units), or `Err` when it cannot be read. For a task:
/// it reads the file.
fn gui_image_info(args: Vec<Value>) -> Res<Value> {
    arity("gui.image_info", &args, 1, 1)?;
    Ok(match picture::info(&args[0]) {
        Ok(i) => ok(map(vec![
            ("width", Value::Integer(i.width as i64)),
            ("height", Value::Integer(i.height as i64)),
            ("format", s(i.format.name())),
        ])),
        Err(e) => err(e),
    })
}

fn gui_dialog(args: Vec<Value>) -> Res<Value> {
    arity("gui.dialog", &args, 1, 2)?;
    let kind = match &args[0] {
        Value::String(t) => t.to_string(),
        _ => {
            return Err(
                "gui.dialog: the kind is a String (open, open_many, save, folder, message)".into(),
            );
        }
    };
    let opts = args.get(1).cloned().unwrap_or(Value::Unit);
    platform::dialog(&kind, &opts)
}

fn gui_menu(args: Vec<Value>) -> Res<Value> {
    arity("gui.menu", &args, 1, 1)?;
    platform::menu(&args[0])
}

/// `gui.context_menu(window, items, #{ x, y })`: a native context menu
/// over the window at `(x, y)` (its content's logical pixels), each item
/// `#{ id, label, keys?, enabled?, checked?, submenu? }` or
/// `"separator"`. `true` when shown: the window then hears `context`
/// with the chosen item's `id` (`()` when dismissed) and the `items` as
/// the native menu held them. `false` on a headless window or a platform
/// without one (`gui.platform().context_menu`): the program draws its own.
fn gui_context_menu(args: Vec<Value>) -> Res<Value> {
    arity("gui.context_menu", &args, 3, 3)?;
    let id = window_id("gui.context_menu", args.first())?;
    let w = window("gui.context_menu", id)?;
    let items = platform::context_items(&args[1], 0)?;
    let x = get_num(&args[2], "x", "gui.context_menu")?.unwrap_or(0.0) as f64;
    let y = get_num(&args[2], "y", "gui.context_menu")?.unwrap_or(0.0) as f64;
    if w.lock().map_err(|_| "gui: window poisoned")?.headless {
        return Ok(Value::Boolean(false));
    }
    Ok(platform::context_menu(id, items, x, y)?)
}

/// `gui.compare(a, b, opts)`: two PNGs (Bytes) compared pixel by pixel,
/// as a pixel snapshot is checked. A pixel differs when a channel moves
/// by more than `opts.threshold` (default 32 of 255, so antialiasing and
/// a renderer's rounding pass); the images are the same when at most
/// `opts.ratio` of their pixels differ (default 0.001). Answers `#{ same,
/// differing, total, worst, diff }`, `diff` a PNG with each differing
/// pixel red over a faded copy of `a` (`()` when the sizes differ).
fn gui_compare(args: Vec<Value>) -> Res<Value> {
    arity("gui.compare", &args, 2, 3)?;
    let what = "gui.compare";
    let decode = |v: &Value, which: &str| -> Res<tiny_skia::Pixmap> {
        let b = crate::stdlib::bytes::bytes_of(v)
            .map_err(|_| format!("{what}: {which} must be a PNG's Bytes"))?;
        tiny_skia::Pixmap::decode_png(b).map_err(|e| format!("{what}: {which} is not a PNG ({e})"))
    };
    let a = decode(&args[0], "the first")?;
    let b = decode(&args[1], "the second")?;
    let opts = args.get(2).cloned().unwrap_or(Value::Unit);
    let threshold = get_num(&opts, "threshold", what)?
        .unwrap_or(32.0)
        .clamp(0.0, 255.0) as u8;
    let ratio = get_num(&opts, "ratio", what)?.unwrap_or(0.001).max(0.0) as f64;
    if a.width() != b.width() || a.height() != b.height() {
        return Ok(map(vec![
            ("same", Value::Boolean(false)),
            (
                "differing",
                Value::Integer((a.width() * a.height()).max(b.width() * b.height()) as i64),
            ),
            ("total", Value::Integer((a.width() * a.height()) as i64)),
            ("worst", Value::Integer(255)),
            ("diff", Value::Unit),
            (
                "sizes",
                s(&format!(
                    "{}×{} against {}×{}",
                    a.width(),
                    a.height(),
                    b.width(),
                    b.height()
                )),
            ),
        ]));
    }
    let mut diff =
        tiny_skia::Pixmap::new(a.width(), a.height()).ok_or("gui.compare: empty image")?;
    let (mut differing, mut worst) = (0i64, 0u8);
    for ((pa, pb), out) in a
        .data()
        .chunks(4)
        .zip(b.data().chunks(4))
        .zip(diff.data_mut().chunks_mut(4))
    {
        let d = (0..4).map(|i| pa[i].abs_diff(pb[i])).max().unwrap_or(0);
        worst = worst.max(d);
        if d > threshold {
            differing += 1;
            out.copy_from_slice(&[230, 0, 0, 255]);
        } else {
            // a faded copy, so the difference shows where it is
            let g = ((pa[0] as u16 + pa[1] as u16 + pa[2] as u16) / 3) as u8;
            let f = 255 - (255 - g) / 4;
            out.copy_from_slice(&[f, f, f, 255]);
        }
    }
    let total = (a.width() * a.height()) as i64;
    let png = diff
        .encode_png()
        .map_err(|e| format!("gui.compare: encoding the difference: {e}"))?;
    Ok(map(vec![
        (
            "same",
            Value::Boolean(differing as f64 <= ratio * total as f64),
        ),
        ("differing", Value::Integer(differing)),
        ("total", Value::Integer(total)),
        ("worst", Value::Integer(worst as i64)),
        ("diff", crate::stdlib::bytes::to_value(png)),
    ]))
}

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
pub mod edit;
pub mod gpu;
pub mod platform;
pub mod raster;
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
    ("fonts", 1),
    ("clipboard_read", 0),
    ("clipboard_write", 1),
    ("dialog", 2),
    ("menu", 1),
];

pub fn create_gui_module() -> Value {
    let mut module = HashMap::new();
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
        "read" => gui_read(args),
        "input" => gui_input(args),
        "set" => gui_set(args),
        "fonts" => gui_fonts(args),
        "clipboard_read" => gui_clipboard_read(args),
        "clipboard_write" => gui_clipboard_write(args),
        "dialog" => gui_dialog(args),
        "menu" => gui_menu(args),
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
        "node" => st.scene.node_value(&key_arg()?).unwrap_or(Value::Unit),
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
        other => {
            return Err(format!(
                "gui.read: unknown \"{other}\" (focus, hover, size, hit, node, value, selection, keys, a11y, pixels, rgba, prims)"
            ));
        }
    })
}

/// The private clipboard of headless windows.
static HEADLESS_CLIP: Mutex<Option<String>> = Mutex::new(None);

pub struct HeadlessClipboard;

impl window::Clipboard for HeadlessClipboard {
    fn get(&mut self) -> Option<String> {
        HEADLESS_CLIP.lock().ok()?.clone()
    }
    fn set(&mut self, text: String) {
        if let Ok(mut c) = HEADLESS_CLIP.lock() {
            *c = Some(text);
        }
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
        },
        "compose" => Input::ImePreedit(get_str(v, "text", what)?.unwrap_or("").to_string(), None),
        "commit" => Input::ImeCommit(get_str(v, "text", what)?.unwrap_or("").to_string()),
        "resize" => Input::Resize {
            width: get_num(v, "width", what)?.ok_or("gui.input: resize needs \"width\"")?,
            height: get_num(v, "height", what)?.ok_or("gui.input: resize needs \"height\"")?,
            scale: get_num(v, "scale", what)?.unwrap_or(1.0),
        },
        "window_focus" => Input::Focused(get_bool(v, "on", what)?.unwrap_or(true)),
        other => {
            return Err(format!(
                "gui.input: unknown kind \"{other}\" (key, text, pointer, wheel, compose, commit, resize, window_focus)"
            ));
        }
    })
}

fn gui_input(args: Vec<Value>) -> Res<Value> {
    arity("gui.input", &args, 2, 2)?;
    let id = window_id("gui.input", args.first())?;
    let w = window("gui.input", id)?;
    let input = input_of(&args[1])?;
    let mut out = Vec::new();
    let headless = {
        let mut st = w.lock().map_err(|_| "gui: window poisoned")?;
        let headless = st.headless;
        let mut clip = HeadlessClipboard;
        st.input(input, &mut clip, &mut out);
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
    arity("gui.fonts", &args, 1, 1)?;
    let items: Vec<Value> = match &args[0] {
        Value::List(l) => l.as_ref().clone(),
        other => vec![other.clone()],
    };
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

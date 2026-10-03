//! The platform side: the event loop on the process's first thread,
//! platform windows and their renderers, keys and pointer from winit,
//! the accessibility adapters, the clipboard, dialogs, and the macOS
//! menu bar.
//!
//! ## The first thread
//!
//! `main` runs the program on a thread of its own and parks the first
//! thread in [`host`]. When the program first opens a window, [`proxy`]
//! asks the first thread to start winit's loop, and every later
//! request (open, close, redraw, set, a dialog, a menu) reaches the loop
//! as a [`Cmd`] through its proxy. When the program ends, the loop is
//! told to exit, and `host` answers the program's exit code. A program
//! that never opens a window never starts a loop: `host` answers when
//! the program ends.
//!
//! While the loop runs, the first thread holds a live guard in `chan`'s
//! census, so a program waiting on `gui.events()` is not mistaken for a
//! deadlock: input can arrive.

use super::values::*;
use super::window::{Clipboard, Input, Mods, PointerAction, WinState};
use super::{OpenOpts, Shared, a11y, emit, gpu, soft, windows};
use crate::ast::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// What the program's threads ask of the loop.
pub enum Cmd {
    Open {
        id: u64,
        opts: OpenOpts,
        reply: crossbeam_channel::Sender<Result<(), String>>,
    },
    Close(u64),
    Redraw(u64),
    Set(u64, Value),
    Dialog {
        kind: String,
        opts: Value,
        reply: crossbeam_channel::Sender<Result<Value, String>>,
    },
    Menu {
        spec: Value,
        reply: crossbeam_channel::Sender<Result<Value, String>>,
    },
    Exit(i32),
    A11y(accesskit_winit::Event),
    MenuEvent(String),
}

impl From<accesskit_winit::Event> for Cmd {
    fn from(e: accesskit_winit::Event) -> Self {
        Cmd::A11y(e)
    }
}

enum MainMsg {
    Start(crossbeam_channel::Sender<Result<(), String>>),
    Done(i32),
}

enum LoopState {
    Idle,
    Starting,
    Running(EventLoopProxy<Cmd>),
    Failed(String),
}

static LOOP: Mutex<LoopState> = Mutex::new(LoopState::Idle);
static PENDING_EXIT: Mutex<Option<i32>> = Mutex::new(None);
static MAIN: OnceLock<crossbeam_channel::Sender<MainMsg>> = OnceLock::new();

/// Run on the process's first thread: start the program (on its own
/// thread, through `spawn`), serve the first thread to `gui` until the
/// program ends, and answer its exit code.
pub fn host(spawn: impl FnOnce() -> std::thread::JoinHandle<i32>) -> i32 {
    let (tx, rx) = crossbeam_channel::unbounded();
    let _ = MAIN.set(tx);
    let program = spawn();
    std::thread::Builder::new()
        .name("gui-watch".to_string())
        .spawn(move || {
            let code = program.join().unwrap_or(1);
            program_done(code);
        })
        .expect("spawning the program watcher");
    loop {
        match rx.recv() {
            Ok(MainMsg::Start(reply)) => {
                if let Some(code) = run_loop(reply) {
                    return code;
                }
            }
            Ok(MainMsg::Done(code)) => return code,
            Err(_) => return 1,
        }
    }
}

fn program_done(code: i32) {
    let state = LOOP.lock().unwrap_or_else(|e| e.into_inner());
    match &*state {
        LoopState::Running(p) => {
            let _ = p.send_event(Cmd::Exit(code));
        }
        LoopState::Starting => {
            *PENDING_EXIT.lock().unwrap_or_else(|e| e.into_inner()) = Some(code);
        }
        _ => {
            if let Some(m) = MAIN.get() {
                let _ = m.send(MainMsg::Done(code));
            }
        }
    }
}

/// The loop's proxy, starting the loop on first use.
fn proxy() -> Result<EventLoopProxy<Cmd>, String> {
    loop {
        {
            let mut state = LOOP.lock().unwrap_or_else(|e| e.into_inner());
            match &*state {
                LoopState::Running(p) => return Ok(p.clone()),
                LoopState::Failed(m) => return Err(m.clone()),
                LoopState::Starting => {}
                LoopState::Idle => {
                    let main = MAIN.get().ok_or(
                        "windows need olang's own main thread (run the program with the olang CLI)",
                    )?;
                    *state = LoopState::Starting;
                    let (rtx, rrx) = crossbeam_channel::bounded(1);
                    main.send(MainMsg::Start(rtx))
                        .map_err(|_| "the main thread is gone".to_string())?;
                    drop(state);
                    match rrx.recv() {
                        Ok(Ok(())) => continue,
                        Ok(Err(e)) => return Err(e),
                        Err(_) => return Err("the main thread is gone".to_string()),
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Whether a window can be opened here: a display, and olang's main
/// thread. Answers without starting the loop.
pub fn available() -> bool {
    if MAIN.get().is_none() {
        return false;
    }
    if matches!(
        &*LOOP.lock().unwrap_or_else(|e| e.into_inner()),
        LoopState::Failed(_)
    ) {
        return false;
    }
    if cfg!(target_os = "linux") {
        return std::env::var_os("WAYLAND_DISPLAY").is_some();
    }
    true
}

/// Start winit on this (the first) thread and run it until the program
/// ends. `None` when the loop could not start (the program goes on,
/// windowless).
fn run_loop(reply: crossbeam_channel::Sender<Result<(), String>>) -> Option<i32> {
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        let msg = "no Wayland display (Loom runs on Wayland only; X11 is not supported — try --face terminal)".to_string();
        *LOOP.lock().unwrap_or_else(|e| e.into_inner()) = LoopState::Failed(msg.clone());
        let _ = reply.send(Err(msg));
        return None;
    }
    let event_loop = match EventLoop::<Cmd>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            let msg = format!("no display: {e}");
            *LOOP.lock().unwrap_or_else(|e| e.into_inner()) = LoopState::Failed(msg.clone());
            let _ = reply.send(Err(msg));
            return None;
        }
    };
    let proxy = event_loop.create_proxy();
    {
        let mut state = LOOP.lock().unwrap_or_else(|e| e.into_inner());
        *state = LoopState::Running(proxy.clone());
        if let Some(code) = PENDING_EXIT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = proxy.send_event(Cmd::Exit(code));
        }
    }
    let _ = reply.send(Ok(()));
    let mut app = App {
        windows: HashMap::new(),
        by_winit: HashMap::new(),
        proxy,
        exit: None,
        _live: crate::stdlib::chan::live_guard(),
        clipboard: None,
        #[cfg(target_os = "macos")]
        menu: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("gui: the event loop ended: {e}");
    }
    Some(app.exit.unwrap_or(0))
}

enum Renderer {
    Gpu(gpu::Surface),
    Soft {
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        _context: softbuffer::Context<Arc<Window>>,
    },
}

struct PWin {
    win: Arc<Window>,
    state: Shared,
    renderer: Renderer,
    a11y: accesskit_winit::Adapter,
    ime_on: bool,
    ime_area: Option<[f32; 4]>,
}

struct App {
    windows: HashMap<u64, PWin>,
    by_winit: HashMap<WindowId, u64>,
    proxy: EventLoopProxy<Cmd>,
    exit: Option<i32>,
    _live: crate::stdlib::chan::LiveGuard,
    clipboard: Option<arboard::Clipboard>,
    #[cfg(target_os = "macos")]
    menu: Option<(muda::Menu, HashMap<muda::MenuId, String>)>,
}

/// The platform clipboard, as the window's editing sees it.
struct PlatformClipboard<'a>(&'a mut Option<arboard::Clipboard>);

impl Clipboard for PlatformClipboard<'_> {
    fn get(&mut self) -> Option<String> {
        if self.0.is_none() {
            *self.0 = arboard::Clipboard::new().ok();
        }
        self.0.as_mut()?.get_text().ok()
    }
    fn set(&mut self, text: String) {
        if self.0.is_none() {
            *self.0 = arboard::Clipboard::new().ok();
        }
        if let Some(c) = self.0.as_mut() {
            let _ = c.set_text(text);
        }
    }
}

impl App {
    fn open(&mut self, el: &ActiveEventLoop, id: u64, opts: OpenOpts) -> Result<(), String> {
        let mut attrs = Window::default_attributes()
            .with_title(opts.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(opts.width, opts.height))
            .with_resizable(opts.resizable)
            // Shown once the accessibility adapter is attached, as
            // AccessKit requires.
            .with_visible(false);
        if let Some((w, h)) = opts.min {
            attrs = attrs.with_min_inner_size(winit::dpi::LogicalSize::new(w, h));
        }
        let win = Arc::new(
            el.create_window(attrs)
                .map_err(|e| format!("creating the window: {e}"))?,
        );
        let a11y = accesskit_winit::Adapter::with_event_loop_proxy(el, &win, self.proxy.clone());
        let scale = win.scale_factor() as f32;
        let size = win.inner_size();
        let renderer = if opts.renderer == "software" {
            soft_renderer(&win)?
        } else {
            match gpu::Surface::new(win.clone(), size.width, size.height) {
                Ok(s) => Renderer::Gpu(s),
                Err(e) => {
                    eprintln!("gui: the GPU renderer is unavailable ({e}); drawing in software");
                    soft_renderer(&win)?
                }
            }
        };
        let logical = size.to_logical::<f32>(scale as f64);
        let mut st = WinState::new(id, false, &opts.title, logical.width, logical.height, scale);
        if let Some(c) = opts.clear {
            st.clear = c;
        }
        let state = Arc::new(Mutex::new(st));
        windows()
            .lock()
            .map_err(|_| "gui: registry poisoned")?
            .insert(id, state.clone());
        self.by_winit.insert(win.id(), id);
        win.set_visible(true);
        win.request_redraw();
        self.windows.insert(
            id,
            PWin {
                win,
                state,
                renderer,
                a11y,
                ime_on: false,
                ime_area: None,
            },
        );
        // The program lays out against the window's real size and scale.
        emit(vec![event(
            "resize",
            vec![
                ("window", Value::Integer(id as i64)),
                ("width", float(logical.width)),
                ("height", float(logical.height)),
                ("scale", float(scale)),
            ],
        )]);
        Ok(())
    }

    fn input(&mut self, id: u64, input: Input) {
        let Some(pw) = self.windows.get_mut(&id) else {
            return;
        };
        let mut out = Vec::new();
        let dirty = {
            let Ok(mut st) = pw.state.lock() else {
                return;
            };
            let mut clip = PlatformClipboard(&mut self.clipboard);
            st.input(input, &mut clip, &mut out);
            st.dirty
        };
        emit(out);
        if dirty {
            pw.win.request_redraw();
        }
    }

    fn render(&mut self, id: u64) {
        let Some(pw) = self.windows.get_mut(&id) else {
            return;
        };
        let trace = std::env::var_os("GUI_TRACE").is_some();
        let (dl, a11y_tree, wants_ime, ime_area) = {
            let Ok(mut st) = pw.state.lock() else {
                return;
            };
            let dl = st.display_list();
            st.dirty = false;
            let tree = if st.a11y_dirty {
                st.a11y_dirty = false;
                Some(a11y::tree(&st))
            } else {
                None
            };
            let wants = st.wants_ime();
            let area = if wants { st.ime_area() } else { None };
            (dl, tree, wants, area)
        };
        if trace {
            eprintln!(
                "gui: window {id} frame {}x{}, {} prims",
                dl.width,
                dl.height,
                dl.prims.len()
            );
        }
        match &mut pw.renderer {
            Renderer::Gpu(s) => match s.render(&dl) {
                Ok(()) => {}
                // Not drawn: the window is covered or the compositor is
                // busy. Keep the frame owed: the scene stays dirty, and the
                // window draws when it shows again (`Occluded(false)`) or,
                // after a timeout, on the next request.
                Err(e) if e == "occluded" || e == "timeout" => {
                    if trace {
                        eprintln!("gui: window {id} frame skipped ({e})");
                    }
                    if let Ok(mut st) = pw.state.lock() {
                        st.dirty = true;
                    }
                    if e == "timeout" {
                        pw.win.request_redraw();
                    }
                }
                Err(e) => eprintln!("gui: {e}"),
            },
            Renderer::Soft { surface, .. } => present_soft(surface, &dl, &pw.win),
        }
        if let Some(tree) = a11y_tree {
            pw.a11y.update_if_active(|| tree);
        }
        if wants_ime != pw.ime_on {
            pw.win.set_ime_allowed(wants_ime);
            pw.ime_on = wants_ime;
        }
        if wants_ime
            && ime_area != pw.ime_area
            && let Some(a) = ime_area
        {
            pw.win.set_ime_cursor_area(
                winit::dpi::LogicalPosition::new(a[0], a[1]),
                winit::dpi::LogicalSize::new(a[2].max(1.0), a[3].max(1.0)),
            );
            pw.ime_area = ime_area;
        }
    }
}

fn soft_renderer(win: &Arc<Window>) -> Result<Renderer, String> {
    let context = softbuffer::Context::new(win.clone())
        .map_err(|e| format!("the software presenter: {e}"))?;
    let surface = softbuffer::Surface::new(&context, win.clone())
        .map_err(|e| format!("the software presenter: {e}"))?;
    Ok(Renderer::Soft {
        surface,
        _context: context,
    })
}

fn present_soft(
    surface: &mut softbuffer::Surface<Arc<Window>, Arc<Window>>,
    dl: &super::window::DisplayList,
    win: &Window,
) {
    use std::num::NonZeroU32;
    let pm = soft::render(dl);
    let (Some(w), Some(h)) = (NonZeroU32::new(pm.width()), NonZeroU32::new(pm.height())) else {
        return;
    };
    if surface.resize(w, h).is_err() {
        return;
    }
    let Ok(mut buf) = surface.buffer_mut() else {
        return;
    };
    for (dst, px) in buf.iter_mut().zip(pm.pixels()) {
        // Opaque windows: the premultiplied colour is the colour.
        *dst = ((px.red() as u32) << 16) | ((px.green() as u32) << 8) | px.blue() as u32;
    }
    win.pre_present_notify();
    let _ = buf.present();
}

/// Heddle's names for keys (`left`, `enter`, `f5`); a character key is
/// its character, lowercased when it is a letter.
fn key_name(key: &Key) -> Option<String> {
    Some(match key {
        Key::Named(n) => match n {
            NamedKey::Enter => "enter".into(),
            NamedKey::Tab => "tab".into(),
            NamedKey::Space => "space".into(),
            NamedKey::Backspace => "backspace".into(),
            NamedKey::Delete => "delete".into(),
            NamedKey::Escape => "escape".into(),
            NamedKey::ArrowLeft => "left".into(),
            NamedKey::ArrowRight => "right".into(),
            NamedKey::ArrowUp => "up".into(),
            NamedKey::ArrowDown => "down".into(),
            NamedKey::Home => "home".into(),
            NamedKey::End => "end".into(),
            NamedKey::PageUp => "pageup".into(),
            NamedKey::PageDown => "pagedown".into(),
            NamedKey::Insert => "insert".into(),
            NamedKey::F1 => "f1".into(),
            NamedKey::F2 => "f2".into(),
            NamedKey::F3 => "f3".into(),
            NamedKey::F4 => "f4".into(),
            NamedKey::F5 => "f5".into(),
            NamedKey::F6 => "f6".into(),
            NamedKey::F7 => "f7".into(),
            NamedKey::F8 => "f8".into(),
            NamedKey::F9 => "f9".into(),
            NamedKey::F10 => "f10".into(),
            NamedKey::F11 => "f11".into(),
            NamedKey::F12 => "f12".into(),
            NamedKey::Shift
            | NamedKey::Control
            | NamedKey::Alt
            | NamedKey::Super
            | NamedKey::Meta
            | NamedKey::CapsLock => return None,
            other => format!("{other:?}").to_lowercase(),
        },
        Key::Character(c) => {
            if c.chars().all(|ch| ch.is_alphabetic()) {
                c.to_lowercase()
            } else {
                c.to_string()
            }
        }
        _ => return None,
    })
}

fn key_input(event: &KeyEvent, mods: Mods) -> Option<Input> {
    if event.state != ElementState::Pressed {
        return None;
    }
    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
    let base = event.key_without_modifiers();
    let key = key_name(&base).or_else(|| key_name(&event.logical_key))?;
    Some(Input::Key {
        key,
        text: event.text.as_ref().map(|t| t.to_string()),
        mods,
        repeat: event.repeat,
    })
}

impl ApplicationHandler<Cmd> for App {
    fn resumed(&mut self, _el: &ActiveEventLoop) {}

    fn user_event(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
        if std::env::var_os("GUI_TRACE").is_some() {
            let what = match &cmd {
                Cmd::Open { id, .. } => format!("open {id}"),
                Cmd::Close(id) => format!("close {id}"),
                Cmd::Redraw(id) => format!("redraw {id}"),
                Cmd::Set(id, _) => format!("set {id}"),
                Cmd::Dialog { kind, .. } => format!("dialog {kind}"),
                Cmd::Menu { .. } => "menu".to_string(),
                Cmd::Exit(c) => format!("exit {c}"),
                Cmd::A11y(_) => "a11y".to_string(),
                Cmd::MenuEvent(id) => format!("menu event {id}"),
            };
            eprintln!("gui: {what}");
        }
        match cmd {
            Cmd::Open { id, opts, reply } => {
                let r = self.open(el, id, opts);
                let _ = reply.send(r);
            }
            Cmd::Close(id) => {
                if let Some(pw) = self.windows.remove(&id) {
                    self.by_winit.remove(&pw.win.id());
                }
            }
            Cmd::Redraw(id) => {
                if let Some(pw) = self.windows.get(&id) {
                    pw.win.request_redraw();
                }
            }
            Cmd::Set(id, v) => {
                if let Some(pw) = self.windows.get(&id) {
                    if let Ok(Some(t)) = get_str(&v, "title", "gui.set") {
                        pw.win.set_title(t);
                    }
                    if let Ok(Some((w, h))) = get_pair(&v, "size", "gui.set") {
                        let _ = pw
                            .win
                            .request_inner_size(winit::dpi::LogicalSize::new(w, h));
                    }
                    if let Ok(Some((w, h))) = get_pair(&v, "min", "gui.set") {
                        pw.win
                            .set_min_inner_size(Some(winit::dpi::LogicalSize::new(w, h)));
                    }
                    if let Ok(Some(b)) = get_bool(&v, "visible", "gui.set") {
                        pw.win.set_visible(b);
                    }
                    if let Ok(Some(c)) = get_str(&v, "cursor", "gui.set") {
                        use winit::window::CursorIcon;
                        pw.win.set_cursor(match c {
                            "pointer" => CursorIcon::Pointer,
                            "text" => CursorIcon::Text,
                            "wait" => CursorIcon::Wait,
                            "move" => CursorIcon::Move,
                            "ew-resize" => CursorIcon::EwResize,
                            "ns-resize" => CursorIcon::NsResize,
                            "not-allowed" => CursorIcon::NotAllowed,
                            _ => CursorIcon::Default,
                        });
                    }
                    if let Ok(Some(true)) = get_bool(&v, "focus", "gui.set") {
                        pw.win.focus_window();
                    }
                    pw.win.request_redraw();
                }
            }
            Cmd::Dialog { kind, opts, reply } => {
                let _ = reply.send(run_dialog(&kind, &opts));
            }
            Cmd::Menu { spec, reply } => {
                let _ = reply.send(self.set_menu(&spec));
            }
            Cmd::MenuEvent(id) => emit(vec![event("menu", vec![("id", s(&id))])]),
            Cmd::Exit(code) => {
                self.exit = Some(code);
                self.windows.clear();
                el.exit();
            }
            Cmd::A11y(ev) => {
                let Some(id) = self.by_winit.get(&ev.window_id).copied() else {
                    return;
                };
                match ev.window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => {
                        if let Some(pw) = self.windows.get_mut(&id)
                            && let Ok(st) = pw.state.lock()
                        {
                            let tree = a11y::tree(&st);
                            drop(st);
                            pw.a11y.update_if_active(|| tree);
                        }
                    }
                    accesskit_winit::WindowEvent::ActionRequested(req) => {
                        if let Some(pw) = self.windows.get(&id) {
                            let mut out = Vec::new();
                            if let Ok(mut st) = pw.state.lock() {
                                a11y::action(&mut st, &req, &mut out);
                            }
                            emit(out);
                            pw.win.request_redraw();
                        }
                    }
                    accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
                }
            }
        }
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, wid: WindowId, we: WindowEvent) {
        let Some(id) = self.by_winit.get(&wid).copied() else {
            return;
        };
        if let Some(pw) = self.windows.get_mut(&id) {
            pw.a11y.process_event(&pw.win, &we);
        }
        let mods = self
            .windows
            .get(&id)
            .and_then(|pw| pw.state.lock().ok().map(|s| s.mods))
            .unwrap_or_default();
        let scale = self
            .windows
            .get(&id)
            .map(|pw| pw.win.scale_factor())
            .unwrap_or(1.0);
        match we {
            WindowEvent::CloseRequested => emit(vec![event(
                "close_requested",
                vec![("window", Value::Integer(id as i64))],
            )]),
            WindowEvent::Resized(size) => {
                let l = size.to_logical::<f32>(scale);
                self.input(
                    id,
                    Input::Resize {
                        width: l.width,
                        height: l.height,
                        scale: scale as f32,
                    },
                );
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(pw) = self.windows.get(&id) {
                    let l = pw.win.inner_size().to_logical::<f32>(scale);
                    self.input(
                        id,
                        Input::Resize {
                            width: l.width,
                            height: l.height,
                            scale: scale as f32,
                        },
                    );
                }
            }
            WindowEvent::RedrawRequested => self.render(id),
            // Shown again after being covered: draw what was skipped.
            WindowEvent::Occluded(false) => {
                if let Some(pw) = self.windows.get(&id) {
                    pw.win.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                let st = m.state();
                self.input(
                    id,
                    Input::Modifiers(Mods {
                        ctrl: st.control_key(),
                        alt: st.alt_key(),
                        shift: st.shift_key(),
                        super_: st.super_key(),
                    }),
                );
            }
            WindowEvent::KeyboardInput { event: ke, .. } => {
                if let Some(i) = key_input(&ke, mods) {
                    self.input(id, i);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let l = position.to_logical::<f32>(scale);
                self.input(
                    id,
                    Input::Pointer {
                        action: PointerAction::Move,
                        x: l.x,
                        y: l.y,
                        button: String::new(),
                        clicks: 0,
                    },
                );
            }
            WindowEvent::CursorLeft { .. } => self.input(
                id,
                Input::Pointer {
                    action: PointerAction::Leave,
                    x: -1.0,
                    y: -1.0,
                    button: String::new(),
                    clicks: 0,
                },
            ),
            WindowEvent::MouseInput { state, button, .. } => {
                let (x, y) = self
                    .windows
                    .get(&id)
                    .and_then(|pw| pw.state.lock().ok().map(|s| s.pointer))
                    .unwrap_or((0.0, 0.0));
                let button = match button {
                    MouseButton::Left => "left",
                    MouseButton::Right => "right",
                    MouseButton::Middle => "middle",
                    MouseButton::Back => "back",
                    MouseButton::Forward => "forward",
                    MouseButton::Other(_) => "other",
                };
                self.input(
                    id,
                    Input::Pointer {
                        action: if state == ElementState::Pressed {
                            PointerAction::Down
                        } else {
                            PointerAction::Up
                        },
                        x,
                        y,
                        button: button.to_string(),
                        clicks: 0,
                    },
                );
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    // A line is three lines of 16 px text, as browsers do.
                    MouseScrollDelta::LineDelta(x, y) => (x * 48.0, y * 48.0),
                    MouseScrollDelta::PixelDelta(p) => {
                        let l = p.to_logical::<f32>(scale);
                        (l.x, l.y)
                    }
                };
                self.input(id, Input::Wheel { dx, dy });
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(t, cursor) => self.input(id, Input::ImePreedit(t, cursor)),
                Ime::Commit(t) => self.input(id, Input::ImeCommit(t)),
                Ime::Enabled | Ime::Disabled => {}
            },
            WindowEvent::Focused(on) => self.input(id, Input::Focused(on)),
            WindowEvent::ThemeChanged(theme) => emit(vec![event(
                "appearance",
                vec![
                    ("window", Value::Integer(id as i64)),
                    ("dark", Value::Boolean(theme == winit::window::Theme::Dark)),
                ],
            )]),
            WindowEvent::DroppedFile(path) => emit(vec![event(
                "files_dropped",
                vec![
                    ("window", Value::Integer(id as i64)),
                    (
                        "paths",
                        Value::List(Arc::new(vec![s(&path.to_string_lossy())])),
                    ),
                ],
            )]),
            _ => {}
        }
    }
}

// ── requests from the program's threads ─────────────────────────────

pub fn open(opts: OpenOpts) -> Result<u64, String> {
    let p = proxy().map_err(|e| format!("gui.open: {e}"))?;
    let id = super::next_id();
    let (tx, rx) = crossbeam_channel::bounded(1);
    p.send_event(Cmd::Open {
        id,
        opts,
        reply: tx,
    })
    .map_err(|_| "gui.open: the event loop has ended".to_string())?;
    rx.recv()
        .map_err(|_| "gui.open: the event loop has ended".to_string())?
        .map_err(|e| format!("gui.open: {e}"))?;
    Ok(id)
}

fn send(cmd: Cmd) {
    if let LoopState::Running(p) = &*LOOP.lock().unwrap_or_else(|e| e.into_inner()) {
        let _ = p.send_event(cmd);
    }
}

pub fn close(id: u64) {
    send(Cmd::Close(id));
}

pub fn redraw(id: u64) {
    send(Cmd::Redraw(id));
}

pub fn set(id: u64, v: Value) {
    send(Cmd::Set(id, v));
}

pub fn clipboard_get() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

pub fn clipboard_set(t: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(t))
        .map_err(|e| format!("gui.clipboard_write: {e}"))
}

pub fn dialog(kind: &str, opts: &Value) -> Result<Value, String> {
    if !["open", "open_many", "save", "folder", "message"].contains(&kind) {
        return Err(format!(
            "gui.dialog: unknown kind \"{kind}\" (open, open_many, save, folder, message)"
        ));
    }
    let p = proxy().map_err(|e| format!("gui.dialog: {e}"))?;
    let (tx, rx) = crossbeam_channel::bounded(1);
    p.send_event(Cmd::Dialog {
        kind: kind.to_string(),
        opts: opts.clone(),
        reply: tx,
    })
    .map_err(|_| "gui.dialog: the event loop has ended".to_string())?;
    rx.recv()
        .map_err(|_| "gui.dialog: the event loop has ended".to_string())?
}

/// A platform dialog, on the loop's thread (macOS requires it).
fn run_dialog(kind: &str, opts: &Value) -> Result<Value, String> {
    let what = "gui.dialog";
    let title = get_str(opts, "title", what)
        .ok()
        .flatten()
        .map(str::to_string);
    let path = |p: std::path::PathBuf| s(&p.to_string_lossy());
    if kind == "message" {
        let text = get_str(opts, "text", what)?.unwrap_or("").to_string();
        let buttons = match get_str(opts, "buttons", what)?.unwrap_or("ok") {
            "ok_cancel" => rfd::MessageButtons::OkCancel,
            "yes_no" => rfd::MessageButtons::YesNo,
            "yes_no_cancel" => rfd::MessageButtons::YesNoCancel,
            _ => rfd::MessageButtons::Ok,
        };
        let level = match get_str(opts, "level", what)?.unwrap_or("info") {
            "warning" => rfd::MessageLevel::Warning,
            "error" => rfd::MessageLevel::Error,
            _ => rfd::MessageLevel::Info,
        };
        let mut d = rfd::MessageDialog::new()
            .set_description(text)
            .set_buttons(buttons)
            .set_level(level);
        if let Some(t) = &title {
            d = d.set_title(t);
        }
        let r = d.show();
        return Ok(s(&match r {
            rfd::MessageDialogResult::Ok => "ok".to_string(),
            rfd::MessageDialogResult::Cancel => "cancel".to_string(),
            rfd::MessageDialogResult::Yes => "yes".to_string(),
            rfd::MessageDialogResult::No => "no".to_string(),
            rfd::MessageDialogResult::Custom(c) => c,
        }));
    }
    let mut d = rfd::FileDialog::new();
    if let Some(t) = &title {
        d = d.set_title(t);
    }
    if let Some(dir) = get_str(opts, "directory", what)? {
        d = d.set_directory(dir);
    }
    if let Some(name) = get_str(opts, "name", what)? {
        d = d.set_file_name(name);
    }
    if let Some(Value::List(filters)) = get(opts, "filters") {
        for f in filters.iter() {
            let name = get_str(f, "name", what)?.unwrap_or("Files").to_string();
            let exts: Vec<String> = match get(f, "extensions") {
                Some(Value::List(l)) => l
                    .iter()
                    .filter_map(|e| match e {
                        Value::String(t) => Some(t.to_string()),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            };
            d = d.add_filter(name, &exts);
        }
    }
    Ok(match kind {
        "open" => d.pick_file().map(path).unwrap_or(Value::Unit),
        "open_many" => match d.pick_files() {
            Some(v) => Value::List(Arc::new(v.into_iter().map(path).collect())),
            None => Value::Unit,
        },
        "save" => d.save_file().map(path).unwrap_or(Value::Unit),
        _ => d.pick_folder().map(path).unwrap_or(Value::Unit),
    })
}

pub fn menu(spec: &Value) -> Result<Value, String> {
    if !cfg!(target_os = "macos") {
        // Elsewhere the menu is drawn in the window, by Loom.
        return Ok(Value::Boolean(false));
    }
    let p = proxy().map_err(|e| format!("gui.menu: {e}"))?;
    let (tx, rx) = crossbeam_channel::bounded(1);
    p.send_event(Cmd::Menu {
        spec: spec.clone(),
        reply: tx,
    })
    .map_err(|_| "gui.menu: the event loop has ended".to_string())?;
    rx.recv()
        .map_err(|_| "gui.menu: the event loop has ended".to_string())?
}

impl App {
    #[cfg(not(target_os = "macos"))]
    fn set_menu(&mut self, _spec: &Value) -> Result<Value, String> {
        Ok(Value::Boolean(false))
    }

    /// The macOS menu bar from a spec: a list of menus, each `#{ title,
    /// items }`, an item `#{ id, label, keys?, enabled?, checked? }` or
    /// `"separator"`. The first menu is the application menu.
    #[cfg(target_os = "macos")]
    fn set_menu(&mut self, spec: &Value) -> Result<Value, String> {
        use muda::accelerator::Accelerator;
        use muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
        let what = "gui.menu";
        let menus = match spec {
            Value::List(l) => l.clone(),
            _ => return Err(format!("{what}: the spec is a list of menus")),
        };
        let bar = Menu::new();
        let mut ids = HashMap::new();
        for m in menus.iter() {
            let title = get_str(m, "title", what)?.unwrap_or("").to_string();
            let sub = Submenu::new(&title, true);
            let items = match get(m, "items") {
                Some(Value::List(l)) => l.clone(),
                _ => Arc::new(vec![]),
            };
            for it in items.iter() {
                if matches!(it, Value::String(t) if t.as_str() == "separator") {
                    sub.append(&PredefinedMenuItem::separator())
                        .map_err(|e| format!("{what}: {e}"))?;
                    continue;
                }
                let id = get_str(it, "id", what)?
                    .ok_or_else(|| format!("{what}: a menu item needs an \"id\""))?
                    .to_string();
                let label = get_str(it, "label", what)?.unwrap_or(&id).to_string();
                let enabled = get_bool(it, "enabled", what)?.unwrap_or(true);
                let accel: Option<Accelerator> = match get_str(it, "keys", what)? {
                    Some(k) => Some(
                        accel_of(k)
                            .parse()
                            .map_err(|e| format!("{what}: keys \"{k}\": {e}"))?,
                    ),
                    None => None,
                };
                let mid = muda::MenuId::new(&id);
                match get_bool(it, "checked", what)? {
                    Some(c) => {
                        let item = CheckMenuItem::with_id(mid.clone(), &label, enabled, c, accel);
                        sub.append(&item).map_err(|e| format!("{what}: {e}"))?;
                    }
                    None => {
                        let item = MenuItem::with_id(mid.clone(), &label, enabled, accel);
                        sub.append(&item).map_err(|e| format!("{what}: {e}"))?;
                    }
                }
                ids.insert(mid, id);
            }
            bar.append(&sub).map_err(|e| format!("{what}: {e}"))?;
        }
        bar.init_for_nsapp();
        let proxy = self.proxy.clone();
        muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
            let _ = proxy.send_event(Cmd::MenuEvent(e.id.0.clone()));
        }));
        self.menu = Some((bar, ids));
        Ok(Value::Boolean(true))
    }
}

/// Loom's chord spelling as muda's: `mod+shift+p` → `CmdOrCtrl+Shift+P`.
#[cfg(target_os = "macos")]
fn accel_of(keys: &str) -> String {
    keys.split('+')
        .map(|p| match p {
            "mod" => "CmdOrCtrl".to_string(),
            "cmd" | "super" => "Super".to_string(),
            "ctrl" => "Control".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "enter" => "Enter".to_string(),
            "space" => "Space".to_string(),
            "tab" => "Tab".to_string(),
            "escape" => "Escape".to_string(),
            "backspace" => "Backspace".to_string(),
            "delete" => "Delete".to_string(),
            "left" => "ArrowLeft".to_string(),
            "right" => "ArrowRight".to_string(),
            "up" => "ArrowUp".to_string(),
            "down" => "ArrowDown".to_string(),
            k if k.len() == 1 => k.to_uppercase(),
            k => {
                let mut c = k.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

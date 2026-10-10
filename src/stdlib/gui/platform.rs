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
    /// The menu bar, read and checked on the program's thread; made here
    /// without the program waiting.
    Menu(MenuSpec),
    Exit(i32),
    A11y(accesskit_winit::Event),
    MenuEvent(String),
    /// The system's settings may have changed (an AppKit notice, or a
    /// test posting one): read them again; `why` goes on the event.
    Settings(&'static str),
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
                crate::boot_trace::mark("the platform's event loop starts");
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

/// Whether the loop, once started, makes a hidden window and lets it go
/// (`prepare` asks; see `App::resumed`).
static WARM_WINDOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Start the platform's loop now, without waiting for it: a program
/// about to open its first window (Loom's `run`) asks before it computes
/// that window's first frame, so the application's start on the main
/// thread (AppKit's, tens of milliseconds) runs meanwhile instead of
/// after. Nothing when the loop is already started, has failed, or there
/// is no olang main thread; the first window waits for the loop as
/// before.
pub fn prepare() {
    let Some(main) = MAIN.get() else {
        return;
    };
    WARM_WINDOW.store(true, std::sync::atomic::Ordering::Release);
    let mut state = LOOP.lock().unwrap_or_else(|e| e.into_inner());
    if !matches!(&*state, LoopState::Idle) {
        return;
    }
    // the reply is not waited for: `proxy` sees the loop start
    let (rtx, _rrx) = crossbeam_channel::bounded(1);
    if main.send(MainMsg::Start(rtx)).is_ok() {
        *state = LoopState::Starting;
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

/// What this platform's windows can do, for a program to check rather
/// than find out: `#{ os, native_menu, clipboard_image, file_drop,
/// drop_position, window_position, system_settings, ime, accessibility }`.
/// Each is what this build does on this OS (not whether a display is
/// there: that is `available`):
///
/// - `native_menu`: `gui.menu` sets the system's menu bar (macOS);
///   elsewhere it answers `false` and Loom draws the window's menu.
/// - `clipboard_image`: `gui.clipboard_image` reads a picture (macOS).
/// - `file_drop`: files dragged in from another program arrive as
///   `files` events (macOS, Windows; winit has no Wayland drag and drop).
/// - `drop_position`: a `files` event's `x`/`y` is where the pointer is
///   as they arrive (macOS); elsewhere the pointer's last place in the
///   window.
/// - `window_position`: a window learns where it is on the screen, so
///   pointer events carry `sx`/`sy` and a drag follows into another
///   window (macOS, Windows); on Wayland a drag stays in its window.
/// - `system_settings`: `gui.context` reads the dark appearance (Auto
///   included), increased contrast, reduced motion, reduced transparency,
///   and the accent colour from the system, and a window hears
///   `appearance` as soon as one changes — no restart (macOS).
/// - `ime`: input methods (composition, a candidate window placed at the
///   caret) — everywhere winit has them (Wayland's text-input-v3).
/// - `accessibility`: the platform's accessibility API through AccessKit
///   (NSAccessibility, UI Automation, AT-SPI).
pub fn capabilities() -> Value {
    let mac = cfg!(target_os = "macos");
    let windows = cfg!(target_os = "windows");
    let b = Value::Boolean;
    map(vec![
        ("os", s(std::env::consts::OS)),
        ("native_menu", b(mac)),
        ("clipboard_image", b(mac)),
        ("file_drop", b(mac || windows)),
        ("drop_position", b(mac)),
        ("window_position", b(mac || windows)),
        ("system_settings", b(mac)),
        ("ime", b(true)),
        ("accessibility", b(true)),
    ])
}

/// Start winit on this (the first) thread and run it until the program
/// ends. `None` when the loop could not start (the program goes on,
/// windowless).
fn run_loop(reply: crossbeam_channel::Sender<Result<(), String>>) -> Option<i32> {
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        let msg = "no Wayland display (Loom runs on Wayland only; X11 is not supported — try --terminal, or LOOM_FACE=terminal)".to_string();
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
    // The system's settings, read here (the main thread) and kept; AppKit
    // says when they change.
    super::context::set_live(super::context::read());
    #[cfg(target_os = "macos")]
    observe_settings(&proxy);
    // before the application finishes launching: the documents it was
    // opened with arrive as it does
    #[cfg(target_os = "macos")]
    observe_documents();
    let mut app = App {
        windows: HashMap::new(),
        by_winit: HashMap::new(),
        proxy,
        exit: None,
        _live: crate::stdlib::chan::live_guard(),
        clipboard: None,
        files: Vec::new(),
        closing: Vec::new(),
        nudged: None,
        #[cfg(target_os = "macos")]
        menu: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("gui: the event loop ended: {e}");
    }
    Some(app.exit.unwrap_or(0))
}

/// Ask AppKit to say when the accessibility display options (contrast,
/// motion, transparency) or the system's colours (the accent) change:
/// each notice reaches the loop as [`Cmd::Settings`]. The appearance's
/// change arrives as a window's `ThemeChanged`. Observers live as long as
/// the process.
#[cfg(target_os = "macos")]
fn observe_settings(proxy: &EventLoopProxy<Cmd>) {
    use objc2_app_kit::{
        NSSystemColorsDidChangeNotification, NSWorkspace,
        NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use std::ptr::NonNull;
    let on = |center: &NSNotificationCenter, name: &objc2_foundation::NSString| {
        let p = proxy.clone();
        let block = block2::RcBlock::new(move |_n: NonNull<NSNotification>| {
            let _ = p.send_event(Cmd::Settings("settings"));
        });
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        std::mem::forget(token);
    };
    let ws = NSWorkspace::sharedWorkspace();
    on(&ws.notificationCenter(), unsafe {
        NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification
    });
    on(&NSNotificationCenter::defaultCenter(), unsafe {
        NSSystemColorsDidChangeNotification
    });
}

/// Post the notice AppKit sends when an accessibility display option
/// changes, in this process, from the main thread — what a test does to
/// check the loop hears it without changing the person's settings.
#[cfg(target_os = "macos")]
pub fn post_settings_notice() {
    use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
    let ws = NSWorkspace::sharedWorkspace();
    unsafe {
        ws.notificationCenter().postNotificationName_object(
            NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
            None,
        )
    };
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
    /// The appearance the program chose for this window (`gui.set`'s
    /// `appearance`), or `None` to follow the system's.
    forced_theme: Option<winit::window::Theme>,
    /// Covered, minimized, or on another space: its animations wait.
    occluded: bool,
    /// When an animation in view next changes: the loop draws again then.
    wake: Option<std::time::Instant>,
    state: Shared,
    renderer: Renderer,
    a11y: accesskit_winit::Adapter,
    ime_on: bool,
    ime_area: Option<[f32; 4]>,
}

/// A window the program closed (or every window, as the loop ends), torn
/// down in an order the platform can follow: its surface and the window
/// first, in an autorelease pool of their own, so the window closes while
/// what it shows, its accessibility included, is as it was. The window's
/// accessibility adapter is returned, to be dropped once the platform has
/// let go of the window ([`Closing`]).
fn close_window(pw: PWin) -> Closing {
    let PWin {
        win,
        renderer,
        a11y,
        state,
        ..
    } = pw;
    let window = Closing::watch(&win);
    let teardown = move || {
        drop(renderer);
        drop(state);
        // the last reference: winit closes the window
        drop(win);
    };
    #[cfg(target_os = "macos")]
    objc2::rc::autoreleasepool(|_| teardown());
    #[cfg(not(target_os = "macos"))]
    teardown();
    Closing {
        _a11y: a11y,
        window,
        at: std::time::Instant::now(),
    }
}

/// A closed window's accessibility adapter, kept until the window itself
/// is gone. On macOS AccessKit makes the window's content view an
/// instance of a subclass of its own; dropping the adapter gives the view
/// its class back and releases it. AppKit lets go of a closed window only
/// as it next handles an event, so the view's class is put back once the
/// window and its frame are torn down, never under them.
struct Closing {
    _a11y: accesskit_winit::Adapter,
    #[cfg(target_os = "macos")]
    window: Option<objc2::rc::Weak<objc2::runtime::AnyObject>>,
    #[cfg(not(target_os = "macos"))]
    window: (),
    /// When the window was closed: the loop nudges AppKit for a while.
    at: std::time::Instant,
}

/// How long the loop keeps nudging AppKit to let go of a closed window
/// (it takes a turn or two); after that the adapter waits quietly.
const CLOSING_NUDGE: std::time::Duration = std::time::Duration::from_secs(2);

impl Closing {
    /// A weak reference to the platform's window behind `win`.
    #[cfg(target_os = "macos")]
    fn watch(win: &Window) -> Option<objc2::rc::Weak<objc2::runtime::AnyObject>> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let RawWindowHandle::AppKit(h) = win.window_handle().ok()?.as_raw() else {
            return None;
        };
        let view = h.ns_view.as_ptr() as *mut objc2::runtime::AnyObject;
        // SAFETY: the handle's view is a live NSView while `win` is;
        // `-[NSView window]` returns it at +0 (retained here).
        let window: Option<objc2::rc::Retained<objc2::runtime::AnyObject>> =
            unsafe { objc2::msg_send![&*view, window] };
        window.map(|w| objc2::rc::Weak::from_retained(&w))
    }

    #[cfg(not(target_os = "macos"))]
    fn watch(_win: &Window) {}

    /// Whether the platform has let go of the window.
    fn gone(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.window.as_ref().is_none_or(|w| w.load().is_none())
        }
        #[cfg(not(target_os = "macos"))]
        {
            true
        }
    }
}

/// Post an application-defined event (one nobody handles) so AppKit
/// handles an event and lets go of the windows that were closed; with
/// the loop otherwise idle it would hold them until the person next
/// moves the pointer or presses a key.
#[cfg(target_os = "macos")]
fn nudge_appkit() {
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{MainThreadMarker, NSPoint};
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let ev = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags(0),
        0.0,
        0,
        None,
        0,
        0,
        0,
    );
    if let Some(ev) = ev {
        NSApplication::sharedApplication(mtm).postEvent_atStart(&ev, false);
    }
}

struct App {
    windows: HashMap<u64, PWin>,
    by_winit: HashMap<WindowId, u64>,
    proxy: EventLoopProxy<Cmd>,
    exit: Option<i32>,
    _live: crate::stdlib::chan::LiveGuard,
    clipboard: Option<arboard::Clipboard>,
    /// Files hovered over or dropped on a window, one platform event a
    /// file: gathered and sent as one `files` event when the loop next
    /// waits (`(window, action, path)`).
    files: Vec<(u64, &'static str, String)>,
    /// Closed windows' accessibility adapters, until the windows are gone.
    closing: Vec<Closing>,
    /// When AppKit was last nudged to let go of closed windows.
    nudged: Option<std::time::Instant>,
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
        #[cfg(target_os = "macos")]
        if let Some(t) = &opts.tabbing {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs = attrs.with_tabbing_identifier(t);
        }
        let win = Arc::new(
            el.create_window(attrs)
                .map_err(|e| format!("creating the window: {e}"))?,
        );
        crate::boot_trace::mark("a window made");
        let a11y = accesskit_winit::Adapter::with_event_loop_proxy(el, &win, self.proxy.clone());
        let scale = win.scale_factor() as f32;
        let size = win.inner_size();
        let renderer = if opts.renderer == "software" {
            soft_renderer(&win)?
        } else {
            match gpu::Surface::new(
                win.clone(),
                el.owned_display_handle(),
                size.width,
                size.height,
            ) {
                Ok(s) => Renderer::Gpu(s),
                Err(e) => {
                    eprintln!("gui: the GPU renderer is unavailable ({e}); drawing in software");
                    soft_renderer(&win)?
                }
            }
        };
        crate::boot_trace::mark("a window's renderer made");
        let logical = size.to_logical::<f32>(scale as f64);
        let mut st = WinState::new(id, false, &opts.title, logical.width, logical.height, scale);
        if let Some(c) = opts.clear {
            st.clear = c;
        }
        st.settings = super::context::system();
        let state = Arc::new(Mutex::new(st));
        windows()
            .lock()
            .map_err(|_| "gui: registry poisoned")?
            .insert(id, state.clone());
        self.by_winit.insert(win.id(), id);
        // joined to another window's tab bar before it is shown: it opens
        // as that window's tab, never as a window of its own first
        #[cfg(target_os = "macos")]
        if let Some(host) = opts.tab_of.and_then(|o| self.windows.get(&o))
            && let (Some(a), Some(b)) = (ns_window(&host.win), ns_window(&win))
        {
            // SAFETY: two live NSWindows; NSWindowAbove is 1.
            let _: () = unsafe { objc2::msg_send![&*a, addTabbedWindow: &*b, ordered: 1isize] };
        }
        // a window of its own asked for: shown alone (the system's automatic
        // tabbing would put it in a bar of its kind), then free to join one
        // (Merge All Windows) as any window is
        #[cfg(target_os = "macos")]
        let alone = if opts.tabbing.is_some() && opts.tab_of.is_none() { ns_window(&win) } else { None };
        #[cfg(target_os = "macos")]
        if let Some(w) = &alone {
            // SAFETY: a live NSWindow; NSWindowTabbingModeDisallowed is 2.
            let _: () = unsafe { objc2::msg_send![&**w, setTabbingMode: 2isize] };
        }
        win.set_visible(true);
        #[cfg(target_os = "macos")]
        if let Some(w) = &alone {
            // SAFETY: as above; NSWindowTabbingModeAutomatic is 0.
            let _: () = unsafe { objc2::msg_send![&**w, setTabbingMode: 0isize] };
        }
        win.request_redraw();
        self.windows.insert(
            id,
            PWin {
                win,
                forced_theme: None,
                occluded: false,
                wake: None,
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
        // and in the appearance the system asks for
        let dark = self
            .windows
            .get(&id)
            .and_then(|pw| pw.win.theme())
            .map(|t| t == winit::window::Theme::Dark);
        emit(vec![super::context::event_for(id, dark, "open")]);
        self.sync_origin(id);
        // the tab bar it opened in (another window's, or its own), as it
        // stands now: a window that is not key says nothing else of it
        #[cfg(target_os = "macos")]
        self.report_tabs(id);
        Ok(())
    }

    /// The window's content origin on the screen, read again (opened,
    /// moved, resized): the window's state keeps it, and the program hears
    /// `moved` when it changed.
    fn sync_origin(&mut self, id: u64) {
        let Some(pw) = self.windows.get(&id) else {
            return;
        };
        let Ok(p) = pw.win.inner_position() else {
            return;
        };
        let l = p.to_logical::<f32>(pw.win.scale_factor());
        let mut out = Vec::new();
        if let Ok(mut st) = pw.state.lock() {
            st.place(l.x, l.y, &mut out);
        }
        emit(out);
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
            // a window a script drives takes no person's input (its size,
            // focus and the system's settings still come)
            if st.scripted
                && matches!(
                    input,
                    Input::Key { .. }
                        | Input::Pointer { .. }
                        | Input::Wheel { .. }
                        | Input::Pinch { .. }
                        | Input::ImePreedit(..)
                        | Input::ImeCommit(_)
                        | Input::Modifiers(_)
                        | Input::Files { .. }
                )
            {
                return;
            }
            let mut clip = PlatformClipboard(&mut self.clipboard);
            st.input(input, &mut clip, &mut out);
            st.dirty
        };
        emit(out);
        if dirty {
            pw.win.request_redraw();
        }
    }

    /// Read the system's settings again (`dark` as a window that follows
    /// the system said): every window takes them and its program hears
    /// `appearance`.
    fn settings_changed(&mut self, why: &'static str, dark: Option<bool>) {
        let mut now = super::context::read();
        if let Some(d) = dark {
            now.dark = Some(d);
        }
        // a notice is rare and answered always (a test posts one); a
        // window coming forward only tells of a change
        if !super::context::set_live(now) && why == "focus" {
            return;
        }
        let mut out = Vec::new();
        for (id, pw) in &self.windows {
            if let Ok(mut st) = pw.state.lock() {
                st.set_settings(now);
            }
            out.push(super::context::event_of(*id, now, None, why));
            pw.win.request_redraw();
        }
        emit(out);
    }

    fn render(&mut self, id: u64) {
        let Some(pw) = self.windows.get_mut(&id) else {
            return;
        };
        let trace = std::env::var_os("GUI_TRACE").is_some();
        let started = std::time::Instant::now();
        let (dl, a11y_tree, wants_ime, ime_area) = {
            let Ok(mut st) = pw.state.lock() else {
                return;
            };
            let dl = st.display_list();
            // an animation in view: the next frame when it is due (none
            // while the window cannot be seen)
            let unseen = pw.occluded
                || pw.win.is_visible() == Some(false)
                || pw.win.is_minimized() == Some(true);
            pw.wake = match st.next_frame {
                Some(t) if !unseen => {
                    let ms = (t - st.clock_ms()).clamp(1.0, 60_000.0);
                    Some(started + std::time::Duration::from_micros((ms * 1000.0) as u64))
                }
                _ => None,
            };
            st.dirty = false;
            // the tree is built only when a client is listening (a long
            // styled text is a run a paragraph)
            let tree = st.a11y_dirty;
            st.a11y_dirty = false;
            let wants = st.wants_ime();
            let area = if wants { st.ime_area() } else { None };
            (dl, tree, wants, area)
        };
        let listed = started.elapsed();
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
        crate::boot_trace::frame_presented();
        // the frame: its display list made, and drawn and presented
        if trace {
            eprintln!(
                "gui: window {id} frame {}x{}, {} prims, listed in {:.2} ms, drawn in {:.2} ms",
                dl.width,
                dl.height,
                dl.prims.len(),
                listed.as_secs_f64() * 1000.0,
                (started.elapsed() - listed).as_secs_f64() * 1000.0
            );
        }
        if a11y_tree {
            let state = pw.state.clone();
            pw.a11y.update_if_active(|| match state.lock() {
                Ok(st) => a11y::tree(&st),
                Err(e) => a11y::tree(&e.into_inner()),
            });
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

impl App {
    /// The files gathered since the loop last waited, as one `files`
    /// event a window and action, at the pointer's place.
    fn flush_files(&mut self) {
        let mut groups: Vec<(u64, &'static str, Vec<String>)> = Vec::new();
        for (id, action, path) in std::mem::take(&mut self.files) {
            match groups.last_mut() {
                Some(g) if g.0 == id && g.1 == action => g.2.push(path),
                _ => groups.push((id, action, vec![path])),
            }
        }
        for (id, action, paths) in groups {
            let at = self.windows.get(&id).and_then(|pw| pointer_now(&pw.win));
            self.input(
                id,
                Input::Files {
                    action: action.into(),
                    paths,
                    at,
                },
            );
        }
    }
}

/// Where the pointer is now in `win`'s content, in logical pixels, when
/// the platform can say: a file dragged in from another program moves
/// no pointer the window hears (macOS sends nothing between the drag
/// entering and the drop), so its place is asked for.
#[cfg(target_os = "macos")]
fn pointer_now(win: &Window) -> Option<(f32, f32)> {
    // points, from the bottom left of the primary display
    let p = objc2_app_kit::NSEvent::mouseLocation();
    let primary = win.primary_monitor().or_else(|| win.current_monitor())?;
    let ph = primary.size().height as f64 / primary.scale_factor();
    let sf = win.scale_factor();
    let inner = win.inner_position().ok()?;
    let (ix, iy) = (inner.x as f64 / sf, inner.y as f64 / sf);
    Some(((p.x - ix) as f32, ((ph - p.y) - iy) as f32))
}

/// Elsewhere the pointer's last place in the window is used.
#[cfg(not(target_os = "macos"))]
fn pointer_now(_win: &Window) -> Option<(f32, f32)> {
    None
}

/// The NSWindow behind `win`, retained.
#[cfg(target_os = "macos")]
fn ns_window(win: &Window) -> Option<objc2::rc::Retained<objc2::runtime::AnyObject>> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(h) = win.window_handle().ok()?.as_raw() else {
        return None;
    };
    let view = h.ns_view.as_ptr() as *mut objc2::runtime::AnyObject;
    // SAFETY: the handle's view is a live NSView while `win` is.
    unsafe { objc2::msg_send![&*view, window] }
}

/// An NSString's text.
#[cfg(target_os = "macos")]
unsafe fn ns_text(s: *mut objc2::runtime::AnyObject) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: the caller's NSString; UTF8String lives as long as it.
    let p: *const std::ffi::c_char = unsafe { objc2::msg_send![&*s, UTF8String] };
    if p.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// `application:openURLs:` — the files and folders the application is
/// asked to open (a document opened with it in the Finder, dropped on
/// its Dock icon, `open -a`): the program hears `open` with their paths.
#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn app_open_urls(
    _this: *mut objc2::runtime::AnyObject,
    _cmd: objc2::runtime::Sel,
    _app: *mut objc2::runtime::AnyObject,
    urls: *mut objc2::runtime::AnyObject,
) {
    if urls.is_null() {
        return;
    }
    let mut paths = Vec::new();
    // SAFETY: AppKit's NSArray of NSURLs, alive for the call.
    unsafe {
        let n: usize = objc2::msg_send![&*urls, count];
        for i in 0..n {
            let url: *mut objc2::runtime::AnyObject = objc2::msg_send![&*urls, objectAtIndex: i];
            if url.is_null() {
                continue;
            }
            let file: bool = objc2::msg_send![&*url, isFileURL];
            if !file {
                continue;
            }
            let path: *mut objc2::runtime::AnyObject = objc2::msg_send![&*url, path];
            if let Some(p) = ns_text(path) {
                paths.push(s(&p));
            }
        }
    }
    if !paths.is_empty() {
        emit(vec![event("open", vec![("paths", Value::List(Arc::new(paths)))])]);
    }
}

/// `newWindowForTab:` — the tab bar's + button: the program hears
/// `new_tab` (and the button shows because someone answers it).
#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn app_new_tab(
    _this: *mut objc2::runtime::AnyObject,
    _cmd: objc2::runtime::Sel,
    _sender: *mut objc2::runtime::AnyObject,
) {
    emit(vec![event("new_tab", vec![])]);
}

/// Teach the application's delegate (winit's) to hear the documents it
/// is asked to open and the tab bar's + button.
#[cfg(target_os = "macos")]
fn observe_documents() {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else {
        return;
    };
    let obj = objc2::rc::Retained::as_ptr(&delegate) as *const AnyObject;
    // SAFETY: the delegate's class, a live class; methods added once (a
    // second add of the same selector is refused by the runtime).
    unsafe {
        let cls = objc2::ffi::object_getClass(obj) as *mut AnyClass;
        if cls.is_null() {
            return;
        }
        let open: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) = app_open_urls;
        let tab: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = app_new_tab;
        objc2::ffi::class_addMethod(
            cls,
            objc2::sel!(application:openURLs:),
            std::mem::transmute::<_, Imp>(open),
            c"v@:@@".as_ptr(),
        );
        objc2::ffi::class_addMethod(
            cls,
            objc2::sel!(newWindowForTab:),
            std::mem::transmute::<_, Imp>(tab),
            c"v@:@".as_ptr(),
        );
    }
}

impl App {
    /// Say which windows window `id` is shown among as tabs (itself
    /// alone when it has no tab bar), in the bar's order: `tabs`.
    #[cfg(target_os = "macos")]
    fn report_tabs(&self, id: u64) {
        let Some(pw) = self.windows.get(&id) else {
            return;
        };
        let Some(w) = ns_window(&pw.win) else {
            return;
        };
        let mine: Vec<(usize, u64)> = self
            .windows
            .iter()
            .filter_map(|(i, p)| ns_window(&p.win).map(|nw| (objc2::rc::Retained::as_ptr(&nw) as usize, *i)))
            .collect();
        let mut ids = Vec::new();
        // SAFETY: a live NSWindow; tabbedWindows is an NSArray or nil.
        unsafe {
            let tabs: *mut objc2::runtime::AnyObject = objc2::msg_send![&*w, tabbedWindows];
            if tabs.is_null() {
                ids.push(id);
            } else {
                let n: usize = objc2::msg_send![&*tabs, count];
                for k in 0..n {
                    let o: *mut objc2::runtime::AnyObject = objc2::msg_send![&*tabs, objectAtIndex: k];
                    if let Some((_, i)) = mine.iter().find(|(p, _)| *p == o as usize) {
                        ids.push(*i);
                    }
                }
            }
        }
        emit(vec![event(
            "tabs",
            vec![
                ("window", Value::Integer(id as i64)),
                ("tabs", Value::List(Arc::new(ids.into_iter().map(|i| Value::Integer(i as i64)).collect()))),
            ],
        )]);
    }
}

impl ApplicationHandler<Cmd> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        // Started ahead of a window (`prepare`): AppKit's first window
        // pays a one-time start of its own (its text input, its window
        // server connection; ~25 ms, a second window ~5), so a hidden
        // one is made and let go now, while the program boots — the
        // program's own window then opens at a second window's cost.
        if WARM_WINDOW.swap(false, std::sync::atomic::Ordering::AcqRel) {
            let attrs = Window::default_attributes()
                .with_inner_size(winit::dpi::LogicalSize::new(1.0, 1.0))
                .with_visible(false);
            if let Ok(w) = el.create_window(attrs) {
                drop(w);
            }
            crate::boot_trace::mark("the platform warmed (a hidden window made and let go)");
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if !self.files.is_empty() {
            self.flush_files();
        }
        if !self.closing.is_empty() {
            self.closing.retain(|c| !c.gone());
        }
        // Animations: a window whose next frame is due draws; the loop
        // sleeps until the soonest of the rest.
        let now = std::time::Instant::now();
        let mut soonest: Option<std::time::Instant> = None;
        // A closed window AppKit has not let go of yet: it does so as it
        // handles an event, so it is given one now and then for a while.
        if self.closing.iter().any(|c| now - c.at < CLOSING_NUDGE) {
            let every = std::time::Duration::from_millis(50);
            if self.nudged.is_none_or(|t| now - t >= every) {
                self.nudged = Some(now);
                #[cfg(target_os = "macos")]
                nudge_appkit();
            }
            soonest = Some(now + every);
        }
        for pw in self.windows.values_mut() {
            match pw.wake {
                Some(t) if t <= now => {
                    pw.wake = None;
                    pw.win.request_redraw();
                }
                Some(t) => soonest = Some(soonest.map_or(t, |s| s.min(t))),
                None => {}
            }
        }
        el.set_control_flow(match soonest {
            Some(t) => winit::event_loop::ControlFlow::WaitUntil(t),
            None => winit::event_loop::ControlFlow::Wait,
        });
    }

    fn user_event(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
        if std::env::var_os("GUI_TRACE").is_some() {
            let what = match &cmd {
                Cmd::Open { id, .. } => format!("open {id}"),
                Cmd::Close(id) => format!("close {id}"),
                Cmd::Redraw(id) => format!("redraw {id}"),
                Cmd::Set(id, _) => format!("set {id}"),
                Cmd::Dialog { kind, .. } => format!("dialog {kind}"),
                Cmd::Menu(_) => "menu".to_string(),
                Cmd::Exit(c) => format!("exit {c}"),
                Cmd::A11y(_) => "a11y".to_string(),
                Cmd::MenuEvent(id) => format!("menu event {id}"),
                Cmd::Settings(why) => format!("settings ({why})"),
            };
            eprintln!("gui: {what}");
        }
        match cmd {
            Cmd::Open { id, opts, reply } => {
                crate::boot_trace::mark("the platform makes the window");
                let r = self.open(el, id, opts);
                let _ = reply.send(r);
            }
            Cmd::Close(id) => {
                if let Some(pw) = self.windows.remove(&id) {
                    self.by_winit.remove(&pw.win.id());
                    let c = close_window(pw);
                    self.closing.push(c);
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
                    // the window's tab bar (macOS): the next or previous
                    // tab chosen, every window merged into this one's
                    // tabs, this tab moved to a window of its own, the
                    // bar shown or hidden
                    #[cfg(target_os = "macos")]
                    if let Ok(Some(op)) = get_str(&v, "tab", "gui.set")
                        && let Some(w) = ns_window(&pw.win)
                    {
                        let none: *const objc2::runtime::AnyObject = std::ptr::null();
                        // SAFETY: a live NSWindow and its standard actions.
                        unsafe {
                            match op {
                                "next" => { let _: () = objc2::msg_send![&*w, selectNextTab: none]; }
                                "previous" => { let _: () = objc2::msg_send![&*w, selectPreviousTab: none]; }
                                "merge" => { let _: () = objc2::msg_send![&*w, mergeAllWindows: none]; }
                                "detach" => { let _: () = objc2::msg_send![&*w, moveTabToNewWindow: none]; }
                                "bar" => { let _: () = objc2::msg_send![&*w, toggleTabBar: none]; }
                                _ => {}
                            }
                        }
                    }
                    // a document's unsaved edits: the dot in the close
                    // button, as every Mac document window shows them
                    #[cfg(target_os = "macos")]
                    if let Ok(Some(e)) = get_bool(&v, "edited", "gui.set") {
                        use winit::platform::macos::WindowExtMacOS;
                        pw.win.set_document_edited(e);
                    }
                    pw.win.request_redraw();
                }
                #[cfg(target_os = "macos")]
                if let Ok(Some(_)) = get_str(&v, "tab", "gui.set") {
                    self.report_tabs(id);
                }
                if let Ok(Some(a)) = get_str(&v, "appearance", "gui.set")
                    && let Some(pw) = self.windows.get_mut(&id)
                {
                    // the title bar and the platform's own controls in the
                    // appearance the program draws in
                    let t = match a {
                        "dark" => Some(winit::window::Theme::Dark),
                        "light" => Some(winit::window::Theme::Light),
                        _ => None,
                    };
                    if pw.forced_theme != t {
                        pw.forced_theme = t;
                        pw.win.set_theme(t);
                    }
                }
            }
            Cmd::Dialog { kind, opts, reply } => {
                let _ = reply.send(run_dialog(&kind, &opts));
            }
            Cmd::Menu(spec) => {
                if let Err(e) = self.set_menu(spec) {
                    eprintln!("{e}");
                }
            }
            Cmd::MenuEvent(id) => emit(vec![event("menu", vec![("id", s(&id))])]),
            Cmd::Settings(why) => self.settings_changed(why, None),
            Cmd::Exit(code) => {
                self.exit = Some(code);
                self.by_winit.clear();
                for (_, pw) in self.windows.drain() {
                    let c = close_window(pw);
                    self.closing.push(c);
                }
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
            WindowEvent::Moved(_) => self.sync_origin(id),
            WindowEvent::Resized(size) => {
                self.sync_origin(id);
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
            // Shown again after being covered: draw what was skipped (and
            // the animations go on).
            WindowEvent::Occluded(false) => {
                if let Some(pw) = self.windows.get_mut(&id) {
                    pw.occluded = false;
                    pw.win.request_redraw();
                }
            }
            // Covered, minimized, or on another space: nothing to animate.
            WindowEvent::Occluded(true) => {
                if let Some(pw) = self.windows.get_mut(&id) {
                    pw.occluded = true;
                    pw.wake = None;
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
                self.input(id, Input::Wheel { dx, dy, mods: None });
            }
            // A trackpad's pinch (macOS; the other platforms send none).
            WindowEvent::PinchGesture { delta, phase, .. } => {
                if delta.is_finite() {
                    let phase = match phase {
                        winit::event::TouchPhase::Started => "start",
                        winit::event::TouchPhase::Moved => "move",
                        winit::event::TouchPhase::Ended => "end",
                        winit::event::TouchPhase::Cancelled => "cancel",
                    };
                    self.input(
                        id,
                        Input::Pinch {
                            delta: delta as f32,
                            phase: phase.to_string(),
                        },
                    );
                }
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(t, cursor) => self.input(id, Input::ImePreedit(t, cursor)),
                Ime::Commit(t) => self.input(id, Input::ImeCommit(t)),
                Ime::Enabled | Ime::Disabled => {}
            },
            WindowEvent::Focused(on) => {
                self.input(id, Input::Focused(on));
                // the tabs it is shown among, as they stand now
                #[cfg(target_os = "macos")]
                if on {
                    self.report_tabs(id);
                }
                // A notice missed (the settings changed while the process
                // was suspended) is caught when a window comes forward.
                if on {
                    self.settings_changed("focus", None);
                }
            }
            // The system's appearance changed (Auto at dusk, or by hand):
            // a window that follows it says so first.
            WindowEvent::ThemeChanged(theme) => {
                let follows = self.windows.get(&id).is_some_and(|pw| pw.forced_theme.is_none());
                self.settings_changed(
                    "theme",
                    follows.then_some(theme == winit::window::Theme::Dark),
                );
            }
            WindowEvent::HoveredFile(path) => {
                self.files
                    .push((id, "hover", path.to_string_lossy().to_string()));
            }
            WindowEvent::DroppedFile(path) => {
                self.files
                    .push((id, "drop", path.to_string_lossy().to_string()));
            }
            WindowEvent::HoveredFileCancelled => {
                self.flush_files();
                self.input(
                    id,
                    Input::Files {
                        action: "cancel".into(),
                        paths: vec![],
                        at: None,
                    },
                );
            }
            _ => {}
        }
    }
}

// ── requests from the program's threads ─────────────────────────────

pub fn open(opts: OpenOpts) -> Result<u64, String> {
    crate::boot_trace::mark("a window asked for (gui.open)");
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

/// The clipboard's image as PNG bytes: a PNG put there as one (a
/// screenshot), or a TIFF (what most macOS programs copy) turned into a
/// PNG. macOS only so far; elsewhere an `Err` that says so.
pub fn clipboard_image() -> Result<Vec<u8>, String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{
            NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG,
            NSPasteboardTypeTIFF,
        };
        let pb = NSPasteboard::generalPasteboard();
        // SAFETY: AppKit's constant pasteboard types, read only.
        let (png_t, tiff_t) = unsafe { (NSPasteboardTypePNG, NSPasteboardTypeTIFF) };
        if let Some(d) = pb.dataForType(png_t) {
            return Ok(d.to_vec());
        }
        if let Some(d) = pb.dataForType(tiff_t) {
            let rep = NSBitmapImageRep::imageRepWithData(&d)
                .ok_or("the clipboard's image could not be read")?;
            let props = objc2_foundation::NSDictionary::new();
            // SAFETY: an empty properties dictionary of the declared type.
            let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }
                .ok_or("the clipboard's image could not be turned into a PNG")?;
            return Ok(png.to_vec());
        }
        Err("the clipboard holds no image".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("an image is read from the clipboard on macOS only so far".into())
    }
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
    // Read and checked here, every error the program's; made on the main
    // thread without waiting for it — a program's first frame is still
    // being presented there when its menu is set, and a menu of a large
    // program's commands takes ~10 ms to make (olang Studio's start
    // waited for both).
    let spec = menu_spec(spec)?;
    let p = proxy().map_err(|e| format!("gui.menu: {e}"))?;
    p.send_event(Cmd::Menu(spec))
        .map_err(|_| "gui.menu: the event loop has ended".to_string())?;
    Ok(Value::Boolean(true))
}

/// A menu bar as `gui.menu` reads it: menus (a title, items), an item a
/// separator or `(id, label, enabled, keys, checked)` with the keys in
/// the platform's spelling, already checked to parse.
pub struct MenuSpec(Vec<(String, Vec<MenuItemSpec>)>);

enum MenuItemSpec {
    Separator,
    Item {
        id: String,
        label: String,
        enabled: bool,
        keys: Option<String>,
        checked: Option<bool>,
    },
}

fn menu_spec(spec: &Value) -> Result<MenuSpec, String> {
    let what = "gui.menu";
    let menus = match spec {
        Value::List(l) => l.clone(),
        _ => return Err(format!("{what}: the spec is a list of menus")),
    };
    let mut out = Vec::with_capacity(menus.len());
    for m in menus.iter() {
        let title = get_str(m, "title", what)?.unwrap_or("").to_string();
        let items = match get(m, "items") {
            Some(Value::List(l)) => l.clone(),
            _ => Arc::new(vec![]),
        };
        let mut its = Vec::with_capacity(items.len());
        for it in items.iter() {
            if matches!(it, Value::String(t) if t.as_str() == "separator") {
                its.push(MenuItemSpec::Separator);
                continue;
            }
            let id = get_str(it, "id", what)?
                .ok_or_else(|| format!("{what}: a menu item needs an \"id\""))?
                .to_string();
            let label = get_str(it, "label", what)?.unwrap_or(&id).to_string();
            let enabled = get_bool(it, "enabled", what)?.unwrap_or(true);
            let keys = match get_str(it, "keys", what)? {
                Some(k) => {
                    let spelled = accel_spelling(k);
                    #[cfg(target_os = "macos")]
                    spelled
                        .parse::<muda::accelerator::Accelerator>()
                        .map_err(|e| format!("{what}: keys \"{k}\": {e}"))?;
                    Some(spelled)
                }
                None => None,
            };
            let checked = get_bool(it, "checked", what)?;
            its.push(MenuItemSpec::Item {
                id,
                label,
                enabled,
                keys,
                checked,
            });
        }
        out.push((title, its));
    }
    Ok(MenuSpec(out))
}

impl App {
    #[cfg(not(target_os = "macos"))]
    fn set_menu(&mut self, _spec: MenuSpec) -> Result<(), String> {
        Ok(())
    }

    /// The macOS menu bar from a spec: a list of menus, each `#{ title,
    /// items }`, an item `#{ id, label, keys?, enabled?, checked? }` or
    /// `"separator"`. The first menu is the application menu.
    #[cfg(target_os = "macos")]
    fn set_menu(&mut self, spec: MenuSpec) -> Result<(), String> {
        use muda::accelerator::Accelerator;
        use muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
        let what = "gui.menu";
        let bar = Menu::new();
        let mut ids = HashMap::new();
        for (title, items) in spec.0 {
            let sub = Submenu::new(&title, true);
            for it in items {
                let (id, label, enabled, keys, checked) = match it {
                    MenuItemSpec::Separator => {
                        sub.append(&PredefinedMenuItem::separator())
                            .map_err(|e| format!("{what}: {e}"))?;
                        continue;
                    }
                    MenuItemSpec::Item {
                        id,
                        label,
                        enabled,
                        keys,
                        checked,
                    } => (id, label, enabled, keys, checked),
                };
                // checked to parse when the spec was read
                let accel: Option<Accelerator> = keys.and_then(|k| k.parse().ok());
                let mid = muda::MenuId::new(&id);
                match checked {
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
        Ok(())
    }
}

/// Loom's chord spelling as muda's: `mod+shift+p` → `CmdOrCtrl+Shift+P`.
fn accel_spelling(keys: &str) -> String {
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

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
    /// A context menu over window `id` at `(x, y)` (its content's
    /// logical pixels), read and checked on the program's thread.
    ContextMenu {
        id: u64,
        items: Vec<ContextItem>,
        x: f64,
        y: f64,
    },
    /// The system's settings may have changed (an AppKit notice, or a
    /// test posting one): read them again; `why` goes on the event.
    Settings(&'static str),
    /// A standard Edit menu item chosen for window `id` (its role:
    /// `copy`, `paste`, …), or a Service's text to put in its field.
    Edit {
        id: u64,
        role: &'static str,
    },
    /// Window `id`'s display refreshed at `at` (ns of the host clock).
    Tick {
        id: u64,
        at: u64,
    },
    /// The program's turn has run past the watch's limit (`true`), or it
    /// ended after it had (`false`): every window says so.
    NotResponding(bool),
    ServiceText {
        id: u64,
        text: String,
    },
    /// Text the platform put in outside a key press and a composition
    /// (dictation, the Character Viewer, a Service): into window `id`'s
    /// focused field, over `replace` (UTF-16 location and length in its
    /// value) when given.
    InsertText {
        id: u64,
        text: String,
        replace: Option<(usize, usize)>,
    },
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
    // An external quit the program was finishing: the platform is told
    // the application may now end (it ends the process with this code).
    if quit_hold::finish(code) {
        return;
    }
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
/// - `window_tabs`: `gui.open` takes `tabbing` and `tab_of`, `gui.set`
///   takes `tab`, and windows say their `tabs` (macOS window tabs).
/// - `open_documents`: the application hears `open` with the files the
///   system asks it to open, and `new_tab` from a tab bar's + (macOS).
/// - `terminal`: a terminal can be built: the `pty` and `vt` modules, a
///   canvas's `ime` (the input method's text as `text` and `preedit`
///   events while it has the focus) and a canvas text's `italic`.
/// - `group_hover`: a style's `hover` look shown over an ancestor too;
///   `scroll_bars`: scrollers draw overlay bars (bars.rs).
/// - `flat_keyed`: `gui.flat_emit` takes `keep` `"keyed"` (a frame whose
///   shape changed: what last frame placed is kept by engine key here);
///   `flat_diff`: `gui.flat_diff` (a frame's placements against last
///   frame's operations: Loom's patch, but its children's order);
///   `flat_index`: `gui.flat_index` (what a frame's placements say, by key).
/// - `transitions`: a style's `transition` (motion.rs), and `gui.set`'s
///   `motion` (a headless window's transitions run on its clock).
///
/// - `edit_menu`: `gui.menu` takes the Mac's standard Edit items
///   (`standard`) and submenus (`submenu`); the view answers Services
///   (macOS).
/// - `turn_watch`: `gui.turn(true|false)` around a program's turn; one
///   that runs past 2 s is shown in its windows while it runs.
///
/// - `a11y_depth`: a node takes `pos` ((index, count): its place in its
///   set, for a virtualized list's rows) and `live: "off"`; the roles
///   take the landmarks (`navigation`, `main`, `banner`, `complementary`,
///   `contentinfo`, `form`) and `grid`, `rowheader`, `rowgroup`,
///   `spinbutton`, `meter`, `toolbar`, `radiogroup`; tables say their
///   rows and columns; assistive output may step a `slider` or a
///   `spinbutton` (`a11y` events `increment`/`decrement`), open and close
///   what has `expanded` (`expand`/`collapse`), and scroll a page.
///
/// - `flash`: `gui.set(w, #{ "flash": true })` tints every region a frame
///   draws, each frame its own colour (the repaint rainbow of Loom's
///   inspector); `false` draws the window whole again.
///
/// A program (Loom) asks before it uses what an older engine refuses.
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
        ("window_tabs", b(mac)),
        ("open_documents", b(mac)),
        ("context_menu", b(mac)),
        ("terminal", b(cfg!(unix))),
        // a style's `group_hover`, and the overlay scroll bars (a code
        // field's `track` and `past_end`)
        ("group_hover", b(true)),
        ("scroll_bars", b(true)),
        // a node's `wheel_x`: it hears the wheel's sideways part only
        ("wheel_x", b(true)),
        // `gui.turn`: a program's turn that runs long is shown in its
        // windows while it runs (the busy cursor, a spinner)
        ("turn_watch", b(true)),
        // `gui.menu` takes the standard Edit items (`standard`, with the
        // `contexts` the program's commands bind their chords in) and
        // submenus (`submenu`); the window's view answers Services
        ("edit_menu", b(mac)),
        // a field's `spellcheck`: misspelled words underlined by the
        // platform's checker (macOS's NSSpellChecker)
        ("spellcheck", b(mac)),
        // the text input client: dictation, Look Up, an input method's
        // context read from the focused field
        ("text_input", b(mac)),
        // `gui.flat_emit`'s keyed keep, and `gui.flat_diff` (flat.rs)
        ("flat_keyed", b(true)),
        ("flat_diff", b(true)),
        ("flat_index", b(true)),
        // a style's `transition` (motion.rs): opacity, box and colours
        // animated, nodes entering and leaving, run at the refresh
        ("transitions", b(true)),
        // a style's `corner` ("continuous": Apple's continuous corners)
        // and `shadow` (an analytic Gaussian shadow), `gui.set`'s
        // `corner` (the window's default); both renderers
        ("corner", b(true)),
        ("shadow", b(true)),
        // a canvas's paths drawn as vectors (curves: `d`; `cap`, `join`,
        // `dash`, `rule`) and radial gradients (`dir: "radial"`)
        ("vector", b(true)),
        // a style's `material` (sidebar, popover, sheet, hud): what is
        // under the box blurred on the GPU, the opaque `bg` in software
        // and under Reduce Transparency
        ("material", b(true)),
        // the accessibility tree's places, roles and actions (a11y.rs)
        ("a11y_depth", b(true)),
        // `gui.set`'s `flash`: the repaint rainbow (damage.rs)
        ("flash", b(true)),
        // `gui.hold_quit`: an external quit (the Dock's Quit, AppleScript's
        // `quit`, a logout, a restart, a shutdown) is the program's `quit`
        // event, and the application ends once the program has
        ("quit_hold", b(mac)),
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
    // a measurement's stand-in refresh, when asked for
    #[cfg(target_os = "macos")]
    let _ = frames::synthetic_hz();
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
        #[cfg(target_os = "macos")]
        context: None,
        ctrl_click: false,
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
            contain_callback("settings_notice", || {
                let _ = p.send_event(Cmd::Settings("settings"));
            });
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

// One a window, held in its PWin and never moved on the hot path.
#[allow(clippy::large_enum_variant)]
enum Renderer {
    Gpu(gpu::Surface),
    Soft {
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        _context: softbuffer::Context<Arc<Window>>,
        /// The last frame, drawn over where the next one changed.
        kept: soft::Retained,
    },
}

impl Renderer {
    /// What the window showed may be gone (shown again, another scale):
    /// the next frame is drawn and presented whole.
    fn invalidate(&mut self) {
        match self {
            Renderer::Gpu(s) => s.invalidate(),
            Renderer::Soft { kept, .. } => kept.invalidate(),
        }
    }
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
    /// The GPU device is lost and that was said: said once until a frame
    /// is drawn again.
    lost_said: bool,
    /// The window's display link (macOS 14: its screen's refreshes,
    /// ProMotion's 120 Hz included), running while frames are on their
    /// way to the screen.
    #[cfg(target_os = "macos")]
    link: Option<objc2::rc::Retained<objc2::runtime::AnyObject>>,
    /// Frames presented and not yet shown, oldest first: when each was
    /// presented, and the input it answers (`GUI_LATENCY`).
    in_flight: std::collections::VecDeque<(u64, Option<u64>)>,
    /// A frame asked for while one was still on its way: drawn at the next
    /// refresh, as that one shows (one frame a refresh, never a queue).
    deferred: bool,
    /// Refreshes since a frame was last on its way (the link pauses).
    idle_ticks: u32,
    /// Something moves at every refresh (a transition, a scroll's
    /// momentum or spring): the next frame is drawn at the display link's
    /// next tick, not on a timer.
    animating: bool,
    /// When the display link last ticked (host ns).
    last_tick: u64,
    /// The size last presented (a frame of another size is never put off:
    /// a live resize draws at once).
    shown_size: (u32, u32),
    /// The cursor the program asked for (put back when the window is
    /// responding again).
    cursor: winit::window::CursorIcon,
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
        #[cfg(target_os = "macos")]
        link,
        ..
    } = pw;
    #[cfg(target_os = "macos")]
    frames::unlink(link);
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
    /// The last context menu shown: its window, its items' ids (muda's
    /// to the program's) and its titles as the native menu held them.
    #[cfg(target_os = "macos")]
    context: Option<(u64, HashMap<String, String>, Value)>,
    /// A left press made a right one (⌃-click on macOS): its release is
    /// the right button's too.
    ctrl_click: bool,
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
        #[cfg(target_os = "macos")]
        edit_menu::view_opened(&win, id);
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
        let alone = if opts.tabbing.is_some() && opts.tab_of.is_none() {
            ns_window(&win)
        } else {
            None
        };
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
        #[cfg(target_os = "macos")]
        let win_for_link = win.clone();
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
                lost_said: false,
                #[cfg(target_os = "macos")]
                link: frames::link_for(&win_for_link, id),
                in_flight: std::collections::VecDeque::new(),
                deferred: false,
                idle_ticks: 0,
                animating: false,
                last_tick: 0,
                shown_size: (0, 0),
                cursor: winit::window::CursorIcon::Default,
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
                        | Input::Scroll { .. }
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
        // A frame still on its way to the screen: this one is drawn at the
        // next refresh, as that one shows, from the state as it is then —
        // a frame presented now would wait behind it (a refresh later,
        // and more as they queue). Never a frame of another size (a live
        // resize draws at once), and only while the display link ticks.
        // (`GUI_PACING=0` presents at once; under a stand-in refresh it
        // holds a frame only behind two, as the layer's own drawables
        // would — what a measurement compares against.)
        #[cfg(target_os = "macos")]
        {
            let synthetic = frames::synthetic_hz().is_some();
            let limit = if frames::pacing() {
                1
            } else if synthetic {
                2
            } else {
                usize::MAX
            };
            if (pw.link.is_some() || synthetic) && pw.in_flight.len() >= limit {
                let now = frames::now_ns();
                // a frame long on its way was shown (a refresh missed: the
                // window was covered, the link paused); a link that has not
                // ticked lately (the system rests it for a window it does
                // not show, and for a display asleep) puts nothing off
                while pw
                    .in_flight
                    .front()
                    .is_some_and(|f| now.saturating_sub(f.0) > 50_000_000)
                {
                    pw.in_flight.pop_front();
                }
                if now.saturating_sub(pw.last_tick) > 50_000_000 {
                    pw.in_flight.clear();
                }
                let size = pw.state.lock().map(|st| st.device_size()).unwrap_or((0, 0));
                if pw.in_flight.len() >= limit && size == pw.shown_size {
                    pw.deferred = true;
                    frames::resume(&pw.link);
                    return;
                }
            }
        }
        pw.deferred = false;
        // covered, minimized or on another space: nothing to draw (nor its
        // damage to work out); the frame stays owed and is drawn as the
        // window shows again (`Occluded(false)`)
        if pw.occluded {
            pw.animating = false;
            if let Ok(mut st) = pw.state.lock() {
                st.dirty = true;
            }
            return;
        }
        let trace = std::env::var_os("GUI_TRACE").is_some();
        let started = std::time::Instant::now();
        let mut moved = Vec::new();
        let (dl, a11y_tree, wants_ime, ime_area) = {
            let Ok(mut st) = pw.state.lock() else {
                return;
            };
            // a glide moves on (its steps said once the frame is out)
            st.tick(&mut moved);
            // a drag-select past a field's edge: its step this frame
            let mut said = Vec::new();
            st.autoscroll(&mut said);
            if !said.is_empty() {
                emit(said);
            }
            #[allow(unused_mut)]
            let mut dl = st.display_list();
            // the program's turn running long: the veil and the spinner,
            // turning (the next frame for it when it next turns)
            let spin = turn_watch::shown().then(|| turn_watch::overlay(&mut dl, st.scale));
            // an animation in view: the next frame when it is due (none
            // while the window cannot be seen)
            let unseen = pw.occluded
                || pw.win.is_visible() == Some(false)
                || pw.win.is_minimized() == Some(true);
            // due by the next refresh (a transition, a scroll moving): the
            // display link's next tick draws it, where the window has one
            pw.animating = false;
            pw.wake = match st.next_frame {
                Some(t) if !unseen => {
                    let ms = (t - st.clock_ms()).clamp(1.0, 60_000.0);
                    #[cfg(target_os = "macos")]
                    let linked =
                        ms <= 8.0 && (pw.link.is_some() || frames::synthetic_hz().is_some());
                    #[cfg(not(target_os = "macos"))]
                    let linked = false;
                    if linked {
                        pw.animating = true;
                        None
                    } else {
                        Some(started + std::time::Duration::from_micros((ms * 1000.0) as u64))
                    }
                }
                _ => None,
            };
            if std::env::var_os("GUI_TRACE").is_some() {
                #[cfg(target_os = "macos")]
                let linked = pw.link.is_some();
                #[cfg(not(target_os = "macos"))]
                let linked = false;
                eprintln!(
                    "gui: window {id} next frame {:?} at clock {:.1} (animating {}, link {linked}, unseen {unseen})",
                    st.next_frame,
                    st.clock_ms(),
                    pw.animating
                );
            }
            if let Some(next) = spin.flatten()
                && !unseen
            {
                pw.wake = Some(pw.wake.map_or(next, |w| w.min(next)));
            }
            st.dirty = false;
            // a frame drawn whole once (the rainbow turned off)
            st.redraw = false;
            // the tree is built only when a client is listening (a long
            // styled text is a run a paragraph)
            let tree = st.a11y_dirty;
            st.a11y_dirty = false;
            let wants = st.wants_ime();
            let area = if wants { st.ime_area() } else { None };
            (dl, tree, wants, area)
        };
        let listed = started.elapsed();
        emit(std::mem::take(&mut moved));
        let presented = match &mut pw.renderer {
            Renderer::Gpu(s) => match s.render(&dl) {
                Ok(()) => s.presented(),
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
                    false
                }
                // The device was lost and none could be made again yet:
                // the frame stays owed and is tried again in a moment.
                Err(e) if e.starts_with("lost") => {
                    if !pw.lost_said {
                        eprintln!(
                            "gui: window {id}: the GPU device is lost; drawing again when one can be had"
                        );
                        pw.lost_said = true;
                    }
                    if let Ok(mut st) = pw.state.lock() {
                        st.dirty = true;
                    }
                    pw.wake = Some(started + std::time::Duration::from_millis(250));
                    return;
                }
                Err(e) => {
                    eprintln!("gui: {e}");
                    false
                }
            },
            Renderer::Soft { surface, kept, .. } => present_soft(surface, kept, &dl, &pw.win),
        };
        pw.lost_said = false;
        PRESENTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if presented {
            SHOWN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        // on its way to the screen: shown at a coming refresh (a frame
        // that changed nothing was not presented: nothing is on its way)
        #[cfg(target_os = "macos")]
        if presented {
            pw.shown_size = (dl.width, dl.height);
            pw.in_flight
                .push_back((frames::now_ns(), frames::answered(id)));
            frames::resume(&pw.link);
            pw.idle_ticks = 0;
        }
        #[cfg(target_os = "macos")]
        if pw.animating {
            frames::resume(&pw.link);
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
        kept: soft::Retained::default(),
    })
}

fn present_soft(
    surface: &mut softbuffer::Surface<Arc<Window>, Arc<Window>>,
    kept: &mut soft::Retained,
    dl: &super::window::DisplayList,
    win: &Window,
) -> bool {
    use super::damage::{Damage, IRect};
    use std::num::NonZeroU32;
    kept.render(dl);
    let damage = kept.presented();
    if damage == Damage::None {
        return false;
    }
    let Some(pm) = kept.pixmap() else {
        return false;
    };
    let (Some(w), Some(h)) = (NonZeroU32::new(pm.width()), NonZeroU32::new(pm.height())) else {
        return false;
    };
    if surface.resize(w, h).is_err() {
        return false;
    }
    let Ok(mut buf) = surface.buffer_mut() else {
        return false;
    };
    // a buffer holding the last frame takes only what changed
    let whole = [IRect::new(0, 0, pm.width() as i32, pm.height() as i32)];
    let rects: &[IRect] = match &damage {
        Damage::Rects(r) if buf.age() == 1 => r,
        _ => &whole,
    };
    let pw = pm.width() as usize;
    let px = pm.pixels();
    for r in rects {
        for y in r.y0 as usize..r.y1 as usize {
            let row = y * pw;
            for x in r.x0 as usize..r.x1 as usize {
                let p = px[row + x];
                // Opaque windows: the premultiplied colour is the colour.
                buf[row + x] =
                    ((p.red() as u32) << 16) | ((p.green() as u32) << 8) | p.blue() as u32;
            }
        }
    }
    win.pre_present_notify();
    let damaged: Vec<softbuffer::Rect> = rects
        .iter()
        .filter_map(|r| {
            Some(softbuffer::Rect {
                x: r.x0 as u32,
                y: r.y0 as u32,
                width: NonZeroU32::new((r.x1 - r.x0) as u32)?,
                height: NonZeroU32::new((r.y1 - r.y0) as u32)?,
            })
        })
        .collect();
    buf.present_with_damage(&damaged).is_ok()
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
/// The phase of the scroll event AppKit is handling (winit hands its
/// `scrollWheel:` on at once, so it is the application's current event):
/// the momentum's first, else the fingers'. `Legacy` when it says none (a
/// mouse wheel, a device without phases) or is not a scroll.
#[cfg(target_os = "macos")]
fn scroll_phase() -> super::scroll::Phase {
    use super::scroll::Phase;
    use objc2_app_kit::{NSApplication, NSEventPhase, NSEventType};
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        return Phase::Legacy;
    };
    let Some(ev) = NSApplication::sharedApplication(mtm).currentEvent() else {
        return Phase::Legacy;
    };
    if ev.r#type() != NSEventType::ScrollWheel {
        return Phase::Legacy;
    }
    let m = ev.momentumPhase();
    let p = ev.phase();
    let is = |x: NSEventPhase, y: NSEventPhase| x.0 & y.0 != 0;
    if m.0 != 0 {
        if is(m, NSEventPhase::Began) {
            Phase::MomentumBegan
        } else if is(m, NSEventPhase::Ended) || is(m, NSEventPhase::Cancelled) {
            Phase::MomentumEnded
        } else {
            Phase::Momentum
        }
    } else if p.0 != 0 {
        if is(p, NSEventPhase::Began) || is(p, NSEventPhase::MayBegin) {
            Phase::Began
        } else if is(p, NSEventPhase::Ended) || is(p, NSEventPhase::Cancelled) {
            Phase::Ended
        } else {
            Phase::Changed
        }
    } else {
        Phase::Legacy
    }
}

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
/// A UTF-16 offset into `s` (AppKit's text input counts so) as
/// characters, clamped to its end.
pub fn utf16_to_chars(s: &str, u16s: usize) -> usize {
    let mut n = 0;
    for (i, c) in s.chars().enumerate() {
        if n >= u16s {
            return i;
        }
        n += c.len_utf16();
    }
    s.chars().count()
}

/// Characters into `s` as a UTF-16 offset.
pub fn chars_to_utf16(s: &str, chars: usize) -> usize {
    s.chars().take(chars).map(char::len_utf16).sum()
}

unsafe fn ns_text(s: *mut objc2::runtime::AnyObject) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: the caller's NSString; UTF8String lives as long as it.
    let p: *const std::ffi::c_char = unsafe { objc2::msg_send![&*s, UTF8String] };
    if p.is_null() {
        return None;
    }
    Some(
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned(),
    )
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
    // AppKit calls in: nothing may unwind into it
    contain_callback("open_urls", || unsafe { open_urls(urls) });
}

/// The body of [`app_open_urls`].
#[cfg(target_os = "macos")]
unsafe fn open_urls(urls: *mut objc2::runtime::AnyObject) {
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
        emit(vec![event(
            "open",
            vec![("paths", Value::List(Arc::new(paths)))],
        )]);
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
    contain_callback("new_tab", || emit(vec![event("new_tab", vec![])]));
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
        let open: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) =
            app_open_urls;
        let tab: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = app_new_tab;
        objc2::ffi::class_addMethod(
            cls,
            objc2::sel!(application:openURLs:),
            std::mem::transmute::<
                unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
                Imp,
            >(open),
            c"v@:@@".as_ptr(),
        );
        let should: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize =
            quit_hold::app_should_terminate;
        objc2::ffi::class_addMethod(
            cls,
            objc2::sel!(applicationShouldTerminate:),
            std::mem::transmute::<
                unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize,
                Imp,
            >(should),
            c"Q@:@".as_ptr(),
        );
        let reply: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) =
            quit_hold::app_reply_terminate;
        objc2::ffi::class_addMethod(
            cls,
            objc2::sel!(olangReplyTerminate:),
            std::mem::transmute::<
                unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                Imp,
            >(reply),
            c"v@:@".as_ptr(),
        );
        quit_hold::set_delegate(obj as usize);
        objc2::ffi::class_addMethod(
            cls,
            objc2::sel!(newWindowForTab:),
            std::mem::transmute::<
                unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                Imp,
            >(tab),
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
            .filter_map(|(i, p)| {
                ns_window(&p.win).map(|nw| (objc2::rc::Retained::as_ptr(&nw) as usize, *i))
            })
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
                    let o: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![&*tabs, objectAtIndex: k];
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
                (
                    "tabs",
                    Value::List(Arc::new(
                        ids.into_iter().map(|i| Value::Integer(i as i64)).collect(),
                    )),
                ),
            ],
        )]);
    }
}

/// Every call winit makes into the application, contained: a panic in
/// handling one is a problem said to the program (a `problem` event) and
/// on stderr, the windows' states made usable again and drawn again —
/// never the event loop stopped (winit stops the application on a panic
/// in a handler and goes on unwinding as it returns) or an abort.
impl ApplicationHandler<Cmd> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        self.contained("resumed", |app| app.on_resumed(el));
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.contained("about_to_wait", |app| app.on_about_to_wait(el));
    }

    fn user_event(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
        self.contained("user_event", |app| app.on_user_event(el, cmd));
    }

    fn window_event(&mut self, el: &ActiveEventLoop, wid: WindowId, we: WindowEvent) {
        self.contained("window_event", |app| app.on_window_event(el, wid, we));
    }

    // The platform is ending the application (AppKit's terminate, after
    // the program finished an external quit it held): the process ends
    // with the program's own code, its output flushed.
    fn exiting(&mut self, _el: &ActiveEventLoop) {
        quit_hold::exiting();
    }
}

/// How many frames the platform's windows have presented.
static PRESENTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many frames the platform's windows have drawn (presented, or found
/// to change nothing): a test checks drawing goes on after a device was
/// lost or a callback panicked.
pub fn frames_presented() -> u64 {
    PRESENTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// How many frames were presented (a frame that changed nothing is not).
static SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many frames the platform's windows presented: a window whose
/// pixels were lost (its device made again) presents its next frame
/// whole, though nothing in it changed.
pub fn frames_shown() -> u64 {
    SHOWN.load(std::sync::atomic::Ordering::SeqCst)
}

/// How many problems the platform side has contained.
static CONTAINED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many panics in the platform's callbacks (and the engine's
/// builtins) were contained.
pub fn problems_contained() -> u64 {
    CONTAINED.load(std::sync::atomic::Ordering::SeqCst)
}

/// A panic contained at `site` (`message` what panicked and where): said
/// on stderr and to the program (`problem`, `#{ where, message }`), and
/// every window's state, poisoned by a panic while it was held, made
/// usable again. The state is as the panic left it: the window goes on
/// from there rather than the process ending with the person's work.
pub fn contained_problem(site: &str, message: &str) {
    CONTAINED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    eprintln!("gui: a problem was contained in the engine ({message}); the program goes on");
    windows().clear_poison();
    let ws: Vec<Shared> = windows()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect();
    for st in ws {
        st.clear_poison();
        if let Ok(mut st) = st.lock() {
            st.dirty = true;
        }
    }
    emit(vec![event(
        "problem",
        vec![("where", s(site)), ("message", s(message))],
    )]);
}

/// Run `f` (a platform callback's body) with a panic inside it contained
/// ([`contained_problem`]); `Some` its answer when it ran to the end.
pub fn contain_callback<R>(site: &str, f: impl FnOnce() -> R) -> Option<R> {
    match crate::contain::catch(site, || {
        crate::contain::inject(site);
        f()
    }) {
        Ok(r) => Some(r),
        Err(e) => {
            contained_problem(site, &e);
            None
        }
    }
}

impl App {
    /// A winit callback's body run contained ([`contain_callback`]); after
    /// a panic every window draws again.
    fn contained(&mut self, site: &str, f: impl FnOnce(&mut App)) {
        if contain_callback(site, || f(self)).is_none() {
            for pw in self.windows.values() {
                pw.win.request_redraw();
            }
        }
    }

    fn on_resumed(&mut self, el: &ActiveEventLoop) {
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

    fn on_about_to_wait(&mut self, el: &ActiveEventLoop) {
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

    fn on_user_event(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
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
                Cmd::ContextMenu { id, .. } => format!("context menu {id}"),
                Cmd::Settings(why) => format!("settings ({why})"),
                Cmd::Edit { id, role } => format!("edit {role} {id}"),
                // a refresh: too many to say
                Cmd::Tick { .. } => String::new(),
                Cmd::NotResponding(on) => format!("not responding {on}"),
                Cmd::ServiceText { id, .. } => format!("service text {id}"),
                Cmd::InsertText { id, .. } => format!("insert text {id}"),
            };
            if !what.is_empty() {
                eprintln!("gui: {what}");
            }
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
                    #[cfg(target_os = "macos")]
                    edit_menu::view_closed(id);
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
                if let Some(pw) = self.windows.get_mut(&id) {
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
                        let icon = match c {
                            "pointer" => CursorIcon::Pointer,
                            "text" => CursorIcon::Text,
                            "wait" => CursorIcon::Wait,
                            "move" => CursorIcon::Move,
                            "ew-resize" => CursorIcon::EwResize,
                            "ns-resize" => CursorIcon::NsResize,
                            "not-allowed" => CursorIcon::NotAllowed,
                            _ => CursorIcon::Default,
                        };
                        if !turn_watch::shown() {
                            pw.win.set_cursor(icon);
                        }
                        pw.cursor = icon;
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
                                "next" => {
                                    let _: () = objc2::msg_send![&*w, selectNextTab: none];
                                }
                                "previous" => {
                                    let _: () = objc2::msg_send![&*w, selectPreviousTab: none];
                                }
                                "merge" => {
                                    let _: () = objc2::msg_send![&*w, mergeAllWindows: none];
                                }
                                "detach" => {
                                    let _: () = objc2::msg_send![&*w, moveTabToNewWindow: none];
                                }
                                "bar" => {
                                    let _: () = objc2::msg_send![&*w, toggleTabBar: none];
                                }
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
            Cmd::MenuEvent(id) => {
                // an item of a context menu: its window hears `context`
                #[cfg(target_os = "macos")]
                if id.starts_with(CTX_PREFIX) {
                    if let Some((win, ids, titles)) = self.context.as_ref()
                        && let Some(item) = ids.get(&id)
                    {
                        emit(vec![event(
                            "context",
                            vec![
                                ("window", Value::Integer(*win as i64)),
                                ("id", s(item)),
                                ("items", titles.clone()),
                                ("picked", Value::Boolean(false)),
                            ],
                        )]);
                    }
                    return;
                }
                emit(vec![event("menu", vec![("id", s(&id))])])
            }
            Cmd::ContextMenu { id, items, x, y } => self.show_context(id, items, x, y),
            Cmd::Settings(why) => self.settings_changed(why, None),
            Cmd::Edit { id, role } => self.edit_item(id, role),
            Cmd::Tick { .. } => self.on_tick(cmd),
            Cmd::NotResponding(on) => self.not_responding(on),
            Cmd::ServiceText { id, text } => self.input(id, Input::ImeCommit(text)),
            Cmd::InsertText { id, text, replace } => {
                if let Some((loc, len)) = replace
                    && let Some(pw) = self.windows.get_mut(&id)
                    && let Ok(mut st) = pw.state.lock()
                    && let Some((value, ..)) = st.text_input()
                {
                    let a = utf16_to_chars(&value, loc);
                    let b = utf16_to_chars(&value, loc + len);
                    st.select_text_input(a, b);
                }
                self.input(id, Input::ImeCommit(text))
            }
            Cmd::Exit(code) => {
                #[cfg(target_os = "macos")]
                frames::report();
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

    fn on_window_event(&mut self, _el: &ActiveEventLoop, wid: WindowId, we: WindowEvent) {
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
        #[cfg(target_os = "macos")]
        if frames::measuring()
            && matches!(
                we,
                WindowEvent::KeyboardInput { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::CursorMoved { .. }
                    | WindowEvent::PinchGesture { .. }
            )
        {
            frames::input(id);
        }
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
                // a screen of another scale: the kept frame is drawn again whole
                if let Some(pw) = self.windows.get_mut(&id) {
                    pw.renderer.invalidate();
                }
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
            WindowEvent::RedrawRequested => {
                contain_callback("render", || self.render(id));
            }
            // Shown again after being covered: draw what was skipped (and
            // the animations go on).
            WindowEvent::Occluded(false) => {
                if let Some(pw) = self.windows.get_mut(&id) {
                    pw.occluded = false;
                    // what it showed may be gone: drawn whole
                    pw.renderer.invalidate();
                    pw.win.request_redraw();
                }
            }
            // Covered, minimized, or on another space: nothing to animate.
            WindowEvent::Occluded(true) => {
                if let Some(pw) = self.windows.get_mut(&id) {
                    pw.occluded = true;
                    pw.wake = None;
                    // no refresh shows them now: nothing is held for them
                    pw.in_flight.clear();
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
                // ⌃-click is the Mac's right click (the context menu)
                let ctrl_left = cfg!(target_os = "macos")
                    && button == MouseButton::Left
                    && if state == ElementState::Pressed {
                        self.ctrl_click = mods.ctrl && !mods.super_ && !mods.alt;
                        self.ctrl_click
                    } else {
                        std::mem::take(&mut self.ctrl_click)
                    };
                let button = match button {
                    MouseButton::Left if ctrl_left => "right",
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
                let (dx, dy, precise) = match delta {
                    // A line is three lines of 16 px text, as browsers do.
                    MouseScrollDelta::LineDelta(x, y) => (x * 48.0, y * 48.0, false),
                    MouseScrollDelta::PixelDelta(p) => {
                        let l = p.to_logical::<f32>(scale);
                        (l.x, l.y, true)
                    }
                };
                // where it is in its gesture: the event's own phases (winit
                // folds the momentum's into the fingers'), so a trackpad
                // stretches and bounces and a mouse wheel glides
                #[cfg(target_os = "macos")]
                let phase = scroll_phase();
                #[cfg(not(target_os = "macos"))]
                let phase = super::scroll::Phase::Legacy;
                if !precise || phase != super::scroll::Phase::Legacy {
                    self.input(
                        id,
                        Input::Scroll {
                            dx,
                            dy,
                            mods: None,
                            phase,
                            precise,
                        },
                    );
                } else {
                    self.input(id, Input::Wheel { dx, dy, mods: None });
                }
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
                let follows = self
                    .windows
                    .get(&id)
                    .is_some_and(|pw| pw.forced_theme.is_none());
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
    // while the platform waits for an external quit to finish, its loop
    // runs no handler: a request that waits for one would never return
    if quit_hold::asked() {
        return Err("gui.open: the application is ending".to_string());
    }
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
            let png = unsafe {
                rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props)
            }
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
    if quit_hold::asked() {
        return Err("gui.dialog: the application is ending".to_string());
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
    /// One of the Mac's standard Edit menu items (`standard`: `undo`,
    /// `redo`, `cut`, `copy`, `paste`, `paste_plain`, `delete`,
    /// `select_all`): AppKit's own selector and keys, sent to the focused
    /// field, else to the program as its chord when one of its commands
    /// binds that chord in a context in `contexts` (`"global"`, a role),
    /// else greyed.
    Standard {
        role: &'static str,
        label: Option<String>,
        contexts: Vec<String>,
    },
    /// A submenu of items (the Edit menu's Find).
    Sub {
        label: String,
        items: Vec<MenuItemSpec>,
    },
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
        out.push((title, menu_items(&items, 0)?));
    }
    Ok(MenuSpec(out))
}

/// A menu's items as `gui.menu` reads them (a submenu's the same, at
/// most four deep).
fn menu_items(items: &[Value], depth: usize) -> Result<Vec<MenuItemSpec>, String> {
    let what = "gui.menu";
    if depth > 4 {
        return Err(format!("{what}: submenus nest at most four deep"));
    }
    {
        let mut its = Vec::with_capacity(items.len());
        for it in items.iter() {
            if matches!(it, Value::String(t) if t.as_str() == "separator") {
                its.push(MenuItemSpec::Separator);
                continue;
            }
            if let Some(role) = get_str(it, "standard", what)? {
                let Some(role) = EDIT_ROLES.iter().find(|r| **r == role) else {
                    return Err(format!(
                        "{what}: \"{role}\" is no standard item ({})",
                        EDIT_ROLES.join(", ")
                    ));
                };
                let contexts = match get(it, "contexts") {
                    Some(Value::List(l)) => l
                        .iter()
                        .filter_map(|c| match c {
                            Value::String(t) => Some(t.to_string()),
                            _ => None,
                        })
                        .collect(),
                    _ => vec![],
                };
                its.push(MenuItemSpec::Standard {
                    role,
                    label: get_str(it, "label", what)?.map(str::to_string),
                    contexts,
                });
                continue;
            }
            if let Some(Value::List(sub)) = get(it, "submenu") {
                let label = get_str(it, "label", what)?
                    .ok_or_else(|| format!("{what}: a submenu needs a \"label\""))?
                    .to_string();
                its.push(MenuItemSpec::Sub {
                    label,
                    items: menu_items(sub, depth + 1)?,
                });
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
        Ok(its)
    }
}

/// The standard Edit menu items' roles, in the Mac's order.
pub const EDIT_ROLES: &[&str] = &[
    "undo",
    "redo",
    "cut",
    "copy",
    "paste",
    "paste_plain",
    "delete",
    "select_all",
];

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
        // the standard Edit items, put in AppKit's menu once muda has made
        // it: (menu, place among its items, role, label)
        let mut standards: Vec<(usize, usize, &'static str, Option<String>)> = Vec::new();
        let mut routes: HashMap<&'static str, Vec<String>> = HashMap::new();
        let sep = |sub: &Submenu| {
            sub.append(&PredefinedMenuItem::separator())
                .map_err(|e| format!("{what}: {e}"))
        };
        for (at, (title, items)) in spec.0.into_iter().enumerate() {
            let sub = Submenu::new(&title, true);
            // the Mac's own items every application has (AppKit's
            // selectors, its keys): the application menu's Services and
            // Hide (⌘H, ⌥⌘H, Show All) before its Quit, the Window menu's
            // Minimize (⌘M) and Zoom first and Bring All to Front last, the
            // View menu's Enter Full Screen (⌃⌘F) last
            let app_menu = at == 0;
            let quit_at = if app_menu {
                items.iter().position(|it| matches!(it, MenuItemSpec::Item { id, label, .. } if id.ends_with("quit") || label.starts_with("Quit")))
            } else {
                None
            };
            if title == "Window" {
                sub.append(&PredefinedMenuItem::minimize(None))
                    .map_err(|e| format!("{what}: {e}"))?;
                sub.append(&PredefinedMenuItem::maximize(Some("Zoom")))
                    .map_err(|e| format!("{what}: {e}"))?;
                if !items.is_empty() {
                    sep(&sub)?;
                }
            }
            let mut prev_sep = true;
            for (i, it) in items.into_iter().enumerate() {
                if quit_at == Some(i) {
                    if !prev_sep {
                        sep(&sub)?;
                    }
                    sub.append(&PredefinedMenuItem::services(None))
                        .map_err(|e| format!("{what}: {e}"))?;
                    sep(&sub)?;
                    sub.append(&PredefinedMenuItem::hide(Some(
                        format!("Hide {title}").as_str(),
                    )))
                    .map_err(|e| format!("{what}: {e}"))?;
                    sub.append(&PredefinedMenuItem::hide_others(None))
                        .map_err(|e| format!("{what}: {e}"))?;
                    sub.append(&PredefinedMenuItem::show_all(None))
                        .map_err(|e| format!("{what}: {e}"))?;
                    sep(&sub)?;
                }
                let (id, label, enabled, keys, checked) = match it {
                    MenuItemSpec::Separator => {
                        sub.append(&PredefinedMenuItem::separator())
                            .map_err(|e| format!("{what}: {e}"))?;
                        prev_sep = true;
                        continue;
                    }
                    MenuItemSpec::Standard {
                        role,
                        label,
                        contexts,
                    } => {
                        let before = standards.iter().filter(|s| s.0 == at).count();
                        standards.push((at, sub.items().len() + before, role, label));
                        routes.insert(role, contexts);
                        prev_sep = false;
                        continue;
                    }
                    MenuItemSpec::Sub { label, items } => {
                        let inner = Submenu::new(&label, true);
                        fill_submenu(&inner, items, &mut ids)?;
                        sub.append(&inner).map_err(|e| format!("{what}: {e}"))?;
                        prev_sep = false;
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
                prev_sep = false;
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
            if app_menu && quit_at.is_none() {
                sep(&sub)?;
                sub.append(&PredefinedMenuItem::services(None))
                    .map_err(|e| format!("{what}: {e}"))?;
                sep(&sub)?;
                sub.append(&PredefinedMenuItem::hide(Some(
                    format!("Hide {title}").as_str(),
                )))
                .map_err(|e| format!("{what}: {e}"))?;
                sub.append(&PredefinedMenuItem::hide_others(None))
                    .map_err(|e| format!("{what}: {e}"))?;
                sub.append(&PredefinedMenuItem::show_all(None))
                    .map_err(|e| format!("{what}: {e}"))?;
            }
            if title == "Window" {
                sep(&sub)?;
                sub.append(&PredefinedMenuItem::bring_all_to_front(None))
                    .map_err(|e| format!("{what}: {e}"))?;
            }
            if title == "View" {
                sep(&sub)?;
                sub.append(&PredefinedMenuItem::fullscreen(Some("Enter Full Screen")))
                    .map_err(|e| format!("{what}: {e}"))?;
            }
            bar.append(&sub).map_err(|e| format!("{what}: {e}"))?;
        }
        // the standard items in place before the bar is the application's:
        // AppKit adds its own to an Edit menu (AutoFill, Start Dictation,
        // Emoji & Symbols) as it takes the bar, once
        {
            use muda::ContextMenu;
            // SAFETY: muda's own menu bar, on the loop's (main) thread.
            unsafe {
                edit_menu::install(
                    bar.ns_menu() as *mut objc2::runtime::AnyObject,
                    &standards,
                    routes,
                );
            }
        }
        bar.init_for_nsapp();
        self.menu_handler();
        self.menu = Some((bar, ids));
        Ok(())
    }

    /// The program's turn has run long (`on`), or ended: every window shows
    /// it is waiting — the busy cursor, a veil and a spinner over the
    /// frame, turning while the engine draws on (its scrolls, looks and
    /// fields still answer) — or is itself again.
    fn not_responding(&mut self, on: bool) {
        use winit::window::CursorIcon;
        for pw in self.windows.values_mut() {
            pw.win
                .set_cursor(if on { CursorIcon::Wait } else { pw.cursor });
            if let Ok(mut st) = pw.state.lock() {
                st.dirty = true;
            }
            pw.win.request_redraw();
        }
    }

    /// A refresh of window `id`'s display: the oldest frame on its way is
    /// on the screen now (one a refresh: the layer shows them in turn), a
    /// frame put off for it is drawn, and a link with nothing on its way
    /// for a few refreshes pauses (an idle window wakes for nothing).
    fn on_tick(&mut self, cmd: Cmd) {
        let Cmd::Tick { id, at } = cmd else {
            return;
        };
        // a refresh stood in for every window (`GUI_REFRESH_HZ`)
        if id == 0 {
            let ids: Vec<u64> = self.windows.keys().copied().collect();
            for w in ids {
                self.on_tick(Cmd::Tick { id: w, at });
            }
            return;
        }
        let Some(pw) = self.windows.get_mut(&id) else {
            return;
        };
        #[cfg(target_os = "macos")]
        {
            pw.last_tick = frames::now_ns();
        }
        if let Some(&(p, t0)) = pw.in_flight.front()
            && p < at
        {
            pw.in_flight.pop_front();
            #[cfg(target_os = "macos")]
            frames::shown(p, t0, at);
            #[cfg(not(target_os = "macos"))]
            let _ = t0;
        }
        // a frame put off for this refresh (render holds it again while
        // the frames before it still wait), or the next of something moving
        if pw.deferred || pw.animating {
            pw.animating = false;
            if let Ok(mut st) = pw.state.lock() {
                st.dirty = true;
            }
            self.render(id);
            return;
        }
        if pw.in_flight.is_empty() && !pw.deferred {
            pw.idle_ticks += 1;
            #[cfg(target_os = "macos")]
            if pw.idle_ticks >= 8 {
                frames::pause(&pw.link);
            }
        }
    }

    /// A standard Edit item chosen for window `id`: its chord, as though
    /// pressed — the focused field edits with it, and anything else hears
    /// it as the key (Loom's command for that chord in the focused role's
    /// context). The modifiers the window knew are kept (a menu clicked
    /// with the mouse holds none).
    fn edit_item(&mut self, id: u64, role: &str) {
        let mac = cfg!(target_os = "macos");
        let cmd = |shift: bool| Mods {
            super_: mac,
            ctrl: !mac,
            shift,
            alt: false,
        };
        let (key, mods) = match role {
            "undo" => ("z", cmd(false)),
            "redo" => ("z", cmd(true)),
            "cut" => ("x", cmd(false)),
            "copy" => ("c", cmd(false)),
            "paste" | "paste_plain" => ("v", cmd(false)),
            "select_all" => ("a", cmd(false)),
            "delete" => ("delete", Mods::default()),
            _ => return,
        };
        let Some(state) = self.windows.get(&id).map(|pw| pw.state.clone()) else {
            return;
        };
        let held = state.lock().map(|st| st.mods).unwrap_or_default();
        self.input(
            id,
            Input::Key {
                key: key.to_string(),
                text: None,
                mods,
                repeat: false,
            },
        );
        if let Ok(mut st) = state.lock() {
            st.mods = held;
        }
    }

    /// Every menu's chosen item comes back through the loop (the menu
    /// bar's as `menu`, a context menu's as `context`).
    #[cfg(target_os = "macos")]
    fn menu_handler(&self) {
        let proxy = self.proxy.clone();
        muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
            // called from AppKit's menu action
            contain_callback("menu_event", || {
                let _ = proxy.send_event(Cmd::MenuEvent(e.id.0.clone()));
            });
        }));
    }

    #[cfg(not(target_os = "macos"))]
    fn show_context(&mut self, id: u64, _items: Vec<ContextItem>, _x: f64, _y: f64) {
        emit(vec![event(
            "context",
            vec![
                ("window", Value::Integer(id as i64)),
                ("id", Value::Unit),
                ("items", Value::List(Arc::new(vec![]))),
                ("picked", Value::Boolean(false)),
            ],
        )]);
    }

    /// A native context menu (NSMenu, `popUpMenuPositioningItem:
    /// atLocation:inView:`) over window `id` at `(x, y)`. It tracks the
    /// pointer until an item is chosen or it is dismissed: chosen, the
    /// window hears `context` with the item's `id` (through the menu's
    /// handler, after this returns); dismissed, `context` with `id` `()`.
    /// A pick a script armed (`gui.input`'s `context_pick`) shows the
    /// menu, then closes it after a moment as though that item was chosen.
    #[cfg(target_os = "macos")]
    fn show_context(&mut self, id: u64, items: Vec<ContextItem>, x: f64, y: f64) {
        use muda::ContextMenu;
        let dismissed = |titles: Value, picked: Option<String>| {
            let got = picked.is_some();
            emit(vec![event(
                "context",
                vec![
                    ("window", Value::Integer(id as i64)),
                    ("id", picked.map(|p| s(&p)).unwrap_or(Value::Unit)),
                    ("items", titles),
                    ("picked", Value::Boolean(got)),
                ],
            )]);
        };
        let Some(pw) = self.windows.get(&id) else {
            return dismissed(Value::List(Arc::new(vec![])), None);
        };
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let view = match pw.win.window_handle().ok().map(|h| h.as_raw()) {
            Some(RawWindowHandle::AppKit(h)) => h.ns_view.as_ptr() as *const std::ffi::c_void,
            _ => return dismissed(Value::List(Arc::new(vec![])), None),
        };
        let menu = muda::Menu::new();
        let mut ids = HashMap::new();
        let mut by_label = HashMap::new();
        let mut n = 0usize;
        if let Err(e) = context_fill(&menu, &items, &mut ids, &mut by_label, &mut n) {
            eprintln!("gui.context_menu: {e}");
            return dismissed(Value::List(Arc::new(vec![])), None);
        }
        self.menu_handler();
        let ns_menu = menu.ns_menu() as *mut objc2::runtime::AnyObject;
        // the titles as the native menu holds them (a live check reads
        // what was shown, not what was asked)
        let titles: Vec<Value> = unsafe {
            let count: isize = objc2::msg_send![&*ns_menu, numberOfItems];
            (0..count)
                .filter_map(|i| {
                    let it: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![&*ns_menu, itemAtIndex: i];
                    if it.is_null() {
                        return None;
                    }
                    let sep: bool = objc2::msg_send![&*it, isSeparatorItem];
                    if sep {
                        return Some(s("-"));
                    }
                    let t: *mut objc2::runtime::AnyObject = objc2::msg_send![&*it, title];
                    ns_text(t).map(|t| s(&t))
                })
                .collect()
        };
        let titles = Value::List(Arc::new(titles));
        let pick = context_picks().lock().ok().and_then(|mut p| p.remove(&id));
        if pick.is_some() {
            // closed by the run loop's tracking mode after a moment: the
            // menu was up, as a person would see it
            unsafe {
                let modes = objc2_foundation::NSArray::from_retained_slice(&[
                    objc2_foundation::NSString::from_str("NSEventTrackingRunLoopMode"),
                    objc2_foundation::NSString::from_str("kCFRunLoopDefaultMode"),
                ]);
                let none: *const objc2::runtime::AnyObject = std::ptr::null();
                let _: () = objc2::msg_send![&*ns_menu, performSelector: objc2::sel!(cancelTracking), withObject: none, afterDelay: 0.6f64, inModes: &*modes];
            }
        }
        self.context = Some((id, ids, titles.clone()));
        let shown_at = std::time::Instant::now();
        // a script's pick also measures where the platform put the menu:
        // a live check compares it with where it asked
        let content = unsafe { content_on_screen(view) };
        let menu_size = unsafe {
            let sz: objc2_foundation::NSSize = objc2::msg_send![&*ns_menu, size];
            (sz.width, sz.height)
        };
        // the menu's own window, read on the main thread while it tracks
        let seen = std::sync::Arc::new(Mutex::new(None::<(f64, f64, f64, f64)>));
        let timer = pick
            .is_some()
            .then(|| unsafe { menu_frame_timer(seen.clone()) });
        let chosen = unsafe { context_popup(ns_menu, view, x, y) };
        if let Some(tm) = &timer {
            unsafe {
                let _: () = objc2::msg_send![&**tm, invalidate];
            }
        }
        if let Some(label) = pick {
            let open_ms = shown_at.elapsed().as_millis();
            let item = by_label.get(&label).cloned();
            if std::env::var_os("GUI_TRACE").is_some() {
                eprintln!("gui: context menu open {open_ms} ms, picked {label:?}");
            }
            let frame = seen.lock().ok().and_then(|f| *f);
            let place = |v: f64, k: &str| (k.to_string(), Value::Float(v));
            let mut fields = vec![
                ("window", Value::Integer(id as i64)),
                ("id", item.map(|p| s(&p)).unwrap_or(Value::Unit)),
                ("items", titles),
                ("picked", Value::Boolean(true)),
            ];
            // both in the screen's global points (y down from the top of the
            // main display): the point asked for (the window's content
            // origin on the screen plus the point) and the menu's own window
            // as AppKit placed it; `size` the menu's
            if let (Some((fx, fy, fw, fh)), Some((cx, cy))) = (frame, content) {
                fields.push(("at", map_of(vec![place(cx + x, "x"), place(cy + y, "y")])));
                fields.push((
                    "frame",
                    map_of(vec![
                        place(fx, "x"),
                        place(fy, "y"),
                        place(fw, "w"),
                        place(fh, "h"),
                    ]),
                ));
                fields.push((
                    "size",
                    map_of(vec![place(menu_size.0, "w"), place(menu_size.1, "h")]),
                ));
            }
            return emit(vec![event("context", fields)]);
        }
        if !chosen {
            dismissed(titles, None);
        }
    }
}

/// A map of `fields` (a context event's places).
#[cfg(target_os = "macos")]
fn map_of(fields: Vec<(String, Value)>) -> Value {
    let mut h = crate::ast::ValueMap::default();
    for (k, v) in fields {
        h.insert(k, v);
    }
    Value::Map(Arc::new(h))
}

/// Where a context menu's top left goes, in AppKit's screen points (y
/// up): at the click, unless the menu would leave the screen's visible
/// part — then it opens upward from the click (its bottom there), or to
/// the left of it (its right edge there), as the Mac's own menus do; held
/// inside when neither fits. `click` and the result are points, `menu`
/// its size, `visible` the screen's visible frame `(x, y, w, h)` (y its
/// bottom).
pub fn context_menu_origin(
    click: (f64, f64),
    menu: (f64, f64),
    visible: (f64, f64, f64, f64),
) -> (f64, f64) {
    let (cx, cy) = click;
    let (mw, mh) = menu;
    let (vx, vy, vw, vh) = visible;
    let (right, top) = (vx + vw, vy + vh);
    let x = if cx + mw <= right {
        cx
    } else if cx - mw >= vx {
        cx - mw
    } else {
        right - mw
    };
    let cy = cy.min(top);
    let y = if cy - mh >= vy {
        cy
    } else if cy + mh <= top {
        cy + mh
    } else {
        (vy + mh).min(top)
    };
    (x.max(vx), y)
}

/// Show `menu` with its top left at `(x, y)`, the window's content
/// points from its top left (Loom's), over `view`. winit's view is
/// flipped (y down), so the point is the view's own; a view that is not
/// would be turned. muda's `show_context_menu_for_nsview` turned it as
/// though every view were unflipped, so a menu opened mirrored about the
/// window's middle (a click near the top opened it near the bottom).
#[cfg(target_os = "macos")]
unsafe fn context_popup(
    menu: *mut objc2::runtime::AnyObject,
    view: *const std::ffi::c_void,
    x: f64,
    y: f64,
) -> bool {
    unsafe {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        use objc2_foundation::{NSPoint, NSRect, NSSize};
        let view = view as *mut AnyObject;
        let window: *mut AnyObject = msg_send![&*view, window];
        let flipped: bool = msg_send![&*view, isFlipped];
        let bounds: NSRect = msg_send![&*view, bounds];
        let local = NSPoint::new(x, if flipped { y } else { bounds.size.height - y });
        let none: *mut AnyObject = std::ptr::null_mut();
        let mut at = local;
        if !window.is_null() {
            let in_win: NSPoint = msg_send![&*view, convertPoint: local, toView: none];
            let on_screen: NSPoint = msg_send![&*window, convertPointToScreen: in_win];
            let screen: *mut AnyObject = msg_send![&*window, screen];
            if !screen.is_null() {
                let vis: NSRect = msg_send![&*screen, visibleFrame];
                let size: NSSize = msg_send![&*menu, size];
                let (ox, oy) = context_menu_origin(
                    (on_screen.x, on_screen.y),
                    (size.width, size.height),
                    (vis.origin.x, vis.origin.y, vis.size.width, vis.size.height),
                );
                let back: NSPoint =
                    msg_send![&*window, convertPointFromScreen: NSPoint::new(ox, oy)];
                at = msg_send![&*view, convertPoint: back, fromView: none];
            }
        }
        let item: *mut AnyObject = std::ptr::null_mut();
        msg_send![&*menu, popUpMenuPositioningItem: item, atLocation: at, inView: &*view]
    }
}

/// The top left of `view`'s content in the screen's global points with y
/// down from the top of the main screen (as the window server says
/// windows' bounds).
#[cfg(target_os = "macos")]
unsafe fn content_on_screen(view: *const std::ffi::c_void) -> Option<(f64, f64)> {
    unsafe {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        use objc2_foundation::NSRect;
        let view = view as *mut AnyObject;
        let window: *mut AnyObject = msg_send![&*view, window];
        if window.is_null() {
            return None;
        }
        let bounds: NSRect = msg_send![&*view, bounds];
        let none: *mut AnyObject = std::ptr::null_mut();
        let in_win: NSRect = msg_send![&*view, convertRect: bounds, toView: none];
        let r: NSRect = msg_send![&*window, convertRectToScreen: in_win];
        let screens: *mut AnyObject = msg_send![objc2::class!(NSScreen), screens];
        let count: usize = msg_send![&*screens, count];
        if count == 0 {
            return None;
        }
        let main: *mut AnyObject = msg_send![&*screens, objectAtIndex: 0usize];
        let mf: NSRect = msg_send![&*main, frame];
        Some((r.origin.x, mf.size.height - (r.origin.y + r.size.height)))
    }
}

/// A timer on the main run loop's tracking mode that keeps the frame of
/// this process's popup menu window (AppKit's NSPopupMenuWindow: the
/// menu itself; its shadow and glass are another, larger window) in
/// `out`, in the screen's global points with y down from the top of the
/// main display. A script's pick reads it: where the menu really opened.
#[cfg(target_os = "macos")]
type MenuFrame = std::sync::Arc<Mutex<Option<(f64, f64, f64, f64)>>>;

#[cfg(target_os = "macos")]
unsafe fn menu_frame_timer(out: MenuFrame) -> objc2::rc::Retained<objc2::runtime::AnyObject> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    unsafe {
        let block = block2::RcBlock::new(move |_t: *mut AnyObject| {
            contain_callback("menu_frame_timer", || menu_frame(&out));
        });
        let timer: objc2::rc::Retained<AnyObject> = msg_send![objc2::class!(NSTimer), timerWithTimeInterval: 0.05f64, repeats: true, block: &*block];
        let rl: *mut AnyObject = msg_send![objc2::class!(NSRunLoop), mainRunLoop];
        let mode = objc2_foundation::NSString::from_str("NSEventTrackingRunLoopMode");
        let _: () = msg_send![&*rl, addTimer: &*timer, forMode: &*mode];
        timer
    }
}

/// The popup menu window's frame kept in `out` ([`menu_frame_timer`]).
#[cfg(target_os = "macos")]
fn menu_frame(out: &MenuFrame) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSRect;
    // SAFETY: AppKit's application, windows and screens, read on the main
    // thread (the timer's).
    unsafe {
        {
            let app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
            let wins: *mut AnyObject = msg_send![&*app, windows];
            let n: usize = msg_send![&*wins, count];
            let screens: *mut AnyObject = msg_send![objc2::class!(NSScreen), screens];
            let ns: usize = msg_send![&*screens, count];
            if ns == 0 {
                return;
            }
            let main: *mut AnyObject = msg_send![&*screens, objectAtIndex: 0usize];
            let mf: NSRect = msg_send![&*main, frame];
            for i in 0..n {
                let w: *mut AnyObject = msg_send![&*wins, objectAtIndex: i];
                let vis: bool = msg_send![&*w, isVisible];
                let cls = (*w).class().name().to_string_lossy().to_string();
                if vis && cls.contains("PopupMenuWindow") {
                    let f: NSRect = msg_send![&*w, frame];
                    if let Ok(mut o) = out.lock() {
                        *o = Some((
                            f.origin.x,
                            mf.size.height - (f.origin.y + f.size.height),
                            f.size.width,
                            f.size.height,
                        ));
                    }
                }
            }
        }
    }
}

/// The prefix of a context menu item's id in muda (the menu bar's are the
/// program's own ids).
#[cfg(target_os = "macos")]
const CTX_PREFIX: &str = "\u{1}ctx:";

/// A context menu's items made into `menu` (a submenu's into its own).
#[cfg(target_os = "macos")]
fn context_fill(
    menu: &dyn ContextAppend,
    items: &[ContextItem],
    ids: &mut HashMap<String, String>,
    by_label: &mut HashMap<String, String>,
    n: &mut usize,
) -> Result<(), String> {
    use muda::accelerator::Accelerator;
    use muda::{CheckMenuItem, MenuItem, PredefinedMenuItem, Submenu};
    for it in items {
        match it {
            ContextItem::Separator => menu.add(&PredefinedMenuItem::separator())?,
            ContextItem::Item {
                id,
                label,
                enabled,
                keys,
                checked,
                submenu,
            } => {
                if let Some(subs) = submenu {
                    let sub = Submenu::new(label, *enabled);
                    context_fill(&sub, subs, ids, by_label, n)?;
                    menu.add(&sub)?;
                    continue;
                }
                *n += 1;
                let mid = format!("{CTX_PREFIX}{n}");
                let accel: Option<Accelerator> = keys.as_ref().and_then(|k| k.parse().ok());
                match checked {
                    Some(c) => menu.add(&CheckMenuItem::with_id(
                        mid.clone(),
                        label,
                        *enabled,
                        *c,
                        accel,
                    ))?,
                    None => menu.add(&MenuItem::with_id(mid.clone(), label, *enabled, accel))?,
                }
                ids.insert(mid, id.clone());
                if *enabled {
                    by_label.entry(label.clone()).or_insert_with(|| id.clone());
                }
            }
        }
    }
    Ok(())
}

/// A menu or a submenu, as a context menu's items are added to it.
#[cfg(target_os = "macos")]
trait ContextAppend {
    fn add(&self, item: &dyn muda::IsMenuItem) -> Result<(), String>;
}

#[cfg(target_os = "macos")]
impl ContextAppend for muda::Menu {
    fn add(&self, item: &dyn muda::IsMenuItem) -> Result<(), String> {
        self.append(item).map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
impl ContextAppend for muda::Submenu {
    fn add(&self, item: &dyn muda::IsMenuItem) -> Result<(), String> {
        self.append(item).map_err(|e| e.to_string())
    }
}

/// The picks scripts armed, by window: the next context menu of that
/// window is closed as though the item with that label was chosen.
fn context_picks() -> &'static Mutex<HashMap<u64, String>> {
    static PICKS: OnceLock<Mutex<HashMap<u64, String>>> = OnceLock::new();
    PICKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Arm window `id`'s next context menu to be closed with `label` chosen
/// (a live check's script: the native menu cannot be driven otherwise).
pub fn context_pick(id: u64, label: String) {
    if let Ok(mut p) = context_picks().lock() {
        p.insert(id, label);
    }
}

/// A context menu's item as `gui.context_menu` reads it.
pub enum ContextItem {
    Separator,
    Item {
        id: String,
        label: String,
        enabled: bool,
        keys: Option<String>,
        checked: Option<bool>,
        submenu: Option<Vec<ContextItem>>,
    },
}

/// A context menu's items read and checked (on the program's thread):
/// each `#{ id, label, keys?, enabled?, checked?, submenu? }` or
/// `"separator"`; a submenu's own items the same.
pub fn context_items(v: &Value, depth: usize) -> Result<Vec<ContextItem>, String> {
    let what = "gui.context_menu";
    let items = match v {
        Value::List(l) => l.clone(),
        _ => return Err(format!("{what}: the items are a list")),
    };
    if depth > 4 {
        return Err(format!("{what}: submenus nest at most four deep"));
    }
    let mut out = Vec::with_capacity(items.len());
    for it in items.iter() {
        if matches!(it, Value::String(t) if t.as_str() == "separator") {
            out.push(ContextItem::Separator);
            continue;
        }
        let label = get_str(it, "label", what)?
            .ok_or_else(|| format!("{what}: an item needs a \"label\""))?
            .to_string();
        let submenu = match get(it, "submenu") {
            Some(Value::List(_)) => Some(context_items(
                get(it, "submenu").expect("present"),
                depth + 1,
            )?),
            Some(Value::Unit) | None => None,
            Some(_) => return Err(format!("{what}: \"{label}\": a submenu is a list of items")),
        };
        let id = match get_str(it, "id", what)? {
            Some(i) => i.to_string(),
            None if submenu.is_some() => String::new(),
            None => return Err(format!("{what}: \"{label}\" needs an \"id\"")),
        };
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
        out.push(ContextItem::Item {
            id,
            label,
            enabled: get_bool(it, "enabled", what)?.unwrap_or(true),
            keys,
            checked: get_bool(it, "checked", what)?,
            submenu,
        });
    }
    Ok(out)
}

/// Show a native context menu over window `id` (macOS): `true` when it
/// is shown (the choice arrives as a `context` event), `false` where the
/// platform has none (the program draws its own).
pub fn context_menu(id: u64, items: Vec<ContextItem>, x: f64, y: f64) -> Result<Value, String> {
    if !cfg!(target_os = "macos") {
        return Ok(Value::Boolean(false));
    }
    let p = proxy().map_err(|e| format!("gui.context_menu: {e}"))?;
    p.send_event(Cmd::ContextMenu { id, items, x, y })
        .map_err(|_| "gui.context_menu: the event loop has ended".to_string())?;
    Ok(Value::Boolean(true))
}

/// A submenu's items made in muda (a standard item has no place there:
/// it is left out).
#[cfg(target_os = "macos")]
fn fill_submenu(
    sub: &muda::Submenu,
    items: Vec<MenuItemSpec>,
    ids: &mut HashMap<muda::MenuId, String>,
) -> Result<(), String> {
    use muda::{CheckMenuItem, MenuItem, PredefinedMenuItem, Submenu};
    let what = "gui.menu";
    for it in items {
        match it {
            MenuItemSpec::Separator => sub
                .append(&PredefinedMenuItem::separator())
                .map_err(|e| format!("{what}: {e}"))?,
            MenuItemSpec::Standard { .. } => {}
            MenuItemSpec::Sub { label, items } => {
                let inner = Submenu::new(&label, true);
                fill_submenu(&inner, items, ids)?;
                sub.append(&inner).map_err(|e| format!("{what}: {e}"))?;
            }
            MenuItemSpec::Item {
                id,
                label,
                enabled,
                keys,
                checked,
            } => {
                let accel: Option<muda::accelerator::Accelerator> =
                    keys.and_then(|k| k.parse().ok());
                let mid = muda::MenuId::new(&id);
                match checked {
                    Some(c) => sub
                        .append(&CheckMenuItem::with_id(
                            mid.clone(),
                            &label,
                            enabled,
                            c,
                            accel,
                        ))
                        .map_err(|e| format!("{what}: {e}"))?,
                    None => sub
                        .append(&MenuItem::with_id(mid.clone(), &label, enabled, accel))
                        .map_err(|e| format!("{what}: {e}"))?,
                }
                ids.insert(mid, id);
            }
        }
    }
    Ok(())
}

/// Whether a standard Edit item (`role`) does something in window `st`
/// now: the focused field takes it (copy and cut with a selection, never
/// from a password; undo when the field keeps its own; paste and select
/// all always), else the program does when one of its commands binds the
/// item's chord in a context the focus is in (`contexts`: `"global"`, or
/// the focused node's role). A field that hands the chord to the program
/// (`pass_keys`) is the program's.
pub fn edit_enabled(st: &WinState, role: &str, contexts: &[String]) -> bool {
    let chord = match role {
        "undo" => "mod+z",
        "redo" => "mod+shift+z",
        "cut" => "mod+x",
        "copy" => "mod+c",
        "paste" | "paste_plain" => "mod+v",
        "select_all" => "mod+a",
        "delete" => "delete",
        _ => return false,
    };
    let focus = st.scene.focus.as_deref();
    let node = focus.and_then(|f| st.scene.nodes.get(f));
    let field = focus
        .and_then(|f| st.editors.get(f))
        .filter(|_| node.is_some_and(|n| !n.disabled))
        .filter(|_| {
            !node
                .and_then(|n| n.edit.as_ref())
                .is_some_and(|e| e.pass_keys.iter().any(|k| k == chord))
        });
    if let Some(ed) = field {
        let mine = match role {
            "cut" | "copy" | "delete" => Some(ed.selected_text().is_some_and(|t| !t.is_empty())),
            "paste" | "paste_plain" | "select_all" => Some(true),
            _ if ed.own_undo() => Some(true),
            // undo and redo the field leaves to the program
            _ => None,
        };
        if let Some(m) = mine {
            return m;
        }
    }
    contexts
        .iter()
        .any(|c| c == "global" || node.is_some_and(|n| n.role == *c))
}

/// The standard Edit menu on macOS: AppKit's own items (their selectors,
/// keys and titles) answered by winit's view, which is the key window's
/// first responder — each item is enabled as [`edit_enabled`] says and
/// sends [`Cmd::Edit`] to the loop. The same view answers the Services
/// menu for a focused field (its selection sent, a Service's text put
/// in).
#[cfg(target_os = "macos")]
pub mod edit_menu {
    use super::{Cmd, EDIT_ROLES, contain_callback, edit_enabled, send};
    use crate::stdlib::gui::windows;
    use objc2::msg_send;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2_foundation::NSString;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    /// Each window's content view (winit's) by its address, to its id.
    fn views() -> &'static Mutex<HashMap<usize, u64>> {
        static VIEWS: OnceLock<Mutex<HashMap<usize, u64>>> = OnceLock::new();
        VIEWS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// The contexts the program's commands bind each standard item's
    /// chord in (the menu's spec says them).
    fn routes() -> &'static Mutex<HashMap<&'static str, Vec<String>>> {
        static ROUTES: OnceLock<Mutex<HashMap<&'static str, Vec<String>>>> = OnceLock::new();
        ROUTES.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub fn view_opened(win: &winit::window::Window, id: u64) {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = win.window_handle()
            && let RawWindowHandle::AppKit(h) = h.as_raw()
        {
            views()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(h.ns_view.as_ptr() as usize, id);
        }
        responder();
    }

    pub fn view_closed(id: u64) {
        views()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, v| *v != id);
    }

    /// Item `role`: its title, selector, key equivalent and modifiers
    /// (NSEventModifierFlags: shift 1<<17, option 1<<19, command 1<<20).
    fn item(role: &str) -> (&'static str, &'static std::ffi::CStr, &'static str, usize) {
        const SHIFT: usize = 1 << 17;
        const OPTION: usize = 1 << 19;
        const COMMAND: usize = 1 << 20;
        match role {
            "undo" => ("Undo", c"undo:", "z", COMMAND),
            "redo" => ("Redo", c"redo:", "z", COMMAND | SHIFT),
            "cut" => ("Cut", c"cut:", "x", COMMAND),
            "copy" => ("Copy", c"copy:", "c", COMMAND),
            "paste" => ("Paste", c"paste:", "v", COMMAND),
            "paste_plain" => (
                "Paste and Match Style",
                c"pasteAsPlainText:",
                "v",
                COMMAND | SHIFT | OPTION,
            ),
            "delete" => ("Delete", c"delete:", "", 0),
            _ => ("Select All", c"selectAll:", "a", COMMAND),
        }
    }

    /// The role of an item's selector.
    fn role_of(sel: Sel) -> Option<&'static str> {
        let name = sel.name();
        EDIT_ROLES.iter().copied().find(|r| item(r).1 == name)
    }

    /// Put the standard items into the menu bar `bar` (`at`: the menu,
    /// `place` among its items), and keep what routes them.
    ///
    /// # Safety
    ///
    /// `bar` is null or muda's menu bar (an NSMenu), and this runs on the
    /// main thread.
    pub unsafe fn install(
        bar: *mut AnyObject,
        standards: &[(usize, usize, &'static str, Option<String>)],
        routes_now: HashMap<&'static str, Vec<String>>,
    ) {
        *routes().lock().unwrap_or_else(|e| e.into_inner()) = routes_now;
        if standards.is_empty() {
            return;
        }
        responder();
        if bar.is_null() {
            return;
        }
        // SAFETY: muda's menu bar (an NSMenu), on the main thread (the
        // loop's); the items made here are retained by their menu.
        unsafe {
            let bar = &*bar;
            let Some(cls) = AnyClass::get(c"NSMenuItem") else {
                return;
            };
            for (at, place, role, label) in standards {
                let n: isize = msg_send![bar, numberOfItems];
                if *at as isize >= n {
                    continue;
                }
                let top: Retained<AnyObject> = msg_send![bar, itemAtIndex: *at as isize];
                let menu: Option<Retained<AnyObject>> = msg_send![&*top, submenu];
                let Some(menu) = menu else { continue };
                let (title, sel, key, mask) = item(role);
                let title = NSString::from_str(label.as_deref().unwrap_or(title));
                let key = NSString::from_str(key);
                let alloc: Allocated<AnyObject> = msg_send![cls, alloc];
                let it: Retained<AnyObject> = msg_send![alloc, initWithTitle: &*title, action: Sel::register(sel), keyEquivalent: &*key];
                let _: () = msg_send![&*it, setKeyEquivalentModifierMask: mask];
                let count: isize = msg_send![&*menu, numberOfItems];
                let _: () =
                    msg_send![&*menu, insertItem: &*it, atIndex: (*place as isize).min(count)];
            }
        }
    }

    /// The window state behind winit's view `this`.
    fn state_of(this: *mut AnyObject) -> Option<(u64, crate::stdlib::gui::Shared)> {
        let id = *views().lock().ok()?.get(&(this as usize))?;
        let st = windows().lock().ok()?.get(&id).cloned()?;
        Some((id, st))
    }

    unsafe extern "C-unwind" fn edit_action(
        this: *mut AnyObject,
        cmd: Sel,
        _sender: *mut AnyObject,
    ) {
        contain_callback("edit_action", || {
            if let (Some(role), Some((id, _))) = (role_of(cmd), state_of(this)) {
                send(Cmd::Edit { id, role });
            }
        });
    }

    unsafe extern "C-unwind" fn validate(
        this: *mut AnyObject,
        _cmd: Sel,
        item: *mut AnyObject,
    ) -> Bool {
        let on = contain_callback("edit_validate", || {
            if item.is_null() {
                return true;
            }
            // SAFETY: AppKit's NSMenuItem, alive for the call.
            let action: Option<Sel> = unsafe { msg_send![&*item, action] };
            let Some(role) = action.and_then(role_of) else {
                return true;
            };
            let Some((_, st)) = state_of(this) else {
                return false;
            };
            let contexts = routes()
                .lock()
                .ok()
                .and_then(|r| r.get(role).cloned())
                .unwrap_or_default();
            st.lock().is_ok_and(|st| edit_enabled(&st, role, &contexts))
        });
        Bool::new(on.unwrap_or(false))
    }

    /// The focused field's selection, when it may be sent (`None` from a
    /// password or with nothing selected).
    fn selection(this: *mut AnyObject) -> Option<String> {
        let (_, st) = state_of(this)?;
        let st = st.lock().ok()?;
        let f = st.scene.focus.as_deref()?;
        st.editors.get(f)?.selected_text().filter(|t| !t.is_empty())
    }

    /// Whether a field has the focus, to take a Service's text.
    fn takes_text(this: *mut AnyObject) -> bool {
        let Some((_, st)) = state_of(this) else {
            return false;
        };
        st.lock().is_ok_and(|st| {
            st.scene.focus.as_deref().is_some_and(|f| {
                st.editors.contains_key(f) && st.scene.nodes.get(f).is_some_and(|n| !n.disabled)
            })
        })
    }

    /// `validRequestorForSendType:returnType:` — the view takes part in
    /// the Services menu for text: it sends a selection, and takes text
    /// back into a focused field.
    unsafe extern "C-unwind" fn services_requestor(
        this: *mut AnyObject,
        _cmd: Sel,
        send_type: *mut AnyObject,
        return_type: *mut AnyObject,
    ) -> *mut AnyObject {
        contain_callback("services", || {
            let is_text = |t: *mut AnyObject| {
                // SAFETY: AppKit's NSString pasteboard types, or nil.
                !t.is_null()
                    && unsafe { super::ns_text(t) }
                        .is_some_and(|s| s == "public.utf8-plain-text" || s == "NSStringPboardType")
            };
            let sends = send_type.is_null() || (is_text(send_type) && selection(this).is_some());
            let takes = return_type.is_null() || (is_text(return_type) && takes_text(this));
            let any = !send_type.is_null() || !return_type.is_null();
            if sends && takes && any {
                this
            } else {
                std::ptr::null_mut()
            }
        })
        .unwrap_or(std::ptr::null_mut())
    }

    /// `writeSelectionToPasteboard:types:` — the selection to a Service.
    unsafe extern "C-unwind" fn services_write(
        this: *mut AnyObject,
        _cmd: Sel,
        pboard: *mut AnyObject,
        _types: *mut AnyObject,
    ) -> Bool {
        let ok = contain_callback("services", || {
            let Some(text) = selection(this) else {
                return false;
            };
            if pboard.is_null() {
                return false;
            }
            let t = NSString::from_str(&text);
            let ty = NSString::from_str("public.utf8-plain-text");
            // SAFETY: AppKit's NSPasteboard, alive for the call.
            unsafe {
                let _: isize = msg_send![&*pboard, clearContents];
                let b: Bool = msg_send![&*pboard, setString: &*t, forType: &*ty];
                b.as_bool()
            }
        });
        Bool::new(ok.unwrap_or(false))
    }

    /// `readSelectionFromPasteboard:` — a Service's text into the field.
    unsafe extern "C-unwind" fn services_read(
        this: *mut AnyObject,
        _cmd: Sel,
        pboard: *mut AnyObject,
    ) -> Bool {
        let ok = contain_callback("services", || {
            if pboard.is_null() || !takes_text(this) {
                return false;
            }
            let ty = NSString::from_str("public.utf8-plain-text");
            // SAFETY: AppKit's NSPasteboard, alive for the call.
            let text: Option<Retained<NSString>> =
                unsafe { msg_send![&*pboard, stringForType: &*ty] };
            let (Some(text), Some((id, _))) = (text, state_of(this)) else {
                return false;
            };
            send(Cmd::ServiceText {
                id,
                text: text.to_string(),
            });
            true
        });
        Bool::new(ok.unwrap_or(false))
    }

    // ── the text input client (dictation, Look Up, input methods) ──────

    /// AppKit's NSRange, NSPoint and NSRect, as passed by value.
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct Range {
        pub location: usize,
        pub length: usize,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct Point {
        pub x: f64,
        pub y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct Rect {
        pub origin: Point,
        pub size: Point,
    }
    // SAFETY: the layouts above are AppKit's (two NSUIntegers; two and
    // four CGFloats, which are f64 on every Mac olang runs on).
    unsafe impl objc2::Encode for Range {
        const ENCODING: objc2::Encoding =
            objc2::Encoding::Struct("_NSRange", &[usize::ENCODING, usize::ENCODING]);
    }
    unsafe impl objc2::RefEncode for Range {
        const ENCODING_REF: objc2::Encoding =
            objc2::Encoding::Pointer(&<Self as objc2::Encode>::ENCODING);
    }
    unsafe impl objc2::Encode for Point {
        const ENCODING: objc2::Encoding =
            objc2::Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
    }
    unsafe impl objc2::Encode for Rect {
        const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
            "CGRect",
            &[
                <Point as objc2::Encode>::ENCODING,
                objc2::Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]),
            ],
        );
    }

    const NOT_FOUND: usize = isize::MAX as usize;

    /// winit's own implementations, replaced below and called through.
    static OLD: OnceLock<Mutex<HashMap<&'static str, usize>>> = OnceLock::new();

    fn old(name: &'static str) -> Option<Imp> {
        let p = *OLD.get()?.lock().ok()?.get(name)?;
        // SAFETY: an IMP method_setImplementation answered.
        Some(unsafe { std::mem::transmute::<usize, Imp>(p) })
    }

    thread_local! {
        /// Inside winit's `keyDown:` (its `interpretKeyEvents:`): text
        /// inserted then is the key's, which winit sends as the key.
        static IN_KEY: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    unsafe fn has_marked(this: *mut AnyObject) -> bool {
        // SAFETY: winit's view, alive for the call.
        let b: Bool = unsafe { msg_send![&*this, hasMarkedText] };
        b.as_bool()
    }

    /// The text of an NSString or an NSAttributedString.
    unsafe fn string_of(obj: *mut AnyObject) -> Option<String> {
        if obj.is_null() {
            return None;
        }
        let attributed = AnyClass::get(c"NSAttributedString")?;
        // SAFETY: AppKit's string object, alive for the call.
        unsafe {
            let is: Bool = msg_send![&*obj, isKindOfClass: attributed];
            if is.as_bool() {
                let st: *mut AnyObject = msg_send![&*obj, string];
                super::ns_text(st)
            } else {
                super::ns_text(obj)
            }
        }
    }

    /// The focused field's text, when the view should answer for it (no
    /// composition under way: during one winit's answers, which count in
    /// the composition, stay).
    unsafe fn field_text(
        this: *mut AnyObject,
    ) -> Option<(u64, crate::stdlib::gui::Shared, String, usize, usize)> {
        if unsafe { has_marked(this) } {
            return None;
        }
        let (id, st) = state_of(this)?;
        let (text, a, b, composing) = st.lock().ok()?.text_input()?;
        if composing {
            return None;
        }
        Some((id, st, text, a, b))
    }

    unsafe extern "C-unwind" fn key_down(this: *mut AnyObject, cmd: Sel, event: *mut AnyObject) {
        let Some(imp) = old("keyDown:") else { return };
        IN_KEY.with(|k| k.set(k.get() + 1));
        struct Out;
        impl Drop for Out {
            fn drop(&mut self) {
                IN_KEY.with(|k| k.set(k.get().saturating_sub(1)));
            }
        }
        let _out = Out;
        // SAFETY: winit's keyDown:, with the arguments AppKit gave.
        unsafe {
            let f = std::mem::transmute::<
                Imp,
                unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
            >(imp);
            f(this, cmd, event)
        }
    }

    /// `insertText:replacementRange:` — winit takes text only to end a
    /// composition; text put in otherwise (dictation, the Character
    /// Viewer) goes to the focused field here.
    unsafe extern "C-unwind" fn insert_text(
        this: *mut AnyObject,
        cmd: Sel,
        string: *mut AnyObject,
        range: Range,
    ) {
        let in_key = IN_KEY.with(|k| k.get()) > 0;
        if !in_key && !unsafe { has_marked(this) } {
            let took = contain_callback("insert_text", || {
                let text = unsafe { string_of(string) }?;
                let (id, st) = state_of(this)?;
                if !st.lock().ok()?.wants_ime() || text.chars().next().is_some_and(char::is_control)
                {
                    return None;
                }
                let replace =
                    (range.location != NOT_FOUND).then_some((range.location, range.length));
                send(Cmd::InsertText { id, text, replace });
                Some(())
            })
            .flatten();
            if took.is_some() {
                return;
            }
        }
        if let Some(imp) = old("insertText:replacementRange:") {
            // SAFETY: winit's method, with the arguments AppKit gave.
            unsafe {
                let f = std::mem::transmute::<
                    Imp,
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Range),
                >(imp);
                f(this, cmd, string, range)
            }
        }
    }

    unsafe extern "C-unwind" fn selected_range(this: *mut AnyObject, cmd: Sel) -> Range {
        let mine = contain_callback("selected_range", || {
            let (_, _, text, a, b) = unsafe { field_text(this) }?;
            let lo = super::chars_to_utf16(&text, a);
            Some(Range {
                location: lo,
                length: super::chars_to_utf16(&text, b) - lo,
            })
        })
        .flatten();
        if let Some(r) = mine {
            return r;
        }
        match old("selectedRange") {
            // SAFETY: winit's method.
            Some(imp) => unsafe {
                std::mem::transmute::<Imp, unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> Range>(
                    imp,
                )(this, cmd)
            },
            None => Range {
                location: NOT_FOUND,
                length: 0,
            },
        }
    }

    unsafe extern "C-unwind" fn attributed_substring(
        this: *mut AnyObject,
        cmd: Sel,
        range: Range,
        actual: *mut Range,
    ) -> *mut AnyObject {
        let mine = contain_callback("attributed_substring", || {
            let (_, _, text, _, _) = unsafe { field_text(this) }?;
            let total = super::chars_to_utf16(&text, text.chars().count());
            if range.location == NOT_FOUND || range.location > total {
                return Some(std::ptr::null_mut());
            }
            let a = super::utf16_to_chars(&text, range.location);
            let b = super::utf16_to_chars(
                &text,
                range.location.saturating_add(range.length).min(total),
            );
            let sub: String = text.chars().skip(a).take(b - a).collect();
            if !actual.is_null() {
                let lo = super::chars_to_utf16(&text, a);
                // SAFETY: AppKit's out pointer.
                unsafe {
                    *actual = Range {
                        location: lo,
                        length: super::chars_to_utf16(&text, b) - lo,
                    };
                }
            }
            let cls = AnyClass::get(c"NSAttributedString")?;
            let ns = NSString::from_str(&sub);
            // SAFETY: a new NSAttributedString, autoreleased as AppKit's
            // getter convention expects.
            unsafe {
                let alloc: Allocated<AnyObject> = msg_send![cls, alloc];
                let s: Retained<AnyObject> = msg_send![alloc, initWithString: &*ns];
                Some(Retained::autorelease_return(s))
            }
        })
        .flatten();
        if let Some(r) = mine {
            return r;
        }
        match old("attributedSubstringForProposedRange:actualRange:") {
            // SAFETY: winit's method.
            Some(imp) => unsafe {
                std::mem::transmute::<
                    Imp,
                    unsafe extern "C-unwind" fn(
                        *mut AnyObject,
                        Sel,
                        Range,
                        *mut Range,
                    ) -> *mut AnyObject,
                >(imp)(this, cmd, range, actual)
            },
            None => std::ptr::null_mut(),
        }
    }

    /// The view's window.
    unsafe fn window_of(this: *mut AnyObject) -> Option<Retained<AnyObject>> {
        // SAFETY: winit's view.
        unsafe { msg_send![&*this, window] }
    }

    unsafe extern "C-unwind" fn character_index(
        this: *mut AnyObject,
        cmd: Sel,
        point: Point,
    ) -> usize {
        let mine = contain_callback("character_index", || {
            let (_, st, text, _, _) = unsafe { field_text(this) }?;
            let win = unsafe { window_of(this) }?;
            // screen → window → the view's (flipped: logical pixels)
            let none: *const AnyObject = std::ptr::null();
            let p: Point = unsafe { msg_send![&*win, convertPointFromScreen: point] };
            let p: Point = unsafe { msg_send![&*this, convertPoint: p, fromView: none] };
            let i = st.lock().ok()?.text_input_index(p.x as f32, p.y as f32)?;
            Some(super::chars_to_utf16(&text, i))
        })
        .flatten();
        if let Some(i) = mine {
            return i;
        }
        match old("characterIndexForPoint:") {
            // SAFETY: winit's method.
            Some(imp) => unsafe {
                std::mem::transmute::<
                    Imp,
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, Point) -> usize,
                >(imp)(this, cmd, point)
            },
            None => NOT_FOUND,
        }
    }

    unsafe extern "C-unwind" fn first_rect(
        this: *mut AnyObject,
        cmd: Sel,
        range: Range,
        actual: *mut Range,
    ) -> Rect {
        let mine = contain_callback("first_rect", || {
            let (_, st, text, _, _) = unsafe { field_text(this) }?;
            if range.location == NOT_FOUND {
                return None;
            }
            let a = super::utf16_to_chars(&text, range.location);
            let b = super::utf16_to_chars(&text, range.location.saturating_add(range.length));
            let r = st.lock().ok()?.text_input_rect(a, b)?;
            if !actual.is_null() {
                let lo = super::chars_to_utf16(&text, a);
                // SAFETY: AppKit's out pointer.
                unsafe {
                    *actual = Range {
                        location: lo,
                        length: super::chars_to_utf16(&text, b) - lo,
                    };
                }
            }
            let win = unsafe { window_of(this) }?;
            let none: *const AnyObject = std::ptr::null();
            let local = Rect {
                origin: Point {
                    x: r[0] as f64,
                    y: r[1] as f64,
                },
                size: Point {
                    x: r[2] as f64,
                    y: r[3] as f64,
                },
            };
            // the view's (flipped) → the window's → the screen's
            let inw: Rect = unsafe { msg_send![&*this, convertRect: local, toView: none] };
            let on: Rect = unsafe { msg_send![&*win, convertRectToScreen: inw] };
            Some(on)
        })
        .flatten();
        if let Some(r) = mine {
            return r;
        }
        match old("firstRectForCharacterRange:actualRange:") {
            // SAFETY: winit's method.
            Some(imp) => unsafe {
                std::mem::transmute::<
                    Imp,
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, Range, *mut Range) -> Rect,
                >(imp)(this, cmd, range, actual)
            },
            None => Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Point { x: 0.0, y: 0.0 },
            },
        }
    }

    /// `quickLookWithEvent:` — a force click (or a three-finger tap) on a
    /// word in a field looks it up, as a Mac's text views do.
    unsafe extern "C-unwind" fn quick_look(this: *mut AnyObject, _cmd: Sel, event: *mut AnyObject) {
        contain_callback("quick_look", || {
            if event.is_null() {
                return None;
            }
            let (_, st) = state_of(this)?;
            let none: *const AnyObject = std::ptr::null();
            let p: Point = unsafe { msg_send![&*event, locationInWindow] };
            let p: Point = unsafe { msg_send![&*this, convertPoint: p, fromView: none] };
            let (word, r, size) = st.lock().ok()?.word_at(p.x as f32, p.y as f32)?;
            unsafe { show_definition(this, &word, r, size) };
            Some(())
        });
    }

    /// Show the system's definition of `word`, set in `size` points, whose
    /// box in the view is `r`.
    unsafe fn show_definition(view: *mut AnyObject, word: &str, r: [f32; 4], size: f32) {
        let (Some(astr), Some(font), Some(dict)) = (
            AnyClass::get(c"NSAttributedString"),
            AnyClass::get(c"NSFont"),
            AnyClass::get(c"NSDictionary"),
        ) else {
            return;
        };
        // SAFETY: AppKit classes; every object made here is retained
        // until the call returns.
        unsafe {
            let f: Retained<AnyObject> = msg_send![font, systemFontOfSize: size as f64];
            let key = NSString::from_str("NSFont");
            let attrs: Retained<AnyObject> =
                msg_send![dict, dictionaryWithObject: &*f, forKey: &*key];
            let ns = NSString::from_str(word);
            let alloc: Allocated<AnyObject> = msg_send![astr, alloc];
            let s: Retained<AnyObject> =
                msg_send![alloc, initWithString: &*ns, attributes: &*attrs];
            // the baseline's start: the box's bottom less the font's descent
            let descender: f64 = msg_send![&*f, descender];
            let at = Point {
                x: r[0] as f64,
                y: (r[1] + r[3]) as f64 + descender,
            };
            let _: () = msg_send![&*view, showDefinitionForAttributedString: &*s, atPoint: at];
        }
    }

    /// The misspelled words of `text` (characters), by AppKit's shared
    /// spell checker in the person's languages.
    fn spell_check(text: &str) -> Vec<(usize, usize)> {
        contain_callback("spell_check", || {
            let cls = AnyClass::get(c"NSSpellChecker")?;
            let ns = NSString::from_str(text);
            let total = super::chars_to_utf16(text, text.chars().count());
            let mut out = Vec::new();
            // SAFETY: AppKit's shared checker; the string lives for the
            // call; the answer is an NSArray of NSTextCheckingResults.
            unsafe {
                let checker: Retained<AnyObject> = msg_send![cls, sharedSpellChecker];
                let none: *const AnyObject = std::ptr::null();
                let words: *mut u8 = std::ptr::null_mut();
                // spelling (NSTextCheckingTypeSpelling), the language
                // found from the text (a word-by-word check of a short
                // text guesses none)
                let results: Option<Retained<AnyObject>> = msg_send![&*checker,
                    checkString: &*ns,
                    range: Range { location: 0, length: total },
                    types: 2u64,
                    options: none,
                    inSpellDocumentWithTag: 0isize,
                    orthography: none,
                    wordCount: words as *mut isize];
                let Some(results) = results else {
                    return Some(out);
                };
                let n: usize = msg_send![&*results, count];
                for i in 0..n.min(10_000) {
                    let r: Retained<AnyObject> = msg_send![&*results, objectAtIndex: i];
                    let range: Range = msg_send![&*r, range];
                    if range.location == NOT_FOUND || range.length == 0 {
                        continue;
                    }
                    out.push((
                        super::utf16_to_chars(text, range.location),
                        super::utf16_to_chars(text, range.location + range.length),
                    ));
                }
            }
            Some(out)
        })
        .flatten()
        .unwrap_or_default()
    }

    /// Replace winit's view's text input methods with the ones above
    /// (each calling winit's where it should), once.
    unsafe fn text_input_client(cls: *mut AnyClass) {
        let mut olds = HashMap::new();
        let swap =
            |olds: &mut HashMap<&'static str, usize>, name: &'static str, sel: Sel, imp: Imp| {
                // SAFETY: winit's class and a selector it implements; the new
                // IMP has the method's signature.
                unsafe {
                    let m = objc2::ffi::class_getInstanceMethod(cls, sel);
                    if m.is_null() {
                        return;
                    }
                    if let Some(o) = objc2::ffi::method_setImplementation(m, imp) {
                        olds.insert(name, o as usize);
                    }
                }
            };
        // SAFETY: the transmutes only erase each function's signature
        // into an IMP; the runtime calls it with that signature.
        unsafe {
            swap(
                &mut olds,
                "keyDown:",
                objc2::sel!(keyDown:),
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                    Imp,
                >(key_down),
            );
            swap(
                &mut olds,
                "insertText:replacementRange:",
                objc2::sel!(insertText:replacementRange:),
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Range),
                    Imp,
                >(insert_text),
            );
            swap(
                &mut olds,
                "selectedRange",
                objc2::sel!(selectedRange),
                std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> Range, Imp>(
                    selected_range,
                ),
            );
            swap(
                &mut olds,
                "attributedSubstringForProposedRange:actualRange:",
                objc2::sel!(attributedSubstringForProposedRange:actualRange:),
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(
                        *mut AnyObject,
                        Sel,
                        Range,
                        *mut Range,
                    ) -> *mut AnyObject,
                    Imp,
                >(attributed_substring),
            );
            swap(
                &mut olds,
                "characterIndexForPoint:",
                objc2::sel!(characterIndexForPoint:),
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, Point) -> usize,
                    Imp,
                >(character_index),
            );
            swap(
                &mut olds,
                "firstRectForCharacterRange:actualRange:",
                objc2::sel!(firstRectForCharacterRange:actualRange:),
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, Range, *mut Range) -> Rect,
                    Imp,
                >(first_rect),
            );
            let ql: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = quick_look;
            objc2::ffi::class_addMethod(
                cls,
                objc2::sel!(quickLookWithEvent:),
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                    Imp,
                >(ql),
                c"v@:@".as_ptr(),
            );
        }
        let _ = OLD.set(Mutex::new(olds));
    }

    /// Teach winit's view the Edit menu's selectors, `validateMenuItem:`
    /// and the Services protocol, once; and tell AppKit the application
    /// sends and takes text for Services.
    fn responder() {
        static DONE: OnceLock<()> = OnceLock::new();
        DONE.get_or_init(|| {
            let Some(cls) = AnyClass::get(c"WinitView") else {
                return;
            };
            let cls = cls as *const AnyClass as *mut AnyClass;
            let bool_enc = if cfg!(target_arch = "aarch64") { "B" } else { "c" };
            let v3 = c"v@:@";
            let validate_enc = std::ffi::CString::new(format!("{bool_enc}@:@")).unwrap_or_default();
            let write_enc = std::ffi::CString::new(format!("{bool_enc}@:@@")).unwrap_or_default();
            // SAFETY: winit's view class, live for the process; each method
            // added once, with the signature AppKit calls it with.
            unsafe {
                let act: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = edit_action;
                for role in EDIT_ROLES {
                    objc2::ffi::class_addMethod(
                        cls,
                        Sel::register(item(role).1),
                        std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject), Imp>(act),
                        v3.as_ptr(),
                    );
                }
                let val: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool = validate;
                objc2::ffi::class_addMethod(
                    cls,
                    objc2::sel!(validateMenuItem:),
                    std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool, Imp>(val),
                    validate_enc.as_ptr(),
                );
                let req: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> *mut AnyObject =
                    services_requestor;
                objc2::ffi::class_addMethod(
                    cls,
                    objc2::sel!(validRequestorForSendType:returnType:),
                    std::mem::transmute::<
                        unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> *mut AnyObject,
                        Imp,
                    >(req),
                    c"@@:@@".as_ptr(),
                );
                let wr: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> Bool = services_write;
                objc2::ffi::class_addMethod(
                    cls,
                    objc2::sel!(writeSelectionToPasteboard:types:),
                    std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> Bool, Imp>(wr),
                    write_enc.as_ptr(),
                );
                let rd: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool = services_read;
                objc2::ffi::class_addMethod(
                    cls,
                    objc2::sel!(readSelectionFromPasteboard:),
                    std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool, Imp>(rd),
                    validate_enc.as_ptr(),
                );
                text_input_client(cls);
                crate::stdlib::gui::edit::set_spell_checker(Some(spell_check));
                // the types this application sends and takes for Services
                if let Some(app) = AnyClass::get(c"NSApplication") {
                    let app: Retained<AnyObject> = msg_send![app, sharedApplication];
                    let t = NSString::from_str("public.utf8-plain-text");
                    let types = objc2_foundation::NSArray::from_retained_slice(&[t]);
                    let _: () = msg_send![&*app, registerServicesMenuSendTypes: &*types, returnTypes: &*types];
                }
            }
        });
    }
}

/// Frames and the display (macOS): each window's display link (its
/// screen's refreshes: CADisplayLink from the view, macOS 14, ProMotion's
/// rate where the screen has it), and input-to-photon measured
/// (`GUI_LATENCY=1`): for the first frame presented after the program
/// answered input (its `gui.apply`), the time from that input event to
/// the frame's present call, and to the refresh that shows it (frames
/// show in turn, one a refresh). Said as percentiles on stderr as the loop
/// ends. `GUI_PACING=0` presents every frame at once, as before (to
/// compare).
#[cfg(target_os = "macos")]
#[allow(deprecated)] // libc's mach clock (the mach2 crate is not a dependency)
pub mod frames {
    use super::{Cmd, send};
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    /// Whether frames are put off while one is on its way (default on).
    pub fn pacing() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var("GUI_PACING").map_or(true, |v| v != "0"))
    }

    /// A refresh rate to stand in for the display's (`GUI_REFRESH_HZ`, a
    /// measurement's: the display asleep has no refreshes): every window
    /// hears a refresh that often.
    pub fn synthetic_hz() -> Option<f64> {
        static HZ: OnceLock<Option<f64>> = OnceLock::new();
        *HZ.get_or_init(|| {
            let hz = std::env::var("GUI_REFRESH_HZ").ok()?.parse::<f64>().ok()?;
            if !(10.0..=1000.0).contains(&hz) {
                return None;
            }
            let period = std::time::Duration::from_secs_f64(1.0 / hz);
            std::thread::Builder::new()
                .name("gui-refresh".into())
                .spawn(move || {
                    let mut next = std::time::Instant::now() + period;
                    loop {
                        if let Some(d) = next.checked_duration_since(std::time::Instant::now()) {
                            std::thread::sleep(d);
                        }
                        next += period;
                        send(Cmd::Tick {
                            id: 0,
                            at: now_ns(),
                        });
                    }
                })
                .ok()?;
            Some(hz)
        })
    }

    /// Whether input-to-photon is measured.
    pub fn measuring() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("GUI_LATENCY").is_some())
    }

    /// Now, in nanoseconds of the host clock (CACurrentMediaTime's, which
    /// the display link's timestamps count).
    pub fn now_ns() -> u64 {
        static TB: OnceLock<(u64, u64)> = OnceLock::new();
        let (n, d) = *TB.get_or_init(|| {
            let mut i = libc::mach_timebase_info { numer: 1, denom: 1 };
            // SAFETY: a struct of the declared type to fill.
            unsafe { libc::mach_timebase_info(&mut i) };
            (i.numer as u64, i.denom.max(1) as u64)
        });
        // SAFETY: no arguments; the host clock.
        let t = unsafe { libc::mach_absolute_time() };
        (t as u128 * n as u128 / d as u128) as u64
    }

    /// Each display link (its address) to its window.
    fn links() -> &'static Mutex<HashMap<usize, u64>> {
        static L: OnceLock<Mutex<HashMap<usize, u64>>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// `tick:` — a refresh: the loop hears it as [`Cmd::Tick`].
    unsafe extern "C-unwind" fn tick(_this: *mut AnyObject, _cmd: Sel, link: *mut AnyObject) {
        super::contain_callback("display_link", || {
            if link.is_null() {
                return;
            }
            let Some(id) = links()
                .lock()
                .ok()
                .and_then(|l| l.get(&(link as usize)).copied())
            else {
                return;
            };
            // SAFETY: the CADisplayLink calling, alive for the call.
            let at: f64 = unsafe { msg_send![&*link, timestamp] };
            if trace_ticks() {
                eprintln!("gui: window {id} tick at {at:.4}");
            }
            send(Cmd::Tick {
                id,
                at: (at * 1e9) as u64,
            });
        });
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        /// `NSRunLoopCommonModes`: a CFStringRef.
        static kCFRunLoopCommonModes: *const std::ffi::c_void;
    }

    /// CAFrameRateRange.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FrameRateRange {
        minimum: f32,
        maximum: f32,
        preferred: f32,
    }

    // SAFETY: the layout of CoreAnimation's CAFrameRateRange (three floats).
    unsafe impl objc2::Encode for FrameRateRange {
        const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
            "CAFrameRateRange",
            &[
                <f32 as objc2::Encode>::ENCODING,
                <f32 as objc2::Encode>::ENCODING,
                <f32 as objc2::Encode>::ENCODING,
            ],
        );
    }

    /// The display links' target: one object of a class of its own.
    fn target() -> Option<&'static AnyObject> {
        struct Target(Retained<AnyObject>);
        // SAFETY: made and used on the main thread only.
        unsafe impl Send for Target {}
        unsafe impl Sync for Target {}
        static T: OnceLock<Option<Target>> = OnceLock::new();
        T.get_or_init(|| {
            let sup = AnyClass::get(c"NSObject")?;
            let cls = match ClassBuilder::new(c"OlangFrameTicker", sup) {
                Some(mut b) => {
                    // SAFETY: `tick:` takes the link, as CADisplayLink calls it.
                    unsafe {
                        b.add_method(
                            objc2::sel!(tick:),
                            tick as unsafe extern "C-unwind" fn(
                                *mut AnyObject,
                                Sel,
                                *mut AnyObject,
                            ),
                        )
                    };
                    b.register()
                }
                None => AnyClass::get(c"OlangFrameTicker")?,
            };
            // SAFETY: a plain NSObject subclass.
            let obj: Retained<AnyObject> = unsafe { msg_send![cls, new] };
            Some(Target(obj))
        })
        .as_ref()
        .map(|t| &*t.0)
    }

    /// A display link for `win`'s view, on the main run loop, paused until
    /// a frame is on its way. `None` before macOS 14 (frames go out at
    /// once, as they always did).
    pub fn link_for(win: &winit::window::Window, id: u64) -> Option<Retained<AnyObject>> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let RawWindowHandle::AppKit(h) = win.window_handle().ok()?.as_raw() else {
            return None;
        };
        let view = h.ns_view.as_ptr() as *mut AnyObject;
        let target = target()?;
        // SAFETY: winit's live view, on the main thread; the link is
        // invalidated as the window closes (`unlink`).
        unsafe {
            let sel = objc2::sel!(displayLinkWithTarget:selector:);
            let can: bool = msg_send![&*view, respondsToSelector: sel];
            if !can {
                return None;
            }
            let link: Option<Retained<AnyObject>> =
                msg_send![&*view, displayLinkWithTarget: target, selector: objc2::sel!(tick:)];
            let link = link?;
            // as fast as the screen refreshes (ProMotion's 120 Hz): a link
            // left at its default may tick at 60 while the layer shows 120
            let fps: isize = {
                let w: Option<Retained<AnyObject>> = msg_send![&*view, window];
                let sc: Option<Retained<AnyObject>> = w.and_then(|w| msg_send![&*w, screen]);
                sc.map_or(60, |sc| msg_send![&*sc, maximumFramesPerSecond])
            };
            // `GUI_LINK_HZ` (a measurement's): ask the screen for no more
            // than that (60 on a ProMotion screen: its 60 Hz pacing)
            let cap = std::env::var("GUI_LINK_HZ")
                .ok()
                .and_then(|v| v.parse::<isize>().ok())
                .filter(|v| (10..=240).contains(v));
            let most = cap.map_or(fps.max(60), |c| c.min(fps.max(60))) as f32;
            let range = FrameRateRange {
                minimum: most.min(60.0),
                maximum: most,
                preferred: most,
            };
            let _: () = msg_send![&*link, setPreferredFrameRateRange: range];
            let _: () = msg_send![&*link, setPaused: true];
            let rl: Retained<AnyObject> = msg_send![objc2::class!(NSRunLoop), mainRunLoop];
            // the common modes' own constant: CFRunLoop knows it by its
            // address, and a string equal to it names a mode of its own
            // that the loop never runs (the link then never ticked)
            let mode = kCFRunLoopCommonModes as *const AnyObject;
            let _: () = msg_send![&*link, addToRunLoop: &*rl, forMode: &*mode];
            if trace_ticks() {
                eprintln!(
                    "gui: window {id} display link {:p} at {most} Hz",
                    Retained::as_ptr(&link)
                );
            }
            links()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(Retained::as_ptr(&link) as usize, id);
            Some(link)
        }
    }

    /// `GUI_TRACE_TICKS`: every refresh and every resume said.
    fn trace_ticks() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("GUI_TRACE_TICKS").is_some())
    }

    /// The link ticks (a frame is on its way, or one is put off).
    pub fn resume(link: &Option<Retained<AnyObject>>) {
        if let Some(l) = link {
            // SAFETY: a live CADisplayLink, on the main thread.
            unsafe {
                let paused: bool = msg_send![&**l, isPaused];
                if trace_ticks() {
                    eprintln!("gui: link resumed (was paused: {paused})");
                }
                if paused {
                    let _: () = msg_send![&**l, setPaused: false];
                }
            }
        }
    }

    /// The link rests (nothing on its way).
    pub fn pause(link: &Option<Retained<AnyObject>>) {
        if let Some(l) = link {
            // SAFETY: as above.
            unsafe { msg_send![&**l, setPaused: true] }
        }
    }

    /// A window closed: its link stopped and let go.
    pub fn unlink(link: Option<Retained<AnyObject>>) {
        if let Some(l) = link {
            links()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&(Retained::as_ptr(&l) as usize));
            // SAFETY: as above.
            unsafe { msg_send![&*l, invalidate] }
        }
    }

    // ── input to photon ──

    /// Inputs a window took, waiting for the program's answer (its next
    /// `gui.apply`): the first since its last answer counts.
    fn waiting() -> &'static Mutex<HashMap<u64, u64>> {
        static W: OnceLock<Mutex<HashMap<u64, u64>>> = OnceLock::new();
        W.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Answers waiting for their frame.
    fn pending() -> &'static Mutex<HashMap<u64, u64>> {
        static P: OnceLock<Mutex<HashMap<u64, u64>>> = OnceLock::new();
        P.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// `(present, shown)` ms from input, a frame each.
    pub fn samples() -> &'static Mutex<Vec<(f64, f64)>> {
        static S: OnceLock<Mutex<Vec<(f64, f64)>>> = OnceLock::new();
        S.get_or_init(|| Mutex::new(Vec::new()))
    }

    /// Window `id` took input now.
    pub fn input(id: u64) {
        if let Ok(mut w) = waiting().lock() {
            w.entry(id).or_insert_with(now_ns);
        }
    }

    /// The program applied a patch to window `id`: the input it answers
    /// is timed to the next frame presented.
    pub fn applied(id: u64) {
        if !measuring() {
            return;
        }
        let Some(t0) = waiting().lock().ok().and_then(|mut w| w.remove(&id)) else {
            return;
        };
        if let Ok(mut p) = pending().lock() {
            p.entry(id).or_insert(t0);
        }
    }

    /// A frame of window `id` presented: the input it answers, if timed.
    pub fn answered(id: u64) -> Option<u64> {
        if !measuring() {
            return None;
        }
        pending().lock().ok().and_then(|mut p| p.remove(&id))
    }

    /// A frame presented at `presented` showed at the refresh `at`.
    pub fn shown(presented: u64, input: Option<u64>, at: u64) {
        let Some(t0) = input else {
            return;
        };
        let ms = |t: u64| t.saturating_sub(t0) as f64 / 1e6;
        if let Ok(mut s) = samples().lock() {
            s.push((ms(presented), ms(at)));
        }
    }

    /// The percentiles said on stderr.
    pub fn report() {
        let s = samples().lock().map(|s| s.clone()).unwrap_or_default();
        if s.is_empty() {
            return;
        }
        let pick = |f: fn(&(f64, f64)) -> f64| {
            let mut v: Vec<f64> = s.iter().map(f).collect();
            v.sort_by(|a, b| a.total_cmp(b));
            let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
            format!("p50 {:.2} p90 {:.2} max {:.2}", at(0.5), at(0.9), at(1.0))
        };
        eprintln!(
            "gui latency: {} answers to input shown{}: to present {}; to the screen {}",
            s.len(),
            if pacing() { "" } else { " (GUI_PACING=0)" },
            pick(|x| x.0),
            pick(|x| x.1)
        );
    }
}

/// The program's turns, watched (`gui.turn`): a turn that runs past a
/// limit (2 s, as long as macOS waits before it shows an application is
/// not responding; `GUI_NOT_RESPONDING_MS`) makes every window show it is
/// waiting until the turn ends. The engine goes on drawing meanwhile: the
/// loop's thread is not the program's, so scrolls, looks, fields and the
/// spinner keep the refresh while the turn runs. A turn costs two atomic
/// writes; the watch's thread sleeps while no turn runs.
pub mod turn_watch {
    use super::{Cmd, send};
    use crate::stdlib::gui::window::{Clip, DisplayList, Prim};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    /// When the running turn began (µs since `epoch`, plus one), or 0.
    static STARTED: AtomicU64 = AtomicU64::new(0);
    /// The windows say the program is not responding.
    static SHOWN: AtomicBool = AtomicBool::new(false);
    /// The watch's thread is asleep with no turn to time.
    static IDLE: AtomicBool = AtomicBool::new(false);

    fn epoch() -> Instant {
        static E: OnceLock<Instant> = OnceLock::new();
        *E.get_or_init(Instant::now)
    }

    fn now_us() -> u64 {
        epoch().elapsed().as_micros() as u64 + 1
    }

    /// How long a turn may run before the windows say so.
    pub fn limit() -> Duration {
        static L: OnceLock<Duration> = OnceLock::new();
        *L.get_or_init(|| {
            let ms = std::env::var("GUI_NOT_RESPONDING_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2000);
            Duration::from_millis(ms.clamp(50, 600_000))
        })
    }

    fn watcher() -> &'static std::thread::Thread {
        static T: OnceLock<std::thread::Thread> = OnceLock::new();
        T.get_or_init(|| {
            std::thread::Builder::new()
                .name("gui-turn-watch".into())
                .spawn(watch)
                .expect("spawning the turn watch")
                .thread()
                .clone()
        })
    }

    fn watch() {
        loop {
            let started = STARTED.load(Ordering::Acquire);
            if started == 0 {
                IDLE.store(true, Ordering::Release);
                // a turn that began as the flag went up is seen at once
                if STARTED.load(Ordering::Acquire) == 0 {
                    std::thread::park();
                }
                IDLE.store(false, Ordering::Release);
                continue;
            }
            let ran = Duration::from_micros(now_us().saturating_sub(started));
            if ran >= limit() {
                if !SHOWN.swap(true, Ordering::AcqRel) {
                    send(Cmd::NotResponding(true));
                }
                // until this turn ends (`turn(false)` takes the windows back)
                std::thread::park_timeout(Duration::from_millis(500));
            } else {
                std::thread::park_timeout(limit() - ran);
            }
        }
    }

    /// The program's turn begins (`on`) or ends.
    pub fn turn(on: bool) {
        if on {
            let _ = epoch();
            STARTED.store(now_us(), Ordering::Release);
            if IDLE.swap(false, Ordering::AcqRel) || !WATCHING.swap(true, Ordering::AcqRel) {
                watcher().unpark();
            }
        } else {
            STARTED.store(0, Ordering::Release);
            if SHOWN.swap(false, Ordering::AcqRel) {
                send(Cmd::NotResponding(false));
            }
        }
    }

    /// The watch's thread was started.
    static WATCHING: AtomicBool = AtomicBool::new(false);

    /// Whether the windows say the program is not responding.
    pub fn shown() -> bool {
        SHOWN.load(Ordering::Acquire)
    }

    /// The veil and the spinner over a frame `dl` (device pixels at
    /// `scale`): eight dots round the middle, the brightest turning a dot
    /// every 80 ms. Answers when it next turns.
    pub fn overlay(dl: &mut DisplayList, scale: f32) -> Option<Instant> {
        let (w, h) = (dl.width as f32, dl.height as f32);
        let all = Clip::rect(0.0, 0.0, w, h);
        let rect = |x: f32, y: f32, rw: f32, rh: f32, fill: [u8; 4], r: f32| Prim::Rect {
            x,
            y,
            w: rw,
            h: rh,
            fill,
            border: [0, 0, 0, 0],
            border_width: 0.0,
            radii: [r; 4],
            continuous: false,
            clip: all,
        };
        dl.prims.push(rect(0.0, 0.0, w, h, [0, 0, 0, 40], 0.0));
        let s = scale.max(0.5);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let card = 64.0 * s;
        dl.prims.push(rect(
            cx - card / 2.0,
            cy - card / 2.0,
            card,
            card,
            [30, 30, 32, 200],
            14.0 * s,
        ));
        let step = (epoch().elapsed().as_millis() / 80) as usize;
        let dot = 6.0 * s;
        let ring = 16.0 * s;
        for i in 0..8 {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            let age = (step + 8 - i) % 8;
            let alpha = (255 - age * 28).max(50) as u8;
            let (x, y) = (cx + ring * a.sin(), cy - ring * a.cos());
            dl.prims.push(rect(
                x - dot / 2.0,
                y - dot / 2.0,
                dot,
                dot,
                [235, 235, 240, alpha],
                dot / 2.0,
            ));
        }
        Some(Instant::now() + Duration::from_millis(80))
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

/// An external quit held for the program: the Dock's Quit, AppleScript's
/// `quit app`, a logout, a restart or a shutdown (AppKit's
/// `applicationShouldTerminate:`). winit only says the loop is ending
/// (`applicationWillTerminate:`) and the process ended with nothing of
/// the program's quit run — no session saved, no children stopped. A
/// program that asks (`gui.hold_quit(true)`, Loom's run loop) hears the
/// request as a `quit` event (`why`: "quit", "logout", "restart" or
/// "shutdown") and the platform waits (NSTerminateLater) until the
/// program ends; its end replies, and the process exits with its code.
pub mod quit_hold {
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

    static HELD: AtomicBool = AtomicBool::new(false);
    static ASKED: AtomicBool = AtomicBool::new(false);
    static CODE: AtomicI32 = AtomicI32::new(0);
    static DELEGATE: AtomicUsize = AtomicUsize::new(0);

    /// The program holds external quits (`true`) or lets them end the
    /// application at once (`false`, the default).
    pub fn hold(on: bool) {
        HELD.store(on, Ordering::SeqCst);
    }

    /// Whether an external quit is waiting for the program to end.
    pub fn asked() -> bool {
        ASKED.load(Ordering::SeqCst)
    }

    pub(super) fn set_delegate(ptr: usize) {
        DELEGATE.store(ptr, Ordering::SeqCst);
    }

    /// The platform asks to end the application: `true` when the request
    /// is the program's to finish (it heard `quit`), `false` to end now.
    pub fn ask(why: &str) -> bool {
        if !HELD.load(Ordering::SeqCst) {
            return false;
        }
        // asked again while the program finishes: still waiting
        if !ASKED.swap(true, Ordering::SeqCst) {
            super::emit(vec![super::event("quit", vec![("why", super::s(why))])]);
        }
        true
    }

    /// The program ended: when it was finishing an external quit, reply
    /// to the platform (which then ends the process — `exiting`) and
    /// answer `true`; a reply that never comes ends the process anyway.
    pub(super) fn finish(code: i32) -> bool {
        if !asked() {
            return false;
        }
        CODE.store(code, Ordering::SeqCst);
        #[cfg(target_os = "macos")]
        {
            let delegate = DELEGATE.load(Ordering::SeqCst);
            if delegate != 0 {
                // SAFETY: the application's delegate, alive for the
                // process; performSelectorOnMainThread is callable from
                // any thread, and runs in the common modes — the modal
                // panel mode AppKit waits for the reply in among them.
                unsafe {
                    let obj = delegate as *mut objc2::runtime::AnyObject;
                    let nil: *mut objc2::runtime::AnyObject = std::ptr::null_mut();
                    let _: () = objc2::msg_send![&*obj,
                        performSelectorOnMainThread: objc2::sel!(olangReplyTerminate:),
                        withObject: nil,
                        waitUntilDone: false];
                }
            }
        }
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            exiting();
        });
        true
    }

    /// The process ends with the program's code, its output flushed.
    pub(super) fn exiting() {
        if asked() {
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            std::process::exit(CODE.load(Ordering::SeqCst));
        }
    }

    /// Why the application is asked to end: the quit Apple event's
    /// reason (`kAEQuitReason`), "quit" when it gives none.
    #[cfg(target_os = "macos")]
    fn reason() -> &'static str {
        // SAFETY: AppKit's shared event manager and the event it is
        // handling, read on the main thread inside the delegate call.
        unsafe {
            let Some(cls) = objc2::runtime::AnyClass::get(c"NSAppleEventManager") else {
                return "quit";
            };
            let mgr: *mut objc2::runtime::AnyObject =
                objc2::msg_send![cls, sharedAppleEventManager];
            if mgr.is_null() {
                return "quit";
            }
            let ev: *mut objc2::runtime::AnyObject = objc2::msg_send![&*mgr, currentAppleEvent];
            if ev.is_null() {
                return "quit";
            }
            let why: *mut objc2::runtime::AnyObject =
                objc2::msg_send![&*ev, attributeDescriptorForKeyword: u32::from_be_bytes(*b"why?")];
            if why.is_null() {
                return "quit";
            }
            let code: u32 = objc2::msg_send![&*why, enumCodeValue];
            match &code.to_be_bytes() {
                b"logo" | b"rlgo" => "logout",
                b"rrst" | b"rest" => "restart",
                b"rsdn" | b"shut" => "shutdown",
                _ => "quit",
            }
        }
    }

    /// `applicationShouldTerminate:` — NSTerminateLater (2) when the
    /// program finishes the quit, NSTerminateNow (1) otherwise.
    #[cfg(target_os = "macos")]
    pub(super) unsafe extern "C-unwind" fn app_should_terminate(
        _this: *mut objc2::runtime::AnyObject,
        _cmd: objc2::runtime::Sel,
        _app: *mut objc2::runtime::AnyObject,
    ) -> usize {
        super::contain_callback("should_terminate", || if ask(reason()) { 2 } else { 1 })
            .unwrap_or(1)
    }

    /// The reply, on the main thread: the application may end.
    #[cfg(target_os = "macos")]
    pub(super) unsafe extern "C-unwind" fn app_reply_terminate(
        _this: *mut objc2::runtime::AnyObject,
        _cmd: objc2::runtime::Sel,
        _arg: *mut objc2::runtime::AnyObject,
    ) {
        super::contain_callback("reply_terminate", || {
            if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
                let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
                app.replyToApplicationShouldTerminate(true);
            }
        });
    }
}

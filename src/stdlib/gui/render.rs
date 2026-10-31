//! The render thread (macOS): one per process, it draws and presents every
//! GPU window's frames, so the main thread never waits on the GPU — a
//! drawable (`nextDrawable` blocks while the compositor holds them), the
//! encode, the submit and present — and a slow frame never holds up
//! input.
//!
//! **The hand-over.** The main thread keeps the scene and the input state
//! behind each window's lock; after each change it lists a frame (the
//! display list: what the window shows, already shaped) and publishes it
//! whole, immutable, as an `Arc<Frame>` in the window's [`Slot`]. The slot
//! is a double buffer: a frame not yet taken is replaced by a newer one,
//! and the renderer always takes the newest and never waits for one.
//! Patches and the scene never reach this thread.
//!
//! **What it owns.** Each window's surface (made on the main thread, which
//! the window's handle needs, then moved here: configured and presented
//! here), the kept frame and its damage (inside the surface), the display
//! link (scheduled on this thread's run loop: its ticks arrive here even
//! while the main thread is busy), the pacing (`in_flight`: one frame on
//! its way a refresh, a frame put off held in the slot until the next
//! tick), and device-loss recovery (the surface remakes its device and
//! draws its kept frame whole). The glyph atlas is filled, and its pages
//! emptied, only as a frame is prepared here, between frames.
//!
//! **What goes back.** Something moving (a transition, a scroll's
//! momentum) asks the main thread for the next frame at the next refresh
//! ([`Cmd::NextFrame`]); a frame of a new size is waited for by the main
//! thread for at most a refresh (a live resize presents the size it
//! drew). Every callback here runs contained: a panic is a `problem`, and
//! every window is drawn again whole.
//!
//! `GUI_RENDER_THREAD=0` draws on the main thread as before (a
//! measurement's comparison). The software renderer and headless windows
//! stay on their own threads; both renderers draw the same display lists.

use super::gpu;
use super::platform::{Cmd, contain_callback, count_frame, frames, send};
use super::window::DisplayList;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A window's frame as the main thread listed it.
pub struct Frame {
    pub dl: DisplayList,
    /// Something in it moves at the next refresh: the main thread is asked
    /// for the next frame then.
    pub animating: bool,
    /// The input it is the first answer to (`GUI_LATENCY`).
    pub covers: Option<u64>,
    /// When the main thread began making it (`GUI_TRACE`).
    pub started: Instant,
}

#[derive(Default)]
struct SlotState {
    /// The newest frame not yet taken.
    newest: Option<Arc<Frame>>,
    /// Frames published, and the last one handled (presented, found to
    /// change nothing, or refused by the surface).
    published: u64,
    handled: u64,
    /// The size last presented.
    shown: (u32, u32),
}

/// One window's hand-over.
#[derive(Default)]
pub struct Slot {
    state: Mutex<SlotState>,
    handled: Condvar,
    /// The main thread put a frame off while one waited here: it is asked
    /// for it as the render thread can take one.
    wanted: std::sync::atomic::AtomicBool,
    /// Frames on their way to the screen (the render thread's count, read
    /// by the main thread).
    on_way: std::sync::atomic::AtomicUsize,
}

impl Slot {
    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A display link handed to this thread (made on the main thread from the
/// window's view; scheduled, paused, resumed and invalidated here only).
struct Link(Retained<AnyObject>);

// SAFETY: the link is used on the render thread only, once handed over.
unsafe impl Send for Link {}

/// The main thread's handle on a window drawn here.
pub struct Handle {
    id: u64,
    slot: Arc<Slot>,
    /// The window has a display link (its refreshes pace it).
    pub linked: bool,
}

impl Handle {
    /// Publish a frame: the render thread draws it as soon as the pacing
    /// lets it. A frame of another size than the one shown is waited for,
    /// at most a refresh, so a live resize presents what it drew.
    pub fn publish(&self, frame: Frame) {
        let size = (frame.dl.width, frame.dl.height);
        let (seq, resized) = {
            let mut s = self.slot.lock();
            s.published += 1;
            s.newest = Some(Arc::new(frame));
            (s.published, s.shown != size)
        };
        post(Msg::Frame(self.id));
        if resized {
            let until = Instant::now() + Duration::from_micros(16_700);
            let mut s = self.slot.lock();
            while s.handled < seq {
                let now = Instant::now();
                if now >= until {
                    break;
                }
                s = self
                    .slot
                    .handled
                    .wait_timeout(s, until - now)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        }
    }

    /// Whether a frame of this size would only wait: the last one handed
    /// over is still held for the next refresh. The main thread then puts
    /// its frame off ([`Handle::want`]) rather than list one that would be
    /// replaced before it is drawn.
    pub fn would_wait(&self, size: (u32, u32)) -> bool {
        let on_way = self.slot.on_way.load(std::sync::atomic::Ordering::Acquire);
        let paced = self.linked || frames::synthetic_hz().is_some();
        let s = self.slot.lock();
        // held for the next refresh, still being drawn, or a frame on its
        // way already
        let busy = s.newest.is_some() || s.handled < s.published || (paced && on_way >= limit());
        busy && s.shown == size
    }

    /// The main thread put a frame off: it is asked for it
    /// ([`Cmd::NextFrame`]) as soon as the render thread can take one.
    pub fn want(&self) {
        self.slot
            .wanted
            .store(true, std::sync::atomic::Ordering::Release);
        post(Msg::Want(self.id));
    }

    /// What the window shows may be gone: the next frame is drawn whole.
    pub fn invalidate(&self) {
        post(Msg::Invalidate(self.id));
    }

    /// Covered, minimized or on another space: nothing is on its way.
    pub fn occluded(&self) {
        post(Msg::Occluded(self.id));
    }

    /// A refresh stood in for the display's (`GUI_REFRESH_HZ`).
    pub fn tick(&self, at: u64) {
        post(Msg::Tick { id: self.id, at });
    }

    /// The window closes: its surface and display link are let go on the
    /// render thread, before the window itself goes.
    pub fn remove(&self) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        post(Msg::Remove(self.id, tx));
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
}

/// Window `id`'s kept frame as the render thread holds it between frames
/// (its size, straight RGBA), or `None` when the window is not drawn
/// there: a test reads what the GPU drew.
pub fn kept(id: u64) -> Option<(u32, u32, Vec<u8>)> {
    thread()?;
    let (tx, rx) = crossbeam_channel::bounded(1);
    post(Msg::Read(id, tx));
    rx.recv_timeout(Duration::from_secs(2)).ok()
}

/// Whether windows are drawn on the render thread (`GUI_RENDER_THREAD=0`:
/// on the main thread, as before).
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("GUI_RENDER_THREAD").map_or(true, |v| v != "0"))
}

/// Whether the render thread runs (started on first use).
pub fn available() -> bool {
    thread().is_some()
}

/// Hand window `id`'s surface (and display link, made on the main thread
/// and not yet scheduled) to the render thread ([`available`] first).
pub fn adopt(id: u64, surface: gpu::Surface, link: Option<Retained<AnyObject>>) -> Handle {
    let slot = Arc::new(Slot::default());
    let linked = link.is_some();
    post(Msg::Add {
        id,
        surface: Box::new(surface),
        slot: slot.clone(),
        link: link.map(Link),
    });
    Handle { id, slot, linked }
}

enum Msg {
    Add {
        id: u64,
        surface: Box<gpu::Surface>,
        slot: Arc<Slot>,
        link: Option<Link>,
    },
    Frame(u64),
    Tick {
        id: u64,
        at: u64,
    },
    Invalidate(u64),
    Occluded(u64),
    /// A frame the surface could not take yet, tried again.
    Retry(u64),
    /// The main thread put a frame off ([`Handle::want`]).
    Want(u64),
    /// The kept frame's size and pixels (a test's).
    Read(u64, crossbeam_channel::Sender<(u32, u32, Vec<u8>)>),
    Remove(u64, crossbeam_channel::Sender<()>),
}

/// CoreFoundation's run loop, as this thread uses it.
mod cf {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct SourceContext {
        pub version: isize,
        pub info: *mut c_void,
        pub retain: *const c_void,
        pub release: *const c_void,
        pub copy_description: *const c_void,
        pub equal: *const c_void,
        pub hash: *const c_void,
        pub schedule: *const c_void,
        pub cancel: *const c_void,
        pub perform: extern "C-unwind" fn(*mut c_void),
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFRunLoopGetCurrent() -> *const c_void;
        pub fn CFRunLoopSourceCreate(
            alloc: *const c_void,
            order: isize,
            ctx: *mut SourceContext,
        ) -> *mut c_void;
        pub fn CFRunLoopAddSource(rl: *mut c_void, src: *mut c_void, mode: *const c_void);
        pub fn CFRunLoopSourceSignal(src: *mut c_void);
        pub fn CFRunLoopWakeUp(rl: *mut c_void);
        pub fn CFRunLoopRun();
        pub static kCFRunLoopCommonModes: *const c_void;
    }
}

struct Thread {
    tx: crossbeam_channel::Sender<Msg>,
    /// The thread's run loop and its source (CoreFoundation objects, safe
    /// to signal and wake from any thread).
    run_loop: usize,
    source: usize,
    id: std::thread::ThreadId,
}

/// The render thread, started on first use; `None` when it could not be.
fn thread() -> Option<&'static Thread> {
    static T: OnceLock<Option<Thread>> = OnceLock::new();
    T.get_or_init(|| {
        let (tx, rx) = crossbeam_channel::unbounded::<Msg>();
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        let handle = std::thread::Builder::new()
            .name("gui-render".into())
            .spawn(move || {
                let mut ctx = cf::SourceContext {
                    version: 0,
                    info: std::ptr::null_mut(),
                    retain: std::ptr::null(),
                    release: std::ptr::null(),
                    copy_description: std::ptr::null(),
                    equal: std::ptr::null(),
                    hash: std::ptr::null(),
                    schedule: std::ptr::null(),
                    cancel: std::ptr::null(),
                    perform,
                };
                // SAFETY: CoreFoundation's documented calls; the context is
                // copied by CFRunLoopSourceCreate.
                let (rl, src) = unsafe {
                    let rl = cf::CFRunLoopGetCurrent() as *mut c_void;
                    let src = cf::CFRunLoopSourceCreate(std::ptr::null(), 0, &mut ctx);
                    cf::CFRunLoopAddSource(rl, src, cf::kCFRunLoopCommonModes);
                    (rl, src)
                };
                STATE.with(|s| {
                    *s.borrow_mut() = Some(State {
                        rx,
                        wins: HashMap::new(),
                    })
                });
                let _ = ready_tx.send((rl as usize, src as usize));
                loop {
                    // SAFETY: this thread's own run loop; it returns only
                    // when it has nothing to run, which a source keeps from
                    // happening.
                    unsafe { cf::CFRunLoopRun() };
                }
            })
            .ok()?;
        let (rl, src) = ready_rx.recv_timeout(Duration::from_secs(5)).ok()?;
        Some(Thread {
            tx,
            run_loop: rl,
            source: src,
            id: handle.thread().id(),
        })
    })
    .as_ref()
}

/// Whether this is the render thread.
pub fn here() -> bool {
    thread().is_some_and(|t| t.id == std::thread::current().id())
}

fn post(m: Msg) {
    let Some(t) = thread() else {
        return;
    };
    if t.tx.send(m).is_ok() {
        // SAFETY: the thread's live source and run loop (never released).
        unsafe {
            cf::CFRunLoopSourceSignal(t.source as *mut c_void);
            cf::CFRunLoopWakeUp(t.run_loop as *mut c_void);
        }
    }
}

/// A frame the surface could not take, tried again in `ms`.
fn retry_in(id: u64, ms: u64) {
    let _ = std::thread::Builder::new()
        .name("gui-render-retry".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            post(Msg::Retry(id));
        });
}

/// A window as the render thread keeps it.
struct Win {
    surface: gpu::Surface,
    slot: Arc<Slot>,
    link: Option<Retained<AnyObject>>,
    /// Frames presented and not yet shown, oldest first: when each was
    /// presented, and the program's answer it carries (`GUI_LATENCY`).
    in_flight: VecDeque<(u64, Option<u64>)>,
    /// A frame waits in the slot for the next refresh.
    pending: bool,
    /// The last frame drawn moves at the next refresh.
    animating: bool,
    idle_ticks: u32,
    last_tick: u64,
    /// A frame the surface could not take is kept to be tried again.
    retry: Option<Arc<Frame>>,
    lost_said: bool,
}

struct State {
    rx: crossbeam_channel::Receiver<Msg>,
    wins: HashMap<u64, Win>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The run loop's source fired: the messages waiting are handled.
extern "C-unwind" fn perform(_info: *mut c_void) {
    let ok = contain_callback("render_thread", || {
        objc2::rc::autoreleasepool(|_| {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                let Some(st) = s.as_mut() else {
                    return;
                };
                while let Ok(m) = st.rx.try_recv() {
                    handle(st, m);
                }
            })
        })
    });
    if ok.is_none() {
        recover();
    }
}

/// After a contained panic: every window drawn again whole.
fn recover() {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return;
        };
        let Some(st) = s.as_mut() else {
            return;
        };
        for (id, w) in st.wins.iter_mut() {
            w.surface.invalidate();
            w.in_flight.clear();
            w.pending = false;
            w.sync();
            send(Cmd::NextFrame(*id));
        }
    });
}

fn handle(st: &mut State, m: Msg) {
    match m {
        Msg::Add {
            id,
            surface,
            slot,
            link,
        } => {
            let link = link.map(|l| l.0);
            if let Some(l) = &link {
                frames::schedule_here(l);
            }
            st.wins.insert(
                id,
                Win {
                    surface: *surface,
                    slot,
                    link,
                    in_flight: VecDeque::new(),
                    pending: false,
                    animating: false,
                    idle_ticks: 0,
                    last_tick: 0,
                    retry: None,
                    lost_said: false,
                },
            );
        }
        Msg::Frame(id) => {
            if let Some(w) = st.wins.get_mut(&id) {
                w.retry = None;
                draw(id, w);
            }
        }
        Msg::Tick { id, at } => tick(st, id, at),
        Msg::Invalidate(id) => {
            if let Some(w) = st.wins.get_mut(&id) {
                w.surface.invalidate();
            }
        }
        Msg::Occluded(id) => {
            if let Some(w) = st.wins.get_mut(&id) {
                // no refresh shows them now: nothing is held for them
                w.in_flight.clear();
                w.animating = false;
                w.sync();
            }
        }
        Msg::Retry(id) => {
            if let Some(w) = st.wins.get_mut(&id)
                && let Some(f) = w.retry.take()
            {
                let mut s = w.slot.lock();
                if s.newest.is_none() {
                    s.newest = Some(f);
                }
                drop(s);
                draw(id, w);
            }
        }
        Msg::Want(id) => {
            if let Some(w) = st.wins.get_mut(&id) {
                prune(w);
                w.sync();
                let free = w.slot.lock().newest.is_none() && w.in_flight.len() < limit();
                if free {
                    ask(id, w);
                } else if w.slot.wanted.load(std::sync::atomic::Ordering::Acquire) {
                    // at the next refresh; or, refreshes not coming (a link
                    // resting, a display asleep), once what is on its way
                    // is old enough to count as shown
                    frames::resume(&w.link);
                    let _ = std::thread::Builder::new()
                        .name("gui-render-want".into())
                        .spawn(move || {
                            std::thread::sleep(Duration::from_millis(60));
                            post(Msg::Want(id));
                        });
                }
            }
        }
        Msg::Read(id, reply) => {
            if let Some(w) = st.wins.get(&id) {
                let _ = reply.send(w.surface.kept_rgba());
            }
        }
        Msg::Remove(id, reply) => {
            if let Some(w) = st.wins.remove(&id) {
                frames::unlink(w.link);
                let surface = w.surface;
                objc2::rc::autoreleasepool(|_| drop(surface));
            }
            let _ = reply.send(());
        }
    }
}

/// A refresh of window `id`'s display (`id` 0: every window's, a stand-in
/// refresh): the oldest frame on its way is on the screen now, a frame put
/// off for it is drawn, something moving asks for its next frame, and a
/// link with nothing to do for a few refreshes rests.
fn tick(st: &mut State, id: u64, at: u64) {
    tick_win(st, id, at);
    if let Some(w) = st.wins.get(&id) {
        w.sync();
    }
}

impl Win {
    /// The count of frames on their way, for the main thread.
    fn sync(&self) {
        self.slot
            .on_way
            .store(self.in_flight.len(), std::sync::atomic::Ordering::Release);
    }
}

fn tick_win(st: &mut State, id: u64, at: u64) {
    if id == 0 {
        let ids: Vec<u64> = st.wins.keys().copied().collect();
        for w in ids {
            tick(st, w, at);
        }
        return;
    }
    let Some(w) = st.wins.get_mut(&id) else {
        return;
    };
    w.last_tick = frames::now_ns();
    if let Some(&(p, t0)) = w.in_flight.front()
        && p < at
    {
        w.in_flight.pop_front();
        frames::shown(p, t0, at);
    }
    // the main thread put a frame off: it lists one now, from the state as
    // it is (what waited here is replaced by it). The frame let go counts
    // as handled: left owed, the main thread found the render thread busy
    // with it and put every frame after it off too, so the window kept
    // showing an older one until something else drew it.
    if ask(id, w) {
        let mut s = w.slot.lock();
        if s.newest.take().is_some() {
            s.handled = s.published;
            drop(s);
            w.slot.handled.notify_all();
        }
        w.pending = false;
        return;
    }
    if w.pending {
        w.pending = false;
        draw(id, w);
        return;
    }
    if w.animating {
        w.animating = false;
        send(Cmd::NextFrame(id));
        return;
    }
    if w.in_flight.is_empty() {
        w.idle_ticks += 1;
        if w.idle_ticks >= 8 {
            frames::pause(&w.link);
        }
    }
}

/// The display link's tick, on this thread (its run loop).
pub fn tick_here(id: u64, at: u64) {
    let ok = contain_callback("render_thread", || {
        objc2::rc::autoreleasepool(|_| {
            STATE.with(|s| {
                let Ok(mut s) = s.try_borrow_mut() else {
                    return;
                };
                if let Some(st) = s.as_mut() {
                    tick(st, id, at);
                }
            })
        })
    });
    if ok.is_none() {
        recover();
    }
}

/// Frames on their way at once: one (pacing), two under a stand-in
/// refresh, any number with `GUI_PACING=0`.
fn limit() -> usize {
    if frames::pacing() {
        1
    } else if frames::synthetic_hz().is_some() {
        2
    } else {
        usize::MAX
    }
}

/// A frame long on its way was shown (a refresh missed); a link that has
/// not ticked lately (resting, a display asleep) puts nothing off.
fn prune(w: &mut Win) {
    let now = frames::now_ns();
    while w
        .in_flight
        .front()
        .is_some_and(|f| now.saturating_sub(f.0) > 50_000_000)
    {
        w.in_flight.pop_front();
    }
    if now.saturating_sub(w.last_tick) > 50_000_000 {
        w.in_flight.clear();
    }
}

/// A frame the main thread put off asked for; whether one was.
fn ask(id: u64, w: &mut Win) -> bool {
    if w.slot
        .wanted
        .swap(false, std::sync::atomic::Ordering::AcqRel)
    {
        send(Cmd::NextFrame(id));
        true
    } else {
        false
    }
}

/// Draw and present the newest frame of window `id`, unless the pacing
/// holds it for the next refresh.
fn draw(id: u64, w: &mut Win) {
    let synthetic = frames::synthetic_hz().is_some();
    let limit = limit();
    if (w.link.is_some() || synthetic) && w.in_flight.len() >= limit {
        prune(w);
    }
    let frame = {
        let mut s = w.slot.lock();
        let Some(f) = s.newest.as_ref() else {
            return;
        };
        // A frame still on its way to the screen: this one waits for the
        // next refresh (one frame a refresh, never a queue) — never a frame
        // of another size (a live resize draws at once), and only while
        // the refreshes come.
        if (w.link.is_some() || synthetic)
            && w.in_flight.len() >= limit
            && (f.dl.width, f.dl.height) == s.shown
        {
            w.pending = true;
            frames::resume(&w.link);
            return;
        }
        s.newest.take().expect("checked above")
    };
    let trace = std::env::var_os("GUI_TRACE").is_some();
    let t0 = Instant::now();
    let size = (frame.dl.width, frame.dl.height);
    let result = w.surface.render(&frame.dl);
    let presented = match result {
        Ok(()) => w.surface.presented(),
        // Not drawn: the window is covered or the compositor busy. A covered
        // window is drawn again as it shows (the main thread publishes);
        // a busy one tries the same frame again in a moment.
        Err(e) if e == "occluded" || e == "timeout" => {
            if trace {
                eprintln!("gui: window {id} frame skipped ({e})");
            }
            if e == "timeout" {
                w.retry = Some(frame);
                retry_in(id, 16);
            }
            handled(w, size, false);
            return;
        }
        // The device was lost and none could be made again yet: the frame
        // is tried again in a moment.
        Err(e) if e.starts_with("lost") => {
            if !w.lost_said {
                eprintln!(
                    "gui: window {id}: the GPU device is lost; drawing again when one can be had"
                );
                w.lost_said = true;
            }
            w.retry = Some(frame);
            retry_in(id, 250);
            handled(w, size, false);
            return;
        }
        Err(e) => {
            eprintln!("gui: {e}");
            false
        }
    };
    w.lost_said = false;
    count_frame(presented);
    if presented {
        frames::engine_presented(id, frame.covers);
        w.in_flight
            .push_back((frames::now_ns(), frames::answered(id)));
        frames::resume(&w.link);
        w.idle_ticks = 0;
    }
    w.animating = frame.animating;
    if w.animating {
        frames::resume(&w.link);
    }
    handled(w, size, presented);
    crate::boot_trace::frame_presented();
    super::holds::record("render_draw", t0.elapsed());
    if trace {
        eprintln!(
            "gui: window {id} frame {}x{}, {} prims, drawn on the render thread in {:.2} ms ({:.2} ms after it was begun)",
            size.0,
            size.1,
            frame.dl.prims.len(),
            t0.elapsed().as_secs_f64() * 1000.0,
            frame.started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// The frame published last is handled: a resize waiting for it goes on.
fn handled(w: &mut Win, size: (u32, u32), presented: bool) {
    w.sync();
    let mut s = w.slot.lock();
    s.handled = s.published;
    if presented {
        s.shown = size;
    }
    drop(s);
    w.slot.handled.notify_all();
}

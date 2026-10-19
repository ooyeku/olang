//! One window's state, whatever shows it — a platform window or a
//! headless one: the scene, the editors, the size and scale, and the
//! input handling that turns keys, pointer, wheel, and input-method
//! events into scene changes and the events a program hears. Building
//! the display list both renderers draw is here too.
//!
//! Nothing in this file touches the platform: the platform loop and
//! `gui.input` (tests) feed it the same [`Input`] values.

use super::edit::{Editor, Field, Outcome};
use super::raster::GlyphKey;
use super::rich::RichEditor;
use super::scene::{self, Applied, Scene};
use super::text::{self, Align, Color, Shaped, TextSystem};
use super::values::*;
use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Command on macOS, the Windows key elsewhere.
    pub super_: bool,
}

impl Mods {
    /// The platform's command modifier: command on macOS, ctrl elsewhere.
    pub fn command(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.super_
        } else {
            self.ctrl
        }
    }

    /// The word-motion modifier: option on macOS, ctrl elsewhere.
    pub fn word(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.alt
        } else {
            self.ctrl
        }
    }

    /// The line-motion modifier: command on macOS; none elsewhere.
    pub fn line(&self) -> bool {
        cfg!(target_os = "macos") && self.super_
    }

    pub fn value(&self) -> Value {
        map(vec![
            ("ctrl", Value::Boolean(self.ctrl)),
            ("alt", Value::Boolean(self.alt)),
            ("shift", Value::Boolean(self.shift)),
            ("super", Value::Boolean(self.super_)),
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerAction {
    Down,
    Up,
    Move,
    Leave,
}

#[derive(Clone, Debug)]
pub enum Input {
    Key {
        key: String,
        text: Option<String>,
        mods: Mods,
        repeat: bool,
    },
    Pointer {
        action: PointerAction,
        x: f32,
        y: f32,
        button: String,
        /// The click count of a press (1, 2, 3); computed by the window
        /// when 0.
        clicks: u32,
    },
    Wheel {
        dx: f32,
        dy: f32,
        /// The modifiers held, when the event says (a test's); else the
        /// window's last.
        mods: Option<Mods>,
    },
    /// A scroll with where it is in its gesture (scroll.rs): a trackpad's
    /// pulls a region past its ends and its momentum bounces there; a
    /// mouse wheel's notch (`precise` false) glides. `Wheel` is one with
    /// no phase.
    Scroll {
        dx: f32,
        dy: f32,
        mods: Option<Mods>,
        phase: super::scroll::Phase,
        precise: bool,
    },
    /// A pinch on a trackpad: `delta` the change of magnification
    /// (positive zooms in), `phase` start, move, end, or cancel.
    Pinch {
        delta: f32,
        phase: String,
    },
    ImePreedit(String, Option<(usize, usize)>),
    ImeCommit(String),
    Resize {
        width: f32,
        height: f32,
        scale: f32,
    },
    Focused(bool),
    Modifiers(Mods),
    /// Files from another program (a file manager): hovering over the
    /// window (`"hover"`), dropped on it (`"drop"`), or the hover gone
    /// (`"cancel"`); `at` the place, in logical pixels, when known (else
    /// the pointer's last).
    Files {
        action: String,
        paths: Vec<String>,
        at: Option<(f32, f32)>,
    },
}

/// The clipboard as the window sees it: the platform's for a real
/// window, a private one for a headless window (so a test never reads
/// or clobbers the user's).
pub trait Clipboard {
    fn get(&mut self) -> Option<String>;
    fn set(&mut self, text: String);
}

struct NoClipboard;

impl Clipboard for NoClipboard {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _: String) {}
}

// A styled field says its first line in view when it moves into another
// band of this many lines (a field says its own band: rich.rs `band`, 16
// lines by default).

pub struct WinState {
    pub id: u64,
    pub headless: bool,
    /// Driven by a script (`gui.set(w, #{ "scripted": true })`, Loom's
    /// `run` with a script): a person's keys, pointer, wheel and drops on
    /// the real window are not taken — only `gui.input`'s — so the mouse
    /// passing over a test's window cannot move what the script drags.
    pub scripted: bool,
    pub title: String,
    pub scene: Scene,
    pub editors: HashMap<String, Field>,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub clear: Color,
    pub mods: Mods,
    pub pointer: (f32, f32),
    pub focused_window: bool,
    last_press: Option<(std::time::Instant, f32, f32, u32)>,
    /// The scene changed since the last frame.
    pub dirty: bool,
    /// The accessibility tree should be rebuilt.
    pub a11y_dirty: bool,
    /// Where the last frame drew the caret, in logical pixels and
    /// clipped to what shows: `None` when no caret was visible.
    pub caret: Option<[f32; 4]>,
    /// The clipboard of input sent through `gui.input` (a test's).
    pub clip: Option<String>,
    /// The text size: the program's units are this many logical pixels.
    /// `width`, `height`, and pointer positions are in units; `scale` is
    /// the platform's scale times the zoom, so physical pixels stay put.
    pub zoom: f32,
    /// The platform's own scale factor.
    pub platform_scale: f32,
    /// Where the window's content sits on the screen, in the platform's
    /// logical pixels: a pointer event carries its place on the screen
    /// too (`sx`, `sy`), so a drag can be followed into another window.
    /// `None` while the platform has not said (Wayland never does: a
    /// window there cannot learn its place), and then a pointer event
    /// carries no `sx`/`sy` — a drag stays in its own window rather than
    /// landing in another by a guess.
    pub origin: Option<(f32, f32)>,
    /// The last pointer event's place on the screen, when the origin is
    /// known.
    screen_pointer: Option<(f32, f32)>,
    /// The system's settings as this window last heard them (a headless
    /// window's from `gui.input`'s `appearance`): reduced motion stops
    /// its pictures' animations.
    pub settings: super::context::Settings,
    /// When the window was made: a real window's clock for animations.
    born: std::time::Instant,
    /// A headless window's clock, in ms (`gui.input`'s `clock`): its
    /// animations move only when a test moves it. `None` for a real one.
    pub test_clock: Option<f64>,
    /// Each animated picture's playing, by node key.
    plays: HashMap<String, Play>,
    /// The animated pictures the last frame drew.
    pub shown_anims: Vec<AnimShown>,
    /// When the next frame of a playing animation in view is due, on
    /// this window's clock (ms): the platform draws again then. `None`
    /// when nothing in view moves.
    pub next_frame: Option<f64>,
    /// The press that played or paused an animation: its release is not
    /// the program's.
    swallow_up: bool,
    /// The press being said was made with the command key (a styled
    /// field's `select` says `click_mod`).
    click_mod: bool,
    /// The presses of the click that moved a field's selection (2: a
    /// double click), said on its `select` as `clicks` (0: not a press).
    click_count: u32,
    /// Each scroller's overlay bar (bars.rs), by its key.
    pub bars: HashMap<String, super::bars::Bar>,
    /// The bars the last frame drew, in the order drawn (the pointer
    /// finds the last first).
    bar_hits: Vec<super::bars::Hit>,
    /// The scroller whose thumb is being dragged.
    bar_drag: Option<String>,
    /// The nodes' transitions and the nodes leaving (motion.rs).
    pub motion: super::motion::Motion,
    /// Each scroller stretched past its ends, springing back or gliding
    /// (scroll.rs), by key.
    pub elastic: HashMap<String, super::scroll::Elastic>,
}

/// An animated picture's playing: since when, and where it stopped.
#[derive(Clone, Copy, Debug)]
struct Play {
    /// The animation it plays (its first frame's picture id): a new
    /// source starts again.
    id: u64,
    /// When it started, on the window's clock.
    start: f64,
    /// Paused this many ms in, or playing.
    paused: Option<f64>,
    /// Played or paused by the person (under reduced motion it otherwise
    /// stands on its first frame).
    chosen: bool,
}

/// An animated picture as the last frame drew it.
#[derive(Clone, Debug)]
pub struct AnimShown {
    pub key: String,
    pub frames: usize,
    pub frame: usize,
    pub paused: bool,
    /// Where a press plays or pauses it (logical pixels), when it has a
    /// button: under reduced motion, or once the person used it.
    pub badge: Option<[f32; 4]>,
}

/// The play (or pause) button over a still animation, `side` device
/// pixels square: a dark disc, a white glyph.
fn badge_picture(side: u32, play: bool) -> Arc<super::canvas::Picture> {
    type Badges = std::sync::Mutex<HashMap<(u32, bool), Arc<super::canvas::Picture>>>;
    static CACHE: std::sync::OnceLock<Badges> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Ok(c) = cache.lock()
        && let Some(p) = c.get(&(side, play))
    {
        return p.clone();
    }
    let side = side.max(8);
    let mut pm = tiny_skia::Pixmap::new(side, side).expect("a small pixmap");
    let f = side as f32;
    let mut paint = tiny_skia::Paint {
        anti_alias: true,
        ..Default::default()
    };
    paint.set_color_rgba8(0, 0, 0, 150);
    if let Some(disc) = tiny_skia::PathBuilder::from_circle(f / 2.0, f / 2.0, f / 2.0) {
        pm.fill_path(
            &disc,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    paint.set_color_rgba8(255, 255, 255, 255);
    let mut pb = tiny_skia::PathBuilder::new();
    if play {
        pb.move_to(f * 0.40, f * 0.30);
        pb.line_to(f * 0.72, f * 0.50);
        pb.line_to(f * 0.40, f * 0.70);
        pb.close();
    } else {
        pb.push_rect(
            tiny_skia::Rect::from_xywh(f * 0.36, f * 0.31, f * 0.10, f * 0.38).expect("a bar"),
        );
        pb.push_rect(
            tiny_skia::Rect::from_xywh(f * 0.54, f * 0.31, f * 0.10, f * 0.38).expect("a bar"),
        );
    }
    if let Some(path) = pb.finish() {
        pm.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    let pic = Arc::new(super::canvas::Picture {
        id: super::canvas::next_picture_id(),
        width: pm.width(),
        height: pm.height(),
        rgba: pm.take(),
    });
    if let Ok(mut c) = cache.lock() {
        c.insert((side, play), pic.clone());
    }
    pic
}

const WHITE: Color = [255, 255, 255, 255];

impl WinState {
    /// Make the window the key one (`on`) or not, as the platform says
    /// it with `window_focus`: the flag, a redraw, and the event — only
    /// when it changes. The headless windows' one key window is kept by
    /// `gui` (`mod.rs`'s key model) through this.
    pub fn set_key(&mut self, on: bool, out: &mut Vec<Value>) {
        if self.focused_window == on {
            return;
        }
        self.focused_window = on;
        self.dirty = true;
        self.a11y_dirty = true;
        out.push(self.ev("window_focus", vec![("on", Value::Boolean(on))]));
    }

    pub fn new(id: u64, headless: bool, title: &str, width: f32, height: f32, scale: f32) -> Self {
        WinState {
            id,
            headless,
            scripted: false,
            title: title.to_string(),
            scene: Scene::default(),
            editors: HashMap::new(),
            width,
            height,
            scale,
            clear: WHITE,
            mods: Mods::default(),
            pointer: (-1.0, -1.0),
            focused_window: true,
            last_press: None,
            dirty: true,
            a11y_dirty: true,
            caret: None,
            clip: None,
            zoom: 1.0,
            platform_scale: scale,
            // a headless window sits at the screen's corner until a test
            // places it; a real one waits for the platform
            origin: if headless { Some((0.0, 0.0)) } else { None },
            screen_pointer: None,
            settings: super::context::Settings::default(),
            born: std::time::Instant::now(),
            test_clock: if headless { Some(0.0) } else { None },
            plays: HashMap::new(),
            shown_anims: Vec::new(),
            next_frame: None,
            swallow_up: false,
            click_mod: false,
            click_count: 0,
            bars: HashMap::new(),
            bar_hits: Vec::new(),
            bar_drag: None,
            motion: super::motion::Motion {
                enabled: !headless,
                ..Default::default()
            },
            elastic: HashMap::new(),
        }
    }

    /// Scrollers the window moved itself (to bring the focus into view):
    /// each says so, as the wheel's do.
    pub fn said_scrolled(&mut self, moved: Vec<(String, (f32, f32))>, out: &mut Vec<Value>) {
        for (k, (x, y)) in moved {
            self.a11y_dirty = true;
            out.push(self.ev(
                "scrolled",
                vec![("key", s(&k)), ("x", float(x)), ("y", float(y))],
            ));
        }
    }

    /// This window's clock for animations, in ms: a headless window's is
    /// the test's.
    pub fn clock_ms(&self) -> f64 {
        self.test_clock
            .unwrap_or_else(|| self.born.elapsed().as_secs_f64() * 1000.0)
    }

    /// A headless window's clock moved (`gui.input`'s `clock`).
    pub fn set_clock(&mut self, ms: f64) {
        if self.test_clock != Some(ms) {
            self.test_clock = Some(ms);
            if !self.shown_anims.is_empty()
                || self.bars_moving()
                || self.motion.moving()
                || !self.elastic.is_empty()
            {
                self.dirty = true;
            }
        }
    }

    /// The animated picture `key` played (or paused) as by its button.
    pub fn toggle_play(&mut self, key: &str) {
        let now = self.clock_ms();
        let rm = self.settings.reduce_motion;
        if let Some(p) = self.plays.get_mut(key) {
            let held = p.paused.or(if rm && !p.chosen { Some(0.0) } else { None });
            match held {
                Some(e) => {
                    p.start = now - e;
                    p.paused = None;
                }
                None => p.paused = Some(now - p.start),
            }
            p.chosen = true;
            self.dirty = true;
            self.a11y_dirty = true;
        }
    }

    /// The animated picture whose button is at `(x, y)` with nothing over
    /// it (`hit` the node there), if any.
    fn badge_at(&self, x: f32, y: f32, hit: Option<&str>) -> Option<String> {
        let hit = hit?;
        self.shown_anims.iter().find_map(|a| {
            let [bx, by, bw, bh] = a.badge?;
            let inside = x >= bx && y >= by && x < bx + bw && y < by + bh;
            (inside && (hit == a.key || self.scene.within(&a.key, hit))).then(|| a.key.clone())
        })
    }

    /// The animation `key` as the last frame drew it.
    pub fn anim_shown(&self, key: &str) -> Option<&AnimShown> {
        self.shown_anims.iter().find(|a| a.key == key)
    }

    /// The system's settings changed: kept, and the window drawn again
    /// (a picture that animated stops under reduced motion).
    pub fn set_settings(&mut self, s: super::context::Settings) {
        if self.settings != s {
            self.settings = s;
            self.dirty = true;
        }
    }

    /// The window's content moved on the screen (or a test placed a
    /// headless one): `moved` with its new origin.
    pub fn place(&mut self, x: f32, y: f32, out: &mut Vec<Value>) {
        if self.origin != Some((x, y)) {
            self.origin = Some((x, y));
            out.push(self.ev("moved", vec![("x", float(x)), ("y", float(y))]));
        }
    }

    fn ev(&self, kind: &str, mut fields: Vec<(&str, Value)>) -> Value {
        fields.push(("window", Value::Integer(self.id as i64)));
        event(kind, fields)
    }

    /// Apply a patch from the program.
    pub fn apply(&mut self, ops: &Value, out: &mut Vec<Value>) -> Res<()> {
        let mut applied = Applied::default();
        let before_focus = self.scene.focus.clone();
        if self.motion.runs(self.settings.reduce_motion) {
            self.motion_patch(ops);
        }
        self.scene.apply(ops, &mut applied)?;
        for k in &applied.removed {
            if !self.scene.nodes.contains_key(k) {
                self.editors.remove(k);
            }
        }
        let mut ts = text::system()
            .lock()
            .map_err(|_| "gui: text system poisoned")?;
        for k in &applied.edits {
            let Some(node) = self.scene.nodes.get(k) else {
                continue;
            };
            let props = node.edit.clone().expect("an edit node");
            let style = node.style.clone();
            // a field that became styled, or plain again, starts over
            if self
                .editors
                .get(k)
                .is_some_and(|e| e.rich().is_some() != props.rich)
            {
                self.editors.remove(k);
            }
            match self.editors.get_mut(k) {
                Some(ed) => {
                    ed.set_flags(props.multiline, props.secure);
                    ed.set_font(&style.font, style.color);
                    let adopted = ed.offer(&props.value, props.rev, &mut ts);
                    let mut reselected = false;
                    if let Some(r) = ed.rich_mut() {
                        r.own_undo = props.undo;
                        let current = adopted || r.value_is(&props.value);
                        if current {
                            r.set_spans(props.styles.as_ref(), props.spans.as_ref());
                        }
                        r.gutter = props.gutter.as_ref().and_then(super::rich::Gutter::parse);
                        r.decorations = super::rich::parse_decorations(props.decorations.as_ref());
                        r.set_lenses(super::rich::parse_lenses(props.lenses.as_ref()));
                        r.carets = super::rich::parse_carets(props.carets.as_ref());
                        r.eol = super::rich::parse_eol(props.eol.as_ref());
                        r.sticky = super::rich::Sticky::parse(props.sticky.as_ref());
                        let (cols, rc) = super::rich::parse_rulers(props.rulers.as_ref());
                        r.rulers = cols;
                        r.ruler_color = rc;
                        r.set_nowrap(!props.wrap);
                        r.readonly = props.readonly;
                        r.band = props.band.unwrap_or(16).max(1);
                        r.track = super::rich::parse_track(props.track.as_ref());
                        r.past_end = props.past_end;
                        if let Some((line, seq)) = props.scroll_to
                            && seq != r.scroll_seq
                            && current
                        {
                            r.scroll_seq = seq;
                            r.scroll_to_line(line.max(0) as usize, &mut ts);
                        }
                        if let Some((a, f, seq)) = props.select
                            && seq != r.select_seq
                            && current
                        {
                            r.select_seq = seq;
                            r.select_chars(a.max(0) as usize, f.max(0) as usize);
                            reselected = true;
                        }
                        if adopted {
                            r.mark_reported();
                        }
                    }
                    // a plain field takes the program's selection too
                    if let super::edit::Field::Plain(e) = &mut *ed
                        && let Some((a, f, seq)) = props.select
                        && seq != e.select_seq
                        && (adopted || e.value() == *props.value)
                    {
                        e.select_seq = seq;
                        e.select_chars(a.max(0) as usize, f.max(0) as usize, &mut ts);
                        reselected = true;
                    }
                    if adopted || reselected {
                        // Adopted the program's value (or its selection):
                        // say so, with the revision it now has (a styled
                        // field's value shared, not copied).
                        let (a, f) = ed.selection_chars();
                        let rev = ed.rev();
                        let v = match ed.rich_mut() {
                            Some(r) => Value::String(r.value_arc()),
                            None => s(&ed.value()),
                        };
                        let mut e = self.changed_event_value(k, v, rev, a, f);
                        if let Some(r) = self.editors.get_mut(k).and_then(|e| e.rich_mut()) {
                            r.said_selection = (a, f);
                        }
                        if !adopted {
                            e = self.ev(
                                "select",
                                vec![
                                    ("key", s(k)),
                                    ("selection", pair(a, f)),
                                    ("rev", Value::Integer(rev)),
                                ],
                            );
                        }
                        out.push(e);
                    }
                }
                None => {
                    let ed = if props.rich {
                        let mut r = RichEditor::new(&props.value, &style.font, style.color);
                        r.secure = props.secure;
                        r.own_undo = props.undo;
                        r.set_spans(props.styles.as_ref(), props.spans.as_ref());
                        r.gutter = props.gutter.as_ref().and_then(super::rich::Gutter::parse);
                        r.decorations = super::rich::parse_decorations(props.decorations.as_ref());
                        r.set_lenses(super::rich::parse_lenses(props.lenses.as_ref()));
                        r.carets = super::rich::parse_carets(props.carets.as_ref());
                        r.eol = super::rich::parse_eol(props.eol.as_ref());
                        r.sticky = super::rich::Sticky::parse(props.sticky.as_ref());
                        let (cols, rc) = super::rich::parse_rulers(props.rulers.as_ref());
                        r.rulers = cols;
                        r.ruler_color = rc;
                        r.set_nowrap(!props.wrap);
                        r.readonly = props.readonly;
                        r.band = props.band.unwrap_or(16).max(1);
                        r.track = super::rich::parse_track(props.track.as_ref());
                        r.past_end = props.past_end;
                        if let Some((a, f, seq)) = props.select {
                            r.select_seq = seq;
                            r.select_chars(a.max(0) as usize, f.max(0) as usize);
                            r.said_selection = r.selection_chars();
                        }
                        if let Some((line, seq)) = props.scroll_to {
                            r.scroll_seq = seq;
                            r.scroll_to_line(line.max(0) as usize, &mut ts);
                        }
                        Field::Rich(Box::new(r))
                    } else {
                        let mut e = Editor::new(
                            &props.value,
                            &style.font,
                            style.color,
                            props.multiline,
                            props.secure,
                        );
                        if let Some((a, f, seq)) = props.select {
                            e.select_seq = seq;
                            e.select_chars(a.max(0) as usize, f.max(0) as usize, &mut ts);
                        }
                        Field::Plain(e)
                    };
                    self.editors.insert(k.clone(), ed);
                }
            }
        }
        // Editors whose node stopped being editable go away.
        let scene = &self.scene;
        self.editors
            .retain(|k, _| scene.nodes.get(k).is_some_and(|n| n.edit.is_some()));
        // A modal layer that appeared takes the focus into it.
        if let Some(m) = self.scene.modal_root() {
            let inside = self
                .scene
                .focus
                .as_ref()
                .is_some_and(|f| self.scene.within(f, &m));
            if !inside {
                self.scene.focus = self.scene.focus_order().first().cloned();
                applied.focus_set = true;
            }
        }
        if applied.focus_set && self.scene.focus != before_focus {
            if let Some(f) = self.scene.focus.clone() {
                let moved = self.scene.reveal(&f);
                self.said_scrolled(moved, out);
            }
            out.push(self.ev("focus", vec![("key", opt_str(self.scene.focus.as_deref()))]));
        }
        self.dirty = true;
        self.a11y_dirty = true;
        Ok(())
    }

    fn changed_event(&self, key: &str, value: String, rev: i64, a: usize, f: usize) -> Value {
        self.changed_event_value(key, s(&value), rev, a, f)
    }

    fn changed_event_value(&self, key: &str, value: Value, rev: i64, a: usize, f: usize) -> Value {
        self.ev(
            "changed",
            vec![
                ("key", s(key)),
                ("value", value),
                ("rev", Value::Integer(rev)),
                ("selection", pair(a, f)),
            ],
        )
    }

    /// A styled field's edit, said: the value, the selection, what
    /// changed by line and by character, and where the caret is.
    fn rich_changed_event(&mut self, key: &str) -> Option<Value> {
        let caret = self.caret_rect(key);
        let top = self.rich_top(key);
        let place = self.rich_place(key);
        let r = self.editors.get_mut(key)?.rich_mut()?;
        let d = r.take_delta();
        let (a, f) = r.selection_chars();
        r.said_selection = (a, f);
        let (v, rev) = (r.value_arc(), r.rev);
        let lines = Value::List(Arc::new(d.lines.iter().map(|l| s(l)).collect()));
        Some(self.ev(
            "changed",
            vec![
                ("key", s(key)),
                ("place", place),
                ("value", Value::String(v)),
                ("rev", Value::Integer(rev)),
                ("selection", pair(a, f)),
                (
                    "delta",
                    map(vec![
                        ("first", Value::Integer(d.first as i64)),
                        ("removed", Value::Integer(d.removed as i64)),
                        ("lines", lines),
                        ("at", Value::Integer(d.at as i64)),
                        ("old_len", Value::Integer(d.old_len as i64)),
                        ("inserted", s(&d.inserted)),
                    ]),
                ),
                ("caret", rect_value(caret)),
                (
                    "top",
                    top.map(|t| Value::Integer(t as i64)).unwrap_or(Value::Unit),
                ),
            ],
        ))
    }

    /// A styled field's selection by line: `(anchor line, its column,
    /// focus line, its column)`, characters within the line.
    fn rich_place(&self, key: &str) -> Value {
        match self.editors.get(key).and_then(|e| e.rich()) {
            Some(r) => {
                let ((al, ac), (fl, fc)) = r.selection_in_paras();
                Value::Tuple(Arc::new(vec![
                    Value::Integer(al as i64),
                    Value::Integer(ac as i64),
                    Value::Integer(fl as i64),
                    Value::Integer(fc as i64),
                ]))
            }
            None => Value::Unit,
        }
    }

    /// A styled field's first line in view.
    fn rich_top(&mut self, key: &str) -> Option<usize> {
        let mut ts = text::system().lock().ok()?;
        let r = self.editors.get_mut(key)?.rich_mut()?;
        let t = r.top_line(&mut ts);
        r.said_top = Some(t);
        Some(t)
    }

    /// A styled field whose selection moved without an edit, said when
    /// the program asked (`report`).
    fn report_selection(&mut self, key: &str, out: &mut Vec<Value>) {
        let wants = self
            .scene
            .nodes
            .get(key)
            .and_then(|n| n.edit.as_ref())
            .is_some_and(|e| e.report);
        if !wants {
            return;
        }
        let Some(r) = self.editors.get(key).and_then(|e| e.rich()) else {
            return;
        };
        let sel = r.selection_chars();
        if sel == r.said_selection {
            return;
        }
        let rev = r.rev;
        let caret = self.caret_rect(key);
        let top = self.rich_top(key);
        if let Some(r) = self.editors.get_mut(key).and_then(|e| e.rich_mut()) {
            r.said_selection = sel;
        }
        let click_mod = std::mem::take(&mut self.click_mod);
        let clicks = std::mem::take(&mut self.click_count);
        let place = self.rich_place(key);
        out.push(self.ev(
            "select",
            vec![
                ("key", s(key)),
                ("place", place),
                ("selection", pair(sel.0, sel.1)),
                ("rev", Value::Integer(rev)),
                ("caret", rect_value(caret)),
                (
                    "top",
                    top.map(|t| Value::Integer(t as i64)).unwrap_or(Value::Unit),
                ),
                ("click_mod", Value::Boolean(click_mod)),
                ("clicks", Value::Integer(clicks as i64)),
            ],
        ));
    }

    /// A selection an assistive client set, said as the pointer's would be.
    pub fn said_selection_of(&mut self, key: &str, out: &mut Vec<Value>) {
        self.report_selection(key, out);
    }

    /// Where a styled field's caret is, logical pixels in the window,
    /// whether or not it shows (scrolled as the next frame will be).
    pub fn caret_rect(&mut self, key: &str) -> Option<[f32; 4]> {
        let (ox, oy, _, h) = self.edit_origin(key)?;
        let s = self.scale.max(0.01);
        let mut ts = text::system().lock().ok()?;
        let r = self.editors.get_mut(key)?.rich_mut()?;
        r.follow_caret(h, &mut ts);
        let b = r.caret_box(1.0, &mut ts);
        Some([
            (ox + b.x0 as f32) / s,
            (oy + b.y0 as f32 - r.scroll_y) / s,
            ((b.x1 - b.x0) as f32).max(1.0) / s,
            ((b.y1 - b.y0) as f32) / s,
        ])
    }

    fn set_focus(&mut self, key: Option<String>, out: &mut Vec<Value>) {
        if self.scene.focus == key {
            return;
        }
        // Leaving a field ends its composition.
        if let Some(old) = &self.scene.focus
            && let Some(ed) = self.editors.get_mut(old)
            && ed.is_composing()
            && let Ok(mut ts) = text::system().lock()
        {
            ed.compose("", None, &mut ts);
        }
        self.scene.focus = key.clone();
        if let Some(k) = &key {
            let moved = self.scene.reveal(k);
            self.said_scrolled(moved, out);
        }
        out.push(self.ev("focus", vec![("key", opt_str(key.as_deref()))]));
        self.dirty = true;
        self.a11y_dirty = true;
    }

    fn move_focus(&mut self, forward: bool, out: &mut Vec<Value>) {
        let order = self.scene.focus_order();
        if order.is_empty() {
            return;
        }
        let at = self
            .scene
            .focus
            .as_ref()
            .and_then(|f| order.iter().position(|k| k == f));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => order.len() - 1,
            (Some(i), true) => (i + 1) % order.len(),
            (Some(i), false) => (i + order.len() - 1) % order.len(),
        };
        self.set_focus(Some(order[next].clone()), out);
    }

    /// The text origin of an editable node, device pixels in the window.
    fn edit_origin(&self, key: &str) -> Option<(f32, f32, f32, f32)> {
        let node = self.scene.nodes.get(key)?;
        let abs = self.scene.absolute(key)?;
        let st = &node.style;
        let x = (abs[0] + st.pad[3]) * self.scale;
        let y = (abs[1] + st.pad[0]) * self.scale;
        let w = (abs[2] - st.pad[1] - st.pad[3]).max(0.0) * self.scale;
        let h = (abs[3] - st.pad[0] - st.pad[2]).max(0.0) * self.scale;
        Some((x, y, w, h))
    }

    /// Vertical offset of a single-line field's text, centring its line
    /// in the content box.
    fn edit_text_dy(&mut self, key: &str, content_h: f32, ts: &mut TextSystem) -> f32 {
        let Some(ed) = self.editors.get_mut(key).and_then(|e| e.plain_mut()) else {
            return 0.0;
        };
        if ed.multiline {
            return 0.0;
        }
        let h = ed.layout(ts).height();
        ((content_h - h) / 2.0).max(0.0)
    }

    /// Set the text size: the window keeps its logical size and lays out
    /// in units of `zoom` logical pixels (a resize event says the new size).
    pub fn set_zoom(&mut self, zoom: f32, out: &mut Vec<Value>) {
        let z = zoom.clamp(0.5, 3.0);
        if (z - self.zoom).abs() < 1e-4 {
            return;
        }
        let (lw, lh) = (self.width * self.zoom, self.height * self.zoom);
        self.zoom = 1.0;
        self.input(
            Input::Resize {
                width: lw,
                height: lh,
                scale: self.platform_scale,
            },
            &mut NoClipboard,
            out,
        );
        self.zoom = z;
        self.input(
            Input::Resize {
                width: lw,
                height: lh,
                scale: self.platform_scale,
            },
            &mut NoClipboard,
            out,
        );
    }

    pub fn input(&mut self, input: Input, clip: &mut dyn Clipboard, out: &mut Vec<Value>) {
        // What the platform says in logical pixels arrives in units.
        let z = self.zoom;
        let input = match input {
            Input::Pointer {
                action,
                x,
                y,
                button,
                clicks,
            } => {
                self.screen_pointer = self.origin.map(|(ox, oy)| (ox + x, oy + y));
                Input::Pointer {
                    action,
                    x: x / z,
                    y: y / z,
                    button,
                    clicks,
                }
            }
            Input::Resize {
                width,
                height,
                scale,
            } => {
                self.platform_scale = scale;
                Input::Resize {
                    width: width / z,
                    height: height / z,
                    scale: scale * z,
                }
            }
            Input::Files { action, paths, at } => Input::Files {
                action,
                paths,
                at: at.map(|(x, y)| (x / z, y / z)),
            },
            other => other,
        };
        match input {
            Input::Resize {
                width,
                height,
                scale,
            } => {
                self.width = width;
                self.height = height;
                self.scale = scale;
                self.dirty = true;
                self.a11y_dirty = true;
                out.push(self.ev(
                    "resize",
                    vec![
                        ("width", float(width)),
                        ("height", float(height)),
                        ("scale", float(scale)),
                    ],
                ));
            }
            Input::Focused(on) => {
                self.focused_window = on;
                self.dirty = true;
                out.push(self.ev("window_focus", vec![("on", Value::Boolean(on))]));
            }
            Input::Modifiers(m) => self.mods = m,
            Input::Files { action, paths, at } => {
                let (x, y) = at.unwrap_or(self.pointer);
                let target = if action == "cancel" {
                    None
                } else {
                    self.scene.hit(x, y)
                };
                out.push(self.ev(
                    "files",
                    vec![
                        ("action", s(&action)),
                        (
                            "paths",
                            Value::List(Arc::new(paths.iter().map(|p| s(p)).collect())),
                        ),
                        ("x", float(x)),
                        ("y", float(y)),
                        ("target", opt_str(target.as_deref())),
                    ],
                ));
            }
            Input::Key {
                key,
                text,
                mods,
                repeat,
            } => self.key(key, text, mods, repeat, clip, out),
            Input::Pointer {
                action,
                x,
                y,
                button,
                clicks,
            } => self.pointer(action, x, y, &button, clicks, out),
            Input::Wheel { dx, dy, mods } => self.wheel(dx, dy, mods, out),
            Input::Scroll {
                dx,
                dy,
                mods,
                phase,
                precise,
            } => self.scroll(dx, dy, mods, phase, precise, out),
            Input::Pinch { delta, phase } => {
                let (x, y) = self.pointer;
                if let Some(hit) = self.scene.hit(x, y)
                    && let Some(k) = self.scene.target_up(&hit, |n| n.wheel)
                {
                    out.push(self.ev(
                        "pinch",
                        vec![
                            ("key", s(&k)),
                            ("delta", float(delta)),
                            ("phase", s(&phase)),
                            ("x", float(x)),
                            ("y", float(y)),
                        ],
                    ));
                }
            }
            Input::ImePreedit(t, cursor) => {
                if let Some(f) = self.input_focus() {
                    out.push(self.ev(
                        "preedit",
                        vec![
                                ("key", s(&f)),
                                ("text", s(&t)),
                                (
                                    "cursor",
                                    cursor
                                        .map(|c| Value::Integer(c.0 as i64))
                                        .unwrap_or(Value::Unit),
                                ),
                            ],
                    ));
                } else if let Some(f) = self.scene.focus.clone()
                    && let Some(ed) = self.editors.get_mut(&f)
                    && let Ok(mut ts) = text::system().lock()
                {
                    ed.compose(&t, cursor, &mut ts);
                    self.dirty = true;
                }
            }
            Input::ImeCommit(t) => {
                if let Some(f) = self.input_focus() {
                    out.push(self.ev("text", vec![("key", s(&f)), ("text", s(&t))]));
                } else if let Some(f) = self.scene.focus.clone() {
                    let outcome = match (self.editors.get_mut(&f), text::system().lock()) {
                        (Some(ed), Ok(mut ts)) => {
                            ed.compose("", None, &mut ts);
                            Some(ed.insert(&t, &mut ts))
                        }
                        _ => None,
                    };
                    if let Some(o) = outcome {
                        self.after_edit(&f, o, out);
                    }
                }
            }
        }
    }

    fn after_edit(&mut self, key: &str, outcome: Outcome, out: &mut Vec<Value>) {
        match outcome {
            Outcome::Changed => {
                if self.editors[key].rich().is_some() {
                    if let Some(e) = self.rich_changed_event(key) {
                        out.push(e);
                    }
                } else {
                    let ed = &self.editors[key];
                    let (a, f) = ed.selection_chars();
                    out.push(self.changed_event(key, ed.value(), ed.rev(), a, f));
                }
                self.a11y_dirty = true;
            }
            Outcome::Submit => out.push(self.ev("submit", vec![("key", s(key))])),
            Outcome::Moved => {
                self.report_selection(key, out);
                self.a11y_dirty = true;
            }
            Outcome::Pass => {}
        }
        self.dirty = true;
    }

    fn chord(key: &str, mods: Mods) -> String {
        let mut c = String::new();
        if mods.ctrl {
            c.push_str("ctrl+");
        }
        if mods.alt {
            c.push_str("alt+");
        }
        if mods.shift {
            c.push_str("shift+");
        }
        if mods.super_ {
            c.push_str("super+");
        }
        c.push_str(key);
        c
    }

    fn key(
        &mut self,
        key: String,
        text: Option<String>,
        mods: Mods,
        repeat: bool,
        clip: &mut dyn Clipboard,
        out: &mut Vec<Value>,
    ) {
        self.mods = mods;
        let plain = !mods.ctrl && !mods.alt && !mods.super_;
        // keyboard navigation shows where the focus is; a shortcut does not
        if !mods.ctrl && !mods.super_ && !self.scene.focus_visible {
            self.scene.focus_visible = true;
            self.dirty = true;
        }
        // a key the focused field hands to the program: no editing, no focus move
        // (a chord with shift says so: "shift+tab"; never while the input
        // method composes, which owns the keys until it commits)
        let composing = self
            .scene
            .focus
            .as_ref()
            .and_then(|f| self.editors.get(f))
            .is_some_and(|e| e.is_composing());
        let spelled = if mods.shift {
            format!("shift+{key}")
        } else {
            key.clone()
        };
        // a chord with ⌘, ⌃ or ⌥ is said as the commands spell it
        // ("mod+shift+k", "alt+up"): a field hands those over too
        let chorded = {
            let mut m = mods;
            if cfg!(target_os = "macos") {
                m.super_ = false;
            } else {
                m.ctrl = false;
            }
            let base = Self::chord(&key, m);
            if mods.command() {
                format!("mod+{base}")
            } else {
                base
            }
        };
        let passed = !composing
            && self
                .scene
                .focus
                .as_ref()
                .and_then(|f| self.scene.nodes.get(f))
                .and_then(|n| n.edit.as_ref())
                .is_some_and(|e| {
                    e.pass_keys
                        .iter()
                        .any(|k| (plain && *k == spelled) || (!plain && *k == chorded))
                });
        // a node that takes the keyboard as it is (a terminal) keeps Tab
        let raw_input = self
            .scene
            .focus
            .as_ref()
            .and_then(|f| self.scene.nodes.get(f))
            .is_some_and(|n| n.input);
        if key == "tab" && plain && !passed && !raw_input {
            self.move_focus(!mods.shift, out);
            return;
        }
        let focus = self.scene.focus.clone();
        let disabled = focus
            .as_ref()
            .and_then(|f| self.scene.nodes.get(f))
            .is_some_and(|n| n.disabled);
        if let Some(f) = focus.clone()
            && !disabled
            && !passed
            && self.editors.contains_key(&f)
            && let Some(outcome) = self.edit_key(&f, &key, text.as_deref(), mods, clip)
            && outcome != Outcome::Pass
        {
            self.after_edit(&f, outcome, out);
            return;
        }
        // Space and Enter activate a focused control.
        if (key == "space" || key == "enter")
            && plain
            && !repeat
            && let Some(f) = &focus
            && let Some(n) = self.scene.nodes.get(f)
            && scene::activates(&n.role)
            && !n.disabled
            && !self.editors.contains_key(f)
        {
            out.push(self.ev("activate", vec![("key", s(f)), ("source", s("key"))]));
            return;
        }
        // The arrows, page keys, home and end scroll a focused region
        // (a picture at its own size, a long text): the keyboard's way
        // to what the wheel and the trackpad move. At its edge the key
        // goes on to the program.
        if plain
            && let Some(f) = &focus
            && let Some(n) = self.scene.nodes.get(f)
            && n.role == "region"
            && n.scrollable
            && let Some(to) = Self::region_step(&key, mods.shift, n.offset, (n.rect[2], n.rect[3]))
        {
            let before = n.offset;
            let k = f.clone();
            if let Some(n) = self.scene.nodes.get_mut(&k) {
                n.offset = to;
            }
            self.scene.clamp_scroll(&k);
            let after = self.scene.nodes[&k].offset;
            if after != before {
                self.dirty = true;
                self.a11y_dirty = true;
                out.push(self.ev(
                    "scrolled",
                    vec![("key", s(&k)), ("x", float(after.0)), ("y", float(after.1))],
                ));
                return;
            }
        }
        let mod_chord = {
            let mut m = mods;
            if cfg!(target_os = "macos") {
                m.super_ = false;
            } else {
                m.ctrl = false;
            }
            let base = Self::chord(&key, m);
            if mods.command() {
                format!("mod+{base}")
            } else {
                base
            }
        };
        out.push(self.ev(
            "key",
            vec![
                ("key", s(&key)),
                ("text", opt_str(text.as_deref())),
                ("chord", s(&Self::chord(&key, mods))),
                ("mod_chord", s(&mod_chord)),
                ("mods", mods.value()),
                ("repeat", Value::Boolean(repeat)),
                ("target", opt_str(focus.as_deref())),
            ],
        ));
    }

    /// Where a key moves a region's scroll from `at` (its box `size`):
    /// 40 pixels an arrow, a page less a line, to either end; `None`
    /// for a key that does not scroll.
    fn region_step(key: &str, shift: bool, at: (f32, f32), size: (f32, f32)) -> Option<(f32, f32)> {
        let line = 40.0;
        let page = (size.1 - line).max(line);
        let (x, y) = at;
        Some(match key {
            "up" => (x, y - line),
            "down" => (x, y + line),
            "left" => (x - line, y),
            "right" => (x + line, y),
            "pageup" => (x, y - page),
            "pagedown" => (x, y + page),
            "space" if shift => (x, y - page),
            "space" => (x, y + page),
            "home" => (0.0, 0.0),
            "end" => (x, f32::MAX / 4.0),
            _ => return None,
        })
    }

    /// A key in a focused text field. `None` when the field does not
    /// take it.
    fn edit_key(
        &mut self,
        f: &str,
        key: &str,
        text: Option<&str>,
        mods: Mods,
        clip: &mut dyn Clipboard,
    ) -> Option<Outcome> {
        let mut ts = text::system().lock().ok()?;
        let ed = self.editors.get_mut(f)?;
        if ed.is_composing() {
            // The input method owns the keys until it commits.
            return Some(Outcome::Moved);
        }
        // ⌥⌘ with a key that is not a move is the program's (⌥⌘↩, ⌥⌘F):
        // never an edit of the field's
        if mods.command()
            && mods.alt
            && !matches!(
                key,
                "left" | "right" | "up" | "down" | "backspace" | "delete"
            )
        {
            return Some(Outcome::Pass);
        }
        if mods.command() && !mods.alt {
            return Some(match (key, mods.shift) {
                ("a", false) => {
                    ed.select_all(&mut ts);
                    Outcome::Moved
                }
                ("c", false) => {
                    if let Some(t) = ed.selected_text() {
                        clip.set(t);
                    }
                    Outcome::Moved
                }
                ("x", false) => match ed.selected_text() {
                    Some(t) => {
                        clip.set(t);
                        ed.delete_selection(&mut ts)
                    }
                    None => Outcome::Moved,
                },
                ("v", false) => match clip.get() {
                    Some(t) => ed.insert(&t, &mut ts),
                    None => Outcome::Moved,
                },
                ("z" | "y", _) if !ed.own_undo() => Outcome::Pass,
                ("z", false) => ed.undo(&mut ts),
                ("z", true) | ("y", false) => ed.redo(&mut ts),
                ("left" | "right" | "up" | "down" | "backspace" | "delete", _) => {
                    ed.key(key, mods.shift, mods.word(), mods.line(), &mut ts)
                }
                // ctrl+home and ctrl+end: the text's start and end, as
                // Windows and Linux fields do (macOS has ⌘↑ and ⌘↓)
                ("home" | "end", _) if !cfg!(target_os = "macos") => {
                    let to = if key == "end" { "down" } else { "up" };
                    ed.key(to, mods.shift, false, true, &mut ts)
                }
                _ => Outcome::Pass,
            });
        }
        let outcome = ed.key(key, mods.shift, mods.word(), mods.line(), &mut ts);
        if outcome != Outcome::Pass {
            return Some(outcome);
        }
        // Printable text: no ctrl or command (option makes characters on
        // macOS, so it is allowed).
        if let Some(t) = text
            && !mods.ctrl
            && !mods.super_
            && !t.chars().any(|c| c.is_control())
            && !t.is_empty()
        {
            return Some(ed.insert(t, &mut ts));
        }
        Some(Outcome::Pass)
    }

    fn pointer(
        &mut self,
        action: PointerAction,
        x: f32,
        y: f32,
        button: &str,
        clicks: u32,
        out: &mut Vec<Value>,
    ) {
        self.pointer = (x, y);
        let hit = if action == PointerAction::Leave {
            None
        } else {
            self.scene.hit(x, y)
        };
        // Hover follows the nearest node with a hover variant, or that
        // activates.
        let hover = hit.as_ref().and_then(|h| {
            self.scene.target_up(h, |n| {
                n.variants[scene::HOVER].is_some() || scene::activates(&n.role)
            })
        });
        if hover != self.scene.hover {
            self.scene.hover = hover;
            self.dirty = true;
        }
        // An animation's button: a press plays or pauses it, and neither
        // the press nor its release is the program's.
        if action == PointerAction::Down && button == "left" && self.headless && self.dirty {
            // a headless window draws when read: what the press lands on
            // is what it would show now
            let _ = self.display_list();
        }
        if action == PointerAction::Down
            && button == "left"
            && let Some(k) = self.badge_at(x, y, hit.as_deref())
        {
            self.toggle_play(&k);
            self.swallow_up = true;
            return;
        }
        if self.bar_pointer(action, x, y, button, hit.as_deref(), out) {
            return;
        }
        if action == PointerAction::Up && self.swallow_up {
            self.swallow_up = false;
            return;
        }
        // the pointer over a playing animation shows its pause button
        if action == PointerAction::Move
            && self
                .shown_anims
                .iter()
                .any(|a| a.badge.is_some() && !a.paused)
        {
            self.dirty = true;
        }
        match action {
            PointerAction::Move => {
                if let Some(p) = self.scene.pressed.clone()
                    && self.editors.get(&p).is_some_and(|e| e.dragging())
                    && let Some((ox, oy, _, h)) = self.edit_origin(&p)
                    && let Ok(mut ts) = text::system().lock()
                {
                    let dy = self.edit_text_dy(&p, h, &mut ts);
                    let ed = self.editors.get_mut(&p).expect("present");
                    let (sx, sy) = ed.scrolled();
                    ed.drag(
                        x * self.scale - ox + sx,
                        y * self.scale - oy - dy + sy,
                        &mut ts,
                    );
                    drop(ts);
                    self.report_selection(&p, out);
                    self.dirty = true;
                }
                self.hover_field(hit.as_deref(), x, y, out);
                out.push(self.ev(
                    "pointer",
                    vec![
                        ("action", s("move")),
                        ("x", float(x)),
                        ("y", float(y)),
                        ("mods", self.mods.value()),
                        ("sx", opt_float(self.screen_pointer.map(|p| p.0))),
                        ("sy", opt_float(self.screen_pointer.map(|p| p.1))),
                        ("target", opt_str(hit.as_deref())),
                        ("hit", self.canvas_hit(hit.as_deref(), x, y)),
                    ],
                ));
            }
            PointerAction::Leave => {
                self.hover_field(None, x, y, out);
                out.push(self.ev("pointer", vec![("action", s("leave"))]));
            }
            PointerAction::Down => {
                // a press: the focus follows the pointer without a ring
                self.scene.focus_visible = false;
                // A press outside the modal layer: the layer hears of it
                // (a popover or a menu closes; a dialog may ignore it).
                if hit.is_none()
                    && let Some(m) = self.scene.modal_root()
                {
                    out.push(self.ev(
                        "outside",
                        vec![("key", s(&m)), ("x", float(x)), ("y", float(y))],
                    ));
                }
                let clicks = if clicks > 0 {
                    clicks
                } else {
                    let now = std::time::Instant::now();
                    let n = match self.last_press {
                        Some((t, px, py, n))
                            if now.duration_since(t).as_millis() < 500
                                && (px - x).abs() < 4.0
                                && (py - y).abs() < 4.0 =>
                        {
                            n + 1
                        }
                        _ => 1,
                    };
                    self.last_press = Some((now, x, y, n));
                    n
                };
                let target = hit.as_ref().and_then(|h| {
                    self.scene.target_up(h, |n| {
                        !n.disabled && (scene::activates(&n.role) || n.focusable)
                    })
                });
                // a right press in a field: the focus and the caret go
                // there (kept in a selection pressed inside), as a Mac's
                // text views do before their context menu opens
                if button == "right"
                    && let Some(f) = hit
                        .as_ref()
                        .and_then(|h| self.scene.target_up(h, |n| n.focusable && !n.disabled))
                    && self.editors.contains_key(&f)
                    && let Some((ox, oy, _, h)) = self.edit_origin(&f)
                    && let Ok(mut ts) = text::system().lock()
                {
                    let dy = self.edit_text_dy(&f, h, &mut ts);
                    let ed = self.editors.get_mut(&f).expect("present");
                    let (sx, sy) = ed.scrolled();
                    ed.context_press(
                        x * self.scale - ox + sx,
                        y * self.scale - oy - dy + sy,
                        &mut ts,
                    );
                    drop(ts);
                    self.set_focus(Some(f.clone()), out);
                    self.report_selection(&f, out);
                    self.dirty = true;
                }
                if button == "left" {
                    self.scene.pressed = target.clone();
                    let focus_to = hit
                        .as_ref()
                        .and_then(|h| self.scene.target_up(h, |n| n.focusable && !n.disabled));
                    if focus_to.is_some() {
                        self.set_focus(focus_to.clone(), out);
                    }
                    if let Some(f) = &focus_to
                        && self.editors.contains_key(f)
                        && let Some((ox, oy, _, h)) = self.edit_origin(f)
                        && let Ok(mut ts) = text::system().lock()
                    {
                        let dy = self.edit_text_dy(f, h, &mut ts);
                        let shift = self.mods.shift;
                        let cmd = self.mods.command();
                        let ed = self.editors.get_mut(f).expect("present");
                        // a press with the command key is said even where
                        // the caret already was (⌘-click: go to definition)
                        if cmd && let Some(r) = ed.rich_mut() {
                            r.said_selection = (usize::MAX, usize::MAX);
                        }
                        let (sx, sy) = ed.scrolled();
                        let (tx, ty) = (x * self.scale - ox + sx, y * self.scale - oy - dy + sy);
                        // a press on a pinned scope's first line (sticky
                        // scroll) is the program's: said as a lens of id
                        // "sticky" on that line, the caret left where it was
                        let pinned = ed.rich_mut().and_then(|r| {
                            let vy = ty - sy;
                            r.sticky_drawn
                                .iter()
                                .find(|(y0, y1, _)| vy >= *y0 && vy < *y1)
                                .map(|d| d.2)
                        });
                        // a press in the changes' lane, on a line: the
                        // program's (said as a lens of id "change" on that
                        // line), the caret left where it was
                        let change = if pinned.is_some()
                            || tx >= 0.0
                            || tx < -super::rich::CHANGE_LANE_HIT * self.scale
                        {
                            None
                        } else {
                            ed.rich_mut().and_then(|r| {
                                if r.gutter
                                    .as_ref()
                                    .map(|g| g.changes.is_empty())
                                    .unwrap_or(true)
                                {
                                    return None;
                                }
                                r.gutter_line_at(ty, &mut ts).map(|(line, _)| line)
                            })
                        };
                        let lens = if pinned.is_some() || change.is_some() {
                            None
                        } else {
                            ed.rich_mut().and_then(|r| {
                                let (k, ly) = r.lens_at(ty, &mut ts)?;
                                let l = r.lenses()[k].clone();
                                Some((l, ly / self.scale))
                            })
                        };
                        if let Some(line) = change {
                            drop(ts);
                            let f = f.clone();
                            out.push(self.ev(
                                "lens",
                                vec![
                                    ("key", s(&f)),
                                    ("line", Value::Integer(line as i64 + 1)),
                                    ("id", s("change")),
                                    ("hit", Value::Unit),
                                    ("x", float(tx / self.scale)),
                                    ("y", float(0.0)),
                                    ("clicks", Value::Integer(clicks as i64)),
                                ],
                            ));
                        } else if let Some(line) = pinned {
                            drop(ts);
                            let f = f.clone();
                            out.push(self.ev(
                                "lens",
                                vec![
                                    ("key", s(&f)),
                                    ("line", Value::Integer(line as i64 + 1)),
                                    ("id", s("sticky")),
                                    ("hit", Value::Unit),
                                    ("x", float(tx / self.scale)),
                                    ("y", float(0.0)),
                                    ("clicks", Value::Integer(clicks as i64)),
                                ],
                            ));
                        } else if let Some((l, ly)) = lens {
                            drop(ts);
                            let lx = tx / self.scale;
                            let width = self.scene.nodes.get(f).map(|n| n.rect[2]).unwrap_or(0.0);
                            let hit = super::canvas::hit(&l.ops, width, l.height, lx, ly);
                            let f = f.clone();
                            out.push(self.ev(
                                "lens",
                                vec![
                                    ("key", s(&f)),
                                    ("line", Value::Integer(l.line as i64 + 1)),
                                    ("id", l.id.clone()),
                                    ("hit", hit),
                                    ("x", float(lx)),
                                    ("y", float(ly)),
                                    ("clicks", Value::Integer(clicks as i64)),
                                ],
                            ));
                        } else {
                            self.click_mod = cmd;
                            self.click_count = clicks;
                            ed.press(tx, ty, clicks, shift, &mut ts);
                            drop(ts);
                            let f = f.clone();
                            self.report_selection(&f, out);
                        }
                    }
                    self.dirty = true;
                }
                out.push(self.ev(
                    "pointer",
                    vec![
                        ("action", s("down")),
                        ("x", float(x)),
                        ("y", float(y)),
                        ("mods", self.mods.value()),
                        ("sx", opt_float(self.screen_pointer.map(|p| p.0))),
                        ("sy", opt_float(self.screen_pointer.map(|p| p.1))),
                        ("button", s(button)),
                        ("clicks", Value::Integer(clicks as i64)),
                        ("target", opt_str(hit.as_deref())),
                        ("hit", self.canvas_hit(hit.as_deref(), x, y)),
                    ],
                ));
            }
            PointerAction::Up => {
                let pressed = self.scene.pressed.take();
                if let Some(p) = &pressed
                    && let Some(ed) = self.editors.get_mut(p)
                {
                    ed.stop_drag();
                }
                out.push(self.ev(
                    "pointer",
                    vec![
                        ("action", s("up")),
                        ("x", float(x)),
                        ("y", float(y)),
                        ("mods", self.mods.value()),
                        ("sx", opt_float(self.screen_pointer.map(|p| p.0))),
                        ("sy", opt_float(self.screen_pointer.map(|p| p.1))),
                        ("button", s(button)),
                        ("target", opt_str(hit.as_deref())),
                        ("hit", self.canvas_hit(hit.as_deref(), x, y)),
                    ],
                ));
                // A click: pressed and released on the same activating node.
                if button == "left"
                    && let Some(p) = pressed
                {
                    let still = hit.as_ref().and_then(|h| {
                        self.scene.target_up(h, |n| {
                            !n.disabled && (scene::activates(&n.role) || n.focusable)
                        })
                    });
                    if still.as_deref() == Some(p.as_str())
                        && self
                            .scene
                            .nodes
                            .get(&p)
                            .is_some_and(|n| scene::activates(&n.role))
                    {
                        out.push(
                            self.ev("activate", vec![("key", s(&p)), ("source", s("pointer"))]),
                        );
                    }
                }
                self.dirty = true;
            }
        }
    }

    fn wheel(&mut self, dx: f32, dy: f32, mods: Option<Mods>, out: &mut Vec<Value>) {
        let (x, y) = self.pointer;
        let Some(hit) = self.scene.hit(x, y) else {
            return;
        };
        // A styled field scrolls its own text, until it is at an end.
        if let Some(k) = self
            .scene
            .target_up(&hit, |n| n.edit.as_ref().is_some_and(|e| e.rich))
            && let Ok(mut ts) = text::system().lock()
            && let Some(r) = self.editors.get_mut(&k).and_then(|e| e.rich_mut())
            && (r.wheel_x(dx) | r.wheel(dy, &mut ts))
        {
            let now = self
                .test_clock
                .unwrap_or_else(|| self.born.elapsed().as_secs_f64() * 1000.0);
            self.bars.entry(k.clone()).or_default().moved_at = Some(now);
            // the first line in view, said when it moves a band of lines
            // (a program asks for what the band shows; not every wheel)
            let top = r.top_line(&mut ts);
            let band = r.band.max(1);
            if r.said_top.map(|t| t / band) != Some(top / band) {
                r.said_top = Some(top);
                drop(ts);
                out.push(self.ev(
                    "viewport",
                    vec![("key", s(&k)), ("top", Value::Integer(top as i64))],
                ));
            }
            self.dirty = true;
            return;
        }
        // A node that hears the wheel itself (a canvas that pans and
        // zooms) takes it before any scroller around it.
        if let Some(k) = self.scene.target_up(&hit, |n| n.wheel) {
            let m = mods.unwrap_or(self.mods);
            out.push(self.ev(
                "wheel",
                vec![
                    ("key", s(&k)),
                    ("dx", float(dx)),
                    ("dy", float(dy)),
                    ("x", float(x)),
                    ("y", float(y)),
                    ("mod", Value::Boolean(m.command())),
                    ("shift", Value::Boolean(m.shift)),
                    ("alt", Value::Boolean(m.alt)),
                    ("ctrl", Value::Boolean(m.ctrl)),
                ],
            ));
            return;
        }
        // Shift turns a mouse wheel's turn sideways (AppKit already does
        // for its own events: then `dx` is set and this does nothing).
        let (dx, dy) = if mods.unwrap_or(self.mods).shift && dx == 0.0 && dy != 0.0 {
            (dy, 0.0)
        } else {
            (dx, dy)
        };
        // A node that hears only the sideways part (`wheel_x`: a diff's
        // lines scrolling across together) takes it; down goes on to the
        // scrollers around it.
        let dx = match self.scene.target_up(&hit, |n| n.wheel_x) {
            Some(k) => {
                if dx != 0.0 {
                    let m = mods.unwrap_or(self.mods);
                    out.push(self.ev(
                        "wheel",
                        vec![
                            ("key", s(&k)),
                            ("dx", float(dx)),
                            ("dy", float(0.0)),
                            ("x", float(x)),
                            ("y", float(y)),
                            ("mod", Value::Boolean(m.command())),
                            ("shift", Value::Boolean(m.shift)),
                            ("alt", Value::Boolean(m.alt)),
                            ("ctrl", Value::Boolean(m.ctrl)),
                        ],
                    ));
                }
                0.0
            }
            None => dx,
        };
        // Each axis goes to the nearest scroller that can still move
        // along it: two fingers across a board's column (which scrolls
        // down) move the board sideways, and a diagonal moves both.
        let (mut rx, mut ry) = (dx, dy);
        let mut at = self.scene.scroller_up(&hit);
        while let Some(k) = at {
            if rx == 0.0 && ry == 0.0 {
                break;
            }
            let before = self.scene.nodes[&k].offset;
            {
                let n = self.scene.nodes.get_mut(&k).expect("present");
                n.offset.0 -= rx;
                n.offset.1 -= ry;
            }
            self.scene.clamp_scroll(&k);
            let after = self.scene.nodes[&k].offset;
            if after != before {
                self.dirty = true;
                self.a11y_dirty = true;
                if after.1 != before.1 {
                    self.bar_moved(&k);
                }
                out.push(self.ev(
                    "scrolled",
                    vec![("key", s(&k)), ("x", float(after.0)), ("y", float(after.1))],
                ));
                // what this one took is spent; the rest goes on out
                if after.0 != before.0 {
                    rx = 0.0;
                }
                if after.1 != before.1 {
                    ry = 0.0;
                }
            }
            // Nothing (more) to scroll here: hand the rest to the next
            // scroller out.
            at = self.scene.nodes[&k]
                .parent
                .as_ref()
                .and_then(|p| self.scene.scroller_up(p));
        }
    }

    /// What a scroll in its gesture moves (scroll.rs): the styled field or
    /// the scroll region under the pointer — none when a node there hears
    /// the wheel itself — and whether it is a field.
    fn elastic_target(&self) -> Option<(String, bool)> {
        let (x, y) = self.pointer;
        let hit = self.scene.hit(x, y)?;
        if let Some(k) = self
            .scene
            .target_up(&hit, |n| n.edit.as_ref().is_some_and(|e| e.rich))
            && self.editors.get(&k).is_some_and(|e| e.rich().is_some())
        {
            return Some((k, true));
        }
        if self.scene.target_up(&hit, |n| n.wheel).is_some() {
            return None;
        }
        self.scene.scroller_up(&hit).map(|k| (k, false))
    }

    /// How far a scroll region's or field's content shows moved past its
    /// ends now (logical px), for a view `dims`.
    pub fn stretch_of(&self, key: &str, dims: [f32; 2]) -> [f32; 2] {
        match self.elastic.get(key) {
            Some(e) => e.shown(self.clock_ms(), dims).0,
            None => [0.0, 0.0],
        }
    }

    /// A region's view and how far it scrolls along each axis.
    fn region_reach(&self, key: &str) -> Option<([f32; 2], [f32; 2])> {
        let n = self.scene.nodes.get(key)?;
        let (cw, ch) = self.scene.content_size(n);
        Some((
            [n.rect[2], n.rect[3]],
            [(cw - n.rect[2]).max(0.0), (ch - n.rect[3]).max(0.0)],
        ))
    }

    /// A scroll in its gesture (scroll.rs). A trackpad's moves the region
    /// or field under the pointer along the axes it scrolls (the others go
    /// on out, as the wheel's do), and past an end pulls against a rubber
    /// band that springs back as the fingers lift; momentum carries on to
    /// an end and bounces. A mouse wheel's notch glides. One with no
    /// phase is the wheel's.
    fn scroll(
        &mut self,
        dx: f32,
        dy: f32,
        mods: Option<Mods>,
        phase: super::scroll::Phase,
        precise: bool,
        out: &mut Vec<Value>,
    ) {
        use super::scroll::Phase;
        let now = self.clock_ms();
        if !precise && phase == Phase::Legacy {
            if !self.glide(dx, dy, mods, now) {
                self.wheel(dx, dy, mods, out);
            }
            return;
        }
        let target = if phase == Phase::Legacy {
            None
        } else {
            self.elastic_target()
        };
        let Some((key, rich)) = target else {
            self.wheel(dx, dy, mods, out);
            return;
        };
        // the field's view, and the axes it scrolls
        let (dims, own) = if rich {
            let sc = self.scale.max(0.01);
            let w = self.scene.nodes.get(&key).map_or(1.0, |n| n.rect[2]);
            let Ok(mut ts) = text::system().lock() else {
                return self.wheel(dx, dy, mods, out);
            };
            let Some(r) = self.editors.get_mut(&key).and_then(|e| e.rich_mut()) else {
                drop(ts);
                return self.wheel(dx, dy, mods, out);
            };
            let max = r.max_scroll(r.view_h, &mut ts);
            ([w, r.view_h / sc], [false, max > 0.5])
        } else {
            let Some((dims, max)) = self.region_reach(&key) else {
                return self.wheel(dx, dy, mods, out);
            };
            (dims, [max[0] > 0.5, max[1] > 0.5])
        };
        // shift turns a wheel's turn sideways (AppKit does for its own)
        let (dx, dy) = if mods.unwrap_or(self.mods).shift && dx == 0.0 && dy != 0.0 && !own[1] {
            (dy, 0.0)
        } else {
            (dx, dy)
        };
        let mut e = self.elastic.remove(&key).unwrap_or_default();
        let was = e.shown(now, dims).0;
        let mut spent = false;
        match phase {
            Phase::Began => e.take_hold(now, dims),
            // momentum after a bounce (or after a stretch let go) is spent
            p if p.momentum() && e.bounced => spent = true,
            _ => {}
        }
        if phase == Phase::MomentumEnded {
            e.bounced = false;
        }
        let mut rest = [0.0f32; 2];
        let mut others = [dx, dy];
        if !spent {
            let d = [dx, dy];
            for i in 0..2 {
                if !own[i] {
                    continue;
                }
                others[i] = 0.0;
                rest[i] = e.relax(i, d[i]);
            }
            // the axes it scrolls: its own offset
            if rest != [0.0, 0.0] {
                rest = if rich {
                    self.scroll_field_by(&key, rest[1], out)
                } else {
                    self.scroll_region_by(&key, rest, out)
                };
            }
        } else {
            others = [0.0, 0.0];
        }
        // what is left past an end
        if phase.gesture() && phase != Phase::Ended {
            for i in 0..2 {
                if own[i] {
                    e.pull[i] += rest[i];
                }
            }
        }
        if phase.momentum() && !e.bounced && rest != [0.0, 0.0] {
            // arriving at an end at the momentum's speed: out and back
            let dt = ((now - e.last_at) as f32).clamp(4.0, 34.0);
            let v = [
                if rest[0] != 0.0 {
                    dx * 1000.0 / dt
                } else {
                    0.0
                },
                if rest[1] != 0.0 {
                    dy * 1000.0 / dt
                } else {
                    0.0
                },
            ];
            e.release(now, dims, v);
            e.bounced = true;
        }
        if phase == Phase::Ended {
            e.gesture = false;
            if e.pull != [0.0, 0.0] {
                e.release(now, dims, [0.0, 0.0]);
                e.bounced = true;
            }
        }
        e.last_at = now;
        if e.shown(now, dims).0 != was {
            self.dirty = true;
        }
        if !e.idle(now, dims) || e.bounced {
            self.elastic.insert(key, e);
        }
        // the axes it does not scroll go on out, as the wheel's do
        if others != [0.0, 0.0] {
            self.wheel(others[0], others[1], mods, out);
        }
    }

    /// Scroll region `key` by `d` (the wheel's sign: positive towards its
    /// start), said as the wheel's are: what could not move (past an end).
    fn scroll_region_by(&mut self, key: &str, d: [f32; 2], out: &mut Vec<Value>) -> [f32; 2] {
        let Some(n) = self.scene.nodes.get_mut(key) else {
            return d;
        };
        let before = n.offset;
        n.offset.0 -= d[0];
        n.offset.1 -= d[1];
        self.scene.clamp_scroll(key);
        let after = self.scene.nodes[key].offset;
        if after != before {
            self.dirty = true;
            self.a11y_dirty = true;
            if after.1 != before.1 {
                self.bar_moved(key);
            }
            out.push(self.ev(
                "scrolled",
                vec![
                    ("key", s(key)),
                    ("x", float(after.0)),
                    ("y", float(after.1)),
                ],
            ));
        }
        [d[0] - (before.0 - after.0), d[1] - (before.1 - after.1)]
    }

    /// Scroll styled field `key` down by `dy` (the wheel's sign): what
    /// could not move.
    fn scroll_field_by(&mut self, key: &str, dy: f32, out: &mut Vec<Value>) -> [f32; 2] {
        let now = self.clock_ms();
        let sc = self.scale.max(0.01);
        let Ok(mut ts) = text::system().lock() else {
            return [0.0, dy];
        };
        let Some(r) = self.editors.get_mut(key).and_then(|e| e.rich_mut()) else {
            return [0.0, dy];
        };
        let before = r.scroll_y;
        if !r.wheel(dy, &mut ts) {
            return [0.0, dy];
        }
        let moved = (before - r.scroll_y) / sc;
        self.bars.entry(key.to_string()).or_default().moved_at = Some(now);
        let top = r.top_line(&mut ts);
        let band = r.band.max(1);
        if r.said_top.map(|t| t / band) != Some(top / band) {
            r.said_top = Some(top);
            drop(ts);
            out.push(self.ev(
                "viewport",
                vec![("key", s(key)), ("top", Value::Integer(top as i64))],
            ));
        }
        self.dirty = true;
        [0.0, dy - moved]
    }

    /// A mouse wheel's notch over a scroll region that moves along it: the
    /// region glides there (scroll.rs) rather than jumping; notches in a
    /// row carry the glide on. `false` when no region takes it.
    fn glide(&mut self, dx: f32, dy: f32, mods: Option<Mods>, now: f64) -> bool {
        let Some((key, false)) = self.elastic_target() else {
            return false;
        };
        let (dx, dy) = if mods.unwrap_or(self.mods).shift && dx == 0.0 && dy != 0.0 {
            (dy, 0.0)
        } else {
            (dx, dy)
        };
        let Some((_, max)) = self.region_reach(&key) else {
            return false;
        };
        if (dx != 0.0 && max[0] <= 0.5) || (dy != 0.0 && max[1] <= 0.5) || (dx == 0.0 && dy == 0.0)
        {
            return false;
        }
        let off = self.scene.nodes[&key].offset;
        let e = self.elastic.entry(key).or_default();
        let (from, aim) = match e.glide {
            Some(g) => (g.at(now).0, g.to),
            None => ([off.0, off.1], [off.0, off.1]),
        };
        let to = [
            (aim[0] - dx).clamp(0.0, max[0]),
            (aim[1] - dy).clamp(0.0, max[1]),
        ];
        // at its end already: the notch is spent here (no jump past it)
        if to != from {
            e.glide = Some(super::scroll::Glide {
                from,
                to,
                start: now,
            });
            self.dirty = true;
        }
        true
    }

    /// Before a frame: the regions gliding move on (each step said, as the
    /// wheel's are), and what has settled is let go.
    pub fn tick(&mut self, out: &mut Vec<Value>) {
        if self.elastic.is_empty() {
            return;
        }
        let now = self.clock_ms();
        let keys: Vec<String> = self.elastic.keys().cloned().collect();
        for k in keys {
            let sc = self.scale.max(0.01);
            let field = self
                .editors
                .get(&k)
                .and_then(|e| e.rich())
                .map(|r| r.view_h / sc);
            let reach = match field {
                Some(h) => self
                    .scene
                    .nodes
                    .get(&k)
                    .map(|n| ([n.rect[2], h], [0.0, 0.0])),
                None => self.region_reach(&k),
            };
            let Some(reach) = reach else {
                self.elastic.remove(&k);
                continue;
            };
            let mut e = self.elastic[&k];
            if let Some(g) = e.glide {
                let (v, done) = g.at(now);
                if done {
                    e.glide = None;
                }
                if !self.editors.contains_key(&k) {
                    let off = self.scene.nodes[&k].offset;
                    let d = [off.0 - v[0], off.1 - v[1]];
                    if d != [0.0, 0.0] {
                        self.scroll_region_by(&k, d, out);
                    }
                }
            }
            if e.idle(now, reach.0) && !e.bounced {
                self.elastic.remove(&k);
            } else {
                // a bounce settled lets the rest of its momentum go too,
                // once nothing has come for a while
                if e.bounced && e.idle(now, reach.0) && now - e.last_at > 250.0 {
                    self.elastic.remove(&k);
                    continue;
                }
                self.elastic.insert(k, e);
            }
        }
    }

    /// Something of the scrolling moves at the next refresh (a glide, a
    /// spring back).
    fn elastic_moving(&self) -> bool {
        let now = self.clock_ms();
        self.elastic.iter().any(|(k, e)| {
            e.glide.is_some()
                || (e.back.is_some() && {
                    let dims = self
                        .scene
                        .nodes
                        .get(k)
                        .map(|n| [n.rect[2], n.rect[3]])
                        .unwrap_or([1.0, 1.0]);
                    e.shown(now, dims).1
                })
        })
    }

    /// Where the input method should place its candidate window: the
    /// focused field's composition or caret, in logical pixels.
    pub fn ime_area(&mut self) -> Option<[f32; 4]> {
        if let Some(f) = self.input_focus() {
            let abs = self.scene.absolute(&f)?;
            let a = self
                .scene
                .nodes
                .get(&f)?
                .ime_area
                .unwrap_or([0.0, 0.0, 1.0, abs[3].min(20.0)]);
            return Some([abs[0] + a[0], abs[1] + a[1], a[2].max(1.0), a[3].max(1.0)]);
        }
        let f = self.scene.focus.clone()?;
        let (ox, oy, _, h) = self.edit_origin(&f)?;
        let mut ts = text::system().lock().ok()?;
        let dy = self.edit_text_dy(&f, h, &mut ts);
        let s = self.platform_scale.max(0.01);
        if let Some(r) = self.editors.get_mut(&f)?.rich_mut() {
            let b = r.ime_box(&mut ts);
            return Some([
                (ox + b.x0 as f32) / s,
                (oy + b.y0 as f32 - r.scroll_y) / s,
                ((b.x1 - b.x0) as f32).max(1.0) / s,
                ((b.y1 - b.y0) as f32) / s,
            ]);
        }
        let ed = self.editors.get_mut(&f)?.plain_mut()?;
        ed.layout(&mut ts);
        let b = ed.ed.ime_cursor_area();
        // physical pixels to the platform's logical ones
        Some([
            (ox + b.x0 as f32 - ed.scroll_x) / s,
            (oy + dy + b.y0 as f32) / s,
            ((b.x1 - b.x0) as f32) / s,
            ((b.y1 - b.y0) as f32) / s,
        ])
    }

    /// Whether the focused node is a text field, or a node that takes the
    /// keyboard as it is (the input method is allowed only then).
    pub fn wants_ime(&self) -> bool {
        self.scene
            .focus
            .as_ref()
            .is_some_and(|f| self.editors.contains_key(f))
            || self.input_focus().is_some()
    }

    /// The focused node when it takes the keyboard as it is (`input`).
    fn input_focus(&self) -> Option<String> {
        let f = self.scene.focus.as_ref()?;
        if self.editors.contains_key(f) {
            return None;
        }
        self.scene
            .nodes
            .get(f)
            .filter(|n| n.input)
            .map(|_| f.clone())
    }

    // ── the display list ────────────────────────────────────────────

    /// The size a frame is drawn at, device pixels (`display_list`'s).
    pub fn device_size(&self) -> (u32, u32) {
        let s = self.scale;
        (
            (self.width * s).round().max(1.0) as u32,
            (self.height * s).round().max(1.0) as u32,
        )
    }

    pub fn display_list(&mut self) -> DisplayList {
        let s = self.scale;
        let pw = (self.width * s).round().max(1.0) as u32;
        let ph = (self.height * s).round().max(1.0) as u32;
        let mut dl = DisplayList {
            width: pw,
            height: ph,
            clear: self.clear,
            prims: Vec::new(),
            scrolls: Vec::new(),
        };
        self.caret = None;
        self.shown_anims.clear();
        self.next_frame = None;
        self.bar_hits.clear();
        let Some(root) = self.scene.root.clone() else {
            return dl;
        };
        let Ok(mut ts) = text::system().lock() else {
            return dl;
        };
        let full = Clip::rect(0.0, 0.0, pw as f32, ph as f32);
        let mut rings = Vec::new();
        self.motion.begin();
        self.paint(&root, 0.0, 0.0, full, 1.0, &mut ts, &mut dl, &mut rings);
        self.paint_ghosts(full, &mut ts, &mut dl, &mut rings);
        self.motion.end();
        // a transition moving, a scroll gliding or springing back: the
        // next frame at the next refresh
        if self.motion.moving() || self.elastic_moving() {
            let due = self.clock_ms() + 1.0;
            self.next_frame = Some(self.next_frame.map_or(due, |d| d.min(due)));
        }
        // Focus rings draw last, over their neighbours, unclipped by the
        // node's own clip but within the window.
        dl.prims.extend(rings);
        dl
    }

    /// Node `key` and what it holds: as its transition shows it now, when
    /// it has one (motion.rs).
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        key: &str,
        ox: f32,
        oy: f32,
        clip: Clip,
        opacity: f32,
        ts: &mut TextSystem,
        dl: &mut DisplayList,
        rings: &mut Vec<Prim>,
    ) {
        let Some(node) = self.scene.nodes.get(key) else {
            return;
        };
        let mut st = self.scene.effective_style(node);
        let rect = node.rect;
        let Some(tr) = st.transition.clone().filter(|_| !self.motion.ghosting) else {
            self.paint_node(key, ox, oy, clip, opacity, st, rect, ts, dl, rings);
            return;
        };
        // still (reduced motion, a still headless window): drawn as it
        // is, nothing kept
        let runs = self.motion.runs(self.settings.reduce_motion);
        if !runs {
            if !self.motion.records.is_empty() {
                self.motion.records.clear();
            }
            self.paint_node(key, ox, oy, clip, opacity, st, rect, ts, dl, rings);
            return;
        }
        let now = self.clock_ms();
        let hovered = self
            .scene
            .hover
            .as_deref()
            .is_some_and(|h| h == key || (st.group_hover && self.scene.is_ancestor(h, key)));
        let target = super::motion::Target {
            rect,
            opacity: st.opacity,
            bg: st.bg,
            border: st.border,
            color: st.color,
            hovered,
        };
        let sh = self.motion.drive(key, &tr, &target, now, runs);
        st.opacity = sh.opacity;
        st.bg = sh.bg;
        st.border = sh.border;
        st.color = sh.color;
        self.paint_posed(
            key, ox, oy, clip, opacity, st, sh.rect, sh.pose, ts, dl, rings,
        );
    }

    /// A node drawn in a pose: faded, moved, and scaled about its centre
    /// (drawn at that scale, so its text is sharp: the scale is kept to
    /// hundredths, so a scale's frames share their glyphs).
    #[allow(clippy::too_many_arguments)]
    fn paint_posed(
        &mut self,
        key: &str,
        ox: f32,
        oy: f32,
        clip: Clip,
        opacity: f32,
        st: scene::Style,
        rect: [f32; 4],
        pose: super::motion::Pose,
        ts: &mut TextSystem,
        dl: &mut DisplayList,
        rings: &mut Vec<Prim>,
    ) {
        let opacity = opacity * pose.opacity;
        let (mut ox, mut oy) = (ox + pose.x, oy + pose.y);
        let k = (pose.scale * 100.0).round() / 100.0;
        if (k - 1.0).abs() < 1e-3 || k <= 0.0 {
            self.paint_node(key, ox, oy, clip, opacity, st, rect, ts, dl, rings);
            return;
        }
        // every logical place in the subtree is drawn at `scale × k`: moved
        // so that its centre stays where it is
        let (cx, cy) = (ox + rect[0] + rect[2] / 2.0, oy + rect[1] + rect[3] / 2.0);
        ox += cx * (1.0 / k - 1.0);
        oy += cy * (1.0 / k - 1.0);
        let scale = self.scale;
        self.scale = scale * k;
        self.paint_node(key, ox, oy, clip, opacity, st, rect, ts, dl, rings);
        self.scale = scale;
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_node(
        &mut self,
        key: &str,
        ox: f32,
        oy: f32,
        clip: Clip,
        opacity: f32,
        st: scene::Style,
        rect: [f32; 4],
        ts: &mut TextSystem,
        dl: &mut DisplayList,
        rings: &mut Vec<Prim>,
    ) {
        let Some(node) = self.scene.nodes.get(key) else {
            return;
        };
        let s = self.scale;
        let x = (ox + rect[0]) * s;
        let y = (oy + rect[1]) * s;
        let w = rect[2] * s;
        let h = rect[3] * s;
        let opacity = opacity * st.opacity;
        if opacity <= 0.0 {
            return;
        }
        let fade =
            |c: Color| -> Color { [c[0], c[1], c[2], (c[3] as f32 * opacity).round() as u8] };
        let radii = st.radius.map(|r| r * s);
        let mut own_rect = None;
        if st.bg.is_some() || (st.border.is_some() && st.border_width > 0.0) {
            own_rect = Some(dl.prims.len());
            dl.prims.push(Prim::Rect {
                x,
                y,
                w,
                h,
                fill: fade(st.bg.unwrap_or([0, 0, 0, 0])),
                border: fade(st.border.unwrap_or([0, 0, 0, 0])),
                border_width: st.border_width * s,
                radii,
                clip,
            });
        }
        let focused = self.scene.focus.as_deref() == Some(key);
        if focused
            && self.focused_window
            && (self.scene.focus_visible || node.edit.is_some())
            && node.variants[scene::FOCUS].is_none()
            && let Some(ring) = st.focus_ring
        {
            let gap = 2.0 * s;
            let width = 2.0 * s;
            rings.push(Prim::Rect {
                x: x - gap - width,
                y: y - gap - width,
                w: w + 2.0 * (gap + width),
                h: h + 2.0 * (gap + width),
                fill: [0, 0, 0, 0],
                border: ring,
                border_width: width,
                radii: radii.map(|r| if r > 0.0 { r + gap + width } else { 0.0 }),
                clip: Clip::rect(0.0, 0.0, dl.width as f32, dl.height as f32),
            });
        }
        let content = [
            x + st.pad[3] * s,
            y + st.pad[0] * s,
            (w - (st.pad[1] + st.pad[3]) * s).max(0.0),
            (h - (st.pad[0] + st.pad[2]) * s).max(0.0),
        ];
        let clips = st.clip || node.scrollable || node.edit.is_some();
        // A clipping node clips its children to the inside of its border,
        // following its corners, and draws the border over them.
        let bw = if st.border.is_some() {
            st.border_width * s
        } else {
            0.0
        };
        let inner_clip = if clips {
            clip.intersect(Clip {
                x0: x + bw,
                y0: y + bw,
                x1: x + w - bw,
                y1: y + h - bw,
                radii: radii.map(|r| (r - bw).max(0.0)),
            })
        } else {
            clip
        };
        let late_border = clips && bw > 0.0 && !node.children.is_empty();
        if late_border
            && let Some(i) = own_rect
            && let Prim::Rect { border_width, .. } = &mut dl.prims[i]
        {
            // The border is drawn after the children; this pass draws
            // the fill only.
            *border_width = 0.0;
        }
        let text = node.text.clone();
        let (scrollable, is_edit) = (node.scrollable, node.edit.is_some());
        let (children, offset) = (node.children.clone(), node.offset);
        let (draw, image, fit) = (node.draw.clone(), node.image.clone(), node.fit.clone());
        if let Some(ops) = &draw
            && let Ok(drawing) = super::canvas::draw(ops, rect[2], rect[3], s)
        {
            self.paint_canvas(drawing, x, y, w, h, inner_clip, &st, opacity, ts, dl);
        }
        if let Some(src) = &image {
            // The picture at the size it shows (a real window's comes from
            // a decoding thread, and the window draws again when it has it).
            let natural = fit == "none";
            let want = if natural { None } else { Some((w, h)) };
            let waiter = if self.headless { None } else { Some(self.id) };
            if let Ok(Some((still, info))) = super::picture::get(src, want, waiter) {
                // An animation: the frame its clock says (once its frames
                // are decoded; its still until then).
                let mut pic = still;
                let mut badge = None;
                if info.animated
                    && let Some(anim) = super::picture::animation(src, info, want, waiter)
                {
                    let now = self.clock_ms();
                    let rm = self.settings.reduce_motion;
                    let first = anim.frames[0].id;
                    let p = self.plays.entry(key.to_string()).or_insert(Play {
                        id: first,
                        start: now,
                        paused: None,
                        chosen: false,
                    });
                    if p.id != first {
                        *p = Play {
                            id: first,
                            start: now,
                            paused: None,
                            chosen: false,
                        };
                    }
                    let p = *p;
                    let held = p.paused.or(if rm && !p.chosen { Some(0.0) } else { None });
                    let (i, next) = anim.at(held.unwrap_or(now - p.start));
                    pic = anim.frames[i].clone();
                    // what of it shows: only a picture in view asks for frames
                    let seen = inner_clip.intersect(Clip::rect(x, y, x + w, y + h));
                    if held.is_none()
                        && seen.x1 > seen.x0
                        && seen.y1 > seen.y0
                        && let Some(n) = next
                    {
                        let due = p.start + n;
                        self.next_frame = Some(self.next_frame.map_or(due, |d| d.min(due)));
                    }
                    // its button: under reduced motion, or once used; on
                    // a picture large enough to hold one
                    let side_l = (w.min(h) / s * 0.4).clamp(0.0, 44.0);
                    let brect = if (rm || p.chosen) && side_l >= 20.0 {
                        let (bx, by) = (
                            ox + rect[0] + rect[2] / 2.0 - side_l / 2.0,
                            oy + rect[1] + rect[3] / 2.0 - side_l / 2.0,
                        );
                        Some([bx, by, side_l, side_l])
                    } else {
                        None
                    };
                    let (px, py) = self.pointer;
                    let over = px >= ox + rect[0]
                        && py >= oy + rect[1]
                        && px < ox + rect[0] + rect[2]
                        && py < oy + rect[1] + rect[3];
                    if let Some(r) = brect
                        && (held.is_some() || over)
                    {
                        badge = Some((r, held.is_some()));
                    }
                    self.shown_anims.push(AnimShown {
                        key: key.to_string(),
                        frames: anim.frames.len(),
                        frame: i,
                        paused: held.is_some(),
                        badge: brect,
                    });
                }
                // Its own size: one image pixel to one display pixel (an
                // SVG's units are logical pixels), from the top left, on
                // whole pixels so nothing is resampled.
                let (iw, ih) = if natural {
                    if info.format == super::picture::Format::Svg {
                        (info.width as f32 * s, info.height as f32 * s)
                    } else {
                        (info.width as f32, info.height as f32)
                    }
                } else {
                    (pic.width as f32, pic.height as f32)
                };
                let (sx, sy) = (w / iw.max(1.0), h / ih.max(1.0));
                let k = match fit.as_str() {
                    "cover" => sx.max(sy),
                    _ => sx.min(sy),
                };
                let (dx, dy, dw, dh) = if natural {
                    (x.round(), y.round(), iw, ih)
                } else if fit == "fill" {
                    (x, y, w, h)
                } else {
                    let (dw, dh) = (iw * k, ih * k);
                    (x + (w - dw) / 2.0, y + (h - dh) / 2.0, dw, dh)
                };
                dl.prims.push(Prim::Image {
                    x: dx,
                    y: dy,
                    w: dw,
                    h: dh,
                    pic,
                    clip: inner_clip.intersect(Clip::rect(x, y, x + w, y + h)),
                    alpha: opacity,
                });
                // the button that plays a still animation (or pauses it)
                if let Some((r, play)) = badge {
                    let side = (r[2] * s).round().max(8.0);
                    dl.prims.push(Prim::Image {
                        x: (r[0] * s).round(),
                        y: (r[1] * s).round(),
                        w: side,
                        h: side,
                        pic: badge_picture(side as u32, play),
                        clip: inner_clip.intersect(Clip::rect(x, y, x + w, y + h)),
                        alpha: opacity,
                    });
                }
            }
        }
        if self.editors.get(key).is_some_and(|e| e.rich().is_some()) {
            self.paint_rich(key, &st, content, inner_clip, focused, opacity, ts, dl);
        } else if self.editors.contains_key(key) {
            self.paint_editor(key, &st, content, inner_clip, focused, opacity, ts, dl);
        } else if let Some(t) = &text {
            let max_w = if st.wrap { Some(content[2] / s) } else { None };
            let align_w = if st.align != Align::Start {
                Some(content[2] / s)
            } else {
                max_w
            };
            // a text that does not wrap is one line: shaped as one, then
            // aligned in its box only when it fits (shaped at the box's
            // width it would wrap onto lines the box cannot hold)
            let one = !st.wrap && st.align != Align::Start;
            let line = if one {
                Some(ts.shape(t, &st.font, fade(st.color), None, Align::Start, s))
            } else {
                None
            };
            let fits = line.as_ref().is_none_or(|l| l.width <= content[2] + 0.5);
            let mut shaped = match line {
                Some(l) if !fits => l,
                _ => ts.shape(t, &st.font, fade(st.color), align_w, st.align, s),
            };
            // a line wider than its box, cut to fit with an ellipsis
            // (whatever its alignment)
            if st.truncate && !st.wrap && shaped.width > content[2] + 0.5 && content[2] > 0.0 {
                let chars: Vec<char> = t.chars().collect();
                let (mut lo, mut hi) = (0usize, chars.len());
                while lo < hi {
                    let mid = (lo + hi).div_ceil(2);
                    let cut: String = chars[..mid]
                        .iter()
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                        + "…";
                    if ts
                        .shape(&cut, &st.font, fade(st.color), None, Align::Start, s)
                        .width
                        <= content[2]
                    {
                        lo = mid;
                    } else {
                        hi = mid - 1;
                    }
                }
                let cut: String = chars[..lo]
                    .iter()
                    .collect::<String>()
                    .trim_end()
                    .to_string()
                    + "…";
                shaped = ts.shape(&cut, &st.font, fade(st.color), align_w, st.align, s);
            }
            let dy = match st.valign.along() {
                Align::Center => ((content[3] - shaped.height) / 2.0).max(0.0),
                Align::End | Align::Right => (content[3] - shaped.height).max(0.0),
                Align::Start | Align::Left => 0.0,
            };
            push_glyphs(&shaped, content[0], content[1] + dy, inner_clip, dl);
        }
        // stretched past an end (a trackpad's pull, a bounce): drawn moved
        let stretch = if scrollable && !self.elastic.is_empty() {
            self.stretch_of(key, [rect[2], rect[3]])
        } else {
            [0.0, 0.0]
        };
        let offset = (offset.0 - stretch[0], offset.1 - stretch[1]);
        let (cx, cy) = (ox + rect[0] - offset.0, oy + rect[1] - offset.1);
        if scrollable && !is_edit {
            // what it shows moves with its offset: damage moves the kept
            // frame with it rather than drawing it again
            use std::hash::{Hash, Hasher};
            let mut h = rustc_hash::FxHasher::default();
            key.hash(&mut h);
            dl.scrolls.push(super::damage::Scroll {
                id: h.finish(),
                view: inner_clip,
                dx: offset.0 * s,
                dy: offset.1 * s,
            });
        }
        for c in children {
            self.paint(&c, cx, cy, inner_clip, opacity, ts, dl, rings);
        }
        // a scroll region's bar, over what it scrolls
        if scrollable && !is_edit {
            let n = &self.scene.nodes[key];
            let (_, ch) = self.scene.content_size(n);
            let view = rect[3];
            let max = (ch - view).max(0.0);
            if max > 0.5 {
                self.paint_bar(
                    key,
                    false,
                    [x, y, w, h],
                    inner_clip,
                    view,
                    ch,
                    offset.1,
                    max,
                    &[],
                    opacity,
                    dl,
                );
            }
        }
        if late_border && let Some(b) = st.border {
            dl.prims.push(Prim::Rect {
                x,
                y,
                w,
                h,
                fill: [0, 0, 0, 0],
                border: fade(b),
                border_width: bw,
                radii,
                clip,
            });
        }
    }

    /// The nodes that left with an `exit`, over everything, each in its
    /// pose now; gone once there. Under nothing: nothing of them is found
    /// by the pointer or said.
    fn paint_ghosts(
        &mut self,
        full: Clip,
        ts: &mut TextSystem,
        dl: &mut DisplayList,
        rings: &mut Vec<Prim>,
    ) {
        if self.motion.ghosts.is_empty() {
            return;
        }
        let now = self.clock_ms();
        let ghosts = std::mem::take(&mut self.motion.ghosts);
        let (hits, caret, anims) = (self.bar_hits.len(), self.caret, self.shown_anims.len());
        let mut kept = Vec::new();
        for mut g in ghosts {
            let (pose, done) = g.pose(now);
            if done || !self.motion.runs(self.settings.reduce_motion) {
                continue;
            }
            std::mem::swap(&mut self.scene, &mut g.scene);
            if let Some(root) = self.scene.root.clone()
                && let Some(node) = self.scene.nodes.get(&root)
            {
                let st = self.scene.effective_style(node);
                let rect = node.rect;
                self.motion.ghosting = true;
                self.paint_posed(&root, 0.0, 0.0, full, 1.0, st, rect, pose, ts, dl, rings);
                self.motion.ghosting = false;
            }
            std::mem::swap(&mut self.scene, &mut g.scene);
            kept.push(g);
        }
        self.bar_hits.truncate(hits);
        self.shown_anims.truncate(anims);
        self.caret = caret;
        self.motion.ghosts = kept;
    }

    /// Before a patch: the nodes it adds that come in with an `enter`, and
    /// the nodes it removes that leave with an `exit` (their subtrees kept
    /// as they were, where they were, to be drawn leaving).
    fn motion_patch(&mut self, ops: &Value) {
        let Value::List(items) = ops else { return };
        let now = self.clock_ms();
        for op in items.iter() {
            let kind = get(op, "op").and_then(|v| match v {
                Value::String(s) => Some(s.as_str()),
                _ => None,
            });
            let Some(Value::String(key)) = get(op, "key") else {
                continue;
            };
            match kind.unwrap_or("node") {
                "node" => {
                    if !self.scene.nodes.contains_key(key.as_str()) {
                        self.motion.ghosts.retain(|g| g.key != key.as_str());
                        if get(op, "style")
                            .and_then(|st| get(st, "transition"))
                            .and_then(|t| get(t, "enter"))
                            .is_some()
                        {
                            self.motion.fresh.insert(key.to_string());
                        }
                    }
                }
                "remove" => {
                    // the outermost nodes under it that leave with an exit
                    let mut stack = vec![key.to_string()];
                    while let Some(k) = stack.pop() {
                        let Some(n) = self.scene.nodes.get(&k) else {
                            continue;
                        };
                        match n.style.transition.clone().filter(|t| t.exit.is_some()) {
                            Some(tr) => {
                                if let Some(g) = self.ghost_of(&k) {
                                    self.motion.leave(k, g, &tr, now);
                                }
                            }
                            None => stack.extend(n.children.iter().cloned()),
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Node `key`'s subtree as a scene of its own, its root where it shows
    /// in the window (nothing hovered, pressed or focused in it).
    fn ghost_of(&self, key: &str) -> Option<Scene> {
        let abs = self.scene.absolute(key)?;
        let mut g = Scene::default();
        let mut stack = vec![key.to_string()];
        while let Some(k) = stack.pop() {
            let Some(n) = self.scene.nodes.get(&k) else {
                continue;
            };
            let mut n = n.clone();
            stack.extend(n.children.iter().cloned());
            if k == key {
                n.parent = None;
                n.rect = abs;
            }
            g.nodes.insert(k, n);
        }
        g.root = Some(key.to_string());
        Some(g)
    }

    /// A canvas's drawing (canvas.rs) at `(x, y)`, `w × h` device pixels,
    /// as the window's own primitives.
    #[allow(clippy::too_many_arguments)]
    fn paint_canvas(
        &self,
        drawing: super::canvas::Drawing,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        inner_clip: Clip,
        st: &scene::Style,
        opacity: f32,
        ts: &mut TextSystem,
        dl: &mut DisplayList,
    ) {
        use super::canvas::Item;
        let s = self.scale;
        let fade =
            |c: Color| -> Color { [c[0], c[1], c[2], (c[3] as f32 * opacity).round() as u8] };
        let cclip = inner_clip.intersect(Clip::rect(x, y, x + w, y + h));
        let sub = |a: [f32; 4]| {
            cclip.intersect(Clip::rect(
                x + a[0] * s,
                y + a[1] * s,
                x + a[2] * s,
                y + a[3] * s,
            ))
        };
        for item in drawing.items {
            match item {
                Item::Rect {
                    x: rx,
                    y: ry,
                    w: rw,
                    h: rh,
                    fill,
                    border,
                    border_width,
                    radii,
                    clip,
                } => {
                    let c = sub(clip);
                    if c.is_empty() {
                        continue;
                    }
                    dl.prims.push(Prim::Rect {
                        x: x + rx * s,
                        y: y + ry * s,
                        w: rw * s,
                        h: rh * s,
                        fill: fade(fill),
                        border: fade(border),
                        border_width: border_width * s,
                        radii: radii.map(|r| r * s),
                        clip: c,
                    });
                }
                Item::Pic {
                    x: px,
                    y: py,
                    w: pw,
                    h: ph,
                    pic,
                    radii,
                    clip,
                } => {
                    let (bx, by, bw, bh) = (x + px * s, y + py * s, pw * s, ph * s);
                    let mut c = sub(clip);
                    if radii.iter().any(|r| *r > 0.0) {
                        let max = bw.min(bh) / 2.0;
                        c = c.intersect(Clip {
                            x0: bx,
                            y0: by,
                            x1: bx + bw,
                            y1: by + bh,
                            radii: radii.map(|r| (r * s).min(max)),
                        });
                    }
                    if c.is_empty() {
                        continue;
                    }
                    dl.prims.push(Prim::Image {
                        x: bx,
                        y: by,
                        w: bw,
                        h: bh,
                        pic,
                        clip: c,
                        alpha: opacity,
                    });
                }
                Item::Image {
                    x: ix,
                    y: iy,
                    w: iw,
                    h: ih,
                    src,
                    fit,
                    clip,
                } => {
                    let (bx, by, bw, bh) = (x + ix * s, y + iy * s, iw * s, ih * s);
                    let waiter = if self.headless { None } else { Some(self.id) };
                    if let Ok(Some((pic, _))) = super::picture::get(&src, Some((bw, bh)), waiter) {
                        let (pw, ph) = (pic.width as f32, pic.height as f32);
                        let (kx, ky) = (bw / pw.max(1.0), bh / ph.max(1.0));
                        let k = if fit == "cover" {
                            kx.max(ky)
                        } else {
                            kx.min(ky)
                        };
                        let (dx, dy, dw, dh) = if fit == "fill" {
                            (bx, by, bw, bh)
                        } else {
                            (
                                bx + (bw - pw * k) / 2.0,
                                by + (bh - ph * k) / 2.0,
                                pw * k,
                                ph * k,
                            )
                        };
                        dl.prims.push(Prim::Image {
                            x: dx,
                            y: dy,
                            w: dw,
                            h: dh,
                            pic,
                            clip: sub(clip).intersect(Clip::rect(bx, by, bx + bw, by + bh)),
                            alpha: opacity,
                        });
                    }
                }
                Item::Text(t) => {
                    let c = sub(t.clip);
                    if c.is_empty() || t.text.is_empty() {
                        continue;
                    }
                    let mut font = st.font.clone();
                    font.size = t.size;
                    font.weight = t.weight;
                    if let Some(f) = &t.font {
                        font.family = f.clone();
                    }
                    if t.italic {
                        font.italic = true;
                    }
                    let color = fade(t.color);
                    let mut shaped = ts.shape(&t.text, &font, color, None, Align::Start, s);
                    // longer than it may be: cut with an ellipsis
                    if let Some(mw) = t.max_w
                        && shaped.width > mw * s + 0.5
                    {
                        let chars: Vec<char> = t.text.chars().collect();
                        let cut_at = |n: usize| {
                            chars[..n].iter().collect::<String>().trim_end().to_string() + "…"
                        };
                        let (mut lo, mut hi) = (0usize, chars.len());
                        while lo < hi {
                            let mid = (lo + hi).div_ceil(2);
                            if ts
                                .shape(&cut_at(mid), &font, color, None, Align::Start, s)
                                .width
                                <= mw * s
                            {
                                lo = mid;
                            } else {
                                hi = mid - 1;
                            }
                        }
                        if lo == 0 {
                            continue;
                        }
                        shaped = ts.shape(&cut_at(lo), &font, color, None, Align::Start, s);
                    }
                    let dx = match t.align {
                        1 => shaped.width / 2.0,
                        2 => shaped.width,
                        _ => 0.0,
                    };
                    push_glyphs(&shaped, x + t.x * s - dx, y + t.y * s, c, dl);
                }
            }
        }
    }

    /// The hit region of canvas `key` under the pointer at `(x, y)`
    /// (logical, in the window), or `()`.
    fn canvas_hit(&self, key: Option<&str>, x: f32, y: f32) -> Value {
        let Some(k) = key else { return Value::Unit };
        let Some(n) = self.scene.nodes.get(k) else {
            return Value::Unit;
        };
        let Some(ops) = &n.draw else {
            return Value::Unit;
        };
        let Some(abs) = self.scene.absolute(k) else {
            return Value::Unit;
        };
        super::canvas::hit(ops, n.rect[2], n.rect[3], x - abs[0], y - abs[1])
    }

    /// A styled field that asks (`hover`) hears which character the
    /// pointer is over, when that changes: `hover` with its line, column
    /// and box (window pixels), or line -1 when it leaves the text.
    fn hover_field(&mut self, hit: Option<&str>, x: f32, y: f32, out: &mut Vec<Value>) {
        let target = hit.and_then(|h| {
            self.scene
                .target_up(h, |n| n.edit.as_ref().is_some_and(|e| e.rich && e.hover))
        });
        // the field the pointer left: said once
        let left: Vec<String> = self
            .editors
            .iter()
            .filter(|(k, e)| {
                Some(k.as_str()) != target.as_deref()
                    && e.rich().is_some_and(|r| r.said_hover.is_some())
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in left {
            if let Some(r) = self.editors.get_mut(&k).and_then(|e| e.rich_mut()) {
                r.said_hover = None;
            }
            out.push(self.ev("hover", hover_left(&k)));
        }
        let Some(k) = target else { return };
        if self.scene.pressed.is_some() {
            return;
        }
        let Some((ox, oy, _, h)) = self.edit_origin(&k) else {
            return;
        };
        let pad_left = self
            .scene
            .nodes
            .get(&k)
            .map(|n| n.style.pad[3])
            .unwrap_or(0.0);
        let Ok(mut ts) = text::system().lock() else {
            return;
        };
        let dy = self.edit_text_dy(&k, h, &mut ts);
        let sc = self.scale;
        let Some(ed) = self.editors.get_mut(&k) else {
            return;
        };
        let (sx, sy) = ed.scrolled();
        let Some(r) = ed.rich_mut() else { return };
        let (tx, ty) = (x * sc - ox + sx, y * sc - oy - dy + sy);
        let mut found = r.hover_at(tx, ty, &mut ts);
        // in the gutter (left of the text, a code editor's): the line, with
        // column -2 and the mark's box
        let mut gutter = false;
        if found.is_none()
            && tx < 0.0
            && r.gutter.is_some()
            && let Some((line, b)) = r.gutter_line_at(ty, &mut ts)
        {
            found = Some((line, usize::MAX, b));
            gutter = true;
        }
        drop(ts);
        let now = found.as_ref().map(|(l, c, _)| (*l, *c));
        if now == r.said_hover {
            return;
        }
        r.said_hover = now;
        let ev = match found {
            Some((line, _, b)) if gutter => {
                // the mark's box, at the gutter's left edge
                let bx = (ox - pad_left * sc + 4.0 * sc) / sc;
                let by = (oy + dy + b.y0 as f32 - sy) / sc;
                let rect = Value::Tuple(Arc::new(vec![
                    float(bx),
                    float(by),
                    float(((b.x1 - b.x0) as f32 / sc).max(1.0)),
                    float((b.y1 - b.y0) as f32 / sc),
                ]));
                // the gutter beside a line: no column
                vec![
                    ("key", s(&k)),
                    ("line", Value::Integer(line as i64 + 1)),
                    ("column", Value::Unit),
                    ("gutter", Value::Boolean(true)),
                    ("rect", rect),
                ]
            }
            Some((line, col, b)) => {
                let bx = (ox + b.x0 as f32 - sx) / sc;
                let by = (oy + dy + b.y0 as f32 - sy) / sc;
                let rect = Value::Tuple(Arc::new(vec![
                    float(bx),
                    float(by),
                    float(((b.x1 - b.x0) as f32 / sc).max(1.0)),
                    float((b.y1 - b.y0) as f32 / sc),
                ]));
                // 1-based, as every line and column olang reports
                vec![
                    ("key", s(&k)),
                    ("line", Value::Integer(line as i64 + 1)),
                    ("column", Value::Integer(col as i64 + 1)),
                    ("rect", rect),
                ]
            }
            None => hover_left(&k),
        };
        out.push(self.ev("hover", ev));
    }

    /// A styled field: the paragraphs in view, their spans' and lines'
    /// backgrounds, the selection, the glyphs, underlines and strikes,
    /// the composition, the caret, and a scroll mark when the text is
    /// longer than the box.
    #[allow(clippy::too_many_arguments)]
    fn paint_rich(
        &mut self,
        key: &str,
        st: &scene::Style,
        content: [f32; 4],
        clip: Clip,
        focused: bool,
        opacity: f32,
        ts: &mut TextSystem,
        dl: &mut DisplayList,
    ) {
        let s = self.scale;
        let placeholder = self.scene.nodes[key]
            .edit
            .as_ref()
            .map(|e| e.placeholder.clone())
            .unwrap_or_default();
        let window_focused = self.focused_window;
        let fade =
            |c: Color| -> Color { [c[0], c[1], c[2], (c[3] as f32 * opacity).round() as u8] };
        let r = self
            .editors
            .get_mut(key)
            .and_then(|e| e.rich_mut())
            .expect("present");
        r.set_font(&st.font, st.color);
        r.set_geometry_keeping(content[2] / s, s, ts);
        r.follow_caret(content[3], ts);
        let ox = content[0] - r.scroll_x;
        let oy = content[1] - r.scroll_y;
        // stretched past an end (a trackpad's pull, a bounce): drawn moved
        let stretch = match self.elastic.get(key) {
            Some(e) => {
                e.shown(
                    self.test_clock
                        .unwrap_or_else(|| self.born.elapsed().as_secs_f64() * 1000.0),
                    [content[2] / s, content[3] / s],
                )
                .0[1]
                    * s
            }
            None => 0.0,
        };
        let r = self
            .editors
            .get_mut(key)
            .and_then(|e| e.rich_mut())
            .expect("present");
        let oy = oy + stretch;
        let outer_clip = clip;
        {
            // its text moves with its scroll: damage moves the kept frame
            use std::hash::{Hash, Hasher};
            let mut h = rustc_hash::FxHasher::default();
            (key, "rich").hash(&mut h);
            dl.scrolls.push(super::damage::Scroll {
                id: h.finish(),
                view: outer_clip,
                dx: r.scroll_x,
                dy: r.scroll_y - stretch,
            });
        }
        // what the text draws from here: clipped below the pinned rows of
        // sticky scroll once they are known (the renderers draw glyphs over
        // rectangles, so a row's background alone does not hide them)
        let first_prim = dl.prims.len();
        let clip = clip.intersect(Clip::rect(
            content[0] - 1.0,
            content[1],
            content[0] + content[2] + 1.0,
            content[1] + content[3],
        ));
        let shown = r.visible(r.scroll_y, r.scroll_y + content[3], ts);
        // a code editor's caret line, edge to edge
        if let Some(c) = r.gutter.as_ref().and_then(|g| g.line_bg)
            && focused
        {
            let fi = r.focus_para();
            if shown.contains(&fi) {
                let (py, _, ph) = r.para(fi);
                dl.prims.push(solid(
                    content[0] - st.pad[3] * s,
                    oy + py,
                    content[2] + (st.pad[3] + st.pad[1]) * s,
                    ph,
                    fade(c),
                    outer_clip,
                ));
            }
        }
        // rulers: a hairline at each column asked for (a code font's
        // characters are one width: the width of ten zeros a tenth each)
        if !r.rulers.is_empty() {
            let mut rf = st.font.clone();
            rf.spans = None;
            let cw = ts
                .shape("0000000000", &rf, st.color, None, Align::Start, s)
                .width
                / 10.0;
            let mut c = r.ruler_color.unwrap_or(st.placeholder);
            if r.ruler_color.is_none() {
                c[3] = (c[3] as f32 * 0.35) as u8;
            }
            for col in r.rulers.clone() {
                let x = (ox + cw * col as f32).round();
                if x > content[0] && x < content[0] + content[2] {
                    dl.prims
                        .push(solid(x, content[1], s.max(1.0), content[3], fade(c), clip));
                }
            }
        }
        // decorations behind the text: a matching bracket, a highlight
        let decos: Vec<(usize, usize, usize, u32)> = r
            .decorations
            .iter()
            .copied()
            .filter(|d| shown.contains(&d.0))
            .collect();
        for &(i, a, b, sty) in &decos {
            let Some(look) = r.style_of(sty).cloned() else {
                continue;
            };
            let (py, _, _) = r.para(i);
            let (ba, bb) = r.para_char_bytes(i, a, b);
            if let Some(c) = look.bg {
                for bx in r.range_boxes(i, ba, bb) {
                    dl.prims.push(Prim::Rect {
                        x: ox + bx.x0 as f32 - s,
                        y: oy + py + bx.y0 as f32,
                        w: (bx.x1 - bx.x0) as f32 + 2.0 * s,
                        h: (bx.y1 - bx.y0) as f32,
                        fill: fade(c),
                        border: [0, 0, 0, 0],
                        border_width: 0.0,
                        radii: [2.0 * s; 4],
                        clip,
                    });
                }
            }
        }
        // backgrounds: a code block's lines edge to edge, a code span's
        // glyphs
        for i in shown.clone() {
            let (py, _, ph) = r.para(i);
            for (sp, look) in r.para_spans(i) {
                if let Some(c) = look.line_bg {
                    dl.prims.push(solid(
                        content[0] - 4.0 * s,
                        oy + py,
                        content[2] + 8.0 * s,
                        ph,
                        fade(c),
                        clip,
                    ));
                }
                if let Some(c) = look.bg {
                    for b in r.range_boxes(i, sp.start, sp.end) {
                        dl.prims.push(Prim::Rect {
                            x: ox + b.x0 as f32 - 2.0 * s,
                            y: oy + py + b.y0 as f32,
                            w: (b.x1 - b.x0) as f32 + 4.0 * s,
                            h: (b.y1 - b.y0) as f32,
                            fill: fade(c),
                            border: [0, 0, 0, 0],
                            border_width: 0.0,
                            radii: [3.0 * s; 4],
                            clip,
                        });
                    }
                }
            }
        }
        if focused {
            // the selection as one shape, as a Mac's text views draw it:
            // each row it covers whole, top to bottom of the row (no gap
            // between rows), and a row it goes on past (a line break
            // selected, a wrapped line's earlier rows) out to the right
            // edge; only its last row ends where the selection does
            let right_edge = content[0] + content[2];
            for i in shown.clone() {
                let Some((a, b, more)) = r.para_selection(i) else {
                    continue;
                };
                let (py, _, ph) = r.para(i);
                let boxes = r.range_boxes(i, a, b);
                let bands = r.row_bands(i);
                // a row's band, the paragraph's first row from its top and
                // its last to its bottom (the leading between lines is
                // the selection's too)
                let nb = bands.len();
                let band_of = |y0: f32, y1: f32| -> (f32, f32) {
                    let mid = (y0 + y1) / 2.0;
                    let k = bands.iter().position(|(t, b)| mid >= *t && mid <= *b);
                    match k {
                        Some(k) => (
                            if k == 0 { 0.0 } else { bands[k].0 },
                            if k + 1 == nb { ph } else { bands[k].1 },
                        ),
                        None => (y0, y1),
                    }
                };
                let n = boxes.len();
                for (k, bx) in boxes.iter().enumerate() {
                    let (t, bt) = band_of(bx.y0 as f32, bx.y1 as f32);
                    let x0 = ox + bx.x0 as f32;
                    let goes_on = more || k + 1 < n;
                    let x1 = if goes_on {
                        right_edge.max(ox + bx.x1 as f32)
                    } else {
                        ox + bx.x1 as f32
                    };
                    // whole device pixels: rows that meet leave no seam
                    let (y0, y1) = ((oy + py + t).round(), (oy + py + bt).round());
                    let (x0, x1) = (x0.round(), x1.round());
                    dl.prims
                        .push(solid(x0, y0, x1 - x0, y1 - y0, fade(st.selection), clip));
                }
                if boxes.is_empty() && more {
                    // an empty line inside the selection: its row whole
                    let (t, bt) = (0.0, ph);
                    let (x0, y0, y1) = (ox.round(), (oy + py + t).round(), (oy + py + bt).round());
                    dl.prims.push(solid(
                        x0,
                        y0,
                        right_edge.round() - x0,
                        y1 - y0,
                        fade(st.selection),
                        clip,
                    ));
                }
            }
        }
        if r.value_is("") && !placeholder.is_empty() {
            let shaped = ts.shape(
                &placeholder,
                &st.font,
                fade(st.placeholder),
                Some(content[2] / s),
                Align::Start,
                s,
            );
            push_glyphs(&shaped, ox, oy, clip, dl);
        }
        for i in shown.clone() {
            let (py, layout, _) = r.para(i);
            let shaped = text::shaped_of(layout, s);
            let first = dl.prims.len();
            push_glyphs(&shaped, ox, oy + py, clip, dl);
            // a decoration that is only a colour (no fill, no underline)
            // recolours its glyphs: a name the server says is unnecessary,
            // faded
            for &(di, a, b, sty) in decos.iter().filter(|d| d.0 == i) {
                let Some(look) = r.style_of(sty).cloned() else {
                    continue;
                };
                let (Some(c), None, false) = (look.color, look.bg, look.underline) else {
                    continue;
                };
                let (ba, bb) = r.para_char_bytes(di, a, b);
                let boxes = r.range_boxes(di, ba, bb);
                for prim in dl.prims[first..].iter_mut() {
                    if let Prim::Glyph { x, y, color, .. } = prim {
                        let hit = boxes.iter().any(|bx| {
                            *x >= ox + bx.x0 as f32 - 0.5
                                && *x < ox + bx.x1 as f32 - 0.5
                                && *y >= oy + py + bx.y0 as f32
                                && *y <= oy + py + bx.y1 as f32
                        });
                        if hit {
                            *color = fade(c);
                        }
                    }
                }
            }
            // underlines and strikes
            for (sp, look) in r.para_spans(i) {
                if !look.underline && !look.strike {
                    continue;
                }
                let c = fade(look.color.unwrap_or(st.color));
                for b in r.range_boxes(i, sp.start, sp.end) {
                    let (x, w) = (ox + b.x0 as f32, (b.x1 - b.x0) as f32);
                    if look.underline {
                        dl.prims.push(solid(
                            x,
                            (oy + py + b.y1 as f32 - 2.0 * s).round(),
                            w,
                            s.max(1.0),
                            c,
                            clip,
                        ));
                    }
                    if look.strike {
                        dl.prims.push(solid(
                            x,
                            (oy + py + ((b.y0 + b.y1) / 2.0) as f32).round(),
                            w,
                            s.max(1.0),
                            c,
                            clip,
                        ));
                    }
                }
            }
            if let Some((a, b)) = r.para_compose(i) {
                for bx in r.range_boxes(i, a, b) {
                    dl.prims.push(solid(
                        ox + bx.x0 as f32,
                        oy + py + bx.y1 as f32 - s,
                        (bx.x1 - bx.x0) as f32,
                        s,
                        fade(st.color),
                        clip,
                    ));
                }
            }
        }
        // text after a line's end (a problem said at its line): quiet,
        // a little smaller, cut with an ellipsis to the room left
        if !r.eol.is_empty() {
            let items: Vec<(usize, String, Option<Color>)> = r
                .eol
                .iter()
                .filter(|e| shown.contains(&e.0))
                .cloned()
                .collect();
            let mut ef = st.font.clone();
            ef.spans = None;
            ef.size = st.font.size * 0.92;
            let right = content[0] + content[2];
            for (i, label, color) in items {
                let (py, _, _) = r.para(i);
                let len = r.para_text(i).len();
                let boxes = r.range_boxes(i, 0, len);
                let lh = st.font.size
                    * s
                    * if st.font.line_height > 0.0 {
                        st.font.line_height
                    } else {
                        1.2
                    };
                let (ex, ey0, ey1) = match boxes.last() {
                    Some(b) if len > 0 => (b.x1 as f32, b.y0 as f32, b.y1 as f32),
                    _ => (0.0, 0.0, lh),
                };
                let x = (ox + ex + 20.0 * s).max(content[0] + 20.0 * s);
                let avail = right - x - 6.0 * s;
                if avail < 48.0 * s {
                    continue;
                }
                let c = fade(color.unwrap_or(st.placeholder));
                let mut shaped = ts.shape(&label, &ef, c, None, Align::Start, s);
                if shaped.width > avail {
                    let n = label.chars().count();
                    let keep = ((n as f32) * (avail / shaped.width.max(1.0))).floor() as usize;
                    match eol_cut(&label, keep.saturating_sub(1)) {
                        Some(cut) => shaped = ts.shape(&cut, &ef, c, None, Align::Start, s),
                        // too little of it would show: it gives way whole
                        // (its underline and its hover still say it)
                        None => continue,
                    }
                }
                let ty = oy + py + ey0 + (((ey1 - ey0) - shaped.height) / 2.0).max(0.0);
                push_glyphs(&shaped, x, ty, clip, dl);
            }
        }
        // lenses: each band under its line, drawn after the text is (the
        // field is let go first: a canvas paints through the window)
        let mut lens_jobs: Vec<(Value, f32, f32, f32, f32)> = Vec::new();
        // drawn gutter marks: (ops, x, y, side, clip), painted with the lenses
        let mut mark_jobs: Vec<(Value, f32, f32, f32, Clip)> = Vec::new();
        for (k, lens) in r.lenses().iter().enumerate() {
            if !shown.contains(&lens.line) {
                continue;
            }
            if let Some((top, h)) = r.lens_band(k) {
                lens_jobs.push((
                    lens.ops.clone(),
                    content[0],
                    oy + top,
                    content[2] + st.pad[1] * s,
                    h,
                ));
            }
        }
        // decorations under the text: a problem's underline
        for &(i, a, b, sty) in &decos {
            let Some(look) = r.style_of(sty).cloned() else {
                continue;
            };
            if !look.underline {
                continue;
            }
            let (py, _, _) = r.para(i);
            let (ba, bb) = r.para_char_bytes(i, a, b);
            let c = fade(look.color.unwrap_or(st.color));
            for bx in r.range_boxes(i, ba, bb.max(ba + 1)) {
                let w = ((bx.x1 - bx.x0) as f32).max(4.0 * s);
                if look.wavy {
                    // a problem's wave: 2 high, a period of 4 (logical),
                    // a pixel square a device pixel along it
                    let base = (oy + py + bx.y1 as f32 - 2.5 * s).round();
                    let x0 = ox + bx.x0 as f32;
                    let step = s.max(1.0);
                    let mut dx = 0.0f32;
                    while dx < w {
                        let phase = (dx / s) * std::f32::consts::PI / 2.0;
                        let yy = base + (1.0 - phase.sin()) * s;
                        dl.prims
                            .push(solid(x0 + dx, yy, step, (1.2 * s).max(1.0), c, clip));
                        dx += step;
                    }
                } else {
                    dl.prims.push(solid(
                        ox + bx.x0 as f32,
                        (oy + py + bx.y1 as f32 - 1.5 * s).round(),
                        w,
                        (1.5 * s).max(1.0),
                        c,
                        clip,
                    ));
                }
            }
        }
        // a code editor's gutter, in the left padding: the line numbers
        // (the caret's brighter) and the marks
        if let Some(g) = r.gutter.clone() {
            let gx0 = content[0] - st.pad[3] * s;
            let gclip = outer_clip.intersect(Clip::rect(
                gx0,
                content[1],
                content[0],
                content[1] + content[3],
            ));
            let fi = r.focus_para();
            let mut font = st.font.clone();
            font.spans = None;
            let base = g.color.unwrap_or(st.placeholder);
            if g.numbers {
                for i in shown.clone() {
                    let (py, _, _) = r.para(i);
                    let color = if i == fi {
                        g.current.unwrap_or(st.color)
                    } else {
                        base
                    };
                    let label = (i + 1).to_string();
                    let shaped = ts.shape(&label, &font, fade(color), None, Align::Start, s);
                    let x = content[0] - 14.0 * s - shaped.width;
                    push_glyphs(&shaped, x, oy + py, gclip, dl);
                }
            }
            // the changes' lane: a bar down each added or modified line
            // (a wrapped line's whole height), a tick across the lane at
            // the edge a removal came out of
            let lane_x =
                content[0] - (super::rich::CHANGE_LANE_RIGHT + super::rich::CHANGE_LANE_W) * s;
            let lane_w = super::rich::CHANGE_LANE_W * s;
            let paras = r.para_count();
            for (line, kind, color) in &g.changes {
                let c = fade(color.unwrap_or(base));
                match kind {
                    super::rich::Change::Removed => {
                        // the edge above `line`: its top, or under the last line
                        let edge = if *line < paras && shown.contains(line) {
                            Some(r.para(*line).0)
                        } else if *line > 0 && *line - 1 < paras && shown.contains(&(*line - 1)) {
                            let (py, _, ph) = r.para(*line - 1);
                            Some(py + ph)
                        } else {
                            None
                        };
                        if let Some(ey) = edge {
                            let h = (3.0 * s).max(2.0);
                            dl.prims.push(solid(
                                lane_x - 2.0 * s,
                                (oy + ey - h / 2.0).round(),
                                lane_w + 4.0 * s,
                                h,
                                c,
                                gclip,
                            ));
                        }
                    }
                    _ => {
                        if !shown.contains(line) || *line >= paras {
                            continue;
                        }
                        let (py, _, ph) = r.para(*line);
                        dl.prims.push(solid(
                            lane_x,
                            (oy + py).round(),
                            lane_w,
                            ph.ceil(),
                            c,
                            gclip,
                        ));
                    }
                }
            }
            for (line, ops) in &g.drawn {
                if !shown.contains(line) {
                    continue;
                }
                let (py, _, _) = r.para(*line);
                let side = super::rich::MARK_BOX * s;
                let lh = font.size
                    * s
                    * if font.line_height > 0.0 {
                        font.line_height
                    } else {
                        1.2
                    };
                mark_jobs.push((
                    ops.clone(),
                    gx0 + 4.0 * s,
                    (oy + py + ((lh - side) / 2.0).max(0.0)).round(),
                    side,
                    gclip,
                ));
            }
            for (line, glyph, color) in &g.marks {
                if !shown.contains(line) {
                    continue;
                }
                let (py, _, _) = r.para(*line);
                let mut mf = font.clone();
                mf.size = font.size * 0.8;
                let shaped = ts.shape(
                    glyph,
                    &mf,
                    fade(color.unwrap_or(base)),
                    None,
                    Align::Start,
                    s,
                );
                let lh = font.size * s * 1.2;
                push_glyphs(
                    &shaped,
                    gx0 + 6.0 * s,
                    oy + py + (lh - shaped.height).max(0.0) / 2.0,
                    gclip,
                    dl,
                );
            }
        }
        // sticky scroll: the first line of each scope the view is inside,
        // pinned at the top (outer first), pushed up as its scope's last
        // line arrives; numbered in the gutter; a hairline under them
        r.sticky_drawn.clear();
        let mut sticky_bottom = 0.0f32;
        let lh = st.font.size
            * s
            * if st.font.line_height > 0.0 {
                st.font.line_height
            } else {
                1.2
            };
        let mut rows: Vec<(usize, f32)> = Vec::new();
        let stk_now = r.sticky.clone();
        if let Some(stk) = stk_now.clone()
            && stk.max > 0
            && !stk.scopes.is_empty()
        {
            let top = r.top_line(ts);
            for &(h, e) in &stk.scopes {
                if rows.len() >= stk.max {
                    break;
                }
                if h > top + stk.max || e < top {
                    continue;
                }
                let slot = rows.len() as f32 * lh;
                let (hy, _) = r.line_box(h, ts);
                if hy >= r.scroll_y + slot {
                    continue;
                }
                let (ey, eh) = r.line_box(e, ts);
                let end_view = ey + eh - r.scroll_y;
                if end_view <= slot + lh * 0.5 {
                    continue;
                }
                rows.push((h, slot.min(end_view - lh)));
            }
        }
        // the edge the text is cut at: the pinned rows' bottom, else the
        // content's top. A row of text the edge half covers (a wrapped
        // line's row as much as a line's) is hidden whole — its glyphs by
        // their baseline, its line number with them — so no half-drawn row
        // shows under the tab bar or the pinned rows; everything else is
        // cut at the edge. A line whose first row went above it keeps its
        // number on its first row still in view.
        let edge = content[1] + rows.iter().map(|r| r.1 + lh).fold(0.0f32, f32::max);
        let mut half: Vec<(f32, f32)> = Vec::new();
        let mut carried: Vec<(usize, f32)> = Vec::new();
        for i in shown.clone() {
            let (py, _, ph) = r.para(i);
            let (t0, t1) = (oy + py, oy + py + ph);
            if t0 < edge - 0.5 && t1 > edge {
                let mut first_seen: Option<f32> = None;
                for (b0, b1) in r.row_bands(i) {
                    let (a, b) = (t0 + b0, t0 + b1);
                    if a < edge - 0.5 && b > edge {
                        half.push((a, b));
                    } else if a >= edge - 0.5 && first_seen.is_none() {
                        first_seen = Some(a);
                    }
                }
                if let Some(a) = first_seen
                    && !rows.iter().any(|(h, _)| *h == i)
                {
                    carried.push((i, a));
                }
            }
        }
        if !half.is_empty() || !rows.is_empty() {
            for p in dl.prims[first_prim..].iter_mut() {
                let hidden = match p {
                    Prim::Glyph { y, .. } => half.iter().any(|(a, b)| *y >= *a && *y <= *b),
                    // a row's own band (its selection, a highlight) goes
                    // with its glyphs, not a strip of colour left alone
                    Prim::Rect { y, h, .. } => half
                        .iter()
                        .any(|(a, b)| *y >= *a - 0.5 && *y + *h <= *b + 0.5),
                    _ => false,
                };
                let c = match p {
                    Prim::Rect { clip, .. }
                    | Prim::Glyph { clip, .. }
                    | Prim::Image { clip, .. } => clip,
                };
                *c = if hidden {
                    c.intersect(Clip::rect(c.x0, c.y1, c.x1, c.y1))
                } else {
                    c.intersect(Clip::rect(c.x0, edge, c.x1, c.y1.max(edge)))
                };
            }
        }
        if let Some(g) = r.gutter.clone()
            && g.numbers
        {
            let gx0 = content[0] - st.pad[3] * s;
            let gclip =
                outer_clip.intersect(Clip::rect(gx0, edge, content[0], content[1] + content[3]));
            let fi = r.focus_para();
            let mut font = st.font.clone();
            font.spans = None;
            let base = g.color.unwrap_or(st.placeholder);
            for &(i, a) in &carried {
                let color = if i == fi {
                    g.current.unwrap_or(st.color)
                } else {
                    base
                };
                let label = (i + 1).to_string();
                let shaped = ts.shape(&label, &font, fade(color), None, Align::Start, s);
                push_glyphs(&shaped, content[0] - 14.0 * s - shaped.width, a, gclip, dl);
            }
        }
        if let Some(stk) = stk_now
            && !rows.is_empty()
        {
            {
                let x0 = content[0] - st.pad[3] * s;
                let x1 = content[0] + content[2] + st.pad[1] * s;
                let field =
                    outer_clip.intersect(Clip::rect(x0, content[1], x1, content[1] + content[3]));
                let bg = fade(stk.bg.or(st.bg).unwrap_or([255, 255, 255, 255]));
                let mut gfont = st.font.clone();
                gfont.spans = None;
                let gcolor = r
                    .gutter
                    .as_ref()
                    .and_then(|g| g.color)
                    .unwrap_or(st.placeholder);
                for &(h, top_y) in rows.iter().rev() {
                    let y = content[1] + top_y;
                    let row_clip = field.intersect(Clip::rect(x0, y, x1, y + lh));
                    dl.prims.push(solid(x0, y, x1 - x0, lh, bg, field));
                    r.ensure_line(h, ts);
                    let (_, layout, _) = r.para(h);
                    let shaped = text::shaped_of(layout, s);
                    let text_clip = row_clip.intersect(Clip::rect(
                        content[0] - 1.0,
                        y,
                        content[0] + content[2] + 1.0,
                        y + lh,
                    ));
                    push_glyphs(&shaped, ox, y, text_clip, dl);
                    if r.gutter.as_ref().is_some_and(|g| g.numbers) {
                        let label = (h + 1).to_string();
                        let sh = ts.shape(&label, &gfont, fade(gcolor), None, Align::Start, s);
                        push_glyphs(&sh, content[0] - 14.0 * s - sh.width, y, row_clip, dl);
                    }
                    r.sticky_drawn.push((top_y, top_y + lh, h));
                }
                let bottom = rows.iter().map(|r| r.1 + lh).fold(0.0f32, f32::max);
                sticky_bottom = bottom;
                if let Some(c) = stk.line {
                    dl.prims.push(solid(
                        x0,
                        content[1] + bottom,
                        x1 - x0,
                        s.max(1.0),
                        fade(c),
                        field,
                    ));
                }
            }
        }
        // the scroll bar over the field's right edge, drawn last: the
        // view, the whole, the marks on its track (the caret's line, the
        // program's) by where their lines lie in the text
        let total = r.content_height(ts);
        let max = r.max_scroll(content[3], ts);
        let bar_job = if content[3] > 0.0 && max > 0.5 * s {
            let n = r.para_count();
            let mut marks: Vec<(f32, Color, bool)> = r
                .track
                .clone()
                .into_iter()
                .filter(|(l, _)| *l < n)
                .map(|(l, c)| (r.line_top(l, ts) / s, c, false))
                .collect();
            if r.gutter.is_some() {
                let fi = r.focus_para();
                marks.push((r.line_top(fi, ts) / s, st.color, true));
            }
            let node_box = [
                content[0] - st.pad[3] * s,
                content[1] - st.pad[0] * s,
                content[2] + (st.pad[1] + st.pad[3]) * s,
                content[3] + (st.pad[0] + st.pad[2]) * s,
            ];
            Some((
                node_box,
                content[3] / s,
                total / s,
                r.scroll_y / s,
                max / s,
                marks,
            ))
        } else {
            None
        };
        // what lies under the pinned rows is hidden by them
        let clip = if sticky_bottom > 0.0 {
            clip.intersect(Clip::rect(
                clip.x0,
                content[1] + sticky_bottom,
                clip.x1,
                clip.y1,
            ))
        } else {
            clip
        };
        // the further carets (a program's further selections)
        let mut more_carets: Vec<(f32, f32, f32, f32)> = Vec::new();
        if focused && window_focused && r.show_cursor() && !r.carets.is_empty() {
            let n = r.para_count();
            for (line, col) in r.carets.clone() {
                if line >= n || !shown.contains(&line) {
                    continue;
                }
                let (b, _) = r.para_char_bytes(line, col, col);
                let c = r.caret_box_at(line, b, (1.5 * s).max(1.0), ts);
                more_carets.push((
                    ox + c.x0 as f32,
                    oy + c.y0 as f32,
                    (c.x1 - c.x0) as f32,
                    (c.y1 - c.y0) as f32,
                ));
            }
        }
        let caret = if focused && window_focused && r.show_cursor() {
            let c = r.caret_box((1.5 * s).max(1.0), ts);
            Some((
                ox + c.x0 as f32,
                oy + c.y0 as f32,
                (c.x1 - c.x0) as f32,
                (c.y1 - c.y0) as f32,
            ))
        } else {
            None
        };
        let lens_clip = outer_clip.intersect(Clip::rect(
            content[0] - 1.0,
            content[1] + sticky_bottom,
            content[0] + content[2] + st.pad[1] * s,
            content[1] + content[3],
        ));
        for (ops, lx, ly, lw, lh) in lens_jobs {
            if let Ok(drawing) = super::canvas::draw(&ops, lw / s, lh / s, s) {
                self.paint_canvas(drawing, lx, ly, lw, lh, lens_clip, st, opacity, ts, dl);
            }
        }
        for (ops, mx, my, side, mclip) in mark_jobs {
            let mclip = if sticky_bottom > 0.0 {
                mclip.intersect(Clip::rect(
                    mclip.x0,
                    content[1] + sticky_bottom,
                    mclip.x1,
                    mclip.y1,
                ))
            } else {
                mclip
            };
            if let Ok(drawing) = super::canvas::draw(&ops, side / s, side / s, s) {
                self.paint_canvas(drawing, mx, my, side, side, mclip, st, opacity, ts, dl);
            }
        }
        for (cx, cy, cw, ch) in more_carets {
            dl.prims.push(solid(cx, cy, cw, ch, fade(st.caret), clip));
        }
        if let Some((cx, cy, cw, ch)) = caret {
            let shown = clip.intersect(Clip::rect(cx, cy, cx + cw, cy + ch));
            if !shown.is_empty() {
                self.caret = Some([
                    shown.x0 / s,
                    shown.y0 / s,
                    (shown.x1 - shown.x0) / s,
                    (shown.y1 - shown.y0) / s,
                ]);
            }
            dl.prims.push(solid(cx, cy, cw, ch, fade(st.caret), clip));
        }
        if let Some((node_box, view, total, off, max, marks)) = bar_job {
            self.paint_bar(
                key, true, node_box, outer_clip, view, total, off, max, &marks, opacity, dl,
            );
        }
    }

    /// A scroller's overlay bar (bars.rs): over the right edge of `bx`
    /// (device pixels), a view `view` long of `total` scrolled `off` of
    /// `max` (logical pixels); the marks on its track (`(y, colour,
    /// caret)`, `y` in the whole) drawn whether or not the thumb shows.
    /// Kept for the pointer (`bar_hits`); the fade asks for its frames.
    #[allow(clippy::too_many_arguments)]
    fn paint_bar(
        &mut self,
        key: &str,
        rich: bool,
        bx: [f32; 4],
        clip: Clip,
        view: f32,
        total: f32,
        off: f32,
        max: f32,
        marks: &[(f32, Color, bool)],
        opacity: f32,
        dl: &mut DisplayList,
    ) {
        use super::bars;
        let s = self.scale;
        let (l, t, wl, hl) = (bx[0] / s, bx[1] / s, bx[2] / s, bx[3] / s);
        if wl < bars::ZONE * 2.0 || hl < bars::MIN_THUMB + 2.0 * bars::INSET {
            return;
        }
        let right = l + wl;
        let track = (t + bars::INSET, hl - 2.0 * bars::INSET);
        // the zone the pointer finds the bar in: the right edge, as much
        // of it as shows
        let z =
            Clip::rect((right - bars::ZONE) * s, t * s, right * s, (t + hl) * s).intersect(clip);
        let hit = bars::Hit {
            key: key.to_string(),
            zone: [z.x0 / s, z.y0 / s, z.x1 / s, z.y1 / s],
            track,
            view,
            total,
            off,
            max,
            rich,
        };
        let (ty, th) = hit.thumb();
        self.bar_hits.push(hit);
        let now = self.clock_ms();
        let b = self.bars.get(key).copied().unwrap_or_default();
        let a = bars::alpha(&b, now, self.settings.reduce_motion) * opacity;
        if let Some(d) = bars::due(&b, now) {
            self.next_frame = Some(self.next_frame.map_or(d, |n| n.min(d)));
        }
        let clip = clip.intersect(Clip::rect(l * s, t * s, right * s, (t + hl) * s));
        let wide = b.hover || b.drag.is_some();
        let bw = if wide { bars::WIDE } else { bars::THIN };
        let x0 = right - bars::INSET - bw;
        let bg = self.bg_under(key);
        if a > 0.0 {
            if wide {
                // the track under the pointer: a quiet well its length
                dl.prims.push(Prim::Rect {
                    x: (x0 - 1.0) * s,
                    y: track.0 * s,
                    w: (bw + 2.0) * s,
                    h: track.1 * s,
                    fill: bars::ink(bg, 0.06 * a),
                    border: [0, 0, 0, 0],
                    border_width: 0.0,
                    radii: [(bw / 2.0 + 1.0) * s; 4],
                    clip,
                });
            }
            let strength = if b.drag.is_some() {
                0.55
            } else if wide {
                0.45
            } else {
                0.32
            };
            dl.prims.push(Prim::Rect {
                x: x0 * s,
                y: ty * s,
                w: bw * s,
                h: th * s,
                fill: bars::ink(bg, strength * a),
                border: [0, 0, 0, 0],
                border_width: 0.0,
                radii: [bw / 2.0 * s; 4],
                clip,
            });
        }
        // the marks, after the thumb so it never hides them: a line's
        // place in the whole text, 2 high (the caret's a hairline across)
        let fade =
            |c: Color| -> Color { [c[0], c[1], c[2], (c[3] as f32 * opacity).round() as u8] };
        let shown_marks = marks.iter().any(|m| !m.2) || a > 0.0;
        if shown_marks {
            for &(my, c, caret) in marks {
                let y = bars::mark_y(track.0, track.1, my, total.max(1.0));
                if caret {
                    let mut ink = bars::ink(bg, 0.7);
                    ink = fade(ink);
                    dl.prims.push(solid(
                        (right - bars::INSET - bars::WIDE) * s,
                        (y * s).round(),
                        bars::WIDE * s,
                        (1.0 * s).max(1.0),
                        ink,
                        clip,
                    ));
                } else {
                    dl.prims.push(Prim::Rect {
                        x: (right - bars::INSET - bars::THIN) * s,
                        y: ((y - 1.0) * s).round(),
                        w: bars::THIN * s,
                        h: 2.0 * s,
                        fill: fade(c),
                        border: [0, 0, 0, 0],
                        border_width: 0.0,
                        radii: [0.5 * s; 4],
                        clip,
                    });
                }
            }
        }
    }

    /// The background `key` is drawn on: its own, else the nearest
    /// ancestor's that is opaque, else the window's.
    fn bg_under(&self, key: &str) -> Color {
        let mut at = self.scene.nodes.get(key);
        while let Some(n) = at {
            // the node's own look (a hover's fill is not what the bar
            // stands on)
            if let Some(c) = n.style.bg
                && c[3] > 200
            {
                return c;
            }
            at = n.parent.as_ref().and_then(|p| self.scene.nodes.get(p));
        }
        self.clear
    }

    /// Whether a bar is fading now (a headless clock moved: draw again).
    fn bars_moving(&self) -> bool {
        let now = self.clock_ms();
        self.bars
            .values()
            .any(|b| super::bars::due(b, now).is_some())
    }

    /// A scroller moved: its bar shows (and fades from now).
    fn bar_moved(&mut self, key: &str) {
        let now = self.clock_ms();
        self.bars.entry(key.to_string()).or_default().moved_at = Some(now);
    }

    /// The bar under the pointer, if the scroller it belongs to is what
    /// the pointer is over (not a layer above it): the last drawn first.
    ///
    /// A styled field's bar lies in its own right padding: the pointer
    /// there finds it always. A region's lies over its content (a row's
    /// button at its end): it is found only while it shows, as the Mac's
    /// overlay scrollers are, so a press on the content is the content's.
    fn bar_at(&self, x: f32, y: f32, hit: Option<&str>) -> Option<super::bars::Hit> {
        let h = hit?;
        let now = self.clock_ms();
        self.bar_hits
            .iter()
            .rev()
            .find(|b| {
                b.contains(x, y)
                    && (b.key == h || self.scene.is_ancestor(&b.key, h))
                    && (b.rich
                        || self
                            .bars
                            .get(&b.key)
                            .is_some_and(|bar| super::bars::alpha(bar, now, false) > 0.0))
            })
            .cloned()
    }

    /// A scroller put at `off` (logical pixels) by its bar: a region says
    /// `scrolled`, a styled field its first line (`viewport`) as the
    /// wheel's do.
    fn bar_scroll(&mut self, key: &str, rich: bool, off: f32, out: &mut Vec<Value>) {
        if rich {
            let s = self.scale;
            let Ok(mut ts) = text::system().lock() else {
                return;
            };
            let Some(r) = self.editors.get_mut(key).and_then(|e| e.rich_mut()) else {
                return;
            };
            if r.scroll_set(off * s, &mut ts) {
                let top = r.top_line(&mut ts);
                let band = r.band.max(1);
                if r.said_top.map(|t| t / band) != Some(top / band) {
                    r.said_top = Some(top);
                    drop(ts);
                    out.push(self.ev(
                        "viewport",
                        vec![
                            ("key", super::values::s(key)),
                            ("top", Value::Integer(top as i64)),
                        ],
                    ));
                }
                self.dirty = true;
            }
        } else {
            let Some(n) = self.scene.nodes.get_mut(key) else {
                return;
            };
            let before = n.offset;
            n.offset.1 = off;
            self.scene.clamp_scroll(key);
            let after = self.scene.nodes[key].offset;
            if after != before {
                self.dirty = true;
                self.a11y_dirty = true;
                out.push(self.ev(
                    "scrolled",
                    vec![
                        ("key", super::values::s(key)),
                        ("x", float(after.0)),
                        ("y", float(after.1)),
                    ],
                ));
            }
        }
        self.bar_moved(key);
    }

    /// The pointer and the scroll bars: a drag goes on and ends, a press
    /// on a bar takes its thumb or pages, the pointer over an edge shows
    /// its bar wide. Answers whether the bar took the event (the program
    /// hears nothing of it).
    fn bar_pointer(
        &mut self,
        action: PointerAction,
        x: f32,
        y: f32,
        button: &str,
        hit: Option<&str>,
        out: &mut Vec<Value>,
    ) -> bool {
        // a headless window draws when read: a press finds the bars it
        // would show now (a dirty one was drawn for the press above); one
        // never drawn draws once
        if self.headless && self.bar_hits.is_empty() && action == PointerAction::Down {
            let _ = self.display_list();
        }
        let now = self.clock_ms();
        if let Some(k) = self.bar_drag.clone() {
            match action {
                PointerAction::Move => {
                    let grab = self.bars.get(&k).and_then(|b| b.drag).unwrap_or(0.0);
                    if let Some(h) = self.bar_hits.iter().find(|b| b.key == k).cloned() {
                        self.bar_scroll(&k, h.rich, h.offset_for(y - grab), out);
                    }
                    return true;
                }
                PointerAction::Up | PointerAction::Leave => {
                    self.bar_drag = None;
                    if let Some(b) = self.bars.get_mut(&k) {
                        b.drag = None;
                        b.moved_at = Some(now);
                    }
                    self.dirty = true;
                    return action == PointerAction::Up;
                }
                _ => return true,
            }
        }
        let over = if action == PointerAction::Leave {
            None
        } else {
            self.bar_at(x, y, hit)
        };
        // the pointer over an edge: that bar wide, any other back to its fade
        let over_key = over.as_ref().map(|h| h.key.clone());
        for (k, b) in self.bars.iter_mut() {
            let on = over_key.as_deref() == Some(k.as_str());
            if b.hover != on {
                b.hover = on;
                if !on {
                    b.moved_at = Some(now);
                }
                self.dirty = true;
            }
        }
        if let Some(k) = &over_key
            && !self.bars.contains_key(k)
        {
            self.bars.insert(
                k.clone(),
                super::bars::Bar {
                    moved_at: None,
                    hover: true,
                    drag: None,
                },
            );
            self.dirty = true;
        }
        if action == PointerAction::Down
            && button == "left"
            && let Some(h) = over
        {
            let (t, len) = h.thumb();
            if y >= t && y < t + len {
                if let Some(b) = self.bars.get_mut(&h.key) {
                    b.drag = Some(y - t);
                }
                self.bar_drag = Some(h.key.clone());
                self.dirty = true;
            } else {
                let to = h.page_toward(y);
                self.bar_scroll(&h.key, h.rich, to, out);
            }
            self.swallow_up = true;
            return true;
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_editor(
        &mut self,
        key: &str,
        st: &scene::Style,
        content: [f32; 4],
        clip: Clip,
        focused: bool,
        opacity: f32,
        ts: &mut TextSystem,
        dl: &mut DisplayList,
    ) {
        let s = self.scale;
        let node = &self.scene.nodes[key];
        let placeholder = node
            .edit
            .as_ref()
            .map(|e| e.placeholder.clone())
            .unwrap_or_default();
        let secure = node.edit.as_ref().is_some_and(|e| e.secure);
        let window_focused = self.focused_window;
        let ed = self
            .editors
            .get_mut(key)
            .and_then(|e| e.plain_mut())
            .expect("present");
        ed.set_font(&st.font, st.color);
        ed.set_geometry(content[2] / s, s);
        let dy = if ed.multiline {
            0.0
        } else {
            let h = ed.layout(ts).height();
            ((content[3] - h) / 2.0).max(0.0)
        };
        ed.layout(ts);
        ed.follow_caret(content[2]);
        let ox = content[0] - ed.scroll_x;
        let oy = content[1] + dy;
        let clip = clip.intersect(Clip::rect(
            content[0] - 1.0,
            content[1],
            content[0] + content[2] + 1.0,
            content[1] + content[3],
        ));
        let fade =
            |c: Color| -> Color { [c[0], c[1], c[2], (c[3] as f32 * opacity).round() as u8] };
        if focused {
            for (b, _) in ed.ed.selection_geometry() {
                dl.prims.push(Prim::Rect {
                    x: ox + b.x0 as f32,
                    y: oy + b.y0 as f32,
                    w: (b.x1 - b.x0) as f32,
                    h: (b.y1 - b.y0) as f32,
                    fill: fade(st.selection),
                    border: [0, 0, 0, 0],
                    border_width: 0.0,
                    radii: [0.0; 4],
                    clip,
                });
            }
        }
        let empty = ed.ed.raw_text().is_empty();
        if empty && !placeholder.is_empty() {
            let shaped = ts.shape(
                &placeholder,
                &st.font,
                fade(st.placeholder),
                if ed.multiline {
                    Some(content[2] / s)
                } else {
                    None
                },
                Align::Start,
                s,
            );
            push_glyphs(&shaped, ox, oy, clip, dl);
        } else if secure {
            let dots: String = "•".repeat(ed.ed.raw_text().chars().count());
            let shaped = ts.shape(&dots, &st.font, fade(st.color), None, Align::Start, s);
            push_glyphs(&shaped, ox, oy, clip, dl);
        } else {
            let layout = ed.ed.try_layout().expect("laid out");
            let shaped = text::shaped_of(layout, s);
            push_glyphs(&shaped, ox, oy, clip, dl);
            if ed.is_composing()
                && let Some(range) = ed.ed.raw_compose().clone()
            {
                // Underline the composition.
                let sel = parley::Selection::new(
                    parley::Cursor::from_byte_index(
                        layout,
                        range.start,
                        parley::Affinity::Downstream,
                    ),
                    parley::Cursor::from_byte_index(layout, range.end, parley::Affinity::Upstream),
                );
                sel.geometry_with(layout, |b, _| {
                    dl.prims.push(Prim::Rect {
                        x: ox + b.x0 as f32,
                        y: oy + b.y1 as f32 - s,
                        w: (b.x1 - b.x0) as f32,
                        h: s,
                        fill: fade(st.color),
                        border: [0, 0, 0, 0],
                        border_width: 0.0,
                        radii: [0.0; 4],
                        clip,
                    });
                });
            }
        }
        if focused
            && window_focused
            && let Some(c) = ed.ed.cursor_geometry((1.5 * s).max(1.0))
        {
            let (cx, cy, cw, ch) = (
                ox + c.x0 as f32,
                oy + c.y0 as f32,
                (c.x1 - c.x0) as f32,
                (c.y1 - c.y0) as f32,
            );
            let shown = clip.intersect(Clip::rect(cx, cy, cx + cw, cy + ch));
            if !shown.is_empty() {
                self.caret = Some([
                    shown.x0 / s,
                    shown.y0 / s,
                    (shown.x1 - shown.x0) / s,
                    (shown.y1 - shown.y0) / s,
                ]);
            }
            dl.prims.push(Prim::Rect {
                x: cx,
                y: cy,
                w: cw,
                h: ch,
                fill: fade(st.caret),
                border: [0, 0, 0, 0],
                border_width: 0.0,
                radii: [0.0; 4],
                clip,
            });
        }
    }
}

fn pair(a: usize, f: usize) -> Value {
    Value::Tuple(Arc::new(vec![
        Value::Integer(a as i64),
        Value::Integer(f as i64),
    ]))
}

fn rect_value(r: Option<[f32; 4]>) -> Value {
    match r {
        Some(r) => Value::Tuple(Arc::new(r.iter().map(|v| float(*v)).collect())),
        None => Value::Unit,
    }
}

fn solid(x: f32, y: f32, w: f32, h: f32, fill: Color, clip: Clip) -> Prim {
    Prim::Rect {
        x,
        y,
        w,
        h,
        fill,
        border: [0, 0, 0, 0],
        border_width: 0.0,
        radii: [0.0; 4],
        clip,
    }
}

/// Text after a line's end cut to `keep` characters, at a word's end,
/// with an ellipsis (no colon or comma left hanging before it); `None`
/// when what would show says too little (under 16 characters, under a
/// third of it, or a "what: which" cut before its which), so it gives
/// way whole rather than as a stub.
pub fn eol_cut(label: &str, keep: usize) -> Option<String> {
    let n = label.chars().count();
    if keep >= n {
        return Some(label.to_string());
    }
    if keep < 16 || keep * 3 < n {
        return None;
    }
    let head: String = label.chars().take(keep).collect();
    let at_word = match head.rfind(char::is_whitespace) {
        Some(b) if head[..b].chars().count() >= 12 => &head[..b],
        _ => head.as_str(),
    };
    let trimmed = at_word
        .trim_end()
        .trim_end_matches([':', ',', ';', '-', '—', '('])
        .trim_end();
    if trimmed.chars().count() < 12 {
        return None;
    }
    // "what: which" cut before the which says nothing of use
    if let Some(p) = label.find(": ")
        && trimmed.len() <= p + 1
    {
        return None;
    }
    Some(trimmed.to_string() + "…")
}

fn push_glyphs(shaped: &Shaped, ox: f32, oy: f32, clip: Clip, dl: &mut DisplayList) {
    // a span's background behind its glyphs
    for d in shaped.decos.iter().filter(|d| !d.over) {
        dl.prims
            .push(solid(ox + d.x, oy + d.y, d.w, d.h, d.color, clip));
    }
    for run in &shaped.runs {
        for (id, gx, gy) in &run.glyphs {
            let x = ox + gx;
            let y = (oy + gy).round();
            dl.prims.push(Prim::Glyph {
                x,
                y,
                key: GlyphKey::new(
                    run.face,
                    *id,
                    run.size,
                    x,
                    &run.coords,
                    run.embolden,
                    run.skew,
                ),
                coords: run.coords.clone(),
                color: run.color,
                clip,
            });
        }
    }
    // a strike or an underline over them
    for d in shaped.decos.iter().filter(|d| d.over) {
        dl.prims
            .push(solid(ox + d.x, oy + d.y, d.w, d.h, d.color, clip));
    }
}

// ── the display list ────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    /// Corner radii (top-left, top-right, bottom-right, bottom-left): a
    /// clipping node with rounded corners clips its children to them.
    pub radii: [f32; 4],
}

impl Clip {
    pub fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Clip {
        Clip {
            x0,
            y0,
            x1,
            y1,
            radii: [0.0; 4],
        }
    }

    pub fn is_rounded(&self) -> bool {
        self.radii.iter().any(|r| *r > 0.0)
    }

    /// The overlap of two clips. Its corners are the rounded ones of
    /// whichever clip it coincides with (nested rounded clips keep the
    /// inner one's corners — exact for a child inside its parent's
    /// border, which is the case clipping is for).
    pub fn intersect(self, o: Clip) -> Clip {
        let x0 = self.x0.max(o.x0);
        let y0 = self.y0.max(o.y0);
        let x1 = self.x1.min(o.x1).max(x0);
        let y1 = self.y1.min(o.y1).max(y0);
        let same = |c: &Clip| c.x0 == x0 && c.y0 == y0 && c.x1 == x1 && c.y1 == y1;
        let radii = if same(&o) && o.is_rounded() {
            o.radii
        } else if same(&self) {
            self.radii
        } else {
            [0.0; 4]
        };
        Clip {
            x0,
            y0,
            x1,
            y1,
            radii,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
}

/// What both renderers draw, in device pixels, in order.
#[derive(Debug)]
pub enum Prim {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Color,
        border: Color,
        border_width: f32,
        radii: [f32; 4],
        clip: Clip,
    },
    Glyph {
        /// The pen position; the subpixel part is in the key.
        x: f32,
        /// The baseline, a whole pixel.
        y: f32,
        key: GlyphKey,
        coords: Arc<[i16]>,
        color: Color,
        clip: Clip,
    },
    /// A picture (a canvas's drawing, an image) scaled into a box.
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        pic: Arc<super::canvas::Picture>,
        clip: Clip,
        /// The picture's opacity (its node's and its ancestors'), 0 … 1.
        alpha: f32,
    },
}

#[derive(Debug)]
pub struct DisplayList {
    pub width: u32,
    pub height: u32,
    pub clear: Color,
    pub prims: Vec<Prim>,
    /// The scrolled regions drawn, for damage to follow their moves.
    pub scrolls: Vec<super::damage::Scroll>,
}

/// A hover that left the text: no line or column.
fn hover_left(key: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("key", Value::String(Arc::new(key.to_string()))),
        ("line", Value::Unit),
        ("column", Value::Unit),
    ]
}

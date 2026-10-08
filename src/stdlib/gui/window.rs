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

/// A styled field says its first line in view when it moves into another
/// band of this many lines.
const VIEWPORT_BAND: usize = 16;

pub struct WinState {
    pub id: u64,
    pub headless: bool,
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
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<(u32, bool), Arc<super::canvas::Picture>>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Ok(c) = cache.lock()
        && let Some(p) = c.get(&(side, play))
    {
        return p.clone();
    }
    let side = side.max(8);
    let mut pm = tiny_skia::Pixmap::new(side, side).expect("a small pixmap");
    let f = side as f32;
    let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
    paint.set_color_rgba8(0, 0, 0, 150);
    if let Some(disc) = tiny_skia::PathBuilder::from_circle(f / 2.0, f / 2.0, f / 2.0) {
        pm.fill_path(&disc, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
    }
    paint.set_color_rgba8(255, 255, 255, 255);
    let mut pb = tiny_skia::PathBuilder::new();
    if play {
        pb.move_to(f * 0.40, f * 0.30);
        pb.line_to(f * 0.72, f * 0.50);
        pb.line_to(f * 0.40, f * 0.70);
        pb.close();
    } else {
        pb.push_rect(tiny_skia::Rect::from_xywh(f * 0.36, f * 0.31, f * 0.10, f * 0.38).expect("a bar"));
        pb.push_rect(tiny_skia::Rect::from_xywh(f * 0.54, f * 0.31, f * 0.10, f * 0.38).expect("a bar"));
    }
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
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
    pub fn new(id: u64, headless: bool, title: &str, width: f32, height: f32, scale: f32) -> Self {
        WinState {
            id,
            headless,
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
        }
    }

    /// Scrollers the window moved itself (to bring the focus into view):
    /// each says so, as the wheel's do.
    pub fn said_scrolled(&mut self, moved: Vec<(String, (f32, f32))>, out: &mut Vec<Value>) {
        for (k, (x, y)) in moved {
            self.a11y_dirty = true;
            out.push(self.ev("scrolled", vec![("key", s(&k)), ("x", float(x)), ("y", float(y))]));
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
            if !self.shown_anims.is_empty() {
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
                        Field::Plain(Editor::new(
                            &props.value,
                            &style.font,
                            style.color,
                            props.multiline,
                            props.secure,
                        ))
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
                ("top", top.map(|t| Value::Integer(t as i64)).unwrap_or(Value::Unit)),
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
        let place = self.rich_place(key);
        out.push(self.ev(
            "select",
            vec![
                ("key", s(key)),
                ("place", place),
                ("selection", pair(sel.0, sel.1)),
                ("rev", Value::Integer(rev)),
                ("caret", rect_value(caret)),
                ("top", top.map(|t| Value::Integer(t as i64)).unwrap_or(Value::Unit)),
                ("click_mod", Value::Boolean(click_mod)),
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
                if let Some(f) = self.scene.focus.clone()
                    && let Some(ed) = self.editors.get_mut(&f)
                    && let Ok(mut ts) = text::system().lock()
                {
                    ed.compose(&t, cursor, &mut ts);
                    self.dirty = true;
                }
            }
            Input::ImeCommit(t) => {
                if let Some(f) = self.scene.focus.clone() {
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
        let passed = plain
            && !composing
            && self
                .scene
                .focus
                .as_ref()
                .and_then(|f| self.scene.nodes.get(f))
                .and_then(|n| n.edit.as_ref())
                .is_some_and(|e| e.pass_keys.iter().any(|k| *k == spelled));
        if key == "tab" && plain && !passed {
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
            && !matches!(key, "left" | "right" | "up" | "down" | "backspace" | "delete")
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
        if action == PointerAction::Up && self.swallow_up {
            self.swallow_up = false;
            return;
        }
        // the pointer over a playing animation shows its pause button
        if action == PointerAction::Move && self.shown_anims.iter().any(|a| a.badge.is_some() && !a.paused) {
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
                        // a press in a lens is the program's: said with the
                        // operation under it, the caret left where it was
                        let lens = ed.rich_mut().and_then(|r| {
                            let (k, ly) = r.lens_at(ty, &mut ts)?;
                            let l = r.lenses()[k].clone();
                            Some((l, ly / self.scale))
                        });
                        if let Some((l, ly)) = lens {
                            drop(ts);
                            let lx = tx / self.scale;
                            let width = self.scene.nodes.get(f).map(|n| n.rect[2]).unwrap_or(0.0);
                            let hit = super::canvas::hit(&l.ops, width, l.height, lx, ly);
                            let f = f.clone();
                            out.push(self.ev(
                                "lens",
                                vec![
                                    ("key", s(&f)),
                                    ("line", Value::Integer(l.line as i64)),
                                    ("id", l.id.clone()),
                                    ("hit", hit),
                                    ("x", float(lx)),
                                    ("y", float(ly)),
                                    ("clicks", Value::Integer(clicks as i64)),
                                ],
                            ));
                        } else {
                            self.click_mod = cmd;
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
            && r.wheel(dy, &mut ts)
        {
            // the first line in view, said when it moves a band of lines
            // (a program asks for what the band shows; not every wheel)
            let top = r.top_line(&mut ts);
            if r.said_top.map(|t| t / VIEWPORT_BAND) != Some(top / VIEWPORT_BAND) {
                r.said_top = Some(top);
                drop(ts);
                out.push(self.ev("viewport", vec![("key", s(&k)), ("top", Value::Integer(top as i64))]));
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

    /// Where the input method should place its candidate window: the
    /// focused field's composition or caret, in logical pixels.
    pub fn ime_area(&mut self) -> Option<[f32; 4]> {
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

    /// Whether the focused node is a text field (the input method is
    /// allowed only then).
    pub fn wants_ime(&self) -> bool {
        self.scene
            .focus
            .as_ref()
            .is_some_and(|f| self.editors.contains_key(f))
    }

    // ── the display list ────────────────────────────────────────────

    pub fn display_list(&mut self) -> DisplayList {
        let s = self.scale;
        let pw = (self.width * s).round().max(1.0) as u32;
        let ph = (self.height * s).round().max(1.0) as u32;
        let mut dl = DisplayList {
            width: pw,
            height: ph,
            clear: self.clear,
            prims: Vec::new(),
        };
        self.caret = None;
        self.shown_anims.clear();
        self.next_frame = None;
        let Some(root) = self.scene.root.clone() else {
            return dl;
        };
        let Ok(mut ts) = text::system().lock() else {
            return dl;
        };
        let full = Clip::rect(0.0, 0.0, pw as f32, ph as f32);
        let mut rings = Vec::new();
        self.paint(&root, 0.0, 0.0, full, 1.0, &mut ts, &mut dl, &mut rings);
        // Focus rings draw last, over their neighbours, unclipped by the
        // node's own clip but within the window.
        dl.prims.extend(rings);
        dl
    }

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
        let st = self.scene.effective_style(node);
        let s = self.scale;
        let x = (ox + node.rect[0]) * s;
        let y = (oy + node.rect[1]) * s;
        let w = node.rect[2] * s;
        let h = node.rect[3] * s;
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
        let (children, offset, rect) = (node.children.clone(), node.offset, node.rect);
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
                    let p = self.plays.entry(key.to_string()).or_insert(Play { id: first, start: now, paused: None, chosen: false });
                    if p.id != first {
                        *p = Play { id: first, start: now, paused: None, chosen: false };
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
                        let (bx, by) = (ox + rect[0] + rect[2] / 2.0 - side_l / 2.0, oy + rect[1] + rect[3] / 2.0 - side_l / 2.0);
                        Some([bx, by, side_l, side_l])
                    } else {
                        None
                    };
                    let (px, py) = self.pointer;
                    let over = px >= ox + rect[0] && py >= oy + rect[1] && px < ox + rect[0] + rect[2] && py < oy + rect[1] + rect[3];
                    if let Some(r) = brect
                        && (held.is_some() || over)
                    {
                        badge = Some((r, held.is_some()));
                    }
                    self.shown_anims.push(AnimShown { key: key.to_string(), frames: anim.frames.len(), frame: i, paused: held.is_some(), badge: brect });
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
            let mut shaped = ts.shape(t, &st.font, fade(st.color), align_w, st.align, s);
            // a line wider than its box, cut to fit with an ellipsis
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
        let (cx, cy) = (ox + rect[0] - offset.0, oy + rect[1] - offset.1);
        for c in children {
            self.paint(&c, cx, cy, inner_clip, opacity, ts, dl, rings);
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
            self.scene.target_up(h, |n| n.edit.as_ref().is_some_and(|e| e.rich && e.hover))
        });
        // the field the pointer left: said once
        let left: Vec<String> = self
            .editors
            .iter()
            .filter(|(k, e)| Some(k.as_str()) != target.as_deref() && e.rich().is_some_and(|r| r.said_hover.is_some()))
            .map(|(k, _)| k.clone())
            .collect();
        for k in left {
            if let Some(r) = self.editors.get_mut(&k).and_then(|e| e.rich_mut()) {
                r.said_hover = None;
            }
            out.push(self.ev("hover", vec![("key", s(&k)), ("line", Value::Integer(-1)), ("col", Value::Integer(-1))]));
        }
        let Some(k) = target else { return };
        if self.scene.pressed.is_some() {
            return;
        }
        let Some((ox, oy, _, h)) = self.edit_origin(&k) else { return };
        let Ok(mut ts) = text::system().lock() else { return };
        let dy = self.edit_text_dy(&k, h, &mut ts);
        let sc = self.scale;
        let Some(ed) = self.editors.get_mut(&k) else { return };
        let (sx, sy) = ed.scrolled();
        let Some(r) = ed.rich_mut() else { return };
        let found = r.hover_at(x * sc - ox + sx, y * sc - oy - dy + sy, &mut ts);
        drop(ts);
        let now = found.as_ref().map(|(l, c, _)| (*l, *c));
        if now == r.said_hover {
            return;
        }
        r.said_hover = now;
        let ev = match found {
            Some((line, col, b)) => {
                let bx = (ox + b.x0 as f32 - sx) / sc;
                let by = (oy + dy + b.y0 as f32 - sy) / sc;
                let rect = Value::Tuple(Arc::new(vec![
                    float(bx),
                    float(by),
                    float(((b.x1 - b.x0) as f32 / sc).max(1.0)),
                    float((b.y1 - b.y0) as f32 / sc),
                ]));
                vec![("key", s(&k)), ("line", Value::Integer(line as i64)), ("col", Value::Integer(col as i64)), ("rect", rect)]
            }
            None => vec![("key", s(&k)), ("line", Value::Integer(-1)), ("col", Value::Integer(-1))],
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
        r.set_geometry(content[2] / s, s);
        r.follow_caret(content[3], ts);
        let ox = content[0];
        let oy = content[1] - r.scroll_y;
        let outer_clip = clip;
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
            for i in shown.clone() {
                let Some((a, b, more)) = r.para_selection(i) else {
                    continue;
                };
                let (py, _, ph) = r.para(i);
                let boxes = r.range_boxes(i, a, b);
                let mut end_x = 0.0f32;
                for bx in &boxes {
                    dl.prims.push(solid(
                        ox + bx.x0 as f32,
                        oy + py + bx.y0 as f32,
                        (bx.x1 - bx.x0) as f32,
                        (bx.y1 - bx.y0) as f32,
                        fade(st.selection),
                        clip,
                    ));
                    end_x = bx.x1 as f32;
                }
                if more {
                    // the line break is selected too
                    let last = boxes.last().map(|b| (b.y0 as f32, (b.y1 - b.y0) as f32));
                    let (y0, h) =
                        last.unwrap_or((ph - st.font.size * s * 1.2, st.font.size * s * 1.2));
                    dl.prims.push(solid(
                        ox + end_x,
                        oy + py + y0,
                        st.font.size * s * 0.4,
                        h,
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
            push_glyphs(&shaped, ox, oy + py, clip, dl);
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
        // lenses: each band under its line, drawn after the text is (the
        // field is let go first: a canvas paints through the window)
        let mut lens_jobs: Vec<(Value, f32, f32, f32, f32)> = Vec::new();
        for (k, lens) in r.lenses().iter().enumerate() {
            if !shown.contains(&lens.line) {
                continue;
            }
            if let Some((top, h)) = r.lens_band(k) {
                lens_jobs.push((lens.ops.clone(), content[0], oy + top, content[2] + st.pad[1] * s, h));
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
        // a code editor's gutter, in the left padding: the line numbers
        // (the caret's brighter) and the marks
        if let Some(g) = r.gutter.clone() {
            let gx0 = content[0] - st.pad[3] * s;
            let gclip = outer_clip.intersect(Clip::rect(gx0, content[1], content[0], content[1] + content[3]));
            let fi = r.focus_para();
            let mut font = st.font.clone();
            font.spans = None;
            let base = g.color.unwrap_or(st.placeholder);
            if g.numbers {
                for i in shown.clone() {
                    let (py, _, _) = r.para(i);
                    let color = if i == fi { g.current.unwrap_or(st.color) } else { base };
                    let label = (i + 1).to_string();
                    let shaped = ts.shape(&label, &font, fade(color), None, Align::Start, s);
                    let x = content[0] - 14.0 * s - shaped.width;
                    push_glyphs(&shaped, x, oy + py, gclip, dl);
                }
            }
            for (line, glyph, color) in &g.marks {
                if !shown.contains(line) {
                    continue;
                }
                let (py, _, _) = r.para(*line);
                let mut mf = font.clone();
                mf.size = font.size * 0.8;
                let shaped = ts.shape(glyph, &mf, fade(color.unwrap_or(base)), None, Align::Start, s);
                let lh = font.size * s * 1.2;
                push_glyphs(&shaped, gx0 + 6.0 * s, oy + py + (lh - shaped.height).max(0.0) / 2.0, gclip, dl);
            }
        }
        // a scroll mark when the text is longer than its box
        let total = r.content_height(ts);
        if total > content[3] + 1.0 && content[3] > 0.0 {
            let track = content[3];
            let h = (track * track / total).max(24.0 * s).min(track);
            let y = content[1] + (track - h) * (r.scroll_y / (total - track).max(1.0));
            let mut c = st.placeholder;
            c[3] = (c[3] as f32 * 0.45) as u8;
            dl.prims.push(Prim::Rect {
                x: content[0] + content[2] + 2.0 * s,
                y,
                w: 3.0 * s,
                h,
                fill: fade(c),
                border: [0, 0, 0, 0],
                border_width: 0.0,
                radii: [1.5 * s; 4],
                clip: clip.intersect(Clip::rect(
                    content[0],
                    content[1],
                    content[0] + content[2] + 6.0 * s,
                    content[1] + content[3],
                )),
            });
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
            content[1],
            content[0] + content[2] + st.pad[1] * s,
            content[1] + content[3],
        ));
        for (ops, lx, ly, lw, lh) in lens_jobs {
            if let Ok(drawing) = super::canvas::draw(&ops, lw / s, lh / s, s) {
                self.paint_canvas(drawing, lx, ly, lw, lh, lens_clip, st, opacity, ts, dl);
            }
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

fn push_glyphs(shaped: &Shaped, ox: f32, oy: f32, clip: Clip, dl: &mut DisplayList) {
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
    },
}

#[derive(Debug)]
pub struct DisplayList {
    pub width: u32,
    pub height: u32,
    pub clear: Color,
    pub prims: Vec<Prim>,
}

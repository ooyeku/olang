//! One window's state, whatever shows it — a platform window or a
//! headless one: the scene, the editors, the size and scale, and the
//! input handling that turns keys, pointer, wheel, and input-method
//! events into scene changes and the events a program hears. Building
//! the display list both renderers draw is here too.
//!
//! Nothing in this file touches the platform: the platform loop and
//! `gui.input` (tests) feed it the same [`Input`] values.

use super::edit::{Editor, Outcome};
use super::raster::GlyphKey;
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

pub struct WinState {
    pub id: u64,
    pub headless: bool,
    pub title: String,
    pub scene: Scene,
    pub editors: HashMap<String, Editor>,
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
            match self.editors.get_mut(k) {
                Some(ed) => {
                    ed.multiline = props.multiline;
                    ed.secure = props.secure;
                    ed.set_font(&style.font, style.color);
                    if ed.offer(&props.value, props.rev, &mut ts) {
                        // Adopted the program's value: say so, with the
                        // revision it now has.
                        let (a, f) = ed.selection_chars();
                        let (v, rev) = (ed.value(), ed.rev);
                        let e = self.changed_event(k, v, rev, a, f);
                        out.push(e);
                    }
                }
                None => {
                    let ed = Editor::new(
                        &props.value,
                        &style.font,
                        style.color,
                        props.multiline,
                        props.secure,
                    );
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
                self.scene.reveal(&f);
            }
            out.push(self.ev("focus", vec![("key", opt_str(self.scene.focus.as_deref()))]));
        }
        self.dirty = true;
        self.a11y_dirty = true;
        Ok(())
    }

    fn changed_event(&self, key: &str, value: String, rev: i64, a: usize, f: usize) -> Value {
        self.ev(
            "changed",
            vec![
                ("key", s(key)),
                ("value", s(&value)),
                ("rev", Value::Integer(rev)),
                (
                    "selection",
                    Value::Tuple(Arc::new(vec![
                        Value::Integer(a as i64),
                        Value::Integer(f as i64),
                    ])),
                ),
            ],
        )
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
            self.scene.reveal(k);
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
        let Some(ed) = self.editors.get_mut(key) else {
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
            } => Input::Pointer {
                action,
                x: x / z,
                y: y / z,
                button,
                clicks,
            },
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
            Input::Wheel { dx, dy } => self.wheel(dx, dy, out),
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
                let ed = &self.editors[key];
                let (a, f) = ed.selection_chars();
                out.push(self.changed_event(key, ed.value(), ed.rev, a, f));
                self.a11y_dirty = true;
            }
            Outcome::Submit => out.push(self.ev("submit", vec![("key", s(key))])),
            Outcome::Moved | Outcome::Pass => {}
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
        let passed = plain
            && !mods.shift
            && self
                .scene
                .focus
                .as_ref()
                .and_then(|f| self.scene.nodes.get(f))
                .and_then(|n| n.edit.as_ref())
                .is_some_and(|e| e.pass_keys.iter().any(|k| *k == key));
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
                ("z", false) => ed.undo(&mut ts),
                ("z", true) | ("y", false) => ed.redo(&mut ts),
                ("left" | "right" | "up" | "down" | "backspace" | "delete", _) => {
                    ed.key(key, mods.shift, mods.word(), mods.line(), &mut ts)
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
        match action {
            PointerAction::Move => {
                if let Some(p) = self.scene.pressed.clone()
                    && self.editors.get(&p).is_some_and(|e| e.dragging)
                    && let Some((ox, oy, _, h)) = self.edit_origin(&p)
                    && let Ok(mut ts) = text::system().lock()
                {
                    let dy = self.edit_text_dy(&p, h, &mut ts);
                    let ed = self.editors.get_mut(&p).expect("present");
                    ed.drag(
                        x * self.scale - ox + ed.scroll_x,
                        y * self.scale - oy - dy,
                        &mut ts,
                    );
                    self.dirty = true;
                }
                out.push(self.ev(
                    "pointer",
                    vec![
                        ("action", s("move")),
                        ("x", float(x)),
                        ("y", float(y)),
                        ("target", opt_str(hit.as_deref())),
                    ],
                ));
            }
            PointerAction::Leave => {
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
                        let ed = self.editors.get_mut(f).expect("present");
                        ed.press(
                            x * self.scale - ox + ed.scroll_x,
                            y * self.scale - oy - dy,
                            clicks,
                            shift,
                            &mut ts,
                        );
                    }
                    self.dirty = true;
                }
                out.push(self.ev(
                    "pointer",
                    vec![
                        ("action", s("down")),
                        ("x", float(x)),
                        ("y", float(y)),
                        ("button", s(button)),
                        ("clicks", Value::Integer(clicks as i64)),
                        ("target", opt_str(hit.as_deref())),
                    ],
                ));
            }
            PointerAction::Up => {
                let pressed = self.scene.pressed.take();
                if let Some(p) = &pressed
                    && let Some(ed) = self.editors.get_mut(p)
                {
                    ed.dragging = false;
                }
                out.push(self.ev(
                    "pointer",
                    vec![
                        ("action", s("up")),
                        ("x", float(x)),
                        ("y", float(y)),
                        ("button", s(button)),
                        ("target", opt_str(hit.as_deref())),
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

    fn wheel(&mut self, dx: f32, dy: f32, out: &mut Vec<Value>) {
        let (x, y) = self.pointer;
        let Some(hit) = self.scene.hit(x, y) else {
            return;
        };
        let mut at = self.scene.scroller_up(&hit);
        while let Some(k) = at {
            let before = self.scene.nodes[&k].offset;
            {
                let n = self.scene.nodes.get_mut(&k).expect("present");
                n.offset.0 -= dx;
                n.offset.1 -= dy;
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
            // Nothing left to scroll here: hand the wheel to the next
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
        let ed = self.editors.get_mut(&f)?;
        ed.layout(&mut ts);
        let b = ed.ed.ime_cursor_area();
        // physical pixels to the platform's logical ones
        let s = self.platform_scale.max(0.01);
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
            && let Ok((pic, texts)) = super::canvas::draw(ops, rect[2], rect[3], s)
        {
            let cclip = inner_clip.intersect(Clip::rect(x, y, x + w, y + h));
            if let Some(pic) = pic {
                dl.prims.push(Prim::Image {
                    x,
                    y,
                    w: pic.width as f32,
                    h: pic.height as f32,
                    pic,
                    clip: cclip,
                });
            }
            for t in texts {
                let mut font = st.font.clone();
                font.size = t.size;
                font.weight = t.weight;
                let shaped = ts.shape(&t.text, &font, fade(t.color), None, Align::Start, s);
                let dx = match t.align {
                    1 => shaped.width / 2.0,
                    2 => shaped.width,
                    _ => 0.0,
                };
                push_glyphs(&shaped, x + t.x * s - dx, y + t.y * s, cclip, dl);
            }
        }
        if let Some(src) = &image
            && let Ok(pic) = super::canvas::image(src)
        {
            // Fit the picture to the box, keeping its proportions unless
            // told to fill.
            let (iw, ih) = (pic.width as f32, pic.height as f32);
            let (sx, sy) = (w / iw.max(1.0), h / ih.max(1.0));
            let k = match fit.as_str() {
                "cover" => sx.max(sy),
                _ => sx.min(sy),
            };
            let (dw, dh) = if fit == "fill" {
                (w, h)
            } else {
                (iw * k, ih * k)
            };
            dl.prims.push(Prim::Image {
                x: x + (w - dw) / 2.0,
                y: y + (h - dh) / 2.0,
                w: dw,
                h: dh,
                pic,
                clip: inner_clip.intersect(Clip::rect(x, y, x + w, y + h)),
            });
        }
        if self.editors.contains_key(key) {
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
        let ed = self.editors.get_mut(key).expect("present");
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

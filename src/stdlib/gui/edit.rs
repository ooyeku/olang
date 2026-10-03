//! Editable text (`input`, `textarea`, `search`): parley's editor, the
//! keys and pointer gestures of a text field, the input method's
//! composition, undo, and the controlled value.
//!
//! The engine edits so that typing, selection, and composition are fast
//! and correct in every script; the program's model stays the authority
//! on the value. Every committed edit bumps a revision and is reported
//! (`changed`, with the revision). A patch whose value differs from the
//! field's is adopted — unless it is the field's own earlier value at an
//! earlier revision, which is the program catching up, not a reset.

use super::text::{Color, Font, TextSystem};
use parley::{GenericFamily, PlainEditor, StyleProperty};
use std::collections::VecDeque;

pub struct Editor {
    pub ed: PlainEditor<Color>,
    pub rev: i64,
    /// (revision, value) reported recently.
    sent: VecDeque<(i64, String)>,
    pub multiline: bool,
    pub secure: bool,
    font: Font,
    color: Color,
    scale: f32,
    width: Option<f32>,
    undo: Vec<(String, usize, usize)>,
    redo: Vec<(String, usize, usize)>,
    /// Horizontal scroll of a single-line field, device pixels.
    pub scroll_x: f32,
    /// The pointer is dragging a selection.
    pub dragging: bool,
}

/// What an edit did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The key is not the field's (Enter in a single-line field, Escape,
    /// Tab, a command chord): the window handles it.
    Pass,
    /// Handled; nothing the program needs to hear.
    Moved,
    /// The value changed.
    Changed,
    /// Enter in a single-line field.
    Submit,
}

impl Editor {
    pub fn new(value: &str, font: &Font, color: Color, multiline: bool, secure: bool) -> Editor {
        let mut ed = PlainEditor::new(font.size);
        ed.set_text(value);
        let mut e = Editor {
            ed,
            rev: 0,
            sent: VecDeque::new(),
            multiline,
            secure,
            font: Font::default(),
            color,
            scale: 1.0,
            width: None,
            undo: Vec::new(),
            redo: Vec::new(),
            scroll_x: 0.0,
            dragging: false,
        };
        e.apply_font(font, color);
        e
    }

    pub fn set_font(&mut self, font: &Font, color: Color) {
        if self.font == *font && self.color == color {
            return;
        }
        self.apply_font(font, color);
    }

    fn apply_font(&mut self, font: &Font, color: Color) {
        self.font = font.clone();
        self.color = color;
        let styles = self.ed.edit_styles();
        for st in font.styles() {
            styles.insert(st);
        }
        styles.insert(StyleProperty::Brush(color));
        if font.family == "mono" {
            styles.insert(StyleProperty::FontFamily(parley::FontFamily::Single(
                parley::FontFamilyName::Generic(GenericFamily::Monospace),
            )));
        }
    }

    pub fn set_geometry(&mut self, width: f32, scale: f32) {
        let w = if self.multiline {
            Some(width * scale)
        } else {
            None
        };
        if self.scale != scale {
            self.scale = scale;
            self.ed.set_scale(scale);
        }
        if self.width != w {
            self.width = w;
            self.ed.set_width(w);
        }
    }

    pub fn value(&self) -> String {
        self.ed.text().to_string()
    }

    /// The selection as character offsets (anchor, focus) into the value.
    pub fn selection_chars(&self) -> (usize, usize) {
        let sel = self.ed.raw_selection();
        let text = self.ed.raw_text();
        let ch = |b: usize| text[..b.min(text.len())].chars().count();
        (ch(sel.anchor().index()), ch(sel.focus().index()))
    }

    /// The program's value from a patch.
    pub fn offer(&mut self, value: &str, rev: Option<i64>, ts: &mut TextSystem) -> bool {
        if self.ed.is_composing() {
            // Never replace text under an active composition; the commit
            // will be reported and the program will answer it.
            return false;
        }
        let current = self.value();
        if current == value {
            return false;
        }
        if let Some(r) = rev
            && r < self.rev
            && self.sent.iter().any(|(sr, sv)| *sr == r && sv == value)
        {
            return false;
        }
        let caret = value.len();
        self.ed.set_text(value);
        let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
        d.move_to_byte(caret);
        self.rev += 1;
        self.remember();
        true
    }

    fn remember(&mut self) {
        let v = self.value();
        self.sent.push_back((self.rev, v));
        while self.sent.len() > 32 {
            self.sent.pop_front();
        }
    }

    fn snapshot(&self) -> (String, usize, usize) {
        let sel = self.ed.raw_selection();
        (
            self.ed.raw_text().to_string(),
            sel.anchor().index(),
            sel.focus().index(),
        )
    }

    fn restore(&mut self, snap: (String, usize, usize), ts: &mut TextSystem) {
        self.ed.set_text(&snap.0);
        let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
        d.select_byte_range(snap.1, snap.2);
    }

    /// Run an edit that may change the value: record undo, bump the
    /// revision when it did.
    fn mutate(
        &mut self,
        ts: &mut TextSystem,
        f: impl FnOnce(&mut parley::PlainEditorDriver<'_, Color>),
    ) -> Outcome {
        let before = self.snapshot();
        {
            let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
            f(&mut d);
        }
        if self.ed.raw_text() == before.0 {
            return Outcome::Moved;
        }
        self.undo.push(before);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.rev += 1;
        self.remember();
        Outcome::Changed
    }

    fn movement(
        &mut self,
        ts: &mut TextSystem,
        f: impl FnOnce(&mut parley::PlainEditorDriver<'_, Color>),
    ) -> Outcome {
        let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
        f(&mut d);
        Outcome::Moved
    }

    /// Typed text (a key's text, a committed composition, a paste).
    pub fn insert(&mut self, text: &str, ts: &mut TextSystem) -> Outcome {
        let text = if self.multiline {
            text.replace("\r\n", "\n")
        } else {
            text.replace(['\r', '\n'], " ")
        };
        if text.is_empty() {
            return Outcome::Moved;
        }
        self.mutate(ts, |d| d.insert_or_replace_selection(&text))
    }

    /// A key, by its name (Heddle's spelling: `left`, `backspace`,
    /// `home`) and modifiers. `word` is alt on macOS, ctrl elsewhere;
    /// `line` is cmd on macOS (home/end elsewhere).
    pub fn key(
        &mut self,
        key: &str,
        shift: bool,
        word: bool,
        line: bool,
        ts: &mut TextSystem,
    ) -> Outcome {
        match key {
            "left" => self.movement(ts, |d| match (shift, word, line) {
                (s, _, true) => {
                    if s {
                        d.select_to_line_start()
                    } else {
                        d.move_to_line_start()
                    }
                }
                (true, true, _) => d.select_word_left(),
                (true, false, _) => d.select_left(),
                (false, true, _) => d.move_word_left(),
                (false, false, _) => d.move_left(),
            }),
            "right" => self.movement(ts, |d| match (shift, word, line) {
                (s, _, true) => {
                    if s {
                        d.select_to_line_end()
                    } else {
                        d.move_to_line_end()
                    }
                }
                (true, true, _) => d.select_word_right(),
                (true, false, _) => d.select_right(),
                (false, true, _) => d.move_word_right(),
                (false, false, _) => d.move_right(),
            }),
            // Up and down move by line in a multi-line field; in a
            // single-line field, and with the line modifier, they go to
            // the start or the end of the text.
            "up" if self.multiline && !line => {
                self.movement(ts, |d| if shift { d.select_up() } else { d.move_up() })
            }
            "down" if self.multiline && !line => self.movement(ts, |d| {
                if shift {
                    d.select_down()
                } else {
                    d.move_down()
                }
            }),
            "up" => self.movement(ts, |d| {
                if shift {
                    d.select_to_text_start()
                } else {
                    d.move_to_text_start()
                }
            }),
            "down" => self.movement(ts, |d| {
                if shift {
                    d.select_to_text_end()
                } else {
                    d.move_to_text_end()
                }
            }),
            "home" => self.movement(ts, |d| {
                if shift {
                    d.select_to_line_start()
                } else {
                    d.move_to_line_start()
                }
            }),
            "end" => self.movement(ts, |d| {
                if shift {
                    d.select_to_line_end()
                } else {
                    d.move_to_line_end()
                }
            }),
            "backspace" => self.mutate(ts, |d| {
                if word {
                    d.backdelete_word()
                } else {
                    d.backdelete()
                }
            }),
            "delete" => self.mutate(ts, |d| if word { d.delete_word() } else { d.delete() }),
            "enter" if self.multiline => self.insert("\n", ts),
            "enter" => Outcome::Submit,
            _ => Outcome::Pass,
        }
    }

    pub fn select_all(&mut self, ts: &mut TextSystem) {
        let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
        d.select_all();
    }

    pub fn selected_text(&self) -> Option<String> {
        if self.secure {
            return None;
        }
        self.ed.selected_text().map(str::to_string)
    }

    pub fn delete_selection(&mut self, ts: &mut TextSystem) -> Outcome {
        self.mutate(ts, |d| d.delete_selection())
    }

    pub fn undo(&mut self, ts: &mut TextSystem) -> Outcome {
        let Some(snap) = self.undo.pop() else {
            return Outcome::Moved;
        };
        let now = self.snapshot();
        self.redo.push(now);
        self.restore(snap, ts);
        self.rev += 1;
        self.remember();
        Outcome::Changed
    }

    pub fn redo(&mut self, ts: &mut TextSystem) -> Outcome {
        let Some(snap) = self.redo.pop() else {
            return Outcome::Moved;
        };
        let now = self.snapshot();
        self.undo.push(now);
        self.restore(snap, ts);
        self.rev += 1;
        self.remember();
        Outcome::Changed
    }

    /// The input method's composition, in progress.
    pub fn compose(&mut self, text: &str, cursor: Option<(usize, usize)>, ts: &mut TextSystem) {
        let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
        if text.is_empty() {
            d.clear_compose();
        } else {
            d.set_compose(text, cursor);
        }
    }

    pub fn is_composing(&self) -> bool {
        self.ed.is_composing()
    }

    /// Point gestures, in device pixels relative to the text's origin.
    pub fn press(&mut self, x: f32, y: f32, clicks: u32, shift: bool, ts: &mut TextSystem) {
        let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
        match clicks {
            2 => d.select_word_at_point(x, y),
            n if n >= 3 => d.select_hard_line_at_point(x, y),
            _ if shift => d.shift_click_extension(x, y),
            _ => d.move_to_point(x, y),
        }
        self.dragging = clicks <= 1;
    }

    pub fn drag(&mut self, x: f32, y: f32, ts: &mut TextSystem) {
        if self.dragging {
            let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
            d.extend_selection_to_point(x, y);
        }
    }

    pub fn layout<'a>(&'a mut self, ts: &mut TextSystem) -> &'a parley::Layout<Color> {
        self.ed.refresh_layout(&mut ts.font_cx, &mut ts.layout_cx);
        self.ed.try_layout().expect("refreshed")
    }

    /// Keep the caret of a single-line field in view: adjust `scroll_x`
    /// for a visible width (device pixels).
    pub fn follow_caret(&mut self, visible: f32) {
        if self.multiline {
            self.scroll_x = 0.0;
            return;
        }
        if let Some(c) = self.ed.cursor_geometry(1.0) {
            let x = c.x0 as f32;
            if x - self.scroll_x > visible - 2.0 {
                self.scroll_x = x - visible + 2.0;
            } else if x - self.scroll_x < 0.0 {
                self.scroll_x = x;
            }
            self.scroll_x = self.scroll_x.max(0.0);
        }
    }
}

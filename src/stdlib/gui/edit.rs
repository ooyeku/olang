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

/// A spell checker: the misspelled words of a text, as character ranges.
pub type SpellChecker = fn(&str) -> Vec<(usize, usize)>;

static SPELL: std::sync::Mutex<Option<SpellChecker>> = std::sync::Mutex::new(None);

/// Set the process's spell checker (the platform's, as its first window
/// opens; a test's own).
pub fn set_spell_checker(f: Option<SpellChecker>) {
    *SPELL.lock().unwrap_or_else(|e| e.into_inner()) = f;
}

/// The longest text checked (characters): past it a field is not
/// checked (a log pasted into a note).
pub const SPELL_MAX_CHARS: usize = 100_000;

/// The misspelled words of `text`, or `None` with no checker.
pub fn spell_check(text: &str) -> Option<Vec<(usize, usize)>> {
    if text.len() > SPELL_MAX_CHARS * 4 || text.chars().count() > SPELL_MAX_CHARS {
        return Some(Vec::new());
    }
    if let Some(words) = &*STAND_IN.lock().unwrap_or_else(|e| e.into_inner()) {
        return Some(stand_in_check(words, text));
    }
    let f = (*SPELL.lock().unwrap_or_else(|e| e.into_inner()))?;
    Some(f(text))
}

/// What the platform's speller does besides checking: a misspelled
/// word's guesses (`"guesses"`, the answer), and `"learn"`, `"unlearn"`,
/// `"ignore"` (in this process's document) and `"learned"` a word.
pub struct Speller {
    pub guesses: fn(&str) -> Vec<String>,
    pub teach: fn(&str, &str) -> bool,
}

static SPELLER: std::sync::Mutex<Option<Speller>> = std::sync::Mutex::new(None);

/// Set the process's speller (the platform's; on macOS NSSpellChecker,
/// which needs no window).
pub fn set_speller(s: Option<Speller>) {
    *SPELLER.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// A stand-in speller for a test: the words it calls misspelled, each
/// with its guesses. While set, it checks and guesses instead of the
/// platform's, and learning or ignoring a word drops it (the platform's
/// dictionary is never written).
static STAND_IN: std::sync::Mutex<Option<std::collections::HashMap<String, Vec<String>>>> =
    std::sync::Mutex::new(None);

/// What learning or ignoring a word changed: a field checked before is
/// checked again (`Spelling` keeps the generation it found its words in).
static SPELL_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn spell_generation() -> u64 {
    SPELL_GEN.load(std::sync::atomic::Ordering::Relaxed)
}

fn spell_changed() {
    SPELL_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn set_stand_in(words: Option<std::collections::HashMap<String, Vec<String>>>) {
    *STAND_IN.lock().unwrap_or_else(|e| e.into_inner()) = words;
    spell_changed();
}

/// The stand-in's misspellings of `text`: its words found whole (letters
/// and digits around them are not).
fn stand_in_check(
    words: &std::collections::HashMap<String, Vec<String>>,
    text: &str,
) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '\'') {
            i += 1;
        }
        let w: String = chars[start..i].iter().collect();
        if words.contains_key(&w) {
            out.push((start, i));
        }
    }
    out
}

/// A misspelled word's guesses, best first (none without a speller).
pub fn spell_guesses(word: &str) -> Vec<String> {
    if let Some(words) = &*STAND_IN.lock().unwrap_or_else(|e| e.into_inner()) {
        return words.get(word).cloned().unwrap_or_default();
    }
    let f = SPELLER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.guesses);
    f.map(|f| f(word)).unwrap_or_default()
}

/// Teach the speller: `"learn"` (the person's dictionary: every program
/// spells it right from now on), `"unlearn"`, `"ignore"` (this process's
/// fields, until it ends), `"learned"` (asks). The fields are checked
/// again. `false` without a speller or for an op it does not know.
pub fn spell_teach(op: &str, word: &str) -> bool {
    if let Some(words) = STAND_IN.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let r = match op {
            "learn" | "ignore" => {
                words.remove(word);
                true
            }
            "unlearn" => true,
            "learned" => false,
            _ => return false,
        };
        if op != "learned" {
            spell_changed();
        }
        return r;
    }
    let f = SPELLER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.teach);
    let Some(f) = f else { return false };
    let r = f(op, word);
    if op != "learned" {
        spell_changed();
    }
    r
}

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
    /// A double or triple click's word or line (bytes, low and high): a
    /// drag after it extends by words or lines.
    drag_unit: Option<(u8, usize, usize)>,
    /// The program's last selection applied (`select`'s seq).
    pub select_seq: i64,
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
            drag_unit: None,
            select_seq: 0,
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

    /// Select characters `a` to `f` (the program's selection: a rename's
    /// name before its extension), clamped to the value.
    pub fn select_chars(&mut self, a: usize, f: usize, ts: &mut TextSystem) {
        let text = self.ed.raw_text().to_string();
        let byte = |c: usize| {
            text.char_indices()
                .nth(c)
                .map(|(i, _)| i)
                .unwrap_or(text.len())
        };
        let (ba, bf) = (byte(a), byte(f));
        let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
        d.select_byte_range(ba, bf);
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
        self.dragging = true;
        let sel = self.ed.raw_selection();
        let (a, f) = (sel.anchor().index(), sel.focus().index());
        self.drag_unit = (clicks >= 2).then(|| (clicks.min(3) as u8, a.min(f), a.max(f)));
    }

    pub fn drag(&mut self, x: f32, y: f32, ts: &mut TextSystem) {
        if !self.dragging {
            return;
        }
        if let Some((unit, lo, hi)) = self.drag_unit {
            // the clicked word or line and every one up to the pointer's
            let layout = self.layout(ts);
            let (a, b) = if unit >= 3 {
                let at = parley::Cursor::from_point(layout, x, y).index();
                let t = self.ed.raw_text();
                let at = at.min(t.len());
                let a = t[..at].rfind('\n').map_or(0, |i| i + 1);
                let b = t[at..].find('\n').map_or(t.len(), |i| at + i);
                (a, b)
            } else {
                let r = parley::Selection::word_from_point(layout, x, y).text_range();
                (r.start, r.end)
            };
            let (anchor, focus) = if a < lo { (hi, a) } else { (lo, b.max(hi)) };
            let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
            d.select_byte_range(anchor, focus);
            return;
        }
        let mut d = self.ed.driver(&mut ts.font_cx, &mut ts.layout_cx);
        d.extend_selection_to_point(x, y);
    }

    pub fn layout<'a>(&'a mut self, ts: &mut TextSystem) -> &'a parley::Layout<Color> {
        self.ed.refresh_layout(&mut ts.font_cx, &mut ts.layout_cx);
        self.ed.try_layout().expect("refreshed")
    }

    fn byte_of_char(&self, chars: usize) -> usize {
        let t = self.ed.raw_text();
        t.char_indices()
            .nth(chars)
            .map(|(b, _)| b)
            .unwrap_or(t.len())
    }

    fn char_of_byte(&self, b: usize) -> usize {
        let t = self.ed.raw_text();
        let mut b = b.min(t.len());
        while b > 0 && !t.is_char_boundary(b) {
            b -= 1;
        }
        t[..b].chars().count()
    }

    /// The box of characters `a..b` (its first line's; a caret's when
    /// empty), device pixels from the text's top-left (no scroll).
    pub fn char_box(&mut self, a: usize, b: usize, ts: &mut TextSystem) -> parley::BoundingBox {
        let (ba, bb) = (self.byte_of_char(a), self.byte_of_char(b.max(a)));
        let layout = self.layout(ts);
        let ca = parley::Cursor::from_byte_index(layout, ba, parley::Affinity::Downstream);
        if bb > ba {
            let cb = parley::Cursor::from_byte_index(layout, bb, parley::Affinity::Upstream);
            let mut first = None;
            parley::Selection::new(ca, cb).geometry_with(layout, |bx, _| {
                if first.is_none() {
                    first = Some(bx);
                }
            });
            if let Some(bx) = first {
                return bx;
            }
        }
        ca.geometry(layout, 1.0)
    }

    /// The character under a point, device pixels from the text's
    /// top-left (scroll included).
    pub fn char_at_point(&mut self, x: f32, y: f32, ts: &mut TextSystem) -> usize {
        let layout = self.layout(ts);
        let at = parley::Cursor::from_point(layout, x, y).index();
        self.char_of_byte(at)
    }

    /// The word under a point (characters), as a double click takes it.
    pub fn word_at_point(&mut self, x: f32, y: f32, ts: &mut TextSystem) -> (usize, usize) {
        let layout = self.layout(ts);
        let r = parley::Selection::word_from_point(layout, x, y).text_range();
        (self.char_of_byte(r.start), self.char_of_byte(r.end))
    }

    /// The text's visual lines (see [`FieldLine`]).
    pub fn lines(&mut self, ts: &mut TextSystem) -> Vec<super::text::FieldLine> {
        let text = self.ed.raw_text().to_string();
        let layout = self.layout(ts);
        super::text::layout_lines(layout, &text)
            .into_iter()
            .map(|(start, _end, top, height, chars)| {
                let para = text[..start].matches('\n').count();
                let pstart = text[..start].rfind('\n').map_or(0, |i| i + 1);
                super::text::FieldLine {
                    para,
                    start: text[pstart..start].chars().count(),
                    top,
                    height,
                    xs: chars.iter().map(|c| c.1).collect(),
                    widths: chars.iter().map(|c| c.2).collect(),
                }
            })
            .collect()
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
            // A right-to-left line's trailing spaces hang left of its
            // start, so the caret can sit before x = 0: scroll as far left
            // as the caret, and no further.
            self.scroll_x = self.scroll_x.max(x.min(0.0));
        }
    }
}

/// A field's editor: a plain one (parley's, the whole text one layout)
/// or a styled one (`rich.rs`, a layout a paragraph).
#[allow(clippy::large_enum_variant)] // one a field, never moved hot
pub enum Field {
    Plain(Editor),
    Rich(Box<super::rich::RichEditor>),
}

impl Field {
    pub fn rich(&self) -> Option<&super::rich::RichEditor> {
        match self {
            Field::Rich(r) => Some(r),
            Field::Plain(_) => None,
        }
    }

    pub fn rich_mut(&mut self) -> Option<&mut super::rich::RichEditor> {
        match self {
            Field::Rich(r) => Some(r),
            Field::Plain(_) => None,
        }
    }

    pub fn plain_mut(&mut self) -> Option<&mut Editor> {
        match self {
            Field::Plain(e) => Some(e),
            Field::Rich(_) => None,
        }
    }

    pub fn multiline(&self) -> bool {
        match self {
            Field::Plain(e) => e.multiline,
            Field::Rich(_) => true,
        }
    }

    pub fn secure(&self) -> bool {
        match self {
            Field::Plain(e) => e.secure,
            Field::Rich(r) => r.secure,
        }
    }

    pub fn set_flags(&mut self, multiline: bool, secure: bool) {
        match self {
            Field::Plain(e) => {
                e.multiline = multiline;
                e.secure = secure;
            }
            Field::Rich(r) => r.secure = secure,
        }
    }

    pub fn rev(&self) -> i64 {
        match self {
            Field::Plain(e) => e.rev,
            Field::Rich(r) => r.rev,
        }
    }

    pub fn set_font(&mut self, font: &Font, color: Color) {
        match self {
            Field::Plain(e) => e.set_font(font, color),
            Field::Rich(r) => r.set_font(font, color),
        }
    }

    pub fn value(&self) -> String {
        match self {
            Field::Plain(e) => e.value(),
            Field::Rich(r) => r.value(),
        }
    }

    pub fn selection_chars(&self) -> (usize, usize) {
        match self {
            Field::Plain(e) => e.selection_chars(),
            Field::Rich(r) => r.selection_chars(),
        }
    }

    pub fn offer(
        &mut self,
        value: &std::sync::Arc<String>,
        rev: Option<i64>,
        ts: &mut TextSystem,
    ) -> bool {
        match self {
            Field::Plain(e) => e.offer(value, rev, ts),
            Field::Rich(r) => r.offer(value, rev, ts),
        }
    }

    pub fn is_composing(&self) -> bool {
        match self {
            Field::Plain(e) => e.is_composing(),
            Field::Rich(r) => r.is_composing(),
        }
    }

    pub fn compose(&mut self, text: &str, cursor: Option<(usize, usize)>, ts: &mut TextSystem) {
        match self {
            Field::Plain(e) => e.compose(text, cursor, ts),
            Field::Rich(r) => r.compose(text, cursor, ts),
        }
    }

    pub fn insert(&mut self, text: &str, ts: &mut TextSystem) -> Outcome {
        match self {
            Field::Plain(e) => e.insert(text, ts),
            Field::Rich(r) => r.insert(text, ts),
        }
    }

    pub fn key(
        &mut self,
        key: &str,
        shift: bool,
        word: bool,
        line: bool,
        ts: &mut TextSystem,
    ) -> Outcome {
        match self {
            Field::Plain(e) => e.key(key, shift, word, line, ts),
            Field::Rich(r) => r.key(key, shift, word, line, ts),
        }
    }

    pub fn select_all(&mut self, ts: &mut TextSystem) {
        match self {
            Field::Plain(e) => e.select_all(ts),
            Field::Rich(r) => r.select_all(ts),
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        match self {
            Field::Plain(e) => e.selected_text(),
            Field::Rich(r) => r.selected_text(),
        }
    }

    pub fn delete_selection(&mut self, ts: &mut TextSystem) -> Outcome {
        match self {
            Field::Plain(e) => e.delete_selection(ts),
            Field::Rich(r) => r.delete_selection(ts),
        }
    }

    /// The field keeps its own undo (else ⌘Z is the program's).
    pub fn own_undo(&self) -> bool {
        match self {
            Field::Plain(_) => true,
            Field::Rich(r) => r.own_undo,
        }
    }

    pub fn undo(&mut self, ts: &mut TextSystem) -> Outcome {
        match self {
            Field::Plain(e) => e.undo(ts),
            Field::Rich(r) => r.undo(ts),
        }
    }

    pub fn redo(&mut self, ts: &mut TextSystem) -> Outcome {
        match self {
            Field::Plain(e) => e.redo(ts),
            Field::Rich(r) => r.redo(ts),
        }
    }

    pub fn press(&mut self, x: f32, y: f32, clicks: u32, shift: bool, ts: &mut TextSystem) {
        match self {
            Field::Plain(e) => e.press(x, y, clicks, shift, ts),
            Field::Rich(r) => r.press(x, y, clicks, shift, ts),
        }
    }

    pub fn drag(&mut self, x: f32, y: f32, ts: &mut TextSystem) {
        match self {
            Field::Plain(e) => e.drag(x, y, ts),
            Field::Rich(r) => r.drag(x, y, ts),
        }
    }

    pub fn dragging(&self) -> bool {
        match self {
            Field::Plain(e) => e.dragging,
            Field::Rich(r) => r.dragging,
        }
    }

    pub fn stop_drag(&mut self) {
        match self {
            Field::Plain(e) => e.dragging = false,
            Field::Rich(r) => r.dragging = false,
        }
    }

    /// A right press (a context menu's) at `(x, y)`: the caret goes there
    /// unless the press is inside the selection, which is kept — what a
    /// Mac's text views do, so the menu's Copy and Go to Definition are
    /// about what was pressed.
    pub fn context_press(&mut self, x: f32, y: f32, ts: &mut TextSystem) {
        let (a, f) = self.selection_chars();
        self.press(x, y, 1, false, ts);
        self.stop_drag();
        let (c, _) = self.selection_chars();
        let (lo, hi) = (a.min(f), a.max(f));
        if lo != hi && c >= lo && c <= hi {
            match self {
                Field::Plain(e) => e.select_chars(a, f, ts),
                Field::Rich(r) => r.select_chars(a, f),
            }
        }
    }

    /// The box of characters `a..b`, device pixels from the text's
    /// top-left (no scroll).
    pub fn char_box(&mut self, a: usize, b: usize, ts: &mut TextSystem) -> parley::BoundingBox {
        match self {
            Field::Plain(e) => e.char_box(a, b, ts),
            Field::Rich(r) => r.char_box(a, b, ts),
        }
    }

    /// The character under a point (device pixels from the text's
    /// top-left, scroll included).
    pub fn char_at_point(&mut self, x: f32, y: f32, ts: &mut TextSystem) -> usize {
        match self {
            Field::Plain(e) => e.char_at_point(x, y, ts),
            Field::Rich(r) => r.char_at_point(x, y, ts),
        }
    }

    /// The word under a point, in characters.
    pub fn word_at_point(&mut self, x: f32, y: f32, ts: &mut TextSystem) -> (usize, usize) {
        match self {
            Field::Plain(e) => e.word_at_point(x, y, ts),
            Field::Rich(r) => r.word_at_point(x, y, ts),
        }
    }

    /// The visual lines of the text in a band `y0..y1` (device pixels
    /// from the text's top, no scroll); a plain field's all of them.
    pub fn lines(&mut self, y0: f32, y1: f32, ts: &mut TextSystem) -> Vec<super::text::FieldLine> {
        match self {
            Field::Plain(e) => e.lines(ts),
            Field::Rich(r) => r.lines(y0, y1, ts),
        }
    }

    /// How far the text is scrolled (device pixels): a press lands there.
    pub fn scrolled(&self) -> (f32, f32) {
        match self {
            Field::Plain(e) => (e.scroll_x, 0.0),
            Field::Rich(r) => (r.scroll_x, r.scroll_y),
        }
    }
}

//! A styled, editable text (`textarea` with `rich: true`): the text as
//! paragraphs (hard lines), each laid out on its own, with styled runs
//! (`spans`) the program computes — a markdown editor's headings,
//! emphasis, code — and the caret, the selection, the input method's
//! composition, the pointer's gestures, and its own vertical scroll.
//!
//! Why paragraphs: a plain field lays its whole text out again on every
//! edit (parley's editor), which is 36 ms for 2,000 lines. Here an edit
//! lays out only the paragraphs it touched; a width change re-breaks the
//! lines it already shaped; a frame draws only the paragraphs in view.
//!
//! Offsets the program sees are characters, over the whole text; inside
//! they are bytes. The program stays the authority on the value (as for
//! a plain field, `edit.rs`): an edit is reported with its revision, the
//! selection, the caret's place, and what changed by line, and a value a
//! patch brings is adopted unless it is the field's own earlier one.

use super::edit::Outcome;
use super::text::{Color, Font, TextSystem};
use crate::ast::Value;
use parley::{
    Affinity, Alignment, AlignmentOptions, BoundingBox, Cursor, FontFamily, FontStyle, FontWeight,
    Layout, Selection, StyleProperty,
};
use std::collections::VecDeque;
use std::sync::Arc;

/// How a span looks. Unset fields keep the field's own font and colour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpanStyle {
    pub weight: Option<f32>,
    pub italic: Option<bool>,
    pub mono: bool,
    pub size: Option<f32>,
    pub color: Option<Color>,
    pub underline: bool,
    pub strike: bool,
    /// Drawn behind the span's glyphs (a code span).
    pub bg: Option<Color>,
    /// Drawn behind the whole line, edge to edge (a code block).
    pub line_bg: Option<Color>,
}

impl SpanStyle {
    pub fn parse(v: &Value) -> SpanStyle {
        use super::values::{get, num, parse_color};
        let b = |k: &str| matches!(get(v, k), Some(Value::Boolean(true)));
        SpanStyle {
            weight: get(v, "weight").and_then(num),
            italic: match get(v, "italic") {
                Some(Value::Boolean(i)) => Some(*i),
                _ => None,
            },
            mono: b("mono")
                || matches!(get(v, "font"), Some(Value::String(f)) if f.as_str() == "mono"),
            size: get(v, "size").and_then(num),
            color: get(v, "color").and_then(parse_color),
            underline: b("underline"),
            strike: b("strike"),
            bg: get(v, "bg").and_then(parse_color),
            line_bg: get(v, "line_bg").and_then(parse_color),
        }
    }

    pub fn props(&self, scale_size: f32) -> Vec<StyleProperty<'static, Color>> {
        let mut out = Vec::new();
        if let Some(w) = self.weight {
            out.push(StyleProperty::FontWeight(FontWeight::new(w)));
        }
        if let Some(i) = self.italic {
            out.push(StyleProperty::FontStyle(if i {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            }));
        }
        if self.mono {
            let mono = Font {
                family: "mono".to_string(),
                ..Font::default()
            };
            out.push(StyleProperty::FontFamily(FontFamily::Source(
                mono.family_source().into(),
            )));
        }
        if let Some(s) = self.size {
            out.push(StyleProperty::FontSize(s * scale_size));
        }
        if let Some(c) = self.color {
            out.push(StyleProperty::Brush(c));
        }
        out
    }
}

/// A styled run inside one paragraph: bytes, and the style's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: u32,
}

struct Para {
    /// Where it starts in the buffer, bytes.
    start: usize,
    /// Its length in bytes, without the line break.
    len: usize,
    spans: Vec<Span>,
    /// The program's span list this paragraph was last given (its
    /// address): the same list again is not read again.
    src: usize,
    layout: Option<Layout<Color>>,
    /// Laid out at a width the paragraph has since lost: re-broken, not
    /// shaped again.
    rebreak: bool,
    /// Device pixels: measured when `measured`, else an estimate.
    h: f32,
    /// `h` is the laid-out height at the current width (it outlives the
    /// layout when a far paragraph's layout is let go).
    measured: bool,
}

impl Para {
    fn new(start: usize, len: usize) -> Para {
        Para {
            start,
            len,
            spans: Vec::new(),
            src: 0,
            layout: None,
            rebreak: false,
            h: 0.0,
            measured: false,
        }
    }
    fn end(&self) -> usize {
        self.start + self.len
    }
}

/// What an edit changed, by line, against the value last reported:
/// `first` the first line that differs, `removed` how many lines there
/// were, `lines` the lines now in their place; and by character: at
/// `at`, `old_len` characters became `inserted`.
#[derive(Clone, Debug, Default)]
pub struct Delta {
    pub first: usize,
    pub removed: usize,
    pub lines: Vec<String>,
    pub at: usize,
    pub old_len: usize,
    pub inserted: String,
}

pub struct RichEditor {
    buffer: String,
    paras: Vec<Para>,
    /// The top of each paragraph, device pixels (valid when `ys_ok`).
    ys: Vec<f32>,
    ys_ok: bool,
    styles: Vec<SpanStyle>,
    anchor: usize,
    focus: usize,
    affinity: Affinity,
    /// Where up and down aim, device pixels across.
    h_pos: Option<f32>,
    compose: Option<std::ops::Range<usize>>,
    show_cursor: bool,
    font: Font,
    color: Color,
    scale: f32,
    /// The wrap width, device pixels.
    width: f32,
    pub rev: i64,
    /// The values reported, by revision: known again by their identity
    /// (the program hands back the very string it was given), never
    /// copied or compared.
    sent: VecDeque<(i64, std::sync::Weak<String>)>,
    pub multiline: bool,
    pub secure: bool,
    /// The field's own undo (`undo: false` hands ⌘Z to the program).
    pub own_undo: bool,
    undo: Vec<(String, usize, usize)>,
    redo: Vec<(String, usize, usize)>,
    /// Vertical scroll, device pixels.
    pub scroll_y: f32,
    /// The caret should be brought into view at the next frame.
    pub follow: bool,
    /// The visible height last painted, device pixels.
    pub view_h: f32,
    pub dragging: bool,
    /// The value last reported to the program (for the next delta).
    reported: Arc<String>,
    /// The value as one shared string, made once an edit (`edits`).
    value_cache: Option<(u64, Arc<String>)>,
    /// The one edit since the last report, said without comparing the
    /// texts: (first paragraph, paragraphs removed, paragraphs now, byte
    /// at, the text replaced, the text put in).
    last_edit: Option<(usize, usize, usize, usize, String, String)>,
    edits_since_report: u32,
    /// The program's `select` sequence last applied.
    pub select_seq: i64,
    /// The selection last reported (chars), for `select` events.
    pub said_selection: (usize, usize),
    pub pending_delta: Option<Delta>,
    /// Edits made (the buffer replaced): `mutate` compares counts, not
    /// copies of the buffer.
    edits: u64,
    /// Average advance of a character, device pixels, from paragraphs laid
    /// out on one line (for the estimate of one that is not laid out).
    char_w: (f32, f32),
    /// Paragraphs holding a layout now.
    built: usize,
    /// A code editor's gutter: line numbers and a mark column, drawn in
    /// the field's left padding.
    pub gutter: Option<Gutter>,
    /// Ranges drawn over the text without restyling it (a matching
    /// bracket, the find's matches, a problem's underline): `(line,
    /// start, end, style)`, characters within the line.
    pub decorations: Vec<(usize, usize, usize, u32)>,
    /// The program's `scroll_to` sequence last applied.
    pub scroll_seq: i64,
    /// The first line in view last said (`viewport`), by its bucket.
    pub said_top: Option<usize>,
    /// Lenses: bands of drawn content under a line (a result, a value),
    /// sorted by line. The paragraph layout leaves room for each; the
    /// caret and the selection pass over them, and the gutter numbers no
    /// band.
    lenses: Vec<Lens>,
    /// The character the pointer rests on last said (`hover` events).
    pub said_hover: Option<(usize, usize)>,
    /// More carets than the one the field moves (a program's further
    /// selections, ⌘D's): `(line, column)`, characters, drawn as the
    /// caret is; editing them is the program's.
    pub carets: Vec<(usize, usize)>,
    /// Text drawn after a line's end, quietly, cut to the room left (a
    /// problem said at its line): `(line, text, colour)`.
    pub eol: Vec<(usize, String, Option<Color>)>,
    /// The scopes whose first line is pinned at the top while the view is
    /// inside them (sticky scroll).
    pub sticky: Option<Sticky>,
    /// The pinned rows last drawn: `(top, bottom, line)`, device pixels
    /// from the field's content top (not scrolled), for a press.
    pub sticky_drawn: Vec<(f32, f32, usize)>,
    /// Vertical guides at these columns, and their colour.
    pub rulers: Vec<usize>,
    pub ruler_color: Option<Color>,
    /// Lines are not wrapped: the text scrolls sideways.
    pub nowrap: bool,
    /// Read, selected and copied, never edited (a preview).
    pub readonly: bool,
    /// Horizontal scroll, device pixels (with `nowrap`).
    pub scroll_x: f32,
    /// The widest paragraph laid out, device pixels (with `nowrap`).
    widest: f32,
}

/// Sticky scroll: `scopes` as `(first line, last line)`, sorted by their
/// first line (an outer scope before the scopes inside it); at most
/// `max` rows pinned; their background and the line under them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sticky {
    pub scopes: Vec<(usize, usize)>,
    pub max: usize,
    pub bg: Option<Color>,
    pub line: Option<Color>,
}

impl Sticky {
    pub fn parse(v: Option<&Value>) -> Option<Sticky> {
        use super::values::{get, num, parse_color};
        let v = v?;
        if !matches!(v, Value::Map(_) | Value::Struct { .. }) {
            return None;
        }
        let mut scopes: Vec<(usize, usize)> = match get(v, "scopes") {
            Some(Value::List(l)) => l
                .iter()
                .filter_map(|x| {
                    let Value::Tuple(t) = x else { return None };
                    let a = t.first().and_then(num)?;
                    let b = t.get(1).and_then(num)?;
                    (a >= 0.0 && b > a).then_some((a as usize, b as usize))
                })
                .collect(),
            _ => Vec::new(),
        };
        scopes.sort_by(|x, y| x.0.cmp(&y.0).then(y.1.cmp(&x.1)));
        Some(Sticky {
            scopes,
            max: get(v, "max").and_then(num).map(|n| n.max(0.0) as usize).unwrap_or(3),
            bg: get(v, "bg").and_then(parse_color),
            line: get(v, "line").and_then(parse_color),
        })
    }
}

/// The further carets of a patch: `(line, column)` tuples.
pub fn parse_carets(v: Option<&Value>) -> Vec<(usize, usize)> {
    use super::values::num;
    let Some(Value::List(l)) = v else {
        return Vec::new();
    };
    l.iter()
        .filter_map(|x| {
            let Value::Tuple(t) = x else { return None };
            let a = t.first().and_then(num)?;
            let b = t.get(1).and_then(num)?;
            (a >= 0.0 && b >= 0.0).then_some((a as usize, b as usize))
        })
        .collect()
}

/// The end-of-line texts of a patch: `(line, text, colour)` tuples.
pub fn parse_eol(v: Option<&Value>) -> Vec<(usize, String, Option<Color>)> {
    use super::values::{num, parse_color};
    let Some(Value::List(l)) = v else {
        return Vec::new();
    };
    let mut out: Vec<(usize, String, Option<Color>)> = l
        .iter()
        .filter_map(|x| {
            let Value::Tuple(t) = x else { return None };
            let line = t.first().and_then(num)?;
            let Some(Value::String(text)) = t.get(1) else { return None };
            (line >= 0.0 && !text.is_empty()).then(|| (line as usize, text.to_string(), t.get(2).and_then(parse_color)))
        })
        .collect();
    out.sort_by_key(|e| e.0);
    out.dedup_by_key(|e| e.0);
    out
}

/// The rulers of a patch: `#{ cols, color }`.
pub fn parse_rulers(v: Option<&Value>) -> (Vec<usize>, Option<Color>) {
    use super::values::{get, num, parse_color};
    let Some(v) = v else { return (Vec::new(), None) };
    let cols = match get(v, "cols") {
        Some(Value::List(l)) => l.iter().filter_map(num).filter(|n| *n > 0.0).map(|n| n as usize).collect(),
        _ => Vec::new(),
    };
    (cols, get(v, "color").and_then(parse_color))
}

/// A lens: `height` logical pixels under paragraph `line`, drawn from
/// canvas operations (`ops`, canvas.rs) across the text's width. A press
/// in it is said to the program (`lens` events) with the operation's
/// `hit` under it; it never moves the caret.
#[derive(Clone, Debug, PartialEq)]
pub struct Lens {
    pub line: usize,
    pub height: f32,
    pub ops: Value,
    pub id: Value,
}

/// The lenses of a patch: `#{ line, height, ops, id }` maps, sorted by
/// line (one lens a line: the last of a line's wins).
pub fn parse_lenses(v: Option<&Value>) -> Vec<Lens> {
    use super::values::{get, num};
    let Some(Value::List(l)) = v else {
        return Vec::new();
    };
    let mut out: Vec<Lens> = l
        .iter()
        .filter_map(|x| {
            let line = get(x, "line").and_then(num)?;
            let height = get(x, "height").and_then(num)?;
            if line < 0.0 || height <= 0.0 {
                return None;
            }
            Some(Lens {
                line: line as usize,
                height,
                ops: get(x, "ops").cloned().unwrap_or(Value::List(Arc::new(Vec::new()))),
                id: get(x, "id").cloned().unwrap_or(Value::Unit),
            })
        })
        .collect();
    out.sort_by_key(|l| l.line);
    out.dedup_by(|b, a| {
        if a.line == b.line {
            *a = b.clone();
            true
        } else {
            false
        }
    });
    out
}

/// A gutter: whether to number the lines, their colour and the caret
/// line's, a background for the caret's line, and the marks — `(line,
/// glyph, colour)` — in a column at the gutter's left edge. A mark whose
/// glyph is a list of canvas operations is drawn from them instead, in a
/// box of `MARK_BOX` logical pixels centred on its line's first row
/// (`drawn`): a shape, not a character of whatever font has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gutter {
    pub numbers: bool,
    pub color: Option<Color>,
    pub current: Option<Color>,
    pub line_bg: Option<Color>,
    pub marks: Vec<(usize, String, Option<Color>)>,
    pub drawn: Vec<(usize, Value)>,
}

/// The side of a drawn gutter mark's box, logical pixels.
pub const MARK_BOX: f32 = 12.0;

impl Gutter {
    pub fn parse(v: &Value) -> Option<Gutter> {
        use super::values::{get, parse_color};
        if !matches!(v, Value::Map(_) | Value::Struct { .. }) {
            return None;
        }
        let mut drawn = Vec::new();
        let marks = match get(v, "marks") {
            Some(Value::List(l)) => l
                .iter()
                .filter_map(|m| {
                    let Value::Tuple(t) = m else { return None };
                    let line = match t.first()? {
                        Value::Integer(i) if *i >= 0 => *i as usize,
                        _ => return None,
                    };
                    let glyph = match t.get(1) {
                        Some(Value::String(g)) => g.to_string(),
                        Some(ops @ Value::List(_)) => {
                            drawn.push((line, ops.clone()));
                            return None;
                        }
                        _ => "●".to_string(),
                    };
                    Some((line, glyph, t.get(2).and_then(parse_color)))
                })
                .collect(),
            _ => Vec::new(),
        };
        Some(Gutter {
            numbers: !matches!(get(v, "numbers"), Some(Value::Boolean(false))),
            color: get(v, "color").and_then(parse_color),
            current: get(v, "current").and_then(parse_color),
            line_bg: get(v, "line_bg").and_then(parse_color),
            marks,
            drawn,
        })
    }
}

/// The decorations of a patch: `(line, start, end, style)` tuples.
pub fn parse_decorations(v: Option<&Value>) -> Vec<(usize, usize, usize, u32)> {
    let Some(Value::List(l)) = v else {
        return Vec::new();
    };
    let n = |v: Option<&Value>| match v {
        Some(Value::Integer(i)) => Some((*i).max(0) as usize),
        Some(Value::Float(f)) => Some(f.max(0.0) as usize),
        _ => None,
    };
    l.iter()
        .filter_map(|d| {
            let Value::Tuple(t) = d else { return None };
            Some((n(t.first())?, n(t.get(1))?, n(t.get(2))?, n(t.get(3))? as u32))
        })
        .collect()
}

/// A paragraph's layouts kept at most: past it, those far from the view
/// and the caret are let go (their measured heights stay).
const KEEP_LAYOUTS: usize = 3000;


fn char_at(s: &str, chars: usize) -> usize {
    // an ASCII prefix: a character is a byte (a code file, mostly)
    let probe = chars.min(s.len());
    if s.as_bytes()[..probe].is_ascii() {
        return probe;
    }
    s.char_indices()
        .nth(chars)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

fn chars_of(s: &str, bytes: usize) -> usize {
    let b = bytes.min(s.len());
    let mut b2 = b;
    while b2 > 0 && !s.is_char_boundary(b2) {
        b2 -= 1;
    }
    s[..b2].chars().count()
}

impl RichEditor {
    pub fn new(value: &str, font: &Font, color: Color) -> RichEditor {
        let mut e = RichEditor {
            buffer: String::new(),
            paras: vec![Para::new(0, 0)],
            ys: Vec::new(),
            ys_ok: false,
            styles: Vec::new(),
            anchor: 0,
            focus: 0,
            affinity: Affinity::Downstream,
            h_pos: None,
            compose: None,
            show_cursor: true,
            font: font.clone(),
            color,
            scale: 1.0,
            width: 240.0,
            rev: 0,
            sent: VecDeque::new(),
            multiline: true,
            secure: false,
            own_undo: true,
            undo: Vec::new(),
            redo: Vec::new(),
            scroll_y: 0.0,
            follow: false,
            view_h: 0.0,
            dragging: false,
            reported: Arc::new(value.to_string()),
            value_cache: None,
            last_edit: None,
            edits_since_report: 0,
            select_seq: 0,
            said_selection: (0, 0),
            pending_delta: None,
            edits: 0,
            char_w: (0.0, 0.0),
            built: 0,
            gutter: None,
            decorations: Vec::new(),
            scroll_seq: 0,
            said_top: None,
            lenses: Vec::new(),
            said_hover: None,
            carets: Vec::new(),
            eol: Vec::new(),
            sticky: None,
            sticky_drawn: Vec::new(),
            rulers: Vec::new(),
            ruler_color: None,
            nowrap: false,
            readonly: false,
            scroll_x: 0.0,
            widest: 0.0,
        };
        e.set_buffer(value);
        e
    }

    fn set_buffer(&mut self, value: &str) {
        self.buffer = value.to_string();
        let mut paras = Vec::new();
        let mut at = 0;
        for line in self.buffer.split('\n') {
            paras.push(Para::new(at, line.len()));
            at += line.len() + 1;
        }
        self.paras = paras;
        self.built = 0;
        self.ys_ok = false;
        self.anchor = self.anchor.min(self.buffer.len());
        self.focus = self.focus.min(self.buffer.len());
        self.snap_selection();
    }

    fn snap_selection(&mut self) {
        while !self.buffer.is_char_boundary(self.anchor) {
            self.anchor -= 1;
        }
        while !self.buffer.is_char_boundary(self.focus) {
            self.focus -= 1;
        }
    }

    // ── the program's side ──────────────────────────────────────────

    pub fn set_font(&mut self, font: &Font, color: Color) {
        if self.font == *font && self.color == color {
            return;
        }
        self.font = font.clone();
        self.color = color;
        self.invalidate();
    }

    fn invalidate(&mut self) {
        for p in &mut self.paras {
            p.layout = None;
            p.measured = false;
        }
        self.built = 0;
        self.char_w = (0.0, 0.0);
        self.ys_ok = false;
    }

    pub fn set_geometry(&mut self, width: f32, scale: f32) {
        if self.scale != scale {
            self.scale = scale;
            self.invalidate();
        }
        let w = (width * scale).max(1.0);
        if (self.width - w).abs() > 0.01 {
            self.width = w;
            for p in &mut self.paras {
                p.rebreak = true;
                p.measured = false;
            }
            self.ys_ok = false;
        }
    }

    /// The styles and the spans of a patch (`styles`: a list of style
    /// maps; `spans`: a list a line, each of `(start, end, style)` in
    /// characters). Read only when the patch's value is the field's: a
    /// program behind the field styles text it has not seen.
    pub fn set_spans(&mut self, styles: Option<&Value>, spans: Option<&Value>) {
        if let Some(Value::List(l)) = styles {
            let parsed: Vec<SpanStyle> = l.iter().map(SpanStyle::parse).collect();
            if parsed != self.styles {
                self.styles = parsed;
                self.invalidate();
                for p in &mut self.paras {
                    p.src = 0;
                }
            }
        }
        let Some(Value::List(lines)) = spans else {
            return;
        };
        for (i, p) in self.paras.iter_mut().enumerate() {
            let v = lines.get(i);
            let (addr, list) = match v {
                Some(Value::List(l)) => (Arc::as_ptr(l) as *const () as usize, Some(l)),
                _ => (1, None),
            };
            if addr == p.src {
                continue;
            }
            p.src = addr;
            let text = &self.buffer[p.start..p.end()];
            let mut spans = Vec::new();
            if let Some(l) = list {
                for sp in l.iter() {
                    let Value::Tuple(t) = sp else { continue };
                    if t.len() < 3 {
                        continue;
                    }
                    let n = |v: &Value| match v {
                        Value::Integer(i) => (*i).max(0) as usize,
                        Value::Float(f) => f.max(0.0) as usize,
                        _ => 0,
                    };
                    let (a, b, st) = (n(&t[0]), n(&t[1]), n(&t[2]));
                    // an empty span still says its line's look (a blank
                    // line in a code block)
                    if b < a
                        || st >= self.styles.len()
                        || (b == a && self.styles[st].line_bg.is_none())
                    {
                        continue;
                    }
                    spans.push(Span {
                        start: char_at(text, a),
                        end: char_at(text, b),
                        style: st as u32,
                    });
                }
            }
            if spans != p.spans {
                p.spans = spans;
                if p.layout.take().is_some() {
                    self.built -= 1;
                }
                // a span's look may change the paragraph's height (a
                // heading's size): measured again when next shown
                p.measured = false;
                self.ys_ok = false;
            }
        }
    }

    pub fn value(&self) -> String {
        match &self.compose {
            None => self.buffer.clone(),
            Some(r) => {
                let mut v = String::with_capacity(self.buffer.len());
                v.push_str(&self.buffer[..r.start]);
                v.push_str(&self.buffer[r.end..]);
                v
            }
        }
    }

    /// Whether the value is `v` (without copying the value).
    pub fn value_is(&self, v: &str) -> bool {
        match &self.compose {
            None => self.buffer == v,
            Some(r) => {
                v.len() == self.buffer.len() - (r.end - r.start)
                    && v.as_bytes()[..r.start] == self.buffer.as_bytes()[..r.start]
                    && v.as_bytes()[r.start..] == self.buffer.as_bytes()[r.end..]
            }
        }
    }

    /// The value as a shared string: one copy an edit, however often it
    /// is asked for (the report, the revision kept, the event).
    pub fn value_arc(&mut self) -> Arc<String> {
        if self.compose.is_none()
            && let Some((n, v)) = &self.value_cache
            && *n == self.edits
        {
            return v.clone();
        }
        let v = Arc::new(self.value());
        if self.compose.is_none() {
            self.value_cache = Some((self.edits, v.clone()));
        }
        v
    }

    pub fn is_composing(&self) -> bool {
        self.compose.is_some()
    }

    pub fn selection_chars(&self) -> (usize, usize) {
        (
            chars_of(&self.buffer, self.anchor),
            chars_of(&self.buffer, self.focus),
        )
    }

    /// Select `(anchor, focus)` in characters (the program's `select`).
    pub fn select_chars(&mut self, a: usize, f: usize) {
        self.anchor = char_at(&self.buffer, a);
        self.focus = char_at(&self.buffer, f);
        self.affinity = Affinity::Downstream;
        self.h_pos = None;
        self.follow = true;
    }

    /// The program's value from a patch: adopted unless it is the field's
    /// own earlier one. The selection stays where it was in the text
    /// that did not change.
    pub fn offer(&mut self, value: &Arc<String>, rev: Option<i64>, ts: &mut TextSystem) -> bool {
        if self.is_composing() {
            return false;
        }
        if self.buffer.len() == value.len() && self.buffer.as_str() == value.as_str() {
            return false;
        }
        if let Some(r) = rev
            && r < self.rev
            && self.sent.iter().any(|(sr, sv)| {
                *sr == r && sv.upgrade().is_some_and(|v| Arc::ptr_eq(&v, value))
            })
        {
            return false;
        }
        let value: &str = value.as_str();
        // the change as one replacement: what both share at each end
        self.refresh(ts);
        let (p, s) = common_ends(&self.buffer, value);
        let old_len = self.buffer.len();
        let inserted = value[p..value.len() - s].to_string();
        let (a, f) = (self.anchor, self.focus);
        self.replace(p, old_len - s, &inserted, ts);
        let new_len = self.buffer.len();
        // the selection, mapped through the change
        let map = |x: usize| {
            if x <= p {
                x
            } else if x >= old_len - s {
                x + new_len - old_len
            } else {
                new_len - s
            }
        };
        self.anchor = map(a).min(new_len);
        self.focus = map(f).min(new_len);
        self.snap_selection();
        self.rev += 1;
        self.remember();
        true
    }

    fn remember(&mut self) {
        let v = Arc::downgrade(&self.value_arc());
        self.sent.push_back((self.rev, v));
        while self.sent.len() > 32 {
            self.sent.pop_front();
        }
    }

    /// What changed since the last report, by line and by character, and
    /// the new value becomes the one reported.
    pub fn take_delta(&mut self) -> Delta {
        let now = self.value_arc();
        let d = match (&self.last_edit, self.edits_since_report, &self.compose) {
            // one edit: said from the edit itself, not by comparing texts
            (Some((first, removed, n, at, old, ins)), 1, None) => Delta {
                first: *first,
                removed: *removed,
                lines: (*first..*first + *n)
                    .map(|i| self.para_text(i).to_string())
                    .collect(),
                at: chars_of(&self.buffer, *at),
                old_len: old.chars().count(),
                inserted: ins.clone(),
            },
            _ => delta_of(&self.reported, &now),
        };
        self.reported = now;
        self.edits_since_report = 0;
        self.last_edit = None;
        d
    }

    /// The value as reported (a value adopted from the program is the
    /// program's own: nothing to say).
    pub fn mark_reported(&mut self) {
        self.reported = self.value_arc();
        self.edits_since_report = 0;
        self.last_edit = None;
    }

    // ── paragraphs ──────────────────────────────────────────────────

    /// The paragraph holding byte `at`, and `at` within it.
    fn locate(&self, at: usize) -> (usize, usize) {
        let i = match self.paras.binary_search_by(|p| p.start.cmp(&at)) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let i = i.min(self.paras.len() - 1);
        (
            i,
            at.saturating_sub(self.paras[i].start)
                .min(self.paras[i].len),
        )
    }

    fn build(&mut self, i: usize, ts: &mut TextSystem) {
        if self.paras[i].layout.is_some() && !self.paras[i].rebreak {
            return;
        }
        let wrap = if self.nowrap { None } else { Some(self.width) };
        if let Some(mut l) = self.paras[i].layout.take() {
            l.break_all_lines(wrap);
            if self.nowrap {
                self.widest = self.widest.max(l.width());
            }
            l.align(Alignment::Start, AlignmentOptions::default());
            let h = l.height();
            let p = &mut self.paras[i];
            let h2 = if h > 0.0 { h } else { p.h };
            if !p.measured || (h2 - p.h).abs() > 0.01 {
                self.ys_ok = false;
            }
            p.h = h2;
            p.measured = true;
            p.layout = Some(l);
            p.rebreak = false;
            return;
        }
        self.built += 1;
        let p = &self.paras[i];
        let text = &self.buffer[p.start..p.end()];
        let mut b = ts
            .layout_cx
            .ranged_builder(&mut ts.font_cx, text, self.scale, true);
        for st in self.font.styles() {
            b.push_default(st);
        }
        b.push_default(StyleProperty::Brush(self.color));
        for sp in &p.spans {
            if sp.end > text.len() || sp.start >= sp.end {
                continue;
            }
            if let Some(st) = self.styles.get(sp.style as usize) {
                for prop in st.props(1.0) {
                    b.push(prop, sp.start..sp.end);
                }
            }
        }
        let mut l = b.build(text);
        l.break_all_lines(wrap);
        if self.nowrap {
            self.widest = self.widest.max(l.width());
        }
        l.align(Alignment::Start, AlignmentOptions::default());
        let h = l.height();
        let line_h = self.font.size * self.scale * 1.2;
        // a paragraph on one line says how wide a character is
        if l.len() == 1 && !text.is_empty() && self.char_w.1 < 20_000.0 {
            self.char_w.0 += l.width();
            self.char_w.1 += text.chars().count() as f32;
        }
        let p = &mut self.paras[i];
        let h2 = if h > 0.0 { h } else { line_h };
        if !p.measured || (h2 - p.h).abs() > 0.01 {
            self.ys_ok = false;
        }
        p.h = h2;
        p.measured = true;
        p.layout = Some(l);
        p.rebreak = false;
    }

    /// The height of paragraph `i` as far as it is known: measured, or
    /// estimated from its length, the width and the average character.
    fn height_of(&self, i: usize) -> f32 {
        let p = &self.paras[i];
        if p.measured {
            return p.h;
        }
        let line_h = self.font.size * self.scale * 1.2;
        let cw = if self.char_w.1 > 0.0 {
            self.char_w.0 / self.char_w.1
        } else {
            self.font.size * self.scale * 0.55
        };
        let lines = if self.nowrap { 1.0 } else { ((p.len as f32 * cw) / self.width.max(1.0)).ceil().max(1.0) };
        line_h * lines
    }

    /// Paragraph `i` laid out (its height measured: the tops after it may
    /// move, said by `ys_ok`).
    fn ensure(&mut self, i: usize, ts: &mut TextSystem) {
        if i < self.paras.len() {
            self.build(i, ts);
        }
    }

    /// Let go of the layouts of paragraphs far from `keep` (a band of
    /// paragraph indexes) and the caret, once too many are held.
    fn trim_layouts(&mut self, keep: std::ops::Range<usize>) {
        if self.built <= KEEP_LAYOUTS {
            return;
        }
        let (fi, _) = self.locate(self.focus);
        let lo = keep.start.saturating_sub(200);
        let hi = keep.end + 200;
        let mut n = 0;
        for (i, p) in self.paras.iter_mut().enumerate() {
            if (i < lo || i >= hi) && i.abs_diff(fi) > 2 && p.layout.is_some() {
                p.layout = None;
                p.rebreak = false;
            }
            if p.layout.is_some() {
                n += 1;
            }
        }
        self.built = n;
    }

    /// The paragraphs' tops, from the heights known (a paragraph not laid
    /// out yet is estimated: only those shown, and the caret's, are laid
    /// out, so a 50,000-line text opens and edits at the cost of a page).
    pub fn refresh(&mut self, _ts: &mut TextSystem) {
        if self.ys_ok {
            return;
        }
        self.ys.clear();
        self.ys.reserve(self.paras.len() + 1);
        let mut y = 0.0;
        let mut k = 0;
        for i in 0..self.paras.len() {
            self.ys.push(y);
            y += self.height_of(i);
            while k < self.lenses.len() && self.lenses[k].line < i {
                k += 1;
            }
            if k < self.lenses.len() && self.lenses[k].line == i {
                y += self.lenses[k].height * self.scale;
            }
        }
        self.ys.push(y);
        self.ys_ok = true;
    }

    pub fn content_height(&mut self, ts: &mut TextSystem) -> f32 {
        self.refresh(ts);
        *self.ys.last().unwrap_or(&0.0)
    }

    fn layout_of(&self, i: usize) -> &Layout<Color> {
        self.paras[i].layout.as_ref().expect("a paragraph used is laid out (ensure)")
    }

    fn cursor(&self, i: usize, local: usize, aff: Affinity) -> Cursor {
        let l = self.layout_of(i);
        if local >= self.paras[i].len {
            Cursor::from_byte_index(l, self.paras[i].len, Affinity::Upstream)
        } else {
            Cursor::from_byte_index(l, local, aff)
        }
    }

    /// Replace bytes `a..b` with `s`: the paragraphs it touched are split
    /// again and laid out; those after it move. The selection becomes a
    /// caret after the insertion.
    fn replace(&mut self, a: usize, b: usize, s: &str, ts: &mut TextSystem) {
        let (pa, la) = self.locate(a);
        let (pb, lb) = self.locate(b);
        let first_start = self.paras[pa].start;
        let old_end = self.paras[pb].end();
        let growth = s.len() as isize - (b - a) as isize;
        // spans kept until the program styles the text again: those of
        // the first paragraph before the change, of the last after it
        let mut head: Vec<Span> = self.paras[pa]
            .spans
            .iter()
            .filter_map(|sp| {
                let e = sp.end.min(la);
                (e > sp.start).then_some(Span { end: e, ..*sp })
            })
            .collect();
        let tail: Vec<Span> = self.paras[pb]
            .spans
            .iter()
            .filter_map(|sp| {
                let st = sp.start.max(lb);
                (sp.end > st).then_some(Span {
                    start: st - lb,
                    end: sp.end - lb,
                    style: sp.style,
                })
            })
            .collect();
        // a span the change sits inside grows with what was typed
        let inside: Vec<u32> = self.paras[pa]
            .spans
            .iter()
            .filter(|sp| pa == pb && sp.start < la && sp.end > lb)
            .map(|sp| sp.style)
            .collect();
        let old_text = self.buffer[a..b].to_string();
        self.buffer.replace_range(a..b, s);
        let new_end = (old_end as isize + growth) as usize;
        let region = &self.buffer[first_start..new_end];
        let mut fresh = Vec::new();
        let mut at = first_start;
        for line in region.split('\n') {
            fresh.push(Para::new(at, line.len()));
            at += line.len() + 1;
        }
        let n = fresh.len();
        let ins_last = s.rsplit('\n').next().map(|x| x.len()).unwrap_or(0);
        if n == 1 {
            let mut spans = head.clone();
            for sp in &tail {
                spans.push(Span {
                    start: sp.start + la + s.len(),
                    end: sp.end + la + s.len(),
                    style: sp.style,
                });
            }
            // the span around the change, joined again
            for st in inside {
                let (h, t) = (
                    head.iter().position(|x| x.style == st && x.end == la),
                    spans
                        .iter()
                        .position(|x| x.style == st && x.start == la + s.len()),
                );
                if let (Some(_), Some(ti)) = (h, t) {
                    let end = spans[ti].end;
                    spans.remove(ti);
                    if let Some(hs) = spans.iter_mut().find(|x| x.style == st && x.end == la) {
                        hs.end = end;
                    }
                }
            }
            fresh[0].spans = spans;
        } else {
            fresh[0].spans = std::mem::take(&mut head);
            fresh[n - 1].spans = tail
                .iter()
                .map(|sp| Span {
                    start: sp.start + ins_last,
                    end: sp.end + ins_last,
                    style: sp.style,
                })
                .collect();
        }
        // compose ranges move with the text
        if let Some(c) = &mut self.compose
            && c.start >= b
        {
            *c = ((c.start as isize + growth) as usize)..((c.end as isize + growth) as usize);
        }
        let dropped = self.paras[pa..=pb].iter().filter(|p| p.layout.is_some()).count();
        self.built -= dropped;
        self.paras.splice(pa..=pb, fresh);
        self.edits += 1;
        self.edits_since_report += 1;
        self.last_edit = Some((pa, pb - pa + 1, n, a, old_text, s.to_string()));
        for p in &mut self.paras[pa + n..] {
            p.start = (p.start as isize + growth) as usize;
        }
        for i in pa..pa + n {
            self.build(i, ts);
        }
        self.ys_ok = false;
        let caret = a + s.len();
        self.anchor = caret;
        self.focus = caret;
        self.affinity = if s.ends_with('\n') {
            Affinity::Downstream
        } else {
            Affinity::Upstream
        };
        self.h_pos = None;
        self.follow = true;
    }

    fn sel_range(&self) -> (usize, usize) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }

    fn collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// Run an edit that may change the value: the field's own undo, the
    /// revision when it did.
    fn mutate(
        &mut self,
        ts: &mut TextSystem,
        f: impl FnOnce(&mut Self, &mut TextSystem),
    ) -> Outcome {
        // a field to read (a preview): selected and copied, never edited
        if self.readonly {
            return Outcome::Pass;
        }
        // the buffer is copied only for the field's own undo; a change is
        // known by the edits made, not by comparing copies
        let before = self
            .own_undo
            .then(|| (self.buffer.clone(), self.anchor, self.focus));
        let edits = self.edits;
        f(self, ts);
        let changed = self.edits != edits && before.as_ref().is_none_or(|b| b.0 != self.buffer);
        if !changed {
            return Outcome::Moved;
        }
        if let Some(before) = before {
            self.undo.push(before);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.rev += 1;
        self.remember();
        Outcome::Changed
    }

    pub fn insert(&mut self, text: &str, ts: &mut TextSystem) -> Outcome {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if text.is_empty() {
            return Outcome::Moved;
        }
        self.refresh(ts);
        self.mutate(ts, |e, ts| {
            let (a, b) = e.sel_range();
            e.replace(a, b, &text, ts);
        })
    }

    pub fn delete_selection(&mut self, ts: &mut TextSystem) -> Outcome {
        self.refresh(ts);
        self.mutate(ts, |e, ts| {
            let (a, b) = e.sel_range();
            if a < b {
                e.replace(a, b, "", ts);
            }
        })
    }

    pub fn selected_text(&self) -> Option<String> {
        if self.secure {
            return None;
        }
        let (a, b) = self.sel_range();
        (a < b).then(|| self.buffer[a..b].to_string())
    }

    pub fn select_all(&mut self, _ts: &mut TextSystem) {
        self.anchor = 0;
        self.focus = self.buffer.len();
        self.h_pos = None;
    }

    pub fn undo(&mut self, ts: &mut TextSystem) -> Outcome {
        let Some(snap) = self.undo.pop() else {
            return Outcome::Moved;
        };
        self.redo
            .push((self.buffer.clone(), self.anchor, self.focus));
        self.restore(snap, ts);
        Outcome::Changed
    }

    pub fn redo(&mut self, ts: &mut TextSystem) -> Outcome {
        let Some(snap) = self.redo.pop() else {
            return Outcome::Moved;
        };
        self.undo
            .push((self.buffer.clone(), self.anchor, self.focus));
        self.restore(snap, ts);
        Outcome::Changed
    }

    fn restore(&mut self, snap: (String, usize, usize), ts: &mut TextSystem) {
        self.refresh(ts);
        let (p, s) = common_ends(&self.buffer, &snap.0);
        let ins = snap.0[p..snap.0.len() - s].to_string();
        let end = self.buffer.len() - s;
        self.replace(p, end, &ins, ts);
        self.anchor = snap.1.min(self.buffer.len());
        self.focus = snap.2.min(self.buffer.len());
        self.snap_selection();
        self.rev += 1;
        self.remember();
    }

    // ── keys ────────────────────────────────────────────────────────

    fn set_focus_to(&mut self, at: usize, aff: Affinity, extend: bool) {
        self.focus = at.min(self.buffer.len());
        self.affinity = aff;
        if !extend {
            self.anchor = self.focus;
        }
        self.follow = true;
    }

    fn caret_x(&mut self, ts: &mut TextSystem) -> f32 {
        if let Some(x) = self.h_pos {
            return x;
        }
        self.refresh(ts);
        let (i, l) = self.locate(self.focus);
        self.ensure(i, ts);
        let g = self
            .cursor(i, l, self.affinity)
            .geometry(self.layout_of(i), 1.0);
        g.x0 as f32
    }

    /// The line of paragraph `i` holding local byte `l`: its index, and
    /// the number of lines.
    fn line_of(&self, i: usize, l: usize) -> (usize, usize) {
        let layout = self.layout_of(i);
        let c = self.cursor(i, l, self.affinity);
        let g = c.geometry(layout, 1.0);
        let mid = ((g.y0 + g.y1) / 2.0) as f32;
        let n = layout.len();
        let mut at = 0;
        for (k, line) in layout.lines().enumerate() {
            let m = line.metrics();
            if mid >= m.block_min_coord {
                at = k;
            }
        }
        (at, n.max(1))
    }

    fn line_mid(&self, i: usize, k: usize) -> f32 {
        let layout = self.layout_of(i);
        match layout.get(k) {
            Some(line) => {
                let m = line.metrics();
                (m.block_min_coord + m.block_max_coord) / 2.0
            }
            None => self.paras[i].h / 2.0,
        }
    }

    fn vertical(&mut self, down: bool, extend: bool, ts: &mut TextSystem) {
        self.refresh(ts);
        let x = self.caret_x(ts);
        let (i, l) = self.locate(self.focus);
        self.ensure(i, ts);
        if i > 0 {
            self.ensure(i - 1, ts);
        }
        self.ensure(i + 1, ts);
        let (k, n) = self.line_of(i, l);
        let target = if down {
            if k + 1 < n {
                Some((i, k + 1))
            } else if i + 1 < self.paras.len() {
                Some((i + 1, 0))
            } else {
                None
            }
        } else if k > 0 {
            Some((i, k - 1))
        } else if i > 0 {
            let pl = self.layout_of(i - 1).len().max(1);
            Some((i - 1, pl - 1))
        } else {
            None
        };
        match target {
            Some((ti, tk)) => {
                let y = self.line_mid(ti, tk);
                let c = Cursor::from_point(self.layout_of(ti), x, y);
                let at = self.paras[ti].start + c.index().min(self.paras[ti].len);
                self.set_focus_to(at, c.affinity(), extend);
            }
            None => {
                let at = if down { self.buffer.len() } else { 0 };
                self.set_focus_to(at, Affinity::Downstream, extend);
            }
        }
        self.h_pos = Some(x);
    }

    fn page(&mut self, down: bool, extend: bool, ts: &mut TextSystem) {
        self.refresh(ts);
        let x = self.caret_x(ts);
        let (i, l) = self.locate(self.focus);
        self.ensure(i, ts);
        self.refresh(ts);
        let g = self
            .cursor(i, l, self.affinity)
            .geometry(self.layout_of(i), 1.0);
        let y = self.ys[i] + ((g.y0 + g.y1) / 2.0) as f32;
        let step =
            (self.view_h - self.font.size * self.scale * 1.5).max(self.font.size * self.scale);
        let ty = if down { y + step } else { y - step };
        let at = self.point_to_byte(x, ty, ts);
        self.set_focus_to(at.0, at.1, extend);
        self.scroll_y = (self.scroll_y + if down { step } else { -step }).max(0.0);
        self.h_pos = Some(x);
    }

    /// The byte under a point, device pixels from the text's top-left.
    fn point_to_byte(&mut self, x: f32, y: f32, ts: &mut TextSystem) -> (usize, Affinity) {
        self.refresh(ts);
        let total = *self.ys.last().unwrap_or(&0.0);
        if y < 0.0 {
            return (0, Affinity::Downstream);
        }
        if y >= total {
            let i = self.paras.len() - 1;
            self.ensure(i, ts);
            let c = Cursor::from_point(self.layout_of(i), x, self.paras[i].h - 0.5);
            return (
                self.paras[i].start + c.index().min(self.paras[i].len),
                c.affinity(),
            );
        }
        let i = match self
            .ys
            .binary_search_by(|v| v.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Less))
        {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
        .min(self.paras.len() - 1);
        self.ensure(i, ts);
        let c = Cursor::from_point(self.layout_of(i), x, y - self.ys[i]);
        (
            self.paras[i].start + c.index().min(self.paras[i].len),
            c.affinity(),
        )
    }

    fn horizontal(&mut self, right: bool, word: bool, extend: bool, ts: &mut TextSystem) {
        self.refresh(ts);
        if !extend && !self.collapsed() && !word {
            let (a, b) = self.sel_range();
            self.set_focus_to(if right { b } else { a }, Affinity::Downstream, false);
            self.h_pos = None;
            return;
        }
        let (i, l) = self.locate(self.focus);
        self.ensure(i, ts);
        let len = self.paras[i].len;
        let at = if right && l >= len {
            if i + 1 < self.paras.len() {
                (self.paras[i + 1].start, Affinity::Downstream)
            } else {
                (self.focus, self.affinity)
            }
        } else if !right && l == 0 {
            if i > 0 {
                (self.paras[i - 1].end(), Affinity::Upstream)
            } else {
                (0, Affinity::Downstream)
            }
        } else {
            let layout = self.layout_of(i);
            let c = self.cursor(i, l, self.affinity);
            let n = match (right, word) {
                (true, true) => c.next_visual_word(layout),
                (true, false) => c.next_visual(layout),
                (false, true) => c.previous_visual_word(layout),
                (false, false) => c.previous_visual(layout),
            };
            (self.paras[i].start + n.index().min(len), n.affinity())
        };
        self.set_focus_to(at.0, at.1, extend);
        self.h_pos = None;
    }

    fn line_edge(&mut self, end: bool, extend: bool, ts: &mut TextSystem) {
        self.refresh(ts);
        let (i, l) = self.locate(self.focus);
        self.ensure(i, ts);
        let layout = self.layout_of(i);
        let c = self.cursor(i, l, self.affinity);
        let sel = Selection::new(c, c);
        let s2 = if end {
            sel.line_end(layout, false)
        } else {
            sel.line_start(layout, false)
        };
        let f = s2.focus();
        let at = self.paras[i].start + f.index().min(self.paras[i].len);
        self.set_focus_to(at, f.affinity(), extend);
        self.h_pos = None;
    }

    fn back_delete(&mut self, word: bool, ts: &mut TextSystem) {
        if !self.collapsed() {
            let (a, b) = self.sel_range();
            self.replace(a, b, "", ts);
            return;
        }
        let (i, l) = self.locate(self.focus);
        if l == 0 {
            if i > 0 {
                let a = self.paras[i].start - 1;
                self.replace(a, a + 1, "", ts);
            }
            return;
        }
        self.ensure(i, ts);
        let layout = self.layout_of(i);
        let c = self.cursor(i, l, self.affinity);
        let start = if word {
            c.previous_logical_word(layout).index()
        } else {
            match c.logical_clusters(layout)[0].clone() {
                Some(cl) if cl.is_emoji() => cl.text_range().start,
                _ => {
                    let text = &self.buffer[self.paras[i].start..self.paras[i].start + l];
                    text.char_indices().next_back().map(|(b, _)| b).unwrap_or(0)
                }
            }
        };
        let base = self.paras[i].start;
        self.replace(base + start.min(l), base + l, "", ts);
    }

    fn fore_delete(&mut self, word: bool, ts: &mut TextSystem) {
        if !self.collapsed() {
            let (a, b) = self.sel_range();
            self.replace(a, b, "", ts);
            return;
        }
        let (i, l) = self.locate(self.focus);
        let len = self.paras[i].len;
        if l >= len {
            if i + 1 < self.paras.len() {
                let a = self.paras[i].end();
                self.replace(a, a + 1, "", ts);
            }
            return;
        }
        self.ensure(i, ts);
        let layout = self.layout_of(i);
        let c = self.cursor(i, l, self.affinity);
        let end = if word {
            c.next_logical_word(layout).index()
        } else {
            match c.logical_clusters(layout)[1].clone() {
                Some(cl) => cl.text_range().end,
                None => len,
            }
        };
        let base = self.paras[i].start;
        self.replace(base + l, base + end.min(len).max(l), "", ts);
        self.anchor = base + l;
        self.focus = base + l;
    }

    /// A key, as `Editor::key`.
    pub fn key(
        &mut self,
        key: &str,
        shift: bool,
        word: bool,
        line: bool,
        ts: &mut TextSystem,
    ) -> Outcome {
        self.refresh(ts);
        let moved = |e: &mut Self| {
            let _ = e;
            Outcome::Moved
        };
        match key {
            "left" | "right" if line => {
                self.line_edge(key == "right", shift, ts);
                moved(self)
            }
            "left" | "right" => {
                self.horizontal(key == "right", word, shift, ts);
                moved(self)
            }
            "up" | "down" if line => {
                let at = if key == "down" { self.buffer.len() } else { 0 };
                self.set_focus_to(at, Affinity::Downstream, shift);
                self.h_pos = None;
                moved(self)
            }
            "up" | "down" => {
                self.vertical(key == "down", shift, ts);
                moved(self)
            }
            "home" | "end" => {
                self.line_edge(key == "end", shift, ts);
                moved(self)
            }
            "pageup" | "pagedown" => {
                self.page(key == "pagedown", shift, ts);
                moved(self)
            }
            "backspace" => self.mutate(ts, |e, ts| {
                if line && e.collapsed() {
                    // ⌘⌫: to the start of the line
                    let end = e.focus;
                    e.line_edge(false, false, ts);
                    let start = e.focus;
                    e.focus = end;
                    e.anchor = end;
                    if start < end {
                        e.replace(start, end, "", ts);
                    }
                } else {
                    e.back_delete(word, ts)
                }
            }),
            "delete" => self.mutate(ts, |e, ts| e.fore_delete(word, ts)),
            "enter" => self.insert("\n", ts),
            _ => Outcome::Pass,
        }
    }

    // ── the input method ────────────────────────────────────────────

    pub fn compose(&mut self, text: &str, cursor: Option<(usize, usize)>, ts: &mut TextSystem) {
        if self.readonly {
            return;
        }
        self.refresh(ts);
        if text.is_empty() {
            if let Some(r) = self.compose.take() {
                self.replace(r.start, r.end, "", ts);
                self.anchor = r.start;
                self.focus = r.start;
            }
            self.show_cursor = true;
            return;
        }
        let start = match self.compose.clone() {
            Some(r) => {
                self.replace(r.start, r.end, text, ts);
                r.start
            }
            None => {
                let (a, b) = self.sel_range();
                self.replace(a, b, text, ts);
                a
            }
        };
        self.compose = Some(start..start + text.len());
        self.show_cursor = cursor.is_some();
        let (c0, c1) = cursor.unwrap_or((0, 0));
        self.anchor = (start + c0).min(self.buffer.len());
        self.focus = (start + c1).min(self.buffer.len());
        self.snap_selection();
    }

    // ── the pointer ─────────────────────────────────────────────────

    /// A press at (x, y), device pixels from the text's top-left (scroll
    /// included).
    pub fn press(&mut self, x: f32, y: f32, clicks: u32, shift: bool, ts: &mut TextSystem) {
        self.refresh(ts);
        let (at, aff) = self.point_to_byte(x, y, ts);
        match clicks {
            2 => {
                let (i, _) = self.locate(at);
                self.ensure(i, ts);
                let s = Selection::word_from_point(self.layout_of(i), x, y - self.ys[i]);
                let r = s.text_range();
                let base = self.paras[i].start;
                self.anchor = base + r.start.min(self.paras[i].len);
                self.focus = base + r.end.min(self.paras[i].len);
            }
            n if n >= 3 => {
                let (i, _) = self.locate(at);
                self.anchor = self.paras[i].start;
                self.focus = self.paras[i].end();
            }
            _ => self.set_focus_to(at, aff, shift),
        }
        self.affinity = aff;
        self.h_pos = None;
        self.dragging = clicks <= 1;
        self.follow = true;
    }

    pub fn drag(&mut self, x: f32, y: f32, ts: &mut TextSystem) {
        if !self.dragging {
            return;
        }
        self.refresh(ts);
        let (at, aff) = self.point_to_byte(x, y, ts);
        self.set_focus_to(at, aff, true);
    }

    /// Lines wrapped at the width (the default) or not (`nowrap`: the
    /// text scrolls sideways).
    pub fn set_nowrap(&mut self, nowrap: bool) {
        if self.nowrap == nowrap {
            return;
        }
        self.nowrap = nowrap;
        self.scroll_x = 0.0;
        self.widest = 0.0;
        for p in &mut self.paras {
            p.rebreak = true;
            p.measured = false;
        }
        self.ys_ok = false;
    }

    /// Sideways, with `nowrap`: whether the view moved.
    pub fn wheel_x(&mut self, dx: f32) -> bool {
        if !self.nowrap {
            return false;
        }
        let max = (self.widest - self.width + 24.0 * self.scale).max(0.0);
        let before = self.scroll_x;
        self.scroll_x = (self.scroll_x - dx * self.scale).clamp(0.0, max);
        (self.scroll_x - before).abs() > 0.01
    }

    /// A caret's box at byte `local` of paragraph `i` (laid out), device
    /// pixels from the text's top-left (no scroll).
    pub fn caret_box_at(&mut self, i: usize, local: usize, width: f32, ts: &mut TextSystem) -> BoundingBox {
        self.ensure(i, ts);
        self.refresh(ts);
        let g = self.cursor(i, local, Affinity::Downstream).geometry(self.layout_of(i), width);
        let y = self.ys[i] as f64;
        BoundingBox::new(g.x0, g.y0 + y, g.x1, g.y1 + y)
    }

    /// Paragraph `i` laid out (for drawing it out of its place).
    pub fn ensure_line(&mut self, i: usize, ts: &mut TextSystem) {
        self.ensure(i, ts);
        self.refresh(ts);
    }

    /// Line `i`'s top and height (device pixels), laid out.
    pub fn line_box(&mut self, i: usize, ts: &mut TextSystem) -> (f32, f32) {
        self.ensure(i, ts);
        self.refresh(ts);
        (self.ys[i], self.paras[i].h)
    }

    pub fn wheel(&mut self, dy: f32, ts: &mut TextSystem) -> bool {
        let total = self.content_height(ts);
        let max = (total - self.view_h).max(0.0);
        let before = self.scroll_y;
        self.scroll_y = (self.scroll_y - dy * self.scale).clamp(0.0, max);
        (self.scroll_y - before).abs() > 0.01
    }

    // ── geometry ────────────────────────────────────────────────────

    /// The caret, device pixels from the text's top-left (no scroll).
    pub fn caret_box(&mut self, width: f32, ts: &mut TextSystem) -> BoundingBox {
        let (i, l) = self.locate(self.focus);
        self.ensure(i, ts);
        self.refresh(ts);
        let g = self
            .cursor(i, l, self.affinity)
            .geometry(self.layout_of(i), width);
        let y = self.ys[i] as f64;
        BoundingBox::new(g.x0, g.y0 + y, g.x1, g.y1 + y)
    }

    pub fn show_cursor(&self) -> bool {
        self.show_cursor
    }

    /// Where the input method's window goes: the composition's start, or
    /// the caret.
    pub fn ime_box(&mut self, ts: &mut TextSystem) -> BoundingBox {
        let at = self.compose.as_ref().map(|r| r.start).unwrap_or(self.focus);
        let (i, l) = self.locate(at);
        self.ensure(i, ts);
        self.refresh(ts);
        let g = self
            .cursor(i, l, Affinity::Downstream)
            .geometry(self.layout_of(i), 1.0);
        let y = self.ys[i] as f64;
        BoundingBox::new(g.x0, g.y0 + y, g.x1, g.y1 + y)
    }

    /// Keep the caret in a view `h` high: adjust `scroll_y`.
    pub fn follow_caret(&mut self, h: f32, ts: &mut TextSystem) {
        self.view_h = h;
        let total = self.content_height(ts);
        let max = (total - h).max(0.0);
        if self.follow {
            let c = self.caret_box(1.0, ts);
            let (y0, y1) = (c.y0 as f32, c.y1 as f32);
            if y1 - self.scroll_y > h {
                self.scroll_y = y1 - h;
            }
            if y0 < self.scroll_y {
                self.scroll_y = y0;
            }
            if self.nowrap {
                let margin = 32.0 * self.scale;
                let (x0, x1) = (c.x0 as f32, c.x1 as f32);
                if x1 - self.scroll_x > self.width - margin {
                    self.scroll_x = x1 - self.width + margin;
                }
                if x0 < self.scroll_x + margin {
                    self.scroll_x = (x0 - margin).max(0.0);
                }
            }
            self.follow = false;
        }
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }

    /// The paragraphs a band `y0..y1` (device pixels, no scroll) shows.
    /// Those shown are laid out (their measured heights may move the
    /// band: it is found again until it holds still).
    pub fn visible(&mut self, y0: f32, y1: f32, ts: &mut TextSystem) -> std::ops::Range<usize> {
        let mut band = 0..0;
        for _ in 0..4 {
            self.refresh(ts);
            let find = |y: f32| match self
                .ys
                .binary_search_by(|v| v.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Less))
            {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };
            let a = find(y0).min(self.paras.len());
            let b = (find(y1) + 1).min(self.paras.len());
            band = a..b.max(a);
            for i in band.clone() {
                self.ensure(i, ts);
            }
            if self.ys_ok {
                break;
            }
        }
        self.refresh(ts);
        self.trim_layouts(band.clone());
        band
    }

    /// The first line in view (a paragraph's index).
    pub fn top_line(&mut self, ts: &mut TextSystem) -> usize {
        self.refresh(ts);
        let y = self.scroll_y;
        match self
            .ys
            .binary_search_by(|v| v.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Less))
        {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
        .min(self.paras.len().saturating_sub(1))
    }

    /// Scroll so line `line` is the first in view.
    pub fn scroll_to_line(&mut self, line: usize, ts: &mut TextSystem) {
        let i = line.min(self.paras.len().saturating_sub(1));
        self.ensure(i, ts);
        self.refresh(ts);
        let total = *self.ys.last().unwrap_or(&0.0);
        let max = (total - self.view_h).max(0.0);
        self.scroll_y = if self.view_h > 0.0 { self.ys[i].min(max) } else { self.ys[i] };
        self.follow = false;
    }

    /// Characters `a..b` of paragraph `i` as its local bytes.
    pub fn para_char_bytes(&self, i: usize, a: usize, b: usize) -> (usize, usize) {
        let p = &self.paras[i];
        let text = &self.buffer[p.start..p.end()];
        (char_at(text, a), char_at(text, b.max(a)))
    }

    /// The paragraph the caret is in.
    pub fn focus_para(&self) -> usize {
        self.locate(self.focus).0
    }

    pub fn style_of(&self, i: u32) -> Option<&SpanStyle> {
        self.styles.get(i as usize)
    }

    /// Paragraph `i`: its top (device pixels), its layout, its height.
    pub fn para(&self, i: usize) -> (f32, &Layout<Color>, f32) {
        (self.ys[i], self.layout_of(i), self.paras[i].h)
    }

    /// Paragraph `i`'s spans with their styles.
    pub fn para_spans(&self, i: usize) -> Vec<(Span, &SpanStyle)> {
        self.paras[i]
            .spans
            .iter()
            .filter_map(|sp| self.styles.get(sp.style as usize).map(|st| (*sp, st)))
            .collect()
    }

    /// The selection's part in paragraph `i`, local bytes, and whether it
    /// goes on past the paragraph's end (its line break is selected).
    pub fn para_selection(&self, i: usize) -> Option<(usize, usize, bool)> {
        let (a, b) = self.sel_range();
        if a == b {
            return None;
        }
        let p = &self.paras[i];
        if b < p.start || a > p.end() {
            return None;
        }
        let s = a.max(p.start) - p.start;
        let e = b.min(p.end()) - p.start;
        Some((s, e, b > p.end()))
    }

    /// The composition's part in paragraph `i`, local bytes.
    pub fn para_compose(&self, i: usize) -> Option<(usize, usize)> {
        let r = self.compose.as_ref()?;
        let p = &self.paras[i];
        if r.end < p.start || r.start > p.end() {
            return None;
        }
        Some((r.start.max(p.start) - p.start, r.end.min(p.end()) - p.start))
    }

    /// Geometry of local bytes `a..b` in paragraph `i`: one box a line.
    pub fn range_boxes(&self, i: usize, a: usize, b: usize) -> Vec<BoundingBox> {
        let layout = self.layout_of(i);
        let sel = Selection::new(
            Cursor::from_byte_index(layout, a, Affinity::Downstream),
            Cursor::from_byte_index(layout, b.min(self.paras[i].len), Affinity::Upstream),
        );
        let mut out = Vec::new();
        sel.geometry_with(layout, |bb, _| out.push(bb));
        out
    }

    /// Paragraph `i`'s top and height (device pixels), when laid out.
    pub fn para_top(&self, i: usize) -> Option<(f32, f32)> {
        (self.ys_ok && i < self.paras.len()).then(|| (self.ys[i], self.paras[i].h))
    }

    pub fn para_count(&self) -> usize {
        self.paras.len()
    }

    /// Paragraph `i`'s text.
    pub fn para_text(&self, i: usize) -> &str {
        let p = &self.paras[i];
        &self.buffer[p.start..p.end()]
    }

    /// The character offset where paragraph `i` starts.
    pub fn para_char_start(&self, i: usize) -> usize {
        chars_of(&self.buffer, self.paras[i].start)
    }

    /// The selection as (paragraph, character in it) for each end.
    pub fn selection_in_paras(&self) -> ((usize, usize), (usize, usize)) {
        let f = |at: usize| {
            let (i, l) = self.locate(at);
            let text = &self.buffer[self.paras[i].start..self.paras[i].start + l];
            (i, text.chars().count())
        };
        (f(self.anchor), f(self.focus))
    }

    /// Select from (paragraph, character) to (paragraph, character): an
    /// assistive client's selection.
    pub fn select_paras(&mut self, a: (usize, usize), f: (usize, usize)) {
        let g = |(i, c): (usize, usize)| {
            let i = i.min(self.paras.len() - 1);
            let p = &self.paras[i];
            p.start + char_at(&self.buffer[p.start..p.end()], c)
        };
        self.anchor = g(a);
        self.focus = g(f);
        self.h_pos = None;
        self.follow = true;
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The character under a point (device pixels from the text's top,
    /// scroll included): its line, its column, and its box — or none
    /// past a line's end, below the text, or over a lens.
    pub fn hover_at(&mut self, x: f32, y: f32, ts: &mut TextSystem) -> Option<(usize, usize, BoundingBox)> {
        if x < 0.0 || y < 0.0 || self.lens_at(y, ts).is_some() {
            return None;
        }
        self.refresh(ts);
        let total = *self.ys.last().unwrap_or(&0.0);
        if y >= total {
            return None;
        }
        let (at, _) = self.point_to_byte(x, y, ts);
        let (i, l) = self.locate(at);
        self.ensure(i, ts);
        let layout = self.layout_of(i);
        if x > layout.width() + 2.0 || y - self.ys[i] > self.paras[i].h {
            return None;
        }
        let g = Cursor::from_byte_index(layout, l, Affinity::Downstream).geometry(layout, 1.0);
        let p = &self.paras[i];
        let col = self.buffer[p.start..p.start + l].chars().count();
        let w = (self.font.size * self.scale * 0.6) as f64;
        let top = self.ys[i] as f64;
        Some((i, col, BoundingBox::new(g.x0, g.y0 + top, g.x0 + w, g.y1 + top)))
    }

    /// The line whose first row is at `y` (the text's own pixels), and the
    /// box a drawn mark takes on it (x from 0) — what the pointer rests on
    /// in the gutter.
    pub fn gutter_line_at(&mut self, y: f32, ts: &mut TextSystem) -> Option<(usize, BoundingBox)> {
        if y < 0.0 || self.lens_at(y, ts).is_some() {
            return None;
        }
        self.refresh(ts);
        let total = *self.ys.last().unwrap_or(&0.0);
        if y >= total {
            return None;
        }
        let (at, _) = self.point_to_byte(0.0, y, ts);
        let (i, _) = self.locate(at);
        self.ensure(i, ts);
        let lh = if self.font.line_height > 0.0 { self.font.line_height } else { 1.2 };
        let row = self.font.size * self.scale * lh;
        let side = MARK_BOX * self.scale;
        let top = self.ys[i] + ((row - side) / 2.0).max(0.0);
        (y - self.ys[i] <= self.paras[i].h)
            .then(|| (i, BoundingBox::new(0.0, top as f64, side as f64, (top + side) as f64)))
    }

    // ── lenses ──────────────────────────────────────────────────────

    /// The program's lenses: the tops move only when a lens's line or
    /// height did.
    pub fn set_lenses(&mut self, lenses: Vec<Lens>) {
        let same_room = lenses.len() == self.lenses.len()
            && lenses.iter().zip(&self.lenses).all(|(a, b)| a.line == b.line && a.height == b.height);
        if !same_room {
            self.ys_ok = false;
        }
        self.lenses = lenses;
    }

    pub fn lenses(&self) -> &[Lens] {
        &self.lenses
    }

    /// Lens `k`'s band: its top (device pixels from the text's top, no
    /// scroll) and height, when its line exists and is laid out.
    pub fn lens_band(&self, k: usize) -> Option<(f32, f32)> {
        let l = self.lenses.get(k)?;
        if !self.ys_ok || l.line >= self.paras.len() {
            return None;
        }
        Some((self.ys[l.line] + self.height_of(l.line), l.height * self.scale))
    }

    /// The lens under `y` (device pixels from the text's top, scroll
    /// included): its index and `y` within it.
    pub fn lens_at(&mut self, y: f32, ts: &mut TextSystem) -> Option<(usize, f32)> {
        if self.lenses.is_empty() {
            return None;
        }
        self.refresh(ts);
        (0..self.lenses.len()).find_map(|k| {
            let (top, h) = self.lens_band(k)?;
            (y >= top && y < top + h).then_some((k, y - top))
        })
    }
}

/// The bytes two texts share at the start and (after that) at the end,
/// on character boundaries.
fn common_ends(a: &str, b: &str) -> (usize, usize) {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let mut p = common_prefix(ab, bb);
    while p > 0 && (!a.is_char_boundary(p) || !b.is_char_boundary(p)) {
        p -= 1;
    }
    let max_s = (ab.len() - p).min(bb.len() - p);
    let mut s = common_suffix(&ab[ab.len() - max_s..], &bb[bb.len() - max_s..]);
    while s > 0 && (!a.is_char_boundary(ab.len() - s) || !b.is_char_boundary(bb.len() - s)) {
        s -= 1;
    }
    (p, s)
}

/// How many bytes two slices share at the start: blocks compared whole
/// (a memory compare), the first that differs byte by byte — a 2 MB text
/// with an edit in its middle is compared in microseconds.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 256;
    let n = a.len().min(b.len());
    let mut at = 0;
    while at + BLOCK <= n && a[at..at + BLOCK] == b[at..at + BLOCK] {
        at += BLOCK;
    }
    while at < n && a[at] == b[at] {
        at += 1;
    }
    at
}

/// How many bytes two slices share at the end (as `common_prefix`).
fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 256;
    let n = a.len().min(b.len());
    let (la, lb) = (a.len(), b.len());
    let mut k = 0;
    while k + BLOCK <= n && a[la - k - BLOCK..la - k] == b[lb - k - BLOCK..lb - k] {
        k += BLOCK;
    }
    while k < n && a[la - k - 1] == b[lb - k - 1] {
        k += 1;
    }
    k
}

/// What changed between two values, by line and by character.
pub fn delta_of(old: &str, now: &str) -> Delta {
    let (p, s) = common_ends(old, now);
    let first = old[..p].matches('\n').count();
    let line_start = old[..p].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let old_end = old.len() - s;
    let removed = old[line_start..old_end].matches('\n').count() + 1;
    let new_end = now.len() - s;
    let new_line_end = now[new_end..]
        .find('\n')
        .map(|i| new_end + i)
        .unwrap_or(now.len());
    let lines = now[line_start..new_line_end]
        .split('\n')
        .map(|x| x.to_string())
        .collect();
    Delta {
        first,
        removed,
        lines,
        at: old[..p].chars().count(),
        old_len: old[p..old_end].chars().count(),
        inserted: now[p..new_end].to_string(),
    }
}

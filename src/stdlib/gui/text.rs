//! Text: shaping, line breaking, and measurement (parley), shared by
//! `gui.measure`, the scene's text nodes, and the editors.
//!
//! One process-wide [`TextSystem`] holds parley's font database and
//! layout scratch space behind a lock. A shaped text is cached by
//! everything that changes its glyphs — the text, the font, the width
//! it wraps at, the alignment, and the display scale — so `gui.measure`
//! followed by drawing the same text shapes it once.
//!
//! Positions in a [`Shaped`] are in *device* pixels (the layout is built
//! at the display's scale, so hinting and quantization see the real
//! size); its `width` and `height` are reported to programs in logical
//! pixels by dividing by the scale.

use parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, FontContext, FontFamily, FontStyle, FontWeight,
    Layout, LayoutContext, LineHeight, PositionedLayoutItem, Selection, StyleProperty,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

/// A straight (not premultiplied) sRGB colour.
pub type Color = [u8; 4];

/// What a text is set in. `family` is `"body"`, `"mono"`, or a CSS
/// family list (`"Inter, sans-serif"`).
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: f32,
    pub weight: f32,
    pub italic: bool,
    /// A multiple of the font size; 0 is the font's own line height.
    pub line_height: f32,
    /// Styled runs within the text (characters): a preview's bold, code,
    /// links (`rich.rs`'s looks).
    pub spans: Option<Arc<Vec<(usize, usize, super::rich::SpanStyle)>>>,
    /// Whether the font's ligatures and contextual alternates are on
    /// (`liga`, `calt`): a code font's `=>` drawn as one arrow. On by
    /// default; a code editor turns them off unless asked.
    pub ligatures: bool,
}

impl Default for Font {
    fn default() -> Self {
        Font {
            family: "body".to_string(),
            size: 14.0,
            weight: 400.0,
            italic: false,
            line_height: 0.0,
            spans: None,
            ligatures: true,
        }
    }
}

impl Eq for Font {}

impl Hash for Font {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.family.hash(h);
        self.size.to_bits().hash(h);
        self.weight.to_bits().hash(h);
        self.italic.hash(h);
        self.line_height.to_bits().hash(h);
        self.ligatures.hash(h);
        if let Some(sp) = &self.spans {
            sp.len().hash(h);
            for (a, b, st) in sp.iter() {
                (a, b).hash(h);
                st.weight.map(f32::to_bits).hash(h);
                st.italic.hash(h);
                st.mono.hash(h);
                st.size.map(f32::to_bits).hash(h);
                st.color.hash(h);
            }
        }
    }
}

impl Font {
    /// The CSS family list parley resolves.
    pub fn family_source(&self) -> String {
        match self.family.as_str() {
            // The bundled Inter and emoji (Loom's fonts) before the
            // system's: a window looks the same on every machine.
            "body" | "" => "Inter Variable, Inter, Noto Color Emoji, system-ui, sans-serif".to_string(),
            // The system's own monospace faces by name first: a generic
            // `monospace` resolves to Courier on macOS. The bundled
            // JetBrains Mono (Loom's fonts) comes before all of them.
            "mono" => "JetBrains Mono, SF Mono, Menlo, Consolas, DejaVu Sans Mono, Noto Sans Mono, Noto Color Emoji, ui-monospace, monospace".to_string(),
            other => other.to_string(),
        }
    }

    pub fn styles(&self) -> Vec<StyleProperty<'static, Color>> {
        let mut v = vec![
            StyleProperty::FontFamily(FontFamily::Source(self.family_source().into())),
            StyleProperty::FontSize(self.size),
            StyleProperty::FontWeight(FontWeight::new(self.weight)),
            StyleProperty::FontStyle(if self.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            }),
        ];
        v.push(optical_size(self.size));
        if self.line_height > 0.0 {
            v.push(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
                self.line_height,
            )));
        }
        if !self.ligatures {
            v.push(StyleProperty::FontFeatures(parley::FontFeatures::Source(
                "\"liga\" 0, \"calt\" 0, \"clig\" 0".into(),
            )));
        }
        v
    }
}

/// A variable font's optical size set to the text's size (logical
/// pixels, which are macOS's points), as Core Text and CSS's
/// `font-optical-sizing: auto` do: Inter's display forms above 14 px, SF
/// Pro's Text and Display. A font with no `opsz` axis ignores it.
pub fn optical_size(size: f32) -> StyleProperty<'static, Color> {
    let opsz = if size.is_finite() {
        size.clamp(1.0, 1000.0)
    } else {
        14.0
    };
    StyleProperty::FontVariations(parley::FontVariations::Source(
        format!("\"opsz\" {opsz}").into(),
    ))
}

/// Horizontal alignment of the lines of a text in its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Align {
    /// Where the text's own direction begins: the left of an English
    /// line, the right of a Hebrew one.
    #[default]
    Start,
    Center,
    End,
    /// The left whatever the text's direction (a right-to-left window
    /// sets `right` on its English labels).
    Left,
    Right,
}

impl Align {
    pub fn parse(s: &str) -> Option<Align> {
        match s {
            "start" => Some(Align::Start),
            "center" => Some(Align::Center),
            "end" => Some(Align::End),
            "left" => Some(Align::Left),
            "right" => Some(Align::Right),
            _ => None,
        }
    }

    pub fn parley(self) -> Alignment {
        match self {
            Align::Start => Alignment::Start,
            Align::Center => Alignment::Center,
            Align::End => Alignment::End,
            Align::Left => Alignment::Left,
            Align::Right => Alignment::Right,
        }
    }

    /// As a position along a box (vertical alignment, where there is no
    /// direction): left is the start, right the end.
    pub fn along(self) -> Align {
        match self {
            Align::Left => Align::Start,
            Align::Right => Align::End,
            other => other,
        }
    }
}

/// A font face as the rasterizer needs it: the file's bytes and the
/// face's index in a collection. Keyed by the bytes' identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FaceKey {
    pub blob: u64,
    pub index: u32,
}

/// One run of glyphs in one face, size, and colour.
#[derive(Clone, Debug)]
pub struct GlyphRun {
    pub face: FaceKey,
    /// Pixels per em, in device pixels.
    pub size: f32,
    pub coords: Arc<[i16]>,
    pub embolden: bool,
    pub skew: bool,
    pub color: Color,
    /// (glyph id, x, y), device pixels from the text's origin; y is the
    /// baseline.
    pub glyphs: Vec<(u16, f32, f32)>,
}

/// One line's metrics, device pixels from the text's origin.
#[derive(Clone, Debug)]
pub struct LineInfo {
    pub top: f32,
    pub height: f32,
    pub baseline: f32,
}

/// A shaped, broken, aligned text.
#[derive(Debug)]
pub struct Shaped {
    /// Device pixels.
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub lines: Vec<LineInfo>,
    pub runs: Vec<GlyphRun>,
    /// A span's background, strike and underline (device pixels from the
    /// layout's origin).
    pub decos: Vec<Deco>,
}

/// What a span draws besides its glyphs: its `bg` behind them (`over`
/// false), a strike or an underline over them.
#[derive(Clone, Copy, Debug)]
pub struct Deco {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Color,
    pub over: bool,
}

/// The decorations of `text`'s spans (character offsets) in `layout`.
fn span_decos(
    layout: &Layout<Color>,
    text: &str,
    spans: &[(usize, usize, super::rich::SpanStyle)],
    fg: Color,
    scale: f32,
) -> Vec<Deco> {
    let mut at: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    at.push(text.len());
    let mut out = Vec::new();
    for (a, b, st) in spans.iter() {
        if st.bg.is_none() && !st.strike && !st.underline {
            continue;
        }
        let (a, b) = (at[(*a).min(at.len() - 1)], at[(*b).min(at.len() - 1)]);
        if a >= b {
            continue;
        }
        let sel = Selection::new(
            Cursor::from_byte_index(layout, a, Affinity::Downstream),
            Cursor::from_byte_index(layout, b, Affinity::Upstream),
        );
        let mut boxes = Vec::new();
        sel.geometry_with(layout, |bb, _| boxes.push(bb));
        let c = st.color.unwrap_or(fg);
        let thin = scale.max(1.0);
        for bb in boxes {
            let (x, y, w, h) = (
                bb.x0 as f32,
                bb.y0 as f32,
                (bb.x1 - bb.x0) as f32,
                (bb.y1 - bb.y0) as f32,
            );
            if let Some(bg) = st.bg {
                out.push(Deco {
                    x,
                    y,
                    w,
                    h,
                    color: bg,
                    over: false,
                });
            }
            if st.strike {
                out.push(Deco {
                    x,
                    y: (y + h * 0.55).round(),
                    w,
                    h: thin,
                    color: c,
                    over: true,
                });
            }
            if st.underline {
                out.push(Deco {
                    x,
                    y: (y + h - 2.0 * scale).round(),
                    w,
                    h: thin,
                    color: c,
                    over: true,
                });
            }
        }
    }
    out
}

impl Shaped {
    pub fn logical_size(&self) -> (f32, f32) {
        (self.width / self.scale, self.height / self.scale)
    }
}

/// What a shaped text is cached by.
#[derive(Clone, PartialEq)]
struct CacheKey {
    text: Box<str>,
    font: Font,
    color: Color,
    max_width: Option<u32>,
    align: Align,
    scale: u32,
}

impl CacheKey {
    fn is(
        &self,
        text: &str,
        font: &Font,
        color: Color,
        max_width: Option<u32>,
        align: Align,
        scale: u32,
    ) -> bool {
        &*self.text == text
            && self.color == color
            && self.max_width == max_width
            && self.align == align
            && self.scale == scale
            && self.font == *font
    }
}

/// The hash a shaped text is found by, from borrowed parts (a lookup
/// copies nothing).
fn cache_hash(
    text: &str,
    font: &Font,
    color: Color,
    max_width: Option<u32>,
    align: Align,
    scale: u32,
) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    text.hash(&mut h);
    font.hash(&mut h);
    color.hash(&mut h);
    max_width.hash(&mut h);
    align.hash(&mut h);
    scale.hash(&mut h);
    h.finish()
}

type ShapeCache =
    HashMap<u64, (CacheKey, Arc<Shaped>), std::hash::BuildHasherDefault<rustc_hash::FxHasher>>;

pub struct TextSystem {
    pub font_cx: FontContext,
    pub layout_cx: LayoutContext<Color>,
    /// Shaped texts in two generations: one used since the last turn-over
    /// and the one before it. A text found in the old generation moves to
    /// the new; a turn-over drops only what neither used — a frame's texts
    /// survive however many were shaped (a full reset re-shaped them all).
    cache: ShapeCache,
    old: ShapeCache,
    /// Shaped (missed) and found, since the start: `gui.read(_, "render")`.
    pub shaped: u64,
    pub found: u64,
}

/// Faces the rasterizer can read, by key. Filled as layouts are made.
static FACES: OnceLock<Mutex<HashMap<FaceKey, parley::FontData>>> = OnceLock::new();

fn faces() -> &'static Mutex<HashMap<FaceKey, parley::FontData>> {
    FACES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The bytes and index of a face seen in some layout.
pub fn face_data(key: FaceKey) -> Option<parley::FontData> {
    faces().lock().ok()?.get(&key).cloned()
}

pub fn remember_face(font: &parley::FontData) -> FaceKey {
    let key = FaceKey {
        blob: font.data.id(),
        index: font.index,
    };
    if let Ok(mut f) = faces().lock() {
        f.entry(key).or_insert_with(|| font.clone());
    }
    key
}

static SYSTEM: OnceLock<Mutex<TextSystem>> = OnceLock::new();

/// Fonts being registered on a thread of their own
/// (`gui.fonts(…, #{ "background": true })`): every use of the text
/// system waits for them, so no text is ever shaped without them.
static FONTS_PENDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static FONTS_GATE: (Mutex<()>, std::sync::Condvar) = (Mutex::new(()), std::sync::Condvar::new());

/// One background registration under way; done when dropped (a panic in
/// it included).
pub struct FontsPending(());

impl FontsPending {
    pub fn begin() -> FontsPending {
        FONTS_PENDING.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        FontsPending(())
    }
}

impl Drop for FontsPending {
    fn drop(&mut self) {
        let _g = FONTS_GATE.0.lock().unwrap_or_else(|e| e.into_inner());
        FONTS_PENDING.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        FONTS_GATE.1.notify_all();
    }
}

fn wait_for_fonts() {
    let mut g = FONTS_GATE.0.lock().unwrap_or_else(|e| e.into_inner());
    while FONTS_PENDING.load(std::sync::atomic::Ordering::Acquire) > 0 {
        g = FONTS_GATE.1.wait(g).unwrap_or_else(|e| e.into_inner());
    }
}

/// The process's text system, once the fonts being registered are in.
/// Created on first use: discovering the system's fonts takes tens of
/// milliseconds, paid once (on the registering thread, when fonts are
/// registered in the background).
pub fn system() -> &'static Mutex<TextSystem> {
    if FONTS_PENDING.load(std::sync::atomic::Ordering::Acquire) > 0 {
        wait_for_fonts();
    }
    system_now()
}

/// The text system without waiting: the background registration's own.
pub fn system_now() -> &'static Mutex<TextSystem> {
    SYSTEM.get_or_init(|| {
        Mutex::new(TextSystem {
            font_cx: FontContext::new(),
            layout_cx: LayoutContext::new(),
            cache: ShapeCache::default(),
            old: ShapeCache::default(),
            shaped: 0,
            found: 0,
        })
    })
}

/// The emoji fonts, bundled first.
const EMOJI_FAMILIES: &str = "Noto Color Emoji, Apple Color Emoji, Segoe UI Emoji, emoji";

/// The byte ranges of the characters followed by the emoji presentation
/// selector, with the selector (and a keycap's combining mark).
fn emoji_presentation_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    for (i, &(at, _c)) in chars.iter().enumerate() {
        if let Some(&(sel, '\u{FE0F}')) = chars.get(i + 1) {
            let mut end = sel + '\u{FE0F}'.len_utf8();
            if let Some(&(k, '\u{20E3}')) = chars.get(i + 2) {
                end = k + '\u{20E3}'.len_utf8();
            }
            out.push(at..end);
        }
    }
    out
}

/// The most shaped texts a generation keeps (the cache holds at most
/// twice this).
const CACHE_LIMIT: usize = 8192;

impl TextSystem {
    /// Register font files (the bundled set, a program's own). Answers
    /// how many faces were added.
    pub fn register(&mut self, bytes: Vec<u8>) -> usize {
        let blob = parley::fontique::Blob::from(bytes);
        let added = self.font_cx.collection.register_fonts(blob, None);
        self.cache.clear();
        self.old.clear();
        added.iter().map(|(_, faces)| faces.len()).sum()
    }

    /// Shape `text` (cached).
    pub fn shape(
        &mut self,
        text: &str,
        font: &Font,
        color: Color,
        max_width: Option<f32>,
        align: Align,
        scale: f32,
    ) -> Arc<Shaped> {
        // Widths are logical; quantize to 1/64 px so float noise in a
        // program's layout does not defeat the cache.
        let qw = max_width.map(|w| (w.max(0.0) * 64.0).round() as u32);
        let qs = (scale * 1000.0).round() as u32;
        let h = cache_hash(text, font, color, qw, align, qs);
        if let Some((k, s)) = self.cache.get(&h)
            && k.is(text, font, color, qw, align, qs)
        {
            self.found += 1;
            return s.clone();
        }
        if let Some((k, s)) = self.old.remove(&h)
            && k.is(text, font, color, qw, align, qs)
        {
            self.found += 1;
            let s2 = s.clone();
            self.keep(h, k, s);
            return s2;
        }
        self.shaped += 1;
        let key = CacheKey {
            text: text.into(),
            font: font.clone(),
            color,
            max_width: qw,
            align,
            scale: qs,
        };
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, scale, true);
        for style in font.styles() {
            builder.push_default(style);
        }
        builder.push_default(StyleProperty::Brush(color));
        // A character asked for in emoji presentation (followed by U+FE0F:
        // "❤️", "1️⃣") is drawn from an emoji font, though the text font
        // has a plain glyph for it.
        if let Some(spans) = &font.spans {
            let mut at: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
            at.push(text.len());
            for (a, b, st) in spans.iter() {
                let (a, b) = (at[(*a).min(at.len() - 1)], at[(*b).min(at.len() - 1)]);
                if a < b {
                    for prop in st.props(1.0) {
                        builder.push(prop, a..b);
                    }
                }
            }
        }
        for range in emoji_presentation_ranges(text) {
            builder.push(
                StyleProperty::FontFamily(FontFamily::Source(EMOJI_FAMILIES.into())),
                range,
            );
        }
        let mut layout: Layout<Color> = builder.build(text);
        layout.break_all_lines(max_width.map(|w| w * scale));
        // A line is aligned within the width it was broken at; with no
        // width there is nothing to align within.
        let alignment = if max_width.is_some() {
            align.parley()
        } else {
            Alignment::Start
        };
        layout.align(alignment, AlignmentOptions::default());
        let mut sh = shaped_of(&layout, scale);
        if let Some(spans) = &font.spans {
            sh.decos = span_decos(&layout, text, spans, color, scale);
        }
        let shaped = Arc::new(sh);
        self.keep(h, key, shaped.clone());
        shaped
    }

    /// Put a shaped text in the new generation, turning the generations
    /// over when it is full.
    fn keep(&mut self, h: u64, key: CacheKey, shaped: Arc<Shaped>) {
        if self.cache.len() >= CACHE_LIMIT {
            self.old = std::mem::take(&mut self.cache);
        }
        self.cache.insert(h, (key, shaped));
    }

    /// Shaped texts kept.
    pub fn cached(&self) -> usize {
        self.cache.len() + self.old.len()
    }
}

/// One visual line of a field's text as assistive output places it: its
/// paragraph, its first character there, its top and height (device
/// pixels from the text's top-left, no scroll), and each of its
/// characters' left edge and width (device pixels from the text's left),
/// in order — what Zoom and VoiceOver need to follow the caret and box a
/// word.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldLine {
    pub para: usize,
    pub start: usize,
    pub top: f32,
    pub height: f32,
    pub xs: Vec<f32>,
    pub widths: Vec<f32>,
}

/// A visual line: (first byte, end byte, top, height, each character's
/// (byte, left, width) in byte order).
pub type VisualLine = (usize, usize, f32, f32, Vec<(usize, f32, f32)>);

/// The visual lines of `layout` (of `text`). A cluster's advance is
/// shared among its characters (a ligature's).
pub fn layout_lines(layout: &Layout<Color>, text: &str) -> Vec<VisualLine> {
    let mut out = Vec::new();
    for line in layout.lines() {
        let m = line.metrics();
        let r = line.text_range();
        let mut chars: Vec<(usize, f32, f32)> = Vec::new();
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(gr) = item else {
                continue;
            };
            let mut x = gr.offset();
            for c in gr.run().visual_clusters() {
                let cr = c.text_range();
                let adv = c.advance();
                let piece = text.get(cr.clone()).unwrap_or("");
                let n = piece.chars().count().max(1) as f32;
                for (k, (b, _)) in piece.char_indices().enumerate() {
                    chars.push((cr.start + b, x + adv * k as f32 / n, adv / n));
                }
                x += adv;
            }
        }
        chars.sort_by_key(|c| c.0);
        chars.dedup_by_key(|c| c.0);
        out.push((
            r.start,
            r.end,
            m.block_min_coord,
            m.block_max_coord - m.block_min_coord,
            chars,
        ));
    }
    out
}

/// The glyph runs and line metrics of a broken, aligned layout.
pub fn shaped_of(layout: &Layout<Color>, scale: f32) -> Shaped {
    let mut runs = Vec::new();
    let mut lines = Vec::new();
    for line in layout.lines() {
        let m = line.metrics();
        lines.push(LineInfo {
            top: m.block_min_coord,
            height: m.block_max_coord - m.block_min_coord,
            baseline: m.baseline,
        });
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let style = run.style();
            let r = run.run();
            let synthesis = r.synthesis();
            let face = remember_face(r.font());
            runs.push(GlyphRun {
                face,
                size: r.font_size(),
                coords: r.normalized_coords().into(),
                embolden: synthesis.embolden(),
                skew: synthesis.skew().is_some(),
                color: style.brush,
                glyphs: run
                    .positioned_glyphs()
                    .map(|g| (g.id as u16, g.x, g.y))
                    .collect(),
            });
        }
    }
    Shaped {
        width: layout.width(),
        height: layout.height(),
        scale,
        lines,
        runs,
        decos: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::emoji_presentation_ranges;

    #[test]
    fn a_character_asked_for_as_emoji_is_sent_to_the_emoji_font() {
        let t = "a ❤️ b 1️⃣ c ❤";
        let got: Vec<&str> = emoji_presentation_ranges(t)
            .into_iter()
            .map(|r| &t[r])
            .collect();
        assert_eq!(got, vec!["❤\u{FE0F}", "1\u{FE0F}\u{20E3}"]);
    }
}

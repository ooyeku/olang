//! Glyph images (swash), shared by both renderers.
//!
//! A glyph is rasterized once per face, glyph, size, variation, and
//! quarter-pixel horizontal offset, and cached. Outlines become 8-bit
//! coverage masks the renderer tints with the text's colour; colour
//! glyphs (COLR outlines, sbix and CBDT bitmaps — emoji) become
//! premultiplied RGBA images drawn as they are. The cache is bounded by
//! count and bytes, the least recently drawn glyphs going first.

use super::text::FaceKey;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};

/// Horizontal positions are rounded to a quarter pixel.
pub const SUBPIXEL_STEPS: f32 = 4.0;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    pub face: FaceKey,
    pub glyph: u16,
    /// Size in 1/64 px.
    pub size: u32,
    pub subpixel: u8,
    pub coords: u64,
    pub embolden: bool,
    pub skew: bool,
    /// The text colour's lightness, in [`TONES`] steps: how much the
    /// outline is darkened (see [`darken`]).
    pub tone: u8,
}

impl GlyphKey {
    pub fn new(
        face: FaceKey,
        glyph: u16,
        size: f32,
        x: f32,
        coords: &[i16],
        embolden: bool,
        skew: bool,
        color: [u8; 4],
    ) -> GlyphKey {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        coords.hash(&mut h);
        GlyphKey {
            face,
            glyph,
            size: (size * 64.0).round() as u32,
            subpixel: subpixel_of(x),
            coords: h.finish(),
            embolden,
            skew,
            tone: tone_of(color),
        }
    }
}

/// The steps of a text colour's lightness a glyph is darkened by.
pub const TONES: u8 = 4;

/// A text colour's lightness (its relative luminance, sRGB-weighted) in
/// [`TONES`] steps: 0 black … `TONES` white.
pub fn tone_of(c: [u8; 4]) -> u8 {
    let l = (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0;
    (l * TONES as f32).round().clamp(0.0, TONES as f32) as u8
}

/// The quarter-pixel bucket of a horizontal position.
pub fn subpixel_of(x: f32) -> u8 {
    (((x - x.floor()) * SUBPIXEL_STEPS).round() as u8) % SUBPIXEL_STEPS as u8
}

/// A rasterized glyph. `left`/`top` place the image relative to the
/// glyph's pen position (whole pixels, the subpixel offset applied);
/// `top` is measured upward from the baseline, as fonts do.
#[derive(Debug)]
pub struct GlyphImage {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// `false`: `data` is a coverage mask, one byte a pixel. `true`:
    /// premultiplied RGBA, four bytes a pixel.
    pub color: bool,
    pub data: Vec<u8>,
}

struct Rasterizer {
    context: ScaleContext,
    /// Each glyph's image (`None`: no ink) and the tick it was last asked for.
    cache: HashMap<GlyphKey, (Option<Arc<GlyphImage>>, u64)>,
    bytes: usize,
    tick: u64,
    evicted: u64,
}

static RASTER: OnceLock<Mutex<Rasterizer>> = OnceLock::new();

/// The most glyph images kept, and their bytes: past either, the least
/// recently asked for quarter goes (it was all of them, at once).
const CACHE_LIMIT: usize = 16384;
const CACHE_BYTES: usize = 64 << 20;

fn rasterizer() -> &'static Mutex<Rasterizer> {
    RASTER.get_or_init(|| {
        Mutex::new(Rasterizer {
            context: ScaleContext::new(),
            cache: HashMap::new(),
            bytes: 0,
            tick: 0,
            evicted: 0,
        })
    })
}

fn image_bytes(i: &Option<Arc<GlyphImage>>) -> usize {
    i.as_ref().map(|i| i.data.len()).unwrap_or(0) + 64
}

impl Rasterizer {
    /// Drop the least recently used quarter of the cache.
    fn evict(&mut self) {
        let mut ticks: Vec<u64> = self.cache.values().map(|v| v.1).collect();
        let cut = ticks.len() / 4;
        if cut == 0 {
            return;
        }
        let (_, nth, _) = ticks.select_nth_unstable(cut);
        let nth = *nth;
        let before = self.cache.len();
        let mut freed = 0;
        self.cache.retain(|_, v| {
            let keep = v.1 > nth;
            if !keep {
                freed += image_bytes(&v.0);
            }
            keep
        });
        self.bytes -= freed;
        self.evicted += (before - self.cache.len()) as u64;
    }
}

/// The image of a glyph, or `None` for one with no ink (a space) or a
/// face that cannot be read.
pub fn glyph(key: &GlyphKey, coords: &[i16]) -> Option<Arc<GlyphImage>> {
    let mut r = rasterizer().lock().ok()?;
    r.tick += 1;
    let now = r.tick;
    if let Some(hit) = r.cache.get_mut(key) {
        hit.1 = now;
        return hit.0.clone();
    }
    if r.cache.len() >= CACHE_LIMIT || r.bytes >= CACHE_BYTES {
        r.evict();
    }
    let image = render(&mut r.context, key, coords).map(Arc::new);
    r.bytes += image_bytes(&image);
    r.cache.insert(key.clone(), (image.clone(), now));
    image
}

/// What the glyph cache holds: `(images, bytes, evicted ever)`.
pub fn stats() -> (usize, usize, u64) {
    match rasterizer().lock() {
        Ok(r) => (r.cache.len(), r.bytes, r.evicted),
        Err(_) => (0, 0, 0),
    }
}

/// The largest glyph drawn, in device pixels: a text at an absurd size (a
/// program's 1e30) has no ink rather than a rasterizer asked for an image
/// past any memory (zeno indexes out of its buffer and the process aborts).
pub const MAX_GLYPH_PX: f32 = 4096.0;

/// How text is darkened to match macOS's own (Core Graphics' font
/// smoothing, which thickens a glyph's stems, the more for light text on
/// a dark ground): the outline grows by `darken` device pixels, and the
/// coverage takes a curve. Tuned against AppKit's NSTextField at 1× and
/// 2×, light and dark.
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// Outline growth for black text and for white text, device pixels
    /// at a 16 px em, and the power of the em it grows with.
    pub grow_dark: f32,
    pub grow_light: f32,
    pub grow_power: f32,
    /// The most an outline grows, device pixels (Core Graphics' stops
    /// growing past a size).
    pub grow_max: f32,
    /// The coverage curve's exponent (`1 - (1 - c)^k`) for black text and
    /// for white text: past 1 a partly covered pixel is filled more.
    pub curve_dark: f32,
    pub curve_light: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            grow_dark: 0.10,
            grow_light: 0.20,
            grow_power: 1.2,
            grow_max: 0.4,
            curve_dark: 1.15,
            curve_light: 1.35,
        }
    }
}

/// The tuning in force: [`Tuning::default`], or `OLANG_GUI_TEXT_DARKEN`
/// (`grow_dark,grow_light,grow_power,curve_dark,curve_light,grow_max`) to tune it,
/// or `OLANG_GUI_TEXT_DARKEN=0` for none.
pub fn tuning() -> Tuning {
    static T: OnceLock<Tuning> = OnceLock::new();
    *T.get_or_init(|| {
        let Ok(v) = std::env::var("OLANG_GUI_TEXT_DARKEN") else {
            return Tuning::default();
        };
        if v.trim() == "0" {
            return Tuning {
                grow_dark: 0.0,
                grow_light: 0.0,
                grow_power: 0.0,
                grow_max: 0.0,
                curve_dark: 1.0,
                curve_light: 1.0,
            };
        }
        let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let d = Tuning::default();
        let at = |i: usize, or: f32| n.get(i).copied().unwrap_or(or);
        Tuning {
            grow_dark: at(0, d.grow_dark),
            grow_light: at(1, d.grow_light),
            grow_power: at(2, d.grow_power),
            curve_dark: at(3, d.curve_dark),
            curve_light: at(4, d.curve_light),
            grow_max: at(5, d.grow_max),
        }
    })
}

fn lerp_tone(dark: f32, light: f32, tone: u8) -> f32 {
    let t = tone.min(TONES) as f32 / TONES as f32;
    dark + (light - dark) * t
}

/// The outline's growth for a glyph `size` device pixels to the em in
/// text of lightness `tone`.
pub fn darken(t: &Tuning, size: f32, tone: u8) -> f32 {
    let g = lerp_tone(t.grow_dark, t.grow_light, tone);
    if g <= 0.0 {
        return 0.0;
    }
    // no more than a twentieth of the em: tiny text stays legible
    (g * (size / 16.0).powf(t.grow_power)).min(t.grow_max).min(size / 20.0)
}

/// The coverage curve for text of lightness `tone`, as a table.
pub fn coverage_curve(t: &Tuning, tone: u8) -> [u8; 256] {
    let k = lerp_tone(t.curve_dark, t.curve_light, tone).max(0.05);
    let mut table = [0u8; 256];
    for (i, v) in table.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = ((1.0 - (1.0 - c).powf(k)) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    table
}

fn render(context: &mut ScaleContext, key: &GlyphKey, coords: &[i16]) -> Option<GlyphImage> {
    let size = key.size as f32 / 64.0;
    if !(size > 0.0 && size <= MAX_GLYPH_PX) {
        return None;
    }
    let data = super::text::face_data(key.face)?;
    let bytes: &[u8] = data.data.data();
    // A COLRv1 glyph (Noto's emoji) is painted from its paint graph; swash
    // would draw only its outline.
    if !key.embolden
        && !key.skew
        && let Some(img) = super::colr::render(
            bytes,
            key.face.index,
            key.glyph,
            size,
            coords,
            key.subpixel as f32 / SUBPIXEL_STEPS,
        )
    {
        return Some(img);
    }
    let font = swash::FontRef::from_index(bytes, key.face.index as usize)?;
    let mut scaler = context
        .builder(font)
        .size(size)
        .hint(false)
        .normalized_coords(coords.iter().copied())
        .build();
    let offset = key.subpixel as f32 / SUBPIXEL_STEPS;
    let mut render = Render::new(&[
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::Outline,
    ]);
    render
        .format(Format::Alpha)
        .offset(Vector::new(offset, 0.0));
    let tuning = tuning();
    let bold = if key.embolden { size / 40.0 } else { 0.0 } + darken(&tuning, size, key.tone);
    if bold > 0.0 {
        render.embolden(bold);
    }
    if key.skew {
        render.transform(Some(swash::zeno::Transform::skew(
            swash::zeno::Angle::from_degrees(14.0),
            swash::zeno::Angle::ZERO,
        )));
    }
    let image = render.render(&mut scaler, key.glyph)?;
    let p = image.placement;
    if p.width == 0 || p.height == 0 {
        return None;
    }
    let color = match image.content {
        Content::Mask => false,
        Content::Color => true,
        // Subpixel masks are not requested; take the green channel if
        // one arrives anyway.
        Content::SubpixelMask => {
            let data = image.data.chunks(4).map(|px| px[1]).collect();
            return Some(GlyphImage {
                left: p.left,
                top: p.top,
                width: p.width,
                height: p.height,
                color: false,
                data,
            });
        }
    };
    let mut data = image.data;
    if !color {
        let curve = coverage_curve(&tuning, key.tone);
        for v in data.iter_mut() {
            *v = curve[*v as usize];
        }
    }
    if color {
        // swash answers straight alpha; both renderers want premultiplied.
        for px in data.chunks_mut(4) {
            let a = px[3] as u32;
            px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
            px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
            px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
        }
    }
    Some(GlyphImage {
        left: p.left,
        top: p.top,
        width: p.width,
        height: p.height,
        color,
        data,
    })
}

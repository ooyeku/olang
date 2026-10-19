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
        }
    }
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
    if key.embolden {
        render.embolden(size / 40.0);
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

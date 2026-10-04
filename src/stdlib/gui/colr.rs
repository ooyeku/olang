//! COLRv1 colour glyphs (Noto Color Emoji, Segoe UI Emoji): skrifa walks
//! the glyph's paint graph and this painter draws it with tiny-skia, as a
//! premultiplied RGBA image placed like any glyph image. swash draws
//! COLRv0 layers and bitmap emoji (sbix, CBDT); a COLRv1 glyph it would
//! draw as its bare outline, or not at all.
//!
//! Supported: transforms, glyph and box clips, solid fills, linear and
//! two-point radial gradients (tiny-skia's), sweep gradients (computed
//! per pixel), and layers with their composite modes.

use super::raster::GlyphImage;
use skrifa::MetadataProvider;
use skrifa::color::{
    Brush, ColorGlyphFormat, ColorPainter, ColorStop, CompositeMode, Extend,
    Transform as CTransform,
};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::types::{F2Dot14, GlyphId};
use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder,
    Pixmap, PixmapPaint, Point, RadialGradient, Rect, Shader, SpreadMode, Transform,
};

/// Largest side of a colour glyph image, in pixels.
const MAX_SIDE: f32 = 512.0;

struct PathPen(PathBuilder);

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, cy0, x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

struct Painter<'a> {
    font: skrifa::FontRef<'a>,
    location: LocationRef<'a>,
    palette: Vec<[u8; 4]>,
    width: u32,
    height: u32,
    /// Font units to the image: the base, then each pushed transform.
    transforms: Vec<Transform>,
    clips: Vec<Option<Mask>>,
    layers: Vec<Pixmap>,
}

fn to_ts(t: &CTransform) -> Transform {
    Transform::from_row(t.xx, t.yx, t.xy, t.yy, t.dx, t.dy)
}

impl Painter<'_> {
    fn transform(&self) -> Transform {
        *self.transforms.last().unwrap_or(&Transform::identity())
    }

    fn clip(&self) -> Option<&Mask> {
        self.clips.last().and_then(|m| m.as_ref())
    }

    fn color(&self, index: u16, alpha: f32) -> Color {
        let [r, g, b, a] = if index == 0xFFFF {
            [0, 0, 0, 255]
        } else {
            *self.palette.get(index as usize).unwrap_or(&[0, 0, 0, 255])
        };
        let a = (a as f32 / 255.0 * alpha).clamp(0.0, 1.0);
        Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
            .unwrap_or(Color::BLACK)
    }

    fn stops(&self, stops: &[ColorStop]) -> Vec<GradientStop> {
        stops
            .iter()
            .map(|s| GradientStop::new(s.offset, self.color(s.palette_index, s.alpha)))
            .collect()
    }

    fn glyph_path(&self, gid: GlyphId) -> Option<Path> {
        let outline = self.font.outline_glyphs().get(gid)?;
        let mut pen = PathPen(PathBuilder::new());
        outline
            .draw(
                DrawSettings::unhinted(Size::unscaled(), self.location),
                &mut pen,
            )
            .ok()?;
        pen.0.finish()
    }

    fn push_mask(&mut self, path: Option<Path>) {
        let mut mask = match self.clip() {
            Some(m) => m.clone(),
            None => {
                let mut m = match Mask::new(self.width, self.height) {
                    Some(m) => m,
                    None => {
                        self.clips.push(None);
                        return;
                    }
                };
                m.data_mut().fill(255);
                m
            }
        };
        match path {
            Some(p) => mask.intersect_path(&p, FillRule::Winding, true, self.transform()),
            None => mask.data_mut().fill(0),
        }
        self.clips.push(Some(mask));
    }

    fn fill_shader(&mut self, shader: Shader<'_>) {
        let paint = Paint {
            shader,
            anti_alias: true,
            ..Paint::default()
        };
        let rect = Rect::from_xywh(0.0, 0.0, self.width as f32, self.height as f32);
        let mask = self.clip().cloned();
        if let (Some(rect), Some(top)) = (rect, self.layers.last_mut()) {
            top.fill_rect(rect, &paint, Transform::identity(), mask.as_ref());
        }
    }

    fn fill_sweep(
        &mut self,
        c0: skrifa::raw::types::Point<f32>,
        start: f32,
        end: f32,
        stops: &[ColorStop],
        extend: Extend,
    ) {
        let Some(inv) = self.transform().invert() else {
            return;
        };
        let Some(mut img) = Pixmap::new(self.width, self.height) else {
            return;
        };
        let resolved: Vec<(f32, [f32; 4])> = stops
            .iter()
            .map(|s| {
                let c = self.color(s.palette_index, s.alpha);
                (
                    s.offset,
                    [
                        c.red() * c.alpha(),
                        c.green() * c.alpha(),
                        c.blue() * c.alpha(),
                        c.alpha(),
                    ],
                )
            })
            .collect();
        if resolved.is_empty() {
            return;
        }
        let span = end - start;
        let w = self.width as usize;
        for (i, px) in img.data_mut().chunks_mut(4).enumerate() {
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let mut p = Point::from_xy(x, y);
            inv.map_point(&mut p);
            // clockwise from the x axis, as skrifa normalizes it (font
            // units are y-up, so the angle's sign flips)
            let mut deg = (-(p.y - c0.y)).atan2(p.x - c0.x).to_degrees();
            deg = (-deg).rem_euclid(360.0);
            let mut t = if span.abs() < 1e-6 {
                0.0
            } else {
                (deg - start) / span
            };
            t = match extend {
                Extend::Repeat => t.rem_euclid(1.0),
                Extend::Reflect => {
                    let r = t.rem_euclid(2.0);
                    if r > 1.0 { 2.0 - r } else { r }
                }
                _ => t.clamp(0.0, 1.0),
            };
            let c = interpolate(&resolved, t);
            px.copy_from_slice(&[
                (c[0] * 255.0) as u8,
                (c[1] * 255.0) as u8,
                (c[2] * 255.0) as u8,
                (c[3] * 255.0) as u8,
            ]);
        }
        let mask = self.clip().cloned();
        if let Some(top) = self.layers.last_mut() {
            top.draw_pixmap(
                0,
                0,
                img.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                mask.as_ref(),
            );
        }
    }
}

fn interpolate(stops: &[(f32, [f32; 4])], t: f32) -> [f32; 4] {
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            let f = if b.0 - a.0 < 1e-6 {
                0.0
            } else {
                (t - a.0) / (b.0 - a.0)
            };
            return std::array::from_fn(|i| a.1[i] + (b.1[i] - a.1[i]) * f);
        }
    }
    stops[stops.len() - 1].1
}

fn spread(e: Extend) -> SpreadMode {
    match e {
        Extend::Repeat => SpreadMode::Repeat,
        Extend::Reflect => SpreadMode::Reflect,
        _ => SpreadMode::Pad,
    }
}

fn blend(mode: CompositeMode) -> BlendMode {
    match mode {
        CompositeMode::Clear => BlendMode::Clear,
        CompositeMode::Src => BlendMode::Source,
        CompositeMode::Dest => BlendMode::Destination,
        CompositeMode::DestOver => BlendMode::DestinationOver,
        CompositeMode::SrcIn => BlendMode::SourceIn,
        CompositeMode::DestIn => BlendMode::DestinationIn,
        CompositeMode::SrcOut => BlendMode::SourceOut,
        CompositeMode::DestOut => BlendMode::DestinationOut,
        CompositeMode::SrcAtop => BlendMode::SourceAtop,
        CompositeMode::DestAtop => BlendMode::DestinationAtop,
        CompositeMode::Xor => BlendMode::Xor,
        CompositeMode::Plus => BlendMode::Plus,
        CompositeMode::Screen => BlendMode::Screen,
        CompositeMode::Overlay => BlendMode::Overlay,
        CompositeMode::Darken => BlendMode::Darken,
        CompositeMode::Lighten => BlendMode::Lighten,
        CompositeMode::ColorDodge => BlendMode::ColorDodge,
        CompositeMode::ColorBurn => BlendMode::ColorBurn,
        CompositeMode::HardLight => BlendMode::HardLight,
        CompositeMode::SoftLight => BlendMode::SoftLight,
        CompositeMode::Difference => BlendMode::Difference,
        CompositeMode::Exclusion => BlendMode::Exclusion,
        CompositeMode::Multiply => BlendMode::Multiply,
        CompositeMode::HslHue => BlendMode::Hue,
        CompositeMode::HslSaturation => BlendMode::Saturation,
        CompositeMode::HslColor => BlendMode::Color,
        CompositeMode::HslLuminosity => BlendMode::Luminosity,
        _ => BlendMode::SourceOver,
    }
}

impl ColorPainter for Painter<'_> {
    fn push_transform(&mut self, t: CTransform) {
        let next = self.transform().pre_concat(to_ts(&t));
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let path = self.glyph_path(glyph_id);
        self.push_mask(path);
    }

    fn push_clip_box(&mut self, b: skrifa::metrics::BoundingBox) {
        let path = Rect::from_ltrb(b.x_min, b.y_min, b.x_max, b.y_max).map(PathBuilder::from_rect);
        self.push_mask(path);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        let ts = self.transform();
        match brush {
            Brush::Solid {
                palette_index,
                alpha,
            } => {
                let c = self.color(palette_index, alpha);
                self.fill_shader(Shader::SolidColor(c));
            }
            Brush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend,
            } => {
                let shader = LinearGradient::new(
                    Point::from_xy(p0.x, p0.y),
                    Point::from_xy(p1.x, p1.y),
                    self.stops(color_stops),
                    spread(extend),
                    ts,
                );
                // a degenerate gradient draws its first colour
                let fallback = color_stops
                    .first()
                    .map(|c| Shader::SolidColor(self.color(c.palette_index, c.alpha)));
                if let Some(s) = shader.or(fallback) {
                    self.fill_shader(s);
                }
            }
            Brush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend,
            } => {
                let shader = RadialGradient::new(
                    Point::from_xy(c0.x, c0.y),
                    r0.max(0.0),
                    Point::from_xy(c1.x, c1.y),
                    r1.max(0.0),
                    self.stops(color_stops),
                    spread(extend),
                    ts,
                );
                let fallback = color_stops
                    .last()
                    .map(|c| Shader::SolidColor(self.color(c.palette_index, c.alpha)));
                if let Some(s) = shader.or(fallback) {
                    self.fill_shader(s);
                }
            }
            Brush::SweepGradient {
                c0,
                start_angle,
                end_angle,
                color_stops,
                extend,
            } => {
                self.fill_sweep(c0, start_angle, end_angle, color_stops, extend);
            }
        }
    }

    fn push_layer(&mut self, _mode: CompositeMode) {
        if let Some(p) = Pixmap::new(self.width, self.height) {
            self.layers.push(p);
        }
    }

    fn pop_layer_with_mode(&mut self, mode: CompositeMode) {
        if self.layers.len() < 2 {
            return;
        }
        let top = self.layers.pop().expect("a layer");
        let paint = PixmapPaint {
            blend_mode: blend(mode),
            ..PixmapPaint::default()
        };
        if let Some(under) = self.layers.last_mut() {
            under.draw_pixmap(0, 0, top.as_ref(), &paint, Transform::identity(), None);
        }
    }
}

/// The image of COLRv1 glyph `glyph` of the face at `index` in `bytes`, at
/// `size` pixels, its normalized variation `coords`, shifted right by
/// `offset` pixels; `None` when the glyph has no COLRv1 paint.
pub fn render(
    bytes: &[u8],
    index: u32,
    glyph: u16,
    size: f32,
    coords: &[i16],
    offset: f32,
) -> Option<GlyphImage> {
    let font = skrifa::FontRef::from_index(bytes, index).ok()?;
    let gid = GlyphId::new(glyph as u32);
    let cg = font
        .color_glyphs()
        .get_with_format(gid, ColorGlyphFormat::ColrV1)?;
    let coords: Vec<F2Dot14> = coords.iter().map(|c| F2Dot14::from_bits(*c)).collect();
    let location = LocationRef::new(&coords);
    let upem = font.head().ok()?.units_per_em() as f32;
    let scale = size / upem.max(1.0);
    // the clip box, else the em square from the descender to the ascender
    let (x0, y0, x1, y1) = match cg.bounding_box(location, Size::new(size)) {
        Some(b) => (b.x_min, b.y_min, b.x_max, b.y_max),
        None => {
            let m = font.metrics(Size::new(size), location);
            (0.0, m.descent, size, m.ascent)
        }
    };
    let left = (x0 + offset).floor();
    let top = y1.ceil();
    let width = ((x1 + offset).ceil() - left).clamp(1.0, MAX_SIDE) as u32;
    let height = (top - y0.floor()).clamp(1.0, MAX_SIDE) as u32;
    let palette = font
        .color_palettes()
        .get(0)
        .map(|p| {
            p.colors()
                .iter()
                .map(|c| [c.red, c.green, c.blue, c.alpha])
                .collect()
        })
        .unwrap_or_default();
    let base = Transform::from_row(scale, 0.0, 0.0, -scale, offset - left, top);
    let mut painter = Painter {
        font: font.clone(),
        location,
        palette,
        width,
        height,
        transforms: vec![base],
        clips: Vec::new(),
        layers: vec![Pixmap::new(width, height)?],
    };
    cg.paint(location, &mut painter).ok()?;
    let out = painter.layers.into_iter().next()?;
    if out.data().chunks(4).all(|p| p[3] == 0) {
        return None;
    }
    Some(GlyphImage {
        left: left as i32,
        top: top as i32,
        width,
        height,
        color: true,
        data: out.take(),
    })
}

//! The software renderer: a display list drawn into a pixel buffer with
//! tiny-skia. It is the reference the GPU renderer is checked against,
//! what every headless window and every test draws with, and the
//! fallback where no GPU adapter is usable.
//!
//! Blending is in sRGB-encoded space, as tiny-skia does and as the GPU
//! renderer matches by drawing to a non-sRGB target.

use super::raster;
use super::text::Color;
use super::window::{Clip, DisplayList, Prim};
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Rect, Transform};

/// Draw a display list. The pixmap is premultiplied RGBA.
pub fn render(dl: &DisplayList) -> Pixmap {
    let mut pm = Pixmap::new(dl.width.max(1), dl.height.max(1)).expect("non-zero size");
    let c = dl.clear;
    pm.fill(tiny_skia::Color::from_rgba8(c[0], c[1], c[2], c[3]));
    let mut masks: Vec<(Clip, Mask)> = Vec::new();
    for prim in &dl.prims {
        match prim {
            Prim::Rect {
                x,
                y,
                w,
                h,
                fill,
                border,
                border_width,
                radii,
                clip,
            } => {
                if clip.is_empty() || *w <= 0.0 || *h <= 0.0 {
                    continue;
                }
                let mask = clip_mask(&mut masks, *clip, dl.width, dl.height);
                draw_rect(
                    &mut pm,
                    [*x, *y, *w, *h],
                    *fill,
                    *border,
                    *border_width,
                    *radii,
                    mask,
                );
            }
            Prim::Glyph {
                x,
                y,
                key,
                coords,
                color,
                clip,
            } => {
                if clip.is_empty() {
                    continue;
                }
                if let Some(img) = raster::glyph(key, coords) {
                    let mask = if clip.is_rounded() {
                        clip_mask(&mut masks, *clip, dl.width, dl.height).cloned()
                    } else {
                        None
                    };
                    blit_glyph(&mut pm, *x, *y, &img, *color, *clip, mask.as_ref());
                }
            }
        }
    }
    pm
}

/// A mask for a clip rectangle, or `None` when the clip is the whole
/// pixmap. Masks are kept per distinct clip for the frame.
fn clip_mask(masks: &mut Vec<(Clip, Mask)>, clip: Clip, w: u32, h: u32) -> Option<&Mask> {
    if clip.x0 <= 0.0 && clip.y0 <= 0.0 && clip.x1 >= w as f32 && clip.y1 >= h as f32 {
        return None;
    }
    let at = match masks.iter().position(|(c, _)| *c == clip) {
        Some(i) => i,
        None => {
            let mut m = Mask::new(w.max(1), h.max(1))?;
            if clip.is_rounded() {
                if let Some(path) = rrect_path(
                    clip.x0,
                    clip.y0,
                    clip.x1 - clip.x0,
                    clip.y1 - clip.y0,
                    clip.radii,
                ) {
                    m.fill_path(&path, FillRule::Winding, true, Transform::identity());
                }
            } else if let Some(r) = Rect::from_ltrb(clip.x0, clip.y0, clip.x1, clip.y1) {
                let path = PathBuilder::from_rect(r);
                m.fill_path(&path, FillRule::Winding, false, Transform::identity());
            }
            masks.push((clip, m));
            masks.len() - 1
        }
    };
    Some(&masks[at].1)
}

fn paint_of(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

/// A rounded rectangle's path; radii top-left, top-right, bottom-right,
/// bottom-left, each clamped to half the shorter side.
pub fn rrect_path(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) -> Option<tiny_skia::Path> {
    let max = (w.min(h) / 2.0).max(0.0);
    let [tl, tr, br, bl] = radii.map(|r| r.clamp(0.0, max));
    if tl == 0.0 && tr == 0.0 && br == 0.0 && bl == 0.0 {
        return Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect);
    }
    // The cubic Bézier approximation of a quarter circle.
    const K: f32 = 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
    pb.line_to(x + w - tr, y);
    pb.cubic_to(
        x + w - tr + tr * K,
        y,
        x + w,
        y + tr - tr * K,
        x + w,
        y + tr,
    );
    pb.line_to(x + w, y + h - br);
    pb.cubic_to(
        x + w,
        y + h - br + br * K,
        x + w - br + br * K,
        y + h,
        x + w - br,
        y + h,
    );
    pb.line_to(x + bl, y + h);
    pb.cubic_to(
        x + bl - bl * K,
        y + h,
        x,
        y + h - bl + bl * K,
        x,
        y + h - bl,
    );
    pb.line_to(x, y + tl);
    pb.cubic_to(x, y + tl - tl * K, x + tl - tl * K, y, x + tl, y);
    pb.close();
    pb.finish()
}

fn draw_rect(
    pm: &mut Pixmap,
    r: [f32; 4],
    fill: Color,
    border: Color,
    bw: f32,
    radii: [f32; 4],
    mask: Option<&Mask>,
) {
    let [x, y, w, h] = r;
    if fill[3] > 0
        && let Some(p) = rrect_path(x, y, w, h, radii)
    {
        pm.fill_path(
            &p,
            &paint_of(fill),
            FillRule::Winding,
            Transform::identity(),
            mask,
        );
    }
    if bw > 0.0 && border[3] > 0 {
        // The border is the ring between the outer shape and the shape
        // inset by its width, so it lies inside the box, as CSS's does.
        let inner_r = radii.map(|r| (r - bw).max(0.0));
        let mut pb = PathBuilder::new();
        if let Some(outer) = rrect_path(x, y, w, h, radii) {
            pb.push_path(&outer);
        }
        if w > 2.0 * bw
            && h > 2.0 * bw
            && let Some(inner) = rrect_path(x + bw, y + bw, w - 2.0 * bw, h - 2.0 * bw, inner_r)
        {
            pb.push_path(&inner);
        }
        if let Some(ring) = pb.finish() {
            pm.fill_path(
                &ring,
                &paint_of(border),
                FillRule::EvenOdd,
                Transform::identity(),
                mask,
            );
        }
    }
}

/// Composite a glyph image at a pen position (premultiplied source-over).
fn blit_glyph(
    pm: &mut Pixmap,
    x: f32,
    y: f32,
    img: &raster::GlyphImage,
    color: Color,
    clip: Clip,
    mask: Option<&Mask>,
) {
    let gx = x.floor() as i32 + img.left;
    let gy = y as i32 - img.top;
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let cx0 = (clip.x0.floor() as i32).max(0);
    let cy0 = (clip.y0.floor() as i32).max(0);
    let cx1 = (clip.x1.ceil() as i32).min(pw);
    let cy1 = (clip.y1.ceil() as i32).min(ph);
    let mask_data = mask.map(|m| m.data().to_vec());
    let data = pm.data_mut();
    let ca = color[3] as u32;
    for row in 0..img.height as i32 {
        let py = gy + row;
        if py < cy0 || py >= cy1 {
            continue;
        }
        for col in 0..img.width as i32 {
            let px = gx + col;
            if px < cx0 || px >= cx1 {
                continue;
            }
            let (sr, sg, sb, sa) = if img.color {
                let i = ((row as u32 * img.width + col as u32) * 4) as usize;
                // Colour glyphs keep their own colours; the text colour's
                // alpha still fades them.
                let f = |v: u8| (v as u32 * ca + 127) / 255;
                (
                    f(img.data[i]),
                    f(img.data[i + 1]),
                    f(img.data[i + 2]),
                    f(img.data[i + 3]),
                )
            } else {
                let cov = img.data[(row as u32 * img.width + col as u32) as usize] as u32;
                let a = (cov * ca + 127) / 255;
                (
                    (color[0] as u32 * a + 127) / 255,
                    (color[1] as u32 * a + 127) / 255,
                    (color[2] as u32 * a + 127) / 255,
                    a,
                )
            };
            let (sr, sg, sb, sa) = match &mask_data {
                Some(md) => {
                    let m = md[(py * pw + px) as usize] as u32;
                    let f = |v: u32| (v * m + 127) / 255;
                    (f(sr), f(sg), f(sb), f(sa))
                }
                None => (sr, sg, sb, sa),
            };
            if sa == 0 {
                continue;
            }
            let i = ((py * pw + px) * 4) as usize;
            let inv = 255 - sa;
            data[i] = (sr + (data[i] as u32 * inv + 127) / 255).min(255) as u8;
            data[i + 1] = (sg + (data[i + 1] as u32 * inv + 127) / 255).min(255) as u8;
            data[i + 2] = (sb + (data[i + 2] as u32 * inv + 127) / 255).min(255) as u8;
            data[i + 3] = (sa + (data[i + 3] as u32 * inv + 127) / 255).min(255) as u8;
        }
    }
}

/// Straight RGBA bytes of a pixmap (for PNG and for comparing).
pub fn rgba(pm: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.data().len());
    for px in pm.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

//! The software renderer: a display list drawn into a pixel buffer with
//! tiny-skia. It is the reference the GPU renderer is checked against,
//! what every headless window and every test draws with, and the
//! fallback where no GPU adapter is usable.
//!
//! Blending is in sRGB-encoded space, as tiny-skia does and as the GPU
//! renderer matches by drawing to a non-sRGB target.
//!
//! A window drawn in software keeps its last frame ([`Retained`]) and
//! redraws only the rectangles [`damage`](super::damage) finds changed.

use super::damage::{self, Damage, IRect, Item, Shift, Tracker};
use super::raster;
use super::text::Color;
use super::window::{Clip, DisplayList, Prim};
use std::hash::{Hash, Hasher};
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Rect, Transform};

/// Draw a display list. The pixmap is premultiplied RGBA.
pub fn render(dl: &DisplayList) -> Pixmap {
    let mut pm = Pixmap::new(dl.width.max(1), dl.height.max(1)).expect("non-zero size");
    let c = dl.clear;
    pm.fill(tiny_skia::Color::from_rgba8(c[0], c[1], c[2], c[3]));
    draw(&mut pm, dl, None);
    pm
}

/// A frame kept between frames, so the next draws only what changed.
#[derive(Default)]
pub struct Retained {
    pm: Option<Pixmap>,
    /// Where a damaged frame's primitives are drawn whole before their
    /// rectangles are copied into `pm` (a primitive drawn through a mask
    /// is not always the same pixels as one drawn without).
    scratch: Option<Pixmap>,
    tracker: Tracker,
    /// The last frame's damage, and the moves before it.
    pub last: Option<Damage>,
    pub last_shifts: Vec<Shift>,
}

impl Retained {
    /// The next frame draws everything.
    pub fn invalidate(&mut self) {
        self.tracker.invalidate();
    }

    /// The kept frame.
    pub fn pixmap(&self) -> Option<&Pixmap> {
        self.pm.as_ref()
    }

    /// Draw a display list over the kept frame: its damage.
    pub fn render(&mut self, dl: &DisplayList) -> &Damage {
        let (w, h) = (dl.width.max(1), dl.height.max(1));
        let items = items(dl);
        if !matches!(&self.pm, Some(p) if (p.width(), p.height()) == (w, h)) {
            self.pm = Pixmap::new(w, h);
            self.tracker.invalidate();
        }
        let (d, shifts, next) = self.tracker.damage(w, h, dl.clear, &items, &dl.scrolls);
        if let Some(pm) = self.pm.as_mut() {
            // the scrolled regions' pixels move with them first
            let pw = w as usize * 4;
            let data = pm.data_mut();
            for s in &shifts {
                let (x0, x1) = (s.src.x0 as usize * 4, s.src.x1 as usize * 4);
                let dx = (s.dst.x0 - s.src.x0) as isize * 4;
                let rows: Vec<i32> = if s.dst.y0 > s.src.y0 {
                    (s.src.y0..s.src.y1).rev().collect()
                } else {
                    (s.src.y0..s.src.y1).collect()
                };
                for y in rows {
                    let from = y as usize * pw;
                    let to = (y + s.dst.y0 - s.src.y0) as usize * pw;
                    data.copy_within(
                        from + x0..from + x1,
                        (to as isize + x0 as isize + dx) as usize,
                    );
                }
            }
            match &d {
                Damage::None => {}
                Damage::Full => {
                    let c = dl.clear;
                    pm.fill(tiny_skia::Color::from_rgba8(c[0], c[1], c[2], c[3]));
                    draw(pm, dl, None);
                }
                Damage::Rects(rects) => {
                    if !matches!(&self.scratch, Some(p) if (p.width(), p.height()) == (w, h)) {
                        self.scratch = Pixmap::new(w, h);
                    }
                    if let Some(sc) = self.scratch.as_mut() {
                        let c = tiny_skia::Color::from_rgba8(dl.clear[0], dl.clear[1], dl.clear[2], dl.clear[3])
                            .premultiply()
                            .to_color_u8();
                        let px = [c.red(), c.green(), c.blue(), c.alpha()];
                        let pw = w as usize;
                        let data = sc.data_mut();
                        for r in rects {
                            for y in r.y0..r.y1 {
                                let row = y as usize * pw;
                                for x in r.x0..r.x1 {
                                    let i = (row + x as usize) * 4;
                                    data[i..i + 4].copy_from_slice(&px);
                                }
                            }
                        }
                        draw(sc, dl, Some((rects, &items)));
                        let (src, dst) = (sc.data(), pm.data_mut());
                        for r in rects {
                            for y in r.y0..r.y1 {
                                let a = (y as usize * pw + r.x0 as usize) * 4;
                                let b = (y as usize * pw + r.x1 as usize) * 4;
                                dst[a..b].copy_from_slice(&src[a..b]);
                            }
                        }
                    }
                }
            }
        }
        damage::count(&d, w, h);
        self.tracker.commit(next);
        self.last = Some(d);
        self.last_shifts = shifts;
        self.last.as_ref().unwrap_or(&Damage::None)
    }

    /// What the last frame changed on the screen: its damage and the
    /// places its moves wrote (nothing: it is not presented).
    pub fn presented(&self) -> Damage {
        let d = self.last.clone().unwrap_or(Damage::None);
        if self.last_shifts.is_empty() {
            return d;
        }
        match d {
            Damage::Full => Damage::Full,
            Damage::None => Damage::Rects(self.last_shifts.iter().map(|s| s.dst).collect()),
            Damage::Rects(mut r) => {
                r.extend(self.last_shifts.iter().map(|s| s.dst));
                Damage::Rects(r)
            }
        }
    }
}

/// What damage reads of each primitive: the pixels it can touch, and
/// hashes of what decides them (empty bounds: it draws nothing; items
/// stay aligned with `dl.prims`).
fn items(dl: &DisplayList) -> Vec<Item> {
    let (ww, wh) = (dl.width.max(1), dl.height.max(1));
    let nothing = |c: &Clip| Item::new(IRect::new(0, 0, 0, 0), 0, 0.0, 0.0, *c);
    let mut out = Vec::with_capacity(dl.prims.len());
    for prim in &dl.prims {
        let item = match prim {
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
                    None
                } else {
                    let mut hs = damage::mix_f32s(1, &[*w, *h, *border_width]);
                    hs = damage::mix_f32s(hs, radii);
                    hs = damage::mix(hs, u32::from_le_bytes(*fill) as u64);
                    hs = damage::mix(hs, u32::from_le_bytes(*border) as u64);
                    let r = radii.iter().fold(*border_width, |a, v| a.max(*v));
                    damage::bounds(*x, *y, *w, *h, 1.0, clip, ww, wh).map(|b| {
                        Item::new(b, hs, *x, *y, *clip)
                            .with_interior(damage::inside(*x, *y, *w, *h, r + 1.0))
                            .pinned(damage::crosses_window(*x, *y, *w, *h, 0.0, ww, wh))
                    })
                }
            }
            Prim::Image {
                x,
                y,
                w,
                h,
                pic,
                clip,
                alpha,
            } => {
                if clip.is_empty() || *w <= 0.0 || *h <= 0.0 || *alpha <= 0.0 {
                    None
                } else {
                    let hs = damage::mix_f32s(damage::mix(2, pic.id), &[*w, *h, *alpha]);
                    damage::bounds(*x, *y, *w, *h, 1.0, clip, ww, wh).map(|b| {
                        Item::new(b, hs, *x, *y, *clip)
                            .pinned(damage::crosses_window(*x, *y, *w, *h, 0.0, ww, wh))
                    })
                }
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
                    None
                } else {
                    raster::glyph(key, coords).and_then(|img| {
                        let gx = x.floor() as i32 + img.left;
                        let gy = *y as i32 - img.top;
                        let mut k = rustc_hash::FxHasher::default();
                        key.hash(&mut k);
                        let hs = damage::mix(damage::mix(3, k.finish()), u32::from_le_bytes(*color) as u64);
                        damage::bounds(
                            gx as f32,
                            gy as f32,
                            img.width as f32,
                            img.height as f32,
                            0.0,
                            clip,
                            ww,
                            wh,
                        )
                        .map(|b| Item::new(b, hs, *x, *y, *clip))
                    })
                }
            }
        };
        out.push(item.unwrap_or_else(|| {
            nothing(match prim {
                Prim::Rect { clip, .. } | Prim::Image { clip, .. } | Prim::Glyph { clip, .. } => clip,
            })
        }));
    }
    out
}

/// Draw a display list's primitives over a pixmap; with `region`, only
/// those that touch its rectangles (`items`, aligned with the primitives,
/// says which), drawn whole (glyphs only within the rectangles): the
/// pixels inside the rectangles are the whole frame's, those outside are
/// not to be kept.
fn draw(pm: &mut Pixmap, dl: &DisplayList, region: Option<(&[IRect], &[Item])>) {
    let mut masks: Vec<(Clip, Mask)> = Vec::new();
    for (i, prim) in dl.prims.iter().enumerate() {
        if let Some((rects, items)) = region {
            let b = &items[i].bounds;
            if b.is_empty() || !rects.iter().any(|r| r.intersects(b)) {
                continue;
            }
        }
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
                    pm,
                    [*x, *y, *w, *h],
                    *fill,
                    *border,
                    *border_width,
                    *radii,
                    mask,
                );
            }
            Prim::Image {
                x,
                y,
                w,
                h,
                pic,
                clip,
                alpha,
            } => {
                if clip.is_empty() || *w <= 0.0 || *h <= 0.0 || *alpha <= 0.0 {
                    continue;
                }
                let Some(src) = tiny_skia::PixmapRef::from_bytes(&pic.rgba, pic.width, pic.height)
                else {
                    continue;
                };
                let mask = clip_mask(&mut masks, *clip, dl.width, dl.height);
                let paint = tiny_skia::PixmapPaint {
                    quality: tiny_skia::FilterQuality::Bilinear,
                    opacity: alpha.clamp(0.0, 1.0),
                    ..Default::default()
                };
                let tf = Transform::from_row(
                    *w / pic.width as f32,
                    0.0,
                    0.0,
                    *h / pic.height as f32,
                    *x,
                    *y,
                );
                pm.draw_pixmap(0, 0, src, &paint, tf, mask);
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
                    // a plain clip is the glyph's bounds; a rounded one a mask
                    let mask = if clip.is_rounded() {
                        clip_mask(&mut masks, *clip, dl.width, dl.height)
                    } else {
                        None
                    };
                    match region {
                        None => blit_glyph(pm, *x, *y, &img, *color, *clip, mask, None),
                        Some((rects, _)) => {
                            for r in rects {
                                blit_glyph(pm, *x, *y, &img, *color, *clip, mask, Some(r));
                            }
                        }
                    }
                }
            }
        }
    }
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

/// Composite a glyph image at a pen position (premultiplied source-over),
/// within `area` when given.
#[allow(clippy::too_many_arguments)]
fn blit_glyph(
    pm: &mut Pixmap,
    x: f32,
    y: f32,
    img: &raster::GlyphImage,
    color: Color,
    clip: Clip,
    mask: Option<&Mask>,
    area: Option<&IRect>,
) {
    let gx = x.floor() as i32 + img.left;
    let gy = y as i32 - img.top;
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let mut cx0 = (clip.x0.floor() as i32).max(0);
    let mut cy0 = (clip.y0.floor() as i32).max(0);
    let mut cx1 = (clip.x1.ceil() as i32).min(pw);
    let mut cy1 = (clip.y1.ceil() as i32).min(ph);
    if let Some(a) = area {
        cx0 = cx0.max(a.x0);
        cy0 = cy0.max(a.y0);
        cx1 = cx1.min(a.x1);
        cy1 = cy1.min(a.y1);
        if cx0 >= cx1 || cy0 >= cy1 || gx >= cx1 || gy >= cy1 || gx + img.width as i32 <= cx0 || gy + img.height as i32 <= cy0 {
            return;
        }
    }
    let mask_data = mask.map(|m| m.data());
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

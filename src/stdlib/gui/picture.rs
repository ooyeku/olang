//! Pictures from files: an `image` node's source decoded — PNG, JPEG,
//! WebP, GIF (its first frame), and SVG — into premultiplied RGBA both
//! renderers draw.
//!
//! **Sizes.** A picture is kept at the size it is drawn at, not always
//! its own: a photo shown as a thumbnail is decoded once and kept
//! scaled down to the next power of two above the thumbnail's size in
//! device pixels, so a page of thumbnails holds thumbnails' worth of
//! memory. Shown at its own size (`fit: "none"`), it is kept whole (to
//! the largest texture side, 8192). An SVG is drawn at the size it is
//! shown, so it is sharp at every scale.
//!
//! **Off the window's thread.** A real window asks with its id: a
//! picture not held yet is decoded on one of two decoding threads, the
//! window draws without it, and the window redraws when it arrives. A
//! headless window decodes at once, so a test's pixels are whole.
//!
//! **The cache** is by source and size: a path with its length and
//! modification time (a file written again is read again), or the
//! content of Bytes. It keeps at most [`BUDGET`] bytes of pixels,
//! dropping the least recently drawn.

use super::canvas::Picture;
use super::values::Res;
use crate::ast::Value;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use tiny_skia::{Pixmap, PixmapPaint, Transform};

/// The most pixels a decoded picture may have (a decompression bomb is
/// refused, not allocated).
pub const MAX_PIXELS: u64 = 100_000_000;
/// The largest side a picture is kept at: a GPU texture's limit.
pub const MAX_SIDE: u32 = 8192;
/// The smallest side a scaled-down picture is kept at.
const MIN_BUCKET: u32 = 64;
/// The bytes of pixels the cache keeps.
pub const BUDGET: usize = 384 << 20;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Format {
    Png,
    Jpeg,
    Webp,
    Gif,
    Svg,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpeg",
            Format::Webp => "webp",
            Format::Gif => "gif",
            Format::Svg => "svg",
        }
    }
}

/// A picture's own size (pixels; an SVG's in its own units, CSS
/// pixels) and format. A JPEG's is as it shows, its EXIF orientation
/// applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub format: Format,
}

/// What the format is, by the first bytes.
pub fn sniff(b: &[u8]) -> Option<Format> {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(Format::Png);
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Format::Jpeg);
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some(Format::Gif);
    }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some(Format::Webp);
    }
    // XML: an optional BOM and whitespace, then a declaration, a comment,
    // a doctype, or the element itself, and an <svg> soon after.
    let head = &b[..b.len().min(4096)];
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head);
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start();
    if (t.starts_with("<svg") || t.starts_with("<?xml") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE"))
        && text.contains("<svg")
    {
        return Some(Format::Svg);
    }
    None
}

// ── reading the source ───────────────────────────────────────────────

/// The bytes of a source, and its identity: a path's (with its length
/// and modification time), or the content's hash.
enum Source {
    Path(String),
    Bytes(Arc<Vec<u8>>),
}

impl Source {
    fn read(&self) -> Res<Arc<Vec<u8>>> {
        match self {
            Source::Path(p) => std::fs::read(p)
                .map(Arc::new)
                .map_err(|e| format!("image: reading {p}: {e}")),
            Source::Bytes(b) => Ok(b.clone()),
        }
    }
}

fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

/// Bytes values seen, by where their content lives: hashing a photo's
/// megabytes each frame would cost more than drawing it. Checked by
/// length and both ends, so an address reused for other bytes misses.
fn bytes_ids() -> &'static Mutex<HashMap<(usize, usize), (u64, [u8; 32])>> {
    static IDS: OnceLock<Mutex<HashMap<(usize, usize), (u64, [u8; 32])>>> = OnceLock::new();
    IDS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn ends(b: &[u8]) -> [u8; 32] {
    let mut e = [0u8; 32];
    let n = b.len().min(16);
    e[..n].copy_from_slice(&b[..n]);
    e[16..16 + n].copy_from_slice(&b[b.len() - n..]);
    e
}

/// A source as given: a path, or Bytes borrowed from the value.
enum SrcRef<'a> {
    Path(String),
    Bytes(&'a [u8]),
}

impl SrcRef<'_> {
    /// Owned, for a decoding thread (Bytes are copied only here).
    fn owned(&self) -> Source {
        match self {
            SrcRef::Path(p) => Source::Path(p.clone()),
            SrcRef::Bytes(b) => Source::Bytes(Arc::new(b.to_vec())),
        }
    }
}

/// The identity of a source (a path, or Bytes) and the source.
fn identify(src: &Value) -> Res<(u64, SrcRef<'_>)> {
    match src {
        Value::String(p) => {
            let meta = std::fs::metadata(p.as_str()).map_err(|e| format!("image: {p}: {e}"))?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let mut h = std::collections::hash_map::DefaultHasher::new();
            ("path", p.as_str(), meta.len(), mtime).hash(&mut h);
            Ok((h.finish(), SrcRef::Path(p.to_string())))
        }
        other => {
            let b = crate::stdlib::bytes::bytes_of(other)
                .map_err(|_| "image: the source is a path or Bytes".to_string())?;
            let at = (b.as_ptr() as usize, b.len());
            let e = ends(b);
            if let Some((id, seen)) = bytes_ids().lock().ok().and_then(|m| m.get(&at).copied())
                && seen == e
            {
                return Ok((id, SrcRef::Bytes(b)));
            }
            let id = hash_bytes(b);
            if let Ok(mut m) = bytes_ids().lock() {
                if m.len() > 1024 {
                    m.clear();
                }
                m.insert(at, (id, e));
            }
            Ok((id, SrcRef::Bytes(b)))
        }
    }
}

// ── decoding ─────────────────────────────────────────────────────────

/// A JPEG's EXIF orientation (1 to 8; 1 when it says none).
fn jpeg_orientation(b: &[u8]) -> u8 {
    let mut i = 2;
    while i + 4 <= b.len() && b[i] == 0xFF {
        let marker = b[i + 1];
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if marker == 0xDA || len < 2 {
            break;
        }
        let seg = &b[(i + 4).min(b.len())..(i + 2 + len).min(b.len())];
        if marker == 0xE1 && seg.starts_with(b"Exif\0\0") {
            return exif_orientation(&seg[6..]).unwrap_or(1);
        }
        i += 2 + len;
    }
    1
}

fn exif_orientation(t: &[u8]) -> Option<u8> {
    if t.len() < 8 {
        return None;
    }
    let le = &t[0..2] == b"II";
    let u16_at = |o: usize| -> Option<u16> {
        let s = t.get(o..o + 2)?;
        Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let s = t.get(o..o + 4)?;
        let a = [s[0], s[1], s[2], s[3]];
        Some(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    };
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    for k in 0..n.min(64) {
        let e = ifd + 2 + k * 12;
        if u16_at(e)? == 0x0112 {
            let v = u16_at(e + 8)?;
            return (1..=8).contains(&v).then_some(v as u8);
        }
    }
    None
}

/// An SVG parsed: text is not drawn (no fonts are loaded for it).
fn svg_tree(b: &[u8]) -> Res<resvg::usvg::Tree> {
    resvg::usvg::Tree::from_data(b, &resvg::usvg::Options::default())
        .map_err(|e| format!("image: not an SVG this can draw ({e})"))
}

fn check_size(w: u32, h: u32) -> Res<()> {
    if w == 0 || h == 0 {
        return Err("image: it has no pixels".into());
    }
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(format!(
            "image: {w}×{h} is more pixels than a picture may have ({MAX_PIXELS})"
        ));
    }
    Ok(())
}

/// A picture's size and format from its header, without its pixels.
pub fn info_of(b: &[u8]) -> Res<Info> {
    let format = sniff(b).ok_or("image: not a PNG, JPEG, WebP, GIF, or SVG")?;
    let (width, height) = match format {
        Format::Png => {
            if b.len() < 24 {
                return Err("image: a PNG cut short".into());
            }
            (
                u32::from_be_bytes([b[16], b[17], b[18], b[19]]),
                u32::from_be_bytes([b[20], b[21], b[22], b[23]]),
            )
        }
        Format::Jpeg => {
            let mut d = zune_jpeg::JpegDecoder::new(zune_jpeg::zune_core::bytestream::ZCursor::new(b));
            d.decode_headers()
                .map_err(|e| format!("image: not a JPEG this can read ({e:?})"))?;
            let (w, h) = d
                .dimensions()
                .ok_or("image: a JPEG with no size")?;
            if jpeg_orientation(b) >= 5 {
                (h as u32, w as u32)
            } else {
                (w as u32, h as u32)
            }
        }
        Format::Webp => {
            let d = image_webp::WebPDecoder::new(std::io::Cursor::new(b))
                .map_err(|e| format!("image: not a WebP this can read ({e})"))?;
            d.dimensions()
        }
        Format::Gif => {
            if b.len() < 10 {
                return Err("image: a GIF cut short".into());
            }
            (
                u16::from_le_bytes([b[6], b[7]]) as u32,
                u16::from_le_bytes([b[8], b[9]]) as u32,
            )
        }
        Format::Svg => {
            let t = svg_tree(b)?;
            let s = t.size();
            (s.width().ceil().max(1.0) as u32, s.height().ceil().max(1.0) as u32)
        }
    };
    check_size(width, height)?;
    Ok(Info {
        width,
        height,
        format,
    })
}

fn premultiply(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a < 255 {
            px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
            px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
            px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
        }
    }
}

fn pixmap_of(w: u32, h: u32, rgba: Vec<u8>) -> Res<Pixmap> {
    let size = tiny_skia::IntSize::from_wh(w, h).ok_or("image: it has no pixels")?;
    Pixmap::from_vec(rgba, size).ok_or_else(|| "image: its pixels do not fill its size".to_string())
}

/// A JPEG's pixels turned as its EXIF orientation says.
fn orient(pm: Pixmap, o: u8) -> Pixmap {
    if o <= 1 || o > 8 {
        return pm;
    }
    let (w, h) = (pm.width(), pm.height());
    let (nw, nh) = if o >= 5 { (h, w) } else { (w, h) };
    let Some(mut out) = Pixmap::new(nw, nh) else {
        return pm;
    };
    let (wf, hf) = (w as f32, h as f32);
    // Where a source pixel goes, as an affine map.
    let tf = match o {
        2 => Transform::from_row(-1.0, 0.0, 0.0, 1.0, wf, 0.0),
        3 => Transform::from_row(-1.0, 0.0, 0.0, -1.0, wf, hf),
        4 => Transform::from_row(1.0, 0.0, 0.0, -1.0, 0.0, hf),
        5 => Transform::from_row(0.0, 1.0, 1.0, 0.0, 0.0, 0.0),
        6 => Transform::from_row(0.0, 1.0, -1.0, 0.0, hf, 0.0),
        7 => Transform::from_row(0.0, -1.0, -1.0, 0.0, hf, wf),
        _ => Transform::from_row(0.0, -1.0, 1.0, 0.0, 0.0, wf),
    };
    let paint = PixmapPaint {
        quality: tiny_skia::FilterQuality::Nearest,
        ..Default::default()
    };
    out.draw_pixmap(0, 0, pm.as_ref(), &paint, tf, None);
    out
}

/// A raster picture's pixels at its own size.
fn decode_raster(b: &[u8], info: Info) -> Res<Pixmap> {
    match info.format {
        Format::Png => Pixmap::decode_png(b).map_err(|e| format!("image: not a PNG this can read ({e})")),
        Format::Jpeg => {
            use zune_jpeg::zune_core::{colorspace::ColorSpace, options::DecoderOptions};
            let opts = DecoderOptions::default()
                .jpeg_set_out_colorspace(ColorSpace::RGBA)
                .set_max_width(65535)
                .set_max_height(65535);
            let mut d = zune_jpeg::JpegDecoder::new_with_options(
                zune_jpeg::zune_core::bytestream::ZCursor::new(b),
                opts,
            );
            let px = d
                .decode()
                .map_err(|e| format!("image: not a JPEG this can read ({e:?})"))?;
            let (w, h) = d.dimensions().ok_or("image: a JPEG with no size")?;
            let pm = pixmap_of(w as u32, h as u32, px)?;
            Ok(orient(pm, jpeg_orientation(b)))
        }
        Format::Webp => {
            let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(b))
                .map_err(|e| format!("image: not a WebP this can read ({e})"))?;
            let (w, h) = d.dimensions();
            let n = d.output_buffer_size().ok_or("image: a WebP too large")?;
            let mut buf = vec![0u8; n];
            d.read_image(&mut buf)
                .map_err(|e| format!("image: not a WebP this can read ({e})"))?;
            let mut rgba = if d.has_alpha() {
                buf
            } else {
                let mut out = Vec::with_capacity((w * h * 4) as usize);
                for p in buf.chunks_exact(3) {
                    out.extend_from_slice(&[p[0], p[1], p[2], 255]);
                }
                out
            };
            premultiply(&mut rgba);
            pixmap_of(w, h, rgba)
        }
        Format::Gif => {
            let mut opts = gif::DecodeOptions::new();
            opts.set_color_output(gif::ColorOutput::RGBA);
            let mut d = opts
                .read_info(std::io::Cursor::new(b))
                .map_err(|e| format!("image: not a GIF this can read ({e})"))?;
            let (w, h) = (d.width() as u32, d.height() as u32);
            let mut canvas = vec![0u8; (w * h * 4) as usize];
            if let Some(f) = d
                .read_next_frame()
                .map_err(|e| format!("image: not a GIF this can read ({e})"))?
            {
                // the first frame, where it sits on the logical screen
                let (fl, ft, fw, fh) = (f.left as u32, f.top as u32, f.width as u32, f.height as u32);
                for y in 0..fh {
                    for x in 0..fw {
                        let (cx, cy) = (fl + x, ft + y);
                        if cx >= w || cy >= h {
                            continue;
                        }
                        let si = ((y * fw + x) * 4) as usize;
                        let di = ((cy * w + cx) * 4) as usize;
                        if si + 4 <= f.buffer.len() {
                            canvas[di..di + 4].copy_from_slice(&f.buffer[si..si + 4]);
                        }
                    }
                }
            }
            premultiply(&mut canvas);
            pixmap_of(w, h, canvas)
        }
        Format::Svg => Err("image: an SVG is drawn, not decoded".into()),
    }
}

/// `pm` drawn at `w × h`: halved while it is more than twice too large
/// (each step averages, so a large reduction does not alias), then
/// scaled the rest of the way.
fn scaled(pm: Pixmap, w: u32, h: u32) -> Pixmap {
    let mut cur = pm;
    while cur.width() / 2 >= w.max(1) && cur.height() / 2 >= h.max(1) && cur.width() > 1 && cur.height() > 1 {
        let (hw, hh) = (cur.width() / 2, cur.height() / 2);
        let Some(mut half) = Pixmap::new(hw, hh) else {
            break;
        };
        let paint = PixmapPaint {
            quality: tiny_skia::FilterQuality::Bilinear,
            ..Default::default()
        };
        half.draw_pixmap(
            0,
            0,
            cur.as_ref(),
            &paint,
            Transform::from_scale(hw as f32 / cur.width() as f32, hh as f32 / cur.height() as f32),
            None,
        );
        cur = half;
    }
    if cur.width() == w && cur.height() == h {
        return cur;
    }
    let Some(mut out) = Pixmap::new(w.max(1), h.max(1)) else {
        return cur;
    };
    let paint = PixmapPaint {
        quality: tiny_skia::FilterQuality::Bicubic,
        ..Default::default()
    };
    out.draw_pixmap(
        0,
        0,
        cur.as_ref(),
        &paint,
        Transform::from_scale(w as f32 / cur.width() as f32, h as f32 / cur.height() as f32),
        None,
    );
    out
}

/// What a window wants of a picture: its own pixels (`None`), or to
/// show it in `(w, h)` device pixels.
pub type Want = Option<(f32, f32)>;

/// The longest side a picture of `info` is kept at for `want`: 0 for
/// its own pixels.
fn bucket(info: Info, want: Want) -> u32 {
    let own = info.width.max(info.height);
    match (info.format, want) {
        // an SVG is drawn at the size it shows; its own size otherwise
        (Format::Svg, None) => own.min(MAX_SIDE),
        (Format::Svg, Some((w, h))) => {
            let k = (w / info.width as f32).max(h / info.height as f32);
            let side = (own as f32 * k).ceil().max(1.0) as u32;
            (side.div_ceil(32) * 32).clamp(32, MAX_SIDE)
        }
        (_, None) => 0,
        (_, Some((w, h))) => {
            // the side it shows at, as `contain` would show it
            let k = (w / info.width as f32).min(h / info.height as f32).max(0.0);
            let side = (own as f32 * k).ceil().max(1.0) as u32;
            let b = side.next_power_of_two().max(MIN_BUCKET);
            if b >= own { 0 } else { b }
        }
    }
}

/// The picture of `bytes` at `bucket` (see [`bucket`]).
fn make(bytes: &[u8], info: Info, bucket: u32) -> Res<Picture> {
    let own = info.width.max(info.height) as f32;
    let side = if bucket == 0 { own.min(MAX_SIDE as f32) } else { bucket as f32 };
    let k = side / own;
    let (w, h) = (
        ((info.width as f32 * k).round() as u32).max(1),
        ((info.height as f32 * k).round() as u32).max(1),
    );
    let pm = if info.format == Format::Svg {
        let tree = svg_tree(bytes)?;
        let mut pm = Pixmap::new(w, h).ok_or("image: too large")?;
        let s = tree.size();
        resvg::render(
            &tree,
            Transform::from_scale(w as f32 / s.width(), h as f32 / s.height()),
            &mut pm.as_mut(),
        );
        pm
    } else {
        let full = decode_raster(bytes, info)?;
        if full.width() == w && full.height() == h {
            full
        } else {
            scaled(full, w, h)
        }
    };
    Ok(Picture {
        id: super::canvas::next_picture_id(),
        width: pm.width(),
        height: pm.height(),
        rgba: pm.take(),
    })
}

// ── the cache ────────────────────────────────────────────────────────

struct Held {
    pic: Arc<Picture>,
    used: u64,
}

#[derive(Default)]
struct Cache {
    infos: HashMap<u64, Result<Info, String>>,
    pics: HashMap<(u64, u32), Held>,
    bytes: usize,
    clock: u64,
    /// Decodes queued or running, by source and size, with the windows
    /// to redraw when each is done.
    pending: HashMap<(u64, u32), HashSet<u64>>,
}

impl Cache {
    fn put(&mut self, key: (u64, u32), pic: Arc<Picture>) {
        self.clock += 1;
        self.bytes += pic.rgba.len();
        if let Some(old) = self.pics.insert(key, Held { pic, used: self.clock }) {
            self.bytes -= old.pic.rgba.len();
        }
        while self.bytes > BUDGET && self.pics.len() > 1 {
            let Some(oldest) = self
                .pics
                .iter()
                .filter(|(k, _)| **k != key)
                .min_by_key(|(_, h)| h.used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            if let Some(h) = self.pics.remove(&oldest) {
                self.bytes -= h.pic.rgba.len();
            }
        }
    }
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache::default()))
}

/// The size and format of a source, read from its header (and kept).
pub fn info(src: &Value) -> Res<Info> {
    let (id, sr) = identify(src)?;
    if let Some(hit) = cache().lock().ok().and_then(|c| c.infos.get(&id).cloned()) {
        return hit;
    }
    let b = sr.owned().read()?;
    let r = info_of(&b);
    if let Ok(mut c) = cache().lock() {
        c.infos.insert(id, r.clone());
    }
    r
}

struct Job {
    id: u64,
    src: Source,
    want: Want,
}

/// Decode a job: its info, then its picture at the size wanted, into the
/// cache; the windows that waited are redrawn.
fn run_job(job: Job) {
    let r = job.src.read().and_then(|b| {
        let known = cache().lock().ok().and_then(|c| c.infos.get(&job.id).cloned());
        let info = match known {
            Some(i) => i?,
            None => {
                let i = info_of(&b);
                if let Ok(mut c) = cache().lock() {
                    c.infos.insert(job.id, i.clone());
                }
                i?
            }
        };
        let k = bucket(info, job.want);
        let pic = make(&b, info, k)?;
        Ok((k, pic))
    });
    let mut waiting = HashSet::new();
    if let Ok(mut c) = cache().lock() {
        match r {
            Ok((k, pic)) => c.put((job.id, k), Arc::new(pic)),
            Err(e) => {
                c.infos.insert(job.id, Err(e));
            }
        }
        // every request for this source that waited: they look again
        let keys: Vec<_> = c.pending.keys().filter(|k| k.0 == job.id).copied().collect();
        for k in keys {
            if let Some(ws) = c.pending.remove(&k) {
                waiting.extend(ws);
            }
        }
    }
    for w in waiting {
        super::platform::redraw(w);
    }
}

fn queue() -> &'static crossbeam_channel::Sender<Job> {
    static Q: OnceLock<crossbeam_channel::Sender<Job>> = OnceLock::new();
    Q.get_or_init(|| {
        let (tx, rx) = crossbeam_channel::unbounded::<Job>();
        for i in 0..2 {
            let rx = rx.clone();
            let _ = std::thread::Builder::new()
                .name(format!("gui-picture-{i}"))
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        run_job(job);
                    }
                });
        }
        tx
    })
}

/// The picture of `src` for `want`, and its info: at once when it is
/// held (or `window` is `None`: decoded here), else `None` while a
/// decoding thread makes it and `window` is redrawn when it is ready.
/// A source that cannot be read answers `Err` (and is not tried again
/// until it changes).
pub fn get(src: &Value, want: Want, window: Option<u64>) -> Res<Option<(Arc<Picture>, Info)>> {
    let (id, sr) = identify(src)?;
    {
        let mut c = cache().lock().map_err(|_| "image: the cache is poisoned")?;
        if let Some(i) = c.infos.get(&id).cloned() {
            let info = i?;
            let k = bucket(info, want);
            c.clock += 1;
            let now = c.clock;
            if let Some(h) = c.pics.get_mut(&(id, k)) {
                h.used = now;
                return Ok(Some((h.pic.clone(), info)));
            }
            if let Some(w) = window {
                let first = !c.pending.contains_key(&(id, k));
                c.pending.entry((id, k)).or_default().insert(w);
                drop(c);
                if first {
                    let _ = queue().send(Job { id, src: sr.owned(), want });
                }
                return Ok(None);
            }
        } else if let Some(w) = window {
            // not even its header read yet: the decoding thread reads both
            let first = !c.pending.keys().any(|k| k.0 == id);
            c.pending.entry((id, u32::MAX)).or_default().insert(w);
            drop(c);
            if first {
                let _ = queue().send(Job { id, src: sr.owned(), want });
            }
            return Ok(None);
        }
    }
    // here, now (a headless window, or a caller off the window's thread)
    let b = match &sr {
        SrcRef::Bytes(b) => std::borrow::Cow::Borrowed(*b),
        SrcRef::Path(_) => std::borrow::Cow::Owned(sr.owned().read()?.to_vec()),
    };
    let info = match cache().lock().ok().and_then(|c| c.infos.get(&id).cloned()) {
        Some(i) => i?,
        None => {
            let i = info_of(&b);
            if let Ok(mut c) = cache().lock() {
                c.infos.insert(id, i.clone());
            }
            i?
        }
    };
    let k = bucket(info, want);
    let pic = Arc::new(make(&b, info, k)?);
    if let Ok(mut c) = cache().lock() {
        c.put((id, k), pic.clone());
    }
    Ok(Some((pic, info)))
}

/// What the cache holds: `(pictures, bytes)`, for a test.
pub fn held() -> (usize, usize) {
    cache().lock().map(|c| (c.pics.len(), c.bytes)).unwrap_or((0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_known_by_their_first_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), Some(Format::Png));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Format::Jpeg));
        assert_eq!(sniff(b"GIF89a...."), Some(Format::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Format::Webp));
        assert_eq!(sniff(b"  <?xml version='1.0'?><svg/>"), Some(Format::Svg));
        assert_eq!(sniff(b"hello"), None);
    }

    #[test]
    fn a_thumbnail_is_kept_at_the_power_of_two_above_it() {
        let photo = Info { width: 4000, height: 3000, format: Format::Jpeg };
        assert_eq!(bucket(photo, Some((200.0, 150.0))), 256);
        assert_eq!(bucket(photo, Some((10.0, 10.0))), MIN_BUCKET);
        assert_eq!(bucket(photo, None), 0);
        let icon = Info { width: 32, height: 32, format: Format::Png };
        assert_eq!(bucket(icon, Some((400.0, 400.0))), 0);
        let svg = Info { width: 24, height: 24, format: Format::Svg };
        assert_eq!(bucket(svg, Some((48.0, 48.0))), 64);
    }

    #[test]
    fn exif_orientation_is_read_from_both_byte_orders() {
        // II*\0, IFD at 8, one entry: 0x0112 SHORT 1 = 6
        let le = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0];
        assert_eq!(exif_orientation(&le), Some(6));
        let be = [b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 3, 0, 0];
        assert_eq!(exif_orientation(&be), Some(3));
    }
}

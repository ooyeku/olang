//! The GPU renderer (wgpu: Metal, Direct3D 12, Vulkan, GL): one
//! instanced pipeline whose fragment shader draws a rounded rectangle
//! (fill and border, antialiased by distance) or a glyph from a shared
//! atlas, clipped per instance. Painter's order is the instance order,
//! so one draw call paints a frame.
//!
//! It draws what the software renderer draws, the same way: colours
//! are blended in sRGB-encoded space (the target is a non-sRGB format),
//! borders lie inside the box, a fill sits under its border, and glyph
//! images come from the same rasterizer at the same subpixel offsets.
//!
//! A window draws into a texture of its own that it keeps (a [`Target`]):
//! a frame redraws only what [`damage`](super::damage) says changed,
//! scissored, over the last one, and the target is copied to the
//! swapchain; a frame that changed no pixel is not presented at all.

use super::canvas::Picture;
use super::damage::{self, Damage, IRect, Item, Scroll, Shift, Tracker};
use super::raster::{self, GlyphKey};
use super::text::Color;
use super::vector;
use super::window::{DisplayList, PathPrim, Prim};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

const SHADER: &str = r#"
struct Globals { size: vec2<f32>, atlas: vec2<f32> };
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Inst {
  @location(0) rect: vec4<f32>,
  @location(1) clip: vec4<f32>,
  @location(2) fill: vec4<f32>,
  @location(3) border: vec4<f32>,
  @location(4) radii: vec4<f32>,
  @location(5) uv: vec4<f32>,
  @location(6) params: vec4<f32>,
  @location(7) clip_radii: vec4<f32>,
};

struct VOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) p: vec2<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) @interpolate(flat) rect: vec4<f32>,
  @location(3) @interpolate(flat) clip: vec4<f32>,
  @location(4) @interpolate(flat) fill: vec4<f32>,
  @location(5) @interpolate(flat) border: vec4<f32>,
  @location(6) @interpolate(flat) radii: vec4<f32>,
  @location(7) @interpolate(flat) params: vec4<f32>,
  @location(8) @interpolate(flat) clip_radii: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, i: Inst) -> VOut {
  let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
  // Shapes grow a pixel each way so their antialiased edge is drawn, a
  // shadow three sigmas and a pixel.
  var pad = 0.0;
  if (i.params.x < 0.5 || i.params.x > 4.5) {
    pad = 1.0;
  } else if (i.params.x > 2.5 && i.params.x < 3.5) {
    pad = ceil(3.0 * i.params.w) + 1.0;
  }
  let p = i.rect.xy - vec2<f32>(pad) + corner * (i.rect.zw + vec2<f32>(2.0 * pad));
  var o: VOut;
  o.pos = vec4<f32>(p / g.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
  o.p = p;
  o.uv = mix(i.uv.xy, i.uv.zw, corner);
  o.rect = i.rect;
  o.clip = i.clip;
  o.fill = i.fill;
  o.border = i.border;
  o.radii = i.radii;
  o.params = i.params;
  o.clip_radii = i.clip_radii;
  return o;
}

fn sd_round_box(q: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
  let d = abs(q) - half + vec2<f32>(r);
  return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - r;
}

fn corner_radius(q: vec2<f32>, radii: vec4<f32>) -> f32 {
  // radii: top-left, top-right, bottom-right, bottom-left
  if (q.x < 0.0) {
    return select(radii.w, radii.x, q.y < 0.0);
  }
  return select(radii.z, radii.y, q.y < 0.0);
}

/// How far past its corner's radius a continuous corner reaches along
/// each edge (Apple's continuous corners; soft.rs draws the same curve).
const CONTINUOUS_EXTENT: f32 = 1.528;

/// The signed distance (an estimate near the edge, exact on its straight
/// parts) to a box of half size `half` whose corner, of radius `r`, is
/// continuous: a superellipse reaching `p` along each edge, as deep at
/// its diagonal as a quarter circle of radius `r`, its curvature easing
/// in from the straight edge.
fn sd_continuous_box(q: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
  if (r <= 0.0) {
    return sd_round_box(q, half, 0.0);
  }
  let p = min(CONTINUOUS_EXTENT * r, min(half.x, half.y));
  let depth = r * (1.0 - 0.70710678);
  let n = 0.69314718 / -log(1.0 - depth / p);
  let a = abs(q) - (half - vec2<f32>(p));
  if (a.x > 0.0 && a.y > 0.0) {
    let m = pow(pow(a.x, n) + pow(a.y, n), 1.0 / n);
    let g = length(vec2<f32>(pow(a.x / m, n - 1.0), pow(a.y / m, n - 1.0)));
    return (m - p) / max(g, 1e-4);
  }
  return max(a.x, a.y) - p;
}

fn sd_box(q: vec2<f32>, half: vec2<f32>, r: f32, continuous: bool) -> f32 {
  if (continuous) {
    return sd_continuous_box(q, half, r);
  }
  return sd_round_box(q, half, r);
}

/// Coverage of a pixel by a rounded clip (1 inside a plain one).
fn clip_cover(v: VOut) -> f32 {
  if (v.clip_radii.x + v.clip_radii.y + v.clip_radii.z + v.clip_radii.w <= 0.0) {
    return 1.0;
  }
  let half = (v.clip.zw - v.clip.xy) * 0.5;
  let q = v.p - (v.clip.xy + half);
  let r = clamp(corner_radius(q, v.clip_radii), 0.0, min(half.x, half.y));
  let continuous = (u32(v.params.z) & 2u) != 0u;
  return clamp(0.5 - sd_box(q, half, r, continuous), 0.0, 1.0);
}

/// erf, to about 5e-4 (Abramowitz and Stegun 7.1.27), ±1 exactly past
/// 2.5 (where it is within 4e-4 of it): a shadow's inside is its colour
/// exactly. soft.rs has the same.
fn erf2(x: vec2<f32>) -> vec2<f32> {
  let s = sign(x);
  let a = abs(x);
  var t = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
  t = t * t;
  return select(s - s / (t * t), s, a >= vec2<f32>(2.5));
}

/// The Gaussian at `y` times the step (soft.rs: `shadow_weight`).
fn shadow_weight(y: f32, sigma: f32, step: f32) -> f32 {
  return exp(-(y * y) / (2.0 * sigma * sigma)) * step;
}

/// The Gaussian blur of a rounded box's coverage at `q` (from its centre):
/// exact across, integrated down in four steps over three sigmas (Evan
/// Wallace's closed form), the steps' weights normalised so the inside is
/// whole. soft.rs computes the same, row by row.
fn shadow_cover(q: vec2<f32>, half: vec2<f32>, sigma: f32, corner: f32) -> f32 {
  let low = q.y - half.y;
  let high = q.y + half.y;
  let start = clamp(-3.0 * sigma, low, high);
  let end = clamp(3.0 * sigma, low, high);
  let step = (end - start) / 4.0;
  let start0 = -3.0 * sigma;
  let step0 = (3.0 * sigma - start0) / 4.0;
  var total = 0.0;
  var value = 0.0;
  for (var i = 0; i < 4; i = i + 1) {
    let y = start + step * (f32(i) + 0.5);
    let yy = q.y - y;
    let delta = min(half.y - corner - abs(yy), 0.0);
    let curved = half.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
    let integral = 0.5 + 0.5 * erf2((q.x + vec2<f32>(-curved, curved)) * (0.70710678 / sigma));
    value = value + (integral.y - integral.x) * shadow_weight(y, sigma, step);
    total = total + shadow_weight(start0 + step0 * (f32(i) + 0.5), sigma, step0);
  }
  return clamp(value / total, 0.0, 1.0);
}

@fragment
fn fs(v: VOut) -> @location(0) vec4<f32> {
  if (v.p.x < v.clip.x || v.p.y < v.clip.y || v.p.x >= v.clip.z || v.p.y >= v.clip.w) {
    discard;
  }
  let cover = clip_cover(v);
  let kind = v.params.x;
  if (kind > 5.5) {
    // A material: what was drawn under the box, blurred (BLUR_SHADER, a
    // texture of `k`-pixel blocks; the quad's corners say where they fall
    // in it), within the box's shape.
    let half = v.rect.zw * 0.5;
    let q = v.p - (v.rect.xy + half);
    let r = clamp(corner_radius(q, v.radii), 0.0, min(half.x, half.y));
    let continuous = (u32(v.params.z) & 1u) != 0u;
    let inside = clamp(0.5 - sd_box(q, half, r, continuous), 0.0, 1.0);
    return textureSample(atlas, samp, v.uv) * inside * cover;
  }
  if (kind > 4.5) {
    // A radial gradient: its strip read by the distance from the box's
    // centre (its farthest corner is the strip's end), within its box.
    let half = v.rect.zw * 0.5;
    let q = v.p - (v.rect.xy + half);
    let r = clamp(corner_radius(q, v.radii), 0.0, min(half.x, half.y));
    let inside = clamp(0.5 - sd_round_box(q, half, r), 0.0, 1.0);
    let t = clamp(length(q) / max(length(half), 1e-3), 0.0, 1.0);
    let u = (t * 255.0 + 0.5) / 256.0;
    return textureSample(atlas, samp, vec2<f32>(u, 0.5)) * v.fill.a * inside * cover;
  }
  if (kind > 3.5) {
    // A vector path: its coverage, drawn on the GPU into an atlas page
    // (COVER_SHADER), of its paint.
    return v.fill * textureSample(atlas, samp, v.uv).r * cover;
  }
  if (kind > 2.5) {
    // A shadow: the blurred shape's coverage of the shadow's colour.
    let half = v.rect.zw * 0.5;
    let q = v.p - (v.rect.xy + half);
    let corner = clamp(corner_radius(q, v.radii), 0.0, min(half.x, half.y));
    return v.fill * shadow_cover(q, half, max(v.params.w, 0.5), corner) * cover;
  }
  if (kind > 1.5) {
    // A colour glyph: premultiplied, faded by the text colour's alpha.
    return textureSample(atlas, samp, v.uv) * v.fill.a * cover;
  }
  if (kind > 0.5) {
    let a = textureSample(atlas, samp, v.uv).a;
    return v.fill * a * cover;
  }
  let half = v.rect.zw * 0.5;
  let q = v.p - (v.rect.xy + half);
  let rmax = min(half.x, half.y);
  let r = clamp(corner_radius(q, v.radii), 0.0, rmax);
  let continuous = (u32(v.params.z) & 1u) != 0u;
  let outer = clamp(0.5 - sd_box(q, half, r, continuous), 0.0, 1.0);
  let bw = v.params.y;
  var ring = vec4<f32>(0.0);
  if (bw > 0.0) {
    let ih = max(half - vec2<f32>(bw), vec2<f32>(0.0));
    let ir = clamp(max(r - bw, 0.0), 0.0, min(ih.x, ih.y));
    var inner = 0.0;
    if (ih.x > 0.0 && ih.y > 0.0) {
      inner = clamp(0.5 - sd_box(q, ih, ir, continuous), 0.0, 1.0);
    }
    ring = v.border * clamp(outer - inner, 0.0, 1.0);
  }
  let fill = v.fill * outer;
  return (ring + fill * (1.0 - ring.a)) * cover;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Inst {
    rect: [f32; 4],
    clip: [f32; 4],
    fill: [f32; 4],
    border: [f32; 4],
    radii: [f32; 4],
    uv: [f32; 4],
    params: [f32; 4],
    clip_radii: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    size: [f32; 2],
    atlas: [f32; 2],
}

/// An instance's flags for its clip (`params.z`): 2 when the clip's
/// corners are continuous.
fn clip_flags(c: &super::window::Clip) -> f32 {
    if c.continuous && c.is_rounded() { 2.0 } else { 0.0 }
}

fn premul(c: Color) -> [f32; 4] {
    let a = c[3] as f32 / 255.0;
    [
        c[0] as f32 / 255.0 * a,
        c[1] as f32 / 255.0 * a,
        c[2] as f32 / 255.0 * a,
        a,
    ]
}

/// An atlas page's side.
const ATLAS: u32 = 2048;
/// The most pages the glyph atlas keeps (16 MB each).
const ATLAS_PAGES: usize = 4;

#[derive(Clone, Copy)]
struct Slot {
    page: u16,
    uv: [f32; 4],
    left: i32,
    top: i32,
    w: u32,
    h: u32,
    color: bool,
    /// The glyph's key hashed: what damage compares, never its place.
    ident: u64,
}

/// A page of the glyph atlas, packed in rows.
struct Page {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    x: u32,
    y: u32,
    row: u32,
    /// The frame that last drew a glyph from it.
    used: u64,
    keys: Vec<GlyphKey>,
}

/// The glyph atlas: pages of glyph images, the least recently drawn page
/// emptied when every page is full. A page a frame has drawn from is never
/// emptied during that frame (its glyphs are already in the frame), so a
/// frame never samples a glyph that was overwritten.
struct Atlas {
    pages: Vec<Page>,
    /// The page new glyphs go on.
    current: usize,
    slots: HashMap<GlyphKey, Option<Slot>>,
    frame: u64,
    /// Frames prepared and not yet drawn (a window's, a test's on another
    /// thread): a page any of them used is not emptied under them.
    open: Vec<u64>,
    /// Pages emptied to make room, ever.
    evicted: u64,
}

fn atlas_page(device: &wgpu::Device) -> Page {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gui glyphs"),
        size: wgpu::Extent3d {
            width: ATLAS,
            height: ATLAS,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Page {
        texture,
        view,
        x: 0,
        y: 0,
        row: 0,
        used: 0,
        keys: Vec::new(),
    }
}

impl Page {
    /// Room for a `w`×`h` image: its place, the packing advanced.
    fn fit(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if self.x + w + 1 > ATLAS {
            self.x = 0;
            self.y += self.row + 1;
            self.row = 0;
        }
        if self.y + h + 1 > ATLAS {
            return None;
        }
        let at = (self.x, self.y);
        self.x += w + 1;
        self.row = self.row.max(h);
        Some(at)
    }
}

/// Past this many remembered glyphs, those with no ink are forgotten.
const SLOTS_MAX: usize = 1 << 16;

impl Atlas {
    /// A new frame begins: pages it draws from are marked with it.
    fn begin_frame(&mut self) -> u64 {
        self.frame += 1;
        self.open.push(self.frame);
        if self.slots.len() > SLOTS_MAX {
            self.slots.retain(|_, s| s.is_some());
        }
        self.frame
    }

    /// Empty a page for new glyphs.
    fn empty(&mut self, i: usize) {
        let keys = std::mem::take(&mut self.pages[i].keys);
        for k in keys {
            self.slots.remove(&k);
        }
        let p = &mut self.pages[i];
        p.x = 0;
        p.y = 0;
        p.row = 0;
        self.evicted += 1;
    }

    /// Place a glyph; `None` when it has no ink, or no page has room
    /// this frame (every page holds glyphs this frame drew: it is drawn
    /// next frame).
    fn place(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: &GlyphKey,
        coords: &[i16],
    ) -> Option<Slot> {
        if let Some(s) = self.slots.get(key) {
            if let Some(s) = s {
                self.pages[s.page as usize].used = self.frame;
            }
            return *s;
        }
        let Some(img) = raster::glyph(key, coords) else {
            self.slots.insert(key.clone(), None);
            return None;
        };
        let (w, h) = (img.width, img.height);
        if w + 1 > ATLAS || h + 1 > ATLAS {
            self.slots.insert(key.clone(), None);
            return None;
        }
        if self.pages.is_empty() {
            self.pages.push(atlas_page(device));
            self.current = 0;
        }
        let mut at = self.pages[self.current].fit(w, h);
        if at.is_none() {
            // a new page while there may be more, else the page drawn
            // from longest ago (never one this frame used)
            let next = if self.pages.len() < ATLAS_PAGES {
                self.pages.push(atlas_page(device));
                Some(self.pages.len() - 1)
            } else {
                let oldest = self.open.iter().copied().min().unwrap_or(self.frame);
                let lru = (0..self.pages.len())
                    .filter(|i| self.pages[*i].used < oldest)
                    .min_by_key(|i| self.pages[*i].used);
                if let Some(i) = lru {
                    self.empty(i);
                }
                lru
            };
            let next = next?;
            self.current = next;
            at = self.pages[next].fit(w, h);
        }
        let (x, y) = at?;
        let page = &mut self.pages[self.current];
        page.used = self.frame;
        page.keys.push(key.clone());
        let mono;
        let rgba: &[u8] = if img.color {
            &img.data
        } else {
            mono = img.data.iter().flat_map(|a| [255, 255, 255, *a]).collect::<Vec<u8>>();
            &mono
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &page.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let f = ATLAS as f32;
        let mut hs = rustc_hash::FxHasher::default();
        key.hash(&mut hs);
        let slot = Slot {
            page: self.current as u16,
            uv: [
                x as f32 / f,
                y as f32 / f,
                (x + w) as f32 / f,
                (y + h) as f32 / f,
            ],
            left: img.left,
            top: img.top,
            w,
            h,
            color: img.color,
            ident: hs.finish(),
        };
        self.slots.insert(key.clone(), Some(slot));
        Some(slot)
    }
}

/// The device, the queue, the pipelines, and the glyph atlas, shared by
/// every window.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    /// By target format, and whether it blends (`false`: it replaces, for
    /// clearing a damaged rectangle and copying a frame).
    pipelines: Mutex<HashMap<(wgpu::TextureFormat, bool), Arc<wgpu::RenderPipeline>>>,
    sampler: wgpu::Sampler,
    /// Pictures (canvases, images) are scaled, so they sample smoothly.
    linear: wgpu::Sampler,
    atlas: Arc<Mutex<Atlas>>,
    /// A texture per picture, by the picture's id.
    pictures: Mutex<Textures>,
    /// The bytes of windows' kept frames (their [`Target`]s).
    targets: std::sync::atomic::AtomicUsize,
    /// Vector paths' coverage: its pipelines and textures.
    cover: Mutex<CoverState>,
    /// Materials' blur: its pipelines and textures.
    blur: Mutex<BlurState>,
    /// The frame's conversion to Display P3, by the surface's format.
    p3: Mutex<Vec<(wgpu::TextureFormat, wgpu::BindGroupLayout, wgpu::RenderPipeline)>>,
    /// The device was lost (the GPU reset, unplugged, the machine slept
    /// through a driver's reset) or ran out of memory: said once, and every
    /// window's frame refused (`"lost"`) rather than drawn on a dead device.
    pub lost: Arc<std::sync::atomic::AtomicBool>,
}

impl Gpu {
    /// Whether the device can no longer draw.
    pub fn is_lost(&self) -> bool {
        self.lost.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The pictures' textures, the least recently drawn dropped past a
/// budget of bytes (an animation's frames are each a picture: a long one
/// cycles through its own without pushing out every other picture).
#[derive(Default)]
struct Textures {
    views: HashMap<u64, (wgpu::TextureView, usize, u64)>,
    bytes: usize,
    clock: u64,
    evicted: u64,
}

/// The bytes of picture textures kept on the GPU.
const TEXTURE_BUDGET: usize = 256 << 20;

/// What the GPU holds, for a HUD and for tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub atlas_pages: usize,
    pub atlas_bytes: usize,
    pub glyphs: usize,
    pub atlas_evictions: u64,
    pub pictures: usize,
    pub picture_bytes: usize,
    pub picture_evictions: u64,
    pub target_bytes: usize,
    /// Vector paths' coverage: its pages and multisampled target.
    pub cover_bytes: usize,
}

impl Gpu {
    pub fn stats(&self) -> Stats {
        let mut s = Stats::default();
        if let Ok(a) = self.atlas.lock() {
            s.atlas_pages = a.pages.len();
            s.atlas_bytes = a.pages.len() * (ATLAS * ATLAS * 4) as usize;
            s.glyphs = a.slots.values().filter(|v| v.is_some()).count();
            s.atlas_evictions = a.evicted;
        }
        if let Ok(p) = self.pictures.lock() {
            s.pictures = p.views.len();
            s.picture_bytes = p.bytes;
            s.picture_evictions = p.evicted;
        }
        s.target_bytes = self.targets.load(std::sync::atomic::Ordering::Relaxed);
        if let Ok(c) = self.cover.lock() {
            s.cover_bytes = c.bytes;
        }
        s
    }
}

/// The process's GPU's stats, when it has one (made by a window or a test).
pub fn stats() -> Option<Stats> {
    current().map(|g| g.stats())
}

/// The process's GPU: made on first use, and made again in place of one
/// whose device was lost ([`recover`]).
static GPU: Mutex<Option<Result<Arc<Gpu>, String>>> = Mutex::new(None);

/// How many times a lost device was replaced ([`recover`]).
static RECOVERED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The process's GPU, made on first use. `compatible` is a surface the
/// adapter must be able to present to (the first window's).
pub fn gpu(
    compatible: Option<&wgpu::Surface<'_>>,
    instance: Option<wgpu::Instance>,
) -> Result<Arc<Gpu>, String> {
    let mut slot = GPU.lock().unwrap_or_else(|e| e.into_inner());
    match &*slot {
        // lost: made again for whoever asks next (a headless window's
        // frame read as pixels, a new window)
        Some(Ok(g)) if g.is_lost() => {
            let g = g.clone();
            drop(slot);
            return recover(&g, compatible);
        }
        Some(g) => return g.clone(),
        None => {}
    }
    let g = make(compatible, instance).map(Arc::new);
    *slot = Some(g.clone());
    g
}

/// The GPU in place of `lost`, whose device was lost (a GPU reset, an
/// eGPU unplugged, out of memory): a new adapter and device on the same
/// instance, able to present to `compatible`, with pipelines, the glyph
/// atlas and pictures' textures made again as they are next drawn. Made
/// once however many windows ask; every later caller gets it. `Err` when
/// no device can be had now (a later frame asks again).
pub fn recover(lost: &Arc<Gpu>, compatible: Option<&wgpu::Surface<'_>>) -> Result<Arc<Gpu>, String> {
    let mut slot = GPU.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(Ok(now)) = &*slot
        && !Arc::ptr_eq(now, lost)
        && !now.is_lost()
    {
        return Ok(now.clone());
    }
    let g = Arc::new(make(compatible, Some(lost.instance.clone()))?);
    RECOVERED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    eprintln!("gui: the GPU device was made again; drawing goes on");
    *slot = Some(Ok(g.clone()));
    Ok(g)
}

/// The process's GPU as it is now, when one was made (never makes one).
pub fn current() -> Option<Arc<Gpu>> {
    match &*GPU.lock().unwrap_or_else(|e| e.into_inner()) {
        Some(Ok(g)) => Some(g.clone()),
        _ => None,
    }
}

/// How many times a lost device has been made again.
pub fn recoveries() -> u64 {
    RECOVERED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Lose the process's device as a GPU reset would (a test's): it is
/// destroyed and marked lost; the next frame of each window makes a new
/// one ([`recover`]).
pub fn simulate_loss() -> bool {
    let g = match &*GPU.lock().unwrap_or_else(|e| e.into_inner()) {
        Some(Ok(g)) => g.clone(),
        _ => return false,
    };
    g.lost.store(true, std::sync::atomic::Ordering::SeqCst);
    g.device.destroy();
    true
}

/// An instance for offscreen drawing (no window).
pub fn new_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env())
}

/// An instance that can present to windows of the event loop's display
/// (Wayland needs the display; elsewhere it is harmless). The display, not
/// a window: the instance is the process's for good, and a window it held
/// would never be dropped when the program closes it — it stayed on the
/// screen, and was torn down only by winit as the loop ended, after its
/// accessibility adapter had gone (a crash in AppKit's teardown).
fn window_instance(display: winit::event_loop::OwnedDisplayHandle) -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
        Box::new(display),
    ))
}

fn make(
    compatible: Option<&wgpu::Surface<'_>>,
    instance: Option<wgpu::Instance>,
) -> Result<Gpu, String> {
    let instance = instance.unwrap_or_else(new_instance);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: false,
        compatible_surface: compatible,
        ..Default::default()
    }))
    .map_err(|e| format!("no usable GPU adapter ({e})"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gui"),
        required_limits:
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .map_err(|e| format!("the GPU refused a device ({e})"))?;
    // wgpu's own handler panics on any error it was not asked to capture,
    // and a panic here aborts the application with the person's work in it:
    // an error is said instead (each kind once), and a device out of memory
    // or lost stops drawing (`lost`) — the windows stay, the program runs.
    let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let said = Arc::new(Mutex::new(std::collections::HashSet::<String>::new()));
    {
        let lost = lost.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            let kind = match &e {
                wgpu::Error::OutOfMemory { .. } => {
                    lost.store(true, std::sync::atomic::Ordering::SeqCst);
                    "out of memory"
                }
                wgpu::Error::Internal { .. } => "internal",
                wgpu::Error::Validation { .. } => "validation",
            };
            let text = e.to_string();
            let first = said.lock().map(|mut s| s.len() < 64 && s.insert(format!("{kind}: {text}"))).unwrap_or(false);
            if first {
                eprintln!("gui: the GPU said ({kind}): {text}");
            }
        }));
    }
    {
        let lost = lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            // dropped on purpose is no loss
            if reason != wgpu::DeviceLostReason::Destroyed {
                lost.store(true, std::sync::atomic::Ordering::SeqCst);
                eprintln!("gui: the GPU device was lost ({reason:?}): {message}");
            }
        });
    }
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gui"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gui"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gui"),
        bind_group_layouts: &[Some(&layout)],
        ..Default::default()
    });
    // Glyphs are drawn at their own pixel size, so nearest sampling
    // reproduces the rasterizer's pixels exactly.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("gui"),
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    let linear = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("gui pictures"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    Ok(Gpu {
        instance,
        adapter,
        device,
        queue,
        layout,
        pipeline_layout,
        shader,
        pipelines: Mutex::new(HashMap::new()),
        sampler,
        linear,
        pictures: Mutex::new(Textures::default()),
        targets: std::sync::atomic::AtomicUsize::new(0),
        cover: Mutex::new(CoverState::default()),
        blur: Mutex::new(BlurState::default()),
        p3: Mutex::new(Vec::new()),
        lost,
        atlas: Arc::new(Mutex::new(Atlas {
            pages: Vec::new(),
            current: 0,
            slots: HashMap::new(),
            frame: 0,
            open: Vec::new(),
            evicted: 0,
        })),
    })
}


/// What an instance samples: no texture (a shape), an atlas page, or a
/// frame's picture (`PIC + its index`).
const ANY: u32 = u32::MAX;
const PIC: u32 = 1 << 16;
/// A vector path's coverage: `COVER + its atlas page`.
const COVER: u32 = 1 << 15;
/// A material's blurred backdrop: `BACKDROP + its index` in the frame.
const BACKDROP: u32 = 1 << 14;

/// A display list made ready to draw: its instances, what each samples,
/// and what damage reads of each (the pixels it can touch, and a hash of
/// what decides them).
pub struct Prepared {
    insts: Vec<Inst>,
    src: Vec<u32>,
    pics: Vec<Arc<Picture>>,
    /// What damage reads of each instance (given to the tracker), and
    /// each one's pixels (kept, to pick those a damaged rectangle needs).
    items: Vec<Item>,
    bounds: Vec<IRect>,
    scrolls: Vec<Scroll>,
    width: u32,
    height: u32,
    clear: Color,
    /// A glyph found no room in the atlas: drawn next frame, whole.
    incomplete: bool,
    /// The vector paths' parts and their instances (coverage made as the
    /// frame is encoded, for those it draws).
    paths: Vec<PathJob>,
    /// The materials: each one's instance, the window's pixels its blur
    /// reads (its box and three sigmas round it), and its sigma.
    backdrops: Vec<(usize, IRect, f32)>,
    /// Its frame in the atlas, open until this is dropped (drawn).
    frame: u64,
    atlas: Arc<Mutex<Atlas>>,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        let mut a = self.atlas.lock().unwrap_or_else(|e| e.into_inner());
        let f = self.frame;
        a.open.retain(|x| *x != f);
    }
}

/// The repaint rainbow: a translucent instance over each region the frame
/// draws, after its primitives (damage has taken their items: the tint is
/// never part of what it compares, so a region keeps the colour of the
/// frame that last drew it).
fn flash(p: &mut Prepared, d: &Damage, n: u32) {
    let fill = premul(damage::flash_color(n));
    for r in damage::drawn(d, p.width, p.height) {
        p.insts.push(Inst {
            rect: [
                r.x0 as f32,
                r.y0 as f32,
                (r.x1 - r.x0) as f32,
                (r.y1 - r.y0) as f32,
            ],
            clip: [-1.0e9, -1.0e9, 1.0e9, 1.0e9],
            fill,
            border: [0.0; 4],
            radii: [0.0; 4],
            uv: [0.0; 4],
            params: [0.0; 4],
            clip_radii: [0.0; 4],
        });
        p.src.push(ANY);
        p.bounds.push(r);
    }
}

/// A vector path's part drawn by instance `inst` over `rect` (device
/// pixels: its pixels in the window and its clip).
struct PathJob {
    prim: Arc<PathPrim>,
    part: vector::Part,
    inst: usize,
    rect: IRect,
}

/// Instances in runs of one source: `(first, end, source)`.
type Runs = Vec<(u32, u32, u32)>;

/// An instance's hash but for its place and clip (damage's `Item::base`).
fn inst_base(i: &Inst, ident: u64) -> u64 {
    let mut h = damage::mix_f32s(ident, &[i.rect[2], i.rect[3]]);
    for v in [i.fill, i.border, i.radii, i.params] {
        h = damage::mix_f32s(h, &v);
    }
    h
}

/// Where this frame's paths' coverage is: an instance's source (its atlas
/// page) and its place there.
type Covered = HashMap<usize, (u32, [f32; 4])>;

/// Append instances `order` of `p` to `out`, in runs of one source (a
/// path's from `covered`).
fn push_runs(p: &Prepared, order: impl Iterator<Item = usize>, out: &mut Vec<Inst>, runs: &mut Runs, covered: &Covered) {
    let mut start = out.len() as u32;
    let mut cur = ANY;
    for i in order {
        let mut inst = p.insts[i];
        let s = if covered.is_empty() {
            p.src[i]
        } else if let Some((src, uv)) = covered.get(&i) {
            inst.uv = *uv;
            *src
        } else if p.src[i] == COVER {
            // a path whose coverage was not made: nothing of it
            continue;
        } else {
            p.src[i]
        };
        // a material reads what is drawn before it: it begins a run of
        // its own whatever came before
        let material = (BACKDROP..COVER).contains(&s);
        if s != ANY && s != cur {
            if (cur != ANY || material) && out.len() as u32 > start {
                runs.push((start, out.len() as u32, cur));
                start = out.len() as u32;
            }
            cur = s;
        }
        out.push(inst);
    }
    if out.len() as u32 > start {
        runs.push((start, out.len() as u32, cur));
    }
}

impl Gpu {
    fn pipeline(&self, format: wgpu::TextureFormat, blend: bool) -> Arc<wgpu::RenderPipeline> {
        let mut p = self.pipelines.lock().unwrap_or_else(|e| e.into_inner());
        p.entry((format, blend))
            .or_insert_with(|| {
                let attrs = wgpu::vertex_attr_array![
                    0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
                    4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
                ];
                Arc::new(
                    self.device
                        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                            label: Some("gui"),
                            layout: Some(&self.pipeline_layout),
                            vertex: wgpu::VertexState {
                                module: &self.shader,
                                entry_point: Some("vs"),
                                compilation_options: Default::default(),
                                buffers: &[Some(wgpu::VertexBufferLayout {
                                    array_stride: std::mem::size_of::<Inst>() as u64,
                                    step_mode: wgpu::VertexStepMode::Instance,
                                    attributes: &attrs,
                                })],
                            },
                            fragment: Some(wgpu::FragmentState {
                                module: &self.shader,
                                entry_point: Some("fs"),
                                compilation_options: Default::default(),
                                targets: &[Some(wgpu::ColorTargetState {
                                    format,
                                    blend: blend
                                        .then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                                    write_mask: wgpu::ColorWrites::ALL,
                                })],
                            }),
                            primitive: wgpu::PrimitiveState {
                                topology: wgpu::PrimitiveTopology::TriangleStrip,
                                ..Default::default()
                            },
                            depth_stencil: None,
                            multisample: wgpu::MultisampleState::default(),
                            multiview_mask: None,
                            cache: None,
                        }),
                )
            })
            .clone()
    }

    /// Make a display list's instances, placing its glyphs in the atlas.
    pub fn prepare(&self, dl: &DisplayList) -> Prepared {
        let mut atlas = self.atlas.lock().unwrap_or_else(|e| e.into_inner());
        let frame = atlas.begin_frame();
        let (ww, wh) = (dl.width.max(1), dl.height.max(1));
        let mut p = Prepared {
            insts: Vec::with_capacity(dl.prims.len()),
            src: Vec::with_capacity(dl.prims.len()),
            pics: Vec::new(),
            items: Vec::with_capacity(dl.prims.len()),
            bounds: Vec::with_capacity(dl.prims.len()),
            scrolls: dl.scrolls.clone(),
            width: ww,
            height: wh,
            clear: dl.clear,
            incomplete: false,
            paths: Vec::new(),
            backdrops: Vec::new(),
            frame,
            atlas: self.atlas.clone(),
        };
        let push = |p: &mut Prepared, inst: Inst, src: u32, pad: f32, ident: u64| {
            let c = super::window::Clip {
                x0: inst.clip[0],
                y0: inst.clip[1],
                x1: inst.clip[2],
                y1: inst.clip[3],
                radii: inst.clip_radii,
                continuous: (inst.params[2] as u32) & 2 != 0,
            };
            let [x, y, w, h] = inst.rect;
            let Some(b) = damage::bounds(x, y, w, h, pad, &c, ww, wh) else {
                return;
            };
            let mut item = Item::new(b, inst_base(&inst, ident), x, y, c);
            if inst.params[0] < 0.5 {
                // a shape: one colour inside its border and corners
                let mut r = inst.radii.iter().fold(0.0f32, |a, v| a.max(*v));
                if (inst.params[2] as u32) & 1 != 0 {
                    r *= damage::CONTINUOUS_EXTENT;
                }
                let r = r.max(inst.params[1]);
                item = item.with_interior(damage::inside(x, y, w, h, r + 1.0));
            }
            p.items.push(item);
            p.bounds.push(b);
            p.insts.push(inst);
            p.src.push(src);
        };
        for prim in &dl.prims {
            match prim {
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
                    // Drawn as a colour glyph: premultiplied, sampled whole,
                    // faded by the fill's alpha (its opacity).
                    let a = alpha.clamp(0.0, 1.0);
                    let src = PIC + p.pics.len() as u32;
                    p.pics.push(pic.clone());
                    push(
                        &mut p,
                        Inst {
                            rect: [*x, *y, *w, *h],
                            clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                            fill: [a, a, a, a],
                            border: [0.0; 4],
                            radii: [0.0; 4],
                            uv: [0.0, 0.0, 1.0, 1.0],
                            params: [2.0, 0.0, clip_flags(clip), 0.0],
                            clip_radii: clip.radii,
                        },
                        src,
                        1.0,
                        damage::mix(0x7069_6374, pic.id),
                    );
                }
                Prim::Rect {
                    x,
                    y,
                    w,
                    h,
                    fill,
                    border,
                    border_width,
                    radii,
                    continuous,
                    clip,
                } => {
                    if clip.is_empty() || *w <= 0.0 || *h <= 0.0 {
                        continue;
                    }
                    let flags = clip_flags(clip) + if *continuous { 1.0 } else { 0.0 };
                    push(
                        &mut p,
                        Inst {
                            rect: [*x, *y, *w, *h],
                            clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                            fill: premul(*fill),
                            border: premul(*border),
                            radii: *radii,
                            uv: [0.0; 4],
                            params: [0.0, *border_width, flags, 0.0],
                            clip_radii: clip.radii,
                        },
                        ANY,
                        1.0,
                        0,
                    );
                }
                Prim::Backdrop {
                    x,
                    y,
                    w,
                    h,
                    radii,
                    continuous,
                    sigma,
                    fallback,
                    clip,
                } => {
                    if clip.is_empty() || *w <= 0.0 || *h <= 0.0 {
                        continue;
                    }
                    let sigma = sigma.max(0.5);
                    let m = (3.0 * sigma).ceil();
                    let Some(region) = damage::bounds(*x, *y, *w, *h, m, &super::window::Clip::rect(0.0, 0.0, ww as f32, wh as f32), ww, wh) else {
                        continue;
                    };
                    let flags = clip_flags(clip) + if *continuous { 1.0 } else { 0.0 };
                    let at = p.insts.len();
                    let n = p.backdrops.len() as u32;
                    push(
                        &mut p,
                        Inst {
                            rect: [*x, *y, *w, *h],
                            clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                            fill: premul(*fallback),
                            border: [0.0; 4],
                            radii: *radii,
                            uv: [0.0; 4],
                            params: [6.0, 0.0, flags, sigma],
                            clip_radii: clip.radii,
                        },
                        BACKDROP + n,
                        1.0,
                        0x6261_636b,
                    );
                    if p.insts.len() > at {
                        p.backdrops.push((at, region, sigma));
                    }
                }
                Prim::Radial {
                    x,
                    y,
                    w,
                    h,
                    strip,
                    radii,
                    clip,
                    alpha,
                } => {
                    if clip.is_empty() || *w <= 0.0 || *h <= 0.0 || *alpha <= 0.0 {
                        continue;
                    }
                    let a = alpha.clamp(0.0, 1.0);
                    let src = PIC + p.pics.len() as u32;
                    p.pics.push(strip.clone());
                    push(
                        &mut p,
                        Inst {
                            rect: [*x, *y, *w, *h],
                            clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                            fill: [a, a, a, a],
                            border: [0.0; 4],
                            radii: *radii,
                            uv: [0.0, 0.0, 1.0, 1.0],
                            params: [5.0, 0.0, clip_flags(clip), 0.0],
                            clip_radii: clip.radii,
                        },
                        src,
                        1.0,
                        damage::mix(0x7261_6469, strip.id),
                    );
                }
                Prim::Path(pp) => {
                    let [bx, by, bw, bh] = pp.bounds();
                    let clip = &pp.clip;
                    if clip.is_empty() || !(bw >= 0.0 && bh >= 0.0) {
                        continue;
                    }
                    let Some(r) = damage::bounds(bx, by, bw, bh, 1.0, clip, ww, wh) else {
                        continue;
                    };
                    let parts = [
                        (vector::Part::Fill, pp.fill.filter(|c| c[3] > 0)),
                        (
                            vector::Part::Stroke,
                            pp.stroke.as_ref().filter(|st| st.width > 0.0).map(|st| st.color).filter(|c| c[3] > 0),
                        ),
                    ];
                    for (part, color) in parts {
                        let Some(color) = color else { continue };
                        let inst = Inst {
                            rect: [r.x0 as f32, r.y0 as f32, (r.x1 - r.x0) as f32, (r.y1 - r.y0) as f32],
                            clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                            fill: premul(color),
                            border: [0.0; 4],
                            radii: [0.0; 4],
                            uv: [0.0; 4],
                            params: [4.0, 0.0, clip_flags(clip), 0.0],
                            clip_radii: clip.radii,
                        };
                        let base = damage::mix(damage::mix(pp.base_hash(), part as u64), u32::from_le_bytes(color) as u64);
                        let item = Item::new(r, base, pp.x, pp.y, *clip)
                            .pinned(damage::crosses_window(bx, by, bw, bh, 1.0, ww, wh));
                        p.paths.push(PathJob {
                            prim: pp.clone(),
                            part,
                            inst: p.insts.len(),
                            rect: r,
                        });
                        p.items.push(item);
                        p.bounds.push(r);
                        p.insts.push(inst);
                        p.src.push(COVER);
                    }
                }
                Prim::Shadow {
                    x,
                    y,
                    w,
                    h,
                    radii,
                    sigma,
                    color,
                    clip,
                } => {
                    if clip.is_empty() || *w <= 0.0 || *h <= 0.0 || color[3] == 0 {
                        continue;
                    }
                    let sigma = sigma.max(0.5);
                    push(
                        &mut p,
                        Inst {
                            rect: [*x, *y, *w, *h],
                            clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                            fill: premul(*color),
                            border: [0.0; 4],
                            radii: *radii,
                            uv: [0.0; 4],
                            params: [3.0, 0.0, clip_flags(clip), sigma],
                            clip_radii: clip.radii,
                        },
                        ANY,
                        (3.0 * sigma).ceil() + 1.0,
                        0,
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
                    let Some(s) = atlas.place(&self.device, &self.queue, key, coords) else {
                        // no ink, or no room this frame
                        if !atlas.slots.contains_key(key) {
                            p.incomplete = true;
                        }
                        continue;
                    };
                    let gx = x.floor() + s.left as f32;
                    let gy = *y - s.top as f32;
                    push(
                        &mut p,
                        Inst {
                            rect: [gx, gy, s.w as f32, s.h as f32],
                            clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                            fill: premul(*color),
                            border: [0.0; 4],
                            radii: [0.0; 4],
                            uv: s.uv,
                            params: [if s.color { 2.0 } else { 1.0 }, 0.0, clip_flags(clip), 0.0],
                            clip_radii: clip.radii,
                        },
                        s.page as u32,
                        0.0,
                        s.ident,
                    );
                }
            }
        }
        p
    }

    /// The texture of a picture, uploaded once.
    fn picture_view(&self, pic: &Picture) -> wgpu::TextureView {
        let mut cache = self.pictures.lock().unwrap_or_else(|e| e.into_inner());
        cache.clock += 1;
        let now = cache.clock;
        if let Some(v) = cache.views.get_mut(&pic.id) {
            v.2 = now;
            return v.0.clone();
        }
        let size_of = pic.rgba.len();
        while cache.bytes + size_of > TEXTURE_BUDGET && !cache.views.is_empty() {
            let Some(old) = cache.views.iter().min_by_key(|(_, v)| v.2).map(|(k, _)| *k) else {
                break;
            };
            if let Some(v) = cache.views.remove(&old) {
                cache.bytes -= v.1;
                cache.evicted += 1;
            }
        }
        let size = wgpu::Extent3d {
            width: pic.width.max(1),
            height: pic.height.max(1),
            depth_or_array_layers: 1,
        };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui picture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pic.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pic.width * 4),
                rows_per_image: Some(pic.height),
            },
            size,
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        cache.views.insert(pic.id, (view.clone(), size_of, now));
        cache.bytes += size_of;
        view
    }

    fn bind(&self, ubuf: &wgpu::Buffer, view: &wgpu::TextureView, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    fn globals(&self, w: u32, h: u32) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let globals = Globals {
            size: [w as f32, h as f32],
            atlas: [ATLAS as f32, ATLAS as f32],
        };
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gui globals"),
                contents: bytemuck::bytes_of(&globals),
                usage: wgpu::BufferUsages::UNIFORM,
            })
    }

    /// Encode a prepared frame into `view` (of `format`): whole (cleared
    /// first), or only `damage`'s rectangles over what `view` holds.
    /// `readable`: `view`'s texture may be sampled (a material blurs what
    /// it holds; where it may not, a material draws its opaque fallback).
    fn encode(
        &self,
        enc: &mut wgpu::CommandEncoder,
        p: &Prepared,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        damage: &Damage,
        readable: bool,
    ) {
        use wgpu::util::DeviceExt;
        let c = premul(p.clear);
        // the instances to draw, and per scissor (none: the whole frame)
        // a clearing instance and the runs over it
        let mut insts: Vec<Inst> = Vec::new();
        let mut passes: Vec<(Option<IRect>, Runs)> = Vec::new();
        // the paths this frame draws: their coverage first, on the GPU
        let used: Vec<usize> = match damage {
            Damage::None => return,
            Damage::Full => (0..p.paths.len()).collect(),
            Damage::Rects(rects) => (0..p.paths.len())
                .filter(|j| rects.iter().any(|r| r.intersects(&p.paths[*j].rect)))
                .collect(),
        };
        let (mut covered, pages) = if used.is_empty() {
            (Covered::new(), Vec::new())
        } else {
            self.cover(enc, p, &used)
        };
        // the materials' blurred backdrops: where each instance reads its
        // texture (the window's pixels its texels span)
        let blurred = if readable && !p.backdrops.is_empty() {
            let need = p.backdrops.iter().fold((1, 1), |a, (_, r, sg)| {
                let (lw, lh) = blur_size(r, *sg);
                (a.0.max(lw), a.1.max(lh))
            });
            let (bv, tw, th) = self.blur_targets(need.0, need.1);
            for (n, (at, r, sg)) in p.backdrops.iter().enumerate() {
                // the quad's corners (its box grown a pixel) in the texture
                let k = blur_step(*sg) as f32;
                let [x, y, w, h] = p.insts[*at].rect;
                let (sx, sy) = (k * tw as f32, k * th as f32);
                let uv = [
                    (x - 1.0 - r.x0 as f32) / sx,
                    (y - 1.0 - r.y0 as f32) / sy,
                    (x + w + 1.0 - r.x0 as f32) / sx,
                    (y + h + 1.0 - r.y0 as f32) / sy,
                ];
                covered.insert(*at, (BACKDROP + n as u32, uv));
            }
            Some(bv)
        } else {
            None
        };

        match damage {
            Damage::None => return,
            Damage::Full => {
                let mut runs = Vec::new();
                push_runs(p, 0..p.insts.len(), &mut insts, &mut runs, &covered);
                passes.push((None, runs));
            }
            Damage::Rects(rects) => {
                // the instances touching each rectangle, in one pass
                let all = rects.iter().skip(1).fold(rects[0], |a, r| IRect {
                    x0: a.x0.min(r.x0),
                    y0: a.y0.min(r.y0),
                    x1: a.x1.max(r.x1),
                    y1: a.y1.max(r.y1),
                });
                let mut touching: Vec<Vec<usize>> = vec![Vec::new(); rects.len()];
                for (i, b) in p.bounds.iter().enumerate() {
                    if !b.intersects(&all) {
                        continue;
                    }
                    for (k, r) in rects.iter().enumerate() {
                        if b.intersects(r) {
                            touching[k].push(i);
                        }
                    }
                }
                for (r, touching) in rects.iter().zip(touching) {
                    // the rectangle's clear colour, replacing what was there
                    // (grown past it: the scissor cuts it exactly)
                    let at = insts.len() as u32;
                    insts.push(Inst {
                        rect: [
                            r.x0 as f32 - 2.0,
                            r.y0 as f32 - 2.0,
                            (r.x1 - r.x0) as f32 + 4.0,
                            (r.y1 - r.y0) as f32 + 4.0,
                        ],
                        clip: [-1.0e9, -1.0e9, 1.0e9, 1.0e9],
                        fill: c,
                        border: [0.0; 4],
                        radii: [0.0; 4],
                        uv: [0.0; 4],
                        params: [0.0; 4],
                        clip_radii: [0.0; 4],
                    });
                    let mut runs = vec![(at, at + 1, u32::MAX - 1)];
                    push_runs(p, touching.into_iter(), &mut insts, &mut runs, &covered);
                    passes.push((Some(*r), runs));
                }
            }
        }
        let ubuf = self.globals(p.width, p.height);
        let ibuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gui instances"),
                contents: if insts.is_empty() {
                    &[0u8; std::mem::size_of::<Inst>()]
                } else {
                    bytemuck::cast_slice(&insts)
                },
                usage: wgpu::BufferUsages::VERTEX,
            });
        // the bind groups the runs need
        let mut binds: HashMap<u32, wgpu::BindGroup> = HashMap::new();
        let mut atlas = self.atlas.lock().unwrap_or_else(|e| e.into_inner());
        if atlas.pages.is_empty() {
            atlas.pages.push(atlas_page(&self.device));
        }
        let page_view = |i: u32| &atlas.pages[(i as usize).min(atlas.pages.len() - 1)].view;
        for (_, runs) in &passes {
            for (_, _, s) in runs {
                let s = if *s >= u32::MAX - 1 { 0 } else { *s };
                if binds.contains_key(&s) {
                    continue;
                }
                let b = if s >= PIC {
                    let v = self.picture_view(&p.pics[(s - PIC) as usize]);
                    self.bind(&ubuf, &v, &self.linear)
                } else if s >= COVER {
                    self.bind(&ubuf, &pages[(s - COVER) as usize], &self.sampler)
                } else if s >= BACKDROP {
                    match &blurred {
                        Some(v) => self.bind(&ubuf, v, &self.linear),
                        None => continue,
                    }
                } else {
                    self.bind(&ubuf, page_view(s), &self.sampler)
                };
                binds.insert(s, b);
            }
        }
        drop(atlas);
        let blend = self.pipeline(format, true);
        let replace = self.pipeline(format, false);
        let load = match damage {
            Damage::Full => wgpu::LoadOp::Clear(wgpu::Color {
                r: c[0] as f64,
                g: c[1] as f64,
                b: c[2] as f64,
                a: c[3] as f64,
            }),
            _ => wgpu::LoadOp::Load,
        };
        let begin = |enc: &mut wgpu::CommandEncoder, load| {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("gui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            pass.set_vertex_buffer(0, ibuf.slice(..));
            pass
        };
        let mut pass = begin(enc, load);
        if insts.is_empty() {
            return;
        }
        for (scissor, runs) in &passes {
            if let Some(r) = scissor {
                pass.set_scissor_rect(r.x0 as u32, r.y0 as u32, (r.x1 - r.x0) as u32, (r.y1 - r.y0) as u32);
            }
            for (a, b, s) in runs {
                if (BACKDROP..COVER).contains(s) {
                    // a material reads what is drawn so far: the pass ends,
                    // the region under it is blurred, the pass goes on
                    drop(pass);
                    let (_, region, sigma) = p.backdrops[(*s - BACKDROP) as usize];
                    self.blur(enc, view, region, sigma);
                    pass = begin(enc, wgpu::LoadOp::Load);
                    if let Some(r) = scissor {
                        pass.set_scissor_rect(r.x0 as u32, r.y0 as u32, (r.x1 - r.x0) as u32, (r.y1 - r.y0) as u32);
                    }
                }
                if *s == u32::MAX - 1 {
                    pass.set_pipeline(&replace);
                } else {
                    pass.set_pipeline(&blend);
                }
                let s = if *s >= u32::MAX - 1 { 0 } else { *s };
                pass.set_bind_group(0, &binds[&s], &[]);
                pass.draw(0..4, *a..*b);
            }
        }
    }

    /// Draw a display list whole into a texture view of `format` (its
    /// texture sampled too: a material reads what is under it).
    pub fn draw(&self, dl: &DisplayList, view: &wgpu::TextureView, format: wgpu::TextureFormat) {
        let p = self.prepare(dl);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gui") });
        self.encode(&mut enc, &p, view, format, &Damage::Full, true);
        self.queue.submit([enc.finish()]);
    }

    /// Draw offscreen and read the pixels back (straight RGBA): the
    /// check that the GPU draws what the software renderer draws.
    pub fn render_rgba(&self, dl: &DisplayList) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let (w, h) = (dl.width.max(1), dl.height.max(1));
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui offscreen"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.draw(dl, &view, format);
        self.read_rgba(&tex, w, h)
    }

    /// A texture's pixels read back, straight RGBA (it must be RGBA8 and
    /// allow copying from).
    fn read_rgba(&self, tex: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gui") });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let Ok(data) = slice.get_mapped_range() else {
            return vec![0; (w * h * 4) as usize];
        };
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            for px in data[start..start + (w * 4) as usize].chunks(4) {
                // Premultiplied → straight.
                let a = px[3] as u32;
                if a == 0 {
                    out.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    let f = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
                    out.extend_from_slice(&[f(px[0]), f(px[1]), f(px[2]), px[3]]);
                }
            }
        }
        out
    }
}

/// A frame kept between frames, so the next draws only what changed.
pub struct Target {
    gpu: Arc<Gpu>,
    format: wgpu::TextureFormat,
    tex: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    tracker: Tracker,
    /// Where a shift's pixels wait between the copies (a texture is not
    /// copied onto itself).
    scratch: Option<(wgpu::Texture, u32, u32)>,
    /// The last frame's damage and moves (for a trace and for tests).
    pub last: Damage,
    pub last_shifts: Vec<Shift>,
}

impl Drop for Target {
    fn drop(&mut self) {
        self.release();
    }
}

impl Target {
    pub fn new(gpu: Arc<Gpu>, format: wgpu::TextureFormat) -> Target {
        Target {
            gpu,
            format,
            tex: None,
            tracker: Tracker::default(),
            scratch: None,
            last: Damage::None,
            last_shifts: Vec::new(),
        }
    }

    fn release(&mut self) {
        if let Some((_, _, w, h)) = self.tex.take() {
            self.gpu
                .targets
                .fetch_sub((w * h * 4) as usize, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some((_, w, h)) = self.scratch.take() {
            self.gpu
                .targets
                .fetch_sub((w * h * 4) as usize, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The next frame draws everything.
    pub fn invalidate(&mut self) {
        self.tracker.invalidate();
    }

    /// The texture, made (or remade) at this size.
    fn ensure(&mut self, w: u32, h: u32) {
        if matches!(self.tex, Some((_, _, tw, th)) if (tw, th) == (w, h)) {
            return;
        }
        self.release();
        let tex = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui frame"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .targets
            .fetch_add((w * h * 4) as usize, std::sync::atomic::Ordering::Relaxed);
        self.tex = Some((tex, view, w, h));
        self.tracker.invalidate();
    }

    /// What a prepared frame must move and draw here.
    fn damage_of(&mut self, p: &mut Prepared) -> (Damage, Vec<Shift>, damage::Next) {
        // a window past the device's largest texture is drawn cut
        let most = self.gpu.device.limits().max_texture_dimension_2d.max(1);
        let (w, h) = (p.width.min(most), p.height.min(most));
        self.ensure(w, h);
        let (d, shifts, next) = self.tracker.damage(w, h, p.clear, std::mem::take(&mut p.items), &p.scrolls);
        (with_backdrops(d, &p.backdrops), shifts, next)
    }

    /// Encode the frame's moves and damage into the kept texture.
    fn encode(&mut self, enc: &mut wgpu::CommandEncoder, p: &Prepared, d: &Damage, shifts: &[Shift]) {
        let Some((tw, th)) = self.tex.as_ref().map(|t| (t.2, t.3)) else {
            return;
        };
        if !shifts.is_empty() && !matches!(self.scratch, Some((_, w, h)) if (w, h) == (tw, th)) {
            if let Some((_, w, h)) = self.scratch.take() {
                self.gpu
                    .targets
                    .fetch_sub((w * h * 4) as usize, std::sync::atomic::Ordering::Relaxed);
            }
            let s = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gui frame moves"),
                size: wgpu::Extent3d {
                    width: tw,
                    height: th,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.gpu
                .targets
                .fetch_add((tw * th * 4) as usize, std::sync::atomic::Ordering::Relaxed);
            self.scratch = Some((s, tw, th));
        }
        let Some((tex, view, _, _)) = &self.tex else {
            return;
        };
        if let Some((scratch, _, _)) = &self.scratch {
            fn at<'a>(t: &'a wgpu::Texture, r: &IRect) -> wgpu::TexelCopyTextureInfo<'a> {
                wgpu::TexelCopyTextureInfo {
                    texture: t,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: r.x0 as u32,
                        y: r.y0 as u32,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                }
            }
            for s in shifts {
                let size = wgpu::Extent3d {
                    width: (s.src.x1 - s.src.x0) as u32,
                    height: (s.src.y1 - s.src.y0) as u32,
                    depth_or_array_layers: 1,
                };
                enc.copy_texture_to_texture(at(tex, &s.src), at(scratch, &s.src), size);
                enc.copy_texture_to_texture(at(scratch, &s.src), at(tex, &s.dst), size);
            }
        }
        self.gpu.encode(enc, p, view, self.format, d, true);
    }

    fn commit(&mut self, p: &Prepared, d: Damage, shifts: Vec<Shift>, next: damage::Next) {
        damage::count(&d, p.width, p.height);
        if p.incomplete {
            self.tracker.invalidate();
        } else {
            self.tracker.commit(next);
        }
        self.last = d;
        self.last_shifts = shifts;
    }

    /// Draw a display list over the kept frame (offscreen: tests, and the
    /// check that damage draws what a whole frame draws). Its damage.
    pub fn render(&mut self, dl: &DisplayList) -> Damage {
        let mut p = self.gpu.prepare(dl);
        if dl.redraw {
            self.tracker.invalidate();
        }
        let (d, shifts, next) = self.damage_of(&mut p);
        if let Some(n) = dl.flash {
            flash(&mut p, &d, n);
        }
        let mut enc = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gui") });
        self.encode(&mut enc, &p, &d, &shifts);
        self.gpu.queue.submit([enc.finish()]);
        self.commit(&p, d.clone(), shifts, next);
        d
    }

    /// The kept frame's pixels, straight RGBA (an RGBA8 target).
    pub fn rgba(&self) -> Vec<u8> {
        match &self.tex {
            Some((tex, _, w, h)) => self.gpu.read_rgba(tex, *w, *h),
            None => Vec::new(),
        }
    }
}

/// A window's surface on the shared GPU.
pub struct Surface {
    pub gpu: Arc<Gpu>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The frame kept between frames.
    target: Target,
    /// The swapchain takes a copy (else the frame is drawn onto it).
    copy: bool,
    /// The last `render` presented a frame (one that changed nothing is
    /// not presented).
    presented: bool,
}

impl Surface {
    pub fn new(
        window: Arc<winit::window::Window>,
        display: winit::event_loop::OwnedDisplayHandle,
        width: u32,
        height: u32,
    ) -> Result<Surface, String> {
        // The instance must be the GPU's own once it exists; the first
        // surface is made on a fresh instance that then becomes the GPU's.
        let made = GPU.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (surface, gpu) = match made.as_ref() {
            Some(Ok(g)) => {
                let s = g
                    .instance
                    .create_surface(window)
                    .map_err(|e| format!("creating a surface: {e}"))?;
                (s, g.clone())
            }
            Some(Err(e)) => return Err(e.clone()),
            None => {
                let instance = window_instance(display);
                let s = instance
                    .create_surface(window)
                    .map_err(|e| format!("creating a surface: {e}"))?;
                let g = gpu(Some(&s), Some(instance))?;
                (s, g)
            }
        };
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or("the surface supports no format")?;
        let copy = caps.usages.contains(wgpu::TextureUsages::COPY_DST);
        let wide = wide_gamut(&caps, format);
        let config = wgpu::SurfaceConfiguration {
            usage: if copy {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            },
            format,
            width: width.clamp(1, gpu.device.limits().max_texture_dimension_2d.max(1)),
            height: height.clamp(1, gpu.device.limits().max_texture_dimension_2d.max(1)),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            // Display P3 where the surface takes it: the layer is then
            // colour-matched (an sRGB token shows as that colour on a P3
            // screen, not stretched to the panel's gamut), the frame
            // converted as it is presented
            color_space: if wide { wgpu::SurfaceColorSpace::DisplayP3 } else { Default::default() },
        };
        surface.configure(&gpu.device, &config);
        let target = Target::new(gpu.clone(), format);
        Ok(Surface {
            gpu,
            surface,
            config,
            target,
            copy,
            presented: false,
        })
    }

    /// The largest side a surface may have on this device (a window
    /// spread past it is drawn cut, never configured past the limit).
    fn max_side(&self) -> u32 {
        self.gpu.device.limits().max_texture_dimension_2d.max(1)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let most = self.max_side();
        let (w, h) = (width.clamp(1, most), height.clamp(1, most));
        if (w, h) != (self.config.width, self.config.height) {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.gpu.device, &self.config);
        }
    }

    /// The device was lost: this surface moved to the GPU made in its
    /// place ([`recover`]) and configured on it.
    fn recover(&mut self) -> Result<(), String> {
        let g = recover(&self.gpu, Some(&self.surface)).map_err(|e| format!("lost ({e})"))?;
        let caps = self.surface.get_capabilities(&g.adapter);
        if !caps.formats.contains(&self.config.format)
            && let Some(f) = caps
                .formats
                .iter()
                .copied()
                .find(|f| !f.is_srgb())
                .or_else(|| caps.formats.first().copied())
        {
            self.config.format = f;
        }
        let most = g.device.limits().max_texture_dimension_2d.max(1);
        self.config.width = self.config.width.clamp(1, most);
        self.config.height = self.config.height.clamp(1, most);
        self.copy = caps.usages.contains(wgpu::TextureUsages::COPY_DST);
        self.config.color_space = if wide_gamut(&caps, self.config.format) {
            wgpu::SurfaceColorSpace::DisplayP3
        } else {
            Default::default()
        };
        self.config.usage = if self.copy {
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST
        } else {
            wgpu::TextureUsages::RENDER_ATTACHMENT
        };
        self.gpu = g;
        self.surface.configure(&self.gpu.device, &self.config);
        // the kept frame was the lost device's: a new one, drawn whole
        self.target = Target::new(self.gpu.clone(), self.config.format);
        self.target.invalidate();
        Ok(())
    }

    /// The next frame draws and presents everything (the window's pixels
    /// were lost: it was shown again, or its layer was remade).
    pub fn invalidate(&mut self) {
        self.target.invalidate();
    }

    /// Whether the last `render` presented a frame.
    pub fn presented(&self) -> bool {
        self.presented
    }

    /// The last frame's damage.
    pub fn last_damage(&self) -> &Damage {
        &self.target.last
    }

    pub fn render(&mut self, dl: &DisplayList) -> Result<(), String> {
        self.presented = false;
        if self.gpu.is_lost() {
            self.recover()?;
        }
        self.resize(dl.width, dl.height);
        let mut p = self.gpu.prepare(dl);
        if dl.redraw {
            self.target.tracker.invalidate();
        }
        let (d, shifts, next) = self.target.damage_of(&mut p);
        if let Some(n) = dl.flash {
            flash(&mut p, &d, n);
        }
        if d == Damage::None && shifts.is_empty() {
            if gpu_timed() {
                eprintln!("gui: gpu nothing drawn or presented");
            }
            self.target.commit(&p, d, shifts, next);
            return Ok(());
        }
        let mut frame = None;
        for _ in 0..2 {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(f)
                | wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                    frame = Some(f);
                    break;
                }
                // Hidden or busy: skip the frame and say so; the platform
                // loop draws again when the window shows (or retries).
                wgpu::CurrentSurfaceTexture::Timeout => return Err("timeout".to_string()),
                wgpu::CurrentSurfaceTexture::Occluded => return Err("occluded".to_string()),
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    self.surface.configure(&self.gpu.device, &self.config);
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    return Err("the window's surface refused the frame".to_string());
                }
            }
        }
        let Some(frame) = frame else {
            return Err("the window's surface is lost".to_string());
        };
        let mut enc = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gui") });
        self.target.encode(&mut enc, &p, &d, &shifts);
        if let Some((tex, view, w, h)) = &self.target.tex {
            let (w, h) = (*w, *h);
            if self.config.color_space == wgpu::SurfaceColorSpace::DisplayP3 {
                // the frame's sRGB in Display P3's numbers
                let out = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                self.gpu.to_p3(&mut enc, view, &out, self.config.format, w.min(frame.texture.width()), h.min(frame.texture.height()));
            } else if self.copy {
                enc.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &frame.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: w.min(frame.texture.width()),
                        height: h.min(frame.texture.height()),
                        depth_or_array_layers: 1,
                    },
                );
            } else {
                // drawn across as one picture, nearest, replacing
                let out = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                self.gpu.blit(&mut enc, view, &out, self.config.format, w, h);
            }
        }
        let timed = gpu_timed();
        let t0 = std::time::Instant::now();
        self.gpu.queue.submit([enc.finish()]);
        if timed {
            // measuring: wait for the GPU, and say what the frame drew
            let _ = self.gpu.device.poll(wgpu::PollType::wait_indefinitely());
            eprintln!(
                "gui: gpu {:.3} ms, drew {:.1}% ({} instances, {}, {} moved)",
                t0.elapsed().as_secs_f64() * 1000.0,
                100.0 * d.area(p.width, p.height) as f64 / (p.width as f64 * p.height as f64),
                p.insts.len(),
                match &d {
                    Damage::Full => "whole".to_string(),
                    Damage::Rects(r) => format!("{} rects", r.len()),
                    Damage::None => "nothing".to_string(),
                },
                shifts.len()
            );
        }
        self.gpu.queue.present(frame);
        self.target.commit(&p, d, shifts, next);
        self.presented = true;
        Ok(())
    }
}

/// `OLANG_GUI_GPU_TIME=1`: each frame waits for the GPU and says how long
/// it took and what it drew (measuring only: it costs the overlap).
fn gpu_timed() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| std::env::var_os("OLANG_GUI_GPU_TIME").is_some_and(|v| !v.is_empty() && v != "0"))
}

impl Gpu {
    /// Copy a frame onto another texture by drawing it (where the
    /// swapchain takes no copies): sampled nearest at pixel centres, so
    /// every pixel is the source's.
    fn blit(
        &self,
        enc: &mut wgpu::CommandEncoder,
        from: &wgpu::TextureView,
        to: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        w: u32,
        h: u32,
    ) {
        use wgpu::util::DeviceExt;
        let ubuf = self.globals(w, h);
        let inst = Inst {
            rect: [0.0, 0.0, w as f32, h as f32],
            clip: [0.0, 0.0, w as f32, h as f32],
            fill: [1.0; 4],
            border: [0.0; 4],
            radii: [0.0; 4],
            uv: [0.0, 0.0, 1.0, 1.0],
            params: [2.0, 0.0, 0.0, 0.0],
            clip_radii: [0.0; 4],
        };
        let ibuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gui blit"),
                contents: bytemuck::bytes_of(&inst),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let bind = self.bind(&ubuf, from, &self.sampler);
        let pipeline = self.pipeline(format, false);
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gui blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: to,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.set_vertex_buffer(0, ibuf.slice(..));
        pass.draw(0..4, 0..1);
    }
}

// ── vector paths: their coverage drawn on the GPU ───────────────────

/// The coverage of a frame's paths (vector.rs's meshes), drawn into atlas
/// pages: a fill's fan triangles counted into the stencil and covered, a
/// stroke's segments by their distance (their maximum: a join drawn
/// once), four samples a pixel, resolved into a page the frame samples.
const COVER_SHADER: &str = r#"
struct G { size: vec2<f32>, pad: vec2<f32> };
@group(0) @binding(0) var<uniform> g: G;

fn to_clip(p: vec2<f32>) -> vec4<f32> {
  return vec4<f32>(p / g.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

@vertex
fn vs_fan(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> {
  return to_clip(p);
}

@fragment
fn fs_none() -> @location(0) vec4<f32> {
  return vec4<f32>(0.0);
}

@vertex
fn vs_cover(@builtin(vertex_index) vi: u32, @location(0) rect: vec4<f32>) -> @builtin(position) vec4<f32> {
  let c = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
  return to_clip(mix(rect.xy, rect.zw, c));
}

@fragment
fn fs_one() -> @location(0) vec4<f32> {
  return vec4<f32>(1.0);
}

struct SegOut {
  @builtin(position) pos: vec4<f32>,
  // shaded at each sample: a sample's coverage is its quarter of the
  // pixel's, and segments meet by their maximum per sample (a sharp turn's
  // inside, where two bands each cover part of a pixel, stays whole)
  @location(0) @interpolate(perspective, sample) p: vec2<f32>,
  @location(1) @interpolate(flat) ab: vec4<f32>,
  @location(2) @interpolate(flat) hc: vec4<f32>,
};

fn seg_axes(a: vec2<f32>, b: vec2<f32>) -> vec3<f32> {
  let d = b - a;
  let l = length(d);
  let t = select(vec2<f32>(1.0, 0.0), d / max(l, 1e-6), l > 1e-6);
  return vec3<f32>(t, l);
}

@vertex
fn vs_seg(@builtin(vertex_index) vi: u32, @location(0) ab: vec4<f32>, @location(1) hc: vec4<f32>) -> SegOut {
  let a = ab.xy;
  let ax = seg_axes(a, ab.zw);
  let t = ax.xy;
  let n = vec2<f32>(-t.y, t.x);
  let reach = hc.x * 1.5 + 1.0;
  let side = hc.x + 1.0;
  let c = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
  let p = a + t * mix(-reach, ax.z + reach, c.x) + n * mix(-side, side, c.y);
  var o: SegOut;
  o.pos = to_clip(p);
  o.p = p;
  o.ab = ab;
  o.hc = hc;
  return o;
}

/// How much of a sample's footprint (half a pixel square) at distance
/// `w` from a band's middle the band of half width `hw` covers (a box
/// filter across it: a hairline is faint).
fn band(w: f32, hw: f32) -> f32 {
  return clamp((min(w + 0.25, hw) - max(w - 0.25, -hw)) * 2.0, 0.0, 1.0);
}

/// The coverage an end allows, `o` past it, `w` across: 0 butt, 1 round,
/// 2 square, 3 a join (round).
fn end_cover(o: f32, w: f32, hw: f32, cap: f32) -> f32 {
  if (cap < 0.5) {
    return band(w, hw) * clamp((0.25 - o) * 2.0, 0.0, 1.0);
  }
  if (cap > 1.5 && cap < 2.5) {
    return band(w, hw) * clamp((hw + 0.25 - o) * 2.0, 0.0, 1.0);
  }
  if (o <= 0.0) {
    return band(w, hw);
  }
  return band(length(vec2<f32>(o, w)), hw);
}

@fragment
fn fs_seg(v: SegOut) -> @location(0) vec4<f32> {
  let a = v.ab.xy;
  let ax = seg_axes(a, v.ab.zw);
  let t = ax.xy;
  let q = v.p - a;
  let u = dot(q, t);
  let w = abs(dot(q, vec2<f32>(-t.y, t.x)));
  let hw = v.hc.x;
  let c = min(end_cover(-u, w, hw, v.hc.y), end_cover(u - ax.z, w, hw, v.hc.z));
  return vec4<f32>(c);
}
"#;

/// Samples a pixel in the coverage pass.
const COVER_SAMPLES: u32 = 4;
const COVER_STENCIL: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;

/// The coverage pass's pipelines.
struct CoverKit {
    layout: wgpu::BindGroupLayout,
    fan: wgpu::RenderPipeline,
    cover_nonzero: wgpu::RenderPipeline,
    cover_even_odd: wgpu::RenderPipeline,
    seg: wgpu::RenderPipeline,
}

/// The coverage pass's textures: the multisampled target and stencil
/// (the size of the largest page), and the pages the frame samples.
#[derive(Default)]
pub(crate) struct CoverState {
    kit: Option<Arc<CoverKit>>,
    msaa: Option<(wgpu::TextureView, wgpu::TextureView, u32, u32)>,
    pages: Vec<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    bytes: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SegInst {
    ab: [f32; 4],
    hc: [f32; 4],
}

/// A page's draws: fan triangles (their vertices), covers (a slot's
/// rectangle), segments, and per path its slot (the scissor: a path
/// clipped reaches past it) and what of each it draws.
#[derive(Default)]
struct PageWork {
    fan: Vec<[f32; 2]>,
    rects: Vec<[f32; 4]>,
    segs: Vec<SegInst>,
    paths: Vec<PageDraw>,
}

struct PageDraw {
    slot: [u32; 4],
    fan: (u32, u32),
    /// Its cover's index in `rects` and whether it is even-odd.
    cover: Option<(u32, bool)>,
    segs: (u32, u32),
}

impl Gpu {
    fn cover_kit(&self, cs: &mut CoverState) -> Arc<CoverKit> {
        if let Some(k) = &cs.kit {
            return k.clone();
        }
        let d = &self.device;
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gui cover"),
            source: wgpu::ShaderSource::Wgsl(COVER_SHADER.into()),
        });
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gui cover"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gui cover"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let max = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Max,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Max,
            },
        };
        let face = |compare, pass_op| wgpu::StencilFaceState {
            compare,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op,
        };
        let stencil = |front, back, read_mask| wgpu::DepthStencilState {
            format: COVER_STENCIL,
            depth_write_enabled: None,
            depth_compare: None,
            stencil: wgpu::StencilState {
                front,
                back,
                read_mask,
                write_mask: 0xff,
            },
            bias: Default::default(),
        };
        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                    topology,
                    blend: Option<wgpu::BlendState>,
                    writes,
                    ds: wgpu::DepthStencilState| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::R8Unorm,
                        blend,
                        write_mask: writes,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(ds),
                multisample: wgpu::MultisampleState {
                    count: COVER_SAMPLES,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let fan_attrs = wgpu::vertex_attr_array![0 => Float32x2];
        let fan_buf = [Some(wgpu::VertexBufferLayout {
            array_stride: 8,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &fan_attrs,
        })];
        let rect_attrs = wgpu::vertex_attr_array![0 => Float32x4];
        let rect_buf = [Some(wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &rect_attrs,
        })];
        let seg_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
        let seg_buf = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SegInst>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &seg_attrs,
        })];
        use wgpu::{CompareFunction as C, StencilOperation as O};
        let fan = make(
            "gui cover fan",
            "vs_fan",
            "fs_none",
            &fan_buf,
            wgpu::PrimitiveTopology::TriangleList,
            None,
            wgpu::ColorWrites::empty(),
            stencil(face(C::Always, O::IncrementWrap), face(C::Always, O::DecrementWrap), 0xff),
        );
        let cover = |read| {
            make(
                "gui cover fill",
                "vs_cover",
                "fs_one",
                &rect_buf,
                wgpu::PrimitiveTopology::TriangleStrip,
                Some(max),
                wgpu::ColorWrites::ALL,
                stencil(face(C::NotEqual, O::Keep), face(C::NotEqual, O::Keep), read),
            )
        };
        let seg = make(
            "gui cover stroke",
            "vs_seg",
            "fs_seg",
            &seg_buf,
            wgpu::PrimitiveTopology::TriangleStrip,
            Some(max),
            wgpu::ColorWrites::ALL,
            stencil(face(C::Always, O::Keep), face(C::Always, O::Keep), 0xff),
        );
        let kit = Arc::new(CoverKit {
            layout,
            fan,
            cover_nonzero: cover(0xff),
            cover_even_odd: cover(0x01),
            seg,
        });
        cs.kit = Some(kit.clone());
        kit
    }

    /// The coverage of paths `used` (indices into `p.paths`), drawn into
    /// atlas pages: where each instance finds its coverage, and the pages.
    fn cover(&self, enc: &mut wgpu::CommandEncoder, p: &Prepared, used: &[usize]) -> (Covered, Vec<wgpu::TextureView>) {
        use wgpu::util::DeviceExt;
        let most = self.device.limits().max_texture_dimension_2d.max(16);
        let side = p.width.max(p.height).max(1024).min(most);
        let mut shelves = vector::Shelves::new(side);
        let mut places = Vec::with_capacity(used.len());
        for j in used {
            let r = &p.paths[*j].rect;
            let (w, h) = ((r.x1 - r.x0) as u32, (r.y1 - r.y0) as u32);
            places.push(shelves.place(w, h));
        }
        let ext = shelves.extents();
        let pw = ext.iter().map(|e| e.0).max().unwrap_or(1);
        let ph = ext.iter().map(|e| e.1).max().unwrap_or(1);
        let mut cs = self.cover.lock().unwrap_or_else(|e| e.into_inner());
        let kit = self.cover_kit(&mut cs);
        // the textures, grown to this frame's pages (never shrunk: the
        // next frame most likely needs as much)
        let grow = |have: Option<(u32, u32)>| match have {
            Some((w, h)) if w >= pw && h >= ph => None,
            Some((w, h)) => Some((w.max(pw), h.max(ph))),
            None => Some((pw, ph)),
        };
        if let Some((w, h)) = grow(cs.msaa.as_ref().map(|m| (m.2, m.3))) {
            let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
            let make = |format, label| {
                self.device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size,
                        mip_level_count: 1,
                        sample_count: COVER_SAMPLES,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            let color = make(wgpu::TextureFormat::R8Unorm, "gui cover samples");
            let stencil = make(COVER_STENCIL, "gui cover stencil");
            let old = cs.msaa.as_ref().map(|m| (m.2 * m.3) as usize * COVER_SAMPLES as usize * 2).unwrap_or(0);
            cs.bytes = cs.bytes + (w * h) as usize * COVER_SAMPLES as usize * 2 - old;
            cs.msaa = Some((color, stencil, w, h));
        }
        let (mw, mh) = cs.msaa.as_ref().map(|m| (m.2, m.3)).unwrap_or((pw, ph));
        while cs.pages.len() < ext.len() {
            cs.pages.push(new_page(&self.device, mw, mh));
            cs.bytes += (mw * mh) as usize;
        }
        for i in 0..ext.len() {
            if (cs.pages[i].2, cs.pages[i].3) != (mw, mh) {
                cs.bytes -= (cs.pages[i].2 * cs.pages[i].3) as usize;
                cs.pages[i] = new_page(&self.device, mw, mh);
                cs.bytes += (mw * mh) as usize;
            }
        }
        // each page's draws, in the page's pixels
        let mut work: Vec<PageWork> = (0..ext.len()).map(|_| PageWork::default()).collect();
        let mut covered = Covered::with_capacity(used.len());
        for (j, (page, ax, ay)) in used.iter().zip(&places) {
            let job = &p.paths[*j];
            let r = &job.rect;
            let (w, h) = (((r.x1 - r.x0) as u32).min(side), ((r.y1 - r.y0) as u32).min(side));
            let (fw, fh) = (mw as f32, mh as f32);
            covered.insert(
                job.inst,
                (
                    COVER + *page as u32,
                    [
                        *ax as f32 / fw,
                        *ay as f32 / fh,
                        (*ax + w) as f32 / fw,
                        (*ay + h) as f32 / fh,
                    ],
                ),
            );
            // the path's own origin, in the page
            let ox = job.prim.x - r.x0 as f32 + *ax as f32;
            let oy = job.prim.y - r.y0 as f32 + *ay as f32;
            let mesh = vector::mesh(&job.prim, job.part);
            let wk = &mut work[*page];
            let f0 = wk.fan.len() as u32;
            let mut cover = None;
            if !mesh.fan.is_empty() {
                wk.fan.extend(mesh.fan.iter().map(|q| [q[0] + ox, q[1] + oy]));
                cover = Some((wk.rects.len() as u32, mesh.even_odd));
                wk.rects.push([*ax as f32, *ay as f32, (*ax + w) as f32, (*ay + h) as f32]);
            }
            let s0 = wk.segs.len() as u32;
            wk.segs.extend(mesh.segs.iter().map(|sg| SegInst {
                ab: [sg.a[0] + ox, sg.a[1] + oy, sg.b[0] + ox, sg.b[1] + oy],
                hc: [sg.hw, sg.caps[0] as f32, sg.caps[1] as f32, 0.0],
            }));
            wk.paths.push(PageDraw {
                slot: [*ax, *ay, w, h],
                fan: (f0, wk.fan.len() as u32),
                cover,
                segs: (s0, wk.segs.len() as u32),
            });
        }
        let globals = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gui cover globals"),
            contents: bytemuck::cast_slice(&[mw as f32, mh as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui cover"),
            layout: &kit.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let Some((color, stencil, _, _)) = cs.msaa.as_ref() else {
            return (Covered::new(), Vec::new());
        };
        for (i, wk) in work.iter().enumerate() {
            let fan = (!wk.fan.is_empty()).then(|| {
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("gui cover fan"),
                    contents: bytemuck::cast_slice(&wk.fan),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            });
            let covers = (!wk.rects.is_empty()).then(|| {
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("gui cover rects"),
                    contents: bytemuck::cast_slice(&wk.rects),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            });
            let segs = (!wk.segs.is_empty()).then(|| {
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("gui cover segments"),
                    contents: bytemuck::cast_slice(&wk.segs),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gui cover"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    depth_slice: None,
                    resolve_target: Some(&cs.pages[i].1),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: stencil,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &bind, &[]);
            pass.set_stencil_reference(0);
            for d in &wk.paths {
                // a path drawn within its slot only (clipped, it reaches past it)
                let [x, y, w, h] = d.slot;
                if w == 0 || h == 0 {
                    continue;
                }
                pass.set_scissor_rect(x, y, w.min(mw - x), h.min(mh - y));
                if let (Some((k, eo)), Some(fan), Some(covers)) = (d.cover, &fan, &covers) {
                    // the fill counted into the stencil, then covered where it counts
                    pass.set_pipeline(&kit.fan);
                    pass.set_vertex_buffer(0, fan.slice(..));
                    pass.draw(d.fan.0..d.fan.1, 0..1);
                    pass.set_pipeline(if eo { &kit.cover_even_odd } else { &kit.cover_nonzero });
                    pass.set_vertex_buffer(0, covers.slice(..));
                    pass.draw(0..4, k..k + 1);
                }
                if let Some(segs) = &segs
                    && d.segs.1 > d.segs.0
                {
                    pass.set_pipeline(&kit.seg);
                    pass.set_vertex_buffer(0, segs.slice(..));
                    pass.draw(0..4, d.segs.0..d.segs.1);
                }
            }
        }
        let pages = cs.pages[..ext.len()].iter().map(|pg| pg.1.clone()).collect();
        (covered, pages)
    }
}

/// An atlas page of coverage, `w` × `h`.
fn new_page(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView, u32, u32) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gui cover page"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
    (t, v, w, h)
}

// ── materials: what is under a box, blurred ─────────────────────────

/// A material's blur: the window's pixels under it averaged in `k`-pixel
/// blocks, then a Gaussian across and down at that size.
const BLUR_SHADER: &str = r#"
struct B { origin: vec2<f32>, size: vec2<f32>, k: f32, sigma: f32, dir: vec2<f32> };
@group(0) @binding(0) var<uniform> b: B;
@group(0) @binding(1) var src: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  let x = f32((vi << 1u) & 2u);
  let y = f32(vi & 2u);
  return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_down(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
  let i = vec2<i32>(floor(pos.xy));
  let k = i32(b.k);
  let lo = vec2<i32>(b.origin);
  let hi = lo + vec2<i32>(b.size) - vec2<i32>(1);
  var sum = vec4<f32>(0.0);
  for (var dy = 0; dy < k; dy = dy + 1) {
    for (var dx = 0; dx < k; dx = dx + 1) {
      let q = clamp(lo + i * k + vec2<i32>(dx, dy), lo, hi);
      sum = sum + textureLoad(src, q, 0);
    }
  }
  return sum / f32(k * k);
}

@fragment
fn fs_blur(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
  let i = vec2<i32>(floor(pos.xy));
  let n = vec2<i32>(b.size);
  let r = min(i32(ceil(3.0 * b.sigma)), 64);
  let d = vec2<i32>(b.dir);
  var sum = vec4<f32>(0.0);
  var total = 0.0;
  for (var t = -r; t <= r; t = t + 1) {
    let w = exp(-f32(t * t) / (2.0 * b.sigma * b.sigma));
    let q = clamp(i + d * t, vec2<i32>(0), n - vec2<i32>(1));
    sum = sum + textureLoad(src, q, 0) * w;
    total = total + w;
  }
  return sum / total;
}
"#;

/// The block a material's blur averages first, by its sigma (device
/// pixels): the Gaussian after it is then a dozen taps or so.
fn blur_step(sigma: f32) -> u32 {
    if sigma < 2.5 {
        1
    } else if sigma < 6.0 {
        2
    } else if sigma < 14.0 {
        4
    } else {
        8
    }
}

/// The texels a material's blur of `region` takes.
fn blur_size(region: &IRect, sigma: f32) -> (u32, u32) {
    let k = blur_step(sigma);
    (
        ((region.x1 - region.x0).max(1) as u32).div_ceil(k),
        ((region.y1 - region.y0).max(1) as u32).div_ceil(k),
    )
}

/// One rectangle for a frame that changed what a material reads: every
/// rectangle and every material's region it meets, so what is under a
/// material is drawn again before it is read (and what it reads outside
/// the rectangle is the kept frame's, unchanged).
fn with_backdrops(d: Damage, backdrops: &[(usize, IRect, f32)]) -> Damage {
    let Damage::Rects(rects) = d else {
        return d;
    };
    if backdrops.is_empty() || !rects.iter().any(|r| backdrops.iter().any(|b| b.1.intersects(r))) {
        return Damage::Rects(rects);
    }
    let join = |a: IRect, b: &IRect| IRect::new(a.x0.min(b.x0), a.y0.min(b.y0), a.x1.max(b.x1), a.y1.max(b.y1));
    let mut u = rects.iter().skip(1).fold(rects[0], |a, r| join(a, r));
    loop {
        let before = u;
        for b in backdrops {
            if b.1.intersects(&u) {
                u = join(u, &b.1);
            }
        }
        if u == before {
            break;
        }
    }
    Damage::Rects(vec![u])
}

/// The blur's pipelines and its two textures.
#[derive(Default)]
pub(crate) struct BlurState {
    kit: Option<(wgpu::BindGroupLayout, wgpu::RenderPipeline, wgpu::RenderPipeline)>,
    tex: Option<[(wgpu::Texture, wgpu::TextureView); 2]>,
    size: (u32, u32),
}

impl Gpu {
    /// The blur's textures at least `w` × `h`: the one a material reads,
    /// and its size.
    fn blur_targets(&self, w: u32, h: u32) -> (wgpu::TextureView, u32, u32) {
        let mut b = self.blur.lock().unwrap_or_else(|e| e.into_inner());
        if b.tex.is_none() || b.size.0 < w || b.size.1 < h {
            let (nw, nh) = (w.max(b.size.0), h.max(b.size.1));
            let make = || {
                let t = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("gui material"),
                    size: wgpu::Extent3d { width: nw, height: nh, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let v = t.create_view(&wgpu::TextureViewDescriptor::default());
                (t, v)
            };
            b.tex = Some([make(), make()]);
            b.size = (nw, nh);
        }
        let v = b.tex.as_ref().map(|t| t[0].1.clone()).expect("made");
        (v, b.size.0, b.size.1)
    }

    /// Blur what `from` (the frame being drawn) holds in `region` into the
    /// blur's first texture: blocks averaged, then a Gaussian across, then
    /// down.
    fn blur(&self, enc: &mut wgpu::CommandEncoder, from: &wgpu::TextureView, region: IRect, sigma: f32) {
        use wgpu::util::DeviceExt;
        let mut b = self.blur.lock().unwrap_or_else(|e| e.into_inner());
        if b.kit.is_none() {
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("gui material"),
                source: wgpu::ShaderSource::Wgsl(BLUR_SHADER.into()),
            });
            let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gui material"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
            let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("gui material"),
                bind_group_layouts: &[Some(&layout)],
                ..Default::default()
            });
            let make = |fs: &str| {
                self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("gui material"),
                    layout: Some(&pl),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fs),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
            b.kit = Some((layout, make("fs_down"), make("fs_blur")));
        }
        let (Some((layout, down, blur)), Some(tex)) = (&b.kit, &b.tex) else {
            return;
        };
        let k = blur_step(sigma);
        let (lw, lh) = blur_size(&region, sigma);
        let (lw, lh) = (lw.min(b.size.0), lh.min(b.size.1));
        let sl = sigma / k as f32;
        let uniform = |origin: [f32; 2], size: [f32; 2], dir: [f32; 2]| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gui material"),
                contents: bytemuck::cast_slice(&[origin[0], origin[1], size[0], size[1], k as f32, sl, dir[0], dir[1]]),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        };
        let step = |enc: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, src: &wgpu::TextureView, dst: &wgpu::TextureView, u: &wgpu::Buffer| {
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gui material"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                ],
            });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gui material"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_viewport(0.0, 0.0, lw as f32, lh as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, 0, lw, lh);
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        };
        let rs = [(region.x1 - region.x0) as f32, (region.y1 - region.y0) as f32];
        let small = [lw as f32, lh as f32];
        step(enc, down, from, &tex[0].1, &uniform([region.x0 as f32, region.y0 as f32], rs, [0.0, 0.0]));
        step(enc, blur, &tex[0].1, &tex[1].1, &uniform([0.0, 0.0], small, [1.0, 0.0]));
        step(enc, blur, &tex[1].1, &tex[0].1, &uniform([0.0, 0.0], small, [0.0, 1.0]));
    }
}

// ── colour: the frame presented in Display P3 ───────────────────────

/// Whether a surface of `format` can be presented in Display P3 (Metal's
/// can: the layer is then colour-matched to the screen).
fn wide_gamut(caps: &wgpu::SurfaceCapabilities, format: wgpu::TextureFormat) -> bool {
    !std::env::var("OLANG_GUI_SRGB_SURFACE").is_ok_and(|v| !v.is_empty() && v != "0")
        && caps.color_spaces(format).contains(wgpu::SurfaceColorSpaces::DISPLAY_P3)
}

/// The frame (sRGB, premultiplied) in Display P3: decoded to linear light,
/// moved by the primaries' matrix, encoded again (P3 shares sRGB's curve).
/// An sRGB colour stays itself on any screen once the layer is matched.
const P3_SHADER: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  let x = f32((vi << 1u) & 2u);
  let y = f32(vi & 2u);
  return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
  return select(pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn to_encoded(c: vec3<f32>) -> vec3<f32> {
  return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), c * 12.92, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
  let c = textureLoad(src, vec2<i32>(floor(pos.xy)), 0);
  if (c.a <= 0.0) {
    return vec4<f32>(0.0);
  }
  let lin = to_linear(clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0)));
  let p3 = vec3<f32>(
    dot(lin, vec3<f32>(0.8224621, 0.1775380, 0.0)),
    dot(lin, vec3<f32>(0.0331941, 0.9668058, 0.0)),
    dot(lin, vec3<f32>(0.0170827, 0.0723974, 0.9105199)));
  return vec4<f32>(to_encoded(clamp(p3, vec3<f32>(0.0), vec3<f32>(1.0))) * c.a, c.a);
}
"#;

impl Gpu {
    /// Draw `from` onto `to` (of `format`) in Display P3's numbers.
    fn to_p3(&self, enc: &mut wgpu::CommandEncoder, from: &wgpu::TextureView, to: &wgpu::TextureView, format: wgpu::TextureFormat, w: u32, h: u32) {
        let mut kits = self.p3.lock().unwrap_or_else(|e| e.into_inner());
        if !kits.iter().any(|k| k.0 == format) {
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("gui p3"),
                source: wgpu::ShaderSource::Wgsl(P3_SHADER.into()),
            });
            let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gui p3"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
            let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("gui p3"),
                bind_group_layouts: &[Some(&layout)],
                ..Default::default()
            });
            let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("gui p3"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            kits.push((format, layout, pipe));
        }
        let Some((_, layout, pipe)) = kits.iter().find(|k| k.0 == format) else {
            return;
        };
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui p3"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(from),
            }],
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gui p3"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: to,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

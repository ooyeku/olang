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
use super::window::{DisplayList, Prim};
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
  // Shapes grow a pixel each way so their antialiased edge is drawn.
  let pad = select(0.0, 1.0, i.params.x < 0.5);
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

/// Coverage of a pixel by a rounded clip (1 inside a plain one).
fn clip_cover(v: VOut) -> f32 {
  if (v.clip_radii.x + v.clip_radii.y + v.clip_radii.z + v.clip_radii.w <= 0.0) {
    return 1.0;
  }
  let half = (v.clip.zw - v.clip.xy) * 0.5;
  let q = v.p - (v.clip.xy + half);
  let r = clamp(corner_radius(q, v.clip_radii), 0.0, min(half.x, half.y));
  return clamp(0.5 - sd_round_box(q, half, r), 0.0, 1.0);
}

@fragment
fn fs(v: VOut) -> @location(0) vec4<f32> {
  if (v.p.x < v.clip.x || v.p.y < v.clip.y || v.p.x >= v.clip.z || v.p.y >= v.clip.w) {
    discard;
  }
  let cover = clip_cover(v);
  let kind = v.params.x;
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
  let outer = clamp(0.5 - sd_round_box(q, half, r), 0.0, 1.0);
  let bw = v.params.y;
  var ring = vec4<f32>(0.0);
  if (bw > 0.0) {
    let ih = max(half - vec2<f32>(bw), vec2<f32>(0.0));
    let ir = clamp(max(r - bw, 0.0), 0.0, min(ih.x, ih.y));
    var inner = 0.0;
    if (ih.x > 0.0 && ih.y > 0.0) {
      inner = clamp(0.5 - sd_round_box(q, ih, ir), 0.0, 1.0);
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
        s
    }
}

/// The process's GPU's stats, when it has one (made by a window or a test).
pub fn stats() -> Option<Stats> {
    match GPU.get() {
        Some(Ok(g)) => Some(g.stats()),
        _ => None,
    }
}

static GPU: OnceLock<Result<Arc<Gpu>, String>> = OnceLock::new();

/// The process's GPU, made on first use. `compatible` is a surface the
/// adapter must be able to present to (the first window's).
pub fn gpu(
    compatible: Option<&wgpu::Surface<'_>>,
    instance: Option<wgpu::Instance>,
) -> Result<Arc<Gpu>, String> {
    GPU.get_or_init(|| make(compatible, instance).map(Arc::new))
        .clone()
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

/// Append instances `order` of `p` to `out`, in runs of one source.
fn push_runs(p: &Prepared, order: impl Iterator<Item = usize>, out: &mut Vec<Inst>, runs: &mut Runs) {
    let mut start = out.len() as u32;
    let mut cur = ANY;
    for i in order {
        let s = p.src[i];
        if s != ANY && s != cur {
            if cur != ANY && out.len() as u32 > start {
                runs.push((start, out.len() as u32, cur));
                start = out.len() as u32;
            }
            cur = s;
        }
        out.push(p.insts[i]);
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
            };
            let [x, y, w, h] = inst.rect;
            let Some(b) = damage::bounds(x, y, w, h, pad, &c, ww, wh) else {
                return;
            };
            let mut item = Item::new(b, inst_base(&inst, ident), x, y, c);
            if inst.params[0] < 0.5 {
                // a shape: one colour inside its border and corners
                let r = inst.radii.iter().fold(inst.params[1], |a, v| a.max(*v));
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
                            params: [2.0, 0.0, 0.0, 0.0],
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
                    clip,
                } => {
                    if clip.is_empty() || *w <= 0.0 || *h <= 0.0 {
                        continue;
                    }
                    push(
                        &mut p,
                        Inst {
                            rect: [*x, *y, *w, *h],
                            clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                            fill: premul(*fill),
                            border: premul(*border),
                            radii: *radii,
                            uv: [0.0; 4],
                            params: [0.0, *border_width, 0.0, 0.0],
                            clip_radii: clip.radii,
                        },
                        ANY,
                        1.0,
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
                            params: [if s.color { 2.0 } else { 1.0 }, 0.0, 0.0, 0.0],
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
    fn encode(
        &self,
        enc: &mut wgpu::CommandEncoder,
        p: &Prepared,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        damage: &Damage,
    ) {
        use wgpu::util::DeviceExt;
        let c = premul(p.clear);
        // the instances to draw, and per scissor (none: the whole frame)
        // a clearing instance and the runs over it
        let mut insts: Vec<Inst> = Vec::new();
        let mut passes: Vec<(Option<IRect>, Runs)> = Vec::new();
        match damage {
            Damage::None => return,
            Damage::Full => {
                let mut runs = Vec::new();
                push_runs(p, 0..p.insts.len(), &mut insts, &mut runs);
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
                    push_runs(p, touching.into_iter(), &mut insts, &mut runs);
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
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
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
        });
        if insts.is_empty() {
            return;
        }
        pass.set_vertex_buffer(0, ibuf.slice(..));
        for (scissor, runs) in &passes {
            if let Some(r) = scissor {
                pass.set_scissor_rect(r.x0 as u32, r.y0 as u32, (r.x1 - r.x0) as u32, (r.y1 - r.y0) as u32);
            }
            for (a, b, s) in runs {
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

    /// Draw a display list whole into a texture view of `format`.
    pub fn draw(&self, dl: &DisplayList, view: &wgpu::TextureView, format: wgpu::TextureFormat) {
        let p = self.prepare(dl);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gui") });
        self.encode(&mut enc, &p, view, format, &Damage::Full);
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
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
        self.tracker.damage(w, h, p.clear, std::mem::take(&mut p.items), &p.scrolls)
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
        self.gpu.encode(enc, p, view, self.format, d);
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
        let (d, shifts, next) = self.damage_of(&mut p);
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
        let (surface, gpu) = match GPU.get() {
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
            color_space: Default::default(),
        };
        surface.configure(&gpu.device, &config);
        let target = Target::new(gpu.clone(), format);
        Ok(Surface {
            gpu,
            surface,
            config,
            target,
            copy,
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

    /// The next frame draws and presents everything (the window's pixels
    /// were lost: it was shown again, or its layer was remade).
    pub fn invalidate(&mut self) {
        self.target.invalidate();
    }

    /// The last frame's damage.
    pub fn last_damage(&self) -> &Damage {
        &self.target.last
    }

    pub fn render(&mut self, dl: &DisplayList) -> Result<(), String> {
        if self.gpu.is_lost() {
            return Err("lost".to_string());
        }
        self.resize(dl.width, dl.height);
        let mut p = self.gpu.prepare(dl);
        let (d, shifts, next) = self.target.damage_of(&mut p);
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
            if self.copy {
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

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

use super::canvas::Picture;
use super::raster::{self, GlyphKey};
use super::text::Color;
use super::window::{DisplayList, Prim};

/// A run of instances from its first, sampling the atlas or a picture.
type Segment = (usize, Option<Arc<Picture>>);
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
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

const ATLAS: u32 = 2048;

#[derive(Clone, Copy)]
struct Slot {
    uv: [f32; 4],
    left: i32,
    top: i32,
    w: u32,
    h: u32,
    color: bool,
}

struct Atlas {
    texture: wgpu::Texture,
    slots: HashMap<GlyphKey, Option<Slot>>,
    x: u32,
    y: u32,
    row: u32,
}

impl Atlas {
    fn reset(&mut self) {
        self.slots.clear();
        self.x = 0;
        self.y = 0;
        self.row = 0;
    }

    /// Place a glyph; `None` when it has no ink, or the atlas is full
    /// (the caller resets and retries next frame).
    fn place(
        &mut self,
        queue: &wgpu::Queue,
        key: &GlyphKey,
        coords: &[i16],
    ) -> Result<Option<Slot>, ()> {
        if let Some(s) = self.slots.get(key) {
            return Ok(*s);
        }
        let Some(img) = raster::glyph(key, coords) else {
            self.slots.insert(key.clone(), None);
            return Ok(None);
        };
        let (w, h) = (img.width, img.height);
        if w + 1 > ATLAS || h + 1 > ATLAS {
            self.slots.insert(key.clone(), None);
            return Ok(None);
        }
        if self.x + w + 1 > ATLAS {
            self.x = 0;
            self.y += self.row + 1;
            self.row = 0;
        }
        if self.y + h + 1 > ATLAS {
            return Err(());
        }
        let (x, y) = (self.x, self.y);
        self.x += w + 1;
        self.row = self.row.max(h);
        let rgba: Vec<u8> = if img.color {
            img.data.clone()
        } else {
            img.data.iter().flat_map(|a| [255, 255, 255, *a]).collect()
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
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
        let slot = Slot {
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
        };
        self.slots.insert(key.clone(), Some(slot));
        Ok(Some(slot))
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
    pipelines: Mutex<HashMap<wgpu::TextureFormat, Arc<wgpu::RenderPipeline>>>,
    sampler: wgpu::Sampler,
    /// Pictures (canvases, images) are scaled, so they sample smoothly.
    linear: wgpu::Sampler,
    atlas: Mutex<Atlas>,
    /// A texture per picture, by the picture's id.
    pictures: Mutex<HashMap<u64, wgpu::TextureView>>,
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

/// An instance that can present to windows of `window`'s display
/// (Wayland needs the display; elsewhere it is harmless).
fn window_instance(window: &Arc<winit::window::Window>) -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
        Box::new(window.clone()),
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
        pictures: Mutex::new(HashMap::new()),
        atlas: Mutex::new(Atlas {
            texture,
            slots: HashMap::new(),
            x: 0,
            y: 0,
            row: 0,
        }),
    })
}

impl Gpu {
    fn pipeline(&self, format: wgpu::TextureFormat) -> Arc<wgpu::RenderPipeline> {
        let mut p = self.pipelines.lock().unwrap_or_else(|e| e.into_inner());
        p.entry(format)
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
                                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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

    /// The instances of a display list, placing glyphs in the atlas, and
    /// the segments to draw them in: each starts at an instance and samples
    /// the atlas (`None`) or a picture.
    fn instances(&self, dl: &DisplayList) -> (Vec<Inst>, Vec<Segment>) {
        let mut atlas = self.atlas.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::with_capacity(dl.prims.len());
        let mut segments: Vec<Segment> = vec![(0, None)];
        let mut reset = false;
        for prim in &dl.prims {
            match prim {
                Prim::Image {
                    x,
                    y,
                    w,
                    h,
                    pic,
                    clip,
                } => {
                    if clip.is_empty() || *w <= 0.0 || *h <= 0.0 {
                        continue;
                    }
                    segments.push((out.len(), Some(pic.clone())));
                    // Drawn as a colour glyph: premultiplied, sampled whole.
                    out.push(Inst {
                        rect: [*x, *y, *w, *h],
                        clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                        fill: [1.0, 1.0, 1.0, 1.0],
                        border: [0.0; 4],
                        radii: [0.0; 4],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        params: [2.0, 0.0, 0.0, 0.0],
                        clip_radii: clip.radii,
                    });
                    // Back to the atlas for what follows.
                    segments.push((out.len(), None));
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
                    out.push(Inst {
                        rect: [*x, *y, *w, *h],
                        clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                        fill: premul(*fill),
                        border: premul(*border),
                        radii: *radii,
                        uv: [0.0; 4],
                        params: [0.0, *border_width, 0.0, 0.0],
                        clip_radii: clip.radii,
                    });
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
                    let slot = match atlas.place(&self.queue, key, coords) {
                        Ok(s) => s,
                        Err(()) => {
                            // Full: start over once; glyphs already
                            // emitted this frame keep valid places until
                            // the reset is drawn over next frame.
                            if reset {
                                None
                            } else {
                                reset = true;
                                atlas.reset();
                                atlas.place(&self.queue, key, coords).ok().flatten()
                            }
                        }
                    };
                    let Some(s) = slot else {
                        continue;
                    };
                    let gx = x.floor() + s.left as f32;
                    let gy = *y - s.top as f32;
                    out.push(Inst {
                        rect: [gx, gy, s.w as f32, s.h as f32],
                        clip: [clip.x0, clip.y0, clip.x1, clip.y1],
                        fill: premul(*color),
                        border: [0.0; 4],
                        radii: [0.0; 4],
                        uv: s.uv,
                        params: [if s.color { 2.0 } else { 1.0 }, 0.0, 0.0, 0.0],
                        clip_radii: clip.radii,
                    });
                }
            }
        }
        (out, segments)
    }

    /// The texture of a picture, uploaded once.
    fn picture_view(&self, pic: &Picture) -> wgpu::TextureView {
        let mut cache = self.pictures.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = cache.get(&pic.id) {
            return v.clone();
        }
        if cache.len() > 64 {
            cache.clear();
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
        cache.insert(pic.id, view.clone());
        view
    }

    /// Draw a display list into a texture view of `format`.
    pub fn draw(&self, dl: &DisplayList, view: &wgpu::TextureView, format: wgpu::TextureFormat) {
        use wgpu::util::DeviceExt;
        let (insts, segments) = self.instances(dl);
        let globals = Globals {
            size: [dl.width as f32, dl.height as f32],
            atlas: [ATLAS as f32, ATLAS as f32],
        };
        let ubuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gui globals"),
                contents: bytemuck::bytes_of(&globals),
                usage: wgpu::BufferUsages::UNIFORM,
            });
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
        let atlas_view = {
            let a = self.atlas.lock().unwrap_or_else(|e| e.into_inner());
            a.texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let bind_for = |view: &wgpu::TextureView, sampler: &wgpu::Sampler| {
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
        };
        let atlas_bind = bind_for(&atlas_view, &self.sampler);
        // Each segment's instances, and the picture it samples if not the atlas.
        let mut draws: Vec<(std::ops::Range<u32>, Option<wgpu::BindGroup>)> = Vec::new();
        for (i, (start, pic)) in segments.iter().enumerate() {
            let end = segments.get(i + 1).map(|s| s.0).unwrap_or(insts.len());
            if end <= *start {
                continue;
            }
            let bind = pic
                .as_ref()
                .map(|p| bind_for(&self.picture_view(p), &self.linear));
            draws.push((*start as u32..end as u32, bind));
        }
        let pipeline = self.pipeline(format);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gui") });
        {
            let c = premul(dl.clear);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: c[0] as f64,
                            g: c[1] as f64,
                            b: c[2] as f64,
                            a: c[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if !insts.is_empty() {
                pass.set_pipeline(&pipeline);
                pass.set_vertex_buffer(0, ibuf.slice(..));
                for (range, bind) in &draws {
                    pass.set_bind_group(0, bind.as_ref().unwrap_or(&atlas_bind), &[]);
                    pass.draw(0..4, range.clone());
                }
            }
        }
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
                texture: &tex,
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

/// A window's surface on the shared GPU.
pub struct Surface {
    pub gpu: Arc<Gpu>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

impl Surface {
    pub fn new(
        window: Arc<winit::window::Window>,
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
                let instance = window_instance(&window);
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
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
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
        Ok(Surface {
            gpu,
            surface,
            config,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        if (w, h) != (self.config.width, self.config.height) {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.gpu.device, &self.config);
        }
    }

    pub fn render(&mut self, dl: &DisplayList) -> Result<(), String> {
        self.resize(dl.width, dl.height);
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
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu.draw(dl, &view, self.config.format);
        self.gpu.queue.present(frame);
        Ok(())
    }
}

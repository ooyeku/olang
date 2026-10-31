//! The renderer: a frame's parts drawn in order into an HDR target, then
//! tone mapped to an 8-bit frame.
//!
//! - **Swarms** are one instanced draw each, whose vertex shader reads the
//!   swarm's columns straight from its storage buffer (a show swarm's own;
//!   a play swarm's uploaded once a frame): glowing points, added.
//! - **Shapes** are signed distance fields (circle, box, capsule, ring,
//!   triangle; a line is a capsule), any size, rotated, rounded or
//!   outlined, antialiased by their distance, in one instanced draw per
//!   run of shapes; their colour may go past 1.0 to glow.
//! - **Sprites** are pictures (`gui`'s decoding: PNG, JPEG, WebP, GIF,
//!   SVG), rotated and tinted, one draw per picture.
//!
//! Shapes and sprites cover what is under them (premultiplied "over");
//! swarms add light. The camera (`center`, `scale`, `rot`) applies to all.

use std::collections::HashMap;
use std::sync::Arc;

/// The camera, shared by every pass: the target's size in pixels, the
/// world point at its centre, pixels per world unit, and its turn.
const VIEW: &str = r#"
struct V { size: vec2<f32>, center: vec2<f32>, scale: f32, rot: f32, pad0: f32, pad1: f32 };
fn to_clip(v: V, p: vec2<f32>) -> vec4<f32> {
    let c = cos(-v.rot); let s = sin(-v.rot);
    let q = p - v.center;
    let r = vec2(q.x * c - q.y * s, q.x * s + q.y * c) * v.scale;
    return vec4(r / (v.size * 0.5), 0.0, 1.0);
}
fn turn(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a); let s = sin(a);
    return vec2(p.x * c - p.y * s, p.x * s + p.y * c);
}
fn corner(i: u32) -> vec2<f32> {
    var c = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
                                vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    return c[i];
}
"#;

const POINTS: &str = r#"
struct D { cap: u32, n: u32, pos: u32, glow: i32, dead: i32, pad: u32, radius: f32, pad2: f32, color: vec4<f32> };
@group(0) @binding(0) var<storage, read> s: array<f32>;
@group(0) @binding(1) var<uniform> v: V;
@group(0) @binding(2) var<uniform> d: D;
struct O { @builtin(position) at: vec4<f32>, @location(0) q: vec2<f32>, @location(1) k: f32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> O {
    var o: O;
    let corner = corner(vi);
    var k = 1.0;
    if (d.glow >= 0) { k = s[u32(d.glow) * d.cap + i]; }
    if (d.dead >= 0 && s[u32(d.dead) * d.cap + i] > 0.5) { k = 0.0; }
    let p = vec2(s[d.pos * d.cap + i], s[(d.pos + 1u) * d.cap + i]);
    // the point's radius is in pixels, whatever the camera's zoom
    o.at = to_clip(v, p) + vec4(corner * d.radius / (v.size * 0.5), 0.0, 0.0);
    if (k <= 0.0) { o.at = vec4(2.0, 2.0, 2.0, 1.0); }
    o.q = corner;
    o.k = k;
    return o;
}
@fragment
fn fs(o: O) -> @location(0) vec4<f32> {
    let r2 = dot(o.q, o.q);
    if (r2 > 1.0) { discard; }
    // a hot core and a soft halo
    let core = exp(-r2 * 18.0);
    let halo = exp(-r2 * 4.0) * 0.35;
    let e = (core + halo) * o.k * d.color.a;
    return vec4(d.color.rgb * e, 0.0);
}
"#;

/// A shape: centre, half size, turn, kind, rounding, outline, colour
/// (premultiplied by nothing: straight, with `glow` folded into rgb).
pub const SHAPE_FLOATS: usize = 12;

const SHAPES: &str = r#"
struct S { c: vec2<f32>, half: vec2<f32>, rot: f32, kind: f32, round: f32, line: f32, color: vec4<f32> };
@group(0) @binding(0) var<storage, read> sh: array<S>;
@group(0) @binding(1) var<uniform> v: V;
struct O { @builtin(position) at: vec4<f32>, @location(0) q: vec2<f32>, @location(1) @interpolate(flat) i: u32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> O {
    let s = sh[i];
    // the quad holds the shape, its outline and a pixel for the edge
    let pad = s.line * 0.5 + 1.5 / v.scale;
    let q = corner(vi) * (s.half + vec2(pad));
    var o: O;
    o.at = to_clip(v, s.c + turn(q, s.rot));
    o.q = q;
    o.i = i;
    return o;
}
fn sd_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2(r);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}
fn sd_tri(p0: vec2<f32>, r: f32) -> f32 {
    // an equilateral triangle, point up, circumradius r
    let k = sqrt(3.0);
    var p = vec2(abs(p0.x) - r * 0.866, p0.y + r * 0.5);
    if (p.x + k * p.y > 0.0) { p = vec2(p.x - k * p.y, -k * p.x - p.y) / 2.0; }
    p.x -= clamp(p.x, -r * 1.732, 0.0);
    return -length(p) * sign(p.y);
}
@fragment
fn fs(o: O) -> @location(0) vec4<f32> {
    let s = sh[o.i];
    let p = o.q;
    var d = 0.0;
    let kind = u32(s.kind + 0.5);
    if (kind == 0u) { d = length(p) - s.half.x; }                       // circle
    else if (kind == 1u) { d = sd_box(p, s.half, min(s.round, min(s.half.x, s.half.y))); } // box
    else if (kind == 2u) {                                               // capsule along x
        let r = s.half.y;
        let x = clamp(p.x, -(s.half.x - r), s.half.x - r);
        d = length(p - vec2(x, 0.0)) - r;
    }
    else if (kind == 3u) { d = sd_tri(p, s.half.x); }                    // triangle
    if (s.line > 0.0) { d = abs(d) - s.line * 0.5; }
    // coverage from the distance in pixels
    let cov = clamp(0.5 - d * v.scale, 0.0, 1.0) * s.color.a;
    if (cov <= 0.0) { discard; }
    return vec4(s.color.rgb * cov, cov);
}
"#;

/// A sprite: centre, half size, turn, alpha, tint (rgba).
pub const SPRITE_FLOATS: usize = 12;

const SPRITES: &str = r#"
struct P { c: vec2<f32>, half: vec2<f32>, rot: f32, pad0: f32, pad1: f32, pad2: f32, tint: vec4<f32> };
@group(0) @binding(0) var<storage, read> sp: array<P>;
@group(0) @binding(1) var<uniform> v: V;
@group(0) @binding(2) var pic: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;
struct O { @builtin(position) at: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) @interpolate(flat) i: u32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> O {
    let s = sp[i];
    let c = corner(vi);
    var o: O;
    o.at = to_clip(v, s.c + turn(c * s.half, s.rot));
    o.uv = vec2(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    o.i = i;
    return o;
}
@fragment
fn fs(o: O) -> @location(0) vec4<f32> {
    // pictures are premultiplied; the texture is sRGB, so this is linear
    let t = sp[o.i].tint;
    let c = textureSample(pic, smp, o.uv);
    return vec4(c.rgb * t.rgb, c.a) * t.a;
}
"#;

const TONE: &str = r#"
struct T { bg: vec4<f32>, exposure: f32, vignette: f32, pad0: f32, pad1: f32 };
@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var<uniform> t: T;
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0))[vi];
    return vec4(p, 0.0, 1.0);
}
// AgX-style: a log encoding and a fitted sigmoid, so bright colours
// desaturate toward white instead of clipping into flat hues.
fn agx(c: vec3<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(vec3(0.842, 0.0423, 0.0424), vec3(0.0784, 0.878, 0.0784), vec3(0.0792, 0.0792, 0.879));
    var x = m * max(c, vec3(1e-10));
    x = clamp((log2(x) + 12.47) / 16.5, vec3(0.0), vec3(1.0));
    let x2 = x * x; let x4 = x2 * x2;
    var y = 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
    let mi = mat3x3<f32>(vec3(1.197, -0.0528, -0.0529), vec3(-0.0980, 1.152, -0.0980), vec3(-0.0990, -0.0990, 1.152));
    return clamp(mi * y, vec3(0.0), vec3(1.0));
}
@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(hdr));
    let c = textureLoad(hdr, vec2<i32>(at.xy), 0).rgb * t.exposure + t.bg.rgb;
    let uv = at.xy / dim - 0.5;
    let v = 1.0 - t.vignette * dot(uv, uv) * 1.6;
    // AgX's output is display-encoded already
    return vec4(agx(c) * v, 1.0);
}
"#;

/// A swarm's part of a frame.
pub struct Layer<'a> {
    pub buf: &'a wgpu::Buffer,
    pub cap: usize,
    pub n: usize,
    pub pos: usize,
    pub glow: Option<usize>,
    pub dead: Option<usize>,
    pub color: [f32; 4],
    pub radius: f32,
}

/// One part of a frame, drawn in order.
pub enum Part<'a> {
    Swarm(Layer<'a>),
    /// `SHAPE_FLOATS` per shape.
    Shapes(Vec<f32>),
    /// Runs of one picture: `SPRITE_FLOATS` per sprite.
    Sprites(Vec<(Arc<crate::stdlib::gui::canvas::Picture>, Vec<f32>)>),
}

pub struct View {
    pub size: (u32, u32),
    pub center: (f32, f32),
    pub scale: f32,
    pub rot: f32,
    pub bg: [f32; 3],
    pub exposure: f32,
    pub vignette: f32,
}

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A live view's HDR target and its output, shown by `gui` as a picture.
struct Target {
    size: (u32, u32),
    hdr: wgpu::Texture,
    out: wgpu::Texture,
}

pub struct Renderer {
    gpu: Arc<crate::stdlib::gui::gpu::Gpu>,
    points: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    shapes: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    sprites: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    tone: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    sampler: wgpu::Sampler,
    /// A live view's textures by its name, kept while its size holds.
    targets: HashMap<String, Target>,
    /// Sprites' textures by picture id.
    pictures: HashMap<u64, wgpu::TextureView>,
}

fn entry(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty,
        count: None,
    }
}

fn storage() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn texture() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        let gpu = crate::stdlib::gui::gpu::gpu(None, None)?;
        let d = &gpu.device;
        let (v, f, vf) = (
            wgpu::ShaderStages::VERTEX,
            wgpu::ShaderStages::FRAGMENT,
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
            d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let pipe = |label: &str,
                    src: String,
                    layout: wgpu::BindGroupLayout,
                    format,
                    blend|
         -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(&layout)],
                ..Default::default()
            });
            let p = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            (p, layout)
        };
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let over = wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING;
        let points = pipe(
            "toad points",
            format!("{VIEW}{POINTS}"),
            layout("toad points", &[entry(0, v, storage()), entry(1, vf, uniform()), entry(2, vf, uniform())]),
            HDR,
            Some(add),
        );
        let shapes = pipe(
            "toad shapes",
            format!("{VIEW}{SHAPES}"),
            layout("toad shapes", &[entry(0, vf, storage()), entry(1, vf, uniform())]),
            HDR,
            Some(over),
        );
        let sprites = pipe(
            "toad sprites",
            format!("{VIEW}{SPRITES}"),
            layout(
                "toad sprites",
                &[
                    entry(0, vf, storage()),
                    entry(1, vf, uniform()),
                    entry(2, f, texture()),
                    entry(3, f, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                ],
            ),
            HDR,
            Some(over),
        );
        let tone = pipe(
            "toad tone",
            TONE.to_string(),
            layout(
                "toad tone",
                &[
                    entry(
                        0,
                        f,
                        wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                    ),
                    entry(1, f, uniform()),
                ],
            ),
            OUT,
            None,
        );
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("toad sprites"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Renderer {
            gpu,
            points,
            shapes,
            sprites,
            tone,
            sampler,
            targets: HashMap::new(),
            pictures: HashMap::new(),
        })
    }

    pub fn gpu(&self) -> &Arc<crate::stdlib::gui::gpu::Gpu> {
        &self.gpu
    }

    fn buffer(&self, label: &str, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        wgpu::util::DeviceExt::create_buffer_init(
            &self.gpu.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage,
            },
        )
    }

    /// A sprite's texture, uploaded once per picture (sRGB, premultiplied).
    fn picture(&mut self, pic: &crate::stdlib::gui::canvas::Picture) -> wgpu::TextureView {
        if let Some(v) = self.pictures.get(&pic.id) {
            return v.clone();
        }
        let d = &self.gpu.device;
        let size = wgpu::Extent3d {
            width: pic.width.max(1),
            height: pic.height.max(1),
            depth_or_array_layers: 1,
        };
        let tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("toad sprite"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        if pic.rgba.len() >= (pic.width * pic.height * 4) as usize {
            self.gpu.queue.write_texture(
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
        }
        let view = tex.create_view(&Default::default());
        // a game's sprites are few; a long run of new pictures starts over
        if self.pictures.len() > 4096 {
            self.pictures.clear();
        }
        self.pictures.insert(pic.id, view.clone());
        view
    }

    /// Draw a frame. A `live` one goes to that name's textures and is shown
    /// by `gui` (an image node with the source `("live", name)`), queued
    /// and not waited for; any other is made, waited for, and its pixels
    /// answered when `read`.
    pub fn frame(&mut self, v: &View, parts: &[Part<'_>], read: bool, live: Option<&str>) -> Option<Vec<u8>> {
        let (w, h) = v.size;
        let ext = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let make = |d: &wgpu::Device| {
            let tex = |format, usage| {
                d.create_texture(&wgpu::TextureDescriptor {
                    label: Some("toad frame"),
                    size: ext,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            (
                tex(
                    HDR,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                ),
                tex(
                    OUT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                ),
            )
        };
        let (hdr, out) = match live {
            Some(name) => match self.targets.get(name).filter(|t| t.size == (w, h)) {
                Some(t) => (t.hdr.clone(), t.out.clone()),
                None => {
                    let (hdr, out) = make(&self.gpu.device);
                    self.targets.insert(
                        name.to_string(),
                        Target {
                            size: (w, h),
                            hdr: hdr.clone(),
                            out: out.clone(),
                        },
                    );
                    (hdr, out)
                }
            },
            None => make(&self.gpu.device),
        };
        // every sprite's texture first (uploads need `&mut self`)
        let mut sprite_views = Vec::new();
        for p in parts {
            if let Part::Sprites(runs) = p {
                for (pic, _) in runs {
                    let view = self.picture(pic);
                    sprite_views.push(view);
                }
            }
        }
        let d = &self.gpu.device;
        let hv = hdr.create_view(&Default::default());
        let ov = out.create_view(&Default::default());
        let vu: [f32; 8] = [w as f32, h as f32, v.center.0, v.center.1, v.scale, v.rot, 0.0, 0.0];
        let vbuf = self.buffer("toad view", bytemuck::cast_slice(&vu), wgpu::BufferUsages::UNIFORM);
        // the bind groups, in draw order: (pipeline, bind group, vertices, instances)
        let mut draws: Vec<(&wgpu::RenderPipeline, wgpu::BindGroup, u32, u32)> = Vec::new();
        let mut views = sprite_views.into_iter();
        for p in parts {
            match p {
                Part::Swarm(l) => {
                    let mut u = [0u32; 12];
                    u[0] = l.cap as u32;
                    u[1] = l.n as u32;
                    u[2] = l.pos as u32;
                    u[3] = l.glow.map(|g| g as i32).unwrap_or(-1) as u32;
                    u[4] = l.dead.map(|g| g as i32).unwrap_or(-1) as u32;
                    u[6] = l.radius.to_bits();
                    for i in 0..4 {
                        u[8 + i] = l.color[i].to_bits();
                    }
                    let ub = self.buffer("toad points", bytemuck::cast_slice(&u), wgpu::BufferUsages::UNIFORM);
                    let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("toad points"),
                        layout: &self.points.1,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: l.buf.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: vbuf.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: ub.as_entire_binding(),
                            },
                        ],
                    });
                    draws.push((&self.points.0, bg, 6, l.n as u32));
                }
                Part::Shapes(inst) if !inst.is_empty() => {
                    let sb = self.buffer("toad shapes", bytemuck::cast_slice(inst), wgpu::BufferUsages::STORAGE);
                    let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("toad shapes"),
                        layout: &self.shapes.1,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: sb.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: vbuf.as_entire_binding(),
                            },
                        ],
                    });
                    draws.push((&self.shapes.0, bg, 6, (inst.len() / SHAPE_FLOATS) as u32));
                }
                Part::Sprites(runs) => {
                    for (_, inst) in runs {
                        let view = views.next().expect("a view per run");
                        if inst.is_empty() {
                            continue;
                        }
                        let sb = self.buffer("toad sprites", bytemuck::cast_slice(inst), wgpu::BufferUsages::STORAGE);
                        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("toad sprites"),
                            layout: &self.sprites.1,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: sb.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: vbuf.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::TextureView(&view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                                },
                            ],
                        });
                        draws.push((&self.sprites.0, bg, 6, (inst.len() / SPRITE_FLOATS) as u32));
                    }
                }
                Part::Shapes(_) => {}
            }
        }
        let mut enc = d.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("toad frame"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("toad draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            for (pipe, bg, verts, n) in &draws {
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, bg, &[]);
                pass.draw(0..*verts, 0..*n);
            }
        }
        let tu: [f32; 8] = [v.bg[0], v.bg[1], v.bg[2], 1.0, v.exposure, v.vignette, 0.0, 0.0];
        let tub = self.buffer("toad tone", bytemuck::cast_slice(&tu), wgpu::BufferUsages::UNIFORM);
        let tbg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("toad tone"),
            layout: &self.tone.1,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hv),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tub.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("toad tone"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ov,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.tone.0);
            pass.set_bind_group(0, &tbg, &[]);
            pass.draw(0..3, 0..1);
        }
        let row = (w * 4).div_ceil(256) * 256;
        let rb = read.then(|| {
            let rb = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("toad frame read"),
                size: (row * h) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &out,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &rb,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(h),
                    },
                },
                ext,
            );
            rb
        });
        self.gpu.queue.submit([enc.finish()]);
        if let Some(name) = live {
            crate::stdlib::gui::live::publish(name, ov, w, h);
        }
        let Some(rb) = rb else {
            if live.is_none() {
                let _ = d.poll(wgpu::PollType::wait_indefinitely());
            }
            return None;
        };
        let slice = rb.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = d.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().ok()?;
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let at = (y * row) as usize;
            px.extend_from_slice(&data[at..at + (w * 4) as usize]);
        }
        Some(px)
    }
}

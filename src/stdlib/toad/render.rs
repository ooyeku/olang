//! T0's renderer: swarms drawn as glowing points into an HDR target, then
//! tone mapped to an 8-bit frame. Each swarm is one instanced draw whose
//! vertex shader reads the swarm's columns straight from its storage
//! buffer (a show swarm's own; a play swarm's uploaded once a frame), so
//! drawing 1M points costs one draw call and no copies on the CPU.

use std::sync::Arc;

const DRAW: &str = r#"
struct D {
    size: vec2<f32>, center: vec2<f32>,
    scale: f32, radius: f32, cap: u32, n: u32,
    color: vec4<f32>,
    pos: u32, glow: i32, dead: i32, pad: u32,
};
@group(0) @binding(0) var<storage, read> s: array<f32>;
@group(0) @binding(1) var<uniform> d: D;
struct O { @builtin(position) at: vec4<f32>, @location(0) q: vec2<f32>, @location(1) k: f32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> O {
    var o: O;
    let corner = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
                                     vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0))[vi];
    var k = 1.0;
    if (d.glow >= 0) { k = s[u32(d.glow) * d.cap + i]; }
    if (d.dead >= 0 && s[u32(d.dead) * d.cap + i] > 0.5) { k = 0.0; }
    let p = vec2(s[d.pos * d.cap + i], s[(d.pos + 1u) * d.cap + i]);
    let px = (p - d.center) * d.scale + corner * d.radius;
    o.at = vec4(px / (d.size * 0.5) * vec2(1.0, 1.0), 0.0, 1.0);
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

pub struct Renderer {
    gpu: Arc<crate::stdlib::gui::gpu::Gpu>,
    draw: wgpu::RenderPipeline,
    draw_layout: wgpu::BindGroupLayout,
    tone: wgpu::RenderPipeline,
    tone_layout: wgpu::BindGroupLayout,
}

/// One swarm's part of a frame.
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

pub struct View {
    pub size: (u32, u32),
    pub center: (f32, f32),
    pub scale: f32,
    pub bg: [f32; 3],
    pub exposure: f32,
    pub vignette: f32,
}

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn uniform_entry(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        let gpu = crate::stdlib::gui::gpu::gpu(None, None)?;
        let d = &gpu.device;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let draw_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("toad draw"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                uniform_entry(1, vf),
            ],
        });
        let tone_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("toad tone"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                uniform_entry(1, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let pipe = |label: &str, src: &str, layout: &wgpu::BindGroupLayout, format, blend| {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                ..Default::default()
            });
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
            })
        };
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let draw = pipe("toad draw", DRAW, &draw_layout, HDR, Some(add));
        let tone = pipe("toad tone", TONE, &tone_layout, OUT, None);
        Ok(Renderer {
            gpu,
            draw,
            draw_layout,
            tone,
            tone_layout,
        })
    }

    pub fn gpu(&self) -> &Arc<crate::stdlib::gui::gpu::Gpu> {
        &self.gpu
    }

    /// Draw a frame and wait for it; answers the frame's pixels when `read`.
    pub fn frame(&self, v: &View, layers: &[Layer<'_>], read: bool) -> Option<Vec<u8>> {
        let d = &self.gpu.device;
        let (w, h) = v.size;
        let ext = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
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
        let hdr = tex(
            HDR,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let out = tex(
            OUT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let hv = hdr.create_view(&Default::default());
        let ov = out.create_view(&Default::default());
        let mut enc = d.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("toad frame"),
        });
        let binds: Vec<(wgpu::BindGroup, u32)> = layers
            .iter()
            .map(|l| {
                let mut u = [0u32; 16];
                let f = |x: f32| x.to_bits();
                u[0] = f(w as f32);
                u[1] = f(h as f32);
                u[2] = f(v.center.0);
                u[3] = f(v.center.1);
                u[4] = f(v.scale);
                u[5] = f(l.radius);
                u[6] = l.cap as u32;
                u[7] = l.n as u32;
                for i in 0..4 {
                    u[8 + i] = f(l.color[i]);
                }
                u[12] = l.pos as u32;
                u[13] = l.glow.map(|g| g as i32).unwrap_or(-1) as u32;
                u[14] = l.dead.map(|g| g as i32).unwrap_or(-1) as u32;
                let ub = wgpu::util::DeviceExt::create_buffer_init(
                    d,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("toad draw"),
                        contents: bytemuck::cast_slice(&u),
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                );
                let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("toad draw"),
                    layout: &self.draw_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: l.buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: ub.as_entire_binding(),
                        },
                    ],
                });
                (bg, l.n as u32)
            })
            .collect();
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
            pass.set_pipeline(&self.draw);
            for (bg, n) in &binds {
                pass.set_bind_group(0, bg, &[]);
                pass.draw(0..6, 0..*n);
            }
        }
        let tu: [f32; 8] = [v.bg[0], v.bg[1], v.bg[2], 1.0, v.exposure, v.vignette, 0.0, 0.0];
        let tub = wgpu::util::DeviceExt::create_buffer_init(
            d,
            &wgpu::util::BufferInitDescriptor {
                label: Some("toad tone"),
                contents: bytemuck::cast_slice(&tu),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let tbg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("toad tone"),
            layout: &self.tone_layout,
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
            pass.set_pipeline(&self.tone);
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
        let Some(rb) = rb else {
            let _ = d.poll(wgpu::PollType::wait_indefinitely());
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

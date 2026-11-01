//! A kernel run on the GPU: the program written out as a WGSL compute
//! shader over one storage buffer (the swarm's columns, column-major) and
//! a uniform block (`dt` and the swarm's uniforms). This is the path for
//! show swarms: hundreds of thousands of entities for what the GPU costs.

use super::kernel::{B, Op, Program, U};
use std::fmt::Write;

/// The uniforms' room: vec4s in the uniform block.
pub const UNI_VEC4: usize = 16;

fn f(c: f32) -> String {
    if c.is_finite() {
        format!("{:?}", c)
    } else if c.is_nan() {
        "(0.0 / 0.0)".to_string()
    } else if c > 0.0 {
        "3.4e38".to_string()
    } else {
        "-3.4e38".to_string()
    }
}

/// The compute shader for `p`.
pub fn source(p: &Program) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "// toad kernel {}", p.name);
    let _ = writeln!(
        s,
        "struct W {{ n: u32, cap: u32, pad0: u32, pad1: u32, u: array<vec4<f32>, {}> }};",
        UNI_VEC4
    );
    s.push_str("@group(0) @binding(0) var<storage, read_write> d: array<f32>;\n");
    s.push_str("@group(0) @binding(1) var<uniform> w: W;\n");
    s.push_str("fn sel(c: f32, a: f32, b: f32) -> f32 { return select(b, a, c > 0.5); }\n");
    s.push_str("fn t(c: bool) -> f32 { return select(0.0, 1.0, c); }\n");
    s.push_str("@compute @workgroup_size(256)\n");
    s.push_str("fn main(@builtin(global_invocation_id) g: vec3<u32>) {\n");
    s.push_str("    let i = g.x;\n    if (i >= w.n) { return; }\n    let c = w.cap;\n");
    let r = |j: usize| format!("r{}", j);
    for (i, op) in p.ops.iter().enumerate() {
        let e = match *op {
            Op::Const(c) => f(c),
            Op::Col(col) => format!("d[{}u * c + i]", col),
            Op::Uni(u) => format!("w.u[{}][{}]", u / 4, u % 4),
            Op::Bin(b, x, y) => {
                let (x, y) = (r(x), r(y));
                match b {
                    B::Add => format!("{} + {}", x, y),
                    B::Sub => format!("{} - {}", x, y),
                    B::Mul => format!("{} * {}", x, y),
                    B::Div => format!("{} / {}", x, y),
                    B::Rem => format!("{} % {}", x, y),
                    B::Lt => format!("t({} < {})", x, y),
                    B::Le => format!("t({} <= {})", x, y),
                    B::Gt => format!("t({} > {})", x, y),
                    B::Ge => format!("t({} >= {})", x, y),
                    B::Eq => format!("t({} == {})", x, y),
                    B::Ne => format!("t({} != {})", x, y),
                    B::And => format!("{} * {}", x, y),
                    B::Or => format!("max({}, {})", x, y),
                    B::Min => format!("min({}, {})", x, y),
                    B::Max => format!("max({}, {})", x, y),
                    B::Pow => format!("pow({}, {})", x, y),
                    B::Atan2 => format!("atan2({}, {})", x, y),
                }
            }
            Op::Un(u, x) => {
                let x = r(x);
                match u {
                    U::Neg => format!("-{}", x),
                    U::Not => format!("1.0 - {}", x),
                    U::Sqrt => format!("sqrt({})", x),
                    U::Abs => format!("abs({})", x),
                    U::Floor => format!("floor({})", x),
                    U::Ceil => format!("ceil({})", x),
                    U::Fract => format!("fract({})", x),
                    U::Sin => format!("sin({})", x),
                    U::Cos => format!("cos({})", x),
                    U::Tan => format!("tan({})", x),
                    U::Exp => format!("exp({})", x),
                    U::Ln => format!("log({})", x),
                    U::Sign => format!("sign({})", x),
                }
            }
            Op::Sel(c, x, y) => format!("sel({}, {}, {})", r(c), r(x), r(y)),
        };
        let _ = writeln!(s, "    let r{}: f32 = {};", i, e);
    }
    for &(col, j) in &p.writes {
        let _ = writeln!(s, "    d[{}u * c + i] = r{};", col, j);
    }
    if let Some(j) = p.dead {
        // the dead flag is the column after the fields
        let _ = writeln!(s, "    d[{}u * c + i] = r{};", p.fields.width, j);
    }
    s.push_str("}\n");
    s
}

/// A swarm's columns on the GPU and the pipeline that steps them.
#[cfg(feature = "gui")]
pub struct GpuSwarm {
    pub gpu: std::sync::Arc<crate::stdlib::gui::gpu::Gpu>,
    pub buf: wgpu::Buffer,
    pub ubuf: wgpu::Buffer,
    pub bind: wgpu::BindGroup,
    pub pipeline: wgpu::ComputePipeline,
    /// Scalars per entity on the GPU: the fields, then the dead flag.
    pub width: usize,
    pub cap: usize,
}

#[cfg(feature = "gui")]
impl GpuSwarm {
    pub fn new(p: &Program, cap: usize) -> Result<Self, String> {
        let gpu = crate::stdlib::gui::gpu::gpu(None, None)?;
        let d = &gpu.device;
        let width = p.fields.width + 1;
        let bytes = (width * cap * 4) as u64;
        let buf = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("toad swarm"),
            size: bytes.max(16),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let ubuf = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("toad world"),
            size: (16 + UNI_VEC4 * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("toad kernel"),
            source: wgpu::ShaderSource::Wgsl(source(p).into()),
        });
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("toad kernel"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("toad kernel"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("toad kernel"),
            layout: Some(&pl),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("toad kernel"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        Ok(GpuSwarm {
            gpu: gpu.clone(),
            buf,
            ubuf,
            bind,
            pipeline,
            width,
            cap,
        })
    }

    /// Upload entities `lo..lo + len` of the CPU columns, every column.
    pub fn write_span(&self, cols: &super::cpu::Columns, lo: usize, len: usize) {
        for c in 0..cols.width {
            let at = c * cols.cap + lo;
            self.gpu.queue.write_buffer(
                &self.buf,
                (at * 4) as u64,
                bytemuck::cast_slice(&cols.data[at..at + len]),
            );
        }
    }

    /// Queue one step of `n` entities (not waited for).
    pub fn step(&self, n: usize, uniforms: &[f32]) {
        let mut u = vec![0u32; 4 + UNI_VEC4 * 4];
        u[0] = n as u32;
        u[1] = self.cap as u32;
        for (i, v) in uniforms.iter().take(UNI_VEC4 * 4).enumerate() {
            u[4 + i] = v.to_bits();
        }
        self.gpu
            .queue
            .write_buffer(&self.ubuf, 0, bytemuck::cast_slice(&u));
        let mut enc = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("toad step"),
            });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("toad step"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.dispatch_workgroups((n as u32).div_ceil(256).max(1), 1, 1);
        }
        self.gpu.queue.submit([enc.finish()]);
    }

    /// Wait for every queued step.
    pub fn sync(&self) {
        let _ = self.gpu.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// The first `n` entities' columns read back (fields, then the dead flag).
    pub fn read(&self, n: usize) -> Vec<f32> {
        let bytes = (self.width * self.cap * 4) as u64;
        let rb = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("toad readback"),
            size: bytes.max(16),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("toad read"),
            });
        enc.copy_buffer_to_buffer(&self.buf, 0, &rb, 0, bytes.max(16));
        self.gpu.queue.submit([enc.finish()]);
        let slice = rb.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.sync();
        let Ok(data) = slice.get_mapped_range() else {
            return Vec::new();
        };
        let all: &[f32] = bytemuck::cast_slice(&data);
        let mut out = Vec::with_capacity(self.width * n);
        for c in 0..self.width {
            out.extend_from_slice(&all[c * self.cap..c * self.cap + n]);
        }
        out
    }
}

/// Show swarms against the world's bodies: each entity a circle at its
/// `pos`, pushed out of every collider it is inside (circles, rounded
/// boxes and capsules, as distance fields), its `vel` bounced; it may die
/// or be marked. One pass after the step, nothing read back.
#[cfg(feature = "gui")]
const COLLIDE: &str = r#"
struct C { n: u32, cap: u32, count: u32, pos: u32, vel: i32, dead: u32, mark: i32, die: u32,
           radius: f32, bounce: f32, friction: f32, rest: f32 };
struct Sh { c: vec2<f32>, rot: f32, kind: f32, half: vec2<f32>, round: f32, pad: f32 };
@group(0) @binding(0) var<storage, read_write> s: array<f32>;
@group(0) @binding(1) var<uniform> u: C;
@group(0) @binding(2) var<storage, read> shapes: array<Sh>;
fn sdf(h: Sh, p: vec2<f32>) -> f32 {
    let co = cos(h.rot); let si = sin(h.rot);
    let d = p - h.c;
    let q = vec2(d.x * co + d.y * si, -d.x * si + d.y * co);
    let kind = u32(h.kind + 0.5);
    if (kind == 0u) { return length(q) - h.round; }
    if (kind == 1u) {
        let r = min(h.round, min(h.half.x, h.half.y));
        let b = abs(q) - h.half + vec2(r);
        return length(max(b, vec2(0.0))) + min(max(b.x, b.y), 0.0) - r;
    }
    let x = clamp(q.x, -h.half.x, h.half.x);
    return length(q - vec2(x, 0.0)) - h.round;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) g: vec3<u32>) {
    let i = g.x;
    if (i >= u.n) { return; }
    let cap = u.cap;
    if (s[u.dead * cap + i] > 0.5) { return; }
    var p = vec2(s[u.pos * cap + i], s[(u.pos + 1u) * cap + i]);
    var v = vec2(0.0);
    if (u.vel >= 0) { v = vec2(s[u32(u.vel) * cap + i], s[(u32(u.vel) + 1u) * cap + i]); }
    var hit = false;
    for (var k = 0u; k < u.count; k++) {
        let h = shapes[k];
        let d = sdf(h, p) - u.radius;
        if (d > 0.001) { continue; }
        hit = true;
        let e = 0.25;
        var n = vec2(sdf(h, p + vec2(e, 0.0)) - sdf(h, p - vec2(e, 0.0)), sdf(h, p + vec2(0.0, e)) - sdf(h, p - vec2(0.0, e)));
        n = n / max(length(n), 1e-6);
        p = p - n * min(d, 0.0);
        let vn = dot(v, n);
        if (vn < 0.0) {
            let vt = v - n * vn;
            var back = -vn * u.bounce;
            if (back < u.rest) { back = 0.0; }
            v = vt * (1.0 - u.friction) + n * back;
        }
    }
    s[u.pos * cap + i] = p.x;
    s[(u.pos + 1u) * cap + i] = p.y;
    if (u.vel >= 0) { s[u32(u.vel) * cap + i] = v.x; s[(u32(u.vel) + 1u) * cap + i] = v.y; }
    if (u.mark >= 0) { s[u32(u.mark) * cap + i] = select(0.0, 1.0, hit); }
    if (hit && u.die == 1u) { s[u.dead * cap + i] = 1.0; }
}
"#;

#[cfg(feature = "gui")]
fn collide_pipeline(d: &wgpu::Device) -> &'static (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    static P: std::sync::OnceLock<(wgpu::ComputePipeline, wgpu::BindGroupLayout)> =
        std::sync::OnceLock::new();
    P.get_or_init(|| {
        let buf = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("toad collide"),
            entries: &[
                buf(0, wgpu::BufferBindingType::Storage { read_only: false }),
                buf(1, wgpu::BufferBindingType::Uniform),
                buf(2, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("toad collide"),
            source: wgpu::ShaderSource::Wgsl(COLLIDE.into()),
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("toad collide"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("toad collide"),
            layout: Some(&pl),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        (pipeline, layout)
    })
}

/// How a show swarm collides: its columns and the options.
#[cfg(feature = "gui")]
pub struct GpuCollide {
    pub pos: usize,
    pub vel: Option<usize>,
    pub mark: Option<usize>,
    pub die: bool,
    pub radius: f32,
    pub bounce: f32,
    pub friction: f32,
    pub rest: f32,
}

#[cfg(feature = "gui")]
impl GpuSwarm {
    /// Collide the first `n` entities with `shapes` (`World::gpu_shapes`),
    /// queued after the steps before it.
    pub fn collide(&self, n: usize, shapes: &[f32], c: &GpuCollide) {
        if n == 0 || shapes.is_empty() {
            return;
        }
        let d = &self.gpu.device;
        let (pipeline, layout) = collide_pipeline(d);
        let words: [u32; 12] = [
            n as u32,
            self.cap as u32,
            (shapes.len() / 8) as u32,
            c.pos as u32,
            c.vel.map(|v| v as i32).unwrap_or(-1) as u32,
            (self.width - 1) as u32,
            c.mark.map(|v| v as i32).unwrap_or(-1) as u32,
            c.die as u32,
            c.radius.to_bits(),
            c.bounce.to_bits(),
            c.friction.to_bits(),
            c.rest.to_bits(),
        ];
        let ub = wgpu::util::DeviceExt::create_buffer_init(
            d,
            &wgpu::util::BufferInitDescriptor {
                label: Some("toad collide"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let sb = wgpu::util::DeviceExt::create_buffer_init(
            d,
            &wgpu::util::BufferInitDescriptor {
                label: Some("toad colliders"),
                contents: bytemuck::cast_slice(shapes),
                usage: wgpu::BufferUsages::STORAGE,
            },
        );
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("toad collide"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ub.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: sb.as_entire_binding(),
                },
            ],
        });
        let mut enc = d.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("toad collide"),
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("toad collide"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((n as u32).div_ceil(256), 1, 1);
        }
        self.gpu.queue.submit([enc.finish()]);
    }
}

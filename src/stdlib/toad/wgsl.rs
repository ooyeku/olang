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

    /// Upload the CPU columns' first `n` entities (all of them, every column).
    pub fn upload(&self, cols: &super::cpu::Columns) {
        for c in 0..cols.width {
            let at = c * cols.cap;
            self.gpu.queue.write_buffer(
                &self.buf,
                (at * 4) as u64,
                bytemuck::cast_slice(&cols.data[at..at + cols.n]),
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
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("toad step") });
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
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("toad read") });
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

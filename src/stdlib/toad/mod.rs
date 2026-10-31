//! `toad` — the engine under the Toad game framework (olang-world/toad).
//!
//! T0, the spike: kernels (plain olang functions compiled to a scalar
//! program, [`kernel`]) and swarms (an engine-owned column store of many
//! entities that a kernel steps, on the CPU for `play` swarms
//! ([`cpu`]) and on the GPU for `show` swarms ([`wgsl`])).
//!
//! ```olang
//! fn spark(p, w) = #{ "pos": p.pos + p.vel * w.dt, "life": p.life - w.dt, "dead": p.life <= 0.0 }
//! let k = toad.kernel(spark, #{ "pos": (0.0, 0.0), "vel": (0.0, 0.0), "life": 1.0 }, #{})?
//! let s = toad.swarm(k, 100000, "show")?
//! toad.spawn(s, 1000, #{ "vel": ("cone", (0.0, 300.0), 0.4), "life": ("between", 0.3, 0.8) })
//! toad.step(s, #{ "dt": 1.0 / 120.0 })
//! ```

pub mod cpu;
pub mod kernel;
#[cfg(feature = "gamepad")]
pub mod pad;
#[cfg(feature = "gui")]
pub mod render;
pub mod wgsl;

use crate::ast::{Value, ValueMap};
use kernel::{Layout, Program, Ty};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type R = Result<Value, Box<dyn std::error::Error>>;

pub fn create_toad_module() -> Value {
    let mut module = ValueMap::default();
    for (name, arity) in [
        ("kernel", 3),
        ("wgsl", 1),
        ("ops", 1),
        ("swarm", 3),
        ("spawn", 3),
        ("step", 2),
        ("sync", 1),
        ("count", 1),
        ("read", 2),
        ("gpu", 0),
        ("frame", 2),
        ("input", 0),
        ("press", 2),
        ("clear", 1),
        ("wanted", 1),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("toad.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

/// A show swarm's columns on the GPU (none in a build without one).
#[cfg(feature = "gui")]
type Device = wgsl::GpuSwarm;
#[cfg(not(feature = "gui"))]
type Device = NoGpu;

/// What a build without a GPU has for one: nothing (no value exists).
#[cfg(not(feature = "gui"))]
enum NoGpu {}

#[cfg(not(feature = "gui"))]
impl NoGpu {
    fn write_span(&self, _: &cpu::Columns, _: usize, _: usize) {
        match *self {}
    }
    fn step(&self, _: usize, _: &[f32]) {
        match *self {}
    }
    fn sync(&self) {
        match *self {}
    }
    fn read(&self, _: usize) -> Vec<f32> {
        match *self {}
    }
}

struct Swarm {
    program: Arc<Program>,
    cols: cpu::Columns,
    gpu: Option<Device>,
    /// A show swarm's next slot to spawn into (it wraps, overwriting the oldest).
    head: usize,
    rng: u64,
}

#[derive(Default)]
struct State {
    kernels: HashMap<i64, Arc<Program>>,
    swarms: HashMap<i64, Swarm>,
    next: i64,
    #[cfg(feature = "gui")]
    renderer: Option<render::Renderer>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

fn num(v: &Value) -> Option<f32> {
    match v {
        Value::Integer(n) => Some(*n as f32),
        Value::Float(x) => Some(*x as f32),
        _ => None,
    }
}

fn pair(v: &Value) -> Option<(f32, f32)> {
    match v {
        Value::Tuple(t) if t.len() == 2 => Some((num(&t[0])?, num(&t[1])?)),
        _ => None,
    }
}

fn map_of<'a>(v: &'a Value, what: &str) -> Result<&'a ValueMap, String> {
    match v {
        Value::Map(m) => Ok(m),
        Value::Struct { fields, .. } => Ok(fields),
        other => Err(format!("toad: {} is {}, expected a map", what, other.type_name())),
    }
}

/// A layout from a map of defaults: a number is a field, a pair a v2.
fn layout(v: &Value, what: &str, first: Option<&str>) -> Result<(Layout, Vec<f32>), String> {
    let m = map_of(v, what)?;
    let mut l = Layout::default();
    let mut defaults = Vec::new();
    if let Some(name) = first {
        l.push(name, Ty::F);
        defaults.push(0.0);
    }
    for (k, v) in m.iter() {
        if Some(k.as_str()) == first {
            continue;
        }
        if let Some(x) = num(v) {
            l.push(k, Ty::F);
            defaults.push(x);
        } else if let Some((x, y)) = pair(v) {
            l.push(k, Ty::V2);
            defaults.extend([x, y]);
        } else {
            return Err(format!(
                "toad: {}'s `{}` is {}; a field is a number or a pair",
                what,
                k,
                v.type_name()
            ));
        }
    }
    Ok((l, defaults))
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

fn err(msg: String) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg))))
}

fn id_of(v: Option<&Value>, what: &str) -> Result<i64, String> {
    match v {
        Some(Value::Integer(n)) => Ok(*n),
        _ => Err(format!("toad: expected a {} (from toad.{})", what, what)),
    }
}

/// splitmix64: a swarm's deterministic random stream.
fn next_rand(s: &mut u64) -> f32 {
    *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// How `spawn` sets a field: a value, or a generator drawn per entity.
enum Gen {
    Fixed(Vec<f32>),
    Between(f32, f32),
    Cone((f32, f32), f32),
    Disc((f32, f32), f32),
    Box((f32, f32), (f32, f32)),
}

fn gen_of(v: &Value, ty: Ty, field: &str) -> Result<Gen, String> {
    if let (Ty::F, Some(x)) = (ty, num(v)) {
        return Ok(Gen::Fixed(vec![x]));
    }
    if let (Ty::V2, Some((x, y))) = (ty, pair(v)) {
        return Ok(Gen::Fixed(vec![x, y]));
    }
    if let Value::Tuple(t) = v
        && let Some(Value::String(kind)) = t.first()
    {
        let a = |i: usize| t.get(i);
        let g = match (kind.as_str(), ty) {
            ("between", Ty::F) => num(a(1).unwrap_or(&Value::Unit))
                .zip(num(a(2).unwrap_or(&Value::Unit)))
                .map(|(lo, hi)| Gen::Between(lo, hi)),
            ("cone", Ty::V2) => pair(a(1).unwrap_or(&Value::Unit))
                .zip(num(a(2).unwrap_or(&Value::Unit)))
                .map(|(d, s)| Gen::Cone(d, s)),
            ("disc", Ty::V2) => pair(a(1).unwrap_or(&Value::Unit))
                .zip(num(a(2).unwrap_or(&Value::Unit)))
                .map(|(c, r)| Gen::Disc(c, r)),
            ("box", Ty::V2) => pair(a(1).unwrap_or(&Value::Unit))
                .zip(pair(a(2).unwrap_or(&Value::Unit)))
                .map(|(lo, hi)| Gen::Box(lo, hi)),
            _ => None,
        };
        if let Some(g) = g {
            return Ok(g);
        }
    }
    Err(format!(
        "toad: spawn's `{}` must be a {} or a generator ({})",
        field,
        if ty == Ty::F { "number" } else { "pair" },
        if ty == Ty::F {
            "(\"between\", lo, hi)"
        } else {
            "(\"cone\", dir, spread), (\"disc\", centre, radius), (\"box\", lo, hi)"
        }
    ))
}

fn draw(g: &Gen, rng: &mut u64, out: &mut [f32]) {
    match g {
        Gen::Fixed(v) => out.copy_from_slice(v),
        Gen::Between(lo, hi) => out[0] = lo + (hi - lo) * next_rand(rng),
        Gen::Cone((dx, dy), spread) => {
            let a = (next_rand(rng) * 2.0 - 1.0) * spread;
            let (s, c) = a.sin_cos();
            out[0] = dx * c - dy * s;
            out[1] = dx * s + dy * c;
        }
        Gen::Disc((cx, cy), r) => {
            let a = next_rand(rng) * std::f32::consts::TAU;
            let d = r * next_rand(rng).sqrt();
            out[0] = cx + d * a.cos();
            out[1] = cy + d * a.sin();
        }
        Gen::Box((x0, y0), (x1, y1)) => {
            out[0] = x0 + (x1 - x0) * next_rand(rng);
            out[1] = y0 + (y1 - y0) * next_rand(rng);
        }
    }
}

fn uniforms(p: &Program, world: &Value) -> Result<Vec<f32>, String> {
    let m = map_of(world, "the world")?;
    let mut u = vec![0.0; p.uniforms.width.max(1)];
    for s in &p.uniforms.slots {
        let Some(v) = m.get(&s.name) else { continue };
        match s.ty {
            Ty::F => u[s.at] = num(v).ok_or_else(|| format!("toad: the world's `{}` is not a number", s.name))?,
            Ty::V2 => {
                let (x, y) =
                    pair(v).ok_or_else(|| format!("toad: the world's `{}` is not a pair", s.name))?;
                u[s.at] = x;
                u[s.at + 1] = y;
            }
        }
    }
    Ok(u)
}

pub fn call_toad_function(name: &str, args: Vec<Value>, scope: &dyn Fn(&str) -> Option<Value>) -> R {
    const UNIT: &Value = &Value::Unit;
    let a = |i: usize| args.get(i).unwrap_or(UNIT);
    match name {
        "kernel" => {
            let Value::Function(f) = a(0) else {
                return Ok(err(format!(
                    "toad.kernel: expected a function, got {}",
                    a(0).type_name()
                )));
            };
            let (fields, defaults) = match layout(a(1), "the kernel's fields", None) {
                Ok(x) => x,
                Err(e) => return Ok(err(e)),
            };
            let (unis, _) = match layout(a(2), "the kernel's world", Some("dt")) {
                Ok(x) => x,
                Err(e) => return Ok(err(e)),
            };
            if unis.width > wgsl::UNI_VEC4 * 4 {
                return Ok(err(format!(
                    "toad.kernel: the world has {} numbers; at most {}",
                    unis.width,
                    wgsl::UNI_VEC4 * 4
                )));
            }
            match kernel::compile(f, &fields, &unis, scope) {
                Ok(mut p) => {
                    p.defaults = defaults;
                    let id = with(|s| {
                        s.next += 1;
                        s.kernels.insert(s.next, Arc::new(p));
                        s.next
                    });
                    Ok(ok(Value::Integer(id)))
                }
                Err(e) => Ok(err(e)),
            }
        }
        "wgsl" | "ops" => {
            let k = id_of(args.first(), "kernel")?;
            let p = with(|s| s.kernels.get(&k).cloned()).ok_or("toad: no such kernel")?;
            Ok(if name == "wgsl" {
                Value::String(Arc::new(wgsl::source(&p)))
            } else {
                Value::Integer(p.ops.len() as i64)
            })
        }
        "swarm" => {
            let k = id_of(args.first(), "kernel")?;
            let cap = num(a(1)).ok_or("toad.swarm: the size is not a number")? as usize;
            let kind = match a(2) {
                Value::String(s) => s.to_string(),
                _ => "play".to_string(),
            };
            let p = with(|s| s.kernels.get(&k).cloned()).ok_or("toad: no such kernel")?;
            let gpu: Option<Device> = match kind.as_str() {
                #[cfg(feature = "gui")]
                "show" => match wgsl::GpuSwarm::new(&p, cap) {
                    Ok(g) => Some(g),
                    Err(e) => return Ok(err(format!("toad.swarm: no GPU for a show swarm: {}", e))),
                },
                #[cfg(not(feature = "gui"))]
                "show" => {
                    return Ok(err("toad.swarm: show swarms need a GPU, which this build has not got".to_string()));
                }
                "play" => None,
                other => {
                    return Ok(err(format!(
                        "toad.swarm: a swarm is \"play\" or \"show\", not \"{}\"",
                        other
                    )));
                }
            };
            // a show swarm keeps a dead flag column after the fields
            let width = p.fields.width + usize::from(gpu.is_some());
            let id = with(|s| {
                s.next += 1;
                s.swarms.insert(
                    s.next,
                    Swarm {
                        program: p,
                        cols: cpu::Columns::new(width, cap),
                        gpu,
                        head: 0,
                        rng: 0x70AD ^ (s.next as u64).wrapping_mul(0x9E37_79B9),
                    },
                );
                s.next
            });
            Ok(ok(Value::Integer(id)))
        }
        "spawn" => {
            let sid = id_of(args.first(), "swarm")?;
            let count = num(a(1)).ok_or("toad.spawn: the count is not a number")? as usize;
            let init = map_of(a(2), "spawn's fields")?.clone();
            with(|st| -> R {
                let sw = st.swarms.get_mut(&sid).ok_or("toad: no such swarm")?;
                let p = sw.program.clone();
                let mut gens = Vec::new();
                for (k, v) in init.iter() {
                    // a seed restarts the swarm's random stream (two swarms
                    // spawned with one seed draw the same entities)
                    if k == "seed" && p.fields.get(k).is_none() {
                        sw.rng = num(v).ok_or("toad.spawn: the seed is not a number")? as u64;
                        continue;
                    }
                    let s = p
                        .fields
                        .get(k)
                        .ok_or_else(|| format!("toad.spawn: the swarm has no `{}`", k))?;
                    gens.push((s.at, s.ty.width(), gen_of(v, s.ty, k)?));
                }
                let cap = sw.cols.cap;
                let mut row = vec![0.0f32; p.fields.width];
                let show = sw.gpu.is_some();
                let mut spans: Vec<(usize, usize)> = Vec::new();
                for _ in 0..count {
                    row.copy_from_slice(&p.defaults);
                    for (at, w, g) in &gens {
                        draw(g, &mut sw.rng, &mut row[*at..*at + *w]);
                    }
                    let slot = if show {
                        let s = sw.head;
                        sw.head = (sw.head + 1) % cap;
                        sw.cols.n = (sw.cols.n.max(s + 1)).min(cap);
                        s
                    } else {
                        if sw.cols.n >= cap {
                            break;
                        }
                        sw.cols.n += 1;
                        sw.cols.n - 1
                    };
                    for (c, v) in row.iter().enumerate() {
                        sw.cols.data[c * cap + slot] = *v;
                    }
                    if show {
                        sw.cols.data[p.fields.width * cap + slot] = 0.0;
                        match spans.last_mut() {
                            Some((lo, len)) if *lo + *len == slot => *len += 1,
                            _ => spans.push((slot, 1)),
                        }
                    }
                }
                if let Some(g) = &sw.gpu {
                    for (lo, len) in spans {
                        g.write_span(&sw.cols, lo, len);
                    }
                }
                Ok(Value::Integer(sw.cols.n as i64))
            })
        }
        "step" => {
            let sid = id_of(args.first(), "swarm")?;
            let world = a(1).clone();
            with(|st| -> R {
                let sw = st.swarms.get_mut(&sid).ok_or("toad: no such swarm")?;
                let u = uniforms(&sw.program, &world)?;
                let mut out = ValueMap::default();
                if let Some(g) = &sw.gpu {
                    g.step(sw.cols.n, &u);
                    out.insert("alive".into(), Value::Integer(sw.cols.n as i64));
                    out.insert("died".into(), Value::Integer(0));
                } else {
                    let dead = cpu::run(&sw.program, &mut sw.cols, &u);
                    let died = dead.map(|d| cpu::compact(&mut sw.cols, &d)).unwrap_or(0);
                    out.insert("alive".into(), Value::Integer(sw.cols.n as i64));
                    out.insert("died".into(), Value::Integer(died as i64));
                }
                Ok(Value::Map(Arc::new(out)))
            })
        }
        "clear" => {
            let sid = id_of(args.first(), "swarm")?;
            with(|st| {
                if let Some(sw) = st.swarms.get_mut(&sid) {
                    sw.cols.n = 0;
                    sw.head = 0;
                }
            });
            Ok(Value::Unit)
        }
        "sync" => {
            let sid = id_of(args.first(), "swarm")?;
            with(|st| {
                if let Some(Swarm { gpu: Some(g), .. }) = st.swarms.get(&sid) {
                    g.sync();
                }
            });
            Ok(Value::Unit)
        }
        "count" => {
            let sid = id_of(args.first(), "swarm")?;
            with(|st| st.swarms.get(&sid).map(|s| Value::Integer(s.cols.n as i64)))
                .ok_or_else(|| "toad: no such swarm".into())
        }
        "read" => {
            let sid = id_of(args.first(), "swarm")?;
            let field = match a(1) {
                Value::String(s) => s.to_string(),
                _ => return Err("toad.read: the field is a string".into()),
            };
            with(|st| -> R {
                let sw = st.swarms.get(&sid).ok_or("toad: no such swarm")?;
                let n = sw.cols.n;
                let data: Vec<f32> = match &sw.gpu {
                    Some(g) => g.read(n),
                    None => (0..sw.cols.width).flat_map(|c| sw.cols.col(c).to_vec()).collect(),
                };
                let (at, ty) = if field == "dead" && sw.gpu.is_some() {
                    (sw.program.fields.width, Ty::F)
                } else {
                    let s = sw
                        .program
                        .fields
                        .get(&field)
                        .ok_or_else(|| format!("toad.read: the swarm has no `{}`", field))?;
                    (s.at, s.ty)
                };
                let col = |c: usize| &data[c * n..c * n + n];
                let list: Vec<Value> = match ty {
                    Ty::F => col(at).iter().map(|x| Value::Float(*x as f64)).collect(),
                    Ty::V2 => col(at)
                        .iter()
                        .zip(col(at + 1))
                        .map(|(x, y)| {
                            Value::Tuple(Arc::new(vec![Value::Float(*x as f64), Value::Float(*y as f64)]))
                        })
                        .collect(),
                };
                Ok(Value::List(Arc::new(list)))
            })
        }
        #[cfg(feature = "gui")]
        "gpu" => Ok(match crate::stdlib::gui::gpu::gpu(None, None) {
            Ok(g) => {
                let i = g.adapter.get_info();
                ok(Value::String(Arc::new(format!("{} ({:?})", i.name, i.backend))))
            }
            Err(e) => err(e),
        }),
        #[cfg(feature = "gui")]
        "frame" => frame(a(0), a(1)),
        #[cfg(feature = "gui")]
        "input" => {
            #[cfg(feature = "gamepad")]
            let (ls, rs, lt, rt, pads) = pad::poll();
            #[cfg(not(feature = "gamepad"))]
            let (ls, rs, lt, rt, pads) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32), 0.0f32, 0.0f32, 0usize);
            let (down, pressed, released) = crate::stdlib::gui::keys::read();
            let list = |v: Vec<String>| Value::List(Arc::new(v.into_iter().map(|k| Value::String(Arc::new(k))).collect()));
            let mut out = ValueMap::default();
            out.insert("down".into(), list(down));
            out.insert("pressed".into(), list(pressed));
            out.insert("released".into(), list(released));
            let p = |v: (f32, f32)| Value::Tuple(Arc::new(vec![Value::Float(v.0 as f64), Value::Float(v.1 as f64)]));
            out.insert("left".into(), p(ls));
            out.insert("right".into(), p(rs));
            out.insert("lt".into(), Value::Float(lt as f64));
            out.insert("rt".into(), Value::Float(rt as f64));
            out.insert("pads".into(), Value::Integer(pads as i64));
            Ok(Value::Map(Arc::new(out)))
        }
        #[cfg(feature = "gui")]
        "press" => {
            // a script's or a test's hand on the keyboard
            let Value::String(k) = a(0) else {
                return Err("toad.press: the key is a string".into());
            };
            if matches!(a(1), Value::Boolean(false)) {
                crate::stdlib::gui::keys::release(k);
            } else {
                crate::stdlib::gui::keys::press(k);
            }
            Ok(Value::Unit)
        }
        #[cfg(feature = "gui")]
        "wanted" => Ok(Value::Boolean(match a(0) {
            Value::String(n) => crate::stdlib::gui::live::wanted(n),
            _ => true,
        })),
        #[cfg(not(feature = "gui"))]
        "gpu" | "frame" | "input" | "press" | "wanted" => Ok(err(format!("toad.{}: this build has no window", name))),
        _ => Err(format!("toad.{} is not a function", name).into()),
    }
}

/// A colour as written, `(r, g, b)` or `(r, g, b, a)` from 0 to 1, or
/// `"#rrggbb"` / `"#rrggbbaa"`: sRGB, as a person and CSS mean it, made
/// linear here for the HDR target (alpha stays as it is).
fn rgba(v: Option<&Value>, d: [f32; 4]) -> [f32; 4] {
    let lin = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let c = match v {
        Some(Value::Tuple(t)) if t.len() >= 3 => {
            let c: Vec<f32> = t.iter().filter_map(num).collect();
            if c.len() < 3 {
                return d;
            }
            [c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)]
        }
        Some(Value::String(s)) if s.starts_with('#') && (s.len() == 7 || s.len() == 9) => {
            let h = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).map(|x| x as f32 / 255.0);
            match (h(1), h(3), h(5)) {
                (Ok(r), Ok(g), Ok(b)) => [r, g, b, if s.len() == 9 { h(7).unwrap_or(1.0) } else { 1.0 }],
                _ => return d,
            }
        }
        _ => return d,
    };
    [lin(c[0]), lin(c[1]), lin(c[2]), c[3]]
}

/// A shape map as its instance: `kind` (circle, box, capsule, ring,
/// triangle, line), `at`, `size` (a radius, or a pair: the whole width
/// and height), `rot`, `round`, `outline`, `color`, `glow` (multiplies
/// the colour past 1.0); a line goes `from` `at` `to` with `width`.
#[cfg(feature = "gui")]
fn shape_of(v: &Value, out: &mut Vec<f32>) -> Result<(), String> {
    let m = map_of(v, "a shape")?;
    let kind = match m.get("kind") {
        Some(Value::String(k)) => k.to_string(),
        _ => "circle".to_string(),
    };
    let n = |k: &str, d: f32| m.get(k).and_then(num).unwrap_or(d);
    let p = |k: &str| m.get(k).and_then(pair);
    let mut c = rgba(m.get("color"), [1.0, 1.0, 1.0, 1.0]);
    let glow = n("glow", 1.0);
    for x in c.iter_mut().take(3) {
        *x *= glow;
    }
    let (at, half, rot, k, line) = match kind.as_str() {
        "line" => {
            let a = p("from").or(p("at")).ok_or("toad.frame: a line needs `from` and `to`")?;
            let b = p("to").ok_or("toad.frame: a line needs `to`")?;
            let wdt = n("width", 2.0);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt();
            (
                ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                (len / 2.0 + wdt / 2.0, wdt / 2.0),
                dy.atan2(dx),
                2.0,
                0.0,
            )
        }
        other => {
            let at = p("at").unwrap_or((0.0, 0.0));
            let half = match m.get("size") {
                Some(v) => match (num(v), pair(v)) {
                    (Some(r), _) => (r, r),
                    (_, Some((w, h))) => (w / 2.0, h / 2.0),
                    _ => return Err("toad.frame: a shape's `size` is a number or a pair".into()),
                },
                None => (8.0, 8.0),
            };
            let (k, line) = match other {
                "circle" => (0.0, n("outline", 0.0)),
                "ring" => (0.0, n("outline", 2.0)),
                "box" => (1.0, n("outline", 0.0)),
                "capsule" => (2.0, n("outline", 0.0)),
                "triangle" => (3.0, n("outline", 0.0)),
                _ => {
                    return Err(format!(
                        "toad.frame: a shape is a circle, box, capsule, ring, triangle or line, not a {}",
                        other
                    ));
                }
            };
            (at, half, n("rot", 0.0), k, line)
        }
    };
    out.extend_from_slice(&[
        at.0, at.1, half.0, half.1, rot, k, n("round", 0.0), line, c[0], c[1], c[2], c[3],
    ]);
    Ok(())
}

/// `toad.frame(layers, view)`: each layer a swarm (`swarm`, `color`,
/// `radius`, `glow`), `shapes` (a list of shape maps) or `sprites` (a
/// list of `#{ src, at, size, rot, tint }`), drawn in order.
#[cfg(feature = "gui")]
fn frame(layers: &Value, view: &Value) -> R {
    let Value::List(layers) = layers else {
        return Err("toad.frame: the layers are a list".into());
    };
    let vm = map_of(view, "the frame's view")?;
    let getp = |k: &str, d: (f32, f32)| vm.get(k).and_then(pair).unwrap_or(d);
    let getn = |k: &str, d: f32| vm.get(k).and_then(num).unwrap_or(d);
    let size = getp("size", (1280.0, 720.0));
    let bg = rgba(vm.get("bg"), [0.0, 0.0, 0.0, 1.0]);
    let view = render::View {
        size: (size.0.max(1.0) as u32, size.1.max(1.0) as u32),
        center: getp("center", (0.0, 0.0)),
        scale: getn("scale", 1.0),
        rot: getn("rot", 0.0),
        bg: [bg[0], bg[1], bg[2]],
        exposure: getn("exposure", 1.0),
        vignette: getn("vignette", 0.0),
    };
    let want_png = matches!(vm.get("png"), Some(Value::Boolean(true)));
    let live = match vm.get("live") {
        Some(Value::String(n)) => Some(n.to_string()),
        _ => None,
    };
    // a live frame no window is waiting for is not drawn (paced to the
    // display; nothing while the window is hidden)
    if let Some(n) = &live
        && !want_png
        && !matches!(vm.get("force"), Some(Value::Boolean(true)))
        && !crate::stdlib::gui::live::wanted(n)
    {
        let mut out = ValueMap::default();
        out.insert("ms".into(), Value::Float(0.0));
        out.insert("skipped".into(), Value::Boolean(true));
        return Ok(Value::Map(Arc::new(out)));
    }
    with(|st| -> R {
        if st.renderer.is_none() {
            st.renderer = Some(render::Renderer::new()?);
        }
        let t0 = std::time::Instant::now();
        // play swarms' columns go up once a frame
        let mut uploads: Vec<(i64, wgpu::Buffer)> = Vec::new();
        for l in layers.iter() {
            let lm = map_of(l, "a frame layer")?;
            let Some(sid) = lm.get("swarm") else { continue };
            let sid = id_of(Some(sid), "swarm")?;
            let sw = st.swarms.get(&sid).ok_or("toad.frame: no such swarm")?;
            if sw.gpu.is_none() && !uploads.iter().any(|(k, _)| *k == sid) {
                let r = st.renderer.as_ref().unwrap();
                let b = wgpu::util::DeviceExt::create_buffer_init(
                    &r.gpu().device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("toad play columns"),
                        contents: bytemuck::cast_slice(&sw.cols.data),
                        usage: wgpu::BufferUsages::STORAGE,
                    },
                );
                uploads.push((sid, b));
            }
        }
        let mut parts = Vec::new();
        for l in layers.iter() {
            let lm = map_of(l, "a frame layer")?;
            if let Some(Value::List(shapes)) = lm.get("shapes") {
                let mut inst = Vec::with_capacity(shapes.len() * render::SHAPE_FLOATS);
                for s in shapes.iter() {
                    shape_of(s, &mut inst)?;
                }
                parts.push(render::Part::Shapes(inst));
                continue;
            }
            if let Some(Value::List(sprites)) = lm.get("sprites") {
                // runs of one picture, in order
                let mut runs: Vec<(Arc<crate::stdlib::gui::canvas::Picture>, Vec<f32>)> = Vec::new();
                for s in sprites.iter() {
                    let sm = map_of(s, "a sprite")?;
                    let src = sm.get("src").ok_or("toad.frame: a sprite needs a `src`")?;
                    let (pic, _) = crate::stdlib::gui::picture::get(src, None, None)?
                        .ok_or("toad.frame: a sprite's picture could not be read")?;
                    let at = sm.get("at").and_then(pair).unwrap_or((0.0, 0.0));
                    let (w, h) = match sm.get("size") {
                        Some(v) => match (num(v), pair(v)) {
                            (Some(k), _) => (pic.width as f32 * k, pic.height as f32 * k),
                            (_, Some(p)) => p,
                            _ => (pic.width as f32, pic.height as f32),
                        },
                        None => (pic.width as f32, pic.height as f32),
                    };
                    let rot = sm.get("rot").and_then(num).unwrap_or(0.0);
                    let t = rgba(sm.get("tint"), [1.0, 1.0, 1.0, 1.0]);
                    let row = [at.0, at.1, w / 2.0, h / 2.0, rot, 0.0, 0.0, 0.0, t[0], t[1], t[2], t[3]];
                    match runs.last_mut() {
                        Some((p, v)) if p.id == pic.id => v.extend_from_slice(&row),
                        _ => runs.push((pic, row.to_vec())),
                    }
                }
                parts.push(render::Part::Sprites(runs));
                continue;
            }
            let sid = id_of(lm.get("swarm"), "swarm")?;
            let sw = st.swarms.get(&sid).ok_or("toad.frame: no such swarm")?;
            let f = &sw.program.fields;
            let pos = f
                .get(match lm.get("pos") {
                    Some(Value::String(s)) => s.as_str(),
                    _ => "pos",
                })
                .filter(|s| s.ty == Ty::V2)
                .ok_or("toad.frame: the swarm has no pair `pos` to draw at")?
                .at;
            let glow = match lm.get("glow") {
                Some(Value::String(s)) => f.get(s).map(|s| s.at),
                _ => None,
            };
            let (buf, dead) = match &sw.gpu {
                Some(g) => (&g.buf, sw.program.dead.map(|_| f.width)),
                None => (&uploads.iter().find(|(k, _)| *k == sid).unwrap().1, None),
            };
            parts.push(render::Part::Swarm(render::Layer {
                buf,
                cap: sw.cols.cap,
                n: sw.cols.n,
                pos,
                glow,
                dead,
                color: rgba(lm.get("color"), [1.0, 1.0, 1.0, 1.0]),
                radius: lm.get("radius").and_then(num).unwrap_or(4.0),
            }));
        }
        let renderer = st.renderer.as_mut().unwrap();
        let px = renderer.frame(&view, &parts, want_png, live.as_deref());
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut out = ValueMap::default();
        out.insert("ms".into(), Value::Float(ms));
        if let Some(px) = px {
            let pm = tiny_skia::Pixmap::from_vec(
                px,
                tiny_skia::IntSize::from_wh(view.size.0, view.size.1).ok_or("toad.frame: no size")?,
            )
            .ok_or("toad.frame: the frame's pixels")?;
            let png = pm.encode_png().map_err(|e| format!("toad.frame: {}", e))?;
            out.insert("png".into(), crate::stdlib::bytes::to_value(png));
        }
        Ok(Value::Map(Arc::new(out)))
    })
}

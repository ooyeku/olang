//! A kernel run on the CPU: the program's ops each run over a chunk of the
//! swarm at a time (a tight loop the compiler vectorises), the chunks
//! spread over the cores. This is the path for play swarms, whose results
//! the game depends on, and the reference the GPU path is held to.

use super::kernel::{B, Op, Program, U};
#[cfg(feature = "native")]
use rayon::prelude::*;

/// Entities per chunk: the ops' scratch for a chunk stays in cache.
const CHUNK: usize = 1024;

pub fn bin(b: B, x: f32, y: f32) -> f32 {
    let t = |c: bool| if c { 1.0 } else { 0.0 };
    match b {
        B::Add => x + y,
        B::Sub => x - y,
        B::Mul => x * y,
        B::Div => x / y,
        // WGSL's `%` on floats truncates, as Rust's does
        B::Rem => x % y,
        B::Lt => t(x < y),
        B::Le => t(x <= y),
        B::Gt => t(x > y),
        B::Ge => t(x >= y),
        B::Eq => t(x == y),
        B::Ne => t(x != y),
        B::And => x * y,
        B::Or => x.max(y),
        B::Min => x.min(y),
        B::Max => x.max(y),
        B::Pow => x.powf(y),
        B::Atan2 => x.atan2(y),
    }
}

pub fn un(u: U, x: f32) -> f32 {
    match u {
        U::Neg => -x,
        U::Not => 1.0 - x,
        U::Sqrt => x.sqrt(),
        U::Abs => x.abs(),
        U::Floor => x.floor(),
        U::Ceil => x.ceil(),
        U::Fract => x - x.floor(),
        U::Sin => x.sin(),
        U::Cos => x.cos(),
        U::Tan => x.tan(),
        U::Exp => x.exp(),
        U::Ln => x.ln(),
        U::Sign => {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
    }
}

/// A swarm's numbers: `width` columns of `cap` scalars, column-major.
pub struct Columns {
    pub width: usize,
    pub cap: usize,
    pub n: usize,
    pub data: Vec<f32>,
}

impl Columns {
    pub fn new(width: usize, cap: usize) -> Self {
        Columns {
            width,
            cap,
            n: 0,
            data: vec![0.0; width * cap],
        }
    }
    pub fn col(&self, c: usize) -> &[f32] {
        &self.data[c * self.cap..c * self.cap + self.n]
    }
}

enum Arg<'a> {
    S(f32),
    C(&'a [f32]),
}

/// Run `p` over every entity; answers the per-entity dead flags when the
/// kernel has them (the caller removes those entities).
pub fn run(p: &Program, cols: &mut Columns, uniforms: &[f32]) -> Option<Vec<bool>> {
    let n = cols.n;
    if n == 0 {
        return p.dead.map(|_| Vec::new());
    }
    let cap = cols.cap;
    let chunks = n.div_ceil(CHUNK);
    // which ops need a scratch column (constants, columns and uniforms are read in place)
    let scratch: Vec<Option<usize>> = {
        let mut k = 0;
        p.ops
            .iter()
            .map(|o| match o {
                Op::Bin(..) | Op::Un(..) | Op::Sel(..) => {
                    k += 1;
                    Some(k - 1)
                }
                _ => None,
            })
            .collect()
    };
    let nscratch = scratch.iter().filter(|s| s.is_some()).count();
    let src = &cols.data;
    // each chunk's results: (writes' values, dead flags)
    // every core natively; one thread in the browser
    #[cfg(feature = "native")]
    let chunked = (0..chunks).into_par_iter();
    #[cfg(not(feature = "native"))]
    let chunked = (0..chunks).into_iter();
    let results: Vec<(Vec<Vec<f32>>, Vec<bool>)> = crate::parallel::install(|| {
        chunked
            .map(|ci| {
                let lo = ci * CHUNK;
                let len = CHUNK.min(n - lo);
                let mut regs = vec![0.0f32; nscratch * len];
                for (i, op) in p.ops.iter().enumerate() {
                    let Some(r) = scratch[i] else { continue };
                    // split so the op's own output is writable while earlier ones are read
                    let (before, rest) = regs.split_at_mut(r * len);
                    let out = &mut rest[..len];
                    let arg = |j: usize| -> Arg<'_> {
                        match p.ops[j] {
                            Op::Const(c) => Arg::S(c),
                            Op::Uni(u) => Arg::S(uniforms.get(u).copied().unwrap_or(0.0)),
                            Op::Col(c) => Arg::C(&src[c * cap + lo..c * cap + lo + len]),
                            _ => {
                                let s = scratch[j].expect("an op's operand is an op");
                                Arg::C(&before[s * len..s * len + len])
                            }
                        }
                    };
                    match *op {
                        Op::Bin(b, x, y) => match (arg(x), arg(y)) {
                            (Arg::C(a), Arg::C(c)) => {
                                for k in 0..len {
                                    out[k] = bin(b, a[k], c[k]);
                                }
                            }
                            (Arg::C(a), Arg::S(c)) => {
                                for k in 0..len {
                                    out[k] = bin(b, a[k], c);
                                }
                            }
                            (Arg::S(a), Arg::C(c)) => {
                                for k in 0..len {
                                    out[k] = bin(b, a, c[k]);
                                }
                            }
                            (Arg::S(a), Arg::S(c)) => out.fill(bin(b, a, c)),
                        },
                        Op::Un(u, x) => match arg(x) {
                            Arg::C(a) => {
                                for k in 0..len {
                                    out[k] = un(u, a[k]);
                                }
                            }
                            Arg::S(a) => out.fill(un(u, a)),
                        },
                        Op::Sel(c, x, y) => {
                            let (c, x, y) = (arg(c), arg(x), arg(y));
                            let at = |a: &Arg<'_>, k: usize| match a {
                                Arg::S(s) => *s,
                                Arg::C(v) => v[k],
                            };
                            for (k, o) in out.iter_mut().enumerate().take(len) {
                                *o = if at(&c, k) > 0.5 {
                                    at(&x, k)
                                } else {
                                    at(&y, k)
                                };
                            }
                        }
                        _ => {}
                    }
                }
                let value = |j: usize, k: usize| match p.ops[j] {
                    Op::Const(c) => c,
                    Op::Uni(u) => uniforms.get(u).copied().unwrap_or(0.0),
                    Op::Col(c) => src[c * cap + lo + k],
                    _ => regs[scratch[j].unwrap() * len + k],
                };
                let writes: Vec<Vec<f32>> = p
                    .writes
                    .iter()
                    .map(|&(_, j)| (0..len).map(|k| value(j, k)).collect())
                    .collect();
                let dead: Vec<bool> = match p.dead {
                    Some(j) => (0..len).map(|k| value(j, k) > 0.5).collect(),
                    None => Vec::new(),
                };
                (writes, dead)
            })
            .collect()
    });
    let mut dead = p.dead.map(|_| Vec::with_capacity(n));
    for (ci, (writes, d)) in results.into_iter().enumerate() {
        let lo = ci * CHUNK;
        for (w, vals) in p.writes.iter().zip(writes) {
            let at = w.0 * cap + lo;
            cols.data[at..at + vals.len()].copy_from_slice(&vals);
        }
        if let Some(all) = dead.as_mut() {
            all.extend(d);
        }
    }
    dead
}

/// Remove the entities flagged dead, keeping the rest in order (so a
/// step's result is the same however it was chunked).
pub fn compact(cols: &mut Columns, dead: &[bool]) -> usize {
    let n = cols.n;
    let mut keep = 0;
    for (i, &gone) in dead.iter().enumerate().take(n) {
        if !gone {
            if keep != i {
                for c in 0..cols.width {
                    cols.data[c * cols.cap + keep] = cols.data[c * cols.cap + i];
                }
            }
            keep += 1;
        }
    }
    cols.n = keep;
    n - keep
}

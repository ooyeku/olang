//! The expression engine against a row-by-row reference: random plans
//! over Float, Int and Bool columns with nulls, every operation, values
//! compared bit for bit and errors where the reference raises. Then the
//! reductions against the eager kernels, and the parallel path against
//! the sequential one.

use olang_ods::fuse::{BinOp, Closed, Expr, Math, Op, Plan, Reduce};
use olang_ods::{DType, Scalar, Series};
use std::sync::Arc;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() % 1_000_000) as f64 / 1e6 < p
    }
    fn f(&mut self) -> f64 {
        (self.next() % 2_000_001) as f64 / 1000.0 - 1000.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum V {
    F(f64),
    I(i64),
    B(bool),
    N,
}

fn val(s: &Series, i: usize) -> V {
    match s.scalar_at(i) {
        Scalar::F64(x) => V::F(x),
        Scalar::I64(x) => V::I(x),
        Scalar::Bool(b) => V::B(b),
        Scalar::Null => V::N,
        _ => unreachable!("random plans are numeric"),
    }
}

fn lit_v(s: &Scalar) -> V {
    match s {
        Scalar::F64(x) => V::F(*x),
        Scalar::I64(x) => V::I(*x),
        Scalar::Bool(b) => V::B(*b),
        _ => V::N,
    }
}

fn fl(v: V) -> f64 {
    match v {
        V::F(x) => x,
        V::I(x) => x as f64,
        V::B(b) => b as i64 as f64,
        V::N => unreachable!(),
    }
}

/// The type a node computes, for widening a chosen branch.
fn is_float(e: &Expr) -> bool {
    matches!(e.dtype(), Ok(Some(DType::F64)) | Ok(None))
}

/// One row of `e`, the slow and obvious way.
fn reference(e: &Expr, i: usize) -> Result<V, ()> {
    Ok(match e {
        Expr::Col(s) => val(s, i),
        Expr::Lit(s) => lit_v(s),
        Expr::Op(op, args) => {
            let a: Vec<V> = args
                .iter()
                .map(|a| reference(a, i))
                .collect::<Result<_, _>>()?;
            let widen = |v: V| {
                if is_float(e) {
                    match v {
                        V::N => V::N,
                        v => V::F(fl(v)),
                    }
                } else {
                    v
                }
            };
            match op {
                Op::Bin(b) => bin(*b, a[0], a[1])?,
                Op::Neg => match a[0] {
                    V::N => V::N,
                    V::F(x) => V::F(-x),
                    V::I(x) => V::I(x.checked_neg().ok_or(())?),
                    _ => unreachable!(),
                },
                Op::Not => match a[0] {
                    V::B(x) => V::B(!x),
                    _ => V::N,
                },
                Op::Math(m) => match a[0] {
                    V::N => V::N,
                    V::I(x) => match m {
                        Math::Abs => V::I(x.checked_abs().ok_or(())?),
                        Math::Sign => V::I(x.signum()),
                        Math::Floor | Math::Ceil | Math::Trunc => V::I(x),
                        _ => math(*m, x as f64)?,
                    },
                    v => math(*m, fl(v))?,
                },
                Op::Round(d) => match a[0] {
                    V::N => V::N,
                    V::I(x) => V::I(x),
                    v => {
                        let x = fl(v);
                        V::F(if *d == 0 {
                            x.round()
                        } else if *d < 0 {
                            let s = 10f64.powi(-*d);
                            (x / s).round() * s
                        } else {
                            let s = 10f64.powi(*d);
                            let r = (x * s).round() / s;
                            if r.is_finite() { r } else { x }
                        })
                    }
                },
                Op::IsNull => V::B(a[0] == V::N),
                Op::NotNull => V::B(a[0] != V::N),
                Op::IsNan => match a[0] {
                    V::N => V::N,
                    v => V::B(fl(v).is_nan()),
                },
                Op::Where => match a[0] {
                    V::B(true) => widen(a[1]),
                    V::B(false) => widen(a[2]),
                    _ => V::N,
                },
                Op::Coalesce => widen(a.iter().copied().find(|v| *v != V::N).unwrap_or(V::N)),
                Op::Clip => match a[0] {
                    V::N => V::N,
                    x => {
                        let float = is_float(e);
                        if float {
                            let mut v = fl(x);
                            if a[1] != V::N && v < fl(a[1]) {
                                v = fl(a[1]);
                            }
                            if a[2] != V::N && v > fl(a[2]) {
                                v = fl(a[2]);
                            }
                            V::F(v)
                        } else {
                            let V::I(mut v) = x else { unreachable!() };
                            if let V::I(l) = a[1]
                                && v < l
                            {
                                v = l;
                            }
                            if let V::I(h) = a[2]
                                && v > h
                            {
                                v = h;
                            }
                            V::I(v)
                        }
                    }
                },
                Op::Between(c) => match a[0] {
                    V::N => V::N,
                    x => {
                        let v = fl(x);
                        let (lc, hc) = match c {
                            Closed::Both => (true, true),
                            Closed::Left => (true, false),
                            Closed::Right => (false, true),
                            Closed::Neither => (false, false),
                        };
                        let above = a[1] == V::N || if lc { v >= fl(a[1]) } else { v > fl(a[1]) };
                        let below = a[2] == V::N || if hc { v <= fl(a[2]) } else { v < fl(a[2]) };
                        V::B(above && below)
                    }
                },
                Op::Cast(to) => match (a[0], to) {
                    (V::N, _) => V::N,
                    (v, DType::F64) => V::F(fl(v)),
                    (V::F(x), DType::I64) => {
                        let t = x.trunc();
                        if t.is_finite() && t >= i64::MIN as f64 && t <= i64::MAX as f64 {
                            V::I(t as i64)
                        } else {
                            V::N
                        }
                    }
                    (V::B(b), DType::I64) => V::I(b as i64),
                    (V::I(x), DType::Bool) => V::B(x != 0),
                    (v, _) => v,
                },
                Op::Temporal(..) => unreachable!("random plans have no dates"),
            }
        }
    })
}

fn math(m: Math, x: f64) -> Result<V, ()> {
    let bad = match m {
        Math::Sqrt => x < 0.0,
        Math::Ln | Math::Log2 | Math::Log10 => x <= 0.0,
        Math::Ln1p => x <= -1.0,
        Math::Asin | Math::Acos => !(-1.0..=1.0).contains(&x),
        Math::Logit => x <= 0.0 || x >= 1.0,
        _ => false,
    };
    if bad {
        return Err(());
    }
    Ok(V::F(match m {
        Math::Abs => x.abs(),
        Math::Sign => {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                x
            }
        }
        Math::Exp => x.exp(),
        Math::Exp2 => x.exp2(),
        Math::Expm1 => x.exp_m1(),
        Math::Ln => x.ln(),
        Math::Ln1p => x.ln_1p(),
        Math::Log2 => x.log2(),
        Math::Log10 => x.log10(),
        Math::Sqrt => x.sqrt(),
        Math::Cbrt => x.cbrt(),
        Math::Sin => x.sin(),
        Math::Cos => x.cos(),
        Math::Tan => x.tan(),
        Math::Asin => x.asin(),
        Math::Acos => x.acos(),
        Math::Atan => x.atan(),
        Math::Sinh => x.sinh(),
        Math::Cosh => x.cosh(),
        Math::Tanh => x.tanh(),
        Math::Sigmoid => {
            if x >= 0.0 {
                1.0 / (1.0 + (-x).exp())
            } else {
                x.exp() / (1.0 + x.exp())
            }
        }
        Math::Logit => (x / (1.0 - x)).ln(),
        Math::Floor => x.floor(),
        Math::Ceil => x.ceil(),
        Math::Trunc => x.trunc(),
        Math::Fract => x.fract(),
        Math::Degrees => x.to_degrees(),
        Math::Radians => x.to_radians(),
    }))
}

fn bin(b: BinOp, x: V, y: V) -> Result<V, ()> {
    use BinOp::*;
    match b {
        And => {
            return Ok(match (x, y) {
                (V::B(false), _) | (_, V::B(false)) => V::B(false),
                (V::B(true), V::B(true)) => V::B(true),
                _ => V::N,
            });
        }
        Or => {
            return Ok(match (x, y) {
                (V::B(true), _) | (_, V::B(true)) => V::B(true),
                (V::B(false), V::B(false)) => V::B(false),
                _ => V::N,
            });
        }
        _ => {}
    }
    if x == V::N || y == V::N {
        return Ok(V::N);
    }
    Ok(match b {
        Xor => match (x, y) {
            (V::B(p), V::B(q)) => V::B(p ^ q),
            _ => unreachable!(),
        },
        Eq | Ne | Lt | Le | Gt | Ge => {
            let o = match (x, y) {
                (V::I(p), V::I(q)) => p.partial_cmp(&q),
                (V::B(p), V::B(q)) => p.partial_cmp(&q),
                _ => fl(x).partial_cmp(&fl(y)),
            };
            V::B(match b {
                Eq => o == Some(std::cmp::Ordering::Equal),
                Ne => o != Some(std::cmp::Ordering::Equal),
                Lt => o == Some(std::cmp::Ordering::Less),
                Le => matches!(
                    o,
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                ),
                Gt => o == Some(std::cmp::Ordering::Greater),
                _ => matches!(
                    o,
                    Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                ),
            })
        }
        Div => {
            if fl(y) == 0.0 {
                return Err(());
            }
            V::F(fl(x) / fl(y))
        }
        Pow => V::F(fl(x).powf(fl(y))),
        _ => match (x, y) {
            (V::I(p), V::I(q)) => V::I(match b {
                Add => p.checked_add(q).ok_or(())?,
                Sub => p.checked_sub(q).ok_or(())?,
                Mul => p.checked_mul(q).ok_or(())?,
                _ => {
                    if q == 0 {
                        return Err(());
                    }
                    p.checked_rem(q).ok_or(())?
                }
            }),
            _ => {
                let (p, q) = (fl(x), fl(y));
                V::F(match b {
                    Add => p + q,
                    Sub => p - q,
                    Mul => p * q,
                    _ => {
                        if q == 0.0 {
                            return Err(());
                        }
                        p % q
                    }
                })
            }
        },
    })
}

#[derive(Clone, Copy, PartialEq)]
enum T {
    F,
    I,
    B,
}

struct Gen {
    rng: Rng,
    f: Vec<Arc<Expr>>,
    i: Vec<Arc<Expr>>,
    b: Vec<Arc<Expr>>,
}

impl Gen {
    fn pick(&mut self, v: &[Arc<Expr>]) -> Arc<Expr> {
        v[self.rng.below(v.len() as u64) as usize].clone()
    }

    fn null_or(&mut self, e: Arc<Expr>) -> Arc<Expr> {
        if self.rng.chance(0.15) {
            Expr::lit(Scalar::Null)
        } else {
            e
        }
    }

    fn leaf(&mut self, t: T) -> Arc<Expr> {
        let lit = self.rng.chance(0.25);
        match t {
            T::F if lit => {
                let x = if self.rng.chance(0.05) {
                    0.0
                } else {
                    self.rng.f() / 10.0
                };
                Expr::lit(Scalar::F64(x))
            }
            T::I if lit => {
                let x = if self.rng.chance(0.03) {
                    i64::MAX
                } else {
                    self.rng.below(41) as i64 - 20
                };
                Expr::lit(Scalar::I64(x))
            }
            T::B if lit => Expr::lit(Scalar::Bool(self.rng.chance(0.5))),
            T::F => {
                let f = self.f.clone();
                self.pick(&f)
            }
            T::I => {
                let i = self.i.clone();
                self.pick(&i)
            }
            T::B => {
                let b = self.b.clone();
                self.pick(&b)
            }
        }
    }

    fn num(&mut self, d: u32) -> Arc<Expr> {
        let t = if self.rng.chance(0.6) { T::F } else { T::I };
        self.make(t, d)
    }

    fn make(&mut self, t: T, d: u32) -> Arc<Expr> {
        if d == 0 || self.rng.chance(0.2) {
            return self.leaf(t);
        }
        let d = d - 1;
        let r = self.rng.below(10);
        match t {
            T::F => match r {
                0 | 1 => {
                    let ops = [
                        BinOp::Add,
                        BinOp::Sub,
                        BinOp::Mul,
                        BinOp::Div,
                        BinOp::Pow,
                        BinOp::Mod,
                    ];
                    let op = ops[self.rng.below(6) as usize];
                    let a = self.make(T::F, d);
                    let b = self.num(d);
                    if self.rng.chance(0.5) {
                        Expr::bin(op, a, b)
                    } else {
                        Expr::bin(op, b, a)
                    }
                }
                2 => Expr::op(Op::Neg, vec![self.make(T::F, d)]),
                3 => {
                    let m = Math::ALL[self.rng.below(Math::ALL.len() as u64) as usize].1;
                    let a = self.num(d);
                    // Keep int-preserving functions on a Float input here.
                    let a = if matches!(
                        m,
                        Math::Abs | Math::Sign | Math::Floor | Math::Ceil | Math::Trunc
                    ) {
                        self.make(T::F, d)
                    } else {
                        a
                    };
                    Expr::op(Op::Math(m), vec![a])
                }
                4 => Expr::op(
                    Op::Round(self.rng.below(5) as i32 - 2),
                    vec![self.make(T::F, d)],
                ),
                5 => {
                    let c = self.make(T::B, d);
                    let a = self.make(T::F, d);
                    let b = self.num(d);
                    let b = self.null_or(b);
                    Expr::op(Op::Where, vec![c, a, b])
                }
                6 => {
                    let a = self.make(T::F, d);
                    let b = self.num(d);
                    let mut v = vec![a, b];
                    if self.rng.chance(0.5) {
                        v.push(Expr::lit(Scalar::F64(7.5)));
                    }
                    Expr::op(Op::Coalesce, v)
                }
                7 => {
                    let x = self.make(T::F, d);
                    let lo = self.leaf(T::F);
                    let lo = self.null_or(lo);
                    let hi = self.make(T::F, d);
                    let hi = self.null_or(hi);
                    Expr::op(Op::Clip, vec![x, lo, hi])
                }
                8 => {
                    let a = if self.rng.chance(0.5) {
                        self.make(T::I, d)
                    } else {
                        self.make(T::B, d)
                    };
                    Expr::op(Op::Cast(DType::F64), vec![a])
                }
                _ => self.leaf(T::F),
            },
            T::I => match r {
                0..=2 => {
                    let ops = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Mod];
                    let op = ops[self.rng.below(4) as usize];
                    Expr::bin(op, self.make(T::I, d), self.make(T::I, d))
                }
                3 => Expr::op(Op::Neg, vec![self.make(T::I, d)]),
                4 => {
                    let ms = [Math::Abs, Math::Sign, Math::Floor];
                    let m = ms[self.rng.below(3) as usize];
                    Expr::op(Op::Math(m), vec![self.make(T::I, d)])
                }
                5 => {
                    let c = self.make(T::B, d);
                    let a = self.make(T::I, d);
                    let b = self.make(T::I, d);
                    let b = self.null_or(b);
                    Expr::op(Op::Where, vec![c, a, b])
                }
                6 => Expr::op(Op::Coalesce, vec![self.make(T::I, d), self.make(T::I, d)]),
                7 => {
                    let x = self.make(T::I, d);
                    let lo = Expr::lit(Scalar::I64(-5));
                    let lo = self.null_or(lo);
                    let hi = self.make(T::I, d);
                    let hi = self.null_or(hi);
                    Expr::op(Op::Clip, vec![x, lo, hi])
                }
                8 => {
                    let a = if self.rng.chance(0.5) {
                        self.make(T::F, d)
                    } else {
                        self.make(T::B, d)
                    };
                    Expr::op(Op::Cast(DType::I64), vec![a])
                }
                _ => self.leaf(T::I),
            },
            T::B => match r {
                0 | 1 => {
                    let ops = [
                        BinOp::Eq,
                        BinOp::Ne,
                        BinOp::Lt,
                        BinOp::Le,
                        BinOp::Gt,
                        BinOp::Ge,
                    ];
                    let op = ops[self.rng.below(6) as usize];
                    let a = self.num(d);
                    let b = self.num(d);
                    Expr::bin(op, a, b)
                }
                2 => {
                    let ops = [BinOp::And, BinOp::Or, BinOp::Xor];
                    let op = ops[self.rng.below(3) as usize];
                    let a = self.make(T::B, d);
                    let b = self.make(T::B, d);
                    let b = self.null_or(b);
                    Expr::bin(op, a, b)
                }
                3 => Expr::op(Op::Not, vec![self.make(T::B, d)]),
                4 => {
                    let t = [T::F, T::I, T::B][self.rng.below(3) as usize];
                    let op = if self.rng.chance(0.5) {
                        Op::IsNull
                    } else {
                        Op::NotNull
                    };
                    Expr::op(op, vec![self.make(t, d)])
                }
                5 => Expr::op(Op::IsNan, vec![self.make(T::F, d)]),
                6 => {
                    let cs = [Closed::Both, Closed::Left, Closed::Right, Closed::Neither];
                    let c = cs[self.rng.below(4) as usize];
                    let x = self.num(d);
                    let lo = self.leaf(T::F);
                    let lo = self.null_or(lo);
                    let hi = self.num(d);
                    let hi = self.null_or(hi);
                    Expr::op(Op::Between(c), vec![x, lo, hi])
                }
                7 => {
                    let c = self.make(T::B, d);
                    let a = self.make(T::B, d);
                    let b = self.make(T::B, d);
                    Expr::op(Op::Where, vec![c, a, b])
                }
                8 => {
                    let a = self.make(T::B, d);
                    let b = self.make(T::B, d);
                    let b = self.null_or(b);
                    Expr::op(Op::Coalesce, vec![a, b])
                }
                _ => Expr::bin(BinOp::Eq, self.make(T::B, d), self.make(T::B, d)),
            },
        }
    }
}

fn columns(
    rng: &mut Rng,
    n: usize,
    nulls: f64,
) -> (Vec<Arc<Expr>>, Vec<Arc<Expr>>, Vec<Arc<Expr>>) {
    let mut f = Vec::new();
    let mut i = Vec::new();
    let mut b = Vec::new();
    for k in 0..3 {
        let p = if k == 0 { 0.0 } else { nulls };
        f.push(Expr::col(Series::from_f64_options(
            (0..n)
                .map(|_| {
                    if rng.chance(p) {
                        None
                    } else if rng.chance(0.001) {
                        Some(0.0)
                    } else if rng.chance(0.002) {
                        Some(f64::NAN)
                    } else {
                        Some(rng.f())
                    }
                })
                .collect(),
        )));
        i.push(Expr::col(Series::from_i64_options(
            (0..n)
                .map(|_| {
                    if rng.chance(p) {
                        None
                    } else {
                        Some(rng.below(201) as i64 - 100)
                    }
                })
                .collect(),
        )));
        b.push(Expr::col(Series::from_bool_options(
            (0..n)
                .map(|_| {
                    if rng.chance(p) {
                        None
                    } else {
                        Some(rng.chance(0.5))
                    }
                })
                .collect(),
        )));
    }
    (f, i, b)
}

fn show(e: &Expr) -> String {
    match e {
        Expr::Col(s) => format!(
            "col{}{}",
            s.dtype(),
            if s.validity().is_some() { "?" } else { "" }
        ),
        Expr::Lit(s) => format!("{s:?}"),
        Expr::Op(op, a) => format!(
            "{op:?}({})",
            a.iter().map(|x| show(x)).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// The smallest subtree that disagrees with the reference at row `i`.
fn smallest(e: &Arc<Expr>, n: usize, i: usize) -> String {
    if let Expr::Op(_, args) = e.as_ref() {
        for a in args {
            if let (Ok(w), Ok(g)) = (reference(a, i), a.eval(n, false)) {
                if !same(val(&g, i), w) {
                    return smallest(a, n, i);
                }
            }
        }
        let kids: Vec<String> = args
            .iter()
            .map(|a| format!("{:?}", reference(a, i)))
            .collect();
        return format!("{} with operands {}", show(e), kids.join(", "));
    }
    show(e)
}

fn same(a: V, b: V) -> bool {
    match (a, b) {
        (V::F(x), V::F(y)) => x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}

fn check(e: &Arc<Expr>, n: usize) {
    let want: Result<Vec<V>, ()> = (0..n).map(|i| reference(e, i)).collect();
    let got = e.eval(n, false);
    match (want, got) {
        (Err(()), Ok(_)) => panic!("expected an error from {e:?}"),
        (Ok(_), Err(err)) => panic!("unexpected error {err} from {e:?}"),
        (Err(()), Err(_)) => {}
        (Ok(want), Ok(got)) => {
            assert_eq!(got.len(), n);
            for (i, w) in want.iter().enumerate() {
                let g = val(&got, i);
                if !same(g, *w) {
                    panic!(
                        "row {i}: got {g:?}, want {w:?}; smallest: {}",
                        smallest(e, n, i)
                    );
                }
            }
            // A result with no null carries no bitmap.
            if got.null_count() == 0 {
                assert!(got.validity().is_none(), "a dense result has no bitmap");
            }
        }
    }
}

#[test]
fn random_plans_match_the_reference_row_by_row() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..600 {
        let n = [1usize, 63, 64, 65, 1023, 1025, 3000][round % 7];
        let nulls = [0.0, 0.1, 0.5][round % 3];
        let (f, i, b) = columns(&mut rng, n, nulls);
        let mut g = Gen {
            rng: Rng(rng.next() | 1),
            f,
            i,
            b,
        };
        let t = [T::F, T::I, T::B][round % 3];
        let e = g.make(t, 4);
        check(&e, n);
    }
}

#[test]
fn several_roots_share_one_plan_and_match_one_by_one() {
    let mut rng = Rng(42);
    for _ in 0..60 {
        let n = 2500;
        let (f, i, b) = columns(&mut rng, n, 0.1);
        let mut g = Gen {
            rng: Rng(rng.next() | 1),
            f,
            i,
            b,
        };
        let shared = g.make(T::F, 2);
        let roots = vec![
            Expr::bin(BinOp::Add, shared.clone(), g.make(T::F, 2)),
            Expr::op(Op::Math(Math::Abs), vec![shared.clone()]),
            g.make(T::B, 3),
            shared.clone(),
            g.make(T::I, 2),
        ];
        let alone: Vec<Result<Series, _>> = roots.iter().map(|r| r.eval(n, false)).collect();
        let plan = Plan::new(&roots, None).unwrap();
        match plan.eval(false) {
            Ok(cols) => {
                for (c, a) in cols.iter().zip(alone) {
                    let a = a.expect("a plan that succeeds succeeds root by root");
                    for k in 0..n {
                        assert!(same(val(c, k), val(&a, k)));
                    }
                }
            }
            Err(_) => assert!(alone.iter().any(|a| a.is_err())),
        }
    }
}

#[test]
fn parallel_evaluation_is_the_sequential_one_bit_for_bit() {
    let mut rng = Rng(7);
    for round in 0..12 {
        let n = 300_001;
        let (f, i, b) = columns(&mut rng, n, 0.05);
        let mut g = Gen {
            rng: Rng(rng.next() | 1),
            f,
            i,
            b,
        };
        let e = g.make([T::F, T::I, T::B][round % 3], 3);
        let seq = e.eval(n, false);
        let par = e.eval(n, true);
        match (seq, par) {
            (Ok(s), Ok(p)) => {
                for k in (0..n).step_by(7) {
                    assert!(same(val(&s, k), val(&p, k)));
                }
                assert_eq!(s.null_count(), p.null_count());
            }
            (Err(a), Err(b)) => assert_eq!(a, b, "the first failing piece's error"),
            (a, b) => panic!("sequential {:?} vs parallel {:?}", a.is_ok(), b.is_ok()),
        }
    }
}

#[test]
fn reductions_fold_what_the_eager_kernels_compute() {
    let mut rng = Rng(99);
    for round in 0..120 {
        let n = [5000usize, 70_000, 200_001][round % 3];
        let (f, i, b) = columns(&mut rng, n, [0.0, 0.2][round % 2]);
        let mut g = Gen {
            rng: Rng(rng.next() | 1),
            f,
            i,
            b,
        };
        let e = g.make([T::F, T::I, T::B][round % 3], 3);
        let Ok(col) = e.eval(n, false) else { continue };
        let plan = Plan::new(std::slice::from_ref(&e), Some(n)).unwrap();
        for par in [false, true] {
            let count = plan.reduce(Reduce::Count, par).unwrap();
            assert_eq!(count, Scalar::I64(col.count() as i64));
            match col.dtype() {
                DType::Bool => {
                    assert_eq!(
                        plan.reduce(Reduce::Sum, par).unwrap(),
                        col.sum(false).unwrap()
                    );
                }
                _ => {
                    for (k, want) in [
                        (Reduce::Min, col.min().unwrap()),
                        (Reduce::Max, col.max().unwrap()),
                    ] {
                        let got = plan.reduce(k, par).unwrap();
                        let same = match (&got, &want) {
                            (Scalar::F64(x), Scalar::F64(y)) => x.to_bits() == y.to_bits(),
                            _ => got == want,
                        };
                        assert!(same, "{k:?}: {got:?} vs {want:?}");
                    }
                    let got = plan.reduce(Reduce::Sum, par);
                    let want = col.sum(false);
                    match (got, want) {
                        (Ok(Scalar::F64(x)), Ok(Scalar::F64(y))) => {
                            if !par && col.validity().is_none() && !x.is_nan() {
                                // No nulls: the eager sum's order exactly.
                                assert!(
                                    x.to_bits() == y.to_bits() || plan_may_null(&e),
                                    "{x} vs {y}"
                                );
                            }
                            assert!(
                                (x - y).abs() <= 1e-9 * y.abs().max(1.0)
                                    || (x.is_nan() && y.is_nan())
                                    || x == y,
                                "{x} vs {y}"
                            );
                        }
                        (Ok(a), Ok(b)) => assert_eq!(a, b),
                        (Err(_), Err(_)) => {}
                        (a, b) => panic!("{a:?} vs {b:?}"),
                    }
                }
            }
        }
    }
}

fn plan_may_null(e: &Expr) -> bool {
    match e {
        Expr::Col(s) => s.validity().is_some(),
        Expr::Lit(s) => matches!(s, Scalar::Null),
        Expr::Op(_, a) => a.iter().any(|x| plan_may_null(x)),
    }
}

#[test]
fn a_dense_float_sum_is_the_eager_sum_bit_for_bit() {
    let n = 100_003;
    let a = Series::from_f64((0..n).map(|i| (i as f64 * 0.37).sin()).collect());
    let b = Series::from_f64((0..n).map(|i| (i as f64 * 0.11).cos()).collect());
    let prod = Expr::bin(BinOp::Mul, Expr::col(a.clone()), Expr::col(b.clone()));
    let plan = Plan::new(std::slice::from_ref(&prod), None).unwrap();
    let eager = a
        .arith(olang_ods::ArithOp::Mul, &b, false)
        .unwrap()
        .sum(false)
        .unwrap();
    assert_eq!(plan.reduce(Reduce::Sum, false).unwrap(), eager);
}

#[test]
fn errors_name_what_the_eager_kernels_name() {
    let a = Series::from_f64(vec![1.0, 2.0]);
    let z = Series::from_f64_options(vec![None, Some(0.0)]);
    let div = Expr::bin(BinOp::Div, Expr::col(a.clone()), Expr::col(z.clone()));
    assert_eq!(
        div.eval(2, false).unwrap_err().to_string(),
        "Division by zero"
    );
    // Under a null the zero divisor does not compute.
    let z2 = Series::from_f64_options(vec![Some(0.0), Some(1.0)]);
    let n = Series::from_f64_options(vec![None, Some(3.0)]);
    let ok = Expr::bin(BinOp::Div, Expr::col(n), Expr::col(z2));
    assert!(ok.eval(2, false).is_ok());
    let big = Series::from_i64(vec![i64::MAX, 1]);
    let over = Expr::bin(BinOp::Add, Expr::col(big), Expr::lit(Scalar::I64(1)));
    assert_eq!(
        over.eval(2, false).unwrap_err().to_string(),
        "Integer overflow in addition"
    );
    let neg = Expr::op(Op::Math(Math::Sqrt), vec![Expr::lit(Scalar::F64(-1.0))]);
    assert_eq!(
        neg.eval(3, false).unwrap_err().to_string(),
        "sqrt: cannot take square root of negative number"
    );
}

#[test]
fn text_columns_compare_and_coalesce() {
    let s = Series::from_str_options(vec![Some("a".into()), None, Some("c".into())]);
    let t = Series::from_str_values(vec!["x".into(), "y".into(), "z".into()]);
    let eq = Expr::bin(
        BinOp::Eq,
        Expr::col(s.clone()),
        Expr::lit(Scalar::Str("c".into())),
    );
    let got = eq.eval(3, false).unwrap();
    assert_eq!(got.scalar_at(2), Scalar::Bool(true));
    assert_eq!(got.scalar_at(1), Scalar::Null);
    let filled = Expr::op(Op::Coalesce, vec![Expr::col(s), Expr::col(t)])
        .eval(3, false)
        .unwrap();
    assert_eq!(filled.scalar_at(1), Scalar::Str("y".into()));
}

/// Every operand kind: a column of each type (a Category's levels not in
/// text order) and a literal of each.
fn every_operand() -> Vec<(String, Arc<Expr>)> {
    let cat = olang_ods::category::to_category(
        &Series::from_str_options(vec![Some("m".into()), Some("s".into()), None]),
        Some(&["s".to_string(), "m".to_string(), "l".to_string()]),
        &|x| x.to_string(),
    )
    .unwrap();
    let cols = [
        (
            "F",
            Series::from_f64_options(vec![Some(1.5), None, Some(-2.0)]),
        ),
        ("I", Series::from_i64_options(vec![Some(1), Some(-4), None])),
        ("B", Series::from_bool(vec![true, false, true])),
        (
            "T",
            Series::from_str_options(vec![Some("s".into()), None, Some("2024-01-01".into())]),
        ),
        (
            "D",
            Series::from_date_options(vec![Some(19_000), None, Some(19_500)]),
        ),
        (
            "DT",
            Series::from_datetime_options(vec![Some(0), Some(86_400_000_000), None]),
        ),
        ("C", cat),
    ];
    let lits = [
        ("f", Scalar::F64(2.0)),
        ("i", Scalar::I64(3)),
        ("b", Scalar::Bool(true)),
        ("level", Scalar::Str("m".into())),
        ("text", Scalar::Str("zz".into())),
        ("iso", Scalar::Str("2024-01-01".into())),
        ("d", Scalar::Date(19_100)),
        ("dt", Scalar::Datetime(1)),
        ("null", Scalar::Null),
    ];
    cols.into_iter()
        .map(|(n, s)| (n.to_string(), Expr::col(s)))
        .chain(lits.into_iter().map(|(n, s)| (n.to_string(), Expr::lit(s))))
        .collect()
}

/// A Series an expression defers is evaluated later, where an error has
/// nowhere to go: so whatever types (`op_dtype`) and does not call itself
/// fallible must evaluate. Every operation over every pair (and triple)
/// of operand kinds.
#[test]
fn what_types_and_cannot_fail_evaluates() {
    let ops = every_operand();
    let bins = [
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Mod,
        BinOp::Pow,
        BinOp::Eq,
        BinOp::Ne,
        BinOp::Lt,
        BinOp::Le,
        BinOp::Gt,
        BinOp::Ge,
        BinOp::And,
        BinOp::Or,
        BinOp::Xor,
    ];
    let mut tried = 0;
    let mut try_one = |e: Arc<Expr>, what: String| {
        let typed = e.dtype();
        if typed.is_err() || e.fallible() || e.len_hint().is_none() {
            return;
        }
        tried += 1;
        let got =
            std::panic::catch_unwind(|| e.eval(3, false).map(|_| ()).map_err(|x| x.to_string()));
        match got {
            Ok(Ok(())) => {}
            Ok(Err(err)) => panic!(
                "{} types as {:?} and is not fallible, but raises: {}",
                what, typed, err
            ),
            Err(_) => panic!(
                "{} types as {:?} and is not fallible, but panics",
                what, typed
            ),
        }
    };
    let unary = [
        Op::Neg,
        Op::Not,
        Op::IsNull,
        Op::NotNull,
        Op::IsNan,
        Op::Math(Math::Exp),
        Op::Math(Math::Abs),
        Op::Round(1),
        Op::Cast(DType::F64),
        Op::Cast(DType::I64),
        Op::Cast(DType::Bool),
        Op::Temporal(olang_ods::fuse::TOp::Year, false),
        Op::Temporal(olang_ods::fuse::TOp::Hour, false),
        Op::Temporal(olang_ods::fuse::TOp::Date, false),
        Op::Temporal(olang_ods::fuse::TOp::AddMonths(1), false),
    ];
    for (na, a) in &ops {
        for op in unary {
            try_one(Expr::op(op, vec![a.clone()]), format!("{:?}({})", op, na));
        }
        for (nb, b) in &ops {
            try_one(
                Expr::op(
                    Op::Temporal(olang_ods::fuse::TOp::AddMonthsBy, false),
                    vec![a.clone(), b.clone()],
                ),
                format!("add_months({}, {})", na, nb),
            );
            try_one(
                Expr::op(
                    Op::Temporal(olang_ods::fuse::TOp::MonthsBetween, false),
                    vec![a.clone(), b.clone()],
                ),
                format!("months_between({}, {})", na, nb),
            );
            for op in bins {
                try_one(
                    Expr::bin(op, a.clone(), b.clone()),
                    format!("{:?}({}, {})", op, na, nb),
                );
            }
            try_one(
                Expr::op(Op::Coalesce, vec![a.clone(), b.clone()]),
                format!("coalesce({}, {})", na, nb),
            );
            for (nc, c) in &ops {
                for closed in [Closed::Both, Closed::Neither] {
                    try_one(
                        Expr::op(Op::Between(closed), vec![a.clone(), b.clone(), c.clone()]),
                        format!("between({}, {}, {})", na, nb, nc),
                    );
                }
                try_one(
                    Expr::op(Op::Where, vec![a.clone(), b.clone(), c.clone()]),
                    format!("where({}, {}, {})", na, nb, nc),
                );
                try_one(
                    Expr::op(Op::Clip, vec![a.clone(), b.clone(), c.clone()]),
                    format!("clip({}, {}, {})", na, nb, nc),
                );
            }
        }
    }
    assert!(tried > 500, "{} combinations tried", tried);
}

/// A Category compares in its levels' order against a level, or against
/// a Category of the same levels, as the eager kernel does; `+` joins
/// text.
#[test]
fn categories_compare_by_level_and_text_joins() {
    let ops = every_operand();
    let get = |n: &str| ops.iter().find(|(k, _)| k == n).unwrap().1.clone();
    let lt = Expr::bin(BinOp::Lt, get("C"), get("level"))
        .eval(3, false)
        .unwrap();
    // m < m, s < m, null
    assert_eq!(
        (0..3).map(|i| lt.scalar_at(i)).collect::<Vec<_>>(),
        vec![Scalar::Bool(false), Scalar::Bool(true), Scalar::Null]
    );
    let ge = Expr::bin(BinOp::Ge, get("level"), get("C"))
        .eval(3, false)
        .unwrap();
    assert_eq!(ge.scalar_at(1), Scalar::Bool(true));
    let between = Expr::op(
        Op::Between(Closed::Both),
        vec![get("C"), Expr::lit(Scalar::Str("s".into())), get("level")],
    );
    assert!(!between.fallible());
    assert_eq!(
        between.eval(3, false).unwrap().scalar_at(0),
        Scalar::Bool(true)
    );
    // An ordering against text that is no level has no answer, and says so.
    let bad = Expr::bin(BinOp::Lt, get("C"), get("text"));
    assert!(bad.fallible());
    assert!(
        bad.eval(3, false)
            .unwrap_err()
            .to_string()
            .contains("not a level")
    );
    let ne = Expr::bin(BinOp::Ne, get("C"), get("text"))
        .eval(3, false)
        .unwrap();
    assert_eq!(ne.scalar_at(0), Scalar::Bool(true));
    let joined = Expr::bin(BinOp::Add, get("C"), Expr::lit(Scalar::Str("!".into())))
        .eval(3, false)
        .unwrap();
    assert_eq!(joined.scalar_at(0), Scalar::Str("m!".into()));
    assert_eq!(joined.scalar_at(2), Scalar::Null);
    let both = Expr::bin(BinOp::Add, get("T"), get("T"))
        .eval(3, false)
        .unwrap();
    assert_eq!(both.scalar_at(0), Scalar::Str("ss".into()));
}

//! The expression engine and the column kernels of the data-ergonomics
//! round, each against the eager kernels it replaces or the obvious
//! composition, sequential and parallel. Run with
//! `cargo bench -p olang-ods --bench expr` (`-- 1000000` for another
//! size). `benches/expr.py` times the NumPy/pandas side of each row;
//! docs/performance.md records both.

use olang_ods::column::Unmapped;
use olang_ods::fuse::{BinOp, Expr, Math, Op, Plan, Reduce};
use olang_ods::{AggOp, ArithOp, Frame, Scalar, Series};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

/// Milliseconds: the fastest of seven rounds.
fn time(mut f: impl FnMut()) -> f64 {
    f();
    let mut best = f64::MAX;
    for _ in 0..7 {
        let t0 = Instant::now();
        f();
        best = best.min(t0.elapsed().as_secs_f64() * 1e3);
    }
    best
}

fn row(name: &str, seq: f64, par: f64) {
    println!("{name:<34} {seq:>9.2} {par:>9.2}");
}

fn pseudo(n: usize, seed: u64) -> impl Iterator<Item = u64> {
    let mut x = seed | 1;
    (0..n).map(move |_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    })
}

fn floats(n: usize, seed: u64, nulls: f64) -> Series {
    Series::from_f64_options(
        pseudo(n, seed)
            .map(|x| {
                if ((x >> 40) % 1000) as f64 / 1000.0 < nulls {
                    None
                } else {
                    Some((x % 1_000_000) as f64 / 1000.0 - 500.0)
                }
            })
            .collect(),
    )
}

fn plan(roots: &[Arc<Expr>]) -> Plan {
    Plan::new(roots, None).unwrap()
}

fn main() {
    let n: usize = std::env::args()
        .skip(1)
        .find_map(|a| a.parse().ok())
        .unwrap_or(10_000_000);
    println!("n = {n}; threads {}; milliseconds", rayon::current_num_threads());
    println!("{:<34} {:>9} {:>9}", "kernel", "seq", "par");
    let a = floats(n, 1, 0.0);
    let b = floats(n, 2, 0.0);
    let c = floats(n, 3, 0.0);
    let an = floats(n, 4, 0.1);
    let bn = floats(n, 5, 0.1);
    let ia = Series::from_i64(pseudo(n, 6).map(|x| (x % 1000) as i64).collect());
    let ib = Series::from_i64(pseudo(n, 7).map(|x| (x % 1000) as i64).collect());
    let col = |s: &Series| Expr::col(s.clone());
    let f = |x: f64| Expr::lit(Scalar::F64(x));

    // One operation: the engine against the eager kernel.
    row(
        "eager a * b",
        time(|| drop(black_box(a.arith(ArithOp::Mul, &b, false).unwrap()))),
        time(|| drop(black_box(a.arith(ArithOp::Mul, &b, true).unwrap()))),
    );
    let mul = plan(&[Expr::bin(BinOp::Mul, col(&a), col(&b))]);
    row(
        "expr a * b",
        time(|| drop(black_box(mul.eval(false).unwrap()))),
        time(|| drop(black_box(mul.eval(true).unwrap()))),
    );
    row(
        "eager a * 2.5",
        time(|| drop(black_box(a.arith_scalar(ArithOp::Mul, Scalar::F64(2.5), false, false).unwrap()))),
        time(|| drop(black_box(a.arith_scalar(ArithOp::Mul, Scalar::F64(2.5), false, true).unwrap()))),
    );
    let muls = plan(&[Expr::bin(BinOp::Mul, col(&a), f(2.5))]);
    row(
        "expr a * 2.5",
        time(|| drop(black_box(muls.eval(false).unwrap()))),
        time(|| drop(black_box(muls.eval(true).unwrap()))),
    );
    row(
        "eager a * b (10% nulls)",
        time(|| drop(black_box(an.arith(ArithOp::Mul, &bn, false).unwrap()))),
        time(|| drop(black_box(an.arith(ArithOp::Mul, &bn, true).unwrap()))),
    );
    let muln = plan(&[Expr::bin(BinOp::Mul, col(&an), col(&bn))]);
    row(
        "expr a * b (10% nulls)",
        time(|| drop(black_box(muln.eval(false).unwrap()))),
        time(|| drop(black_box(muln.eval(true).unwrap()))),
    );
    row(
        "eager map exp",
        time(|| drop(black_box(a.map_unary("exp").unwrap()))),
        f64::NAN,
    );
    let exp = plan(&[Expr::op(Op::Math(Math::Exp), vec![col(&a)])]);
    row(
        "expr exp",
        time(|| drop(black_box(exp.eval(false).unwrap()))),
        time(|| drop(black_box(exp.eval(true).unwrap()))),
    );
    for (name, m) in [("sqrt", Math::Abs), ("sigmoid", Math::Sigmoid), ("ln", Math::Ln)] {
        let src = if name == "ln" {
            Expr::op(Op::Math(Math::Abs), vec![col(&a)])
        } else {
            col(&a)
        };
        let src = if name == "ln" {
            Expr::bin(BinOp::Add, src, f(1.0))
        } else {
            src
        };
        let m = if name == "sqrt" { Math::Sqrt } else { m };
        let src = if name == "sqrt" {
            Expr::op(Op::Math(Math::Abs), vec![col(&a)])
        } else {
            src
        };
        let p = plan(&[Expr::op(Op::Math(m), vec![src])]);
        row(
            &format!("expr {name}"),
            time(|| drop(black_box(p.eval(false).unwrap()))),
            time(|| drop(black_box(p.eval(true).unwrap()))),
        );
    }

    // Chains: eager, one materialization per operator, against fused.
    let eager_chain = |nulls: bool, par: bool| {
        let (x, y) = if nulls { (&an, &bn) } else { (&a, &b) };
        let t = x.arith(ArithOp::Mul, y, par).unwrap();
        let t = t.arith(ArithOp::Add, &c, par).unwrap();
        let t = t.arith_scalar(ArithOp::Mul, Scalar::F64(0.5), false, par).unwrap();
        t.arith_scalar(ArithOp::Sub, Scalar::F64(1.0), false, par).unwrap()
    };
    let chain = |x: &Series, y: &Series| {
        Expr::bin(
            BinOp::Sub,
            Expr::bin(
                BinOp::Mul,
                Expr::bin(BinOp::Add, Expr::bin(BinOp::Mul, col(x), col(y)), col(&c)),
                f(0.5),
            ),
            f(1.0),
        )
    };
    row(
        "eager (a*b + c)*0.5 - 1",
        time(|| drop(black_box(eager_chain(false, false)))),
        time(|| drop(black_box(eager_chain(false, true)))),
    );
    let p = plan(&[chain(&a, &b)]);
    row(
        "fused (a*b + c)*0.5 - 1",
        time(|| drop(black_box(p.eval(false).unwrap()))),
        time(|| drop(black_box(p.eval(true).unwrap()))),
    );
    row(
        "eager same, 10% nulls",
        time(|| drop(black_box(eager_chain(true, false)))),
        time(|| drop(black_box(eager_chain(true, true)))),
    );
    let p = plan(&[chain(&an, &bn)]);
    row(
        "fused same, 10% nulls",
        time(|| drop(black_box(p.eval(false).unwrap()))),
        time(|| drop(black_box(p.eval(true).unwrap()))),
    );
    // Int chain: i * 3 + j (checked) against eager.
    row(
        "eager i*3 + j (Int)",
        time(|| {
            let t = ia.arith_scalar(ArithOp::Mul, Scalar::I64(3), false, false).unwrap();
            drop(black_box(t.arith(ArithOp::Add, &ib, false).unwrap()))
        }),
        f64::NAN,
    );
    let p = plan(&[Expr::bin(
        BinOp::Add,
        Expr::bin(BinOp::Mul, col(&ia), Expr::lit(Scalar::I64(3))),
        col(&ib),
    )]);
    row(
        "fused i*3 + j (Int)",
        time(|| drop(black_box(p.eval(false).unwrap()))),
        time(|| drop(black_box(p.eval(true).unwrap()))),
    );
    // Reductions after an expression.
    row(
        "eager sum(a*b)",
        time(|| drop(black_box(a.arith(ArithOp::Mul, &b, false).unwrap().sum(false).unwrap()))),
        time(|| drop(black_box(a.arith(ArithOp::Mul, &b, true).unwrap().sum(true).unwrap()))),
    );
    let p = plan(&[Expr::bin(BinOp::Mul, col(&a), col(&b))]);
    row(
        "fused sum(a*b)",
        time(|| drop(black_box(p.reduce(Reduce::Sum, false).unwrap()))),
        time(|| drop(black_box(p.reduce(Reduce::Sum, true).unwrap()))),
    );
    row(
        "eager dot(a, b)",
        time(|| { black_box(a.dot(&b, false).unwrap()); }),
        time(|| { black_box(a.dot(&b, true).unwrap()); }),
    );
    let mse = plan(&[{
        let d = Expr::bin(BinOp::Sub, col(&a), col(&b));
        Expr::bin(BinOp::Mul, d.clone(), d)
    }]);
    row(
        "fused mean((a-b)^2) (rmse)",
        time(|| drop(black_box(mse.reduce(Reduce::Mean, false).unwrap()))),
        time(|| drop(black_box(mse.reduce(Reduce::Mean, true).unwrap()))),
    );
    let mask = plan(&[Expr::bin(BinOp::Gt, col(&a), f(0.0))]);
    row(
        "fused mean(a > 0)",
        time(|| drop(black_box(mask.reduce(Reduce::Mean, false).unwrap()))),
        time(|| drop(black_box(mask.reduce(Reduce::Mean, true).unwrap()))),
    );
    // Log loss: -mean(y ln p + (1-y) ln(1-p)), p = sigmoid(a / 100).
    let p_ = Expr::op(Op::Math(Math::Sigmoid), vec![Expr::bin(BinOp::Div, col(&a), f(100.0))]);
    let y = Expr::op(
        Op::Cast(olang_ods::DType::F64),
        vec![Expr::bin(BinOp::Gt, col(&b), f(0.0))],
    );
    let ll = Expr::bin(
        BinOp::Add,
        Expr::bin(BinOp::Mul, y.clone(), Expr::op(Op::Math(Math::Ln), vec![p_.clone()])),
        Expr::bin(
            BinOp::Mul,
            Expr::bin(BinOp::Sub, f(1.0), y),
            Expr::op(Op::Math(Math::Ln), vec![Expr::bin(BinOp::Sub, f(1.0), p_)]),
        ),
    );
    let ll = plan(&[ll]);
    row(
        "fused log loss (sigmoid, 2 ln)",
        time(|| drop(black_box(ll.reduce(Reduce::Mean, false).unwrap()))),
        time(|| drop(black_box(ll.reduce(Reduce::Mean, true).unwrap()))),
    );

    // Conditionals and nulls.
    let wh = plan(&[Expr::op(Op::Where, vec![Expr::bin(BinOp::Gt, col(&a), f(0.0)), col(&a), col(&b)])]);
    row(
        "where(a > 0, a, b)",
        time(|| drop(black_box(wh.eval(false).unwrap()))),
        time(|| drop(black_box(wh.eval(true).unwrap()))),
    );
    let cl = plan(&[Expr::op(Op::Clip, vec![col(&a), f(-100.0), f(100.0)])]);
    row(
        "clip(a, -100, 100)",
        time(|| drop(black_box(cl.eval(false).unwrap()))),
        time(|| drop(black_box(cl.eval(true).unwrap()))),
    );
    let co = plan(&[Expr::op(Op::Coalesce, vec![col(&an), col(&b)])]);
    row(
        "coalesce(a?, b)",
        time(|| drop(black_box(co.eval(false).unwrap()))),
        time(|| drop(black_box(co.eval(true).unwrap()))),
    );
    row(
        "eager fill_null(a?, 0.0)",
        time(|| drop(black_box(an.clone().fill_null(Scalar::F64(0.0)).unwrap()))),
        f64::NAN,
    );
    let fz = plan(&[Expr::op(Op::Coalesce, vec![col(&an), f(0.0)])]);
    row(
        "fused fill_null(a?, 0.0)",
        time(|| drop(black_box(fz.eval(false).unwrap()))),
        time(|| drop(black_box(fz.eval(true).unwrap()))),
    );
    let bt = plan(&[Expr::op(Op::Between(olang_ods::fuse::Closed::Both), vec![col(&a), f(-100.0), f(100.0)])]);
    row(
        "between(a, -100, 100)",
        time(|| drop(black_box(bt.eval(false).unwrap()))),
        time(|| drop(black_box(bt.eval(true).unwrap()))),
    );
    row("fill ffill", time(|| drop(black_box(an.fill_direction(true, None)))), f64::NAN);
    row("interpolate", time(|| drop(black_box(an.interpolate(None).unwrap()))), f64::NAN);
    row("diff", time(|| drop(black_box(a.diff(1).unwrap()))), f64::NAN);
    let pos = plan(&[Expr::bin(BinOp::Add, Expr::op(Op::Math(Math::Abs), vec![col(&c)]), f(1.0))])
        .eval(false)
        .unwrap()
        .pop()
        .unwrap();
    row("pct_change", time(|| drop(black_box(pos.pct_change(1).unwrap()))), f64::NAN);
    row("cumprod", time(|| drop(black_box(a.cumprod().unwrap()))), f64::NAN);
    row("argmax", time(|| { black_box(a.arg_extreme(true).unwrap()); }), f64::NAN);
    row("mode (Int, 1000 values)", time(|| drop(black_box(ia.mode().unwrap()))), f64::NAN);
    let sorted = a.sort(true).unwrap();
    row(
        "searchsorted x 1000",
        time(|| {
            for k in 0..1000 {
                black_box(sorted.searchsorted(&Scalar::F64(k as f64 - 500.0), false).unwrap());
            }
        }),
        f64::NAN,
    );
    row(
        "sort descending",
        time(|| drop(black_box(a.sort_by_dir(true, false).unwrap()))),
        time(|| drop(black_box(a.sort_by_dir(true, true).unwrap()))),
    );
    row(
        "argsort descending",
        {
            olang_ods::set_parallel_enabled(false);
            let t = time(|| drop(black_box(a.argsort_by(true).unwrap())));
            olang_ods::set_parallel_enabled(true);
            t
        },
        time(|| drop(black_box(a.argsort_by(true).unwrap()))),
    );
    let keys = Series::from_str_values((0..1000).map(|k| format!("k{k}")).collect());
    let codes = Series::from_i64(pseudo(n, 9).map(|x| (x % 1000) as i64).collect());
    let labels = keys.take(&codes).unwrap();
    let from = keys.clone();
    let to = Series::from_str_values((0..1000).map(|k| format!("v{k}")).collect());
    row(
        "recode (1000-key map, text)",
        time(|| drop(black_box(labels.recode(&from, &to, &Unmapped::Keep).unwrap()))),
        f64::NAN,
    );
    let g = Series::from_i64(pseudo(n, 10).map(|x| (x % 1000) as i64).collect());
    let run = |op| {
        olang_ods::set_parallel_enabled(false);
        let s = time(|| drop(black_box(olang_ods::frame::over(&a, &[&g], op).unwrap())));
        olang_ods::set_parallel_enabled(true);
        let p = time(|| drop(black_box(olang_ods::frame::over(&a, &[&g], op).unwrap())));
        (s, p)
    };
    let (s, p) = run(AggOp::Mean);
    row("over mean (1000 groups)", s, p);
    let (s, p) = run(AggOp::Median);
    row("over median (1000 groups)", s, p);
    let f = Frame::new(vec![("g".into(), g.clone()), ("a".into(), a.clone())]).unwrap();
    let (s, p) = {
        olang_ods::set_parallel_enabled(false);
        let s = time(|| drop(black_box(f.sort_by_keys(&[(&g, false), (&a, true)]).unwrap())));
        olang_ods::set_parallel_enabled(true);
        let p = time(|| drop(black_box(f.sort_by_keys(&[(&g, false), (&a, true)]).unwrap())));
        (s, p)
    };
    row("sort_by [g asc, a desc]", s, p);
}

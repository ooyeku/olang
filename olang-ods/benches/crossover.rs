//! Where automatic parallelism starts to pay, kernel class by kernel
//! class: each kernel timed with its sequential path and its parallel
//! path forced, across sizes, so the thresholds in `ParClass` and the
//! frame's own (`PAR_GROUPBY_ROWS`, `PAR_JOIN_ROWS`, `PAR_TAKE_ROWS`)
//! can be read off the table. Run with
//! `cargo bench -p olang-ods --bench crossover` (`-- 15000,1000000` for
//! other sizes). docs/performance.md records the result.
//!
//! The frame kernels decide for themselves; their two columns are the
//! engine's switch off and on (`set_parallel_enabled`).

use olang_ods::{AggOp, AggSpec, ArithOp, CmpOp, Frame, JoinHow, Scalar, Series};
use std::hint::black_box;
use std::time::Instant;

/// One call's time in microseconds: the fastest of five rounds, each
/// repeating the call until ~20M elements were touched.
fn time(n: usize, mut f: impl FnMut()) -> f64 {
    let reps = (20_000_000 / n).max(2);
    let mut best = f64::MAX;
    for _ in 0..5 {
        let t0 = Instant::now();
        for _ in 0..reps {
            f();
        }
        best = best.min(t0.elapsed().as_secs_f64() * 1e6 / reps as f64);
    }
    best
}

fn row(name: &str, n: usize, seq: f64, par: f64) {
    println!(
        "{name:<14} {n:>10} {seq:>12.1} {par:>12.1} {:>8.2}x",
        seq / par
    );
}

fn pseudo(n: usize, modulo: u64) -> Vec<u64> {
    let mut x: u64 = 0x243F6A8885A308D3;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % modulo
        })
        .collect()
}

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .find(|a| a.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|a| a.split(',').filter_map(|s| s.parse().ok()).collect())
        .unwrap_or_else(|| {
            vec![
                15_000, 50_000, 100_000, 250_000, 500_000, 1_000_000, 2_000_000, 4_000_000,
                10_000_000,
            ]
        });
    println!(
        "threads {}; microseconds per call",
        rayon::current_num_threads()
    );
    println!(
        "{:<14} {:>10} {:>12} {:>12} {:>9}",
        "kernel", "n", "sequential", "parallel", "speedup"
    );
    for &n in &sizes {
        let a = Series::from_f64((0..n).map(|i| 1.0 + i as f64 / n as f64).collect());
        let b = Series::from_f64((0..n).map(|i| 3.0 - i as f64 / n as f64).collect());
        let shuffled = Series::from_f64(pseudo(n, 1 << 30).into_iter().map(|x| x as f64).collect());
        for (name, op) in [("mul", ArithOp::Mul), ("div", ArithOp::Div)] {
            let seq = time(n, || {
                black_box(a.arith(op, &b, false).unwrap());
            });
            let par = time(n, || {
                black_box(a.arith(op, &b, true).unwrap());
            });
            row(name, n, seq, par);
        }
        let leaf = |s: &Series| match s {
            Series::F64 { values, .. } => olang_ods::fuse::Expr::Leaf(values.clone()),
            _ => unreachable!(),
        };
        let chain = olang_ods::fuse::Expr::Bin(
            ArithOp::Add,
            std::sync::Arc::new(olang_ods::fuse::Expr::Bin(
                ArithOp::Mul,
                std::sync::Arc::new(leaf(&a)),
                std::sync::Arc::new(leaf(&b)),
            )),
            std::sync::Arc::new(olang_ods::fuse::Expr::Const(1.0)),
        );
        row(
            "fused a*b+1",
            n,
            time(n, || {
                black_box(chain.eval(n, false));
            }),
            time(n, || {
                black_box(chain.eval(n, true));
            }),
        );
        let scalar_chain = olang_ods::fuse::Expr::Bin(
            ArithOp::Add,
            std::sync::Arc::new(leaf(&a)),
            std::sync::Arc::new(olang_ods::fuse::Expr::Const(1.5)),
        );
        row(
            "fused a+1.5",
            n,
            time(n, || {
                black_box(scalar_chain.eval(n, false));
            }),
            time(n, || {
                black_box(scalar_chain.eval(n, true));
            }),
        );
        row(
            "add scalar",
            n,
            time(n, || {
                black_box(a.arith_scalar(ArithOp::Add, Scalar::F64(1.5), false, false).unwrap());
            }),
            time(n, || {
                black_box(a.arith_scalar(ArithOp::Add, Scalar::F64(1.5), false, true).unwrap());
            }),
        );
        row(
            "sum",
            n,
            time(n, || {
                black_box(a.sum(false).unwrap());
            }),
            time(n, || {
                black_box(a.sum(true).unwrap());
            }),
        );
        row(
            "std",
            n,
            time(n, || {
                black_box(a.std(false).unwrap());
            }),
            time(n, || {
                black_box(a.std(true).unwrap());
            }),
        );
        row(
            "dot",
            n,
            time(n, || {
                black_box(a.dot(&b, false).unwrap());
            }),
            time(n, || {
                black_box(a.dot(&b, true).unwrap());
            }),
        );
        row(
            "sort",
            n,
            time(n, || {
                black_box(shuffled.sort(false).unwrap());
            }),
            time(n, || {
                black_box(shuffled.sort(true).unwrap());
            }),
        );
        // compare has no parallel path: one column, for the record
        let cmp = time(n, || {
            black_box(a.compare_scalar(CmpOp::Gt, Scalar::F64(1.5), false).unwrap());
        });
        row("compare", n, cmp, cmp);

        // the frame's own decisions, switch off and on
        let keys = Series::from_i64(pseudo(n, 1000).into_iter().map(|x| x as i64).collect());
        let frame = Frame::new(vec![
            ("key".to_string(), keys.clone()),
            ("value".to_string(), shuffled.clone()),
        ])
        .unwrap();
        let aggs = [AggSpec {
            out_name: "mu".to_string(),
            op: AggOp::Mean,
            col: "value".to_string(),
        }];
        let lookup = Frame::new(vec![
            (
                "key".to_string(),
                Series::from_i64((0..1000).collect()),
            ),
            (
                "w".to_string(),
                Series::from_f64((0..1000).map(|i| i as f64).collect()),
            ),
        ])
        .unwrap();
        let mask = shuffled
            .compare_scalar(CmpOp::Gt, Scalar::F64((1u64 << 29) as f64), false)
            .unwrap();
        let on_off = |name: &str, f: &dyn Fn()| {
            olang_ods::set_parallel_enabled(false);
            let seq = time(n, f);
            olang_ods::set_parallel_enabled(true);
            let par = time(n, f);
            row(name, n, seq, par);
        };
        on_off("group_by", &|| {
            black_box(frame.group_by(&["key".to_string()], &aggs).unwrap());
        });
        on_off("join", &|| {
            black_box(frame.join(&lookup, "key", "key", JoinHow::Inner).unwrap());
        });
        on_off("sort_by", &|| {
            black_box(frame.sort_by("value", false).unwrap());
        });
        on_off("filter", &|| {
            black_box(frame.filter(&mask).unwrap());
        });
        println!();
    }
}

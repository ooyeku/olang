#![cfg(feature = "parallel")]
//! The parallel policy (`ParClass`, `par_worth`) and the kernels it
//! steers: a parallel path computes what the sequential one does, bit
//! for bit, and a fused chain what the eager kernels do.

use olang_ods::fuse::{Expr, Plan};
use olang_ods::{ArithOp, ParClass, Scalar, Series, par_worth, parallel_enabled};
use std::sync::Arc;

fn values(s: &Series) -> Vec<f64> {
    match s {
        Series::F64 { values, .. } => values.as_ref().clone(),
        other => panic!("expected F64, got {:?}", other.dtype()),
    }
}

fn leaf(v: &[f64]) -> Arc<Expr> {
    Expr::col(Series::from_f64(v.to_vec()))
}

fn konst(c: f64) -> Arc<Expr> {
    Expr::lit(Scalar::F64(c))
}

#[test]
fn small_columns_never_fan_out_and_large_ones_do() {
    for class in [
        ParClass::Elementwise,
        ParClass::ElementwiseScalar,
        ParClass::Reduce,
        ParClass::Sort,
    ] {
        assert!(!par_worth(class, 15_000), "{class:?} at 15,000");
        assert!(
            !par_worth(class, class.min_rows() - 1),
            "{class:?} below its threshold"
        );
        assert_eq!(par_worth(class, class.min_rows()), parallel_enabled());
    }
    assert!(ParClass::Elementwise.min_rows() >= 250_000);
    assert!(ParClass::Reduce.min_rows() >= 500_000);
}

#[test]
fn a_kernel_on_a_pool_worker_stays_sequential() {
    // Inside the pool (an olang par_map stage) nothing fans out again.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    assert!(!pool.install(parallel_enabled));
    assert!(!pool.install(|| par_worth(ParClass::Sort, 10_000_000)));
}

#[test]
fn parallel_arithmetic_is_the_sequential_arithmetic() {
    let n = 600_000;
    let a: Vec<f64> = (0..n).map(|i| (i as f64).sin() * 3.0 + 0.1).collect();
    let b: Vec<f64> = (0..n).map(|i| (i as f64).cos() + 2.0).collect();
    let (sa, sb) = (Series::from_f64(a.clone()), Series::from_f64(b.clone()));
    for op in [ArithOp::Add, ArithOp::Sub, ArithOp::Mul, ArithOp::Div] {
        let seq = values(&sa.arith(op, &sb, false).unwrap());
        let par = values(&sa.arith(op, &sb, true).unwrap());
        assert_eq!(seq, par, "{op:?}");
        for swapped in [false, true] {
            let seq = values(
                &sa.arith_scalar(op, Scalar::F64(1.75), swapped, false)
                    .unwrap(),
            );
            let par = values(
                &sa.arith_scalar(op, Scalar::F64(1.75), swapped, true)
                    .unwrap(),
            );
            assert_eq!(seq, par, "{op:?} swapped {swapped}");
        }
    }
    let seq = sa.sort(false).unwrap();
    let par = sa.sort(true).unwrap();
    assert_eq!(values(&seq), values(&par));
}

#[test]
fn a_fused_chain_is_the_eager_kernels_bit_for_bit() {
    // Lengths off the chunk size, so the last chunk is partial.
    for n in [1, 1023, 1025, 4097, 300_001] {
        let a: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin() + 0.5).collect();
        let b: Vec<f64> = (0..n).map(|i| (i as f64 * 0.11).cos() - 0.25).collect();
        let c: Vec<f64> = (0..n).map(|i| i as f64 * 1e-3).collect();
        let (sa, sb, sc) = (
            Series::from_f64(a.clone()),
            Series::from_f64(b.clone()),
            Series::from_f64(c.clone()),
        );
        let eager = |x: &Series, op, y: &Series| x.arith(op, y, false).unwrap();
        let eager_s = |x: &Series, op, k: f64, swapped| {
            x.arith_scalar(op, Scalar::F64(k), swapped, false).unwrap()
        };
        let ops = [ArithOp::Add, ArithOp::Sub, ArithOp::Mul];
        for op in ops {
            for op2 in ops {
                let cases: Vec<(Arc<Expr>, Series)> = vec![
                    // a op b
                    (
                        Expr::bin(op.into(), leaf(&a), leaf(&b)),
                        eager(&sa, op, &sb),
                    ),
                    // a op 2.5, 2.5 op a
                    (
                        Expr::bin(op.into(), leaf(&a), konst(2.5)),
                        eager_s(&sa, op, 2.5, false),
                    ),
                    (
                        Expr::bin(op.into(), konst(2.5), leaf(&a)),
                        eager_s(&sa, op, 2.5, true),
                    ),
                    // (a op b) op2 1.5, 1.5 op2 (a op b)
                    (
                        Expr::bin(
                            op2.into(),
                            Expr::bin(op.into(), leaf(&a), leaf(&b)),
                            konst(1.5),
                        ),
                        eager_s(&eager(&sa, op, &sb), op2, 1.5, false),
                    ),
                    (
                        Expr::bin(
                            op2.into(),
                            konst(1.5),
                            Expr::bin(op.into(), leaf(&a), leaf(&b)),
                        ),
                        eager_s(&eager(&sa, op, &sb), op2, 1.5, true),
                    ),
                    // (a op b) op2 c, c op2 (a op b)
                    (
                        Expr::bin(
                            op2.into(),
                            Expr::bin(op.into(), leaf(&a), leaf(&b)),
                            leaf(&c),
                        ),
                        eager(&eager(&sa, op, &sb), op2, &sc),
                    ),
                    (
                        Expr::bin(
                            op2.into(),
                            leaf(&c),
                            Expr::bin(op.into(), leaf(&a), leaf(&b)),
                        ),
                        eager(&sc, op2, &eager(&sa, op, &sb)),
                    ),
                    // (a op 3.0) op2 (b op c): both sides chains
                    (
                        Expr::bin(
                            op2.into(),
                            Expr::bin(op.into(), leaf(&a), konst(3.0)),
                            Expr::bin(op.into(), leaf(&b), leaf(&c)),
                        ),
                        eager(&eager_s(&sa, op, 3.0, false), op2, &eager(&sb, op, &sc)),
                    ),
                ];
                for (expr, want) in cases {
                    let want = values(&want);
                    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
                    assert_eq!(
                        bits(&values(&expr.eval(n, false).unwrap())),
                        bits(&want),
                        "{expr:?} n={n}"
                    );
                    assert_eq!(
                        bits(&values(&expr.eval(n, true).unwrap())),
                        bits(&want),
                        "{expr:?} n={n} par"
                    );
                }
            }
        }
    }
}

#[test]
fn a_chain_over_one_column_is_classed_with_scalar_arithmetic() {
    let a = [1.0, 2.0];
    let one = Expr::bin(
        ArithOp::Add.into(),
        Expr::bin(ArithOp::Mul.into(), leaf(&a), konst(2.0)),
        konst(1.0),
    );
    let plan = Plan::new(&[one], None).unwrap();
    assert!(plan.par_rows() <= ParClass::ElementwiseScalar.min_rows());
    let col = leaf(&a);
    let two = Expr::bin(ArithOp::Mul.into(), col.clone(), col);
    // The same column twice is read once.
    let plan = Plan::new(&[two], None).unwrap();
    assert_eq!(plan.par_rows(), ParClass::ElementwiseScalar.min_rows());
    let three = Expr::bin(ArithOp::Mul.into(), leaf(&a), leaf(&[3.0, 4.0]));
    let plan = Plan::new(&[three], None).unwrap();
    assert_eq!(plan.par_rows(), ParClass::Elementwise.min_rows());
}

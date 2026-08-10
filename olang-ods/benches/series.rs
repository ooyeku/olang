//! Phase 1 acceptance benchmarks (docs/design/ods.md, B1/B2/B3/B5).
//! Run with `cargo bench -p olang-ods`. NumPy reference numbers are
//! collected separately and recorded in the design doc.

use criterion::{Criterion, criterion_group, criterion_main};
use olang_ods::{ArithOp, Scalar, Series};
use std::hint::black_box;

const N: usize = 10_000_000;

fn data_dense() -> Series {
    Series::from_f64((0..N).map(|i| (i % 1000) as f64 * 0.25).collect())
}

fn data_with_nulls() -> Series {
    // 10% nulls, deterministic pattern (B5).
    Series::from_f64_options(
        (0..N)
            .map(|i| {
                if i % 10 == 3 {
                    None
                } else {
                    Some((i % 1000) as f64 * 0.25)
                }
            })
            .collect(),
    )
}

fn bench_reductions(c: &mut Criterion) {
    let dense = data_dense();
    let nulls = data_with_nulls();
    let mut g = c.benchmark_group("reductions_10m");
    g.sample_size(20);
    g.bench_function("b1_sum_seq", |b| {
        b.iter(|| black_box(dense.sum(false).unwrap()))
    });
    g.bench_function("b1_sum_par", |b| {
        b.iter(|| black_box(dense.sum(true).unwrap()))
    });
    g.bench_function("b1_mean_seq", |b| {
        b.iter(|| black_box(dense.mean(false).unwrap()))
    });
    g.bench_function("b1_std_seq", |b| {
        b.iter(|| black_box(dense.std(false).unwrap()))
    });
    g.bench_function("b5_null_mean", |b| {
        b.iter(|| black_box(nulls.mean(false).unwrap()))
    });
    g.finish();
}

fn bench_elementwise(c: &mut Criterion) {
    let a = data_dense();
    let b2 = data_dense();
    let mut g = c.benchmark_group("elementwise_10m");
    g.sample_size(10);
    g.bench_function("b2_mul_add_seq", |bch| {
        bch.iter(|| {
            let prod = a.arith(ArithOp::Mul, &b2, false).unwrap();
            black_box(
                prod.arith_scalar(ArithOp::Add, Scalar::F64(1.0), false, false)
                    .unwrap(),
            )
        })
    });
    g.bench_function("b2_mul_add_par", |bch| {
        bch.iter(|| {
            let prod = a.arith(ArithOp::Mul, &b2, true).unwrap();
            black_box(
                prod.arith_scalar(ArithOp::Add, Scalar::F64(1.0), false, true)
                    .unwrap(),
            )
        })
    });
    g.finish();
}

fn bench_sort(c: &mut Criterion) {
    // Pseudo-random payload so the sort does real work.
    let mut x: u64 = 0x243F6A8885A308D3;
    let vals: Vec<f64> = (0..N)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 1_000_000) as f64
        })
        .collect();
    let s = Series::from_f64(vals);
    let mut g = c.benchmark_group("sort_10m");
    g.sample_size(10);
    g.bench_function("b3_sort_seq", |b| {
        b.iter(|| black_box(s.sort(false).unwrap()))
    });
    g.bench_function("b3_sort_par", |b| {
        b.iter(|| black_box(s.sort(true).unwrap()))
    });
    g.finish();
}

criterion_group!(benches, bench_reductions, bench_elementwise, bench_sort);
criterion_main!(benches);

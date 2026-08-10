//! B4 acceptance benchmark (docs/design/ods.md): OLS fit, 1M rows × 20
//! regressors, vs numpy.linalg.lstsq. Run with
//! `cargo bench -p olang-ods --bench ols`.

use criterion::{Criterion, criterion_group, criterion_main};
use olang_ods::{Series, stats};
use std::hint::black_box;

const N: usize = 1_000_000;
const K: usize = 20;

fn xorshift_stream(seed: u64, n: usize) -> Vec<f64> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 2_000_000) as f64 / 1_000_000.0 - 1.0
        })
        .collect()
}

fn bench_ols(c: &mut Criterion) {
    let cols: Vec<Series> = (0..K)
        .map(|j| Series::from_f64(xorshift_stream(0x9E3779B97F4A7C15 ^ (j as u64 + 1), N)))
        .collect();
    // y = 1 + sum_j (j/10) * x_j + noise-ish column
    let noise = xorshift_stream(0xDEADBEEF, N);
    let mut y = vec![0.0f64; N];
    for (r, yr) in y.iter_mut().enumerate() {
        let mut acc = 1.0 + 0.1 * noise[r];
        for (j, col) in cols.iter().enumerate() {
            if let olang_ods::Scalar::F64(v) = col.scalar_at(r) {
                acc += (j as f64 / 10.0) * v;
            }
        }
        *yr = acc;
    }
    let y = Series::from_f64(y);
    let col_refs: Vec<&Series> = cols.iter().collect();

    let mut g = c.benchmark_group("ols_1m_x20");
    g.sample_size(10);
    g.bench_function("b4_ols_seq", |b| {
        b.iter(|| black_box(stats::ols(&y, &col_refs, false).unwrap()))
    });
    g.bench_function("b4_ols_par", |b| {
        b.iter(|| black_box(stats::ols(&y, &col_refs, true).unwrap()))
    });
    g.finish();
}

criterion_group!(benches, bench_ols);
criterion_main!(benches);

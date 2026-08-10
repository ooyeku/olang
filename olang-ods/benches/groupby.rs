//! B6 acceptance benchmark (docs/design/ods.md): group-by aggregate,
//! 10M rows, 1k groups, vs Polars. Run with
//! `cargo bench -p olang-ods --bench groupby`.

use criterion::{Criterion, criterion_group, criterion_main};
use olang_ods::{AggOp, AggSpec, Frame, Series};
use std::hint::black_box;

const N: usize = 10_000_000;
const GROUPS: u64 = 1_000;

fn bench_groupby(c: &mut Criterion) {
    let mut x: u64 = 0x243F6A8885A308D3;
    let mut keys = Vec::with_capacity(N);
    let mut vals = Vec::with_capacity(N);
    for _ in 0..N {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        keys.push((x % GROUPS) as i64);
        vals.push((x % 10_000) as f64 * 0.01);
    }
    let frame = Frame::new(vec![
        ("key".to_string(), Series::from_i64(keys)),
        ("value".to_string(), Series::from_f64(vals)),
    ])
    .unwrap();
    let aggs = [
        AggSpec {
            out_name: "total".to_string(),
            op: AggOp::Sum,
            col: "value".to_string(),
        },
        AggSpec {
            out_name: "mu".to_string(),
            op: AggOp::Mean,
            col: "value".to_string(),
        },
    ];

    let mut g = c.benchmark_group("groupby_10m_1k");
    g.sample_size(10);
    g.bench_function("b6_sum_mean", |b| {
        b.iter(|| black_box(frame.group_by(&["key".to_string()], &aggs).unwrap()))
    });
    g.finish();
}

criterion_group!(benches, bench_groupby);
criterion_main!(benches);

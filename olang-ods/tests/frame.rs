//! Frame verb tests: construction, selection, sorting, group_by
//! aggregation (pinned against hand-computed references, including null
//! and string keys), and joins.

use olang_ods::{AggOp, AggSpec, Frame, JoinHow, Scalar, Series};

fn agg(out: &str, op: AggOp, col: &str) -> AggSpec {
    AggSpec {
        out_name: out.to_string(),
        op,
        col: col.to_string(),
    }
}

fn strings(v: &[&str]) -> Series {
    Series::from_str_values(v.iter().map(|s| s.to_string()).collect())
}

fn col_values(f: &Frame, name: &str) -> Vec<Scalar> {
    let c = f.column(name).unwrap();
    (0..c.len()).map(|i| c.scalar_at(i)).collect()
}

fn sales() -> Frame {
    Frame::new(vec![
        (
            "region".to_string(),
            strings(&["east", "west", "east", "north", "west", "east"]),
        ),
        (
            "revenue".to_string(),
            Series::from_f64(vec![10.0, 20.0, 30.0, 5.0, 15.0, 25.0]),
        ),
        (
            "units".to_string(),
            Series::from_i64(vec![1, 2, 3, 1, 2, 4]),
        ),
    ])
    .unwrap()
}

#[test]
fn construction_validates() {
    assert!(
        Frame::new(vec![
            ("a".to_string(), Series::from_i64(vec![1, 2])),
            ("a".to_string(), Series::from_i64(vec![3, 4])),
        ])
        .is_err()
    );
    assert!(
        Frame::new(vec![
            ("a".to_string(), Series::from_i64(vec![1, 2])),
            ("b".to_string(), Series::from_i64(vec![3])),
        ])
        .is_err()
    );
    let f = sales();
    assert_eq!(f.n_rows(), 6);
    assert_eq!(f.n_cols(), 3);
    assert!(f.column("nope").is_err());
}

#[test]
fn select_with_column_head() {
    let f = sales();
    let s = f.select(&["revenue".to_string()]).unwrap();
    assert_eq!(s.n_cols(), 1);

    let doubled = f
        .column("revenue")
        .unwrap()
        .arith_scalar(olang_ods::ArithOp::Mul, Scalar::F64(2.0), false, false)
        .unwrap();
    let f2 = f.with_column("double_rev", doubled).unwrap();
    assert_eq!(f2.n_cols(), 4);
    assert_eq!(
        f2.column("double_rev").unwrap().scalar_at(0),
        Scalar::F64(20.0)
    );
    // Replacement keeps position and count.
    let f3 = f2
        .with_column("units", Series::from_i64(vec![0; 6]))
        .unwrap();
    assert_eq!(f3.n_cols(), 4);

    let h = f.head(2);
    assert_eq!(h.n_rows(), 2);
    assert_eq!(f.head(100).n_rows(), 6);
}

#[test]
fn sort_by_with_nulls_and_descending() {
    let f = Frame::new(vec![
        (
            "k".to_string(),
            Series::from_f64_options(vec![Some(2.0), None, Some(1.0), Some(3.0)]),
        ),
        ("v".to_string(), Series::from_i64(vec![20, 99, 10, 30])),
    ])
    .unwrap();
    let asc = f.sort_by("k", false).unwrap();
    assert_eq!(
        col_values(&asc, "v"),
        vec![
            Scalar::I64(10),
            Scalar::I64(20),
            Scalar::I64(30),
            Scalar::I64(99)
        ]
    );
    let desc = f.sort_by("k", true).unwrap();
    assert_eq!(
        col_values(&desc, "v"),
        vec![
            Scalar::I64(30),
            Scalar::I64(20),
            Scalar::I64(10),
            Scalar::I64(99)
        ]
    );
}

#[test]
fn group_by_string_key_matches_reference() {
    let f = sales();
    let g = f
        .group_by(
            &["region".to_string()],
            &[
                agg("total", AggOp::Sum, "revenue"),
                agg("mu", AggOp::Mean, "revenue"),
                agg("n", AggOp::Count, "revenue"),
                agg("max_units", AggOp::Max, "units"),
            ],
        )
        .unwrap();
    // First-seen order: east, west, north.
    assert_eq!(
        col_values(&g, "region"),
        vec![
            Scalar::Str("east".to_string()),
            Scalar::Str("west".to_string()),
            Scalar::Str("north".to_string())
        ]
    );
    assert_eq!(
        col_values(&g, "total"),
        vec![Scalar::F64(65.0), Scalar::F64(35.0), Scalar::F64(5.0)]
    );
    assert_eq!(
        col_values(&g, "mu"),
        vec![Scalar::F64(65.0 / 3.0), Scalar::F64(17.5), Scalar::F64(5.0)]
    );
    assert_eq!(
        col_values(&g, "n"),
        vec![Scalar::I64(3), Scalar::I64(2), Scalar::I64(1)]
    );
    assert_eq!(
        col_values(&g, "max_units"),
        vec![Scalar::I64(4), Scalar::I64(2), Scalar::I64(1)]
    );
}

#[test]
fn group_by_null_key_is_its_own_group_and_null_values_skip() {
    let f = Frame::new(vec![
        (
            "k".to_string(),
            Series::from_i64_options(vec![Some(1), None, Some(1), None, Some(2)]),
        ),
        (
            "v".to_string(),
            Series::from_f64_options(vec![Some(10.0), Some(5.0), None, Some(7.0), Some(2.0)]),
        ),
    ])
    .unwrap();
    let g = f
        .group_by(
            &["k".to_string()],
            &[agg("s", AggOp::Sum, "v"), agg("n", AggOp::Count, "v")],
        )
        .unwrap();
    // Groups first-seen: 1, null, 2.
    assert_eq!(
        col_values(&g, "k"),
        vec![Scalar::I64(1), Scalar::Null, Scalar::I64(2)]
    );
    // Sum skips the null v in group 1; count counts rows.
    assert_eq!(
        col_values(&g, "s"),
        vec![Scalar::F64(10.0), Scalar::F64(12.0), Scalar::F64(2.0)]
    );
    assert_eq!(
        col_values(&g, "n"),
        vec![Scalar::I64(2), Scalar::I64(2), Scalar::I64(1)]
    );
}

#[test]
fn group_by_multi_key() {
    let f = Frame::new(vec![
        ("a".to_string(), strings(&["x", "x", "y", "x"])),
        ("b".to_string(), Series::from_i64(vec![1, 2, 1, 1])),
        ("v".to_string(), Series::from_i64(vec![10, 20, 30, 40])),
    ])
    .unwrap();
    let g = f
        .group_by(
            &["a".to_string(), "b".to_string()],
            &[agg("s", AggOp::Sum, "v")],
        )
        .unwrap();
    assert_eq!(g.n_rows(), 3);
    assert_eq!(
        col_values(&g, "s"),
        vec![Scalar::I64(50), Scalar::I64(20), Scalar::I64(30)]
    );
}

#[test]
fn inner_and_left_join() {
    let orders = Frame::new(vec![
        ("id".to_string(), Series::from_i64(vec![1, 2, 3, 4])),
        (
            "region".to_string(),
            strings(&["east", "west", "east", "south"]),
        ),
    ])
    .unwrap();
    let regions = Frame::new(vec![
        ("name".to_string(), strings(&["east", "west", "north"])),
        ("tax".to_string(), Series::from_f64(vec![0.07, 0.09, 0.05])),
    ])
    .unwrap();

    let inner = orders
        .join(&regions, "region", "name", JoinHow::Inner)
        .unwrap();
    assert_eq!(inner.n_rows(), 3); // south unmatched
    assert_eq!(
        col_values(&inner, "tax"),
        vec![Scalar::F64(0.07), Scalar::F64(0.09), Scalar::F64(0.07)]
    );

    let left = orders
        .join(&regions, "region", "name", JoinHow::Left)
        .unwrap();
    assert_eq!(left.n_rows(), 4);
    assert_eq!(left.column("tax").unwrap().scalar_at(3), Scalar::Null);
}

#[test]
fn join_duplicates_multiply_and_null_keys_never_match() {
    let l = Frame::new(vec![(
        "k".to_string(),
        Series::from_i64_options(vec![Some(1), None]),
    )])
    .unwrap();
    let r = Frame::new(vec![
        (
            "k".to_string(),
            Series::from_i64_options(vec![Some(1), Some(1), None]),
        ),
        ("v".to_string(), Series::from_i64(vec![10, 20, 99])),
    ])
    .unwrap();
    let inner = l.join(&r, "k", "k", JoinHow::Inner).unwrap();
    // left row 0 matches right rows 0 and 1; nulls never match nulls.
    assert_eq!(inner.n_rows(), 2);
    assert_eq!(
        col_values(&inner, "v"),
        vec![Scalar::I64(10), Scalar::I64(20)]
    );
    let left = l.join(&r, "k", "k", JoinHow::Left).unwrap();
    assert_eq!(left.n_rows(), 3);
    assert_eq!(left.column("v").unwrap().scalar_at(2), Scalar::Null);
}

#[test]
fn join_name_collision_gets_suffix() {
    let l = Frame::new(vec![
        ("k".to_string(), Series::from_i64(vec![1])),
        ("v".to_string(), Series::from_i64(vec![7])),
    ])
    .unwrap();
    let r = Frame::new(vec![
        ("k".to_string(), Series::from_i64(vec![1])),
        ("v".to_string(), Series::from_i64(vec![8])),
    ])
    .unwrap();
    let j = l.join(&r, "k", "k", JoinHow::Inner).unwrap();
    assert_eq!(col_values(&j, "v"), vec![Scalar::I64(7)]);
    assert_eq!(col_values(&j, "v_right"), vec![Scalar::I64(8)]);
}

#[test]
fn string_series_kernels() {
    let s = Series::from_str_options(vec![
        Some("banana".to_string()),
        None,
        Some("apple".to_string()),
    ]);
    let sorted = s.sort(false).unwrap();
    assert_eq!(sorted.scalar_at(0), Scalar::Str("apple".to_string()));
    assert_eq!(sorted.scalar_at(2), Scalar::Null);

    let mask = s
        .compare_scalar(
            olang_ods::CmpOp::Eq,
            Scalar::Str("apple".to_string()),
            false,
        )
        .unwrap();
    assert_eq!(mask.scalar_at(2), Scalar::Bool(true));
    assert_eq!(mask.scalar_at(1), Scalar::Null);

    let filled = s.clone().fill_null(Scalar::Str("?".to_string())).unwrap();
    assert_eq!(filled.scalar_at(1), Scalar::Str("?".to_string()));
    assert_eq!(s.min().unwrap(), Scalar::Str("apple".to_string()));
    assert!(s.sum(false).is_err());
}

/// DP1: a join above the parallel threshold (50k left rows) must produce
/// exactly the sequential result. This exercises the parallel probe under
/// `--features parallel` and the sequential path otherwise; the assertions
/// hold either way, so parallel == sequential is pinned. Left keys have
/// many duplicates and some non-matching rows to stress ordering and the
/// left-join null path.
#[test]
fn large_join_is_correct_and_order_stable() {
    let n: i64 = 120_000;
    // left key = row % 1000, so keys 0..999 each appear 120 times; keys
    // 1000+ never appear (all rows match on the inner join).
    let left = Frame::new(vec![
        (
            "k".to_string(),
            Series::from_i64((0..n).map(|i| i % 1000).collect()),
        ),
        ("v".to_string(), Series::from_i64((0..n).collect())),
    ])
    .unwrap();
    // right has keys 0..1499; only 0..999 match anything on the left.
    let right = Frame::new(vec![
        ("k".to_string(), Series::from_i64((0..1500).collect())),
        (
            "tag".to_string(),
            Series::from_i64((0..1500).map(|i| i * 10).collect()),
        ),
    ])
    .unwrap();

    let inner = left.join(&right, "k", "k", JoinHow::Inner).unwrap();
    assert_eq!(
        inner.n_rows(),
        n as usize,
        "every left row matches one right row"
    );
    // Rows stay in left order, so row i keeps left value i, and its tag is
    // (i % 1000) * 10.
    let v = inner.column("v").unwrap();
    let tag = inner.column("tag").unwrap();
    for &i in &[0usize, 1, 999, 1000, 50_000, 119_999] {
        assert_eq!(v.scalar_at(i), Scalar::I64(i as i64), "row {i} value");
        assert_eq!(
            tag.scalar_at(i),
            Scalar::I64(((i as i64) % 1000) * 10),
            "row {i} tag"
        );
    }

    // A left join with non-matching right keys keeps unmatched left rows.
    let sparse_right = Frame::new(vec![
        ("k".to_string(), Series::from_i64(vec![0, 1, 2])),
        ("tag".to_string(), Series::from_i64(vec![100, 101, 102])),
    ])
    .unwrap();
    let left_join = left.join(&sparse_right, "k", "k", JoinHow::Left).unwrap();
    assert_eq!(
        left_join.n_rows(),
        n as usize,
        "left join keeps all left rows"
    );
    // Row 3 has key 3, which is not in sparse_right → null tag.
    assert_eq!(left_join.column("tag").unwrap().scalar_at(3), Scalar::Null);
    // Row 0 has key 0 → tag 100.
    assert_eq!(
        left_join.column("tag").unwrap().scalar_at(0),
        Scalar::I64(100)
    );
}

/// Not a gate (timing is machine-dependent) — run manually with
/// `cargo test -p olang-ods --features parallel --test frame join_speedup
/// -- --ignored --nocapture` to see the multi-core win on the join probe.
#[test]
#[ignore]
#[cfg(feature = "parallel")]
fn join_speedup() {
    use std::time::Instant;
    let n: i64 = 4_000_000;
    let left = Frame::new(vec![
        (
            "k".to_string(),
            Series::from_i64((0..n).map(|i| i % 100_000).collect()),
        ),
        ("v".to_string(), Series::from_i64((0..n).collect())),
    ])
    .unwrap();
    let right = Frame::new(vec![
        ("k".to_string(), Series::from_i64((0..100_000).collect())),
        ("t".to_string(), Series::from_i64((0..100_000).collect())),
    ])
    .unwrap();

    let time_with = |threads: usize| -> f64 {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let t = Instant::now();
            let out = left.join(&right, "k", "k", JoinHow::Inner).unwrap();
            assert_eq!(out.n_rows(), n as usize);
            t.elapsed().as_secs_f64() * 1000.0
        })
    };
    let one = time_with(1);
    let many = time_with(0); // 0 = rayon default (all cores)
    println!(
        "join {n} rows: 1 thread {one:.1} ms, all cores {many:.1} ms, speedup {:.2}x",
        one / many
    );
}

/// DP1b: a group_by above the parallel threshold (100k rows) must be
/// correct. Integer sum/count/min/max are bit-identical to sequential
/// (associative), and a float aggregation is deterministic run to run. This
/// exercises the parallel scatter under `--features parallel` and the
/// sequential path otherwise; the assertions hold either way.
#[test]
fn large_group_by_is_correct() {
    let n: i64 = 500_000;
    // 5 groups (key = row % 5), each with n/5 rows. Integer value is 1 and
    // float value is 1.0, so per-group sum = 100_000 exactly (small integers
    // and 1.0 sums are exact in f64 regardless of reduction order).
    let f = Frame::new(vec![
        (
            "k".to_string(),
            Series::from_i64((0..n).map(|i| i % 5).collect()),
        ),
        ("iv".to_string(), Series::from_i64(vec![1i64; n as usize])),
        ("fv".to_string(), Series::from_f64(vec![1.0f64; n as usize])),
    ])
    .unwrap();

    let spec = || {
        vec![
            agg("cnt", AggOp::Count, "iv"),
            agg("isum", AggOp::Sum, "iv"),
            agg("imin", AggOp::Min, "iv"),
            agg("imax", AggOp::Max, "iv"),
            agg("fsum", AggOp::Sum, "fv"),
            agg("fmean", AggOp::Mean, "fv"),
        ]
    };
    let g = f.group_by(&["k".to_string()], &spec()).unwrap();
    assert_eq!(g.n_rows(), 5);
    let per = 100_000i64;
    for row in 0..5 {
        assert_eq!(g.column("cnt").unwrap().scalar_at(row), Scalar::I64(per));
        assert_eq!(g.column("isum").unwrap().scalar_at(row), Scalar::I64(per));
        assert_eq!(g.column("imin").unwrap().scalar_at(row), Scalar::I64(1));
        assert_eq!(g.column("imax").unwrap().scalar_at(row), Scalar::I64(1));
        assert_eq!(
            g.column("fsum").unwrap().scalar_at(row),
            Scalar::F64(per as f64)
        );
        assert_eq!(g.column("fmean").unwrap().scalar_at(row), Scalar::F64(1.0));
    }

    // Run-to-run determinism: the same aggregation twice is identical.
    let g2 = f.group_by(&["k".to_string()], &spec()).unwrap();
    assert_eq!(col_values(&g, "fsum"), col_values(&g2, "fsum"));
}

/// Manual timing (not a gate): `cargo test -p olang-ods --features parallel
/// --test frame group_by_speedup -- --ignored --nocapture`.
#[test]
#[ignore]
#[cfg(feature = "parallel")]
fn group_by_speedup() {
    use std::time::Instant;
    let n: i64 = 8_000_000;
    let f = Frame::new(vec![
        (
            "k".to_string(),
            Series::from_i64((0..n).map(|i| i % 1000).collect()),
        ),
        (
            "v".to_string(),
            Series::from_f64((0..n).map(|i| (i % 97) as f64).collect()),
        ),
    ])
    .unwrap();
    let spec = [
        agg("s", AggOp::Sum, "v"),
        agg("m", AggOp::Mean, "v"),
        agg("n", AggOp::Count, "v"),
    ];
    let time_with = |threads: usize| -> f64 {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let t = Instant::now();
            let g = f.group_by(&["k".to_string()], &spec).unwrap();
            assert_eq!(g.n_rows(), 1000);
            t.elapsed().as_secs_f64() * 1000.0
        })
    };
    let one = time_with(1);
    let many = time_with(0);
    println!(
        "group_by {n} rows, 1000 groups: 1 thread {one:.1} ms, all cores {many:.1} ms, speedup {:.2}x",
        one / many
    );
}

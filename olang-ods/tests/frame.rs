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
    assert!(Frame::new(vec![
        ("a".to_string(), Series::from_i64(vec![1, 2])),
        ("a".to_string(), Series::from_i64(vec![3, 4])),
    ])
    .is_err());
    assert!(Frame::new(vec![
        ("a".to_string(), Series::from_i64(vec![1, 2])),
        ("b".to_string(), Series::from_i64(vec![3])),
    ])
    .is_err());
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

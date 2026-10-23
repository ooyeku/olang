//! The column kernels (`column.rs`, the faster `shift` and `argsort`,
//! `over`, multi-key sorting) against the obvious implementations.

use olang_ods::column::{self, Unmapped};
use olang_ods::frame::{over, sort_order};
use olang_ods::{AggOp, Scalar, Series};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn opts_f(s: &Series) -> Vec<Option<f64>> {
    (0..s.len())
        .map(|i| match s.scalar_at(i) {
            Scalar::F64(x) => Some(x),
            Scalar::I64(x) => Some(x as f64),
            _ => None,
        })
        .collect()
}

fn random_f(rng: &mut Rng, n: usize, nulls: u64, distinct: u64) -> Series {
    Series::from_f64_options(
        (0..n)
            .map(|_| {
                let x = rng.next();
                if x % 100 < nulls {
                    None
                } else {
                    Some(((x >> 8) % distinct) as f64 - (distinct / 2) as f64)
                }
            })
            .collect(),
    )
}

#[test]
fn shift_is_the_indexed_definition() {
    let mut rng = Rng(3);
    for n in [0usize, 1, 5, 63, 64, 65, 200, 1000] {
        let s = random_f(&mut rng, n, 20, 1000);
        let i = Series::from_i64((0..n as i64).collect());
        for by in [-70i64, -64, -3, -1, 0, 1, 2, 63, 64, 65, 2000] {
            let got = opts_f(&s.shift(by).unwrap());
            let want: Vec<Option<f64>> = (0..n as i64)
                .map(|k| {
                    let from = k - by;
                    if (0..n as i64).contains(&from) {
                        opts_f(&s)[from as usize]
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(got, want, "n={n} by={by}");
            let gi = opts_f(&i.shift(by).unwrap());
            assert_eq!(
                gi.iter().filter(|x| x.is_none()).count(),
                (by.unsigned_abs() as usize).min(n)
            );
        }
    }
}

#[test]
fn fills_look_along_the_column() {
    let s = Series::from_f64_options(vec![None, Some(1.0), None, None, None, Some(5.0), None]);
    assert_eq!(
        opts_f(&s.fill_direction(true, None)),
        vec![
            None,
            Some(1.0),
            Some(1.0),
            Some(1.0),
            Some(1.0),
            Some(5.0),
            Some(5.0)
        ]
    );
    assert_eq!(
        opts_f(&s.fill_direction(true, Some(2))),
        vec![
            None,
            Some(1.0),
            Some(1.0),
            Some(1.0),
            None,
            Some(5.0),
            Some(5.0)
        ]
    );
    assert_eq!(
        opts_f(&s.fill_direction(false, None)),
        vec![
            Some(1.0),
            Some(1.0),
            Some(5.0),
            Some(5.0),
            Some(5.0),
            Some(5.0),
            None
        ]
    );
    assert_eq!(
        opts_f(&s.interpolate(None).unwrap()),
        vec![
            None,
            Some(1.0),
            Some(2.0),
            Some(3.0),
            Some(4.0),
            Some(5.0),
            None
        ]
    );
    assert_eq!(
        opts_f(&s.interpolate(Some(1)).unwrap()),
        vec![None, Some(1.0), Some(2.0), None, None, Some(5.0), None]
    );
    let t = Series::from_str_options(vec![Some("a".into()), None, Some("c".into())]);
    assert_eq!(
        t.fill_direction(true, None).scalar_at(1),
        Scalar::Str("a".into())
    );
    // The mean of an Int column fills as a Float unless it is whole.
    let m = Series::from_i64_options(vec![Some(1), None, Some(4)]);
    assert_eq!(m.fill_stat(false).unwrap().scalar_at(1), Scalar::F64(2.5));
    let m = Series::from_i64_options(vec![Some(1), None, Some(3)]);
    assert_eq!(m.fill_stat(false).unwrap().scalar_at(1), Scalar::I64(2));
    assert_eq!(m.fill_stat(true).unwrap().scalar_at(1), Scalar::I64(2));
}

#[test]
fn differences_and_products() {
    let s = Series::from_i64_options(vec![Some(1), Some(4), None, Some(9)]);
    assert_eq!(
        opts_f(&s.diff(1).unwrap()),
        vec![None, Some(3.0), None, None]
    );
    assert_eq!(
        opts_f(&s.diff(-1).unwrap()),
        vec![Some(-3.0), None, None, None]
    );
    assert_eq!(s.diff(1).unwrap().dtype(), olang_ods::DType::I64);
    let f = Series::from_f64(vec![2.0, 3.0, 6.0]);
    assert_eq!(
        opts_f(&f.pct_change(1).unwrap()),
        vec![None, Some(0.5), Some(1.0)]
    );
    assert_eq!(
        opts_f(&s.cumprod().unwrap()),
        vec![Some(1.0), Some(4.0), None, Some(36.0)]
    );
    let big = Series::from_i64(vec![i64::MAX, 2]);
    assert!(big.cumprod().is_err());
}

#[test]
fn positions_modes_and_searches() {
    let s = Series::from_f64_options(vec![
        Some(3.0),
        None,
        Some(f64::NAN),
        Some(7.0),
        Some(7.0),
        Some(-1.0),
    ]);
    assert_eq!(s.arg_extreme(true).unwrap(), Some(3));
    assert_eq!(s.arg_extreme(false).unwrap(), Some(5));
    assert_eq!(
        Series::from_f64_options(vec![None])
            .arg_extreme(true)
            .unwrap(),
        None
    );
    assert_eq!(
        Series::from_i64(vec![3, 1, 1, 3, 2]).mode().unwrap(),
        Scalar::I64(1)
    );
    assert_eq!(
        Series::from_str_values(vec!["b".into(), "a".into(), "b".into()])
            .mode()
            .unwrap(),
        Scalar::Str("b".into())
    );
    assert_eq!(
        Series::from_f64_options(vec![None]).mode().unwrap(),
        Scalar::Null
    );
    let sorted = Series::from_i64(vec![1, 3, 3, 5]);
    assert_eq!(sorted.searchsorted(&Scalar::I64(3), false).unwrap(), 1);
    assert_eq!(sorted.searchsorted(&Scalar::I64(3), true).unwrap(), 3);
    assert_eq!(sorted.searchsorted(&Scalar::F64(4.5), false).unwrap(), 3);
    assert_eq!(sorted.searchsorted(&Scalar::I64(9), false).unwrap(), 4);
    let mask = Series::from_bool_options(vec![Some(false), None, Some(true)]);
    assert!(mask.any_all(false).unwrap());
    assert!(!mask.any_all(true).unwrap());
}

#[test]
fn constructors() {
    assert_eq!(
        opts_f(&column::arange_i64(0, 5, 2).unwrap()),
        vec![Some(0.0), Some(2.0), Some(4.0)]
    );
    assert_eq!(
        opts_f(&column::arange_i64(5, 0, -2).unwrap()),
        vec![Some(5.0), Some(3.0), Some(1.0)]
    );
    assert_eq!(column::arange_f64(0.0, 1.0, 0.25).unwrap().len(), 4);
    assert!(column::arange_i64(0, 5, 0).is_err());
    let s = Series::from_i64_options(vec![Some(1), None]);
    assert_eq!(opts_f(&s.repeat(2)), vec![Some(1.0), Some(1.0), None, None]);
    assert_eq!(opts_f(&s.tile(2)), vec![Some(1.0), None, Some(1.0), None]);
    assert_eq!(
        opts_f(&s.repeat_each(&[0, 3]).unwrap()),
        vec![None, None, None]
    );
    assert_eq!(
        column::full(3, &Scalar::Str("x".into())).scalar_at(2),
        Scalar::Str("x".into())
    );
}

#[test]
fn recode_maps_each_distinct_value_once() {
    let s = Series::from_str_options(vec![
        Some("a".into()),
        Some("b".into()),
        None,
        Some("z".into()),
    ]);
    let from = Series::from_str_values(vec!["a".into(), "b".into()]);
    let to = Series::from_str_values(vec!["A".into(), "B".into()]);
    let keep = s.recode(&from, &to, &Unmapped::Keep).unwrap();
    assert_eq!(keep.scalar_at(0), Scalar::Str("A".into()));
    assert_eq!(keep.scalar_at(2), Scalar::Null);
    assert_eq!(keep.scalar_at(3), Scalar::Str("z".into()));
    let null = s.recode(&from, &to, &Unmapped::Null).unwrap();
    assert_eq!(null.scalar_at(3), Scalar::Null);
    let other = s
        .recode(&from, &to, &Unmapped::Value(Scalar::Str("?".into())))
        .unwrap();
    assert_eq!(other.scalar_at(3), Scalar::Str("?".into()));
    assert_eq!(other.scalar_at(2), Scalar::Null);
    // A large map takes the hashed path; numbers match across kinds.
    let n = 100;
    let ints = Series::from_i64((0..1000).map(|i| i % 150).collect());
    let from = Series::from_f64((0..n).map(|i| i as f64).collect());
    let to = Series::from_i64((0..n).map(|i| i * 10).collect());
    let out = ints.recode(&from, &to, &Unmapped::Keep).unwrap();
    for i in 0..1000 {
        let v = (i % 150) as i64;
        let want = if v < n { v * 10 } else { v };
        assert_eq!(out.scalar_at(i), Scalar::I64(want));
    }
}

#[test]
fn packed_argsort_is_the_stable_sort() {
    let mut rng = Rng(11);
    for n in [0usize, 1, 10, 1000, 200_001] {
        let s = random_f(&mut rng, n, 10, 50);
        let vals = opts_f(&s);
        for desc in [false, true] {
            let got: Vec<i64> = match s.argsort_by(desc).unwrap() {
                Series::I64 { values, .. } => values.as_ref().clone(),
                _ => unreachable!(),
            };
            let mut want: Vec<usize> = (0..n).filter(|&i| vals[i].is_some()).collect();
            want.sort_by(|&a, &b| {
                let o = vals[a].unwrap().total_cmp(&vals[b].unwrap());
                if desc { o.reverse() } else { o }
            });
            want.extend((0..n).filter(|&i| vals[i].is_none()));
            assert_eq!(
                got,
                want.iter().map(|&i| i as i64).collect::<Vec<_>>(),
                "n={n} desc={desc}"
            );
        }
    }
}

#[test]
fn multi_key_order_is_lexicographic_and_stable() {
    let mut rng = Rng(5);
    for n in [1usize, 50, 5000, 150_000] {
        let a = random_f(&mut rng, n, 5, 7);
        let b = Series::from_str_options(
            (0..n)
                .map(|_| {
                    let x = rng.next();
                    (!x.is_multiple_of(20)).then(|| format!("k{}", x % 13))
                })
                .collect(),
        );
        let c = Series::from_bool((0..n).map(|_| rng.next().is_multiple_of(2)).collect());
        for dirs in [
            [false, false, false],
            [true, false, true],
            [false, true, false],
        ] {
            let got = sort_order(&[(&a, dirs[0]), (&b, dirs[1]), (&c, dirs[2])]).unwrap();
            let key = |s: &Series, i: usize| s.scalar_at(i);
            let cmp1 = |s: &Series, d: bool, x: usize, y: usize| {
                let (p, q) = (key(s, x), key(s, y));
                use std::cmp::Ordering::*;
                match (&p, &q) {
                    (Scalar::Null, Scalar::Null) => Equal,
                    (Scalar::Null, _) => Greater,
                    (_, Scalar::Null) => Less,
                    _ => {
                        let o = match (&p, &q) {
                            (Scalar::F64(u), Scalar::F64(v)) => u.total_cmp(v),
                            (Scalar::Str(u), Scalar::Str(v)) => u.cmp(v),
                            (Scalar::Bool(u), Scalar::Bool(v)) => u.cmp(v),
                            _ => unreachable!(),
                        };
                        if d { o.reverse() } else { o }
                    }
                }
            };
            let mut want: Vec<usize> = (0..n).collect();
            want.sort_by(|&x, &y| {
                cmp1(&a, dirs[0], x, y)
                    .then(cmp1(&b, dirs[1], x, y))
                    .then(cmp1(&c, dirs[2], x, y))
            });
            assert_eq!(got, want, "n={n} dirs={dirs:?}");
        }
    }
}

#[test]
fn over_broadcasts_each_groups_aggregate() {
    let x = Series::from_f64_options(vec![Some(1.0), Some(10.0), None, Some(3.0), Some(30.0)]);
    let g = Series::from_str_values(vec![
        "a".into(),
        "b".into(),
        "a".into(),
        "a".into(),
        "b".into(),
    ]);
    let mean = over(&x, &[&g], AggOp::Mean).unwrap();
    assert_eq!(
        opts_f(&mean),
        vec![Some(2.0), Some(20.0), Some(2.0), Some(2.0), Some(20.0)]
    );
    let count = over(&x, &[&g], AggOp::Count).unwrap();
    assert_eq!(
        opts_f(&count),
        vec![Some(2.0), Some(2.0), Some(2.0), Some(2.0), Some(2.0)]
    );
    let first = over(&x, &[&g], AggOp::First).unwrap();
    assert_eq!(
        opts_f(&first),
        vec![Some(1.0), Some(10.0), Some(1.0), Some(1.0), Some(10.0)]
    );
    let med = over(&x, &[&g], AggOp::Median).unwrap();
    assert_eq!(opts_f(&med)[0], Some(2.0));
}

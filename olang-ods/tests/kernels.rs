//! Every kernel pinned against a naive reference implementation over
//! generated data, including null patterns — the property-test discipline
//! the design doc requires (docs/design/ods.md, Phase 1 gate).

use olang_ods::{ArithOp, CmpOp, DType, OdsError, Scalar, Series};

/// Deterministic xorshift so failures reproduce; no rand dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn f64(&mut self) -> f64 {
        // Roughly [-100, 100), coarse enough to hit equal values sometimes.
        ((self.next() % 20_000) as f64 - 10_000.0) / 100.0
    }

    fn i64(&mut self) -> i64 {
        (self.next() % 2_001) as i64 - 1_000
    }

    fn maybe<T>(&mut self, v: T, null_rate_pct: u64) -> Option<T> {
        if self.next() % 100 < null_rate_pct {
            None
        } else {
            Some(v)
        }
    }
}

fn gen_f64(rng: &mut Rng, n: usize, null_pct: u64) -> Vec<Option<f64>> {
    (0..n)
        .map(|_| {
            let v = rng.f64();
            rng.maybe(v, null_pct)
        })
        .collect()
}

fn gen_i64(rng: &mut Rng, n: usize, null_pct: u64) -> Vec<Option<i64>> {
    (0..n)
        .map(|_| {
            let v = rng.i64();
            rng.maybe(v, null_pct)
        })
        .collect()
}

fn to_options(s: &Series) -> Vec<Scalar> {
    (0..s.len()).map(|i| s.scalar_at(i)).collect()
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

#[test]
fn f64_arith_matches_reference_with_nulls() {
    let mut rng = Rng(7);
    for &par in &[false, true] {
        let a = gen_f64(&mut rng, 257, 10);
        let b = gen_f64(&mut rng, 257, 10);
        let sa = Series::from_f64_options(a.clone());
        let sb = Series::from_f64_options(b.clone());
        for op in [ArithOp::Add, ArithOp::Sub, ArithOp::Mul] {
            let out = sa.arith(op, &sb, par).unwrap();
            for i in 0..257 {
                let expect = match (a[i], b[i]) {
                    (Some(x), Some(y)) => Scalar::F64(match op {
                        ArithOp::Add => x + y,
                        ArithOp::Sub => x - y,
                        ArithOp::Mul => x * y,
                        ArithOp::Div => unreachable!(),
                    }),
                    _ => Scalar::Null,
                };
                match (out.scalar_at(i), expect) {
                    (Scalar::F64(got), Scalar::F64(want)) => assert!(approx(got, want)),
                    (got, want) => assert_eq!(got, want, "position {}", i),
                }
            }
        }
    }
}

#[test]
fn division_by_zero_only_errors_at_valid_positions() {
    // Zero divisor under a null: never computed, so no error.
    let a = Series::from_f64_options(vec![Some(1.0), Some(2.0)]);
    let b = Series::from_f64_options(vec![Some(2.0), None]);
    // b's second value is stored as default 0.0 but is null — must not error.
    let out = a.arith(ArithOp::Div, &b, false).unwrap();
    assert_eq!(out.scalar_at(0), Scalar::F64(0.5));
    assert_eq!(out.scalar_at(1), Scalar::Null);

    let c = Series::from_f64(vec![2.0, 0.0]);
    assert_eq!(
        a.arith(ArithOp::Div, &c, false).unwrap_err(),
        OdsError::DivisionByZero
    );
}

#[test]
fn i64_arith_is_checked_and_null_aware() {
    let a = Series::from_i64_options(vec![Some(5), None, Some(7)]);
    let b = Series::from_i64_options(vec![Some(2), Some(3), None]);
    let out = a.arith(ArithOp::Mul, &b, false).unwrap();
    assert_eq!(
        to_options(&out),
        vec![Scalar::I64(10), Scalar::Null, Scalar::Null]
    );

    let big = Series::from_i64(vec![i64::MAX]);
    let one = Series::from_i64(vec![1]);
    assert_eq!(
        big.arith(ArithOp::Add, &one, false).unwrap_err(),
        OdsError::IntegerOverflow("addition")
    );
    // Int division mirrors the language: truncating, zero errors.
    let d = Series::from_i64(vec![7]).arith(ArithOp::Div, &Series::from_i64(vec![2]), false);
    assert_eq!(to_options(&d.unwrap()), vec![Scalar::I64(3)]);
}

#[test]
fn mixed_int_float_widens() {
    let a = Series::from_i64(vec![1, 2, 3]);
    let b = Series::from_f64(vec![0.5, 0.5, 0.5]);
    let out = a.arith(ArithOp::Add, &b, false).unwrap();
    assert_eq!(out.dtype(), olang_ods::DType::F64);
    assert_eq!(out.scalar_at(2), Scalar::F64(3.5));
}

#[test]
fn scalar_broadcast_including_swapped() {
    let s = Series::from_f64(vec![1.0, 2.0, 4.0]);
    let doubled = s
        .arith_scalar(ArithOp::Mul, Scalar::F64(2.0), false, false)
        .unwrap();
    assert_eq!(
        to_options(&doubled),
        vec![Scalar::F64(2.0), Scalar::F64(4.0), Scalar::F64(8.0)]
    );
    // 10 - s (swapped subtraction)
    let inv = s
        .arith_scalar(ArithOp::Sub, Scalar::I64(10), true, false)
        .unwrap();
    assert_eq!(inv.scalar_at(2), Scalar::F64(6.0));
    // 8 / s vs s / 0
    assert!(
        s.arith_scalar(ArithOp::Div, Scalar::F64(0.0), false, false)
            .is_err()
    );
    let swapped_zero = Series::from_f64(vec![1.0, 0.0]);
    assert!(
        swapped_zero
            .arith_scalar(ArithOp::Div, Scalar::F64(8.0), true, false)
            .is_err()
    );
}

#[test]
fn reductions_skip_nulls_and_match_reference() {
    let mut rng = Rng(99);
    let opts = gen_f64(&mut rng, 1_003, 15);
    let s = Series::from_f64_options(opts.clone());
    let valid: Vec<f64> = opts.iter().flatten().copied().collect();
    let n = valid.len() as f64;

    let sum_ref: f64 = valid.iter().sum();
    let mean_ref = sum_ref / n;
    let var_ref: f64 = valid
        .iter()
        .map(|x| (x - mean_ref) * (x - mean_ref))
        .sum::<f64>()
        / (n - 1.0);

    for &par in &[false, true] {
        match s.sum(par).unwrap() {
            Scalar::F64(got) => assert!(approx(got, sum_ref)),
            other => panic!("sum returned {:?}", other),
        }
        assert!(approx(s.mean(par).unwrap().unwrap(), mean_ref));
        assert!(approx(s.var(par).unwrap().unwrap(), var_ref));
        assert!(approx(s.std(par).unwrap().unwrap(), var_ref.sqrt()));
    }

    let min_ref = valid.iter().copied().fold(f64::INFINITY, f64::min);
    let max_ref = valid.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert_eq!(s.min().unwrap(), Scalar::F64(min_ref));
    assert_eq!(s.max().unwrap(), Scalar::F64(max_ref));
}

#[test]
fn empty_and_all_null_reductions() {
    let empty = Series::from_f64(Vec::new());
    assert_eq!(empty.sum(false).unwrap(), Scalar::F64(0.0));
    assert_eq!(empty.mean(false).unwrap(), None);
    let nulls = Series::from_f64_options(vec![None, None]);
    assert_eq!(nulls.sum(false).unwrap(), Scalar::F64(0.0));
    assert_eq!(nulls.mean(false).unwrap(), None);
    assert_eq!(nulls.min().unwrap(), Scalar::Null);
    // var needs two valid values
    let one = Series::from_f64_options(vec![Some(3.0), None]);
    assert_eq!(one.var(false).unwrap(), None);
}

#[test]
fn quantile_interpolates_like_numpy() {
    let s = Series::from_f64(vec![4.0, 1.0, 3.0, 2.0]);
    assert_eq!(s.quantile(0.0).unwrap(), Some(1.0));
    assert_eq!(s.quantile(1.0).unwrap(), Some(4.0));
    assert_eq!(s.quantile(0.5).unwrap(), Some(2.5));
    assert_eq!(s.quantile(0.25).unwrap(), Some(1.75));
    assert!(s.quantile(1.5).is_err());
    let with_nulls = Series::from_f64_options(vec![Some(1.0), None, Some(3.0)]);
    assert_eq!(with_nulls.quantile(0.5).unwrap(), Some(2.0));
}

#[test]
fn sort_and_argsort_put_nulls_last() {
    let s = Series::from_f64_options(vec![Some(3.0), None, Some(1.0), Some(2.0)]);
    for &par in &[false, true] {
        let sorted = s.sort(par).unwrap();
        assert_eq!(
            to_options(&sorted),
            vec![
                Scalar::F64(1.0),
                Scalar::F64(2.0),
                Scalar::F64(3.0),
                Scalar::Null
            ]
        );
    }
    let idx = s.argsort().unwrap();
    assert_eq!(
        to_options(&idx),
        vec![
            Scalar::I64(2),
            Scalar::I64(3),
            Scalar::I64(0),
            Scalar::I64(1)
        ]
    );
    // take(argsort) == sort — the pair of kernels must agree
    let gathered = s.take(&idx).unwrap();
    assert!(gathered.series_eq(&s.sort(false).unwrap()));
}

#[test]
fn take_supports_negative_indices_and_bounds_checks() {
    let s = Series::from_i64(vec![10, 20, 30]);
    let taken = s.take(&Series::from_i64(vec![-1, 0])).unwrap();
    assert_eq!(to_options(&taken), vec![Scalar::I64(30), Scalar::I64(10)]);
    assert_eq!(
        s.take(&Series::from_i64(vec![3])).unwrap_err(),
        OdsError::IndexOutOfBounds {
            index: 3,
            length: 3
        }
    );
}

#[test]
fn filter_drops_false_and_null_mask_positions() {
    let s = Series::from_i64(vec![1, 2, 3, 4]);
    let mask = Series::from_bool_options(vec![Some(true), Some(false), None, Some(true)]);
    let kept = s.filter(&mask).unwrap();
    assert_eq!(to_options(&kept), vec![Scalar::I64(1), Scalar::I64(4)]);
}

#[test]
fn cumsum_carries_across_nulls() {
    let s = Series::from_i64_options(vec![Some(1), None, Some(2), Some(3)]);
    let c = s.cumsum().unwrap();
    assert_eq!(
        to_options(&c),
        vec![Scalar::I64(1), Scalar::Null, Scalar::I64(3), Scalar::I64(6)]
    );
}

#[test]
fn dot_skips_null_pairs() {
    let a = Series::from_f64_options(vec![Some(1.0), Some(2.0), None]);
    let b = Series::from_f64_options(vec![Some(3.0), None, Some(5.0)]);
    assert_eq!(a.dot(&b, false).unwrap(), 3.0);
    let dense_a = Series::from_f64(vec![1.0, 2.0]);
    let dense_b = Series::from_f64(vec![3.0, 4.0]);
    for &par in &[false, true] {
        assert_eq!(dense_a.dot(&dense_b, par).unwrap(), 11.0);
    }
}

#[test]
fn comparisons_produce_null_aware_masks() {
    let a = Series::from_f64_options(vec![Some(1.0), None, Some(3.0)]);
    let b = Series::from_f64(vec![2.0, 2.0, 2.0]);
    let m = a.compare(CmpOp::Gt, &b).unwrap();
    assert_eq!(
        to_options(&m),
        vec![Scalar::Bool(false), Scalar::Null, Scalar::Bool(true)]
    );
    let ms = a.compare_scalar(CmpOp::Le, Scalar::I64(1), false).unwrap();
    assert_eq!(ms.scalar_at(0), Scalar::Bool(true));
    // swapped: 1 <= a  ≡  a >= 1
    let sw = a.compare_scalar(CmpOp::Le, Scalar::I64(1), true).unwrap();
    assert_eq!(sw.scalar_at(2), Scalar::Bool(true));
}

#[test]
fn fill_null_is_copy_on_write() {
    let s = Series::from_f64_options(vec![Some(1.0), None, Some(3.0)]);
    let filled = s.clone().fill_null(Scalar::F64(0.0)).unwrap();
    assert_eq!(
        to_options(&filled),
        vec![Scalar::F64(1.0), Scalar::F64(0.0), Scalar::F64(3.0)]
    );
    assert_eq!(filled.null_count(), 0);
    // Original untouched (the clone shared, so fill_null had to copy).
    assert_eq!(s.scalar_at(1), Scalar::Null);
    // Dtype mismatch refuses.
    assert!(s.fill_null(Scalar::Bool(true)).is_err());
}

#[test]
fn is_null_and_null_count_agree() {
    let mut rng = Rng(5);
    let opts = gen_i64(&mut rng, 300, 20);
    let s = Series::from_i64_options(opts.clone());
    let mask = s.is_null();
    let count = (0..mask.len())
        .filter(|&i| mask.scalar_at(i) == Scalar::Bool(true))
        .count();
    assert_eq!(count, s.null_count());
    assert_eq!(count, opts.iter().filter(|o| o.is_none()).count());
}

#[test]
fn range_and_linspace_constructors() {
    let r = Series::from_range(1, 4, false);
    assert_eq!(
        to_options(&r),
        vec![Scalar::I64(1), Scalar::I64(2), Scalar::I64(3)]
    );
    let ri = Series::from_range(1, 3, true);
    assert_eq!(ri.len(), 3);
    let l = Series::linspace(0.0, 1.0, 5).unwrap();
    assert_eq!(l.scalar_at(2), Scalar::F64(0.5));
    assert_eq!(l.scalar_at(4), Scalar::F64(1.0));
    assert_eq!(Series::zeros(3).sum(false).unwrap(), Scalar::F64(0.0));
}

#[test]
fn series_eq_ignores_values_under_nulls() {
    let a = Series::from_i64_options(vec![Some(1), None]);
    let b = Series::from_i64_options(vec![Some(1), None]);
    assert!(a.series_eq(&b));
    let c = Series::from_i64(vec![1, 0]);
    assert!(!a.series_eq(&c));
    assert!(!a.series_eq(&Series::from_f64(vec![1.0, 0.0])));
}

#[test]
fn map_unary_matches_std_and_propagates_nulls() {
    // Every supported function equals the naive scalar loop over std f64
    // methods — bit-identical, since map_unary calls the same methods.
    let src = [0.1, 0.3, 0.9, 1.0, 2.5, 7.0];
    let s = Series::from_f64(src.to_vec());
    type UnaryFn = fn(f64) -> f64;
    let cases: &[(&str, UnaryFn)] = &[
        ("sin", f64::sin),
        ("cos", f64::cos),
        ("tan", f64::tan),
        ("atan", f64::atan),
        ("tanh", f64::tanh),
        ("exp", f64::exp),
        ("exp2", f64::exp2),
        ("sqrt", f64::sqrt),
        ("cbrt", f64::cbrt),
        ("ln", f64::ln),
        ("log2", f64::log2),
        ("log10", f64::log10),
        ("floor", f64::floor),
        ("ceil", f64::ceil),
        ("round", f64::round),
        ("trunc", f64::trunc),
        ("fract", f64::fract),
        ("abs", f64::abs),
        ("degrees", f64::to_degrees),
        ("radians", f64::to_radians),
    ];
    for (name, f) in cases {
        let got = s.map_unary(name).unwrap();
        for (i, &x) in src.iter().enumerate() {
            assert_eq!(got.scalar_at(i), Scalar::F64(f(x)), "{} at {}", name, i);
        }
    }

    // Integer input converts to f64.
    let is = Series::from_i64(vec![1, 4, 9]);
    let r = is.map_unary("sqrt").unwrap();
    assert_eq!(r.scalar_at(1), Scalar::F64(2.0));

    // Nulls stay null; the underlying value is not evaluated.
    let n = Series::from_f64_options(vec![Some(4.0), None, Some(9.0)]);
    let rn = n.map_unary("sqrt").unwrap();
    assert_eq!(rn.scalar_at(0), Scalar::F64(2.0));
    assert_eq!(rn.scalar_at(1), Scalar::Null);
    assert_eq!(rn.scalar_at(2), Scalar::F64(3.0));
}

#[test]
fn map_unary_domain_and_type_errors() {
    // Domain errors match math.* exactly (they error, not NaN).
    let neg = Series::from_f64(vec![1.0, -4.0]);
    assert!(neg.map_unary("sqrt").is_err());
    assert!(Series::from_f64(vec![0.0]).map_unary("ln").is_err());
    assert!(Series::from_f64(vec![2.0]).map_unary("asin").is_err());
    // A null at the bad position means no error — it is never evaluated.
    let masked = Series::from_f64_options(vec![Some(1.0), None]);
    let mut bad = masked;
    if let Series::F64 { values, .. } = &mut bad {
        std::sync::Arc::make_mut(values)[1] = -9.0;
    }
    assert!(bad.map_unary("sqrt").is_ok());
    // Unknown function and non-numeric series both refuse.
    assert!(Series::from_f64(vec![1.0]).map_unary("wat").is_err());
    assert!(
        Series::from_str_values(vec!["a".into()])
            .map_unary("sin")
            .is_err()
    );
}

// ── cast ──────────────────────────────────────────────────────────────

#[test]
fn cast_to_int_yields_null_where_the_value_is_not_representable() {
    // Rust's `as` would saturate 1e30 to i64::MAX and turn NaN into 0 —
    // two wrong answers that look exactly like data. Neither can be
    // constructed from olang source (the language refuses `0.0 / 0.0`
    // and `math.sqrt(-1.0)`), but a Float column can carry them in from
    // a file, so the guard is tested where a NaN can be written down.
    let s = Series::from_f64(vec![2.7, -2.7, f64::NAN, 1e30, -1e30, f64::INFINITY]);
    let out = s.cast(DType::I64, &|x| x.to_string()).expect("cast");
    let got: Vec<Scalar> = (0..out.len()).map(|i| out.scalar_at(i)).collect();
    assert_eq!(
        got,
        vec![
            Scalar::I64(2),
            Scalar::I64(-2),
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
        ]
    );
    assert_eq!(out.null_count(), 4);
}

#[test]
fn cast_keeps_nulls_null_in_every_direction() {
    let s = Series::from_i64_options(vec![Some(1), None, Some(0)]);
    for to in [DType::F64, DType::Bool, DType::Str] {
        let out = s.cast(to, &|x| x.to_string()).expect("cast");
        assert_eq!(out.null_count(), 1, "null lost casting to {}", to);
        assert_eq!(out.scalar_at(1), Scalar::Null, "wrong position for {}", to);
    }
}

#[test]
fn cast_uses_the_formatter_it_is_given_for_floats() {
    // The engine holds no opinion on how a float is spelled; olang hands
    // in its own so a cast column matches `to_string`.
    let s = Series::from_f64(vec![320.0]);
    let plain = s.cast(DType::Str, &|x| x.to_string()).expect("cast");
    let olangish = s
        .cast(DType::Str, &|x| {
            let t = x.to_string();
            if t.contains('.') {
                t
            } else {
                format!("{}.0", t)
            }
        })
        .expect("cast");
    assert_eq!(plain.scalar_at(0), Scalar::Str("320".to_string()));
    assert_eq!(olangish.scalar_at(0), Scalar::Str("320.0".to_string()));
}

// ── unique / value_counts ─────────────────────────────────────────────

#[test]
fn value_counts_and_unique_see_all_nans_as_one_value() {
    // Keys are float bit patterns with NaN canonicalized, matching
    // group_by. This differs from `==` on the scalars, where no NaN
    // equals any other — stated in the docs, pinned here.
    let s = Series::from_f64(vec![f64::NAN, 1.0, f64::NAN, 1.0, 2.0]);
    assert_eq!(s.n_unique(), 3);
    let (values, counts) = s.value_counts().expect("value_counts");
    assert_eq!(values.len(), 3);
    // NaN and 1.0 both occur twice; NaN appeared first, so it leads.
    let got: Vec<Scalar> = (0..counts.len()).map(|i| counts.scalar_at(i)).collect();
    assert_eq!(got, vec![Scalar::I64(2), Scalar::I64(2), Scalar::I64(1)]);
    assert!(matches!(values.scalar_at(0), Scalar::F64(x) if x.is_nan()));
}

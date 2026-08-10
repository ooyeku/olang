//! Phase 2 gate: inference results pinned against scipy reference values
//! (docs/design/ods.md). Every constant below was produced by scipy
//! 1.x / NumPy 2.5.2 — the generating snippets are noted inline so the
//! numbers can be re-derived.

#![cfg(feature = "stats")]

use olang_ods::stats::{chi2_gof, corr, cov, ols, t_test_one_sample, t_test_welch};
use olang_ods::{Series, dist};

fn close(got: f64, want: f64, tol: f64) {
    assert!(
        (got - want).abs() <= tol * (1.0 + want.abs()),
        "got {}, want {}",
        got,
        want
    );
}

const TIGHT: f64 = 1e-9;

#[test]
fn distributions_match_scipy() {
    // scipy.stats.norm/t/chi2/f — see docs/design/ods.md Phase 2 gate.
    close(
        dist::norm_pdf(1.5, 0.0, 1.0).unwrap(),
        0.12951759566589174,
        TIGHT,
    );
    close(
        dist::norm_cdf(1.96, 0.0, 1.0).unwrap(),
        0.9750021048517795,
        TIGHT,
    );
    close(
        dist::norm_ppf(0.975, 0.0, 1.0).unwrap(),
        1.959963984540054,
        1e-8,
    );
    close(
        dist::norm_cdf(120.0, 100.0, 15.0).unwrap(),
        0.9087887802741321,
        TIGHT,
    );

    close(dist::t_pdf(2.0, 7.0).unwrap(), 0.06313533730266196, TIGHT);
    close(dist::t_cdf(2.0, 7.0).unwrap(), 0.957190335718512, TIGHT);
    close(dist::t_ppf(0.975, 7.0).unwrap(), 2.364624251592784, 1e-8);

    close(dist::chi2_cdf(3.5, 2.0).unwrap(), 0.8262260565495549, TIGHT);
    close(dist::chi2_ppf(0.95, 4.0).unwrap(), 9.487729036781154, 1e-8);

    close(
        dist::f_cdf(2.5, 3.0, 10.0).unwrap(),
        0.8809604373417219,
        TIGHT,
    );
    close(
        dist::f_ppf(0.95, 3.0, 10.0).unwrap(),
        3.7082648190468435,
        1e-8,
    );

    assert!(dist::norm_pdf(0.0, 0.0, -1.0).is_err());
    assert!(dist::t_ppf(1.5, 7.0).is_err());
}

fn sample_a() -> Series {
    Series::from_f64(vec![5.1, 4.9, 6.2, 5.7, 5.5, 4.8, 5.9, 6.1])
}

fn sample_b() -> Series {
    Series::from_f64(vec![4.2, 4.8, 4.5, 5.0, 4.4, 4.1, 4.9])
}

#[test]
fn one_sample_t_matches_scipy() {
    // scipy.stats.ttest_1samp(a, 5.0)
    let r = t_test_one_sample(&sample_a(), 5.0).unwrap();
    close(r.t, 2.7406110459365625, TIGHT);
    close(r.p_value, 0.02889268808679156, 1e-8);
    close(r.df, 7.0, TIGHT);
}

#[test]
fn welch_t_matches_scipy() {
    // scipy.stats.ttest_ind(a, b, equal_var=False)
    let r = t_test_welch(&sample_a(), &sample_b()).unwrap();
    close(r.t, 4.15548385691924, TIGHT);
    close(r.p_value, 0.0013163190997012772, 1e-8);
    close(r.df, 12.074694444561887, 1e-8);
    close(r.mean, 5.525, TIGHT);
}

#[test]
fn t_test_skips_nulls() {
    // Nulls drop out: same numbers as sample_a with nulls interleaved.
    let vals: Vec<Option<f64>> = vec![
        Some(5.1),
        None,
        Some(4.9),
        Some(6.2),
        Some(5.7),
        None,
        Some(5.5),
        Some(4.8),
        Some(5.9),
        Some(6.1),
    ];
    let r = t_test_one_sample(&Series::from_f64_options(vals), 5.0).unwrap();
    close(r.t, 2.7406110459365625, TIGHT);
}

#[test]
fn chi2_gof_matches_scipy() {
    // scipy.stats.chisquare([18, 22, 27, 33], [25, 25, 25, 25])
    let obs = Series::from_f64(vec![18.0, 22.0, 27.0, 33.0]);
    let exp = Series::from_f64(vec![25.0, 25.0, 25.0, 25.0]);
    let r = chi2_gof(&obs, &exp).unwrap();
    close(r.statistic, 5.04, TIGHT);
    close(r.df, 3.0, TIGHT);
    close(r.p_value, 0.16889147065779933, 1e-8);
}

#[test]
fn corr_cov_match_scipy() {
    // scipy.stats.pearsonr / np.cov(ddof=1)
    let x = Series::from_f64(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y = Series::from_f64(vec![2.1, 3.9, 6.2, 8.1, 9.8, 12.3]);
    close(corr(&x, &y).unwrap().unwrap(), 0.9988210605417898, TIGHT);
    close(cov(&x, &y).unwrap().unwrap(), 7.060000000000001, TIGHT);
    // Pairwise-complete: a null in either drops the pair from both.
    let xn = Series::from_f64_options(vec![Some(1.0), None, Some(3.0), Some(4.0)]);
    let yn = Series::from_f64_options(vec![Some(2.0), Some(9.0), Some(6.0), None]);
    // Complete pairs: (1,2), (3,6) — perfectly correlated.
    close(corr(&xn, &yn).unwrap().unwrap(), 1.0, TIGHT);
    // Degenerate cases are None, not garbage.
    let flat = Series::from_f64(vec![2.0, 2.0, 2.0]);
    assert_eq!(
        corr(&flat, &Series::from_f64(vec![1.0, 2.0, 3.0])).unwrap(),
        None
    );
}

#[test]
fn ols_matches_numpy_lstsq() {
    // np.linalg.lstsq on y ~ 1 + x1 + x2, with SE/t/p via (X'X)^-1 and
    // scipy's t distribution — constants from the generator snippet.
    let x1 = Series::from_f64(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let x2 = Series::from_f64(vec![2.0, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]);
    let y = Series::from_f64(vec![3.1, 4.2, 7.3, 7.9, 11.2, 11.8, 15.3, 15.9]);
    for &par in &[false, true] {
        let fit = ols(&y, &[&x1, &x2], par).unwrap();
        let want_coef = [0.6437500000000069, 1.3562499999999995, 0.631249999999999];
        let want_se = [
            0.11851292123646251,
            0.05403991580304316,
            0.05403991580304315,
        ];
        let want_t = [5.431897157572944, 25.09718936171297, 11.681180301995427];
        let want_p = [
            0.002867462592200947,
            1.874213966504236e-06,
            8.079034177521314e-05,
        ];
        for j in 0..3 {
            close(fit.coef[j], want_coef[j], 1e-8);
            close(fit.se[j], want_se[j], 1e-8);
            close(fit.t[j], want_t[j], 1e-7);
            close(fit.p_value[j], want_p[j], 1e-6);
        }
        close(fit.r2, 0.999301056268897, TIGHT);
        close(fit.adj_r2, 0.9990214787764558, TIGHT);
        assert_eq!(fit.n, 8);
        close(fit.df_resid, 5.0, TIGHT);
    }
}

#[test]
fn ols_drops_null_rows_and_reports_n() {
    let x = Series::from_f64_options(vec![
        Some(1.0),
        Some(2.0),
        None,
        Some(4.0),
        Some(5.0),
        Some(6.0),
    ]);
    let y = Series::from_f64_options(vec![
        Some(2.0),
        Some(4.0),
        Some(9.0),
        None,
        Some(10.0),
        Some(12.0),
    ]);
    // Complete rows: (1,2), (2,4), (5,10), (6,12) — exact fit y = 2x.
    let fit = ols(&y, &[&x], false).unwrap();
    assert_eq!(fit.n, 4);
    close(fit.coef[0], 0.0, 1e-9);
    close(fit.coef[1], 2.0, 1e-9);
}

#[test]
fn ols_rejects_collinear_and_underdetermined() {
    let x = Series::from_f64(vec![1.0, 2.0, 3.0, 4.0]);
    let y = Series::from_f64(vec![1.0, 2.0, 3.0, 4.0]);
    // x duplicated → singular Gram matrix.
    assert!(ols(&y, &[&x, &x], false).is_err());
    // Two rows, three coefficients.
    let tiny_y = Series::from_f64(vec![1.0, 2.0]);
    let tiny_x = Series::from_f64(vec![1.0, 2.0]);
    assert!(ols(&tiny_y, &[&tiny_x, &tiny_x], false).is_err());
}

//! The data-ergonomics stats kernels against SciPy (and statsmodels'
//! formulas) on fixed data: tests/fixtures/stats/reference.json, written
//! by make_reference.py beside it. The tests never run Python.

#![cfg(feature = "stats")]

use olang_ods::dist::{Dist, DistFn, n_for_power, power_t};
use olang_ods::robust::{self, MAD_NORMAL};
use olang_ods::stats::{mann_whitney, spearman, t_test_one_sample, t_test_welch};
use olang_ods::ts::{self, HoltWinters, Model, Season};
use olang_ods::{Series, dist};
use serde_json::Value;

fn reference() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/stats/reference.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("reference.json")).expect("json")
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn fs(v: &Value) -> Vec<f64> {
    v.as_array().expect("array").iter().map(f).collect()
}

fn close(got: f64, want: f64, tol: f64, what: &str) {
    if want.is_infinite() || got.is_infinite() {
        assert_eq!(got, want, "{what}");
        return;
    }
    assert!(
        (got - want).abs() <= tol * (1.0 + want.abs()),
        "{what}: got {got}, want {want}"
    );
}

#[test]
fn every_family_matches_scipy() {
    let r = reference();
    for (name, d) in r["dist"].as_object().unwrap() {
        let params = fs(&d["params"]);
        let dist = Dist::new(name, &params).unwrap();
        let xs = fs(&d["x"]);
        for (i, x) in xs.iter().enumerate() {
            close(
                dist.eval(DistFn::Pdf, *x),
                f(&d["density"][i]),
                1e-9,
                &format!("{name} density({x})"),
            );
            close(
                dist.eval(DistFn::Cdf, *x),
                f(&d["cdf"][i]),
                1e-9,
                &format!("{name} cdf({x})"),
            );
        }
        // the vectorized pass is the scalar one
        let col = dist.apply(DistFn::Cdf, &xs, true);
        for (i, x) in xs.iter().enumerate() {
            assert_eq!(col[i], dist.eval(DistFn::Cdf, *x));
        }
        for (i, p) in fs(&d["p"]).iter().enumerate() {
            let tol = if name == "beta" || name == "gamma" {
                1e-7
            } else {
                1e-8
            };
            close(
                dist.eval(DistFn::Ppf, *p),
                f(&d["ppf"][i]),
                tol,
                &format!("{name} ppf({p})"),
            );
        }
    }
}

#[test]
fn t_test_interval_and_effect_match_scipy() {
    let r = &reference()["t_test"];
    let a = Series::from_f64(fs(&r["a"]));
    let b = Series::from_f64(fs(&r["b"]));
    let t = t_test_welch(&a, &b).unwrap();
    close(t.t, f(&r["t"]), 1e-10, "t");
    close(t.df, f(&r["df"]), 1e-10, "df");
    close(t.p_value, f(&r["p"]), 1e-9, "p");
    let diff = t.mean - t.mean_other;
    let q = dist::t_ppf(0.975, t.df).unwrap();
    close(diff - q * t.se, f(&r["lo"]), 1e-8, "lo");
    close(diff + q * t.se, f(&r["hi"]), 1e-8, "hi");
    close(t.d, f(&r["d"]), 1e-10, "d");
    close(
        1.0 - dist::t_cdf(t.t, t.df).unwrap(),
        f(&r["p_greater"]),
        1e-9,
        "p greater",
    );
    close(
        diff - dist::t_ppf(0.95, t.df).unwrap() * t.se,
        f(&r["lo_greater"]),
        1e-8,
        "lo greater",
    );
    let one = t_test_one_sample(&a, f(&r["one_mu"])).unwrap();
    close(one.t, f(&r["one_t"]), 1e-10, "one t");
    close(one.p_value, f(&r["one_p"]), 1e-9, "one p");
    let q = dist::t_ppf(0.95, one.df).unwrap();
    // one sample: the interval of the mean (SciPy's and R's)
    close(one.mean - q * one.se, f(&r["one_lo90"]), 1e-8, "one lo");
    close(one.mean + q * one.se, f(&r["one_hi90"]), 1e-8, "one hi");
    close(one.d, f(&r["one_d"]), 1e-10, "one d");
}

#[test]
fn rank_tests_match_scipy() {
    let r = &reference()["rank"];
    let x = Series::from_f64(fs(&r["x"]));
    let y = Series::from_f64(fs(&r["y"]));
    let mw = mann_whitney(&x, &y, "two-sided").unwrap();
    close(mw.u, f(&r["u"]), 1e-12, "U");
    close(mw.p_value, f(&r["p"]), 1e-10, "p");
    close(
        mann_whitney(&x, &y, "less").unwrap().p_value,
        f(&r["p_less"]),
        1e-10,
        "p less",
    );
    let s = spearman(
        &Series::from_f64(fs(&r["s_x"])),
        &Series::from_f64(fs(&r["s_y"])),
    )
    .unwrap();
    close(s.rho, f(&r["rho"]), 1e-12, "rho");
    close(s.p_value, f(&r["rho_p"]), 1e-9, "rho p");
}

#[test]
fn robust_statistics_match_scipy() {
    let r = &reference()["robust"];
    let x = Series::from_f64(fs(&r["x"]));
    close(
        robust::mad(&x, MAD_NORMAL).unwrap(),
        f(&r["mad_normal"]),
        1e-12,
        "mad normal",
    );
    close(
        robust::mad(&x, 1.0).unwrap(),
        f(&r["mad_raw"]),
        1e-12,
        "mad",
    );
    close(robust::iqr(&x).unwrap(), f(&r["iqr"]), 1e-12, "iqr");
    let (lo, hi) = robust::tukey_fences(&x, 2.0).unwrap();
    close(lo, f(&r["fence_lo"]), 1e-12, "fence lo");
    close(hi, f(&r["fence_hi"]), 1e-12, "fence hi");
    for (robust_, key) in [(false, "z"), (true, "z_robust")] {
        let z = robust::zscore(&x, robust_, false).unwrap();
        let Series::F64 { values, .. } = z else {
            panic!()
        };
        for (g, w) in values.iter().zip(fs(&r[key])) {
            close(*g, w, 1e-10, key);
        }
    }
}

#[test]
fn power_matches_the_noncentral_t() {
    let r = &reference()["power"];
    for c in r["cases"].as_array().unwrap() {
        let got = power_t(
            f(&c["effect"]),
            f(&c["n"]),
            f(&c["alpha"]),
            c["alternative"].as_str().unwrap(),
            c["two_sample"].as_bool().unwrap(),
            f(&c["ratio"]),
        )
        .unwrap();
        close(got, f(&c["power"]), 1e-9, &format!("{c}"));
    }
    for c in r["solved"].as_array().unwrap() {
        let got = n_for_power(
            f(&c["effect"]),
            f(&c["power"]),
            0.05,
            c["alternative"].as_str().unwrap(),
            c["two_sample"].as_bool().unwrap(),
            1.0,
        )
        .unwrap();
        close(got, f(&c["n"]), 1e-7, &format!("{c}"));
    }
}

fn opt(v: &Value) -> Vec<Option<f64>> {
    v.as_array().unwrap().iter().map(|x| x.as_f64()).collect()
}

fn same_with_gaps(got: &[f64], want: &[Option<f64>], tol: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what} length");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        match w {
            None => assert!(g.is_nan(), "{what}[{i}]: got {g}, want a gap"),
            Some(w) => close(*g, *w, tol, &format!("{what}[{i}]")),
        }
    }
}

#[test]
fn acf_pacf_and_decompose_match_statsmodels() {
    let r = &reference()["ts"];
    let y = fs(&r["y"]);
    let c = ts::acf(&y, 30, 0.95).unwrap();
    for (k, w) in fs(&r["acf"]).iter().enumerate() {
        close(c.acf[k], *w, 1e-10, &format!("acf[{k}]"));
        close(c.lo[k], f(&r["acf_lo"][k]), 1e-9, "acf lo");
        close(c.hi[k], f(&r["acf_hi"][k]), 1e-9, "acf hi");
    }
    for (key, adjusted) in [("pacf_yw", true), ("pacf_ywm", false)] {
        let p = ts::pacf(&y, 20, adjusted, 0.95).unwrap();
        for (k, w) in fs(&r[key]).iter().enumerate() {
            close(p.acf[k], *w, 1e-9, &format!("{key}[{k}]"));
        }
        close(
            p.hi[3] - p.acf[3],
            f(&r["pacf_half"]),
            1e-12,
            "pacf half width",
        );
    }
    for (model, sfx) in [(Model::Additive, ""), (Model::Multiplicative, "_mul")] {
        let d = ts::decompose(&y, 12, model).unwrap();
        same_with_gaps(
            &d.trend,
            &opt(&r[format!("trend{sfx}").as_str()]),
            1e-10,
            "trend",
        );
        same_with_gaps(
            &d.seasonal,
            &opt(&r[format!("seasonal{sfx}").as_str()]),
            1e-10,
            "seasonal",
        );
        same_with_gaps(
            &d.resid,
            &opt(&r[format!("resid{sfx}").as_str()]),
            1e-9,
            "resid",
        );
    }
}

#[test]
fn holt_winters_recursion_and_fit() {
    let r = &reference()["ts"];
    let y = fs(&r["y"]);
    let rates = fs(&r["hw_rates"]);
    let hw = HoltWinters::fit(
        &y,
        12,
        true,
        Season::Add,
        Some(rates[0]),
        Some(rates[1]),
        Some(rates[2]),
    )
    .unwrap();
    same_with_gaps(&hw.fitted, &opt(&r["hw_fitted"]), 1e-9, "fitted");
    close(hw.sse, f(&r["hw_sse"]), 1e-9, "sse");
    for (g, w) in hw.forecast(12).iter().zip(fs(&r["hw_forecast"])) {
        close(*g, w, 1e-9, "forecast");
    }
    let pos: Vec<f64> = y.iter().map(|v| v + 10.0).collect();
    let mul = HoltWinters::fit(
        &pos,
        12,
        true,
        Season::Mul,
        Some(rates[0]),
        Some(rates[1]),
        Some(rates[2]),
    )
    .unwrap();
    same_with_gaps(&mul.fitted, &opt(&r["hw_mul_fitted"]), 1e-9, "mul fitted");
    close(mul.sse, f(&r["hw_mul_sse"]), 1e-9, "mul sse");
    for (g, w) in mul.forecast(12).iter().zip(fs(&r["hw_mul_forecast"])) {
        close(*g, w, 1e-9, "mul forecast");
    }
    // Fitted: no worse than SciPy's bounded L-BFGS-B from five starts.
    let fit = HoltWinters::fit(&y, 12, true, Season::Add, None, None, None).unwrap();
    let best = f(&r["hw_best_sse"]);
    assert!(
        fit.sse <= best * (1.0 + 1e-4),
        "Nelder–Mead {} against L-BFGS-B {}",
        fit.sse,
        best
    );
    assert!(
        (0.0..=1.0).contains(&fit.alpha)
            && (0.0..=1.0).contains(&fit.beta)
            && (0.0..=1.0).contains(&fit.gamma)
    );
}

#[test]
fn bootstrap_interval_covers_the_textbook_one() {
    // The mean of 400 normals: the percentile interval of 4,000 resamples
    // is within a few percent of mean ± 1.96 sd/√n.
    let mut g = olang_ods::random::Pcg64::new(11);
    let xs = olang_ods::random::normal(&mut g, 400, 5.0, 2.0, false);
    let s = Series::from_f64(xs.clone());
    let b = robust::bootstrap(
        &s,
        None,
        robust::Stat::Mean,
        robust::Compare::Diff,
        4000,
        3,
        0.95,
        true,
    )
    .unwrap();
    let m = xs.iter().sum::<f64>() / 400.0;
    let sd = (xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / 399.0).sqrt();
    let half = 1.959964 * sd / 20.0;
    close(b.estimate, m, 1e-12, "estimate");
    assert!(
        (b.lo - (m - half)).abs() < 0.08 * half,
        "{} {}",
        b.lo,
        m - half
    );
    assert!(
        (b.hi - (m + half)).abs() < 0.08 * half,
        "{} {}",
        b.hi,
        m + half
    );
    close(b.se, sd / 20.0, 0.06, "se");
}

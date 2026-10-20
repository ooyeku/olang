//! Statistics and time series (data ergonomics D5): vectorized
//! distributions, the generator's columns, the bootstrap, the FFT
//! autocorrelation and the Holt-Winters fit. Run with `cargo bench -p
//! olang-ods --bench stats` (`-- 1000000` for another size);
//! `benches/stats.py` times NumPy and SciPy on the same work, and
//! docs/performance.md records both.

use olang_ods::Series;
use olang_ods::dist::{Dist, DistFn};
use olang_ods::random::{self, Pcg64};
use olang_ods::robust::{self, Compare, Stat};
use olang_ods::ts::{self, HoltWinters, Season};
use std::hint::black_box;
use std::time::Instant;

/// Milliseconds: the fastest of seven rounds.
fn time(mut f: impl FnMut()) -> f64 {
    f();
    let mut best = f64::MAX;
    for _ in 0..7 {
        let t0 = Instant::now();
        f();
        best = best.min(t0.elapsed().as_secs_f64() * 1e3);
    }
    best
}

fn row(name: &str, seq: f64, par: Option<f64>) {
    match par {
        Some(p) => println!("{name:<34} {seq:>9.2} {p:>9.2}"),
        None => println!("{name:<34} {seq:>9.2} {:>9}", "—"),
    }
}

fn main() {
    let n: usize = std::env::args()
        .skip(1)
        .find_map(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    println!(
        "n = {n}; threads {}; milliseconds",
        rayon::current_num_threads()
    );
    println!("{:<34} {:>9} {:>9}", "kernel", "seq", "par");

    let mut g = Pcg64::new(1);
    let x = random::normal(&mut g, n, 0.0, 1.0, false);
    let u = random::uniform(&mut g, n, 0.0, 1.0, false);

    for (name, fam, params, f, data) in [
        ("norm.cdf", "norm", vec![0.0, 1.0], DistFn::Cdf, &x),
        ("norm.pdf", "norm", vec![0.0, 1.0], DistFn::Pdf, &x),
        ("norm.ppf", "norm", vec![0.0, 1.0], DistFn::Ppf, &u),
        ("t.cdf (df 10)", "t", vec![10.0], DistFn::Cdf, &x),
        (
            "gamma.cdf (shape 2)",
            "gamma",
            vec![2.0, 1.0],
            DistFn::Cdf,
            &u,
        ),
    ] {
        let d = Dist::new(fam, &params).unwrap();
        let seq = time(|| {
            black_box(d.apply(f, data, false));
        });
        let par = time(|| {
            black_box(d.apply(f, data, true));
        });
        row(name, seq, Some(par));
    }

    for (name, which) in [
        ("rng.normal", 0),
        ("rng.uniform", 1),
        ("rng.poisson (lam 3)", 2),
        ("rng.integers", 3),
    ] {
        let draw = |par: bool| {
            let mut g = Pcg64::new(7);
            match which {
                0 => black_box(random::normal(&mut g, n, 0.0, 1.0, par).len()),
                1 => black_box(random::uniform(&mut g, n, 0.0, 1.0, par).len()),
                2 => black_box(random::poisson(&mut g, n, 3.0, par).len()),
                _ => black_box(random::integers(&mut g, n, 0, 100, par).len()),
            }
        };
        let seq = time(|| {
            draw(false);
        });
        let par = time(|| {
            draw(true);
        });
        row(name, seq, Some(par));
    }
    let w: Vec<f64> = (0..50).map(|i| (i + 1) as f64).collect();
    let seq = time(|| {
        let mut g = Pcg64::new(7);
        black_box(random::choice(&mut g, 50, n, Some(&w), true, false).unwrap());
    });
    let par = time(|| {
        let mut g = Pcg64::new(7);
        black_box(random::choice(&mut g, 50, n, Some(&w), true, true).unwrap());
    });
    row("rng.choice (50 weights)", seq, Some(par));

    // The bootstrap: 10,000 resamples of the mean of 10,000 values.
    let small = Series::from_f64(x[..10_000.min(n)].to_vec());
    for (name, stat) in [
        ("bootstrap mean 10k x 10k", Stat::Mean),
        ("bootstrap median 10k x 2k", Stat::Median),
    ] {
        let reps = if stat == Stat::Mean { 10_000 } else { 2_000 };
        let seq = time(|| {
            black_box(
                robust::bootstrap(&small, None, stat, Compare::Diff, reps, 1, 0.95, false).unwrap(),
            );
        });
        let par = time(|| {
            black_box(
                robust::bootstrap(&small, None, stat, Compare::Diff, reps, 1, 0.95, true).unwrap(),
            );
        });
        row(name, seq, Some(par));
    }

    let s = Series::from_f64(x.clone());
    let spec = olang_ods::numfmt::Spec::parse(",.2f").unwrap();
    row(
        "format \",.2f\"",
        time(|| {
            black_box(olang_ods::text::format_numbers(&s, &spec).unwrap());
        }),
        None,
    );
    let pct = olang_ods::numfmt::Spec::parse(".1%").unwrap();
    row(
        "format \".1%\"",
        time(|| {
            black_box(olang_ods::text::format_numbers(&s, &pct).unwrap());
        }),
        None,
    );
    row(
        "mad",
        time(|| {
            black_box(robust::mad(&s, robust::MAD_NORMAL).unwrap());
        }),
        None,
    );

    // The autocorrelation of the whole column to 100 lags, and of the
    // energy notebook's 26,280 hours to 336.
    let walk: Vec<f64> = x
        .iter()
        .scan(0.0, |a, v| {
            *a = *a * 0.9 + v;
            Some(*a)
        })
        .collect();
    row(
        "acf (100 lags)",
        time(|| {
            black_box(ts::acf(&walk, 100, 0.95).unwrap());
        }),
        Some(time(|| {
            black_box(ts::acf_par(&walk, 100, 0.95, true).unwrap());
        })),
    );
    let hours = &walk[..26_280.min(n)];
    row(
        "acf 26,280 h (336 lags)",
        time(|| {
            black_box(ts::acf(hours, 336, 0.95).unwrap());
        }),
        None,
    );
    row(
        "pacf 26,280 h (48 lags)",
        time(|| {
            black_box(ts::pacf(hours, 48, true, 0.95).unwrap());
        }),
        None,
    );
    row(
        "decompose 26,280 h (168)",
        time(|| {
            black_box(ts::decompose(hours, 168, ts::Model::Additive).unwrap());
        }),
        None,
    );

    // Holt-Winters fits: three years of days (season 7), and the energy
    // notebook's hours (season 168).
    let days: Vec<f64> = (0..1095)
        .map(|t| 1000.0 + 0.1 * t as f64 + 50.0 * ((t % 7) as f64 - 3.0) + 20.0 * walk[t])
        .collect();
    row(
        "holt_winters fit 1,095 d (7)",
        time(|| {
            black_box(HoltWinters::fit(&days, 7, true, Season::Add, None, None, None).unwrap());
        }),
        None,
    );
    let hourly: Vec<f64> = (0..26_280.min(n))
        .map(|t| {
            1000.0
                + 0.001 * t as f64
                + 80.0 * (((t % 24) as f64) / 24.0 * 6.283).sin()
                + 10.0 * walk[t]
        })
        .collect();
    let t0 = Instant::now();
    black_box(HoltWinters::fit(&hourly, 168, true, Season::Add, None, None, None).unwrap());
    row(
        "holt_winters fit 26,280 h (168)",
        t0.elapsed().as_secs_f64() * 1e3,
        None,
    );
}

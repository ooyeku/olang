//! Statistics, generators, formatting and Frames in SQLite (data
//! ergonomics D5) on every tier: the interpreter alone (`--no-ovm`),
//! every function compiled at its first call (`--ovm-tier=1`), and the
//! default. The numbers themselves are pinned against SciPy in
//! olang-ods/tests/stats_reference.rs; here the language surface is:
//! vectorized distributions agree with the scalar calls, keywords and
//! methods bind, a generator's draws are the same on every tier and every
//! thread count, and the errors say the same thing everywhere.

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run(code: &str, flags: &[&str], tag: &str, env: &[(&str, &str)]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("olang_stats_ergo_{}_{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("p{}.ol", flags.len()));
    std::fs::write(&path, code).unwrap();
    let mut cmd = Command::new(olang());
    cmd.args(flags).arg(&path).current_dir(&dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

/// The program prints `expected` on every tier. The work happens inside
/// `fn main_()`, so the compiled tier compiles it.
fn agrees(tag: &str, body: &str, expected: &str) {
    let code =
        format!("fn every(xs) = all(xs, (x) => x == true)\nfn main_() = {{\n{body}\n}}\nmain_()\n");
    for flags in TIERS {
        let (out, err) = run(&code, flags, tag, &[]);
        assert_eq!(
            out.trim_end(),
            expected.trim_end(),
            "flags {flags:?}\nstderr: {err}\nprogram:\n{code}"
        );
    }
}

/// The program raises an error holding `needle`, the same on every tier.
fn raises(tag: &str, body: &str, needle: &str) {
    let code = format!("fn main_() = {{\n{body}\n}}\nmain_()\n");
    let mut first: Option<String> = None;
    for flags in TIERS {
        let (_, err) = run(&code, flags, tag, &[]);
        assert!(
            err.contains(needle),
            "flags {flags:?}: {err}\n  wanted: {needle}"
        );
        let line = err
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or("")
            .to_string();
        match &first {
            None => first = Some(line),
            Some(f) => assert_eq!(f, &line, "tiers raise differently"),
        }
    }
}

#[test]
fn a_distribution_over_a_column_is_the_scalar_call_per_value() {
    agrees(
        "dists",
        r#"
    let xs = [0.0, 0.3, 1.0, 2.5, 7.0]
    let ps = [0.01, 0.2, 0.5, 0.8, 0.99]
    let col = ods.series(xs)
    let pcol = ods.series(ps)
    fn same(c, f) = c.to_list() == map(xs, f)
    fn samep(c, f) = c.to_list() == map(ps, f)
    println(every([
        same(stats.norm.cdf(col, 1.0, 2.0), (x) => stats.norm.cdf(x, 1.0, 2.0)),
        same(stats.t.pdf(col, 5.0), (x) => stats.t.pdf(x, 5.0)),
        same(stats.chi2.cdf(col, 3.0), (x) => stats.chi2.cdf(x, 3.0)),
        same(stats.f.cdf(col, 3.0, 9.0), (x) => stats.f.cdf(x, 3.0, 9.0)),
        same(stats.lognorm.pdf(col, 0.2, 0.5), (x) => stats.lognorm.pdf(x, 0.2, 0.5)),
        same(stats.gamma.cdf(col, 2.0, scale: 1.5), (x) => stats.gamma.cdf(x, 2.0, 1.5)),
        same(stats.beta.cdf(col / 8.0, 2.0, 3.0), (x) => stats.beta.cdf(x / 8.0, 2.0, 3.0)),
        same(stats.poisson.pmf(col, 2.5), (x) => stats.poisson.pmf(x, 2.5)),
        same(stats.binom.cdf(col, 10, 0.3), (x) => stats.binom.cdf(x, 10, 0.3)),
        samep(stats.norm.ppf(pcol), (p) => stats.norm.ppf(p)),
        samep(stats.binom.ppf(pcol, trials: 12, p: 0.4), (p) => stats.binom.ppf(p, 12, 0.4)),
    ]))
    // nulls stay null; an Int column reads as numbers
    println(stats.norm.cdf(ods.series([0, (), 1])).to_list() |> map((p) => if p == () => p else => str.fmt(p, ".10f")))
    println(stats.poisson.pmf(3, 2.0) |> str.fmt(".6f"))
    println(stats.norm.ppf(ods.series([0.5, 1.5])).to_list())
"#,
        "true\n[\"0.5000000000\", (), \"0.8413447461\"]\n0.180447\n[0.0, NaN]",
    );
}

#[test]
fn tests_power_and_robust_statistics() {
    agrees(
        "tests",
        r#"
    let a = ods.series([5.1, 4.9, 6.2, 5.7, 5.5, 4.8, 5.9, 6.1])
    let b = ods.series([4.2, 4.8, 4.5, 5.0, 4.4, 4.1, 4.9])
    let t = stats.t_test(a, b)
    println(`${t.t |> str.fmt(".6f")} ${t.df |> str.fmt(".4f")} ${t.p_value |> str.fmt(".6f")}`)
    println(`${t.ci[0] |> str.fmt(".6f")} ${t.ci[1] |> str.fmt(".6f")} ${t.d |> str.fmt(".6f")}`)
    let g = a.t_test(b, level: 0.9, alternative: "greater")
    println(`${g.p_value |> str.fmt(".6f")} ${g.ci[0] |> str.fmt(".6f")} ${g.ci[1]}`)
    let one = stats.t_test(a, 5.0)
    println(`${one.mean |> str.fmt(".4f")} ${one.ci[0] |> str.fmt(".4f")} ${one.ci[1] |> str.fmt(".4f")}`)
    println(stats.power_t(0.5, 64) |> str.fmt(".6f"))
    println(stats.power_t(0.3, 50, kind: "one-sample") |> str.fmt(".6f"))
    println(stats.n_for_power(0.5) |> str.fmt(".4f"))
    let mw = stats.mann_whitney(a, b)
    println(`${mw.u} ${mw.p_value |> str.fmt(".6f")} ${mw.effect |> str.fmt(".4f")}`)
    let sp = a.spearman(ods.series([1.0, 0.0, 6.0, 4.0, 3.0, -1.0, 5.0, 5.5]))
    println(`${sp.rho |> str.fmt(".6f")} ${sp.p_value |> str.fmt(".6f")}`)
    let x = ods.series([1.0, 2.0, 3.0, 4.0, 100.0, ()])
    println([stats.mad(x, scale: 1.0), stats.iqr(x), x.mad() |> str.fmt(".4f")])
    println(stats.tukey_fences(x))
    println(x.zscore(robust: true).to_list())
"#,
        // SciPy: ttest_ind(equal_var=False) and its confidence_interval(),
        // ttest_1samp, mannwhitneyu(method="asymptotic"), spearmanr,
        // median_abs_deviation(scale="normal"); the power is the
        // noncentral t's (statsmodels' TTestPower)
        "4.155484 12.0747 0.001316\n0.460736 1.474978 2.088402\n0.000658 0.652093 inf\n5.5250 5.0720 5.9780\n0.801460\n0.547657\n63.7656\n52.0 0.006441 0.9286\n1.000000 0.000000\n[1.0, 2.0, \"1.4826\"]\n(-1.0, 7.0)\n[-1.3489795003921634, -0.6744897501960817, 0.0, 0.6744897501960817, 65.42550576901992, ()]",
    );
}

#[test]
fn the_bootstrap_is_native_seeded_and_the_same_on_every_thread_count() {
    let body = r#"
    let x = random.rng(3).normal(300, 10.0, 2.0)
    let b = stats.bootstrap(x, "median", n: 500, seed: 9)
    let again = x.bootstrap("median", n: 500, rng: 9)
    println(b.draws.to_list() == again.draws.to_list())
    println(`${b.estimate |> str.fmt(".6f")} ${b.lo |> str.fmt(".6f")} ${b.hi |> str.fmt(".6f")} ${b.se |> str.fmt(".6f")}`)
    let q = stats.bootstrap(x, 0.9, n: 200, seed: 1, level: 0.8)
    println(q.lo < q.estimate && q.estimate < q.hi)
    let two = stats.bootstrap([x + 1.0, x], compare: "diff", n: 300, seed: 4)
    println(two.estimate |> str.fmt(".6f"))
    let r = stats.bootstrap([x * 2.0, x], "mean", compare: "ratio", seed: 4)
    println(`${r.estimate |> str.fmt(".6f")} ${r.draws.len()}`)
"#;
    let code = format!("fn main_() = {{\n{body}\n}}\nmain_()\n");
    let mut outs = Vec::new();
    for flags in TIERS {
        for threads in ["1", "8"] {
            let (out, err) = run(
                &code,
                flags,
                "boot",
                &[
                    ("OLANG_PARALLELISM", threads),
                    ("RAYON_NUM_THREADS", threads),
                ],
            );
            assert!(err.is_empty(), "{flags:?}: {err}");
            outs.push(out);
        }
    }
    assert!(outs.iter().all(|o| *o == outs[0]), "{outs:?}");
    let lines: Vec<&str> = outs[0].lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "1.000000");
    assert!(lines[4].starts_with("2.000000 2000"), "{}", lines[4]);
}

#[test]
fn a_generator_draws_the_same_columns_on_every_tier_and_thread_count() {
    // past a million values a draw runs in parallel blocks: the values
    // are the sequential ones whatever the thread count
    let body = r#"
    let g = random.rng(2026)
    let big = g.normal(1500000, 1.0, 2.0)
    println([big.len(), big.sum() |> str.fmt(".6f")])
    let p = g.poisson(1200000, 4.5)
    println([p.sum(), p.max()])
    println(g.normal(3).to_list())
    println(g.uniform(2, -1.0, 1.0).to_list())
    println(g.integers(6, 0, 10).to_list())
    println(g.lognormal(2, sigma: 0.5).to_list())
    println(g.binomial(4, trials: 20, p: 0.25).to_list())
    println(g.choice(["a", "b", "c"], 6, weights: [1, 0, 3]).to_list())
    println(g.choice(ods.series([10, 20, 30, 40]), 4, replace: false).to_list())
    println(g.shuffle([1, 2, 3, 4, 5]))
    println(g.permutation(5).to_list())
    println([g.normal(), g.integers(low: 0, high: 2), g.choice([7, 8])])
    // children: one per task, the same results in any schedule
    let sums = par_map(random.rng(5).spawn(6), (r) => r.normal(200000).sum() |> str.fmt(".6f"))
    println(sums == map(random.rng(5).spawn(6), (r) => r.normal(200000).sum() |> str.fmt(".6f")))
    println(sums[0] != sums[1])
    // independent of the global stream, both ways
    random.seed(1)
    let a = random.random()
    random.seed(1)
    let h = random.rng(4)
    let _ = h.normal(10)
    println(a == random.random())
"#;
    let code = format!("fn main_() = {{\n{body}\n}}\nmain_()\n");
    let mut outs = Vec::new();
    for flags in TIERS {
        for threads in ["1", "8"] {
            let (out, err) = run(
                &code,
                flags,
                "rng",
                &[
                    ("OLANG_PARALLELISM", threads),
                    ("RAYON_NUM_THREADS", threads),
                ],
            );
            assert!(err.is_empty(), "{flags:?} threads {threads}: {err}");
            outs.push(out);
        }
    }
    assert!(outs.iter().all(|o| *o == outs[0]), "{outs:?}");
    let lines: Vec<&str> = outs[0].lines().collect();
    assert!(lines[0].starts_with("[1500000, "), "{}", lines[0]);
    assert!(
        !lines[7].contains("\"b\""),
        "a zero weight is never drawn: {}",
        lines[7]
    );
    let n = lines.len();
    assert_eq!(&lines[n - 3..], &["true", "true", "true"]);
}

#[test]
fn the_data_stack_takes_a_generator_where_it_takes_a_seed() {
    agrees(
        "rngargs",
        r#"
    let f = ods.frame({ x: ods.arange(0, 60).cast("Float") })
    println(ods.sample(f, 4, seed: 3)["x"].to_list() == f.sample(4, rng: 3)["x"].to_list())
    let g = random.rng(8)
    println(f.sample(4, rng: g)["x"].to_list() != f.sample(4, rng: g)["x"].to_list())
    println(f["x"].sample(3, seed: 2).len())
    let s1 = learn.split(f, test: 0.25, rng: 6)
    let s2 = learn.split(f, test: 0.25, rng: random.rng(6))
    println(s1.test.n_rows() == 15 && s2.test.n_rows() == 15)
    println(stats.norm.sample(4, rng: 11).to_list() == stats.norm.sample(4, 0.0, 1.0, random.rng(11)).to_list())
    println(stats.binom.sample(3, 10, 0.5, rng: 1).to_list())
"#,
        "true\ntrue\n3\ntrue\ntrue\n[5, 4, 5]",
    );
    raises(
        "both",
        "learn.split(ods.frame({ x: [1.0, 2.0] }), seed: 1, rng: 2)",
        "give seed or rng, not both",
    );
    raises(
        "bad_rng",
        "ods.sample(ods.series([1, 2]), 1, rng: \"x\")",
        "rng must be a generator",
    );
}

#[test]
fn time_series_functions_and_the_holt_winters_value() {
    agrees(
        "ts",
        r#"
    let t = ods.arange(0, 140)
    let y = t * 0.5 + 100.0 + (t % 7).cast("Float") * 2.0
    let hw = stats.ts.holt_winters(y, season: 7)
    println(hw.forecast(3).to_list() |> map((v) => str.fmt(v, ".4f")))
    println([hw.n, hw.m, hw.sse < 1e-9, hw.fitted.null_count(), hw.season.len()])
    println(hw.params.alpha >= 0.0 && hw.params.alpha <= 1.0)
    let fixed = y.holt_winters(season: 7, alpha: 0.3, beta: 0.1, gamma: 0.2, fit: false)
    println(fixed.params)
    let simple = stats.ts.holt_winters(ods.series([3.0, 5.0, 4.0, 6.0, 5.0]), trend: "none", alpha: 0.5)
    println([simple.params.beta, simple.forecast(2).to_list()])
    let mul = stats.ts.holt_winters(y, season: 7, seasonal: "mul", trend: "none")
    println(mul.m)
    let c = y.diff().fill_null(0.0).acf(14)
    println([c.n_rows(), c["acf"][7] > 0.9, c["lo"][0], c["hi"][0]])
    let p = stats.ts.pacf(y, 5, method: "ywm")
    println(p.columns())
    let d = stats.ts.decompose(y, 7)
    println([d.n_rows(), d["trend"].null_count(), d["seasonal"][0] |> str.fmt(".4f")])
    let start = ods.series(["2026-01-31"]).to_date()[0]
    let months = ods.series(map(range(0, 24), (k) => unwrap(dates.add_months(start, k))))
    let sn = stats.ts.seasonal_naive(ods.arange(0, 24).cast("Float"), 12, 3, time: months)
    println([sn["time"].to_list() |> map((d) => to_string(d)), sn["forecast"].to_list()])
    println(stats.ts.decompose(y, 7, model: "multiplicative")["resid"].null_count())
"#,
        "[\"170.0000\", \"172.5000\", \"175.0000\"]\n[140, 7, true, 7, 7]\ntrue\n#{\"alpha\": 0.3, \"beta\": 0.1, \"gamma\": 0.2}\n[(), [5.0, 5.0]]\n7\n[15, true, 1.0, 1.0]\n[\"lag\", \"pacf\", \"lo\", \"hi\"]\n[140, 6, \"-6.0000\"]\n[[\"2028-01-31\", \"2028-02-29\", \"2028-03-31\"], [12.0, 13.0, 14.0]]\n6",
    );
    raises(
        "hw_fit",
        "stats.ts.holt_winters(ods.series([1.0, 2.0, 3.0, 4.0]), season: 2, fit: false)",
        "fit: false needs every rate",
    );
    raises(
        "ts_null",
        "stats.ts.acf(ods.series([1.0, (), 3.0]), 1)",
        "the series holds 1 null",
    );
    raises(
        "hw_field",
        "let m = stats.ts.holt_winters(ods.series([1.0, 2.0, 3.0])); m.forecast",
        "'forecast' is a method",
    );
}

#[test]
fn numbers_format_by_a_spec() {
    agrees(
        "fmt",
        r#"
    println(str.fmt(3.14159, ".2f"))
    println(str.fmt(1234567.891, ",.0f"))
    println(str.fmt(0.1234, ".1%"))
    println(str.fmt(-1234.5, "$,.0f"))
    println(str.fmt(12345.678, "e"))
    println(str.fmt(-0.001, ".2f"))
    println(str.fmt(42, ">6d") + "|")
    println(str.fmt(9007199254740993, ",d"))
    println(str.fmt("{} is {:.1%} of {:,d}", "x", 0.256, 10000))
    println(str.fmt("{} and {}", 1, "b"))
    println([str.pct(0.5), str.pct(0.12345, 2), str.money(1234.5), str.money(-0.4, "€", digits: 2)])
    println(0.0731 |> str.pct())
    println(ods.series([1.5, (), -2.25]).format("+.1f").to_list())
    println(ods.series([1200, 35]).format(",d").to_list())
    println(str.money(ods.series([10.0, 2500.0])).to_list())
"#,
        "3.14\n1,234,568\n12.3%\n-$1,234\n1.234568e+04\n0.00\n    42|\n9,007,199,254,740,993\nx is 25.6% of 10,000\n1 and b\n[\"50.0%\", \"12.35%\", \"$1,234\", \"-€0.40\"]\n7.3%\n[\"+1.5\", (), \"-2.2\"]\n[\"1,200\", \"35\"]\n[\"$10\", \"$2,500\"]",
    );
    raises("badspec", "str.fmt(1.0, \".2x\")", "format spec \".2x\"");
    raises("badfmt", "ods.series([1.0]).format(\".q\")", "format spec");
}

#[test]
fn frames_go_to_sqlite_and_back_typed() {
    agrees(
        "db",
        r#"
    let c = unwrap(db.open(":memory:"))
    let f = ods.frame({
        day: ods.series(["2026-01-05", "2026-01-06", ()]).to_date(),
        at: ods.series(["2026-01-05T10:30:00", "2026-01-06T00:00:00.25", "2026-01-07T23:59:59"]).to_datetime(),
        store: ["north", (), "north"],
        n: [3, 4, ()],
        ok: [true, false, true],
        revenue: [1200.5, 980.0, 1410.25],
        kind: ods.series(["a", "b", "a"]).to_category(),
    })
    println(unwrap(db.insert_frame(c, "t", f)))
    let back = unwrap(db.query_frame(c, "SELECT * FROM t"))
    println(back.columns())
    println(back["day"].to_list() |> map((d) => to_string(d)))
    println(back["at"].to_list() |> map((d) => to_string(d)))
    println([back["store"].to_list(), back["n"].to_list(), back["ok"].to_list(), back["kind"].to_list()])
    println(unwrap(db.insert_frame(c, "t", f)))
    println(unwrap(db.query_frame(c, "SELECT count(*) AS n, sum(revenue) AS r FROM t WHERE n > ?", [3])).to_records())
    println(unwrap(db.insert_frame(c, "t", ods.frame({ x: [1] }), replace: true)))
    println(unwrap(db.query_frame(c, "SELECT * FROM t")).columns())
    println(unwrap(db.query_frame(c, "SELECT x AS a, x AS a FROM t")).columns())
    let empty = unwrap(db.query_frame(c, "SELECT * FROM t WHERE x > 5"))
    println(empty.n_rows())
    match db.query_frame(c, "SELECT * FROM missing") {
        Ok(_) => println("?"),
        Err(e) => println(e),
    }
    unwrap(db.close(c))
"#,
        "3\n[\"day\", \"at\", \"store\", \"n\", \"ok\", \"revenue\", \"kind\"]\n[\"2026-01-05\", \"2026-01-06\", \"()\"]\n[\"2026-01-05T10:30:00\", \"2026-01-06T00:00:00.25\", \"2026-01-07T23:59:59\"]\n[[\"north\", (), \"north\"], [3, 4, ()], [true, false, true], [\"a\", \"b\", \"a\"]]\n3\n[#{\"n\": 2, \"r\": 1960.0}]\n1\n[\"x\"]\n[\"a\", \"a_2\"]\n0\ndb.query_frame: no such table: missing",
    );
}

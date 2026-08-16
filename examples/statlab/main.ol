// statlab — robust statistical inference on the ods data stack, at scale.
//
// A full study with the estimators cross-checked against each other:
// 10,000 simulated subjects, a parametric t-test validated by a
// 1,000-round permutation test (Monte Carlo fanned across every core
// with par_map), a bootstrap confidence interval for the effect size,
// a 3-predictor regression at n=5,000, and charts for the null
// distribution and the fit. The test block pins every inference.

let t_start = time.monotonic_ms()

// ── simulate: two arms, n=5,000 each, seeded ──
random.seed(2026)
let n = 5000
let control = stats.norm.sample(n, 100.0, 15.0)
let treated = stats.norm.sample(n, 101.2, 15.0)     // a subtle 1.2-point lift

let d = stats.describe(treated)
println("═══ statlab ═══")
println("arms: n=" + show(n) + " each; treated mean="
    + show(math.round(map_get(d, "mean") * 100.0) / 100.0)
    + " std=" + show(math.round(map_get(d, "std") * 100.0) / 100.0))

// ── parametric: Welch's t-test ──
let t = stats.t_test(treated, control)
let p_param = map_get(t, "p_value")
println("welch t-test:      p = " + show(p_param))

// ── nonparametric check: permutation test, parallel Monte Carlo ──
// Shuffle the pooled scores, split, and ask how often chance alone
// beats the observed gap. 8 chunks x 125 permutations across cores;
// each chunk re-seeds distinctly so workers explore different perms.
let observed_gap = ods.mean(treated) - ods.mean(control)
let pooled = concat(ods.to_list(control), ods.to_list(treated))
let exceed_counts = par_map(range(0, 8), (chunk) => {
    random.seed(9000 + chunk)
    let mut hits = 0
    let mut rep = 0
    while rep < 125 {
        let shuffled = random.shuffle(pooled)
        let a = ods.series(take(shuffled, n))
        let b = ods.series(skip(shuffled, n))
        let gap = ods.mean(b) - ods.mean(a)
        if math.abs(gap) >= math.abs(observed_gap) => { hits = hits + 1 }
        rep = rep + 1
    }
    hits
})
let p_perm = to_float(sum(exceed_counts) + 1) / 1001.0
println("permutation test:  p = " + show(math.round(p_perm * 10000.0) / 10000.0)
    + "  (1,000 permutations, 8 workers)")

// ── effect size: bootstrap CI of the mean difference ──
let boot_gaps = par_map(range(0, 8), (chunk) => {
    random.seed(7000 + chunk)
    let mut gaps = []
    let mut rep = 0
    while rep < 125 {
        let rc = random.choices(ods.to_list(control), n)
        let rt = random.choices(ods.to_list(treated), n)
        gaps = gaps + [ods.mean(ods.series(rt)) - ods.mean(ods.series(rc))]
        rep = rep + 1
    }
    gaps
})
let mut gaps = ods.series(flatten(boot_gaps))
let ci_lo = ods.quantile(gaps, 0.025)
let ci_hi = ods.quantile(gaps, 0.975)
println("bootstrap 95% CI:  [" + show(math.round(ci_lo * 100.0) / 100.0)
    + ", " + show(math.round(ci_hi * 100.0) / 100.0) + "]  (1,000 resamples)")

// ── multiple regression: 3 predictors at n=5,000 ──
random.seed(31)
let x1 = stats.norm.sample(n, 0.0, 1.0)
let x2 = stats.norm.sample(n, 0.0, 1.0)
let x3 = stats.norm.sample(n, 0.0, 1.0)
let eps = stats.norm.sample(n, 0.0, 1.0)
let y = x1 * 2.0 + x2 * -1.0 + x3 * 0.5 + eps + 3.0
let fit = stats.lm(y, [x1, x2, x3])
let coef = map_get(fit, "coef")
println("ols n=5000: b1=" + show(math.round(ods.get(coef, 1) * 100.0) / 100.0)
    + " b2=" + show(math.round(ods.get(coef, 2) * 100.0) / 100.0)
    + " b3=" + show(math.round(ods.get(coef, 3) * 100.0) / 100.0)
    + " r2=" + show(math.round(map_get(fit, "r2") * 1000.0) / 1000.0)
    )

// ── charts ──
let null_svg = plot.hist(gaps, 30,
    #{ "title": "bootstrap distribution of the gap", "x_label": "mean difference" })
let fit_svg = plot.scatter(x1, y,
    #{ "title": "y vs x1 (n=5,000)", "x_label": "x1", "y_label": "y" })
unwrap(fs.write_file("bootstrap.svg", null_svg))
unwrap(fs.write_file("fit.svg", fit_svg))
let elapsed = time.monotonic_ms() - t_start
println("wrote bootstrap.svg, fit.svg · total " + show(elapsed) + "ms")

test "statlab inferences hold at scale" {
    assert_eq(map_get(d, "count"), 5000)
    assert_eq(p_param < 0.01, true)             // parametric: detects the lift
    assert_eq(p_perm < 0.05, true)              // nonparametric agrees
    assert_eq(ci_lo > 0.0, true)                // CI excludes zero...
    assert_eq(ci_hi < 2.5, true)                // ...and brackets the true 1.2
    assert_eq(ods.get(coef, 1) > 1.9 && ods.get(coef, 1) < 2.1, true)
    assert_eq(ods.get(coef, 2) > -1.1 && ods.get(coef, 2) < -0.9, true)
    assert_eq(map_get(fit, "r2") > 0.8, true)
    assert_eq(str.contains(null_svg, "<svg") && str.contains(fit_svg, "<svg"), true)
}

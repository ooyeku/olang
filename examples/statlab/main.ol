// statlab — statistical inference and charts on the ods data stack.
//
// A miniature study, end to end: simulate a reproducible A/B experiment,
// describe the samples, test the difference, fit a dose-response line,
// and render both as standalone SVG charts. Everything computes for
// real; the test block pins the inferences.

// ── simulate: two arms, seeded so every run agrees ──
random.seed(1234)
let control = stats.norm.sample(200, 100.0, 15.0)
let treated = stats.norm.sample(200, 104.0, 15.0)

let d = stats.describe(treated)
println("═══ statlab ═══")
println("treated arm: n=" + show(map_get(d, "count"))
    + " mean=" + show(math.round(map_get(d, "mean") * 100.0) / 100.0)
    + " std=" + show(math.round(map_get(d, "std") * 100.0) / 100.0))

// ── inference: Welch's t-test on the two arms ──
let t = stats.t_test(treated, control)
let p = map_get(t, "p_value")
println("t-test: p = " + show(p) + " -> " +
    (if p < 0.05 => "significant at 5%" else => "not significant"))

// ── regression: a dose-response fit ──
let dose = ods.series([0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0])
let noise = stats.norm.sample(8, 0.0, 0.6)
let response = dose * 1.8 + 2.0 + noise
let fit = stats.lm(response, dose)
let slope = ods.get(map_get(fit, "coef"), 1)
println("fit: slope=" + show(math.round(slope * 100.0) / 100.0)
    + " r2=" + show(math.round(map_get(fit, "r2") * 1000.0) / 1000.0))

// ── charts: a histogram of the arms' gap, a scatter of the fit ──
let hist_svg = plot.hist(treated, 20, #{ "title": "treated arm", "x_label": "score" })
let fit_svg = plot.scatter(dose, response,
    #{ "title": "dose vs response", "x_label": "dose", "y_label": "response" })
unwrap(fs.write_file("hist.svg", hist_svg))
unwrap(fs.write_file("fit.svg", fit_svg))
println("wrote hist.svg (" + show(str.length(hist_svg)) + " bytes) and fit.svg ("
    + show(str.length(fit_svg)) + " bytes)")

test "statlab inferences hold" {
    assert_eq(map_get(d, "count"), 200)
    assert_eq(p < 0.05, true)                 // 4-point lift, n=200: detectable
    assert_eq(slope > 1.4 && slope < 2.2, true)
    assert_eq(map_get(fit, "r2") > 0.9, true)
    assert_eq(str.contains(hist_svg, "<svg") && str.contains(fit_svg, "<svg"), true)
}

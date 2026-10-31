//! learn, stats and gg behaviour, checked on every tier: the interpreter alone (`--no-ovm`), every function compiled at
//! its first call (`--ovm-tier=1`), and the default.
//!
//! - options given as an object (`{ n_trees: 7 }`) are options, as a
//!   map is, everywhere learn and ml take them; an unknown key raises;
//! - `ml.grid_search` takes an object grid;
//! - a fit's spec keeps its options under `options`;
//! - gg orders a Category by its levels (axis, legend, facet panels);
//! - constant colour mappings in several layers each draw their colour;
//! - `gg.aes` holds a Series (`ymin: f.y - 1.0`);
//! - `gg.density(linewidth:)`, and a rect's missing edge is the panel's;
//! - `stats.prop_z_test` (statsmodels' `proportions_ztest`);
//! - formulas nest (`a / b`) and cross to a power (`(a + b)^2`);
//! - a Frame's methods include learn's (`f.linear("y ~ x")`).

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run(code: &str, flags: &[&str], tag: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("olang_ml_gg_gaps_{}_{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("p{}.ol", flags.len()));
    std::fs::write(&path, code).unwrap();
    let out = Command::new(olang())
        .args(flags)
        .arg(&path)
        .current_dir(&dir)
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

/// The program prints `expected` on every tier. The work happens inside
/// `fn main_()`, so the compiled tier compiles it.
fn agrees(tag: &str, body: &str, expected: &str) {
    let code = format!("use ml\nuse gg\nfn main_() = {{\n{body}\n}}\nmain_()\n");
    for flags in TIERS {
        let (out, err) = run(&code, flags, tag);
        assert_eq!(
            out.trim_end(),
            expected.trim_end(),
            "flags {flags:?}\nstderr: {err}\nprogram:\n{code}"
        );
    }
}

const DATA: &str = r#"
    let f = ods.frame({ x: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], g: ["a", "b", "a", "b", "a", "b", "a", "b"],
                        y: [1.1, 2.3, 2.9, 4.2, 5.1, 5.8, 7.2, 8.1] })
"#;

#[test]
fn options_given_as_an_object_are_options() {
    agrees(
        "objects",
        &format!(
            r#"{DATA}
    let s1 = ml.spec("boost", y: "y", x: ["x"], options: {{ n_trees: 7 }})
    println(s1.options.n_trees)
    println(s1.options.seed != ())
    let s2 = learn.spec("forest", "y", ["x"], {{ n_trees: 5, seed: 3 }})
    println(s2.options.n_trees)
    let fit = learn.forest(f, "y", ["x"], {{ n_trees: 4, seed: 1 }})
    println(fit.spec.options.n_trees)
    println(fit.spec.opts)
    println(learn.fit(s2, f).spec.options.seed)
    let w = learn.logistic(f, "g", ["x"], {{ class_weight: {{ a: 1.0, b: 2.0 }} }})
    println(w.spec.kind)
    let gs = ml.grid_search(ml.spec("ridge", y: "y", x: ["x"]), f, {{ alpha: [0.1, 1.0] }}, folds: 2, seed: 1)
    println(ods.n_rows(gs.results))
    println(gs.best.alpha == 0.1 || gs.best.alpha == 1.0)
"#
        ),
        "7\ntrue\n5\n4\n()\n3\nlogistic\n2\ntrue",
    );
}

#[test]
fn an_unknown_option_raises_where_it_was_ignored() {
    agrees(
        "unknown",
        &format!(
            r#"{DATA}
    println(attempt(() => ml.spec("boost", y: "y", x: ["x"], options: {{ n_tree: 7 }})))
    println(attempt(() => ml.spec("kmeans", x: ["x"], options: #{{ "k": 2, "seeds": 1 }})))
    println(attempt(() => ml.spec("boost", y: "y", x: ["x"], options: 7)))
    println(attempt(() => ml.grid_search(ml.spec("ridge", y: "y", x: ["x"]), f, {{ alpah: [0.1] }}, folds: 2)))
"#
        ),
        "Err(\"learn.spec: unknown option 'n_tree' — the options are weights, task, class_weight, n_trees, learning_rate, max_depth, max_leaf_nodes, min_samples_leaf, l2, subsample, max_bins, early_stopping, validation_fraction, validation_rows, validation_tail, n_iter_no_change, tol, seed\")\n\
         Err(\"learn.spec: unknown option 'seeds' — the options are k, n_init, max_iter, tol, seed\")\n\
         Err(\"learn.spec: the options must be a map (#{...}) or an object ({ k: v }), got Int\")\n\
         Err(\"learn.grid_search: a ridge spec has no option 'alpah' — its options are weights, task, class_weight, alpha, intercept\")",
    );
}

#[test]
fn gg_orders_a_category_by_its_levels() {
    agrees(
        "category",
        r#"
    let f = ods.frame({ m: ods.series(["Feb", "Jan", "Mar", "Feb"]).to_category(levels: ["Jan", "Feb", "Mar"]),
                        k: ods.series(["z", "y", "z", "y"]).to_category(levels: ["z", "y"]),
                        t: ["Feb", "Jan", "Mar", "Feb"], x: [1.0, 2.0, 3.0, 4.0], y: [2.0, 1.0, 3.0, 4.0] })
    let before = (s, a, b) => str.index_of(s, a) < str.index_of(s, b)
    let months = (s) => before(s, ">Jan<", ">Feb<") && before(s, ">Feb<", ">Mar<")
    println(months(gg.svg(gg.plot(f, gg.aes(x: "m", y: "y")) |> gg.col())))
    println(months(gg.svg(gg.plot(f, gg.aes(x: "x", y: "y", color: "m")) |> gg.point())))
    println(months(gg.svg(gg.plot(f, gg.aes(x: "x", y: "y", fill: "m")) |> gg.col())))
    println(months(gg.svg(gg.plot(f, gg.aes(x: "x", y: "y")) |> gg.point() |> gg.facet_wrap("m"))))
    let grid = gg.svg(gg.plot(f, gg.aes(x: "x", y: "y")) |> gg.point() |> gg.facet_grid(rows: "k", cols: "m"))
    println(months(grid) && before(grid, ">z<", ">y<"))
    // limits still win; a String keeps its natural sort
    println(before(gg.svg(gg.plot(f, gg.aes(x: "m", y: "y")) |> gg.col() |> gg.scale_x_discrete(limits: ["Mar", "Feb", "Jan"])), ">Mar<", ">Feb<"))
    println(before(gg.svg(gg.plot(f, gg.aes(x: "t", y: "y")) |> gg.col()), ">Feb<", ">Jan<"))
"#,
        "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue",
    );
}

#[test]
fn constant_colours_in_several_layers_each_draw_their_own() {
    agrees(
        "constants",
        r##"
    let h = ods.frame({ x: [1, 2, 3], a: [1.0, 2.0, 3.0], b: [3.0, 2.0, 1.0] })
    let p = gg.plot(h, gg.aes(x: "x"))
        |> gg.line(gg.aes(y: "a", color: gg.constant("first")))
        |> gg.line(gg.aes(y: "b", color: gg.constant("second")))
        |> gg.scale_color_manual(values: ["#ff0000", "#0000ff"])
    let s = gg.svg(p)
    // each draws its line (it was the first colour for both: 5 and 1)
    println(`${str.count(s, "stroke=\"#ff0000\"")} ${str.count(s, "stroke=\"#0000ff\"")}`)
"##,
        "3 3",
    );
}

#[test]
fn gg_aes_holds_a_series_and_the_small_gaps_close() {
    agrees(
        "aes_series",
        r##"
    let f = ods.frame({ x: [1.0, 2.0, 3.0, 4.0], y: [2.0, 1.0, 3.0, 4.0] })
    let s = gg.svg(gg.plot(f, gg.aes(x: "x", y: "y", ymin: f.y - 1.0, ymax: f.y + 1.0)) |> gg.pointrange())
    println(str.count(s, "<circle") >= 4)
    let thin = gg.svg(gg.plot(f, gg.aes(x: "y")) |> gg.density(linewidth: 0.5))
    let thick = gg.svg(gg.plot(f, gg.aes(x: "y")) |> gg.density(linewidth: 3.0))
    println(thin != thick && str.contains(thick, "stroke-width=\"8.4"))
    // a rect's edge left out is the panel's: a band the panel's height
    let band = gg.svg(gg.plot(f, gg.aes(x: "x", y: "y")) |> gg.point() |> gg.annotate("rect", xmin: 1.5, xmax: 2.5, fill: "#123456"))
    println(len(unwrap(re.find_all("<rect[^>]*#123456", band))))
"##,
        "true\ntrue\n1",
    );
}

#[test]
fn prop_z_test_is_statsmodels_proportions_ztest() {
    // statsmodels 0.15: proportions_ztest, confint_proportions_2indep(method="wald"),
    // proportion_confint(method="normal")
    agrees(
        "prop_z",
        r#"
    let p9 = (x) => str.fmt(x, ".9f")
    let t = stats.prop_z_test([45, 30], [100, 90])
    println(`${p9(t.z)} ${p9(t.p_value)} ${p9(t.ci[0])} ${p9(t.ci[1])} ${p9(t.diff)}`)
    let g = stats.prop_z_test(ods.series([45, 30]), ods.series([100, 90]), alternative: "greater")
    println(`${p9(g.z)} ${p9(g.p_value)}`)
    let l = stats.prop_z_test([45, 30], [100, 90], value: 0.05, alternative: "less")
    println(`${p9(l.z)} ${p9(l.p_value)}`)
    let o = stats.prop_z_test(37, 80, value: 0.5)
    println(`${p9(o.z)} ${p9(o.p_value)} ${p9(o.ci[0])} ${p9(o.ci[1])} ${o.prop}`)
    println(attempt(() => stats.prop_z_test([5, 3], [4, 9])))
"#,
        "1.642726613 0.100439510 -0.021147317 0.254480650 0.116666667\n\
         1.642726613 0.050219755\n\
         0.938700922 0.826057840\n\
         -0.672715073 0.501128577 0.353243269 0.571756731 0.4625\n\
         Err(\"prop_z_test: each count lies in [0, nobs] and each nobs is positive; got 5 of 4\")",
    );
}

#[test]
fn formulas_nest_and_cross_and_a_frame_fits_itself() {
    agrees(
        "formulas",
        &format!(
            r#"{DATA}
    println(f.linear("y ~ x").params.x == learn.linear(f, "y ~ x").params.x)
    println(f.linear("y ~ g / x").features == f.linear("y ~ g + g:x").features)
    println(f.linear("y ~ (x + g)^2").features)
    println(f.linear("y ~ (x + g)**2").features == f.linear("y ~ x * g").features)
    println(ods.frame({{ silhouette: [1.5], split: [2] }}).silhouette.to_list())
    println(f.forest(y: "y", x: ["x"], n_trees: 3, seed: 1).spec.options.n_trees)
    println(attempt(() => f.linear("y ~ (x + g)^0")))
"#
        ),
        "true\ntrue\n[\"x\", \"g_b\", \"x:g_b\"]\ntrue\n[1.5]\n3\n\
         Err(\"learn: in the formula, a power of terms is a whole number of at least 1 (`(a + b)^2`); arithmetic goes inside I(…):\\n    y ~ (x + g)^0\\n                ^\")",
    );
}

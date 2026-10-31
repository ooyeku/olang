//! The data-ergonomics prerequisites (docs/ods.md,
//! L1–L6) on every tier: keyword arguments and defaults for native
//! builtins, methods on native values, the native operator hooks
//! (unary, bitwise, `@`), `len` of a Series or Frame, and a Frame's
//! columns as fields.
//!
//! Each program runs three ways — the interpreter alone (`--no-ovm`),
//! every function compiled at its first call (`--ovm-tier=1`, where the
//! JIT takes what it can), and the default — and all three must print
//! exactly the expected text.

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run(code: &str, flags: &[&str]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "olang_native_ergo_{}_{}",
        std::process::id(),
        code.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("p{}.ol", flags.len()));
    std::fs::write(&path, code).unwrap();
    let out = Command::new(olang())
        .args(flags)
        .arg(&path)
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// The program prints `expected` on every tier.
fn agrees(code: &str, expected: &str) {
    for flags in [&["--no-ovm"][..], &["--ovm-tier=1"][..], &[][..]] {
        let (out, err) = run(code, flags);
        assert_eq!(
            out.trim_end(),
            expected.trim_end(),
            "flags {flags:?}\nstderr: {err}\nprogram:\n{code}"
        );
    }
}

/// The functions named were compiled to the bytecode tier (not refused).
fn promoted(code: &str, names: &[&str]) {
    let (_, err) = run(code, &["-v", "--ovm-tier=1"]);
    for name in names {
        assert!(
            err.contains(&format!("promoted '{name}'")),
            "'{name}' was not promoted:\n{err}"
        );
    }
}

// ── L1: names and defaults ──────────────────────────────────────────
const KWARGS: &str = r#"
let s = ods.series([3.0, 1.0, 2.0, 5.0])
let f = ods.frame([["a", s], ["b", [1, 2, 3, 4]]])
println(ods.rolling(s, window: 2, agg: "max"))
println(ods.rolling(s, agg: "min", window: 2))
println(s |> ods.rolling(2, agg: "sum"))
println(ods.to_list(ods.sort_by(f, "a", descending: true)["b"]))
println(ods.to_list(ods.sort_by(f, by: "a")["b"]))
println(ods.n_rows(ods.head(f)))
println(stats.norm.cdf(0.0))
println(stats.norm.cdf(2.0, std: 2.0) == stats.norm.cdf(1.0))
println(str.contains(plot.line(s, s, title: "t", width: 300), "<svg"))
println(str.contains(plot.hist(s), "<svg"))
fn g(x) = ods.rolling(x, window: 2, agg: "mean")
fn h(x) = ods.sort_by(ods.frame([["v", x]]), "v", descending: true)
println(ods.sum(g(s)))
println(ods.to_list(h(s)["v"]))
println(attempt(() => ods.rolling(s, windw: 2, agg: "mean")))
println(attempt(() => ods.rolling(s, 2, window: 3, agg: "mean")))
println(attempt(() => ods.rolling(s, agg: "mean")))
println(attempt(() => ods.rolling(s, window: 2, window: 2, agg: "mean")))
println(attempt(() => len(s, x: 1)))
println(attempt(() => plot.line(s, s, colour: "red")))
"#;

const KWARGS_OUT: &str = r#"Series[Float; 4] [null, 3.0, 2.0, 5.0]
Series[Float; 4] [null, 1.0, 1.0, 2.0]
Series[Float; 4] [null, 4.0, 3.0, 7.0]
[4, 1, 3, 2]
[2, 3, 1, 4]
4
0.5
true
true
true
7.0
[5.0, 3.0, 2.0, 1.0]
Err("ods.rolling: no parameter named 'windw' — it takes s, window, agg, center, min_periods, exact")
Err("ods.rolling: argument 'window' is given both by position and by name")
Err("ods.rolling: missing argument 'window' (argument 2)")
Err("Duplicate named argument: window")
Err("len takes its arguments by position: named argument 'x' is not accepted by a builtin")
Err("plot.line: no parameter or option named 'colour' — it takes x, y, and the options title, x_label, y_label, width, height, theme, paper, ink, responsive, interactive, colors, vary, scale, font_size, font")"#;

#[test]
fn native_builtins_take_names_and_defaults_on_every_tier() {
    agrees(KWARGS, KWARGS_OUT);
    promoted(KWARGS, &["g", "h"]);
}

#[test]
fn learn_takes_its_options_by_name() {
    let code = r#"
random.seed(3)
let x = stats.norm.sample(200)
let y = x * 2.0 + stats.norm.sample(200, std: 0.1)
let f = ods.frame([["x", x], ["y", y]])
let parts = learn.split(f, test: 0.25, seed: 7)
println(ods.n_rows(parts.test))
let opts = learn.split(f, #{ "test": 0.5 }, seed: 7)
println(ods.n_rows(opts.test))
let fit = learn.linear(parts.train, y: "y")
println(fit.params.x > 1.9)
println(attempt(() => learn.split(f, tst: 0.2)))
fn fitted(t) = learn.ridge(t, y: "y", alpha: 0.5)
println(fitted(parts.train).params.x > 1.9)
"#;
    agrees(
        code,
        "50\n100\ntrue\nErr(\"learn.split: no parameter or option named 'tst' — it takes data, and the options test, stratify, shuffle, seed\")\ntrue",
    );
}

// ── L2: methods ─────────────────────────────────────────────────────
const METHODS: &str = r#"
let s = ods.series([3.0, (), 2.0, 5.0])
let f = ods.frame([["a", s], ["b", [1, 2, 3, 4]]])
println(s.fill_null(0.0).sum())
println(s.rolling(2, "mean"))
println(s.rolling(agg: "max", window: 2))
println(f.sort_by("b", descending: true).head(2).n_rows())
println(f.a.null_count())
println(f["b"].cast("Float").mean())
fn chain(x) = x.fill_null(1.0).sort().to_list()
fn top(t) = t.sort_by("b", descending: true).head(1).b.to_list()
println(chain(s))
println(top(f))
let d = unwrap(dates.date(2026, 10, 20))
println(d.weekday())
println(unwrap(d.add_days(3)).day())
trait Loud { fn sum(self) }
impl Loud for Series { fn sum(self) = "the impl" }
println(s.sum())
println(attempt(() => s.nope()))
println(attempt(() => f.read_csv()))
"#;

const METHODS_OUT: &str = r#"10.0
Series[Float; 4] [null, 3.0, 2.0, 3.5]
Series[Float; 4] [null, 3.0, 2.0, 5.0]
2
1
2.5
[1.0, 2.0, 3.0, 5.0]
[4]
Ok(2)
Ok(23)
the impl
Err("a Series has no method 'nope' (and no `impl` gives it one)")
Err("a Frame has no method 'read_csv' (and no `impl` gives it one)")"#;

#[test]
fn native_values_have_their_modules_functions_as_methods() {
    agrees(METHODS, METHODS_OUT);
    promoted(METHODS, &["chain", "top"]);
}

#[test]
fn a_fit_takes_the_model_methods() {
    let code = r#"
let f = ods.frame([["x", [1.0, 2.0, 3.0, 4.0, 5.0]], ["y", [2.1, 3.9, 6.2, 7.8, 10.1]]])
let fit = learn.linear(f, "y")
let p = fit.predict(f)
println(p == learn.predict(fit, f))
println(fit.model.predict(f) == p)
println(ods.n_rows(fit.summary(level: 0.9)) > 0)
fn score(m, t) = learn.rmse(t["y"], m.predict(t)) < 0.5
println(score(fit, f))
println(attempt(() => fit.nope()))
"#;
    agrees(
        code,
        "true\ntrue\ntrue\ntrue\nErr(\"Map has no key or method 'nope': `.nope` reads `()`, which cannot be called\")",
    );
}

// ── L3/L4: the operator hooks ───────────────────────────────────────
#[test]
fn unary_bitwise_and_matmul_reach_the_native_hook() {
    let code = r#"
let p = __ods_test.probe(6)
let q = __ods_test.probe(3)
println(__ods_test.probe_tag(-p))
println(__ods_test.probe_tag(!p))
println(__ods_test.probe_tag(p & q))
println(__ods_test.probe_tag(p | q))
println(__ods_test.probe_tag(p ^ q))
println(p @ q)
fn ops(a, b) = [__ods_test.probe_tag(-a), __ods_test.probe_tag(a & b), a @ b]
println(ops(p, q))
let s = ods.series([1.0, 2.0, 3.0])
println(s @ s)
println(s @ s * 2.0)
println(attempt(() => 1 @ 2))
println(attempt(() => s @ 2))
"#;
    agrees(
        code,
        "-6\n7\n2\n7\n5\n18\n[-6, 2, 18]\n14.0\n28.0\nErr(\"`@` needs a Matrix or Series on each side, got Int and Int\")\nErr(\"`@` needs a Matrix or Series on each side, got Series and Int\")",
    );
}

#[test]
fn a_line_starting_with_at_is_still_a_macro_or_decorator() {
    let code = r#"
meta fn twice(e) = `(${e}) + (${e})`
meta fn say(e) = `println(${e})`
let x = 2
@say(x)
println(@twice(x))
"#;
    agrees(code, "2\n4");
}

// ── L5/L6: len, fields ──────────────────────────────────────────────
#[test]
fn len_and_columns_as_fields() {
    let code = r#"
let s = ods.series([1.0, 2.0, 3.0])
let f = ods.frame([["price", s], ["sum", [1, 2, 3]]])
println(len(s))
println(len(s * 2.0 + 1.0))
println(len(f))
println(f.price)
println(f["sum"])
println(attempt(() => f.head))
println(attempt(() => f.missing))
fn col(t) = t.price
println(col(f))
"#;
    agrees(
        code,
        "3\n3\n3\nSeries[Float; 3] [1.0, 2.0, 3.0]\nSeries[Int; 3] [1, 2, 3]\nErr(\"'head' is a method of a Frame (ods.head): call it, `value.head(…)`\")\nErr(\"a Frame has no column 'missing' — its columns are price, sum\")\nSeries[Float; 3] [1.0, 2.0, 3.0]",
    );
}

// ── D0 × D1: the column functions through names, methods and operators ───
const COLUMNS: &str = r#"
use ods { col }
let s = ods.series([1.0, (), -3.0, 4.0])
let f = ods.frame([["g", ["a", "a", "b", "b"]], ["x", [1.0, 2.0, 4.0, 8.0]], ["y", [2.0, (), 1.0, 3.0]]])
fn chain(v) = v.fill_null("ffill").clip(lo: 0.0, hi: 3.0).round(digits: 0).to_list()
fn masks(v) = {
    let pos = v > 0.0
    let small = v < 3.0
    [(pos & small).to_list(), (pos | small).to_list(), (pos ^ small).to_list(), (!pos).to_list(), (pos & small).sum()]
}
fn neg(v) = (-v).to_list()
fn dotted(a, b) = a @ b
fn grouped(t) = t.with({
    m: col("x").mean().over("g"),
    d: t.x - t.x.mean(),
    z: col("y").fill_null(0.0).sqrt().alias("ignored"),
    w: ods.where(col("x") > 2.0, col("x"), -col("x")),
})
println(chain(s))
println(masks(s))
println(neg(s))
println(dotted(f.x, f.x))
println(len(s), len(f))
let g = grouped(f)
println(g.m.to_list())
println(g.d.to_list())
println(g.z.to_list())
println(g.w.to_list())
println(s.between([0.0, 4.0], closed: "left").to_list())
println(f.x.diff(lag: 2).to_list(), f.x.pct_change().to_list())
println(f.x.sort(descending: true).to_list(), f.x.argsort(descending: true).to_list())
println(f.x.searchsorted(4.0, side: "right"))
println(ods.arange(0, 6, step: 2).to_list(), ods.full(2, value: 7).to_list())
println(f.x.repeat(times: 2).len(), f.x.tile(n: 2).len())
println(ods.select([f.x > 4.0, f.x > 1.0], ["big", "mid"], default: "small").to_list())
println(f.y.recode([[2.0, 20.0]], default: 0.0).to_list())
println(f.y.interpolate().to_list(), s.coalesce(0.0).to_list())
println(f.x.cumprod().to_list(), f.x.argmax(), f.g.mode(), (f.x > 0.0).all(), f.y.count())
println(f.filter(col("x") > 1.5).n_rows(), f.select(["g", col("x") * 2.0]).columns())
println(f.sort_by(col("x"), descending: true).x.to_list())
println(attempt(() => s.clip(low: 0.0)))
"#;

const COLUMNS_OUT: &str = r#"
[1.0, 1.0, 0.0, 3.0]
[[true, (), false, false], [true, (), true, true], [false, (), true, true], [false, (), true, false], 1]
[-1.0, (), 3.0, -4.0]
85.0
4 4
[1.5, 1.5, 6.0, 6.0]
[-2.75, -1.75, 0.25, 4.25]
[1.4142135623730951, 0.0, 1.0, 1.7320508075688772]
[-1.0, -2.0, 4.0, 8.0]
[true, (), false, false]
[(), (), 3.0, 6.0] [(), 1.0, 1.0, 1.0]
[8.0, 4.0, 2.0, 1.0] [3, 2, 1, 0]
3
[0, 2, 4] [7, 7]
8 8
["small", "mid", "mid", "big"]
[20.0, (), 0.0, 0.0]
[2.0, 1.5, 1.0, 3.0] [1.0, 0.0, -3.0, 4.0]
[1.0, 2.0, 8.0, 64.0] 3 a true 3
3 ["g", "x"]
[8.0, 4.0, 2.0, 1.0]
Err("ods.clip: no parameter named 'low' — it takes s, lo, hi")
"#;

#[test]
fn column_functions_take_names_methods_and_operators_on_every_tier() {
    agrees(COLUMNS, COLUMNS_OUT.trim_start());
    promoted(COLUMNS, &["chain", "masks", "neg", "dotted", "grouped"]);
}

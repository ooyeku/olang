//! Frames: shape, group, join, time (docs/design/data-ergonomics.md §4)
//! on every tier: the GroupBy value and its verbs, aggregation
//! expressions and `over`, `apply` calling olang once per group, keyed
//! lookup and slicing, crosstabs and runs, joins on several keys with
//! `how`/`suffix`/`validate`, `resample`, and the rolling options.
//!
//! Each program runs three ways — the interpreter alone (`--no-ovm`),
//! every function compiled at its first call (`--ovm-tier=1`), and the
//! default — and all three must print exactly the expected text.

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run_env(code: &str, flags: &[&str], env: &[(&str, &str)]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "olang_frames_ergo_{}_{}",
        std::process::id(),
        code.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("p{}.ol", flags.len()));
    std::fs::write(&path, code).unwrap();
    let mut cmd = Command::new(olang());
    cmd.args(flags).arg(&path);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// The program prints `expected` on every tier.
fn agrees(code: &str, expected: &str) {
    for flags in [&["--no-ovm"][..], &["--ovm-tier=1"][..], &[][..]] {
        let (out, err) = run_env(code, flags, &[]);
        assert_eq!(
            out.trim_end(),
            expected.trim_start_matches('\n').trim_end(),
            "flags {flags:?}\nstderr: {err}\nprogram:\n{code}"
        );
    }
}

/// The functions named were compiled to the bytecode tier (not refused).
fn promoted(code: &str, names: &[&str]) {
    let (_, err) = run_env(code, &["-v", "--ovm-tier=1"], &[]);
    for name in names {
        assert!(
            err.contains(&format!("promoted '{name}'")),
            "'{name}' was not promoted:\n{err}"
        );
    }
}

const GROUPS: &str = r#"
use ods { col }
let f = ods.frame({ region: ["n", "s", "n", "s", "n"], month: [1, 1, 2, 2, 1], amount: [10.0, 20.0, 30.0, 40.0, 50.0], qty: [1, 2, 3, 4, 5], big: [false, false, true, true, true] })
let by = f |> ods.group_by("region")
println(typeof(by))
println(len(by))
let a = by.agg({ total: col("amount").sum(), orders: ods.count(), med: col("amount").median(), q9: col("amount").quantile(0.9), share: col("amount").sum() / col("qty").sum(), top: col("amount").argmax(), bigs: col("big").sum(), any_big: col("big").any(), first_m: col("month").first(), n_m: col("month").n_unique(), sd: col("amount").std() })
println(ods.columns(a))
for name in ["total", "orders", "med", "q9", "share", "top", "bigs", "any_big", "first_m", "n_m"] {
    println(`${name}: ${ods.to_list(a[name])}`)
}
println(ods.to_list(by.mean()["amount"]))
println(ods.columns(by.mean()))
println(ods.to_list(by.size()["size"]))
println(ods.to_list(by.transform(col("amount").mean())))
println(ods.to_list(ods.quantile(by, 0.5)["qty"]))
println(ods.to_list(by.count()["amount"]))
let two = f.group_by(["region", "month"])
println(ods.to_list(two.agg({ s: col("amount").sum() })["s"]))
println(ods.to_list(ods.group_by(f, "region", { s: col("amount").sum() })["s"]))
println(ods.to_list(ods.group_by(f, "region", [["s", "sum", "amount"], ["n", "count"]])["n"]))
println(ods.to_list(by.agg([["s", "sum", "amount"]])["s"]))
fn per_group(t) = by.apply((g) => { n: ods.n_rows(g), top: ods.max(g["amount"]) + t })
let ap = per_group(0.5)
println(ods.columns(ap))
println(ods.to_list(ap["top"]))
println(ods.to_list(by.apply((g) => ods.head(g, 1))["amount"]))
println(ods.to_list(by.apply((g) => ods.n_rows(g))["value"]))
println(len(by.apply((g) => [1, 2])))
for (k, g) in by.groups() {
    println(`${k}: ${ods.n_rows(g)}`)
}
println(ods.to_list(ods.with(f, { d: col("amount") - col("amount").mean().over("region") })["d"]))
println(ods.to_list(ods.with(f, { n: ods.count().over(["region", "month"]) })["n"]))
println(ods.to_list(ods.with(f, { r: (col("amount").sum() / col("qty").sum()).over("region") })["r"]))
println(attempt(() => by.agg({ x: col("amount") })))
println(attempt(() => ods.agg(f, { x: col("amount").sum() })))
"#;

#[test]
fn a_group_by_is_a_value_with_verbs() {
    agrees(
        GROUPS,
        r#"
GroupBy
2
["region", "total", "orders", "med", "q9", "share", "top", "bigs", "any_big", "first_m", "n_m", "sd"]
total: [90.0, 60.0]
orders: [3, 2]
med: [30.0, 30.0]
q9: [46.0, 38.0]
share: [10.0, 10.0]
top: [4, 3]
bigs: [2, 1]
any_big: [true, true]
first_m: [1, 1]
n_m: [2, 2]
[30.0, 30.0]
["region", "month", "amount", "qty"]
[3, 2]
[30.0, 30.0, 30.0, 30.0, 30.0]
[3.0, 3.0]
[3, 2]
[60.0, 20.0, 30.0, 40.0]
[90.0, 60.0]
[3, 2]
[90.0, 60.0]
["region", "n", "top"]
[50.5, 40.5]
[10.0, 20.0]
[3, 2]
2
n: 3
s: 2
[-20.0, -10.0, 0.0, 10.0, 20.0]
[2, 1, 1, 1, 2]
[10.0, 10.0, 10.0, 10.0, 10.0]
Err("ods.agg: 'x': col(\"amount\") is not a key, so it has many values in a group; reduce it, like col(\"amount\").sum() or col(\"amount\").first()")
Err("ods.agg: argument 1 must be a GroupBy (from ods.group_by(f, keys)), got Frame")
"#,
    );
}

#[test]
fn apply_and_agg_compile() {
    promoted(
        r#"
use ods { col }
let f = ods.frame({ k: ["a", "b", "a"], v: [1.0, 2.0, 3.0] })
fn spans(t) = t.group_by("k").apply((g) => ods.max(g["v"]) - ods.min(g["v"]))
fn sums(t) = t.group_by("k").agg({ s: col("v").sum() })
println(spans(f))
println(sums(f))
"#,
        &["spans", "sums"],
    );
}

const SHAPE: &str = r#"
let f = ods.frame({ id: [3, 1, 2], name: ["c", "a", "b"], score: [0.5, 0.25, 0.75] })
println(ods.columns(f[["score", "id"]]))
println(ods.to_list(ods.slice(f, 1, 5)["id"]))
println(ods.to_list(ods.slice(f, -2, 1)["id"]))
println(ods.to_list(f["score"].slice(0, 2)))
println(ods.row(f, 0))
println(ods.row(f, -1).name)
println(ods.lookup(f, "id", 2, "name"))
println(ods.lookup(f, "id", 9, "name"))
println(ods.lookup(f, "score", 0.25, "id"))
println(ods.to_list(f.lookup("name", ["a", "z", "c"], "score")))
println(ods.columns(ods.rename(f, { score: "points" })))
println(ods.to_list(ods.sort_by(ods.frame({ g: [1, 2, 1, 2], v: [1, 2, 3, 4] }), ["g", "v"], descending: [false, true])["v"]))
println(attempt(() => f[0]))
println(attempt(() => ods.row(f, 3)))
println(attempt(() => f[["id", "nope"]]))
"#;

#[test]
fn shape_and_keyed_access() {
    agrees(
        SHAPE,
        r#"
["score", "id"]
[1, 2]
[1]
[0.5, 0.25]
#{"id": 3, "name": "c", "score": 0.5}
b
b
()
1
[0.25, (), 0.5]
["id", "name", "points"]
[3, 1, 4, 2]
Err("a Frame is indexed by column, not by row position. For rows, use ods.row(f, i), ods.slice(f, start, n) or ods.take(f, indices)")
Err("ods.row: row 3 is out of range for a Frame of 3 rows")
Err("no column 'nope' in this Frame. It has: id, name, score")
"#,
    );
}

const TABLES: &str = r#"
use ods { col }
let a = ods.series([1, 0, 1, 1, ()])
let b = ods.series(["y", "x", "x", "y", "x"])
let v = ods.series([1.0, 2.0, 3.0, 4.0, 5.0])
let t = ods.crosstab(a, b, name: "a")
println(ods.columns(t))
println(ods.to_list(t["y"]))
println(ods.to_list(ods.crosstab(a, b, normalize: "columns")["x"]))
println(ods.to_list(ods.crosstab(a, b, values: v, agg: "sum")["y"]))
println(ods.to_list(ods.crosstab(a, b, normalize: true)["y"]))
let m = ods.series([true, true, false, (), true, false, true])
let r = ods.runs(m)
println(ods.to_list(r["start"]))
println(ods.to_list(r["end"]))
println(ods.to_list(r["length"]))
let w = ods.runs(ods.series(["a", "a", "b", "a"]))
println(ods.to_list(w["value"]))
println(ods.to_list(ods.run_id(m)))
println(ods.to_list(ods.run_id([ods.series([1, 1, 1, 2]), ods.series([0, 0, 1, 1])])))
let f = ods.frame({ k: ["a", "a", "b", "b", "b"], x: [1, 1, 2, 2, 1] })
println(ods.to_list(ods.with(f, { run: ods.run_id(col("x")) })["run"]))
let p = ods.pivot(ods.frame({ r: ["e", "e", "w"], q: ["Q1", "Q2", "Q1"], v: [1.0, 2.0, 3.0] }), "r", "q", "v", "sum", fill: 0.0)
println(ods.to_list(p["Q2"]))
println(attempt(() => ods.crosstab(a, b, agg: "sum")))
"#;

#[test]
fn crosstabs_runs_and_pivot_fill() {
    agrees(
        TABLES,
        r#"
["a", "x", "y"]
[0, 2]
[0.5, 0.5]
[(), 5.0]
[0.0, 0.5]
[0, 4, 6]
[1, 4, 6]
[2, 1, 1]
["a", "b", "a"]
[0, 0, 1, 2, 3, 4, 5]
[0, 0, 1, 2]
[0, 0, 1, 1, 2]
[2.0, 0.0]
Err("ods.crosstab: agg \"sum\" needs values to aggregate")
"#,
    );
}

const JOINS: &str = r#"
let sales = ods.frame({ region: ["n", "s", "n", ()], week: [1, 1, 2, 2], units: [5, 7, 9, 1] })
let promos = ods.frame({ region: ["n", "s", "e"], week: [2, 1, 1], discount: [0.1, 0.2, 0.3], units: [0, 0, 0] })
let j = ods.join(sales, promos, ["region", "week"], how: "left")
println(ods.columns(j))
println(ods.to_list(j["discount"]))
println(ods.n_rows(ods.join(sales, promos, ["region", "week"])))
println(ods.to_list(ods.join(sales, promos, ["region", "week"], how: "semi")["units"]))
println(ods.to_list(ods.join(sales, promos, ["region", "week"], how: "anti")["units"]))
let full = ods.join(sales, promos, ["region", "week"], how: "full")
println(ods.to_list(full["region"]))
println(ods.columns(ods.join_left(sales, promos, ["region", "week"], suffix: "_promo")))
let other = ods.frame({ r: ["n", "s"], w: [2, 1], d: [0.5, 0.6] })
println(ods.to_list(ods.join(sales, other, ["region", "week"], ["r", "w"], how: "left")["d"]))
println(ods.n_rows(ods.join(sales, promos, "region", validate: "m:m")))
println(attempt(() => ods.join(sales, ods.concat([promos, promos]), ["region", "week"], validate: "many_to_one")))
println(attempt(() => ods.join(ods.concat([sales, sales]), promos, ["region", "week"], validate: "one_to_one")))
println(attempt(() => ods.join(sales, promos, "region", how: "outer!")))
println(attempt(() => ods.join(sales, promos, ["region", "nope"])))
let readings = ods.frame({ t: ["2024-01-01T00:00", "2024-01-01T06:00", "2024-01-01T12:00"], site: ["a", "a", "b"], temp: [1.0, 2.0, 3.0] })
let ev = ods.frame({ t: ["2024-01-01T07:00", "2024-01-01T05:00", "2024-01-01T13:00"], site: ["a", "a", "a"] })
println(ods.to_list(ods.asof_join(ev, readings, "t", by: "site")["temp"]))
println(ods.to_list(ods.asof_join(ev, readings, "t", direction: "forward")["temp"]))
println(ods.to_list(ods.asof_join(ods.frame({ t: [5, 9] }), ods.frame({ t: [4, 10], v: [1, 2] }), "t", direction: "nearest", tolerance: 1)["v"]))
println(attempt(() => ods.asof_join(ev, readings, "t", direction: "sideways")))
"#;

#[test]
fn joins_on_several_keys_with_options() {
    agrees(
        JOINS,
        r#"
["region", "week", "units", "discount", "units_right"]
[(), 0.2, 0.1, ()]
2
[7, 9]
[5, 1]
["n", "s", "n", (), "e"]
["region", "week", "units", "discount", "units_promo"]
[(), 0.6, 0.5, ()]
3
Err("ods.join: validate \"many_to_one\" wants each key once on the right, but key (\"n\", 2) is on 2 rows of it — this join would repeat left rows")
Err("ods.join: validate \"one_to_one\" wants each key once on the left, but key (\"n\", 1) is on rows 0 and 4 of it")
Err("ods.join: how is \"inner\", \"left\", \"full\", \"semi\" or \"anti\", got \"outer!\"")
Err("ods.join: no column 'nope' in the left Frame. It has: region, week, units")
[2.0, 1.0, 2.0]
[3.0, 2.0, ()]
[1, 2]
Err("ods.asof_join: direction is \"backward\", \"forward\" or \"nearest\", got \"sideways\"")
"#,
    );
}

const TIME: &str = r#"
use ods { col }
let t = ods.frame({
    ts: ["2024-01-01T01:00", "2024-01-01T13:30", "2024-01-02T05:00", "2024-02-03T00:00", ()],
    v: [1.0, 2.0, 3.0, 4.0, 5.0],
    site: ["a", "b", "a", "a", "a"],
})
let d = ods.resample(t, "ts", "1d", { total: col("v").sum(), n: ods.count() })
println(ods.to_list(d["ts"]))
println(ods.to_list(d["total"]))
println(ods.to_list(ods.resample(t, "ts", "1h", { total: col("v").sum() })["ts"]))
println(ods.to_list(ods.resample(t, "ts", "1mo", { total: col("v").sum() }, by: "site")["site"]))
println(ods.to_list(ods.resample(t, "ts", "1w", { total: col("v").sum() })["ts"]))
println(ods.to_list(ods.resample(t, "ts", "1y", [["total", "sum", "v"]])["total"]))
println(attempt(() => ods.resample(t, "ts", "1x", { n: ods.count() })))
println(attempt(() => ods.resample(ods.frame({ ts: ["soon"] }), "ts", "1d", { n: ods.count() })))
let s = ods.series([1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
println(ods.to_list(ods.rolling(s, 4, "sum", center: true)))
println(ods.to_list(s.rolling(4, "sum", center: true, min_periods: 1)))
println(ods.to_list(s.rolling(3, "median", center: true)))
println(ods.to_list(s.rolling(3, "std", exact: true)) == ods.to_list(s.rolling(3, "std")))
println(ods.to_list(ods.rolling(ods.series([1, (), 3, 4]), 2, "sum")))
println(ods.to_list(ods.rolling(ods.series([1, (), 3, 4]), 2, "max", min_periods: 1)))
println(ods.to_list(ods.with(ods.frame({ x: [1.0, 2.0, 3.0] }), { m: col("x").rolling(2, "mean", min_periods: 1) })["m"]))
println(ods.to_list(ods.series(["2024-03-17 13:45:10", "2024-03-17 09:05:30"]).floor("1h")))
println(ods.to_list(ods.with(t, { w: col("ts").floor("1w") })["w"]))
println(ods.to_list(ods.floor(ods.series([1.5, -0.5]))))
"#;

#[test]
fn resample_and_rolling_options() {
    agrees(
        TIME,
        r#"
["2024-01-01", "2024-01-02", "2024-02-03", ()]
[3.0, 3.0, 4.0, 5.0]
["2024-01-01T01:00", "2024-01-01T13:00", "2024-01-02T05:00", "2024-02-03T00:00", ()]
["a", "a", "a", "b"]
["2024-01-01", "2024-01-29", ()]
[10.0, 5.0]
Err("ods.resample: '1x' is not an interval: write a count and a unit, like \"1d\", \"6h\", \"15m\", \"30s\", \"1w\", \"1mo\", \"1q\" or \"1y\"")
Err("ods.resample: 'ts': \"soon\" (row 0) is not an ISO date or date-time like 2024-03-01 or 2024-03-01T13:45")
[(), (), 10.0, 14.0, 18.0, ()]
[3.0, 6.0, 10.0, 14.0, 18.0, 15.0]
[(), 2.0, 3.0, 4.0, 5.0, ()]
true
[(), 1, 3, 7]
[1, 1, 3, 4]
[1.0, 1.5, 2.5]
["2024-03-17 13:00:00", "2024-03-17 09:00:00"]
["2024-01-01", "2024-01-01", "2024-01-01", "2024-01-29", ()]
[1.0, -1.0]
"#,
    );
}

/// A large frame groups, joins and rolls the same on one thread as on
/// all of them.
#[test]
fn large_frames_agree_on_one_thread_and_many() {
    let code = r#"
use ods { col }
let n = 600000
let i = ods.arange(n)
let f = ods.frame({ k: (i * 7919) % 1000, k2: (i * 31) % 50, x: ods.sin(ods.cast(i, "Float") * 0.001) * 100.0 + 500.0 })
let a = f.group_by("k").agg({ s: col("x").sum(), m: col("x").median(), q: col("x").quantile(0.25), n: ods.count(), top: col("x").argmax() })
println(ods.sum(a["s"]))
println(ods.sum(a["m"]))
println(ods.sum(a["q"]))
println(ods.sum(a["top"]))
println(ods.sum(f.group_by("k").transform(col("x").mean())))
let right = ods.frame({ k: ods.repeat(ods.arange(1000), 50), k2: ods.tile(ods.arange(50), 1000), w: ods.arange(50000) })
let j = ods.join(f, right, ["k", "k2"], how: "left", validate: "many_to_one")
println(ods.sum(j["w"]))
println(ods.sum(f.group_by("k").apply((g) => ods.n_rows(g))["value"]))
println(ods.sum(ods.rolling(f["x"], 365, "std")))
println(ods.sum(ods.crosstab(f["k2"], f["k"] % 20)["3"]))
"#;
    let (many, e1) = run_env(code, &[], &[]);
    let (again, _) = run_env(code, &[], &[]);
    let (one, e2) = run_env(code, &[], &[("OLANG_PARALLELISM", "1")]);
    assert!(many.lines().count() == 9, "{e1}");
    // Run to run, every answer is the same.
    assert_eq!(many, again);
    // On one thread too, but for a grouped float sum (and the mean made
    // from it), whose additions a parallel scatter reorders.
    for (k, (a, b)) in many.lines().zip(one.lines()).enumerate() {
        if k != 0 && k != 4 {
            assert_eq!(a, b, "line {k}\n{e1}\n{e2}");
        }
    }
}

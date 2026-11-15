//! Dates, datetimes, categories and text columns (data ergonomics D2)
//! on every tier: the interpreter alone (`--no-ovm`), every function
//! compiled at its first call (`--ovm-tier=1`, the JIT taking what it
//! can), and the default. Each column kernel is checked against the
//! scalar language (`dates.*`, `str.*`) element by element, and nulls,
//! round trips, joins, groups and the error messages are pinned.

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run(code: &str, flags: &[&str], tag: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("olang_dtypes_{}_{}", std::process::id(), tag));
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

/// The program prints `expected` on every tier. The work happens inside
/// `fn main_()`, so the compiled tier compiles it.
fn agrees(tag: &str, body: &str, expected: &str) {
    let code =
        format!("fn every(xs) = all(xs, (x) => x == true)\nfn main_() = {{\n{body}\n}}\nmain_()\n");
    for flags in [&["--no-ovm"][..], &["--ovm-tier=1"][..], &[][..]] {
        let (out, err) = run(&code, flags, tag);
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
    for flags in [&["--no-ovm"][..], &["--ovm-tier=1"][..], &[][..]] {
        let (_, err) = run(&code, flags, tag);
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

const DAYS: &str = r#"
    // every day from 1999-12-01 for four years and a bit, and a null
    let ks = range(10926, 12500)
    let ds = map(ks, (k) => unwrap(dates.from_epoch_day(k)))
    let col = ods.series(ds + [()])
"#;

#[test]
fn date_parts_are_the_scalar_functions_over_a_column() {
    agrees(
        "parts",
        &format!(
            "{DAYS}{}",
            r#"
    fn same(s, f) = s.to_list() == map(ds, f) + [()]
    println(every([
        same(col.year(), (d) => unwrap(dates.year(d))),
        same(col.month(), (d) => unwrap(dates.month(d))),
        same(col.day(), (d) => unwrap(dates.day(d))),
        same(col.weekday(), (d) => unwrap(dates.iso_weekday(d))),
        same(col.quarter(), (d) => (unwrap(dates.month(d)) - 1) / 3 + 1),
        same(col.is_weekend(), (d) => unwrap(dates.iso_weekday(d)) >= 6),
        same(col.floor("1mo"), (d) => unwrap(dates.start_of_month(d))),
        same(col.floor("1w"), (d) => unwrap(dates.start_of_week(d))),
        same(col.add_months(13), (d) => unwrap(dates.add_months(d, 13))),
        same(col + 40, (d) => unwrap(dates.add_days(d, 40))),
        same(col.add_days(-3), (d) => unwrap(dates.add_days(d, -3))),
        same(col.day_of_year(), (d) => unwrap(dates.diff_days(d, unwrap(dates.date(unwrap(dates.year(d)), 1, 1)))) + 1),
        same(col.date(), (d) => d),
        same(col - col.shift(1), (d) => if d == ds[0] => () else => 1),
    ]))
    // ISO weeks at the edges of years
    let edges = ods.series(map(["2020-12-31", "2021-01-03", "2021-01-04", "2024-12-30"], (t) => unwrap(dates.parse(t))))
    println(edges.week().to_list())
    println(edges.day_name().to_list())
    println(edges.month_name().levels().to_list()[0])
"#
        ),
        "true\n[53, 53, 1, 1]\n[\"Thursday\", \"Sunday\", \"Monday\", \"Monday\"]\nJanuary",
    );
}

#[test]
fn datetimes_carry_their_time_of_day() {
    agrees(
        "datetimes",
        r#"
    let texts = ["2024-03-01T13:45:07", "2024-03-02 00:10", "1969-12-31T23:59:59.25", "2024-03-03T08:00:00+02:00"]
    let ts = ods.series(texts + [()]).to_datetime()
    println(ts.to_list())
    println(ts.hour().to_list())
    println(ts.minute().to_list())
    println(ts.second().to_list())
    println(ts.date().to_list())
    println(ts.floor("15m").to_list()[0])
    println(ts.floor("1d").to_list()[2])
    println(ts.add_days(1).to_list()[0])
    println((ts - ts.shift(1)).to_list())
    println(ods.schema(ods.frame([["t", ts]]))["dtype"].to_list())
    println(every([
        ts.hour().to_list()[0] == unwrap(dates.hour("2024-03-01T13:45:07")),
        (ts > "2024-03-01").to_list() == [true, true, false, true, ()],
        ts.max() == "2024-03-03T06:00:00",
    ]))
"#,
        "[\"2024-03-01T13:45:07\", \"2024-03-02T00:10:00\", \"1969-12-31T23:59:59.25\", \"2024-03-03T06:00:00\", ()]\n\
         [13, 0, 23, 6, ()]\n[45, 10, 59, 0, ()]\n[7, 0, 59, 0, ()]\n\
         [2024-03-01, 2024-03-02, 1969-12-31, 2024-03-03, ()]\n\
         2024-03-01T13:45:00\n1969-12-31T00:00:00\n2024-03-02T13:45:07\n\
         [(), 37493.0, -1709338200.75, 1709445600.75, ()]\n[\"Datetime\"]\ntrue",
    );
}

#[test]
fn dates_compare_with_dates_and_iso_text_and_filter_frames() {
    agrees(
        "compare",
        &format!(
            "{DAYS}{}",
            r#"
    let cut = unwrap(dates.date(2001, 6, 1))
    let a = (col >= cut).to_list()
    let b = (col >= "2001-06-01").to_list()
    let want = map(ds, (d) => d >= cut) + [()]
    let f = ods.frame([["d", col], ["k", ods.arange(0, len(col))]])
    let g = f |> ods.filter(ods.col("d").between("2001-01-01", "2001-01-31"))
    println(every([a == want, b == want, ods.n_rows(g) == 31,
        ods.is_in(col, ["2000-02-29", cut]).sum() == 2,
        col.min() == ds[0], col.max() == ds[len(ds) - 1],
        col.sort(descending: true).to_list()[0] == ds[len(ds) - 1],
        col.argmax() == len(ds) - 1]))
"#
        ),
        "true",
    );
}

#[test]
fn date_columns_group_join_and_count() {
    agrees(
        "group",
        r#"
    let f = ods.read_csv("ts,v\n2024-01-01T10:00,1\n2024-01-01T11:00,2\n2024-01-02T09:00,4\n,8\n", dates: ["ts"])
    let by = f |> ods.with({ day: ods.col("ts").date() }) |> ods.group_by("day", [["v", "sum", "v"], ["first", "min", "ts"]])
    println(by)
    let cal = ods.frame([["day", ods.series(["2024-01-01", "2024-01-02"]).to_date()], ["holiday", [true, false]]])
    let j = f |> ods.with({ day: ods.col("ts").date() }) |> ods.join_left(cal, "day")
    println(j["holiday"].to_list())
    println(ods.value_counts(f["ts"].date())["count"].to_list())
    println(ods.unique(f["ts"].date()).to_list())
    println(ods.n_unique(f["ts"]))
"#,
        "Frame[3 x 3]
┌────────────┬─────┬─────────────────────┐
│ day        │   v │ first               │
│ Date       │ Int │ Datetime            │
├────────────┼─────┼─────────────────────┤
│ 2024-01-01 │   3 │ 2024-01-01T10:00:00 │
│ 2024-01-02 │   4 │ 2024-01-02T09:00:00 │
│ —          │   8 │ —                   │
└────────────┴─────┴─────────────────────┘
[true, true, false, ()]
[2, 1, 1]
[2024-01-01, 2024-01-02, ()]
4",
    );
}

#[test]
fn text_methods_are_the_string_functions_over_a_column() {
    agrees(
        "text",
        r##"
    let xs = ["  Old Town ", "riverside", "ÉTÉ à Paris", "a,b,,c", "", "Docklands"]
    let s = ods.series(xs + [()])
    fn same(c, f) = c.to_list() == map(xs, f) + [()]
    println(every([
        same(s.lower(), (x) => str.to_lower(x)),
        same(s.upper(), (x) => str.to_upper(x)),
        same(s.trim(), (x) => str.trim(x)),
        same(s.trim_start(), (x) => str.trim_start(x)),
        same(s.trim_end(), (x) => str.trim_end(x)),
        same(s.replace(" ", "_"), (x) => str.replace(x, " ", "_")),
        same(s.contains("o"), (x) => str.contains(x, "o")),
        same(s.starts_with("r"), (x) => str.starts_with(x, "r")),
        same(s.ends_with("s"), (x) => str.ends_with(x, "s")),
        same(s.len_chars(), (x) => str.length(x)),
        same(s.substring(1, 4), (x) => str.substring(x, 1, 4)),
        same(s.pad_start(12, "."), (x) => str.pad_start(x, 12, ".")),
        same(s.pad_end(12), (x) => str.pad_end(x, 12, " ")),
    ]))
    println(s.contains("OLD", case: false).to_list())
    println(s.split_n(",", 2).to_list())
    println(s.split_n(",", -1).to_list())
    println(ods.series(["a1b22", "c", ()]).replace_re("[0-9]+", "#").to_list())
    println(ods.series([" 12", "x", "3.5", ()]).to_int().to_list())
    println(ods.series([" 12", "x", "3.5", ()]).to_float().to_list())
    println((s.lower() + "|").to_list()[1])
"##,
        "true
[true, false, false, false, false, false, ()]
[(), (), (), \"\", (), (), ()]
[\"  Old Town \", \"riverside\", \"ÉTÉ à Paris\", \"c\", \"\", \"Docklands\", ()]
[\"a#b#\", \"c\", ()]
[12, (), (), ()]
[12.0, (), 3.5, ()]
riverside|",
    );
}

#[test]
fn text_reads_as_dates_with_or_without_a_format() {
    agrees(
        "parse",
        r#"
    let raw = ods.series(["2024-05-21", "2024/05/21", "21/05/2024", "2024-02-30", ()])
    println(raw.to_date().to_list())
    println(raw.replace("/", "-").to_date().to_list())
    println(raw.to_date("%d/%m/%Y").to_list())
    println(ods.series(["Mar 3, 2024 1:05 PM"]).to_datetime("%b %d, %Y %I:%M %p").to_list())
    println(ods.series(["2024-03-01"]).to_date().format("%d %B %Y").to_list())
    println(ods.cast(ods.series(["2024-03-01", "nope"]), "Date").to_list())
    println(ods.cast(ods.series(["2024-03-01"]).to_date(), "String").to_list())
    println(ods.cast(ods.series(["2024-03-01"]).to_date(), "Int").to_list())
    println(ods.cast(ods.series([19783]), "Date").to_list())
    println(ods.cast(ods.series(["2024-03-01"]).to_date(), "Datetime").to_list())
"#,
        "[2024-05-21, (), (), (), ()]
[2024-05-21, 2024-05-21, (), (), ()]
[(), (), 2024-05-21, (), ()]
[\"2024-03-03T13:05:00\"]
[\"01 March 2024\"]
[2024-03-01, ()]
[\"2024-03-01\"]
[19783]
[2024-03-01]
[\"2024-03-01T00:00:00\"]",
    );
}

#[test]
fn categories_keep_their_levels_order_and_codes() {
    agrees(
        "category",
        r#"
    let s = ods.series(["med", "low", (), "high", "low"])
    let c = s.to_category()
    println(c.levels().to_list())
    println(c.codes().to_list())
    let o = s.to_category(levels: ["low", "med", "high"])
    println(o.codes().to_list())
    println(o.sort().to_list())
    println((o < "high").to_list())
    println((o == "low").to_list())
    println(o.max())
    println(s.to_category(levels: ["low"]).to_list())
    println(ods.to_category(ods.series([3, 1, 20])).levels().to_list())
    let f = ods.frame([["g", o], ["v", [1, 2, 3, 4, 5]]])
    println(ods.group_by(f, "g", [["n", "count"], ["v", "sum", "v"]]))
    println(ods.sort_by(f, "g")["v"].to_list())
    println(ods.concat([f, f])["g"].levels().to_list())
    println(o.is_in(["med", "high"]).to_list())
    println(ods.cast(o, "String").to_list())
    println(o.lower().to_list())
    println(o.fill_null("none").levels().to_list())
"#,
        "[\"high\", \"low\", \"med\"]
[2, 1, (), 0, 1]
[1, 0, (), 2, 0]
[\"low\", \"low\", \"med\", \"high\", ()]
[true, true, (), false, true]
[false, true, (), false, true]
high
[(), \"low\", (), (), \"low\"]
[\"1\", \"3\", \"20\"]
Frame[4 x 3]
┌──────────┬─────┬─────┐
│ g        │   n │   v │
│ Category │ Int │ Int │
├──────────┼─────┼─────┤
│ med      │   1 │   1 │
│ low      │   2 │   7 │
│ —        │   1 │   3 │
│ high     │   1 │   4 │
└──────────┴─────┴─────┘
[2, 5, 1, 4, 3]
[\"low\", \"med\", \"high\"]
[true, false, (), true, false]
[\"med\", \"low\", (), \"high\", \"low\"]
[\"med\", \"low\", (), \"high\", \"low\"]
[\"low\", \"med\", \"high\", \"none\"]",
    );
}

#[test]
fn dummies_and_interactions() {
    agrees(
        "dummies",
        r#"
    let f = ods.frame([["contract", ["monthly", "annual", "monthly", "two_year", ()]], ["seg", ["a", "b", "a", "b", "a"]], ["x", [1.0, 2.0, 3.0, 4.0, 5.0]]])
    let d = ods.dummies(f)
    println(ods.columns(d))
    println(d["contract_monthly"].to_list())
    println(ods.columns(ods.dummies(f, ["contract"], drop_first: false, dtype: "Int")))
    println(ods.dummies(f["seg"], prefix: "s", drop_first: false, dtype: "Bool")["s_b"].to_list())
    let o = f["contract"].to_category(levels: ["monthly", "annual", "two_year"])
    println(ods.columns(ods.dummies(o, prefix: "c")))
    let i = ods.interact(f["seg"], ods.series([1, 2, 1, 1, 2]))
    println(i.levels().to_list())
    println(i.codes().to_list())
"#,
        "[\"contract_monthly\", \"contract_two_year\", \"seg_b\", \"x\"]
[1.0, 0.0, 1.0, 0.0, 0.0]
[\"contract_annual\", \"contract_monthly\", \"contract_two_year\", \"seg\", \"x\"]
[false, true, false, true, false]
[\"c_annual\", \"c_two_year\"]
[\"a:1\", \"a:2\", \"b:1\", \"b:2\"]
[0, 3, 0, 2, 1]",
    );
}

#[test]
fn files_keep_dates_and_categories() {
    agrees(
        "files",
        r#"
    let f = ods.frame([
        ["d", ods.series(["2024-01-31", (), "1969-07-20"]).to_date()],
        ["t", ods.series(["2024-01-31T10:30:00.5", "2024-02-01T00:00", ()]).to_datetime()],
        ["c", ods.series(["x", "y", ()]).to_category(levels: ["y", "x"])],
        ["v", [1.5, (), 3.25]],
    ])
    let w = unwrap(ods.write_frame(f, "dt.olc"))
    let back = unwrap(ods.read_frame("dt.olc"))
    println(back == f)
    println(back["c"].levels().to_list())
    println(ods.schema(back)["dtype"].to_list())
    let csv = ods.to_csv(f)
    print(csv)
    let r = ods.read_csv(csv, dates: "auto", dtypes: { c: "Category" })
    println(ods.schema(r)["dtype"].to_list())
    println(r["d"].to_list() == f["d"].to_list() && r["t"].to_list() == f["t"].to_list())
    let j = ods.read_jsonl(ods.to_jsonl(f), dates: ["d", "t"])
    println(ods.schema(unwrap(j))["dtype"].to_list())
    println(ods.to_csv(ods.frame([["x", [1.0 / 3.0, 2.0]]]), float_fmt: "%.3f"))
    println(ods.read_csv("a;b\n1;NA\n007;x\n", sep: ";", nulls: ["NA"], dtypes: { a: "String" })["a"].to_list())
"#,
        "true
[\"y\", \"x\"]
[\"Date\", \"Datetime\", \"Category\", \"Float\"]
d,t,c,v
2024-01-31,2024-01-31T10:30:00.5,x,1.5
,2024-02-01T00:00:00,y,
1969-07-20,,,3.25
[\"Date\", \"Datetime\", \"Category\", \"Float\"]
true
[\"Date\", \"Datetime\", \"String\", \"Float\"]
x
0.333
2.000

[\"1\", \"007\"]",
    );
}

#[test]
fn date_parts_fuse_in_expressions() {
    agrees(
        "exprs",
        r#"
    use ods { col }
    let raw = ods.read_csv("timestamp,demand\n2022-01-01T00:00,10.0\n2022-01-01T01:00,12.0\n2022-01-08T18:00,30.0\n2022-01-09T19:00,20.0\n", dates: ["timestamp"])
    let h = raw |> ods.with({
        hour: col("timestamp").hour(), weekday: col("timestamp").weekday(), day: col("timestamp").date(),
        daytype: ods.select([col("timestamp").weekday() == 7, col("timestamp").weekday() == 6], ["sunday", "saturday"], "workday"),
        evening: col("timestamp").hour().between(17, 20),
        city: ods.lit("  Old Town ").trim(),
    })
    println(ods.select(h, ["hour", "weekday", "day", "daytype", "evening"]))
    println(col("t").hour() + 1)
    println(h["city"].to_list()[0])
"#,
        "Frame[4 x 5]
┌──────┬─────────┬────────────┬──────────┬─────────┐
│ hour │ weekday │ day        │ daytype  │ evening │
│  Int │     Int │ Date       │ String   │ Bool    │
├──────┼─────────┼────────────┼──────────┼─────────┤
│    0 │       6 │ 2022-01-01 │ saturday │ false   │
│    1 │       6 │ 2022-01-01 │ saturday │ false   │
│   18 │       6 │ 2022-01-08 │ saturday │ true    │
│   19 │       7 │ 2022-01-09 │ sunday   │ true    │
└──────┴─────────┴────────────┴──────────┴─────────┘
(hour(col(\"t\")) + 1)
Old Town",
    );
}

#[test]
fn misuse_is_named() {
    raises(
        "e1",
        r#"ods.series(["2024"]).year()"#,
        "ods.year: needs a Date or Datetime column",
    );
    raises(
        "e2",
        r#"ods.series(["2024-01-01"]).to_date().hour()"#,
        "hour: a Date has no time of day",
    );
    raises(
        "e3",
        r#"let d = ods.series(["2024-01-01"]).to_date()
    d + d"#,
        "cannot apply addition to Series[Date] and Series[Date]",
    );
    raises(
        "e4",
        r#"ods.series([1]).lower()"#,
        "ods.lower: needs a String Series, got a Int Series",
    );
    raises(
        "e5",
        r#"ods.series(["a"]).to_category() < "zz""#,
        "\"zz\" is not a level of this Category",
    );
    raises(
        "e6",
        r#"ods.dummies(ods.series(["a"]), dtype: "Date")"#,
        "ods.dummies: dtype is \"Float\", \"Int\" or \"Bool\"",
    );
    raises(
        "e7",
        r#"ods.read_csv("d\n2024/01/02\n", dates: ["d"])"#,
        "column 'd' has \"2024/01/02\"",
    );
    raises(
        "e8",
        r#"ods.series(["2024-01-01"]).to_date().floor("1h")"#,
        "floor: a Date cannot be cut finer than a day",
    );
    raises(
        "e9",
        r#"ods.series(["2024-01-01"]).to_date() > "soon""#,
        "\"soon\" is not an ISO date",
    );
    raises(
        "e10",
        r#"ods.series(["x"]).to_date("%Q")"#,
        "ods.to_date: the format has %Q",
    );
}

#[test]
fn models_encode_a_category_by_its_levels() {
    agrees(
        "learn",
        r#"
    let f = ods.frame([["size", ods.series(["s", "m", "l", "m", "s", "l"]).to_category(levels: ["s", "m", "l"])], ["y", [1.0, 2.1, 3.0, 1.9, 1.1, 3.2]]])
    let fit = learn.linear(f, "y", ["size"])
    println(learn.params(fit) |> map_keys)
    let enc = learn.one_hot(f, ["size"])
    println(ods.columns(learn.transform(enc, f)))
"#,
        "[\"intercept\", \"size_m\", \"size_l\"]\n[\"y\", \"size_s\", \"size_m\", \"size_l\"]",
    );
}

#[test]
fn a_date_column_is_time_on_a_gg_axis() {
    agrees(
        "gg",
        r#"
    use gg
    let f = ods.frame([["d", ods.series(["2024-01-01", "2024-06-01", "2024-12-31"]).to_date()], ["v", [1.0, 2.0, 3.0]]])
    let p = gg.plot(f, gg.aes(x: "d", y: "v")) |> gg.line()
    let svg = plot.gg(p)
    println(str.contains(svg, "2024"))
    let q = gg.plot(ods.with(f, { d: ods.cast(f["d"], "String") }), gg.aes(x: "d", y: "v")) |> gg.line()
    println(plot.gg(q) == svg)
"#,
        "true\ntrue",
    );
}

#[test]
fn frame_verbs_take_dates_and_categories() {
    agrees(
        "frames",
        r#"
    use ods { col }
    let f = ods.read_csv("ts,k,v\n2024-01-01T10:00,a,1\n2024-01-01T11:30,b,2\n2024-01-02T09:00,a,4\n2024-01-09T09:00,b,8\n", dates: ["ts"])
    let g = f |> ods.with({ c: col("k").to_category(), d: col("ts").date() })
    let daily = ods.resample(g, "ts", "1d", { total: col("v").sum() })
    println(ods.schema(daily)["dtype"].to_list())
    println(daily["total"].to_list())
    println(ods.resample(g, "d", "1w", { total: col("v").sum() })["d"].to_list())
    println(g.group_by("c").agg({ n: ods.count(), first: col("d").min() })["first"].to_list())
    let quotes = ods.frame([["t", ods.series(["2024-01-01T00:00", "2024-01-02T00:00"]).to_datetime()], ["price", [10.0, 20.0]]])
    println(ods.asof_join(g.with({ t: col("ts") }), quotes, "t")["price"].to_list())
    println(ods.n_rows(ods.join(g, ods.frame([["c", ods.series(["a"]).to_category()], ["z", [1]]]), "c")))
    println(ods.run_id(g["c"]).to_list())
    println(ods.lookup(g, "d", "2024-01-02", "v"))
"#,
        "[\"Datetime\", \"Int\"]\n[3, 4, 8]\n[2024-01-01, 2024-01-08]\n[2024-01-01, 2024-01-01]\n[10.0, 10.0, 20.0, 20.0]\n2\n[0, 1, 2, 3]\n4",
    );
}

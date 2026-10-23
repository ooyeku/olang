//! The frame and dtype gaps the D6 notebook rewrite found, on every tier
//! (the interpreter alone, every function compiled at its first call, and
//! the default): a Category compared in an expression and by `ods.eq`
//! (which panicked), text joined by `+` in an expression, a Datetime's
//! reductions in an expression, `unpivot` over Int and Float, a number
//! spec's `0` flag, strftime's directives, `take` of a list and
//! `add_months` of a column.

use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run(code: &str, flags: &[&str], tag: &str) -> (String, String) {
    let dir =
        std::env::temp_dir().join(format!("olang_frames_gaps_{}_{}", std::process::id(), tag));
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

/// The program prints `expected` on every tier; the work happens inside
/// `fn main_()`, so the compiled tier compiles it.
fn agrees(tag: &str, body: &str, expected: &str) {
    let code = format!("use ods {{ col }}\nfn main_() = {{\n{body}\n}}\nmain_()\n");
    for flags in TIERS {
        let (out, err) = run(&code, flags, tag);
        assert_eq!(
            out.trim_end(),
            expected.trim_end(),
            "flags {flags:?}\nstderr: {err}\nprogram:\n{code}"
        );
    }
}

#[test]
fn a_category_compares_by_level_in_an_expression_and_in_ods_eq() {
    agrees(
        "cat",
        r#"
    let size = ods.series(["m", "s", "l", "m", ()]).to_category(levels: ["s", "m", "l"])
    let f = ods.frame({ size: size, n: [1, 2, 3, 4, 5] })
    println(ods.eq(size, "m").to_list())
    println(ods.ne(size, "m").to_list())
    println(ods.eq(size, "xl").to_list())
    println(f.filter(col("size") == "m").n.to_list())
    println(f.filter(col("size") >= "m").n.to_list() == f.filter(f.size >= "m").n.to_list())
    println(f.filter(col("size") >= "m").n.to_list())
    println(f.filter(ods.between(col("size"), "s", "m")).n.to_list())
    println(attempt(() => f.filter(col("size") < "xl")))
    println(f.filter(col("size") == col("size")).n_rows())
"#,
        r#"[true, false, false, true, ()]
[false, true, true, false, ()]
[false, false, false, false, ()]
[1, 4]
true
[1, 3, 4]
[1, 2, 4]
Err("ods.filter: \"xl\" is not a level of this Category, so it has no place in its order (levels: \"s\", \"m\", \"l\")")
4"#,
    );
}

#[test]
fn plus_joins_text_in_an_expression_as_it_does_eagerly() {
    agrees(
        "text",
        r#"
    let f = ods.frame({ n: [1, 2, ()], s: ["a", (), "c"], k: ods.series(["x", "y", "x"]).to_category() })
    println(f.with({ t: "n=" + col("n").cast("String") }).t.to_list())
    println(f.with({ t: col("s") + "!" }).t.to_list() == (f.s + "!").to_list())
    println(f.with({ t: col("s") + col("k") }).t.to_list())
    println((f.k + "!").to_list())
    println(attempt(() => f.with({ t: col("s") + 1 })))
"#,
        r#"["n=1", "n=2", ()]
true
["ax", (), "cx"]
["x!", "y!", "x!"]
Err("ods.with: cannot apply addition to Series[String] and Series[Int]")"#,
    );
}

#[test]
fn a_datetimes_reductions_stay_datetimes_in_an_expression() {
    agrees(
        "dt",
        r#"
    let f = ods.frame({ t: ods.series(["2024-01-01T06:00", "2024-01-01T00:00", "2024-01-02T00:00"]).to_datetime(),
                        d: ods.series(["2024-01-05", "2024-03-01", "2024-02-01"]).to_date() })
    println(f.with({ h: col("t") - col("t").min() }).h.to_list())
    println(f.with({ h: col("t").max() - col("t") }).h.to_list())
    println(f.with({ h: col("t") - col("t").first() }).h.to_list())
    println(f.with({ e: col("d") - col("d").min() }).e.to_list())
    println(f.with({ e: col("d").max() - col("d") }).e.to_list())
    println((f.t - f.t.min()).to_list())
    println((f.d - "2024-01-01").to_list())
    println(typeof(f.d.min()))
    println(attempt(() => f.t - "soon"))
"#,
        r#"[21600.0, 0.0, 86400.0]
[64800.0, 86400.0, 0.0]
[0.0, -21600.0, 64800.0]
[0, 56, 27]
[56, 0, 29]
[21600.0, 0.0, 86400.0]
[4, 60, 31]
Date
Err("\"soon\" is not an ISO date (YYYY-MM-DD[THH:MM[:SS]]) to subtract from a Datetime with")"#,
    );
}

#[test]
fn unpivot_widens_int_beside_float() {
    agrees(
        "unpivot",
        r#"
    let f = ods.frame({ id: ["a", "b"], i: [1, 2], x: [1.5, 2.5] })
    let long = f.unpivot("id")
    println(long.value.to_list())
    println(typeof(long.value[0]))
    println(attempt(() => ods.frame({ i: [1], s: ["x"] }).unpivot([])))
"#,
        r#"[1.0, 2.0, 1.5, 2.5]
Float
Err("unpivot: the value columns stack into one column, so they must share a type — 'i' is Int but 's' is String. Cast them first")"#,
    );
}

#[test]
fn a_number_spec_pads_with_zeros_as_python_does() {
    agrees(
        "fmt",
        r##"
    println(ods.series([5, -3, 123]).format("02d").to_list())
    println(ods.series([5]).format("+05d").to_list())
    println(ods.series([1234]).format("08,d").to_list())
    println(ods.series([3.14159]).format("08.3f").to_list())
    println(ods.series([255]).format("#06x").to_list())
    println(str.fmt(7, "03d"))
    println(str.fmt("{:02d}:{:02d}", 9, 5))
"##,
        r##"["05", "-3", "123"]
["+0005"]
["0,001,234"]
["0003.142"]
["0x00ff"]
007
09:05"##,
    );
}

#[test]
fn strftime_abbreviates_with_b_and_a() {
    agrees(
        "strftime",
        r#"
    let d = ods.series(["2024-01-07", "2024-12-30"]).to_date()
    println(d.format("%b %e %Y (%a)").to_list())
    println(d.format("%B %d, %A").to_list())
    println(d.format("%G-W%V-%u %j %U %W").to_list())
    println(d.format("%D").to_list())
    println(attempt(() => ods.series(["2024-01"]).to_date("%Y-%V")))
"#,
        r#"["Jan  7 2024 (Sun)", "Dec 30 2024 (Mon)"]
["January 07, Sunday", "December 30, Monday"]
["2024-W01-7 007 01 01", "2025-W01-1 365 52 53"]
["01/07/24", "12/30/24"]
Err("ods.to_date: \"%Y-%V\" has %C, %G, %U, %V or %W, which format a date but say too little to read one")"#,
    );
}

#[test]
fn take_takes_a_list_and_add_months_a_column() {
    agrees(
        "take",
        r#"
    let f = ods.frame({ d: ods.series(["2024-01-31", "2024-03-15", ()]).to_date(), n: [1, -2, 3], x: [1.0, 2.0, 3.0] })
    println(ods.take(f.x, [2, 0]).to_list())
    println(f.take([1]).n.to_list())
    println(attempt(() => ods.take(f.x, [1.5])))
    println(f.d.add_months(f.n).to_list())
    println(f.with({ e: col("d").add_months(col("n")) }).e.to_list())
    println(f.with({ e: col("d").add_months(1) }).e.to_list())
    println(attempt(() => f.d.add_months(f.x)))
"#,
        r#"[3.0, 1.0]
[-2]
Err("ods.take: the indices must be an Int Series or a list of Ints, got a list holding a Float")
[2024-02-29, 2024-01-15, ()]
[2024-02-29, 2024-01-15, ()]
[2024-02-29, 2024-04-15, ()]
Err("ods.add_months: the months are an Int, got Series[Float]")"#,
    );
}

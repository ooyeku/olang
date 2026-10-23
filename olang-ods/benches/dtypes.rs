//! Text, dates and categories (data ergonomics D2): parsing dates, a
//! date's parts and truncation, the text kernels, indicator columns and
//! grouping by a category. Run with `cargo bench -p olang-ods --bench
//! dtypes` (`-- 1000000` for another size); `benches/dtypes.py` times the
//! pandas side of each row, and docs/performance.md records both.

use olang_ods::fuse::{BinOp, Expr, Op, Plan, TOp};
use olang_ods::temporal::{US_PER_SEC, format_date, format_datetime};
use olang_ods::time::Every;
use olang_ods::{AggOp, AggSpec, DType, Frame, Series, StrCol, category, text};
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

fn pseudo(n: usize, seed: u64) -> impl Iterator<Item = u64> {
    let mut x = seed | 1;
    (0..n).map(move |_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    })
}

const NAMES: [&str; 20] = [
    "Northgate",
    "Riverside",
    "Old Town",
    "Hillcrest",
    "Docklands",
    "Meadowbrook",
    "Ashford",
    "Brookside",
    "Cedar Park",
    "Elmwood",
    "Fairview",
    "Glenmore",
    "Harbourside",
    "Ironbridge",
    "Kingsway",
    "Lakeview",
    "Millbrook",
    "Northfield",
    "Oakridge",
    "Parkside",
];

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

    // Datetimes over four years from 2020, to the second.
    let start = 18262 * 86_400; // 2020-01-01
    let secs: Vec<i64> = pseudo(n, 1)
        .map(|x| start + (x % (4 * 365 * 86_400)) as i64)
        .collect();
    let ts = Series::from_datetimes(secs.iter().map(|s| s * US_PER_SEC).collect());
    let iso = Series::from_str_values(
        secs.iter()
            .map(|&s| format_datetime(s * US_PER_SEC))
            .collect(),
    );
    let days = Series::from_str_values(secs.iter().map(|&s| format_date(s / 86_400)).collect());
    let dmy = Series::from_str_values(
        secs.iter()
            .map(|&s| {
                let t = format_datetime(s * US_PER_SEC);
                format!("{}/{}/{} {}", &t[8..10], &t[5..7], &t[0..4], &t[11..16])
            })
            .collect(),
    );
    row(
        "to_datetime, ISO",
        time(|| drop(black_box(text::to_datetime(&iso, None).unwrap()))),
        None,
    );
    row(
        "to_date, ISO",
        time(|| drop(black_box(text::to_date(&days, None).unwrap()))),
        None,
    );
    row(
        "to_datetime, \"%d/%m/%Y %H:%M\"",
        time(|| {
            drop(black_box(
                text::to_datetime(&dmy, Some("%d/%m/%Y %H:%M")).unwrap(),
            ))
        }),
        None,
    );
    let part = |t: TOp| {
        let e = Expr::op(Op::Temporal(t, false), vec![Expr::col(ts.clone())]);
        let plan = Plan::new(&[e], None).unwrap();
        (
            time(|| drop(black_box(plan.eval(false).unwrap()))),
            time(|| drop(black_box(plan.eval(true).unwrap()))),
        )
    };
    for (name, t) in [
        ("hour", TOp::Hour),
        ("weekday", TOp::Weekday),
        ("year", TOp::Year),
        ("month", TOp::Month),
        ("floor(\"1h\")", TOp::Floor(Every::parse("1h").unwrap())),
        ("floor(\"1mo\")", TOp::Floor(Every::parse("1mo").unwrap())),
    ] {
        let (s, p) = part(t);
        row(name, s, Some(p));
    }

    // Text: a name and a number, about fourteen bytes a cell.
    let words = Series::from_str_values(
        pseudo(n, 2)
            .map(|x| format!(" {} {} ", NAMES[(x % 20) as usize], (x >> 8) % 10_000))
            .collect(),
    );
    row(
        "lower",
        time(|| drop(black_box(text::lower(&words).unwrap()))),
        None,
    );
    row(
        "upper",
        time(|| drop(black_box(text::upper(&words).unwrap()))),
        None,
    );
    row(
        "trim",
        time(|| drop(black_box(text::trim(&words, true, true).unwrap()))),
        None,
    );
    row(
        "contains(\"ark\")",
        time(|| drop(black_box(text::contains(&words, "ark", true).unwrap()))),
        None,
    );
    row(
        "contains(\"ark\", case: false)",
        time(|| drop(black_box(text::contains(&words, "ARK", false).unwrap()))),
        None,
    );
    row(
        "replace(\"o\", \"0\")",
        time(|| drop(black_box(text::replace(&words, "o", "0").unwrap()))),
        None,
    );
    row(
        "len_chars",
        time(|| drop(black_box(text::len_chars(&words).unwrap()))),
        None,
    );
    row(
        "split_n(\" \", 2)",
        time(|| drop(black_box(text::split_n(&words, " ", 2).unwrap()))),
        None,
    );

    // Categories: twenty levels.
    let hoods = Series::from_str_values(
        pseudo(n, 3)
            .map(|x| NAMES[(x % 20) as usize].to_string())
            .collect(),
    );
    let fmt = |x: f64| x.to_string();
    row(
        "to_category",
        time(|| {
            drop(black_box(
                category::to_category(&hoods, None, &fmt).unwrap(),
            ))
        }),
        None,
    );
    let cat = category::to_category(&hoods, None, &fmt).unwrap();
    row(
        "dummies, 20 levels",
        time(|| {
            drop(black_box(
                category::dummies(&cat, Some("h"), true, DType::F64, &fmt).unwrap(),
            ))
        }),
        None,
    );
    row(
        "lower, a Category",
        time(|| drop(black_box(text::lower(&cat).unwrap()))),
        None,
    );

    // Grouping by a category: 1,000 levels.
    let codes: Vec<u32> = pseudo(n, 4).map(|x| (x % 1000) as u32).collect();
    let levels = StrCol::from_strings((0..1000).map(|i| format!("level {i:04}")));
    let g = Series::category(codes, levels, None);
    let gs = g.cast(DType::Str, &fmt).unwrap();
    let v = Series::from_f64(pseudo(n, 5).map(|x| (x % 1000) as f64).collect());
    let spec = [AggSpec {
        out_name: "v".into(),
        op: AggOp::Sum,
        col: "v".into(),
    }];
    let fc = Frame::new(vec![("g".into(), g), ("v".into(), v.clone())]).unwrap();
    let fs = Frame::new(vec![("g".into(), gs), ("v".into(), v)]).unwrap();
    row(
        "group_by(Category) sum",
        time(|| drop(black_box(fc.group_by(&["g".into()], &spec).unwrap()))),
        None,
    );
    row(
        "group_by(String) sum",
        time(|| drop(black_box(fs.group_by(&["g".into()], &spec).unwrap()))),
        None,
    );

    // The D6 gaps: a Category against a level and text joined in an
    // expression, months added by a column, strftime and a padded number.
    // planned and evaluated each round: a Category's comparison and a
    // text join are computed as the plan is made
    let plan_time = |e: std::sync::Arc<Expr>| {
        let run = |par: bool| {
            Plan::new(std::slice::from_ref(&e), None)
                .unwrap()
                .eval(par)
                .unwrap()
        };
        (
            time(|| drop(black_box(run(false)))),
            time(|| drop(black_box(run(true)))),
        )
    };
    let lit = |t: &str| Expr::lit(olang_ods::Scalar::Str(t.into()));
    for (name, op, level) in [
        ("col(cat) == \"Riverside\"", BinOp::Eq, "Riverside"),
        ("col(cat) < \"Kingsway\"", BinOp::Lt, "Kingsway"),
    ] {
        let (s, p) = plan_time(Expr::bin(op, Expr::col(cat.clone()), lit(level)));
        row(name, s, Some(p));
    }
    let (s, p) = plan_time(Expr::bin(BinOp::Add, Expr::col(words.clone()), lit("!")));
    row("col(text) + \"!\"", s, Some(p));
    let dates = Series::from_dates(secs.iter().map(|&x| (x / 86_400) as i32).collect());
    let months = Series::from_i64(pseudo(n, 6).map(|x| (x % 25) as i64 - 12).collect());
    let (s, p) = plan_time(Expr::op(
        Op::Temporal(TOp::AddMonthsBy, false),
        vec![Expr::col(dates.clone()), Expr::col(months)],
    ));
    row("add_months(col(n))", s, Some(p));
    row(
        "format(\"%b %d %Y\")",
        time(|| drop(black_box(text::format(&dates, "%b %d %Y").unwrap()))),
        None,
    );
    let ints = Series::from_i64(pseudo(n, 7).map(|x| (x % 100_000) as i64).collect());
    row(
        "format(\"05d\")",
        time(|| drop(black_box(text::format(&ints, "05d").unwrap()))),
        None,
    );
}

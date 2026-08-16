//! Phase 3 integration tests: the Frame verbs from olang source — CSV
//! and records bridges, the tidyverse pipeline, group_by references,
//! joins, and tier transparency.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn eval(source: &str, tier_threshold: Option<u32>) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        if let Some(threshold) = tier_threshold {
            interpreter.enable_bytecode_tier(threshold, false);
        }
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

fn assert_tier_transparent(source: &str) -> Result<Value, String> {
    let interpreted = eval(source, None);
    let promoted = eval(source, Some(2));
    match (&interpreted, &promoted) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "promotion changed the result\n  source: {}", source),
        (Err(_), Err(_)) => {}
        _ => panic!(
            "promotion changed success/failure\n  interpreted: {:?}\n  promoted: {:?}",
            interpreted, promoted
        ),
    }
    interpreted
}

const SALES_CSV: &str =
    "region,revenue,units\neast,10.5,1\nwest,20.0,2\neast,30.0,3\nnorth,5.5,1\nwest,15.0,2\n";

#[test]
fn read_csv_infers_types() {
    let result = eval(
        &format!(
            r#"
        let df = ods.read_csv("{}")
        let rev = ods.column(df, "revenue")
        let units = ods.column(df, "units")
        let region = ods.column(df, "region")
        [
            ods.n_rows(df), ods.n_cols(df), ods.columns(df),
            typeof(df), to_string(rev) |> str.contains("Float"),
            to_string(units) |> str.contains("Int"),
            to_string(region) |> str.contains("String")
        ]
        "#,
            SALES_CSV.replace('\n', "\\n")
        ),
        None,
    )
    .unwrap();
    let items = match result {
        Value::List(items) => items,
        other => panic!("expected list, got {:?}", other),
    };
    assert_eq!(items[0], Value::Integer(5));
    assert_eq!(items[1], Value::Integer(3));
    assert_eq!(items[3], Value::String("Frame".to_string().into()));
    assert_eq!(items[4], Value::Boolean(true));
    assert_eq!(items[5], Value::Boolean(true));
    assert_eq!(items[6], Value::Boolean(true));
}

#[test]
fn tidyverse_pipeline_group_by_matches_reference() {
    // filter → group_by → sort_by, all piped.
    let result = eval(
        &format!(
            r#"
        let df = ods.read_csv("{}")
        let rev = ods.column(df, "revenue")
        let summary = df
            |> ods.filter(rev > 6.0)
            |> ods.group_by("region", [["total", "sum", "revenue"], ["n", "count"]])
            |> ods.sort_by("total", true)
        ods.to_records(summary)
        "#,
            SALES_CSV.replace('\n', "\\n")
        ),
        None,
    )
    .unwrap();
    // After filter (>6): east 10.5+30, west 20+15. Sorted desc by total:
    // east 40.5 (n=2), west 35 (n=2).
    let recs = match result {
        Value::List(items) => items,
        other => panic!("expected list, got {:?}", other),
    };
    assert_eq!(recs.len(), 2);
    let get = |rec: &Value, k: &str| match rec {
        Value::Map(m) => m.get(k).cloned().unwrap(),
        other => panic!("expected map, got {:?}", other),
    };
    assert_eq!(
        get(&recs[0], "region"),
        Value::String("east".to_string().into())
    );
    assert_eq!(get(&recs[0], "total"), Value::Float(40.5));
    assert_eq!(get(&recs[0], "n"), Value::Integer(2));
    assert_eq!(
        get(&recs[1], "region"),
        Value::String("west".to_string().into())
    );
    assert_eq!(get(&recs[1], "total"), Value::Float(35.0));
}

#[test]
fn frame_construction_select_with_column() {
    let result = assert_tier_transparent(
        r#"
        let df = ods.frame([
            ["name", ["ada", "grace", "alan"]],
            ["score", [90.0, 95.0, 88.0]]
        ])
        let curved = ods.with_column(df, "curved", ods.column(df, "score") + 5.0)
        let thin = ods.select(curved, ["name", "curved"])
        [ods.n_cols(thin), ods.to_list(ods.column(thin, "curved"))]
        "#,
    )
    .unwrap();
    let items = match result {
        Value::List(items) => items,
        other => panic!("expected list, got {:?}", other),
    };
    assert_eq!(items[0], Value::Integer(2));
    assert_eq!(
        items[1],
        Value::List(vec![Value::Float(95.0), Value::Float(100.0), Value::Float(93.0)].into())
    );
}

#[test]
fn frame_from_records_bridges_json() {
    let result = eval(
        r#"
        let parsed = unwrap(json.parse("[{\"a\": 1, \"b\": \"x\"}, {\"a\": 2}, {\"b\": \"z\", \"a\": 3}]"))
        let df = ods.frame_from_records(parsed)
        let b = ods.column(df, "b")
        [ods.n_rows(df), ods.columns(df), ods.null_count(b), ods.get(b, 1)]
        "#,
        None,
    )
    .unwrap();
    let items = match result {
        Value::List(items) => items,
        other => panic!("expected list, got {:?}", other),
    };
    assert_eq!(items[0], Value::Integer(3));
    assert_eq!(
        items[1],
        Value::List(
            vec![
                Value::String("a".to_string().into()),
                Value::String("b".to_string().into())
            ]
            .into()
        )
    );
    assert_eq!(items[2], Value::Integer(1));
    assert_eq!(items[3], Value::Unit);
}

#[test]
fn joins_from_source() {
    let result = eval(
        r#"
        let orders = ods.frame([
            ["id", [1, 2, 3]],
            ["region", ["east", "south", "west"]]
        ])
        let tax = ods.frame([
            ["name", ["east", "west"]],
            ["rate", [0.07, 0.09]]
        ])
        let inner = ods.join(orders, tax, "region", "name")
        let left = ods.join_left(orders, tax, "region", "name")
        [
            ods.n_rows(inner),
            ods.n_rows(left),
            ods.to_list(ods.column(left, "rate"))
        ]
        "#,
        None,
    )
    .unwrap();
    let items = match result {
        Value::List(items) => items,
        other => panic!("expected list, got {:?}", other),
    };
    assert_eq!(items[0], Value::Integer(2));
    assert_eq!(items[1], Value::Integer(3));
    assert_eq!(
        items[2],
        Value::List(vec![Value::Float(0.07), Value::Unit, Value::Float(0.09)].into())
    );
}

#[test]
fn frame_equality_and_display() {
    let result = assert_tier_transparent(
        r#"
        let a = ods.frame([["x", [1, 2]]])
        let b = ods.frame([["x", [1, 2]]])
        let c = ods.frame([["x", [1, 3]]])
        [a == b, a == c, to_string(a)]
        "#,
    )
    .unwrap();
    let items = match result {
        Value::List(items) => items,
        other => panic!("expected list, got {:?}", other),
    };
    assert_eq!(items[0], Value::Boolean(true));
    assert_eq!(items[1], Value::Boolean(false));
    assert_eq!(
        items[2],
        Value::String("Frame[2 x 1](x: Int)".to_string().into())
    );
}

#[test]
fn take_and_head_and_string_masks() {
    let result = eval(
        r#"
        let df = ods.frame([
            ["name", ["ada", "grace", "alan", "edsger"]],
            ["score", [90, 95, 88, 92]]
        ])
        let names = ods.column(df, "name")
        let grace_only = ods.filter(df, names == "grace")
        let top2 = df |> ods.sort_by("score", true) |> ods.head(2)
        [
            ods.n_rows(grace_only),
            ods.to_list(ods.column(top2, "name"))
        ]
        "#,
        None,
    )
    .unwrap();
    let items = match result {
        Value::List(items) => items,
        other => panic!("expected list, got {:?}", other),
    };
    assert_eq!(items[0], Value::Integer(1));
    assert_eq!(
        items[1],
        Value::List(
            vec![
                Value::String("grace".to_string().into()),
                Value::String("edsger".to_string().into())
            ]
            .into()
        )
    );
}

#[test]
fn frame_error_paths() {
    let err = eval("ods.frame([[1, 2]])", None).unwrap_err();
    assert!(err.contains("column name must be a String"), "got: {}", err);
    let err = eval(
        r#"ods.group_by(ods.frame([["x", [1]]]), "x", [["s", "median", "x"]])"#,
        None,
    )
    .unwrap_err();
    assert!(err.contains("unknown aggregation"), "got: {}", err);
    let err = eval(r#"ods.column(ods.frame([["x", [1]]]), "y")"#, None).unwrap_err();
    assert!(err.contains("no column named"), "got: {}", err);
}

// ── DP2: the file IO surface ──────────────────────────────────────────
//
// The data chapter described an end-to-end story the stack could not
// finish: it could read CSV *text* but had no way to reach a file, and no
// way to write anything out at all.

#[test]
fn a_frame_round_trips_through_csv_text() {
    // to_csv is the exact inverse of read_csv, which is the property that
    // makes it worth having: nulls become empty cells, and empty cells
    // read back as null.
    let out = eval(
        r#"
let f = ods.frame_from_records([
    #{ "name": "a", "n": 1, "x": 1.5, "ok": true },
    #{ "name": "b", "n": 2, "x": 2.5, "ok": false },
])
let back = ods.read_csv(ods.to_csv(f))
show([ods.n_rows(back), ods.n_cols(back)]) + " " + show(ods.sum(ods.column(back, "x")))
"#,
        None,
    )
    .expect("round-trip");
    assert_eq!(out.to_string(), "\"[2, 4] 4.0\"".to_string());
}

#[test]
fn csv_text_quotes_separators_and_quotes() {
    // A value containing the delimiter or a quote has to survive, or the
    // round-trip silently changes the column count.
    let out = eval(
        r#"
let f = ods.frame_from_records([#{ "note": "a, b" }, #{ "note": "say \"hi\"" }])
let back = ods.read_csv(ods.to_csv(f))
show(ods.n_cols(back)) + " " + show(ods.to_records(back))
"#,
        None,
    )
    .expect("quoting");
    let s = out.to_string();
    assert!(s.contains("a, b"), "{s}");
    assert!(s.contains("say \"hi\""), "{s}");
    assert!(s.starts_with("\"1 "), "column count changed: {s}");
}

#[test]
fn write_and_read_a_csv_file() {
    let dir = std::env::temp_dir().join(format!("olang_dp2_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.csv");
    let src = format!(
        r#"
let f = ods.frame_from_records([#{{ "k": "x", "v": 10 }}, #{{ "k": "y", "v": 32 }}])
let w = ods.write_csv(f, "{p}")
let back = unwrap(ods.read_csv_file("{p}"))
show(is_ok(w)) + " " + show(ods.sum(ods.column(back, "v")))
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("file round-trip");
    assert_eq!(out.to_string(), "\"true 42\"".to_string());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_file_is_an_err_not_a_raise() {
    // The 0.64 rule: a failure the caller could handle is a Result. A
    // missing file is the caller's input, not their mistake.
    let out = eval(
        "show(is_err(ods.read_csv_file(\"/nope/definitely/missing.csv\")))\n",
        None,
    )
    .expect("should not raise");
    assert_eq!(out.to_string(), "\"true\"".to_string());
}

#[test]
fn file_io_works_the_same_on_the_bytecode_tier() {
    // The tier reaches these through the same dispatch, so it must agree.
    assert_tier_transparent(
        r#"
let f = ods.frame_from_records([#{ "n": 1 }, #{ "n": 2 }, #{ "n": 3 }])
ods.to_csv(f)
"#,
    )
    .expect("tier agreement");
}

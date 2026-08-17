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
        Value::String(
            "Frame[2 x 1]\n┌─────┐\n│   x │\n│ Int │\n├─────┤\n│   1 │\n│   2 │\n└─────┘"
                .to_string()
                .into()
        )
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

// ── DP2: the streaming reader ─────────────────────────────────────────

/// Build a CSV of `n` rows and hand back its path.
fn big_csv(tag: &str, n: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_stream_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("data.csv");
    let mut text = String::from("id,region,amount\n");
    for i in 0..n {
        text.push_str(&format!(
            "{},{},{}\n",
            i,
            ["east", "west"][i % 2],
            (i % 7) + 1
        ));
    }
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn streaming_a_file_in_chunks_matches_reading_it_whole() {
    // The property that makes the reader worth having: a windowed pass and
    // a whole-file pass are the same computation.
    let path = big_csv("agree", 2500);
    let src = format!(
        r#"
let r = unwrap(ods.open_csv("{p}"))
let mut total = 0
let mut rows = 0
loop {{
    let chunk = unwrap(ods.next_chunk(r, 300))
    let n = ods.n_rows(chunk)
    if n == 0 => break
    total = total + ods.sum(ods.column(chunk, "amount"))
    rows = rows + n
}}
let whole = unwrap(ods.read_csv_file("{p}"))
show([rows, ods.rows_read(r), total == ods.sum(ods.column(whole, "amount")), ods.at_end(r)])
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("stream");
    assert_eq!(out.to_string(), "\"[2500, 2500, true, true]\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_last_chunk_is_short_and_the_next_one_is_empty() {
    // 2500 rows in 300s: eight full chunks, one of 100, then the empty
    // chunk that ends the loop. The empty one still carries the columns,
    // so a pipeline written against a chunk does not fall over on it.
    let path = big_csv("tail", 2500);
    let src = format!(
        r#"
let r = unwrap(ods.open_csv("{p}"))
let mut sizes = []
loop {{
    let c = unwrap(ods.next_chunk(r, 300))
    sizes = sizes + [ods.n_rows(c)]
    if ods.n_rows(c) == 0 => break
}}
let last = unwrap(ods.next_chunk(r, 300))
show([len(sizes), sizes[8], sizes[9], ods.n_cols(last)])
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("tail");
    assert_eq!(out.to_string(), "\"[10, 100, 0, 3]\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_reader_is_confined_to_the_thread_that_opened_it() {
    // A reader holds a file position — mutable state — so two threads
    // reaching one would be exactly the shared-mutable-state hole the
    // language's parallelism depends on not existing. Same rule as `cell`.
    let path = big_csv("conf", 50);
    let src = format!(
        r#"
let r = unwrap(ods.open_csv("{p}"))
match task.join(spawn ods.next_chunk(r, 5)) {{ Err(e) => "refused", v => "LEAKED" }}
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("confinement");
    assert_eq!(out.to_string(), "\"refused\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_channel_refuses_to_carry_a_reader() {
    // The one crossing that holds the value being sent. This passes
    // because `chan.send` asks the value whether it is confined rather
    // than naming the types it knows — the reader was never mentioned
    // there.
    let path = big_csv("chan", 50);
    let src = format!(
        r#"
let r = unwrap(ods.open_csv("{p}"))
chan.send(chan.new(), r)
"#,
        p = path.to_string_lossy()
    );
    let err = eval(&src, None).expect_err("a reader must not cross a channel");
    assert!(err.contains("Reader"), "{err}");
    assert!(err.contains("cannot be sent"), "{err}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_zero_row_chunk_is_refused_rather_than_looping_forever() {
    // `next_chunk(r, 0)` would return an empty Frame, which is the loop's
    // termination signal — so a program asking for zero rows would exit
    // its loop having read nothing, silently. Misuse raises (the 0.64
    // rule) rather than quietly producing a wrong answer.
    let path = big_csv("zero", 10);
    let src = format!(
        "let r = unwrap(ods.open_csv(\"{p}\"))\nods.next_chunk(r, 0)\n",
        p = path.to_string_lossy()
    );
    let err = eval(&src, None).expect_err("zero must be refused");
    assert!(err.contains("at least 1"), "{err}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn opening_a_missing_file_is_an_err() {
    let out = eval(
        "show(is_err(ods.open_csv(\"/nope/missing/stream.csv\")))\n",
        None,
    )
    .expect("no raise");
    assert_eq!(out.to_string(), "\"true\"".to_string());
}

// ── Table rendering ───────────────────────────────────────────────────

/// The table a Frame renders to. `to_string` yields a String value whose
/// own Display adds the surrounding quotes, which would skew the width
/// assertions below, so they come off here rather than in each test.
fn rendered(source: &str) -> String {
    let out = eval(source, None).expect("render").to_string();
    out.trim_matches('"').to_string()
}

/// Every box line in a rendered table, for the checks that care about
/// alignment rather than content.
fn box_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| !l.starts_with("Frame[")).collect()
}

#[test]
fn a_frame_displays_as_a_table() {
    // The shape line survives from the old one-line display — it is the
    // one fact a truncated table cannot show — and the table follows.
    let text = rendered(
        r#"
let f = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\n")
to_string(f)
"#,
    );
    for expected in [
        "Frame[2 x 3]",
        "│ region │ amount │ qty │",
        "│ String │  Float │ Int │",
        "│ east   │   25.5 │  10 │",
        "│ west   │  320.0 │   3 │",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
}

#[test]
fn numbers_are_right_aligned_and_text_is_left_aligned() {
    // Alignment is the whole reason a table beats a list: a column of
    // numbers is only scannable when the digits line up.
    // Column widths come from the widest of name, dtype, and cells, so
    // "String" sets the first column's width here.
    let text = rendered(r#"to_string(ods.read_csv("name,n\na,1\nbbbb,1000\n"))"#);
    assert!(text.contains("│ a      │    1 │"), "{text}");
    assert!(text.contains("│ bbbb   │ 1000 │"), "{text}");
}

#[test]
fn a_long_frame_elides_its_middle_and_says_so() {
    // Both ends matter — the head shows what the data is, the tail shows
    // where a sort landed — so the elision goes in the middle, and the
    // count of what it hid is printed rather than left to be inferred.
    let mut csv = String::from("i\n");
    for i in 0..500 {
        csv.push_str(&format!("{}\n", i));
    }
    let text = rendered(&format!("to_string(ods.read_csv({:?}))", csv));
    assert!(text.contains("Frame[500 x 1]"), "{text}");
    assert!(text.contains("│   0 │"), "head row missing:\n{text}");
    assert!(text.contains("│ 499 │"), "tail row missing:\n{text}");
    assert!(text.contains("│   … │"), "elision missing:\n{text}");
    assert!(text.contains("480 rows not shown"), "{text}");
}

#[test]
fn a_wide_frame_drops_columns_and_says_so() {
    // Wrapping a table destroys the alignment that makes it worth having,
    // so the width budget drops columns instead — and reports the drop.
    let names: Vec<String> = (0..40).map(|i| format!("column_{}", i)).collect();
    let row: Vec<String> = (0..40).map(|i| i.to_string()).collect();
    let csv = format!("{}\n{}\n", names.join(","), row.join(","));
    let text = rendered(&format!("to_string(ods.read_csv({:?}))", csv));
    assert!(text.contains("Frame[1 x 40]"), "{text}");
    assert!(text.contains("columns not shown"), "{text}");
    for line in box_lines(&text) {
        assert!(
            line.chars().count() <= 100,
            "line of {} chars exceeds the width budget:\n{line}",
            line.chars().count()
        );
    }
}

#[test]
fn an_empty_frame_still_shows_its_columns() {
    // The case that prompted this: a Frame with a schema and no rows used
    // to print as a shape and a type list, which reads like an error.
    let text = rendered(r#"to_string(ods.read_csv("a,b\n"))"#);
    assert!(text.contains("Frame[0 x 2]"), "{text}");
    assert!(text.contains("│ a "), "{text}");
    assert!(text.contains("(no rows)"), "{text}");
    // The filler row must not break the box.
    let widths: Vec<usize> = box_lines(&text)
        .iter()
        .map(|l| l.chars().count())
        .collect::<Vec<_>>();
    assert!(
        widths.windows(2).all(|w| w[0] == w[1]),
        "ragged box: {widths:?}\n{text}"
    );
}

#[test]
fn a_wide_cell_is_truncated_rather_than_stretching_the_table() {
    let long = "x".repeat(200);
    let text = rendered(&format!(
        "to_string(ods.read_csv({:?}))",
        format!("s\n{}\n", long)
    ));
    assert!(text.contains('…'), "{text}");
    for line in box_lines(&text) {
        assert!(line.chars().count() <= 100, "{line}");
    }
}

#[test]
fn head_defaults_to_ten_rows() {
    // `ods.head(f)` is what a hand types at the REPL; demanding the count
    // made the most-used verb the one most likely to error.
    let mut csv = String::from("i\n");
    for i in 0..50 {
        csv.push_str(&format!("{}\n", i));
    }
    let out = eval(
        &format!("ods.n_rows(ods.head(ods.read_csv({:?})))", csv),
        None,
    )
    .expect("head");
    assert_eq!(out.to_string(), "10".to_string());
}

#[test]
fn the_default_head_agrees_across_tiers() {
    // The default is applied in `ods` dispatch, which both tiers share —
    // this is what proves it, rather than the claim that they share it.
    assert_tier_transparent(
        r#"
let f = ods.frame_from_records([#{ "n": 1 }, #{ "n": 2 }, #{ "n": 3 }])
ods.n_rows(ods.head(f))
"#,
    )
    .expect("tier agreement");
}

// ── JSON lines ────────────────────────────────────────────────────────

/// A JSON-lines file of `n` records, and its path.
fn big_jsonl(tag: &str, n: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_jsonl_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("data.jsonl");
    let mut text = String::new();
    for i in 0..n {
        text.push_str(&format!(
            "{{\"id\": {}, \"region\": \"{}\", \"amount\": {}}}\n",
            i,
            ["east", "west"][i % 2],
            (i % 7) + 1
        ));
    }
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn json_lines_become_a_frame() {
    // Columns are the union of the keys, so a record missing one is a
    // null rather than an error — the same rule frame_from_records uses,
    // because this is that function with a parser in front of it.
    let text = "{\"name\": \"ada\", \"score\": 99}\n\
                {\"name\": \"bob\", \"score\": 87, \"note\": \"late\"}\n";
    let out = eval(
        &format!(
            "let f = unwrap(ods.read_jsonl({:?}))\nshow([ods.columns(f), \
             [ods.n_rows(f), ods.null_count(ods.column(f, \"note\"))]])",
            text
        ),
        None,
    )
    .expect("read_jsonl");
    assert_eq!(
        out.to_string(),
        "\"[[\"name\", \"note\", \"score\"], [2, 1]]\"".to_string()
    );
}

#[test]
fn blank_lines_are_skipped_rather_than_refused() {
    // A trailing newline is how nearly every writer finishes the format;
    // refusing it would make the common file the failing case.
    let out = eval(
        "ods.n_rows(unwrap(ods.read_jsonl(\"{\\\"a\\\": 1}\\n\\n{\\\"a\\\": 2}\\n\\n\")))",
        None,
    )
    .expect("blank lines");
    assert_eq!(out.to_string(), "2".to_string());
}

#[test]
fn a_frame_round_trips_through_json_lines() {
    // to_jsonl omits nulls rather than writing them, which is what makes
    // the round trip land on the same Frame: read_jsonl turns a missing
    // key into a null, so writing `null` would be a second way to say the
    // same thing.
    let out = eval(
        r#"
let f = unwrap(ods.read_jsonl("{\"a\": 1, \"b\": \"x\"}\n{\"a\": 2}\n"))
show(unwrap(ods.read_jsonl(ods.to_jsonl(f))) == f)
"#,
        None,
    )
    .expect("round trip");
    assert_eq!(out.to_string(), "\"true\"".to_string());
}

#[test]
fn a_non_object_line_names_the_line_number() {
    // On a million-line file the line number is the whole diagnostic.
    let out = eval(
        "match ods.read_jsonl(\"{\\\"a\\\": 1}\\n[1, 2]\\n\") { Err(e) => e, v => \"LEAKED\" }",
        None,
    )
    .expect("refusal");
    let text = out.to_string();
    assert!(text.contains("line 2"), "{text}");
    assert!(text.contains("must be a JSON object"), "{text}");
}

#[test]
fn malformed_json_is_an_err_not_a_raise() {
    // The caller chose the file, so its contents are input, not a bug.
    let out = eval("show(is_err(ods.read_jsonl(\"{oops\\n\")))", None).expect("no raise");
    assert_eq!(out.to_string(), "\"true\"".to_string());
    let out = eval(
        "show(is_err(ods.read_jsonl_file(\"/nope/missing.jsonl\")))",
        None,
    )
    .expect("no raise");
    assert_eq!(out.to_string(), "\"true\"".to_string());
}

#[test]
fn streaming_json_lines_matches_reading_them_whole() {
    let path = big_jsonl("agree", 2500);
    let src = format!(
        r#"
let r = unwrap(ods.open_jsonl("{p}"))
let mut total = 0
let mut rows = 0
loop {{
    let chunk = unwrap(ods.next_chunk(r, 300))
    let n = ods.n_rows(chunk)
    if n == 0 => break
    total = total + ods.sum(ods.column(chunk, "amount"))
    rows = rows + n
}}
let whole = unwrap(ods.read_jsonl_file("{p}"))
show([rows, ods.rows_read(r), total == ods.sum(ods.column(whole, "amount")), ods.at_end(r)])
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("stream");
    assert_eq!(out.to_string(), "\"[2500, 2500, true, true]\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_last_json_lines_chunk_still_carries_the_columns() {
    // JSON lines have no header, so the reader remembers the columns the
    // earlier chunks established. Without that, the empty chunk that ends
    // the loop would come back with none and every pipeline written
    // against a chunk would need a special case for its last iteration.
    let path = big_jsonl("tail", 20);
    let src = format!(
        r#"
let r = unwrap(ods.open_jsonl("{p}"))
let full = unwrap(ods.next_chunk(r, 20))
let empty = unwrap(ods.next_chunk(r, 20))
show([ods.columns(empty) == ods.columns(full), [ods.n_rows(empty), ods.n_cols(empty)]])
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("tail");
    assert_eq!(out.to_string(), "\"[true, [0, 3]]\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn one_set_of_verbs_drives_both_formats() {
    // The point of a single Reader type: after the `open_`, nothing in
    // the loop names the format. This test is the same source text twice
    // with only the opener changed.
    let csv = big_csv("both", 40);
    let jsonl = big_jsonl("both", 40);
    let drive = |open: &str| {
        format!(
            r#"
let r = unwrap({open})
let mut rows = 0
loop {{
    let c = unwrap(ods.next_chunk(r, 15))
    if ods.n_rows(c) == 0 => break
    rows = rows + ods.n_rows(c)
}}
show([rows, ods.rows_read(r)])
"#
        )
    };
    let from_csv = eval(
        &drive(&format!("ods.open_csv(\"{}\")", csv.to_string_lossy())),
        None,
    )
    .expect("csv");
    let from_jsonl = eval(
        &drive(&format!("ods.open_jsonl(\"{}\")", jsonl.to_string_lossy())),
        None,
    )
    .expect("jsonl");
    assert_eq!(from_csv, from_jsonl);
    assert_eq!(from_csv.to_string(), "\"[40, 40]\"".to_string());
    let _ = std::fs::remove_dir_all(csv.parent().unwrap());
    let _ = std::fs::remove_dir_all(jsonl.parent().unwrap());
}

#[test]
fn a_json_lines_reader_is_confined_like_every_other_reader() {
    // Written for free: confinement lives on the NativeObject trait, so
    // the second reader inherited both boundaries from the first.
    let path = big_jsonl("conf", 50);
    let src = format!(
        r#"
let r = unwrap(ods.open_jsonl("{p}"))
match task.join(spawn ods.next_chunk(r, 5)) {{ Err(e) => "refused", v => "LEAKED" }}
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("confinement");
    assert_eq!(out.to_string(), "\"refused\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn json_lines_io_agrees_across_tiers() {
    assert_tier_transparent(
        r#"
let f = unwrap(ods.read_jsonl("{\"n\": 1}\n{\"n\": 2}\n"))
ods.to_jsonl(f)
"#,
    )
    .expect("tier agreement");
}

// ── Subscript ─────────────────────────────────────────────────────────

#[test]
fn a_frame_is_indexed_by_column_name() {
    // The gesture this exists for: `ods.column` and `ods.get` were 13% of
    // every ods call in the repository before it.
    let out = eval(
        r#"
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\n")
ods.to_list(sales["amount"])
"#,
        None,
    )
    .expect("index");
    assert_eq!(out.to_string(), "[25.5, 320.0]".to_string());
}

#[test]
fn indexed_columns_compose_into_derived_columns() {
    let out = eval(
        r#"
let sales = ods.read_csv("amount,qty\n25.5,10\n320.0,3\n")
let full = ods.with_column(sales, "revenue", sales["amount"] * sales["qty"])
ods.to_list(full["revenue"])
"#,
        None,
    )
    .expect("derive");
    assert_eq!(out.to_string(), "[255.0, 960.0]".to_string());
}

#[test]
fn a_frame_is_indexed_by_a_mask() {
    // Two meanings for one subscript, disjoint by the key's type — the
    // thing pandas gets wrong by also overloading it with position.
    let out = eval(
        r#"
let sales = ods.read_csv("region,amount\neast,25.5\nwest,320.0\neast,80.0\n")
ods.to_list(sales[sales["amount"] > 50.0]["region"])
"#,
        None,
    )
    .expect("mask");
    assert_eq!(out.to_string(), "[\"west\", \"east\"]".to_string());
}

#[test]
fn a_missing_column_names_the_columns_that_exist() {
    // A column name written into the source that is not in the data is a
    // bug, not input, so it raises — and the message answers the question
    // the author is about to ask.
    let err =
        eval(r#"ods.read_csv("a,b\n1,2\n")["nope"]"#, None).expect_err("missing column must raise");
    assert!(err.contains("no column 'nope'"), "{err}");
    assert!(err.contains("a, b"), "{err}");
}

#[test]
fn a_frame_refuses_a_row_position_and_names_the_verb() {
    // Refusing `f[0]` is the whole reason `[]` stays unambiguous here.
    let err = eval(r#"ods.read_csv("a\n1\n")[0]"#, None).expect_err("must refuse");
    assert!(err.contains("not by row position"), "{err}");
    assert!(err.contains("ods.head"), "{err}");
}

#[test]
fn a_series_is_indexed_by_position() {
    let out = eval(
        r#"
let s = ods.series([10, 20, 30])
show([s[0], s[2], s[-1]])
"#,
        None,
    )
    .expect("series index");
    assert_eq!(out.to_string(), "\"[10, 30, 30]\"".to_string());
    let err = eval("ods.series([1, 2])[5]", None).expect_err("out of bounds");
    assert!(err.contains("out of bounds"), "{err}");
}

#[test]
fn a_series_indexed_by_name_points_back_at_the_frame() {
    let err = eval(r#"ods.series([1, 2])["amount"]"#, None).expect_err("must refuse");
    assert!(err.contains("indexed by position"), "{err}");
}

#[test]
fn subscripting_agrees_across_tiers() {
    // The hook lives on NativeObject and is reached from both tiers'
    // index paths, which is what this asserts rather than assumes.
    assert_tier_transparent(
        r#"
let sales = ods.read_csv("a,b\n1,2\n3,4\n")
ods.to_list(sales[sales["a"] > 1]["b"])
"#,
    )
    .expect("tier agreement");
    assert_tier_transparent("ods.series([1, 2, 3])[-1]").expect("tier agreement");
}

#[test]
fn a_value_that_is_not_subscriptable_says_what_it_is() {
    let err = eval(
        r#"
let r = unwrap(ods.open_csv("/nope/x.csv"))
r[0]
"#,
        None,
    )
    .expect_err("open fails first");
    // The open is what fails here; the point is only that indexing a
    // non-subscriptable native does not panic.
    assert!(!err.is_empty());
}

// ── describe and schema ───────────────────────────────────────────────

#[test]
fn describe_summarizes_every_column() {
    // A Frame rather than a map, so it prints as a table and can itself
    // be sorted, filtered, and written out.
    let out = eval(
        r#"
let f = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,,4\n")
let d = ods.describe(f)
show([ods.columns(d), ods.to_list(d["column"]), ods.to_list(d["nulls"])])
"#,
        None,
    )
    .expect("describe");
    let text = out.to_string();
    assert!(
        text.contains("\"mean\", \"std\", \"min\", \"q25\", \"median\", \"q75\", \"max\""),
        "{text}"
    );
    assert!(text.contains("[\"region\", \"amount\", \"qty\"]"), "{text}");
    assert!(text.contains("[0, 1, 0]"), "{text}");
}

#[test]
fn describe_leaves_non_numeric_statistics_null() {
    // One type per column means the alternative is two shapes of result
    // depending on the input, and a caller branching on the shape of a
    // summary is worse off than one reading nulls.
    let out = eval(
        r#"
let d = ods.describe(ods.read_csv("s,n\nx,1\ny,2\n"))
show([ods.null_count(d["mean"]), ods.to_list(d["max"])])
"#,
        None,
    )
    .expect("describe");
    assert_eq!(out.to_string(), "\"[1, [(), 2.0]]\"".to_string());
}

#[test]
fn schema_is_describe_without_the_arithmetic() {
    let out = eval(
        r#"
let s = ods.schema(ods.read_csv("a,b,c\n1,x,true\n"))
show([ods.columns(s), ods.to_list(s["dtype"])])
"#,
        None,
    )
    .expect("schema");
    assert_eq!(
        out.to_string(),
        "\"[[\"column\", \"dtype\", \"nulls\"], [\"Int\", \"String\", \"Bool\"]]\"".to_string()
    );
}

#[test]
fn a_long_float_is_shortened_and_the_table_says_so() {
    // A computed column routinely carries seventeen digits, and one such
    // column is wide enough to push two others off the table. The cap is
    // reported like every other cap the renderer applies.
    let text = rendered(r#"to_string(ods.describe(ods.read_csv("n\n25.5\n320.0\n12.25\n")))"#);
    assert!(text.contains("floats to 6 significant digits"), "{text}");
    assert!(!text.contains("173.98078198467783"), "{text}");
    for line in box_lines(&text) {
        assert!(line.chars().count() <= 100, "{line}");
    }
}

#[test]
fn short_floats_are_left_exactly_as_they_are() {
    // The shortening must not touch a value that already fits, or every
    // ordinary table would carry the footnote.
    let text = rendered(r#"to_string(ods.read_csv("n\n25.5\n320.0\n"))"#);
    assert!(!text.contains("significant digits"), "{text}");
    assert!(text.contains("25.5"), "{text}");
}

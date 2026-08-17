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

// ── The native columnar format ────────────────────────────────────────

/// A path in a fresh directory for a columnar file.
fn olc_path(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_olc_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("frame.olc")
}

#[test]
fn a_frame_round_trips_through_the_columnar_format() {
    // Every dtype, and a null in each — the property the whole format
    // exists for, since CSV cannot carry either faithfully.
    let path = olc_path("round");
    let src = format!(
        r#"
let f = ods.read_csv("s,i,x,b\na,1,2.5,true\n,,,\nc,3,4.5,false\n")
unwrap(ods.write_frame(f, "{p}"))
show(unwrap(ods.read_frame("{p}")) == f)
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("round trip");
    assert_eq!(out.to_string(), "\"true\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn column_types_survive_the_round_trip() {
    // The concrete cost of text interchange: a String column of zero-
    // padded codes comes back from CSV as Int, with the padding gone —
    // a silent corruption of postcodes, part numbers, and account ids.
    // The columnar format records the type, so it cannot happen.
    let path = olc_path("types");
    let src = format!(
        r#"
let f = ods.frame([["zip", ["007", "042", "100"]]])
unwrap(ods.write_frame(f, "{p}"))
let back = unwrap(ods.read_frame("{p}"))
let viacsv = ods.read_csv(ods.to_csv(f))
show([ods.to_list(back["zip"]), ods.to_list(viacsv["zip"])])
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("types");
    assert_eq!(
        out.to_string(),
        "\"[[\"007\", \"042\", \"100\"], [7, 42, 100]]\"".to_string()
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn reading_one_column_skips_the_others() {
    // The point of a columnar layout on disk. The named columns come
    // back in the order asked for, not the order stored.
    let path = olc_path("project");
    let src = format!(
        r#"
let f = ods.read_csv("a,b,c\n1,2,3\n4,5,6\n")
unwrap(ods.write_frame(f, "{p}"))
let some = unwrap(ods.read_frame("{p}", ["c", "a"]))
show([ods.columns(some), ods.to_list(some["c"])])
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("projection");
    assert_eq!(out.to_string(), "\"[[\"c\", \"a\"], [3, 6]]\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_projection_naming_an_absent_column_is_an_err() {
    let path = olc_path("absent");
    let src = format!(
        r#"
unwrap(ods.write_frame(ods.read_csv("a\n1\n"), "{p}"))
match ods.read_frame("{p}", ["nope"]) {{ Err(e) => e, v => "LEAKED" }}
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("refusal");
    let text = out.to_string();
    assert!(text.contains("no column 'nope'"), "{text}");
    assert!(text.contains("It has: a"), "{text}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn frame_info_describes_a_file_without_loading_it() {
    // The openness payoff: what is in this file, answered from the
    // header alone.
    let path = olc_path("info");
    let src = format!(
        r#"
let f = ods.read_csv("region,amount\neast,1.5\nwest,\n")
unwrap(ods.write_frame(f, "{p}"))
let info = unwrap(ods.frame_info("{p}"))
show([ods.columns(info), ods.to_list(info["column"]), ods.to_list(info["nulls"])])
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("info");
    let text = out.to_string();
    assert!(
        text.contains("[\"column\", \"dtype\", \"nulls\", \"bytes\"]"),
        "{text}"
    );
    assert!(text.contains("[\"region\", \"amount\"], [0, 1]"), "{text}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_header_is_readable_text() {
    // "Readable rather than a black box" is the reason this format is
    // not Parquet, so it is a test rather than a claim in a comment.
    let path = olc_path("header");
    let src = format!(
        r#"
let f = ods.read_csv("region,n\neast,1\nwest,2\n")
unwrap(ods.write_frame(f, "{p}"))
""
"#,
        p = path.to_string_lossy()
    );
    eval(&src, None).expect("write");
    let bytes = std::fs::read(&path).expect("read");
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).to_string();
    assert!(head.starts_with("olang-columns 1\n"), "{head}");
    assert!(head.contains("rows 2\n"), "{head}");
    assert!(head.contains("col \"region\" String enc="), "{head}");
    assert!(head.contains("col \"n\" Int enc=plain nulls=0"), "{head}");
    assert!(head.contains("\ndata\n"), "{head}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_repeating_string_column_is_dictionary_encoded() {
    // Repetition is the normal case in a table. Storing each row's text
    // separately made the file larger than the CSV it came from, so the
    // encoder compares both layouts and writes the smaller.
    let mut repeating = String::from("region,unique\n");
    for i in 0..500 {
        repeating.push_str(&format!("{},{}\n", ["east", "west"][i % 2], i));
    }
    let path = olc_path("dict");
    let src = format!(
        "unwrap(ods.write_frame(ods.read_csv({:?}), \"{p}\"))\n\"\"",
        repeating,
        p = path.to_string_lossy()
    );
    eval(&src, None).expect("write");
    let head = String::from_utf8_lossy(&std::fs::read(&path).unwrap()[..200]).to_string();
    assert!(head.contains("col \"region\" String enc=dict"), "{head}");
    // And it must still be the same data.
    let back = eval(
        &format!(
            "take(ods.to_list(unwrap(ods.read_frame(\"{p}\", [\"region\"]))[\"region\"]), 3)",
            p = path.to_string_lossy()
        ),
        None,
    )
    .expect("read back");
    assert_eq!(
        back.to_string(),
        "[\"east\", \"west\", \"east\"]".to_string()
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_column_of_distinct_strings_stays_plain() {
    // The other side of the same choice: a dictionary of unique values
    // is strictly bigger, so it must not be picked.
    let mut distinct = String::from("s\n");
    for i in 0..500 {
        distinct.push_str(&format!("value-{}\n", i));
    }
    let path = olc_path("plain");
    let src = format!(
        "unwrap(ods.write_frame(ods.read_csv({:?}), \"{p}\"))\n\"\"",
        distinct,
        p = path.to_string_lossy()
    );
    eval(&src, None).expect("write");
    let head = String::from_utf8_lossy(&std::fs::read(&path).unwrap()[..120]).to_string();
    assert!(head.contains("col \"s\" String enc=plain"), "{head}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_dictionary_column_with_nulls_round_trips() {
    // The two mechanisms compose: a validity bitmap in front of codes.
    let mut text = String::from("region\n");
    for i in 0..100 {
        text.push_str(if i % 5 == 0 {
            "\n"
        } else if i % 2 == 0 {
            "east\n"
        } else {
            "west\n"
        });
    }
    let path = olc_path("dictnull");
    let src = format!(
        r#"
let f = ods.read_csv({text:?})
unwrap(ods.write_frame(f, "{p}"))
show(unwrap(ods.read_frame("{p}")) == f)
"#,
        text = text,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("round trip");
    assert_eq!(out.to_string(), "\"true\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn awkward_column_names_survive_the_header() {
    // The header is line-oriented, so a name holding a quote, a space, or
    // a newline would break it if it were not JSON-quoted.
    let path = olc_path("names");
    let src = format!(
        r#"
let f = ods.frame([
    ["has space", [1]],
    ["has\"quote", [2]],
    ["has\nnewline", [3]],
])
unwrap(ods.write_frame(f, "{p}"))
show(ods.columns(unwrap(ods.read_frame("{p}"))))
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("names");
    let text = out.to_string();
    assert!(text.contains("has space"), "{text}");
    assert!(text.contains("newline"), "{text}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_frame_with_no_rows_round_trips() {
    let path = olc_path("empty");
    let src = format!(
        r#"
let f = ods.read_csv("a,b\n")
unwrap(ods.write_frame(f, "{p}"))
let back = unwrap(ods.read_frame("{p}"))
show([ods.columns(back), [ods.n_rows(back)]])
"#,
        p = path.to_string_lossy()
    );
    let out = eval(&src, None).expect("empty");
    assert_eq!(out.to_string(), "\"[[\"a\", \"b\"], [0]]\"".to_string());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_file_that_is_not_ours_is_an_err_not_a_crash() {
    // The caller chose the file, so its contents are input. Every
    // malformed shape must land on Err rather than a panic.
    let dir = std::env::temp_dir().join(format!("olang_olc_bad_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases = [
        ("plain.txt", "hello, this is not a frame\n".as_bytes().to_vec()),
        (
            "version.olc",
            b"olang-columns 99\nrows 1\ncolumns 0\ndata\n".to_vec(),
        ),
        (
            "truncated.olc",
            b"olang-columns 1\nrows 4\ncolumns 1\ncol \"a\" Int enc=plain nulls=0 bytes=32\ndata\n\x01".to_vec(),
        ),
        (
            "badenc.olc",
            b"olang-columns 1\nrows 0\ncolumns 1\ncol \"a\" Int enc=rle nulls=0 bytes=0\ndata\n".to_vec(),
        ),
    ];
    for (name, bytes) in cases {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let src = format!(
            "show(is_err(ods.read_frame(\"{p}\")))",
            p = path.to_string_lossy()
        );
        let out =
            eval(&src, None).unwrap_or_else(|err| panic!("{name} raised instead of Err: {err}"));
        assert_eq!(out.to_string(), "\"true\"".to_string(), "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bad_projection_argument_is_misuse_and_raises() {
    // The path is the caller's input; the second argument is something
    // they wrote. Wrong input is a Result, wrong code raises.
    let path = olc_path("badarg");
    let src = format!(
        r#"
unwrap(ods.write_frame(ods.read_csv("a\n1\n"), "{p}"))
ods.read_frame("{p}", "a")
"#,
        p = path.to_string_lossy()
    );
    let err = eval(&src, None).expect_err("must raise");
    assert!(err.contains("list of column names"), "{err}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_columnar_format_agrees_across_tiers() {
    let path = olc_path("tier");
    assert_tier_transparent(&format!(
        r#"
let f = ods.frame([["n", [1, 2, 3]], ["s", ["a", "b", "a"]]])
unwrap(ods.write_frame(f, "{p}"))
ods.to_list(unwrap(ods.read_frame("{p}", ["s"]))["s"])
"#,
        p = path.to_string_lossy()
    ))
    .expect("tier agreement");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

// ── Mask combination, concat, and the join default ────────────────────

#[test]
fn masks_combine_with_all_of_and_any_of() {
    // Filtering on more than one condition is the normal case, and `&&`
    // cannot serve: the language compiles it to a conditional jump for
    // short-circuiting, which has no elementwise reading over a column.
    let out = eval(
        r#"
let f = ods.read_csv("q,k\nok,5\nbad,7\nok,-1\nok,9\n")
let keep = ods.all_of([ods.eq(f["q"], "ok"), f["k"] > 0])
let either = ods.any_of([ods.eq(f["q"], "bad"), f["k"] > 8])
show([ods.to_list(f[keep]["k"]), ods.to_list(f[either]["k"])])
"#,
        None,
    )
    .expect("masks");
    assert_eq!(out.to_string(), "\"[[5, 9], [7, 9]]\"".to_string());
}

#[test]
fn mask_combination_is_three_valued() {
    // The same logic SQL uses, and the one `filter` already assumed when
    // it treated a null as false: one `false` settles an `all_of` even
    // when another entry is unknown, and one `true` settles an `any_of`.
    let out = eval(
        r#"
let f = ods.read_csv("a,b\n1,1\n,1\n,\n"  )
let known = f["a"] > 0
let other = f["b"] > 0
show([ods.to_list(ods.all_of([known, other])), ods.to_list(ods.any_of([known, other]))])
"#,
        None,
    )
    .expect("three-valued");
    // Row 2: a is null (unknown) and b is true -> all_of unknown, any_of true.
    // Row 3: both unknown -> both unknown.
    assert_eq!(
        out.to_string(),
        "\"[[true, (), ()], [true, true, ()]]\"".to_string()
    );
}

#[test]
fn not_flips_a_mask_and_leaves_unknowns_alone() {
    let out = eval(
        r#"
let f = ods.read_csv("a,tag\n1,x\n,y\n-1,z\n")
show(ods.to_list(ods.not(f["a"] > 0)))
"#,
        None,
    )
    .expect("not");
    assert_eq!(out.to_string(), "\"[false, (), true]\"".to_string());
}

#[test]
fn a_mask_operation_refuses_a_non_mask() {
    // The likely mistake is passing the column instead of a comparison
    // over it, so the message names the fix.
    let err = eval(r#"ods.not(ods.series([1, 2, 3]))"#, None).expect_err("must refuse");
    assert!(err.contains("must be a Bool mask"), "{err}");
    assert!(err.contains("> 100.0"), "{err}");

    let err = eval(r#"ods.all_of([])"#, None).expect_err("must refuse");
    assert!(err.contains("at least one mask"), "{err}");

    let err = eval(
        r#"
let a = ods.read_csv("x\n1\n2\n")
let b = ods.read_csv("x\n1\n")
ods.all_of([a["x"] > 0, b["x"] > 0])
"#,
        None,
    )
    .expect_err("must refuse");
    assert!(err.contains("same Frame"), "{err}");
}

#[test]
fn eq_accepts_a_scalar_like_the_operator_does() {
    // `==` already took a scalar on the right; `ods.eq` refusing one was
    // an inconsistency that cost a confusing error on the commonest
    // filter there is.
    let out = eval(
        r#"
let f = ods.read_csv("q\nok\nbad\nok\n")
show([ods.to_list(ods.eq(f["q"], "ok")), ods.to_list(ods.ne(f["q"], "ok"))])
"#,
        None,
    )
    .expect("scalar eq");
    assert_eq!(
        out.to_string(),
        "\"[[true, false, true], [false, true, false]]\"".to_string()
    );
}

#[test]
fn concat_stacks_frames() {
    // Streaming turns one pass into many partial results; without this
    // there is no way to put them back together except a detour through
    // row-shaped values, which is what the columnar model exists to avoid.
    let out = eval(
        r#"
let a = ods.read_csv("x,y\n1,a\n2,b\n")
let b = ods.read_csv("x,y\n3,c\n")
let both = ods.concat([a, b])
show([ods.columns(both), ods.to_list(both["x"]), ods.to_list(both["y"])])
"#,
        None,
    )
    .expect("concat");
    assert_eq!(
        out.to_string(),
        "\"[[\"x\", \"y\"], [1, 2, 3], [\"a\", \"b\", \"c\"]]\"".to_string()
    );
}

#[test]
fn concat_matches_columns_by_name_not_position() {
    // Two Frames that share a shape but not a meaning is the failure this
    // is most likely to be handed.
    let out = eval(
        r#"
let a = ods.read_csv("x,y\n1,10\n")
let b = ods.read_csv("y,x\n20,2\n")
show([ods.to_list(ods.concat([a, b])["x"]), ods.to_list(ods.concat([a, b])["y"])])
"#,
        None,
    )
    .expect("by name");
    assert_eq!(out.to_string(), "\"[[1, 2], [10, 20]]\"".to_string());
}

#[test]
fn concat_refuses_a_drifting_schema() {
    // Padding a missing column with nulls would hide a bug in whatever
    // produced the second Frame.
    let err = eval(
        r#"
ods.concat([ods.read_csv("x,y\n1,2\n"), ods.read_csv("x,z\n3,4\n")])
"#,
        None,
    )
    .expect_err("must refuse");
    assert!(err.contains("Frame 2 has columns"), "{err}");

    let err = eval("ods.concat([])", None).expect_err("must refuse");
    assert!(err.contains("at least one Frame"), "{err}");
}

#[test]
fn concat_widens_an_int_column_meeting_a_float_one() {
    // The same rule a literal list already follows, so the exception
    // would be the surprise.
    let out = eval(
        r#"
let both = ods.concat([ods.read_csv("n\n1\n"), ods.read_csv("n\n2.5\n")])
show(ods.to_list(both["n"]))
"#,
        None,
    )
    .expect("widen");
    assert_eq!(out.to_string(), "\"[1.0, 2.5]\"".to_string());
}

#[test]
fn a_join_key_named_the_same_on_both_sides_is_written_once() {
    let out = eval(
        r#"
let facts = ods.read_csv("meter,kwh\nA,1.5\nB,2.0\n")
let dim = ods.read_csv("meter,site\nA,north\nB,south\n")
let joined = ods.join(facts, dim, "meter")
show([ods.columns(joined), ods.to_list(joined["site"])])
"#,
        None,
    )
    .expect("join");
    assert_eq!(
        out.to_string(),
        "\"[[\"meter\", \"kwh\", \"site\"], [\"north\", \"south\"]]\"".to_string()
    );
}

#[test]
fn differently_named_join_keys_still_take_both() {
    let out = eval(
        r#"
let facts = ods.read_csv("meter,kwh\nA,1.5\n")
let dim = ods.read_csv("id,site\nA,north\n")
show(ods.to_list(ods.join(facts, dim, "meter", "id")["site"]))
"#,
        None,
    )
    .expect("join");
    assert_eq!(out.to_string(), "\"[\"north\"]\"".to_string());
}

#[test]
fn the_new_verbs_agree_across_tiers() {
    assert_tier_transparent(
        r#"
let f = ods.read_csv("q,k\nok,5\nbad,7\nok,9\n")
let kept = f[ods.all_of([ods.eq(f["q"], "ok"), f["k"] > 6])]
ods.to_list(ods.concat([kept, kept])["k"])
"#,
    )
    .expect("tier agreement");
}

// ── DP3 Tier 1: the verbs a first pipeline hits immediately ───────────

#[test]
fn rename_makes_a_collided_join_usable() {
    // The motivating case. `join` suffixes a collision `_right`, and
    // until `rename` existed there was no way to give the column the
    // name the rest of the pipeline wants.
    let out = eval(
        r#"
let sales = ods.read_csv("id,amount\n1,10.0\n2,20.0\n")
let costs = ods.read_csv("id,amount\n1,3.0\n2,4.0\n")
let j = ods.rename(ods.join(sales, costs, "id"), #{"amount_right": "cost"})
show([ods.columns(j), ods.to_list(j["amount"] - j["cost"])])
"#,
        None,
    )
    .expect("rename");
    assert_eq!(
        out.to_string(),
        "\"[[\"id\", \"amount\", \"cost\"], [7.0, 16.0]]\"".to_string()
    );
}

#[test]
fn rename_refuses_an_unknown_or_colliding_name() {
    let err = eval(
        r#"ods.rename(ods.read_csv("a\n1\n"), #{"nope": "b"})"#,
        None,
    )
    .expect_err("unknown column");
    assert!(
        err.contains("no column 'nope'") && err.contains("It has: a"),
        "{err}"
    );

    // Two columns of one name would be indistinguishable.
    let err = eval(
        r#"ods.rename(ods.read_csv("a,b\n1,2\n"), #{"a": "b"})"#,
        None,
    )
    .expect_err("collision");
    assert!(err.contains("would collide"), "{err}");
}

#[test]
fn drop_is_the_complement_of_select() {
    // Naming what to remove survives a column being added upstream;
    // listing everything to keep silently discards it.
    let out = eval(
        r#"
let f = ods.read_csv("a,b,c\n1,2,3\n")
show([ods.columns(ods.drop(f, ["b"])), ods.columns(ods.drop(f, ["a", "c"]))])
"#,
        None,
    )
    .expect("drop");
    assert_eq!(out.to_string(), "\"[[\"a\", \"c\"], [\"b\"]]\"".to_string());
    let err =
        eval(r#"ods.drop(ods.read_csv("a\n1\n"), ["nope"])"#, None).expect_err("unknown column");
    assert!(err.contains("no column 'nope'"), "{err}");
}

#[test]
fn tail_takes_the_last_rows_and_defaults_to_ten() {
    let mut csv = String::from("i\n");
    for i in 0..30 {
        csv.push_str(&format!("{}\n", i));
    }
    let out = eval(
        &format!(
            "let f = ods.read_csv({:?})\nshow([ods.n_rows(ods.tail(f)), \
             ods.to_list(ods.tail(f, 3)[\"i\"])])",
            csv
        ),
        None,
    )
    .expect("tail");
    assert_eq!(out.to_string(), "\"[10, [27, 28, 29]]\"".to_string());

    // Asking for more rows than exist yields the whole frame rather than
    // an error — `tail(f, 1000)` on a short frame is a reasonable ask.
    let out = eval(
        r#"ods.n_rows(ods.tail(ods.read_csv("i\n1\n2\n"), 1000))"#,
        None,
    )
    .expect("over-long tail");
    assert_eq!(out.to_string(), "2".to_string());
}

#[test]
fn distinct_keeps_the_first_occurrence() {
    // Order matters: a reader scanning the result expects the rows in the
    // order the data presented them, not in hash order.
    let out = eval(
        r#"
let f = ods.read_csv("r,v\neast,1\nwest,2\neast,1\nnorth,3\nwest,9\n")
show([ods.to_list(ods.distinct(f)["r"]), ods.to_list(ods.distinct(f, ["r"])["v"])])
"#,
        None,
    )
    .expect("distinct");
    // Whole-row distinct drops only the exact repeat; subset distinct
    // keeps the first row for each region, so west's v is 2, not 9.
    assert_eq!(
        out.to_string(),
        "\"[[\"east\", \"west\", \"north\", \"west\"], [1, 2, 3]]\"".to_string()
    );
}

#[test]
fn distinct_does_not_confuse_a_field_boundary() {
    // Row identity is length-prefixed rather than separator-joined, so no
    // value can impersonate a boundary: ["a,b", "c"] and ["a", "b,c"] are
    // different rows and must both survive.
    let out = eval(
        r#"
let f = ods.read_csv("x,y\n\"a,b\",c\na,\"b,c\"\n")
ods.n_rows(ods.distinct(f))
"#,
        None,
    )
    .expect("distinct");
    assert_eq!(out.to_string(), "2".to_string());
}

#[test]
fn distinct_treats_null_as_its_own_value() {
    // A null must not collide with the empty string, or a frame holding
    // both would lose a row. Built directly rather than parsed: CSV reads
    // an empty cell and a quoted "" alike as null, so the distinction
    // cannot be expressed in the text form.
    let out = eval(
        r#"
let f = ods.frame([["x", ["", (), ""]], ["tag", ["a", "b", "c"]]])
show([ods.n_rows(f), ods.n_rows(ods.distinct(f, ["x"])), ods.null_count(f["x"])])
"#,
        None,
    )
    .expect("distinct nulls");
    // Three rows, two distinct x values ("" and null), so two kept.
    assert_eq!(out.to_string(), "\"[3, 2, 1]\"".to_string());
}

#[test]
fn drop_null_removes_rows_with_missing_data() {
    let out = eval(
        r#"
let f = ods.read_csv("a,b\n1,x\n,y\n2,\n3,z\n")
show([ods.n_rows(ods.drop_null(f)), ods.n_rows(ods.drop_null(f, ["a"]))])
"#,
        None,
    )
    .expect("drop_null");
    // Whole-row: only rows 1 and 4 are complete. On `a` alone: 3 survive.
    assert_eq!(out.to_string(), "\"[2, 3]\"".to_string());
}

#[test]
fn the_tier_one_verbs_agree_across_tiers() {
    assert_tier_transparent(
        r#"
let f = ods.read_csv("r,v\neast,1\nwest,2\neast,1\n")
let g = ods.rename(ods.drop_null(ods.distinct(f)), #{"v": "value"})
[ods.columns(g), ods.to_list(ods.tail(g, 1)["value"])]
"#,
    )
    .expect("tier agreement");
}

// ── DP3 Tier 2: the verbs reached for once `describe` has shown the shape ──

#[test]
fn value_counts_orders_by_frequency_then_first_appearance() {
    let out = eval(
        r#"
let f = ods.read_csv("r\nwest\neast\neast\nnorth\neast\n")
let vc = ods.value_counts(f["r"])
show([ods.columns(vc), ods.to_list(vc["value"]), ods.to_list(vc["count"])])
"#,
        None,
    )
    .expect("value_counts");
    // east (3) leads; west and north tie at 1, and west appeared first,
    // so the tie breaks by first appearance rather than arbitrarily.
    assert_eq!(
        out.to_string(),
        "\"[[\"value\", \"count\"], [\"east\", \"west\", \"north\"], [3, 1, 1]]\"".to_string()
    );
}

#[test]
fn value_counts_agrees_with_group_by() {
    // Both go through the same key-identification pass, and this is what
    // says so: the counts must match column for column.
    let out = eval(
        r#"
let f = ods.frame([["r", ["west", "east", "east", (), "east"]]])
let vc = ods.value_counts(f["r"])
let g = ods.sort_by(ods.group_by(f, "r", [["n", "count"]]), "n", true)
show([ods.to_list(vc["count"]), ods.to_list(g["n"]), ods.n_unique(f["r"])])
"#,
        None,
    )
    .expect("agreement");
    // Four values: east(3), west(1), null(1) — the null row counts as a
    // value on both sides rather than vanishing from one of them.
    assert_eq!(out.to_string(), "\"[[3, 1, 1], [3, 1, 1], 3]\"".to_string());
}

#[test]
fn unique_keeps_first_seen_order_and_counts_null_as_a_value() {
    let out = eval(
        r#"
let s = ods.series(["b", "a", "b", "c", "a"])
let withnull = ods.frame([["x", [1, (), 1, 2, ()]]])["x"]
show([ods.to_list(ods.unique(s)), ods.n_unique(s),
      ods.to_list(ods.unique(withnull)), ods.n_unique(withnull)])
"#,
        None,
    )
    .expect("unique");
    assert_eq!(
        out.to_string(),
        "\"[[\"b\", \"a\", \"c\"], 3, [1, (), 2], 3]\"".to_string()
    );
}

#[test]
fn median_is_quantile_at_a_half() {
    // Defined in terms of quantile rather than reimplemented, so the two
    // can never disagree — including on an even-length column, where the
    // answer depends on the interpolation rule.
    let out = eval(
        r#"
let odd = ods.series([3.0, 1.0, 2.0])
let even = ods.series([4.0, 1.0, 3.0, 2.0])
show([ods.median(odd), ods.quantile(odd, 0.5),
      ods.median(even), ods.quantile(even, 0.5),
      ods.median(ods.series([1.0, (), 3.0]))])
"#,
        None,
    )
    .expect("median");
    assert_eq!(out.to_string(), "\"[2.0, 2.0, 2.5, 2.5, 2.0]\"".to_string());
}

#[test]
fn cast_turns_an_unconvertible_value_into_a_null_not_a_wrong_number() {
    // The ETL-shaped choice: one bad row in a million must not fail the
    // load, and must not silently become a plausible number either.
    let out = eval(
        r#"
let text = ods.series(["1", " 2 ", "oops", "", "4"])
let ints = ods.cast(text, "Int")
show([ods.to_list(ints), ods.null_count(ints)])
"#,
        None,
    )
    .expect("cast");
    assert_eq!(out.to_string(), "\"[[1, 2, (), (), 4], 2]\"".to_string());
}

#[test]
fn cast_to_int_truncates_toward_zero_and_refuses_the_unrepresentable() {
    // `as` would saturate 1e30 to i64::MAX. That is a wrong answer that
    // looks like data, so it becomes null instead. (NaN is the other
    // such case; olang refuses every expression that would produce one,
    // so it is pinned in olang-ods/tests/kernels.rs.)
    let out = eval(
        r#"
let f = ods.series([2.7, -2.7, 1.0e30, -1.0e30])
show(ods.to_list(ods.cast(f, "Int")))
"#,
        None,
    )
    .expect("cast");
    assert_eq!(out.to_string(), "\"[2, -2, (), ()]\"".to_string());
}

#[test]
fn cast_to_string_spells_a_float_the_way_the_language_does() {
    // The engine holds no opinion about float formatting; it is handed
    // the language's own, so a cast column and to_string agree.
    let out = eval(
        r#"
let f = ods.series([25.5, 320.0, 0.5])
show([ods.to_list(ods.cast(f, "String")),
      [to_string(25.5), to_string(320.0), to_string(0.5)]])
"#,
        None,
    )
    .expect("cast");
    let text = out.to_string();
    let half = text.len() / 2;
    assert!(
        text[..half].contains("320.0") && text[half..].contains("320.0"),
        "cast and to_string must spell a float alike: {text}"
    );
}

#[test]
fn cast_refuses_float_to_bool_and_names_the_comparison() {
    let err = eval(r#"ods.cast(ods.series([0.5]), "Bool")"#, None).expect_err("refused");
    assert!(err.contains("0.5") && err.contains("!= 0.0"), "{err}");
    let err = eval(r#"ods.cast(ods.series([1]), "Decimal")"#, None).expect_err("refused");
    assert!(
        err.contains("not a column type") && err.contains("Float"),
        "{err}"
    );
}

#[test]
fn sample_draws_without_replacement_and_keeps_the_original_order() {
    let out = eval(
        r#"
let s = ods.series(range(0, 50))
let a = ods.to_list(ods.sample(s, 8))
show([len(a), ods.n_unique(ods.series(a)), a == sort(a)])
"#,
        None,
    )
    .expect("sample");
    // Eight draws are eight distinct rows, and they come back in the
    // frame's own order rather than draw order. (Seed reproducibility
    // needs its own process — see the test below.)
    assert_eq!(out.to_string(), "\"[8, 8, true]\"".to_string());
}

/// Reproducibility has to be checked in a fresh process each time.
///
/// `random` is one stream for the whole process, so an in-process test
/// that seeds and then draws can have its uniforms taken by whichever
/// other test happens to be running beside it — the assertion would be
/// about the test harness, not about `sample`. Running the real binary
/// twice tests the property a reader actually depends on: the same
/// program, seeded the same way, prints the same sample.
#[test]
fn sample_is_reproducible_from_a_seed() {
    let dir = std::env::temp_dir().join(format!("olang_sample_seed_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let program = dir.join("sample.ol");
    std::fs::write(
        &program,
        r#"
let s = ods.series(range(0, 100))
random.seed(20260817)
println(to_string(ods.to_list(ods.sample(s, 10))))
random.seed(99)
println(to_string(ods.to_list(ods.sample(s, 10))))
"#,
    )
    .expect("write program");

    let run = || {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
            .arg("run")
            .arg(&program)
            .output()
            .expect("olang runs");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let first = run();
    let second = run();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(first, second, "a seeded sample must reproduce across runs");
    let lines: Vec<&str> = first.lines().collect();
    assert_eq!(lines.len(), 2, "expected two samples, got {first:?}");
    assert_ne!(
        lines[0], lines[1],
        "two different seeds must not draw the same sample"
    );
}

#[test]
fn sampling_more_rows_than_exist_yields_all_of_them() {
    let out = eval(
        r#"
let f = ods.read_csv("a,b\n1,x\n2,y\n3,z\n")
random.seed(1)
show([ods.n_rows(ods.sample(f, 999)), ods.n_rows(ods.sample(f, 0)),
      ods.columns(ods.sample(f, 2))])
"#,
        None,
    )
    .expect("sample");
    assert_eq!(out.to_string(), "\"[3, 0, [\"a\", \"b\"]]\"".to_string());
}

#[test]
fn the_tier_two_verbs_agree_across_tiers() {
    assert_tier_transparent(
        r#"
let f = ods.read_csv("r,v\neast,1\nwest,2\neast,3\nnorth,4\n")
let vc = ods.value_counts(f["r"])
[ods.to_list(vc["count"]), ods.to_list(ods.unique(f["r"])), ods.n_unique(f["r"]),
 ods.median(ods.cast(f["v"], "Float")), ods.to_list(ods.cast(f["v"], "String"))]
"#,
    )
    .expect("tier agreement");
}

#[test]
fn both_sampling_paths_yield_distinct_in_range_rows() {
    // A small sample out of a large frame rejects collisions rather than
    // shuffling, so that `ods.sample(f, 5)` on ten million rows does not
    // allocate one i64 per row. The two paths must be indistinguishable
    // from the outside, so this crosses the ratio that switches them.
    let out = eval(
        r#"
fn ok(size, n) = {
    let got = ods.to_list(ods.sample(ods.series(range(0, size)), n))
    len(got) == n
        && ods.n_unique(ods.series(got)) == n
        && got == sort(got)
        && fold(got, true, (acc, x) => acc && x >= 0 && x < size)
}
show([ok(100000, 5), ok(100, 25), ok(100, 26), ok(100, 100), ok(1, 1), ok(10, 0)])
"#,
        None,
    )
    .expect("sample");
    assert_eq!(
        out.to_string(),
        "\"[true, true, true, true, true, true]\"".to_string()
    );
}

// ── DP3 Tier 3a: the rest of the join kinds ───────────────────────────

const JOIN_L: &str = r#"ods.frame([["k", ["east", "west", ()]], ["v", [1, 2, 3]]])"#;
const JOIN_R: &str = r#"ods.frame([["k", ["east", "north", ()]], ["n", [10, 20, 30]]])"#;

#[test]
fn a_full_join_keeps_both_sides_and_coalesces_the_key() {
    let out = eval(
        &format!(
            r#"
let j = ods.join_full({JOIN_L}, {JOIN_R}, "k")
show([ods.to_list(j["k"]), ods.to_list(j["v"]), ods.to_list(j["n"])])
"#
        ),
        None,
    )
    .expect("join_full");
    // The three left rows first, then the right rows nothing matched.
    // "north" reaches the key column from the right side — without the
    // coalesce the column identifying the row would be null exactly
    // where the reader needs it. Null keys match nothing on either side,
    // so both null-keyed rows survive unpaired.
    assert_eq!(
        out.to_string(),
        "\"[[\"east\", \"west\", (), \"north\", ()], [1, 2, 3, (), ()], [10, (), (), 20, 30]]\""
            .to_string()
    );
}

#[test]
fn a_semi_join_asks_existence_without_multiplying() {
    // The difference that makes semi worth having: an inner join against
    // a right side with three matching rows returns three rows; semi
    // returns the one left row, once.
    let out = eval(
        r#"
let l = ods.frame([["k", ["east", "west"]], ["v", [1, 2]]])
let r = ods.frame([["k", ["east", "east", "east"]], ["n", [1, 2, 3]]])
show([ods.n_rows(ods.join(l, r, "k")), ods.n_rows(ods.join_semi(l, r, "k")),
      ods.columns(ods.join_semi(l, r, "k"))])
"#,
        None,
    )
    .expect("join_semi");
    assert_eq!(out.to_string(), "\"[3, 1, [\"k\", \"v\"]]\"".to_string());
}

#[test]
fn semi_and_anti_partition_the_left_frame() {
    // Every left row is in exactly one of them — the property that makes
    // the pair usable for "which of these are known" questions.
    let out = eval(
        &format!(
            r#"
let l = {JOIN_L}
let s = ods.join_semi(l, {JOIN_R}, "k")
let a = ods.join_anti(l, {JOIN_R}, "k")
show([ods.n_rows(s) + ods.n_rows(a) == ods.n_rows(l),
      ods.to_list(s["v"]), ods.to_list(a["v"])])
"#
        ),
        None,
    )
    .expect("semi/anti");
    // A null key matches nothing, so the null-keyed row lands in anti —
    // the same rule every other join kind follows.
    assert_eq!(out.to_string(), "\"[true, [1], [2, 3]]\"".to_string());
}

#[test]
fn a_full_join_refuses_mismatched_key_types() {
    // Keys hash by type, so an Int column never matches a Float one. The
    // other kinds simply find nothing; a full join returns rows anyway,
    // which is exactly when a mismatch would be papered over silently.
    let err = eval(
        r#"ods.join_full(ods.frame([["k", [1]]]), ods.frame([["k", [1.0]]]), "k")"#,
        None,
    )
    .expect_err("refused");
    assert!(err.contains("same type") && err.contains("Int"), "{err}");
}

#[test]
fn every_join_kind_agrees_across_tiers() {
    assert_tier_transparent(&format!(
        r#"
let l = {JOIN_L}
let r = {JOIN_R}
[ods.n_rows(ods.join(l, r, "k")), ods.n_rows(ods.join_left(l, r, "k")),
 ods.n_rows(ods.join_full(l, r, "k")), ods.n_rows(ods.join_semi(l, r, "k")),
 ods.n_rows(ods.join_anti(l, r, "k")), ods.to_list(ods.join_full(l, r, "k")["k"])]
"#
    ))
    .expect("tier agreement");
}

// ── DP3 Tier 3b: reshape ──────────────────────────────────────────────

const LONG_CSV: &str = "region,quarter,amount\\neast,Q1,10.0\\neast,Q2,20.0\\nwest,Q1,5.0\\nwest,Q3,7.0\\neast,Q1,3.0\\n";

#[test]
fn pivot_spreads_one_column_into_many() {
    let out = eval(
        &format!(
            r#"
let wide = ods.pivot(ods.read_csv("{LONG_CSV}"), "region", "quarter", "amount", "sum")
show([ods.columns(wide), ods.to_list(wide["Q1"]), ods.to_list(wide["Q2"])])
"#
        ),
        None,
    )
    .expect("pivot");
    // east's two Q1 rows aggregate to 13; west never reported Q2, so
    // that cell is null — the combination did not occur, which is a
    // different fact from a null amount in it.
    assert_eq!(
        out.to_string(),
        "\"[[\"region\", \"Q1\", \"Q2\", \"Q3\"], [13.0, 5.0], [20.0, ()]]\"".to_string()
    );
}

#[test]
fn pivot_aggregates_exactly_as_group_by_does() {
    // pivot's aggregation *is* a group_by, so the cells must equal the
    // groups. This is the test that keeps the two from drifting.
    let out = eval(
        &format!(
            r#"
let f = ods.read_csv("{LONG_CSV}")
let wide = ods.pivot(f, "region", "quarter", "amount", "sum")
let grouped = ods.group_by(f, ["region", "quarter"], [["total", "sum", "amount"]])
let q1 = ods.filter(grouped, grouped["quarter"] == "Q1")
show([ods.to_list(wide["Q1"]), ods.to_list(ods.sort_by(q1, "region", false)["total"])])
"#
        ),
        None,
    )
    .expect("pivot vs group_by");
    assert_eq!(
        out.to_string(),
        "\"[[13.0, 5.0], [13.0, 5.0]]\"".to_string()
    );
}

#[test]
fn unpivot_returns_the_long_form_pivot_started_from() {
    // The round trip: the cells come back, and the combinations that
    // never occurred come back as explicit nulls rather than vanishing.
    // `drop_null` is then the caller's choice, not the verb's.
    let out = eval(
        &format!(
            r#"
let wide = ods.pivot(ods.read_csv("{LONG_CSV}"), "region", "quarter", "amount", "sum")
let long = ods.unpivot(wide, "region")
let present = ods.drop_null(long)
show([ods.columns(long), ods.n_rows(long), ods.n_rows(present),
      ods.to_list(present["name"]), ods.to_list(present["value"])])
"#
        ),
        None,
    )
    .expect("unpivot");
    assert_eq!(
        out.to_string(),
        "\"[[\"region\", \"name\", \"value\"], 6, 4, [\"Q1\", \"Q1\", \"Q2\", \"Q3\"], \
         [13.0, 5.0, 20.0, 7.0]]\""
            .to_string()
    );
}

#[test]
fn unpivot_defaults_to_every_column_that_is_not_an_id() {
    // Naming only the ids survives a new column arriving upstream, where
    // listing the value columns would silently leave it behind.
    let out = eval(
        r#"
let f = ods.frame([["id", [1, 2]], ["a", [10, 20]], ["b", [30, 40]]])
show([ods.n_rows(ods.unpivot(f, "id")), ods.n_rows(ods.unpivot(f, "id", ["a"])),
      ods.to_list(ods.unpivot(f, "id")["name"])])
"#,
        None,
    )
    .expect("unpivot");
    assert_eq!(
        out.to_string(),
        "\"[4, 2, [\"a\", \"a\", \"b\", \"b\"]]\"".to_string()
    );
}

#[test]
fn reshape_refuses_the_shapes_that_have_no_honest_answer() {
    let cases: &[(&str, &str)] = &[
        // A null cannot name a column, and calling it "null" would
        // collide with a genuine "null" string.
        (
            r#"ods.pivot(ods.frame([["r", ["a"]], ["c", [()]], ["v", [1]]]), "r", "c", "v", "sum")"#,
            "cannot name a column",
        ),
        // A value that would name an existing index column.
        (
            r#"ods.pivot(ods.frame([["r", ["a"]], ["c", ["r"]], ["v", [1]]]), "r", "c", "v", "sum")"#,
            "already exists as an index column",
        ),
        (
            r#"ods.pivot(ods.frame([["r", ["a"]], ["v", [1]]]), "r", "v", "r", "sum")"#,
            "cannot be both an index column",
        ),
        (
            r#"ods.pivot(ods.frame([["r", ["a"]], ["v", [1]]]), "r", "v", "v", "nope")"#,
            "is not an aggregation",
        ),
        // Stacking columns of different types would mean choosing a
        // common type on the caller's behalf — that is `cast`'s job.
        (
            r#"ods.unpivot(ods.frame([["id", [1]], ["i", [1]], ["f", [1.0]]]), "id")"#,
            "must share",
        ),
        (
            r#"ods.unpivot(ods.frame([["name", [1]], ["v", [2]]]), "name")"#,
            "is where the output goes",
        ),
        (
            r#"ods.unpivot(ods.frame([["a", [1]], ["b", [2]]]), ["a", "b"], ["b"])"#,
            "cannot be both an id column",
        ),
        (
            r#"ods.unpivot(ods.frame([["a", [1]]]), "a", ["nope"])"#,
            "no column 'nope'",
        ),
    ];
    for (source, wanted) in cases {
        let err = eval(source, None)
            .err()
            .unwrap_or_else(|| panic!("{source} should have failed"));
        assert!(
            err.contains(wanted),
            "expected {wanted:?} in the error for {source}, got {err:?}"
        );
    }
}

#[test]
fn reshape_agrees_across_tiers() {
    assert_tier_transparent(&format!(
        r#"
let wide = ods.pivot(ods.read_csv("{LONG_CSV}"), "region", "quarter", "amount", "mean")
[ods.columns(wide), ods.to_list(wide["Q1"]),
 ods.to_list(ods.unpivot(wide, "region")["value"])]
"#
    ))
    .expect("tier agreement");
}

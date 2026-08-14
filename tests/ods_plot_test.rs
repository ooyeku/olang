//! Phase 4 integration tests: the plot namespace from olang source —
//! SVG text out, options handling, null behavior, and composition with
//! the Frame pipeline.

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

fn eval(source: &str) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

fn as_string(v: Value) -> String {
    match v {
        Value::String(s) => s.as_ref().clone(),
        other => panic!("expected String, got {:?}", other),
    }
}

#[test]
fn line_chart_is_svg_text_with_escaped_title() {
    let svg = as_string(
        eval(
            r#"
            let x = ods.series(1..8)
            let y = x * x
            plot.line(x, y, #{ "title": "y < x*x & more", "x_label": "x", "y_label": "y" })
            "#,
        )
        .unwrap(),
    );
    assert!(
        svg.starts_with("<svg xmlns"),
        "got: {}",
        &svg[..60.min(svg.len())]
    );
    assert!(svg.ends_with("</svg>"));
    assert!(svg.contains("y &lt; x*x &amp; more"));
    assert!(svg.contains("stroke-width=\"2\""));
}

#[test]
fn multi_series_scatter_hist_bar() {
    let result = eval(
        r#"
        let x = ods.series([1.0, 2.0, 3.0, 4.0])
        let a = ods.series([1.0, 2.0, 3.0, 4.0])
        let b = ods.series([4.0, 3.0, 2.0, 1.0])
        let multi = plot.lines(x, [["up", a], ["down", b]], #{})
        let sc = plot.scatter(x, a, #{})
        let bars = plot.bar(["q1", "q2"], ods.series([10, 20]), #{ "title": "rev" })
        let h = plot.hist(stats.norm.sample(500, 0.0, 1.0), 20, #{})
        [
            str.contains(multi, "up") && str.contains(multi, "down"),
            str.contains(sc, "circle"),
            str.contains(bars, "q1"),
            str.contains(h, "svg")
        ]
        "#,
    )
    .unwrap();
    assert_eq!(
        result,
        Value::List(
            vec![
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true),
            ]
            .into()
        )
    );
}

#[test]
fn frame_pipeline_composes_into_bars() {
    // The dataproc shape: CSV → group_by → chart.
    let result = eval(
        r#"
        let df = ods.read_csv("region,revenue\neast,10.5\nwest,20.0\neast,30.0\n")
        let by_region = df |> ods.group_by("region", [["total", "sum", "revenue"]])
        let svg = plot.bar(ods.column(by_region, "region"), ods.column(by_region, "total"), #{})
        [str.contains(svg, "east"), str.contains(svg, "west")]
        "#,
    )
    .unwrap();
    assert_eq!(
        result,
        Value::List(vec![Value::Boolean(true), Value::Boolean(true)].into())
    );
}

#[test]
fn null_pairs_drop_in_xy_and_refuse_in_bars() {
    let result = eval(
        r#"
        let missing = map_get(#{}, "absent")
        let x = ods.series([1.0, 2.0, 3.0])
        let y = ods.series([1.0, missing, 3.0])
        let svg = plot.scatter(x, y, #{})
        [str.contains(svg, "circle")]
        "#,
    )
    .unwrap();
    assert_eq!(result, Value::List(vec![Value::Boolean(true)].into()));

    let err = eval(
        r#"
        let missing = map_get(#{}, "absent")
        plot.bar(["a", "b"], ods.series([1.0, missing]), #{})
        "#,
    )
    .unwrap_err();
    assert!(err.contains("must not contain nulls"), "got: {}", err);
}

#[test]
fn area_grouped_stacked_heatmap_box() {
    let result = eval(
        r#"
        let x = ods.series([1.0, 2.0, 3.0])
        let y = ods.series([2.0, 5.0, 3.0])
        let a = plot.area(x, y, #{})
        let labels = ["q1", "q2"]
        let pairs = [["east", ods.series([10, 20])], ["west", ods.series([5, 15])]]
        let grouped = plot.bars(labels, pairs, #{})
        let stacked = plot.stacked(labels, pairs, #{})
        let hm = plot.heatmap(["mon", "tue"], ["am", "pm"], [[1, 2], [3, 4]], #{})
        let bx = plot.box([["east", ods.series([1.0, 5.0, 3.0, 2.0])]], #{})
        [
            str.contains(a, "linearGradient"),
            str.contains(grouped, "east") && str.contains(grouped, "west"),
            str.contains(stacked, "east"),
            str.contains(hm, "mon") && str.contains(hm, "am"),
            str.contains(bx, "east")
        ]
        "#,
    )
    .unwrap();
    assert_eq!(result, Value::List(vec![Value::Boolean(true); 5].into()));
}

#[test]
fn theme_and_responsive_options() {
    let svg = as_string(
        eval(
            r#"
            let x = ods.series([1.0, 2.0])
            plot.line(x, x, #{ "theme": "dark", "responsive": true })
            "#,
        )
        .unwrap(),
    );
    assert!(svg.contains("#101720"), "dark surface missing");
    assert!(
        svg.starts_with(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" style=\"width:100%;height:auto\""
        ),
        "responsive sizing missing from the svg tag"
    );

    let err = eval(r#"plot.line(ods.series([1.0]), ods.series([1.0]), #{ "theme": "sepia" })"#)
        .unwrap_err();
    assert!(err.contains("\"light\" or \"dark\""), "got: {}", err);

    let err = eval(r#"plot.stacked(["a"], [["s", ods.series([0 - 1])]], #{})"#).unwrap_err();
    assert!(err.contains("non-negative"), "got: {}", err);
}

#[test]
fn json_records_round_trip_into_frames_and_charts() {
    // The browser shape: dom.fetch_json delivers parsed records; a
    // frame and a chart are one call each. Nulls survive the trip.
    let result = eval(
        r#"
        let body = "[{\"region\":\"east\",\"rev\":10},{\"region\":\"west\",\"rev\":20},{\"region\":\"east\",\"rev\":null}]"
        let records = unwrap(json.parse(body))
        let df = ods.frame_from_records(records)
        let by = df |> ods.group_by("region", [["n", "count", "rev"]])
        let svg = plot.bar(ods.column(by, "region"), ods.column(by, "n"), #{})
        let back = unwrap(json.stringify(ods.to_records(df)))
        [ods.n_rows(df) == 3, str.contains(svg, "east"), str.contains(back, "west")]
        "#,
    )
    .unwrap();
    assert_eq!(result, Value::List(vec![Value::Boolean(true); 3].into()));
}

#[test]
fn color_system_options() {
    let result = eval(
        r##"
let vary = plot.bar(["a", "b", "c"], ods.series([1, 2, 3]), #{ "theme": "dark", "vary": true })
let own = plot.bar(["a"], ods.series([1]), #{ "colors": ["#123456"] })
let hm = plot.heatmap(["m"], ["r", "s"], [[1], [2]], #{ "scale": "diverging", "theme": "dark" })
let x = ods.series([1.0, 2.0])
let xy = plot.xy([["p", "scatter", x, x, ["#ff0000", "#00ff00"]]], #{})
[
    str.contains(vary, "#3ddc97") && str.contains(vary, "#5aa9e6") && str.contains(vary, "#f4b84c"),
    str.contains(own, "#123456"),
    str.contains(hm, "#5aa9e6") || str.contains(hm, "#f4b84c"),
    str.contains(xy, "#ff0000") && str.contains(xy, "#00ff00"),
    str.contains(plot.ramp("thermal", 1.0), "#"),
    plot.ramp("diverging", 0.5) == "#121a24"
]"##,
    )
    .unwrap();
    assert_eq!(result, Value::List(vec![Value::Boolean(true); 6].into()));

    let err = eval(r#"plot.heatmap(["m"], ["r"], [[1]], #{ "scale": "sepia" })"#).unwrap_err();
    assert!(err.contains("thermal"), "got: {}", err);
    let err = eval(r##"plot.xy([["p", "scatter", ods.series([1.0]), ods.series([1.0]), ["#f00", "#0f0"]]], #{})"##)
        .unwrap_err();
    assert!(
        err.contains("lengths differ") || err.contains("mismatch") || err.contains("Length"),
        "got: {}",
        err
    );
}

#[test]
fn option_errors_are_informative() {
    let err = eval(r#"plot.line(ods.series([1.0]), ods.series([1.0]), #{ "tite": "typo" })"#)
        .unwrap_err();
    assert!(err.contains("unknown option 'tite'"), "got: {}", err);

    let err =
        eval(r#"plot.line(ods.series([1.0]), ods.series([1.0]), #{ "width": 5 })"#).unwrap_err();
    assert!(err.contains("between 100 and 4000"), "got: {}", err);

    let err = eval(r#"plot.line(ods.series([1.0]), 3, #{})"#).unwrap_err();
    assert!(err.contains("must be a Series"), "got: {}", err);
}

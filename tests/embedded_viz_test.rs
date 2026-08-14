//! The embedded `viz` module: chart specs as values, compiled to plot
//! SVG. Pure given records, so the grammar is testable natively — the
//! browser only changes where the SVG lands.

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

const ROWS: &str = r#"
let rows = [
    #{ "day": 1, "value": 3.0, "kind": "a" }, #{ "day": 2, "value": 5.0, "kind": "a" },
    #{ "day": 1, "value": 2.0, "kind": "b" }, #{ "day": 2, "value": 6.0, "kind": "b" }
]
"#;

fn assert_all_true(source: &str, n: usize) {
    let result = eval(source).unwrap();
    assert_eq!(result, Value::List(vec![Value::Boolean(true); n].into()));
}

#[test]
fn color_encoding_splits_series() {
    assert_all_true(
        &format!(
            r##"use viz
{ROWS}
let svg = viz.chart(#{{ "data": rows, "mark": "line", "x": "day", "y": "value",
    "color": "kind", "theme": "dark" }})
[
    str.contains(svg, ">a<") && str.contains(svg, ">b<"),
    str.contains(svg, "#0b0e14")
]"##
        ),
        2,
    );
}

#[test]
fn layers_compose_marks_over_shared_scales() {
    assert_all_true(
        &format!(
            r##"use viz
{ROWS}
let svg = viz.chart(#{{ "data": rows, "layers": [
    #{{ "mark": "area", "x": "day", "y": "value" }},
    #{{ "mark": "line", "x": "day", "y": "value" }},
    #{{ "mark": "point", "x": "day", "y": "value" }}
]}})
[
    str.contains(svg, "fill-opacity=\"0.22\""),
    str.contains(svg, "circle"),
    str.contains(svg, "stroke-width=\"2\"")
]"##
        ),
        3,
    );
}

#[test]
fn bar_marks_pivot_and_stack() {
    assert_all_true(
        &format!(
            r##"use viz
{ROWS}
let grouped = viz.chart(#{{ "data": rows, "mark": "bar", "x": "day", "y": "value", "color": "kind" }})
let stacked = viz.chart(#{{ "data": rows, "mark": "bar", "x": "day", "y": "value",
    "color": "kind", "stack": true }})
let plain = viz.chart(#{{ "data": rows, "mark": "bar", "x": "kind", "y": "value" }})
[
    str.contains(grouped, ">a<") && str.contains(grouped, ">b<"),
    str.contains(stacked, "<rect"),
    str.contains(plain, ">a<")
]"##
        ),
        3,
    );
}

#[test]
fn box_hist_and_frame_data() {
    assert_all_true(
        &format!(
            r##"use viz
{ROWS}
let bx = viz.chart(#{{ "data": rows, "mark": "box", "x": "kind", "y": "value" }})
let hs = viz.chart(#{{ "data": rows, "mark": "hist", "y": "value", "bins": 4 }})
let df = ods.frame_from_records(rows)
let ff = viz.chart(#{{ "data": df, "mark": "bar", "x": "kind", "y": "value" }})
[
    str.contains(bx, ">a<") && str.contains(bx, ">b<"),
    str.contains(hs, "<svg"),
    str.contains(ff, ">a<")
]"##
        ),
        3,
    );
}

#[test]
fn interactive_passes_through_to_marks() {
    assert_all_true(
        &format!(
            r##"use viz
{ROWS}
let sc = viz.chart(#{{ "data": rows, "mark": "point", "x": "day", "y": "value",
    "interactive": true }})
let br = viz.chart(#{{ "data": rows, "mark": "bar", "x": "kind", "y": "value",
    "color": "kind", "stack": true, "interactive": true }})
let plain = viz.chart(#{{ "data": rows, "mark": "point", "x": "day", "y": "value" }})
[
    str.contains(sc, "data-x=") && str.contains(sc, "data-y="),
    str.contains(br, "data-s=") && str.contains(br, "data-value="),
    str.contains(plain, "data-x=") == false
]"##
        ),
        3,
    );
}

#[test]
fn plot_xy_rejects_unknown_marks() {
    let err = eval(
        r#"
let x = ods.series([1.0])
plot.xy([["s", "sparkles", x, x]], #{})
"#,
    )
    .unwrap_err();
    assert!(err.contains("mark must be"), "got: {}", err);
}

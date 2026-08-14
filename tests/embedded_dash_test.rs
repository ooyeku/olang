//! The embedded `dash` module: dashboard shells as pure HTML strings —
//! KPI tiles, cards, the grid, and the kit stylesheet, all testable
//! natively because nothing here touches a browser.

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

fn assert_all_true(source: &str, n: usize) {
    let result = eval(source).unwrap();
    assert_eq!(result, Value::List(vec![Value::Boolean(true); n].into()));
}

#[test]
fn kpi_tiles_escape_and_structure() {
    assert_all_true(
        r##"use dash
let tile = dash.kpi("open <issues>", "12", "+3 & counting")
let bare = dash.kpi("points", 42, "")
[
    str.contains(tile, "open &lt;issues&gt;"),
    str.contains(tile, ">12<") && str.contains(tile, "+3 &amp; counting"),
    str.contains(bare, ">42<") && str.contains(bare, "dash-note") == false
]"##,
        3,
    );
}

#[test]
fn cards_and_grid_compose() {
    assert_all_true(
        r##"use dash
let c = dash.card("by <status>", "<div id=\"mount\"></div>")
let w = dash.wide("trend", "<svg></svg>")
let g = dash.grid([dash.kpi("a", "1", ""), c, w], 4)
[
    str.contains(c, "by &lt;status&gt;") && str.contains(c, "<div id=\"mount\">"),
    str.contains(w, "dash-wide"),
    str.contains(g, "repeat(4,1fr)") && str.contains(g, "dash-kpi"),
    str.contains(dash.styles(), ".dash-grid{")
]"##,
        4,
    );
}

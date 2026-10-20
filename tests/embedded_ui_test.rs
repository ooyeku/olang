//! `ui` — the embedded declarative view module. The pure half (tree
//! building and HTML rendering) runs anywhere; the reconciling half
//! needs a page and is proven in playground/dom_harness.mjs.

use olang::Value;
use olang::interpreter::Interpreter;
use olang::parser::Parser;

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

#[test]
fn trees_render_to_escaped_html() {
    let v = eval(
        r#"
use ui { h, hk, html }
html(h("div", #{ "class": "card" }, [
    h("h1", #{}, ["Hi <there> & \"you\""]),
    hk("r1", "p", #{ "data-row": "1" }, ["first"]),
    h("br", #{}, []),
    h("ul", #{}, [h("li", #{}, ["a"]), h("li", #{}, ["b"])])
]))
"#,
    );
    assert_eq!(
        v,
        Value::String(
            "<div class=\"card\"><h1>Hi &lt;there&gt; &amp; &quot;you&quot;</h1>\
             <p data-row=\"1\">first</p><br /><ul><li>a</li><li>b</li></ul></div>"
                .to_string()
                .into()
        )
    );
}

#[test]
fn attrs_render_in_their_order_and_numbers_unquote() {
    let v = eval(
        r#"
use ui { h, html }
html(h("td", #{ "colspan": 2, "b": "x", "a": "y" }, [show(42)]))
"#,
    );
    assert_eq!(
        v,
        Value::String(
            "<td colspan=\"2\" b=\"x\" a=\"y\">42</td>"
                .to_string()
                .into()
        )
    );
}

#[test]
fn escaping_covers_the_dangerous_four() {
    let v = eval(
        r#"
use ui { esc }
esc("<script>\"&\"</script>")
"#,
    );
    assert_eq!(
        v,
        Value::String(
            "&lt;script&gt;&quot;&amp;&quot;&lt;/script&gt;"
                .to_string()
                .into()
        )
    );
}

fn eval_err(src: &str) -> String {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new()
        .eval_program(program)
        .expect_err("must raise")
        .to_string()
}

#[test]
fn a_control_is_a_node_that_renders_a_real_html_control() {
    let v = eval(
        r#"
use ui
let s = ui.control("slider", 3, #{ "min": 1, "max": 10, "step": 1 })
[ui.html(s), s.control,
 ui.html(ui.control("select", "b", #{ "options": ["a", "b"] })),
 ui.html(ui.control("date", "2026-10-16", #{})),
 ui.html(ui.control("text", "a <b>", #{})),
 ui.html(ui.control("toggle", true, #{}))]
"#,
    );
    let shown: Vec<String> = match &v {
        Value::List(items) => items
            .iter()
            .map(|i| match i {
                Value::String(t) => t.to_string(),
                other => other.to_string(),
            })
            .collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        shown,
        vec![
            "<input type=\"range\" min=\"1\" max=\"10\" value=\"3\" step=\"1\" />",
            "slider",
            "<select><option value=\"a\">a</option><option value=\"b\" selected=\"selected\">b</option></select>",
            "<input type=\"date\" value=\"2026-10-16\" />",
            "<input type=\"text\" value=\"a &lt;b&gt;\" />",
            "<input type=\"checkbox\" checked=\"checked\" />",
        ]
    );
}

#[test]
fn with_no_host_a_control_is_its_value() {
    let v = eval(
        r#"
use ui
let chart = plot.line(ods.series([1, 2]), ods.series([3, 4]), #{})
[ui.slider(0.08, 0.0, 0.3, 0.01), ui.slider(3, 1, 10), ui.select("west", ["east", "west"]),
 ui.date("2026-10-16"), ui.text("hi"), ui.toggle(false), ui.brush(chart), ui.brush(chart, [1, 2])]
"#,
    );
    assert_eq!(
        v.to_string(),
        "[0.08, 3, \"west\", \"2026-10-16\", \"hi\", false, (), [1.0, 2.0]]"
    );
}

#[test]
fn a_brush_takes_a_gg_plot_and_keeps_it_as_its_chart() {
    let v = eval(
        r#"
use ui
use gg
let p = gg.plot(ods.frame([["x", ods.series([1.0, 2.0])], ["y", ods.series([3.0, 4.0])]]), gg.aes(x: "x", y: "y")) |> gg.point()
let node = ui.control("brush", [1.0, 1.5], #{ "chart": p })
[ui.brush(p), ui.brush(p, [1, 1.5]), node.spec.chart == p, ui.html(node)]
"#,
    );
    assert_eq!(
        v.to_string(),
        "[(), [1.0, 1.5], true, \"<div class=\\\"brush\\\" data-x0=\\\"1.0\\\" data-x1=\\\"1.5\\\"></div>\"]"
    );
}

#[test]
fn a_control_refuses_a_value_it_cannot_hold() {
    for (src, says) in [
        ("ui.slider(0.5, 0.0, 0.3)", "outside [0.0, 0.3]"),
        ("ui.slider(1, 2, 1)", "is above max"),
        ("ui.slider(\"a\", 0, 1)", "are numbers"),
        ("ui.slider(1, 0, 2, 0)", "step is a positive number"),
        ("ui.select(\"north\", [\"east\"])", "not one of the options"),
        ("ui.select(\"a\", [])", "non-empty list"),
        ("ui.date(\"16/10/2026\")", "YYYY-MM-DD"),
        ("ui.text(3)", "is a String"),
        ("ui.toggle(1)", "is a Bool"),
        ("ui.brush(\"<svg></svg>\")", "carries its description"),
        ("ui.brush(#{ \"x\": 1 })", "a gg plot or a plot chart's SVG"),
        ("ui.control(\"knob\", 1, #{})", "no control is a knob"),
    ] {
        let err = eval_err(&format!("use ui\n{}", src));
        assert!(err.contains(says), "{src}: {err}");
    }
}

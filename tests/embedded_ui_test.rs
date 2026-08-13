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
fn attrs_render_in_stable_order_and_numbers_unquote() {
    let v = eval(
        r#"
use ui { h, html }
html(h("td", #{ "colspan": 2, "b": "x", "a": "y" }, [show(42)]))
"#,
    );
    assert_eq!(
        v,
        Value::String(
            "<td a=\"y\" b=\"x\" colspan=\"2\">42</td>"
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

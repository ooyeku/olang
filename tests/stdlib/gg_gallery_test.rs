//! The gg gallery against its golden SVGs: every plot of
//! tests/fixtures/gg/gallery.ol rendered in light and dark and compared
//! byte for byte with tests/golden/gg/<name>-<appearance>.svg.
//! `GG_BLESS=1` writes them again (look at each before committing:
//! the scratch gallery script renders them to PNG).

use olang::ast::Value;
use olang::{Interpreter, Parser};
use std::path::PathBuf;

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn eval(source: String) -> Result<Value, String> {
    with_big_stack(move || {
        let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
        Interpreter::new()
            .eval_program(program)
            .map_err(|e| e.to_string())
    })
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn gallery() -> Vec<(String, String)> {
    let src =
        std::fs::read_to_string(root().join("tests/fixtures/gg/gallery.ol")).expect("gallery");
    let program = format!(
        "{}\nfold(gallery(), [], (acc, item) => acc + map([\"light\", \"dark\"], (app) => [item[0] + \"-\" + app, gg.svg(item[1] |> gg.theme(appearance: app))]))\n",
        src
    );
    let v = eval(program).expect("the gallery renders");
    let Value::List(ref items) = v else {
        panic!("the gallery is a list")
    };
    items
        .iter()
        .map(|it| match it {
            Value::List(p) => match (&p[0], &p[1]) {
                (Value::String(a), Value::String(b)) => (a.to_string(), b.to_string()),
                _ => panic!("[name, svg]"),
            },
            _ => panic!("[name, svg]"),
        })
        .collect()
}

#[test]
fn every_gallery_plot_matches_its_golden_svg() {
    let dir = root().join("tests/golden/gg");
    let bless = std::env::var("GG_BLESS").is_ok();
    if bless {
        std::fs::create_dir_all(&dir).expect("golden dir");
    }
    let plots = gallery();
    assert!(plots.len() >= 50, "{} plots", plots.len());
    let mut differ = Vec::new();
    for (name, svg) in &plots {
        assert!(
            svg.starts_with("<svg") && svg.ends_with("</svg>"),
            "{name} is an svg"
        );
        assert!(
            svg.contains("<metadata class=\"olang-chart\">"),
            "{name} carries its description"
        );
        let path = dir.join(format!("{name}.svg"));
        if bless {
            std::fs::write(&path, svg).expect("write golden");
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(g) if g == *svg => {}
            Ok(_) => differ.push(format!("{name} differs")),
            Err(_) => differ.push(format!("{name} has no golden (GG_BLESS=1)")),
        }
    }
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

#[test]
fn a_plot_spec_is_pure_data_and_its_scene_says_its_marks() {
    let v = eval(
        r#"use gg
let f = ods.frame([["x", ods.series([1.0, 2.0, 3.0])], ["y", ods.series([2.0, 1.0, 3.0])], ["g", ods.series(["a", "b", "a"])]])
let p = gg.plot(f, gg.aes(x: "x", y: "y", color: "g")) |> gg.point() |> gg.labs(title: "t")
let s = unwrap(json.parse(gg.scene(p)))
[typeof(p), len(p.layers), s.type, len(filter(s.ops, (o) => o.op == "points")), s.panels[0].h_axis.kind, s.layers[0].fields]
"#
        .to_string(),
    )
    .expect("runs");
    let text = format!("{v}");
    assert!(text.contains("\"Map\", 1, \"scene\""), "{text}");
    assert!(text.contains("\"continuous\""), "{text}");
    assert!(text.contains("[\"x\", \"y\", \"g\"]"), "{text}");
}

#[test]
fn errors_name_what_is_wrong() {
    for (src, want) in [
        (
            r#"gg.plot(f, gg.aes(x: "nope", y: "y")) |> gg.point()"#,
            "no such column",
        ),
        (
            r#"gg.plot(f, gg.aes(x: "x", y: "y")) |> gg.layer("blob")"#,
            "unknown geom",
        ),
        (
            r#"gg.plot(f, gg.aes(x: "g")) |> gg.histogram()"#,
            "categorical",
        ),
        (
            r#"gg.plot(f, gg.aes(x: "x", y: "y")) |> gg.point(color: "notacolour")"#,
            "unknown colour",
        ),
    ] {
        let program = format!(
            "use gg\nlet f = ods.frame([[\"x\", ods.series([1.0, 2.0])], [\"y\", ods.series([1.0, 2.0])], [\"g\", ods.series([\"a\", \"b\"])]])\ngg.svg({src})\n"
        );
        let e = eval(program).expect_err(src);
        assert!(e.contains(want), "{src}: {e}");
    }
}

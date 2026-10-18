//! `olang check`'s `no_effect` advisory: an expression statement that only
//! computes a value — a literal, a name, arithmetic or a comparison over
//! them — whose value is dropped. A block's last statement (its value),
//! the program's last, and a notebook cell's last (the cell's output) are
//! not dropped values.

use olang::parser::Parser;

fn no_effect(source: &str) -> Vec<(u32, String)> {
    let program = Parser::new().parse(source).expect("parses");
    olang::tools::check::check_program(&program)
        .into_iter()
        .filter(|d| d.class == Some("no_effect"))
        .map(|d| (d.line, d.message))
        .collect()
}

#[test]
fn a_negation_on_a_line_of_its_own_is_reported() {
    // The formula that lost its half: `-10.0` glued is a negation of its
    // own, not a continuation of the line above.
    let found = no_effect("let a = 1.0 + 2.0\n-10.0\nprintln(a)\n");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, 2);
    assert!(found[0].1.contains("`- x`"), "{}", found[0].1);
}

#[test]
fn a_continued_line_is_one_statement_and_clean() {
    assert!(no_effect("let a = 1.0 + 2.0\n    - 10.0\nprintln(a)\n").is_empty());
    assert!(no_effect("let a = 1.0\n    + 2.0\nprintln(a)\n").is_empty());
}

#[test]
fn dropped_values_inside_blocks_are_reported() {
    let src = "fn f(x) = {\n    x == 3\n    x * 2\n    \"label\"\n    x\n}\nprintln(f(1))\n";
    let lines: Vec<u32> = no_effect(src).into_iter().map(|(l, _)| l).collect();
    assert_eq!(lines, vec![2, 3, 4]);
}

#[test]
fn values_that_are_used_are_clean() {
    for src in [
        // a block's last statement is its value
        "fn f(x) = {\n    let y = x + 1\n    y * 2\n}\nprintln(f(1))\n",
        // an if branch's and a lambda body's
        "let g = (x) => {\n    println(x)\n    x + 1\n}\nprintln(if true => { 1 } else => { 2 })\n",
        // the program's last statement is its result
        "let x = 4\nx * 2\n",
        // calls, assignments and `()` are not reported
        "let mut n = 0\nn = n + 1\nprintln(n)\n()\nprintln(n)\n",
        // a loop body's last statement is the body's value
        "for i in 0..3 {\n    println(i)\n    i\n}\nprintln(1)\n",
    ] {
        assert!(no_effect(src).is_empty(), "{src}: {:?}", no_effect(src));
    }
}

#[test]
fn a_top_level_dropped_value_is_reported() {
    let found = no_effect("let x = 4\nx\nprintln(x)\n");
    assert_eq!(found.iter().map(|f| f.0).collect::<Vec<_>>(), vec![2]);
}

#[test]
fn it_is_advisory_and_promotable() {
    assert!(olang::pkg::manifest::PROMOTE_CLASSES.contains(&"no_effect"));
    let program = Parser::new().parse("1 + 1\nprintln(2)\n").unwrap();
    let d = olang::tools::check::check_program(&program);
    assert!(d.iter().all(|d| d.warning), "{d:?}");
}

#[test]
fn a_notebook_cell_s_last_statement_is_its_output() {
    // Each cell's last statement is shown under it; only a dropped value
    // before it in the same cell is reported.
    let dir = std::env::temp_dir().join(format!("olang-no-effect-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let nb = dir.join("cells.olmd");
    std::fs::write(
        &nb,
        "# Cells\n\n```olang\nlet a = 2\na * 3\n```\n\nProse.\n\n```olang\nlet b = a\n-1\nb + 1\n```\n\n```olang\nb\n```\n",
    )
    .unwrap();
    let lines = olang::notebook::cell_value_lines(&std::fs::read_to_string(&nb).unwrap());
    assert!(lines.contains(&5) && lines.contains(&13) && lines.contains(&17), "{lines:?}");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["check", "--format", "json"])
        .arg(&nb)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let doc: serde_json::Value = serde_json::from_str(&text).expect("json");
    let found: Vec<i64> = doc["files"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "no_effect")
        .map(|d| d["line"].as_i64().unwrap())
        .collect();
    assert_eq!(found, vec![12], "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

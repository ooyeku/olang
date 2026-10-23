//! What building olang's website found in the data stack and the tools,
//! held: a module's missing member raises on every tier; `group_by` and
//! `agg` refuse text where an aggregation goes; a gg plot's ids are its
//! own, and its theme takes a paper and an ink; `olang test` walks what a
//! checkout keeps and runs a test from its own folder; a rule's
//! finding may be an object; `notebook export --json` carries what a page
//! needs, both papers too, and writes nothing beside the notebook (and
//! draws on no paper when asked); a `select` of aggregates is one row;
//! `olang check` keeps a builtin a namespace import does not name;
//! `spawn` is a keyword token; other languages highlight; and
//! `runtime.features` names the functions a profile cannot run.

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_site_findings_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_in(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(olang())
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

/// The program prints `expected` on every tier; the work happens inside
/// `fn main_()`, so the compiled tier compiles it.
fn agrees(tag: &str, body: &str, expected: &str) {
    let dir = scratch(tag);
    let code = format!("use ods {{ col }}\nfn main_() = {{\n{body}\n}}\nmain_()\n");
    std::fs::write(dir.join("p.ol"), &code).unwrap();
    for flags in TIERS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push("p.ol");
        let (out, err, _) = run_in(&dir, &args);
        assert_eq!(out.trim_end(), expected.trim_end(), "flags {flags:?}\nstderr: {err}\nprogram:\n{code}");
    }
}

#[test]
fn a_missing_module_member_raises_naming_the_nearest() {
    agrees(
        "member",
        r#"
    println(attempt(() => math.pi))
    println(attempt(() => math.sqrtt(4.0)))
    let m = math
    println(attempt(() => m.pi))
    println(map_get(math, "pi"))
    println(map_get(math, "PI") == math.PI)
    let mut s = 0.0
    for i in range(0, 50) { s = s + math.PI }
    println(s > 150.0)
"#,
        r#"Err("module 'math' has no member 'pi'; did you mean 'PI'?")
Err("module 'math' has no member 'sqrtt'; did you mean 'sqrt'?")
Err("module 'math' has no member 'pi'; did you mean 'PI'?")
()
true
true"#,
    );
}

#[test]
fn text_where_an_aggregation_goes_is_refused() {
    agrees(
        "agg_text",
        r#"
    let f = ods.frame({ k: ["a", "a", "b"], v: [1.0, 2.0, 3.0] })
    println(attempt(() => ods.group_by(f, ["k"], #{ "v": "mean" })))
    println(attempt(() => f.group_by("k").agg({ v: "label" })))
    let g = f.group_by("k").agg({ m: col("v").mean(), tag: ods.lit("x") })
    println(g.m.to_list())
    println(g.tag.to_list())
"#,
        r#"Err("ods.group_by: 'v' is the text \"mean\"; an aggregation is an expression like col(\"v\").mean() (ods.lit(\"mean\") for a constant column; the triple [\"v\", \"mean\", \"v\"] names an op)")
Err("ods.agg: 'v' is the text \"label\"; an aggregation is an expression like col(\"x\").sum() (ods.lit(\"label\") for a constant column; the triple [\"v\", \"label\", \"v\"] names an op)")
[1.5, 3.0]
["x", "x"]"#,
    );
}

fn ids(svg: &str) -> Vec<String> {
    svg.match_indices(" id=\"")
        .map(|(at, m)| {
            let rest = &svg[at + m.len()..];
            rest[..rest.find('"').unwrap()].to_string()
        })
        .collect()
}

#[test]
fn a_gg_plots_ids_are_its_own_and_its_theme_takes_paper_and_ink() {
    let dir = scratch("gg");
    std::fs::write(
        dir.join("p.ol"),
        r#"use gg
let f = ods.frame({ x: [1.0, 2.0, 3.0], y: [2.0, 1.0, 3.0], g: ["a", "b", "a"] })
let p = gg.plot(f, gg.aes(x: "x", y: "y")) |> gg.point()
let q = gg.plot(f, gg.aes(x: "y", y: "x")) |> gg.line() |> gg.facet_wrap("g")
fs.write_file("p.svg", gg.svg(p))
fs.write_file("p2.svg", gg.svg(p))
fs.write_file("pd.svg", gg.svg(p |> gg.theme(appearance: "dark")))
fs.write_file("q.svg", gg.svg(q))
fs.write_file("t.svg", gg.svg(p |> gg.theme(paper: "transparent", ink: "currentColor", id: "fig 1")))
println(plot.describe(gg.svg(p)) |> is_ok())
"#,
    )
    .unwrap();
    let (out, err, ok) = run_in(&dir, &["p.ol"]);
    assert!(ok && out.trim() == "true", "{out}{err}");
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).unwrap();
    let (p, p2, pd, q, t) = (read("p.svg"), read("p2.svg"), read("pd.svg"), read("q.svg"), read("t.svg"));
    assert_eq!(p, p2, "the same plot writes the same ids");
    assert!(p.contains("<metadata class=\"olang-chart\">"));
    let mut all: Vec<String> = [&p, &pd, &q].iter().flat_map(|s| ids(s)).collect();
    let n = all.len();
    assert!(n >= 4, "{all:?}");
    all.sort();
    all.dedup();
    assert_eq!(all.len(), n, "no id shared between plots or appearances");
    for id in ids(&q) {
        assert!(q.contains(&format!("url(#{id})")), "{id} is referenced");
    }
    assert_eq!(ids(&t), vec!["fig1-clip1".to_string()]);
    assert!(t.contains("fill=\"transparent\""));
    assert!(t.contains("stroke=\"currentColor\" stroke-opacity=\"0.12\""));
    assert!(!t.contains("#fcfcfb"), "the paper replaces the surface");
}

#[test]
fn olang_test_walks_what_a_checkout_keeps_and_runs_a_test_from_its_folder() {
    let dir = scratch("walk");
    std::fs::write(dir.join("olang.toml"), "[package]\nname = \"p\"\nversion = \"0.1.0\"\n").unwrap();
    std::fs::write(dir.join(".gitignore"), "out/\n").unwrap();
    std::fs::write(dir.join("data.txt"), "x").unwrap();
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(
        dir.join("tests/a_test.ol"),
        "test \"reads beside the test\" { assert_eq(unwrap(fs.read_file(\"../data.txt\")), \"x\") }\n",
    )
    .unwrap();
    for d in ["out", "dist", "target", "node_modules/x", ".cache"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
        std::fs::write(dir.join(d).join("b_test.ol"), "test \"never\" { assert_eq(1, 2) }\n").unwrap();
    }
    let (out, err, ok) = run_in(&dir, &["test"]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("1 passed, 0 failed") && out.contains("1 test file"), "{out}");
    // a folder named on the command line is walked whatever ignores it
    let (out, _, ok) = run_in(&dir, &["test", "out"]);
    assert!(!ok && out.contains("1 failed"), "{out}");
}

#[test]
fn a_rule_finding_may_be_an_object() {
    let dir = scratch("rules");
    std::fs::write(
        dir.join("rules.ol"),
        r#"share fn rule_obj(nodes) = nodes |> filter((n) => n.kind == "call" && n.target == "unwrap") |> map((n) => { message: "bare unwrap", line: n.line })
share fn rule_one(nodes) = { message: "one object", line: 1 }
"#,
    )
    .unwrap();
    std::fs::write(dir.join("a.ol"), "let x = unwrap(Ok(1))\n").unwrap();
    let (out, err, ok) = run_in(&dir, &["check", "--rules", "rules.ol", "a.ol"]);
    assert!(!ok);
    let all = format!("{out}{err}");
    assert!(all.contains("[rule_obj] bare unwrap") && all.contains("[rule_one] one object"), "{all}");
}

#[test]
fn notebook_export_json_carries_what_a_page_needs_and_writes_nothing_beside() {
    let dir = scratch("nb");
    let nb_dir = dir.join("nb");
    std::fs::create_dir_all(&nb_dir).unwrap();
    std::fs::write(
        nb_dir.join("x.olmd"),
        r#"---
title: X
params:
  n: 30
---

# X

There are ${n} rows; the mean is ${str.fmt(mean_y, ".2f")}.

```olang {name: "frame"}
let f = ods.frame({ x: range(0, n) |> map((i) => i * 1.0), y: range(0, n) |> map((i) => i * 2.0) })
let mean_y = ods.mean(f.y)
f
```

```olang {name: "pick"}
use ui
let k = ui.select("b", ["a", "b"])
```

```olang {name: "chart"}
use gg
gg.plot(f, gg.aes(x: "x", y: "y")) |> gg.line()
```
"#,
    )
    .unwrap();
    let (out, err, ok) = run_in(&dir, &["notebook", "export", "nb/x.olmd", "--json", "o.json", "--html", "o.html", "--rows", "25"]);
    assert!(ok, "{out}{err}");
    let names: Vec<String> = std::fs::read_dir(&nb_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["x.olmd".to_string()], "nothing written beside the notebook");
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("o.json")).unwrap()).unwrap();
    assert_eq!(doc["params"][0]["default"], "30");
    let ctl = &doc["controls"][0];
    assert_eq!(ctl["kind"], "select");
    assert_eq!(ctl["default"], "b");
    assert_eq!(ctl["options"], serde_json::json!(["a", "b"]));
    let blocks = doc["blocks"].as_array().unwrap();
    let prose = blocks.iter().find(|b| b["kind"] == "prose" && !b["exprs"].as_array().unwrap().is_empty()).unwrap();
    assert_eq!(prose["exprs"][0]["text"], "30");
    assert_eq!(prose["exprs"][1]["text"], "29.00");
    let codes: Vec<&serde_json::Value> = blocks.iter().filter(|b| b["kind"] == "code").collect();
    assert_eq!(codes.len(), 3);
    assert!(codes[0]["source"].as_str().unwrap().starts_with("let f = ods.frame"));
    let cell = |id: &serde_json::Value| doc["cells"].as_array().unwrap().iter().find(|c| &c["id"] == id).unwrap().clone();
    let frame = cell(&codes[0]["id"]);
    let v = &frame["reply"]["value"];
    assert_eq!(v["table"]["cols"], serde_json::json!(["x", "y"]));
    assert_eq!(v["rows"].as_array().unwrap().len(), 25);
    let chart = cell(&codes[2]["id"]);
    let svg = &chart["reply"]["value"]["svg"];
    let (light, dark) = (svg["light"].as_str().unwrap(), svg["dark"].as_str().unwrap());
    assert!(light.contains("#fcfcfb") && dark.contains("#16181d"), "each appearance drawn natively");
    assert!(chart["reply"]["value"]["chart"]["type"] == "scene");
    let html = std::fs::read_to_string(dir.join("o.html")).unwrap();
    let mut seen = std::collections::HashSet::new();
    for id in ids(&html) {
        assert!(seen.insert(id.clone()), "id {id} used twice in the page");
    }
}

#[test]
fn a_select_of_aggregates_and_literals_is_one_row() {
    agrees(
        "select_one_row",
        r#"
    let f = ods.frame({ t: [3, 1, 2], g: ["a", "b", "a"] })
    let one = f.select({ n: ods.lit(1), lo: col("t").min(), hi: col("t").max() + 1 })
    println([one.n_rows(), one.lo.to_list(), one.hi.to_list(), one.n.to_list()])
    let mixed = f.select({ t: col("t"), lo: col("t").min() })
    println([mixed.n_rows(), mixed.lo.to_list()])
    let kept = ods.with(f, { lo: col("t").min() })
    println([kept.n_rows(), kept.lo.to_list()])
    let grouped = f.select({ m: col("t").max().over(["g"]) })
    println(grouped.n_rows())
    println(ods.frame({ t: [] }).select({ c: col("t").count() }).n_rows())
"#,
        r#"[1, [1], [4], [1]]
[3, [1, 1, 1]]
[3, [1, 1, 1]]
3
1"#,
    );
}

#[test]
fn check_keeps_a_builtin_a_namespace_import_does_not_name() {
    let dir = scratch("check_find");
    std::fs::write(
        dir.join("shelf.toml"),
        format!("[libraries]\nweb = \"{}/frameworks/web-sdk\"\n", env!("CARGO_MANIFEST_DIR")),
    )
    .unwrap();
    std::fs::write(
        dir.join("p.ol"),
        "use gg\nuse web { el }\nprintln(show(find([1, 2, 3], (r) => r > 1)))\n",
    )
    .unwrap();
    let out = Command::new(olang())
        .args(["check", "p.ol"])
        .current_dir(&dir)
        .env("OLANG_HOME", dir.join(".home"))
        .env("OLANG_SHELF", dir.join("shelf.toml"))
        .output()
        .expect("run olang");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && !text.contains("Missing required argument"), "{text}");
}

#[test]
fn spawn_is_a_keyword_and_other_languages_highlight() {
    agrees(
        "tokens_langs",
        r#"
    let kind = (src, word) => first(filter(meta.tokens(src), (t) => t.text == word)).kind
    println([kind("let h = spawn { 1 }", "spawn"), kind("let g = spawn work(1)", "spawn"), kind("let spawn = 1\nspawn + 1", "spawn"), kind("task.spawn(f)", "spawn")])
    println([kind("await x", "await"), kind("meta.tokens(s)", "meta"), kind("meta fn m(e) = e", "meta")])
    for lang in ["python", "rust", "js", "shell", "toml", "json", "markdown", "py", "sh", "md"] {
        let src = "x = \"a<b\" # c\n[t]\n{\"k\": 1} // é ✓\n`q`"
        assert(join(map(meta.tokens(src, lang), (t) => t.text), "") == src, lang)
    }
    println(meta.highlight("x = 1  # n", "python"))
    println(meta.highlight("[a]\nb = \"<c>\"", "toml"))
    println(md.render("```json\n{\"a\": true}\n```\n\n```cobol\nX\n```\n", #{ "highlight": true }).html)
    println(attempt(() => meta.highlight("x", "cobol")))
"#,
        r#"["keyword", "keyword", "ident", "function"]
["keyword", "module", "keyword"]
<span class="ol-ident">x</span> <span class="ol-operator">=</span> <span class="ol-number">1</span>  <span class="ol-comment"># n</span>
<span class="ol-punct">[</span><span class="ol-type">a</span><span class="ol-punct">]</span>
<span class="ol-property">b</span> <span class="ol-operator">=</span> <span class="ol-string">&quot;&lt;c&gt;&quot;</span>
<pre><code class="language-json"><span class="ol-punct">{</span><span class="ol-property">&quot;a&quot;</span><span class="ol-operator">:</span> <span class="ol-constant">true</span><span class="ol-punct">}</span>
</code></pre>
<pre><code class="language-cobol">X
</code></pre>

Err("meta.highlight: no tokenizer for 'cobol' (it knows olang, python, rust, js, shell, toml, json, markdown)")"#,
    );
}

#[test]
fn runtime_features_answers_any_profile_and_names_what_needs_a_power() {
    agrees(
        "features_needs",
        r#"
    let b = runtime.features("browser")
    let f = runtime.features("full")
    let n = runtime.features("native")
    println([b.profile, b.fs, map_get(b.needs, "ods.open_csv"), map_get(b.needs, "gg.save"), map_get(b.needs, "spawn")])
    println([contains(b.modules, "re"), contains(f.modules, "re"), contains(b.modules, "ods"), contains(b.modules, "fs")])
    println([f.needs == b.needs, len(map_keys(n.needs)), map_has_key(b.needs, "ods.read_csv")])
"#,
        r#"["browser", false, "fs", "fs", "threads"]
[false, true, true, false]
[true, 0, false]"#,
    );
}

#[test]
fn notebook_export_names_both_papers_and_draws_on_none_when_asked() {
    let dir = scratch("nb_paper");
    std::fs::write(
        dir.join("x.olmd"),
        "# X\n\n```olang {name: \"chart\"}\nuse gg\ngg.plot(ods.frame({ x: [1.0, 2.0], y: [3.0, 4.0] }), gg.aes(x: \"x\", y: \"y\")) |> gg.line()\n```\n\n```olang {name: \"bars\"}\nplot.bar([\"a\", \"b\"], ods.series([1.0, 2.0]))\n```\n",
    )
    .unwrap();
    let svgs = |args: &[&str], out: &str| -> Vec<serde_json::Value> {
        let mut all = vec!["notebook", "export", "x.olmd", "--json", out];
        all.extend_from_slice(args);
        let (o, e, ok) = run_in(&dir, &all);
        assert!(ok, "{o}{e}");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(out)).unwrap()).unwrap();
        doc["cells"].as_array().unwrap().iter().map(|c| c["reply"]["value"]["svg"].clone()).collect()
    };
    for svg in svgs(&[], "a.json") {
        let paper = &svg["paper"];
        let (l, d) = (paper["light"].as_str().unwrap(), paper["dark"].as_str().unwrap());
        assert!(l != d && !l.is_empty() && !d.is_empty(), "{paper}");
        assert!(svg["light"].as_str().unwrap().contains(&format!("fill=\"{l}\"")));
        assert!(svg["dark"].as_str().unwrap().contains(&format!("fill=\"{d}\"")));
    }
    for svg in svgs(&["--paper", "transparent"], "b.json") {
        // the colours are still named, and the paper's rect is clear
        assert!(svg["paper"]["light"].as_str().is_some_and(|p| p != "none"), "{}", svg["paper"]);
        assert!(svg["paper"]["dark"].as_str().is_some_and(|p| p != "none"));
        assert_eq!(svg["bg"], "none");
        for key in ["light", "dark", "text"] {
            let text = svg[key].as_str().unwrap();
            let rect = &text[text.find("<rect").unwrap()..];
            let rect = &rect[..rect.find('>').unwrap()];
            assert!(rect.contains("fill=\"none\""), "{key}: {rect}");
        }
    }
    let (_, e, ok) = run_in(&dir, &["notebook", "export", "x.olmd", "--json", "c.json", "--paper", "blue"]);
    assert!(!ok && e.contains("transparent"), "{e}");
}

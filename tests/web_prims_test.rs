//! What a web framework stands on: `md` (Markdown with hooks, anchors,
//! the outline and front matter), `meta.tokens` / `meta.highlight`,
//! `runtime.features`, the brotli codec and `http.encode_query`, each on
//! every tier (tree-walker, VM, the default tiering) with one answer.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_web_prims_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, src: &str, flags: &[&str]) -> (String, String, i32) {
    let file = dir.join("main.ol");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(flags)
        .arg(&file)
        .current_dir(dir)
        .output()
        .expect("olang runs");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// The program's output, the same on every tier.
fn agree(tag: &str, src: &str) -> String {
    let dir = workspace(tag);
    let mut first: Option<String> = None;
    for flags in [&["--no-ovm"][..], &["--ovm-tier=1"][..], &[][..]] {
        let (out, err, rc) = run(&dir, src, flags);
        assert_eq!(rc, 0, "{flags:?}\n{out}\n{err}");
        match &first {
            None => first = Some(out),
            Some(f) => assert_eq!(&out, f, "tiers disagree under {flags:?}"),
        }
    }
    first.unwrap()
}

#[test]
fn markdown_renders_with_hooks_on_every_tier() {
    let out = agree(
        "md",
        r##"
let src = "# The book\n\n## `http` — HTTP\n\nSee [ods](ods.md#frames), [gone](x) and https://olang.dev.\n\n<img src=\"a.svg\">\n\n| a | b |\n|---|--:|\n| 1 | 2 |\n\n- [x] done\n\n```olang no-run\nlet x = 1 < 2\n```\n\n```text\nplain\n```\n\n## Same\n\n## Same\n"
let seen = cell.new([])
let r = md.render(src, #{
    "link": (u) => if u == "x" => () else if str.contains(u, ".md") => "/book/" + str.replace(u, ".md", "/") else => u,
    "code": (lang, info, text) => {
        cell.set(seen, cell.get(seen) + [info])
        if lang == "olang" => "<pre class=\"ol\">" + meta.highlight(text) + "</pre>" else => ()
    },
    "heading": (level, slug, html) => if level == 1 => () else => "<h" + to_string(level) + " id=\"" + slug + "\">" + html + " #</h" + to_string(level) + ">"
})
println(r.title)
println(map(r.headings, (h) => h.slug))
println(map(r.headings, (h) => h.level))
for want in ["<h1 id=\"the-book\">The book</h1>", "<h2 id=\"http--http\"><code>http</code> — HTTP #</h2>",
             "href=\"/book/ods/#frames\"", " gone and ", "<a href=\"https://olang.dev\">https://olang.dev</a>.",
             "<img src=\"a.svg\">", "text-align: right", "checkbox", "<pre class=\"ol\"><span class=\"ol-keyword\">let</span>",
             "<pre><code class=\"language-text\">plain\n</code></pre>", "id=\"same-1\""] {
    if !str.contains(r.html, want) => println("missing: " + want)
}
println(cell.get(seen))
println(str.trim(md.render("# T", #{ "shift": 2 }).html))
println(str.trim(md.render("<b>x</b> [j](javascript:alert(1))", #{ "safe": true }).html))
println(str.trim(md.render("<b>x</b>", #{ "html": "drop" }).html))
println(attempt(() => md.render("[a](b)", #{ "link": (u) => 1 })))
println(attempt(() => md.render("x", #{ "colour": true })))
println(attempt(() => md.render("[a](b)", #{ "link": (u) => raise("no " + u) })))
"##,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "The book");
    assert_eq!(lines[1], r#"["the-book", "http--http", "same", "same-1"]"#);
    assert_eq!(lines[2], "[1, 2, 2, 2]");
    assert!(!out.contains("missing:"), "{out}");
    assert_eq!(lines[3], r#"["olang no-run", "text"]"#);
    assert_eq!(lines[4], "<h3 id=\"t\">T</h3>");
    assert!(lines[5].contains("&lt;b&gt;x&lt;/b&gt;") && !out.contains("javascript:"), "{out}");
    assert_eq!(lines[6], "<p>x</p>");
    assert!(lines[7].starts_with("Err(") && lines[7].contains("`link` hook"), "{out}");
    assert!(lines[8].contains("unknown option `colour`"), "{out}");
    assert!(lines[9].contains("no b"), "{out}");
}

#[test]
fn outline_slug_front_matter_and_text() {
    let out = agree(
        "front",
        r##"
println(md.headings("# A\n\n## B c\n"))
println(md.slug("`ods.read_csv` takes text; `ods.read_csv_file` takes a path"))
let f = unwrap(md.front("---\ntitle: \"Hi: there\"\ndate: 2026-10-20\ndraft: false\nweight: 3\ntags: [a, b]\nauthor:\n  name: Ola\n---\n# Body\n"))
println([f.format, f.data.title, f.data.date, f.data.draft, f.data.weight, f.data.tags, f.data.author.name, f.body])
let t = unwrap(md.front("+++\ntitle = \"T\"\n[extra]\nn = 1\n+++\nrest"))
println([t.format, t.data.title, t.data.extra.n, t.body])
println(unwrap(md.front("plain")))
println(is_err(md.front("---\ntitle: x\n")))
println(md.text("# A *b*\n\nc `d` [e](f)\n"))
println(str.trim(md.render("---\ntitle: x\n---\n# H\n").html))
"##,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].contains("\"slug\": \"b-c\"") && lines[0].contains("\"level\": 2"), "{out}");
    assert_eq!(lines[1], "odsread_csv-takes-text-odsread_csv_file-takes-a-path");
    assert_eq!(
        lines[2],
        r##"["yaml", "Hi: there", "2026-10-20", false, 3, ["a", "b"], "Ola", "# Body\n"]"##
    );
    assert_eq!(lines[3], r#"["toml", "T", 1, "rest"]"#);
    assert!(lines[4].contains("\"format\": \"\"") && lines[4].contains("\"body\": \"plain\""), "{out}");
    assert_eq!(lines[5], "true");
    assert_eq!(lines[6], "A b");
    assert_eq!(lines[7], "c d e");
    assert_eq!(lines[8], "<h1 id=\"h\">H</h1>");
}

#[test]
fn tokens_cover_every_olang_file_in_the_repository() {
    // meta.tokens is lossless over real code: every .ol file under
    // examples/ and frameworks/, joined back, is itself.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["examples", "frameworks", "src/stdlib/embedded"] {
        let mut stack = vec![root.join(dir)];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "ol") {
                    files.push(p);
                }
            }
        }
    }
    assert!(files.len() > 50, "found {} files", files.len());
    let dir = workspace("tokens");
    let list: Vec<String> = files.iter().map(|p| format!("{:?}", p.display().to_string())).collect();
    let src = format!(
        "let files = [{}]\nlet mut bad = []\nlet mut n = 0\nfor f in files {{\n    let text = unwrap(fs.read_file(f))\n    let toks = meta.tokens(text)\n    n = n + len(toks)\n    if join(map(toks, (t) => t.text), \"\") != text => {{ bad = bad + [f] }}\n    if len(toks) > 0 && last(toks).end != str.length(text) => {{ bad = bad + [f + \" (end)\"] }}\n}}\nprintln(bad)\nprintln(n > 10000)\n",
        list.join(", ")
    );
    let (out, err, rc) = run(&dir, &src, &[]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "[]\ntrue\n");
}

#[test]
fn tokens_and_highlight_agree_across_tiers() {
    let out = agree(
        "tok",
        r##"
let toks = meta.tokens("use web { el }\nfn f(a) = `v ${a + 1}` // c\n@trace(x)\nlet t = true")
println(map(filter(toks, (t) => t.kind != "space"), (t) => t.kind + ":" + t.text))
println(map(take(toks, 3), (t) => [t.start, t.end, t.line]))
println(meta.highlight("let s = \"<a>\""))
"##,
    );
    let lines: Vec<&str> = out.lines().collect();
    for want in [
        "keyword:use",
        "module:web",
        "keyword:fn",
        "function:f",
        "string:`v ",
        "interpolation:${",
        "operator:+",
        "number:1",
        "interpolation:}",
        "comment:// c",
        "macro:@trace",
        "constant:true",
    ] {
        assert!(lines[0].contains(want), "{want} in {}", lines[0]);
    }
    assert_eq!(lines[1], "[[0, 3, 1], [3, 4, 1], [4, 7, 1]]");
    assert_eq!(
        lines[2],
        "<span class=\"ol-keyword\">let</span> <span class=\"ol-ident\">s</span> <span class=\"ol-operator\">=</span> <span class=\"ol-string\">&quot;&lt;a&gt;&quot;</span>"
    );
}

#[test]
fn runtime_features_name_each_profile() {
    let out = agree(
        "features",
        r##"
let n = runtime.features()
let b = runtime.features("browser")
let f = runtime.features("full")
println([n.profile, n.threads, n.fs, n.jit, contains(n.modules, "fs"), contains(n.modules, "re"), contains(n.modules, "md")])
println([b.profile, b.threads, b.fs, b.dom, contains(b.modules, "fs"), contains(b.modules, "re"), contains(b.modules, "md"), contains(b.modules, "gg")])
println([f.profile, contains(f.modules, "re"), contains(f.modules, "http")])
println(attempt(() => runtime.features("mars")))
"##,
    );
    assert_eq!(
        out,
        "[\"native\", true, true, true, true, true, true]\n\
         [\"browser\", false, false, true, false, false, true, true]\n\
         [\"full\", true, false]\n\
         Err(\"runtime.features: profile is \\\"native\\\", \\\"browser\\\" or \\\"full\\\", got \\\"mars\\\"\")\n"
    );
}

#[test]
fn brotli_and_encode_query() {
    let out = agree(
        "codec",
        r##"
let body = str.repeat("olang ", 2000)
let small = compress.brotli_level(body, 11)
println(len(small) < len(body) / 10)
println(unwrap(bytes.to_string(unwrap(compress.unbrotli(small)))) == body)
println(is_err(compress.unbrotli(bytes.from_list([255, 255, 255, 255]))))
println(attempt(() => compress.brotli_level(body, 12)))
println("?" + http.encode_query(#{ "q": "a b", "t": ["x", "y"] }))
"##,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(&lines[..3], ["true", "true", "true"]);
    assert!(lines[3].starts_with("Err("), "{out}");
    assert_eq!(lines[4], "?q=a%20b&t=x&t=y");
}

//! Notebooks (`.olmd`) read as programs: `olang run`, `test`, `check`,
//! `fmt` and `expand` take one, every location is the file's own line,
//! the front matter's params are bound and its capabilities enforced.
//! Runs the real binary over tests/fixtures/notebooks/, on the
//! interpreter, the bytecode VM and the tiered default.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notebooks")
}

/// The three ways a program runs: the tree-walking interpreter, every
/// function on the bytecode VM from its first call, and the default.
const TIERS: &[&[&str]] = &[&["--no-ovm"], &["--ovm-tier=1"], &[]];

fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(olang())
        .args(args)
        .current_dir(dir)
        .env_remove("OLANG_DENY")
        .output()
        .expect("olang runs")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_nb_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

#[test]
fn the_spec_example_runs_on_every_tier() {
    // Run from the fixtures' folder, not the notebook's: the notebook runs
    // in its own folder, so `data/sales.csv` is the one beside it.
    for tier in TIERS {
        let mut args = tier.to_vec();
        args.push("sales/sales.olmd");
        let out = run_in(&fixtures(), &args);
        assert!(out.status.success(), "{:?}: {}", tier, stderr(&out));
        assert_eq!(
            stdout(&out),
            "east since 2026-01-01: 360\ntrue\n",
            "{:?}",
            tier
        );
    }
    let out = run_in(&fixtures(), &["run", "sales/sales.olmd"]);
    assert_eq!(stdout(&out), "east since 2026-01-01: 360\ntrue\n");
    let out = run_in(&fixtures(), &["check", "sales/sales.olmd"]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
}

#[test]
fn params_bind_with_their_types_and_param_overrides() {
    for tier in TIERS {
        let mut args = tier.to_vec();
        args.push("params.olmd");
        let out = run_in(&fixtures(), &args);
        assert_eq!(
            stdout(&out),
            "Int String List Map\n3 east 2 2.5 true\n",
            "{:?}: {}",
            tier,
            stderr(&out)
        );
    }
    // after the notebook, as `olang run x.olmd --param region=west`
    let out = run_in(
        &fixtures(),
        &["run", "params.olmd", "--param", "n=40", "--param=name=west"],
    );
    assert_eq!(stdout(&out), "Int String List Map\n41 west 2 2.5 true\n");
    // before it, as a run option
    let out = run_in(&fixtures(), &["--param", "tags=[1, 2, 3]", "params.olmd"]);
    assert_eq!(stdout(&out), "Int String List Map\n3 east 3 2.5 true\n");
    // a value that is no olang literal is a String
    let out = run_in(&fixtures(), &["params.olmd", "--param", "n=2026-01-01"]);
    assert!(
        stdout(&out).starts_with("String String"),
        "{}",
        stdout(&out)
    );
    // `olang test` and `olang check` take them too
    let out = run_in(&fixtures(), &["test", "params.olmd", "--param", "n=3"]);
    assert!(!out.status.success());
    assert!(
        stdout(&out).contains("Integer(6) != Integer(4)"),
        "{}",
        stdout(&out)
    );
    let out = run_in(&fixtures(), &["test", "params.olmd"]);
    assert!(out.status.success(), "{}", stdout(&out));
    // a --param naming no declared param is an error at `params:`'s line
    let out = run_in(&fixtures(), &["params.olmd", "--param", "nope=1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("params.olmd:3:1: error: --param nope"),
        "{}",
        stderr(&out)
    );
    let out = run_in(&fixtures(), &["check", "params.olmd", "--param", "nope=1"]);
    assert_eq!(out.status.code(), Some(1));
    // malformed, or on a file that is no notebook: misuse
    let out = run_in(&fixtures(), &["params.olmd", "--param", "n"]);
    assert_eq!(out.status.code(), Some(2));
    let out = run_in(&fixtures(), &["--param", "n=1", "greet/index.ol"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("no notebook"), "{}", stderr(&out));
}

#[test]
fn a_cell_redefining_a_param_is_an_error() {
    let ws = workspace("redefine");
    write(
        &ws.join("r.olmd"),
        "---\nparams:\n  region: east\n---\n\n```olang\nfn region() = 1\n```\n",
    );
    let out = run_in(&ws, &["r.olmd"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("r.olmd:7:1: error: `region` is a param of the notebook"),
        "{}",
        stderr(&out)
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn a_runtime_error_points_at_the_files_line() {
    for tier in TIERS {
        let mut args = tier.to_vec();
        args.push("lines.olmd");
        let out = run_in(&fixtures(), &args);
        assert_eq!(out.status.code(), Some(1), "{:?}", tier);
        assert_eq!(stdout(&out), "2\n", "{:?}", tier);
        let err = stderr(&out);
        assert!(err.contains("5 is past 3"), "{:?}: {}", tier, err);
        // the raise in the cell's function, shown with the file's own text
        assert!(err.contains("lines.olmd:17:"), "{:?}: {}", tier, err);
        assert!(err.contains("fn checked(n) = {"), "{:?}: {}", tier, err);
        // the no-run cell is never run
        assert!(!err.contains("never runs"), "{:?}: {}", tier, err);
    }
}

#[test]
fn check_reports_the_files_lines_as_text_and_json() {
    let out = run_in(&fixtures(), &["check", "check_me.olmd"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    for at in [
        "check_me.olmd:12:11",
        "check_me.olmd:17:1",
        "check_me.olmd:13:1",
    ] {
        assert!(err.contains(at), "{}: {}", at, err);
    }
    let out = run_in(&fixtures(), &["check", "check_me.olmd", "--format", "json"]);
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("one JSON document");
    let diags = doc["files"][0]["diagnostics"].as_array().unwrap();
    let at = |code: &str, line: u64| diags.iter().any(|d| d["code"] == code && d["line"] == line);
    assert!(at("notebook", 12), "{:#}", doc);
    assert!(at("notebook", 17), "{:#}", doc);
    assert!(at("runtime", 13), "{:#}", doc);
    let option = diags.iter().find(|d| d["line"] == 12).unwrap();
    assert_eq!(option["column"], 11);
    assert_eq!(option["range"]["start"]["line"], 11);
    // a param no cell reads is no unused name
    assert!(
        !diags.iter().any(|d| d["line"] == 3),
        "the front matter's lines carry no hint: {:#}",
        doc
    );
    // a no-run cell is checked: one that does not parse fails the check
    let ws = workspace("norun");
    write(&ws.join("n.olmd"), "# x\n\n```olang no-run\nlet = 1\n```\n");
    let out = run_in(&ws, &["check", "n.olmd"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("line 4"), "{}", stderr(&out));
    // and it is left out of a run, which a cell that does not parse
    // would stop
    let out = run_in(&ws, &["n.olmd"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_blocks_run_and_fail_on_their_line() {
    for tier in TIERS {
        // a tier flag goes before the command
        let args: Vec<&str> = tier
            .iter()
            .copied()
            .chain(["test", "tests.olmd", "--format", "json"])
            .collect();
        let out = run_in(&fixtures(), &args);
        assert_eq!(out.status.code(), Some(1), "{:?}", tier);
        let events: Vec<serde_json::Value> = stdout(&out)
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let passed = events.iter().find(|e| e["event"] == "passed").unwrap();
        assert_eq!(passed["line"], 10);
        let failed = events.iter().find(|e| e["event"] == "failed").unwrap();
        assert_eq!(failed["line"], 14);
        assert_eq!(failed["at"]["line"], 15, "{:?}", tier);
        assert!(failed["file"].as_str().unwrap().ends_with("tests.olmd"));
    }
    // a folder's notebooks are found beside its .ol files
    let ws = workspace("walk");
    write(
        &ws.join("a_test.ol"),
        "test \"in a file\" { assert_eq(1, 1) }\n",
    );
    write(
        &ws.join("sub/b.olmd"),
        "# b\n\n```olang\ntest \"in a notebook\" { assert_eq(2, 2) }\n```\n",
    );
    let out = run_in(&ws, &["test", "."]);
    assert!(out.status.success(), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(
        text.contains("in a file") && text.contains("in a notebook"),
        "{}",
        text
    );
    assert!(text.contains("2 passed"), "{}", text);
    let out = run_in(&ws, &["check", "."]);
    assert!(stdout(&out).contains("2 files clean"), "{}", stdout(&out));
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn options_are_parsed_and_validated() {
    let out = run_in(&fixtures(), &["bad_option.olmd"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("bad_option.olmd:3:30: error: `hide` is one of"),
        "{}",
        stderr(&out)
    );
    assert!(stdout(&out).is_empty(), "nothing runs: {}", stdout(&out));
    let out = run_in(
        &fixtures(),
        &["check", "bad_option.olmd", "--format", "json"],
    );
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(doc["files"][0]["diagnostics"][0]["line"], 3);
    assert_eq!(doc["files"][0]["diagnostics"][0]["code"], "notebook");
    // every option a cell takes, and the ones it does not
    let ws = workspace("options");
    write(
        &ws.join("ok.olmd"),
        "```olang {name: load, hide: code, run: manual, pin: true}\nprintln(1)\n```\n\n```output {cell: load}\n1\n```\n\n```olang no-run {name: \"two words\", as: markdown}\nprintln(2)\n```\n",
    );
    let out = run_in(&ws, &["ok.olmd"]);
    assert_eq!(stdout(&out), "1\n", "{}", stderr(&out));
    write(
        &ws.join("bad.olmd"),
        "```olang {name: a}\n```\n\n```olang {name: a, colour: red}\n```\n\n```olang fast\n```\n",
    );
    let out = run_in(&ws, &["bad.olmd"]);
    let err = stderr(&out);
    assert!(
        err.contains("bad.olmd:4:20: error: a cell has no option `colour`"),
        "{}",
        err
    );
    assert!(
        err.contains("bad.olmd:7:10: error: `fast` after `olang`"),
        "{}",
        err
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn an_unclosed_fence_is_a_diagnostic() {
    let out = run_in(&fixtures(), &["unclosed.olmd"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("unclosed.olmd:3:1: error: this fence is never closed"),
        "{}",
        stderr(&out)
    );
    let out = run_in(&fixtures(), &["check", "unclosed.olmd"]);
    assert_eq!(out.status.code(), Some(1));
    let out = run_in(&fixtures(), &["fmt", "--check", "unclosed.olmd"]);
    assert_eq!(out.status.code(), Some(1));
    // `~~~` fences and longer backtick fences are fences too
    let ws = workspace("fences");
    write(
        &ws.join("f.olmd"),
        "~~~olang\nprintln(1)\n~~~\n\n````olang\nprintln(\"```\")\n````\n\n    ```olang\n    not a fence: indented four\n",
    );
    let out = run_in(&ws, &["f.olmd"]);
    assert_eq!(stdout(&out), "1\n```\n", "{}", stderr(&out));
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn the_capabilities_grant_is_enforced_and_fail_closed() {
    let ws = workspace("caps");
    write(
        &ws.join("c.olmd"),
        "---\ncapabilities:\n  fs: read\n  net: false\n---\n\n```olang\nprintln(show(caps.granted()))\nprintln(fs.exists(\"c.olmd\"))\nunwrap(fs.write_file(\"out.txt\", \"x\"))\n```\n",
    );
    for tier in TIERS {
        let mut args = tier.to_vec();
        args.push("c.olmd");
        let out = run_in(&ws, &args);
        assert_eq!(out.status.code(), Some(1), "{:?}", tier);
        assert_eq!(
            stdout(&out),
            "#{\"fs\": \"read\", \"net\": false, \"proc\": true, \"db\": true, \"env\": true}\ntrue\n",
            "{:?}",
            tier
        );
        assert!(
            stderr(&out).contains("capability 'fs' denied"),
            "{}",
            stderr(&out)
        );
        assert!(!ws.join("out.txt").exists());
    }
    // with --deny, and with the enclosing project's grant: both intersect
    let out = run_in(&ws, &["--deny", "env", "c.olmd"]);
    assert!(
        stdout(&out).contains("\"db\": true, \"env\": false"),
        "{}",
        stdout(&out)
    );
    write(
        &ws.join("olang.toml"),
        "[package]\nname = \"capsnb\"\nversion = \"0.1.0\"\nlock = \"never\"\n\n[capabilities]\nproc = false\n",
    );
    let out = run_in(&ws, &["c.olmd"]);
    assert!(
        stdout(&out).contains("\"fs\": \"read\", \"net\": false, \"proc\": false"),
        "{}",
        stdout(&out)
    );
    // `olang test` runs a notebook under the same grant
    write(
        &ws.join("t.olmd"),
        "---\ncapabilities:\n  fs: read\n  net: false\n---\n\n```olang\ntest \"the grant\" {\n    println(show(caps.granted()))\n}\n```\n",
    );
    let out = run_in(&ws, &["test", "t.olmd"]);
    assert!(
        stdout(&out).contains("\"fs\": \"read\", \"net\": false, \"proc\": false"),
        "{}",
        stdout(&out)
    );
    // an unknown capability or value refuses to run, with its line
    for (text, at) in [
        (
            "  gpu: true\n",
            "bad.olmd:3:3: error: capabilities has no capability `gpu`",
        ),
        ("  fs: write\n", "bad.olmd:3:7: error: capabilities: fs is"),
        (
            "  net: \"no\"\n",
            "bad.olmd:3:8: error: capabilities: net is",
        ),
    ] {
        write(
            &ws.join("bad.olmd"),
            &format!(
                "---\ncapabilities:\n{}---\n\n```olang\nprintln(\"ran\")\n```\n",
                text
            ),
        );
        let out = run_in(&ws, &["bad.olmd"]);
        assert_eq!(out.status.code(), Some(1), "{}", text);
        assert!(stdout(&out).is_empty(), "{}: {}", text, stdout(&out));
        assert!(stderr(&out).contains(at), "{}", stderr(&out));
        let out = run_in(&ws, &["check", "bad.olmd"]);
        assert_eq!(out.status.code(), Some(1), "{}", text);
        let out = run_in(&ws, &["test", "bad.olmd"]);
        assert_eq!(out.status.code(), Some(1), "{}", text);
    }
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn fmt_formats_cells_and_leaves_prose_byte_for_byte() {
    let ws = workspace("fmt");
    let messy = "---\ntitle:    Messy   \nparams:\n  n:   2\n---\n\n#   Heading   \n\nprose  with  trailing   \r\n\n```olang {name: a}\nlet   x=1   \n\n\n\nfn f(a)=a+n\n```\n\n~~~olang no-run\nlet y =   2  \n~~~\n\n```output {cell: a}\n  untouched   \n```\n\n```sql\nselect   1   \n```\n";
    let tidy = "---\ntitle:    Messy   \nparams:\n  n:   2\n---\n\n#   Heading   \n\nprose  with  trailing   \r\n\n```olang {name: a}\nlet x = 1\n\nfn f(a) = a + n\n```\n\n~~~olang no-run\nlet y = 2\n~~~\n\n```output {cell: a}\n  untouched   \n```\n\n```sql\nselect   1   \n```\n";
    write(&ws.join("m.olmd"), messy);
    let out = run_in(&ws, &["fmt", "--check", "m.olmd"]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert_eq!(std::fs::read_to_string(ws.join("m.olmd")).unwrap(), messy);
    let out = run_in(&ws, &["fmt", "."]);
    assert!(out.status.success(), "{}", stdout(&out));
    assert_eq!(std::fs::read_to_string(ws.join("m.olmd")).unwrap(), tidy);
    let out = run_in(&ws, &["fmt", "--check", "m.olmd"]);
    assert!(out.status.success(), "{}", stdout(&out));
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn use_resolves_from_the_notebooks_folder() {
    // a sibling module, the project's lib/, and the project's dependency
    // (olang.toml above the notebook), run from outside the project
    for tier in TIERS {
        let mut args = tier.to_vec();
        args.push("project/notes/analysis.olmd");
        let out = run_in(&fixtures(), &args);
        assert_eq!(
            stdout(&out),
            "hello, notebook\n42\n",
            "{:?}: {}",
            tier,
            stderr(&out)
        );
    }
    let out = run_in(&fixtures(), &["check", "project/notes/analysis.olmd"]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    assert!(
        !fixtures().join("project/olang.lock").exists(),
        "lock = \"never\": a run writes no lock"
    );
}

#[test]
fn expand_prints_the_program_on_the_files_lines() {
    let out = run_in(&fixtures(), &["expand", "lines.olmd"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = std::fs::read_to_string(fixtures().join("lines.olmd")).unwrap();
    let program = stdout(&out);
    assert_eq!(program.split('\n').count(), text.split('\n').count());
    let line = |s: &str, n: usize| s.split('\n').nth(n - 1).unwrap_or("").to_string();
    assert_eq!(line(&program, 4), "let limit = 3");
    assert_eq!(line(&program, 9), "", "prose is an empty line");
    assert_eq!(line(&program, 17), line(&text, 17));
    assert_eq!(
        line(&program, 23),
        "",
        "a no-run cell is not in the program run"
    );
}

#[test]
fn the_repl_runs_a_notebook() {
    let mut child = Command::new(olang())
        .arg("repl")
        .current_dir(fixtures())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin
            .write_all(b":run sales/sales.olmd\nprintln(fs.exists(\"sales\"))\n")
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let text = stdout(&out);
    assert!(
        text.contains("east since 2026-01-01: 360"),
        "{}{}",
        text,
        stderr(&out)
    );
    // the REPL's own folder is back after the notebook ran in its own
    assert!(text.contains("executed successfully\ntrue"), "{}", text);
}

#[test]
fn check_holds_a_notebook_to_one_name_one_cell_and_top_to_bottom() {
    let ws = workspace("rules");
    let text = "# Rules\n\n```olang\nlet total = 1\nprintln(later(2))\n```\n\n```olang\nlet total = 2\nfn later(x) = x * 2\n```\n\n```olang\nlet both = total + scale\n```\n\n```olang\nlet scale = 3\n```\n";
    write(&ws.join("r.olmd"), text);
    let out = run_in(&ws, &["check", "r.olmd", "--format", "json"]);
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("one JSON document");
    let diags: Vec<&serde_json::Value> = doc["files"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "notebook")
        .collect();
    let at = |line: u64, column: u64, says: &str| {
        diags.iter().any(|d| {
            d["line"] == line
                && d["column"] == column
                && d["message"].as_str().unwrap().contains(says)
        })
    };
    // one name, one cell: at both places, each naming the other
    assert!(
        at(4, 1, "`total` is defined in two cells, here and at line 9"),
        "{:#?}",
        diags
    );
    assert!(
        at(9, 1, "`total` is defined in two cells, here and at line 4"),
        "{:#?}",
        diags
    );
    // top to bottom: a function a later cell declares, and a let
    assert!(
        at(5, 9, "`later` is defined below, in a later cell (line 10)"),
        "{:#?}",
        diags
    );
    assert!(
        at(
            14,
            20,
            "`scale` is defined below, in a later cell (line 18)"
        ),
        "{:#?}",
        diags
    );
    assert_eq!(diags.len(), 4, "{:#?}", diags);
    // the text form names the file's lines
    let out = run_in(&ws, &["check", "r.olmd"]);
    assert!(
        stderr(&out).contains("defined in two cells"),
        "{}",
        stderr(&out)
    );
    // the rules are a notebook's: the same program in a .ol file keeps the
    // language's (shadowing, functions hoisted)
    write(
        &ws.join("same.ol"),
        "let total = 1\nprintln(later(2))\nlet total = 2\nfn later(x) = x * 2\nprintln(total)\n",
    );
    let out = run_in(&ws, &["check", "same.ol"]);
    assert!(out.status.success(), "{}", stderr(&out));
    // a run reads the program as the language does: not refused for them
    write(
        &ws.join("twice.olmd"),
        "```olang\nlet x = 1\n```\n\n```olang\nlet x = 2\nprintln(x)\n```\n",
    );
    let out = run_in(&ws, &["twice.olmd"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "2\n");
    let out = run_in(&ws, &["check", "twice.olmd"]);
    assert_eq!(out.status.code(), Some(1));
    // a `use` of one name in two cells is no clash; a name read inside a
    // function the cell itself declares later is no use of a later cell's
    write(
        &ws.join("fine.olmd"),
        "```olang\nuse lib.tools { twice }\nfn a() = b()\nfn b() = twice(1)\n```\n\n```olang\nuse lib.tools { twice }\nprintln(a() + twice(2))\n```\n",
    );
    write(&ws.join("lib/tools.ol"), "share fn twice(x) = x * 2\n");
    let out = run_in(&ws, &["check", "fine.olmd"]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn a_cells_defines_and_uses_are_read_from_its_text_on_the_files_lines() {
    use olang::notebook::cell_deps;
    let d = cell_deps(
        "let mine = filter(sales, region)\nfn pct(x) = x * share\nlet n = len(mine)",
        20,
    )
    .unwrap();
    let defs: Vec<(&str, u32, bool)> = d.defines.iter().map(|x| (x.0.as_str(), x.1, x.3)).collect();
    assert_eq!(
        defs,
        [("mine", 20, false), ("pct", 21, false), ("n", 22, false)]
    );
    // what it reads that it does not define, at its first use; builtins are none
    let uses: Vec<(&str, u32, u32)> = d.uses.iter().map(|x| (x.0.as_str(), x.1, x.2)).collect();
    assert_eq!(
        uses,
        [("sales", 20, 19), ("region", 20, 26), ("share", 21, 17)]
    );
    // a `use` binds names, said so
    let u = cell_deps("use util { twice, half as h }\ntwice(2)", 3).unwrap();
    assert_eq!(
        u.defines
            .iter()
            .map(|x| (x.0.as_str(), x.3))
            .collect::<Vec<_>>(),
        [("twice", true), ("h", true)]
    );
    assert!(u.uses.is_empty());
    // a cell that does not parse is an error
    assert!(cell_deps("let = 1", 1).is_err());
}

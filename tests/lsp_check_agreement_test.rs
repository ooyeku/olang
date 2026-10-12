//! The language server's errors and warnings are `olang check`'s, file
//! for file. The server once ran its own analyzer and published its
//! stops as errors, so an editor showed problems the checker (and the
//! program) did not have: an import of a name the language also provides
//! was a "Duplicate variable", a constant written below the function that
//! reads it an "Undefined variable", and the first such stop hid every
//! later finding. Each class has a fixture here; every fixture file is
//! opened in the server and checked by the binary, and the two lists are
//! compared. Names nothing reads stay in the editor as hints (tagged
//! unnecessary), and those must be the genuinely unused ones only.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Client {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
            .args(["lsp", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn olang lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut c = Client {
            child,
            stdin,
            stdout,
        };
        c.send(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "capabilities": {} }
        }));
        while c.recv()["id"] != 1 {}
        c.send(&serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
        c
    }

    fn send(&mut self, msg: &serde_json::Value) {
        let body = serde_json::to_string(msg).unwrap();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self) -> serde_json::Value {
        let mut len = 0usize;
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "the server ended"
            );
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
        }
        let mut buf = vec![0u8; len];
        self.stdout.read_exact(&mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    /// The diagnostics the server publishes for `path` on open.
    fn diagnostics(&mut self, path: &Path) -> Vec<serde_json::Value> {
        let uri = format!("file://{}", path.display());
        let text = std::fs::read_to_string(path).unwrap();
        self.send(&serde_json::json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen","params":{
                "textDocument":{"uri":uri,"languageId":"olang","version":1,"text":text}}
        }));
        loop {
            let m = self.recv();
            if m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri {
                return m["params"]["diagnostics"].as_array().unwrap().clone();
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A fresh project directory holding `files`.
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_lsp_agree_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
    // the temp directory's real path: the server resolves imports from
    // the document's directory, the checker from the file's
    dir.canonicalize().unwrap()
}

/// `olang check FILE`: (errors, warnings) and its whole report.
fn check(file: &Path) -> (usize, usize, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("check")
        .arg(file)
        .current_dir(file.parent().unwrap())
        .output()
        .expect("run olang check");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let number_before = |word: &str| -> usize {
        report
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .rev()
            .find(|w| w[1].trim_end_matches(',').starts_with(word))
            .and_then(|w| w[0].parse().ok())
            .unwrap_or(0)
    };
    (number_before("problem"), number_before("warning"), report)
}

/// The server's (errors, warnings, hints) for `file`, and each error and
/// warning is one the checker reported (its message's start is in the
/// checker's report).
fn agree(c: &mut Client, file: &Path) -> Vec<serde_json::Value> {
    let ds = c.diagnostics(file);
    let (errors, warnings, report) = check(file);
    let count = |sev: u64| ds.iter().filter(|d| d["severity"] == sev).count();
    let flat: String = report.split_whitespace().collect::<Vec<_>>().join(" ");
    for d in ds
        .iter()
        .filter(|d| d["severity"] == 1 || d["severity"] == 2)
    {
        let msg = d["message"].as_str().unwrap();
        let head: String = msg.chars().take(28).collect();
        let head = head.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains(&head),
            "{}: the server says {msg:?}, the checker does not:\n{report}",
            file.display()
        );
    }
    assert_eq!(
        (count(1), count(2)),
        (errors, warnings),
        "{}: server {ds:#?}\nchecker:\n{report}",
        file.display()
    );
    ds
}

fn hinted(ds: &[serde_json::Value]) -> Vec<String> {
    let mut v: Vec<String> = ds
        .iter()
        .filter(|d| d["severity"] == 4)
        .map(|d| {
            assert_eq!(
                d["tags"],
                serde_json::json!([1]),
                "an unused name is tagged unnecessary"
            );
            d["message"]
                .as_str()
                .unwrap()
                .trim_start_matches("unused variable: ")
                .to_string()
        })
        .collect();
    v.sort();
    v
}

const UTIL: &str = "\
share let LIMIT = 3
share fn find(xs, v) = filter(xs, (x) => x == v)
share fn task(n) = n + 1
share type Shade = enum { Red, Blue }
";

#[test]
fn an_imported_name_the_language_also_provides_is_not_a_duplicate() {
    // `use loom { task, find, split }`: Studio, Loom and open-track each
    // showed "Duplicate variable" over an import line.
    let dir = project(
        "builtin_import",
        &[
            ("lib/util.ol", UTIL),
            (
                "main.ol",
                "use lib.util { find, task }\nprintln(show(find([1, 2], 2)))\nprintln(show(task(1)))\n",
            ),
        ],
    );
    let mut c = Client::start();
    let ds = agree(&mut c, &dir.join("main.ol"));
    assert!(ds.iter().all(|d| d["severity"] != 1), "no error: {ds:#?}");
    assert_eq!(hinted(&ds), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_constant_below_the_function_that_reads_it_is_defined_and_used() {
    // Studio's `PK_BUILTIN` (core/plugins.ol), Loom's `TEXT_STEPS`: a
    // top-level `let` written after the function body naming it.
    let dir = project(
        "forward_const",
        &[(
            "main.ol",
            "fn steps_up(cur) = fold(STEPS, STEPS[0], (acc, v) => if v > cur => v else => acc)\n\
             fn scaled(xs) = map(xs, (x) => x * SCALE)\n\
             let STEPS = [1.0, 1.5, 2.0]\n\
             let SCALE = 2\n\
             println(show(steps_up(1.2)))\n\
             println(show(scaled([1])))\n",
        )],
    );
    let mut c = Client::start();
    let ds = agree(&mut c, &dir.join("main.ol"));
    assert!(ds.iter().all(|d| d["severity"] != 1), "{ds:#?}");
    assert_eq!(hinted(&ds), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cross_module_constants_reexports_and_variants_resolve() {
    let dir = project(
        "cross_module",
        &[
            ("lib/util.ol", UTIL),
            // a re-export is the module's interface, never unused
            (
                "api.ol",
                "share use lib.util { find, LIMIT, Shade, Red, Blue }\n",
            ),
            (
                "main.ol",
                "use api { find, LIMIT, Red, Blue }\n\
                 fn tone(s) = match s {\n    Red => \"warm\"\n    Blue => \"cool\"\n}\n\
                 fn under(xs) = filter(xs, (x) => x < LIMIT)\n\
                 println(tone(Red))\n\
                 println(show(under([1, 5])))\n\
                 println(show(find([1], 1)))\n",
            ),
        ],
    );
    let mut c = Client::start();
    for f in ["api.ol", "main.ol"] {
        let ds = agree(&mut c, &dir.join(f));
        assert!(ds.iter().all(|d| d["severity"] != 1), "{f}: {ds:#?}");
        assert_eq!(hinted(&ds), Vec::<String>::new(), "{f}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uses_inside_lambdas_guards_templates_and_asserts_count() {
    // One redeclared name (`let n` twice, legal shadowing) used to stop
    // the walk, so every use below it was missed and every name above it
    // reported unused.
    let dir = project(
        "uses",
        &[(
            "main.ol",
            "let n = 1\nlet n = n + 1\n\
             let captured = 10\n\
             let in_guard = 3\n\
             let in_template = \"x\"\n\
             let in_assert = 4\n\
             let truly_unused = 5\n\
             let _on_purpose = 6\n\
             let add = (x) => x + captured\n\
             fn size(v) = match v {\n    x if x > in_guard => \"big\"\n    _ => \"small\"\n}\n\
             println(show(add(n)))\n\
             println(size(4))\n\
             println(`${in_template}!`)\n\
             test \"asserts read names\" {\n    assert_eq(in_assert, 4)\n}\n",
        )],
    );
    let mut c = Client::start();
    let ds = agree(&mut c, &dir.join("main.ol"));
    assert!(ds.iter().all(|d| d["severity"] != 1), "{ds:#?}");
    assert_eq!(hinted(&ds), vec!["truly_unused".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn genuine_findings_are_still_reported_and_agree() {
    let dir = project(
        "genuine",
        &[
            ("lib/util.ol", UTIL),
            (
                "main.ol",
                "use lib.util { find }\n\
                 fn f(x) = nowhere_defined(x) + 1\n\
                 let fixed = 1\n\
                 fixed = 2\n\
                 fn g() = {\n    let cell = 1\n    cell + 1\n}\n\
                 println(show(f(1) + g() + fixed))\n\
                 println(show(find([1], 1)))\n",
            ),
        ],
    );
    let mut c = Client::start();
    let ds = agree(&mut c, &dir.join("main.ol"));
    let errors: Vec<&str> = ds
        .iter()
        .filter(|d| d["severity"] == 1)
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    assert!(
        errors
            .iter()
            .any(|m| m.starts_with("Undefined variable: nowhere_defined")),
        "{errors:?}"
    );
    assert!(errors.iter().any(|m| m.contains("fixed")), "{errors:?}");
    assert!(
        ds.iter()
            .any(|d| d["severity"] == 2 && d["message"].as_str().unwrap().contains("cell")),
        "the shadowing advisory is a warning: {ds:#?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The checker's shadow advisories, taken the cheap way (only the
/// statements that can warn are turned into meta nodes), are the full
/// conversion's findings exactly — over every program in the repository,
/// plus the cases each half of the shortcut exists for.
#[test]
fn shadow_advisories_taken_cheaply_are_the_full_findings() {
    fn ol_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                ol_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "ol") {
                out.push(p);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for d in ["examples", "tests", "std", "lib"] {
        ol_files(&root.join(d), &mut files);
    }
    let mut sources: Vec<String> = files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .collect();
    sources.push(
        "use lib.util { find, task }\nfn f(find, n) = find(n)\nfn g(task) = task\n".to_string(),
    );
    sources.push(
        "use app.fs\nfn h() = {\n    let mut cell = 1\n    cell\n}\nlet json = 2\n".to_string(),
    );
    sources.push("use app.fs\nuse lib.util { find }\nfn k(find) = find(1)\n".to_string());
    let key = |ds: Vec<olang::tools::check::CheckDiagnostic>| -> Vec<(u32, u32, String)> {
        ds.into_iter()
            .map(|d| (d.line, d.column, d.message))
            .collect()
    };
    let parser = olang::parser::Parser::new();
    let mut compared = 0;
    let mut warned = 0;
    for src in &sources {
        let Ok(program) = parser.parse_raw(src) else {
            continue;
        };
        let full = key(olang::tools::check::shadow_warnings(&program));
        let cheap = key(olang::tools::check::shadow_warnings_in(&program, src));
        assert_eq!(cheap, full, "in:\n{src}");
        compared += 1;
        warned += usize::from(!full.is_empty());
    }
    assert!(
        compared > 50 && warned >= 2,
        "{compared} compared, {warned} with findings"
    );
}

/// `olang check --format json FILE`: its exit code and document.
fn check_json(file: &Path) -> (i32, serde_json::Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["check", "--format", "json"])
        .arg(file)
        .current_dir(file.parent().unwrap())
        .output()
        .expect("run olang check --format json");
    let doc = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code().unwrap_or(-1), doc)
}

#[test]
fn check_as_json_is_what_the_server_publishes() {
    let dir = project(
        "json",
        &[
            ("lib/util.ol", UTIL),
            (
                "main.ol",
                "use lib.util { find }\nfn f(x: Int) -> Int = x + 1\nlet unused_thing = 3\nlet s = \"😀\"; let e = f(s)\nlet n = 1 == \"a\"\nprintln(find([1], 1))\n",
            ),
            ("bad.ol", "fn f(x) = {\nlet y = \n}\n"),
        ],
    );
    let mut c = Client::start();
    let main = dir.join("main.ol");
    let served = c.diagnostics(&main);
    let (code, doc) = check_json(&main);
    assert_eq!(code, 1, "an error: exit 1 as the text form ({doc})");
    assert_eq!(doc["kind"], "check");
    let file = &doc["files"][0];
    assert_eq!(file["file"].as_str().unwrap(), main.to_str().unwrap());
    let ds = file["diagnostics"].as_array().unwrap();
    assert_eq!(ds.len(), served.len(), "{ds:#?}\nserver: {served:#?}");
    let sev = |n: &serde_json::Value| match n.as_u64() {
        Some(1) => "error",
        Some(2) => "warning",
        Some(3) => "information",
        _ => "hint",
    };
    for (d, s) in ds.iter().zip(served.iter()) {
        assert_eq!(d["message"], s["message"]);
        assert_eq!(d["range"], s["range"]);
        assert_eq!(d["code"], s["code"]);
        assert_eq!(d["severity"], sev(&s["severity"]));
        assert_eq!(
            d["line"].as_u64().unwrap(),
            s["range"]["start"]["line"].as_u64().unwrap() + 1
        );
    }
    let by_code = |c: &str| ds.iter().filter(|d| d["code"] == c).count();
    assert_eq!(by_code("runtime"), 1, "{ds:#?}");
    assert_eq!(by_code("advisory"), 1, "{ds:#?}");
    assert!(by_code("unused") >= 1, "{ds:#?}");
    let unused = ds.iter().find(|d| d["code"] == "unused").unwrap();
    assert_eq!(unused["severity"], "hint");
    assert_eq!(unused["tags"], serde_json::json!(["unnecessary"]));
    // a column counts characters (the server's range counts UTF-16 units)
    let typed = ds.iter().find(|d| d["code"] == "runtime").unwrap();
    assert_eq!(
        (typed["line"].as_u64(), typed["column"].as_u64()),
        (Some(4), Some(14)),
        "{typed}"
    );
    assert_eq!(
        typed["range"]["start"]["character"], 14,
        "after a character of two UTF-16 units: {typed}"
    );
    assert_eq!(doc["errors"], 1);
    assert_eq!(doc["warnings"], 1);

    // a syntax error, and a folder of files
    let (code, doc) = check_json(&dir);
    assert_eq!(code, 1);
    let files = doc["files"].as_array().unwrap();
    assert_eq!(files.len(), 3, "{doc}");
    let bad = files
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with("bad.ol"))
        .unwrap();
    assert_eq!(bad["diagnostics"][0]["code"], "syntax");
    let util = files
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with("util.ol"))
        .unwrap();
    assert!(
        util["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["severity"] != "error")
    );
}

#[test]
fn a_unit_binding_is_not_a_tuple() {
    // `let mut fg = ()` set to a String later is a placeholder, and any
    // value compared with `()` is the presence test: no always-true or
    // always-false advisory (Studio's tui/view.ol)
    let dir = project(
        "unit",
        &[(
            "main.ol",
            "fn f(k) = {\n    let mut fg = ()\n    let mut bg = ()\n    if k > 1 => { fg = \"danger\" }\n    if k > 2 => { bg = \"selection\" }\n    let u = ()\n    (fg != \"danger\", bg == \"selection\", u == \"x\", \"y\" != ())\n}\nprintln(f(3))\nlet t = (1, 2) == \"a\"\nprintln(t)\n",
        )],
    );
    let main = dir.join("main.ol");
    let mut c = Client::start();
    let ds = agree(&mut c, &main);
    let warnings: Vec<&str> = ds
        .iter()
        .filter(|d| d["severity"] == 2)
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    // a real tuple against a String is still said
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("between Tuple and String"),
        "{warnings:?}"
    );
}

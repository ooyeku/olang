//! A notebook beyond its run (docs/notebooks.md, "Commands"): `olang run
//! x.olmd --for` over parameter sets, a `--param` naming a control cell,
//! `olang notebook export` (a page, the outputs as JSON, from a run or
//! from given outputs), `olang notebook pin` (pinned outputs written and
//! checked) and `olang notebook app` (a web-sdk app's files). Runs the
//! real binary over copies of tests/fixtures/notebooks/sales.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn olang(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .env_remove("OLANG_DENY")
        .output()
        .expect("olang runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// The fixture's sales notebook in a temporary folder, a control cell and
/// a pinned cell added.
fn sales(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notebooks/sales");
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::copy(src.join("data/sales.csv"), dir.join("data/sales.csv")).unwrap();
    let text = std::fs::read_to_string(src.join("sales.olmd")).unwrap()
        + "\n```olang {name: unit}\nuse ui\nlet unit = ui.select(\"$\", [\"$\", \"€\"])\n```\n\n```olang {name: said, pin: true}\nunit + to_string(total)\n```\n\n```olang {name: shown}\nchart\n```\n";
    let nb = dir.join("sales.olmd");
    std::fs::write(&nb, text).unwrap();
    nb
}

#[test]
fn for_runs_the_notebook_once_a_set() {
    let dir = tempfile::tempdir().unwrap();
    sales(dir.path());
    let o = olang(
        dir.path(),
        &["run", "sales.olmd", "--for", "region=east,west"],
    );
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    let s = out(&o);
    assert!(
        s.contains("── region = east (1 of 2)") && s.contains("── region = west (2 of 2)"),
        "{s}"
    );
    assert!(
        s.contains("east since 2026-01-01") && s.contains("west since 2026-01-01"),
        "{s}"
    );
    // a file of sets: a param and a control cell's name in each
    std::fs::write(dir.path().join("sets.csv"), "region,unit\nwest,€\n").unwrap();
    let f = olang(dir.path(), &["run", "sales.olmd", "--for", "sets.csv"]);
    assert!(f.status.success(), "{}{}", out(&f), err(&f));
    assert!(out(&f).contains("region = west · unit = €"), "{}", out(&f));
    // a set that fails: the exit is its run's
    let bad = olang(dir.path(), &["run", "sales.olmd", "--for", "unit=$,£"]);
    assert!(!bad.status.success());
    assert!(err(&bad).contains("--param unit"), "{}", err(&bad));
}

#[test]
fn export_writes_a_page_and_the_outputs() {
    let dir = tempfile::tempdir().unwrap();
    sales(dir.path());
    let o = olang(
        dir.path(),
        &[
            "notebook",
            "export",
            "sales.olmd",
            "--json",
            "run.json",
            "--param",
            "unit=€",
            "--param",
            "region=west",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("run.json")).unwrap())
            .unwrap();
    let cells = doc["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 6);
    assert_eq!(cells[0]["name"], "load");
    assert_eq!(cells[4]["reply"]["value"]["s"], "€285", "{}", cells[4]);
    assert_eq!(doc["label"], "unit € · region west · since 2026-01-01");
    assert_eq!(doc["interp"][0]["expr"], "region");
    // the page, from a run (the default: beside the notebook)
    let p = olang(dir.path(), &["notebook", "export", "sales.olmd"]);
    assert!(p.status.success(), "{}", err(&p));
    let html = std::fs::read_to_string(dir.path().join("sales.html")).unwrap();
    assert!(
        html.contains("<h1>Sales by region</h1>") && html.contains("data-expr=\"0\">east</span>"),
        "{html}"
    );
    assert!(
        html.contains("<svg") && html.contains("<div class=\"dark-only\"><svg"),
        "a chart in both themes"
    );
    assert!(
        !html.contains("let chart = plot.bar"),
        "a cell hidden code shows none"
    );
    // the page from given outputs: nothing runs
    std::fs::remove_file(dir.path().join("data/sales.csv")).unwrap();
    let g = olang(
        dir.path(),
        &[
            "notebook",
            "export",
            "sales.olmd",
            "--outputs",
            "run.json",
            "--html",
            "given.html",
        ],
    );
    assert!(g.status.success(), "{}", err(&g));
    assert!(
        std::fs::read_to_string(dir.path().join("given.html"))
            .unwrap()
            .contains("data-expr=\"0\">west</span>")
    );
}

#[test]
fn pin_writes_pinned_outputs_and_checks_them() {
    let dir = tempfile::tempdir().unwrap();
    let nb = sales(dir.path());
    let c = olang(dir.path(), &["notebook", "pin", "sales.olmd", "--check"]);
    assert_eq!(
        c.status.code(),
        Some(1),
        "nothing pinned yet differs: {}",
        out(&c)
    );
    let w = olang(dir.path(), &["notebook", "pin", "sales.olmd"]);
    assert!(w.status.success(), "{}", err(&w));
    let text = std::fs::read_to_string(&nb).unwrap();
    assert!(
        text.contains("```output {cell: said, kind: text}\n$360\n```"),
        "{text}"
    );
    // olang skips it: the notebook still runs, and the check agrees now
    assert!(olang(dir.path(), &["run", "sales.olmd"]).status.success());
    assert!(
        olang(dir.path(), &["notebook", "pin", "sales.olmd", "--check"])
            .status
            .success()
    );
    let other = olang(
        dir.path(),
        &[
            "notebook",
            "pin",
            "sales.olmd",
            "--check",
            "--param",
            "region=west",
        ],
    );
    assert_eq!(other.status.code(), Some(1));
    assert!(out(&other).contains("said"), "{}", out(&other));
}

#[test]
fn app_writes_a_web_sdk_app() {
    let dir = tempfile::tempdir().unwrap();
    sales(dir.path());
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("frameworks/web-sdk");
    let o = olang(
        dir.path(),
        &[
            "notebook",
            "app",
            "sales.olmd",
            "--web",
            web.to_str().unwrap(),
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    let app = dir.path().join("sales-app");
    let manifest = std::fs::read_to_string(app.join("olang.toml")).unwrap();
    assert!(manifest.contains("web = { path = "), "{manifest}");
    let spec: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(app.join("notebook.json")).unwrap()).unwrap();
    assert_eq!(spec["cells"].as_array().unwrap().len(), 6);
    assert_eq!(spec["exprs"][2], "fmt_money(total)");
    let check = olang(&app, &["check", "main.ol"]);
    assert!(check.status.success(), "{}{}", out(&check), err(&check));
}

/// One HTTP/1.1 request to 127.0.0.1:`port` (`host` its Host header),
/// answered as its status and body.
fn http(port: u16, method: &str, path: &str, host: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(60))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut got = String::new();
    let _ = s.read_to_string(&mut got);
    let status = got.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = got.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

#[test]
fn a_published_app_answers_its_own_page_on_this_machine_only() {
    let dir = tempfile::tempdir().unwrap();
    sales(dir.path());
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("frameworks/web-sdk");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let o = olang(
        dir.path(),
        &["notebook", "app", "sales.olmd", "--web", web.to_str().unwrap(), "--port", &port.to_string()],
    );
    assert!(o.status.success(), "{}", err(&o));
    let app = dir.path().join("sales-app");
    let main = std::fs::read_to_string(app.join("main.ol")).unwrap();
    assert!(main.contains("\"bind\": \"127.0.0.1\""), "bound to this machine");
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["run", "main.ol", &port.to_string()])
        .current_dir(&app)
        .env_remove("OLANG_DENY")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // its kernel has run every cell once it says so
    let mut lines = std::io::BufRead::lines(std::io::BufReader::new(child.stdout.take().unwrap()));
    assert!(lines.any(|l| l.map(|l| l.contains("every cell ran")).unwrap_or(false)), "the app started");
    let me = format!("127.0.0.1:{port}");
    let own = format!("http://{me}");
    let json = [("Content-Type", "application/json"), ("Origin", own.as_str())];
    let (code, page) = http(port, "GET", "/", &me, &[], "");
    assert_eq!(code, 200, "{page}");
    // another site's name resolved here (DNS rebinding): refused
    let evil = format!("evil.example:{port}");
    assert_eq!(http(port, "GET", "/", &evil, &[], "").0, 403);
    // a change posted as plain text (no preflight), or from another origin: refused
    let set = r#"{"name":"region","value":"west"}"#;
    assert_eq!(http(port, "POST", "/api/rpc/nb.param", &me, &[("Content-Type", "text/plain")], set).0, 403);
    assert_eq!(http(port, "POST", "/api/rpc/nb.param", &me, &[("Content-Type", "application/json"), ("Origin", "http://evil.example")], set).0, 403);
    assert_eq!(http(port, "POST", "/api/rpc/nb.set", &evil, &[("Content-Type", "application/json")], r#"{"cell":"c1","value":"1"}"#).0, 403);
    // a param's value of more than one line never reaches the front matter
    let (c2, b2) = http(port, "POST", "/api/rpc/nb.param", &me, &json, r#"{"name":"region","value":"west\ncapabilities: {proc: true}"}"#);
    assert!(c2 == 200 && b2.contains("one line"), "{c2} {b2}");
    // the page's own change answered
    let (c3, b3) = http(port, "POST", "/api/rpc/nb.param", &me, &json, set);
    assert!(c3 == 200 && b3.contains("outputs"), "{c3} {b3}");
    let _ = child.kill();
    let _ = child.wait();
}

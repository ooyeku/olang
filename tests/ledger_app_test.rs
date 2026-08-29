//! The ledger example (`examples/web/ledger/`) is a real backend: this test
//! boots the actual main.ol on an ephemeral port with an in-memory
//! database and locks the API contract: validation envelopes, the month
//! window, budget upserts, category guards, and 404/405/400 semantics.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn app_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/web/ledger")
}

fn spawn_ledger(token: Option<&str>) -> (KillOnDrop, u16) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    cmd.arg("main.ol")
        .arg("0")
        .arg(":memory:")
        .current_dir(app_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match token {
        Some(t) => {
            cmd.env("LEDGER_TOKEN", t);
        }
        None => {
            cmd.env_remove("LEDGER_TOKEN");
        }
    }
    let mut child = cmd.spawn().expect("spawn ledger");

    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let port = loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read ledger stdout");
        assert!(n > 0, "ledger exited before reporting its port");
        if let Some(rest) = line.trim().strip_prefix("listening on http://127.0.0.1:") {
            break rest.parse::<u16>().expect("port");
        }
    };
    // Drain the access log so the server never blocks on a full pipe.
    std::thread::spawn(move || {
        let mut sink = String::new();
        loop {
            sink.clear();
            if reader.read_line(&mut sink).unwrap_or(0) == 0 {
                break;
            }
        }
    });
    (KillOnDrop(child), port)
}

fn call(port: u16, method: &str, path: &str, body: &str, extra_headers: &str) -> (u16, String) {
    let raw = format!(
        "{method} {path} HTTP/1.1\r\nHost: l\r\nConnection: close\r\n{extra_headers}Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(raw.as_bytes()).expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read");
    let status: u16 = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status line");
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

#[test]
fn transactions_budgets_categories_and_errors() {
    let (_guard, port) = spawn_ledger(None);

    // Health names the service; first run seeds categories, no transactions.
    let (status, body) = call(port, "GET", "/api/health", "", "");
    assert_eq!(status, 200);
    assert!(body.contains("\"olang-ledger\""), "got: {body}");
    assert!(body.contains("\"categories\":8"), "got: {body}");
    assert!(body.contains("\"transactions\":0"), "got: {body}");

    // Create: valid → 201 echoing the row joined with its category.
    let (status, body) = call(
        port,
        "POST",
        "/api/transactions",
        r#"{"date":"2026-08-15","amount_cents":-1250,"category_id":1,"note":"coffee"}"#,
        "",
    );
    assert_eq!(status, 201, "got: {body}");
    assert!(body.contains("\"amount_cents\":-1250"));
    assert!(
        body.contains("\"category\":"),
        "row carries the joined name: {body}"
    );

    // Create: invalid → 422 listing each field problem.
    let (status, body) = call(
        port,
        "POST",
        "/api/transactions",
        r#"{"date":"nope","amount_cents":0,"category_id":0}"#,
        "",
    );
    assert_eq!(status, 422, "got: {body}");
    assert!(body.contains("\"date\""));
    assert!(body.contains("\"amount_cents\""));
    assert!(body.contains("\"category_id\""));

    // The month window: a July row stays out of an August-only window.
    let (status, _) = call(
        port,
        "POST",
        "/api/transactions",
        r#"{"date":"2026-07-02","amount_cents":-500,"category_id":1,"note":"july"}"#,
        "",
    );
    assert_eq!(status, 201);
    let (_, aug) = call(
        port,
        "GET",
        "/api/transactions?from=2026-08&to=2026-08",
        "",
        "",
    );
    assert!(
        aug.contains("coffee") && !aug.contains("july"),
        "got: {aug}"
    );
    let (_, both) = call(
        port,
        "GET",
        "/api/transactions?from=2026-07&to=2026-08",
        "",
        "",
    );
    assert!(
        both.contains("coffee") && both.contains("july"),
        "got: {both}"
    );

    // PATCH is partial; an unknown id is 404; a bad id is 400.
    let (status, body) = call(
        port,
        "PATCH",
        "/api/transactions/1",
        r#"{"amount_cents":-1300}"#,
        "",
    );
    assert_eq!(status, 200, "got: {body}");
    assert!(body.contains("\"amount_cents\":-1300"));
    let (status, _) = call(
        port,
        "PATCH",
        "/api/transactions/999",
        r#"{"note":"x"}"#,
        "",
    );
    assert_eq!(status, 404);
    let (status, _) = call(
        port,
        "PATCH",
        "/api/transactions/abc",
        r#"{"note":"x"}"#,
        "",
    );
    assert_eq!(status, 400);

    // Budgets: upsert twice, then clear with zero.
    let (status, body) = call(
        port,
        "PUT",
        "/api/budgets",
        r#"{"category_id":1,"month":"2026-08","amount_cents":50000}"#,
        "",
    );
    assert_eq!(status, 200, "got: {body}");
    assert!(body.contains("\"amount_cents\":50000"));
    let (_, body) = call(
        port,
        "PUT",
        "/api/budgets",
        r#"{"category_id":1,"month":"2026-08","amount_cents":60000}"#,
        "",
    );
    assert!(body.contains("\"amount_cents\":60000"));
    let (_, listed) = call(port, "GET", "/api/budgets?from=2026-08&to=2026-08", "", "");
    assert!(listed.contains("\"amount_cents\":60000"), "got: {listed}");
    let (status, _) = call(
        port,
        "PUT",
        "/api/budgets",
        r#"{"category_id":1,"month":"2026-08","amount_cents":0}"#,
        "",
    );
    assert_eq!(status, 200);
    let (_, listed) = call(port, "GET", "/api/budgets?from=2026-08&to=2026-08", "", "");
    assert!(!listed.contains("60000"), "cleared: {listed}");

    // Budgets: a malformed month is a 422.
    let (status, body) = call(
        port,
        "PUT",
        "/api/budgets",
        r#"{"category_id":1,"month":"2026-8","amount_cents":100}"#,
        "",
    );
    assert_eq!(status, 422, "got: {body}");

    // Categories: create, duplicate name 422, in-use delete 409,
    // unused delete 200.
    let (status, body) = call(
        port,
        "POST",
        "/api/categories",
        r#"{"name":"Books","kind":"expense"}"#,
        "",
    );
    assert_eq!(status, 201, "got: {body}");
    let new_id: u64 = body
        .split("\"id\":")
        .nth(1)
        .and_then(|s| s.split([',', '}']).next())
        .and_then(|s| s.trim().parse().ok())
        .expect("new category id");
    let (status, _) = call(
        port,
        "POST",
        "/api/categories",
        r#"{"name":"Books","kind":"expense"}"#,
        "",
    );
    assert_eq!(status, 422);
    let (status, _) = call(port, "DELETE", "/api/categories/1", "", "");
    assert_eq!(status, 409, "category 1 has transactions");
    let (status, _) = call(port, "DELETE", &format!("/api/categories/{new_id}"), "", "");
    assert_eq!(status, 200);

    // Delete a transaction; the second delete is a 404.
    let (status, _) = call(port, "DELETE", "/api/transactions/1", "", "");
    assert_eq!(status, 200);
    let (status, _) = call(port, "DELETE", "/api/transactions/1", "", "");
    assert_eq!(status, 404);

    // Route semantics: unknown path 404; wrong method 405 with Allow;
    // bad JSON body 400.
    let (status, _) = call(port, "GET", "/api/nothing", "", "");
    assert_eq!(status, 404);
    let (status, body) = call(port, "DELETE", "/api/transactions", "", "");
    assert_eq!(status, 405, "got: {body}");
    let (status, _) = call(port, "POST", "/api/transactions", "{not json", "");
    assert_eq!(status, 400);

    // The page and the frontend source are served by the same process.
    let (status, page) = call(port, "GET", "/", "", "");
    assert_eq!(status, 200);
    assert!(page.contains("ledger"));
    let (status, src) = call(port, "GET", "/ledger.ol", "", "");
    assert_eq!(status, 200);
    assert!(src.contains("ledger.ol — the ledger frontend"));
}

#[test]
fn bearer_token_gates_writes_only() {
    let (_guard, port) = spawn_ledger(Some("s3cret"));

    // Reads stay open.
    let (status, _) = call(port, "GET", "/api/categories", "", "");
    assert_eq!(status, 200);

    // Writes without the token are refused; with it they pass.
    let tx = r#"{"date":"2026-08-15","amount_cents":-100,"category_id":1,"note":""}"#;
    let (status, _) = call(port, "POST", "/api/transactions", tx, "");
    assert_eq!(status, 401);
    let (status, _) = call(
        port,
        "POST",
        "/api/transactions",
        tx,
        "Authorization: Bearer s3cret\r\n",
    );
    assert_eq!(status, 201);
}

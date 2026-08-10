//! The tracker example (`examples/app/`) is a real backend: this test
//! boots it — the actual main.ol, not a copy — on an ephemeral port with
//! an in-memory database and locks the API contract: validation shapes,
//! filtering, comments, the audit trail, stats, CSV export, 404/405
//! semantics, and bearer-token auth.

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
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/app")
}

fn spawn_tracker(token: Option<&str>) -> (KillOnDrop, u16) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    cmd.arg("main.ol")
        .arg("0")
        .arg(":memory:")
        .current_dir(app_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match token {
        Some(t) => {
            cmd.env("TRACKER_TOKEN", t);
        }
        None => {
            cmd.env_remove("TRACKER_TOKEN");
        }
    }
    let mut child = cmd.spawn().expect("spawn tracker");

    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let port = loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read tracker stdout");
        assert!(n > 0, "tracker exited before reporting its port");
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
        "{method} {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n{extra_headers}Content-Length: {}\r\n\r\n{body}",
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
fn crud_validation_filters_and_stats() {
    let (_guard, port) = spawn_tracker(None);

    // Health names the service and counts the seed data.
    let (status, body) = call(port, "GET", "/health", "", "");
    assert_eq!(status, 200);
    assert!(body.contains("\"olang-tracker\""), "got: {body}");
    assert!(body.contains("\"issues\":4"), "got: {body}");

    // Create: valid → 201 with the row back.
    let (status, body) = call(
        port,
        "POST",
        "/api/issues",
        r#"{"title":"From the test","points":8,"status":"in-progress","assignee":"test"}"#,
        "",
    );
    assert_eq!(status, 201, "got: {body}");
    assert!(body.contains("\"From the test\""));

    // Create: invalid → 422 listing each field problem.
    let (status, body) = call(
        port,
        "POST",
        "/api/issues",
        r#"{"title":"","points":900,"status":"bogus"}"#,
        "",
    );
    assert_eq!(status, 422, "got: {body}");
    for field in ["title", "points", "status"] {
        assert!(
            body.contains(&format!("\"{field}\"")),
            "missing {field}: {body}"
        );
    }

    // Filtering + pagination envelope; rows carry their comment count.
    let (status, body) = call(port, "GET", "/api/issues?assignee=test&limit=2", "", "");
    assert_eq!(status, 200);
    assert!(body.contains("\"total\":1"), "got: {body}");
    assert!(body.contains("\"limit\":2"), "got: {body}");
    assert!(body.contains("\"comments\":0"), "got: {body}");

    // Search narrows; unknown sort column falls back safely.
    let (status, body) = call(port, "GET", "/api/issues?q=keyboard&sort=evil", "", "");
    assert_eq!(status, 200);
    assert!(body.contains("Spreadsheet keyboard nav"), "got: {body}");
    assert!(body.contains("\"total\":1"), "got: {body}");

    // Comments: 404 for a missing issue, then add + embed.
    let (status, _) = call(
        port,
        "POST",
        "/api/issues/999/comments",
        r#"{"text":"x"}"#,
        "",
    );
    assert_eq!(status, 404);
    let (status, _) = call(
        port,
        "POST",
        "/api/issues/1/comments",
        r#"{"text":"looks right","author":"test"}"#,
        "",
    );
    assert_eq!(status, 201);
    let (status, body) = call(port, "GET", "/api/issues/1", "", "");
    assert_eq!(status, 200);
    assert!(body.contains("\"looks right\""), "comments embed: {body}");

    // PATCH partial update; then the audit trail shows the whole story.
    let (status, body) = call(port, "PATCH", "/api/issues/1", r#"{"status":"done"}"#, "");
    assert_eq!(status, 200, "got: {body}");
    let (status, body) = call(port, "GET", "/api/activity?limit=10", "", "");
    assert_eq!(status, 200);
    for action in ["seed", "create", "comment", "update"] {
        assert!(
            body.contains(&format!("\"{action}\"")),
            "missing {action}: {body}"
        );
    }

    // Stats: SQL rollups plus ods point quantiles.
    let (status, body) = call(port, "GET", "/api/stats", "", "");
    assert_eq!(status, 200);
    assert!(body.contains("\"by_status\""), "got: {body}");
    assert!(body.contains("\"point_quantiles\""), "got: {body}");
    assert!(body.contains("\"p50\""), "got: {body}");

    // CSV export is real CSV with the header row.
    let (status, body) = call(port, "GET", "/api/export.csv", "", "");
    assert_eq!(status, 200);
    assert!(
        body.starts_with("id,title,status,priority,assignee,points,notes,updated"),
        "got: {body}"
    );

    // DELETE cascades and reports; a second delete 404s.
    let (status, body) = call(port, "DELETE", "/api/issues/1", "", "");
    assert_eq!(status, 200, "got: {body}");
    let (status, _) = call(port, "DELETE", "/api/issues/1", "", "");
    assert_eq!(status, 404);

    // Route semantics: 404 for unknown, 405 (with Allow) for wrong method,
    // 400 for a non-integer id, HEAD rides GET.
    let (status, _) = call(port, "GET", "/api/nope", "", "");
    assert_eq!(status, 404);
    let (status, body) = call(port, "PUT", "/api/issues/2", "", "");
    assert_eq!(status, 405);
    assert!(body.contains("method_not_allowed"), "got: {body}");
    let (status, body) = call(port, "GET", "/api/issues/abc", "", "");
    assert_eq!(status, 400);
    assert!(body.contains("bad_id"), "got: {body}");
    let (status, _) = call(port, "HEAD", "/health", "", "");
    assert_eq!(status, 200);
}

#[test]
fn bearer_token_gates_writes_only() {
    let (_guard, port) = spawn_tracker(Some("hunter2"));

    // Reads stay open.
    let (status, _) = call(port, "GET", "/api/issues", "", "");
    assert_eq!(status, 200);

    // Writes without the token refuse; with it, proceed.
    let (status, body) = call(port, "POST", "/api/issues", r#"{"title":"x"}"#, "");
    assert_eq!(status, 401, "got: {body}");
    let (status, body) = call(
        port,
        "POST",
        "/api/issues",
        r#"{"title":"authed"}"#,
        "Authorization: Bearer hunter2\r\n",
    );
    assert_eq!(status, 201, "got: {body}");
    let (status, _) = call(
        port,
        "POST",
        "/api/issues",
        r#"{"title":"wrong"}"#,
        "Authorization: Bearer nope\r\n",
    );
    assert_eq!(status, 401);
}

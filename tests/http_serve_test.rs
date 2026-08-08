//! `http.serve` really serves: spawn the olang binary running a tiny server
//! on an ephemeral port (port 0 — the OS assigns, the server reports), then
//! talk HTTP to it over a raw TCP stream and assert on the wire bytes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Kill the server child even when an assertion panics.
struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_server(source: &str) -> (KillOnDrop, u16) {
    let dir = std::env::temp_dir().join(format!("olang_http_it_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("server.ol");
    std::fs::write(&file, source).expect("write server source");

    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg(&file)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn olang server");

    // The server prints "listening on http://127.0.0.1:PORT" once bound.
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let port = loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read server stdout");
        assert!(n > 0, "server exited before reporting its port");
        if let Some(rest) = line.trim().strip_prefix("listening on http://127.0.0.1:") {
            break rest.parse::<u16>().expect("port number");
        }
    };
    (KillOnDrop(child), port)
}

fn request(port: u16, raw: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(raw.as_bytes()).expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

const SERVER: &str = r#"
fn handle(req) = {
    if req.path == "/ping" => req.method + " " + req.path + " pong"
    else if req.path == "/echo" => http.response_with_headers(200, req.body,
        #{ "Content-Type": "application/json" })
    else if req.path == "/q" => "q=" + to_string(map_get(req.query, "q"))
    else => http.response(404, "nope")
}
http.serve(0, handle)
"#;

#[test]
fn serves_get_requests_with_parsed_method_and_path() {
    let (_guard, port) = spawn_server(SERVER);
    let resp = request(port, "GET /ping HTTP/1.1\r\nHost: t\r\n\r\n");
    assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "got: {resp}");
    assert!(resp.contains("Content-Length: 14"), "got: {resp}");
    assert!(resp.ends_with("GET /ping pong"), "got: {resp}");
}

#[test]
fn serves_post_bodies_and_custom_headers() {
    let (_guard, port) = spawn_server(SERVER);
    let body = r#"{"n":1}"#;
    let raw = format!(
        "POST /echo HTTP/1.1\r\nHost: t\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let resp = request(port, &raw);
    assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "got: {resp}");
    assert!(
        resp.contains("Content-Type: application/json\r\n"),
        "got: {resp}"
    );
    assert!(resp.ends_with(body), "got: {resp}");
}

#[test]
fn parses_query_strings_and_reports_status_codes() {
    let (_guard, port) = spawn_server(SERVER);
    let resp = request(port, "GET /q?q=hello%20world HTTP/1.1\r\nHost: t\r\n\r\n");
    assert!(resp.contains("hello world"), "got: {resp}");

    let resp = request(port, "GET /missing HTTP/1.1\r\nHost: t\r\n\r\n");
    assert!(
        resp.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "got: {resp}"
    );

    // A malformed request is a 400, and the server keeps serving after it.
    let resp = request(port, "GARBAGE\r\n\r\n");
    assert!(
        resp.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "got: {resp}"
    );
    let resp = request(port, "GET /ping HTTP/1.1\r\nHost: t\r\n\r\n");
    assert!(resp.ends_with("GET /ping pong"), "still serving: {resp}");
}

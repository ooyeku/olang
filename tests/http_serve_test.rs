//! `http.serve` really serves: spawn the olang binary running a tiny server
//! on an ephemeral port (port 0 — the OS assigns, the server reports), then
//! talk HTTP to it over a raw TCP stream and assert on the wire bytes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

static NEXT_SERVER: AtomicU64 = AtomicU64::new(1);

/// Kill the server child even when an assertion panics.
struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_server(source: &str) -> (KillOnDrop, u16) {
    let id = NEXT_SERVER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("olang_http_it_{}_{}", std::process::id(), id));
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
    let resp = request(
        port,
        "GET /ping HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
    );
    assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "got: {resp}");
    assert!(resp.contains("Content-Length: 14"), "got: {resp}");
    assert!(resp.ends_with("GET /ping pong"), "got: {resp}");
}

#[test]
fn serves_post_bodies_and_custom_headers() {
    let (_guard, port) = spawn_server(SERVER);
    let body = r#"{"n":1}"#;
    let raw = format!(
        "POST /echo HTTP/1.1\r\nHost: t\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
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
    let resp = request(
        port,
        "GET /q?q=hello%20world HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
    );
    assert!(resp.contains("hello world"), "got: {resp}");

    let resp = request(
        port,
        "GET /missing HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
    );
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
    let resp = request(
        port,
        "GET /ping HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
    );
    assert!(resp.ends_with("GET /ping pong"), "still serving: {resp}");
}

/// Read exactly one HTTP response from the stream: headers, then
/// Content-Length body bytes. Leaves the connection open.
fn read_one_response(stream: &mut TcpStream) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    // headers
    while !buf.ends_with(b"\r\n\r\n") {
        let n = stream.read(&mut byte).expect("read header byte");
        assert!(n > 0, "connection closed mid-headers");
        buf.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let content_length: usize = head
        .lines()
        .find_map(|l| {
            l.to_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse().unwrap())
        })
        .expect("content-length header");
    let mut body = vec![0u8; content_length];
    stream.read_exact(&mut body).expect("read body");
    head + &String::from_utf8_lossy(&body)
}

#[test]
fn keep_alive_serves_multiple_requests_on_one_connection() {
    let (_guard, port) = spawn_server(SERVER);
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    // First request: no Connection header — HTTP/1.1 defaults to keep-alive.
    stream
        .write_all(b"GET /ping HTTP/1.1\r\nHost: t\r\n\r\n")
        .unwrap();
    let first = read_one_response(&mut stream);
    assert!(first.contains("Connection: keep-alive"), "got: {first}");
    assert!(first.ends_with("GET /ping pong"), "got: {first}");

    // Second request on the SAME connection.
    stream
        .write_all(b"GET /q?q=again HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut rest = String::new();
    stream.read_to_string(&mut rest).expect("read second");
    assert!(rest.contains("Connection: close"), "got: {rest}");
    assert!(rest.contains("again"), "got: {rest}");
}

#[test]
fn worker_pool_serves_independent_connections_concurrently() {
    let source = r#"
fn handle(req) = {
    time.sleep(500)
    req.remote_addr + " done"
}
http.serve(0, handle, #{ "workers": 2, "queue_capacity": 8 })
"#;
    let (_guard, port) = spawn_server(source);
    let barrier = Arc::new(Barrier::new(3));
    let mut clients = Vec::new();
    for _ in 0..2 {
        let barrier = Arc::clone(&barrier);
        clients.push(std::thread::spawn(move || {
            barrier.wait();
            request(
                port,
                "GET /slow HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
            )
        }));
    }

    let started = Instant::now();
    barrier.wait();
    for client in clients {
        let response = client.join().expect("client thread");
        assert!(response.ends_with("done"), "got: {response}");
        assert!(response.contains("127.0.0.1"), "remote address: {response}");
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(850),
        "two 500ms handlers took {elapsed:?}; connections ran sequentially"
    );
}

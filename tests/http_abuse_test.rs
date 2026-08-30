//! The HTTP abuse suite (W7): `http.serve` probed the way an adversary
//! would — malformed request lines, oversized headers and bodies,
//! unparseable Content-Length, slow-dripping clients, and header
//! injection through a handler — each answered with the right status
//! and a closed connection, while a well-formed request on the same
//! server keeps working. The limits under test are explicit `serve`
//! options (`max_header_bytes`, `max_body_bytes`, `request_timeout_ms`).

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

const SERVER: &str = r#"
fn handler(req) = {
    if req.path == "/inject" => {
        // A handler interpolating attacker-controlled text into a
        // header: the server must keep the response to one line per
        // header, whatever this contains.
        let evil = if map_has_key(req.query, "v") => map_get(req.query, "v") else => "x"
        { status: 200, headers: #{ "x-echo": evil }, body: "ok" }
    } else => {
        { status: 200, body: `echo ${req.method} ${req.path} ${len(req.body)}` }
    }
}
http.serve(0, handler, #{
    "request_timeout_ms": 1500,
    "max_header_bytes": 4096,
    "max_body_bytes": 2048,
    "idle_timeout_ms": 700
})
"#;

fn spawn_server() -> (KillOnDrop, u16) {
    let dir = std::env::temp_dir().join("olang_http_abuse_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("server.ol");
    std::fs::write(&path, SERVER).expect("write");
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("run")
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let port = loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read server stdout");
        assert!(n > 0, "server exited before reporting its port");
        if let Some(rest) = line.trim().strip_prefix("listening on http://127.0.0.1:") {
            break rest.parse::<u16>().expect("port");
        }
    };
    std::thread::spawn(move || {
        let mut sink = String::new();
        while reader.read_line(&mut sink).unwrap_or(0) > 0 {
            sink.clear();
        }
    });
    (KillOnDrop(child), port)
}

/// Send raw bytes, read the whole response (until the server closes or
/// both sides go quiet).
fn raw(port: u16, bytes: &[u8]) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(bytes).expect("write");
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    String::from_utf8_lossy(&out).to_string()
}

fn status_line(response: &str) -> &str {
    response.lines().next().unwrap_or("")
}

#[test]
fn the_gauntlet() {
    // One server, every probe — the well-formed check runs first and
    // last, proving the abuse in between broke nothing.
    let (_guard, port) = spawn_server();

    let well_formed = || {
        raw(
            port,
            b"POST /hello HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
        )
    };
    let ok = well_formed();
    assert!(status_line(&ok).contains("200"), "baseline:\n{ok}");
    assert!(ok.contains("echo POST /hello 3"), "baseline body:\n{ok}");

    // Not HTTP at all.
    let r = raw(port, b"\x16\x03\x01\x02\x00garbage\r\n\r\n");
    assert!(status_line(&r).contains("400"), "garbage:\n{r}");

    // A request line with no target.
    let r = raw(port, b"GET\r\n\r\n");
    assert!(status_line(&r).contains("400"), "no target:\n{r}");

    // A header block past max_header_bytes: 431.
    let mut big = b"GET / HTTP/1.1\r\n".to_vec();
    big.extend(std::iter::repeat_n(b'a', 8 * 1024));
    big.extend(b": x\r\n\r\n");
    let r = raw(port, &big);
    assert!(status_line(&r).contains("431"), "oversized header:\n{r}");

    // A body past max_body_bytes: 413, refused before it is read.
    let r = raw(port, b"POST / HTTP/1.1\r\nContent-Length: 1000000\r\n\r\n");
    assert!(status_line(&r).contains("413"), "oversized body:\n{r}");

    // A Content-Length that is not a number: 400, not a silent zero
    // that would desynchronize the kept-alive stream.
    let r = raw(port, b"POST / HTTP/1.1\r\nContent-Length: abc\r\n\r\nxxx");
    assert!(status_line(&r).contains("400"), "bad content-length:\n{r}");

    // Header injection through the handler: the evil value must stay on
    // one line — no injected header, no response splitting.
    let r = raw(
        port,
        b"GET /inject?v=a%0d%0ax-forged:%20yes HTTP/1.1\r\nConnection: close\r\n\r\n",
    );
    assert!(status_line(&r).contains("200"), "inject route:\n{r}");
    // The payload must not have become a header line of its own: it
    // stays inside x-echo's value, flattened onto one line.
    assert!(
        !r.lines().any(|l| l.starts_with("x-forged")),
        "header injected:\n{r}"
    );
    assert!(
        r.lines()
            .any(|l| l.starts_with("x-echo:") && l.contains("x-forged: yes")),
        "payload not flattened into the value:\n{r}"
    );

    let ok = well_formed();
    assert!(
        status_line(&ok).contains("200"),
        "server unhealthy after the gauntlet:\n{ok}"
    );
}

#[test]
fn a_dripping_client_is_cut_off_at_the_request_deadline() {
    let (_guard, port) = spawn_server();
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let started = std::time::Instant::now();
    // Drip a byte every 200 ms: each read beats the idle timeout, so
    // only the whole-request deadline (1500 ms here) can end this.
    let mut cut_off = false;
    for chunk in "GET /slow HTTP/1.1\r\nHost: x".as_bytes().chunks(1) {
        if s.write_all(chunk).is_err() {
            cut_off = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
        if started.elapsed() > Duration::from_secs(8) {
            break;
        }
    }
    // Either the write failed (connection closed under us) or the
    // response is a 408 and the stream then closes.
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out);
    assert!(
        cut_off || text.contains("408"),
        "drip survived past the deadline (elapsed {:?}):\n{}",
        started.elapsed(),
        text
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "worker held too long: {:?}",
        started.elapsed()
    );
}

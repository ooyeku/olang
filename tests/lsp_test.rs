//! Protocol-level tests for `olang lsp`: a minimal LSP client over the
//! real binary's stdio, covering the full loop — initialize, diagnostics
//! on open/change, completions, formatting, clean shutdown.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Client {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn olang lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Client {
            child,
            stdin,
            stdout,
        }
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
            self.stdout.read_line(&mut line).unwrap();
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

    fn recv_until(&mut self, pred: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
        for _ in 0..32 {
            let m = self.recv();
            if pred(&m) {
                return m;
            }
        }
        panic!("expected message not received");
    }
}

fn diagnostics_of(m: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    (m["method"] == "textDocument/publishDiagnostics").then(|| {
        m["params"]["diagnostics"]
            .as_array()
            .expect("diagnostics array")
    })
}

#[test]
fn full_protocol_loop() {
    let mut c = Client::start();
    let uri = "file:///probe.ol";

    c.send(&serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let init = c.recv_until(|m| m["id"] == 1);
    assert!(init["result"]["capabilities"]["completionProvider"].is_object());
    c.send(&serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}));

    // Broken file: an error diagnostic with a position on line 2.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri,"languageId":"olang","version":1,
                            "text":"fn f(x) = {\nlet y = \n}\n"}}
    }));
    let m = c.recv_until(|m| diagnostics_of(m).is_some());
    let ds = diagnostics_of(&m).unwrap();
    assert_eq!(ds[0]["severity"], 1);
    assert_eq!(ds[0]["range"]["start"]["line"], 2);

    // Fixed file with a top-level unused variable: a warning at its
    // declaration site.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri,"version":2},
            "contentChanges":[{"text":
                "fn f(x) = {\n    x + 1\n}\nlet unused_thing = 4\nprintln(show(f(1)))\n"}]}
    }));
    let m = c.recv_until(|m| diagnostics_of(m).is_some());
    let ds = diagnostics_of(&m).unwrap();
    let w = ds
        .iter()
        .find(|d| d["message"].as_str().unwrap().contains("unused"))
        .expect("unused-variable warning");
    assert_eq!(w["severity"], 2);
    assert_eq!(w["range"]["start"]["line"], 3);
    assert_eq!(w["range"]["start"]["character"], 4);

    // Completions carry keywords, builtins, modules, document symbols.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{
            "textDocument":{"uri":uri},"position":{"line":4,"character":0}}
    }));
    let comp = c.recv_until(|m| m["id"] == 2);
    let labels: Vec<&str> = comp["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    for want in ["par", "par_map", "ods", "println", "f", "unused_thing"] {
        assert!(labels.contains(&want), "missing completion: {want}");
    }

    // Formatting responds (empty edits for already-formatted text).
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":3,"method":"textDocument/formatting","params":{
            "textDocument":{"uri":uri},
            "options":{"tabSize":4,"insertSpaces":true}}
    }));
    let fmt = c.recv_until(|m| m["id"] == 3);
    assert!(fmt["result"].is_array());

    // Hover over the call site of f: shows the signature.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":4,"method":"textDocument/hover","params":{
            "textDocument":{"uri":uri},"position":{"line":4,"character":14}}
    }));
    let hov = c.recv_until(|m| m["id"] == 4);
    let val = hov["result"]["contents"]["value"].as_str().unwrap_or("");
    assert_eq!(val, "fn f(x)", "hover: {val}");

    // Go-to-definition from the call site lands on the declaration name.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":5,"method":"textDocument/definition","params":{
            "textDocument":{"uri":uri},"position":{"line":4,"character":14}}
    }));
    let def = c.recv_until(|m| m["id"] == 5);
    assert_eq!(def["result"]["range"]["start"]["line"], 0);
    assert_eq!(def["result"]["range"]["start"]["character"], 3);

    // Clean shutdown.
    c.send(&serde_json::json!({"jsonrpc":"2.0","id":9,"method":"shutdown","params":null}));
    c.recv_until(|m| m["id"] == 9);
    c.send(&serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}));
    drop(c.stdin);
    let status = c.child.wait().expect("server exit");
    assert!(status.success());
}

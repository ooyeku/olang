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
            .args(["lsp", "--stdio"])
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

/// Expansion-aware diagnostics: the semantic pass runs on the EXPANDED
/// program with every position translated back through the line map, so
/// calls to macro-generated functions are not errors, problems in
/// untouched code land on their true buffer line even after a decorator
/// shifted the expanded text, generated-code problems land on the `@`
/// site with the macro named, and a failing macro is a diagnostic at its
/// site rather than silence.
#[test]
fn macro_files_get_mapped_diagnostics() {
    let mut c = Client::start();
    c.send(&serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    c.recv_until(|m| m["id"] == 1);
    c.send(&serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    let uri = "file:///macro_probe.ol";
    let open = |c: &mut Client, version: u64, text: &str| {
        c.send(&serde_json::json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen","params":{
                "textDocument":{"uri":uri,"languageId":"olang","version":version,
                                "text":text}}
        }));
    };

    // 1. A generated function is called: no false "undefined" error, and
    //    the only diagnostic is the unused-variable warning in UNTOUCHED
    //    code — mapped to its true buffer line (4, 0-based 3) even though
    //    the decorator's output shifted the expanded text below it.
    open(
        &mut c,
        1,
        "meta fn grow(decl) = decl + `\nfn made() = 42`\n@grow\ntype P = struct { x: Int }\nlet unused_here = 1\nprintln(to_string(made()))\n",
    );
    let m = c.recv_until(|m| diagnostics_of(m).is_some());
    let ds = diagnostics_of(&m).unwrap();
    let msgs: Vec<String> = ds.iter().map(|d| d["message"].to_string()).collect();
    assert!(
        !msgs.iter().any(|s| s.contains("made")),
        "generated fn must not be flagged: {msgs:?}"
    );
    let unused = ds
        .iter()
        .find(|d| d["message"].as_str().unwrap_or("").contains("unused_here"))
        .expect("unused warning survives mapping");
    assert_eq!(
        unused["range"]["start"]["line"], 4,
        "warning must sit on the buffer's line, not the expanded line"
    );

    // 2. A macro that fails at expansion: an error diagnostic anchored at
    //    the @ site's line (2, 0-based).
    open(
        &mut c,
        2,
        "meta fn bad(e) = `${e} +`\nlet x = 1\nlet y = @bad(x)\nprintln(to_string(y))\n",
    );
    let m = c.recv_until(|m| diagnostics_of(m).is_some());
    let ds = diagnostics_of(&m).unwrap();
    assert_eq!(ds[0]["severity"], 1);
    assert_eq!(
        ds[0]["range"]["start"]["line"], 2,
        "expansion failure must anchor at the @ site: {ds:?}"
    );
    assert!(ds[0]["message"].as_str().unwrap_or("").contains("@bad"));

    // 3. A checker violation inside GENERATED code: reported at the @
    //    site, message prefixed with the macro's name.
    open(
        &mut c,
        3,
        "meta fn typed(decl) = decl + `\nfn bad_ret() -> Int = \"nope\"`\n@typed\ntype Q = struct { y: Int }\nprintln(to_string(Q { y: 1 }.y))\n",
    );
    let m = c.recv_until(|m| diagnostics_of(m).is_some());
    let ds = diagnostics_of(&m).unwrap();
    let generated = ds
        .iter()
        .find(|d| {
            d["message"]
                .as_str()
                .unwrap_or("")
                .contains("generated by @typed")
        })
        .expect("generated-code diagnostic must name the macro");
    assert_eq!(
        generated["range"]["start"]["line"], 2,
        "generated-code diagnostic must anchor at the @ site: {ds:?}"
    );
}

/// The rebuilt server: correct vocabularies, context-aware completion,
/// registry-driven hover, and the navigation set — with positions in
/// UTF-16 code units at every boundary, which is what the protocol
/// speaks and what the old char-indexed server got wrong on any line
/// containing non-ASCII.
#[test]
fn rebuilt_server_features() {
    let mut c = Client::start();
    c.send(&serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let init = c.recv_until(|m| m["id"] == 1);
    for cap in [
        "documentSymbolProvider",
        "referencesProvider",
        "renameProvider",
        "signatureHelpProvider",
        "documentHighlightProvider",
    ] {
        assert!(
            !init["result"]["capabilities"][cap].is_null(),
            "capability {cap} must be advertised"
        );
    }
    c.send(&serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}));

    let uri = "file:///features.ol";
    // Line 3 has a π (1 char, 2 UTF-16 units, 2 bytes) BEFORE the use of
    // `total`, so char-indexed and UTF-16 positions diverge on it.
    let text = "fn add(a, b) = a + b\nlet total = add(2, 3)\nlet label = `π = ${total}`\nprintln(label)\ntest \"adds\" { assert_eq(add(1, 1), 2) }\n";
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri,"languageId":"olang","version":1,"text":text}}
    }));
    c.recv_until(|m| diagnostics_of(m).is_some());

    // 1. Completion after `str.` is the str module, not keywords.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":10,"method":"textDocument/completion","params":{
            "textDocument":{"uri":uri},"position":{"line":0,"character":0},
            "context":{"triggerKind":1}}
    }));
    let m = c.recv_until(|m| m["id"] == 10);
    let labels: Vec<String> = m["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap().to_string())
        .collect();
    for gone in ["async", "await", "try", "catch"] {
        assert!(
            !labels.contains(&gone.to_string()),
            "removed keyword '{gone}' must not be completed"
        );
    }
    assert!(
        labels.contains(&"meta".to_string()),
        "meta must be completed"
    );
    assert!(labels.contains(&"add".to_string()), "file decls complete");

    // A synthetic `str.` line for module completion.
    let text2 = format!("{text}str.\n");
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri,"version":2},
            "contentChanges":[{"text": text2}]}
    }));
    c.recv_until(|m| diagnostics_of(m).is_some());
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":11,"method":"textDocument/completion","params":{
            "textDocument":{"uri":uri},"position":{"line":5,"character":4},
            "context":{"triggerKind":2,"triggerCharacter":"."}}
    }));
    let m = c.recv_until(|m| m["id"] == 11);
    let labels: Vec<String> = m["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap().to_string())
        .collect();
    assert!(
        labels.contains(&"trim".to_string()),
        "str. completes trim: {labels:?}"
    );
    assert!(
        !labels.contains(&"fn".to_string()),
        "str. must not complete keywords"
    );

    // 2. Hover on a builtin gives registry docs.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":12,"method":"textDocument/hover","params":{
            "textDocument":{"uri":uri},"position":{"line":3,"character":2}}
    }));
    let m = c.recv_until(|m| m["id"] == 12);
    let hover = m["result"]["contents"]["value"].as_str().unwrap_or("");
    assert!(
        hover.contains("println"),
        "builtin hover from the registry: {hover}"
    );

    // 3. UTF-16: on line 2 (`let label = ...`), hover `total` inside the
    //    template — after the π. The word starts at char 17… wire pos
    //    must be UTF-16. `total` occurs at chars 19..24; π adds one extra
    //    UTF-16 unit? (π is 1 unit — BMP — so char==utf16 here; use a
    //    surrogate-pair emoji to force divergence instead.)
    let text3 = format!("{text2}let wide = `😀 ${{total}}`\n");
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri,"version":3},
            "contentChanges":[{"text": text3}]}
    }));
    c.recv_until(|m| diagnostics_of(m).is_some());
    // Line 6: let wide = `😀 ${total}`  — chars: 😀 at 12; `total` chars
    // 17..22; UTF-16: emoji is 2 units, so total is units 18..23.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":13,"method":"textDocument/hover","params":{
            "textDocument":{"uri":uri},"position":{"line":6,"character":20}}
    }));
    let m = c.recv_until(|m| m["id"] == 13);
    let hover = m["result"]["contents"]["value"].as_str().unwrap_or("");
    assert!(
        hover.contains("total"),
        "UTF-16 position after an emoji must still land on `total`: {hover}"
    );

    // 4. Outline: functions, types, tests.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":14,"method":"textDocument/documentSymbol","params":{
            "textDocument":{"uri":uri}}
    }));
    let m = c.recv_until(|m| m["id"] == 14);
    let names: Vec<String> = m["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"add".to_string()));
    assert!(
        names.iter().any(|n| n.contains("adds")),
        "test block in outline: {names:?}"
    );

    // 5. References on `add` (declared line 0, used line 1): 3 with decl
    //    (fn add, add(2,3), assert_eq(add(1,1)) in the test).
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":15,"method":"textDocument/references","params":{
            "textDocument":{"uri":uri},"position":{"line":0,"character":4},
            "context":{"includeDeclaration":true}}
    }));
    let m = c.recv_until(|m| m["id"] == 15);
    assert_eq!(
        m["result"].as_array().unwrap().len(),
        3,
        "references to add"
    );

    // 6. Rename `total` → `sum_total`: edits at every occurrence.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":16,"method":"textDocument/rename","params":{
            "textDocument":{"uri":uri},"position":{"line":1,"character":5},
            "newName":"sum_total"}
    }));
    let m = c.recv_until(|m| m["id"] == 16);
    let edits = &m["result"]["changes"][uri];
    assert_eq!(
        edits.as_array().unwrap().len(),
        3,
        "total occurs thrice: {edits}"
    );

    // Renaming a builtin is refused with a reason.
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":17,"method":"textDocument/rename","params":{
            "textDocument":{"uri":uri},"position":{"line":3,"character":2},
            "newName":"shout"}
    }));
    let m = c.recv_until(|m| m["id"] == 17);
    assert!(
        m["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("standard-library"),
        "renaming println must be refused: {m}"
    );

    // 7. Signature help inside map_get's second argument.
    let text4 = format!("{text3}let v = map_get(#{{}}, \n");
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri,"version":4},
            "contentChanges":[{"text": text4}]}
    }));
    c.recv_until(|m| diagnostics_of(m).is_some());
    c.send(&serde_json::json!({
        "jsonrpc":"2.0","id":18,"method":"textDocument/signatureHelp","params":{
            "textDocument":{"uri":uri},"position":{"line":7,"character":22}}
    }));
    let m = c.recv_until(|m| m["id"] == 18);
    let sig = &m["result"]["signatures"][0];
    assert!(
        sig["label"].as_str().unwrap_or("").contains("map_get"),
        "{m}"
    );
    assert_eq!(
        m["result"]["activeParameter"], 1,
        "cursor is in the second argument: {m}"
    );
}

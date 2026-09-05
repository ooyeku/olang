//! Roadmap W12: the expansion scope as a closed list, positions on
//! test-body statements, the callee's name from a compiled frame,
//! `task.watch`, and the boot pin (image decode beats source parse).

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w12c_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

fn olang(dir: &Path, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn the_expansion_scope_is_exactly_the_stated_list() {
    let ws = workspace("scope");
    write(
        &ws,
        "lib/rules.ol",
        "share fn shout(s) = str.to_upper(s)\nfn hidden(s) = s\n",
    );
    // 1. own file's declarations; 2. an imported share fn; 3. the pure stdlib.
    write(
        &ws,
        "ok.ol",
        "use lib.rules { shout }\n\
         fn twice(s) = s + s\n\
         meta fn m(e) = `\"${twice(shout(str.trim(unwrap(meta.eval(e)))))}\"`\n\
         println(@m(\" hi \"))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "ok.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "HIHI\n");
    // Exclusions: an imported module's private fn, and an effectful module.
    write(
        &ws,
        "private.ol",
        "use lib.rules { shout }\nmeta fn m(e) = `\"${hidden(\"x\")}\"`\nprintln(@m(1))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "private.ol"]);
    assert_ne!(rc, 0);
    assert!(
        (out + &err).contains("hidden"),
        "private fn must be out of scope"
    );
    write(
        &ws,
        "effect.ol",
        "meta fn m(e) = `\"${unwrap(fs.read_file(\"/etc/hosts\"))}\"`\nprintln(@m(1))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "effect.ol"]);
    assert_ne!(rc, 0);
    let all = out + &err;
    assert!(
        all.contains("fs") && (all.contains("expansion") || all.contains("meta fn")),
        "{all}"
    );
}

#[test]
fn a_test_body_statement_carries_its_own_position() {
    let ws = workspace("pos");
    write(
        &ws,
        "t.ol",
        "test \"t\" {\n    let fs = 1\n    assert_eq(fs, 1)\n}\n",
    );
    let (out, err, _) = olang(&ws, &["check", "t.ol"]);
    let all = out + &err;
    // The shadow warning is placed at the `let`, line 2, not the block's line 1.
    assert!(all.contains("t.ol:2:"), "{all}");
    let (out, _, rc) = olang(
        &ws,
        &[
            "eval",
            "println(show(map_get(unwrap(meta.parse(\"test \\\"t\\\" {\\n    let q = 3\\n}\"))[0], \"body\")))",
        ],
    );
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("\"line\": 2"), "{out}");
}

#[test]
fn a_compiled_frame_names_the_callee_it_could_not_call() {
    let ws = workspace("callee");
    write(
        &ws,
        "c.ol",
        "fn row(s, span) = span(s)\n\
         let mut warm = 0\n\
         for i in range(0, 5) { warm = warm + 1 }\n\
         println(row(1, 2))\n",
    );
    for flags in [vec!["run", "c.ol"], vec!["--no-ovm", "run", "c.ol"]] {
        let (out, err, rc) = olang(&ws, &flags);
        assert_ne!(rc, 0);
        let all = out + &err;
        assert!(
            all.contains("'span' is an Int, not a function"),
            "{flags:?}: {all}"
        );
        assert!(!all.contains("'row' is"), "{flags:?}: {all}");
    }
}

#[test]
fn a_watched_channel_closes_when_its_task_dies() {
    let ws = workspace("watch");
    write(
        &ws,
        "w.ol",
        "let inbox = chan.new()\n\
         let service = spawn { unwrap(Err(\"the builder died\")) }\n\
         task.watch(service, inbox)\n\
         println(show(chan.recv_timeout(inbox, 2000)))\n\
         // A task that already ended closes at once.\n\
         let done = spawn { 1 }\n\
         let x = task.join(done)\n\
         let late = chan.new()\n\
         task.watch(done, late)\n\
         println(show(chan.recv_timeout(late, 2000)))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "w.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("Err("), "{out}");
    assert!(
        !lines[0].contains("timed out"),
        "the recv must see a closed channel: {out}"
    );
    assert!(
        lines[1].starts_with("Err(") && !lines[1].contains("timed out"),
        "{out}"
    );
}

#[test]
fn the_image_loads_faster_than_the_source_parses() {
    let wasm = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/wasm32-unknown-unknown/release/olang_playground.wasm");
    let node = Command::new("node").arg("--version").output();
    if !wasm.exists() || node.is_err() {
        eprintln!("skipping: needs node and the wasm artifact (make wasm)");
        return;
    }
    let ws = workspace("boot");
    let source = "fn f(x) = x * 2\n".repeat(40)
        + "let xs = map([1, 2, 3], (x) => f(x))\nprintln(to_string(xs))\n";
    write(&ws, "bundle.ol", &source);
    write(
        &ws,
        "encode.ol",
        "let src = unwrap(fs.read_file(\"bundle.ol\"))\nunwrap(fs.write_bytes(\"bundle.olb\", unwrap(meta.encode(src))))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "encode.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    let harness = Path::new(env!("CARGO_MANIFEST_DIR")).join("playground/dom_harness.mjs");
    let out = Command::new("node")
        .arg(&harness)
        .arg(&wasm)
        .arg("--boot")
        .arg(ws.join("bundle.ol"))
        .arg(ws.join("bundle.olb"))
        .output()
        .expect("node");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line = text.lines().last().unwrap_or("");
    let json: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|_| panic!("{text}"));
    assert_eq!(json["image_retry_with_source"], false, "{text}");
    let source_ms = json["source_load_ms"].as_f64().unwrap();
    let image_ms = json["image_load_ms"].as_f64().unwrap();
    assert!(
        image_ms < source_ms,
        "image {image_ms} ms vs source {source_ms} ms"
    );
}

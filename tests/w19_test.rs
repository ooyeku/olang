//! Roadmap W19 — the embedded runtime: `runtime.wasm()`, the image
//! stamp, and the cross-file check that was already there.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w19_{}_{}", std::process::id(), tag));
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
fn runtime_wasm_answers_the_embedded_runtime_or_says_how_to_build_one() {
    let ws = workspace("runtime");
    write(
        &ws,
        "r.ol",
        "println(runtime.version())\n\
         match runtime.wasm() {\n\
             Ok(r) => {\n\
                 println(\"ok \" + to_string(str.length(map_get(r, \"hash\"))) + \" \" + map_get(r, \"version\"))\n\
                 println(show(bytes.len(map_get(r, \"bytes\")) > 1000000))\n\
                 println(show(bytes.len(map_get(r, \"br\")) < bytes.len(map_get(r, \"gzip\"))))\n\
                 println(show(bytes.len(map_get(r, \"gzip\")) < bytes.len(map_get(r, \"bytes\"))))\n\
                 // The first bytes are the wasm magic.\n\
                 println(show(bytes.slice(map_get(r, \"bytes\"), 1, 4) == bytes.from_string(\"asm\")))\n\
             },\n\
             Err(e) => println(\"err \" + e)\n\
         }\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "r.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    let mut lines = out.lines();
    assert_eq!(lines.next().unwrap(), env!("CARGO_PKG_VERSION"));
    let second = lines.next().unwrap();
    if second.starts_with("ok ") {
        assert_eq!(second, format!("ok 16 {}", env!("CARGO_PKG_VERSION")));
        assert_eq!(
            lines.collect::<Vec<_>>(),
            vec!["true", "true", "true", "true"]
        );
    } else {
        // A binary built before `cargo xtask wasm`: the answer names the build.
        assert!(second.contains("cargo xtask wasm"), "{second}");
    }
}

#[test]
fn olang_check_reads_aliases_and_enums_across_files() {
    let ws = workspace("xfile");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"xf\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/types.ol",
        "share type Task = { title: String, done: Bool }\n\
         share type Shape = enum { Circle(Float), Rect(Float, Float) }\n\
         share fn mk() = #{ \"title\": \"x\", \"done\": false }\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.types { Task, Shape, mk }\n\
         fn title_of(t: Task) = map_get(t, \"titel\")\n\
         fn area(s: Shape) = match s { Circle(r) => 1.0 }\n\
         println(title_of(mk()))\n",
    );
    let (out, err, _) = olang(&ws, &["check", "."]);
    let all = out + &err;
    assert!(
        all.contains("`titel` is not a key of the declared shape"),
        "{all}"
    );
    assert!(
        all.contains("match over `Shape` is not exhaustive: Rect has no arm"),
        "{all}"
    );
}

/// The boot budget as a gate: the web SDK demo's client, bundled and
/// encoded as the server does it (hot hints included), boots from its
/// image in the dom harness under a ceiling. Measured at 8 ms (1 ms
/// decode, 7 ms run, the first render included) on an M-series laptop
/// in 2026-09; the ceiling leaves room for a slower CI host and fails
/// on a regression of the kind W16 found (217 ms with an empty list),
/// not on noise. Skips where node or the wasm artifact is absent.
#[test]
fn the_sdk_demo_boots_from_its_image_under_the_budget() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let wasm = root.join("target/wasm32-unknown-unknown/release/olang_playground.wasm");
    let node = Command::new("node").arg("--version").output();
    if !wasm.exists() || node.is_err() {
        eprintln!("skipping: needs node and the wasm artifact (cargo xtask wasm)");
        return;
    }
    let ws = workspace("bootbudget");
    write(
        &ws,
        "bundle.ol",
        "use lib.server { bundle_client, client_fn_names }\n\
         let out = os.args()[1]\n\
         let src = unwrap(fs.read_file(\"demo/client.ol\"))\n\
         let bundle = bundle_client(src)\n\
         unwrap(fs.write_file(out + \"/bundle.ol\", bundle))\n\
         unwrap(fs.write_bytes(out + \"/bundle.olb\", unwrap(meta.encode(bundle, #{ \"hot\": client_fn_names([src]) }))))\n",
    );
    let sdk = root.join("frameworks/web-sdk");
    let script = ws.join("bundle.ol").to_string_lossy().to_string();
    let out_dir = ws.to_string_lossy().to_string();
    let (out, err, rc) = olang(&sdk, &["run", &script, &out_dir]);
    assert_eq!(rc, 0, "{out}{err}");
    let harness = root.join("playground/dom_harness.mjs");
    let mut best: Option<(f64, f64)> = None;
    for _ in 0..3 {
        let run = Command::new("node")
            .arg(&harness)
            .arg(&wasm)
            .arg("--boot")
            .arg(ws.join("bundle.ol"))
            .arg(ws.join("bundle.olb"))
            .output()
            .expect("node");
        let text = String::from_utf8_lossy(&run.stdout);
        assert!(
            run.status.success(),
            "{text}{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let line = text.lines().last().unwrap_or("");
        let json: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|_| panic!("{text}"));
        assert_eq!(json["error"], serde_json::Value::Null, "{text}");
        assert_eq!(json["image_retry_with_source"], false, "{text}");
        let load = json["image_load_ms"].as_f64().unwrap();
        let run_ms = json["image_run_ms"].as_f64().unwrap();
        best = Some(match best {
            Some((l, r)) if l + r <= load + run_ms => (l, r),
            _ => (load, run_ms),
        });
    }
    let (load, run_ms) = best.unwrap();
    assert!(
        load + run_ms < 60.0,
        "the SDK demo's image boot took {load} + {run_ms} ms; the budget is 60 ms"
    );
}

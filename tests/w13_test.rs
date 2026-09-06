//! Roadmap W13 — the third reading of open-track: a macro imported from a
//! library's front door, an imported module's tests running once per
//! invocation, `trust_proxy` and `req.remote_addr`, and a graceful drain
//! through `os.on_shutdown` + `http.shutdown`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w13_{}_{}", std::process::id(), tag));
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
fn a_macro_imports_from_the_librarys_front_door() {
    let ws = workspace("frontdoor");
    let lib = ws
        .parent()
        .unwrap()
        .join(format!("frontpkg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&lib);
    write(
        &lib,
        "lib/html.ol",
        "share fn text(s) = \"<\" + s + \">\"\nmeta fn when(cond, node) = `if ${cond} => ${node} else => text(\"\")`\n",
    );
    write(&lib, "index.ol", "share use lib.html { text, when }\n");
    write(
        &lib,
        "olang.toml",
        "[package]\nname = \"front\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "olang.toml",
        &format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nfront = {{ path = \"{}\" }}\n",
            lib.display()
        ),
    );
    write(
        &ws,
        "app.ol",
        "use front { text, when }\nlet t = ()\nprintln(@when(t != (), text(\"x\")))\nprintln(@when(t == (), text(\"y\")))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "app.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    // The fixture's text("") renders "<>": the false branch built nothing.
    assert_eq!(out, "<>\n<y>\n");
    let _ = std::fs::remove_dir_all(&lib);
}

#[test]
fn an_imported_modules_tests_run_once_per_invocation() {
    let ws = workspace("once");
    write(
        &ws,
        "lib/shared.ol",
        "share fn twice(x) = x * 2\ntest \"shared twice\" { assert_eq(twice(2), 4) }\n",
    );
    for i in 1..=3 {
        write(
            &ws,
            &format!("t{i}.ol"),
            &format!(
                "use lib.shared {{ twice }}\ntest \"own {i}\" {{ assert_eq(twice({i}), {}) }}\n",
                i * 2
            ),
        );
    }
    let (out, err, rc) = olang(&ws, &["test", "."]);
    assert_eq!(rc, 0, "{out}{err}");
    // Three own tests plus the shared module's one, not three of it.
    assert!(out.contains("4 passed, 0 failed"), "{out}");
    assert_eq!(out.matches("shared twice").count(), 1, "{out}");
}

#[test]
fn remote_addr_honours_the_forwarded_header_only_when_asked() {
    let ws = workspace("proxy");
    write(
        &ws,
        "s.ol",
        "let port = 42000 + time.monotonic_ms() % 3000\n\
         let plain = spawn { http.serve(port, (req) => req.remote_addr) }\n\
         let trusting = spawn { http.serve(port + 1, (req) => req.remote_addr, #{ \"trust_proxy\": true }) }\n\
         time.sleep(300)\n\
         let opts = #{ \"headers\": #{ \"X-Forwarded-For\": \"203.0.113.9, 10.0.0.1\" } }\n\
         let a = unwrap(http.get(\"http://127.0.0.1:\" + to_string(port) + \"/\", opts))\n\
         let b = unwrap(http.get(\"http://127.0.0.1:\" + to_string(port + 1) + \"/\", opts))\n\
         println(str.starts_with(a.body, \"127.0.0.1:\"))\n\
         println(b.body)\n\
         http.shutdown()\n\
         task.join(plain)\n\
         task.join(trusting)\n\
         println(\"drained\")\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "s.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    // The servers announce themselves first; the answers are the tail.
    let tail: Vec<&str> = out.lines().rev().take(3).collect();
    assert_eq!(tail, vec!["drained", "203.0.113.9", "true"], "{out}");
}

#[test]
fn a_signal_drains_the_server_through_the_shutdown_handler() {
    let ws = workspace("signal");
    write(
        &ws,
        "d.ol",
        "unwrap(os.on_shutdown((why) => { println(\"handler: \" + why) http.shutdown() }))\n\
         println(\"up\")\n\
         let r = http.serve(0, (req) => \"ok\")\n\
         println(\"served: \" + show(r))\n",
    );
    let child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["run", "d.ol"])
        .current_dir(&ws)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(700));
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("handler: shutdown"), "{text}");
    // http.serve returns once the drain completes.
    assert!(text.contains("served: Ok(())"), "{text}");
}

#[test]
fn a_captured_binding_named_like_a_builtin_shadows_it_on_every_tier() {
    let ws = workspace("shadowbuiltin");
    write(
        &ws,
        "h.ol",
        "let head = (x) => \"mine\"\n\
         fn f(r) = head(r)\n\
         fn g() = {\n\
             let len = (x) => \"inner\"\n\
             fn h(r) = len(r)\n\
             h(1)\n\
         }\n\
         println(f(1))\n\
         println(g())\n\
         println(to_string(len([1, 2])))\n",
    );
    for flags in [vec!["run", "h.ol"], vec!["--no-ovm", "run", "h.ol"]] {
        let (out, err, rc) = olang(&ws, &flags);
        assert_eq!(rc, 0, "{flags:?}: {out}{err}");
        assert_eq!(out, "mine\ninner\n2\n", "{flags:?}");
    }
}

#[test]
fn a_lib_module_call_inside_a_task_costs_what_it_costs_inline() {
    let ws = workspace("taskcost");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"taskcost\"\nversion = \"0.1.0\"\n",
    );
    write(
        &ws,
        "lib/app.ol",
        "share fn as_str(v) = if typeof(v) == \"String\" => v else => to_string(v)\n",
    );
    write(
        &ws,
        "lib/checklist.ol",
        "use lib.app { as_str }\n\
         share fn count(issue) = {\n\
             let desc = as_str(map_get(issue, \"description\"))\n\
             len(filter(str.lines(desc), (l) => str.starts_with(str.trim(l), \"- [\")))\n\
         }\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.checklist\n\
         let issue = unwrap(json.parse(\"{\\\"description\\\": \\\"- [ ] a\\\\n- [x] b\\\\nplain\\\"}\"))\n\
         fn in_task(f) = {\n\
             let c = chan.new()\n\
             let t = spawn { chan.send(c, f()) }\n\
             chan.recv(c)\n\
         }\n\
         fn bench() = {\n\
             let t0 = time.monotonic_ms()\n\
             let mut acc = 0\n\
             for i in range(0, 20000) { acc = acc + checklist.count(issue) }\n\
             time.monotonic_ms() - t0\n\
         }\n\
         let inline = bench()\n\
         let in_a_task = unwrap(in_task(() => bench()))\n\
         println(to_string(inline) + \" \" + to_string(in_a_task))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "main.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    let nums: Vec<f64> = out.trim().split(' ').map(|n| n.parse().unwrap()).collect();
    // The report was 50× (a bridge Interpreter rebuilt per nested builtin
    // call). What remains is the function value running interpreted on
    // the bridge rather than compiled — the tree-walk's own ~15× on this
    // loop — so the bound sits above that and below the rebuild's cost.
    assert!(
        nums[1] <= nums[0].max(5.0) * 25.0,
        "inline {} ms vs in a task {} ms",
        nums[0],
        nums[1]
    );
}

//! Roadmap W17 — the sixth reading of open-track and Shuttle.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w17_{}_{}", std::process::id(), tag));
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

const MODES: [&[&str]; 3] = [&["run"], &["--no-ovm", "run"], &["--in-task", "run"]];

fn in_every_mode(dir: &Path, file: &str, expect: &str) {
    for mode in MODES {
        let mut args: Vec<&str> = mode.to_vec();
        args.push(file);
        let (out, err, rc) = olang(dir, &args);
        assert_eq!(rc, 0, "{mode:?}: {out}{err}");
        assert_eq!(out, expect, "{mode:?}");
    }
}

#[test]
fn a_function_declared_below_its_caller_resolves_at_the_entry_files_top_level() {
    let ws = workspace("below");
    write(
        &ws,
        "b.ol",
        "share fn outer(c) = Ok(later(c))\n\
         println(show(outer(1)))\n\
         fn later(c) = \"found \" + to_string(c)\n\
         let conn = 7\n\
         fn uses_conn() = conn + 1\n\
         println(to_string(uses_conn()))\n",
    );
    in_every_mode(&ws, "b.ol", "Ok(\"found 1\")\n8\n");
}

#[test]
fn a_packages_private_function_is_not_shadowed_by_the_apps_on_a_task() {
    let ws = workspace("shadow");
    let pkg = ws
        .parent()
        .unwrap()
        .join(format!("shadow_pkg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pkg);
    write(
        &pkg,
        "olang.toml",
        "[package]\nname = \"shuttle\"\nversion = \"0.1.0\"\n",
    );
    write(
        &pkg,
        "lib/snapshot.ol",
        "share fn start_snapshot() = {\n\
             let inbox = chan.new()\n\
             let t = spawn { serve_snapshot(inbox) }\n\
             inbox\n\
         }\n\
         share fn ask(inbox, req) = {\n\
             let reply = chan.new()\n\
             chan.send(inbox, #{ \"req\": req, \"reply\": reply })\n\
             chan.recv(reply)\n\
         }\n\
         fn serve_snapshot(inbox) = {\n\
             let mut going = true\n\
             while going {\n\
                 match chan.recv(inbox) {\n\
                     Ok(msg) => {\n\
                         let out = attempt(() => compute_here([], #{}, map_get(msg, \"req\")))\n\
                         chan.send(map_get(msg, \"reply\"), out)\n\
                         if map_get(msg, \"req\") == \"stop\" => { going = false } else => ()\n\
                     },\n\
                     Err(e) => { going = false }\n\
                 }\n\
             }\n\
         }\n\
         fn compute_here(builders, slots, req) = \"computed \" + to_string(req)\n",
    );
    write(
        &pkg,
        "index.ol",
        "share use lib.snapshot { start_snapshot, ask }\n",
    );
    write(
        &ws,
        "olang.toml",
        &format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nshuttle = {{ path = \"{}\" }}\n",
            pkg.display()
        ),
    );
    write(
        &ws,
        "lib/cache.ol",
        "share fn compute_here(service, op, arg, version) = \"app \" + op\n",
    );
    write(
        &ws,
        "main.ol",
        "use shuttle { start_snapshot, ask }\n\
         use lib.cache { compute_here }\n\
         let inbox = start_snapshot()\n\
         println(show(ask(inbox, \"one\")))\n\
         println(compute_here(\"s\", \"op\", 1, 2))\n\
         println(show(ask(inbox, \"stop\")))\n",
    );
    in_every_mode(
        &ws,
        "main.ol",
        "Ok(Ok(\"computed one\"))\napp op\nOk(Ok(\"computed stop\"))\n",
    );
    let _ = std::fs::remove_dir_all(&pkg);
}

#[test]
fn a_parsed_json_object_equals_a_map_with_the_same_contents() {
    let ws = workspace("jsoneq");
    write(
        &ws,
        "e.ol",
        "let j = unwrap(json.parse(\"{\\\"new\\\": 0, \\\"open\\\": 1}\"))\n\
         fn same(a, b) = a == b\n\
         let mut warm = 0\n\
         for i in range(0, 40) { if same(j, j) => { warm = warm + 1 } else => () }\n\
         println(show(same(j, #{ \"new\": 0, \"open\": 1 })))\n\
         println(show(same(#{ \"new\": 0, \"open\": 1 }, j)))\n\
         println(show(same(j, #{ \"new\": 0, \"open\": 2 })))\n\
         println(show(same(map_get(unwrap(json.parse(\"{\\\"v\\\": {\\\"a\\\": [1, 2.0]}}\")), \"v\"), #{ \"a\": [1, 2] })))\n\
         println(show(same(map_get(unwrap(json.parse(\"{\\\"v\\\": {\\\"a\\\": [1, 2]}}\")), \"v\"), #{ \"a\": [1, 2] })))\n\
         println(show(same({ x: 1 }, #{ \"x\": 1 })))\n\
         println(show(typeof(j)))\n",
    );
    // Nested, an Int and a Float stay distinct ([1, 2.0] is not [1, 2]),
    // as they always were; the map kinds compare by contents.
    in_every_mode(&ws, "e.ol", "true\ntrue\nfalse\nfalse\ntrue\ntrue\nJsonObject\n");
}

#[test]
fn chan_ask_is_the_reply_pattern_in_one_call() {
    let ws = workspace("ask");
    write(
        &ws,
        "a.ol",
        "fn service(inbox) = {\n\
             let mut going = true\n\
             while going {\n\
                 match chan.recv(inbox) {\n\
                     Ok(msg) => {\n\
                         let req = map_get(msg, \"req\")\n\
                         if req == \"stop\" => { going = false } else => ()\n\
                         chan.send(map_get(msg, \"reply\"), \"answer:\" + to_string(req))\n\
                     },\n\
                     Err(e) => { going = false }\n\
                 }\n\
             }\n\
         }\n\
         let inbox = chan.new()\n\
         let t = spawn { service(inbox) }\n\
         println(show(chan.ask(inbox, 7)))\n\
         let t0 = time.monotonic_ms()\n\
         for i in range(0, 2000) { chan.ask(inbox, i) }\n\
         let per_us = (time.monotonic_ms() - t0) * 1000 / 2000\n\
         println(show(chan.ask(inbox, \"stop\")))\n\
         println(show(per_us < 250))\n",
    );
    in_every_mode(&ws, "a.ol", "Ok(\"answer:7\")\nOk(\"answer:stop\")\ntrue\n");
}

#[test]
fn a_deferred_handler_parks_the_connection_and_respond_answers_it_from_a_task() {
    use std::io::{Read, Write};
    let ws = workspace("defer");
    let port = 45000 + (std::process::id() % 2000) as u16;
    write(
        &ws,
        "d.ol",
        &format!(
            "println(\"up\")\n\
             let r = http.serve({port}, (req) => {{\n\
                 if req.path == \"/wait\" => {{\n\
                     let ticket = http.defer()\n\
                     spawn {{ time.sleep(400); http.respond(ticket, http.response(200, \"late\")) }}\n\
                     ticket\n\
                 }} else if req.path == \"/stop\" => {{ http.shutdown(); \"bye\" }}\n\
                 else => \"quick\"\n\
             }}, #{{ \"workers\": 1 }})\n\
             println(\"served: \" + show(r))\n"
        ),
    );
    let child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["run", "d.ol"])
        .current_dir(&ws)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Wait for the listener rather than a fixed pause: the test binary
    // runs its cases in parallel, and a debug-profile boot under that
    // load can take longer than a sleep guesses.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "server did not start listening"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let get = move |path: &str| -> (String, std::time::Duration) {
        let started = std::time::Instant::now();
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        s.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
        let mut body = Vec::new();
        let _ = s.read_to_end(&mut body);
        (
            String::from_utf8_lossy(&body).to_string(),
            started.elapsed(),
        )
    };
    // The deferred request is in flight on the one worker...
    let waiter = std::thread::spawn(move || get("/wait"));
    std::thread::sleep(std::time::Duration::from_millis(100));
    // ...and the same worker answers another request meanwhile: the
    // parked connection cost a socket, not the worker.
    let (quick, quick_took) = get("/quick");
    assert!(quick.contains("quick"), "{quick}");
    assert!(
        quick_took < std::time::Duration::from_millis(250),
        "{quick_took:?}"
    );
    let (late, late_took) = waiter.join().unwrap();
    assert!(late.contains("HTTP/1.1 200"), "{late}");
    assert!(late.ends_with("late"), "{late}");
    assert!(
        late_took >= std::time::Duration::from_millis(350),
        "{late_took:?}"
    );
    let _ = get("/stop");
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("served: Ok(())"), "{text}");
}

#[test]
fn the_checker_reports_a_missing_enum_constructor_in_a_match() {
    let ws = workspace("adt");
    write(
        &ws,
        "s.ol",
        "type Shape = enum { Circle(Float), Rect(Float, Float), Unknown }\n\
         fn area(s: Shape) = match s { Circle(r) => 3.0 * r * r, Unknown => 0.0 }\n\
         fn area2(s: Shape) = match s { Circle(r) => 1.0, Rect(w, h) => w * h, Unknown => 0.0 }\n\
         fn area3() = match Rect(1.0, 2.0) { Circle(r) => 1.0, other => 0.0 }\n\
         fn area4(s: Shape) = match s { Circle(r) if r > 1.0 => 1.0, Rect(w, h) => 2.0, Unknown => 0.0 }\n\
         println(to_string(area(Unknown)))\n",
    );
    let (out, err, _) = olang(&ws, &["check", "s.ol"]);
    let all = out + &err;
    assert!(
        all.contains("match over `Shape` is not exhaustive: Rect has no arm"),
        "{all}"
    );
    // The guarded Circle arm proves nothing: Circle is reported for area4.
    assert!(all.contains("Circle has no arm"), "{all}");
    assert_eq!(all.matches("is not exhaustive").count(), 2, "{all}");
}

#[test]
fn the_checker_reads_a_declared_record_shape_at_literal_key_sites() {
    let ws = workspace("shape");
    write(
        &ws,
        "r.ol",
        "fn summary_of(r: { summary: String, count: Int }) = map_get(r, \"summry\")\n\
         fn count_of(r: { summary: String, count: Int }) = r.count + r[\"cont\"]\n\
         let rec: { summary: String, count: Int } = #{ \"summary\": \"s\", \"count\": 1 }\n\
         println(to_string(map_get(rec, \"count\")))\n\
         meta fn shaped(decl) = str.replace(decl, \"(r)\", \"(r: { title: String })\")\n\
         @shaped\n\
         fn read_title(r) = map_get(r, \"titel\")\n",
    );
    let (out, err, _) = olang(&ws, &["check", "r.ol"]);
    let all = out + &err;
    assert!(
        all.contains("`summry` is not a key of the declared shape { summary: String"),
        "{all}"
    );
    assert!(all.contains("did you mean `summary`?"), "{all}");
    assert!(all.contains("`cont` is not a key"), "{all}");
    // The shape a macro's output declared is checked like any other.
    assert!(
        all.contains("`titel` is not a key of the declared shape { title: String }"),
        "{all}"
    );
    assert_eq!(all.matches("is not a key").count(), 3, "{all}");
    let (out, err, rc) = olang(&ws, &["run", "r.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "1\n");
}

#[test]
fn meta_exports_reads_an_imported_modules_literal_bindings_at_expansion_time() {
    let ws = workspace("exports");
    write(
        &ws,
        "lib/decl.ol",
        "share let RESOURCES = [\"issues\", \"people\"]\n\
         let PRIVATE_LIMIT = 40\n\
         share let SHAPE = #{ \"issues\": [\"id\", \"title\"] }\n\
         share let COMPUTED = len(RESOURCES)\n",
    );
    write(&ws, "pkg/index.ol", "share use lib.rules { RULES }\n");
    write(
        &ws,
        "pkg/lib/rules.ol",
        "share let RULES = [\"a\", \"b\", \"c\"]\n",
    );
    write(
        &ws,
        "main.ol",
        "use lib.decl { RESOURCES }\n\
         use pkg\n\
         meta fn count() = {\n\
             let ex = unwrap(meta.exports(\"lib.decl\"))\n\
             meta.lit(len(map_get(ex, \"RESOURCES\")) + map_get(ex, \"PRIVATE_LIMIT\"))\n\
         }\n\
         meta fn keys_of(res) = meta.lit(map_get(map_get(unwrap(meta.exports(\"lib.decl\")), \"SHAPE\"), unwrap(meta.eval(res))))\n\
         meta fn computed_seen() = meta.lit(map_has_key(unwrap(meta.exports(\"lib.decl\")), \"COMPUTED\"))\n\
         meta fn rules() = meta.lit(len(map_get(unwrap(meta.exports(\"pkg\")), \"RULES\")))\n\
         meta fn missing() = meta.lit(show(meta.exports(\"lib.other\")))\n\
         println(to_string(@count()))\n\
         println(show(@keys_of(\"issues\")))\n\
         println(show(@computed_seen()))\n\
         println(to_string(@rules()))\n\
         println(@missing())\n\
         println(show(meta.exports(\"lib.decl\")))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "main.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(
        out,
        "42\n[\"id\", \"title\"]\nfalse\n3\nErr(\"meta.exports: 'lib.other' is not a module this file imports (imported: lib.decl, pkg)\")\n\
         Err(\"meta.exports is answered at expansion time, inside a meta fn: at runtime a module's bindings are reached by importing them\")\n"
    );
}

#[test]
fn a_fresh_name_cannot_be_spelled_by_a_program() {
    let ws = workspace("fresh");
    write(
        &ws,
        "ok.ol",
        "meta fn twice(e) = {\n    let n = meta.fresh(\"t\")\n    `{ let ${n} = ${e}; ${n} + ${n} }`\n}\n\
         let t__m0_ish = 1 // a different shape: not reserved\n\
         println(to_string(@twice(5) + t__m0_ish))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "ok.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "11\n");
    let expanded = olang(&ws, &["expand", "ok.ol"]).0;
    assert!(expanded.contains("t__m0"), "{expanded}");
    // The rule applies to a program that expands macros — the only
    // program a generated temporary can collide with.
    write(
        &ws,
        "bad.ol",
        "meta fn twice(e) = {\n    let n = meta.fresh(\"t\")\n    `{ let ${n} = ${e}; ${n} + ${n} }`\n}\n\
         let total = 3\n// a comment mentioning x__m1 is fine\nlet s = \"and so is x__m2 in a string\"\nlet t__m0 = 3\nprintln(to_string(@twice(t__m0)))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "bad.ol"]);
    assert_ne!(rc, 0, "{out}");
    assert!(
        err.contains("line 8: `t__m0` is a name reserved for macro-generated temporaries"),
        "{err}"
    );
}

#[test]
fn a_recording_covers_db_reads_channels_and_joins_and_replays_without_workers() {
    let ws = workspace("timeline");
    write(
        &ws,
        "t.ol",
        "let conn = unwrap(db.open(\":memory:\"))\n\
         unwrap(db.execute(conn, \"CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)\"))\n\
         unwrap(db.execute(conn, \"INSERT INTO t (v) VALUES (?)\", [random.randint(1, 1000000000)]))\n\
         println(\"rows: \" + show(unwrap(db.query(conn, \"SELECT v FROM t\"))))\n\
         let inbox = chan.new()\n\
         let t = spawn {\n\
             let d = random.randint(1, 1000000000)\n\
             println(\"worker ran\")\n\
             chan.send(inbox, d)\n\
             d * 2\n\
         }\n\
         let got = unwrap(chan.recv(inbox))\n\
         let joined = task.join(t)\n\
         println(\"consistent \" + show(joined == got * 2))\n\
         let svc = chan.new()\n\
         let s = spawn {\n\
             match chan.recv(svc) { Ok(m) => chan.send(map_get(m, \"reply\"), map_get(m, \"req\") + 1), Err(e) => () }\n\
         }\n\
         println(show(chan.ask(svc, 41)))\n",
    );
    let (rec_out, rec_err, rc) = olang(&ws, &["--record", "run.olt", "t.ol"]);
    assert_eq!(rc, 0, "{rec_out}{rec_err}");
    assert!(rec_out.contains("worker ran"), "{rec_out}");
    assert!(rec_out.contains("consistent true"), "{rec_out}");
    assert!(rec_out.contains("Ok(42)"), "{rec_out}");
    assert!(rec_err.contains("recorded 8 event(s)"), "{rec_err}");
    assert!(
        rec_err.contains("note: this recorded run started a task"),
        "{rec_err}"
    );
    let (rep_out, rep_err, rc) = olang(&ws, &["replay", "run.olt"]);
    assert_eq!(rc, 0, "{rep_out}{rep_err}");
    // Byte-for-byte the recorded run's main-thread output, minus the
    // worker's own line: under replay no worker starts.
    assert_eq!(rep_out, rec_out.replace("worker ran\n", ""), "{rep_err}");
    assert!(rep_err.contains("clean"), "{rep_err}");
    assert!(!rep_err.contains("note: this recorded run"), "{rep_err}");
}

#[test]
fn an_image_carries_hot_hints_and_a_top_level_range_map_is_native() {
    let ws = workspace("image");
    write(
        &ws,
        "e.ol",
        "let src = \"fn view(s) = s\\nfn update(s, a) = s\\nprintln(\\\"x\\\")\"\n\
         let plain = unwrap(meta.encode(src))\n\
         let hinted = unwrap(meta.encode(src, #{ \"hot\": [\"view\", \"update\"] }))\n\
         println(show(len(hinted) > len(plain)))\n\
         println(show(attempt(() => meta.encode(src, #{ \"hot\": 3 }))))\n\
         println(show(attempt(() => meta.encode(src, #{ \"warm\": [] }))))\n\
         let big = map(0..3000000, (i) => 0)\n\
         let odd = filter(0..3000000, (i) => i % 2 == 1)\n\
         println(to_string(len(big)) + \" \" + to_string(len(odd)) + \" \" + to_string(odd[1]))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "e.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(
        out,
        "true\nErr(\"meta.encode: hot must be a list of function names, got Int\")\nErr(\"meta.encode: unknown option 'warm' — the option is hot\")\n3000000 1500000 3\n"
    );
}

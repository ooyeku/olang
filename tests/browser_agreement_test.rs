//! The browser runtime does what native does: a program run in the
//! wasm runtime (playground/run_harness.mjs, the playground's own
//! `olang_run`) prints what the native binary prints, for every ruling of
//! the 1.0 freeze pass the browser has a module for — the printed form,
//! `.` on `()`, Bool conditions, negative indexes, records, reserved
//! words, assignment, equality in one walk, dates, math, absence — and a
//! client bundle keeps each module's names its own, as native modules do.
//! Skips where node or the wasm artifact (`cargo xtask wasm`) is absent.

use std::path::{Path, PathBuf};
use std::process::Command;

fn wasm() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let wasm = root.join("target/wasm32-unknown-unknown/release/olang_playground.wasm");
    let node = Command::new("node").arg("--version").output();
    if !wasm.exists() || node.is_err() {
        eprintln!("skipping: needs node and the wasm artifact (cargo xtask wasm)");
        return None;
    }
    Some(wasm)
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_browser_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    dir
}

fn in_browser(wasm: &Path, program: &Path) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("node")
        .arg(root.join("playground/run_harness.mjs"))
        .arg(wasm)
        .arg(program)
        .output()
        .expect("node");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

const PROGRAM: &str = r#"// printed form
type P = struct { x: Int, y: Int }
type Shape = enum { Circle(Float), Sq(Float) }
println([1, 2.5, "a\"b", #{ "k": [1, ()] }, { a: 1 }, P { x: 1, y: 2 }, Circle(1.0), Ok("x"), Err(1), (1, "t")])
println(show("plain") + " " + to_string([true]) + " " + `${#{ "n": "v" }}`)
// `.` on (), field of ()
let s = #{ "pp": () }
println([s.pp.cur, ().k ?? 5, map_get((), "k"), { a: 1 }.b])
println(attempt(() => 5.k))
// Bool conditions
println(attempt(() => if 0 => 1 else => 2))
println(attempt(() => { let mut n = 0; while "" { n = n + 1 }; n }))
// negative indexes
println([attempt(() => [1, 2][-1]), attempt(() => "ab"[-1]), first([]), last([3, 4]), str.char_at("ab", 5)])
// task.join and join_timeout
fn work() = 21 * 2
fn boom() = raise("bad")
// records as Maps / objects
println(typeof(json.parse("{\"a\":1}") |> unwrap))
println(csv.parse_with_headers("a,b\n1,2") |> unwrap)
// reserved words
println([meta.parse("let await = 1") |> is_err, meta.parse("let select = 1") |> is_ok, meta.parse("a == b == c") |> is_err])
// assignment answers (), block values
let mut z = 0
println([{ z = 3 }, { let q = 1 }, { 1; }, z])
fn empty() = {}
println(empty())
// == one walk, equality as one relation
fn g() = 1
println([[1, g] == [2, g], attempt(() => [g] == [g]), Ok(1) == Ok(1), 1 == 1.0, [1] == [1.0], 9007199254740993 == 9007199254740992.0, contains([1, 2], 1.0)])
// map keys
println([#{ 1: "a", 1.0: "b" }, map_get(#{ "1": "x" }, 1.0)])
// dates
println([dates.from_timestamp(86400000), dates.from_timestamp_s(86400), dates.format_date(unwrap(dates.parse("2026-01-02")), "%Y/%m/%d"), dates.format_date(unwrap(dates.parse("2026-01-02")), "%Q")])
println(dates.parse_datetime("2026-01-02T03:04:05+02:00"))
// math
println([math.floor(2.7), math.round(-2.5), math.pow(2, 10), math.min(3, 1.5), math.max([1, 9, 3]), attempt(() => math.pow(2, -1))])
// naming decisions
println([find([1, 2, 3], (x) => x > 1), find([1], (x) => x > 5), reduce([1, 2, 3], (a, b) => a + b), reduce([], (a, b) => a)])
// absence
println([chan.try_recv(chan.new()), str.char_at("ab", 9)])
// bitwise precedence, line continuation
println([1 + 2 << 1, 6 & 3 == 2, 1 | 2 ^ 3])
let w = 10
    - 3
println(w)
// numbers
println([attempt(() => to_int(1e30)), attempt(() => 1 << 63), -7 % 3, -9223372036854775808])
// top-level names at call time
let mut count = 1
fn read_count() = count
count = 2
println(read_count())
// patterns
println(match 1.0 { 1 => "int", _ => "other" })
println(attempt(() => match 5 { 1 => "one" }))
fn record(v) = match v { {} => "record", _ => "other" }
println(map([[1], "s", (), 3, 2.5, #{}, { a: 1 }, P { x: 1, y: 2 }, json.parse("{}") |> unwrap], record))
// sort stability, strings
println([col.sort_by([[1, "b"], [0, "x"], [1, "a"]], (p) => p[0]), split("ab", ""), "\x41", str.substring("abc", 1, 99)])
"#;

#[test]
fn the_browser_runtime_agrees_with_native_on_the_freeze_pass() {
    let Some(wasm) = wasm() else { return };
    let ws = workspace("agree");
    let program = ws.join("agree.ol");
    std::fs::write(&program, PROGRAM).unwrap();
    let native = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg(&program)
        .output()
        .expect("olang");
    let native = String::from_utf8_lossy(&native.stdout).to_string();
    assert!(!native.contains("ERROR"), "{native}");
    assert_eq!(in_browser(&wasm, &program), native);
}

const FIELDS: &str = r#"share fn code(min, max) = #{ "kind": "code", "min": min, "max": max }
fn helper() = "fields"
share fn which() = helper()
"#;

const CLIENT: &str = r#"use web { code, div, render }
use lib.fields { code as field_code, which }
fn helper() = "client"
println(render(code(#{}, ["x"])))
println(field_code(1, 2))
println([which(), helper()])
"#;

#[test]
fn a_client_bundle_keeps_each_modules_names_its_own_in_the_browser() {
    // The web SDK shares a `code` element and the app a `code` field
    // (Shuttle's); each module has a private `helper`. One namespace made
    // them one function each, and the duplicate-fn rule refused the
    // bundle; each module's scope is its own again.
    let Some(wasm) = wasm() else { return };
    let ws = workspace("bundle");
    std::fs::write(ws.join("lib/fields.ol"), FIELDS).unwrap();
    std::fs::write(ws.join("client.ol"), CLIENT).unwrap();
    std::fs::write(
        ws.join("b.ol"),
        "use lib.server { bundle_clients_in }\n\
         let dir = os.args()[1]\n\
         let files = [dir + \"/lib/fields.ol\", dir + \"/client.ol\"]\n\
         unwrap(fs.write_file(dir + \"/bundle.ol\", bundle_clients_in(\".\", map(files, (f) => unwrap(fs.read_file(f))), files)))\n",
    )
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let made = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args([
            "run",
            &ws.join("b.ol").to_string_lossy(),
            &ws.to_string_lossy(),
        ])
        .current_dir(root.join("frameworks/web-sdk"))
        .output()
        .expect("olang");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    assert_eq!(
        in_browser(&wasm, &ws.join("bundle.ol")),
        "<code>x</code>\n#{\"kind\": \"code\", \"max\": 2, \"min\": 1}\n[\"fields\", \"client\"]\n"
    );
}

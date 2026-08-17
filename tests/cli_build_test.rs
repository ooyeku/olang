//! `olang build` — the self-contained-executable path. These are true
//! end-to-end tests: they invoke the built `olang` binary to produce a
//! standalone executable, then run *that* and check its behavior.

use std::process::Command;

fn olang_bin() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn tmp(sub: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("olang_build_{}_{}", sub, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn build_produces_a_runnable_standalone() {
    let dir = tmp("run");
    let src = dir.join("hi.ol");
    // A self-contained tool: only stdlib + the embedded `cli` package,
    // which travel inside the runtime, so nothing external is needed.
    std::fs::write(
        &src,
        r#"
use cli
let spec = #{ "name": "hi", "args": [ #{ "name": "who", "required": true } ] }
match cli.parse(spec, cli.args()) {
    Err(e) => { println("ERR " + e); os.exit(2) },
    Ok(a) => println("hi " + map_get(a, "who"))
}
"#,
    )
    .unwrap();
    let out = dir.join("hi");

    let build = Command::new(olang_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run olang build");
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(out.exists(), "output binary was not created");

    // The standalone runs its embedded program, and its own argv reaches
    // cli.args() — proof the payload is detected and dispatched.
    let happy = Command::new(&out).arg("Ada").output().expect("run tool");
    assert!(happy.status.success());
    assert_eq!(String::from_utf8_lossy(&happy.stdout).trim(), "hi Ada");

    // The tool's own error path (a missing required arg) exits non-zero.
    let miss = Command::new(&out).output().expect("run tool no args");
    assert_eq!(miss.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&miss.stdout).contains("missing required"),
        "stdout: {}",
        String::from_utf8_lossy(&miss.stdout)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_roundtrips_rich_language_features() {
    // `olang build` embeds the *parsed* AST (rung B) and the built tool
    // deserializes it at startup instead of re-parsing. This is only sound
    // if the serde round-trip is lossless across the language, so exercise a
    // spread — enums with payloads and patterns, structs, closures capturing
    // a variable, higher-order builtins, recursion, and string work — and
    // confirm the standalone produces exactly what the interpreter does.
    let dir = tmp("rich");
    let src = dir.join("rich.ol");
    std::fs::write(
        &src,
        r#"
type Shape = enum { Circle(Float), Rect(Float, Float) }
fn area(s) = match s {
    Circle(r) => 3.14 * r * r,
    Rect(w, h) => w * h
}
fn fact(n) = if n <= 1 => 1 else => n * fact(n - 1)
let shapes = [Circle(2.0), Rect(3.0, 4.0)]
let areas = map(shapes, (s) => area(s))
let bump = 10
let bumped = map([1, 2, 3], (x) => x + bump)
let total = sum(bumped)
println("areas=" + show(areas))
println("fact6=" + show(fact(6)) + " total=" + show(total))
"#,
    )
    .unwrap();
    let out = dir.join("rich");

    let build = Command::new(olang_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run olang build");
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    // The standalone (deserialized AST) matches the interpreter run of the
    // same source, line for line.
    let bundled = Command::new(&out).output().expect("run tool");
    let interpreted = Command::new(olang_bin())
        .arg(src.to_str().unwrap())
        .output()
        .expect("run source");
    assert!(bundled.status.success());
    assert_eq!(
        String::from_utf8_lossy(&bundled.stdout),
        String::from_utf8_lossy(&interpreted.stdout),
        "bundled AST diverged from the interpreter"
    );
    assert!(
        String::from_utf8_lossy(&bundled.stdout).contains("fact6=720"),
        "stdout: {}",
        String::from_utf8_lossy(&bundled.stdout)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_rejects_a_program_that_does_not_parse() {
    let dir = tmp("parse");
    let src = dir.join("broken.ol");
    std::fs::write(&src, "let x = = = broken\n").unwrap();
    let out = dir.join("broken");

    let build = Command::new(olang_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run olang build");
    assert!(!build.status.success(), "a broken program must not build");
    assert!(
        String::from_utf8_lossy(&build.stderr).contains("does not parse"),
        "stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(!out.exists() || std::fs::metadata(&out).is_ok());

    let _ = std::fs::remove_dir_all(&dir);
}

/// C2: a transparency record must be validated before anything trusts a
/// field of it. `format` selects which bytes the digest covers, and the
/// check was `>= 3` — so a bundle claiming format 99 took the format-3
/// path, recomputed a digest that does not cover the meta record, matched
/// it, and printed "verified". The only verdict worse than "unverifiable"
/// is a confident wrong one.
#[test]
fn a_bundle_claiming_an_unknown_format_is_refused() {
    let dir = tmp("meta_format");
    let src = dir.join("prog.ol");
    std::fs::write(&src, "println(\"hi\")\n").unwrap();
    let exe = dir.join("prog");
    let built = std::process::Command::new(olang_bin())
        .args(["build", src.to_str().unwrap(), "-o", exe.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(built.status.success(), "build failed");

    // Sanity: the honest bundle inspects clean.
    let good = std::process::Command::new(olang_bin())
        .args(["inspect", exe.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&good.stdout).contains("[verified]"),
        "the unmodified bundle should verify"
    );

    // Rewrite only `format`, leaving the digest untouched and still
    // correct for the payload it covers.
    let bytes = std::fs::read(&exe).unwrap();
    let n = bytes.len();
    let lens = &bytes[n - 32..n - 8];
    let get = |i: usize| u64::from_le_bytes(lens[i * 8..i * 8 + 8].try_into().unwrap()) as usize;
    let (sl, al, ml) = (get(0), get(1), get(2));
    let start = n - 32 - (sl + al + ml);
    let meta: serde_json::Value =
        serde_json::from_slice(&bytes[start + sl + al..start + sl + al + ml]).unwrap();
    let mut meta = meta.as_object().unwrap().clone();
    meta.insert("format".to_string(), serde_json::json!(99));
    let new_meta = serde_json::to_vec(&meta).unwrap();

    let mut forged = Vec::new();
    forged.extend_from_slice(&bytes[..start + sl + al]);
    forged.extend_from_slice(&new_meta);
    for len in [sl as u64, al as u64, new_meta.len() as u64] {
        forged.extend_from_slice(&len.to_le_bytes());
    }
    forged.extend_from_slice(&bytes[n - 8..]);
    let forged_path = dir.join("forged");
    std::fs::write(&forged_path, &forged).unwrap();

    let out = std::process::Command::new(olang_bin())
        .args(["inspect", forged_path.to_str().unwrap()])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !text.contains("[verified]"),
        "a bundle of an unknown format must not be reported as verified: {text}"
    );
}

//! `Any`: the type of any value. A struct field, a parameter or a
//! return that takes whatever arrives — the sandbox's `run` result, a
//! code-mode stub's `Result<Any, String>` — says so, and nothing is
//! checked there: not by the runtime on any tier, and not by `olang
//! check`. It once raised "names unknown type 'Any'".

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

const TIERS: [&[&str]; 3] = [&["--no-ovm"], &["--ovm-tier=1"], &[]];

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_any_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_in(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(olang())
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

const PROGRAM: &str = r#"type RunResult = struct { ok: Bool, value: Any }
fn wrap(v: Any) -> Any = v
fn result_of(v) -> Result<Any, String> = Ok(v)
fn many(xs: [Any]) -> List<Any> = xs
fn table(m: Map<String, Any>) -> Map<String, Any> = m
fn either(x: Int | Any) = x
let rs = [
    RunResult { ok: true, value: 1 },
    RunResult { ok: false, value: "two" },
    RunResult { ok: true, value: [3, "x"] },
    RunResult { ok: true, value: () },
]
println(show(map(rs, (r) => r.value)))
println(show(map([1, "a", 2.5, true, [1], #{ "k": 1 }], wrap)))
println(show(result_of("x")))
println(show(many([1, "b"])))
println(show(table(#{ "a": 1, "b": "c" })))
println(show(either("s")))
fn hot(n) = fold(range(0, n), 0, (acc, i) => acc + len(show(wrap(if i % 2 == 0 => i else => "s"))))
println(show(hot(400)))
let fixed: Any = "anything"
println(fixed)
match attempt(() => RunResult { ok: 1, value: 2 }) {
    Ok(_) => println("unchecked")
    Err(e) => println(e)
}
"#;

#[test]
fn any_takes_every_value_on_every_tier() {
    let dir = scratch("tiers");
    std::fs::write(dir.join("main.ol"), PROGRAM).unwrap();
    let mut first: Option<String> = None;
    for flags in TIERS {
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(["run", "main.ol"]);
        let (out, err, ok) = run_in(&dir, &args);
        assert!(ok, "flags {flags:?}: {out}{err}");
        assert!(
            out.contains("[1, \"two\", [3, \"x\"], ()]"),
            "flags {flags:?}: {out}"
        );
        // the field beside it is still checked
        assert!(
            out.contains("field 'ok' of RunResult expects Bool, got Int"),
            "flags {flags:?}: {out}"
        );
        match &first {
            None => first = Some(out),
            Some(seen) => assert_eq!(&out, seen, "flags {flags:?}"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_says_nothing_about_any() {
    let dir = scratch("check");
    std::fs::write(
        dir.join("main.ol"),
        PROGRAM.replace("ok: 1, value: 2", "ok: true, value: 2"),
    )
    .unwrap();
    let (out, err, ok) = run_in(&dir, &["check", "main.ol"]);
    assert!(ok, "{out}{err}");
    // a named argument into an `Any` parameter is not judged either
    std::fs::write(
        dir.join("named.ol"),
        "fn put(key: String, value: Any) = key\nprintln(put(key: \"k\", value: [1, \"two\"]))\nprintln(put(\"k\", value: 3))\n",
    )
    .unwrap();
    let (out, err, ok) = run_in(&dir, &["check", "named.ol"]);
    assert!(ok, "{out}{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

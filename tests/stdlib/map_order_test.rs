//! A map keeps its keys in the order they were first set (a Python dict's,
//! a JavaScript object's order), on every tier: iteration, printing, JSON
//! and a record's columns follow it; equality ignores it.

use std::process::Command;

fn run(src: &str, flags: &[&str]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("order.ol");
    std::fs::write(&path, src).unwrap();
    let mut args: Vec<&str> = flags.to_vec();
    let p = path.to_string_lossy().to_string();
    args.push(&p);
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(&args)
        .current_dir(dir.path())
        .output()
        .expect("olang runs");
    assert!(
        out.status.success(),
        "{flags:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

const PROGRAM: &str = r#"
fn build(n) = fold(range(0, n), #{}, (m, i) => map_set(m, "k" + to_string(n - i), i))
let m = #{ "zeta": 1, "alpha": 2 } |> map_set("mid", 3) |> map_set("zeta", 9)
println(map_keys(m))
println(m)
println(map_values(m))
println(map_remove(m, "alpha"))
println(#{ ...m, "alpha": 0, "new": 4 })
println(entries(build(3)))
println(#{ "a": 1, "b": 2 } == #{ "b": 2, "a": 1 })
let o = { y: 1, x: 2 }
println(o)
println(map_keys(map_set(o, "w", 3)))
println(unwrap(json.stringify(#{ "z": 1, "a": [#{ "q": 1, "b": 2 }] })))
let back = unwrap(json.parse("{\"repo\": \"loom\", \"files\": 3, \"lines\": 40}"))
println(map_keys(back))
println(unwrap(json.stringify(back)))
println(map_keys(unwrap(toml.parse("name = \"x\"\nversion = \"1\"\nedition = 2"))))
let f = ods.frame_from_records([#{ "repo": "olang", "files": 10, "lines": 900 }, #{ "repo": "loom", "files": 3, "lines": 40, "commits": 7 }])
println(ods.columns(f))
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE t (name TEXT, lines INT, commits INT)"))
unwrap(db.execute(c, "INSERT INTO t VALUES ('a', 1, 2)"))
let rows = unwrap(db.query(c, "SELECT name, lines, commits FROM t"))
println(map_keys(rows[0]))
println(ods.columns(ods.frame_from_records(rows)))
println(meta.lit(#{ "b": 1, "a": #{ "d": 2, "c": 3 } }))
println(http.encode_query(#{ "q": "x", "page": 2 }))
"#;

const EXPECTED: &str = r#"["zeta", "alpha", "mid"]
#{"zeta": 9, "alpha": 2, "mid": 3}
[9, 2, 3]
#{"zeta": 9, "mid": 3}
#{"zeta": 9, "alpha": 0, "mid": 3, "new": 4}
[("k3", 0), ("k2", 1), ("k1", 2)]
true
{ y: 1, x: 2 }
["y", "x", "w"]
{"z":1,"a":[{"q":1,"b":2}]}
["repo", "files", "lines"]
{"repo":"loom","files":3,"lines":40}
["name", "version", "edition"]
["repo", "files", "lines", "commits"]
["name", "lines", "commits"]
["name", "lines", "commits"]
#{"b": 1, "a": #{"d": 2, "c": 3}}
q=x&page=2
"#;

#[test]
fn a_map_keeps_its_keys_in_the_order_they_were_set_on_every_tier() {
    for flags in [&[][..], &["--no-ovm"][..]] {
        assert_eq!(run(PROGRAM, flags), EXPECTED, "{flags:?}");
    }
}

/// Built in a compiled loop (the bytecode tier and the JIT hold records
/// as shapes): the order still the order the keys were set, a key set
/// again kept in its place.
#[test]
fn a_record_built_on_the_compiled_tiers_keeps_its_order() {
    let src = r#"
fn rec(i) = { b: i, a: i * 2 }
fn grow(n) = {
    let mut acc = #{}
    for i in 0..n {
        acc = map_set(acc, "x" + to_string(n - i), i)
    }
    acc
}
let mut last = ()
for i in 0..2000 {
    last = map_set(rec(i), "b", -1)
}
println(last)
println(map_keys(grow(4)))
"#;
    for flags in [&[][..], &["--no-ovm"][..]] {
        assert_eq!(
            run(src, flags),
            "{ b: -1, a: 3998 }\n[\"x4\", \"x3\", \"x2\", \"x1\"]\n",
            "{flags:?}"
        );
    }
}

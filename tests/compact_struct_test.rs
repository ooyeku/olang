//! Compact structs (OL-431): a struct compiled code builds is one
//! allocation with its scalar fields raw, and it crosses into the
//! interpreter as itself, wrapped, instead of expanding into a map of
//! named fields. Both tiers agree on everything a struct does across that
//! crossing, and the memory budget holds.

use olang::ast::Value;
use olang::{Interpreter, Parser};
use std::process::Command;

fn eval(source: &str, tier: bool) -> Result<Value, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let parser = Parser::new();
            let program = parser.parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if tier {
                interpreter.enable_bytecode_tier(1, false);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .expect("spawn")
        .join()
        .expect("join")
}

/// The source's value on the tree-walker alone and with the compiled
/// tiers, which must agree; the value itself is returned to check.
fn agree(src: &str) -> Value {
    let interp = eval(src, false).expect("interpreted");
    let tiered = eval(src, true).expect("tiered");
    assert_eq!(
        interp.to_string(),
        tiered.to_string(),
        "the tiers disagree on:\n{src}"
    );
    tiered
}

/// A struct-building function hot enough to compile, its results used at
/// the top level (the interpreter) and handed back to compiled code.
const PRELUDE: &str = "\
type P = struct { a: Int, b: Int, c: Float, d: String, e: Bool }\n\
type Box = struct { p: P, xs: List, m: Map }\n\
fn mk(i) = P { a: i, b: i + 1, c: 1.5, d: `n${i}`, e: i % 2 == 0 }\n\
fn many(n) = {\n\
    let mut xs = []\n\
    let mut i = 0\n\
    while i < n { xs = xs + [mk(i)]; i = i + 1 }\n\
    xs\n\
}\n\
fn boxed(i) = Box { p: mk(i), xs: [mk(i), mk(i + 1)], m: #{ \"k\": mk(i) } }\n\
fn total(ps) = fold(ps, 0, (acc, p) => acc + p.a + p.b)\n\
fn get_d(p) = p.d\n\
let ps = many(200)\n";

#[test]
fn field_reads_agree_across_the_crossing() {
    let v = agree(&format!(
        "{PRELUDE}[ps[7].a, ps[7].b, ps[7].c, ps[7].d, ps[7].e, len(ps)]"
    ));
    assert_eq!(v.to_string(), "[7, 8, 1.5, \"n7\", false, 200]");
}

#[test]
fn a_struct_passed_back_into_compiled_code_reads_the_same() {
    let v = agree(&format!("{PRELUDE}[total(ps), get_d(ps[3]), get_d(mk(4))]"));
    assert_eq!(v.to_string(), "[40000, \"n3\", \"n4\"]");
}

#[test]
fn equality_agrees() {
    agree(&format!(
        "{PRELUDE}[ps[1] == mk(1), ps[1] == ps[2], ps[1] != mk(1), \
         ps[1] == P {{ a: 1, b: 2, c: 1.5, d: \"n1\", e: false }}, \
         boxed(3) == boxed(3), boxed(3) == boxed(4)]"
    ));
    let v = eval(
        &format!(
            "{PRELUDE}[ps[1] == mk(1), ps[1] == ps[2], \
             ps[1] == P {{ a: 1, b: 2, c: 1.5, d: \"n1\", e: false }}, boxed(3) == boxed(3)]"
        ),
        true,
    )
    .unwrap();
    assert_eq!(v.to_string(), "[true, false, true, true]");
}

#[test]
fn show_and_json_agree() {
    agree(&format!(
        "{PRELUDE}[to_string(ps[2]), show(boxed(1)), json.stringify(ps[2]), \
         json.stringify(boxed(2))]"
    ));
}

#[test]
fn hash_agrees_with_an_interpreter_built_struct() {
    let v = agree(&format!(
        "{PRELUDE}[hash(ps[5]) == hash(P {{ a: 5, b: 6, c: 1.5, d: \"n5\", e: false }}), \
         hash(boxed(2)) == hash(boxed(2))]"
    ));
    assert_eq!(v.to_string(), "[true, true]");
}

#[test]
fn value_encode_round_trips() {
    let v = agree(&format!(
        "{PRELUDE}let b = value.encode(boxed(3))\n\
         let back = unwrap(value.decode(b))\n\
         [back == boxed(3), back.p.d, value.encode(ps[4]) == value.encode(mk(4))]"
    ));
    assert_eq!(v.to_string(), "[true, \"n3\", true]");
}

#[test]
fn pattern_matching_agrees() {
    let v = agree(&format!(
        "{PRELUDE}fn kind(p) = match p {{ P {{ a, d }} => `${{a}}:${{d}}`, _ => \"?\" }}\n\
         [match ps[9] {{ P {{ a, d }} => `${{a}}:${{d}}`, _ => \"?\" }}, kind(ps[9]), kind(mk(2))]"
    ));
    assert_eq!(v.to_string(), "[\"9:n9\", \"9:n9\", \"2:n2\"]");
}

#[test]
fn map_get_keys_and_updates_agree() {
    let v = agree(&format!(
        "{PRELUDE}let p = ps[6]\n\
         let q = map_set(p, \"a\", 100)\n\
         [map_get(p, \"d\"), map_has_key(p, \"c\"), map_keys(p), q.a, p.a, \
          q == mk(6), map_get(map_set(boxed(1), \"m\", 0), \"m\")]"
    ));
    assert_eq!(
        v.to_string(),
        "[\"n6\", true, [\"a\", \"b\", \"c\", \"d\", \"e\"], 100, 6, false, 0]"
    );
}

#[test]
fn nested_structs_in_lists_and_maps_agree() {
    let v = agree(&format!(
        "{PRELUDE}let b = boxed(10)\n\
         [b.p.a, b.xs[1].a, b.m.k.d, map_get(b.m, \"k\").b, len(b.xs), \
          get_d(b.xs[0]), total(b.xs)]"
    ));
    assert_eq!(v.to_string(), "[10, 11, \"n10\", 11, 2, \"n10\", 44]");
}

#[test]
fn a_struct_the_interpreter_renames_or_writes_keeps_its_own_fields() {
    // A write turns the wrapped struct into the interpreter's own map;
    // the original, still held, is unchanged.
    let v = agree(&format!(
        "{PRELUDE}let p = ps[3]\n\
         let r = {{ ...p, a: 0 }}\n\
         [p.a, r.a, r.d, get_d(map_set(p, \"d\", \"z\")), get_d(p)]"
    ));
    assert_eq!(v.to_string(), "[3, 0, \"n3\", \"z\", \"n3\"]");
}

/// The probe of OL-431: five fields, built in a compiled loop, measured in
/// real allocator bytes inside compiled code and once returned to the
/// top level.
#[test]
fn a_five_field_struct_costs_under_100_bytes_in_compiled_code() {
    let dir = std::env::temp_dir().join(format!("olang_compact_struct_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = "type P = struct { a: Int, b: Int, c: Float, d: Int, e: Int }\n\
fn build(n) = {\n\
    let mut xs = []\n\
    let mut i = 0\n\
    while i < n { xs = xs + [P { a: i, b: i + 1, c: 1.5, d: i * 2, e: 7 }]; i = i + 1 }\n\
    xs\n\
}\n\
fn inside(n) = {\n\
    let before = runtime.memory().heap\n\
    let xs = build(n)\n\
    let after = runtime.memory().heap\n\
    (after - before) / n + len(xs) * 0\n\
}\n\
let n = 100000\n\
println(to_string(inside(n)))\n\
let h0 = runtime.memory().heap\n\
let ys = build(n)\n\
let h1 = runtime.memory().heap\n\
println(to_string((h1 - h0) / n))\n\
println(to_string(ys[5].d))\n";
    std::fs::write(dir.join("main.ol"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("main.ol")
        .current_dir(&dir)
        .env("OLANG_COMPILE_CACHE", "0")
        .output()
        .expect("run olang");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<&str> = stdout.lines().collect();
    let inside: i64 = lines[0].trim().parse().unwrap();
    let top: i64 = lines[1].trim().parse().unwrap();
    assert_eq!(lines[2], "10");
    assert!(inside < 100, "a struct costs {inside} B in compiled code");
    // Wrapped at the crossing: a list slot of the interpreter's and the
    // same object, no map of named fields (it was 702 B).
    assert!(top < 200, "a struct costs {top} B once returned");
}

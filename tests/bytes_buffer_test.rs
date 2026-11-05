//! Bytes as a buffer (OL-428): views, the builder, joins, typed reads,
//! pack and unpack, searching, and byte reads in compiled code. Each
//! program runs on the interpreter and on the bytecode tier (compiled at
//! the first call, so hot loops go native), which must agree.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str, tier: bool) -> Result<Value, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if tier {
                interpreter.enable_bytecode_tier(1, false);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .unwrap()
        .join()
        .unwrap()
}

fn shows(source: &str) -> String {
    let interpreted = eval(source, false);
    let tiered = eval(source, true);
    assert_eq!(tiered, interpreted, "the tiers disagree on:\n{source}");
    match &interpreted {
        Ok(Value::String(s)) => s.to_string(),
        Ok(v) => v.to_string(),
        Err(e) => panic!("{source}\nraised: {e}"),
    }
}

fn raises(source: &str) -> String {
    let interpreted = eval(source, false);
    assert_eq!(
        eval(source, true),
        interpreted,
        "the tiers disagree on:\n{source}"
    );
    match interpreted {
        Ok(v) => panic!("expected an error, got {v}"),
        Err(e) => e,
    }
}

#[test]
fn a_builder_grows_in_place_and_builds_once() {
    assert_eq!(
        shows(
            "fn make(n) = {\n    let out = bytes.builder()\n    for i in range(0, n) { bytes.push(out, i % 256) }\n    bytes.push(out, \"ok\")\n    bytes.push(out, bytes.from_list([1, 2]))\n    bytes.push(out, [3, 4])\n    let b = bytes.build(out)\n    [len(b), b[0], b[n], b[n + 1], len(out), len(bytes.build(out))]\n}\nmake(1000)"
        ),
        "[1006, 0, 111, 107, 0, 0]"
    );
}

#[test]
fn a_builder_belongs_to_its_thread() {
    let e = raises("let out = bytes.builder()\nlet ch = chan.new()\nchan.send(ch, out)");
    assert!(e.contains("cannot be sent through a channel"), "{e}");
    let e = raises("bytes.push(bytes.builder(), 300)");
    assert!(e.contains("outside a byte's range"), "{e}");
}

#[test]
fn join_index_of_and_compare() {
    assert_eq!(
        shows(
            "let j = bytes.join([bytes.from_string(\"ab\"), \"cd\", bytes.from_list([])], \"-\")\n\
             [unwrap(bytes.to_string(j)), bytes.index_of(j, \"cd\"), bytes.index_of(j, 45, 3), bytes.index_of(j, \"zz\"),\n\
              bytes.compare(j, j), bytes.compare(bytes.from_list([1]), bytes.from_list([1, 0])), bytes.compare(bytes.from_list([2]), bytes.from_list([1, 9]))]"
        ),
        "[\"ab-cd-\", 3, 5, (), 0, -1, 1]"
    );
}

#[test]
fn typed_reads_in_both_byte_orders() {
    assert_eq!(
        shows(
            "let b = bytes.from_list([1, 2, 3, 4, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 248, 63])\n\
             [bytes.get_u8(b, 0), bytes.get_i8(b, 4), bytes.get_u16(b, 0), bytes.get_u16(b, 0, \"be\"),\n\
              bytes.get_u32(b, 0), bytes.get_u32(b, 0, \"be\"), bytes.get_i32(b, 4), bytes.get_u32(b, 4),\n\
              bytes.get_f64(b, 8), bytes.get_u64(b, 4) < 0]"
        ),
        "[1, -1, 513, 258, 67305985, 16909060, -1, 4294967295, 1.5, false]"
    );
    let e = raises("bytes.get_u32(bytes.from_list([1, 2, 3]), 0)");
    assert!(e.contains("run past the end"), "{e}");
}

#[test]
fn pack_and_unpack_agree() {
    assert_eq!(
        shows(
            "let layout = \"u8 i16 u32be i64 f32 f64be u64\"\n\
             let vals = [255, -2, 16909060, -9223372036854775808, 1.5, -0.25, -1]\n\
             let b = bytes.pack(layout, vals)\n\
             [len(b), bytes.unpack(layout, b) == vals, bytes.unpack(\"u32be\", b, 3)]"
        ),
        "[35, true, [16909060]]"
    );
    let e = raises("bytes.pack(\"u8\", [256])");
    assert!(e.contains("does not fit in a u8"), "{e}");
    let e = raises("bytes.pack(\"u24\", [1])");
    assert!(e.contains("unknown type"), "{e}");
}

#[test]
fn a_slice_is_a_view_equal_to_a_copy() {
    assert_eq!(
        shows(
            "let b = bytes.from_list(range(0, 100))\n\
             let s = bytes.slice(b, 10, 20)\n\
             let t = bytes.slice(s, 5, 50)\n\
             [len(s), s[0], t[0], len(t), s == bytes.from_list(range(10, 20)), hash(s) == hash(bytes.from_list(range(10, 20))),\n\
              unwrap(value.decode(value.encode(t))) == t, len(bytes.slice(b, 50, 10))]"
        ),
        "[10, 10, 15, 5, true, true, true, 0]"
    );
}

#[test]
fn bytes_index_natively_and_deopt_out_of_range() {
    let src = "fn total(b) = {\n    let mut t = 0\n    let mut i = 0\n    let n = len(b)\n    while i < n { t = t + b[i]; i = i + 1 }\n    t\n}\n\
               fn at(b, i) = b[i]\n\
               let b = bytes.from_list(map(range(0, 3000), (i) => i % 256))\n\
               let mut s = 0\n\
               for r in range(0, 5) { s = s + total(b) + total(bytes.slice(b, 100, 200)) }\n\
               [s, at(b, 2999), attempt(() => at(b, 3000)), attempt(() => at(b, -1))]";
    assert_eq!(
        shows(src),
        "[1954130, 183, Err(\"Index 3000 out of bounds for Bytes of length 3000\"), Err(\"Index -1 out of bounds for Bytes of length 3000\")]"
    );
    // `total` runs native code.
    let dir = std::env::temp_dir().join(format!("olang-bytes-native-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.ol");
    std::fs::write(&file, format!("{src}\n")).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg(&file)
        .env("OLANG_TIER_STATS", "1")
        .output()
        .expect("run olang");
    let _ = std::fs::remove_dir_all(&dir);
    let stats = String::from_utf8_lossy(&out.stderr);
    let line = stats
        .lines()
        .find(|l| l.starts_with("tier-fn: total "))
        .unwrap_or_else(|| panic!("no tier line for total:\n{stats}"));
    assert!(
        !line.contains("native_calls=0"),
        "total stayed off native: {line}"
    );
}

//! The program image from the language side (roadmap W10): `meta.encode`
//! yields bytes behind the `olb1` header, a syntax error is an `Err`,
//! and the Ok(String) miss names its fix.

use std::process::Command;

fn run(source: &str) -> (String, i32) {
    let dir = std::env::temp_dir().join("olang_boot_bundle_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let path = dir.join(format!("t{:x}.ol", h.finish()));
    std::fs::write(&path, source).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("run")
        .arg(&path)
        .output()
        .expect("run");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn meta_encode_yields_an_image_behind_its_header() {
    let (out, rc) = run(
        "let image = unwrap(meta.encode(\"fn f(x) = x * 2\\nprintln(to_string(f(21)))\"))\n\
         println(unwrap(bytes.to_string(bytes.slice(image, 0, 4))))\n\
         println(to_string(bytes.len(image) > 8))\n\
         match meta.encode(\"fn (\") { Ok(b) => println(\"ok\"), Err(e) => println(\"err\") }\n\
         // Macros are expanded before encoding: a macro-using program encodes.\n\
         let a = meta.encode(\"meta fn twice(e) = `${e} * 2`\\nprintln(@twice(21))\")\n\
         println(to_string(is_ok(a)))\n\
         // A macro that fails to expand is an Err, not a crash.\n\
         match meta.encode(\"println(@nope(1))\") { Ok(b) => println(\"ok\"), Err(e) => println(\"err\") }",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, "olb1\ntrue\nerr\ntrue\nerr\n");
}

#[test]
fn meta_encode_names_the_unwrap_fix() {
    let (out, rc) = run("meta.encode(Ok(\"println(1)\"))");
    assert_ne!(rc, 0);
    assert!(out.contains("unwrap the read first"), "{out}");
}

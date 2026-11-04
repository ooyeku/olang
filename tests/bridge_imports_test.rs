//! A program's import of a builtin's name is the program's, also in a
//! function the bytecode tier hands to its bridge (OL-419).
//!
//! A function holding a `spawn` stays interpreted, so a compiled caller
//! runs it through the VM's bridge interpreter. The bridge's root holds
//! the builtins, and an imported `find` lost to the builtin `find` there:
//! `find(a, 2, 3)` raised "Arity mismatch: expected 2, got 3".

use std::process::Command;

#[test]
fn an_import_shadowing_a_builtin_reaches_a_function_the_bridge_runs() {
    let dir = std::env::temp_dir().join(format!("olang_ol419_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/routes.ol"),
        "share fn find(a, b, c) = a + b + c\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.ol"),
        "use lib.routes { find }\n\
fn outer(a) = inner(a)\n\
fn inner(a) = {\n\
    let x = find(a, 2, 3)\n\
    if false => spawn { 1 } else => x\n\
}\n\
fn builtin_still(xs) = {\n\
    let y = map(xs, (v) => v * 2)\n\
    if false => spawn { 1 } else => y\n\
}\n\
fn via(xs) = builtin_still(xs)\n\
println(outer(1))\n\
println(via([1, 2]))\n",
    )
    .unwrap();
    for flag in [None, Some("--no-ovm")] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
        if let Some(f) = flag {
            cmd.arg(f);
        }
        let out = cmd.arg("main.ol").current_dir(&dir).output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "6\n[2, 4]\n",
            "{flag:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

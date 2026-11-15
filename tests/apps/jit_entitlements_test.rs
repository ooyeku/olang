//! The JIT under the hardened runtime (OL-442): a copy of the runtime
//! signed with the hardened runtime and `allow-jit` alone runs hot code
//! natively (its code pages are MAP_JIT), and a copy signed with no
//! entitlement at all runs the same program on bytecode instead of being
//! killed. The copies are ad-hoc signed in a scratch directory; the
//! shipped entitlements file is one of the sets tried.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

const HOT: &str = "fn f(n) = { let mut a = 0; let mut i = 0; while i < n { a = (a + i * 3) % 1000003; i = i + 1 }; a }\n\
let mut t = 0\n\
for k in range(0, 300) { t = t + f(20000) }\n\
println(t)\n";

fn plist(keys: &[&str]) -> String {
    let mut body = String::new();
    for k in keys {
        body.push_str(&format!("<key>{k}</key><true/>"));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>{body}</dict></plist>\n"
    )
}

/// A hardened, ad-hoc signed copy of the runtime with these entitlements.
fn signed_copy(dir: &Path, name: &str, entitlements: &Path) -> Option<PathBuf> {
    let exe = dir.join(name);
    std::fs::copy(env!("CARGO_BIN_EXE_olang"), &exe).ok()?;
    let ok = Command::new("codesign")
        .args([
            "--force",
            "-s",
            "-",
            "--options",
            "runtime",
            "--entitlements",
        ])
        .arg(entitlements)
        .arg(&exe)
        .output()
        .ok()?
        .status
        .success();
    ok.then_some(exe)
}

/// Run the hot loop: (exit code, stdout).
fn run(exe: &Path, program: &Path) -> (Option<i32>, String) {
    let out = Command::new(exe)
        .arg("--ovm-stats")
        .arg(program)
        .env_remove("OLANG_NO_JIT")
        .output()
        .expect("run the signed copy");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

fn native_calls(stdout: &str) -> u64 {
    stdout
        .split(" native calls")
        .next()
        .and_then(|s| s.rsplit(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[test]
fn the_jit_needs_allow_jit_alone_and_falls_back_without_it() {
    if Command::new("codesign").arg("--help").output().is_err() {
        eprintln!("no codesign here; skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("olang_jit_ent_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("hot.ol");
    std::fs::write(&program, HOT).unwrap();

    let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/macos/entitlements.plist");
    let shipped_text = std::fs::read_to_string(&shipped).unwrap();
    assert!(
        shipped_text.contains("com.apple.security.cs.allow-jit")
            && !shipped_text.contains("allow-unsigned-executable-memory")
            && !shipped_text.contains("disable-library-validation"),
        "the shipped entitlements are allow-jit alone"
    );

    let sets: Vec<(&str, PathBuf, bool)> = vec![
        ("none", dir.join("none.plist"), false),
        ("shipped", shipped.clone(), true),
        ("jit", dir.join("jit.plist"), true),
        ("unsigned", dir.join("unsigned.plist"), true),
    ];
    std::fs::write(&sets[0].1, plist(&[])).unwrap();
    std::fs::write(&sets[2].1, plist(&["com.apple.security.cs.allow-jit"])).unwrap();
    std::fs::write(
        &sets[3].1,
        plist(&["com.apple.security.cs.allow-unsigned-executable-memory"]),
    )
    .unwrap();

    for (name, entitlements, native) in &sets {
        let exe = signed_copy(&dir, &format!("olang-{name}"), entitlements)
            .unwrap_or_else(|| panic!("sign a copy with the {name} entitlements"));
        let (code, stdout) = run(&exe, &program);
        assert_eq!(
            code,
            Some(0),
            "{name}: the hot loop runs to the end:\n{stdout}"
        );
        assert!(
            stdout.starts_with("290460900"),
            "{name}: the right answer:\n{stdout}"
        );
        let calls = native_calls(&stdout);
        if *native {
            assert!(calls > 0, "{name}: the JIT ran native code:\n{stdout}");
        } else {
            assert_eq!(
                calls, 0,
                "{name}: no native code without allow-jit:\n{stdout}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

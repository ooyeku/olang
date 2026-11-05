//! A built program runs only on the runtime that built it (OL-443): a
//! payload whose record names another olang version, another AST format
//! or another commit is refused with its own exit code and a message
//! that says which, and the record's runtime identity is inside the
//! digest, so relabelling a payload fails `olang inspect --verify`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn tmp(sub: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("olang_payload_rt_{}_{}", sub, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn build(dir: &Path) -> PathBuf {
    std::fs::write(dir.join("hi.ol"), "println(\"hi\")\n").unwrap();
    let out = Command::new(olang())
        .current_dir(dir)
        .args(["build", "hi.ol", "-o", "hi"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    dir.join("hi")
}

/// The record's JSON and where it sits.
fn record(bytes: &[u8]) -> (usize, usize) {
    assert_eq!(&bytes[bytes.len() - 8..], b"oLaNgMeT");
    let lens = &bytes[bytes.len() - 32..bytes.len() - 8];
    let meta_len = u64::from_le_bytes(lens[16..24].try_into().unwrap()) as usize;
    let end = bytes.len() - 32;
    (end - meta_len, end)
}

/// The binary with one field of its record replaced by a value of the
/// same length (so every length in the footer still holds).
fn relabel(bin: &Path, field: &str, to: impl Fn(&str) -> String) -> PathBuf {
    let mut bytes = std::fs::read(bin).unwrap();
    let (start, end) = record(&bytes);
    let meta: serde_json::Value = serde_json::from_slice(&bytes[start..end]).unwrap();
    let old = meta[field].as_str().unwrap().to_string();
    let new = to(&old);
    assert_eq!(old.len(), new.len(), "same length");
    let text = String::from_utf8(bytes[start..end].to_vec()).unwrap();
    let needle = format!("\"{field}\":\"{old}\"");
    assert!(text.contains(&needle), "{text}");
    let patched = text.replacen(&needle, &format!("\"{field}\":\"{new}\""), 1);
    bytes.splice(start..end, patched.into_bytes());
    let out = bin.with_file_name(format!("hi-{field}"));
    std::fs::write(&out, &bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("codesign").args(["--force", "-s", "-"]).arg(&out).output();
    }
    out
}

#[test]
fn a_payload_from_another_runtime_is_refused() {
    let dir = tmp("refuse");
    let bin = build(&dir);
    let ok = Command::new(&bin).output().unwrap();
    assert_eq!(ok.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&ok.stdout), "hi\n");

    let cases: [(&str, &str); 3] = [
        ("olang_version", "built by olang 9"),
        ("ast_format", "AST format"),
        ("build_commit", "at commit"),
    ];
    for (field, says) in cases {
        let bytes = std::fs::read(&bin).unwrap();
        let (s, e) = record(&bytes);
        let meta: serde_json::Value = serde_json::from_slice(&bytes[s..e]).unwrap();
        if meta[field].as_str().unwrap_or("").is_empty() {
            // an olang built outside a checkout knows no commit
            continue;
        }
        let other = relabel(&bin, field, |old| {
            let mut c: Vec<char> = old.chars().collect();
            c[0] = if c[0] == '9' { '8' } else { '9' };
            c.into_iter().collect()
        });
        let out = Command::new(&other).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(65), "{field}: refused with its own code: {err}");
        assert!(out.stdout.is_empty(), "{field}: nothing of the program ran");
        assert!(
            err.contains("refusing to run") && err.contains(says),
            "{field}: the message says why: {err}"
        );

        // and the relabelled record no longer verifies
        let inspect = Command::new(olang())
            .args(["inspect", "--verify"])
            .arg(&other)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&inspect.stdout);
        assert!(!inspect.status.success(), "{field}: --verify fails:\n{text}");
        assert!(text.contains("MISMATCH"), "{field}: the digest says so:\n{text}");
        assert!(text.contains("runs here:    no"), "{field}:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inspect_names_the_runtime_a_payload_was_built_for() {
    let dir = tmp("inspect");
    let bin = build(&dir);
    let out = Command::new(olang())
        .args(["inspect", "--verify"])
        .arg(&bin)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("runtime:      AST format"), "{text}");
    assert!(text.contains("runs here:    yes"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

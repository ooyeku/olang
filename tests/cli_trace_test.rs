//! `olang --trace`: where the start of a run goes, on stderr. Each module a
//! `use` finds says where it came from, the landmarks are timed, and the
//! summary at exit counts the modules and compiles. The program's own
//! output is untouched.

use std::process::Command;

#[test]
fn trace_reports_modules_landmarks_and_the_summary() {
    let dir = std::env::temp_dir().join(format!("olang_cli_trace_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helper.ol"), "share fn twice(x) = x * 2\n").unwrap();
    std::fs::write(
        dir.join("main.ol"),
        "use helper { twice }\nuse heap\nprintln(twice(21))\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["--trace", "main.ol"])
        .current_dir(&dir)
        .env("OLANG_HOME", dir.join(".home"))
        .env_remove("OLANG_BOOT_TRACE")
        .output()
        .expect("run olang");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stdout}{stderr}");

    assert_eq!(stdout.trim(), "42", "the program's output stays its own");
    assert!(stderr.contains("the command line read"), "{stderr}");
    assert!(stderr.contains("the entry file parsed"), "{stderr}");
    assert!(
        stderr.contains("use helper:") && stderr.contains("(beside the importing file)"),
        "{stderr}"
    );
    assert!(stderr.contains("use heap: built into olang"), "{stderr}");
    assert!(
        stderr.contains("exit") && stderr.contains("modules "),
        "{stderr}"
    );

    // Without the flag, nothing extra.
    let quiet = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("main.ol")
        .current_dir(&dir)
        .env("OLANG_HOME", dir.join(".home"))
        .env_remove("OLANG_BOOT_TRACE")
        .output()
        .expect("run olang");
    assert_eq!(String::from_utf8_lossy(&quiet.stderr), "");
    let _ = std::fs::remove_dir_all(&dir);
}

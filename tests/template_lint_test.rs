//! Template-literal lints (roadmap W8): backtick strings process no
//! escapes and do not nest, and both mistakes surfaced only at run time.
//! The semantics are unchanged by ruling — instead, `olang check` warns
//! on escape-looking sequences (`\n`/`\t`/`\r`) inside templates, and
//! the parse errors produced by a backtick inside `${...}` now name the
//! nesting rule. Runtime cases are differentials (tiered vs `--no-ovm`).

use std::path::PathBuf;
use std::process::Command;

fn project(files: &[(&str, &str)]) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (p, s) in files {
        p.hash(&mut h);
        s.hash(&mut h);
    }
    let dir = std::env::temp_dir().join(format!("olang_template_lint_{:x}", h.finish()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, source) in files {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).expect("mkdir");
        std::fs::write(&full, source).expect("write");
    }
    dir
}

fn run_in(dir: &std::path::Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
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
fn check_warns_on_escape_looking_sequences_in_templates() {
    let dir = project(&[(
        "bar.ol",
        "let done = 3\n\
         print(`\\rprogress ${done}\\t of 10\\n`)\n",
    )]);
    let (out, rc) = run_in(&dir, &["check", "bar.ol"]);
    // Advisory only: the file is clean, the program still runs.
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("3 warnings"), "{out}");
    assert!(
        out.contains("backtick template is a literal backslash"),
        "{out}"
    );
    // Each warning points at the exact backslash, not the statement.
    assert!(out.contains("bar.ol:2:8"), "{out}");
}

#[test]
fn deliberate_and_interpolated_escapes_stay_silent() {
    let dir = project(&[(
        "fine.ol",
        // `\\n` opts out; `"\n"` inside `${...}` is real string syntax;
        // a double-quoted string outside templates is untouched; and a
        // repeated `\r` warns once per template, not per occurrence.
        "println(`deliberate \\\\n two chars`)\n\
         println(`joined: ${ \"a\\nb\" }`)\n\
         println(\"real\\nescape\")\n",
    )]);
    let (out, rc) = run_in(&dir, &["check", "fine.ol"]);
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("1 file clean"), "{out}");
    assert!(!out.contains("warning"), "{out}");
}

#[test]
fn repeated_sequences_warn_once_per_template() {
    let dir = project(&[(
        "spin.ol",
        "println(`\\r one \\r two \\r three`)\n\
         println(`\\r a second template warns again`)\n",
    )]);
    let (out, rc) = run_in(&dir, &["check", "spin.ol"]);
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("2 warnings"), "{out}");
}

#[test]
fn the_warned_program_still_runs_identically_on_both_tiers() {
    let dir = project(&[(
        "run.ol",
        "let done = 7\n\
         println(`\\rprogress ${done}\\n`)\n",
    )]);
    let (tiered, rc_t) = run_in(&dir, &["run", "run.ol"]);
    let (oracle, rc_o) = run_in(&dir, &["--no-ovm", "run", "run.ol"]);
    assert_eq!(tiered, oracle);
    assert_eq!(rc_t, rc_o);
    assert_eq!(rc_t, 0, "{tiered}");
    // The escapes are literal characters, exactly as before the lint.
    assert!(tiered.contains("\\rprogress 7\\n"), "{tiered:?}");
}

#[test]
fn a_nested_backtick_parse_error_names_the_nesting_rule() {
    let dir = project(&[("nested.ol", "println(`outer ${ `inner` } end`)\n")]);
    let (out, rc) = run_in(&dir, &["check", "nested.ol"]);
    assert_ne!(rc, 0);
    assert!(out.contains("templates do not nest"), "{out}");
    assert!(out.contains("bind the inner template to a name"), "{out}");
    // The same message reaches `run`.
    let (out, rc) = run_in(&dir, &["run", "nested.ol"]);
    assert_ne!(rc, 0);
    assert!(out.contains("templates do not nest"), "{out}");
}

#[test]
fn an_unclosed_interpolation_mentions_the_backtick_cause() {
    let dir = project(&[("open.ol", "let x = `a ${ b`\nprintln(x)\n")]);
    let (out, rc) = run_in(&dir, &["check", "open.ol"]);
    assert_ne!(rc, 0);
    assert!(out.contains("Unclosed template interpolation"), "{out}");
    assert!(out.contains("templates do not nest"), "{out}");
}

#[test]
fn unrelated_parse_errors_do_not_gain_the_nesting_hint() {
    let dir = project(&[("plain.ol", "println(`ok ${ 1 + 1 }` +)\n")]);
    let (out, rc) = run_in(&dir, &["check", "plain.ol"]);
    assert_ne!(rc, 0);
    assert!(!out.contains("templates do not nest"), "{out}");
}

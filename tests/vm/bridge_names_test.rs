//! A main-file function reached through two bridges reads the program's
//! imported names. Since top-level functions capture nothing (top-level
//! names resolve at call time), a function the tier declines runs on the
//! bridge interpreter and finds the program's names through the host's
//! root. A bridge's own tier handed the next bridge the first bridge's
//! root instead, so a main-file function two bridges down — a page's
//! state shaper called from a served route, in open-track — missed every
//! imported name: "Undefined variable: as_str". Every tier.

use std::path::PathBuf;
use std::process::Command;

fn workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_bridge_names_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, text) in [
        (
            "pkg/olang.toml",
            "[package]\nname = \"q\"\nversion = \"0.1.0\"\n",
        ),
        ("pkg/index.ol", "share use lib.values { as_str }\n"),
        ("pkg/lib/values.ol", "share fn as_str(v) = to_string(v)\n"),
        (
            "olang.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nlock = \"never\"\n\n[dependencies.q]\npath = \"pkg\"\n",
        ),
        // a second `as_str`, so the name is not one function the tier
        // could take by name
        (
            "lib/core.ol",
            "share fn as_str(v) = \"<\" + to_string(v) + \">\"\n",
        ),
        (
            "lib/view.ol",
            "use lib.core { as_str }\nshare fn view(x) = as_str(x)\n",
        ),
        ("lib/pages.ol", "share fn apply_to(f, x) = f(x)\n"),
        (
            "main.ol",
            "use q { as_str }\nuse lib.view { view }\nuse lib.pages { apply_to }\n\
             fn shape(x) = as_str(x) + \"?\"\n\
             for i in 0..3 { let _ = view(i) }\n\
             // each lambda starts a task, which the tier declines: both run on a bridge\n\
             let inner = (x) => { let t = spawn 1; let _ = task.join(t); shape(x) }\n\
             let outer = (x) => { let t = spawn 2; let _ = task.join(t); apply_to(inner, x) }\n\
             println(apply_to(outer, \"m\"))\n\
             println(task.join(spawn apply_to(outer, \"w\")))\n",
        ),
    ] {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

#[test]
fn a_main_file_function_two_bridges_down_reads_the_programs_imports() {
    let ws = workspace();
    for mode in [&[][..], &["--ovm-tier=1"], &["--no-ovm"]] {
        let out = Command::new(env!("CARGO_BIN_EXE_olang"))
            .args(mode)
            .arg("main.ol")
            .current_dir(&ws)
            .env("OLANG_HOME", ws.join(".home"))
            .output()
            .expect("olang");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(text, "m?\nOk(\"w?\")\n", "{mode:?}");
    }
}

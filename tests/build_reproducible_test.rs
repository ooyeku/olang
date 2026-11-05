//! `olang build --app` is deterministic (OL-444): the same sources build
//! the same bytes under another OLANG_HOME and another checkout place.
//! The application's files are keyed by package-relative virtual paths,
//! its maps are sorted, and no tier profile goes in unless `--warm` names
//! one, so neither build machine's runs, home nor folders show.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// An application with a path dependency at `root` and a shelf library
/// kept in `home`.
fn project(root: &Path, home: &Path) -> PathBuf {
    let app = root.join("app");
    write(
        &app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ndep = { path = \"../dep\" }\nkit = { shelf = \"kit\" }\n",
    );
    write(
        &app.join("main.ol"),
        "use lib.greet { greet }\nuse dep { shout }\nuse kit { twice }\nfn spin(n) = { let mut a = 0; let mut i = 0; while i < n { a = (a + i) % 7919; i = i + 1 }; a }\nlet mut t = 0\nfor k in range(0, 200) { t = t + spin(2000) }\nprintln(twice(shout(greet(\"you\"))) + \" \" + to_string(t))\n",
    );
    write(&app.join("lib/greet.ol"), "share fn greet(w) = \"hi \" + w\n");
    write(
        &root.join("dep/olang.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nassets = [\"data\"]\n",
    );
    write(&root.join("dep/index.ol"), "share use lib.words { shout }\n");
    write(
        &root.join("dep/lib/words.ol"),
        "share fn shout(s) = str.to_upper(s) + \"!\"\n",
    );
    write(&root.join("dep/data/b.txt"), "b");
    write(&root.join("dep/data/a/c.txt"), "c");
    let kit = home.join("libs/kit");
    write(
        &kit.join("olang.toml"),
        "[package]\nname = \"kit\"\nversion = \"0.1.0\"\n",
    );
    write(&kit.join("index.ol"), "share fn twice(s) = s + s\n");
    write(
        &home.join("shelf.toml"),
        &format!("version = 1\n\n[libraries]\nkit = {:?}\n", kit.display().to_string()),
    );
    app
}

fn olang_in(app: &Path, home: &Path, args: &[&str]) -> String {
    let out = Command::new(olang())
        .args(args)
        .current_dir(app)
        .env("OLANG_HOME", home)
        .env_remove("OLANG_WARM_DIR")
        .env_remove("OLANG_SHELF")
        .output()
        .expect("run olang");
    assert!(
        out.status.success(),
        "olang {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn two_builds_of_the_same_sources_are_the_same_bytes() {
    let base = std::env::temp_dir().join(format!("olang_repro_{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let (root1, home1) = (base.join("one"), base.join("home-one"));
    let (root2, home2) = (base.join("two/a/much/deeper/place"), base.join("h2"));
    let app1 = project(&root1, &home1);
    let app2 = project(&root2, &home2);

    let mut outputs = Vec::new();
    for (app, home, name) in [(&app1, &home1, "one"), (&app2, &home2, "two")] {
        // a run first: it leaves a tier profile in the home, which a build
        // must not take on its own
        let ran = olang_in(app, home, &["main.ol"]);
        assert!(ran.starts_with("HI YOU!HI YOU!"), "{ran}");
        let payload = base.join(format!("{name}.olb"));
        let exe = base.join(format!("{name}-app"));
        olang_in(app, home, &["build", "--app", "main.ol", "--payload", payload.to_str().unwrap()]);
        olang_in(app, home, &["build", "--app", "main.ol", "-o", exe.to_str().unwrap()]);
        outputs.push((fs::read(&payload).unwrap(), fs::read(&exe).unwrap()));
    }
    let ((p1, e1), (p2, e2)) = (&outputs[0], &outputs[1]);
    assert!(p1 == p2, "the payloads differ");
    assert!(e1 == e2, "the executables differ");

    // nothing of either machine's places is in them
    let text = String::from_utf8_lossy(p1);
    for place in [&root1, &home1, &root2, &home2] {
        assert!(
            !text.contains(place.to_str().unwrap()),
            "the payload names {}",
            place.display()
        );
    }
    assert!(!text.contains("[[functions]]"), "no tier profile without --warm");

    // and the built application still runs with its sources gone
    fs::remove_dir_all(&root1).unwrap();
    fs::remove_dir_all(&home1).unwrap();
    let exe = base.join("one-app");
    #[cfg(target_os = "macos")]
    let _ = Command::new("codesign").args(["--force", "-s", "-"]).arg(&exe).output();
    let out = Command::new(&exe).current_dir(&base).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("HI YOU!HI YOU!"));
    let _ = fs::remove_dir_all(&base);
}

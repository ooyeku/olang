//! `olang build --app`: an application carries every module its entry
//! uses — its own package's and its dependencies' — and the assets the
//! packages declare, and runs where none of them exist any more.

use std::fs;
use std::process::Command;

#[test]
fn an_application_runs_after_its_sources_are_gone() {
    let ws = std::env::temp_dir().join(format!("olang_build_app_{}", std::process::id()));
    let _ = fs::remove_dir_all(&ws);
    let (app, dep) = (ws.join("app"), ws.join("dep"));
    fs::create_dir_all(app.join("lib")).unwrap();
    fs::create_dir_all(dep.join("lib")).unwrap();
    fs::create_dir_all(dep.join("data/nested")).unwrap();
    fs::write(
        dep.join("olang.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nassets = [\"data\"]\npublic = [\"lib.words\"]\n",
    )
    .unwrap();
    fs::write(dep.join("data/nested/greeting.txt"), "hello from an asset").unwrap();
    fs::write(
        dep.join("lib/words.ol"),
        "share fn shout(s) = str.to_upper(s) + \"!\"\n",
    )
    .unwrap();
    fs::write(dep.join("index.ol"), "share use lib.words { shout }\n").unwrap();
    fs::write(
        app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.dep]\npath = \"../dep\"\n",
    )
    .unwrap();
    fs::write(
        app.join("lib/greet.ol"),
        "use dep { shout }\nshare fn greet(who) = shout(\"hi \" + who)\n",
    )
    .unwrap();
    fs::write(
        app.join("main.ol"),
        "use lib.greet { greet }\nuse dep.lib.words { shout }\nprintln(greet(\"you\"))\nprintln(shout(unwrap(bytes.to_string(unwrap(asset.read(\"dep\", \"data/nested/greeting.txt\"))))))\nprintln(asset.exists(\"dep\", \"data/missing.txt\"))\n",
    )
    .unwrap();
    let exe = ws.join("built-app");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["build", "--app", "main.ol", "-o"])
        .arg(&exe)
        .current_dir(&app)
        .output()
        .expect("olang build");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // the sources and the assets are gone; the application has its own
    fs::remove_dir_all(&app).unwrap();
    fs::remove_dir_all(&dep).unwrap();
    let run = Command::new(&exe)
        .current_dir(&ws)
        .output()
        .expect("run the app");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(stdout.trim(), "HI YOU!\nHELLO FROM AN ASSET!\nfalse");
    // a file changed inside the application: it starts (each file is
    // checked as it is read, not the whole before), but the changed one is
    // not there; inspect --verify, which checks the whole, refuses it
    let mut bytes = fs::read(&exe).unwrap();
    let at = bytes
        .windows(19)
        .position(|w| w == b"hello from an asset")
        .expect("the asset in the binary");
    bytes[at] = b'j';
    let bad = ws.join("tampered-app");
    fs::write(&bad, &bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run2 = Command::new(&bad)
        .current_dir(&ws)
        .output()
        .expect("run the changed app");
    let out2 =
        String::from_utf8_lossy(&run2.stdout).to_string() + &String::from_utf8_lossy(&run2.stderr);
    assert!(out2.contains("HI YOU!"), "{out2}");
    assert!(!out2.contains("JELLO"), "{out2}");
    assert!(!run2.status.success(), "{out2}");
    let _ = fs::remove_dir_all(&ws);
}

/// OL-537: path dependencies named through a symbolic link (a worktree's
/// `../../libraries/dep`, the link beside it) lost their own `lib.*`
/// modules. Some of a dependency's files were loaded by the link's path
/// and some by the real one, and the latter were carried outside the
/// package: the application failed "Cannot find module 'lib.debug'"
/// anywhere but its source folder.
#[cfg(unix)]
#[test]
fn a_dependency_reached_through_a_symbolic_link_keeps_its_modules() {
    let ws = std::env::temp_dir().join(format!("olang_build_app_link_{}", std::process::id()));
    let _ = fs::remove_dir_all(&ws);
    let libs = ws.join("libraries");
    let write = |path: std::path::PathBuf, text: &str| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(
        libs.join("dep/olang.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[dependencies]\ndep2 = { path = \"../dep2\" }\n",
    );
    write(libs.join("dep/index.ol"), "share use lib.engine { run }\n");
    write(
        libs.join("dep/lib/engine.ol"),
        "use lib.debug { d }\nuse dep2 { two }\nshare fn run() = d() + two()\n",
    );
    write(libs.join("dep/lib/debug.ol"), "share fn d() = \"debug \"\n");
    write(
        libs.join("dep2/olang.toml"),
        "[package]\nname = \"dep2\"\nversion = \"0.1.0\"\n",
    );
    write(libs.join("dep2/index.ol"), "share use lib.engine { two }\n");
    write(libs.join("dep2/lib/engine.ol"), "share fn two() = \"two\"\n");
    // apps/<app>/.claude/libraries -> ../../../libraries, and the app in
    // a worktree two folders below it
    let claude = ws.join("apps/studio/.claude");
    fs::create_dir_all(claude.join("worktrees/w")).unwrap();
    std::os::unix::fs::symlink("../../../libraries", claude.join("libraries")).unwrap();
    let app = claude.join("worktrees/w");
    write(
        app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ndep = { path = \"../../libraries/dep\" }\ndep2 = { path = \"../../libraries/dep2\" }\n",
    );
    write(app.join("main.ol"), "use dep { run }\nprintln(run())\n");
    let exe = ws.join("built-app");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["build", "--app", "main.ol", "-o"])
        .arg(&exe)
        .current_dir(&app)
        .output()
        .expect("olang build");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // run where none of the sources are
    fs::remove_dir_all(&libs).unwrap();
    let run = Command::new(&exe)
        .current_dir(&ws)
        .output()
        .expect("run the application");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "debug two",
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = fs::remove_dir_all(&ws);
}

/// `asset.read` reads only inside the package, where the system takes
/// the path: a package can ship a link (`leak -> ~/.ssh/…`), and the
/// file it led to outside the package was answered. And only what the
/// package declares under `assets` (OL-619).
#[test]
#[cfg(unix)]
fn an_asset_link_out_of_its_package_is_refused() {
    let ws = std::env::temp_dir().join(format!("olang_asset_link_{}", std::process::id()));
    let _ = fs::remove_dir_all(&ws);
    let app = ws.join("app");
    fs::create_dir_all(app.join("data")).unwrap();
    fs::write(ws.join("secret.txt"), "outside").unwrap();
    fs::write(app.join("data/in.txt"), "inside").unwrap();
    std::os::unix::fs::symlink(ws.join("secret.txt"), app.join("data/leak")).unwrap();
    std::os::unix::fs::symlink("in.txt", app.join("data/near")).unwrap();
    fs::write(
        app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nassets = [\"data\"]\n",
    )
    .unwrap();
    fs::write(
        app.join("main.ol"),
        "fn text(r) = match r { Ok(b) => unwrap(bytes.to_string(b)), Err(e) => e }\nprintln(text(asset.read(\"app\", \"data/in.txt\")))\nprintln(text(asset.read(\"app\", \"data/near\")))\nprintln(text(asset.read(\"app\", \"data/leak\")))\nprintln(text(asset.read(\"app\", \"olang.toml\")))\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("main.ol")
        .current_dir(&app)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(lines[0], "inside");
    assert_eq!(lines[1], "inside");
    assert!(lines[2].contains("leads outside the package"), "{}", lines[2]);
    // OL-619: a file the package does not declare under `assets` is
    // refused from source, as a built application would not carry it.
    assert!(
        lines[3].contains("not one of package \"app\"'s assets") && lines[3].contains("[\"data\"]"),
        "{}",
        lines[3]
    );
    let _ = fs::remove_dir_all(&ws);
}

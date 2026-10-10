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
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nassets = [\"data\"]\n",
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
    let at = bytes.windows(19).position(|w| w == b"hello from an asset").expect("the asset in the binary");
    bytes[at] = b'j';
    let bad = ws.join("tampered-app");
    fs::write(&bad, &bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run2 = Command::new(&bad).current_dir(&ws).output().expect("run the changed app");
    let out2 = String::from_utf8_lossy(&run2.stdout).to_string() + &String::from_utf8_lossy(&run2.stderr);
    assert!(out2.contains("HI YOU!"), "{out2}");
    assert!(!out2.contains("JELLO"), "{out2}");
    assert!(!run2.status.success(), "{out2}");
    let _ = fs::remove_dir_all(&ws);
}

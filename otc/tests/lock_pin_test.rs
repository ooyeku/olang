//! A lock that pins a shelf library at a commit the shelf has moved past
//! still means that commit: its snapshot under `pins/` is what runs, a
//! missing one is made again from the library's repository, a tampered
//! one is refused by its checksum, and one that cannot be had is named
//! with both commits and the command that settles it. A dependency's own
//! lock pins its shelf dependencies the same way, and `otc doctor` says a
//! lock behind the shelf as information.
//!
//! Every test runs against a shelf of its own under the temp directory;
//! programs run in this process, which reads the shelf's location from
//! the environment, so the tests take turns.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

static SHELF_ENV: Mutex<()> = Mutex::new(());

fn otc(dir: &Path, home: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_otc"))
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", home)
        .env("OLANG_SHELF", home.join("shelf.toml"))
        .output()
        .expect("spawn otc");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Run `main.ol` of `project` the way `olang main.ol` does: a dependency
/// that does not resolve is an error naming it and why.
fn run_program(project: &Path) -> Result<String, String> {
    let (map, missing) =
        olang::pkg::install_lenient(project, &olang::pkg::InstallOptions::default());
    let source = std::fs::read_to_string(project.join("main.ol")).unwrap();
    let program = olang::parser::Parser::new().parse(&source).unwrap();
    let mut interp = olang::interpreter::Interpreter::new();
    interp.set_current_file(&project.join("main.ol"));
    interp.set_missing_dependencies(missing.into_iter().collect());
    interp.set_dependency_map(map.into_iter().collect());
    interp
        .eval_program(program)
        .map(|v| format!("{v}"))
        .map_err(|e| e.to_string())
}

/// A sandbox: a `greeter` library repository with two commits (A says
/// "one", B "two"), an `app` that depends on it by shelf name, and a
/// home whose shelf has had greeter shelved at A, the app installed (its
/// lock pins A), then greeter re-shelved at B.
struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    lib: PathBuf,
    app: PathBuf,
    a: String,
    b: String,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Sandbox {
    fn new(tag: &str) -> Option<Sandbox> {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("skipping: needs git");
            return None;
        }
        let guard = SHELF_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("otc_lock_pin_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, lib, app) = (root.join("home"), root.join("greeter"), root.join("app"));
        for d in [&home, &lib, &app] {
            std::fs::create_dir_all(d).unwrap();
        }
        unsafe { std::env::set_var("OLANG_SHELF", home.join("shelf.toml")) };
        std::fs::write(
            lib.join("olang.toml"),
            "[package]\nname = \"greeter\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(lib.join("index.ol"), "share fn greet() = \"one\"\n").unwrap();
        git(&lib, &["init", "-q", "."]);
        git(&lib, &["add", "."]);
        git(&lib, &["commit", "-qm", "one"]);
        let a = git(&lib, &["rev-parse", "HEAD"]);
        std::fs::write(lib.join("index.ol"), "share fn greet() = \"two\"\n").unwrap();
        git(&lib, &["commit", "-qam", "two"]);
        let b = git(&lib, &["rev-parse", "HEAD"]);
        std::fs::write(
            app.join("olang.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ngreeter = { shelf = \"greeter\" }\n",
        )
        .unwrap();
        std::fs::write(app.join("main.ol"), "use greeter { greet }\ngreet()\n").unwrap();
        let s = Sandbox {
            root,
            home,
            lib,
            app,
            a,
            b,
            _guard: guard,
        };
        let lib_path = s.lib.to_string_lossy().to_string();
        let (ok, out) = otc(&s.root, &s.home, &["lib", "add", &lib_path, "--rev", &s.a]);
        assert!(ok, "{out}");
        let (ok, out) = otc(&s.app, &s.home, &["install"]);
        assert!(ok, "{out}");
        let (ok, out) = otc(&s.root, &s.home, &["lib", "add", &lib_path, "--rev", &s.b]);
        assert!(ok, "{out}");
        assert!(
            s.lock().contains(&format!("rev = \"{}\"", s.a)),
            "{}",
            s.lock()
        );
        Some(s)
    }

    fn lock(&self) -> String {
        std::fs::read_to_string(self.app.join("olang.lock")).unwrap()
    }

    fn pin(&self, rev: &str) -> PathBuf {
        self.home
            .join("shelf/pins")
            .join(format!("greeter-{}", &rev[..12]))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        unsafe { std::env::remove_var("OLANG_SHELF") };
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_stale_lock_runs_the_commit_it_pins() {
    let Some(s) = Sandbox::new("stale") else {
        return;
    };
    let before = s.lock();
    assert!(s.pin(&s.a).is_dir() && s.pin(&s.b).is_dir());
    assert_eq!(run_program(&s.app).as_deref(), Ok("\"one\""));
    assert_eq!(s.lock(), before, "running never moves the lock");

    // `otc install` moves the lock to the shelf's pin, as it always has.
    let (ok, out) = otc(&s.app, &s.home, &["install"]);
    assert!(ok && out.contains("re-pinning"), "{out}");
    assert!(
        s.lock().contains(&format!("rev = \"{}\"", s.b)),
        "{}",
        s.lock()
    );
    assert_eq!(run_program(&s.app).as_deref(), Ok("\"two\""));
}

#[test]
fn a_missing_snapshot_is_made_again_from_the_repository() {
    let Some(s) = Sandbox::new("remake") else {
        return;
    };
    std::fs::remove_dir_all(s.pin(&s.a)).unwrap();
    assert_eq!(run_program(&s.app).as_deref(), Ok("\"one\""));
    assert!(s.pin(&s.a).join("index.ol").exists());
    // The shelf's own registration is left as it was.
    let shelf = std::fs::read_to_string(s.home.join("shelf.toml")).unwrap();
    assert!(shelf.contains(&format!("greeter = \"{}\"", s.b)), "{shelf}");
}

#[test]
fn a_missing_snapshot_with_no_repository_names_both_commits() {
    let Some(s) = Sandbox::new("norepo") else {
        return;
    };
    std::fs::remove_dir_all(s.pin(&s.a)).unwrap();
    std::fs::remove_dir_all(&s.lib).unwrap();
    let err = run_program(&s.app).unwrap_err();
    for want in [
        "dependency 'greeter'",
        &format!("pins shelf library 'greeter' at {}", &s.a[..12]),
        &format!("the shelf has it at {}", &s.b[..12]),
        "`otc install`",
    ] {
        assert!(err.contains(want), "missing {want:?} in: {err}");
    }
}

#[test]
fn a_snapshot_that_does_not_match_its_checksum_is_refused() {
    let Some(s) = Sandbox::new("tamper") else {
        return;
    };
    std::fs::write(
        s.pin(&s.a).join("index.ol"),
        "share fn greet() = \"evil\"\n",
    )
    .unwrap();
    let err = run_program(&s.app).unwrap_err();
    assert!(err.contains("checksum mismatch for 'greeter'"), "{err}");
}

#[test]
fn a_locked_rev_that_is_not_a_commit_id_names_no_directory() {
    let Some(s) = Sandbox::new("notarev") else {
        return;
    };
    // A committed lock is not trusted to point outside the shelf.
    let lock = s.lock().replace(&s.a, "../../greeter");
    std::fs::write(s.app.join("olang.lock"), lock).unwrap();
    let err = run_program(&s.app).unwrap_err();
    assert!(err.contains("is not a commit id"), "{err}");
}

#[test]
fn a_dependency_s_own_lock_pins_its_shelf_dependencies() {
    let Some(s) = Sandbox::new("transitive") else {
        return;
    };
    // `mid` depends on greeter by shelf name; its own lock pins A. The app
    // names only mid, and greeter's shelf pin is B.
    let mid = s.root.join("mid");
    std::fs::create_dir_all(&mid).unwrap();
    std::fs::write(
        mid.join("olang.toml"),
        "[package]\nname = \"mid\"\nversion = \"0.1.0\"\n\n[dependencies]\ngreeter = { shelf = \"greeter\" }\n",
    )
    .unwrap();
    std::fs::write(
        mid.join("index.ol"),
        "use greeter { greet }\nshare fn hello() = \"mid \" + greet()\n",
    )
    .unwrap();
    std::fs::write(mid.join("olang.lock"), s.lock()).unwrap();
    let (ok, out) = otc(&s.root, &s.home, &["lib", "add", &mid.to_string_lossy()]);
    assert!(ok, "{out}");
    std::fs::write(
        s.app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmid = { shelf = \"mid\" }\n",
    )
    .unwrap();
    std::fs::remove_file(s.app.join("olang.lock")).unwrap();
    std::fs::write(s.app.join("main.ol"), "use mid { hello }\nhello()\n").unwrap();
    assert_eq!(run_program(&s.app).as_deref(), Ok("\"mid one\""));
    // The project's lock pins mid; greeter is pinned through mid's lock.
    let lock = s.lock();
    assert!(
        lock.contains("[package.mid]") && !lock.contains("[package.greeter]"),
        "{lock}"
    );
    assert_eq!(
        run_program(&s.app).as_deref(),
        Ok("\"mid one\""),
        "replayed"
    );

    // mid's lock pins a snapshot whose content does not match: refused.
    std::fs::write(
        s.pin(&s.a).join("index.ol"),
        "share fn greet() = \"evil\"\n",
    )
    .unwrap();
    let err = run_program(&s.app).unwrap_err();
    assert!(
        err.contains("checksum mismatch for 'greeter'") && err.contains("mid"),
        "{err}"
    );
}

#[test]
fn doctor_says_a_lock_behind_the_shelf_as_information() {
    let Some(s) = Sandbox::new("doctor") else {
        return;
    };
    let (ok, out) = otc(&s.app, &s.home, &["doctor"]);
    assert!(ok, "{out}");
    let note = out
        .lines()
        .find(|l| l.contains("note: olang.lock has shelf library 'greeter'"))
        .unwrap_or_else(|| panic!("no note in:\n{out}"));
    assert!(
        note.contains(&format!("at {}", &s.a[..12]))
            && note.contains(&format!("the shelf has it at {}", &s.b[..12]))
            && note.contains("is what runs")
            && note.contains("otc install"),
        "{note}"
    );
    assert!(!note.contains('✗'), "{note}");

    // Moved to the shelf's pin, there is nothing to say.
    let (ok, out) = otc(&s.app, &s.home, &["install"]);
    assert!(ok, "{out}");
    let (_, out) = otc(&s.app, &s.home, &["doctor"]);
    assert!(!out.contains("note: olang.lock"), "{out}");
}

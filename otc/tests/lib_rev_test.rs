//! `otc lib add <path> --rev <commit>`: a shelved library pinned at a
//! commit is a snapshot the shelf owns, the commit travels in
//! `olang.lock`, and a shelf that holds the library at another commit —
//! or following its checkout — does not satisfy that lock.
//!
//! The first test runs a program, so it sets the shelf's location in
//! this process's environment; the second only drives `otc`, which is
//! handed the location explicitly.

use std::path::{Path, PathBuf};
use std::process::Command;

fn otc(dir: &Path, shelf: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_otc"))
        .args(args)
        .current_dir(dir)
        .env("OLANG_SHELF", shelf)
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

fn run_program(project: &Path) -> Result<String, String> {
    let (map, missing) =
        olang::pkg::install_lenient(project, &olang::pkg::InstallOptions::default());
    if let Some((name, why)) = missing.iter().find(|(name, _)| name == "greeter") {
        return Err(format!("{name}: {why}"));
    }
    let source = std::fs::read_to_string(project.join("main.ol")).unwrap();
    let program = olang::parser::Parser::new().parse(&source).unwrap();
    let mut interp = olang::interpreter::Interpreter::new();
    interp.set_current_file(&project.join("main.ol"));
    interp.set_dependency_map(map.into_iter().collect());
    interp
        .eval_program(program)
        .map(|v| format!("{v}"))
        .map_err(|e| e.to_string())
}

#[test]
fn a_shelved_library_pinned_at_a_commit_travels_in_the_lock() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: needs git");
        return;
    }
    let root: PathBuf = std::env::temp_dir().join(format!("otc_lib_rev_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (lib, app, shelf) = (
        root.join("greeter"),
        root.join("app"),
        root.join("home/shelf.toml"),
    );
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::create_dir_all(&app).unwrap();
    unsafe { std::env::set_var("OLANG_SHELF", &shelf) };

    std::fs::write(
        lib.join("olang.toml"),
        "[package]\nname = \"greeter\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(lib.join("index.ol"), "share fn greet() = \"one\"\n").unwrap();
    git(&lib, &["init", "-q", "."]);
    git(&lib, &["add", "."]);
    git(&lib, &["commit", "-qm", "one"]);
    let first = git(&lib, &["rev-parse", "HEAD"]);
    std::fs::write(lib.join("index.ol"), "share fn greet() = \"two\"\n").unwrap();
    git(&lib, &["commit", "-qam", "two"]);

    std::fs::write(
        app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ngreeter = { shelf = \"greeter\" }\n",
    )
    .unwrap();
    std::fs::write(app.join("main.ol"), "use greeter { greet }\ngreet()\n").unwrap();

    // Pinned at the first commit: the checkout says "two", the shelf "one".
    let lib_path = lib.to_string_lossy().to_string();
    let (ok, out) = otc(
        &root,
        &shelf,
        &["lib", "add", &lib_path, "--rev", &first[..10]],
    );
    assert!(ok && out.contains(&first[..12]), "{out}");
    let (_, listed) = otc(&root, &shelf, &["lib", "list"]);
    assert!(
        listed.contains(&format!("pinned at {}", &first[..12])),
        "{listed}"
    );
    assert_eq!(run_program(&app).as_deref(), Ok("\"one\""));
    let lock = std::fs::read_to_string(app.join("olang.lock")).unwrap();
    assert!(lock.contains(&format!("rev = \"{first}\"")), "{lock}");

    // The shelf drifts to the checkout: the lock is not satisfied, and the
    // program is told which commit to shelve instead of running "two".
    let (ok, _) = otc(&root, &shelf, &["lib", "add", &lib_path]);
    assert!(ok);
    let refused = run_program(&app).unwrap_err();
    assert!(
        refused.contains("pins shelf library 'greeter'") && refused.contains(&first[..12]),
        "{refused}"
    );

    // Shelved at that commit again, it runs; an unknown commit is refused.
    let (ok, _) = otc(&root, &shelf, &["lib", "add", &lib_path, "--rev", &first]);
    assert!(ok);
    assert_eq!(run_program(&app).as_deref(), Ok("\"one\""));
    let (ok, out) = otc(
        &root,
        &shelf,
        &["lib", "add", &lib_path, "--rev", "0000000"],
    );
    assert!(!ok && out.contains("has no commit"), "{out}");

    unsafe { std::env::remove_var("OLANG_SHELF") };
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_library_below_its_repository_root_is_pinned_from_its_own_subtree() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: needs git");
        return;
    }
    let root: PathBuf =
        std::env::temp_dir().join(format!("otc_lib_rev_sub_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (repo, shelf) = (root.join("mono"), root.join("home/shelf.toml"));
    let lib = repo.join("frameworks/sdk");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(repo.join("README.md"), "the repository's root\n").unwrap();
    std::fs::write(
        lib.join("olang.toml"),
        "[package]\nname = \"sdk\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(lib.join("index.ol"), "share fn hello() = \"sdk\"\n").unwrap();
    git(&repo, &["init", "-q", "."]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "one"]);
    let sha = git(&repo, &["rev-parse", "HEAD"]);

    let (ok, out) = otc(
        &root,
        &shelf,
        &["lib", "add", &lib.to_string_lossy(), "--rev", &sha[..10]],
    );
    assert!(ok, "{out}");
    let pinned = PathBuf::from(
        out.lines()
            .find_map(|l| l.split(" → ").nth(1))
            .expect("the shelved path is named")
            .trim(),
    );
    assert!(pinned.ends_with(format!("sdk-{}", &sha[..12])), "{out}");
    assert!(pinned.join("index.ol").exists(), "{out}");
    assert!(
        !pinned.join("README.md").exists(),
        "the snapshot is the library's subtree, not the repository"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn otc_install_repins_a_shelf_library_reshelved_at_another_commit() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: needs git");
        return;
    }
    let root: PathBuf = std::env::temp_dir().join(format!("otc_lib_repin_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (lib, app, shelf) = (
        root.join("greeter"),
        root.join("app"),
        root.join("home/shelf.toml"),
    );
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::create_dir_all(&app).unwrap();
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
    let lib_path = lib.to_string_lossy().to_string();
    let lock = || std::fs::read_to_string(app.join("olang.lock")).unwrap();

    // Pinned at A, installed: the lock carries A.
    let (ok, out) = otc(&root, &shelf, &["lib", "add", &lib_path, "--rev", &a]);
    assert!(ok, "{out}");
    let (ok, out) = otc(&app, &shelf, &["install"]);
    assert!(ok, "{out}");
    assert!(lock().contains(&format!("rev = \"{a}\"")), "{}", lock());

    // Re-shelved at B: --frozen refuses the drift and leaves the lock be;
    // a plain install re-pins to B and says so.
    let (ok, out) = otc(&root, &shelf, &["lib", "add", &lib_path, "--rev", &b]);
    assert!(ok, "{out}");
    let (ok, out) = otc(&app, &shelf, &["install", "--frozen"]);
    assert!(!ok, "--frozen must fail on a moved shelf pin: {out}");
    assert!(lock().contains(&format!("rev = \"{a}\"")), "{}", lock());
    let (ok, out) = otc(&app, &shelf, &["install"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("shelf library 'greeter' moved") && out.contains(&b[..12]),
        "{out}"
    );
    let text = lock();
    assert!(text.contains(&format!("rev = \"{b}\"")), "{text}");
    assert!(!text.contains(&a), "{text}");

    // Installing again is a replay: nothing moved, the lock is unchanged.
    let (ok, out) = otc(&app, &shelf, &["install"]);
    assert!(
        ok && out.contains("olang.lock unchanged") && !out.contains("moved"),
        "{out}"
    );

    // Unpinned (registered by directory): the lock drops its rev.
    let (ok, out) = otc(&root, &shelf, &["lib", "add", &lib_path]);
    assert!(ok, "{out}");
    let (ok, out) = otc(&app, &shelf, &["install"]);
    assert!(ok && out.contains("following its directory"), "{out}");
    let text = lock();
    assert!(
        text.contains("kind = \"shelf\"") && !text.contains("rev ="),
        "{text}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

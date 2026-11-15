//! The epic's "Done when" (OL-576), walked: a person new to the machine
//! sees every release and every local olang, switches between them in one
//! command, adds a library from GitHub and from a folder, keeps two
//! versions of it, upgrades and removes it — and at each step otc says
//! what it did and what is now in use. On a temporary home, with a local
//! fake of GitHub: no step reaches the network.
//!
//! One test: the steps build on each other, and the in-process program
//! runs read the shelf's location from this process's environment.

mod support;

use std::path::Path;
use support::*;

/// Run `main.ol` of `project` in this process, as `olang main.ol` would.
fn run_program(m: &Machine, project: &Path) -> String {
    unsafe {
        std::env::set_var("OLANG_HOME", &m.home);
        std::env::remove_var("OLANG_SHELF");
        std::env::remove_var("OLANG_CACHE");
        std::env::remove_var("OLANG_LOCK_READONLY");
        std::env::set_var("OLANG_GITHUB", &m.github);
    }
    let (map, missing) =
        olang::pkg::install_lenient(project, &olang::pkg::InstallOptions::default());
    assert!(missing.is_empty(), "unresolved: {missing:?}");
    let source = std::fs::read_to_string(project.join("main.ol")).unwrap();
    let program = olang::parser::Parser::new().parse(&source).unwrap();
    let mut interp = olang::interpreter::Interpreter::new();
    interp.set_current_file(&project.join("main.ol"));
    interp.set_dependency_map(map.into_iter().collect());
    interp
        .eval_program(program)
        .map(|v| format!("{v}").trim_matches('"').to_string())
        .unwrap_or_else(|e| panic!("{project:?}: {e}"))
}

fn project(m: &Machine, name: &str) -> std::path::PathBuf {
    m.ok(&m.dir, &["new", name]);
    let dir = m.dir.join(name);
    std::fs::write(dir.join("main.ol"), "use geometry\ngeometry.version()\n").unwrap();
    dir
}

#[test]
fn a_person_new_to_the_machine_can_do_it_all_without_the_docs() {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: needs git");
        return;
    }
    let scratch = std::env::temp_dir().join(format!("otc_done_gh_{}", std::process::id()));
    let _ = make_writable_and_remove(&scratch);
    let gh = FakeGitHub::start(&scratch);
    publish_releases(
        &gh,
        &[
            ("1.0.0-rc.8", "2026-11-10", true),
            ("1.0.0-rc.7", "2026-11-01", true),
            ("0.88.0", "2026-09-20", false),
            ("0.80.0", "2026-06-02", false),
        ],
    );
    publish_repo(
        &gh,
        "ooyeku/geometry",
        &[
            ("v1.1.0", "1.1.0", "geometry 1.1.0", ""),
            (
                "v1.2.0",
                "1.2.0",
                "geometry 1.2.0",
                "[capabilities]\nfs = \"read\"\n",
            ),
        ],
        true,
    );
    publish_repo(&gh, "ooyeku/notes", &[("v1.0.0", "1.0.0", "", "")], false);
    let m = Machine::new("done_when", Some(&gh));

    // ── every release, and every local olang ────────────────────────
    let out = m.ok(&m.dir, &["toolchain", "list", "--available"]);
    for (v, date) in [
        ("1.0.0-rc.8", "2026-11-10"),
        ("1.0.0-rc.7", "2026-11-01"),
        ("0.88.0", "2026-09-20"),
        ("0.80.0", "2026-06-02"),
    ] {
        assert!(
            out.lines().any(|l| l.contains(v) && l.contains(date)),
            "{v} {date} listed:\n{out}"
        );
    }
    assert!(
        out.contains("prerelease") && out.contains("stable"),
        "{out}"
    );
    let j = m
        .otc(
            &m.dir,
            &[
                "toolchain",
                "list",
                "--available",
                "--stable",
                "--format",
                "json",
            ],
        )
        .json();
    assert_eq!(j["kind"], "toolchains");
    assert!(
        j["available"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["prerelease"] == false),
        "{j}"
    );

    // ── switch in one command: `otc use rc` installs the newest rc ─────
    let out = m.ok(&m.dir, &["use", "rc"]);
    assert!(out.contains("installed olang 1.0.0-rc.8"), "{out}");
    assert!(
        out.contains("`olang --version` now answers olang 1.0.0-rc.8"),
        "{out}"
    );
    assert_eq!(m.olang_version(), "olang 1.0.0-rc.8");
    let out = m.ok(&m.dir, &["use", "0.88"]);
    assert!(out.contains("now answers olang 0.88.0"), "{out}");
    assert_eq!(m.olang_version(), "olang 0.88.0");
    // back and forth with nothing downloaded: no request reaches "GitHub"
    let before = gh.hits();
    let started = std::time::Instant::now();
    let out = m.ok(&m.dir, &["use", "-"]);
    assert!(out.contains("now answers olang 1.0.0-rc.8"), "{out}");
    m.ok(&m.dir, &["use", "0.88"]);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "switching took {:?}",
        started.elapsed()
    );
    let out = m.ok(&m.dir, &["toolchain", "list", "--available", "--offline"]);
    assert!(
        out.lines()
            .any(|l| l.contains("1.0.0-rc.8") && l.contains("installed")),
        "{out}"
    );
    assert_eq!(
        gh.hits(),
        before,
        "switching between installed olangs and listing offline used the network"
    );
    // a dry run changes nothing
    let out = m.ok(&m.dir, &["use", "0.80", "--dry-run"]);
    assert!(out.contains("would install olang 0.80.0"), "{out}");
    assert!(!m.home.join("toolchains/0.80.0").exists());

    // ── a local build is a toolchain of its own ─────────────────────
    let checkout = m.dir.join("olang-src");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::write(checkout.join("Cargo.toml"), "[package]\nname = \"olang\"\n").unwrap();
    git(&checkout, &["init", "-q", "-b", "main"]);
    git(&checkout, &["add", "-A"]);
    git(&checkout, &["commit", "-q", "-m", "release"]);
    git(&checkout, &["tag", "v1.0.0-rc.8"]);
    for i in 0..2 {
        std::fs::write(checkout.join("work.txt"), format!("{i}")).unwrap();
        git(&checkout, &["add", "-A"]);
        git(&checkout, &["commit", "-q", "-m", "more"]);
    }
    fake_olang(&checkout.join("target/release/olang"), "1.0.0-rc.8");
    let out = m.ok(&m.dir, &["toolchain", "link", checkout.to_str().unwrap()]);
    assert!(out.contains("linked 'main'"), "{out}");
    let out = m.ok(&m.dir, &["toolchain", "list"]);
    assert!(out.contains("(main, 2 commits past v1.0.0-rc.8)"), "{out}");
    // a new commit shows at once; a build older than it is called out
    std::fs::write(checkout.join("work.txt"), "later").unwrap();
    git(&checkout, &["add", "-A"]);
    let future = format!(
        "{} +0000",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 2 * 86_400
            + 600
    );
    support::git_env(
        &checkout,
        &["commit", "-q", "-m", "later"],
        &[("GIT_COMMITTER_DATE", future.as_str())],
    );
    let out = m.ok(&m.dir, &["toolchain", "list"]);
    assert!(out.contains("3 commits past v1.0.0-rc.8"), "{out}");
    assert!(out.contains("built 2 days before the last commit"), "{out}");
    let out = m.ok(&m.dir, &["use", "main"]);
    assert!(out.contains("now answers olang 1.0.0-rc.8"), "{out}");
    assert!(m.olang_version().contains("1.0.0-rc.8"));
    m.ok(&m.dir, &["use", "0.88"]);

    // ── a library from GitHub ───────────────────────────────────────
    let app = project(&m, "app");
    let out = m.ok(&app, &["add", "ooyeku/geometry@v1.1.0"]);
    assert!(
        out.contains("GitHub repository ooyeku/geometry"),
        "says what it understood: {out}"
    );
    assert!(
        out.contains("geometry asks for"),
        "shows the capabilities: {out}"
    );
    assert!(
        out.contains("geometry is now 1.1.0"),
        "says what is in use: {out}"
    );
    let lock = std::fs::read_to_string(app.join("olang.lock")).unwrap();
    assert!(
        lock.contains("github = \"ooyeku/geometry\"")
            && lock.contains("rev = ")
            && lock.contains("checksum = "),
        "{lock}"
    );
    assert_eq!(run_program(&m, &app), "geometry 1.1.0");
    // a repository that is not an olang project is refused in one line
    let out = m.fails(&app, &["add", "ooyeku/notes"]);
    assert!(out.contains("not an olang library"), "{out}");
    assert!(
        out.lines().filter(|l| l.starts_with("error:")).count() == 1 && out.contains("next: "),
        "{out}"
    );
    assert!(
        !m.home.join("shelf/notes").exists(),
        "nothing of it was kept"
    );

    // ── two versions side by side, and the rule that picks ──────────
    m.ok(&m.dir, &["shelf", "add", "ooyeku/geometry@v1.2.0"]);
    let local = m.dir.join("geometry-work");
    write_library(&local, "geometry", "1.3.0", "geometry local", "");
    let out = m.ok(&m.dir, &["shelf", "add", local.to_str().unwrap()]);
    assert!(out.contains("a folder"), "{out}");
    let out = m.ok(&m.dir, &["shelf", "list", "geometry"]);
    assert!(
        out.contains("1.1.0") && out.contains("1.2.0") && out.contains("local"),
        "{out}"
    );
    let fresh = project(&m, "fresh");
    m.ok(&fresh, &["add", "geometry"]);
    assert_eq!(run_program(&m, &fresh), "geometry local");
    assert!(
        m.ok(&fresh, &["list"])
            .contains("(local: the shelf's local folder)")
    );
    let old = project(&m, "old");
    olang::pkg::manifest::Manifest::write_dependency(
        &old,
        "geometry",
        &olang::pkg::manifest::Dependency::Registry("1.1".to_string()),
    )
    .unwrap();
    m.ok(&old, &["install"]);
    assert_eq!(run_program(&m, &old), "geometry 1.1.0");
    assert!(m.ok(&old, &["list"]).contains("1.1.0"), "asked");
    assert!(
        m.ok(&old, &["list"])
            .contains("asked: olang.toml asks for 1.1")
    );
    // the app's lock keeps its copy whatever the shelf holds now
    assert_eq!(run_program(&m, &app), "geometry 1.1.0");

    // ── upgrade ─────────────────────────────────────────────────────
    let out = m.ok(&app, &["outdated"]);
    assert!(
        out.lines().any(|l| l.starts_with("geometry")
            && l.contains("1.1.0")
            && l.contains("1.2.0")
            && l.contains("upgrade")),
        "{out}"
    );
    let lock_before = std::fs::read(app.join("olang.lock")).unwrap();
    let out = m.ok(&app, &["upgrade", "--dry-run"]);
    assert!(
        out.contains("would upgrade geometry: 1.1.0 → 1.2.0"),
        "{out}"
    );
    assert_eq!(
        std::fs::read(app.join("olang.lock")).unwrap(),
        lock_before,
        "a dry run writes nothing"
    );
    // 1.2.0 asks to read files: said, and not taken without --yes
    let out = m.fails(&app, &["upgrade"]);
    assert!(
        out.contains("new capabilit") && out.contains("next: otc upgrade --yes"),
        "{out}"
    );
    assert_eq!(
        std::fs::read(app.join("olang.lock")).unwrap(),
        lock_before,
        "a refused upgrade writes nothing"
    );
    let out = m.ok(&app, &["upgrade", "--yes"]);
    assert!(out.contains("upgraded geometry: 1.1.0 → 1.2.0"), "{out}");
    assert!(out.contains("geometry is now 1.2.0"), "{out}");
    assert_eq!(run_program(&m, &app), "geometry 1.2.0");

    // ── remove, saying what depends first ───────────────────────────
    let out = m.fails(&m.dir, &["shelf", "remove", "geometry@1.2.0"]);
    assert!(
        out.contains(&app.display().to_string()),
        "names the project: {out}"
    );
    assert!(
        out.contains("next: otc shelf remove geometry@1.2.0 --yes"),
        "{out}"
    );
    let out = m.ok(&app, &["remove", "geometry"]);
    assert!(out.contains("removed geometry"), "{out}");
    let out = m.ok(&m.dir, &["shelf", "prune"]);
    assert!(
        out.contains("would remove geometry 1.2.0"),
        "nothing uses it now: {out}"
    );
    assert!(
        !out.contains("would remove geometry 1.1.0"),
        "old/ still uses 1.1.0: {out}"
    );
    m.ok(&m.dir, &["shelf", "prune", "--yes"]);
    assert!(
        !m.ok(&m.dir, &["shelf", "list", "geometry"])
            .contains("1.2.0")
    );

    // ── one overview, as text and as data ───────────────────────────
    let out = m.ok(&old, &["status"]);
    assert!(
        out.contains("olang 0.88.0 runs here") && out.contains("geometry") && out.contains("next:"),
        "{out}"
    );
    let j = m.otc(&old, &["--format", "json"]).json();
    assert_eq!(j["kind"], "status");
    assert!(j["next"].as_array().unwrap().len() <= 3);
    let j = m
        .otc(&old, &["remove", "nothing", "--format", "json"])
        .json();
    assert_eq!(j["kind"], "error");
    assert!(j["next"].as_str().unwrap().starts_with("otc "), "{j}");

    m.cleanup();
    let _ = make_writable_and_remove(&scratch);
}

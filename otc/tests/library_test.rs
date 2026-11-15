//! Libraries and the shelf (OL-581 to OL-584) beyond the end-to-end walk:
//! a tag that moves, an install from the lock with GitHub gone, one
//! project needing two versions of a name, the old home layout moved by
//! `otc doctor --fix`, and the doctor's checks of the shelf.

mod support;

use support::*;

#[test]
fn a_moved_tag_is_refused_until_accepted_and_the_lock_needs_no_network() {
    let scratch = std::env::temp_dir().join(format!("otc_gh_lib_moved_{}", std::process::id()));
    let _ = make_writable_and_remove(&scratch);
    let gh = FakeGitHub::start(&scratch);
    let work = publish_repo(&gh, "me/shapes", &[("v1.0.0", "1.0.0", "one", "")], true);
    let m = Machine::new("lib_moved", Some(&gh));
    m.ok(&m.dir, &["new", "app"]);
    let app = m.dir.join("app");
    let out = m.ok(&app, &["add", "me/shapes"]);
    assert!(out.contains("shapes is now 1.0.0"), "{out}");
    let j = m.otc(&app, &["list", "--format", "json"]).json();
    assert_eq!(j["dependencies"][0]["rule"], "asked", "{j}");
    assert_eq!(j["dependencies"][0]["source"], "github", "{j}");

    // the tag is moved to another commit upstream
    write_library(&work, "shapes", "1.0.0", "one, moved", "");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "moved"]);
    git(&work, &["tag", "-f", "v1.0.0"]);
    republish(&gh, "me/shapes");
    let out = m.fails(&m.dir, &["shelf", "add", "me/shapes@v1.0.0"]);
    assert!(
        out.contains("has moved")
            && out.contains("next: otc shelf add me/shapes@v1.0.0 --accept-moved"),
        "{out}"
    );
    m.ok(
        &m.dir,
        &["shelf", "add", "me/shapes@v1.0.0", "--accept-moved"],
    );

    // GitHub gone: an install from the lock needs only the shelf
    let mut offline = Machine::new("lib_moved_offline", None);
    offline.home = m.home.clone();
    let out = offline.ok(&app, &["install", "--frozen"]);
    assert!(out.contains("olang.lock unchanged"), "{out}");
    m.cleanup();
    offline.cleanup();
    let _ = make_writable_and_remove(&scratch);
}

#[test]
fn one_project_gets_one_version_of_a_name() {
    let scratch = std::env::temp_dir().join(format!("otc_gh_lib_two_{}", std::process::id()));
    let _ = make_writable_and_remove(&scratch);
    let gh = FakeGitHub::start(&scratch);
    publish_repo(
        &gh,
        "me/geometry",
        &[("v1.1.0", "1.1.0", "a", ""), ("v2.0.0", "2.0.0", "b", "")],
        true,
    );
    let m = Machine::new("lib_two", Some(&gh));
    m.ok(&m.dir, &["shelf", "add", "me/geometry@v1.1.0"]);
    m.ok(&m.dir, &["shelf", "add", "me/geometry@v2.0.0"]);
    // a library that asks for geometry 1.1
    let lib = m.dir.join("drawing");
    write_library(&lib, "drawing", "0.1.0", "d", "");
    let manifest = std::fs::read_to_string(lib.join("olang.toml")).unwrap();
    std::fs::write(
        lib.join("olang.toml"),
        format!("{manifest}\n[dependencies]\ngeometry = \"1.1\"\n"),
    )
    .unwrap();
    m.ok(&m.dir, &["new", "app"]);
    let app = m.dir.join("app");
    m.ok(&app, &["add", "geometry@2"]);
    let out = m.fails(&app, &["add", lib.to_str().unwrap()]);
    assert!(
        out.contains("this project gets geometry 2.0.0")
            && out.contains("drawing asks for geometry 1.1"),
        "{out}"
    );
    assert!(
        !std::fs::read_to_string(app.join("olang.toml"))
            .unwrap()
            .contains("drawing"),
        "a failed add changes nothing"
    );
    // the shelf's default for a bare name, said and used
    let out = m.ok(&m.dir, &["shelf", "use", "geometry@1.1.0"]);
    assert!(
        out.contains("a bare `geometry` now gives 1.1.0 (default)"),
        "{out}"
    );
    m.ok(&m.dir, &["new", "other"]);
    let other = m.dir.join("other");
    m.ok(&other, &["add", "geometry"]);
    assert!(m.ok(&other, &["list"]).contains("1.1.0"), "default");
    m.cleanup();
    let _ = make_writable_and_remove(&scratch);
}

#[test]
fn doctor_fix_moves_the_old_layout_and_locks_keep_working() {
    let m = Machine::new("lib_migrate", None);
    // the old layout: a starter in shelf/<name>/, a snapshot in pins/, revs
    let home = &m.home;
    let starter = home.join("shelf/textkit");
    write_library(&starter, "textkit", "0.88.0", "old textkit", "");
    let pin = home.join("shelf/pins/sdk-4f2a91c03e1b");
    write_library(&pin, "sdk", "0.3.0", "sdk at a commit", "");
    let mine = m.dir.join("mine");
    write_library(&mine, "mine", "0.1.0", "mine", "");
    std::fs::write(
        home.join("shelf.toml"),
        format!(
            "version = 1\n\n[libraries]\ntextkit = \"{}\"\nsdk = \"{}\"\nmine = \"{}\"\n\n[revs]\nsdk = \"4f2a91c03e1b0000000000000000000000000000\"\n",
            starter.display(),
            pin.display(),
            mine.display()
        ),
    )
    .unwrap();
    // an old link folder: toolchains/<made-up>/bin/olang -> a checkout's build
    let checkout = m.dir.join("olang-src");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::write(checkout.join("Cargo.toml"), "[package]\nname = \"olang\"\n").unwrap();
    git(&checkout, &["init", "-q", "-b", "main"]);
    git(&checkout, &["add", "-A"]);
    git(&checkout, &["commit", "-q", "-m", "x"]);
    fake_olang(&checkout.join("target/release/olang"), "1.0.0-rc.8");
    std::fs::create_dir_all(home.join("toolchains/9.9.9/bin")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        checkout.join("target/release/olang"),
        home.join("toolchains/9.9.9/bin/olang"),
    )
    .unwrap();

    // refused, naming the fix
    let out = m.fails(&m.dir, &["shelf", "list"]);
    assert!(out.contains("next: otc doctor --fix"), "{out}");
    let out = m.ok(&m.dir, &["doctor"]);
    assert!(
        out.contains("old layout") && out.contains("old-style link"),
        "{out}"
    );
    let out = m.ok(&m.dir, &["doctor", "--fix"]);
    assert!(
        out.contains("moved the shelf") && out.contains("is now the linked build 'main'"),
        "{out}"
    );
    assert!(!home.join("shelf/pins").exists());
    assert!(
        home.join("shelf/textkit/0.88.0/index.ol").is_file(),
        "the starter is a copy"
    );
    assert!(
        home.join("shelf/sdk/0.3.0/index.ol").is_file(),
        "the snapshot is a copy"
    );
    let out = m.ok(&m.dir, &["shelf", "list"]);
    assert!(
        out.contains("sdk  gives 0.3.0 (default)") && out.contains("mine  gives the local folder"),
        "{out}"
    );
    assert!(m.ok(&m.dir, &["toolchain", "list"]).contains("(main)"));
    // a project whose lock pinned the old snapshot still resolves it
    m.ok(&m.dir, &["new", "app"]);
    let app = m.dir.join("app");
    m.ok(&app, &["add", "sdk"]);
    assert!(
        std::fs::read_to_string(app.join("olang.lock"))
            .unwrap()
            .contains("rev = \"4f2a91c03e1b")
    );
    // the doctor checks the shelf: a copy changed since fetched
    let copy = home.join("shelf/sdk/0.3.0/index.ol");
    let _ = std::process::Command::new("chmod")
        .arg("u+w")
        .arg(&copy)
        .status();
    let _ = std::process::Command::new("chmod")
        .arg("u+w")
        .arg(copy.parent().unwrap())
        .status();
    std::fs::write(&copy, "share fn version() = \"tampered\"\n").unwrap();
    std::fs::create_dir_all(home.join("shelf/stray/1.0.0")).unwrap();
    let out = m.ok(&m.dir, &["doctor"]);
    assert!(out.contains("was changed since it was fetched"), "{out}");
    assert!(
        out.contains("stray/1.0.0 is on the shelf's disk but recorded nowhere"),
        "{out}"
    );
    m.cleanup();
}

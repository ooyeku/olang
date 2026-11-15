//! olang versions (OL-578, OL-579, OL-580): the releases list and its
//! cache, `otc use` and what it accepts, remove and prune that name the
//! projects pinning a version, and linked local builds. Each test is its
//! own temporary home and its own fake GitHub; none reaches the network
//! except the last, and only with OLANG_NET_TESTS=1.

mod support;

use support::*;

fn releases(gh: &FakeGitHub) {
    publish_releases(
        gh,
        &[
            ("1.0.0-rc.8", "2026-11-10", true),
            ("1.0.0-rc.7", "2026-11-01", true),
            ("0.88.0", "2026-09-20", false),
            ("0.80.0", "2026-06-02", false),
        ],
    );
}

#[test]
fn the_releases_list_is_cached_and_works_offline() {
    let scratch = std::env::temp_dir().join(format!("otc_gh_tc_avail_{}", std::process::id()));
    let _ = make_writable_and_remove(&scratch);
    let gh = FakeGitHub::start(&scratch);
    releases(&gh);
    let m = Machine::new("tc_avail", Some(&gh));

    // offline before any fetch: said, with the command that fetches
    let out = m.fails(&m.dir, &["toolchain", "list", "--available", "--offline"]);
    assert!(
        out.contains("no releases have been fetched yet")
            && out.contains("next: otc toolchain list --available"),
        "{out}"
    );
    assert_eq!(gh.hits(), 0);

    let out = m.ok(&m.dir, &["toolchain", "list", "--available"]);
    assert!(out.contains("released (4 on GitHub)"), "{out}");
    let first = gh.hits();
    assert!(first > 0);
    // within the hour: the cache answers
    m.ok(&m.dir, &["toolchain", "list", "--available"]);
    assert_eq!(
        gh.hits(),
        first,
        "a second list within the hour asks GitHub again"
    );
    let out = m.ok(&m.dir, &["toolchain", "list", "--available", "--stable"]);
    assert!(
        !out.contains("1.0.0-rc.8") && out.contains("0.88.0"),
        "{out}"
    );
    // newest first, newer-than-what-runs marked (nothing runs: none marked)
    let j = m
        .otc(
            &m.dir,
            &["toolchain", "list", "--available", "--format", "json"],
        )
        .json();
    let versions: Vec<&str> = j["available"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["version"].as_str().unwrap())
        .collect();
    assert_eq!(versions, ["1.0.0-rc.8", "1.0.0-rc.7", "0.88.0", "0.80.0"]);

    // GitHub unreachable and the cache stale: said, and the cache shown
    let stale = m.home.join("cache/releases.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&stale).unwrap()).unwrap();
    v["fetched_at"] = 1.into();
    std::fs::write(&stale, v.to_string()).unwrap();
    let mut gone = Machine::new("tc_avail_offline", None);
    gone.home = m.home.clone();
    let out = gone.ok(&m.dir, &["toolchain", "list", "--available"]);
    assert!(
        out.contains("GitHub could not be reached") && out.contains("1.0.0-rc.8"),
        "{out}"
    );
    assert!(!out.contains("panicked"), "{out}");
    m.cleanup();
    let _ = make_writable_and_remove(&scratch);
}

#[test]
fn use_installs_switches_and_names_what_pins_a_version() {
    let scratch = std::env::temp_dir().join(format!("otc_gh_tc_use_{}", std::process::id()));
    let _ = make_writable_and_remove(&scratch);
    let gh = FakeGitHub::start(&scratch);
    releases(&gh);
    let m = Machine::new("tc_use", Some(&gh));

    let out = m.ok(&m.dir, &["use", "stable"]);
    assert!(out.contains("now answers olang 0.88.0"), "{out}");
    m.ok(&m.dir, &["use", "latest"]);
    assert_eq!(m.olang_version(), "olang 1.0.0-rc.8");
    let out = m.ok(&m.dir, &["toolchain", "add", "0.80"]);
    assert!(
        out.contains("installed olang 0.80.0") && out.contains("`otc use 0.80.0` switches to it"),
        "{out}"
    );
    assert_eq!(m.olang_version(), "olang 1.0.0-rc.8", "add does not switch");
    let out = m.fails(&m.dir, &["use", "banana"]);
    assert!(
        out.contains("next: otc toolchain list --available"),
        "{out}"
    );
    let out = m.fails(&m.dir, &["use", "2.0"]);
    assert!(out.contains("no release of olang answers '2.0'"), "{out}");

    // a project pins one: --project writes the manifest and the lock
    m.ok(&m.dir, &["new", "app"]);
    let app = m.dir.join("app");
    let out = m.ok(&app, &["use", "0.80", "--project"]);
    assert!(out.contains("now runs olang 0.80.0"), "{out}");
    assert!(
        std::fs::read_to_string(app.join("olang.toml"))
            .unwrap()
            .contains("olang = \"0.80.0\"")
    );
    assert!(
        std::fs::read_to_string(app.join("olang.lock"))
            .unwrap()
            .contains("olang = \"0.80.0\"")
    );
    assert!(
        m.ok(&app, &["toolchain", "which"])
            .contains("toolchains/0.80.0/bin/olang")
    );
    // a remove names the project, and needs --yes
    let out = m.fails(&m.dir, &["toolchain", "remove", "0.80.0"]);
    assert!(
        out.contains(&app.display().to_string())
            && out.contains("next: otc toolchain remove 0.80.0 --yes"),
        "{out}"
    );
    // the one in use is never removed
    let out = m.fails(&m.dir, &["toolchain", "remove", "1.0.0-rc.8"]);
    assert!(out.contains("the default, the one in use"), "{out}");
    // prune: says what first, keeps the default and the pinned
    let out = m.ok(&m.dir, &["toolchain", "prune"]);
    assert!(out.contains("would remove olang 0.88.0"), "{out}");
    assert!(
        out.contains("keeping olang 1.0.0-rc.8 (the default)")
            && out.contains("keeping olang 0.80.0 (pinned by"),
        "{out}"
    );
    assert!(
        m.home.join("toolchains/0.88.0").exists(),
        "prune without --yes removes nothing"
    );
    m.ok(&m.dir, &["toolchain", "prune", "--yes"]);
    assert!(!m.home.join("toolchains/0.88.0").exists());
    assert!(m.home.join("toolchains/0.80.0").exists());
    m.cleanup();
    let _ = make_writable_and_remove(&scratch);
}

#[test]
fn a_linked_build_is_named_tracked_and_kept_out_of_shared_locks() {
    let m = Machine::new("tc_link", None);
    let checkout = m.dir.join("olang");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::write(checkout.join("Cargo.toml"), "[package]\nname = \"olang\"\n").unwrap();
    git(&checkout, &["init", "-q", "-b", "work"]);
    git(&checkout, &["add", "-A"]);
    git(&checkout, &["commit", "-q", "-m", "one"]);
    // linked before it is built: allowed, and the list says it is missing
    m.ok(&m.dir, &["toolchain", "link", checkout.to_str().unwrap()]);
    let out = m.ok(&m.dir, &["toolchain", "list"]);
    assert!(
        out.contains("has no build") && out.contains("cargo build --release"),
        "{out}"
    );
    let out = m.fails(&m.dir, &["use", "work"]);
    assert!(out.contains("next: cargo build --release"), "{out}");
    fake_olang(&checkout.join("target/release/olang"), "1.0.0-rc.9");
    m.ok(&m.dir, &["use", "work"]);
    assert_eq!(m.olang_version(), "olang 1.0.0-rc.9");
    // a rebuild is seen at once: the shim points into the checkout
    fake_olang(&checkout.join("target/release/olang"), "1.0.0-rc.10");
    assert_eq!(m.olang_version(), "olang 1.0.0-rc.10");
    // a name that reads as a version is refused
    let out = m.fails(
        &m.dir,
        &[
            "toolchain",
            "link",
            checkout.to_str().unwrap(),
            "--name",
            "1.0",
        ],
    );
    assert!(out.contains("would read as a version"), "{out}");
    // a project pins it only in its lock, and only when asked
    m.ok(&m.dir, &["new", "app"]);
    let app = m.dir.join("app");
    let out = m.fails(&app, &["use", "work", "--project"]);
    assert!(
        out.contains("next: otc use work --project --local"),
        "{out}"
    );
    m.ok(&app, &["use", "work", "--project", "--local"]);
    assert!(
        std::fs::read_to_string(app.join("olang.lock"))
            .unwrap()
            .contains("olang = \"work\"")
    );
    assert!(
        !std::fs::read_to_string(app.join("olang.toml"))
            .unwrap()
            .contains("work")
    );
    assert!(
        m.ok(&app, &["toolchain", "which"])
            .contains("target/release/olang")
    );
    // the one in use is not unlinked; a moved checkout is marked missing
    let out = m.fails(&m.dir, &["toolchain", "unlink", "work"]);
    assert!(out.contains("the default, the one in use"), "{out}");
    std::fs::rename(&checkout, m.dir.join("moved")).unwrap();
    let out = m.ok(&m.dir, &["toolchain", "list"]);
    assert!(
        out.contains("is gone") && out.contains("otc toolchain link"),
        "{out}"
    );
    m.cleanup();
}

/// Against the real GitHub: run with OLANG_NET_TESTS=1.
#[test]
fn the_real_releases_list() {
    if std::env::var("OLANG_NET_TESTS").as_deref() != Ok("1") {
        eprintln!("skipping: set OLANG_NET_TESTS=1 to read olang's releases from GitHub");
        return;
    }
    let mut m = Machine::new("tc_net", None);
    m.github = "https://github.com".to_string();
    m.api = "https://api.github.com".to_string();
    let j = m
        .otc(
            &m.dir,
            &["toolchain", "list", "--available", "--format", "json"],
        )
        .json();
    let available = j["available"].as_array().expect("a list");
    assert!(
        available.iter().any(|r| r["version"]
            .as_str()
            .is_some_and(|v| v.starts_with("1.0.0-rc."))),
        "{j}"
    );
    m.cleanup();
}

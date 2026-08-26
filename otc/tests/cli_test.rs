//! The otc CLI, end to end: scaffold each shape, shelve a library, add
//! it by name from another project, and verify the manifest and
//! lockfile that result. These drive the real binary; the runtime side
//! of the same flows (a program importing a shelf dependency actually
//! runs) is covered in-process, since the olang crate is a dependency.
//!
//! Everything runs inside one temp tree with `OLANG_SHELF` pointed into
//! it, so the user's real shelf is never touched. One test rather than
//! several: the flows share the scaffolds, and the shelf env var is
//! process-global.

use std::path::{Path, PathBuf};
use std::process::Command;

fn otc() -> &'static str {
    env!("CARGO_BIN_EXE_otc")
}

struct Run {
    ok: bool,
    output: String,
}

fn run_in(dir: &Path, shelf: &Path, args: &[&str]) -> Run {
    let out = Command::new(otc())
        .args(args)
        .current_dir(dir)
        .env("OLANG_SHELF", shelf)
        .output()
        .expect("spawn otc");
    Run {
        ok: out.status.success(),
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    }
}

fn parses(path: &Path) {
    let source = std::fs::read_to_string(path).expect("read");
    olang::parser::Parser::new()
        .parse(&source)
        .unwrap_or_else(|e| panic!("{} does not parse: {}", path.display(), e));
}

/// Run an olang program in-process, resolving the project's
/// dependencies through the same install path the CLI uses — which is
/// exactly what makes this a test of the shelf: install() must resolve
/// the `{ shelf = ... }` entry for the import to exist at all.
fn run_olang(project: &Path, rel: &str) -> String {
    let map = olang::pkg::install(project, &olang::pkg::InstallOptions::default())
        .expect("install resolves the shelf dependency");
    let source = std::fs::read_to_string(project.join(rel)).expect("read program");
    let program = olang::parser::Parser::new().parse(&source).expect("parse");
    let mut interp = olang::interpreter::Interpreter::new();
    interp.set_current_file(&project.join(rel));
    interp.set_dependency_map(map.into_iter().collect());
    format!("{:?}", interp.eval_program(program).expect("program runs"))
}

#[test]
fn the_local_workflow_end_to_end() {
    let base = std::env::temp_dir().join(format!("otc_cli_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let shelf: PathBuf = base.join("shelf.toml");
    // The in-process runs (run_olang) resolve the shelf through the same
    // env var the spawned commands receive. SAFETY: single test in this
    // binary, no parallel access.
    unsafe { std::env::set_var("OLANG_SHELF", &shelf) };

    // ── scaffold all three shapes ──────────────────────────────────
    let r = run_in(&base, &shelf, &["new", "greetings", "--lib"]);
    assert!(r.ok, "new --lib: {}", r.output);
    parses(&base.join("greetings/index.ol"));
    parses(&base.join("greetings/lib/greet.ol"));

    let r = run_in(&base, &shelf, &["new", "myapp"]);
    assert!(r.ok, "new: {}", r.output);
    parses(&base.join("myapp/src/main.ol"));

    let r = run_in(&base, &shelf, &["new", "webby", "--web"]);
    assert!(r.ok, "new --web: {}", r.output);
    for rel in ["main.ol", "lib/router.ol", "static/app.ol"] {
        parses(&base.join("webby").join(rel));
    }
    for rel in [
        "static/index.html",
        "static/olang-dom.js",
        "static/style.css",
        "README.md",
    ] {
        assert!(
            base.join("webby").join(rel).exists(),
            "web scaffold missing {rel}"
        );
    }

    // Refusals: an existing directory, and both shapes at once.
    let r = run_in(&base, &shelf, &["new", "myapp"]);
    assert!(!r.ok, "existing dir must refuse");
    let r = run_in(&base, &shelf, &["new", "x", "--lib", "--web"]);
    assert!(!r.ok, "--lib --web must refuse: {}", r.output);

    // ── the shelf ──────────────────────────────────────────────────
    let r = run_in(&base, &shelf, &["lib", "add", "greetings"]);
    assert!(r.ok, "lib add: {}", r.output);
    assert!(r.output.contains("Shelved 'greetings'"), "{}", r.output);
    let r = run_in(&base, &shelf, &["lib", "list"]);
    assert!(r.output.contains("greetings"), "{}", r.output);

    // A directory that is not a library refuses with the fix named.
    let junk = base.join("junk");
    std::fs::create_dir_all(&junk).unwrap();
    let r = run_in(&base, &shelf, &["lib", "add", "junk"]);
    assert!(!r.ok);
    assert!(r.output.contains("index.ol"), "{}", r.output);

    // ── add by name, from the app ──────────────────────────────────
    let app = base.join("myapp");
    let r = run_in(&app, &shelf, &["add", "greetings"]);
    assert!(r.ok, "add by name: {}", r.output);
    assert!(r.output.contains("from your shelf"), "{}", r.output);
    let manifest = std::fs::read_to_string(app.join("olang.toml")).unwrap();
    assert!(
        manifest.contains("shelf = \"greetings\""),
        "manifest records the name, not a path: {manifest}"
    );
    let lock = std::fs::read_to_string(app.join("olang.lock")).unwrap();
    assert!(
        lock.contains("greetings") && lock.contains("checksum"),
        "lock pins the resolved dir with a checksum: {lock}"
    );

    // The dependency actually runs.
    std::fs::write(
        app.join("src/main.ol"),
        "use greetings { hello }\nlet out = hello(\"cli\")\nprintln(out)\n",
    )
    .unwrap();
    let got = run_olang(&app, "src/main.ol");
    assert!(got.contains("Unit") || !got.is_empty(), "ran: {got}");

    // A name not on the shelf refuses and lists what is.
    let r = run_in(&app, &shelf, &["add", "nonexistent"]);
    assert!(!r.ok);
    assert!(
        r.output.contains("on your shelf: greetings"),
        "{}",
        r.output
    );

    // Double-add refuses without --force.
    let r = run_in(&app, &shelf, &["add", "greetings"]);
    assert!(!r.ok, "double add must refuse: {}", r.output);

    // ── list, install --frozen, remove ─────────────────────────────
    let r = run_in(&app, &shelf, &["list"]);
    assert!(r.output.contains("shelf:greetings"), "{}", r.output);

    let r = run_in(&app, &shelf, &["install", "--frozen"]);
    assert!(r.ok, "frozen replay of a fresh lock: {}", r.output);

    let r = run_in(&app, &shelf, &["remove", "greetings"]);
    assert!(r.ok, "remove: {}", r.output);
    let manifest = std::fs::read_to_string(app.join("olang.toml")).unwrap();
    assert!(!manifest.contains("greetings"), "{manifest}");
    let r = run_in(&app, &shelf, &["remove", "greetings"]);
    assert!(!r.ok, "removing a non-dependency must refuse");

    // ── add by path, auto-init ─────────────────────────────────────
    let fresh = base.join("fresh");
    std::fs::create_dir_all(&fresh).unwrap();
    let r = run_in(&fresh, &shelf, &["add", "../greetings"]);
    assert!(r.ok, "add by path with auto-init: {}", r.output);
    assert!(r.output.contains("created one"), "{}", r.output);
    let manifest = std::fs::read_to_string(fresh.join("olang.toml")).unwrap();
    assert!(
        manifest.contains("path = \"../greetings\""),
        "path recorded relative to the root: {manifest}"
    );

    // ── shelf removal ──────────────────────────────────────────────
    let r = run_in(&base, &shelf, &["lib", "remove", "greetings"]);
    assert!(r.ok, "{}", r.output);
    let r = run_in(&base, &shelf, &["lib", "list"]);
    assert!(
        !r.output.contains("greetings"),
        "removed library must be gone: {}",
        r.output
    );
    // The starters seeded on first touch are still there — a user
    // removal never disturbs them.
    assert!(r.output.contains("textkit"), "{}", r.output);
    assert!(r.output.contains("(starter)"), "{}", r.output);

    // ── starters: remove deletes the shelf-owned copy; restore brings
    // it back and a project can use it ─────────────────────────────
    let r = run_in(&base, &shelf, &["lib", "remove", "markdown"]);
    assert!(r.ok, "{}", r.output);
    assert!(
        r.output.contains("restore"),
        "names the way back: {}",
        r.output
    );
    let r = run_in(&base, &shelf, &["lib", "list"]);
    assert!(!r.output.contains("markdown"), "{}", r.output);
    let r = run_in(&base, &shelf, &["lib", "restore", "markdown"]);
    assert!(r.ok, "restore: {}", r.output);
    let r = run_in(&base, &shelf, &["lib", "restore", "nonsense"]);
    assert!(!r.ok, "unknown starter must refuse");
    assert!(
        r.output.contains("textkit"),
        "lists the starters: {}",
        r.output
    );

    let r = run_in(&app, &shelf, &["add", "textkit"]);
    assert!(r.ok, "add starter by name: {}", r.output);
    std::fs::write(
        app.join("src/main.ol"),
        "use textkit { money }
println(money(-150))
",
    )
    .unwrap();
    let got = run_olang(&app, "src/main.ol");
    assert!(!got.is_empty(), "starter import runs: {got}");

    unsafe { std::env::remove_var("OLANG_SHELF") };
    let _ = std::fs::remove_dir_all(&base);
}

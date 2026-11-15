//! `olang doc` extraction: the doc-comment scanner that turns `//!` and
//! `///` conventions into a structured reference. Pure over source text.

use olang::tools::doc::{
    Item, PackageFiles, PublicModule, Stability, extract, public_surface, run, run_against,
    stable_breaks,
};
use std::path::Path;

fn item(kind: &str, name: &str, sig: &str, doc: &str, shared: bool) -> Item {
    Item {
        kind: kind.into(),
        name: name.into(),
        signature: sig.into(),
        doc: doc.into(),
        shared,
        stability: None,
    }
}

#[test]
fn module_note_and_documented_items() {
    let src = r#"//! greet — a tiny module.
//! Two lines of module note.

/// Greet someone by name.
share fn greet(name) = "Hello, " + name

/// A record type.
type Person = struct { name: String }

// not a doc comment — this helper is invisible to the reference
fn helper(x) = x + 1

/// Loud greeting.
/// Spans two lines.
share fn shout(name) = str.to_upper(greet(name))
"#;
    let doc = extract(src);
    assert_eq!(
        doc.module_doc,
        "greet — a tiny module.\nTwo lines of module note."
    );
    assert_eq!(
        doc.items,
        vec![
            item("fn", "greet", "greet(name)", "Greet someone by name.", true),
            item("type", "Person", "type Person", "A record type.", false),
            item(
                "fn",
                "shout",
                "shout(name)",
                "Loud greeting.\nSpans two lines.",
                true
            ),
        ]
    );
}

#[test]
fn undocumented_declarations_are_omitted() {
    // Only declarations preceded by /// appear — the doc generator
    // reports what is documented, and nothing else.
    let src = r#"
share fn public_but_undocumented() = 1

/// This one is documented.
fn documented() = 2
"#;
    let doc = extract(src);
    assert_eq!(doc.items.len(), 1);
    assert_eq!(doc.items[0].name, "documented");
    assert!(doc.module_doc.is_empty());
}

#[test]
fn a_blank_line_between_doc_and_decl_is_tolerated() {
    let src = "/// doc\n\nshare fn f() = 1\n";
    let doc = extract(src);
    assert_eq!(doc.items.len(), 1);
    assert_eq!(doc.items[0].doc, "doc");
}

// ── stability markers and a package's public surface (OL-469) ───────

#[test]
fn stability_markers_leave_the_doc_and_mark_the_item() {
    let src = r#"/// Area of a rectangle.
/// @stable
share fn area(w, h) = w * h

/// @experimental: the shape of the answer may change
/// Bounds of a shape.
share fn bounds(s) = s

/// Not marked.
/// @stablex is not a marker
share fn plain() = 1
"#;
    let doc = extract(src);
    assert_eq!(doc.items[0].stability, Some(Stability::Stable));
    assert_eq!(doc.items[0].doc, "Area of a rectangle.");
    assert_eq!(doc.items[1].stability, Some(Stability::Experimental));
    assert_eq!(
        doc.items[1].doc,
        "the shape of the answer may change\nBounds of a shape."
    );
    assert_eq!(doc.items[2].stability, None);
    assert!(doc.items[2].doc.contains("@stablex"));
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A package with a root that re-exports, a listed public module, and
/// internals that are `share`d within the package only.
fn geometry(root: &Path, area_sig: &str) {
    write(
        root,
        "olang.toml",
        "[package]\nname = \"geometry\"\nversion = \"0.1.0\"\npublic = [\"shapes.circle\"]\n",
    );
    write(
        root,
        "index.ol",
        "//! Plane geometry.\nshare use lib.area { area, perimeter as edge }\n\n/// The origin.\n/// @stable\nshare let ORIGIN = (0, 0)\n\n/// Root-private.\nfn helper() = 1\n",
    );
    write(
        root,
        "lib/area.ol",
        &format!(
            "/// Rectangle area.\n/// @stable\nshare fn {area_sig} = w * h\n\n/// Rectangle perimeter.\n/// @experimental\nshare fn perimeter(w, h) = 2 * (w + h)\n\n/// Internal to the package.\nshare fn scale(x) = x\n"
        ),
    );
    write(
        root,
        "shapes/circle.ol",
        "/// A circle's area.\nshare fn circle(r) = 3 * r * r\n",
    );
    write(
        root,
        "lib/internal.ol",
        "/// Never public.\nshare fn secret() = 1\n",
    );
}

fn surface(root: &Path) -> Vec<PublicModule> {
    let manifest = olang::pkg::manifest::Manifest::load(root).unwrap();
    public_surface(&manifest, &PackageFiles::on_disk(root))
}

fn names(m: &PublicModule) -> Vec<&str> {
    m.doc.items.iter().map(|i| i.name.as_str()).collect()
}

#[test]
fn a_packages_surface_is_its_root_its_listed_modules_and_their_re_exports() {
    let dir = tempfile::tempdir().unwrap();
    geometry(dir.path(), "area(w, h)");
    let s = surface(dir.path());
    assert_eq!(
        s.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["geometry", "geometry.shapes.circle"]
    );
    assert_eq!(s[0].doc.module_doc, "Plane geometry.");
    // the root's own share, then what it re-exports, under its alias;
    // not the root's private helper, not lib's unexported `scale`
    assert_eq!(names(&s[0]), ["ORIGIN", "area", "edge"]);
    let edge = &s[0].doc.items[2];
    assert_eq!(edge.signature, "edge(w, h)");
    assert_eq!(edge.stability, Some(Stability::Experimental));
    assert_eq!(names(&s[1]), ["circle"]);
}

#[test]
fn doc_of_a_package_root_documents_the_public_surface_only() {
    let dir = tempfile::tempdir().unwrap();
    geometry(dir.path(), "area(w, h)");
    let out = dir.path().join("API.md");
    assert_eq!(run(&[dir.path().to_path_buf()], Some(&out), true), 0);
    let md = std::fs::read_to_string(&out).unwrap();
    assert!(md.contains("### `area` · shared · stable"), "{md}");
    assert!(md.contains("### `edge` · shared · experimental"), "{md}");
    assert!(md.contains("## `geometry.shapes.circle`"), "{md}");
    for internal in ["secret", "scale", "helper", "`perimeter`"] {
        assert!(!md.contains(internal), "{internal} leaked: {md}");
    }

    let html = dir.path().join("api.html");
    assert_eq!(run(&[dir.path().to_path_buf()], Some(&html), false), 0);
    let page = std::fs::read_to_string(&html).unwrap();
    assert!(page.contains("<span class=\"badge stable\">stable</span>"));
    assert!(page.contains("<span class=\"badge experimental\">experimental</span>"));
}

#[test]
fn stable_breaks_names_each_removed_changed_or_unmarked_stable_item() {
    let before = tempfile::tempdir().unwrap();
    geometry(before.path(), "area(w, h)");
    let now = tempfile::tempdir().unwrap();
    geometry(now.path(), "area(w, h, unit)");
    // ORIGIN gone; `edge` (experimental) is free to change
    write(now.path(), "index.ol", "share use lib.area { area }\n");
    let breaks = stable_breaks(&surface(before.path()), &surface(now.path()), "0.1.0");
    assert_eq!(
        breaks,
        [
            "@stable value ORIGIN in geometry was removed since 0.1.0",
            "@stable fn area in geometry changed since 0.1.0: was `area(w, h)`, now `area(w, h, unit)`",
        ]
    );

    // the same signature, no longer marked
    let unmarked = tempfile::tempdir().unwrap();
    geometry(unmarked.path(), "area(w, h)");
    write(
        unmarked.path(),
        "lib/area.ol",
        "/// Rectangle area.\nshare fn area(w, h) = w * h\n",
    );
    let breaks = stable_breaks(&surface(before.path()), &surface(unmarked.path()), "0.1.0");
    assert_eq!(
        breaks,
        ["@stable fn area in geometry is no longer marked @stable (it was in 0.1.0)"]
    );
}

#[test]
fn check_against_a_folder_and_a_git_tag() {
    let old = tempfile::tempdir().unwrap();
    geometry(old.path(), "area(w, h)");
    let repo = tempfile::tempdir().unwrap();
    let pkg = repo.path().join("geometry");
    geometry(&pkg, "area(w, h)");
    // unchanged: nothing broken
    assert_eq!(run_against(&pkg, old.path().to_str().unwrap()), 0);

    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "0.1.0"]);
    git(&["tag", "v0.1.0"]);
    geometry(&pkg, "area(w, h, unit)");
    // `0.1.0` finds the tag `v0.1.0`; the package is below the repo's root
    assert_eq!(run_against(&pkg, "0.1.0"), 1);
    assert_eq!(run_against(&pkg.join("lib"), "v0.1.0"), 1);
    assert_eq!(run_against(&pkg, "no-such-version"), 2);
}

//! `olang doc` — generate an API reference from doc comments.
//!
//! The convention is source-level and needs no grammar support, because
//! a doc comment is already a valid olang comment: `//!` at the top of a
//! file documents the module, and `///` lines immediately above a
//! declaration document it. `olang doc` scans `.ol` files for those,
//! pairs each `///` block with the declaration that follows, and renders
//! a browsable, suite-themed HTML page (or Markdown with `--md`).
//!
//! A `///` line that is `@stable` or `@experimental` marks the item's
//! stability; doc renders it as a badge. Given a package's root (the
//! folder with its `olang.toml`), doc documents the package's public
//! surface only: the root module, the modules `[package] public` lists,
//! and what those re-export with `share use`, `share`d items alone.
//! `olang check --against <version>` compares that surface with an
//! earlier one and reports each `@stable` item removed or changed.

use std::path::{Path, PathBuf};

/// One documented declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: String,                 // "fn", "type", "error", "trait", "value"
    pub name: String,                 // the declared name
    pub signature: String,            // e.g. `h(tag, attrs, children)`
    pub doc: String,                  // the joined `///` lines
    pub shared: bool,                 // exported with `share`
    pub stability: Option<Stability>, // `/// @stable` or `/// @experimental`
}

/// An item's promise, from a `/// @stable` or `/// @experimental` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stability {
    Stable,
    Experimental,
}

impl Stability {
    pub fn as_str(self) -> &'static str {
        match self {
            Stability::Stable => "stable",
            Stability::Experimental => "experimental",
        }
    }
}

/// The marker a doc line carries: `@stable` or `@experimental` as the
/// line's first word, and the rest of the line (a note, kept as doc).
fn marker(line: &str) -> Option<(Stability, &str)> {
    let t = line.trim();
    for (word, s) in [
        ("@stable", Stability::Stable),
        ("@experimental", Stability::Experimental),
    ] {
        if let Some(rest) = t.strip_prefix(word)
            && (rest.is_empty() || rest.starts_with([' ', ':', '\t']))
        {
            return Some((s, rest.trim_start_matches(':').trim()));
        }
    }
    None
}

/// A `///` block's text and stability: the first marker line sets it and
/// leaves the block (its note, if any, stays as text).
fn doc_and_stability(lines: &[String]) -> (String, Option<Stability>) {
    let mut stability = None;
    let mut kept: Vec<&str> = Vec::new();
    for line in lines {
        match marker(line) {
            Some((s, note)) => {
                stability.get_or_insert(s);
                if !note.is_empty() {
                    kept.push(note);
                }
            }
            None => kept.push(line),
        }
    }
    (kept.join("\n").trim().to_string(), stability)
}

/// A file's documentation: its `//!` module note and its documented items.
#[derive(Debug, PartialEq)]
pub struct ModuleDoc {
    pub module_doc: String,
    pub items: Vec<Item>,
}

/// Parse a declaration line into `(kind, name, signature, shared)`, or
/// `None` when the line does not open a declaration.
fn parse_decl(line: &str) -> Option<(String, String, String, bool)> {
    let (shared, rest) = match line.strip_prefix("share ") {
        Some(r) => (true, r.trim_start()),
        None => (false, line),
    };
    if let Some(r) = rest.strip_prefix("fn ") {
        let sig = r.split(" =").next().unwrap_or(r).trim().to_string();
        let name = sig.split('(').next().unwrap_or(&sig).trim().to_string();
        if name.is_empty() {
            return None;
        }
        return Some(("fn".into(), name, sig, shared));
    }
    if let Some(r) = rest.strip_prefix("type ") {
        let name: String = r
            .split([' ', '=', '<'])
            .next()
            .unwrap_or(r)
            .trim()
            .to_string();
        return Some((
            "type".into(),
            name.clone(),
            format!("type {}", name),
            shared,
        ));
    }
    if let Some(r) = rest.strip_prefix("error ") {
        let name: String = r.split([' ', '{']).next().unwrap_or(r).trim().to_string();
        return Some((
            "error".into(),
            name.clone(),
            format!("error {}", name),
            shared,
        ));
    }
    if let Some(r) = rest.strip_prefix("trait ") {
        let name: String = r.split([' ', '{']).next().unwrap_or(r).trim().to_string();
        return Some((
            "trait".into(),
            name.clone(),
            format!("trait {}", name),
            shared,
        ));
    }
    if let Some(r) = rest.strip_prefix("let ") {
        let name: String = r.split([' ', '=']).next().unwrap_or(r).trim().to_string();
        return Some(("value".into(), name.clone(), name, shared));
    }
    None
}

/// Extract the module note and documented items from one file's source.
pub fn extract(source: &str) -> ModuleDoc {
    let mut module_doc = String::new();
    let mut items = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut seen_code = false;

    for raw in source.lines() {
        let line = raw.trim_start();
        if let Some(rest) = line.strip_prefix("//!") {
            // Module note — only the leading block, before any code.
            if !seen_code {
                module_doc.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                module_doc.push('\n');
            }
        } else if let Some(rest) = line.strip_prefix("///") {
            pending.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        } else if line.is_empty() {
            // A blank line is tolerated between a doc block and its
            // declaration; it does not orphan the pending doc.
        } else if let Some((kind, name, signature, shared)) = parse_decl(line) {
            seen_code = true;
            if !pending.is_empty() {
                let (doc, stability) = doc_and_stability(&pending);
                items.push(Item {
                    kind,
                    name,
                    signature,
                    doc,
                    shared,
                    stability,
                });
            }
            pending.clear();
        } else {
            // Any other code line ends an unclaimed doc block.
            seen_code = true;
            pending.clear();
        }
    }
    ModuleDoc {
        module_doc: module_doc.trim().to_string(),
        items,
    }
}

/// The declaration line for `name`, documented or not — the signature
/// fallback `:help` uses when a loaded function carries no `///` block.
pub fn declaration_of(source: &str, name: &str) -> Option<Item> {
    for line in source.lines() {
        if let Some((kind, n, signature, shared)) = parse_decl(line.trim_start())
            && n == name
        {
            return Some(Item {
                kind,
                name: n,
                signature,
                doc: String::new(),
                shared,
                stability: None,
            });
        }
    }
    None
}

fn module_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

// ── a package's public surface ─────────────────────────────────────

/// One public module of a package: its dotted name (`geometry`,
/// `geometry.shapes.circle`), its file relative to the package, and its
/// documented public items.
#[derive(Debug, PartialEq)]
pub struct PublicModule {
    pub name: String,
    pub file: String,
    pub doc: ModuleDoc,
}

/// Reads a package file by its path relative to the package's root.
pub type ReadFile<'a> = Box<dyn Fn(&str) -> Option<String> + 'a>;

/// A package's `.ol` files as paths relative to its root (`/`-separated)
/// and a reader of them: the folder on disk, or the package at a git
/// revision for `check --against`.
pub struct PackageFiles<'a> {
    pub files: Vec<String>,
    pub read: ReadFile<'a>,
}

impl<'a> PackageFiles<'a> {
    /// The package in `root` on disk.
    pub fn on_disk(root: &'a Path) -> Self {
        let mut files: Vec<String> = super::discover_ol_files(root)
            .iter()
            .filter_map(|f| f.strip_prefix(root).ok())
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .collect();
        files.sort();
        PackageFiles {
            files,
            read: Box::new(move |rel| std::fs::read_to_string(root.join(rel)).ok()),
        }
    }
}

/// The dotted module a package file is (`lib/area.ol` is `lib.area`,
/// `shapes/index.ol` and `src/shapes.ol` are `shapes`).
fn module_of_file(rel: &str) -> String {
    let rel = rel.strip_prefix("src/").unwrap_or(rel);
    let rel = rel.strip_suffix(".ol").unwrap_or(rel);
    let rel = rel
        .strip_suffix("/index")
        .or_else(|| rel.strip_suffix("/mod"))
        .unwrap_or(rel);
    rel.replace('/', ".")
}

/// The root module's file: the first of `index.ol`, `mod.ol`,
/// `<name>.ol`, `src/index.ol` there is, as `use <name>` resolves it.
fn root_file(name: &str, files: &[String]) -> Option<String> {
    [
        "index.ol".to_string(),
        "mod.ol".to_string(),
        format!("{name}.ol"),
        "src/index.ol".to_string(),
    ]
    .into_iter()
    .find(|c| files.contains(c))
}

/// The file a `use` path names from `from` (a file of the package): next
/// to it, then from the package's root, the package's own name allowed
/// as the path's first segment.
fn resolve_use(path: &str, from: &str, package: &str, files: &[String]) -> Option<String> {
    let dir = match from.rfind('/') {
        Some(i) => &from[..=i],
        None => "",
    };
    let own = path
        .strip_prefix(package)
        .and_then(|r| r.strip_prefix('.'))
        .unwrap_or(path);
    for (base, p) in [(dir.to_string(), path), (String::new(), own)] {
        let sub = p.replace('.', "/");
        for c in [
            format!("{base}{sub}.ol"),
            format!("{base}{sub}/index.ol"),
            format!("{base}{sub}/mod.ol"),
            format!("{base}src/{sub}.ol"),
        ] {
            if files.contains(&c) {
                return Some(c);
            }
        }
    }
    None
}

/// The `share use path { a, b as c }` re-exports in a module's source,
/// as `(path, [(name, exported as)])`. A list may span lines.
fn re_exports(source: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out = Vec::new();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.trim_start().strip_prefix("share use ") else {
            continue;
        };
        let mut text = rest.to_string();
        if text.contains('{') {
            while !text.contains('}') {
                match lines.next() {
                    Some(more) => {
                        text.push(' ');
                        text.push_str(more.trim());
                    }
                    None => break,
                }
            }
        }
        let Some((path, list)) = text.split_once('{') else {
            continue;
        };
        let list = list.split('}').next().unwrap_or("");
        let names = list
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| match n.split_once(" as ") {
                Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
                None => (n.to_string(), n.to_string()),
            })
            .collect();
        out.push((path.trim().to_string(), names));
    }
    out
}

/// A package's public surface, documented: its root module, the modules
/// its `[package] public` lists, each with its `share`d documented items
/// and those it re-exports with `share use` (under the name it gives
/// them, with the doc and stability they carry where they are written).
pub fn public_surface(
    manifest: &crate::pkg::manifest::Manifest,
    pkg: &PackageFiles,
) -> Vec<PublicModule> {
    let name = manifest.package.name.as_str();
    let mut chosen: Vec<(String, String)> = Vec::new();
    if let Some(root) = root_file(name, &pkg.files) {
        chosen.push((name.to_string(), root));
    }
    for f in &pkg.files {
        let module = module_of_file(f);
        if chosen.iter().any(|(_, file)| file == f) || !manifest.lists_public(&module) {
            continue;
        }
        let dotted = format!("{name}.{module}");
        if !chosen.iter().any(|(m, _)| *m == dotted) {
            chosen.push((dotted, f.clone()));
        }
    }
    let mut out = Vec::new();
    for (module, file) in chosen {
        let Some(source) = (pkg.read)(&file) else {
            continue;
        };
        let full = extract(&source);
        let mut items: Vec<Item> = full.items.into_iter().filter(|i| i.shared).collect();
        for (path, names) in re_exports(&source) {
            let Some(target) = resolve_use(&path, &file, name, &pkg.files) else {
                continue;
            };
            let Some(target_src) = (pkg.read)(&target) else {
                continue;
            };
            let there = extract(&target_src);
            for (orig, alias) in names {
                if let Some(found) = there.items.iter().find(|i| i.name == orig && i.shared) {
                    let mut item = found.clone();
                    if alias != orig {
                        if item.signature.starts_with(&orig) {
                            item.signature = format!("{alias}{}", &item.signature[orig.len()..]);
                        } else {
                            item.signature = item.signature.replace(&orig, &alias);
                        }
                        item.name = alias;
                    }
                    items.retain(|i| i.name != item.name);
                    items.push(item);
                }
            }
        }
        out.push(PublicModule {
            name: module,
            file,
            doc: ModuleDoc {
                module_doc: full.module_doc,
                items,
            },
        });
    }
    out
}

/// The package whose root is `dir` (it has an `olang.toml`), loaded.
fn package_at(dir: &Path) -> Option<crate::pkg::manifest::Manifest> {
    if !dir.join("olang.toml").is_file() {
        return None;
    }
    crate::pkg::manifest::Manifest::load(dir).ok()
}

/// `olang doc [paths] [-o out.html] [--md]`.
/// `output`: the file to write; `None` writes HTML to `doc.html` and
/// Markdown to stdout. A path that is a package's root documents the
/// package's public surface; any other path, every documented
/// declaration of its files.
pub fn run(paths: &[PathBuf], output: Option<&Path>, markdown: bool) -> i32 {
    let mut modules: Vec<(String, ModuleDoc)> = Vec::new();
    let mut files = Vec::new();
    for p in paths {
        if let Some(manifest) = package_at(p) {
            let pkg = PackageFiles::on_disk(p);
            for m in public_surface(&manifest, &pkg) {
                if !(m.doc.items.is_empty() && m.doc.module_doc.is_empty()) {
                    modules.push((m.name, m.doc));
                }
            }
        } else {
            files.extend(super::discover_ol_files(p));
        }
    }
    files.sort();
    files.dedup();

    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let doc = extract(&src);
        if doc.items.is_empty() && doc.module_doc.is_empty() {
            continue;
        }
        modules.push((module_name(f), doc));
    }

    if modules.is_empty() {
        eprintln!("olang doc: nothing to document — add /// comments above declarations");
        return 1;
    }

    let item_count: usize = modules.iter().map(|(_, d)| d.items.len()).sum();
    if markdown && output.is_none() {
        print!("{}", render_markdown(&modules));
    } else {
        let output = output.unwrap_or(Path::new("doc.html"));
        let html = if markdown {
            render_markdown(&modules)
        } else {
            render_html(&modules, "olang reference")
        };
        if let Err(e) = std::fs::write(output, html) {
            eprintln!("olang doc: cannot write {}: {}", output.display(), e);
            return 1;
        }
        println!(
            "wrote {} — {} item(s) across {} module(s)",
            output.display(),
            item_count,
            modules.len()
        );
    }
    0
}

// ── olang check --against ──────────────────────────────────────────

/// What an earlier version's `@stable` items lost in `now`, one line
/// each: an item no public module has any more, one whose signature
/// changed, one no longer marked `@stable`. `version` names the earlier
/// version in the lines.
pub fn stable_breaks(before: &[PublicModule], now: &[PublicModule], version: &str) -> Vec<String> {
    let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = Vec::new();
    for m in before {
        for old in m
            .doc
            .items
            .iter()
            .filter(|i| i.stability == Some(Stability::Stable))
        {
            let found = now
                .iter()
                .find(|n| n.name == m.name)
                .and_then(|n| n.doc.items.iter().find(|i| i.name == old.name));
            let what = format!("@stable {} {} in {}", old.kind, old.name, m.name);
            match found {
                None => out.push(format!("{what} was removed since {version}")),
                Some(new)
                    if squash(&new.signature) != squash(&old.signature) || new.kind != old.kind =>
                {
                    out.push(format!(
                        "{what} changed since {version}: was `{}`, now `{}`",
                        old.signature, new.signature
                    ))
                }
                Some(new) if new.stability != Some(Stability::Stable) => out.push(format!(
                    "{what} is no longer marked @stable (it was in {version})"
                )),
                Some(_) => {}
            }
        }
    }
    out
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The package in `root` as it was at `version`: a folder holding that
/// version (its own `olang.toml`), or a git revision of the package's
/// repository, `version` itself or `v<version>` (a tag `v1.2.0`).
fn package_then<'a>(
    root: &'a Path,
    version: &'a str,
) -> Result<(crate::pkg::manifest::Manifest, PackageFiles<'a>), String> {
    let as_dir = Path::new(version);
    if as_dir.is_dir() {
        let manifest = crate::pkg::manifest::Manifest::load(as_dir)
            .map_err(|e| format!("{version} has no package: {e}"))?;
        return Ok((manifest, PackageFiles::on_disk(as_dir)));
    }
    let rev = [version.to_string(), format!("v{version}")]
        .into_iter()
        .find(|c| git(root, &["rev-parse", "--verify", "--quiet", &format!("{c}^{{commit}}")]).is_some())
        .ok_or_else(|| {
            format!("{version} is neither a folder nor a git revision (tried `{version}` and `v{version}`) of the package's repository")
        })?;
    let listed = git(root, &["ls-tree", "-r", "--name-only", &rev, "--", "."]).unwrap_or_default();
    let files = listed
        .lines()
        .filter(|l| l.ends_with(".ol"))
        .map(str::to_string)
        .collect();
    let show = move |rel: &str| git(root, &["show", &format!("{rev}:./{rel}")]);
    let toml =
        show("olang.toml").ok_or_else(|| format!("the package has no olang.toml at {version}"))?;
    let manifest = crate::pkg::manifest::Manifest::from_toml(&toml)
        .map_err(|e| format!("olang.toml at {version}: {e}"))?;
    Ok((
        manifest,
        PackageFiles {
            files,
            read: Box::new(show),
        },
    ))
}

/// `olang check --against <version>`: the package `start` is in, its
/// public surface now against the one at `version`. Each `@stable` item
/// removed or changed is an error. 0 when none is, 1 when one is, 2 when
/// either surface cannot be read.
pub fn run_against(start: &Path, version: &str) -> i32 {
    let abs = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    let Some(root) = crate::pkg::manifest::Manifest::find_root(&abs) else {
        eprintln!(
            "olang check: --against needs a package: no olang.toml at or above {}",
            start.display()
        );
        return 2;
    };
    let manifest = match crate::pkg::manifest::Manifest::load(&root) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("olang check: {}: {e}", root.join("olang.toml").display());
            return 2;
        }
    };
    let (then_manifest, then_files) = match package_then(&root, version) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("olang check: --against {e}");
            return 2;
        }
    };
    let now = public_surface(&manifest, &PackageFiles::on_disk(&root));
    let before = public_surface(&then_manifest, &then_files);
    let stable: usize = before
        .iter()
        .map(|m| {
            m.doc
                .items
                .iter()
                .filter(|i| i.stability == Some(Stability::Stable))
                .count()
        })
        .sum();
    let breaks = stable_breaks(&before, &now, version);
    for b in &breaks {
        eprintln!("{}: error: {b}", root.join("olang.toml").display());
    }
    if breaks.is_empty() {
        println!(
            "olang check: the {stable} @stable item{} of {version} still stand",
            if stable == 1 { "" } else { "s" }
        );
        0
    } else {
        eprintln!(
            "olang check: {} @stable item{} broken since {version}",
            breaks.len(),
            if breaks.len() == 1 { "" } else { "s" }
        );
        1
    }
}

// ── rendering ───────────────────────────────────────────────────────
fn render_markdown(modules: &[(String, ModuleDoc)]) -> String {
    let mut out = String::from("# API Reference\n\n");
    for (name, doc) in modules {
        out.push_str(&format!("## `{}`\n\n", name));
        if !doc.module_doc.is_empty() {
            out.push_str(&doc.module_doc);
            out.push_str("\n\n");
        }
        for item in &doc.items {
            let mut badge = String::from(if item.shared { " · shared" } else { "" });
            if let Some(s) = item.stability {
                badge.push_str(" · ");
                badge.push_str(s.as_str());
            }
            out.push_str(&format!(
                "### `{}`{}\n\n`{}`\n\n",
                item.name, badge, item.signature
            ));
            if !item.doc.is_empty() {
                out.push_str(&item.doc);
                out.push_str("\n\n");
            }
        }
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Minimal inline formatting: escape, then render `code` spans.
fn inline(s: &str) -> String {
    let mut out = String::new();
    let mut code = false;
    for ch in esc(s).chars() {
        if ch == '`' {
            out.push_str(if code { "</code>" } else { "<code>" });
            code = !code;
        } else {
            out.push(ch);
        }
    }
    if code {
        out.push_str("</code>");
    }
    out
}

/// Doc text into paragraphs (blank-line separated).
fn paragraphs(doc: &str) -> String {
    doc.split("\n\n")
        .map(|p| format!("<p>{}</p>", inline(&p.replace('\n', " "))))
        .collect::<Vec<_>>()
        .join("")
}

fn render_html(modules: &[(String, ModuleDoc)], title: &str) -> String {
    let mut nav = String::new();
    let mut body = String::new();
    for (name, doc) in modules {
        nav.push_str(&format!("<a href=\"#m-{n}\">{n}</a>", n = esc(name)));
        body.push_str(&format!(
            "<section><h2 id=\"m-{n}\">{n}</h2>",
            n = esc(name)
        ));
        if !doc.module_doc.is_empty() {
            body.push_str(&format!(
                "<div class=\"mod\">{}</div>",
                paragraphs(&doc.module_doc)
            ));
        }
        for item in &doc.items {
            let mut badge = String::from(if item.shared {
                "<span class=\"badge\">shared</span>"
            } else {
                ""
            });
            if let Some(s) = item.stability {
                badge.push_str(&format!("<span class=\"badge {0}\">{0}</span>", s.as_str()));
            }
            body.push_str(&format!(
                "<div class=\"item\"><div class=\"sig\"><code>{}</code>{}</div>{}</div>",
                esc(&item.signature),
                badge,
                if item.doc.is_empty() {
                    String::new()
                } else {
                    paragraphs(&item.doc)
                }
            ));
        }
        body.push_str("</section>");
    }

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"/>\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/>\
<title>{title}</title><style>\
:root{{color-scheme:dark}}\
*{{box-sizing:border-box}}\
body{{margin:0;background:#0b0f14;color:#d9e6ef;\
font:15px/1.6 ui-monospace,SFMono-Regular,Menlo,monospace;padding:0 0 4rem}}\
nav{{position:sticky;top:0;display:flex;flex-wrap:wrap;gap:1rem;padding:0.7rem 1.2rem;\
background:rgba(11,15,20,0.9);border-bottom:1px solid #1d2937;backdrop-filter:blur(8px)}}\
nav a{{color:#7f93a3;text-decoration:none;font-size:13px}}nav a:hover{{color:#3ddc97}}\
main{{max-width:900px;margin:0 auto;padding:1.5rem 1.2rem}}\
h1{{color:#3ddc97;font-size:1.3rem}}\
h2{{color:#3ddc97;font-size:1.1rem;margin:2.4rem 0 0.4rem;padding-top:0.6rem;\
border-top:1px solid #1d2937}}\
.mod{{color:#9aa4b2}}\
.item{{background:#101720;border:1px solid #1d2937;border-radius:10px;\
padding:0.7rem 0.9rem;margin:0.7rem 0}}\
.sig{{display:flex;align-items:center;gap:0.6rem}}\
.sig code{{color:#5aa9e6;font-size:14px}}\
.badge{{color:#3ddc97;background:rgba(61,220,151,0.12);border-radius:6px;\
padding:1px 7px;font-size:11px}}\
.badge.experimental{{color:#f4b84c;background:rgba(244,184,76,0.12)}}\
p{{margin:0.5rem 0 0}}code{{color:#f4b84c}}\
</style></head><body><nav>{nav}</nav><main><h1>{title}</h1>{body}</main></body></html>",
        title = esc(title),
        nav = nav,
        body = body,
    )
}

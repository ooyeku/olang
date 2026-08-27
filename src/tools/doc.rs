//! `olang doc` — generate an API reference from doc comments.
//!
//! The convention is source-level and needs no grammar support, because
//! a doc comment is already a valid olang comment: `//!` at the top of a
//! file documents the module, and `///` lines immediately above a
//! declaration document it. `olang doc` scans `.ol` files for those,
//! pairs each `///` block with the declaration that follows, and renders
//! a browsable, suite-themed HTML page (or Markdown with `--md`).

use std::path::{Path, PathBuf};

/// One documented declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: String,      // "fn", "type", "error", "trait", "value"
    pub name: String,      // the declared name
    pub signature: String, // e.g. `h(tag, attrs, children)`
    pub doc: String,       // the joined `///` lines
    pub shared: bool,      // exported with `share`
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
                items.push(Item {
                    kind,
                    name,
                    signature,
                    doc: pending.join("\n").trim().to_string(),
                    shared,
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
        if let Some((kind, n, signature, shared)) = parse_decl(line.trim_start()) {
            if n == name {
                return Some(Item {
                    kind,
                    name: n,
                    signature,
                    doc: String::new(),
                    shared,
                });
            }
        }
    }
    None
}

fn module_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// `olang doc [paths] [-o out.html] [--md]`.
pub fn run(paths: &[PathBuf], output: &Path, markdown: bool) -> i32 {
    let mut files = Vec::new();
    for p in paths {
        files.extend(super::discover_ol_files(p));
    }
    files.sort();
    files.dedup();

    let mut modules: Vec<(String, ModuleDoc)> = Vec::new();
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
    if markdown {
        print!("{}", render_markdown(&modules));
    } else {
        let html = render_html(&modules, "olang reference");
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

// ── rendering ───────────────────────────────────────────────────────────

fn render_markdown(modules: &[(String, ModuleDoc)]) -> String {
    let mut out = String::from("# API Reference\n\n");
    for (name, doc) in modules {
        out.push_str(&format!("## `{}`\n\n", name));
        if !doc.module_doc.is_empty() {
            out.push_str(&doc.module_doc);
            out.push_str("\n\n");
        }
        for item in &doc.items {
            let badge = if item.shared { " · shared" } else { "" };
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
            let badge = if item.shared {
                "<span class=\"badge\">shared</span>"
            } else {
                ""
            };
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
p{{margin:0.5rem 0 0}}code{{color:#f4b84c}}\
</style></head><body><nav>{nav}</nav><main><h1>{title}</h1>{body}</main></body></html>",
        title = esc(title),
        nav = nav,
        body = body,
    )
}

//! Every `chapter.md#anchor` link in the book points at a heading that
//! exists.
//!
//! Renaming a heading in one chapter silently breaks the links other
//! chapters aim at it; this test is the thread between them.
//!
//! Slugs follow GitHub's convention, which any renderer of the book
//! must use too: lowercase, punctuation dropped, spaces to hyphens.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn slugify(heading: &str) -> String {
    let mut out = String::with_capacity(heading.len());
    for c in heading.trim().chars() {
        match c {
            ' ' => out.push('-'),
            '-' | '_' => out.push(c),
            c if c.is_alphanumeric() => out.extend(c.to_lowercase()),
            // Backticks, em-dashes, commas, colons, and the rest drop.
            _ => {}
        }
    }
    out
}

#[test]
fn every_cross_chapter_anchor_resolves() {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let mut files: Vec<_> = std::fs::read_dir(&docs)
        .expect("docs/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();

    // Headings per file, with code fences skipped: a `# comment` inside
    // an olang block is not a heading.
    let mut headings: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(path).expect("readable");
        let mut in_fence = false;
        let slugs = text
            .lines()
            .filter(|line| {
                if line.trim_start().starts_with("```") {
                    in_fence = !in_fence;
                }
                !in_fence
            })
            .filter_map(|line| {
                let trimmed = line.trim_start_matches('#');
                (trimmed.len() < line.len() && line.starts_with('#')).then(|| slugify(trimmed))
            })
            .collect();
        headings.insert(name, slugs);
    }

    let mut broken = Vec::new();
    let mut checked = 0usize;
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(path).expect("readable");
        let bytes = text.as_bytes();
        let mut i = 0;
        // Find every ](target.md#anchor) without pulling in a regex crate.
        while let Some(open) = text[i..].find("](") {
            let start = i + open + 2;
            let Some(close) = text[start..].find(')') else {
                break;
            };
            let link = &text[start..start + close];
            i = start + close;
            let _ = bytes; // text-indexed; kept for clarity
            let Some((target, anchor)) = link.split_once('#') else {
                continue;
            };
            if !target.ends_with(".md") || target.contains("://") {
                continue;
            }
            let file = target.rsplit('/').next().unwrap_or(target).to_string();
            let Some(slugs) = headings.get(&file) else {
                continue; // a missing *file* is the link checker's job
            };
            checked += 1;
            if !slugs.contains(anchor) {
                let line = text[..start].matches('\n').count() + 1;
                broken.push(format!("  {}:{} → {}#{}", name, line, target, anchor));
            }
        }
    }

    assert!(
        checked > 30,
        "only {checked} anchored links found — the extractor is broken, \
         not the book"
    );
    assert!(
        broken.is_empty(),
        "{} link(s) point at a heading that does not exist — usually a \
         heading renamed in one chapter while another still aims at the \
         old name:\n{}",
        broken.len(),
        broken.join("\n")
    );
}

/// The slugifier must match the convention, or every finding is noise.
#[test]
fn the_slugifier_follows_the_github_convention() {
    for (heading, slug) in [
        (
            "Reserved — not accepted, and additive if they ever are",
            "reserved--not-accepted-and-additive-if-they-ever-are",
        ),
        ("`plot` — charts as SVG text", "plot--charts-as-svg-text"),
        ("How `use` resolves", "how-use-resolves"),
        ("Operators and precedence", "operators-and-precedence"),
        ("`map_get` and `m[k]`", "map_get-and-mk"),
    ] {
        assert_eq!(slugify(heading), slug);
    }
}

/// The headings of every chapter, by file name, fences skipped.
fn book_slugs(docs: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let mut headings = BTreeMap::new();
    for e in std::fs::read_dir(docs).expect("docs/").flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable");
        let mut in_fence = false;
        let slugs = text
            .lines()
            .filter(|line| {
                if line.trim_start().starts_with("```") {
                    in_fence = !in_fence;
                }
                !in_fence
            })
            .filter_map(|line| {
                let trimmed = line.trim_start_matches('#');
                (trimmed.len() < line.len() && line.starts_with('#')).then(|| slugify(trimmed))
            })
            .collect();
        headings.insert(
            path.file_name().unwrap().to_string_lossy().to_string(),
            slugs,
        );
    }
    headings
}

/// A link from anywhere in the repository into the book (a README in
/// benchmarks/, an example's notes) aims at a heading that exists: a
/// chapter renamed or moved breaks those as surely as its siblings'.
#[test]
fn every_link_into_the_book_from_the_repository_resolves() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let docs = root.join("docs").canonicalize().expect("docs/");
    let headings = book_slugs(&docs);
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !matches!(
                    name.as_str(),
                    "target" | "vendor" | "node_modules" | ".git" | "docs"
                ) && !name.starts_with('.')
                {
                    stack.push(p);
                }
            } else if name.ends_with(".md") {
                files.push(p);
            }
        }
    }
    let mut broken = Vec::new();
    let mut checked = 0usize;
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let base = path.parent().unwrap();
        let mut i = 0;
        while let Some(open) = text[i..].find("](") {
            let start = i + open + 2;
            let Some(close) = text[start..].find(')') else {
                break;
            };
            let link = &text[start..start + close];
            i = start + close;
            let Some((target, anchor)) = link.split_once('#') else {
                continue;
            };
            if !target.ends_with(".md") || target.contains("://") {
                continue;
            }
            let Ok(resolved) = base.join(target).canonicalize() else {
                continue;
            };
            if resolved.parent() != Some(docs.as_path()) {
                continue;
            }
            let file = resolved.file_name().unwrap().to_string_lossy().to_string();
            let Some(slugs) = headings.get(&file) else {
                continue;
            };
            checked += 1;
            if !slugs.contains(anchor) {
                let line = text[..start].matches('\n').count() + 1;
                broken.push(format!(
                    "  {}:{} → {}#{}",
                    path.strip_prefix(root).unwrap_or(path).display(),
                    line,
                    target,
                    anchor
                ));
            }
        }
    }
    assert!(
        checked > 0,
        "no link into the book found: the extractor is broken"
    );
    assert!(
        broken.is_empty(),
        "{} link(s) into the book point at a heading that does not exist:\n{}",
        broken.len(),
        broken.join("\n")
    );
}

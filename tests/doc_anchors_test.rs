//! Every `chapter.md#anchor` link in the book points at a heading that
//! exists.
//!
//! The failure this guards against is indirect: renaming a heading in
//! one chapter silently breaks the links *other* chapters aimed at it.
//! R1's prose pass hit exactly that — `stability.md`'s "Reserved —
//! parses today, semantics later" heading was corrected (nothing under
//! it parses), and `types.md` was still linking to the old anchor while
//! repeating the old claim. The claim had one home in each chapter; the
//! anchor was the thread between them, and nothing checked the thread.
//!
//! Slugs follow the GitHub/marked convention the website's renderer
//! uses: lowercase, punctuation dropped, spaces to hyphens. If the
//! website's slugifier ever changes, this test's `slugify` must change
//! with it — they are one convention spelled twice.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn slugify(heading: &str) -> String {
    let mut out = String::with_capacity(heading.len());
    for c in heading.trim().chars() {
        match c {
            'A'..='Z' => out.push(c.to_ascii_lowercase()),
            'a'..='z' | '0'..='9' | '-' => out.push(c),
            ' ' => out.push('-'),
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
    ] {
        assert_eq!(slugify(heading), slug);
    }
}

//! A `// comment` that states an output must state the real one.
//!
//! `doc_examples_test` proves the book's programs run, and
//! `doc_references_test` proves the names it drops exist. Neither looks
//! at the value a line claims to print. So an example could run
//! perfectly while its comment taught something false — and several did:
//! `ods.linspace(0.0, 1.0, 5)` was annotated `// [0, 0.25, 0.5, 0.75,
//! 1]`, teaching that a Float column prints without its `.0`, and a
//! `show`n enum variant was annotated `// other: Blue` where the runtime
//! qualifies it as `Color.Blue`.
//!
//! A reader trusts these more than prose. They are the only place the
//! book says what a thing *is*, rather than what it does.
//!
//! The whole difficulty is telling a claimed value from a note. Most
//! comments are notes — `// skips the null`, `// both ran concurrently`,
//! `// 1 + 9 + 25` — and flagging those would make this a guard someone
//! turns off. So it checks only comments that are unambiguously a value
//! (a list, a number, a bool, a quoted string, or `()`), and only in
//! blocks where every `println` produced exactly one line, since a
//! program printing a `\n` inside a string breaks the correspondence
//! between the two.

use std::path::Path;
use std::process::Command;

/// Is this comment claiming a value, rather than remarking on one?
///
/// Deliberately narrow. A comment this rejects is simply not checked,
/// which costs a little coverage; a comment it wrongly accepts produces
/// a false failure, which costs the guard its credibility.
fn is_a_value(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    // A note that happens to begin with a value ("2 of them survive")
    // is not a value; require the whole comment to be one.
    if t.starts_with('[') && t.ends_with(']') {
        return true;
    }
    // A quoted string — but not an enumeration of them, which is a
    // remark that happens to begin and end with a quote.
    if t.starts_with('"') && t.ends_with('"') {
        return !t.contains('|') && t.matches('"').count() == 2;
    }
    if t == "()" || t == "true" || t == "false" {
        return true;
    }
    // A bare number, possibly negative or fractional — but not a range,
    // an ellipsis, or an approximation, which are all prose.
    let numeric = t.strip_prefix('-').unwrap_or(t);
    !numeric.is_empty()
        && numeric.chars().all(|c| c.is_ascii_digit() || c == '.')
        && numeric.chars().any(|c| c.is_ascii_digit())
        && !t.contains("..")
}

struct Block {
    origin: String,
    code: String,
}

fn runnable_blocks() -> Vec<Block> {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let mut files: Vec<_> = std::fs::read_dir(&docs)
        .expect("docs/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();

    let mut blocks = Vec::new();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        // The roadmap describes work not yet done; its snippets are
        // illustrations, not programs.
        if name == "roadmap.md" {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable");
        let mut in_block = false;
        let mut skip = false;
        let mut start = 0usize;
        let mut code = String::new();
        for (idx, line) in text.lines().enumerate() {
            if let Some(info) = line.strip_prefix("```") {
                if in_block {
                    if !skip {
                        blocks.push(Block {
                            origin: format!("{}:{}", name, start),
                            code: std::mem::take(&mut code),
                        });
                    } else {
                        code.clear();
                    }
                    in_block = false;
                } else {
                    let info = info.trim();
                    if info.split_whitespace().next() == Some("olang") {
                        in_block = true;
                        skip = info.contains("no-run");
                        start = idx + 2;
                    }
                }
            } else if in_block {
                code.push_str(line);
                code.push('\n');
            }
        }
    }
    blocks
}

#[test]
fn a_comment_that_states_an_output_states_the_real_one() {
    let dir = std::env::temp_dir().join(format!("olang_doc_outputs_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let file = dir.join("block.ol");

    let mut wrong = Vec::new();
    let mut checked = 0usize;

    for block in runnable_blocks() {
        // Only blocks that claim a value are worth running.
        let claims: Vec<(usize, String)> = block
            .code
            .lines()
            .enumerate()
            .filter_map(|(i, line)| {
                let trimmed = line.trim_start();
                if !trimmed.starts_with("println(") {
                    return None;
                }
                let comment = line.split("//").nth(1)?;
                is_a_value(comment).then(|| (i, comment.trim().to_string()))
            })
            .collect();
        if claims.is_empty() {
            continue;
        }

        std::fs::write(&file, &block.code).expect("write");
        let out = Command::new(env!("CARGO_BIN_EXE_olang"))
            .arg("run")
            .arg(&file)
            .current_dir(&dir)
            .output()
            .expect("olang runs");
        if !out.status.success() {
            // Failing programs are doc_examples_test's business.
            continue;
        }
        let printed: Vec<&str> = std::str::from_utf8(&out.stdout)
            .expect("utf-8")
            .lines()
            .collect();

        // The correspondence only holds when each println emitted one
        // line. A `\n` inside a printed string breaks it, and rather
        // than guess which output belongs to which call, skip the block.
        let println_lines: Vec<usize> = block
            .code
            .lines()
            .enumerate()
            .filter(|(_, l)| l.trim_start().starts_with("println("))
            .map(|(i, _)| i)
            .collect();
        if println_lines.len() != printed.len() {
            continue;
        }

        for (line, claim) in claims {
            let position = println_lines
                .iter()
                .position(|&l| l == line)
                .expect("the claim came from a println line");
            checked += 1;
            let actual = printed[position].trim();
            if actual != claim {
                wrong.push(format!(
                    "\n── {} (line {}) ──\n   comment: {}\n   printed: {}",
                    block.origin,
                    block.origin.split(':').next_back().unwrap_or("?"),
                    claim,
                    actual
                ));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        checked > 20,
        "only {checked} value comments found — the extractor is broken, \
         not the book"
    );
    assert!(
        wrong.is_empty(),
        "{} comment(s) claim an output the program does not print. The \
         example runs either way, which is exactly why this is easy to \
         miss — a reader trusts the comment.\n{}",
        wrong.len(),
        wrong.join("")
    );
}

/// The extractor has to reject remarks, or the guard becomes noise.
#[test]
fn only_unambiguous_values_are_checked() {
    for value in [
        "[1.0, 2.0]",
        "42",
        "-3",
        "2.5",
        "true",
        "false",
        "()",
        "\"hi\"",
    ] {
        assert!(is_a_value(value), "{value} is a value");
    }
    for note in [
        "skips the null: 2.0",
        "1 + 9 + 25",
        "both ran concurrently",
        "1.9599...",
        "≈ 0.975",
        "0..3",
        "",
        "entries are sorted by key",
    ] {
        assert!(!is_a_value(note), "{note:?} is a remark, not a value");
    }
}

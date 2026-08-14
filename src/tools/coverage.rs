//! Line coverage for `olang test --coverage`.
//!
//! The test runner records, per source file, the set of line numbers that
//! actually executed (see `Interpreter::enable_coverage`). This module
//! supplies the *denominator* — every line a file could have executed —
//! and renders the report.
//!
//! Both halves key off the same thing: `Statement::Located`. The runtime
//! records a line the moment a located statement runs; the denominator is
//! every located line reachable in the file's AST. Deriving the executable
//! set from the parsed program (rather than counting non-blank source
//! lines) means the two are measured the same way, so a fully-exercised
//! file reports exactly 100% — never 96% because of a brace-only line.

use colored::*;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::Parser;

/// Every line carrying an executable statement in `source` — the set of
/// lines coverage can possibly record. Collected from the parsed AST by
/// gathering every `Located` node's line. A parse failure yields an empty
/// set (the file is then reported as unmeasured and skipped).
///
/// The walk is over the *serialized* AST rather than a hand-written match
/// on all ~50 expression variants: a `Located` statement can nest anywhere
/// an expression can (a block passed as a call argument, an `if` branch,
/// a `let` value), so completeness matters more than avoiding the JSON
/// round-trip, which happens once per file at report time.
pub fn executable_lines(source: &str) -> BTreeSet<u32> {
    let parser = Parser::new();
    let Ok(program) = parser.parse(source) else {
        return BTreeSet::new();
    };
    let Ok(value) = serde_json::to_value(&program) else {
        return BTreeSet::new();
    };
    let mut lines = BTreeSet::new();
    collect_located_lines(&value, &mut lines);
    lines
}

/// Recurse the serialized AST, collecting the `line` of every `Located`
/// node. `Statement::Located { line, .. }` serializes (serde's default
/// external tagging) as `{"Located": {"line": N, "column": M, "stmt": …}}`.
fn collect_located_lines(value: &serde_json::Value, out: &mut BTreeSet<u32>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(located) = map.get("Located")
                && let Some(line) = located.get("line").and_then(|l| l.as_u64())
            {
                out.insert(line as u32);
            }
            for v in map.values() {
                collect_located_lines(v, out);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                collect_located_lines(v, out);
            }
        }
        _ => {}
    }
}

/// One file's coverage: `covered` of `total` executable lines.
struct FileReport {
    display: String,
    covered: usize,
    total: usize,
    missing: Vec<u32>,
}

/// Render the coverage report from the recorded hits (file path -> the set
/// of lines executed in it), aggregated across every test file. Only real
/// on-disk source files are reported; synthetic module paths (the embedded
/// packages) are skipped. `show_missing` lists the un-executed lines.
pub fn print_report(recorded: &HashMap<String, BTreeSet<u32>>, show_missing: bool) {
    let cwd = std::env::current_dir().ok();
    let mut reports: Vec<FileReport> = Vec::new();

    for (file, hits) in recorded {
        let path = Path::new(file);
        // Embedded packages and other non-file modules have no source on
        // disk to measure against — skip them rather than guess.
        if !path.is_file() {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let exec = executable_lines(&source);
        if exec.is_empty() {
            continue;
        }
        // Intersect so a hit can never exceed the executable set — coverage
        // is always in [0, 100]% even if attribution ever drifts.
        let covered = hits.iter().filter(|l| exec.contains(l)).count();
        let missing: Vec<u32> = exec.iter().filter(|l| !hits.contains(l)).copied().collect();
        reports.push(FileReport {
            display: display_path(file, cwd.as_deref()),
            covered,
            total: exec.len(),
            missing,
        });
    }

    if reports.is_empty() {
        return;
    }
    reports.sort_by(|a, b| a.display.cmp(&b.display));

    let name_w = reports
        .iter()
        .map(|r| r.display.chars().count())
        .max()
        .unwrap_or(0)
        .max("total".len());

    println!("{}", "─".repeat(40));
    println!("  {}", "coverage".bold());

    let mut tot_c = 0usize;
    let mut tot_t = 0usize;
    for r in &reports {
        tot_c += r.covered;
        tot_t += r.total;
        let pct = pct_of(r.covered, r.total);
        println!(
            "    {:<name_w$}  {}  {}",
            r.display,
            colored_pct(pct),
            format!("{}/{}", r.covered, r.total).dimmed(),
            name_w = name_w,
        );
        if show_missing && !r.missing.is_empty() {
            println!(
                "      {} {}",
                "uncovered".dimmed(),
                fmt_ranges(&r.missing).dimmed()
            );
        }
    }

    let overall = pct_of(tot_c, tot_t);
    println!(
        "    {:<name_w$}  {}  {}",
        "total".bold(),
        colored_pct(overall),
        format!("{}/{}", tot_c, tot_t).dimmed(),
        name_w = name_w,
    );
}

fn pct_of(covered: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * covered as f64 / total as f64
    }
}

/// A right-aligned "NN%" (or "100%"), colored by health: green at 80+,
/// yellow at 50+, red below. Padded before coloring so columns line up.
fn colored_pct(pct: f64) -> ColoredString {
    let s = format!("{:>3}%", pct.round() as i64);
    if pct >= 80.0 {
        s.green().bold()
    } else if pct >= 50.0 {
        s.yellow().bold()
    } else {
        s.red().bold()
    }
}

/// Make a file path relative to the current directory for display, so the
/// report reads `src/tools/coverage.rs`, not a long absolute path.
fn display_path(file: &str, cwd: Option<&Path>) -> String {
    if let Some(cwd) = cwd
        && let Ok(rel) = Path::new(file).strip_prefix(cwd)
    {
        return rel.to_string_lossy().into_owned();
    }
    file.to_string()
}

/// Collapse a sorted line list into compact ranges: `[3,4,5,9,10] -> "3-5, 9-10"`.
fn fmt_ranges(lines: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let start = lines[i];
        let mut end = start;
        while i + 1 < lines.len() && lines[i + 1] == end + 1 {
            end += 1;
            i += 1;
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        i += 1;
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_lines_counts_located_statements() {
        // Three statements on lines 1, 2, 3 — each a located statement.
        let src = "let a = 1\nlet b = 2\nprintln(show(a + b))\n";
        let lines = executable_lines(src);
        assert_eq!(lines, BTreeSet::from([1, 2, 3]));
    }

    #[test]
    fn executable_lines_reaches_into_nested_blocks() {
        // The body statements (lines 2, 3) live inside a function block, and
        // the call is line 5. All must be found — nesting can't hide a line.
        let src = "fn f(x) = {\n  let y = x + 1\n  y * 2\n}\nlet z = f(3)\n";
        let lines = executable_lines(src);
        assert!(lines.contains(&2), "body line inside block: {lines:?}");
        assert!(lines.contains(&3), "body line inside block: {lines:?}");
        assert!(lines.contains(&5), "top-level call: {lines:?}");
    }

    #[test]
    fn ranges_collapse() {
        assert_eq!(fmt_ranges(&[3, 4, 5, 9, 10]), "3-5, 9-10");
        assert_eq!(fmt_ranges(&[7]), "7");
        assert_eq!(fmt_ranges(&[]), "");
    }
}

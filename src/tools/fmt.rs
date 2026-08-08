//! `olang fmt [paths] [--check]` — a conservative, safety-verified
//! formatter.
//!
//! v1 scope is whitespace hygiene, applied only *outside* multi-line string
//! literals:
//!
//! - CRLF line endings become LF
//! - trailing spaces and tabs are stripped
//! - leading tabs become 4 spaces (the book's indentation unit)
//! - runs of blank lines collapse to one
//! - the file ends with exactly one newline
//!
//! It deliberately does not re-indent, re-wrap, or reflow — comments and
//! layout are the author's. The safety property is absolute: the formatted
//! source must parse to an AST identical to the original's, or the file is
//! left untouched and reported. `--check` reports files that would change
//! and exits non-zero without writing anything.

use crate::Parser;
use colored::*;

/// Where the scanner is, with respect to string-ish tokens that whitespace
/// rules must not reach into.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Code,
    DoubleQuote,       // "..." — single line in practice, tracked for safety
    RawString,         // r"..." — may span lines
    Template,          // `...` — may span lines
    TemplateExpr(u32), // ${ ... } inside a template, brace-depth counted
}

/// Track string state across a line, returning the mode at line end.
/// `starts_in` is the mode the line begins in.
fn scan_line_mode(line: &str, starts_in: Mode) -> Mode {
    let mut mode = starts_in;
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let next = bytes.get(i + 1).copied();
        match mode {
            Mode::Code => match c {
                '/' if next == Some('/') => return mode, // comment: rest of line is inert
                'r' if next == Some('"') => {
                    mode = Mode::RawString;
                    i += 1;
                }
                '"' => mode = Mode::DoubleQuote,
                '`' => mode = Mode::Template,
                _ => {}
            },
            Mode::DoubleQuote => match c {
                '\\' => i += 1, // skip escaped char
                '"' => mode = Mode::Code,
                _ => {}
            },
            Mode::RawString => match c {
                '\\' if next == Some('"') => i += 1,
                '"' => mode = Mode::Code,
                _ => {}
            },
            Mode::Template => match c {
                '\\' => i += 1,
                '`' => mode = Mode::Code,
                '$' if next == Some('{') => {
                    mode = Mode::TemplateExpr(0);
                    i += 1;
                }
                _ => {}
            },
            Mode::TemplateExpr(depth) => match c {
                '{' => mode = Mode::TemplateExpr(depth + 1),
                '}' => {
                    mode = if depth == 0 {
                        Mode::Template
                    } else {
                        Mode::TemplateExpr(depth - 1)
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    mode
}

/// Apply the whitespace rules. Lines that begin inside a multi-line string
/// are passed through byte-for-byte.
pub fn format_source(source: &str) -> String {
    let source = source.replace("\r\n", "\n");
    let mut out: Vec<String> = Vec::new();
    let mut mode = Mode::Code;
    let mut last_was_blank = false;

    for line in source.split('\n') {
        let starts_in_string = mode != Mode::Code;
        let end_mode = scan_line_mode(line, mode);

        if starts_in_string {
            // Inside a template/raw string: whitespace is content.
            out.push(line.to_string());
            last_was_blank = false;
        } else {
            // Leading tabs -> 4 spaces (indentation only).
            let mut rebuilt = String::new();
            let mut in_indent = true;
            for c in line.chars() {
                match c {
                    '\t' if in_indent => rebuilt.push_str("    "),
                    c => {
                        if c != ' ' {
                            in_indent = false;
                        }
                        rebuilt.push(c);
                    }
                }
            }
            // Trailing whitespace — but only when the line ENDS in code; a
            // line that opens a multi-line string keeps its tail intact.
            let cleaned = if end_mode == Mode::Code {
                rebuilt.trim_end().to_string()
            } else {
                rebuilt
            };

            let is_blank = cleaned.is_empty();
            if is_blank && last_was_blank {
                // collapse runs of blank lines
            } else {
                out.push(cleaned);
            }
            last_was_blank = is_blank;
        }
        mode = end_mode;
    }

    // Exactly one trailing newline.
    while out.last().map(|l| l.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out.push(String::new());
    out.join("\n")
}

/// Format files under the given paths. Returns the process exit code.
pub fn run(paths: &[std::path::PathBuf], check: bool) -> i32 {
    let parser = Parser::new();
    let mut changed: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    for path in paths {
        for file in super::discover_ol_files(path) {
            scanned += 1;
            let original = match std::fs::read_to_string(&file) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let formatted = format_source(&original);
            if formatted == original {
                continue;
            }

            // Safety gate: identical AST or nothing.
            let before = parser.parse(&original);
            let after = parser.parse(&formatted);
            match (before, after) {
                (Ok(a), Ok(b)) if a == b => {
                    changed.push(file.display().to_string());
                    if !check && std::fs::write(&file, &formatted).is_err() {
                        eprintln!("  could not write {}", file.display());
                    }
                }
                (Err(_), _) => {
                    // Unparseable source: not ours to touch.
                    skipped.push(format!("{} (does not parse)", file.display()));
                }
                _ => {
                    skipped.push(format!(
                        "{} (formatting would change the AST)",
                        file.display()
                    ));
                }
            }
        }
    }

    for s in &skipped {
        println!("  {} {}", "~ skipped".yellow(), s);
    }
    if changed.is_empty() {
        println!(
            "  {} ({} file{} scanned)",
            "all formatted".green(),
            scanned,
            if scanned == 1 { "" } else { "s" }
        );
        0
    } else if check {
        for f in &changed {
            println!("  {} {}", "would reformat".red(), f);
        }
        println!(
            "  {} of {} file{} need formatting",
            changed.len(),
            scanned,
            if scanned == 1 { "" } else { "s" }
        );
        1
    } else {
        for f in &changed {
            println!("  {} {}", "reformatted".green(), f);
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_whitespace_and_collapses_blanks() {
        let src = "let x = 1   \n\n\n\nlet y = 2\t\n";
        assert_eq!(format_source(src), "let x = 1\n\nlet y = 2\n");
    }

    #[test]
    fn leading_tabs_become_spaces() {
        let src = "fn f(x) = {\n\tx + 1\n}\n";
        assert_eq!(format_source(src), "fn f(x) = {\n    x + 1\n}\n");
    }

    #[test]
    fn exactly_one_trailing_newline() {
        assert_eq!(format_source("let x = 1"), "let x = 1\n");
        assert_eq!(format_source("let x = 1\n\n\n"), "let x = 1\n");
    }

    #[test]
    fn template_string_interiors_are_untouched() {
        let src = "let t = `line one   \n\ttabbed content\n\n\nstill inside`\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn trailing_spaces_inside_a_double_quoted_string_survive() {
        let src = "let s = \"padded   \" + \"x\"\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn comment_lines_are_cleaned_but_kept() {
        let src = "// a comment with trailing space   \nlet x = 1\n";
        assert_eq!(
            format_source(src),
            "// a comment with trailing space\nlet x = 1\n"
        );
    }

    #[test]
    fn idempotent() {
        let src = "let x = 1   \n\n\n\n\tlet y = 2\n";
        let once = format_source(src);
        assert_eq!(format_source(&once), once);
    }
}

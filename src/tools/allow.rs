//! `// allow: <class>[, <class>…]` — one line's advisories silenced where
//! they are, by class.
//!
//! The comment goes at the end of the line the finding is on, or on a
//! line of its own directly above it (other comment lines between are
//! allowed; a blank line or code ends it). It silences that line's
//! advisories of the classes it names — `all` for every advisory class,
//! `unused` for the hint of a name nothing reads, `tier` for a function
//! `olang check --tier` finds the bytecode tier refuses (the comment on
//! its `fn` line), a `rule_*` name for a `--rules` rule's findings — and
//! nothing else: a syntax, scope, type or run-time error is never
//! silenced. An allow wins over `[check] promote`: it is explicit and
//! local, so a promoted class's finding on an allowed line is not
//! reported at all. A name that is no class is itself warned about.
//!
//! The text after the list is free (`// allow: no_effect — the lens
//! evaluates it`): the list ends at the first name not followed by a
//! comma.

use std::collections::HashMap;

use super::check::CheckDiagnostic;

/// The names an allow takes besides a `rule_*` rule's: every class
/// `[check] promote` knows, the `unused` hint's, and `all`.
pub fn known_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = crate::pkg::manifest::PROMOTE_CLASSES.to_vec();
    v.push("unused");
    v.push("all");
    v
}

/// A file's allow comments: the names allowed on each line (1-based),
/// and the names that are no class, where they are written.
#[derive(Debug, Default)]
pub struct Allows {
    lines: HashMap<u32, Vec<String>>,
    unknown: Vec<(u32, u32, String)>,
    empty: Vec<(u32, u32)>,
}

impl Allows {
    /// Does an allow on `line` name `class` (or `all`)? `all` covers the
    /// checker's classes, never a project rule's.
    pub fn allows(&self, line: u32, class: &str) -> bool {
        self.lines.get(&line).is_some_and(|names| {
            names
                .iter()
                .any(|n| n == class || (n == "all" && !class.starts_with("rule_")))
        })
    }

    /// Is this finding an advisory an allow on its line silences? Only a
    /// warning with a class, so an error never is.
    pub fn silences(&self, d: &CheckDiagnostic) -> bool {
        d.warning && d.class.is_some_and(|c| self.allows(d.line, c))
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.unknown.is_empty() && self.empty.is_empty()
    }

    /// The warnings an allow comment earns itself: a name that is no
    /// class (with the nearest), a list with no name. Class `allow`;
    /// never silenced or promoted.
    pub fn problems(&self) -> Vec<CheckDiagnostic> {
        let known = known_names();
        let mut out: Vec<CheckDiagnostic> = self
            .unknown
            .iter()
            .map(|(line, column, name)| {
                let nearest = known
                    .iter()
                    .filter(|k| {
                        crate::interpreter::IntuitiveErrorFormatter::levenshtein_distance(
                            &name.to_ascii_lowercase(),
                            k,
                        ) <= 2
                    })
                    .min_by_key(|k| {
                        crate::interpreter::IntuitiveErrorFormatter::levenshtein_distance(name, k)
                    });
                let hint = match nearest {
                    Some(k) => format!(" — did you mean `{k}`?"),
                    None => format!(" (the classes: {}, or a rule's rule_* name)", known.join(", ")),
                };
                CheckDiagnostic {
                    line: *line,
                    column: *column,
                    message: format!("`// allow:` names `{name}`, which is no advisory class: it allows nothing{hint}"),
                    runtime: false,
                    warning: true,
                    scope: false,
                    class: Some("allow"),
                }
            })
            .collect();
        out.extend(self.empty.iter().map(|(line, column)| CheckDiagnostic {
            line: *line,
            column: *column,
            message: "`// allow:` names no class: it allows nothing (`// allow: no_effect`, `// allow: shape, keys`)".to_string(),
            runtime: false,
            warning: true,
            scope: false,
            class: Some("allow"),
        }));
        out.sort_by_key(|d| (d.line, d.column));
        out
    }
}

/// Where each line's `//` comment starts (its byte offset in the line),
/// outside strings — `"…"` (across lines too), `r"…"`, a template's
/// backquotes, a `'x'` character — or `None`.
fn comment_starts(source: &str) -> Vec<Option<usize>> {
    #[derive(PartialEq, Clone, Copy)]
    enum In {
        Code,
        Str,
        Raw,
        Template,
    }
    let mut state = In::Code;
    let mut out = Vec::new();
    for line in source.split('\n') {
        let b = line.as_bytes();
        let mut found = None;
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            match state {
                In::Str | In::Template => {
                    if c == b'\\' {
                        i += 1;
                    } else if (state == In::Str && c == b'"')
                        || (state == In::Template && c == b'`')
                    {
                        state = In::Code;
                    }
                }
                In::Raw => {
                    if c == b'\\' && b.get(i + 1) == Some(&b'"') {
                        i += 1;
                    } else if c == b'"' {
                        state = In::Code;
                    }
                }
                In::Code => {
                    if c == b'/' && b.get(i + 1) == Some(&b'/') {
                        found = Some(i);
                        break;
                    } else if c == b'"' {
                        let raw = i > 0
                            && b[i - 1] == b'r'
                            && (i < 2 || !(b[i - 2].is_ascii_alphanumeric() || b[i - 2] == b'_'));
                        state = if raw { In::Raw } else { In::Str };
                    } else if c == b'`' {
                        state = In::Template;
                    } else if c == b'\'' {
                        // a character literal: 'x' (one character, any width)
                        if let Some(ch) = line[i + 1..].chars().next() {
                            let after = i + 1 + ch.len_utf8();
                            if b.get(after) == Some(&b'\'') {
                                i = after;
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        out.push(found);
    }
    out
}

/// The allow list a comment's text (after its `//`) carries: its names
/// with their byte offsets in the text, or `None` when it is no allow.
fn allow_list(text: &str) -> Option<Vec<(usize, String)>> {
    let lead = text.len() - text.trim_start().len();
    text[lead..].strip_prefix("allow:")?;
    let mut at = lead + "allow:".len();
    let b = text.as_bytes();
    let mut names = Vec::new();
    loop {
        while at < b.len() && (b[at] == b' ' || b[at] == b'\t') {
            at += 1;
        }
        let start = at;
        while at < b.len() && (b[at].is_ascii_alphanumeric() || b[at] == b'_') {
            at += 1;
        }
        if at == start {
            break;
        }
        names.push((start, text[start..at].to_string()));
        let mut j = at;
        while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
            j += 1;
        }
        if b.get(j) == Some(&b',') {
            at = j + 1;
        } else {
            break;
        }
    }
    Some(names)
}

/// Every allow comment in `source`, by the line it applies to.
pub fn scan(source: &str) -> Allows {
    let mut allows = Allows::default();
    if !source.contains("allow:") {
        return allows;
    }
    let known = known_names();
    let starts = comment_starts(source);
    // the names of allow comments on lines of their own, waiting for
    // the line they stand above
    let mut armed: Vec<String> = Vec::new();
    for (i, line) in source.split('\n').enumerate() {
        let n = (i + 1) as u32;
        let code_before = |at: usize| !line[..at].trim().is_empty();
        let comment = starts[i];
        let names = comment.and_then(|at| {
            let list = allow_list(&line[at + 2..])?;
            Some((at, list))
        });
        let mut here: Vec<String> = Vec::new();
        if let Some((at, list)) = &names {
            if list.is_empty() {
                allows
                    .empty
                    .push((n, line[..*at].chars().count() as u32 + 1));
            }
            for (off, name) in list {
                if known.contains(&name.as_str()) || name.starts_with("rule_") {
                    here.push(name.clone());
                } else {
                    let col = line[..at + 2 + off].chars().count() as u32 + 1;
                    allows.unknown.push((n, col, name.clone()));
                }
            }
        }
        let own_line = comment.is_some_and(|at| !code_before(at));
        if own_line {
            // a comment line: an allow arms the line below; others pass
            armed.extend(here);
        } else if line.trim().is_empty() {
            armed.clear();
        } else {
            // code: the allows above and its own end
            let mut all = std::mem::take(&mut armed);
            all.extend(here);
            if !all.is_empty() {
                allows.lines.entry(n).or_default().extend(all);
            }
        }
    }
    allows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_and_above() {
        let a = scan(
            "\"a\" // allow: no_effect\n// allow: shape, keys\n// why\nx.y\n\n// allow: copy\n\nz\n",
        );
        assert!(a.allows(1, "no_effect"));
        assert!(!a.allows(1, "shape"));
        assert!(a.allows(4, "shape") && a.allows(4, "keys"));
        assert!(!a.allows(8, "copy"), "a blank line ends an allow above");
        assert!(a.problems().is_empty());
    }

    #[test]
    fn strings_are_not_comments() {
        let a = scan(
            "let u = \"http://x // allow: shape\"\nlet c = '\"' // allow: keys\nlet r = r\"\\\" // allow: copy\"\n",
        );
        assert!(!a.allows(1, "shape"));
        assert!(a.allows(2, "keys"));
        assert!(!a.allows(3, "copy"));
    }

    #[test]
    fn unknown_and_free_text() {
        let a = scan(
            "x // allow: no_efect, shape\ny // allow: result — dropped on purpose, rule_x\nz // allow:\n",
        );
        assert!(a.allows(1, "shape"));
        assert!(a.allows(2, "result"));
        assert!(
            !a.allows(2, "rule_x"),
            "the list ends at the first name not followed by a comma"
        );
        let p = a.problems();
        assert_eq!(p.len(), 2);
        assert!(
            p[0].message.contains("no_efect") && p[0].message.contains("did you mean `no_effect`")
        );
        assert_eq!((p[0].line, p[0].column), (1, 13));
        assert_eq!(p[1].line, 3);
    }

    #[test]
    fn all_and_rules() {
        let a = scan("x // allow: all\ny // allow: rule_no_print\n");
        assert!(a.allows(1, "no_effect") && a.allows(1, "unused"));
        assert!(
            !a.allows(1, "rule_no_print"),
            "all is the checker's classes"
        );
        assert!(a.allows(2, "rule_no_print"));
    }
}

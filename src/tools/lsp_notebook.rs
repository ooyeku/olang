//! A notebook (`.olmd`) as the language server reads it: one document
//! whose cells are regions of one module. The server answers about the
//! notebook's program (docs/notebooks.md, "The program": every line that
//! is not a cell's code made empty, the params' `let`s on their front
//! matter lines), so a cell's line and column are the file's and a
//! position inside a cell needs no translation. What the program holds
//! that the file does not say the same way — a param's `let` on its key's
//! line — is mapped: the key in the front matter is the param's name.
//! A position in prose, a fence or the front matter's other lines is
//! about nothing; a `${expr}` in prose reads names too, so references
//! and a rename find them there.

use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};

use super::{char_to_utf16_col, utf16_range, utf16_to_char_col};

/// A param's name where the file and the program write it.
struct Param {
    name: String,
    /// 0-based line (its key's, in the front matter).
    line: u32,
    /// UTF-16 column of the key in the file's line.
    key: u32,
    /// UTF-16 column of the name in the program's line (after `let `).
    at: u32,
}

/// A notebook read for the server.
pub(super) struct Notebook {
    /// The program the server analyses (`olang check`'s: `no-run` cells
    /// in, no `--param` given).
    pub program: String,
    problems: Vec<crate::notebook::Diagnostic>,
    /// 1-based lines of the front matter's `---`s.
    front: Option<(u32, u32)>,
    /// Per 0-based line: whether it is a cell's code.
    code: Vec<bool>,
    params: Vec<Param>,
    /// The file's lines.
    lines: Vec<String>,
    /// Per 0-based line: whether it is in a pinned output block (its
    /// fences too): a cell's output, never prose a name is read in.
    pinned: Vec<bool>,
}

/// Whether a document is a notebook (its name ends `.olmd`).
pub(super) fn is_notebook(uri: &lsp_types::Url) -> bool {
    uri.path().ends_with(".olmd")
}

/// The notebook in `text`.
pub(super) fn read(text: &str) -> Notebook {
    let nb = crate::notebook::parse(text);
    let program = crate::notebook::program_with(text, crate::notebook::Mode::Check, &[]);
    let lines: Vec<String> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect();
    let mut code = vec![false; lines.len()];
    for cell in &nb.cells {
        for l in cell.code_lines() {
            if let Some(slot) = code.get_mut(l as usize - 1) {
                *slot = true;
            }
        }
    }
    let mut pinned = vec![false; lines.len()];
    for o in &nb.outputs {
        for l in o.fence_line..=o.close_line {
            if let Some(slot) = pinned.get_mut(l as usize - 1) {
                *slot = true;
            }
        }
    }
    let plines: Vec<&str> = program.source.split('\n').collect();
    let params = nb
        .params
        .iter()
        .filter_map(|p| {
            let line = p.line.checked_sub(1)?;
            let file = lines.get(line as usize)?;
            let key = key_col(file, &p.name)?;
            let prog = plines.get(line as usize)?;
            let at = prog.find(&format!("let {} =", p.name))? + 4;
            Some(Param {
                name: p.name.clone(),
                line,
                key: char_to_utf16_col(file, file[..key].chars().count()),
                at: char_to_utf16_col(prog, prog[..at].chars().count()),
            })
        })
        .collect();
    Notebook {
        program: program.source,
        problems: program.diagnostics,
        front: program.front_matter,
        code,
        params,
        lines,
        pinned,
    }
}

/// The byte offset of a param's key on its front matter line: the name
/// first after the indentation, followed by `:`.
fn key_col(line: &str, name: &str) -> Option<usize> {
    let at = line.len() - line.trim_start().len();
    let rest = &line[at..];
    let after = rest.strip_prefix(name)?;
    after.trim_start().starts_with(':').then_some(at)
}

impl Notebook {
    fn is_code(&self, line: u32) -> bool {
        self.code.get(line as usize).copied().unwrap_or(false)
    }

    fn in_front(&self, line: u32) -> bool {
        // 0-based `line` between the `---` lines (1-based open, close)
        matches!(self.front, Some((open, close)) if line + 1 > open && line + 1 < close)
    }

    /// A file position as the program's: the same inside a cell, a
    /// param's key as its name after `let`; `None` anywhere else.
    pub fn pos_in(&self, pos: Position) -> Option<Position> {
        if self.is_code(pos.line) {
            return Some(pos);
        }
        let p = self.params.iter().find(|p| p.line == pos.line)?;
        let len = p.name.encode_utf16().count() as u32;
        (pos.character >= p.key && pos.character <= p.key + len)
            .then(|| Position::new(pos.line, p.at + (pos.character - p.key)))
    }

    /// A program range as the file's: the same inside a cell, a param's
    /// name as its key; `None` for anything else.
    pub fn range_out(&self, r: Range) -> Option<Range> {
        if self.is_code(r.start.line) && self.is_code(r.end.line) {
            return Some(r);
        }
        if r.start.line != r.end.line {
            return None;
        }
        let p = self.params.iter().find(|p| p.line == r.start.line)?;
        let len = p.name.encode_utf16().count() as u32;
        (r.start.character == p.at && r.end.character == p.at + len).then(|| {
            Range::new(
                Position::new(p.line, p.key),
                Position::new(p.line, p.key + len),
            )
        })
    }

    /// Whether the program's line `line` is a cell's code (what a token,
    /// a hint or an action on it is about).
    pub fn code_line(&self, line: u32) -> bool {
        self.is_code(line)
    }

    /// Every standalone `name` read by a `${expr}` in prose (not after a
    /// `.`: a field), as the file's ranges.
    pub fn prose_occurrences(&self, name: &str) -> Vec<Range> {
        let target: Vec<char> = name.chars().collect();
        let mut out = Vec::new();
        if target.is_empty() {
            return out;
        }
        for (ln, line) in self.lines.iter().enumerate() {
            let l = ln as u32;
            if self.is_code(l)
                || self.pinned.get(l as usize).copied().unwrap_or(false)
                || self.in_front(l)
                || self.front.is_some_and(|(o, c)| l + 1 == o || l + 1 == c)
            {
                continue;
            }
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i + 1 < chars.len() {
                if chars[i] != '$' || chars[i + 1] != '{' {
                    i += 1;
                    continue;
                }
                // the expression's end: its braces balanced
                let mut depth = 0;
                let mut j = i + 1;
                let mut end = None;
                while j < chars.len() {
                    match chars[j] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(j);
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let Some(end) = end else { break };
                let mut k = i + 2;
                while k + target.len() <= end {
                    let word = |c: char| c.is_alphanumeric() || c == '_';
                    if chars[k..k + target.len()] == target[..]
                        && !(k > 0 && (word(chars[k - 1]) || chars[k - 1] == '.'))
                        && !chars.get(k + target.len()).is_some_and(|c| word(*c))
                    {
                        out.push(Range::new(
                            Position::new(l, char_to_utf16_col(line, k)),
                            Position::new(l, char_to_utf16_col(line, k + target.len())),
                        ));
                        k += target.len();
                        continue;
                    }
                    k += 1;
                }
                i = end + 1;
            }
        }
        out
    }

    /// The word a `${expr}` in prose has at `pos`, when it is one: what
    /// references and a rename asked there are about.
    pub fn prose_word(&self, pos: Position) -> Option<String> {
        if self.is_code(pos.line) || self.in_front(pos.line) {
            return None;
        }
        let line = self.lines.get(pos.line as usize)?;
        let chars: Vec<char> = line.chars().collect();
        let at = utf16_to_char_col(line, pos.character).min(chars.len());
        // inside a `${ … }` on its line
        let open = line[..line
            .char_indices()
            .nth(at)
            .map(|(b, _)| b)
            .unwrap_or(line.len())]
            .rfind("${")?;
        let open_c = line[..open].chars().count();
        if line[open..]
            .find('}')
            .map(|e| line[..open + e].chars().count())
            .is_some_and(|e| e < at)
        {
            return None;
        }
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut s = at;
        while s > open_c + 2 && word(chars[s - 1]) {
            s -= 1;
        }
        let mut e = at;
        while e < chars.len() && word(chars[e]) {
            e += 1;
        }
        (e > s && !chars[s].is_ascii_digit()).then(|| chars[s..e].iter().collect())
    }

    /// The notebook's problems: its own (`notebook`), then its program's
    /// as a `.ol` file's (`olang check`'s), a param no cell reads no
    /// unused name.
    pub fn diagnostics(&self, text: &str, dir: Option<&std::path::Path>) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = self
            .problems
            .iter()
            .map(|d| {
                let line = d.line.saturating_sub(1);
                let col = d.column.saturating_sub(1) as usize;
                Diagnostic {
                    range: utf16_range(text, line, col, col + 1),
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: Some(NumberOrString::String("notebook".to_string())),
                    source: Some("olang".to_string()),
                    message: d.message.clone(),
                    ..Default::default()
                }
            })
            .collect();
        let found = super::diagnostics(&self.program, dir);
        // a name only prose reads (`${fmt_money(total)}`) is used
        out.extend(found.into_iter().filter(|d| {
            let unused = matches!(&d.code, Some(NumberOrString::String(c)) if c == "unused");
            let read_in_prose = || {
                d.message
                    .strip_prefix("unused variable: ")
                    .is_some_and(|name| !self.prose_occurrences(name).is_empty())
            };
            !(unused && (self.in_front(d.range.start.line) || read_in_prose()))
        }));
        out
    }
}

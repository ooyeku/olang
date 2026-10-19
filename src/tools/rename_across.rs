//! A rename across files: a name a module `share`s, renamed where it is
//! declared and in every file of the folder that imports it — its `use`
//! lists (`use m { name }`, the source side of `name as alias`, a
//! re-export), the places reading the import's binding, and the module's
//! namespace (`m.name`). The language server renamed in one file only, so
//! renaming a shared function broke every importer (and renaming it at an
//! import broke the importer itself: its `use` named nothing).
//!
//! Text in, places out: the caller (the language server) gives each
//! file's program (an open document's unsaved text, else the disk's) and
//! turns the places into its edits. Nothing here writes.

use crate::ast::{Program, ShareDecl, Statement, UseDecl, UseItem};
use crate::tools::scope_refs;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One file's places to rename: 0-based line, start and end character.
pub type Places = Vec<(u32, usize, usize)>;

/// Where a name is declared and shared, if the name at `line`, `col`
/// (characters) of `file`'s program `text` is one that other files may
/// import: `None` for anything else (a local, a builtin, a private name,
/// an alias), which a rename in one file serves.
fn declaring(
    text: &str,
    file: &Path,
    line: u32,
    col: usize,
    read: &dyn Fn(&Path) -> Option<String>,
) -> Option<(String, PathBuf)> {
    let r = scope_refs::resolve_at(text, line, col)?;
    if !r.bound || !r.top_level {
        return None;
    }
    let name = r.name;
    let program = crate::parser::Parser::new().parse(text).ok()?;
    match top_decl(&program, &name) {
        Decl::Shared => return Some((name, file.to_path_buf())),
        Decl::Private => return None,
        Decl::None => {}
    }
    // an import: the module it names, and the one that declares it behind
    // a re-export
    let dir = file.parent()?;
    for u in uses(&program) {
        let binds = u
            .items
            .iter()
            .any(|i| matches!(i, UseItem::Specific(n) if *n == name))
            || u.items.is_empty()
            || u.items.iter().any(|i| matches!(i, UseItem::Wildcard));
        if !binds {
            continue;
        }
        if let Some(found) = module_of(u, dir).and_then(|m| origin(&m, &name, read, 0)) {
            return Some((name, found));
        }
    }
    None
}

enum Decl {
    Shared,
    Private,
    None,
}

/// Whether `program` declares `name` at its top level, shared or not.
fn top_decl(program: &Program, name: &str) -> Decl {
    for s in &program.statements {
        let shared = match s.unwrapped() {
            Statement::ShareDecl(ShareDecl::Function(f)) if f.name == name => true,
            Statement::ShareDecl(ShareDecl::Type(t)) if t.name == name => true,
            Statement::ShareDecl(ShareDecl::Let(l)) if let_names(l).iter().any(|n| n == name) => {
                true
            }
            Statement::FunctionDecl(f) if f.name == name => false,
            Statement::TypeDecl(t) if t.name == name => false,
            Statement::LetDecl(l) if let_names(l).iter().any(|n| n == name) => false,
            _ => continue,
        };
        return if shared { Decl::Shared } else { Decl::Private };
    }
    Decl::None
}

fn let_names(l: &crate::ast::LetDecl) -> Vec<String> {
    match &l.pattern {
        crate::ast::Pattern::Identifier(n) => vec![n.clone()],
        _ => Vec::new(),
    }
}

/// Every `use` and `share use` of a program.
fn uses(program: &Program) -> Vec<&UseDecl> {
    program
        .statements
        .iter()
        .filter_map(|s| match s.unwrapped() {
            Statement::UseDecl(u) => Some(u),
            Statement::ShareDecl(ShareDecl::Use(u)) => Some(u),
            _ => None,
        })
        .collect()
}

/// Whether a `use` is a re-export.
fn reexports(program: &Program, u: &UseDecl) -> bool {
    program.statements.iter().any(
        |s| matches!(s.unwrapped(), Statement::ShareDecl(ShareDecl::Use(x)) if std::ptr::eq(x, u)),
    )
}

/// The file a `use` in a file of `dir` reads, as the checker finds it.
fn module_of(u: &UseDecl, dir: &Path) -> Option<PathBuf> {
    if u.path.is_empty() {
        return None;
    }
    let line = format!("use {}", u.path.join("."));
    // the module named comes last (re-exports it carries come first)
    let found = crate::tools::check::module_programs(&line, Some(dir));
    found.last().map(|(p, _, _)| canonical(p))
}

/// The module that declares `name` and shares it, following re-exports
/// from `module`.
fn origin(
    module: &Path,
    name: &str,
    read: &dyn Fn(&Path) -> Option<String>,
    depth: u32,
) -> Option<PathBuf> {
    if depth > 8 {
        return None;
    }
    let text = read(module)?;
    let program = crate::parser::Parser::new().parse(&text).ok()?;
    if let Decl::Shared = top_decl(&program, name) {
        return Some(module.to_path_buf());
    }
    let dir = module.parent()?;
    for u in uses(&program) {
        let passes = reexports(&program, u)
            && u.items.iter().any(|i| {
                matches!(i, UseItem::Specific(n) if n == name) || matches!(i, UseItem::Wildcard)
            });
        if passes
            && let Some(found) = module_of(u, dir).and_then(|m| origin(&m, name, read, depth + 1))
        {
            return Some(found);
        }
    }
    None
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The places to rename `name` in each file, when the name at `line`,
/// `col` of `file` (its program `text`) is shared by a module of the
/// folder `root`: `None` when it is not (one file's rename serves);
/// `Err` when the rename cannot be made whole (the module is outside the
/// folder, or an importer does not parse). `files` are the folder's
/// programs (`.ol` and `.olmd`); `read` answers a file's program text (an
/// open document's own).
pub fn rename_across(
    file: &Path,
    text: &str,
    line: u32,
    col: usize,
    root: &Path,
    files: &[PathBuf],
    read: &dyn Fn(&Path) -> Option<String>,
) -> Option<Result<BTreeMap<PathBuf, Places>, String>> {
    let file = canonical(file);
    let (name, decl) = declaring(text, &file, line, col, read)?;
    let root = canonical(root);
    if !decl.starts_with(&root) {
        return Some(Err(format!(
            "'{}' is declared in {}, outside the folder open — rename it there",
            name,
            decl.display()
        )));
    }
    let mut out: BTreeMap<PathBuf, Places> = BTreeMap::new();
    let decl_text = read(&decl)?;
    match scope_refs::resolve_top(&decl_text, &name) {
        Some(r) => {
            out.insert(decl.clone(), r.places);
        }
        None => return Some(Err(format!("{} does not parse", decl.display()))),
    }
    // the folder's files that name it, read once
    let mut read_files: Vec<(PathBuf, String, Option<Program>)> = Vec::new();
    for f in files {
        let f = canonical(f);
        if f == decl {
            continue;
        }
        let Some(t) = read(&f) else { continue };
        if !t.contains(name.as_str()) {
            continue;
        }
        let program = crate::parser::Parser::new().parse(&t).ok();
        read_files.push((f, t, program));
    }
    // the modules that share it: its own, and those re-exporting it
    let mut sharing = vec![decl.clone()];
    loop {
        let mut more = false;
        for (f, _, program) in &read_files {
            let Some(program) = program else { continue };
            if sharing.contains(f) {
                continue;
            }
            let dir = f.parent().unwrap_or(Path::new("."));
            let reexported = uses(program).into_iter().any(|u| {
                reexports(program, u)
                    && u.items.iter().any(|i| {
                        matches!(i, UseItem::Specific(n) if *n == name)
                            || matches!(i, UseItem::Wildcard)
                    })
                    && module_of(u, dir).is_some_and(|m| sharing.contains(&m))
            });
            if reexported {
                sharing.push(f.clone());
                more = true;
            }
        }
        if !more {
            break;
        }
    }
    for (f, t, program) in &read_files {
        // which of its `use`s read a module that shares the name: by its
        // `use` lines alone when the file does not parse (mid-edit)
        let dir = f.parent().unwrap_or(Path::new("."));
        let lines_only;
        let prog = match program {
            Some(p) => p,
            None => {
                let use_lines: Vec<&str> = t
                    .lines()
                    .filter(|l| {
                        l.trim_start().starts_with("use ")
                            || l.trim_start().starts_with("share use ")
                    })
                    .collect();
                lines_only = match crate::parser::Parser::new().parse(&use_lines.join("\n")) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                &lines_only
            }
        };
        let hits: Vec<&UseDecl> = uses(prog)
            .into_iter()
            .filter(|u| module_of(u, dir).is_some_and(|m| sharing.contains(&m)))
            .collect();
        if hits.is_empty() {
            continue;
        }
        if program.is_none() {
            return Some(Err(format!(
                "{} imports '{}' and does not parse — fix it, then rename",
                f.display(),
                name
            )));
        }
        let mut places: Places = Vec::new();
        let listed = hits.iter().any(|u| {
            u.items
                .iter()
                .any(|i| matches!(i, UseItem::Specific(n) if *n == name))
        });
        let wild = hits
            .iter()
            .any(|u| u.items.is_empty() || u.items.iter().any(|i| matches!(i, UseItem::Wildcard)));
        if listed {
            // the import's binding: its item in the list and every place
            // reading it
            if let Some(r) = scope_refs::resolve_top(t, &name) {
                places.extend(r.places);
            }
        } else if wild && let Some(r) = unbound_reads(t, &name) {
            places.extend(r);
        }
        // `name as alias`: the source side only (the alias is the file's)
        places.extend(aliased(t, &name));
        // the namespace's `m.name`
        for u in &hits {
            if let Some(ns) = u.path.last() {
                places.extend(qualified(t, ns, &name));
            }
        }
        places.sort();
        places.dedup();
        if !places.is_empty() {
            out.insert(f.clone(), places);
        }
    }
    Some(Ok(out))
}

/// The places reading `name` bound nowhere in the program (what a
/// wildcard import brings): `None` when the program binds a `name` of its
/// own (it shadows the import) or never reads it.
fn unbound_reads(text: &str, name: &str) -> Option<Places> {
    let masked = mask(text);
    for (ln, l) in masked.split('\n').enumerate() {
        for (c, _) in words(l, name) {
            if let Some(r) = scope_refs::resolve_at(text, ln as u32, c)
                && r.name == name
            {
                return if r.bound { None } else { Some(r.places) };
            }
        }
    }
    None
}

/// `name` where a `use` list says `name as alias`.
fn aliased(text: &str, name: &str) -> Places {
    let masked = mask(text);
    let mut out = Vec::new();
    let mut in_use = false;
    for (ln, l) in masked.split('\n').enumerate() {
        let t = l.trim_start();
        let starts = t.starts_with("use ") || t.starts_with("share use ");
        if starts {
            in_use = true;
        }
        if in_use {
            let chars: Vec<char> = l.chars().collect();
            for (c, end) in words(l, name) {
                let rest: String = chars[end..].iter().collect();
                let rest = rest.trim_start();
                if rest.starts_with("as ") || rest.starts_with("as\t") {
                    out.push((ln as u32, c, end));
                }
            }
            // a list ends at its brace; `use m` alone is one line
            if l.contains('}') || (starts && !l.contains('{')) {
                in_use = false;
            }
        }
    }
    out
}

/// `ns.name` (the module's namespace read), outside strings and comments.
fn qualified(text: &str, ns: &str, name: &str) -> Places {
    let masked = mask(text);
    let mut out = Vec::new();
    for (ln, l) in masked.split('\n').enumerate() {
        let chars: Vec<char> = l.chars().collect();
        for (c, end) in words(l, name) {
            let n = ns.chars().count();
            if c < n + 1 || chars[c - 1] != '.' {
                continue;
            }
            let start = c - 1 - n;
            let before_ok = start == 0
                || !(chars[start - 1].is_alphanumeric()
                    || chars[start - 1] == '_'
                    || chars[start - 1] == '.');
            let spelled: String = chars[start..c - 1].iter().collect();
            if before_ok && spelled == ns {
                out.push((ln as u32, c, end));
            }
        }
    }
    out
}

/// The character spans of `name` as a whole word on `line`.
fn words(line: &str, name: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let target: Vec<char> = name.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + target.len() <= chars.len() {
        if chars[i..i + target.len()] == target[..] {
            let before_ok = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
            let after_ok = chars
                .get(i + target.len())
                .is_none_or(|c| !(c.is_alphanumeric() || *c == '_'));
            if before_ok && after_ok {
                out.push((i, i + target.len()));
                i += target.len();
                continue;
            }
        }
        i += 1;
    }
    out
}

/// `text` with its strings' contents and its comments blanked, every
/// character kept in its column (a name said in a string or a comment is
/// not renamed).
fn mask(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_str = false;
    let mut in_comment = false;
    let mut esc = false;
    let mut prev = '\0';
    for ch in text.chars() {
        if ch == '\n' {
            in_comment = false;
            out.push('\n');
            prev = ch;
            continue;
        }
        if in_comment {
            out.push(' ');
        } else if in_str {
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
                out.push('"');
                prev = ch;
                continue;
            }
            out.push(' ');
        } else if ch == '"' {
            in_str = true;
            out.push('"');
        } else if ch == '/' && prev == '/' {
            out.pop();
            out.push(' ');
            out.push(' ');
            in_comment = true;
        } else {
            out.push(ch);
        }
        prev = ch;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_and_comments_are_masked_in_place() {
        let m = mask("let a = \"helper\" // helper\nhelper(1)");
        assert_eq!(m, "let a = \"      \"          \nhelper(1)");
    }

    #[test]
    fn a_namespace_read_and_an_alias_are_found() {
        assert_eq!(
            qualified("let y = a.helper(4) + b.a.helper(1)", "a", "helper"),
            vec![(0, 10, 16)]
        );
        assert_eq!(
            aliased("use a { helper as h, other }\nh(1)", "helper"),
            vec![(0, 8, 14)]
        );
        assert_eq!(
            aliased("use a {\n    helper as h,\n}\n", "helper"),
            vec![(1, 4, 10)]
        );
    }
}

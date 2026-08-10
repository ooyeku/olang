//! `otc unused` — shared functions no other file in the tree imports.
//!
//! Usage is counted three ways, so the report errs toward "used" rather
//! than flagging live code:
//! - a specific import: `use utils { helper }`
//! - a wildcard or bare import of the module: `use utils { * }` / `use utils`
//!   (every shared function of that module counts as used)
//! - a namespace reference: `utils.helper(...)` anywhere in a file

use anyhow::{Context, Result};
use olang::parser::Parser;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub fn execute(dir_path: String, verbose: bool) -> Result<()> {
    let path = Path::new(&dir_path);

    if verbose {
        println!(
            "Scanning for unused shared functions in: {}",
            path.display()
        );
    }

    let analysis = analyze_unused_functions(path)?;
    display_unused_analysis(&analysis, verbose);

    Ok(())
}

#[derive(Debug)]
struct UnusedAnalysis {
    /// module (file stem) -> its shared functions
    shared_functions: BTreeMap<String, Vec<String>>,
    /// module -> the shared functions nothing else imports or references
    unused_functions: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Default)]
struct Usage {
    /// Names imported specifically: `use m { name }`.
    named: HashSet<String>,
    /// Modules imported wholesale: `use m { * }` or bare `use m`.
    whole_modules: HashSet<String>,
    /// Raw source of every scanned file, for namespace references (`m.f`).
    sources: Vec<String>,
}

fn analyze_unused_functions(dir_path: &Path) -> Result<UnusedAnalysis> {
    let ol_files = find_ol_files(dir_path)?;

    let mut shared_functions = BTreeMap::new();
    let mut usage = Usage::default();

    for file_path in &ol_files {
        let source = std::fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path.display()))?;
        let parser = Parser::new();
        let ast = parser
            .parse(&source)
            .with_context(|| format!("Failed to parse file: {}", file_path.display()))?;

        let module = file_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let mut functions = Vec::new();
        for statement in &ast.statements {
            match statement {
                olang::ast::Statement::ShareDecl(olang::ast::ShareDecl::Function(func)) => {
                    functions.push(func.name.clone());
                }
                olang::ast::Statement::UseDecl(use_decl) => {
                    let mut whole = use_decl.items.is_empty();
                    for item in &use_decl.items {
                        match item {
                            olang::ast::UseItem::Specific(name) => {
                                usage.named.insert(name.clone());
                            }
                            olang::ast::UseItem::Wildcard => whole = true,
                        }
                    }
                    if whole {
                        // `use foo` / `use foo.bar { * }`: both the head (a
                        // dependency name) and the last segment (a sibling
                        // file's stem) are how the module may be known here.
                        for segment in [use_decl.path.first(), use_decl.path.last()]
                            .into_iter()
                            .flatten()
                        {
                            usage.whole_modules.insert(segment.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        if !functions.is_empty() {
            shared_functions.insert(module, functions);
        }
        usage.sources.push(source);
    }

    let mut unused_functions = BTreeMap::new();
    for (module, functions) in &shared_functions {
        let unused: Vec<String> = functions
            .iter()
            .filter(|func| !is_used(module, func, &usage))
            .cloned()
            .collect();

        if !unused.is_empty() {
            unused_functions.insert(module.clone(), unused);
        }
    }

    Ok(UnusedAnalysis {
        shared_functions,
        unused_functions,
    })
}

fn is_used(module: &str, func: &str, usage: &Usage) -> bool {
    if usage.named.contains(func) || usage.whole_modules.contains(module) {
        return true;
    }
    // Namespace reference: `module.func` as a standalone token pair.
    let needle = format!("{}.{}", module, func);
    usage
        .sources
        .iter()
        .any(|source| contains_token(source, &needle))
}

/// Whether `needle` occurs in `haystack` with no identifier character on
/// either side — so `geometry.area` matches, but `mygeometry.area` and
/// `geometry.area_of` don't. Occurrences inside strings or comments still
/// count; for a linter, over-counting usage is the safe direction.
fn contains_token(haystack: &str, needle: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let begin = start + pos;
        let end = begin + needle.len();
        let before_ok = haystack[..begin]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident(c));
        let after_ok = haystack[end..].chars().next().is_none_or(|c| !is_ident(c));
        if before_ok && after_ok {
            return true;
        }
        start = begin + 1;
    }
    false
}

fn find_ol_files(dir_path: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    if dir_path.is_file() && dir_path.extension().is_some_and(|ext| ext == "ol") {
        files.push(dir_path.to_path_buf());
        return Ok(files);
    }

    if dir_path.is_dir() {
        for entry in std::fs::read_dir(dir_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && path.extension().is_some_and(|ext| ext == "ol") {
                files.push(path);
            } else if path.is_dir() {
                files.extend(find_ol_files(&path)?);
            }
        }
    }

    Ok(files)
}

fn display_unused_analysis(analysis: &UnusedAnalysis, verbose: bool) {
    if analysis.unused_functions.is_empty() {
        println!("No unused shared functions found!");
        return;
    }

    println!("Unused shared functions:");

    for (file, functions) in &analysis.unused_functions {
        println!("\n{}:", file);
        for func in functions {
            println!("  - {}", func);
        }
    }

    if verbose {
        println!("\n=== Analysis Summary ===");
        let total_shared = analysis
            .shared_functions
            .values()
            .map(|v| v.len())
            .sum::<usize>();
        let total_unused = analysis
            .unused_functions
            .values()
            .map(|v| v.len())
            .sum::<usize>();

        println!("Total shared functions: {}", total_shared);
        println!("Total unused functions: {}", total_unused);

        if total_shared > 0 {
            let usage_rate = ((total_shared - total_unused) as f32 / total_shared as f32) * 100.0;
            println!("Usage rate: {:.1}%", usage_rate);
        }
    }
}

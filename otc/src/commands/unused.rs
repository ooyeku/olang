use anyhow::{Context, Result};
use olang::parser::Parser;
use std::collections::{HashMap, HashSet};
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
    shared_functions: HashMap<String, Vec<String>>, // file -> functions
    used_functions: HashSet<String>,
    unused_functions: HashMap<String, Vec<String>>, // file -> unused functions
}

fn analyze_unused_functions(dir_path: &Path) -> Result<UnusedAnalysis> {
    let mut shared_functions = HashMap::new();
    let mut used_functions = HashSet::new();

    // Find all .ol files
    let ol_files = find_ol_files(dir_path)?;

    // First pass: collect all shared functions
    for file_path in &ol_files {
        let functions = extract_shared_functions(file_path)?;
        if !functions.is_empty() {
            shared_functions.insert(
                file_path.file_stem().unwrap().to_string_lossy().to_string(),
                functions,
            );
        }
    }

    // Second pass: collect all used functions
    for file_path in &ol_files {
        let used = extract_used_functions(file_path)?;
        used_functions.extend(used);
    }

    // Calculate unused functions
    let mut unused_functions = HashMap::new();
    for (file, functions) in &shared_functions {
        let unused: Vec<String> = functions
            .iter()
            .filter(|func| !used_functions.contains(*func))
            .cloned()
            .collect();

        if !unused.is_empty() {
            unused_functions.insert(file.clone(), unused);
        }
    }

    Ok(UnusedAnalysis {
        shared_functions,
        used_functions,
        unused_functions,
    })
}

fn find_ol_files(dir_path: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    if dir_path.is_file() && dir_path.extension().map_or(false, |ext| ext == "ol") {
        files.push(dir_path.to_path_buf());
        return Ok(files);
    }

    if dir_path.is_dir() {
        for entry in std::fs::read_dir(dir_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && path.extension().map_or(false, |ext| ext == "ol") {
                files.push(path);
            } else if path.is_dir() {
                files.extend(find_ol_files(&path)?);
            }
        }
    }

    Ok(files)
}

fn extract_shared_functions(file_path: &Path) -> Result<Vec<String>> {
    let source = std::fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    let parser = Parser::new();
    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", file_path.display()))?;

    let mut functions = Vec::new();

    for statement in &ast.statements {
        if let olang::ast::Statement::ShareDecl(share_decl) = statement {
            if let olang::ast::ShareDecl::Function(func) = share_decl {
                functions.push(func.name.clone());
            }
        }
    }

    Ok(functions)
}

fn extract_used_functions(file_path: &Path) -> Result<Vec<String>> {
    let source = std::fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    let parser = Parser::new();
    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", file_path.display()))?;

    let mut used = Vec::new();

    for statement in &ast.statements {
        if let olang::ast::Statement::UseDecl(use_decl) = statement {
            for item in &use_decl.items {
                if let olang::ast::UseItem::Specific(name) = item {
                    used.push(name.clone());
                }
                // Note: Wildcard imports can't be tracked for unused analysis
                // since we don't know what specific functions they import
            }
        }
    }

    Ok(used)
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
        println!("Total used functions: {}", analysis.used_functions.len());
        println!("Total unused functions: {}", total_unused);

        if total_shared > 0 {
            let usage_rate = ((total_shared - total_unused) as f32 / total_shared as f32) * 100.0;
            println!("Usage rate: {:.1}%", usage_rate);
        }
    }
}

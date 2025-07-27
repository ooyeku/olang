use anyhow::{Context, Result};
use olang::parser::Parser;
use std::path::Path;

pub fn execute(file_path: String, verbose: bool) -> Result<()> {
    let path = Path::new(&file_path);

    if verbose {
        println!("Analyzing dependencies for: {}", path.display());
    }

    let deps = analyze_dependencies(path)?;
    display_dependencies(&file_path, &deps, verbose);

    Ok(())
}

fn analyze_dependencies(file_path: &Path) -> Result<Vec<String>> {
    let source = std::fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    let parser = Parser::new();
    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", file_path.display()))?;

    let mut dependencies = Vec::new();

    // Traverse AST to find use statements
    extract_use_statements(&ast, &mut dependencies);

    Ok(dependencies)
}

fn extract_use_statements(ast: &olang::ast::Program, dependencies: &mut Vec<String>) {
    for statement in &ast.statements {
        if let olang::ast::Statement::UseDecl(use_decl) = statement {
            let module_path = use_decl.path.join(".");
            dependencies.push(module_path);
        }
    }
}

fn display_dependencies(file_path: &str, dependencies: &[String], verbose: bool) {
    println!("Dependencies for {}:", file_path);
    
    if dependencies.is_empty() {
        println!("  No dependencies found");
        return;
    }

    for (i, dep) in dependencies.iter().enumerate() {
        if verbose {
            println!("  {}: {}", i + 1, dep);
        } else {
            println!("  {}", dep);
        }
    }

    if verbose {
        println!("\nTotal dependencies: {}", dependencies.len());
    }
} 
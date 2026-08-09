use anyhow::{Context, Result};
use olang::analyze::Analyzer;
use olang::parser::Parser;
use std::path::Path;

pub fn execute(file_path: String, verbose: bool) -> Result<()> {
    let path = Path::new(&file_path);

    if verbose {
        println!("Checking file: {}", path.display());
    }

    let source = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    let parser = Parser::new();
    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", path.display()))?;

    let mut analyzer = Analyzer::new();
    match analyzer.analyze_program(&ast) {
        Ok(_) => {
            println!("No issues found in {}.", path.display());
            // Here you could add more checks, like for unused variables
        }
        Err(e) => {
            eprintln!("Error in {}: {}", path.display(), e);
            // Indicate that an error was found
            std::process::exit(1);
        }
    }

    Ok(())
}

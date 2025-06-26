use anyhow::{Context, Result};
use olang::{interpreter::Interpreter, parser::Parser};
use std::path::Path;

pub fn execute(file_path: Option<String>, verbose: bool) -> Result<()> {
    // If file is None, fall back to interactive REPL (stdin batch mode)
    let file_path = file_path.ok_or_else(|| anyhow::anyhow!("No file specified"))?;
    let path = Path::new(&file_path);

    if verbose {
        println!("Running file: {}", path.display());
    }

    let source = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    let parser = Parser::new();
    let mut interpreter = Interpreter::new();

    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", path.display()))?;

    let result = interpreter
        .eval_program(ast)
        .map_err(|e| anyhow::anyhow!("Runtime error: {}", e))?;

    if verbose {
        println!("Result: {:?}", result);
    }

    Ok(())
}

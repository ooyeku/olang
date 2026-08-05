use anyhow::{Context, Result};
use olang::{parser::Parser, Interpreter};
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

    // The interpreter with the bytecode tier: eligible functions compile on
    // their first call (same execution model as the olang CLI default)
    let mut interpreter = Interpreter::new();
    interpreter.enable_bytecode_tier(1, verbose);
    interpreter.set_current_file(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));

    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", path.display()))?;

    let result = interpreter
        .eval_program(ast)
        .map_err(|e| anyhow::anyhow!("Runtime error: {}", e))?;

    if verbose {
        println!("Result: {:?}", result);

        if let Some(tier) = interpreter.bytecode_tier_stats() {
            println!("\nBytecode tier statistics:");
            println!("  Functions promoted: {}", tier.promoted);
            println!("  Functions rejected: {}", tier.rejected);
            println!("  Bytecode calls:     {}", tier.bytecode_calls);
        }
    }

    Ok(())
}

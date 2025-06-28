use anyhow::{Context, Result};
use olang::{ovm_integration::{IntegrationConfig, OvmInterpreter}, ovm::OvmConfig, parser::Parser};
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
    
    // Use OVM interpreter with JIT compilation by default
    let integration_config = IntegrationConfig {
        use_ovm_by_default: true,
        ovm_complexity_threshold: 5,
        auto_compile_functions: true,
        enable_ovm_lazy_eval: true,
        fallback_on_error: true,
        enable_ovm_builtins: true,
        ovm_preferred_builtins: vec![
            "len".to_string(),
            "typeof".to_string(),
            "to_string".to_string(),
            "sum".to_string(),
            "average".to_string(),
            "min".to_string(),
            "max".to_string(),
            "reverse".to_string(),
            "sort".to_string(),
            "contains".to_string(),
        ],
    };

    let mut interpreter = OvmInterpreter::with_config(integration_config);
    
    // Initialize OVM with default configuration
    let ovm_config = OvmConfig::default();
    interpreter.initialize_ovm(ovm_config)
        .with_context(|| "Failed to initialize OVM")?;

    if verbose {
        println!("🚀 OVM initialized with JIT compilation enabled");
    }

    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", path.display()))?;

    let result = interpreter
        .eval_program(ast)
        .map_err(|e| anyhow::anyhow!("Runtime error: {}", e))?;

    if verbose {
        println!("Result: {:?}", result);
        
        // Show OVM performance statistics
        let stats = interpreter.get_stats();
        println!("\n📊 OVM Performance Statistics:");
        println!("  Classic executions: {}", stats.classic_executions);
        println!("  OVM executions: {}", stats.ovm_executions);
        println!("  Fallback executions: {}", stats.fallback_executions);
        println!("  Compilations: {}", stats.compilation_count);
        println!("  Average classic time: {:.2}ms", stats.average_classic_time_ms);
        println!("  Average OVM time: {:.2}ms", stats.average_ovm_time_ms);
    }

    Ok(())
}

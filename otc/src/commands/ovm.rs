//! OVM Command - Execute Olang programs using the Olang Virtual Machine
//!
//! Provides high-performance execution with lazy evaluation, garbage collection,
//! and JIT compilation capabilities.

use clap::Args;
use olang::ovm::config::OptimizationLevel;
use olang::{ExecutionMode, IntegrationConfig, OvmConfig, OvmInterpreter, Parser, Program};
use std::fs;
use std::path::PathBuf;

#[derive(Args)]
pub struct OvmCommand {
    /// Input file to execute
    #[arg(value_name = "FILE")]
    pub file: PathBuf,

    /// Execution mode
    #[arg(short, long, default_value = "auto")]
    pub mode: String,

    /// Optimization level
    #[arg(short, long, default_value = "release")]
    pub optimization: String,

    /// Enable detailed performance monitoring
    #[arg(long)]
    pub performance: bool,

    /// Disable OVM fallback to classic interpreter
    #[arg(long)]
    pub no_fallback: bool,

    /// Force garbage collection after execution
    #[arg(long)]
    pub force_gc: bool,

    /// Show execution statistics
    #[arg(long)]
    pub stats: bool,

    /// Enable lazy evaluation by default
    #[arg(long)]
    pub lazy: bool,

    /// Memory threshold for GC trigger (MB)
    #[arg(long, default_value = "64")]
    pub memory_threshold: usize,

    /// JIT compilation threshold
    #[arg(long, default_value = "100")]
    pub jit_threshold: usize,
}

impl OvmCommand {
    pub fn execute(&self) -> anyhow::Result<()> {
        // Read the input file
        let content = fs::read_to_string(&self.file)
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", self.file.display(), e))?;

        // Parse execution mode
        let execution_mode = match self.mode.as_str() {
            "auto" => ExecutionMode::Auto,
            "classic" => ExecutionMode::Classic,
            "ovm" => ExecutionMode::Ovm,
            "benchmark" => ExecutionMode::Benchmark,
            _ => return Err(anyhow::anyhow!("Invalid execution mode: {}", self.mode)),
        };

        // Parse optimization level
        let optimization_level = match self.optimization.as_str() {
            "debug" => OptimizationLevel::Debug,
            "balanced" => OptimizationLevel::Balanced,
            "release" => OptimizationLevel::Release,
            "adaptive" => OptimizationLevel::Adaptive,
            _ => {
                return Err(anyhow::anyhow!(
                    "Invalid optimization level: {}",
                    self.optimization
                ))
            }
        };

        // Create integration configuration
        let integration_config = IntegrationConfig {
            use_ovm_by_default: matches!(execution_mode, ExecutionMode::Auto | ExecutionMode::Ovm),
            ovm_complexity_threshold: 1,
            auto_compile_functions: true,
            enable_ovm_lazy_eval: self.lazy,
            fallback_on_error: !self.no_fallback,
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

        // Create OVM configuration
        let ovm_config = match optimization_level {
            OptimizationLevel::Debug => OvmConfig::development(),
            OptimizationLevel::Balanced => OvmConfig::high_performance(),
            OptimizationLevel::Release => OvmConfig::high_performance(),
            OptimizationLevel::Adaptive => OvmConfig::high_performance(),
        };

        // Create interpreter with OVM integration
        let mut interpreter = OvmInterpreter::with_config(integration_config);

        // Initialize OVM
        if matches!(
            execution_mode,
            ExecutionMode::Auto | ExecutionMode::Ovm | ExecutionMode::Benchmark
        ) {
            interpreter
                .initialize_ovm(ovm_config)
                .map_err(|e| anyhow::anyhow!("Failed to initialize OVM: {}", e))?;

            if self.performance {
                println!(
                    "✓ OVM initialized with {:?} optimization",
                    optimization_level
                );
            }
        }

        // Parse the program
        let parser = Parser::new();
        let program = parser
            .parse(&content)
            .map_err(|e| anyhow::anyhow!("Parse error: {:?}", e))?;

        // Execute the program
        let start_time = std::time::Instant::now();

        let result = match execution_mode {
            ExecutionMode::Auto => interpreter.eval_program(program),
            ExecutionMode::Classic => interpreter.eval_program_classic(program),
            ExecutionMode::Ovm => interpreter.eval_program_ovm(program),
            ExecutionMode::Benchmark => {
                self.run_benchmark(&mut interpreter, program)?;
                return Ok(());
            }
        };

        let execution_time = start_time.elapsed();

        // Handle result
        match result {
            Ok(value) => {
                // Only print non-unit values
                if !matches!(value, olang::Value::Unit) {
                    println!("{:?}", value);
                }

                if self.performance {
                    println!("Execution time: {}ms", execution_time.as_millis());
                }
            }
            Err(e) => {
                eprintln!("Runtime error: {}", e);
                std::process::exit(1);
            }
        }

        // Force GC if requested
        if self.force_gc {
            interpreter
                .force_gc()
                .map_err(|e| anyhow::anyhow!("GC failed: {}", e))?;

            if self.performance {
                println!("✓ Garbage collection completed");
            }
        }

        // Show statistics if requested
        if self.stats {
            self.print_statistics(&interpreter);
        }

        Ok(())
    }

    fn run_benchmark(
        &self,
        interpreter: &mut OvmInterpreter,
        program: Program,
    ) -> anyhow::Result<()> {
        println!("Running benchmark for: {}", self.file.display());
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        const WARMUP_ROUNDS: usize = 3;
        const BENCHMARK_ROUNDS: usize = 10;

        // Warmup phase
        if self.performance {
            println!("Warming up...");
        }

        for _ in 0..WARMUP_ROUNDS {
            let _ = interpreter.eval_program_classic(program.clone());
            if interpreter.is_ovm_available() {
                let _ = interpreter.eval_program_ovm(program.clone());
            }
        }

        // Benchmark classic interpreter
        let mut classic_times = Vec::new();
        for _ in 0..BENCHMARK_ROUNDS {
            let start = std::time::Instant::now();
            let _ = interpreter.eval_program_classic(program.clone())?;
            classic_times.push(start.elapsed());
        }

        let classic_avg =
            classic_times.iter().sum::<std::time::Duration>() / classic_times.len() as u32;
        
        // Safe handling of min/max without unwrap
        let (classic_min, classic_max) = match (classic_times.iter().min(), classic_times.iter().max()) {
            (Some(min), Some(max)) => (min, max),
            _ => {
                eprintln!("Error: Failed to calculate classic interpreter benchmark statistics");
                return Err(anyhow::anyhow!("Benchmark calculation failed"));
            }
        };

        println!("Classic Interpreter:");
        println!("  Average: {}µs", classic_avg.as_micros());
        println!("  Min:     {}µs", classic_min.as_micros());
        println!("  Max:     {}µs", classic_max.as_micros());

        // Benchmark OVM if available
        if interpreter.is_ovm_available() {
            let mut ovm_times = Vec::new();
            for _ in 0..BENCHMARK_ROUNDS {
                let start = std::time::Instant::now();
                let _ = interpreter.eval_program_ovm(program.clone())?;
                ovm_times.push(start.elapsed());
            }

            let ovm_avg = ovm_times.iter().sum::<std::time::Duration>() / ovm_times.len() as u32;
            
            // Safe handling of min/max without unwrap
            let (ovm_min, ovm_max) = match (ovm_times.iter().min(), ovm_times.iter().max()) {
                (Some(min), Some(max)) => (min, max),
                _ => {
                    eprintln!("Error: Failed to calculate OVM benchmark statistics");
                    return Err(anyhow::anyhow!("OVM benchmark calculation failed"));
                }
            };

            println!("OVM:");
            println!("  Average: {}µs", ovm_avg.as_micros());
            println!("  Min:     {}µs", ovm_min.as_micros());
            println!("  Max:     {}µs", ovm_max.as_micros());

            let speedup = classic_avg.as_secs_f64() / ovm_avg.as_secs_f64();
            let best_speedup = classic_min.as_secs_f64() / ovm_min.as_secs_f64();

            println!("Performance:");
            println!("  Average speedup: {:.2}x", speedup);
            println!("  Best speedup:    {:.2}x", best_speedup);

            if speedup > 1.0 {
                println!("OVM is faster for this workload");
            } else {
                println!("Classic interpreter is faster for this workload");
            }
        } else {
            println!("OVM is not available for this workload");
        }

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        Ok(())
    }

    fn print_statistics(&self, interpreter: &OvmInterpreter) {
        let stats = interpreter.get_stats();

        println!("\nExecution Statistics:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  Classic executions:   {}", stats.classic_executions);
        println!("  OVM executions:       {}", stats.ovm_executions);

        if stats.fallback_executions > 0 {
            println!("  Fallback executions:  {}", stats.fallback_executions);
        }

        if stats.compilation_count > 0 {
            println!("  Functions compiled:   {}", stats.compilation_count);
        }

        if stats.classic_executions > 0 {
            println!(
                "  Avg classic time:     {:.2}ms",
                stats.average_classic_time_ms
            );
        }

        if stats.ovm_executions > 0 {
            println!("  Avg OVM time:         {:.2}ms", stats.average_ovm_time_ms);
        }

        if stats.classic_executions > 0 && stats.ovm_executions > 0 {
            let speedup = stats.average_classic_time_ms / stats.average_ovm_time_ms;
            println!("  Performance ratio:    {:.2}x", speedup);
        }

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }
}

//! Enhanced REPL with OVM Integration
//!
//! Provides an advanced Read-Eval-Print-Loop with OVM support,
//! performance monitoring, and execution mode switching.

use crate::ovm::OvmConfig;
use crate::ovm_integration::{IntegrationConfig, IntegrationError, OvmInterpreter};
use crate::parser::Parser;
use std::io::{self, Write};

/// Enhanced REPL with OVM integration capabilities
pub struct OvmRepl {
    /// The integrated interpreter
    interpreter: OvmInterpreter,

    /// Parser for input processing
    parser: Parser,

    /// Current execution mode
    execution_mode: ExecutionMode,

    /// Performance monitoring enabled
    performance_monitoring: bool,

    /// Command history
    history: Vec<String>,

    /// Show detailed execution statistics
    show_detailed_stats: bool,
}

/// Available execution modes
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExecutionMode {
    /// Automatic mode - let the system decide
    Auto,
    /// Force classic interpreter
    Classic,
    /// Force OVM execution
    Ovm,
    /// Performance comparison mode
    Benchmark,
}

impl OvmRepl {
    /// Create a new OVM REPL with default configuration
    pub fn new() -> Self {
        Self {
            interpreter: OvmInterpreter::new(),
            parser: Parser::new(),
            execution_mode: ExecutionMode::Auto,
            performance_monitoring: true,
            history: Vec::new(),
            show_detailed_stats: false,
        }
    }

    /// Create with custom integration configuration
    pub fn with_config(integration_config: IntegrationConfig) -> Self {
        Self {
            interpreter: OvmInterpreter::with_config(integration_config),
            parser: Parser::new(),
            execution_mode: ExecutionMode::Auto,
            performance_monitoring: true,
            history: Vec::new(),
            show_detailed_stats: false,
        }
    }

    /// Initialize OVM with default configuration
    pub fn initialize_ovm(&mut self) -> Result<(), IntegrationError> {
        self.interpreter.initialize_ovm_default()
    }

    /// Initialize OVM with custom configuration
    pub fn initialize_ovm_with_config(
        &mut self,
        ovm_config: OvmConfig,
    ) -> Result<(), IntegrationError> {
        self.interpreter.initialize_ovm(ovm_config)
    }

    /// Start the REPL
    pub fn run(&mut self) -> io::Result<()> {
        self.print_welcome();

        loop {
            print!("olang> ");
            io::stdout().flush()?;

            let mut input = String::new();
            match io::stdin().read_line(&mut input) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let input = input.trim();
                    if input.is_empty() {
                        continue;
                    }

                    // Handle special commands
                    if input.starts_with(':') {
                        self.handle_command(input);
                        continue;
                    }

                    // Add to history
                    self.history.push(input.to_string());

                    // Parse and evaluate
                    self.evaluate_input(input);
                }
                Err(error) => {
                    crate::log_error!("ovm_repl", "Error reading input: {}", error);
                    break;
                }
            }
        }

        self.print_goodbye();
        Ok(())
    }

    /// Set execution mode
    pub fn set_execution_mode(&mut self, mode: ExecutionMode) {
        self.execution_mode = mode;
                    crate::log_info!("ovm_repl", "Execution mode set to: {:?}", mode);
    }

    /// Enable/disable performance monitoring
    pub fn set_performance_monitoring(&mut self, enabled: bool) {
        self.performance_monitoring = enabled;
        println!(
            "Performance monitoring: {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    /// Enable/disable detailed statistics
    pub fn set_detailed_stats(&mut self, enabled: bool) {
        self.show_detailed_stats = enabled;
        println!(
            "Detailed statistics: {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    // Private methods

    fn print_welcome(&self) {
        println!("Olang Enhanced REPL with OVM Integration");
        println!("Type ':help' for commands, ':quit' to exit");

        if self.interpreter.is_ovm_available() {
            println!("✓ OVM is available and ready");
        } else {
            println!("⚠ OVM not initialized - using classic interpreter");
            println!("  Use ':init-ovm' to enable high-performance execution");
        }

        println!("Execution mode: {:?}", self.execution_mode);
        println!();
    }

    fn print_goodbye(&self) {
        println!("\nGoodbye!");

        if self.performance_monitoring {
            let stats = self.interpreter.get_stats();
            println!("\nSession Statistics:");
            println!("  Classic executions: {}", stats.classic_executions);
            println!("  OVM executions: {}", stats.ovm_executions);
            if stats.fallback_executions > 0 {
                println!("  Fallback executions: {}", stats.fallback_executions);
            }
            if stats.compilation_count > 0 {
                println!("  Functions compiled: {}", stats.compilation_count);
            }

            if stats.classic_executions > 0 && stats.ovm_executions > 0 {
                println!(
                    "  Average classic time: {:.2}ms",
                    stats.average_classic_time_ms
                );
                println!("  Average OVM time: {:.2}ms", stats.average_ovm_time_ms);

                let speedup = stats.average_classic_time_ms / stats.average_ovm_time_ms;
                if speedup > 1.0 {
                    println!("  OVM speedup: {:.2}x", speedup);
                }
            }
        }
    }

    fn handle_command(&mut self, command: &str) {
        let parts: Vec<&str> = command[1..].split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        match parts[0] {
            "help" => self.print_help(),
            "quit" | "exit" => std::process::exit(0),
            "init-ovm" => self.init_ovm_command(),
            "mode" => self.handle_mode_command(&parts[1..]),
            "stats" => self.print_stats(),
            "detailed-stats" => self.toggle_detailed_stats(),
            "gc" => self.force_gc(),
            "history" => self.print_history(),
            "clear" => self.clear_screen(),
            "performance" => self.toggle_performance_monitoring(),
            "benchmark" => self.run_benchmark(&parts[1..]),
            _ => println!("Unknown command: {}", parts[0]),
        }
    }

    fn print_help(&self) {
        println!("Available commands:");
        println!("  :help              - Show this help message");
        println!("  :quit, :exit       - Exit the REPL");
        println!("  :init-ovm          - Initialize OVM with default config");
        println!("  :mode <mode>       - Set execution mode (auto|classic|ovm|benchmark)");
        println!("  :stats             - Show execution statistics");
        println!("  :detailed-stats    - Toggle detailed statistics");
        println!("  :gc                - Force garbage collection");
        println!("  :history           - Show command history");
        println!("  :clear             - Clear screen");
        println!("  :performance       - Toggle performance monitoring");
        println!("  :benchmark <expr>  - Benchmark expression in all modes");
        println!();
        println!("Current configuration:");
        println!("  Execution mode: {:?}", self.execution_mode);
        println!("  OVM available: {}", self.interpreter.is_ovm_available());
        println!("  Performance monitoring: {}", self.performance_monitoring);
    }

    fn init_ovm_command(&mut self) {
        match self.interpreter.initialize_ovm_default() {
            Ok(()) => println!("✓ OVM initialized successfully"),
            Err(e) => println!("✗ Failed to initialize OVM: {}", e),
        }
    }

    fn handle_mode_command(&mut self, args: &[&str]) {
        if args.is_empty() {
            println!("Current execution mode: {:?}", self.execution_mode);
            return;
        }

        match args[0] {
            "auto" => self.set_execution_mode(ExecutionMode::Auto),
            "classic" => self.set_execution_mode(ExecutionMode::Classic),
            "ovm" => self.set_execution_mode(ExecutionMode::Ovm),
            "benchmark" => self.set_execution_mode(ExecutionMode::Benchmark),
            _ => println!("Invalid mode. Use: auto, classic, ovm, or benchmark"),
        }
    }

    fn print_stats(&self) {
        let stats = self.interpreter.get_stats();

        println!("Execution Statistics:");
        println!("  Classic executions: {}", stats.classic_executions);
        println!("  OVM executions: {}", stats.ovm_executions);

        if stats.fallback_executions > 0 {
            println!("  Fallback executions: {}", stats.fallback_executions);
        }

        if stats.compilation_count > 0 {
            println!("  Functions compiled: {}", stats.compilation_count);
        }

        if self.show_detailed_stats {
            if stats.classic_executions > 0 {
                println!(
                    "  Average classic time: {:.2}ms",
                    stats.average_classic_time_ms
                );
            }
            if stats.ovm_executions > 0 {
                println!("  Average OVM time: {:.2}ms", stats.average_ovm_time_ms);
            }

            if stats.classic_executions > 0 && stats.ovm_executions > 0 {
                let speedup = stats.average_classic_time_ms / stats.average_ovm_time_ms;
                println!("  Performance ratio: {:.2}x", speedup);
            }
        }
    }

    fn toggle_detailed_stats(&mut self) {
        self.show_detailed_stats = !self.show_detailed_stats;
        self.set_detailed_stats(self.show_detailed_stats);
    }

    fn force_gc(&mut self) {
        match self.interpreter.force_gc() {
            Ok(()) => println!("✓ Garbage collection completed"),
            Err(e) => println!("✗ GC failed: {}", e),
        }
    }

    fn print_history(&self) {
        if self.history.is_empty() {
            println!("No command history");
            return;
        }

        println!("Command history:");
        for (i, cmd) in self.history.iter().enumerate() {
            println!("  {}: {}", i + 1, cmd);
        }
    }

    fn clear_screen(&self) {
        print!("\x1B[2J\x1B[1;1H");
        if let Err(e) = io::stdout().flush() {
            crate::log_warn!("ovm_repl", "Failed to flush stdout: {}", e);
        }
    }

    fn toggle_performance_monitoring(&mut self) {
        self.performance_monitoring = !self.performance_monitoring;
        self.set_performance_monitoring(self.performance_monitoring);
    }

    fn run_benchmark(&mut self, args: &[&str]) {
        if args.is_empty() {
            println!("Usage: :benchmark <expression>");
            return;
        }

        let expr = args.join(" ");
        println!("Benchmarking: {}", expr);

        // Parse once
        let program = match self.parser.parse(&expr) {
            Ok(program) => program,
            Err(e) => {
                println!("Parse error: {:?}", e);
                return;
            }
        };

        // Benchmark classic
        let start = std::time::Instant::now();
        match self.interpreter.eval_program_classic(program.clone()) {
            Ok(result) => {
                let classic_time = start.elapsed();
                println!("Classic: {:?} ({}µs)", result, classic_time.as_micros());

                // Benchmark OVM if available
                if self.interpreter.is_ovm_available() {
                    let start = std::time::Instant::now();
                    match self.interpreter.eval_program_ovm(program) {
                        Ok(ovm_result) => {
                            let ovm_time = start.elapsed();
                            println!("OVM: {:?} ({}µs)", ovm_result, ovm_time.as_micros());

                            let speedup = classic_time.as_secs_f64() / ovm_time.as_secs_f64();
                            println!("Speedup: {:.2}x", speedup);
                        }
                        Err(e) => println!("OVM error: {}", e),
                    }
                } else {
                    println!("OVM not available for comparison");
                }
            }
            Err(e) => println!("Classic error: {}", e),
        }
    }

    fn evaluate_input(&mut self, input: &str) {
        // Parse the input
        let program = match self.parser.parse(input) {
            Ok(program) => program,
            Err(e) => {
                println!("Parse error: {:?}", e);
                return;
            }
        };

        let start_time = std::time::Instant::now();

        // Execute based on mode
        let result = match self.execution_mode {
            ExecutionMode::Auto => self.interpreter.eval_program(program),
            ExecutionMode::Classic => self.interpreter.eval_program_classic(program),
            ExecutionMode::Ovm => {
                if self.interpreter.is_ovm_available() {
                    self.interpreter.eval_program_ovm(program)
                } else {
                    println!("OVM not available, falling back to classic");
                    self.interpreter.eval_program_classic(program)
                }
            }
            ExecutionMode::Benchmark => {
                self.run_benchmark(&[input]);
                return;
            }
        };

        let execution_time = start_time.elapsed();

        // Display result
        match result {
            Ok(value) => {
                println!("{:?}", value);

                if self.performance_monitoring && self.show_detailed_stats {
                    println!("(executed in {}µs)", execution_time.as_micros());
                }
            }
            Err(e) => println!("Error: {}", e),
        }
    }
}

impl Default for OvmRepl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ovm_repl_creation() {
        let repl = OvmRepl::new();
        assert_eq!(repl.execution_mode, ExecutionMode::Auto);
        assert!(repl.performance_monitoring);
    }

    #[test]
    fn test_execution_mode_switching() {
        let mut repl = OvmRepl::new();

        repl.set_execution_mode(ExecutionMode::Classic);
        assert_eq!(repl.execution_mode, ExecutionMode::Classic);

        repl.set_execution_mode(ExecutionMode::Ovm);
        assert_eq!(repl.execution_mode, ExecutionMode::Ovm);
    }
}

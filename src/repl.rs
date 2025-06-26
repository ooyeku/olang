use crate::ast::Value;
use crate::help::HelpSystem;
use crate::interpreter::InterpreterError;
use crate::ovm_integration::{IntegrationConfig, IntegrationError, OvmInterpreter};
use crate::parser::{ParseError, Parser};
use crate::version::VERSION;
use colored::*;
use rustyline::error::ReadlineError;
use rustyline::{history::DefaultHistory, Config, Editor};
use std::time::Instant;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ReplError {
    #[error("Readline error: {0}")]
    Readline(#[from] ReadlineError),
    #[error("Parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("Interpreter error: {0}")]
    Interpreter(#[from] InterpreterError),
    #[error("Integration error: {0}")]
    Integration(#[from] IntegrationError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct ReplConfig {
    pub prompt: String,
    pub show_types: bool,
    pub auto_save: bool,
    pub max_history: usize,
    pub debug_mode: bool,
    pub time_commands: bool,
}

impl Default for ReplConfig {
    fn default() -> Self {
        Self {
            prompt: "olang> ".to_string(),
            show_types: false,
            auto_save: true,
            max_history: 1000,
            debug_mode: false,
            time_commands: false,
        }
    }
}

pub struct Repl {
    editor: Editor<(), DefaultHistory>,
    ovm_interpreter: OvmInterpreter,
    parser: Parser,
    verbose: bool,
    history_file: String,
    help_system: HelpSystem,
    config: ReplConfig,
    multiline_mode: bool,
    multiline_buffer: String,
    command_history: Vec<String>,
    ovm_health_check_counter: u32,
}

impl Repl {
    pub fn new(verbose: bool) -> Result<Self, ReplError> {
        // Initialize parallelization for optimal performance
        if let Err(e) = crate::parallel::initialize_parallelization(None) {
            if verbose {
                eprintln!("Warning: Failed to initialize parallel processing: {}", e);
            }
        } else if verbose {
            println!("✅ Multi-threading enabled: {} CPU cores detected", num_cpus::get());
        }

        // Set a very aggressive parallel threshold for maximum multi-threading by default
        crate::parallel::set_parallel_threshold(10);
        
        if verbose {
            println!("🚀 Automatic parallelization: Lists with 10+ items will use all {} cores", num_cpus::get());
        }

        let config = Config::builder()
            .auto_add_history(true)
            .history_ignore_space(true)
            .build();

        let mut editor = Editor::<(), DefaultHistory>::with_config(config)?;

        // Load history if available
        let history_file = format!(
            "{}/.olang_history",
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        );
        if let Err(e) = editor.load_history(&history_file) {
            if verbose {
                eprintln!("Could not load history: {}", e);
            }
        }

        // Create OVM configuration with auto mode (OVM by default)
        let integration_config = IntegrationConfig {
            use_ovm_by_default: true,
            ovm_complexity_threshold: 50,
            auto_compile_functions: true,
            enable_ovm_lazy_eval: true,
            fallback_on_error: true,
        };

        let mut ovm_interpreter = OvmInterpreter::with_config(integration_config);

        // Initialize OVM with default configuration
        if let Err(e) = ovm_interpreter.initialize_ovm_default() {
            if verbose {
                eprintln!(
                    "OVM initialization failed, falling back to classic interpreter: {}",
                    e
                );
            }
        } else if verbose {
            println!("OVM initialized successfully - enhanced performance enabled");
        }

        Ok(Self {
            editor,
            ovm_interpreter,
            parser: Parser::new(),
            verbose,
            history_file,
            help_system: HelpSystem::new(),
            config: ReplConfig::default(),
            multiline_mode: false,
            multiline_buffer: String::new(),
            command_history: Vec::new(),
            ovm_health_check_counter: 0,
        })
    }

    pub fn run(&mut self) -> Result<(), ReplError> {
        println!("Olang v{} - A minimal, expressive language", VERSION);
        
        // Display OVM status with enhanced messaging
        if self.ovm_interpreter.is_ovm_available() {
            let stats = self.ovm_interpreter.get_stats();
            println!("{}", "🚀 OVM (Olang Virtual Machine) is running in the background".bright_green());
            println!("{}", "   • Advanced pipeline fusion optimization active".bright_cyan());
            println!("{}", "   • Automatic SIMD vectorization enabled".bright_cyan());
            println!("{}", "   • Garbage collection and memory optimization active".bright_cyan());
            println!("{}", "   • Your code is being optimized automatically!".bright_white());
            
            if stats.ovm_executions > 0 || stats.classic_executions > 0 {
                println!("   • Previous session: {} total executions", 
                    (stats.ovm_executions + stats.classic_executions).to_string().bright_white());
            }
        } else {
            println!("{}", "⚠️  OVM not available - using classic interpreter".bright_yellow());
        }
        
        println!();
        println!("Type 'help' for help, ':ovm status' for OVM details, ':env' to see environment, 'quit' to exit");
        println!();

        loop {
            let prompt = if self.multiline_mode {
                "...> "
            } else {
                &self.config.prompt
            };

            let line = match self.editor.readline(prompt) {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => {
                    println!("^C");
                    if self.multiline_mode {
                        self.multiline_mode = false;
                        self.multiline_buffer.clear();
                    }
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    println!("^D");
                    break;
                }
                Err(err) => {
                    eprintln!("Error: {}", err);
                    break;
                }
            };

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Handle multiline mode
            if self.multiline_mode {
                if line == ":end" {
                    self.multiline_mode = false;
                    let code = self.multiline_buffer.clone();
                    self.multiline_buffer.clear();

                    // Add to command history
                    self.command_history.push(code.clone());

                    match self.eval_line(&code) {
                        Ok(value) => {
                            if value != Value::Unit {
                                if self.config.show_types {
                                    println!("{} : {}", value, Self::get_type_name(&value));
                                } else {
                                    println!("{}", value);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            if self.verbose {
                                eprintln!("Debug: {:?}", e);
                            }
                        }
                    }
                } else {
                    if !self.multiline_buffer.is_empty() {
                        self.multiline_buffer.push('\n');
                    }
                    self.multiline_buffer.push_str(line);
                }
                continue;
            }

            // Handle special commands
            if line.starts_with(':') || line == "help" || line == "quit" {
                if let Err(e) = self.handle_command(line) {
                    eprintln!("Command error: {}", e);
                }
                continue;
            }

            // Check for incomplete expressions (auto-multiline)
            if self.is_incomplete_expression(line) {
                self.multiline_mode = true;
                self.multiline_buffer = line.to_string();
                continue;
            }

            // Add to command history
            self.command_history.push(line.to_string());

            // Evaluate the expression
            let start_time = if self.config.time_commands {
                Some(Instant::now())
            } else {
                None
            };

            match self.eval_line(line) {
                Ok(value) => {
                    if let Some(start) = start_time {
                        let duration = start.elapsed();
                        if value != Value::Unit {
                            if self.config.show_types {
                                println!("{} : {}", value, Self::get_type_name(&value));
                            } else {
                                println!("{}", value);
                            }
                        }
                        println!("Execution time: {:.2}ms", duration.as_secs_f64() * 1000.0);
                    } else if value != Value::Unit {
                        if self.config.show_types {
                            println!("{} : {}", value, Self::get_type_name(&value));
                        } else {
                            println!("{}", value);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    if self.verbose || self.config.debug_mode {
                        eprintln!("Debug: {:?}", e);
                    }
                }
            }
        }

        // Save history
        if let Err(e) = self.editor.save_history(&self.history_file) {
            eprintln!("Could not save history: {}", e);
        }

        Ok(())
    }

    fn handle_command(&mut self, command: &str) -> Result<(), ReplError> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        let command_name = parts[0];

        match command_name {
            "help" | ":help" => {
                if parts.len() == 1 {
                    println!("{}", self.help_system.show_overview());
                } else {
                    let topic = parts[1];
                    match topic {
                        "list" => println!("{}", self.help_system.list_functions()),
                        "examples" => println!("{}", self.help_system.show_examples()),
                        "syntax" => println!("{}", self.help_system.show_syntax()),
                        _ => {
                            if self.help_system.has_function(topic) {
                                println!("{}", self.help_system.show_function_help(topic));
                            } else if self.help_system.has_category(topic) {
                                println!("{}", self.help_system.show_category(topic));
                            } else {
                                println!("{}", self.help_system.show_function_help(topic));
                            }
                        }
                    }
                }
            }
            "quit" | ":quit" => {
                std::process::exit(0);
            }
            ":env" => {
                if parts.len() > 1 && parts[1] == "--full" {
                    self.show_environment(true);
                } else {
                    self.show_environment(false);
                }
            }
            ":clear" => {
                if parts.len() > 1 {
                    match parts[1] {
                        "env" => {
                            self.ovm_interpreter
                                .get_classic_interpreter()
                                .clear_user_environment();
                            println!("User environment cleared.");
                        }
                        "history" => {
                            self.command_history.clear();
                            self.editor.clear_history()?;
                            println!("Command history cleared.");
                        }
                        _ => {
                            println!("Usage: :clear [env|history]");
                        }
                    }
                } else {
                    print!("\x1B[2J\x1B[1;1H");
                }
            }
            ":ovm" => {
                if parts.len() > 1 {
                    match parts[1] {
                        "status" => {
                            println!("=== OVM Status ===");
                            let ovm_status = self.ovm_interpreter.get_ovm_status();
                            println!("  Initialized: {}", ovm_status.initialized.to_string().bright_green());
                            println!("  Running: {}", ovm_status.running.to_string().bright_green());
                            println!("  Healthy: {}", self.ovm_interpreter.is_ovm_healthy().to_string().bright_green());
                            
                            println!("\n=== Execution Statistics ===");
                            let stats = self.ovm_interpreter.get_stats();
                            let total_executions = stats.ovm_executions + stats.classic_executions;
                            
                            println!("  Total executions: {}", total_executions.to_string().bright_white());
                            if total_executions > 0 {
                                let ovm_percentage = (stats.ovm_executions as f64 / total_executions as f64) * 100.0;
                                let classic_percentage = (stats.classic_executions as f64 / total_executions as f64) * 100.0;
                                
                                println!("  OVM executions: {} ({:.1}%)", 
                                    stats.ovm_executions.to_string().bright_cyan(), 
                                    ovm_percentage.to_string().bright_cyan());
                                println!("  Classic executions: {} ({:.1}%)", 
                                    stats.classic_executions.to_string().bright_yellow(), 
                                    classic_percentage.to_string().bright_yellow());
                            } else {
                                println!("  OVM executions: {}", stats.ovm_executions.to_string().bright_cyan());
                                println!("  Classic executions: {}", stats.classic_executions.to_string().bright_yellow());
                            }
                            println!("  Fallback executions: {}", stats.fallback_executions.to_string().bright_red());
                            println!("  Function compilations: {}", stats.compilation_count.to_string().bright_blue());
                            
                            if stats.average_ovm_time_ms > 0.0 {
                                println!("  Avg OVM time: {:.2}ms", stats.average_ovm_time_ms);
                            }
                            if stats.average_classic_time_ms > 0.0 {
                                println!("  Avg classic time: {:.2}ms", stats.average_classic_time_ms);
                            }
                            
                            if ovm_status.running {
                                println!("\n{}", "✅ OVM is actively optimizing your code in the background".bright_green());
                            } else {
                                println!("\n{}", "⚠️  OVM is not running - falling back to classic interpreter".bright_yellow());
                            }
                        }
                        "gc" => {
                            if let Err(e) = self.ovm_interpreter.force_gc() {
                                eprintln!("GC failed: {}", e);
                            } else {
                                println!("✅ Garbage collection completed successfully");
                            }
                        }
                        "restart" => {
                            match self.ovm_interpreter.ensure_ovm_running() {
                                Ok(()) => println!("✅ OVM restarted successfully"),
                                Err(e) => eprintln!("Failed to restart OVM: {}", e),
                            }
                        }
                        _ => {
                            println!("Usage: :ovm [status|gc|restart]");
                        }
                    }
                } else {
                    let available = self.ovm_interpreter.is_ovm_available();
                    let healthy = self.ovm_interpreter.is_ovm_healthy();
                    println!("OVM available: {} | Healthy: {}", 
                        available.to_string().bright_green(), 
                        healthy.to_string().bright_green());
                    if available && healthy {
                        println!("{}", "✅ OVM is running in the background".bright_green());
                    } else {
                        println!("{}", "⚠️  OVM may need attention - use ':ovm status' for details".bright_yellow());
                    }
                }
            }
            ":version" | "version" => {
                println!("Olang v{}", VERSION);
            }
            ":history" => {
                if parts.len() > 1 {
                    if parts[1] == "search" && parts.len() > 2 {
                        let pattern = parts[2];
                        println!("Command history search for '{}':", pattern);
                        for (i, cmd) in self.command_history.iter().enumerate() {
                            if cmd.contains(pattern) {
                                println!("  {}: {}", i + 1, cmd);
                            }
                        }
                    } else if let Ok(count) = parts[1].parse::<usize>() {
                        println!("Last {} commands:", count);
                        let start = self.command_history.len().saturating_sub(count);
                        for (i, cmd) in self.command_history.iter().enumerate().skip(start) {
                            println!("  {}: {}", i + 1, cmd);
                        }
                    } else {
                        println!("Usage: :history [<count>|search <pattern>]");
                    }
                } else {
                    println!("Command history:");
                    for (i, cmd) in self.command_history.iter().enumerate() {
                        println!("  {}: {}", i + 1, cmd);
                    }
                }
            }
            ":type" => {
                if parts.len() > 1 {
                    let expr = parts[1..].join(" ");
                    match self.parser.parse(&expr) {
                        Ok(program) => {
                            if let Some(stmt) = program.statements.first() {
                                // For now, just evaluate and show the type of the result
                                match self.eval_line(&expr) {
                                    Ok(value) => {
                                        println!("{} : {}", expr, Self::get_type_name(&value));
                                    }
                                    Err(e) => {
                                        println!("Type check failed: {}", e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            println!("Parse error: {}", e);
                        }
                    }
                } else {
                    println!("Usage: :type <expression>");
                }
            }
            ":time" => {
                if parts.len() > 1 {
                    let expr = parts[1..].join(" ");
                    let start = Instant::now();
                    match self.eval_line(&expr) {
                        Ok(value) => {
                            let duration = start.elapsed();
                            if value != Value::Unit {
                                println!("{}", value);
                            }
                            println!("Execution time: {:.2}ms", duration.as_secs_f64() * 1000.0);
                        }
                        Err(e) => {
                            println!("Error: {}", e);
                        }
                    }
                } else {
                    println!("Usage: :time <expression>");
                }
            }
            ":memory" => {
                println!("Memory Usage Statistics:");
                println!("  Command history entries: {}", self.command_history.len());

                let user_vars = self
                    .ovm_interpreter
                    .get_classic_interpreter()
                    .get_user_variables();
                println!("  User variables: {}", user_vars.len());

                if self.ovm_interpreter.is_ovm_available() {
                    let stats = self.ovm_interpreter.get_stats();
                    println!("  OVM executions: {}", stats.ovm_executions);
                    println!("  Classic executions: {}", stats.classic_executions);
                    println!("  Fallback executions: {}", stats.fallback_executions);
                }

                // Try to get system memory info (simplified)
                println!("  Note: Detailed memory profiling requires OVM performance monitoring");
            }
            ":stats" => {
                println!("Execution Statistics:");

                if self.ovm_interpreter.is_ovm_available() {
                    let stats = self.ovm_interpreter.get_stats();
                    let total_executions = stats.ovm_executions + stats.classic_executions;

                    println!("  Total expressions executed: {}", total_executions);

                    if total_executions > 0 {
                        let ovm_percentage =
                            (stats.ovm_executions as f64 / total_executions as f64) * 100.0;
                        let classic_percentage =
                            (stats.classic_executions as f64 / total_executions as f64) * 100.0;

                        println!(
                            "  OVM executions: {} ({:.1}%)",
                            stats.ovm_executions, ovm_percentage
                        );
                        println!(
                            "  Classic interpreter executions: {} ({:.1}%)",
                            stats.classic_executions, classic_percentage
                        );
                    } else {
                        println!("  OVM executions: {}", stats.ovm_executions);
                        println!(
                            "  Classic interpreter executions: {}",
                            stats.classic_executions
                        );
                    }

                    println!("  Fallback executions: {}", stats.fallback_executions);

                    if stats.average_ovm_time_ms > 0.0 {
                        println!(
                            "  Average OVM execution time: {:.2}ms",
                            stats.average_ovm_time_ms
                        );
                    }
                    if stats.average_classic_time_ms > 0.0 {
                        println!(
                            "  Average classic execution time: {:.2}ms",
                            stats.average_classic_time_ms
                        );
                    }

                    println!("  Performance: OVM is actively optimizing your code");
                } else {
                    println!("  Classic interpreter mode");
                    println!("  Note: Enable OVM for detailed performance statistics");
                }
            }
            ":parallel" => {
                if parts.len() > 1 {
                    match parts[1] {
                        "status" => {
                            let config = crate::parallel::get_config();
                            println!("=== Parallel Processing Status ===");
                            println!("  Enabled: {}", config.enabled.to_string().bright_green());
                            println!("  Max threads: {}", config.max_threads.to_string().bright_cyan());
                            println!("  Parallel threshold: {} items", config.min_parallel_size.to_string().bright_yellow());
                            println!("  Available CPU cores: {}", num_cpus::get().to_string().bright_white());
                            
                            // Test parallel processing
                            let large_list: Vec<usize> = (1..=config.min_parallel_size + 100).collect();
                            println!("  Test: List of {} items would use {} processing", 
                                large_list.len(),
                                if crate::parallel::should_parallelize(large_list.len()) { 
                                    "PARALLEL".bright_green() 
                                } else { 
                                    "SEQUENTIAL".bright_red() 
                                }
                            );
                        }
                        "enable" => {
                            crate::parallel::set_parallel_enabled(true);
                            println!("✅ Parallel processing enabled");
                        }
                        "disable" => {
                            crate::parallel::set_parallel_enabled(false);
                            println!("⚠️  Parallel processing disabled");
                        }
                        "threshold" => {
                            if parts.len() > 2 {
                                if let Ok(threshold) = parts[2].parse::<usize>() {
                                    crate::parallel::set_parallel_threshold(threshold);
                                    println!("✅ Parallel threshold set to {} items", threshold);
                                } else {
                                    println!("Error: Invalid threshold value");
                                }
                            } else {
                                let config = crate::parallel::get_config();
                                println!("Current parallel threshold: {} items", config.min_parallel_size);
                                println!("Usage: :parallel threshold <number>");
                            }
                        }
                        _ => {
                            println!("Usage: :parallel [status|enable|disable|threshold <number>]");
                        }
                    }
                } else {
                    let config = crate::parallel::get_config();
                    println!("Parallel processing: {} | Threads: {} | Threshold: {} items", 
                        if config.enabled { "enabled".bright_green() } else { "disabled".bright_red() },
                        config.max_threads.to_string().bright_cyan(),
                        config.min_parallel_size.to_string().bright_yellow());
                }
            }
            ":run" => {
                if parts.len() > 1 {
                    let filename = parts[1];
                    match std::fs::read_to_string(filename) {
                        Ok(content) => match self.eval_line(&content) {
                            Ok(value) => {
                                if value != Value::Unit {
                                    println!("{}", value);
                                }
                                println!("File '{}' executed successfully", filename);
                            }
                            Err(e) => {
                                eprintln!("Error executing '{}': {}", filename, e);
                            }
                        },
                        Err(e) => {
                            eprintln!("Error reading file '{}': {}", filename, e);
                        }
                    }
                } else {
                    println!("Usage: :run <filename>");
                }
            }
            ":debug" => {
                if parts.len() > 1 {
                    match parts[1] {
                        "on" => {
                            self.config.debug_mode = true;
                            println!("Debug mode enabled");
                        }
                        "off" => {
                            self.config.debug_mode = false;
                            println!("Debug mode disabled");
                        }
                        _ => {
                            println!("Usage: :debug [on|off]");
                        }
                    }
                } else {
                    println!(
                        "Debug mode: {}",
                        if self.config.debug_mode { "on" } else { "off" }
                    );
                }
            }
            ":config" => {
                if parts.len() > 2 {
                    let setting = parts[1];
                    let value = parts[2];
                    match setting {
                        "prompt" => {
                            self.config.prompt = value.to_string();
                            println!("Prompt set to: {}", value);
                        }
                        "show_types" => {
                            self.config.show_types = value.parse().unwrap_or(false);
                            println!("Show types: {}", self.config.show_types);
                        }
                        "debug_mode" => {
                            self.config.debug_mode = value.parse().unwrap_or(false);
                            println!("Debug mode: {}", self.config.debug_mode);
                        }
                        "time_commands" => {
                            self.config.time_commands = value.parse().unwrap_or(false);
                            println!("Time commands: {}", self.config.time_commands);
                        }
                        _ => {
                            println!("Unknown setting: {}", setting);
                        }
                    }
                } else {
                    println!("Current configuration:");
                    println!("  prompt: {}", self.config.prompt);
                    println!("  show_types: {}", self.config.show_types);
                    println!("  debug_mode: {}", self.config.debug_mode);
                    println!("  time_commands: {}", self.config.time_commands);
                    println!("  max_history: {}", self.config.max_history);
                }
            }
            ":benchmark" => {
                if parts.len() > 1 {
                    let expr = parts[1..].join(" ");
                    println!("Benchmarking: {}", expr);

                    let iterations = 5;
                    let mut times = Vec::new();

                    for i in 1..=iterations {
                        let start = Instant::now();
                        match self.eval_line(&expr) {
                            Ok(_) => {
                                let duration = start.elapsed();
                                times.push(duration);
                                println!("  Run {}: {:.2}ms", i, duration.as_secs_f64() * 1000.0);
                            }
                            Err(e) => {
                                println!("  Run {} failed: {}", i, e);
                                return Ok(());
                            }
                        }
                    }

                    if !times.is_empty() {
                        let total: std::time::Duration = times.iter().sum();
                        let avg = total / times.len() as u32;
                        let min = times.iter().min().unwrap();
                        let max = times.iter().max().unwrap();

                        println!("Benchmark results:");
                        println!("  Average: {:.2}ms", avg.as_secs_f64() * 1000.0);
                        println!("  Min: {:.2}ms", min.as_secs_f64() * 1000.0);
                        println!("  Max: {:.2}ms", max.as_secs_f64() * 1000.0);
                    }
                } else {
                    println!("Usage: :benchmark <expression>");
                }
            }
            ":search" => {
                if parts.len() > 1 {
                    let query = parts[1];
                    println!("Searching for functions matching '{}':", query);

                    let builtins = self
                        .ovm_interpreter
                        .get_classic_interpreter()
                        .get_builtin_functions();
                    let mut found = false;

                    for name in builtins.keys() {
                        if name.contains(query) && self.help_system.has_function(name) {
                            println!("  {} - Built-in function", name.bright_blue());
                            found = true;
                        }
                    }

                    if !found {
                        println!("  No functions found matching '{}'", query);
                    }
                } else {
                    println!("Usage: :search <query>");
                }
            }
            ":examples" => {
                if parts.len() > 1 {
                    let function_name = parts[1];
                    if self.help_system.has_function(function_name) {
                        println!("{}", self.help_system.show_function_help(function_name));
                    } else {
                        println!("No examples found for function: {}", function_name);
                    }
                } else {
                    println!("Usage: :examples <function_name>");
                }
            }
            cmd if cmd.starts_with(":!") => {
                if let Ok(num) = cmd[2..].parse::<usize>() {
                    if num > 0 && num <= self.command_history.len() {
                        let command = self.command_history[num - 1].clone();
                        println!("Re-executing: {}", command);
                        match self.eval_line(&command) {
                            Ok(value) => {
                                if value != Value::Unit {
                                    println!("{}", value);
                                }
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e);
                            }
                        }
                    } else {
                        println!("Invalid history number: {}", num);
                    }
                } else {
                    println!("Usage: :!<number>");
                }
            }
            _ => {
                return Err(ReplError::Parse(ParseError::InvalidSyntax {
                    message: format!("Unknown command: {}", command),
                }));
            }
        }
        Ok(())
    }

    fn eval_line(&mut self, line: &str) -> Result<Value, ReplError> {
        let program = self.parser.parse(line)?;

        if program.statements.is_empty() {
            return Ok(Value::Unit);
        }

        // Periodic OVM health check (every 10 evaluations)
        self.ovm_health_check_counter += 1;
        if self.ovm_health_check_counter % 10 == 0 {
            if let Err(e) = self.ovm_interpreter.ensure_ovm_running() {
                if self.verbose {
                    eprintln!("Warning: OVM health check failed: {}", e);
                }
            }
        }

        let result = self.ovm_interpreter.eval_program(program)?;
        Ok(result)
    }

    fn show_environment(&mut self, full: bool) {
        println!("\n{}", "=== Current Environment ===".bright_cyan().bold());
        println!();

        // Collect data first to avoid borrow conflicts
        let (builtin_count, builtin_names, user_var_count) = {
            let classic_interpreter = self.ovm_interpreter.get_classic_interpreter();
            let builtins = classic_interpreter.get_builtin_functions();
            let user_vars = classic_interpreter.get_user_variables();

            let mut builtin_names: Vec<String> = builtins.keys().cloned().collect();
            builtin_names.sort();

            (builtins.len(), builtin_names, user_vars.len())
        };

        // Built-in functions
        println!(
            "{} ({}):",
            "Core Functions".bright_green().bold(),
            builtin_count.to_string().bright_white()
        );

        if full {
            for name in &builtin_names {
                if self.help_system.has_function(name) {
                    let help = self.help_system.show_function_help(name);
                    let first_line = help.lines().nth(2).unwrap_or(name);
                    println!("  {} - {}", name.bright_blue(), first_line.bright_white());
                } else {
                    println!("  {}", name.bright_blue());
                }
            }
        } else {
            for chunk in builtin_names.chunks(6) {
                println!(
                    "  {}",
                    chunk
                        .iter()
                        .map(|name| name.bright_blue().to_string())
                        .collect::<Vec<_>>()
                        .join("  ")
                );
            }
            println!(
                "  {}",
                "(use :env --full for detailed descriptions)".bright_black()
            );
        }

        // User-defined variables
        println!(
            "\n{} ({}):",
            "User Environment".bright_yellow().bold(),
            user_var_count.to_string().bright_white()
        );

        if user_var_count == 0 {
            println!("  {}", "(none)".bright_black());
        } else {
            // Get user variables again (after releasing previous borrow)
            let user_vars = self
                .ovm_interpreter
                .get_classic_interpreter()
                .get_user_variables();
            for (name, value) in &user_vars {
                let type_name = Self::get_type_name(value);
                let value_str = Self::format_value_preview(value);
                println!(
                    "    {} : {} = {}",
                    name.bright_green(),
                    type_name.bright_yellow(),
                    value_str.bright_white()
                );
            }
        }

        println!();
    }

    fn get_type_name(value: &Value) -> &'static str {
        match value {
            Value::Integer(_) => "int",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Boolean(_) => "bool",
            Value::List(_) => "list",
            Value::Tuple(_) => "tuple",
            Value::Struct { .. } => "struct",
            Value::Function(_) => "function",
            Value::Builtin(_) => "builtin",
            Value::Range { .. } => "range",
            Value::Unit => "unit",
            Value::Ok(_) => "result",
            Value::Err(_) => "result",
            Value::Promise { .. } => "promise",
        }
    }

    fn format_value_preview(value: &Value) -> String {
        match value {
            Value::String(s) => {
                if s.len() > 50 {
                    format!("\"{}...\"", &s[..47])
                } else {
                    format!("\"{}\"", s)
                }
            }
            Value::List(items) => {
                if items.len() > 5 {
                    format!("[{} items]", items.len())
                } else if items.is_empty() {
                    "[]".to_string()
                } else {
                    let preview: Vec<String> =
                        items.iter().take(3).map(|v| format!("{}", v)).collect();
                    if items.len() > 3 {
                        format!("[{}, ...]", preview.join(", "))
                    } else {
                        format!("[{}]", preview.join(", "))
                    }
                }
            }
            Value::Struct { type_name, .. } => {
                format!("{} {{ ... }}", type_name)
            }
            _ => format!("{}", value),
        }
    }

    fn is_incomplete_expression(&self, line: &str) -> bool {
        let mut brace_count = 0;
        let mut paren_count = 0;
        let mut bracket_count = 0;
        let mut in_string = false;
        let mut escape_next = false;

        for ch in line.chars() {
            if escape_next {
                escape_next = false;
                continue;
            }

            match ch {
                '"' if !in_string => in_string = true,
                '"' if in_string => in_string = false,
                '\\' if in_string => escape_next = true,
                '{' if !in_string => brace_count += 1,
                '}' if !in_string => brace_count -= 1,
                '(' if !in_string => paren_count += 1,
                ')' if !in_string => paren_count -= 1,
                '[' if !in_string => bracket_count += 1,
                ']' if !in_string => bracket_count -= 1,
                _ => {}
            }
        }

        in_string || brace_count > 0 || paren_count > 0 || bracket_count > 0
    }
}

pub trait ReplExt {
    fn eval_expression(&mut self, expr: &str) -> Result<Value, ReplError>;
    fn define_variable(&mut self, name: &str, value: Value);
    fn get_variable(&self, name: &str) -> Option<Value>;
}

impl ReplExt for Repl {
    fn eval_expression(&mut self, expr: &str) -> Result<Value, ReplError> {
        self.eval_line(expr)
    }

    fn define_variable(&mut self, name: &str, value: Value) {
        self.ovm_interpreter
            .get_classic_interpreter()
            .define_variable(name.to_string(), value);
    }

    fn get_variable(&self, name: &str) -> Option<Value> {
        // Note: This would need &mut self to work properly, but keeping for compatibility
        None // Simplified for now
    }
}

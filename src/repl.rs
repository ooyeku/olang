use crate::ast::Value;
use crate::help::{HelpSystem, HelpContext, SearchFilters, Colors};
use crate::interpreter::InterpreterError;
use crate::ovm_integration::{IntegrationConfig, IntegrationError, OvmInterpreter};
use crate::parser::{ErrorSuggestion, ParseError, Parser, SuggestionSeverity};
use crate::version::VERSION;
use colored::*;
use rustyline::error::ReadlineError;
use rustyline::{history::DefaultHistory, Config, Editor};
use std::collections::{HashMap, HashSet};
use std::io::Write;
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
    #[error("Debug error: {0}")]
    Debug(String),
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

/// Call frame for debugging stack traces
#[derive(Debug, Clone)]
pub struct CallFrame {
    pub function_name: String,
    pub local_variables: HashMap<String, Value>,
    pub line_number: Option<usize>,
    pub file_name: Option<String>,
}

impl CallFrame {
    pub fn new(function_name: String) -> Self {
        Self {
            function_name,
            local_variables: HashMap::new(),
            line_number: None,
            file_name: None,
        }
    }
    
    pub fn with_location(mut self, line: usize, file: Option<String>) -> Self {
        self.line_number = Some(line);
        self.file_name = file;
        self
    }
    
    pub fn add_variable(&mut self, name: String, value: Value) {
        self.local_variables.insert(name, value);
    }
}

/// Interactive debugger for enhanced REPL debugging capabilities
#[derive(Debug, Clone)]
pub struct InteractiveDebugger {
    /// Variables being watched for changes
    watched_variables: HashSet<String>,
    
    /// Call stack for stack traces and debugging
    call_stack: Vec<CallFrame>,
    
    /// Whether step-through debugging is enabled
    debug_mode: bool,
    
    /// Breakpoints set by the user
    breakpoints: HashSet<String>,
    
    /// Function calls being traced
    traced_functions: HashSet<String>,
    
    /// Previous variable values for change detection
    variable_history: HashMap<String, Value>,
    
    /// Profiling data for performance analysis
    profiling_data: HashMap<String, Vec<f64>>,
}

impl Default for InteractiveDebugger {
    fn default() -> Self {
        Self::new()
    }
}

impl InteractiveDebugger {
    pub fn new() -> Self {
        Self {
            watched_variables: HashSet::new(),
            call_stack: Vec::new(),
            debug_mode: false,
            breakpoints: HashSet::new(),
            traced_functions: HashSet::new(),
            variable_history: HashMap::new(),
            profiling_data: HashMap::new(),
        }
    }
    
    pub fn watch_variable(&mut self, name: String) {
        self.watched_variables.insert(name);
    }
    
    pub fn unwatch_variable(&mut self, name: &str) {
        self.watched_variables.remove(name);
    }
    
    pub fn is_watching(&self, name: &str) -> bool {
        self.watched_variables.contains(name)
    }
    
    pub fn add_breakpoint(&mut self, location: String) {
        self.breakpoints.insert(location);
    }
    
    pub fn remove_breakpoint(&mut self, location: &str) {
        self.breakpoints.remove(location);
    }
    
    pub fn trace_function(&mut self, name: String) {
        self.traced_functions.insert(name);
    }
    
    pub fn untrace_function(&mut self, name: &str) {
        self.traced_functions.remove(name);
    }
    
    pub fn push_call_frame(&mut self, frame: CallFrame) {
        self.call_stack.push(frame);
    }
    
    pub fn pop_call_frame(&mut self) -> Option<CallFrame> {
        self.call_stack.pop()
    }
    
    pub fn get_call_stack(&self) -> &[CallFrame] {
        &self.call_stack
    }
    
    pub fn update_variable_history(&mut self, name: String, value: Value) {
        self.variable_history.insert(name, value);
    }
    
    pub fn record_profiling_data(&mut self, expression: String, time_ms: f64) {
        self.profiling_data.entry(expression).or_insert_with(Vec::new).push(time_ms);
    }
    
    pub fn get_profiling_data(&self, expression: &str) -> Option<&Vec<f64>> {
        self.profiling_data.get(expression)
    }
    
    pub fn clear_profiling_data(&mut self) {
        self.profiling_data.clear();
    }
    
    pub fn check_watched_variables(&self, current_vars: &HashMap<String, Value>) -> Vec<String> {
        let mut changes = Vec::new();
        
        for var_name in &self.watched_variables {
            if let Some(current_value) = current_vars.get(var_name) {
                if let Some(previous_value) = self.variable_history.get(var_name) {
                    if current_value != previous_value {
                        changes.push(format!(
                            "Variable '{}' changed: {} -> {}",
                            var_name.bright_yellow(),
                            previous_value.to_string().bright_red(),
                            current_value.to_string().bright_green()
                        ));
                    }
                } else {
                    changes.push(format!(
                        "Variable '{}' created: {}",
                        var_name.bright_yellow(),
                        current_value.to_string().bright_green()
                    ));
                }
            } else if self.variable_history.contains_key(var_name) {
                changes.push(format!(
                    "Variable '{}' removed",
                    var_name.bright_yellow()
                ));
            }
        }
        
        changes
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
    debugger: InteractiveDebugger,
}

impl Repl {
    pub fn new(verbose: bool) -> Result<Self, ReplError> {
        // Initialize parallelization for optimal performance
        if let Err(e) = crate::parallel::initialize_parallelization(None) {
            if verbose {
                eprintln!("Warning: Failed to initialize parallel processing: {}", e);
            }
        } else if verbose {
            println!(
                "Multi-threading enabled: {} CPU cores detected",
                num_cpus::get()
            );
        }

        // Set a very aggressive parallel threshold for maximum multi-threading by default
        crate::parallel::set_parallel_threshold(10);

        if verbose {
            println!(
                "Automatic parallelization: Lists with 10+ items will use all {} cores",
                num_cpus::get()
            );
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

        // Create OVM configuration with auto mode (OVM ENABLED by default for performance)
        let integration_config = IntegrationConfig {
            use_ovm_by_default: true,
            ovm_complexity_threshold: 1,
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
            debugger: InteractiveDebugger::new(),
        })
    }

    pub fn run(&mut self) -> Result<(), ReplError> {
        println!("Olang v{}", VERSION);

        // Display OVM status with enhanced messaging
        if self.ovm_interpreter.is_ovm_available() {
            let stats = self.ovm_interpreter.get_stats();
            println!(
                "{}",
                "OVM (Olang Virtual Machine) is running in the background".bright_green()
            );
            println!(
                "{}",
                "   • Advanced pipeline fusion optimization active".bright_cyan()
            );
            println!(
                "{}",
                "   • Automatic SIMD vectorization enabled".bright_cyan()
            );
            println!(
                "{}",
                "   • Garbage collection and memory optimization active".bright_cyan()
            );
            println!(
                "{}",
                "   • Your code is being optimized automatically!".bright_white()
            );

            if stats.ovm_executions > 0 || stats.classic_executions > 0 {
                println!(
                    "   • Previous session: {} total executions",
                    (stats.ovm_executions + stats.classic_executions)
                        .to_string()
                        .bright_white()
                );
            }
        } else {
            println!(
                "{}",
                "OVM not available - using classic interpreter".bright_yellow()
            );
        }

        println!();
        println!("Type 'help' for help, ':debug' for debugging commands, ':ovm status' for OVM details, ':env' to see environment, 'quit' to exit");
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
                            self.show_enhanced_error(&e);
                        }
                    }
                    
                    // Clean up module cache to prevent memory accumulation
                    self.ovm_interpreter.get_classic_interpreter().clear_module_cache();
                    
                    let _ = self.ovm_interpreter.force_gc();
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
                let _ = self.ovm_interpreter.force_gc();
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
                    self.show_enhanced_error(&e);
                }
            }
            
            // Clean up module cache to prevent memory accumulation
            self.ovm_interpreter.get_classic_interpreter().clear_module_cache();
            
            let _ = self.ovm_interpreter.force_gc();
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
                        "tutorials" => println!("{}", self.help_system.format_tutorial_list()),
                        "tutorial" => {
                            if parts.len() > 2 {
                                let tutorial_name = parts[2];
                                if let Some(tutorial) = self.help_system.get_tutorial(tutorial_name) {
                                    println!("{}", self.help_system.format_tutorial(tutorial));
                                } else {
                                    println!("Tutorial '{}' not found. Use 'help tutorials' to see available tutorials.", tutorial_name);
                                }
                            } else {
                                println!("{}", self.help_system.format_tutorial_list());
                            }
                        }
                        "search" => {
                            if parts.len() > 2 {
                                let query = parts[2..].join(" ");
                                let results = self.help_system.search(&query, None);
                                println!("{}", self.help_system.format_search_results(&results));
                            } else {
                                println!("Usage: help search <query>");
                                println!("Example: help search \"list functions\"");
                            }
                        }
                        "contextual" => {
                            let context = self.build_help_context();
                            println!("{}", self.help_system.format_contextual_help(&context));
                        }
                        _ => {
                            if self.help_system.has_function(topic) {
                                println!("{}", self.help_system.show_function_help(topic));
                            } else if self.help_system.has_category(topic) {
                                println!("{}", self.help_system.show_category(topic));
                            } else {
                                // Try advanced search if direct lookup fails
                                let results = self.help_system.search(topic, None);
                                if !results.is_empty() {
                                    println!("{}", self.help_system.format_search_results(&results));
                                } else {
                                    println!("No help found for '{}'. Try 'help search {}' for advanced search.", topic, topic);
                                }
                            }
                        }
                    }
                }
            }
            "quit" | ":quit" => {
                std::process::exit(0);
            }
            ":gc" => {
                if self.ovm_interpreter.is_ovm_available() {
                    println!("Forcing garbage collection...");
                    if let Err(e) = self.ovm_interpreter.force_gc() {
                        eprintln!("GC failed: {}", e);
                    } else {
                        println!("Garbage collection completed successfully");
                    }
                } else {
                    println!("OVM is not available, cannot run GC.");
                }
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
                            println!(
                                "  Initialized: {}",
                                ovm_status.initialized.to_string().bright_green()
                            );
                            println!(
                                "  Running: {}",
                                ovm_status.running.to_string().bright_green()
                            );
                            println!(
                                "  Healthy: {}",
                                self.ovm_interpreter
                                    .is_ovm_healthy()
                                    .to_string()
                                    .bright_green()
                            );

                            println!("\n=== Execution Statistics ===");
                            let stats = self.ovm_interpreter.get_stats();
                            let total_executions = stats.ovm_executions + stats.classic_executions;

                            println!(
                                "  Total executions: {}",
                                total_executions.to_string().bright_white()
                            );
                            if total_executions > 0 {
                                let ovm_percentage =
                                    (stats.ovm_executions as f64 / total_executions as f64) * 100.0;
                                let classic_percentage = (stats.classic_executions as f64
                                    / total_executions as f64)
                                    * 100.0;

                                println!(
                                    "  OVM executions: {} ({:.1}%)",
                                    stats.ovm_executions.to_string().bright_cyan(),
                                    ovm_percentage.to_string().bright_cyan()
                                );
                                println!(
                                    "  Classic executions: {} ({:.1}%)",
                                    stats.classic_executions.to_string().bright_yellow(),
                                    classic_percentage.to_string().bright_yellow()
                                );
                            } else {
                                println!(
                                    "  OVM executions: {}",
                                    stats.ovm_executions.to_string().bright_cyan()
                                );
                                println!(
                                    "  Classic executions: {}",
                                    stats.classic_executions.to_string().bright_yellow()
                                );
                            }
                            println!(
                                "  Fallback executions: {}",
                                stats.fallback_executions.to_string().bright_red()
                            );
                            println!(
                                "  Function compilations: {}",
                                stats.compilation_count.to_string().bright_blue()
                            );

                            if stats.average_ovm_time_ms > 0.0 {
                                println!("  Avg OVM time: {:.2}ms", stats.average_ovm_time_ms);
                            }
                            if stats.average_classic_time_ms > 0.0 {
                                println!(
                                    "  Avg classic time: {:.2}ms",
                                    stats.average_classic_time_ms
                                );
                            }

                            if ovm_status.running {
                                println!(
                                    "\n{}",
                                    "OVM is actively optimizing your code in the background"
                                        .bright_green()
                                );
                            } else {
                                println!(
                                    "\n{}",
                                    "OVM is not running - falling back to classic interpreter"
                                        .bright_yellow()
                                );
                            }
                        }
                        "gc" => {
                            if let Err(e) = self.ovm_interpreter.force_gc() {
                                eprintln!("GC failed: {}", e);
                            } else {
                                println!("Garbage collection completed successfully");
                            }
                        }
                        "restart" => match self.ovm_interpreter.ensure_ovm_running() {
                            Ok(()) => println!("OVM restarted successfully"),
                            Err(e) => eprintln!("Failed to restart OVM: {}", e),
                        },
                        _ => {
                            println!("Usage: :ovm [status|gc|restart]");
                        }
                    }
                } else {
                    let available = self.ovm_interpreter.is_ovm_available();
                    let healthy = self.ovm_interpreter.is_ovm_healthy();
                    println!(
                        "OVM available: {} | Healthy: {}",
                        available.to_string().bright_green(),
                        healthy.to_string().bright_green()
                    );
                    if available && healthy {
                        println!("{}", "OVM is running in the background".bright_green());
                    } else {
                        println!(
                            "{}",
                            "OVM may need attention - use ':ovm status' for details"
                                .bright_yellow()
                        );
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
                            if let Some(_stmt) = program.statements.first() {
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
                            println!(
                                "  Max threads: {}",
                                config.max_threads.to_string().bright_cyan()
                            );
                            println!(
                                "  Parallel threshold: {} items",
                                config.min_parallel_size.to_string().bright_yellow()
                            );
                            println!(
                                "  Available CPU cores: {}",
                                num_cpus::get().to_string().bright_white()
                            );

                            // Test parallel processing
                            let large_list: Vec<usize> =
                                (1..=config.min_parallel_size + 100).collect();
                            println!(
                                "  Test: List of {} items would use {} processing",
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
                            println!("Parallel processing enabled");
                        }
                        "disable" => {
                            crate::parallel::set_parallel_enabled(false);
                            println!("Parallel processing disabled");
                        }
                        "threshold" => {
                            if parts.len() > 2 {
                                if let Ok(threshold) = parts[2].parse::<usize>() {
                                    crate::parallel::set_parallel_threshold(threshold);
                                        println!("Parallel threshold set to {} items", threshold);
                                } else {
                                    println!("Error: Invalid threshold value");
                                }
                            } else {
                                let config = crate::parallel::get_config();
                                println!(
                                    "Current parallel threshold: {} items",
                                    config.min_parallel_size
                                );
                                println!("Usage: :parallel threshold <number>");
                            }
                        }
                        _ => {
                            println!("Usage: :parallel [status|enable|disable|threshold <number>]");
                        }
                    }
                } else {
                    let config = crate::parallel::get_config();
                    println!(
                        "Parallel processing: {} | Threads: {} | Threshold: {} items",
                        if config.enabled {
                            "enabled".bright_green()
                        } else {
                            "disabled".bright_red()
                        },
                        config.max_threads.to_string().bright_cyan(),
                        config.min_parallel_size.to_string().bright_yellow()
                    );
                }
            }
            ":run" => {
                if parts.len() > 1 {
                    let filename = parts[1];
                    match std::fs::read_to_string(filename) {
                        Ok(content) => {
                            match self.eval_line(&content) {
                                Ok(value) => {
                                    if value != Value::Unit {
                                        println!("{}", value);
                                    }
                                    println!("File '{}' executed successfully", filename);
                                }
                                Err(e) => {
                                    eprintln!("Error executing '{}': {}", filename, e);
                                }
                            }
                            // Aggressive cleanup after script execution
                            // 1. Clear user environment to free variables
                            self.ovm_interpreter
                                .get_classic_interpreter()
                                .clear_user_environment();

                            // 2. Force garbage collection multiple times
                            if self.ovm_interpreter.is_ovm_available() {
                                for _ in 0..3 {
                                    if let Err(e) = self.ovm_interpreter.force_gc() {
                                        if self.verbose {
                                            eprintln!("GC after script execution failed: {}", e);
                                        }
                                        break;
                                    }
                                }
                            }
                        }
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
                            self.debugger.debug_mode = true;
                            println!("Debug mode enabled");
                        }
                        "off" => {
                            self.config.debug_mode = false;
                            self.debugger.debug_mode = false;
                            println!("Debug mode disabled");
                        }
                        _ => {
                            // Step through expression evaluation
                            let expr = parts[1..].join(" ");
                            self.debug_evaluate_expression(&expr)?;
                        }
                    }
                } else {
                    println!(
                        "Debug mode: {}",
                        if self.config.debug_mode { "on" } else { "off" }
                    );
                    self.show_debug_help();
                }
            }
            ":watch" => {
                if parts.len() > 1 {
                    let var_name = parts[1].to_string();
                    if self.debugger.is_watching(&var_name) {
                        self.debugger.unwatch_variable(&var_name);
                        println!("Stopped watching variable: {}", var_name.bright_yellow());
                    } else {
                        self.debugger.watch_variable(var_name.clone());
                        println!("Now watching variable: {}", var_name.bright_yellow());
                        
                        // Store current value for change detection
                        let user_vars = self.ovm_interpreter.get_classic_interpreter().get_user_variables();
                        if let Some(value) = user_vars.get(&var_name) {
                            self.debugger.update_variable_history(var_name, (*value).clone());
                        }
                    }
                } else {
                    println!("Currently watched variables:");
                    if self.debugger.watched_variables.is_empty() {
                        println!("  {}", "(none)".bright_black());
                    } else {
                        for var in &self.debugger.watched_variables {
                            println!("  {}", var.bright_yellow());
                        }
                    }
                    println!("Usage: :watch <variable>");
                }
            }
            ":inspect" => {
                if parts.len() > 1 {
                    let var_name = parts[1];
                    self.inspect_variable(var_name)?;
                } else {
                    println!("Usage: :inspect <variable>");
                }
            }
            ":trace" => {
                if parts.len() > 1 {
                    let func_name = parts[1].to_string();
                    if self.debugger.traced_functions.contains(&func_name) {
                        self.debugger.untrace_function(&func_name);
                        println!("Stopped tracing function: {}", func_name.bright_blue());
                    } else {
                        self.debugger.trace_function(func_name.clone());
                        println!("Now tracing function: {}", func_name.bright_blue());
                    }
                } else {
                    println!("Currently traced functions:");
                    if self.debugger.traced_functions.is_empty() {
                        println!("  {}", "(none)".bright_black());
                    } else {
                        for func in &self.debugger.traced_functions {
                            println!("  {}", func.bright_blue());
                        }
                    }
                    println!("Usage: :trace <function>");
                }
            }
            ":set" => {
                if parts.len() > 2 {
                    let var_name = parts[1].to_string();
                    let value_expr = parts[2..].join(" ");
                    self.set_variable(var_name, value_expr)?;
                } else {
                    println!("Usage: :set <variable> <value>");
                }
            }
            ":stack" => {
                self.show_call_stack();
            }
            ":profile" => {
                if parts.len() > 1 {
                    let expr = parts[1..].join(" ");
                    self.profile_expression(&expr)?;
                } else {
                    self.show_profiling_summary();
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
                        
                        // Safe handling of min/max without unwrap
                        match (times.iter().min(), times.iter().max()) {
                            (Some(min), Some(max)) => {
                                println!("Benchmark results:");
                                println!("  Average: {:.2}ms", avg.as_secs_f64() * 1000.0);
                                println!("  Min: {:.2}ms", min.as_secs_f64() * 1000.0);
                                println!("  Max: {:.2}ms", max.as_secs_f64() * 1000.0);
                            }
                            _ => {
                                eprintln!("Error: Failed to calculate benchmark statistics");
                            }
                        }
                    }
                } else {
                    println!("Usage: :benchmark <expression>");
                }
            }
            ":search" => {
                if parts.len() > 1 {
                    let query = parts[1..].join(" ");
                    
                    // Parse advanced search options
                    let mut filters = SearchFilters::default();
                    let mut search_query = query.clone();
                    
                    // Check for category filter
                    if query.contains("category:") {
                        if let Some(category_part) = query.split("category:").nth(1) {
                            let category = category_part.split_whitespace().next().unwrap_or("");
                            filters.category = Some(category.to_string());
                            search_query = query.replace(&format!("category:{}", category), "").trim().to_string();
                        }
                    }
                    
                    // Check for return type filter
                    if query.contains("returns:") {
                        if let Some(return_part) = query.split("returns:").nth(1) {
                            let return_type = return_part.split_whitespace().next().unwrap_or("");
                            filters.return_type = Some(return_type.to_string());
                            search_query = query.replace(&format!("returns:{}", return_type), "").trim().to_string();
                        }
                    }
                    
                    // Check for max results filter
                    if query.contains("limit:") {
                        if let Some(limit_part) = query.split("limit:").nth(1) {
                            if let Ok(limit) = limit_part.split_whitespace().next().unwrap_or("").parse::<usize>() {
                                filters.max_results = limit;
                                search_query = query.replace(&format!("limit:{}", limit), "").trim().to_string();
                            }
                        }
                    }
                    
                    let results = self.help_system.search(&search_query, Some(filters));
                    println!("{}", self.help_system.format_search_results(&results));
                    
                    // Show search tips
                    if results.is_empty() {
                        println!("\n{}Search Tips:{}", Colors::CYAN, Colors::RESET);
                        println!("  • Try broader terms: 'list' instead of 'list_operations'");
                        println!("  • Use category filters: 'category:List map'");
                        println!("  • Use return type filters: 'returns:List filter'");
                        println!("  • Use fuzzy search: 'lst' will match 'list' functions");
                        println!("  • Search in descriptions: 'transform' finds map, filter, etc.");
                        
                        // Show suggestions based on partial input
                        let suggestions = self.help_system.get_suggestions(&search_query);
                        if !suggestions.is_empty() {
                            println!("\n{}Did you mean:{}", Colors::YELLOW, Colors::RESET);
                            for suggestion in suggestions.iter().take(5) {
                                println!("  • {}", suggestion);
                            }
                        }
                    }
                } else {
                    println!("Usage: :search <query> [filters]");
                    println!("\n{}Advanced Search Options:{}", Colors::CYAN, Colors::RESET);
                    println!("  {}category:<name>{} - Filter by category", Colors::BLUE, Colors::RESET);
                    println!("  {}returns:<type>{} - Filter by return type", Colors::BLUE, Colors::RESET);
                    println!("  {}limit:<num>{} - Limit number of results", Colors::BLUE, Colors::RESET);
                    println!("\n{}Examples:{}", Colors::GREEN, Colors::RESET);
                    println!("  :search map category:List");
                    println!("  :search returns:Bool");
                    println!("  :search transform limit:5");
                    println!("  :search \"file operations\"");
                }
            }
            ":tutorial" => {
                if parts.len() > 1 {
                    let tutorial_name = parts[1];
                    if let Some(tutorial) = self.help_system.get_tutorial(tutorial_name) {
                        println!("{}", self.help_system.format_tutorial(tutorial));
                        
                        // Offer interactive mode
                        println!("\n{}Interactive Mode:{}", Colors::CYAN, Colors::RESET);
                        println!("  Would you like to try the tutorial interactively?");
                        println!("  Use ':tutorial run {}' to start interactive mode", tutorial_name);
                    } else {
                        println!("Tutorial '{}' not found.", tutorial_name);
                        println!("{}", self.help_system.format_tutorial_list());
                    }
                } else {
                    println!("{}", self.help_system.format_tutorial_list());
                }
            }
                         ":tutorial_run" => {
                 if parts.len() > 1 {
                     let tutorial_name = parts[1];
                     if let Some(tutorial) = self.help_system.get_tutorial(tutorial_name).cloned() {
                         self.run_interactive_tutorial(&tutorial)?;
                     } else {
                         println!("Tutorial '{}' not found.", tutorial_name);
                     }
                 } else {
                     println!("Usage: :tutorial_run <tutorial_name>");
                 }
             }
            ":contextual_help" => {
                let context = self.build_help_context();
                println!("{}", self.help_system.format_contextual_help(&context));
            }
            ":help_advanced" => {
                println!("\n{}=== Advanced Help Features ==={}", Colors::BOLD, Colors::RESET);
                println!("\n{}Advanced Search:{}", Colors::CYAN, Colors::RESET);
                println!("  {}:search <query>{} - Fuzzy search with relevance scoring", Colors::BLUE, Colors::RESET);
                println!("  {}help search <query>{} - Same as :search", Colors::BLUE, Colors::RESET);
                println!("  {}category:<name>{} - Filter by category", Colors::GREEN, Colors::RESET);
                println!("  {}returns:<type>{} - Filter by return type", Colors::GREEN, Colors::RESET);
                println!("  {}limit:<num>{} - Limit results", Colors::GREEN, Colors::RESET);
                
                println!("\n{}Interactive Tutorials:{}", Colors::CYAN, Colors::RESET);
                println!("  {}help tutorials{} - List all available tutorials", Colors::BLUE, Colors::RESET);
                println!("  {}help tutorial <name>{} - View specific tutorial", Colors::BLUE, Colors::RESET);
                println!("  {}:tutorial <name>{} - View tutorial with interactive options", Colors::BLUE, Colors::RESET);
                println!("  {}:tutorial_run <name>{} - Start interactive tutorial mode", Colors::BLUE, Colors::RESET);
                
                println!("\n{}Context-Sensitive Help:{}", Colors::CYAN, Colors::RESET);
                println!("  {}:contextual_help{} - Get suggestions based on recent activity", Colors::BLUE, Colors::RESET);
                println!("  {}help contextual{} - Same as :contextual_help", Colors::BLUE, Colors::RESET);
                println!("  Automatic suggestions based on:");
                println!("    • Recent commands and patterns");
                println!("    • Last error messages");
                println!("    • Current variables and types");
                println!("    • Working category context");
                
                println!("\n{}Enhanced Examples:{}", Colors::CYAN, Colors::RESET);
                println!("  {}:examples <function>{} - Interactive examples with execution", Colors::BLUE, Colors::RESET);
                println!("  {}help examples{} - Comprehensive example gallery", Colors::BLUE, Colors::RESET);
                println!("  All examples are runnable and include explanations");
                
                println!("\n{}Search Result Types:{}", Colors::CYAN, Colors::RESET);
                println!("  Exact name matches (highest priority)");
                println!("  Fuzzy name matches");
                println!("  Description matches");
                println!("  Category matches");
                println!("  Example matches");
                println!("  Parameter matches");
                println!("  Signature matches");
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
        // Check for watched variables before execution
        let watching_vars = !self.debugger.watched_variables.is_empty();
        if watching_vars {
            let vars = self.ovm_interpreter.get_classic_interpreter().get_user_variables();
            for (name, value) in &vars {
                if self.debugger.is_watching(name) {
                    self.debugger.update_variable_history(name.clone(), (*value).clone());
                }
            }
        }

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

        // Check for watched variable changes after execution
        if watching_vars {
            let user_vars_after = self.ovm_interpreter.get_classic_interpreter().get_user_variables();
            
            // Convert HashMap<String, &Value> to HashMap<String, Value> for compatibility
            let user_vars_owned: HashMap<String, Value> = user_vars_after.iter()
                .map(|(k, v)| (k.clone(), (*v).clone()))
                .collect();
            
            let changes = self.debugger.check_watched_variables(&user_vars_owned);
            
            if !changes.is_empty() {
                println!("\n{}", "Watched variable changes:".bright_yellow().bold());
                for change in changes {
                    println!("  {}", change);
                }
            }
            
            // Update variable history
            for (name, value) in &user_vars_after {
                if self.debugger.is_watching(name) {
                    self.debugger.update_variable_history(name.clone(), (*value).clone());
                }
            }
        }

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
            Value::Enum { type_name: _, .. } => {
                // Use a static string for REPL display
                "enum"
            },
            Value::Promise { .. } => "promise",
            Value::Map(_) => "map",
            Value::TypeInfo { .. } => "type",
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

    /// Enhanced debugging methods
    fn show_debug_help(&self) {
        println!("\n{}", "=== Interactive Debugging Commands ===".bright_cyan().bold());
        println!("  {}  - Step through expression evaluation", ":debug <expression>".bright_blue());
        println!("  {}     - Toggle variable watching", ":watch <variable>".bright_blue());
        println!("  {}   - Detailed variable inspection", ":inspect <variable>".bright_blue());
        println!("  {}     - Trace function calls", ":trace <function>".bright_blue());
        println!("  {}      - Modify variable values", ":set <var> <value>".bright_blue());
        println!("  {}           - Show call stack", ":stack".bright_blue());
        println!("  {}    - Profile expression performance", ":profile <expression>".bright_blue());
        println!("  {}       - Enable/disable debug mode", ":debug [on|off]".bright_blue());
        println!();
    }

    fn debug_evaluate_expression(&mut self, expr: &str) -> Result<(), ReplError> {
        println!("{}", format!("Debugging: {}", expr).bright_cyan().bold());
        
        // Check for watched variables before execution
        let user_vars_before = self.ovm_interpreter.get_classic_interpreter().get_user_variables();
        for (name, value) in &user_vars_before {
            if self.debugger.is_watching(name) {
                self.debugger.update_variable_history(name.clone(), (*value).clone());
            }
        }
        
        // Parse and show AST if in debug mode
        match self.parser.parse(expr) {
            Ok(program) => {
                if self.debugger.debug_mode {
                    println!("  {}: {:?}", "Parsed AST".bright_green(), program);
                }
                
                // Execute with timing
                let start = Instant::now();
                match self.eval_line(expr) {
                    Ok(value) => {
                        let duration = start.elapsed();
                        let time_ms = duration.as_secs_f64() * 1000.0;
                        
                        if value != Value::Unit {
                            println!("  {}: {}", "Result".bright_green(), value);
                        }
                        println!("  {}: {:.2}ms", "Execution time".bright_blue(), time_ms);
                        
                        // Record profiling data
                        self.debugger.record_profiling_data(expr.to_string(), time_ms);
                        
                        // Check for watched variable changes
                        let user_vars_after = self.ovm_interpreter.get_classic_interpreter().get_user_variables();
                        
                        // Convert HashMap<String, &Value> to HashMap<String, Value> for compatibility
                        let user_vars_owned: HashMap<String, Value> = user_vars_after.iter()
                            .map(|(k, v)| (k.clone(), (*v).clone()))
                            .collect();
                        
                        let changes = self.debugger.check_watched_variables(&user_vars_owned);
                        
                        if !changes.is_empty() {
                            println!("  {}:", "Variable changes".bright_yellow().bold());
                            for change in changes {
                                println!("    {}", change);
                            }
                        }
                        
                        // Update variable history
                        for (name, value) in &user_vars_after {
                            if self.debugger.is_watching(name) {
                                self.debugger.update_variable_history(name.clone(), (*value).clone());
                            }
                        }
                    }
                    Err(e) => {
                        let duration = start.elapsed();
                        println!("  {}: {}", "Error".bright_red(), e);
                        println!("  {}: {:.2}ms", "Time to error".bright_blue(), duration.as_secs_f64() * 1000.0);
                        
                        // Enhanced error context
                        self.show_enhanced_error(&e);
                    }
                }
            }
            Err(e) => {
                println!("  {}: {}", "Parse error".bright_red(), e);
            }
        }
        
        Ok(())
    }

    fn inspect_variable(&mut self, var_name: &str) -> Result<(), ReplError> {
        let user_vars = self.ovm_interpreter.get_classic_interpreter().get_user_variables();
        
        if let Some(value) = user_vars.get(var_name) {
            println!("\n{}", format!("=== Variable Inspection: {} ===", var_name).bright_cyan().bold());
            println!("  {}: {}", "Type".bright_yellow(), Self::get_type_name(value));
            println!("  {}: {}", "Value".bright_green(), value);
            
            // Additional type-specific information
            match value {
                Value::String(s) => {
                    println!("  {}: {} characters", "Length".bright_blue(), s.len());
                    if s.contains('\n') {
                        println!("  {}: {} lines", "Lines".bright_blue(), s.lines().count());
                    }
                }
                Value::List(items) => {
                    println!("  {}: {} items", "Length".bright_blue(), items.len());
                    if !items.is_empty() {
                        let first_type = Self::get_type_name(&items[0]);
                        let all_same_type = items.iter().all(|item| Self::get_type_name(item) == first_type);
                        if all_same_type {
                            println!("  {}: {}", "Element type".bright_blue(), first_type);
                        } else {
                            println!("  {}: mixed", "Element type".bright_blue());
                        }
                    }
                }
                Value::Struct { type_name, fields } => {
                    println!("  {}: {}", "Struct type".bright_blue(), type_name);
                    println!("  {}: {} fields", "Field count".bright_blue(), fields.len());
                    for (field_name, field_value) in fields {
                        println!("    {}: {} = {}", 
                            field_name.bright_magenta(), 
                            Self::get_type_name(field_value),
                            Self::format_value_preview(field_value)
                        );
                    }
                }
                Value::Function(func) => {
                    println!("  {}: {} parameters", "Arity".bright_blue(), func.parameters.len());
                    if !func.parameters.is_empty() {
                        println!("  {}: {}", "Parameters".bright_blue(), 
                            func.parameters.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")
                        );
                    }
                }
                _ => {}
            }
            
            // Show if variable is being watched
            if self.debugger.is_watching(var_name) {
                println!("  {}: {}", "Status".bright_green(), "Being watched".bright_green());
            }
            
        } else {
            // Check if it's a builtin function
            let builtins = self.ovm_interpreter.get_classic_interpreter().get_builtin_functions();
            if let Some(_builtin) = builtins.get(var_name) {
                println!("\n{}", format!("=== Builtin Function: {} ===", var_name).bright_cyan().bold());
                println!("  {}: builtin function", "Type".bright_yellow());
                
                // Show help if available
                if self.help_system.has_function(var_name) {
                    println!("\n{}", self.help_system.show_function_help(var_name));
                } else {
                    println!("  {}: No documentation available", "Help".bright_blue());
                }
            } else {
                return Err(ReplError::Debug(format!("Variable '{}' not found", var_name)));
            }
        }
        
        Ok(())
    }

    fn set_variable(&mut self, var_name: String, value_expr: String) -> Result<(), ReplError> {
        // Parse and evaluate the value expression
        match self.eval_line(&value_expr) {
            Ok(value) => {
                // Check if variable exists
                let user_vars = self.ovm_interpreter.get_classic_interpreter().get_user_variables();
                let existed = user_vars.contains_key(&var_name);
                
                // Set the variable
                self.ovm_interpreter
                    .get_classic_interpreter()
                    .define_variable(var_name.clone(), value.clone());
                
                if existed {
                    println!("Variable '{}' updated to: {}", var_name.bright_yellow(), value);
                } else {
                    println!("Variable '{}' created with value: {}", var_name.bright_yellow(), value);
                }
                
                // Update variable history if being watched
                if self.debugger.is_watching(&var_name) {
                    self.debugger.update_variable_history(var_name, value);
                }
            }
            Err(e) => {
                return Err(ReplError::Debug(format!("Failed to evaluate value expression '{}': {}", value_expr, e)));
            }
        }
        
        Ok(())
    }

    fn show_call_stack(&self) {
        println!("\n{}", "=== Call Stack ===".bright_cyan().bold());
        
        let stack = self.debugger.get_call_stack();
        
        if stack.is_empty() {
            println!("  {}", "(empty - no active function calls)".bright_black());
        } else {
            for (i, frame) in stack.iter().enumerate().rev() {
                let frame_num = stack.len() - i - 1;
                println!("  #{}: {}", 
                    frame_num.to_string().bright_white(),
                    frame.function_name.bright_blue()
                );
                
                if let Some(line) = frame.line_number {
                    print!("      at line {}", line.to_string().bright_cyan());
                    if let Some(file) = &frame.file_name {
                        print!(" in {}", file.bright_green());
                    }
                    println!();
                }
                
                if !frame.local_variables.is_empty() {
                    println!("      local variables:");
                    for (name, value) in &frame.local_variables {
                        println!("        {} = {}", 
                            name.bright_yellow(),
                            Self::format_value_preview(value)
                        );
                    }
                }
            }
        }
        
        println!();
    }

    fn profile_expression(&mut self, expr: &str) -> Result<(), ReplError> {
        println!("{}", format!("Profiling: {}", expr).bright_cyan().bold());
        
        let iterations = 5;
        let mut times = Vec::new();
        
        for i in 1..=iterations {
            let start = Instant::now();
            match self.eval_line(expr) {
                Ok(_) => {
                    let duration = start.elapsed();
                    let time_ms = duration.as_secs_f64() * 1000.0;
                    times.push(time_ms);
                    println!("  Run {}: {:.2}ms", i, time_ms);
                }
                Err(e) => {
                    println!("  Run {} failed: {}", i, e);
                    return Ok(());
                }
            }
        }
        
        if !times.is_empty() {
            let sum: f64 = times.iter().sum();
            let avg = sum / times.len() as f64;
            let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            
            println!("\n  {}:", "Profile Results".bright_green().bold());
            println!("    Average: {:.2}ms", avg);
            println!("    Min: {:.2}ms", min);
            println!("    Max: {:.2}ms", max);
            println!("    Total: {:.2}ms", sum);
            
            // Record profiling data
            for time in times {
                self.debugger.record_profiling_data(expr.to_string(), time);
            }
        }
        
        Ok(())
    }

    fn show_profiling_summary(&self) {
        println!("\n{}", "=== Profiling Summary ===".bright_cyan().bold());
        
        if self.debugger.profiling_data.is_empty() {
            println!("  {}", "No profiling data available".bright_black());
            println!("  Use :profile <expression> to collect performance data");
        } else {
            for (expr, times) in &self.debugger.profiling_data {
                if !times.is_empty() {
                    let sum: f64 = times.iter().sum();
                    let avg = sum / times.len() as f64;
                    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
                    let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    
                    println!("\n  {}: {}", "Expression".bright_blue(), expr);
                    println!("    Runs: {}", times.len());
                    println!("    Average: {:.2}ms", avg);
                    println!("    Min: {:.2}ms", min);
                    println!("    Max: {:.2}ms", max);
                }
            }
        }
        
        println!();
    }

    fn show_enhanced_error(&mut self, error: &ReplError) {
        println!("\n{}", "═══ Error Details ═══".bright_red().bold());
        
        match error {
            ReplError::Parse(parse_error) => {
                self.show_parse_error(parse_error);
            }
            ReplError::Interpreter(interpreter_error) => {
                self.show_interpreter_error(interpreter_error);
            }
            ReplError::Integration(integration_error) => {
                println!("  {}: {}", "Integration Error".bright_red(), integration_error);
            }
            ReplError::Readline(readline_error) => {
                println!("  {}: {}", "Input Error".bright_red(), readline_error);
            }
            ReplError::Io(io_error) => {
                println!("  {}: {}", "IO Error".bright_red(), io_error);
            }
            ReplError::Debug(debug_error) => {
                println!("  {}: {}", "Debug Error".bright_red(), debug_error);
            }
        }
        
        println!();
    }
    
    fn show_parse_error(&mut self, parse_error: &ParseError) {
        // Display the main error message with enhanced formatting
        match parse_error {
            ParseError::InvalidSyntaxWithPosition { message, line, column, snippet } => {
                println!("  {}: {}", "Parse Error".bright_red().bold(), message.bright_white());
                println!("\n  {}", "Location:".bright_yellow().bold());
                println!("    Line {}, Column {}", line.to_string().bright_cyan(), column.to_string().bright_cyan());
                
                // Show the formatted code snippet with highlighting
                if !snippet.trim().is_empty() {
                    println!("\n  {}", "Code Context:".bright_blue().bold());
                    self.show_highlighted_snippet(snippet);
                }
            }
            ParseError::UnexpectedTokenWithPosition { token, line, column, snippet } => {
                println!("  {}: Unexpected token '{}'", 
                    "Parse Error".bright_red().bold(), 
                    token.bright_yellow().bold()
                );
     
                println!("    Line {}, Column {}", line.to_string().bright_cyan(), column.to_string().bright_cyan());
                
                if !snippet.trim().is_empty() {
                    println!("\n  {}", "Code Context:".bright_blue().bold());
                    self.show_highlighted_snippet(snippet);
                }
            }
            _ => {
                println!("  {}: {}", "Parse Error".bright_red().bold(), parse_error);
            }
        }
        
        // Get the last command for context
        let empty_string = String::new();
        let last_command = self.command_history.last().unwrap_or(&empty_string).clone();
        
        // Get suggestions from the parser
        let suggestions = self.parser.get_suggestions(parse_error, &last_command);
        
        if !suggestions.is_empty() {
            println!("\n  {}", "Suggestions:".bright_cyan().bold());
            for suggestion in suggestions {
                self.show_suggestion(&suggestion);
            }
        }
        
        // Show related help if available
        self.show_contextual_help(parse_error, &last_command);
    }
    
    fn show_highlighted_snippet(&self, snippet: &str) {
        // Parse and display the snippet with syntax highlighting
        let lines: Vec<&str> = snippet.lines().collect();
        
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            
            // Check if this line contains a caret pointer
            if line.contains("^") {
                // This is the error pointer line - highlight it specially
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 2 {
                    let line_num = parts[0].trim();
                    let pointer = parts[1];
                    println!("    {}{}│{}{}", 
                        line_num.bright_black(),
                        " ".repeat(4 - line_num.len().min(4)),
                        " ",
                        pointer.bright_red().bold()
                    );
                }
            } else {
                // Regular code line - apply basic syntax highlighting
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 2 {
                    let line_num = parts[0].trim();
                    let code = parts[1];
                    
                    // Basic syntax highlighting
                    let highlighted_code = self.apply_basic_highlighting(code);
                    
                    println!("    {}{}│ {}", 
                        line_num.bright_blue(),
                        " ".repeat(4 - line_num.len().min(4)),
                        highlighted_code
                    );
                }
            }
        }
    }
    
    fn apply_basic_highlighting(&self, code: &str) -> String {
        let mut result = String::new();
        let mut chars = code.chars().peekable();
        let mut current_word = String::new();
        
        while let Some(ch) = chars.next() {
            match ch {
                // String literals
                '"' => {
                    if !current_word.is_empty() {
                        result.push_str(&self.highlight_word(&current_word));
                        current_word.clear();
                    }
                    result.push_str(&format!("{}", "\"".bright_green()));
                    
                    // Consume the string content
                    while let Some(str_ch) = chars.next() {
                        if str_ch == '"' {
                            result.push_str(&format!("{}", str_ch.to_string().bright_green()));
                            break;
                        } else if str_ch == '\\' {
                            result.push_str(&format!("{}", str_ch.to_string().bright_green()));
                            if let Some(escaped) = chars.next() {
                                result.push_str(&format!("{}", escaped.to_string().bright_green()));
                            }
                        } else {
                            result.push_str(&format!("{}", str_ch.to_string().bright_green()));
                        }
                    }
                }
                // Numbers
                c if c.is_ascii_digit() => {
                    current_word.push(c);
                    
                    // Look ahead to consume the full number
                    while let Some(&next_ch) = chars.peek() {
                        if next_ch.is_ascii_digit() || next_ch == '.' || next_ch == '_' {
                            current_word.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&format!("{}", current_word.bright_magenta()));
                    current_word.clear();
                }
                // Identifiers and keywords
                c if c.is_alphabetic() || c == '_' => {
                    current_word.push(c);
                }
                // Operators and punctuation
                '=' | '+' | '-' | '*' | '/' | '%' | '<' | '>' | '!' | '&' | '|' => {
                    if !current_word.is_empty() {
                        result.push_str(&self.highlight_word(&current_word));
                        current_word.clear();
                    }
                    
                    // Look ahead for compound operators
                    let mut op = ch.to_string();
                    if let Some(&next_ch) = chars.peek() {
                        if (ch == '=' && next_ch == '=') ||
                           (ch == '!' && next_ch == '=') ||
                           (ch == '<' && next_ch == '=') ||
                           (ch == '>' && next_ch == '=') ||
                           (ch == '&' && next_ch == '&') ||
                           (ch == '|' && next_ch == '|') {
                            op.push(chars.next().unwrap());
                        }
                    }
                    result.push_str(&format!("{}", op.bright_yellow()));
                }
                // Brackets and parentheses
                '(' | ')' | '[' | ']' | '{' | '}' => {
                    if !current_word.is_empty() {
                        result.push_str(&self.highlight_word(&current_word));
                        current_word.clear();
                    }
                    result.push_str(&format!("{}", ch.to_string().bright_cyan()));
                }
                // Other characters
                _ => {
                    if !current_word.is_empty() {
                        result.push_str(&self.highlight_word(&current_word));
                        current_word.clear();
                    }
                    result.push(ch);
                }
            }
        }
        
        // Handle any remaining word
        if !current_word.is_empty() {
            result.push_str(&self.highlight_word(&current_word));
        }
        
        result
    }
    
    fn highlight_word(&self, word: &str) -> String {
        match word {
            // Keywords
            "fn" | "let" | "if" | "else" | "match" | "for" | "while" | "loop" | "break" | "continue" |
            "true" | "false" | "async" | "await" | "try" | "catch" | "import" | "export" | "type" => {
                format!("{}", word.bright_blue().bold())
            }
            // Types
            "Int" | "Float" | "String" | "Bool" | "List" | "Map" | "Unit" | "Result" | "Ok" | "Err" => {
                format!("{}", word.bright_magenta())
            }
            // Built-in functions (common ones)
            "println" | "print" | "map" | "filter" | "reduce" | "range" | "len" | "head" | "tail" => {
                format!("{}", word.bright_cyan())
            }
            _ => word.to_string(),
        }
    }
    
    fn show_contextual_help(&mut self, parse_error: &ParseError, input: &str) {
        // Extract keywords from the error and input to suggest relevant help
        let error_msg = format!("{:?}", parse_error);
        let mut help_topics = HashSet::new();
        
        // Suggest error-specific help topics
        if error_msg.contains("InvalidSyntax") || error_msg.contains("UnexpectedToken") {
            help_topics.insert("error.syntax");
        }
        
        // Check for specific syntax issues
        if error_msg.contains("bracket") || error_msg.contains("parenthesis") || error_msg.contains("brace") {
            help_topics.insert("error.syntax");
        }
        
        if error_msg.contains("quote") || error_msg.contains("string") {
            help_topics.insert("error.syntax");
        }
        
        // Check for function-related errors
        if error_msg.contains("fn") || input.contains("fn") {
            help_topics.insert("functions");
            help_topics.insert("error.syntax");
        }
        
        // Check for let-related errors
        if error_msg.contains("let") || input.contains("let") {
            help_topics.insert("variables");
            help_topics.insert("error.scope");
        }
        
        // Check for match-related errors
        if error_msg.contains("match") || input.contains("match") {
            help_topics.insert("pattern_matching");
            help_topics.insert("error.types");
        }
        
        // Check for type-related errors
        if error_msg.contains("type") || input.contains(": ") {
            help_topics.insert("types");
            help_topics.insert("error.types");
        }
        
        // Check for common language migration issues
        if input.contains("console.log") || input.contains("printf") || input.contains(";") {
            help_topics.insert("error.differences");
        }
        
        // Check for assignment/comparison confusion
        if input.contains("=") && !input.contains("==") && !input.contains("let") {
            help_topics.insert("error.syntax");
            help_topics.insert("error.differences");
        }
        
        if !help_topics.is_empty() {
            println!("\n  {}", "Related Help Topics:".bright_cyan().bold());
            let has_error_topics = help_topics.iter().any(|t| t.starts_with("error."));
            
            for topic in &help_topics {
                if self.help_system.has_function(topic) || self.help_system.has_category(topic) {
                    println!("    • Type {} for help on {}", 
                        format!("help {}", topic).bright_cyan(),
                        topic.replace("error.", "").replace("_", " ").bright_white()
                    );
                }
            }
            
            // Always suggest the general error help
            if has_error_topics {
                println!("    • Type {} for comprehensive error guidance", 
                    "help error.fixes".bright_cyan()
                );
            }
        }
    }
    


    /// Handle module debug commands for troubleshooting module resolution
    #[allow(dead_code)]
    fn handle_module_debug_command(&mut self, args: &[&str]) -> Result<(), ReplError> {
        match args.get(0).copied() {
            Some("trace") => {
                self.ovm_interpreter
                    .get_classic_interpreter()
                    .module_debug_config
                    .enable_resolution_tracing = true;
                    println!("Module resolution tracing enabled");
                println!("  Run import statements to see detailed resolution tracing");
            }
            Some("paths") => {
                if let Some(module) = args.get(1) {
                    self.show_module_search_paths(module);
                } else {
                    println!("Usage: :module paths <module_name>");
                }
            }
            Some("list") => {
                self.list_loaded_modules();
            }
            Some("config") => {
                let config = &self.ovm_interpreter
                    .get_classic_interpreter()
                    .module_debug_config;
                
                println!("=== Module Debug Configuration ===");
                println!("  Resolution tracing: {}", 
                    if config.enable_resolution_tracing { "enabled".bright_green() } else { "disabled".bright_red() });
                println!("  Log search paths: {}", 
                    if config.log_search_paths { "enabled".bright_green() } else { "disabled".bright_red() });
                println!("  Show timing: {}", 
                    if config.show_resolution_timing { "enabled".bright_green() } else { "disabled".bright_red() });
                println!("  Verbose errors: {}", 
                    if config.verbose_error_messages { "enabled".bright_green() } else { "disabled".bright_red() });
                
                // Check environment variable
                if std::env::var("OLANG_DEBUG_MODULES").is_ok() {
                    println!("  Environment: OLANG_DEBUG_MODULES is set");
                } else {
                    println!("  Environment: OLANG_DEBUG_MODULES not set");
                    println!("    Tip: export OLANG_DEBUG_MODULES=1 for automatic tracing");
                }
            }
            _ => {
                println!("Module debug commands:");
                println!("  :module trace         - Enable resolution tracing");
                println!("  :module paths <name>  - Show search paths for module");
                println!("  :module list          - List loaded modules");
                println!("  :module config        - Show debug configuration");
            }
        }
        Ok(())
    }

    /// Show what paths would be searched for a given module
    #[allow(dead_code)]
    fn show_module_search_paths(&self, module_path: &str) {
        use std::path::PathBuf;
        
        println!("Module search paths for '{}':", module_path);
        
        let current_dir = match std::env::current_dir() {
            Ok(dir) => dir,
            Err(e) => {
                println!("Error getting current directory: {}", e);
                return;
            }
        };
        
        let candidates = vec![
            // Relative to current directory
            current_dir.join(format!("{}.ol", module_path)),
            current_dir.join(format!("{}/mod.ol", module_path)),
            current_dir.join(format!("{}/index.ol", module_path)),
            
            // Relative to src directory
            current_dir.join("src").join(format!("{}.ol", module_path)),
            current_dir.join("src").join(format!("{}/mod.ol", module_path)),
            current_dir.join("src").join(format!("{}/index.ol", module_path)),
            
            // Absolute path if it looks like one
            PathBuf::from(format!("{}.ol", module_path)),
        ];

        for (i, candidate) in candidates.iter().enumerate() {
            let status = if candidate.exists() { 
                if candidate.is_file() { "" } else { "" }
            } else { 
                "" 
            };
            println!("  {}. {} {}", i + 1, status, candidate.display());
        }
        
        // Check for stdlib module
        let stdlib = crate::stdlib::get_stdlib();
        if stdlib.contains_key(module_path) {
            println!("Available as stdlib module: {}", module_path);
        }
    }

    /// List all currently loaded modules
    #[allow(dead_code)]
    fn list_loaded_modules(&mut self) {
        println!("=== Loaded Modules ===");
        
        let env = self.ovm_interpreter.get_classic_interpreter().get_environment();
        let stdlib = crate::stdlib::get_stdlib();
        
        println!("\nStandard Library Modules:");
        for module_name in stdlib.keys() {
            if env.get(module_name).is_some() {
                println!("{}", module_name.bright_cyan());
            } else {
                println!("{} (available but not loaded)", module_name.bright_black());
            }
        }
        
        println!("\nUser Modules:");
        let mut found_user_modules = false;
        for (name, value) in env.get_all_variables() {
            if !stdlib.contains_key(name as &str) {
                if let crate::ast::Value::Struct { type_name, .. } = value {
                    if type_name == "Module" {
                        println!("{}", name.bright_yellow());
                        found_user_modules = true;
                    }
                }
            }
        }
        
        if !found_user_modules {
            println!("  (no user modules loaded)");
        }
    }

    fn show_interpreter_error(&mut self, interpreter_error: &InterpreterError) {
        match interpreter_error {
            InterpreterError::UndefinedVariable { name } => {
                println!("  {}: Variable '{}' is not defined", 
                    "Undefined Variable".bright_red().bold(), 
                    name.bright_yellow()
                );
                
                // Suggest similar variables
                let user_vars = self.ovm_interpreter.get_classic_interpreter().get_user_variables();
                let mut suggestions = Vec::new();
                
                for var_name in user_vars.keys() {
                    if Self::is_similar_name(name, var_name) {
                        suggestions.push(var_name.clone());
                    }
                }
                
                if !suggestions.is_empty() {
                    println!("\n  {}", "Did you mean:".bright_cyan().bold());
                    for suggestion in suggestions {
                        println!("    • {}", suggestion.bright_green());
                    }
                } else {
                    println!("\n  {}: Use {} to see available variables", 
                        "Hint".bright_blue().bold(), 
                        ":env".bright_cyan()
                    );
                }
            }
            InterpreterError::TypeError { message } => {
                println!("  {}: {}", "Type Error".bright_red().bold(), message);
                
                // Type-specific suggestions
                if message.contains("division by zero") {
                    println!("\n  {}: Check that denominators are not zero before division", 
                        "Hint".bright_blue().bold());
                } else if message.contains("cannot convert") {
                    println!("\n  {}: Use type conversion functions like {} or {}", 
                        "Hint".bright_blue().bold(),
                        "to_int()".bright_cyan(),
                        "to_float()".bright_cyan()
                    );
                } else if message.contains("invalid binary operation") {
                    println!("\n  {}: Check that operands are compatible types", 
                        "Hint".bright_blue().bold());
                    println!("    • Numbers: {} with {}", "42".bright_green(), "3.14".bright_green());
                    println!("    • Strings: {} with {}", "\"hello\"".bright_green(), "\"world\"".bright_green());
                    println!("    • Booleans: {} with {}", "true".bright_green(), "false".bright_green());
                }
            }
            InterpreterError::RuntimeError { message } => {
                println!("  {}: {}", "Runtime Error".bright_red().bold(), message);
                
                // Runtime-specific suggestions
                if message.contains("break") {
                    println!("\n  {}: {} can only be used inside loops", 
                        "Hint".bright_blue().bold(),
                        "break".bright_cyan()
                    );
                } else if message.contains("continue") {
                    println!("\n  {}: {} can only be used inside loops", 
                        "Hint".bright_blue().bold(),
                        "continue".bright_cyan()
                    );
                }
            }
            InterpreterError::ArityMismatch { expected, got } => {
                println!("  {}: Expected {} arguments, got {}", 
                    "Arity Mismatch".bright_red().bold(), 
                    expected.to_string().bright_cyan(),
                    got.to_string().bright_yellow()
                );
                
                println!("\n  {}: Check the function signature and provide the correct number of arguments", 
                    "Hint".bright_blue().bold());
            }
            InterpreterError::PatternMatchFailed => {
                println!("  {}: Pattern matching failed", 
                    "Pattern Match Error".bright_red().bold());
                
                println!("\n  {}: Ensure the pattern matches the structure of the value", 
                    "Hint".bright_blue().bold());
                println!("    • Use {} to match any value", "_".bright_cyan());
                println!("    • Use {} or {} for Result types", "Ok(value)".bright_cyan(), "Err(error)".bright_cyan());
            }
            _ => {
                // Handle all other error types (lazy evaluation errors, etc.)
                println!("  {}: {}", "Error".bright_red().bold(), interpreter_error);
                
                println!("\n  {}: This appears to be a system-level error", 
                    "Hint".bright_blue().bold());
            }
        }
    }
    
    fn is_similar_name(target: &str, candidate: &str) -> bool {
        // Simple similarity check - could be enhanced with edit distance
        if target.len() < 3 || candidate.len() < 3 {
            return false;
        }
        
        // Check if one contains the other
        if target.contains(candidate) || candidate.contains(target) {
            return true;
        }
        
        // Check for common prefixes/suffixes
        let target_lower = target.to_lowercase();
        let candidate_lower = candidate.to_lowercase();
        
        // Check for similar starts
        if target_lower.starts_with(&candidate_lower[..2]) || 
           candidate_lower.starts_with(&target_lower[..2]) {
            return true;
        }
        
        // Check for similar endings
        if target_lower.ends_with(&candidate_lower[candidate_lower.len()-2..]) || 
           candidate_lower.ends_with(&target_lower[target_lower.len()-2..]) {
            return true;
        }
        
        false
    }
    
    fn show_suggestion(&self, suggestion: &ErrorSuggestion) {
        let severity_icon = match suggestion.severity {
            SuggestionSeverity::Error => "ERROR",
            SuggestionSeverity::Warning => "WARNING",
            SuggestionSeverity::Hint => "HINT",
            SuggestionSeverity::Info => "INFO",
        };
        
        let severity_color = match suggestion.severity {
            SuggestionSeverity::Error => "red",
            SuggestionSeverity::Warning => "yellow",
            SuggestionSeverity::Hint => "cyan",
            SuggestionSeverity::Info => "blue",
        };
        
        println!("    {} {}", severity_icon, suggestion.message.color(severity_color).bold());
        
        if let Some(fix) = &suggestion.fix {
            println!("      {}: {}", "Fix".bright_green().bold(), fix);
        }
        
        if let Some(help) = &suggestion.help {
            println!("      {}: {}", "Help".bright_blue().bold(), help);
        }
    }
    
    /// Build help context from current REPL state
    fn build_help_context(&mut self) -> HelpContext {
        let recent_commands = self.command_history.iter()
            .rev()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        
        let current_variables = self.ovm_interpreter
            .get_classic_interpreter()
            .get_user_variables()
            .keys()
            .cloned()
            .collect();
        
        // Get last error from debug state if available
        let last_error = None; // TODO: Implement error tracking
        
        // Determine working category from recent commands
        let current_working_category = self.determine_working_category(&recent_commands);
        
        HelpContext {
            recent_commands,
            current_variables,
            last_error,
            current_working_category,
        }
    }
    
    /// Determine the current working category based on recent commands
    fn determine_working_category(&self, recent_commands: &[String]) -> Option<String> {
        for command in recent_commands {
            if command.contains("map") || command.contains("filter") || command.contains("reduce") {
                return Some("List".to_string());
            }
            if command.contains("fs.") {
                return Some("File System".to_string());
            }
            if command.contains("http.") {
                return Some("HTTP".to_string());
            }
            if command.contains("math.") {
                return Some("Math".to_string());
            }
            if command.contains("json.") {
                return Some("JSON".to_string());
            }
            if command.contains("random.") {
                return Some("Random".to_string());
            }
        }
        None
    }
    
    /// Run an interactive tutorial
    fn run_interactive_tutorial(&mut self, tutorial: &crate::help::Tutorial) -> Result<(), ReplError> {
        println!("\n{}Starting Interactive Tutorial: {}{}", Colors::BOLD, tutorial.name, Colors::RESET);
        println!("{}", tutorial.description);
        println!("{}Difficulty: {} | Estimated Time: {}{}", Colors::DIM, tutorial.difficulty, tutorial.estimated_time, Colors::RESET);
        println!();
        
        for (i, step) in tutorial.steps.iter().enumerate() {
            println!("{}=== Step {}/{}: {} ==={}", Colors::CYAN, i + 1, tutorial.steps.len(), step.title, Colors::RESET);
            println!("{}", step.description);
            println!();
            
            // Show the code to try
            println!("{}Code to try:{}", Colors::MAGENTA, Colors::RESET);
            println!("{}{}{}", Colors::BLUE, step.code, Colors::RESET);
            println!();
            
            // Show expected output
                            println!("{}Expected output:{}", Colors::GREEN, Colors::RESET);
            println!("{}{}{}", Colors::GREEN, step.expected_output, Colors::RESET);
            println!();
            
            // Interactive prompt
            println!("{}Options:{}", Colors::YELLOW, Colors::RESET);
            println!("  {}r{} - Run the code automatically", Colors::BLUE, Colors::RESET);
            println!("  {}t{} - Try it yourself (enter your own code)", Colors::BLUE, Colors::RESET);
            println!("  {}e{} - Show explanation", Colors::BLUE, Colors::RESET);
            println!("  {}h{} - Show hints", Colors::BLUE, Colors::RESET);
            println!("  {}n{} - Next step", Colors::BLUE, Colors::RESET);
            println!("  {}q{} - Quit tutorial", Colors::BLUE, Colors::RESET);
            
            loop {
                print!("{}Tutorial> {}", Colors::CYAN, Colors::RESET);
                std::io::stdout().flush().unwrap();
                
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).unwrap();
                let input = input.trim();
                
                match input {
                    "r" => {
                        // Run the tutorial code
                        println!("{}Running tutorial code...{}", Colors::YELLOW, Colors::RESET);
                        match self.eval_line(&step.code) {
                            Ok(value) => {
                                if value != Value::Unit {
                                    println!("{}", value);
                                }
                                println!("{}Code executed successfully!{}", Colors::GREEN, Colors::RESET);
                            }
                            Err(e) => {
                                println!("{}ERROR executing code:{}", Colors::RED, Colors::RESET);
                                println!("{}", e);
                            }
                        }
                        break;
                    }
                    "t" => {
                        println!("{}Enter your code (press Enter twice to execute):{}", Colors::YELLOW, Colors::RESET);
                        let mut user_code = String::new();
                        loop {
                            let mut line = String::new();
                            std::io::stdin().read_line(&mut line).unwrap();
                            if line.trim().is_empty() {
                                break;
                            }
                            user_code.push_str(&line);
                        }
                        
                        if !user_code.trim().is_empty() {
                            match self.eval_line(&user_code) {
                                Ok(value) => {
                                    if value != Value::Unit {
                                        println!("{}", value);
                                    }
                                    println!("{}Great job!{}", Colors::GREEN, Colors::RESET);
                                }
                                Err(e) => {
                                    println!("{}Error:{}", Colors::RED, Colors::RESET);
                                    println!("{}", e);
                                    println!("{} Try again or use 'r' to run the tutorial code{}", Colors::YELLOW, Colors::RESET);
                                    continue;
                                }
                            }
                        }
                        break;
                    }
                    "e" => {
                        println!("{}📖 Explanation:{}", Colors::YELLOW, Colors::RESET);
                        println!("{}", step.explanation);
                        println!();
                    }
                    "h" => {
                        if !step.hints.is_empty() {
                            println!("{}Hints:{}", Colors::CYAN, Colors::RESET);
                            for hint in &step.hints {
                                println!("  • {}", hint);
                            }
                        } else {
                            println!("{}No hints available for this step.{}", Colors::DIM, Colors::RESET);
                        }
                        println!();
                    }
                    "n" => {
                        println!("{}Moving to next step...{}", Colors::CYAN, Colors::RESET);
                        break;
                    }
                    "q" => {
                        println!("{}Exiting tutorial. Progress saved.{}", Colors::YELLOW, Colors::RESET);
                        return Ok(());
                    }
                    _ => {
                        println!("{}Invalid option. Use r/t/e/h/n/q{}", Colors::RED, Colors::RESET);
                    }
                }
            }
            
            println!();
        }
        
        // Tutorial completion
        println!("{}Congratulations! You've completed the '{}' tutorial!{}", Colors::GREEN, tutorial.name, Colors::RESET);
        println!("{}You've learned:{}", Colors::YELLOW, Colors::RESET);
        for step in &tutorial.steps {
            println!("  {}", step.title);
        }
        
        // Suggest next steps
        if !tutorial.prerequisites.is_empty() {
            println!("\n{}Consider these related tutorials:{}", Colors::CYAN, Colors::RESET);
            let all_tutorials = self.help_system.get_tutorials();
            for other_tutorial in all_tutorials {
                if other_tutorial.prerequisites.contains(&tutorial.name) {
                    println!("  • {} ({})", other_tutorial.name, other_tutorial.difficulty);
                }
            }
        }
        
        Ok(())
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

    fn get_variable(&self, _name: &str) -> Option<Value> {
        // Note: This would need &mut self to work properly, but keeping for compatibility
        None // Simplified for now
    }
}

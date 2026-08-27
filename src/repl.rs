use crate::ast::Value;
use crate::help::{Colors, HelpContext, HelpSystem, SearchFilters};
use crate::interpreter::InterpreterError;
use crate::parser::{ErrorSuggestion, ParseError, Parser, SuggestionSeverity};
use crate::version::VERSION;
use colored::*;
use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Config, Context, Editor, Helper, history::DefaultHistory};
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
        self.profiling_data
            .entry(expression)
            .or_default()
            .push(time_ms);
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
                changes.push(format!("Variable '{}' removed", var_name.bright_yellow()));
            }
        }

        changes
    }
}

fn olang_interpreter_levenshtein(a: &str, b: &str) -> usize {
    crate::interpreter::IntuitiveErrorFormatter::levenshtein_distance(a, b)
}

/// All REPL colon-commands, used for TAB completion of the command word.
const REPL_COMMANDS: &[&str] = &[
    ":help",
    ":quit",
    ":env",
    ":clear",
    ":ovm",
    ":version",
    ":history",
    ":type",
    ":time",
    ":memory",
    ":stats",
    ":parallel",
    ":run",
    ":debug",
    ":watch",
    ":inspect",
    ":trace",
    ":set",
    ":stack",
    ":profile",
    ":config",
    ":benchmark",
    ":search",
    ":tutorial",
    ":tutorial_run",
    ":contextual_help",
    ":help_advanced",
    ":sh",
    ":cd",
    ":pwd",
    ":ls",
];

/// Commands whose arguments are file paths (get filename completion).
const PATH_COMMANDS: &[&str] = &[":sh", ":cd", ":ls", ":run"];

/// Rustyline helper: completes REPL command names, file paths (after shell /
/// path-taking commands and inside double-quoted strings), and known
/// function / variable identifiers.
struct ReplHelper {
    file_completer: FilenameCompleter,
    /// Builtin + stdlib function names (static after startup)
    function_names: Vec<String>,
    /// User-defined variables/functions, refreshed after each evaluation
    user_identifiers: Vec<String>,
}

impl ReplHelper {
    fn new(function_names: Vec<String>) -> Self {
        Self {
            file_completer: FilenameCompleter::new(),
            function_names,
            user_identifiers: Vec::new(),
        }
    }

    /// Is the cursor inside an unclosed double-quoted string literal?
    fn in_string_literal(before_cursor: &str) -> bool {
        let mut in_string = false;
        let mut escaped = false;
        for ch in before_cursor.chars() {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = !in_string;
            }
        }
        in_string
    }

    /// The identifier-ish token ending at the cursor (letters, digits, `_`, `.`).
    fn current_identifier(before_cursor: &str) -> &str {
        let start = before_cursor
            .rfind(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.')
            .map(|i| i + 1)
            .unwrap_or(0);
        &before_cursor[start..]
    }

    fn complete_identifiers(&self, line: &str, pos: usize) -> (usize, Vec<Pair>) {
        let before = &line[..pos];
        let word = Self::current_identifier(before);
        let start = pos - word.len();
        if word.is_empty() {
            return (start, Vec::new());
        }
        let mut candidates: Vec<Pair> = self
            .function_names
            .iter()
            .chain(self.user_identifiers.iter())
            .filter(|name| name.starts_with(word) && name.as_str() != word)
            .map(|name| Pair {
                display: name.clone(),
                replacement: name.clone(),
            })
            .collect();
        candidates.sort_by(|a, b| a.display.cmp(&b.display));
        candidates.dedup_by(|a, b| a.display == b.display);
        (start, candidates)
    }

    fn complete_commands(&self, word: &str) -> Vec<Pair> {
        REPL_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(word))
            .map(|cmd| Pair {
                display: cmd.to_string(),
                replacement: cmd.to_string(),
            })
            .collect()
    }
}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let before = &line[..pos];

        // Shell shortcut: `!command args...` — complete paths everywhere
        if line.starts_with('!') {
            return self.file_completer.complete(line, pos, ctx);
        }

        if line.starts_with(':') {
            let first_word_end = line.find(char::is_whitespace).unwrap_or(line.len());
            if pos <= first_word_end {
                // Completing the command name itself
                return Ok((0, self.complete_commands(before)));
            }
            let command = &line[..first_word_end];
            if PATH_COMMANDS.contains(&command) {
                return self.file_completer.complete(line, pos, ctx);
            }
            if command == ":help" || command == ":type" || command == ":search" {
                return Ok(self.complete_identifiers(line, pos));
            }
            // Default for other commands: try file paths (e.g. `:config load <file>`)
            return self.file_completer.complete(line, pos, ctx);
        }

        // Regular olang code: complete file paths inside string literals
        // (fs.read("src/ma<TAB>), identifiers elsewhere
        if Self::in_string_literal(before) {
            return self.file_completer.complete(line, pos, ctx);
        }

        Ok(self.complete_identifiers(line, pos))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;
}

/// Live syntax color for the input line: keywords, literals, comments,
/// and the pipeline arrows, re-rendered as the user types. The rendered
/// string must keep the original's display width, so colors only wrap
/// the original characters — nothing is inserted or dropped.
impl Highlighter for ReplHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> std::borrow::Cow<'l, str> {
        if !colored::control::SHOULD_COLORIZE.should_colorize() {
            return std::borrow::Cow::Borrowed(line);
        }
        std::borrow::Cow::Owned(highlight_source(line))
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> std::borrow::Cow<'b, str> {
        if !colored::control::SHOULD_COLORIZE.should_colorize() {
            return std::borrow::Cow::Borrowed(prompt);
        }
        if prompt.trim_start().starts_with("...") {
            std::borrow::Cow::Owned(prompt.dimmed().to_string())
        } else {
            std::borrow::Cow::Owned(prompt.green().bold().to_string())
        }
    }

    fn highlight_char(&self, _line: &str, _pos: usize, kind: CmdKind) -> bool {
        !matches!(kind, CmdKind::MoveCursor)
    }
}
impl Validator for ReplHelper {}
impl Helper for ReplHelper {}

pub struct Repl {
    editor: Editor<ReplHelper, DefaultHistory>,
    interpreter: crate::interpreter::Interpreter,
    parser: Parser,
    #[allow(dead_code)] // consumed at construction; kept for future diagnostics
    verbose: bool,
    history_file: String,
    /// Meta fn declarations entered this session, kept as source. A meta
    /// fn is stripped from every expanded program, so without this a
    /// macro defined on one line would be gone by the next. Each input
    /// that uses macros is expanded with these prepended.
    meta_prelude: Vec<String>,
    help_system: HelpSystem,
    config: ReplConfig,
    multiline_mode: bool,
    multiline_buffer: String,
    command_history: Vec<String>,
    debugger: InteractiveDebugger,
    /// The rebind tip fires once per session: the first time a bundled
    /// collection's write operation is called without rebinding its
    /// handle, the REPL explains the convention instead of letting the
    /// unchanged variable look like a bug.
    rebind_tip_shown: bool,
    /// `///` doc comments from declarations entered this session, so
    /// `:help name` answers for the user's own functions the moment
    /// they are defined. Later re-definitions replace earlier ones.
    session_docs: Vec<crate::tools::doc::Item>,
    /// A `///` block entered on its own line(s), waiting for the
    /// declaration that follows on the next input.
    pending_doc_lines: Vec<String>,
    /// Every file this session has loaded a module from. The REPL
    /// clears the interpreter's module cache after each input, so the
    /// `:help` doc scan keeps its own record of where code came from.
    loaded_doc_files: std::collections::BTreeSet<std::path::PathBuf>,
    /// Registered module name → file, from the same harvest. This is
    /// what maps `:help geometry.point` to the package whose entry
    /// file is `index.ol` — a name a file stem alone cannot answer.
    loaded_doc_modules: std::collections::BTreeMap<String, std::path::PathBuf>,
}

impl Repl {
    pub fn new(verbose: bool) -> Result<Self, ReplError> {
        Self::with_tier(verbose, true)
    }

    pub fn with_tier(verbose: bool, enable_tier: bool) -> Result<Self, ReplError> {
        // Initialize parallelization for optimal performance
        match crate::parallel::initialize_parallelization(None) {
            Err(e) => {
                if verbose {
                    eprintln!("Warning: Failed to initialize parallel processing: {}", e);
                }
            }
            _ => {
                if verbose {
                    println!(
                        "Multi-threading enabled: {} CPU cores detected",
                        num_cpus::get()
                    );
                }
            }
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

        let mut editor = Editor::<ReplHelper, DefaultHistory>::with_config(config)?;

        // TAB completion: REPL commands, file paths, and known identifiers
        let help_system = HelpSystem::new();
        let mut function_names = help_system.get_function_names();
        function_names.extend(
            crate::builtin::BuiltinFunctions::new()
                .get_functions()
                .keys()
                .cloned(),
        );
        // Enumerate every registered stdlib module so `module.function` names
        // (str.trim, re.find_all, col.min_by, ...) complete without a manual
        // list — a new module is picked up automatically.
        for (module_name, module_value) in crate::stdlib::get_stdlib() {
            function_names.push(module_name.clone());
            if let Value::Struct { fields, .. } = module_value {
                for field in fields.values() {
                    if let Value::Builtin(func) = field {
                        // func.name is already "module.function"
                        function_names.push(func.name.clone());
                    }
                }
            }
        }
        function_names.sort();
        function_names.dedup();
        editor.set_helper(Some(ReplHelper::new(function_names)));

        // Load history if available
        let history_file = format!(
            "{}/.olang_history",
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        );
        if let Err(e) = editor.load_history(&history_file)
            && verbose
        {
            eprintln!("Could not load history: {}", e);
        }

        // Create OVM configuration with auto mode (OVM ENABLED by default for performance)
        let mut interpreter = crate::interpreter::Interpreter::new();

        if enable_tier {
            // Promote eligible functions to bytecode on their first call
            interpreter.enable_bytecode_tier(1, verbose);
        }

        Ok(Self {
            editor,
            interpreter,
            parser: Parser::new(),
            verbose,
            history_file,
            meta_prelude: Vec::new(),
            help_system,
            config: ReplConfig::default(),
            multiline_mode: false,
            multiline_buffer: String::new(),
            command_history: Vec::new(),
            debugger: InteractiveDebugger::new(),
            rebind_tip_shown: false,
            session_docs: Vec::new(),
            pending_doc_lines: Vec::new(),
            loaded_doc_files: std::collections::BTreeSet::new(),
            loaded_doc_modules: std::collections::BTreeMap::new(),
        })
    }

    /// If the working directory is inside a package, resolve its dependencies
    /// and hand the interpreter the map so `use <dep>` works in the REPL.
    /// Returns the package name when one was loaded. Called at startup and
    /// after `:cd`, since packages are resolved relative to the directory.
    fn load_packages(&mut self) -> Option<String> {
        let cwd = std::env::current_dir().ok()?;
        let root = crate::pkg::manifest::Manifest::find_root(&cwd)?;
        // Startup / :cd sets a fresh context (replace the map).
        self.load_package_at(&root, true)
    }

    /// Resolve the package rooted at `root`, add it (and its dependencies, and
    /// itself by name) to the interpreter's dependency map. `replace` clears
    /// the existing map first (a fresh context, for startup/:cd); otherwise
    /// the package is merged in, so `:pkg load <path>` is additive and you can
    /// pull in packages by path without cd-ing into them.
    fn load_package_at(&mut self, root: &std::path::Path, replace: bool) -> Option<String> {
        let name = crate::pkg::manifest::Manifest::load(root)
            .ok()
            .map(|m| m.package.name);
        let opts = crate::pkg::InstallOptions {
            registry: std::env::var("OLANG_REGISTRY")
                .ok()
                .map(std::path::PathBuf::from),
            ..Default::default()
        };
        match crate::pkg::install(root, &opts) {
            Ok(map) => {
                let mut map: std::collections::HashMap<_, _> = map.into_iter().collect();
                // Make the package referable by its own name, so you can test a
                // package in its own REPL (`use geometry { ... }` from inside
                // geometry resolves to its own root module).
                if let Some(pkg_name) = &name {
                    map.entry(pkg_name.clone())
                        .or_insert_with(|| root.to_path_buf());
                }
                if replace {
                    // Set the file context to the package root so relative
                    // `use` of sibling modules also resolves.
                    self.interpreter.set_current_file(&root.join("olang.toml"));
                    self.interpreter.set_dependency_map(map);
                } else {
                    self.interpreter.add_to_dependency_map(map);
                }
                name
            }
            Err(e) => {
                eprintln!("{}: {}", "package resolution".bright_yellow(), e);
                None
            }
        }
    }

    pub fn run(&mut self) -> Result<(), ReplError> {
        println!(
            "{} {} — {} for help · {} or ^D to exit",
            "olang".bold(),
            VERSION,
            ":help".cyan(),
            "quit".cyan()
        );
        if let Some(pkg) = self.load_packages() {
            println!(
                "Package '{}' loaded — its dependencies are available via `use`",
                pkg.bright_green()
            );
        }
        println!();

        loop {
            self.refresh_completions();

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
                            self.after_eval(&code, &value);
                            if value != Value::Unit {
                                if self.config.show_types {
                                    println!(
                                        "{} : {}",
                                        repl_format(&value),
                                        Self::get_type_name(&value)
                                    );
                                } else {
                                    repl_print(&value);
                                }
                            }
                        }
                        Err(e) => {
                            self.show_enhanced_error(&e);
                        }
                    }

                    // Clean up module cache to prevent memory accumulation
                    // — after remembering which files it loaded, so
                    // `:help` can still find their doc comments.
                    self.loaded_doc_files
                        .extend(self.interpreter.loaded_module_files());
                    self.loaded_doc_modules
                        .extend(self.interpreter.loaded_modules());
                    self.interpreter.clear_module_cache();
                } else {
                    if !self.multiline_buffer.is_empty() {
                        self.multiline_buffer.push('\n');
                    }
                    self.multiline_buffer.push_str(line);
                }
                continue;
            }

            // Shell escape: `!command` runs through the user's shell
            if let Some(shell_cmd) = line.strip_prefix('!') {
                self.run_shell_command(shell_cmd);
                continue;
            }

            // Handle special commands
            if line.starts_with(':') || line == "quit" {
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
                    self.after_eval(line, &value);
                    if let Some(start) = start_time {
                        let duration = start.elapsed();
                        if value != Value::Unit {
                            if self.config.show_types {
                                println!(
                                    "{} {} {}",
                                    color_value(&value),
                                    ":".dimmed(),
                                    Self::get_type_name(&value).cyan().dimmed()
                                );
                            } else {
                                repl_print(&value);
                            }
                        }
                        println!(
                            "{}",
                            format!("Execution time: {:.2}ms", duration.as_secs_f64() * 1000.0)
                                .dimmed()
                        );
                    } else if value != Value::Unit {
                        if self.config.show_types {
                            println!(
                                "{} {} {}",
                                color_value(&value),
                                ":".dimmed(),
                                Self::get_type_name(&value).cyan().dimmed()
                            );
                        } else {
                            repl_print(&value);
                        }
                    }
                }
                Err(e) => {
                    self.show_enhanced_error(&e);
                }
            }

            // Clean up module cache to prevent memory accumulation —
            // after remembering which files it loaded, so `:help` can
            // still find their doc comments.
            self.loaded_doc_files
                .extend(self.interpreter.loaded_module_files());
            self.loaded_doc_modules
                .extend(self.interpreter.loaded_modules());
            self.interpreter.clear_module_cache();
        }

        // Save history
        if let Err(e) = self.editor.save_history(&self.history_file) {
            eprintln!("Could not save history: {}", e);
        }

        Ok(())
    }

    /// Every file worth scanning for user docs: the session harvest
    /// plus whatever is live in the module cache right now. When
    /// `module` is given, only files behind a matching registered name
    /// (`geometry`, `lib.money` matching `money`) or file stem — plus,
    /// for a package entry, its `lib/` siblings, where re-exported
    /// declarations actually live.
    fn doc_candidate_files(&self, module: Option<&str>) -> Vec<std::path::PathBuf> {
        let mut pairs: Vec<(String, std::path::PathBuf)> = self
            .loaded_doc_modules
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        pairs.extend(self.interpreter.loaded_modules());
        for f in self
            .loaded_doc_files
            .iter()
            .cloned()
            .chain(self.interpreter.loaded_module_files())
        {
            pairs.push((String::new(), f));
        }
        let mut out: Vec<std::path::PathBuf> = Vec::new();
        let mut push = |p: std::path::PathBuf, out: &mut Vec<std::path::PathBuf>| {
            if !out.contains(&p) {
                out.push(p);
            }
        };
        for (key, path) in pairs {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let matched = match module {
                None => true,
                Some(m) => key == m || key.ends_with(&format!(".{}", m)) || stem == m,
            };
            if !matched {
                continue;
            }
            push(path.clone(), &mut out);
            // A package's surface is often re-exports: the declarations
            // (and their /// docs) live beside the entry file in lib/.
            if let Some(dir) = path.parent() {
                let lib = dir.join("lib");
                if let Ok(entries) = std::fs::read_dir(&lib) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.extension().map(|x| x == "ol").unwrap_or(false) {
                            push(p, &mut out);
                        }
                    }
                }
            }
        }
        out
    }

    /// The user's own documentation for `topic`, if any declaration in
    /// scope carries a `///` block with that name: session declarations
    /// first (latest wins), then every file this run has loaded.
    /// `module.name` matches through the registered module name (a
    /// package's entry file is `index.ol` — the stem says nothing), and
    /// an *undocumented* declaration still answers with its signature.
    fn find_user_doc(&self, topic: &str) -> Option<(crate::tools::doc::Item, String)> {
        let (mod_part, name_part) = match topic.rsplit_once('.') {
            Some((m, n)) => (Some(m), n),
            None => (None, topic),
        };
        if mod_part.is_none()
            && let Some(item) = self.session_docs.iter().rev().find(|i| i.name == name_part)
        {
            return Some((item.clone(), "defined this session".to_string()));
        }
        let mut signature_only: Option<(crate::tools::doc::Item, String)> = None;
        for file in self.doc_candidate_files(mod_part) {
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
            if let Some(item) = crate::tools::doc::extract(&source)
                .items
                .into_iter()
                .find(|i| i.name == name_part)
            {
                return Some((item, file.display().to_string()));
            }
            if signature_only.is_none()
                && let Some(item) = crate::tools::doc::declaration_of(&source, name_part)
            {
                signature_only = Some((item, file.display().to_string()));
            }
        }
        signature_only
    }

    /// A loaded module whose registered name or file stem matches
    /// `topic` and which carries a `//!` module note or documented
    /// items — the module view of the user-doc lookup.
    fn find_user_module_doc(
        &self,
        topic: &str,
    ) -> Option<(String, crate::tools::doc::ModuleDoc, String)> {
        for file in self.doc_candidate_files(Some(topic)) {
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
            let md = crate::tools::doc::extract(&source);
            if !md.module_doc.is_empty() || !md.items.is_empty() {
                return Some((topic.to_string(), md, file.display().to_string()));
            }
        }
        None
    }

    /// Render one user-documented declaration in the same visual
    /// language as the builtin help: name and signature, the doc text
    /// as written, and where it came from.
    fn print_user_doc(&self, item: &crate::tools::doc::Item, origin: &str) {
        use crate::help::Colors;
        let shared = if item.shared { "share " } else { "" };
        println!(
            "\n{}{}═══ {} ═══{}",
            Colors::BOLD,
            Colors::CYAN,
            item.name,
            Colors::RESET
        );
        // Type/trait signatures already open with their keyword; only
        // prefix the kind when it adds information (fn, value, ...).
        let kind_prefix = if item.signature.starts_with(&item.kind) {
            String::new()
        } else {
            format!("{} ", item.kind)
        };
        println!(
            "\n  {}{}{}{}{}",
            Colors::GREEN,
            shared,
            kind_prefix,
            Colors::RESET,
            item.signature
        );
        if !item.doc.is_empty() {
            println!();
            for line in item.doc.lines() {
                println!("  {}", line);
            }
        } else {
            println!(
                "\n  {}undocumented — /// lines above the declaration appear here{}",
                Colors::DIM,
                Colors::RESET
            );
        }
        println!("\n  {}{}{}", Colors::DIM, origin, Colors::RESET);
    }

    /// Render a user module: its `//!` note, then each documented item
    /// on one line — the same shape as the builtin module listings.
    fn print_user_module_doc(&self, module: &(String, crate::tools::doc::ModuleDoc, String)) {
        use crate::help::Colors;
        let (name, md, path) = module;
        println!(
            "\n{}{}═══ module {} ═══{}",
            Colors::BOLD,
            Colors::CYAN,
            name,
            Colors::RESET
        );
        if !md.module_doc.is_empty() {
            println!();
            for line in md.module_doc.lines() {
                println!("  {}", line);
            }
        }
        if !md.items.is_empty() {
            println!();
            for item in &md.items {
                let first = item.doc.lines().next().unwrap_or("");
                println!(
                    "  {}{}{}  {}",
                    Colors::GREEN,
                    item.signature,
                    Colors::RESET,
                    first
                );
            }
            println!(
                "\n  {}:help {}.<name> shows any one of them in full{}",
                Colors::DIM,
                name,
                Colors::RESET
            );
        }
        println!(
            "\n  {}documented in: {}{}",
            Colors::DIM,
            path,
            Colors::RESET
        );
    }

    /// Post-evaluation duties shared by the single-line and multiline
    /// paths: `_` tracks the last printed value, and a bundled
    /// collection's write called without the rebind gets the one-time
    /// tip — the unchanged handle is the convention, not a bug.
    fn after_eval(&mut self, line: &str, value: &Value) {
        if line.contains("///") || !self.pending_doc_lines.is_empty() {
            let combined = if self.pending_doc_lines.is_empty() {
                line.to_string()
            } else {
                format!("{}\n{}", self.pending_doc_lines.join("\n"), line)
            };
            let extracted = crate::tools::doc::extract(&combined);
            let only_comments = line
                .lines()
                .all(|l| l.trim_start().starts_with("///") || l.trim().is_empty());
            if extracted.items.is_empty() && only_comments {
                // A doc block on its own: hold it for the declaration
                // the next input brings.
                self.pending_doc_lines.push(line.to_string());
            } else {
                for item in extracted.items {
                    self.session_docs.retain(|i| i.name != item.name);
                    self.session_docs.push(item);
                }
                self.pending_doc_lines.clear();
            }
        }
        if *value != Value::Unit {
            // `it` is the last printed result — `_` would collide with
            // the wildcard pattern in the grammar.
            self.interpreter.define_global("it", value.clone());
        }
        if !self.rebind_tip_shown && Self::is_unrebound_collection_write(line) {
            self.rebind_tip_shown = true;
            println!(
                "{}",
                "tip: collection operations return the new handle — rebind it: h = heap.push(h, ...)"
                    .dimmed()
            );
        }
    }

    /// A line that calls a bundled collection's *write* operation but
    /// assigns nothing: `heap.push(h, 1, 2)` rather than
    /// `h = heap.push(h, 1, 2)`. Conservative on purpose — any `=` on
    /// the line, or a read-only operation, and it stays quiet.
    fn is_unrebound_collection_write(line: &str) -> bool {
        if line.contains('=') {
            return false;
        }
        let rest = line.trim_start();
        let rest = rest.strip_prefix("collections.").unwrap_or(rest);
        const WRITERS: &[(&str, &[&str])] = &[
            ("heap.", &["push", "pop", "from_lists"]),
            (
                "deque.",
                &["push_back", "push_front", "pop_back", "pop_front"],
            ),
            ("table.", &["put", "remove"]),
            ("dsu.", &["union"]),
            ("bitset.", &["add", "remove"]),
        ];
        for (module, ops) in WRITERS {
            if let Some(after) = rest.strip_prefix(module) {
                return ops
                    .iter()
                    .any(|op| after.strip_prefix(op).is_some_and(|t| t.starts_with('(')));
            }
        }
        false
    }

    /// Refresh TAB-completion candidates with the current user-defined
    /// variables and functions.
    fn refresh_completions(&mut self) {
        let names: Vec<String> = self
            .interpreter
            .get_user_variables()
            .keys()
            .cloned()
            .collect();
        if let Some(helper) = self.editor.helper_mut() {
            helper.user_identifiers = names;
        }
    }

    /// Run a command through the user's shell, streaming its output.
    /// `cd` is intercepted so it changes the REPL's own working directory.
    fn run_shell_command(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            println!("Usage: :sh <command>   (or !<command>)");
            return;
        }

        if cmd == "cd" || cmd.starts_with("cd ") {
            let target = cmd.strip_prefix("cd").unwrap_or("").trim();
            self.change_directory(target);
            return;
        }

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        match std::process::Command::new(&shell)
            .arg("-c")
            .arg(cmd)
            .status()
        {
            Ok(status) => {
                if !status.success() {
                    match status.code() {
                        Some(code) => println!("{}", format!("(exit code {})", code).bright_red()),
                        None => println!("{}", "(terminated by signal)".bright_red()),
                    }
                }
            }
            Err(e) => eprintln!("Failed to run '{}' via {}: {}", cmd, shell, e),
        }
    }

    /// Change the REPL's working directory (with `~` expansion); empty target
    /// goes home.
    fn change_directory(&mut self, target: &str) {
        let target = if target.is_empty() { "~" } else { target };
        let expanded = if target == "~" || target.starts_with("~/") {
            match std::env::var("HOME") {
                Ok(home) => target.replacen('~', &home, 1),
                Err(_) => target.to_string(),
            }
        } else {
            target.to_string()
        };

        match std::env::set_current_dir(&expanded) {
            Ok(()) => {
                if let Ok(cwd) = std::env::current_dir() {
                    println!("{}", cwd.display().to_string().bright_cyan());
                }
                // The package context is directory-relative, so re-resolve.
                if let Some(pkg) = self.load_packages() {
                    println!("Package '{}' loaded", pkg.bright_green());
                }
            }
            Err(e) => eprintln!("cd: {}: {}", expanded, e),
        }
    }

    /// Render help for a bound module: its name and callable functions. For a
    /// function whose full name (`module.fn`) has a static doc entry, `:help
    /// module.fn` shows the detail; this lists what is available.
    fn show_module_help(&self, name: &str, members: &[String]) {
        println!(
            "\n{} {}",
            "═══ Module:".bright_cyan().bold(),
            format!("{} ═══", name).bright_cyan().bold()
        );
        println!(
            "  {} function(s), call as {}:",
            members.len().to_string().bright_white(),
            format!("{}.<fn>(...)", name).bright_cyan()
        );
        for chunk in members.chunks(4) {
            println!("    {}", chunk.join(", "));
        }
        // The embedded olang mirrors (colx, mathx) reproduce a native module;
        // point at the native module's detailed per-function docs.
        let mirror_of = match name {
            "colx" => Some("col"),
            "mathx" => Some("math"),
            _ => None,
        };
        if let Some(native) = mirror_of {
            println!(
                "\n  {} {} mirrors the native {} module — see {} for details on any function.",
                "note:".bright_yellow(),
                name,
                native.bright_green(),
                format!("':help {}.<fn>'", native).bright_cyan()
            );
        }
        println!(
            "\n  Try {} for a function's documentation.",
            format!(":help {}.<fn>", name).bright_cyan()
        );
    }

    fn handle_command(&mut self, command: &str) -> Result<(), ReplError> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        let command_name = parts[0];

        match command_name {
            ":help" => {
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
                                if let Some(tutorial) = self.help_system.get_tutorial(tutorial_name)
                                {
                                    println!("{}", self.help_system.format_tutorial(tutorial));
                                } else {
                                    println!(
                                        "Tutorial '{}' not found. Use ':help tutorials' to see available tutorials.",
                                        tutorial_name
                                    );
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
                            // Exact function name first (`:help proc.spawn`, `:help len`).
                            // A bare module name (`proc`) is deliberately NOT matched
                            // here — has_exact_function skips fuzzy matching so the
                            // module-listing branch below can run instead of resolving
                            // to a single arbitrary member.
                            if self.help_system.has_exact_function(topic) {
                                println!("{}", self.help_system.show_function_help(topic));
                            } else if let Some((item, origin)) = self.find_user_doc(topic) {
                                // The user's own `///` doc, from this
                                // session or any loaded file.
                                self.print_user_doc(&item, &origin);
                            } else {
                                let module_fns = self.help_system.functions_in_module(topic);
                                if !module_fns.is_empty() {
                                    // A documented module/namespace — list every member.
                                    // Module names are all-lowercase; echo the canonical
                                    // form, not the user's casing (`:help STATS` must
                                    // not advertise `STATS.<fn>(...)`).
                                    self.show_module_help(&topic.to_lowercase(), &module_fns);
                                } else if let Some(user_mod) = self.find_user_module_doc(topic) {
                                    self.print_user_module_doc(&user_mod);
                                } else if let Some(members) = self.interpreter.module_members(topic)
                                {
                                    // An imported/bound module with no static docs —
                                    // list its functions from the live environment.
                                    self.show_module_help(topic, &members);
                                } else if self.help_system.has_category(topic) {
                                    println!("{}", self.help_system.show_category(topic));
                                } else if self.help_system.has_function(topic) {
                                    // Fuzzy function match as a last resort (typos).
                                    println!("{}", self.help_system.show_function_help(topic));
                                } else {
                                    // Try advanced search if direct lookup fails
                                    let results = self.help_system.search(topic, None);
                                    if !results.is_empty() {
                                        println!(
                                            "{}",
                                            self.help_system.format_search_results(&results)
                                        );
                                    } else {
                                        println!(
                                            "No help found for '{}'. Try 'help search {}' for advanced search.",
                                            topic, topic
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "quit" | ":quit" => {
                std::process::exit(0);
            }
            ":sh" => {
                // Preserve the raw remainder (quotes, pipes, etc.)
                let cmd = command[":sh".len()..].trim().to_string();
                self.run_shell_command(&cmd);
            }
            ":pwd" => match std::env::current_dir() {
                Ok(cwd) => println!("{}", cwd.display()),
                Err(e) => eprintln!("pwd: {}", e),
            },
            ":cd" => {
                let target = command[":cd".len()..].trim().to_string();
                self.change_directory(&target);
            }
            ":ls" => {
                let args = command[":ls".len()..].trim();
                self.run_shell_command(&format!("ls {}", args));
            }

            ":pkg" => {
                if parts.len() > 2 && parts[1] == "load" {
                    // `:pkg load <path>` — make a package at an explicit path
                    // available by name, without cd-ing into it. Additive.
                    let raw = parts[2..].join(" ");
                    let expanded = if raw == "~" || raw.starts_with("~/") {
                        match std::env::var("HOME") {
                            Ok(home) => raw.replacen('~', &home, 1),
                            Err(_) => raw.clone(),
                        }
                    } else {
                        raw.clone()
                    };
                    let path = std::path::Path::new(&expanded);
                    match crate::pkg::manifest::Manifest::find_root(path) {
                        Some(root) => {
                            if let Some(name) = self.load_package_at(&root, false) {
                                // Only promise `use <name>` when a public
                                // root module actually exists. An
                                // application package (main.ol + lib/) has
                                // none, and telling the reader to import it
                                // by name sends them into an error.
                                let has_root = ["index.ol", "mod.ol", "src/index.ol"]
                                    .iter()
                                    .map(|f| root.join(f))
                                    .chain(std::iter::once(root.join(format!("{name}.ol"))))
                                    .any(|p| p.exists());
                                println!(
                                    "Loaded package '{}' from {}",
                                    name.bright_green(),
                                    root.display()
                                );
                                if has_root {
                                    println!("  import it with `use {name}`");
                                } else {
                                    let modules =
                                        crate::interpreter::Interpreter::importable_modules(&root);
                                    if modules.is_empty() {
                                        println!(
                                            "  {}",
                                            "no importable modules — this package is an entry point, not a library"
                                                .bright_yellow()
                                        );
                                    } else {
                                        println!(
                                            "  no root module, so `use {name}` will not resolve. Import a module directly:"
                                        );
                                        for m in modules.iter().take(8) {
                                            println!("    use {name}.{m}");
                                        }
                                        if modules.len() > 8 {
                                            println!("    … and {} more", modules.len() - 8);
                                        }
                                    }
                                }
                            }
                        }
                        None => println!(
                            "{}: no olang.toml at or above '{}'",
                            "pkg load".bright_yellow(),
                            expanded
                        ),
                    }
                } else {
                    // `:pkg` — re-resolve the current directory's package.
                    match self.load_packages() {
                        Some(name) => println!(
                            "Package '{}' loaded — dependencies available via `use`",
                            name.bright_green()
                        ),
                        None => println!(
                            "{}",
                            "No package here. Use ':pkg load <path>', or run from a directory with an olang.toml."
                                .bright_yellow()
                        ),
                    }
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
                            self.interpreter.clear_user_environment();
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
                println!("\n{}", "=== Bytecode Tier ===".bright_cyan().bold());
                match self.interpreter.bytecode_tier_stats() {
                    Some(tier) => {
                        println!(
                            "  Status: {}",
                            "enabled (functions compile on first call)".bright_green()
                        );
                        println!(
                            "  Promoted: {}  Rejected: {}  Bytecode calls: {}",
                            tier.promoted.to_string().bright_white(),
                            tier.rejected.to_string().bright_white(),
                            tier.bytecode_calls.to_string().bright_white()
                        );
                    }
                    None => println!(
                        "  Status: {} (restart without --no-ovm to enable)",
                        "disabled".bright_yellow()
                    ),
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
                                // `:type` evaluates its argument to learn the
                                // runtime type — refuse inputs that would
                                // mutate the session (a `let` or an
                                // assignment), so a type query never changes
                                // what it inspects.
                                let mutates = match stmt.unwrapped() {
                                    crate::ast::Statement::LetDecl(_) => true,
                                    crate::ast::Statement::Expression(e) => {
                                        matches!(e, crate::ast::Expr::Assignment { .. })
                                    }
                                    _ => false,
                                };
                                if mutates {
                                    println!(
                                        "`:type` evaluates its argument and won't run a binding or assignment.\nQuery the value instead: `:type {}`",
                                        expr.split('=').next_back().unwrap_or(&expr).trim()
                                    );
                                } else {
                                    match self.eval_line(&expr) {
                                        Ok(value) => {
                                            println!("{} : {}", expr, Self::deep_type_of(&value));
                                        }
                                        Err(e) => {
                                            println!("Type check failed: {}", e);
                                        }
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
                                repl_print(&value);
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
                println!("\n{}", "=== Memory ===".bright_cyan().bold());
                println!("  Values are reference-counted and reclaimed deterministically.");
                let user_vars = self.interpreter.get_user_variables().len();
                println!("  User bindings: {}", user_vars.to_string().bright_white());
            }
            ":stats" => {
                println!("\n{}", "=== Execution Statistics ===".bright_cyan().bold());
                match self.interpreter.bytecode_tier_stats() {
                    Some(tier) => {
                        println!("  Bytecode tier: {}", "enabled".bright_green());
                        println!(
                            "  Functions promoted: {}",
                            tier.promoted.to_string().bright_white()
                        );
                        println!(
                            "  Functions rejected:  {}",
                            tier.rejected.to_string().bright_white()
                        );
                        println!(
                            "  Bytecode calls:      {}",
                            tier.bytecode_calls.to_string().bright_white()
                        );
                    }
                    None => println!("  Bytecode tier: {}", "disabled".bright_yellow()),
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
                            // Set the file context for proper module resolution
                            let file_path = std::path::Path::new(filename);
                            let absolute_path = if file_path.is_absolute() {
                                file_path.to_path_buf()
                            } else {
                                std::env::current_dir().unwrap_or_default().join(file_path)
                            };
                            self.interpreter.set_current_file(&absolute_path);
                            // The file's own argv, exactly as `olang run`
                            // would install it: argv[0] is the script path
                            // (run_all.ol resolves its whole target set
                            // from it). Restored below so prompt-level
                            // os.args() keeps its previous meaning.
                            let prior_args = crate::stdlib::os::set_script_args(vec![
                                absolute_path.to_string_lossy().into_owned(),
                            ]);

                            match self.eval_line(&content) {
                                Ok(value) => {
                                    if value != Value::Unit {
                                        repl_print(&value);
                                    }
                                    println!("File '{}' executed successfully", filename);
                                }
                                Err(e) => {
                                    eprintln!("Error executing '{}': {}", filename, e);
                                }
                            }

                            // Clear file context after execution
                            self.interpreter.clear_current_file();
                            crate::stdlib::os::restore_script_args(prior_args);

                            // Aggressive cleanup after script execution
                            // 1. Clear user environment to free variables
                            self.interpreter.clear_user_environment();

                            // 2. Values are reference-counted; clearing the
                            // environment above already released them
                            {}
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
                        let user_vars = self.interpreter.get_user_variables();
                        if let Some(value) = user_vars.get(&var_name) {
                            self.debugger
                                .update_variable_history(var_name, (*value).clone());
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
                    println!(
                        "Usage: :profile <code> — runs it under the sampling profiler
                         (time by tier, per-function self/total, hottest call paths).
                         Example: :profile fold(range(0, 5000000), 0, (a, x) => a + x)"
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
                    // Parse advanced search options token-wise: pull out
                    // `category:X`/`returns:X`/`limit:N` filters, keep the
                    // rest as the query. Token-wise extraction (rather than
                    // string replacement against the original query) means
                    // combined filters compose, `limit:05` parses, and a bad
                    // limit is reported instead of silently polluting the
                    // query text.
                    let mut filters = SearchFilters::default();
                    let mut query_words: Vec<&str> = Vec::new();
                    for token in &parts[1..] {
                        if let Some(category) = token.strip_prefix("category:") {
                            filters.category = Some(category.to_string());
                        } else if let Some(return_type) = token.strip_prefix("returns:") {
                            filters.return_type = Some(return_type.to_string());
                        } else if let Some(limit) = token.strip_prefix("limit:") {
                            match limit.parse::<usize>() {
                                Ok(n) => filters.max_results = n,
                                Err(_) => {
                                    println!(
                                        "Invalid limit '{}' — expected a number, e.g. limit:5",
                                        limit
                                    );
                                    return Ok(());
                                }
                            }
                        } else {
                            query_words.push(token);
                        }
                    }
                    // Quotes carry no meaning to the matcher; the usage
                    // examples show quoted phrases, so strip them.
                    let search_query = query_words.join(" ").replace('"', "");

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
                    println!(
                        "\n{}Advanced Search Options:{}",
                        Colors::CYAN,
                        Colors::RESET
                    );
                    println!(
                        "  {}category:<name>{} - Filter by category",
                        Colors::BLUE,
                        Colors::RESET
                    );
                    println!(
                        "  {}returns:<type>{} - Filter by return type",
                        Colors::BLUE,
                        Colors::RESET
                    );
                    println!(
                        "  {}limit:<num>{} - Limit number of results",
                        Colors::BLUE,
                        Colors::RESET
                    );
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
                        println!(
                            "  Use ':tutorial_run {}' to start interactive mode",
                            tutorial_name
                        );
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
                println!(
                    "\n{}=== Advanced Help Features ==={}",
                    Colors::BOLD,
                    Colors::RESET
                );
                println!("\n{}Advanced Search:{}", Colors::CYAN, Colors::RESET);
                println!(
                    "  {}:search <query>{} - Fuzzy search with relevance scoring",
                    Colors::BLUE,
                    Colors::RESET
                );
                println!(
                    "  {}help search <query>{} - Same as :search",
                    Colors::BLUE,
                    Colors::RESET
                );
                println!(
                    "  {}category:<name>{} - Filter by category",
                    Colors::GREEN,
                    Colors::RESET
                );
                println!(
                    "  {}returns:<type>{} - Filter by return type",
                    Colors::GREEN,
                    Colors::RESET
                );
                println!(
                    "  {}limit:<num>{} - Limit results",
                    Colors::GREEN,
                    Colors::RESET
                );

                println!("\n{}Interactive Tutorials:{}", Colors::CYAN, Colors::RESET);
                println!(
                    "  {}help tutorials{} - List all available tutorials",
                    Colors::BLUE,
                    Colors::RESET
                );
                println!(
                    "  {}help tutorial <name>{} - View specific tutorial",
                    Colors::BLUE,
                    Colors::RESET
                );
                println!(
                    "  {}:tutorial <name>{} - View tutorial with interactive options",
                    Colors::BLUE,
                    Colors::RESET
                );
                println!(
                    "  {}:tutorial_run <name>{} - Start interactive tutorial mode",
                    Colors::BLUE,
                    Colors::RESET
                );

                println!("\n{}Context-Sensitive Help:{}", Colors::CYAN, Colors::RESET);
                println!(
                    "  {}:contextual_help{} - Get suggestions based on recent activity",
                    Colors::BLUE,
                    Colors::RESET
                );
                println!(
                    "  {}help contextual{} - Same as :contextual_help",
                    Colors::BLUE,
                    Colors::RESET
                );
                println!("  Automatic suggestions based on:");
                println!("    • Recent commands and patterns");
                println!("    • Last error messages");
                println!("    • Current variables and types");
                println!("    • Working category context");

                println!("\n{}Enhanced Examples:{}", Colors::CYAN, Colors::RESET);
                println!(
                    "  {}:examples <function>{} - Interactive examples with execution",
                    Colors::BLUE,
                    Colors::RESET
                );
                println!(
                    "  {}help examples{} - Comprehensive example gallery",
                    Colors::BLUE,
                    Colors::RESET
                );
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
                                    repl_print(&value);
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
            // A colon in front of a language statement is a natural
            // reflex (`:use collections`); run the statement rather than
            // rejecting the reflex.
            ":use" | ":let" | ":fn" => {
                let stripped = command.trim_start_matches(':');
                match self.eval_line(stripped) {
                    Ok(value) => {
                        if value != Value::Unit {
                            repl_print(&value);
                        }
                    }
                    Err(e) => self.show_enhanced_error(&e),
                }
            }
            _ => {
                // Say it plainly, with the nearest real command — not
                // through the error chain's stacked prefixes.
                let max_distance = (command_name.chars().count() / 3).clamp(1, 3);
                let nearest = REPL_COMMANDS
                    .iter()
                    .map(|c| (olang_interpreter_levenshtein(command_name, c), *c))
                    .filter(|(d, _)| *d <= max_distance)
                    .min();
                match nearest {
                    Some((_, c)) => println!(
                        "unknown command {} — did you mean {}?",
                        command_name.red(),
                        c.cyan()
                    ),
                    None => println!(
                        "unknown command {} — {} lists them. Statements need no colon: `use collections`, `let x = 1`",
                        command_name.red(),
                        ":help".cyan()
                    ),
                }
            }
        }
        Ok(())
    }

    fn eval_line(&mut self, line: &str) -> Result<Value, ReplError> {
        // Check for watched variables before execution
        let watching_vars = !self.debugger.watched_variables.is_empty();
        if watching_vars {
            let vars = self.interpreter.get_user_variables();
            for (name, value) in &vars {
                if self.debugger.is_watching(name) {
                    self.debugger
                        .update_variable_history(name.clone(), (*value).clone());
                }
            }
        }

        // Macro support in the session: meta fns are stripped from every
        // expanded program, so remember each one's source and prepend the
        // collection whenever a later input needs expansion. The prelude
        // contributes nothing at runtime — stripping blanks it — so
        // non-macro inputs are untouched.
        let uses_macros = line.contains('@') || crate::expand::has_meta_fn_token(line);
        if uses_macros && let Ok(raw) = self.parser.parse_raw(line) {
            for st in &raw.statements {
                if let crate::ast::Statement::MetaFnDecl { span, .. } = st.unwrapped() {
                    self.meta_prelude.push(line[span.0..span.1].to_string());
                }
            }
        }
        let program = if uses_macros && !self.meta_prelude.is_empty() {
            let with_prelude = format!(
                "{}
{}",
                self.meta_prelude.join(
                    "
"
                ),
                line
            );
            self.parser.parse(&with_prelude)?
        } else {
            self.parser.parse(line)?
        };

        if program.statements.is_empty() {
            return Ok(Value::Unit);
        }

        let result = self.interpreter.eval_program(program)?;

        // Check for watched variable changes after execution
        if watching_vars {
            let user_vars_after = self.interpreter.get_user_variables();

            // Convert HashMap<String, &Value> to HashMap<String, Value> for compatibility
            let user_vars_owned: HashMap<String, Value> = user_vars_after
                .iter()
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
                    self.debugger
                        .update_variable_history(name.clone(), (*value).clone());
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
            let classic_interpreter = &mut self.interpreter;
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
            let user_vars = self.interpreter.get_user_variables();
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

    /// A deep, value-derived type description for `:type`: element types
    /// where the elements agree (`List<Int>`), honest fallbacks where
    /// they don't (`List`), Result sides, tuple shapes, and function
    /// signatures from their annotations.
    fn deep_type_of(v: &Value) -> String {
        fn unify<I: Iterator<Item = String>>(mut it: I) -> Option<String> {
            let first = it.next()?;
            for t in it {
                if t != first {
                    return None;
                }
            }
            Some(first)
        }
        match v {
            Value::List(items) => match unify(items.iter().map(Self::deep_type_of)) {
                Some(e) => format!("List<{}>", e),
                None => "List".to_string(),
            },
            Value::Map(m) => match unify(m.values().map(Self::deep_type_of)) {
                Some(val) => format!("Map<String, {}>", val),
                None => "Map".to_string(),
            },
            Value::Tuple(items) => format!(
                "({})",
                items
                    .iter()
                    .map(Self::deep_type_of)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Value::Ok(p) => format!("Result<{}, _>", Self::deep_type_of(p)),
            Value::Err(p) => format!("Result<_, {}>", Self::deep_type_of(p)),
            Value::Function(f) => format!(
                "({}) -> ?",
                f.parameters
                    .iter()
                    .map(|p| p
                        .type_annotation
                        .as_ref()
                        .map(|a| a.display_source())
                        .unwrap_or_else(|| "?".to_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            other => other.type_name(),
        }
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
            Value::EnumConstructor { .. } => "enum_constructor",
            Value::Enum { .. } => {
                // Use a static string for REPL display
                "enum"
            }
            Value::Map(_) => "map",
            Value::TypeInfo { .. } => "type",
            Value::Native(handle) => handle.0.type_name(),
        }
    }

    fn format_value_preview(value: &Value) -> String {
        match value {
            Value::String(s) => {
                if s.chars().count() > 50 {
                    let preview: String = s.chars().take(47).collect();
                    format!("\"{}...\"", preview)
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
        let mut prev_slash = false;
        let mut in_comment = false;

        for ch in line.chars() {
            if in_comment {
                if ch == '\n' {
                    in_comment = false;
                }
                continue;
            }
            if escape_next {
                escape_next = false;
                prev_slash = false;
                continue;
            }

            // A `//` outside a string starts a comment: nothing up to the end
            // of the line can open or close a bracket.
            if ch == '/' && !in_string {
                if prev_slash {
                    in_comment = true;
                }
                prev_slash = !prev_slash;
                continue;
            }
            prev_slash = false;

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
        println!(
            "\n{}",
            "=== Interactive Debugging Commands ==="
                .bright_cyan()
                .bold()
        );
        println!(
            "  {}  - Step through expression evaluation",
            ":debug <expression>".bright_blue()
        );
        println!(
            "  {}     - Toggle variable watching",
            ":watch <variable>".bright_blue()
        );
        println!(
            "  {}   - Detailed variable inspection",
            ":inspect <variable>".bright_blue()
        );
        println!(
            "  {}     - Trace function calls",
            ":trace <function>".bright_blue()
        );
        println!(
            "  {}      - Modify variable values",
            ":set <var> <value>".bright_blue()
        );
        println!("  {}           - Show call stack", ":stack".bright_blue());
        println!(
            "  {}    - Profile expression performance",
            ":profile <expression>".bright_blue()
        );
        println!(
            "  {}       - Enable/disable debug mode",
            ":debug [on|off]".bright_blue()
        );
        println!();
    }

    fn debug_evaluate_expression(&mut self, expr: &str) -> Result<(), ReplError> {
        println!("{}", format!("Debugging: {}", expr).bright_cyan().bold());

        // Check for watched variables before execution
        let user_vars_before = self.interpreter.get_user_variables();
        for (name, value) in &user_vars_before {
            if self.debugger.is_watching(name) {
                self.debugger
                    .update_variable_history(name.clone(), (*value).clone());
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
                        self.debugger
                            .record_profiling_data(expr.to_string(), time_ms);

                        // Check for watched variable changes
                        let user_vars_after = self.interpreter.get_user_variables();

                        // Convert HashMap<String, &Value> to HashMap<String, Value> for compatibility
                        let user_vars_owned: HashMap<String, Value> = user_vars_after
                            .iter()
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
                                self.debugger
                                    .update_variable_history(name.clone(), (*value).clone());
                            }
                        }
                    }
                    Err(e) => {
                        let duration = start.elapsed();
                        println!("  {}: {}", "Error".bright_red(), e);
                        println!(
                            "  {}: {:.2}ms",
                            "Time to error".bright_blue(),
                            duration.as_secs_f64() * 1000.0
                        );

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
        let user_vars = self.interpreter.get_user_variables();

        if let Some(value) = user_vars.get(var_name) {
            println!(
                "\n{}",
                format!("=== Variable Inspection: {} ===", var_name)
                    .bright_cyan()
                    .bold()
            );
            println!(
                "  {}: {}",
                "Type".bright_yellow(),
                Self::get_type_name(value)
            );
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
                        let all_same_type = items
                            .iter()
                            .all(|item| Self::get_type_name(item) == first_type);
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
                    for (field_name, field_value) in fields.iter() {
                        println!(
                            "    {}: {} = {}",
                            field_name.bright_magenta(),
                            Self::get_type_name(field_value),
                            Self::format_value_preview(field_value)
                        );
                    }
                }
                Value::Function(func) => {
                    println!(
                        "  {}: {} parameters",
                        "Arity".bright_blue(),
                        func.parameters.len()
                    );
                    if !func.parameters.is_empty() {
                        println!(
                            "  {}: {}",
                            "Parameters".bright_blue(),
                            func.parameters
                                .iter()
                                .map(|p| p.name.clone())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                }
                _ => {}
            }

            // Show if variable is being watched
            if self.debugger.is_watching(var_name) {
                println!(
                    "  {}: {}",
                    "Status".bright_green(),
                    "Being watched".bright_green()
                );
            }
        } else {
            // Check if it's a builtin function
            let builtins = self.interpreter.get_builtin_functions();
            if let Some(_builtin) = builtins.get(var_name) {
                println!(
                    "\n{}",
                    format!("=== Builtin Function: {} ===", var_name)
                        .bright_cyan()
                        .bold()
                );
                println!("  {}: builtin function", "Type".bright_yellow());

                // Show help if available
                if self.help_system.has_function(var_name) {
                    println!("\n{}", self.help_system.show_function_help(var_name));
                } else {
                    println!("  {}: No documentation available", "Help".bright_blue());
                }
            } else {
                return Err(ReplError::Debug(format!(
                    "Variable '{}' not found",
                    var_name
                )));
            }
        }

        Ok(())
    }

    fn set_variable(&mut self, var_name: String, value_expr: String) -> Result<(), ReplError> {
        // Parse and evaluate the value expression
        match self.eval_line(&value_expr) {
            Ok(value) => {
                // Check if variable exists
                let user_vars = self.interpreter.get_user_variables();
                let existed = user_vars.contains_key(&var_name);

                // Set the variable
                self.interpreter
                    .define_variable(var_name.clone(), value.clone());

                if existed {
                    println!(
                        "Variable '{}' updated to: {}",
                        var_name.bright_yellow(),
                        value
                    );
                } else {
                    println!(
                        "Variable '{}' created with value: {}",
                        var_name.bright_yellow(),
                        value
                    );
                }

                // Update variable history if being watched
                if self.debugger.is_watching(&var_name) {
                    self.debugger.update_variable_history(var_name, value);
                }
            }
            Err(e) => {
                return Err(ReplError::Debug(format!(
                    "Failed to evaluate value expression '{}': {}",
                    value_expr, e
                )));
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
                println!(
                    "  #{}: {}",
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
                        println!(
                            "        {} = {}",
                            name.bright_yellow(),
                            Self::format_value_preview(value)
                        );
                    }
                }
            }
        }

        println!();
    }

    /// `:profile <code>` — run the code once under the same sampling
    /// profiler as `olang profile`, then print the value and the full
    /// report: time by tier, per-function self/total, hottest call
    /// paths, and the findings notes. A short snippet may land few
    /// samples; the report says so rather than inventing precision.
    fn profile_expression(&mut self, expr: &str) -> Result<(), ReplError> {
        // A finer interval than the CLI default: REPL snippets are
        // usually shorter than whole programs.
        let session = crate::profile::start(250);
        let started = Instant::now();
        let outcome = self.eval_line(expr);
        let elapsed = started.elapsed();
        let label: String = expr.chars().take(48).collect();
        let report = session.finish(elapsed, 20, &label);
        match outcome {
            Ok(value) => {
                self.after_eval(expr, &value);
                if value != Value::Unit {
                    repl_print(&value);
                }
            }
            Err(e) => self.show_enhanced_error(&e),
        }
        println!("{}", report);
        Ok(())
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
            ParseError::InvalidSyntaxWithPosition {
                message,
                line,
                column,
                snippet,
            } => {
                println!(
                    "  {}: {}",
                    "Parse Error".bright_red().bold(),
                    message.bright_white()
                );
                println!("\n  {}", "Location:".bright_yellow().bold());
                println!(
                    "    Line {}, Column {}",
                    line.to_string().bright_cyan(),
                    column.to_string().bright_cyan()
                );

                // Show the formatted code snippet with highlighting
                if !snippet.trim().is_empty() {
                    println!("\n  {}", "Code Context:".bright_blue().bold());
                    self.show_highlighted_snippet(snippet);
                }
            }
            ParseError::UnexpectedTokenWithPosition {
                token,
                line,
                column,
                snippet,
            } => {
                println!(
                    "  {}: Unexpected token '{}'",
                    "Parse Error".bright_red().bold(),
                    token.bright_yellow().bold()
                );

                println!(
                    "    Line {}, Column {}",
                    line.to_string().bright_cyan(),
                    column.to_string().bright_cyan()
                );

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
                    println!(
                        "    {}{}│ {}",
                        line_num.bright_black(),
                        " ".repeat(4 - line_num.len().min(4)),
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

                    println!(
                        "    {}{}│ {}",
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
                    if let Some(&next_ch) = chars.peek()
                        && ((next_ch == '=' && matches!(ch, '=' | '!' | '<' | '>'))
                            || (ch == '&' && next_ch == '&')
                            || (ch == '|' && next_ch == '|'))
                    {
                        op.push(chars.next().unwrap());
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
            "fn" | "let" | "if" | "else" | "match" | "for" | "while" | "loop" | "break"
            | "continue" | "true" | "false" | "async" | "await" | "try" | "catch" | "import"
            | "export" | "type" | "trait" | "impl" | "struct" | "enum" => {
                format!("{}", word.bright_blue().bold())
            }
            // Types
            "Int" | "Float" | "String" | "Bool" | "List" | "Map" | "Unit" | "Result" | "Ok"
            | "Err" => {
                format!("{}", word.bright_magenta())
            }
            // Built-in functions (common ones)
            "println" | "print" | "map" | "filter" | "reduce" | "range" | "len" | "head"
            | "tail" => {
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
        if error_msg.contains("bracket")
            || error_msg.contains("parenthesis")
            || error_msg.contains("brace")
        {
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
                    println!(
                        "    • Type {} for help on {}",
                        format!("help {}", topic).bright_cyan(),
                        topic.replace("error.", "").replace("_", " ").bright_white()
                    );
                }
            }

            // Always suggest the general error help
            if has_error_topics {
                println!(
                    "    • Type {} for comprehensive error guidance",
                    "help error.fixes".bright_cyan()
                );
            }
        }
    }

    fn show_interpreter_error(&mut self, interpreter_error: &InterpreterError) {
        match interpreter_error {
            InterpreterError::UndefinedVariable { name } => {
                println!(
                    "  {}: Variable '{}' is not defined",
                    "Undefined Variable".bright_red().bold(),
                    name.bright_yellow()
                );

                // Suggest similar variables
                let user_vars = self.interpreter.get_user_variables();
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
                    println!(
                        "\n  {}: Use {} to see available variables",
                        "Hint".bright_blue().bold(),
                        ":env".bright_cyan()
                    );
                }
            }
            InterpreterError::TypeError { message } => {
                println!("  {}: {}", "Type Error".bright_red().bold(), message);

                // Type-specific suggestions
                if message.contains("division by zero") {
                    println!(
                        "\n  {}: Check that denominators are not zero before division",
                        "Hint".bright_blue().bold()
                    );
                } else if message.contains("cannot convert") {
                    println!(
                        "\n  {}: Use type conversion functions like {} or {}",
                        "Hint".bright_blue().bold(),
                        "to_int()".bright_cyan(),
                        "to_float()".bright_cyan()
                    );
                } else if message.contains("invalid binary operation") {
                    println!(
                        "\n  {}: Check that operands are compatible types",
                        "Hint".bright_blue().bold()
                    );
                    println!(
                        "    • Numbers: {} with {}",
                        "42".bright_green(),
                        "3.14".bright_green()
                    );
                    println!(
                        "    • Strings: {} with {}",
                        "\"hello\"".bright_green(),
                        "\"world\"".bright_green()
                    );
                    println!(
                        "    • Booleans: {} with {}",
                        "true".bright_green(),
                        "false".bright_green()
                    );
                }
            }
            InterpreterError::RuntimeError { message } => {
                println!("  {}: {}", "Runtime Error".bright_red().bold(), message);

                // Runtime-specific suggestions
                if message.contains("break") {
                    println!(
                        "\n  {}: {} can only be used inside loops",
                        "Hint".bright_blue().bold(),
                        "break".bright_cyan()
                    );
                } else if message.contains("continue") {
                    println!(
                        "\n  {}: {} can only be used inside loops",
                        "Hint".bright_blue().bold(),
                        "continue".bright_cyan()
                    );
                }
            }
            InterpreterError::ArityMismatch { expected, got } => {
                println!(
                    "  {}: Expected {} arguments, got {}",
                    "Arity Mismatch".bright_red().bold(),
                    expected.to_string().bright_cyan(),
                    got.to_string().bright_yellow()
                );

                println!(
                    "\n  {}: Check the function signature and provide the correct number of arguments",
                    "Hint".bright_blue().bold()
                );
            }
            InterpreterError::PatternMatchFailed => {
                println!(
                    "  {}: Pattern matching failed",
                    "Pattern Match Error".bright_red().bold()
                );

                println!(
                    "\n  {}: Ensure the pattern matches the structure of the value",
                    "Hint".bright_blue().bold()
                );
                println!("    • Use {} to match any value", "_".bright_cyan());
                println!(
                    "    • Use {} or {} for Result types",
                    "Ok(value)".bright_cyan(),
                    "Err(error)".bright_cyan()
                );
            }
            _ => {
                // Handle all other error types (lazy evaluation errors, etc.)
                println!("  {}: {}", "Error".bright_red().bold(), interpreter_error);

                println!(
                    "\n  {}: This appears to be a system-level error",
                    "Hint".bright_blue().bold()
                );
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

        // Compare 2-char prefixes/suffixes by chars — byte slicing panics on
        // multibyte identifiers (e.g. `xé`)
        let target_chars: Vec<char> = target_lower.chars().collect();
        let candidate_chars: Vec<char> = candidate_lower.chars().collect();
        if target_chars.len() >= 2 && candidate_chars.len() >= 2 {
            // Check for similar starts
            if target_chars[..2] == candidate_chars[..2] {
                return true;
            }

            // Check for similar endings
            if target_chars[target_chars.len() - 2..]
                == candidate_chars[candidate_chars.len() - 2..]
            {
                return true;
            }
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

        println!(
            "    {} {}",
            severity_icon,
            suggestion.message.color(severity_color).bold()
        );

        if let Some(fix) = &suggestion.fix {
            println!("      {}: {}", "Fix".bright_green().bold(), fix);
        }

        if let Some(help) = &suggestion.help {
            println!("      {}: {}", "Help".bright_blue().bold(), help);
        }
    }

    /// Build help context from current REPL state
    fn build_help_context(&mut self) -> HelpContext {
        let recent_commands = self
            .command_history
            .iter()
            .rev()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();

        let current_variables = self
            .interpreter
            .get_user_variables()
            .keys()
            .cloned()
            .collect();

        // The REPL does not retain past errors; context-sensitive help
        // works from the command history alone.
        let last_error = None;

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
    fn run_interactive_tutorial(
        &mut self,
        tutorial: &crate::help::Tutorial,
    ) -> Result<(), ReplError> {
        println!(
            "\n{}Starting Interactive Tutorial: {}{}",
            Colors::BOLD,
            tutorial.name,
            Colors::RESET
        );
        println!("{}", tutorial.description);
        println!(
            "{}Difficulty: {} | Estimated Time: {}{}",
            Colors::DIM,
            tutorial.difficulty,
            tutorial.estimated_time,
            Colors::RESET
        );
        println!();

        for (i, step) in tutorial.steps.iter().enumerate() {
            println!(
                "{}=== Step {}/{}: {} ==={}",
                Colors::CYAN,
                i + 1,
                tutorial.steps.len(),
                step.title,
                Colors::RESET
            );
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
            println!(
                "  {}r{} - Run the code automatically",
                Colors::BLUE,
                Colors::RESET
            );
            println!(
                "  {}t{} - Try it yourself (enter your own code)",
                Colors::BLUE,
                Colors::RESET
            );
            println!("  {}e{} - Show explanation", Colors::BLUE, Colors::RESET);
            println!("  {}h{} - Show hints", Colors::BLUE, Colors::RESET);
            println!("  {}n{} - Next step", Colors::BLUE, Colors::RESET);
            println!("  {}q{} - Quit tutorial", Colors::BLUE, Colors::RESET);

            loop {
                print!("{}Tutorial> {}", Colors::CYAN, Colors::RESET);
                std::io::stdout().flush().unwrap();

                let mut input = String::new();
                // EOF (Ok(0)) or a read error must end the tutorial: with a
                // closed/piped stdin, re-prompting would spin forever.
                match std::io::stdin().read_line(&mut input) {
                    Ok(0) | Err(_) => {
                        println!(
                            "{}Input closed — exiting tutorial.{}",
                            Colors::YELLOW,
                            Colors::RESET
                        );
                        return Ok(());
                    }
                    Ok(_) => {}
                }
                let input = input.trim();

                match input {
                    "r" => {
                        // Run the tutorial code
                        println!(
                            "{}Running tutorial code...{}",
                            Colors::YELLOW,
                            Colors::RESET
                        );
                        match self.eval_line(&step.code) {
                            Ok(value) => {
                                if value != Value::Unit {
                                    repl_print(&value);
                                }
                                println!(
                                    "{}Code executed successfully!{}",
                                    Colors::GREEN,
                                    Colors::RESET
                                );
                            }
                            Err(e) => {
                                println!("{}ERROR executing code:{}", Colors::RED, Colors::RESET);
                                println!("{}", e);
                            }
                        }
                        break;
                    }
                    "t" => {
                        println!(
                            "{}Enter your code (press Enter twice to execute):{}",
                            Colors::YELLOW,
                            Colors::RESET
                        );
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
                                        repl_print(&value);
                                    }
                                    println!("{}Great job!{}", Colors::GREEN, Colors::RESET);
                                }
                                Err(e) => {
                                    println!("{}Error:{}", Colors::RED, Colors::RESET);
                                    println!("{}", e);
                                    println!(
                                        "{} Try again or use 'r' to run the tutorial code{}",
                                        Colors::YELLOW,
                                        Colors::RESET
                                    );
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
                            println!(
                                "{}No hints available for this step.{}",
                                Colors::DIM,
                                Colors::RESET
                            );
                        }
                        println!();
                    }
                    "n" => {
                        println!("{}Moving to next step...{}", Colors::CYAN, Colors::RESET);
                        break;
                    }
                    "q" | "quit" | "exit" => {
                        println!(
                            "{}Exiting tutorial. Progress saved.{}",
                            Colors::YELLOW,
                            Colors::RESET
                        );
                        return Ok(());
                    }
                    _ => {
                        println!(
                            "{}Invalid option. Use r/t/e/h/n/q{}",
                            Colors::RED,
                            Colors::RESET
                        );
                    }
                }
            }

            println!();
        }

        // Tutorial completion
        println!(
            "{}Congratulations! You've completed the '{}' tutorial!{}",
            Colors::GREEN,
            tutorial.name,
            Colors::RESET
        );
        println!("{}You've learned:{}", Colors::YELLOW, Colors::RESET);
        for step in &tutorial.steps {
            println!("  {}", step.title);
        }

        // Suggest next steps
        if !tutorial.prerequisites.is_empty() {
            println!(
                "\n{}Consider these related tutorials:{}",
                Colors::CYAN,
                Colors::RESET
            );
            let all_tutorials = self.help_system.get_tutorials();
            for other_tutorial in all_tutorials {
                if other_tutorial.prerequisites.contains(&tutorial.name) {
                    println!(
                        "  • {} ({})",
                        other_tutorial.name, other_tutorial.difficulty
                    );
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
        self.interpreter.define_variable(name.to_string(), value);
    }

    fn get_variable(&self, _name: &str) -> Option<Value> {
        // Note: This would need &mut self to work properly, but keeping for compatibility
        None // Simplified for now
    }
}

// Pretty-print helpers for REPL output to avoid quoted strings
fn repl_format(value: &Value) -> String {
    match value {
        Value::String(s) => s.as_str().to_string(),
        _ => format!("{}", value),
    }
}

fn repl_print(value: &Value) {
    if colored::control::SHOULD_COLORIZE.should_colorize() {
        // A top-level string prints bare (repl_format's contract), so it
        // colors as content, not as a quoted literal.
        if let Value::String(s) = value {
            println!("{}", s.as_str().green());
        } else {
            println!("{}", color_value(value));
        }
    } else {
        println!("{}", repl_format(value));
    }
}

/// The words the grammar reserves, colored as keywords by the input
/// highlighter. Kept in sync with grammar.pest by eye — a missed keyword
/// colors as plain text, nothing worse.
const KEYWORDS: &[&str] = &[
    "break", "continue", "else", "enum", "error", "fn", "for", "if", "impl", "in", "let", "match",
    "meta", "mut", "return", "share", "spawn", "struct", "test", "trait", "type", "use", "while",
];

/// Syntax-color one input line: comments dim, strings and templates
/// green, numbers yellow, keywords magenta, `true`/`false` yellow,
/// `:commands` cyan, and the arrows (`|>`, `=>`) blue. Character-level
/// and single-pass — this runs on every keystroke.
fn highlight_source(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();

    // A leading `:name` is a REPL command; color the command word and
    // leave its arguments to the normal rules.
    let mut i = 0;
    if line.trim_start().starts_with(':') {
        let indent = n - line.trim_start().chars().count();
        let mut end = indent + 1;
        while end < n && (chars[end].is_alphanumeric() || chars[end] == '_' || chars[end] == '!') {
            end += 1;
        }
        let word: String = chars[..end].iter().collect();
        out.push_str(&word.cyan().to_string());
        i = end;
    }

    while i < n {
        let c = chars[i];
        // Comment: the rest of the line, dimmed.
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            let rest: String = chars[i..].iter().collect();
            out.push_str(&rest.dimmed().to_string());
            break;
        }
        // Strings and templates, escapes honored; an unterminated
        // literal colors to end of line, which reads as intended while
        // still typing it.
        if c == '"' || c == '`' {
            let quote = c;
            let mut end = i + 1;
            while end < n {
                if chars[end] == '\\' {
                    end += 2;
                    continue;
                }
                if chars[end] == quote {
                    end += 1;
                    break;
                }
                end += 1;
            }
            let end = end.min(n);
            let lit: String = chars[i..end].iter().collect();
            out.push_str(&lit.green().to_string());
            i = end;
            continue;
        }
        // Numbers: digit-led runs, dots and underscores included.
        if c.is_ascii_digit() {
            let mut end = i;
            while end < n
                && (chars[end].is_ascii_digit()
                    || chars[end] == '_'
                    || (chars[end] == '.' && end + 1 < n && chars[end + 1].is_ascii_digit()))
            {
                end += 1;
            }
            let num: String = chars[i..end].iter().collect();
            out.push_str(&num.yellow().to_string());
            i = end;
            continue;
        }
        // Identifiers and keywords.
        if c.is_alphabetic() || c == '_' {
            let mut end = i;
            while end < n && (chars[end].is_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            let word: String = chars[i..end].iter().collect();
            if word == "true" || word == "false" {
                out.push_str(&word.yellow().to_string());
            } else if KEYWORDS.contains(&word.as_str()) {
                out.push_str(&word.magenta().to_string());
            } else if end < n && chars[end] == '(' {
                out.push_str(&word.cyan().to_string());
            } else {
                out.push_str(&word);
            }
            i = end;
            continue;
        }
        // The two arrows that give olang its shape.
        if (c == '|' || c == '=') && i + 1 < n && chars[i + 1] == '>' {
            out.push_str(&format!("{}>", c).blue().to_string());
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// A value, colored the way the input highlighter colors source:
/// numbers yellow, strings green, booleans yellow, keys and callables
/// cyan, type names blue, `Ok` green / `Err` red, structure dim. The
/// text is exactly `Display`'s — only the colors are added.
fn color_value(value: &Value) -> String {
    match value {
        Value::Integer(n) => n.to_string().yellow().to_string(),
        Value::Float(x) => crate::ast::format_float(*x).yellow().to_string(),
        Value::String(s) => format!("\"{}\"", s).green().to_string(),
        Value::Boolean(b) => b.to_string().yellow().to_string(),
        Value::Unit => "()".dimmed().to_string(),
        Value::List(items) => {
            let inner: Vec<String> = items.iter().map(color_value).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Tuple(items) => {
            let inner: Vec<String> = items.iter().map(color_value).collect();
            format!("({})", inner.join(", "))
        }
        Value::Map(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", format!("\"{}\"", k).cyan(), color_value(v)))
                .collect();
            format!("#{{{}}}", inner.join(", "))
        }
        Value::Struct { type_name, fields } => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k.cyan(), color_value(v)))
                .collect();
            format!(
                "{}{{{}}}",
                format!("<struct: {}>", type_name).blue(),
                inner.join(", ")
            )
        }
        Value::Ok(inner) => format!("{}({})", "Ok".green().bold(), color_value(inner)),
        Value::Err(inner) => format!("{}({})", "Err".red().bold(), color_value(inner)),
        Value::Range {
            start,
            end,
            inclusive,
        } => format!(
            "{}{}{}",
            start.to_string().yellow(),
            if *inclusive { "..=" } else { ".." },
            end.to_string().yellow()
        ),
        Value::Function(f) => match &f.name {
            Some(name) => format!("<function: {}>", name).cyan().to_string(),
            None => "<function>".cyan().to_string(),
        },
        Value::Builtin(b) => format!("<builtin: {}>", b.name).cyan().to_string(),
        Value::Enum {
            type_name,
            variant_name,
            variant_data,
        } => {
            let head = format!("{}.{}", type_name.blue(), variant_name.cyan());
            match variant_data {
                crate::ast::EnumVariantData::Unit => head,
                crate::ast::EnumVariantData::Tuple(values) => {
                    let inner: Vec<String> = values.iter().map(color_value).collect();
                    format!("{}({})", head, inner.join(", "))
                }
                crate::ast::EnumVariantData::Struct(fields) => {
                    let inner: Vec<String> = fields
                        .iter()
                        .map(|(k, v)| format!("{}: {}", k.cyan(), color_value(v)))
                        .collect();
                    format!("{} {{ {} }}", head, inner.join(", "))
                }
            }
        }
        Value::EnumConstructor {
            type_name,
            variant_name,
            ..
        } => format!("{}.{}", type_name.blue(), variant_name.cyan()),
        Value::TypeInfo { name, .. } => format!("<type: {}>", name).blue().to_string(),
        // Native handles format themselves (ods tables and friends);
        // recoloring their internals isn't this function's business.
        Value::Native(_) => format!("{}", value).cyan().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyline::history::DefaultHistory;

    fn helper() -> ReplHelper {
        ReplHelper::new(vec![
            "println".to_string(),
            "print".to_string(),
            "map".to_string(),
        ])
    }

    fn complete(h: &ReplHelper, line: &str, pos: usize) -> (usize, Vec<String>) {
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        let (start, pairs) = h.complete(line, pos, &ctx).unwrap();
        (start, pairs.into_iter().map(|p| p.replacement).collect())
    }

    #[test]
    fn completes_repl_command_names() {
        let h = helper();
        let (start, names) = complete(&h, ":p", 2);
        assert_eq!(start, 0);
        assert!(names.contains(&":pwd".to_string()));
        assert!(names.contains(&":profile".to_string()));
        assert!(!names.contains(&":help".to_string()));
    }

    #[test]
    fn completes_file_paths_after_shell_commands() {
        let h = helper();
        // `:sh cat Car` should offer Cargo.toml / Cargo.lock from the repo root
        let (_, names) = complete(&h, ":sh cat Car", 11);
        assert!(
            names.iter().any(|n| n.contains("Cargo.toml")),
            "expected Cargo.toml in {:?}",
            names
        );
        // Bare `!` shell shortcut too
        let (_, names) = complete(&h, "!cat Car", 8);
        assert!(names.iter().any(|n| n.contains("Cargo.toml")));
    }

    #[test]
    fn completes_file_paths_inside_string_literals() {
        let h = helper();
        let line = "fs.read(\"Car";
        let (_, names) = complete(&h, line, line.len());
        assert!(
            names.iter().any(|n| n.contains("Cargo.toml")),
            "expected Cargo.toml in {:?}",
            names
        );
    }

    #[test]
    fn completes_identifiers_in_code() {
        let mut h = helper();
        h.user_identifiers = vec!["my_variable".to_string()];
        let (start, names) = complete(&h, "pri", 3);
        assert_eq!(start, 0);
        assert_eq!(names, vec!["print".to_string(), "println".to_string()]);
        let (_, names) = complete(&h, "1 + my_v", 8);
        assert_eq!(names, vec!["my_variable".to_string()]);
    }

    #[test]
    fn string_literal_detection_handles_escapes() {
        assert!(ReplHelper::in_string_literal("fs.read(\"abc"));
        assert!(!ReplHelper::in_string_literal("fs.read(\"abc\")"));
        assert!(ReplHelper::in_string_literal("x = \"a\\\"b"));
        assert!(!ReplHelper::in_string_literal("let x = 1"));
    }

    #[test]
    fn incomplete_expression_ignores_brackets_in_comments() {
        let repl = Repl::new(false).expect("repl");
        // A bracket inside a `//` comment must not trigger multiline mode.
        assert!(!repl.is_incomplete_expression("1 + 1 // {"));
        assert!(!repl.is_incomplete_expression("f() // ( [ {"));
        // Real open brackets still do.
        assert!(repl.is_incomplete_expression("if x {"));
        assert!(repl.is_incomplete_expression("[1, 2,"));
        // `//` inside a string is content, not a comment.
        assert!(repl.is_incomplete_expression("\"http://x\" + ("));
        assert!(!repl.is_incomplete_expression("\"http://x\""));
        // Division doesn't start a comment.
        assert!(repl.is_incomplete_expression("(1 / 2"));
        assert!(!repl.is_incomplete_expression("1 / 2"));
    }
}

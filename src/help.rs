use std::collections::HashMap;
use std::fmt;

/// Color codes for terminal output
pub struct Colors;

impl Colors {
    pub const RESET: &'static str = "\x1b[0m";
    pub const BOLD: &'static str = "\x1b[1m";
    pub const BLUE: &'static str = "\x1b[34m";
    pub const GREEN: &'static str = "\x1b[32m";
    pub const YELLOW: &'static str = "\x1b[33m";
    pub const CYAN: &'static str = "\x1b[36m";
    pub const MAGENTA: &'static str = "\x1b[35m";
    pub const RED: &'static str = "\x1b[31m";
    pub const DIM: &'static str = "\x1b[2m";
}

/// Search result with relevance scoring
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub function_doc: FunctionDoc,
    pub relevance_score: f64,
    pub match_type: MatchType,
    pub matched_text: String,
}

/// Type of match found during search
#[derive(Debug, Clone)]
pub enum MatchType {
    ExactName,
    FuzzyName,
    Description,
    Category,
    Example,
    Parameter,
    Signature,
}

/// Search filters for advanced search
#[derive(Debug, Clone)]
pub struct SearchFilters {
    pub category: Option<String>,
    pub return_type: Option<String>,
    pub min_relevance: f64,
    pub max_results: usize,
    pub include_examples: bool,
    pub include_repl_commands: bool,
}

impl Default for SearchFilters {
    fn default() -> Self {
        Self {
            category: None,
            return_type: None,
            min_relevance: 0.1,
            max_results: 20,
            include_examples: true,
            include_repl_commands: true,
        }
    }
}

/// Tutorial step for interactive learning
#[derive(Debug, Clone)]
pub struct TutorialStep {
    pub title: String,
    pub description: String,
    pub code: String,
    pub expected_output: String,
    pub explanation: String,
    pub hints: Vec<String>,
}

/// Interactive tutorial definition
#[derive(Debug, Clone)]
pub struct Tutorial {
    pub name: String,
    pub description: String,
    pub difficulty: String,
    pub estimated_time: String,
    pub prerequisites: Vec<String>,
    pub steps: Vec<TutorialStep>,
}

/// Context information for help suggestions
#[derive(Debug, Clone)]
pub struct HelpContext {
    pub recent_commands: Vec<String>,
    pub current_variables: Vec<String>,
    pub last_error: Option<String>,
    pub current_working_category: Option<String>,
}

/// Function documentation
#[derive(Debug, Clone)]
pub struct FunctionDoc {
    pub name: String,
    pub description: String,
    pub syntax: String,
    pub parameters: Vec<String>,
    pub return_type: String,
    pub examples: Vec<String>,
    pub category: String,
    pub see_also: Vec<String>,
}

/// Comprehensive help system for Olang
pub struct HelpSystem {
    functions: HashMap<String, FunctionDoc>,
    categories: HashMap<String, Vec<String>>,
    tutorials: HashMap<String, Tutorial>,
}

impl Default for HelpSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl HelpSystem {
    pub fn new() -> Self {
        let mut help_system = Self {
            functions: HashMap::new(),
            categories: HashMap::new(),
            tutorials: HashMap::new(),
        };
        help_system.initialize_documentation();
        help_system.initialize_tutorials();
        help_system
    }

    /// Advanced search with fuzzy matching and filters
    pub fn search(&self, query: &str, filters: Option<SearchFilters>) -> Vec<SearchResult> {
        let filters = filters.unwrap_or_default();
        let mut results = Vec::new();

        for function in self.functions.values() {
            // Skip REPL commands if not included
            if !filters.include_repl_commands && function.name.starts_with(':') {
                continue;
            }

            // Category filter
            if let Some(ref category) = filters.category
                && !function
                    .category
                    .to_lowercase()
                    .contains(&category.to_lowercase())
            {
                continue;
            }

            // Return type filter
            if let Some(ref return_type) = filters.return_type
                && !function
                    .return_type
                    .to_lowercase()
                    .contains(&return_type.to_lowercase())
            {
                continue;
            }

            // Calculate relevance score
            let relevance = self.calculate_relevance(query, function);

            if relevance >= filters.min_relevance {
                let (match_type, matched_text) = self.determine_match_type(query, function);
                results.push(SearchResult {
                    function_doc: function.clone(),
                    relevance_score: relevance,
                    match_type,
                    matched_text,
                });
            }
        }

        // Sort by relevance score (descending)
        results.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Limit results
        results.truncate(filters.max_results);

        results
    }

    /// Calculate relevance score for search query
    fn calculate_relevance(&self, query: &str, function: &FunctionDoc) -> f64 {
        let query_lower = query.to_lowercase();
        let mut score = 0.0;

        // Exact name match (highest priority)
        if function.name.to_lowercase() == query_lower {
            score += 100.0;
        }
        // Name contains query
        else if function.name.to_lowercase().contains(&query_lower) {
            score += 80.0;
        }
        // Fuzzy name match
        else {
            score += self.fuzzy_match_score(&query_lower, &function.name.to_lowercase()) * 60.0;
        }

        // Description match
        if function.description.to_lowercase().contains(&query_lower) {
            score += 40.0;
        }

        // Category match
        if function.category.to_lowercase().contains(&query_lower) {
            score += 30.0;
        }

        // Example match
        for example in &function.examples {
            if example.to_lowercase().contains(&query_lower) {
                score += 20.0;
                break;
            }
        }

        // Parameter match
        for param in &function.parameters {
            if param.to_lowercase().contains(&query_lower) {
                score += 15.0;
                break;
            }
        }

        // Return type match
        if function.return_type.to_lowercase().contains(&query_lower) {
            score += 10.0;
        }

        // See also match
        for see_also in &function.see_also {
            if see_also.to_lowercase().contains(&query_lower) {
                score += 5.0;
                break;
            }
        }

        // Normalize score (0-1 range)
        score / 100.0
    }

    /// Determine the type of match found
    fn determine_match_type(&self, query: &str, function: &FunctionDoc) -> (MatchType, String) {
        let query_lower = query.to_lowercase();

        if function.name.to_lowercase() == query_lower {
            (MatchType::ExactName, function.name.clone())
        } else if function.name.to_lowercase().contains(&query_lower) {
            (MatchType::FuzzyName, function.name.clone())
        } else if function.description.to_lowercase().contains(&query_lower) {
            (MatchType::Description, function.description.clone())
        } else if function.category.to_lowercase().contains(&query_lower) {
            (MatchType::Category, function.category.clone())
        } else if function.return_type.to_lowercase().contains(&query_lower) {
            (MatchType::Signature, function.return_type.clone())
        } else {
            for example in &function.examples {
                if example.to_lowercase().contains(&query_lower) {
                    return (MatchType::Example, example.clone());
                }
            }
            for param in &function.parameters {
                if param.to_lowercase().contains(&query_lower) {
                    return (MatchType::Parameter, param.clone());
                }
            }
            (MatchType::FuzzyName, function.name.clone())
        }
    }

    /// Format search results for display
    pub fn format_search_results(&self, results: &[SearchResult]) -> String {
        if results.is_empty() {
            return format!(
                "{}No functions found matching your search.{}",
                Colors::YELLOW,
                Colors::RESET
            );
        }

        let mut output = format!(
            "{}=== Search Results ({} found) ==={}\n\n",
            Colors::BOLD,
            results.len(),
            Colors::RESET
        );

        for (i, result) in results.iter().enumerate() {
            let match_icon = match result.match_type {
                MatchType::ExactName => "EXACT",
                MatchType::FuzzyName => "FUZZY",
                MatchType::Description => "DESC",
                MatchType::Category => "CAT",
                MatchType::Example => "EX",
                MatchType::Parameter => "PARAM",
                MatchType::Signature => "SIG",
            };

            output.push_str(&format!(
                "{}{}. {}{} {}{}{} {}({}){}\n",
                Colors::DIM,
                i + 1,
                match_icon,
                Colors::RESET,
                Colors::BLUE,
                result.function_doc.name,
                Colors::RESET,
                Colors::DIM,
                result.function_doc.category,
                Colors::RESET
            ));

            output.push_str(&format!(
                "   {}{}{}\n",
                Colors::GREEN,
                result.function_doc.description,
                Colors::RESET
            ));

            // Show match context
            match result.match_type {
                MatchType::Description => {
                    output.push_str(&format!(
                        "   {}Match:{} {}\n",
                        Colors::YELLOW,
                        Colors::RESET,
                        result.matched_text
                    ));
                }
                MatchType::Example => {
                    output.push_str(&format!(
                        "   {}Example:{} {}{}{}\n",
                        Colors::YELLOW,
                        Colors::RESET,
                        Colors::CYAN,
                        result.matched_text,
                        Colors::RESET
                    ));
                }
                _ => {}
            }

            output.push_str(&format!(
                "   {}Syntax:{} {}{}{}\n",
                Colors::MAGENTA,
                Colors::RESET,
                Colors::CYAN,
                result.function_doc.syntax,
                Colors::RESET
            ));

            output.push('\n');
        }

        output.push_str(&format!(
            "{}TIP: Use ':help <function>' for detailed documentation{}\n",
            Colors::DIM,
            Colors::RESET
        ));

        output
    }

    /// Search with category filter
    pub fn search_in_category(&self, query: &str, category: &str) -> Vec<SearchResult> {
        let filters = SearchFilters {
            category: Some(category.to_string()),
            ..Default::default()
        };
        self.search(query, Some(filters))
    }

    /// Search by return type
    pub fn search_by_return_type(&self, return_type: &str) -> Vec<SearchResult> {
        let filters = SearchFilters {
            return_type: Some(return_type.to_string()),
            ..Default::default()
        };
        self.search(return_type, Some(filters))
    }

    /// Initialize interactive tutorials
    fn initialize_tutorials(&mut self) {
        self.add_tutorial(Tutorial {
            name: "basic_operations".to_string(),
            description: "Learn basic Olang operations and syntax".to_string(),
            difficulty: "Beginner".to_string(),
            estimated_time: "5 minutes".to_string(),
            prerequisites: vec![],
            steps: vec![
                TutorialStep {
                    title: "Basic Arithmetic".to_string(),
                    description: "Let's start with simple arithmetic operations".to_string(),
                    code: "2 + 3 * 4".to_string(),
                    expected_output: "14".to_string(),
                    explanation: "Olang follows standard operator precedence: multiplication before addition".to_string(),
                    hints: vec!["Use parentheses to change precedence: (2 + 3) * 4".to_string()],
                },
                TutorialStep {
                    title: "Variables".to_string(),
                    description: "Creating and using variables".to_string(),
                    code: "let name = \"Alice\"\nlet age = 25\nprintln(name + \" is \" + age + \" years old\")".to_string(),
                    expected_output: "Alice is 25 years old".to_string(),
                    explanation: "Variables are created with 'let' and can be reassigned".to_string(),
                    hints: vec!["Use to_string() to convert numbers to strings if needed".to_string()],
                },
                TutorialStep {
                    title: "Functions".to_string(),
                    description: "Defining and calling functions".to_string(),
                    code: "fn greet(name) = \"Hello, \" + name + \"!\"\ngreet(\"World\")".to_string(),
                    expected_output: "Hello, World!".to_string(),
                    explanation: "Functions are defined with 'fn' and can have parameters".to_string(),
                    hints: vec!["Use => for multi-line functions: fn name() => { ... }".to_string()],
                },
            ],
        });

        self.add_tutorial(Tutorial {
            name: "list_operations".to_string(),
            description: "Master list operations and functional programming".to_string(),
            difficulty: "Intermediate".to_string(),
            estimated_time: "10 minutes".to_string(),
            prerequisites: vec!["basic_operations".to_string()],
            steps: vec![
                TutorialStep {
                    title: "Creating Lists".to_string(),
                    description: "Different ways to create lists".to_string(),
                    code: "let numbers = [1, 2, 3, 4, 5]\nlet range_list = range(5)\nlet empty_list = []".to_string(),
                    expected_output: "[1, 2, 3, 4, 5]\n[0, 1, 2, 3, 4]\n[]".to_string(),
                    explanation: "Lists can be created with literals, ranges, or empty".to_string(),
                    hints: vec!["Use 1..5 for range syntax".to_string()],
                },
                TutorialStep {
                    title: "Map Function".to_string(),
                    description: "Transform lists with map".to_string(),
                    code: "let numbers = [1, 2, 3, 4, 5]\nmap(numbers, (x) => x * 2)".to_string(),
                    expected_output: "[2, 4, 6, 8, 10]".to_string(),
                    explanation: "Map applies a function to each element".to_string(),
                    hints: vec!["Use pipeline syntax: numbers |> map((x) => x * 2)".to_string()],
                },
                TutorialStep {
                    title: "Filter Function".to_string(),
                    description: "Filter lists based on conditions".to_string(),
                    code: "let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]\nfilter(numbers, (x) => x % 2 == 0)".to_string(),
                    expected_output: "[2, 4, 6, 8, 10]".to_string(),
                    explanation: "Filter keeps only elements that satisfy the condition".to_string(),
                    hints: vec!["Use > 5 to filter numbers greater than 5".to_string()],
                },
                TutorialStep {
                    title: "Pipeline Operations".to_string(),
                    description: "Chain operations with pipelines".to_string(),
                    code: "range(10) |> map((x) => x * x) |> filter((x) => x > 10)".to_string(),
                    expected_output: "[16, 25, 36, 49, 64, 81]".to_string(),
                    explanation: "Pipelines allow chaining operations left-to-right".to_string(),
                    hints: vec!["Add |> reduce(0, (a, b) => a + b) to sum the results".to_string()],
                },
            ],
        });

        self.add_tutorial(Tutorial {
            name: "file_handling".to_string(),
            description: "Working with files and directories".to_string(),
            difficulty: "Intermediate".to_string(),
            estimated_time: "8 minutes".to_string(),
            prerequisites: vec!["basic_operations".to_string()],
            steps: vec![
                TutorialStep {
                    title: "Check File Existence".to_string(),
                    description: "Check if a file exists before working with it".to_string(),
                    code: "fs.exists(\"README.md\")".to_string(),
                    expected_output: "true".to_string(),
                    explanation: "Always check file existence to avoid errors".to_string(),
                    hints: vec!["Use fs.is_file() to check if it's specifically a file".to_string()],
                },
                TutorialStep {
                    title: "Read File Content".to_string(),
                    description: "Read content from a file".to_string(),
                    code: "match fs.read_file(\"README.md\") {\n  Ok(content) => println(\"File size: \" + length(content)),\n  Err(error) => println(\"Error: \" + error)\n}".to_string(),
                    expected_output: "File size: 1234".to_string(),
                    explanation: "File operations return Result types for error handling".to_string(),
                    hints: vec!["Use fs.read_file() for text files".to_string()],
                },
                TutorialStep {
                    title: "List Directory".to_string(),
                    description: "List files and directories".to_string(),
                    code: "match fs.list_dir(\".\") {\n  Ok(files) => println(\"Files: \" + length(files)),\n  Err(error) => println(\"Error: \" + error)\n}".to_string(),
                    expected_output: "Files: 15".to_string(),
                    explanation: "Directory operations also return Result types".to_string(),
                    hints: vec!["Use filter to find specific file types".to_string()],
                },
            ],
        });

        self.add_tutorial(Tutorial {
            name: "web_requests".to_string(),
            description: "Making HTTP requests and handling responses".to_string(),
            difficulty: "Advanced".to_string(),
            estimated_time: "12 minutes".to_string(),
            prerequisites: vec!["basic_operations".to_string(), "file_handling".to_string()],
            steps: vec![
                TutorialStep {
                    title: "Simple GET Request".to_string(),
                    description: "Make a basic HTTP GET request".to_string(),
                    code: "match http.get(\"https://httpbin.org/get\") {\n  Ok(response) => println(\"Status: \" + response.status),\n  Err(error) => println(\"Error: \" + error)\n}".to_string(),
                    expected_output: "Status: 200".to_string(),
                    explanation: "HTTP requests return Result types with response objects".to_string(),
                    hints: vec!["Access response.body for the response content".to_string()],
                },
                TutorialStep {
                    title: "JSON Response Handling".to_string(),
                    description: "Parse JSON responses from APIs".to_string(),
                    code: "match http.get(\"https://api.github.com/users/octocat\") {\n  Ok(response) => match json.parse(response.body) {\n    Ok(user) => println(\"User: \" + user.login),\n    Err(e) => println(\"JSON Error: \" + e)\n  },\n  Err(e) => println(\"HTTP Error: \" + e)\n}".to_string(),
                    expected_output: "User: octocat".to_string(),
                    explanation: "Chain HTTP requests with JSON parsing for API data".to_string(),
                    hints: vec!["Use json.get() for accessing nested JSON properties".to_string()],
                },
                TutorialStep {
                    title: "POST Request with Data".to_string(),
                    description: "Send data with POST requests".to_string(),
                    code: "let data = json.stringify({\"name\": \"test\", \"email\": \"test@example.com\"})\nmatch http.post(\"https://httpbin.org/post\", data) {\n  Ok(response) => println(\"Created: \" + response.status),\n  Err(error) => println(\"Error: \" + error)\n}".to_string(),
                    expected_output: "Created: 200".to_string(),
                    explanation: "POST requests can send JSON data to APIs".to_string(),
                    hints: vec!["Use http.put() for updates, http.delete() for deletions".to_string()],
                },
            ],
        });
    }

    /// Add a tutorial to the system
    fn add_tutorial(&mut self, tutorial: Tutorial) {
        self.tutorials.insert(tutorial.name.clone(), tutorial);
    }

    /// Get available tutorials
    pub fn get_tutorials(&self) -> Vec<&Tutorial> {
        self.tutorials.values().collect()
    }

    /// Get specific tutorial
    pub fn get_tutorial(&self, name: &str) -> Option<&Tutorial> {
        self.tutorials.get(name)
    }

    /// Format tutorial list for display
    pub fn format_tutorial_list(&self) -> String {
        let mut output = format!(
            "{}=== Interactive Tutorials ==={}\n\n",
            Colors::BOLD,
            Colors::RESET
        );

        let mut tutorials: Vec<_> = self.tutorials.values().collect();
        tutorials.sort_by(|a, b| {
            let order = ["Beginner", "Intermediate", "Advanced"];
            let a_idx = order.iter().position(|&x| x == a.difficulty).unwrap_or(999);
            let b_idx = order.iter().position(|&x| x == b.difficulty).unwrap_or(999);
            a_idx.cmp(&b_idx)
        });

        for tutorial in tutorials {
            let difficulty_color = match tutorial.difficulty.as_str() {
                "Beginner" => Colors::GREEN,
                "Intermediate" => Colors::YELLOW,
                "Advanced" => Colors::RED,
                _ => Colors::BLUE,
            };

            output.push_str(&format!(
                "{}TUTORIAL: {}{} {}({}{}{}){}\n",
                Colors::BLUE,
                tutorial.name,
                Colors::RESET,
                difficulty_color,
                Colors::BOLD,
                tutorial.difficulty,
                Colors::RESET,
                Colors::RESET
            ));

            output.push_str(&format!(
                "   {}{}{}\n",
                Colors::DIM,
                tutorial.description,
                Colors::RESET
            ));

            output.push_str(&format!(
                "   {}TIME: {} • {} steps{}\n",
                Colors::DIM,
                tutorial.estimated_time,
                tutorial.steps.len(),
                Colors::RESET
            ));

            if !tutorial.prerequisites.is_empty() {
                output.push_str(&format!(
                    "   {}Prerequisites: {}{}\n",
                    Colors::DIM,
                    tutorial.prerequisites.join(", "),
                    Colors::RESET
                ));
            }

            output.push('\n');
        }

        output.push_str(&format!(
            "{}TIP: Start a tutorial: :help tutorial <name>{}\n",
            Colors::DIM,
            Colors::RESET
        ));

        output
    }

    /// Format tutorial for display
    pub fn format_tutorial(&self, tutorial: &Tutorial) -> String {
        let mut output = format!(
            "{}=== Tutorial: {} ==={}\n\n",
            Colors::BOLD,
            tutorial.name,
            Colors::RESET
        );

        output.push_str(&format!(
            "{}{}{}\n",
            Colors::GREEN,
            tutorial.description,
            Colors::RESET
        ));

        output.push_str(&format!(
            "{}Difficulty:{} {}\n",
            Colors::YELLOW,
            Colors::RESET,
            tutorial.difficulty
        ));

        output.push_str(&format!(
            "{}Estimated Time:{} {}\n",
            Colors::YELLOW,
            Colors::RESET,
            tutorial.estimated_time
        ));

        if !tutorial.prerequisites.is_empty() {
            output.push_str(&format!(
                "{}Prerequisites:{} {}\n",
                Colors::YELLOW,
                Colors::RESET,
                tutorial.prerequisites.join(", ")
            ));
        }

        output.push('\n');

        for (i, step) in tutorial.steps.iter().enumerate() {
            output.push_str(&format!(
                "{}Step {}: {}{}\n",
                Colors::CYAN,
                i + 1,
                step.title,
                Colors::RESET
            ));

            output.push_str(&format!(
                "{}{}{}\n\n",
                Colors::DIM,
                step.description,
                Colors::RESET
            ));

            output.push_str(&format!(
                "{}Code to try:{}\n",
                Colors::MAGENTA,
                Colors::RESET
            ));

            output.push_str(&format!(
                "{}{}\n{}\n\n",
                Colors::BLUE,
                step.code,
                Colors::RESET
            ));

            output.push_str(&format!(
                "{}Expected Output:{}\n",
                Colors::GREEN,
                Colors::RESET
            ));

            output.push_str(&format!(
                "{}{}\n{}\n\n",
                Colors::GREEN,
                step.expected_output,
                Colors::RESET
            ));

            output.push_str(&format!(
                "{}Explanation:{}\n",
                Colors::YELLOW,
                Colors::RESET
            ));

            output.push_str(&format!(
                "{}{}{}\n\n",
                Colors::DIM,
                step.explanation,
                Colors::RESET
            ));

            if !step.hints.is_empty() {
                output.push_str(&format!("{}HINTS:{}\n", Colors::CYAN, Colors::RESET));

                for hint in &step.hints {
                    output.push_str(&format!("   • {}{}{}\n", Colors::DIM, hint, Colors::RESET));
                }
                output.push('\n');
            }

            output.push_str("---\n\n");
        }

        output.push_str(&format!(
            "{}CONGRATULATIONS! You've completed the {} tutorial!{}\n",
            Colors::GREEN,
            tutorial.name,
            Colors::RESET
        ));

        output
    }

    /// Context-sensitive help suggestions
    pub fn get_contextual_help(&self, context: &HelpContext) -> Vec<String> {
        let mut suggestions = Vec::new();

        // Analyze recent commands for patterns
        for command in &context.recent_commands {
            if command.contains("map") || command.contains("filter") || command.contains("reduce") {
                suggestions
                    .push("Tutorial: list_operations - Learn advanced list processing".to_string());
                suggestions.push("help pipeline - Learn about pipeline operations".to_string());
                break;
            }
        }

        // Check for file operations
        for command in &context.recent_commands {
            if command.contains("fs.") {
                suggestions.push("Tutorial: file_handling - Master file operations".to_string());
                suggestions.push("help fs - File system functions".to_string());
                break;
            }
        }

        // Check for HTTP operations
        for command in &context.recent_commands {
            if command.contains("http.") {
                suggestions.push("Tutorial: web_requests - HTTP and API interactions".to_string());
                suggestions.push("help http - HTTP client functions".to_string());
                break;
            }
        }

        // Error-based suggestions
        if let Some(ref error) = context.last_error {
            if error.contains("ParseError") || error.contains("syntax") {
                suggestions.push("help error.syntax - Common syntax errors and fixes".to_string());
            }
            if error.contains("TypeError") || error.contains("type") {
                suggestions.push(":help error.types - Type errors and conversions".to_string());
            }
            if error.contains("undefined") || error.contains("not found") {
                suggestions.push(":help error.runtime - Runtime error debugging".to_string());
            }
        }

        // Variable-based suggestions
        if !context.current_variables.is_empty() {
            let has_lists = context
                .current_variables
                .iter()
                .any(|v| v.contains("list") || v.contains("array"));
            if has_lists {
                suggestions.push(":help list - List manipulation functions".to_string());
            }
        }

        // Working category suggestions
        if let Some(ref category) = context.current_working_category
            && category == "List"
        {
            suggestions.push(":help map - Transform lists with functions".to_string());
            suggestions.push(":help filter - Filter lists by conditions".to_string());
            suggestions.push(":help reduce - Reduce lists to single values".to_string());
        }

        // Always include general suggestions
        if suggestions.is_empty() {
            suggestions.push("Tutorial: basic_operations - Learn Olang fundamentals".to_string());
            suggestions.push(":help examples - See practical examples".to_string());
            suggestions.push(":help syntax - Language syntax reference".to_string());
        }

        suggestions
    }

    /// Format contextual help for display
    pub fn format_contextual_help(&self, context: &HelpContext) -> String {
        let suggestions = self.get_contextual_help(context);

        let mut output = format!(
            "{}=== Contextual Help Suggestions ==={}\n\n",
            Colors::BOLD,
            Colors::RESET
        );

        output.push_str(&format!(
            "{}Based on your recent activity:{}\n\n",
            Colors::GREEN,
            Colors::RESET
        ));

        for (i, suggestion) in suggestions.iter().enumerate() {
            output.push_str(&format!(
                "{}{}. {}{}{}\n",
                Colors::BLUE,
                i + 1,
                Colors::RESET,
                suggestion,
                Colors::RESET
            ));
        }

        output.push('\n');

        if !context.recent_commands.is_empty() {
            output.push_str(&format!(
                "{}Recent Commands:{}\n",
                Colors::YELLOW,
                Colors::RESET
            ));

            for command in context.recent_commands.iter().take(3) {
                output.push_str(&format!(
                    "  • {}{}{}\n",
                    Colors::DIM,
                    command,
                    Colors::RESET
                ));
            }
            output.push('\n');
        }

        if let Some(ref error) = context.last_error {
            output.push_str(&format!("{}Last Error:{}\n", Colors::RED, Colors::RESET));

            output.push_str(&format!("  {}{}{}\n\n", Colors::DIM, error, Colors::RESET));
        }

        output.push_str(&format!(
            "{}TIP: Type ':help <topic>' for detailed information{}\n",
            Colors::DIM,
            Colors::RESET
        ));

        output
    }

    /// Initialize all function documentation
    fn initialize_documentation(&mut self) {
        // REPL Commands Documentation
        self.add_repl_commands();

        // Error Help Topics
        self.add_error_help_topics();

        // I/O Functions
        self.add_function(FunctionDoc {
            name: "println".to_string(),
            description: "Prints a value followed by a newline to standard output".to_string(),
            syntax: "println(value)".to_string(),
            parameters: vec!["value: Any - The value to print".to_string()],
            return_type: "Unit".to_string(),
            examples: vec![
                "println(\"Hello, World!\")".to_string(),
                "println(42)".to_string(),
                "println([1, 2, 3])".to_string(),
            ],
            category: "I/O".to_string(),
            see_also: vec!["print".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "print".to_string(),
            description: "Prints a value to standard output without a trailing newline".to_string(),
            syntax: "print(value)".to_string(),
            parameters: vec!["value: Any - The value to print".to_string()],
            return_type: "Unit".to_string(),
            examples: vec![
                "print(\"Hello\"); print(\" World!\")".to_string(),
                "print(42)".to_string(),
            ],
            category: "I/O".to_string(),
            see_also: vec!["println".to_string()],
        });

        // List Functions
        self.add_function(FunctionDoc {
            name: "map".to_string(),
            description: "Applies a function to each element in a list, returning a new list"
                .to_string(),
            syntax: "map(list, function)".to_string(),
            parameters: vec![
                "list: List[T] - The list to transform".to_string(),
                "function: T -> U - Function to apply to each element".to_string(),
            ],
            return_type: "List[U]".to_string(),
            examples: vec![
                "map([1, 2, 3], (x) => x * 2)  // [2, 4, 6]".to_string(),
                "[1, 2, 3] |> map((x) => x + 1)  // [2, 3, 4]".to_string(),
                "map(range(5), (x) => x * x)  // [0, 1, 4, 9, 16]".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec![
                "filter".to_string(),
                "reduce".to_string(),
                "fold".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "par_map".to_string(),
            description: "map fanned out across OS threads: same results in the same order, \
                          one worker interpreter per thread (no GIL). The function runs against \
                          worker snapshots like spawn, so mutations to enclosing state are not \
                          visible to the caller"
                .to_string(),
            syntax: "par_map(list, function)".to_string(),
            parameters: vec![
                "list: List[T] - The list to transform".to_string(),
                "function: T -> U - Function to apply to each element (effectively pure)"
                    .to_string(),
            ],
            return_type: "List[U]".to_string(),
            examples: vec![
                "par_map([1, 2, 3], (x) => x * 2)  // [2, 4, 6]".to_string(),
                "1..1000 |> par_map(expensive_fn)  // uses every core".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec![
                "map".to_string(),
                "par_filter".to_string(),
                "spawn".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "par_filter".to_string(),
            description: "filter fanned out across OS threads — the parallel twin of filter, \
                          with the same keep-on-true rule and ordering"
                .to_string(),
            syntax: "par_filter(list, predicate)".to_string(),
            parameters: vec![
                "list: List[T] - The list to filter".to_string(),
                "predicate: T -> Bool - Keep elements where this returns true".to_string(),
            ],
            return_type: "List[T]".to_string(),
            examples: vec!["par_filter(1..100, (x) => is_expensive_check(x))".to_string()],
            category: "List".to_string(),
            see_also: vec!["filter".to_string(), "par_map".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "filter".to_string(),
            description: "Returns a new list containing only elements that satisfy the predicate"
                .to_string(),
            syntax: "filter(list, predicate)".to_string(),
            parameters: vec![
                "list: List[T] - The list to filter".to_string(),
                "predicate: T -> Bool - Function that returns true for elements to keep"
                    .to_string(),
            ],
            return_type: "List[T]".to_string(),
            examples: vec![
                "filter([1, 2, 3, 4, 5], (x) => x % 2 == 0)  // [2, 4]".to_string(),
                "range(10) |> filter((x) => x > 5)  // [6, 7, 8, 9]".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec!["map".to_string(), "reduce".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "reduce".to_string(),
            description: "Reduces a list to a single value using a binary function".to_string(),
            syntax: "reduce(list, initial, function)".to_string(),
            parameters: vec![
                "list: List[T] - The list to reduce".to_string(),
                "initial: U - Initial accumulator value".to_string(),
                "function: (U, T) -> U - Binary function for reduction".to_string(),
            ],
            return_type: "U".to_string(),
            examples: vec![
                "reduce([1, 2, 3, 4], 0, (acc, x) => acc + x)  // 10".to_string(),
                "reduce([\"a\", \"b\", \"c\"], \"\", (acc, s) => acc + s)  // \"abc\"".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec!["fold".to_string(), "map".to_string(), "filter".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fold".to_string(),
            description: "Alias for reduce - folds a list into a single value".to_string(),
            syntax: "fold(list, initial, function)".to_string(),
            parameters: vec![
                "list: List[T] - The list to fold".to_string(),
                "initial: U - Initial accumulator value".to_string(),
                "function: (U, T) -> U - Binary function for folding".to_string(),
            ],
            return_type: "U".to_string(),
            examples: vec!["fold([1, 2, 3], 0, (acc, x) => acc + x)  // 6".to_string()],
            category: "List".to_string(),
            see_also: vec!["reduce".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "len".to_string(),
            description: "Returns the length of a list, string, or tuple".to_string(),
            syntax: "len(collection)".to_string(),
            parameters: vec![
                "collection: List[T] | String | Tuple - The collection to measure".to_string(),
            ],
            return_type: "Int".to_string(),
            examples: vec![
                "len([1, 2, 3, 4])  // 4".to_string(),
                "len(\"hello\")  // 5".to_string(),
            ],
            category: "Collection".to_string(),
            see_also: vec!["head".to_string(), "tail".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "head".to_string(),
            description: "Returns the first element of a non-empty list".to_string(),
            syntax: "head(list)".to_string(),
            parameters: vec!["list: List[T] - A non-empty list".to_string()],
            return_type: "T".to_string(),
            examples: vec![
                "head([1, 2, 3])  // 1".to_string(),
                "range(5) |> head  // 0".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec!["tail".to_string(), "cons".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "tail".to_string(),
            description: "Returns all elements except the first from a non-empty list".to_string(),
            syntax: "tail(list)".to_string(),
            parameters: vec!["list: List[T] - A non-empty list".to_string()],
            return_type: "List[T]".to_string(),
            examples: vec![
                "tail([1, 2, 3])  // [2, 3]".to_string(),
                "range(5) |> tail  // [1, 2, 3, 4]".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec!["head".to_string(), "cons".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "cons".to_string(),
            description: "Prepends an element to the front of a list".to_string(),
            syntax: "cons(element, list)".to_string(),
            parameters: vec![
                "element: T - Element to prepend".to_string(),
                "list: List[T] - List to prepend to".to_string(),
            ],
            return_type: "List[T]".to_string(),
            examples: vec![
                "cons(1, [2, 3])  // [1, 2, 3]".to_string(),
                "cons(0, range(3))  // [0, 0, 1, 2]".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec!["head".to_string(), "tail".to_string()],
        });

        // Range and Generation
        self.add_function(FunctionDoc {
            name: "range".to_string(),
            description: "Generates a sequence of integers with flexible argument patterns"
                .to_string(),
            syntax: "range(end) | range(start, end) | range(start, end, step)".to_string(),
            parameters: vec![
                "start: Int (optional) - Starting value (default: 0)".to_string(),
                "end: Int - End value (exclusive)".to_string(),
                "step: Int (optional) - Step size (default: 1)".to_string(),
            ],
            return_type: "List[Int]".to_string(),
            examples: vec![
                "range(5)  // [0, 1, 2, 3, 4]".to_string(),
                "range(2, 8)  // [2, 3, 4, 5, 6, 7]".to_string(),
                "range(0, 10, 2)  // [0, 2, 4, 6, 8]".to_string(),
                "1..5  // [1, 2, 3, 4] (syntax alternative)".to_string(),
                "1..=5  // [1, 2, 3, 4, 5] (inclusive)".to_string(),
            ],
            category: "Generation".to_string(),
            see_also: vec!["zip".to_string(), "map".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "zip".to_string(),
            description: "Combines two lists into a list of tuples".to_string(),
            syntax: "zip(list1, list2)".to_string(),
            parameters: vec![
                "list1: List[T] - First list to zip".to_string(),
                "list2: List[U] - Second list to zip".to_string(),
            ],
            return_type: "List[(T, U)]".to_string(),
            examples: vec![
                "zip([1, 2, 3], [\"a\", \"b\", \"c\"])  // [(1, \"a\"), (2, \"b\"), (3, \"c\")]"
                    .to_string(),
                "zip(range(3), [\"x\", \"y\", \"z\"])".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec!["range".to_string(), "map".to_string()],
        });

        // Type Conversion
        self.add_function(FunctionDoc {
            name: "to_string".to_string(),
            description: "Converts any value to its string representation".to_string(),
            syntax: "to_string(value)".to_string(),
            parameters: vec!["value: Any - Value to convert to string".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                "to_string(42)  // \"42\"".to_string(),
                "to_string(true)  // \"true\"".to_string(),
            ],
            category: "Conversion".to_string(),
            see_also: vec!["to_int".to_string(), "to_float".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "to_int".to_string(),
            description: "Converts a value to an integer".to_string(),
            syntax: "to_int(value)".to_string(),
            parameters: vec![
                "value: Int | Float | String - Value to convert to integer".to_string(),
            ],
            return_type: "Int".to_string(),
            examples: vec![
                "to_int(3.14)  // 3".to_string(),
                "to_int(\"42\")  // 42".to_string(),
            ],
            category: "Conversion".to_string(),
            see_also: vec!["to_float".to_string(), "to_string".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "to_float".to_string(),
            description: "Converts a value to a floating-point number".to_string(),
            syntax: "to_float(value)".to_string(),
            parameters: vec!["value: Int | Float | String - Value to convert to float".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "to_float(42)  // 42.0".to_string(),
                "to_float(\"3.14\")  // 3.14".to_string(),
            ],
            category: "Conversion".to_string(),
            see_also: vec!["to_int".to_string(), "to_string".to_string()],
        });

        // Type introspection
        self.add_function(FunctionDoc {
            name: "typeof".to_string(),
            description:
                "Returns the type name of a value as a string for debugging and introspection"
                    .to_string(),
            syntax: "typeof(value)".to_string(),
            parameters: vec!["value: Any - The value to get the type of".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                "typeof(42)  // \"Int\"".to_string(),
                "typeof(\"hello\")  // \"String\"".to_string(),
                "typeof([1, 2, 3])  // \"List\"".to_string(),
                "typeof(true)  // \"Bool\"".to_string(),
                "typeof(3.14)  // \"Float\"".to_string(),
            ],
            category: "Introspection".to_string(),
            see_also: vec!["to_string".to_string()],
        });

        // List manipulation utilities
        self.add_function(FunctionDoc {
            name: "reverse".to_string(),
            description: "Returns a new list with elements in reverse order".to_string(),
            syntax: "reverse(list)".to_string(),
            parameters: vec!["list: List[T] - The list to reverse".to_string()],
            return_type: "List[T]".to_string(),
            examples: vec![
                "reverse([1, 2, 3, 4])  // [4, 3, 2, 1]".to_string(),
                "reverse([\"a\", \"b\", \"c\"])  // [\"c\", \"b\", \"a\"]".to_string(),
                "range(5) |> reverse  // [4, 3, 2, 1, 0]".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec!["sort".to_string(), "map".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "sort".to_string(),
            description: "Returns a new list with elements sorted in ascending order".to_string(),
            syntax: "sort(list)".to_string(),
            parameters: vec![
                "list: List[T] - The list to sort (elements must be comparable)".to_string(),
            ],
            return_type: "List[T]".to_string(),
            examples: vec![
                "sort([3, 1, 4, 1, 5])  // [1, 1, 3, 4, 5]".to_string(),
                "sort([\"zebra\", \"apple\", \"banana\"])  // [\"apple\", \"banana\", \"zebra\"]"
                    .to_string(),
                "sort([true, false, true])  // [false, true, true]".to_string(),
            ],
            category: "List".to_string(),
            see_also: vec!["reverse".to_string(), "filter".to_string()],
        });

        // String/List conversion utilities
        self.add_function(FunctionDoc {
            name: "join".to_string(),
            description: "Joins list elements into a single string using a separator".to_string(),
            syntax: "join(list, separator)".to_string(),
            parameters: vec![
                "list: List[T] - The list of elements to join".to_string(),
                "separator: String - The string to place between elements".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "join([\"hello\", \"world\"], \" \")  // \"hello world\"".to_string(),
                "join([1, 2, 3, 4], \", \")  // \"1, 2, 3, 4\"".to_string(),
                "join([\"a\", \"b\", \"c\"], \"\")  // \"abc\"".to_string(),
                "range(3) |> join(\"-\")  // \"0-1-2\"".to_string(),
            ],
            category: "Conversion".to_string(),
            see_also: vec!["split".to_string(), "to_string".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "split".to_string(),
            description: "Splits a string into a list of strings using a separator".to_string(),
            syntax: "split(string, separator)".to_string(),
            parameters: vec![
                "string: String - The string to split".to_string(),
                "separator: String - The separator to split on".to_string(),
            ],
            return_type: "List[String]".to_string(),
            examples: vec![
                "split(\"hello world\", \" \")  // [\"hello\", \"world\"]".to_string(),
                "split(\"a,b,c,d\", \",\")  // [\"a\", \"b\", \"c\", \"d\"]".to_string(),
                "split(\"one-two-three\", \"-\")  // [\"one\", \"two\", \"three\"]".to_string(),
            ],
            category: "Conversion".to_string(),
            see_also: vec!["join".to_string(), "filter".to_string()],
        });

        // Search and query utilities
        self.add_function(FunctionDoc {
            name: "contains".to_string(),
            description: "Checks if a list contains a specific element".to_string(),
            syntax: "contains(list, element)".to_string(),
            parameters: vec![
                "list: List[T] - The list to search in".to_string(),
                "element: T - The element to search for".to_string(),
            ],
            return_type: "Bool".to_string(),
            examples: vec![
                "contains([1, 2, 3, 4], 3)  // true".to_string(),
                "contains([\"a\", \"b\", \"c\"], \"d\")  // false".to_string(),
                "contains(range(10), 5)  // true".to_string(),
                "contains([], 42)  // false".to_string(),
            ],
            category: "Search".to_string(),
            see_also: vec!["filter".to_string(), "len".to_string()],
        });

        // === v0.5 draft built-ins ===
        self.add_function(FunctionDoc {
            name: "sum".to_string(),
            description: "Returns the arithmetic sum of all numeric elements in a list".to_string(),
            syntax: "sum(list)".to_string(),
            parameters: vec!["list: [Int|Float] – Numeric list".to_string()],
            return_type: "Int | Float".to_string(),
            examples: vec![
                "sum([1,2,3])  // 6".to_string(),
                "sum([1.5,2.5])  // 4.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["average".to_string(), "reduce".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "average".to_string(),
            description: "Mean value of a numeric list; returns Err on empty list".to_string(),
            syntax: "average(list)".to_string(),
            parameters: vec!["list: [Int|Float]".to_string()],
            return_type: "Result<Float, EmptyList>".to_string(),
            examples: vec!["average([1,2,3,4])  // 2.5".to_string()],
            category: "Math".to_string(),
            see_also: vec!["sum".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "min".to_string(),
            description: "Smallest element of a list".to_string(),
            syntax: "min(list)".to_string(),
            parameters: vec!["list: [T]".to_string()],
            return_type: "Result<T, EmptyList>".to_string(),
            examples: vec!["min([5,2,9])  // 2".to_string()],
            category: "Math".to_string(),
            see_also: vec!["max".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "max".to_string(),
            description: "Largest element of a list".to_string(),
            syntax: "max(list)".to_string(),
            parameters: vec!["list: [T]".to_string()],
            return_type: "Result<T, EmptyList>".to_string(),
            examples: vec!["max([5,2,9])  // 9".to_string()],
            category: "Math".to_string(),
            see_also: vec!["min".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "clamp".to_string(),
            description: "Restricts numeric x to the inclusive range [lo,hi]".to_string(),
            syntax: "clamp(x, lo, hi)".to_string(),
            parameters: vec![
                "x: Int|Float".to_string(),
                "lo: Int|Float".to_string(),
                "hi: Int|Float".to_string(),
            ],
            return_type: "Same as x".to_string(),
            examples: vec!["clamp(10,0,5)  // 5".to_string()],
            category: "Math".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "flatten".to_string(),
            description: "Flattens one level of nested lists".to_string(),
            syntax: "flatten(list_of_lists)".to_string(),
            parameters: vec!["list_of_lists: [[T]]".to_string()],
            return_type: "[T]".to_string(),
            examples: vec!["flatten([[1,2],[3]])  // [1,2,3]".to_string()],
            category: "List".to_string(),
            see_also: vec!["chunk".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "chunk".to_string(),
            description: "Splits a list into fixed-size chunks".to_string(),
            syntax: "chunk(list, size)".to_string(),
            parameters: vec!["list: [T]".to_string(), "size: Int>0".to_string()],
            return_type: "[[T]]".to_string(),
            examples: vec!["chunk([1,2,3,4,5],2)  // [[1,2],[3,4],[5]]".to_string()],
            category: "List".to_string(),
            see_also: vec!["flatten".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "enumerate".to_string(),
            description: "Pairs each element with its index".to_string(),
            syntax: "enumerate(list)".to_string(),
            parameters: vec!["list: [T]".to_string()],
            return_type: "[(Int,T)]".to_string(),
            examples: vec!["enumerate([\"a\", \"b\"])  // [(0,\"a\"),(1,\"b\")]".to_string()],
            category: "List".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "find".to_string(),
            description: "First element satisfying predicate; Err if none".to_string(),
            syntax: "find(list, predicate)".to_string(),
            parameters: vec!["list: [T]".to_string(), "predicate: T->Bool".to_string()],
            return_type: "Result<T, NotFound>".to_string(),
            examples: vec!["find([1,2,3], (x)=>x>2)  // Ok(3)".to_string()],
            category: "Search".to_string(),
            see_also: vec!["filter".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "starts_with".to_string(),
            description: "Checks if a string starts with a prefix".to_string(),
            syntax: "starts_with(text,prefix)".to_string(),
            parameters: vec!["text: String".to_string(), "prefix: String".to_string()],
            return_type: "Bool".to_string(),
            examples: vec!["starts_with(\"hello\",\"he\")  // true".to_string()],
            category: "String".to_string(),
            see_also: vec!["ends_with".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "ends_with".to_string(),
            description: "Checks if a string ends with a suffix".to_string(),
            syntax: "ends_with(text,suffix)".to_string(),
            parameters: vec!["text: String".to_string(), "suffix: String".to_string()],
            return_type: "Bool".to_string(),
            examples: vec!["ends_with(\"lang\",\"ng\")  // true".to_string()],
            category: "String".to_string(),
            see_also: vec!["starts_with".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "group_by".to_string(),
            description: "Groups list elements by key".to_string(),
            syntax: "group_by(list,key_fn)".to_string(),
            parameters: vec!["list: [V]".to_string(), "key_fn: V->K".to_string()],
            return_type: "[(K,[V])]".to_string(),
            examples: vec!["group_by([1,2,3,4], (x)=>x%2)  // [(0,[2,4]),(1,[1,3])]".to_string()],
            category: "List".to_string(),
            see_also: vec!["map".to_string()],
        });

        // === Filesystem Functions (stdlib) ===
        self.add_fs_functions();

        // === HTTP Functions (stdlib) ===
        self.add_http_functions();

        // === Math Functions (stdlib) ===
        self.add_math_functions();
        self.add_bigint_functions();
        self.add_bundled_collections_functions();

        // === Random Functions (stdlib) ===
        self.add_random_functions();

        // === Dates Functions (stdlib) ===
        self.add_dates_functions();

        // === CSV Functions (stdlib) ===
        self.add_csv_functions();

        // === JSON Functions (stdlib) ===
        self.add_json_functions();

        // === Testing Functions (stdlib) ===
        self.add_testing_functions();

        // === OS Functions (stdlib) ===
        self.add_os_functions();

        // === Crypto Functions (stdlib) ===
        self.add_crypto_functions();

        // === Base64 Functions (stdlib) ===
        self.add_base64_functions();

        // === String Functions (stdlib) ===
        self.add_string_functions();

        // === Regex Functions (stdlib) ===
        self.add_regex_functions();

        // === Collections Functions (stdlib) ===
        self.add_collections_functions();

        // === Database Functions (stdlib) ===
        self.add_db_functions();

        // === Result Type Functions ===
        self.add_result_functions();

        // === Modules the hand-written blocks above never covered ===
        // (proc, chan, time, toml, ods, stats, plot, dom, and the embedded
        // packages cli/term/ui/viz/dash), plus recent additions to os/str.
        self.add_process_and_concurrency_docs();
        self.add_time_and_encoding_docs();
        self.add_data_stack_docs();
        self.add_browser_and_toolkit_docs();
        self.add_recent_stdlib_additions();
        self.add_stdlib_coverage_gaps();
        self.add_embedded_utility_docs();
        self.add_undocumented_global_docs();

        // === meta — the program as data (the Open AST) ===
        self.add_meta_functions();

        // Build category index
        self.build_category_index();
    }

    fn add_function(&mut self, function: FunctionDoc) {
        self.functions.insert(function.name.clone(), function);
    }

    /// Compact registration for a stdlib/module function: name, one-line
    /// syntax, return type, category (the module name), and a one-line
    /// description. Parameters/examples/see-also are left empty — the
    /// syntax and description carry the signal, and this keeps hundreds of
    /// module functions documented without a 13-line literal each.
    fn doc(
        &mut self,
        name: &str,
        syntax: &str,
        return_type: &str,
        category: &str,
        description: &str,
    ) {
        self.add_function(FunctionDoc {
            name: name.to_string(),
            description: description.to_string(),
            syntax: syntax.to_string(),
            parameters: Vec::new(),
            return_type: return_type.to_string(),
            examples: Vec::new(),
            category: category.to_string(),
            see_also: Vec::new(),
        });
    }

    /// The `meta` module: the program as data (the Open AST).
    fn add_meta_functions(&mut self) {
        self.add_function(FunctionDoc {
            name: "meta.eval".to_string(),
            description: "Evaluate olang source in a fresh, pure interpreter and return the program's final value. Runs in meta mode — no filesystem, network, processes, clock, or randomness — so the result is a deterministic function of the source. This is the compute half of compile-time evaluation: a meta fn calls it on an argument's source and splices the result with meta.lit (docs/macros.md).".to_string(),
            syntax: "meta.eval(source)".to_string(),
            parameters: vec!["source: String - olang source text to evaluate".to_string()],
            return_type: "Result<value, Error>".to_string(),
            examples: vec![
                r#"unwrap(meta.eval("2 + 3"))  // 5"#.to_string(),
                r#"meta.eval("fs.read_file(...)")  // Err: not available at expansion time"#.to_string(),
            ],
            category: "Meta".to_string(),
            see_also: vec!["meta.lit".to_string(), "meta.parse".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "meta.lit".to_string(),
            description: "Render a value as olang source that evaluates back to it: strings escaped, floats with their point, containers recursive, map keys in sorted order so output is reproducible. A value with no literal form (a function, a native handle) is an error. The generation half of compile-time evaluation (docs/macros.md).".to_string(),
            syntax: "meta.lit(value)".to_string(),
            parameters: vec!["value - the value to render as source".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                r#"meta.lit([1, 2.5, "a"])  // the list as source text"#.to_string(),
                "unwrap(meta.eval(meta.lit(v))) == v  // the round trip".to_string(),
            ],
            category: "Meta".to_string(),
            see_also: vec!["meta.eval".to_string(), "show".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "meta.fresh".to_string(),
            description: "A name no program writes by hand, for macro-generated temporaries that must not collide with call-site bindings (docs/macros.md). Each call yields a distinct name built from the prefix.".to_string(),
            syntax: "meta.fresh(prefix)".to_string(),
            parameters: vec!["prefix: String - a readable stem for the generated name".to_string()],
            return_type: "String".to_string(),
            examples: vec![r#"meta.fresh("tmp")  // "tmp_m0", then "tmp_m1", ..."#.to_string()],
            category: "Meta".to_string(),
            see_also: vec!["meta.eval".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "meta.parse".to_string(),
            description: "Parse olang source into its syntax tree as ordinary olang values: a list of kind-tagged node maps. The AST shapes are a stable, documented format, so linters, codemods, and import extractors are written in olang rather than as compiler changes. Read nodes with map_get; every node carries a \"kind\" key and a \"line\" number.".to_string(),
            syntax: "meta.parse(source)".to_string(),
            parameters: vec!["source: String - olang source text to parse".to_string()],
            return_type: "Result<List<Map>, Error>".to_string(),
            examples: vec![
                "let prog = unwrap(meta.parse(\"use fmt\\nfn f() = 1\"))".to_string(),
                "prog |> filter((n) => map_get(n, \"kind\") == \"use\")  // the imports".to_string(),
                "meta.parse(\"fn (\")  // Err(parse error text)".to_string(),
            ],
            category: "Meta".to_string(),
            see_also: vec![
                "json.parse".to_string(),
                "map_get".to_string(),
                "map_has_key".to_string(),
            ],
        });
    }

    /// The globals that had no entry of their own — found when the
    /// language server started rendering everything from this registry
    /// and nineteen builtins turned out to be invisible to it (and to
    /// `:help`, which fell back to fuzzy search for them). The coverage
    /// test in `tests/help_coverage_test.rs` diffs the dispatcher's name
    /// list against this registry, so the set can never quietly grow
    /// again.
    fn add_undocumented_global_docs(&mut self) {
        for (name, syntax, ret, category, description, example) in [
            (
                "show",
                "show(value)",
                "String",
                "Strings",
                "The display rendering of a value: strings bare, everything else as to_string.                  The form for building human output — template interpolation uses the same                  rendering. to_string is the repr form, quoting strings.",
                "show(\"hi\")  // hi (to_string gives \"hi\")",
            ),
            (
                "concat",
                "concat(a, b)",
                "List",
                "Lists",
                "A new list holding every element of a followed by every element of b.",
                "concat([1, 2], [3])  // [1, 2, 3]",
            ),
            (
                "take",
                "take(list, n)",
                "List",
                "Lists",
                "The first n elements, or the whole list when it is shorter than n.",
                "take([1, 2, 3, 4], 2)  // [1, 2]",
            ),
            (
                "skip",
                "skip(list, n)",
                "List",
                "Lists",
                "The list without its first n elements; empty when n reaches past the end.",
                "skip([1, 2, 3, 4], 2)  // [3, 4]",
            ),
            (
                "entries",
                "entries(map)",
                "List",
                "Maps",
                "The (key, value) pairs of a map or struct-like value, as tuples sorted by                  key — deterministic order, made for `for (k, v) in entries(m)`.",
                "entries(#{ \"b\": 2, \"a\": 1 })  // [(\"a\", 1), (\"b\", 2)]",
            ),
            (
                "map_get",
                "map_get(map, key)",
                "Any",
                "Maps",
                "The value under key, or Unit () when the key is absent — a missing key is                  not an error. Reads maps, structs, objects, and parsed JSON uniformly.",
                "map_get(#{ \"a\": 1 }, \"a\")  // 1",
            ),
            (
                "map_get_or",
                "map_get_or(map, key, default)",
                "Any",
                "Maps",
                "The value under key, or the default when the key is absent — the \
                 one-call form of the missing-key-with-default idiom (map_get \
                 returns Unit for absence, which unwrap_or cannot take).",
                "map_get_or(#{ \"a\": 1 }, \"z\", 0)  // 0",
            ),
            (
                "map_has_key",
                "map_has_key(map, key)",
                "Bool",
                "Maps",
                "Whether key is present — the way to tell an absent key from one stored                  with a Unit value.",
                "map_has_key(#{ \"a\": 1 }, \"b\")  // false",
            ),
            (
                "map_keys",
                "map_keys(map)",
                "List",
                "Maps",
                "Every key, sorted — the same deterministic order entries uses.",
                "map_keys(#{ \"b\": 2, \"a\": 1 })  // [\"a\", \"b\"]",
            ),
            (
                "map_values",
                "map_values(map)",
                "List",
                "Maps",
                "Every value, in the sorted-key order map_keys reports.",
                "map_values(#{ \"b\": 2, \"a\": 1 })  // [1, 2]",
            ),
            (
                "map_len",
                "map_len(map)",
                "Int",
                "Maps",
                "The number of entries.",
                "map_len(#{ \"a\": 1 })  // 1",
            ),
            (
                "map_set",
                "map_set(map, key, value)",
                "Map",
                "Maps",
                "A new map with key bound to value; the original is unchanged, like every                  olang value.",
                "map_set(#{}, \"a\", 1)  // #{\"a\": 1}",
            ),
            (
                "map_remove",
                "map_remove(map, key)",
                "Map",
                "Maps",
                "A new map without key; removing an absent key returns an equal map.",
                "map_remove(#{ \"a\": 1 }, \"a\")  // #{}",
            ),
            (
                "map_merge",
                "map_merge(a, b)",
                "Map",
                "Maps",
                "A new map holding both maps' entries; where keys collide, b wins.",
                "map_merge(#{ \"a\": 1 }, #{ \"a\": 2 })  // #{\"a\": 2}",
            ),
            (
                "map_clear",
                "map_clear(map)",
                "Map",
                "Maps",
                "An empty map of the same shape — equivalent to #{} and present for                  symmetry with the other map builtins.",
                "map_clear(#{ \"a\": 1 })  // #{}",
            ),
            (
                "map_filtered",
                "map_filtered(list, predicate, function)",
                "List",
                "Higher-Order",
                "Filter then map in one pass: function applied to each element the                  predicate accepts.",
                "map_filtered([1, 2, 3, 4], (x) => x % 2 == 0, (x) => x * 10)  // [20, 40]",
            ),
            (
                "implements",
                "implements(value, trait_name)",
                "Bool",
                "Traits",
                "Whether the value's type has an impl for the named trait (given as a                  String).",
                "implements(shape, \"Area\")  // true",
            ),
            (
                "set_parallel",
                "set_parallel(enabled)",
                "Unit",
                "Concurrency",
                "Turn the parallel-capable builtins' thread pool on or off for this                  process.",
                "set_parallel(false)",
            ),
            (
                "lazy",
                "lazy(value)",
                "Any",
                "Evaluation",
                "The identity function. olang is eager — the argument is already evaluated                  when lazy receives it — so this defers nothing; it exists for source                  compatibility and force is its inverse in name only.",
                "force(lazy(5))  // 5",
            ),
            (
                "force",
                "force(value)",
                "Any",
                "Evaluation",
                "The identity function, paired with lazy. See lazy for why neither defers                  anything.",
                "force(lazy(5))  // 5",
            ),
        ] {
            self.add_function(FunctionDoc {
                name: name.to_string(),
                description: description.to_string(),
                syntax: syntax.to_string(),
                parameters: vec![],
                return_type: ret.to_string(),
                examples: vec![example.to_string()],
                category: category.to_string(),
                see_also: vec![],
            });
        }
    }

    fn add_process_and_concurrency_docs(&mut self) {
        // --- proc ---
        self.doc(
            "proc.spawn",
            "proc.spawn(program, args, opts?)",
            "Result",
            "proc",
            "start a child with piped stdin/stdout/stderr and return a live Process handle",
        );
        self.doc(
            "proc.write",
            "proc.write(p, s)",
            "Result",
            "proc",
            "write a string to the child's stdin",
        );
        self.doc(
            "proc.write_line",
            "proc.write_line(p, s)",
            "Result",
            "proc",
            "write a string plus a newline to the child's stdin",
        );
        self.doc(
            "proc.close_stdin",
            "proc.close_stdin(p)",
            "Result",
            "proc",
            "close the child's stdin, signalling end-of-input (idempotent)",
        );
        self.doc(
            "proc.read_line",
            "proc.read_line(p)",
            "Result",
            "proc",
            "the next line of the child's stdout, or Err(\"eof\") once stdout closes",
        );
        self.doc(
            "proc.read_all",
            "proc.read_all(p)",
            "Result",
            "proc",
            "the rest of the child's stdout as one string",
        );
        self.doc(
            "proc.stderr",
            "proc.stderr(p)",
            "Result",
            "proc",
            "everything the child has written to stderr (complete after exit)",
        );
        self.doc(
            "proc.wait",
            "proc.wait(p)",
            "Result",
            "proc",
            "block until the child exits, returning Ok(#{ code })",
        );
        self.doc(
            "proc.kill",
            "proc.kill(p)",
            "Result",
            "proc",
            "terminate the child immediately (SIGKILL)",
        );
        self.doc(
            "proc.pid",
            "proc.pid(p)",
            "Result",
            "proc",
            "the child's OS process id",
        );
        self.doc(
            "proc.pipeline",
            "proc.pipeline(stages, opts?)",
            "Result",
            "proc",
            "run a chain of commands wired stdout-to-stdin, the shell's a | b | c",
        );

        // --- chan ---
        // task — background threads started by `spawn`.
        self.doc(
            "task.join",
            "task.join(t)",
            "Any",
            "task",
            "block until the task finishes; its value, or Err(e) if it failed",
        );
        self.doc(
            "task.join_timeout",
            "task.join_timeout(t, ms)",
            "Result",
            "task",
            "Ok(v) if it finished within ms, else Err(\"timed out\") — the task keeps running",
        );

        // cell — the one mutable location, confined to its creating thread.
        self.doc(
            "cell.new",
            "cell(v) / cell.new(v)",
            "Cell",
            "cell",
            "make a cell holding v; the module is callable, so cell(0) is cell.new(0)",
        );
        self.doc(
            "cell.get",
            "cell.get(c)",
            "Any",
            "cell",
            "read the cell's current value",
        );
        self.doc(
            "cell.set",
            "cell.set(c, v)",
            "Unit",
            "cell",
            "replace the cell's value",
        );
        self.doc(
            "cell.update",
            "cell.update(c, f)",
            "Any",
            "cell",
            "apply f to the current value, store the result, and return it",
        );

        self.doc(
            "chan.new",
            "chan.new()",
            "Channel",
            "chan",
            "make an unbounded channel",
        );
        self.doc(
            "chan.bounded",
            "chan.bounded(n)",
            "Channel",
            "chan",
            "make a channel holding at most n in-flight messages (0 is a rendezvous channel)",
        );
        self.doc(
            "chan.send",
            "chan.send(c, v)",
            "Result",
            "chan",
            "send a value; Ok(()), or Err when the channel is closed",
        );
        self.doc(
            "chan.recv",
            "chan.recv(c)",
            "Result",
            "chan",
            "block for a message; Ok(value), or Err when closed and drained",
        );
        self.doc(
            "chan.try_recv",
            "chan.try_recv(c)",
            "Result",
            "chan",
            "Ok(value), Err(\"channel is empty\"), or Err(\"channel is closed\")",
        );
        self.doc(
            "chan.recv_timeout",
            "chan.recv_timeout(c, ms)",
            "Result",
            "chan",
            "like recv, plus Err(\"timed out\") after ms milliseconds",
        );
        self.doc(
            "chan.close",
            "chan.close(c)",
            "Unit",
            "chan",
            "close the sending side (idempotent); queued messages still drain",
        );
    }

    fn add_time_and_encoding_docs(&mut self) {
        // --- time ---
        self.doc(
            "time.now_ms",
            "time.now_ms()",
            "Int",
            "time",
            "milliseconds since the Unix epoch",
        );
        self.doc(
            "time.monotonic_ms",
            "time.monotonic_ms()",
            "Int",
            "time",
            "monotonic milliseconds (never goes backwards) — the clock for durations",
        );
        self.doc(
            "time.sleep",
            "time.sleep(ms)",
            "Unit",
            "time",
            "block for ms milliseconds",
        );

        // --- toml ---
        self.doc(
            "toml.parse",
            "toml.parse(text)",
            "Result",
            "toml",
            "parse a TOML document into a Map (tables → Maps, arrays → Lists, datetimes → strings)",
        );
        self.doc(
            "toml.stringify",
            "toml.stringify(value)",
            "Result",
            "toml",
            "render a Map (or struct-like value) as pretty TOML",
        );
        self.doc(
            "toml.validate",
            "toml.validate(text)",
            "Bool",
            "toml",
            "does the text parse as TOML?",
        );
    }

    fn add_data_stack_docs(&mut self) {
        // --- ods ---
        self.doc(
            "ods.series",
            "ods.series(list|range)",
            "Series",
            "ods",
            "build a typed, null-aware Series from a list or range",
        );
        self.doc(
            "ods.zeros",
            "ods.zeros(n)",
            "Series",
            "ods",
            "a Series of n float zeros",
        );
        self.doc(
            "ods.linspace",
            "ods.linspace(a, b, n)",
            "Series",
            "ods",
            "n evenly spaced floats from a to b, inclusive",
        );
        self.doc("ods.map", "ods.map(s, fn)", "Series", "ods", "apply a math.* unary function (named as a String, like \"sin\") over the whole column in one kernel pass");
        self.doc(
            "ods.eq",
            "ods.eq(a, b)",
            "Series",
            "ods",
            "elementwise equality mask between two Series",
        );
        self.doc(
            "ods.ne",
            "ods.ne(a, b)",
            "Series",
            "ods",
            "elementwise inequality mask between two Series",
        );
        self.doc(
            "ods.is_null",
            "ods.is_null(s)",
            "Series",
            "ods",
            "a Bool mask marking the null positions of s",
        );
        self.doc(
            "ods.fill_null",
            "ods.fill_null(s, v)",
            "Series",
            "ods",
            "replace every null in s with v",
        );
        self.doc(
            "ods.shift",
            "ods.shift(s, by)",
            "Series",
            "ods",
            "move values `by` positions down (negative moves up); vacated slots are null",
        );
        self.doc(
            "ods.cum_max",
            "ods.cum_max(s)",
            "Series",
            "ods",
            "the running maximum, alongside cumsum",
        );
        self.doc(
            "ods.cum_min",
            "ods.cum_min(s)",
            "Series",
            "ods",
            "the running minimum, alongside cumsum",
        );
        self.doc(
            "ods.rank",
            "ods.rank(s, method = \"min\")",
            "Series",
            "ods",
            "the rank of each element; method is min, max, average, ordinal, or dense",
        );
        self.doc(
            "ods.rolling",
            "ods.rolling(s, window, agg)",
            "Series",
            "ods",
            "a trailing-window aggregate; the first window-1 elements are null",
        );
        self.doc(
            "ods.unique",
            "ods.unique(s)",
            "Series",
            "ods",
            "the distinct values of s, in first-seen order (a null is a value)",
        );
        self.doc(
            "ods.n_unique",
            "ods.n_unique(s)",
            "Int",
            "ods",
            "how many distinct values s has",
        );
        self.doc(
            "ods.value_counts",
            "ods.value_counts(s)",
            "Frame",
            "ods",
            "a value/count Frame for s, most frequent first",
        );
        self.doc(
            "ods.median",
            "ods.median(s)",
            "Float",
            "ods",
            "the middle value of s, skipping nulls — quantile(s, 0.5)",
        );
        self.doc(
            "ods.cast",
            "ods.cast(s, type)",
            "Series",
            "ods",
            "s converted to \"Float\", \"Int\", \"Bool\", or \"String\"; what will not convert becomes null",
        );
        self.doc(
            "ods.sample",
            "ods.sample(f, n)",
            "Frame",
            "ods",
            "n random rows of a Frame or Series, without replacement, in original order",
        );
        self.doc(
            "ods.null_count",
            "ods.null_count(s)",
            "Int",
            "ods",
            "how many values in s are null",
        );
        self.doc(
            "ods.sum",
            "ods.sum(s)",
            "Value",
            "ods",
            "sum of s, skipping nulls",
        );
        self.doc(
            "ods.mean",
            "ods.mean(s)",
            "Float",
            "ods",
            "arithmetic mean of s, skipping nulls",
        );
        self.doc(
            "ods.var",
            "ods.var(s)",
            "Float",
            "ods",
            "sample variance of s, skipping nulls",
        );
        self.doc(
            "ods.std",
            "ods.std(s)",
            "Float",
            "ods",
            "sample standard deviation of s, skipping nulls",
        );
        self.doc(
            "ods.min",
            "ods.min(s)",
            "Value",
            "ods",
            "smallest non-null value in s",
        );
        self.doc(
            "ods.max",
            "ods.max(s)",
            "Value",
            "ods",
            "largest non-null value in s",
        );
        self.doc(
            "ods.quantile",
            "ods.quantile(s, q)",
            "Float",
            "ods",
            "the q-quantile of s (q in [0, 1]), skipping nulls",
        );
        self.doc(
            "ods.cumsum",
            "ods.cumsum(s)",
            "Series",
            "ods",
            "running cumulative sum of s",
        );
        self.doc(
            "ods.dot",
            "ods.dot(a, b)",
            "Float",
            "ods",
            "dot product of two numeric Series",
        );
        self.doc(
            "ods.sort",
            "ods.sort(s)",
            "Series",
            "ods",
            "s sorted ascending, nulls last",
        );
        self.doc(
            "ods.argsort",
            "ods.argsort(s)",
            "Series",
            "ods",
            "the indices that would sort s",
        );
        self.doc(
            "ods.take",
            "ods.take(s, idx)",
            "Series",
            "ods",
            "gather elements (or Frame rows) at the integer indices in idx",
        );
        self.doc(
            "ods.get",
            "ods.get(s, i)",
            "Value",
            "ods",
            "the element at index i (negative counts from the end)",
        );
        self.doc(
            "ods.to_list",
            "ods.to_list(s)",
            "List",
            "ods",
            "s as a language list, with nulls turned into ()",
        );
        self.doc(
            "ods.len",
            "ods.len(s)",
            "Int",
            "ods",
            "number of elements in s",
        );
        self.doc(
            "ods.frame",
            "ods.frame(columns)",
            "Frame",
            "ods",
            "build a Frame from [name, values] column pairs",
        );
        self.doc(
            "ods.frame_from_records",
            "ods.frame_from_records(records)",
            "Frame",
            "ods",
            "build a Frame from a list of record maps",
        );
        self.doc(
            "ods.read_csv",
            "ods.read_csv(text)",
            "Frame",
            "ods",
            "parse CSV text into a Frame, inferring column types (a parse failure raises)",
        );
        self.doc(
            "ods.read_csv_file",
            "ods.read_csv_file(path)",
            "Result<Frame, Error>",
            "ods",
            "read a CSV file into a Frame, inferring column types. Requires the fs capability at read level; an unreadable file or malformed CSV is an Err",
        );
        self.doc(
            "ods.open_csv",
            "ods.open_csv(path)",
            "Result<Reader, Error>",
            "ods",
            "open a CSV file for streaming: the reader holds its position, so a file larger than memory is read one chunk at a time. Requires the fs capability at read level; the reader is confined to the thread that opened it",
        );
        self.doc(
            "ods.next_chunk",
            "ods.next_chunk(reader, rows)",
            "Result<Frame, Error>",
            "ods",
            "pull up to `rows` more rows as a Frame; the Frame is empty when the file is exhausted, which is how a streaming loop ends. Works on any reader, from open_csv or open_jsonl alike",
        );
        self.doc(
            "ods.rows_read",
            "ods.rows_read(reader)",
            "Int",
            "ods",
            "how many rows this reader has handed out so far",
        );
        self.doc(
            "ods.at_end",
            "ods.at_end(reader)",
            "Bool",
            "ods",
            "whether the reader has reached the end of its file",
        );
        self.doc(
            "caps.allowed",
            "caps.allowed(name)",
            "Bool",
            "caps",
            "whether the calling code holds a capability (\"fs\", \"net\", \"proc\", \"db\", \"env\"). Answers for the *caller*: attenuated dependency code sees its own grant. Use it to choose a path before attempting a call — a denial still stops the program",
        );
        self.doc(
            "caps.level",
            "caps.level(name)",
            "String",
            "caps",
            "the granted level of a capability: \"none\", \"read\", or \"full\". Only `fs` has a middle level; every other capability answers \"none\" or \"full\"",
        );
        self.doc(
            "caps.granted",
            "caps.granted()",
            "Map",
            "caps",
            "the calling code's whole grant as a map — `fs` as a level string, the rest as Bool. For reporting a grant rather than branching on it",
        );
        self.doc(
            "ods.all_of",
            "ods.all_of(masks)",
            "Series",
            "ods",
            "combine Bool masks with AND, elementwise. Takes a list because a real filter has several conditions; `&&` cannot serve, since the language compiles it to a short-circuiting jump. Three-valued: one false settles the result even if another entry is null",
        );
        self.doc(
            "ods.any_of",
            "ods.any_of(masks)",
            "Series",
            "ods",
            "combine Bool masks with OR, elementwise. Three-valued: one true settles the result even if another entry is null",
        );
        self.doc(
            "ods.not",
            "ods.not(mask)",
            "Series",
            "ods",
            "invert a Bool mask elementwise; a null stays null",
        );
        self.doc(
            "ods.concat",
            "ods.concat(frames)",
            "Frame",
            "ods",
            "stack Frames vertically, matching columns by name. A missing or extra column is refused rather than padded with nulls; an Int column meeting a Float one widens. This is how partial results from a streaming loop are put back together",
        );
        self.doc(
            "ods.write_frame",
            "ods.write_frame(f, path)",
            "Result<Unit, Error>",
            "ods",
            "write a Frame in olang's native columnar format: types survive exactly, the load is a read rather than a parse, and one column can be fetched without the others. Requires the fs capability at write level",
        );
        self.doc(
            "ods.read_frame",
            "ods.read_frame(path, columns = all)",
            "Result<Frame, Error>",
            "ods",
            "read a native columnar file. Pass a list of column names to decode only those, in that order — the rest are skipped by the byte lengths in the header. Requires the fs capability at read level",
        );
        self.doc(
            "ods.frame_info",
            "ods.frame_info(path)",
            "Result<Frame, Error>",
            "ods",
            "the schema of a native columnar file — column, dtype, nulls, bytes — read from its text header without loading the data. Requires the fs capability at read level",
        );
        self.doc(
            "ods.describe",
            "ods.describe(f)",
            "Frame",
            "ods",
            "summary statistics per column — count, nulls, mean, std, min, q25, median, q75, max — as a Frame, so it prints as a table and can be sorted or written out. Numeric statistics are null for String and Bool columns",
        );
        self.doc(
            "ods.schema",
            "ods.schema(f)",
            "Frame",
            "ods",
            "name, type, and null count per column: describe without the arithmetic, for a Frame too wide to summarize",
        );
        self.doc(
            "ods.open_jsonl",
            "ods.open_jsonl(path)",
            "Result<Reader, Error>",
            "ods",
            "open a JSON-lines file for streaming, driven by the same next_chunk/rows_read/at_end verbs as open_csv. Requires the fs capability at read level; the reader is confined to the thread that opened it",
        );
        self.doc(
            "ods.read_jsonl",
            "ods.read_jsonl(text)",
            "Result<Frame, Error>",
            "ods",
            "parse JSON-lines text (one JSON object per line) into a Frame; columns are the union of the keys and a missing key is a null. Blank lines are skipped; a malformed or non-object line is an Err naming the line number",
        );
        self.doc(
            "ods.read_jsonl_file",
            "ods.read_jsonl_file(path)",
            "Result<Frame, Error>",
            "ods",
            "read a JSON-lines file into a Frame. Requires the fs capability at read level",
        );
        self.doc(
            "ods.to_jsonl",
            "ods.to_jsonl(f)",
            "String",
            "ods",
            "serialize a Frame as JSON-lines text, one object per row; nulls are omitted rather than written, so it round-trips through read_jsonl",
        );
        self.doc(
            "ods.write_jsonl",
            "ods.write_jsonl(f, path)",
            "Result<Unit, Error>",
            "ods",
            "write a Frame to a JSON-lines file. Requires the fs capability at write level",
        );
        self.doc(
            "ods.to_csv",
            "ods.to_csv(f)",
            "String",
            "ods",
            "serialize a Frame as CSV text with a header row; nulls become empty cells, so it round-trips through read_csv",
        );
        self.doc(
            "ods.write_csv",
            "ods.write_csv(f, path)",
            "Result<Unit, Error>",
            "ods",
            "write a Frame to a CSV file. Requires the fs capability at write level",
        );
        self.doc(
            "ods.columns",
            "ods.columns(f)",
            "List",
            "ods",
            "the column names of f",
        );
        self.doc(
            "ods.column",
            "ods.column(f, name)  //  or f[name]",
            "Series",
            "ods",
            "the named column of f as a Series",
        );
        self.doc(
            "ods.n_rows",
            "ods.n_rows(f)",
            "Int",
            "ods",
            "number of rows in f",
        );
        self.doc(
            "ods.n_cols",
            "ods.n_cols(f)",
            "Int",
            "ods",
            "number of columns in f",
        );
        self.doc(
            "ods.head",
            "ods.head(f, n = 10)",
            "Frame",
            "ods",
            "the first n rows of f (n defaults to 10)",
        );
        self.doc(
            "ods.tail",
            "ods.tail(f, n = 10)",
            "Frame",
            "ods",
            "the last n rows of f (n defaults to 10)",
        );
        self.doc(
            "ods.rename",
            "ods.rename(f, mapping)",
            "Frame",
            "ods",
            "f with columns renamed, given a Map of old name to new name",
        );
        self.doc(
            "ods.drop",
            "ods.drop(f, names)",
            "Frame",
            "ods",
            "a Frame without the named columns (the complement of select)",
        );
        self.doc(
            "ods.distinct",
            "ods.distinct(f, names = every column)",
            "Frame",
            "ods",
            "f with duplicate rows removed, keeping the first occurrence",
        );
        self.doc(
            "ods.drop_null",
            "ods.drop_null(f, names = every column)",
            "Frame",
            "ods",
            "f without the rows that are null in any of the named columns",
        );
        self.doc(
            "ods.to_records",
            "ods.to_records(f)",
            "List",
            "ods",
            "f as a list of per-row record maps",
        );
        self.doc(
            "ods.select",
            "ods.select(f, names)",
            "Frame",
            "ods",
            "a Frame keeping only the named columns",
        );
        self.doc(
            "ods.with_column",
            "ods.with_column(f, name, series)",
            "Frame",
            "ods",
            "f with a column added or replaced",
        );
        self.doc(
            "ods.filter",
            "ods.filter(f, mask)",
            "Frame",
            "ods",
            "keep the rows (or Series elements) where the Bool mask is true",
        );
        self.doc(
            "ods.sort_by",
            "ods.sort_by(f, name, descending)",
            "Frame",
            "ods",
            "f sorted by the named column, descending when true",
        );
        self.doc(
            "ods.group_by",
            "ods.group_by(f, key, aggs)",
            "Frame",
            "ods",
            "group rows by key and reduce each group with the given aggregations",
        );
        self.doc(
            "ods.join",
            "ods.join(a, b, on_a, on_b = on_a)",
            "Frame",
            "ods",
            "inner hash join of a and b on one key column from each side",
        );
        self.doc(
            "ods.pivot",
            "ods.pivot(f, index, columns, values, agg)",
            "Frame",
            "ods",
            "long to wide: a row per index value, a column per distinct `columns` value, cells aggregated by agg",
        );
        self.doc(
            "ods.unpivot",
            "ods.unpivot(f, id_columns, value_columns = the rest)",
            "Frame",
            "ods",
            "wide to long: keep the id columns, turn the rest into name/value rows",
        );
        self.doc(
            "ods.join_full",
            "ods.join_full(a, b, on, on_b = on)",
            "Frame",
            "ods",
            "every row from both sides; the key column takes whichever side has it",
        );
        self.doc(
            "ods.join_semi",
            "ods.join_semi(a, b, on, on_b = on)",
            "Frame",
            "ods",
            "the rows of a that have a match in b, once each, a's columns only",
        );
        self.doc(
            "ods.join_anti",
            "ods.join_anti(a, b, on, on_b = on)",
            "Frame",
            "ods",
            "the rows of a that have no match in b, a's columns only",
        );
        self.doc(
            "ods.join_left",
            "ods.join_left(a, b, on_a, on_b)",
            "Frame",
            "ods",
            "left join keeping every row of a, filling unmatched right columns with nulls",
        );

        // --- stats ---
        self.doc(
            "stats.describe",
            "stats.describe(s)",
            "Map",
            "stats",
            "count, null count, mean, std, min, quartiles, and max of s in one map",
        );
        self.doc(
            "stats.corr",
            "stats.corr(a, b)",
            "Float",
            "stats",
            "Pearson correlation of a and b over pairwise-complete rows",
        );
        self.doc(
            "stats.cov",
            "stats.cov(a, b)",
            "Float",
            "stats",
            "sample covariance of a and b over pairwise-complete rows",
        );
        self.doc("stats.t_test", "stats.t_test(a, b)", "Map", "stats", "t-test: Welch's two-sample when b is a Series, one-sample against mean b when b is a number");
        self.doc(
            "stats.chi2_test",
            "stats.chi2_test(obs, exp)",
            "Map",
            "stats",
            "chi-square goodness-of-fit test of observed against expected counts",
        );
        self.doc("stats.lm", "stats.lm(y, x)", "Map", "stats", "OLS regression of y on regressor(s) x, returning coefficients, standard errors, t-stats, p-values, and r2");
        self.doc(
            "stats.norm.pdf",
            "stats.norm.pdf(x, mean, std)",
            "Float",
            "stats",
            "normal probability density at x",
        );
        self.doc(
            "stats.norm.cdf",
            "stats.norm.cdf(x, mean, std)",
            "Float",
            "stats",
            "normal cumulative probability at x",
        );
        self.doc(
            "stats.norm.ppf",
            "stats.norm.ppf(p, mean, std)",
            "Float",
            "stats",
            "normal quantile (inverse CDF) for probability p",
        );
        self.doc(
            "stats.norm.sample",
            "stats.norm.sample(n, mean, std)",
            "Series",
            "stats",
            "n normal draws from the random module's seeded stream",
        );
        self.doc(
            "stats.t.pdf",
            "stats.t.pdf(x, df)",
            "Float",
            "stats",
            "Student's t probability density at x",
        );
        self.doc(
            "stats.t.cdf",
            "stats.t.cdf(x, df)",
            "Float",
            "stats",
            "Student's t cumulative probability at x",
        );
        self.doc(
            "stats.t.ppf",
            "stats.t.ppf(p, df)",
            "Float",
            "stats",
            "Student's t quantile (inverse CDF) for probability p",
        );
        self.doc(
            "stats.t.sample",
            "stats.t.sample(n, df)",
            "Series",
            "stats",
            "n Student's t draws from the seeded stream",
        );
        self.doc(
            "stats.chi2.pdf",
            "stats.chi2.pdf(x, df)",
            "Float",
            "stats",
            "chi-square probability density at x",
        );
        self.doc(
            "stats.chi2.cdf",
            "stats.chi2.cdf(x, df)",
            "Float",
            "stats",
            "chi-square cumulative probability at x",
        );
        self.doc(
            "stats.chi2.ppf",
            "stats.chi2.ppf(p, df)",
            "Float",
            "stats",
            "chi-square quantile (inverse CDF) for probability p",
        );
        self.doc(
            "stats.chi2.sample",
            "stats.chi2.sample(n, df)",
            "Series",
            "stats",
            "n chi-square draws from the seeded stream",
        );
        self.doc(
            "stats.f.pdf",
            "stats.f.pdf(x, d1, d2)",
            "Float",
            "stats",
            "F-distribution probability density at x",
        );
        self.doc(
            "stats.f.cdf",
            "stats.f.cdf(x, d1, d2)",
            "Float",
            "stats",
            "F-distribution cumulative probability at x",
        );
        self.doc(
            "stats.f.ppf",
            "stats.f.ppf(p, d1, d2)",
            "Float",
            "stats",
            "F-distribution quantile (inverse CDF) for probability p",
        );
        self.doc(
            "stats.f.sample",
            "stats.f.sample(n, d1, d2)",
            "Series",
            "stats",
            "n F-distribution draws from the seeded stream",
        );

        // --- plot ---
        self.doc(
            "plot.line",
            "plot.line(x, y, opts)",
            "String",
            "plot",
            "a line chart of y against x, as a standalone SVG string",
        );
        self.doc(
            "plot.scatter",
            "plot.scatter(x, y, opts)",
            "String",
            "plot",
            "a scatter plot of y against x, as an SVG string",
        );
        self.doc(
            "plot.area",
            "plot.area(x, y, opts)",
            "String",
            "plot",
            "an area chart of y against x, as an SVG string",
        );
        self.doc(
            "plot.lines",
            "plot.lines(x, series, opts)",
            "String",
            "plot",
            "a multi-series line chart with legend from [label, y] pairs sharing x",
        );
        self.doc(
            "plot.xy",
            "plot.xy(layers, opts)",
            "String",
            "plot",
            "layered marks over shared scales from [label, mark, x, y] entries",
        );
        self.doc(
            "plot.bar",
            "plot.bar(labels, values, opts)",
            "String",
            "plot",
            "a bar chart of values against labels, as an SVG string",
        );
        self.doc(
            "plot.bars",
            "plot.bars(labels, series, opts)",
            "String",
            "plot",
            "a grouped bar chart from [label, values] series sharing the category labels",
        );
        self.doc(
            "plot.stacked",
            "plot.stacked(labels, series, opts)",
            "String",
            "plot",
            "a stacked bar chart from [label, values] series sharing the category labels",
        );
        self.doc(
            "plot.hist",
            "plot.hist(s, bins, opts)",
            "String",
            "plot",
            "a histogram of s over the given number of bins, as an SVG string",
        );
        self.doc(
            "plot.heatmap",
            "plot.heatmap(x_labels, y_labels, rows, opts)",
            "String",
            "plot",
            "a heatmap of the row-major value grid, as an SVG string",
        );
        self.doc(
            "plot.box",
            "plot.box(series, opts)",
            "String",
            "plot",
            "box-and-whisker plots from [label, values] series, nulls dropped",
        );
        self.doc("plot.ramp", "plot.ramp(scale, t)", "String", "plot", "one color at position t in [0, 1] from a named ramp (auto, ocean, ember, thermal, diverging)");
    }

    fn add_browser_and_toolkit_docs(&mut self) {
        // --- dom ---
        self.doc(
            "dom.query",
            "dom.query(sel)",
            "Node",
            "dom",
            "First element matching a CSS selector — an error if none matches.",
        );
        self.doc(
            "dom.set_text",
            "dom.set_text(el, s)",
            "Unit",
            "dom",
            "Write an element's text content.",
        );
        self.doc(
            "dom.get_text",
            "dom.get_text(el)",
            "String",
            "dom",
            "Read an element's text content.",
        );
        self.doc(
            "dom.set_html",
            "dom.set_html(el, html)",
            "Unit",
            "dom",
            "Replace an element's inner HTML — the render primitive.",
        );
        self.doc(
            "dom.value",
            "dom.value(el)",
            "String",
            "dom",
            "Read a form control's value.",
        );
        self.doc(
            "dom.set_value",
            "dom.set_value(el, s)",
            "Unit",
            "dom",
            "Write a form control's value.",
        );
        self.doc(
            "dom.on",
            "dom.on(el, event, handler)",
            "Unit",
            "dom",
            "Attach an event handler; the handler receives a structured event Map.",
        );
        self.doc(
            "dom.fetch",
            "dom.fetch(method, path, body, callback)",
            "Unit",
            "dom",
            "Asynchronous HTTP from the page — the callback receives the response text.",
        );
        self.doc(
            "dom.focus",
            "dom.focus(el)",
            "Unit",
            "dom",
            "Focus an element.",
        );
        self.doc(
            "dom.set_class",
            "dom.set_class(el, c)",
            "Unit",
            "dom",
            "Replace an element's class list wholesale.",
        );
        self.doc(
            "dom.get_attr",
            "dom.get_attr(el, name)",
            "String",
            "dom",
            "Read an element's attribute value.",
        );
        self.doc(
            "dom.set_attr",
            "dom.set_attr(el, name, v)",
            "Unit",
            "dom",
            "Set an element's attribute.",
        );
        self.doc(
            "dom.remove_attr",
            "dom.remove_attr(el, name)",
            "Unit",
            "dom",
            "Remove an element's attribute.",
        );
        self.doc(
            "dom.class_add",
            "dom.class_add(el, c)",
            "Unit",
            "dom",
            "Add a class to an element's class list.",
        );
        self.doc(
            "dom.class_remove",
            "dom.class_remove(el, c)",
            "Unit",
            "dom",
            "Remove a class from an element's class list.",
        );
        self.doc(
            "dom.class_toggle",
            "dom.class_toggle(el, c)",
            "Unit",
            "dom",
            "Toggle a class on an element's class list.",
        );
        self.doc(
            "dom.set_style",
            "dom.set_style(el, prop, v)",
            "Unit",
            "dom",
            "Set one CSS style property on an element.",
        );
        self.doc(
            "dom.measure",
            "dom.measure(el)",
            "Map",
            "dom",
            "Bounding rect as a Map: x, y, width, height.",
        );
        self.doc(
            "dom.create",
            "dom.create(tag)",
            "Node",
            "dom",
            "Create a detached element of the given tag.",
        );
        self.doc(
            "dom.append",
            "dom.append(parent, child)",
            "Unit",
            "dom",
            "Append a child element to a parent.",
        );
        self.doc(
            "dom.remove",
            "dom.remove(el)",
            "Unit",
            "dom",
            "Remove an element from the document.",
        );
        self.doc(
            "dom.scroll_into_view",
            "dom.scroll_into_view(el)",
            "Unit",
            "dom",
            "Scroll an element into view.",
        );
        self.doc(
            "dom.set_timeout",
            "dom.set_timeout(ms, fn)",
            "Unit",
            "dom",
            "Run a function once after a delay in milliseconds.",
        );
        self.doc(
            "dom.set_interval",
            "dom.set_interval(ms, fn)",
            "Timer",
            "dom",
            "Run a function repeatedly every ms milliseconds; returns a timer handle.",
        );
        self.doc(
            "dom.clear_interval",
            "dom.clear_interval(t)",
            "Unit",
            "dom",
            "Cancel an interval timer by its handle.",
        );
        self.doc(
            "dom.request_frame",
            "dom.request_frame(fn)",
            "Unit",
            "dom",
            "Schedule one animation frame; re-arm inside the handler for a loop.",
        );
        self.doc(
            "dom.on_frame",
            "dom.on_frame(fn)",
            "Unit",
            "dom",
            "Register the persistent animation loop, called every frame with a millisecond delta.",
        );
        self.doc(
            "dom.draw",
            "dom.draw(canvas, ops)",
            "Unit",
            "dom",
            "Replay a draw-list onto a canvas — the whole scene crosses the boundary once.",
        );
        self.doc("dom.draw_points", "dom.draw_points(canvas, xs, ys, style)", "Unit", "dom", "Bulk point/path plotting; coordinates cross as one packed binary buffer with a host-side affine.");
        self.doc(
            "dom.insert_before",
            "dom.insert_before(parent, child, before)",
            "Unit",
            "dom",
            "Position a child before another node (0 appends).",
        );
        self.doc(
            "dom.push_state",
            "dom.push_state(path)",
            "Unit",
            "dom",
            "Push an SPA navigation entry for the given path.",
        );
        self.doc(
            "dom.location",
            "dom.location()",
            "Map",
            "dom",
            "The current location as a Map of path and query.",
        );
        self.doc(
            "dom.on_route",
            "dom.on_route(fn)",
            "Unit",
            "dom",
            "Register the back/forward listener — a route event Map with path and query.",
        );
        self.doc(
            "dom.storage_get",
            "dom.storage_get(k)",
            "String",
            "dom",
            "Read a localStorage value (missing keys read as \"\").",
        );
        self.doc(
            "dom.storage_set",
            "dom.storage_set(k, v)",
            "Unit",
            "dom",
            "Write a localStorage value.",
        );
        self.doc(
            "dom.state_get",
            "dom.state_get(k)",
            "Value",
            "dom",
            "Read from the page-lifetime session state store (missing keys read as Unit).",
        );
        self.doc(
            "dom.state_set",
            "dom.state_set(k, v)",
            "Unit",
            "dom",
            "Write to the page-lifetime session state store (a Map/list round-trips).",
        );
        self.doc(
            "dom.storage_remove",
            "dom.storage_remove(k)",
            "Unit",
            "dom",
            "Remove a localStorage key.",
        );
        self.doc(
            "dom.worker",
            "dom.worker(path)",
            "Worker",
            "dom",
            "Boot a second olang program in a Web Worker; returns a worker handle.",
        );
        self.doc(
            "dom.worker_send",
            "dom.worker_send(w, value)",
            "Unit",
            "dom",
            "Send a value to a worker.",
        );
        self.doc(
            "dom.worker_on",
            "dom.worker_on(w, handler)",
            "Unit",
            "dom",
            "Receive values from a worker.",
        );
        self.doc(
            "dom.worker_close",
            "dom.worker_close(w)",
            "Unit",
            "dom",
            "Terminate a worker.",
        );
        self.doc(
            "dom.post",
            "dom.post(value)",
            "Unit",
            "dom",
            "Worker-side mirror: post a value to the page.",
        );
        self.doc(
            "dom.on_message",
            "dom.on_message(handler)",
            "Unit",
            "dom",
            "Worker-side mirror: receive values from the page.",
        );
        self.doc(
            "dom.fetch_json",
            "dom.fetch_json(method, path, body, callback)",
            "Unit",
            "dom",
            "Like dom.fetch, but the callback receives the parsed value directly.",
        );

        // --- cli ---
        self.doc(
            "cli.args",
            "cli.args()",
            "List",
            "cli",
            "The program's own arguments, with the program path (argv[0]) dropped.",
        );
        self.doc(
            "cli.parse",
            "cli.parse(spec, argv)",
            "Result",
            "cli",
            "Parse argv against spec; Ok(values) (a map incl. help) or Err(message).",
        );
        self.doc(
            "cli.help",
            "cli.help(spec)",
            "String",
            "cli",
            "Render the usage/help text for spec as a string.",
        );

        // --- term ---
        self.doc(
            "term.color",
            "term.color()",
            "Bool",
            "term",
            "Whether styled output should be emitted right now.",
        );
        self.doc(
            "term.black",
            "term.black(s)",
            "String",
            "term",
            "Wrap s in black.",
        );
        self.doc(
            "term.red",
            "term.red(s)",
            "String",
            "term",
            "Wrap s in red.",
        );
        self.doc(
            "term.green",
            "term.green(s)",
            "String",
            "term",
            "Wrap s in green.",
        );
        self.doc(
            "term.yellow",
            "term.yellow(s)",
            "String",
            "term",
            "Wrap s in yellow.",
        );
        self.doc(
            "term.blue",
            "term.blue(s)",
            "String",
            "term",
            "Wrap s in blue.",
        );
        self.doc(
            "term.magenta",
            "term.magenta(s)",
            "String",
            "term",
            "Wrap s in magenta.",
        );
        self.doc(
            "term.cyan",
            "term.cyan(s)",
            "String",
            "term",
            "Wrap s in cyan.",
        );
        self.doc(
            "term.white",
            "term.white(s)",
            "String",
            "term",
            "Wrap s in white.",
        );
        self.doc(
            "term.gray",
            "term.gray(s)",
            "String",
            "term",
            "Wrap s in gray.",
        );
        self.doc("term.bold", "term.bold(s)", "String", "term", "Bold s.");
        self.doc("term.dim", "term.dim(s)", "String", "term", "Dim s.");
        self.doc(
            "term.italic",
            "term.italic(s)",
            "String",
            "term",
            "Italicize s.",
        );
        self.doc(
            "term.underline",
            "term.underline(s)",
            "String",
            "term",
            "Underline s.",
        );
        self.doc(
            "term.style",
            "term.style(s, opts)",
            "String",
            "term",
            "Style s with opts: fg, bg, bold, dim, italic, underline.",
        );
        self.doc(
            "term.rule",
            "term.rule(width)",
            "String",
            "term",
            "A horizontal rule of box-drawing dashes.",
        );
        self.doc(
            "term.visible_len",
            "term.visible_len(s)",
            "Int",
            "term",
            "The visible screen width of s, with ANSI styling escapes discounted.",
        );
        self.doc(
            "term.table",
            "term.table(headers, rows)",
            "String",
            "term",
            "An aligned table; columns pad to their widest cell by visible width.",
        );
        self.doc(
            "term.bar",
            "term.bar(fraction, width)",
            "String",
            "term",
            "A progress bar for a fraction in [0, 1].",
        );
        self.doc(
            "term.prompt",
            "term.prompt(question)",
            "String",
            "term",
            "Prompt for a line of input.",
        );
        self.doc(
            "term.confirm",
            "term.confirm(question)",
            "Bool",
            "term",
            "Yes/no question; anything starting with y/Y is true, else false.",
        );
        self.doc(
            "term.select",
            "term.select(question, options)",
            "Result",
            "term",
            "A numbered menu; returns Ok(chosen option string) or Err on a bad choice.",
        );

        // --- ui ---
        self.doc("ui.h", "ui.h(tag, attrs, children)", "Node", "ui", "Build a virtual node — a tag, an attribute map, and a list of child nodes or text strings.");
        self.doc(
            "ui.hk",
            "ui.hk(key, tag, attrs, children)",
            "Node",
            "ui",
            "Like h, plus a stable reconciliation key for keyed lists.",
        );
        self.doc(
            "ui.esc",
            "ui.esc(s)",
            "String",
            "ui",
            "HTML-escape a string (&amp; &lt; &gt; \" to entities).",
        );
        self.doc(
            "ui.html",
            "ui.html(node)",
            "String",
            "ui",
            "Render a node tree to an HTML string — pure, testable without a browser.",
        );
        self.doc(
            "ui.render",
            "ui.render(el, children)",
            "Unit",
            "ui",
            "Mount and reconcile a keyed child list into a live DOM element (browser only).",
        );

        // --- viz ---
        self.doc(
            "viz.chart",
            "viz.chart(spec)",
            "String",
            "viz",
            "Compile a chart spec to plot SVG — pure, testable anywhere.",
        );
        self.doc("viz.draw", "viz.draw(el, spec)", "Unit", "viz", "Compile the same xy specs to a canvas draw-list, for data too big to render as SVG nodes.");
        self.doc("viz.tooltip", "viz.tooltip(el)", "Unit", "viz", "Attach a hover tooltip to a chart container: hovering an interactive mark shows its datum.");
        self.doc("viz.on_mark", "viz.on_mark(el, event, handler)", "Unit", "viz", "Delegated mark events: the handler fires only for an interactive mark and receives its data map.");
        self.doc("viz.brush", "viz.brush(el, handler)", "Unit", "viz", "Horizontal brush; the handler receives #{ from, to } as fractions of the element's width.");

        // --- dash ---
        self.doc(
            "dash.kpi",
            "dash.kpi(label, value, note)",
            "String",
            "dash",
            "A KPI tile: the number big, the label above, a note below (\"\" omits it).",
        );
        self.doc(
            "dash.stat",
            "dash.stat(label, value, note, accent)",
            "String",
            "dash",
            "A KPI tile with its own accent color for the value.",
        );
        self.doc(
            "dash.card",
            "dash.card(title, inner)",
            "String",
            "dash",
            "A card: a titled panel around arbitrary (unescaped) inner HTML.",
        );
        self.doc(
            "dash.half",
            "dash.half(title, inner)",
            "String",
            "dash",
            "A card that spans two grid columns — the chart-friendly width.",
        );
        self.doc(
            "dash.wide",
            "dash.wide(title, inner)",
            "String",
            "dash",
            "A card that spans the full grid width.",
        );
        self.doc(
            "dash.grid",
            "dash.grid(cards, columns)",
            "String",
            "dash",
            "The grid: cards flow into columns columns; dash-wide cards break out to full width.",
        );
        self.doc(
            "dash.styles",
            "dash.styles()",
            "String",
            "dash",
            "The kit's stylesheet — prepend once to the mount's HTML and the classes just work.",
        );
    }

    fn add_recent_stdlib_additions(&mut self) {
        // os — input, terminal, and signals (previously undocumented)
        self.doc(
            "os.stdin",
            "os.stdin()",
            "Result",
            "os",
            "read all of standard input to end-of-file as one string",
        );
        self.doc(
            "os.stdin_lines",
            "os.stdin_lines()",
            "Result",
            "os",
            "all of standard input as a list of lines, endings stripped",
        );
        self.doc(
            "os.read_line",
            "os.read_line()",
            "Result",
            "os",
            "one line from stdin as Ok(line), or Err(\"eof\") at end-of-input",
        );
        self.doc("os.exec", "os.exec(program, args, opts?)", "Result", "os", "run a program to completion, returning Ok(#{ code, stdout, stderr }); opts sets cwd/stdin/env");
        self.doc(
            "os.is_tty",
            "os.is_tty()",
            "Bool",
            "os",
            "whether standard output is a terminal (Ok(bool))",
        );
        self.doc("os.flush", "os.flush()", "Unit", "os", "flush buffered standard output — needed to show a progress bar drawn with a leading carriage return");
        self.doc(
            "os.on_interrupt",
            "os.on_interrupt()",
            "Result",
            "os",
            "trap Ctrl-C (SIGINT) so it sets a flag instead of terminating — for graceful shutdown",
        );
        self.doc(
            "os.interrupted",
            "os.interrupted()",
            "Bool",
            "os",
            "whether Ctrl-C has been pressed since on_interrupt/reset_interrupt (Ok(bool))",
        );
        self.doc(
            "os.reset_interrupt",
            "os.reset_interrupt()",
            "Unit",
            "os",
            "clear the interrupt flag, arming for the next Ctrl-C",
        );

        // str — the format helper
        self.doc("str.fmt", "str.fmt(template, ...)", "String", "str", "fill {} placeholders in a template with the display form of each argument ({{ and }} escape; a placeholder/argument count mismatch raises)");
    }

    fn add_stdlib_coverage_gaps(&mut self) {
        // --- math: the trig/hyperbolic/exp/log family and integer helpers ---
        // (math.sin already has a rich entry with examples; don't overwrite it.)
        self.doc(
            "math.tan",
            "math.tan(radians)",
            "Float",
            "math",
            "tangent of an angle in radians",
        );
        self.doc(
            "math.asin",
            "math.asin(x)",
            "Float",
            "math",
            "arc sine of x, result in radians",
        );
        self.doc(
            "math.acos",
            "math.acos(x)",
            "Float",
            "math",
            "arc cosine of x, result in radians",
        );
        self.doc(
            "math.atan",
            "math.atan(x)",
            "Float",
            "math",
            "arc tangent of x, result in radians",
        );
        self.doc(
            "math.atan2",
            "math.atan2(y, x)",
            "Float",
            "math",
            "angle of the point (x, y) from the positive x-axis, in radians (quadrant-correct)",
        );
        self.doc(
            "math.sinh",
            "math.sinh(x)",
            "Float",
            "math",
            "hyperbolic sine of x",
        );
        self.doc(
            "math.cosh",
            "math.cosh(x)",
            "Float",
            "math",
            "hyperbolic cosine of x",
        );
        self.doc(
            "math.tanh",
            "math.tanh(x)",
            "Float",
            "math",
            "hyperbolic tangent of x",
        );
        self.doc(
            "math.exp",
            "math.exp(x)",
            "Float",
            "math",
            "e raised to the power x",
        );
        self.doc(
            "math.exp2",
            "math.exp2(x)",
            "Float",
            "math",
            "2 raised to the power x",
        );
        self.doc(
            "math.log",
            "math.log(x, base)",
            "Float",
            "math",
            "logarithm of x in the given base",
        );
        self.doc(
            "math.log2",
            "math.log2(x)",
            "Float",
            "math",
            "base-2 logarithm of x",
        );
        self.doc(
            "math.cbrt",
            "math.cbrt(x)",
            "Float",
            "math",
            "cube root of x",
        );
        self.doc(
            "math.trunc",
            "math.trunc(x)",
            "Float",
            "math",
            "x with its fractional part removed (rounded toward zero)",
        );
        self.doc(
            "math.fract",
            "math.fract(x)",
            "Float",
            "math",
            "the fractional part of x (x minus its truncation)",
        );
        self.doc(
            "math.sign",
            "math.sign(x)",
            "Int",
            "math",
            "-1, 0, or 1 according to the sign of x",
        );
        self.doc(
            "math.gcd",
            "math.gcd(a, b)",
            "Int",
            "math",
            "greatest common divisor of two integers",
        );
        self.doc(
            "math.lcm",
            "math.lcm(a, b)",
            "Int",
            "math",
            "least common multiple of two integers",
        );

        // --- fs: path helpers and directory walking ---
        self.doc(
            "fs.join",
            "fs.join(segments)",
            "String",
            "fs",
            "join a list of path segments with the platform separator — pure, no disk access",
        );
        self.doc(
            "fs.basename",
            "fs.basename(path)",
            "String",
            "fs",
            "the final component of a path",
        );
        self.doc(
            "fs.dirname",
            "fs.dirname(path)",
            "String",
            "fs",
            "the directory portion of a path",
        );
        self.doc(
            "fs.ext",
            "fs.ext(path)",
            "String",
            "fs",
            "the file extension of a path, without the dot",
        );
        self.doc(
            "fs.abs_path",
            "fs.abs_path(path)",
            "Result",
            "fs",
            "resolve a path to an absolute path against the current directory",
        );
        self.doc(
            "fs.walk",
            "fs.walk(dir)",
            "Result",
            "fs",
            "every file below a directory, recursively (Ok(list of paths))",
        );
        self.doc(
            "fs.glob",
            "fs.glob(pattern)",
            "Result",
            "fs",
            "paths matching a glob where * matches within a segment and ** across segments",
        );

        // --- csv: the mutating builder verbs and JSON bridge ---
        self.doc(
            "csv.add_row",
            "csv.add_row(csv, row)",
            "CSV",
            "csv",
            "a copy of csv with one row (a list of cells) appended",
        );
        self.doc(
            "csv.add_column",
            "csv.add_column(csv, values, name)",
            "Result",
            "csv",
            "a copy of csv with a column appended: values[0] is a header-row placeholder, name is the header",
        );
        self.doc(
            "csv.set_cell",
            "csv.set_cell(csv, row, col, value)",
            "Result",
            "csv",
            "a copy of csv with one cell replaced",
        );
        self.doc(
            "csv.set_headers",
            "csv.set_headers(csv, headers)",
            "CSV",
            "csv",
            "a copy of csv with its header row set",
        );
        self.doc(
            "csv.filter_rows",
            "csv.filter_rows(csv, col_index, value)",
            "Result",
            "csv",
            "keep only the rows whose column at integer index col_index equals the string value",
        );
        self.doc(
            "csv.sort_by_column",
            "csv.sort_by_column(csv, col_index, ascending)",
            "CSV",
            "csv",
            "a copy of csv sorted by the column at col_index — ascending when true, descending when false",
        );
        self.doc(
            "csv.to_json",
            "csv.to_json(rows, with_headers)",
            "Result",
            "csv",
            "parsed rows to JSON text (with_headers treats row 0 as column names)",
        );
        self.doc(
            "csv.from_json",
            "csv.from_json(json, headers)",
            "Result",
            "csv",
            "JSON text plus a header list to CSV rows (a list of string-lists, headers first)",
        );

        // --- db: transactions ---
        self.doc(
            "db.begin",
            "db.begin(conn)",
            "Result",
            "db",
            "begin a transaction on the connection",
        );
        self.doc(
            "db.commit",
            "db.commit(conn)",
            "Result",
            "db",
            "commit the open transaction",
        );
        self.doc(
            "db.rollback",
            "db.rollback(conn)",
            "Result",
            "db",
            "roll back the open transaction",
        );

        // --- random: remaining generators ---
        self.doc(
            "random.gauss",
            "random.gauss(mean, std)",
            "Float",
            "random",
            "a normally distributed draw with the given mean and standard deviation",
        );
        self.doc(
            "random.randstr_alnum",
            "random.randstr_alnum(n)",
            "String",
            "random",
            "a random alphanumeric string of length n",
        );
    }

    fn add_embedded_utility_docs(&mut self) {
        // --- colx: the full collections toolkit (use colx) ---
        self.doc(
            "colx.unique",
            "colx.unique(xs)",
            "List",
            "colx",
            "the distinct elements of xs, first occurrence wins, order preserved",
        );
        self.doc(
            "colx.partition",
            "colx.partition(xs, pred)",
            "Tuple",
            "colx",
            "split xs into a (matching, non-matching) tuple by a predicate",
        );
        self.doc(
            "colx.sum_by",
            "colx.sum_by(xs, f)",
            "Number",
            "colx",
            "the sum of f(x) over every element of xs",
        );
        self.doc(
            "colx.all",
            "colx.all(xs, pred)",
            "Bool",
            "colx",
            "does pred hold for every element of xs?",
        );
        self.doc(
            "colx.any",
            "colx.any(xs, pred)",
            "Bool",
            "colx",
            "does pred hold for at least one element of xs?",
        );
        self.doc(
            "colx.count_by",
            "colx.count_by(xs, key_fn)",
            "Map",
            "colx",
            "a map from key_fn(x) to how many elements share that key",
        );
        self.doc(
            "colx.take_while",
            "colx.take_while(xs, pred)",
            "List",
            "colx",
            "the leading run of elements satisfying pred",
        );
        self.doc(
            "colx.drop_while",
            "colx.drop_while(xs, pred)",
            "List",
            "colx",
            "xs with the leading run satisfying pred removed",
        );
        self.doc(
            "colx.flat_map",
            "colx.flat_map(xs, f)",
            "List",
            "colx",
            "map f over xs and concatenate the resulting lists",
        );
        self.doc(
            "colx.frequencies",
            "colx.frequencies(xs)",
            "Map",
            "colx",
            "a map from each distinct element to how often it occurs",
        );
        self.doc(
            "colx.last",
            "colx.last(xs)",
            "Value",
            "colx",
            "the last element of xs",
        );
        self.doc(
            "colx.min_by",
            "colx.min_by(xs, key_fn)",
            "Value",
            "colx",
            "the element with the smallest key_fn(x)",
        );
        self.doc(
            "colx.max_by",
            "colx.max_by(xs, key_fn)",
            "Value",
            "colx",
            "the element with the largest key_fn(x)",
        );
        self.doc(
            "colx.sort_by",
            "colx.sort_by(xs, key_fn)",
            "List",
            "colx",
            "xs sorted ascending by key_fn(x)",
        );
        self.doc(
            "colx.window",
            "colx.window(xs, size)",
            "List",
            "colx",
            "every contiguous sublist of the given size (a sliding window)",
        );
        self.doc(
            "colx.zip_with",
            "colx.zip_with(a, b, f)",
            "List",
            "colx",
            "combine two lists elementwise with f, stopping at the shorter",
        );

        // --- mathx: math in olang source (use mathx) ---
        self.doc(
            "mathx.PI",
            "mathx.PI",
            "Float",
            "mathx",
            "the constant pi (3.14159…)",
        );
        self.doc(
            "mathx.E",
            "mathx.E",
            "Float",
            "mathx",
            "Euler's number e (2.71828…)",
        );
        self.doc(
            "mathx.TAU",
            "mathx.TAU",
            "Float",
            "mathx",
            "the constant tau, 2*pi (6.28318…)",
        );
        self.doc(
            "mathx.abs",
            "mathx.abs(x)",
            "Number",
            "mathx",
            "the absolute value of x",
        );
        self.doc(
            "mathx.sign",
            "mathx.sign(x)",
            "Int",
            "mathx",
            "-1, 0, or 1 according to the sign of x",
        );
        self.doc(
            "mathx.min",
            "mathx.min(a, b)",
            "Number",
            "mathx",
            "the smaller of a and b",
        );
        self.doc(
            "mathx.max",
            "mathx.max(a, b)",
            "Number",
            "mathx",
            "the larger of a and b",
        );
        self.doc(
            "mathx.gcd",
            "mathx.gcd(a, b)",
            "Int",
            "mathx",
            "greatest common divisor of two integers",
        );
        self.doc(
            "mathx.lcm",
            "mathx.lcm(a, b)",
            "Int",
            "mathx",
            "least common multiple of two integers",
        );
        self.doc(
            "mathx.factorial",
            "mathx.factorial(n)",
            "Int",
            "mathx",
            "n! — the product of 1..n",
        );
        self.doc(
            "mathx.trunc",
            "mathx.trunc(x)",
            "Float",
            "mathx",
            "x with its fractional part removed (toward zero)",
        );
        self.doc(
            "mathx.floor",
            "mathx.floor(x)",
            "Float",
            "mathx",
            "the largest whole number not greater than x",
        );
        self.doc(
            "mathx.ceil",
            "mathx.ceil(x)",
            "Float",
            "mathx",
            "the smallest whole number not less than x",
        );
        self.doc(
            "mathx.round",
            "mathx.round(x)",
            "Float",
            "mathx",
            "x rounded to the nearest whole number, halves away from zero",
        );
        self.doc(
            "mathx.fract",
            "mathx.fract(x)",
            "Float",
            "mathx",
            "the fractional part of x (x minus its truncation)",
        );
        self.doc(
            "mathx.radians",
            "mathx.radians(deg)",
            "Float",
            "mathx",
            "degrees converted to radians",
        );
        self.doc(
            "mathx.degrees",
            "mathx.degrees(rad)",
            "Float",
            "mathx",
            "radians converted to degrees",
        );
        self.doc(
            "mathx.sqrt",
            "mathx.sqrt(x)",
            "Float",
            "mathx",
            "the square root of x (Newton's method)",
        );
    }

    /// Add filesystem function documentation
    fn add_db_functions(&mut self) {
        self.add_function(FunctionDoc {
            name: "db.open".to_string(),
            description: "Open a SQLite database. Use \":memory:\" for an in-memory database. Returns a Connection.".to_string(),
            syntax: "db.open(path)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["let c = unwrap(db.open(\":memory:\"))".to_string()],
            category: "Database".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "db.execute".to_string(),
            description: "Run a statement that changes data (CREATE/INSERT/UPDATE/DELETE). Returns rows affected. Bind values with ? placeholders.".to_string(),
            syntax: "db.execute(conn, sql[, params])".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["unwrap(db.execute(c, \"INSERT INTO t VALUES (?, ?)\", [1, \"ann\"]))".to_string()],
            category: "Database".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "db.query".to_string(),
            description:
                "Run a SELECT. Returns a list of rows, each a map from column name to value."
                    .to_string(),
            syntax: "db.query(conn, sql[, params])".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec![
                "unwrap(db.query(c, \"SELECT * FROM t WHERE age >= ?\", [18]))".to_string(),
            ],
            category: "Database".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "db.query_one".to_string(),
            description: "Like query but returns the first row (a map), or unit if none."
                .to_string(),
            syntax: "db.query_one(conn, sql[, params])".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["unwrap(db.query_one(c, \"SELECT COUNT(*) AS n FROM t\"))".to_string()],
            category: "Database".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "db.close".to_string(),
            description: "Close a connection and release it.".to_string(),
            syntax: "db.close(conn)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["unwrap(db.close(c))".to_string()],
            category: "Database".to_string(),
            see_also: vec![],
        });
    }

    fn add_string_functions(&mut self) {
        self.add_function(FunctionDoc {
            name: "str.to_upper".to_string(),
            description: "Uppercase a string.".to_string(),
            syntax: "str.to_upper(s)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.to_upper(\"hi\") -> \"HI\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.to_lower".to_string(),
            description: "Lowercase a string.".to_string(),
            syntax: "str.to_lower(s)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.to_lower(\"HI\") -> \"hi\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.trim".to_string(),
            description: "Remove leading and trailing whitespace.".to_string(),
            syntax: "str.trim(s)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.trim(\"  x  \") -> \"x\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.trim_start".to_string(),
            description: "Remove leading whitespace.".to_string(),
            syntax: "str.trim_start(s)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.trim_start(\"  x\") -> \"x\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.trim_end".to_string(),
            description: "Remove trailing whitespace.".to_string(),
            syntax: "str.trim_end(s)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.trim_end(\"x  \") -> \"x\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.replace".to_string(),
            description: "Replace every occurrence of a substring.".to_string(),
            syntax: "str.replace(s, from, to)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.replace(\"a-b-c\", \"-\", \"+\") -> \"a+b+c\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.replace_first".to_string(),
            description: "Replace the first occurrence only.".to_string(),
            syntax: "str.replace_first(s, from, to)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.replace_first(\"a-b\", \"-\", \"+\") -> \"a+b\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.split".to_string(),
            description: "Split a string on a separator.".to_string(),
            syntax: "str.split(s, sep)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["str.split(\"a,b,c\", \",\") -> [\"a\",\"b\",\"c\"]".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.join".to_string(),
            description: "Join a list of strings with a separator.".to_string(),
            syntax: "str.join(list, sep)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.join([\"a\",\"b\"], \"-\") -> \"a-b\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.substring".to_string(),
            description: "Character-indexed slice; indices clamp to bounds.".to_string(),
            syntax: "str.substring(s, start, end)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.substring(\"hello\", 0, 3) -> \"hel\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.index_of".to_string(),
            description:
                "Character index of the first match, or Unit when absent (test with != ())."
                    .to_string(),
            syntax: "str.index_of(s, sub)".to_string(),
            parameters: vec![],
            return_type: "Int | ()".to_string(),
            examples: vec![
                "str.index_of(\"hello\", \"llo\") -> 2".to_string(),
                "str.index_of(\"hello\", \"z\") -> ()".to_string(),
            ],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.last_index_of".to_string(),
            description:
                "Character index of the last match, or Unit when absent (test with != ())."
                    .to_string(),
            syntax: "str.last_index_of(s, sub)".to_string(),
            parameters: vec![],
            return_type: "Int | ()".to_string(),
            examples: vec!["str.last_index_of(\"a-a\", \"a\") -> 2".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.contains".to_string(),
            description: "Whether the string contains a substring.".to_string(),
            syntax: "str.contains(s, sub)".to_string(),
            parameters: vec![],
            return_type: "Bool".to_string(),
            examples: vec!["str.contains(\"hello\", \"ell\") -> true".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.starts_with".to_string(),
            description: "Whether the string starts with a prefix.".to_string(),
            syntax: "str.starts_with(s, prefix)".to_string(),
            parameters: vec![],
            return_type: "Bool".to_string(),
            examples: vec!["str.starts_with(\"hello\", \"he\") -> true".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.ends_with".to_string(),
            description: "Whether the string ends with a suffix.".to_string(),
            syntax: "str.ends_with(s, suffix)".to_string(),
            parameters: vec![],
            return_type: "Bool".to_string(),
            examples: vec!["str.ends_with(\"hello\", \"lo\") -> true".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.repeat".to_string(),
            description: "Repeat a string n times.".to_string(),
            syntax: "str.repeat(s, n)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.repeat(\"ab\", 3) -> \"ababab\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.count".to_string(),
            description: "Count non-overlapping occurrences of a substring.".to_string(),
            syntax: "str.count(s, sub)".to_string(),
            parameters: vec![],
            return_type: "Int".to_string(),
            examples: vec!["str.count(\"banana\", \"a\") -> 3".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.char_at".to_string(),
            description: "Character at index i (empty if out of range).".to_string(),
            syntax: "str.char_at(s, i)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.char_at(\"hello\", 1) -> \"e\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.reverse".to_string(),
            description: "Reverse a string by character.".to_string(),
            syntax: "str.reverse(s)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.reverse(\"abc\") -> \"cba\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.capitalize".to_string(),
            description: "Uppercase the first character.".to_string(),
            syntax: "str.capitalize(s)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.capitalize(\"hi\") -> \"Hi\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.chars".to_string(),
            description: "Split into a list of single-character strings.".to_string(),
            syntax: "str.chars(s)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["str.chars(\"ab\") -> [\"a\",\"b\"]".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.lines".to_string(),
            description: "Split into a list of lines.".to_string(),
            syntax: "str.lines(s)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["str.lines(text)".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.words".to_string(),
            description: "Split on whitespace, dropping empties.".to_string(),
            syntax: "str.words(s)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["str.words(\"  a b \") -> [\"a\",\"b\"]".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.pad_start".to_string(),
            description: "Left-pad to a target length.".to_string(),
            syntax: "str.pad_start(s, len, pad)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.pad_start(\"7\", 3, \"0\") -> \"007\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.pad_end".to_string(),
            description: "Right-pad to a target length.".to_string(),
            syntax: "str.pad_end(s, len, pad)".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec!["str.pad_end(\"7\", 3, \"0\") -> \"700\"".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.is_empty".to_string(),
            description: "Whether the string is empty.".to_string(),
            syntax: "str.is_empty(s)".to_string(),
            parameters: vec![],
            return_type: "Bool".to_string(),
            examples: vec!["str.is_empty(\"\") -> true".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.length".to_string(),
            description: "Length in characters.".to_string(),
            syntax: "str.length(s)".to_string(),
            parameters: vec![],
            return_type: "Int".to_string(),
            examples: vec!["str.length(\"hello\") -> 5".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.parse_int".to_string(),
            description: "Parse an integer; Ok(n) or Err.".to_string(),
            syntax: "str.parse_int(s)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["unwrap(str.parse_int(\"42\")) -> 42".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.parse_float".to_string(),
            description: "Parse a float; Ok(f) or Err.".to_string(),
            syntax: "str.parse_float(s)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["unwrap(str.parse_float(\"3.5\")) -> 3.5".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
    }

    fn add_regex_functions(&mut self) {
        self.add_function(FunctionDoc {
            name: "re.is_valid".to_string(),
            description: "Whether a regex pattern compiles.".to_string(),
            syntax: "re.is_valid(pattern)".to_string(),
            parameters: vec![],
            return_type: "Bool".to_string(),
            examples: vec!["re.is_valid(\"[a-z]+\") -> true".to_string()],
            category: "Regex".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "re.is_match".to_string(),
            description: "Whether the pattern matches anywhere in text.".to_string(),
            syntax: "re.is_match(pattern, text)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["re.is_match(\"[0-9]+\", \"a1\")".to_string()],
            category: "Regex".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "re.find".to_string(),
            description: "First match as a string, or empty if none.".to_string(),
            syntax: "re.find(pattern, text)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["re.find(\"[0-9]+\", \"a12\")".to_string()],
            category: "Regex".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "re.find_all".to_string(),
            description: "All non-overlapping matches as a list.".to_string(),
            syntax: "re.find_all(pattern, text)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["re.find_all(\"[0-9]+\", \"a1b22\")".to_string()],
            category: "Regex".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "re.captures".to_string(),
            description: "Capture groups of the first match; index 0 is the whole match."
                .to_string(),
            syntax: "re.captures(pattern, text)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["re.captures(\"(a)(b)\", \"ab\")".to_string()],
            category: "Regex".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "re.split".to_string(),
            description: "Split text on a pattern.".to_string(),
            syntax: "re.split(pattern, text)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["re.split(\", *\", \"a, b,c\")".to_string()],
            category: "Regex".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "re.replace".to_string(),
            description: "Replace the first match.".to_string(),
            syntax: "re.replace(pattern, text, rep)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["re.replace(\"[0-9]\", \"a1b1\", \"X\")".to_string()],
            category: "Regex".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "re.replace_all".to_string(),
            description: "Replace all matches.".to_string(),
            syntax: "re.replace_all(pattern, text, rep)".to_string(),
            parameters: vec![],
            return_type: "Result".to_string(),
            examples: vec!["re.replace_all(\" +\", \"a  b\", \"_\")".to_string()],
            category: "Regex".to_string(),
            see_also: vec![],
        });
    }

    fn add_collections_functions(&mut self) {
        self.add_function(FunctionDoc {
            name: "col.min_by".to_string(),
            description: "Element with the smallest key.".to_string(),
            syntax: "col.min_by(list, key_fn)".to_string(),
            parameters: vec![],
            return_type: "Any".to_string(),
            examples: vec!["col.min_by(people, (p) => p.age)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.max_by".to_string(),
            description: "Element with the largest key.".to_string(),
            syntax: "col.max_by(list, key_fn)".to_string(),
            parameters: vec![],
            return_type: "Any".to_string(),
            examples: vec!["col.max_by(people, (p) => p.age)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.sort_by".to_string(),
            description: "Sort ascending by a key function.".to_string(),
            syntax: "col.sort_by(list, key_fn)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["col.sort_by([3,1,2], (x) => x)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.count_by".to_string(),
            description: "Count elements per key.".to_string(),
            syntax: "col.count_by(list, key_fn)".to_string(),
            parameters: vec![],
            return_type: "Map".to_string(),
            examples: vec!["col.count_by(people, (p) => p.team)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.frequencies".to_string(),
            description: "Count occurrences of each value.".to_string(),
            syntax: "col.frequencies(list)".to_string(),
            parameters: vec![],
            return_type: "Map".to_string(),
            examples: vec!["col.frequencies([1,2,2])".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.partition".to_string(),
            description: "Split into (matching, non-matching).".to_string(),
            syntax: "col.partition(list, pred)".to_string(),
            parameters: vec![],
            return_type: "Tuple".to_string(),
            examples: vec!["col.partition([1,2,3], (x) => x > 1)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.flat_map".to_string(),
            description: "Map then flatten one level.".to_string(),
            syntax: "col.flat_map(list, fn)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["col.flat_map([1,2], (x) => [x, x])".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.take_while".to_string(),
            description: "Longest prefix satisfying the predicate.".to_string(),
            syntax: "col.take_while(list, pred)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["col.take_while([1,2,9], (x) => x < 5)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.drop_while".to_string(),
            description: "Drop the leading prefix satisfying the predicate.".to_string(),
            syntax: "col.drop_while(list, pred)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["col.drop_while([1,2,9], (x) => x < 5)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.all".to_string(),
            description: "Whether every element satisfies the predicate.".to_string(),
            syntax: "col.all(list, pred)".to_string(),
            parameters: vec![],
            return_type: "Bool".to_string(),
            examples: vec!["col.all([2,4], (x) => x % 2 == 0)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.any".to_string(),
            description: "Whether any element satisfies the predicate.".to_string(),
            syntax: "col.any(list, pred)".to_string(),
            parameters: vec![],
            return_type: "Bool".to_string(),
            examples: vec!["col.any([1,2], (x) => x % 2 == 0)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.sum_by".to_string(),
            description: "Sum a projection over the list.".to_string(),
            syntax: "col.sum_by(list, fn)".to_string(),
            parameters: vec![],
            return_type: "Number".to_string(),
            examples: vec!["col.sum_by(items, (x) => x.price)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.unique".to_string(),
            description: "Deduplicate, preserving first-seen order.".to_string(),
            syntax: "col.unique(list)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["col.unique([1,1,2])".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.window".to_string(),
            description: "Sliding windows of a fixed size.".to_string(),
            syntax: "col.window(list, size)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["col.window([1,2,3], 2)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.zip_with".to_string(),
            description: "Combine two lists element-wise.".to_string(),
            syntax: "col.zip_with(a, b, fn)".to_string(),
            parameters: vec![],
            return_type: "List".to_string(),
            examples: vec!["col.zip_with([1,2],[3,4],(a,b)=>a+b)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "col.last".to_string(),
            description: "The last element of a list.".to_string(),
            syntax: "col.last(list)".to_string(),
            parameters: vec![],
            return_type: "Any".to_string(),
            examples: vec!["col.last([7,8,9]) -> 9".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
    }

    fn add_fs_functions(&mut self) {
        // The bytes module: immutable binary data.
        self.doc(
            "bytes.from_list",
            "bytes.from_list(ints)",
            "Bytes",
            "Bytes",
            "Build a Bytes value from a list of integers 0..=255. Raises on a non-integer or out-of-range element.",
        );
        self.doc(
            "bytes.to_list",
            "bytes.to_list(b)",
            "List",
            "Bytes",
            "The bytes as a list of integers 0..=255.",
        );
        self.doc(
            "bytes.from_string",
            "bytes.from_string(s)",
            "Bytes",
            "Bytes",
            "A string's UTF-8 bytes as a Bytes value.",
        );
        self.doc(
            "bytes.to_string",
            "bytes.to_string(b)",
            "Result",
            "Bytes",
            "Decode Bytes as UTF-8 text: Ok(string), or Err when the bytes are not valid UTF-8.",
        );
        self.doc(
            "bytes.len",
            "bytes.len(b)",
            "Int",
            "Bytes",
            "The byte count. The global len(b) answers the same.",
        );
        self.doc(
            "bytes.slice",
            "bytes.slice(b, from, to)",
            "Bytes",
            "Bytes",
            "Half-open byte slice, clamped to the value's bounds — the same shape as str.substring.",
        );
        self.doc(
            "bytes.concat",
            "bytes.concat(a, b)",
            "Bytes",
            "Bytes",
            "The concatenation of two Bytes values.",
        );

        // File I/O operations
        self.add_function(FunctionDoc {
            name: "fs.read_file".to_string(),
            description: "Read the entire contents of a file as a string".to_string(),
            syntax: "fs.read_file(path)".to_string(),
            parameters: vec!["path: String - The file path to read from".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "fs.read_file(\"config.txt\")  // Ok(\"file contents\")".to_string(),
                "match fs.read_file(\"data.json\") { Ok(content) => println(content); Err(e) => println(\"Error: \" + e) }".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.write_file".to_string(), "fs.append_file".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.write_file".to_string(),
            description: "Write string contents to a file, creating it if it doesn't exist"
                .to_string(),
            syntax: "fs.write_file(path, contents)".to_string(),
            parameters: vec![
                "path: String - The file path to write to".to_string(),
                "contents: String - The content to write".to_string(),
            ],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "fs.write_file(\"output.txt\", \"Hello, World!\")  // Ok(Unit)".to_string(),
                "fs.write_file(\"/tmp/data.json\", json.stringify(data))".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.read_file".to_string(), "fs.append_file".to_string()],
        });

        self.doc(
            "fs.read_bytes",
            "fs.read_bytes(path)",
            "Result",
            "Filesystem",
            "Read a file's raw bytes — the binary twin of fs.read_file, for content that is not UTF-8 text. Ok(Bytes), or Err with the OS error. Requires the fs capability at read level.",
        );
        self.doc(
            "fs.write_bytes",
            "fs.write_bytes(path, b)",
            "Result",
            "Filesystem",
            "Write a Bytes value to a file — the binary twin of fs.write_file. Requires the fs capability at write level.",
        );

        self.add_function(FunctionDoc {
            name: "fs.append_file".to_string(),
            description:
                "Append string contents to the end of a file, creating it if it doesn't exist"
                    .to_string(),
            syntax: "fs.append_file(path, contents)".to_string(),
            parameters: vec![
                "path: String - The file path to append to".to_string(),
                "contents: String - The content to append".to_string(),
            ],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "fs.append_file(\"log.txt\", \"New log entry\\n\")".to_string(),
                "fs.append_file(\"data.csv\", \"new,row,data\\n\")".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.write_file".to_string(), "fs.read_file".to_string()],
        });

        // File/directory existence and type checking
        self.add_function(FunctionDoc {
            name: "fs.exists".to_string(),
            description: "Check if a file or directory exists at the given path".to_string(),
            syntax: "fs.exists(path)".to_string(),
            parameters: vec!["path: String - The path to check".to_string()],
            return_type: "Bool".to_string(),
            examples: vec![
                "fs.exists(\"/home/user/file.txt\")  // true or false".to_string(),
                "if fs.exists(\"config.json\") { println(\"Config found\") }".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.is_file".to_string(), "fs.is_dir".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.is_file".to_string(),
            description: "Check if the path points to a regular file".to_string(),
            syntax: "fs.is_file(path)".to_string(),
            parameters: vec!["path: String - The path to check".to_string()],
            return_type: "Bool".to_string(),
            examples: vec![
                "fs.is_file(\"document.pdf\")  // true if it's a file".to_string(),
                "filter(fs.list_dir(\".\"), (name) => fs.is_file(name))".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.is_dir".to_string(), "fs.exists".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.is_dir".to_string(),
            description: "Check if the path points to a directory".to_string(),
            syntax: "fs.is_dir(path)".to_string(),
            parameters: vec!["path: String - The path to check".to_string()],
            return_type: "Bool".to_string(),
            examples: vec![
                "fs.is_dir(\"/home/user\")  // true if it's a directory".to_string(),
                "filter(fs.list_dir(\".\"), (name) => fs.is_dir(name))".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.is_file".to_string(), "fs.list_dir".to_string()],
        });

        // Directory operations
        self.add_function(FunctionDoc {
            name: "fs.list_dir".to_string(),
            description: "List the contents of a directory as a list of filenames".to_string(),
            syntax: "fs.list_dir(path)".to_string(),
            parameters: vec!["path: String - The directory path to list".to_string()],
            return_type: "Result<List<String>, Error>".to_string(),
            examples: vec![
                "fs.list_dir(\".\")  // Ok([\"file1.txt\", \"dir1\", \"file2.py\"])".to_string(),
                "match fs.list_dir(\"/tmp\") { Ok(files) => map(files, println); Err(e) => println(\"Error: \" + e) }".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.is_dir".to_string(), "fs.create_dir".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.create_dir".to_string(),
            description: "Create a single directory (parent must exist)".to_string(),
            syntax: "fs.create_dir(path)".to_string(),
            parameters: vec!["path: String - The directory path to create".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "fs.create_dir(\"new_folder\")  // Ok(Unit)".to_string(),
                "fs.create_dir(\"/home/user/projects\")".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.create_dir_all".to_string(), "fs.remove_dir".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.create_dir_all".to_string(),
            description: "Create a directory and all missing parent directories".to_string(),
            syntax: "fs.create_dir_all(path)".to_string(),
            parameters: vec!["path: String - The directory path to create recursively".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "fs.create_dir_all(\"/path/to/deep/folder\")  // Creates all missing dirs"
                    .to_string(),
                "fs.create_dir_all(\"projects/olang/src\")".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.create_dir".to_string(), "fs.remove_dir_all".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.remove_dir".to_string(),
            description: "Remove an empty directory".to_string(),
            syntax: "fs.remove_dir(path)".to_string(),
            parameters: vec!["path: String - The empty directory path to remove".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "fs.remove_dir(\"empty_folder\")  // Ok(Unit)".to_string(),
                "fs.remove_dir(\"/tmp/old_dir\")".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.remove_dir_all".to_string(), "fs.create_dir".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.remove_dir_all".to_string(),
            description: "Remove a directory and all its contents recursively".to_string(),
            syntax: "fs.remove_dir_all(path)".to_string(),
            parameters: vec!["path: String - The directory path to remove recursively".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "fs.remove_dir_all(\"old_project\")  // Removes everything inside".to_string(),
                "fs.remove_dir_all(\"/tmp/cache\")".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.remove_dir".to_string(), "fs.remove_file".to_string()],
        });

        // File manipulation
        self.add_function(FunctionDoc {
            name: "fs.remove_file".to_string(),
            description: "Delete a file from the filesystem".to_string(),
            syntax: "fs.remove_file(path)".to_string(),
            parameters: vec!["path: String - The file path to delete".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "fs.remove_file(\"old_file.txt\")  // Ok(Unit)".to_string(),
                "fs.remove_file(\"/tmp/temp_data.json\")".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.copy_file".to_string(), "fs.move_file".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.copy_file".to_string(),
            description: "Copy a file from source to destination path".to_string(),
            syntax: "fs.copy_file(source, destination)".to_string(),
            parameters: vec![
                "source: String - The source file path".to_string(),
                "destination: String - The destination file path".to_string(),
            ],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "fs.copy_file(\"original.txt\", \"backup.txt\")  // Ok(Unit)".to_string(),
                "fs.copy_file(\"/home/user/doc.pdf\", \"/backup/doc.pdf\")".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.move_file".to_string(), "fs.remove_file".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.move_file".to_string(),
            description: "Move or rename a file from old path to new path".to_string(),
            syntax: "fs.move_file(old_path, new_path)".to_string(),
            parameters: vec![
                "old_path: String - The current file path".to_string(),
                "new_path: String - The new file path".to_string(),
            ],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "fs.move_file(\"temp.txt\", \"final.txt\")  // Rename".to_string(),
                "fs.move_file(\"data.json\", \"/archive/data.json\")  // Move".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.copy_file".to_string(), "fs.remove_file".to_string()],
        });

        // File information
        self.add_function(FunctionDoc {
            name: "fs.file_size".to_string(),
            description: "Get the size of a file in bytes".to_string(),
            syntax: "fs.file_size(path)".to_string(),
            parameters: vec!["path: String - The file path to get size of".to_string()],
            return_type: "Result<Int, Error>".to_string(),
            examples: vec![
                "fs.file_size(\"document.pdf\")  // Ok(1048576) for 1MB file".to_string(),
                "match fs.file_size(\"data.txt\") { Ok(size) => println(\"Size: \" + to_string(size) + \" bytes\"); Err(e) => println(\"Error: \" + e) }".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.file_info".to_string(), "fs.exists".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "fs.file_info".to_string(),
            description: "Get detailed information about a file or directory".to_string(),
            syntax: "fs.file_info(path)".to_string(),
            parameters: vec!["path: String - The path to get information about".to_string()],
            return_type: "Result<FileInfo, Error>".to_string(),
            examples: vec![
                "fs.file_info(\"script.py\")  // Ok(FileInfo { size: 1024, is_file: true, is_dir: false, readonly: false })".to_string(),
                "match fs.file_info(\".\") { Ok(info) => println(\"Directory size: \" + to_string(info.size)); Err(e) => println(\"Error: \" + e) }".to_string(),
            ],
            category: "Filesystem".to_string(),
            see_also: vec!["fs.file_size".to_string(), "fs.is_file".to_string(), "fs.is_dir".to_string()],
        });
    }

    /// Add HTTP function documentation
    fn add_http_functions(&mut self) {
        // HTTP Client operations
        self.add_function(FunctionDoc {
            name: "http.get".to_string(),
            description: "Make an HTTP GET request to the specified URL".to_string(),
            syntax: "http.get(url)".to_string(),
            parameters: vec!["url: String - The URL to send the GET request to".to_string()],
            return_type: "Result<HttpResponse, Error>".to_string(),
            examples: vec![
                "http.get(\"https://api.github.com/users/octocat\")  // Ok(HttpResponse { status: 200, body: \"...\", success: true })".to_string(),
                "match http.get(\"https://httpbin.org/get\") { Ok(response) => println(response.body); Err(e) => println(\"Error: \" + e) }".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.post".to_string(), "http.request".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "http.post".to_string(),
            description: "Make an HTTP POST request with the specified body data".to_string(),
            syntax: "http.post(url, body)".to_string(),
            parameters: vec![
                "url: String - The URL to send the POST request to".to_string(),
                "body: String - The request body content".to_string(),
            ],
            return_type: "Result<HttpResponse, Error>".to_string(),
            examples: vec![
                "http.post(\"https://httpbin.org/post\", \"{\\\"name\\\": \\\"John\\\"}\")"
                    .to_string(),
                "http.post(\"https://api.example.com/users\", json.stringify(user_data))"
                    .to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.get".to_string(), "http.put".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "http.put".to_string(),
            description: "Make an HTTP PUT request to update a resource".to_string(),
            syntax: "http.put(url, body)".to_string(),
            parameters: vec![
                "url: String - The URL to send the PUT request to".to_string(),
                "body: String - The request body content".to_string(),
            ],
            return_type: "Result<HttpResponse, Error>".to_string(),
            examples: vec![
                "http.put(\"https://api.example.com/users/123\", \"{\\\"name\\\": \\\"Jane\\\"}\")"
                    .to_string(),
                "http.put(\"https://httpbin.org/put\", updated_data)".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.post".to_string(), "http.delete".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "http.delete".to_string(),
            description: "Make an HTTP DELETE request to remove a resource".to_string(),
            syntax: "http.delete(url)".to_string(),
            parameters: vec!["url: String - The URL to send the DELETE request to".to_string()],
            return_type: "Result<HttpResponse, Error>".to_string(),
            examples: vec![
                "http.delete(\"https://api.example.com/users/123\")".to_string(),
                "http.delete(\"https://httpbin.org/delete\")".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.put".to_string(), "http.get".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "http.request".to_string(),
            description: "Make a custom HTTP request with specified method, URL, and body"
                .to_string(),
            syntax: "http.request(method, url, body)".to_string(),
            parameters: vec![
                "method: String - HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD)".to_string(),
                "url: String - The URL to send the request to".to_string(),
                "body: String - The request body content (ignored for GET/HEAD)".to_string(),
            ],
            return_type: "Result<HttpResponse, Error>".to_string(),
            examples: vec![
                "http.request(\"PATCH\", \"https://api.example.com/users/123\", patch_data)"
                    .to_string(),
                "http.request(\"HEAD\", \"https://httpbin.org/status/200\", \"\")".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.get".to_string(), "http.post".to_string()],
        });

        // HTTP Server operations
        self.add_function(FunctionDoc {
            name: "http.serve".to_string(),
            description: "Start an HTTP server on the specified port with a request handler".to_string(),
            syntax: "http.serve(port, handler)".to_string(),
            parameters: vec![
                "port: Int - The port number to listen on".to_string(),
                "handler: Function - Request handler function (request) -> response".to_string(),
            ],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "http.serve(8080, (req) => http.response(200, \"Hello World!\"))".to_string(),
                "let handler = (request) => http.response(200, \"Welcome to Olang server!\")\\nhttp.serve(3000, handler)".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.response".to_string(), "http.response_with_headers".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "http.response".to_string(),
            description: "Create an HTTP response with status code and body".to_string(),
            syntax: "http.response(status, body)".to_string(),
            parameters: vec![
                "status: Int - HTTP status code (200, 404, 500, etc.)".to_string(),
                "body: String - Response body content".to_string(),
            ],
            return_type: "HttpResponse".to_string(),
            examples: vec![
                "http.response(200, \"Success!\")".to_string(),
                "http.response(404, \"Not Found\")".to_string(),
                "http.response(201, json.stringify(created_user))".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec![
                "http.response_with_headers".to_string(),
                "http.serve".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "http.response_with_headers".to_string(),
            description: "Create an HTTP response with status code, body, and custom headers".to_string(),
            syntax: "http.response_with_headers(status, body, headers)".to_string(),
            parameters: vec![
                "status: Int - HTTP status code".to_string(),
                "body: String - Response body content".to_string(),
                "headers: Struct - Custom headers as key-value pairs".to_string(),
            ],
            return_type: "HttpResponse".to_string(),
            examples: vec![
                "let headers = { \"Content-Type\": \"application/json\", \"Cache-Control\": \"no-cache\" }\\nhttp.response_with_headers(200, json_data, headers)".to_string(),
                "http.response_with_headers(201, \"Created\", { \"Location\": \"/users/123\" })".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.response".to_string(), "http.serve".to_string()],
        });

        // HTTP Utility functions
        self.add_function(FunctionDoc {
            name: "http.parse_url".to_string(),
            description: "Parse a URL string into its component parts".to_string(),
            syntax: "http.parse_url(url)".to_string(),
            parameters: vec!["url: String - The URL to parse".to_string()],
            return_type: "Result<UrlInfo, Error>".to_string(),
            examples: vec![
                "http.parse_url(\"https://example.com:8080/path?query=value#fragment\")".to_string(),
                "match http.parse_url(url) { Ok(info) => println(\"Host: \" + info.host); Err(e) => println(\"Invalid URL\") }".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.encode_query".to_string(), "http.decode_query".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "http.encode_query".to_string(),
            description: "Encode a struct of parameters into a URL query string".to_string(),
            syntax: "http.encode_query(params)".to_string(),
            parameters: vec!["params: Struct - Key-value pairs to encode as query parameters".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "let params = { \"name\": \"John Doe\", \"age\": \"30\", \"city\": \"New York\" }\\nhttp.encode_query(params)  // Ok(\"name=John%20Doe&age=30&city=New%20York\")".to_string(),
                "http.encode_query({ \"q\": \"hello world\", \"limit\": \"10\" })".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.decode_query".to_string(), "http.parse_url".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "http.decode_query".to_string(),
            description: "Decode a URL query string into a struct of parameters".to_string(),
            syntax: "http.decode_query(query_string)".to_string(),
            parameters: vec!["query_string: String - The query string to decode".to_string()],
            return_type: "Map".to_string(),
            examples: vec![
                "http.decode_query(\"name=John%20Doe&age=30&city=New%20York\")".to_string(),
                "match http.decode_query(request.query) { Ok(params) => println(params.name); Err(e) => println(\"Invalid query\") }".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.encode_query".to_string(), "http.parse_url".to_string()],
        });
    }

    /// Add math module documentation
    /// The bundled olang-source collections (Campaign 6) and their
    /// Rust-side primitives: entries drive :help, editor completions,
    /// hover, and signature help.
    fn add_bundled_collections_functions(&mut self) {
        self.add_function(FunctionDoc {
            name: "col.set".to_string(),
            description: "The list with element i replaced by v. `xs = col.set(xs, i, v)` writes in place when xs holds the only reference - the collections' write primitive. Negative i counts from the end; out of bounds raises.".to_string(),
            syntax: "col.set(xs, i, v)".to_string(),
            parameters: vec![
                "xs: List".to_string(),
                "i: Int - index, negatives from the end".to_string(),
                "v: value".to_string(),
            ],
            return_type: "List".to_string(),
            examples: vec!["col.set([1, 2, 3], 1, 9)  // [1, 9, 3]".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["col.swap".to_string(), "col.filled".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "col.swap".to_string(),
            description: "The list with elements i and j exchanged. Fuses to an in-place O(1) swap under `xs = col.swap(xs, i, j)`.".to_string(),
            syntax: "col.swap(xs, i, j)".to_string(),
            parameters: vec![
                "xs: List".to_string(),
                "i: Int".to_string(),
                "j: Int".to_string(),
            ],
            return_type: "List".to_string(),
            examples: vec!["col.swap([1, 2, 3], 0, 2)  // [3, 2, 1]".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["col.set".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "col.filled".to_string(),
            description: "A list of n copies of v - the preallocation primitive flat-array structures build their backing stores with.".to_string(),
            syntax: "col.filled(n, v)".to_string(),
            parameters: vec![
                "n: Int - non-negative length".to_string(),
                "v: value".to_string(),
            ],
            return_type: "List".to_string(),
            examples: vec!["col.filled(3, 0)  // [0, 0, 0]".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["col.set".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "str.char_code".to_string(),
            description:
                "The Unicode code point of the string's first character, or () for an empty string."
                    .to_string(),
            syntax: "str.char_code(s)".to_string(),
            parameters: vec!["s: String".to_string()],
            return_type: "Int | Unit".to_string(),
            examples: vec!["str.char_code(\"A\")  // 65".to_string()],
            category: "String".to_string(),
            see_also: vec!["str.char_at".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.heap.new".to_string(),
            description: "A new, empty binary min-heap of (priority, item) pairs. Automatically available - no `use` needed. Rebind through every write: h = heap.push(h, p, x).".to_string(),
            syntax: "heap.new()".to_string(),
            parameters: vec![],
            return_type: "Heap handle".to_string(),
            examples: vec!["let mut h = heap.new()".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.heap.push".to_string(), "collections.heap.pop".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.heap.push".to_string(),
            description: "The heap with (prio, item) added, O(log n) in place under the rebind convention. prio is an Int or Float; item is any value.".to_string(),
            syntax: "heap.push(h, prio, item)".to_string(),
            parameters: vec![
                "h: Heap handle".to_string(),
                "prio: Int | Float".to_string(),
                "item: value".to_string(),
            ],
            return_type: "Heap handle".to_string(),
            examples: vec!["h = heap.push(h, 3, \"job\")".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.heap.pop".to_string(), "collections.heap.top_prio".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.heap.pop".to_string(),
            description: "The heap with its smallest pair removed. Read top_prio/top_item first; popping an empty heap raises.".to_string(),
            syntax: "heap.pop(h)".to_string(),
            parameters: vec![
                "h: Heap handle".to_string(),
            ],
            return_type: "Heap handle".to_string(),
            examples: vec!["h = heap.pop(h)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.heap.top_prio".to_string(), "collections.heap.top_item".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.heap.top_prio".to_string(),
            description: "The smallest priority, without removing it.".to_string(),
            syntax: "heap.top_prio(h)".to_string(),
            parameters: vec!["h: Heap handle".to_string()],
            return_type: "Int | Float".to_string(),
            examples: vec!["heap.top_prio(h)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.heap.top_item".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.heap.top_item".to_string(),
            description: "The item paired with the smallest priority, without removing it."
                .to_string(),
            syntax: "heap.top_item(h)".to_string(),
            parameters: vec!["h: Heap handle".to_string()],
            return_type: "value".to_string(),
            examples: vec!["heap.top_item(h)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.heap.top_prio".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.heap.size".to_string(),
            description: "The number of pairs in the heap.".to_string(),
            syntax: "heap.size(h)".to_string(),
            parameters: vec!["h: Heap handle".to_string()],
            return_type: "Int".to_string(),
            examples: vec!["heap.size(h)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.heap.is_empty".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.heap.is_empty".to_string(),
            description: "True when the heap holds nothing.".to_string(),
            syntax: "heap.is_empty(h)".to_string(),
            parameters: vec!["h: Heap handle".to_string()],
            return_type: "Bool".to_string(),
            examples: vec!["while !heap.is_empty(h) { ... }".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.heap.size".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.heap.from_lists".to_string(),
            description: "A heap built from parallel priority and item lists in O(n), against O(n log n) repeated pushes.".to_string(),
            syntax: "heap.from_lists(prios, items)".to_string(),
            parameters: vec![
                "prios: List of Int | Float".to_string(),
                "items: List".to_string(),
            ],
            return_type: "Heap handle".to_string(),
            examples: vec!["heap.from_lists([3, 1], [\"a\", \"b\"])".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.heap.push".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.new".to_string(),
            description: "A new, empty double-ended queue over a ring buffer. Automatically available. Rebind through every write: q = deque.push_back(q, x).".to_string(),
            syntax: "deque.new()".to_string(),
            parameters: vec![],
            return_type: "Deque handle".to_string(),
            examples: vec!["let mut q = deque.new()".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.deque.push_back".to_string(), "collections.deque.pop_front".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.push_back".to_string(),
            description: "The deque with x appended at the back, amortized O(1).".to_string(),
            syntax: "deque.push_back(q, x)".to_string(),
            parameters: vec!["q: Deque handle".to_string(), "x: value".to_string()],
            return_type: "Deque handle".to_string(),
            examples: vec!["q = deque.push_back(q, job)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![
                "collections.deque.push_front".to_string(),
                "collections.deque.pop_back".to_string(),
            ],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.push_front".to_string(),
            description: "The deque with x prepended at the front, amortized O(1).".to_string(),
            syntax: "deque.push_front(q, x)".to_string(),
            parameters: vec!["q: Deque handle".to_string(), "x: value".to_string()],
            return_type: "Deque handle".to_string(),
            examples: vec!["q = deque.push_front(q, job)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.deque.push_back".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.pop_front".to_string(),
            description: "The deque with its front element removed. Read front first; popping an empty deque raises.".to_string(),
            syntax: "deque.pop_front(q)".to_string(),
            parameters: vec![
                "q: Deque handle".to_string(),
            ],
            return_type: "Deque handle".to_string(),
            examples: vec!["q = deque.pop_front(q)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.deque.front".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.pop_back".to_string(),
            description: "The deque with its back element removed. Read back first; popping an empty deque raises.".to_string(),
            syntax: "deque.pop_back(q)".to_string(),
            parameters: vec![
                "q: Deque handle".to_string(),
            ],
            return_type: "Deque handle".to_string(),
            examples: vec!["q = deque.pop_back(q)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.deque.back".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.front".to_string(),
            description: "The front element, without removing it.".to_string(),
            syntax: "deque.front(q)".to_string(),
            parameters: vec!["q: Deque handle".to_string()],
            return_type: "value".to_string(),
            examples: vec!["deque.front(q)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.deque.back".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.back".to_string(),
            description: "The back element, without removing it.".to_string(),
            syntax: "deque.back(q)".to_string(),
            parameters: vec!["q: Deque handle".to_string()],
            return_type: "value".to_string(),
            examples: vec!["deque.back(q)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.deque.front".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.size".to_string(),
            description: "The number of elements.".to_string(),
            syntax: "deque.size(q)".to_string(),
            parameters: vec!["q: Deque handle".to_string()],
            return_type: "Int".to_string(),
            examples: vec!["deque.size(q)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.deque.is_empty".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.is_empty".to_string(),
            description: "True when the deque holds nothing.".to_string(),
            syntax: "deque.is_empty(q)".to_string(),
            parameters: vec!["q: Deque handle".to_string()],
            return_type: "Bool".to_string(),
            examples: vec!["while !deque.is_empty(q) { ... }".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.deque.size".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.deque.to_list".to_string(),
            description: "The elements front-to-back as a plain list.".to_string(),
            syntax: "deque.to_list(q)".to_string(),
            parameters: vec!["q: Deque handle".to_string()],
            return_type: "List".to_string(),
            examples: vec!["deque.to_list(q)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.new".to_string(),
            description: "A new, empty set of small non-negative integers with capacity n. Automatically available. Rebind through every write: b = bitset.add(b, i).".to_string(),
            syntax: "bitset.new(n)".to_string(),
            parameters: vec![
                "n: Int - capacity, members are 0..n-1".to_string(),
            ],
            return_type: "Bitset handle".to_string(),
            examples: vec!["let mut b = bitset.new(1000)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.bitset.add".to_string(), "collections.bitset.has".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.add".to_string(),
            description: "The set with i added, O(1) in place under the rebind convention."
                .to_string(),
            syntax: "bitset.add(b, i)".to_string(),
            parameters: vec![
                "b: Bitset handle".to_string(),
                "i: Int in 0..capacity".to_string(),
            ],
            return_type: "Bitset handle".to_string(),
            examples: vec!["b = bitset.add(b, 42)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![
                "collections.bitset.remove".to_string(),
                "collections.bitset.has".to_string(),
            ],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.remove".to_string(),
            description: "The set with i removed (a no-op when absent).".to_string(),
            syntax: "bitset.remove(b, i)".to_string(),
            parameters: vec!["b: Bitset handle".to_string(), "i: Int".to_string()],
            return_type: "Bitset handle".to_string(),
            examples: vec!["b = bitset.remove(b, 42)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.bitset.add".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.has".to_string(),
            description: "True when i is a member.".to_string(),
            syntax: "bitset.has(b, i)".to_string(),
            parameters: vec!["b: Bitset handle".to_string(), "i: Int".to_string()],
            return_type: "Bool".to_string(),
            examples: vec!["bitset.has(b, 42)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.bitset.count".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.count".to_string(),
            description: "The number of members.".to_string(),
            syntax: "bitset.count(b)".to_string(),
            parameters: vec!["b: Bitset handle".to_string()],
            return_type: "Int".to_string(),
            examples: vec!["bitset.count(b)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.bitset.to_list".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.capacity".to_string(),
            description: "The capacity the set was created with.".to_string(),
            syntax: "bitset.capacity(b)".to_string(),
            parameters: vec!["b: Bitset handle".to_string()],
            return_type: "Int".to_string(),
            examples: vec!["bitset.capacity(b)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.bitset.new".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.union".to_string(),
            description: "Members of either set (equal capacities), a word at a time.".to_string(),
            syntax: "bitset.union(a, b)".to_string(),
            parameters: vec![
                "a: Bitset handle".to_string(),
                "b: Bitset handle".to_string(),
            ],
            return_type: "Bitset handle".to_string(),
            examples: vec!["bitset.union(a, b)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![
                "collections.bitset.intersect".to_string(),
                "collections.bitset.difference".to_string(),
            ],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.intersect".to_string(),
            description: "Members of both sets (equal capacities).".to_string(),
            syntax: "bitset.intersect(a, b)".to_string(),
            parameters: vec![
                "a: Bitset handle".to_string(),
                "b: Bitset handle".to_string(),
            ],
            return_type: "Bitset handle".to_string(),
            examples: vec!["bitset.intersect(a, b)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.bitset.union".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.difference".to_string(),
            description: "Members of a not in b (equal capacities).".to_string(),
            syntax: "bitset.difference(a, b)".to_string(),
            parameters: vec![
                "a: Bitset handle".to_string(),
                "b: Bitset handle".to_string(),
            ],
            return_type: "Bitset handle".to_string(),
            examples: vec!["bitset.difference(a, b)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.bitset.union".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.bitset.to_list".to_string(),
            description: "The members in ascending order, as a list of Ints.".to_string(),
            syntax: "bitset.to_list(b)".to_string(),
            parameters: vec!["b: Bitset handle".to_string()],
            return_type: "List of Int".to_string(),
            examples: vec!["bitset.to_list(b)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.bitset.count".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.dsu.new".to_string(),
            description: "A new disjoint-sets (union-find) structure of n elements, each its own group. Automatically available. Rebind through unions: d = dsu.union(d, a, b).".to_string(),
            syntax: "dsu.new(n)".to_string(),
            parameters: vec![
                "n: Int - element count".to_string(),
            ],
            return_type: "Dsu handle".to_string(),
            examples: vec!["let mut d = dsu.new(100)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.dsu.union".to_string(), "collections.dsu.connected".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.dsu.union".to_string(),
            description: "The structure with a's and b's groups merged - by rank, compressing both walked paths.".to_string(),
            syntax: "dsu.union(d, a, b)".to_string(),
            parameters: vec![
                "d: Dsu handle".to_string(),
                "a: Int".to_string(),
                "b: Int".to_string(),
            ],
            return_type: "Dsu handle".to_string(),
            examples: vec!["d = dsu.union(d, 3, 7)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.dsu.connected".to_string(), "collections.dsu.find".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.dsu.find".to_string(),
            description: "The root representative of x's group. A read - follows links without rewriting them.".to_string(),
            syntax: "dsu.find(d, x)".to_string(),
            parameters: vec![
                "d: Dsu handle".to_string(),
                "x: Int".to_string(),
            ],
            return_type: "Int".to_string(),
            examples: vec!["dsu.find(d, 3)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.dsu.connected".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.dsu.connected".to_string(),
            description: "True when a and b are in the same group.".to_string(),
            syntax: "dsu.connected(d, a, b)".to_string(),
            parameters: vec![
                "d: Dsu handle".to_string(),
                "a: Int".to_string(),
                "b: Int".to_string(),
            ],
            return_type: "Bool".to_string(),
            examples: vec!["dsu.connected(d, 3, 7)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.dsu.union".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.dsu.size".to_string(),
            description: "The number of elements (not groups).".to_string(),
            syntax: "dsu.size(d)".to_string(),
            parameters: vec!["d: Dsu handle".to_string()],
            return_type: "Int".to_string(),
            examples: vec!["dsu.size(d)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.dsu.groups".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.dsu.groups".to_string(),
            description: "The number of distinct groups.".to_string(),
            syntax: "dsu.groups(d)".to_string(),
            parameters: vec!["d: Dsu handle".to_string()],
            return_type: "Int".to_string(),
            examples: vec!["dsu.groups(d)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.dsu.union".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.table.new".to_string(),
            description: "A new, empty flat hash table (open addressing, linear probing). Keys are Ints or Strings; values any value. Automatically available. Rebind through every write: t = table.put(t, k, v).".to_string(),
            syntax: "table.new()".to_string(),
            parameters: vec![],
            return_type: "Table handle".to_string(),
            examples: vec!["let mut t = table.new()".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.table.put".to_string(), "collections.table.get".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.table.put".to_string(),
            description:
                "The table with k set to v (inserted or overwritten), amortized O(1) in place."
                    .to_string(),
            syntax: "table.put(t, k, v)".to_string(),
            parameters: vec![
                "t: Table handle".to_string(),
                "k: Int | String".to_string(),
                "v: value".to_string(),
            ],
            return_type: "Table handle".to_string(),
            examples: vec!["t = table.put(t, \"hits\", 1)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![
                "collections.table.get".to_string(),
                "collections.table.remove".to_string(),
            ],
        });
        self.add_function(FunctionDoc {
            name: "collections.table.get".to_string(),
            description: "The value under k, or () when absent (Unit is the absence value; use has to distinguish a stored ()).".to_string(),
            syntax: "table.get(t, k)".to_string(),
            parameters: vec![
                "t: Table handle".to_string(),
                "k: Int | String".to_string(),
            ],
            return_type: "value | Unit".to_string(),
            examples: vec!["table.get(t, \"hits\")".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.table.get_or".to_string(), "collections.table.has".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.table.get_or".to_string(),
            description: "The value under k, or fallback when absent - the counter pattern: t = table.put(t, k, table.get_or(t, k, 0) + 1).".to_string(),
            syntax: "table.get_or(t, k, fallback)".to_string(),
            parameters: vec![
                "t: Table handle".to_string(),
                "k: Int | String".to_string(),
                "fallback: value".to_string(),
            ],
            return_type: "value".to_string(),
            examples: vec!["table.get_or(t, \"hits\", 0)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.table.get".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.table.has".to_string(),
            description: "True when k is present.".to_string(),
            syntax: "table.has(t, k)".to_string(),
            parameters: vec!["t: Table handle".to_string(), "k: Int | String".to_string()],
            return_type: "Bool".to_string(),
            examples: vec!["table.has(t, \"hits\")".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.table.get".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.table.remove".to_string(),
            description: "The table with k removed (a no-op when absent).".to_string(),
            syntax: "table.remove(t, k)".to_string(),
            parameters: vec!["t: Table handle".to_string(), "k: Int | String".to_string()],
            return_type: "Table handle".to_string(),
            examples: vec!["t = table.remove(t, \"hits\")".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.table.put".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.table.size".to_string(),
            description: "The number of live entries.".to_string(),
            syntax: "table.size(t)".to_string(),
            parameters: vec!["t: Table handle".to_string()],
            return_type: "Int".to_string(),
            examples: vec!["table.size(t)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.table.keys".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.table.keys".to_string(),
            description: "The live keys, in slot order (stable between writes).".to_string(),
            syntax: "table.keys(t)".to_string(),
            parameters: vec!["t: Table handle".to_string()],
            return_type: "List".to_string(),
            examples: vec!["table.keys(t)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.table.values".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.table.values".to_string(),
            description: "The live values, in the same slot order as keys.".to_string(),
            syntax: "table.values(t)".to_string(),
            parameters: vec!["t: Table handle".to_string()],
            return_type: "List".to_string(),
            examples: vec!["table.values(t)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.table.keys".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.sort".to_string(),
            description: "The list sorted ascending and stable - iterative merge sort, O(n log n) always. Elements must be mutually comparable. Automatically available.".to_string(),
            syntax: "alg.sort(xs)".to_string(),
            parameters: vec![
                "xs: List".to_string(),
            ],
            return_type: "List".to_string(),
            examples: vec!["alg.sort([3, 1, 2])  // [1, 2, 3]".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.sort_by_key".to_string(), "collections.alg.select_kth".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.sort_by_key".to_string(),
            description: "The list sorted ascending and stable by key_fn(x) - called once per element, never O(n log n) times.".to_string(),
            syntax: "alg.sort_by_key(xs, key_fn)".to_string(),
            parameters: vec![
                "xs: List".to_string(),
                "key_fn: Function - one argument, returns a comparable key".to_string(),
            ],
            return_type: "List".to_string(),
            examples: vec!["alg.sort_by_key(words, (w) => len(w))".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.sort".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.lower_bound".to_string(),
            description: "The first index in sorted xs whose element is >= x - the insertion point. len(xs) when all are smaller.".to_string(),
            syntax: "alg.lower_bound(xs, x)".to_string(),
            parameters: vec![
                "xs: sorted List".to_string(),
                "x: value".to_string(),
            ],
            return_type: "Int".to_string(),
            examples: vec!["alg.lower_bound([1, 3, 3, 7], 3)  // 1".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.upper_bound".to_string(), "collections.alg.bin_search".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.upper_bound".to_string(),
            description: "The first index in sorted xs whose element is > x. With lower_bound, brackets the run equal to x.".to_string(),
            syntax: "alg.upper_bound(xs, x)".to_string(),
            parameters: vec![
                "xs: sorted List".to_string(),
                "x: value".to_string(),
            ],
            return_type: "Int".to_string(),
            examples: vec!["alg.upper_bound([1, 3, 3, 7], 3)  // 3".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.lower_bound".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.bin_search".to_string(),
            description: "The index of x in sorted xs, or () when absent.".to_string(),
            syntax: "alg.bin_search(xs, x)".to_string(),
            parameters: vec!["xs: sorted List".to_string(), "x: value".to_string()],
            return_type: "Int | Unit".to_string(),
            examples: vec!["alg.bin_search([1, 3, 7], 3)  // 1".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.lower_bound".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.select_kth".to_string(),
            description: "The k-th smallest element (0-based) without sorting - quickselect, expected O(n). Medians and percentiles when a full sort is more than the question needs.".to_string(),
            syntax: "alg.select_kth(xs, k)".to_string(),
            parameters: vec![
                "xs: List".to_string(),
                "k: Int".to_string(),
            ],
            return_type: "value".to_string(),
            examples: vec!["alg.select_kth(latencies, len(latencies) / 2)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.sort".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.graph".to_string(),
            description: "A directed graph of n nodes from an edge list, as a flat CSR handle - build once, traverse many times.".to_string(),
            syntax: "alg.graph(n, edges)".to_string(),
            parameters: vec![
                "n: Int - node count".to_string(),
                "edges: List of [u, v] pairs".to_string(),
            ],
            return_type: "Graph handle".to_string(),
            examples: vec!["alg.graph(3, [[0, 1], [1, 2]])".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.bfs".to_string(), "collections.alg.topo_sort".to_string(), "collections.alg.wgraph".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.wgraph".to_string(),
            description: "A weighted directed graph from [u, v, w] triples - the CSR handle with a parallel weight array.".to_string(),
            syntax: "alg.wgraph(n, edges)".to_string(),
            parameters: vec![
                "n: Int".to_string(),
                "edges: List of [u, v, w] triples".to_string(),
            ],
            return_type: "Graph handle".to_string(),
            examples: vec!["alg.wgraph(3, [[0, 1, 4], [1, 2, 1]])".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.dijkstra".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.bfs".to_string(),
            description:
                "Hop distances from src over a graph handle: a list with -1 for unreachable nodes."
                    .to_string(),
            syntax: "alg.bfs(g, src)".to_string(),
            parameters: vec!["g: Graph handle".to_string(), "src: Int".to_string()],
            return_type: "List of Int".to_string(),
            examples: vec!["alg.bfs(g, 0)".to_string()],
            category: "Collections".to_string(),
            see_also: vec![
                "collections.alg.dijkstra".to_string(),
                "collections.alg.topo_sort".to_string(),
            ],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.topo_sort".to_string(),
            description: "The nodes in topological order as Ok(order), or Err(\"cycle\") - a cycle is data about the input, not a bug.".to_string(),
            syntax: "alg.topo_sort(g)".to_string(),
            parameters: vec![
                "g: Graph handle".to_string(),
            ],
            return_type: "Result<List, String>".to_string(),
            examples: vec!["alg.topo_sort(g) |> unwrap".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.bfs".to_string()],
        });
        self.add_function(FunctionDoc {
            name: "collections.alg.dijkstra".to_string(),
            description: "Shortest-path distances from src over a weighted graph (non-negative weights): a list with -1 for unreachable nodes. Runs heap over flat CSR - the collections composed.".to_string(),
            syntax: "alg.dijkstra(g, src)".to_string(),
            parameters: vec![
                "g: wgraph handle".to_string(),
                "src: Int".to_string(),
            ],
            return_type: "List".to_string(),
            examples: vec!["alg.dijkstra(g, 0)".to_string()],
            category: "Collections".to_string(),
            see_also: vec!["collections.alg.bfs".to_string(), "collections.alg.wgraph".to_string()],
        });
    }

    fn add_bigint_functions(&mut self) {
        self.add_function(FunctionDoc {
            name: "bigint.of".to_string(),
            description: "Make an arbitrary-precision integer from an Int or a digit string. BigInt values use the ordinary operators (+ - * / % and comparisons); an Int operand promotes on contact. Floats never mix implicitly.".to_string(),
            syntax: "bigint.of(value)".to_string(),
            parameters: vec![
                "value: Int | String - The integer, or its decimal digits (for values beyond Int range)".to_string(),
            ],
            return_type: "BigInt".to_string(),
            examples: vec![
                "bigint.of(2)  // 2".to_string(),
                "bigint.of(\"123456789012345678901234567890\")".to_string(),
                "bigint.of(2) + 1  // 3 — Int promotes to BigInt".to_string(),
            ],
            category: "BigInt".to_string(),
            see_also: vec!["bigint.parse".to_string(), "bigint.to_int".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "bigint.parse".to_string(),
            description: "Parse a decimal string into a BigInt, as a Result. The twin of bigint.of for data you don't control: a malformed string comes back as Err instead of raising.".to_string(),
            syntax: "bigint.parse(text)".to_string(),
            parameters: vec!["text: String - Decimal digits, optionally signed".to_string()],
            return_type: "Result<BigInt, String>".to_string(),
            examples: vec![
                "bigint.parse(\"340282366920938463463374607431768211456\")  // Ok(2^128)".to_string(),
                "bigint.parse(\"nope\")  // Err(...)".to_string(),
            ],
            category: "BigInt".to_string(),
            see_also: vec!["bigint.of".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "bigint.to_int".to_string(),
            description: "Convert a BigInt back to a 64-bit Int, as a Result — whether the value fits is a property of the data, not the call.".to_string(),
            syntax: "bigint.to_int(b)".to_string(),
            parameters: vec!["b: BigInt | Int - The value to narrow".to_string()],
            return_type: "Result<Int, String>".to_string(),
            examples: vec![
                "bigint.to_int(bigint.of(42))  // Ok(42)".to_string(),
                "bigint.to_int(bigint.pow(bigint.of(2), 100))  // Err(...)".to_string(),
            ],
            category: "BigInt".to_string(),
            see_also: vec!["bigint.to_float".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "bigint.to_float".to_string(),
            description: "Convert a BigInt to a Float, rounding to 53 bits of precision. The only sanctioned door between BigInt and Float — the operators refuse to mix them implicitly.".to_string(),
            syntax: "bigint.to_float(b)".to_string(),
            parameters: vec!["b: BigInt | Int - The value to convert".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "bigint.to_float(bigint.pow(bigint.of(2), 100))  // 1.2676506002282294e30".to_string(),
            ],
            category: "BigInt".to_string(),
            see_also: vec!["bigint.to_int".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "bigint.abs".to_string(),
            description: "The absolute value of a BigInt.".to_string(),
            syntax: "bigint.abs(b)".to_string(),
            parameters: vec!["b: BigInt | Int - The value".to_string()],
            return_type: "BigInt".to_string(),
            examples: vec!["bigint.abs(bigint.of(-7))  // 7".to_string()],
            category: "BigInt".to_string(),
            see_also: vec!["bigint.neg".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "bigint.neg".to_string(),
            description:
                "Negate a BigInt. (Unary minus does not apply to BigInt; 0 - b also works.)"
                    .to_string(),
            syntax: "bigint.neg(b)".to_string(),
            parameters: vec!["b: BigInt | Int - The value".to_string()],
            return_type: "BigInt".to_string(),
            examples: vec!["bigint.neg(bigint.of(7))  // -7".to_string()],
            category: "BigInt".to_string(),
            see_also: vec!["bigint.abs".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "bigint.pow".to_string(),
            description: "Raise a BigInt to a non-negative Int power.".to_string(),
            syntax: "bigint.pow(base, exponent)".to_string(),
            parameters: vec![
                "base: BigInt | Int - The base".to_string(),
                "exponent: Int - Non-negative power".to_string(),
            ],
            return_type: "BigInt".to_string(),
            examples: vec![
                "bigint.pow(bigint.of(2), 200)  // 1606938044258990275541962092341162602522202993782792835301376".to_string(),
            ],
            category: "BigInt".to_string(),
            see_also: vec!["bigint.mod_pow".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "bigint.mod_pow".to_string(),
            description: "Modular exponentiation: base^exponent mod modulus, without materializing base^exponent — the workhorse of number-theoretic and cryptographic code.".to_string(),
            syntax: "bigint.mod_pow(base, exponent, modulus)".to_string(),
            parameters: vec![
                "base: BigInt | Int - The base".to_string(),
                "exponent: BigInt | Int - Non-negative power".to_string(),
                "modulus: BigInt | Int - Non-zero modulus".to_string(),
            ],
            return_type: "BigInt".to_string(),
            examples: vec![
                "bigint.mod_pow(bigint.of(7), bigint.of(560), bigint.of(561))  // 1".to_string(),
            ],
            category: "BigInt".to_string(),
            see_also: vec!["bigint.pow".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "bigint.gcd".to_string(),
            description: "The greatest common divisor of two BigInts (always non-negative)."
                .to_string(),
            syntax: "bigint.gcd(a, b)".to_string(),
            parameters: vec![
                "a: BigInt | Int - First value".to_string(),
                "b: BigInt | Int - Second value".to_string(),
            ],
            return_type: "BigInt".to_string(),
            examples: vec!["bigint.gcd(bigint.of(48), 18)  // 6".to_string()],
            category: "BigInt".to_string(),
            see_also: vec!["bigint.abs".to_string()],
        });
    }

    fn add_math_functions(&mut self) {
        // Mathematical constants (these are fields, not functions, but documented for completeness)
        self.add_function(FunctionDoc {
            name: "math.PI".to_string(),
            description: "Mathematical constant π (pi) ≈ 3.14159".to_string(),
            syntax: "math.PI".to_string(),
            parameters: vec![],
            return_type: "Float".to_string(),
            examples: vec![
                "let circumference = 2 * math.PI * radius".to_string(),
                "math.sin(math.PI / 2)  // Returns 1.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.E".to_string(), "math.TAU".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "math.E".to_string(),
            description: "Mathematical constant e (Euler's number) ≈ 2.71828".to_string(),
            syntax: "math.E".to_string(),
            parameters: vec![],
            return_type: "Float".to_string(),
            examples: vec![
                "math.exp(1)  // Returns math.E".to_string(),
                "math.ln(math.E)  // Returns 1.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec![
                "math.PI".to_string(),
                "math.exp".to_string(),
                "math.ln".to_string(),
            ],
        });

        // Basic math functions
        self.add_function(FunctionDoc {
            name: "math.abs".to_string(),
            description: "Return the absolute value of a number".to_string(),
            syntax: "math.abs(number)".to_string(),
            parameters: vec!["number: Int|Float - The number to get absolute value of".to_string()],
            return_type: "Int|Float".to_string(),
            examples: vec![
                "math.abs(-5)  // Returns 5".to_string(),
                "math.abs(-3.14)  // Returns 3.14".to_string(),
                "math.abs(42)  // Returns 42".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.sign".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "math.min".to_string(),
            description: "Return the smaller of two numbers".to_string(),
            syntax: "math.min(a, b)".to_string(),
            parameters: vec![
                "a: Int|Float - First number".to_string(),
                "b: Int|Float - Second number".to_string(),
            ],
            return_type: "Int|Float".to_string(),
            examples: vec![
                "math.min(5, 3)  // Returns 3".to_string(),
                "math.min(-1, 2)  // Returns -1".to_string(),
                "math.min(3.14, 2.71)  // Returns 2.71".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.max".to_string(), "math.clamp".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "math.max".to_string(),
            description: "Return the larger of two numbers".to_string(),
            syntax: "math.max(a, b)".to_string(),
            parameters: vec![
                "a: Int|Float - First number".to_string(),
                "b: Int|Float - Second number".to_string(),
            ],
            return_type: "Int|Float".to_string(),
            examples: vec![
                "math.max(5, 3)  // Returns 5".to_string(),
                "math.max(-1, 2)  // Returns 2".to_string(),
                "math.max(3.14, 2.71)  // Returns 3.14".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.min".to_string(), "math.clamp".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "math.pow".to_string(),
            description: "Raise a number to a power".to_string(),
            syntax: "math.pow(base, exponent)".to_string(),
            parameters: vec![
                "base: Int|Float - The base number".to_string(),
                "exponent: Int|Float - The exponent".to_string(),
            ],
            return_type: "Float".to_string(),
            examples: vec![
                "math.pow(2, 3)  // Returns 8.0".to_string(),
                "math.pow(4, 0.5)  // Returns 2.0 (square root)".to_string(),
                "math.pow(math.E, 1)  // Returns math.E".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.sqrt".to_string(), "math.exp".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "math.sqrt".to_string(),
            description: "Calculate the square root of a number".to_string(),
            syntax: "math.sqrt(number)".to_string(),
            parameters: vec!["number: Int|Float - The number (must be non-negative)".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.sqrt(16)  // Returns 4.0".to_string(),
                "math.sqrt(2)  // Returns 1.414...".to_string(),
                "math.sqrt(0)  // Returns 0.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.cbrt".to_string(), "math.pow".to_string()],
        });

        // Trigonometric functions
        self.add_function(FunctionDoc {
            name: "math.sin".to_string(),
            description: "Calculate the sine of an angle in radians".to_string(),
            syntax: "math.sin(radians)".to_string(),
            parameters: vec!["radians: Float - Angle in radians".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.sin(0)  // Returns 0.0".to_string(),
                "math.sin(math.PI / 2)  // Returns 1.0".to_string(),
                "math.sin(math.PI)  // Returns 0.0 (approximately)".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec![
                "math.cos".to_string(),
                "math.tan".to_string(),
                "math.asin".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "math.cos".to_string(),
            description: "Calculate the cosine of an angle in radians".to_string(),
            syntax: "math.cos(radians)".to_string(),
            parameters: vec!["radians: Float - Angle in radians".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.cos(0)  // Returns 1.0".to_string(),
                "math.cos(math.PI / 2)  // Returns 0.0 (approximately)".to_string(),
                "math.cos(math.PI)  // Returns -1.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec![
                "math.sin".to_string(),
                "math.tan".to_string(),
                "math.acos".to_string(),
            ],
        });

        // Rounding functions
        self.add_function(FunctionDoc {
            name: "math.floor".to_string(),
            description: "Round down to the nearest integer".to_string(),
            syntax: "math.floor(number)".to_string(),
            parameters: vec!["number: Int|Float - The number to round down".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.floor(3.7)  // Returns 3.0".to_string(),
                "math.floor(-2.3)  // Returns -3.0".to_string(),
                "math.floor(5.0)  // Returns 5.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec![
                "math.ceil".to_string(),
                "math.round".to_string(),
                "math.trunc".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "math.ceil".to_string(),
            description: "Round up to the nearest integer".to_string(),
            syntax: "math.ceil(number)".to_string(),
            parameters: vec!["number: Int|Float - The number to round up".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.ceil(3.2)  // Returns 4.0".to_string(),
                "math.ceil(-2.7)  // Returns -2.0".to_string(),
                "math.ceil(5.0)  // Returns 5.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec![
                "math.floor".to_string(),
                "math.round".to_string(),
                "math.trunc".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "math.round".to_string(),
            description: "Round to the nearest integer".to_string(),
            syntax: "math.round(number)".to_string(),
            parameters: vec!["number: Int|Float - The number to round".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.round(3.6)  // Returns 4.0".to_string(),
                "math.round(3.4)  // Returns 3.0".to_string(),
                "math.round(-2.5)  // Returns -3.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.floor".to_string(), "math.ceil".to_string()],
        });

        // Logarithmic functions
        self.add_function(FunctionDoc {
            name: "math.ln".to_string(),
            description: "Calculate the natural logarithm (base e)".to_string(),
            syntax: "math.ln(number)".to_string(),
            parameters: vec!["number: Int|Float - The number (must be positive)".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.ln(math.E)  // Returns 1.0".to_string(),
                "math.ln(1)  // Returns 0.0".to_string(),
                "math.ln(2.718281828)  // Returns approximately 1.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec![
                "math.log".to_string(),
                "math.log10".to_string(),
                "math.exp".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "math.log10".to_string(),
            description: "Calculate the base-10 logarithm".to_string(),
            syntax: "math.log10(number)".to_string(),
            parameters: vec!["number: Int|Float - The number (must be positive)".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.log10(100)  // Returns 2.0".to_string(),
                "math.log10(1000)  // Returns 3.0".to_string(),
                "math.log10(1)  // Returns 0.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.ln".to_string(), "math.log".to_string()],
        });

        // Utility functions
        self.add_function(FunctionDoc {
            name: "math.degrees".to_string(),
            description: "Convert radians to degrees".to_string(),
            syntax: "math.degrees(radians)".to_string(),
            parameters: vec!["radians: Float - Angle in radians".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.degrees(math.PI)  // Returns 180.0".to_string(),
                "math.degrees(math.PI / 2)  // Returns 90.0".to_string(),
                "math.degrees(0)  // Returns 0.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.radians".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "math.radians".to_string(),
            description: "Convert degrees to radians".to_string(),
            syntax: "math.radians(degrees)".to_string(),
            parameters: vec!["degrees: Float - Angle in degrees".to_string()],
            return_type: "Float".to_string(),
            examples: vec![
                "math.radians(180)  // Returns math.PI".to_string(),
                "math.radians(90)  // Returns math.PI / 2".to_string(),
                "math.radians(0)  // Returns 0.0".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.degrees".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "math.factorial".to_string(),
            description: "Calculate the factorial of a non-negative integer (max 20)".to_string(),
            syntax: "math.factorial(n)".to_string(),
            parameters: vec!["n: Int - Non-negative integer (0 <= n <= 20)".to_string()],
            return_type: "Int".to_string(),
            examples: vec![
                "math.factorial(5)  // Returns 120".to_string(),
                "math.factorial(0)  // Returns 1".to_string(),
                "math.factorial(3)  // Returns 6".to_string(),
            ],
            category: "Math".to_string(),
            see_also: vec!["math.gcd".to_string(), "math.lcm".to_string()],
        });
    }

    /// Add random module documentation
    fn add_random_functions(&mut self) {
        // Basic random generation
        self.add_function(FunctionDoc {
            name: "random.random".to_string(),
            description: "Generate a random float between 0.0 and 1.0 (exclusive)".to_string(),
            syntax: "random.random()".to_string(),
            parameters: vec![],
            return_type: "Float".to_string(),
            examples: vec![
                "random.random()  // Returns 0.123456...".to_string(),
                "let prob = random.random()  // Use for probability".to_string(),
                "if random.random() < 0.5 => \"heads\" else \"tails\"".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec!["random.uniform".to_string(), "random.randbool".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "random.randint".to_string(),
            description: "Generate a random integer between min and max (inclusive)".to_string(),
            syntax: "random.randint(min, max)".to_string(),
            parameters: vec![
                "min: Int - Minimum value (inclusive)".to_string(),
                "max: Int - Maximum value (inclusive)".to_string(),
            ],
            return_type: "Int".to_string(),
            examples: vec![
                "random.randint(1, 6)  // Dice roll: 1-6".to_string(),
                "random.randint(-10, 10)  // Random between -10 and 10".to_string(),
                "random.randint(0, 100)  // Percentage: 0-100".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec!["random.uniform".to_string(), "random.choice".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "random.uniform".to_string(),
            description: "Generate a random float between min and max".to_string(),
            syntax: "random.uniform(min, max)".to_string(),
            parameters: vec![
                "min: Float - Minimum value".to_string(),
                "max: Float - Maximum value".to_string(),
            ],
            return_type: "Float".to_string(),
            examples: vec![
                "random.uniform(0.0, 1.0)  // Same as random.random()".to_string(),
                "random.uniform(-5.0, 5.0)  // Random between -5 and 5".to_string(),
                "random.uniform(98.6, 100.4)  // Body temperature range".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec!["random.random".to_string(), "random.gauss".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "random.randbool".to_string(),
            description: "Generate a random boolean value (true or false)".to_string(),
            syntax: "random.randbool()".to_string(),
            parameters: vec![],
            return_type: "Boolean".to_string(),
            examples: vec![
                "random.randbool()  // Returns true or false".to_string(),
                "if random.randbool() => \"yes\" else \"no\"".to_string(),
                "let flip = random.randbool()  // Coin flip".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec!["random.random".to_string(), "random.choice".to_string()],
        });

        // Random choices
        self.add_function(FunctionDoc {
            name: "random.choice".to_string(),
            description: "Choose a random element from a list".to_string(),
            syntax: "random.choice(list)".to_string(),
            parameters: vec!["list: List[T] - Non-empty list to choose from".to_string()],
            return_type: "T".to_string(),
            examples: vec![
                "random.choice([1, 2, 3, 4])  // Returns one of the numbers".to_string(),
                "random.choice([\"red\", \"green\", \"blue\"])  // Random color".to_string(),
                "let options = [\"rock\", \"paper\", \"scissors\"]; random.choice(options)"
                    .to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec!["random.choices".to_string(), "random.sample".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "random.choices".to_string(),
            description: "Choose multiple random elements with replacement".to_string(),
            syntax: "random.choices(list, k)".to_string(),
            parameters: vec![
                "list: List[T] - List to choose from".to_string(),
                "k: Int - Number of elements to choose".to_string(),
            ],
            return_type: "List[T]".to_string(),
            examples: vec![
                "random.choices([1, 2, 3], 5)  // May return [2, 1, 3, 2, 1]".to_string(),
                "random.choices([\"A\", \"B\", \"C\"], 3)  // Random sequence".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec!["random.choice".to_string(), "random.sample".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "random.sample".to_string(),
            description: "Choose multiple random elements without replacement".to_string(),
            syntax: "random.sample(list, k)".to_string(),
            parameters: vec![
                "list: List[T] - List to sample from".to_string(),
                "k: Int - Number of elements to choose (k <= len(list))".to_string(),
            ],
            return_type: "List[T]".to_string(),
            examples: vec![
                "random.sample([1, 2, 3, 4, 5], 3)  // Returns 3 unique elements".to_string(),
                "random.sample([\"Alice\", \"Bob\", \"Charlie\", \"David\"], 2)  // Pick 2 people"
                    .to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec!["random.choices".to_string(), "random.shuffle".to_string()],
        });

        // Random strings
        self.add_function(FunctionDoc {
            name: "random.randstr".to_string(),
            description: "Generate a random alphanumeric string of specified length".to_string(),
            syntax: "random.randstr(length)".to_string(),
            parameters: vec!["length: Int - Length of the string to generate".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                "random.randstr(8)  // Returns \"a1B2c3D4\" (example)".to_string(),
                "random.randstr(12)  // Generate random password".to_string(),
                "let session_id = random.randstr(16)  // Session identifier".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec![
                "random.randstr_alpha".to_string(),
                "random.randstr_numeric".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "random.randstr_alpha".to_string(),
            description: "Generate a random alphabetic string (a-z, A-Z only)".to_string(),
            syntax: "random.randstr_alpha(length)".to_string(),
            parameters: vec!["length: Int - Length of the string to generate".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                "random.randstr_alpha(6)  // Returns \"AbCdEf\" (example)".to_string(),
                "random.randstr_alpha(10)  // Random name generator".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec![
                "random.randstr".to_string(),
                "random.randstr_numeric".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "random.randstr_numeric".to_string(),
            description: "Generate a random numeric string (0-9 only)".to_string(),
            syntax: "random.randstr_numeric(length)".to_string(),
            parameters: vec!["length: Int - Length of the string to generate".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                "random.randstr_numeric(4)  // Returns \"1234\" (example)".to_string(),
                "random.randstr_numeric(6)  // Generate PIN code".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec![
                "random.randstr".to_string(),
                "random.randstr_alpha".to_string(),
            ],
        });

        // Utility functions
        self.add_function(FunctionDoc {
            name: "random.shuffle".to_string(),
            description: "Return a shuffled copy of a list".to_string(),
            syntax: "random.shuffle(list)".to_string(),
            parameters: vec!["list: List[T] - List to shuffle".to_string()],
            return_type: "List[T]".to_string(),
            examples: vec![
                "random.shuffle([1, 2, 3, 4])  // Returns [3, 1, 4, 2] (example)".to_string(),
                "let deck = [\"A\", \"K\", \"Q\", \"J\"]; random.shuffle(deck)".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec!["random.sample".to_string(), "random.choices".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "random.seed".to_string(),
            description: "Seed the random number generator for reproducible results".to_string(),
            syntax: "random.seed(seed)".to_string(),
            parameters: vec!["seed: Int - Seed value for the random number generator".to_string()],
            return_type: "Nil".to_string(),
            examples: vec![
                "random.seed(12345)  // Set seed for reproducible randomness".to_string(),
                "random.seed(42); random.randint(1, 100)  // Always same result".to_string(),
            ],
            category: "Random".to_string(),
            see_also: vec!["random.random".to_string()],
        });
    }

    /// Add dates module documentation
    fn add_dates_functions(&mut self) {
        // Current date/time functions
        self.add_function(FunctionDoc {
            name: "dates.now".to_string(),
            description: "Get the current local date and time in RFC3339 format".to_string(),
            syntax: "dates.now()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "dates.now()  // \"2024-06-15T14:30:00+00:00\"".to_string(),
                "let current = dates.now(); println(\"Current time: \" + current)".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.utc_now".to_string(), "dates.today".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.utc_now".to_string(),
            description: "Get the current UTC date and time in RFC3339 format".to_string(),
            syntax: "dates.utc_now()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "dates.utc_now()  // \"2024-06-15T14:30:00Z\"".to_string(),
                "let utc_time = dates.utc_now(); println(\"UTC: \" + utc_time)".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.now".to_string(), "dates.today".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.today".to_string(),
            description: "Get today's date in ISO format (YYYY-MM-DD)".to_string(),
            syntax: "dates.today()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "dates.today()  // \"2024-06-15\"".to_string(),
                "println(\"Today is \" + dates.today())".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.now".to_string(), "dates.date".to_string()],
        });

        // Date creation functions
        self.add_function(FunctionDoc {
            name: "dates.date".to_string(),
            description: "Create a date from year, month, and day components".to_string(),
            syntax: "dates.date(year, month, day)".to_string(),
            parameters: vec![
                "year: Int - The year (e.g., 2024)".to_string(),
                "month: Int - The month (1-12)".to_string(),
                "day: Int - The day of month (1-31)".to_string(),
            ],
            return_type: "Result<Date, Error>".to_string(),
            examples: vec![
                "unwrap(dates.date(2024, 6, 15))  // a Date; displays as 2024-06-15".to_string(),
                "let birthday = unwrap(dates.date(1990, 5, 15))".to_string(),
                "unwrap(dates.date(2024, 6, 15)) < unwrap(dates.date(2024, 7, 1))  // true"
                    .to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.parse".to_string(), "dates.datetime".to_string()],
        });

        self.doc(
            "dates.parse",
            "dates.parse(s)",
            "Result",
            "Dates",
            "Parse a Date value from a date or datetime string — the canonical constructor from text; accepts every format this module emits. Date values compare chronologically, subtract to day counts (d2 - d1), and shift by days (d + 7).",
        );

        self.add_function(FunctionDoc {
            name: "dates.datetime".to_string(),
            description: "Create a datetime from year, month, day, hour, minute, second"
                .to_string(),
            syntax: "dates.datetime(year, month, day, hour, minute, second)".to_string(),
            parameters: vec![
                "year: Int - The year".to_string(),
                "month: Int - The month (1-12)".to_string(),
                "day: Int - The day of month (1-31)".to_string(),
                "hour: Int - The hour (0-23)".to_string(),
                "minute: Int - The minute (0-59)".to_string(),
                "second: Int - The second (0-59)".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.datetime(2024, 6, 15, 14, 30, 0)  // \"2024-06-15T14:30:00\"".to_string(),
                "let meeting = dates.datetime(2024, 12, 25, 14, 30, 0)".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.date".to_string(), "dates.time".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.time".to_string(),
            description: "Create a time from hour, minute, and second components".to_string(),
            syntax: "dates.time(hour, minute, second)".to_string(),
            parameters: vec![
                "hour: Int - The hour (0-23)".to_string(),
                "minute: Int - The minute (0-59)".to_string(),
                "second: Int - The second (0-59)".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.time(14, 30, 0)  // \"14:30:00\"".to_string(),
                "let lunch_time = dates.time(12, 30, 0)".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.datetime".to_string(), "dates.date".to_string()],
        });

        // Parsing functions
        self.add_function(FunctionDoc {
            name: "dates.parse_date".to_string(),
            description: "Parse a date string in ISO format (YYYY-MM-DD)".to_string(),
            syntax: "dates.parse_date(date_string)".to_string(),
            parameters: vec!["date_string: String - Date in YYYY-MM-DD format".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                "dates.parse_date(\"2024-06-15\")  // \"2024-06-15\"".to_string(),
                "let parsed = dates.parse_date(\"2024-12-25\")".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec![
                "dates.parse_datetime".to_string(),
                "dates.format_date".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "dates.parse_datetime".to_string(),
            description: "Parse a datetime string in ISO format or RFC3339 format".to_string(),
            syntax: "dates.parse_datetime(datetime_string)".to_string(),
            parameters: vec![
                "datetime_string: String - Datetime in ISO or RFC3339 format".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.parse_datetime(\"2024-06-15T14:30:00\")".to_string(),
                "dates.parse_datetime(\"2024-06-15T14:30:00Z\")".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec![
                "dates.parse_date".to_string(),
                "dates.format_datetime".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "dates.parse_time".to_string(),
            description: "Parse a time string in HH:MM:SS format".to_string(),
            syntax: "dates.parse_time(time_string)".to_string(),
            parameters: vec!["time_string: String - Time in HH:MM:SS format".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                "dates.parse_time(\"14:30:00\")  // \"14:30:00\"".to_string(),
                "let parsed_time = dates.parse_time(\"09:15:30\")".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec![
                "dates.parse_datetime".to_string(),
                "dates.format_time".to_string(),
            ],
        });

        // Formatting functions
        self.add_function(FunctionDoc {
            name: "dates.format_date".to_string(),
            description: "Format a date string using a custom format pattern".to_string(),
            syntax: "dates.format_date(date_string, format_pattern)".to_string(),
            parameters: vec![
                "date_string: String - Date in YYYY-MM-DD format".to_string(),
                "format_pattern: String - Format pattern (e.g., \"%B %d, %Y\")".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.format_date(\"2024-06-15\", \"%B %d, %Y\")  // \"June 15, 2024\""
                    .to_string(),
                "dates.format_date(\"2024-06-15\", \"%d/%m/%Y\")  // \"15/06/2024\"".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec![
                "dates.format_datetime".to_string(),
                "dates.parse_date".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "dates.format_datetime".to_string(),
            description: "Format a datetime string using a custom format pattern".to_string(),
            syntax: "dates.format_datetime(datetime_string, format_pattern)".to_string(),
            parameters: vec![
                "datetime_string: String - Datetime in ISO format".to_string(),
                "format_pattern: String - Format pattern".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.format_datetime(\"2024-06-15T14:30:00\", \"%B %d, %Y at %I:%M %p\")"
                    .to_string(),
                "dates.format_datetime(\"2024-06-15T14:30:00\", \"%Y-%m-%d %H:%M\")".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec![
                "dates.format_date".to_string(),
                "dates.format_time".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "dates.format_time".to_string(),
            description: "Format a time string using a custom format pattern".to_string(),
            syntax: "dates.format_time(time_string, format_pattern)".to_string(),
            parameters: vec![
                "time_string: String - Time in HH:MM:SS format".to_string(),
                "format_pattern: String - Format pattern (e.g., \"%I:%M %p\")".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.format_time(\"14:30:00\", \"%I:%M %p\")  // \"02:30 PM\"".to_string(),
                "dates.format_time(\"09:15:30\", \"%H:%M\")  // \"09:15\"".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec![
                "dates.format_datetime".to_string(),
                "dates.parse_time".to_string(),
            ],
        });

        // Date arithmetic
        self.add_function(FunctionDoc {
            name: "dates.add_days".to_string(),
            description: "Add or subtract days from a date".to_string(),
            syntax: "dates.add_days(date_string, days)".to_string(),
            parameters: vec![
                "date_string: String - Date in YYYY-MM-DD format".to_string(),
                "days: Int - Number of days to add (negative to subtract)".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.add_days(\"2024-06-15\", 7)  // \"2024-06-22\"".to_string(),
                "dates.add_days(\"2024-06-15\", -10)  // \"2024-06-05\"".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec![
                "dates.add_weeks".to_string(),
                "dates.add_months".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "dates.add_weeks".to_string(),
            description: "Add or subtract weeks from a date".to_string(),
            syntax: "dates.add_weeks(date_string, weeks)".to_string(),
            parameters: vec![
                "date_string: String - Date in YYYY-MM-DD format".to_string(),
                "weeks: Int - Number of weeks to add (negative to subtract)".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.add_weeks(\"2024-06-15\", 2)  // \"2024-06-29\"".to_string(),
                "dates.add_weeks(\"2024-06-15\", -1)  // \"2024-06-08\"".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.add_days".to_string(), "dates.add_months".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.add_months".to_string(),
            description: "Add or subtract months from a date".to_string(),
            syntax: "dates.add_months(date_string, months)".to_string(),
            parameters: vec![
                "date_string: String - Date in YYYY-MM-DD format".to_string(),
                "months: Int - Number of months to add (negative to subtract)".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.add_months(\"2024-06-15\", 3)  // \"2024-09-15\"".to_string(),
                "dates.add_months(\"2024-06-15\", -2)  // \"2024-04-15\"".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.add_days".to_string(), "dates.add_years".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.add_years".to_string(),
            description: "Add or subtract years from a date".to_string(),
            syntax: "dates.add_years(date_string, years)".to_string(),
            parameters: vec![
                "date_string: String - Date in YYYY-MM-DD format".to_string(),
                "years: Int - Number of years to add (negative to subtract)".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "dates.add_years(\"2024-06-15\", 1)  // \"2025-06-15\"".to_string(),
                "dates.add_years(\"2024-06-15\", -5)  // \"2019-06-15\"".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.add_months".to_string(), "dates.add_days".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.diff_days".to_string(),
            description: "Calculate the difference in days between two dates".to_string(),
            syntax: "dates.diff_days(date1, date2)".to_string(),
            parameters: vec![
                "date1: String - First date in YYYY-MM-DD format".to_string(),
                "date2: String - Second date in YYYY-MM-DD format".to_string(),
            ],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.diff_days(\"2024-06-22\", \"2024-06-15\")  // 7".to_string(),
                "dates.diff_days(\"2024-06-15\", \"2024-06-22\")  // -7".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.add_days".to_string()],
        });

        // Component extraction
        self.add_function(FunctionDoc {
            name: "dates.year".to_string(),
            description: "Extract the year from a date string".to_string(),
            syntax: "dates.year(date_string)".to_string(),
            parameters: vec!["date_string: String - Date in YYYY-MM-DD format".to_string()],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.year(\"2024-06-15\")  // 2024".to_string(),
                "let birth_year = dates.year(\"1990-05-15\")".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.month".to_string(), "dates.day".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.month".to_string(),
            description: "Extract the month from a date string".to_string(),
            syntax: "dates.month(date_string)".to_string(),
            parameters: vec!["date_string: String - Date in YYYY-MM-DD format".to_string()],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.month(\"2024-06-15\")  // 6".to_string(),
                "let birth_month = dates.month(\"1990-05-15\")  // 5".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.year".to_string(), "dates.day".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.day".to_string(),
            description: "Extract the day of month from a date string".to_string(),
            syntax: "dates.day(date_string)".to_string(),
            parameters: vec!["date_string: String - Date in YYYY-MM-DD format".to_string()],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.day(\"2024-06-15\")  // 15".to_string(),
                "let birth_day = dates.day(\"1990-05-15\")  // 15".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.year".to_string(), "dates.month".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.hour".to_string(),
            description: "Extract the hour from a datetime string".to_string(),
            syntax: "dates.hour(datetime_string)".to_string(),
            parameters: vec!["datetime_string: String - Datetime in ISO format".to_string()],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.hour(\"2024-06-15T14:30:00\")  // 14".to_string(),
                "let meeting_hour = dates.hour(\"2024-06-15T09:30:00\")".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.minute".to_string(), "dates.second".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.minute".to_string(),
            description: "Extract the minute from a datetime string".to_string(),
            syntax: "dates.minute(datetime_string)".to_string(),
            parameters: vec!["datetime_string: String - Datetime in ISO format".to_string()],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.minute(\"2024-06-15T14:30:00\")  // 30".to_string(),
                "let meeting_minute = dates.minute(\"2024-06-15T09:15:00\")".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.hour".to_string(), "dates.second".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.second".to_string(),
            description: "Extract the second from a datetime string".to_string(),
            syntax: "dates.second(datetime_string)".to_string(),
            parameters: vec!["datetime_string: String - Datetime in ISO format".to_string()],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.second(\"2024-06-15T14:30:45\")  // 45".to_string(),
                "let precise_second = dates.second(\"2024-06-15T09:15:30\")".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.hour".to_string(), "dates.minute".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.weekday".to_string(),
            description: "Get the day of the week from a date (0=Sunday, 6=Saturday)".to_string(),
            syntax: "dates.weekday(date_string)".to_string(),
            parameters: vec!["date_string: String - Date in YYYY-MM-DD format".to_string()],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.weekday(\"2024-06-15\")  // 6 (Saturday)".to_string(),
                "dates.weekday(\"2024-06-16\")  // 0 (Sunday)".to_string(),
                "dates.weekday(\"2024-06-17\")  // 1 (Monday)".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.day".to_string()],
        });

        // Utility functions
        self.add_function(FunctionDoc {
            name: "dates.is_leap_year".to_string(),
            description: "Check if a given year is a leap year".to_string(),
            syntax: "dates.is_leap_year(year)".to_string(),
            parameters: vec!["year: Int - The year to check".to_string()],
            return_type: "Bool".to_string(),
            examples: vec![
                "dates.is_leap_year(2024)  // true".to_string(),
                "dates.is_leap_year(2023)  // false".to_string(),
                "dates.is_leap_year(2000)  // true (divisible by 400)".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.days_in_month".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.days_in_month".to_string(),
            description: "Get the number of days in a specific month of a year".to_string(),
            syntax: "dates.days_in_month(year, month)".to_string(),
            parameters: vec![
                "year: Int - The year".to_string(),
                "month: Int - The month (1-12)".to_string(),
            ],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.days_in_month(2024, 2)  // 29 (leap year)".to_string(),
                "dates.days_in_month(2023, 2)  // 28".to_string(),
                "dates.days_in_month(2024, 4)  // 30".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.is_leap_year".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.timestamp".to_string(),
            description: "Convert a datetime string to Unix timestamp".to_string(),
            syntax: "dates.timestamp(datetime_string)".to_string(),
            parameters: vec!["datetime_string: String - Datetime in ISO format".to_string()],
            return_type: "Int".to_string(),
            examples: vec![
                "dates.timestamp(\"2024-06-15T14:30:00\")  // 1718461800".to_string(),
                "let ts = dates.timestamp(\"2024-01-01T00:00:00\")".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.from_timestamp".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "dates.from_timestamp".to_string(),
            description: "Convert a Unix timestamp to datetime string".to_string(),
            syntax: "dates.from_timestamp(timestamp)".to_string(),
            parameters: vec!["timestamp: Int - Unix timestamp (seconds since epoch)".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                "dates.from_timestamp(1718461800)  // \"2024-06-15T14:30:00\"".to_string(),
                "let dt = dates.from_timestamp(1704067200)  // \"2024-01-01T00:00:00\"".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.timestamp".to_string()],
        });
    }

    /// Add REPL command documentation
    fn add_repl_commands(&mut self) {
        // Shell command execution
        self.add_function(FunctionDoc {
            name: ":sh".to_string(),
            description: "Run a command through your shell ($SHELL), streaming its output. `!<command>` is a shortcut for the same thing. `cd` changes the REPL's own working directory. TAB completes file paths in shell commands."
                .to_string(),
            syntax: ":sh <command>   or   !<command>".to_string(),
            parameters: vec![
                "command - Any shell command line (pipes, globs, and quotes work)".to_string(),
            ],
            return_type: "Display".to_string(),
            examples: vec![
                ":sh ls -la              // List files".to_string(),
                "!git status             // Shortcut form".to_string(),
                ":sh cat data.csv | head // Pipes work".to_string(),
                "!cd src                 // Changes the REPL's working directory".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":cd".to_string(), ":pwd".to_string(), ":ls".to_string()],
        });

        self.add_function(FunctionDoc {
            name: ":cd".to_string(),
            description: "Change the REPL's working directory (supports ~ expansion; no argument goes home). Affects relative paths used by fs. functions and shell commands."
                .to_string(),
            syntax: ":cd [directory]".to_string(),
            parameters: vec![
                "directory (optional) - Target directory; defaults to $HOME".to_string(),
            ],
            return_type: "Display".to_string(),
            examples: vec![
                ":cd src                 // Enter a subdirectory".to_string(),
                ":cd ~/projects          // ~ expands to your home directory".to_string(),
                ":cd                     // Go home".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":pwd".to_string(), ":ls".to_string(), ":sh".to_string()],
        });

        self.add_function(FunctionDoc {
            name: ":pwd".to_string(),
            description: "Print the REPL's current working directory".to_string(),
            syntax: ":pwd".to_string(),
            parameters: vec![],
            return_type: "Display".to_string(),
            examples: vec![":pwd                    // /Users/you/projects".to_string()],
            category: "REPL".to_string(),
            see_also: vec![":cd".to_string(), ":ls".to_string()],
        });

        self.add_function(FunctionDoc {
            name: ":ls".to_string(),
            description: "List directory contents (passes arguments through to your system's ls)"
                .to_string(),
            syntax: ":ls [args]".to_string(),
            parameters: vec!["args (optional) - Any arguments your system ls accepts".to_string()],
            return_type: "Display".to_string(),
            examples: vec![
                ":ls                     // List current directory".to_string(),
                ":ls -la src             // Long listing of src/".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":cd".to_string(), ":pwd".to_string(), ":sh".to_string()],
        });

        // Environment Inspection
        self.add_function(FunctionDoc {
            name: ":env".to_string(),
            description:
                "Display all variables, functions, and built-ins in the current session environment"
                    .to_string(),
            syntax: ":env [--full]".to_string(),
            parameters: vec![
                "--full (optional) - Show detailed descriptions for all functions".to_string(),
            ],
            return_type: "Display".to_string(),
            examples: vec![
                ":env                    // Show compact environment overview".to_string(),
                ":env --full             // Show detailed function descriptions".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":clear".to_string(), ":memory".to_string()],
        });

        // History Management
        self.add_function(FunctionDoc {
            name: ":history".to_string(),
            description: "Show, search, and manage command history with numbering for re-execution"
                .to_string(),
            syntax: ":history [<count>|search <pattern>]".to_string(),
            parameters: vec![
                "count (optional) - Number of recent commands to show".to_string(),
                "search <pattern> - Search history for commands containing pattern".to_string(),
            ],
            return_type: "Display".to_string(),
            examples: vec![
                ":history                // Show all command history".to_string(),
                ":history 5              // Show last 5 commands".to_string(),
                ":history search map     // Find commands containing 'map'".to_string(),
                ":!3                     // Re-execute command number 3".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":clear".to_string()],
        });

        // Type Inspection
        self.add_function(FunctionDoc {
            name: ":type".to_string(),
            description: "Inspect the type of any expression without evaluating side effects"
                .to_string(),
            syntax: ":type <expression>".to_string(),
            parameters: vec!["expression - Any valid Olang expression".to_string()],
            return_type: "Type Name".to_string(),
            examples: vec![
                ":type 42                // Int".to_string(),
                ":type [1, 2, 3]         // List[Int]".to_string(),
                ":type (x) => x * 2      // Function: (Int) -> Int".to_string(),
                ":type Person { name: \"Alice\", age: 30 }  // Person".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec!["typeof".to_string(), ":debug".to_string()],
        });

        // Multi-line Input
        self.add_function(FunctionDoc {
            name: ":ml".to_string(),
            description: "Enter multi-line mode for complex expressions and function definitions"
                .to_string(),
            syntax: ":ml".to_string(),
            parameters: vec!["None - Enters multi-line mode until :end is typed".to_string()],
            return_type: "Mode Change".to_string(),
            examples: vec![
                ":ml                     // Enter multi-line mode".to_string(),
                "fn complex(x) =         // Type multi-line function".to_string(),
                "  if x > 10 => \"big\"     // Continue on next line".to_string(),
                "  else => \"small\"       // Continue on next line".to_string(),
                ":end                    // Exit multi-line mode and execute".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":type".to_string()],
        });

        // Session Management
        self.add_function(FunctionDoc {
            name: ":save".to_string(),
            description:
                "Save current session variables and definitions to a file for later loading"
                    .to_string(),
            syntax: ":save <filename>".to_string(),
            parameters: vec!["filename - Path to save session data".to_string()],
            return_type: "File Operation".to_string(),
            examples: vec![
                ":save my_session.ol     // Save current environment".to_string(),
                ":save work/math_utils.ol // Save to subdirectory".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":load".to_string(), ":export".to_string()],
        });

        self.add_function(FunctionDoc {
            name: ":load".to_string(),
            description: "Load previously saved session data from a file into current environment"
                .to_string(),
            syntax: ":load <filename>".to_string(),
            parameters: vec!["filename - Path to session file to load".to_string()],
            return_type: "Environment Update".to_string(),
            examples: vec![
                ":load my_session.ol     // Load saved session".to_string(),
                ":load examples/demo.ol  // Load example definitions".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":save".to_string(), ":env".to_string()],
        });

        // Debug Mode
        self.add_function(FunctionDoc {
            name: ":debug".to_string(),
            description:
                "Toggle debug mode for enhanced error reporting and step-by-step evaluation"
                    .to_string(),
            syntax: ":debug [on|off]".to_string(),
            parameters: vec!["on/off (optional) - Enable or disable debug mode".to_string()],
            return_type: "Mode Change".to_string(),
            examples: vec![
                ":debug                  // Show current debug status".to_string(),
                ":debug on               // Enable debug mode".to_string(),
                ":debug off              // Disable debug mode".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":time".to_string(), ":benchmark".to_string()],
        });

        // Performance Measurement
        self.add_function(FunctionDoc {
            name: ":time".to_string(),
            description: "Measure execution time of expressions for performance analysis"
                .to_string(),
            syntax: ":time <expression>".to_string(),
            parameters: vec!["expression - Any valid Olang expression to time".to_string()],
            return_type: "Result + Timing".to_string(),
            examples: vec![
                ":time range(1000) |> map((x) => x * x)  // Time complex operation".to_string(),
                ":time fibonacci(20)     // Time recursive function".to_string(),
                ":time [1..10000] |> reduce(0, (a,b) => a + b)  // Time reduction".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":benchmark".to_string(), ":memory".to_string()],
        });

        self.add_function(FunctionDoc {
            name: ":memory".to_string(),
            description: "Display memory usage statistics and environment size information"
                .to_string(),
            syntax: ":memory".to_string(),
            parameters: vec!["None".to_string()],
            return_type: "Memory Statistics".to_string(),
            examples: vec![":memory                 // Show current memory usage".to_string()],
            category: "REPL".to_string(),
            see_also: vec![":env".to_string(), ":time".to_string()],
        });

        self.add_function(FunctionDoc {
            name: ":benchmark".to_string(),
            description: "Run performance benchmarks on expressions with statistical analysis"
                .to_string(),
            syntax: ":benchmark <expression>".to_string(),
            parameters: vec![
                "expression - Expression to benchmark across multiple iterations".to_string(),
            ],
            return_type: "Performance Statistics".to_string(),
            examples: vec![
                ":benchmark range(1000) |> map((x) => x * x)  // Benchmark with stats".to_string(),
                ":benchmark sort([3,1,4,1,5,9,2,6,5])        // Benchmark sorting".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":time".to_string(), ":memory".to_string()],
        });

        // Function Discovery
        self.add_function(FunctionDoc {
            name: ":examples".to_string(),
            description:
                "Show interactive examples for any built-in function with option to run them"
                    .to_string(),
            syntax: ":examples <function_name>".to_string(),
            parameters: vec!["function_name - Name of function to show examples for".to_string()],
            return_type: "Interactive Examples".to_string(),
            examples: vec![
                ":examples map           // Show examples for map function".to_string(),
                ":examples filter        // Show examples for filter function".to_string(),
                ":examples range         // Show examples for range function".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":search".to_string(), "help".to_string()],
        });

        self.add_function(FunctionDoc {
            name: ":search".to_string(),
            description: "Search for functions by keyword, description, or signature pattern"
                .to_string(),
            syntax: ":search <query>".to_string(),
            parameters: vec![
                "query - Search term to find in function names or descriptions".to_string(),
            ],
            return_type: "Search Results".to_string(),
            examples: vec![
                ":search list            // Find all list-related functions".to_string(),
                ":search \"transform\"      // Find functions that transform data".to_string(),
                ":search \"List[T] -> Int\" // Find functions with specific signature".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":examples".to_string(), ":help list".to_string()],
        });

        // Environment Management
        self.add_function(FunctionDoc {
            name: ":clear".to_string(),
            description: "Clear screen, user environment, or command history selectively"
                .to_string(),
            syntax: ":clear [env|history]".to_string(),
            parameters: vec![
                "env (optional) - Clear all user-defined variables and functions".to_string(),
                "history (optional) - Clear command history".to_string(),
                "No args - Clear screen only".to_string(),
            ],
            return_type: "Cleanup Operation".to_string(),
            examples: vec![
                ":clear                  // Clear screen".to_string(),
                ":clear env              // Clear user variables".to_string(),
                ":clear history          // Clear command history".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":env".to_string(), ":history".to_string()],
        });

        self.add_function(FunctionDoc {
            name: ":export".to_string(),
            description: "Export current environment to a runnable Olang script file".to_string(),
            syntax: ":export <filename>".to_string(),
            parameters: vec!["filename - Path where to save the generated script".to_string()],
            return_type: "File Generation".to_string(),
            examples: vec![
                ":export my_script.ol    // Export environment as script".to_string(),
                ":export utils/math.ol   // Export to subdirectory".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":save".to_string(), ":env".to_string()],
        });

        // Configuration
        self.add_function(FunctionDoc {
            name: ":config".to_string(),
            description: "Configure REPL behavior including prompts, type display, and debug settings".to_string(),
            syntax: ":config [<setting> <value>]".to_string(),
            parameters: vec![
                "setting - Configuration option: prompt, show_types, debug_mode, time_commands, max_history".to_string(),
                "value - New value for the setting".to_string(),
                "No args - Show current configuration".to_string(),
            ],
            return_type: "Configuration Change".to_string(),
            examples: vec![
                ":config                 // Show all settings".to_string(),
                ":config prompt \"> \"    // Change prompt".to_string(),
                ":config show_types true // Show types with results".to_string(),
                ":config debug_mode true // Enable debug by default".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":debug".to_string(), ":time".to_string()],
        });

        // Re-execution
        self.add_function(FunctionDoc {
            name: ":!<number>".to_string(),
            description: "Re-execute a command from history by its line number".to_string(),
            syntax: ":!<number>".to_string(),
            parameters: vec!["number - Line number from :history command".to_string()],
            return_type: "Command Re-execution".to_string(),
            examples: vec![
                ":!1                     // Re-execute first command in history".to_string(),
                ":!5                     // Re-execute command number 5".to_string(),
            ],
            category: "REPL".to_string(),
            see_also: vec![":history".to_string()],
        });
    }

    fn build_category_index(&mut self) {
        self.categories.clear();
        for (name, func) in &self.functions {
            // A dotted name's category IS its module — derived, so the
            // index cannot drift into near-duplicate hand-written
            // categories ("math" beside "Math", "CSV" beside "csv").
            // Only global builtins keep their curated category strings.
            let category = match name.split_once('.') {
                Some((module, _)) => module.to_string(),
                None => func.category.clone(),
            };
            self.categories
                .entry(category)
                .or_default()
                .push(name.clone());
        }
    }

    /// Show general help overview
    pub fn show_overview(&self) -> String {
        let mut o = String::new();
        let b = Colors::BOLD;
        let r = Colors::RESET;
        let c = Colors::CYAN;
        let y = Colors::YELLOW;
        let d = Colors::DIM;
        o.push_str(&format!("\n{b}olang help{r}\n\n"));
        o.push_str(&format!("  {c}:help <name>{r}     a function's documentation      {d}:help map, :help collections.heap.push{r}\n"));
        o.push_str(&format!("  {c}:help <module>{r}   a module's functions            {d}:help str, :help collections{r}\n"));
        o.push_str(&format!(
            "  {c}:help list{r}       every function, grouped\n"
        ));
        o.push_str(&format!(
            "  {c}:help syntax{r}     language syntax reference\n"
        ));
        o.push_str(&format!("  {c}:help examples{r}   worked examples                 {d}:help tutorials for guided ones{r}\n"));
        o.push_str(&format!(
            "  {c}:help search <q>{r} find functions by keyword\n\n"
        ));
        o.push_str(&format!(
            "{y}Modules{r} {d}(:help <module> to open one){r}\n"
        ));
        o.push_str(&format!("  core       {c}str col math json toml re dates time random crypto base64 bytes bigint{r}\n"));
        o.push_str(&format!(
            "  system     {c}fs os proc http db chan task cell caps meta testing{r}\n"
        ));
        o.push_str(&format!(
            "  data       {c}ods stats plot csv collections{r}\n"
        ));
        o.push_str(&format!(
            "  via use    {c}cli term ui viz dash colx mathx{r}\n\n"
        ));
        o.push_str(&format!("{y}REPL{r}\n"));
        o.push_str(&format!("  {c}:env{r} variables    {c}:type <expr>{r} a value's type    {c}:time <expr>{r} wall-clock an expression\n"));
        o.push_str(&format!("  {c}:history{r} this session    {c}:clear{r} screen or environment    {c}:cd :pwd :ls{r} filesystem\n"));
        o.push_str(&format!("  {c}:sh <cmd>{r} or {c}!<cmd>{r} shell    {c}TAB{r} completes    {c}it{r} holds the last printed result\n\n"));
        o.push_str(&format!("Anything without a leading colon is olang: {c}use collections {{ heap }}{r}, {c}let x = 1{r}, {c}1 + 1{r}.\n"));
        o
    }

    /// Format detailed documentation for a function
    fn format_function_documentation(&self, func: &FunctionDoc) -> String {
        let mut output = format!(
            "\n{}═══ Function: {}{} ═══{}\n\n",
            Colors::BOLD,
            Colors::CYAN,
            func.name,
            Colors::RESET
        );

        output.push_str(&format!(
            "{}Description:{} {}\n",
            Colors::YELLOW,
            Colors::RESET,
            func.description
        ));

        output.push_str(&format!(
            "{}Category:   {} {}\n",
            Colors::YELLOW,
            Colors::RESET,
            func.category
        ));

        output.push_str(&format!(
            "{}Syntax:     {} {}{}{}\n",
            Colors::YELLOW,
            Colors::RESET,
            Colors::CYAN,
            func.syntax,
            Colors::RESET
        ));

        output.push_str(&format!(
            "{}Returns:    {} {}{}{}\n",
            Colors::YELLOW,
            Colors::RESET,
            Colors::GREEN,
            func.return_type,
            Colors::RESET
        ));

        if !func.parameters.is_empty() {
            output.push_str(&format!("\n{}Parameters:{}\n", Colors::BOLD, Colors::RESET));
            for param in &func.parameters {
                output.push_str(&format!("  • {}\n", param));
            }
        }

        if !func.examples.is_empty() {
            output.push_str(&format!("\n{}Examples:{}\n", Colors::BOLD, Colors::RESET));
            for example in &func.examples {
                output.push_str(&format!(
                    "  {}>>{} {}{}{}\n",
                    Colors::GREEN,
                    Colors::RESET,
                    Colors::CYAN,
                    example,
                    Colors::RESET
                ));
            }
        }

        if !func.see_also.is_empty() {
            output.push_str(&format!(
                "\n{}See Also:{} {}\n",
                Colors::DIM,
                Colors::RESET,
                func.see_also.join(", ")
            ));
        }

        output
    }

    /// List all available functions
    pub fn list_functions(&self) -> String {
        let mut output = format!(
            "{}=== All Available Functions ==={}\n\n",
            Colors::BOLD,
            Colors::RESET
        );

        let mut categories: Vec<_> = self.categories.keys().collect();
        categories.sort();

        for category in categories {
            if let Some(functions) = self.categories.get(category) {
                output.push_str(&format!(
                    "{}{}:{}\n",
                    Colors::YELLOW,
                    category,
                    Colors::RESET
                ));

                let mut sorted_functions = functions.clone();
                sorted_functions.sort();

                for func_name in sorted_functions {
                    if let Some(func) = self.functions.get(&func_name) {
                        output.push_str(&format!(
                            "  {}{}{} - {}\n",
                            Colors::BLUE,
                            func_name,
                            Colors::RESET,
                            func.description
                        ));
                    }
                }
                output.push('\n');
            }
        }

        output.push_str(&format!(
            "\n{}Total: {} functions available{}",
            Colors::DIM,
            self.functions.len(),
            Colors::RESET
        ));

        output
    }

    /// Show practical examples  
    pub fn show_examples(&self) -> String {
        format!(
            "{}=== Olang Examples ==={}

{}Basic Operations:{}
  2 + 3 * 4                    // Arithmetic: 14
  \"Hello \" + \"World!\"         // String concatenation
  [1, 2, 3] + [4, 5]           // List concatenation

{}Working with Lists:{}
  let numbers = [1, 2, 3, 4, 5]
  numbers |> map((x) => x * 2)           // [2, 4, 6, 8, 10]
  numbers |> filter((x) => x > 3)        // [4, 5]
  numbers |> reduce(0, (a, b) => a + b)  // 15

{}Function Definition:{}
  fn square(x) = x * x
  fn factorial(n) = if n <= 1 => 1 else n * factorial(n - 1)

{}Standard Library Usage:{}
  math.sqrt(16)                // 4.0
  math.pow(2, 3)              // 8.0
  random.randint(1, 10)       // Random number 1-10
  fs.exists(\"myfile.txt\")     // Check if file exists

{}HTTP Requests:{}
  http.get(\"https://api.github.com/users/octocat\")
  http.post(\"https://httpbin.org/post\", {{ \"key\": \"value\" }})

{}Error Handling:{}
  try {{
    let result = risky_operation()
    println(\"Success: \" + result)
  }} catch (error) {{
    println(\"Error: \" + error)
  }}

For more examples on specific functions, use: {}:help <function_name>{}",
            Colors::BOLD,
            Colors::RESET,
            Colors::GREEN,
            Colors::RESET,
            Colors::CYAN,
            Colors::RESET,
            Colors::YELLOW,
            Colors::RESET,
            Colors::MAGENTA,
            Colors::RESET,
            Colors::BLUE,
            Colors::RESET,
            Colors::RED,
            Colors::RESET,
            Colors::BLUE,
            Colors::RESET
        )
    }

    /// Show syntax reference
    pub fn show_syntax(&self) -> String {
        format!(
            "{}=== Olang Syntax Reference ==={}

{}Variables:{}
  let name = \"Alice\"          // Immutable binding
  let mut age = 25             // Reassignable binding
  age = 26                     // Reassignment (requires `let mut`)

{}Functions:{}
  fn add(a, b) = a + b                       // One expression
  fn dist(a, b) = {{ let d = b - a  d * d }}   // A block; last expression is the value
  fn label(n: Int) -> String = `#${{n}}`       // With type annotations

{}Lambdas:{}
  (x) => x * 2                 // Single parameter
  (a, b) => a + b              // Multiple parameters
  () => \"Hello\"                // No parameters
  let double = (x) => x * 2    // Bind one to a name

{}Control Flow:{}
  if cond => value1 else => value2
  if cond => {{ ... }} else => {{ ... }}
  for item in list {{ ... }}
  while cond {{ ... }}

{}Lists, Tuples, Ranges:{}
  [1, 2, 3, 4]                 // List
  (\"Alice\", 25, true)          // Tuple
  1..10                        // Range (exclusive); 1..=10 inclusive

{}Pipeline Operator:{}
  data |> map((x) => x * 2) |> filter((x) => x > 10) |> fold(0, (a, x) => a + x)

{}Pattern Matching:{}
  match value {{
    Ok(result) => \"Success: \" + result,
    Err(error) => \"Error: \" + error
  }}

{}Type Annotations:{}
  let count: Int = 3
  fn process(items: List<Int>) -> Int = len(items)

{}Strings:{}
  `hello ${{name}}`              // Template string with interpolation

{}Comments:{}
  // Single line comment
  /* Multi-line
     comment */

{}Modules:{}
  use collections {{ heap, table }}   // import from a module
  share fn helper(x) = x + 1         // export from this module

For function-specific syntax, use: {}:help <function_name>{}",
            Colors::BOLD,
            Colors::RESET,
            Colors::GREEN,
            Colors::RESET,
            Colors::CYAN,
            Colors::RESET,
            Colors::YELLOW,
            Colors::RESET,
            Colors::MAGENTA,
            Colors::RESET,
            Colors::BLUE,
            Colors::RESET,
            Colors::RED,
            Colors::RESET,
            Colors::DIM,
            Colors::RESET,
            Colors::GREEN,
            Colors::RESET,
            Colors::MAGENTA,
            Colors::RESET,
            Colors::CYAN,
            Colors::RESET,
            Colors::YELLOW,
            Colors::RESET,
            Colors::BLUE,
            Colors::RESET
        )
    }

    /// Get all function names (for tab completion)
    pub fn get_function_names(&self) -> Vec<String> {
        self.functions.keys().cloned().collect()
    }

    /// One function's documentation, by its registered name (bare for
    /// globals — `"len"` — and module-qualified otherwise — `"str.trim"`).
    /// The language server renders hovers, completions, and signature
    /// help from these entries, so `:help`, the book, and the editor all
    /// speak from the same registry.
    pub fn get_function(&self, name: &str) -> Option<&FunctionDoc> {
        self.functions.get(name)
    }

    /// Qualified names of every stdlib function documented as returning a
    /// `Result` (`"fs.write_file"`, `"str.parse_int"`, ...).
    ///
    /// The checker uses this to spot a discarded failure. The help registry
    /// is the right source: it already records each function's return type,
    /// a coverage test keeps it complete for every callable module, and it
    /// is the same text `:help` shows — so the lint and the documentation
    /// cannot disagree about which functions can fail.
    pub fn result_returning_functions(&self) -> std::collections::HashSet<String> {
        self.functions
            .values()
            .filter(|d| d.return_type.starts_with("Result"))
            .map(|d| d.name.clone())
            .collect()
    }

    /// Get all category names
    pub fn get_category_names(&self) -> Vec<String> {
        self.categories.keys().cloned().collect()
    }

    /// Add CSV functions to the help system  
    fn add_csv_functions(&mut self) {
        // Core parsing and serialization
        self.add_function(FunctionDoc {
            name: "csv.parse".to_string(),
            description: "Parse a CSV string into a 2D array (list of lists) for programmatic access".to_string(),
            syntax: "csv.parse(csv_string)".to_string(),
            parameters: vec!["csv_string: String - Valid CSV string to parse".to_string()],
            return_type: "Result<List<List<String>>, Error>".to_string(),
            examples: vec![
                "csv.parse(\"name,age\\nAlice,30\\nBob,25\")  // Ok([[\"name\", \"age\"], [\"Alice\", \"30\"], [\"Bob\", \"25\"]])".to_string(),
                "csv.parse(\"a,b,c\\n1,2,3\")  // Ok([[\"a\", \"b\", \"c\"], [\"1\", \"2\", \"3\"]])".to_string(),
            ],
            category: "CSV".to_string(),
            see_also: vec!["csv.parse_with_headers".to_string(), "csv.stringify".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "csv.parse_with_headers".to_string(),
            description: "Parse CSV string with headers into objects with named fields".to_string(),
            syntax: "csv.parse_with_headers(csv_string)".to_string(),
            parameters: vec!["csv_string: String - CSV string with header row".to_string()],
            return_type: "Result<List<Object>, Error>".to_string(),
            examples: vec![
                "csv.parse_with_headers(\"name,age\\nAlice,30\\nBob,25\")".to_string(),
                "// Ok([{name: \"Alice\", age: \"30\"}, {name: \"Bob\", age: \"25\"}])".to_string(),
            ],
            category: "CSV".to_string(),
            see_also: vec![
                "csv.parse".to_string(),
                "csv.stringify_with_headers".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "csv.stringify".to_string(),
            description: "Convert a 2D array (list of lists) to CSV string representation".to_string(),
            syntax: "csv.stringify(data)".to_string(),
                        parameters: vec!["data: List<List<String>> - 2D array to convert to CSV".to_string()],
            return_type: "String".to_string(),
                     examples: vec![
            "csv.stringify([[\"name\", \"age\"], [\"Alice\", \"30\"]])  // \"name,age\\nAlice,30\\n\"".to_string(),
            "csv.stringify([[\"a\", \"b\"], [\"1\", \"2\"]])  // CSV string output".to_string(),
        ],
            category: "CSV".to_string(),
            see_also: vec!["csv.parse".to_string(), "csv.stringify_with_headers".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "csv.stringify_with_headers".to_string(),
            description: "Convert objects to CSV string with specified headers".to_string(),
            syntax: "csv.stringify_with_headers(objects, headers)".to_string(),
            parameters: vec![
                "objects: List<Object> - Objects to convert".to_string(),
                "headers: List<String> - Column headers".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "csv.stringify_with_headers([{name: \"Alice\", age: 30}], [\"name\", \"age\"])"
                    .to_string(),
                "// Ok(\"name,age\\nAlice,30\\n\")".to_string(),
            ],
            category: "CSV".to_string(),
            see_also: vec![
                "csv.parse_with_headers".to_string(),
                "csv.stringify".to_string(),
            ],
        });

        // Reading operations
        self.add_function(FunctionDoc {
            name: "csv.read_row".to_string(),
            description: "Read a specific row from CSV data by index".to_string(),
            syntax: "csv.read_row(csv_data, index)".to_string(),
            parameters: vec![
                "csv_data: List<List<String>> - Parsed CSV data".to_string(),
                "index: Int - Row index (0-based)".to_string(),
            ],
            return_type: "Result<List<String>, Error>".to_string(),
            examples: vec![
                "csv.read_row(parsed_csv, 0)  // Ok([\"header1\", \"header2\"])".to_string(),
                "csv.read_row(parsed_csv, 1)  // Ok([\"value1\", \"value2\"])".to_string(),
            ],
            category: "CSV".to_string(),
            see_also: vec!["csv.read_column".to_string(), "csv.read_cell".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "csv.read_column".to_string(),
            description: "Read a specific column from CSV data by index".to_string(),
            syntax: "csv.read_column(csv_data, index)".to_string(),
            parameters: vec![
                "csv_data: List<List<String>> - Parsed CSV data".to_string(),
                "index: Int - Column index (0-based)".to_string(),
            ],
            return_type: "[String]".to_string(),
            examples: vec![
                "csv.read_column(parsed_csv, 0)  // First column values".to_string(),
                "csv.read_column(parsed_csv, 1)  // Second column values".to_string(),
            ],
            category: "CSV".to_string(),
            see_also: vec!["csv.read_row".to_string(), "csv.read_cell".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "csv.read_cell".to_string(),
            description: "Read a specific cell from CSV data by row and column index".to_string(),
            syntax: "csv.read_cell(csv_data, row, column)".to_string(),
            parameters: vec![
                "csv_data: List<List<String>> - Parsed CSV data".to_string(),
                "row: Int - Row index (0-based)".to_string(),
                "column: Int - Column index (0-based)".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "csv.read_cell(parsed_csv, 1, 0)  // Cell at row 1, column 0".to_string(),
                "csv.read_cell(parsed_csv, 0, 1)  // Cell at row 0, column 1".to_string(),
            ],
            category: "CSV".to_string(),
            see_also: vec!["csv.read_row".to_string(), "csv.read_column".to_string()],
        });

        // Utility operations
        self.add_function(FunctionDoc {
            name: "csv.get_headers".to_string(),
            description: "Get the header row (first row) from CSV data".to_string(),
            syntax: "csv.get_headers(csv_data)".to_string(),
            parameters: vec!["csv_data: List<List<String>> - Parsed CSV data".to_string()],
            return_type: "Result<List<String>, Error>".to_string(),
            examples: vec![
                "csv.get_headers(parsed_csv)  // Ok([\"name\", \"age\", \"city\"])".to_string(),
            ],
            category: "CSV".to_string(),
            see_also: vec![
                "csv.parse_with_headers".to_string(),
                "csv.row_count".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "csv.row_count".to_string(),
            description: "Get the number of rows in CSV data".to_string(),
            syntax: "csv.row_count(csv_data)".to_string(),
            parameters: vec!["csv_data: List<List<String>> - Parsed CSV data".to_string()],
            return_type: "Int".to_string(),
            examples: vec!["csv.row_count(parsed_csv)  // Ok(5)  // 5 rows total".to_string()],
            category: "CSV".to_string(),
            see_also: vec!["csv.column_count".to_string(), "len".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "csv.column_count".to_string(),
            description: "Get the number of columns in CSV data (based on first row)".to_string(),
            syntax: "csv.column_count(csv_data)".to_string(),
            parameters: vec!["csv_data: List<List<String>> - Parsed CSV data".to_string()],
            return_type: "Result<Int, Error>".to_string(),
            examples: vec!["csv.column_count(parsed_csv)  // Ok(3)  // 3 columns".to_string()],
            category: "CSV".to_string(),
            see_also: vec!["csv.row_count".to_string(), "csv.get_headers".to_string()],
        });
    }

    /// Add JSON functions to the help system  
    fn add_json_functions(&mut self) {
        // Core parsing and serialization
        self.add_function(FunctionDoc {
            name: "json.parse".to_string(),
            description: "Parse a JSON string into Olang values with comprehensive type mapping".to_string(),
            syntax: "json.parse(json_string)".to_string(),
            parameters: vec!["json_string: String - Valid JSON string to parse".to_string()],
            return_type: "Result<Value, Error>".to_string(),
            examples: vec![
                "json.parse(\"{\\\"name\\\": \\\"John\\\", \\\"age\\\": 30}\")  // Ok({name: \"John\", age: 30})".to_string(),
                "json.parse(\"[1, 2, 3, true, null]\")  // Ok([1, 2, 3, true, ()])".to_string(),
                "json.parse(\"invalid json\")  // Err(\"JSON parse error: ...\")".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.stringify".to_string(), "json.validate".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.stringify".to_string(),
            description: "Convert Olang values to JSON string representation".to_string(),
            syntax: "json.stringify(value)".to_string(),
            parameters: vec!["value: Any - Olang value to convert to JSON".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "json.stringify({name: \"Alice\", age: 25})  // Ok(\"{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":25}\")".to_string(),
                "json.stringify([1, 2, 3])  // Ok(\"[1,2,3]\")".to_string(),
                "json.stringify(true)  // Ok(\"true\")".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.parse".to_string(), "json.prettify".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.prettify".to_string(),
            description: "Format JSON string with indentation for better readability".to_string(),
            syntax: "json.prettify(json_string)".to_string(),
            parameters: vec!["json_string: String - JSON string to format".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "json.prettify(\"{\\\"name\\\":\\\"John\\\",\\\"age\\\":30}\")".to_string(),
                "// Returns formatted JSON with indentation".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.minify".to_string(), "json.stringify".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.minify".to_string(),
            description: "Remove all unnecessary whitespace from JSON string".to_string(),
            syntax: "json.minify(json_string)".to_string(),
            parameters: vec!["json_string: String - JSON string to minify".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "json.minify(\"{\\n  \\\"name\\\": \\\"John\\\",\\n  \\\"age\\\": 30\\n}\")"
                    .to_string(),
                "// Returns: \"{\\\"name\\\":\\\"John\\\",\\\"age\\\":30}\"".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.prettify".to_string(), "json.validate".to_string()],
        });

        // Validation and inspection
        self.add_function(FunctionDoc {
            name: "json.validate".to_string(),
            description: "Check if a string is valid JSON without parsing it".to_string(),
            syntax: "json.validate(json_string)".to_string(),
            parameters: vec!["json_string: String - String to validate".to_string()],
            return_type: "Bool".to_string(),
            examples: vec![
                "json.validate(\"{\\\"name\\\": \\\"John\\\"}\")  // true".to_string(),
                "json.validate(\"invalid json\")  // false".to_string(),
                "json.validate(\"null\")  // true".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.parse".to_string(), "json.get_type".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.get_type".to_string(),
            description:
                "Get the JSON type of a JSON value (null, boolean, number, string, array, object)"
                    .to_string(),
            syntax: "json.get_type(json_string)".to_string(),
            parameters: vec!["json_string: String - JSON string to inspect".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "json.get_type(\"{\\\"name\\\": \\\"John\\\"}\")  // Ok(\"object\")".to_string(),
                "json.get_type(\"[1, 2, 3]\")  // Ok(\"array\")".to_string(),
                "json.get_type(\"42\")  // Ok(\"number\")".to_string(),
                "json.get_type(\"null\")  // Ok(\"null\")".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.validate".to_string(), "typeof".to_string()],
        });

        // Object operations
        self.add_function(FunctionDoc {
            name: "json.has_key".to_string(),
            description: "Check if a JSON object contains a specific key".to_string(),
            syntax: "json.has_key(json_object, key)".to_string(),
            parameters: vec![
                "json_object: String - JSON object string".to_string(),
                "key: String - Key to check for".to_string(),
            ],
            return_type: "Result<Bool, Error>".to_string(),
            examples: vec![
                "json.has_key(\"{\\\"name\\\": \\\"John\\\", \\\"age\\\": 30}\", \"name\")  // Ok(true)".to_string(),
                "json.has_key(\"{\\\"name\\\": \\\"John\\\"}\", \"email\")  // Ok(false)".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.get".to_string(), "json.get_keys".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.get_keys".to_string(),
            description: "Get all keys from a JSON object as a list".to_string(),
            syntax: "json.get_keys(json_object)".to_string(),
            parameters: vec!["json_object: String - JSON object string".to_string()],
            return_type: "Result<List<String>, Error>".to_string(),
            examples: vec![
                "json.get_keys(\"{\\\"name\\\": \\\"John\\\", \\\"age\\\": 30}\")  // Ok([\"name\", \"age\"])".to_string(),
                "json.get_keys(\"{}\")  // Ok([])".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.get_values".to_string(), "json.has_key".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.get_values".to_string(),
            description: "Get all values from a JSON object as a list".to_string(),
            syntax: "json.get_values(json_object)".to_string(),
            parameters: vec!["json_object: String - JSON object string".to_string()],
            return_type: "Result<List<Value>, Error>".to_string(),
            examples: vec![
                "json.get_values(\"{\\\"name\\\": \\\"John\\\", \\\"age\\\": 30}\")  // Ok([\"John\", 30])".to_string(),
                "json.get_values(\"{}\")  // Ok([])".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.get_keys".to_string(), "json.get".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.get".to_string(),
            description: "Get a specific value from a JSON object by key".to_string(),
            syntax: "json.get(json_object, key)".to_string(),
            parameters: vec![
                "json_object: String - JSON object string".to_string(),
                "key: String - Key to retrieve value for".to_string(),
            ],
            return_type: "Result<Value, Error>".to_string(),
            examples: vec![
                "json.get(\"{\\\"name\\\": \\\"John\\\", \\\"age\\\": 30}\", \"name\")  // Ok(\"John\")".to_string(),
                "json.get(\"{\\\"data\\\": [1, 2, 3]}\", \"data\")  // Ok([1, 2, 3])".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.set".to_string(), "json.has_key".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.set".to_string(),
            description: "Set a value in a JSON object, creating or updating the key".to_string(),
            syntax: "json.set(json_object, key, value)".to_string(),
            parameters: vec![
                "json_object: String - JSON object string".to_string(),
                "key: String - Key to set".to_string(),
                "value: Any - Value to set for the key".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "json.set(\"{\\\"name\\\": \\\"John\\\"}\", \"age\", 30)".to_string(),
                "// Returns: \"{\\\"name\\\":\\\"John\\\",\\\"age\\\":30}\"".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.get".to_string(), "json.remove".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.remove".to_string(),
            description: "Remove a key-value pair from a JSON object".to_string(),
            syntax: "json.remove(json_object, key)".to_string(),
            parameters: vec![
                "json_object: String - JSON object string".to_string(),
                "key: String - Key to remove".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "json.remove(\"{\\\"name\\\": \\\"John\\\", \\\"age\\\": 30}\", \"age\")"
                    .to_string(),
                "// Returns: \"{\\\"name\\\":\\\"John\\\"}\"".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.set".to_string(), "json.has_key".to_string()],
        });

        // Array operations
        self.add_function(FunctionDoc {
            name: "json.array_get".to_string(),
            description: "Get an element from a JSON array by index".to_string(),
            syntax: "json.array_get(json_array, index)".to_string(),
            parameters: vec![
                "json_array: String - JSON array string".to_string(),
                "index: Int - Zero-based index to retrieve".to_string(),
            ],
            return_type: "Result<Value, Error>".to_string(),
            examples: vec![
                "json.array_get(\"[\\\"a\\\", \\\"b\\\", \\\"c\\\"]\", 1)  // Ok(\"b\")"
                    .to_string(),
                "json.array_get(\"[10, 20, 30]\", 0)  // Ok(10)".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec![
                "json.array_length".to_string(),
                "json.array_push".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "json.array_length".to_string(),
            description: "Get the length of a JSON array".to_string(),
            syntax: "json.array_length(json_array)".to_string(),
            parameters: vec!["json_array: String - JSON array string".to_string()],
            return_type: "Result<Int, Error>".to_string(),
            examples: vec![
                "json.array_length(\"[1, 2, 3, 4]\")  // Ok(4)".to_string(),
                "json.array_length(\"[]\")  // Ok(0)".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.array_get".to_string(), "len".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.array_push".to_string(),
            description: "Add an element to the end of a JSON array".to_string(),
            syntax: "json.array_push(json_array, value)".to_string(),
            parameters: vec![
                "json_array: String - JSON array string".to_string(),
                "value: Any - Value to add to the array".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "json.array_push(\"[1, 2, 3]\", 4)  // Ok(\"[1,2,3,4]\")".to_string(),
                "json.array_push(\"[]\", \"hello\")  // Ok(\"[\\\"hello\\\"]\")".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec![
                "json.array_get".to_string(),
                "json.array_length".to_string(),
            ],
        });

        // Utility functions
        self.add_function(FunctionDoc {
            name: "json.merge".to_string(),
            description:
                "Merge two JSON objects, with the second object's values taking precedence"
                    .to_string(),
            syntax: "json.merge(json_object1, json_object2)".to_string(),
            parameters: vec![
                "json_object1: String - First JSON object".to_string(),
                "json_object2: String - Second JSON object (values take precedence)".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "json.merge(\"{\\\"a\\\": 1, \\\"b\\\": 2}\", \"{\\\"b\\\": 3, \\\"c\\\": 4}\")"
                    .to_string(),
                "// Returns: \"{\\\"a\\\":1,\\\"b\\\":3,\\\"c\\\":4}\"".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.set".to_string(), "json.deep_clone".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "json.deep_clone".to_string(),
            description: "Create a deep copy of a JSON value".to_string(),
            syntax: "json.deep_clone(json_string)".to_string(),
            parameters: vec!["json_string: String - JSON string to clone".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "json.deep_clone(\"{\\\"nested\\\": {\\\"value\\\": 42}}\")".to_string(),
                "// Returns identical JSON string".to_string(),
            ],
            category: "JSON".to_string(),
            see_also: vec!["json.parse".to_string(), "json.stringify".to_string()],
        });
    }

    /// Add testing function documentation
    fn add_testing_functions(&mut self) {
        // Basic assertion functions
        self.add_function(FunctionDoc {
            name: "testing.assert_eq".to_string(),
            description: "Assert that two values are equal, failing the test if they differ"
                .to_string(),
            syntax: "testing.assert_eq(expected, actual)".to_string(),
            parameters: vec![
                "expected: Any - The expected value".to_string(),
                "actual: Any - The actual value to compare".to_string(),
            ],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "testing.assert_eq(42, 42)  // Ok(Unit)".to_string(),
                "testing.assert_eq(\"hello\", \"hello\")  // Ok(Unit)".to_string(),
                "testing.assert_eq([1, 2, 3], [1, 2, 3])  // Ok(Unit)".to_string(),
                "testing.assert_eq(42, 24)  // Err(\"Assertion failed: expected 42 but got 24\")"
                    .to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec![
                "testing.assert_ne".to_string(),
                "testing.assert_true".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "testing.assert_ne".to_string(),
            description: "Assert that two values are not equal, failing the test if they are equal".to_string(),
            syntax: "testing.assert_ne(value1, value2)".to_string(),
            parameters: vec![
                "value1: Any - The first value".to_string(),
                "value2: Any - The second value to compare".to_string(),
            ],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "testing.assert_ne(42, 24)  // Ok(Unit)".to_string(),
                "testing.assert_ne(\"hello\", \"world\")  // Ok(Unit)".to_string(),
                "testing.assert_ne(42, 42)  // Err(\"Assertion failed: expected 42 to not equal 42\")".to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec!["testing.assert_eq".to_string(), "testing.assert_false".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "testing.assert_true".to_string(),
            description: "Assert that a condition is true, failing the test if it is false".to_string(),
            syntax: "testing.assert_true(condition)".to_string(),
            parameters: vec!["condition: Bool - The condition to test".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "testing.assert_true(true)  // Ok(Unit)".to_string(),
                "testing.assert_true(5 > 3)  // Ok(Unit)".to_string(),
                "testing.assert_true(false)  // Err(\"Assertion failed: expected true but got false\")".to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec!["testing.assert_false".to_string(), "testing.assert_eq".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "testing.assert_false".to_string(),
            description: "Assert that a condition is false, failing the test if it is true".to_string(),
            syntax: "testing.assert_false(condition)".to_string(),
            parameters: vec!["condition: Bool - The condition to test".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "testing.assert_false(false)  // Ok(Unit)".to_string(),
                "testing.assert_false(3 > 5)  // Ok(Unit)".to_string(),
                "testing.assert_false(true)  // Err(\"Assertion failed: expected false but got true\")".to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec!["testing.assert_true".to_string(), "testing.assert_ne".to_string()],
        });

        // Result assertion functions
        self.add_function(FunctionDoc {
            name: "testing.assert_ok".to_string(),
            description: "Assert that a Result value is Ok, failing the test if it is Err".to_string(),
            syntax: "testing.assert_ok(result)".to_string(),
            parameters: vec!["result: Result<T, E> - The Result value to test".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "testing.assert_ok(Ok(42))  // Ok(Unit)".to_string(),
                "testing.assert_ok(fs.read_file(\"existing_file.txt\"))  // Ok(Unit) if file exists".to_string(),
                "testing.assert_ok(Err(\"error\"))  // Err(\"Assertion failed: expected Ok but got Err(error)\")".to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec!["testing.assert_err".to_string(), "testing.assert_eq".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "testing.assert_err".to_string(),
            description: "Assert that a Result value is Err, failing the test if it is Ok".to_string(),
            syntax: "testing.assert_err(result)".to_string(),
            parameters: vec!["result: Result<T, E> - The Result value to test".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "testing.assert_err(Err(\"error\"))  // Ok(Unit)".to_string(),
                "testing.assert_err(fs.read_file(\"nonexistent_file.txt\"))  // Ok(Unit) if file doesn't exist".to_string(),
                "testing.assert_err(Ok(42))  // Err(\"Assertion failed: expected Err but got Ok(42)\")".to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec!["testing.assert_ok".to_string(), "testing.assert_ne".to_string()],
        });

        // Test control functions
        self.add_function(FunctionDoc {
            name: "testing.fail".to_string(),
            description: "Explicitly fail a test with a custom message".to_string(),
            syntax: "testing.fail(message)".to_string(),
            parameters: vec!["message: String - The failure message".to_string()],
            return_type: "Error".to_string(),
            examples: vec![
                "testing.fail(\"This should never happen\")  // Err(\"Test failed: This should never happen\")".to_string(),
                "if some_unexpected_condition { testing.fail(\"Unexpected condition occurred\") }".to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec!["testing.run_test".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "testing.run_test".to_string(),
            description: "Run a test function with a descriptive name".to_string(),
            syntax: "testing.run_test(name, test_function)".to_string(),
            parameters: vec![
                "name: String - The name of the test".to_string(),
                "test_function: Function - The test function to execute".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "let my_test = () => { testing.assert_eq(2 + 2, 4) }\\ntesting.run_test(\"addition_test\", my_test)  // Ok(\"Test 'addition_test' passed\")".to_string(),
                "testing.run_test(\"failing_test\", () => testing.fail(\"oops\"))  // Err(\"Test failed: oops\")".to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec!["testing.test_summary".to_string(), "testing.fail".to_string()],
        });

        // Utility functions
        self.add_function(FunctionDoc {
            name: "testing.test_summary".to_string(),
            description: "Get a summary of test results and statistics".to_string(),
            syntax: "testing.test_summary()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "testing.test_summary()  // \"Test Summary: 0 tests run, 0 passed, 0 failed\""
                    .to_string(),
                "println(testing.test_summary())  // Print test statistics".to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec![
                "testing.run_test".to_string(),
                "testing.reset_tests".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "testing.reset_tests".to_string(),
            description: "Reset test statistics and counters".to_string(),
            syntax: "testing.reset_tests()".to_string(),
            parameters: vec![],
            return_type: "Unit".to_string(),
            examples: vec![
                "testing.reset_tests()  // Reset all test counters".to_string(),
                "testing.reset_tests(); testing.test_summary()  // Start fresh".to_string(),
            ],
            category: "Testing".to_string(),
            see_also: vec![
                "testing.test_summary".to_string(),
                "testing.run_test".to_string(),
            ],
        });
    }

    /// Add OS module functions to the help system
    fn add_os_functions(&mut self) {
        // Environment variables
        self.add_function(FunctionDoc {
            name: "os.get_env".to_string(),
            description: "Get the value of an environment variable".to_string(),
            syntax: "os.get_env(var_name)".to_string(),
            parameters: vec!["var_name: String - The environment variable name".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "os.get_env(\"PATH\")  // Ok(\"/usr/bin:/usr/local/bin:...\")".to_string(),
                "os.get_env(\"HOME\")  // Ok(\"/home/username\")".to_string(),
                "os.get_env(\"NONEXISTENT\")  // Err(\"Environment variable 'NONEXISTENT' not found\")".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.set_env".to_string(), "os.has_env".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.set_env".to_string(),
            description: "Set an environment variable for the current process".to_string(),
            syntax: "os.set_env(var_name, value)".to_string(),
            parameters: vec![
                "var_name: String - The environment variable name".to_string(),
                "value: String - The value to set".to_string(),
            ],
            return_type: "Unit".to_string(),
            examples: vec![
                "os.set_env(\"MY_VAR\", \"my_value\")  // Ok(Unit)".to_string(),
                "os.set_env(\"LANG\", \"en_US.UTF-8\")".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.get_env".to_string(), "os.remove_env".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.remove_env".to_string(),
            description: "Remove an environment variable from the current process".to_string(),
            syntax: "os.remove_env(var_name)".to_string(),
            parameters: vec!["var_name: String - The environment variable name".to_string()],
            return_type: "Unit".to_string(),
            examples: vec![
                "os.remove_env(\"MY_VAR\")  // Ok(Unit)".to_string(),
                "os.remove_env(\"TEMP_SETTING\")".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.set_env".to_string(), "os.has_env".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.list_env".to_string(),
            description: "Get all environment variables as a struct".to_string(),
            syntax: "os.list_env()".to_string(),
            parameters: vec![],
            return_type: "Map".to_string(),
            examples: vec![
                "os.list_env()  // Ok({ PATH: \"/usr/bin\", HOME: \"/home/user\", ... })"
                    .to_string(),
                "match os.list_env() { Ok(env) => println(env.HOME); Err(e) => println(e) }"
                    .to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.get_env".to_string(), "os.has_env".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.has_env".to_string(),
            description: "Check if an environment variable exists".to_string(),
            syntax: "os.has_env(var_name)".to_string(),
            parameters: vec!["var_name: String - The environment variable name".to_string()],
            return_type: "Bool".to_string(),
            examples: vec![
                "os.has_env(\"PATH\")  // Ok(true)".to_string(),
                "os.has_env(\"NONEXISTENT\")  // Ok(false)".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.get_env".to_string(), "os.list_env".to_string()],
        });

        // System information
        self.add_function(FunctionDoc {
            name: "os.hostname".to_string(),
            description: "Get the system hostname".to_string(),
            syntax: "os.hostname()".to_string(),
            parameters: vec![],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "os.hostname()  // Ok(\"my-computer.local\")".to_string(),
                "match os.hostname() { Ok(name) => println(\"Running on \" + name); Err(e) => println(e) }".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.username".to_string(), "os.os_type".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.username".to_string(),
            description: "Get the current username".to_string(),
            syntax: "os.username()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "os.username()  // Ok(\"johndoe\")".to_string(),
                "println(\"Hello, \" + os.username())".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.hostname".to_string(), "os.home_dir".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.os_type".to_string(),
            description: "Get the operating system type (linux, macos, windows, etc.)".to_string(),
            syntax: "os.os_type()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "os.os_type()  // Ok(\"linux\") or Ok(\"macos\") or Ok(\"windows\")".to_string(),
                "if os.os_type() == Ok(\"windows\") { println(\"Running on Windows\") }"
                    .to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.arch".to_string(), "os.family".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.arch".to_string(),
            description: "Get the system architecture (x86_64, aarch64, etc.)".to_string(),
            syntax: "os.arch()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "os.arch()  // Ok(\"x86_64\") or Ok(\"aarch64\")".to_string(),
                "println(\"Running on \" + os.arch() + \" architecture\")".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.os_type".to_string(), "os.family".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.family".to_string(),
            description: "Get the operating system family (unix, windows, wasm)".to_string(),
            syntax: "os.family()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "os.family()  // Ok(\"unix\") or Ok(\"windows\")".to_string(),
                "if os.family() == Ok(\"unix\") { println(\"Unix-like system\") }".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.os_type".to_string(), "os.arch".to_string()],
        });

        // Process information
        self.add_function(FunctionDoc {
            name: "os.pid".to_string(),
            description: "Get the current process ID".to_string(),
            syntax: "os.pid()".to_string(),
            parameters: vec![],
            return_type: "Int".to_string(),
            examples: vec![
                "os.pid()  // Ok(12345)".to_string(),
                "println(\"Process ID: \" + to_string(os.pid()))".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.args".to_string(), "os.exe_path".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.args".to_string(),
            description: "Get command line arguments as a list".to_string(),
            syntax: "os.args()".to_string(),
            parameters: vec![],
            return_type: "[String]".to_string(),
            examples: vec![
                "os.args()  // Ok([\"program\", \"arg1\", \"arg2\"])".to_string(),
                "match os.args() { Ok(args) => map(args, println); Err(e) => println(e) }"
                    .to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.exe_path".to_string(), "os.pid".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.exe_path".to_string(),
            description: "Get the path to the current executable".to_string(),
            syntax: "os.exe_path()".to_string(),
            parameters: vec![],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "os.exe_path()  // Ok(\"/usr/local/bin/olang\")".to_string(),
                "println(\"Running from: \" + os.exe_path())".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.args".to_string(), "os.cwd".to_string()],
        });

        // Working directory
        self.add_function(FunctionDoc {
            name: "os.cwd".to_string(),
            description: "Get the current working directory".to_string(),
            syntax: "os.cwd()".to_string(),
            parameters: vec![],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "os.cwd()  // Ok(\"/home/user/projects\")".to_string(),
                "println(\"Working in: \" + os.cwd())".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.chdir".to_string(), "os.home_dir".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.chdir".to_string(),
            description: "Change the current working directory".to_string(),
            syntax: "os.chdir(path)".to_string(),
            parameters: vec!["path: String - The directory path to change to".to_string()],
            return_type: "Result<Unit, Error>".to_string(),
            examples: vec![
                "os.chdir(\"/tmp\")  // Ok(Unit)".to_string(),
                "os.chdir(\"../parent_dir\")  // Ok(Unit)".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.cwd".to_string(), "fs.create_dir".to_string()],
        });

        // Path operations
        self.add_function(FunctionDoc {
            name: "os.path_separator".to_string(),
            description: "Get the path separator for the current OS ('/' or '\\\\')".to_string(),
            syntax: "os.path_separator()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "os.path_separator()  // Ok(\"/\") on Unix, Ok(\"\\\\\") on Windows".to_string(),
                "let sep = os.path_separator(); let path = \"dir\" + sep + \"file.txt\""
                    .to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.os_type".to_string(), "os.home_dir".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.home_dir".to_string(),
            description: "Get the user's home directory path".to_string(),
            syntax: "os.home_dir()".to_string(),
            parameters: vec![],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "os.home_dir()  // Ok(\"/home/user\") or Ok(\"C:\\\\Users\\\\user\")".to_string(),
                "let config_path = os.home_dir() + \"/.config/myapp\"".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.temp_dir".to_string(), "os.cwd".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "os.temp_dir".to_string(),
            description: "Get the system temporary directory path".to_string(),
            syntax: "os.temp_dir()".to_string(),
            parameters: vec![],
            return_type: "String".to_string(),
            examples: vec![
                "os.temp_dir()  // Ok(\"/tmp\") or Ok(\"C:\\\\Temp\")".to_string(),
                "let temp_file = os.temp_dir() + \"/myapp_\" + to_string(os.pid()) + \".tmp\""
                    .to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["os.home_dir".to_string(), "fs.create_dir".to_string()],
        });

        // System exit
        self.add_function(FunctionDoc {
            name: "os.exit".to_string(),
            description: "Exit the program with a status code (0 for success)".to_string(),
            syntax: "os.exit(code)".to_string(),
            parameters: vec![
                "code: Int - Exit status code (0 = success, non-zero = error)".to_string(),
            ],
            return_type: "Never returns".to_string(),
            examples: vec![
                "os.exit(0)  // Exit successfully".to_string(),
                "os.exit(1)  // Exit with error code 1".to_string(),
                "if error_occurred { os.exit(1) }".to_string(),
            ],
            category: "OS".to_string(),
            see_also: vec!["testing.fail".to_string()],
        });
    }

    /// Add Crypto module functions to the help system
    fn add_crypto_functions(&mut self) {
        // Hash functions
        self.add_function(FunctionDoc {
            name: "crypto.md5".to_string(),
            description: "Compute MD5 hash of input string (returns hex string)".to_string(),
            syntax: "crypto.md5(input)".to_string(),
            parameters: vec!["input: String - The string to hash".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.md5(\"hello\")  // Ok(\"5d41402abc4b2a76b9719d911017c592\")".to_string(),
                "crypto.md5(\"password123\")  // Ok(\"482c811da5d5b4bc6d497ffa98491e38\")"
                    .to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.sha1".to_string(), "crypto.sha256".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.sha1".to_string(),
            description: "Compute SHA1 hash of input string (returns hex string)".to_string(),
            syntax: "crypto.sha1(input)".to_string(),
            parameters: vec!["input: String - The string to hash".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.sha1(\"hello\")  // Ok(\"aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d\")"
                    .to_string(),
                "crypto.sha1(\"test\")  // Ok(\"a94a8fe5ccb19ba61c4c0873d391e987982fbbd3\")"
                    .to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.md5".to_string(), "crypto.sha256".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.sha256".to_string(),
            description: "Compute SHA256 hash of input string (returns hex string)".to_string(),
            syntax: "crypto.sha256(input)".to_string(),
            parameters: vec!["input: String - The string to hash".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.sha256(\"hello\")  // Ok(\"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\")".to_string(),
                "crypto.sha256(\"secure data\")  // Secure hash".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.sha512".to_string(), "crypto.hmac_sha256".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.sha512".to_string(),
            description: "Compute SHA512 hash of input string (returns hex string)".to_string(),
            syntax: "crypto.sha512(input)".to_string(),
            parameters: vec!["input: String - The string to hash".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.sha512(\"hello\")  // Ok(\"9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043\")".to_string(),
                "crypto.sha512(\"high security\")  // Strongest hash".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.sha256".to_string(), "crypto.hmac_sha512".to_string()],
        });

        // HMAC functions
        self.add_function(FunctionDoc {
            name: "crypto.hmac_sha256".to_string(),
            description: "Compute HMAC-SHA256 with key and message".to_string(),
            syntax: "crypto.hmac_sha256(key, message)".to_string(),
            parameters: vec![
                "key: String - The secret key".to_string(),
                "message: String - The message to authenticate".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.hmac_sha256(\"secret\", \"message\")  // Ok(\"8b5829582c5c40d126588b5b22f8daf48a95efb5ecb3341a4eb5299bbf965733\")".to_string(),
                "crypto.hmac_sha256(api_key, request_body)  // Authenticate API request".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.hmac_sha512".to_string(), "crypto.sha256".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.hmac_sha512".to_string(),
            description: "Compute HMAC-SHA512 with key and message".to_string(),
            syntax: "crypto.hmac_sha512(key, message)".to_string(),
            parameters: vec![
                "key: String - The secret key".to_string(),
                "message: String - The message to authenticate".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.hmac_sha512(\"secret\", \"message\")  // Secure authentication".to_string(),
                "crypto.hmac_sha512(signing_key, payload)  // Sign data".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec![
                "crypto.hmac_sha256".to_string(),
                "crypto.sha512".to_string(),
            ],
        });

        // Password hashing
        self.add_function(FunctionDoc {
            name: "crypto.hash_password".to_string(),
            description: "Hash a password using bcrypt (secure for storage)".to_string(),
            syntax: "crypto.hash_password(password)".to_string(),
            parameters: vec!["password: String - The password to hash".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.hash_password(\"mypassword\")  // Ok(\"$2b$12$...\")".to_string(),
                "let hash = crypto.hash_password(user_password); fs.write_file(\"password.hash\", hash)".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.verify_password".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.verify_password".to_string(),
            description: "Verify a password against a bcrypt hash".to_string(),
            syntax: "crypto.verify_password(password, hash)".to_string(),
            parameters: vec![
                "password: String - The password to verify".to_string(),
                "hash: String - The bcrypt hash to verify against".to_string(),
            ],
            return_type: "Result<Bool, Error>".to_string(),
            examples: vec![
                "crypto.verify_password(\"mypassword\", \"$2b$12$...\")  // Ok(true) or Ok(false)".to_string(),
                "if crypto.verify_password(input_password, stored_hash) == Ok(true) { println(\"Login successful\") }".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.hash_password".to_string(), "crypto.secure_compare".to_string()],
        });

        // Random generation
        self.add_function(FunctionDoc {
            name: "crypto.random_bytes".to_string(),
            description: "Generate cryptographically secure random bytes".to_string(),
            syntax: "crypto.random_bytes(count)".to_string(),
            parameters: vec![
                "count: Int - Number of random bytes to generate (max 1024)".to_string(),
            ],
            return_type: "Result<[Int], Error>".to_string(),
            examples: vec![
                "crypto.random_bytes(16)  // Ok([255, 42, 128, ...]) - 16 random bytes".to_string(),
                "crypto.random_bytes(32)  // Generate 256-bit key".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec![
                "crypto.random_hex".to_string(),
                "random.randint".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "crypto.random_hex".to_string(),
            description: "Generate cryptographically secure random hex string".to_string(),
            syntax: "crypto.random_hex(count)".to_string(),
            parameters: vec![
                "count: Int - Number of random bytes to generate as hex (max 1024)".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.random_hex(16)  // Ok(\"a5f3b2c8...\") - 32 char hex string".to_string(),
                "crypto.random_hex(32)  // Generate secure token".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec![
                "crypto.random_bytes".to_string(),
                "crypto.hex_encode".to_string(),
            ],
        });

        // Encoding utilities
        self.add_function(FunctionDoc {
            name: "crypto.hex_encode".to_string(),
            description: "Encode a string to hexadecimal representation".to_string(),
            syntax: "crypto.hex_encode(input)".to_string(),
            parameters: vec!["input: String - The string to encode".to_string()],
            return_type: "String".to_string(),
            examples: vec![
                "crypto.hex_encode(\"hello\")  // Ok(\"68656c6c6f\")".to_string(),
                "crypto.hex_encode(\"ABC\")  // Ok(\"414243\")".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.hex_decode".to_string(), "base64.encode".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.hex_decode".to_string(),
            description: "Decode a hexadecimal string back to text".to_string(),
            syntax: "crypto.hex_decode(hex_string)".to_string(),
            parameters: vec!["hex_string: String - The hex string to decode".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.hex_decode(\"68656c6c6f\")  // Ok(\"hello\")".to_string(),
                "crypto.hex_decode(\"414243\")  // Ok(\"ABC\")".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.hex_encode".to_string(), "base64.decode".to_string()],
        });

        // Constant-time comparison
        self.add_function(FunctionDoc {
            name: "crypto.secure_compare".to_string(),
            description: "Constant-time string comparison (prevents timing attacks)".to_string(),
            syntax: "crypto.secure_compare(str1, str2)".to_string(),
            parameters: vec![
                "str1: String - First string to compare".to_string(),
                "str2: String - Second string to compare".to_string(),
            ],
            return_type: "Result<Bool, Error>".to_string(),
            examples: vec![
                "crypto.secure_compare(\"secret1\", \"secret1\")  // Ok(true)".to_string(),
                "crypto.secure_compare(api_key, expected_key)  // Secure API key check".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.verify_password".to_string()],
        });

        // Advanced encryption/decryption
        self.add_function(FunctionDoc {
            name: "crypto.encrypt_aes".to_string(),
            description: "Encrypt data using AES-256-GCM with authenticated encryption".to_string(),
            syntax: "crypto.encrypt_aes(data, key, nonce)".to_string(),
            parameters: vec![
                "data: String - The data to encrypt".to_string(),
                "key: String - 32-byte key as hex string (64 characters)".to_string(),
                "nonce: String - 12-byte nonce as hex string (24 characters)".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "let key = crypto.random_hex(32); let nonce = crypto.random_hex(12)".to_string(),
                "crypto.encrypt_aes(\"secret message\", key, nonce)  // Ok(\"encrypted_hex\")"
                    .to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec![
                "crypto.decrypt_aes".to_string(),
                "crypto.random_hex".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "crypto.decrypt_aes".to_string(),
            description: "Decrypt data using AES-256-GCM with authentication".to_string(),
            syntax: "crypto.decrypt_aes(encrypted_data, key) or crypto.decrypt_aes(ciphertext, key, nonce)".to_string(),
            parameters: vec![
                "encrypted_data: String - Output of encrypt_aes (hex of nonce || ciphertext)".to_string(),
                "key: String - 32-byte key as hex string (64 characters)".to_string(),
                "nonce: String - Optional 12-byte nonce as hex (24 characters) when the ciphertext does not embed it".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.decrypt_aes(encrypted_data, key)  // Ok(\"original message\")".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.encrypt_aes".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.encrypt_rsa".to_string(),
            description: "Encrypt data using RSA public key encryption".to_string(),
            syntax: "crypto.encrypt_rsa(data, public_key)".to_string(),
            parameters: vec![
                "data: String - The data to encrypt".to_string(),
                "public_key: String - RSA public key in PEM format".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.encrypt_rsa(\"secret\", public_key_pem)  // Ok(\"base64_encrypted\")"
                    .to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec![
                "crypto.decrypt_rsa".to_string(),
                "crypto.generate_key_pair".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "crypto.decrypt_rsa".to_string(),
            description: "Decrypt data using RSA private key".to_string(),
            syntax: "crypto.decrypt_rsa(encrypted_data, private_key)".to_string(),
            parameters: vec![
                "encrypted_data: String - Base64 encoded encrypted data".to_string(),
                "private_key: String - RSA private key in PEM format".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.decrypt_rsa(encrypted_data, private_key_pem)  // Ok(\"original data\")"
                    .to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.encrypt_rsa".to_string()],
        });

        // Key management
        self.add_function(FunctionDoc {
            name: "crypto.derive_key".to_string(),
            description: "Derive a cryptographic key from a password using Argon2".to_string(),
            syntax: "crypto.derive_key(password, salt, key_length)".to_string(),
            parameters: vec![
                "password: String - The password to derive key from".to_string(),
                "salt: String - Salt in base64 format".to_string(),
                "key_length: Int - Length of key to derive (max 64 bytes)".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "let salt = base64.encode(crypto.random_hex(16))".to_string(),
                "crypto.derive_key(\"mypassword\", salt, 32)  // Ok(\"derived_key_hex\")"
                    .to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.random_hex".to_string(), "base64.encode".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.generate_key_pair".to_string(),
            description: "Generate a new RSA key pair (2048-bit)".to_string(),
            syntax: "crypto.generate_key_pair()".to_string(),
            parameters: vec![],
            return_type: "Result<{private_key: String, public_key: String}, Error>".to_string(),
            examples: vec![
                "let key_pair = crypto.generate_key_pair()".to_string(),
                "let private_key = key_pair.private_key; let public_key = key_pair.public_key"
                    .to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec![
                "crypto.export_public_key".to_string(),
                "crypto.import_public_key".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "crypto.export_public_key".to_string(),
            description: "Extract public key from a key pair struct".to_string(),
            syntax: "crypto.export_public_key(key_pair)".to_string(),
            parameters: vec![
                "key_pair: {private_key: String, public_key: String} - Key pair struct".to_string(),
            ],
            return_type: "String".to_string(),
            examples: vec![
                "let public_key = crypto.export_public_key(key_pair)  // Ok(\"-----BEGIN PUBLIC KEY-----\")".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.generate_key_pair".to_string(), "crypto.import_public_key".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.import_public_key".to_string(),
            description: "Import a public key from PEM format".to_string(),
            syntax: "crypto.import_public_key(pem_string)".to_string(),
            parameters: vec!["pem_string: String - Public key in PEM format".to_string()],
            return_type: "Result<{pem: String}, Error>".to_string(),
            examples: vec![
                "let public_key = crypto.import_public_key(pem_string)  // Ok({pem: \"...\"})"
                    .to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.export_public_key".to_string()],
        });

        // Digital signatures
        self.add_function(FunctionDoc {
            name: "crypto.sign_data".to_string(),
            description: "Sign data using RSA private key with SHA-256".to_string(),
            syntax: "crypto.sign_data(data, private_key)".to_string(),
            parameters: vec![
                "data: String - The data to sign".to_string(),
                "private_key: String - RSA private key in PEM format".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.sign_data(\"important message\", private_key)  // Ok(\"base64_signature\")"
                    .to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.verify_signature".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.verify_signature".to_string(),
            description: "Verify a digital signature using RSA public key".to_string(),
            syntax: "crypto.verify_signature(data, signature, public_key)".to_string(),
            parameters: vec![
                "data: String - The original data that was signed".to_string(),
                "signature: String - Base64 encoded signature".to_string(),
                "public_key: String - RSA public key in PEM format".to_string(),
            ],
            return_type: "Result<Bool, Error>".to_string(),
            examples: vec![
                "crypto.verify_signature(\"message\", signature, public_key)  // Ok(true) or Ok(false)".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.sign_data".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "crypto.create_certificate_signing_request".to_string(),
            description: "Create a certificate signing request (CSR) for SSL/TLS certificates"
                .to_string(),
            syntax: "crypto.create_certificate_signing_request(common_name, private_key)"
                .to_string(),
            parameters: vec![
                "common_name: String - The domain name for the certificate".to_string(),
                "private_key: String - RSA private key in PEM format".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "crypto.create_certificate_signing_request(\"example.com\", private_key)"
                    .to_string(),
                "// Returns PEM-formatted CSR".to_string(),
            ],
            category: "Crypto".to_string(),
            see_also: vec!["crypto.generate_key_pair".to_string()],
        });
    }

    /// Add Base64 module functions to the help system
    fn add_base64_functions(&mut self) {
        // Core encoding and decoding
        self.add_function(FunctionDoc {
            name: "base64.encode".to_string(),
            description: "Encode a string to base64 format".to_string(),
            syntax: "base64.encode(input)".to_string(),
            parameters: vec!["input: String - The string to encode".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "base64.encode(\"hello world\")  // Ok(\"aGVsbG8gd29ybGQ=\")".to_string(),
                "base64.encode(\"Olang rocks!\")  // Ok(\"T2xhbmcgcm9ja3Mh\")".to_string(),
            ],
            category: "Base64".to_string(),
            see_also: vec![
                "base64.decode".to_string(),
                "base64.encode_url_safe".to_string(),
            ],
        });

        self.doc(
            "base64.decode_bytes",
            "base64.decode_bytes(s)",
            "Result",
            "Base64",
            "Decode base64 to raw Bytes — the twin of base64.decode for payloads that are not UTF-8 text. base64.encode accepts Bytes as well as strings.",
        );

        self.add_function(FunctionDoc {
            name: "base64.decode".to_string(),
            description: "Decode a base64 string back to text".to_string(),
            syntax: "base64.decode(base64_string)".to_string(),
            parameters: vec!["base64_string: String - The base64 string to decode".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "base64.decode(\"aGVsbG8gd29ybGQ=\")  // Ok(\"hello world\")".to_string(),
                "base64.decode(\"T2xhbmcgcm9ja3Mh\")  // Ok(\"Olang rocks!\")".to_string(),
            ],
            category: "Base64".to_string(),
            see_also: vec![
                "base64.encode".to_string(),
                "base64.decode_url_safe".to_string(),
            ],
        });

        // URL-safe variants
        self.add_function(FunctionDoc {
            name: "base64.encode_url_safe".to_string(),
            description: "Encode to URL-safe base64 (uses '-' and '_' instead of '+' and '/')"
                .to_string(),
            syntax: "base64.encode_url_safe(input)".to_string(),
            parameters: vec!["input: String - The string to encode".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "base64.encode_url_safe(\"hello?world\")  // URL-safe encoding".to_string(),
                "let token = base64.encode_url_safe(crypto.random_hex(16))".to_string(),
            ],
            category: "Base64".to_string(),
            see_also: vec![
                "base64.decode_url_safe".to_string(),
                "base64.encode".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "base64.decode_url_safe".to_string(),
            description: "Decode URL-safe base64 string".to_string(),
            syntax: "base64.decode_url_safe(base64_string)".to_string(),
            parameters: vec![
                "base64_string: String - The URL-safe base64 string to decode".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "base64.decode_url_safe(url_token)  // Decode URL-safe token".to_string(),
                "base64.decode_url_safe(query_param)  // Decode from URL parameter".to_string(),
            ],
            category: "Base64".to_string(),
            see_also: vec![
                "base64.encode_url_safe".to_string(),
                "base64.decode".to_string(),
            ],
        });

        // Validation and utility
        self.add_function(FunctionDoc {
            name: "base64.validate".to_string(),
            description: "Validate a base64 string, returning decoded value or error details"
                .to_string(),
            syntax: "base64.validate(base64_string)".to_string(),
            parameters: vec!["base64_string: String - The base64 string to validate".to_string()],
            return_type: "Bool".to_string(),
            examples: vec![
                "base64.validate(\"aGVsbG8=\")  // Ok(\"hello\")".to_string(),
                "base64.validate(\"invalid!!!\")  // Err(\"Invalid base64: ...\")".to_string(),
            ],
            category: "Base64".to_string(),
            see_also: vec!["base64.is_valid".to_string(), "base64.decode".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "base64.is_valid".to_string(),
            description: "Check if a string is valid base64 without decoding".to_string(),
            syntax: "base64.is_valid(base64_string)".to_string(),
            parameters: vec!["base64_string: String - The string to check".to_string()],
            return_type: "Result<Bool, Error>".to_string(),
            examples: vec![
                "base64.is_valid(\"aGVsbG8=\")  // Ok(true)".to_string(),
                "base64.is_valid(\"not-base64!!!\")  // Ok(false)".to_string(),
            ],
            category: "Base64".to_string(),
            see_also: vec!["base64.validate".to_string()],
        });

        // Encoding with options
        self.add_function(FunctionDoc {
            name: "base64.encode_no_pad".to_string(),
            description: "Encode to base64 without padding characters ('=')".to_string(),
            syntax: "base64.encode_no_pad(input)".to_string(),
            parameters: vec!["input: String - The string to encode".to_string()],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "base64.encode_no_pad(\"hello\")  // Ok(\"aGVsbG8\") - no '=' padding".to_string(),
                "base64.encode_no_pad(\"hi\")  // Ok(\"aGk\")".to_string(),
            ],
            category: "Base64".to_string(),
            see_also: vec![
                "base64.decode_no_pad".to_string(),
                "base64.encode".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "base64.decode_no_pad".to_string(),
            description: "Decode base64 string that has no padding".to_string(),
            syntax: "base64.decode_no_pad(base64_string)".to_string(),
            parameters: vec![
                "base64_string: String - The unpadded base64 string to decode".to_string(),
            ],
            return_type: "Result<String, Error>".to_string(),
            examples: vec![
                "base64.decode_no_pad(\"aGVsbG8\")  // Ok(\"hello\")".to_string(),
                "base64.decode_no_pad(\"aGk\")  // Ok(\"hi\")".to_string(),
            ],
            category: "Base64".to_string(),
            see_also: vec![
                "base64.encode_no_pad".to_string(),
                "base64.decode".to_string(),
            ],
        });
    }

    /// Add Result type utility functions documentation
    fn add_result_functions(&mut self) {
        self.add_function(FunctionDoc {
            name: "unwrap".to_string(),
            description: "Extract the value from a Result type, panicking if it's an error"
                .to_string(),
            syntax: "unwrap(result)".to_string(),
            parameters: vec!["result: Result<T, E> - The Result value to unwrap".to_string()],
            return_type: "T".to_string(),
            examples: vec![
                "unwrap(Ok(42))  // Returns 42".to_string(),
                "unwrap(fs.read_file(\"config.txt\"))  // Returns file content or panics"
                    .to_string(),
                "unwrap(Err(\"failure\"))  // Panics with error message".to_string(),
            ],
            category: "Result".to_string(),
            see_also: vec![
                "unwrap_or".to_string(),
                "is_ok".to_string(),
                "is_err".to_string(),
            ],
        });

        self.add_function(FunctionDoc {
            name: "unwrap_or".to_string(),
            description:
                "Extract the value from a Result, returning a default value if it's an error"
                    .to_string(),
            syntax: "unwrap_or(result, default)".to_string(),
            parameters: vec![
                "result: Result<T, E> - The Result value to unwrap".to_string(),
                "default: T - The default value to return if result is an error".to_string(),
            ],
            return_type: "T".to_string(),
            examples: vec![
                "unwrap_or(Ok(42), 0)  // Returns 42".to_string(),
                "unwrap_or(Err(\"failed\"), 0)  // Returns 0".to_string(),
                "fs.read_file(\"config.txt\") |> unwrap_or(\"default config\")".to_string(),
            ],
            category: "Result".to_string(),
            see_also: vec!["unwrap".to_string(), "unwrap_or_else".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "unwrap_or_else".to_string(),
            description: "Extract the value from a Result, calling a function to compute a default if it's an error".to_string(),
            syntax: "unwrap_or_else(result, function)".to_string(),
            parameters: vec![
                "result: Result<T, E> - The Result value to unwrap".to_string(),
                "function: (E) -> T - Function to call with the error to compute default".to_string(),
            ],
            return_type: "T".to_string(),
            examples: vec![
                "unwrap_or_else(Ok(42), (err) => 0)  // Returns 42".to_string(),
                "unwrap_or_else(Err(\"failed\"), (err) => len(err))  // Returns length of error message".to_string(),
                "fs.read_file(\"config.txt\") |> unwrap_or_else((err) => \"Error: \" + err)".to_string(),
            ],
            category: "Result".to_string(),
            see_also: vec!["unwrap_or".to_string(), "result_map".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "is_ok".to_string(),
            description: "Check if a Result value is Ok (contains a success value)".to_string(),
            syntax: "is_ok(result)".to_string(),
            parameters: vec!["result: Result<T, E> - The Result value to check".to_string()],
            return_type: "Bool".to_string(),
            examples: vec![
                "is_ok(Ok(42))  // Returns true".to_string(),
                "is_ok(Err(\"failed\"))  // Returns false".to_string(),
                "fs.read_file(\"config.txt\") |> is_ok  // Check if file read succeeded"
                    .to_string(),
            ],
            category: "Result".to_string(),
            see_also: vec!["is_err".to_string(), "unwrap".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "is_err".to_string(),
            description: "Check if a Result value is Err (contains an error)".to_string(),
            syntax: "is_err(result)".to_string(),
            parameters: vec!["result: Result<T, E> - The Result value to check".to_string()],
            return_type: "Bool".to_string(),
            examples: vec![
                "is_err(Ok(42))  // Returns false".to_string(),
                "is_err(Err(\"failed\"))  // Returns true".to_string(),
                "fs.read_file(\"config.txt\") |> is_err  // Check if file read failed".to_string(),
            ],
            category: "Result".to_string(),
            see_also: vec!["is_ok".to_string(), "unwrap".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "result_map".to_string(),
            description:
                "Transform the Ok value of a Result using a function, leaving Err values unchanged"
                    .to_string(),
            syntax: "result_map(result, function)".to_string(),
            parameters: vec![
                "result: Result<T, E> - The Result value to transform".to_string(),
                "function: (T) -> U - Function to apply to the Ok value".to_string(),
            ],
            return_type: "Result<U, E>".to_string(),
            examples: vec![
                "result_map(Ok(42), (x) => x * 2)  // Returns Ok(84)".to_string(),
                "result_map(Err(\"failed\"), (x) => x * 2)  // Returns Err(\"failed\")".to_string(),
                "fs.read_file(\"numbers.txt\") |> result_map((content) => len(content))"
                    .to_string(),
            ],
            category: "Result".to_string(),
            see_also: vec!["result_map_err".to_string(), "result_and_then".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "result_map_err".to_string(),
            description: "Transform the Err value of a Result using a function, leaving Ok values unchanged".to_string(),
            syntax: "result_map_err(result, function)".to_string(),
            parameters: vec![
                "result: Result<T, E> - The Result value to transform".to_string(),
                "function: (E) -> F - Function to apply to the Err value".to_string(),
            ],
            return_type: "Result<T, F>".to_string(),
            examples: vec![
                "result_map_err(Ok(42), (err) => \"Error: \" + err)  // Returns Ok(42)".to_string(),
                "result_map_err(Err(\"failed\"), (err) => \"Error: \" + err)  // Returns Err(\"Error: failed\")".to_string(),
                "fs.read_file(\"config.txt\") |> result_map_err((err) => \"Config error: \" + err)".to_string(),
            ],
            category: "Result".to_string(),
            see_also: vec!["result_map".to_string(), "result_or_else".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "result_and_then".to_string(),
            description: "Chain Result-returning operations, short-circuiting on the first error"
                .to_string(),
            syntax: "result_and_then(result, function)".to_string(),
            parameters: vec![
                "result: Result<T, E> - The Result value to process".to_string(),
                "function: (T) -> Result<U, E> - Function that returns another Result".to_string(),
            ],
            return_type: "Result<U, E>".to_string(),
            examples: vec![
                "result_and_then(Ok(42), (x) => Ok(x * 2))  // Returns Ok(84)".to_string(),
                "result_and_then(Err(\"failed\"), (x) => Ok(x * 2))  // Returns Err(\"failed\")"
                    .to_string(),
                "fs.read_file(\"config.txt\") |> result_and_then((content) => json.parse(content))"
                    .to_string(),
            ],
            category: "Result".to_string(),
            see_also: vec!["result_map".to_string(), "result_or_else".to_string()],
        });

        self.add_function(FunctionDoc {
            name: "result_or_else".to_string(),
            description: "Handle errors by calling a function that returns a Result".to_string(),
            syntax: "result_or_else(result, function)".to_string(),
            parameters: vec![
                "result: Result<T, E> - The Result value to process".to_string(),
                "function: (E) -> Result<T, F> - Function to handle errors".to_string(),
            ],
            return_type: "Result<T, F>".to_string(),
            examples: vec![
                "result_or_else(Ok(42), (err) => Ok(0))  // Returns Ok(42)".to_string(),
                "result_or_else(Err(\"failed\"), (err) => Ok(0))  // Returns Ok(0)".to_string(),
                "fs.read_file(\"config.txt\") |> result_or_else((err) => fs.read_file(\"default.txt\"))".to_string(),
            ],
            category: "Result".to_string(),
            see_also: vec!["result_and_then".to_string(), "unwrap_or_else".to_string()],
        });
    }

    /// Add error-specific help topics to the help system
    fn add_error_help_topics(&mut self) {
        // Syntax Errors
        self.add_function(FunctionDoc {
            name: "error.syntax".to_string(),
            description: "Help with common syntax errors and fixes".to_string(),
            syntax: "help error.syntax".to_string(),
            parameters: vec!["No parameters - displays syntax error help".to_string()],
            return_type: "Help Display".to_string(),
            examples: vec![
                "Unclosed brackets: Check that all (, [, { have matching closing brackets".to_string(),
                "Unclosed strings: Make sure all \" quotes are properly closed".to_string(),
                "Invalid function syntax: Use fn name(params) = expression or fn name(params) { block }".to_string(),
                "Missing operators: Use == for comparison, = only in let declarations".to_string(),
            ],
            category: "Errors".to_string(),
            see_also: vec!["error.types".to_string(), "error.runtime".to_string()],
        });

        // Type Errors
        self.add_function(FunctionDoc {
            name: "error.types".to_string(),
            description: "Help with type errors and type conversion".to_string(),
            syntax: "help error.types".to_string(),
            parameters: vec!["No parameters - displays type error help".to_string()],
            return_type: "Help Display".to_string(),
            examples: vec![
                "Type mismatch: Use to_int(), to_float(), to_string() for conversion".to_string(),
                "Division by zero: Check denominators before division operations".to_string(),
                "Invalid operations: Ensure operands are compatible types (number + number, string + string)".to_string(),
                "Pattern matching: Use Ok(value) and Err(error) for Result types".to_string(),
            ],
            category: "Errors".to_string(),
            see_also: vec!["to_int".to_string(), "to_float".to_string(), "to_string".to_string()],
        });

        // Runtime Errors
        self.add_function(FunctionDoc {
            name: "error.runtime".to_string(),
            description: "Help with runtime errors and debugging".to_string(),
            syntax: "help error.runtime".to_string(),
            parameters: vec!["No parameters - displays runtime error help".to_string()],
            return_type: "Help Display".to_string(),
            examples: vec![
                "Undefined variable: Use :env to see available variables".to_string(),
                "Break/continue outside loop: These can only be used inside for/while loops"
                    .to_string(),
                "Pattern match failed: Ensure patterns match the value structure".to_string(),
                "Function arity mismatch: Check function signature and argument count".to_string(),
            ],
            category: "Errors".to_string(),
            see_also: vec![
                ":env".to_string(),
                ":debug".to_string(),
                ":type".to_string(),
            ],
        });

        // Common Fixes
        self.add_function(FunctionDoc {
            name: "error.fixes".to_string(),
            description: "Common error fixes and best practices".to_string(),
            syntax: "help error.fixes".to_string(),
            parameters: vec!["No parameters - displays common fixes".to_string()],
            return_type: "Help Display".to_string(),
            examples: vec![
                "Bracket matching: Use an editor with syntax highlighting".to_string(),
                "Type checking: Use :type <expression> to check types".to_string(),
                "Variable inspection: Use :inspect <variable> to examine values".to_string(),
                "Debug mode: Use :debug on to get more detailed error information".to_string(),
            ],
            category: "Errors".to_string(),
            see_also: vec![
                ":type".to_string(),
                ":inspect".to_string(),
                ":debug".to_string(),
            ],
        });

        // Language Differences
        self.add_function(FunctionDoc {
            name: "error.differences".to_string(),
            description: "Common mistakes when coming from other languages".to_string(),
            syntax: "help error.differences".to_string(),
            parameters: vec!["No parameters - displays language difference help".to_string()],
            return_type: "Help Display".to_string(),
            examples: vec![
                "JavaScript: Use println() instead of console.log()".to_string(),
                "Python: Use println() instead of print(), no colons for blocks".to_string(),
                "C/Java: No semicolons needed, use = only in let declarations".to_string(),
                "Rust: Functions use = for expression bodies, {} for block bodies".to_string(),
            ],
            category: "Errors".to_string(),
            see_also: vec!["println".to_string(), "error.syntax".to_string()],
        });

        // Variable Scope
        self.add_function(FunctionDoc {
            name: "error.scope".to_string(),
            description: "Help with variable scope and binding errors".to_string(),
            syntax: "help error.scope".to_string(),
            parameters: vec!["No parameters - displays scope help".to_string()],
            return_type: "Help Display".to_string(),
            examples: vec![
                "Variable not in scope: Define variables with let before using them".to_string(),
                "Shadowing: Inner scopes can redefine variables from outer scopes".to_string(),
                "Function scope: Parameters are only available inside the function body"
                    .to_string(),
                "Pattern matching scope: Variables in patterns create new bindings".to_string(),
            ],
            category: "Errors".to_string(),
            see_also: vec!["let".to_string(), ":env".to_string()],
        });
    }

    /// Enhanced fuzzy matching with Levenshtein distance
    #[allow(clippy::needless_range_loop)] // matrix DP is clearest indexed
    fn levenshtein_distance(&self, a: &str, b: &str) -> usize {
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        let a_len = a_chars.len();
        let b_len = b_chars.len();

        if a_len == 0 {
            return b_len;
        }
        if b_len == 0 {
            return a_len;
        }

        let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];

        // Initialize first row and column
        for i in 0..=a_len {
            matrix[i][0] = i;
        }
        for j in 0..=b_len {
            matrix[0][j] = j;
        }

        // Fill the matrix
        for i in 1..=a_len {
            for j in 1..=b_len {
                let cost = if a_chars[i - 1] == b_chars[j - 1] {
                    0
                } else {
                    1
                };
                matrix[i][j] = std::cmp::min(
                    std::cmp::min(
                        matrix[i - 1][j] + 1, // deletion
                        matrix[i][j - 1] + 1, // insertion
                    ),
                    matrix[i - 1][j - 1] + cost, // substitution
                );
            }
        }

        matrix[a_len][b_len]
    }

    /// Calculate similarity score between two strings (0.0 = no match, 1.0 = perfect match)
    fn similarity_score(&self, query: &str, target: &str) -> f64 {
        let query_lower = query.to_lowercase();
        let target_lower = target.to_lowercase();

        // Exact match
        if query_lower == target_lower {
            return 1.0;
        }

        // Starts with match (high score)
        if target_lower.starts_with(&query_lower) {
            return 0.9;
        }

        // Contains match (medium score)
        if target_lower.contains(&query_lower) {
            return 0.7;
        }

        // Levenshtein distance based similarity
        let distance = self.levenshtein_distance(&query_lower, &target_lower);
        let max_len = std::cmp::max(query.len(), target.len());

        if max_len == 0 {
            return 0.0;
        }

        let similarity = 1.0 - (distance as f64 / max_len as f64);

        // Only consider it a match if similarity is above threshold
        if similarity > 0.6 {
            similarity * 0.5 // Scale down fuzzy matches
        } else {
            0.0
        }
    }

    /// Exact (case-insensitive) function-name match, with no fuzzy fallback.
    /// `:help <topic>` uses this so a bare module name like `proc` is not
    /// fuzzily resolved to a single member (`proc.write_line`) before the
    /// module-listing branch gets a chance to run.
    pub fn has_exact_function(&self, name: &str) -> bool {
        let name_lower = name.to_lowercase();
        self.functions
            .keys()
            .any(|k| k.to_lowercase() == name_lower)
    }

    /// The short names of every documented function under `module.` — i.e.
    /// `proc` → `["spawn", "write", …]`. Empty when nothing is namespaced
    /// under that prefix. Drives the `:help <module>` listing uniformly for
    /// native modules, embedded packages, and nested namespaces (`stats.norm`).
    pub fn functions_in_module(&self, module: &str) -> Vec<String> {
        let prefix = format!("{}.", module.to_lowercase());
        let mut names: Vec<String> = self
            .functions
            .keys()
            .filter(|k| k.to_lowercase().starts_with(&prefix))
            .map(|k| k[prefix.len()..].to_string())
            .collect();
        names.sort();
        names
    }

    /// Find function by name with case-insensitive and fuzzy matching
    pub fn find_function_by_name(&self, name: &str) -> Option<&FunctionDoc> {
        let name_lower = name.to_lowercase();

        // First try exact case-insensitive match
        for (func_name, func_doc) in &self.functions {
            if func_name.to_lowercase() == name_lower {
                return Some(func_doc);
            }
        }

        // Then try fuzzy matching
        let mut best_match: Option<&FunctionDoc> = None;
        let mut best_score = 0.0;

        for (func_name, func_doc) in &self.functions {
            let score = self.similarity_score(name, func_name);
            if score > best_score && score > 0.6 {
                best_score = score;
                best_match = Some(func_doc);
            }
        }

        best_match
    }

    /// Find category by name with case-insensitive and fuzzy matching
    pub fn find_category_by_name(&self, name: &str) -> Option<String> {
        let name_lower = name.to_lowercase();

        // First try exact case-insensitive match
        for category_name in self.categories.keys() {
            if category_name.to_lowercase() == name_lower {
                return Some(category_name.clone());
            }
        }

        // Then try fuzzy matching
        let mut best_match: Option<String> = None;
        let mut best_score = 0.0;

        for category_name in self.categories.keys() {
            let score = self.similarity_score(name, category_name);
            if score > best_score && score > 0.6 {
                best_score = score;
                best_match = Some(category_name.clone());
            }
        }

        best_match
    }

    /// Enhanced suggestions with fuzzy matching
    pub fn get_suggestions(&self, partial_input: &str) -> Vec<String> {
        let mut suggestions = Vec::new();
        let partial_lower = partial_input.to_lowercase();

        // Collect function name suggestions with scores
        let mut function_suggestions: Vec<(String, f64)> = Vec::new();
        for function_name in self.functions.keys() {
            let score = self.similarity_score(&partial_lower, function_name);
            if score > 0.3 {
                function_suggestions.push((function_name.clone(), score));
            }
        }

        // Collect category suggestions with scores
        let mut category_suggestions: Vec<(String, f64)> = Vec::new();
        for category in self.categories.keys() {
            let score = self.similarity_score(&partial_lower, category);
            if score > 0.3 {
                category_suggestions.push((category.clone(), score));
            }
        }

        // Sort all suggestions by score (highest first)
        function_suggestions
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        category_suggestions
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Add function suggestions
        for (name, _score) in function_suggestions.iter().take(8) {
            suggestions.push(name.clone());
        }

        // Add category suggestions
        for (name, _score) in category_suggestions.iter().take(4) {
            suggestions.push(format!("category:{}", name));
        }

        suggestions.truncate(10); // Limit to top 10 suggestions
        suggestions
    }

    /// Enhanced case-insensitive function lookup
    pub fn has_function(&self, name: &str) -> bool {
        self.find_function_by_name(name).is_some()
    }

    /// Enhanced case-insensitive category lookup
    pub fn has_category(&self, name: &str) -> bool {
        self.find_category_by_name(name).is_some()
    }

    /// Enhanced case-insensitive function help with fuzzy matching
    pub fn show_function_help(&self, function_name: &str) -> String {
        if let Some(func) = self.find_function_by_name(function_name) {
            self.format_function_documentation(func)
        } else {
            // Try to suggest similar functions
            let suggestions = self
                .get_suggestions(function_name)
                .into_iter()
                .filter(|s| !s.starts_with("category:"))
                .take(3)
                .collect::<Vec<_>>();

            if suggestions.is_empty() {
                format!(
                    "{}{}Function '{}' not found.{}",
                    Colors::BOLD,
                    Colors::RED,
                    function_name,
                    Colors::RESET
                )
            } else {
                format!(
                    "{}{}Function '{}' not found.{} Did you mean:\n{}",
                    Colors::BOLD,
                    Colors::RED,
                    function_name,
                    Colors::RESET,
                    suggestions
                        .iter()
                        .map(|s| format!(
                            "  {}•{} {}{}{}",
                            Colors::YELLOW,
                            Colors::RESET,
                            Colors::CYAN,
                            s,
                            Colors::RESET
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        }
    }

    /// Enhanced case-insensitive category display
    pub fn show_category(&self, category: &str) -> String {
        if let Some(actual_category) = self.find_category_by_name(category) {
            let mut output = format!(
                "{}=== {} Functions ==={}\n\n",
                Colors::BOLD,
                actual_category,
                Colors::RESET
            );

            if let Some(functions) = self.categories.get(&actual_category) {
                // Sort functions within category
                let mut sorted_functions = functions.clone();
                sorted_functions.sort();

                for func_name in sorted_functions {
                    if let Some(func) = self.functions.get(&func_name) {
                        output.push_str(&format!(
                            "{}{}{}  {}\n",
                            Colors::BLUE,
                            func.name,
                            Colors::RESET,
                            func.description
                        ));
                    }
                }

                output.push_str(&format!(
                    "\n{}Use ':help <function_name>' for detailed documentation{}\n",
                    Colors::DIM,
                    Colors::RESET
                ));
            }

            output
        } else {
            // Try to suggest similar categories
            let suggestions = self
                .get_suggestions(category)
                .into_iter()
                .filter(|s| s.starts_with("category:"))
                .map(|s| s.strip_prefix("category:").unwrap_or(&s).to_string())
                .take(3)
                .collect::<Vec<_>>();

            if suggestions.is_empty() {
                format!(
                    "{}{}Category '{}' not found.{}",
                    Colors::BOLD,
                    Colors::RED,
                    category,
                    Colors::RESET
                )
            } else {
                format!(
                    "{}{}Category '{}' not found.{} Did you mean:\n{}",
                    Colors::BOLD,
                    Colors::RED,
                    category,
                    Colors::RESET,
                    suggestions
                        .iter()
                        .map(|s| format!(
                            "  {}•{} {}{}{}",
                            Colors::YELLOW,
                            Colors::RESET,
                            Colors::CYAN,
                            s,
                            Colors::RESET
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        }
    }

    /// Enhanced fuzzy matching score (improved version)
    fn fuzzy_match_score(&self, query: &str, target: &str) -> f64 {
        self.similarity_score(query, target)
    }
}

/// Display helper for formatted output
impl fmt::Display for HelpSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.show_overview())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_function_lookup_has_no_fuzzy_fallback() {
        let h = HelpSystem::new();
        assert!(h.has_exact_function("proc.spawn"));
        assert!(h.has_exact_function("PROC.SPAWN")); // case-insensitive
        // A bare module name must NOT resolve as a function — that's what
        // routes `:help proc` to the module listing instead of one member.
        assert!(!h.has_exact_function("proc"));
        assert!(!h.has_exact_function("stats"));
    }

    #[test]
    fn module_listing_covers_every_stdlib_module() {
        let h = HelpSystem::new();
        for (module, min_fns) in [
            ("proc", 11),
            ("chan", 7),
            ("time", 3),
            ("toml", 3),
            ("ods", 40),
            ("stats", 22),
            ("plot", 12),
            ("dom", 45),
            ("cli", 3),
            ("term", 22),
            ("ui", 5),
            ("viz", 5),
            ("dash", 7),
            ("colx", 16),
            ("mathx", 18),
            ("str", 30),
            ("math", 34),
            ("fs", 23),
            ("csv", 18),
            ("db", 8),
            ("random", 14),
            ("os", 20),
        ] {
            let fns = h.functions_in_module(module);
            assert!(
                fns.len() >= min_fns,
                "expected >= {} documented functions under `{}.`, got {}: {:?}",
                min_fns,
                module,
                fns.len(),
                fns
            );
        }
        // Nested namespaces list too.
        assert_eq!(h.functions_in_module("stats.norm").len(), 4);
        // And unknown prefixes are empty, not an error.
        assert!(h.functions_in_module("nosuchmodule").is_empty());
    }

    #[test]
    fn no_terse_entry_shadows_a_rich_one() {
        // Guard against a compact doc() call overwriting a full FunctionDoc
        // (this happened to math.sin once): entries with examples must keep
        // them.
        let h = HelpSystem::new();
        let sin = h.find_function_by_name("math.sin").expect("math.sin");
        assert!(
            !sin.examples.is_empty(),
            "math.sin lost its rich entry (examples are gone)"
        );
    }
}

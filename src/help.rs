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
            if let Some(ref category) = filters.category {
                if !function
                    .category
                    .to_lowercase()
                    .contains(&category.to_lowercase())
                {
                    continue;
                }
            }

            // Return type filter
            if let Some(ref return_type) = filters.return_type {
                if !function
                    .return_type
                    .to_lowercase()
                    .contains(&return_type.to_lowercase())
                {
                    continue;
                }
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
        if let Some(ref category) = context.current_working_category {
            if category == "List" {
                suggestions.push(":help map - Transform lists with functions".to_string());
                suggestions.push(":help filter - Filter lists by conditions".to_string());
                suggestions.push(":help reduce - Reduce lists to single values".to_string());
            }
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
                "value: Int | Float | String - Value to convert to integer".to_string()
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

        // Build category index
        self.build_category_index();
    }

    fn add_function(&mut self, function: FunctionDoc) {
        self.functions.insert(function.name.clone(), function);
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
            description: "Character index of the first match, or -1.".to_string(),
            syntax: "str.index_of(s, sub)".to_string(),
            parameters: vec![],
            return_type: "Int".to_string(),
            examples: vec!["str.index_of(\"hello\", \"llo\") -> 2".to_string()],
            category: "String".to_string(),
            see_also: vec![],
        });
        self.add_function(FunctionDoc {
            name: "str.last_index_of".to_string(),
            description: "Character index of the last match, or -1.".to_string(),
            syntax: "str.last_index_of(s, sub)".to_string(),
            parameters: vec![],
            return_type: "Int".to_string(),
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
            return_type: "Result<QueryParams, Error>".to_string(),
            examples: vec![
                "http.decode_query(\"name=John%20Doe&age=30&city=New%20York\")".to_string(),
                "match http.decode_query(request.query) { Ok(params) => println(params.name); Err(e) => println(\"Invalid query\") }".to_string(),
            ],
            category: "HTTP".to_string(),
            see_also: vec!["http.encode_query".to_string(), "http.parse_url".to_string()],
        });
    }

    /// Add math module documentation
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
            return_type: "String".to_string(),
            examples: vec![
                "dates.date(2024, 6, 15)  // \"2024-06-15\"".to_string(),
                "let birthday = dates.date(1990, 5, 15)".to_string(),
                "dates.date(2024, 2, 29)  // \"2024-02-29\" (leap year)".to_string(),
            ],
            category: "Dates".to_string(),
            see_also: vec!["dates.datetime".to_string(), "dates.time".to_string()],
        });

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
            self.categories
                .entry(func.category.clone())
                .or_default()
                .push(name.clone());
        }
    }

    /// Format a brief overview of available categories
    fn format_category_overview(&self) -> String {
        let mut categories: Vec<_> = self.categories.keys().collect();
        categories.sort();

        let mut output = String::new();
        for (i, category) in categories.iter().enumerate() {
            if let Some(functions) = self.categories.get(*category) {
                output.push_str(&format!(
                    "  {}{}{} ({})",
                    Colors::CYAN,
                    category,
                    Colors::RESET,
                    functions.len()
                ));
                if i < categories.len() - 1 {
                    output.push_str(", ");
                }
                if (i + 1) % 3 == 0 && i < categories.len() - 1 {
                    output.push('\n');
                }
            }
        }
        output
    }

    /// Show general help overview
    pub fn show_overview(&self) -> String {
        format!(
            "{}=== Olang Interactive Help System ==={}

{}Welcome to Olang!{} This interactive help system provides documentation for all built-in functions and REPL commands.

{}Quick Start:{}
  :help <function>      - Show detailed help for a specific function
  :help list            - List all available functions by category  
  :help examples        - Show practical examples
  :help syntax          - Show language syntax reference

{}Popular Functions:{}
  {}println{}, {}print{}       - Output text and values
  {}map{}, {}filter{}, {}reduce{}  - Transform and process lists  
  {}range{}, {}length{}, {}head{}  - Work with sequences
  {}fs.{}, {}http.{}, {}math.{}, {}random.{} - Standard library modules

{}REPL Commands:{}
  {}:env{}               - Show current environment
  {}:history{}           - Show command history
  {}:type <expr>{}       - Check expression type
  {}:clear{}             - Clear screen or environment
  :sh <cmd> or !<cmd>  - Run a shell command
  :cd, :pwd, :ls       - Navigate the filesystem
  TAB                  - Complete commands, functions, and file paths

{}Categories Available:{}
{}

Type '{}:help <function>{}' for detailed documentation on any function.
Type '{}:help list{}' to see all functions organized by category.

{}Happy coding! {}",
            Colors::BOLD, Colors::RESET,
            Colors::GREEN, Colors::RESET,
            Colors::YELLOW, Colors::RESET,
            Colors::CYAN, Colors::RESET,
            Colors::BLUE, Colors::RESET, Colors::BLUE, Colors::RESET,
            Colors::BLUE, Colors::RESET, Colors::BLUE, Colors::RESET, Colors::BLUE, Colors::RESET,
            Colors::BLUE, Colors::RESET, Colors::BLUE, Colors::RESET, Colors::BLUE, Colors::RESET,
            Colors::BLUE, Colors::RESET, Colors::BLUE, Colors::RESET, Colors::BLUE, Colors::RESET, Colors::BLUE, Colors::RESET,
            Colors::MAGENTA, Colors::RESET,
            Colors::BLUE, Colors::RESET,
            Colors::BLUE, Colors::RESET,
            Colors::BLUE, Colors::RESET,
            Colors::BLUE, Colors::RESET,
            Colors::YELLOW, Colors::RESET,
            self.format_category_overview(),
            Colors::CYAN, Colors::RESET,
            Colors::CYAN, Colors::RESET,
            Colors::GREEN, Colors::RESET
        )
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
  let name = \"Alice\"          // Immutable variable
  let age = 25                 // Type inferred
  age = 26                     // Mutation (reassignment)

{}Functions:{}
  fn add(a, b) = a + b         // Simple function
  fn greet(name: String) => String = \"Hello \" + name  // With type annotations
  
{}Lambda Functions:{}
  (x) => x * 2                 // Single parameter
  (a, b) => a + b              // Multiple parameters
  () => \"Hello\"                // No parameters

{}Control Flow:{}
  if condition => value1 else value2
  for item in list => {{ ... }}
  while condition => {{ ... }}

{}Lists and Tuples:{}
  [1, 2, 3, 4]                 // List
  (\"Alice\", 25, true)         // Tuple
  [1..10]                      // Range syntax

{}Pipeline Operator:{}
  data |> map(transform) |> filter(predicate) |> reduce(0, combine)

{}Pattern Matching:{}
  match value {{
    Ok(result) => \"Success: \" + result,
    Err(error) => \"Error: \" + error
  }}

{}Type Annotations:{}
  let numbers: List[Int] = [1, 2, 3]
  fn process(items: List[String]) => List[Int] = ...

{}Comments:{}
  // Single line comment
  /* Multi-line
     comment */

{}Modules:{}
  import my_module
  export {{ function_name, CONSTANT }}

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
            return_type: "Result<List<String>, Error>".to_string(),
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
            return_type: "Result<Int, Error>".to_string(),
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
            return_type: "Result<Unit, Error>".to_string(),
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
            return_type: "Result<Unit, Error>".to_string(),
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
            return_type: "Result<{String: String}, Error>".to_string(),
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
            return_type: "Result<Bool, Error>".to_string(),
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
            return_type: "Result<String, Error>".to_string(),
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
            return_type: "Result<String, Error>".to_string(),
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
            return_type: "Result<String, Error>".to_string(),
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
            return_type: "Result<String, Error>".to_string(),
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
            return_type: "Result<Int, Error>".to_string(),
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
            return_type: "Result<[String], Error>".to_string(),
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
            return_type: "Result<String, Error>".to_string(),
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
            return_type: "Result<String, Error>".to_string(),
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
                "count: Int - Number of random bytes to generate (max 1024)".to_string()
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
            return_type: "Result<String, Error>".to_string(),
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
            return_type: "Result<String, Error>".to_string(),
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
            return_type: "Result<String, Error>".to_string(),
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

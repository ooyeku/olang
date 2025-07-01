use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Project templates available for scaffolding
#[derive(Debug, Clone)]
pub enum ProjectTemplate {
    Default,
    Web,
    Cli,
    Library,
}

impl ProjectTemplate {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "default" => Ok(ProjectTemplate::Default),
            "web" => Ok(ProjectTemplate::Web),
            "cli" => Ok(ProjectTemplate::Cli),
            "library" | "lib" => Ok(ProjectTemplate::Library),
            _ => Err(anyhow::anyhow!("Unknown template: {}. Available: default, web, cli, library", s)),
        }
    }
}

/// Execute the new project command
pub fn execute(name: String, is_lib: bool, template: String, verbose: bool) -> Result<()> {
    let template = ProjectTemplate::from_str(&template)?;
    
    if verbose {
        println!("Creating new Olang project: {}", name);
        println!("Template: {:?}", template);
        println!("Type: {}", if is_lib { "Library" } else { "Application" });
    }

    // Check if directory already exists
    if Path::new(&name).exists() {
        return Err(anyhow::anyhow!("Directory '{}' already exists", name));
    }

    // Create project structure
    create_project_structure(&name, is_lib, &template, verbose)?;
    
    println!("✅ Created new Olang project: {}", name);
    println!("📁 Project structure:");
    print_project_structure(&name);
    println!("\n🚀 Get started:");
    println!("   cd {}", name);
    println!("   otc run main.ol");
    
    Ok(())
}

/// Create the complete project structure
fn create_project_structure(
    name: &str, 
    is_lib: bool, 
    template: &ProjectTemplate,
    verbose: bool
) -> Result<()> {
    // Create root directory
    fs::create_dir(name)?;
    if verbose {
        println!("Created directory: {}", name);
    }

    // Create subdirectories
    let dirs = match template {
        ProjectTemplate::Web => vec!["src", "src/routes", "src/middleware", "src/models", "static", "templates", "tests"],
        ProjectTemplate::Cli => vec!["src", "src/commands", "src/utils", "tests", "docs"],
        ProjectTemplate::Library => vec!["src", "src/lib", "examples", "tests", "docs"],
        ProjectTemplate::Default => vec!["src", "src/modules", "tests"],
    };

    for dir in dirs {
        let dir_path = format!("{}/{}", name, dir);
        fs::create_dir_all(&dir_path)?;
        if verbose {
            println!("Created directory: {}", dir_path);
        }
    }

    // Create project manifest
    create_project_manifest(name, is_lib, template)?;
    
    // Create main files
    create_main_files(name, is_lib, template)?;
    
    // Create module examples
    create_module_examples(name, template)?;
    
    // Create tests
    create_test_files(name, template)?;
    
    // Create documentation
    create_docs(name, template)?;

    Ok(())
}

/// Create project manifest file (olang.toml)
fn create_project_manifest(name: &str, is_lib: bool, template: &ProjectTemplate) -> Result<()> {
    let manifest_content = match template {
        ProjectTemplate::Web => format!(r#"[project]
name = "{}"
version = "0.1.0"
type = "web"
description = "A web application built with Olang"

[dependencies]
# Add your dependencies here
# http = "^1.0"
# json = "^1.0"

[web]
port = 8080
host = "127.0.0.1"
static_dir = "static"
template_dir = "templates"

[build]
entry_point = "src/main.ol"
output_dir = "dist"
"#, name),
        
        ProjectTemplate::Cli => format!(r#"[project]
name = "{}"
version = "0.1.0"
type = "cli"
description = "A command-line application built with Olang"

[dependencies]
# Add your dependencies here
# fs = "^1.0"
# os = "^1.0"

[cli]
binary_name = "{}"

[build]
entry_point = "src/main.ol"
output_dir = "dist"
"#, name, name),
        
        ProjectTemplate::Library => format!(r#"[project]
name = "{}"
version = "0.1.0"
type = "library"
description = "A library built with Olang"

[dependencies]
# Add your dependencies here

[library]
export_modules = ["src/lib/mod.ol"]

[build]
entry_point = "src/lib/mod.ol"
output_dir = "dist"
"#, name),
        
        ProjectTemplate::Default => format!(r#"[project]
name = "{}"
version = "0.1.0"
type = "{}"
description = "An Olang project"

[dependencies]
# Add your dependencies here
# math = "^1.0"
# dates = "^1.0"

[build]
entry_point = "src/main.ol"
output_dir = "dist"
"#, name, if is_lib { "library" } else { "application" }),
    };

    fs::write(format!("{}/olang.toml", name), manifest_content)?;
    Ok(())
}

/// Create main application files
fn create_main_files(name: &str, is_lib: bool, template: &ProjectTemplate) -> Result<()> {
    match template {
        ProjectTemplate::Web => {
            // Main web server file
            let main_content = r#"// Web Application Entry Point
import { Server } from "src/server"
import { router } from "src/routes/mod"

let server = Server.new()
server.use_router(router)

println("🚀 Starting web server on http://localhost:8080")
server.listen(8080)
"#;
            fs::write(format!("{}/src/main.ol", name), main_content)?;

            // Server module
            let server_content = r#"// HTTP Server Implementation
export Server = {
    new: () => {
        port: 8080,
        host: "127.0.0.1",
        routes: [],
        
        use_router: (self, router) => {
            self.routes = self.routes ++ router.routes
        },
        
        listen: (self, port) => {
            self.port = port
            // Start HTTP server
            http.serve(self.host, self.port, self.routes)
        }
    }
}
"#;
            fs::write(format!("{}/src/server.ol", name), server_content)?;

            // Routes module
            let routes_content = r#"// Route Definitions
import { index_handler, about_handler } from "src/routes/handlers"

export router = {
    routes: [
        { method: "GET", path: "/", handler: index_handler },
        { method: "GET", path: "/about", handler: about_handler },
    ]
}
"#;
            fs::write(format!("{}/src/routes/mod.ol", name), routes_content)?;

            let handlers_content = r#"// Route Handlers
export index_handler = (req) => {
    http.response_with_headers(
        200,
        "Welcome to Olang Web!",
        { "Content-Type": "text/html" }
    )
}

export about_handler = (req) => {
    http.response_with_headers(
        200, 
        "About this Olang application",
        { "Content-Type": "text/html" }
    )
}
"#;
            fs::write(format!("{}/src/routes/handlers.ol", name), handlers_content)?;
        },

        ProjectTemplate::Cli => {
            let main_content = r#"// CLI Application Entry Point
import { parse_args, run_command } from "src/commands/mod"

let args = os.args()
let command = parse_args(args)

match command {
    { cmd: "help" } => print_help(),
    { cmd: "version" } => print_version(),
    { cmd: name, args: cmd_args } => run_command(name, cmd_args),
    _ => {
        println("Unknown command. Use --help for usage information.")
        os.exit(1)
    }
}

let print_help = () => {
    println("Usage: {} [COMMAND] [OPTIONS]", os.args()[0])
    println("")
    println("Commands:")
    println("  help     Show this help message")
    println("  version  Show version information")
}

let print_version = () => {
    println("{} version 0.1.0", os.args()[0])
}
"#;
            fs::write(format!("{}/src/main.ol", name), main_content)?;

            let commands_content = r#"// Command Parsing and Execution
export parse_args = (args) => {
    if len(args) < 2 => {
        cmd: "help"
    }
    else => {
        cmd: args[1],
        args: tail(tail(args))
    }
}

export run_command = (name, args) => {
    println("Running command: {} with args: {:?}", name, args)
    // Add your command implementations here
}
"#;
            fs::write(format!("{}/src/commands/mod.ol", name), commands_content)?;
        },

        ProjectTemplate::Library => {
            let lib_content = r#"// Library Main Module
export { math_utils } from "src/lib/math"
export { string_utils } from "src/lib/strings"
export { collection_utils } from "src/lib/collections"

// Re-export common utilities
export fibonacci = math_utils.fibonacci
export reverse_string = string_utils.reverse
export unique = collection_utils.unique
"#;
            fs::write(format!("{}/src/lib/mod.ol", name), lib_content)?;

            let math_content = r#"// Mathematical Utilities
export math_utils = {
    fibonacci: (n) => {
        if n <= 1 => n
        else => math_utils.fibonacci(n - 1) + math_utils.fibonacci(n - 2)
    },
    
    factorial: (n) => {
        if n <= 1 => 1
        else => n * math_utils.factorial(n - 1)
    },
    
    gcd: (a, b) => {
        if b == 0 => a
        else => math_utils.gcd(b, a % b)
    }
}
"#;
            fs::write(format!("{}/src/lib/math.ol", name), math_content)?;

            let strings_content = r#"// String Utilities
export string_utils = {
    reverse: (s) => {
        s |> to_list |> reverse |> join("")
    },
    
    capitalize: (s) => {
        if len(s) == 0 => s
        else => {
            let first = s[0] |> upper
            let rest = s |> substring(1)
            first + rest
        }
    },
    
    words: (s) => {
        s |> split(" ") |> filter((w) => len(w) > 0)
    }
}
"#;
            fs::write(format!("{}/src/lib/strings.ol", name), strings_content)?;

            let collections_content = r#"// Collection Utilities
export collection_utils = {
    unique: (list) => {
        list |> reduce([], (acc, item) => {
            if acc |> contains(item) => acc
            else => acc ++ [item]
        })
    },
    
    group_by: (list, key_fn) => {
        list |> reduce({}, (acc, item) => {
            let key = key_fn(item)
            let current = acc |> get(key, [])
            acc |> set(key, current ++ [item])
        })
    },
    
    chunk: (list, size) => {
        if len(list) <= size => [list]
        else => {
            let chunk = list |> take(size)
            let rest = list |> skip(size)
            [chunk] ++ collection_utils.chunk(rest, size)
        }
    }
}
"#;
            fs::write(format!("{}/src/lib/collections.ol", name), collections_content)?;

            // Main entry point for library example
            let main_content = r#"// Library Example Usage
import { fibonacci, reverse_string, unique } from "src/lib/mod"

// Demonstrate library functions
println("Fibonacci(10): {}", fibonacci(10))
println("Reverse 'hello': {}", reverse_string("hello"))
println("Unique [1,2,2,3,1]: {:?}", unique([1,2,2,3,1]))
"#;
            fs::write(format!("{}/src/main.ol", name), main_content)?;
        },

        ProjectTemplate::Default => {
            let main_content = if is_lib {
                r#"// Library Module
export { utilities } from "src/modules/utils"
export { helpers } from "src/modules/helpers"

// Example library function
export greet = (name) => {
    "Hello, " + name + "!"
}
"#
            } else {
                r#"// Main Application Entry Point
import { utilities } from "src/modules/utils"
import { helpers } from "src/modules/helpers"

// Main application logic
let main = () => {
    println("Welcome to your new Olang project!")
    
    let result = utilities.process_data([1, 2, 3, 4, 5])
    println("Processed data: {:?}", result)
    
    let message = helpers.format_message("Olang", "awesome")
    println(message)
}

// Run the application
main()
"#
            };
            fs::write(format!("{}/src/main.ol", name), main_content)?;
        }
    }

    Ok(())
}

/// Create example modules
fn create_module_examples(name: &str, template: &ProjectTemplate) -> Result<()> {
    match template {
        ProjectTemplate::Default => {
            let utils_content = r#"// Utility Functions Module
export utilities = {
    process_data: (data) => {
        data |> map((x) => x * 2) |> filter((x) => x > 5)
    },
    
    calculate_sum: (numbers) => {
        numbers |> reduce(0, (acc, x) => acc + x)
    },
    
    format_list: (items) => {
        "[" + (items |> map(to_string) |> join(", ")) + "]"
    }
}
"#;
            fs::write(format!("{}/src/modules/utils.ol", name), utils_content)?;

            let helpers_content = r#"// Helper Functions Module
export helpers = {
    format_message: (name, adjective) => {
        `${name} is ${adjective}!`
    },
    
    current_timestamp: () => {
        dates.now() |> dates.format("%Y-%m-%d %H:%M:%S")
    },
    
    safe_divide: (a, b) => {
        if b == 0 => Err("Division by zero")
        else => Ok(a / b)
    }
}
"#;
            fs::write(format!("{}/src/modules/helpers.ol", name), helpers_content)?;
        },
        _ => {} // Other templates create their modules in create_main_files
    }

    Ok(())
}

/// Create test files
fn create_test_files(name: &str, template: &ProjectTemplate) -> Result<()> {
    let test_content = match template {
        ProjectTemplate::Library => r#"// Library Tests
import { fibonacci, reverse_string, unique } from "../src/lib/mod"

// Test fibonacci function
let test_fibonacci = () => {
    testing.assert_eq(fibonacci(0), 0, "fibonacci(0) should be 0")
    testing.assert_eq(fibonacci(1), 1, "fibonacci(1) should be 1")
    testing.assert_eq(fibonacci(5), 5, "fibonacci(5) should be 5")
    testing.assert_eq(fibonacci(10), 55, "fibonacci(10) should be 55")
}

// Test string utilities
let test_string_utils = () => {
    testing.assert_eq(reverse_string("hello"), "olleh", "Should reverse string")
    testing.assert_eq(reverse_string(""), "", "Should handle empty string")
}

// Test collection utilities
let test_collection_utils = () => {
    testing.assert_eq(unique([1,2,2,3,1]), [1,2,3], "Should remove duplicates")
    testing.assert_eq(unique([]), [], "Should handle empty array")
}

// Run all tests
println("Running library tests...")
test_fibonacci()
test_string_utils()
test_collection_utils()
println("✅ All tests passed!")
"#,
        _ => r#"// Project Tests
import { utilities } from "../src/modules/utils"
import { helpers } from "../src/modules/helpers"

// Test utilities module
let test_utilities = () => {
    let result = utilities.process_data([1, 2, 3, 4, 5])
    testing.assert_eq(result, [6, 8, 10], "process_data should double and filter")
    
    let sum = utilities.calculate_sum([1, 2, 3, 4])
    testing.assert_eq(sum, 10, "calculate_sum should sum correctly")
}

// Test helpers module
let test_helpers = () => {
    let message = helpers.format_message("Olang", "great")
    testing.assert_eq(message, "Olang is great!", "format_message should template correctly")
    
    let division = helpers.safe_divide(10, 2)
    testing.assert_eq(division, Ok(5), "safe_divide should divide correctly")
    
    let zero_division = helpers.safe_divide(10, 0)
    match zero_division {
        Err(_) => {}, // Expected
        Ok(_) => testing.fail("Should return error for division by zero")
    }
}

// Run all tests
println("Running project tests...")
test_utilities()
test_helpers()
println("✅ All tests passed!")
"#,
    };

    fs::write(format!("{}/tests/main_test.ol", name), test_content)?;
    Ok(())
}

/// Create documentation files
fn create_docs(name: &str, template: &ProjectTemplate) -> Result<()> {
    let readme_content = match template {
        ProjectTemplate::Web => format!(r#"# {}

A web application built with Olang.

## Features

- HTTP server with routing
- Static file serving
- Template rendering
- Middleware support

## Getting Started

```bash
# Run the web server
otc run src/main.ol

# Access the application
curl http://localhost:8080
```

## Project Structure

```
{}/
├── src/
│   ├── main.ol           # Application entry point
│   ├── server.ol         # HTTP server implementation
│   └── routes/
│       ├── mod.ol        # Route definitions
│       └── handlers.ol   # Route handlers
├── static/               # Static assets
├── templates/            # HTML templates
├── tests/                # Test files
└── olang.toml           # Project configuration
```

## Configuration

Edit `olang.toml` to configure the web server:

```toml
[web]
port = 8080
host = "127.0.0.1"
static_dir = "static"
template_dir = "templates"
```
"#, name, name),

        ProjectTemplate::Cli => format!(r#"# {}

A command-line application built with Olang.

## Installation

```bash
# Build the CLI tool
otc build

# Run locally
otc run src/main.ol --help
```

## Usage

```bash
# Show help
{} help

# Show version
{} version

# Run commands
{} [COMMAND] [OPTIONS]
```

## Project Structure

```
{}/
├── src/
│   ├── main.ol           # CLI entry point
│   ├── commands/
│   │   └── mod.ol        # Command parsing
│   └── utils/            # Utility functions
├── tests/                # Test files
└── olang.toml           # Project configuration
```
"#, name, name, name, name, name),

        ProjectTemplate::Library => format!(r#"# {}

A library built with Olang.

## Installation

Add to your `olang.toml`:

```toml
[dependencies]
{} = "0.1.0"
```

## Usage

```olang
import {{ fibonacci, reverse_string, unique }} from "{}"

let result = fibonacci(10)
let reversed = reverse_string("hello")
let unique_items = unique([1, 2, 2, 3])
```

## API Documentation

### Math Utilities

- `fibonacci(n)` - Calculate the nth Fibonacci number
- `factorial(n)` - Calculate factorial of n
- `gcd(a, b)` - Calculate greatest common divisor

### String Utilities

- `reverse(s)` - Reverse a string
- `capitalize(s)` - Capitalize first letter
- `words(s)` - Split string into words

### Collection Utilities

- `unique(list)` - Remove duplicates from list
- `group_by(list, fn)` - Group items by key function
- `chunk(list, size)` - Split list into chunks

## Project Structure

```
{}/
├── src/
│   └── lib/
│       ├── mod.ol        # Main library exports
│       ├── math.ol       # Math utilities
│       ├── strings.ol    # String utilities
│       └── collections.ol # Collection utilities
├── examples/             # Usage examples
├── tests/                # Test files
└── olang.toml           # Project configuration
```
"#, name, name, name, name),

        ProjectTemplate::Default => format!(r#"# {}

An Olang project.

## Getting Started

```bash
# Run the project
otc run src/main.ol

# Run tests
otc run tests/main_test.ol
```

## Project Structure

```
{}/
├── src/
│   ├── main.ol           # Application entry point
│   └── modules/
│       ├── utils.ol      # Utility functions
│       └── helpers.ol    # Helper functions
├── tests/                # Test files
└── olang.toml           # Project configuration
```

## Development

1. Edit source files in `src/`
2. Add modules in `src/modules/`
3. Write tests in `tests/`
4. Configure project in `olang.toml`
"#, name, name),
    };

    fs::write(format!("{}/README.md", name), readme_content)?;

    // Create .gitignore
    let gitignore_content = r#"# Olang build artifacts
/dist/
/target/

# IDE files
.vscode/
.idea/
*.swp
*.swo

# OS files
.DS_Store
Thumbs.db

# Logs
*.log

# Environment files
.env
.env.local

# Temporary files
/tmp/
"#;
    fs::write(format!("{}/.gitignore", name), gitignore_content)?;

    Ok(())
}

/// Print the project structure
fn print_project_structure(name: &str) {
    println!("   {}/", name);
    println!("   ├── src/");
    println!("   │   ├── main.ol");
    println!("   │   └── modules/");
    println!("   ├── tests/");
    println!("   ├── olang.toml");
    println!("   ├── README.md");
    println!("   └── .gitignore");
}

/// Build the current project
pub fn build_project(release: bool, verbose: bool) -> Result<()> {
    // Check if we're in an Olang project
    if !Path::new("olang.toml").exists() {
        return Err(anyhow::anyhow!("Not in an Olang project directory (olang.toml not found)"));
    }

    if verbose {
        println!("Building project...");
        println!("Mode: {}", if release { "Release" } else { "Debug" });
    }

    // Create output directory
    let output_dir = if release { "dist/release" } else { "dist/debug" };
    fs::create_dir_all(output_dir)?;

    // Read project configuration
    let config = read_project_config()?;
    
    // Determine entry point
    let entry_point = config.build.entry_point.unwrap_or_else(|| "src/main.ol".to_string());
    
    if !Path::new(&entry_point).exists() {
        return Err(anyhow::anyhow!("Entry point not found: {}", entry_point));
    }

    if verbose {
        println!("Entry point: {}", entry_point);
        println!("Output directory: {}", output_dir);
    }

    // For now, we'll copy the source files to the output directory
    // In a future version, this would compile to bytecode or native code
    copy_source_files(&entry_point, output_dir, verbose)?;

    println!("✅ Build completed successfully");
    println!("📦 Output: {}/", output_dir);

    Ok(())
}

/// Run project tests
pub fn test_project(filter: Option<String>, verbose: bool) -> Result<()> {
    // Check if we're in an Olang project
    if !Path::new("olang.toml").exists() {
        return Err(anyhow::anyhow!("Not in an Olang project directory (olang.toml not found)"));
    }

    if verbose {
        println!("Running tests...");
        if let Some(ref filter) = filter {
            println!("Filter: {}", filter);
        }
    }

    // Find test files
    let test_files = find_test_files(filter.as_deref())?;
    
    if test_files.is_empty() {
        println!("No test files found");
        return Ok(());
    }

    println!("Found {} test file(s)", test_files.len());

    // Run each test file
    let mut passed = 0;
    let mut failed = 0;

    for test_file in test_files {
        if verbose {
            println!("Running: {}", test_file.display());
        }

        match run_test_file(&test_file) {
            Ok(_) => {
                println!("✅ {}", test_file.display());
                passed += 1;
            }
            Err(e) => {
                println!("❌ {}: {}", test_file.display(), e);
                failed += 1;
            }
        }
    }

    // Print summary
    println!("\nTest Results:");
    println!("  Passed: {}", passed);
    println!("  Failed: {}", failed);
    println!("  Total:  {}", passed + failed);

    if failed > 0 {
        Err(anyhow::anyhow!("{} test(s) failed", failed))
    } else {
        println!("🎉 All tests passed!");
        Ok(())
    }
}

/// Read project configuration from olang.toml
fn read_project_config() -> Result<ProjectConfig> {
    let content = fs::read_to_string("olang.toml")?;
    // For now, we'll do simple parsing. In the future, use a proper TOML parser
    Ok(ProjectConfig {
        project: ProjectInfo {
            name: extract_toml_value(&content, "name").unwrap_or_else(|| "project".to_string()),
            version: extract_toml_value(&content, "version").unwrap_or_else(|| "0.1.0".to_string()),
            project_type: extract_toml_value(&content, "type").unwrap_or_else(|| "application".to_string()),
        },
        build: BuildConfig {
            entry_point: extract_toml_value(&content, "entry_point"),
            output_dir: extract_toml_value(&content, "output_dir"),
        },
    })
}

/// Extract a value from TOML content (simple implementation)
fn extract_toml_value(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with(key) && line.contains('=') {
            let value = line.split('=').nth(1)?;
            let value = value.trim().trim_matches('"');
            return Some(value.to_string());
        }
    }
    None
}

/// Copy source files to output directory
fn copy_source_files(entry_point: &str, output_dir: &str, verbose: bool) -> Result<()> {
    // Copy the entry point
    let entry_name = Path::new(entry_point)
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid entry point"))?;
    
    let output_path = Path::new(output_dir).join(entry_name);
    fs::copy(entry_point, &output_path)?;
    
    if verbose {
        println!("Copied: {} -> {}", entry_point, output_path.display());
    }

    // Copy src directory if it exists
    if Path::new("src").exists() {
        copy_directory("src", &format!("{}/src", output_dir), verbose)?;
    }

    Ok(())
}

/// Copy a directory recursively
fn copy_directory(src: &str, dst: &str, verbose: bool) -> Result<()> {
    fs::create_dir_all(dst)?;
    
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = Path::new(dst).join(entry.file_name());

        if file_type.is_dir() {
            copy_directory(
                src_path.to_str().unwrap(),
                dst_path.to_str().unwrap(),
                verbose,
            )?;
        } else if file_type.is_file() {
            fs::copy(&src_path, &dst_path)?;
            if verbose {
                println!("Copied: {} -> {}", src_path.display(), dst_path.display());
            }
        }
    }

    Ok(())
}

/// Find test files
fn find_test_files(filter: Option<&str>) -> Result<Vec<PathBuf>> {
    let mut test_files = Vec::new();
    
    // Look in tests directory
    if Path::new("tests").exists() {
        for entry in fs::read_dir("tests")? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_file() && path.extension().map_or(false, |ext| ext == "ol") {
                if let Some(filter) = filter {
                    if path.to_string_lossy().contains(filter) {
                        test_files.push(path);
                    }
                } else {
                    test_files.push(path);
                }
            }
        }
    }

    Ok(test_files)
}

/// Run a single test file
fn run_test_file(test_file: &Path) -> Result<()> {
    use std::process::Command;

    let output = Command::new("olang")
        .arg(test_file)
        .output();

    match output {
        Ok(output) => {
            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("Test failed: {}", stderr))
            }
        }
        Err(e) => {
            // If olang command not found, try using otc run
            let output = Command::new("otc")
                .arg("run")
                .arg(test_file)
                .output()
                .map_err(|_| anyhow::anyhow!("Failed to run test (olang/otc not found): {}", e))?;

            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("Test failed: {}", stderr))
            }
        }
    }
}

/// Project configuration structures
#[derive(Debug)]
struct ProjectConfig {
    project: ProjectInfo,
    build: BuildConfig,
}

#[derive(Debug)]
struct ProjectInfo {
    name: String,
    version: String,
    project_type: String,
}

#[derive(Debug)]
struct BuildConfig {
    entry_point: Option<String>,
    output_dir: Option<String>,
} 
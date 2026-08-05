//! Project Scaffolding Overhaul - Feature 2
//!
//! Simple, reliable project scaffold that always generates working code.
//! Replaces the complex template system with a single, validated approach.

use crate::config::{ensure_project_directory, OlangProject};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Execute the simplified new project command
pub fn execute(name: String, _is_lib: bool, _template: String, verbose: bool) -> Result<()> {
    if verbose {
        println!("Creating new Olang project: {}", name);
    }

    // Check if directory already exists
    if Path::new(&name).exists() {
        return Err(anyhow::anyhow!("Directory '{}' already exists", name));
    }

    // Create simple, reliable project structure
    create_simple_project(&name, verbose)?;

    // Validate that the generated code works
    validate_generated_project(&name, verbose)?;

    println!("Created new Olang project: {}", name);
    println!();
    println!("Project structure:");
    println!("   {}/", name);
    println!("   ├── olang.toml        # Project configuration");
    println!("   ├── README.md         # Documentation");
    println!("   ├── src/");
    println!("   │   └── main.ol       # Main entry point");
    println!("   └── .gitignore        # Git ignore rules");
    println!();
    println!("Get started:");
    println!("   cd {}", name);
    println!("   otc run src/main.ol");

    Ok(())
}

/// Create a simple, reliable project structure
fn create_simple_project(name: &str, verbose: bool) -> Result<()> {
    // Create root directory
    fs::create_dir(name)
        .with_context(|| format!("Failed to create project directory: {}", name))?;

    if verbose {
        println!("Created directory: {}", name);
    }

    // Create src directory
    let src_dir = format!("{}/src", name);
    fs::create_dir(&src_dir)
        .with_context(|| format!("Failed to create src directory: {}", src_dir))?;

    if verbose {
        println!("Created directory: {}", src_dir);
    }

    // Create project files
    create_olang_toml(name, verbose)?;
    create_main_ol(name, verbose)?;
    create_readme_md(name, verbose)?;
    create_gitignore(name, verbose)?;

    Ok(())
}

/// Create a simple, valid olang.toml file
fn create_olang_toml(name: &str, verbose: bool) -> Result<()> {
    let toml_content = format!(
        r#"[project]
name = "{}"
type = "application"
version = "0.1.0"
description = "A new Olang project"
authors = ["Your Name <you@example.com>"]

[dependencies]
# Add dependencies here
# Example: math-utils = "https://github.com/user/math-utils.git"
"#,
        name
    );

    let toml_path = format!("{}/olang.toml", name);
    fs::write(&toml_path, toml_content)
        .with_context(|| format!("Failed to create olang.toml: {}", toml_path))?;

    if verbose {
        println!("Created file: {}", toml_path);
    }

    Ok(())
}

/// Create a simple main.ol file that is guaranteed to parse and run
fn create_main_ol(name: &str, verbose: bool) -> Result<()> {
    let main_content = format!(
        r#"// A simple, working Olang program
println("Hello from Olang!")
println("Project: {}")

// Example variables
let name = "Olang"
let number = 42
let message = "Welcome to " + name + "!"

// Print examples
println(message)
println("Example number: ", number)

// Example list
let numbers = [1, 2, 3, 4, 5]
println("Example list: ", numbers)

// Simple calculation
let result = 5 + 3
println("5 + 3 = ", result)

println("Project setup complete!")
"#,
        name
    );

    let main_path = format!("{}/src/main.ol", name);
    fs::write(&main_path, main_content)
        .with_context(|| format!("Failed to create src/main.ol: {}", main_path))?;

    if verbose {
        println!("Created file: {}", main_path);
    }

    Ok(())
}

/// Create a helpful README.md file
fn create_readme_md(name: &str, verbose: bool) -> Result<()> {
    let readme_content = format!(
        r#"# {}

A new Olang project.

## Getting Started

```bash
# Run the project
otc run src/main.ol

# Check for errors
otc check src/main.ol
```

## Project Structure

```
{}/
├── olang.toml          # Project configuration
├── README.md           # This file
├── src/
│   └── main.ol         # Main entry point
└── .gitignore         # Git ignore rules
```

## Adding Dependencies

Edit `olang.toml` to add dependencies:

```toml
[dependencies]
math-utils = "https://github.com/user/math-utils.git"
```

Then use in your code:

```olang
use math_utils {{ calculate_area }}

fn main() {{
    let area = calculate_area(5.0)
    println("Area: " + to_string(area))
}}
```

## Development

1. Edit `src/main.ol` to add your code
2. Run with `otc run src/main.ol`
3. Add dependencies in `olang.toml` as needed
4. Check your code with `otc check src/main.ol`

## Learn More

- [Olang Documentation](https://github.com/ooyeku/olang)
- [Olang Examples](https://github.com/ooyeku/olang/tree/main/examples)
"#,
        name, name
    );

    let readme_path = format!("{}/README.md", name);
    fs::write(&readme_path, readme_content)
        .with_context(|| format!("Failed to create README.md: {}", readme_path))?;

    if verbose {
        println!("Created file: {}", readme_path);
    }

    Ok(())
}

/// Create a .gitignore file
fn create_gitignore(name: &str, verbose: bool) -> Result<()> {
    let gitignore_content = r#"# Olang build artifacts
target/
*.lock

# Editor files
.vscode/
.idea/
*.swp
*.swo
*~

# OS files
.DS_Store
Thumbs.db
desktop.ini

# Logs
*.log

# Environment files
.env
.env.local

# Temporary files
/tmp/
temp/
"#;

    let gitignore_path = format!("{}/.gitignore", name);
    fs::write(&gitignore_path, gitignore_content)
        .with_context(|| format!("Failed to create .gitignore: {}", gitignore_path))?;

    if verbose {
        println!("Created file: {}", gitignore_path);
    }

    Ok(())
}

/// Validate that the generated project actually works
fn validate_generated_project(name: &str, verbose: bool) -> Result<()> {
    if verbose {
        println!("Validating generated project...");
    }

    // Validate that olang.toml is syntactically correct
    validate_toml_file(name, verbose)?;

    // Validate that the Olang code parses correctly
    validate_olang_code(name, verbose)?;

    if verbose {
        println!("Project validation completed successfully");
    }

    Ok(())
}

/// Validate that the olang.toml file is syntactically correct
fn validate_toml_file(name: &str, verbose: bool) -> Result<()> {
    let toml_path = format!("{}/olang.toml", name);

    if verbose {
        println!("Validating {}", toml_path);
    }

    // Try to parse the TOML file we just created
    let toml_content =
        fs::read_to_string(&toml_path).with_context(|| format!("Failed to read {}", toml_path))?;

    // Parse it to ensure it's valid TOML
    let _: toml::Value = toml::from_str(&toml_content)
        .with_context(|| format!("Generated olang.toml is not valid TOML: {}", toml_path))?;

    // Also try to parse it as an OlangProject to ensure it's compatible
    OlangProject::parse_from_str(&toml_content).with_context(|| {
        format!(
            "Generated olang.toml is not a valid Olang project file: {}",
            toml_path
        )
    })?;

    if verbose {
        println!("✓ olang.toml validation passed");
    }

    Ok(())
}

/// Validate that the generated Olang code parses correctly
fn validate_olang_code(name: &str, verbose: bool) -> Result<()> {
    let main_path = format!("{}/src/main.ol", name);

    if verbose {
        println!("Validating {}", main_path);
    }

    // Read the generated Olang code
    let code_content =
        fs::read_to_string(&main_path).with_context(|| format!("Failed to read {}", main_path))?;

    // Try to parse it with the Olang parser
    let parser = olang::parser::Parser::new();
    match parser.parse(&code_content) {
        Ok(_ast) => {
            if verbose {
                println!("✓ Olang code parsing validation passed");
            }
        }
        Err(parse_error) => {
            return Err(anyhow::anyhow!(
                "Generated Olang code failed to parse: {}\nError: {:?}",
                main_path,
                parse_error
            ));
        }
    }

    Ok(())
}

/// Build the current project (legacy function kept for compatibility)
pub fn build_project(release: bool, verbose: bool) -> Result<()> {
    // Check if we're in an Olang project and load configuration
    ensure_project_directory()?;
    let config = OlangProject::load_current()?;

    // Validate configuration
    config.validate()?;

    if verbose {
        println!("Building project: {}", config.project.name);
        println!("Version: {}", config.project.version);
        println!("Mode: {}", if release { "Release" } else { "Debug" });
    }

    // Create output directory
    let output_dir = if release {
        format!("{}/release", config.get_output_dir())
    } else {
        format!("{}/debug", config.get_output_dir())
    };
    fs::create_dir_all(&output_dir)?;

    let entry_point = config.get_entry_point();

    if !Path::new(entry_point).exists() {
        return Err(anyhow::anyhow!("Entry point not found: {}", entry_point));
    }

    if verbose {
        println!("Entry point: {}", entry_point);
        println!("Output directory: {}", output_dir);
    }

    // For now, we'll copy the source files to the output directory
    // In a future version, this would compile to bytecode or native code
    copy_source_files(entry_point, &output_dir, verbose)?;

    println!("Build completed successfully");
    println!("Output: {}/", output_dir);

    Ok(())
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

/// Run project tests (legacy - use commands::test::execute instead)
#[allow(dead_code)]
pub fn test_project(filter: Option<String>, verbose: bool) -> Result<()> {
    // Check if we're in an Olang project and load configuration
    ensure_project_directory()?;
    let config = OlangProject::load_current()?;

    // Validate configuration
    config.validate()?;

    if verbose {
        println!("Running tests for project: {}", config.project.name);
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
                println!("{}", test_file.display());
                passed += 1;
            }
            Err(e) => {
                println!("{}: {}", test_file.display(), e);
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
        println!("All tests passed!");
        Ok(())
    }
}

/// Find test files
#[allow(dead_code)]
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
#[allow(dead_code)]
fn run_test_file(test_file: &Path) -> Result<()> {
    use std::process::Command;

    let output = Command::new("olang").arg(test_file).output();

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

use crate::ast::{Statement, TestDecl};
use crate::interpreter::{Interpreter, InterpreterError};
use crate::parser::{Parser, ParseError};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TestError {
    #[error("Parse error: {0}")]
    ParseError(#[from] ParseError),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Interpreter error: {0}")]
    InterpreterError(#[from] InterpreterError),
    #[error("Test discovery error: {message}")]
    DiscoveryError { message: String },
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub file_path: PathBuf,
    pub status: TestStatus,
    pub duration: Duration,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct TestSuite {
    pub name: String,
    pub file_path: PathBuf,
    pub tests: Vec<TestDecl>,
    pub results: Vec<TestResult>,
}

#[derive(Debug)]
pub struct TestRunner {
    pub test_suites: Vec<TestSuite>,
    pub total_tests: usize,
    pub passed_tests: usize,
    pub failed_tests: usize,
    pub skipped_tests: usize,
    pub total_duration: Duration,
}

impl TestRunner {
    pub fn new() -> Self {
        Self {
            test_suites: Vec::new(),
            total_tests: 0,
            passed_tests: 0,
            failed_tests: 0,
            skipped_tests: 0,
            total_duration: Duration::new(0, 0),
        }
    }

    /// Discover test files in a directory (recursively)
    pub fn discover_tests(&mut self, directory: &Path) -> Result<(), TestError> {
        if !directory.exists() {
            return Err(TestError::DiscoveryError {
                message: format!("Directory does not exist: {}", directory.display()),
            });
        }

        self.discover_tests_recursive(directory)?;
        Ok(())
    }

    fn discover_tests_recursive(&mut self, directory: &Path) -> Result<(), TestError> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Recursively search subdirectories
                self.discover_tests_recursive(&path)?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("ol") {
                // Check if this is a test file
                if self.is_test_file(&path)? {
                    self.load_test_file(&path)?;
                }
            }
        }
        Ok(())
    }

    fn is_test_file(&self, file_path: &Path) -> Result<bool, TestError> {
        let content = fs::read_to_string(file_path)?;
        
        // Simple heuristic: check if file contains test declarations
        // In a more sophisticated implementation, we could parse and check for test blocks
        Ok(content.contains("test \"") || 
           file_path.file_name()
               .and_then(|name| name.to_str())
               .map(|name| name.contains("test"))
               .unwrap_or(false))
    }

    fn load_test_file(&mut self, file_path: &Path) -> Result<(), TestError> {
        let content = fs::read_to_string(file_path)?;
        let parser = Parser::new();
        let program = parser.parse(&content)?;

        let mut tests = Vec::new();
        for statement in &program.statements {
            if let Statement::TestDecl(test_decl) = statement {
                tests.push(test_decl.clone());
            }
        }

        if !tests.is_empty() {
            let test_suite = TestSuite {
                name: file_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                file_path: file_path.to_path_buf(),
                tests,
                results: Vec::new(),
            };
            self.test_suites.push(test_suite);
        }

        Ok(())
    }

    /// Run all discovered tests
    pub fn run_tests(&mut self) -> Result<(), TestError> {
        let start_time = Instant::now();

        // Collect indices to avoid borrowing issues
        let suite_count = self.test_suites.len();
        for i in 0..suite_count {
            self.run_test_suite_by_index(i)?;
        }

        self.total_duration = start_time.elapsed();
        self.calculate_totals();
        Ok(())
    }

    fn run_test_suite_by_index(&mut self, index: usize) -> Result<(), TestError> {
        let test_suite = &self.test_suites[index];
        println!("Running tests in {}", test_suite.file_path.display());

        let tests = test_suite.tests.clone();
        let file_path = test_suite.file_path.clone();
        
        for test in &tests {
            let result = self.run_single_test(test, &file_path);
            self.test_suites[index].results.push(result);
        }

        Ok(())
    }

    fn run_single_test(&self, test: &TestDecl, file_path: &Path) -> TestResult {
        let start_time = Instant::now();
        let mut interpreter = Interpreter::new();

        // Load the test file context first (imports, etc.)
        if let Err(e) = self.load_test_context(&mut interpreter, file_path) {
            return TestResult {
                name: test.name.clone(),
                file_path: file_path.to_path_buf(),
                status: TestStatus::Failed,
                duration: start_time.elapsed(),
                error_message: Some(format!("Failed to load test context: {}", e)),
            };
        }

        // Run the test body
        let status = match self.execute_test_body(&mut interpreter, &test.body) {
            Ok(_) => {
                println!("  ✓ {}", test.name);
                TestStatus::Passed
            }
            Err(e) => {
                println!("  ✗ {} - {}", test.name, e);
                TestStatus::Failed
            }
        };

        let error_message = if status == TestStatus::Failed {
            Some(format!("Test failed"))
        } else {
            None
        };

        TestResult {
            name: test.name.clone(),
            file_path: file_path.to_path_buf(),
            status,
            duration: start_time.elapsed(),
            error_message,
        }
    }

    fn load_test_context(&self, interpreter: &mut Interpreter, file_path: &Path) -> Result<(), TestError> {
        let content = fs::read_to_string(file_path)?;
        let parser = Parser::new();
        let program = parser.parse(&content)?;

        // Execute all non-test statements to set up context
        for statement in &program.statements {
            if !matches!(statement, Statement::TestDecl(_)) {
                interpreter.eval_statement(statement.clone())?;
            }
        }

        Ok(())
    }

    fn execute_test_body(&self, interpreter: &mut Interpreter, test_body: &[Statement]) -> Result<(), InterpreterError> {
        for statement in test_body {
            interpreter.eval_statement(statement.clone())?;
        }
        Ok(())
    }

    fn calculate_totals(&mut self) {
        self.total_tests = 0;
        self.passed_tests = 0;
        self.failed_tests = 0;
        self.skipped_tests = 0;

        for test_suite in &self.test_suites {
            for result in &test_suite.results {
                self.total_tests += 1;
                match result.status {
                    TestStatus::Passed => self.passed_tests += 1,
                    TestStatus::Failed => self.failed_tests += 1,
                    TestStatus::Skipped => self.skipped_tests += 1,
                }
            }
        }
    }

    /// Generate a comprehensive test report
    pub fn generate_report(&self) -> TestReport {
        TestReport {
            total_tests: self.total_tests,
            passed_tests: self.passed_tests,
            failed_tests: self.failed_tests,
            skipped_tests: self.skipped_tests,
            total_duration: self.total_duration,
            test_suites: self.test_suites.clone(),
            success_rate: if self.total_tests > 0 {
                (self.passed_tests as f64 / self.total_tests as f64) * 100.0
            } else {
                0.0
            },
        }
    }

    /// Run tests and watch for file changes (for continuous testing)
    pub fn run_watch_mode(&mut self, directory: &Path) -> Result<(), TestError> {
        println!(" Starting test watch mode for directory: {}", directory.display());
        println!("Press Ctrl+C to stop...\n");

        // Initial test run
        self.discover_tests(directory)?;
        self.run_tests()?;
        self.print_summary();

        // In a full implementation, this would use a file watcher
        // For now, we'll just run once
        println!("\n Watch mode is not fully implemented yet.");
        println!("   Run the test command again to re-run tests.");

        Ok(())
    }

    pub fn print_summary(&self) {
        println!("\n{}", "=".repeat(50));
        println!("TEST SUMMARY");
        println!("{}", "=".repeat(50));
        println!("Total tests: {}", self.total_tests);
        println!("Passed: {} ✓", self.passed_tests);
        println!("Failed: {} ✗", self.failed_tests);
        
        if self.skipped_tests > 0 {
            println!("Skipped: {} -", self.skipped_tests);
        }
        
        println!("Duration: {:.2}s", self.total_duration.as_secs_f64());
        
        if self.total_tests > 0 {
            let success_rate = (self.passed_tests as f64 / self.total_tests as f64) * 100.0;
            println!("Success rate: {:.1}%", success_rate);
        }

        if self.failed_tests > 0 {
            println!("\n{}", "FAILED TESTS".red());
            println!("{}", "-".repeat(20));
            for test_suite in &self.test_suites {
                for result in &test_suite.results {
                    if result.status == TestStatus::Failed {
                        println!("  {} in {}", result.name, result.file_path.display());
                        if let Some(error) = &result.error_message {
                            println!("    {}", error);
                        }
                    }
                }
            }
        }

        println!("{}", "=".repeat(50));
    }
}

#[derive(Debug, Clone)]
pub struct TestReport {
    pub total_tests: usize,
    pub passed_tests: usize,
    pub failed_tests: usize,
    pub skipped_tests: usize,
    pub total_duration: Duration,
    pub test_suites: Vec<TestSuite>,
    pub success_rate: f64,
}

// Color extension trait for output formatting
trait ColorExt {
    fn red(&self) -> String;
}

impl ColorExt for str {
    fn red(&self) -> String {
        format!("\x1b[31m{}\x1b[0m", self)
    }
} 
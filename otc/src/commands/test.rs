use olang::test_framework::{TestRunner, TestError};
use std::path::Path;
use anyhow::anyhow;

pub fn execute(
    filter: Option<String>, 
    dir: String, 
    watch: bool, 
    threads: Option<usize>, 
    verbose: bool
) -> Result<(), anyhow::Error> {
    let test_dir = Path::new(&dir);

    if !test_dir.exists() {
        return Err(anyhow!("Test directory does not exist: {}", dir));
    }

    if verbose {
        println!("Olang Test Runner");
        println!("Directory: {}", test_dir.display());
        if let Some(ref pattern) = filter {
            println!("Filter: {}", pattern);
        }
        if let Some(thread_count) = threads {
            println!("Threads: {}", thread_count);
        }
        println!();
    }

    let mut test_runner = TestRunner::new();

    if watch {
        // Run in watch mode
        test_runner.run_watch_mode(test_dir)?;
    } else {
        // Run tests once
        run_tests_once(&mut test_runner, test_dir, filter.as_deref(), verbose)?;
    }

    Ok(())
}

fn run_tests_once(
    test_runner: &mut TestRunner,
    test_dir: &Path,
    filter: Option<&str>,
    verbose: bool,
) -> Result<(), TestError> {
    // Discover tests
    if verbose {
        println!("Discovering tests...");
    }
    
    test_runner.discover_tests(test_dir)?;
    
    let total_suites = test_runner.test_suites.len();
    let total_tests: usize = test_runner.test_suites.iter()
        .map(|suite| suite.tests.len())
        .sum();
    
    if total_tests == 0 {
        println!("No tests found in {}", test_dir.display());
        println!("   Make sure your test files contain test blocks:");
        println!("   test \"test name\" {{");
        println!("       assert_eq(actual, expected)");
        println!("   }}");
        return Ok(());
    }

    if verbose {
        println!("   Found {} test suites with {} tests total", total_suites, total_tests);
        println!();
    }

    // Apply filter if specified
    if let Some(filter_pattern) = filter {
        filter_tests(test_runner, filter_pattern, verbose);
    }

    // Run tests
    println!("Running tests...");
    println!();
    
    test_runner.run_tests()?;
    
    // Print results
    test_runner.print_summary();
    
    // Exit with error code if tests failed
    if test_runner.failed_tests > 0 {
        std::process::exit(1);
    }

    Ok(())
}

fn filter_tests(test_runner: &mut TestRunner, pattern: &str, verbose: bool) {
    let original_count: usize = test_runner.test_suites.iter()
        .map(|suite| suite.tests.len())
        .sum();
    
    // Filter tests by name pattern
    for suite in &mut test_runner.test_suites {
        suite.tests.retain(|test| {
            test.name.contains(pattern) || 
            suite.name.contains(pattern) ||
            suite.file_path.to_string_lossy().contains(pattern)
        });
    }
    
    // Remove empty test suites
    test_runner.test_suites.retain(|suite| !suite.tests.is_empty());
    
    let filtered_count: usize = test_runner.test_suites.iter()
        .map(|suite| suite.tests.len())
        .sum();
    
    if verbose && filtered_count != original_count {
        println!("Filter applied: {} tests (was {})", filtered_count, original_count);
        println!();
    }
}

// Tests would go here but are omitted for now to avoid dependency issues 
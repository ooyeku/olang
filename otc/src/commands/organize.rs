use anyhow::{Context, Result};
use olang::parser::Parser;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn execute(dir_path: String, verbose: bool) -> Result<()> {
    let path = Path::new(&dir_path);

    if verbose {
        println!("Analyzing project organization for: {}", path.display());
    }

    let analysis = analyze_organization(path)?;
    display_organization_suggestions(&analysis, verbose);

    Ok(())
}

#[derive(Debug)]
struct OrganizationAnalysis {
    files: Vec<FileAnalysis>,
    suggestions: Vec<OrganizationSuggestion>,
    metrics: OrganizationMetrics,
}

#[derive(Debug, Clone)]
struct FileAnalysis {
    path: PathBuf,
    module_name: String,
    shared_functions: Vec<String>,
    dependencies: Vec<String>,
    dependents: Vec<String>,
    coupling_score: f32,
    cohesion_score: f32,
}

#[derive(Debug)]
struct OrganizationSuggestion {
    suggestion_type: SuggestionType,
    description: String,
    files_involved: Vec<String>,
    priority: Priority,
}

#[derive(Debug)]
enum SuggestionType {
    MoveToFolder,
    ExtractModule,
    MergeFiles,
    BreakCircularDep,
    CreateIndex,
}

#[derive(Debug)]
enum Priority {
    High,
    Medium,
    Low,
}

#[derive(Debug)]
struct OrganizationMetrics {
    total_files: usize,
    avg_coupling: f32,
    avg_cohesion: f32,
    circular_deps: usize,
    large_files: usize,
    organization_score: f32,
}

fn analyze_organization(dir_path: &Path) -> Result<OrganizationAnalysis> {
    let ol_files = find_ol_files(dir_path)?;
    let mut files = Vec::new();
    let mut dependency_graph = HashMap::new();

    // Analyze each file
    for file_path in &ol_files {
        let module_name = path_to_module_name(file_path, dir_path);
        let shared_functions = extract_shared_functions(file_path)?;
        let dependencies = extract_dependencies(file_path)?;

        dependency_graph.insert(module_name.clone(), dependencies.clone());

        files.push(FileAnalysis {
            path: file_path.clone(),
            module_name,
            shared_functions,
            dependencies,
            dependents: Vec::new(), // Will be filled later
            coupling_score: 0.0,    // Will be calculated later
            cohesion_score: 0.0,    // Will be calculated later
        });
    }

    // Calculate dependents (fix borrow checker issue)
    let files_clone = files.clone();
    for file in &mut files {
        for other_file in &files_clone {
            if other_file.dependencies.contains(&file.module_name) {
                file.dependents.push(other_file.module_name.clone());
            }
        }
    }

    // Calculate coupling and cohesion scores
    let files_for_scoring = files.clone();
    for file in &mut files {
        file.coupling_score = calculate_coupling_score(file, &files_for_scoring);
        file.cohesion_score = calculate_cohesion_score(file);
    }

    // Generate suggestions
    let suggestions = generate_suggestions(&files, &dependency_graph);

    // Calculate metrics
    let metrics = calculate_metrics(&files, &dependency_graph);

    Ok(OrganizationAnalysis {
        files,
        suggestions,
        metrics,
    })
}

fn find_ol_files(dir_path: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    if dir_path.is_file() && dir_path.extension().is_some_and(|ext| ext == "ol") {
        files.push(dir_path.to_path_buf());
        return Ok(files);
    }

    if dir_path.is_dir() {
        for entry in std::fs::read_dir(dir_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && path.extension().is_some_and(|ext| ext == "ol") {
                files.push(path);
            } else if path.is_dir() {
                files.extend(find_ol_files(&path)?);
            }
        }
    }

    Ok(files)
}

fn path_to_module_name(file_path: &Path, root: &Path) -> String {
    let relative = file_path.strip_prefix(root).unwrap_or(file_path);
    let without_ext = relative.with_extension("");

    without_ext
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join(".")
}

fn extract_shared_functions(file_path: &Path) -> Result<Vec<String>> {
    let source = std::fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    let parser = Parser::new();
    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", file_path.display()))?;

    let mut functions = Vec::new();

    for statement in &ast.statements {
        if let olang::ast::Statement::ShareDecl(olang::ast::ShareDecl::Function(func)) = statement {
            functions.push(func.name.clone());
        }
    }

    Ok(functions)
}

fn extract_dependencies(file_path: &Path) -> Result<Vec<String>> {
    let source = std::fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    let parser = Parser::new();
    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", file_path.display()))?;

    let mut dependencies = Vec::new();

    for statement in &ast.statements {
        if let olang::ast::Statement::UseDecl(use_decl) = statement {
            let module_path = use_decl.path.join(".");
            dependencies.push(module_path);
        }
    }

    Ok(dependencies)
}

fn calculate_coupling_score(file: &FileAnalysis, all_files: &[FileAnalysis]) -> f32 {
    let total_connections = file.dependencies.len() + file.dependents.len();
    let max_possible = all_files.len().saturating_sub(1);

    if max_possible == 0 {
        0.0
    } else {
        total_connections as f32 / max_possible as f32
    }
}

fn calculate_cohesion_score(file: &FileAnalysis) -> f32 {
    // Simple cohesion metric based on function count and naming patterns
    let function_count = file.shared_functions.len();

    if function_count == 0 {
        return 1.0;
    }

    // Check for common prefixes/themes in function names
    let common_prefixes = find_common_prefixes(&file.shared_functions);
    let prefix_score = if common_prefixes.is_empty() {
        0.0
    } else {
        common_prefixes.len() as f32 / function_count as f32
    };

    // Lower score for too many functions (suggests low cohesion)
    let size_penalty = if function_count > 10 {
        0.5
    } else if function_count > 5 {
        0.8
    } else {
        1.0
    };

    (prefix_score + size_penalty) / 2.0
}

fn find_common_prefixes(functions: &[String]) -> Vec<String> {
    let mut prefixes = HashSet::new();

    for func in functions {
        if let Some(underscore_pos) = func.find('_') {
            prefixes.insert(func[..underscore_pos].to_string());
        }
    }

    prefixes.into_iter().collect()
}

fn generate_suggestions(
    files: &[FileAnalysis],
    dependency_graph: &HashMap<String, Vec<String>>,
) -> Vec<OrganizationSuggestion> {
    let mut suggestions = Vec::new();

    // Check for circular dependencies and suggest fixes
    for (module, deps) in dependency_graph {
        for dep in deps {
            if let Some(dep_deps) = dependency_graph.get(dep) {
                if dep_deps.contains(module) {
                    suggestions.push(OrganizationSuggestion {
                        suggestion_type: SuggestionType::BreakCircularDep,
                        description: format!(
                            "Break circular dependency between '{}' and '{}'",
                            module, dep
                        ),
                        files_involved: vec![module.clone(), dep.clone()],
                        priority: Priority::High,
                    });
                }
            }
        }
    }

    // Suggest moving related files to folders
    for file in files {
        if file.coupling_score > 0.7 && !file.module_name.contains('.') {
            suggestions.push(OrganizationSuggestion {
                suggestion_type: SuggestionType::MoveToFolder,
                description: format!(
                    "Consider moving '{}' to a folder with its related modules",
                    file.module_name
                ),
                files_involved: vec![file.module_name.clone()],
                priority: Priority::Medium,
            });
        }
    }

    // Suggest extracting large modules
    for file in files {
        if file.shared_functions.len() > 8 {
            suggestions.push(OrganizationSuggestion {
                suggestion_type: SuggestionType::ExtractModule,
                description: format!(
                    "Consider splitting '{}' into smaller, more focused modules ({} functions)",
                    file.module_name,
                    file.shared_functions.len()
                ),
                files_involved: vec![file.module_name.clone()],
                priority: Priority::High,
            });
        }
    }

    // Suggest merging small, related files
    let small_files: Vec<_> = files
        .iter()
        .filter(|f| f.shared_functions.len() <= 2 && !f.dependencies.is_empty())
        .collect();

    for small_file in small_files {
        for dep in &small_file.dependencies {
            if let Some(dep_file) = files.iter().find(|f| &f.module_name == dep) {
                if dep_file.shared_functions.len() <= 3 {
                    suggestions.push(OrganizationSuggestion {
                        suggestion_type: SuggestionType::MergeFiles,
                        description: format!(
                            "Consider merging '{}' and '{}' (both are small and related)",
                            small_file.module_name, dep
                        ),
                        files_involved: vec![small_file.module_name.clone(), dep.clone()],
                        priority: Priority::Low,
                    });
                }
            }
        }
    }

    // Suggest creating index files for directories
    let mut dirs_with_multiple_files = HashMap::new();
    for file in files {
        if let Some(dir) = file.module_name.rfind('.') {
            let dir_name = &file.module_name[..dir];
            *dirs_with_multiple_files
                .entry(dir_name.to_string())
                .or_insert(0) += 1;
        }
    }

    for (dir, count) in dirs_with_multiple_files {
        if count >= 3 {
            suggestions.push(OrganizationSuggestion {
                suggestion_type: SuggestionType::CreateIndex,
                description: format!(
                    "Consider creating an index file for '{}' directory ({} files)",
                    dir, count
                ),
                files_involved: vec![format!("{}.index", dir)],
                priority: Priority::Medium,
            });
        }
    }

    suggestions
}

fn calculate_metrics(
    files: &[FileAnalysis],
    dependency_graph: &HashMap<String, Vec<String>>,
) -> OrganizationMetrics {
    let total_files = files.len();
    let avg_coupling = if total_files > 0 {
        files.iter().map(|f| f.coupling_score).sum::<f32>() / total_files as f32
    } else {
        0.0
    };

    let avg_cohesion = if total_files > 0 {
        files.iter().map(|f| f.cohesion_score).sum::<f32>() / total_files as f32
    } else {
        0.0
    };

    let large_files = files
        .iter()
        .filter(|f| f.shared_functions.len() > 8)
        .count();

    // Detect circular dependencies
    let circular_deps = detect_circular_dependencies(dependency_graph);

    // Simple organization score (higher is better)
    let organization_score = (avg_cohesion * 0.6 + (1.0 - avg_coupling) * 0.4) * 100.0;

    OrganizationMetrics {
        total_files,
        avg_coupling,
        avg_cohesion,
        circular_deps,
        large_files,
        organization_score,
    }
}

fn detect_circular_dependencies(dependency_graph: &HashMap<String, Vec<String>>) -> usize {
    let mut cycles = 0;
    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();

    for module in dependency_graph.keys() {
        if !visited.contains(module)
            && has_cycle_dfs(module, dependency_graph, &mut visited, &mut rec_stack)
        {
            cycles += 1;
        }
    }

    cycles
}

fn has_cycle_dfs(
    module: &str,
    graph: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    rec_stack: &mut HashSet<String>,
) -> bool {
    visited.insert(module.to_string());
    rec_stack.insert(module.to_string());

    if let Some(dependencies) = graph.get(module) {
        for dep in dependencies {
            if !visited.contains(dep) {
                if has_cycle_dfs(dep, graph, visited, rec_stack) {
                    return true;
                }
            } else if rec_stack.contains(dep) {
                return true;
            }
        }
    }

    rec_stack.remove(module);
    false
}

fn display_organization_suggestions(analysis: &OrganizationAnalysis, verbose: bool) {
    println!("Organization Analysis");
    println!("====================");
    println!();

    // Display metrics
    println!("Project Metrics:");
    println!("  Files: {}", analysis.metrics.total_files);
    println!("  Average coupling: {:.2}", analysis.metrics.avg_coupling);
    println!("  Average cohesion: {:.2}", analysis.metrics.avg_cohesion);
    println!(
        "  Circular dependencies: {}",
        analysis.metrics.circular_deps
    );
    println!(
        "  Large files (>8 functions): {}",
        analysis.metrics.large_files
    );
    println!(
        "  Organization score: {:.1}/100",
        analysis.metrics.organization_score
    );
    println!();

    if analysis.suggestions.is_empty() {
        println!("No organization suggestions - your project structure looks good!");
        return;
    }

    println!("Suggestions:");

    let high_priority: Vec<_> = analysis
        .suggestions
        .iter()
        .filter(|s| matches!(s.priority, Priority::High))
        .collect();
    let medium_priority: Vec<_> = analysis
        .suggestions
        .iter()
        .filter(|s| matches!(s.priority, Priority::Medium))
        .collect();
    let low_priority: Vec<_> = analysis
        .suggestions
        .iter()
        .filter(|s| matches!(s.priority, Priority::Low))
        .collect();

    if !high_priority.is_empty() {
        println!("\nHigh Priority:");
        for suggestion in high_priority {
            println!("  • {}", suggestion.description);
            if verbose && !suggestion.files_involved.is_empty() {
                println!("    Files: {}", suggestion.files_involved.join(", "));
                println!("    Type: {:?}", suggestion.suggestion_type);
            }
        }
    }

    if !medium_priority.is_empty() {
        println!("\nMedium Priority:");
        for suggestion in &medium_priority {
            println!("  • {}", suggestion.description);
            if verbose && !suggestion.files_involved.is_empty() {
                println!("    Files: {}", suggestion.files_involved.join(", "));
                println!("    Type: {:?}", suggestion.suggestion_type);
            }
        }
    }

    let has_low_priority = !low_priority.is_empty();
    if has_low_priority && verbose {
        println!("\nLow Priority:");
        for suggestion in &low_priority {
            println!("  • {}", suggestion.description);
            if !suggestion.files_involved.is_empty() {
                println!("    Files: {}", suggestion.files_involved.join(", "));
                println!("    Type: {:?}", suggestion.suggestion_type);
            }
        }
    }

    if !verbose && (has_low_priority || !medium_priority.is_empty()) {
        println!("\nUse --verbose to see all suggestions and detailed file information");
    }

    // Show detailed file analysis in verbose mode
    if verbose && !analysis.files.is_empty() {
        println!("\n=== Detailed File Analysis ===");
        for file in &analysis.files {
            println!("\nFile: {} ({})", file.module_name, file.path.display());
            println!("  Functions: {}", file.shared_functions.len());
            println!(
                "  Dependencies: {} -> [{}]",
                file.dependencies.len(),
                file.dependencies.join(", ")
            );
            println!(
                "  Dependents: {} <- [{}]",
                file.dependents.len(),
                file.dependents.join(", ")
            );
            println!("  Coupling: {:.2}", file.coupling_score);
            println!("  Cohesion: {:.2}", file.cohesion_score);
        }
    }
}

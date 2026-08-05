use anyhow::{Context, Result};
use olang::parser::Parser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn execute(dir_path: String, verbose: bool) -> Result<()> {
    let path = Path::new(&dir_path);

    if verbose {
        println!("Visualizing project structure for: {}", path.display());
    }

    let tree = analyze_project_structure(path)?;
    display_project_tree(&tree, verbose);

    Ok(())
}

#[derive(Debug)]
struct ProjectTree {
    root: String,
    files: Vec<FileInfo>,
    total_files: usize,
    total_functions: usize,
    total_shared_functions: usize,
}

#[derive(Debug)]
struct FileInfo {
    path: PathBuf,
    relative_path: String,
    module_name: String,
    shared_functions: Vec<String>,
    private_functions: Vec<String>,
    dependencies: Vec<String>,
}

fn analyze_project_structure(dir_path: &Path) -> Result<ProjectTree> {
    let ol_files = find_ol_files(dir_path)?;
    let mut files = Vec::new();

    let mut total_functions = 0;
    let mut total_shared_functions = 0;

    for file_path in ol_files {
        let relative_path = file_path
            .strip_prefix(dir_path)
            .unwrap_or(&file_path)
            .to_string_lossy()
            .to_string();

        let module_name = path_to_module_name(&file_path, dir_path);
        let (shared_funcs, private_funcs) = extract_functions(&file_path)?;
        let deps = extract_dependencies(&file_path)?;

        total_functions += shared_funcs.len() + private_funcs.len();
        total_shared_functions += shared_funcs.len();

        files.push(FileInfo {
            path: file_path,
            relative_path,
            module_name,
            shared_functions: shared_funcs,
            private_functions: private_funcs,
            dependencies: deps,
        });
    }

    Ok(ProjectTree {
        root: dir_path.to_string_lossy().to_string(),
        total_files: files.len(),
        total_functions,
        total_shared_functions,
        files,
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

fn extract_functions(file_path: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let source = std::fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    let parser = Parser::new();
    let ast = parser
        .parse(&source)
        .with_context(|| format!("Failed to parse file: {}", file_path.display()))?;

    let mut shared_functions = Vec::new();
    let mut private_functions = Vec::new();

    for statement in &ast.statements {
        match statement {
            olang::ast::Statement::ShareDecl(olang::ast::ShareDecl::Function(func)) => {
                shared_functions.push(func.name.clone());
            }
            olang::ast::Statement::FunctionDecl(func) => {
                private_functions.push(func.name.clone());
            }
            _ => {}
        }
    }

    Ok((shared_functions, private_functions))
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

fn display_project_tree(tree: &ProjectTree, verbose: bool) {
    println!("Project Structure: {}", tree.root);
    println!("├── {} files", tree.total_files);
    println!("├── {} total functions", tree.total_functions);
    println!("└── {} shared functions", tree.total_shared_functions);
    println!();

    // Group files by directory
    let mut by_dir: HashMap<String, Vec<&FileInfo>> = HashMap::new();

    for file in &tree.files {
        let dir = if let Some(parent) = Path::new(&file.relative_path).parent() {
            if parent == Path::new("") {
                ".".to_string()
            } else {
                parent.to_string_lossy().to_string()
            }
        } else {
            ".".to_string()
        };

        by_dir.entry(dir).or_default().push(file);
    }

    // Sort directories
    let mut dirs: Vec<_> = by_dir.keys().collect();
    dirs.sort();

    for (i, dir) in dirs.iter().enumerate() {
        let files = &by_dir[*dir];
        let is_last_dir = i == dirs.len() - 1;

        let prefix = if is_last_dir {
            "└── "
        } else {
            "├── "
        };
        println!("{}{}/", prefix, dir);

        for (j, file) in files.iter().enumerate() {
            let is_last_file = j == files.len() - 1;
            let dir_prefix = if is_last_dir { "    " } else { "│   " };
            let file_prefix = if is_last_file {
                "└── "
            } else {
                "├── "
            };

            let file_name = Path::new(&file.relative_path)
                .file_name()
                .unwrap()
                .to_string_lossy();

            print!("{}{}{}", dir_prefix, file_prefix, file_name);

            if verbose {
                let shared_count = file.shared_functions.len();
                let private_count = file.private_functions.len();
                let deps_count = file.dependencies.len();

                print!(" ({} shared, {} private", shared_count, private_count);
                if deps_count > 0 {
                    print!(", {} deps", deps_count);
                }
                print!(")");
            }
            println!();

            if verbose
                && (!file.shared_functions.is_empty()
                    || !file.dependencies.is_empty()
                    || !file.private_functions.is_empty())
            {
                let func_prefix =
                    format!("{}{}    ", dir_prefix, if is_last_file { " " } else { "│" });

                println!(
                    "{}module: {} ({})",
                    func_prefix,
                    file.module_name,
                    file.path.display()
                );

                if !file.shared_functions.is_empty() {
                    println!(
                        "{}shared: {}",
                        func_prefix,
                        file.shared_functions.join(", ")
                    );
                }

                if !file.private_functions.is_empty() {
                    println!(
                        "{}private: {}",
                        func_prefix,
                        file.private_functions.join(", ")
                    );
                }

                if !file.dependencies.is_empty() {
                    println!("{}uses: {}", func_prefix, file.dependencies.join(", "));
                }
            }
        }
    }
}

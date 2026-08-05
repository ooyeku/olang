//! Install Command - Simple Git Package System Implementation
//!
//! Provides package installation from git repositories using the Simple Git Package System.

use crate::git_package::{check_git_available, validate_git_url, GitPackageManager};
use crate::global::GlobalOtc;
use crate::lock_file::{LockFile, LockedPackage};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Execute package installation from git URL or from olang.toml dependencies (legacy wrapper)
#[allow(dead_code)]
pub fn execute(url: Option<String>, verbose: bool) -> Result<()> {
    execute_with_options(url, verbose, false)
}

/// Execute package installation with offline mode support
pub fn execute_with_options(url: Option<String>, verbose: bool, offline: bool) -> Result<()> {
    // Check git availability (unless in offline mode)
    if !offline {
        check_git_available().with_context(|| "Git is required for package installation")?;
    }

    if let Some(package_url) = url {
        // Install specific package from URL
        install_from_url(&package_url, verbose, offline)
    } else {
        // Install all dependencies from olang.toml
        install_from_manifest(verbose, offline)
    }
}

/// Install a specific package from a git URL
fn install_from_url(url: &str, verbose: bool, offline: bool) -> Result<()> {
    if verbose {
        if offline {
            println!("Installing package from URL: {} (offline mode)", url);
        } else {
            println!("Installing package from URL: {}", url);
        }
    }

    // Validate URL format first
    validate_git_url(url).with_context(|| format!("Invalid git URL: {}", url))?;

    // In offline mode, check if package is already available in cache
    if offline {
        return Err(anyhow::anyhow!(
            "Offline installation from URLs not yet supported.\n\
            Use 'otc install' to install from olang.toml with offline mode."
        ));
    }

    // Set up package manager
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    // Install the package
    let install_result = package_manager
        .install_package(url, verbose)
        .with_context(|| format!("Failed to install package from {}", url))?;

    // Update or create lock file for single package installs
    if Path::new("olang.toml").exists() {
        let mut lock_file = LockFile::load().with_context(|| "Failed to load lock file")?;

        let locked_package = LockedPackage::with_dependencies(
            url.to_string(),
            install_result.commit_hash.clone(),
            install_result.version.clone(),
            install_result.manifest.dependencies.clone(),
        );

        lock_file.add_package(install_result.package_name.clone(), locked_package);
        lock_file
            .save()
            .with_context(|| "Failed to save lock file")?;

        if verbose {
            println!("Updated lock file");
        }
    }

    // Success message
    println!("Package installed successfully:");
    println!("  Name: {}", install_result.package_name);
    println!("  Version: {}", install_result.version);
    println!("  Commit: {}", &install_result.commit_hash[..8]);
    println!("  Location: {}", install_result.install_path.display());

    if let Some(description) = &install_result.manifest.package.description {
        println!("  Description: {}", description);
    }

    if !install_result.manifest.dependencies.is_empty() {
        println!(
            "  Dependencies: {}",
            install_result.manifest.dependencies.len()
        );
        if verbose {
            for (name, url) in &install_result.manifest.dependencies {
                println!("    {} -> {}", name, url);
            }
        }
    }

    Ok(())
}

/// Install all dependencies from olang.toml manifest with lock file support
fn install_from_manifest(verbose: bool, offline: bool) -> Result<()> {
    // Check if we're in an Olang project
    if !Path::new("olang.toml").exists() {
        return Err(anyhow::anyhow!(
            "No olang.toml found. Run 'otc new <name>' to create a new project."
        ));
    }

    if verbose {
        if offline {
            println!("Installing dependencies from olang.toml (offline mode)");
        } else {
            println!("Installing dependencies from olang.toml");
        }
    }

    // Load project configuration
    let project_config = crate::config::OlangProject::load_current()
        .with_context(|| "Failed to load project configuration")?;

    if project_config.dependencies.is_empty() {
        println!("No dependencies specified in olang.toml");
        return Ok(());
    }

    // Load or create lock file
    let mut lock_file = LockFile::load().with_context(|| "Failed to load lock file")?;

    if verbose {
        if lock_file.is_empty() {
            println!("No lock file found, will create new one");
        } else {
            println!(
                "Using existing lock file with {} packages",
                lock_file.package_count()
            );
        }
    }

    // Set up package manager
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    // Perform dependency resolution
    let resolved_deps = resolve_dependencies(&project_config.dependencies, verbose)?;

    println!("Installing {} total dependencies...", resolved_deps.len());

    let mut installed_count = 0;
    let mut failed_count = 0;
    let mut updated_lock = false;

    // Install each resolved dependency
    for (name, url) in &resolved_deps {
        if verbose {
            println!("\nInstalling {}: {}", name, url);
        } else {
            print!("Installing {}... ", name);
        }

        // Check if package is locked and skip if in offline mode
        if let Some(locked_pkg) = lock_file.get_package(name) {
            if offline {
                if verbose {
                    println!(
                        "Using locked version {} ({})",
                        locked_pkg.version,
                        locked_pkg.short_commit()
                    );
                } else {
                    println!("✓ {} (locked)", locked_pkg.version);
                }
                installed_count += 1;
                continue;
            }
        } else if offline {
            if !verbose {
                println!("✗ not in cache");
            }
            eprintln!("Package {} not available in offline mode", name);
            failed_count += 1;
            continue;
        }

        // Install or update the package
        match package_manager.install_package(url, verbose) {
            Ok(install_result) => {
                if !verbose {
                    println!("✓ {}", install_result.version);
                }

                // Update lock file
                let locked_package = LockedPackage::with_dependencies(
                    url.clone(),
                    install_result.commit_hash.clone(),
                    install_result.version.clone(),
                    install_result.manifest.dependencies.clone(),
                );

                lock_file.add_package(name.clone(), locked_package);
                updated_lock = true;
                installed_count += 1;
            }
            Err(e) => {
                if !verbose {
                    println!("✗ failed");
                }
                eprintln!("Failed to install {}: {}", name, e);
                failed_count += 1;
            }
        }
    }

    // Save lock file if updated
    if updated_lock {
        lock_file
            .save()
            .with_context(|| "Failed to save lock file")?;

        if verbose {
            println!("Updated lock file");
        }
    }

    // Summary
    println!("\nInstallation complete:");
    println!("  Installed: {}", installed_count);
    if failed_count > 0 {
        println!("  Failed: {}", failed_count);
        return Err(anyhow::anyhow!(
            "{} packages failed to install",
            failed_count
        ));
    }

    println!("  Lock file: olang.lock");
    Ok(())
}

/// Perform recursive dependency resolution
fn resolve_dependencies(
    root_deps: &HashMap<String, String>,
    verbose: bool,
) -> Result<HashMap<String, String>> {
    let mut resolved = HashMap::new();
    let mut visited = HashSet::new();
    let mut stack = Vec::new();

    if verbose {
        println!("Resolving dependencies...");
    }

    // Add root dependencies to the resolution queue
    for (name, url) in root_deps {
        stack.push((name.clone(), url.clone(), 0)); // (name, url, depth)
    }

    while let Some((name, url, depth)) = stack.pop() {
        // Avoid cycles
        if visited.contains(&name) {
            if verbose {
                println!("  {}: {} (already resolved)", " ".repeat(depth * 2), name);
            }
            continue;
        }

        visited.insert(name.clone());
        resolved.insert(name.clone(), url.clone());

        if verbose {
            println!("  {}: {} -> {}", " ".repeat(depth * 2), name, url);
        }

        // For now, we'll implement a simple resolution without fetching transitive dependencies
        // In a full implementation, we would:
        // 1. Parse the git URL to get package metadata
        // 2. Check the package's olang.toml for its dependencies
        // 3. Add those dependencies to the stack

        // This is a simplified implementation for Feature 4
        // A complete implementation would require downloading and parsing each package's manifest
    }

    if verbose {
        println!("Resolved {} total dependencies", resolved.len());
    }

    Ok(resolved)
}

/// List installed packages
pub fn list(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    let packages = package_manager
        .list_packages()
        .with_context(|| "Failed to list packages")?;

    if packages.is_empty() {
        println!("No packages installed");
        return Ok(());
    }

    println!("Installed packages:");
    for (name, manifest) in packages {
        if verbose {
            println!("  {} ({})", name, manifest.package.version);
            if let Some(description) = manifest.package.description {
                println!("    {}", description);
            }
            if !manifest.dependencies.is_empty() {
                println!("    Dependencies: {}", manifest.dependencies.len());
            }
            println!();
        } else {
            println!("  {}      {}", name, manifest.package.version);
        }
    }

    Ok(())
}

/// Remove an installed package and update lock file
pub fn remove(name: String, verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    // Remove from package cache
    package_manager
        .remove_package(&name, verbose)
        .with_context(|| format!("Failed to remove package: {}", name))?;

    // Update lock file if we're in a project
    if Path::new("olang.toml").exists() {
        let mut lock_file = LockFile::load().with_context(|| "Failed to load lock file")?;

        if lock_file.is_package_locked(&name) {
            lock_file.remove_package(&name);
            lock_file
                .save()
                .with_context(|| "Failed to save lock file")?;

            if verbose {
                println!("Updated lock file");
            }
        }
    }

    println!("Package '{}' removed successfully", name);
    Ok(())
}

/// Update all installed packages using lock file information
pub fn update(verbose: bool) -> Result<()> {
    // Check if we're in an Olang project
    if !Path::new("olang.toml").exists() {
        return Err(anyhow::anyhow!(
            "No olang.toml found. Run 'otc new <name>' to create a new project."
        ));
    }

    // Load lock file to get current package information
    let lock_file = LockFile::load().with_context(|| "Failed to load lock file")?;

    if lock_file.is_empty() {
        println!("No packages to update (no lock file found)");
        println!("Run 'otc install' to install dependencies first");
        return Ok(());
    }

    if verbose {
        println!("Updating packages using lock file information...");
    }

    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    let packages = lock_file.all_packages();
    println!("Updating {} packages...", packages.len());

    let mut updated_count = 0;
    let mut failed_count = 0;
    let mut new_lock_file = LockFile::new();

    for (name, locked_pkg) in packages {
        if verbose {
            println!("\nUpdating {}: {}", name, locked_pkg.url);
        } else {
            print!("Updating {}... ", name);
        }

        // Re-install package to get latest version
        match package_manager.install_package(&locked_pkg.url, verbose) {
            Ok(install_result) => {
                let new_commit = &install_result.commit_hash;
                let old_commit = &locked_pkg.commit;

                if new_commit != old_commit {
                    if !verbose {
                        println!(
                            "✓ {} -> {} ({})",
                            locked_pkg.version,
                            install_result.version,
                            &new_commit[..8]
                        );
                    }

                    // Add updated package to new lock file
                    let updated_package = LockedPackage::with_dependencies(
                        locked_pkg.url.clone(),
                        install_result.commit_hash,
                        install_result.version,
                        install_result.manifest.dependencies,
                    );

                    new_lock_file.add_package(name.clone(), updated_package);
                } else {
                    if !verbose {
                        println!("✓ {} (up to date)", locked_pkg.version);
                    }

                    // Keep existing package in lock file
                    new_lock_file.add_package(name.clone(), locked_pkg.clone());
                }

                updated_count += 1;
            }
            Err(e) => {
                if !verbose {
                    println!("✗ failed");
                }
                eprintln!("Failed to update {}: {}", name, e);

                // Keep existing package in lock file on failure
                new_lock_file.add_package(name.clone(), locked_pkg.clone());
                failed_count += 1;
            }
        }
    }

    // Save updated lock file
    new_lock_file
        .save()
        .with_context(|| "Failed to save updated lock file")?;

    println!("\nUpdate complete:");
    println!("  Updated: {}", updated_count);
    if failed_count > 0 {
        println!("  Failed: {}", failed_count);
    }
    println!("  Lock file: olang.lock (updated)");

    Ok(())
}

/// Clean package cache
pub fn clean(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();

    if verbose {
        println!("Cleaning package cache: {}", cache_dir.display());
    }

    // For now, this is a simple implementation
    // In practice, we'd remove unused packages, clean up temporary files, etc.
    println!("Cache cleaning is not yet fully implemented");
    println!("Cache location: {}", cache_dir.display());

    Ok(())
}

/// Validate a package URL without installing
#[allow(dead_code)]
pub fn validate(url: String, verbose: bool) -> Result<()> {
    if verbose {
        println!("Validating package URL: {}", url);
    }

    // Check git availability
    check_git_available().with_context(|| "Git is required for package validation")?;

    // Validate URL format
    let git_url = crate::git_package::parse_git_url(&url)
        .with_context(|| format!("Invalid git URL: {}", url))?;

    println!("✓ URL format is valid");
    if verbose {
        println!("  Host: {}", git_url.host);
        println!("  Owner: {}", git_url.owner);
        println!("  Repository: {}", git_url.repo);
        println!("  Package name: {}", git_url.package_name);
        if let Some(version) = &git_url.version {
            println!("  Version: {}", version);
        }
    }

    // TODO: Additional validation could include:
    // - Check if repository exists (without cloning)
    // - Validate that olang.toml exists in the repository
    // - Check for security issues

    println!("Package URL appears to be valid");
    Ok(())
}

/// Show package information
pub fn info(url: String, verbose: bool) -> Result<()> {
    if verbose {
        println!("Getting package information for: {}", url);
    }

    // Parse URL to extract information
    let git_url = crate::git_package::parse_git_url(&url)
        .with_context(|| format!("Invalid git URL: {}", url))?;

    println!("Package Information:");
    println!("  URL: {}", url);
    println!("  Host: {}", git_url.host);
    println!("  Owner: {}", git_url.owner);
    println!("  Repository: {}", git_url.repo);
    println!("  Package name: {}", git_url.package_name);

    if let Some(version) = &git_url.version {
        println!("  Requested version: {}", version);
    } else {
        println!("  Version: latest (default branch)");
    }

    // Check if already installed
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    if package_manager.is_package_installed(&git_url.package_name) {
        println!("  Status: Already installed");
        if let Some(path) = package_manager.get_package_path(&git_url.package_name) {
            println!("  Location: {}", path.display());
        }
    } else {
        println!("  Status: Not installed");
    }

    println!("\nTo install this package:");
    println!("  otc install {}", url);

    Ok(())
}

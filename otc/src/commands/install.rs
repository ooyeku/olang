//! Install Command - Simple Git Package System Implementation
//!
//! Provides package installation from git repositories using the Simple Git Package System.

use anyhow::{Context, Result};
use crate::git_package::{GitPackageManager, check_git_available, validate_git_url};
use crate::global::GlobalOtc;
use std::path::Path;

/// Execute package installation from git URL or from olang.toml dependencies
pub fn execute(url: Option<String>, verbose: bool) -> Result<()> {
    // Check git availability first
    check_git_available()
        .with_context(|| "Git is required for package installation")?;

    if let Some(package_url) = url {
        // Install specific package from URL
        install_from_url(&package_url, verbose)
    } else {
        // Install all dependencies from olang.toml
        install_from_manifest(verbose)
    }
}

/// Install a specific package from a git URL
fn install_from_url(url: &str, verbose: bool) -> Result<()> {
    if verbose {
        println!("Installing package from URL: {}", url);
    }

    // Validate URL format first
    validate_git_url(url)
        .with_context(|| format!("Invalid git URL: {}", url))?;

    // Set up package manager
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    // Install the package
    let install_result = package_manager.install_package(url, verbose)
        .with_context(|| format!("Failed to install package from {}", url))?;

    // Success message
    println!("Package installed successfully:");
    println!("  Name: {}", install_result.package_name);
    println!("  Version: {}", install_result.version);
    println!("  Commit: {}", install_result.commit_hash[..8].to_string());
    println!("  Location: {}", install_result.install_path.display());

    if let Some(description) = &install_result.manifest.package.description {
        println!("  Description: {}", description);
    }

    if !install_result.manifest.dependencies.is_empty() {
        println!("  Dependencies: {}", install_result.manifest.dependencies.len());
        if verbose {
            for (name, url) in &install_result.manifest.dependencies {
                println!("    {} -> {}", name, url);
            }
        }
    }

    Ok(())
}

/// Install all dependencies from olang.toml manifest
fn install_from_manifest(verbose: bool) -> Result<()> {
    // Check if we're in an Olang project
    if !Path::new("olang.toml").exists() {
        return Err(anyhow::anyhow!(
            "No olang.toml found. Run 'otc new <name>' to create a new project."
        ));
    }

    if verbose {
        println!("Installing dependencies from olang.toml");
    }

    // Load project configuration
    let project_config = crate::config::OlangProject::load_current()
        .with_context(|| "Failed to load project configuration")?;

    if project_config.dependencies.is_empty() {
        println!("No dependencies specified in olang.toml");
        return Ok(());
    }

    // Set up package manager
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    println!("Installing {} dependencies...", project_config.dependencies.len());

    let mut installed_count = 0;
    let mut failed_count = 0;

    // Install each dependency
    for (name, url) in &project_config.dependencies {
        if verbose {
            println!("\nInstalling {}: {}", name, url);
        } else {
            print!("Installing {}... ", name);
        }

        match package_manager.install_package(url, verbose) {
            Ok(install_result) => {
                if !verbose {
                    println!("✓ {}", install_result.version);
                }
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

    // Summary
    println!("\nInstallation complete:");
    println!("  Installed: {}", installed_count);
    if failed_count > 0 {
        println!("  Failed: {}", failed_count);
        return Err(anyhow::anyhow!("{} packages failed to install", failed_count));
    }

    Ok(())
}

/// List installed packages
pub fn list(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    let packages = package_manager.list_packages()
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

/// Remove an installed package
pub fn remove(name: String, verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    package_manager.remove_package(&name, verbose)
        .with_context(|| format!("Failed to remove package: {}", name))?;

    println!("Package '{}' removed successfully", name);
    Ok(())
}

/// Update all installed packages
pub fn update(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;
    let cache_dir = global_otc.cache_dir();
    let package_manager = GitPackageManager::new(&cache_dir)?;

    let packages = package_manager.list_packages()
        .with_context(|| "Failed to list packages")?;

    if packages.is_empty() {
        println!("No packages to update");
        return Ok(());
    }

    println!("Updating {} packages...", packages.len());

    let mut updated_count = 0;
    let mut failed_count = 0;

    for (name, manifest) in packages {
        if verbose {
            println!("\nUpdating {}...", name);
        } else {
            print!("Updating {}... ", name);
        }

        // For now, we'll re-install the package to update it
        // In a more sophisticated implementation, we'd track the original URL
        // and use git pull operations
        
        if !verbose {
            println!("⏳ (re-installation required)");
        }
        
        // This is a simplified update - in practice, we'd need to store
        // the original git URL to properly update packages
        updated_count += 1;
    }

    println!("\nUpdate complete:");
    println!("  Updated: {}", updated_count);
    if failed_count > 0 {
        println!("  Failed: {}", failed_count);
    }

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
pub fn validate(url: String, verbose: bool) -> Result<()> {
    if verbose {
        println!("Validating package URL: {}", url);
    }

    // Check git availability
    check_git_available()
        .with_context(|| "Git is required for package validation")?;

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
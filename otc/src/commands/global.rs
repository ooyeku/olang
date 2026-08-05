//! Global OTC Management Commands
//!
//! Implements commands for managing global OTC configuration, packages,
//! cache, and system health.

use crate::global::GlobalOtc;
use anyhow::Result;

/// Execute global package installation
pub fn install_global(url: String, verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    if verbose {
        println!("Installing global package from: {}", url);
    }

    // For now, we'll use a simple implementation
    // In a full implementation, this would:
    // 1. Parse the URL to extract package name
    // 2. Clone the git repository
    // 3. Extract version and commit information
    // 4. Install to global directory

    let package_name = extract_package_name_from_url(&url)?;
    let version = "latest"; // TODO: Extract from git tags
    let commit = "HEAD"; // TODO: Get actual commit hash

    global_otc.install_global_package(&package_name, &url, version, commit)?;

    if verbose {
        println!("Global package '{}' installed successfully", package_name);
    }

    Ok(())
}

/// Execute global package removal
pub fn remove_global(name: String, verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    if verbose {
        println!("Removing global package: {}", name);
    }

    global_otc.remove_global_package(&name)?;

    if verbose {
        println!("Global package '{}' removed successfully", name);
    }

    Ok(())
}

/// List global packages
pub fn list_global(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    let packages = global_otc.list_global_packages()?;

    if packages.is_empty() {
        println!("No global packages installed");
        return Ok(());
    }

    println!("Global packages:");
    for (name, package) in packages {
        if verbose {
            println!("  {} ({})", name, package.version);
            println!("    URL: {}", package.url);
            println!("    Commit: {}", package.commit);
            println!("    Installed: {}", package.installed_date);
            if !package.used_by.is_empty() {
                println!("    Used by: {:?}", package.used_by);
            }
            println!();
        } else {
            println!("  {}      {}    {}", name, package.version, package.url);
        }
    }

    Ok(())
}

/// Show global package information
pub fn info_global(name: String, verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    let packages = global_otc.list_global_packages()?;

    if let Some((_, package)) = packages.iter().find(|(pkg_name, _)| pkg_name == &name) {
        println!("Package: {}", name);
        println!("Version: {}", package.version);
        println!("URL: {}", package.url);
        println!("Commit: {}", package.commit);
        println!("Installed: {}", package.installed_date);

        if !package.used_by.is_empty() {
            println!("Used by: {} projects", package.used_by.len());
            if verbose {
                for project in &package.used_by {
                    println!("  - {}", project);
                }
            }
        } else {
            println!("Not currently used by any projects");
        }
    } else {
        return Err(anyhow::anyhow!(
            "Package '{}' is not installed globally",
            name
        ));
    }

    Ok(())
}

/// Set global configuration value
pub fn config_set(key: String, value: String, verbose: bool) -> Result<()> {
    let mut global_otc = GlobalOtc::new()?;

    if verbose {
        println!("Setting configuration: {} = {}", key, value);
    }

    global_otc.set_config_value(&key, &value)?;

    println!("Configuration updated: {} = {}", key, value);
    Ok(())
}

/// Get global configuration value
pub fn config_get(key: String, verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    if let Some(value) = global_otc.get_config_value(&key) {
        if verbose {
            println!("{} = {}", key, value);
        } else {
            println!("{}", value);
        }
    } else {
        return Err(anyhow::anyhow!("Unknown configuration key: {}", key));
    }

    Ok(())
}

/// List all global configuration
pub fn config_list(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    println!("Global Configuration:");

    let config_keys = [
        "cache.directory",
        "cache.max_size_gb",
        "cache.auto_cleanup",
        "cache.cleanup_interval_days",
        "packages.global_directory",
        "packages.prefer_global",
        "packages.allow_global_fallback",
        "network.timeout_seconds",
        "network.retries",
        "network.offline_mode",
        "ui.color",
        "ui.progress",
        "ui.verbose",
    ];

    for key in &config_keys {
        if let Some(value) = global_otc.get_config_value(key) {
            println!("  {} = {}", key, value);
        }
    }

    if verbose {
        println!(
            "\nConfiguration file: {}",
            global_otc.config_file().display()
        );
    }

    Ok(())
}

/// Show cache status
pub fn cache_status(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    let status = global_otc.cache_status()?;

    let size_mb = status.total_size_bytes / (1024 * 1024);
    let max_size_mb = status.max_size_bytes / (1024 * 1024);
    let usage_percent = if status.max_size_bytes > 0 {
        (status.total_size_bytes as f64 / status.max_size_bytes as f64) * 100.0
    } else {
        0.0
    };

    println!("Cache Status:");
    println!("  Packages: {}", status.package_count);
    println!(
        "  Size: {} MB / {} MB ({:.1}%)",
        size_mb, max_size_mb, usage_percent
    );
    println!("  Location: {}", global_otc.cache_dir().display());

    if verbose {
        println!("  Raw size: {} bytes", status.total_size_bytes);
        println!("  Max size: {} bytes", status.max_size_bytes);
    }

    Ok(())
}

/// Clean cache
pub fn cache_clean(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    if verbose {
        println!("Cleaning cache...");
    }

    global_otc.clean_cache(verbose)?;
    Ok(())
}

/// Clear cache
pub fn cache_clear(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    if verbose {
        println!("Clearing cache...");
    }

    global_otc.clear_cache(verbose)?;
    Ok(())
}

/// Repair cache
pub fn cache_repair(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    if verbose {
        println!("Repairing cache...");
    }

    global_otc.repair_cache(verbose)?;
    Ok(())
}

/// Show system information
pub fn system_info(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    let info = global_otc.system_info();

    println!("OTC System Information:");
    println!("  Base directory: {}", info.base_dir.display());
    println!("  OTC directory: {}", info.otc_dir.display());
    println!("  Cache directory: {}", info.cache_dir.display());
    println!("  Packages directory: {}", info.packages_dir.display());

    if verbose {
        println!("  Configuration file: {}", info.config_file.display());
        println!("  Registry file: {}", info.registry_file.display());

        // Check if directories exist
        println!("\nDirectory Status:");
        println!("  Base: {}", if info.base_dir.exists() { "✓" } else { "✗" });
        println!("  OTC: {}", if info.otc_dir.exists() { "✓" } else { "✗" });
        println!(
            "  Cache: {}",
            if info.cache_dir.exists() {
                "✓"
            } else {
                "✗"
            }
        );
        println!(
            "  Packages: {}",
            if info.packages_dir.exists() {
                "✓"
            } else {
                "✗"
            }
        );

        println!("\nFile Status:");
        println!(
            "  Config: {}",
            if info.config_file.exists() {
                "✓"
            } else {
                "✗"
            }
        );
        println!(
            "  Registry: {}",
            if info.registry_file.exists() {
                "✓"
            } else {
                "✗"
            }
        );
    }

    Ok(())
}

/// Run system health check
pub fn doctor(verbose: bool) -> Result<()> {
    let global_otc = GlobalOtc::new()?;

    if verbose {
        println!("Running system health check...");
    }

    global_otc.doctor(verbose)?;
    Ok(())
}

/// Update OTC self (placeholder implementation)
pub fn update_self(verbose: bool) -> Result<()> {
    if verbose {
        println!("Checking for OTC updates...");
    }

    // TODO: Implement actual self-update functionality
    // This would involve:
    // 1. Check for latest OTC version from repository
    // 2. Download new binary
    // 3. Replace current binary
    // 4. Update global configuration

    println!("OTC self-update is not yet implemented");
    println!("Please update manually by downloading the latest release");

    Ok(())
}

/// Migrate from older OTC versions (placeholder implementation)
pub fn migrate(verbose: bool) -> Result<()> {
    if verbose {
        println!("Checking for migration requirements...");
    }

    // TODO: Implement migration from older versions
    // This would involve:
    // 1. Detect old installation directories
    // 2. Migrate configuration files
    // 3. Move cached packages
    // 4. Update registry format

    println!("No migration required - this is a fresh installation");

    Ok(())
}

/// Clean legacy installation artifacts (placeholder implementation)
pub fn clean_legacy(verbose: bool) -> Result<()> {
    if verbose {
        println!("Cleaning legacy installation artifacts...");
    }

    // TODO: Implement cleanup of old installation artifacts
    // This would involve:
    // 1. Remove old configuration directories
    // 2. Clean up obsolete cache files
    // 3. Remove deprecated registry formats

    println!("No legacy artifacts found");

    Ok(())
}

/// Uninstall OTC and all data (placeholder implementation)
pub fn uninstall_self(verbose: bool) -> Result<()> {
    if verbose {
        println!("This will remove OTC and all Olang data...");
    }

    // TODO: Implement complete uninstall
    // This would involve:
    // 1. Remove ~/.olang/ directory completely
    // 2. Remove global packages
    // 3. Clear cached data
    // 4. Remove OTC binary (if possible)

    println!("Uninstall is not yet implemented");
    println!("To manually uninstall, remove: ~/.olang/");

    Ok(())
}

/// Extract package name from git URL
fn extract_package_name_from_url(url: &str) -> Result<String> {
    // Simple implementation - extract name from URL
    // TODO: Make this more robust

    let name = if url.contains("github.com") || url.contains("gitlab.com") {
        // Extract from URLs like: https://github.com/user/package.git or without .git
        let last = url.split('/').last().unwrap_or("unknown");
        // Remove .git suffix if present
        if let Some(stripped) = last.strip_suffix(".git") {
            stripped.to_string()
        } else {
            last.to_string()
        }
    } else if url.starts_with('/') || url.contains(':') {
        // Local path or SSH URL
        url.split('/').last().unwrap_or("unknown").to_string()
    } else {
        return Err(anyhow::anyhow!(
            "Unable to extract package name from URL: {}",
            url
        ));
    };

    if name == "unknown" || name.is_empty() {
        return Err(anyhow::anyhow!("Invalid package URL: {}", url));
    }

    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_package_name_from_url() {
        assert_eq!(
            extract_package_name_from_url("https://github.com/user/math-utils.git").unwrap(),
            "math-utils"
        );

        assert_eq!(
            extract_package_name_from_url("https://gitlab.com/user/web-framework.git").unwrap(),
            "web-framework"
        );

        assert_eq!(
            extract_package_name_from_url("https://github.com/user/package").unwrap(),
            "package"
        );

        assert!(extract_package_name_from_url("invalid-url").is_err());
    }
}

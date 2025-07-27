//! Global OTC Management System
//!
//! Implements the standardized global directory structure and configuration
//! for OTC installation, global packages, and system management.
//!
//! Global Directory Structure:
//! ~/.olang/                        # Main Olang directory
//! ├── otc/                         # OTC-specific files
//! │   ├── config.toml             # Global OTC configuration
//! │   ├── bin/                    # OTC binary and tools
//! │   ├── logs/                   # OTC operation logs
//! │   └── cache/                  # Temporary cache for operations
//! ├── local/                      # Local package cache
//! │   ├── packages/               # Downloaded git repositories
//! │   │   ├── github.com_user_pkg/
//! │   │   └── gitlab.com_user_pkg/
//! │   └── metadata/               # Package metadata cache
//! └── share/                      # Global shared packages
//!     ├── bin/                    # Global package binaries
//!     ├── lib/                    # Global package libraries
//!     └── installed.toml          # Global package registry

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Global OTC configuration structure
#[derive(Debug, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub otc: OtcConfig,
    pub cache: CacheConfig,
    pub packages: PackageConfig,
    pub network: NetworkConfig,
    pub ui: UiConfig,
}

/// OTC-specific configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct OtcConfig {
    pub version: String,
    pub installation_path: String,
    pub last_update_check: String,
}

/// Cache configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct CacheConfig {
    pub directory: String,
    pub max_size_gb: u64,
    pub auto_cleanup: bool,
    pub cleanup_interval_days: u64,
}

/// Package management configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct PackageConfig {
    pub global_directory: String,
    pub prefer_global: bool,
    pub allow_global_fallback: bool,
}

/// Network configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub timeout_seconds: u64,
    pub retries: u64,
    pub offline_mode: bool,
}

/// UI configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct UiConfig {
    pub color: String,
    pub progress: bool,
    pub verbose: bool,
}

/// Global package registry entry
#[derive(Debug, Serialize, Deserialize)]
pub struct GlobalPackage {
    pub version: String,
    pub url: String,
    pub commit: String,
    pub installed_date: String,
    pub used_by: Vec<String>,
}

/// Global package registry
#[derive(Debug, Serialize, Deserialize)]
pub struct GlobalRegistry {
    pub packages: HashMap<String, GlobalPackage>,
}

/// Global OTC management system
pub struct GlobalOtc {
    pub base_dir: PathBuf,
    pub config: GlobalConfig,
}

impl GlobalOtc {
    /// Initialize the global OTC system
    pub fn new() -> Result<Self> {
        let base_dir = get_global_base_dir()?;
        
        // Create directory structure if it doesn't exist
        create_global_directories(&base_dir)?;
        
        // Load or create configuration
        let config = load_or_create_config(&base_dir)?;
        
        Ok(Self { base_dir, config })
    }

    /// Get the global base directory path
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Get the OTC directory path
    pub fn otc_dir(&self) -> PathBuf {
        self.base_dir.join("otc")
    }

    /// Get the cache directory path
    pub fn cache_dir(&self) -> PathBuf {
        self.base_dir.join("local")
    }

    /// Get the global packages directory path
    pub fn packages_dir(&self) -> PathBuf {
        self.base_dir.join("share")
    }

    /// Get the configuration file path
    pub fn config_file(&self) -> PathBuf {
        self.otc_dir().join("config.toml")
    }

    /// Get the global registry file path
    pub fn registry_file(&self) -> PathBuf {
        self.packages_dir().join("installed.toml")
    }

    /// Save the current configuration
    pub fn save_config(&self) -> Result<()> {
        let config_content = toml::to_string_pretty(&self.config)
            .context("Failed to serialize global configuration")?;
        
        fs::write(self.config_file(), config_content)
            .context("Failed to write global configuration file")?;
        
        Ok(())
    }

    /// Update a configuration value
    pub fn set_config_value(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "cache.max_size_gb" => {
                self.config.cache.max_size_gb = value.parse()
                    .context("Invalid max_size_gb value")?;
            }
            "cache.auto_cleanup" => {
                self.config.cache.auto_cleanup = value.parse()
                    .context("Invalid auto_cleanup value")?;
            }
            "packages.prefer_global" => {
                self.config.packages.prefer_global = value.parse()
                    .context("Invalid prefer_global value")?;
            }
            "packages.allow_global_fallback" => {
                self.config.packages.allow_global_fallback = value.parse()
                    .context("Invalid allow_global_fallback value")?;
            }
            "network.timeout_seconds" => {
                self.config.network.timeout_seconds = value.parse()
                    .context("Invalid timeout_seconds value")?;
            }
            "network.retries" => {
                self.config.network.retries = value.parse()
                    .context("Invalid retries value")?;
            }
            "network.offline_mode" => {
                self.config.network.offline_mode = value.parse()
                    .context("Invalid offline_mode value")?;
            }
            "ui.color" => {
                if matches!(value, "auto" | "always" | "never") {
                    self.config.ui.color = value.to_string();
                } else {
                    return Err(anyhow::anyhow!("Invalid color value. Use: auto, always, never"));
                }
            }
            "ui.progress" => {
                self.config.ui.progress = value.parse()
                    .context("Invalid progress value")?;
            }
            "ui.verbose" => {
                self.config.ui.verbose = value.parse()
                    .context("Invalid verbose value")?;
            }
            _ => return Err(anyhow::anyhow!("Unknown configuration key: {}", key)),
        }
        
        self.save_config()?;
        Ok(())
    }

    /// Get a configuration value
    pub fn get_config_value(&self, key: &str) -> Option<String> {
        match key {
            "cache.directory" => Some(self.config.cache.directory.clone()),
            "cache.max_size_gb" => Some(self.config.cache.max_size_gb.to_string()),
            "cache.auto_cleanup" => Some(self.config.cache.auto_cleanup.to_string()),
            "cache.cleanup_interval_days" => Some(self.config.cache.cleanup_interval_days.to_string()),
            "packages.global_directory" => Some(self.config.packages.global_directory.clone()),
            "packages.prefer_global" => Some(self.config.packages.prefer_global.to_string()),
            "packages.allow_global_fallback" => Some(self.config.packages.allow_global_fallback.to_string()),
            "network.timeout_seconds" => Some(self.config.network.timeout_seconds.to_string()),
            "network.retries" => Some(self.config.network.retries.to_string()),
            "network.offline_mode" => Some(self.config.network.offline_mode.to_string()),
            "ui.color" => Some(self.config.ui.color.clone()),
            "ui.progress" => Some(self.config.ui.progress.to_string()),
            "ui.verbose" => Some(self.config.ui.verbose.to_string()),
            _ => None,
        }
    }

    /// Load the global package registry
    pub fn load_registry(&self) -> Result<GlobalRegistry> {
        let registry_file = self.registry_file();
        
        if !registry_file.exists() {
            // Create empty registry
            let registry = GlobalRegistry {
                packages: HashMap::new(),
            };
            self.save_registry(&registry)?;
            return Ok(registry);
        }

        let content = fs::read_to_string(&registry_file)
            .context("Failed to read global registry file")?;
        
        let registry: GlobalRegistry = toml::from_str(&content)
            .context("Failed to parse global registry file")?;
        
        Ok(registry)
    }

    /// Save the global package registry
    pub fn save_registry(&self, registry: &GlobalRegistry) -> Result<()> {
        let registry_content = toml::to_string_pretty(registry)
            .context("Failed to serialize global registry")?;
        
        fs::write(self.registry_file(), registry_content)
            .context("Failed to write global registry file")?;
        
        Ok(())
    }

    /// Install a package globally
    pub fn install_global_package(&self, name: &str, url: &str, version: &str, commit: &str) -> Result<()> {
        let mut registry = self.load_registry()?;
        
        let package = GlobalPackage {
            version: version.to_string(),
            url: url.to_string(),
            commit: commit.to_string(),
            installed_date: chrono::Utc::now().to_rfc3339(),
            used_by: Vec::new(),
        };

        registry.packages.insert(name.to_string(), package);
        self.save_registry(&registry)?;
        
        println!("Installed global package: {} ({})", name, version);
        Ok(())
    }

    /// Remove a global package
    pub fn remove_global_package(&self, name: &str) -> Result<()> {
        let mut registry = self.load_registry()?;
        
        if registry.packages.remove(name).is_some() {
            self.save_registry(&registry)?;
            
            // Remove package directory
            let package_dir = self.packages_dir().join("lib").join(name);
            if package_dir.exists() {
                fs::remove_dir_all(&package_dir)
                    .context("Failed to remove package directory")?;
            }
            
            println!("Removed global package: {}", name);
        } else {
            return Err(anyhow::anyhow!("Package '{}' is not installed globally", name));
        }
        
        Ok(())
    }

    /// List global packages
    pub fn list_global_packages(&self) -> Result<Vec<(String, GlobalPackage)>> {
        let registry = self.load_registry()?;
        let mut packages: Vec<_> = registry.packages.into_iter().collect();
        packages.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(packages)
    }

    /// Check cache status
    pub fn cache_status(&self) -> Result<CacheStatus> {
        let cache_dir = self.cache_dir();
        let packages_dir = cache_dir.join("packages");
        
        let mut total_size = 0u64;
        let mut package_count = 0usize;
        
        if packages_dir.exists() {
            for entry in fs::read_dir(&packages_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    package_count += 1;
                    total_size += calculate_dir_size(&entry.path())?;
                }
            }
        }

        Ok(CacheStatus {
            total_size_bytes: total_size,
            package_count,
            max_size_bytes: self.config.cache.max_size_gb * 1024 * 1024 * 1024,
        })
    }

    /// Clean unused cached packages
    pub fn clean_cache(&self, verbose: bool) -> Result<()> {
        let cache_dir = self.cache_dir().join("packages");
        
        if !cache_dir.exists() {
            if verbose {
                println!("Cache directory does not exist");
            }
            return Ok(());
        }

        let mut cleaned_count = 0;
        let mut freed_bytes = 0u64;

        // For now, we'll implement a simple cleanup strategy
        // In a full implementation, this would check for unused packages
        for entry in fs::read_dir(&cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_dir() {
                // Check if package is old (simple heuristic for now)
                if let Ok(metadata) = fs::metadata(&path) {
                    if let Ok(modified) = metadata.modified() {
                        let age = std::time::SystemTime::now()
                            .duration_since(modified)
                            .unwrap_or_default();
                        
                        // Remove packages older than cleanup interval
                        let cleanup_duration = std::time::Duration::from_secs(
                            self.config.cache.cleanup_interval_days * 24 * 60 * 60
                        );
                        
                        if age > cleanup_duration {
                            let size = calculate_dir_size(&path)?;
                            fs::remove_dir_all(&path)?;
                            cleaned_count += 1;
                            freed_bytes += size;
                            
                            if verbose {
                                println!("Removed: {}", path.display());
                            }
                        }
                    }
                }
            }
        }

        println!("Cache cleanup completed:");
        println!("  Removed {} packages", cleaned_count);
        println!("  Freed {} bytes", freed_bytes);
        
        Ok(())
    }

    /// Clear all cached packages
    pub fn clear_cache(&self, verbose: bool) -> Result<()> {
        let cache_dir = self.cache_dir().join("packages");
        
        if cache_dir.exists() {
            let size_before = calculate_dir_size(&cache_dir)?;
            fs::remove_dir_all(&cache_dir)?;
            fs::create_dir_all(&cache_dir)?;
            
            if verbose {
                println!("Cleared cache directory: {}", cache_dir.display());
                println!("Freed {} bytes", size_before);
            } else {
                println!("Cache cleared");
            }
        } else {
            println!("Cache directory does not exist");
        }
        
        Ok(())
    }

    /// Repair cache integrity
    pub fn repair_cache(&self, verbose: bool) -> Result<()> {
        let cache_dir = self.cache_dir();
        
        // Ensure all required directories exist
        create_cache_directories(&cache_dir)?;
        
        if verbose {
            println!("Cache directories verified");
        }

        // TODO: Add more sophisticated cache validation
        // - Check for corrupted git repositories
        // - Validate metadata files
        // - Repair incomplete downloads
        
        println!("Cache repair completed");
        Ok(())
    }

    /// Show system information
    pub fn system_info(&self) -> SystemInfo {
        SystemInfo {
            base_dir: self.base_dir.clone(),
            otc_dir: self.otc_dir(),
            cache_dir: self.cache_dir(),
            packages_dir: self.packages_dir(),
            config_file: self.config_file(),
            registry_file: self.registry_file(),
        }
    }

    /// Run system health check
    pub fn doctor(&self, verbose: bool) -> Result<()> {
        let mut issues = Vec::new();

        // Check directory structure
        if !self.base_dir.exists() {
            issues.push("Global base directory does not exist".to_string());
        }
        if !self.otc_dir().exists() {
            issues.push("OTC directory does not exist".to_string());
        }
        if !self.cache_dir().exists() {
            issues.push("Cache directory does not exist".to_string());
        }
        if !self.packages_dir().exists() {
            issues.push("Packages directory does not exist".to_string());
        }

        // Check configuration file
        if !self.config_file().exists() {
            issues.push("Global configuration file does not exist".to_string());
        }

        // Check registry file
        if !self.registry_file().exists() {
            issues.push("Global registry file does not exist".to_string());
        }

        // Check permissions
        if !is_directory_writable(&self.base_dir) {
            issues.push("Global base directory is not writable".to_string());
        }

        if issues.is_empty() {
            println!("System health check passed - no issues found");
        } else {
            println!("System health check found {} issues:", issues.len());
            for (i, issue) in issues.iter().enumerate() {
                println!("  {}: {}", i + 1, issue);
            }

            // Attempt to fix issues
            println!("\nAttempting to fix issues...");
            create_global_directories(&self.base_dir)?;
            
            if !self.config_file().exists() {
                let default_config = create_default_config(&self.base_dir)?;
                let config_content = toml::to_string_pretty(&default_config)?;
                fs::write(self.config_file(), config_content)?;
                println!("Created default configuration file");
            }

            if !self.registry_file().exists() {
                let registry = GlobalRegistry {
                    packages: HashMap::new(),
                };
                self.save_registry(&registry)?;
                println!("Created global registry file");
            }

            println!("Issues fixed - run doctor again to verify");
        }

        Ok(())
    }
}

/// Cache status information
#[derive(Debug)]
pub struct CacheStatus {
    pub total_size_bytes: u64,
    pub package_count: usize,
    pub max_size_bytes: u64,
}

/// System information
#[derive(Debug)]
pub struct SystemInfo {
    pub base_dir: PathBuf,
    pub otc_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub packages_dir: PathBuf,
    pub config_file: PathBuf,
    pub registry_file: PathBuf,
}

/// Get the global base directory path for the current platform
pub fn get_global_base_dir() -> Result<PathBuf> {
    if let Ok(portable_dir) = std::env::var("OLANG_PORTABLE_DIR") {
        // Portable mode - use specified directory
        return Ok(PathBuf::from(portable_dir).join("olang-data"));
    }

    // Use platform-specific home directory
    if let Some(home_dir) = dirs::home_dir() {
        Ok(home_dir.join(".olang"))
    } else {
        Err(anyhow::anyhow!("Unable to determine home directory"))
    }
}

/// Create the global directory structure
pub fn create_global_directories(base_dir: &Path) -> Result<()> {
    // Create main directories
    fs::create_dir_all(base_dir)?;
    fs::create_dir_all(base_dir.join("otc"))?;
    fs::create_dir_all(base_dir.join("otc/bin"))?;
    fs::create_dir_all(base_dir.join("otc/logs"))?;
    fs::create_dir_all(base_dir.join("otc/cache"))?;
    
    create_cache_directories(&base_dir.join("local"))?;
    
    fs::create_dir_all(base_dir.join("share"))?;
    fs::create_dir_all(base_dir.join("share/bin"))?;
    fs::create_dir_all(base_dir.join("share/lib"))?;

    // Set appropriate permissions (user read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(base_dir)?.permissions();
        perms.set_mode(0o700); // User read/write/execute only
        fs::set_permissions(base_dir, perms)?;
    }

    Ok(())
}

/// Create cache directories
pub fn create_cache_directories(cache_dir: &Path) -> Result<()> {
    fs::create_dir_all(cache_dir)?;
    fs::create_dir_all(cache_dir.join("packages"))?;
    fs::create_dir_all(cache_dir.join("metadata"))?;
    Ok(())
}

/// Load or create the global configuration
pub fn load_or_create_config(base_dir: &Path) -> Result<GlobalConfig> {
    let config_file = base_dir.join("otc/config.toml");
    
    if config_file.exists() {
        let content = fs::read_to_string(&config_file)
            .context("Failed to read global configuration file")?;
        
        toml::from_str(&content)
            .context("Failed to parse global configuration file")
    } else {
        let config = create_default_config(base_dir)?;
        
        let config_content = toml::to_string_pretty(&config)
            .context("Failed to serialize default configuration")?;
        
        fs::write(&config_file, config_content)
            .context("Failed to write default configuration file")?;
        
        Ok(config)
    }
}

/// Create default global configuration
pub fn create_default_config(base_dir: &Path) -> Result<GlobalConfig> {
    Ok(GlobalConfig {
        otc: OtcConfig {
            version: env!("CARGO_PKG_VERSION").to_string(),
            installation_path: base_dir.join("otc").to_string_lossy().to_string(),
            last_update_check: chrono::Utc::now().to_rfc3339(),
        },
        cache: CacheConfig {
            directory: base_dir.join("local").to_string_lossy().to_string(),
            max_size_gb: 10,
            auto_cleanup: true,
            cleanup_interval_days: 30,
        },
        packages: PackageConfig {
            global_directory: base_dir.join("share").to_string_lossy().to_string(),
            prefer_global: false,
            allow_global_fallback: true,
        },
        network: NetworkConfig {
            timeout_seconds: 30,
            retries: 3,
            offline_mode: false,
        },
        ui: UiConfig {
            color: "auto".to_string(),
            progress: true,
            verbose: false,
        },
    })
}

/// Calculate directory size recursively
fn calculate_dir_size(dir: &Path) -> Result<u64> {
    let mut total_size = 0u64;
    
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_dir() {
                total_size += calculate_dir_size(&path)?;
            } else {
                total_size += entry.metadata()?.len();
            }
        }
    }
    
    Ok(total_size)
}

/// Check if a directory is writable
fn is_directory_writable(dir: &Path) -> bool {
    // Try to create a temporary file to test writability
    let test_file = dir.join(".olang_write_test");
    match fs::write(&test_file, b"test") {
        Ok(_) => {
            let _ = fs::remove_file(&test_file);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_create_global_directories() {
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path();
        
        create_global_directories(base_dir).unwrap();
        
        assert!(base_dir.join("otc").exists());
        assert!(base_dir.join("otc/bin").exists());
        assert!(base_dir.join("otc/logs").exists());
        assert!(base_dir.join("local/packages").exists());
        assert!(base_dir.join("share/lib").exists());
    }

    #[test]
    fn test_default_config_creation() {
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path();
        
        let config = create_default_config(base_dir).unwrap();
        
        assert_eq!(config.cache.max_size_gb, 10);
        assert_eq!(config.network.timeout_seconds, 30);
        assert_eq!(config.ui.color, "auto");
    }

    #[test]
    fn test_config_serialization() {
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path();
        
        let config = create_default_config(base_dir).unwrap();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: GlobalConfig = toml::from_str(&serialized).unwrap();
        
        assert_eq!(config.cache.max_size_gb, deserialized.cache.max_size_gb);
        assert_eq!(config.network.timeout_seconds, deserialized.network.timeout_seconds);
    }
} 
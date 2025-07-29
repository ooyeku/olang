//! Lock File Management - Feature 4
//!
//! Manages olang.lock files for reproducible package installations.
//! Records exact versions and commit hashes for all installed packages.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Lock file structure for reproducible builds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    /// Metadata about the lock file
    #[serde(default)]
    pub metadata: LockMetadata,
    /// Exact package information
    pub packages: HashMap<String, LockedPackage>,
}

/// Lock file metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMetadata {
    /// Lock file format version
    #[serde(default = "default_version")]
    pub version: String,
    /// When the lock file was generated
    #[serde(default)]
    pub generated: Option<String>,
    /// OTC version that generated this lock file
    #[serde(default)]
    pub otc_version: Option<String>,
}

/// Locked package information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedPackage {
    /// Original git URL
    pub url: String,
    /// Exact commit hash
    pub commit: String,
    /// Package version (from manifest or git tag)
    pub version: String,
    /// When this package was installed
    #[serde(default)]
    pub installed_date: Option<String>,
    /// Dependencies of this package
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
}

impl Default for LockMetadata {
    fn default() -> Self {
        Self {
            version: default_version(),
            generated: Some(chrono::Utc::now().to_rfc3339()),
            otc_version: Some(olang::VERSION.to_string()),
        }
    }
}

fn default_version() -> String {
    "1".to_string()
}

impl LockFile {
    /// Create a new empty lock file
    pub fn new() -> Self {
        Self {
            metadata: LockMetadata::default(),
            packages: HashMap::new(),
        }
    }

    /// Load lock file from current directory
    pub fn load() -> Result<Self> {
        Self::load_from_path("olang.lock")
    }

    /// Load lock file from specific path
    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::new());
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read lock file: {}", path.display()))?;

        let lock_file: LockFile = toml::from_str(&content)
            .with_context(|| format!("Failed to parse lock file: {}", path.display()))?;

        Ok(lock_file)
    }

    /// Save lock file to current directory
    pub fn save(&self) -> Result<()> {
        self.save_to_path("olang.lock")
    }

    /// Save lock file to specific path
    pub fn save_to_path<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();

        let content = toml::to_string_pretty(self)
            .with_context(|| "Failed to serialize lock file")?;

        fs::write(path, content)
            .with_context(|| format!("Failed to write lock file: {}", path.display()))?;

        Ok(())
    }

    /// Add or update a package in the lock file
    pub fn add_package(&mut self, name: String, package: LockedPackage) {
        self.packages.insert(name, package);
        // Update generation timestamp
        self.metadata.generated = Some(chrono::Utc::now().to_rfc3339());
    }

    /// Remove a package from the lock file
    pub fn remove_package(&mut self, name: &str) -> Option<LockedPackage> {
        let result = self.packages.remove(name);
        if result.is_some() {
            // Update generation timestamp
            self.metadata.generated = Some(chrono::Utc::now().to_rfc3339());
        }
        result
    }

    /// Get a package from the lock file
    pub fn get_package(&self, name: &str) -> Option<&LockedPackage> {
        self.packages.get(name)
    }

    /// Check if a package is locked
    pub fn is_package_locked(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    /// Get all locked packages
    pub fn all_packages(&self) -> &HashMap<String, LockedPackage> {
        &self.packages
    }

    /// Check if lock file is empty
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Get package count
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Validate lock file integrity
    #[allow(dead_code)]
    pub fn validate(&self) -> Result<()> {
        // Check version compatibility
        if self.metadata.version != "1" {
            return Err(anyhow::anyhow!(
                "Unsupported lock file version: {}. Expected version 1.",
                self.metadata.version
            ));
        }

        // Validate each package entry
        for (name, package) in &self.packages {
            if name.is_empty() {
                return Err(anyhow::anyhow!("Empty package name in lock file"));
            }

            if package.url.is_empty() {
                return Err(anyhow::anyhow!("Empty URL for package: {}", name));
            }

            if package.commit.is_empty() {
                return Err(anyhow::anyhow!("Empty commit hash for package: {}", name));
            }

            if package.version.is_empty() {
                return Err(anyhow::anyhow!("Empty version for package: {}", name));
            }

            // Validate commit hash format (should be hex string)
            if !package.commit.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(anyhow::anyhow!(
                    "Invalid commit hash format for package {}: {}",
                    name, package.commit
                ));
            }

            if package.commit.len() < 7 {
                return Err(anyhow::anyhow!(
                    "Commit hash too short for package {}: {}",
                    name, package.commit
                ));
            }
        }

        Ok(())
    }

    /// Get dependencies for a specific package
    #[allow(dead_code)]
    pub fn get_package_dependencies(&self, name: &str) -> Option<&HashMap<String, String>> {
        self.packages.get(name).map(|pkg| &pkg.dependencies)
    }

    /// Update metadata with current timestamp
    #[allow(dead_code)]
    pub fn update_metadata(&mut self) {
        self.metadata.generated = Some(chrono::Utc::now().to_rfc3339());
        self.metadata.otc_version = Some(olang::VERSION.to_string());
    }

    /// Check if lock file is stale compared to olang.toml
    #[allow(dead_code)]
    pub fn is_stale(&self, olang_toml_path: &Path) -> Result<bool> {
        if !olang_toml_path.exists() {
            return Ok(false); // No olang.toml to compare against
        }

        let lock_path = olang_toml_path.with_file_name("olang.lock");
        if !lock_path.exists() {
            return Ok(true); // No lock file exists, so it's stale
        }

        // Compare modification times
        let toml_modified = fs::metadata(olang_toml_path)?
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        
        let lock_modified = fs::metadata(&lock_path)?
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        Ok(toml_modified > lock_modified)
    }

    /// Generate summary of locked packages
    #[allow(dead_code)]
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "No packages locked".to_string();
        }

        let mut summary = format!("Locked {} packages:", self.package_count());
        
        for (name, package) in &self.packages {
            summary.push_str(&format!(
                "\n  {} {} ({})",
                name,
                package.version,
                &package.commit[..8]
            ));
        }

        summary
    }
}

impl LockedPackage {
    /// Create a new locked package
    #[allow(dead_code)]
    pub fn new(url: String, commit: String, version: String) -> Self {
        Self {
            url,
            commit,
            version,
            installed_date: Some(chrono::Utc::now().to_rfc3339()),
            dependencies: HashMap::new(),
        }
    }

    /// Create a locked package with dependencies
    pub fn with_dependencies(
        url: String,
        commit: String,
        version: String,
        dependencies: HashMap<String, String>,
    ) -> Self {
        Self {
            url,
            commit,
            version,
            installed_date: Some(chrono::Utc::now().to_rfc3339()),
            dependencies,
        }
    }

    /// Get short commit hash (first 8 characters)
    pub fn short_commit(&self) -> &str {
        if self.commit.len() >= 8 {
            &self.commit[..8]
        } else {
            &self.commit
        }
    }

    /// Check if package has dependencies
    #[allow(dead_code)]
    pub fn has_dependencies(&self) -> bool {
        !self.dependencies.is_empty()
    }

    /// Get dependency count
    #[allow(dead_code)]
    pub fn dependency_count(&self) -> usize {
        self.dependencies.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lock_file_creation() {
        let lock_file = LockFile::new();
        assert!(lock_file.is_empty());
        assert_eq!(lock_file.package_count(), 0);
        assert_eq!(lock_file.metadata.version, "1");
    }

    #[test]
    fn test_add_remove_package() {
        let mut lock_file = LockFile::new();
        
        let package = LockedPackage::new(
            "https://github.com/user/test.git".to_string(),
            "abc123def456".to_string(),
            "v1.0.0".to_string(),
        );

        lock_file.add_package("test-package".to_string(), package);
        assert!(!lock_file.is_empty());
        assert_eq!(lock_file.package_count(), 1);
        assert!(lock_file.is_package_locked("test-package"));

        let removed = lock_file.remove_package("test-package");
        assert!(removed.is_some());
        assert!(lock_file.is_empty());
        assert!(!lock_file.is_package_locked("test-package"));
    }

    #[test]
    fn test_lock_file_serialization() {
        let mut lock_file = LockFile::new();
        
        let package = LockedPackage::new(
            "https://github.com/user/test.git".to_string(),
            "abc123def456".to_string(),
            "v1.0.0".to_string(),
        );

        lock_file.add_package("test-package".to_string(), package);

        let serialized = toml::to_string_pretty(&lock_file).unwrap();
        let deserialized: LockFile = toml::from_str(&serialized).unwrap();
        
        assert_eq!(lock_file.package_count(), deserialized.package_count());
        assert!(deserialized.is_package_locked("test-package"));
    }

    #[test]
    fn test_lock_file_validation() {
        let mut lock_file = LockFile::new();
        
        // Valid package
        let valid_package = LockedPackage::new(
            "https://github.com/user/test.git".to_string(),
            "abc123def456".to_string(),
            "v1.0.0".to_string(),
        );
        lock_file.add_package("valid-package".to_string(), valid_package);
        
        assert!(lock_file.validate().is_ok());

        // Invalid package with empty commit
        let invalid_package = LockedPackage {
            url: "https://github.com/user/test.git".to_string(),
            commit: "".to_string(),
            version: "v1.0.0".to_string(),
            installed_date: None,
            dependencies: HashMap::new(),
        };
        lock_file.add_package("invalid-package".to_string(), invalid_package);
        
        assert!(lock_file.validate().is_err());
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("test.lock");

        let mut original = LockFile::new();
        let package = LockedPackage::new(
            "https://github.com/user/test.git".to_string(),
            "abc123def456".to_string(),
            "v1.0.0".to_string(),
        );
        original.add_package("test-package".to_string(), package);

        // Save
        original.save_to_path(&lock_path).unwrap();
        assert!(lock_path.exists());

        // Load
        let loaded = LockFile::load_from_path(&lock_path).unwrap();
        assert_eq!(original.package_count(), loaded.package_count());
        assert!(loaded.is_package_locked("test-package"));
    }
} 
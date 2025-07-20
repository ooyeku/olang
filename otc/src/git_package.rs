//! Simple Git Package System - Feature 3
//!
//! Implements direct installation of packages from git repositories with minimal
//! configuration and maximum reliability. No complex registries, just git URLs.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Git package information parsed from URLs
#[derive(Debug, Clone, PartialEq)]
pub struct GitPackageUrl {
    pub url: String,
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub version: Option<String>,
    pub package_name: String,
}

/// Package metadata from olang.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub package: PackageInfo,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
}

/// Simple git-based package manager
pub struct GitPackageManager {
    cache_dir: PathBuf,
    packages_dir: PathBuf,
    metadata_dir: PathBuf,
}

/// Package installation result
#[derive(Debug)]
pub struct InstallResult {
    pub package_name: String,
    pub version: String,
    pub commit_hash: String,
    pub install_path: PathBuf,
    pub manifest: PackageManifest,
}

impl GitPackageManager {
    /// Create a new package manager with the specified cache directory
    pub fn new<P: AsRef<Path>>(cache_dir: P) -> Result<Self> {
        let cache_dir = cache_dir.as_ref().to_path_buf();
        let packages_dir = cache_dir.join("packages");
        let metadata_dir = cache_dir.join("metadata");

        // Create directories
        fs::create_dir_all(&packages_dir)
            .with_context(|| format!("Failed to create packages directory: {}", packages_dir.display()))?;
        fs::create_dir_all(&metadata_dir)
            .with_context(|| format!("Failed to create metadata directory: {}", metadata_dir.display()))?;

        Ok(Self {
            cache_dir,
            packages_dir,
            metadata_dir,
        })
    }

    /// Install a package from a git URL
    pub fn install_package(&self, url: &str, verbose: bool) -> Result<InstallResult> {
        if verbose {
            println!("Installing package from: {}", url);
        }

        // Parse the git URL
        let git_url = parse_git_url(url)?;
        if verbose {
            println!("Parsed package: {} from {}/{}", git_url.package_name, git_url.owner, git_url.repo);
        }

        // Create package-specific directory
        let package_cache_dir = self.create_package_cache_dir(&git_url)?;
        
        // Clone or update the repository
        let repo_dir = self.clone_repository(&git_url, &package_cache_dir, verbose)?;
        
        // Get the current commit hash
        let commit_hash = get_commit_hash(&repo_dir)?;
        if verbose {
            println!("Repository commit: {}", commit_hash);
        }

        // Validate package structure
        let manifest = self.validate_package(&repo_dir, &git_url)?;
        
        // Create installation metadata
        let install_result = InstallResult {
            package_name: git_url.package_name.clone(),
            version: git_url.version.unwrap_or_else(|| "latest".to_string()),
            commit_hash,
            install_path: repo_dir,
            manifest,
        };

        // Save package metadata
        self.save_package_metadata(&install_result)?;

        if verbose {
            println!("Package '{}' installed successfully", install_result.package_name);
        }

        Ok(install_result)
    }

    /// List installed packages
    pub fn list_packages(&self) -> Result<Vec<(String, PackageManifest)>> {
        let mut packages = Vec::new();
        
        if !self.metadata_dir.exists() {
            return Ok(packages);
        }

        for entry in fs::read_dir(&self.metadata_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_file() && path.extension().map_or(false, |ext| ext == "toml") {
                if let Some(package_name) = path.file_stem().and_then(|s| s.to_str()) {
                    match self.load_package_metadata(package_name) {
                        Ok(manifest) => packages.push((package_name.to_string(), manifest)),
                        Err(e) => eprintln!("Warning: Failed to load metadata for {}: {}", package_name, e),
                    }
                }
            }
        }

        Ok(packages)
    }

    /// Check if a package is installed
    pub fn is_package_installed(&self, name: &str) -> bool {
        self.metadata_dir.join(format!("{}.toml", name)).exists()
    }

    /// Get package installation path
    pub fn get_package_path(&self, name: &str) -> Option<PathBuf> {
        if self.is_package_installed(name) {
            // Find the package directory
            if let Ok(entries) = fs::read_dir(&self.packages_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && path.file_name()
                        .and_then(|n| n.to_str())
                        .map_or(false, |n| n.contains(name)) {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    /// Remove an installed package
    pub fn remove_package(&self, name: &str, verbose: bool) -> Result<()> {
        if !self.is_package_installed(name) {
            return Err(anyhow::anyhow!("Package '{}' is not installed", name));
        }

        // Remove package directory
        if let Some(package_path) = self.get_package_path(name) {
            if verbose {
                println!("Removing package directory: {}", package_path.display());
            }
            fs::remove_dir_all(&package_path)
                .with_context(|| format!("Failed to remove package directory: {}", package_path.display()))?;
        }

        // Remove metadata
        let metadata_path = self.metadata_dir.join(format!("{}.toml", name));
        if metadata_path.exists() {
            if verbose {
                println!("Removing package metadata: {}", metadata_path.display());
            }
            fs::remove_file(&metadata_path)
                .with_context(|| format!("Failed to remove package metadata: {}", metadata_path.display()))?;
        }

        if verbose {
            println!("Package '{}' removed successfully", name);
        }

        Ok(())
    }

    /// Create package-specific cache directory
    fn create_package_cache_dir(&self, git_url: &GitPackageUrl) -> Result<PathBuf> {
        // Create a safe directory name from the URL
        let dir_name = format!("{}_{}_{}",
            git_url.host.replace('.', "_"),
            git_url.owner,
            git_url.repo
        );
        
        let package_dir = self.packages_dir.join(dir_name);
        fs::create_dir_all(&package_dir)
            .with_context(|| format!("Failed to create package directory: {}", package_dir.display()))?;
        
        Ok(package_dir)
    }

    /// Clone or update repository
    fn clone_repository(&self, git_url: &GitPackageUrl, cache_dir: &Path, verbose: bool) -> Result<PathBuf> {
        let repo_dir = cache_dir.join(&git_url.repo);

        if repo_dir.exists() {
            // Repository already exists, update it
            if verbose {
                println!("Updating existing repository: {}", repo_dir.display());
            }
            self.update_repository(&repo_dir, git_url, verbose)?;
        } else {
            // Clone the repository
            if verbose {
                println!("Cloning repository to: {}", repo_dir.display());
            }
            self.clone_fresh_repository(&git_url.url, &repo_dir, verbose)?;
        }

        // Checkout specific version if specified
        if let Some(version) = &git_url.version {
            self.checkout_version(&repo_dir, version, verbose)?;
        }

        Ok(repo_dir)
    }

    /// Clone a fresh repository
    fn clone_fresh_repository(&self, url: &str, target_dir: &Path, verbose: bool) -> Result<()> {
        let mut cmd = Command::new("git");
        cmd.args(["clone", url, target_dir.to_str().unwrap()]);
        
        if !verbose {
            cmd.args(["--quiet"]);
        }

        let output = cmd.output()
            .with_context(|| "Failed to execute git clone command")?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Git clone failed: {}", error));
        }

        Ok(())
    }

    /// Update existing repository
    fn update_repository(&self, repo_dir: &Path, git_url: &GitPackageUrl, verbose: bool) -> Result<()> {
        let mut cmd = Command::new("git");
        cmd.args(["-C", repo_dir.to_str().unwrap(), "pull", "origin"]);
        
        // Determine the branch to pull
        let branch = git_url.version.as_deref().unwrap_or("main");
        cmd.arg(branch);

        if !verbose {
            cmd.args(["--quiet"]);
        }

        let output = cmd.output()
            .with_context(|| "Failed to execute git pull command")?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            // Don't fail on pull errors, as the repository might still be usable
            if verbose {
                eprintln!("Warning: Git pull failed: {}", error);
            }
        }

        Ok(())
    }

    /// Checkout specific version (tag or branch)
    fn checkout_version(&self, repo_dir: &Path, version: &str, verbose: bool) -> Result<()> {
        if verbose {
            println!("Checking out version: {}", version);
        }

        let mut cmd = Command::new("git");
        cmd.args(["-C", repo_dir.to_str().unwrap(), "checkout", version]);
        
        if !verbose {
            cmd.args(["--quiet"]);
        }

        let output = cmd.output()
            .with_context(|| format!("Failed to checkout version {}", version))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Git checkout failed for version '{}': {}", version, error));
        }

        Ok(())
    }

    /// Validate package structure and load manifest
    fn validate_package(&self, repo_dir: &Path, git_url: &GitPackageUrl) -> Result<PackageManifest> {
        // Check for olang.toml
        let manifest_path = repo_dir.join("olang.toml");
        if !manifest_path.exists() {
            return Err(anyhow::anyhow!(
                "Package invalid: missing olang.toml manifest\n\
                Add olang.toml with [package] section to make this a valid Olang package"
            ));
        }

        // Load and parse manifest
        let manifest_content = fs::read_to_string(&manifest_path)
            .with_context(|| format!("Failed to read package manifest: {}", manifest_path.display()))?;

        let manifest: PackageManifest = toml::from_str(&manifest_content)
            .with_context(|| format!("Invalid package manifest format: {}", manifest_path.display()))?;

        // Validate package name matches expected
        let expected_name = &git_url.package_name;
        if manifest.package.name != *expected_name {
            eprintln!("Warning: Package name '{}' in manifest doesn't match expected name '{}'", 
                manifest.package.name, expected_name);
        }

        Ok(manifest)
    }

    /// Save package metadata
    fn save_package_metadata(&self, install_result: &InstallResult) -> Result<()> {
        let metadata_path = self.metadata_dir.join(format!("{}.toml", install_result.package_name));
        
        let metadata_content = toml::to_string_pretty(&install_result.manifest)
            .with_context(|| "Failed to serialize package metadata")?;
        
        fs::write(&metadata_path, metadata_content)
            .with_context(|| format!("Failed to save package metadata: {}", metadata_path.display()))?;

        Ok(())
    }

    /// Load package metadata
    fn load_package_metadata(&self, package_name: &str) -> Result<PackageManifest> {
        let metadata_path = self.metadata_dir.join(format!("{}.toml", package_name));
        
        let metadata_content = fs::read_to_string(&metadata_path)
            .with_context(|| format!("Failed to read package metadata: {}", metadata_path.display()))?;
        
        let manifest: PackageManifest = toml::from_str(&metadata_content)
            .with_context(|| format!("Failed to parse package metadata: {}", metadata_path.display()))?;
        
        Ok(manifest)
    }
}

/// Parse git URL and extract package information
pub fn parse_git_url(url: &str) -> Result<GitPackageUrl> {
    // Handle version specification in URL (e.g., url@version)
    let (base_url, version) = if let Some(at_pos) = url.rfind('@') {
        let base = &url[..at_pos];
        let ver = &url[at_pos + 1..];
        // Don't treat user@host as version specification
        if ver.contains(':') || ver.contains('/') {
            (url, None)
        } else {
            (base, Some(ver.to_string()))
        }
    } else {
        (url, None)
    };

    // Parse different URL formats
    if base_url.starts_with("https://") || base_url.starts_with("http://") {
        parse_https_url(base_url, version)
    } else if base_url.starts_with("git@") {
        parse_ssh_url(base_url, version)
    } else if base_url.starts_with('/') || base_url.contains(':') {
        parse_local_url(base_url, version)
    } else {
        Err(anyhow::anyhow!(
            "Unsupported URL format: {}\n\
            Supported formats:\n\
            - HTTPS: https://github.com/user/repo.git\n\
            - SSH: git@github.com:user/repo.git\n\
            - Local: /path/to/repo", url
        ))
    }
}

/// Parse HTTPS git URL
fn parse_https_url(url: &str, version: Option<String>) -> Result<GitPackageUrl> {
    // Remove .git suffix if present
    let clean_url = url.strip_suffix(".git").unwrap_or(url);
    
    // Parse URL: https://host/owner/repo
    let parts: Vec<&str> = clean_url.split('/').collect();
    if parts.len() < 5 {
        return Err(anyhow::anyhow!("Invalid HTTPS URL format: {}", url));
    }

    let host = parts[2];
    let owner = parts[3];
    let repo = parts[4];
    let package_name = repo.to_string();

    Ok(GitPackageUrl {
        url: url.to_string(),
        host: host.to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        version,
        package_name,
    })
}

/// Parse SSH git URL
fn parse_ssh_url(url: &str, version: Option<String>) -> Result<GitPackageUrl> {
    // Format: git@host:owner/repo.git
    if !url.contains(':') {
        return Err(anyhow::anyhow!("Invalid SSH URL format: {}", url));
    }

    let (user_host, path) = url.split_once(':').unwrap();
    let host = user_host.strip_prefix("git@").unwrap_or(user_host);
    
    // Remove .git suffix if present
    let clean_path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<&str> = clean_path.split('/').collect();
    
    if parts.len() < 2 {
        return Err(anyhow::anyhow!("Invalid SSH URL format: {}", url));
    }

    let owner = parts[0];
    let repo = parts[1];
    let package_name = repo.to_string();

    Ok(GitPackageUrl {
        url: url.to_string(),
        host: host.to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        version,
        package_name,
    })
}

/// Parse local git URL
fn parse_local_url(url: &str, version: Option<String>) -> Result<GitPackageUrl> {
    let path = Path::new(url);
    let repo = path.file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid local path: {}", url))?;

    let package_name = repo.to_string();

    Ok(GitPackageUrl {
        url: url.to_string(),
        host: "localhost".to_string(),
        owner: "local".to_string(),
        repo: repo.to_string(),
        version,
        package_name,
    })
}

/// Get current commit hash of a repository
fn get_commit_hash(repo_dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", repo_dir.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .with_context(|| "Failed to get commit hash")?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Failed to get commit hash: {}", error));
    }

    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(hash)
}

/// Validate git URL format without cloning
pub fn validate_git_url(url: &str) -> Result<()> {
    parse_git_url(url)?;
    Ok(())
}

/// Check if git is available on the system
pub fn check_git_available() -> Result<()> {
    let output = Command::new("git")
        .args(["--version"])
        .output()
        .with_context(|| "Git command not found. Please install git and ensure it's in your PATH")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("Git is not working properly"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_https_url() {
        let url = "https://github.com/user/math-utils.git";
        let parsed = parse_git_url(url).unwrap();
        
        assert_eq!(parsed.host, "github.com");
        assert_eq!(parsed.owner, "user");
        assert_eq!(parsed.repo, "math-utils");
        assert_eq!(parsed.package_name, "math-utils");
        assert_eq!(parsed.version, None);
    }

    #[test]
    fn test_parse_https_url_with_version() {
        let url = "https://github.com/user/math-utils.git@v1.2.0";
        let parsed = parse_git_url(url).unwrap();
        
        assert_eq!(parsed.version, Some("v1.2.0".to_string()));
    }

    #[test]
    fn test_parse_ssh_url() {
        let url = "git@github.com:user/math-utils.git";
        let parsed = parse_git_url(url).unwrap();
        
        assert_eq!(parsed.host, "github.com");
        assert_eq!(parsed.owner, "user");
        assert_eq!(parsed.repo, "math-utils");
    }

    #[test]
    fn test_parse_local_url() {
        let url = "/local/path/to/package";
        let parsed = parse_git_url(url).unwrap();
        
        assert_eq!(parsed.host, "localhost");
        assert_eq!(parsed.owner, "local");
        assert_eq!(parsed.repo, "package");
    }

    #[test]
    fn test_invalid_url() {
        let url = "invalid-url";
        assert!(parse_git_url(url).is_err());
    }
} 